//! Memory pools (`mempool_base` family), docs/analysis/04_mempool.md.
//!
//! Every method is replaced with an inline detour on its body, so both virtual
//! calls and the many direct (devirtualized) calls reach the port; the original
//! stays callable through its trampoline. Each hook shadow-checks the port
//! against the original on every call:
//! - pure methods: both run, results compared (re-reading the original once on
//!   a difference, since another thread may have allocated in between);
//! - methods that change the pool: snapshot, original, rewind, port, compare,
//!   keeping the original's outcome on a mismatch.
//!
//! `static_mempool_base` layout: +0x00 vtable, +0x04 u8 disabled, +0x08 u32,
//! +0x0C char name[0x20] (+0x2B forced 0), +0x2C u8 flags (bit 0: single
//! allocation), +0x30 u32, +0x34 last allocation, +0x38 base, +0x3C size,
//! +0x40 default alignment, +0x44 used, +0x48 mark, +0x4C limit (bytes from
//! base) and +0x50 saved limit for `mempool`; `virtual_mempool` keeps a
//! CRITICAL_SECTION at +0x4C..+0x64, then +0x68 committed, +0x6C flags (bit 0
//! zero pages, bit 1 never decommit), +0x70 page tracker.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

// --- Game functions and data used by the ports ------------------------------

const SLOT_IS_VALID: usize = 1;
const SLOT_BYTES_FREE: usize = 3;
const SLOT_ALLOC: usize = 7;
const SLOT_RELEASE_TO: usize = 13;
const SLOT_SET_LIMIT: usize = 22;
const SLOT_TRACKED_ALLOC_IMPL: usize = 28;

const CRT_PRINTF: usize = 0x00C9_9400;
const CRT_MEMSET: usize = 0x00CA_0BC0;
const CRT_STRNCPY: usize = 0x00C9_90F0;
const IAT_ENTER_CS: usize = 0x00DB_80CC;
const IAT_LEAVE_CS: usize = 0x00DB_80D0;
const IAT_VIRTUAL_ALLOC: usize = 0x00DB_80A0;
const IAT_VIRTUAL_FREE: usize = 0x00DB_81D4;
const PAGE_TRACKER_RESERVE: usize = 0x00C1_52C0; // ECX = pages, EDX = tracker -> AL
const PAGE_TRACKER_RELEASE: usize = 0x00C1_53A0; // ECX = pages, ESI = tracker

const MSG_ALLOC_FAILED: usize = 0x00E5_9420; // "Failed aligned alloc of\n%d bytes in %s pool\n..."
const MSG_ASSERT_PERMISSION: usize = 0x00E5_92D8; // "ERROR: assert_permission"
const MSG_NO_PAGES: usize = 0x00E5_92F4; // "No page pool pages left for virtual_mempool allocation.\n"
const MSG_OUT_OF_MEMORY: usize = 0x00E5_9330; // "Mempool %s is out of memory allocating %d bytes, ..."

/// Bytes of a static pool compared/restored by the shadow check.
const HEADER: usize = 0x54;
/// `virtual_mempool`: header size and its lock's bytes (never compared or restored).
const VM_HEADER: usize = 0x74;
const VM_LOCK: usize = 0x4C;
const VM_LOCK_END: usize = 0x64;
/// Page tracker counters: +0x1C total, +0x20 max free, +0x24 free pages, +0x28 high-water.
const TRACKER_COUNTERS: usize = 0x1C;

// --- The ported methods (index, name, address, stolen prologue) -------------

const IS_VALID: usize = 0;
const BYTES_FREE_END: usize = 1;
const BYTES_FREE_END_STR: usize = 2;
const BYTES_FREE_SIZE_V: usize = 3;
const BYTES_FREE_SIZE_A: usize = 4;
const BYTES_FREE_SIZE_C: usize = 5;
const USED_BYTES: usize = 6;
const SIZE: usize = 7;
const BASE: usize = 8;
const MARK: usize = 9;
const RELEASE_TO: usize = 10;
const CAN_ALLOC: usize = 11;
const ALLOC: usize = 12;
const RESIZE_LAST: usize = 13;
const RELEASE: usize = 14;
const ALIGN_TO: usize = 15;
const RESET_STATIC: usize = 16;
const RESET_MEMPOOL: usize = 17;
const SET_LIMIT: usize = 18;
const INIT: usize = 19;
const V_ALLOC: usize = 20;
const V_RELEASE_TO: usize = 21;
const A_ALLOC: usize = 22;
const T_ALLOC: usize = 23;
const T_ALLOC_IMPL: usize = 24;
const COUNT: usize = 25;

