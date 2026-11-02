//! Port module: fifteen weapon and audio leaves from doc 44 ("Port candidates"), each checked
//! against the disassembly (doc 44's recipes were wrong in a few small places, see the notes at
//! the individual ports).
//!
//! Every port goes through a `pushad` thunk: all registers come back as the caller left them
//! except EAX, which carries the result (an `f32` result stays on ST0). That is a superset of
//! what each original preserves. Where an original returns AL only, the rest of EAX is
//! reproduced as the original leaves it, so the whole of EAX is the same.
//!
//! Shadow check: for the first `SHADOW_CALLS` calls of each port a relocated copy of the
//! original runs on the same seven registers (and stack arguments) and its EAX / ST0 / memory
//! writes are compared with the Rust version's; on a mismatch the original's are kept.
//! - Pure ports: the original runs, EAX (or ST0) is compared.
//! - `weapon_instance_free` (game thread only): the two list heads and both words of the nodes it
//!   can touch are snapshotted, the original runs, its result is saved, the snapshot restored,
//!   the Rust version runs and the two are compared.
//! - The audio-thread-visible writers (`audio_voice_set_volume`, `audio_voices_stop_category`,
//!   `audio_voices_mark_dirty_all`, `audio_set_category_throttle`) are never run twice on live
//!   state (the audio thread also uses it): the Rust code runs on a private copy of the bytes it
//!   touches, the original runs on the live state, and the copy is compared with the live bytes
//!   afterwards. The original's writes always stand. A write by the audio thread between the
//!   copy and the compare can show up as a (harmless) mismatch.
//!
//! Second round (37 more ports, see the section after `throttle_entry`): the voice fade / orientation /
//! parameter wrappers, the 15 `audio_play_params_init*` variants, pause / resume / fade-out-all, the
//! 3D-position and play-prepared writers, and the ammo store / weapon instance functions. Voice-slot
//! writers keep the original's order of reads and writes with volatile accesses; the originals of
//! the second round are run through `call_regs` in the shadow checks.
//!
//! No original here takes a lock or a critical section, and the Rust versions take none either.
//! Read-modify-write sequences are kept in the same shape as the originals (a single `or word
//! [m], imm` stays a single instruction; a load / test / store stays a load / test / store).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- game addresses -------------------------------------------------------------------------------

const AMMO_TYPE_COUNT: usize = 0x022D_7B50; // number of entries in an ammo store
const MP_FLAG_A: usize = 0x0252_8B48; // byte, 1 in multiplayer ...
const MP_FLAG_B: usize = 0x0252_8B49; // ... with this one 0
const WEAPON_INFOS: usize = 0x022D_7BCC; // base of the 0x498-byte infos
const WEAPON_INFO_COUNT: usize = 0x022D_7BD0;
const INST_LIVE: usize = 0x022D_7B6C; // circular list heads of the weapon instances
const INST_FREE: usize = 0x022D_7BC4;
const AI_RANGE_DEFAULT: usize = 0x00E5_C110; // f32 1.275
const FOLEY_COUNT: usize = 0x0252_6AF0; // u16
const FOLEY_TABLE: usize = 0x00EE_5558; // pointers to entries whose first dword is the hash
const MUSIC_COUNT: usize = 0x0252_6AF4;
const MUSIC_TABLE: usize = 0x00EE_7178;
const LINE_COUNT: usize = 0x0252_6AF8;
const LINE_TABLE: usize = 0x0257_4360; // 16-byte entries, hash first
const VOICE_SLOTS: usize = 0x00EE_B8F8; // 128 voice slots of 0x19C bytes
const SLOT_SIZE: usize = 0x19C;
const SLOT_COUNT: usize = 128;
const THROTTLE: usize = 0x00EE_34F8; // 4 f32

// --- registers ------------------------------------------------------------------------------------

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)] // esp is never read
struct Pushad {
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// The seven general registers in and out of `call_full`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Regs {
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

impl Regs {
    fn of(p: &Pushad) -> Regs {
        Regs { eax: p.eax, ecx: p.ecx, edx: p.edx, ebx: p.ebx, esi: p.esi, edi: p.edi, ebp: p.ebp }
    }
    fn get(&self, i: usize) -> u32 {
        [self.eax, self.ecx, self.edx, self.ebx, self.esi, self.edi, self.ebp][i]
    }
}

const REG_NAMES: [&str; 7] = ["eax", "ecx", "edx", "ebx", "esi", "edi", "ebp"];
const EAX: u8 = 1;
const ECX: u8 = 2;
const EDX: u8 = 4;
/// EBX / ESI / EDI / EBP: callee-saved in every original here.
const SAVED: u8 = 8 | 16 | 32 | 64;

#[repr(C)]
#[allow(dead_code)] // fields are used by the asm only
struct Block {
    f: u32,
    r: Regs,
    arg: u32,
    esp: u32,
    st0: f64,
    want_st0: u32,
}

/// Call `f` with all seven registers from `r` and one stack argument (harmless for the callees
/// that take none), then capture all seven registers into `r` and, when `want_st0`, pop ST0
/// (returned). For cdecl callees (or `ret 4` ones).
unsafe fn call_full(f: usize, r: &mut Regs, arg: u32, want_st0: bool) -> f64 {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0, st0: 0.0, want_st0: want_st0 as u32 };
    unsafe {
        // Above the argument: the target, then the block pointer twice, so that it is at
        // [esp + 12] after `push ebx` whether the callee popped its argument or not.
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 36], esp",
            "push eax",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 4]",
            "push ebx",
            "mov ebx, [esp + 12]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "cmp dword ptr [ebx + 48], 0",
            "je 2f",
            "fstp qword ptr [ebx + 40]",
            "2:",
            "mov esp, [ebx + 36]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
    b.st0
}

#[repr(C)]
#[allow(dead_code)] // fields are used by the asm only
struct Block2 {
    f: u32,
    r: Regs,
    a0: u32,
    a1: u32,
    esp: u32,
    pad: u32,
}

/// `call_full` for a callee taking two stack arguments and popping both (`ret 8`).
unsafe fn call_full2(f: usize, r: &mut Regs, a0: u32, a1: u32) {
    let mut b = Block2 { f: f as u32, r: *r, a0, a1, esp: 0, pad: 0 };
    unsafe {
        // Stack at the call, low to high: a0, a1, target, block, block. The callee pops a0 and
        // a1, which leaves the second block pointer at [esp + 12] after `push ebx`.
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 40], esp",
            "push eax",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 8]",
            "push ebx",
            "mov ebx, [esp + 12]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 40]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block2 => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX; an `f32` return stays on ST0.
macro_rules! regs_thunk {
    ($name:ident, $f:ident) => {
        regs_thunk!($name, $f, "ret");
    };
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

// --- the port table -------------------------------------------------------------------------------

struct Port {
    name: &'static str,
    addr: usize,
    /// Registers the original leaves unchanged on every path (checked in the shadow).
    preserved: u8,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize, preserved: u8) -> Port {
    Port { name, addr, preserved, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const TOTAL_AMMO: usize = 0;
const STORE_COUNT: usize = 1;
const STORE_FIND: usize = 2;
const STORE_CAPACITY: usize = 3;
const INFO_INDEX: usize = 4;
const IS_LETHAL: usize = 5;
const USES_SEWAGE: usize = 6;
const USES_BULLET: usize = 7;
const USES_PROJECTILE: usize = 8;
const HAS_AMMO: usize = 9;
const AI_RANGE_SQ: usize = 10;
const AI_RANGE: usize = 11;
const INSTANCE_FREE: usize = 12;
const MAX_RESERVE: usize = 13;
const FOLEY_FIND: usize = 14;
const LINE_FIND: usize = 15;
const MUSIC_FIND: usize = 16;
const SET_VOLUME: usize = 17;
const GET_VOLUME: usize = 18;
const STOP_CATEGORY: usize = 19;
const MARK_DIRTY: usize = 20;
const THROTTLE_SET: usize = 21;
const SLOT_FADE_OUT: usize = 22;
const FADE_OUT: usize = 23;
const FADE_OUT2: usize = 24;
const SET_ORIENT: usize = 25;
const SET_ORIENT2: usize = 26;
const IS_2D: usize = 27;
const GET_P30: usize = 28;
const SET_P28: usize = 29;
const SET_P30: usize = 30;
const FADE_IN: usize = 31;
const PI_DEFAULT: usize = 32;
const PI_BASIC: usize = 33;
const PI_VOLUME: usize = 34;
const PI_VARIANT: usize = 35;
const PI_VARIANT_VOLUME: usize = 36;
const PI_EXT: usize = 37;
const PI_3D: usize = 38;
const PI_3D_CHAN: usize = 39;
const PI_3D_CHAN2: usize = 40;
const PI_3D_CHAN_FLAG: usize = 41;
const PI_3D_VOLUME: usize = 42;
const PI_3D_EXT: usize = 43;
const PI_3D_VARIANT: usize = 44;
const PI_3D_VARIANT_VOLUME: usize = 45;
const PI_3D_VARIANT_EXT: usize = 46;
const PAUSE_ALL: usize = 47;
const RESUME_ALL: usize = 48;
const FADE_ALL: usize = 49;
const SET_3D_POS: usize = 50;
const PLAY_PREPARED: usize = 51;
const ENTRY_ADD: usize = 52;
const ADD_TYPE: usize = 53;
const STORE_REMOVE: usize = 54;
const WEAPON_STORE_REMOVE: usize = 55;
const STORE_CLEAR: usize = 56;
const IS_THROWN: usize = 57;
const INST_ALLOC: usize = 58;
const OLD_COUNT: usize = 59;
const SLOT_ALLOC_ID: usize = 59;
const START_2D_ID: usize = 60;
const VOICE_START_ID: usize = 61;
const VOICE_PREPARE_ID: usize = 62;
const FOLEY_CUE_ID: usize = 63;
const VOICE_LINE_ID: usize = 64;
const MUSIC_CUE_ID: usize = 65;
const RELOAD_ID: usize = 66;
const HANDLE_VALID_ID: usize = 67;
const INFO_IS_MELEE_ID: usize = 68;
const CLASS_IS_MELEE_ID: usize = 69;
const MOUNT_FIND_ID: usize = 70;
const PLAY_FOLEY_2D_ID: usize = 71;
const PLAY_FOLEY_2D_TAGGED_ID: usize = 72;
const PLAY_FOLEY_3D_ID: usize = 73;
const PLAY_LINE_2D_ID: usize = 74;
const AUDIO_PREPARE_ID: usize = 75;
const SET_LISTENER_ID: usize = 76;
const STATION_STOP_ID: usize = 77;
const LOCK_CLEAR_ID: usize = 78;
const REMOVE_RESERVE_ID: usize = 79;
const CUE_ADD_ID: usize = 80;
const RADIO_TRACK_FIND_ID: usize = 81;
const RADIO_STATION_FIND_ID: usize = 82;
const DJ_STOP_ID: usize = 83;
const RADIO_OFF_ID: usize = 84;
const RADIO_BLOCK_ID: usize = 85;
const COUNT: usize = 86;

static PORTS: [Port; COUNT] = [
    port("weapon_total_ammo", 0x00B6_9020, SAVED),
    port("ammo_store_count", 0x00B6_8EB0, SAVED),
    port("ammo_store_find", 0x00B6_8CE0, SAVED),
    port("ammo_store_capacity", 0x00B6_8F60, SAVED),
    port("weapon_info_index", 0x00B7_3B70, SAVED),
    port("weapon_info_is_lethal", 0x00B7_2F80, SAVED),
    port("weapon_info_uses_sewage_ammo", 0x00B7_2840, ECX | SAVED),
    port("weapon_info_uses_bullet_ammo", 0x00B7_28A0, ECX | SAVED),
    port("weapon_info_uses_projectile_ammo", 0x00B7_2E90, ECX | EDX | SAVED),
    port("weapon_instance_has_ammo", 0x00B7_2CA0, EDX | SAVED),
    port("weapon_info_ai_range_sq", 0x00B7_1A60, EAX | ECX | EDX | SAVED),
    port("weapon_info_ai_range", 0x00B7_1A30, EAX | ECX | EDX | SAVED),
    port("weapon_instance_free", 0x00B7_1C80, EAX | SAVED),
    port("weapon_info_max_reserve", 0x009C_3F50, SAVED),
    port("audio_foley_find_by_hash", 0x0047_9A50, SAVED),
    port("audio_voice_line_find_by_hash", 0x0047_AFB0, SAVED),
    port("audio_music_find_by_hash", 0x0047_A2B0, SAVED),
    port("audio_voice_set_volume", 0x0048_0070, EDX | SAVED),
    port("audio_voice_get_volume", 0x0046_6140, ECX | EDX | SAVED),
    port("audio_voices_stop_category", 0x0047_F200, EDX | SAVED),
    port("audio_voices_mark_dirty_all", 0x0047_F2E0, SAVED),
    port("audio_set_category_throttle", 0x0046_ED20, ECX | EDX | SAVED),
    port("audio_voice_slot_fade_out", 0x0047_C740, EDX | SAVED),
    port("audio_voice_fade_out", 0x0046_5F70, SAVED),
    port("audio_voice_fade_out_2", 0x0047_FF80, SAVED),
    port("audio_voice_set_orientation", 0x0046_5FD0, SAVED),
    port("audio_voice_set_orientation_2", 0x0048_03B0, SAVED),
    port("audio_voice_is_2d", 0x0046_60B0, EDX | SAVED),
    port("audio_voice_get_param_30", 0x0046_6180, ECX | EDX | SAVED),
    port("audio_voice_set_param_28", 0x0048_00C0, SAVED),
    port("audio_voice_set_param_30", 0x0048_0170, SAVED),
    port("audio_voice_slot_fade_in", 0x0047_CED0, SAVED),
    port("audio_play_params_init_empty", 0x0046_4860, SAVED),
    port("audio_play_params_init", 0x0046_4920, SAVED),
    port("audio_play_params_init_volume", 0x0046_4970, SAVED),
    port("audio_play_params_init_variant", 0x0046_49C0, SAVED),
    port("audio_play_params_init_variant_volume", 0x0046_4A10, SAVED),
    port("audio_play_params_init_ext", 0x0046_4A60, SAVED),
    port("audio_play_params_init_3d", 0x0046_4AC0, SAVED),
    port("audio_play_params_init_3d_channel", 0x0046_4B30, SAVED),
    port("audio_play_params_init_3d_channel2", 0x0046_4BA0, SAVED),
    port("audio_play_params_init_3d_channel_flag", 0x0046_4C10, SAVED),
    port("audio_play_params_init_3d_volume", 0x0046_4CA0, SAVED),
    port("audio_play_params_init_3d_ext", 0x0046_4D10, SAVED),
    port("audio_play_params_init_3d_variant", 0x0046_4D90, SAVED),
    port("audio_play_params_init_3d_variant_volume", 0x0046_4E00, SAVED),
    port("audio_play_params_init_3d_variant_ext", 0x0046_4E70, SAVED),
    port("audio_pause_all", 0x0047_F080, SAVED),
    port("audio_resume_all", 0x0047_F170, SAVED),
    port("audio_voices_fade_out_all", 0x0047_F020, SAVED),
    port("audio_voice_set_3d_position", 0x0048_02D0, ECX | SAVED),
    port("audio_voice_play_prepared", 0x0047_F800, SAVED),
    port("ammo_store_entry_add", 0x00B6_8C20, SAVED),
    port("ammo_store_add_type", 0x00B6_90C0, SAVED),
    port("ammo_store_remove", 0x00B6_9190, SAVED),
    port("weapon_ammo_store_remove", 0x00B6_9230, SAVED),
    port("ammo_store_clear", 0x00B6_93E0, SAVED),
    port("weapon_info_is_thrown", 0x00B7_2C40, EDX | SAVED),
    port("weapon_instance_alloc", 0x00B7_1AC0, SAVED),
    port("audio_voice_slot_alloc", 0x0047CDB0, SAVED),
    port("audio_voice_start_2d_channel", 0x0047FBA0, SAVED),
    port("audio_voice_start", 0x0047F4B0, SAVED),
    port("audio_voice_prepare", 0x0047F880, SAVED),
    port("audio_play_foley_cue", 0x00479B60, SAVED),
    port("audio_play_voice_line", 0x0047B0B0, SAVED),
    port("audio_play_music_cue", 0x0047A4A0, SAVED),
    port("weapon_reload_from_store", 0x00B69260, SAVED),
    port("audio_voice_handle_valid", 0x00465EC0, EDX | SAVED),
    port("weapon_info_is_melee", 0x00B72740, ECX | EDX | SAVED),
    port("weapon_class_is_melee", 0x00B72780, ECX | EDX | SAVED),
    port("vehicle_weapon_mount_find", 0x00B6B4A0, SAVED),
    port("audio_play_foley_2d", 0x00465C40, SAVED),
    port("audio_play_foley_2d_tagged", 0x00465CD0, SAVED),
    port("audio_play_foley_3d", 0x00465D40, SAVED),
    port("audio_play_voice_line_2d", 0x00465D70, SAVED),
    port("audio_prepare", 0x00465B00, SAVED),
    port("audio_set_listener", 0x0047E7C0, SAVED),
    port("radio_vehicle_station_stop", 0x0048D900, SAVED),
    port("weapon_target_lock_clear", 0x00B6B460, SAVED),
    port("weapon_remove_reserve_ammo", 0x009C3E50, SAVED),
    port("audio_cue_table_add", 0x00481D20, SAVED),
    port("radio_track_find_by_name", 0x0048F780, SAVED),
    port("radio_station_find_by_name", 0x004904F0, SAVED),
    port("radio_dj_voice_stop", 0x0048D970, SAVED),
    port("vehicle_radio_turn_off", 0x0048E330, SAVED),
    port("vehicle_radio_block", 0x00490EB0, SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_weapons_audio: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_weapons_audio: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_weapons_audio: {}: the original changed {} ({:#x} -> {:#x})",
                PORTS[id].name,
                REG_NAMES[i],
                inp.get(i),
                out.get(i)
            ));
        }
    }
}

/// Run the original copy on the caller's registers (and stack argument); returns its outputs.
fn run_original(id: usize, orig: usize, p: &Pushad, arg: u32) -> Regs {
    let inp = Regs::of(p);
    let mut r = inp;
    unsafe { call_full(orig, &mut r, arg, false) };
    check_preserved(id, &inp, &r);
    r
}

/// The shadow for the register-only pure ports: the original copy's EAX wins on a mismatch.
fn checked_eax(id: usize, p: &Pushad, arg: u32, ours: u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let r = run_original(id, orig, p, arg);
    if r.eax != ours {
        mismatch(id, n, format!("in {:x?} arg {arg:#x}: original {:#x}, Rust {ours:#x}", Regs::of(p), r.eax));
        return r.eax;
    }
    ours
}

// --- shared helpers -------------------------------------------------------------------------------

unsafe fn rw(p: usize) -> u16 {
    unsafe { *(p as *const u16) }
}

unsafe fn ww(p: usize, v: u16) {
    unsafe { *(p as *mut u16) = v }
}

/// `or word ptr [p], v` as one instruction.
unsafe fn or16(p: usize, v: u16) {
    unsafe { std::arch::asm!("or word ptr [{p}], {v:x}", p = in(reg) p, v = in(reg) v as u32, options(nostack)) }
}

/// `and word ptr [p], v` as one instruction.
unsafe fn and16(p: usize, v: u16) {
    unsafe { std::arch::asm!("and word ptr [{p}], {v:x}", p = in(reg) p, v = in(reg) v as u32, options(nostack)) }
}

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// EAX with AL replaced by a bool.
fn lo(eax: u32, al: bool) -> u32 {
    (eax & !0xFF) | al as u32
}

// --- ammo stores ----------------------------------------------------------------------------------
// Entry i of a store (`*store` is the base) is 12 bytes: +0 ammo type, +4 count, +8 bonus. The
// loop runs while i < the signed count at 0x22D7B50 and compares `type->+4` with the hash (EDI).

/// The entry address (base + 12 * i) whose type hash is `hash`, or None.
unsafe fn store_scan(store: u32, hash: u32) -> Option<u32> {
    unsafe {
        let n = rd(AMMO_TYPE_COUNT) as i32;
        if n <= 0 {
            return None;
        }
        let base = rd(store as usize);
        let mut i = 0i32;
        let mut e = base;
        loop {
            let ty = rd(e as usize);
            if rd(ty as usize + 4) == hash {
                return Some(e);
            }
            i += 1;
            e = e.wrapping_add(12);
            if i >= n {
                return None;
            }
        }
    }
}

/// 0xB68EB0 ammo_store_count (ECX = &store, EDI = hash -> EAX entry +4, 0 when absent). The `lea`
/// result is tested for 0 before the field is read.
unsafe fn store_count(store: u32, hash: u32) -> u32 {
    unsafe {
        match store_scan(store, hash) {
            Some(e) if e != 0 => rd(e as usize + 4),
            _ => 0,
        }
    }
}

extern "C" fn store_count_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(STORE_COUNT, &p, 0, unsafe { store_count(p.ecx, p.edi) });
}
regs_thunk!(store_count_thunk, store_count_entry);

/// 0xB68CE0 ammo_store_find (ECX = &store, EDI = hash -> EAX the entry address, 0 when absent;
/// unlike the other two it does not test the address for 0).
unsafe fn store_find(store: u32, hash: u32) -> u32 {
    unsafe { store_scan(store, hash).unwrap_or(0) }
}

extern "C" fn store_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(STORE_FIND, &p, 0, unsafe { store_find(p.ecx, p.edi) });
}
regs_thunk!(store_find_thunk, store_find_entry);

/// The reserve limit of an ammo type: +0x10 in multiplayer (byte 0x2528B48 == 1 and byte
/// 0x2528B49 == 0), else +0xC.
unsafe fn type_limit(ty: u32) -> u32 {
    unsafe {
        if rb(MP_FLAG_A) == 1 && rb(MP_FLAG_B) == 0 { rd(ty as usize + 0x10) } else { rd(ty as usize + 0xC) }
    }
}

/// 0xB68F60 ammo_store_capacity (ECX = &store, EDI = hash -> EAX entry bonus +8 + type limit).
unsafe fn store_capacity(store: u32, hash: u32) -> u32 {
    unsafe {
        match store_scan(store, hash) {
            Some(e) if e != 0 => {
                let ty = rd(e as usize);
                let lim = type_limit(ty);
                rd(e as usize + 8).wrapping_add(lim)
            }
            _ => 0,
        }
    }
}

extern "C" fn store_capacity_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(STORE_CAPACITY, &p, 0, unsafe { store_capacity(p.ecx, p.edi) });
}
regs_thunk!(store_capacity_thunk, store_capacity_entry);

// --- 0xB69020 weapon_total_ammo (ESI = instance, stdcall (store, u8 alt) -> EAX) ----------------

/// 0 without an instance; the ammo type is info (+8) +0xD8 for alt, else +0xD4; no type gives the
/// clip +0x10, else clip + the store's count of the type's hash (type +4). The alt path still
/// adds the primary clip (+0x10), as the original does.
unsafe fn total_ammo(inst: u32, store: u32, alt: u8) -> u32 {
    unsafe {
        if inst == 0 {
            return 0;
        }
        let info = rd(inst as usize + 8);
        let ty = rd(info as usize + if alt != 0 { 0xD8 } else { 0xD4 });
        if ty == 0 {
            return rd(inst as usize + 0x10);
        }
        let n = store_count(store, rd(ty as usize + 4));
        rd(inst as usize + 0x10).wrapping_add(n)
    }
}

extern "C" fn total_ammo_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (store, alt) = unsafe { (*args, *args.add(1)) };
    let ours = unsafe { total_ammo(p.esi, store, alt as u8) };
    let Some((n, orig)) = shadowing(TOTAL_AMMO) else {
        r.eax = ours;
        return;
    };
    let inp = Regs::of(&p);
    let mut out = inp;
    unsafe { call_full2(orig, &mut out, store, alt) };
    check_preserved(TOTAL_AMMO, &inp, &out);
    if out.eax != ours {
        mismatch(TOTAL_AMMO, n, format!("esi {:#x} store {store:#x} alt {alt:#x}: original {:#x}, Rust {ours:#x}", p.esi, out.eax));
        r.eax = out.eax;
    } else {
        r.eax = ours;
    }
}
regs_thunk!(total_ammo_thunk, total_ammo_entry, "ret 8");

// --- 0xB73B70 weapon_info_index (EAX = info -> EAX) ----------------------------------------------

/// (info - base) / 0x498 for base <= info < base + count * 0x498 (unsigned, the product wraps
/// at 32 bits like the `imul`): the quotient comes from `imul 0x6F74AE27; sar edx, 9` plus the
/// sign bit, as in the original. Null, below the table or above it: `or al, 0xFF` (the rest of
/// EAX stays the pointer).
unsafe fn info_index(info: u32) -> u32 {
    unsafe {
        let invalid = info | 0xFF;
        if info == 0 {
            return invalid;
        }
        let base = rd(WEAPON_INFOS);
        if info < base {
            return invalid;
        }
        let end = rd(WEAPON_INFO_COUNT).wrapping_mul(0x498).wrapping_add(base);
        if info >= end {
            return invalid;
        }
        let d = info.wrapping_sub(base) as i32 as i64;
        let hi = ((d * 0x6F74_AE27i64) >> 32) as i32;
        let q = hi >> 9;
        (((q as u32) >> 31) as i32).wrapping_add(q) as u32
    }
}

extern "C" fn info_index_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(INFO_INDEX, &p, 0, unsafe { info_index(p.eax) });
}
regs_thunk!(info_index_thunk, info_index_entry);

// --- ammo-flag tests on a weapon info -------------------------------------------------------------

/// Does the ammo type at `info + off` exist and have a flag of `mask` in its low flag byte (+8)?
unsafe fn type_has(info: u32, off: u32, mask: u8) -> bool {
    unsafe {
        let ty = rd(info as usize + off as usize);
        ty != 0 && rb(ty as usize + 8) & mask != 0
    }
}

/// 0xB72F80 weapon_info_is_lethal (ECX = info, cdecl (u8 melee_only) -> AL or EAX). Null: AL = 0;
/// "lethal melee" (+0x10 & 0x400): AL = 1; melee_only: AL = 0 (the upper bytes of EAX stay the
/// caller's in these three cases); else the full EAX is 1 when the primary (+0xD4) or alt (+0xD8)
/// ammo type has flag 4, else 0.
unsafe fn is_lethal(eax: u32, info: u32, melee_only: u8) -> u32 {
    unsafe {
        if info == 0 {
            return lo(eax, false);
        }
        if rd(info as usize + 0x10) & 0x400 != 0 {
            return lo(eax, true);
        }
        if melee_only != 0 {
            return lo(eax, false);
        }
        (type_has(info, 0xD4, 4) || type_has(info, 0xD8, 4)) as u32
    }
}

extern "C" fn is_lethal_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let arg = unsafe { *args };
    r.eax = checked_eax(IS_LETHAL, &p, arg, unsafe { is_lethal(p.eax, p.ecx, arg as u8) });
}
regs_thunk!(is_lethal_thunk, is_lethal_entry);

/// 0xB72840 / 0xB728A0 (ECX = info -> full EAX 0 / 1): null gives 0; the primary or alt ammo type
/// has the flag.
unsafe fn uses_flag(info: u32, mask: u8) -> u32 {
    unsafe { (info != 0 && (type_has(info, 0xD4, mask) || type_has(info, 0xD8, mask))) as u32 }
}

extern "C" fn uses_sewage_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(USES_SEWAGE, &p, 0, unsafe { uses_flag(p.ecx, 0x80) });
}
regs_thunk!(uses_sewage_thunk, uses_sewage_entry);

extern "C" fn uses_bullet_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(USES_BULLET, &p, 0, unsafe { uses_flag(p.ecx, 8) });
}
regs_thunk!(uses_bullet_thunk, uses_bullet_entry);

/// 0xB72E90 weapon_info_uses_projectile_ammo: EAX (not ECX) = info; the primary ammo type only,
/// flag 0x10. Full EAX 0 / 1.
unsafe fn uses_projectile(info: u32) -> u32 {
    unsafe { (info != 0 && type_has(info, 0xD4, 0x10)) as u32 }
}

extern "C" fn uses_projectile_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(USES_PROJECTILE, &p, 0, unsafe { uses_projectile(p.eax) });
}
regs_thunk!(uses_projectile_thunk, uses_projectile_entry);

// --- 0xB72CA0 weapon_instance_has_ammo (EAX = instance -> AL) -----------------------------------

/// AL = 1 when the clip (+0x10) or the reserve (+0x14) is > 0 (signed); or the info (+8) has a
/// zero Magazine_Size (u16 +0xDC); or the info has flag 0x20 at +0x10 (its low byte) and the alt
/// clip +0x18 or alt reserve +0x1C is > 0. The rest of EAX stays the instance pointer. With a
/// null info and both counts <= 0 the original reads byte [0x10] (a crash): so does this.
unsafe fn has_ammo(inst: u32) -> u32 {
    unsafe {
        let i = inst as usize;
        if (rd(i + 0x10) as i32) > 0 || (rd(i + 0x14) as i32) > 0 {
            return lo(inst, true);
        }
        let info = rd(i + 8);
        if info != 0 && rw(info as usize + 0xDC) == 0 {
            return lo(inst, true);
        }
        if rb(info as usize + 0x10) & 0x20 == 0 {
            return lo(inst, false);
        }
        lo(inst, (rd(i + 0x18) as i32) > 0 || (rd(i + 0x1C) as i32) > 0)
    }
}

extern "C" fn has_ammo_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(HAS_AMMO, &p, 0, unsafe { has_ammo(p.eax) });
}
regs_thunk!(has_ammo_thunk, has_ammo_entry);

// --- 0xB71A60 / 0xB71A30 weapon_info_ai_range_sq / _ai_range (EAX = info -> ST0) ----------------

/// AI_Range_Max (+0xE4) of the info, or the f32 at 0xE5C110 (1.275) for a null info; loaded with
/// `fld dword`, so a signalling NaN comes out quiet.
unsafe fn ai_range_of(info: u32) -> f32 {
    unsafe {
        let bits = if info != 0 { rd(info as usize + 0xE4) } else { rd(AI_RANGE_DEFAULT) };
        f32::from_bits(q32(bits))
    }
}

/// The product of two f32 values is exact at 53 bits, and `fstp dword` rounds it once, so one
/// f32 multiply gives the same bits.
unsafe fn ai_range_sq(info: u32) -> f32 {
    let r = unsafe { ai_range_of(info) };
    r * r
}

extern "C" fn ai_range_sq_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let ours = unsafe { ai_range_sq(p.eax) };
    shadow_st0(AI_RANGE_SQ, &p, ours)
}
regs_thunk!(ai_range_sq_thunk, ai_range_sq_entry);

extern "C" fn ai_range_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let ours = unsafe { ai_range_of(p.eax) };
    shadow_st0(AI_RANGE, &p, ours)
}
regs_thunk!(ai_range_thunk, ai_range_entry);

/// The shadow for a register-only port that returns on ST0 (bits compared).
fn shadow_st0(id: usize, p: &Pushad, ours: f32) -> f32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let inp = Regs::of(p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(id, &inp, &out);
    if theirs.to_bits() != ours.to_bits() {
        mismatch(id, n, format!("in {inp:x?}: original {:#x}, Rust {:#x}", theirs.to_bits(), ours.to_bits()));
        return theirs;
    }
    ours
}

// --- 0xB71C80 weapon_instance_free (EAX = node) --------------------------------------------------

/// Unlink the node from the live list (circular, +0 prev / +4 next, head 0x22D7B6C) and insert it
/// before the head of the free list (0x22D7BC4), in the original's order of reads and writes
/// (so the aliasing cases come out the same, including the fault when the node is the free head).
unsafe fn instance_free(node: u32) {
    unsafe {
        let n = node as usize;
        let mut skip_unlink = false;
        if rd(INST_LIVE) == node {
            let next = rd(n + 4);
            if next == node {
                wd(INST_LIVE, 0);
                skip_unlink = true;
            } else {
                wd(INST_LIVE, next);
            }
        }
        if !skip_unlink {
            let prev = rd(n);
            if prev != 0 {
                wd(prev as usize + 4, rd(n + 4));
            }
            let next = rd(n + 4);
            if next != 0 {
                wd(next as usize, rd(n));
            }
        }
        let f = rd(INST_FREE);
        wd(n, 0);
        wd(n + 4, 0);
        if f == 0 {
            wd(n + 4, node);
            wd(n, node);
            wd(INST_FREE, node);
        } else {
            let fp = rd(f as usize);
            wd(n, fp);
            wd(n + 4, f);
            let fp2 = rd(f as usize);
            wd(fp2 as usize + 4, node);
            wd(f as usize, node);
        }
    }
}

extern "C" fn instance_free_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(INSTANCE_FREE) else {
        unsafe { instance_free(p.eax) };
        return;
    };
    unsafe {
        // Everything the original can write: the heads, and both words of the node, its
        // neighbours, the free head and the free head's predecessor.
        let node = p.eax;
        let mut addrs: Vec<usize> = vec![INST_LIVE, INST_FREE];
        if node != 0 {
            let prev = rd(node as usize);
            let next = rd(node as usize + 4);
            let f = rd(INST_FREE);
            let mut objs = vec![node, prev, next, f];
            if f != 0 {
                objs.push(rd(f as usize));
            }
            for o in objs {
                if o != 0 {
                    addrs.push(o as usize);
                    addrs.push(o as usize + 4);
                }
            }
        }
        let read = |a: &[usize]| -> Vec<u32> { a.iter().map(|&x| rd(x)).collect() };
        let before = read(&addrs);
        let t = run_original(INSTANCE_FREE, orig, &p, 0);
        let theirs = read(&addrs);
        for (&a, &v) in addrs.iter().zip(&before) {
            wd(a, v);
        }
        instance_free(node);
        let ours = read(&addrs);
        if ours != theirs || t.eax != p.eax {
            mismatch(INSTANCE_FREE, n, format!("node {node:#x}: original {theirs:x?}, Rust {ours:x?}"));
            for (&a, &v) in addrs.iter().zip(&theirs) {
                wd(a, v);
            }
        }
    }
}
regs_thunk!(instance_free_thunk, instance_free_entry);

// --- 0x9C3F50 weapon_info_max_reserve (EAX = info, cdecl (u8 alt) -> EAX) -----------------------

/// 0 without an info or an ammo type (+0xD8 for alt, else +0xD4); else the type's limit.
unsafe fn max_reserve(info: u32, alt: u8) -> u32 {
    unsafe {
        if info == 0 {
            return 0;
        }
        let ty = rd(info as usize + if alt != 0 { 0xD8 } else { 0xD4 });
        if ty == 0 {
            return 0;
        }
        type_limit(ty)
    }
}

extern "C" fn max_reserve_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let arg = unsafe { *args };
    r.eax = checked_eax(MAX_RESERVE, &p, arg, unsafe { max_reserve(p.eax, arg as u8) });
}
regs_thunk!(max_reserve_thunk, max_reserve_entry);

// --- audio hash lookups (EDX = &hash -> EAX index or 0xFFFF) -------------------------------------

/// The u16 count is the loop bound; with 0 the answer is 0xFFFF without reading the hash. Else
/// the hash is read once and i = 0.. is compared via `entry(i)` (the address of the entry's first
/// dword); found gives i, not found `or ax, 0xFFFF` on i = n: 0xFFFF.
unsafe fn find_hash(count_addr: usize, hash_ptr: u32, entry: impl Fn(usize) -> usize) -> u32 {
    unsafe {
        let n = rw(count_addr);
        if n == 0 {
            return 0xFFFF;
        }
        let h = rd(hash_ptr as usize);
        let mut i = 0usize;
        loop {
            if rd(entry(i)) == h {
                return i as u32;
            }
            i += 1;
            if i >= n as usize {
                return 0xFFFF;
            }
        }
    }
}

extern "C" fn foley_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { find_hash(FOLEY_COUNT, p.edx, |i| rd(FOLEY_TABLE + i * 4) as usize) };
    r.eax = checked_eax(FOLEY_FIND, &p, 0, ours);
}
regs_thunk!(foley_find_thunk, foley_find_entry);

extern "C" fn music_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { find_hash(MUSIC_COUNT, p.edx, |i| rd(MUSIC_TABLE + i * 4) as usize) };
    r.eax = checked_eax(MUSIC_FIND, &p, 0, ours);
}
regs_thunk!(music_find_thunk, music_find_entry);

extern "C" fn line_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { find_hash(LINE_COUNT, p.edx, |i| LINE_TABLE + (i << 4)) };
    r.eax = checked_eax(LINE_FIND, &p, 0, ours);
}
regs_thunk!(line_find_thunk, line_find_entry);

// --- voice handles ---------------------------------------------------------------------------------
// A handle is a dword: byte 0 = slot (0xFF = none), bits 8.. = generation (0 = none). It is good
// when the generation equals slot +0xB4 and the slot's +0 is not -1.

/// 0x480070 audio_voice_set_volume (ECX = &handle, cdecl (f32) -> AL). `slots` is the slot array
/// base (the live one, or a private copy for the shadow). Early failures (byte 0xFF, generation
/// 0) clear AL of the caller's EAX; the later ones and the success leave the slot offset in EAX
/// (AL = 0 / 1). The volume is stored through `fld` / `fstp` (quiet NaN), the flags word is
/// read BEFORE the volume store and bit 1 is set with a separate store, as the original does.
unsafe fn set_volume_on(slots: usize, eax: u32, h: usize, vol: u32) -> u32 {
    unsafe {
        let al = rb(h);
        if al == 0xFF {
            return lo(eax, false);
        }
        let gen_ = rd(h) >> 8;
        if gen_ == 0 {
            return lo(eax, false);
        }
        let off = al as u32 * SLOT_SIZE as u32;
        let s = slots.wrapping_add(off as usize);
        if gen_ != rd(s + 0xB4) || rd(s) == 0xFFFF_FFFF {
            return lo(off, false);
        }
        let fl = rw(s + 0x196);
        wd(s + 0x24, q32(vol));
        if fl & 0x800 == 0 {
            ww(s + 0x196, fl | 1);
        }
        lo(off, true)
    }
}

extern "C" fn set_volume_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let vol = unsafe { *args };
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(SET_VOLUME) else {
        r.eax = unsafe { set_volume_on(VOICE_SLOTS, p.eax, h, vol) };
        return;
    };
    unsafe {
        // The Rust version runs on a private copy of the slot; the original on the live one.
        let al = rb(h);
        let ok = al != 0xFF && (rd(h) >> 8) != 0;
        let off = al as usize * SLOT_SIZE;
        let live = VOICE_SLOTS.wrapping_add(off);
        let mut buf = [0u8; SLOT_SIZE];
        if ok {
            std::ptr::copy_nonoverlapping(live as *const u8, buf.as_mut_ptr(), SLOT_SIZE);
        }
        let fake = (buf.as_ptr() as usize).wrapping_sub(off);
        let ours = set_volume_on(if ok { fake } else { VOICE_SLOTS }, p.eax, h, vol);
        let t = run_original(SET_VOLUME, orig, &p, vol);
        let b = buf.as_ptr() as usize;
        let same = !ok || (rd(b + 0x24) == rd(live + 0x24) && rw(b + 0x196) == rw(live + 0x196));
        if t.eax != ours || !same {
            mismatch(
                SET_VOLUME,
                n,
                format!("handle {:#x} vol {vol:#x}: original eax {:#x}, Rust {ours:#x}, fields equal {same}", rd(h), t.eax),
            );
        }
        r.eax = t.eax;
    }
}
regs_thunk!(set_volume_thunk, set_volume_entry);

