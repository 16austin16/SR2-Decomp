//! Port module: small game functions (events, diversions, predicates, queue helpers, cutscene
//! actions, ...), ported from the originals' bytes.
//!
//! Every port runs behind one `pushad` thunk that also saves XMM1-7 (an LTCG caller may keep a value
//! there across a call to a leaf that never touches them); `r.eax` carries the result. Callees stay
//! original and are called by address through `call_regs` with the registers the original sets up.
//!
//! Checks:
//! - pure: a relocated-original shadow for the first `SHADOW_CALLS` calls (EAX compared, the
//!   original's result wins on a mismatch);
//! - snap: a snapshot shadow (the small known write ranges saved, the original run, restored, the
//!   Rust version run, compared; the original's writes win on a mismatch);
//! - count: the function calls side-effecting callees, so it keeps per-function / per-branch call
//!   counters only (see `summary`).
//! - Every port is also checked offline against the original bytes (`mod tests`): random worlds, every
//!   call out recorded by a stub and compared with what the original does.

#![allow(unused_imports, dead_code, unused_variables, unused_macros)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- the offline test shifts the game's data (see `mod tests`) ------------------------------------

#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn tr(p: usize) -> usize {
    if (0xD0_0000..0x360_0000).contains(&p) { p + DELTA } else { p }
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
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(tr(p) as *const f32) }
}
#[cfg(test)]
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(tr(p) as *mut u32) = v }
}
#[cfg(test)]
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(tr(p) as *mut u8) = v }
}
#[cfg(test)]
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(tr(p) as *mut f32) = v }
}

/// Every call into the game goes through the recording stubs in the offline test.
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(tests::redirect(f), eax, ecx, edx, esi, edi, args) }
}

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
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

/// `pushad`, XMM1-7 saved, `f(&mut block, &stack args)`, XMM1-7 restored, `popad`.
macro_rules! hot_thunk {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 112",
                "movups xmmword ptr [esp], xmm1",
                "movups xmmword ptr [esp + 16], xmm2",
                "movups xmmword ptr [esp + 32], xmm3",
                "movups xmmword ptr [esp + 48], xmm4",
                "movups xmmword ptr [esp + 64], xmm5",
                "movups xmmword ptr [esp + 80], xmm6",
                "movups xmmword ptr [esp + 96], xmm7",
                "lea eax, [esp + 112]",
                "lea ecx, [esp + 148]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm1, xmmword ptr [esp]",
                "movups xmm2, xmmword ptr [esp + 16]",
                "movups xmm3, xmmword ptr [esp + 32]",
                "movups xmm4, xmmword ptr [esp + 48]",
                "movups xmm5, xmmword ptr [esp + 64]",
                "movups xmm6, xmmword ptr [esp + 80]",
                "movups xmm7, xmmword ptr [esp + 96]",
                "add esp, 112",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// Same, with XMM0 = the dword in the block's `esp` slot (a float result).
macro_rules! hot_thunk_x0 {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 112",
                "movups xmmword ptr [esp], xmm1",
                "movups xmmword ptr [esp + 16], xmm2",
                "movups xmmword ptr [esp + 32], xmm3",
                "movups xmmword ptr [esp + 48], xmm4",
                "movups xmmword ptr [esp + 64], xmm5",
                "movups xmmword ptr [esp + 80], xmm6",
                "movups xmmword ptr [esp + 96], xmm7",
                "lea eax, [esp + 112]",
                "lea ecx, [esp + 148]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm1, xmmword ptr [esp]",
                "movups xmm2, xmmword ptr [esp + 16]",
                "movups xmm3, xmmword ptr [esp + 32]",
                "movups xmm4, xmmword ptr [esp + 48]",
                "movups xmm5, xmmword ptr [esp + 64]",
                "movups xmm6, xmmword ptr [esp + 80]",
                "movups xmm7, xmmword ptr [esp + 96]",
                "movd xmm0, dword ptr [esp + 124]",
                "add esp, 112",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    calls_at: &'static [usize],
    /// 0 = counters only, 1 = pure shadow, 2 = snapshot shadow
    kind: u8,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize, body: &'static [u8], calls_at: &'static [usize], kind: u8) -> Port {
    Port { name, addr, body, calls_at, kind, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

/// Per-branch counters (4 per port).
static BR: [AtomicU32; 4 * 128] = [const { AtomicU32::new(0) }; 4 * 128];

#[inline(always)]
fn hit(id: usize, b: usize) {
    BR[id * 4 + b].fetch_add(1, Ordering::Relaxed);
}

#[inline(always)]
unsafe fn args_of<'a>(a: *const u32, n: usize) -> &'a [u32] {
    unsafe { std::slice::from_raw_parts(a, n) }
}

#[cold]
#[inline(never)]
unsafe fn pure_slow(id: usize, r: &Pushad, a: *const u32, nargs: usize, ours: u32) -> u32 {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n + 1 == SHADOW_CALLS {
        log(&format!("port_small_game: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    if orig == 0 {
        return ours;
    }
    let o = unsafe { call_regs(orig, r.eax, r.ecx, r.edx, r.esi, r.edi, args_of(a, nargs)) };
    if o == ours {
        return ours;
    }
    let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!(
            "port_small_game: {} mismatch (call {n}): regs eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x}: original {o:#x} Rust {ours:#x}",
            p.name, r.eax, r.ecx, r.edx, r.esi, r.edi
        ));
    }
    o
}

/// The result of a pure port, shadowed for the first `SHADOW_CALLS` calls.
#[inline(always)]
unsafe fn pure_done(id: usize, r: &Pushad, a: *const u32, nargs: usize, ours: u32) -> u32 {
    if PORTS[id].calls.load(Ordering::Relaxed) >= SHADOW_CALLS {
        return ours;
    }
    unsafe { pure_slow(id, r, a, nargs, ours) }
}

const SNAP_MAX: usize = 256;

#[cold]
#[inline(never)]
unsafe fn snap_slow(id: usize, r: &Pushad, a: *const u32, nargs: usize, ranges: &[(usize, usize)], rust: &mut dyn FnMut() -> u32) -> u32 {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n + 1 == SHADOW_CALLS {
        log(&format!("port_small_game: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    let total: usize = ranges.iter().map(|x| x.1).sum();
    if orig == 0 || total > SNAP_MAX {
        return rust();
    }
    let save = |buf: &mut [u8; SNAP_MAX]| {
        let mut o = 0;
        for &(ad, l) in ranges {
            unsafe { std::ptr::copy_nonoverlapping(ad as *const u8, buf.as_mut_ptr().add(o), l) };
            o += l;
        }
    };
    let restore = |buf: &[u8; SNAP_MAX]| {
        let mut o = 0;
        for &(ad, l) in ranges {
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr().add(o), ad as *mut u8, l) };
            o += l;
        }
    };
    let (mut s0, mut s1, mut s2) = ([0u8; SNAP_MAX], [0u8; SNAP_MAX], [0u8; SNAP_MAX]);
    save(&mut s0);
    let o = unsafe { call_regs(orig, r.eax, r.ecx, r.edx, r.esi, r.edi, args_of(a, nargs)) };
    save(&mut s1);
    restore(&s0);
    let ours = rust();
    save(&mut s2);
    if o == ours && s1[..total] == s2[..total] {
        return ours;
    }
    restore(&s1);
    let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!(
            "port_small_game: {} mismatch (call {n}): regs eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x}: eax original {o:#x} Rust {ours:#x}, memory {}",
            p.name, r.eax, r.ecx, r.edx, r.esi, r.edi, if s1[..total] == s2[..total] { "same" } else { "differs" }
        ));
    }
    o
}

#[inline(always)]
unsafe fn snap_done(id: usize, r: &Pushad, a: *const u32, nargs: usize, ranges: &[(usize, usize)], mut rust: impl FnMut() -> u32) -> u32 {
    if PORTS[id].calls.load(Ordering::Relaxed) >= SHADOW_CALLS {
        return rust();
    }
    unsafe { snap_slow(id, r, a, nargs, ranges, &mut rust) }
}

#[inline(always)]
unsafe fn cnt(id: usize) {
    PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
}

/// The game-event callback table: current index, and 0x6c-byte entries.
const EVT_CUR: usize = 0x00E9_6C00;
const EVT_TAB: usize = 0x00E9_6C10;

#[inline(always)]
unsafe fn evt_entry() -> usize {
    EVT_TAB.wrapping_add(unsafe { rd(EVT_CUR) }.wrapping_mul(0x6c) as usize)
}

/// The diversion handler slots (37 object pointers) and the "diversions active" flag.
const DIV_TAB: usize = 0x0140_B168;
const DIV_END: usize = 0x0140_B1FC;
const DIV_ON: usize = 0x0217_03D4;

/// Walk the handler slots; `f(ecx = object, esi = slot address, eax so far)` runs for each active
/// object and returns the new EAX.
#[inline(always)]
unsafe fn div_walk(mut eax: u32, mut f: impl FnMut(u32, u32, u32) -> u32) -> u32 {
    let mut esi = DIV_TAB as u32;
    while (esi as i32) < DIV_END as i32 {
        let ecx = unsafe { rd(esi as usize) };
        if unsafe { rb(ecx as usize + 0x14) } != 0 {
            eax = f(ecx, esi, eax);
        }
        esi = esi.wrapping_add(4);
    }
    eax
}

// ---- game_event_23 @ 0x68f9a0 (25 bytes): count ----
const GAME_EVENT_23_BODY: [u8; 25] = [
    0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x5C,
    0x85, 0xC0, 0x74, 0x04, 0x51, 0xFF, 0xD0, 0x59, 0xC3,
];
const GAME_EVENT_23_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_23_ABS: [usize; 2] = [1, 9];
#[cfg(test)]
const GAME_EVENT_23_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_23: usize = 0;
hot_thunk!(thunk_game_event_23, port_game_event_23, "ret");
unsafe extern "C" fn port_game_event_23(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_23);
        let fp = rd(evt_entry() + 0x5c);
        if fp == 0 {
            r.eax = 0;
            return;
        }
        r.eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, r.edi, &[r.ecx]);
    }
}

// ---- game_event_24 @ 0x68f9c0 (28 bytes): count ----
const GAME_EVENT_24_BODY: [u8; 28] = [
    0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x60,
    0x85, 0xC0, 0x74, 0x07, 0x51, 0x52, 0xFF, 0xD0, 0x83, 0xC4, 0x08, 0xC3,
];
const GAME_EVENT_24_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_24_ABS: [usize; 2] = [1, 9];
#[cfg(test)]
const GAME_EVENT_24_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_24: usize = 1;
hot_thunk!(thunk_game_event_24, port_game_event_24, "ret");
unsafe extern "C" fn port_game_event_24(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_24);
        let fp = rd(evt_entry() + 0x60);
        if fp == 0 {
            r.eax = 0;
            return;
        }
        r.eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, r.edi, &[r.edx, r.ecx]);
    }
}

// ---- game_event_25 @ 0x68f9e0 (28 bytes): count ----
const GAME_EVENT_25_BODY: [u8; 28] = [
    0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x64,
    0x85, 0xC0, 0x74, 0x07, 0x51, 0x52, 0xFF, 0xD0, 0x83, 0xC4, 0x08, 0xC3,
];
const GAME_EVENT_25_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_25_ABS: [usize; 2] = [1, 9];
#[cfg(test)]
const GAME_EVENT_25_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_25: usize = 2;
hot_thunk!(thunk_game_event_25, port_game_event_25, "ret");
unsafe extern "C" fn port_game_event_25(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_25);
        let fp = rd(evt_entry() + 0x64);
        if fp == 0 {
            r.eax = 0;
            return;
        }
        r.eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, r.edi, &[r.edx, r.ecx]);
    }
}

// ---- game_event_26 @ 0x68fb10 (33 bytes): count ----
const GAME_EVENT_26_BODY: [u8; 33] = [
    0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x68,
    0x85, 0xC0, 0x74, 0x0C, 0x51, 0x8B, 0x4C, 0x24, 0x08, 0x52, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x0C,
    0xC3,
];
const GAME_EVENT_26_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_26_ABS: [usize; 2] = [1, 9];
#[cfg(test)]
const GAME_EVENT_26_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_26: usize = 3;
hot_thunk!(thunk_game_event_26, port_game_event_26, "ret");
unsafe extern "C" fn port_game_event_26(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_26);
        let fp = rd(evt_entry() + 0x68);
        if fp == 0 {
            r.eax = 0;
            return;
        }
        let a0 = *a;
        r.eax = call_regs(fp as usize, fp, a0, r.edx, r.esi, r.edi, &[a0, r.edx]);
    }
}

// ---- game_event_3 @ 0x68f650 (33 bytes): count ----
const GAME_EVENT_3_BODY: [u8; 33] = [
    0x56, 0x8B, 0x35, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xF6, 0x6C, 0x81, 0xC6, 0x10, 0x6C, 0xE9, 0x00,
    0x8B, 0x46, 0x0C, 0xFF, 0xD0, 0x8B, 0x4E, 0x08, 0x6A, 0x00, 0xFF, 0xD1, 0x83, 0xC4, 0x04, 0x5E,
    0xC3,
];
const GAME_EVENT_3_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_3_ABS: [usize; 2] = [3, 12];
#[cfg(test)]
const GAME_EVENT_3_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_3: usize = 4;
hot_thunk!(thunk_game_event_3, port_game_event_3, "ret");
unsafe extern "C" fn port_game_event_3(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_3);
        let e = evt_entry() as u32;
        let fp1 = rd(e as usize + 0xc);
        let e1 = call_regs(fp1 as usize, fp1, r.ecx, r.edx, e, r.edi, &[]);
        let fp2 = rd(e as usize + 8);
        r.eax = call_regs(fp2 as usize, e1, fp2, r.edx, e, r.edi, &[0]);
    }
}

// ---- game_event_20 @ 0x68f910 (36 bytes): count ----
const GAME_EVENT_20_BODY: [u8; 36] = [
    0x57, 0x8B, 0xF8, 0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00,
    0x8B, 0x40, 0x50, 0x85, 0xC0, 0x74, 0x06, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x4E, 0x4D,
    0xF7, 0xFF, 0x5F, 0xC3,
];
const GAME_EVENT_20_CALLS: [usize; 1] = [0x1d];
#[cfg(test)]
const GAME_EVENT_20_ABS: [usize; 2] = [4, 12];
#[cfg(test)]
const GAME_EVENT_20_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_20: usize = 5;
hot_thunk!(thunk_game_event_20, port_game_event_20, "ret");
unsafe extern "C" fn port_game_event_20(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_20);
        let edi = r.eax;
        let fp = rd(evt_entry() + 0x50);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, edi, &[edi]);
        }
        r.eax = call_regs(0x604680, eax, r.ecx, r.edx, r.esi, edi, &[]);
    }
}

// ---- game_event_21 @ 0x68f940 (36 bytes): count ----
const GAME_EVENT_21_BODY: [u8; 36] = [
    0x57, 0x8B, 0xF8, 0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00,
    0x8B, 0x40, 0x54, 0x85, 0xC0, 0x74, 0x06, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x4E, 0x4D,
    0xF7, 0xFF, 0x5F, 0xC3,
];
const GAME_EVENT_21_CALLS: [usize; 1] = [0x1d];
#[cfg(test)]
const GAME_EVENT_21_ABS: [usize; 2] = [4, 12];
#[cfg(test)]
const GAME_EVENT_21_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_21: usize = 6;
hot_thunk!(thunk_game_event_21, port_game_event_21, "ret");
unsafe extern "C" fn port_game_event_21(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_21);
        let edi = r.eax;
        let fp = rd(evt_entry() + 0x54);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, edi, &[edi]);
        }
        r.eax = call_regs(0x6046b0, eax, r.ecx, r.edx, r.esi, edi, &[]);
    }
}

// ---- game_event_22 @ 0x68f970 (36 bytes): count ----
const GAME_EVENT_22_BODY: [u8; 36] = [
    0x57, 0x8B, 0xF8, 0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00,
    0x8B, 0x40, 0x58, 0x85, 0xC0, 0x74, 0x06, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x4E, 0x4D,
    0xF7, 0xFF, 0x5F, 0xC3,
];
const GAME_EVENT_22_CALLS: [usize; 1] = [0x1d];
#[cfg(test)]
const GAME_EVENT_22_ABS: [usize; 2] = [4, 12];
#[cfg(test)]
const GAME_EVENT_22_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_22: usize = 7;
hot_thunk!(thunk_game_event_22, port_game_event_22, "ret");
unsafe extern "C" fn port_game_event_22(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_22);
        let edi = r.eax;
        let fp = rd(evt_entry() + 0x58);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, edi, &[edi]);
        }
        r.eax = call_regs(0x6046e0, eax, r.ecx, r.edx, r.esi, edi, &[]);
    }
}

// ---- game_event_18 @ 0x68fae0 (36 bytes): count ----
const GAME_EVENT_18_BODY: [u8; 36] = [
    0x57, 0x8B, 0xF8, 0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00,
    0x8B, 0x40, 0x48, 0x85, 0xC0, 0x74, 0x06, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x6E, 0x4E,
    0xF7, 0xFF, 0x5F, 0xC3,
];
const GAME_EVENT_18_CALLS: [usize; 1] = [0x1d];
#[cfg(test)]
const GAME_EVENT_18_ABS: [usize; 2] = [4, 12];
#[cfg(test)]
const GAME_EVENT_18_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_18: usize = 8;
hot_thunk!(thunk_game_event_18, port_game_event_18, "ret");
unsafe extern "C" fn port_game_event_18(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_18);
        let edi = r.eax;
        let fp = rd(evt_entry() + 0x48);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, edi, &[edi]);
        }
        r.eax = call_regs(0x604970, eax, r.ecx, r.edx, r.esi, edi, &[]);
    }
}

// ---- game_event_16 @ 0x68fa00 (47 bytes): count ----
const GAME_EVENT_16_BODY: [u8; 47] = [
    0x8B, 0x0D, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC9, 0x6C, 0x81, 0xC1, 0x10, 0x6C, 0xE9, 0x00, 0x8B,
    0x49, 0x40, 0x33, 0xC0, 0x85, 0xC9, 0x74, 0x16, 0xA1, 0x00, 0x9E, 0x52, 0x02, 0x50, 0x8B, 0x44,
    0x24, 0x08, 0x68, 0x60, 0x0B, 0xF3, 0x02, 0x52, 0x50, 0xFF, 0xD1, 0x83, 0xC4, 0x10, 0xC3,
];
const GAME_EVENT_16_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_16_ABS: [usize; 4] = [2, 11, 25, 35];
#[cfg(test)]
const GAME_EVENT_16_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_16: usize = 9;
hot_thunk!(thunk_game_event_16, port_game_event_16, "ret");
unsafe extern "C" fn port_game_event_16(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_16);
        let fp = rd(evt_entry() + 0x40);
        if fp == 0 {
            r.eax = 0;
            return;
        }
        let a0 = *a;
        r.eax = call_regs(fp as usize, a0, fp, r.edx, r.esi, r.edi, &[a0, r.edx, 0x02F3_0B60, rd(0x0252_9E00)]);
    }
}

// ---- game_event_player_joined @ 0x68f680 (43 bytes): count ----
const GAME_EVENT_PLAYER_JOINED_BODY: [u8; 43] = [
    0x80, 0xBE, 0x68, 0x13, 0x00, 0x00, 0x00, 0x75, 0x21, 0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x6B, 0xC0,
    0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x20, 0x85, 0xC0, 0x74, 0x06, 0x56, 0xFF, 0xD0,
    0x83, 0xC4, 0x04, 0xC6, 0x86, 0x68, 0x13, 0x00, 0x00, 0x01, 0xC3,
];
const GAME_EVENT_PLAYER_JOINED_CALLS: [usize; 0] = [];
#[cfg(test)]
const GAME_EVENT_PLAYER_JOINED_ABS: [usize; 2] = [10, 18];
#[cfg(test)]
const GAME_EVENT_PLAYER_JOINED_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_PLAYER_JOINED: usize = 10;
hot_thunk!(thunk_game_event_player_joined, port_game_event_player_joined, "ret");
unsafe extern "C" fn port_game_event_player_joined(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_PLAYER_JOINED);
        let p = r.esi as usize;
        if rb(p + 0x1368) != 0 {
            return;
        }
        let fp = rd(evt_entry() + 0x20);
        if fp != 0 {
            r.eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, r.edi, &[r.esi]);
        } else {
            r.eax = 0;
        }
        wb(p + 0x1368, 1);
    }
}

// ---- game_event_player_left @ 0x68f6b0 (52 bytes): count ----
const GAME_EVENT_PLAYER_LEFT_BODY: [u8; 52] = [
    0x57, 0x8B, 0xF8, 0x80, 0xBF, 0x68, 0x13, 0x00, 0x00, 0x00, 0x74, 0x26, 0xA1, 0x00, 0x6C, 0xE9,
    0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x24, 0x85, 0xC0, 0x74, 0x06,
    0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x65, 0x52, 0xF7, 0xFF, 0xC6, 0x87, 0x68, 0x13, 0x00,
    0x00, 0x00, 0x5F, 0xC3,
];
const GAME_EVENT_PLAYER_LEFT_CALLS: [usize; 1] = [0x26];
#[cfg(test)]
const GAME_EVENT_PLAYER_LEFT_ABS: [usize; 2] = [13, 21];
#[cfg(test)]
const GAME_EVENT_PLAYER_LEFT_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_PLAYER_LEFT: usize = 11;
hot_thunk!(thunk_game_event_player_left, port_game_event_player_left, "ret");
unsafe extern "C" fn port_game_event_player_left(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_PLAYER_LEFT);
        let edi = r.eax;
        if rb(edi as usize + 0x1368) == 0 {
            return;
        }
        let fp = rd(evt_entry() + 0x24);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, r.ecx, r.edx, r.esi, edi, &[edi]);
        }
        r.eax = call_regs(0x0060_4940, eax, r.ecx, r.edx, r.esi, edi, &[]);
        wb(edi as usize + 0x1368, 0);
    }
}

// ---- diversions_on_game_event_18 @ 0x604970 (38 bytes): count ----
const DIVERSIONS_ON_GAME_EVENT_18_BODY: [u8; 38] = [
    0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x0B, 0x8B, 0x01,
    0x8B, 0x90, 0x84, 0x00, 0x00, 0x00, 0x57, 0xFF, 0xD2, 0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1,
    0x40, 0x01, 0x7C, 0xE2, 0x5E, 0xC3,
];
const DIVERSIONS_ON_GAME_EVENT_18_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_18_ABS: [usize; 2] = [2, 30];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_18_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_GAME_EVENT_18: usize = 12;
hot_thunk!(thunk_diversions_on_game_event_18, port_diversions_on_game_event_18, "ret");
unsafe extern "C" fn port_diversions_on_game_event_18(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_GAME_EVENT_18);
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x84);
            call_regs(f as usize, vt, ecx, f, esi, r.edi, &[r.edi])
        });
    }
}

// ---- diversions_on_game_event_20 @ 0x604680 (45 bytes): count ----
const DIVERSIONS_ON_GAME_EVENT_20_BODY: [u8; 45] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x74, 0x23, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x90,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x6C, 0x57, 0xFF, 0xD2,
    0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01, 0x7C, 0xE5, 0x5E, 0xC3,
];
const DIVERSIONS_ON_GAME_EVENT_20_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_20_ABS: [usize; 3] = [2, 11, 37];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_20_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_GAME_EVENT_20: usize = 13;
hot_thunk!(thunk_diversions_on_game_event_20, port_diversions_on_game_event_20, "ret");
unsafe extern "C" fn port_diversions_on_game_event_20(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_GAME_EVENT_20);
        if rd(DIV_ON) == 0 {
            return;
        }
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x6c);
            call_regs(f as usize, vt, ecx, f, esi, r.edi, &[r.edi])
        });
    }
}

// ---- diversions_on_game_event_21 @ 0x6046b0 (45 bytes): count ----
const DIVERSIONS_ON_GAME_EVENT_21_BODY: [u8; 45] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x74, 0x23, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x90,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x70, 0x57, 0xFF, 0xD2,
    0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01, 0x7C, 0xE5, 0x5E, 0xC3,
];
const DIVERSIONS_ON_GAME_EVENT_21_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_21_ABS: [usize; 3] = [2, 11, 37];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_21_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_GAME_EVENT_21: usize = 14;
hot_thunk!(thunk_diversions_on_game_event_21, port_diversions_on_game_event_21, "ret");
unsafe extern "C" fn port_diversions_on_game_event_21(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_GAME_EVENT_21);
        if rd(DIV_ON) == 0 {
            return;
        }
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x70);
            call_regs(f as usize, vt, ecx, f, esi, r.edi, &[r.edi])
        });
    }
}

// ---- diversions_on_game_event_22 @ 0x6046e0 (45 bytes): count ----
const DIVERSIONS_ON_GAME_EVENT_22_BODY: [u8; 45] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x74, 0x23, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x90,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x74, 0x57, 0xFF, 0xD2,
    0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01, 0x7C, 0xE5, 0x5E, 0xC3,
];
const DIVERSIONS_ON_GAME_EVENT_22_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_22_ABS: [usize; 3] = [2, 11, 37];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_22_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_GAME_EVENT_22: usize = 15;
hot_thunk!(thunk_diversions_on_game_event_22, port_diversions_on_game_event_22, "ret");
unsafe extern "C" fn port_diversions_on_game_event_22(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_GAME_EVENT_22);
        if rd(DIV_ON) == 0 {
            return;
        }
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x74);
            call_regs(f as usize, vt, ecx, f, esi, r.edi, &[r.edi])
        });
    }
}

// ---- diversions_on_object_damaged @ 0x604740 (70 bytes): count ----
const DIVERSIONS_ON_OBJECT_DAMAGED_BODY: [u8; 70] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x10, 0x55, 0x8B, 0x6C, 0x24,
    0x10, 0x74, 0x30, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x0F, 0x8B, 0x01, 0x8B, 0x54, 0x24, 0x10, 0x8B, 0x40,
    0x64, 0x57, 0x53, 0x55, 0x52, 0xFF, 0xD0, 0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01,
    0x7C, 0xDE, 0x5E, 0x5D, 0x5B, 0xC3,
];
const DIVERSIONS_ON_OBJECT_DAMAGED_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_OBJECT_DAMAGED_ABS: [usize; 3] = [2, 21, 60];
#[cfg(test)]
const DIVERSIONS_ON_OBJECT_DAMAGED_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_OBJECT_DAMAGED: usize = 16;
hot_thunk!(thunk_diversions_on_object_damaged, port_diversions_on_object_damaged, "ret");
unsafe extern "C" fn port_diversions_on_object_damaged(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_OBJECT_DAMAGED);
        if rd(DIV_ON) == 0 {
            return;
        }
        let (a0, a1, a2) = (*a, *a.add(1), *a.add(2));
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x64);
            call_regs(f as usize, f, ecx, a0, esi, r.edi, &[a0, a1, a2, r.edi])
        });
    }
}

// ---- diversions_on_vehicle_destroyed @ 0x6047f0 (70 bytes): count ----
const DIVERSIONS_ON_VEHICLE_DESTROYED_BODY: [u8; 70] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x10, 0x55, 0x8B, 0x6C, 0x24,
    0x10, 0x74, 0x30, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x0F, 0x8B, 0x01, 0x8B, 0x54, 0x24, 0x10, 0x8B, 0x40,
    0x5C, 0x57, 0x53, 0x55, 0x52, 0xFF, 0xD0, 0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01,
    0x7C, 0xDE, 0x5E, 0x5D, 0x5B, 0xC3,
];
const DIVERSIONS_ON_VEHICLE_DESTROYED_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_VEHICLE_DESTROYED_ABS: [usize; 3] = [2, 21, 60];
#[cfg(test)]
const DIVERSIONS_ON_VEHICLE_DESTROYED_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_VEHICLE_DESTROYED: usize = 17;
hot_thunk!(thunk_diversions_on_vehicle_destroyed, port_diversions_on_vehicle_destroyed, "ret");
unsafe extern "C" fn port_diversions_on_vehicle_destroyed(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_VEHICLE_DESTROYED);
        if rd(DIV_ON) == 0 {
            return;
        }
        let (a0, a1, a2) = (*a, *a.add(1), *a.add(2));
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x5c);
            call_regs(f as usize, f, ecx, a0, esi, r.edi, &[a0, a1, a2, r.edi])
        });
    }
}

// ---- diversions_on_object_killed @ 0x604840 (70 bytes): count ----
const DIVERSIONS_ON_OBJECT_KILLED_BODY: [u8; 70] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x10, 0x55, 0x8B, 0x6C, 0x24,
    0x10, 0x74, 0x30, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x0F, 0x8B, 0x01, 0x8B, 0x54, 0x24, 0x10, 0x8B, 0x40,
    0x58, 0x57, 0x53, 0x55, 0x52, 0xFF, 0xD0, 0x83, 0xC6, 0x04, 0x81, 0xFE, 0xFC, 0xB1, 0x40, 0x01,
    0x7C, 0xDE, 0x5E, 0x5D, 0x5B, 0xC3,
];
const DIVERSIONS_ON_OBJECT_KILLED_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_OBJECT_KILLED_ABS: [usize; 3] = [2, 21, 60];
#[cfg(test)]
const DIVERSIONS_ON_OBJECT_KILLED_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_OBJECT_KILLED: usize = 18;
hot_thunk!(thunk_diversions_on_object_killed, port_diversions_on_object_killed, "ret");
unsafe extern "C" fn port_diversions_on_object_killed(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_OBJECT_KILLED);
        if rd(DIV_ON) == 0 {
            return;
        }
        let (a0, a1, a2) = (*a, *a.add(1), *a.add(2));
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x58);
            call_regs(f as usize, f, ecx, a0, esi, r.edi, &[a0, a1, a2, r.edi])
        });
    }
}

// ---- diversions_on_game_event_15 @ 0x604630 (75 bytes): count ----
const DIVERSIONS_ON_GAME_EVENT_15_BODY: [u8; 75] = [
    0x83, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x14, 0x55, 0x8B, 0x6C, 0x24,
    0x14, 0x74, 0x35, 0x56, 0xBE, 0x68, 0xB1, 0x40, 0x01, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0E, 0x80, 0x79, 0x14, 0x00, 0x74, 0x14, 0x8B, 0x54, 0x24, 0x14, 0x8B, 0x01, 0x8B, 0x40,
    0x68, 0x57, 0x53, 0x55, 0x52, 0x8B, 0x54, 0x24, 0x20, 0x52, 0xFF, 0xD0, 0x83, 0xC6, 0x04, 0x81,
    0xFE, 0xFC, 0xB1, 0x40, 0x01, 0x7C, 0xD9, 0x5E, 0x5D, 0x5B, 0xC3,
];
const DIVERSIONS_ON_GAME_EVENT_15_CALLS: [usize; 0] = [];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_15_ABS: [usize; 3] = [2, 21, 65];
#[cfg(test)]
const DIVERSIONS_ON_GAME_EVENT_15_JMPS: [usize; 0] = [];
const ID_DIVERSIONS_ON_GAME_EVENT_15: usize = 19;
hot_thunk!(thunk_diversions_on_game_event_15, port_diversions_on_game_event_15, "ret");
unsafe extern "C" fn port_diversions_on_game_event_15(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_DIVERSIONS_ON_GAME_EVENT_15);
        if rd(DIV_ON) == 0 {
            return;
        }
        let (a0, a1, a2, a3) = (*a, *a.add(1), *a.add(2), *a.add(3));
        r.eax = div_walk(r.eax, |ecx, esi, _eax| {
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x68);
            call_regs(f as usize, f, ecx, a0, esi, r.edi, &[a0, a1, a2, a3, r.edi])
        });
    }
}

// ---- game_event_15 @ 0x68f8c0 (67 bytes): count ----
const GAME_EVENT_15_BODY: [u8; 67] = [
    0x53, 0x8B, 0x5C, 0x24, 0x10, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x57, 0x8B, 0xF8, 0xA1, 0x00, 0x6C,
    0xE9, 0x00, 0x6B, 0xC0, 0x6C, 0x05, 0x10, 0x6C, 0xE9, 0x00, 0x8B, 0x40, 0x3C, 0x85, 0xC0, 0x74,
    0x0E, 0x8B, 0x4C, 0x24, 0x10, 0x57, 0x56, 0x53, 0x55, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x14, 0x8B,
    0x54, 0x24, 0x10, 0x56, 0x53, 0x55, 0x52, 0xE8, 0x34, 0x4D, 0xF7, 0xFF, 0x83, 0xC4, 0x10, 0x5F,
    0x5D, 0x5B, 0xC3,
];
const GAME_EVENT_15_CALLS: [usize; 1] = [0x37];
#[cfg(test)]
const GAME_EVENT_15_ABS: [usize; 2] = [14, 22];
#[cfg(test)]
const GAME_EVENT_15_JMPS: [usize; 0] = [];
const ID_GAME_EVENT_15: usize = 20;
hot_thunk!(thunk_game_event_15, port_game_event_15, "ret");
unsafe extern "C" fn port_game_event_15(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_GAME_EVENT_15);
        let edi = r.eax;
        let (a0, a1, a2) = (*a, *a.add(1), *a.add(2));
        let fp = rd(evt_entry() + 0x3c);
        let mut eax = 0;
        if fp != 0 {
            eax = call_regs(fp as usize, fp, a0, r.edx, r.esi, edi, &[a0, a1, a2, r.esi, edi]);
        }
        r.eax = call_regs(0x0060_4630, eax, r.ecx, a0, r.esi, edi, &[a0, a1, a2, r.esi]);
    }
}

