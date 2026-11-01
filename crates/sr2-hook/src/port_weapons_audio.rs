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
const COUNT: usize = 22;

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

// --- install ------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy.
unsafe fn set_rel32(copy: usize, off: usize, dest: usize) {
    let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
    unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
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
    let mut ok = 0;
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
    if parts.is_empty() { String::new() } else { format!("port_weapons_audio: {}", parts.join(", ")) }
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
