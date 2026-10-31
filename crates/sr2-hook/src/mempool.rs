//! Memory pools (`mempool_base` family), docs/analysis/04_mempool.md.
//!
//! Methods are replaced by swapping vtable slots, so no code bytes change and a
//! method can be ported for every class that shares it at once.
//!
//! `static_mempool_base` layout (the part the bump allocator uses):
//! +0x00 vtable, +0x04 u8 "disabled" (1 = refuse allocations), +0x0C char name[],
//! +0x2C flags (bit 0: single allocation only), +0x34 last allocation,
//! +0x38 base, +0x3C size, +0x40 default alignment, +0x44 used (cursor),
//! +0x4C end (`mempool`; `virtual_mempool` keeps a CRITICAL_SECTION there).

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

use crate::log;

/// `static_mempool_base::alloc(size, align)` (thiscall, slot 7).
const MEMPOOL_ALLOC: usize = 0x00C0_0DB0;
const SLOT_IS_VALID: usize = 1;
const SLOT_BYTES_FREE: usize = 3;
const SLOT_ALLOC: usize = 7;

/// Pool vtables (docs/rtti_classes.csv); slot 7 is patched where it is 0xC00DB0.
const POOL_VTABLES: [(&str, usize); 11] = [
    ("mempool", 0x00E5_8CD4),
    ("virtual_mempool", 0x00E5_93A4),
    ("page_mempool", 0x00E5_AC64),
    ("split_page_mempool", 0x00E5_A6C4),
    ("async_mempool", 0x00DE_1B94),
    ("async_virtual_mempool", 0x00E5_A744),
    ("compacting_mempool", 0x00DE_18D4),
    ("tracked_mempool", 0x00E5_94DC),
    ("string_pool", 0x00DD_A834),
    ("string_hash_table", 0x00E5_959C),
    ("split_mempool_template<mempool>", 0x00DE_186C),
];

const CRT_PRINTF: usize = 0x00C9_9400;
const ALLOC_FAILED_FMT: usize = 0x00E5_9420; // "Failed aligned alloc of\n%d bytes in %s pool\n..."

/// Run the original on the same state first and compare (see [`alloc_hook`]).
const ALLOC_SHADOW: bool = true;
/// Bytes of the pool object compared and restored by the shadow check.
const HEADER: usize = 0x50;

static ALLOC_CALLS: AtomicU32 = AtomicU32::new(0);
static ALLOC_FAILS: AtomicU32 = AtomicU32::new(0);
static ALLOC_MISMATCHES: AtomicU32 = AtomicU32::new(0);
static ALLOC_PATCHED: AtomicUsize = AtomicUsize::new(0);

unsafe fn vcall<F: Copy>(this: *mut u8, slot: usize) -> F {
    unsafe {
        let vt = *(this as *const *const usize);
        std::mem::transmute_copy(&*vt.add(slot))
    }
}

/// Rust port of `static_mempool_base::alloc` (0xC00DB0): a bump allocator.
unsafe extern "thiscall" fn alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        let f = |o: usize| this.add(o) as *mut u32;
        if *this.add(4) == 1 {
            return std::ptr::null_mut();
        }
        let is_valid: extern "thiscall" fn(*mut u8) -> u8 = vcall(this, SLOT_IS_VALID);
        if is_valid(this) == 0 {
            return std::ptr::null_mut();
        }
        if *f(0x44) != 0 && *f(0x2C) & 1 != 0 {
            return std::ptr::null_mut();
        }
        let a = if align != 0 { align } else { *f(0x40) };
        let rem = (*f(0x44)).wrapping_add(*f(0x38)) % a;
        let pad = if rem == 0 { 0 } else { a - rem };
        let bytes_free: extern "thiscall" fn(*mut u8) -> u32 = vcall(this, SLOT_BYTES_FREE);
        let free = bytes_free(this);
        if pad.wrapping_add(size) > free {
            let size_field = *f(0x3C);
            let free = bytes_free(this);
            let printf: unsafe extern "C" fn(*const u8, ...) -> i32 = std::mem::transmute(CRT_PRINTF);
            printf(
                ALLOC_FAILED_FMT as *const u8,
                size,
                this.add(0x0C),
                size_field as f64 * (1.0 / 1_048_576.0),
                size_field,
                free,
            );
            ALLOC_FAILS.fetch_add(1, Ordering::Relaxed);
            return std::ptr::null_mut();
        }
        *f(0x34) = (*f(0x44)).wrapping_add(*f(0x38)).wrapping_add(pad);
        *f(0x44) = (*f(0x44)).wrapping_add(pad).wrapping_add(size);
        *f(0x34) as *mut u8
    }
}

/// Patched into the vtables. With [`ALLOC_SHADOW`], the original runs first on
/// the real object; its result and header are kept, the header is rewound, the
/// port runs, and the two outcomes are compared. On a mismatch the original's
/// outcome is restored.
unsafe extern "thiscall" fn alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    if !ALLOC_SHADOW {
        return unsafe { alloc(this, size, align) };
    }
    unsafe {
        let original: extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 = std::mem::transmute(MEMPOOL_ALLOC);
        let mut before = [0u8; HEADER];
        std::ptr::copy_nonoverlapping(this, before.as_mut_ptr(), HEADER);
        let ro = original(this, size, align);
        let mut after_o = [0u8; HEADER];
        std::ptr::copy_nonoverlapping(this, after_o.as_mut_ptr(), HEADER);
        std::ptr::copy_nonoverlapping(before.as_ptr(), this, HEADER);
        let rr = alloc(this, size, align);
        let after_r = std::slice::from_raw_parts(this, HEADER);
        if rr != ro || after_r != after_o {
            if ALLOC_MISMATCHES.fetch_add(1, Ordering::Relaxed) < 10 {
                log(&format!(
                    "mempool alloc mismatch in {:?}: size {size:#x} align {align}: original {ro:p}, rust {rr:p}",
                    String::from_utf8_lossy(crate::hooks::cstr(this.add(0x0C)))
                ));
            }
            std::ptr::copy_nonoverlapping(after_o.as_ptr(), this, HEADER);
            return ro;
        }
        rr
    }
}

pub fn install() {
    let hook = alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize;
    let mut patched = Vec::new();
    for &(name, vt) in &POOL_VTABLES {
        let slot = (vt + 4 * SLOT_ALLOC) as *mut usize;
        if unsafe { *slot } != MEMPOOL_ALLOC {
            continue;
        }
        let mut old = 0u32;
        unsafe {
            if VirtualProtect(slot as *const c_void, 4, PAGE_READWRITE, &mut old) == 0 {
                log(&format!("mempool: VirtualProtect failed for {name}"));
                continue;
            }
            *slot = hook;
            VirtualProtect(slot as *const c_void, 4, old, &mut old);
        }
        patched.push(name);
    }
    ALLOC_PATCHED.store(patched.len(), Ordering::Relaxed);
    log(&format!(
        "replaced static_mempool_base::alloc @ 0xc00db0 with Rust in {} vtables ({}){}",
        patched.len(),
        patched.join(", "),
        if ALLOC_SHADOW { ", shadow-checked" } else { "" }
    ));
}

pub fn summary() -> String {
    format!(
        "mempool alloc live: {} calls, {} failed, {} mismatches",
        ALLOC_CALLS.load(Ordering::Relaxed),
        ALLOC_FAILS.load(Ordering::Relaxed),
        ALLOC_MISMATCHES.load(Ordering::Relaxed)
    )
}
