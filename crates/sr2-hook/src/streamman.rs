//! World streamer (`streamman`), docs/analysis/05_streamer.md.
//!
//! Ported so far: the request queue the streamer uses for its requests and
//! read batches (a fixed-capacity, priority-picked list), the slot-state
//! queries behind resource handles, and the control flow on top of them: the
//! per-frame pump, unload, abort and wait. The queue and the queries are inline
//! detours that run the port and shadow-check it against the original on every
//! call:
//! - queries: both run, results compared (re-reading once on a difference);
//! - queue methods: snapshot the queue, run the original, rewind, run the port,
//!   compare the whole queue and the result, and keep the original's outcome on
//!   a mismatch.
//!
//! The control flow calls I/O and virtual functions, so it can't be rewound.
//! `PUMP_RUST` switches it between the port and the originals; both modes
//! record the same distinct events in `sr2-streamman.log`, so an original run
//! and a Rust run can be compared.
//!
//! Queue layout (0x18 bytes): +0x00 count, +0x04 capacity, +0x08 node array,
//! +0x0C used list, +0x10 free list, +0x14 compare callback
//! `int(*)(u32 best, u32 candidate)`. Nodes are 12 bytes `{value, next, prev}`
//! on circular doubly linked lists; a node's index is its position in the array.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// `0x2225B3C[type]`: that type's array of 0x28-byte slot records.
const SLOT_ARRAYS: usize = 0x0222_5B3C;
const SLOT_SIZE: usize = 0x28;

/// The slot being processed (NULL when none), the request queue, and the batch lists.
const CURRENT: usize = 0x0252_9FF0;
const REQUEST_QUEUE: usize = 0x02F9_B88C;
const BATCHES_FREE: usize = 0x0252_A018;
const BATCHES_ACTIVE: usize = 0x0252_A01C;

/// Game functions the control flow calls (all cdecl).
const READ_CANCEL: usize = 0x00A7_4150; // (id lo, id hi)
const READ_RELEASE: usize = 0x00A7_4270; // (id lo, id hi, mode)
const BATCH_CANCEL_READS: usize = 0x00A7_D9C0; // (batch)
const BATCH_RELEASE_A: usize = 0x00A7_DA30; // (batch, mode)
const BATCH_RELEASE_B: usize = 0x00A7_DB30; // (batch, mode)
const START_LOAD_EXTRAS: usize = 0x00A7_E100; // (slot), type 4
const POLL_BATCH: usize = 0x00A7_ED40; // (batch)
const POLL_BATCH_EXTRAS: usize = 0x00A7_E900; // (batch), type 4
const PROCESS_ANIMATION: usize = 0x00A7_D0C0; // (), type 6
const STREAMMAN_NOP: usize = 0x00A7_D810;
const IAT_SLEEP: usize = 0x00DB_80B8;
const FILE_GET_SIZE: usize = 0x00BF_E0E0; // ESI = u32 *out, cdecl (name) -> AL
const READ_NODE_ALLOC: usize = 0x00A7_D930; // () -> node, from the pool 0x252A020
const BATCH_FREE_NODES: usize = 0x00A7_DC30; // (batch)
const ASYNC_IO_SUBMIT: usize = 0x00A7_3290; // stdcall (io manager, desc[8], bool) -> u64 request id
const IO_MANAGER: usize = 0x00EB_80E0; // -> io manager*

/// Run the control flow in Rust (false: the originals, with the same event log).
const PUMP_RUST: bool = true;

const Q_HEADER: usize = 0x18;
const NODE: usize = 12;

const Q_INIT: usize = 0;
const Q_PUSH: usize = 1;
const Q_REMOVE: usize = 2;
const Q_POP_BEST: usize = 3;
const Q_NODE_INDEX: usize = 4;
const IS_IDLE: usize = 5;
const IS_LOADED: usize = 6;
const IS_UNUSED: usize = 7;
const GET_SLOT_INFO: usize = 8;
const CLEAR_SLOT: usize = 9;
const ABORT_CURRENT: usize = 10;
const UNLOAD: usize = 11;
const PROCESS: usize = 12;
const WAIT: usize = 13;
const START_LOAD_I: usize = 14;
const COUNT: usize = 15;

/// (name, address, stolen prologue): whole, position-independent instructions.
const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("queue_init", 0x00A7_2E20, &[0x53, 0x33, 0xDB, 0x57, 0x33, 0xFF]),
    ("queue_push", 0x00A7_2C70, &[0x8B, 0x01, 0x3B, 0x41, 0x04]),
    ("queue_remove", 0x00A7_2D10, &[0x8B, 0x4A, 0x08, 0x8D, 0x04, 0x40]),
    ("queue_pop_best", 0x00A7_2DA0, &[0x53, 0x8B, 0x5C, 0x24, 0x08]),
    ("queue_node_index", 0x00A7_2E80, &[0x2B, 0x41, 0x08, 0x8B, 0xC8]),
    ("is_idle", 0x00A7_D560, &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0xC8]),
    ("is_loaded", 0x00A7_D5A0, &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0xC8]),
    ("is_unused", 0x00A7_D5D0, &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0xC8]),
    ("get_slot_info", 0x00A7_D670, &[0x8B, 0x44, 0x24, 0x0C, 0x8B, 0xC8]),
    ("clear_slot", 0x00A7_D6A0, &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0xC8]),
    ("abort_current", 0x00A7_CE70, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8]),
    ("unload", 0x00A7_D4F0, &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0xC8]),
    ("process", 0x00A7_D6D0, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8]),
    ("wait", 0x00A7_D600, &[0x56, 0x8B, 0x74, 0x24, 0x08]),
    ("start_load", 0x00A7_DDE0, &[0x83, 0xEC, 0x30, 0x53, 0x8B, 0x1D, 0x18, 0xA0, 0x52, 0x02]),
];