/// Each prologue is whole, position-independent instructions (checked against
/// the exe). The one-line getters steal their `ret` and an int3 of padding; the
/// trampoline still works because the `ret` runs before the jump back.
const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("is_valid", 0x00C0_0C50, &[0xA0, 0xF7, 0x7B, 0x52, 0x02, 0xB0, 0x01]),
    ("mempool::bytes_free", 0x004A_4AB0, &[0x8B, 0x41, 0x4C, 0x8B, 0x49, 0x44]),
    ("string_pool::bytes_free", 0x004A_4AA0, &[0x8B, 0x41, 0x4C, 0x8B, 0x49, 0x44]),
    ("virtual_mempool::bytes_free", 0x00A7_F2B0, &[0x8B, 0x51, 0x44, 0x8B, 0x41, 0x3C]),
    ("async_mempool::bytes_free", 0x0052_2320, &[0x8B, 0x51, 0x44, 0x8B, 0x41, 0x3C]),
    ("compacting_mempool::bytes_free", 0x0052_11B0, &[0x8B, 0x51, 0x44, 0x8B, 0x41, 0x3C]),
    ("used_bytes", 0x004A_4AE0, &[0x56, 0x8B, 0xF1, 0x8B, 0x06, 0x8B, 0x50, 0x0C]),
    ("size", 0x004A_4B10, &[0x8B, 0x41, 0x3C, 0xC3, 0xCC]),
    ("base", 0x004A_4B20, &[0x8B, 0x41, 0x38, 0xC3, 0xCC]),
    ("mark", 0x00C0_0CF0, &[0x8B, 0x41, 0x44, 0x89, 0x41, 0x48]),
    ("release_to", 0x00C0_0D00, &[0x56, 0x8B, 0xF1, 0x8A, 0x46, 0x04]),
    ("can_alloc", 0x00C0_0D40, &[0x56, 0x8B, 0xF1, 0x8B, 0x06, 0x8B, 0x50, 0x04]),
    ("alloc", 0x00C0_0DB0, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xC0, 0x83, 0xEC, 0x34]),
    ("resize_last", 0x00C0_0EC0, &[0x56, 0x8B, 0xF1, 0x8A, 0x46, 0x04]),
    ("release", 0x00C0_0F80, &[0x56, 0x8B, 0xF1, 0x57, 0x8B, 0x7E, 0x48]),
    ("align_to", 0x00C0_0F40, &[0x8B, 0x41, 0x44, 0x03, 0x41, 0x38]),
    ("reset", 0x00C0_0FA0, &[0x8B, 0x01, 0x8B, 0x50, 0x34, 0x6A, 0x00]),
    ("mempool::reset", 0x00BF_4AF0, &[0x56, 0x8B, 0xF1, 0x8B, 0x06, 0x8B, 0x50, 0x34]),
    ("set_limit", 0x00BF_4AB0, &[0x56, 0x8B, 0xF1, 0x8A, 0x46, 0x04]),
    ("init", 0x00BF_4A10, &[0x8B, 0x44, 0x24, 0x0C, 0x56, 0x8B, 0xF1]),
    ("virtual_mempool::alloc", 0x00BF_CA60, &[0x6A, 0xFF, 0x68, 0xD8, 0x32, 0xDA, 0x00, 0x64, 0xA1, 0, 0, 0, 0]),
    ("virtual_mempool::release_to", 0x00BF_C870, &[0x6A, 0xFF, 0x68, 0x38, 0x33, 0xDA, 0x00, 0x64, 0xA1, 0, 0, 0, 0]),
    ("async_mempool::alloc", 0x00C0_BC00, &[0x53, 0x55, 0x56, 0x8B, 0x74, 0x24, 0x14, 0x85, 0xF6]),
    ("tracked_mempool::alloc", 0x00C0_1040, &[0x8B, 0x91, 0xA0, 0x00, 0x00, 0x00]),
    ("tracked_mempool::alloc_impl", 0x00C0_1060, &[0x8B, 0x54, 0x24, 0x04, 0x56, 0x8B, 0xF1]),
];

/// Trampolines to the originals.
static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
/// virtual_mempool commits/decommits done by the port, and shadow checks skipped
/// because the page tracker compacted (that cannot be rewound).
static V_COMMITS: AtomicU32 = AtomicU32::new(0);
static V_SKIPPED: AtomicU32 = AtomicU32::new(0);
/// Serializes the lock-free async pool while it is shadow-checked. Every call
/// reaches the hook, so nothing else moves `used` meanwhile.
static ASYNC_LOCK: Mutex<()> = Mutex::new(());

fn orig<F: Copy>(i: usize) -> Option<F> {
    let p = ORIG[i].load(Ordering::Acquire);
    (p != 0).then(|| unsafe { std::mem::transmute_copy(&p) })
}

fn mismatch(i: usize, this: *mut u8, detail: String) {
    if MISMATCHES[i].fetch_add(1, Ordering::Relaxed) < 5 {
        let name = String::from_utf8_lossy(unsafe { crate::hooks::cstr(this.add(0x0C)) }).into_owned();
        log(&format!("mempool {} mismatch in {name:?}: {detail}", PORTS[i].0));
    }
}

/// Pure method: compare the port with the original (twice, in case another
/// thread changed the pool between the two reads).
fn pure<T: PartialEq + Copy + std::fmt::Debug>(i: usize, this: *mut u8, rust: T, original: impl Fn() -> T) -> T {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    let o = original();
    if o == rust || original() == rust {
        return rust;
    }
    mismatch(i, this, format!("original {o:?}, rust {rust:?}"));
    o
}

/// State-changing method on a static pool: snapshot `HEADER` bytes, run the
/// original, rewind, run the port, compare result and header.
unsafe fn stateful<T: PartialEq + Copy + std::fmt::Debug>(
    i: usize,
    this: *mut u8,
    original: impl FnOnce() -> T,
    rust: impl FnOnce() -> T,
) -> T {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let mut before = [0u8; HEADER];
        std::ptr::copy_nonoverlapping(this, before.as_mut_ptr(), HEADER);
        let ro = original();
        let mut after_o = [0u8; HEADER];
        std::ptr::copy_nonoverlapping(this, after_o.as_mut_ptr(), HEADER);
        std::ptr::copy_nonoverlapping(before.as_ptr(), this, HEADER);
        let rr = rust();
        if rr != ro || std::slice::from_raw_parts(this, HEADER) != after_o {
            let at = (0..HEADER).find(|&k| *this.add(k) != after_o[k]);
            mismatch(i, this, format!("original {ro:?}, rust {rr:?}, first differing byte {at:x?}"));
            std::ptr::copy_nonoverlapping(after_o.as_ptr(), this, HEADER);
            return ro;
        }
        rr
    }
}

unsafe fn vcall<F: Copy>(this: *mut u8, slot: usize) -> F {
    unsafe {
        let vt = *(this as *const *const usize);
        std::mem::transmute_copy(&*vt.add(slot))
    }
}