/// 0x466140 audio_voice_get_volume (ECX = &handle -> ST0, EAX as left by the original: the
/// byte at the handle in AL for the early failures, else the slot offset). ECX is preserved.
/// The volume is 1.0 for a stale handle, else the slot's f32 +0x24 (quiet NaN through the
/// `fld` / `fstp` / `fld` chain).
unsafe fn get_volume(eax: u32, h: usize) -> (f32, u32) {
    unsafe {
        let al = rb(h);
        if al == 0xFF {
            return (1.0, (eax & !0xFF) | al as u32);
        }
        let gen_ = rd(h) >> 8;
        if gen_ == 0 {
            return (1.0, (eax & !0xFF) | al as u32);
        }
        let off = al as u32 * SLOT_SIZE as u32;
        let s = VOICE_SLOTS.wrapping_add(off as usize);
        if gen_ != rd(s + 0xB4) || rd(s) == 0xFFFF_FFFF {
            return (1.0, off);
        }
        (f32::from_bits(q32(rd(s + 0x24))), off)
    }
}

extern "C" fn get_volume_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let (ours, eax) = unsafe { get_volume(p.eax, p.ecx as usize) };
    let Some((n, orig)) = shadowing(GET_VOLUME) else {
        r.eax = eax;
        return ours;
    };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(GET_VOLUME, &inp, &out);
    if theirs.to_bits() != ours.to_bits() || out.eax != eax {
        mismatch(
            GET_VOLUME,
            n,
            format!("handle {:#x}: original {:#x} eax {:#x}, Rust {:#x} eax {eax:#x}", unsafe { rd(p.ecx as usize) }, theirs.to_bits(), out.eax, ours.to_bits()),
        );
        r.eax = out.eax;
        return theirs;
    }
    r.eax = eax;
    ours
}
regs_thunk!(get_volume_thunk, get_volume_entry);

// --- 0x47F200 audio_voices_stop_category / 0x47F2E0 audio_voices_mark_dirty_all -----------------

/// Slots whose id (+0) is not -1, or which have flag 0x200 in the word at +0x196, are "in use".
unsafe fn slot_in_use(s: usize) -> bool {
    unsafe { rd(s) != 0xFFFF_FFFF || rw(s + 0x196) & 0x200 != 0 }
}

/// For every in-use slot whose category (+0xA4, dword) equals `cat` (or any slot when `cat` is
/// -1): `or word [+0x198], 0x400` and `and word [+0x194], 0xFF7F`, each a single instruction.
/// Returns the EAX the original ends with.
unsafe fn stop_category_on(slots: usize, cat: u32) -> u32 {
    unsafe {
        for k in 0..SLOT_COUNT {
            let s = slots + k * SLOT_SIZE;
            if slot_in_use(s) && (rd(s + 0xA4) == cat || cat == 0xFFFF_FFFF) {
                or16(s + 0x198, 0x400);
                and16(s + 0x194, 0xFF7F);
            }
        }
    }
    0x00EE_BA90 + 32 * 0x670
}

/// For every in-use slot whose flags word (+0x196) has no 0x800: set bit 1 (load, test, store).
/// Returns the EAX the original ends with.
unsafe fn mark_dirty_on(slots: usize) -> u32 {
    unsafe {
        for k in 0..SLOT_COUNT {
            let s = slots + k * SLOT_SIZE;
            if slot_in_use(s) {
                let fl = rw(s + 0x196);
                if fl & 0x800 == 0 {
                    ww(s + 0x196, fl | 1);
                }
            }
        }
    }
    0xCE00
}

/// Copy the whole slot array, run the Rust version on the copy and the original on the live
/// array, then compare the words the two can write (+0x194, +0x196, +0x198).
fn slots_shadow(id: usize, n: u32, orig: usize, p: &Pushad, ours: impl FnOnce(usize) -> u32) -> u32 {
    let total = SLOT_SIZE * SLOT_COUNT;
    let mut buf = vec![0u8; total];
    unsafe {
        std::ptr::copy_nonoverlapping(VOICE_SLOTS as *const u8, buf.as_mut_ptr(), total);
        let b = buf.as_ptr() as usize;
        let e = ours(b);
        let t = run_original(id, orig, p, 0);
        for k in 0..SLOT_COUNT {
            for f in [0x194usize, 0x196, 0x198] {
                let (a, c) = (rw(b + k * SLOT_SIZE + f), rw(VOICE_SLOTS + k * SLOT_SIZE + f));
                if a != c {
                    mismatch(id, n, format!("slot {k} +{f:#x}: original {c:#x}, Rust {a:#x} (edx {:#x})", p.edx));
                    return t.eax;
                }
            }
        }
        if t.eax != e {
            mismatch(id, n, format!("eax: original {:#x}, Rust {e:#x}", t.eax));
        }
        t.eax
    }
}

extern "C" fn stop_category_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(STOP_CATEGORY) else {
        r.eax = unsafe { stop_category_on(VOICE_SLOTS, p.edx) };
        return;
    };
    r.eax = slots_shadow(STOP_CATEGORY, n, orig, &p, |b| unsafe { stop_category_on(b, p.edx) });
}
regs_thunk!(stop_category_thunk, stop_category_entry);

extern "C" fn mark_dirty_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(MARK_DIRTY) else {
        r.eax = unsafe { mark_dirty_on(VOICE_SLOTS) };
        return;
    };
    r.eax = slots_shadow(MARK_DIRTY, n, orig, &p, |b| unsafe { mark_dirty_on(b) });
}
regs_thunk!(mark_dirty_thunk, mark_dirty_entry);

// --- 0x46ED20 audio_set_category_throttle (ECX = category, cdecl (f32 v)) -----------------------

/// For category < 4 (unsigned): the f32 is stored at `table[category]` exactly when it is
/// ordered and 0 <= v <= 1 (the x87 compares set C0 for v < 0, for 1 < v and for a NaN). A single
/// aligned dword store, no lock. (The original leaves the x87 status word in AX: not a result.)
unsafe fn throttle_on(table: usize, cat: u32, vbits: u32) {
    unsafe {
        if cat >= 4 {
            return;
        }
        let v = f32::from_bits(vbits);
        if v >= 0.0 && v <= 1.0 {
            wd(table + cat as usize * 4, vbits);
        }
    }
}

extern "C" fn throttle_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let vbits = unsafe { *args };
    let Some((n, orig)) = shadowing(THROTTLE_SET) else {
        unsafe { throttle_on(THROTTLE, p.ecx, vbits) };
        return;
    };
    unsafe {
        let mut buf = [0u32; 4];
        std::ptr::copy_nonoverlapping(THROTTLE as *const u32, buf.as_mut_ptr(), 4);
        throttle_on(buf.as_ptr() as usize, p.ecx, vbits);
        run_original(THROTTLE_SET, orig, &p, vbits);
        let live: [u32; 4] = [rd(THROTTLE), rd(THROTTLE + 4), rd(THROTTLE + 8), rd(THROTTLE + 12)];
        if live != buf {
            mismatch(THROTTLE_SET, n, format!("category {} v {vbits:#x}: original {live:x?}, Rust {buf:x?}", p.ecx));
        }
    }
}
regs_thunk!(throttle_thunk, throttle_entry);

// =====================================================================================================
// Second round: voice fades and orientation wrappers, the play-params family, pause / resume.
//
// Voice slots are shared with the audio thread, so everything below that touches a slot keeps the
// original's order of reads and writes, uses volatile accesses for the slot words, and keeps every
// read-modify-write that is a single instruction in the original a single instruction (`or16`,
// `xor16`, `add1_32`, `sub1_32`). None of these originals takes a lock or a critical section.
// Shadow runs follow the pattern at the top of the file: the Rust version runs on a private copy of
// what it writes, the original runs on the live state (its writes always stand), the copy is
// compared with the live bytes afterwards. The play-params writers only touch their own 0x5C-byte
// buffer (the caller's stack), so there the Rust version runs on a scratch buffer. The originals
// of this round are run through `call_regs` (any number of stack arguments); only EAX is compared.
// =====================================================================================================

const MEMSET: usize = 0x00CA_0BC0; // cdecl (dest, value, count)
const SLOT_SET_ORIENTATION: usize = 0x0047_D3A0; // ECX = transform, cdecl (slot *)
const SLOT_UPDATE_MUTE: usize = 0x0047_D780; // ECX = slot -> AL
const AUDIO_PAUSE_CATEGORY: usize = 0x0048_1E90; // cdecl (u8)
const WINDOW_ACTIVE: usize = 0x0252_A406; // byte
const MASTER_MUTED: usize = 0x0252_6AFB; // byte
const WINDOW_PAUSED: usize = 0x0252_6AD7; // byte
const PAUSE_DEPTH: usize = 0x0252_6B44; // i32
const MASTER_VOICE: usize = 0x00F5_2E30; // pointer to an object whose vtable +0x28 is a SetVolume-like call
const SLOTS_END_EAX: u32 = 0x00EE_BA90 + (SLOT_COUNT * SLOT_SIZE) as u32; // EAX at the end of the pause / resume loops
const SLOT_WORDS: usize = SLOT_SIZE / 4;
const ONE_BITS: u32 = 0x3F80_0000;

unsafe fn vr8(p: usize) -> u8 {
    unsafe { std::ptr::read_volatile(p as *const u8) }
}
unsafe fn vr16(p: usize) -> u16 {
    unsafe { std::ptr::read_volatile(p as *const u16) }
}
unsafe fn vw16(p: usize, v: u16) {
    unsafe { std::ptr::write_volatile(p as *mut u16, v) }
}
unsafe fn vr32(p: usize) -> u32 {
    unsafe { std::ptr::read_volatile(p as *const u32) }
}
unsafe fn vw32(p: usize, v: u32) {
    unsafe { std::ptr::write_volatile(p as *mut u32, v) }
}

/// `xor word ptr [p], v` as one instruction.
unsafe fn xor16(p: usize, v: u16) {
    unsafe { std::arch::asm!("xor word ptr [{p}], {v:x}", p = in(reg) p, v = in(reg) v as u32, options(nostack)) }
}

/// `add dword ptr [p], 1` as one instruction.
unsafe fn add1_32(p: usize) {
    unsafe { std::arch::asm!("add dword ptr [{p}], 1", p = in(reg) p, options(nostack)) }
}

/// `sub dword ptr [p], 1` as one instruction; true when the result is negative (SF).
unsafe fn sub1_32(p: usize) -> bool {
    let neg: u8;
    unsafe { std::arch::asm!("sub dword ptr [{p}], 1", "sets {n}", p = in(reg) p, n = out(reg_byte) neg, options(nostack)) };
    neg != 0
}

/// Run the original copy with the caller's registers and cdecl / stdcall stack arguments;
/// returns its EAX.
unsafe fn run_orig(orig: usize, p: &Pushad, args: &[u32]) -> u32 {
    unsafe { call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, args) }
}

/// What the handle check at the top of every `audio_voice_*` wrapper finds. The reads are in the
/// original's order: the byte, the dword (generation), then the slot's +0xB4 and +0.
enum H {
    /// Slot byte 0xFF (AL = the byte).
    Ff(u8),
    /// Generation 0 (AL = the slot byte).
    Gen0(u8),
    /// Stale or free slot: the slot index.
    Stale(u32),
    /// Good: the slot index.
    Good(u32),
}

unsafe fn hcheck(slots: usize, h: usize) -> H {
    unsafe {
        let al = rb(h);
        if al == 0xFF {
            return H::Ff(al);
        }
        let gen_ = rd(h) >> 8;
        if gen_ == 0 {
            return H::Gen0(al);
        }
        let s = slots.wrapping_add(al as usize * SLOT_SIZE);
        if gen_ != vr32(s + 0xB4) || vr32(s) == 0xFFFF_FFFF {
            return H::Stale(al as u32);
        }
        H::Good(al as u32)
    }
}

/// The slot byte of a handle when it passes the first two checks (the Rust version may touch the
/// slot then), None otherwise.
unsafe fn handle_idx(h: usize) -> Option<usize> {
    unsafe {
        let al = rb(h);
        if al == 0xFF || rd(h) >> 8 == 0 { None } else { Some(al as usize) }
    }
}

/// A private copy of one slot, addressed as if the whole array were there.
struct SlotCopy {
    buf: [u32; SLOT_WORDS],
    idx: usize,
}

impl SlotCopy {
    unsafe fn new(idx: usize) -> SlotCopy {
        let mut buf = [0u32; SLOT_WORDS];
        unsafe { std::ptr::copy_nonoverlapping((VOICE_SLOTS + idx * SLOT_SIZE) as *const u32, buf.as_mut_ptr(), SLOT_WORDS) };
        SlotCopy { buf, idx }
    }
    /// The array base that puts slot `idx` at the copy.
    fn base(&self) -> usize {
        self.slot().wrapping_sub(self.idx * SLOT_SIZE)
    }
    fn slot(&self) -> usize {
        self.buf.as_ptr() as usize
    }
    /// The first (offset, copy dword, live dword) of the (offset, length) ranges that differ.
    unsafe fn diff(&self, fields: &[(usize, usize)]) -> Option<(usize, u32, u32)> {
        let live = VOICE_SLOTS + self.idx * SLOT_SIZE;
        for &(off, len) in fields {
            let a = unsafe { std::slice::from_raw_parts((self.slot() + off) as *const u8, len) };
            let b = unsafe { std::slice::from_raw_parts((live + off) as *const u8, len) };
            if a != b {
                let (x, y) = unsafe { (std::ptr::read_unaligned((self.slot() + off) as *const u32), std::ptr::read_unaligned((live + off) as *const u32)) };
                return Some((off, x, y));
            }
        }
        None
    }
}

/// The shadow for a voice writer: the Rust version (`ours(array base)` -> the EAX the original
/// ends with, compared when `cmp_eax`) runs on a private copy of the slot the handle / index names,
/// the original on the live slot, then the listed (offset, length) ranges are compared. Returns
/// the original's EAX; its writes always stand.
unsafe fn shadow_slot_op(
    id: usize,
    n: u32,
    orig: usize,
    p: &Pushad,
    args: &[u32],
    idx: Option<usize>,
    fields: &[(usize, usize)],
    cmp_eax: bool,
    ours: impl FnOnce(usize) -> u32,
) -> u32 {
    unsafe {
        match idx {
            Some(i) if i < SLOT_COUNT => {
                let sc = SlotCopy::new(i);
                let e = ours(sc.base());
                let t = run_orig(orig, p, args);
                if let Some((off, a, b)) = sc.diff(fields) {
                    mismatch(id, n, format!("in {:x?} args {args:x?} slot {i} +{off:#x}: original {b:#x}, Rust {a:#x}", Regs::of(p)));
                } else if cmp_eax && t != e {
                    mismatch(id, n, format!("in {:x?} args {args:x?}: original eax {t:#x}, Rust {e:#x}", Regs::of(p)));
                }
                t
            }
            Some(_) => run_orig(orig, p, args),
            None => {
                let e = ours(VOICE_SLOTS);
                let t = run_orig(orig, p, args);
                if cmp_eax && t != e {
                    mismatch(id, n, format!("in {:x?} args {args:x?}: original eax {t:#x}, Rust {e:#x}", Regs::of(p)));
                }
                t
            }
        }
    }
}

// --- 0x47C740 audio_voice_slot_fade_out (EAX = slot index, EDX = fade ms) -------------------------

/// Clear the fade-in bit and set the fade-out bit of the flags word (+0x196); when fade-out was not
/// already set, +0xC4 = 0.0. The rate is +0x1C x 0.5 for ms < 32, else +0x1C / (ms >> 5) (an
/// unsigned `fild` with the 2^32 fix-up, which the shifted count never needs). It is stored to
/// +0xA8 through an f32 round trip, only when the current +0xA8 is ordered and smaller (`fcomp`
/// then `test ah, 5 / jp`: only C0 alone, current < new, falls through to the store). Reads and
/// writes: flags, (+0xC4), +0x1C, flags, +0xA8, (+0xA8). The x87 status word the original leaves
/// in AX is not reproduced.
unsafe fn slot_fade_out_on(slots: usize, idx: u32, ms: u32) {
    unsafe {
        let s = slots.wrapping_add(idx.wrapping_mul(SLOT_SIZE as u32) as usize);
        let fl = vr16(s + 0x196) as u32;
        if fl & 0x80 == 0 {
            vw32(s + 0xC4, 0);
        }
        let cur = f32::from_bits(vr32(s + 0x1C)) as f64;
        vw16(s + 0x196, ((fl & 0xFEFF) | 0x80) as u16);
        let v = if ms < 0x20 {
            cur * std::ptr::read_unaligned(0x00E5_BF70 as *const f64)
        } else {
            let k = (ms >> 5) as i32;
            let mut d = k as f64;
            if k < 0 {
                d += std::ptr::read_unaligned(0x00E5_BFD8 as *const f32) as f64;
            }
            cur / d
        };
        let new = v as f32;
        let old = f32::from_bits(vr32(s + 0xA8));
        if old < new {
            vw32(s + 0xA8, new.to_bits());
        }
    }
}

const FADE_FIELDS: &[(usize, usize)] = &[(0xA8, 4), (0xC4, 4), (0x196, 2)];

extern "C" fn slot_fade_out_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(SLOT_FADE_OUT) else {
        unsafe { slot_fade_out_on(VOICE_SLOTS, p.eax, p.edx) };
        return;
    };
    unsafe {
        let idx = Some(p.eax as usize);
        shadow_slot_op(SLOT_FADE_OUT, n, orig, &p, &[], idx, FADE_FIELDS, false, |b| {
            slot_fade_out_on(b, p.eax, p.edx);
            0
        });
    }
}
regs_thunk!(slot_fade_out_thunk, slot_fade_out_entry);

// --- 0x465F70 audio_voice_fade_out / 0x47FF80 audio_voice_fade_out_2 (ECX = &handle, cdecl (ms)) ---

unsafe fn fade_out_on(slots: usize, h: usize, ms: u32) {
    unsafe {
        if let H::Good(idx) = hcheck(slots, h) {
            slot_fade_out_on(slots, idx, ms);
        }
    }
}

fn fade_out_common(id: usize, r: &mut Pushad, args: *const u32) {
    let p = *r;
    let ms = unsafe { *args };
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(id) else {
        unsafe { fade_out_on(VOICE_SLOTS, h, ms) };
        return;
    };
    unsafe {
        let idx = handle_idx(h);
        shadow_slot_op(id, n, orig, &p, &[ms], idx, FADE_FIELDS, false, |b| {
            fade_out_on(b, h, ms);
            0
        });
    }
}

extern "C" fn fade_out_entry(r: &mut Pushad, args: *const u32) {
    fade_out_common(FADE_OUT, r, args);
}
regs_thunk!(fade_out_thunk, fade_out_entry);

extern "C" fn fade_out2_entry(r: &mut Pushad, args: *const u32) {
    fade_out_common(FADE_OUT2, r, args);
}
regs_thunk!(fade_out2_thunk, fade_out2_entry);

// --- 0x465FD0 audio_voice_set_orientation / 0x4803B0 _2 (ECX = &handle, cdecl (transform)) --------

/// The handle check, then audio_voice_slot_set_orientation (0x47D3A0, ECX = transform, the slot
/// pointer on the stack) called by address; EAX is whatever it leaves.
unsafe fn set_orientation_on(slots: usize, eax: u32, h: usize, transform: u32) -> u32 {
    unsafe {
        match hcheck(slots, h) {
            H::Good(idx) => {
                let s = slots.wrapping_add(idx as usize * SLOT_SIZE) as u32;
                call_regs(SLOT_SET_ORIENTATION, s, transform, 0, 0, 0, &[s])
            }
            _ => eax,
        }
    }
}

const ORIENT_FIELDS: &[(usize, usize)] = &[(0x44, 0x18)];

fn orient_common(id: usize, r: &mut Pushad, args: *const u32) {
    let p = *r;
    let transform = unsafe { *args };
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(id) else {
        r.eax = unsafe { set_orientation_on(VOICE_SLOTS, p.eax, h, transform) };
        return;
    };
    unsafe {
        let idx = handle_idx(h);
        r.eax = shadow_slot_op(id, n, orig, &p, &[transform], idx, ORIENT_FIELDS, false, |b| set_orientation_on(b, p.eax, h, transform));
    }
}

extern "C" fn orient_entry(r: &mut Pushad, args: *const u32) {
    orient_common(SET_ORIENT, r, args);
}
regs_thunk!(orient_thunk, orient_entry);

extern "C" fn orient2_entry(r: &mut Pushad, args: *const u32) {
    orient_common(SET_ORIENT2, r, args);
}
regs_thunk!(orient2_thunk, orient2_entry);

// --- 0x4660B0 audio_voice_is_2d (ECX = &handle -> AL) ------------------------------------------

/// AL = 1 when the handle is good and the 0x800 (3D) bit of the flags word is clear; the rest of
/// EAX as the original leaves it (the early failures keep the caller's EAX with AL cleared, the
/// stale case the slot offset with AL cleared, the good case the slot offset's upper word).
unsafe fn is_2d(eax: u32, h: usize) -> u32 {
    unsafe {
        match hcheck(VOICE_SLOTS, h) {
            H::Ff(_) | H::Gen0(_) => lo(eax, false),
            H::Stale(idx) => lo(idx * SLOT_SIZE as u32, false),
            H::Good(idx) => {
                let off = idx * SLOT_SIZE as u32;
                let fl = rw(VOICE_SLOTS + off as usize + 0x196) as u32;
                (off & 0xFFFF_0000) | (!(fl >> 11) & 1)
            }
        }
    }
}

extern "C" fn is_2d_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(IS_2D, &p, 0, unsafe { is_2d(p.eax, p.ecx as usize) });
}
regs_thunk!(is_2d_thunk, is_2d_entry);

// --- 0x466180 audio_voice_get_param_30 (ECX = &handle -> ST0) ----------------------------------

/// ST0 = the slot's f32 +0x30 (quiet NaN through the `fld` / `fstp` / `fld` chain), 0.0 for a
/// stale handle. ECX is preserved (pushed and popped). EAX as in `get_volume`.
unsafe fn get_param_30(eax: u32, h: usize) -> (f32, u32) {
    unsafe {
        match hcheck(VOICE_SLOTS, h) {
            H::Ff(al) | H::Gen0(al) => (0.0, (eax & !0xFF) | al as u32),
            H::Stale(idx) => (0.0, idx * SLOT_SIZE as u32),
            H::Good(idx) => {
                let off = idx * SLOT_SIZE as u32;
                (f32::from_bits(q32(rd(VOICE_SLOTS + off as usize + 0x30))), off)
            }
        }
    }
}

extern "C" fn get_param_30_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let (ours, eax) = unsafe { get_param_30(p.eax, p.ecx as usize) };
    let Some((n, orig)) = shadowing(GET_P30) else {
        r.eax = eax;
        return ours;
    };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(GET_P30, &inp, &out);
    if theirs.to_bits() != ours.to_bits() || out.eax != eax {
        mismatch(
            GET_P30,
            n,
            format!("handle {:#x}: original {:#x} eax {:#x}, Rust {:#x} eax {eax:#x}", unsafe { rd(p.ecx as usize) }, theirs.to_bits(), out.eax, ours.to_bits()),
        );
        r.eax = out.eax;
        return theirs;
    }
    r.eax = eax;
    ours
}
regs_thunk!(get_param_30_thunk, get_param_30_entry);

// --- 0x4800C0 audio_voice_set_param_28 / 0x480170 audio_voice_set_param_30 (ECX = &handle, cdecl (f32) -> AL)

/// `or word [+0x196], flag` (one instruction), then the f32 is stored to `field` (+0x28 / +0x30,
/// quiet NaN), then the flags word is re-read and, when the 0x800 bit is clear and the +0x194 bit
/// `shift` (3 / 4) is set, bit 1 is set with a separate store. AL = 1 (rest of EAX: the slot
/// offset); a stale handle gives AL = 0 as in `set_volume_on`.
unsafe fn set_param_on(slots: usize, eax: u32, h: usize, vbits: u32, field: usize, flag: u16, shift: u32) -> u32 {
    unsafe {
        let idx = match hcheck(slots, h) {
            H::Ff(_) | H::Gen0(_) => return lo(eax, false),
            H::Stale(idx) => return lo(idx * SLOT_SIZE as u32, false),
            H::Good(idx) => idx,
        };
        let off = idx * SLOT_SIZE as u32;
        let s = slots.wrapping_add(off as usize);
        or16(s + 0x196, flag);
        let b194 = vr8(s + 0x194) as u32;
        vw32(s + field, q32(vbits));
        let fl = vr16(s + 0x196) as u32;
        let cl = (b194 >> shift) & 1;
        if (!(fl >> 11) & cl) & 0xFF != 0 {
            vw16(s + 0x196, (fl | 1) as u16);
        }
        lo(off, true)
    }
}

const PARAM_FIELDS: &[(usize, usize)] = &[(0x28, 0xC), (0x196, 2)];

fn set_param_common(id: usize, r: &mut Pushad, args: *const u32, field: usize, flag: u16, shift: u32) {
    let p = *r;
    let vbits = unsafe { *args };
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(id) else {
        r.eax = unsafe { set_param_on(VOICE_SLOTS, p.eax, h, vbits, field, flag, shift) };
        return;
    };
    unsafe {
        let idx = handle_idx(h);
        r.eax = shadow_slot_op(id, n, orig, &p, &[vbits], idx, PARAM_FIELDS, true, |b| set_param_on(b, p.eax, h, vbits, field, flag, shift));
    }
}

extern "C" fn set_param_28_entry(r: &mut Pushad, args: *const u32) {
    set_param_common(SET_P28, r, args, 0x28, 2, 3);
}
regs_thunk!(set_param_28_thunk, set_param_28_entry);

extern "C" fn set_param_30_entry(r: &mut Pushad, args: *const u32) {
    set_param_common(SET_P30, r, args, 0x30, 4, 4);
}
regs_thunk!(set_param_30_thunk, set_param_30_entry);

// --- 0x47CED0 audio_voice_slot_fade_in (ECX = slot index, cdecl (ms, f32 start volume)) ----------

/// Unless the fade-in bit (0x100) is set: +0x1C = start (f32 round trip), the flags word with 0x80
/// cleared and 0x100 set, +0xC4 = 0.0, +0xA8 = (1 - start) / (ms / 32) with the signed divide by 32
/// of `cdq / and 0x1f / add / sar 5` and `fidiv` (division by zero gives the x87 result: +-inf or
/// the default NaN, as SSE does). The writes come in that order.
unsafe fn slot_fade_in_on(slots: usize, idx: u32, ms: u32, start: u32) {
    unsafe {
        let s = slots.wrapping_add(idx.wrapping_mul(SLOT_SIZE as u32) as usize);
        let fl = vr16(s + 0x196) as u32;
        if fl & 0x100 != 0 {
            return;
        }
        let st = f32::from_bits(start) as f64;
        vw32(s + 0x1C, q32(start));
        vw16(s + 0x196, ((fl & 0xFF7F) | 0x100) as u16);
        vw32(s + 0xC4, 0);
        let m = ms as i32;
        let k = m.wrapping_add(((m >> 31) as u32 & 0x1F) as i32) >> 5;
        let rate = ((1.0 - st) / k as f64) as f32;
        vw32(s + 0xA8, rate.to_bits());
    }
}

const FADE_IN_FIELDS: &[(usize, usize)] = &[(0x1C, 4), (0xA8, 4), (0xC4, 4), (0x196, 2)];

extern "C" fn slot_fade_in_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (ms, start) = unsafe { (*args, *args.add(1)) };
    let Some((n, orig)) = shadowing(FADE_IN) else {
        unsafe { slot_fade_in_on(VOICE_SLOTS, p.ecx, ms, start) };
        return;
    };
    unsafe {
        let idx = Some(p.ecx as usize);
        shadow_slot_op(FADE_IN, n, orig, &p, &[ms, start], idx, FADE_IN_FIELDS, false, |b| {
            slot_fade_in_on(b, p.ecx, ms, start);
            0
        });
    }
}
regs_thunk!(slot_fade_in_thunk, slot_fade_in_entry);

// --- the audio_play_params_init family (ESI = params, 0x5C bytes; EDI = &position for the 3D ones) --
// Every variant zeroes the 0x5C bytes with memset (0xCA0BC0, called by address with the same cdecl
// arguments), then stores its fields; the f32 arguments go through `fld` / `fstp` (quiet NaN).
// EAX = ESI at the end (the no-argument variant ends with memset's return in EAX). Only the
// caller's buffer is written, so the shadow runs the Rust version on a scratch buffer and the
// original on the real one, then compares the 0x5C bytes.

unsafe fn pm(esi: usize) -> u32 {
    unsafe { call_regs(MEMSET, 0, 0, 0, 0, 0, &[esi as u32, 0, 0x5C]) }
}

unsafe fn p32(esi: usize, off: usize, v: u32) {
    unsafe { wd(esi + off, v) }
}

unsafe fn p16(esi: usize, off: usize, v: u16) {
    unsafe { ww(esi + off, v) }
}

/// +0x1C..+0x24 = the three dwords at EDI; then `or word [+0x58], 1` (the positional flag).
unsafe fn ppos(esi: usize, edi: usize) {
    unsafe {
        p32(esi, 0x1C, rd(edi));
        p32(esi, 0x20, rd(edi + 4));
        p32(esi, 0x24, rd(edi + 8));
        p16(esi, 0x58, rw(esi + 0x58) | 1);
    }
}

unsafe fn parg(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

type Build = unsafe fn(usize, usize, *const u32) -> u32;

/// 0x464860 (no arguments, plain ret): the "empty" params: cue 0xFFFF, type -1.
unsafe fn build_default(esi: usize, _edi: usize, _a: *const u32) -> u32 {
    unsafe {
        let m = pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, 0xFFFF);
        p32(esi, 0x18, 0xFFFF_FFFF);
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x4C, 0);
        p32(esi, 0x54, 0);
        m
    }
}

/// 0x464920 (u16 cue, type +0x18, tag +0x4C) ret 0xC.
unsafe fn build_basic(esi: usize, _edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        esi as u32
    }
}

/// 0x464970 (u16 cue, type +0x18, tag +0x4C, f32 volume +0xC) ret 0x10.
unsafe fn build_volume(esi: usize, _edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0xC, q32(parg(a, 3)));
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        esi as u32
    }
}

/// 0x4649C0 (u16 cue, type +0x18, tag +0x4C, variant +0x28) ret 0x10.
unsafe fn build_variant(esi: usize, _edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x28, parg(a, 3));
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        esi as u32
    }
}

/// 0x464A10 (u16 cue, tag +0x4C, variant +0x28, f32 volume) ret 0x10; type 0.
unsafe fn build_variant_volume(esi: usize, _edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0xC, q32(parg(a, 3)));
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, 0);
        p32(esi, 0x4C, parg(a, 1));
        p32(esi, 0x28, parg(a, 2));
        esi as u32
    }
}

/// 0x464A60 (u16 cue, type, tag, f32 volume, u16 +2, dword +4) ret 0x18.
unsafe fn build_ext(esi: usize, _edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0x10, ONE_BITS);
        p32(esi, 0xC, q32(parg(a, 3)));
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x18, parg(a, 1));
        p16(esi, 2, parg(a, 4) as u16);
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x54, 0);
        p32(esi, 0x4C, parg(a, 2));
        p32(esi, 4, parg(a, 5));
        esi as u32
    }
}

/// 0x464AC0 (u16 cue, type, tag) ret 0xC; position at EDI.
unsafe fn build_3d(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p16(esi, 0x32, 1);
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        ppos(esi, edi);
        esi as u32
    }
}

/// 0x464B30 (u16 cue, tag, u16 +0x30) ret 0xC; type 1, position at EDI.
unsafe fn build_3d_chan(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0x32, 1);
        p32(esi, 0x18, 1);
        p32(esi, 0x4C, parg(a, 1));
        p32(esi, 0x28, 0);
        p32(esi, 0x54, 0);
        ppos(esi, edi);
        p16(esi, 0x30, parg(a, 2) as u16);
        esi as u32
    }
}

/// 0x464BA0 (u16 cue, tag, u16 +0x30, u16 +0x32) ret 0x10; type 1, position at EDI.
unsafe fn build_3d_chan2(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0x32, 1);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x18, 1);
        p32(esi, 0x4C, parg(a, 1));
        p32(esi, 0x28, 0);
        p32(esi, 0x54, 0);
        ppos(esi, edi);
        p16(esi, 0x30, parg(a, 2) as u16);
        p16(esi, 0x32, parg(a, 3) as u16);
        esi as u32
    }
}

/// 0x464C10 (u16 cue, tag, u16 +0x30, u16 +0x32, u8 flag -> +0x58 bit 6) ret 0x14; type 1, position
/// at EDI. The +0x58 word: `((flag << 6) ^ lowbyte) & 0x40) ^ word) | 1`, as the setne / shl / xor /
/// and / xor / or chain computes it.
unsafe fn build_3d_chan_flag(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p32(esi, 0x4C, parg(a, 1));
        p16(esi, 0x32, 1);
        p16(esi, 0x30, 0xFFFF);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x18, 1);
        p32(esi, 0x28, 0);
        p32(esi, 0x54, 0);
        p32(esi, 0x1C, rd(edi));
        p32(esi, 0x20, rd(edi + 4));
        p16(esi, 0x30, parg(a, 2) as u16);
        p32(esi, 0x24, rd(edi + 8));
        p16(esi, 0x32, parg(a, 3) as u16);
        let flag = (parg(a, 4) as u8 != 0) as u8;
        let w = rw(esi + 0x58);
        let t = (((flag << 6) ^ (w as u8)) & 0x40) as u16;
        p16(esi, 0x58, (t ^ w) | 1);
        esi as u32
    }
}

/// 0x464CA0 (u16 cue, type, tag, f32 volume) ret 0x10; position at EDI.
unsafe fn build_3d_volume(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p16(esi, 0x32, 1);
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        p32(esi, 0xC, q32(parg(a, 3)));
        ppos(esi, edi);
        esi as u32
    }
}

/// 0x464D10 (u16 cue, type, tag, f32 volume, u16 +2, dword +4) ret 0x18; position at EDI.
unsafe fn build_3d_ext(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p16(esi, 0x32, 1);
        p32(esi, 0x18, parg(a, 1));
        p32(esi, 0x4C, parg(a, 2));
        p32(esi, 0x28, 0);
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        p32(esi, 0xC, q32(parg(a, 3)));
        ppos(esi, edi);
        p16(esi, 2, parg(a, 4) as u16);
        p32(esi, 4, parg(a, 5));
        esi as u32
    }
}

/// 0x464D90 (u16 cue, tag, variant +0x28) ret 0xC; type 0, position at EDI.
unsafe fn build_3d_variant(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x28, 0);
        p16(esi, 0x32, 1);
        p32(esi, 0x4C, parg(a, 1));
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, 0);
        ppos(esi, edi);
        p32(esi, 0x28, parg(a, 2));
        esi as u32
    }
}

/// 0x464E00 (u16 cue, tag, variant +0x28, f32 volume) ret 0x10; type 0, position at EDI.
unsafe fn build_3d_variant_volume(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x28, 0);
        p16(esi, 0x32, 1);
        p32(esi, 0x4C, parg(a, 1));
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        p32(esi, 0x18, 0);
        ppos(esi, edi);
        p32(esi, 0xC, q32(parg(a, 3)));
        p32(esi, 0x28, parg(a, 2));
        esi as u32
    }
}

/// 0x464E70 (u16 cue, type +0x18, tag +0x4C, variant +0x28, f32 volume, u16 +2, dword +4) ret
/// 0x1C; position at EDI.
unsafe fn build_3d_variant_ext(esi: usize, edi: usize, a: *const u32) -> u32 {
    unsafe {
        pm(esi);
        p32(esi, 0xC, ONE_BITS);
        p32(esi, 0x10, ONE_BITS);
        p16(esi, 0, parg(a, 0) as u16);
        p32(esi, 0x4C, parg(a, 2));
        p32(esi, 0x28, 0);
        p16(esi, 0x32, 1);
        p32(esi, 0x18, parg(a, 1));
        p16(esi, 0x30, 0xFFFF);
        p32(esi, 0x54, 0);
        ppos(esi, edi);
        p32(esi, 0xC, q32(parg(a, 4)));
        p32(esi, 0x28, parg(a, 3));
        p16(esi, 2, parg(a, 5) as u16);
        p32(esi, 4, parg(a, 6));
        esi as u32
    }
}

/// Run one params variant: live, or (while shadowed) on a scratch buffer against the original on
/// the real one. The original's bytes stand.
unsafe fn params_run(id: usize, p: &Pushad, a: *const u32, nargs: usize, build: Build) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return build(p.esi as usize, p.edi as usize, a) };
        let mut scratch = [0xCCCC_CCCCu32; 0x5C / 4];
        let sp = scratch.as_mut_ptr() as usize;
        let ours = build(sp, p.edi as usize, a);
        let ours = if ours == sp as u32 { p.esi } else { ours };
        let args = std::slice::from_raw_parts(a, nargs);
        let theirs = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, args);
        let live = std::slice::from_raw_parts(p.esi as usize as *const u32, 0x5C / 4);
        if live != scratch || theirs != ours {
            let i = (0..scratch.len()).find(|&i| live[i] != scratch[i]);
            mismatch(
                id,
                n,
                format!("in {:x?} args {:x?}: eax original {theirs:#x}, Rust {ours:#x}; first differing dword {:?} ({:x?} vs {:x?})", Regs::of(p), args, i, i.map(|i| live[i]), i.map(|i| scratch[i])),
            );
        }
        theirs
    }
}

macro_rules! params_port {
    ($entry:ident, $thunk:ident, $id:expr, $build:ident, $nargs:expr, $ret:literal) => {
        extern "C" fn $entry(r: &mut Pushad, a: *const u32) {
            let p = *r;
            r.eax = unsafe { params_run($id, &p, a, $nargs, $build) };
        }
        regs_thunk!($thunk, $entry, $ret);
    };
}

params_port!(pi_default_entry, pi_default_thunk, PI_DEFAULT, build_default, 0, "ret");
params_port!(pi_basic_entry, pi_basic_thunk, PI_BASIC, build_basic, 3, "ret 0xC");
params_port!(pi_volume_entry, pi_volume_thunk, PI_VOLUME, build_volume, 4, "ret 0x10");
params_port!(pi_variant_entry, pi_variant_thunk, PI_VARIANT, build_variant, 4, "ret 0x10");
params_port!(pi_variant_volume_entry, pi_variant_volume_thunk, PI_VARIANT_VOLUME, build_variant_volume, 4, "ret 0x10");
params_port!(pi_ext_entry, pi_ext_thunk, PI_EXT, build_ext, 6, "ret 0x18");
params_port!(pi_3d_entry, pi_3d_thunk, PI_3D, build_3d, 3, "ret 0xC");
params_port!(pi_3d_chan_entry, pi_3d_chan_thunk, PI_3D_CHAN, build_3d_chan, 3, "ret 0xC");
params_port!(pi_3d_chan2_entry, pi_3d_chan2_thunk, PI_3D_CHAN2, build_3d_chan2, 4, "ret 0x10");
params_port!(pi_3d_chan_flag_entry, pi_3d_chan_flag_thunk, PI_3D_CHAN_FLAG, build_3d_chan_flag, 5, "ret 0x14");
params_port!(pi_3d_volume_entry, pi_3d_volume_thunk, PI_3D_VOLUME, build_3d_volume, 4, "ret 0x10");
params_port!(pi_3d_ext_entry, pi_3d_ext_thunk, PI_3D_EXT, build_3d_ext, 6, "ret 0x18");
params_port!(pi_3d_variant_entry, pi_3d_variant_thunk, PI_3D_VARIANT, build_3d_variant, 3, "ret 0xC");
params_port!(pi_3d_variant_volume_entry, pi_3d_variant_volume_thunk, PI_3D_VARIANT_VOLUME, build_3d_variant_volume, 4, "ret 0x10");
params_port!(pi_3d_variant_ext_entry, pi_3d_variant_ext_thunk, PI_3D_VARIANT_EXT, build_3d_variant_ext, 7, "ret 0x1C");