/// Trampolines to the originals.
static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

fn mismatch(i: usize, detail: String) {
    if MISMATCHES[i].fetch_add(1, Ordering::Relaxed) < 5 {
        log(&format!("streamman {} mismatch: {detail}", PORTS[i].0));
    }
}

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}

// --- The queue ------------------------------------------------------------------

/// Unlink `n` from whatever list it is on (the neighbours' links only).
unsafe fn unlink(n: usize) {
    unsafe {
        let (next, prev) = (rd(n + 4), rd(n + 8));
        if prev != 0 {
            wr(prev as usize + 4, next);
        }
        if next != 0 {
            wr(next as usize + 8, prev);
        }
        wr(n + 4, 0);
        wr(n + 8, 0);
    }
}

/// Append `n` at the tail of the circular list whose head is at `head`.
unsafe fn append(head: usize, n: usize) {
    unsafe {
        let h = rd(head) as usize;
        if h == 0 {
            wr(n + 4, n as u32);
            wr(n + 8, n as u32);
            wr(head, n as u32);
        } else {
            let tail = rd(h + 8) as usize;
            wr(n + 8, tail as u32);
            wr(n + 4, h as u32);
            wr(tail + 4, n as u32);
            wr(h + 8, n as u32);
        }
    }
}

fn node_index(q: usize, n: usize) -> i32 {
    unsafe { (n as i32).wrapping_sub(rd(q + 8) as i32) / NODE as i32 }
}

/// `queue_init` (0xA72E20): empty the queue and put every node on the free list in order.
unsafe fn q_init(q: usize) {
    unsafe {
        wr(q + 0x0C, 0);
        wr(q + 0x10, 0);
        wr(q, 0);
        let nodes = rd(q + 8) as usize;
        for i in 0..rd(q + 4) as usize {
            let n = nodes + i * NODE;
            wr(n, 0);
            wr(n + 4, 0);
            wr(n + 8, 0);
            append(q + 0x10, n);
        }
    }
}

/// `queue_push` (0xA72C70): take the free list's head, store `value` and append it
/// to the used list. Returns the node index, or -1 when the queue is full.
unsafe fn q_push(q: usize, value: u32) -> i32 {
    unsafe {
        if rd(q) == rd(q + 4) {
            return -1;
        }
        let n = rd(q + 0x10) as usize;
        if rd(n + 4) as usize == n {
            wr(n + 4, 0);
            wr(n + 8, 0);
            wr(q + 0x10, 0);
        } else {
            wr(q + 0x10, rd(n + 4));
            unlink(n);
        }
        append(q + 0x0C, n);
        wr(n, value);
        wr(q, rd(q).wrapping_add(1));
        node_index(q, n)
    }
}

/// `queue_remove` (0xA72D10): move node `index` from the used list to the free
/// list's tail and return its value. Returns the node's address too (the
/// original leaves it in ECX).
unsafe fn q_remove(q: usize, index: u32) -> (u32, usize) {
    unsafe {
        let n = (rd(q + 8) as usize).wrapping_add(index.wrapping_mul(NODE as u32) as usize);
        let value = rd(n);
        wr(n, 0);
        if rd(q + 0x0C) as usize == n && rd(n + 4) as usize == n {
            wr(n + 4, 0);
            wr(n + 8, 0);
            wr(q + 0x0C, 0);
        } else {
            if rd(q + 0x0C) as usize == n {
                wr(q + 0x0C, rd(n + 4));
            }
            unlink(n);
        }
        append(q + 0x10, n);
        wr(q, rd(q).wrapping_sub(1));
        (value, n)
    }
}

/// `queue_pop_best` (0xA72DA0): pick a value from the used list with the compare
/// callback (1: keep the lower value, the later one on a tie; 2: take the
/// candidate; anything else: keep the current best), remove it and return it.
/// Returns 0 when the queue is empty.
unsafe fn q_pop_best(q: usize) -> u32 {
    unsafe {
        if rd(q) == 0 {
            return 0;
        }
        let compare: extern "C" fn(u32, u32) -> i32 = std::mem::transmute(rd(q + 0x14) as usize);
        let head = rd(q + 0x0C) as usize;
        let mut best = 0usize;
        let mut cur = head;
        while cur != 0 {
            if best == 0 {
                best = cur;
            } else {
                match compare(rd(best), rd(cur)) {
                    1 if rd(best) >= rd(cur) => best = cur,
                    2 => best = cur,
                    _ => {}
                }
            }
            cur = rd(cur + 4) as usize;
            if cur == head {
                break;
            }
        }
        let value = rd(best);
        q_remove(q, node_index(q, best) as u32);
        value
    }
}