unsafe fn u32_at(this: *mut u8, o: usize) -> *mut u32 {
    unsafe { this.add(o) as *mut u32 }
}

unsafe fn disabled_or_invalid(this: *mut u8) -> bool {
    unsafe {
        let is_valid: extern "thiscall" fn(*mut u8) -> u8 = vcall(this, SLOT_IS_VALID);
        *this.add(4) == 1 || is_valid(this) == 0
    }
}

unsafe fn printf() -> unsafe extern "C" fn(*const u8, ...) -> i32 {
    unsafe { std::mem::transmute(CRT_PRINTF) }
}

// --- Getters ----------------------------------------------------------------

unsafe extern "thiscall" fn is_valid_hook(this: *mut u8) -> u8 {
    // The original reads a debug byte (0x2527BF7) and ignores it.
    pure(IS_VALID, this, 1u8, || orig::<extern "thiscall" fn(*mut u8) -> u8>(IS_VALID).map_or(1, |f| f(this)))
}

unsafe extern "thiscall" fn bytes_free_end_hook<const I: usize>(this: *mut u8) -> u32 {
    let rust = unsafe { (*u32_at(this, 0x4C)).wrapping_sub(*u32_at(this, 0x44)) };
    pure(I, this, rust, || orig::<extern "thiscall" fn(*mut u8) -> u32>(I).map_or(rust, |f| f(this)))
}

unsafe extern "thiscall" fn bytes_free_size_hook<const I: usize>(this: *mut u8) -> u32 {
    let rust = unsafe { (*u32_at(this, 0x3C)).wrapping_sub(*u32_at(this, 0x44)) };
    pure(I, this, rust, || orig::<extern "thiscall" fn(*mut u8) -> u32>(I).map_or(rust, |f| f(this)))
}

unsafe extern "thiscall" fn used_bytes_hook(this: *mut u8) -> u32 {
    let rust = unsafe {
        let bytes_free: extern "thiscall" fn(*mut u8) -> u32 = vcall(this, SLOT_BYTES_FREE);
        (*u32_at(this, 0x3C)).wrapping_sub(bytes_free(this))
    };
    pure(USED_BYTES, this, rust, || orig::<extern "thiscall" fn(*mut u8) -> u32>(USED_BYTES).map_or(rust, |f| f(this)))
}

unsafe extern "thiscall" fn size_hook(this: *mut u8) -> u32 {
    let rust = unsafe { *u32_at(this, 0x3C) };
    pure(SIZE, this, rust, || orig::<extern "thiscall" fn(*mut u8) -> u32>(SIZE).map_or(rust, |f| f(this)))
}

unsafe extern "thiscall" fn base_hook(this: *mut u8) -> u32 {
    let rust = unsafe { *u32_at(this, 0x38) };
    pure(BASE, this, rust, || orig::<extern "thiscall" fn(*mut u8) -> u32>(BASE).map_or(rust, |f| f(this)))
}

// --- static_mempool_base / mempool --------------------------------------------

/// `mark()`: remember `used` so `release_to(-1)` can return to it.
unsafe fn mark(this: *mut u8) {
    unsafe { *u32_at(this, 0x48) = *u32_at(this, 0x44) }
}

/// `release_to(pos)`: rewind `used` (and the mark) to `pos`, or to the mark for -1.
unsafe fn release_to(this: *mut u8, pos: u32) -> u8 {
    unsafe {
        if disabled_or_invalid(this) {
            return 0;
        }
        let p = if pos == u32::MAX { *u32_at(this, 0x48) } else { pos };
        *u32_at(this, 0x44) = p;
        *u32_at(this, 0x48) = p;
        1
    }
}

/// Padding to align `base + used` to `align` (or the pool default).
unsafe fn pad_for(this: *mut u8, align: u32) -> u32 {
    unsafe {
        let a = if align != 0 { align } else { *u32_at(this, 0x40) };
        let rem = (*u32_at(this, 0x44)).wrapping_add(*u32_at(this, 0x38)) % a;
        if rem == 0 { 0 } else { a - rem }
    }
}

/// `can_alloc(size, align)`: alloc's checks without changing anything. Unlike
/// alloc it asks `is_valid()` before looking at the disabled byte.
unsafe fn can_alloc(this: *mut u8, size: u32, align: u32) -> u8 {
    unsafe {
        let is_valid: extern "thiscall" fn(*mut u8) -> u8 = vcall(this, SLOT_IS_VALID);
        if is_valid(this) == 0 || *this.add(4) == 1 {
            return 0;
        }
        if *u32_at(this, 0x44) != 0 && *this.add(0x2C) & 1 != 0 {
            return 0;
        }
        let pad = pad_for(this, align);
        let bytes_free: extern "thiscall" fn(*mut u8) -> u32 = vcall(this, SLOT_BYTES_FREE);
        (pad.wrapping_add(size) <= bytes_free(this)) as u8
    }
}

/// `alloc(size, align)` (0xC00DB0): the bump allocator.
unsafe fn alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        if *this.add(4) == 1 {
            return std::ptr::null_mut();
        }
        let is_valid: extern "thiscall" fn(*mut u8) -> u8 = vcall(this, SLOT_IS_VALID);
        if is_valid(this) == 0 {
            return std::ptr::null_mut();
        }
        if *u32_at(this, 0x44) != 0 && *this.add(0x2C) & 1 != 0 {
            return std::ptr::null_mut();
        }
        let pad = pad_for(this, align);
        let bytes_free: extern "thiscall" fn(*mut u8) -> u32 = vcall(this, SLOT_BYTES_FREE);
        if pad.wrapping_add(size) > bytes_free(this) {
            let size_field = *u32_at(this, 0x3C);
            let free = bytes_free(this);
            printf()(
                MSG_ALLOC_FAILED as *const u8,
                size,
                this.add(0x0C),
                size_field as f64 * (1.0 / 1_048_576.0),
                size_field,
                free,
            );
            return std::ptr::null_mut();
        }
        let used = u32_at(this, 0x44);
        *u32_at(this, 0x34) = (*used).wrapping_add(*u32_at(this, 0x38)).wrapping_add(pad);
        *used = (*used).wrapping_add(pad).wrapping_add(size);
        *u32_at(this, 0x34) as *mut u8
    }
}