// --- 0x47F080 audio_pause_all / 0x47F170 audio_resume_all / 0x47F020 audio_voices_fade_out_all -----

/// The vtable call on the master voice object: this = [0xF52E30], ECX = its vtable, EDX = vtable
/// +0x28, stdcall (this, f32 volume, 0). Returns the callee's EAX.
unsafe fn master_volume_call(volume_bits: u32, set_flag: impl FnOnce(), esi: u32, edi: u32) -> u32 {
    unsafe {
        let e = rd(MASTER_VOICE);
        let c = rd(e as usize);
        let d = rd(c as usize + 0x28);
        set_flag();
        call_regs(d as usize, e, c, d, esi, edi, &[e, volume_bits, 0])
    }
}

/// The pause loop over the slots: for every in-use slot the pause bit 0x2000 of +0x198 is set
/// (after the 0x4000 bit is cleared and written back) unless +0x196 has 0x40 or the +0x194 bit 0
/// (category -1) / bit 0 of the earlier +0x194 word (other categories) is set; a slot with +0x194
/// bit 1 gets the stop bit 0x400 (a single `or word`) and +0x194 &= 0xFF7F instead.
unsafe fn pause_loop(slots: usize) {
    unsafe {
        for k in 0..SLOT_COUNT {
            let s = slots + k * SLOT_SIZE;
            if !(vr32(s) != 0xFFFF_FFFF || vr16(s + 0x196) & 0x200 != 0) {
                continue;
            }
            if vr32(s + 0xA4) == 0xFFFF_FFFF {
                let cx = vr16(s + 0x198) & 0xBFFF;
                let t = vr8(s + 0x196) & 0x40;
                vw16(s + 0x198, cx);
                if t != 0 {
                    continue;
                }
                if vr8(s + 0x194) & 1 != 0 {
                    continue;
                }
                vw16(s + 0x198, cx | 0x2000);
            } else {
                let dx = vr16(s + 0x194);
                if dx & 2 != 0 {
                    or16(s + 0x198, 0x400);
                    vw16(s + 0x194, dx & 0xFF7F);
                } else {
                    let cx = vr16(s + 0x198) & 0xBFFF;
                    let t = vr8(s + 0x196) & 0x40;
                    vw16(s + 0x198, cx);
                    if t != 0 {
                        continue;
                    }
                    if dx & 1 != 0 {
                        continue;
                    }
                    vw16(s + 0x198, cx | 0x2000);
                }
            }
        }
    }
}

/// The resume loop: every slot with 0x40 in the low byte of +0x196 gets +0x198 = (word & 0xDFFF)
/// | 0x4000 (load, store).
unsafe fn resume_loop(slots: usize) {
    unsafe {
        for k in 0..SLOT_COUNT {
            let s = slots + k * SLOT_SIZE;
            if vr8(s + 0x196) & 0x40 != 0 {
                let si = (vr16(s + 0x198) & 0xDFFF) | 0x4000;
                vw16(s + 0x198, si);
            }
        }
    }
}

/// audio_pause_all with the game's state. While the window is inactive (byte 0x252A406 == 0): once
/// (byte 0x2526AFB != 1) pause category 1 (0x481E90(1)), call the master voice's SetVolume-like
/// method with 0.0, and set the muted byte before that call; then byte 0x2526AD7 = 1. Otherwise
/// the pause depth 0x2526B44 += 1 (one instruction) and the loop. EAX: the vtable call's result, the
/// caller's EAX when already muted, the loop's end value 0xEEBA90 + 128 * 0x19C.
unsafe fn pause_all_live(eax: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        if rb(WINDOW_ACTIVE) == 0 {
            let mut out = eax;
            if rb(MASTER_MUTED) != 1 {
                call_regs(AUDIO_PAUSE_CATEGORY, 0, 0, 0, 0, 0, &[1]);
                out = master_volume_call(0, || wb(MASTER_MUTED, 1), esi, edi);
            }
            wb(WINDOW_PAUSED, 1);
            return out;
        }
        add1_32(PAUSE_DEPTH);
        pause_loop(VOICE_SLOTS);
        SLOTS_END_EAX
    }
}

extern "C" fn pause_all_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(PAUSE_ALL) else {
        r.eax = unsafe { pause_all_live(p.eax, p.esi, p.edi) };
        return;
    };
    unsafe {
        // Window active: the loop is shadowed on a copy of all slots (the original also bumps the
        // pause depth). Window inactive: the original runs, then its two bytes are checked.
        if rb(WINDOW_ACTIVE) != 0 {
            r.eax = slots_shadow(PAUSE_ALL, n, orig, &p, |b| {
                pause_loop(b);
                SLOTS_END_EAX
            });
        } else {
            let t = run_original(PAUSE_ALL, orig, &p, 0);
            if rb(MASTER_MUTED) != 1 || rb(WINDOW_PAUSED) != 1 {
                mismatch(PAUSE_ALL, n, format!("flags after: muted {} paused {}", rb(MASTER_MUTED), rb(WINDOW_PAUSED)));
            }
            r.eax = t.eax;
        }
    }
}
regs_thunk!(pause_all_thunk, pause_all_entry);

/// audio_resume_all with the game's state. When 0x2526AD7 is set: if muted, 0x481E90(0), the master
/// voice call with 1.0 (the muted byte cleared before it), then 0x2526AD7 = 0. Otherwise the pause
/// depth -= 1 (one instruction): a negative result is stored as 0 and the loop runs; a positive one
/// returns; 0 runs the loop. EAX as in pause_all.
unsafe fn resume_all_live(eax: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        if rb(WINDOW_PAUSED) != 0 {
            let mut out = eax;
            if rb(MASTER_MUTED) != 0 {
                call_regs(AUDIO_PAUSE_CATEGORY, 0, 0, 0, 0, 0, &[0]);
                out = master_volume_call(ONE_BITS, || wb(MASTER_MUTED, 0), esi, edi);
            }
            wb(WINDOW_PAUSED, 0);
            return out;
        }
        if sub1_32(PAUSE_DEPTH) {
            wd(PAUSE_DEPTH, 0);
        } else if rd(PAUSE_DEPTH) as i32 > 0 {
            return eax;
        }
        resume_loop(VOICE_SLOTS);
        SLOTS_END_EAX
    }
}

extern "C" fn resume_all_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(RESUME_ALL) else {
        r.eax = unsafe { resume_all_live(p.eax, p.esi, p.edi) };
        return;
    };
    unsafe {
        if rb(WINDOW_PAUSED) != 0 {
            let t = run_original(RESUME_ALL, orig, &p, 0);
            if rb(MASTER_MUTED) != 0 || rb(WINDOW_PAUSED) != 0 {
                mismatch(RESUME_ALL, n, format!("flags after: muted {} paused {}", rb(MASTER_MUTED), rb(WINDOW_PAUSED)));
            }
            r.eax = t.eax;
        } else if (rd(PAUSE_DEPTH) as i32).wrapping_sub(1) <= 0 {
            r.eax = slots_shadow(RESUME_ALL, n, orig, &p, |b| {
                resume_loop(b);
                SLOTS_END_EAX
            });
        } else {
            let t = run_original(RESUME_ALL, orig, &p, 0);
            if t.eax != p.eax {
                mismatch(RESUME_ALL, n, format!("eax: original {:#x}, Rust {:#x}", t.eax, p.eax));
            }
            r.eax = t.eax;
        }
    }
}
regs_thunk!(resume_all_thunk, resume_all_entry);

/// The fade-out-all loop: every in-use slot gets `or word [+0x194], 1` (one instruction) and
/// audio_voice_slot_fade_out(index, ms). (The depth bump and the argument read stay with the caller.)
unsafe fn fade_all_on(slots: usize, ms: u32) {
    unsafe {
        for k in 0..SLOT_COUNT {
            let s = slots + k * SLOT_SIZE;
            if vr32(s) != 0xFFFF_FFFF || vr16(s + 0x196) & 0x200 != 0 {
                or16(s + 0x194, 1);
                slot_fade_out_on(slots, k as u32, ms);
            }
        }
    }
}

const FADE_ALL_FIELDS: &[(usize, usize)] = &[(0xA8, 4), (0xC4, 4), (0x194, 4)];

extern "C" fn fade_all_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(FADE_ALL) else {
        unsafe {
            add1_32(PAUSE_DEPTH);
            let ms = *args;
            fade_all_on(VOICE_SLOTS, ms);
        }
        return;
    };
    unsafe {
        let ms = *args;
        let total = SLOT_SIZE * SLOT_COUNT;
        let mut buf = vec![0u8; total];
        std::ptr::copy_nonoverlapping(VOICE_SLOTS as *const u8, buf.as_mut_ptr(), total);
        let b = buf.as_ptr() as usize;
        fade_all_on(b, ms);
        run_orig(orig, &p, &[ms]);
        'cmp: for k in 0..SLOT_COUNT {
            for &(f, len) in FADE_ALL_FIELDS {
                let x = std::slice::from_raw_parts((b + k * SLOT_SIZE + f) as *const u8, len);
                let y = std::slice::from_raw_parts((VOICE_SLOTS + k * SLOT_SIZE + f) as *const u8, len);
                if x != y {
                    mismatch(FADE_ALL, n, format!("slot {k} +{f:#x}: original {y:x?}, Rust {x:x?} (ms {ms})"));
                    break 'cmp;
                }
            }
        }
    }
}
regs_thunk!(fade_all_thunk, fade_all_entry);

// --- 0x4802D0 audio_voice_set_3d_position (ECX = &handle, EDI = &velocity, cdecl (&position, transform) -> AL)

/// Valid handle: audio_voice_slot_set_orientation(transform) by address, velocity -> +0x68..+0x70,
/// position -> +0x5C..+0x64 (each an f32 round trip, the third position word loaded before the two
/// `or`s and stored after them), `or word [+0x198], 1`, `or word [+0x196], 0x812` (single
/// instructions), +0x190 = 0, then 0x47D780(slot) and `or word [+0x196], 0x20` when it returns
/// non-zero. AL = 1 (the rest of EAX from 0x47D780's return); stale: AL = 0 as elsewhere.
unsafe fn set_3d_position_on(slots: usize, eax: u32, h: usize, vel: usize, pos: usize, transform: u32) -> u32 {
    unsafe {
        let idx = match hcheck(slots, h) {
            H::Ff(_) | H::Gen0(_) => return lo(eax, false),
            H::Stale(idx) => return lo(idx * SLOT_SIZE as u32, false),
            H::Good(idx) => idx,
        };
        let s = slots.wrapping_add(idx as usize * SLOT_SIZE);
        call_regs(SLOT_SET_ORIENTATION, s as u32, transform, 0, 0, 0, &[s as u32]);
        vw32(s + 0x68, q32(rd(vel)));
        vw32(s + 0x6C, q32(rd(vel + 4)));
        vw32(s + 0x70, q32(rd(vel + 8)));
        vw32(s + 0x5C, q32(rd(pos)));
        vw32(s + 0x60, q32(rd(pos + 4)));
        let z = q32(rd(pos + 8));
        or16(s + 0x198, 1);
        or16(s + 0x196, 0x812);
        vw32(s + 0x64, z);
        vw32(s + 0x190, 0);
        let e = call_regs(SLOT_UPDATE_MUTE, 0, s as u32, 0, 0, 0, &[]);
        if e & 0xFF != 0 {
            or16(s + 0x196, 0x20);
        }
        lo(e, true)
    }
}

const POS_FIELDS: &[(usize, usize)] = &[(0x44, 0x30), (0x190, 2), (0x194, 2), (0x196, 2), (0x198, 2)];

extern "C" fn set_3d_position_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (pos, transform) = unsafe { (*args, *args.add(1)) };
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(SET_3D_POS) else {
        r.eax = unsafe { set_3d_position_on(VOICE_SLOTS, p.eax, h, p.edi as usize, pos as usize, transform) };
        return;
    };
    unsafe {
        let idx = handle_idx(h);
        r.eax = shadow_slot_op(SET_3D_POS, n, orig, &p, &[pos, transform], idx, POS_FIELDS, false, |b| {
            set_3d_position_on(b, p.eax, h, p.edi as usize, pos as usize, transform)
        });
    }
}
regs_thunk!(set_3d_position_thunk, set_3d_position_entry);

// --- 0x47F800 audio_voice_play_prepared (ECX = &handle) -----------------------------------------

/// A good handle whose +0x198 has no stop bit (0x400): 0x47D780(slot); when it returns non-zero and
/// +0x196 has no 0x200, +0x196 |= 0x20 (load, store); then bit 0x200 of +0x198 is made the inverse
/// of the same bit of +0x196 with `xor word [+0x198], ax` (one instruction, ax from the re-read
/// flags and +0x198). EAX: the final ax (0 / 0x200); the failures keep the caller's EAX with AL =
/// the slot byte (byte 0xFF / generation 0) or leave the slot offset (stale, stopping).
unsafe fn play_prepared_on(slots: usize, eax: u32, h: usize) -> u32 {
    unsafe {
        let idx = match hcheck(slots, h) {
            H::Ff(al) | H::Gen0(al) => return (eax & !0xFF) | al as u32,
            H::Stale(idx) => return idx * SLOT_SIZE as u32,
            H::Good(idx) => idx,
        };
        let s = slots.wrapping_add(idx as usize * SLOT_SIZE);
        if vr16(s + 0x198) & 0x400 != 0 {
            return idx * SLOT_SIZE as u32;
        }
        let e = call_regs(SLOT_UPDATE_MUTE, 0, s as u32, 0, 0, 0, &[]);
        if e & 0xFF != 0 {
            let fl = vr16(s + 0x196) as u32;
            if fl & 0x200 == 0 {
                vw16(s + 0x196, (fl | 0x20) as u16);
            }
        }
        let mut ax = vr16(s + 0x196) >> 9;
        ax = !ax;
        ax <<= 9;
        ax ^= vr16(s + 0x198);
        ax &= 0x200;
        xor16(s + 0x198, ax);
        ax as u32
    }
}

const PLAY_PREPARED_FIELDS: &[(usize, usize)] = &[(0x194, 2), (0x196, 2), (0x198, 2)];

extern "C" fn play_prepared_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let h = p.ecx as usize;
    let Some((n, orig)) = shadowing(PLAY_PREPARED) else {
        r.eax = unsafe { play_prepared_on(VOICE_SLOTS, p.eax, h) };
        return;
    };
    unsafe {
        let idx = handle_idx(h);
        r.eax = shadow_slot_op(PLAY_PREPARED, n, orig, &p, &[], idx, PLAY_PREPARED_FIELDS, true, |b| play_prepared_on(b, p.eax, h));
    }
}
regs_thunk!(play_prepared_thunk, play_prepared_entry);

// =====================================================================================================
// Weapon / ammo round two. All of these run on the game thread only (the ammo stores and the weapon
// instance lists are never read by the audio thread), so the shadow is the snapshot pattern of
// `weapon_instance_free`: snapshot what the original can write, run the original, keep its result,
// restore the snapshot, run the Rust version and compare; the original's writes stand on a mismatch.
// =====================================================================================================

/// Snapshot shadow: `addrs` are the dwords the original can write. Returns the EAX to give back.
unsafe fn snap_shadow(id: usize, n: u32, orig: usize, p: &Pushad, args: &[u32], addrs: &[usize], ours: impl FnOnce() -> u32) -> u32 {
    unsafe {
        let before: Vec<u32> = addrs.iter().map(|&a| rd(a)).collect();
        let theirs_eax = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, args);
        let theirs: Vec<u32> = addrs.iter().map(|&a| rd(a)).collect();
        for (&a, &v) in addrs.iter().zip(&before) {
            wd(a, v);
        }
        let ours_eax = ours();
        let mine: Vec<u32> = addrs.iter().map(|&a| rd(a)).collect();
        if mine != theirs || ours_eax != theirs_eax {
            mismatch(id, n, format!("in {:x?} args {args:x?}: original eax {theirs_eax:#x} {theirs:x?}, Rust eax {ours_eax:#x} {mine:x?}", Regs::of(p)));
            for (&a, &v) in addrs.iter().zip(&theirs) {
                wd(a, v);
            }
        }
        theirs_eax
    }
}

/// The three dwords of an ammo store entry (type *, count, bonus).
fn entry_addrs(e: u32) -> Vec<usize> {
    if e == 0 { Vec::new() } else { vec![e as usize, e as usize + 4, e as usize + 8] }
}

// --- 0xB68C20 ammo_store_entry_add (ESI = entry, EAX = amount, stdcall (u8 overflow) -> EAX added)

/// room = bonus - count + limit (signed); added = amount when overflow is allowed, else min(amount,
/// room) (signed compare); count += added; with overflow and room < amount the bonus is set to
/// amount - room.
unsafe fn entry_add(entry: u32, amount: u32, ovf: u32) -> u32 {
    unsafe {
        let e = entry as usize;
        let ovf = ovf as u8 != 0;
        let lim = type_limit(rd(e));
        let count = rd(e + 4);
        let bonus = rd(e + 8);
        let room = bonus.wrapping_sub(count).wrapping_add(lim) as i32;
        let amt = amount as i32;
        let added = if ovf || amt < room { amt } else { room };
        wd(e + 4, count.wrapping_add(added as u32));
        if ovf && room < amt {
            wd(e + 8, amt.wrapping_sub(room) as u32);
        }
        added as u32
    }
}

extern "C" fn entry_add_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let ovf = unsafe { *args };
    let Some((n, orig)) = shadowing(ENTRY_ADD) else {
        r.eax = unsafe { entry_add(p.esi, p.eax, ovf) };
        return;
    };
    unsafe { r.eax = snap_shadow(ENTRY_ADD, n, orig, &p, &[ovf], &entry_addrs(p.esi), || entry_add(p.esi, p.eax, ovf)) };
}
regs_thunk!(entry_add_thunk, entry_add_entry, "ret 4");

// --- 0xB690C0 ammo_store_add_type (ECX = &store, EDI = hash, stdcall (amount, overflow) -> EAX) ---

/// The entry for the hash (0 when absent or when the address is 0), then ammo_store_entry_add with
/// EAX = amount and the overflow argument. (doc 44 had the two arguments the other way round: the
/// first stack argument is the amount, the second the overflow flag.)
unsafe fn add_type(store: u32, hash: u32, amount: u32, ovf: u32) -> u32 {
    unsafe {
        match store_scan(store, hash) {
            Some(e) if e != 0 => entry_add(e, amount, ovf),
            _ => 0,
        }
    }
}

extern "C" fn add_type_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (amount, ovf) = unsafe { (*args, *args.add(1)) };
    let Some((n, orig)) = shadowing(ADD_TYPE) else {
        r.eax = unsafe { add_type(p.ecx, p.edi, amount, ovf) };
        return;
    };
    unsafe {
        let e = store_scan(p.ecx, p.edi).unwrap_or(0);
        r.eax = snap_shadow(ADD_TYPE, n, orig, &p, &[amount, ovf], &entry_addrs(e), || add_type(p.ecx, p.edi, amount, ovf));
    }
}
regs_thunk!(add_type_thunk, add_type_entry, "ret 8");

// --- 0xB69190 ammo_store_remove (ECX = &store, EDI = hash, stdcall (amount) -> EAX removed) ------

/// removed = min(amount, count) (signed); count -= removed; with a non-zero bonus the bonus becomes
/// bonus - removed (through the original's `cmp / sbb / and` max, which keeps the wrapped value).
unsafe fn store_remove(store: u32, hash: u32, amount: u32) -> u32 {
    unsafe {
        let e = match store_scan(store, hash) {
            Some(e) if e != 0 => e as usize,
            _ => return 0,
        };
        let count = rd(e + 4);
        let removed = if (amount as i32) < count as i32 { amount } else { count };
        wd(e + 4, count.wrapping_sub(removed));
        let bonus = rd(e + 8);
        if bonus != 0 {
            let edx = bonus.wrapping_sub(removed);
            let esi = if 0u32 < edx { u32::MAX } else { 0 };
            wd(e + 8, esi & edx);
        }
        removed
    }
}

extern "C" fn store_remove_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let amount = unsafe { *args };
    let Some((n, orig)) = shadowing(STORE_REMOVE) else {
        r.eax = unsafe { store_remove(p.ecx, p.edi, amount) };
        return;
    };
    unsafe {
        let e = store_scan(p.ecx, p.edi).unwrap_or(0);
        r.eax = snap_shadow(STORE_REMOVE, n, orig, &p, &[amount], &entry_addrs(e), || store_remove(p.ecx, p.edi, amount));
    }
}
regs_thunk!(store_remove_thunk, store_remove_entry, "ret 4");

// --- 0xB69230 weapon_ammo_store_remove (ECX = instance, EAX = amount, stdcall (store) -> EAX) ------

/// No instance: EAX stays the amount (the original returns without touching it). No info or no
/// primary ammo type: 0. Else ammo_store_remove(store, the type's hash +4, amount).
unsafe fn weapon_store_remove(inst: u32, eax: u32, store: u32) -> u32 {
    unsafe {
        if inst == 0 {
            return eax;
        }
        let info = rd(inst as usize + 8);
        if info == 0 {
            return 0;
        }
        let ty = rd(info as usize + 0xD4);
        if ty == 0 {
            return 0;
        }
        store_remove(store, rd(ty as usize + 4), eax)
    }
}

extern "C" fn weapon_store_remove_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let store = unsafe { *args };
    let Some((n, orig)) = shadowing(WEAPON_STORE_REMOVE) else {
        r.eax = unsafe { weapon_store_remove(p.ecx, p.eax, store) };
        return;
    };
    unsafe {
        let mut e = 0;
        if p.ecx != 0 {
            let info = rd(p.ecx as usize + 8);
            if info != 0 {
                let ty = rd(info as usize + 0xD4);
                if ty != 0 {
                    e = store_scan(store, rd(ty as usize + 4)).unwrap_or(0);
                }
            }
        }
        r.eax = snap_shadow(WEAPON_STORE_REMOVE, n, orig, &p, &[store], &entry_addrs(e), || weapon_store_remove(p.ecx, p.eax, store));
    }
}
regs_thunk!(weapon_store_remove_thunk, weapon_store_remove_entry, "ret 4");

// --- 0xB693E0 ammo_store_clear (EDI = &store) ----------------------------------------------------

/// For i = 0.. while i < the signed count (re-read every iteration; the store base [EDI] is re-read
/// too): count and bonus of entry i = 0. EAX ends as the last entry address (unchanged when the
/// loop does not run).
unsafe fn store_clear(store: u32, eax: u32) -> u32 {
    unsafe {
        if (rd(AMMO_TYPE_COUNT) as i32) <= 0 {
            return eax;
        }
        let mut out;
        let mut i = 0i32;
        let mut off = 0u32;
        loop {
            out = rd(store as usize).wrapping_add(off);
            i += 1;
            wd(out as usize + 4, 0);
            wd(out as usize + 8, 0);
            off = off.wrapping_add(12);
            if i >= rd(AMMO_TYPE_COUNT) as i32 {
                break;
            }
        }
        out
    }
}

extern "C" fn store_clear_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(STORE_CLEAR) else {
        r.eax = unsafe { store_clear(p.edi, p.eax) };
        return;
    };
    unsafe {
        let mut addrs = Vec::new();
        let cnt = rd(AMMO_TYPE_COUNT) as i32;
        if cnt > 0 && cnt < 4096 {
            let base = rd(p.edi as usize);
            for i in 0..cnt as u32 {
                addrs.push(base.wrapping_add(i * 12) as usize + 4);
                addrs.push(base.wrapping_add(i * 12) as usize + 8);
            }
        }
        r.eax = snap_shadow(STORE_CLEAR, n, orig, &p, &[], &addrs, || store_clear(p.edi, p.eax));
    }
}
regs_thunk!(store_clear_thunk, store_clear_entry);

// --- 0xB72C40 weapon_info_is_thrown (EAX = info -> AL) -------------------------------------------

/// AL = class (+0x34) == 5; null: AL = 0. The rest of EAX stays the pointer.
unsafe fn is_thrown(info: u32) -> u32 {
    unsafe {
        if info == 0 {
            return lo(info, false);
        }
        lo(info, rd(info as usize + 0x34) == 5)
    }
}

extern "C" fn is_thrown_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(IS_THROWN, &p, 0, unsafe { is_thrown(p.eax) });
}
regs_thunk!(is_thrown_thunk, is_thrown_entry);

// --- 0xB71AC0 weapon_instance_alloc (ESI = weapon info -> EAX instance or 0) -------------------

/// Take the head of the free list (circular, +0 prev / +4 next): unlink it, insert it before the
/// head of the live list (or make it the only node), then fill in the fields, in the original's
/// order of reads and writes.
unsafe fn instance_alloc(info: u32) -> u32 {
    unsafe {
        let node = rd(INST_FREE);
        if node == 0 {
            return 0;
        }
        let n = node as usize;
        let nx = n + 4; // &node.next
        let next = rd(nx);
        if next == node {
            wd(INST_FREE, 0);
        } else {
            wd(INST_FREE, next);
            let prev = rd(n);
            if prev != 0 {
                wd(prev as usize + 4, rd(nx));
            }
            let next = rd(nx);
            if next != 0 {
                wd(next as usize, rd(n));
            }
        }
        let head = rd(INST_LIVE);
        wd(n, 0);
        wd(nx, 0);
        if head == 0 {
            wd(nx, node);
            wd(n, node);
            wd(INST_LIVE, node);
        } else {
            wd(n, rd(head as usize));
            wd(nx, head);
            let hp = rd(head as usize);
            wd(hp as usize + 4, node);
            wd(head as usize, node);
        }
        let i = info as usize;
        wd(n + 8, info);
        ww(n + 0xC, 0);
        let x = rd(i + 0x10);
        let cl = (((x >> 3) as u8) << 4) & 0x10;
        wb(n + 0xC, rb(n + 0xC) ^ cl);
        let alt = rb(i + 0x10) & 0x20 != 0;
        let cl = rb(n + 0xC);
        let nb: u8 = if alt { ((rd(i + 0x10) >> 4) & 1) as u8 } else { 0 };
        wb(n + 0xC, (((nb << 5) ^ cl) & 0x20) ^ cl);
        let mag = rw(i + 0xDC) as u32;
        wd(n + 0x10, mag);
        wd(n + 0x14, rw(i + 0xDC) as u32);
        let altmag = if rb(i + 0x10) & 0x20 != 0 { rw(i + 0xDC) as u32 } else { 0 };
        wd(n + 0x18, altmag);
        let altmag2 = if rb(i + 0x10) & 0x20 != 0 { rw(i + 0xDC) as u32 } else { 0 };
        wd(n + 0x40, 0);
        wd(n + 0x1C, altmag2);
        wd(n + 0x44, 0);
        wd(n + 0x48, 0);
        wd(n + 0x24, 0xFFFF_FFFF);
        wd(n + 0x4C, q32(rd(0x00E5_C53C)));
        wd(n + 0x28, 0xFFFF_FFFF);
        wd(n + 0x2C, 0xFFFF_FFFF);
        wd(n + 0x30, 0xFFFF_FFFF);
        wd(n + 0xA8, 0xFFFF_FFFF);
        wd(n + 0xAC, 0xFFFF_FFFF);
        wb(n + 0x3C, 0);
        wd(n + 0x50, 0);
        wd(n + 0x34, 0xFFFF_FFFF);
        for off in [0x54usize, 0x60, 0x6C, 0x78, 0x84, 0x90, 0x9C] {
            wd(n + off, 0xFF);
        }
        for off in [0xB4usize, 0xB8, 0xD0, 0xCC, 0xBC, 0xC0, 0xC4, 0xC8] {
            wd(n + off, 0);
        }
        if rd(i + 0x34) == 4 {
            let mut v = rd(0x00E8_43AC);
            wd(n + 0x30, v);
            if v as i32 > 0x6B49_D200 {
                v = v.wrapping_add(0x94B6_2E00);
                wd(n + 0x30, v);
            }
        }
        if rd(i + 0x34) == 5 {
            wb(n + 0xD, rb(n + 0xD) | 4);
        }
        wb(n + 0xD, rb(n + 0xD) | 0x20);
        wd(n + 0xE8, 0);
        wd(n + 0x38, 0xFFFF_FFFF);
        ww(n + 0xE4, 0xFFFF);
        wd(n + 0xEC, 0xFFFF_FFFF);
        node
    }
}

extern "C" fn instance_alloc_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(INST_ALLOC) else {
        r.eax = unsafe { instance_alloc(p.esi) };
        return;
    };
    unsafe {
        // Everything the original can write: both heads, the free node's 0xF0 bytes, and the link
        // words of the nodes around it and around the live head.
        let mut addrs: Vec<usize> = vec![INST_FREE, INST_LIVE];
        let f = rd(INST_FREE);
        if f != 0 {
            let fp = rd(f as usize);
            let fnx = rd(f as usize + 4);
            let h = rd(INST_LIVE);
            let mut objs = vec![fp, fnx, h];
            if h != 0 {
                objs.push(rd(h as usize));
            }
            for k in 0..0xF0 / 4 {
                addrs.push(f as usize + k * 4);
            }
            for o in objs {
                if o != 0 {
                    addrs.push(o as usize);
                    addrs.push(o as usize + 4);
                }
            }
        }
        r.eax = snap_shadow(INST_ALLOC, n, orig, &p, &[], &addrs, || instance_alloc(p.esi));
    }
}
regs_thunk!(instance_alloc_thunk, instance_alloc_entry);

// =====================================================================================================
// Round three: the voice-slot allocator (under the critical section 0x25F5360), the voice start /
// prepare / 2D-channel writers, the three cue players and `weapon_reload_from_store`.
//
// The voice writers touch a slot the audio thread also reads, and call originals with side effects
// (the allocator marks slots, the fade-in writes the slot), so none of them can be run twice: they
// have no shadow. Each is checked by per-branch counters in game and, offline, against the original
// bytes in Unicorn with every callee mocked identically on both sides (order of the calls, their
// register and stack arguments and every byte written are compared). The slot writes keep the
// original's order, with volatile accesses and single `or` / `xor` instructions where the original
// has them.
// =====================================================================================================

const VOICE_LOCK: u32 = 0x025F_5360; // CRITICAL_SECTION of the voice slots
const IAT_ENTER_CS: usize = 0x00DB_80CC; // EnterCriticalSection import slot
const IAT_LEAVE_CS: usize = 0x00DB_80D0; // LeaveCriticalSection import slot
const VOICE_GENERATION: usize = 0x00E9_9C00; // the handle generation counter
const DIST_PROFILES: usize = 0x00EE_9178; // pointers to the distance profiles
const LISTENER: usize = 0x00EE_B254; // listener position (3 f32)
const SLOT_FIND_FREE: usize = 0x0047_CD40; // EAX = class -> AL slot or 0xFF
const SLOT_PICK_LOW: usize = 0x0047_C7C0; // cdecl (class) -> AL slot or 0xFF
const SLOT_PICK_STEAL: usize = 0x0047_CAA0; // cdecl (class, f32 priority) -> AL slot or 0xFF
const SLOT_LINK: usize = 0x0047_C270; // cdecl (u8, u8)
const SLOT_FADE_OUT_FN: usize = 0x0047_C740; // EAX = slot, EDX = ms
const SLOT_ALLOC_FN: usize = 0x0047_CDB0;
const SLOT_FADE_IN_FN: usize = 0x0047_CED0; // ECX = slot, cdecl (ms, f32)
const VOICE_START_FN: usize = 0x0047_F4B0;
const VOICE_PREPARE_FN: usize = 0x0047_F880;
const VECTOR_LENGTH_SQ: usize = 0x0040_EA60; // EAX = &vec3 -> ST0 (an f32 value)
const FOLEY_RANGE: usize = 0x0047_9710; // EAX = foley record -> ST0
const MUSIC_RANGE: usize = 0x0047_9D10; // EAX = music record -> ST0
const LINE_ADVANCE: usize = 0x0047_A710; // EDI = line record -> AL
const DIST_INDEX_OF: usize = 0x0046_F700; // AX = id -> EAX distance profile index
const HANDLE_LIVE: usize = 0x0047_B860; // EAX = &handle -> AL
const HANDLE_SET_FLAG: usize = 0x0048_03F0; // ECX = &handle, DL = on
const FOCUS_REGISTER: usize = 0x0046_EB90; // EDI = &handle, cdecl (index, f32)
const MUSIC_BLOCKED: usize = 0x0252_6ADE; // byte
const LINE_FOCUS_CURRENT: usize = 0x0252_6AA0; // dword
const RANGE_SCALE_A: usize = 0x00E5_C400; // f64
const RANGE_SCALE_B: usize = 0x00E5_C3D8; // f64
const STORE_FIND_FN: usize = 0x00B6_8CE0;
const CLIP_SIZE_FN: usize = 0x00B7_3CC0; // EAX = human, ESI = instance -> EAX
const STORE_TAKE_FN: usize = 0x00B6_8BF0; // EAX = wanted, ESI = entry -> EAX taken

// Branch counters (per port, up to 8 each), reported by summary().
static BRANCH: [AtomicU32; 12 * COUNT] = [const { AtomicU32::new(0) }; 12 * COUNT];

fn bump(id: usize, i: usize) {
    BRANCH[id * 12 + i].fetch_add(1, Ordering::Relaxed);
}

/// `call_regs` for a callee that returns an f32 value on ST0 (read as f64).
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    let mut out = 0f64;
    let block: [u32; 9] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "mov ebx, eax",
            "mov ecx, [ebx + 24]",
            "mov esi, [ebx + 20]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]", "mov esi, [ebx + 12]", "mov edi, [ebx + 16]",
            "mov eax, [ebx + 28]",
            "call dword ptr [ebx]",
            "mov ecx, [ebx + 32]",
            "fstp qword ptr [ecx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    out
}

unsafe fn rf64(p: usize) -> f64 {
    unsafe { std::ptr::read_unaligned(p as *const f64) }
}

unsafe fn lock_enter() {
    unsafe { call_regs(import(IAT_ENTER_CS), 0, 0, 0, 0, 0, &[VOICE_LOCK]) };
}

unsafe fn lock_leave() {
    unsafe { call_regs(import(IAT_LEAVE_CS), 0, 0, 0, 0, 0, &[VOICE_LOCK]) };
}

// --- 0x47CDB0 audio_voice_slot_alloc (ESI = class, EDI = &out byte B, cdecl (&out byte A, f32 priority)) ---

/// Under the lock: a free slot of the class (0x47CD40); failing that class 0..2 takes the pick of
/// 0x47C7C0, class 3 the steal pick 0x47CAA0, and with a pick in hand a slot of class 4 is
/// linked to it after the pick is faded out (250 ms). Writes B then A (A = the slot to use for the
/// handle, B = the slot to fill), 0xFF both when none.
unsafe fn slot_alloc_on(class: u32, b_ptr: usize, a_ptr: usize, prio: u32) {
    unsafe {
        lock_enter();
        let al = call_regs(SLOT_FIND_FREE, class, 0, 0, 0, 0, &[]) as u8;
        if al != 0xFF {
            bump(SLOT_ALLOC_ID, 0);
            wb(b_ptr, al);
            wb(a_ptr, al);
            lock_leave();
            return;
        }
        let c = class as i32;
        let pick = if c < 0 {
            None
        } else if c <= 2 {
            Some(call_regs(SLOT_PICK_LOW, 0, 0, 0, 0, 0, &[class]) as u8)
        } else if c == 3 {
            Some(call_regs(SLOT_PICK_STEAL, 0, 0, 0, 0, 0, &[class, q32(prio)]) as u8)
        } else {
            None
        };
        if let Some(bl) = pick {
            if bl != 0xFF {
                let al2 = call_regs(SLOT_FIND_FREE, 4, 0, 0, 0, 0, &[]) as u8;
                if al2 != 0xFF {
                    bump(SLOT_ALLOC_ID, 1);
                    call_regs(SLOT_FADE_OUT_FN, bl as u32, 0, 0xFA, 0, 0, &[]);
                    call_regs(SLOT_LINK, 0, 0, 0, 0, 0, &[al2 as u32, bl as u32]);
                    wb(a_ptr, bl);
                    wb(b_ptr, al2);
                    lock_leave();
                    return;
                }
            }
        }
        bump(SLOT_ALLOC_ID, 2);
        wb(b_ptr, 0xFF);
        wb(a_ptr, 0xFF);
        lock_leave();
    }
}

extern "C" fn slot_alloc_entry(r: &mut Pushad, args: *const u32) {
    let (a, prio) = unsafe { (*args, *args.add(1)) };
    unsafe { slot_alloc_on(r.esi, r.edi as usize, a as usize, prio) };
}
regs_thunk!(slot_alloc_thunk, slot_alloc_entry);

// --- the voice writers -----------------------------------------------------------------------------

/// Run the allocator the way every caller does: ESI = class, EDI = &B, args (&A, priority).
/// Returns (A, B).
unsafe fn alloc_call(class: u32, prio: u32) -> (u8, u8) {
    let mut a: u8 = 0xFF;
    let mut b: u8 = 0xFF;
    unsafe {
        call_regs(SLOT_ALLOC_FN, 0, 0, 0, class, &mut b as *mut u8 as u32, &[&mut a as *mut u8 as u32, prio]);
        (std::ptr::read_volatile(&a), std::ptr::read_volatile(&b))
    }
}

unsafe fn write_out(out: usize, first: u32) {
    unsafe {
        wd(out, first);
        wd(out + 4, !0);
        wd(out + 8, !0);
    }
}

/// `dst` with the bits of `mask` taken from `src` (the xor / and / xor idiom).
fn merge16(cur: u16, src: u16, mask: u16) -> u16 {
    cur ^ ((src ^ cur) & mask)
}

/// The slot fields shared by audio_voice_start and audio_voice_prepare, up to the 0xE4 store.
/// `s` = slot, `p` = params, `idx` = the slot index (B). Returns nothing; the 0x196 `or` and the
/// 0xE4 store are done here too (the 0xE4 value is read before the `or`).
unsafe fn voice_fill(s: usize, p: usize, idx: u8, cue: u32, prof: u32, a5: u32, a7: u32) -> u16 {
    unsafe {
        let dist = rd(DIST_PROFILES + prof as usize * 4) as usize;
        vw32(s + 4, cue);
        vw32(s + 8, 0);
        let dp8 = (dist + 8) as u32;
        vw32(s + 0x24, q32(rd(p + 0xC)));
        vw32(s + 0xAC, q32(rd(p + 8)));
        vw32(s + 0x5C, q32(rd(p + 0x1C)));
        vw32(s + 0x60, q32(rd(p + 0x20)));
        vw32(s + 0x88, dp8);
        vw32(s + 0x64, q32(rd(p + 0x24)));
        vw32(s + 0x8C, dp8);
        vw32(s + 0xA0, q32(a7));
        vw32(s + 0x9C, q32(rd(dist + 0x28)));
        vw32(s + 0x80, q32(rd(dist)));
        vw32(s + 0xA4, rd(p + 0x18));
        vw32(s + 0xB8, rd(p + 0x4C));
        vw32(s + 0xC0, rd(p + 0x54));
        vw32(s + 0xBC, rd(p + 0x50));
        vw32(s + 0x30, q32(rd(p + 0x14)));
        vw32(s + 0x3C, q32(rd(p + 0x10)));
        vw32(s + 0xB0, rd(p + 0x2C));
        vw32(s + 0x18, q32(a5));
        vw32(s + 0x1C, ONE_BITS);
        let ms = rd(p + 4);
        if ms > 0x20 {
            call_regs(SLOT_FADE_IN_FN, 0, idx as u32, 0, 0, 0, &[ms, 0]);
        }
        vw32(s + 0xD0, rd(p + 0x34));
        vw32(s + 0xD4, rd(p + 0x38));
        vw32(s + 0xD8, rd(p + 0x3C));
        vw32(s + 0xDC, rd(p + 0x40));
        vw32(s + 0xE0, rd(p + 0x44));
        let e4 = rd(p + 0x48);
        or16(s + 0x196, 0x8000);
        let w196 = vr16(s + 0x196);
        vw32(s + 0xE4, e4);
        w196
    }
}