/// A queue's header and nodes, for the shadow check.
struct QueueState {
    header: [u8; Q_HEADER],
    nodes: Vec<u8>,
}

unsafe fn capture(q: usize) -> QueueState {
    unsafe {
        let mut header = [0u8; Q_HEADER];
        std::ptr::copy_nonoverlapping(q as *const u8, header.as_mut_ptr(), Q_HEADER);
        let (cap, nodes) = (rd(q + 4) as usize, rd(q + 8) as usize);
        let nodes = if nodes == 0 { Vec::new() } else { std::slice::from_raw_parts(nodes as *const u8, cap * NODE).to_vec() };
        QueueState { header, nodes }
    }
}

/// Write a captured state back. The node array's address and capacity are part
/// of the header, so they are restored before the nodes.
unsafe fn restore(q: usize, s: &QueueState) {
    unsafe {
        std::ptr::copy_nonoverlapping(s.header.as_ptr(), q as *mut u8, Q_HEADER);
        let nodes = rd(q + 8) as usize;
        if nodes != 0 {
            std::ptr::copy_nonoverlapping(s.nodes.as_ptr(), nodes as *mut u8, s.nodes.len());
        }
    }
}

impl QueueState {
    fn same(&self, o: &QueueState) -> bool {
        self.header == o.header && self.nodes == o.nodes
    }
}

/// Run the original, rewind, run the port, compare; on a difference keep the original's result.
unsafe fn shadow<R: PartialEq + Copy + std::fmt::Debug>(
    i: usize,
    q: usize,
    original: impl FnOnce() -> R,
    rust: impl FnOnce() -> R,
) -> R {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if orig(i) == 0 {
            return rust();
        }
        let before = capture(q);
        let ro = original();
        let after_o = capture(q);
        restore(q, &before);
        let rr = rust();
        let after_r = capture(q);
        if rr != ro || !after_r.same(&after_o) {
            mismatch(i, format!("queue {q:#x} count {}: original {ro:?}, rust {rr:?}, state equal {}", rd(q), after_r.same(&after_o)));
            restore(q, &after_o);
            return ro;
        }
        rr
    }
}

unsafe extern "thiscall" fn q_init_hook(q: usize) {
    let tramp = orig(Q_INIT);
    unsafe {
        shadow(
            Q_INIT,
            q,
            || {
                std::arch::asm!(
                    "push ebx", "push esi", "push edi", "push ebp",
                    "call {f}",
                    "pop ebp", "pop edi", "pop esi", "pop ebx",
                    f = in(reg) tramp, in("ecx") q, clobber_abi("C"),
                );
            },
            || q_init(q),
        )
    }
}

unsafe extern "thiscall" fn q_push_hook(q: usize, value: u32) -> i32 {
    unsafe {
        shadow(
            Q_PUSH,
            q,
            || {
                let f: extern "thiscall" fn(usize, u32) -> i32 = std::mem::transmute(orig(Q_PUSH));
                f(q, value)
            },
            || q_push(q, value),
        )
    }
}

/// Called by the remove thunk: EAX = index, EDX = queue. Returns the value and
/// stores the node address for the thunk to leave in ECX.
unsafe extern "C" fn q_remove_hook(index: u32, q: usize, node_out: *mut usize) -> u32 {
    let tramp = orig(Q_REMOVE);
    unsafe {
        let (v, n) = shadow(
            Q_REMOVE,
            q,
            || {
                let (v, n): (u32, usize);
                std::arch::asm!(
                    "push ebx", "push esi", "push edi", "push ebp",
                    "call {f}",
                    "pop ebp", "pop edi", "pop esi", "pop ebx",
                    f = in(reg) tramp, inout("eax") index => v, in("edx") q, lateout("ecx") n, clobber_abi("C"),
                );
                (v, n)
            },
            || q_remove(q, index),
        );
        *node_out = n;
        v
    }
}

unsafe extern "stdcall" fn q_pop_best_hook(q: usize) -> u32 {
    unsafe {
        shadow(
            Q_POP_BEST,
            q,
            || {
                let f: extern "stdcall" fn(usize) -> u32 = std::mem::transmute(orig(Q_POP_BEST));
                f(q)
            },
            || q_pop_best(q),
        )
    }
}

unsafe extern "C" fn q_node_index_hook(node: usize, q: usize) -> i32 {
    CALLS[Q_NODE_INDEX].fetch_add(1, Ordering::Relaxed);
    let rust = node_index(q, node);
    let tramp = orig(Q_NODE_INDEX);
    if tramp != 0 {
        let o: i32;
        unsafe {
            std::arch::asm!(
                "push ebx", "push esi", "push edi", "push ebp",
                "call {f}",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                f = in(reg) tramp, inout("eax") node => o, in("ecx") q, clobber_abi("C"),
            );
        }
        if o != rust {
            mismatch(Q_NODE_INDEX, format!("queue {q:#x} node {node:#x}: original {o}, rust {rust}"));
            return o;
        }
    }
    rust
}

