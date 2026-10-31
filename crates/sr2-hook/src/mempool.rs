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

// --- virtual_mempool::alloc (0xBFCA60) --------------------------------------

/// `virtual_mempool::alloc(size, align)` (thiscall, slot 7). Reserves address
/// space up front (+0x38 base, +0x3C size) and commits pages as `used` grows.
const VIRTUAL_MEMPOOL_ALLOC: usize = 0x00BF_CA60;
/// Extra `virtual_mempool` fields: +0x4C CRITICAL_SECTION (24 bytes), +0x68
/// committed bytes, +0x6C flags (bit 0: zero new pages), +0x70 page tracker.
const VM_LOCK: usize = 0x4C;
const VM_LOCK_END: usize = 0x64;
const VM_HEADER: usize = 0x74;

const PAGE_TRACKER_RESERVE: usize = 0x00C1_52C0; // ECX = pages, EDX = tracker -> AL
const PAGE_TRACKER_RELEASE: usize = 0x00C1_53A0; // ECX = pages, ESI = tracker
const IAT_ENTER_CS: usize = 0x00DB_80CC;
const IAT_LEAVE_CS: usize = 0x00DB_80D0;
const IAT_VIRTUAL_ALLOC: usize = 0x00DB_80A0;
const CRT_MEMSET: usize = 0x00CA_0BC0;
const MSG_ASSERT_PERMISSION: usize = 0x00E5_92D8; // "ERROR: assert_permission"
const MSG_NO_PAGES: usize = 0x00E5_92F4; // "No page pool pages left for virtual_mempool allocation.\n"
const MSG_OUT_OF_MEMORY: usize = 0x00E5_9330; // "Mempool %s is out of memory allocating %d bytes, ..."

const VALLOC_SHADOW: bool = true;
static VALLOC_CALLS: AtomicU32 = AtomicU32::new(0);
static VALLOC_COMMITS: AtomicU32 = AtomicU32::new(0);
static VALLOC_TRACKER_SKIPPED: AtomicU32 = AtomicU32::new(0);
static VALLOC_MISMATCHES: AtomicU32 = AtomicU32::new(0);

unsafe fn lock(cs: *mut u8, enter: bool) {
    unsafe {
        let f: extern "system" fn(*mut u8) =
            std::mem::transmute(*((if enter { IAT_ENTER_CS } else { IAT_LEAVE_CS }) as *const usize));
        f(cs);
    }
}

unsafe fn page_tracker_reserve(tracker: *mut u8, pages: u32) -> bool {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) PAGE_TRACKER_RESERVE,
            in("ecx") pages,
            in("edx") tracker,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

unsafe fn page_tracker_release(tracker: *mut u8, pages: u32) {
    unsafe {
        std::arch::asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {t}",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) PAGE_TRACKER_RELEASE,
            t = in(reg) tracker,
            in("ecx") pages,
            clobber_abi("C"),
        );
    }
}