/// The flag-word sequence both writers start with: +0x196 bit 0x800 from flags bit 0, then the
/// first three stores to +0x194 (bits 0, 1, 2 from the flags and the argument bit). Returns the
/// last value stored to +0x194 (the writers carry it in a register).
unsafe fn voice_flags_head(s: usize, f: u16, w196: u16, a6: u32) -> u16 {
    unsafe {
        let flo = f as u8;
        vw16(s + 0x196, merge16(w196, f << 11, 0x800));
        let dl = (flo >> 1) ^ vr8(s + 0x194);
        let bl = (a6 as u8) & 1;
        let ax0 = ((bl as u16) << 1) << 1;
        xor16(s + 0x194, (dl & 1) as u16);
        let cx = vr16(s + 0x194) & 0xFFF9;
        let ax = ax0 | (((flo >> 1) & 2) as u16) | cx;
        vw16(s + 0x194, ax);
        ax
    }
}

/// Bits 4, 3 and 5 of +0x194 from the flags' low byte, one store each.
unsafe fn voice_flags_mid(s: usize, f: u16, mut cur: u16) -> u16 {
    unsafe {
        let flo = (f as u8) as u16;
        for mask in [0x10u16, 0x08, 0x20] {
            cur = merge16(cur, flo, mask);
            vw16(s + 0x194, cur);
        }
        cur
    }
}

/// The generation counter step both writers share: the old value goes to +0xB4, the counter wraps
/// to 1 above 16,000,000.
unsafe fn voice_generation(s: usize) {
    unsafe {
        let g = vr32(VOICE_GENERATION);
        vw32(s + 0xB4, g);
        let g1 = g.wrapping_add(1);
        vw32(VOICE_GENERATION, g1);
        if (g1 as i32) > 0x00F4_2400 {
            vw32(VOICE_GENERATION, 1);
        }
    }
}

// --- 0x47F4B0 audio_voice_start (cdecl (out*, params*, a2, cue, profile, f32 a5, a6, f32 a7) -> EAX = out) ---

unsafe fn voice_start_on(out: usize, p: usize, a2: u32, cue: u32, prof: u32, a5: u32, a6: u32, a7: u32) -> u32 {
    unsafe {
        let (a, b) = alloc_call(rd(p + 0x18), q32(rd(p + 8)));
        if a == 0xFF || b == 0xFF {
            bump(VOICE_START_ID, 1);
            write_out(out, 0xFF);
            return out as u32;
        }
        bump(VOICE_START_ID, 0);
        let s = VOICE_SLOTS + b as usize * SLOT_SIZE;
        let w196 = voice_fill(s, p, b, cue, prof, a5, a7);
        let f = rw(p + 0x58);
        let flo = f as u8;
        let mut cur = voice_flags_head(s, f, w196, a6);
        cur = voice_flags_mid(s, f, cur);
        // bit 6 from flags bit 7, then bits 10, 11, 12 from flags bits 8, 9, 10
        cur = merge16(cur, (flo >> 1) as u16, 0x40);
        vw16(s + 0x194, cur);
        for mask in [0x400u16, 0x800, 0x1000] {
            cur = merge16(cur, f << 2, mask);
            vw16(s + 0x194, cur);
        }
        let mut e = vr16(s + 0x196) as u32;
        if e & 0x800 == 0 {
            e |= 1;
            vw16(s + 0x196, e as u16);
        }
        let w2 = rw(p + 2);
        if w2 > 0xA {
            or16(s + 0x196, 0x200);
            vw32(s + 0xC8, (w2 as f32).to_bits());
        }
        let mute = call_regs(SLOT_UPDATE_MUTE, 0, s as u32, 0, 0, 0, &[]) as u8;
        if mute != 0 {
            let e = vr16(s + 0x196) as u32;
            if e & 0x200 == 0 {
                vw16(s + 0x196, (e | 0x20) as u16);
            }
        }
        voice_generation(s);
        let fl = vr16(s + 0x196);
        let gen_ = vr32(s + 0xB4);
        let mut dx: u16 = !(fl >> 9);
        dx <<= 9;
        dx ^= vr16(s + 0x198);
        dx &= 0x200;
        xor16(s + 0x198, dx);
        vw32(s, a2);
        write_out(out, (gen_ << 8) | a as u32);
        out as u32
    }
}

extern "C" fn voice_start_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe {
        voice_start_on(parg(a, 0) as usize, parg(a, 1) as usize, parg(a, 2), parg(a, 3), parg(a, 4), parg(a, 5), parg(a, 6), parg(a, 7))
    };
}
regs_thunk!(voice_start_thunk, voice_start_entry);

// --- 0x47F880 audio_voice_prepare (same arguments; leaves the cue prepared, flags differ) ----------

unsafe fn voice_prepare_on(out: usize, p: usize, a2: u32, cue: u32, prof: u32, a5: u32, a6: u32, a7: u32) -> u32 {
    unsafe {
        let (a, b) = alloc_call(rd(p + 0x18), q32(rd(p + 8)));
        if a == 0xFF || b == 0xFF {
            bump(VOICE_PREPARE_ID, 1);
            write_out(out, 0xFF);
            return out as u32;
        }
        bump(VOICE_PREPARE_ID, 0);
        let s = VOICE_SLOTS + b as usize * SLOT_SIZE;
        let w196 = voice_fill(s, p, b, cue, prof, a5, a7);
        let f = rw(p + 0x58);
        let flo = f as u8;
        let mut cur = voice_flags_head(s, f, w196, a6);
        cur = voice_flags_mid(s, f, cur);
        // bit 6 from flags bit 7, after clearing +0x196 bit 0x20 (the word is re-read for later)
        and16(s + 0x196, 0xFFDF);
        let edx = vr16(s + 0x196) as u32;
        cur = merge16(cur, (flo >> 1) as u16, 0x40);
        vw16(s + 0x194, cur);
        // bit 10 is a read-modify-write xor on memory, then bits 11 and 12 from registers
        let cx = ((f << 2) ^ cur) & 0x400;
        xor16(s + 0x194, cx);
        let ecx = vr16(s + 0x194);
        let ax = merge16(ecx, f << 2, 0x800);
        vw16(s + 0x194, ax);
        let cx = merge16(ax, f << 2, 0x1000);
        vw16(s + 0x194, cx);
        if edx & 0x800 == 0 {
            vw16(s + 0x196, (edx | 1) as u16);
        }
        let w2 = rw(p + 2);
        if w2 > 0xA {
            or16(s + 0x196, 0x200);
            vw32(s + 0xC8, (w2 as f32).to_bits());
        }
        voice_generation(s);
        let gen_ = vr32(s + 0xB4);
        or16(s + 0x198, 0x100);
        vw32(s, a2);
        write_out(out, (gen_ << 8) | a as u32);
        out as u32
    }
}

extern "C" fn voice_prepare_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe {
        voice_prepare_on(parg(a, 0) as usize, parg(a, 1) as usize, parg(a, 2), parg(a, 3), parg(a, 4), parg(a, 5), parg(a, 6), parg(a, 7))
    };
}
regs_thunk!(voice_prepare_thunk, voice_prepare_entry);

// --- 0x47FBA0 audio_voice_start_2d_channel (cdecl (out*, cue, profile, f32 volume, u8 channel) -> EAX = out) ---

unsafe fn start_2d_on(out: usize, cue: u32, prof: u32, vol: u32, channel: u32) -> u32 {
    unsafe {
        let (a, b) = alloc_call(1, ONE_BITS);
        if a == 0xFF || b == 0xFF {
            bump(START_2D_ID, 1);
            write_out(out, 0xFF);
            return out as u32;
        }
        bump(START_2D_ID, 0);
        let s = VOICE_SLOTS + b as usize * SLOT_SIZE;
        let dist = rd(DIST_PROFILES + prof as usize * 4) as usize;
        vw32(s + 4, cue);
        vw32(s + 0x24, ONE_BITS);
        vw32(s + 0xAC, ONE_BITS);
        let dp8 = (dist + 8) as u32;
        vw32(s + 0x88, dp8);
        vw32(s + 0x5C, 0);
        vw32(s + 0x8C, dp8);
        let dx = vr16(s + 0x194) & 0x302;
        vw32(s + 0x60, 0);
        vw32(s + 0x64, 0);
        vw32(s + 8, 0);
        vw32(s + 0xA0, ONE_BITS);
        vw32(s + 0x9C, q32(rd(dist + 0x28)));
        let d0 = q32(rd(dist));
        let cx = (((channel as u8) as u16) << 14) | dx | 0x2440;
        vw32(s + 0x80, d0);
        vw32(s + 0x3C, ONE_BITS);
        vw16(s + 0x194, cx);
        let c196 = vr16(s + 0x196);
        vw32(s + 0x18, q32(vol));
        vw32(s + 0x1C, ONE_BITS);
        vw16(s + 0x196, (c196 & 0xF5FF) | 0x8001);
        let g = vr32(VOICE_GENERATION);
        vw32(s + 0xB4, g);
        let g1 = g.wrapping_add(1);
        vw32(s + 0xA4, 1);
        vw32(s + 0xB8, 0);
        vw32(s + 0xBC, 0);
        vw32(s + 0xC0, 0);
        vw32(s + 0xB0, 0);
        vw32(VOICE_GENERATION, g1);
        if (g1 as i32) > 0x00F4_2400 {
            vw32(VOICE_GENERATION, 1);
        }
        let gen_ = vr32(s + 0xB4);
        or16(s + 0x198, 0x100);
        vw32(s, 0);
        write_out(out, (gen_ << 8) | a as u32);
        out as u32
    }
}

extern "C" fn start_2d_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { start_2d_on(parg(a, 0) as usize, parg(a, 1), parg(a, 2), parg(a, 3), parg(a, 4)) };
}
regs_thunk!(start_2d_thunk, start_2d_entry);

// --- the three cue players ---------------------------------------------------------------------------

/// params.xyz - listener or listener - params.xyz, each component rounded to f32 once.
unsafe fn offset_vec(p: usize, params_first: bool) -> [u32; 3] {
    unsafe {
        let mut v = [0u32; 3];
        for i in 0..3 {
            let pv = rf(p + 0x1C + i * 4) as f64;
            let lv = rf(LISTENER + i * 4) as f64;
            v[i] = (if params_first { pv - lv } else { lv - pv } as f32).to_bits();
        }
        v
    }
}

/// Call audio_voice_start / audio_voice_prepare with a temporary out handle and copy it to `dst`.
unsafe fn start_or_prepare(dst: usize, prepare: bool, args: [u32; 7]) {
    unsafe {
        let mut tmp = [0u32; 3];
        let f = if prepare { VOICE_PREPARE_FN } else { VOICE_START_FN };
        call_regs(f, 0, 0, 0, 0, 0, &[tmp.as_mut_ptr() as u32, args[0], args[1], args[2], args[3], args[4], args[5], args[6]]);
        let t0 = std::ptr::read_volatile(&tmp[0]);
        let t1 = std::ptr::read_volatile(&tmp[1]);
        let t2 = std::ptr::read_volatile(&tmp[2]);
        wd(dst, t0);
        wd(dst + 4, t1);
        wd(dst + 8, t2);
    }
}

// --- 0x479B60 audio_play_foley_cue (EDI = &out, cdecl (params, u8 prepare_only) -> EAX = EDI) ---------

unsafe fn foley_cue_on(out: usize, p: usize, prepare: u32) -> u32 {
    unsafe {
        let fail = |k: usize| {
            bump(FOLEY_CUE_ID, k);
            write_out(out, 0xFF);
            out as u32
        };
        let id = rw(p);
        if id >= rw(FOLEY_COUNT) {
            return fail(0);
        }
        let mut variant = rd(p + 0x28);
        let rec = rd(FOLEY_TABLE + id as usize * 4) as usize;
        let mode = (rd(rec + 0x10) >> 12) & 0x3F;
        if mode == 1 {
            variant = 0;
        } else if mode == 0 || variant == 0xFFFF_FFFF {
            return fail(1);
        }
        let cue_at = (rd(rec + 4) as usize).wrapping_add((variant as usize).wrapping_mul(4));
        if rd(cue_at) == 0xFFFF_FFFF {
            return fail(2);
        }
        let flags = rw(p + 0x58);
        if flags & 1 != 0 && flags & 0x40 == 0 {
            let v = offset_vec(p, true);
            let r = call_regs_st0(FOLEY_RANGE, rec as u32, 0, 0, 0, 0, &[]);
            let range2 = (r * r) as f32;
            let len2 = call_regs_st0(VECTOR_LENGTH_SQ, v.as_ptr() as u32, 0, 0, 0, 0, &[]) as f32;
            if range2 < len2 {
                return fail(3);
            }
        }
        bump(FOLEY_CUE_ID, 4 + (prepare as u8 != 0) as usize);
        let w10 = rd(rec + 0x10);
        let a5 = q32(rd(rec + 8));
        let a7 = q32(rd(rec + 0xC));
        let cue = rd(cue_at);
        start_or_prepare(out, prepare as u8 != 0, [p as u32, 0, cue, w10 & 0xFFF, a5, (w10 >> 0x12) & 0xFFFF_FF01, a7]);
        out as u32
    }
}

extern "C" fn foley_cue_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { foley_cue_on(r.edi as usize, parg(a, 0) as usize, parg(a, 1)) };
}
regs_thunk!(foley_cue_thunk, foley_cue_entry);

// --- 0x47A4A0 audio_play_music_cue (ESI = &out, cdecl (params, u8 prepare_only) -> EAX = ESI) -------

unsafe fn music_cue_on(out: usize, p: usize, prepare: u32) -> u32 {
    unsafe {
        let fail = |k: usize| {
            bump(MUSIC_CUE_ID, k);
            write_out(out, 0xFF);
            out as u32
        };
        if rb(MUSIC_BLOCKED) != 0 {
            return fail(0);
        }
        let id = rw(p);
        if id >= rw(MUSIC_COUNT) {
            return fail(1);
        }
        let rec = rd(MUSIC_TABLE + id as usize * 4) as usize;
        let cues = rd(rec + 4) as usize;
        if cues == 0 {
            return fail(1);
        }
        let slot = ((rd(rec + 0x10) >> 0x11) & 0x1F) as usize;
        if rd(cues + slot * 4) == 0xFFFF_FFFF {
            return fail(1);
        }
        if rb(p + 0x58) & 1 != 0 {
            let v = offset_vec(p, true);
            let r = call_regs_st0(MUSIC_RANGE, rec as u32, 0, 0, 0, 0, &[]);
            let range2 = (r * r) as f32;
            let len2 = call_regs_st0(VECTOR_LENGTH_SQ, v.as_ptr() as u32, 0, 0, 0, 0, &[]) as f32;
            if range2 < len2 {
                return fail(2);
            }
        }
        bump(MUSIC_CUE_ID, 3 + (prepare as u8 != 0) as usize);
        let w10 = rd(rec + 0x10);
        let a5 = q32(rd(rec + 0xC));
        let cue = rd(cues + slot * 4);
        start_or_prepare(out, prepare as u8 != 0, [p as u32, (w10 >> 0x18) & 3, cue, w10 & 0xFFF, a5, 0, ONE_BITS]);
        out as u32
    }
}

extern "C" fn music_cue_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { music_cue_on(r.esi as usize, parg(a, 0) as usize, parg(a, 1)) };
}
regs_thunk!(music_cue_thunk, music_cue_entry);

// --- 0x47B0B0 audio_play_voice_line (ESI = &out, cdecl (params, u8 prepare_only) -> EAX = ESI) -------

unsafe fn voice_line_on(out: usize, p: usize, prepare: u32) -> u32 {
    unsafe {
        wd(out, 0xFF);
        wd(out + 8, !0);
        wd(out + 4, !0);
        let id = rw(p);
        if id >= rw(LINE_COUNT) || id == 0xFFFF {
            bump(VOICE_LINE_ID, 0);
            return out as u32;
        }
        let rec = LINE_TABLE + id as usize * 16;
        if rd(rec + 4) == 0 {
            bump(VOICE_LINE_ID, 0);
            return out as u32;
        }
        let v30 = rw(p + 0x30);
        if v30 == 0xFFFF {
            if call_regs(LINE_ADVANCE, 0, 0, 0, 0, rec as u32, &[]) as u8 == 0 {
                bump(VOICE_LINE_ID, 1);
                return out as u32;
            }
        } else {
            let ecx = rd(rec + 8);
            let edx = ecx >> 12;
            let limit = edx & 0x7F;
            let pick = if (v30 as u32) < limit { v30 as u32 } else { edx.wrapping_sub(1) };
            wd(rec + 8, ecx ^ (((pick << 0x13) ^ ecx) & 0x03F8_0000));
        }
        ww(p + 0x30, ((rd(rec + 8) >> 0x13) & 0x7F) as u16);
        let n = ((rd(rec + 8) >> 0x13) & 0x7F) as usize;
        let entry = (rd(rec + 4) as usize).wrapping_add(n * 12);
        let cue = rd(entry);
        if cue == 0xFFFF_FFFF {
            bump(VOICE_LINE_ID, 2);
            return out as u32;
        }
        let dist = call_regs(DIST_INDEX_OF, rw(p + 0x32) as u32, 0, 0, 0, 0, &[]);
        let radius = q32(rd(rd(DIST_PROFILES + dist as usize * 4) as usize + 0x28));
        let flags = rw(p + 0x58);
        if flags & 1 != 0 && flags & 0x40 == 0 {
            let v = offset_vec(p, true);
            let r = f32::from_bits(radius) as f64 * rf64(RANGE_SCALE_A);
            let range2 = (r * r) as f32;
            let len2 = call_regs_st0(VECTOR_LENGTH_SQ, v.as_ptr() as u32, 0, 0, 0, 0, &[]) as f32;
            if range2 < len2 {
                bump(VOICE_LINE_ID, 3);
                return out as u32;
            }
        }
        start_or_prepare(out, prepare as u8 != 0, [p as u32, 0, cue, dist, ONE_BITS, 0, ONE_BITS]);
        if call_regs(HANDLE_LIVE, out as u32, 0, 0, 0, 0, &[]) as u8 == 0 {
            bump(VOICE_LINE_ID, 4);
            return out as u32;
        }
        let cl = rb(entry + 4);
        wd(out + 8, ((((cl << 2) as i8) >> 2) as i32) as u32);
        let focus = (rd(rec + 8) >> 0x1A) & 0xF;
        if focus == rd(LINE_FOCUS_CURRENT) {
            bump(VOICE_LINE_ID, 5);
            return out as u32;
        }
        if rb(p + 0x58) & 1 == 0 {
            bump(VOICE_LINE_ID, 6);
            call_regs(HANDLE_SET_FLAG, 0, out as u32, 1, 0, 0, &[]);
            let idx = (rd(rec + 8) >> 0x1A) & 0xF;
            call_regs(FOCUS_REGISTER, 0, 0, 0, 0, out as u32, &[idx, 0]);
            return out as u32;
        }
        let v = offset_vec(p, false);
        let r = f32::from_bits(radius) as f64;
        let len2 = call_regs_st0(VECTOR_LENGTH_SQ, v.as_ptr() as u32, 0, 0, 0, 0, &[]) as f32;
        let lim = ((r * r) * rf64(RANGE_SCALE_B)) as f32;
        if !(lim > len2) {
            bump(VOICE_LINE_ID, 7);
            return out as u32;
        }
        bump(VOICE_LINE_ID, 8);
        call_regs(HANDLE_SET_FLAG, 0, out as u32, 1, 0, 0, &[]);
        let idx = (rd(rec + 8) >> 0x1A) & 0xF;
        call_regs(FOCUS_REGISTER, 0, 0, 0, 0, out as u32, &[idx, radius]);
        out as u32
    }
}

extern "C" fn voice_line_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { voice_line_on(r.esi as usize, parg(a, 0) as usize, parg(a, 1)) };
}
regs_thunk!(voice_line_thunk, voice_line_entry);

// --- 0xB69260 weapon_reload_from_store (stdcall (store, instance, human) ret 12) ---------------------

/// Move up to (clip size - clip) from the ammo store into the instance's clip (+0x10), lowering
/// the bonus (+8) too; with alt fire (info +0x10 bit 0x20 and an alt ammo type at info +0xD8) the
/// same for the alt clip (+0x18) through 0xB68BF0. Returns EAX as the original leaves it, `None`
/// when it leaves the caller's (no instance).
unsafe fn reload_from_store(store: u32, inst: u32, human: u32) -> Option<u32> {
    unsafe {
        if inst == 0 {
            return None;
        }
        let i = inst as usize;
        let info = rd(i + 8) as usize;
        let hash = rd(rd(info + 0xD4) as usize + 4);
        let e1 = call_regs(STORE_FIND_FN, 0, store, 0, 0, hash, &[]) as usize;
        if e1 == 0 {
            bump(RELOAD_ID, 0);
            return Some(0);
        }
        let clip = call_regs(CLIP_SIZE_FN, human, 0, 0, inst, 0, &[]);
        let want = clip.wrapping_sub(rd(i + 0x10)) as i32;
        let have = rd(e1 + 4) as i32;
        let take = if want < have { want } else { have };
        wd(e1 + 4, (have - take) as u32);
        let bonus = rd(e1 + 8);
        if bonus != 0 {
            wd(e1 + 8, bonus.wrapping_sub(take as u32));
        }
        wd(i + 0x10, rd(i + 0x10).wrapping_add(take as u32));
        let info2 = rd(i + 8);
        if rb(info2 as usize + 0x10) & 0x20 == 0 {
            bump(RELOAD_ID, 1);
            return Some(info2);
        }
        let alt = rd(info2 as usize + 0xD8) as usize;
        if alt == 0 {
            bump(RELOAD_ID, 2);
            return Some(0);
        }
        let hash2 = rd(alt + 4);
        let e2 = call_regs(STORE_FIND_FN, 0, store, 0, 0, hash2, &[]);
        if e2 == 0 {
            bump(RELOAD_ID, 3);
            return Some(0);
        }
        let clip2 = call_regs(CLIP_SIZE_FN, human, 0, 0, inst, 0, &[]);
        let need = clip2.wrapping_sub(rd(i + 0x18));
        let took = call_regs(STORE_TAKE_FN, need, 0, 0, e2, 0, &[]);
        wd(i + 0x18, rd(i + 0x18).wrapping_add(took));
        bump(RELOAD_ID, 4);
        Some(took)
    }
}

extern "C" fn reload_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (store, inst, human) = unsafe { (*args, *args.add(1), *args.add(2)) };
    let Some((n, orig)) = shadowing(RELOAD_ID) else {
        if let Some(e) = unsafe { reload_from_store(store, inst, human) } {
            r.eax = e;
        }
        return;
    };
    unsafe {
        let mut addrs: Vec<usize> = Vec::new();
        if inst != 0 {
            let i = inst as usize;
            addrs.push(i + 0x10);
            addrs.push(i + 0x18);
            let info = rd(i + 8) as usize;
            let e1 = call_regs(STORE_FIND_FN, 0, store, 0, 0, rd(rd(info + 0xD4) as usize + 4), &[]);
            if e1 != 0 {
                addrs.extend([e1 as usize + 4, e1 as usize + 8]);
                let alt = rd(info + 0xD8) as usize;
                if rb(info + 0x10) & 0x20 != 0 && alt != 0 {
                    let e2 = call_regs(STORE_FIND_FN, 0, store, 0, 0, rd(alt + 4), &[]);
                    if e2 != 0 {
                        addrs.extend([e2 as usize + 4, e2 as usize + 8]);
                    }
                }
            }
        }
        r.eax = snap_shadow(RELOAD_ID, n, orig, &p, &[store, inst, human], &addrs, || {
            reload_from_store(store, inst, human).unwrap_or(p.eax)
        });
    }
}
regs_thunk!(reload_thunk, reload_entry, "ret 12");


// =====================================================================================================
// Round three, part two: leaf and wrapper functions around the cue players.
//
// - the pure readers (handle check, foley max distance, the two melee tests, vehicle mount find)
//   are checked by a relocated-original shadow for the first SHADOW_CALLS calls;
// - the wrappers that start sounds, the listener / ramp writers, the cue-table add and the others
//   with side effects have per-branch counters in game and are checked offline in Unicorn.
// =====================================================================================================

const PLAY_PARAMS_SIZE: usize = 0x5C;
const AUDIO_PLAY_FN: usize = 0x0046_5B90; // EAX = params, ECX = &out handle
const PLAY_PARAMS_INIT_3D_FN: usize = 0x0046_4AC0; // ESI = params, EDI = &position, stdcall (cue, type, tag)
const PAUSE_DEPTH_ADDR: usize = 0x0252_6B44;
const FOLEY_CUE_FN: usize = 0x0047_9B60;
const VOICE_LINE_FN: usize = 0x0047_B0B0;
const MUSIC_CUE_FN: usize = 0x0047_A4A0;
const LISTENER_BLOCK: usize = 0x00EE_B230;
const SLOT_FADE_OUT_ADDR: usize = SLOT_FADE_OUT_FN;
const STATION_HANDLE: usize = 0x00E9_A524; // voice handle of the vehicle radio
const STATION_OWNER: usize = 0x00E9_A530;
const STATION_VOLUME: usize = 0x00E9_A534; // f32
const STATION_FLAG: usize = 0x0252_7CD0;
const TARGET_LOCK_A: usize = 0x0252_A12C;
const TARGET_LOCK_B: usize = 0x0252_A130;
const RETICULE_UPDATE_FN: usize = 0x00B6_A8B0; // ECX, EDX, cdecl (f32)
const VOICE_STOP_FN: usize = 0x0047_FEB0; // ECX = &handle
const LOCK_HANDLE: u32 = 0x00EB_D05C;
const HUMAN_IS_PLAYER_FN: usize = 0x0092_6600; // ECX = human -> AL
const AMMO_STORE_REMOVE_FN: usize = 0x00B6_9230; // ECX = instance, EAX = amount, stdcall (store)
const CUE_BANK_FIND_FN: usize = 0x0048_1760; // ECX = bank name -> EAX bank (0xFF none)
const CUE_IN_BANK_FN: usize = 0x0047_0230; // cdecl (bank record, cue name) -> AX cue or 0xFFFF
const CUE_HASH_INSERT_FN: usize = 0x0047_0180; // EDI = key struct -> EAX index or -1
const CUE_COMPARE_FN: usize = 0x0048_1700;
const BANK_TABLE_PTR: usize = 0x00EF_8704;
const CUE_HASH_TABLE: usize = 0x0252_6ACC;
const CUE_COUNT_ADDR: usize = 0x0252_6B54;
const CUE_ENTRIES: usize = 0x00EF_A208;

// --- 0x465EC0 audio_voice_handle_valid (ECX = &handle -> AL; the rest of EAX as the original leaves it) ---

/// AL = the slot is in use by this handle. EAX: the caller's EAX with AL replaced when the handle is
/// 0xFF / generation 0; otherwise the slot offset (slot * 0x19C) with AL replaced.
unsafe fn handle_valid(eax: u32, h: usize) -> u32 {
    unsafe {
        let al = rb(h);
        let hi = eax & !0xFF;
        if al == 0xFF {
            return hi;
        }
        let g = rd(h) >> 8;
        if g == 0 {
            return hi;
        }
        let off = (al as u32).wrapping_mul(SLOT_SIZE as u32);
        let s = VOICE_SLOTS + off as usize;
        if g != vr32(s + 0xB4) {
            return off & !0xFF;
        }
        (off & !0xFF) | (vr32(s) != 0xFFFF_FFFF) as u32
    }
}

extern "C" fn handle_valid_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(HANDLE_VALID_ID, &p, 0, unsafe { handle_valid(p.eax, p.ecx as usize) });
}
regs_thunk!(handle_valid_thunk, handle_valid_entry);

// --- 0xB72740 weapon_info_is_melee (EAX = info) / 0xB72780 weapon_class_is_melee (EAX = class) -> AL ----

/// The jump tables of both functions: classes 6..=19, entry 0 = the `mov al, 1` target, 1 = `xor al, al`.
const MELEE_TABLE: [u8; 14] = [0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0];

/// EAX as the original leaves it for a class: 0 / 1, or (class - 6) with AL cleared when out of range.
fn class_is_melee(class: u32) -> u32 {
    let c = class.wrapping_sub(6);
    if c > 0xD {
        return c & !0xFF;
    }
    (MELEE_TABLE[c as usize] == 0) as u32
}

unsafe fn info_is_melee(info: u32) -> u32 {
    unsafe {
        if info == 0 {
            return 0;
        }
        class_is_melee(rd(info as usize + 0x34))
    }
}

extern "C" fn info_is_melee_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(INFO_IS_MELEE_ID, &p, 0, unsafe { info_is_melee(p.eax) });
}
regs_thunk!(info_is_melee_thunk, info_is_melee_entry);

extern "C" fn class_is_melee_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(CLASS_IS_MELEE_ID, &p, 0, class_is_melee(p.eax));
}
regs_thunk!(class_is_melee_thunk, class_is_melee_entry);

// --- 0xB6B4A0 vehicle_weapon_mount_find (EAX = vehicle, EDI = key -> EAX entry or 0) ------------------

unsafe fn mount_find(veh: u32, key: u32) -> u32 {
    unsafe {
        if veh == 0 {
            return 0;
        }
        let a = rd(veh as usize + 0x84E4) as usize + 4;
        if rb(a + 0x78) != 1 {
            return 0;
        }
        let state = rd(a) as usize;
        let count = rd(state + 0xC8);
        let mut i = 0u32;
        if count != 0 {
            let mut p = rd(state + 0xCC) as usize;
            loop {
                if rd(p) == key {
                    break;
                }
                i += 1;
                p += 0x20;
                if i >= count {
                    break;
                }
            }
        }
        if i == count {
            return 0;
        }
        (i << 5).wrapping_add(rd(state + 0xCC))
    }
}

extern "C" fn mount_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(MOUNT_FIND_ID, &p, 0, unsafe { mount_find(p.eax, p.edi) });
}
regs_thunk!(mount_find_thunk, mount_find_entry);

// --- the four play wrappers ------------------------------------------------------------------------------

/// A zeroed params struct for audio_play: the CRT memset by address, as the originals do.
unsafe fn zeroed_params(buf: &mut [u32; 24]) -> usize {
    let p = buf.as_mut_ptr() as usize;
    unsafe { call_regs(MEMSET, 0, 0, 0, 0, 0, &[p as u32, 0, PLAY_PARAMS_SIZE as u32]) };
    p
}

/// 0x465C40 audio_play_foley_2d (ESI = &out, cdecl (u16 cue) -> EAX = ESI).
unsafe fn play_foley_2d(out: u32, cue: u32) -> u32 {
    unsafe {
        bump(PLAY_FOLEY_2D_ID, 0);
        let mut buf = [0u32; 24];
        let p = zeroed_params(&mut buf);
        wd(p + 0xC, ONE_BITS);
        ww(p, cue as u16);
        wd(p + 0x10, ONE_BITS);
        wd(p + 0x28, 0);
        ww(p + 0x30, 0xFFFF);
        ww(p + 0x32, 1);
        wd(p + 0x54, 0);
        wd(p + 0x18, 0);
        wd(p + 0x4C, 0);
        call_regs(AUDIO_PLAY_FN, p as u32, out, 0, 0, 0, &[]);
        out
    }
}

extern "C" fn play_foley_2d_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { play_foley_2d(r.esi, parg(a, 0)) };
}
regs_thunk!(play_foley_2d_thunk, play_foley_2d_entry);

/// 0x465CD0 audio_play_foley_2d_tagged (ESI = &out, cdecl (u16 cue, tag) -> EAX = ESI).
unsafe fn play_foley_2d_tagged(out: u32, cue: u32, tag: u32) -> u32 {
    unsafe {
        bump(PLAY_FOLEY_2D_TAGGED_ID, 0);
        let mut buf = [0u32; 24];
        let p = zeroed_params(&mut buf);
        wd(p + 0xC, ONE_BITS);
        ww(p, cue as u16);
        wd(p + 0x10, ONE_BITS);
        wd(p + 0x28, 0);
        ww(p + 0x30, 0xFFFF);
        ww(p + 0x32, 1);
        wd(p + 0x54, 0);
        wd(p + 0x18, 0);
        wd(p + 0x4C, tag);
        call_regs(AUDIO_PLAY_FN, p as u32, out, 0, 0, 0, &[]);
        out
    }
}

extern "C" fn play_foley_2d_tagged_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { play_foley_2d_tagged(r.esi, parg(a, 0), parg(a, 1)) };
}
regs_thunk!(play_foley_2d_tagged_thunk, play_foley_2d_tagged_entry);

/// 0x465D40 audio_play_foley_3d (ECX = cue, EAX = tag, EDI = &position, cdecl (&out) -> EAX = out).
unsafe fn play_foley_3d(out: u32, cue: u32, tag: u32, pos: u32) -> u32 {
    unsafe {
        bump(PLAY_FOLEY_3D_ID, 0);
        let mut buf = [0u32; 24];
        let p = buf.as_mut_ptr() as usize;
        call_regs(PLAY_PARAMS_INIT_3D_FN, 0, 0, 0, p as u32, pos, &[cue, 0, tag]);
        call_regs(AUDIO_PLAY_FN, p as u32, out, 0, 0, 0, &[]);
        out
    }
}

extern "C" fn play_foley_3d_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { play_foley_3d(parg(a, 0), r.ecx, r.eax, r.edi) };
}
regs_thunk!(play_foley_3d_thunk, play_foley_3d_entry);

/// 0x465D70 audio_play_voice_line_2d (ESI = &out, cdecl (u16 voice line id) -> EAX = ESI).
unsafe fn play_voice_line_2d(out: u32, cue: u32) -> u32 {
    unsafe {
        bump(PLAY_LINE_2D_ID, 0);
        let mut buf = [0u32; 24];
        let p = zeroed_params(&mut buf);
        wd(p + 0x10, ONE_BITS);
        wd(p + 0xC, ONE_BITS);
        ww(p + 0x32, 1);
        ww(p, cue as u16);
        wd(p + 0x18, 1);
        wd(p + 0x28, 0);
        ww(p + 0x30, 0xFFFF);
        wd(p + 0x54, 0);
        wd(p + 0x4C, 0);
        call_regs(AUDIO_PLAY_FN, p as u32, out, 0, 0, 0, &[]);
        out
    }
}

extern "C" fn play_voice_line_2d_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { play_voice_line_2d(r.esi, parg(a, 0)) };
}
regs_thunk!(play_voice_line_2d_thunk, play_voice_line_2d_entry);

// --- 0x465B00 audio_prepare (EAX = params, ECX = &out -> EAX = out) ----------------------------------------

unsafe fn audio_prepare(params: u32, out: u32) -> u32 {
    unsafe {
        let p = params as usize;
        let o = out as usize;
        let fail = |k: usize| {
            bump(AUDIO_PREPARE_ID, k);
            wd(o, 0xFF);
            wd(o + 4, !0);
            wd(o + 8, !0);
            out
        };
        if rw(p) == 0xFFFF {
            return fail(0);
        }
        if (rd(PAUSE_DEPTH_ADDR) as i32) > 0 && rb(p + 0x58) & 2 == 0 {
            return fail(1);
        }
        match rd(p + 0x18) {
            0 => {
                bump(AUDIO_PREPARE_ID, 2);
                call_regs(FOLEY_CUE_FN, params, 0, 0xFFFF_FFFF, out, out, &[params, 1]);
            }
            1 => {
                bump(AUDIO_PREPARE_ID, 3);
                call_regs(VOICE_LINE_FN, params, 0, 0xFFFF_FFFF, out, 0xFFFF_FFFF, &[params, 1]);
            }
            2 => {
                bump(AUDIO_PREPARE_ID, 4);
                call_regs(MUSIC_CUE_FN, params, 0, 0xFFFF_FFFF, out, 0xFFFF_FFFF, &[params, 1]);
            }
            _ => return fail(5),
        }
        out
    }
}

extern "C" fn audio_prepare_entry(r: &mut Pushad, _: *const u32) {
    r.eax = unsafe { audio_prepare(r.eax, r.ecx) };
}
regs_thunk!(audio_prepare_thunk, audio_prepare_entry);

// --- 0x47E7C0 audio_set_listener (EAX = orientation block, ECX = velocity, cdecl (&position, a, b, c)) ----

/// Writes the listener block in the original's order (volatile). The positions and vectors are
/// copied with `fld` / `fstp` (signalling NaNs come out quiet); the three scalars are plain dwords.
unsafe fn set_listener(basis: usize, vel: usize, pos: usize, a1: u32, a2: u32, a3: u32) {
    unsafe {
        bump(SET_LISTENER_ID, 0);
        let q = |p: usize| q32(vr32(p));
        vw32(0x00EE_B254, q(pos));
        vw32(0x00EE_B258, q(pos + 4));
        vw32(0x00EE_B25C, q(pos + 8));
        vw32(LISTENER_BLOCK, a3);
        vw32(0x00EE_B23C, q(basis + 0x18));
        vw32(0x00EE_B240, q(basis + 0x1C));
        vw32(0x00EE_B244, q(basis + 0x20));
        vw32(0x00EE_B248, q(basis + 0x0C));
        vw32(0x00EE_B24C, q(basis + 0x10));
        vw32(0x00EE_B250, q(basis + 0x14));
        vw32(0x00EE_B238, a1);
        vw32(0x00EE_B260, q(vel));
        vw32(0x00EE_B264, q(vel + 4));
        vw32(0x00EE_B268, q(vel + 8));
        vw32(0x00EE_B234, a2);
    }
}

extern "C" fn set_listener_entry(r: &mut Pushad, a: *const u32) {
    unsafe { set_listener(r.eax as usize, r.ecx as usize, parg(a, 0) as usize, parg(a, 1), parg(a, 2), parg(a, 3)) };
}
regs_thunk!(set_listener_thunk, set_listener_entry);

// --- 0x48D900 radio_vehicle_station_stop (EAX = vehicle) ----------------------------------------------------

unsafe fn station_stop(obj: u32) {
    unsafe {
        if obj == 0 {
            bump(STATION_STOP_ID, 0);
            return;
        }
        if rd(obj as usize + 0x44) != rd(STATION_OWNER) {
            bump(STATION_STOP_ID, 1);
            return;
        }
        let al = rb(STATION_HANDLE);
        if al != 0xFF {
            let g = rd(STATION_HANDLE) >> 8;
            if g != 0 {
                let s = VOICE_SLOTS + al as usize * SLOT_SIZE;
                if g == vr32(s + 0xB4) && vr32(s) != 0xFFFF_FFFF {
                    bump(STATION_STOP_ID, 2);
                    call_regs(SLOT_FADE_OUT_ADDR, al as u32, 0, 0x2EE, 0, 0, &[]);
                }
            }
        }
        bump(STATION_STOP_ID, 3);
        wd(STATION_FLAG, 0);
        wd(STATION_VOLUME, 0);
        wd(STATION_OWNER, 0);
    }
}