/// EAX = index, EDX = queue -> EAX value; EDX kept, ECX = the node (as the original).
#[unsafe(naked)]
extern "C" fn q_remove_entry() {
    std::arch::naked_asm!(
        "push edx",
        "sub esp, 4",
        "push esp", // &node_out
        "push edx",
        "push eax",
        "call {h}",
        "add esp, 12",
        "pop ecx",
        "pop edx",
        "ret",
        h = sym q_remove_hook,
    )
}

/// ECX = queue; ECX kept (as the original), EAX/EDX free.
#[unsafe(naked)]
extern "C" fn q_init_entry() {
    std::arch::naked_asm!("push ecx", "call {h}", "pop ecx", "ret", h = sym q_init_hook)
}

/// EAX = node, ECX = queue -> EAX index.
#[unsafe(naked)]
extern "C" fn q_node_index_entry() {
    std::arch::naked_asm!("push ecx", "push eax", "call {h}", "add esp, 8", "ret", h = sym q_node_index_hook)
}

// --- Slot queries ---------------------------------------------------------------

/// Slot record of a handle (`type << 16 | index`), as the originals compute it.
unsafe fn slot(handle: u32) -> usize {
    unsafe { (rd(SLOT_ARRAYS + (handle >> 16) as usize * 4) as usize).wrapping_add((handle & 0xFFFF) as usize * SLOT_SIZE) }
}

fn query(i: usize, handle: u32, rust: impl Fn() -> u32) -> u32 {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    let tramp = orig(i);
    let r = rust();
    if tramp == 0 {
        return r;
    }
    let f: extern "C" fn(u32) -> u32 = unsafe { std::mem::transmute(tramp) };
    let o = f(handle) & 0xFF; // the originals only define AL
    if o != r && o != rust() {
        mismatch(i, format!("handle {handle:#x}: original {o}, rust {r}"));
        return o;
    }
    r
}

/// `streamman_is_idle` (0xA7D560): state 0 (idle), 3 (aborted/finished) or 4 (loaded).
extern "C" fn is_idle_hook(handle: u32) -> u32 {
    query(IS_IDLE, handle, || unsafe {
        let s = slot(handle);
        (s == 0 || matches!(rd(s + 4), 0 | 3 | 4)) as u32
    })
}

/// `streamman_is_loaded` (0xA7D5A0): state 4.
extern "C" fn is_loaded_hook(handle: u32) -> u32 {
    query(IS_LOADED, handle, || unsafe {
        let s = slot(handle);
        (s != 0 && rd(s + 4) == 4) as u32
    })
}

/// `streamman_is_unused` (0xA7D5D0): state 0.
extern "C" fn is_unused_hook(handle: u32) -> u32 {
    query(IS_UNUSED, handle, || unsafe {
        let s = slot(handle);
        (s == 0 || rd(s + 4) == 0) as u32
    })
}

/// `streamman_get_slot_info` (0xA7D670): `*a = slot+8`, `*b = slot+0xC`.
extern "C" fn get_slot_info_hook(a: *mut u32, b: *mut u32, handle: u32) {
    CALLS[GET_SLOT_INFO].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = slot(handle);
        let rust = (rd(s + 8), rd(s + 0x0C));
        let tramp = orig(GET_SLOT_INFO);
        if tramp != 0 {
            let f: extern "C" fn(*mut u32, *mut u32, u32) = std::mem::transmute(tramp);
            let mut o = (0u32, 0u32);
            f(&mut o.0, &mut o.1, handle);
            if o != rust {
                mismatch(GET_SLOT_INFO, format!("handle {handle:#x}: original {o:x?}, rust {rust:x?}"));
                (*a, *b) = o;
                return;
            }
        }
        (*a, *b) = rust;
    }
}

/// `streamman_clear_slot` (0xA7D6A0): state and +8/+0xC = 0. Three stores, so
/// it is not shadow-checked.
extern "C" fn clear_slot_hook(handle: u32) {
    CALLS[CLEAR_SLOT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = slot(handle);
        wr(s + 4, 0);
        wr(s + 8, 0);
        wr(s + 0x0C, 0);
    }
}

// --- Control flow: abort, unload, the pump, wait -------------------------------

/// Call a cdecl game function with up to three arguments, saving EBX/ESI/EDI/EBP
/// around it (LTCG callees may not).
unsafe fn call(f: usize, args: &[u32]) -> u32 {
    let mut a = [0u32; 3];
    a[..args.len()].copy_from_slice(args);
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "push dword ptr [{a} + 8]",
            "push dword ptr [{a} + 4]",
            "push dword ptr [{a}]",
            "call {f}",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            f = in(reg) f, a = in(reg) a.as_ptr(),
            lateout("eax") r, clobber_abi("C"),
        );
    }
    r
}

/// Call loading-info vtable slot `i` (thiscall, no arguments).
unsafe fn info_vcall(info: usize, i: usize) {
    unsafe {
        let f = rd(rd(info) as usize + i * 4) as usize;
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "call {f}",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            f = in(reg) f, in("ecx") info, clobber_abi("C"),
        );
    }
}

