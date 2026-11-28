//! Port module: the shop proximity / trigger code that runs every frame (doc 56, "Port recipes"),
//! plus `shop_human_enter` / `shop_human_leave` and the shop ambient audio start / stop.
//!
//! Every port goes through a `pushad` thunk (all registers come back as the caller left them;
//! EAX carries a result where the original has one). `shop_release_if_no_human_near` ends in a
//! tail call through the trigger object's vtable, so it has a hand-written thunk that jumps there.
//!
//! Checks:
//! - `character_table_find_index_by_hash`: pure, relocated-original shadow for the first
//!   `SHADOW_CALLS` calls (EAX compared, the original's wins on a mismatch).
//! - `transform_table_get_entry`, `chunk_object_attach_world_transform`: snapshot shadow (the
//!   out vector / matrix, or the object's transform block, saved, the original run, restored, the
//!   Rust version run, compared; the original's writes win on a mismatch).
//! - All the others write state through side-effecting callees (or tail-call one), so they keep
//!   per-branch call counters (see `summary`).
//! - Every port is also checked offline against the original bytes (`mod difftest` and the tests
//!   at the end of this file): random worlds, every call out recorded by a stub and compared.
//!
//! The float code that must match the x87 bit for bit (squared distances, the float adds) is
//! written as the same `fld` / `fsub` / `fstp dword` sequences.

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- the offline test shifts the game's globals (see `difftest`) ----------------------------------

#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn tr(p: usize) -> usize {
    if (0xD0_0000..0x350_0000).contains(&p) { p + DELTA } else { p }
}
#[cfg(test)]
unsafe fn rd(p: usize) -> u32 {
    unsafe { *(tr(p) as *const u32) }
}
#[cfg(test)]
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(tr(p) as *const u8) }
}
#[cfg(test)]
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(tr(p) as *mut u32) = v }
}
#[cfg(test)]
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(tr(p) as *mut u8) = v }
}

/// Every call into the game goes through the recording stubs in the offline test.
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(difftest::redirect(f), eax, ecx, edx, esi, edi, args) }
}

/// Like `call_regs` for a callee that returns a double on ST0.
unsafe fn call_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    #[cfg(test)]
    let f = difftest::redirect(f);
    let mut block: [u32; 10] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, 0, 0];
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
            "fstp qword ptr [ebx + 32]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_mut_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    f64::from_bits(block[8] as u64 | (block[9] as u64) << 32)
}

// --- game addresses -------------------------------------------------------------------------------

const PLAYER: usize = 0x0217_03D4; // the player's human
const HUMAN_RING: usize = 0x0217_043C; // head of the circular list of humans (next at +0x1294)
const PLAYER_POS: usize = 0x025F_5B14; // the player's position, three floats
const SESSION: usize = 0x0252_8C14; // net session
const NET_B56: usize = 0x0252_8B56;
const NET_B2B: usize = 0x0252_8B2B;
const CUR_PLAYER: usize = 0x0252_97EC; // the player's current shop
const CUR_OTHER: usize = 0x0252_97F0; // the other human's current shop
const SHOP_COUNT: usize = 0x021F_2AA4;
const SHOP_ARRAY: usize = 0x021F_2AA8;
const SHOP_RANGE_SQ: usize = 0x00E5_CAD4; // float
const SHOP_CAPTURE_SQ: usize = 0x00DD_0778; // float
const SHOP_BEST_INIT: usize = 0x00E8_725C; // float
const SHOP_MARGIN: usize = 0x00E5_C130; // double
const MODE_21603CC: usize = 0x0216_03CC;
const FLAG_D14: usize = 0x0252_7D14;
const FLAG_CF7: usize = 0x0252_7CF7;
const FLAG_D15: usize = 0x0252_7D15;
const GAME_NOW: usize = 0x00E8_43AC;
const WRAP: u32 = 0x6B49_D200;

// callees
const SHOP_LEAVE: usize = 0x009F_3BB0; // EAX = human, ESI = shop
const SHOP_ENTER: usize = 0x009F_3C00; // cdecl (human), ESI = shop
const SHOP_STOP_FAR: usize = 0x009F_3D00; // EAX = shop
const SHOP_RELEASE: usize = 0x009F_3DB0; // cdecl (shop)
const SHOP_FIND_NEAREST: usize = 0x009F_3F00; // cdecl (human)
const AUDIO_STOP: usize = 0x009F_6FC0; // stdcall (record)
const SQRTF: usize = 0x0040_EA20; // cdecl (float) -> ST0
const VEC3_DIST_SQ: usize = 0x00BD_EA30; // EAX, ECX = vec3* -> ST0
const COOP_PARTNER: usize = 0x009D_7BD0;
const HUMAN_IS_DEAD: usize = 0x0098_5630; // cdecl (human) -> AL
const UI_BLOCKS: usize = 0x009F_6240; // -> AL
const TRIGGER_ACTIVATE: usize = 0x00A6_BCE0; // ESI = trigger
const TRIGGER_DEACTIVATE: usize = 0x00A6_BD60; // EAX = trigger

// --- registers and thunks -------------------------------------------------------------------------

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
}

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    arg: [u32; 2],
    esp: u32,
}

/// Call `f` with all seven registers from `r` and two stack arguments (harmless for the callees
/// that take fewer: the shadows pass the caller's arguments through), then capture all seven.
unsafe fn call_full(f: usize, r: &mut Regs, arg: [u32; 2]) {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0 };
    unsafe {
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
            "mov ebx, [esp + 16]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 40]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX.
macro_rules! regs_thunk {
    ($name:ident, $f:ident) => {
        regs_thunk!($name, $f, "ret");
    };
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 128",
                "movups [esp], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "lea eax, [esp + 128]",
                "lea ecx, [esp + 164]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
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
    calls: AtomicU32,
    mismatches: AtomicU32,
    /// A relocated copy of the original (shadowed ports only).
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize) -> Port {
    Port { name, addr, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const FIND_INDEX: usize = 0;
const XFORM: usize = 1;
const ATTACH: usize = 2;
const STOP_FAR: usize = 3;
const RELEASE: usize = 4;
const NEAREST: usize = 5;
const UPDATE_PROX: usize = 6;
const TRIGGERS: usize = 7;
const EVENT_TICK: usize = 8;
const PART_VIS: usize = 9;
const LEAVE: usize = 10;
const ENTER: usize = 11;
const AUDIO_START: usize = 12;
const AUDIO_STOP_P: usize = 13;
const COUNT: usize = 14;

static PORTS: [Port; COUNT] = [
    port("character_table_find_index_by_hash", 0x006E_12A0),
    port("transform_table_get_entry", 0x006E_6070),
    port("chunk_object_attach_world_transform", 0x0091_E040),
    port("shop_stop_audio_if_player_far", 0x009F_3D00),
    port("shop_release_if_no_human_near", 0x009F_3DB0),
    port("shop_find_nearest_for_human", 0x009F_3F00),
    port("shops_update_proximity", 0x009F_4150),
    port("shop_triggers_update", 0x009F_41D0),
    port("shop_current_event_tick", 0x009F_4480),
    port("pcu_worn_apply_part_visibility", 0x009E_5DB0),
    port("shop_human_leave", 0x009F_3BB0),
    port("shop_human_enter", 0x009F_3C00),
    port("shop_ambient_audio_start", 0x009F_6E80),
    port("shop_ambient_audio_stop", 0x009F_6FC0),
];

fn count(id: usize) {
    PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
}

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_shops: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_shops: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

/// Per-branch hit counters (the unshadowed ports).
const H_NAMES: [&str; 20] = [
    "nearest.net_gate_return",
    "nearest.scored",
    "nearest.left_far",
    "nearest.switched",
    "update.audio_checked",
    "triggers.activate",
    "triggers.deactivate",
    "tick.posted",
    "tick.queue_full",
    "tick.timer_rearmed",
    "partvis.parts",
    "partvis.saved_list",
    "leave.player",
    "enter.net_register",
    "audio_start.stop_path",
    "audio_start.voice_started",
    "audio_stop.voices_stopped",
    "release.tail_call",
    "stop_far.stopped",
    "release.near_human",
];
const H_NEAREST_GATE: usize = 0;
const H_NEAREST_SCORED: usize = 1;
const H_NEAREST_LEFT: usize = 2;
const H_NEAREST_SWITCH: usize = 3;
const H_UPDATE_AUDIO: usize = 4;
const H_TRIG_ACT: usize = 5;
const H_TRIG_DEACT: usize = 6;
const H_TICK_POSTED: usize = 7;
const H_TICK_FULL: usize = 8;
const H_TICK_REARM: usize = 9;
const H_PARTS: usize = 10;
const H_SAVED: usize = 11;
const H_LEAVE_PLAYER: usize = 12;
const H_ENTER_NET: usize = 13;
const H_AUDIO_STOP_PATH: usize = 14;
const H_AUDIO_VOICE: usize = 15;
const H_AUDIO_STOPPED: usize = 16;
const H_RELEASE_TAIL: usize = 17;
const H_STOP_FAR: usize = 18;
const H_RELEASE_NEAR: usize = 19;
static HITS: [AtomicU32; 20] = [const { AtomicU32::new(0) }; 20];

fn hit(i: usize) {
    HITS[i].fetch_add(1, Ordering::Relaxed);
}

// --- shared helpers -------------------------------------------------------------------------------

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

#[repr(C)]
struct D3 {
    a: [u32; 3],
    b: [u32; 3],
    t: u32,
    acc: u32,
}

/// `(a.x-b.x)^2 + (a.y-b.y)^2 + (a.z-b.z)^2` through the x87 exactly as the originals do it: each
/// difference is stored to and reloaded from a float, each partial sum is stored as a float.
/// Returns the bits of the float.
fn dist2(a: [u32; 3], b: [u32; 3]) -> u32 {
    let mut d = D3 { a, b, t: 0, acc: 0 };
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{p}]",
            "fsub dword ptr [{p} + 12]",
            "fstp dword ptr [{p} + 24]",
            "fld dword ptr [{p} + 24]",
            "fmul st(0), st(0)",
            "fstp dword ptr [{p} + 28]",
            "fld dword ptr [{p} + 4]",
            "fsub dword ptr [{p} + 16]",
            "fstp dword ptr [{p} + 24]",
            "fld dword ptr [{p} + 24]",
            "fmul st(0), st(0)",
            "fadd dword ptr [{p} + 28]",
            "fstp dword ptr [{p} + 28]",
            "fld dword ptr [{p} + 8]",
            "fsub dword ptr [{p} + 20]",
            "fstp dword ptr [{p} + 24]",
            "fld dword ptr [{p} + 24]",
            "fmul st(0), st(0)",
            "fadd dword ptr [{p} + 28]",
            "fstp dword ptr [{p} + 28]",
            p = in(reg) &mut d as *mut D3,
            clobber_abi("C"),
        );
    }
    d.acc
}

/// `fld a; fadd b; fstp dword`: the bits of the float sum.
fn fadd32(a: u32, b: u32) -> u32 {
    let mut m = [a, b, 0u32];
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{p}]",
            "fadd dword ptr [{p} + 4]",
            "fstp dword ptr [{p} + 8]",
            p = in(reg) m.as_mut_ptr(),
            clobber_abi("C"),
        );
    }
    m[2]
}

fn flt(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The position the proximity code uses for a human: the player's globals, else +0x12A0.
unsafe fn human_pos(h: u32, player: u32) -> [u32; 3] {
    unsafe {
        if h == player {
            [rd(PLAYER_POS), rd(PLAYER_POS + 4), rd(PLAYER_POS + 8)]
        } else {
            let h = h as usize;
            [rd(h + 0x12A0), rd(h + 0x12A4), rd(h + 0x12A8)]
        }
    }
}

/// A virtual call: thiscall, `slot` of the vtable, the registers as the originals set them
/// (EAX = the function, EDX = the vtable).
unsafe fn vcall(obj: u32, slot: usize, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe {
        let vt = rd(obj as usize);
        let f = rd(vt as usize + slot * 4);
        call_regs(f as usize, f, obj, vt, esi, edi, args)
    }
}

unsafe fn ui_blocks() -> bool {
    unsafe { call_regs(UI_BLOCKS, 0, 0, 0, 0, 0, &[]) as u8 != 0 }
}

/// The "net session lets the shop code act" rule of `shop_human_enter` / `shop_release_if_no_human_near`.
unsafe fn shop_net_exit(s: u32) -> bool {
    unsafe { (rb(NET_B56) != 0 && rb(NET_B2B) == 0) || rb(s as usize + 0x3F8) == 0 }
}

// --- 0x6E12A0 character_table_find_index_by_hash (EDX = &hash -> EAX) -----------------------------

/// Two circular lists: `[0x145A4FC]` (next +0, key +8, value +0xC), then `[0x2527D4C]` (key +0xC,
/// result +0x10). -1 when absent or when the value equals `[0x33D630C]`.
/// Returns (EAX, ECX, EDX) as the original leaves them: the list cursors / heads stay in ECX / EDX.
unsafe fn find_index(edx0: u32) -> (u32, u32, u32) {
    unsafe {
        let head = rd(0x0145_A4FC);
        if head == 0 {
            return (u32::MAX, 0, edx0);
        }
        let h = rd(edx0 as usize);
        let mut n = head;
        loop {
            if rd(n as usize + 8) == h {
                break;
            }
            n = rd(n as usize);
            if n == head || n == 0 {
                return (u32::MAX, head, h);
            }
        }
        let v = rd(n as usize + 0xC);
        if v == rd(0x033D_630C) {
            return (u32::MAX, head, h);
        }
        let head2 = rd(0x0252_7D4C);
        if head2 == 0 {
            return (u32::MAX, 0, 0);
        }
        let mut m = head2;
        loop {
            if rd(m as usize + 0xC) == v {
                return (rd(m as usize + 0x10), m, head2);
            }
            m = rd(m as usize);
            if m == head2 {
                return (u32::MAX, m, head2);
            }
            if m == 0 {
                return (u32::MAX, 0, head2);
            }
        }
    }
}

extern "C" fn find_index_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (ours, fecx, fedx) = unsafe { find_index(p.edx) };
    r.ecx = fecx;
    r.edx = fedx;
    let Some((n, orig)) = shadowing(FIND_INDEX) else {
        r.eax = ours;
        return;
    };
    let mut o = Regs::of(&p);
    unsafe { call_full(orig, &mut o, [0, 0]) };
    if o.eax != ours {
        mismatch(FIND_INDEX, n, format!("edx {:#x}: original {:#x}, Rust {ours:#x}", p.edx, o.eax));
        r.eax = o.eax;
    } else {
        r.eax = ours;
    }
}
regs_thunk!(find_index_thunk, find_index_entry);

// --- 0x6E6070 transform_table_get_entry (EAX = index, EDX = out vec3, stack (table, out mat33)) ---

const IDENTITY33: usize = 0x00EC_5AE4;
const QUAT_TO_MAT33: usize = 0x00C0_57D0; // EAX = quaternion, ECX = out mat33

/// Returns the new EAX. A bad index copies the identity matrix and gives AL = 0; the original
/// copies even into a null matrix pointer (and faults); this skips that copy.
unsafe fn xform_get(eax: u32, edx: u32, table: u32, mat: u32) -> u32 {
    unsafe {
        let lo = |al: u32| (eax & !0xFF) | al;
        if table == 0 {
            return lo(0);
        }
        let idx = eax as i32;
        if idx < 0 || idx >= rd(table as usize + 0xC) as i32 >> 16 {
            if mat != 0 {
                for k in 0..9 {
                    wd(mat as usize + k * 4, rd(IDENTITY33 + k * 4));
                }
            }
            return lo(0);
        }
        let off = eax.wrapping_mul(0x60);
        let rec = rd(table as usize + 0x10).wrapping_add(off) as usize;
        let s = rec + 0x44;
        let d = edx as usize;
        wd(d, rd(s));
        wd(d + 4, rd(s + 4));
        wd(d + 8, rd(s + 8));
        let mut e = off;
        if mat != 0 {
            let t = rd(table as usize + 0x10);
            e = off.wrapping_add(t).wrapping_add(0x50);
            e = call_regs(QUAT_TO_MAT33, e, mat, t, 0, 0, &[]);
        }
        (e & !0xFF) | 1
    }
}

extern "C" fn xform_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (table, mat) = unsafe { (*args, *args.add(1)) };
    let fail_null_mat = unsafe { table != 0 && mat == 0 && ((p.eax as i32) < 0 || p.eax as i32 >= rd(table as usize + 0xC) as i32 >> 16) };
    let Some((n, orig)) = shadowing(XFORM).filter(|_| !fail_null_mat) else {
        r.eax = unsafe { xform_get(p.eax, p.edx, table, mat) };
        return;
    };
    // Snapshot shadow: the out vector (12 bytes at EDX) and matrix (36 at `mat`).
    let save = |a: u32, n: usize, out: &mut [u8; 48]| unsafe {
        if a != 0 {
            std::ptr::copy_nonoverlapping(a as *const u8, out.as_mut_ptr(), n);
        }
    };
    let put = |a: u32, n: usize, from: &[u8; 48]| unsafe {
        if a != 0 {
            std::ptr::copy_nonoverlapping(from.as_ptr(), a as *mut u8, n);
        }
    };
    let (edx_a, mat_a) = (p.edx, mat);
    let mut v0 = [0u8; 48];
    let mut m0 = [0u8; 48];
    save(edx_a, 12, &mut v0);
    save(mat_a, 36, &mut m0);
    let mut o = Regs::of(&p);
    unsafe { call_full(orig, &mut o, [table, mat]) };
    let (mut v1, mut m1) = ([0u8; 48], [0u8; 48]);
    save(edx_a, 12, &mut v1);
    save(mat_a, 36, &mut m1);
    put(edx_a, 12, &v0);
    put(mat_a, 36, &m0);
    let ours = unsafe { xform_get(p.eax, p.edx, table, mat) };
    let (mut v2, mut m2) = ([0u8; 48], [0u8; 48]);
    save(edx_a, 12, &mut v2);
    save(mat_a, 36, &mut m2);
    if (o.eax & 0xFF) != (ours & 0xFF) || v1 != v2 || m1 != m2 {
        mismatch(XFORM, n, format!("eax {:#x} edx {:#x} table {table:#x} mat {mat:#x}: original al {:#x}, Rust {:#x}", p.eax, p.edx, o.eax & 0xFF, ours & 0xFF));
        put(edx_a, 12, &v1);
        put(mat_a, 36, &m1);
        r.eax = o.eax;
    } else {
        r.eax = ours;
    }
}
regs_thunk!(xform_thunk, xform_entry);

// --- 0x91E040 chunk_object_attach_world_transform (EDX = rig table, stack (object)) --------------

const VEC3_MUL_MAT33: usize = 0x00BD_E570; // ECX = vec3*, EAX = mat33*
const MAT33_MUL: usize = 0x00BE_2F50; // ECX = a, EAX = out, stack (b) -> EAX = out

/// The attachment transform: bone `[[obj+0xC]+0x84]` (signed; <= -1 copies the part's values
/// through), `v = (v * bone_matrix) + bone_position`, `r = r * bone_matrix`, the matrix product
/// bone * M into obj+0x6C.
unsafe fn attach(table: u32, obj: u32) {
    unsafe {
        let ob = obj as usize;
        let o = rd(ob + 0xC) as usize;
        let b = rd(o + 0x84) as i32;
        if b <= -1 {
            wd(ob + 0x54, rd(o + 0x3C));
            wd(ob + 0x58, rd(o + 0x40));
            wd(ob + 0x5C, rd(o + 0x44));
            wd(ob + 0x60, rd(o + 0x30));
            wd(ob + 0x64, rd(o + 0x34));
            wd(ob + 0x68, rd(o + 0x38));
            for k in 0..9 {
                wd(ob + 0x6C + k * 4, rd(o + 0x48 + k * 4));
            }
            return;
        }
        let mut v = [q32(rd(o + 0x3C)), q32(rd(o + 0x40)), q32(rd(o + 0x44))];
        let mut r = [q32(rd(o + 0x30)), q32(rd(o + 0x34)), q32(rd(o + 0x38))];
        let mut m = [0u32; 9];
        for (k, x) in m.iter_mut().enumerate() {
            *x = q32(rd(o + 0x48 + k * 4));
        }
        let rec = rd(table as usize + 0x4C).wrapping_add((b as u32).wrapping_mul(0x50)) as usize;
        let mp = rd(rec + 0xC) as usize;
        let (pos, bp) = if mp != 0 {
            ([rd(mp + 0x14), rd(mp + 0x18), rd(mp + 0x1C)], mp + 0x20)
        } else {
            let q = rd(rec + 0x40) as usize;
            ([rd(q), rd(q + 4), rd(q + 8)], q + 0xC)
        };
        let mut bone = [0u32; 9];
        for (k, x) in bone.iter_mut().enumerate() {
            *x = rd(bp + k * 4);
        }
        let bone_p = bone.as_mut_ptr() as u32;
        let a = call_regs(VEC3_MUL_MAT33, bone_p, v.as_mut_ptr() as u32, 0, 0, 0, &[]);
        v[0] = fadd32(pos[0], v[0]);
        v[1] = fadd32(pos[1], v[1]);
        v[2] = fadd32(pos[2], v[2]);
        call_regs(VEC3_MUL_MAT33, a, r.as_mut_ptr() as u32, 0, 0, 0, &[]);
        wd(ob + 0x54, v[0]);
        wd(ob + 0x60, r[0]);
        wd(ob + 0x58, v[1]);
        wd(ob + 0x64, r[1]);
        wd(ob + 0x5C, v[2]);
        wd(ob + 0x68, r[2]);
        let mut out = [0u32; 9];
        let res = call_regs(MAT33_MUL, out.as_mut_ptr() as u32, bone_p, 0, 0, 0, &[m.as_ptr() as u32]) as usize;
        for k in 0..9 {
            wd(ob + 0x6C + k * 4, *((res + k * 4) as *const u32));
        }
    }
}