extern "C" fn station_stop_entry(r: &mut Pushad, _: *const u32) {
    unsafe { station_stop(r.eax) };
}
regs_thunk!(station_stop_thunk, station_stop_entry);

// --- 0xB6B460 weapon_target_lock_clear -----------------------------------------------------------------------

/// With a lock set: the reticule update with 0.0, both lock words cleared, then (tail call) the
/// voice stop of the lock-on sound handle. Returns the EAX of that last call, `None` with no lock.
unsafe fn target_lock_clear() -> Option<u32> {
    unsafe {
        if rd(TARGET_LOCK_A) == 0 {
            bump(LOCK_CLEAR_ID, 0);
            return None;
        }
        bump(LOCK_CLEAR_ID, 1);
        call_regs(RETICULE_UPDATE_FN, 0, 0, 0, 0, 0, &[0]);
        wd(TARGET_LOCK_B, 0);
        wd(TARGET_LOCK_A, 0);
        Some(call_regs(VOICE_STOP_FN, 0, LOCK_HANDLE, 0, 0, 0, &[]))
    }
}

extern "C" fn target_lock_clear_entry(r: &mut Pushad, _: *const u32) {
    if let Some(e) = unsafe { target_lock_clear() } {
        r.eax = e;
    }
}
regs_thunk!(target_lock_clear_thunk, target_lock_clear_entry);

// --- 0x9C3E50 weapon_remove_reserve_ammo (ECX = human, ESI = instance, EDX = amount -> EAX removed) -------

unsafe fn remove_reserve_ammo(human: u32, inst: u32, amount: u32) -> u32 {
    unsafe {
        if human == 0 || inst == 0 || (amount as i32) <= 0 {
            bump(REMOVE_RESERVE_ID, 0);
            return 0;
        }
        let i = inst as usize;
        if call_regs(HUMAN_IS_PLAYER_FN, 0, human, 0, 0, 0, &[]) as u8 != 0 {
            let info = rd(i + 8) as usize;
            if rd(info + 0x34) != 0x16 {
                bump(REMOVE_RESERVE_ID, 1);
                let store = rd(human as usize + 0x11DC);
                return call_regs(AMMO_STORE_REMOVE_FN, amount, inst, 0, 0, 0, &[store]);
            }
        }
        bump(REMOVE_RESERVE_ID, 2);
        let have = rd(i + 0x14) as i32;
        let take = if (amount as i32) < have { amount as i32 } else { have };
        wd(i + 0x14, (have - take) as u32);
        take as u32
    }
}

extern "C" fn remove_reserve_ammo_entry(r: &mut Pushad, _: *const u32) {
    r.eax = unsafe { remove_reserve_ammo(r.ecx, r.esi, r.edx) };
}
regs_thunk!(remove_reserve_ammo_thunk, remove_reserve_ammo_entry);

// --- 0x481D20 audio_cue_table_add (ECX = bank name, EAX = cue name, cdecl (category, u8 flag) -> EAX index or -1) ---

/// Finds the cue in its bank, then returns the index of an equal entry (hash table 0x470180 when
/// 0x2526ACC is set, else a linear scan of the 4-byte entries at 0xEFA208) or appends one
/// {u16 cue, u8 bank, category | flag << 7} (max 0x16000 entries).
unsafe fn cue_table_add(eax: u32, ecx: u32, edx: u32, edi: u32, category: u32, flag: u32) -> u32 {
    unsafe {
        let bank = call_regs(CUE_BANK_FIND_FN, eax, ecx, edx, eax, edi, &[]);
        if bank == 0xFF {
            bump(CUE_ADD_ID, 0);
            return !0;
        }
        let rec = (bank << 5).wrapping_add(rd(BANK_TABLE_PTR));
        if rd(rec as usize + 0xC) == 0 || eax == 0 {
            bump(CUE_ADD_ID, 0);
            return !0;
        }
        let cue = call_regs(CUE_IN_BANK_FN, 0, 0, 0, 0, 0, &[rec, eax]) as u16;
        if cue == 0xFFFF {
            bump(CUE_ADD_ID, 0);
            return !0;
        }
        let flagb = flag as u8;
        let mut count;
        if rd(CUE_HASH_TABLE) != 0 {
            let mut key = [0u32; 6];
            key[0] = cue as u32;
            key[1] = bank;
            key[2] = category;
            key[3] = flagb as u32;
            key[4] = rd(CUE_COUNT_ADDR);
            key[5] = CUE_COMPARE_FN as u32;
            let found = call_regs(CUE_HASH_INSERT_FN, 0, 0, 0, 0, key.as_mut_ptr() as u32, &[]);
            count = rd(CUE_COUNT_ADDR);
            if found != count {
                bump(CUE_ADD_ID, 1);
                return found;
            }
        } else {
            count = rd(CUE_COUNT_ADDR);
            let mut i = 0u32;
            while i < count {
                let w = rw(CUE_ENTRIES + 2 + i as usize * 4) as u32;
                if category == ((w >> 8) & 0x7F)
                    && bank == (w & 0xFF)
                    && cue == rw(CUE_ENTRIES + i as usize * 4)
                    && (((w as u16) >> 15) as u8) == flagb
                {
                    bump(CUE_ADD_ID, 2);
                    return i;
                }
                i += 1;
            }
        }
        if count >= 0x16000 {
            bump(CUE_ADD_ID, 3);
            return !0;
        }
        let e = CUE_ENTRIES + count as usize * 4;
        count += 1;
        wd(CUE_COUNT_ADDR, count);
        if bank >= 0xFF {
            bump(CUE_ADD_ID, 3);
            return !0;
        }
        bump(CUE_ADD_ID, 4);
        wb(e + 2, bank as u8);
        let hi = (flagb << 7) | (category as u8 & 0x7F);
        ww(e, cue);
        let lo = rb(e + 2);
        ww(e + 2, ((hi as u16) << 8) | lo as u16);
        count - 1
    }
}

extern "C" fn cue_table_add_entry(r: &mut Pushad, a: *const u32) {
    r.eax = unsafe { cue_table_add(r.eax, r.ecx, r.edx, r.edi, parg(a, 0), parg(a, 1)) };
}
regs_thunk!(cue_table_add_thunk, cue_table_add_entry);


// =====================================================================================================
// Round three, part three: the radio helpers (name lookups, DJ voice stop, vehicle radio off / block).
// The two name lookups are pure readers (relocated-original shadow); the others stop voices and are
// checked by branch counters in game and offline in Unicorn.
// =====================================================================================================

const NAME_HASH_FN: usize = 0x00BD_CB30; // EAX = 0 (seed), ECX = string, stdcall (&out hash) -> EAX = &out
const RADIO_TRACKS: usize = 0x00F5_3090; // pointer to the 0x14-byte track records (+0 hash)
const RADIO_TRACK_COUNT: usize = 0x0252_6B94; // u16
const RADIO_STATIONS: usize = 0x00F5_3094; // pointer to the 0x48-byte station records (+4 hash)
const RADIO_STATION_COUNT: usize = 0x0252_6BC8; // u16
const RADIO_STATION_DEFAULT: usize = 0x0252_6BA0; // u16
const DJ_HANDLE: usize = 0x00E9_A538;
const DJ_OWNER: usize = 0x00E9_A544;
const DJ_FLAG: usize = 0x00E9_A548;
const STATION_STOP_FN: usize = 0x0048_D900; // EAX = vehicle
const DJ_STOP_FN: usize = 0x0048_D970; // cdecl (fade ms)

/// The string hash of 0xBDCB30 with seed 0 (ECX = the name).
unsafe fn name_hash(name: u32) -> u32 {
    let mut h: u32 = 0;
    unsafe {
        call_regs(NAME_HASH_FN, 0, name, 0, 0, 0, &[&mut h as *mut u32 as u32]);
        std::ptr::read_volatile(&h)
    }
}

// --- 0x48F780 radio_track_find_by_name (ECX = name -> AX track index, 0xFFFF when absent) --------------

unsafe fn radio_track_find(name: u32) -> u32 {
    unsafe {
        let hash = name_hash(name);
        let count = rw(RADIO_TRACK_COUNT) as u32;
        let mut i = 0u32;
        if count != 0 {
            let base = rd(RADIO_TRACKS) as usize;
            loop {
                if rd(base + (i as usize & 0xFFFF) * 0x14) == hash {
                    return i;
                }
                i += 1;
                if (i as u16) >= count as u16 {
                    break;
                }
            }
        }
        0xFFFF
    }
}

extern "C" fn radio_track_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(RADIO_TRACK_FIND_ID, &p, 0, unsafe { radio_track_find(p.ecx) });
}
regs_thunk!(radio_track_find_thunk, radio_track_find_entry);

// --- 0x4904F0 radio_station_find_by_name (ECX = name -> AX station index, the default when absent) ------

unsafe fn radio_station_find(name: u32) -> u32 {
    unsafe {
        let hash = name_hash(name);
        let count = rw(RADIO_STATION_COUNT) as u32;
        let mut i = 0u32;
        if count != 0 {
            let base = rd(RADIO_STATIONS) as usize;
            loop {
                if rd(base + (i as usize & 0xFFFF) * 0x48 + 4) == hash {
                    return i;
                }
                i += 1;
                if (i as u16) >= count as u16 {
                    break;
                }
            }
        }
        // `mov ax, [default]` keeps the upper half of EAX, which is the loop's count (< 0x10000) or 0
        rw(RADIO_STATION_DEFAULT) as u32
    }
}

extern "C" fn radio_station_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(RADIO_STATION_FIND_ID, &p, 0, unsafe { radio_station_find(p.ecx) });
}
regs_thunk!(radio_station_find_thunk, radio_station_find_entry);

// --- 0x48D970 radio_dj_voice_stop (cdecl (fade ms)) ---------------------------------------------------

unsafe fn dj_voice_stop(ms: u32) {
    unsafe {
        let h = rd(DJ_HANDLE);
        let al = h as u8;
        if al != 0xFF {
            let g = h >> 8;
            if g != 0 {
                let s = VOICE_SLOTS + al as usize * SLOT_SIZE;
                if g == vr32(s + 0xB4) && vr32(s) != 0xFFFF_FFFF {
                    bump(DJ_STOP_ID, 0);
                    call_regs(SLOT_FADE_OUT_FN, al as u32, 0, ms, 0, 0, &[]);
                }
            }
        }
        bump(DJ_STOP_ID, 1);
        wd(0x0252_7CD0, 0);
        wd(DJ_FLAG, 0);
        wd(DJ_OWNER, 0);
    }
}

extern "C" fn dj_voice_stop_entry(r: &mut Pushad, a: *const u32) {
    unsafe { dj_voice_stop(parg(a, 0)) };
    r.eax = 0;
}
regs_thunk!(dj_voice_stop_thunk, dj_voice_stop_entry);

// --- 0x48E330 vehicle_radio_turn_off (ESI = vehicle -> AL) / 0x490EB0 vehicle_radio_block (ESI = vehicle) ---

/// Stop the three radio voices at +0x8D40 / +0x8D4C / +0x8D58, the station voice, and the DJ voice
/// (750 ms) when this vehicle owns it. Returns the EAX the last call leaves.
unsafe fn stop_radio_voices(veh: u32) -> u32 {
    unsafe {
        let v = veh as usize;
        call_regs(VOICE_STOP_FN, 0, veh + 0x8D40, 0, 0, 0, &[]);
        call_regs(VOICE_STOP_FN, 0, veh + 0x8D4C, 0, 0, 0, &[]);
        call_regs(VOICE_STOP_FN, 0, veh + 0x8D58, 0, 0, 0, &[]);
        let mut e = call_regs(STATION_STOP_FN, veh, 0, 0, 0, 0, &[]);
        if rd(v + 0x44) == rd(DJ_OWNER) {
            e = call_regs(DJ_STOP_FN, 0, 0, 0, 0, 0, &[0x2EE]);
        }
        e
    }
}

unsafe fn vehicle_radio_turn_off(veh: u32) -> u32 {
    unsafe {
        let v = veh as usize;
        let info = rd(v + 0x84E4);
        if rd(info as usize + 0x5B0) & 0x0400_0000 == 0 {
            bump(RADIO_OFF_ID, 0);
            return info & !0xFF;
        }
        if rb(v + 0x8D84) & 1 != 0 {
            bump(RADIO_OFF_ID, 0);
            return info & !0xFF;
        }
        bump(RADIO_OFF_ID, 1);
        let e = stop_radio_voices(veh);
        (e & !0xFF) | 1
    }
}

extern "C" fn vehicle_radio_turn_off_entry(r: &mut Pushad, _: *const u32) {
    r.eax = unsafe { vehicle_radio_turn_off(r.esi) };
}
regs_thunk!(vehicle_radio_turn_off_thunk, vehicle_radio_turn_off_entry);

unsafe fn vehicle_radio_block(veh: u32) {
    unsafe {
        let v = veh as usize;
        let al = rb(v + 0x8D84);
        if al & 1 != 0 {
            bump(RADIO_BLOCK_ID, 0);
            return;
        }
        bump(RADIO_BLOCK_ID, 1);
        wb(v + 0x8D84, al | 1);
        stop_radio_voices(veh);
    }
}

extern "C" fn vehicle_radio_block_entry(r: &mut Pushad, _: *const u32) {
    unsafe { vehicle_radio_block(r.esi) };
}
regs_thunk!(vehicle_radio_block_thunk, vehicle_radio_block_entry);


// --- install ------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy.
unsafe fn set_rel32(copy: usize, off: usize, dest: usize) {
    let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
    unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); OLD_COUNT] = [
        (TOTAL_AMMO, &TOTAL_AMMO_BODY, &TOTAL_AMMO_BODY_CALLS, total_ammo_thunk),
        (STORE_COUNT, &STORE_COUNT_BODY, &STORE_COUNT_BODY_CALLS, store_count_thunk),
        (STORE_FIND, &STORE_FIND_BODY, &STORE_FIND_BODY_CALLS, store_find_thunk),
        (STORE_CAPACITY, &STORE_CAPACITY_BODY, &STORE_CAPACITY_BODY_CALLS, store_capacity_thunk),
        (INFO_INDEX, &INFO_INDEX_BODY, &INFO_INDEX_BODY_CALLS, info_index_thunk),
        (IS_LETHAL, &IS_LETHAL_BODY, &IS_LETHAL_BODY_CALLS, is_lethal_thunk),
        (USES_SEWAGE, &SEWAGE_BODY, &SEWAGE_BODY_CALLS, uses_sewage_thunk),
        (USES_BULLET, &BULLET_BODY, &BULLET_BODY_CALLS, uses_bullet_thunk),
        (USES_PROJECTILE, &PROJECTILE_BODY, &PROJECTILE_BODY_CALLS, uses_projectile_thunk),
        (HAS_AMMO, &HAS_AMMO_BODY, &HAS_AMMO_BODY_CALLS, has_ammo_thunk),
        (AI_RANGE_SQ, &AI_RANGE_SQ_BODY, &AI_RANGE_SQ_BODY_CALLS, ai_range_sq_thunk),
        (AI_RANGE, &AI_RANGE_BODY, &AI_RANGE_BODY_CALLS, ai_range_thunk),
        (INSTANCE_FREE, &INSTANCE_FREE_BODY, &INSTANCE_FREE_BODY_CALLS, instance_free_thunk),
        (MAX_RESERVE, &MAX_RESERVE_BODY, &MAX_RESERVE_BODY_CALLS, max_reserve_thunk),
        (FOLEY_FIND, &FOLEY_BODY, &FOLEY_BODY_CALLS, foley_find_thunk),
        (LINE_FIND, &VOICE_LINE_BODY, &VOICE_LINE_BODY_CALLS, line_find_thunk),
        (MUSIC_FIND, &MUSIC_BODY, &MUSIC_BODY_CALLS, music_find_thunk),
        (SET_VOLUME, &SET_VOLUME_BODY, &SET_VOLUME_BODY_CALLS, set_volume_thunk),
        (GET_VOLUME, &GET_VOLUME_BODY, &GET_VOLUME_BODY_CALLS, get_volume_thunk),
        (STOP_CATEGORY, &STOP_CATEGORY_BODY, &STOP_CATEGORY_BODY_CALLS, stop_category_thunk),
        (MARK_DIRTY, &DIRTY_ALL_BODY, &DIRTY_ALL_BODY_CALLS, mark_dirty_thunk),
        (THROTTLE_SET, &THROTTLE_BODY, &THROTTLE_BODY_CALLS, throttle_thunk),
        (SLOT_FADE_OUT, &SLOT_FADE_OUT_BODY, &SLOT_FADE_OUT_BODY_CALLS, slot_fade_out_thunk),
        (FADE_OUT, &FADE_OUT_BODY, &FADE_OUT_BODY_CALLS, fade_out_thunk),
        (FADE_OUT2, &FADE_OUT2_BODY, &FADE_OUT2_BODY_CALLS, fade_out2_thunk),
        (SET_ORIENT, &ORIENT_BODY, &ORIENT_BODY_CALLS, orient_thunk),
        (SET_ORIENT2, &ORIENT2_BODY, &ORIENT2_BODY_CALLS, orient2_thunk),
        (IS_2D, &IS_2D_BODY, &IS_2D_BODY_CALLS, is_2d_thunk),
        (GET_P30, &GET_P30_BODY, &GET_P30_BODY_CALLS, get_param_30_thunk),
        (SET_P28, &SET_P28_BODY, &SET_P28_BODY_CALLS, set_param_28_thunk),
        (SET_P30, &SET_P30_BODY, &SET_P30_BODY_CALLS, set_param_30_thunk),
        (FADE_IN, &FADE_IN_BODY, &FADE_IN_BODY_CALLS, slot_fade_in_thunk),
        (PI_DEFAULT, &PI_DEFAULT_BODY, &PI_DEFAULT_BODY_CALLS, pi_default_thunk),
        (PI_BASIC, &PI_BASIC_BODY, &PI_BASIC_BODY_CALLS, pi_basic_thunk),
        (PI_VOLUME, &PI_VOLUME_BODY, &PI_VOLUME_BODY_CALLS, pi_volume_thunk),
        (PI_VARIANT, &PI_VARIANT_BODY, &PI_VARIANT_BODY_CALLS, pi_variant_thunk),
        (PI_VARIANT_VOLUME, &PI_VARIANT_VOLUME_BODY, &PI_VARIANT_VOLUME_BODY_CALLS, pi_variant_volume_thunk),
        (PI_EXT, &PI_EXT_BODY, &PI_EXT_BODY_CALLS, pi_ext_thunk),
        (PI_3D, &PI_3D_BODY, &PI_3D_BODY_CALLS, pi_3d_thunk),
        (PI_3D_CHAN, &PI_3D_CHAN_BODY, &PI_3D_CHAN_BODY_CALLS, pi_3d_chan_thunk),
        (PI_3D_CHAN2, &PI_3D_CHAN2_BODY, &PI_3D_CHAN2_BODY_CALLS, pi_3d_chan2_thunk),
        (PI_3D_CHAN_FLAG, &PI_3D_CHAN_FLAG_BODY, &PI_3D_CHAN_FLAG_BODY_CALLS, pi_3d_chan_flag_thunk),
        (PI_3D_VOLUME, &PI_3D_VOLUME_BODY, &PI_3D_VOLUME_BODY_CALLS, pi_3d_volume_thunk),
        (PI_3D_EXT, &PI_3D_EXT_BODY, &PI_3D_EXT_BODY_CALLS, pi_3d_ext_thunk),
        (PI_3D_VARIANT, &PI_3D_VARIANT_BODY, &PI_3D_VARIANT_BODY_CALLS, pi_3d_variant_thunk),
        (PI_3D_VARIANT_VOLUME, &PI_3D_VARIANT_VOLUME_BODY, &PI_3D_VARIANT_VOLUME_BODY_CALLS, pi_3d_variant_volume_thunk),
        (PI_3D_VARIANT_EXT, &PI_3D_VARIANT_EXT_BODY, &PI_3D_VARIANT_EXT_BODY_CALLS, pi_3d_variant_ext_thunk),
        (PAUSE_ALL, &PAUSE_ALL_BODY, &PAUSE_ALL_BODY_CALLS, pause_all_thunk),
        (RESUME_ALL, &RESUME_ALL_BODY, &RESUME_ALL_BODY_CALLS, resume_all_thunk),
        (FADE_ALL, &FADE_ALL_BODY, &FADE_ALL_BODY_CALLS, fade_all_thunk),
        (SET_3D_POS, &SET_3D_POS_BODY, &SET_3D_POS_BODY_CALLS, set_3d_position_thunk),
        (PLAY_PREPARED, &PLAY_PREPARED_BODY, &PLAY_PREPARED_BODY_CALLS, play_prepared_thunk),
        (ENTRY_ADD, &AMMO_ENTRY_ADD_BODY, &AMMO_ENTRY_ADD_BODY_CALLS, entry_add_thunk),
        (ADD_TYPE, &AMMO_ADD_TYPE_BODY, &AMMO_ADD_TYPE_BODY_CALLS, add_type_thunk),
        (STORE_REMOVE, &AMMO_REMOVE_BODY, &AMMO_REMOVE_BODY_CALLS, store_remove_thunk),
        (WEAPON_STORE_REMOVE, &WEAPON_AMMO_REMOVE_BODY, &WEAPON_AMMO_REMOVE_BODY_CALLS, weapon_store_remove_thunk),
        (STORE_CLEAR, &AMMO_CLEAR_BODY, &AMMO_CLEAR_BODY_CALLS, store_clear_thunk),
        (IS_THROWN, &IS_THROWN_BODY, &IS_THROWN_BODY_CALLS, is_thrown_thunk),
        (INST_ALLOC, &INST_ALLOC_BODY, &INST_ALLOC_BODY_CALLS, instance_alloc_thunk),
    ];
    // The shadow copies first (replacing patches the entries).
    for &(id, body, call_offsets, _) in &list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_weapons_audio: {}: no shadow copy: {e:?}", port.name)),
        }
    }
    // weapon_total_ammo's copy calls the copy of ammo_store_count, not the patched entry.
    let (total, count) = (PORTS[TOTAL_AMMO].original.load(Ordering::Relaxed), PORTS[STORE_COUNT].original.load(Ordering::Relaxed));
    if total != 0 && count != 0 {
        unsafe { set_rel32(total, 0x37, count) };
    } else {
        PORTS[TOTAL_AMMO].original.store(0, Ordering::Relaxed);
    }
    // The fade-out wrappers end in `jmp audio_voice_slot_fade_out`: their copies jump to the copy of
    // it, so a shadow run never goes through the Rust version of the slot function.
    let slot_copy = PORTS[SLOT_FADE_OUT].original.load(Ordering::Relaxed);
    for id in [FADE_OUT, FADE_OUT2] {
        let c = PORTS[id].original.load(Ordering::Relaxed);
        if c != 0 && slot_copy != 0 {
            unsafe { set_rel32(c, 0x2D, slot_copy) };
        } else {
            PORTS[id].original.store(0, Ordering::Relaxed);
        }
    }
    // Copies that call another ported function call that function's copy, so a shadow run of the
    // original never goes through a Rust version.
    for (id, off, callee) in [(FADE_ALL, 0x32usize, SLOT_FADE_OUT), (ADD_TYPE, 0x44, ENTRY_ADD), (WEAPON_STORE_REMOVE, 0x25, STORE_REMOVE)] {
        let (c, d) = (PORTS[id].original.load(Ordering::Relaxed), PORTS[callee].original.load(Ordering::Relaxed));
        if c != 0 && d != 0 {
            unsafe { set_rel32(c, off, d) };
        } else {
            PORTS[id].original.store(0, Ordering::Relaxed);
        }
    }
    let list2: [(usize, &[u8], &[usize], extern "C" fn(), bool); 27] = [
        (SLOT_ALLOC_ID, &SLOT_ALLOC_R3_BODY, &SLOT_ALLOC_R3_BODY_CALLS, slot_alloc_thunk, false),
        (START_2D_ID, &START_2D_R3_BODY, &START_2D_R3_BODY_CALLS, start_2d_thunk, false),
        (VOICE_START_ID, &VOICE_START_R3_BODY, &VOICE_START_R3_BODY_CALLS, voice_start_thunk, false),
        (VOICE_PREPARE_ID, &VOICE_PREPARE_R3_BODY, &VOICE_PREPARE_R3_BODY_CALLS, voice_prepare_thunk, false),
        (FOLEY_CUE_ID, &FOLEY_CUE_R3_BODY, &FOLEY_CUE_R3_BODY_CALLS, foley_cue_thunk, false),
        (VOICE_LINE_ID, &VOICE_LINE_R3_BODY, &VOICE_LINE_R3_BODY_CALLS, voice_line_thunk, false),
        (MUSIC_CUE_ID, &MUSIC_CUE_R3_BODY, &MUSIC_CUE_R3_BODY_CALLS, music_cue_thunk, false),
        (RELOAD_ID, &RELOAD_R3_BODY, &RELOAD_R3_BODY_CALLS, reload_thunk, true),
        (HANDLE_VALID_ID, &HANDLE_VALID_R3_BODY, &HANDLE_VALID_R3_BODY_CALLS, handle_valid_thunk, true),
        (INFO_IS_MELEE_ID, &INFO_IS_MELEE_R3_BODY, &INFO_IS_MELEE_R3_BODY_CALLS, info_is_melee_thunk, true),
        (CLASS_IS_MELEE_ID, &CLASS_IS_MELEE_R3_BODY, &CLASS_IS_MELEE_R3_BODY_CALLS, class_is_melee_thunk, true),
        (MOUNT_FIND_ID, &MOUNT_FIND_R3_BODY, &MOUNT_FIND_R3_BODY_CALLS, mount_find_thunk, true),
        (PLAY_FOLEY_2D_ID, &PLAY_FOLEY_2D_R3_BODY, &PLAY_FOLEY_2D_R3_BODY_CALLS, play_foley_2d_thunk, false),
        (PLAY_FOLEY_2D_TAGGED_ID, &PLAY_FOLEY_2D_TAGGED_R3_BODY, &PLAY_FOLEY_2D_TAGGED_R3_BODY_CALLS, play_foley_2d_tagged_thunk, false),
        (PLAY_FOLEY_3D_ID, &PLAY_FOLEY_3D_R3_BODY, &PLAY_FOLEY_3D_R3_BODY_CALLS, play_foley_3d_thunk, false),
        (PLAY_LINE_2D_ID, &PLAY_LINE_2D_R3_BODY, &PLAY_LINE_2D_R3_BODY_CALLS, play_voice_line_2d_thunk, false),
        (AUDIO_PREPARE_ID, &AUDIO_PREPARE_R3_BODY, &AUDIO_PREPARE_R3_BODY_CALLS, audio_prepare_thunk, false),
        (SET_LISTENER_ID, &SET_LISTENER_R3_BODY, &SET_LISTENER_R3_BODY_CALLS, set_listener_thunk, false),
        (STATION_STOP_ID, &STATION_STOP_R3_BODY, &STATION_STOP_R3_BODY_CALLS, station_stop_thunk, false),
        (LOCK_CLEAR_ID, &LOCK_CLEAR_R3_BODY, &LOCK_CLEAR_R3_BODY_CALLS, target_lock_clear_thunk, false),
        (REMOVE_RESERVE_ID, &REMOVE_RESERVE_R3_BODY, &REMOVE_RESERVE_R3_BODY_CALLS, remove_reserve_ammo_thunk, false),
        (CUE_ADD_ID, &CUE_ADD_R3_BODY, &CUE_ADD_R3_BODY_CALLS, cue_table_add_thunk, false),
        (RADIO_TRACK_FIND_ID, &RADIO_TRACK_FIND_R3_BODY, &RADIO_TRACK_FIND_R3_BODY_CALLS, radio_track_find_thunk, true),
        (RADIO_STATION_FIND_ID, &RADIO_STATION_FIND_R3_BODY, &RADIO_STATION_FIND_R3_BODY_CALLS, radio_station_find_thunk, true),
        (DJ_STOP_ID, &DJ_STOP_R3_BODY, &DJ_STOP_R3_BODY_CALLS, dj_voice_stop_thunk, false),
        (RADIO_OFF_ID, &RADIO_OFF_R3_BODY, &RADIO_OFF_R3_BODY_CALLS, vehicle_radio_turn_off_thunk, false),
        (RADIO_BLOCK_ID, &RADIO_BLOCK_R3_BODY, &RADIO_BLOCK_R3_BODY_CALLS, vehicle_radio_block_thunk, false),
    ];
    for &(id, body, call_offsets, _, shadowed) in &list2 {
        if !shadowed {
            continue;
        }
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_weapons_audio: {}: no shadow copy: {e:?}", port.name)),
        }
    }
    // The reload copy's two ammo_store_find calls go to the copy of that port.
    {
        let (c, d) = (PORTS[RELOAD_ID].original.load(Ordering::Relaxed), PORTS[STORE_FIND].original.load(Ordering::Relaxed));
        if c != 0 && d != 0 {
            unsafe {
                set_rel32(c, 0x23, d);
                set_rel32(c, 0x7A, d);
            }
        } else {
            PORTS[RELOAD_ID].original.store(0, Ordering::Relaxed);
        }
    }
    let mut ok = 0;
    for &(id, body, _, thunk, _) in &list2 {
        let port = &PORTS[id];
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    for &(id, body, _, thunk) in &list {
        let port = &PORTS[id];
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_weapons_audio: replaced {ok}/{COUNT} with Rust"));
}

/// One line for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    let mut br: Vec<String> = Vec::new();
    for id in OLD_COUNT..COUNT {
        let v: Vec<String> = (0..12).map(|i| BRANCH[id * 12 + i].load(Ordering::Relaxed)).map(|c| c.to_string()).collect();
        if v.iter().any(|c| c != "0") {
            br.push(format!("{} {}", PORTS[id].name, v.join("/")));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("port_weapons_audio: {} | branches {}", parts.join(", "), br.join(", "))
    }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const TOTAL_AMMO_BODY: [u8; 71] = [
    0x85, 0xF6, 0x75, 0x05, 0x33, 0xC0, 0xC2, 0x08, 0x00, 0x80, 0x7C, 0x24, 0x08, 0x00, 0x8B, 0x46,
    0x08, 0x75, 0x08, 0x8B, 0x80, 0xD4, 0x00, 0x00, 0x00, 0xEB, 0x06, 0x8B, 0x80, 0xD8, 0x00, 0x00,
    0x00, 0x85, 0xC0, 0x75, 0x0A, 0x8B, 0x46, 0x10, 0x33, 0xC9, 0x03, 0xC1, 0xC2, 0x08, 0x00, 0x8B,
    0x4C, 0x24, 0x04, 0x57, 0x8B, 0x78, 0x04, 0xE8, 0x54, 0xFE, 0xFF, 0xFF, 0x8B, 0xC8, 0x8B, 0x46,
    0x10, 0x5F, 0x03, 0xC1, 0xC2, 0x08, 0x00,
];
const TOTAL_AMMO_BODY_CALLS: [usize; 1] = [0x37];
const STORE_COUNT_BODY: [u8; 61] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x33, 0xC0, 0x85, 0xD2, 0x56, 0x7E, 0x24, 0x8B, 0x31,
    0x8B, 0xCE, 0x8B, 0x19, 0x39, 0x7B, 0x04, 0x74, 0x0F, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x0C, 0x3B,
    0xC2, 0x7C, 0xEF, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8D, 0x14, 0x40, 0x8D, 0x04, 0x96, 0x85, 0xC0,
    0x75, 0x05, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8B, 0x40, 0x04, 0x5E, 0x5B, 0xC3,
];
const STORE_COUNT_BODY_CALLS: [usize; 0] = [];
const STORE_FIND_BODY: [u8; 49] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x33, 0xC0, 0x85, 0xD2, 0x56, 0x7E, 0x15, 0x8B, 0x31,
    0x8B, 0xCE, 0x8B, 0x19, 0x39, 0x7B, 0x04, 0x74, 0x0F, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x0C, 0x3B,
    0xC2, 0x7C, 0xEF, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8D, 0x14, 0x40, 0x8D, 0x04, 0x96, 0x5E, 0x5B,
    0xC3,
];
const STORE_FIND_BODY_CALLS: [usize; 0] = [];
const STORE_CAPACITY_BODY: [u8; 100] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x33, 0xC0, 0x85, 0xD2, 0x56, 0x7E, 0x24, 0x8B, 0x31,
    0x8B, 0xCE, 0x8B, 0x19, 0x39, 0x7B, 0x04, 0x74, 0x0F, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x0C, 0x3B,
    0xC2, 0x7C, 0xEF, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8D, 0x14, 0x40, 0x8D, 0x04, 0x96, 0x85, 0xC0,
    0x75, 0x05, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8A, 0x15, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xFA, 0x01,
    0x8B, 0x08, 0x75, 0x15, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x75, 0x0B, 0x8B, 0x49,
    0x10, 0x8B, 0x40, 0x08, 0x5E, 0x03, 0xC1, 0x5B, 0xC3, 0x8B, 0x49, 0x0C, 0x8B, 0x40, 0x08, 0x5E,
    0x03, 0xC1, 0x5B, 0xC3,
];
const STORE_CAPACITY_BODY_CALLS: [usize; 0] = [];
const INFO_INDEX_BODY: [u8; 57] = [
    0x85, 0xC0, 0x74, 0x32, 0x8B, 0x0D, 0xCC, 0x7B, 0x2D, 0x02, 0x3B, 0xC1, 0x72, 0x28, 0x8B, 0x15,
    0xD0, 0x7B, 0x2D, 0x02, 0x69, 0xD2, 0x98, 0x04, 0x00, 0x00, 0x03, 0xD1, 0x3B, 0xC2, 0x73, 0x16,
    0x2B, 0xC1, 0x8B, 0xC8, 0xB8, 0x27, 0xAE, 0x74, 0x6F, 0xF7, 0xE9, 0xC1, 0xFA, 0x09, 0x8B, 0xC2,
    0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0xC3, 0x0C, 0xFF, 0xC3,
];
const INFO_INDEX_BODY_CALLS: [usize; 0] = [];
const IS_LETHAL_BODY: [u8; 67] = [
    0x85, 0xC9, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0xF7, 0x41, 0x10, 0x00, 0x04, 0x00, 0x00, 0x74, 0x03,
    0xB0, 0x01, 0xC3, 0x80, 0x7C, 0x24, 0x04, 0x00, 0x75, 0xEA, 0x8B, 0x81, 0xD4, 0x00, 0x00, 0x00,
    0x85, 0xC0, 0xB2, 0x04, 0x74, 0x05, 0x84, 0x50, 0x08, 0x75, 0x0F, 0x8B, 0x89, 0xD8, 0x00, 0x00,
    0x00, 0x85, 0xC9, 0x74, 0x0B, 0x84, 0x51, 0x08, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
    0x33, 0xC0, 0xC3,
];
const IS_LETHAL_BODY_CALLS: [usize; 0] = [];
const SEWAGE_BODY: [u8; 45] = [
    0x85, 0xC9, 0x74, 0x26, 0x8B, 0x81, 0xD4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0xB2, 0x80, 0x74, 0x05,
    0x84, 0x50, 0x08, 0x75, 0x0F, 0x8B, 0x81, 0xD8, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0B, 0x84,
    0x50, 0x08, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const SEWAGE_BODY_CALLS: [usize; 0] = [];
const BULLET_BODY: [u8; 45] = [
    0x85, 0xC9, 0x74, 0x26, 0x8B, 0x81, 0xD4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0xB2, 0x08, 0x74, 0x05,
    0x84, 0x50, 0x08, 0x75, 0x0F, 0x8B, 0x81, 0xD8, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0B, 0x84,
    0x50, 0x08, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const BULLET_BODY_CALLS: [usize; 0] = [];
const PROJECTILE_BODY: [u8; 29] = [
    0x85, 0xC0, 0x74, 0x16, 0x8B, 0x80, 0xD4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0C, 0xF6, 0x40,
    0x08, 0x10, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const PROJECTILE_BODY_CALLS: [usize; 0] = [];
const HAS_AMMO_BODY: [u8; 53] = [
    0x83, 0x78, 0x10, 0x00, 0x7F, 0x2C, 0x83, 0x78, 0x14, 0x00, 0x7F, 0x26, 0x8B, 0x48, 0x08, 0x85,
    0xC9, 0x74, 0x0A, 0x66, 0x83, 0xB9, 0xDC, 0x00, 0x00, 0x00, 0x00, 0x74, 0x15, 0xF6, 0x41, 0x10,
    0x20, 0x74, 0x0C, 0x83, 0x78, 0x18, 0x00, 0x7F, 0x09, 0x83, 0x78, 0x1C, 0x00, 0x7F, 0x03, 0x32,
    0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const HAS_AMMO_BODY_CALLS: [usize; 0] = [];
const AI_RANGE_SQ_BODY: [u8; 49] = [
    0x51, 0x85, 0xC0, 0x74, 0x16, 0xD9, 0x80, 0xE4, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04,
    0x24, 0xDC, 0xC8, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3, 0xD9, 0x05, 0x10, 0xC1, 0xE5,
    0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59,
    0xC3,
];
const AI_RANGE_SQ_BODY_CALLS: [usize; 0] = [];
const AI_RANGE_BODY: [u8; 33] = [
    0x51, 0x85, 0xC0, 0x74, 0x0E, 0xD9, 0x80, 0xE4, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04,
    0x24, 0x59, 0xC3, 0xD9, 0x05, 0x10, 0xC1, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59,
    0xC3,
];
const AI_RANGE_BODY_CALLS: [usize; 0] = [];
const INSTANCE_FREE_BODY: [u8; 99] = [
    0x39, 0x05, 0x6C, 0x7B, 0x2D, 0x02, 0x75, 0x17, 0x8B, 0x48, 0x04, 0x3B, 0xC8, 0x75, 0x0A, 0x33,
    0xD2, 0x89, 0x15, 0x6C, 0x7B, 0x2D, 0x02, 0xEB, 0x21, 0x89, 0x0D, 0x6C, 0x7B, 0x2D, 0x02, 0x8B,
    0x08, 0x33, 0xD2, 0x3B, 0xCA, 0x56, 0x74, 0x06, 0x8B, 0x70, 0x04, 0x89, 0x71, 0x04, 0x8B, 0x48,
    0x04, 0x3B, 0xCA, 0x74, 0x04, 0x8B, 0x30, 0x89, 0x31, 0x5E, 0x8B, 0x0D, 0xC4, 0x7B, 0x2D, 0x02,
    0x3B, 0xCA, 0x89, 0x10, 0x89, 0x50, 0x04, 0x75, 0x0B, 0x89, 0x40, 0x04, 0x89, 0x00, 0xA3, 0xC4,
    0x7B, 0x2D, 0x02, 0xC3, 0x8B, 0x11, 0x89, 0x10, 0x89, 0x48, 0x04, 0x8B, 0x11, 0x89, 0x42, 0x04,
    0x89, 0x01, 0xC3,
];
const INSTANCE_FREE_BODY_CALLS: [usize; 0] = [];
const MAX_RESERVE_BODY: [u8; 61] = [
    0x85, 0xC0, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x80, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x08, 0x8B, 0x80,
    0xD4, 0x00, 0x00, 0x00, 0xEB, 0x06, 0x8B, 0x80, 0xD8, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0xE4,
    0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01, 0x75, 0x0E, 0x8A, 0x15, 0x49, 0x8B, 0x52,
    0x02, 0x84, 0xD2, 0x75, 0x04, 0x8B, 0x40, 0x10, 0xC3, 0x8B, 0x40, 0x0C, 0xC3,
];
const MAX_RESERVE_BODY_CALLS: [usize; 0] = [];
const FOLEY_BODY: [u8; 45] = [
    0x66, 0x8B, 0x0D, 0xF0, 0x6A, 0x52, 0x02, 0x33, 0xC0, 0x66, 0x85, 0xC9, 0x56, 0x76, 0x18, 0x8B,
    0x12, 0x0F, 0xB7, 0xF0, 0x8B, 0x34, 0xB5, 0x58, 0x55, 0xEE, 0x00, 0x39, 0x16, 0x74, 0x0C, 0x83,
    0xC0, 0x01, 0x66, 0x3B, 0xC1, 0x72, 0xEA, 0x66, 0x0D, 0xFF, 0xFF, 0x5E, 0xC3,
];
const FOLEY_BODY_CALLS: [usize; 0] = [];
const VOICE_LINE_BODY: [u8; 45] = [
    0x66, 0x8B, 0x0D, 0xF8, 0x6A, 0x52, 0x02, 0x33, 0xC0, 0x66, 0x85, 0xC9, 0x56, 0x76, 0x18, 0x8B,
    0x12, 0x0F, 0xB7, 0xF0, 0xC1, 0xE6, 0x04, 0x39, 0x96, 0x60, 0x43, 0x57, 0x02, 0x74, 0x0C, 0x83,
    0xC0, 0x01, 0x66, 0x3B, 0xC1, 0x72, 0xEA, 0x66, 0x0D, 0xFF, 0xFF, 0x5E, 0xC3,
];
const VOICE_LINE_BODY_CALLS: [usize; 0] = [];
const MUSIC_BODY: [u8; 45] = [
    0x66, 0x8B, 0x0D, 0xF4, 0x6A, 0x52, 0x02, 0x33, 0xC0, 0x66, 0x85, 0xC9, 0x56, 0x76, 0x18, 0x8B,
    0x12, 0x0F, 0xB7, 0xF0, 0x8B, 0x34, 0xB5, 0x78, 0x71, 0xEE, 0x00, 0x39, 0x16, 0x74, 0x0C, 0x83,
    0xC0, 0x01, 0x66, 0x3B, 0xC1, 0x72, 0xEA, 0x66, 0x0D, 0xFF, 0xFF, 0x5E, 0xC3,
];
const MUSIC_BODY_CALLS: [usize; 0] = [];
const SET_VOLUME_BODY: [u8; 80] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x0F, 0xB7, 0x88, 0x8E, 0xBA, 0xEE,
    0x00, 0xD9, 0x44, 0x24, 0x04, 0xF7, 0xC1, 0x00, 0x08, 0x00, 0x00, 0xD9, 0x98, 0x1C, 0xB9, 0xEE,
    0x00, 0x75, 0x0A, 0x83, 0xC9, 0x01, 0x66, 0x89, 0x88, 0x8E, 0xBA, 0xEE, 0x00, 0xB0, 0x01, 0xC3,
];
const SET_VOLUME_BODY_CALLS: [usize; 0] = [];
const GET_VOLUME_BODY: [u8; 64] = [
    0x51, 0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6,
    0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83,
    0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x0A, 0xD9, 0xE8, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24,
    0x59, 0xC3, 0xD9, 0x80, 0x1C, 0xB9, 0xEE, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
const GET_VOLUME_BODY_CALLS: [usize; 0] = [];
const STOP_CATEGORY_BODY: [u8; 210] = [
    0x53, 0x55, 0x56, 0x57, 0xB8, 0x90, 0xBA, 0xEE, 0x00, 0xBD, 0x20, 0x00, 0x00, 0x00, 0x83, 0xCF,
    0xFF, 0xBB, 0x00, 0x02, 0x00, 0x00, 0xBE, 0x00, 0x04, 0x00, 0x00, 0xB9, 0x7F, 0xFF, 0x00, 0x00,
    0x39, 0xB8, 0x68, 0xFE, 0xFF, 0xFF, 0x75, 0x06, 0x66, 0x85, 0x58, 0xFE, 0x74, 0x13, 0x39, 0x90,
    0x0C, 0xFF, 0xFF, 0xFF, 0x74, 0x04, 0x3B, 0xD7, 0x75, 0x07, 0x66, 0x09, 0x30, 0x66, 0x21, 0x48,
    0xFC, 0x39, 0x78, 0x04, 0x75, 0x09, 0x66, 0x85, 0x98, 0x9A, 0x01, 0x00, 0x00, 0x74, 0x1A, 0x39,
    0x90, 0xA8, 0x00, 0x00, 0x00, 0x74, 0x04, 0x3B, 0xD7, 0x75, 0x0E, 0x66, 0x09, 0xB0, 0x9C, 0x01,
    0x00, 0x00, 0x66, 0x21, 0x88, 0x98, 0x01, 0x00, 0x00, 0x39, 0xB8, 0xA0, 0x01, 0x00, 0x00, 0x75,
    0x09, 0x66, 0x85, 0x98, 0x36, 0x03, 0x00, 0x00, 0x74, 0x1A, 0x39, 0x90, 0x44, 0x02, 0x00, 0x00,
    0x74, 0x04, 0x3B, 0xD7, 0x75, 0x0E, 0x66, 0x09, 0xB0, 0x38, 0x03, 0x00, 0x00, 0x66, 0x21, 0x88,
    0x34, 0x03, 0x00, 0x00, 0x39, 0xB8, 0x3C, 0x03, 0x00, 0x00, 0x75, 0x09, 0x66, 0x85, 0x98, 0xD2,
    0x04, 0x00, 0x00, 0x74, 0x1A, 0x39, 0x90, 0xE0, 0x03, 0x00, 0x00, 0x74, 0x04, 0x3B, 0xD7, 0x75,
    0x0E, 0x66, 0x09, 0xB0, 0xD4, 0x04, 0x00, 0x00, 0x66, 0x21, 0x88, 0xD0, 0x04, 0x00, 0x00, 0x05,
    0x70, 0x06, 0x00, 0x00, 0x83, 0xED, 0x01, 0x0F, 0x85, 0x53, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D,
    0x5B, 0xC3,
];
const STOP_CATEGORY_BODY_CALLS: [usize; 0] = [];
const DIRTY_ALL_BODY: [u8; 202] = [
    0x56, 0x33, 0xC0, 0x83, 0xCE, 0xFF, 0xBA, 0x00, 0x02, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x39, 0xB0, 0xF8, 0xB8, 0xEE, 0x00, 0x75, 0x09, 0x66, 0x85, 0x90, 0x8E, 0xBA, 0xEE, 0x00, 0x74,
    0x19, 0x0F, 0xB7, 0x88, 0x8E, 0xBA, 0xEE, 0x00, 0xF7, 0xC1, 0x00, 0x08, 0x00, 0x00, 0x75, 0x0A,
    0x83, 0xC9, 0x01, 0x66, 0x89, 0x88, 0x8E, 0xBA, 0xEE, 0x00, 0x39, 0xB0, 0x94, 0xBA, 0xEE, 0x00,
    0x75, 0x09, 0x66, 0x85, 0x90, 0x2A, 0xBC, 0xEE, 0x00, 0x74, 0x19, 0x0F, 0xB7, 0x88, 0x2A, 0xBC,
    0xEE, 0x00, 0xF7, 0xC1, 0x00, 0x08, 0x00, 0x00, 0x75, 0x0A, 0x83, 0xC9, 0x01, 0x66, 0x89, 0x88,
    0x2A, 0xBC, 0xEE, 0x00, 0x39, 0xB0, 0x30, 0xBC, 0xEE, 0x00, 0x75, 0x09, 0x66, 0x85, 0x90, 0xC6,
    0xBD, 0xEE, 0x00, 0x74, 0x19, 0x0F, 0xB7, 0x88, 0xC6, 0xBD, 0xEE, 0x00, 0xF7, 0xC1, 0x00, 0x08,
    0x00, 0x00, 0x75, 0x0A, 0x83, 0xC9, 0x01, 0x66, 0x89, 0x88, 0xC6, 0xBD, 0xEE, 0x00, 0x39, 0xB0,
    0xCC, 0xBD, 0xEE, 0x00, 0x75, 0x09, 0x66, 0x85, 0x90, 0x62, 0xBF, 0xEE, 0x00, 0x74, 0x19, 0x0F,
    0xB7, 0x88, 0x62, 0xBF, 0xEE, 0x00, 0xF7, 0xC1, 0x00, 0x08, 0x00, 0x00, 0x75, 0x0A, 0x83, 0xC9,
    0x01, 0x66, 0x89, 0x88, 0x62, 0xBF, 0xEE, 0x00, 0x05, 0x70, 0x06, 0x00, 0x00, 0x3D, 0x00, 0xCE,
    0x00, 0x00, 0x0F, 0x82, 0x48, 0xFF, 0xFF, 0xFF, 0x5E, 0xC3,
];
const DIRTY_ALL_BODY_CALLS: [usize; 0] = [];
const THROTTLE_BODY: [u8; 44] = [
    0x83, 0xF9, 0x04, 0x73, 0x26, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD,
    0xD9, 0xF6, 0xC4, 0x01, 0x75, 0x13, 0xD9, 0xE8, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x75,
    0x08, 0xD9, 0x1C, 0x8D, 0xF8, 0x34, 0xEE, 0x00, 0xC3, 0xDD, 0xD8, 0xC3,
];
const THROTTLE_BODY_CALLS: [usize; 0] = [];
const FADE_OUT_BODY: [u8; 51] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x2C, 0x8B, 0x11, 0xC1, 0xEA, 0x08, 0x74, 0x25, 0x0F, 0xB6, 0xC0,
    0x8B, 0xC8, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x91, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x12,
    0x83, 0xB9, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x74, 0x09, 0x8B, 0x54, 0x24, 0x04, 0xE9, 0x9E, 0x67,
    0x01, 0x00, 0xC3,
];
const FADE_OUT_BODY_CALLS: [usize; 0] = [];
const FADE_OUT2_BODY: [u8; 51] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x2C, 0x8B, 0x11, 0xC1, 0xEA, 0x08, 0x74, 0x25, 0x0F, 0xB6, 0xC0,
    0x8B, 0xC8, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x91, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x12,
    0x83, 0xB9, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x74, 0x09, 0x8B, 0x54, 0x24, 0x04, 0xE9, 0x8E, 0xC7,
    0xFF, 0xFF, 0xC3,
];
const FADE_OUT2_BODY_CALLS: [usize; 0] = [];
const ORIENT_BODY: [u8; 57] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x32, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x2B, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x1A, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x8D, 0x80, 0xF8, 0xB8, 0xEE, 0x00, 0x74, 0x0B, 0x8B, 0x4C, 0x24,
    0x04, 0x50, 0xE8, 0x99, 0x73, 0x01, 0x00, 0x59, 0xC3,
];
const ORIENT_BODY_CALLS: [usize; 1] = [0x32];
const ORIENT2_BODY: [u8; 57] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x32, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x2B, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x1A, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x8D, 0x80, 0xF8, 0xB8, 0xEE, 0x00, 0x74, 0x0B, 0x8B, 0x4C, 0x24,
    0x04, 0x50, 0xE8, 0xB9, 0xCF, 0xFF, 0xFF, 0x59, 0xC3,
];
const ORIENT2_BODY_CALLS: [usize; 1] = [0x32];
const SLOT_FADE_OUT_BODY: [u8; 120] = [
    0x51, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x05, 0xF8, 0xB8, 0xEE, 0x00, 0x8B, 0xC8, 0x0F, 0xB7,
    0x81, 0x96, 0x01, 0x00, 0x00, 0x84, 0xC0, 0x78, 0x08, 0xD9, 0xEE, 0xD9, 0x99, 0xC4, 0x00, 0x00,
    0x00, 0xD9, 0x41, 0x1C, 0x25, 0xFF, 0xFE, 0x00, 0x00, 0x0D, 0x80, 0x00, 0x00, 0x00, 0x83, 0xFA,
    0x20, 0x66, 0x89, 0x81, 0x96, 0x01, 0x00, 0x00, 0x73, 0x08, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00,
    0xEB, 0x15, 0xC1, 0xEA, 0x05, 0x85, 0xD2, 0x89, 0x14, 0x24, 0xDB, 0x04, 0x24, 0x7D, 0x06, 0xD8,
    0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xDE, 0xF9, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x81, 0xA8,
    0x00, 0x00, 0x00, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x08, 0xD9, 0x99, 0xA8, 0x00,
    0x00, 0x00, 0x59, 0xC3, 0xDD, 0xD8, 0x59, 0xC3,
];
const SLOT_FADE_OUT_BODY_CALLS: [usize; 0] = [];
const PI_DEFAULT_BODY: [u8; 67] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x56, 0xC3, 0x83, 0x00, 0xD9, 0xE8, 0xD9, 0x56, 0x0C, 0x83,
    0xC4, 0x0C, 0xD9, 0x5E, 0x10, 0x66, 0xC7, 0x06, 0xFF, 0xFF, 0xC7, 0x46, 0x18, 0xFF, 0xFF, 0xFF,
    0xFF, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0xC7,
    0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x4C, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00,
    0x00, 0x00, 0xC3,
];
const PI_DEFAULT_BODY_CALLS: [usize; 1] = [0x5];
const PI_BASIC_BODY: [u8; 74] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x96, 0xC2, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x44, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x4C, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0x8B, 0x54, 0x24, 0x18, 0x66,
    0x89, 0x06, 0x83, 0xC4, 0x0C, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x46, 0x30,
    0xFF, 0xFF, 0x66, 0xC7, 0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x89,
    0x4E, 0x18, 0x89, 0x56, 0x4C, 0x8B, 0xC6, 0xC2, 0x0C, 0x00,
];
const PI_BASIC_BODY_CALLS: [usize; 1] = [0x5];
const PI_VOLUME_BODY: [u8; 78] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x46, 0xC2, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x44, 0x24,
    0x10, 0xD9, 0x5E, 0x10, 0x8B, 0x4C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x1C, 0x8B, 0x54, 0x24, 0x18,
    0xD9, 0x5E, 0x0C, 0x66, 0x89, 0x06, 0x83, 0xC4, 0x0C, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00,
    0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0xC7, 0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x54, 0x00,
    0x00, 0x00, 0x00, 0x89, 0x4E, 0x18, 0x89, 0x56, 0x4C, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_VOLUME_BODY_CALLS: [usize; 1] = [0x5];