/// `streamman_batch_release` (0xA7DCD0, ESI = batch, EDI = mode; only called by
/// abort_current, so not detoured): cancel and release the batch's reads, then
/// move it from the active list to the free list's tail. Batches start with
/// `{prev, next}`.
unsafe fn batch_release(batch: usize, mode: u32) {
    unsafe {
        call(BATCH_CANCEL_READS, &[batch as u32]);
        call(BATCH_RELEASE_A, &[batch as u32, mode]);
        call(BATCH_RELEASE_B, &[batch as u32, mode]);
        if rd(BATCHES_ACTIVE) as usize == batch && rd(batch + 4) as usize == batch {
            wr(BATCHES_ACTIVE, 0);
        } else {
            if rd(BATCHES_ACTIVE) as usize == batch {
                wr(BATCHES_ACTIVE, rd(batch + 4));
            }
            let (prev, next) = (rd(batch), rd(batch + 4));
            if prev != 0 {
                wr(prev as usize + 4, next);
            }
            if next != 0 {
                wr(next as usize, prev);
            }
        }
        wr(batch, 0);
        wr(batch + 4, 0);
        let head = rd(BATCHES_FREE) as usize;
        if head == 0 {
            wr(batch, batch as u32);
            wr(batch + 4, batch as u32);
            wr(BATCHES_FREE, batch as u32);
        } else {
            let tail = rd(head) as usize;
            wr(batch, tail as u32);
            wr(batch + 4, head as u32);
            wr(tail + 4, batch as u32);
            wr(head, batch as u32);
        }
    }
}

/// Take a slot off the request queue if it is still queued.
unsafe fn dequeue(s: usize) {
    unsafe {
        if rd(s + 0x14) != u32::MAX {
            q_remove(REQUEST_QUEUE, rd(s + 0x14));
            wr(s + 0x14, u32::MAX);
        }
    }
}

/// `streamman_abort_current` (0xA7CE70): stop the current slot. A slot past the
/// request stage (state > 1) has its read (+0x18/+0x1C, a 64-bit id) or else its
/// batch (+0x20) cancelled, in mode 1 when the info's destination offset is above
/// 2 GB, else 2. Then loading-info slot 1 (abort) if `use_abort`, else slot 0
/// (unload); state 3, off the queue, no current slot.
unsafe fn abort_current(use_abort: bool) {
    unsafe {
        let cur = rd(CURRENT) as usize;
        if rd(cur + 4) as i32 > 1 {
            let mode = if rd(rd(cur + 0x10) as usize + 8) > 0x7FFF_FFFF { 1 } else { 2 };
            let (lo, hi) = (rd(cur + 0x18), rd(cur + 0x1C));
            if lo | hi != 0 {
                call(READ_CANCEL, &[lo, hi]);
                let cur = rd(CURRENT) as usize;
                call(READ_RELEASE, &[rd(cur + 0x18), rd(cur + 0x1C), mode]);
                let cur = rd(CURRENT) as usize;
                wr(cur + 0x18, 0);
                wr(cur + 0x1C, 0);
            } else if rd(cur + 0x20) != 0 {
                batch_release(rd(cur + 0x20) as usize, mode);
            }
        }
        info_vcall(rd(rd(CURRENT) as usize + 0x10) as usize, use_abort as usize);
        let cur = rd(CURRENT) as usize;
        wr(cur + 4, 3);
        dequeue(cur);
        wr(CURRENT, 0);
    }
}

/// `streamman_unload` (0xA7D4F0): the current slot is aborted through its
/// unload; any other gets loading-info slot 0 directly. Then off the queue,
/// state 0. Returns 1 only for a NULL slot.
unsafe fn unload(handle: u32) -> u32 {
    unsafe {
        let s = slot(handle);
        if s == 0 {
            return 1;
        }
        if rd(CURRENT) as usize == s {
            abort_current(false);
        } else {
            info_vcall(rd(s + 0x10) as usize, 0);
        }
        dequeue(s);
        wr(s + 4, 0);
        0
    }
}

/// `streamman_poll_active` (0xA7F0C0, only called by process_generic): poll every
/// active batch. A poll may unlink its batch, so the next one is read first.
unsafe fn poll_active() {
    unsafe {
        let mut b = rd(BATCHES_ACTIVE) as usize;
        while b != 0 {
            let next = rd(b + 4) as usize;
            let next = if next != rd(BATCHES_ACTIVE) as usize { next } else { 0 };
            let poll = if rd(rd(b + 0x14) as usize) == 4 { POLL_BATCH_EXTRAS } else { POLL_BATCH };
            call(poll, &[b as u32]);
            b = next;
        }
    }
}

/// `streamman_process_generic` (0xA7F100, only called by the pump): start the
/// current slot if it is requested (state 1), then poll the active batches.
unsafe fn process_generic() {
    unsafe {
        let cur = rd(CURRENT) as usize;
        if rd(cur + 4) == 1 {
            if rd(cur) == 4 {
                call(START_LOAD_EXTRAS, &[cur as u32]);
            } else {
                start_load_hook(cur as u32); // through the hook, for the event log
            }
        }
        poll_active();
    }
}