// ---- gfx_ring_has_data @ 0xcf5940 (11 bytes): pure ----
const GFX_RING_HAS_DATA_BODY: [u8; 11] = [
    0x8B, 0x48, 0x14, 0x3B, 0x48, 0x08, 0x1B, 0xC0, 0xF7, 0xD8, 0xC3,
];
const GFX_RING_HAS_DATA_CALLS: [usize; 0] = [];
#[cfg(test)]
const GFX_RING_HAS_DATA_ABS: [usize; 0] = [];
#[cfg(test)]
const GFX_RING_HAS_DATA_JMPS: [usize; 0] = [];
const ID_GFX_RING_HAS_DATA: usize = 21;
hot_thunk!(thunk_gfx_ring_has_data, port_gfx_ring_has_data, "ret");
unsafe extern "C" fn port_gfx_ring_has_data(r: &mut Pushad, a: *const u32) {

    unsafe {
        let o = r.eax as usize;
        let v = (rd(o + 0x14) < rd(o + 8)) as u32;
        r.eax = pure_done(ID_GFX_RING_HAS_DATA, r, a, 0, v);
    }
}

// ---- action_id_flags2_test @ 0x6ef730 (22 bytes): pure ----
const ACTION_ID_FLAGS2_TEST_BODY: [u8; 22] = [
    0x83, 0xF8, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8D, 0x04, 0x40, 0x85, 0x0C, 0x85, 0x98, 0xEE,
    0xE8, 0x00, 0x0F, 0x95, 0xC0, 0xC3,
];
const ACTION_ID_FLAGS2_TEST_CALLS: [usize; 0] = [];
#[cfg(test)]
const ACTION_ID_FLAGS2_TEST_ABS: [usize; 1] = [14];
#[cfg(test)]
const ACTION_ID_FLAGS2_TEST_JMPS: [usize; 0] = [];
const ID_ACTION_ID_FLAGS2_TEST: usize = 22;
hot_thunk!(thunk_action_id_flags2_test, port_action_id_flags2_test, "ret");
unsafe extern "C" fn port_action_id_flags2_test(r: &mut Pushad, a: *const u32) {

    unsafe {
        let id = r.eax;
        let v = if id == 0xFFFF_FFFF {
            id & !0xff
        } else {
            let t = id.wrapping_mul(3);
            let m = rd(0x00E8_EE98usize.wrapping_add((t as usize).wrapping_mul(4)));
            (t & !0xff) | (m & r.ecx != 0) as u32
        };
        r.eax = pure_done(ID_ACTION_ID_FLAGS2_TEST, r, a, 0, v);
    }
}

// ---- cutscene_player_flag_b01 @ 0x6c6d70 (27 bytes): pure ----
const CUTSCENE_PLAYER_FLAG_B01_BODY: [u8; 27] = [
    0xA1, 0x10, 0x7D, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x80, 0xB8, 0x01, 0x0B, 0x00, 0x00, 0x00,
    0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const CUTSCENE_PLAYER_FLAG_B01_CALLS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_PLAYER_FLAG_B01_ABS: [usize; 1] = [1];
#[cfg(test)]
const CUTSCENE_PLAYER_FLAG_B01_JMPS: [usize; 0] = [];
const ID_CUTSCENE_PLAYER_FLAG_B01: usize = 23;
hot_thunk!(thunk_cutscene_player_flag_b01, port_cutscene_player_flag_b01, "ret");
unsafe extern "C" fn port_cutscene_player_flag_b01(r: &mut Pushad, a: *const u32) {

    unsafe {
        let p = rd(0x0252_7D10) as usize;
        let v = (p != 0 && rb(p + 0xb01) != 0) as u32;
        r.eax = pure_done(ID_CUTSCENE_PLAYER_FLAG_B01, r, a, 0, v);
    }
}

// ---- cutscene_running @ 0x6c6d50 (27 bytes): pure ----
const CUTSCENE_RUNNING_BODY: [u8; 27] = [
    0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x0F, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00,
    0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const CUTSCENE_RUNNING_CALLS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_RUNNING_ABS: [usize; 2] = [2, 11];
#[cfg(test)]
const CUTSCENE_RUNNING_JMPS: [usize; 0] = [];
const ID_CUTSCENE_RUNNING: usize = 24;
hot_thunk!(thunk_cutscene_running, port_cutscene_running, "ret");
unsafe extern "C" fn port_cutscene_running(r: &mut Pushad, a: *const u32) {

    unsafe {
        let v = (rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) == 0) as u32;
        r.eax = pure_done(ID_CUTSCENE_RUNNING, r, a, 0, v);
    }
}

// ---- human_party_leader @ 0x98e710 (60 bytes): pure ----
const HUMAN_PARTY_LEADER_BODY: [u8; 60] = [
    0x85, 0xD2, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x8A, 0xA4, 0x02, 0x00, 0x00, 0x85, 0xC9, 0x74,
    0x28, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC0, 0x74, 0x14, 0x39, 0x48, 0x44, 0x75, 0x0F, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x09,
    0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x74, 0x02, 0x8B, 0xC2, 0xC3,
];
const HUMAN_PARTY_LEADER_CALLS: [usize; 0] = [];
#[cfg(test)]
const HUMAN_PARTY_LEADER_ABS: [usize; 1] = [29];
#[cfg(test)]
const HUMAN_PARTY_LEADER_JMPS: [usize; 0] = [];
const ID_HUMAN_PARTY_LEADER: usize = 25;
hot_thunk!(thunk_human_party_leader, port_human_party_leader, "ret");
unsafe extern "C" fn port_human_party_leader(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.edx;
        let v = if h == 0 {
            0
        } else {
            let ecx = rd(h as usize + 0x2a4);
            if ecx == 0 {
                h
            } else {
                let o = rd((((ecx & 0xfff) << 4) as usize).wrapping_add(0x0214_9C64)) as usize;
                if o == 0 || rd(o + 0x44) != ecx || rd(o + 0x4c) != 1 || rb(o + 0x80) & 0xb != 0 { h } else { o as u32 }
            }
        };
        r.eax = pure_done(ID_HUMAN_PARTY_LEADER, r, a, 0, v);
    }
}

// ---- pcu_item_find_material_by_id @ 0x7c1bd0 (40 bytes): pure ----
const PCU_ITEM_FIND_MATERIAL_BY_ID_BODY: [u8; 40] = [
    0x33, 0xC0, 0x85, 0xD2, 0x74, 0x21, 0x8B, 0x4A, 0x28, 0x8B, 0x52, 0x2C, 0x85, 0xD2, 0x7E, 0x0F,
    0x39, 0x71, 0x10, 0x74, 0x0A, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x70, 0x3B, 0xC2, 0x7C, 0xF1, 0x2B,
    0xC2, 0xF7, 0xD8, 0x1B, 0xC0, 0x23, 0xC1, 0xC3,
];
const PCU_ITEM_FIND_MATERIAL_BY_ID_CALLS: [usize; 0] = [];
#[cfg(test)]
const PCU_ITEM_FIND_MATERIAL_BY_ID_ABS: [usize; 0] = [];
#[cfg(test)]
const PCU_ITEM_FIND_MATERIAL_BY_ID_JMPS: [usize; 0] = [];
const ID_PCU_ITEM_FIND_MATERIAL_BY_ID: usize = 26;
hot_thunk!(thunk_pcu_item_find_material_by_id, port_pcu_item_find_material_by_id, "ret");
unsafe extern "C" fn port_pcu_item_find_material_by_id(r: &mut Pushad, a: *const u32) {

    unsafe {
        let item = r.edx as usize;
        let v = if item == 0 {
            0
        } else {
            let mut ecx = rd(item + 0x28);
            let cnt = rd(item + 0x2c);
            let mut i = 0u32;
            if (cnt as i32) > 0 {
                loop {
                    if rd(ecx as usize + 0x10) == r.esi {
                        break;
                    }
                    i = i.wrapping_add(1);
                    ecx = ecx.wrapping_add(0x70);
                    if (i as i32) >= (cnt as i32) {
                        break;
                    }
                }
            }
            if i != cnt { ecx } else { 0 }
        };
        r.eax = pure_done(ID_PCU_ITEM_FIND_MATERIAL_BY_ID, r, a, 0, v);
    }
}

// ---- gfx_ring_read_u32 @ 0xbd5550 (18 bytes): snap ----
const GFX_RING_READ_U32_BODY: [u8; 18] = [
    0x8B, 0x48, 0x14, 0x8B, 0x11, 0x8B, 0x4C, 0x24, 0x04, 0x89, 0x11, 0x83, 0x40, 0x14, 0x04, 0xC2,
    0x04, 0x00,
];
const GFX_RING_READ_U32_CALLS: [usize; 0] = [];
#[cfg(test)]
const GFX_RING_READ_U32_ABS: [usize; 0] = [];
#[cfg(test)]
const GFX_RING_READ_U32_JMPS: [usize; 0] = [];
const ID_GFX_RING_READ_U32: usize = 27;
hot_thunk!(thunk_gfx_ring_read_u32, port_gfx_ring_read_u32, "ret 4");
unsafe extern "C" fn port_gfx_ring_read_u32(r: &mut Pushad, a: *const u32) {

    unsafe {
        let ring = r.eax as usize;
        let dst = *a as usize;
        r.eax = snap_done(ID_GFX_RING_READ_U32, r, a, 1, &[(dst, 4), (ring + 0x14, 4)], || {
            let p = rd(ring + 0x14);
            let v = rd(p as usize);
            wd(dst, v);
            wd(ring + 0x14, rd(ring + 0x14).wrapping_add(4));
            ring as u32
        });
    }
}

// ---- idirect3ddevice9lost_static_init @ 0xcf58e0 (26 bytes): snap ----
const IDIRECT3DDEVICE9LOST_STATIC_INIT_BODY: [u8; 26] = [
    0xC7, 0x05, 0xCC, 0x24, 0xEC, 0x00, 0xA4, 0x84, 0xE5, 0x00, 0xC7, 0x05, 0xD0, 0x24, 0xEC, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xB8, 0xCC, 0x24, 0xEC, 0x00, 0xC3,
];
const IDIRECT3DDEVICE9LOST_STATIC_INIT_CALLS: [usize; 0] = [];
#[cfg(test)]
const IDIRECT3DDEVICE9LOST_STATIC_INIT_ABS: [usize; 4] = [2, 6, 12, 21];
#[cfg(test)]
const IDIRECT3DDEVICE9LOST_STATIC_INIT_JMPS: [usize; 0] = [];
const ID_IDIRECT3DDEVICE9LOST_STATIC_INIT: usize = 28;
hot_thunk!(thunk_idirect3ddevice9lost_static_init, port_idirect3ddevice9lost_static_init, "ret");
unsafe extern "C" fn port_idirect3ddevice9lost_static_init(r: &mut Pushad, a: *const u32) {

    unsafe {
        r.eax = snap_done(ID_IDIRECT3DDEVICE9LOST_STATIC_INIT, r, a, 0, &[(0x00EC_24CC, 8)], || {
            wd(0x00EC_24CC, 0x00E5_84A4);
            wd(0x00EC_24D0, 0);
            0x00EC_24CC
        });
    }
}

// ---- human_flag_c8_stamp_set @ 0x964560 (63 bytes): snap ----
const HUMAN_FLAG_C8_STAMP_SET_BODY: [u8; 63] = [
    0x8B, 0x88, 0xC8, 0x00, 0x00, 0x00, 0x81, 0xE1, 0xFF, 0xF7, 0xFF, 0xFF, 0x81, 0xC9, 0x00, 0x04,
    0x00, 0x00, 0x89, 0x88, 0xC8, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x81, 0xF9,
    0x00, 0xD2, 0x49, 0x6B, 0x89, 0x88, 0x68, 0x0D, 0x00, 0x00, 0x7E, 0x0C, 0x81, 0xC1, 0x00, 0x2E,
    0xB6, 0x94, 0x89, 0x88, 0x68, 0x0D, 0x00, 0x00, 0x89, 0x90, 0x6C, 0x0D, 0x00, 0x00, 0xC3,
];
const HUMAN_FLAG_C8_STAMP_SET_CALLS: [usize; 0] = [];
#[cfg(test)]
const HUMAN_FLAG_C8_STAMP_SET_ABS: [usize; 1] = [26];
#[cfg(test)]
const HUMAN_FLAG_C8_STAMP_SET_JMPS: [usize; 0] = [];
const ID_HUMAN_FLAG_C8_STAMP_SET: usize = 29;
hot_thunk!(thunk_human_flag_c8_stamp_set, port_human_flag_c8_stamp_set, "ret");
unsafe extern "C" fn port_human_flag_c8_stamp_set(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.eax as usize;
        let (edx, eax) = (r.edx, r.eax);
        r.eax = snap_done(ID_HUMAN_FLAG_C8_STAMP_SET, r, a, 0, &[(h + 0xc8, 4), (h + 0xd68, 8)], || {
            let c8 = (rd(h + 0xc8) & 0xFFFF_F7FF) | 0x400;
            wd(h + 0xc8, c8);
            let mut t = rd(0x00E8_43AC);
            wd(h + 0xd68, t);
            if (t as i32) > 0x6B49_D200 {
                t = t.wrapping_add(0x94B6_2E00);
                wd(h + 0xd68, t);
            }
            wd(h + 0xd6c, edx);
            eax
        });
    }
}

// ---- nav_search_begin @ 0xa0ffa0 (36 bytes): snap ----
const NAV_SEARCH_BEGIN_BODY: [u8; 36] = [
    0x33, 0xC0, 0xA3, 0xC0, 0x60, 0xE1, 0x02, 0xA3, 0xC4, 0xE0, 0xE2, 0x02, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x05, 0xBC, 0x60, 0xE1, 0x02, 0xA2, 0xB8, 0x60, 0xE1, 0x02, 0x75, 0x05, 0xA3, 0xBC,
    0x60, 0xE1, 0x02, 0xC3,
];
const NAV_SEARCH_BEGIN_CALLS: [usize; 0] = [];
#[cfg(test)]
const NAV_SEARCH_BEGIN_ABS: [usize; 5] = [3, 8, 19, 24, 31];
#[cfg(test)]
const NAV_SEARCH_BEGIN_JMPS: [usize; 0] = [];
const ID_NAV_SEARCH_BEGIN: usize = 30;
hot_thunk!(thunk_nav_search_begin, port_nav_search_begin, "ret");
unsafe extern "C" fn port_nav_search_begin(r: &mut Pushad, a: *const u32) {

    unsafe {
        r.eax = snap_done(ID_NAV_SEARCH_BEGIN, r, a, 0, &[(0x02E1_60B8, 12), (0x02E2_E0C4, 4)], || {
            wd(0x02E1_60C0, 0);
            wd(0x02E2_E0C4, 0);
            let n = rd(0x02E1_60BC).wrapping_add(1);
            wd(0x02E1_60BC, n);
            wb(0x02E1_60B8, 1);
            if n != 0 {
                1
            } else {
                wd(0x02E1_60BC, 1);
                1
            }
        });
    }
}

// ---- havok_removal_queue_push @ 0x6f5cd0 (38 bytes): count ----
const HAVOK_REMOVAL_QUEUE_PUSH_BODY: [u8; 38] = [
    0xA1, 0xC0, 0x7D, 0x52, 0x02, 0x3D, 0xD0, 0x07, 0x00, 0x00, 0x72, 0x0A, 0xE8, 0x2F, 0xFF, 0xFF,
    0xFF, 0xA1, 0xC0, 0x7D, 0x52, 0x02, 0x89, 0x34, 0x85, 0x38, 0xB7, 0x68, 0x01, 0x83, 0xC0, 0x01,
    0xA3, 0xC0, 0x7D, 0x52, 0x02, 0xC3,
];
const HAVOK_REMOVAL_QUEUE_PUSH_CALLS: [usize; 1] = [0xc];
#[cfg(test)]
const HAVOK_REMOVAL_QUEUE_PUSH_ABS: [usize; 4] = [1, 18, 25, 33];
#[cfg(test)]
const HAVOK_REMOVAL_QUEUE_PUSH_JMPS: [usize; 0] = [];
const ID_HAVOK_REMOVAL_QUEUE_PUSH: usize = 31;
hot_thunk!(thunk_havok_removal_queue_push, port_havok_removal_queue_push, "ret");
unsafe extern "C" fn port_havok_removal_queue_push(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HAVOK_REMOVAL_QUEUE_PUSH);
        let mut eax = rd(0x0252_7DC0);
        if eax >= 0x7d0 {
            hit(ID_HAVOK_REMOVAL_QUEUE_PUSH, 0);
            call_regs(0x006F_5C10, eax, r.ecx, r.edx, r.esi, r.edi, &[]);
            eax = rd(0x0252_7DC0);
        }
        wd(0x0168_B738usize.wrapping_add((eax as usize).wrapping_mul(4)), r.esi);
        eax = eax.wrapping_add(1);
        wd(0x0252_7DC0, eax);
        r.eax = eax;
    }
}

// ---- traffic_record_activate @ 0x938c30 (121 bytes): count ----
const TRAFFIC_RECORD_ACTIVATE_BODY: [u8; 121] = [
    0x8B, 0x50, 0x10, 0x83, 0xE1, 0x01, 0x83, 0xE2, 0xFC, 0x0B, 0xCA, 0x39, 0x05, 0xAC, 0xD7, 0x15,
    0x02, 0x89, 0x48, 0x10, 0x75, 0x16, 0x8B, 0x08, 0x3B, 0xC8, 0x75, 0x0A, 0x33, 0xD2, 0x89, 0x15,
    0xAC, 0xD7, 0x15, 0x02, 0xEB, 0x21, 0x89, 0x0D, 0xAC, 0xD7, 0x15, 0x02, 0x8B, 0x48, 0x04, 0x33,
    0xD2, 0x3B, 0xCA, 0x57, 0x74, 0x04, 0x8B, 0x38, 0x89, 0x39, 0x8B, 0x08, 0x3B, 0xCA, 0x74, 0x06,
    0x8B, 0x78, 0x04, 0x89, 0x79, 0x04, 0x5F, 0x8B, 0x0D, 0x30, 0xDA, 0x15, 0x02, 0x83, 0x05, 0xA8,
    0xD7, 0x15, 0x02, 0x01, 0x3B, 0xCA, 0x89, 0x50, 0x04, 0x89, 0x10, 0x75, 0x0B, 0x89, 0x00, 0x89,
    0x40, 0x04, 0xA3, 0x30, 0xDA, 0x15, 0x02, 0xC3, 0x8B, 0x51, 0x04, 0x89, 0x50, 0x04, 0x89, 0x08,
    0x8B, 0x51, 0x04, 0x89, 0x02, 0x89, 0x41, 0x04, 0xC3,
];
const TRAFFIC_RECORD_ACTIVATE_CALLS: [usize; 0] = [];
#[cfg(test)]
const TRAFFIC_RECORD_ACTIVATE_ABS: [usize; 6] = [13, 32, 40, 73, 79, 99];
#[cfg(test)]
const TRAFFIC_RECORD_ACTIVATE_JMPS: [usize; 0] = [];
const ID_TRAFFIC_RECORD_ACTIVATE: usize = 32;
hot_thunk!(thunk_traffic_record_activate, port_traffic_record_activate, "ret");
unsafe extern "C" fn port_traffic_record_activate(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_TRAFFIC_RECORD_ACTIVATE);
        let rec = r.eax as usize;
        let flags = (r.ecx & 1) | (rd(rec + 0x10) & !3);
        let head_a = rd(0x0215_D7AC);
        wd(rec + 0x10, flags);
        let mut unlink = true;
        if head_a == rec as u32 {
            let next = rd(rec);
            if next == rec as u32 {
                wd(0x0215_D7AC, 0);
                unlink = false;
            } else {
                wd(0x0215_D7AC, next);
            }
        }
        if unlink {
            let prev = rd(rec + 4);
            if prev != 0 {
                wd(prev as usize, rd(rec));
            }
            let next = rd(rec);
            if next != 0 {
                wd(next as usize + 4, rd(rec + 4));
            }
        }
        let head_b = rd(0x0215_DA30);
        wd(0x0215_D7A8, rd(0x0215_D7A8).wrapping_add(1));
        wd(rec + 4, 0);
        wd(rec, 0);
        if head_b == 0 {
            wd(rec, rec as u32);
            wd(rec + 4, rec as u32);
            wd(0x0215_DA30, rec as u32);
        } else {
            let h = head_b as usize;
            let tail = rd(h + 4);
            wd(rec + 4, tail);
            wd(rec, head_b);
            let tail = rd(h + 4);
            wd(tail as usize, rec as u32);
            wd(h + 4, rec as u32);
        }
    }
}

// ---- traffic_record_release @ 0x938cb0 (107 bytes): count ----
const TRAFFIC_RECORD_RELEASE_BODY: [u8; 107] = [
    0x39, 0x05, 0x30, 0xDA, 0x15, 0x02, 0x75, 0x16, 0x8B, 0x08, 0x3B, 0xC8, 0x75, 0x0A, 0x33, 0xD2,
    0x89, 0x15, 0x30, 0xDA, 0x15, 0x02, 0xEB, 0x21, 0x89, 0x0D, 0x30, 0xDA, 0x15, 0x02, 0x8B, 0x48,
    0x04, 0x33, 0xD2, 0x3B, 0xCA, 0x56, 0x74, 0x04, 0x8B, 0x30, 0x89, 0x31, 0x8B, 0x08, 0x3B, 0xCA,
    0x74, 0x06, 0x8B, 0x70, 0x04, 0x89, 0x71, 0x04, 0x5E, 0x8B, 0x0D, 0xAC, 0xD7, 0x15, 0x02, 0x83,
    0x2D, 0xA8, 0xD7, 0x15, 0x02, 0x01, 0x3B, 0xCA, 0x89, 0x50, 0x04, 0x89, 0x10, 0x75, 0x0B, 0x89,
    0x00, 0x89, 0x40, 0x04, 0xA3, 0xAC, 0xD7, 0x15, 0x02, 0xC3, 0x8B, 0x51, 0x04, 0x89, 0x50, 0x04,
    0x89, 0x08, 0x8B, 0x51, 0x04, 0x89, 0x02, 0x89, 0x41, 0x04, 0xC3,
];
const TRAFFIC_RECORD_RELEASE_CALLS: [usize; 0] = [];
#[cfg(test)]
const TRAFFIC_RECORD_RELEASE_ABS: [usize; 6] = [2, 18, 26, 59, 65, 85];
#[cfg(test)]
const TRAFFIC_RECORD_RELEASE_JMPS: [usize; 0] = [];
const ID_TRAFFIC_RECORD_RELEASE: usize = 33;
hot_thunk!(thunk_traffic_record_release, port_traffic_record_release, "ret");
unsafe extern "C" fn port_traffic_record_release(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_TRAFFIC_RECORD_RELEASE);
        let rec = r.eax as usize;
        let mut unlink = true;
        if rd(0x0215_DA30) == rec as u32 {
            let next = rd(rec);
            if next == rec as u32 {
                wd(0x0215_DA30, 0);
                unlink = false;
            } else {
                wd(0x0215_DA30, next);
            }
        }
        if unlink {
            let prev = rd(rec + 4);
            if prev != 0 {
                wd(prev as usize, rd(rec));
            }
            let next = rd(rec);
            if next != 0 {
                wd(next as usize + 4, rd(rec + 4));
            }
        }
        let head_a = rd(0x0215_D7AC);
        wd(0x0215_D7A8, rd(0x0215_D7A8).wrapping_sub(1));
        wd(rec + 4, 0);
        wd(rec, 0);
        if head_a == 0 {
            wd(rec, rec as u32);
            wd(rec + 4, rec as u32);
            wd(0x0215_D7AC, rec as u32);
        } else {
            let h = head_a as usize;
            let tail = rd(h + 4);
            wd(rec + 4, tail);
            wd(rec, head_a);
            let tail = rd(h + 4);
            wd(tail as usize, rec as u32);
            wd(h + 4, rec as u32);
        }
    }
}

// ---- traffic_record_release_by_vehicle @ 0x93d910 (49 bytes): count ----
const TRAFFIC_RECORD_RELEASE_BY_VEHICLE_BODY: [u8; 49] = [
    0x56, 0x8B, 0x35, 0x30, 0xDA, 0x15, 0x02, 0x8B, 0xC6, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x85, 0xC0, 0x74, 0x1B, 0x8B, 0x10, 0x8B, 0xCA, 0x2B, 0xCE, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xCA,
    0x39, 0x78, 0x08, 0x74, 0x04, 0x8B, 0xC1, 0xEB, 0xE7, 0x5E, 0xE9, 0x71, 0xB3, 0xFF, 0xFF, 0x5E,
    0xC3,
];
const TRAFFIC_RECORD_RELEASE_BY_VEHICLE_CALLS: [usize; 0] = [];
#[cfg(test)]
const TRAFFIC_RECORD_RELEASE_BY_VEHICLE_ABS: [usize; 1] = [3];
#[cfg(test)]
const TRAFFIC_RECORD_RELEASE_BY_VEHICLE_JMPS: [usize; 1] = [42];
const ID_TRAFFIC_RECORD_RELEASE_BY_VEHICLE: usize = 34;
hot_thunk!(thunk_traffic_record_release_by_vehicle, port_traffic_record_release_by_vehicle, "ret");
unsafe extern "C" fn port_traffic_record_release_by_vehicle(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_TRAFFIC_RECORD_RELEASE_BY_VEHICLE);
        let head = rd(0x0215_DA30);
        let mut eax = head;
        loop {
            if eax == 0 {
                r.eax = 0;
                return;
            }
            let edx = rd(eax as usize);
            let next = if edx != head { edx } else { 0 };
            if rd(eax as usize + 8) == r.edi {
                r.eax = call_regs(0x0093_8CB0, eax, next, edx, r.esi, r.edi, &[]);
                return;
            }
            eax = next;
        }
    }
}

// ---- tod_object_node_alloc @ 0x4d27e0 (88 bytes): count ----
const TOD_OBJECT_NODE_ALLOC_BODY: [u8; 88] = [
    0xA1, 0x24, 0x99, 0xF9, 0x00, 0x33, 0xC9, 0x3B, 0xC1, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8D, 0x50,
    0x10, 0x56, 0x8B, 0x32, 0x3B, 0xF0, 0x75, 0x08, 0x89, 0x0D, 0x24, 0x99, 0xF9, 0x00, 0xEB, 0x20,
    0x89, 0x35, 0x24, 0x99, 0xF9, 0x00, 0x8B, 0x70, 0x14, 0x3B, 0xF1, 0x57, 0x74, 0x05, 0x8B, 0x3A,
    0x89, 0x7E, 0x10, 0x8B, 0x32, 0x3B, 0xF1, 0x74, 0x06, 0x8B, 0x78, 0x14, 0x89, 0x7E, 0x14, 0x5F,
    0x89, 0x48, 0x14, 0x89, 0x0A, 0x89, 0x08, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x89, 0x48, 0x0C,
    0x89, 0x48, 0x10, 0x89, 0x48, 0x14, 0x5E, 0xC3,
];
const TOD_OBJECT_NODE_ALLOC_CALLS: [usize; 0] = [];
#[cfg(test)]
const TOD_OBJECT_NODE_ALLOC_ABS: [usize; 3] = [1, 26, 34];
#[cfg(test)]
const TOD_OBJECT_NODE_ALLOC_JMPS: [usize; 0] = [];
const ID_TOD_OBJECT_NODE_ALLOC: usize = 35;
hot_thunk!(thunk_tod_object_node_alloc, port_tod_object_node_alloc, "ret");
unsafe extern "C" fn port_tod_object_node_alloc(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_TOD_OBJECT_NODE_ALLOC);
        let eax = rd(0x00F9_9924) as usize;
        if eax == 0 {
            r.eax = 0;
            return;
        }
        let edx = eax + 0x10;
        let esi = rd(edx);
        if esi != eax as u32 {
            wd(0x00F9_9924, esi);
            let s2 = rd(eax + 0x14);
            if s2 != 0 {
                wd(s2 as usize + 0x10, rd(edx));
            }
            let s3 = rd(edx);
            if s3 != 0 {
                wd(s3 as usize + 0x14, rd(eax + 0x14));
            }
        } else {
            wd(0x00F9_9924, 0);
        }
        wd(eax + 0x14, 0);
        wd(edx, 0);
        wd(eax, 0);
        wd(eax + 4, 0);
        wd(eax + 8, 0);
        wd(eax + 0xc, 0);
        wd(eax + 0x10, 0);
        wd(eax + 0x14, 0);
        r.eax = eax as u32;
    }
}

// ---- ptr_array_compact @ 0x523b80 (79 bytes): count ----
const PTR_ARRAY_COMPACT_BODY: [u8; 79] = [
    0x33, 0xC9, 0x39, 0x0A, 0x74, 0x0F, 0x3B, 0xCE, 0x7D, 0x44, 0x83, 0xC2, 0x04, 0x83, 0xC1, 0x01,
    0x83, 0x3A, 0x00, 0x75, 0xF1, 0x3B, 0xCE, 0x7D, 0x35, 0x8B, 0xC2, 0x57, 0x8D, 0x64, 0x24, 0x00,
    0x83, 0x38, 0x00, 0x75, 0x0F, 0x3B, 0xCE, 0x7D, 0x24, 0x83, 0xC0, 0x04, 0x83, 0xC1, 0x01, 0x83,
    0x38, 0x00, 0x74, 0xF1, 0x3B, 0xCE, 0x7D, 0x15, 0x8B, 0x38, 0x89, 0x3A, 0xC7, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x83, 0xC2, 0x04, 0x83, 0xC0, 0x04, 0x83, 0xC1, 0x01, 0xEB, 0xD3, 0x5F, 0xC3,
];
const PTR_ARRAY_COMPACT_CALLS: [usize; 0] = [];
#[cfg(test)]
const PTR_ARRAY_COMPACT_ABS: [usize; 0] = [];
#[cfg(test)]
const PTR_ARRAY_COMPACT_JMPS: [usize; 0] = [];
const ID_PTR_ARRAY_COMPACT: usize = 36;
hot_thunk!(thunk_ptr_array_compact, port_ptr_array_compact, "ret");
unsafe extern "C" fn port_ptr_array_compact(r: &mut Pushad, a: *const u32) {

    unsafe {
        let (mut edx, esi) = (r.edx, r.esi as i32);
        let mut ecx = 0i32;
        // phase 1: skip the leading non-null entries
        loop {
            if rd(edx as usize) == 0 {
                break;
            }
            if ecx >= esi {
                r.ecx = ecx as u32;
                r.edx = edx;
                return;
            }
            edx = edx.wrapping_add(4);
            ecx += 1;
        }
        if ecx >= esi {
            r.ecx = ecx as u32;
            r.edx = edx;
            return;
        }
        let mut eax = edx;
        loop {
            if rd(eax as usize) == 0 {
                loop {
                    if ecx >= esi {
                        r.eax = eax;
                        r.ecx = ecx as u32;
                        r.edx = edx;
                        return;
                    }
                    eax = eax.wrapping_add(4);
                    ecx += 1;
                    if rd(eax as usize) != 0 {
                        break;
                    }
                }
            }
            if ecx >= esi {
                r.eax = eax;
                r.ecx = ecx as u32;
                r.edx = edx;
                return;
            }
            let v = rd(eax as usize);
            wd(edx as usize, v);
            wd(eax as usize, 0);
            edx = edx.wrapping_add(4);
            eax = eax.wrapping_add(4);
            ecx += 1;
        }
    }
}