const PI_VARIANT_BODY: [u8; 74] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xF6, 0xC1, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x44, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x4C, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0x8B, 0x54, 0x24, 0x18, 0x66,
    0x89, 0x06, 0x8B, 0x44, 0x24, 0x1C, 0x89, 0x46, 0x28, 0x83, 0xC4, 0x0C, 0x66, 0xC7, 0x46, 0x30,
    0xFF, 0xFF, 0x66, 0xC7, 0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x89,
    0x4E, 0x18, 0x89, 0x56, 0x4C, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_VARIANT_BODY_CALLS: [usize; 1] = [0x5];
const PI_VARIANT_VOLUME_BODY: [u8; 78] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xA6, 0xC1, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x44, 0x24,
    0x10, 0xD9, 0x5E, 0x10, 0x8B, 0x4C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x1C, 0x8B, 0x54, 0x24, 0x18,
    0xD9, 0x5E, 0x0C, 0x66, 0x89, 0x06, 0x83, 0xC4, 0x0C, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66,
    0xC7, 0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x18, 0x00,
    0x00, 0x00, 0x00, 0x89, 0x4E, 0x4C, 0x89, 0x56, 0x28, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_VARIANT_VOLUME_BODY_CALLS: [usize; 1] = [0x5];
const PI_EXT_BODY: [u8; 94] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x56, 0xC1, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x44, 0x24,
    0x10, 0xD9, 0x5E, 0x10, 0x8B, 0x4C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x1C, 0x8B, 0x54, 0x24, 0x18,
    0xD9, 0x5E, 0x0C, 0x66, 0x89, 0x06, 0x66, 0x8B, 0x44, 0x24, 0x20, 0x89, 0x4E, 0x18, 0x8B, 0x4C,
    0x24, 0x24, 0x66, 0x89, 0x46, 0x02, 0x83, 0xC4, 0x0C, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00,
    0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0xC7, 0x46, 0x32, 0x01, 0x00, 0xC7, 0x46, 0x54, 0x00,
    0x00, 0x00, 0x00, 0x89, 0x56, 0x4C, 0x89, 0x4E, 0x04, 0x8B, 0xC6, 0xC2, 0x18, 0x00,
];
const PI_EXT_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_BODY: [u8; 98] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xF6, 0xC0, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0x66, 0x89, 0x0E, 0x8B, 0x4C,
    0x24, 0x18, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0xC7, 0x46, 0x28, 0x00, 0x00,
    0x00, 0x00, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x89,
    0x56, 0x18, 0x89, 0x4E, 0x4C, 0x8B, 0x17, 0x89, 0x56, 0x1C, 0x8B, 0x4F, 0x04, 0x89, 0x4E, 0x20,
    0x8B, 0x57, 0x08, 0x66, 0x09, 0x46, 0x58, 0x83, 0xC4, 0x0C, 0x89, 0x56, 0x24, 0x8B, 0xC6, 0xC2,
    0x0C, 0x00,
];
const PI_3D_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_CHAN_BODY: [u8; 103] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x86, 0xC0, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0x89, 0x46, 0x32, 0x89, 0x46, 0x18, 0x89, 0x56, 0x4C,
    0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x66, 0x89,
    0x0E, 0x8B, 0x0F, 0x89, 0x4E, 0x1C, 0x8B, 0x57, 0x04, 0x89, 0x56, 0x20, 0x8B, 0x4F, 0x08, 0x66,
    0x8B, 0x54, 0x24, 0x18, 0x66, 0x09, 0x46, 0x58, 0x83, 0xC4, 0x0C, 0x89, 0x4E, 0x24, 0x66, 0x89,
    0x56, 0x30, 0x8B, 0xC6, 0xC2, 0x0C, 0x00,
];
const PI_3D_CHAN_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_CHAN2_BODY: [u8; 112] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x16, 0xC0, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x66, 0x89, 0x46, 0x32, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0x89, 0x0E, 0x89, 0x46, 0x18,
    0x89, 0x56, 0x4C, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00,
    0x00, 0x8B, 0x0F, 0x89, 0x4E, 0x1C, 0x8B, 0x57, 0x04, 0x89, 0x56, 0x20, 0x8B, 0x4F, 0x08, 0x66,
    0x8B, 0x54, 0x24, 0x18, 0x66, 0x09, 0x46, 0x58, 0x89, 0x4E, 0x24, 0x66, 0x8B, 0x4C, 0x24, 0x1C,
    0x83, 0xC4, 0x0C, 0x66, 0x89, 0x56, 0x30, 0x66, 0x89, 0x4E, 0x32, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_3D_CHAN2_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_CHAN_FLAG_BODY: [u8; 137] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xA6, 0xBF, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0x89, 0x56, 0x4C, 0xB8, 0x01,
    0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0x66, 0x89, 0x0E,
    0x89, 0x46, 0x18, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00,
    0x00, 0x8B, 0x0F, 0x89, 0x4E, 0x1C, 0x8B, 0x57, 0x04, 0x89, 0x56, 0x20, 0x8B, 0x4F, 0x08, 0x66,
    0x8B, 0x54, 0x24, 0x18, 0x66, 0x89, 0x56, 0x30, 0x33, 0xD2, 0x83, 0xC4, 0x0C, 0x38, 0x54, 0x24,
    0x14, 0x89, 0x4E, 0x24, 0x66, 0x8B, 0x4C, 0x24, 0x10, 0x0F, 0x95, 0xC2, 0xC0, 0xE2, 0x06, 0x32,
    0x56, 0x58, 0x66, 0x89, 0x4E, 0x32, 0x83, 0xE2, 0x40, 0x66, 0x33, 0x56, 0x58, 0x66, 0x0B, 0xD0,
    0x66, 0x89, 0x56, 0x58, 0x8B, 0xC6, 0xC2, 0x14, 0x00,
];
const PI_3D_CHAN_FLAG_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_VOLUME_BODY: [u8; 105] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x16, 0xBF, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xD9, 0x44, 0x24, 0x1C, 0x66,
    0x89, 0x0E, 0x8B, 0x4C, 0x24, 0x18, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0xC7,
    0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00,
    0x00, 0x00, 0x00, 0x89, 0x56, 0x18, 0x89, 0x4E, 0x4C, 0x8B, 0x17, 0x89, 0x56, 0x1C, 0x8B, 0x4F,
    0x04, 0x89, 0x4E, 0x20, 0x8B, 0x57, 0x08, 0xD9, 0x5E, 0x0C, 0x66, 0x09, 0x46, 0x58, 0x83, 0xC4,
    0x0C, 0x89, 0x56, 0x24, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_3D_VOLUME_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_EXT_BODY: [u8; 121] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xA6, 0xBE, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xD9, 0x44, 0x24, 0x1C, 0x66,
    0x89, 0x0E, 0x8B, 0x4C, 0x24, 0x18, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0x89,
    0x56, 0x18, 0x89, 0x4E, 0x4C, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x46, 0x30,
    0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x17, 0x89, 0x56, 0x1C, 0x8B, 0x4F,
    0x04, 0x89, 0x4E, 0x20, 0x8B, 0x57, 0x08, 0xD9, 0x5E, 0x0C, 0x66, 0x8B, 0x4C, 0x24, 0x20, 0x66,
    0x09, 0x46, 0x58, 0x89, 0x56, 0x24, 0x8B, 0x54, 0x24, 0x24, 0x83, 0xC4, 0x0C, 0x66, 0x89, 0x4E,
    0x02, 0x89, 0x56, 0x04, 0x8B, 0xC6, 0xC2, 0x18, 0x00,
];
const PI_3D_EXT_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_VARIANT_BODY: [u8; 105] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x26, 0xBE, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0x89, 0x56, 0x4C, 0x66, 0xC7,
    0x46, 0x30, 0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x66, 0x89, 0x0E, 0xC7, 0x46,
    0x18, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x0F, 0x89, 0x4E, 0x1C, 0x8B, 0x57, 0x04, 0x89, 0x56, 0x20,
    0x8B, 0x4F, 0x08, 0x8B, 0x54, 0x24, 0x18, 0x66, 0x09, 0x46, 0x58, 0x83, 0xC4, 0x0C, 0x89, 0x4E,
    0x24, 0x89, 0x56, 0x28, 0x8B, 0xC6, 0xC2, 0x0C, 0x00,
];
const PI_3D_VARIANT_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_VARIANT_VOLUME_BODY: [u8; 112] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0xB6, 0xBD, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xD9, 0x44, 0x24, 0x1C, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0x89,
    0x56, 0x4C, 0x66, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x66,
    0x89, 0x0E, 0xC7, 0x46, 0x18, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x0F, 0x89, 0x4E, 0x1C, 0x8B, 0x57,
    0x04, 0x89, 0x56, 0x20, 0x8B, 0x4F, 0x08, 0xD9, 0x5E, 0x0C, 0x8B, 0x54, 0x24, 0x18, 0x66, 0x09,
    0x46, 0x58, 0x83, 0xC4, 0x0C, 0x89, 0x4E, 0x24, 0x89, 0x56, 0x28, 0x8B, 0xC6, 0xC2, 0x10, 0x00,
];
const PI_3D_VARIANT_VOLUME_BODY_CALLS: [usize; 1] = [0x5];
const PI_3D_VARIANT_EXT_BODY: [u8; 128] = [
    0x6A, 0x5C, 0x6A, 0x00, 0x56, 0xE8, 0x46, 0xBD, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24,
    0x10, 0xD9, 0x56, 0x0C, 0x8B, 0x54, 0x24, 0x14, 0xD9, 0x5E, 0x10, 0xD9, 0x44, 0x24, 0x20, 0x66,
    0x89, 0x0E, 0x8B, 0x4C, 0x24, 0x18, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x89, 0x4E, 0x4C, 0xC7, 0x46,
    0x28, 0x00, 0x00, 0x00, 0x00, 0x66, 0x89, 0x46, 0x32, 0x89, 0x56, 0x18, 0x66, 0xC7, 0x46, 0x30,
    0xFF, 0xFF, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x17, 0x89, 0x56, 0x1C, 0x8B, 0x4F,
    0x04, 0x89, 0x4E, 0x20, 0x8B, 0x57, 0x08, 0xD9, 0x5E, 0x0C, 0x8B, 0x4C, 0x24, 0x1C, 0x66, 0x09,
    0x46, 0x58, 0x89, 0x56, 0x24, 0x66, 0x8B, 0x54, 0x24, 0x24, 0x89, 0x4E, 0x28, 0x8B, 0x4C, 0x24,
    0x28, 0x83, 0xC4, 0x0C, 0x66, 0x89, 0x56, 0x02, 0x89, 0x4E, 0x04, 0x8B, 0xC6, 0xC2, 0x1C, 0x00,
];
const PI_3D_VARIANT_EXT_BODY_CALLS: [usize; 1] = [0x5];
const PAUSE_ALL_BODY: [u8; 234] = [
    0x80, 0x3D, 0x06, 0xA4, 0x52, 0x02, 0x00, 0x75, 0x37, 0x80, 0x3D, 0xFB, 0x6A, 0x52, 0x02, 0x01,
    0x74, 0x26, 0x6A, 0x01, 0xE8, 0xF7, 0x2D, 0x00, 0x00, 0xD9, 0xEE, 0xA1, 0x30, 0x2E, 0xF5, 0x00,
    0x8B, 0x08, 0x8B, 0x51, 0x28, 0x83, 0xC4, 0x04, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x50, 0xC6,
    0x05, 0xFB, 0x6A, 0x52, 0x02, 0x01, 0xFF, 0xD2, 0xC6, 0x05, 0xD7, 0x6A, 0x52, 0x02, 0x01, 0xC3,
    0x83, 0x05, 0x44, 0x6B, 0x52, 0x02, 0x01, 0x53, 0x55, 0x56, 0x57, 0xB8, 0x90, 0xBA, 0xEE, 0x00,
    0xBF, 0x80, 0x00, 0x00, 0x00, 0x83, 0xCD, 0xFF, 0xB3, 0x40, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0xA8, 0x68, 0xFE, 0xFF, 0xFF, 0x75, 0x08, 0x66, 0xF7, 0x40, 0xFE, 0x00, 0x02, 0x74, 0x67,
    0x39, 0xA8, 0x0C, 0xFF, 0xFF, 0xFF, 0x75, 0x24, 0x66, 0x8B, 0x08, 0x66, 0x81, 0xE1, 0xFF, 0xBF,
    0x84, 0x58, 0xFE, 0x0F, 0xB7, 0xD1, 0x66, 0x89, 0x08, 0x75, 0x4C, 0xF6, 0x40, 0xFC, 0x01, 0x75,
    0x46, 0x81, 0xCA, 0x00, 0x20, 0x00, 0x00, 0x66, 0x89, 0x10, 0xEB, 0x3B, 0x0F, 0xB7, 0x50, 0xFC,
    0xF6, 0xC2, 0x02, 0x74, 0x11, 0x66, 0x81, 0x08, 0x00, 0x04, 0x81, 0xE2, 0x7F, 0xFF, 0x00, 0x00,
    0x66, 0x89, 0x50, 0xFC, 0xEB, 0x21, 0x66, 0x8B, 0x08, 0x66, 0x81, 0xE1, 0xFF, 0xBF, 0x84, 0x58,
    0xFE, 0x0F, 0xB7, 0xF1, 0x66, 0x89, 0x08, 0x75, 0x0E, 0xF6, 0xC2, 0x01, 0x75, 0x09, 0x81, 0xCE,
    0x00, 0x20, 0x00, 0x00, 0x66, 0x89, 0x30, 0x05, 0x9C, 0x01, 0x00, 0x00, 0x83, 0xEF, 0x01, 0x0F,
    0x85, 0x7B, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3,
];
const PAUSE_ALL_BODY_CALLS: [usize; 1] = [0x14];
const RESUME_ALL_BODY: [u8; 137] = [
    0x53, 0x33, 0xDB, 0x38, 0x1D, 0xD7, 0x6A, 0x52, 0x02, 0x74, 0x33, 0x38, 0x1D, 0xFB, 0x6A, 0x52,
    0x02, 0x74, 0x23, 0x53, 0xE8, 0x07, 0x2D, 0x00, 0x00, 0xD9, 0xE8, 0xA1, 0x30, 0x2E, 0xF5, 0x00,
    0x8B, 0x08, 0x8B, 0x51, 0x28, 0x83, 0xC4, 0x04, 0x53, 0x51, 0xD9, 0x1C, 0x24, 0x50, 0x88, 0x1D,
    0xFB, 0x6A, 0x52, 0x02, 0xFF, 0xD2, 0x88, 0x1D, 0xD7, 0x6A, 0x52, 0x02, 0x5B, 0xC3, 0x83, 0x2D,
    0x44, 0x6B, 0x52, 0x02, 0x01, 0x79, 0x08, 0x89, 0x1D, 0x44, 0x6B, 0x52, 0x02, 0xEB, 0x08, 0x39,
    0x1D, 0x44, 0x6B, 0x52, 0x02, 0x7F, 0x30, 0xB8, 0x90, 0xBA, 0xEE, 0x00, 0xB9, 0x80, 0x00, 0x00,
    0x00, 0xB3, 0x40, 0x83, 0xCA, 0xFF, 0x56, 0x84, 0x58, 0xFE, 0x74, 0x10, 0x66, 0x8B, 0x30, 0x66,
    0x81, 0xE6, 0xFF, 0xDF, 0x66, 0x81, 0xCE, 0x00, 0x40, 0x66, 0x89, 0x30, 0x05, 0x9C, 0x01, 0x00,
    0x00, 0x83, 0xE9, 0x01, 0x75, 0xE1, 0x5E, 0x5B, 0xC3,
];
const RESUME_ALL_BODY_CALLS: [usize; 1] = [0x14];
const FADE_ALL_BODY: [u8; 76] = [
    0x83, 0x05, 0x44, 0x6B, 0x52, 0x02, 0x01, 0x55, 0x8B, 0x6C, 0x24, 0x08, 0x56, 0x57, 0x33, 0xFF,
    0x33, 0xF6, 0x83, 0xBE, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x0B, 0x66, 0xF7, 0x86, 0x8E, 0xBA,
    0xEE, 0x00, 0x00, 0x02, 0x74, 0x11, 0x66, 0x83, 0x8E, 0x8C, 0xBA, 0xEE, 0x00, 0x01, 0x8B, 0xD5,
    0x8B, 0xC7, 0xE8, 0xE9, 0xD6, 0xFF, 0xFF, 0x81, 0xC6, 0x9C, 0x01, 0x00, 0x00, 0x83, 0xC7, 0x01,
    0x81, 0xFE, 0x00, 0xCE, 0x00, 0x00, 0x72, 0xCA, 0x5F, 0x5E, 0x5D, 0xC3,
];
const FADE_ALL_BODY_CALLS: [usize; 1] = [0x32];
const IS_2D_BODY: [u8; 58] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x66, 0x8B, 0x80, 0x8E, 0xBA, 0xEE,
    0x00, 0x66, 0xC1, 0xE8, 0x0B, 0xF6, 0xD0, 0x24, 0x01, 0xC3,
];
const IS_2D_BODY_CALLS: [usize; 0] = [];
const GET_P30_BODY: [u8; 64] = [
    0x51, 0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6,
    0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83,
    0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x0A, 0xD9, 0xEE, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24,
    0x59, 0xC3, 0xD9, 0x80, 0x28, 0xB9, 0xEE, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
const GET_P30_BODY_CALLS: [usize; 0] = [];
const SET_P28_BODY: [u8; 107] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x66, 0x83, 0x88, 0x8E, 0xBA, 0xEE,
    0x00, 0x02, 0xD9, 0x44, 0x24, 0x04, 0x0F, 0xB6, 0x88, 0x8C, 0xBA, 0xEE, 0x00, 0xD9, 0x98, 0x20,
    0xB9, 0xEE, 0x00, 0x56, 0x0F, 0xB7, 0xB0, 0x8E, 0xBA, 0xEE, 0x00, 0x0F, 0xB7, 0xD6, 0xC1, 0xE9,
    0x03, 0xC1, 0xEA, 0x0B, 0x83, 0xE1, 0x01, 0xF7, 0xD2, 0x84, 0xCA, 0x74, 0x0A, 0x83, 0xCE, 0x01,
    0x66, 0x89, 0xB0, 0x8E, 0xBA, 0xEE, 0x00, 0xB0, 0x01, 0x5E, 0xC3,
];
const SET_P28_BODY_CALLS: [usize; 0] = [];
const SET_P30_BODY: [u8; 107] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x66, 0x83, 0x88, 0x8E, 0xBA, 0xEE,
    0x00, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x0F, 0xB6, 0x88, 0x8C, 0xBA, 0xEE, 0x00, 0xD9, 0x98, 0x28,
    0xB9, 0xEE, 0x00, 0x56, 0x0F, 0xB7, 0xB0, 0x8E, 0xBA, 0xEE, 0x00, 0x0F, 0xB7, 0xD6, 0xC1, 0xE9,
    0x04, 0xC1, 0xEA, 0x0B, 0x83, 0xE1, 0x01, 0xF7, 0xD2, 0x84, 0xCA, 0x74, 0x0A, 0x83, 0xCE, 0x01,
    0x66, 0x89, 0xB0, 0x8E, 0xBA, 0xEE, 0x00, 0xB0, 0x01, 0x5E, 0xC3,
];
const SET_P30_BODY_CALLS: [usize; 0] = [];
const FADE_IN_BODY: [u8; 90] = [
    0x51, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x81, 0xC1, 0xF8, 0xB8, 0xEE, 0x00, 0x0F, 0xB7, 0x81,
    0x96, 0x01, 0x00, 0x00, 0xA9, 0x00, 0x01, 0x00, 0x00, 0x75, 0x3D, 0xD9, 0x44, 0x24, 0x0C, 0x25,
    0x7F, 0xFF, 0x00, 0x00, 0x0D, 0x00, 0x01, 0x00, 0x00, 0xD9, 0x51, 0x1C, 0xD9, 0xEE, 0x66, 0x89,
    0x81, 0x96, 0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x08, 0xD9, 0x99, 0xC4, 0x00, 0x00, 0x00, 0xD9,
    0xE8, 0x99, 0x83, 0xE2, 0x1F, 0xDE, 0xE1, 0x03, 0xC2, 0xC1, 0xF8, 0x05, 0x89, 0x04, 0x24, 0xDA,
    0x34, 0x24, 0xD9, 0x99, 0xA8, 0x00, 0x00, 0x00, 0x59, 0xC3,
];
const FADE_IN_BODY_CALLS: [usize; 0] = [];
const SET_3D_POS_BODY: [u8; 157] = [
    0x51, 0x8A, 0x01, 0x3C, 0xFF, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x56, 0x74, 0x27, 0x8B, 0x09, 0xC1,
    0xE9, 0x08, 0x74, 0x20, 0x0F, 0xB6, 0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC,
    0xB9, 0xEE, 0x00, 0x75, 0x0F, 0x83, 0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x8D, 0xB0, 0xF8, 0xB8,
    0xEE, 0x00, 0x75, 0x06, 0x32, 0xC0, 0x5E, 0x5B, 0x59, 0xC3, 0x8B, 0x4C, 0x24, 0x14, 0x56, 0xE8,
    0x8C, 0xD0, 0xFF, 0xFF, 0xD9, 0x07, 0xD9, 0x5E, 0x68, 0x83, 0xC4, 0x04, 0xD9, 0x47, 0x04, 0x8B,
    0xCE, 0xD9, 0x5E, 0x6C, 0xD9, 0x47, 0x08, 0xD9, 0x5E, 0x70, 0xD9, 0x03, 0xD9, 0x5E, 0x5C, 0xD9,
    0x43, 0x04, 0xD9, 0x5E, 0x60, 0xD9, 0x43, 0x08, 0x66, 0x83, 0x8E, 0x98, 0x01, 0x00, 0x00, 0x01,
    0x66, 0x81, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x12, 0x08, 0xD9, 0x5E, 0x64, 0xC7, 0x86, 0x90, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x25, 0xD4, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x08, 0x66,
    0x83, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x20, 0x5E, 0xB0, 0x01, 0x5B, 0x59, 0xC3,
];
const SET_3D_POS_BODY_CALLS: [usize; 2] = [0x3f, 0x86];
const PLAY_PREPARED_BODY: [u8; 126] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x77, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x70, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x5F, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x8D, 0x88, 0xF8, 0xB8, 0xEE, 0x00, 0x74, 0x50, 0x66, 0xF7, 0x81,
    0x98, 0x01, 0x00, 0x00, 0x00, 0x04, 0x75, 0x45, 0xE8, 0x43, 0xDF, 0xFF, 0xFF, 0x84, 0xC0, 0x74,
    0x18, 0x0F, 0xB7, 0x81, 0x96, 0x01, 0x00, 0x00, 0xA9, 0x00, 0x02, 0x00, 0x00, 0x75, 0x0A, 0x83,
    0xC8, 0x20, 0x66, 0x89, 0x81, 0x96, 0x01, 0x00, 0x00, 0x66, 0x8B, 0x81, 0x96, 0x01, 0x00, 0x00,
    0x66, 0xC1, 0xE8, 0x09, 0x66, 0xF7, 0xD0, 0x66, 0xC1, 0xE0, 0x09, 0x66, 0x33, 0x81, 0x98, 0x01,
    0x00, 0x00, 0x66, 0x25, 0x00, 0x02, 0x66, 0x31, 0x81, 0x98, 0x01, 0x00, 0x00, 0xC3,
];
const PLAY_PREPARED_BODY_CALLS: [usize; 1] = [0x38];
const AMMO_ENTRY_ADD_BODY: [u8; 90] = [
    0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01, 0x53, 0x8A, 0x5C, 0x24, 0x08, 0x57, 0x8B,
    0xF8, 0x8B, 0x06, 0x75, 0x0F, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x75, 0x05, 0x8B,
    0x50, 0x10, 0xEB, 0x03, 0x8B, 0x50, 0x0C, 0x8B, 0x46, 0x04, 0x8B, 0x4E, 0x08, 0x2B, 0xC8, 0x03,
    0xCA, 0x84, 0xDB, 0x74, 0x04, 0x8B, 0xD7, 0xEB, 0x06, 0x3B, 0xF9, 0x8B, 0xD1, 0x7C, 0x02, 0x8B,
    0xFA, 0x03, 0xC7, 0x84, 0xDB, 0x89, 0x46, 0x04, 0x8B, 0xC7, 0x74, 0x09, 0x3B, 0xCA, 0x7D, 0x05,
    0x2B, 0xD1, 0x89, 0x56, 0x08, 0x5F, 0x5B, 0xC2, 0x04, 0x00,
];
const AMMO_ENTRY_ADD_BODY_CALLS: [usize; 0] = [];
const AMMO_ADD_TYPE_BODY: [u8; 78] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x33, 0xC0, 0x85, 0xD2, 0x56, 0x7E, 0x26, 0x8B, 0x31,
    0x8B, 0xCE, 0x8B, 0x19, 0x39, 0x7B, 0x04, 0x74, 0x11, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x0C, 0x3B,
    0xC2, 0x7C, 0xEF, 0x5E, 0x33, 0xC0, 0x5B, 0xC2, 0x08, 0x00, 0x8D, 0x14, 0x40, 0x8D, 0x34, 0x96,
    0x85, 0xF6, 0x75, 0x07, 0x5E, 0x33, 0xC0, 0x5B, 0xC2, 0x08, 0x00, 0x8B, 0x44, 0x24, 0x10, 0x50,
    0x8B, 0x44, 0x24, 0x10, 0xE8, 0x17, 0xFB, 0xFF, 0xFF, 0x5E, 0x5B, 0xC2, 0x08, 0x00,
];
const AMMO_ADD_TYPE_BODY_CALLS: [usize; 1] = [0x44];
const AMMO_REMOVE_BODY: [u8; 108] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x33, 0xC0, 0x85, 0xD2,
    0x56, 0x7E, 0x27, 0x8B, 0x31, 0x8B, 0xCE, 0x8B, 0x29, 0x39, 0x7D, 0x04, 0x74, 0x12, 0x83, 0xC0,
    0x01, 0x83, 0xC1, 0x0C, 0x3B, 0xC2, 0x7C, 0xEF, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC2, 0x04, 0x00,
    0x8D, 0x14, 0x40, 0x8D, 0x0C, 0x96, 0x85, 0xC9, 0x75, 0x08, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC2,
    0x04, 0x00, 0x8B, 0x51, 0x04, 0x3B, 0xDA, 0x8B, 0xC3, 0x7C, 0x02, 0x8B, 0xC2, 0x2B, 0xD0, 0x89,
    0x51, 0x04, 0x8B, 0x51, 0x08, 0x85, 0xD2, 0x76, 0x0D, 0x33, 0xF6, 0x2B, 0xD0, 0x3B, 0xF2, 0x1B,
    0xF6, 0x23, 0xF2, 0x89, 0x71, 0x08, 0x5E, 0x5D, 0x5B, 0xC2, 0x04, 0x00,
];
const AMMO_REMOVE_BODY_CALLS: [usize; 0] = [];
const WEAPON_AMMO_REMOVE_BODY: [u8; 46] = [
    0x85, 0xC9, 0x74, 0x27, 0x8B, 0x49, 0x08, 0x85, 0xC9, 0x75, 0x05, 0x33, 0xC0, 0xC2, 0x04, 0x00,
    0x8B, 0x89, 0xD4, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0xF1, 0x8B, 0x49, 0x04, 0x57, 0x8B, 0xF9,
    0x8B, 0x4C, 0x24, 0x08, 0x50, 0xE8, 0x36, 0xFF, 0xFF, 0xFF, 0x5F, 0xC2, 0x04, 0x00,
];
const WEAPON_AMMO_REMOVE_BODY_CALLS: [usize; 1] = [0x25];
const AMMO_CLEAR_BODY: [u8; 42] = [
    0x56, 0x33, 0xF6, 0x33, 0xC9, 0x39, 0x35, 0x50, 0x7B, 0x2D, 0x02, 0x7E, 0x1B, 0x33, 0xD2, 0x90,
    0x8B, 0x07, 0x03, 0xC2, 0x83, 0xC1, 0x01, 0x89, 0x70, 0x04, 0x89, 0x70, 0x08, 0x83, 0xC2, 0x0C,
    0x3B, 0x0D, 0x50, 0x7B, 0x2D, 0x02, 0x7C, 0xE8, 0x5E, 0xC3,
];
const AMMO_CLEAR_BODY_CALLS: [usize; 0] = [];
const IS_THROWN_BODY: [u8; 19] = [
    0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x33, 0xC9, 0x83, 0x78, 0x34, 0x05, 0x0F, 0x94, 0xC1,
    0x8A, 0xC1, 0xC3,
];
const IS_THROWN_BODY_CALLS: [usize; 0] = [];
const INST_ALLOC_BODY: [u8; 436] = [
    0xA1, 0xC4, 0x7B, 0x2D, 0x02, 0x53, 0x33, 0xDB, 0x3B, 0xC3, 0x75, 0x04, 0x33, 0xC0, 0x5B, 0xC3,
    0x8B, 0x48, 0x04, 0x3B, 0xC8, 0x8D, 0x50, 0x04, 0x57, 0x75, 0x08, 0x89, 0x1D, 0xC4, 0x7B, 0x2D,
    0x02, 0xEB, 0x1B, 0x89, 0x0D, 0xC4, 0x7B, 0x2D, 0x02, 0x8B, 0x08, 0x3B, 0xCB, 0x74, 0x05, 0x8B,
    0x3A, 0x89, 0x79, 0x04, 0x8B, 0x0A, 0x3B, 0xCB, 0x74, 0x04, 0x8B, 0x38, 0x89, 0x39, 0x8B, 0x3D,
    0x6C, 0x7B, 0x2D, 0x02, 0x3B, 0xFB, 0x89, 0x18, 0x89, 0x1A, 0x75, 0x0B, 0x89, 0x02, 0x89, 0x00,
    0xA3, 0x6C, 0x7B, 0x2D, 0x02, 0xEB, 0x0D, 0x8B, 0x0F, 0x89, 0x08, 0x89, 0x3A, 0x8B, 0x17, 0x89,
    0x42, 0x04, 0x89, 0x07, 0x89, 0x70, 0x08, 0x66, 0x89, 0x58, 0x0C, 0x8B, 0x4E, 0x10, 0xC1, 0xE9,
    0x03, 0xC0, 0xE1, 0x04, 0x32, 0xCB, 0x80, 0xE1, 0x10, 0x30, 0x48, 0x0C, 0xF6, 0x46, 0x10, 0x20,
    0x8A, 0x48, 0x0C, 0x74, 0x0B, 0x8B, 0x56, 0x10, 0xC1, 0xEA, 0x04, 0x83, 0xE2, 0x01, 0xEB, 0x02,
    0x33, 0xD2, 0xC0, 0xE2, 0x05, 0x32, 0xD1, 0x80, 0xE2, 0x20, 0x32, 0xD1, 0x88, 0x50, 0x0C, 0x0F,
    0xB7, 0x96, 0xDC, 0x00, 0x00, 0x00, 0x89, 0x50, 0x10, 0x0F, 0xB7, 0x8E, 0xDC, 0x00, 0x00, 0x00,
    0x89, 0x48, 0x14, 0xF6, 0x46, 0x10, 0x20, 0x74, 0x09, 0x0F, 0xB7, 0x8E, 0xDC, 0x00, 0x00, 0x00,
    0xEB, 0x02, 0x33, 0xC9, 0x89, 0x48, 0x18, 0xF6, 0x46, 0x10, 0x20, 0x74, 0x09, 0x0F, 0xB7, 0x8E,
    0xDC, 0x00, 0x00, 0x00, 0xEB, 0x02, 0x33, 0xC9, 0xD9, 0xEE, 0x83, 0xCA, 0xFF, 0xD9, 0x50, 0x40,
    0x89, 0x48, 0x1C, 0xD9, 0x50, 0x44, 0xBF, 0xFF, 0x00, 0x00, 0x00, 0xD9, 0x58, 0x48, 0x33, 0xC9,
    0xD9, 0x05, 0x3C, 0xC5, 0xE5, 0x00, 0x89, 0x50, 0x24, 0xD9, 0x58, 0x4C, 0x89, 0x50, 0x28, 0x89,
    0x50, 0x2C, 0x89, 0x50, 0x30, 0x89, 0x90, 0xA8, 0x00, 0x00, 0x00, 0x89, 0x90, 0xAC, 0x00, 0x00,
    0x00, 0x88, 0x58, 0x3C, 0x89, 0x58, 0x50, 0x89, 0x50, 0x34, 0x89, 0x78, 0x54, 0x89, 0x78, 0x60,
    0x89, 0x78, 0x6C, 0x89, 0x78, 0x78, 0x89, 0xB8, 0x84, 0x00, 0x00, 0x00, 0x89, 0xB8, 0x90, 0x00,
    0x00, 0x00, 0x89, 0xB8, 0x9C, 0x00, 0x00, 0x00, 0x89, 0x98, 0xB4, 0x00, 0x00, 0x00, 0x89, 0x98,
    0xB8, 0x00, 0x00, 0x00, 0x89, 0x98, 0xD0, 0x00, 0x00, 0x00, 0x89, 0x98, 0xCC, 0x00, 0x00, 0x00,
    0x89, 0x88, 0xBC, 0x00, 0x00, 0x00, 0x89, 0x88, 0xC0, 0x00, 0x00, 0x00, 0x89, 0x88, 0xC4, 0x00,
    0x00, 0x00, 0x89, 0x88, 0xC8, 0x00, 0x00, 0x00, 0xB9, 0x04, 0x00, 0x00, 0x00, 0x39, 0x4E, 0x34,
    0x75, 0x1A, 0x8B, 0x3D, 0xAC, 0x43, 0xE8, 0x00, 0x81, 0xFF, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x78,
    0x30, 0x7E, 0x09, 0x81, 0xC7, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x78, 0x30, 0x83, 0x7E, 0x34, 0x05,
    0x5F, 0x75, 0x03, 0x08, 0x48, 0x0D, 0x80, 0x48, 0x0D, 0x20, 0x89, 0x98, 0xE8, 0x00, 0x00, 0x00,
    0x89, 0x50, 0x38, 0x66, 0xC7, 0x80, 0xE4, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x89, 0x90, 0xEC, 0x00,
    0x00, 0x00, 0x5B, 0xC3,
];
const INST_ALLOC_BODY_CALLS: [usize; 0] = [];
const SLOT_ALLOC_R3_BODY: [u8; 204] = [
    0x83, 0xEC, 0x08, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x68, 0x60, 0x53, 0x5F, 0x02, 0xFF, 0x15, 0xCC,
    0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x76, 0xFF, 0xFF, 0xFF, 0x3C, 0xFF, 0x74, 0x15, 0x68, 0x60,
    0x53, 0x5F, 0x02, 0x88, 0x07, 0x88, 0x45, 0x00, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5D, 0x83,
    0xC4, 0x08, 0xC3, 0x85, 0xF6, 0x53, 0x7C, 0x7C, 0x83, 0xFE, 0x02, 0x7E, 0x6C, 0x83, 0xFE, 0x03,
    0x75, 0x72, 0xD9, 0x44, 0x24, 0x18, 0x51, 0xD9, 0x1C, 0x24, 0x56, 0xE8, 0xA0, 0xFC, 0xFF, 0xFF,
    0x83, 0xC4, 0x08, 0x8A, 0xD8, 0x80, 0xFB, 0xFF, 0x88, 0x5C, 0x24, 0x0C, 0x74, 0x56, 0xB8, 0x04,
    0x00, 0x00, 0x00, 0xE8, 0x28, 0xFF, 0xFF, 0xFF, 0x3C, 0xFF, 0x88, 0x44, 0x24, 0x08, 0x74, 0x44,
    0x0F, 0xB6, 0xC3, 0xBA, 0xFA, 0x00, 0x00, 0x00, 0xE8, 0x13, 0xF9, 0xFF, 0xFF, 0x8B, 0x44, 0x24,
    0x0C, 0x8B, 0x4C, 0x24, 0x08, 0x50, 0x51, 0xE8, 0x34, 0xF4, 0xFF, 0xFF, 0x8A, 0x54, 0x24, 0x10,
    0x83, 0xC4, 0x08, 0x68, 0x60, 0x53, 0x5F, 0x02, 0x88, 0x5D, 0x00, 0x88, 0x17, 0xFF, 0x15, 0xD0,
    0x80, 0xDB, 0x00, 0x5B, 0x5D, 0x83, 0xC4, 0x08, 0xC3, 0x56, 0xE8, 0x61, 0xF9, 0xFF, 0xFF, 0x83,
    0xC4, 0x04, 0xEB, 0x9F, 0x68, 0x60, 0x53, 0x5F, 0x02, 0xC6, 0x07, 0xFF, 0xC6, 0x45, 0x00, 0xFF,
    0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5B, 0x5D, 0x83, 0xC4, 0x08, 0xC3,
];
const SLOT_ALLOC_R3_BODY_CALLS: [usize; 6] = [0x15, 0x4b, 0x63, 0x78, 0x87, 0xaa];
const START_2D_R3_BODY: [u8; 373] = [
    0x51, 0xD9, 0xE8, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x56, 0x57, 0x51, 0x8D, 0x44, 0x24, 0x12, 0xD9,
    0x1C, 0x24, 0x50, 0xBE, 0x01, 0x00, 0x00, 0x00, 0x8D, 0x7C, 0x24, 0x17, 0xC6, 0x44, 0x24, 0x16,
    0xFF, 0xC6, 0x44, 0x24, 0x17, 0xFF, 0xE8, 0xE5, 0xD1, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x80, 0x7C,
    0x24, 0x0E, 0xFF, 0x0F, 0x84, 0x25, 0x01, 0x00, 0x00, 0x8A, 0x44, 0x24, 0x0F, 0x3C, 0xFF, 0x0F,
    0x84, 0x19, 0x01, 0x00, 0x00, 0xD9, 0xE8, 0x8B, 0x4C, 0x24, 0x18, 0x8B, 0x54, 0x24, 0x1C, 0x0F,
    0xB6, 0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x05, 0xF8, 0xB8, 0xEE, 0x00, 0x89, 0x48, 0x04,
    0x8B, 0x0C, 0x95, 0x78, 0x91, 0xEE, 0x00, 0xD9, 0x50, 0x24, 0x8D, 0x51, 0x08, 0xD9, 0x90, 0xAC,
    0x00, 0x00, 0x00, 0x33, 0xFF, 0xD9, 0xEE, 0x89, 0x90, 0x88, 0x00, 0x00, 0x00, 0xD9, 0x50, 0x5C,
    0x89, 0x90, 0x8C, 0x00, 0x00, 0x00, 0x66, 0x8B, 0x90, 0x94, 0x01, 0x00, 0x00, 0xD9, 0x50, 0x60,
    0xD9, 0x58, 0x64, 0x89, 0x78, 0x08, 0x66, 0x81, 0xE2, 0x02, 0x03, 0xD9, 0x90, 0xA0, 0x00, 0x00,
    0x00, 0xD9, 0x41, 0x28, 0xD9, 0x98, 0x9C, 0x00, 0x00, 0x00, 0xD9, 0x01, 0x66, 0x0F, 0xB6, 0x4C,
    0x24, 0x24, 0x66, 0xC1, 0xE1, 0x0E, 0xD9, 0x98, 0x80, 0x00, 0x00, 0x00, 0x66, 0x0B, 0xCA, 0x66,
    0x81, 0xC9, 0x40, 0x24, 0xD9, 0x50, 0x3C, 0xD9, 0x44, 0x24, 0x20, 0x66, 0x89, 0x88, 0x94, 0x01,
    0x00, 0x00, 0x0F, 0xB7, 0x88, 0x96, 0x01, 0x00, 0x00, 0xD9, 0x58, 0x18, 0xD9, 0x58, 0x1C, 0x66,
    0x81, 0xE1, 0xFF, 0xF5, 0x66, 0x81, 0xC9, 0x01, 0x80, 0x66, 0x89, 0x88, 0x96, 0x01, 0x00, 0x00,
    0x8B, 0x0D, 0x00, 0x9C, 0xE9, 0x00, 0x89, 0x88, 0xB4, 0x00, 0x00, 0x00, 0x03, 0xCE, 0x81, 0xF9,
    0x00, 0x24, 0xF4, 0x00, 0x89, 0xB0, 0xA4, 0x00, 0x00, 0x00, 0x89, 0xB8, 0xB8, 0x00, 0x00, 0x00,
    0x89, 0xB8, 0xBC, 0x00, 0x00, 0x00, 0x89, 0xB8, 0xC0, 0x00, 0x00, 0x00, 0x89, 0xB8, 0xB0, 0x00,
    0x00, 0x00, 0x89, 0x0D, 0x00, 0x9C, 0xE9, 0x00, 0x7E, 0x06, 0x89, 0x35, 0x00, 0x9C, 0xE9, 0x00,
    0x8B, 0x90, 0xB4, 0x00, 0x00, 0x00, 0x66, 0x81, 0x88, 0x98, 0x01, 0x00, 0x00, 0x00, 0x01, 0x89,
    0x38, 0x0F, 0xB6, 0x44, 0x24, 0x0E, 0xC1, 0xE2, 0x08, 0x0B, 0xD0, 0x83, 0xC8, 0xFF, 0x5F, 0x89,
    0x45, 0x08, 0x89, 0x45, 0x04, 0x5E, 0x89, 0x55, 0x00, 0x8B, 0xC5, 0x5D, 0x59, 0xC3, 0x83, 0xC8,
    0xFF, 0x5F, 0x89, 0x45, 0x08, 0x89, 0x45, 0x04, 0x5E, 0xC7, 0x45, 0x00, 0xFF, 0x00, 0x00, 0x00,
    0x8B, 0xC5, 0x5D, 0x59, 0xC3,
];
const START_2D_R3_BODY_CALLS: [usize; 1] = [0x26];
const VOICE_START_R3_BODY: [u8; 846] = [
    0x83, 0xEC, 0x08, 0x53, 0x8A, 0x5C, 0x24, 0x28, 0x55, 0x8B, 0x6C, 0x24, 0x18, 0xD9, 0x45, 0x08,
    0x56, 0x8B, 0x75, 0x18, 0x57, 0x51, 0x8D, 0x44, 0x24, 0x24, 0xD9, 0x1C, 0x24, 0x50, 0x8D, 0x7C,
    0x24, 0x1B, 0xC6, 0x44, 0x24, 0x28, 0xFF, 0xC6, 0x44, 0x24, 0x1B, 0xFF, 0xE8, 0xCF, 0xD8, 0xFF,
    0xFF, 0x83, 0xC4, 0x08, 0x80, 0x7C, 0x24, 0x20, 0xFF, 0x0F, 0x84, 0xF4, 0x02, 0x00, 0x00, 0x8A,
    0x44, 0x24, 0x13, 0x3C, 0xFF, 0x0F, 0x84, 0xE8, 0x02, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x28, 0x0F,
    0xB6, 0xC8, 0x8B, 0x44, 0x24, 0x2C, 0x8B, 0x04, 0x85, 0x78, 0x91, 0xEE, 0x00, 0x8B, 0xF1, 0x69,
    0xF6, 0x9C, 0x01, 0x00, 0x00, 0x81, 0xC6, 0xF8, 0xB8, 0xEE, 0x00, 0x89, 0x56, 0x04, 0xC7, 0x46,
    0x08, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x50, 0x08, 0xD9, 0x45, 0x0C, 0xD9, 0x5E, 0x24, 0xD9, 0x45,
    0x08, 0xD9, 0x9E, 0xAC, 0x00, 0x00, 0x00, 0xD9, 0x45, 0x1C, 0xD9, 0x5E, 0x5C, 0xD9, 0x45, 0x20,
    0xD9, 0x5E, 0x60, 0xD9, 0x45, 0x24, 0x89, 0x96, 0x88, 0x00, 0x00, 0x00, 0xD9, 0x5E, 0x64, 0x89,
    0x96, 0x8C, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x38, 0xD9, 0x9E, 0xA0, 0x00, 0x00, 0x00, 0xD9,
    0x40, 0x28, 0xD9, 0x9E, 0x9C, 0x00, 0x00, 0x00, 0xD9, 0x00, 0xD9, 0x9E, 0x80, 0x00, 0x00, 0x00,
    0x8B, 0x55, 0x18, 0x89, 0x96, 0xA4, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x4C, 0x89, 0x86, 0xB8, 0x00,
    0x00, 0x00, 0x8B, 0x55, 0x54, 0x89, 0x96, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x50, 0x89, 0x86,
    0xBC, 0x00, 0x00, 0x00, 0xD9, 0x45, 0x14, 0xD9, 0x5E, 0x30, 0xD9, 0x45, 0x10, 0xD9, 0x5E, 0x3C,
    0x8B, 0x55, 0x2C, 0xD9, 0x44, 0x24, 0x30, 0x89, 0x96, 0xB0, 0x00, 0x00, 0x00, 0xD9, 0x5E, 0x18,
    0xD9, 0xE8, 0xD9, 0x5E, 0x1C, 0x8B, 0x45, 0x04, 0x83, 0xF8, 0x20, 0x76, 0x0F, 0xD9, 0xEE, 0x51,
    0xD9, 0x1C, 0x24, 0x50, 0xE8, 0x07, 0xD9, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x8B, 0x45, 0x34, 0x89,
    0x86, 0xD0, 0x00, 0x00, 0x00, 0x8B, 0x4D, 0x38, 0x89, 0x8E, 0xD4, 0x00, 0x00, 0x00, 0x8B, 0x55,
    0x3C, 0x89, 0x96, 0xD8, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x40, 0x89, 0x86, 0xDC, 0x00, 0x00, 0x00,
    0x8B, 0x4D, 0x44, 0x89, 0x8E, 0xE0, 0x00, 0x00, 0x00, 0x8B, 0x55, 0x48, 0x66, 0x81, 0x8E, 0x96,
    0x01, 0x00, 0x00, 0x00, 0x80, 0x0F, 0xB7, 0x86, 0x96, 0x01, 0x00, 0x00, 0x89, 0x96, 0xE4, 0x00,
    0x00, 0x00, 0x0F, 0xB7, 0x4D, 0x58, 0x66, 0xC1, 0xE1, 0x0B, 0x66, 0x33, 0xC8, 0x66, 0x81, 0xE1,
    0x00, 0x08, 0x66, 0x33, 0xC8, 0x66, 0x89, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x55, 0x58,
    0xD0, 0xEA, 0x32, 0x96, 0x94, 0x01, 0x00, 0x00, 0x80, 0xE3, 0x01, 0x66, 0x0F, 0xB6, 0xC3, 0x66,
    0x03, 0xC0, 0x66, 0x03, 0xC0, 0x83, 0xE2, 0x01, 0x66, 0x31, 0x96, 0x94, 0x01, 0x00, 0x00, 0x0F,
    0xB6, 0x55, 0x58, 0x0F, 0xB7, 0x8E, 0x94, 0x01, 0x00, 0x00, 0xD0, 0xEA, 0x81, 0xE1, 0xF9, 0xFF,
    0x00, 0x00, 0x83, 0xE2, 0x02, 0x66, 0x0B, 0xC2, 0x66, 0x0B, 0xC1, 0x66, 0x89, 0x86, 0x94, 0x01,
    0x00, 0x00, 0x0F, 0xB6, 0x4D, 0x58, 0x32, 0xC8, 0x83, 0xE1, 0x10, 0x33, 0xC8, 0x66, 0x89, 0x8E,
    0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x45, 0x58, 0x32, 0xC1, 0x83, 0xE0, 0x08, 0x33, 0xC1, 0x66,
    0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x4D, 0x58, 0x32, 0xC8, 0x83, 0xE1, 0x20, 0x33,
    0xC8, 0x66, 0x89, 0x8E, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x55, 0x58, 0xD0, 0xEA, 0x32, 0xD1,
    0x83, 0xE2, 0x40, 0x33, 0xD1, 0x66, 0x89, 0x96, 0x94, 0x01, 0x00, 0x00, 0x66, 0x8B, 0x45, 0x58,
    0x66, 0x03, 0xC0, 0x66, 0x03, 0xC0, 0x66, 0x33, 0xC2, 0x66, 0x25, 0x00, 0x04, 0x66, 0x33, 0xC2,
    0x66, 0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x66, 0x8B, 0x4D, 0x58, 0x66, 0x03, 0xC9, 0x66, 0x03,
    0xC9, 0x66, 0x33, 0xC8, 0x66, 0x81, 0xE1, 0x00, 0x08, 0x66, 0x33, 0xC8, 0x66, 0x89, 0x8E, 0x94,
    0x01, 0x00, 0x00, 0x0F, 0xB7, 0x45, 0x58, 0x66, 0x03, 0xC0, 0x66, 0x03, 0xC0, 0x66, 0x33, 0xC1,
    0x66, 0x25, 0x00, 0x10, 0x66, 0x33, 0xC1, 0x66, 0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB7,
    0x86, 0x96, 0x01, 0x00, 0x00, 0xA9, 0x00, 0x08, 0x00, 0x00, 0x75, 0x0A, 0x83, 0xC8, 0x01, 0x66,
    0x89, 0x86, 0x96, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0x45, 0x02, 0x66, 0x3D, 0x0A, 0x00, 0x76, 0x1A,
    0x66, 0x81, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x00, 0x02, 0x0F, 0xB7, 0xC8, 0x89, 0x4C, 0x24, 0x14,
    0xDB, 0x44, 0x24, 0x14, 0xD9, 0x9E, 0xC8, 0x00, 0x00, 0x00, 0x8B, 0xCE, 0xE8, 0x2F, 0xE0, 0xFF,
    0xFF, 0x84, 0xC0, 0x74, 0x18, 0x0F, 0xB7, 0x86, 0x96, 0x01, 0x00, 0x00, 0xA9, 0x00, 0x02, 0x00,
    0x00, 0x75, 0x0A, 0x83, 0xC8, 0x20, 0x66, 0x89, 0x86, 0x96, 0x01, 0x00, 0x00, 0xA1, 0x00, 0x9C,
    0xE9, 0x00, 0x89, 0x86, 0xB4, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3D, 0x00, 0x24, 0xF4, 0x00,
    0xA3, 0x00, 0x9C, 0xE9, 0x00, 0x7E, 0x0A, 0xC7, 0x05, 0x00, 0x9C, 0xE9, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x66, 0x8B, 0x96, 0x96, 0x01, 0x00, 0x00, 0x8B, 0x8E, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0x44,
    0x24, 0x24, 0x66, 0xC1, 0xEA, 0x09, 0x66, 0xF7, 0xD2, 0x66, 0xC1, 0xE2, 0x09, 0x66, 0x33, 0x96,
    0x98, 0x01, 0x00, 0x00, 0xC1, 0xE1, 0x08, 0x66, 0x81, 0xE2, 0x00, 0x02, 0x66, 0x31, 0x96, 0x98,
    0x01, 0x00, 0x00, 0x0F, 0xB6, 0x54, 0x24, 0x20, 0x5F, 0x89, 0x06, 0x8B, 0x44, 0x24, 0x18, 0x0B,
    0xCA, 0x5E, 0x89, 0x08, 0x83, 0xC9, 0xFF, 0x5D, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x5B, 0x83,
    0xC4, 0x08, 0xC3, 0x8B, 0x44, 0x24, 0x1C, 0x5F, 0x5E, 0x83, 0xC9, 0xFF, 0x5D, 0xC7, 0x00, 0xFF,
    0x00, 0x00, 0x00, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x5B, 0x83, 0xC4, 0x08, 0xC3,
];
const VOICE_START_R3_BODY_CALLS: [usize; 3] = [0x2c, 0x114, 0x29c];
const VOICE_PREPARE_R3_BODY: [u8; 798] = [
    0x83, 0xEC, 0x08, 0x53, 0x8A, 0x5C, 0x24, 0x28, 0x55, 0x8B, 0x6C, 0x24, 0x18, 0xD9, 0x45, 0x08,
    0x56, 0x8B, 0x75, 0x18, 0x57, 0x51, 0x8D, 0x44, 0x24, 0x24, 0xD9, 0x1C, 0x24, 0x50, 0x8D, 0x7C,
    0x24, 0x1B, 0xC6, 0x44, 0x24, 0x28, 0xFF, 0xC6, 0x44, 0x24, 0x1B, 0xFF, 0xE8, 0xFF, 0xD4, 0xFF,
    0xFF, 0x83, 0xC4, 0x08, 0x80, 0x7C, 0x24, 0x20, 0xFF, 0x0F, 0x84, 0xC4, 0x02, 0x00, 0x00, 0x8A,
    0x44, 0x24, 0x13, 0x3C, 0xFF, 0x0F, 0x84, 0xB8, 0x02, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x28, 0x0F,
    0xB6, 0xC8, 0x8B, 0x44, 0x24, 0x2C, 0x8B, 0x04, 0x85, 0x78, 0x91, 0xEE, 0x00, 0x8B, 0xF1, 0x69,
    0xF6, 0x9C, 0x01, 0x00, 0x00, 0x81, 0xC6, 0xF8, 0xB8, 0xEE, 0x00, 0x89, 0x56, 0x04, 0xC7, 0x46,
    0x08, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x50, 0x08, 0xD9, 0x45, 0x0C, 0xD9, 0x5E, 0x24, 0xD9, 0x45,
    0x08, 0xD9, 0x9E, 0xAC, 0x00, 0x00, 0x00, 0xD9, 0x45, 0x1C, 0xD9, 0x5E, 0x5C, 0xD9, 0x45, 0x20,
    0xD9, 0x5E, 0x60, 0xD9, 0x45, 0x24, 0x89, 0x96, 0x88, 0x00, 0x00, 0x00, 0xD9, 0x5E, 0x64, 0x89,
    0x96, 0x8C, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x38, 0xD9, 0x9E, 0xA0, 0x00, 0x00, 0x00, 0xD9,
    0x40, 0x28, 0xD9, 0x9E, 0x9C, 0x00, 0x00, 0x00, 0xD9, 0x00, 0xD9, 0x9E, 0x80, 0x00, 0x00, 0x00,
    0x8B, 0x55, 0x18, 0x89, 0x96, 0xA4, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x4C, 0x89, 0x86, 0xB8, 0x00,
    0x00, 0x00, 0x8B, 0x55, 0x54, 0x89, 0x96, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x50, 0x89, 0x86,
    0xBC, 0x00, 0x00, 0x00, 0xD9, 0x45, 0x14, 0xD9, 0x5E, 0x30, 0xD9, 0x45, 0x10, 0xD9, 0x5E, 0x3C,
    0x8B, 0x55, 0x2C, 0xD9, 0x44, 0x24, 0x30, 0x89, 0x96, 0xB0, 0x00, 0x00, 0x00, 0xD9, 0x5E, 0x18,
    0xD9, 0xE8, 0xD9, 0x5E, 0x1C, 0x8B, 0x45, 0x04, 0x83, 0xF8, 0x20, 0x76, 0x0F, 0xD9, 0xEE, 0x51,
    0xD9, 0x1C, 0x24, 0x50, 0xE8, 0x37, 0xD5, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x8B, 0x45, 0x34, 0x89,
    0x86, 0xD0, 0x00, 0x00, 0x00, 0x8B, 0x4D, 0x38, 0x89, 0x8E, 0xD4, 0x00, 0x00, 0x00, 0x8B, 0x55,
    0x3C, 0x89, 0x96, 0xD8, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x40, 0x89, 0x86, 0xDC, 0x00, 0x00, 0x00,
    0x8B, 0x4D, 0x44, 0x89, 0x8E, 0xE0, 0x00, 0x00, 0x00, 0x8B, 0x55, 0x48, 0x66, 0x81, 0x8E, 0x96,
    0x01, 0x00, 0x00, 0x00, 0x80, 0x0F, 0xB7, 0x86, 0x96, 0x01, 0x00, 0x00, 0x89, 0x96, 0xE4, 0x00,
    0x00, 0x00, 0x0F, 0xB7, 0x4D, 0x58, 0x66, 0xC1, 0xE1, 0x0B, 0x66, 0x33, 0xC8, 0x66, 0x81, 0xE1,
    0x00, 0x08, 0x66, 0x33, 0xC8, 0x66, 0x89, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x55, 0x58,
    0xD0, 0xEA, 0x32, 0x96, 0x94, 0x01, 0x00, 0x00, 0x80, 0xE3, 0x01, 0x66, 0x0F, 0xB6, 0xC3, 0x66,
    0x03, 0xC0, 0x66, 0x03, 0xC0, 0x83, 0xE2, 0x01, 0x66, 0x31, 0x96, 0x94, 0x01, 0x00, 0x00, 0x0F,
    0xB6, 0x55, 0x58, 0x0F, 0xB7, 0x8E, 0x94, 0x01, 0x00, 0x00, 0x81, 0xE1, 0xF9, 0xFF, 0x00, 0x00,
    0xD0, 0xEA, 0x83, 0xE2, 0x02, 0x66, 0x0B, 0xC2, 0x66, 0x0B, 0xC1, 0x66, 0x89, 0x86, 0x94, 0x01,
    0x00, 0x00, 0x0F, 0xB6, 0x4D, 0x58, 0x32, 0xC8, 0x83, 0xE1, 0x10, 0x33, 0xC8, 0x66, 0x89, 0x8E,
    0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x45, 0x58, 0x32, 0xC1, 0x83, 0xE0, 0x08, 0x33, 0xC1, 0x66,
    0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x4D, 0x58, 0x32, 0xC8, 0x83, 0xE1, 0x20, 0x33,
    0xC8, 0x66, 0x89, 0x8E, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB6, 0x45, 0x58, 0x66, 0x81, 0xA6, 0x96,
    0x01, 0x00, 0x00, 0xDF, 0xFF, 0xD0, 0xE8, 0x32, 0xC1, 0x0F, 0xB7, 0x96, 0x96, 0x01, 0x00, 0x00,
    0x83, 0xE0, 0x40, 0x33, 0xC1, 0x66, 0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0x4D, 0x58,
    0x66, 0x03, 0xC9, 0x66, 0x03, 0xC9, 0x66, 0x33, 0xC8, 0x66, 0x81, 0xE1, 0x00, 0x04, 0x66, 0x31,
    0x8E, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0x8E, 0x94, 0x01, 0x00, 0x00, 0x66, 0x8B, 0x45, 0x58,
    0x66, 0x03, 0xC0, 0x66, 0x03, 0xC0, 0x66, 0x33, 0xC1, 0x66, 0x25, 0x00, 0x08, 0x66, 0x33, 0xC1,
    0x66, 0x89, 0x86, 0x94, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0x4D, 0x58, 0x66, 0x03, 0xC9, 0x66, 0x03,
    0xC9, 0x66, 0x33, 0xC8, 0x66, 0x81, 0xE1, 0x00, 0x10, 0x66, 0x33, 0xC8, 0x66, 0x89, 0x8E, 0x94,
    0x01, 0x00, 0x00, 0xF7, 0xC2, 0x00, 0x08, 0x00, 0x00, 0x75, 0x0A, 0x83, 0xCA, 0x01, 0x66, 0x89,
    0x96, 0x96, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0x45, 0x02, 0x66, 0x3D, 0x0A, 0x00, 0x76, 0x1A, 0x66,
    0x81, 0x8E, 0x96, 0x01, 0x00, 0x00, 0x00, 0x02, 0x0F, 0xB7, 0xD0, 0x89, 0x54, 0x24, 0x14, 0xDB,
    0x44, 0x24, 0x14, 0xD9, 0x9E, 0xC8, 0x00, 0x00, 0x00, 0xA1, 0x00, 0x9C, 0xE9, 0x00, 0x89, 0x86,
    0xB4, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3D, 0x00, 0x24, 0xF4, 0x00, 0xA3, 0x00, 0x9C, 0xE9,
    0x00, 0x7E, 0x0A, 0xC7, 0x05, 0x00, 0x9C, 0xE9, 0x00, 0x01, 0x00, 0x00, 0x00, 0x8B, 0x8E, 0xB4,
    0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x24, 0x0F, 0xB6, 0x54, 0x24, 0x20, 0x66, 0x81, 0x8E, 0x98,
    0x01, 0x00, 0x00, 0x00, 0x01, 0xC1, 0xE1, 0x08, 0x5F, 0x89, 0x06, 0x8B, 0x44, 0x24, 0x18, 0x0B,
    0xCA, 0x5E, 0x89, 0x08, 0x83, 0xC9, 0xFF, 0x5D, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x5B, 0x83,
    0xC4, 0x08, 0xC3, 0x8B, 0x44, 0x24, 0x1C, 0x5F, 0x5E, 0x83, 0xC9, 0xFF, 0x5D, 0xC7, 0x00, 0xFF,
    0x00, 0x00, 0x00, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x5B, 0x83, 0xC4, 0x08, 0xC3,
];
const VOICE_PREPARE_R3_BODY_CALLS: [usize; 2] = [0x2c, 0x114];
const FOLEY_CUE_R3_BODY: [u8; 333] = [
    0x83, 0xEC, 0x18, 0x53, 0x8B, 0x5C, 0x24, 0x20, 0x0F, 0xB7, 0x03, 0x55, 0x83, 0xCD, 0xFF, 0x66,
    0x3B, 0x05, 0xF0, 0x6A, 0x52, 0x02, 0x56, 0xBA, 0xFF, 0x00, 0x00, 0x00, 0x0F, 0x83, 0x14, 0x01,
    0x00, 0x00, 0x8B, 0x4B, 0x28, 0x0F, 0xB7, 0xC0, 0x8B, 0x34, 0x85, 0x58, 0x55, 0xEE, 0x00, 0x8B,
    0x46, 0x10, 0xC1, 0xE8, 0x0C, 0x83, 0xE0, 0x3F, 0x83, 0xF8, 0x01, 0x75, 0x04, 0x33, 0xC9, 0xEB,
    0x1B, 0x73, 0x14, 0x83, 0xC8, 0xFF, 0x89, 0x17, 0x89, 0x47, 0x04, 0x5E, 0x89, 0x6F, 0x08, 0x5D,
    0x8B, 0xC7, 0x5B, 0x83, 0xC4, 0x18, 0xC3, 0x83, 0xF9, 0xFF, 0x74, 0xE7, 0x8B, 0x46, 0x04, 0x83,
    0x3C, 0x88, 0xFF, 0x8D, 0x2C, 0x88, 0x0F, 0x84, 0xCA, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x43, 0x58,
    0xA8, 0x01, 0x74, 0x5F, 0xA8, 0x40, 0x75, 0x5B, 0xD9, 0x43, 0x1C, 0x8B, 0xC6, 0xD8, 0x25, 0x54,
    0xB2, 0xEE, 0x00, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x43, 0x20, 0xD8, 0x25, 0x58, 0xB2, 0xEE, 0x00,
    0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x43, 0x24, 0xD8, 0x25, 0x5C, 0xB2, 0xEE, 0x00, 0xD9, 0x5C, 0x24,
    0x14, 0xE8, 0x0A, 0xFB, 0xFF, 0xFF, 0xDC, 0xC8, 0x8D, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x28,
    0xD9, 0x44, 0x24, 0x28, 0xD9, 0x5C, 0x24, 0x28, 0xE8, 0x43, 0x4E, 0xF9, 0xFF, 0xD9, 0x44, 0x24,
    0x28, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x09, 0xB8, 0xFF, 0x00, 0x00, 0x00, 0x89,
    0x07, 0xEB, 0x65, 0x8B, 0x46, 0x10, 0xD9, 0x46, 0x0C, 0x8B, 0x55, 0x00, 0x51, 0xD9, 0x1C, 0x24,
    0x8B, 0xC8, 0xD9, 0x46, 0x08, 0xC1, 0xE9, 0x12, 0x81, 0xE1, 0x01, 0xFF, 0xFF, 0xFF, 0x51, 0x51,
    0xD9, 0x1C, 0x24, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0x80, 0x7C, 0x24, 0x38, 0x00, 0x50, 0x52, 0x6A,
    0x00, 0x53, 0x74, 0x0C, 0x8D, 0x44, 0x24, 0x28, 0x50, 0xE8, 0x12, 0x5C, 0x00, 0x00, 0xEB, 0x0A,
    0x8D, 0x44, 0x24, 0x34, 0x50, 0xE8, 0x36, 0x58, 0x00, 0x00, 0x8B, 0x08, 0x8B, 0x50, 0x04, 0x8B,
    0x40, 0x08, 0x83, 0xC4, 0x20, 0x5E, 0x89, 0x0F, 0x89, 0x57, 0x04, 0x5D, 0x89, 0x47, 0x08, 0x8B,
    0xC7, 0x5B, 0x83, 0xC4, 0x18, 0xC3, 0x89, 0x17, 0x83, 0xC8, 0xFF, 0x89, 0x47, 0x04, 0x5E, 0x83,
    0xC8, 0xFF, 0x5D, 0x89, 0x47, 0x08, 0x8B, 0xC7, 0x5B, 0x83, 0xC4, 0x18, 0xC3,
];
const FOLEY_CUE_R3_BODY_CALLS: [usize; 4] = [0xa1, 0xb8, 0x109, 0x115];
const VOICE_LINE_R3_BODY: [u8; 660] = [
    0x83, 0xEC, 0x28, 0x83, 0xC8, 0xFF, 0x53, 0x8B, 0x5C, 0x24, 0x30, 0xC7, 0x06, 0xFF, 0x00, 0x00,
    0x00, 0x89, 0x46, 0x08, 0x89, 0x46, 0x04, 0x0F, 0xB7, 0x03, 0x66, 0x3B, 0x05, 0xF8, 0x6A, 0x52,
    0x02, 0x0F, 0x83, 0x66, 0x02, 0x00, 0x00, 0x66, 0x3D, 0xFF, 0xFF, 0x0F, 0x84, 0x5C, 0x02, 0x00,
    0x00, 0x57, 0x0F, 0xB7, 0xF8, 0xC1, 0xE7, 0x04, 0x81, 0xC7, 0x60, 0x43, 0x57, 0x02, 0x83, 0x7F,
    0x04, 0x00, 0x0F, 0x84, 0x44, 0x02, 0x00, 0x00, 0x0F, 0xB7, 0x43, 0x30, 0x66, 0x3D, 0xFF, 0xFF,
    0x55, 0x75, 0x12, 0xE8, 0x08, 0xF6, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x41, 0x5D, 0x5F, 0x8B, 0xC6,
    0x5B, 0x83, 0xC4, 0x28, 0xC3, 0x8B, 0x4F, 0x08, 0x8B, 0xD1, 0xC1, 0xEA, 0x0C, 0x0F, 0xB7, 0xC0,
    0x8B, 0xEA, 0x83, 0xE5, 0x7F, 0x3B, 0xC5, 0x73, 0x11, 0xC1, 0xE0, 0x13, 0x33, 0xC1, 0x25, 0x00,
    0x00, 0xF8, 0x03, 0x33, 0xC1, 0x89, 0x47, 0x08, 0xEB, 0x13, 0x83, 0xC2, 0xFF, 0xC1, 0xE2, 0x13,
    0x33, 0xD1, 0x81, 0xE2, 0x00, 0x00, 0xF8, 0x03, 0x33, 0xD1, 0x89, 0x57, 0x08, 0x8B, 0x47, 0x08,
    0xC1, 0xE8, 0x13, 0x83, 0xE0, 0x7F, 0x66, 0x89, 0x43, 0x30, 0x8B, 0x47, 0x08, 0x8B, 0x57, 0x04,
    0xC1, 0xE8, 0x13, 0x83, 0xE0, 0x7F, 0x8D, 0x0C, 0x40, 0x8B, 0x2C, 0x8A, 0x83, 0xFD, 0xFF, 0x8D,
    0x04, 0x8A, 0x89, 0x44, 0x24, 0x14, 0x0F, 0x84, 0xBF, 0x01, 0x00, 0x00, 0x66, 0x8B, 0x43, 0x32,
    0xE8, 0x7B, 0x45, 0xFF, 0xFF, 0x8B, 0x0C, 0x85, 0x78, 0x91, 0xEE, 0x00, 0xD9, 0x41, 0x28, 0x0F,
    0xB7, 0x4B, 0x58, 0xF6, 0xC1, 0x01, 0xD9, 0x5C, 0x24, 0x38, 0x89, 0x44, 0x24, 0x10, 0x74, 0x62,
    0xF6, 0xC1, 0x40, 0x75, 0x5D, 0xD9, 0x43, 0x1C, 0x8D, 0x44, 0x24, 0x1C, 0xD8, 0x25, 0x54, 0xB2,
    0xEE, 0x00, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x43, 0x20, 0xD8, 0x25, 0x58, 0xB2, 0xEE, 0x00, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x43, 0x24, 0xD8, 0x25, 0x5C, 0xB2, 0xEE, 0x00, 0xD9, 0x5C, 0x24, 0x24,
    0xD9, 0x44, 0x24, 0x38, 0xDC, 0x0D, 0x00, 0xC4, 0xE5, 0x00, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x0C, 0xE8, 0x73, 0x38, 0xF9, 0xFF, 0xD9, 0x44, 0x24,
    0x0C, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F, 0x8B, 0x3D, 0x01, 0x00, 0x00, 0x8B, 0x44,
    0x24, 0x10, 0x80, 0x7C, 0x24, 0x3C, 0x00, 0xD9, 0xE8, 0x51, 0xD9, 0x14, 0x24, 0x6A, 0x00, 0x51,
    0xD9, 0x1C, 0x24, 0x50, 0x55, 0x6A, 0x00, 0x53, 0x74, 0x1C, 0x8D, 0x54, 0x24, 0x38, 0x52, 0xE8,
    0x5C, 0x46, 0x00, 0x00, 0x8B, 0x08, 0x89, 0x0E, 0x8B, 0x50, 0x04, 0x89, 0x56, 0x04, 0x8B, 0x40,
    0x08, 0x89, 0x46, 0x08, 0xEB, 0x1A, 0x8D, 0x4C, 0x24, 0x44, 0x51, 0xE8, 0x70, 0x42, 0x00, 0x00,
    0x8B, 0x10, 0x89, 0x16, 0x8B, 0x48, 0x04, 0x89, 0x4E, 0x04, 0x8B, 0x50, 0x08, 0x89, 0x56, 0x08,
    0x83, 0xC4, 0x20, 0x8B, 0xC6, 0xE8, 0x06, 0x06, 0x00, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0xD9, 0x00,
    0x00, 0x00, 0x8B, 0x44, 0x24, 0x14, 0x8A, 0x48, 0x04, 0x02, 0xC9, 0x02, 0xC9, 0x0F, 0xBE, 0xD1,
    0xC1, 0xFA, 0x02, 0x89, 0x56, 0x08, 0x8B, 0x47, 0x08, 0xC1, 0xE8, 0x1A, 0x83, 0xE0, 0x0F, 0x3B,
    0x05, 0xA0, 0x6A, 0x52, 0x02, 0x0F, 0x84, 0xB0, 0x00, 0x00, 0x00, 0xF6, 0x43, 0x58, 0x01, 0x0F,
    0x84, 0x83, 0x00, 0x00, 0x00, 0xD9, 0x05, 0x54, 0xB2, 0xEE, 0x00, 0x8D, 0x44, 0x24, 0x1C, 0xD8,
    0x63, 0x1C, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x05, 0x58, 0xB2, 0xEE, 0x00, 0xD8, 0x63, 0x20, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x05, 0x5C, 0xB2, 0xEE, 0x00, 0xD8, 0x63, 0x24, 0xD9, 0x5C, 0x24, 0x24,
    0xD9, 0x44, 0x24, 0x38, 0xDD, 0x5C, 0x24, 0x14, 0xE8, 0x93, 0x37, 0xF9, 0xFF, 0xDD, 0x44, 0x24,
    0x14, 0xDC, 0xC8, 0xDC, 0x0D, 0xD8, 0xC3, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x14, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x51, 0xB2, 0x01, 0x8B, 0xCE, 0xE8, 0xFD,
    0x50, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x38, 0x51, 0x8B, 0x4F, 0x08, 0xD9, 0x1C, 0x24, 0xC1, 0xE9,
    0x1A, 0x83, 0xE1, 0x0F, 0x51, 0x8B, 0xFE, 0xE8, 0x84, 0x38, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x5D,
    0x5F, 0x8B, 0xC6, 0x5B, 0x83, 0xC4, 0x28, 0xC3, 0xB2, 0x01, 0x8B, 0xCE, 0xE8, 0xCF, 0x50, 0x00,
    0x00, 0xD9, 0xEE, 0x8B, 0x57, 0x08, 0x51, 0xC1, 0xEA, 0x1A, 0xD9, 0x1C, 0x24, 0x83, 0xE2, 0x0F,
    0x52, 0x8B, 0xFE, 0xE8, 0x58, 0x38, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x5D, 0x5F, 0x8B, 0xC6, 0x5B,
    0x83, 0xC4, 0x28, 0xC3,
];
const VOICE_LINE_R3_BODY_CALLS: [usize; 11] = [0x53, 0xd0, 0x138, 0x16f, 0x18b, 0x1a5, 0x218, 0x23e, 0x257, 0x26c, 0x283];
const MUSIC_CUE_R3_BODY: [u8; 341] = [
    0x83, 0xC9, 0xFF, 0x83, 0xEC, 0x1C, 0x0B, 0xD1, 0x80, 0x3D, 0xDE, 0x6A, 0x52, 0x02, 0x00, 0x55,
    0x8B, 0x6C, 0x24, 0x24, 0xB8, 0xFF, 0x00, 0x00, 0x00, 0x74, 0x0F, 0x89, 0x06, 0x89, 0x56, 0x04,
    0x89, 0x4E, 0x08, 0x8B, 0xC6, 0x5D, 0x83, 0xC4, 0x1C, 0xC3, 0x0F, 0xB7, 0x55, 0x00, 0x66, 0x3B,
    0x15, 0xF4, 0x6A, 0x52, 0x02, 0x53, 0x57, 0x0F, 0x83, 0x04, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD2,
    0x8B, 0x3C, 0x95, 0x78, 0x71, 0xEE, 0x00, 0x8B, 0x5F, 0x04, 0x85, 0xDB, 0x0F, 0x84, 0xEF, 0x00,
    0x00, 0x00, 0x8B, 0x57, 0x10, 0xC1, 0xEA, 0x11, 0x83, 0xE2, 0x1F, 0x83, 0x3C, 0x93, 0xFF, 0x0F,
    0x84, 0xDC, 0x00, 0x00, 0x00, 0xF6, 0x45, 0x58, 0x01, 0x74, 0x6E, 0xD9, 0x45, 0x1C, 0x8B, 0xC7,
    0xD8, 0x25, 0x54, 0xB2, 0xEE, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x45, 0x20, 0xD8, 0x25, 0x58,
    0xB2, 0xEE, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x45, 0x24, 0xD8, 0x25, 0x5C, 0xB2, 0xEE, 0x00,
    0xD9, 0x5C, 0x24, 0x18, 0xE8, 0xD7, 0xF7, 0xFF, 0xFF, 0xDC, 0xC8, 0x8D, 0x44, 0x24, 0x10, 0xD9,
    0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x0C, 0xE8, 0x10, 0x45, 0xF9, 0xFF,
    0xD9, 0x44, 0x24, 0x0C, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x1C, 0xB8, 0xFF, 0x00,
    0x00, 0x00, 0x89, 0x06, 0x83, 0xC8, 0xFF, 0x89, 0x46, 0x04, 0x5F, 0x83, 0xC8, 0xFF, 0x5B, 0x89,
    0x46, 0x08, 0x8B, 0xC6, 0x5D, 0x83, 0xC4, 0x1C, 0xC3, 0xD9, 0xE8, 0x8B, 0x47, 0x10, 0x51, 0xD9,
    0x1C, 0x24, 0xD9, 0x47, 0x0C, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x8B, 0xD0, 0x8B, 0xC8, 0xC1,
    0xEA, 0x11, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0x51, 0x83, 0xE2, 0x1F, 0x8B, 0x0C, 0x93, 0xC1,
    0xE8, 0x18, 0x83, 0xE0, 0x03, 0x80, 0x7C, 0x24, 0x40, 0x00, 0x51, 0x50, 0x55, 0x74, 0x0C, 0x8D,
    0x54, 0x24, 0x2C, 0x52, 0xE8, 0xC7, 0x52, 0x00, 0x00, 0xEB, 0x0A, 0x8D, 0x54, 0x24, 0x38, 0x52,
    0xE8, 0xEB, 0x4E, 0x00, 0x00, 0x8B, 0x08, 0x8B, 0x50, 0x04, 0x8B, 0x40, 0x08, 0x83, 0xC4, 0x20,
    0x5F, 0x89, 0x0E, 0x89, 0x56, 0x04, 0x5B, 0x89, 0x46, 0x08, 0x8B, 0xC6, 0x5D, 0x83, 0xC4, 0x1C,
    0xC3, 0x89, 0x06, 0x5F, 0x83, 0xC8, 0xFF, 0x89, 0x46, 0x04, 0x5B, 0x89, 0x4E, 0x08, 0x8B, 0xC6,
    0x5D, 0x83, 0xC4, 0x1C, 0xC3,
];
const MUSIC_CUE_R3_BODY_CALLS: [usize; 4] = [0x94, 0xab, 0x114, 0x120];
const RELOAD_R3_BODY: [u8; 160] = [
    0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x85, 0xDB, 0x55, 0x8B, 0x6C, 0x24, 0x14, 0x0F, 0x84, 0x89, 0x00,
    0x00, 0x00, 0x8B, 0x43, 0x08, 0x8B, 0x88, 0xD4, 0x00, 0x00, 0x00, 0x57, 0x8B, 0x79, 0x04, 0x8B,
    0x4C, 0x24, 0x10, 0xE8, 0x58, 0xFA, 0xFF, 0xFF, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x6C, 0x56, 0x8B,
    0xC5, 0x8B, 0xF3, 0xE8, 0x28, 0xAA, 0x00, 0x00, 0x2B, 0x43, 0x10, 0x8B, 0x4F, 0x04, 0x3B, 0xC1,
    0x7C, 0x02, 0x8B, 0xC1, 0x2B, 0xC8, 0x89, 0x4F, 0x04, 0x8B, 0x4F, 0x08, 0x85, 0xC9, 0x76, 0x0D,
    0x33, 0xD2, 0x2B, 0xC8, 0x3B, 0xD1, 0x1B, 0xD2, 0x23, 0xD1, 0x89, 0x57, 0x08, 0x01, 0x43, 0x10,
    0x8B, 0x43, 0x08, 0xF6, 0x40, 0x10, 0x20, 0x74, 0x30, 0x8B, 0x80, 0xD8, 0x00, 0x00, 0x00, 0x85,
    0xC0, 0x74, 0x26, 0x8B, 0x78, 0x04, 0x8B, 0x4C, 0x24, 0x14, 0xE8, 0x01, 0xFA, 0xFF, 0xFF, 0x8B,
    0xF8, 0x85, 0xFF, 0x74, 0x14, 0x8B, 0xC5, 0xE8, 0xD4, 0xA9, 0x00, 0x00, 0x2B, 0x43, 0x18, 0x8B,
    0xF7, 0xE8, 0xFA, 0xF8, 0xFF, 0xFF, 0x01, 0x43, 0x18, 0x5E, 0x5F, 0x5D, 0x5B, 0xC2, 0x0C, 0x00,
];
const RELOAD_R3_BODY_CALLS: [usize; 5] = [0x23, 0x33, 0x7a, 0x87, 0x91];
const HANDLE_VALID_R3_BODY: [u8; 44] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x07, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x75, 0x03, 0x32, 0xC0, 0xC3,
    0x0F, 0xB6, 0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75,
    0xEC, 0x83, 0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x0F, 0x95, 0xC0, 0xC3,
];
const HANDLE_VALID_R3_BODY_CALLS: [usize; 0] = [];
const INFO_IS_MELEE_R3_BODY: [u8; 58] = [
    0x85, 0xC0, 0x74, 0x1C, 0x8B, 0x40, 0x34, 0x83, 0xC0, 0xFA, 0x83, 0xF8, 0x0D, 0x77, 0x11, 0x0F,
    0xB6, 0x80, 0x6C, 0x27, 0xB7, 0x00, 0xFF, 0x24, 0x85, 0x64, 0x27, 0xB7, 0x00, 0xB0, 0x01, 0xC3,
    0x32, 0xC0, 0xC3, 0x90, 0x5D, 0x27, 0xB7, 0x00, 0x60, 0x27, 0xB7, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00,
];
const INFO_IS_MELEE_R3_BODY_CALLS: [usize; 0] = [];
const CLASS_IS_MELEE_R3_BODY: [u8; 50] = [
    0x83, 0xC0, 0xFA, 0x83, 0xF8, 0x0D, 0x77, 0x11, 0x0F, 0xB6, 0x80, 0xA4, 0x27, 0xB7, 0x00, 0xFF,
    0x24, 0x85, 0x9C, 0x27, 0xB7, 0x00, 0xB0, 0x01, 0xC3, 0x32, 0xC0, 0xC3, 0x96, 0x27, 0xB7, 0x00,
    0x99, 0x27, 0xB7, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01,
    0x01, 0x00,
];
const CLASS_IS_MELEE_R3_BODY_CALLS: [usize; 0] = [];
const MOUNT_FIND_R3_BODY: [u8; 81] = [
    0x85, 0xC0, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x80, 0xE4, 0x84, 0x00, 0x00, 0x83, 0xC0, 0x04,
    0x80, 0x78, 0x78, 0x01, 0x75, 0xEE, 0x56, 0x8B, 0x30, 0x8B, 0x8E, 0xC8, 0x00, 0x00, 0x00, 0x33,
    0xC0, 0x85, 0xC9, 0x76, 0x19, 0x8B, 0x96, 0xCC, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x39, 0x3A, 0x74, 0x0A, 0x83, 0xC0, 0x01, 0x83, 0xC2, 0x20, 0x3B, 0xC1, 0x72, 0xF2, 0x3B, 0xC1,
    0x75, 0x04, 0x33, 0xC0, 0x5E, 0xC3, 0xC1, 0xE0, 0x05, 0x03, 0x86, 0xCC, 0x00, 0x00, 0x00, 0x5E,
    0xC3,
];
const MOUNT_FIND_R3_BODY_CALLS: [usize; 0] = [];
const PLAY_FOLEY_2D_R3_BODY: [u8; 90] = [
    0x83, 0xEC, 0x5C, 0x57, 0x6A, 0x5C, 0x33, 0xFF, 0x8D, 0x44, 0x24, 0x08, 0x57, 0x50, 0xE8, 0x6D,
    0xAF, 0x83, 0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24, 0x70, 0xD9, 0x54, 0x24, 0x1C, 0x66, 0x89,
    0x4C, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x20, 0x83, 0xC4, 0x0C, 0x8D, 0x44, 0x24, 0x04, 0x8B, 0xCE,
    0x89, 0x7C, 0x24, 0x2C, 0x66, 0xC7, 0x44, 0x24, 0x34, 0xFF, 0xFF, 0x66, 0xC7, 0x44, 0x24, 0x36,
    0x01, 0x00, 0x89, 0x7C, 0x24, 0x58, 0x89, 0x7C, 0x24, 0x1C, 0x89, 0x7C, 0x24, 0x50, 0xE8, 0xFD,
    0xFE, 0xFF, 0xFF, 0x8B, 0xC6, 0x5F, 0x83, 0xC4, 0x5C, 0xC3,
];
const PLAY_FOLEY_2D_R3_BODY_CALLS: [usize; 2] = [0xe, 0x4e];
const PLAY_FOLEY_2D_TAGGED_R3_BODY: [u8; 100] = [
    0x83, 0xEC, 0x5C, 0x6A, 0x5C, 0x8D, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x50, 0xE8, 0xDF, 0xAE, 0x83,
    0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24, 0x6C, 0xD9, 0x54, 0x24, 0x18, 0x8B, 0x54, 0x24, 0x70,
    0xD9, 0x5C, 0x24, 0x1C, 0x66, 0x89, 0x4C, 0x24, 0x0C, 0x8D, 0x44, 0x24, 0x0C, 0x8B, 0xCE, 0xC7,
    0x44, 0x24, 0x34, 0x00, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x44, 0x24, 0x3C, 0xFF, 0xFF, 0x66, 0xC7,
    0x44, 0x24, 0x3E, 0x01, 0x00, 0xC7, 0x44, 0x24, 0x60, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x44, 0x24,
    0x24, 0x00, 0x00, 0x00, 0x00, 0x89, 0x54, 0x24, 0x58, 0xE8, 0x62, 0xFE, 0xFF, 0xFF, 0x8B, 0xC6,
    0x83, 0xC4, 0x68, 0xC3,
];
const PLAY_FOLEY_2D_TAGGED_R3_BODY_CALLS: [usize; 2] = [0xc, 0x59];
const PLAY_FOLEY_3D_R3_BODY: [u8; 39] = [
    0x83, 0xEC, 0x5C, 0x53, 0x8B, 0x5C, 0x24, 0x64, 0x56, 0x50, 0x6A, 0x00, 0x51, 0x8D, 0x74, 0x24,
    0x14, 0xE8, 0x6A, 0xED, 0xFF, 0xFF, 0x8B, 0xC6, 0x8B, 0xCB, 0xE8, 0x31, 0xFE, 0xFF, 0xFF, 0x5E,
    0x8B, 0xC3, 0x5B, 0x83, 0xC4, 0x5C, 0xC3,
];
const PLAY_FOLEY_3D_R3_BODY_CALLS: [usize; 2] = [0x11, 0x1a];
const PLAY_LINE_2D_R3_BODY: [u8; 99] = [
    0x83, 0xEC, 0x5C, 0x6A, 0x5C, 0x8D, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x50, 0xE8, 0x3F, 0xAE, 0x83,
    0x00, 0xD9, 0xE8, 0x66, 0x8B, 0x4C, 0x24, 0x6C, 0xD9, 0x54, 0x24, 0x18, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0xD9, 0x5C, 0x24, 0x1C, 0x66, 0x89, 0x44, 0x24, 0x3E, 0x66, 0x89, 0x4C, 0x24, 0x0C, 0x89,
    0x44, 0x24, 0x24, 0x8D, 0x44, 0x24, 0x0C, 0x8B, 0xCE, 0xC7, 0x44, 0x24, 0x34, 0x00, 0x00, 0x00,
    0x00, 0x66, 0xC7, 0x44, 0x24, 0x3C, 0xFF, 0xFF, 0xC7, 0x44, 0x24, 0x60, 0x00, 0x00, 0x00, 0x00,
    0xC7, 0x44, 0x24, 0x58, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xC3, 0xFD, 0xFF, 0xFF, 0x8B, 0xC6, 0x83,
    0xC4, 0x68, 0xC3,
];
const PLAY_LINE_2D_R3_BODY_CALLS: [usize; 2] = [0xc, 0x58];
const AUDIO_PREPARE_R3_BODY: [u8; 135] = [
    0x56, 0x83, 0xCA, 0xFF, 0x57, 0x0B, 0xFA, 0x66, 0x81, 0x38, 0xFF, 0xFF, 0x8B, 0xF1, 0xB9, 0xFF,
    0x00, 0x00, 0x00, 0x75, 0x0D, 0x89, 0x0E, 0x89, 0x7E, 0x04, 0x5F, 0x89, 0x56, 0x08, 0x8B, 0xC6,
    0x5E, 0xC3, 0x83, 0x3D, 0x44, 0x6B, 0x52, 0x02, 0x00, 0x7E, 0x06, 0xF6, 0x40, 0x58, 0x02, 0x74,
    0xE4, 0x53, 0x8B, 0x58, 0x18, 0x83, 0xEB, 0x00, 0x74, 0x3A, 0x83, 0xEB, 0x01, 0x74, 0x24, 0x83,
    0xEB, 0x01, 0x74, 0x0E, 0x89, 0x0E, 0x89, 0x7E, 0x04, 0x5B, 0x5F, 0x89, 0x56, 0x08, 0x8B, 0xC6,
    0x5E, 0xC3, 0x6A, 0x01, 0x50, 0xE8, 0x46, 0x49, 0x01, 0x00, 0x83, 0xC4, 0x08, 0x5B, 0x5F, 0x8B,
    0xC6, 0x5E, 0xC3, 0x6A, 0x01, 0x50, 0xE8, 0x45, 0x55, 0x01, 0x00, 0x83, 0xC4, 0x08, 0x5B, 0x5F,
    0x8B, 0xC6, 0x5E, 0xC3, 0x6A, 0x01, 0x50, 0x8B, 0xFE, 0xE8, 0xE2, 0x3F, 0x01, 0x00, 0x83, 0xC4,
    0x08, 0x5B, 0x5F, 0x8B, 0xC6, 0x5E, 0xC3,
];
const AUDIO_PREPARE_R3_BODY_CALLS: [usize; 3] = [0x55, 0x66, 0x79];
const SET_LISTENER_R3_BODY: [u8; 140] = [
    0x8B, 0x54, 0x24, 0x04, 0xD9, 0x02, 0xD9, 0x1D, 0x54, 0xB2, 0xEE, 0x00, 0xD9, 0x42, 0x04, 0xD9,
    0x1D, 0x58, 0xB2, 0xEE, 0x00, 0xD9, 0x42, 0x08, 0x8B, 0x54, 0x24, 0x10, 0xD9, 0x1D, 0x5C, 0xB2,
    0xEE, 0x00, 0x89, 0x15, 0x30, 0xB2, 0xEE, 0x00, 0xD9, 0x40, 0x18, 0xD9, 0x1D, 0x3C, 0xB2, 0xEE,
    0x00, 0xD9, 0x40, 0x1C, 0xD9, 0x1D, 0x40, 0xB2, 0xEE, 0x00, 0xD9, 0x40, 0x20, 0xD9, 0x1D, 0x44,
    0xB2, 0xEE, 0x00, 0xD9, 0x40, 0x0C, 0xD9, 0x1D, 0x48, 0xB2, 0xEE, 0x00, 0xD9, 0x40, 0x10, 0xD9,
    0x1D, 0x4C, 0xB2, 0xEE, 0x00, 0xD9, 0x40, 0x14, 0x8B, 0x44, 0x24, 0x08, 0xD9, 0x1D, 0x50, 0xB2,
    0xEE, 0x00, 0xA3, 0x38, 0xB2, 0xEE, 0x00, 0xD9, 0x01, 0xD9, 0x1D, 0x60, 0xB2, 0xEE, 0x00, 0xD9,
    0x41, 0x04, 0xD9, 0x1D, 0x64, 0xB2, 0xEE, 0x00, 0xD9, 0x41, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0xD9,
    0x1D, 0x68, 0xB2, 0xEE, 0x00, 0x89, 0x0D, 0x34, 0xB2, 0xEE, 0x00, 0xC3,
];
const SET_LISTENER_R3_BODY_CALLS: [usize; 0] = [];
const STATION_STOP_R3_BODY: [u8; 102] = [
    0x85, 0xC0, 0x74, 0x61, 0x8B, 0x40, 0x44, 0x3B, 0x05, 0x30, 0xA5, 0xE9, 0x00, 0x75, 0x56, 0xA0,
    0x24, 0xA5, 0xE9, 0x00, 0x3C, 0xFF, 0x74, 0x31, 0x8B, 0x15, 0x24, 0xA5, 0xE9, 0x00, 0xC1, 0xEA,
    0x08, 0x74, 0x26, 0x0F, 0xB6, 0xC0, 0x8B, 0xC8, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x91,
    0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x13, 0x83, 0xB9, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x74, 0x0A, 0xBA,
    0xEE, 0x02, 0x00, 0x00, 0xE8, 0xF7, 0xED, 0xFE, 0xFF, 0xD9, 0xEE, 0xC7, 0x05, 0xD0, 0x7C, 0x52,
    0x02, 0x00, 0x00, 0x00, 0x00, 0xD9, 0x1D, 0x34, 0xA5, 0xE9, 0x00, 0xC7, 0x05, 0x30, 0xA5, 0xE9,
    0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const STATION_STOP_R3_BODY_CALLS: [usize; 1] = [0x44];
const LOCK_CLEAR_R3_BODY: [u8; 58] = [
    0x83, 0x3D, 0x2C, 0xA1, 0x52, 0x02, 0x00, 0x74, 0x30, 0xD9, 0xEE, 0x51, 0x33, 0xD2, 0xD9, 0x1C,
    0x24, 0x33, 0xC9, 0xE8, 0x38, 0xF4, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0xC7, 0x05, 0x30, 0xA1, 0x52,
    0x02, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x05, 0x2C, 0xA1, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0xB9,
    0x5C, 0xD0, 0xEB, 0x00, 0xE9, 0x17, 0x4A, 0x91, 0xFF, 0xC3,
];
const LOCK_CLEAR_R3_BODY_CALLS: [usize; 1] = [0x13];
const REMOVE_RESERVE_R3_BODY: [u8; 69] = [
    0x85, 0xC9, 0x74, 0x3E, 0x85, 0xF6, 0x74, 0x3A, 0x85, 0xD2, 0x7C, 0x36, 0x74, 0x34, 0xE8, 0x9D,
    0x27, 0xF6, 0xFF, 0x84, 0xC0, 0x74, 0x1A, 0x8B, 0x46, 0x08, 0x83, 0x78, 0x34, 0x16, 0x74, 0x11,
    0x8B, 0x89, 0xDC, 0x11, 0x00, 0x00, 0x51, 0x8B, 0xC2, 0x8B, 0xCE, 0xE8, 0xB0, 0x53, 0x1A, 0x00,
    0xC3, 0x8B, 0x4E, 0x14, 0x3B, 0xD1, 0x8B, 0xC2, 0x7C, 0x02, 0x8B, 0xC1, 0x2B, 0xC8, 0x89, 0x4E,
    0x14, 0xC3, 0x33, 0xC0, 0xC3,
];
const REMOVE_RESERVE_R3_BODY_CALLS: [usize; 2] = [0xe, 0x2b];
const CUE_ADD_R3_BODY: [u8; 330] = [
    0x83, 0xEC, 0x24, 0x53, 0x8B, 0x5C, 0x24, 0x2C, 0x55, 0x56, 0x57, 0x8B, 0xF0, 0xE8, 0x2E, 0xFA,
    0xFF, 0xFF, 0x3D, 0xFF, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x14, 0x0F, 0x84, 0x1E, 0x01, 0x00,
    0x00, 0x8B, 0x0D, 0x04, 0x87, 0xEF, 0x00, 0xC1, 0xE0, 0x05, 0x03, 0xC1, 0x83, 0x78, 0x0C, 0x00,
    0x0F, 0x84, 0x09, 0x01, 0x00, 0x00, 0x85, 0xF6, 0x0F, 0x84, 0x01, 0x01, 0x00, 0x00, 0x56, 0x50,
    0xE8, 0xCB, 0xE4, 0xFE, 0xFF, 0x0F, 0xB7, 0xE8, 0x83, 0xC4, 0x08, 0x66, 0x81, 0xFD, 0xFF, 0xFF,
    0x0F, 0x84, 0xE9, 0x00, 0x00, 0x00, 0x83, 0x3D, 0xCC, 0x6A, 0x52, 0x02, 0x00, 0x74, 0x46, 0x8B,
    0x54, 0x24, 0x14, 0x8A, 0x44, 0x24, 0x3C, 0x8B, 0x0D, 0x54, 0x6B, 0x52, 0x02, 0x8D, 0x7C, 0x24,
    0x18, 0x66, 0x89, 0x6C, 0x24, 0x18, 0x89, 0x54, 0x24, 0x1C, 0x89, 0x5C, 0x24, 0x20, 0x88, 0x44,
    0x24, 0x24, 0x89, 0x4C, 0x24, 0x28, 0xC7, 0x44, 0x24, 0x2C, 0x00, 0x17, 0x48, 0x00, 0xE8, 0xCD,
    0xE3, 0xFE, 0xFF, 0x8B, 0x3D, 0x54, 0x6B, 0x52, 0x02, 0x3B, 0xC7, 0x74, 0x52, 0x5F, 0x5E, 0x5D,
    0x5B, 0x83, 0xC4, 0x24, 0xC3, 0x8B, 0x3D, 0x54, 0x6B, 0x52, 0x02, 0x33, 0xC0, 0x85, 0xFF, 0x76,
    0x3E, 0x0F, 0xB7, 0x34, 0x85, 0x0A, 0xA2, 0xEF, 0x00, 0x0F, 0xB7, 0xCE, 0x8B, 0xD1, 0xC1, 0xEA,
    0x08, 0x83, 0xE2, 0x7F, 0x3B, 0xDA, 0x75, 0x20, 0x0F, 0xB6, 0xC9, 0x39, 0x4C, 0x24, 0x14, 0x75,
    0x17, 0x66, 0x3B, 0x2C, 0x85, 0x08, 0xA2, 0xEF, 0x00, 0x75, 0x0D, 0x66, 0x8B, 0xD6, 0x66, 0xC1,
    0xEA, 0x0F, 0x3A, 0x54, 0x24, 0x3C, 0x74, 0x5A, 0x83, 0xC0, 0x01, 0x3B, 0xC7, 0x72, 0xC2, 0x81,
    0xFF, 0x00, 0x60, 0x01, 0x00, 0x73, 0x48, 0x8D, 0x04, 0xBD, 0x08, 0xA2, 0xEF, 0x00, 0x83, 0xC7,
    0x01, 0x85, 0xC0, 0x89, 0x3D, 0x54, 0x6B, 0x52, 0x02, 0x74, 0x34, 0x8B, 0x4C, 0x24, 0x14, 0x81,
    0xF9, 0xFF, 0x00, 0x00, 0x00, 0x73, 0x28, 0x88, 0x48, 0x02, 0x8A, 0x4C, 0x24, 0x3C, 0xC0, 0xE1,
    0x07, 0x80, 0xE3, 0x7F, 0x0A, 0xCB, 0x33, 0xD2, 0x8A, 0xF1, 0x66, 0x89, 0x28, 0x8A, 0x50, 0x02,
    0x66, 0x89, 0x50, 0x02, 0x8D, 0x47, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x24, 0xC3, 0x83,
    0xC8, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x24, 0xC3,
];
const CUE_ADD_R3_BODY_CALLS: [usize; 3] = [0xd, 0x40, 0x8e];
const RADIO_TRACK_FIND_R3_BODY: [u8; 66] = [
    0x51, 0x56, 0x57, 0x8D, 0x44, 0x24, 0x08, 0x50, 0x33, 0xC0, 0xE8, 0xA1, 0xD3, 0x74, 0x00, 0x66,
    0x8B, 0x3D, 0x94, 0x6B, 0x52, 0x02, 0x33, 0xC0, 0x66, 0x85, 0xFF, 0x76, 0x1D, 0x8B, 0x54, 0x24,
    0x08, 0x8B, 0x35, 0x90, 0x30, 0xF5, 0x00, 0x0F, 0xB7, 0xC8, 0x8D, 0x0C, 0x89, 0x39, 0x14, 0x8E,
    0x74, 0x0C, 0x83, 0xC0, 0x01, 0x66, 0x3B, 0xC7, 0x72, 0xED, 0x66, 0x0D, 0xFF, 0xFF, 0x5F, 0x5E,
    0x59, 0xC3,
];
const RADIO_TRACK_FIND_R3_BODY_CALLS: [usize; 1] = [0xa];
const RADIO_STATION_FIND_R3_BODY: [u8; 78] = [
    0x51, 0x56, 0x57, 0x8D, 0x44, 0x24, 0x08, 0x50, 0x33, 0xC0, 0xE8, 0x31, 0xC6, 0x74, 0x00, 0x66,
    0x8B, 0x3D, 0xC8, 0x6B, 0x52, 0x02, 0x33, 0xC0, 0x66, 0x85, 0xFF, 0x76, 0x27, 0x8B, 0x54, 0x24,
    0x08, 0x8B, 0x35, 0x94, 0x30, 0xF5, 0x00, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0xB7, 0xC8, 0x8D, 0x0C, 0xC9, 0x3B, 0x54, 0xCE, 0x04, 0x74, 0x0E, 0x83, 0xC0, 0x01, 0x66,
    0x3B, 0xC7, 0x72, 0xEC, 0x66, 0xA1, 0xA0, 0x6B, 0x52, 0x02, 0x5F, 0x5E, 0x59, 0xC3,
];
const RADIO_STATION_FIND_R3_BODY_CALLS: [usize; 1] = [0xa];
const DJ_STOP_R3_BODY: [u8; 71] = [
    0xA1, 0x38, 0xA5, 0xE9, 0x00, 0x3C, 0xFF, 0x74, 0x2C, 0x8B, 0xD0, 0xC1, 0xEA, 0x08, 0x74, 0x25,
    0x0F, 0xB6, 0xC0, 0x8B, 0xC8, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x91, 0xAC, 0xB9, 0xEE,
    0x00, 0x75, 0x12, 0x83, 0xB9, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x74, 0x09, 0x8B, 0x54, 0x24, 0x04,
    0xE8, 0x9B, 0xED, 0xFE, 0xFF, 0x33, 0xC0, 0xA3, 0xD0, 0x7C, 0x52, 0x02, 0xA3, 0x48, 0xA5, 0xE9,
    0x00, 0xA3, 0x44, 0xA5, 0xE9, 0x00, 0xC3,
];
const DJ_STOP_R3_BODY_CALLS: [usize; 1] = [0x30];
const RADIO_OFF_R3_BODY: [u8; 97] = [
    0x8B, 0x86, 0xE4, 0x84, 0x00, 0x00, 0xF7, 0x80, 0xB0, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04,
    0x75, 0x03, 0x32, 0xC0, 0xC3, 0xF6, 0x86, 0x84, 0x8D, 0x00, 0x00, 0x01, 0x75, 0xF4, 0x8D, 0x8E,
    0x40, 0x8D, 0x00, 0x00, 0xE8, 0x57, 0x1B, 0xFF, 0xFF, 0x8D, 0x8E, 0x4C, 0x8D, 0x00, 0x00, 0xE8,
    0x4C, 0x1B, 0xFF, 0xFF, 0x8D, 0x8E, 0x58, 0x8D, 0x00, 0x00, 0xE8, 0x41, 0x1B, 0xFF, 0xFF, 0x8B,
    0xC6, 0xE8, 0x8A, 0xF5, 0xFF, 0xFF, 0x8B, 0x4E, 0x44, 0x3B, 0x0D, 0x44, 0xA5, 0xE9, 0x00, 0x75,
    0x0D, 0x68, 0xEE, 0x02, 0x00, 0x00, 0xE8, 0xE5, 0xF5, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0xB0, 0x01,
    0xC3,
];
const RADIO_OFF_R3_BODY_CALLS: [usize; 5] = [0x24, 0x2f, 0x3a, 0x41, 0x56];
const RADIO_BLOCK_R3_BODY: [u8; 81] = [
    0x8A, 0x86, 0x84, 0x8D, 0x00, 0x00, 0xA8, 0x01, 0x75, 0x46, 0x0C, 0x01, 0x8D, 0x8E, 0x40, 0x8D,
    0x00, 0x00, 0x88, 0x86, 0x84, 0x8D, 0x00, 0x00, 0xE8, 0xE3, 0xEF, 0xFE, 0xFF, 0x8D, 0x8E, 0x4C,
    0x8D, 0x00, 0x00, 0xE8, 0xD8, 0xEF, 0xFE, 0xFF, 0x8D, 0x8E, 0x58, 0x8D, 0x00, 0x00, 0xE8, 0xCD,
    0xEF, 0xFE, 0xFF, 0x8B, 0xC6, 0xE8, 0x16, 0xCA, 0xFF, 0xFF, 0x8B, 0x46, 0x44, 0x3B, 0x05, 0x44,
    0xA5, 0xE9, 0x00, 0x75, 0x0B, 0x68, 0xEE, 0x02, 0x00, 0x00, 0xE8, 0x71, 0xCA, 0xFF, 0xFF, 0x59,
    0xC3,
];
const RADIO_BLOCK_R3_BODY_CALLS: [usize; 5] = [0x18, 0x23, 0x2e, 0x35, 0x4a];