unsafe fn file_get_size(name: u32, out: &mut u32) -> bool {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov esi, {o}",
            "push {n}", "call {f}", "add esp, 4",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            o = in(reg) out as *mut u32, n = in(reg) name, f = in(reg) FILE_GET_SIZE,
            lateout("eax") r, clobber_abi("C"),
        );
    }
    r as u8 != 0
}

/// `async_io_submit` (stdcall): queue a read on the io thread; returns its 64-bit id.
unsafe fn io_submit(desc: &[u32; 8]) -> (u32, u32) {
    let (lo, hi): (u32, u32);
    unsafe {
        let mgr = rd(rd(IO_MANAGER) as usize);
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push 0", "push {d}", "push {m}", "call {f}",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            d = in(reg) desc.as_ptr(), m = in(reg) mgr, f = in(reg) ASYNC_IO_SUBMIT,
            lateout("eax") lo, lateout("edx") hi, clobber_abi("C"),
        );
    }
    (lo, hi)
}

/// Append `n` to the circular `{prev, next}` list whose head pointer is at `head`.
unsafe fn list_append(head: usize, n: usize) {
    unsafe {
        let h = rd(head) as usize;
        if h == 0 {
            wr(n, n as u32);
            wr(n + 4, n as u32);
            wr(head, n as u32);
        } else {
            let tail = rd(h) as usize;
            wr(n, tail as u32);
            wr(n + 4, h as u32);
            wr(tail + 4, n as u32);
            wr(h, n as u32);
        }
    }
}

/// `streamman_start_load` (0xA7DDE0): issue the reads for a requested slot.
///
/// Takes the free list's head batch (+0x08..+0x14 cleared, +0x18 = the chunk id for
/// type 0, else 0xFFFF). For each read step that passes the info's filter (+0x10;
/// default: the step names a file): set it as the current step, run its begin
/// callback, align the destination cursor (unless the info manages its own memory,
/// flag 0x100000), get the file's size and, if non-zero, check it fits, take a
/// read node `{prev, next, file, 0, .., +0x20 dest, +0x24 size, +0x28 priority
/// (0x70 for chunk geom/peg, else 0x40), +0x2C step}` onto the batch, then the issue
/// and after callbacks (after is skipped for chunk pegs) and advance the cursor. A
/// failing callback or a file too big (flag 0x200000) frees the nodes and aborts
/// the slot. If any reads were taken, the batch goes on the active list, every
/// node is submitted to the io thread (id at +0x10/+0x14) and the slot reads
/// (state 2). Otherwise the batch goes back on the free list after a failure;
/// after a run with nothing to read it is left off both lists, as in the original.
unsafe fn start_load(s: usize) {
    unsafe {
        let batch = rd(BATCHES_FREE) as usize;
        let info = rd(s + 0x10) as usize;
        if rd(batch + 4) as usize == batch {
            wr(batch + 4, 0);
            wr(batch, 0);
            wr(BATCHES_FREE, 0);
        } else {
            let (prev, next) = (rd(batch), rd(batch + 4));
            wr(BATCHES_FREE, next);
            if prev != 0 {
                wr(prev as usize + 4, next);
            }
            if next != 0 {
                wr(next as usize, prev);
            }
            wr(batch + 4, 0);
            wr(batch, 0);
        }
        for o in [8, 0x0C, 0x10, 0x14] {
            wr(batch + o, 0);
        }
        let id = if rd(s) == 0 { *((info + 0x52E) as *const u16) } else { 0xFFFF };
        *((batch + 0x18) as *mut u16) = id;

        let flags = |info: usize| rd(info + 0x20);
        let call_bool = |f: u32, arg: u32| call(f as usize, &[arg]) as u8 != 0;
        let (mut ok, mut failed) = (true, false);
        let mut i = 0u32;
        while i < (flags(info) >> 10) & 0x3FF {
            let step = rd(info + 0x14) as usize + i as usize * 0x244;
            let file = rd(step);
            let filter = rd(info + 0x10);
            let wanted = if filter != 0 { call_bool(filter, file) } else { file != 0 };
            if wanted {
                wr(info + 0x20, (flags(info) & !0x3FF) | (i & 0x3FF));
                let begin = rd(step + 0x22C);
                if begin != 0 && !call_bool(begin, info as u32) {
                    failed = true;
                    break;
                }
                if flags(info) & 0x10_0000 == 0 {
                    let align = *((step + 0x214) as *const u16) as u32;
                    let r = rd(info + 8) % align;
                    let pad = if r == 0 { 0 } else { align - r };
                    wr(info + 8, rd(info + 8).wrapping_add(pad));
                    wr(info + 0x0C, rd(info + 0x0C).wrapping_sub(pad));
                    wr(step + 0x218, rd(info + 8));
                }
                if file_get_size(file, &mut *((step + 0x210) as *mut u32)) {
                    let size = rd(step + 0x210);
                    if size != 0 {
                        if size > rd(info + 0x0C) {
                            wr(info + 0x20, flags(info) | 0x20_0000);
                            failed = true;
                            break;
                        }
                        let node = call(READ_NODE_ALLOC, &[]) as usize;
                        wr(node + 8, file);
                        wr(node + 0x0C, 0);
                        *((node + 0x28) as *mut u8) = if rd(s) <= 1 { 0x70 } else { 0x40 };
                        wr(node + 0x20, rd(info + 8));
                        wr(node + 0x2C, i);
                        wr(node + 0x24, size);
                        list_append(batch + 0x0C, node);
                        let issue = rd(step + 0x234);
                        if issue != 0 {
                            call(issue as usize, &[info as u32]);
                        }
                        let after = rd(step + 0x238);
                        if after != 0 && rd(s) != 1 && !call_bool(after, info as u32) {
                            failed = true;
                            break;
                        }
                        if flags(info) & 0x10_0000 == 0 {
                            wr(info + 8, rd(info + 8).wrapping_add(size));
                            wr(info + 0x0C, rd(info + 0x0C).wrapping_sub(size));
                        }
                    }
                }
            }
            i += 1;
        }
        if failed {
            ok = false;
            if rd(batch + 0x0C) != 0 {
                call(BATCH_FREE_NODES, &[batch as u32]);
            }
            abort_current_hook(0); // through the hook, for the event log
        }
        if rd(batch + 0x0C) != 0 {
            wr(s + 0x20, batch as u32);
            wr(s + 0x18, 0);
            wr(s + 0x1C, 0);
            wr(batch + 0x14, s as u32);
            list_append(BATCHES_ACTIVE, batch);
            let head = rd(batch + 0x0C) as usize;
            let mut n = head;
            while n != 0 {
                let desc = [rd(n + 8), rd(n + 0x0C), 0, rd(n + 0x24), rd(n + 0x20), *((n + 0x28) as *const u8) as u32, 0, 0];
                let (lo, hi) = io_submit(&desc);
                wr(n + 0x10, lo);
                wr(n + 0x14, hi);
                n = rd(n + 4) as usize;
                if n == rd(batch + 0x0C) as usize {
                    break;
                }
            }
            wr(s + 4, 2);
        } else if !ok {
            list_append(BATCHES_FREE, batch);
        }
    }
}