/// `resize_last(ptr, size)`: grow or shrink the most recent allocation in place.
unsafe fn resize_last(this: *mut u8, ptr: *mut u8, size: u32) -> *mut u8 {
    unsafe {
        if disabled_or_invalid(this) || ptr as u32 != *u32_at(this, 0x34) {
            return std::ptr::null_mut();
        }
        let new_used = (*u32_at(this, 0x34)).wrapping_sub(*u32_at(this, 0x38)).wrapping_add(size);
        let bytes_free: extern "thiscall" fn(*mut u8) -> u32 = vcall(this, SLOT_BYTES_FREE);
        if new_used.wrapping_sub(*u32_at(this, 0x44)) > bytes_free(this) {
            return std::ptr::null_mut();
        }
        *u32_at(this, 0x44) = new_used;
        *u32_at(this, 0x34) as *mut u8
    }
}

/// `release(n)`: give back the last `n` bytes, keeping the mark.
unsafe fn release(this: *mut u8, n: u32) -> u8 {
    unsafe {
        let saved_mark = *u32_at(this, 0x48);
        let release_to: extern "thiscall" fn(*mut u8, u32) -> u8 = vcall(this, SLOT_RELEASE_TO);
        let r = release_to(this, (*u32_at(this, 0x44)).wrapping_sub(n));
        *u32_at(this, 0x48) = saved_mark;
        r
    }
}

/// `align_to(a)`: pad `used` so the next allocation starts aligned.
unsafe fn align_to(this: *mut u8, a: u32) -> u8 {
    unsafe {
        let rem = (*u32_at(this, 0x44)).wrapping_add(*u32_at(this, 0x38)) % a;
        if rem == 0 || a - rem == 0 {
            return 1;
        }
        let alloc: extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 = vcall(this, SLOT_ALLOC);
        (!alloc(this, a - rem, 1).is_null()) as u8
    }
}

/// `reset()` (static pools): `release_to(0)`.
unsafe fn reset_static(this: *mut u8) -> u8 {
    unsafe {
        let release_to: extern "thiscall" fn(*mut u8, u32) -> u8 = vcall(this, SLOT_RELEASE_TO);
        release_to(this, 0)
    }
}

/// `mempool::reset()`: `release_to(0)`, then restore the full limit.
unsafe fn reset_mempool(this: *mut u8) -> u32 {
    unsafe {
        let release_to: extern "thiscall" fn(*mut u8, u32) -> u8 = vcall(this, SLOT_RELEASE_TO);
        if release_to(this, 0) == 0 {
            return 0;
        }
        let set_limit: extern "thiscall" fn(*mut u8, u32) -> u8 = vcall(this, SLOT_SET_LIMIT);
        (set_limit(this, *u32_at(this, 0x3C)) != 0) as u32
    }
}

/// `set_limit(x)`: how far (from base) the pool may grow; -1 restores the saved limit.
unsafe fn set_limit(this: *mut u8, x: u32) -> u8 {
    unsafe {
        if disabled_or_invalid(this) {
            return 0;
        }
        let v = if x == u32::MAX { *u32_at(this, 0x50) } else { x };
        *u32_at(this, 0x4C) = v;
        *u32_at(this, 0x50) = v;
        1
    }
}

/// `init(base, size, name, align, flags)` (`ret 0x14`).
unsafe fn init(this: *mut u8, base: u32, size: u32, name: *const u8, align: u32, flags: u32) {
    unsafe {
        let strncpy: extern "C" fn(*mut u8, *const u8, u32) -> *mut u8 = std::mem::transmute(CRT_STRNCPY);
        strncpy(this.add(0x0C), name, 0x20);
        *this.add(0x2B) = 0;
        *this.add(0x2C) = flags as u8;
        *u32_at(this, 0x30) = 0;
        *u32_at(this, 0x38) = base;
        *u32_at(this, 0x08) = 0;
        *this.add(4) = 0;
        *u32_at(this, 0x3C) = size;
        *u32_at(this, 0x40) = align;
        *u32_at(this, 0x44) = 0;
        *u32_at(this, 0x48) = 0;
        *u32_at(this, 0x4C) = size;
        *u32_at(this, 0x50) = size;
    }
}

type Thiscall0<R> = extern "thiscall" fn(*mut u8) -> R;
type Thiscall1<R> = extern "thiscall" fn(*mut u8, u32) -> R;
type Thiscall2<R> = extern "thiscall" fn(*mut u8, u32, u32) -> R;

unsafe extern "thiscall" fn mark_hook(this: *mut u8) {
    match orig::<Thiscall0<()>>(MARK) {
        Some(f) => unsafe { stateful(MARK, this, || f(this), || mark(this)) },
        None => unsafe { mark(this) },
    }
}

unsafe extern "thiscall" fn release_to_hook(this: *mut u8, pos: u32) -> u8 {
    match orig::<Thiscall1<u8>>(RELEASE_TO) {
        Some(f) => unsafe { stateful(RELEASE_TO, this, || f(this, pos), || release_to(this, pos)) },
        None => unsafe { release_to(this, pos) },
    }
}