/// Rust port of `virtual_mempool::alloc` (0xBFCA60), docs/analysis/04_mempool.md.
unsafe extern "thiscall" fn virtual_alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        let f = |o: usize| this.add(o) as *mut u32;
        let printf: unsafe extern "C" fn(*const u8, ...) -> i32 = std::mem::transmute(CRT_PRINTF);
        let is_valid: extern "thiscall" fn(*mut u8) -> u8 = vcall(this, SLOT_IS_VALID);
        if *this.add(4) == 1 || is_valid(this) == 0 {
            printf(MSG_ASSERT_PERMISSION as *const u8);
            return std::ptr::null_mut();
        }
        let cs = this.add(VM_LOCK);
        lock(cs, true);
        if *f(0x2C) & 1 != 0 && *f(0x44) != 0 {
            lock(cs, false);
            return std::ptr::null_mut();
        }
        let a = if align != 0 { align } else { *f(0x40) };
        let rem = (*f(0x44)).wrapping_add(*f(0x38)) % a;
        let pad = if rem == 0 { 0 } else { a - rem };
        let new_used = (*f(0x44)).wrapping_add(pad).wrapping_add(size);
        if new_used > *f(0x3C) {
            lock(cs, false);
            return std::ptr::null_mut();
        }
        let committed = *f(0x68);
        if new_used > committed {
            let r = new_used & 0xFFF;
            let end = new_used.wrapping_add(if r == 0 { 0 } else { 0x1000 - r });
            let tracker = *f(0x70) as *mut u8;
            let pages = end.wrapping_sub(committed) >> 12;
            if !tracker.is_null() && !page_tracker_reserve(tracker, pages) {
                printf(MSG_NO_PAGES as *const u8);
                lock(cs, false);
                return std::ptr::null_mut();
            }
            let virtual_alloc: extern "system" fn(usize, usize, u32, u32) -> usize =
                std::mem::transmute(*(IAT_VIRTUAL_ALLOC as *const usize));
            let at = (*f(0x38)).wrapping_add(committed) as usize;
            if virtual_alloc(at, end.wrapping_sub(committed) as usize, 0x1000, 4) == 0 {
                // The original reads tracker+0x24 even when there is no tracker
                // (and faults); here a missing tracker reads as 0 pages.
                let free_pages = if tracker.is_null() { 0 } else { *(tracker.add(0x24) as *const u32) };
                printf(
                    MSG_OUT_OF_MEMORY as *const u8,
                    this.add(0x0C),
                    size,
                    (free_pages << 12).wrapping_sub(committed),
                    *f(0x3C),
                );
                if !tracker.is_null() {
                    page_tracker_release(tracker, pages);
                }
                lock(cs, false);
                return std::ptr::null_mut();
            }
            if *f(0x6C) & 1 != 0 {
                let memset: extern "C" fn(usize, i32, u32) -> usize = std::mem::transmute(CRT_MEMSET);
                memset(at, 0, end.wrapping_sub(committed));
            }
            *f(0x68) = end;
            VALLOC_COMMITS.fetch_add(1, Ordering::Relaxed);
        }
        *f(0x34) = (*f(0x44)).wrapping_add(*f(0x38)).wrapping_add(pad);
        *f(0x44) = (*f(0x44)).wrapping_add(pad).wrapping_add(size);
        let r = *f(0x34) as *mut u8;
        lock(cs, false);
        r
    }
}

/// Page tracker (the "page pool" behind virtual pools): CRITICAL_SECTION at
/// +0x00, then +0x1C total bytes, +0x24 free pages, +0x28 high-water mark.
/// Reserving pages lowers +0x24, raises +0x28 and decommits that many pages of
/// the tracker's own reserve (repeatable).
const TRACKER_COUNTERS: std::ops::Range<usize> = 0x1C..0x2C;

/// Shadow check as for [`alloc_hook`], holding the pool's (recursive) lock,
/// and the page tracker's when there is one, across both runs so no other
/// thread sees rewound state. The lock bytes are never compared or restored.
/// A commit through the tracker is rewound with the tracker's counters; only
/// a reservation larger than the tracker's free pages (which runs its
/// compaction, 0xC15810) is left to the original. A plain commit is safe to
/// repeat (`MEM_COMMIT` and the zero-fill are idempotent at that point).
unsafe extern "thiscall" fn virtual_alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    VALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    if !VALLOC_SHADOW {
        return unsafe { virtual_alloc(this, size, align) };
    }
    unsafe {
        let original: extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 = std::mem::transmute(VIRTUAL_MEMPOOL_ALLOC);
        let cs = this.add(VM_LOCK);
        lock(cs, true);
        let tracker = *(this.add(0x70) as *const *mut u8);
        if !tracker.is_null() {
            lock(tracker, true);
        }
        let r = virtual_alloc_shadow(this, size, align, original, tracker);
        if !tracker.is_null() {
            lock(tracker, false);
        }
        lock(cs, false);
        r
    }
}

