//! Minimal x86 inline detours.
//!
//! The caller supplies the exact prologue bytes being replaced. Their length must
//! cover whole instructions (>= 5 bytes) and contain no relative branches. The bytes
//! are verified before patching, so a mismatch (still encrypted, wrong exe version)
//! fails safely and the game is left untouched.

use std::ffi::c_void;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualProtect,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[derive(Debug)]
#[allow(dead_code)] // fields are read via Debug in logs
pub enum DetourError {
    PrologueTooShort,
    PrologueMismatch { found: Vec<u8> },
    Alloc,
    Protect,
}

fn jmp_rel32(from: usize, to: usize) -> [u8; 5] {
    let rel = (to as i32).wrapping_sub(from as i32 + 5);
    let r = rel.to_le_bytes();
    [0xE9, r[0], r[1], r[2], r[3]]
}

/// Redirect `target` to `replacement`. Returns a trampoline that runs the
/// original function (stolen prologue + jump back).
///
/// # Safety
/// `target` must be the start of a function whose first `prologue.len()` bytes are
/// whole, position-independent instructions, and no thread may be executing them.
pub unsafe fn install(target: usize, prologue: &[u8], replacement: usize) -> Result<usize, DetourError> {
    let n = prologue.len();
    if n < 5 {
        return Err(DetourError::PrologueTooShort);
    }
    let current = unsafe { std::slice::from_raw_parts(target as *const u8, n) };
    if current != prologue {
        return Err(DetourError::PrologueMismatch { found: current.to_vec() });
    }

    // Trampoline: stolen bytes, then jmp back to target + n.
    let tramp = unsafe {
        VirtualAlloc(std::ptr::null(), n + 5, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
    } as *mut u8;
    if tramp.is_null() {
        return Err(DetourError::Alloc);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(prologue.as_ptr(), tramp, n);
        let back = jmp_rel32(tramp as usize + n, target + n);
        std::ptr::copy_nonoverlapping(back.as_ptr(), tramp.add(n), 5);
    }

    // Patch target: jmp replacement, pad the rest with int3.
    let mut old = 0u32;
    if unsafe { VirtualProtect(target as *const c_void, n, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err(DetourError::Protect);
    }
    let jmp = jmp_rel32(target, replacement);
    unsafe {
        let p = target as *mut u8;
        std::ptr::copy_nonoverlapping(jmp.as_ptr(), p, 5);
        for i in 5..n {
            *p.add(i) = 0xCC;
        }
        VirtualProtect(target as *const c_void, n, old, &mut old);
        FlushInstructionCache(GetCurrentProcess(), target as *const c_void, n);
    }
    Ok(tramp as usize)
}