extern "C" fn attach_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let obj = unsafe { *args };
    let Some((n, orig)) = shadowing(ATTACH) else {
        unsafe { attach(p.edx, obj) };
        return;
    };
    // Snapshot shadow over obj+0x54 .. obj+0x90.
    const LEN: usize = 0x3C;
    let at = obj as usize + 0x54;
    let mut s0 = [0u8; LEN];
    let mut s1 = [0u8; LEN];
    let mut s2 = [0u8; LEN];
    unsafe {
        std::ptr::copy_nonoverlapping(at as *const u8, s0.as_mut_ptr(), LEN);
        let mut o = Regs::of(&p);
        call_full(orig, &mut o, [obj, 0]);
        std::ptr::copy_nonoverlapping(at as *const u8, s1.as_mut_ptr(), LEN);
        std::ptr::copy_nonoverlapping(s0.as_ptr(), at as *mut u8, LEN);
        attach(p.edx, obj);
        std::ptr::copy_nonoverlapping(at as *const u8, s2.as_mut_ptr(), LEN);
        if s1 != s2 {
            mismatch(ATTACH, n, format!("obj {obj:#x} table {:#x}: original {s1:x?} Rust {s2:x?}", p.edx));
            std::ptr::copy_nonoverlapping(s1.as_ptr(), at as *mut u8, LEN);
        }
    }
}
regs_thunk!(attach_thunk, attach_entry);

// --- 0x9F3D00 shop_stop_audio_if_player_far (EAX = shop) ------------------------------------------

/// The squared distance from the player (and from the co-op partner, if closer) to the shop's
/// door position; past `[0xE5CAD4]` the shop's ambient audio is stopped.
unsafe fn stop_far(shop: u32) {
    unsafe {
        let s = shop as usize;
        let q = rd(rd(rd(s + 4) as usize + 0x10) as usize + 0x18) as usize + 0x1C;
        let p = rd(PLAYER) as usize;
        let qv = [rd(q), rd(q + 4), rd(q + 8)];
        let mut d2 = dist2([rd(p + 0x14), rd(p + 0x18), rd(p + 0x1C)], qv);
        let c = call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]);
        if c != 0 {
            // vec3_distance_sq(EAX = q, ECX = partner + 0x14) -> ST0, stored as a float
            let e = call_st0(VEC3_DIST_SQ, q as u32, c.wrapping_add(0x14), 0, q as u32, shop, &[]) as f32;
            // fcom: keep d2 only when e > d2 (ordered)
            if !(e > flt(d2)) {
                d2 = q32(e.to_bits());
            }
        }
        // fld R; fcomp d2: stop only when R < d2 (ordered)
        if flt(rd(SHOP_RANGE_SQ)) < flt(d2) {
            hit(H_STOP_FAR);
            call_regs(AUDIO_STOP, 0, 0, 0, q as u32, shop.wrapping_add(0x28), &[shop.wrapping_add(0x28)]);
        }
    }
}

extern "C" fn stop_far_entry(r: &mut Pushad, _: *const u32) {
    count(STOP_FAR);
    unsafe { stop_far(r.eax) };
}
regs_thunk!(stop_far_thunk, stop_far_entry);

// --- 0x9F3DB0 shop_release_if_no_human_near (cdecl (shop); tail jump) ------------------------------

/// Returns the trigger object whose vtable slot 7 (+0x1C) the original tail-calls, or 0 when it
/// just returns.
unsafe fn release_decide(shop: u32) -> u32 {
    unsafe {
        let s = rd(SESSION);
        if s != 0 && shop_net_exit(s) {
            return 0;
        }
        let sh = shop as usize;
        let trig = rd(rd(sh + 4) as usize + 0x10);
        let vt = rd(trig as usize);
        let f = rd(vt as usize + 0x34);
        let obj = call_regs(f as usize, f, trig, vt, 0, shop, &[]);
        if obj == 0 || rd(CUR_PLAYER) == shop || rd(CUR_OTHER) == shop {
            return 0;
        }
        let head = rd(HUMAN_RING);
        if head != 0 {
            let player = rd(PLAYER);
            let ob = obj as usize;
            let target = [rd(ob + 0x14), rd(ob + 0x18), rd(ob + 0x1C)];
            let range = flt(rd(SHOP_RANGE_SQ));
            let mut h = head;
            loop {
                let d2 = dist2(human_pos(h, player), target);
                if flt(d2) < range {
                    hit(H_RELEASE_NEAR);
                    return 0;
                }
                h = rd(h as usize + 0x1294);
                if h == head || h == 0 {
                    break;
                }
            }
        }
        hit(H_RELEASE_TAIL);
        rd(rd(sh + 4) as usize + 0x10)
    }
}

extern "C" fn release_entry(shop: u32) -> u32 {
    count(RELEASE);
    unsafe { release_decide(shop) }
}

/// cdecl (shop). Returns, or jumps to the trigger object's vtable slot 7 with ECX = the object
/// (the shop argument stays on the stack for it, as in the original).
#[unsafe(naked)]
extern "C" fn release_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 4]",
        "call {f}",
        "add esp, 4",
        "test eax, eax",
        "jz 2f",
        "mov ecx, eax",
        "mov edx, dword ptr [ecx]",
        "mov eax, dword ptr [edx + 0x1c]",
        "jmp eax",
        "2:",
        "ret",
        f = sym release_entry,
    )
}

// --- 0x9F3BB0 shop_human_leave (EAX = human, ESI = shop) ------------------------------------------

const NET_RELEASE: usize = 0x00A7_9240; // EDX, stack (5, 1, 0)
const NET_ADD_REF: usize = 0x00A7_9120; // ECX = 1, EDI, stack (5, 0)

/// Returns the callee's EAX when the player path ran (ECX is then whatever the callee leaves).
unsafe fn leave(human: u32, shop: u32, edi: u32) -> Option<u32> {
    unsafe {
        let player = rd(PLAYER);
        let is_player = human != 0 && human == player;
        if is_player {
            wd(CUR_PLAYER, 0);
        } else {
            wd(CUR_OTHER, 0);
        }
        let sh = shop as usize;
        if is_player {
            hit(H_LEAVE_PLAYER);
            let ecx = rd(rd(sh + 4) as usize + 0x10);
            let eax = rd(rd(ecx as usize + 0x6C) as usize + 8);
            let edx = rd(eax as usize + 0x68);
            let r = call_regs(NET_RELEASE, eax, ecx, edx, shop, edi, &[5, 1, 0]);
            wb(sh + 0x25, 0);
            wb(sh + 0x9C, 0);
            return Some(r);
        }
        wb(sh + 0x25, 0);
        wb(sh + 0x9C, 0);
        None
    }
}

extern "C" fn leave_entry(r: &mut Pushad, _: *const u32) {
    count(LEAVE);
    // the original leaves ECX = the local player (`mov ecx, [0x21703D4]`) unless the callee clobbers it
    match unsafe { leave(r.eax, r.esi, r.edi) } {
        Some(e) => r.eax = e,
        None => r.ecx = unsafe { rd(PLAYER) },
    }
}
regs_thunk!(leave_thunk, leave_entry);

// --- 0x9F3C00 shop_human_enter (cdecl (human), ESI = shop) ----------------------------------------

const NET_OBJECT_REGISTER: usize = 0x0091_2370; // cdecl (id, flag)
const RSCHED_FN_804F20: usize = 0x0080_4F20; // ECX = handle -> EAX
const RSCHED_FN_805000: usize = 0x0080_5000; // EDI = handle, cdecl (5)

unsafe fn enter(human: u32, shop: u32, edi0: u32) {
    unsafe {
        let player = rd(PLAYER);
        let sh = shop as usize;
        let mut edi = edi0;
        if human != 0 && human == player {
            let ecx = rd(rd(sh + 4) as usize + 0x10);
            let eax = rd(rd(ecx as usize + 0x6C) as usize + 8);
            edi = rd(eax as usize + 0x68);
            wd(CUR_PLAYER, shop);
            call_regs(NET_ADD_REF, eax, 1, rd(ecx as usize + 0x6C), shop, edi, &[5, 0]);
        } else {
            wd(CUR_OTHER, shop);
        }
        let s = rd(SESSION);
        if s == 0 {
            return;
        }
        if rb(NET_B56) != 0 && rb(NET_B2B) == 0 {
            return;
        }
        if rb(s as usize + 0x3F8) == 0 {
            return;
        }
        if rb(FLAG_D14) != 0 && rb(FLAG_CF7) != 0 {
            return;
        }
        if rb(FLAG_D15) != 0 {
            return;
        }
        let trig = || rd(rd(sh + 4) as usize + 0x10);
        if vcall(trig(), 0x2C >> 2, shop, edi, &[]) as u8 != 0 {
            return;
        }
        hit(H_ENTER_NET);
        vcall(trig(), 0x18 >> 2, shop, edi, &[]);
        let r = vcall(trig(), 0x34 >> 2, shop, edi, &[1]);
        let edx = rd(rd(sh + 4) as usize + 0x14);
        call_regs(NET_OBJECT_REGISTER, r, rd(sh + 4), edx, shop, edi, &[edx, r]);
        if human != 0 && human == rd(PLAYER) {
            return;
        }
        let r = vcall(trig(), 0x34 >> 2, shop, edi, &[]);
        let ecx = rd(r as usize + 0x44);
        let eax = call_regs(RSCHED_FN_804F20, r, ecx, 0, shop, edi, &[]);
        if eax != 0 {
            return;
        }
        call_regs(RSCHED_FN_805000, eax, ecx, 0, shop, ecx, &[5]);
    }
}

extern "C" fn enter_entry(r: &mut Pushad, args: *const u32) {
    count(ENTER);
    unsafe { enter(*args, r.esi, r.edi) };
}
regs_thunk!(enter_thunk, enter_entry);

// --- 0x9F3F00 shop_find_nearest_for_human (cdecl (human)) -----------------------------------------

/// The per-human proximity pass: the nearest active shop within the capture radius is entered
/// (the previous one left); a current shop that has drifted past the range is left.
unsafe fn find_nearest(human: u32) {
    unsafe {
        let player = rd(PLAYER);
        let s = rd(SESSION);
        if s != 0 {
            let rule = (rb(NET_B56) != 0 && rb(NET_B2B) == 0) || rb(s as usize + 0x3F8) == 0;
            if rule && (human == 0 || human != player) {
                hit(H_NEAREST_GATE);
                return;
            }
        }
        let pos = human_pos(human, player);
        let mut best = q32(rd(SHOP_BEST_INIT));
        let mut nearest = 0u32;
        let is_player = human != 0 && human == player;
        let mut cur = if is_player { rd(CUR_PLAYER) } else { rd(CUR_OTHER) };
        let mut i = 0i32;
        let mut off = 0u32;
        if (rd(SHOP_COUNT) as i32) > 0 {
            loop {
                let shop = rd(SHOP_ARRAY).wrapping_add(off);
                let sh = shop as usize;
                if rb(sh + 0x24) & 4 != 0 {
                    let q = rd(rd(rd(sh + 4) as usize + 0x10) as usize + 0x18) as usize + 0x1C;
                    let mut d2 = dist2(pos, [rd(q), rd(q + 4), rd(q + 8)]);
                    let mut score = true;
                    if shop == cur {
                        if !(flt(d2) > flt(rd(SHOP_RANGE_SQ))) {
                            // inside the range (or NaN): the distance to the edge of the margin
                            let st = call_st0(SQRTF, 0, 0, 0, shop, cur, &[d2]);
                            let dd = (st - f64::from_bits(rd(SHOP_MARGIN) as u64 | (rd(SHOP_MARGIN + 4) as u64) << 32)) as f32;
                            let d = if 0.0f32 < dd { dd } else { 0.0 };
                            d2 = ((d as f64 * d as f64) as f32).to_bits();
                        } else {
                            hit(H_NEAREST_LEFT);
                            call_regs(SHOP_LEAVE, human, 0, 0, shop, cur, &[]);
                            cur = 0;
                            score = false;
                        }
                    }
                    if score {
                        hit(H_NEAREST_SCORED);
                        call_regs(SHOP_RELEASE, 0, 0, 0, shop, cur, &[shop]);
                        let d2f = flt(d2);
                        if !(d2f > flt(rd(SHOP_CAPTURE_SQ))) && rb(sh + 0x26) & 0x80 == 0 && flt(best) > d2f {
                            best = d2;
                            nearest = shop;
                        }
                    }
                }
                i = i.wrapping_add(1);
                off = off.wrapping_add(0xA0);
                if !(i < rd(SHOP_COUNT) as i32) {
                    break;
                }
            }
        }
        if nearest != 0 && nearest != cur {
            hit(H_NEAREST_SWITCH);
            if cur != 0 {
                call_regs(SHOP_LEAVE, human, 0, 0, cur, cur, &[]);
            }
            if human != 0 && human == rd(PLAYER) {
                wd(CUR_PLAYER, nearest);
            } else {
                wd(CUR_OTHER, nearest);
            }
            call_regs(SHOP_ENTER, 0, 0, 0, nearest, cur, &[human]);
        }
    }
}

extern "C" fn nearest_entry(r: &mut Pushad, args: *const u32) {
    let _ = r;
    count(NEAREST);
    unsafe { find_nearest(*args) };
}
regs_thunk!(nearest_thunk, nearest_entry);

// --- 0x9F4150 shops_update_proximity ---------------------------------------------------------------

unsafe fn update_proximity() {
    unsafe {
        let mut i = 0i32;
        let mut off = 0u32;
        if (rd(SHOP_COUNT) as i32) > 0 {
            loop {
                let shop = rd(SHOP_ARRAY).wrapping_add(off);
                if (rd(shop as usize + 0x2C) as i32) >= 0 {
                    hit(H_UPDATE_AUDIO);
                    call_regs(SHOP_STOP_FAR, shop, 0, 0, off, i as u32, &[]);
                }
                i = i.wrapping_add(1);
                off = off.wrapping_add(0xA0);
                if !(i < rd(SHOP_COUNT) as i32) {
                    break;
                }
            }
        }
        if rd(CUR_OTHER) != 0 && rd(MODE_21603CC) == 1 {
            wd(CUR_OTHER, 0);
        }
        let mut h = rd(HUMAN_RING);
        if h != 0 {
            loop {
                call_regs(SHOP_FIND_NEAREST, 0, 0, 0, h, i as u32, &[h]);
                h = rd(h as usize + 0x1294);
                if h == rd(HUMAN_RING) || h == 0 {
                    break;
                }
            }
        }
    }
}

extern "C" fn update_entry(_: &mut Pushad, _: *const u32) {
    count(UPDATE_PROX);
    unsafe { update_proximity() };
}
regs_thunk!(update_thunk, update_entry);

// --- 0x9F41D0 shop_triggers_update (EAX = shop) ----------------------------------------------------

const POINT_BLOCKED: usize = 0x0061_05D0; // cdecl (vec3 by value) -> AL

unsafe fn trig_act(t: u32) {
    unsafe { call_regs(TRIGGER_ACTIVATE, 0, 0, 0, t, 0, &[]) };
}
unsafe fn trig_deact(t: u32, edi: u32) {
    unsafe { call_regs(TRIGGER_DEACTIVATE, t, 0, 0, 0, edi, &[]) };
}

/// Deactivates every trigger of the shop (the last one a tail call in the original).
unsafe fn triggers_off(shop: u32) {
    unsafe {
        hit(H_TRIG_DEACT);
        let s = shop as usize;
        if rb(s + 0x24) & 4 != 0 {
            trig_deact(rd(rd(s + 4) as usize + 0x38), shop);
        }
        if rb(s + 0x24) & 8 != 0 {
            trig_deact(rd(rd(s + 8) as usize), shop);
        }
        if rd(s + 0x14) != 0 {
            let t = rd(rd(s + 0x14) as usize + 0x10);
            if t != 0 {
                trig_deact(t, shop);
            }
            let t = rd(rd(s + 0x14) as usize + 8);
            if t != 0 {
                trig_deact(t, shop);
            }
            let t = rd(rd(s + 0x14) as usize + 0xC);
            if t != 0 {
                trig_deact(t, shop);
            }
        }
        let t = rd(s + 0xC);
        if t != 0 {
            trig_deact(t, shop);
        }
    }
}

unsafe fn triggers_update(shop: u32) {
    unsafe {
        let s = shop as usize;
        let fl26 = rb(s + 0x26);
        // BL: the shop is blocked
        let blocked = if fl26 & 4 == 0
            || (rb(FLAG_D14) != 0 && rb(FLAG_CF7) != 0)
            || rb(FLAG_D15) != 0
            || (fl26 as i8) < 0
            || ui_blocks()
        {
            true
        } else {
            let a = rd(s + 4);
            if a == 0 {
                false
            } else {
                let z = rd(a as usize + 0x38);
                if z == 0 {
                    false
                } else {
                    let zz = z as usize;
                    let v = [q32(rd(zz + 0x14)), q32(rd(zz + 0x18)), q32(rd(zz + 0x1C))];
                    call_regs(POINT_BLOCKED, z, 0, 0, 0, shop, &v) as u8 != 0
                }
            }
        };
        let flag = rd(0x027B_3C60) != 0;
        let fl24 = rb(s + 0x24);
        let activate = if fl24 & 4 != 0 {
            let trig = rd(rd(s + 4) as usize + 0x10);
            let h = vcall(trig, 0x34 >> 2, 0, shop, &[]);
            !blocked
                && (call_regs(HUMAN_IS_DEAD, 0, 0, 0, 0, shop, &[h]) as u8 == 0)
                && rb(s + 0x9C) == 0
                && rb(s + 0x25) & 4 == 0
                && rb(h as usize + 0x149) == 0
        } else {
            !blocked
        };
        if !activate || ui_blocks() {
            triggers_off(shop);
            return;
        }
        hit(H_TRIG_ACT);
        if rb(s + 0x24) & 4 != 0 {
            trig_act(rd(rd(s + 4) as usize + 0x38));
        }
        if rb(s + 0x24) & 8 != 0 {
            trig_act(rd(rd(s + 8) as usize));
        }
        if rd(s + 0x14) != 0 {
            let t = rd(rd(s + 0x14) as usize + 0x10);
            if t != 0 {
                trig_act(t);
            }
            let t = rd(rd(s + 0x14) as usize + 8);
            if t != 0 {
                trig_act(t);
            }
            let t = rd(rd(s + 0x14) as usize + 0xC);
            if t != 0 {
                trig_act(t);
            }
        }
        if rb(s + 0x24) & 0x10 == 0 {
            return;
        }
        let t = rd(s + 0xC);
        if t == 0 {
            return;
        }
        if rb(s + 0x26) & 1 != 0 {
            trig_deact(t, shop);
            return;
        }
        let r = call_regs(0x004B_1720, rd(s + 0x18), 0, 0, t, shop, &[]);
        if r != 0 || flag || ui_blocks() {
            trig_deact(t, shop);
            return;
        }
        trig_act(t);
    }
}

extern "C" fn triggers_entry(r: &mut Pushad, _: *const u32) {
    count(TRIGGERS);
    unsafe { triggers_update(r.eax) };
}
regs_thunk!(triggers_thunk, triggers_entry);

// --- 0x9F4480 shop_current_event_tick --------------------------------------------------------------