// ---- cutscene_seen_flags_store_bits @ 0x6c3ad0 (95 bytes): count ----
const CUTSCENE_SEEN_FLAGS_STORE_BITS_BODY: [u8; 95] = [
    0x33, 0xC0, 0x39, 0x05, 0xAC, 0xA4, 0x45, 0x01, 0x7E, 0x54, 0x57, 0x33, 0xFF, 0x8D, 0x49, 0x00,
    0x8B, 0x0D, 0xA4, 0xA4, 0x45, 0x01, 0x80, 0x7C, 0x0F, 0x08, 0x00, 0x74, 0x19, 0x8B, 0x0E, 0x8B,
    0xD0, 0xC1, 0xEA, 0x03, 0x03, 0xCA, 0x8B, 0xD0, 0x83, 0xE2, 0x07, 0x8A, 0x14, 0x95, 0x58, 0x85,
    0xDC, 0x00, 0x08, 0x11, 0xEB, 0x19, 0x8B, 0x16, 0x8B, 0xC8, 0xC1, 0xE9, 0x03, 0x03, 0xCA, 0x8B,
    0xD0, 0x83, 0xE2, 0x07, 0x8A, 0x14, 0x95, 0x58, 0x85, 0xDC, 0x00, 0xF6, 0xD2, 0x20, 0x11, 0x83,
    0xC0, 0x01, 0x83, 0xC7, 0x14, 0x3B, 0x05, 0xAC, 0xA4, 0x45, 0x01, 0x7C, 0xB3, 0x5F, 0xC3,
];
const CUTSCENE_SEEN_FLAGS_STORE_BITS_CALLS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_SEEN_FLAGS_STORE_BITS_ABS: [usize; 5] = [4, 18, 46, 71, 87];
#[cfg(test)]
const CUTSCENE_SEEN_FLAGS_STORE_BITS_JMPS: [usize; 0] = [];
const ID_CUTSCENE_SEEN_FLAGS_STORE_BITS: usize = 37;
hot_thunk!(thunk_cutscene_seen_flags_store_bits, port_cutscene_seen_flags_store_bits, "ret");
unsafe extern "C" fn port_cutscene_seen_flags_store_bits(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_SEEN_FLAGS_STORE_BITS);
        let count = rd(0x0145_A4AC) as i32;
        if count <= 0 {
            r.eax = 0;
            return;
        }
        let mut eax = 0i32;
        let mut edi = 0usize;
        loop {
            let tbl = rd(0x0145_A4A4) as usize;
            let bit = rd(0x00DC_8558 + (eax as usize & 7) * 4) as u8;
            let byte = (rd(r.esi as usize) as usize).wrapping_add((eax >> 3) as usize);
            if rb(edi.wrapping_add(tbl) + 8) != 0 {
                wb(byte, rb(byte) | bit);
            } else {
                wb(byte, rb(byte) & !bit);
            }
            eax += 1;
            edi = edi.wrapping_add(0x14);
            if eax >= rd(0x0145_A4AC) as i32 {
                break;
            }
        }
        r.eax = eax as u32;
    }
}

// ---- ambient_vehicle_budget_remaining @ 0x93c1d0 (51 bytes): count ----
const AMBIENT_VEHICLE_BUDGET_REMAINING_BODY: [u8; 51] = [
    0xA0, 0x48, 0x8B, 0x52, 0x02, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x2B, 0x35, 0xD4, 0xA0, 0x52,
    0x02, 0x3C, 0x01, 0x75, 0x0E, 0x8A, 0x0D, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x75, 0x04, 0x8B,
    0xC6, 0x5E, 0xC3, 0xE8, 0xD8, 0xB9, 0x09, 0x00, 0x85, 0xC0, 0x8B, 0xC6, 0x74, 0x03, 0x8D, 0x46,
    0xFC, 0x5E, 0xC3,
];
const AMBIENT_VEHICLE_BUDGET_REMAINING_CALLS: [usize; 1] = [0x23];
#[cfg(test)]
const AMBIENT_VEHICLE_BUDGET_REMAINING_ABS: [usize; 3] = [1, 13, 23];
#[cfg(test)]
const AMBIENT_VEHICLE_BUDGET_REMAINING_JMPS: [usize; 0] = [];
const ID_AMBIENT_VEHICLE_BUDGET_REMAINING: usize = 38;
hot_thunk!(thunk_ambient_vehicle_budget_remaining, port_ambient_vehicle_budget_remaining, "ret");
unsafe extern "C" fn port_ambient_vehicle_budget_remaining(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_AMBIENT_VEHICLE_BUDGET_REMAINING);
        let al = rb(0x0252_8B48);
        let esi = 0x20u32.wrapping_sub(rd(0x0252_A0D4));
        let mut ecx = r.ecx;
        if al == 1 {
            let cl = rb(0x0252_8B49);
            ecx = (ecx & !0xff) | cl as u32;
            if cl == 0 {
                r.eax = esi;
                return;
            }
        }
        let c = call_regs(0x009D_7BD0, (r.eax & !0xff) | al as u32, ecx, r.edx, esi, r.edi, &[]);
        r.eax = if c == 0 { esi } else { esi.wrapping_sub(4) };
    }
}

// ---- current_spawn_group_query @ 0x957660 (61 bytes): count ----
const CURRENT_SPAWN_GROUP_QUERY_BODY: [u8; 61] = [
    0x8B, 0x0D, 0xBC, 0xE5, 0x15, 0x02, 0x85, 0xC9, 0x56, 0x8B, 0xF1, 0x75, 0x04, 0x32, 0xC0, 0x5E,
    0xC3, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0x53, 0xFF, 0xD2, 0x8A, 0xD8, 0x8B, 0x06, 0x8B, 0x50, 0x38,
    0x8B, 0xCE, 0xFF, 0xD2, 0x85, 0xC0, 0x0F, 0x9F, 0xC0, 0x84, 0xDB, 0x5B, 0x74, 0x0B, 0x84, 0xC0,
    0x74, 0x07, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x33, 0xC0, 0x5E, 0xC3,
];
const CURRENT_SPAWN_GROUP_QUERY_CALLS: [usize; 0] = [];
#[cfg(test)]
const CURRENT_SPAWN_GROUP_QUERY_ABS: [usize; 1] = [2];
#[cfg(test)]
const CURRENT_SPAWN_GROUP_QUERY_JMPS: [usize; 0] = [];
const ID_CURRENT_SPAWN_GROUP_QUERY: usize = 39;
hot_thunk!(thunk_current_spawn_group_query, port_current_spawn_group_query, "ret");
unsafe extern "C" fn port_current_spawn_group_query(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CURRENT_SPAWN_GROUP_QUERY);
        let this = rd(0x0215_E5BC);
        if this == 0 {
            r.eax = r.eax & !0xff;
            return;
        }
        let vt = rd(this as usize);
        let f1 = rd(vt as usize + 0x34);
        let e1 = call_regs(f1 as usize, vt, this, f1, this, r.edi, &[]);
        let vt2 = rd(this as usize);
        let f2 = rd(vt2 as usize + 0x38);
        let e2 = call_regs(f2 as usize, vt2, this, f2, this, r.edi, &[]);
        r.eax = ((e1 & 0xff) != 0 && (e2 as i32) > 0) as u32;
    }
}

// ---- current_spawn_group_vehicle_index @ 0x9392a0 (51 bytes): count ----
const CURRENT_SPAWN_GROUP_VEHICLE_INDEX_BODY: [u8; 51] = [
    0x83, 0x3D, 0xBC, 0xE5, 0x15, 0x02, 0x00, 0x74, 0x26, 0xE8, 0xB2, 0xE3, 0x01, 0x00, 0x84, 0xC0,
    0x74, 0x1D, 0x8B, 0x0D, 0xBC, 0xE5, 0x15, 0x02, 0x85, 0xC9, 0x74, 0x13, 0x8B, 0x01, 0x8B, 0x50,
    0x14, 0xFF, 0xD2, 0x85, 0xC0, 0x7C, 0x08, 0x3B, 0x05, 0xD8, 0xA0, 0x52, 0x02, 0x7C, 0x03, 0x83,
    0xC8, 0xFF, 0xC3,
];
const CURRENT_SPAWN_GROUP_VEHICLE_INDEX_CALLS: [usize; 1] = [0x9];
#[cfg(test)]
const CURRENT_SPAWN_GROUP_VEHICLE_INDEX_ABS: [usize; 3] = [2, 20, 41];
#[cfg(test)]
const CURRENT_SPAWN_GROUP_VEHICLE_INDEX_JMPS: [usize; 0] = [];
const ID_CURRENT_SPAWN_GROUP_VEHICLE_INDEX: usize = 40;
hot_thunk!(thunk_current_spawn_group_vehicle_index, port_current_spawn_group_vehicle_index, "ret");
unsafe extern "C" fn port_current_spawn_group_vehicle_index(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CURRENT_SPAWN_GROUP_VEHICLE_INDEX);
        let neg = |r: &mut Pushad| r.eax = 0xFFFF_FFFF;
        if rd(0x0215_E5BC) == 0 {
            return neg(r);
        }
        let q = call_regs(0x0095_7660, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        if (q & 0xff) == 0 {
            return neg(r);
        }
        let this = rd(0x0215_E5BC);
        if this == 0 {
            return neg(r);
        }
        let vt = rd(this as usize);
        let f = rd(vt as usize + 0x14);
        let e = call_regs(f as usize, vt, this, f, r.esi, r.edi, &[]);
        if (e as i32) < 0 {
            return neg(r);
        }
        if (e as i32) < rd(0x0252_A0D8) as i32 {
            r.eax = e;
        } else {
            neg(r)
        }
    }
}

// ---- human_is_non_ally_state7 @ 0x9636c0 (23 bytes): count ----
const HUMAN_IS_NON_ALLY_STATE7_BODY: [u8; 23] = [
    0xE8, 0xDB, 0xB1, 0x02, 0x00, 0x84, 0xC0, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0x80, 0xBA, 0x1C, 0x11,
    0x00, 0x00, 0x07, 0x0F, 0x94, 0xC0, 0xC3,
];
const HUMAN_IS_NON_ALLY_STATE7_CALLS: [usize; 1] = [0x0];
#[cfg(test)]
const HUMAN_IS_NON_ALLY_STATE7_ABS: [usize; 0] = [];
#[cfg(test)]
const HUMAN_IS_NON_ALLY_STATE7_JMPS: [usize; 0] = [];
const ID_HUMAN_IS_NON_ALLY_STATE7: usize = 41;
hot_thunk!(thunk_human_is_non_ally_state7, port_human_is_non_ally_state7, "ret");
unsafe extern "C" fn port_human_is_non_ally_state7(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HUMAN_IS_NON_ALLY_STATE7);
        let e = call_regs(0x0098_E8A0, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        r.eax = if (e & 0xff) != 0 { e & !0xff } else { (e & !0xff) | (rb(r.edx as usize + 0x111c) == 7) as u32 };
    }
}

// ---- human_is_follower @ 0x98e880 (27 bytes): count ----
const HUMAN_IS_FOLLOWER_BODY: [u8; 27] = [
    0x57, 0x8B, 0xF8, 0x85, 0xFF, 0x75, 0x04, 0x32, 0xC0, 0x5F, 0xC3, 0xE8, 0x30, 0xFF, 0xFF, 0xFF,
    0x33, 0xC9, 0x3B, 0xC7, 0x0F, 0x95, 0xC1, 0x8A, 0xC1, 0x5F, 0xC3,
];
const HUMAN_IS_FOLLOWER_CALLS: [usize; 1] = [0xb];
#[cfg(test)]
const HUMAN_IS_FOLLOWER_ABS: [usize; 0] = [];
#[cfg(test)]
const HUMAN_IS_FOLLOWER_JMPS: [usize; 0] = [];
const ID_HUMAN_IS_FOLLOWER: usize = 42;
hot_thunk!(thunk_human_is_follower, port_human_is_follower, "ret");
unsafe extern "C" fn port_human_is_follower(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HUMAN_IS_FOLLOWER);
        let edi = r.eax;
        if edi == 0 {
            r.eax = r.eax & !0xff;
            return;
        }
        let e = call_regs(0x0098_E7C0, r.eax, r.ecx, r.edx, r.esi, edi, &[]);
        r.eax = (e & !0xff) | (e != edi) as u32;
    }
}

// ---- human_action_flag_bit4 @ 0x96d930 (33 bytes): count ----
const HUMAN_ACTION_FLAG_BIT4_BODY: [u8; 33] = [
    0x50, 0xE8, 0x6A, 0x1A, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x83, 0xF8, 0xFF, 0x75, 0x03, 0x32, 0xC0,
    0xC3, 0x8D, 0x0C, 0x40, 0x8B, 0x04, 0x8D, 0x48, 0x0E, 0xE9, 0x00, 0xC1, 0xE8, 0x04, 0x24, 0x01,
    0xC3,
];
const HUMAN_ACTION_FLAG_BIT4_CALLS: [usize; 1] = [0x1];
#[cfg(test)]
const HUMAN_ACTION_FLAG_BIT4_ABS: [usize; 1] = [23];
#[cfg(test)]
const HUMAN_ACTION_FLAG_BIT4_JMPS: [usize; 0] = [];
const ID_HUMAN_ACTION_FLAG_BIT4: usize = 43;
hot_thunk!(thunk_human_action_flag_bit4, port_human_action_flag_bit4, "ret");
unsafe extern "C" fn port_human_action_flag_bit4(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HUMAN_ACTION_FLAG_BIT4);
        let e = call_regs(0x0096_F3A0, r.eax, r.ecx, r.edx, r.esi, r.edi, &[r.eax]);
        if e == 0xFFFF_FFFF {
            r.eax = e & !0xff;
            return;
        }
        let v = rd(0x00E9_0E48usize.wrapping_add((e.wrapping_mul(3) as usize).wrapping_mul(4))) >> 4;
        r.eax = (v & !0xff) | (v & 1);
    }
}

// ---- weather_force_stage_and_sync @ 0x4ded80 (49 bytes): count ----
const WEATHER_FORCE_STAGE_AND_SYNC_BODY: [u8; 49] = [
    0x69, 0xC0, 0xD8, 0x00, 0x00, 0x00, 0x03, 0x05, 0xD0, 0x6D, 0x52, 0x02, 0xA3, 0x10, 0x6E, 0x52,
    0x02, 0xA1, 0xB4, 0x43, 0xE8, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x60, 0xD4, 0xE9, 0x00,
    0x7E, 0x0A, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0xA3, 0x60, 0xD4, 0xE9, 0x00, 0xE9, 0xDF, 0xF5, 0xFF,
    0xFF,
];
const WEATHER_FORCE_STAGE_AND_SYNC_CALLS: [usize; 0] = [];
#[cfg(test)]
const WEATHER_FORCE_STAGE_AND_SYNC_ABS: [usize; 5] = [8, 13, 18, 28, 40];
#[cfg(test)]
const WEATHER_FORCE_STAGE_AND_SYNC_JMPS: [usize; 1] = [44];
const ID_WEATHER_FORCE_STAGE_AND_SYNC: usize = 44;
hot_thunk!(thunk_weather_force_stage_and_sync, port_weather_force_stage_and_sync, "ret");
unsafe extern "C" fn port_weather_force_stage_and_sync(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_WEATHER_FORCE_STAGE_AND_SYNC);
        let eax = r.eax.wrapping_mul(0xd8).wrapping_add(rd(0x0252_6DD0));
        wd(0x0252_6E10, eax);
        let mut t = rd(0x00E8_43B4);
        wd(0x00E9_D460, t);
        if (t as i32) > 0x6B49_D200 {
            t = t.wrapping_add(0x94B6_2E00);
            wd(0x00E9_D460, t);
        }
        r.eax = call_regs(0x004D_E390, t, r.ecx, r.edx, r.esi, r.edi, &[]);
    }
}

// ---- input_actions_reset @ 0x500170 (69 bytes): count ----
const INPUT_ACTIONS_RESET_BODY: [u8; 69] = [
    0x33, 0xD2, 0x33, 0xC0, 0x83, 0xC9, 0xFF, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x89, 0x88, 0x38, 0xEA, 0xF9, 0x00, 0x89, 0x88, 0x3C, 0xEA, 0xF9, 0x00, 0x89, 0x88, 0x40, 0xEA,
    0xF9, 0x00, 0x89, 0x90, 0x44, 0xEA, 0xF9, 0x00, 0x83, 0xC0, 0x10, 0x3D, 0x40, 0x08, 0x00, 0x00,
    0x72, 0xDE, 0xA0, 0x9B, 0x54, 0xF8, 0x00, 0x24, 0xF1, 0x0C, 0x01, 0xA2, 0x9B, 0x54, 0xF8, 0x00,
    0xE9, 0x3B, 0xF2, 0xFF, 0xFF,
];
const INPUT_ACTIONS_RESET_CALLS: [usize; 0] = [];
#[cfg(test)]
const INPUT_ACTIONS_RESET_ABS: [usize; 6] = [18, 24, 30, 36, 51, 60];
#[cfg(test)]
const INPUT_ACTIONS_RESET_JMPS: [usize; 1] = [64];
const ID_INPUT_ACTIONS_RESET: usize = 45;
hot_thunk!(thunk_input_actions_reset, port_input_actions_reset, "ret");
unsafe extern "C" fn port_input_actions_reset(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_INPUT_ACTIONS_RESET);
        let mut o = 0usize;
        while o < 0x840 {
            wd(0x00F9_EA38 + o, 0xFFFF_FFFF);
            wd(0x00F9_EA3C + o, 0xFFFF_FFFF);
            wd(0x00F9_EA40 + o, 0xFFFF_FFFF);
            wd(0x00F9_EA44 + o, 0);
            o += 0x10;
        }
        let al = (rb(0x00F8_549B) & 0xf1) | 1;
        wb(0x00F8_549B, al);
        r.eax = call_regs(0x004F_F3F0, 0x800 | al as u32, 0xFFFF_FFFF, 0, r.esi, r.edi, &[]);
    }
}

// ---- render_list0_add @ 0x52fda0 (71 bytes): count ----
const RENDER_LIST0_ADD_BODY: [u8; 71] = [
    0x68, 0x90, 0x49, 0x78, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x00, 0x06, 0x00, 0x00,
    0x77, 0x28, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85,
    0x7C, 0xD1, 0x77, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x8B, 0x54, 0x24,
    0x10, 0x89, 0x48, 0x08, 0x89, 0x50, 0x0C, 0xC2, 0x10, 0x00, 0xC7, 0x05, 0x90, 0x49, 0x78, 0x02,
    0x00, 0x06, 0x00, 0x00, 0xC2, 0x10, 0x00,
];
const RENDER_LIST0_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST0_ADD_ABS: [usize; 4] = [1, 7, 32, 60];
#[cfg(test)]
const RENDER_LIST0_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST0_ADD: usize = 46;
hot_thunk!(thunk_render_list0_add, port_render_list0_add, "ret 16");
unsafe extern "C" fn port_render_list0_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST0_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x2784990]);
        if n > 0x600 {
            wd(0x2784990, 0x600);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x277d17c);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, *a.add(3));
        r.eax = p as u32;
    }
}

// ---- render_list0b_add @ 0x52fe10 (71 bytes): count ----
const RENDER_LIST0B_ADD_BODY: [u8; 71] = [
    0x68, 0x98, 0x99, 0x78, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x00, 0x04, 0x00, 0x00,
    0x77, 0x28, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85,
    0x84, 0x49, 0x78, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x8B, 0x54, 0x24,
    0x10, 0x89, 0x48, 0x08, 0x89, 0x50, 0x0C, 0xC2, 0x10, 0x00, 0xC7, 0x05, 0x98, 0x99, 0x78, 0x02,
    0x00, 0x04, 0x00, 0x00, 0xC2, 0x10, 0x00,
];
const RENDER_LIST0B_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST0B_ADD_ABS: [usize; 4] = [1, 7, 32, 60];
#[cfg(test)]
const RENDER_LIST0B_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST0B_ADD: usize = 47;
hot_thunk!(thunk_render_list0b_add, port_render_list0b_add, "ret 16");
unsafe extern "C" fn port_render_list0b_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST0B_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x2789998]);
        if n > 0x400 {
            wd(0x2789998, 0x400);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x2784984);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, *a.add(3));
        r.eax = p as u32;
    }
}

// ---- render_list_vis_add @ 0x52fe80 (71 bytes): count ----
const RENDER_LIST_VIS_ADD_BODY: [u8; 71] = [
    0x68, 0xA0, 0xC1, 0x78, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x00, 0x02, 0x00, 0x00,
    0x77, 0x28, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85,
    0x8C, 0x99, 0x78, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x89, 0x48, 0x08,
    0xC7, 0x40, 0x0C, 0x00, 0x00, 0x00, 0x00, 0xC2, 0x0C, 0x00, 0xC7, 0x05, 0xA0, 0xC1, 0x78, 0x02,
    0x00, 0x02, 0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const RENDER_LIST_VIS_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST_VIS_ADD_ABS: [usize; 4] = [1, 7, 32, 60];
#[cfg(test)]
const RENDER_LIST_VIS_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST_VIS_ADD: usize = 48;
hot_thunk!(thunk_render_list_vis_add, port_render_list_vis_add, "ret 12");
unsafe extern "C" fn port_render_list_vis_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST_VIS_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x278c1a0]);
        if n > 0x200 {
            wd(0x278c1a0, 0x200);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x278998c);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, 0);
        r.eax = p as u32;
    }
}

// ---- render_list3_add @ 0x52ff60 (69 bytes): count ----
const RENDER_LIST3_ADD_BODY: [u8; 69] = [
    0x68, 0x38, 0xEC, 0x78, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x83, 0xF8, 0x20, 0x77, 0x28,
    0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85, 0xA4, 0xE9,
    0x78, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x89, 0x48, 0x08, 0xC7, 0x40,
    0x0C, 0x00, 0x00, 0x00, 0x00, 0xC2, 0x0C, 0x00, 0xC7, 0x05, 0x38, 0xEC, 0x78, 0x02, 0x20, 0x00,
    0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const RENDER_LIST3_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST3_ADD_ABS: [usize; 4] = [1, 7, 30, 58];
#[cfg(test)]
const RENDER_LIST3_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST3_ADD: usize = 49;
hot_thunk!(thunk_render_list3_add, port_render_list3_add, "ret 12");
unsafe extern "C" fn port_render_list3_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST3_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x278ec38]);
        if n > 0x20 {
            wd(0x278ec38, 0x20);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x278e9a4);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, 0);
        r.eax = p as u32;
    }
}

// ---- render_list4_add @ 0x52ffd0 (63 bytes): count ----
const RENDER_LIST4_ADD_BODY: [u8; 63] = [
    0x68, 0x80, 0xED, 0x78, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x83, 0xF8, 0x10, 0x77, 0x22,
    0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85, 0x2C, 0xEC,
    0x78, 0x02, 0x89, 0x08, 0x33, 0xC9, 0x89, 0x50, 0x04, 0x89, 0x48, 0x08, 0x89, 0x48, 0x0C, 0xC2,
    0x08, 0x00, 0xC7, 0x05, 0x80, 0xED, 0x78, 0x02, 0x10, 0x00, 0x00, 0x00, 0xC2, 0x08, 0x00,
];
const RENDER_LIST4_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST4_ADD_ABS: [usize; 4] = [1, 7, 30, 52];
#[cfg(test)]
const RENDER_LIST4_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST4_ADD: usize = 50;
hot_thunk!(thunk_render_list4_add, port_render_list4_add, "ret 8");
unsafe extern "C" fn port_render_list4_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST4_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x278ed80]);
        if n > 0x10 {
            wd(0x278ed80, 0x10);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x278ec2c);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, 0);
        wd(p + 0xc, 0);
        r.eax = p as u32;
    }
}

// ---- render_list6_add @ 0x530030 (71 bytes): count ----
const RENDER_LIST6_ADD_BODY: [u8; 71] = [
    0x68, 0x48, 0x0D, 0x79, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x96, 0x00, 0x00, 0x00,
    0x77, 0x28, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85,
    0x7C, 0x01, 0x79, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x89, 0x48, 0x08,
    0xC7, 0x40, 0x0C, 0x00, 0x00, 0x00, 0x00, 0xC2, 0x0C, 0x00, 0xC7, 0x05, 0x48, 0x0D, 0x79, 0x02,
    0x96, 0x00, 0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const RENDER_LIST6_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST6_ADD_ABS: [usize; 4] = [1, 7, 32, 60];
#[cfg(test)]
const RENDER_LIST6_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST6_ADD: usize = 51;
hot_thunk!(thunk_render_list6_add, port_render_list6_add, "ret 12");
unsafe extern "C" fn port_render_list6_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST6_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x2790d48]);
        if n > 0x96 {
            wd(0x2790d48, 0x96);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x279017c);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, 0);
        r.eax = p as u32;
    }
}

// ---- render_list7_add @ 0x5300a0 (71 bytes): count ----
const RENDER_LIST7_ADD_BODY: [u8; 71] = [
    0x68, 0x40, 0x18, 0x79, 0x02, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x8C, 0x00, 0x00, 0x00,
    0x77, 0x28, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85,
    0x3C, 0x0D, 0x79, 0x02, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x0C, 0x89, 0x50, 0x04, 0x8B, 0x54, 0x24,
    0x10, 0x89, 0x48, 0x08, 0x89, 0x50, 0x0C, 0xC2, 0x10, 0x00, 0xC7, 0x05, 0x40, 0x18, 0x79, 0x02,
    0x8C, 0x00, 0x00, 0x00, 0xC2, 0x10, 0x00,
];
const RENDER_LIST7_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST7_ADD_ABS: [usize; 4] = [1, 7, 32, 60];
#[cfg(test)]
const RENDER_LIST7_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST7_ADD: usize = 52;
hot_thunk!(thunk_render_list7_add, port_render_list7_add, "ret 16");
unsafe extern "C" fn port_render_list7_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST7_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let n = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0x2791840]);
        if n > 0x8c {
            wd(0x2791840, 0x8c);
            r.eax = n;
            return;
        }
        let p = (n.wrapping_mul(20) as usize).wrapping_add(0x2790d3c);
        wd(p + 0x0, *a.add(0));
        wd(p + 0x4, *a.add(1));
        wd(p + 0x8, *a.add(2));
        wd(p + 0xc, *a.add(3));
        r.eax = p as u32;
    }
}

// ---- render_list_small_add @ 0x52fef0 (69 bytes): count ----
const RENDER_LIST_SMALL_ADD_BODY: [u8; 69] = [
    0x56, 0x8D, 0xB7, 0x00, 0x14, 0x00, 0x00, 0x56, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0x3D, 0x00,
    0x01, 0x00, 0x00, 0x77, 0x26, 0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x54, 0x24, 0x0C, 0x8D, 0x04, 0x80,
    0x8D, 0x44, 0x87, 0xEC, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x10, 0x89, 0x50, 0x04, 0x89, 0x48, 0x08,
    0xC7, 0x40, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC2, 0x0C, 0x00, 0xC7, 0x06, 0x00, 0x01, 0x00,
    0x00, 0x5E, 0xC2, 0x0C, 0x00,
];
const RENDER_LIST_SMALL_ADD_CALLS: [usize; 0] = [];
#[cfg(test)]
const RENDER_LIST_SMALL_ADD_ABS: [usize; 1] = [10];
#[cfg(test)]
const RENDER_LIST_SMALL_ADD_JMPS: [usize; 0] = [];
const ID_RENDER_LIST_SMALL_ADD: usize = 53;
hot_thunk!(thunk_render_list_small_add, port_render_list_small_add, "ret 12");
unsafe extern "C" fn port_render_list_small_add(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_RENDER_LIST_SMALL_ADD);
        let f = rd(0x00DB_80A4) as usize;
        let esi = r.edi.wrapping_add(0x1400);
        let n = call_regs(f, r.eax, r.ecx, r.edx, esi, r.edi, &[esi]);
        if n > 0x100 {
            wd(esi as usize, 0x100);
            r.eax = n;
            return;
        }
        let p = (r.edi as usize).wrapping_add((n.wrapping_mul(5) as usize).wrapping_mul(4)).wrapping_sub(0x14);
        wd(p, *a);
        wd(p + 4, *a.add(1));
        wd(p + 8, *a.add(2));
        wd(p + 0xc, 0);
        r.eax = p as u32;
    }
}

// ---- unlockable_parse_weapon_primary @ 0x6bb3e0 (52 bytes): count ----
const UNLOCKABLE_PARSE_WEAPON_PRIMARY_BODY: [u8; 52] = [
    0x85, 0xC0, 0x75, 0x05, 0x32, 0xC0, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x04, 0x58, 0xDC, 0x00, 0xE8,
    0xEC, 0x0A, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x50, 0xE8, 0xDE, 0x62,
    0x4B, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x89, 0x41, 0x08, 0x0F, 0x95,
    0xC0, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_WEAPON_PRIMARY_CALLS: [usize; 2] = [0xf, 0x1d];
#[cfg(test)]
const UNLOCKABLE_PARSE_WEAPON_PRIMARY_ABS: [usize; 1] = [11];
#[cfg(test)]
const UNLOCKABLE_PARSE_WEAPON_PRIMARY_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_WEAPON_PRIMARY: usize = 54;
hot_thunk!(thunk_unlockable_parse_weapon_primary, port_unlockable_parse_weapon_primary, "ret 4");
unsafe extern "C" fn port_unlockable_parse_weapon_primary(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_WEAPON_PRIMARY);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0xdc5804, &[]);
        let v = if f != 0 { rd(f as usize + 0xc) } else { f };
        let e = call_regs(0xb716e0, v, r.ecx, r.edx, r.esi, r.edi, &[v]);
        wd(*a as usize + 8, e);
        r.eax = (e & !0xff) | (e != 0) as u32;
    }
}

// ---- unlockable_parse_weapon_secondary @ 0x6bb420 (52 bytes): count ----
const UNLOCKABLE_PARSE_WEAPON_SECONDARY_BODY: [u8; 52] = [
    0x85, 0xC0, 0x75, 0x05, 0x32, 0xC0, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x04, 0x58, 0xDC, 0x00, 0xE8,
    0xAC, 0x0A, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x50, 0xE8, 0x9E, 0x62,
    0x4B, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x89, 0x41, 0x08, 0x0F, 0x95,
    0xC0, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_WEAPON_SECONDARY_CALLS: [usize; 2] = [0xf, 0x1d];
#[cfg(test)]
const UNLOCKABLE_PARSE_WEAPON_SECONDARY_ABS: [usize; 1] = [11];
#[cfg(test)]
const UNLOCKABLE_PARSE_WEAPON_SECONDARY_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_WEAPON_SECONDARY: usize = 55;
hot_thunk!(thunk_unlockable_parse_weapon_secondary, port_unlockable_parse_weapon_secondary, "ret 4");
unsafe extern "C" fn port_unlockable_parse_weapon_secondary(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_WEAPON_SECONDARY);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0xdc5804, &[]);
        let v = if f != 0 { rd(f as usize + 0xc) } else { f };
        let e = call_regs(0xb716e0, v, r.ecx, r.edx, r.esi, r.edi, &[v]);
        wd(*a as usize + 8, e);
        r.eax = (e & !0xff) | (e != 0) as u32;
    }
}

// ---- unlockable_parse_contact @ 0x6bb390 (70 bytes): count ----
const UNLOCKABLE_PARSE_CONTACT_BODY: [u8; 70] = [
    0x85, 0xC0, 0x75, 0x05, 0x32, 0xC0, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x04, 0x58, 0xDC, 0x00, 0xE8,
    0x3C, 0x0B, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x75, 0x15, 0xE8, 0x02, 0xBC, 0x0C, 0x00, 0x8B, 0x4C,
    0x24, 0x04, 0x83, 0xF8, 0xFF, 0x89, 0x41, 0x08, 0x0F, 0x95, 0xC0, 0xC2, 0x04, 0x00, 0x8B, 0x40,
    0x0C, 0xE8, 0xEA, 0xBB, 0x0C, 0x00, 0x8B, 0x4C, 0x24, 0x04, 0x83, 0xF8, 0xFF, 0x89, 0x41, 0x08,
    0x0F, 0x95, 0xC0, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_CONTACT_CALLS: [usize; 3] = [0xf, 0x19, 0x31];
#[cfg(test)]
const UNLOCKABLE_PARSE_CONTACT_ABS: [usize; 1] = [11];
#[cfg(test)]
const UNLOCKABLE_PARSE_CONTACT_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_CONTACT: usize = 56;
hot_thunk!(thunk_unlockable_parse_contact, port_unlockable_parse_contact, "ret 4");
unsafe extern "C" fn port_unlockable_parse_contact(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_CONTACT);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0x00DC_5804, &[]);
        let v = if f != 0 { rd(f as usize + 0xc) } else { 0 };
        let e = call_regs(0x0078_6FB0, v, r.ecx, r.edx, r.esi, r.edi, &[]);
        wd(*a as usize + 8, e);
        r.eax = (e & !0xff) | (e != 0xFFFF_FFFF) as u32;
    }
}