unsafe extern "thiscall" fn can_alloc_hook(this: *mut u8, size: u32, align: u32) -> u8 {
    let rust = unsafe { can_alloc(this, size, align) };
    pure(CAN_ALLOC, this, rust, || orig::<Thiscall2<u8>>(CAN_ALLOC).map_or(rust, |f| f(this, size, align)))
}

unsafe extern "thiscall" fn alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    match orig::<Thiscall2<*mut u8>>(ALLOC) {
        Some(f) => unsafe { stateful(ALLOC, this, || f(this, size, align), || alloc(this, size, align)) },
        None => unsafe { alloc(this, size, align) },
    }
}

unsafe extern "thiscall" fn resize_last_hook(this: *mut u8, ptr: *mut u8, size: u32) -> *mut u8 {
    type F = extern "thiscall" fn(*mut u8, *mut u8, u32) -> *mut u8;
    match orig::<F>(RESIZE_LAST) {
        Some(f) => unsafe { stateful(RESIZE_LAST, this, || f(this, ptr, size), || resize_last(this, ptr, size)) },
        None => unsafe { resize_last(this, ptr, size) },
    }
}

unsafe extern "thiscall" fn release_hook(this: *mut u8, n: u32) -> u8 {
    match orig::<Thiscall1<u8>>(RELEASE) {
        Some(f) => unsafe { stateful(RELEASE, this, || f(this, n), || release(this, n)) },
        None => unsafe { release(this, n) },
    }
}

unsafe extern "thiscall" fn align_to_hook(this: *mut u8, a: u32) -> u8 {
    match orig::<Thiscall1<u8>>(ALIGN_TO) {
        Some(f) => unsafe { stateful(ALIGN_TO, this, || f(this, a), || align_to(this, a)) },
        None => unsafe { align_to(this, a) },
    }
}

unsafe extern "thiscall" fn reset_static_hook(this: *mut u8) -> u8 {
    match orig::<Thiscall0<u8>>(RESET_STATIC) {
        Some(f) => unsafe { stateful(RESET_STATIC, this, || f(this), || reset_static(this)) },
        None => unsafe { reset_static(this) },
    }
}

unsafe extern "thiscall" fn reset_mempool_hook(this: *mut u8) -> u32 {
    match orig::<Thiscall0<u32>>(RESET_MEMPOOL) {
        Some(f) => unsafe { stateful(RESET_MEMPOOL, this, || f(this), || reset_mempool(this)) },
        None => unsafe { reset_mempool(this) },
    }
}

unsafe extern "thiscall" fn set_limit_hook(this: *mut u8, x: u32) -> u8 {
    match orig::<Thiscall1<u8>>(SET_LIMIT) {
        Some(f) => unsafe { stateful(SET_LIMIT, this, || f(this, x), || set_limit(this, x)) },
        None => unsafe { set_limit(this, x) },
    }
}

unsafe extern "thiscall" fn init_hook(this: *mut u8, base: u32, size: u32, name: *const u8, align: u32, flags: u32) {
    type F = extern "thiscall" fn(*mut u8, u32, u32, *const u8, u32, u32);
    match orig::<F>(INIT) {
        Some(f) => unsafe {
            stateful(INIT, this, || f(this, base, size, name, align, flags), || init(this, base, size, name, align, flags))
        },
        None => unsafe { init(this, base, size, name, align, flags) },
    }
}

// --- tracked_mempool ------------------------------------------------------------

/// `tracked_mempool::alloc(size, align)`: forwards to slot 28 with the tracking
/// arguments (`this+0x60`, `[this+0xA0]`), which this build ignores.
unsafe fn tracked_alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        type F = extern "thiscall" fn(*mut u8, u32, u32, *mut u8, u32) -> *mut u8;
        let impl_: F = vcall(this, SLOT_TRACKED_ALLOC_IMPL);
        impl_(this, size, align, this.add(0x60), *u32_at(this, 0xA0))
    }
}

/// `tracked_mempool::alloc_impl(size, align, tag, line)` (`ret 0x10`): the base
/// alloc, called directly (so through [`alloc_hook`]).
unsafe fn tracked_alloc_impl(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        let alloc: Thiscall2<*mut u8> = std::mem::transmute(PORTS[ALLOC].1);
        alloc(this, size, align)
    }
}

unsafe extern "thiscall" fn tracked_alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    match orig::<Thiscall2<*mut u8>>(T_ALLOC) {
        Some(f) => unsafe { stateful(T_ALLOC, this, || f(this, size, align), || tracked_alloc(this, size, align)) },
        None => unsafe { tracked_alloc(this, size, align) },
    }
}

unsafe extern "thiscall" fn tracked_alloc_impl_hook(this: *mut u8, size: u32, align: u32, tag: *mut u8, line: u32) -> *mut u8 {
    type F = extern "thiscall" fn(*mut u8, u32, u32, *mut u8, u32) -> *mut u8;
    match orig::<F>(T_ALLOC_IMPL) {
        Some(f) => unsafe {
            stateful(T_ALLOC_IMPL, this, || f(this, size, align, tag, line), || tracked_alloc_impl(this, size, align))
        },
        None => unsafe { tracked_alloc_impl(this, size, align) },
    }
}

// --- async_mempool ---------------------------------------------------------------

/// `async_mempool::alloc(size, align)`: lock-free. Over-allocates by `align`
/// when it exceeds the pool default, rounds the request up, adds it to `used`
/// atomically, and aligns the returned pointer. On overflow `used` is clamped
/// to the size and NULL returned.
unsafe fn async_alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        let default = *u32_at(this, 0x40);
        let (a, n) = if align != 0 && align > default { (align, size.wrapping_add(align)) } else { (default, size) };
        let round = |v: u32| {
            let r = v % a;
            v.wrapping_add(if r == 0 { 0 } else { a - r })
        };
        let n = round(n);
        let used = AtomicU32::from_ptr(u32_at(this, 0x44));
        let old = used.fetch_add(n, Ordering::SeqCst);
        if old.wrapping_add(n) > *u32_at(this, 0x3C) {
            used.store(*u32_at(this, 0x3C), Ordering::SeqCst);
            return std::ptr::null_mut();
        }
        round((*u32_at(this, 0x38)).wrapping_add(old)) as *mut u8
    }
}