unsafe fn virtual_alloc_shadow(
    this: *mut u8,
    size: u32,
    align: u32,
    original: extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8,
    tracker: *mut u8,
) -> *mut u8 {
    unsafe {
        let counters = |t: *mut u8| -> [u8; 16] {
            let mut c = [0u8; 16];
            if !t.is_null() {
                std::ptr::copy_nonoverlapping(t.add(TRACKER_COUNTERS.start), c.as_mut_ptr(), 16);
            }
            c
        };
        let tracker_before = counters(tracker);
        let free_pages = |c: &[u8; 16]| u32::from_le_bytes(c[8..12].try_into().unwrap());
        let save = |buf: &mut [u8; VM_HEADER]| std::ptr::copy_nonoverlapping(this, buf.as_mut_ptr(), VM_HEADER);
        let restore = |buf: &[u8; VM_HEADER]| {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), this, VM_LOCK);
            std::ptr::copy_nonoverlapping(buf.as_ptr().add(VM_LOCK_END), this.add(VM_LOCK_END), VM_HEADER - VM_LOCK_END);
        };
        let same = |a: &[u8; VM_HEADER], b: &[u8; VM_HEADER]| a[..VM_LOCK] == b[..VM_LOCK] && a[VM_LOCK_END..] == b[VM_LOCK_END..];
        let mut before = [0u8; VM_HEADER];
        save(&mut before);
        let ro = original(this, size, align);
        let mut after_o = [0u8; VM_HEADER];
        save(&mut after_o);
        let tracker_after_o = counters(tracker);
        let committed = |b: &[u8; VM_HEADER]| u32::from_le_bytes(b[0x68..0x6C].try_into().unwrap());
        if !tracker.is_null() && committed(&after_o) != committed(&before) {
            // Pages the original reserved; more than were free means it compacted.
            let used = free_pages(&tracker_before).wrapping_sub(free_pages(&tracker_after_o));
            let needed = committed(&after_o).wrapping_sub(committed(&before)) >> 12;
            if needed > free_pages(&tracker_before) || used != needed {
                VALLOC_TRACKER_SKIPPED.fetch_add(1, Ordering::Relaxed);
                return ro;
            }
            std::ptr::copy_nonoverlapping(tracker_before.as_ptr(), tracker.add(TRACKER_COUNTERS.start), 16);
        }
        restore(&before);
        let rr = virtual_alloc(this, size, align);
        let mut after_r = [0u8; VM_HEADER];
        save(&mut after_r);
        let tracker_after_r = counters(tracker);
        if rr != ro || !same(&after_r, &after_o) || tracker_after_r != tracker_after_o {
            if VALLOC_MISMATCHES.fetch_add(1, Ordering::Relaxed) < 10 {
                log(&format!(
                    "virtual_mempool alloc mismatch in {:?}: size {size:#x} align {align}: original {ro:p}, rust {rr:p}, tracker {}",
                    String::from_utf8_lossy(crate::hooks::cstr(this.add(0x0C))),
                    if tracker_after_r == tracker_after_o { "same" } else { "DIFFERS" }
                ));
            }
            restore(&after_o);
            if !tracker.is_null() {
                std::ptr::copy_nonoverlapping(tracker_after_o.as_ptr(), tracker.add(TRACKER_COUNTERS.start), 16);
            }
            return ro;
        }
        rr
    }
}

/// Swap `slot` in every pool vtable that currently holds `original`.
fn swap_slot(slot: usize, original: usize, hook: usize) -> Vec<&'static str> {
    let mut patched = Vec::new();
    for &(name, vt) in &POOL_VTABLES {
        let p = (vt + 4 * slot) as *mut usize;
        if unsafe { *p } != original {
            continue;
        }
        let mut old = 0u32;
        unsafe {
            if VirtualProtect(p as *const c_void, 4, PAGE_READWRITE, &mut old) == 0 {
                log(&format!("mempool: VirtualProtect failed for {name}"));
                continue;
            }
            *p = hook;
            VirtualProtect(p as *const c_void, 4, old, &mut old);
        }
        patched.push(name);
    }
    patched
}

pub fn install() {
    let patched = swap_slot(
        SLOT_ALLOC,
        MEMPOOL_ALLOC,
        alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
    );
    ALLOC_PATCHED.store(patched.len(), Ordering::Relaxed);
    log(&format!(
        "replaced static_mempool_base::alloc @ 0xc00db0 with Rust in {} vtables ({}){}",
        patched.len(),
        patched.join(", "),
        if ALLOC_SHADOW { ", shadow-checked" } else { "" }
    ));
    let patched = swap_slot(
        SLOT_ALLOC,
        VIRTUAL_MEMPOOL_ALLOC,
        virtual_alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
    );
    log(&format!(
        "replaced virtual_mempool::alloc @ 0xbfca60 with Rust in {} vtables ({}){}",
        patched.len(),
        patched.join(", "),
        if VALLOC_SHADOW { ", shadow-checked" } else { "" }
    ));
}

pub fn summary() -> String {
    format!(
        "mempool alloc live: {} calls, {} failed, {} mismatches | virtual_mempool alloc: {} calls, {} commits (rust), {} tracker commits left to the original, {} mismatches",
        ALLOC_CALLS.load(Ordering::Relaxed),
        ALLOC_FAILS.load(Ordering::Relaxed),
        ALLOC_MISMATCHES.load(Ordering::Relaxed),
        VALLOC_CALLS.load(Ordering::Relaxed),
        VALLOC_COMMITS.load(Ordering::Relaxed),
        VALLOC_TRACKER_SKIPPED.load(Ordering::Relaxed),
        VALLOC_MISMATCHES.load(Ordering::Relaxed)
    )
}