// ---- unlockable_parse_crib_ref @ 0x6bba90 (51 bytes): count ----
const UNLOCKABLE_PARSE_CRIB_REF_BODY: [u8; 51] = [
    0x85, 0xC0, 0x75, 0x05, 0x32, 0xC0, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x04, 0x58, 0xDC, 0x00, 0xE8,
    0x3C, 0x04, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8B, 0xC8, 0xE8, 0x7D,
    0xC8, 0xE8, 0xFF, 0x8B, 0x4C, 0x24, 0x04, 0x83, 0xF8, 0xFF, 0x89, 0x41, 0x08, 0x0F, 0x95, 0xC0,
    0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_CRIB_REF_CALLS: [usize; 2] = [0xf, 0x1e];
#[cfg(test)]
const UNLOCKABLE_PARSE_CRIB_REF_ABS: [usize; 1] = [11];
#[cfg(test)]
const UNLOCKABLE_PARSE_CRIB_REF_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_CRIB_REF: usize = 57;
hot_thunk!(thunk_unlockable_parse_crib_ref, port_unlockable_parse_crib_ref, "ret 4");
unsafe extern "C" fn port_unlockable_parse_crib_ref(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_CRIB_REF);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0x00DC_5804, &[]);
        let v = if f != 0 { rd(f as usize + 0xc) } else { f };
        let e = call_regs(0x0054_8330, v, v, r.edx, r.esi, r.edi, &[]);
        wd(*a as usize + 8, e);
        r.eax = (e & !0xff) | (e != 0xFFFF_FFFF) as u32;
    }
}

// ---- unlockable_parse_gang_type @ 0x6bbb40 (61 bytes): count ----
const UNLOCKABLE_PARSE_GANG_TYPE_BODY: [u8; 61] = [
    0x51, 0x85, 0xC0, 0x75, 0x06, 0x32, 0xC0, 0x59, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0xF4, 0xC3, 0xE0,
    0x00, 0xE8, 0x8A, 0x03, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x8B,
    0x48, 0x0C, 0x8D, 0x04, 0x24, 0x50, 0x33, 0xC0, 0xE8, 0xC3, 0x0F, 0x52, 0x00, 0x8B, 0x0C, 0x24,
    0x8B, 0x54, 0x24, 0x08, 0x89, 0x4A, 0x08, 0xB0, 0x01, 0x59, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_GANG_TYPE_CALLS: [usize; 2] = [0x11, 0x28];
#[cfg(test)]
const UNLOCKABLE_PARSE_GANG_TYPE_ABS: [usize; 1] = [13];
#[cfg(test)]
const UNLOCKABLE_PARSE_GANG_TYPE_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_GANG_TYPE: usize = 58;
hot_thunk!(thunk_unlockable_parse_gang_type, port_unlockable_parse_gang_type, "ret 4");
unsafe extern "C" fn port_unlockable_parse_gang_type(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_GANG_TYPE);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0xe0c3f4, &[]);
        let str_ = if f != 0 { rd(f as usize + 0xc) } else { 0 };
        let mut local = 0u32;
        let p = call_regs(0x00BD_CB30, 0, str_, r.edx, r.esi, r.edi, &[&mut local as *mut u32 as u32]);
        wd(*a as usize + 8, local);
        r.eax = (p & !0xff) | 1;
    }
}

// ---- unlockable_parse_vehicle_group @ 0x6bbb80 (61 bytes): count ----
const UNLOCKABLE_PARSE_VEHICLE_GROUP_BODY: [u8; 61] = [
    0x51, 0x85, 0xC0, 0x75, 0x06, 0x32, 0xC0, 0x59, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x58, 0xF1, 0xDF,
    0x00, 0xE8, 0x4A, 0x03, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x8B,
    0x48, 0x0C, 0x8D, 0x04, 0x24, 0x50, 0x33, 0xC0, 0xE8, 0x83, 0x0F, 0x52, 0x00, 0x8B, 0x0C, 0x24,
    0x8B, 0x54, 0x24, 0x08, 0x89, 0x4A, 0x08, 0xB0, 0x01, 0x59, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_VEHICLE_GROUP_CALLS: [usize; 2] = [0x11, 0x28];
#[cfg(test)]
const UNLOCKABLE_PARSE_VEHICLE_GROUP_ABS: [usize; 1] = [13];
#[cfg(test)]
const UNLOCKABLE_PARSE_VEHICLE_GROUP_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_VEHICLE_GROUP: usize = 59;
hot_thunk!(thunk_unlockable_parse_vehicle_group, port_unlockable_parse_vehicle_group, "ret 4");
unsafe extern "C" fn port_unlockable_parse_vehicle_group(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_VEHICLE_GROUP);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0xdff158, &[]);
        let str_ = if f != 0 { rd(f as usize + 0xc) } else { 0 };
        let mut local = 0u32;
        let p = call_regs(0x00BD_CB30, 0, str_, r.edx, r.esi, r.edi, &[&mut local as *mut u32 as u32]);
        wd(*a as usize + 8, local);
        r.eax = (p & !0xff) | 1;
    }
}

// ---- unlockable_parse_style @ 0x6bbbc0 (61 bytes): count ----
const UNLOCKABLE_PARSE_STYLE_BODY: [u8; 61] = [
    0x51, 0x85, 0xC0, 0x75, 0x06, 0x32, 0xC0, 0x59, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x40, 0x5A, 0xDD,
    0x00, 0xE8, 0x0A, 0x03, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x8B,
    0x48, 0x0C, 0x8D, 0x04, 0x24, 0x50, 0x33, 0xC0, 0xE8, 0x43, 0x0F, 0x52, 0x00, 0x8B, 0x0C, 0x24,
    0x8B, 0x54, 0x24, 0x08, 0x89, 0x4A, 0x08, 0xB0, 0x01, 0x59, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_STYLE_CALLS: [usize; 2] = [0x11, 0x28];
#[cfg(test)]
const UNLOCKABLE_PARSE_STYLE_ABS: [usize; 1] = [13];
#[cfg(test)]
const UNLOCKABLE_PARSE_STYLE_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_STYLE: usize = 60;
hot_thunk!(thunk_unlockable_parse_style, port_unlockable_parse_style, "ret 4");
unsafe extern "C" fn port_unlockable_parse_style(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_STYLE);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0xdd5a40, &[]);
        let str_ = if f != 0 { rd(f as usize + 0xc) } else { 0 };
        let mut local = 0u32;
        let p = call_regs(0x00BD_CB30, 0, str_, r.edx, r.esi, r.edi, &[&mut local as *mut u32 as u32]);
        wd(*a as usize + 8, local);
        r.eax = (p & !0xff) | 1;
    }
}

// ---- unlockable_parse_item @ 0x6bb460 (68 bytes): count ----
const UNLOCKABLE_PARSE_ITEM_BODY: [u8; 68] = [
    0x51, 0x85, 0xC0, 0x75, 0x06, 0x32, 0xC0, 0x59, 0xC2, 0x04, 0x00, 0x57, 0xBF, 0x04, 0x58, 0xDC,
    0x00, 0xE8, 0x6A, 0x0A, 0x54, 0x00, 0x85, 0xC0, 0x5F, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x8B,
    0x48, 0x0C, 0x8D, 0x04, 0x24, 0x50, 0x33, 0xC0, 0xE8, 0xA3, 0x16, 0x52, 0x00, 0x8B, 0xC8, 0xE8,
    0xFC, 0x0D, 0x26, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x85, 0xC0, 0x89, 0x41, 0x08, 0x0F, 0x95, 0xC0,
    0x59, 0xC2, 0x04, 0x00,
];
const UNLOCKABLE_PARSE_ITEM_CALLS: [usize; 3] = [0x11, 0x28, 0x2f];
#[cfg(test)]
const UNLOCKABLE_PARSE_ITEM_ABS: [usize; 1] = [13];
#[cfg(test)]
const UNLOCKABLE_PARSE_ITEM_JMPS: [usize; 0] = [];
const ID_UNLOCKABLE_PARSE_ITEM: usize = 61;
hot_thunk!(thunk_unlockable_parse_item, port_unlockable_parse_item, "ret 4");
unsafe extern "C" fn port_unlockable_parse_item(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_UNLOCKABLE_PARSE_ITEM);
        if r.eax == 0 {
            return;
        }
        let f = call_regs(0x00BF_BEE0, r.eax, r.ecx, r.edx, r.esi, 0x00DC_5804, &[]);
        let str_ = if f != 0 { rd(f as usize + 0xc) } else { 0 };
        let mut local = 0u32;
        let p = call_regs(0x00BD_CB30, 0, str_, r.edx, r.esi, r.edi, &[&mut local as *mut u32 as u32]);
        let e = call_regs(0x0091_C290, p, p, r.edx, r.esi, r.edi, &[]);
        wd(*a as usize + 8, e);
        r.eax = (e & !0xff) | (e != 0) as u32;
    }
}

// ---- act_heli_text @ 0x576e60 (46 bytes): count ----
const ACT_HELI_TEXT_BODY: [u8; 46] = [
    0x85, 0xC0, 0x56, 0x57, 0x75, 0x0F, 0x33, 0xFF, 0xBE, 0x8C, 0x8E, 0xDE, 0x00, 0xE8, 0x6E, 0xDB,
    0x27, 0x00, 0x5F, 0x5E, 0xC3, 0x83, 0xF8, 0x02, 0x75, 0x0F, 0x33, 0xFF, 0xBE, 0x9C, 0x8E, 0xDE,
    0x00, 0xE8, 0x5A, 0xDB, 0x27, 0x00, 0x5F, 0x5E, 0xC3, 0x5F, 0x33, 0xC0, 0x5E, 0xC3,
];
const ACT_HELI_TEXT_CALLS: [usize; 2] = [0xd, 0x21];
#[cfg(test)]
const ACT_HELI_TEXT_ABS: [usize; 2] = [9, 29];
#[cfg(test)]
const ACT_HELI_TEXT_JMPS: [usize; 0] = [];
const ID_ACT_HELI_TEXT: usize = 62;
hot_thunk!(thunk_act_heli_text, port_act_heli_text, "ret");
unsafe extern "C" fn port_act_heli_text(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_ACT_HELI_TEXT);
        let esi = match r.eax {
            0 => 0x00DE_8E8C,
            2 => 0x00DE_8E9C,
            _ => {
                r.eax = 0;
                return;
            }
        };
        r.eax = call_regs(0x007F_49E0, r.eax, r.ecx, r.edx, esi, 0, &[]);
    }
}

// ---- act_holdup_start_msg_text @ 0x655700 (74 bytes): count ----
const ACT_HOLDUP_START_MSG_TEXT_BODY: [u8; 74] = [
    0x56, 0x33, 0xC0, 0x57, 0x33, 0xFF, 0xBE, 0x38, 0xBD, 0xDF, 0x00, 0xA3, 0xE4, 0xC7, 0x40, 0x01,
    0xA3, 0xE0, 0xC7, 0x40, 0x01, 0xE8, 0xC6, 0xF2, 0x19, 0x00, 0xBE, 0xB8, 0xBE, 0xDF, 0x00, 0xA3,
    0xD4, 0xC7, 0x40, 0x01, 0xE8, 0xB7, 0xF2, 0x19, 0x00, 0xBE, 0xD0, 0xBE, 0xDF, 0x00, 0xA3, 0xD8,
    0xC7, 0x40, 0x01, 0xE8, 0xA8, 0xF2, 0x19, 0x00, 0x5F, 0xA3, 0xDC, 0xC7, 0x40, 0x01, 0xC7, 0x05,
    0x9C, 0xA7, 0xEA, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x5E, 0xC3,
];
const ACT_HOLDUP_START_MSG_TEXT_CALLS: [usize; 3] = [0x15, 0x24, 0x33];
#[cfg(test)]
const ACT_HOLDUP_START_MSG_TEXT_ABS: [usize; 9] = [7, 12, 17, 27, 32, 42, 47, 58, 64];
#[cfg(test)]
const ACT_HOLDUP_START_MSG_TEXT_JMPS: [usize; 0] = [];
const ID_ACT_HOLDUP_START_MSG_TEXT: usize = 63;
hot_thunk!(thunk_act_holdup_start_msg_text, port_act_holdup_start_msg_text, "ret");
unsafe extern "C" fn port_act_holdup_start_msg_text(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_ACT_HOLDUP_START_MSG_TEXT);
        wd(0x0140_C7E4, 0);
        wd(0x0140_C7E0, 0);
        let e1 = call_regs(0x007F_49E0, 0, r.ecx, r.edx, 0x00DF_BD38, 0, &[]);
        wd(0x0140_C7D4, e1);
        let e2 = call_regs(0x007F_49E0, e1, r.ecx, r.edx, 0x00DF_BEB8, 0, &[]);
        wd(0x0140_C7D8, e2);
        let e3 = call_regs(0x007F_49E0, e2, r.ecx, r.edx, 0x00DF_BED0, 0, &[]);
        wd(0x0140_C7DC, e3);
        wd(0x00EA_A79C, 0xFFFF_FFFF);
        r.eax = e3;
    }
}

// ---- act_level_text @ 0x53a7a0 (125 bytes): count ----
const ACT_LEVEL_TEXT_BODY: [u8; 125] = [
    0x56, 0x57, 0xBF, 0x50, 0x2E, 0xDE, 0x00, 0xBE, 0x58, 0x2E, 0xDE, 0x00, 0xE8, 0x1F, 0xA3, 0x2B,
    0x00, 0xBF, 0x64, 0x2E, 0xDE, 0x00, 0xBE, 0x6C, 0x2E, 0xDE, 0x00, 0xA3, 0x14, 0x74, 0x52, 0x02,
    0xE8, 0x0B, 0xA3, 0x2B, 0x00, 0xBF, 0x78, 0x2E, 0xDE, 0x00, 0xBE, 0x80, 0x2E, 0xDE, 0x00, 0xA3,
    0x18, 0x74, 0x52, 0x02, 0xE8, 0xF7, 0xA2, 0x2B, 0x00, 0xBF, 0x8C, 0x2E, 0xDE, 0x00, 0xBE, 0x94,
    0x2E, 0xDE, 0x00, 0xA3, 0x1C, 0x74, 0x52, 0x02, 0xE8, 0xE3, 0xA2, 0x2B, 0x00, 0xBF, 0xA0, 0x2E,
    0xDE, 0x00, 0xBE, 0xA8, 0x2E, 0xDE, 0x00, 0xA3, 0x20, 0x74, 0x52, 0x02, 0xE8, 0xCF, 0xA2, 0x2B,
    0x00, 0xBF, 0xB4, 0x2E, 0xDE, 0x00, 0xBE, 0xBC, 0x2E, 0xDE, 0x00, 0xA3, 0x24, 0x74, 0x52, 0x02,
    0xE8, 0xBB, 0xA2, 0x2B, 0x00, 0x5F, 0xA3, 0x28, 0x74, 0x52, 0x02, 0x5E, 0xC3,
];
const ACT_LEVEL_TEXT_CALLS: [usize; 6] = [0xc, 0x20, 0x34, 0x48, 0x5c, 0x70];
#[cfg(test)]
const ACT_LEVEL_TEXT_ABS: [usize; 18] = [3, 8, 18, 23, 28, 38, 43, 48, 58, 63, 68, 78, 83, 88, 98, 103, 108, 119];
#[cfg(test)]
const ACT_LEVEL_TEXT_JMPS: [usize; 0] = [];
const ID_ACT_LEVEL_TEXT: usize = 64;
hot_thunk!(thunk_act_level_text, port_act_level_text, "ret");
unsafe extern "C" fn port_act_level_text(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_ACT_LEVEL_TEXT);
        const T: [(u32, u32, usize); 6] = [
            (0x00DE_2E50, 0x00DE_2E58, 0x0252_7414),
            (0x00DE_2E64, 0x00DE_2E6C, 0x0252_7418),
            (0x00DE_2E78, 0x00DE_2E80, 0x0252_741C),
            (0x00DE_2E8C, 0x00DE_2E94, 0x0252_7420),
            (0x00DE_2EA0, 0x00DE_2EA8, 0x0252_7424),
            (0x00DE_2EB4, 0x00DE_2EBC, 0x0252_7428),
        ];
        let mut eax = r.eax;
        for (edi, esi, dst) in T {
            eax = call_regs(0x007F_4AD0, eax, r.ecx, r.edx, esi, edi, &[]);
            wd(dst, eax);
        }
        r.eax = eax;
    }
}

// ---- cutscene_object_is_cutscene_vehicle @ 0x6c6ee0 (106 bytes): count ----
const CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_BODY: [u8; 106] = [
    0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x56, 0x74, 0x58, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0x33,
    0xF6, 0x39, 0xB0, 0xE8, 0x0A, 0x00, 0x00, 0x7E, 0x49, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x80, 0xE0, 0x0A, 0x00, 0x00, 0x8B, 0x0C, 0xB0, 0x8B, 0x11, 0x8B, 0x42, 0x34, 0xFF, 0xD0,
    0x85, 0xC0, 0x74, 0x1E, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0x8B, 0x91, 0xE0, 0x0A, 0x00, 0x00,
    0x8B, 0x0C, 0xB2, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x8B, 0x4F, 0x44, 0x3B, 0x48, 0x44,
    0x74, 0x14, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0x83, 0xC6, 0x01, 0x3B, 0xB0, 0xE8, 0x0A, 0x00, 0x00,
    0x7C, 0xBE, 0x32, 0xC0, 0x5E, 0xC3, 0xB0, 0x01, 0x5E, 0xC3,
];
const CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_CALLS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_ABS: [usize; 4] = [2, 11, 54, 83];
#[cfg(test)]
const CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_JMPS: [usize; 0] = [];
const ID_CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE: usize = 65;
hot_thunk!(thunk_cutscene_object_is_cutscene_vehicle, port_cutscene_object_is_cutscene_vehicle, "ret");
unsafe extern "C" fn port_cutscene_object_is_cutscene_vehicle(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE);
        let mut eax = r.eax;
        if rb(0x0252_7D14) == 0 {
            r.eax = eax & !0xff;
            return;
        }
        eax = rd(0x0252_7D10);
        let mut esi = 0u32;
        if (rd(eax as usize + 0xae8) as i32) <= 0 {
            r.eax = eax & !0xff;
            return;
        }
        loop {
            let arr = rd(eax as usize + 0xae0);
            let ecx = rd(arr as usize + esi as usize * 4);
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x34);
            eax = call_regs(f as usize, f, ecx, vt, esi, r.edi, &[]);
            if eax != 0 {
                let c = rd(0x0252_7D10) as usize;
                let arr = rd(c + 0xae0);
                let ecx = rd(arr as usize + esi as usize * 4);
                let vt = rd(ecx as usize);
                let f = rd(vt as usize + 0x34);
                eax = call_regs(f as usize, vt, ecx, f, esi, r.edi, &[]);
                if rd(r.edi as usize + 0x44) == rd(eax as usize + 0x44) {
                    r.eax = (eax & !0xff) | 1;
                    return;
                }
            }
            eax = rd(0x0252_7D10);
            esi += 1;
            if (esi as i32) >= rd(eax as usize + 0xae8) as i32 {
                break;
            }
        }
        r.eax = eax & !0xff;
    }
}

// ---- camera_ctx_pred_zoom @ 0x49a870 (33 bytes): pure ----
const CAMERA_CTX_PRED_ZOOM_BODY: [u8; 33] = [
    0x85, 0xC9, 0x74, 0x1A, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x39, 0x41, 0x4C, 0x75, 0x10, 0x38, 0x81,
    0xCC, 0x03, 0x00, 0x00, 0x75, 0x08, 0x84, 0x81, 0x61, 0x11, 0x00, 0x00, 0x75, 0x02, 0x32, 0xC0,
    0xC3,
];
const CAMERA_CTX_PRED_ZOOM_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_ZOOM_ABS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_ZOOM_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_ZOOM: usize = 66;
hot_thunk!(thunk_camera_ctx_pred_zoom, port_camera_ctx_pred_zoom, "ret");
unsafe extern "C" fn port_camera_ctx_pred_zoom(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.ecx as usize;
        let v = if h == 0 {
            r.eax & !0xff
        } else {
            (rd(h + 0x4c) == 1 && rb(h + 0x3cc) == 1 && rb(h + 0x1161) & 1 != 0) as u32
        };
        r.eax = pure_done(ID_CAMERA_CTX_PRED_ZOOM, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_fine_aim @ 0x49a920 (34 bytes): pure ----
const CAMERA_CTX_PRED_FINE_AIM_BODY: [u8; 34] = [
    0x85, 0xC9, 0x74, 0x1B, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x39, 0x41, 0x4C, 0x75, 0x11, 0x38, 0x81,
    0xCC, 0x03, 0x00, 0x00, 0x75, 0x09, 0xF6, 0x81, 0x63, 0x11, 0x00, 0x00, 0x08, 0x75, 0x02, 0x32,
    0xC0, 0xC3,
];
const CAMERA_CTX_PRED_FINE_AIM_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FINE_AIM_ABS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FINE_AIM_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_FINE_AIM: usize = 67;
hot_thunk!(thunk_camera_ctx_pred_fine_aim, port_camera_ctx_pred_fine_aim, "ret");
unsafe extern "C" fn port_camera_ctx_pred_fine_aim(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.ecx as usize;
        let v = if h == 0 {
            r.eax & !0xff
        } else {
            (rd(h + 0x4c) == 1 && rb(h + 0x3cc) == 1 && rb(h + 0x1163) & 8 != 0) as u32
        };
        r.eax = pure_done(ID_CAMERA_CTX_PRED_FINE_AIM, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_melee_lock @ 0x49a980 (34 bytes): pure ----
const CAMERA_CTX_PRED_MELEE_LOCK_BODY: [u8; 34] = [
    0x85, 0xC9, 0x74, 0x1B, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x39, 0x41, 0x4C, 0x75, 0x11, 0x38, 0x81,
    0xCC, 0x03, 0x00, 0x00, 0x75, 0x09, 0xF6, 0x81, 0x63, 0x11, 0x00, 0x00, 0x10, 0x75, 0x02, 0x32,
    0xC0, 0xC3,
];
const CAMERA_CTX_PRED_MELEE_LOCK_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_MELEE_LOCK_ABS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_MELEE_LOCK_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_MELEE_LOCK: usize = 68;
hot_thunk!(thunk_camera_ctx_pred_melee_lock, port_camera_ctx_pred_melee_lock, "ret");
unsafe extern "C" fn port_camera_ctx_pred_melee_lock(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.ecx as usize;
        let v = if h == 0 {
            r.eax & !0xff
        } else {
            (rd(h + 0x4c) == 1 && rb(h + 0x3cc) == 1 && rb(h + 0x1163) & 0x10 != 0) as u32
        };
        r.eax = pure_done(ID_CAMERA_CTX_PRED_MELEE_LOCK, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_fine_aim_crouch @ 0x49a950 (47 bytes): pure ----
const CAMERA_CTX_PRED_FINE_AIM_CROUCH_BODY: [u8; 47] = [
    0x85, 0xC9, 0x74, 0x28, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x39, 0x41, 0x4C, 0x75, 0x1E, 0x38, 0x81,
    0xCC, 0x03, 0x00, 0x00, 0x75, 0x16, 0xF6, 0x81, 0x63, 0x11, 0x00, 0x00, 0x08, 0x74, 0x0D, 0x8B,
    0x89, 0xC0, 0x00, 0x00, 0x00, 0xC1, 0xE9, 0x07, 0x84, 0xC8, 0x75, 0x02, 0x32, 0xC0, 0xC3,
];
const CAMERA_CTX_PRED_FINE_AIM_CROUCH_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FINE_AIM_CROUCH_ABS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FINE_AIM_CROUCH_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_FINE_AIM_CROUCH: usize = 69;
hot_thunk!(thunk_camera_ctx_pred_fine_aim_crouch, port_camera_ctx_pred_fine_aim_crouch, "ret");
unsafe extern "C" fn port_camera_ctx_pred_fine_aim_crouch(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.ecx as usize;
        let v = if h == 0 {
            r.eax & !0xff
        } else {
            (rd(h + 0x4c) == 1 && rb(h + 0x3cc) == 1 && rb(h + 0x1163) & 8 != 0 && (rd(h + 0xc0) >> 7) & 1 != 0) as u32
        };
        r.eax = pure_done(ID_CAMERA_CTX_PRED_FINE_AIM_CROUCH, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_freefall @ 0x49aa80 (45 bytes): pure ----
const CAMERA_CTX_PRED_FREEFALL_BODY: [u8; 45] = [
    0x8B, 0x80, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0xC8, 0xC1, 0xE9, 0x1D, 0xF6, 0xC1, 0x01, 0x74, 0x0D,
    0xC1, 0xE8, 0x1E, 0xA8, 0x01, 0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
    0x8B, 0x80, 0xC0, 0x00, 0x00, 0x00, 0xC1, 0xE8, 0x1E, 0x83, 0xE0, 0x01, 0xC3,
];
const CAMERA_CTX_PRED_FREEFALL_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FREEFALL_ABS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FREEFALL_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_FREEFALL: usize = 70;
hot_thunk!(thunk_camera_ctx_pred_freefall, port_camera_ctx_pred_freefall, "ret");
unsafe extern "C" fn port_camera_ctx_pred_freefall(r: &mut Pushad, a: *const u32) {

    unsafe {
        let f = rd(r.eax as usize + 0xc0);
        let v = ((f >> 0x1d) & 1 != 0 && (f >> 0x1e) & 1 == 0) as u32;
        r.eax = pure_done(ID_CAMERA_CTX_PRED_FREEFALL, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_falling @ 0x49a8c0 (94 bytes): pure ----
const CAMERA_CTX_PRED_FALLING_BODY: [u8; 94] = [
    0x8B, 0x82, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x80, 0x64, 0x01, 0x00, 0x00, 0x83, 0xF8, 0xFF, 0x74,
    0x2C, 0x83, 0xF8, 0x39, 0x8D, 0x0C, 0x40, 0x8B, 0x0C, 0x8D, 0x44, 0x0E, 0xE9, 0x00, 0x7C, 0x05,
    0x83, 0xF8, 0x3E, 0x7E, 0x36, 0x81, 0xF9, 0x3B, 0x05, 0x00, 0x00, 0x74, 0x2E, 0x81, 0xF9, 0x3C,
    0x05, 0x00, 0x00, 0x7E, 0x08, 0x81, 0xF9, 0x3E, 0x05, 0x00, 0x00, 0x7E, 0x1E, 0x80, 0xBA, 0x91,
    0x05, 0x00, 0x00, 0x06, 0x75, 0x12, 0xF7, 0x82, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00,
    0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3, 0x32, 0xC0, 0xC3,
];
const CAMERA_CTX_PRED_FALLING_CALLS: [usize; 0] = [];
#[cfg(test)]
const CAMERA_CTX_PRED_FALLING_ABS: [usize; 1] = [26];
#[cfg(test)]
const CAMERA_CTX_PRED_FALLING_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_FALLING: usize = 71;
hot_thunk!(thunk_camera_ctx_pred_falling, port_camera_ctx_pred_falling, "ret");
unsafe extern "C" fn port_camera_ctx_pred_falling(r: &mut Pushad, a: *const u32) {

    unsafe {
        let h = r.edx as usize;
        let rig = rd(h + 0x5e0) as usize;
        let eax = rd(rig + 0x164);
        let fd = |h: usize| -> u32 { (rb(h + 0x591) == 6 && rd(h + 0xc8) & 0x1000 == 0) as u32 };
        let v = if eax == 0xFFFF_FFFF {
            fd(h)
        } else {
            let ecx = rd(0x00E9_0E44usize.wrapping_add((eax.wrapping_mul(3) as usize).wrapping_mul(4))) as i32;
            let l_e5 = if (eax as i32) < 0x39 { true } else { (eax as i32) > 0x3e };
            if !l_e5 {
                eax & !0xff
            } else if ecx == 0x53b {
                eax & !0xff
            } else if ecx <= 0x53c {
                fd(h)
            } else if ecx <= 0x53e {
                eax & !0xff
            } else {
                fd(h)
            }
        };
        r.eax = pure_done(ID_CAMERA_CTX_PRED_FALLING, r, a, 0, v);
    }
}

// ---- camera_ctx_pred_sprint @ 0x49aa00 (55 bytes): count ----
const CAMERA_CTX_PRED_SPRINT_BODY: [u8; 55] = [
    0x56, 0x8B, 0xF0, 0xE8, 0x18, 0x57, 0x4C, 0x00, 0x84, 0xC0, 0x74, 0x27, 0x83, 0xBE, 0xA8, 0x0E,
    0x00, 0x00, 0x00, 0x75, 0x1E, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x85, 0xC0, 0x74, 0x0E, 0x57, 0x8D,
    0x78, 0x14, 0xE8, 0x09, 0x0A, 0x01, 0x00, 0x84, 0xC0, 0x5F, 0x75, 0x07, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0x5E, 0xC3, 0x33, 0xC0, 0x5E, 0xC3,
];
const CAMERA_CTX_PRED_SPRINT_CALLS: [usize; 2] = [0x3, 0x22];
#[cfg(test)]
const CAMERA_CTX_PRED_SPRINT_ABS: [usize; 1] = [22];
#[cfg(test)]
const CAMERA_CTX_PRED_SPRINT_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_SPRINT: usize = 72;
hot_thunk!(thunk_camera_ctx_pred_sprint, port_camera_ctx_pred_sprint, "ret");
unsafe extern "C" fn port_camera_ctx_pred_sprint(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CAMERA_CTX_PRED_SPRINT);
        let h = r.eax;
        let c = call_regs(0x0096_0120, h, r.ecx, r.edx, h, r.edi, &[]);
        if (c & 0xff) == 0 || rd(h as usize + 0xea8) != 0 {
            r.eax = 0;
            return;
        }
        let p = rd(0x0217_03D4);
        if p != 0 {
            let q = call_regs(0x004A_B430, p, r.ecx, r.edx, h, p.wrapping_add(0x14), &[]);
            if (q & 0xff) != 0 {
                r.eax = 0;
                return;
            }
        }
        r.eax = 1;
    }
}

// ---- camera_ctx_pred_interior_sprint @ 0x49aa40 (55 bytes): count ----
const CAMERA_CTX_PRED_INTERIOR_SPRINT_BODY: [u8; 55] = [
    0x56, 0x8B, 0xF0, 0xE8, 0xD8, 0x56, 0x4C, 0x00, 0x84, 0xC0, 0x74, 0x27, 0x83, 0xBE, 0xA8, 0x0E,
    0x00, 0x00, 0x00, 0x75, 0x1E, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x85, 0xC0, 0x74, 0x15, 0x57, 0x8D,
    0x78, 0x14, 0xE8, 0xC9, 0x09, 0x01, 0x00, 0x84, 0xC0, 0x5F, 0x74, 0x07, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0x5E, 0xC3, 0x33, 0xC0, 0x5E, 0xC3,
];
const CAMERA_CTX_PRED_INTERIOR_SPRINT_CALLS: [usize; 2] = [0x3, 0x22];
#[cfg(test)]
const CAMERA_CTX_PRED_INTERIOR_SPRINT_ABS: [usize; 1] = [22];
#[cfg(test)]
const CAMERA_CTX_PRED_INTERIOR_SPRINT_JMPS: [usize; 0] = [];
const ID_CAMERA_CTX_PRED_INTERIOR_SPRINT: usize = 73;
hot_thunk!(thunk_camera_ctx_pred_interior_sprint, port_camera_ctx_pred_interior_sprint, "ret");
unsafe extern "C" fn port_camera_ctx_pred_interior_sprint(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CAMERA_CTX_PRED_INTERIOR_SPRINT);
        let h = r.eax;
        let c = call_regs(0x0096_0120, h, r.ecx, r.edx, h, r.edi, &[]);
        if (c & 0xff) == 0 || rd(h as usize + 0xea8) != 0 {
            r.eax = 0;
            return;
        }
        let p = rd(0x0217_03D4);
        if p == 0 {
            r.eax = 0;
            return;
        }
        let q = call_regs(0x004A_B430, p, r.ecx, r.edx, h, p.wrapping_add(0x14), &[]);
        r.eax = ((q & 0xff) != 0) as u32;
    }
}

// ---- audio_job_workers_mode_reset @ 0x464fe0 (43 bytes): count ----
const AUDIO_JOB_WORKERS_MODE_RESET_BODY: [u8; 43] = [
    0x56, 0xBE, 0xF8, 0x63, 0x3D, 0x03, 0xC7, 0x05, 0x30, 0x64, 0x3D, 0x03, 0x03, 0x00, 0x00, 0x00,
    0xE8, 0x6B, 0xE7, 0x78, 0x00, 0xBE, 0xC8, 0x64, 0x3D, 0x03, 0xC7, 0x05, 0x00, 0x65, 0x3D, 0x03,
    0x03, 0x00, 0x00, 0x00, 0xE8, 0x57, 0xE7, 0x78, 0x00, 0x5E, 0xC3,
];
const AUDIO_JOB_WORKERS_MODE_RESET_CALLS: [usize; 2] = [0x10, 0x24];
#[cfg(test)]
const AUDIO_JOB_WORKERS_MODE_RESET_ABS: [usize; 4] = [2, 8, 22, 28];
#[cfg(test)]
const AUDIO_JOB_WORKERS_MODE_RESET_JMPS: [usize; 0] = [];
const ID_AUDIO_JOB_WORKERS_MODE_RESET: usize = 74;
hot_thunk!(thunk_audio_job_workers_mode_reset, port_audio_job_workers_mode_reset, "ret");
unsafe extern "C" fn port_audio_job_workers_mode_reset(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_AUDIO_JOB_WORKERS_MODE_RESET);
        wd(0x033D_6430, 3);
        call_regs(0x00BF_3760, r.eax, r.ecx, r.edx, 0x033D_63F8, r.edi, &[]);
        wd(0x033D_6500, 3);
        r.eax = call_regs(0x00BF_3760, r.eax, r.ecx, r.edx, 0x033D_64C8, r.edi, &[]);
    }
}

// ---- havok_prestep_then_raycast @ 0x6f5d40 (25 bytes): count ----
const HAVOK_PRESTEP_THEN_RAYCAST_BODY: [u8; 25] = [
    0xE8, 0x2B, 0x99, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x4C, 0x24, 0x08, 0x6A, 0x01, 0x50,
    0xE8, 0xDB, 0x45, 0x00, 0x00, 0x83, 0xC4, 0x08, 0xC3,
];
const HAVOK_PRESTEP_THEN_RAYCAST_CALLS: [usize; 2] = [0x0, 0x10];
#[cfg(test)]
const HAVOK_PRESTEP_THEN_RAYCAST_ABS: [usize; 0] = [];
#[cfg(test)]
const HAVOK_PRESTEP_THEN_RAYCAST_JMPS: [usize; 0] = [];
const ID_HAVOK_PRESTEP_THEN_RAYCAST: usize = 75;
hot_thunk!(thunk_havok_prestep_then_raycast, port_havok_prestep_then_raycast, "ret");
unsafe extern "C" fn port_havok_prestep_then_raycast(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HAVOK_PRESTEP_THEN_RAYCAST);
        call_regs(0x006F_F670, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        let (a1, a2) = (*a.add(1), *a.add(2));
        r.eax = call_regs(0x006F_A330, a2, a1, r.edx, r.esi, r.edi, &[a2, 1]);
    }
}

// ---- havok_entity_refresh_collision_filter @ 0x6fc720 (35 bytes): count ----
const HAVOK_ENTITY_REFRESH_COLLISION_FILTER_BODY: [u8; 35] = [
    0x51, 0x83, 0x3D, 0xAC, 0x7D, 0x52, 0x02, 0x00, 0x76, 0x07, 0xE8, 0x11, 0x90, 0xFF, 0xFF, 0x59,
    0xC3, 0xA1, 0x94, 0x7D, 0x52, 0x02, 0x6A, 0x01, 0x52, 0x50, 0x33, 0xC0, 0xE8, 0xFF, 0x04, 0x66,
    0x00, 0x59, 0xC3,
];
const HAVOK_ENTITY_REFRESH_COLLISION_FILTER_CALLS: [usize; 2] = [0xa, 0x1c];
#[cfg(test)]
const HAVOK_ENTITY_REFRESH_COLLISION_FILTER_ABS: [usize; 2] = [3, 18];
#[cfg(test)]
const HAVOK_ENTITY_REFRESH_COLLISION_FILTER_JMPS: [usize; 0] = [];
const ID_HAVOK_ENTITY_REFRESH_COLLISION_FILTER: usize = 76;
hot_thunk!(thunk_havok_entity_refresh_collision_filter, port_havok_entity_refresh_collision_filter, "ret");
unsafe extern "C" fn port_havok_entity_refresh_collision_filter(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_HAVOK_ENTITY_REFRESH_COLLISION_FILTER);
        if rd(0x0252_7DAC) > 0 {
            hit(ID_HAVOK_ENTITY_REFRESH_COLLISION_FILTER, 0);
            r.eax = call_regs(0x006F_5740, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
            return;
        }
        hit(ID_HAVOK_ENTITY_REFRESH_COLLISION_FILTER, 1);
        let g = rd(0x0252_7D94);
        r.eax = call_regs(0x00D5_CC40, 0, r.ecx, r.edx, r.esi, r.edi, &[g, r.edx, 1]);
    }
}

// ---- video_stop @ 0x689130 (53 bytes): count ----
const VIDEO_STOP_BODY: [u8; 53] = [
    0x51, 0xE8, 0x9A, 0xF7, 0xFF, 0xFF, 0xE8, 0x35, 0x60, 0xDF, 0xFF, 0x80, 0x3D, 0x76, 0x7B, 0x52,
    0x02, 0x00, 0xC6, 0x05, 0x3E, 0x4D, 0xE9, 0x00, 0x01, 0x74, 0x18, 0x6A, 0x00, 0xE8, 0x2E, 0x9A,
    0xE9, 0xFF, 0x83, 0xC4, 0x04, 0xB0, 0x01, 0xE8, 0x64, 0x7B, 0x3F, 0x00, 0xC6, 0x05, 0x76, 0x7B,
    0x52, 0x02, 0x00, 0x59, 0xC3,
];
const VIDEO_STOP_CALLS: [usize; 4] = [0x1, 0x6, 0x1d, 0x27];
#[cfg(test)]
const VIDEO_STOP_ABS: [usize; 3] = [13, 20, 46];
#[cfg(test)]
const VIDEO_STOP_JMPS: [usize; 0] = [];
const ID_VIDEO_STOP: usize = 77;
hot_thunk!(thunk_video_stop, port_video_stop, "ret");
unsafe extern "C" fn port_video_stop(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_VIDEO_STOP);
        call_regs(0x0068_88D0, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        let mut eax = call_regs(0x0047_F170, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        let flag = rb(0x0252_7B76);
        wb(0x00E9_4D3E, 1);
        if flag != 0 {
            hit(ID_VIDEO_STOP, 0);
            eax = call_regs(0x0052_2B80, eax, r.ecx, r.edx, r.esi, r.edi, &[0]);
            eax = call_regs(0x00A8_0CC0, (eax & !0xff) | 1, r.ecx, r.edx, r.esi, r.edi, &[]);
            wb(0x0252_7B76, 0);
        }
        r.eax = eax;
    }
}

// ---- cutscene_action_door_open @ 0x6cc5a0 (50 bytes): count ----
const CUTSCENE_ACTION_DOOR_OPEN_BODY: [u8; 50] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x26, 0xE8, 0x30, 0xF1, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x1D, 0x8B, 0x88, 0x94, 0x00, 0x00, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0x51, 0xE8,
    0xCC, 0xEF, 0x24, 0x00, 0x83, 0xC4, 0x04, 0x8B, 0xC8, 0xE8, 0xB2, 0xE7, 0x24, 0x00, 0x83, 0xC4,
    0x08, 0xC3,
];
const CUTSCENE_ACTION_DOOR_OPEN_CALLS: [usize; 3] = [0xb, 0x1f, 0x29];
#[cfg(test)]
const CUTSCENE_ACTION_DOOR_OPEN_ABS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_ACTION_DOOR_OPEN_JMPS: [usize; 0] = [];
const ID_CUTSCENE_ACTION_DOOR_OPEN: usize = 78;
hot_thunk!(thunk_cutscene_action_door_open, port_cutscene_action_door_open, "ret");
unsafe extern "C" fn port_cutscene_action_door_open(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_ACTION_DOOR_OPEN);
        let act = *a as usize;
        let name = rd(act + 0x10);
        if name == 0 {
            r.eax = 0;
            return;
        }
        let door = call_regs(0x00A5_B6E0, name, r.ecx, r.edx, r.esi, r.edi, &[]);
        if door == 0 {
            r.eax = 0;
            return;
        }
        let ecx = rd(door as usize + 0x94);
        let e = call_regs(0x0091_B590, door, ecx, r.edx, r.esi, r.edi, &[ecx, 0, 1]);
        r.eax = call_regs(0x0091_AD80, e, e, r.edx, r.esi, r.edi, &[0, 1]);
    }
}