unsafe extern "thiscall" fn async_alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    let Some(f) = orig::<Thiscall2<*mut u8>>(A_ALLOC) else { return unsafe { async_alloc(this, size, align) } };
    CALLS[A_ALLOC].fetch_add(1, Ordering::Relaxed);
    let _guard = ASYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let used = u32_at(this, 0x44);
        let before = *used;
        let ro = f(this, size, align);
        let after_o = *used;
        *used = before;
        let rr = async_alloc(this, size, align);
        if rr != ro || *used != after_o {
            mismatch(A_ALLOC, this, format!("size {size:#x} align {align}: original {ro:p} used {after_o:#x}, rust {rr:p} used {:#x}", *used));
            *used = after_o;
            return ro;
        }
        rr
    }
}

// --- virtual_mempool ---------------------------------------------------------------

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

fn page_round(v: u32) -> u32 {
    let r = v & 0xFFF;
    v.wrapping_add(if r == 0 { 0 } else { 0x1000 - r })
}

/// `virtual_mempool::alloc(size, align)` (0xBFCA60): the bump allocator under
/// the pool lock, committing pages (through the page tracker) as it grows.
unsafe fn virtual_alloc(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    unsafe {
        let f = |o: usize| u32_at(this, o);
        if disabled_or_invalid(this) {
            printf()(MSG_ASSERT_PERMISSION as *const u8);
            return std::ptr::null_mut();
        }
        let cs = this.add(VM_LOCK);
        lock(cs, true);
        if *this.add(0x2C) & 1 != 0 && *f(0x44) != 0 {
            lock(cs, false);
            return std::ptr::null_mut();
        }
        let pad = pad_for(this, align);
        let new_used = (*f(0x44)).wrapping_add(pad).wrapping_add(size);
        if new_used > *f(0x3C) {
            lock(cs, false);
            return std::ptr::null_mut();
        }
        let committed = *f(0x68);
        if new_used > committed {
            let end = page_round(new_used);
            let tracker = *f(0x70) as *mut u8;
            let pages = end.wrapping_sub(committed) >> 12;
            if !tracker.is_null() && !page_tracker_reserve(tracker, pages) {
                printf()(MSG_NO_PAGES as *const u8);
                lock(cs, false);
                return std::ptr::null_mut();
            }
            let virtual_alloc: extern "system" fn(usize, usize, u32, u32) -> usize =
                std::mem::transmute(*(IAT_VIRTUAL_ALLOC as *const usize));
            let at = (*f(0x38)).wrapping_add(committed) as usize;
            if virtual_alloc(at, end.wrapping_sub(committed) as usize, 0x1000, 4) == 0 {
                // The original reads tracker+0x24 even without a tracker (and
                // faults); here a missing tracker reads as 0 pages.
                let free_pages = if tracker.is_null() { 0 } else { *(tracker.add(0x24) as *const u32) };
                printf()(
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
            V_COMMITS.fetch_add(1, Ordering::Relaxed);
        }
        *f(0x34) = (*f(0x44)).wrapping_add(*f(0x38)).wrapping_add(pad);
        *f(0x44) = (*f(0x44)).wrapping_add(pad).wrapping_add(size);
        let r = *f(0x34) as *mut u8;
        lock(cs, false);
        r
    }
}

/// `virtual_mempool::release_to(pos)` (0xBFC870): rewind `used` (the mark is
/// not updated, unlike the static version), decommit whole pages above it and
/// give them back to the page tracker, unless flag bit 1 is set. With flag
/// bit 0, the released bytes are zeroed after the lock is dropped.
unsafe fn virtual_release_to(this: *mut u8, pos: u32) -> u8 {
    unsafe {
        let f = |o: usize| u32_at(this, o);
        if disabled_or_invalid(this) {
            return 0;
        }
        let p = if pos == u32::MAX { *f(0x48) } else { pos };
        let cs = this.add(VM_LOCK);
        lock(cs, true);
        *f(0x44) = p;
        if *f(0x6C) & 2 != 0 {
            lock(cs, false);
            return 1;
        }
        let end = page_round(*f(0x44));
        let committed = *f(0x68);
        if end < committed {
            let virtual_free: extern "system" fn(usize, usize, u32) -> i32 =
                std::mem::transmute(*(IAT_VIRTUAL_FREE as *const usize));
            virtual_free((*f(0x38)).wrapping_add(end) as usize, committed.wrapping_sub(end) as usize, 0x4000);
            let tracker = *f(0x70) as *mut u8;
            if !tracker.is_null() {
                page_tracker_release(tracker, committed.wrapping_sub(end) >> 12);
            }
            *f(0x68) = end;
            V_COMMITS.fetch_add(1, Ordering::Relaxed);
        }
        lock(cs, false);
        if *f(0x6C) & 1 != 0 {
            let memset: extern "C" fn(usize, i32, u32) -> usize = std::mem::transmute(CRT_MEMSET);
            memset((*f(0x44)).wrapping_add(*f(0x38)) as usize, 0, (*f(0x68)).wrapping_sub(*f(0x44)));
        }
        1
    }
}

/// Shadow check for the virtual pool: holds the pool lock and the tracker's
/// (both recursive, same order as the original) across both runs, never
/// compares or restores the lock bytes, and rewinds and compares the tracker
/// counters (MEM_COMMIT/MEM_DECOMMIT and the zero-fill are repeatable). A
/// reservation that made the tracker compact is left to the original.
unsafe fn virtual_shadow<T: PartialEq + Copy + std::fmt::Debug>(
    i: usize,
    this: *mut u8,
    original: impl FnOnce() -> T,
    rust: impl FnOnce() -> T,
) -> T {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let cs = this.add(VM_LOCK);
        lock(cs, true);
        let tracker = *(this.add(0x70) as *const *mut u8);
        if !tracker.is_null() {
            lock(tracker, true);
        }
        let header = || {
            let mut b = [0u8; VM_HEADER];
            std::ptr::copy_nonoverlapping(this, b.as_mut_ptr(), VM_HEADER);
            b[VM_LOCK..VM_LOCK_END].fill(0);
            b
        };
        let set_header = |b: &[u8; VM_HEADER]| {
            std::ptr::copy_nonoverlapping(b.as_ptr(), this, VM_LOCK);
            std::ptr::copy_nonoverlapping(b.as_ptr().add(VM_LOCK_END), this.add(VM_LOCK_END), VM_HEADER - VM_LOCK_END);
        };
        let counters = || {
            let mut c = [0u8; 16];
            if !tracker.is_null() {
                std::ptr::copy_nonoverlapping(tracker.add(TRACKER_COUNTERS), c.as_mut_ptr(), 16);
            }
            c
        };
        let set_counters = |c: &[u8; 16]| {
            if !tracker.is_null() {
                std::ptr::copy_nonoverlapping(c.as_ptr(), tracker.add(TRACKER_COUNTERS), 16);
            }
        };
        let free_pages = |c: &[u8; 16]| u32::from_le_bytes(c[8..12].try_into().unwrap());
        let committed = |b: &[u8; VM_HEADER]| u32::from_le_bytes(b[0x68..0x6C].try_into().unwrap());

        let (h0, c0) = (header(), counters());
        let ro = original();
        let (h1, c1) = (header(), counters());
        let mut result = ro;
        let grew = committed(&h1) > committed(&h0);
        let compacted = grew
            && !tracker.is_null()
            && free_pages(&c0).wrapping_sub(free_pages(&c1)) != (committed(&h1) - committed(&h0)) >> 12;
        if compacted {
            V_SKIPPED.fetch_add(1, Ordering::Relaxed);
        } else {
            set_header(&h0);
            set_counters(&c0);
            let rr = rust();
            let (h2, c2) = (header(), counters());
            if rr != ro || h2 != h1 || c2 != c1 {
                let at = (0..VM_HEADER).find(|&k| h2[k] != h1[k]);
                mismatch(i, this, format!(
                    "original {ro:?}, rust {rr:?}, first differing header byte {at:x?}, tracker {}",
                    if c2 == c1 { "same" } else { "DIFFERS" }
                ));
                set_header(&h1);
                set_counters(&c1);
            } else {
                result = rr;
            }
        }
        if !tracker.is_null() {
            lock(tracker, false);
        }
        lock(cs, false);
        result
    }
}

unsafe extern "thiscall" fn virtual_alloc_hook(this: *mut u8, size: u32, align: u32) -> *mut u8 {
    match orig::<Thiscall2<*mut u8>>(V_ALLOC) {
        Some(f) => unsafe { virtual_shadow(V_ALLOC, this, || f(this, size, align), || virtual_alloc(this, size, align)) },
        None => unsafe { virtual_alloc(this, size, align) },
    }
}

unsafe extern "thiscall" fn virtual_release_to_hook(this: *mut u8, pos: u32) -> u8 {
    match orig::<Thiscall1<u8>>(V_RELEASE_TO) {
        Some(f) => unsafe { virtual_shadow(V_RELEASE_TO, this, || f(this, pos), || virtual_release_to(this, pos)) },
        None => unsafe { virtual_release_to(this, pos) },
    }
}

// --- Register-exact entry thunks for the leaf methods ------------------------------
//
// With LTCG, a direct caller of a leaf function may keep values in the
// registers that function happens not to touch (the first build without these
// thunks crashed in a text parser at 0xBF82F3 for that reason). Methods that
// call a virtual function clobber EAX/ECX/EDX anyway, so only the leaves need
// this. Each thunk reproduces its original's exact register outputs.

/// `is_valid`: only AL changes (`mov al,[dbg]; mov al,1`).
#[unsafe(naked)]
extern "C" fn is_valid_entry() {
    std::arch::naked_asm!(
        "push eax",
        "push ecx",
        "push edx",
        "call {h}", // thiscall, ECX = this
        "pop edx",
        "pop ecx",
        "mov byte ptr [esp], al",
        "pop eax",
        "ret",
        h = sym is_valid_hook,
    )
}

/// `mempool`/`string_pool::bytes_free`: EDX kept, ECX left = `used`.
#[unsafe(naked)]
extern "C" fn bytes_free_end_entry() {
    std::arch::naked_asm!(
        "push edx",
        "push ecx",
        "call {h}",
        "pop ecx",
        "mov ecx, [ecx + 0x44]",
        "pop edx",
        "ret",
        h = sym bytes_free_end_hook::<1>,
    )
}

#[unsafe(naked)]
extern "C" fn bytes_free_end_str_entry() {
    std::arch::naked_asm!(
        "push edx",
        "push ecx",
        "call {h}",
        "pop ecx",
        "mov ecx, [ecx + 0x44]",
        "pop edx",
        "ret",
        h = sym bytes_free_end_hook::<2>,
    )
}

/// `size - used` variants: ECX kept, EDX left = `used`.
macro_rules! bytes_free_size_entry {
    ($name:ident, $i:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx",
                "call {h}",
                "pop ecx",
                "mov edx, [ecx + 0x44]",
                "ret",
                h = sym bytes_free_size_hook::<$i>,
            )
        }
    };
}
bytes_free_size_entry!(bytes_free_size_v_entry, 3);
bytes_free_size_entry!(bytes_free_size_a_entry, 4);
bytes_free_size_entry!(bytes_free_size_c_entry, 5);

