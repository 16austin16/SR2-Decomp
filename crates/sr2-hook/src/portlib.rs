//! Shared helpers for the ported game functions: raw memory access, calling the game's
//! functions with their LTCG register conventions, the two circular-list walk shapes, and the
//! naked thunks that keep ECX / EDX around a Rust body (callers of LTCG leaf functions may keep
//! live values there).
//!
//! Conventions used throughout the ports:
//! - `call_regs(f, eax, ecx, edx, esi, edi, &[stack args...])` calls a game function with those
//!   registers set and the arguments pushed cdecl (last pushed first). For a stdcall callee that
//!   pops its own arguments, pass them the same way; `call_regs` restores ESP itself.
//! - Functions that return on the x87 stack: `call_f32`.
//! - The game runs the x87 at 53-bit precision, so f64 arithmetic reproduces it; every `fstp
//!   dword` round trip in the original is an `as f32` here.
//! - A port is pinned to the original's whole body with `detour::replace(addr, &BODY, thunk)`, so
//!   a different build leaves the game untouched.

#![allow(dead_code, unused_macros, unused_imports)]

pub(crate) use crate::gfx_cmd::call_regs;

pub(crate) unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

pub(crate) unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

pub(crate) unsafe fn rf(p: usize) -> f32 {
    unsafe { *(p as *const f32) }
}

pub(crate) unsafe fn wd(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}

pub(crate) unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}

pub(crate) unsafe fn wf(p: usize, v: f32) {
    unsafe { *(p as *mut f32) = v }
}

/// A volatile byte read, for flags another thread writes.
pub(crate) unsafe fn vb(p: usize) -> u8 {
    unsafe { std::ptr::read_volatile(p as *const u8) }
}

pub(crate) unsafe fn copy_dwords(dst: usize, src: usize, n: usize) {
    unsafe { std::ptr::copy(src as *const u32, dst as *mut u32, n) }
}

/// Virtual method `slot` of `obj`.
pub(crate) unsafe fn vslot(obj: u32, slot: usize) -> usize {
    unsafe { rd(rd(obj as usize) as usize + slot * 4) as usize }
}

/// A function pointer from an import slot.
pub(crate) unsafe fn import(slot: usize) -> usize {
    unsafe { rd(slot) as usize }
}

/// Call a game function that takes nothing and returns nothing (or whose result is unused).
pub(crate) unsafe fn call(f: usize) {
    unsafe { call_regs(f, 0, 0, 0, 0, 0, &[]) };
}

pub(crate) unsafe fn calls(fs: &[usize]) {
    for &f in fs {
        unsafe { call(f) };
    }
}

/// Call a function with one cdecl f32 argument.
pub(crate) unsafe fn call_f(f: usize, v: f32) {
    unsafe { call_regs(f, 0, 0, 0, 0, 0, &[v.to_bits()]) };
}

/// Call a function that returns on the x87 stack, rounded to f32 as the callers store it.
pub(crate) unsafe fn call_f32(f: usize) -> f32 {
    let mut out = 0f32;
    unsafe {
        std::arch::asm!(
            "push edi",
            "call {f}",
            "pop edi",
            "fstp dword ptr [edi]",
            f = in(reg) f,
            in("edi") &mut out as *mut f32,
            clobber_abi("C"),
        );
    }
    out
}

/// Call a function that returns on the x87 stack, read back as f64 (no rounding).
pub(crate) unsafe fn call_f64(f: usize) -> f64 {
    let mut out = 0f64;
    unsafe {
        std::arch::asm!(
            "push edi",
            "call {f}",
            "pop edi",
            "fstp qword ptr [edi]",
            f = in(reg) f,
            in("edi") &mut out as *mut f64,
            clobber_abi("C"),
        );
    }
    out
}

/// Shape A over a circular list: `next = link != head ? link : 0`, read before the body, so the
/// body may free the node.
pub(crate) unsafe fn walk_a(head: usize, link: usize, mut body: impl FnMut(u32)) {
    unsafe {
        let mut n = rd(head);
        while n != 0 {
            let l = rd(n as usize + link);
            let next = if l != rd(head) { l } else { 0 };
            body(n);
            n = next;
        }
    }
}

/// Shape B: next read after the body, stopping at the live head. The body must not free the node.
pub(crate) unsafe fn walk_b(head: usize, link: usize, mut body: impl FnMut(u32)) {
    unsafe {
        let mut n = rd(head);
        while n != 0 {
            body(n);
            n = rd(n as usize + link);
            if n == rd(head) {
                break;
            }
        }
    }
}

/// Shape B, but stopping at the head as it was on entry (not the live head).
pub(crate) unsafe fn walk_b_fixed(head: usize, link: usize, mut body: impl FnMut(u32)) {
    unsafe {
        let h = rd(head);
        let mut n = h;
        while n != 0 {
            body(n);
            n = rd(n as usize + link);
            if n == h {
                break;
            }
        }
    }
}

/// The game-time "due" test used all over the frame loop: timestamps are ms wrapping at
/// 1,800,000,000, and a gap of 900,000,000 or more counts as wrapped. True when `t` (>= 0) has
/// been reached by `now`.
pub(crate) fn time_due(t: i32, now: i32) -> bool {
    t >= 0 && if t > now { t.wrapping_sub(now) >= 0x35A4_E900 } else { now.wrapping_sub(t) < 0x35A4_E900 }
}

/// `now + ms`, wrapped the way the game stores timestamps.
pub(crate) fn time_add(now: i32, ms: i32) -> i32 {
    let v = now.wrapping_add(ms);
    if v > 0x6B49_D200 { v - 0x6B49_D200 } else { v }
}

/// A thunk that keeps ECX / EDX around a Rust `extern "C" fn()` with no arguments. EAX carries
/// the Rust function's return value out (for `-> u32` bodies).
macro_rules! keep_ecx_edx {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!("push ecx", "push edx", "call {f}", "pop edx", "pop ecx", "ret", f = sym $f)
        }
    };
}
pub(crate) use keep_ecx_edx;

/// A thunk that keeps ECX / EDX around a Rust `extern "C" fn(u32)` taking one cdecl argument.
macro_rules! keep_ecx_edx_arg1 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "push dword ptr [esp + 12]",
                "call {f}",
                "add esp, 4",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}
pub(crate) use keep_ecx_edx_arg1;

/// A thunk for a Rust `extern "C" fn(u32, u32)` taking two cdecl arguments, keeping ECX / EDX.
macro_rules! keep_ecx_edx_arg2 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "push edx",
                "push dword ptr [esp + 16]",
                "push dword ptr [esp + 16]",
                "call {f}",
                "add esp, 8",
                "pop edx",
                "pop ecx",
                "ret",
                f = sym $f,
            )
        }
    };
}
pub(crate) use keep_ecx_edx_arg2;

/// Install a port pinned to its whole original body, logging the outcome.
pub(crate) fn replace_pinned(name: &str, addr: usize, body: &[u8], thunk: usize) -> bool {
    match unsafe { crate::detour::replace(addr, body, thunk) } {
        Ok(()) => true,
        Err(e) => {
            crate::log(&format!("{name}: replacing @ {addr:#x} failed: {e:?}"));
            false
        }
    }
}