extern "C" fn start_load_hook(s: u32) {
    CALLS[START_LOAD_I].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = s as usize;
        let e = format!("start {}", describe(s));
        if PUMP_RUST {
            start_load(s);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(START_LOAD_I));
            f(s as u32);
        }
        // What was issued: the batch's reads (count, bytes, destination offsets
        // relative to the first), read back the same way in both modes.
        let (mut reads, mut bytes, mut dests) = (0u32, 0u32, String::new());
        if rd(s + 4) == 2 {
            let batch = rd(s + 0x20) as usize;
            let head = rd(batch + 0x0C) as usize;
            let mut n = head;
            while n != 0 {
                reads += 1;
                bytes = bytes.wrapping_add(rd(n + 0x24));
                dests += &format!(" {:x}/{:x}", rd(n + 0x20).wrapping_sub(rd(head + 0x20)), *((n + 0x28) as *const u8));
                n = rd(n + 4) as usize;
                if n == head {
                    break;
                }
            }
        }
        let flags = rd(rd(s + 0x10) as usize + 0x20);
        event(format!(
            "{e} -> s{} reads {reads} bytes {bytes:#x}{dests} too_big {} current {}",
            rd(s + 4),
            (flags >> 21) & 1,
            (rd(CURRENT) as usize == s) as u8
        ));
    }
}

/// `streamman_process` (0xA7D6D0), the pump: with no current slot, make the best
/// request current (nothing queued: return). Animations (type 6, and any type
/// above 9) have their own path.
unsafe fn process() {
    unsafe {
        let mut cur = rd(CURRENT) as usize;
        if cur == 0 {
            if rd(REQUEST_QUEUE) == 0 {
                call(STREAMMAN_NOP, &[]);
                return;
            }
            cur = q_pop_best(REQUEST_QUEUE) as usize;
            wr(CURRENT, cur as u32);
            wr(cur + 0x14, u32::MAX);
        }
        match rd(cur) {
            0..=5 | 7..=9 => process_generic(),
            _ => {
                call(PROCESS_ANIMATION, &[]);
            }
        }
    }
}

/// `streamman_wait` (0xA7D600): pump, with Sleep(1), until the slot is idle;
/// returns whether it ended up loaded.
unsafe fn wait(handle: u32) -> u32 {
    unsafe {
        let sleep: extern "system" fn(u32) = std::mem::transmute(rd(IAT_SLEEP) as usize);
        let idle = || {
            let s = slot(handle);
            s == 0 || matches!(rd(s + 4), 0 | 3 | 4)
        };
        while !idle() {
            process_hook(); // through the hook, so the event log matches the original's
            sleep(1);
        }
        let s = slot(handle);
        (s != 0 && rd(s + 4) == 4) as u32
    }
}

/// Distinct control-flow events with their counts, written to `sr2-streamman.log`.
static EVENTS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

fn event(e: String) {
    *EVENTS.lock().unwrap_or_else(|p| p.into_inner()).entry(e).or_insert(0) += 1;
}