/// `size`/`base`: only EAX changes.
#[unsafe(naked)]
extern "C" fn size_entry() {
    std::arch::naked_asm!("push ecx", "push edx", "call {h}", "pop edx", "pop ecx", "ret", h = sym size_hook)
}

#[unsafe(naked)]
extern "C" fn base_entry() {
    std::arch::naked_asm!("push ecx", "push edx", "call {h}", "pop edx", "pop ecx", "ret", h = sym base_hook)
}

/// `mark`: ECX/EDX kept, EAX left = `used`.
#[unsafe(naked)]
extern "C" fn mark_entry() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "call {h}",
        "pop edx",
        "pop ecx",
        "mov eax, [ecx + 0x44]",
        "ret",
        h = sym mark_hook,
    )
}

/// `used_bytes`: calls a virtual (so EDX is fair game), but leaves ECX = EAX.
#[unsafe(naked)]
extern "C" fn used_bytes_entry() {
    std::arch::naked_asm!("call {h}", "mov ecx, eax", "ret", h = sym used_bytes_hook)
}

// --- Install and report ---------------------------------------------------------------

fn hook_address(i: usize) -> usize {
    const _: () = assert!(BYTES_FREE_END == 1 && BYTES_FREE_END_STR == 2);
    const _: () = assert!(BYTES_FREE_SIZE_V == 3 && BYTES_FREE_SIZE_A == 4 && BYTES_FREE_SIZE_C == 5);
    match i {
        IS_VALID => is_valid_entry as extern "C" fn() as usize,
        BYTES_FREE_END => bytes_free_end_entry as extern "C" fn() as usize,
        BYTES_FREE_END_STR => bytes_free_end_str_entry as extern "C" fn() as usize,
        BYTES_FREE_SIZE_V => bytes_free_size_v_entry as extern "C" fn() as usize,
        BYTES_FREE_SIZE_A => bytes_free_size_a_entry as extern "C" fn() as usize,
        BYTES_FREE_SIZE_C => bytes_free_size_c_entry as extern "C" fn() as usize,
        USED_BYTES => used_bytes_entry as extern "C" fn() as usize,
        SIZE => size_entry as extern "C" fn() as usize,
        BASE => base_entry as extern "C" fn() as usize,
        MARK => mark_entry as extern "C" fn() as usize,
        RELEASE_TO => release_to_hook as unsafe extern "thiscall" fn(*mut u8, u32) -> u8 as usize,
        CAN_ALLOC => can_alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> u8 as usize,
        ALLOC => alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
        RESIZE_LAST => resize_last_hook as unsafe extern "thiscall" fn(*mut u8, *mut u8, u32) -> *mut u8 as usize,
        RELEASE => release_hook as unsafe extern "thiscall" fn(*mut u8, u32) -> u8 as usize,
        ALIGN_TO => align_to_hook as unsafe extern "thiscall" fn(*mut u8, u32) -> u8 as usize,
        RESET_STATIC => reset_static_hook as unsafe extern "thiscall" fn(*mut u8) -> u8 as usize,
        RESET_MEMPOOL => reset_mempool_hook as unsafe extern "thiscall" fn(*mut u8) -> u32 as usize,
        SET_LIMIT => set_limit_hook as unsafe extern "thiscall" fn(*mut u8, u32) -> u8 as usize,
        INIT => init_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32, *const u8, u32, u32) as usize,
        V_ALLOC => virtual_alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
        V_RELEASE_TO => virtual_release_to_hook as unsafe extern "thiscall" fn(*mut u8, u32) -> u8 as usize,
        A_ALLOC => async_alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
        T_ALLOC => tracked_alloc_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32) -> *mut u8 as usize,
        T_ALLOC_IMPL => {
            tracked_alloc_impl_hook as unsafe extern "thiscall" fn(*mut u8, u32, u32, *mut u8, u32) -> *mut u8 as usize
        }
        _ => unreachable!(),
    }
}