const TIMER_ELAPSED: usize = 0x00BD_D230; // EAX = &timer -> AL
const HUMAN_LOCAL_OWNED: usize = 0x0080_6720; // EAX = human -> AL
const EVENT_POST_FULL: usize = 0x0055_EC40; // EAX = event, ECX = human

/// The event slot is game memory (translated in the offline test) unless the queue is full, when
/// it is a scratch event on the stack.
unsafe fn sw(full: bool, a: usize, v: u32) {
    unsafe {
        if full { *(a as *mut u32) = v } else { wd(a, v) }
    }
}
unsafe fn sw8(full: bool, a: usize, v: u8) {
    unsafe {
        if full { *(a as *mut u8) = v } else { wb(a, v) }
    }
}
unsafe fn sr8(full: bool, a: usize) -> u8 {
    unsafe {
        if full { *(a as *const u8) } else { rb(a) }
    }
}

unsafe fn event_tick() {
    unsafe {
        let p = rd(PLAYER);
        let s = rd(p as usize + 0x13BC);
        if s == 0 {
            return;
        }
        let sh = s as usize;
        if rb(sh + 0x24) & 4 == 0 || rb(sh + 0x25) & 1 == 0 {
            return;
        }
        let timer = s.wrapping_add(0x94);
        if call_regs(TIMER_ELAPSED, timer, 0, 0, p, timer, &[]) as u8 == 0 {
            return;
        }
        wd(0x0140_9EA0, rd(0x0279_7410));
        wd(0x0140_9EA8, 0);
        wd(0x0140_9EAC, 3);
        wb(0x0140_9EB0, 0);
        if call_regs(HUMAN_LOCAL_OWNED, p, 0, 0, p, timer, &[]) as u8 != 0
            && call_regs(HUMAN_IS_DEAD, 0, 0, 0, p, timer, &[p]) as u8 == 0
            && !(rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0)
            && rd(0x0252_6D9C) == 0
        {
            hit(H_TICK_POSTED);
            let k = rd(0x0140_9B0C);
            let full = (k as i32) >= 0x1E;
            let mut scratch = [0u32; 5];
            let slot: usize = if full {
                hit(H_TICK_FULL);
                scratch.as_mut_ptr() as usize
            } else {
                wd(0x0140_9B0C, k.wrapping_add(1));
                0x0279_A808 + (k as usize) * 0x14
            };
            let off = rd(0x0140_9EA4);
            let now = rd(GAME_NOW);
            let w = |a: usize, v: u32| sw(full, a, v);
            w(slot, 0x11);
            w(slot + 4, 5);
            let t = off.wrapping_add(now);
            if (off as i32) >= 0 {
                w(slot + 8, t);
                if (t as i32) > WRAP as i32 {
                    w(slot + 8, t.wrapping_add(0x94B6_2E00));
                }
            } else {
                w(slot + 8, t);
                if (t as i32) < 0 {
                    w(slot + 8, t.wrapping_add(WRAP));
                }
            }
            let b = sr8(full, slot + 0x10);
            sw8(full, slot + 0x10, (b & 0xFD) | 1);
            let ses = rd(SESSION);
            let who = if ses != 0 { rd(ses as usize + 8) } else { rd(0x0212_B160) };
            w(slot + 0xC, who);
            if full {
                call_regs(EVENT_POST_FULL, slot as u32, rd(PLAYER), 0, p, timer, &[]);
            }
        }
        hit(H_TICK_REARM);
        let t = rd(GAME_NOW).wrapping_add(0x3E8);
        wd(timer as usize, t);
        if (t as i32) > WRAP as i32 {
            wd(timer as usize, t.wrapping_add(0x94B6_2E00));
        }
    }
}

extern "C" fn event_tick_entry(_: &mut Pushad, _: *const u32) {
    count(EVENT_TICK);
    unsafe { event_tick() };
}
regs_thunk!(event_tick_thunk, event_tick_entry);

// --- 0x9E5DB0 pcu_worn_apply_part_visibility (ECX = customisation object) ------------------------

const PARTS_SHOW: usize = 0x006E_7D50; // EDI = parts object, cdecl (part)
const PARTS_HIDE: usize = 0x006E_7CC0; // EDI = parts object, cdecl (part)
const STR_HASH: usize = 0x00BD_CB30; // EAX = 0, ECX = string, stdcall (out) -> EAX = out
const PART_LIST_FIND: usize = 0x006E_3CD0; // EDX = &hash, ESI = &count -> EAX = records
const PART_LIST_APPLY: usize = 0x006E_49A0; // EAX = index or -1, cdecl (list)
const NO_NAME: usize = 0x033D_630C;

unsafe fn rd16(p: usize) -> i32 {
    unsafe { (rb(p) as u16 | (rb(p + 1) as u16) << 8) as i16 as i32 }
}

/// Marks every slot hidden by a worn item and shows / hides each model part of the human, then
/// re-applies the saved part list. The original's second flag table sits right below the first on
/// its stack and so, for a slot byte of 0x58 or more, sets bytes of the hidden-slot table; a slot
/// byte below -4 or so would clobber the original's own locals (not reproduced).
unsafe fn part_visibility(obj: u32) {
    unsafe {
        let mut a = [0u8; 256];
        let head = rd(obj as usize);
        // pass 1: the slots each worn item hides
        let mut n = head;
        while n != 0 {
            let it = rd(n as usize + 8);
            if it != 0 && rd(it as usize + 0x3C) & 1 != 0 && rb(n as usize + 4) == 0 {
                let e = rd(n as usize) as usize;
                let cnt = rd16(e + 0x1C);
                if cnt > 0 {
                    let list = rd(e + 0x28) as usize;
                    for k in 0..cnt as usize {
                        a[rb(list + k) as usize] = 1;
                    }
                }
            }
            n = rd(n as usize + 0x1C);
            if n == head {
                break;
            }
        }
        // pass 2: the worn-slot table (dead stores; the high slots land in `a`)
        let mut n = head;
        while n != 0 {
            if rb(n as usize + 4) == 0 {
                let e = rd(n as usize) as usize;
                let idx = rb(e + 0x15) as i8 as i32;
                if idx >= 0x58 {
                    a[(idx - 0x58) as usize] = 1;
                }
            }
            n = rd(n as usize + 0x1C);
            if n == head {
                break;
            }
        }
        // pass 3: show / hide the parts
        let mut n = head;
        while n != 0 {
            let it = rd(n as usize + 8);
            if it != 0 && rd(it as usize + 0x3C) & 1 != 0 {
                let list = rd(rd(n as usize) as usize + 0x2C) as usize;
                let cnt = rd(list + 4) as i32;
                let mut i = 0i32;
                while i < cnt {
                    hit(H_PARTS);
                    let l2 = rd(rd(rd(n as usize) as usize + 0x2C) as usize) as usize;
                    let part = rd(l2 + i as usize * 4);
                    let parts = rd(rd(n as usize + 8) as usize + 0x2C);
                    let f = if a.get(part as usize).copied().unwrap_or(0) != 0 { PARTS_SHOW } else { PARTS_HIDE };
                    call_regs(f, part, 0, 0, n, parts, &[part]);
                    i += 1;
                }
            }
            n = rd(n as usize + 0x1C);
            if n == rd(obj as usize) || n == 0 {
                break;
            }
        }
        // the saved part list of the human
        let h = rd(obj as usize + 0x18);
        let hh = h as usize;
        if rb(hh + 0x26C) != 1 {
            return;
        }
        let name = rd(hh + 0x258);
        let s = rd(hh + 0x264).wrapping_add(8);
        let mut found = u32::MAX;
        if name != rd(NO_NAME) && s != 0 {
            let mut tmp = 0u32;
            let mut cnt = 0u32;
            let r = call_regs(STR_HASH, 0, s, 0, 0, h, &[&mut tmp as *mut u32 as u32]);
            let base = call_regs(PART_LIST_FIND, r, s, r, &mut cnt as *mut u32 as u32, h, &[]);
            if base != 0 {
                let mut i = 0u32;
                while i < cnt {
                    if rd(base as usize + 4 + i as usize * 0x2C) == name {
                        found = i;
                        break;
                    }
                    i += 1;
                }
            }
        }
        call_regs(PART_LIST_APPLY, found, rd(hh + 0x268), 0, 0, h, &[rd(hh + 0x268)]);
        hit(H_SAVED);
        let sv = rd(hh + 0x268);
        let cnt = if sv == 0 {
            0
        } else {
            let q = rd(sv as usize + 0xC);
            if q != 0 { rd(q as usize + 0x18) } else { 0 }
        };
        // (the original reserves cnt * 4 bytes on its stack)
        if sv == 0 {
            return;
        }
        let q = rd(sv as usize + 0xC);
        if q == 0 || cnt == 0 || (cnt as i32) < 0 {
            return;
        }
        let q = q as usize;
        if (cnt as i32) < rd(q + 0x18) as i32 {
            return;
        }
        let l = rd(q + 0x20);
        if l == 0 {
            return;
        }
        let w = rd(l as usize);
        if (w as i32) < 0 {
            return;
        }
        let src = (w & 0x7FFF_FFFF).wrapping_add(rd(rd(q + 0x1C) as usize + 0x38));
        if src == 0 {
            return;
        }
        let len = rd(q + 0x18) as usize;
        let mut small = [0u32; 128];
        let mut big: Vec<u32>;
        let buf: &mut [u32] = if len <= 128 {
            &mut small[..len]
        } else {
            big = vec![0u32; len];
            &mut big[..]
        };
        std::ptr::copy_nonoverlapping(src as *const u32, buf.as_mut_ptr(), len);
        for k in 0..cnt as usize {
            let id = if k < len { buf[k] } else { 0 };
            if a.get(id as usize).copied().unwrap_or(0) != 0 {
                let l = rd(hh + 0x268);
                if l != 0 {
                    let parts = rd(l as usize + 0xC);
                    call_regs(PARTS_SHOW, id, l, 0, id, parts, &[id]);
                }
            }
        }
    }
}

extern "C" fn part_vis_entry(r: &mut Pushad, _: *const u32) {
    count(PART_VIS);
    unsafe { part_visibility(r.ecx) };
}
regs_thunk!(part_vis_thunk, part_vis_entry);

// --- 0x9F6E80 shop_ambient_audio_start / 0x9F6FC0 shop_ambient_audio_stop (stdcall (record)) ------

const AUDIO_PARAMS_INIT: usize = 0x0046_4AC0; // ESI = out (0x5C bytes), EDI = position, stdcall (id, 0, record)
const AUDIO_PLAY: usize = 0x0046_5B90; // EAX = params, ECX = out (12 bytes) -> EAX = out
const AUDIO_VOICE_STOP: usize = 0x0047_FEB0; // ECX = voice handle*
const AUDIO_SLOT_RELEASE: usize = 0x009F_4AB0; // stdcall (sound id, 0)
const SOUND_TABLE: usize = 0x00EE_B8F8; // stride 0x19C: +0 voice, +0xB4 = 0xEEB9AC id

unsafe fn audio_start(rec: u32) -> u32 {
    unsafe {
        let r = rec as usize;
        if (rd(r + 4) as i32) < 0 {
            if rd(r + 0x68) != 0 {
                wd(r + 0x68, 0);
            }
            return 0;
        }
        if (rb(FLAG_D14) != 0 && rb(FLAG_CF7) != 0) || rb(FLAG_D15) != 0 {
            hit(H_AUDIO_STOP_PATH);
            return call_regs(AUDIO_STOP, 0, 0, 0, 0, 0, &[rec]);
        }
        if rd(r + 0x68) == 0 {
            return 0;
        }
        let mut i = 0u32;
        loop {
            let slot = r + 0x38 + i as usize * 12;
            let b0 = rb(slot);
            let hi = rd(slot) >> 8;
            let have = b0 != 0xFF && hi != 0 && {
                let e = b0 as usize * 0x19C;
                hi == rd(0x00EE_B9AC + e) && rd(SOUND_TABLE + e) != u32::MAX
            };
            if !have {
                hit(H_AUDIO_VOICE);
                let id = (rb(0x021F_2A98) as u32) | (rb(0x021F_2A99) as u32) << 8;
                let mut params = [0u32; 23];
                let mut out = [0u32; 3];
                let pos = (r + 8 + i as usize * 12) as u32;
                call_regs(AUDIO_PARAMS_INIT, id, 0, 0, params.as_mut_ptr() as u32, pos, &[id, 0, rec]);
                let res = call_regs(AUDIO_PLAY, params.as_mut_ptr() as u32, out.as_mut_ptr() as u32, 0, params.as_mut_ptr() as u32, pos, &[]) as usize;
                let v0 = rd(res);
                wd(slot, v0);
                let v1 = rd(res + 4);
                wd(slot + 4, v1);
                let v2 = rd(res + 8);
                wd(slot + 8, v2);
            }
            i = i.wrapping_add(1);
            if !(i < rd(r + 0x68)) {
                break;
            }
        }
        i
    }
}

extern "C" fn audio_start_entry(r: &mut Pushad, args: *const u32) {
    count(AUDIO_START);
    r.eax = unsafe { audio_start(*args) };
}
regs_thunk!(audio_start_thunk, audio_start_entry, "ret 4");

unsafe fn audio_stop(rec: u32, eax0: u32, edx0: u32) -> u32 {
    unsafe {
        let r = rec as usize;
        let n = rd(r + 0x68);
        wd(r + 4, u32::MAX);
        let mut i = 0u32;
        if n != 0 {
            loop {
                hit(H_AUDIO_STOPPED);
                let p = (r + 0x38 + i as usize * 12) as u32;
                call_regs(AUDIO_VOICE_STOP, eax0, p, edx0, i, p, &[]);
                i = i.wrapping_add(1);
                if !(i < rd(r + 0x68)) {
                    break;
                }
            }
        }
        let sound = rd(r);
        wd(r + 0x68, 0);
        call_regs(AUDIO_SLOT_RELEASE, 0, 0, 0, i, 0, &[sound, 0])
    }
}

extern "C" fn audio_stop_entry(r: &mut Pushad, args: *const u32) {
    count(AUDIO_STOP_P);
    r.eax = unsafe { audio_stop(*args, r.eax, r.edx) };
}
regs_thunk!(audio_stop_thunk, audio_stop_entry, "ret 4");

// --- install ---------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    // (port, original body, its call/jmp offsets, thunk, shadow copy wanted)
    let list: [(usize, &[u8], &[usize], usize, bool); COUNT] = [
        (FIND_INDEX, &FIND_INDEX_BODY, &FIND_INDEX_CALLS, find_index_thunk as *const () as usize, true),
        (XFORM, &XFORM_BODY, &XFORM_CALLS, xform_thunk as *const () as usize, true),
        (ATTACH, &ATTACH_BODY, &ATTACH_CALLS, attach_thunk as *const () as usize, true),
        (STOP_FAR, &STOP_FAR_BODY, &STOP_FAR_CALLS, stop_far_thunk as *const () as usize, false),
        (RELEASE, &RELEASE_BODY, &RELEASE_CALLS, release_thunk as *const () as usize, false),
        (NEAREST, &NEAREST_BODY, &NEAREST_CALLS, nearest_thunk as *const () as usize, false),
        (UPDATE_PROX, &UPDATE_PROX_BODY, &UPDATE_PROX_CALLS, update_thunk as *const () as usize, false),
        (TRIGGERS, &TRIGGERS_BODY, &TRIGGERS_CALLS, triggers_thunk as *const () as usize, false),
        (EVENT_TICK, &EVENT_TICK_BODY, &EVENT_TICK_CALLS, event_tick_thunk as *const () as usize, false),
        (PART_VIS, &PART_VIS_BODY, &PART_VIS_CALLS, part_vis_thunk as *const () as usize, false),
        (LEAVE, &LEAVE_BODY, &LEAVE_CALLS, leave_thunk as *const () as usize, false),
        (ENTER, &ENTER_BODY, &ENTER_CALLS, enter_thunk as *const () as usize, false),
        (AUDIO_START, &AUDIO_START_BODY, &AUDIO_START_CALLS, audio_start_thunk as *const () as usize, false),
        (AUDIO_STOP_P, &AUDIO_STOP_BODY, &AUDIO_STOP_CALLS, audio_stop_thunk as *const () as usize, false),
    ];
    let mut ok = 0;
    for (id, body, calls, thunk, shadow) in list {
        let port = &PORTS[id];
        if shadow {
            // The shadow copy first: replacing patches the entry.
            match unsafe { detour::relocate(port.addr, body, calls) } {
                Ok(copy) => port.original.store(copy, Ordering::Relaxed),
                Err(e) => log(&format!("port_shops: {}: no shadow copy: {e:?}", port.name)),
            }
        }
        if replace_pinned(port.name, port.addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_shops: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| {
                let shadowed = p.original.load(Ordering::Relaxed) != 0;
                if shadowed {
                    format!("{} {c} ({} shadow mismatches)", p.name, p.mismatches.load(Ordering::Relaxed))
                } else {
                    format!("{} {c} (difftest-checked)", p.name)
                }
            })
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let hits: Vec<String> = HITS
        .iter()
        .zip(H_NAMES)
        .filter_map(|(h, n)| {
            let c = h.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{n}={c}"))
        })
        .collect();
    format!("port_shops: {} [{}]", parts.join(", "), hits.join(" "))
}

// --- original bodies (tools/body_bytes.py, via the module's generator) ------------------------------