// ---- cutscene_action_door_close @ 0x6cc5e0 (43 bytes): count ----
const CUTSCENE_ACTION_DOOR_CLOSE_BODY: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x1F, 0xE8, 0xF0, 0xF0, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x16, 0x8B, 0x88, 0x94, 0x00, 0x00, 0x00, 0x51, 0xE8, 0x90, 0xEF, 0x24, 0x00,
    0x83, 0xC4, 0x04, 0x8B, 0xC8, 0xE9, 0x36, 0xEB, 0x24, 0x00, 0xC3,
];
const CUTSCENE_ACTION_DOOR_CLOSE_CALLS: [usize; 2] = [0xb, 0x1b];
#[cfg(test)]
const CUTSCENE_ACTION_DOOR_CLOSE_ABS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_ACTION_DOOR_CLOSE_JMPS: [usize; 1] = [37];
const ID_CUTSCENE_ACTION_DOOR_CLOSE: usize = 79;
hot_thunk!(thunk_cutscene_action_door_close, port_cutscene_action_door_close, "ret");
unsafe extern "C" fn port_cutscene_action_door_close(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_ACTION_DOOR_CLOSE);
        let act = *a as usize;
        let name = rd(act + 0x10);
        if name == 0 {
            r.eax = 0;
            return;
        }
        let door = call_regs(0x00A5_B6E0, name, r.ecx, r.edx, r.esi, r.edi, &[]);
        if door == 0 {
            r.eax = 0;
            return;
        }
        let ecx = rd(door as usize + 0x94);
        let e = call_regs(0x0091_B590, door, ecx, r.edx, r.esi, r.edi, &[ecx]);
        r.eax = call_regs(0x0091_B140, e, e, r.edx, r.esi, r.edi, &[*a]);
    }
}

// ---- cutscene_action_mesh_mover_play_update @ 0x6cc7a0 (43 bytes): count ----
const CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_BODY: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x1D, 0xE8, 0x30, 0xEF, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x14, 0x8B, 0x80, 0x94, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0A, 0x0F, 0xB6,
    0x40, 0x0A, 0xD1, 0xE8, 0x83, 0xE0, 0x01, 0xC3, 0xB0, 0x01, 0xC3,
];
const CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_CALLS: [usize; 1] = [0xb];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_ABS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_JMPS: [usize; 0] = [];
const ID_CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE: usize = 80;
hot_thunk!(thunk_cutscene_action_mesh_mover_play_update, port_cutscene_action_mesh_mover_play_update, "ret");
unsafe extern "C" fn port_cutscene_action_mesh_mover_play_update(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE);
        let name = rd(*a as usize + 8);
        if name == 0 {
            r.eax = 1;
            return;
        }
        let door = call_regs(0x00A5_B6E0, name, r.ecx, r.edx, r.esi, r.edi, &[]);
        if door == 0 {
            r.eax = 1;
            return;
        }
        let p = rd(door as usize + 0x94);
        if p == 0 {
            r.eax = 1;
            return;
        }
        r.eax = (rb(p as usize + 0xa) as u32 >> 1) & 1;
    }
}

// ---- cutscene_action_mesh_mover_stop @ 0x6cc7d0 (28 bytes): count ----
const CUTSCENE_ACTION_MESH_MOVER_STOP_BODY: [u8; 28] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x10, 0xE8, 0x00, 0xEF, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x07, 0x8B, 0xC8, 0xE9, 0x55, 0xF7, 0x38, 0x00, 0xC3,
];
const CUTSCENE_ACTION_MESH_MOVER_STOP_CALLS: [usize; 1] = [0xb];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_STOP_ABS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_STOP_JMPS: [usize; 1] = [22];
const ID_CUTSCENE_ACTION_MESH_MOVER_STOP: usize = 81;
hot_thunk!(thunk_cutscene_action_mesh_mover_stop, port_cutscene_action_mesh_mover_stop, "ret");
unsafe extern "C" fn port_cutscene_action_mesh_mover_stop(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_ACTION_MESH_MOVER_STOP);
        let name = rd(*a as usize + 8);
        if name == 0 {
            r.eax = 0;
            return;
        }
        let door = call_regs(0x00A5_B6E0, name, r.ecx, r.edx, r.esi, r.edi, &[]);
        if door == 0 {
            r.eax = 0;
            return;
        }
        r.eax = call_regs(0x00A5_BF40, door, door, r.edx, r.esi, r.edi, &[*a]);
    }
}

// ---- cutscene_action_mesh_mover_reset @ 0x6cc7f0 (30 bytes): count ----
const CUTSCENE_ACTION_MESH_MOVER_RESET_BODY: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x12, 0xE8, 0xE0, 0xEE, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x09, 0x8B, 0x10, 0x8B, 0xC8, 0x8B, 0x42, 0x24, 0xFF, 0xE0, 0xC3,
];
const CUTSCENE_ACTION_MESH_MOVER_RESET_CALLS: [usize; 1] = [0xb];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_RESET_ABS: [usize; 0] = [];
#[cfg(test)]
const CUTSCENE_ACTION_MESH_MOVER_RESET_JMPS: [usize; 0] = [];
const ID_CUTSCENE_ACTION_MESH_MOVER_RESET: usize = 82;
hot_thunk!(thunk_cutscene_action_mesh_mover_reset, port_cutscene_action_mesh_mover_reset, "ret");
unsafe extern "C" fn port_cutscene_action_mesh_mover_reset(r: &mut Pushad, a: *const u32) {

    unsafe {
        cnt(ID_CUTSCENE_ACTION_MESH_MOVER_RESET);
        let name = rd(*a as usize + 8);
        if name == 0 {
            r.eax = 0;
            return;
        }
        let door = call_regs(0x00A5_B6E0, name, r.ecx, r.edx, r.esi, r.edi, &[]);
        if door == 0 {
            r.eax = 0;
            return;
        }
        let vt = rd(door as usize);
        let f = rd(vt as usize + 0x24);
        r.eax = call_regs(f as usize, f, door, vt, r.esi, r.edi, &[*a]);
    }
}