/// A slot as "type:index sState", or "-" for none.
unsafe fn describe(s: usize) -> String {
    unsafe {
        if s == 0 {
            return "-".into();
        }
        let t = rd(s) as usize;
        let base = if t < 10 { rd(SLOT_ARRAYS + t * 4) as usize } else { 0 };
        format!("{t}:{} s{}", s.wrapping_sub(base) / SLOT_SIZE, rd(s + 4))
    }
}

extern "C" fn abort_current_hook(use_abort: u32) {
    CALLS[ABORT_CURRENT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let cur = rd(CURRENT) as usize;
        let e = format!(
            "abort {} use_abort {} reads {} batch {}",
            describe(cur),
            use_abort as u8,
            (rd(cur + 0x18) | rd(cur + 0x1C) != 0) as u8,
            (rd(cur + 0x20) != 0) as u8
        );
        if PUMP_RUST {
            abort_current(use_abort as u8 != 0);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(ABORT_CURRENT));
            f(use_abort);
        }
        event(format!("{e} -> s{} queued {}", rd(cur + 4), rd(cur + 0x14) as i32));
    }
}

extern "C" fn unload_hook(handle: u32) -> u32 {
    CALLS[UNLOAD].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = slot(handle);
        let e = format!("unload {} current {}", describe(s), (s != 0 && rd(CURRENT) as usize == s) as u8);
        let r = if PUMP_RUST {
            unload(handle)
        } else {
            let f: extern "C" fn(u32) -> u32 = std::mem::transmute(orig(UNLOAD));
            f(handle) & 0xFF
        };
        event(format!("{e} -> {r}"));
        r
    }
}

extern "C" fn process_hook() {
    CALLS[PROCESS].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let before = rd(CURRENT) as usize;
        let e = format!("process {} queued {}", describe(before), rd(REQUEST_QUEUE));
        if PUMP_RUST {
            process();
        } else {
            let f: extern "C" fn() = std::mem::transmute(orig(PROCESS));
            f();
        }
        let after = rd(CURRENT) as usize;
        if after != before {
            event(format!("{e} -> {}", describe(after)));
        }
    }
}

extern "C" fn wait_hook(handle: u32) -> u32 {
    CALLS[WAIT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let e = format!("wait {}", describe(slot(handle)));
        let r = if PUMP_RUST {
            wait(handle)
        } else {
            let f: extern "C" fn(u32) -> u32 = std::mem::transmute(orig(WAIT));
            f(handle) & 0xFF
        };
        event(format!("{e} -> {r}"));
        r
    }
}

fn write_events() {
    let events = EVENTS.lock().unwrap_or_else(|p| p.into_inner());
    let text: String = events.iter().map(|(e, n)| format!("{e}\t{n}\n")).collect();
    let _ = std::fs::write("sr2-streamman.log", text);
}

// --- Install and report ---------------------------------------------------------------

fn hook_address(i: usize) -> usize {
    match i {
        Q_INIT => q_init_entry as extern "C" fn() as usize,
        Q_PUSH => q_push_hook as unsafe extern "thiscall" fn(usize, u32) -> i32 as usize,
        Q_REMOVE => q_remove_entry as extern "C" fn() as usize,
        Q_POP_BEST => q_pop_best_hook as unsafe extern "stdcall" fn(usize) -> u32 as usize,
        Q_NODE_INDEX => q_node_index_entry as extern "C" fn() as usize,
        IS_IDLE => is_idle_hook as extern "C" fn(u32) -> u32 as usize,
        IS_LOADED => is_loaded_hook as extern "C" fn(u32) -> u32 as usize,
        IS_UNUSED => is_unused_hook as extern "C" fn(u32) -> u32 as usize,
        GET_SLOT_INFO => get_slot_info_hook as extern "C" fn(*mut u32, *mut u32, u32) as usize,
        CLEAR_SLOT => clear_slot_hook as extern "C" fn(u32) as usize,
        ABORT_CURRENT => abort_current_hook as extern "C" fn(u32) as usize,
        UNLOAD => unload_hook as extern "C" fn(u32) -> u32 as usize,
        PROCESS => process_hook as extern "C" fn() as usize,
        WAIT => wait_hook as extern "C" fn(u32) -> u32 as usize,
        START_LOAD_I => start_load_hook as extern "C" fn(u32) as usize,
        _ => unreachable!(),
    }
}

/// Detour every ported function. The queues are initialised during boot, after
/// the hooks go in (the first `DirectInput8Create`), so init is covered too.
pub fn install() {
    let mut ok = 0;
    for (i, &(name, addr, prologue)) in PORTS.iter().enumerate() {
        match unsafe { detour::install(addr, prologue, hook_address(i)) } {
            Ok(tramp) => {
                ORIG[i].store(tramp, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("streamman: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    let pump = if PUMP_RUST { "Rust" } else { "original, observed" };
    log(&format!(
        "replaced {ok}/{COUNT} streamman functions (inline detours; queue and queries Rust, shadow-checked; pump/unload/abort/wait: {pump})"
    ));
}

pub fn summary() -> String {
    write_events();
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
    format!("streamman live: {} calls, {} mismatches | {}", total.0, total.1, parts.join(", "))
}