const FIND_INDEX_BODY: [u8; 87] = [
    0x8B, 0x0D, 0xFC, 0xA4, 0x45, 0x01, 0x85, 0xC9, 0x8B, 0xC1, 0x74, 0x13, 0x8B, 0x12, 0x8B, 0xFF,
    0x39, 0x50, 0x08, 0x74, 0x0E, 0x8B, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xF1, 0x83,
    0xC8, 0xFF, 0xC3, 0x8B, 0x40, 0x0C, 0x3B, 0x05, 0x0C, 0x63, 0x3D, 0x03, 0x74, 0xF1, 0x8B, 0x15,
    0x4C, 0x7D, 0x52, 0x02, 0x85, 0xD2, 0x8B, 0xCA, 0x74, 0xE5, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x41, 0x0C, 0x74, 0x0E, 0x8B, 0x09, 0x3B, 0xCA, 0x74, 0xD4, 0x85, 0xC9, 0x75, 0xF1, 0x83,
    0xC8, 0xFF, 0xC3, 0x8B, 0x41, 0x10, 0xC3,
];
const FIND_INDEX_CALLS: [usize; 0] = [];
#[cfg(test)]
const FIND_INDEX_DISPS: [usize; 3] = [0x2, 0x28, 0x30];
const XFORM_BODY: [u8; 97] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x85, 0xF6, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x74, 0x4E, 0x85, 0xC0,
    0x7C, 0x3E, 0x0F, 0xBF, 0x4E, 0x0E, 0x3B, 0xC1, 0x7D, 0x36, 0x8B, 0x4E, 0x10, 0x8D, 0x04, 0x40,
    0xC1, 0xE0, 0x05, 0x85, 0xFF, 0x8D, 0x4C, 0x08, 0x44, 0x53, 0x8B, 0x19, 0x89, 0x1A, 0x8B, 0x59,
    0x04, 0x89, 0x5A, 0x04, 0x8B, 0x49, 0x08, 0x89, 0x4A, 0x08, 0x5B, 0x74, 0x0E, 0x8B, 0x56, 0x10,
    0x8D, 0x44, 0x10, 0x50, 0x8B, 0xCF, 0xE8, 0x15, 0xF7, 0x51, 0x00, 0x5F, 0xB0, 0x01, 0x5E, 0xC3,
    0xB9, 0x09, 0x00, 0x00, 0x00, 0xBE, 0xE4, 0x5A, 0xEC, 0x00, 0xF3, 0xA5, 0x5F, 0x32, 0xC0, 0x5E,
    0xC3,
];
const XFORM_CALLS: [usize; 1] = [0x46];
#[cfg(test)]
const XFORM_DISPS: [usize; 1] = [0x56];
const ATTACH_BODY: [u8; 417] = [
    0x81, 0xEC, 0x90, 0x00, 0x00, 0x00, 0x53, 0x8B, 0x9C, 0x24, 0x98, 0x00, 0x00, 0x00, 0x8B, 0x43,
    0x0C, 0x8B, 0x88, 0x84, 0x00, 0x00, 0x00, 0x83, 0xF9, 0xFF, 0x56, 0x57, 0x0F, 0x8E, 0x44, 0x01,
    0x00, 0x00, 0xD9, 0x40, 0x3C, 0x8D, 0x0C, 0x89, 0xD9, 0x5C, 0x24, 0x0C, 0xC1, 0xE1, 0x04, 0xD9,
    0x40, 0x40, 0x03, 0x4A, 0x4C, 0xD9, 0x5C, 0x24, 0x10, 0x8D, 0x7C, 0x24, 0x54, 0xD9, 0x40, 0x44,
    0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x40, 0x30, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x40, 0x34, 0xD9, 0x5C,
    0x24, 0x28, 0xD9, 0x40, 0x38, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x40, 0x48, 0xD9, 0x5C, 0x24, 0x30,
    0xD9, 0x40, 0x4C, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x40, 0x50, 0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x40,
    0x54, 0xD9, 0x5C, 0x24, 0x3C, 0xD9, 0x40, 0x58, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x40, 0x5C, 0xD9,
    0x5C, 0x24, 0x44, 0xD9, 0x40, 0x60, 0xD9, 0x5C, 0x24, 0x48, 0xD9, 0x40, 0x64, 0xD9, 0x5C, 0x24,
    0x4C, 0xD9, 0x40, 0x68, 0x8B, 0x41, 0x0C, 0x85, 0xC0, 0xD9, 0x5C, 0x24, 0x50, 0x74, 0x1A, 0x8B,
    0x48, 0x14, 0x8B, 0x50, 0x18, 0x89, 0x4C, 0x24, 0x18, 0x8B, 0x48, 0x1C, 0x89, 0x54, 0x24, 0x1C,
    0x89, 0x4C, 0x24, 0x20, 0x8D, 0x70, 0x20, 0xEB, 0x1A, 0x8B, 0x49, 0x40, 0x8B, 0x11, 0x8B, 0x41,
    0x04, 0x89, 0x54, 0x24, 0x18, 0x8B, 0x51, 0x08, 0x89, 0x44, 0x24, 0x1C, 0x89, 0x54, 0x24, 0x20,
    0x8D, 0x71, 0x0C, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x8D, 0x44, 0x24, 0x54, 0x8D, 0x4C,
    0x24, 0x0C, 0xE8, 0x49, 0x04, 0x2C, 0x00, 0x8D, 0x4C, 0x24, 0x24, 0xD9, 0x44, 0x24, 0x18, 0xD8,
    0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x10, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x20, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x14, 0xE8,
    0x1C, 0x04, 0x2C, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x4C, 0x24, 0x10, 0x8B, 0x54, 0x24, 0x14,
    0x89, 0x43, 0x54, 0x8B, 0x44, 0x24, 0x24, 0x89, 0x43, 0x60, 0x89, 0x4B, 0x58, 0x8B, 0x4C, 0x24,
    0x28, 0x8D, 0x44, 0x24, 0x30, 0x89, 0x53, 0x5C, 0x8B, 0x54, 0x24, 0x2C, 0x89, 0x4B, 0x64, 0x50,
    0x8D, 0x4C, 0x24, 0x58, 0x8D, 0x44, 0x24, 0x7C, 0x89, 0x53, 0x68, 0xE8, 0xC0, 0x4D, 0x2C, 0x00,
    0x8D, 0x7B, 0x6C, 0x8B, 0xF0, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x5F, 0x5E, 0x5B, 0x81,
    0xC4, 0x90, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x48, 0x3C, 0x89, 0x4B, 0x54, 0x8B, 0x50, 0x40, 0x89,
    0x53, 0x58, 0x8B, 0x48, 0x44, 0x89, 0x4B, 0x5C, 0x8B, 0x50, 0x30, 0x89, 0x53, 0x60, 0x8B, 0x48,
    0x34, 0x89, 0x4B, 0x64, 0x8B, 0x50, 0x38, 0x8D, 0x7B, 0x6C, 0x89, 0x53, 0x68, 0x8D, 0x70, 0x48,
    0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x5F, 0x5E, 0x5B, 0x81, 0xC4, 0x90, 0x00, 0x00, 0x00,
    0xC3,
];
const ATTACH_CALLS: [usize; 3] = [0xe2, 0x10f, 0x14b];
#[cfg(test)]
const ATTACH_DISPS: [usize; 0] = [];
const STOP_FAR_BODY: [u8; 173] = [
    0x83, 0xEC, 0x0C, 0x56, 0x57, 0x8B, 0xF8, 0x8B, 0x47, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x41, 0x18,
    0x8D, 0x70, 0x1C, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0xD9, 0x40, 0x14, 0xD8, 0x26, 0xD9, 0x5C, 0x24,
    0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x40, 0x18, 0xD8, 0x66,
    0x04, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x10, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x40, 0x1C, 0xD8, 0x66, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24,
    0x0C, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xE8, 0x70, 0x3E, 0xFE, 0xFF,
    0x85, 0xC0, 0x74, 0x29, 0x8D, 0x48, 0x14, 0x8B, 0xC6, 0xE8, 0xC2, 0xAC, 0x1E, 0x00, 0xD9, 0x5C,
    0x24, 0x0C, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9,
    0xF6, 0xC4, 0x41, 0x74, 0x06, 0xD9, 0x5C, 0x24, 0x10, 0xEB, 0x02, 0xDD, 0xD8, 0xD9, 0x05, 0xD4,
    0xCA, 0xE5, 0x00, 0xD8, 0x5C, 0x24, 0x10, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x09, 0x83, 0xC7,
    0x28, 0x57, 0xE8, 0x19, 0x32, 0x00, 0x00, 0x5F, 0x5E, 0x83, 0xC4, 0x0C, 0xC3,
];
const STOP_FAR_CALLS: [usize; 3] = [0x5b, 0x69, 0xa2];
#[cfg(test)]
const STOP_FAR_DISPS: [usize; 2] = [0x14, 0x8f];
const RELEASE_BODY: [u8; 332] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x83, 0xEC, 0x14, 0x85, 0xC0, 0x74, 0x23, 0x80, 0x3D, 0x56, 0x8B,
    0x52, 0x02, 0x00, 0x74, 0x0D, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x0F, 0x84, 0x26, 0x01,
    0x00, 0x00, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x19, 0x01, 0x00, 0x00, 0x57,
    0x8B, 0x7C, 0x24, 0x1C, 0x8B, 0x47, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x11, 0x8B, 0x42, 0x34, 0xFF,
    0xD0, 0x8B, 0xD0, 0x85, 0xD2, 0x0F, 0x84, 0xFC, 0x00, 0x00, 0x00, 0x39, 0x3D, 0xEC, 0x97, 0x52,
    0x02, 0x0F, 0x84, 0xF0, 0x00, 0x00, 0x00, 0x39, 0x3D, 0xF0, 0x97, 0x52, 0x02, 0x0F, 0x84, 0xE4,
    0x00, 0x00, 0x00, 0x53, 0x55, 0x56, 0x8B, 0x35, 0x3C, 0x04, 0x17, 0x02, 0x8B, 0xCE, 0x85, 0xC9,
    0x0F, 0x84, 0xB8, 0x00, 0x00, 0x00, 0x8B, 0x3D, 0x1C, 0x5B, 0x5F, 0x02, 0xD9, 0x05, 0xD4, 0xCA,
    0xE5, 0x00, 0x8B, 0x1D, 0x18, 0x5B, 0x5F, 0x02, 0x8B, 0x2D, 0x14, 0x5B, 0x5F, 0x02, 0x3B, 0x0D,
    0xD4, 0x03, 0x17, 0x02, 0x74, 0x20, 0x8B, 0x81, 0xA0, 0x12, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18,
    0x8B, 0x81, 0xA4, 0x12, 0x00, 0x00, 0x89, 0x44, 0x24, 0x1C, 0x8B, 0x81, 0xA8, 0x12, 0x00, 0x00,
    0x89, 0x44, 0x24, 0x20, 0xEB, 0x0C, 0x89, 0x6C, 0x24, 0x18, 0x89, 0x5C, 0x24, 0x1C, 0x89, 0x7C,
    0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0x62, 0x14, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24,
    0x10, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x62, 0x18, 0xD9, 0x5C,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x14,
    0xD9, 0x44, 0x24, 0x20, 0xD8, 0x62, 0x1C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDC,
    0xC8, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xD9, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x2C, 0x8B, 0x89, 0x94, 0x12, 0x00, 0x00, 0x3B, 0xCE, 0x74, 0x08,
    0x85, 0xC9, 0x0F, 0x85, 0x66, 0xFF, 0xFF, 0xFF, 0x8B, 0x7C, 0x24, 0x28, 0xDD, 0xD8, 0x8B, 0x4F,
    0x04, 0x8B, 0x49, 0x10, 0x8B, 0x11, 0x8B, 0x42, 0x1C, 0x5E, 0x5D, 0x5B, 0x5F, 0x83, 0xC4, 0x14,
    0xFF, 0xE0, 0x5E, 0xDD, 0xD8, 0x5D, 0x5B, 0x5F, 0x83, 0xC4, 0x14, 0xC3,
];
const RELEASE_CALLS: [usize; 0] = [];
#[cfg(test)]
const RELEASE_DISPS: [usize; 11] = [0x1, 0xe, 0x17, 0x4d, 0x59, 0x68, 0x78, 0x7e, 0x84, 0x8a, 0x90];
const NEAREST_BODY: [u8; 589] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x83, 0xEC, 0x20, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x2C, 0x33, 0xDB,
    0x3B, 0xC3, 0x56, 0x57, 0x0F, 0x84, 0x19, 0x01, 0x00, 0x00, 0x38, 0x1D, 0x56, 0x8B, 0x52, 0x02,
    0x74, 0x08, 0x38, 0x1D, 0x2B, 0x8B, 0x52, 0x02, 0x74, 0x0C, 0x38, 0x98, 0xF8, 0x03, 0x00, 0x00,
    0x0F, 0x85, 0xFD, 0x00, 0x00, 0x00, 0x3B, 0xEB, 0x0F, 0x84, 0x07, 0x02, 0x00, 0x00, 0xA1, 0xD4,
    0x03, 0x17, 0x02, 0x3B, 0xE8, 0x0F, 0x85, 0xFA, 0x01, 0x00, 0x00, 0x8B, 0x15, 0x14, 0x5B, 0x5F,
    0x02, 0x8B, 0x0D, 0x18, 0x5B, 0x5F, 0x02, 0x89, 0x54, 0x24, 0x24, 0x8B, 0x15, 0x1C, 0x5B, 0x5F,
    0x02, 0x89, 0x4C, 0x24, 0x28, 0x89, 0x54, 0x24, 0x2C, 0x3B, 0xEB, 0xD9, 0x05, 0x5C, 0x72, 0xE8,
    0x00, 0x8B, 0x3D, 0xEC, 0x97, 0x52, 0x02, 0xD9, 0x5C, 0x24, 0x1C, 0x89, 0x5C, 0x24, 0x14, 0x74,
    0x04, 0x3B, 0xE8, 0x74, 0x06, 0x8B, 0x3D, 0xF0, 0x97, 0x52, 0x02, 0x39, 0x1D, 0xA4, 0x2A, 0x1F,
    0x02, 0x89, 0x5C, 0x24, 0x20, 0x0F, 0x8E, 0xAA, 0x01, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0xA1, 0xA8, 0x2A, 0x1F, 0x02, 0xF6, 0x44, 0x03, 0x24, 0x04, 0x8D, 0x34, 0x03, 0x0F, 0x84, 0x22,
    0x01, 0x00, 0x00, 0x8B, 0x4E, 0x04, 0xD9, 0x44, 0x24, 0x24, 0x8B, 0x51, 0x10, 0x8B, 0x42, 0x18,
    0xD8, 0x60, 0x1C, 0x83, 0xC0, 0x1C, 0x3B, 0xF7, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10,
    0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x28, 0xD8, 0x60, 0x04, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x10, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x18, 0xD9,
    0x44, 0x24, 0x2C, 0xD8, 0x60, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDC, 0xC8,
    0xD8, 0x44, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x18, 0x0F, 0x85, 0x8A, 0x00, 0x00, 0x00, 0xD9, 0x05,
    0xD4, 0xCA, 0xE5, 0x00, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4,
    0x41, 0x75, 0x40, 0x8B, 0xC5, 0xDD, 0xD8, 0xE8, 0x84, 0xFB, 0xFF, 0xFF, 0x33, 0xFF, 0xE9, 0xA2,
    0x00, 0x00, 0x00, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x3B, 0xE8, 0x0F, 0x84, 0x0B, 0xFF, 0xFF, 0xFF,
    0x8B, 0x8D, 0xA0, 0x12, 0x00, 0x00, 0x8B, 0x95, 0xA4, 0x12, 0x00, 0x00, 0x89, 0x4C, 0x24, 0x24,
    0x8B, 0x8D, 0xA8, 0x12, 0x00, 0x00, 0x89, 0x54, 0x24, 0x28, 0x89, 0x4C, 0x24, 0x2C, 0xE9, 0x06,
    0xFF, 0xFF, 0xFF, 0x51, 0xD9, 0x1C, 0x24, 0xE8, 0xB4, 0xA9, 0xA1, 0xFF, 0xDC, 0x25, 0x30, 0xC1,
    0xE5, 0x00, 0x83, 0xC4, 0x04, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0xEE, 0xD8, 0x54, 0x24, 0x18, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x06, 0xD9, 0x5C, 0x24, 0x18, 0xEB, 0x02, 0xDD, 0xD8, 0xD9, 0x44,
    0x24, 0x18, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x18, 0x56, 0xE8, 0x12, 0xFD, 0xFF, 0xFF, 0xD9, 0x05,
    0x78, 0x07, 0xDD, 0x00, 0xD9, 0x44, 0x24, 0x1C, 0x83, 0xC4, 0x04, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD,
    0xD9, 0xF6, 0xC4, 0x41, 0x74, 0x1D, 0xF6, 0x46, 0x26, 0x80, 0x75, 0x17, 0xD9, 0x44, 0x24, 0x1C,
    0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x0A, 0xD9, 0x5C, 0x24, 0x1C, 0x89, 0x74, 0x24,
    0x14, 0xEB, 0x02, 0xDD, 0xD8, 0x8B, 0x44, 0x24, 0x20, 0x83, 0xC0, 0x01, 0x81, 0xC3, 0xA0, 0x00,
    0x00, 0x00, 0x3B, 0x05, 0xA4, 0x2A, 0x1F, 0x02, 0x89, 0x44, 0x24, 0x20, 0x0F, 0x8C, 0xAE, 0xFE,
    0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0, 0x74, 0x4B, 0x3B, 0xC7, 0x74, 0x47, 0x85, 0xFF,
    0x74, 0x09, 0x8B, 0xC5, 0x8B, 0xF7, 0xE8, 0xA5, 0xFA, 0xFF, 0xFF, 0x85, 0xED, 0x74, 0x23, 0x3B,
    0x2D, 0xD4, 0x03, 0x17, 0x02, 0x75, 0x1B, 0x8B, 0x74, 0x24, 0x14, 0x55, 0x89, 0x35, 0xEC, 0x97,
    0x52, 0x02, 0xE8, 0xD9, 0xFA, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4,
    0x20, 0xC3, 0x8B, 0x74, 0x24, 0x14, 0x55, 0x89, 0x35, 0xF0, 0x97, 0x52, 0x02, 0xE8, 0xBE, 0xFA,
    0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x20, 0xC3,
];
const NEAREST_CALLS: [usize; 6] = [0x127, 0x167, 0x199, 0x206, 0x222, 0x23d];
#[cfg(test)]
const NEAREST_DISPS: [usize; 20] = [0x1, 0x1c, 0x24, 0x3f, 0x4d, 0x53, 0x5d, 0x6d, 0x73, 0x87, 0x8d, 0xa1, 0x110, 0x134, 0x16e, 0x1a0, 0x1e4, 0x211, 0x21e, 0x239];
const UPDATE_PROX_BODY: [u8; 127] = [
    0x51, 0x56, 0x57, 0x33, 0xFF, 0x39, 0x3D, 0xA4, 0x2A, 0x1F, 0x02, 0x7E, 0x26, 0x33, 0xF6, 0x90,
    0xA1, 0xA8, 0x2A, 0x1F, 0x02, 0x03, 0xC6, 0x83, 0x78, 0x2C, 0x00, 0x7C, 0x05, 0xE8, 0x8E, 0xFB,
    0xFF, 0xFF, 0x83, 0xC7, 0x01, 0x81, 0xC6, 0xA0, 0x00, 0x00, 0x00, 0x3B, 0x3D, 0xA4, 0x2A, 0x1F,
    0x02, 0x7C, 0xDD, 0x83, 0x3D, 0xF0, 0x97, 0x52, 0x02, 0x00, 0x74, 0x13, 0x83, 0x3D, 0xCC, 0x03,
    0x16, 0x02, 0x01, 0x75, 0x0A, 0xC7, 0x05, 0xF0, 0x97, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0x8B,
    0x35, 0x3C, 0x04, 0x17, 0x02, 0x85, 0xF6, 0x74, 0x22, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x56, 0xE8, 0x4A, 0xFD, 0xFF, 0xFF, 0x8B, 0xB6, 0x94, 0x12, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x3B,
    0x35, 0x3C, 0x04, 0x17, 0x02, 0x74, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x5F, 0x5E, 0x59, 0xC3,
];
const UPDATE_PROX_CALLS: [usize; 2] = [0x1d, 0x61];
#[cfg(test)]
const UPDATE_PROX_DISPS: [usize; 8] = [0x7, 0x11, 0x2d, 0x35, 0x3e, 0x47, 0x51, 0x71];
const TRIGGERS_BODY: [u8; 459] = [
    0x83, 0xEC, 0x08, 0x53, 0x56, 0x57, 0x8B, 0xF8, 0x8A, 0x47, 0x26, 0xA8, 0x04, 0x74, 0x5C, 0x80,
    0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00, 0x75,
    0x4A, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02, 0x00, 0x75, 0x41, 0x84, 0xC0, 0x78, 0x3D, 0xE8, 0x3D,
    0x20, 0x00, 0x00, 0x84, 0xC0, 0x75, 0x34, 0x8B, 0x47, 0x04, 0x85, 0xC0, 0x74, 0x29, 0x8B, 0x40,
    0x38, 0x85, 0xC0, 0x74, 0x22, 0xD9, 0x40, 0x14, 0x83, 0xEC, 0x0C, 0x8B, 0xCC, 0xD9, 0x19, 0xD9,
    0x40, 0x18, 0xD9, 0x59, 0x04, 0xD9, 0x40, 0x1C, 0xD9, 0x59, 0x08, 0xE8, 0xA0, 0xC3, 0xC1, 0xFF,
    0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x75, 0x04, 0x32, 0xDB, 0xEB, 0x02, 0xB3, 0x01, 0x83, 0x3D, 0x60,
    0x3C, 0x7B, 0x02, 0x00, 0x0F, 0x95, 0x44, 0x24, 0x13, 0xF6, 0x47, 0x24, 0x04, 0x0F, 0x84, 0xA0,
    0x00, 0x00, 0x00, 0x8B, 0x47, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x11, 0x8B, 0x42, 0x34, 0xFF, 0xD0,
    0x84, 0xDB, 0x8B, 0xF0, 0x75, 0x23, 0x56, 0xE8, 0xC4, 0x13, 0xF9, 0xFF, 0x83, 0xC4, 0x04, 0x84,
    0xC0, 0x75, 0x16, 0x38, 0x87, 0x9C, 0x00, 0x00, 0x00, 0x75, 0x0E, 0xF6, 0x47, 0x25, 0x04, 0x75,
    0x08, 0x38, 0x86, 0x49, 0x01, 0x00, 0x00, 0x74, 0x6E, 0xF6, 0x47, 0x24, 0x04, 0x74, 0x0B, 0x8B,
    0x57, 0x04, 0x8B, 0x42, 0x38, 0xE8, 0xC6, 0x7A, 0x07, 0x00, 0xF6, 0x47, 0x24, 0x08, 0x74, 0x0A,
    0x8B, 0x47, 0x08, 0x8B, 0x00, 0xE8, 0xB6, 0x7A, 0x07, 0x00, 0x8B, 0x47, 0x14, 0x85, 0xC0, 0x74,
    0x2A, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x05, 0xE8, 0xA3, 0x7A, 0x07, 0x00, 0x8B, 0x4F, 0x14,
    0x8B, 0x41, 0x08, 0x85, 0xC0, 0x74, 0x05, 0xE8, 0x94, 0x7A, 0x07, 0x00, 0x8B, 0x57, 0x14, 0x8B,
    0x42, 0x0C, 0x85, 0xC0, 0x74, 0x05, 0xE8, 0x85, 0x7A, 0x07, 0x00, 0x8B, 0x7F, 0x0C, 0x85, 0xFF,
    0x0F, 0x84, 0xA1, 0x00, 0x00, 0x00, 0x8B, 0xC7, 0x5F, 0x5E, 0x5B, 0x83, 0xC4, 0x08, 0xE9, 0x6D,
    0x7A, 0x07, 0x00, 0x84, 0xDB, 0x75, 0x92, 0xE8, 0x44, 0x1F, 0x00, 0x00, 0x84, 0xC0, 0x75, 0x89,
    0xF6, 0x47, 0x24, 0x04, 0x74, 0x0B, 0x8B, 0x4F, 0x04, 0x8B, 0x71, 0x38, 0xE8, 0xCF, 0x79, 0x07,
    0x00, 0xF6, 0x47, 0x24, 0x08, 0x74, 0x0A, 0x8B, 0x57, 0x08, 0x8B, 0x32, 0xE8, 0xBF, 0x79, 0x07,
    0x00, 0x8B, 0x47, 0x14, 0x85, 0xC0, 0x74, 0x2C, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x07, 0x8B,
    0xF0, 0xE8, 0xAA, 0x79, 0x07, 0x00, 0x8B, 0x47, 0x14, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x05,
    0xE8, 0x9B, 0x79, 0x07, 0x00, 0x8B, 0x4F, 0x14, 0x8B, 0x71, 0x0C, 0x85, 0xF6, 0x74, 0x05, 0xE8,
    0x8C, 0x79, 0x07, 0x00, 0xF6, 0x47, 0x24, 0x10, 0x74, 0x2D, 0x8B, 0x77, 0x0C, 0x85, 0xF6, 0x74,
    0x26, 0xF6, 0x47, 0x26, 0x01, 0x75, 0x27, 0x8B, 0x47, 0x18, 0xE8, 0xB1, 0xD3, 0xAB, 0xFF, 0x85,
    0xC0, 0x75, 0x1B, 0x38, 0x44, 0x24, 0x13, 0x75, 0x15, 0xE8, 0xC2, 0x1E, 0x00, 0x00, 0x84, 0xC0,
    0x75, 0x0C, 0xE8, 0x59, 0x79, 0x07, 0x00, 0x5F, 0x5E, 0x5B, 0x83, 0xC4, 0x08, 0xC3, 0x5F, 0x8B,
    0xC6, 0x5E, 0x5B, 0x83, 0xC4, 0x08, 0xE9, 0xC5, 0x79, 0x07, 0x00,
];
const TRIGGERS_CALLS: [usize; 19] = [0x2e, 0x5b, 0x97, 0xc5, 0xd5, 0xe8, 0xf7, 0x106, 0x11e, 0x127, 0x13c, 0x14c, 0x161, 0x170, 0x17f, 0x19a, 0x1a9, 0x1b2, 0x1c6];
#[cfg(test)]
const TRIGGERS_DISPS: [usize; 4] = [0x11, 0x1a, 0x23, 0x6f];
const EVENT_TICK_BODY: [u8; 371] = [
    0x83, 0xEC, 0x14, 0x56, 0x8B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x86, 0xBC, 0x13, 0x00, 0x00,
    0x85, 0xC0, 0x57, 0x0F, 0x84, 0x54, 0x01, 0x00, 0x00, 0xF6, 0x40, 0x24, 0x04, 0x0F, 0x84, 0x4A,
    0x01, 0x00, 0x00, 0xF6, 0x40, 0x25, 0x01, 0x0F, 0x84, 0x40, 0x01, 0x00, 0x00, 0x8D, 0xB8, 0x94,
    0x00, 0x00, 0x00, 0x8B, 0xC7, 0xE8, 0x76, 0x8D, 0x1E, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0x2B, 0x01,
    0x00, 0x00, 0xA1, 0x10, 0x74, 0x79, 0x02, 0xA3, 0xA0, 0x9E, 0x40, 0x01, 0x8B, 0xC6, 0xC7, 0x05,
    0xA8, 0x9E, 0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x05, 0xAC, 0x9E, 0x40, 0x01, 0x03, 0x00,
    0x00, 0x00, 0xC6, 0x05, 0xB0, 0x9E, 0x40, 0x01, 0x00, 0xE8, 0x32, 0x22, 0xE1, 0xFF, 0x84, 0xC0,
    0x0F, 0x84, 0xDD, 0x00, 0x00, 0x00, 0x56, 0xE8, 0x34, 0x11, 0xF9, 0xFF, 0x83, 0xC4, 0x04, 0x84,
    0xC0, 0x0F, 0x85, 0xCC, 0x00, 0x00, 0x00, 0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01,
    0x75, 0x0E, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x0F, 0x84, 0xB3, 0x00, 0x00, 0x00,
    0x83, 0x3D, 0x9C, 0x6D, 0x52, 0x02, 0x00, 0x0F, 0x85, 0xA6, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0x0C,
    0x9B, 0x40, 0x01, 0x83, 0xF9, 0x1E, 0x0F, 0x9D, 0xC2, 0x84, 0xD2, 0xC7, 0x44, 0x24, 0x10, 0xFF,
    0xFF, 0xFF, 0xFF, 0x74, 0x06, 0x8D, 0x44, 0x24, 0x08, 0xEB, 0x13, 0x8D, 0x04, 0x89, 0x83, 0xC1,
    0x01, 0x8D, 0x04, 0x85, 0x08, 0xA8, 0x79, 0x02, 0x89, 0x0D, 0x0C, 0x9B, 0x40, 0x01, 0x8B, 0x0D,
    0xA4, 0x9E, 0x40, 0x01, 0x85, 0xC9, 0x8B, 0x35, 0xAC, 0x43, 0xE8, 0x00, 0xC7, 0x00, 0x11, 0x00,
    0x00, 0x00, 0xC7, 0x40, 0x04, 0x05, 0x00, 0x00, 0x00, 0x7C, 0x15, 0x03, 0xCE, 0x81, 0xF9, 0x00,
    0xD2, 0x49, 0x6B, 0x89, 0x48, 0x08, 0x7E, 0x18, 0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0xEB, 0x0D,
    0x03, 0xCE, 0x89, 0x48, 0x08, 0x79, 0x09, 0x81, 0xC1, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x48, 0x08,
    0x8A, 0x48, 0x10, 0x80, 0xE1, 0xFD, 0x80, 0xC9, 0x01, 0x88, 0x48, 0x10, 0x8B, 0x0D, 0x14, 0x8C,
    0x52, 0x02, 0x85, 0xC9, 0x74, 0x05, 0x8B, 0x49, 0x08, 0xEB, 0x06, 0x8B, 0x0D, 0x60, 0xB1, 0x12,
    0x02, 0x84, 0xD2, 0x89, 0x48, 0x0C, 0x74, 0x0B, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0xE8, 0x6D,
    0xA6, 0xB6, 0xFF, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0xE8, 0x03, 0x00, 0x00, 0x3D, 0x00, 0xD2,
    0x49, 0x6B, 0x89, 0x07, 0x7E, 0x07, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x07, 0x5F, 0x5E, 0x83,
    0xC4, 0x14, 0xC3,
];
const EVENT_TICK_CALLS: [usize; 4] = [0x35, 0x69, 0x77, 0x14e];
#[cfg(test)]
const EVENT_TICK_DISPS: [usize; 18] = [0x6, 0x43, 0x48, 0x50, 0x5a, 0x64, 0x89, 0x94, 0xa2, 0xaf, 0xd4, 0xda, 0xe0, 0xe8, 0x12e, 0x13d, 0x14a, 0x154];
const PART_VIS_BODY: [u8; 587] = [
    0x55, 0x8D, 0xAC, 0x24, 0x18, 0xFF, 0xFF, 0xFF, 0x81, 0xEC, 0x68, 0x01, 0x00, 0x00, 0xA1, 0x50,
    0x0B, 0xE8, 0x00, 0x33, 0xC5, 0x89, 0x85, 0xE4, 0x00, 0x00, 0x00, 0x53, 0x56, 0x57, 0x68, 0x00,
    0x01, 0x00, 0x00, 0x8D, 0x45, 0xE4, 0x8B, 0xF9, 0x6A, 0x00, 0x50, 0x89, 0x7D, 0x84, 0xE8, 0xDD,
    0xAD, 0x2B, 0x00, 0x8B, 0x37, 0x8B, 0xCE, 0x83, 0xC4, 0x0C, 0x85, 0xC9, 0x74, 0x42, 0x8B, 0xFF,
    0x8B, 0x41, 0x08, 0x85, 0xC0, 0x74, 0x2B, 0xF6, 0x40, 0x3C, 0x01, 0x74, 0x25, 0x80, 0x79, 0x04,
    0x00, 0x75, 0x1F, 0x8B, 0x19, 0x0F, 0xBF, 0x53, 0x1C, 0x33, 0xC0, 0x85, 0xD2, 0x7E, 0x13, 0x8B,
    0x5B, 0x28, 0x0F, 0xB6, 0x3C, 0x03, 0x83, 0xC0, 0x01, 0x3B, 0xC2, 0xC6, 0x44, 0x3D, 0xE4, 0x01,
    0x7C, 0xF0, 0x8B, 0x49, 0x1C, 0x3B, 0xCE, 0x74, 0x04, 0x85, 0xC9, 0x75, 0xC3, 0x8B, 0x7D, 0x84,
    0x8B, 0xC6, 0x85, 0xC0, 0x74, 0x1C, 0x80, 0x78, 0x04, 0x00, 0x75, 0x0B, 0x8B, 0x08, 0x0F, 0xBE,
    0x51, 0x15, 0xC6, 0x44, 0x15, 0x8C, 0x01, 0x8B, 0x40, 0x1C, 0x3B, 0xC6, 0x74, 0x04, 0x85, 0xC0,
    0x75, 0xE4, 0x85, 0xF6, 0x74, 0x61, 0x8B, 0x46, 0x08, 0x85, 0xC0, 0x74, 0x4C, 0xF6, 0x40, 0x3C,
    0x01, 0x74, 0x46, 0x8B, 0x06, 0x8B, 0x48, 0x2C, 0x8B, 0x41, 0x04, 0x33, 0xDB, 0x85, 0xC0, 0x89,
    0x45, 0x88, 0x7E, 0x35, 0x8B, 0x16, 0x8B, 0x42, 0x2C, 0x8B, 0x08, 0x8B, 0x04, 0x99, 0x80, 0x7C,
    0x05, 0xE4, 0x00, 0x50, 0x74, 0x0D, 0x8B, 0x56, 0x08, 0x8B, 0x7A, 0x2C, 0xE8, 0xBF, 0x1E, 0xD0,
    0xFF, 0xEB, 0x0B, 0x8B, 0x46, 0x08, 0x8B, 0x78, 0x2C, 0xE8, 0x22, 0x1E, 0xD0, 0xFF, 0x83, 0xC3,
    0x01, 0x83, 0xC4, 0x04, 0x3B, 0x5D, 0x88, 0x7C, 0xCB, 0x8B, 0x76, 0x1C, 0x8B, 0x7D, 0x84, 0x3B,
    0x37, 0x74, 0x04, 0x85, 0xF6, 0x75, 0x9F, 0x8B, 0x7F, 0x18, 0x80, 0xBF, 0x6C, 0x02, 0x00, 0x00,
    0x01, 0x89, 0x7D, 0x84, 0x0F, 0x85, 0x11, 0x01, 0x00, 0x00, 0x8B, 0x8F, 0x64, 0x02, 0x00, 0x00,
    0x8B, 0x9F, 0x58, 0x02, 0x00, 0x00, 0x83, 0xC1, 0x08, 0x3B, 0x1D, 0x0C, 0x63, 0x3D, 0x03, 0x74,
    0x3D, 0x85, 0xC9, 0x74, 0x39, 0x8D, 0x55, 0x80, 0x52, 0x33, 0xC0, 0xE8, 0x40, 0x6C, 0x1F, 0x00,
    0x8B, 0xD0, 0x8D, 0x75, 0x88, 0xE8, 0xD6, 0xDD, 0xCF, 0xFF, 0x85, 0xC0, 0x74, 0x20, 0x8B, 0x55,
    0x88, 0x33, 0xC9, 0x85, 0xD2, 0x76, 0x17, 0x83, 0xC0, 0x04, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x18, 0x74, 0x2A, 0x83, 0xC1, 0x01, 0x83, 0xC0, 0x2C, 0x3B, 0xCA, 0x72, 0xF2, 0x83, 0xC8,
    0xFF, 0x8B, 0x8F, 0x68, 0x02, 0x00, 0x00, 0x51, 0xE8, 0x73, 0xEA, 0xCF, 0xFF, 0x8B, 0xB7, 0x68,
    0x02, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xF6, 0x75, 0x08, 0x33, 0xDB, 0xEB, 0x10, 0x8B, 0xC1,
    0xEB, 0xDF, 0x8B, 0x46, 0x0C, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x18, 0x8B, 0xD8, 0x8D, 0x04,
    0x9D, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x66, 0xD6, 0x2C, 0x00, 0x85, 0xF6, 0x8B, 0xFC, 0x89, 0x7D,
    0x88, 0x74, 0x78, 0x8B, 0x76, 0x0C, 0x85, 0xF6, 0x74, 0x71, 0x85, 0xDB, 0x74, 0x6D, 0x8B, 0x56,
    0x18, 0x3B, 0xDA, 0x7C, 0x66, 0x8B, 0x46, 0x20, 0x85, 0xC0, 0x74, 0x5F, 0x8B, 0x00, 0x8B, 0xC8,
    0xC1, 0xE9, 0x1F, 0xF7, 0xD1, 0x83, 0xE1, 0x01, 0x74, 0x51, 0x8B, 0x4E, 0x1C, 0x25, 0xFF, 0xFF,
    0xFF, 0x7F, 0x03, 0x41, 0x38, 0x85, 0xC0, 0x74, 0x42, 0x03, 0xD2, 0x03, 0xD2, 0x52, 0x50, 0x57,
    0xE8, 0xAB, 0xA8, 0x2B, 0x00, 0x83, 0xC4, 0x0C, 0x33, 0xF6, 0x85, 0xDB, 0x7E, 0x2D, 0x8B, 0x04,
    0xB7, 0x80, 0x7C, 0x05, 0xE4, 0x00, 0x74, 0x1C, 0x8B, 0x4D, 0x84, 0x8B, 0x89, 0x68, 0x02, 0x00,
    0x00, 0x85, 0xC9, 0x74, 0x0F, 0x8B, 0x79, 0x0C, 0x50, 0xE8, 0x82, 0x1D, 0xD0, 0xFF, 0x8B, 0x7D,
    0x88, 0x83, 0xC4, 0x04, 0x83, 0xC6, 0x01, 0x3B, 0xF3, 0x7C, 0xD3, 0x8D, 0xA5, 0x74, 0xFF, 0xFF,
    0xFF, 0x5F, 0x5E, 0x5B, 0x8B, 0x8D, 0xE4, 0x00, 0x00, 0x00, 0x33, 0xCD, 0xE8, 0x5E, 0x00, 0x2B,
    0x00, 0x81, 0xC5, 0xE8, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const PART_VIS_CALLS: [usize; 10] = [0x2e, 0xdc, 0xe9, 0x13b, 0x145, 0x178, 0x1a5, 0x1f0, 0x219, 0x23c];
#[cfg(test)]
const PART_VIS_DISPS: [usize; 2] = [0xf, 0x12b];
const LEAVE_BODY: [u8; 78] = [
    0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0x53, 0x33, 0xDB, 0x3B, 0xC3, 0x74, 0x0C, 0x3B, 0xC1, 0x75,
    0x08, 0x89, 0x1D, 0xEC, 0x97, 0x52, 0x02, 0xEB, 0x06, 0x89, 0x1D, 0xF0, 0x97, 0x52, 0x02, 0x3B,
    0xC3, 0x74, 0x20, 0x3B, 0xC1, 0x75, 0x1C, 0x8B, 0x46, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x51, 0x6C,
    0x8B, 0x42, 0x08, 0x8B, 0x50, 0x68, 0x53, 0x6A, 0x01, 0x6A, 0x05, 0xE8, 0x50, 0x56, 0x08, 0x00,
    0x83, 0xC4, 0x0C, 0x88, 0x5E, 0x25, 0x88, 0x9E, 0x9C, 0x00, 0x00, 0x00, 0x5B, 0xC3,
];
const LEAVE_CALLS: [usize; 1] = [0x3b];
#[cfg(test)]
const LEAVE_DISPS: [usize; 3] = [0x2, 0x13, 0x1b];
const ENTER_BODY: [u8; 252] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x85, 0xDB, 0x57, 0x74, 0x30, 0x3B, 0x1D, 0xD4, 0x03, 0x17, 0x02,
    0x75, 0x28, 0x8B, 0x46, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x51, 0x6C, 0x8B, 0x42, 0x08, 0x8B, 0x78,
    0x68, 0x6A, 0x00, 0x6A, 0x05, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x89, 0x35, 0xEC, 0x97, 0x52, 0x02,
    0xE8, 0xEB, 0x54, 0x08, 0x00, 0x83, 0xC4, 0x08, 0xEB, 0x06, 0x89, 0x35, 0xF0, 0x97, 0x52, 0x02,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0xAC, 0x00, 0x00, 0x00, 0x80, 0x3D, 0x56,
    0x8B, 0x52, 0x02, 0x00, 0x74, 0x0D, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x0F, 0x84, 0x96,
    0x00, 0x00, 0x00, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x89, 0x00, 0x00, 0x00,
    0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00,
    0x75, 0x77, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02, 0x00, 0x75, 0x6E, 0x8B, 0x4E, 0x04, 0x8B, 0x49,
    0x10, 0x8B, 0x11, 0x8B, 0x42, 0x2C, 0xFF, 0xD0, 0x84, 0xC0, 0x75, 0x5D, 0x8B, 0x4E, 0x04, 0x8B,
    0x49, 0x10, 0x8B, 0x11, 0x8B, 0x42, 0x18, 0xFF, 0xD0, 0x8B, 0x4E, 0x04, 0x8B, 0x49, 0x10, 0x8B,
    0x11, 0x8B, 0x42, 0x34, 0x6A, 0x01, 0xFF, 0xD0, 0x8B, 0x4E, 0x04, 0x8B, 0x51, 0x14, 0x50, 0x52,
    0xE8, 0xAB, 0xE6, 0xF1, 0xFF, 0x83, 0xC4, 0x0C, 0x85, 0xDB, 0x74, 0x08, 0x3B, 0x1D, 0xD4, 0x03,
    0x17, 0x02, 0x74, 0x25, 0x8B, 0x46, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x11, 0x8B, 0x42, 0x34, 0xFF,
    0xD0, 0x8B, 0x48, 0x44, 0xE8, 0x37, 0x12, 0xE1, 0xFF, 0x85, 0xC0, 0x75, 0x0C, 0x6A, 0x05, 0x8B,
    0xF9, 0xE8, 0x0A, 0x13, 0xE1, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5B, 0xC3,
];
const ENTER_CALLS: [usize; 4] = [0x30, 0xc0, 0xe4, 0xf1];
#[cfg(test)]
const ENTER_DISPS: [usize; 10] = [0xc, 0x2c, 0x3c, 0x41, 0x4f, 0x58, 0x72, 0x7b, 0x84, 0xce];
const AUDIO_START_BODY: [u8; 223] = [
    0x83, 0xEC, 0x70, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x7C, 0x33, 0xC0, 0x39, 0x45, 0x04, 0x56, 0x57,
    0x0F, 0x8C, 0xB7, 0x00, 0x00, 0x00, 0x38, 0x05, 0x14, 0x7D, 0x52, 0x02, 0x74, 0x08, 0x38, 0x05,
    0xF7, 0x7C, 0x52, 0x02, 0x75, 0x08, 0x38, 0x05, 0x15, 0x7D, 0x52, 0x02, 0x74, 0x10, 0x55, 0xE8,
    0x0C, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x70, 0xC2, 0x04, 0x00, 0x39, 0x45,
    0x68, 0x89, 0x84, 0x24, 0x84, 0x00, 0x00, 0x00, 0x0F, 0x86, 0x87, 0x00, 0x00, 0x00, 0x8D, 0x5D,
    0x38, 0x8A, 0x03, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x0B, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6,
    0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83,
    0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x32, 0x0F, 0xB7, 0x05, 0x98, 0x2A, 0x1F, 0x02, 0x55,
    0x6A, 0x00, 0x8D, 0x7B, 0xD0, 0x50, 0x8D, 0x74, 0x24, 0x2C, 0xE8, 0xB1, 0xDB, 0xA6, 0xFF, 0x8B,
    0xC6, 0x8D, 0x4C, 0x24, 0x14, 0xE8, 0x76, 0xEC, 0xA6, 0xFF, 0x8B, 0x08, 0x89, 0x0B, 0x8B, 0x50,
    0x04, 0x89, 0x53, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x43, 0x08, 0x8B, 0x84, 0x24, 0x84, 0x00, 0x00,
    0x00, 0x83, 0xC0, 0x01, 0x83, 0xC3, 0x0C, 0x3B, 0x45, 0x68, 0x89, 0x84, 0x24, 0x84, 0x00, 0x00,
    0x00, 0x72, 0x8E, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x70, 0xC2, 0x04, 0x00, 0x39, 0x45, 0x68,
    0x76, 0x03, 0x89, 0x45, 0x68, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x70, 0xC2, 0x04, 0x00,
];
const AUDIO_START_CALLS: [usize; 3] = [0x2f, 0x8a, 0x95];
#[cfg(test)]
const AUDIO_START_DISPS: [usize; 6] = [0x18, 0x20, 0x28, 0x69, 0x71, 0x7b];
const AUDIO_STOP_BODY: [u8; 74] = [
    0x51, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x56, 0x33, 0xF6, 0x39, 0x73, 0x68, 0x57, 0xC7, 0x43, 0x04,
    0xFF, 0xFF, 0xFF, 0xFF, 0x76, 0x1C, 0x8D, 0x7B, 0x38, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0xCF, 0xE8, 0xC9, 0x8E, 0xA8, 0xFF, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x0C, 0x3B, 0x73, 0x68,
    0x72, 0xEE, 0x8B, 0x03, 0x6A, 0x00, 0x50, 0xC7, 0x43, 0x68, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xAD,
    0xDA, 0xFF, 0xFF, 0x5F, 0x5E, 0x5B, 0x59, 0xC2, 0x04, 0x00,
];
const AUDIO_STOP_CALLS: [usize; 2] = [0x22, 0x3e];
#[cfg(test)]
const AUDIO_STOP_DISPS: [usize; 0] = [];
#[cfg(test)]
const V3MUL: [u8; 111] = [
    0x83, 0xEC, 0x0C, 0xD9, 0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04, 0xD9, 0x1C, 0x24, 0xD9,
    0x41, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x40, 0x0C, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCA,
    0xD9, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9, 0x40,
    0x18, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9, 0x19, 0xD9,
    0x40, 0x10, 0xD8, 0xCA, 0xD9, 0x40, 0x04, 0xD8, 0xCC, 0xDE, 0xC1, 0xD9, 0x40, 0x1C, 0xD8, 0xCA,
    0xDE, 0xC1, 0xD9, 0x59, 0x04, 0xD9, 0x40, 0x14, 0xDE, 0xCA, 0xD9, 0x40, 0x08, 0xDE, 0xCB, 0xD9,
    0xC9, 0xDE, 0xC2, 0xD8, 0x48, 0x20, 0xDE, 0xC1, 0xD9, 0x59, 0x08, 0x83, 0xC4, 0x0C, 0xC3,
];
#[cfg(test)]
const V3MUL_CALLS: [usize; 0] = [];
#[cfg(test)]
const M33MUL: [u8; 501] = [
    0x83, 0xEC, 0x3C, 0x8B, 0x54, 0x24, 0x40, 0xD9, 0x42, 0x1C, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x41,
    0x0C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x42, 0x18, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x01, 0xD9, 0x5C,
    0x24, 0x40, 0xD9, 0x41, 0x18, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x42, 0x20, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x40, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCC, 0xDE, 0xC1,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x08, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x30, 0xD9, 0x41, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x41, 0x04, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x41, 0x1C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0xCB, 0xD9, 0x44, 0x24,
    0x10, 0xD8, 0xCD, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x34, 0xD9, 0x41, 0x14, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD9, 0x5C, 0x24, 0x40, 0xD9,
    0x41, 0x20, 0xD9, 0x1C, 0x24, 0xD9, 0x44, 0x24, 0x40, 0xD9, 0xC0, 0xDE, 0xCC, 0xD9, 0x44, 0x24,
    0x04, 0xD9, 0xC0, 0xDE, 0xCE, 0xD9, 0xCC, 0xDE, 0xC5, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCB,
    0xD9, 0xCD, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x42, 0x10, 0xD9, 0x1C, 0x24,
    0xD9, 0x42, 0x0C, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x42, 0x14, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x40, 0xD8, 0xCD, 0xD9, 0x04, 0x24, 0xD8, 0xCB, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8,
    0x4C, 0x24, 0x08, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24,
    0x0C, 0xD9, 0x04, 0x24, 0xD8, 0x4C, 0x24, 0x10, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x4C,
    0x24, 0x14, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0xC9, 0xD9, 0x04,
    0x24, 0xD8, 0xCC, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xCD, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x2C, 0xD9, 0x42, 0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x02, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x42, 0x08,
    0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCE, 0xD9, 0x04, 0x24, 0xD9,
    0xC0, 0xDE, 0xCC, 0xD9, 0xCE, 0xDE, 0xC3, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24, 0x08, 0xDE,
    0xC3, 0xD9, 0xCA, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0xC1, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x18, 0xD8,
    0x4C, 0x24, 0x0C, 0xF3, 0x0F, 0x11, 0x00, 0xD9, 0xC5, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x24, 0xD8,
    0x4C, 0x24, 0x10, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24, 0x14, 0xDE, 0xC1, 0xD9,
    0x5C, 0x24, 0x1C, 0xDE, 0xC9, 0xD9, 0xCB, 0xDE, 0xC9, 0xDE, 0xC2, 0xD8, 0x4C, 0x24, 0x40, 0xDE,
    0xC1, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x20,
    0xD9, 0x58, 0x08, 0xF3, 0x0F, 0x11, 0x40, 0x0C, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x28, 0xF3, 0x0F,
    0x11, 0x40, 0x10, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x2C, 0xF3, 0x0F, 0x11, 0x40, 0x14, 0xF3, 0x0F,
    0x10, 0x44, 0x24, 0x30, 0xF3, 0x0F, 0x11, 0x40, 0x18, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x34, 0xF3,
    0x0F, 0x11, 0x40, 0x1C, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x38, 0xF3, 0x0F, 0x11, 0x40, 0x20, 0x83,
    0xC4, 0x3C, 0xC2, 0x04, 0x00,
];
#[cfg(test)]
const M33MUL_CALLS: [usize; 0] = [];
#[cfg(test)]
pub(crate) mod difftest {
    #![allow(dead_code)]
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    pub const EAX: u8 = 1;
    pub const ECX: u8 = 2;
    pub const EDX: u8 = 4;
    pub const ESI: u8 = 8;
    pub const EDI: u8 = 16;