/// Detour every ported method. Pools are created and used before this runs, so
/// a call can be inside a method while it is patched; each prologue is whole
/// instructions and the jump is written in one 5-byte store per method, the
/// same as the other hooks.
pub fn install() {
    let mut ok = 0;
    for (i, &(name, addr, prologue)) in PORTS.iter().enumerate() {
        match unsafe { detour::install(addr, prologue, hook_address(i)) } {
            Ok(tramp) => {
                ORIG[i].store(tramp, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("mempool: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    log(&format!("replaced {ok}/{COUNT} memory-pool methods with Rust (inline detours, shadow-checked)"));
}

pub fn summary() -> String {
    let mut parts = Vec::new();
    let mut total = (0u64, 0u64);
    for (i, &(name, ..)) in PORTS.iter().enumerate() {
        let (c, m) = (CALLS[i].load(Ordering::Relaxed), MISMATCHES[i].load(Ordering::Relaxed));
        total.0 += c as u64;
        total.1 += m as u64;
        if c > 0 {
            parts.push(format!("{name} {c}{}", if m > 0 { format!(" ({m} MISMATCH)") } else { String::new() }));
        }
    }
    format!(
        "mempool live: {} calls, {} mismatches, {} virtual commits/decommits by Rust, {} tracker compactions left to the original | {}",
        total.0,
        total.1,
        V_COMMITS.load(Ordering::Relaxed),
        V_SKIPPED.load(Ordering::Relaxed),
        parts.join(", ")
    )
}