const COUNT: usize = 83;
static PORTS: [Port; COUNT] = [
    port("game_event_23", 0x68f9a0, &GAME_EVENT_23_BODY, &GAME_EVENT_23_CALLS, 0),
    port("game_event_24", 0x68f9c0, &GAME_EVENT_24_BODY, &GAME_EVENT_24_CALLS, 0),
    port("game_event_25", 0x68f9e0, &GAME_EVENT_25_BODY, &GAME_EVENT_25_CALLS, 0),
    port("game_event_26", 0x68fb10, &GAME_EVENT_26_BODY, &GAME_EVENT_26_CALLS, 0),
    port("game_event_3", 0x68f650, &GAME_EVENT_3_BODY, &GAME_EVENT_3_CALLS, 0),
    port("game_event_20", 0x68f910, &GAME_EVENT_20_BODY, &GAME_EVENT_20_CALLS, 0),
    port("game_event_21", 0x68f940, &GAME_EVENT_21_BODY, &GAME_EVENT_21_CALLS, 0),
    port("game_event_22", 0x68f970, &GAME_EVENT_22_BODY, &GAME_EVENT_22_CALLS, 0),
    port("game_event_18", 0x68fae0, &GAME_EVENT_18_BODY, &GAME_EVENT_18_CALLS, 0),
    port("game_event_16", 0x68fa00, &GAME_EVENT_16_BODY, &GAME_EVENT_16_CALLS, 0),
    port("game_event_player_joined", 0x68f680, &GAME_EVENT_PLAYER_JOINED_BODY, &GAME_EVENT_PLAYER_JOINED_CALLS, 0),
    port("game_event_player_left", 0x68f6b0, &GAME_EVENT_PLAYER_LEFT_BODY, &GAME_EVENT_PLAYER_LEFT_CALLS, 0),
    port("diversions_on_game_event_18", 0x604970, &DIVERSIONS_ON_GAME_EVENT_18_BODY, &DIVERSIONS_ON_GAME_EVENT_18_CALLS, 0),
    port("diversions_on_game_event_20", 0x604680, &DIVERSIONS_ON_GAME_EVENT_20_BODY, &DIVERSIONS_ON_GAME_EVENT_20_CALLS, 0),
    port("diversions_on_game_event_21", 0x6046b0, &DIVERSIONS_ON_GAME_EVENT_21_BODY, &DIVERSIONS_ON_GAME_EVENT_21_CALLS, 0),
    port("diversions_on_game_event_22", 0x6046e0, &DIVERSIONS_ON_GAME_EVENT_22_BODY, &DIVERSIONS_ON_GAME_EVENT_22_CALLS, 0),
    port("diversions_on_object_damaged", 0x604740, &DIVERSIONS_ON_OBJECT_DAMAGED_BODY, &DIVERSIONS_ON_OBJECT_DAMAGED_CALLS, 0),
    port("diversions_on_vehicle_destroyed", 0x6047f0, &DIVERSIONS_ON_VEHICLE_DESTROYED_BODY, &DIVERSIONS_ON_VEHICLE_DESTROYED_CALLS, 0),
    port("diversions_on_object_killed", 0x604840, &DIVERSIONS_ON_OBJECT_KILLED_BODY, &DIVERSIONS_ON_OBJECT_KILLED_CALLS, 0),
    port("diversions_on_game_event_15", 0x604630, &DIVERSIONS_ON_GAME_EVENT_15_BODY, &DIVERSIONS_ON_GAME_EVENT_15_CALLS, 0),
    port("game_event_15", 0x68f8c0, &GAME_EVENT_15_BODY, &GAME_EVENT_15_CALLS, 0),
    port("gfx_ring_has_data", 0xcf5940, &GFX_RING_HAS_DATA_BODY, &GFX_RING_HAS_DATA_CALLS, 1),
    port("action_id_flags2_test", 0x6ef730, &ACTION_ID_FLAGS2_TEST_BODY, &ACTION_ID_FLAGS2_TEST_CALLS, 1),
    port("cutscene_player_flag_b01", 0x6c6d70, &CUTSCENE_PLAYER_FLAG_B01_BODY, &CUTSCENE_PLAYER_FLAG_B01_CALLS, 1),
    port("cutscene_running", 0x6c6d50, &CUTSCENE_RUNNING_BODY, &CUTSCENE_RUNNING_CALLS, 1),
    port("human_party_leader", 0x98e710, &HUMAN_PARTY_LEADER_BODY, &HUMAN_PARTY_LEADER_CALLS, 1),
    port("pcu_item_find_material_by_id", 0x7c1bd0, &PCU_ITEM_FIND_MATERIAL_BY_ID_BODY, &PCU_ITEM_FIND_MATERIAL_BY_ID_CALLS, 1),
    port("gfx_ring_read_u32", 0xbd5550, &GFX_RING_READ_U32_BODY, &GFX_RING_READ_U32_CALLS, 2),
    port("idirect3ddevice9lost_static_init", 0xcf58e0, &IDIRECT3DDEVICE9LOST_STATIC_INIT_BODY, &IDIRECT3DDEVICE9LOST_STATIC_INIT_CALLS, 2),
    port("human_flag_c8_stamp_set", 0x964560, &HUMAN_FLAG_C8_STAMP_SET_BODY, &HUMAN_FLAG_C8_STAMP_SET_CALLS, 2),
    port("nav_search_begin", 0xa0ffa0, &NAV_SEARCH_BEGIN_BODY, &NAV_SEARCH_BEGIN_CALLS, 2),
    port("havok_removal_queue_push", 0x6f5cd0, &HAVOK_REMOVAL_QUEUE_PUSH_BODY, &HAVOK_REMOVAL_QUEUE_PUSH_CALLS, 0),
    port("traffic_record_activate", 0x938c30, &TRAFFIC_RECORD_ACTIVATE_BODY, &TRAFFIC_RECORD_ACTIVATE_CALLS, 0),
    port("traffic_record_release", 0x938cb0, &TRAFFIC_RECORD_RELEASE_BODY, &TRAFFIC_RECORD_RELEASE_CALLS, 0),
    port("traffic_record_release_by_vehicle", 0x93d910, &TRAFFIC_RECORD_RELEASE_BY_VEHICLE_BODY, &TRAFFIC_RECORD_RELEASE_BY_VEHICLE_CALLS, 0),
    port("tod_object_node_alloc", 0x4d27e0, &TOD_OBJECT_NODE_ALLOC_BODY, &TOD_OBJECT_NODE_ALLOC_CALLS, 0),
    port("ptr_array_compact", 0x523b80, &PTR_ARRAY_COMPACT_BODY, &PTR_ARRAY_COMPACT_CALLS, 0),
    port("cutscene_seen_flags_store_bits", 0x6c3ad0, &CUTSCENE_SEEN_FLAGS_STORE_BITS_BODY, &CUTSCENE_SEEN_FLAGS_STORE_BITS_CALLS, 0),
    port("ambient_vehicle_budget_remaining", 0x93c1d0, &AMBIENT_VEHICLE_BUDGET_REMAINING_BODY, &AMBIENT_VEHICLE_BUDGET_REMAINING_CALLS, 0),
    port("current_spawn_group_query", 0x957660, &CURRENT_SPAWN_GROUP_QUERY_BODY, &CURRENT_SPAWN_GROUP_QUERY_CALLS, 0),
    port("current_spawn_group_vehicle_index", 0x9392a0, &CURRENT_SPAWN_GROUP_VEHICLE_INDEX_BODY, &CURRENT_SPAWN_GROUP_VEHICLE_INDEX_CALLS, 0),
    port("human_is_non_ally_state7", 0x9636c0, &HUMAN_IS_NON_ALLY_STATE7_BODY, &HUMAN_IS_NON_ALLY_STATE7_CALLS, 0),
    port("human_is_follower", 0x98e880, &HUMAN_IS_FOLLOWER_BODY, &HUMAN_IS_FOLLOWER_CALLS, 0),
    port("human_action_flag_bit4", 0x96d930, &HUMAN_ACTION_FLAG_BIT4_BODY, &HUMAN_ACTION_FLAG_BIT4_CALLS, 0),
    port("weather_force_stage_and_sync", 0x4ded80, &WEATHER_FORCE_STAGE_AND_SYNC_BODY, &WEATHER_FORCE_STAGE_AND_SYNC_CALLS, 0),
    port("input_actions_reset", 0x500170, &INPUT_ACTIONS_RESET_BODY, &INPUT_ACTIONS_RESET_CALLS, 0),
    port("render_list0_add", 0x52fda0, &RENDER_LIST0_ADD_BODY, &RENDER_LIST0_ADD_CALLS, 0),
    port("render_list0b_add", 0x52fe10, &RENDER_LIST0B_ADD_BODY, &RENDER_LIST0B_ADD_CALLS, 0),
    port("render_list_vis_add", 0x52fe80, &RENDER_LIST_VIS_ADD_BODY, &RENDER_LIST_VIS_ADD_CALLS, 0),
    port("render_list3_add", 0x52ff60, &RENDER_LIST3_ADD_BODY, &RENDER_LIST3_ADD_CALLS, 0),
    port("render_list4_add", 0x52ffd0, &RENDER_LIST4_ADD_BODY, &RENDER_LIST4_ADD_CALLS, 0),
    port("render_list6_add", 0x530030, &RENDER_LIST6_ADD_BODY, &RENDER_LIST6_ADD_CALLS, 0),
    port("render_list7_add", 0x5300a0, &RENDER_LIST7_ADD_BODY, &RENDER_LIST7_ADD_CALLS, 0),
    port("render_list_small_add", 0x52fef0, &RENDER_LIST_SMALL_ADD_BODY, &RENDER_LIST_SMALL_ADD_CALLS, 0),
    port("unlockable_parse_weapon_primary", 0x6bb3e0, &UNLOCKABLE_PARSE_WEAPON_PRIMARY_BODY, &UNLOCKABLE_PARSE_WEAPON_PRIMARY_CALLS, 0),
    port("unlockable_parse_weapon_secondary", 0x6bb420, &UNLOCKABLE_PARSE_WEAPON_SECONDARY_BODY, &UNLOCKABLE_PARSE_WEAPON_SECONDARY_CALLS, 0),
    port("unlockable_parse_contact", 0x6bb390, &UNLOCKABLE_PARSE_CONTACT_BODY, &UNLOCKABLE_PARSE_CONTACT_CALLS, 0),
    port("unlockable_parse_crib_ref", 0x6bba90, &UNLOCKABLE_PARSE_CRIB_REF_BODY, &UNLOCKABLE_PARSE_CRIB_REF_CALLS, 0),
    port("unlockable_parse_gang_type", 0x6bbb40, &UNLOCKABLE_PARSE_GANG_TYPE_BODY, &UNLOCKABLE_PARSE_GANG_TYPE_CALLS, 0),
    port("unlockable_parse_vehicle_group", 0x6bbb80, &UNLOCKABLE_PARSE_VEHICLE_GROUP_BODY, &UNLOCKABLE_PARSE_VEHICLE_GROUP_CALLS, 0),
    port("unlockable_parse_style", 0x6bbbc0, &UNLOCKABLE_PARSE_STYLE_BODY, &UNLOCKABLE_PARSE_STYLE_CALLS, 0),
    port("unlockable_parse_item", 0x6bb460, &UNLOCKABLE_PARSE_ITEM_BODY, &UNLOCKABLE_PARSE_ITEM_CALLS, 0),
    port("act_heli_text", 0x576e60, &ACT_HELI_TEXT_BODY, &ACT_HELI_TEXT_CALLS, 0),
    port("act_holdup_start_msg_text", 0x655700, &ACT_HOLDUP_START_MSG_TEXT_BODY, &ACT_HOLDUP_START_MSG_TEXT_CALLS, 0),
    port("act_level_text", 0x53a7a0, &ACT_LEVEL_TEXT_BODY, &ACT_LEVEL_TEXT_CALLS, 0),
    port("cutscene_object_is_cutscene_vehicle", 0x6c6ee0, &CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_BODY, &CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_CALLS, 0),
    port("camera_ctx_pred_zoom", 0x49a870, &CAMERA_CTX_PRED_ZOOM_BODY, &CAMERA_CTX_PRED_ZOOM_CALLS, 1),
    port("camera_ctx_pred_fine_aim", 0x49a920, &CAMERA_CTX_PRED_FINE_AIM_BODY, &CAMERA_CTX_PRED_FINE_AIM_CALLS, 1),
    port("camera_ctx_pred_melee_lock", 0x49a980, &CAMERA_CTX_PRED_MELEE_LOCK_BODY, &CAMERA_CTX_PRED_MELEE_LOCK_CALLS, 1),
    port("camera_ctx_pred_fine_aim_crouch", 0x49a950, &CAMERA_CTX_PRED_FINE_AIM_CROUCH_BODY, &CAMERA_CTX_PRED_FINE_AIM_CROUCH_CALLS, 1),
    port("camera_ctx_pred_freefall", 0x49aa80, &CAMERA_CTX_PRED_FREEFALL_BODY, &CAMERA_CTX_PRED_FREEFALL_CALLS, 1),
    port("camera_ctx_pred_falling", 0x49a8c0, &CAMERA_CTX_PRED_FALLING_BODY, &CAMERA_CTX_PRED_FALLING_CALLS, 1),
    port("camera_ctx_pred_sprint", 0x49aa00, &CAMERA_CTX_PRED_SPRINT_BODY, &CAMERA_CTX_PRED_SPRINT_CALLS, 0),
    port("camera_ctx_pred_interior_sprint", 0x49aa40, &CAMERA_CTX_PRED_INTERIOR_SPRINT_BODY, &CAMERA_CTX_PRED_INTERIOR_SPRINT_CALLS, 0),
    port("audio_job_workers_mode_reset", 0x464fe0, &AUDIO_JOB_WORKERS_MODE_RESET_BODY, &AUDIO_JOB_WORKERS_MODE_RESET_CALLS, 0),
    port("havok_prestep_then_raycast", 0x6f5d40, &HAVOK_PRESTEP_THEN_RAYCAST_BODY, &HAVOK_PRESTEP_THEN_RAYCAST_CALLS, 0),
    port("havok_entity_refresh_collision_filter", 0x6fc720, &HAVOK_ENTITY_REFRESH_COLLISION_FILTER_BODY, &HAVOK_ENTITY_REFRESH_COLLISION_FILTER_CALLS, 0),
    port("video_stop", 0x689130, &VIDEO_STOP_BODY, &VIDEO_STOP_CALLS, 0),
    port("cutscene_action_door_open", 0x6cc5a0, &CUTSCENE_ACTION_DOOR_OPEN_BODY, &CUTSCENE_ACTION_DOOR_OPEN_CALLS, 0),
    port("cutscene_action_door_close", 0x6cc5e0, &CUTSCENE_ACTION_DOOR_CLOSE_BODY, &CUTSCENE_ACTION_DOOR_CLOSE_CALLS, 0),
    port("cutscene_action_mesh_mover_play_update", 0x6cc7a0, &CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_BODY, &CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_CALLS, 0),
    port("cutscene_action_mesh_mover_stop", 0x6cc7d0, &CUTSCENE_ACTION_MESH_MOVER_STOP_BODY, &CUTSCENE_ACTION_MESH_MOVER_STOP_CALLS, 0),
    port("cutscene_action_mesh_mover_reset", 0x6cc7f0, &CUTSCENE_ACTION_MESH_MOVER_RESET_BODY, &CUTSCENE_ACTION_MESH_MOVER_RESET_CALLS, 0),
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, usize); COUNT] = [
        (ID_GAME_EVENT_23, thunk_game_event_23 as *const () as usize),
        (ID_GAME_EVENT_24, thunk_game_event_24 as *const () as usize),
        (ID_GAME_EVENT_25, thunk_game_event_25 as *const () as usize),
        (ID_GAME_EVENT_26, thunk_game_event_26 as *const () as usize),
        (ID_GAME_EVENT_3, thunk_game_event_3 as *const () as usize),
        (ID_GAME_EVENT_20, thunk_game_event_20 as *const () as usize),
        (ID_GAME_EVENT_21, thunk_game_event_21 as *const () as usize),
        (ID_GAME_EVENT_22, thunk_game_event_22 as *const () as usize),
        (ID_GAME_EVENT_18, thunk_game_event_18 as *const () as usize),
        (ID_GAME_EVENT_16, thunk_game_event_16 as *const () as usize),
        (ID_GAME_EVENT_PLAYER_JOINED, thunk_game_event_player_joined as *const () as usize),
        (ID_GAME_EVENT_PLAYER_LEFT, thunk_game_event_player_left as *const () as usize),
        (ID_DIVERSIONS_ON_GAME_EVENT_18, thunk_diversions_on_game_event_18 as *const () as usize),
        (ID_DIVERSIONS_ON_GAME_EVENT_20, thunk_diversions_on_game_event_20 as *const () as usize),
        (ID_DIVERSIONS_ON_GAME_EVENT_21, thunk_diversions_on_game_event_21 as *const () as usize),
        (ID_DIVERSIONS_ON_GAME_EVENT_22, thunk_diversions_on_game_event_22 as *const () as usize),
        (ID_DIVERSIONS_ON_OBJECT_DAMAGED, thunk_diversions_on_object_damaged as *const () as usize),
        (ID_DIVERSIONS_ON_VEHICLE_DESTROYED, thunk_diversions_on_vehicle_destroyed as *const () as usize),
        (ID_DIVERSIONS_ON_OBJECT_KILLED, thunk_diversions_on_object_killed as *const () as usize),
        (ID_DIVERSIONS_ON_GAME_EVENT_15, thunk_diversions_on_game_event_15 as *const () as usize),
        (ID_GAME_EVENT_15, thunk_game_event_15 as *const () as usize),
        (ID_GFX_RING_HAS_DATA, thunk_gfx_ring_has_data as *const () as usize),
        (ID_ACTION_ID_FLAGS2_TEST, thunk_action_id_flags2_test as *const () as usize),
        (ID_CUTSCENE_PLAYER_FLAG_B01, thunk_cutscene_player_flag_b01 as *const () as usize),
        (ID_CUTSCENE_RUNNING, thunk_cutscene_running as *const () as usize),
        (ID_HUMAN_PARTY_LEADER, thunk_human_party_leader as *const () as usize),
        (ID_PCU_ITEM_FIND_MATERIAL_BY_ID, thunk_pcu_item_find_material_by_id as *const () as usize),
        (ID_GFX_RING_READ_U32, thunk_gfx_ring_read_u32 as *const () as usize),
        (ID_IDIRECT3DDEVICE9LOST_STATIC_INIT, thunk_idirect3ddevice9lost_static_init as *const () as usize),
        (ID_HUMAN_FLAG_C8_STAMP_SET, thunk_human_flag_c8_stamp_set as *const () as usize),
        (ID_NAV_SEARCH_BEGIN, thunk_nav_search_begin as *const () as usize),
        (ID_HAVOK_REMOVAL_QUEUE_PUSH, thunk_havok_removal_queue_push as *const () as usize),
        (ID_TRAFFIC_RECORD_ACTIVATE, thunk_traffic_record_activate as *const () as usize),
        (ID_TRAFFIC_RECORD_RELEASE, thunk_traffic_record_release as *const () as usize),
        (ID_TRAFFIC_RECORD_RELEASE_BY_VEHICLE, thunk_traffic_record_release_by_vehicle as *const () as usize),
        (ID_TOD_OBJECT_NODE_ALLOC, thunk_tod_object_node_alloc as *const () as usize),
        (ID_PTR_ARRAY_COMPACT, thunk_ptr_array_compact as *const () as usize),
        (ID_CUTSCENE_SEEN_FLAGS_STORE_BITS, thunk_cutscene_seen_flags_store_bits as *const () as usize),
        (ID_AMBIENT_VEHICLE_BUDGET_REMAINING, thunk_ambient_vehicle_budget_remaining as *const () as usize),
        (ID_CURRENT_SPAWN_GROUP_QUERY, thunk_current_spawn_group_query as *const () as usize),
        (ID_CURRENT_SPAWN_GROUP_VEHICLE_INDEX, thunk_current_spawn_group_vehicle_index as *const () as usize),
        (ID_HUMAN_IS_NON_ALLY_STATE7, thunk_human_is_non_ally_state7 as *const () as usize),
        (ID_HUMAN_IS_FOLLOWER, thunk_human_is_follower as *const () as usize),
        (ID_HUMAN_ACTION_FLAG_BIT4, thunk_human_action_flag_bit4 as *const () as usize),
        (ID_WEATHER_FORCE_STAGE_AND_SYNC, thunk_weather_force_stage_and_sync as *const () as usize),
        (ID_INPUT_ACTIONS_RESET, thunk_input_actions_reset as *const () as usize),
        (ID_RENDER_LIST0_ADD, thunk_render_list0_add as *const () as usize),
        (ID_RENDER_LIST0B_ADD, thunk_render_list0b_add as *const () as usize),
        (ID_RENDER_LIST_VIS_ADD, thunk_render_list_vis_add as *const () as usize),
        (ID_RENDER_LIST3_ADD, thunk_render_list3_add as *const () as usize),
        (ID_RENDER_LIST4_ADD, thunk_render_list4_add as *const () as usize),
        (ID_RENDER_LIST6_ADD, thunk_render_list6_add as *const () as usize),
        (ID_RENDER_LIST7_ADD, thunk_render_list7_add as *const () as usize),
        (ID_RENDER_LIST_SMALL_ADD, thunk_render_list_small_add as *const () as usize),
        (ID_UNLOCKABLE_PARSE_WEAPON_PRIMARY, thunk_unlockable_parse_weapon_primary as *const () as usize),
        (ID_UNLOCKABLE_PARSE_WEAPON_SECONDARY, thunk_unlockable_parse_weapon_secondary as *const () as usize),
        (ID_UNLOCKABLE_PARSE_CONTACT, thunk_unlockable_parse_contact as *const () as usize),
        (ID_UNLOCKABLE_PARSE_CRIB_REF, thunk_unlockable_parse_crib_ref as *const () as usize),
        (ID_UNLOCKABLE_PARSE_GANG_TYPE, thunk_unlockable_parse_gang_type as *const () as usize),
        (ID_UNLOCKABLE_PARSE_VEHICLE_GROUP, thunk_unlockable_parse_vehicle_group as *const () as usize),
        (ID_UNLOCKABLE_PARSE_STYLE, thunk_unlockable_parse_style as *const () as usize),
        (ID_UNLOCKABLE_PARSE_ITEM, thunk_unlockable_parse_item as *const () as usize),
        (ID_ACT_HELI_TEXT, thunk_act_heli_text as *const () as usize),
        (ID_ACT_HOLDUP_START_MSG_TEXT, thunk_act_holdup_start_msg_text as *const () as usize),
        (ID_ACT_LEVEL_TEXT, thunk_act_level_text as *const () as usize),
        (ID_CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE, thunk_cutscene_object_is_cutscene_vehicle as *const () as usize),
        (ID_CAMERA_CTX_PRED_ZOOM, thunk_camera_ctx_pred_zoom as *const () as usize),
        (ID_CAMERA_CTX_PRED_FINE_AIM, thunk_camera_ctx_pred_fine_aim as *const () as usize),
        (ID_CAMERA_CTX_PRED_MELEE_LOCK, thunk_camera_ctx_pred_melee_lock as *const () as usize),
        (ID_CAMERA_CTX_PRED_FINE_AIM_CROUCH, thunk_camera_ctx_pred_fine_aim_crouch as *const () as usize),
        (ID_CAMERA_CTX_PRED_FREEFALL, thunk_camera_ctx_pred_freefall as *const () as usize),
        (ID_CAMERA_CTX_PRED_FALLING, thunk_camera_ctx_pred_falling as *const () as usize),
        (ID_CAMERA_CTX_PRED_SPRINT, thunk_camera_ctx_pred_sprint as *const () as usize),
        (ID_CAMERA_CTX_PRED_INTERIOR_SPRINT, thunk_camera_ctx_pred_interior_sprint as *const () as usize),
        (ID_AUDIO_JOB_WORKERS_MODE_RESET, thunk_audio_job_workers_mode_reset as *const () as usize),
        (ID_HAVOK_PRESTEP_THEN_RAYCAST, thunk_havok_prestep_then_raycast as *const () as usize),
        (ID_HAVOK_ENTITY_REFRESH_COLLISION_FILTER, thunk_havok_entity_refresh_collision_filter as *const () as usize),
        (ID_VIDEO_STOP, thunk_video_stop as *const () as usize),
        (ID_CUTSCENE_ACTION_DOOR_OPEN, thunk_cutscene_action_door_open as *const () as usize),
        (ID_CUTSCENE_ACTION_DOOR_CLOSE, thunk_cutscene_action_door_close as *const () as usize),
        (ID_CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE, thunk_cutscene_action_mesh_mover_play_update as *const () as usize),
        (ID_CUTSCENE_ACTION_MESH_MOVER_STOP, thunk_cutscene_action_mesh_mover_stop as *const () as usize),
        (ID_CUTSCENE_ACTION_MESH_MOVER_RESET, thunk_cutscene_action_mesh_mover_reset as *const () as usize),
    ];
    let mut ok = 0;
    for (id, thunk) in list {
        let p = &PORTS[id];
        if p.kind != 0 {
            // the shadow copy first: replacing patches the entry
            match unsafe { detour::relocate(p.addr, p.body, p.calls_at) } {
                Ok(copy) => p.original.store(copy, Ordering::Relaxed),
                Err(e) => log(&format!("port_small_game: {}: no shadow copy: {e:?}", p.name)),
            }
        }
        if replace_pinned(p.name, p.addr, p.body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_small_game: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log (shadowed ports count their first `SHADOW_CALLS` calls).
pub fn summary() -> String {
    let mut parts: Vec<String> = Vec::new();
    for (i, p) in PORTS.iter().enumerate() {
        let c = p.calls.load(Ordering::Relaxed);
        let b: Vec<u32> = (0..4).map(|k| BR[i * 4 + k].load(Ordering::Relaxed)).collect();
        if c == 0 && b.iter().all(|&x| x == 0) {
            continue;
        }
        if p.kind == 0 {
            parts.push(format!("{} {c} (branches {}/{}/{}/{})", p.name, b[0], b[1], b[2], b[3]));
        } else {
            parts.push(format!("{} {c} ({} shadow mismatches)", p.name, p.mismatches.load(Ordering::Relaxed)));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("port_small_game: {}", parts.join(", "))
}

// =================================================================================================
// Offline differential test
// =================================================================================================
//
// The original bytes run at their real addresses: the test maps the needed 64 KB blocks of the game's
// address space (code copied from the unpacked exe, data randomised per case). Every callee the
// original reaches is replaced by a recording stub (also at its real address), so the Rust port,
// which calls the same addresses, hits the same stubs. A case passes when both runs leave the same
// registers (EAX, and ECX / EDX / XMM0 where the function defines them), the same memory in the mapped
// regions and the test arena, and the same sequence of calls out (address, registers the callee
// reads, stack arguments).
//
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000"
//   cargo test --release -p sr2-hook --target i686-pc-windows-msvc <name> -- --ignored --test-threads=1
#[cfg(test)]
#[allow(clippy::all)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    // --- the shifted game data, the fixed test regions ----------------------------------------------

    static EXE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    static BLOCKS: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

    fn exe_off(va: usize) -> Option<usize> {
        let mut g = EXE.lock().unwrap();
        if g.is_empty() {
            let p = std::env::var("SR2_EXE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/SR2_pc.unpacked.exe").to_string());
            *g = std::fs::read(p).expect("exe");
        }
        let exe = &*g;
        let rd32 = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
        let pe = rd32(0x3C);
        let nsec = u16::from_le_bytes(exe[pe + 6..pe + 8].try_into().unwrap()) as usize;
        let opt = u16::from_le_bytes(exe[pe + 20..pe + 22].try_into().unwrap()) as usize;
        for i in 0..nsec {
            let o = pe + 24 + opt + i * 40;
            let (vsz, va0, rsz, raw) = (rd32(o + 8), rd32(o + 12), rd32(o + 16), rd32(o + 20));
            let base = va0 + 0x400000;
            if va >= base && va < base + vsz.max(rsz) {
                let off = va - base;
                return if off < rsz { Some(raw + off) } else { None };
            }
        }
        None
    }

    fn fixed_alloc(addr: usize, len: usize) {
        let p = unsafe { VirtualAlloc(addr as *const _, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p == addr, "cannot map {addr:#x}");
    }

    /// Map the shifted game data covering `[addr, addr + len)` (64 KB blocks, initial bytes from the exe).
    pub fn map(addr: usize, len: usize) {
        let mut g = BLOCKS.lock().unwrap();
        let set = g.get_or_insert_with(HashSet::new);
        let mut b = addr & !0xFFFF;
        while b < addr + len.max(1) {
            if set.insert(b) {
                fixed_alloc(tr(b), 0x10000);
                for pg in (0..0x10000).step_by(4) {
                    if let Some(o) = exe_off(b + pg) {
                        let exe = EXE.lock().unwrap();
                        if let Some(sl) = exe.get(o..o + 4) {
                            unsafe { *(tr(b + pg) as *mut u32) = u32::from_le_bytes(sl.try_into().unwrap()) };
                        }
                    }
                }
            }
            b += 0x10000;
        }
    }

    const ARENA_BASE: usize = 0x2000_0000;
    const CODE_BASE: usize = 0x2100_0000;
    static CODE_NEXT: Mutex<usize> = Mutex::new(0);

    /// Executable memory at a fixed place (outside the shifted game range and the heap).
    fn exec_alloc(len: usize) -> usize {
        let mut g = CODE_NEXT.lock().unwrap();
        if *g == 0 {
            fixed_alloc(CODE_BASE, 0x100000);
            *g = CODE_BASE;
        }
        let p = *g;
        *g += (len + 15) & !15;
        assert!(*g < CODE_BASE + 0x100000);
        p
    }

    /// The original function's bytes in executable memory: absolute data operands shifted, calls and
    /// tail jumps aimed at the recording stubs.
    pub struct Orig {
        pub body: &'static [u8],
        pub calls: &'static [usize],
        pub abs: &'static [usize],
        pub jmps: &'static [usize],
        pub addr: usize,
    }

    impl Orig {
        pub fn new(body: &'static [u8], calls: &'static [usize], abs: &'static [usize], jmps: &'static [usize], addr: usize) -> Orig {
            Orig { body, calls, abs, jmps, addr }
        }
        pub fn build(&self) -> usize {
            let p = exec_alloc(self.body.len() + 16);
            let mut b = self.body.to_vec();
            for &o in self.abs {
                let v = u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
                b[o..o + 4].copy_from_slice(&(tr(v) as u32).to_le_bytes());
            }
            unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len()) };
            for &o in self.calls.iter().chain(self.jmps) {
                let rel = i32::from_le_bytes(self.body[o + 1..o + 5].try_into().unwrap());
                let dest = (self.addr + o + 5).wrapping_add(rel as usize);
                let to = redirect(dest);
                let rel2 = (to as i32).wrapping_sub((p + o + 5) as i32);
                unsafe { std::ptr::write_unaligned((p + o + 1) as *mut i32, rel2) };
            }
            p
        }
    }

    static STUB_MAP: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

    /// The test copy (recording stub) of a game function; other addresses (fake callbacks) pass through.
    pub fn redirect(f: usize) -> usize {
        let m = STUB_MAP.lock().unwrap();
        if let Some(e) = m.iter().find(|e| e.0 == f) {
            return e.1;
        }
        assert!(!(0x40_1000..0xDB_8000).contains(&f), "callee {f:#x} has no stub");
        f
    }

    // --- the recording stubs -----------------------------------------------------------------

    #[derive(Clone, Copy)]
    pub struct StubSpec {
        pub addr: usize,
        pub popn: u32,
        pub nlog: u32,
        /// registers the callee reads (bit0 eax, 1 ecx, 2 edx, 3 esi, 4 edi)
        pub mask: u32,
        /// the callee leaves ECX / EDX alone
        pub keep: bool,
        /// writes its (hashed) result to the dword at the first stack argument and returns that pointer
        pub out0: bool,
        /// draw the result from the second palette
        pub pal2: bool,
    }

    impl StubSpec {
        pub fn p2(mut self) -> StubSpec {
            self.pal2 = true;
            self
        }
    }

    /// A callee at its real address: cdecl with `nlog` logged stack arguments.
    pub fn s(addr: usize, nlog: u32, mask: u32) -> StubSpec {
        StubSpec { addr, popn: 0, nlog, mask, keep: false, out0: false, pal2: false }
    }
    /// A cdecl callee that preserves ECX and EDX.
    pub fn sk(addr: usize, nlog: u32, mask: u32) -> StubSpec {
        StubSpec { addr, popn: 0, nlog, mask, keep: true, out0: false, pal2: false }
    }
    /// string_hash_lower-like: stdcall with one out pointer.
    pub fn so(addr: usize, mask: u32) -> StubSpec {
        StubSpec { addr, popn: 4, nlog: 0, mask, keep: false, out0: true, pal2: false }
    }
    /// A stdcall / thiscall callee that pops `n` dwords.
    pub fn sp(addr: usize, n: u32, mask: u32) -> StubSpec {
        StubSpec { addr, popn: n * 4, nlog: n, mask, keep: false, out0: false, pal2: false }
    }

    static STUBS: Mutex<Vec<StubSpec>> = Mutex::new(Vec::new());
    static LOG: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
    static SEED: Mutex<(u32, u32)> = Mutex::new((0, 0)); // (seed, call counter)
    static PALETTE: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    static PALETTE2: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    static COMMON: Mutex<usize> = Mutex::new(0);

    fn mix(a: u32, b: u32, c: u32) -> u32 {
        let mut x = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B) ^ c.wrapping_mul(0xC2B2_AE35);
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        x = x.wrapping_mul(0x297A_2D39);
        x ^ (x >> 15)
    }

    extern "C" fn rec_rs(block: *mut u32) {
        unsafe {
            let id = *block.add(6) as usize;
            let spec = STUBS.lock().unwrap()[id];
            let (seed, n) = {
                let mut g = SEED.lock().unwrap();
                g.1 += 1;
                *g
            };
            let norm = |v: u32| if ((DELTA + 0xD0_0000)..(DELTA + 0x360_0000)).contains(&(v as usize)) { v - DELTA as u32 } else { v };
            let mut e = vec![spec.addr as u32];
            for i in 0..5 {
                e.push(if spec.mask >> i & 1 != 0 { norm(*block.add(i)) } else { 0 });
            }
            for i in 0..spec.nlog as usize {
                e.push(norm(*block.add(8 + i)));
            }
            LOG.lock().unwrap().push(e);
            let h = mix(seed, spec.addr as u32, n);
            let pal = if spec.pal2 { PALETTE2.lock().unwrap() } else { PALETTE.lock().unwrap() };
            *block = if pal.is_empty() { h } else { pal[h as usize % pal.len()] };
            if std::env::var("DT_TRACE2").is_ok() { eprintln!("  stub {:#x} -> {:#x}", spec.addr, *block); }
            if spec.out0 {
                let p = *block.add(8) as *mut u32;
                *p = mix(h, 3, 5);
                *block = p as u32;
            }
            if !spec.keep {
                *block.add(1) = mix(h, 1, 7);
                *block.add(2) = mix(h, 2, 9);
            }
        }
    }

    #[unsafe(naked)]
    extern "C" fn rec_common() {
        std::arch::naked_asm!(
            "push edi", "push esi", "push edx", "push ecx", "push eax",
            "push esp",
            "call {rec}",
            "add esp, 4",
            "pop eax", "pop ecx", "pop edx", "pop esi", "pop edi",
            "ret",
            rec = sym rec_rs,
        )
    }

    /// Write the trampoline for `spec` at `at`.
    unsafe fn write_tramp(at: usize, id: usize, popn: u32) {
        unsafe {
            let p = at as *mut u8;
            *p = 0x68; // push id
            *(p.add(1) as *mut u32) = id as u32;
            *p.add(5) = 0xE8; // call rec_common
            let rel = (rec_common as *const () as usize as i32).wrapping_sub((at + 10) as i32);
            *(p.add(6) as *mut i32) = rel;
            *p.add(10) = 0x83; // add esp, 4 (drops the id)
            *p.add(11) = 0xC4;
            *p.add(12) = 0x04;
            if popn == 0 {
                *p.add(13) = 0xC3;
            } else {
                *p.add(13) = 0xC2;
                *(p.add(14) as *mut u16) = popn as u16;
            }
        }
    }

    pub fn install_stubs(specs: &[StubSpec]) {
        let mut g = STUBS.lock().unwrap();
        let mut m = STUB_MAP.lock().unwrap();
        m.clear();
        for sp in specs {
            let at = exec_alloc(32);
            g.push(*sp);
            unsafe { write_tramp(at, g.len() - 1, sp.popn) };
            m.push((sp.addr, at));
        }
    }

    /// A pool of fake callees (callback targets stored in game memory), `n` of each kind.
    pub fn fake(n: usize, popn: u32, nlog: u32, mask: u32) -> Vec<usize> {
        let mut out = Vec::new();
        for _ in 0..n {
            let at = exec_alloc(32);
            let mut g = STUBS.lock().unwrap();
            let spec = StubSpec { addr: at, popn: popn * 4, nlog, mask, keep: false, out0: false, pal2: false };
            g.push(spec);
            unsafe { write_tramp(at, g.len() - 1, popn * 4) };
            out.push(at);
        }
        out
    }

    // --- the call harness (53-bit x87, clean stack, canary registers) ------------------------------

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct TB {
        f: u32,
        eax: u32,
        ecx: u32,
        edx: u32,
        ebx: u32,
        esi: u32,
        edi: u32,
        ebp: u32,
        args: u32,
        nargs: u32,
        pub o_eax: u32,
        pub o_ecx: u32,
        pub o_edx: u32,
        pub o_ebx: u32,
        pub o_esi: u32,
        pub o_edi: u32,
        pub o_ebp: u32,
        top0: u32,
        top1: u32,
        esp: u32,
        pub xmm: [u32; 32],
        pub st0: u64,
    }

    static mut SLOTS: [u32; 2] = [0; 2];

    #[derive(Clone, Copy)]
    pub struct In {
        pub eax: u32,
        pub ecx: u32,
        pub edx: u32,
        pub esi: u32,
        pub edi: u32,
    }

    const EBX_IN: u32 = 0xB1B1_B1B1;
    const EBP_IN: u32 = 0xE9E9_E9E9;

    fn xmm_pattern() -> [u32; 32] {
        let mut x = [0u32; 32];
        for (i, w) in x.iter_mut().enumerate() {
            *w = 0x3F00_0000 + (i as u32) * 0x0101_0101;
        }
        x
    }

    unsafe fn call_t(f: usize, r: In, args: &[u32]) -> TB {
        let mut b = TB {
            f: f as u32, eax: r.eax, ecx: r.ecx, edx: r.edx, ebx: EBX_IN, esi: r.esi, edi: r.edi, ebp: EBP_IN,
            args: args.as_ptr() as u32, nargs: args.len() as u32,
            o_eax: 0, o_ecx: 0, o_edx: 0, o_ebx: 0, o_esi: 0, o_edi: 0, o_ebp: 0, top0: 0, top1: 0, esp: 0,
            xmm: xmm_pattern(), st0: 0,
        };
        unsafe {
            let slots = std::ptr::addr_of_mut!(SLOTS);
            (*slots)[0] = &mut b as *mut TB as u32;
            (*slots)[1] = f as u32;
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 76], esp",
                "mov [{s}], eax",
                "mov ebx, eax",
                "fninit",
                "push 0x27F",
                "fldcw word ptr [esp]",
                "add esp, 4",
                "movups xmm0, xmmword ptr [ebx + 80]",
                "movups xmm1, xmmword ptr [ebx + 96]",
                "movups xmm2, xmmword ptr [ebx + 112]",
                "movups xmm3, xmmword ptr [ebx + 128]",
                "movups xmm4, xmmword ptr [ebx + 144]",
                "movups xmm5, xmmword ptr [ebx + 160]",
                "movups xmm6, xmmword ptr [ebx + 176]",
                "movups xmm7, xmmword ptr [ebx + 192]",
                "mov ecx, [ebx + 36]",
                "mov esi, [ebx + 32]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "fnstsw word ptr [ebx + 68]",
                "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 20]", "mov edi, [ebx + 24]",
                "mov ebp, [ebx + 28]", "mov eax, [ebx + 4]", "mov ebx, [ebx + 16]",
                "call dword ptr [{s} + 4]",
                "push eax",
                "mov eax, dword ptr [{s}]",
                "mov [eax + 44], ecx", "mov [eax + 48], edx", "mov [eax + 52], ebx", "mov [eax + 56], esi",
                "mov [eax + 60], edi", "mov [eax + 64], ebp",
                "pop dword ptr [eax + 40]",
                "movups xmmword ptr [eax + 80], xmm0",
                "movups xmmword ptr [eax + 96], xmm1",
                "movups xmmword ptr [eax + 112], xmm2",
                "movups xmmword ptr [eax + 128], xmm3",
                "movups xmmword ptr [eax + 144], xmm4",
                "movups xmmword ptr [eax + 160], xmm5",
                "movups xmmword ptr [eax + 176], xmm6",
                "movups xmmword ptr [eax + 192], xmm7",
                "fnstsw word ptr [eax + 72]",
                "movzx ecx, word ptr [eax + 72]",
                "and ecx, 0x3800",
                "movzx edx, word ptr [eax + 68]",
                "and edx, 0x3800",
                "cmp ecx, edx",
                "je 5f",
                "fstp qword ptr [eax + 208]",
                "5:",
                "mov esp, [eax + 76]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                s = sym SLOTS,
                inout("eax") &mut b as *mut TB => _,
                clobber_abi("C"),
            );
        }
        b
    }

    // --- random numbers -------------------------------------------------------------------------

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
        pub fn chance(&mut self, n: u32) -> bool {
            self.below(n) == 0
        }
        /// 0, small, or a full random word.
        pub fn word(&mut self) -> u32 {
            match self.below(6) {
                0 => 0,
                1 => self.below(8),
                2 => 0xFFFF_FFFF,
                3 => self.u32() & 0xFF,
                _ => self.u32(),
            }
        }
        pub fn f32bits(&mut self) -> u32 {
            match self.below(8) {
                0 => 0,
                1 => 0x8000_0000,
                2 => 0x3F80_0000,
                _ => ((((self.u32() as f64 / 4294967296.0) * 2.0 - 1.0) * 1000.0) as f32).to_bits(),
            }
        }
    }

    // --- the world ----------------------------------------------------------------------------

    pub struct W {
        pub arena: &'static mut [u32],
        pub regions: Vec<(usize, usize)>,
    }

    impl W {
        /// Pointer to arena word `i`.
        pub fn p(&self, i: usize) -> u32 {
            self.arena.as_ptr() as u32 + (i as u32) * 4
        }
        /// Fill the arena with words (zeros, small numbers, arena pointers, random).
        pub fn soup(&mut self, rng: &mut Rng) {
            let n = self.arena.len();
            let base = self.arena.as_ptr() as u32;
            for i in 0..n {
                self.arena[i] = match rng.below(8) {
                    0 | 1 => 0,
                    2 => rng.below(8),
                    3 | 4 => base + rng.below(n as u32) * 4,
                    5 => rng.u32() & 0xFF,
                    _ => rng.u32(),
                };
            }
        }
        /// Randomise a game region (bytes).
        pub fn rand_region(&self, addr: usize, len: usize, rng: &mut Rng) {
            for o in (0..len).step_by(4) {
                unsafe { wd(addr + o, rng.word()) };
            }
        }
    }

    pub struct Cfg {
        pub regions: Vec<(usize, usize)>,
        pub stubs: Vec<StubSpec>,
        pub arena_words: usize,
        /// output registers the function defines besides EAX
        pub out_ecx: bool,
        pub out_edx: bool,
        pub out_xmm0: bool,
        pub palette: Vec<u32>,
        /// compare ESI / EDI / EBX / EBP between the runs (default: they must equal the inputs)
        pub cases: u32,
        /// compare only AL of the result (the rest is a stack address)
        pub eax_lo8: bool,
    }

    impl Cfg {
        pub fn new() -> Cfg {
            Cfg { regions: vec![], stubs: vec![], arena_words: 512, out_ecx: false, out_edx: false, out_xmm0: false, palette: vec![], cases: 0, eax_lo8: false }
        }
    }

    fn cases(c: &Cfg) -> u32 {
        if c.cases != 0 {
            return c.cases;
        }
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    /// A value inside the shifted game range, shifted back (so a stored game address compares equal).
    fn nz(v: u32) -> u32 {
        if ((DELTA + 0xD0_0000)..(DELTA + 0x360_0000)).contains(&(v as usize)) { v - DELTA as u32 } else { v }
    }

    fn norm_mem(v: &mut Vec<u8>) {
        for c in v.chunks_exact_mut(4) {
            let x = nz(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
            c.copy_from_slice(&x.to_le_bytes());
        }
    }

    fn snapshot(cfg: &Cfg, arena: &[u32]) -> Vec<u8> {
        let mut v = Vec::new();
        for &(a, l) in &cfg.regions {
            v.extend_from_slice(unsafe { std::slice::from_raw_parts(tr(a) as *const u8, l) });
        }
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(arena.as_ptr() as *const u8, arena.len() * 4) });
        v
    }

    fn restore(cfg: &Cfg, arena: &mut [u32], snap: &[u8]) {
        let mut o = 0;
        for &(a, l) in &cfg.regions {
            unsafe { std::ptr::copy_nonoverlapping(snap.as_ptr().add(o), tr(a) as *mut u8, l) };
            o += l;
        }
        unsafe { std::ptr::copy_nonoverlapping(snap.as_ptr().add(o), arena.as_mut_ptr() as *mut u8, arena.len() * 4) };
    }

    fn one_run(f: usize, r: In, args: &[u32], seed: u32) -> (TB, Vec<Vec<u32>>) {
        *SEED.lock().unwrap() = (seed, 0);
        LOG.lock().unwrap().clear();
        let t = unsafe { call_t(f, r, args) };
        let lg = std::mem::take(&mut *LOG.lock().unwrap());
        (t, lg)
    }

    fn fmt_log(l: &[Vec<u32>]) -> String {
        l.iter().map(|e| format!("{:x?}", e)).collect::<Vec<_>>().join(" ")
    }

    /// Drive `cases` random worlds through the original and the thunk.
    pub fn drive(name: &str, orig: Orig, thunk: usize, mut cfg: Cfg, mut setup: impl FnMut(&mut W, &mut Rng) -> (In, Vec<u32>)) {
        let trace = std::env::var("DT_TRACE").is_ok();
        for &(a, l) in &cfg.regions {
            map(a, l);
        }
        if trace { eprintln!("mapped"); }
        install_stubs(&cfg.stubs.clone());
        let orig_addr = orig.addr;
        let orig = orig.build();
        if trace { eprintln!("built {orig:#x}"); }
        *PALETTE.lock().unwrap() = cfg.palette.clone();
        {
            let mut g = BLOCKS.lock().unwrap();
            let set = g.get_or_insert_with(HashSet::new);
            if set.insert(ARENA_BASE) {
                fixed_alloc(ARENA_BASE, 0x100000);
            }
        }
        let arena: &'static mut [u32] = unsafe { std::slice::from_raw_parts_mut(ARENA_BASE as *mut u32, cfg.arena_words) };
        arena.fill(0);
        let mut w = W { arena, regions: cfg.regions.clone() };
        let mut rng = Rng(0x1234_5678_9ABC_DEF1 ^ (orig_addr as u64) << 8);
        let n = cases(&cfg);
        let mut bad = 0;
        let mut mem_changed = 0;
        let mut calls_out = 0;
        for i in 0..n {
            let (r, args) = setup(&mut w, &mut rng);
            let seed = rng.u32();
            let snap = snapshot(&cfg, w.arena);
            if trace { eprintln!("case {i} orig: regs eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} args {:x?}", r.eax, r.ecx, r.edx, r.esi, r.edi, args); if i == 0 { for rg in &cfg.regions { eprintln!("region {:#x}: {:x?}", rg.0, unsafe { std::slice::from_raw_parts(tr(rg.0) as *const u32, rg.1 / 4) }); } eprintln!("arena {:x?}", &w.arena[..64.min(w.arena.len())]); } }
            let (o, olog) = one_run(orig, r, &args, seed);
            let ostate = snapshot(&cfg, w.arena);
            restore(&cfg, w.arena, &snap);
            if trace { eprintln!("case {i} thunk"); }
            let (t, tlog) = one_run(thunk, r, &args, seed);
            let tstate = snapshot(&cfg, w.arena);
            restore(&cfg, w.arena, &snap);
            if ostate != snap {
                mem_changed += 1;
            }
            calls_out += olog.len();
            let mut err = String::new();
            if (if cfg.eax_lo8 { (o.o_eax & 0xff, t.o_eax & 0xff) } else { (nz(o.o_eax), nz(t.o_eax)) }).0 != (if cfg.eax_lo8 { (o.o_eax & 0xff, t.o_eax & 0xff) } else { (nz(o.o_eax), nz(t.o_eax)) }).1 {
                err += &format!(" eax original {:#x} Rust {:#x};", o.o_eax, t.o_eax);
            }
            if o.o_ebx != EBX_IN || o.o_ebp != EBP_IN {
                err += " ORIGINAL changed ebx/ebp;";
            }
            if t.o_ebx != EBX_IN || t.o_ebp != EBP_IN {
                err += " Rust changed ebx/ebp;";
            }
            if nz(o.o_esi) != nz(t.o_esi) || nz(o.o_edi) != nz(t.o_edi) {
                err += &format!(" esi/edi original {:#x}/{:#x} Rust {:#x}/{:#x};", o.o_esi, o.o_edi, t.o_esi, t.o_edi);
            }
            if cfg.out_ecx && nz(o.o_ecx) != nz(t.o_ecx) {
                err += &format!(" ecx original {:#x} Rust {:#x};", o.o_ecx, t.o_ecx);
            }
            if cfg.out_edx && nz(o.o_edx) != nz(t.o_edx) {
                err += &format!(" edx original {:#x} Rust {:#x};", o.o_edx, t.o_edx);
            }
            if o.o_ecx == r.ecx && t.o_ecx != r.ecx {
                err += &format!(" ecx: original preserved it, Rust gave {:#x};", t.o_ecx);
            }
            if o.o_edx == r.edx && t.o_edx != r.edx {
                err += &format!(" edx: original preserved it, Rust gave {:#x};", t.o_edx);
            }
            if t.xmm[4..] != xmm_pattern()[4..] {
                err += " xmm1-7 changed;";
            }
            if cfg.out_xmm0 && o.xmm[0] != t.xmm[0] {
                err += &format!(" xmm0 original {:#x} Rust {:#x};", o.xmm[0], t.xmm[0]);
            }
            let od = ((o.top0 & 0x3800).wrapping_sub(o.top1 & 0x3800) & 0x3800) >> 11;
            let td = ((t.top0 & 0x3800).wrapping_sub(t.top1 & 0x3800) & 0x3800) >> 11;
            if od != td {
                err += &format!(" x87 depth original {od} Rust {td};");
            }
            let (mut ostate, mut tstate) = (ostate, tstate);
            norm_mem(&mut ostate);
            norm_mem(&mut tstate);
            if ostate != tstate {
                let k = ostate.iter().zip(&tstate).position(|(a, b)| a != b).unwrap();
                err += &format!(" memory differs at byte {k} (original {:#x} Rust {:#x});", ostate[k], tstate[k]);
            }
            if olog != tlog {
                err += &format!("\n    calls original: {}\n    calls Rust:     {}", fmt_log(&olog), fmt_log(&tlog));
            }
            if !err.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name} case {i}: in eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} args {:x?}:{err}", r.eax, r.ecx, r.edx, r.esi, r.edi, args);
                }
            }
        }
        eprintln!("{name}: {n} cases, {bad} differ ({mem_changed} cases wrote memory, {calls_out} calls out)");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }

    pub fn inr(rng: &mut Rng) -> In {
        In { eax: rng.word(), ecx: rng.word(), edx: rng.word(), esi: rng.word() | 0x100, edi: rng.word() | 0x100 }
    }

    /// The game-event callback table: index 0..4 and 4 entries whose slots hold fake callbacks or 0.
    pub fn evt_fill(rng: &mut Rng, fk: &[usize]) {
        unsafe {
            wd(0xE96C00, rng.below(4));
            for e in 0..4usize {
                for sl in 0..27usize {
                    let v = if rng.chance(3) { 0 } else { fk[rng.below(fk.len() as u32) as usize] as u32 };
                    wd(0xE96C10 + e * 0x6c + sl * 4, v);
                }
                // the byte flags other functions keep in the slots stay random words
            }
        }
    }

    /// Diversion handlers: 37 slots -> 6 arena objects (vtable at word 0, flag byte at +0x14) whose
    /// vtables (4 x 64 words at arena words 0..256) hold fake methods. Returns nothing; the arena
    /// words 0..320 are overwritten.
    pub fn div_fill(w: &mut W, rng: &mut Rng, fk: &[usize]) {
        for k in 0..4usize {
            for s in 0..64usize {
                w.arena[k * 64 + s] = fk[rng.below(fk.len() as u32) as usize] as u32;
            }
        }
        for o in 0..6usize {
            let b = 256 + o * 8;
            for k in 0..8usize {
                w.arena[b + k] = rng.u32();
            }
            w.arena[b] = w.p(rng.below(4) as usize * 64);
            w.arena[b + 5] = if rng.chance(2) { 0 } else { rng.below(256) | 1 };
        }
        unsafe {
            for i in 0..37usize {
                wd(0x0140_B168 + i * 4, w.p(256 + rng.below(6) as usize * 8));
            }
            wd(0x0217_03D4, if rng.chance(4) { 0 } else { 1 + rng.below(3) });
        }
    }


    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_23() {

        let fk = fake(6, 0, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        drive("game_event_23", Orig::new(&GAME_EVENT_23_BODY, &GAME_EVENT_23_CALLS, &GAME_EVENT_23_ABS, &GAME_EVENT_23_JMPS, 0x68f9a0), thunk_game_event_23 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_24() {

        let fk = fake(6, 0, 2, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        drive("game_event_24", Orig::new(&GAME_EVENT_24_BODY, &GAME_EVENT_24_CALLS, &GAME_EVENT_24_ABS, &GAME_EVENT_24_JMPS, 0x68f9c0), thunk_game_event_24 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_25() {

        let fk = fake(6, 0, 2, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        drive("game_event_25", Orig::new(&GAME_EVENT_25_BODY, &GAME_EVENT_25_CALLS, &GAME_EVENT_25_ABS, &GAME_EVENT_25_JMPS, 0x68f9e0), thunk_game_event_25 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_26() {

        let fk = fake(6, 0, 2, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        drive("game_event_26", Orig::new(&GAME_EVENT_26_BODY, &GAME_EVENT_26_CALLS, &GAME_EVENT_26_ABS, &GAME_EVENT_26_JMPS, 0x68fb10), thunk_game_event_26 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![rng.word()])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_3() {

        let fk = fake(6, 0, 1, 0b11001);
        let fk0 = fake(6, 0, 0, 0b11001);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        drive("game_event_3", Orig::new(&GAME_EVENT_3_BODY, &GAME_EVENT_3_CALLS, &GAME_EVENT_3_ABS, &GAME_EVENT_3_JMPS, 0x68f650), thunk_game_event_3 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            unsafe {
                // every entry's slots 2 and 3 (offsets 8 and 0xc) are always set
                for e in 0..4usize {
                    wd(0xE96C10 + e * 0x6c + 8, fk[rng.below(6) as usize] as u32);
                    wd(0xE96C10 + e * 0x6c + 0xc, fk0[rng.below(6) as usize] as u32);
                }
            }
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_20() {

        let fk = fake(6, 0, 1, 0);
        let dv = 0x604680;
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(dv, 0, 0b10000)];
        drive("game_event_20", Orig::new(&GAME_EVENT_20_BODY, &GAME_EVENT_20_CALLS, &GAME_EVENT_20_ABS, &GAME_EVENT_20_JMPS, 0x68f910), thunk_game_event_20 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_21() {

        let fk = fake(6, 0, 1, 0);
        let dv = 0x6046b0;
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(dv, 0, 0b10000)];
        drive("game_event_21", Orig::new(&GAME_EVENT_21_BODY, &GAME_EVENT_21_CALLS, &GAME_EVENT_21_ABS, &GAME_EVENT_21_JMPS, 0x68f940), thunk_game_event_21 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_22() {

        let fk = fake(6, 0, 1, 0);
        let dv = 0x6046e0;
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(dv, 0, 0b10000)];
        drive("game_event_22", Orig::new(&GAME_EVENT_22_BODY, &GAME_EVENT_22_CALLS, &GAME_EVENT_22_ABS, &GAME_EVENT_22_JMPS, 0x68f970), thunk_game_event_22 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_18() {

        let fk = fake(6, 0, 1, 0);
        let dv = 0x604970;
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(dv, 0, 0b10000)];
        drive("game_event_18", Orig::new(&GAME_EVENT_18_BODY, &GAME_EVENT_18_CALLS, &GAME_EVENT_18_ABS, &GAME_EVENT_18_JMPS, 0x68fae0), thunk_game_event_18 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_16() {

        let fk = fake(6, 0, 4, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0), (0x0252_9E00, 4)];
        drive("game_event_16", Orig::new(&GAME_EVENT_16_BODY, &GAME_EVENT_16_CALLS, &GAME_EVENT_16_ABS, &GAME_EVENT_16_JMPS, 0x68fa00), thunk_game_event_16 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            unsafe { wd(0x0252_9E00, rng.word()) };
            (inr(rng), vec![rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_player_joined() {

        let fk = fake(6, 0, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.arena_words = 2048;
        drive("game_event_player_joined", Orig::new(&GAME_EVENT_PLAYER_JOINED_BODY, &GAME_EVENT_PLAYER_JOINED_CALLS, &GAME_EVENT_PLAYER_JOINED_ABS, &GAME_EVENT_PLAYER_JOINED_JMPS, 0x68f680), thunk_game_event_player_joined as *const () as usize, cfg, |w, rng| {
            evt_fill(rng, &fk);
            w.soup(rng);
            let mut r = inr(rng);
            r.esi = w.p(0) + 4 * rng.below(256);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_player_left() {

        let fk = fake(6, 0, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(0x0060_4940, 0, 0b10000)];
        cfg.arena_words = 2048;
        drive("game_event_player_left", Orig::new(&GAME_EVENT_PLAYER_LEFT_BODY, &GAME_EVENT_PLAYER_LEFT_CALLS, &GAME_EVENT_PLAYER_LEFT_ABS, &GAME_EVENT_PLAYER_LEFT_JMPS, 0x68f6b0), thunk_game_event_player_left as *const () as usize, cfg, |w, rng| {
            evt_fill(rng, &fk);
            w.soup(rng);
            let mut r = inr(rng);
            r.eax = w.p(0) + 4 * rng.below(256);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_game_event_18() {

        let fk = fake(8, 1, 1, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_game_event_18", Orig::new(&DIVERSIONS_ON_GAME_EVENT_18_BODY, &DIVERSIONS_ON_GAME_EVENT_18_CALLS, &DIVERSIONS_ON_GAME_EVENT_18_ABS, &DIVERSIONS_ON_GAME_EVENT_18_JMPS, 0x604970), thunk_diversions_on_game_event_18 as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_game_event_20() {

        let fk = fake(8, 1, 1, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_game_event_20", Orig::new(&DIVERSIONS_ON_GAME_EVENT_20_BODY, &DIVERSIONS_ON_GAME_EVENT_20_CALLS, &DIVERSIONS_ON_GAME_EVENT_20_ABS, &DIVERSIONS_ON_GAME_EVENT_20_JMPS, 0x604680), thunk_diversions_on_game_event_20 as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_game_event_21() {

        let fk = fake(8, 1, 1, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_game_event_21", Orig::new(&DIVERSIONS_ON_GAME_EVENT_21_BODY, &DIVERSIONS_ON_GAME_EVENT_21_CALLS, &DIVERSIONS_ON_GAME_EVENT_21_ABS, &DIVERSIONS_ON_GAME_EVENT_21_JMPS, 0x6046b0), thunk_diversions_on_game_event_21 as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_game_event_22() {

        let fk = fake(8, 1, 1, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_game_event_22", Orig::new(&DIVERSIONS_ON_GAME_EVENT_22_BODY, &DIVERSIONS_ON_GAME_EVENT_22_CALLS, &DIVERSIONS_ON_GAME_EVENT_22_ABS, &DIVERSIONS_ON_GAME_EVENT_22_JMPS, 0x6046e0), thunk_diversions_on_game_event_22 as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_object_damaged() {

        let fk = fake(8, 4, 4, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_object_damaged", Orig::new(&DIVERSIONS_ON_OBJECT_DAMAGED_BODY, &DIVERSIONS_ON_OBJECT_DAMAGED_CALLS, &DIVERSIONS_ON_OBJECT_DAMAGED_ABS, &DIVERSIONS_ON_OBJECT_DAMAGED_JMPS, 0x604740), thunk_diversions_on_object_damaged as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_vehicle_destroyed() {

        let fk = fake(8, 4, 4, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_vehicle_destroyed", Orig::new(&DIVERSIONS_ON_VEHICLE_DESTROYED_BODY, &DIVERSIONS_ON_VEHICLE_DESTROYED_CALLS, &DIVERSIONS_ON_VEHICLE_DESTROYED_ABS, &DIVERSIONS_ON_VEHICLE_DESTROYED_JMPS, 0x6047f0), thunk_diversions_on_vehicle_destroyed as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_object_killed() {

        let fk = fake(8, 4, 4, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_object_killed", Orig::new(&DIVERSIONS_ON_OBJECT_KILLED_BODY, &DIVERSIONS_ON_OBJECT_KILLED_CALLS, &DIVERSIONS_ON_OBJECT_KILLED_ABS, &DIVERSIONS_ON_OBJECT_KILLED_JMPS, 0x604840), thunk_diversions_on_object_killed as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_diversions_on_game_event_15() {

        let fk = fake(8, 5, 5, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_B168, 0x98), (0x0217_03D4, 4)];
        cfg.arena_words = 512;
        drive("diversions_on_game_event_15", Orig::new(&DIVERSIONS_ON_GAME_EVENT_15_BODY, &DIVERSIONS_ON_GAME_EVENT_15_CALLS, &DIVERSIONS_ON_GAME_EVENT_15_ABS, &DIVERSIONS_ON_GAME_EVENT_15_JMPS, 0x604630), thunk_diversions_on_game_event_15 as *const () as usize, cfg, |w, rng| {
            div_fill(w, rng, &fk);
            (inr(rng), vec![rng.word(), rng.word(), rng.word(), rng.word()])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_game_event_15() {

        let fk = fake(6, 0, 5, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE96C00, 0x1C0)];
        cfg.stubs = vec![s(0x0060_4630, 4, 0b10000)];
        drive("game_event_15", Orig::new(&GAME_EVENT_15_BODY, &GAME_EVENT_15_CALLS, &GAME_EVENT_15_ABS, &GAME_EVENT_15_JMPS, 0x68f8c0), thunk_game_event_15 as *const () as usize, cfg, |_w, rng| {
            evt_fill(rng, &fk);
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_gfx_ring_has_data() {

        let cfg = Cfg::new();
        drive("gfx_ring_has_data", Orig::new(&GFX_RING_HAS_DATA_BODY, &GFX_RING_HAS_DATA_CALLS, &GFX_RING_HAS_DATA_ABS, &GFX_RING_HAS_DATA_JMPS, 0xcf5940), thunk_gfx_ring_has_data as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let mut r = inr(rng);
            r.eax = w.p(rng.below(100) as usize);
            if rng.chance(3) {
                let o = ((r.eax - w.p(0)) / 4) as usize;
                w.arena[o + 5] = w.arena[o + 2];
            }
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_action_id_flags2_test() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE8EE98, 0x1000)];
        drive("action_id_flags2_test", Orig::new(&ACTION_ID_FLAGS2_TEST_BODY, &ACTION_ID_FLAGS2_TEST_CALLS, &ACTION_ID_FLAGS2_TEST_ABS, &ACTION_ID_FLAGS2_TEST_JMPS, 0x6ef730), thunk_action_id_flags2_test as *const () as usize, cfg, |_w, rng| {
            unsafe {
                for i in 0..0x400usize {
                    wd(0xE8EE98 + i * 4, rng.word());
                }
            }
            let mut r = inr(rng);
            r.eax = if rng.chance(5) { 0xFFFF_FFFF } else { rng.below(300) };
            r.ecx = match rng.below(4) { 0 => 0, 1 => 1 << rng.below(32), _ => rng.u32() };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_player_flag_b01() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7D10, 4)];
        cfg.arena_words = 1024;
        drive("cutscene_player_flag_b01", Orig::new(&CUTSCENE_PLAYER_FLAG_B01_BODY, &CUTSCENE_PLAYER_FLAG_B01_CALLS, &CUTSCENE_PLAYER_FLAG_B01_ABS, &CUTSCENE_PLAYER_FLAG_B01_JMPS, 0x6c6d70), thunk_cutscene_player_flag_b01 as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            unsafe { wd(0x0252_7D10, if rng.chance(4) { 0 } else { w.p(rng.below(100) as usize) }) };
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_running() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7CF4, 4), (0x0252_7D14, 4)];
        drive("cutscene_running", Orig::new(&CUTSCENE_RUNNING_BODY, &CUTSCENE_RUNNING_CALLS, &CUTSCENE_RUNNING_ABS, &CUTSCENE_RUNNING_JMPS, 0x6c6d50), thunk_cutscene_running as *const () as usize, cfg, |_w, rng| {
            unsafe {
                wd(0x0252_7CF4, if rng.chance(2) { 0 } else { rng.u32() });
                wd(0x0252_7D14, if rng.chance(2) { 0 } else { rng.u32() });
            }
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_human_party_leader() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0214_9C64, 0x10000)];
        cfg.arena_words = 2048;
        drive("human_party_leader", Orig::new(&HUMAN_PARTY_LEADER_BODY, &HUMAN_PARTY_LEADER_CALLS, &HUMAN_PARTY_LEADER_ABS, &HUMAN_PARTY_LEADER_JMPS, 0x98e710), thunk_human_party_leader as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // 8 humans / leaders of 0x100 bytes at arena words 0..512
            let k = rng.below(4096);
            let v = k | (rng.below(8) << 12);
            unsafe {
                for i in 0..4096usize {
                    wd(0x0214_9C64 + i * 16, if rng.chance(3) { 0 } else { w.p(rng.below(8) as usize * 64) });
                }
                wd(0x0214_9C64 + k as usize * 16, w.p(rng.below(8) as usize * 64));
            }
            for o in 0..8usize {
                w.arena[o * 64 + 0x44 / 4] = if rng.chance(5) { rng.u32() } else { v };
                w.arena[o * 64 + 0x4c / 4] = if rng.chance(5) { rng.below(3) } else { 1 };
                w.arena[o * 64 + 0x80 / 4] = if rng.chance(3) { rng.u32() } else { rng.u32() & !0xb };
            }
            // humans at arena words 1024..: [h + 0x2a4]
            let hum = 1024 + rng.below(8) as usize * 64;
            w.arena[hum + 0x2a4 / 4] = if rng.chance(8) { 0 } else { v };
            let mut r = inr(rng);
            r.edx = if rng.chance(10) { 0 } else { w.p(hum) };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_pcu_item_find_material_by_id() {

        let cfg = Cfg::new();
        drive("pcu_item_find_material_by_id", Orig::new(&PCU_ITEM_FIND_MATERIAL_BY_ID_BODY, &PCU_ITEM_FIND_MATERIAL_BY_ID_CALLS, &PCU_ITEM_FIND_MATERIAL_BY_ID_ABS, &PCU_ITEM_FIND_MATERIAL_BY_ID_JMPS, 0x7c1bd0), thunk_pcu_item_find_material_by_id as *const () as usize, cfg, |w, rng| {
            w.arena.fill(0);
            // item at word 0: base pointer at +0x28 (word 10), count at +0x2c (word 11)
            let n = rng.below(10) as usize;
            let cnt = match rng.below(12) { 0 => 0, 1 => 0xFFFF_FFFF, 2 => 0xFFFF_FFF0, _ => n as u32 };
            w.arena[10] = w.p(64);
            w.arena[11] = cnt;
            let want = rng.below(16);
            for k in 0..10usize {
                w.arena[64 + k * 28 + 4] = if rng.chance(3) { want } else { rng.below(16) };
            }
            let mut r = inr(rng);
            r.edx = if rng.chance(10) { 0 } else { w.p(0) };
            r.esi = want;
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_gfx_ring_read_u32() {

        let cfg = Cfg::new();
        drive("gfx_ring_read_u32", Orig::new(&GFX_RING_READ_U32_BODY, &GFX_RING_READ_U32_CALLS, &GFX_RING_READ_U32_ABS, &GFX_RING_READ_U32_JMPS, 0xbd5550), thunk_gfx_ring_read_u32 as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let ring = rng.below(64) as usize;
            w.arena[ring + 5] = w.p(64 + rng.below(64) as usize);
            let mut r = inr(rng);
            r.eax = w.p(ring);
            let dst = if rng.chance(8) { w.p(ring + 5) } else { w.p(200 + rng.below(50) as usize) };
            (r, vec![dst])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_idirect3ddevice9lost_static_init() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xEC24CC, 8)];
        drive("idirect3ddevice9lost_static_init", Orig::new(&IDIRECT3DDEVICE9LOST_STATIC_INIT_BODY, &IDIRECT3DDEVICE9LOST_STATIC_INIT_CALLS, &IDIRECT3DDEVICE9LOST_STATIC_INIT_ABS, &IDIRECT3DDEVICE9LOST_STATIC_INIT_JMPS, 0xcf58e0), thunk_idirect3ddevice9lost_static_init as *const () as usize, cfg, |w, rng| {
            w.rand_region(0xEC24CC, 8, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_human_flag_c8_stamp_set() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE843AC, 4)];
        cfg.arena_words = 2048;
        drive("human_flag_c8_stamp_set", Orig::new(&HUMAN_FLAG_C8_STAMP_SET_BODY, &HUMAN_FLAG_C8_STAMP_SET_CALLS, &HUMAN_FLAG_C8_STAMP_SET_ABS, &HUMAN_FLAG_C8_STAMP_SET_JMPS, 0x964560), thunk_human_flag_c8_stamp_set as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            unsafe { wd(0xE843AC, match rng.below(4) { 0 => 0x6B49_D200, 1 => 0x6B49_D201, 2 => rng.u32() & 0x7FFF_FFFF, _ => rng.u32() }) };
            let mut r = inr(rng);
            r.eax = w.p(rng.below(400) as usize * 4 + 0) - 0xc8 + 0xc8;
            r.eax = w.p(0) + 4 * rng.below(256);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_nav_search_begin() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x02E1_60B8, 0x10), (0x02E2_E0C4, 4)];
        drive("nav_search_begin", Orig::new(&NAV_SEARCH_BEGIN_BODY, &NAV_SEARCH_BEGIN_CALLS, &NAV_SEARCH_BEGIN_ABS, &NAV_SEARCH_BEGIN_JMPS, 0xa0ffa0), thunk_nav_search_begin as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x02E1_60B8, 0x10, rng);
            w.rand_region(0x02E2_E0C4, 4, rng);
            unsafe { wd(0x02E1_60BC, match rng.below(4) { 0 => 0xFFFF_FFFF, 1 => 0, _ => rng.u32() }) };
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_havok_removal_queue_push() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7DC0, 4), (0x0168_B738, 0x1F50)];
        cfg.stubs = vec![s(0x006F_5C10, 0, 0b01000)];
        drive("havok_removal_queue_push", Orig::new(&HAVOK_REMOVAL_QUEUE_PUSH_BODY, &HAVOK_REMOVAL_QUEUE_PUSH_CALLS, &HAVOK_REMOVAL_QUEUE_PUSH_ABS, &HAVOK_REMOVAL_QUEUE_PUSH_JMPS, 0x6f5cd0), thunk_havok_removal_queue_push as *const () as usize, cfg, |_w, rng| {
            unsafe {
                // the callee is a stub: the count stays where the test puts it
                wd(0x0252_7DC0, match rng.below(4) { 0 => 0x7d0, 1 => 0x7cf, 2 => 0x7d1 - rng.below(3) , _ => rng.below(0x7d0) });
            }
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_traffic_record_activate() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0215_D7A8, 8), (0x0215_DA30, 4)];
        cfg.arena_words = 512;
        drive("traffic_record_activate", Orig::new(&TRAFFIC_RECORD_ACTIVATE_BODY, &TRAFFIC_RECORD_ACTIVATE_CALLS, &TRAFFIC_RECORD_ACTIVATE_ABS, &TRAFFIC_RECORD_ACTIVATE_JMPS, 0x938c30), thunk_traffic_record_activate as *const () as usize, cfg, |w, rng| {
            // 8 records of 8 words: [0] next, [1] prev, [2] vehicle, [4] flags
            w.soup(rng);
            for i in 0..8usize {
                w.arena[i * 8] = w.p(rng.below(8) as usize * 8);
                w.arena[i * 8 + 1] = w.p(rng.below(8) as usize * 8);
                for k in 2..8usize {
                    w.arena[i * 8 + k] = rng.word();
                }
            }
            unsafe {
                wd(0x0215_D7AC, if rng.chance(4) { 0 } else { w.p(rng.below(8) as usize * 8) });
                wd(0x0215_DA30, if rng.chance(4) { 0 } else { w.p(rng.below(8) as usize * 8) });
                wd(0x0215_D7A8, rng.below(10));
            }
            let mut r = inr(rng);
            r.eax = w.p(rng.below(8) as usize * 8);
            // the original crashes when the record is already the head of the list it is pushed to
            unsafe {
                if rd(0x0215_DA30) == r.eax {
                    wd(0x0215_DA30, 0);
                }
            }
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_traffic_record_release() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0215_D7A8, 8), (0x0215_DA30, 4)];
        cfg.arena_words = 512;
        drive("traffic_record_release", Orig::new(&TRAFFIC_RECORD_RELEASE_BODY, &TRAFFIC_RECORD_RELEASE_CALLS, &TRAFFIC_RECORD_RELEASE_ABS, &TRAFFIC_RECORD_RELEASE_JMPS, 0x938cb0), thunk_traffic_record_release as *const () as usize, cfg, |w, rng| {
            // 8 records of 8 words: [0] next, [1] prev, [2] vehicle, [4] flags
            w.soup(rng);
            for i in 0..8usize {
                w.arena[i * 8] = w.p(rng.below(8) as usize * 8);
                w.arena[i * 8 + 1] = w.p(rng.below(8) as usize * 8);
                for k in 2..8usize {
                    w.arena[i * 8 + k] = rng.word();
                }
            }
            unsafe {
                wd(0x0215_D7AC, if rng.chance(4) { 0 } else { w.p(rng.below(8) as usize * 8) });
                wd(0x0215_DA30, if rng.chance(4) { 0 } else { w.p(rng.below(8) as usize * 8) });
                wd(0x0215_D7A8, rng.below(10));
            }
            let mut r = inr(rng);
            r.eax = w.p(rng.below(8) as usize * 8);
            // the original crashes when the record is already the head of the list it is pushed to
            unsafe {
                if rd(0x0215_D7AC) == r.eax {
                    wd(0x0215_D7AC, 0);
                }
            }
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_traffic_record_release_by_vehicle() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0215_DA30, 4)];
        cfg.stubs = vec![s(0x0093_8CB0, 0, 0b00001)];
        cfg.arena_words = 512;
        drive("traffic_record_release_by_vehicle", Orig::new(&TRAFFIC_RECORD_RELEASE_BY_VEHICLE_BODY, &TRAFFIC_RECORD_RELEASE_BY_VEHICLE_CALLS, &TRAFFIC_RECORD_RELEASE_BY_VEHICLE_ABS, &TRAFFIC_RECORD_RELEASE_BY_VEHICLE_JMPS, 0x93d910), thunk_traffic_record_release_by_vehicle as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // a circular list of n records
            let n = 1 + rng.below(6) as usize;
            for i in 0..n {
                w.arena[i * 8] = w.p(((i + 1) % n) * 8);
                w.arena[i * 8 + 2] = rng.below(5);
            }
            unsafe { wd(0x0215_DA30, if rng.chance(6) { 0 } else { w.p(0) }) };
            let mut r = inr(rng);
            r.edi = rng.below(5);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_tod_object_node_alloc() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x00F9_9924, 4)];
        cfg.arena_words = 256;
        drive("tod_object_node_alloc", Orig::new(&TOD_OBJECT_NODE_ALLOC_BODY, &TOD_OBJECT_NODE_ALLOC_CALLS, &TOD_OBJECT_NODE_ALLOC_ABS, &TOD_OBJECT_NODE_ALLOC_JMPS, 0x4d27e0), thunk_tod_object_node_alloc as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            for i in 0..8usize {
                w.arena[i * 8 + 4] = if rng.chance(4) { w.p(i * 8) } else { w.p(rng.below(8) as usize * 8) };
                w.arena[i * 8 + 5] = if rng.chance(3) { 0 } else { w.p(rng.below(8) as usize * 8) };
            }
            unsafe { wd(0x00F9_9924, if rng.chance(8) { 0 } else { w.p(rng.below(8) as usize * 8) }) };
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_ptr_array_compact() {

        let mut cfg = Cfg::new();
        cfg.out_ecx = true;
        cfg.out_edx = true;
        cfg.arena_words = 256;
        drive("ptr_array_compact", Orig::new(&PTR_ARRAY_COMPACT_BODY, &PTR_ARRAY_COMPACT_CALLS, &PTR_ARRAY_COMPACT_ABS, &PTR_ARRAY_COMPACT_JMPS, 0x523b80), thunk_ptr_array_compact as *const () as usize, cfg, |w, rng| {
            let n = rng.below(40) as usize;
            for i in 0..64usize {
                w.arena[i] = if rng.chance(3) { 0 } else { 0x1000 + rng.below(1000) };
            }
            let mut r = inr(rng);
            r.edx = w.p(rng.below(8) as usize);
            r.esi = if rng.chance(20) { 0xFFFF_FFFF } else { n as u32 };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_seen_flags_store_bits() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0145_A4A4, 8), (0x00DC_8558, 0x20)];
        cfg.arena_words = 1024;
        drive("cutscene_seen_flags_store_bits", Orig::new(&CUTSCENE_SEEN_FLAGS_STORE_BITS_BODY, &CUTSCENE_SEEN_FLAGS_STORE_BITS_CALLS, &CUTSCENE_SEEN_FLAGS_STORE_BITS_ABS, &CUTSCENE_SEEN_FLAGS_STORE_BITS_JMPS, 0x6c3ad0), thunk_cutscene_seen_flags_store_bits as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let n = match rng.below(10) { 0 => 0, 1 => 0xFFFF_FFFF, _ => 1 + rng.below(40) };
            unsafe {
                wd(0x0145_A4AC, n);
                wd(0x0145_A4A4, w.p(64));
            }
            // the bitmap holder: [esi] -> bitmap at arena word 512
            w.arena[8] = w.p(512);
            for i in 0..40usize {
                w.arena[64 + i * 5 + 2] = if rng.chance(2) { 0 } else { rng.below(300) };
            }
            let mut r = inr(rng);
            r.esi = w.p(8);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_ambient_vehicle_budget_remaining() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_8B48, 4), (0x0252_A0D4, 4)];
        cfg.stubs = vec![s(0x009D_7BD0, 0, 0)];
        cfg.palette = vec![0, 0, 1, 0x1234_5600, 0xFFFF_FFFF];
        drive("ambient_vehicle_budget_remaining", Orig::new(&AMBIENT_VEHICLE_BUDGET_REMAINING_BODY, &AMBIENT_VEHICLE_BUDGET_REMAINING_CALLS, &AMBIENT_VEHICLE_BUDGET_REMAINING_ABS, &AMBIENT_VEHICLE_BUDGET_REMAINING_JMPS, 0x93c1d0), thunk_ambient_vehicle_budget_remaining as *const () as usize, cfg, |_w, rng| {
            unsafe {
                wd(0x0252_8B48, rng.below(2) | (rng.u32() & 0xFFFF_FF00 & rng.u32()));
                wd(0x0252_A0D4, rng.below(40));
            }
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_current_spawn_group_query() {

        let fk = fake(6, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0215_E5BC, 4)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0xFFFF_FFFF, 5, 0x7FFF_FFFF, 0x8000_0000];
        cfg.arena_words = 256;
        drive("current_spawn_group_query", Orig::new(&CURRENT_SPAWN_GROUP_QUERY_BODY, &CURRENT_SPAWN_GROUP_QUERY_CALLS, &CURRENT_SPAWN_GROUP_QUERY_ABS, &CURRENT_SPAWN_GROUP_QUERY_JMPS, 0x957660), thunk_current_spawn_group_query as *const () as usize, cfg, |w, rng| {
            for k in 0..64usize {
                w.arena[k] = fk[rng.below(6) as usize] as u32;
            }
            w.arena[64] = w.p(0);
            unsafe { wd(0x0215_E5BC, if rng.chance(4) { 0 } else { w.p(64) }) };
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_current_spawn_group_vehicle_index() {

        let fk = fake(6, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0215_E5BC, 4), (0x0252_A0D8, 4)];
        cfg.stubs = vec![s(0x0095_7660, 0, 0)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0xFFFF_FFFF, 5, 3, 4, 0x7FFF_FFFF, 0x8000_0000, 2];
        cfg.arena_words = 256;
        drive("current_spawn_group_vehicle_index", Orig::new(&CURRENT_SPAWN_GROUP_VEHICLE_INDEX_BODY, &CURRENT_SPAWN_GROUP_VEHICLE_INDEX_CALLS, &CURRENT_SPAWN_GROUP_VEHICLE_INDEX_ABS, &CURRENT_SPAWN_GROUP_VEHICLE_INDEX_JMPS, 0x9392a0), thunk_current_spawn_group_vehicle_index as *const () as usize, cfg, |w, rng| {
            for k in 0..64usize {
                w.arena[k] = fk[rng.below(6) as usize] as u32;
            }
            w.arena[64] = w.p(0);
            unsafe {
                wd(0x0215_E5BC, if rng.chance(5) { 0 } else { w.p(64) });
                wd(0x0252_A0D8, rng.below(8));
            }
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_human_is_non_ally_state7() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![sk(0x0098_E8A0, 0, 0b00100)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0xFF, 0x1234_5600, 0x1234_5601];
        cfg.arena_words = 2048;
        drive("human_is_non_ally_state7", Orig::new(&HUMAN_IS_NON_ALLY_STATE7_BODY, &HUMAN_IS_NON_ALLY_STATE7_CALLS, &HUMAN_IS_NON_ALLY_STATE7_ABS, &HUMAN_IS_NON_ALLY_STATE7_JMPS, 0x9636c0), thunk_human_is_non_ally_state7 as *const () as usize, cfg, |w, rng| {
            let mut r = inr(rng);
            r.edx = w.p(0) + 4 * rng.below(256);
            unsafe { wb(r.edx as usize + 0x111c, if rng.chance(2) { 7 } else { rng.below(10) as u8 }) };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_human_is_follower() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x0098_E7C0, 0, 0b10000)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0xFF, 0x1234_5600];
        drive("human_is_follower", Orig::new(&HUMAN_IS_FOLLOWER_BODY, &HUMAN_IS_FOLLOWER_CALLS, &HUMAN_IS_FOLLOWER_ABS, &HUMAN_IS_FOLLOWER_JMPS, 0x98e880), thunk_human_is_follower as *const () as usize, cfg, |w, rng| {
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { rng.word() };
            // sometimes the leader is the human itself
            *PALETTE.lock().unwrap() = if rng.chance(2) { vec![r.eax, 0, 1] } else { vec![0, 1, 0x100, r.eax, 0x1234_5600] };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_human_action_flag_bit4() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE90E48, 0x400)];
        cfg.stubs = vec![s(0x0096_F3A0, 1, 0)];
        cfg.palette = vec![0xFFFF_FFFF, 0xFFFF_FFFF, 0, 1, 2, 3, 10, 50, 80];
        drive("human_action_flag_bit4", Orig::new(&HUMAN_ACTION_FLAG_BIT4_BODY, &HUMAN_ACTION_FLAG_BIT4_CALLS, &HUMAN_ACTION_FLAG_BIT4_ABS, &HUMAN_ACTION_FLAG_BIT4_JMPS, 0x96d930), thunk_human_action_flag_bit4 as *const () as usize, cfg, |w, rng| {
            w.rand_region(0xE90E48, 0x400, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_weather_force_stage_and_sync() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_6DD0, 4), (0x0252_6E10, 4), (0xE843B4, 4), (0xE9D460, 4)];
        cfg.stubs = vec![s(0x004D_E390, 0, 0)];
        drive("weather_force_stage_and_sync", Orig::new(&WEATHER_FORCE_STAGE_AND_SYNC_BODY, &WEATHER_FORCE_STAGE_AND_SYNC_CALLS, &WEATHER_FORCE_STAGE_AND_SYNC_ABS, &WEATHER_FORCE_STAGE_AND_SYNC_JMPS, 0x4ded80), thunk_weather_force_stage_and_sync as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x0252_6DD0, 0x48, rng);
            unsafe { wd(0xE843B4, match rng.below(4) { 0 => 0x6B49_D200, 1 => 0x6B49_D201, 2 => rng.u32() & 0x7FFF_FFFF, _ => rng.u32() }) };
            let mut r = inr(rng);
            r.eax = rng.below(30);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_input_actions_reset() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xF9EA38, 0x840), (0xF85498, 4)];
        cfg.stubs = vec![s(0x004F_F3F0, 0, 0)];
        drive("input_actions_reset", Orig::new(&INPUT_ACTIONS_RESET_BODY, &INPUT_ACTIONS_RESET_CALLS, &INPUT_ACTIONS_RESET_ABS, &INPUT_ACTIONS_RESET_JMPS, 0x500170), thunk_input_actions_reset as *const () as usize, cfg, |w, rng| {
            w.rand_region(0xF9EA38, 0x840, rng);
            w.rand_region(0xF85498, 4, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list0_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x277d16c, 0x782c), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x5ff, 0x600, 0x601, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list0_add", Orig::new(&RENDER_LIST0_ADD_BODY, &RENDER_LIST0_ADD_CALLS, &RENDER_LIST0_ADD_ABS, &RENDER_LIST0_ADD_JMPS, 0x52fda0), thunk_render_list0_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x277d16c, 0x782c, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list0b_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x2784974, 0x502c), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x3ff, 0x400, 0x401, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list0b_add", Orig::new(&RENDER_LIST0B_ADD_BODY, &RENDER_LIST0B_ADD_CALLS, &RENDER_LIST0B_ADD_ABS, &RENDER_LIST0B_ADD_JMPS, 0x52fe10), thunk_render_list0b_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x2784974, 0x502c, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list_vis_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x278997c, 0x282c), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x1ff, 0x200, 0x201, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list_vis_add", Orig::new(&RENDER_LIST_VIS_ADD_BODY, &RENDER_LIST_VIS_ADD_CALLS, &RENDER_LIST_VIS_ADD_ABS, &RENDER_LIST_VIS_ADD_JMPS, 0x52fe80), thunk_render_list_vis_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x278997c, 0x282c, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list3_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x278e994, 0x2ac), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x1f, 0x20, 0x21, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list3_add", Orig::new(&RENDER_LIST3_ADD_BODY, &RENDER_LIST3_ADD_CALLS, &RENDER_LIST3_ADD_ABS, &RENDER_LIST3_ADD_JMPS, 0x52ff60), thunk_render_list3_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x278e994, 0x2ac, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list4_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x278ec1c, 0x16c), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0xf, 0x10, 0x11, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list4_add", Orig::new(&RENDER_LIST4_ADD_BODY, &RENDER_LIST4_ADD_CALLS, &RENDER_LIST4_ADD_ABS, &RENDER_LIST4_ADD_JMPS, 0x52ffd0), thunk_render_list4_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x278ec1c, 0x16c, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list6_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x279016c, 0xbe4), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x95, 0x96, 0x97, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list6_add", Orig::new(&RENDER_LIST6_ADD_BODY, &RENDER_LIST6_ADD_CALLS, &RENDER_LIST6_ADD_ABS, &RENDER_LIST6_ADD_JMPS, 0x530030), thunk_render_list6_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x279016c, 0xbe4, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list7_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x2790d2c, 0xb1c), (0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0x8b, 0x8c, 0x8d, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        drive("render_list7_add", Orig::new(&RENDER_LIST7_ADD_BODY, &RENDER_LIST7_ADD_CALLS, &RENDER_LIST7_ADD_ABS, &RENDER_LIST7_ADD_JMPS, 0x5300a0), thunk_render_list7_add as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x2790d2c, 0xb1c, rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            (inr(rng), vec![rng.word(), rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_render_list_small_add() {

        let fk = fake(1, 1, 1, 0);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xDB80A4, 4)];
        cfg.palette = vec![1, 2, 0xFF, 0x100, 0x101, 0x7FFF_FFFF, 0xFFFF_FFFF, 3];
        cfg.arena_words = 2048;
        drive("render_list_small_add", Orig::new(&RENDER_LIST_SMALL_ADD_BODY, &RENDER_LIST_SMALL_ADD_CALLS, &RENDER_LIST_SMALL_ADD_ABS, &RENDER_LIST_SMALL_ADD_JMPS, 0x52fef0), thunk_render_list_small_add as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            unsafe { wd(0xDB80A4, fk[0] as u32) };
            let mut r = inr(rng);
            r.edi = w.p(0);
            (r, vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_weapon_primary() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), s(0x00B7_16E0, 1, 0)];
        cfg.arena_words = 256;
        
        drive("unlockable_parse_weapon_primary", Orig::new(&UNLOCKABLE_PARSE_WEAPON_PRIMARY_BODY, &UNLOCKABLE_PARSE_WEAPON_PRIMARY_CALLS, &UNLOCKABLE_PARSE_WEAPON_PRIMARY_ABS, &UNLOCKABLE_PARSE_WEAPON_PRIMARY_JMPS, 0x6bb3e0), thunk_unlockable_parse_weapon_primary as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_weapon_secondary() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), s(0x00B7_16E0, 1, 0)];
        cfg.arena_words = 256;
        
        drive("unlockable_parse_weapon_secondary", Orig::new(&UNLOCKABLE_PARSE_WEAPON_SECONDARY_BODY, &UNLOCKABLE_PARSE_WEAPON_SECONDARY_CALLS, &UNLOCKABLE_PARSE_WEAPON_SECONDARY_ABS, &UNLOCKABLE_PARSE_WEAPON_SECONDARY_JMPS, 0x6bb420), thunk_unlockable_parse_weapon_secondary as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_contact() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), s(0x0078_6FB0, 0, 0b00001)];
        cfg.arena_words = 256;
        
        drive("unlockable_parse_contact", Orig::new(&UNLOCKABLE_PARSE_CONTACT_BODY, &UNLOCKABLE_PARSE_CONTACT_CALLS, &UNLOCKABLE_PARSE_CONTACT_ABS, &UNLOCKABLE_PARSE_CONTACT_JMPS, 0x6bb390), thunk_unlockable_parse_contact as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_crib_ref() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), s(0x0054_8330, 0, 0b00010)];
        cfg.arena_words = 256;
        
        drive("unlockable_parse_crib_ref", Orig::new(&UNLOCKABLE_PARSE_CRIB_REF_BODY, &UNLOCKABLE_PARSE_CRIB_REF_CALLS, &UNLOCKABLE_PARSE_CRIB_REF_ABS, &UNLOCKABLE_PARSE_CRIB_REF_JMPS, 0x6bba90), thunk_unlockable_parse_crib_ref as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_gang_type() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), so(0x00BD_CB30, 0b00010)];
        cfg.arena_words = 256;
        cfg.eax_lo8 = true;
        drive("unlockable_parse_gang_type", Orig::new(&UNLOCKABLE_PARSE_GANG_TYPE_BODY, &UNLOCKABLE_PARSE_GANG_TYPE_CALLS, &UNLOCKABLE_PARSE_GANG_TYPE_ABS, &UNLOCKABLE_PARSE_GANG_TYPE_JMPS, 0x6bbb40), thunk_unlockable_parse_gang_type as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_vehicle_group() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), so(0x00BD_CB30, 0b00010)];
        cfg.arena_words = 256;
        cfg.eax_lo8 = true;
        drive("unlockable_parse_vehicle_group", Orig::new(&UNLOCKABLE_PARSE_VEHICLE_GROUP_BODY, &UNLOCKABLE_PARSE_VEHICLE_GROUP_CALLS, &UNLOCKABLE_PARSE_VEHICLE_GROUP_ABS, &UNLOCKABLE_PARSE_VEHICLE_GROUP_JMPS, 0x6bbb80), thunk_unlockable_parse_vehicle_group as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_style() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), so(0x00BD_CB30, 0b00010)];
        cfg.arena_words = 256;
        cfg.eax_lo8 = true;
        drive("unlockable_parse_style", Orig::new(&UNLOCKABLE_PARSE_STYLE_BODY, &UNLOCKABLE_PARSE_STYLE_CALLS, &UNLOCKABLE_PARSE_STYLE_ABS, &UNLOCKABLE_PARSE_STYLE_JMPS, 0x6bbbc0), thunk_unlockable_parse_style as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_unlockable_parse_item() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00BF_BEE0, 0, 0b10001).p2(), so(0x00BD_CB30, 0b00010), s(0x0091_C290, 0, 0)];
        cfg.arena_words = 256;
        
        drive("unlockable_parse_item", Orig::new(&UNLOCKABLE_PARSE_ITEM_BODY, &UNLOCKABLE_PARSE_ITEM_CALLS, &UNLOCKABLE_PARSE_ITEM_ABS, &UNLOCKABLE_PARSE_ITEM_JMPS, 0x6bb460), thunk_unlockable_parse_item as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            *PALETTE2.lock().unwrap() = vec![0, 0, w.p(8), w.p(16), w.p(24)];
            *PALETTE.lock().unwrap() = vec![0, 0, 0xFFFF_FFFF, 1, 0x1234_5678, 7];
            let mut r = inr(rng);
            r.eax = if rng.chance(6) { 0 } else { w.p(40) };
            (r, vec![w.p(100)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_act_heli_text() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x007F_49E0, 0, 0b11000)];
        drive("act_heli_text", Orig::new(&ACT_HELI_TEXT_BODY, &ACT_HELI_TEXT_CALLS, &ACT_HELI_TEXT_ABS, &ACT_HELI_TEXT_JMPS, 0x576e60), thunk_act_heli_text as *const () as usize, cfg, |_w, rng| {
            let mut r = inr(rng);
            r.eax = match rng.below(4) { 0 => 0, 1 => 2, 2 => rng.below(5), _ => rng.word() };
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_act_holdup_start_msg_text() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0140_C7D4, 0x14), (0xEAA79C, 4)];
        cfg.stubs = vec![s(0x007F_49E0, 0, 0b11000)];
        drive("act_holdup_start_msg_text", Orig::new(&ACT_HOLDUP_START_MSG_TEXT_BODY, &ACT_HOLDUP_START_MSG_TEXT_CALLS, &ACT_HOLDUP_START_MSG_TEXT_ABS, &ACT_HOLDUP_START_MSG_TEXT_JMPS, 0x655700), thunk_act_holdup_start_msg_text as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x0140_C7D4, 0x14, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_act_level_text() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7414, 0x18)];
        cfg.stubs = vec![s(0x007F_4AD0, 0, 0b11000)];
        drive("act_level_text", Orig::new(&ACT_LEVEL_TEXT_BODY, &ACT_LEVEL_TEXT_CALLS, &ACT_LEVEL_TEXT_ABS, &ACT_LEVEL_TEXT_JMPS, 0x53a7a0), thunk_act_level_text as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x0252_7414, 0x18, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_object_is_cutscene_vehicle() {

        let fk = fake(8, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7D10, 8)];
        cfg.arena_words = 4096;
        drive("cutscene_object_is_cutscene_vehicle", Orig::new(&CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_BODY, &CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_CALLS, &CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_ABS, &CUTSCENE_OBJECT_IS_CUTSCENE_VEHICLE_JMPS, 0x6c6ee0), thunk_cutscene_object_is_cutscene_vehicle as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // manager at word 0 (+0xae0 = word 696 -> array pointer, +0xae8 = word 698 -> count)
            let n = rng.below(5);
            w.arena[696] = w.p(1000);
            w.arena[698] = if rng.chance(12) { 0xFFFF_FFFF } else { n };
            // 4 objects at words 1100.., each with a vtable at 1300.. (slot 0x34/4 = 13)
            for o in 0..4usize {
                w.arena[1000 + o] = w.p(1100 + o * 64);
                w.arena[1100 + o * 64] = w.p(1300 + o * 64);
                for k in 0..32usize {
                    w.arena[1300 + o * 64 + k] = fk[rng.below(8) as usize] as u32;
                }
            }
            // two candidate result objects with ids
            let id = rng.below(3);
            w.arena[1600 + 0x44 / 4] = id;
            w.arena[1700 + 0x44 / 4] = if rng.chance(2) { id } else { id + 1 };
            *PALETTE.lock().unwrap() = vec![w.p(1600), w.p(1700), w.p(1600), w.p(1700)];
            // the argument object (edi)
            w.arena[1800 + 0x44 / 4] = id;
            unsafe {
                wd(0x0252_7D10, w.p(0));
                wd(0x0252_7D14, rng.below(2));
            }
            let mut r = inr(rng);
            r.edi = w.p(1800);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_zoom() {

        let mut cfg = Cfg::new();
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_zoom", Orig::new(&CAMERA_CTX_PRED_ZOOM_BODY, &CAMERA_CTX_PRED_ZOOM_CALLS, &CAMERA_CTX_PRED_ZOOM_ABS, &CAMERA_CTX_PRED_ZOOM_JMPS, 0x49a870), thunk_camera_ctx_pred_zoom as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let h = 8 + rng.below(8) as usize;
            let hp = w.p(h);
            let base = h; // word index of the human
            w.arena[base + 0x4c / 4] = if rng.chance(5) { rng.below(3) } else { 1 };
            let o = (hp as usize + 0x3cc) as *mut u8;
            unsafe {
                *o = if rng.chance(5) { rng.below(3) as u8 } else { 1 };
                *((hp as usize + 0x1161) as *mut u8) = rng.u32() as u8;
                *((hp as usize + 0x1163) as *mut u8) = rng.u32() as u8;
            }
            w.arena[base + 0xc0 / 4] = rng.u32();
            let mut r = inr(rng);
            r.ecx = if rng.chance(8) { 0 } else { hp };
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_fine_aim() {

        let mut cfg = Cfg::new();
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_fine_aim", Orig::new(&CAMERA_CTX_PRED_FINE_AIM_BODY, &CAMERA_CTX_PRED_FINE_AIM_CALLS, &CAMERA_CTX_PRED_FINE_AIM_ABS, &CAMERA_CTX_PRED_FINE_AIM_JMPS, 0x49a920), thunk_camera_ctx_pred_fine_aim as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let h = 8 + rng.below(8) as usize;
            let hp = w.p(h);
            let base = h; // word index of the human
            w.arena[base + 0x4c / 4] = if rng.chance(5) { rng.below(3) } else { 1 };
            let o = (hp as usize + 0x3cc) as *mut u8;
            unsafe {
                *o = if rng.chance(5) { rng.below(3) as u8 } else { 1 };
                *((hp as usize + 0x1161) as *mut u8) = rng.u32() as u8;
                *((hp as usize + 0x1163) as *mut u8) = rng.u32() as u8;
            }
            w.arena[base + 0xc0 / 4] = rng.u32();
            let mut r = inr(rng);
            r.ecx = if rng.chance(8) { 0 } else { hp };
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_melee_lock() {

        let mut cfg = Cfg::new();
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_melee_lock", Orig::new(&CAMERA_CTX_PRED_MELEE_LOCK_BODY, &CAMERA_CTX_PRED_MELEE_LOCK_CALLS, &CAMERA_CTX_PRED_MELEE_LOCK_ABS, &CAMERA_CTX_PRED_MELEE_LOCK_JMPS, 0x49a980), thunk_camera_ctx_pred_melee_lock as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let h = 8 + rng.below(8) as usize;
            let hp = w.p(h);
            let base = h; // word index of the human
            w.arena[base + 0x4c / 4] = if rng.chance(5) { rng.below(3) } else { 1 };
            let o = (hp as usize + 0x3cc) as *mut u8;
            unsafe {
                *o = if rng.chance(5) { rng.below(3) as u8 } else { 1 };
                *((hp as usize + 0x1161) as *mut u8) = rng.u32() as u8;
                *((hp as usize + 0x1163) as *mut u8) = rng.u32() as u8;
            }
            w.arena[base + 0xc0 / 4] = rng.u32();
            let mut r = inr(rng);
            r.ecx = if rng.chance(8) { 0 } else { hp };
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_fine_aim_crouch() {

        let mut cfg = Cfg::new();
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_fine_aim_crouch", Orig::new(&CAMERA_CTX_PRED_FINE_AIM_CROUCH_BODY, &CAMERA_CTX_PRED_FINE_AIM_CROUCH_CALLS, &CAMERA_CTX_PRED_FINE_AIM_CROUCH_ABS, &CAMERA_CTX_PRED_FINE_AIM_CROUCH_JMPS, 0x49a950), thunk_camera_ctx_pred_fine_aim_crouch as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let h = 8 + rng.below(8) as usize;
            let hp = w.p(h);
            let base = h; // word index of the human
            w.arena[base + 0x4c / 4] = if rng.chance(5) { rng.below(3) } else { 1 };
            let o = (hp as usize + 0x3cc) as *mut u8;
            unsafe {
                *o = if rng.chance(5) { rng.below(3) as u8 } else { 1 };
                *((hp as usize + 0x1161) as *mut u8) = rng.u32() as u8;
                *((hp as usize + 0x1163) as *mut u8) = rng.u32() as u8;
            }
            w.arena[base + 0xc0 / 4] = rng.u32();
            let mut r = inr(rng);
            r.ecx = if rng.chance(8) { 0 } else { hp };
            (r, vec![])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_freefall() {

        let cfg = Cfg::new();
        drive("camera_ctx_pred_freefall", Orig::new(&CAMERA_CTX_PRED_FREEFALL_BODY, &CAMERA_CTX_PRED_FREEFALL_CALLS, &CAMERA_CTX_PRED_FREEFALL_ABS, &CAMERA_CTX_PRED_FREEFALL_JMPS, 0x49aa80), thunk_camera_ctx_pred_freefall as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let mut r = inr(rng);
            r.eax = w.p(rng.below(100) as usize);
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_falling() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0xE90E44, 0x400)];
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_falling", Orig::new(&CAMERA_CTX_PRED_FALLING_BODY, &CAMERA_CTX_PRED_FALLING_CALLS, &CAMERA_CTX_PRED_FALLING_ABS, &CAMERA_CTX_PRED_FALLING_JMPS, 0x49a8c0), thunk_camera_ctx_pred_falling as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            unsafe {
                for i in 0..0x100usize {
                    let v = match rng.below(8) { 0 => 0x53b, 1 => 0x53c, 2 => 0x53d, 3 => 0x53e, 4 => 0x53f, 5 => 0x53a, _ => rng.u32() };
                    wd(0xE90E44 + i * 4, v);
                }
            }
            let h = 8 + rng.below(8) as usize;
            let hp = w.p(h);
            w.arena[h + 0x5e0 / 4] = w.p(600);
            w.arena[600 + 0x164 / 4] = match rng.below(6) { 0 => 0xFFFF_FFFF, 1 => 0x39 + rng.below(8), 2 => 0x38, _ => rng.below(0x50) };
            unsafe {
                *((hp as usize + 0x591) as *mut u8) = if rng.chance(2) { 6 } else { rng.u32() as u8 };
            }
            w.arena[h + 0xc8 / 4] = rng.u32();
            let mut r = inr(rng);
            r.edx = hp;
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_sprint() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0217_03D4, 4)];
        cfg.stubs = vec![s(0x0096_0120, 0, 0b01001), s(0x004A_B430, 0, 0b11000)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0x1234_5600];
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_sprint", Orig::new(&CAMERA_CTX_PRED_SPRINT_BODY, &CAMERA_CTX_PRED_SPRINT_CALLS, &CAMERA_CTX_PRED_SPRINT_ABS, &CAMERA_CTX_PRED_SPRINT_JMPS, 0x49aa00), thunk_camera_ctx_pred_sprint as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let mut r = inr(rng);
            r.eax = w.p(8 + rng.below(8) as usize);
            unsafe {
                wd(0x0217_03D4, if rng.chance(3) { 0 } else { w.p(300) });
                wd(r.eax as usize + 0xea8, if rng.chance(3) { 0 } else { rng.u32() });
            }
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_camera_ctx_pred_interior_sprint() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0217_03D4, 4)];
        cfg.stubs = vec![s(0x0096_0120, 0, 0b01001), s(0x004A_B430, 0, 0b11000)];
        cfg.palette = vec![0, 1, 0x100, 0x101, 0x1234_5600];
        cfg.arena_words = 2048;
        drive("camera_ctx_pred_interior_sprint", Orig::new(&CAMERA_CTX_PRED_INTERIOR_SPRINT_BODY, &CAMERA_CTX_PRED_INTERIOR_SPRINT_CALLS, &CAMERA_CTX_PRED_INTERIOR_SPRINT_ABS, &CAMERA_CTX_PRED_INTERIOR_SPRINT_JMPS, 0x49aa40), thunk_camera_ctx_pred_interior_sprint as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            let mut r = inr(rng);
            r.eax = w.p(8 + rng.below(8) as usize);
            unsafe {
                wd(0x0217_03D4, if rng.chance(3) { 0 } else { w.p(300) });
                wd(r.eax as usize + 0xea8, if rng.chance(3) { 0 } else { rng.u32() });
            }
            (r, vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_audio_job_workers_mode_reset() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x033D_6430, 4), (0x033D_6500, 4)];
        cfg.stubs = vec![s(0x00BF_3760, 0, 0b01000)];
        drive("audio_job_workers_mode_reset", Orig::new(&AUDIO_JOB_WORKERS_MODE_RESET_BODY, &AUDIO_JOB_WORKERS_MODE_RESET_CALLS, &AUDIO_JOB_WORKERS_MODE_RESET_ABS, &AUDIO_JOB_WORKERS_MODE_RESET_JMPS, 0x464fe0), thunk_audio_job_workers_mode_reset as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x033D_6430, 4, rng);
            w.rand_region(0x033D_6500, 4, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_havok_prestep_then_raycast() {

        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x006F_F670, 0, 0b11000), s(0x006F_A330, 2, 0b00010)];
        drive("havok_prestep_then_raycast", Orig::new(&HAVOK_PRESTEP_THEN_RAYCAST_BODY, &HAVOK_PRESTEP_THEN_RAYCAST_CALLS, &HAVOK_PRESTEP_THEN_RAYCAST_ABS, &HAVOK_PRESTEP_THEN_RAYCAST_JMPS, 0x6f5d40), thunk_havok_prestep_then_raycast as *const () as usize, cfg, |_w, rng| {
            (inr(rng), vec![rng.word(), rng.word(), rng.word()])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_havok_entity_refresh_collision_filter() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7DA8, 8), (0x0252_7D94, 4)];
        cfg.stubs = vec![s(0x006F_5740, 0, 0b00110), sp(0x00D5_CC40, 3, 0b00110)];
        drive("havok_entity_refresh_collision_filter", Orig::new(&HAVOK_ENTITY_REFRESH_COLLISION_FILTER_BODY, &HAVOK_ENTITY_REFRESH_COLLISION_FILTER_CALLS, &HAVOK_ENTITY_REFRESH_COLLISION_FILTER_ABS, &HAVOK_ENTITY_REFRESH_COLLISION_FILTER_JMPS, 0x6fc720), thunk_havok_entity_refresh_collision_filter as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x0252_7DA8, 8, rng);
            unsafe { wd(0x0252_7DAC, match rng.below(3) { 0 => 0, 1 => 1, _ => rng.u32() }) };
            w.rand_region(0x0252_7D94, 4, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_video_stop() {

        let mut cfg = Cfg::new();
        cfg.regions = vec![(0x0252_7B74, 4), (0xE94D3C, 4)];
        cfg.stubs = vec![s(0x0068_88D0, 0, 0), s(0x0047_F170, 0, 0), s(0x0052_2B80, 1, 0), s(0x00A8_0CC0, 0, 0b00001)];
        drive("video_stop", Orig::new(&VIDEO_STOP_BODY, &VIDEO_STOP_CALLS, &VIDEO_STOP_ABS, &VIDEO_STOP_JMPS, 0x689130), thunk_video_stop as *const () as usize, cfg, |w, rng| {
            w.rand_region(0x0252_7B74, 4, rng);
            w.rand_region(0xE94D3C, 4, rng);
            (inr(rng), vec![])
        });
    
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_action_door_open() {

        let fk = fake(4, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00A5_B6E0, 0, 0b00001).p2(), s(0x0091_B590, 1, 0b00010), s(0x0091_AD80, 2, 0b00010)];
        cfg.arena_words = 1024;
        drive("cutscene_action_door_open", Orig::new(&CUTSCENE_ACTION_DOOR_OPEN_BODY, &CUTSCENE_ACTION_DOOR_OPEN_CALLS, &CUTSCENE_ACTION_DOOR_OPEN_ABS, &CUTSCENE_ACTION_DOOR_OPEN_JMPS, 0x6cc5a0), thunk_cutscene_action_door_open as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // the action at word 0: name pointers at +8 and +0x10
            w.arena[2] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            w.arena[4] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            // doors at words 200..: [door] vtable, [door + 0x94] pointer, [ptr + 0xa] flags
            for d in 0..4usize {
                w.arena[200 + d * 64] = w.p(500 + d * 16);
                for k in 0..16usize {
                    w.arena[500 + d * 16 + k] = fk[rng.below(4) as usize] as u32;
                }
                w.arena[200 + d * 64 + 0x94 / 4] = if rng.chance(4) { 0 } else { w.p(800 + rng.below(4) as usize) };
            }
            for k in 0..4usize {
                w.arena[800 + k] = rng.u32();
            }
            *PALETTE2.lock().unwrap() = vec![0, w.p(200), w.p(264), w.p(328), w.p(392)];
            *PALETTE.lock().unwrap() = vec![0, 1, 0x100, 0x1234_5678];
            (inr(rng), vec![w.p(0)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_action_door_close() {

        let fk = fake(4, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00A5_B6E0, 0, 0b00001).p2(), s(0x0091_B590, 1, 0b00010), s(0x0091_B140, 1, 0b00010)];
        cfg.arena_words = 1024;
        drive("cutscene_action_door_close", Orig::new(&CUTSCENE_ACTION_DOOR_CLOSE_BODY, &CUTSCENE_ACTION_DOOR_CLOSE_CALLS, &CUTSCENE_ACTION_DOOR_CLOSE_ABS, &CUTSCENE_ACTION_DOOR_CLOSE_JMPS, 0x6cc5e0), thunk_cutscene_action_door_close as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // the action at word 0: name pointers at +8 and +0x10
            w.arena[2] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            w.arena[4] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            // doors at words 200..: [door] vtable, [door + 0x94] pointer, [ptr + 0xa] flags
            for d in 0..4usize {
                w.arena[200 + d * 64] = w.p(500 + d * 16);
                for k in 0..16usize {
                    w.arena[500 + d * 16 + k] = fk[rng.below(4) as usize] as u32;
                }
                w.arena[200 + d * 64 + 0x94 / 4] = if rng.chance(4) { 0 } else { w.p(800 + rng.below(4) as usize) };
            }
            for k in 0..4usize {
                w.arena[800 + k] = rng.u32();
            }
            *PALETTE2.lock().unwrap() = vec![0, w.p(200), w.p(264), w.p(328), w.p(392)];
            *PALETTE.lock().unwrap() = vec![0, 1, 0x100, 0x1234_5678];
            (inr(rng), vec![w.p(0)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_action_mesh_mover_play_update() {

        let fk = fake(4, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00A5_B6E0, 0, 0b00001).p2(), s(0x0091_B590, 1, 0b00010)];
        cfg.arena_words = 1024;
        drive("cutscene_action_mesh_mover_play_update", Orig::new(&CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_BODY, &CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_CALLS, &CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_ABS, &CUTSCENE_ACTION_MESH_MOVER_PLAY_UPDATE_JMPS, 0x6cc7a0), thunk_cutscene_action_mesh_mover_play_update as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // the action at word 0: name pointers at +8 and +0x10
            w.arena[2] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            w.arena[4] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            // doors at words 200..: [door] vtable, [door + 0x94] pointer, [ptr + 0xa] flags
            for d in 0..4usize {
                w.arena[200 + d * 64] = w.p(500 + d * 16);
                for k in 0..16usize {
                    w.arena[500 + d * 16 + k] = fk[rng.below(4) as usize] as u32;
                }
                w.arena[200 + d * 64 + 0x94 / 4] = if rng.chance(4) { 0 } else { w.p(800 + rng.below(4) as usize) };
            }
            for k in 0..4usize {
                w.arena[800 + k] = rng.u32();
            }
            *PALETTE2.lock().unwrap() = vec![0, w.p(200), w.p(264), w.p(328), w.p(392)];
            *PALETTE.lock().unwrap() = vec![0, 1, 0x100, 0x1234_5678];
            (inr(rng), vec![w.p(0)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_action_mesh_mover_stop() {

        let fk = fake(4, 0, 0, 0b00010);
        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00A5_B6E0, 0, 0b00001).p2(), s(0x00A5_BF40, 1, 0b00010)];
        cfg.arena_words = 1024;
        drive("cutscene_action_mesh_mover_stop", Orig::new(&CUTSCENE_ACTION_MESH_MOVER_STOP_BODY, &CUTSCENE_ACTION_MESH_MOVER_STOP_CALLS, &CUTSCENE_ACTION_MESH_MOVER_STOP_ABS, &CUTSCENE_ACTION_MESH_MOVER_STOP_JMPS, 0x6cc7d0), thunk_cutscene_action_mesh_mover_stop as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // the action at word 0: name pointers at +8 and +0x10
            w.arena[2] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            w.arena[4] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            // doors at words 200..: [door] vtable, [door + 0x94] pointer, [ptr + 0xa] flags
            for d in 0..4usize {
                w.arena[200 + d * 64] = w.p(500 + d * 16);
                for k in 0..16usize {
                    w.arena[500 + d * 16 + k] = fk[rng.below(4) as usize] as u32;
                }
                w.arena[200 + d * 64 + 0x94 / 4] = if rng.chance(4) { 0 } else { w.p(800 + rng.below(4) as usize) };
            }
            for k in 0..4usize {
                w.arena[800 + k] = rng.u32();
            }
            *PALETTE2.lock().unwrap() = vec![0, w.p(200), w.p(264), w.p(328), w.p(392)];
            *PALETTE.lock().unwrap() = vec![0, 1, 0x100, 0x1234_5678];
            (inr(rng), vec![w.p(0)])
        });

    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn t_cutscene_action_mesh_mover_reset() {

        let fk = fake(4, 0, 1, 0b00010);
        let mut cfg = Cfg::new();
        cfg.stubs = vec![s(0x00A5_B6E0, 0, 0b00001).p2(), s(0x0091_B590, 1, 0)];
        cfg.arena_words = 1024;
        drive("cutscene_action_mesh_mover_reset", Orig::new(&CUTSCENE_ACTION_MESH_MOVER_RESET_BODY, &CUTSCENE_ACTION_MESH_MOVER_RESET_CALLS, &CUTSCENE_ACTION_MESH_MOVER_RESET_ABS, &CUTSCENE_ACTION_MESH_MOVER_RESET_JMPS, 0x6cc7f0), thunk_cutscene_action_mesh_mover_reset as *const () as usize, cfg, |w, rng| {
            w.soup(rng);
            // the action at word 0: name pointers at +8 and +0x10
            w.arena[2] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            w.arena[4] = if rng.chance(4) { 0 } else { w.p(100 + rng.below(8) as usize) };
            // doors at words 200..: [door] vtable, [door + 0x94] pointer, [ptr + 0xa] flags
            for d in 0..4usize {
                w.arena[200 + d * 64] = w.p(500 + d * 16);
                for k in 0..16usize {
                    w.arena[500 + d * 16 + k] = fk[rng.below(4) as usize] as u32;
                }
                w.arena[200 + d * 64 + 0x94 / 4] = if rng.chance(4) { 0 } else { w.p(800 + rng.below(4) as usize) };
            }
            for k in 0..4usize {
                w.arena[800 + k] = rng.u32();
            }
            *PALETTE2.lock().unwrap() = vec![0, w.p(200), w.p(264), w.p(328), w.p(392)];
            *PALETTE.lock().unwrap() = vec![0, 1, 0x100, 0x1234_5678];
            (inr(rng), vec![w.p(0)])
        });

    }
}