    /// Where a callee's result comes from.
    #[derive(Clone, Copy, PartialEq)]
    pub enum Ret {
        /// Nothing in EAX that matters (a random value comes back anyway).
        Any,
        /// A flag: 0, 1, 0x100 (AL clear but EAX not), or random.
        Bool,
        /// 0 or a member of pool `n`.
        Ptr(usize),
        /// Always 0.
        Zero,
        /// Always a member of pool `n`.
        PtrNz(usize),
        /// Returns a double on the x87 stack (and a random value in EAX).
        St0,
        /// A random value below `n`.
        Mod(u32),
        /// EAX comes back unchanged (the /GS cookie check).
        Keep,
    }

    /// What a callee does besides returning.
    #[derive(Clone, Copy, PartialEq)]
    pub enum Fx {
        None,
        /// Writes a random u32 at [ESI] (the xtbl getters).
        WriteUint,
        /// Writes a random f32 at [ESI].
        WriteFloat,
        /// Writes a random small count (0..6) at [ESI].
        WriteSmall,
        /// Logs the bytes of a buffer: `args[0]` pointer, `args[1]` length (byte 3 masked when the
        /// length is 9: the original leaves it unwritten).
        LogBuf,
        /// Logs `ECX * 3` dwords at [EDX] (a vector list).
        LogVec,
        /// Logs the three bytes at [EDX].
        LogEdx3,
        /// Logs `n` dwords at the pointer in `args[0]`.
        LogArg0(usize),
        /// Logs `n` bytes at the pointer in EAX.
        LogEaxBytes(usize),
        /// Removes one handle record from the player's pending list (args: player, object).
        RemoveListEntry,
    }

    #[derive(Clone)]
    pub struct Callee {
        pub target: usize,
        pub nargs: usize,
        pub pop: usize,
        /// Registers the callee reads on entry (the only ones compared).
        pub reads: u8,
        pub ret: Ret,
        pub fx: Fx,
        pub silent: bool,
        /// Stack arguments the callee reads as one byte (the caller leaves stale upper bits).
        pub byte_args: u32,
        /// Registers the callee reads as one byte (the caller leaves stale upper bits).
        pub byte_regs: u8,
        /// A native replacement the original's call is aimed at (0: the recording stub).
        pub native: usize,
    }

    pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
        Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, native: 0 }
    }

    /// Prints what already occupies the address space the game's globals need.
    pub fn dump_occupied() {
        use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, MEM_FREE, VirtualQuery};
        let mut a = 0x40_0000usize;
        while a < 0x400_0000 {
            let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { VirtualQuery(a as *const c_void, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                break;
            }
            if m.State != MEM_FREE {
                eprintln!("occupied {:#x}..{:#x} state {:#x} type {:#x}", m.BaseAddress as usize, m.BaseAddress as usize + m.RegionSize, m.State, m.Type);
            }
            a = m.BaseAddress as usize + m.RegionSize;
        }
    }

    pub struct Rng(pub u64);
    impl Rng {
        pub fn u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        pub fn below(&mut self, n: u32) -> u32 {
            self.u32() % n
        }
        /// True once in `n`.
        pub fn one_in(&mut self, n: u32) -> bool {
            self.below(n) == 0
        }
        /// One of `v`, or (one time in four) a random value below `modulus` (any u32 when 0).
        pub fn mix(&mut self, v: &[u32], modulus: u32) -> u32 {
            if self.below(4) == 0 {
                let r = self.u32();
                if modulus == 0 { r } else { r % modulus }
            } else {
                self.pick(v)
            }
        }
        pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[self.below(v.len() as u32) as usize]
        }
        /// A float bit pattern with the awkward cases over-represented.
        pub fn float_bits(&mut self) -> u32 {
            match self.below(12) {
                0 => 0,
                1 => 0x8000_0000,
                2 => 0x3F80_0000,
                3 => 0xBF80_0000,
                4 => 0x7FC0_0000,
                5 => 0x7F80_0001, // signalling NaN
                6 => 0x7F80_0000,
                7 => 0x0000_0001, // denormal
                8 => (self.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
                9 => (self.below(20000) as f32 / 10.0).to_bits(),
                _ => self.u32(),
            }
        }
    }

    pub fn fixed(addr: usize, len: usize) {
        let mut p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
        if p.is_null() {
            p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
        }
        assert_eq!(p as usize, addr, "cannot map {addr:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
    }

    /// The fixed regions the original touches: snapshotted, restored and compared as one.
    pub struct Mem {
        pub regions: Vec<(usize, usize)>,
    }

    impl Mem {
        /// Regions: every absolute address in `body` that falls in the game's data range (moved by
        /// `DELTA`), plus `extra` (game addresses), plus the arena; all rounded out to 64 KB.
        pub fn new(body: &[u8], extra: &[(usize, usize)]) -> Mem {
            let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::tr(a), l)).collect();
            for i in 0..body.len().saturating_sub(3) {
                let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                if (0xD0_0000..0x350_0000).contains(&v) {
                    spans.push((super::tr(v), 1));
                }
            }
            spans.push((ARENA, ARENA_LEN));
            let mut pages: Vec<usize> = Vec::new();
            for (a, l) in spans {
                let mut p = a & !0xFFFF;
                while p < a + l {
                    pages.push(p);
                    p += 0x1_0000;
                }
            }
            pages.sort_unstable();
            pages.dedup();
            let mut regions: Vec<(usize, usize)> = Vec::new();
            for p in pages {
                match regions.last_mut() {
                    Some(r) if r.0 + r.1 == p => r.1 += 0x1_0000,
                    _ => regions.push((p, 0x1_0000)),
                }
            }
            for &(a, l) in &regions {
                fixed(a, l);
            }
            Mem { regions }
        }

        pub fn fill_random(&self, rng: &mut Rng) {
            for &(a, l) in &self.regions {
                let s = unsafe { std::slice::from_raw_parts_mut(a as *mut u32, l / 4) };
                for w in s.iter_mut() {
                    *w = rng.u32();
                }
            }
        }

        /// Zero `l` bytes at the game address `a`.
        pub fn zero(&self, a: usize, l: usize) {
            unsafe { std::ptr::write_bytes(super::tr(a) as *mut u8, 0, l) };
        }

        pub fn snapshot(&self) -> Vec<u8> {
            let mut v = Vec::new();
            for &(a, l) in &self.regions {
                v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, l) });
            }
            v
        }

        pub fn restore(&self, snap: &[u8]) {
            let mut at = 0;
            for &(a, l) in &self.regions {
                unsafe { std::ptr::copy_nonoverlapping(snap[at..at + l].as_ptr(), a as *mut u8, l) };
                at += l;
            }
        }

        /// The first difference between two snapshots, as an address.
        pub fn first_diff(&self, a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
            let mut at = 0;
            for &(base, l) in &self.regions {
                for i in 0..l {
                    if a[at + i] != b[at + i] {
                        return Some((base + i, a[at + i], b[at + i]));
                    }
                }
                at += l;
            }
            None
        }

        pub fn count_diffs(&self, a: &[u8], b: &[u8]) -> usize {
            a.iter().zip(b).filter(|(x, y)| x != y).count()
        }
    }

    #[repr(C)]
    pub struct Pushad {
        edi: u32,
        esi: u32,
        ebp: u32,
        esp: u32,
        ebx: u32,
        edx: u32,
        ecx: u32,
        eax: u32,
    }

    thread_local! {
        static CALLEES: RefCell<Vec<Callee>> = const { RefCell::new(Vec::new()) };
        static STUBS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
        static POOLS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
        static LOG: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
        static SRNG: Cell<u64> = const { Cell::new(1) };
    }

    pub fn set_pools(p: Vec<Vec<u32>>) {
        POOLS.with(|x| *x.borrow_mut() = p);
    }

    /// The address a call to `f` should really go to: its stub when it has one.
    pub fn redirect(f: usize) -> usize {
        CALLEES.with(|c| {
            let c = c.borrow();
            match c.iter().position(|x| x.target == f) {
                Some(i) if c[i].native != 0 => c[i].native,
                Some(i) => STUBS.with(|s| s.borrow()[i]),
                None => f,
            }
        })
    }

    fn srng() -> u32 {
        SRNG.with(|s| {
            let mut r = Rng(s.get());
            let v = r.u32();
            s.set(r.0);
            v
        })
    }

    extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
        let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
        let r = unsafe { &*regs };
        if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("stub {:#x} eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} a0 {:#x} a1 {:#x}", c.target, r.eax, r.ecx, r.edx, r.esi, r.edi, unsafe { *args }, unsafe { *args.add(1) });
        }
        // Pointers into the stack differ between the two runs: log a marker for them.
        let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
        if !c.silent {
            let mut e = vec![c.target as u32];
            for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else { mask(v) });
            }
            for k in 0..c.nargs {
                let v = unsafe { *args.add(k) };
                e.push(if c.byte_args >> k & 1 != 0 { v & 0xFF } else { mask(v) });
            }
            match c.fx {
                Fx::LogVec => {
                    for k in 0..(r.ecx.min(40) * 3) as usize {
                        e.push(mask(unsafe { *((r.edx as usize + k * 4) as *const u32) }));
                    }
                }
                Fx::LogEdx3 => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.edx as usize + k) as *const u8) } as u32);
                    }
                }
                Fx::LogEaxBytes(n) => {
                    for k in 0..n {
                        e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
                    }
                }
                Fx::LogArg0(n) => {
                    let p = unsafe { *args } as usize;
                    for k in 0..n {
                        e.push(mask(unsafe { *((p + k * 4) as *const u32) }));
                    }
                }
                _ => {}
            }
            if c.fx == Fx::LogBuf {
                let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                assert!(len < 64, "LogBuf: callee {:#x} len {len:#x} ptr {p:#x}", c.target);
                for k in 0..len {
                    let mut b = unsafe { *((p + k) as *const u8) };
                    if (len == 9 || len == 7) && k == 3 {
                        b = 0;
                    }
                    e.push(b as u32);
                }
            }
            LOG.with(|l| l.borrow_mut().push(e));
        }
        if c.fx == Fx::RemoveListEntry {
            unsafe {
                let p = *args as usize;
                let o = *args.add(1);
                let cnt = *((p + 0xC3C) as *const u32) as usize;
                if cnt > 0 && cnt < 64 {
                    let obj_of = |h: u32| -> u32 {
                        if h == 0 {
                            return 0;
                        }
                        let e = *(super::tr(0x0214_9C64 + (((h & 0xFFF) as usize) << 4)) as *const u32);
                        if e != 0 && *((e as usize + 0x44) as *const u32) == h { e } else { 0 }
                    };
                    let hk = |k: usize| *((p + 0xC40 + k * 0x18) as *const u32);
                    let mut sel = None;
                    if o != 0 {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == o);
                    }
                    if sel.is_none() {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == 0);
                    }
                    let j = sel.unwrap_or(0);
                    for k in j..cnt - 1 {
                        std::ptr::copy((p + 0xC40 + (k + 1) * 0x18) as *const u8, (p + 0xC40 + k * 0x18) as *mut u8, 0x18);
                    }
                    *((p + 0xC3C) as *mut u32) = (cnt - 1) as u32;
                }
            }
        }
        match c.fx {
            Fx::WriteUint => {
                let v = match srng() % 6 {
                    0 => 0,
                    1 => srng() % 10,
                    2 => srng() % 100,
                    3 => srng() % 5000,
                    4 => 0x7FFF_FFFF,
                    _ => srng(),
                };
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::WriteSmall => {
                let v = srng() % 6;
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::WriteFloat => {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = rng.float_bits() };
            }
            _ => {}
        }
        match c.ret {
            Ret::Zero => 0,
            Ret::Mod(n) => srng() % n,
            Ret::Keep => r.eax,
            Ret::Any => srng(),
            Ret::St0 => {
                let v = match srng() % 8 {
                    0 => 0.0,
                    1 => f32::from_bits(srng()) as f64,
                    2 => srng() as f64 / 7.0,
                    3 => (srng() % 5000) as f64 / 10.0,
                    4 => f64::from_bits((srng() as u64) << 32 | srng() as u64),
                    5 => 1000.0,
                    _ => (srng() % 100) as f64,
                };
                unsafe { ST0V = v };
                srng()
            }
            Ret::PtrNz(n) => POOLS.with(|p| {
                let p = p.borrow();
                let pool = &p[n];
                pool[srng() as usize % pool.len()]
            }),
            Ret::Bool => match srng() % 6 {
                0 | 1 => 0,
                2 | 3 => 1,
                4 => 0x100,
                _ => srng(),
            },
            Ret::Ptr(n) => {
                if srng() % 4 == 0 {
                    0
                } else {
                    POOLS.with(|p| {
                        let p = p.borrow();
                        let pool = &p[n];
                        pool[srng() as usize % pool.len()]
                    })
                }
            }
        }
    }

    static mut ST0V: f64 = 0.0;
    pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// `stub_common` for the callees that return on ST0.
    #[unsafe(naked)]
    extern "C" fn stub_common_st0() {
        std::arch::naked_asm!(
            "pushad",
            "lea eax, [esp + 44]",
            "push eax",
            "push dword ptr [esp + 40]",
            "lea eax, [esp + 8]",
            "push eax",
            "call {f}",
            "add esp, 12",
            "mov [esp + 28], eax",
            "popad",
            "fld qword ptr [{v}]",
            "ret 4",
            f = sym stub_rs,
            v = sym ST0V,
        )
    }

    #[unsafe(naked)]
    extern "C" fn stub_common() {
        std::arch::naked_asm!(
            "pushad",
            "lea eax, [esp + 44]",
            "push eax",
            "push dword ptr [esp + 40]",
            "lea eax, [esp + 8]",
            "push eax",
            "call {f}",
            "add esp, 12",
            "mov [esp + 28], eax",
            "popad",
            "ret 4",
            f = sym stub_rs,
        )
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct Regs {
        pub eax: u32,
        pub ecx: u32,
        pub edx: u32,
        pub ebx: u32,
        pub esi: u32,
        pub edi: u32,
        pub ebp: u32,
    }

    #[repr(C)]
    struct Block {
        f: u32,
        r: Regs,
        args: [u32; 3],
        esp: u32,
        stack: u32,
    }

    /// Call `f` with all seven registers from `r` and three stack arguments, capture them after.
    pub unsafe fn call_orig(f: usize, r: &mut Regs, args: [u32; 3]) {
        let mut b = Block { f: f as u32, r: *r, args, esp: 0, stack: (STACK + STACK_LEN - 0x100) as u32 };
        unsafe {
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 44], esp",
                "mov esp, [eax + 48]",
                "push eax", "push eax", "push eax", "push eax",
                "push dword ptr [eax]",
                "push dword ptr [eax + 40]",
                "push dword ptr [eax + 36]",
                "push dword ptr [eax + 32]",
                "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
                "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
                "mov eax, [eax + 4]",
                "call dword ptr [esp + 12]",
                "push ebx",
                "mov ebx, [esp + 20]",
                "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
                "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
                "pop dword ptr [ebx + 16]",
                "mov esp, [ebx + 44]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                inout("eax") &mut b as *mut Block => _,
                lateout("ecx") _,
                lateout("edx") _,
                clobber_abi("C"),
            );
        }
        *r = b.r;
    }

    /// Reports a crash with the faulting address and where it happened (original copy offset).
    unsafe extern "system" fn veh(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
        unsafe {
            let rec = &*(*info).ExceptionRecord;
            let ctx = &*(*info).ContextRecord;
            let orig = ORIG_AT.with(|o| o.get());
            eprintln!(
                "EXCEPTION {:#x} at eip {:#x} (original +{:#x}) access {:x?} eax {:#x} ecx {:#x} edx {:#x} ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}",
                rec.ExceptionCode, ctx.Eip, (ctx.Eip as usize).wrapping_sub(orig), &rec.ExceptionInformation[..2], ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx, ctx.Esi, ctx.Edi, ctx.Ebp
            );
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
        }
        0
    }

    thread_local! {
        static ORIG_AT: Cell<usize> = const { Cell::new(0) };
    }

    pub struct Harness {
        pub mem: Mem,
        /// The original body, relocated to a private copy with its calls aimed at the stubs.
        pub orig: usize,
        pub thunk: usize,
        /// Also compare EAX after the call.
        pub cmp_eax: bool,
        /// The registers the call starts with.
        pub input: Cell<Regs>,
    }

    /// Executable memory for the stubs and the patched copy.
    pub fn exec_alloc(len: usize) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0);
        p
    }

    impl Harness {
        /// `base`: the original's address; `body` / `calls`: its bytes and the offsets of its `call
        /// rel32`s; `thunk`: the Rust replacement's entry.
        pub fn new(base: usize, body: &[u8], calls: &[usize], disps: &[usize], callees: Vec<Callee>, extra: &[(usize, usize)], thunk: usize) -> Harness {
            let mem = Mem::new(body, extra);
            fixed(STACK, STACK_LEN);
            let stubs = exec_alloc(callees.len() * 16 + 16);
            let mut addrs = Vec::new();
            for (i, c) in callees.iter().enumerate() {
                let a = stubs + i * 16;
                let mut code = [0x90u8; 16];
                code[0] = 0x68;
                code[1..5].copy_from_slice(&(i as u32).to_le_bytes());
                code[5] = 0xE8;
                let common = if c.ret == Ret::St0 { stub_common_st0 as usize } else { stub_common as usize };
                let rel = (common as i32).wrapping_sub((a + 10) as i32);
                code[6..10].copy_from_slice(&rel.to_le_bytes());
                if c.pop == 0 {
                    code[10] = 0xC3;
                } else {
                    code[10] = 0xC2;
                    code[11..13].copy_from_slice(&(c.pop as u16).to_le_bytes());
                }
                unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), a as *mut u8, 16) };
                addrs.push(a);
            }
            CALLEES.with(|c| *c.borrow_mut() = callees.clone());
            STUBS.with(|s| *s.borrow_mut() = addrs.clone());
            let orig = exec_alloc(body.len() + 16);
            unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), orig as *mut u8, body.len()) };
            for &off in disps {
                let v = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()).wrapping_add(super::DELTA as u32);
                unsafe { std::ptr::write_unaligned((orig + off) as *mut u32, v) };
            }
            for &off in calls {
                let rel = i32::from_le_bytes(body[off + 1..off + 5].try_into().unwrap());
                let target = (base + off + 5).wrapping_add(rel as usize);
                let i = callees.iter().position(|c| c.target == target).unwrap_or_else(|| panic!("call at {off:#x} to {target:#x} has no callee entry"));
                let dest = if callees[i].native != 0 { callees[i].native } else { addrs[i] };
                let r = (dest as i32).wrapping_sub((orig + off + 5) as i32);
                unsafe { std::ptr::write_unaligned((orig + off + 1) as *mut i32, r) };
            }
            ORIG_AT.with(|o| o.set(orig));
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(veh));
            });
            Harness { mem, orig, thunk, cmp_eax: false, input: Cell::new(Regs { eax: 0xAAAA_0001, ecx: 0, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 }) }
        }

        fn run(&self, f: usize, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
            SRNG.with(|s| s.set(seed | 1));
            LOG.with(|l| l.borrow_mut().clear());
            let before = self.input.get();
            let mut r = before;
            unsafe { call_orig(f, &mut r, args) };
            (LOG.with(|l| std::mem::take(&mut *l.borrow_mut())), before, r)
        }

        /// One case on the current memory: original, then Rust, from the same snapshot.
        pub fn check(&self, args: [u32; 3], seed: u64) -> Result<(), String> {
            let snap = self.mem.snapshot();
            let (la, ba, ra) = self.run(self.orig, args, seed);
            let after_a = self.mem.snapshot();
            self.mem.restore(&snap);
            let (lb, _bb, rb) = self.run(self.thunk, args, seed);
            let after_b = self.mem.snapshot();
            if self.cmp_eax && ra.eax != rb.eax {
                return Err(format!("eax: original {:#x} rust {:#x}", ra.eax, rb.eax));
            }
            // callee-saved registers must come back unchanged
            for (name, x, y, z) in [("ebx", ba.ebx, ra.ebx, rb.ebx), ("esi", ba.esi, ra.esi, rb.esi), ("edi", ba.edi, ra.edi, rb.edi), ("ebp", ba.ebp, ra.ebp, rb.ebp)] {
                if x != y || x != z {
                    return Err(format!("register {name}: in {x:#x} original {y:#x} rust {z:#x}"));
                }
            }
            for (i, (a, b)) in la.iter().zip(&lb).enumerate() {
                if a != b {
                    let prev: Vec<&Vec<u32>> = la[i.saturating_sub(3)..i].iter().collect();
                    return Err(format!("call #{i} differs:
  previous {:x?}
  original {:x?}
  rust     {:x?}", prev, a, b));
                }
            }
            if la.len() != lb.len() {
                let longer = if la.len() > lb.len() { ("original", &la) } else { ("rust", &lb) };
                return Err(format!("original made {} calls, rust {}; the {} next calls {:x?}", la.len(), lb.len(), longer.0, longer.1[la.len().min(lb.len())]));
            }
            if let Some((a, x, y)) = self.mem.first_diff(&after_a, &after_b) {
                return Err(format!("memory differs at {a:#x}: original {x:#x} rust {y:#x} ({} bytes differ)", self.mem.count_diffs(&after_a, &after_b)));
            }
            Ok(())
        }
    }

    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Where the test world's objects live.
    pub const ARENA: usize = 0x6000_0000;
    /// The tests run on a stack of their own, outside the game's address range.
    pub const STACK: usize = 0x6100_0000;
    pub const STACK_LEN: usize = 0x10_0000;
    pub const ARENA_LEN: usize = 0x18_0000;

    /// A bump allocator over the arena for one case.
    pub struct Arena(pub usize);
    impl Arena {
        pub fn new() -> Arena {
            Arena(ARENA)
        }
        /// `n` objects of `size` bytes (random contents are the caller's business).
        pub fn pool(&mut self, n: usize, size: usize) -> Vec<u32> {
            let v = (0..n).map(|i| (self.0 + i * size) as u32).collect();
            self.0 += n * size;
            assert!(self.0 <= ARENA + ARENA_LEN, "arena full");
            v
        }
    }

    pub unsafe fn put(a: u32, off: usize, v: u32) {
        unsafe { *((a as usize + off) as *mut u32) = v }
    }
    pub unsafe fn putb(a: u32, off: usize, v: u8) {
        unsafe { *((a as usize + off) as *mut u8) = v }
    }
}

// =================================================================================================
// Offline differential test
// =================================================================================================
//
// Each test runs the ORIGINAL bytes (relocated to a private copy whose absolute references are
// shifted by DELTA and whose calls go to recording stubs) and the Rust port (through its real
// thunk) on the same random world, and compares: the callee-saved registers, EAX where it is a
// result, every byte of every mapped region, and the whole stream of calls into the rest of the
// game (target, the registers the callee reads, its stack arguments).
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_shops -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
mod tests {
    use super::difftest::*;
    use super::difftest::Regs;
    use super::*;

    const V: usize = 0xCB20_0000; // virtual-function stub ids: V + slot

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    /// A float bit pattern: mostly ordinary values in +-scale, some special ones.
    fn fl(rng: &mut Rng, scale: f32) -> u32 {
        match rng.below(24) {
            0 => rng.float_bits(),
            1 => 0,
            2 | 3 => ((rng.below(21) as f32 - 10.0) * scale / 10.0).to_bits(),
            _ => ((rng.below(20001) as f32 - 10000.0) / 10000.0 * scale).to_bits(),
        }
    }

    fn g(addr: usize, v: u32) {
        unsafe { wd(addr, v) }
    }
    fn gb(addr: usize, v: u8) {
        unsafe { wb(addr, v) }
    }

    fn pick_ptr(rng: &mut Rng, pool: &[u32], z: u32) -> u32 {
        if z != 0 && rng.one_in(z) { 0 } else { pool[rng.below(pool.len() as u32) as usize] }
    }

    fn run(name: &str, h: &Harness, mut world: impl FnMut(&Harness, &mut Rng) -> [u32; 3]) {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1 ^ name.len() as u64 * 0x9E37_79B9);
        let n = cases();
        let mut bad = 0;
        for i in 0..n {
            let args = world(h, &mut rng);
            if let Err(e) = h.check(args, rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name} case {i}: regs {:x?} args {args:x?}: {e}", h.input.get());
                }
            }
        }
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }

    fn regs(eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32) -> Regs {
        Regs { eax, ecx, edx, ebx: 0xB1B1_B1B1, esi, edi, ebp: 0xE9E9_E9E9 }
    }

    // --- natives -------------------------------------------------------------------------------

    fn native_copy(b: &[u8]) -> usize {
        let p = exec_alloc(b.len());
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len()) };
        p
    }
    extern "C" fn n_memset(p: *mut u8, v: i32, n: usize) -> *mut u8 {
        unsafe { std::ptr::write_bytes(p, v as u8, n) };
        p
    }
    extern "C" fn n_memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
        unsafe { std::ptr::copy_nonoverlapping(s, d, n) };
        d
    }
    #[unsafe(naked)]
    extern "C" fn n_alloca() {
        std::arch::naked_asm!("pop edx", "sub esp, eax", "and esp, -16", "jmp edx")
    }

    fn native(target: usize, nargs: usize, pop: usize, addr: usize) -> Callee {
        let mut c = callee(target, nargs, pop, 0, Ret::Any, Fx::None);
        c.native = addr;
        c
    }

    // --- 0x6E12A0 character_table_find_index_by_hash ---------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn find_index_matches_original() {
        let _g = lock();
        let mut h = Harness::new(0x6E12A0, &FIND_INDEX_BODY, &FIND_INDEX_CALLS, &FIND_INDEX_DISPS, vec![], &[(0x0145_A000, 0x1000), (0x033D_6000, 0x1000), (0x0252_7000, 0x1000)], find_index_thunk as *const () as usize);
        h.cmp_eax = true;
        run("find_index", &h, |h, rng| unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let keys: Vec<u32> = (0..6).map(|_| rng.mix(&[1, 2, 3, 0x1234, 0xFFFF_FFFF, 0], 0)).collect();
            let vals: Vec<u32> = (0..6).map(|_| rng.mix(&[10, 11, 12, 13, 0, 0xDEAD], 0)).collect();
            let hash = a.pool(1, 16)[0];
            put(hash, 0, rng.pick(&keys));
            // list 1: key +8, value +0xC
            let n1 = rng.below(8) as usize;
            let nodes1 = a.pool(8, 0x20);
            for i in 0..8usize {
                let nx = if i + 1 < n1 { nodes1[i + 1] } else if rng.one_in(4) { 0 } else { nodes1[0] };
                put(nodes1[i], 0, nx);
                put(nodes1[i], 8, rng.pick(&keys));
                put(nodes1[i], 0xC, rng.pick(&vals));
            }
            g(0x0145_A4FC, if n1 == 0 { 0 } else { nodes1[0] });
            let n2 = rng.below(8) as usize;
            let nodes2 = a.pool(8, 0x20);
            for i in 0..8usize {
                let nx = if i + 1 < n2 { nodes2[i + 1] } else if rng.one_in(4) { 0 } else { nodes2[0] };
                put(nodes2[i], 0, nx);
                put(nodes2[i], 0xC, rng.pick(&vals));
                put(nodes2[i], 0x10, rng.u32());
            }
            g(0x0252_7D4C, if n2 == 0 { 0 } else { nodes2[0] });
            g(0x033D_630C, rng.pick(&vals));
            h.input.set(regs(rng.u32(), rng.u32(), hash, rng.u32(), rng.u32()));
            [0; 3]
        });
    }

    // --- 0x6E6070 transform_table_get_entry -------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn xform_matches_original() {
        let _g = lock();
        let callees = vec![callee(0xC057D0, 0, 0, EAX | ECX | EDX, Ret::Any, Fx::LogArg0(0))];
        let mut h = Harness::new(0x6E6070, &XFORM_BODY, &XFORM_CALLS, &XFORM_DISPS, callees, &[(0x00EC_5000, 0x1000)], xform_thunk as *const () as usize);
        h.cmp_eax = true;
        run("xform", &h, |h, rng| unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let recs = a.pool(6, 0x60);
            let table = a.pool(1, 0x40)[0];
            let n = rng.below(7) as i32;
            put(table, 0xC, ((n as u32) << 16) | rng.below(0x10000));
            put(table, 0x10, recs[0]);
            let outv = a.pool(1, 0x20)[0];
            let outm = a.pool(1, 0x40)[0];
            let idx = rng.mix(&[0, 1, 2, 3, 4, 5, 6, 0xFFFF_FFFF, 0x8000_0000], 8);
            let valid = (idx as i32) >= 0 && (idx as i32) < n;
            let t = if rng.one_in(10) { 0 } else { table };
            let m = if valid && rng.one_in(4) { 0 } else { outm };
            h.input.set(regs(idx, rng.u32(), outv, rng.u32(), rng.u32()));
            [t, m, 0]
        });
    }

    // --- 0x91E040 chunk_object_attach_world_transform ---------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn attach_matches_original() {
        let _g = lock();
        let callees = vec![native(0xBDE570, 0, 0, native_copy(&V3MUL)), native(0xBE2F50, 1, 4, native_copy(&M33MUL))];
        let h = Harness::new(0x91E040, &ATTACH_BODY, &ATTACH_CALLS, &ATTACH_DISPS, callees, &[], attach_thunk as *const () as usize);
        run("attach", &h, |h, rng| unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let table = a.pool(1, 0x80)[0];
            let recs = a.pool(5, 0x50);
            put(table, 0x4C, recs[0]);
            let ms = a.pool(5, 0x60);
            let qs = a.pool(5, 0x60);
            for i in 0..5usize {
                put(recs[i], 0xC, if rng.one_in(2) { 0 } else { ms[i] });
                put(recs[i], 0x40, qs[i]);
            }
            let part = a.pool(1, 0xC0)[0];
            let obj = a.pool(1, 0xC0)[0];
            let sc = [1.0f32, 100.0, 1e-3, 1e30];
            let scale = sc[rng.below(4) as usize];
            for off in (0x30..=0x68).step_by(4) {
                put(part, off, fl(rng, scale));
            }
            put(part, 0x84, rng.pick(&[0xFFFF_FFFF, 0, 1, 2, 3, 4, 0x8000_0000, 2, 3]));
            put(obj, 0xC, part);
            h.input.set(regs(rng.u32(), rng.u32(), table, rng.u32(), rng.u32()));
            [obj, 0, 0]
        });
    }

    // --- the shop world --------------------------------------------------------------------------

    struct W {
        shops: Vec<u32>,
        humans: Vec<u32>,
        objs: Vec<u32>,
        zz: Vec<u32>,
    }

    /// Fills the shop array, its zones / triggers, the human ring and the globals every shop
    /// function reads. `n`: shop count.
    unsafe fn shop_world(h: &Harness, rng: &mut Rng, n: usize) -> W {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let vt = a.pool(1, 0x100)[0];
            for s in [6usize, 7, 11, 13] {
                put(vt, s * 4, redirect(V + s) as u32);
            }
            let shops = a.pool(n.max(1), 0xA0);
            let zones = a.pool(n.max(1), 0x80);
            let trigs = a.pool(n.max(1), 0x100);
            let blocks = a.pool(n.max(1), 0x40);
            let xs = a.pool(n.max(1), 0x20);
            let ys = a.pool(n.max(1), 0x80);
            let zz = a.pool(n.max(1), 0x40);
            let sets = a.pool(n.max(1), 0x20);
            let cells = a.pool(n.max(1), 0x10);
            let tpool = a.pool(12, 0x120); // generic trigger-like objects
            let humans = a.pool(5, 0x1400);
            let objs = a.pool(6, 0x40);
            let sess = a.pool(1, 0x500)[0];
            let ses_pool = [0u32, sess];
            let pos_scale = rng.pick(&[10.0f32, 60.0, 400.0]);
            let range = rng.pick(&[100.0f32, 900.0, 2500.0, 40000.0]);
            g(0x00E5_CAD4, if rng.one_in(30) { rng.float_bits() } else { range.to_bits() });
            g(0x00DD_0778, if rng.one_in(30) { rng.float_bits() } else { (range * rng.pick(&[0.25f32, 1.0, 3.0])).to_bits() });
            g(0x00E8_725C, if rng.one_in(30) { rng.float_bits() } else { rng.pick(&[1e30f32, 1e10, 5000.0, 100.0]).to_bits() });
            let m = rng.pick(&[0.0f64, 2.0, 5.5, 30.0, 1e9]);
            g(0x00E5_C130, m.to_bits() as u32);
            g(0x00E5_C134, (m.to_bits() >> 32) as u32);
            for &tp in &tpool {
                put(tp, 0, vt);
            }
            for i in 0..n {
                let s = shops[i];
                put(s, 4, zones[i]);
                put(s, 8, cells[i]);
                put(cells[i], 0, pick_ptr(rng, &tpool, 6));
                put(s, 0xC, pick_ptr(rng, &tpool, 4));
                put(s, 0x14, sets[i]);
                for k in [8usize, 0xC, 0x10] {
                    put(sets[i], k, pick_ptr(rng, &tpool, 3));
                }
                if rng.one_in(6) {
                    put(s, 0x14, 0);
                }
                put(s, 0x18, rng.mix(&[0, 1, 2, 5, 0xFFFF_FFFF], 10));
                putb(s, 0x24, rng.below(256) as u8 | if rng.one_in(3) { 4 } else { 0 });
                putb(s, 0x25, rng.u32() as u8);
                putb(s, 0x26, rng.u32() as u8);
                put(s, 0x2C, rng.mix(&[0, 1, 0xFFFF_FFFF, 0x8000_0000], 0));
                putb(s, 0x9C, rng.below(2) as u8);
                put(zones[i], 0x10, trigs[i]);
                put(zones[i], 0x14, rng.u32());
                put(zones[i], 0x38, if rng.one_in(5) { 0 } else { zz[i] });
                put(trigs[i], 0, vt);
                put(trigs[i], 0x18, blocks[i]);
                put(trigs[i], 0x6C, xs[i]);
                put(xs[i], 8, ys[i]);
                put(ys[i], 0x68, rng.u32());
                for k in 0..3usize {
                    put(blocks[i], 0x1C + k * 4, fl(rng, pos_scale));
                    put(zz[i], 0x14 + k * 4, fl(rng, pos_scale));
                }
            }
            // humans: a ring, possibly open-ended
            let nh = 1 + rng.below(4) as usize;
            for i in 0..5usize {
                let nx = if i + 1 < nh { humans[i + 1] } else if rng.one_in(5) { 0 } else { humans[0] };
                put(humans[i], 0x1294, nx);
                for k in 0..3usize {
                    put(humans[i], 0x12A0 + k * 4, fl(rng, pos_scale));
                    put(humans[i], 0x14 + k * 4, fl(rng, pos_scale));
                }
                putb(humans[i], 0x149, rng.below(2) as u8);
                put(humans[i], 0x13BC, pick_ptr(rng, &shops, 3));
                put(humans[i], 0x44, rng.u32());
            }
            g(0x0217_043C, humans[0]);
            let player = match rng.below(8) {
                0 => 0,
                1 => humans[4],
                _ => humans[rng.below(nh as u32) as usize],
            };
            g(0x0217_03D4, player);
            for k in 0..3usize {
                g(0x025F_5B14 + k * 4, fl(rng, pos_scale));
            }
            let cur = |rng: &mut Rng| -> u32 { if rng.one_in(3) { 0 } else { shops[rng.below(shops.len() as u32) as usize] } };
            g(0x0252_97EC, cur(rng));
            g(0x0252_97F0, cur(rng));
            g(0x0252_8C14, ses_pool[rng.below(2) as usize]);
            put(sess, 0x3F8, rng.u32());
            put(sess, 8, rng.u32());
            gb(0x0252_8B56, rng.below(2) as u8);
            gb(0x0252_8B2B, rng.below(2) as u8);
            gb(0x0252_7D14, rng.below(2) as u8);
            gb(0x0252_7CF7, rng.below(2) as u8);
            gb(0x0252_7D15, if rng.one_in(4) { 1 } else { 0 });
            g(0x021F_2AA4, n as u32);
            g(0x021F_2AA8, shops[0]);
            g(0x0216_03CC, rng.pick(&[0, 1, 1, 2]));
            g(0x027B_3C60, rng.pick(&[0, 0, 5]));
            set_pools(vec![objs.clone(), humans.clone(), tpool.clone(), vec![0x0]]);
            for &o in &objs {
                put(o, 0, vt);
                for k in 0..3usize {
                    put(o, 0x14 + k * 4, fl(rng, pos_scale));
                }
            }
            W { shops, humans, objs, zz }
        }
    }

    const EXTRA_SHOP: [(usize, usize); 11] = [
        (0x00E8_7000, 0x1000),
        (0x027B_3000, 0x1000),
        (0x0252_7000, 0x1000),
        (0x00E5_C000, 0x2000),
        (0x00DD_0000, 0x1000),
        (0x021F_2000, 0x1000),
        (0x0252_8000, 0x2000),
        (0x0252_9000, 0x1000),
        (0x025F_5000, 0x1000),
        (0x0217_0000, 0x1000),
        (0x0216_0000, 0x1000),
    ];

    // --- 0x9F3D00 shop_stop_audio_if_player_far --------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn stop_far_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(0x9D7BD0, 0, 0, 0, Ret::Ptr(1), Fx::None),
            callee(0xBDEA30, 0, 0, EAX | ECX, Ret::St0, Fx::None),
            callee(0x9F6FC0, 1, 4, 0, Ret::Any, Fx::None),
        ];
        let h = Harness::new(0x9F3D00, &STOP_FAR_BODY, &STOP_FAR_CALLS, &STOP_FAR_DISPS, callees, &EXTRA_SHOP, stop_far_thunk as *const () as usize);
        run("stop_far", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 3);
            let shop = rng.pick(&w.shops);
            h.input.set(regs(shop, rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            // the player is a human with a position at +0x14
            if rd(0x0217_03D4) == 0 {
                g(0x0217_03D4, w.humans[0]);
            }
            [0; 3]
        });
    }

    // --- 0x9F3DB0 shop_release_if_no_human_near ---------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn release_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(V + 13, 0, 0, EAX | ECX | EDX | EDI, Ret::Ptr(0), Fx::None),
            callee(V + 7, 1, 0, EAX | ECX | EDX | ESI | EDI, Ret::Any, Fx::None),
        ];
        let h = Harness::new(0x9F3DB0, &RELEASE_BODY, &RELEASE_CALLS, &RELEASE_DISPS, callees, &EXTRA_SHOP, release_thunk as *const () as usize);
        run("release", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 3);
            let shop = rng.pick(&w.shops);
            // the cur-shop globals sometimes hold this shop
            if rng.one_in(5) {
                g(0x0252_97EC, shop);
            }
            if rng.one_in(5) {
                g(0x0252_97F0, shop);
            }
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [shop, 0, 0]
        });
    }

    // --- 0x9F3F00 shop_find_nearest_for_human -----------------------------------------------------

    fn nearest_callees() -> Vec<Callee> {
        vec![
            callee(0x9F3BB0, 0, 0, EAX | ESI | EDI, Ret::Any, Fx::None),
            callee(0x9F3DB0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x9F3C00, 1, 0, ESI | EDI, Ret::Any, Fx::None),
            callee(0x40EA20, 1, 0, 0, Ret::St0, Fx::None),
        ]
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn nearest_matches_original() {
        let _g = lock();
        let h = Harness::new(0x9F3F00, &NEAREST_BODY, &NEAREST_CALLS, &NEAREST_DISPS, nearest_callees(), &EXTRA_SHOP, nearest_thunk as *const () as usize);
        run("nearest", &h, |h, rng| unsafe {
            let n = rng.below(6) as usize;
            let w = shop_world(h, rng, n);
            // sometimes a current shop sits right where the human is
            let player = rd(0x0217_03D4);
            let human = match rng.below(6) {
                0 => w.humans[rng.below(5) as usize],
                1 | 2 => player,
                _ => w.humans[rng.below(5) as usize],
            };
            if n > 0 && rng.one_in(2) {
                let s = rng.pick(&w.shops);
                let cell = if rng.one_in(2) { 0x0252_97EC } else { 0x0252_97F0 };
                g(cell, s);
            }
            if human == 0 && player != 0 {
                return [w.humans[0], 0, 0];
            }
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [human, 0, 0]
        });
    }

    // --- 0x9F4150 shops_update_proximity ----------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn update_matches_original() {
        let _g = lock();
        let callees = vec![callee(0x9F3D00, 0, 0, EAX, Ret::Any, Fx::None), callee(0x9F3F00, 1, 0, 0, Ret::Any, Fx::None)];
        let h = Harness::new(0x9F4150, &UPDATE_PROX_BODY, &UPDATE_PROX_CALLS, &UPDATE_PROX_DISPS, callees, &EXTRA_SHOP, update_thunk as *const () as usize);
        run("update_proximity", &h, |h, rng| unsafe {
            let n = rng.below(5) as usize;
            let _w = shop_world(h, rng, n);
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [0; 3]
        });
    }

    // --- 0x9F41D0 shop_triggers_update ------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn triggers_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(0x9F6240, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x6105D0, 3, 0, 0, Ret::Bool, Fx::None),
            callee(V + 13, 0, 0, EAX | ECX | EDX, Ret::PtrNz(1), Fx::None),
            callee(0x985630, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0xA6BD60, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0xA6BCE0, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x4B1720, 0, 0, EAX, Ret::Mod(3), Fx::None),
        ];
        let h = Harness::new(0x9F41D0, &TRIGGERS_BODY, &TRIGGERS_CALLS, &TRIGGERS_DISPS, callees, &EXTRA_SHOP, triggers_thunk as *const () as usize);
        run("triggers", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 2);
            let shop = rng.pick(&w.shops);
            if rng.one_in(8) {
                put(shop, 4, 0);
                putb(shop, 0x24, rd(shop as usize + 0x24) as u8 & !4);
            }
            // make the common paths likely: no blockers, the right flag bits
            if rng.one_in(2) {
                putb(shop, 0x26, 4 | (rng.u32() as u8 & 0x3F));
                gb(0x0252_7D14, 0);
                gb(0x0252_7D15, 0);
            }
            h.input.set(regs(shop, rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [0; 3]
        });
    }

    // --- 0x9F4480 shop_current_event_tick ---------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn event_tick_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(0xBDD230, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x806720, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x985630, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x55EC40, 0, 0, EAX | ECX, Ret::Any, Fx::LogEaxBytes(16)),
        ];
        let mut extra = EXTRA_SHOP.to_vec();
        extra.extend([(0x0279_7000, 0x1000), (0x0279_A000, 0x1000), (0x0140_9000, 0x1000), (0x0212_B000, 0x1000), (0x0252_6000, 0x1000), (0x00E8_4000, 0x1000)]);
        let h = Harness::new(0x9F4480, &EVENT_TICK_BODY, &EVENT_TICK_CALLS, &EVENT_TICK_DISPS, callees, &extra, event_tick_thunk as *const () as usize);
        run("event_tick", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 3);
            let p = w.humans[0];
            g(0x0217_03D4, p);
            put(p, 0x13BC, if rng.one_in(8) { 0 } else { rng.pick(&w.shops) });
            for &s in &w.shops {
                if rng.one_in(2) {
                    putb(s, 0x24, rng.u32() as u8 | 4);
                    putb(s, 0x25, rng.u32() as u8 | 1);
                }
            }
            g(0x0140_9B0C, rng.mix(&[0, 1, 5, 0x1D, 0x1E, 0x1F, 0xFFFF_FFFF, 0xFFFF_FFFE], 0x24) % 0x28);
            g(0x0140_9EA4, rng.mix(&[0, 1, 1000, 0xFFFF_FC18, 0x8000_0000, 0x6B49_D200, 0x7FFF_FFFF, 0xF000_0000], 0));
            g(GAME_NOW, rng.mix(&[0, 100, 5000, WRAP - 3000, WRAP, WRAP + 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, WRAP - 1000], 0xFF_FFFF));
            gb(0x0252_8B48, rng.pick(&[0, 1, 1, 2]));
            gb(0x0252_8B49, rng.below(2) as u8);
            g(0x0252_6D9C, rng.pick(&[0, 0, 0, 1]));
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [0; 3]
        });
    }

    // --- 0x9F3BB0 shop_human_leave / 0x9F3C00 shop_human_enter ------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn leave_matches_original() {
        let _g = lock();
        let callees = vec![callee(0xA79240, 3, 0, EAX | ECX | EDX | ESI | EDI, Ret::Any, Fx::None)];
        let h = Harness::new(0x9F3BB0, &LEAVE_BODY, &LEAVE_CALLS, &LEAVE_DISPS, callees, &EXTRA_SHOP, leave_thunk as *const () as usize);
        run("leave", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 3);
            let shop = rng.pick(&w.shops);
            let player = rd(0x0217_03D4);
            let human = rng.pick(&[player, player, w.humans[1], w.humans[2], 0]);
            h.input.set(regs(human, rng.u32(), rng.u32(), shop, rng.u32()));
            [0; 3]
        });
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn enter_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(0xA79120, 2, 0, ECX | EDI | EAX | EDX | ESI, Ret::Any, Fx::None),
            callee(V + 11, 0, 0, EAX | ECX | EDX | ESI | EDI, Ret::Bool, Fx::None),
            callee(V + 6, 0, 0, EAX | ECX | EDX | ESI | EDI, Ret::Any, Fx::None),
            callee(V + 13, 0, 0, EAX | ECX | EDX | ESI | EDI, Ret::PtrNz(0), Fx::None),
            callee(0x912370, 2, 0, EAX | ECX | EDX | ESI | EDI, Ret::Any, Fx::None),
            callee(0x804F20, 0, 0, ECX, Ret::Mod(3), Fx::None),
            callee(0x805000, 1, 0, EDI, Ret::Any, Fx::None),
        ];
        let h = Harness::new(0x9F3C00, &ENTER_BODY, &ENTER_CALLS, &ENTER_DISPS, callees, &EXTRA_SHOP, enter_thunk as *const () as usize);
        run("enter", &h, |h, rng| unsafe {
            let w = shop_world(h, rng, 3);
            let shop = rng.pick(&w.shops);
            let player = rd(0x0217_03D4);
            let human = rng.pick(&[player, player, w.humans[1], w.humans[2], 0]);
            // the returned objects need a +0x44 handle
            for &o in &w.objs {
                put(o, 0x44, rng.u32());
            }
            if rng.one_in(2) {
                gb(0x0252_7D14, 0);
                gb(0x0252_7D15, 0);
                gb(0x0252_8B56, 0);
            }
            if rd(0x0252_8C14) != 0 {
                put(rd(0x0252_8C14), 0x3F8, rng.pick(&[0, 1, 7]));
            }
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), shop, rng.u32()));
            [human, 0, 0]
        });
    }

    // --- 0x9E5DB0 pcu_worn_apply_part_visibility --------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn part_vis_matches_original() {
        let _g = lock();
        let mut ck = callee(0xC9604F, 0, 0, 0, Ret::Zero, Fx::None);
        ck.silent = true;
        let callees = vec![
            native(0xCA0BC0, 3, 0, n_memset as *const () as usize),
            callee(0x6E7D50, 1, 0, EDI, Ret::Any, Fx::None),
            callee(0x6E7CC0, 1, 0, EDI, Ret::Any, Fx::None),
            callee(0xBDCB30, 1, 4, EAX | ECX, Ret::PtrNz(0), Fx::None),
            callee(0x6E3CD0, 0, 0, EDX | ESI, Ret::Ptr(1), Fx::WriteSmall),
            callee(0x6E49A0, 1, 0, EAX, Ret::Any, Fx::None),
            native(0xCB35C0, 0, 0, n_alloca as *const () as usize),
            native(0xCA0850, 3, 0, n_memcpy as *const () as usize),
            ck,
        ];
        let h = Harness::new(0x9E5DB0, &PART_VIS_BODY, &PART_VIS_CALLS, &PART_VIS_DISPS, callees, &[(0x00E8_0000, 0x1000), (0x033D_6000, 0x1000)], part_vis_thunk as *const () as usize);
        run("part_vis", &h, |h, rng| unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let nn = 1 + rng.below(5) as usize;
            let nodes = a.pool(5, 0x40);
            let ents = a.pool(5, 0x80);
            let items = a.pool(5, 0x40);
            let byte_lists = a.pool(5, 0x40);
            let plists = a.pool(5, 0x10);
            let plist_arrays = a.pool(5, 0x40);
            let part_arrays = a.pool(5, 0x40);
            let partobjs = a.pool(5, 0x40);
            let hashes = a.pool(1, 16);
            let recs = a.pool(1, 0x2C * 8)[0];
            set_pools(vec![hashes.clone(), vec![recs]]);
            let obj = a.pool(1, 0x40)[0];
            let human = a.pool(1, 0x300)[0];
            let sv = a.pool(1, 0x40)[0];
            let q = a.pool(1, 0x40)[0];
            let ctx = a.pool(1, 0x80)[0];
            let wptr = a.pool(1, 0x10)[0];
            let data = a.pool(1, 0x400)[0];
            let ids = |rng: &mut Rng| rng.mix(&[0, 1, 2, 3, 4, 5, 10, 20, 0x30, 0x58, 0x77, 0xFF], 0x100);
            for i in 0..5usize {
                let nx = if i + 1 < nn { nodes[i + 1] } else if rng.one_in(5) { 0 } else { nodes[0] };
                put(nodes[i], 0x1C, nx);
                put(nodes[i], 0, ents[i]);
                putb(nodes[i], 4, if rng.one_in(4) { 1 } else { 0 });
                put(nodes[i], 8, if rng.one_in(5) { 0 } else { items[i] });
                put(items[i], 0x3C, rng.u32());
                put(items[i], 0x2C, partobjs[i]);
                // the entry: slot byte, hidden-slot list, part list
                let slot = loop {
                    let s = rng.mix(&[0, 1, 2, 5, 7, 0x20, 0x57, 0x58, 0x59, 0x60, 0x7F, 0xFF, 0xFC, 0x80], 0x100) as u8;
                    if !(0xE8..=0xFB).contains(&s) {
                        break s;
                    }
                };
                putb(ents[i], 0x15, slot);
                let cnt = rng.below(5);
                put(ents[i], 0x1C, rng.below(0x10000) << 16 | cnt);
                put(ents[i], 0x28, byte_lists[i]);
                for k in 0..8usize {
                    putb(byte_lists[i], k, ids(rng) as u8);
                }
                put(ents[i], 0x2C, plists[i]);
                put(plists[i], 0, plist_arrays[i]);
                put(plists[i], 4, rng.below(6));
                for k in 0..8usize {
                    put(plist_arrays[i], k * 4, ids(rng));
                }
            }
            put(obj, 0, if rng.one_in(6) { 0 } else { nodes[0] });
            put(obj, 0x18, human);
            putb(human, 0x26C, if rng.one_in(5) { 0 } else { 1 });
            let strp = a.pool(1, 0x40)[0];
            put(human, 0x264, if rng.one_in(6) { 0xFFFF_FFF8 } else { strp.wrapping_sub(8) });
            let keys = [0x11u32, 0x22, 0x33, 0x44];
            put(human, 0x258, if rng.one_in(6) { rd(0x033D_630C) } else { rng.pick(&keys) });
            put(human, 0x268, if rng.one_in(8) { 0 } else { sv });
            for k in 0..8usize {
                put(recs, 4 + k * 0x2C, rng.pick(&keys));
            }
            put(sv, 0xC, if rng.one_in(8) { 0 } else { q });
            let cnt = rng.mix(&[0, 1, 2, 3, 6, 10], 0) % 12;
            put(q, 0x18, cnt);
            put(q, 0x1C, ctx);
            put(ctx, 0x38, data);
            put(q, 0x20, if rng.one_in(8) { 0 } else { wptr });
            put(wptr, 0, if rng.one_in(8) { 0x8000_0000 } else { 0 });
            for k in 0..12usize {
                put(data, k * 4, ids(rng));
            }
            h.input.set(regs(rng.u32(), obj, rng.u32(), rng.u32(), rng.u32()));
            [0; 3]
        });
    }

    // --- 0x9F6E80 / 0x9F6FC0 shop ambient audio -------------------------------------------------

    fn audio_world(h: &Harness, rng: &mut Rng) -> [u32; 3] {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let rec = a.pool(1, 0x80)[0];
            let outs = a.pool(4, 16);
            set_pools(vec![outs]);
            put(rec, 4, rng.mix(&[0, 1, 5, 0xFFFF_FFFF, 0x8000_0000], 0x100));
            put(rec, 0, rng.u32());
            put(rec, 0x68, rng.mix(&[0, 1, 2, 3, 4], 5));
            for i in 0..4usize {
                let b = rng.pick(&[0u32, 1, 2, 5, 0xFF, 0x7F]);
                let hi = rng.mix(&[0, 1, 0x1234, 0xFF_FFFF], 0x100_0000);
                put(rec, 0x38 + i * 12, b | hi << 8);
                put(rec, 0x38 + i * 12 + 4, rng.u32());
                put(rec, 0x38 + i * 12 + 8, rng.u32());
                // sound table entries that sometimes match the slot
                let e = b as usize * 0x19C;
                g(0x00EE_B9AC + e, if rng.one_in(2) { hi } else { rng.u32() });
                g(0x00EE_B8F8 + e, rng.mix(&[0xFFFF_FFFF, 0, 7], 0));
            }
            gb(0x0252_7D14, rng.below(2) as u8);
            gb(0x0252_7CF7, rng.below(2) as u8);
            gb(0x0252_7D15, if rng.one_in(4) { 1 } else { 0 });
            g(0x021F_2A98, rng.u32());
            h.input.set(regs(rng.u32(), rng.u32(), rng.u32(), rng.u32(), rng.u32()));
            [rec, 0, 0]
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn audio_start_matches_original() {
        let _g = lock();
        let callees = vec![
            callee(0x9F6FC0, 1, 4, 0, Ret::Any, Fx::None),
            callee(0x464AC0, 3, 12, ESI | EDI, Ret::Any, Fx::None),
            callee(0x465B90, 0, 0, EAX | ECX, Ret::PtrNz(0), Fx::None),
        ];
        let extra = [(0x00EE_B000, 0x1B000), (0x021F_2000, 0x1000), (0x0252_7000, 0x1000)];
        let h = Harness::new(0x9F6E80, &AUDIO_START_BODY, &AUDIO_START_CALLS, &AUDIO_START_DISPS, callees, &extra, audio_start_thunk as *const () as usize);
        run("audio_start", &h, audio_world);
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn audio_stop_matches_original() {
        let _g = lock();
        let callees = vec![callee(0x47FEB0, 0, 0, ECX, Ret::Any, Fx::None), callee(0x9F4AB0, 2, 8, 0, Ret::Any, Fx::None)];
        let h = Harness::new(0x9F6FC0, &AUDIO_STOP_BODY, &AUDIO_STOP_CALLS, &AUDIO_STOP_DISPS, callees, &[(0x00EE_B000, 0x1B000), (0x021F_2000, 0x1000), (0x0252_7000, 0x1000)], audio_stop_thunk as *const () as usize);
        run("audio_stop", &h, audio_world);
    }
}
