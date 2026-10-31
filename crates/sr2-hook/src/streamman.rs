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

const STREAMMAN_NOP: usize = 0x00A7_D810;
const IAT_SLEEP: usize = 0x00DB_80B8;
const FILE_GET_SIZE: usize = 0x00BF_E0E0; // ESI = u32 *out, cdecl (name) -> AL
const READ_NODE_ALLOC: usize = 0x00A7_D930; // () -> node, from the pool 0x252A020
const BATCH_FREE_NODES: usize = 0x00A7_DC30; // (batch)
const ASYNC_IO_SUBMIT: usize = 0x00A7_3290; // stdcall (io manager, desc[8], bool) -> u64 request id
const IO_MANAGER: usize = 0x00EB_80E0; // -> io manager*
const IO_STATUS: usize = 0x00A7_41A0; // cdecl (request, sequence) -> 1 queued, 2 reading, 3 done, 4 cancelled, 5 failed
const IO_FREE: usize = 0x00A7_3650; // stdcall (mgr, request, sequence, mode)
const READ_NODE_POOL: usize = 0x0252_A020;
const READ_NODE_POOL_COUNT: usize = 0x0252_A024;
/// Written by check_batch when a finished request's sequence no longer matches (a debug "last error" copy).
const IO_LAST_ERROR_SRC: usize = 0x0252_9E24;
const IO_LAST_ERROR: usize = 0x0252_9E28;

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
const CHECK_BATCH: usize = 15;
const FINISH: usize = 16;
const START_EXTRAS: usize = 17;
const PROCESS_SEQ: usize = 18;
const POLL_EXTRAS: usize = 19;
const STEP_INIT: usize = 20;
const UNLOAD_CHUNK_PEGS_EXTRAS: usize = 21;
const ANY_CHUNK_BUSY: usize = 22;
const CANCEL_CHUNK_LOADS: usize = 23;
const UNLOAD_FLAGGED_GEOM: usize = 24;
const REQUEST_ANIMATION: usize = 25;
const REQUEST_CHARACTER: usize = 26;
const REQUEST_VEHICLE: usize = 27;
const REQUEST_GEOM: usize = 28;
const REQUEST_PEG: usize = 29;
const COUNT: usize = 30;

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
    ("check_batch", 0x00A7_ED40, &[0x83, 0xEC, 0x50, 0xA1, 0x50, 0x0B, 0xE8, 0x00]),
    ("finish_current", 0x00A7_CFC0, &[0x83, 0xEC, 0x08, 0x53, 0x55, 0x56]),
    ("start_chunk_extras", 0x00A7_E100, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8]),
    ("process_sequential", 0x00A7_D0C0, &[0x83, 0xEC, 0x0C, 0x8B, 0x15, 0xF0, 0x9F, 0x52, 0x02]),
    ("poll_chunk_extras", 0x00A7_E900, &[0x83, 0xEC, 0x58, 0xA1, 0x50, 0x0B, 0xE8, 0x00]),
    ("step_init", 0x00A7_F770, &[0x8B, 0x54, 0x24, 0x04, 0x83, 0xA6, 0x1C, 0x02, 0x00, 0x00, 0xFE]),
    ("unload_chunk_pegs_extras", 0x00A8_0A40, &[0x51, 0x53, 0x66, 0x8B, 0x5C, 0x24, 0x0C]),
    ("any_chunk_busy", 0x00A8_0B40, &[0x53, 0x56, 0x57, 0xBF, 0x84, 0x7B, 0xE8, 0x00]),
    ("cancel_chunk_loads", 0x00A8_0BC0, &[0x83, 0xEC, 0x0C, 0x53, 0x55, 0x56]),
    ("unload_flagged_geom", 0x00A8_0CC0, &[0x56, 0x33, 0xF6, 0x39, 0x35, 0xB0, 0x7B, 0xE8, 0x00]),
    ("request_animation", 0x00A8_8240, &[0x53, 0x8B, 0x5C, 0x24, 0x08]),
    ("request_character", 0x00A7_FC30, &[0x8B, 0x4C, 0x24, 0x24, 0x53]),
    ("request_vehicle", 0x00A8_7F80, &[0x8B, 0x44, 0x24, 0x0C, 0x53]),
    ("request_chunk_geom", 0x00A8_5140, &[0x83, 0xEC, 0x08, 0x53, 0x56]),
    ("request_chunk_peg", 0x00A8_5890, &[0x83, 0xEC, 0x08, 0x53, 0x56]),
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
        let nodes =
            if nodes == 0 { Vec::new() } else { std::slice::from_raw_parts(nodes as *const u8, cap * NODE).to_vec() };
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
            mismatch(
                i,
                format!(
                    "queue {q:#x} count {}: original {ro:?}, rust {rr:?}, state equal {}",
                    rd(q),
                    after_r.same(&after_o)
                ),
            );
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
    unsafe {
        (rd(SLOT_ARRAYS + (handle >> 16) as usize * 4) as usize).wrapping_add((handle & 0xFFFF) as usize * SLOT_SIZE)
    }
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
            if rd(rd(b + 0x14) as usize) == 4 {
                poll_extras_hook(b as u32); // through the hook, for the event log
            } else {
                check_batch_hook(b as u32); // through the hook, for the event log
            }
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
                start_extras_hook(cur as u32); // through the hook, for the event log
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
                let desc =
                    [rd(n + 8), rd(n + 0x0C), 0, rd(n + 0x24), rd(n + 0x20), *((n + 0x28) as *const u8) as u32, 0, 0];
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

/// `async_io_free(request->manager, request, sequence, 0)` (stdcall, 4 arguments).
unsafe fn io_free(req: usize, seq: u32) {
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push 0", "push {q}", "push {r}", "push dword ptr [{r} + 0x3C]", "call {f}",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            r = in(reg) req, q = in(reg) seq, f = in(reg) IO_FREE, clobber_abi("C"),
        );
    }
}

/// Remove `n` from the circular `{prev, next}` list whose head pointer is at
/// `head` (moving the head on if it was `n`) and clear its links.
unsafe fn list_remove(head: usize, n: usize) {
    unsafe {
        if rd(head) as usize == n && rd(n + 4) as usize == n {
            wr(head, 0);
        } else {
            if rd(head) as usize == n {
                wr(head, rd(n + 4));
            }
            let (prev, next) = (rd(n), rd(n + 4));
            if prev != 0 {
                wr(prev as usize + 4, next);
            }
            if next != 0 {
                wr(next as usize, prev);
            }
        }
        wr(n, 0);
        wr(n + 4, 0);
    }
}

/// `streamman_check_batch` (0xA7ED40): poll a batch's pending reads (+0x0C) in
/// order. Queued: stop. Being read: the step's progress callback (+0x230, `(info,
/// node)`; false aborts), then stop. Done: for chunk pegs the step's after callback
/// (+0x238; false aborts), then the node moves to the done list (+0x10). Failed:
/// the step's retry callback (+0x240) must return 1, else the slot is aborted.
/// With nothing left pending, `finish_current` runs the completion, every request
/// is freed (`async_io_free`, mode 0), the nodes go back to the pool and the batch
/// to the free list.
///
/// Left out: two debug-only calls into the empty `streamman_nop` (0xA7D810) with
/// a formatted file name or error name, which have no effect.
unsafe fn check_batch(batch: usize) {
    unsafe {
        let s = rd(batch + 0x14) as usize;
        let info = rd(s + 0x10) as usize;
        let ok = |r: u32| r as u8 != 0;
        let mut node = rd(batch + 0x0C) as usize;
        while node != 0 {
            let next = rd(node + 4) as usize;
            let next = if next != rd(batch + 0x0C) as usize { next } else { 0 };
            let step = rd(info + 0x14) as usize + rd(node + 0x2C) as usize * 0x244;
            match call(IO_STATUS, &[rd(node + 0x10), rd(node + 0x14)]) {
                1 => break,
                2 => {
                    let progress = rd(step + 0x230) as usize;
                    if progress != 0 && !ok(call(progress, &[info as u32, node as u32])) {
                        abort_current_hook(0);
                        return;
                    }
                    break;
                }
                3 => {
                    let after = rd(step + 0x238) as usize;
                    if rd(s) == 1 && after != 0 && !ok(call(after, &[info as u32])) {
                        abort_current_hook(0);
                        return;
                    }
                    list_remove(batch + 0x0C, node);
                    list_append(batch + 0x10, node);
                    node = next;
                }
                5 => {
                    let retry = rd(step + 0x240) as usize;
                    if retry == 0 || call(retry, &[info as u32]) as u8 != 1 {
                        abort_current_hook(0);
                        return;
                    }
                    node = next;
                }
                _ => node = next,
            }
        }
        if rd(batch + 0x0C) != 0 {
            return;
        }
        wr(s + 0x20, 0);
        finish_current_hook(); // through the hook, for the event log
        let mut node = rd(batch + 0x10) as usize;
        while node != 0 {
            let next = rd(node + 4) as usize;
            let next = if next != rd(batch + 0x10) as usize { next } else { 0 };
            let (req, seq) = (rd(node + 0x10) as usize, rd(node + 0x14));
            if req == 0 || seq != rd(req) {
                wr(IO_LAST_ERROR, rd(rd(IO_LAST_ERROR_SRC) as usize));
            }
            io_free(req, seq);
            list_remove(batch + 0x10, node);
            wr(node + 0x10, 0);
            wr(node + 0x14, 0);
            wr(READ_NODE_POOL_COUNT, rd(READ_NODE_POOL_COUNT).wrapping_add(1));
            list_append(READ_NODE_POOL, node);
            node = next;
        }
        list_remove(BATCHES_ACTIVE, batch);
        list_append(BATCHES_FREE, batch);
    }
}

extern "C" fn check_batch_hook(batch: u32) {
    CALLS[CHECK_BATCH].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = rd(batch as usize + 0x14) as usize;
        let e = format!("check {}", describe(s));
        let state = rd(s + 4);
        if PUMP_RUST {
            check_batch(batch as usize);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(CHECK_BATCH));
            f(batch);
        }
        // Only state changes (completions, aborts): how many polls a read takes varies.
        if rd(s + 4) != state {
            event(format!("{e} -> s{} current {}", rd(s + 4), (rd(CURRENT) as usize == s) as u8));
        }
    }
}

/// `streamman_finish_current` (0xA7CFC0): replay the info's step sizes and
/// alignments into the current slot's byte counts (+0x0C for steps with flag
/// bit 0 at +0x21C, else +8), then run the info's completion callback (+0x1C).
/// True: state 4 (loaded), no current slot. False: abort (state 3).
unsafe fn finish_current() -> bool {
    unsafe {
        let cur = rd(CURRENT) as usize;
        let info = rd(cur + 0x10) as usize;
        wr(cur + 8, 0);
        wr(cur + 0x0C, 0);
        for k in 0..((rd(info + 0x20) >> 10) & 0x3FF) as usize {
            let step = rd(info + 0x14) as usize + k * 0x244;
            let size = rd(step + 0x210);
            let align = *((step + 0x214) as *const u16) as u32;
            let field = if rd(step + 0x21C) & 1 != 0 { cur + 0x0C } else { cur + 8 };
            if align > 0 {
                let r = rd(field) % align;
                wr(field, rd(field).wrapping_add(if r == 0 { 0 } else { align - r }));
            }
            wr(field, rd(field).wrapping_add(size));
        }
        if call(rd(info + 0x1C) as usize, &[info as u32]) as u8 == 0 {
            abort_current_hook(0);
            return false;
        }
        wr(rd(CURRENT) as usize + 4, 4);
        wr(CURRENT, 0);
        true
    }
}

extern "C" fn finish_current_hook() -> u32 {
    CALLS[FINISH].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let cur = rd(CURRENT) as usize;
        let e = format!("finish {}", describe(cur));
        let r = if PUMP_RUST {
            finish_current() as u32
        } else {
            let f: extern "C" fn() -> u32 = std::mem::transmute(orig(FINISH));
            f() & 0xFF
        };
        event(format!("{e} -> {r} s{} bytes {:#x}/{:#x}", rd(cur + 4), rd(cur + 8), rd(cur + 0x0C)));
        r
    }
}

// --- Chunk extras start ---------------------------------------------------------

const OBJECT_TEMPLATES: usize = 0x02BC_5B00; // 0x90-byte entries: +4 name hash, +8 name
const OBJECT_TEMPLATES_COUNT: usize = 0x0215_D788;
/// Streamed mesh cache: the two pools' active record lists (circular, next at +0;
/// record +0x110 = object template, +0x10C = mesh object, +0x122 = loaded).
const SMESH_CACHE_ACTIVE: [usize; 2] = [0x0151_259C, 0x0147_A514];
const STRING_HASH: usize = 0x00BD_CB30; // ECX = string, EAX = seed, (u32 *out), ret 4
const PATH_CHANGE_EXTENSION: usize = 0x00BF_F930; // ECX = src, cdecl (out, size, ext) -> bool
const FILE_RESOLVE: usize = 0x00BF_D880; // EAX = location (0x110 bytes, size at +0x108), cdecl (name) -> AL
const SMESH_CACHE_ADD_REF: usize = 0x006E_63F0; // ESI = object template, cdecl (chunk id) -> record
const SMESH_CACHE_RECORD_ADD_CHUNK: usize = 0x006E_6B30; // EDI = record, cdecl (chunk id) -> bool
const READ_NODES_FREE_LIST: usize = 0x00A7_DD50; // EAX = node list: every node back to the pool
const ANIM_EXT: usize = 0x00E0_CACC; // ".anim_pc"
const PART_EXTS: usize = 0x00E8_7B90; // [".smesh_pc", ".g_smesh_pc", ".peg_pc", ".g_peg_pc"]
const PART_ALIGNS: usize = 0x00E8_7BA0; // [0x40, 0x1000, 4, 0x40]
const SCRATCH_POOL: usize = 0x02FA_4018; // virtual_mempool for step 2
const VIRTUAL_MEMPOOL_ALLOC: usize = 0x00BF_CA60;

/// Call a game function with EAX/ECX/ESI/EDI set and three stack arguments,
/// saving EBX/ESI/EDI/EBP. ESP is restored from EBP afterwards, so it works for
/// cdecl and callee-clean functions alike, and for ones taking fewer arguments
/// (the extra pushes are ignored). Everything goes through one block in EDX so
/// no operand can sit in a register the sequence overwrites.
unsafe fn game_call(f: usize, eax: u32, ecx: u32, esi: u32, edi: u32, args: [u32; 3]) -> u32 {
    unsafe { game_call_edx(f, eax, ecx, 0, esi, edi, args).0 }
}

/// [`game_call`] with EDX as an input too; returns EDX:EAX as (EAX, EDX).
unsafe fn game_call_edx(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: [u32; 3]) -> (u32, u32) {
    let block: [u32; 7] = [f as u32, esi, edi, edx, args[0], args[1], args[2]];
    let (lo, hi): (u32, u32);
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "push dword ptr [edx + 24]", "push dword ptr [edx + 20]", "push dword ptr [edx + 16]",
            "mov esi, [edx + 4]", "mov edi, [edx + 8]", "mov ebx, [edx]", "mov edx, [edx + 12]",
            "call ebx",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("edx") block.as_ptr() => hi, inout("eax") eax => lo, inout("ecx") ecx => _,
            clobber_abi("C"),
        );
    }
    (lo, hi)
}

unsafe fn thiscall(f: usize, this: usize, args: [u32; 3]) -> u32 {
    unsafe { game_call(f, 0, this as u32, 0, 0, args) }
}

unsafe fn vslot(obj: usize, slot: usize) -> usize {
    unsafe { rd(rd(obj) as usize + slot * 4) as usize }
}

/// Resolve `name` (a full file name); returns its size, or None.
unsafe fn resolve_size(name: *const u8) -> Option<u32> {
    unsafe {
        let mut loc = [0u8; 0x110];
        let ok = game_call(FILE_RESOLVE, loc.as_mut_ptr() as u32, 0, 0, 0, [name as u32, 0, 0]);
        (ok as u8 != 0).then(|| u32::from_le_bytes(loc[0x108..0x10C].try_into().unwrap()))
    }
}

/// `name` + extension into a 0x40-byte buffer.
unsafe fn with_ext(name: u32, ext: u32) -> [u8; 0x40] {
    unsafe {
        let mut buf = [0u8; 0x40];
        game_call(PATH_CHANGE_EXTENSION, 0, name, 0, 0, [buf.as_mut_ptr() as u32, 0x40, ext]);
        buf
    }
}

/// A read node from the pool with the fields every extras read sets.
unsafe fn extras_node(file: u32, ext: u32, dest: u32, kind: u32, size: u32) -> usize {
    unsafe {
        let node = call(READ_NODE_ALLOC, &[]) as usize;
        wr(node + 8, file);
        wr(node + 0x0C, ext);
        *((node + 0x28) as *mut u8) = 0x30;
        wr(node + 0x20, dest);
        wr(node + 0x2C, kind);
        wr(node + 0x24, size);
        node
    }
}

/// Append circular list `b` (head node) to the tail of the list whose head pointer is at `head`.
unsafe fn list_splice(head: usize, b: usize) {
    unsafe {
        let a = rd(head) as usize;
        if a == 0 {
            wr(head, b as u32);
        } else if b != 0 {
            let (a_tail, b_tail) = (rd(a) as usize, rd(b) as usize);
            wr(a_tail + 4, b as u32);
            wr(b_tail + 4, a as u32);
            wr(b, a_tail as u32);
            wr(a, b_tail as u32);
        }
    }
}

/// Step 0 of the chunk extras, for one object name: find its object template (by
/// name hash) and its streamed mesh cache record (on either pool's active list),
/// and queue up to 4 parts. A record that is already loaded only gets this chunk
/// added (`smesh_cache_record_add_chunk`) and nothing is read; without a record
/// one is taken (`smesh_cache_add_ref`). Each part is read into the record's mesh
/// object (+0x10C; its peg object at +0x54 for the two GPU parts) if the file
/// exists and fits. A missing or too-big mesh (parts 0/1) drops the whole object;
/// missing pegs are skipped. Each part repeats the lookup, as the original does.
/// Returns the object's node list (head), or 0.
unsafe fn extras_object(info: usize, name: u32) -> usize {
    unsafe {
        let chunk = *((info + 0x772) as *const u16) as u32;
        let mut nodes = 0usize;
        for part in 0..4usize {
            let mut hash = 0u32;
            game_call(STRING_HASH, 0, name, 0, 0, [&mut hash as *mut u32 as u32, 0, 0]);
            let mut entry = 0usize;
            for k in 0..rd(OBJECT_TEMPLATES_COUNT) as usize {
                let e = OBJECT_TEMPLATES + k * 0x90;
                if rd(e + 4) == hash {
                    entry = e;
                    break;
                }
            }
            let mut rec = 0usize;
            'lists: for list in SMESH_CACHE_ACTIVE {
                let head = rd(list) as usize;
                let mut o = head;
                while o != 0 {
                    if rd(o + 0x110) as usize == entry {
                        rec = o;
                        break 'lists;
                    }
                    o = rd(o) as usize;
                    if o == head {
                        break;
                    }
                }
            }
            if rec != 0 && *((rec + 0x122) as *const u8) != 0 {
                game_call(SMESH_CACHE_RECORD_ADD_CHUNK, 0, 0, 0, rec as u32, [chunk, 0, 0]);
                rec = 0;
            } else if rec == 0 {
                rec = game_call(SMESH_CACHE_ADD_REF, 0, 0, entry as u32, 0, [chunk, 0, 0]) as usize;
            }
            if rec == 0 {
                game_call(READ_NODES_FREE_LIST, nodes as u32, 0, 0, 0, [0, 0, 0]);
                return 0;
            }
            let ext = rd(PART_EXTS + part * 4);
            let file = rd(entry + 8);
            let buf = with_ext(file, ext);
            let size = resolve_size(buf.as_ptr()).filter(|&s| s != 0);
            let mut pool = rd(rec + 0x10C) as usize;
            if part == 1 || part == 3 {
                pool = rd(pool + 0x54) as usize;
            }
            match size {
                Some(size) if size <= thiscall(vslot(pool, 3), pool, [0; 3]) => {
                    let dest = thiscall(vslot(pool, 7), pool, [size, rd(PART_ALIGNS + part * 4), 0]);
                    let node = extras_node(file, ext, dest, 0, size);
                    wr(node + 0x30, name);
                    list_append(&mut nodes as *mut usize as usize, node);
                }
                _ if part < 2 => {
                    game_call(READ_NODES_FREE_LIST, nodes as u32, 0, 0, 0, [0, 0, 0]);
                    return 0;
                }
                _ => {}
            }
        }
        nodes
    }
}

/// `streamman_start_chunk_extras` (0xA7E100): like start_load, for type 4. Read
/// steps: 0 = objects (see `extras_object`), 1 = `.anim_pc` files into the step's
/// pool (+0x220), 2 = one file into the scratch pool (reset first), NUL-terminated.
/// Steps are filtered like start_load's; read priority 0x30. If nothing was
/// queued the batch goes back on the free list and the slot stays requested
/// (it is retried on the next pump); there is no abort path.
unsafe fn start_extras(s: usize) {
    unsafe {
        let info = rd(s + 0x10) as usize;
        let batch = rd(BATCHES_FREE) as usize;
        if rd(batch + 4) as usize == batch {
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
        }
        for o in [0, 4, 8, 0x0C, 0x10, 0x14] {
            wr(batch + o, 0);
        }
        *((batch + 0x18) as *mut u16) = 0xFFFF;

        let mut i = 0usize;
        while (i as u32) < (rd(info + 0x20) >> 10) & 0x3FF {
            let step = rd(info + 0x14) as usize + i * 0x244;
            let file = rd(step);
            let filter = rd(info + 0x10);
            let wanted = if filter != 0 { call(filter as usize, &[file]) as u8 != 0 } else { file != 0 };
            if wanted {
                match i {
                    1 => {
                        for j in 0..(rd(step + 0x208) as i32).max(0) as usize {
                            let name = rd(step + 8 + j * 4);
                            let buf = with_ext(name, ANIM_EXT as u32);
                            let Some(size) = resolve_size(buf.as_ptr()).filter(|&s| s != 0) else {
                                continue;
                            };
                            let pool = rd(step + 0x220) as usize;
                            if size > thiscall(vslot(pool, 3), pool, [0; 3]) {
                                continue;
                            }
                            let align = *((step + 0x214) as *const u16) as u32;
                            let dest = thiscall(vslot(pool, 7), pool, [size, align, 0]);
                            let node = extras_node(name, ANIM_EXT as u32, dest, 1, size);
                            list_append(batch + 0x0C, node);
                        }
                    }
                    2 => {
                        let mut size = 0u32;
                        if file_get_size(file, &mut size) && size != 0 {
                            thiscall(vslot(SCRATCH_POOL, 13), SCRATCH_POOL, [0; 3]);
                            if size <= rd(SCRATCH_POOL + 0x3C).wrapping_sub(rd(SCRATCH_POOL + 0x44)) {
                                let align = *((step + 0x214) as *const u16) as u32;
                                let dest = thiscall(VIRTUAL_MEMPOOL_ALLOC, SCRATCH_POOL, [size + 1, align, 0]);
                                let node = extras_node(file, 0, dest, 2, size);
                                *((dest as usize + size as usize) as *mut u8) = 0;
                                list_append(batch + 0x0C, node);
                            }
                        }
                    }
                    0 => {
                        for j in 0..(rd(step + 0x208) as i32).max(0) as usize {
                            let nodes = extras_object(info, rd(step + 8 + j * 4));
                            list_splice(batch + 0x0C, nodes);
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
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
                let desc =
                    [rd(n + 8), rd(n + 0x0C), 0, rd(n + 0x24), rd(n + 0x20), *((n + 0x28) as *const u8) as u32, 0, 0];
                let (lo, hi) = io_submit(&desc);
                wr(n + 0x10, lo);
                wr(n + 0x14, hi);
                n = rd(n + 4) as usize;
                if n == rd(batch + 0x0C) as usize {
                    break;
                }
            }
            wr(s + 4, 2);
        } else {
            list_append(BATCHES_FREE, batch);
        }
    }
}

extern "C" fn start_extras_hook(s: u32) {
    CALLS[START_EXTRAS].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = s as usize;
        let e = format!("start_extras {}", describe(s));
        if PUMP_RUST {
            start_extras(s);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(START_EXTRAS));
            f(s as u32);
        }
        // Reads per kind (+0x2C: 0 object part, 1 anim, 2 file) with their bytes.
        let mut kinds = [(0u32, 0u32); 3];
        if rd(s + 4) == 2 {
            let head = rd(rd(s + 0x20) as usize + 0x0C) as usize;
            let mut n = head;
            while n != 0 {
                let k = (rd(n + 0x2C) as usize).min(2);
                kinds[k].0 += 1;
                kinds[k].1 = kinds[k].1.wrapping_add(rd(n + 0x24));
                n = rd(n + 4) as usize;
                if n == head {
                    break;
                }
            }
        }
        event(format!("{e} -> s{} parts {:?} anims {:?} file {:?}", rd(s + 4), kinds[0], kinds[1], kinds[2]));
    }
}

// --- Chunk extras poll ----------------------------------------------------------

const CHUNK_POOL_INDEX: usize = 0x004A_A9F0; // ESI = chunk pool -> index (pools at 0x2771894, stride 0x58)
const OBJECT_ANIM_ADD_LOADED: usize = 0x006E_9BD0; // ESI = name, cdecl (data, size, pool index)
const CHUNK_EXTRAS_PARSE_NODES: usize = 0x00A7_E7A0; // cdecl (node, chunk id)
const OBJECT_TEMPLATE_FIND: usize = 0x0093_1060; // ECX = name -> template
const SMESH_CACHE_FIND: usize = 0x006E_6310; // EDX = template -> record
const CRT_STRICMP: usize = 0x00C9_9030; // cdecl (a, b)
const SMESH_TABLE_ADD_STREAMED: usize = 0x006E_5960; // ECX = name, EDX = record names (+8), cdecl (smesh, g_smesh)
const PEG_REGISTER_LOADED: usize = 0x00C0_8B80; // ESI = peg data, ECX = name, cdecl (peg size, g_peg data)

/// `streamman_poll_chunk_extras` (0xA7E900): check_batch for type 4, with no
/// callbacks. Per pending node: queued stops; being read stops (after a debug-only
/// name format, left out); done is handled by kind (+0x2C) and the node moves to
/// the finished list; failed just moves; anything else (cancelling) is skipped in
/// place. Kinds: 0 a mesh/peg part, stored on its mesh cache record (unless the
/// record is already loaded) by extension; when the raw next node belongs to
/// another object the record's mesh is registered (`smesh_table_add_streamed`),
/// then its g_peg if there is one, and it is marked loaded. 1 an anim
/// (`object_anim_add_loaded`). 2 the node file (`parse_nodes`). With nothing
/// pending: free the finished nodes, clear the slot's batch, batch to the free
/// list, then `finish_current`.
unsafe fn poll_extras(batch: usize) {
    unsafe {
        let s = rd(batch + 0x14) as usize;
        let info = rd(s + 0x10) as usize;
        let mut node = rd(batch + 0x0C) as usize;
        while node != 0 {
            let next = rd(node + 4) as usize;
            let next = if next != rd(batch + 0x0C) as usize { next } else { 0 };
            match call(IO_STATUS, &[rd(node + 0x10), rd(node + 0x14)]) {
                1 | 2 => break,
                3 => {
                    match rd(node + 0x2C) {
                        0 => extras_part_done(node),
                        1 => {
                            let index = game_call(CHUNK_POOL_INDEX, 0, 0, rd(info + 0x778), 0, [0; 3]);
                            game_call(
                                OBJECT_ANIM_ADD_LOADED,
                                0,
                                0,
                                rd(node + 8),
                                0,
                                [rd(node + 0x20), rd(node + 0x24), index],
                            );
                        }
                        2 => {
                            call(CHUNK_EXTRAS_PARSE_NODES, &[node as u32, *((info + 0x772) as *const u16) as u32]);
                        }
                        _ => {}
                    }
                    list_remove(batch + 0x0C, node);
                    list_append(batch + 0x10, node);
                }
                5 => {
                    list_remove(batch + 0x0C, node);
                    list_append(batch + 0x10, node);
                }
                _ => {}
            }
            node = next;
        }
        if rd(batch + 0x0C) != 0 {
            return;
        }
        let mut node = rd(batch + 0x10) as usize;
        while node != 0 {
            let next = rd(node + 4) as usize;
            let next = if next != rd(batch + 0x10) as usize { next } else { 0 };
            let (req, seq) = (rd(node + 0x10) as usize, rd(node + 0x14));
            if req == 0 || seq != rd(req) {
                wr(IO_LAST_ERROR, rd(rd(IO_LAST_ERROR_SRC) as usize));
            }
            io_free(req, seq);
            list_remove(batch + 0x10, node);
            wr(node + 0x10, 0);
            wr(node + 0x14, 0);
            wr(READ_NODE_POOL_COUNT, rd(READ_NODE_POOL_COUNT).wrapping_add(1));
            list_append(READ_NODE_POOL, node);
            node = next;
        }
        wr(s + 0x20, 0);
        list_remove(BATCHES_ACTIVE, batch);
        list_append(BATCHES_FREE, batch);
        finish_current_hook(); // through the hook, for the event log
    }
}

/// A finished mesh/peg part (kind 0): see `poll_extras`.
unsafe fn extras_part_done(node: usize) {
    unsafe {
        let template = game_call(OBJECT_TEMPLATE_FIND, 0, rd(node + 0x30), 0, 0, [0; 3]);
        let rec = game_call_edx(SMESH_CACHE_FIND, 0, 0, template, 0, 0, [0; 3]).0 as usize;
        if *((rec + 0x122) as *const u8) != 0 {
            return;
        }
        let ext = rd(node + 0x0C);
        let (data, size) = (rd(node + 0x20), rd(node + 0x24));
        let is = |k: usize| call(CRT_STRICMP, &[ext, rd(PART_EXTS + k * 4)]) == 0;
        if is(0) {
            wr(rec + 0x124, data);
        } else if is(1) {
            wr(rec + 0x128, data);
        } else if is(2) {
            wr(rec + 0x12C, data);
            wr(rec + 0x130, size);
        } else if is(3) {
            wr(rec + 0x134, data);
            wr(rec + 0x138, size);
        }
        let raw_next = rd(node + 4) as usize;
        if raw_next != 0 && rd(raw_next + 0x30) == rd(node + 0x30) {
            return;
        }
        game_call_edx(
            SMESH_TABLE_ADD_STREAMED,
            0,
            rd(node + 0x30),
            (rec + 8) as u32,
            0,
            0,
            [rd(rec + 0x124), rd(rec + 0x128), 0],
        );
        if rd(rec + 0x134) != 0 {
            game_call(
                PEG_REGISTER_LOADED,
                0,
                (rec + 0x8A) as u32,
                rd(rec + 0x12C),
                0,
                [rd(rec + 0x130), rd(rec + 0x134), 0],
            );
        }
        *((rec + 0x122) as *mut u8) = 1;
    }
}

extern "C" fn poll_extras_hook(batch: u32) {
    CALLS[POLL_EXTRAS].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let s = rd(batch as usize + 0x14) as usize;
        let e = format!("poll_extras {}", describe(s));
        let state = rd(s + 4);
        if PUMP_RUST {
            poll_extras(batch as usize);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(POLL_EXTRAS));
            f(batch);
        }
        if rd(s + 4) != state {
            event(format!("{e} -> s{} current {}", rd(s + 4), (rd(CURRENT) as usize == s) as u8));
        }
    }
}

// --- Sequential (animation) path ---------------------------------------------------

const SUBMIT_SINGLE_READ: usize = 0x00A7_CF50; // EAX = file, ECX = dest, EDI = info+4, cdecl (space left) -> EDX:EAX

/// Advance the info's current step (bits 0-9) by one; returns whether it reached
/// the step count (bits 10-19).
unsafe fn next_step(info: usize) -> bool {
    unsafe {
        let f = rd(info + 0x20);
        let n = (f & !0x3FF) | (f.wrapping_add(1) & 0x3FF);
        wr(info + 0x20, n);
        (n ^ (n >> 10)) & 0x3FF == 0
    }
}

/// `streamman_process_sequential` (0xA7D0C0), the pump's path for type 6
/// (animations): one read at a time, kept at slot +0x18/+0x1C, no batch.
///
/// State 1: find the next step from the current one that passes the filter (a
/// filter must return exactly 1). None left: `finish_current`. Otherwise make it
/// current, run its begin callback (false aborts), align the cursor as start_load
/// does, submit the read (priority 0x70, `submit_single_read`), run the issue
/// callback and go to state 2. A current step already past the count is issued
/// anyway, as in the original.
///
/// State 2: poll the read (`file_get_size` refreshes the step's size at +0x210 on
/// the way). Still running: the progress callback `(info, 0)` (false aborts). Done,
/// cancelled or failed: poll again; unless that says done, the retry callback must
/// return 1, else abort. Then the after callback (false aborts), release the read,
/// advance the cursor, and advance the step: plain steps by one; a multi-part step
/// (u8 +4) only once its part counter +0x20C reaches +0x208, then past following
/// multi-part steps with nothing to read (+0x208 <= 0). Back to state 1 while
/// steps remain (counted against the step count read on entry), else
/// `finish_current`. Two debug-only `streamman_nop` calls are left out.
unsafe fn process_sequential() {
    unsafe {
        let cur = rd(CURRENT) as usize;
        let info = rd(cur + 0x10) as usize;
        let steps = |i: u32| rd(info + 0x14) as usize + i as usize * 0x244;
        let ok = |r: u32| r as u8 != 0;
        match rd(cur + 4) {
            1 => {
                let flags = rd(info + 0x20);
                let (mut i, n) = (flags & 0x3FF, (flags >> 10) & 0x3FF);
                while i < n {
                    let file = rd(steps(i));
                    let filter = rd(info + 0x10);
                    let runnable = if filter != 0 { call(filter as usize, &[file]) as u8 == 1 } else { file != 0 };
                    if runnable {
                        break;
                    }
                    i += 1;
                }
                if i == n {
                    finish_current_hook();
                    return;
                }
                wr(info + 0x20, (rd(info + 0x20) & !0x3FF) | (i & 0x3FF));
                let step = steps(i);
                let begin = rd(step + 0x22C) as usize;
                if begin != 0 && !ok(call(begin, &[info as u32])) {
                    abort_current_hook(0);
                    return;
                }
                if rd(info + 0x20) & 0x10_0000 == 0 {
                    let align = *((step + 0x214) as *const u16) as u32;
                    let r = rd(info + 8) % align;
                    let pad = if r == 0 { 0 } else { align - r };
                    wr(info + 8, rd(info + 8).wrapping_add(pad));
                    wr(info + 0x0C, rd(info + 0x0C).wrapping_sub(pad));
                    wr(step + 0x218, rd(info + 8));
                }
                let (lo, hi) = game_call_edx(
                    SUBMIT_SINGLE_READ,
                    rd(step),
                    rd(info + 8),
                    0,
                    0,
                    rd(info + 4),
                    [rd(info + 0x0C), 0, 0],
                );
                wr(cur + 0x18, lo);
                wr(cur + 0x1C, hi);
                let issue = rd(step + 0x234) as usize;
                if issue != 0 {
                    call(issue, &[info as u32]);
                }
                wr(cur + 4, 2);
            }
            2 => {
                let flags = rd(info + 0x20);
                let n = (flags >> 10) & 0x3FF;
                let step = steps(flags & 0x3FF);
                let status = call(IO_STATUS, &[rd(cur + 0x18), rd(cur + 0x1C)]);
                file_get_size(rd(step), &mut *((step + 0x210) as *mut u32));
                if !matches!(status, 3..=5) {
                    let progress = rd(step + 0x230) as usize;
                    if progress != 0 && !ok(call(progress, &[info as u32, 0])) {
                        abort_current_hook(0);
                    }
                    return;
                }
                if call(IO_STATUS, &[rd(cur + 0x18), rd(cur + 0x1C)]) != 3 {
                    let retry = rd(step + 0x240) as usize;
                    if retry == 0 || call(retry, &[info as u32]) as u8 != 1 {
                        abort_current_hook(0);
                        return;
                    }
                }
                let after = rd(step + 0x238) as usize;
                if after != 0 && !ok(call(after, &[info as u32])) {
                    abort_current_hook(0);
                    return;
                }
                call(READ_RELEASE, &[rd(cur + 0x18), rd(cur + 0x1C), 0]);
                wr(cur + 0x18, 0);
                wr(cur + 0x1C, 0);
                if rd(info + 0x20) & 0x10_0000 == 0 {
                    let size = rd(step + 0x210);
                    wr(info + 8, rd(info + 8).wrapping_add(size));
                    wr(info + 0x0C, rd(info + 0x0C).wrapping_sub(size));
                }
                if *((step + 4) as *const u8) == 0 {
                    next_step(info);
                } else if rd(step + 0x20C) as i32 >= rd(step + 0x208) as i32 && !next_step(info) {
                    loop {
                        let s2 = steps(rd(info + 0x20) & 0x3FF);
                        if *((s2 + 4) as *const u8) == 0 || rd(s2 + 0x208) as i32 > 0 || next_step(info) {
                            break;
                        }
                    }
                }
                if rd(info + 0x20) & 0x3FF == n {
                    finish_current_hook();
                } else {
                    wr(cur + 4, 1);
                }
            }
            _ => {}
        }
    }
}

extern "C" fn process_sequential_hook() {
    CALLS[PROCESS_SEQ].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let cur = rd(CURRENT) as usize;
        let e = format!("sequential {}", describe(cur));
        let state = rd(cur + 4);
        if PUMP_RUST {
            process_sequential();
        } else {
            let f: extern "C" fn() = std::mem::transmute(orig(PROCESS_SEQ));
            f();
        }
        if rd(cur + 4) != state {
            let info = rd(cur + 0x10) as usize;
            event(format!(
                "{e} -> s{} step {} current {}",
                rd(cur + 4),
                rd(info + 0x20) & 0x3FF,
                (rd(CURRENT) as usize == cur) as u8
            ));
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
                process_sequential_hook(); // through the hook, for the event log
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

// --- Request side -----------------------------------------------------------------
//
// The typed requests fill a slot's loading info and read steps, set the slot to
// requested (state 1) and (re)queue it. In `PUMP_RUST` mode they run in Rust;
// otherwise the originals run. Either way the hooks log the same request events.

const PACKFILE_FIND_ENTRY: usize = 0x00C0_AD80; // ECX = name -> entry
const INDEXED_FILE_LOOKUP: usize = 0x00C1_DA80; // ECX = name, cdecl (0, 0, u32 *size) -> AL
const DISABLE_INDEXED_LOOKUP: usize = 0x0252_72DC; // u8
const CRT_STRNCPY: usize = 0x00C9_90F0;
const EXT_G_PEG: usize = 0x00DD_A764; // ".g_peg_pc"
const EXT_G_MORPH: usize = 0x00E0_DA70; // ".g_morph_pc"
const CHARACTER_MODE_CHECK: usize = 0x006C_9090; // ECX = name -> AL
const VEHICLE_REQUESTED: usize = 0x0048_40A0; // EDI = *(object + 4)
const WORLD_CHUNKS: usize = 0x00F7_AAE8; // [0x866] chunk entry*
const WORLD_CHUNK_RELEASE: usize = 0x004A_9DA0; // cdecl (entry, -1)
const CHUNK_TYPES: usize = 0x00E8_7B84; // [3] the chunk slot types (0, 1, 4)
const SLOT_COUNTS: usize = 0x00E8_7BB0; // [10] slots per type
const UNLOAD_FLAGGED_GEOM_FLAG: usize = 0x00E8_7B4F; // u8

/// Call a game function with EAX/ECX/EDX/ESI set and any number of stack
/// arguments (cdecl or callee-clean alike: ESP is restored from EBP).
unsafe fn game_call_n(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, args: &[u32]) -> (u32, u32) {
    let block: [u32; 7] = [f as u32, eax, ecx, edx, esi, args.as_ptr() as u32, args.len() as u32];
    let (lo, hi): (u32, u32);
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "mov ebx, edx",
            "mov ecx, [ebx + 24]",
            "mov esi, [ebx + 20]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov eax, [ebx + 4]", "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 16]",
            "call dword ptr [ebx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("edx") block.as_ptr() => hi, lateout("eax") lo, lateout("ecx") _,
            clobber_abi("C"),
        );
    }
    (lo, hi)
}

unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}

/// "No handle": the originals store 0 and 0xFFFF as two words.
unsafe fn out_none(out: usize) {
    unsafe {
        *(out as *mut u16) = 0;
        *((out + 2) as *mut u16) = 0xFFFF;
    }
}

/// Remove the slot from the request queue if it is queued, then push it (again).
unsafe fn requeue(s: usize) {
    unsafe {
        if rd(s + 0x14) != u32::MAX {
            q_remove(REQUEST_QUEUE, rd(s + 0x14));
        }
        wr(s + 0x14, q_push(REQUEST_QUEUE, s as u32) as u32);
    }
}

/// `file_exists` (0xA7D750, ESI = name -> AL): in a mounted packfile, or (unless
/// disabled) found with a non-zero size by the indexed lookup. Returns the raw AL.
unsafe fn file_exists(name: u32) -> u8 {
    unsafe {
        if game_call(PACKFILE_FIND_ENTRY, 0, name, 0, 0, [0; 3]) != 0 {
            return 1;
        }
        if rb(DISABLE_INDEXED_LOOKUP) != 0 {
            return 0;
        }
        let mut size = 0u32;
        let r = game_call(INDEXED_FILE_LOOKUP, 0, name, 0, 0, [0, 0, &mut size as *mut u32 as u32]) as u8;
        if size != 0 { r } else { 0 }
    }
}

/// `streamman_step_init` (0xA7F770; ESI = step, EAX = file, CX = alignment;
/// stdcall (begin, issue, progress, after)): set up one read step.
unsafe fn step_init(step: usize, file: u32, align: u16, begin: u32, issue: u32, progress: u32, after: u32) {
    unsafe {
        wr(step + 0x21C, rd(step + 0x21C) & !1);
        wr(step, file);
        wr(step + 0x230, progress);
        *((step + 0x214) as *mut u16) = align;
        wr(step + 0x22C, begin);
        wr(step + 0x210, 0);
        wr(step + 0x218, 0);
        wr(step + 0x234, issue);
        wr(step + 0x238, after);
        wb(step + 4, 0);
        wr(step + 0x208, 0);
        wr(step + 0x20C, 0);
        std::ptr::write_bytes((step + 8) as *mut u8, 0, 0x200);
        wr(step + 0x21C, rd(step + 0x21C) & !2);
        wr(step + 0x240, 0);
        wr(step + 0x23C, 0);
    }
}

unsafe fn step_empty(step: usize, align: u16) {
    unsafe { step_init(step, 0, align, 0, 0, 0, 0) }
}

/// `path_change_extension(src, out, 0x41, ext)`.
unsafe fn change_extension(src: u32, out: usize, ext: usize) {
    unsafe {
        game_call(PATH_CHANGE_EXTENSION, 0, src, 0, 0, [out as u32, 0x41, ext as u32]);
    }
}

/// A single-slot type's slot, or None (after writing "no handle") if it is busy.
unsafe fn single_slot(t: usize, out: usize) -> Option<usize> {
    unsafe {
        let s = rd(SLOT_ARRAYS + t * 4) as usize;
        if rd(s + 4) != 0 {
            out_none(out);
            return None;
        }
        Some(s)
    }
}

/// Mark a slot requested: state 1, +8/+0xC cleared; then the info's current step
/// and bits 20-22 are cleared (the step count is preset by the info).
unsafe fn claim(s: usize) -> usize {
    unsafe {
        wr(s + 8, 0);
        wr(s + 0x0C, 0);
        wr(s + 4, 1);
        let info = rd(s + 0x10) as usize;
        wr(info + 0x20, rd(info + 0x20) & 0xFF8F_FC00);
        info
    }
}

/// `streamman_request_animation` (0xA88240; EDX = dest; cdecl (out, name, size,
/// object) -> out): type 6, one step, completion 0xA87C50.
unsafe fn request_animation(dest: u32, out: usize, name: u32, size: u32, object: u32) {
    unsafe {
        let Some(s) = single_slot(6, out) else { return };
        let info = claim(s);
        wr(info + 8, dest);
        wr(info + 0x0C, size);
        wr(info + 4, 0);
        wr(info + 0x10, 0);
        wr(info + 0x18, object);
        wr(info + 0x1C, 0x00A8_7C50);
        step_init(info + 0x24, name, 1, 0, 0, 0, 0);
        requeue(s);
        wr(out, 0x6_0000);
    }
}

/// `streamman_request_character` (0xA7FC30; EAX = dest A, EDX = size A; cdecl
/// (out, a1..a10) -> out): type 3, 10 steps into buffers A and B (bit 0 of +0x21C
/// selects B), completion `character_on_loaded`.
unsafe fn request_character(dest_a: u32, size_a: u32, a: &[u32; 11]) {
    unsafe {
        let out = a[0] as usize;
        let Some(s) = single_slot(3, out) else { return };
        let info = claim(s);
        wr(info + 0x0C, size_a);
        wr(info + 0x1758, size_a);
        wr(info + 4, 0);
        wr(info + 8, dest_a);
        wr(info + 0x10, 0);
        wr(info + 0x18, a[10]);
        wr(info + 0x1C, 0x00A7_FA40);
        wr(info + 0x1750, dest_a);
        wr(info + 0x1754, a[8]);
        wr(info + 0x175C, a[9]);
        wr(info + 0x1760, dest_a);
        wr(info + 0x1764, a[8]);
        let cond = (rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) != 0) || rb(0x0252_7D15) != 0;
        let mode = cond && game_call(CHARACTER_MODE_CHECK, 0, a[1], 0, 0, [0; 3]) as u8 != 0;
        wb(info + 0x1768, mode as u8);
        let (begin, after) = (0x00A7_F840, 0x00A7_F9C0);
        let step = |k: usize| info + 0x24 + k * 0x244;
        let full = |k: usize, file: u32, align: u16| step_init(step(k), file, align, begin, 0, 0, after);
        let optional = |k: usize, file: u32, align: u16| {
            if file_exists(file) == 1 {
                full(k, file, align);
                wr(step(k) + 0x21C, rd(step(k) + 0x21C) | 1);
            } else {
                step_empty(step(k), 0);
            }
        };
        full(0, a[1], 0x40);
        optional(1, a[2], 0x1000);
        full(2, a[4], 0x10);
        if a[4] != 0 {
            file_get_size(a[4], &mut *((step(2) + 0x210) as *mut u32));
        }
        change_extension(a[3], info + 0x170D, EXT_G_MORPH);
        optional(3, (info + 0x170D) as u32, 0x10);
        full(4, a[3], 4);
        change_extension(a[3], info + 0x16CC, EXT_G_PEG);
        optional(5, (info + 0x16CC) as u32, 0x40);
        full(6, a[5], 0x10);
        full(7, a[7], 4);
        full(8, a[6], 4);
        step_empty(step(9), 0);
        requeue(s);
        wr(out, 0x3_0000);
    }
}

/// `streamman_request_vehicle` (0xA87F80; EDX = size A, ECX = dest B; cdecl (out,
/// files[11], dest A, value, object) -> out): type 2, 11 steps, filter 0xA87AF0,
/// completion `vehicle_on_loaded`. Steps 4, 6, 8 and 10 only if the file exists.
unsafe fn request_vehicle(size_a: u32, dest_b: u32, a: &[u32; 5]) {
    unsafe {
        let (out, files, dest_a, value, object) = (a[0] as usize, a[1] as usize, a[2], a[3], a[4] as usize);
        let Some(s) = single_slot(2, out) else { return };
        let info = claim(s);
        wr(info + 4, 0);
        wr(info + 0x0C, size_a);
        wr(info + 0x1918, size_a);
        wr(info + 8, dest_a);
        wr(info + 0x18, object as u32);
        wr(info + 0x1910, dest_a);
        wr(info + 0x1914, dest_b);
        wr(info + 0x1920, dest_a);
        wr(info + 0x1924, dest_b);
        wr(info + 0x10, 0x00A8_7AF0);
        wr(info + 0x1C, 0x00A8_7BF0);
        wr(info + 0x191C, value);
        let (begin, after) = (0x00A8_7B10, 0x00A8_7BB0);
        let f = |i: usize| rd(files + i * 4);
        let step = |k: usize| info + 0x24 + k * 0x244;
        let full = |k: usize, align: u16| step_init(step(k), f(k), align, begin, 0, 0, after);
        let optional = |k: usize, align: u16| -> bool {
            if file_exists(f(k)) != 0 {
                full(k, align);
                true
            } else {
                step_empty(step(k), align);
                false
            }
        };
        full(0, 0x40);
        full(1, 0x1000);
        full(9, 4);
        full(2, 4);
        full(3, 0x20);
        optional(4, 0x20);
        full(5, 0x20);
        optional(6, 0x20);
        full(7, 0x20);
        optional(8, 0x20);
        if optional(10, 0x40) {
            let bit = !(rd(rd(object + 8) as usize + 0x5B0) >> 14) & 1;
            wr(step(10) + 0x21C, (rd(step(10) + 0x21C) & !1) | bit);
        }
        requeue(s);
        game_call(VEHICLE_REQUESTED, 0, 0, 0, rd(object + 4), [0; 3]);
        wr(out, 0x2_0000);
    }
}

/// `streamman_request_chunk_geom` (0xA85140; EAX = request; cdecl (out) -> out):
/// always slot 0 of type 0, state not checked. 2 steps (chunk_pc and, if it
/// exists, g_chunk_pc), no destination accounting; completion
/// `chunk_geom_on_loaded`. A reservation of -1 resets the pool's GPU part.
unsafe fn request_geom(req: usize, out: usize) {
    unsafe {
        if file_exists(rd(req)) == 0 {
            out_none(out);
            return;
        }
        let s = rd(SLOT_ARRAYS) as usize;
        let info = rd(s + 0x10) as usize;
        wr(info + 0x20, (rd(info + 0x20) & 0xFF90_0800) | 0x10_0800);
        *((info + 0x52E) as *mut u16) = *((req + 0x0C) as *const u16);
        wb(info + 0x541, rb(req + 0x14));
        wb(info + 0x540, rb(req + 0x15));
        let (pool, reservation) = (rd(req + 0x10) as usize, rd(req + 0x18));
        for o in [4, 8, 0x0C, 0x10, 0x18] {
            wr(info + o, 0);
        }
        wr(info + 0x1C, 0x00A8_4FF0);
        wr(info + 0x534, pool as u32);
        wr(info + 0x53C, reservation);
        wr(info + 0x530, 0);
        wb(info + 0x542, rb(req + 0x16));
        wb(info + 0x543, rb(req + 0x1C));
        if reservation == u32::MAX {
            wr(pool + 0x48, rd(pool + 0x44));
            let gpu = rd(rd(info + 0x534) as usize + 0x54) as usize;
            thiscall(vslot(gpu, 12), gpu, [0; 3]);
        }
        call(CRT_STRNCPY, &[(info + 0x4AC) as u32, rd(req), 0x41]);
        wb(info + 0x4EC, 0);
        call(CRT_STRNCPY, &[(info + 0x4ED) as u32, rd(req + 4), 0x41]);
        wb(info + 0x52D, 0);
        let steps = rd(info + 0x14) as usize;
        step_init(steps, (info + 0x4AC) as u32, 0x10, 0x00A8_4E60, 0x00A8_4F80, 0, 0x00A8_4F90);
        if file_exists((info + 0x4ED) as u32) != 0 {
            step_init(steps + 0x244, (info + 0x4ED) as u32, 0x10, 0x00A8_4F00, 0, 0, 0x00A8_4FC0);
        } else {
            wr(info + 0x20, (rd(info + 0x20) & 0xFFF0_07FF) | 0x400);
            wr(steps + 0x454, 0);
            wr(steps + 0x45C, 0);
        }
        if rb(info + 0x540) != 0 {
            wr(steps + 0x21C, rd(steps + 0x21C) | 2);
            wr(steps + 0x460, rd(steps + 0x460) | 2);
        }
        wr(s + 4, 1);
        wr(s + 8, 0);
        wr(s + 0x0C, 0);
        requeue(s);
        wr(out, 0);
    }
}

/// `streamman_request_chunk_peg` (0xA85890; cdecl (out, req {name, u16 id, pool,
/// value, u8 wait}) -> out): the first idle type-1 slot; steps .peg_pc and
/// .g_peg_pc (both with the retry callback), completion `chunk_peg_on_loaded`.
unsafe fn request_peg(out: usize, req: usize) {
    unsafe {
        let name = rd(req);
        if file_exists(name) == 0 {
            out_none(out);
            return;
        }
        let Some(i) = (0..rd(SLOT_COUNTS + 4)).find(|&i| {
            let s = slot(0x1_0000 | i);
            s == 0 || matches!(rd(s + 4), 0 | 3 | 4)
        }) else {
            out_none(out);
            return;
        };
        let s = rd(SLOT_ARRAYS + 4) as usize + i as usize * SLOT_SIZE;
        let info = rd(s + 0x10) as usize;
        for o in [4, 8, 0x0C, 0x10, 0x18] {
            wr(info + o, 0);
        }
        wr(info + 0x1C, 0x00A8_57A0);
        wr(info + 0x20, (rd(info + 0x20) & 0xFF9F_FC00) | 0x10_0000);
        wr(info + 0x53C, rd(req + 8));
        wr(info + 0x534, 0);
        wr(info + 0x538, 0);
        wr(info + 0x530, 0);
        *((info + 0x52E) as *mut u16) = *((req + 4) as *const u16);
        wb(info + 0x540, rb(req + 0x10));
        call(CRT_STRNCPY, &[(info + 0x4AC) as u32, name, 0x41]);
        wb(info + 0x4EC, 0);
        let steps = rd(info + 0x14) as usize;
        step_init(steps, (info + 0x4AC) as u32, 4, 0x00A8_5450, 0x00A8_5660, 0, 0x00A8_56B0);
        wr(info + 4, rd(req + 0x0C));
        wr(steps + 0x240, 0x00A8_56E0);
        change_extension((info + 0x4AC) as u32, info + 0x4ED, EXT_G_PEG);
        step_init(steps + 0x244, (info + 0x4ED) as u32, 0x40, 0x00A8_5510, 0, 0x00A8_5600, 0);
        wr(steps + 0x244 + 0x240, 0x00A8_56E0);
        wr(steps + 0x21C, rd(steps + 0x21C) | 2);
        wr(steps + 0x460, rd(steps + 0x460) | 2);
        wr(s + 4, 1);
        wr(s + 8, 0);
        wr(s + 0x0C, 0);
        requeue(s);
        wr(out, 0x1_0000 | i);
    }
}

/// Busy chunk slots of the three chunk types, as handles.
unsafe fn busy_chunk_slots() -> Vec<u32> {
    unsafe {
        let mut v = Vec::new();
        for k in 0..3 {
            let t = rd(CHUNK_TYPES + k * 4);
            for i in 0..rd(SLOT_COUNTS + t as usize * 4) {
                let h = t << 16 | (i & 0xFFFF);
                let s = slot(h);
                if s != 0 && !matches!(rd(s + 4), 0 | 3 | 4) {
                    v.push(h);
                }
            }
        }
        v
    }
}

/// `streamman_unload_chunk_pegs_extras` (0xA80A40, cdecl (u16 chunk id)): unload
/// the busy peg and extras slots loading that chunk.
unsafe fn unload_chunk_pegs_extras(id: u16) {
    unsafe {
        for (t, field) in [(1u32, 0x52E), (4u32, 0x772)] {
            for i in 0..rd(SLOT_COUNTS + t as usize * 4) {
                let info = rd(rd(SLOT_ARRAYS + t as usize * 4) as usize + i as usize * SLOT_SIZE + 0x10) as usize;
                if *((info + field) as *const u16) != id {
                    continue;
                }
                let h = t << 16 | (i & 0xFFFF);
                let s = slot(h);
                if s != 0 && !matches!(rd(s + 4), 0 | 3 | 4) {
                    unload_hook(h);
                }
            }
        }
    }
}

/// `streamman_cancel_chunk_loads` (0xA80BC0): busy geom slots are unloaded; busy
/// peg/extras slots release their world chunk entry instead.
unsafe fn cancel_chunk_loads() {
    unsafe {
        for k in 0..3 {
            let t = rd(CHUNK_TYPES + k * 4);
            for i in 0..rd(SLOT_COUNTS + t as usize * 4) as usize {
                let h = t << 16 | (i as u32 & 0xFFFF);
                let s = slot(h);
                if s == 0 || matches!(rd(s + 4), 0 | 3 | 4) {
                    continue;
                }
                if t == 0 {
                    unload_hook(h);
                    continue;
                }
                let id = match t {
                    1 => *((0x02FA_45C6 + i * 0x544) as *const u16) as u32,
                    4 => *((0x02FA_5D1A + i * 0x77C) as *const u16) as u32,
                    _ => 0xFFFF,
                };
                if id != 0xFFFF && id < 0x866 {
                    let e = rd(WORLD_CHUNKS + id as usize * 4);
                    if e != 0 {
                        call(WORLD_CHUNK_RELEASE, &[e, u32::MAX]);
                    }
                }
            }
        }
    }
}

/// `streamman_unload_flagged_geom` (0xA80CC0, AL = flag): store the flag, unload
/// every geom slot whose info has +0x542 set (state not checked).
unsafe fn unload_flagged_geom(flag: u8) {
    unsafe {
        wb(UNLOAD_FLAGGED_GEOM_FLAG, flag);
        for i in 0..rd(SLOT_COUNTS) {
            let info = rd(rd(SLOT_ARRAYS) as usize + i as usize * SLOT_SIZE + 0x10) as usize;
            if rb(info + 0x542) != 0 {
                unload_hook(i & 0xFFFF);
            }
        }
    }
}

/// A C string for the event log (at most 64 bytes), or "-".
unsafe fn cstr_lossy(p: u32) -> String {
    unsafe {
        if p == 0 {
            return "-".into();
        }
        let bytes = std::slice::from_raw_parts(p as *const u8, 64);
        let n = bytes.iter().position(|&b| b == 0).unwrap_or(64);
        String::from_utf8_lossy(&bytes[..n]).into_owned()
    }
}

/// A request's outcome for the event log: the handle, slot state and every step
/// (file, alignment, +0x21C flags, which callbacks are set).
unsafe fn describe_request(kind: &str, out: usize) -> String {
    unsafe {
        let h = rd(out);
        if h >> 16 == 0xFFFF {
            return format!("req_{kind} -> none");
        }
        let s = slot(h);
        let info = rd(s + 0x10) as usize;
        let flags = rd(info + 0x20);
        let mut steps = String::new();
        for k in 0..((flags >> 10) & 0x3FF) as usize {
            let st = rd(info + 0x14) as usize + k * 0x244;
            let cbs: u32 = [0x22C, 0x234, 0x230, 0x238, 0x240]
                .iter()
                .enumerate()
                .map(|(b, &o)| ((rd(st + o) != 0) as u32) << b)
                .sum();
            steps += &format!(
                " [{} a{:x} f{} cb{:x}]",
                cstr_lossy(rd(st)),
                *((st + 0x214) as *const u16),
                rd(st + 0x21C) & 3,
                cbs
            );
        }
        format!("req_{kind} -> {h:#x} s{} flags {:#x}{steps}", rd(s + 4), flags & 0xFFFF_FC00)
    }
}

/// Entry for functions with register arguments: hands the hook EAX/ECX/EDX and a
/// pointer to the caller's stack arguments; returns EAX. Caller-clean.
macro_rules! regs_entry {
    ($entry:ident, $hook:ident) => {
        #[unsafe(naked)]
        extern "C" fn $entry() {
            std::arch::naked_asm!(
                "push edx",
                "push ecx",
                "push eax",
                "lea eax, [esp + 16]",
                "push eax",
                "lea eax, [esp + 4]",
                "push eax",
                "call {h}",
                "add esp, 20",
                "ret",
                h = sym $hook,
            )
        }
    };
}

regs_entry!(request_animation_entry, request_animation_hook);
regs_entry!(request_character_entry, request_character_hook);
regs_entry!(request_vehicle_entry, request_vehicle_hook);
regs_entry!(request_geom_entry, request_geom_hook);

/// Read `N` stack arguments.
unsafe fn stack_args<const N: usize>(p: *const u32) -> [u32; N] {
    unsafe { std::array::from_fn(|i| *p.add(i)) }
}

/// regs = [EAX, ECX, EDX].
extern "C" fn request_animation_hook(regs: *const [u32; 3], args: *const u32) -> u32 {
    CALLS[REQUEST_ANIMATION].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = *regs;
        let a: [u32; 4] = stack_args(args);
        if PUMP_RUST {
            request_animation(r[2], a[0] as usize, a[1], a[2], a[3]);
        } else {
            game_call_n(orig(REQUEST_ANIMATION), r[0], r[1], r[2], 0, &a);
        }
        event(describe_request("animation", a[0] as usize));
        a[0]
    }
}

extern "C" fn request_character_hook(regs: *const [u32; 3], args: *const u32) -> u32 {
    CALLS[REQUEST_CHARACTER].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = *regs;
        let a: [u32; 11] = stack_args(args);
        if PUMP_RUST {
            request_character(r[0], r[2], &a);
        } else {
            game_call_n(orig(REQUEST_CHARACTER), r[0], r[1], r[2], 0, &a);
        }
        event(describe_request("character", a[0] as usize));
        a[0]
    }
}

extern "C" fn request_vehicle_hook(regs: *const [u32; 3], args: *const u32) -> u32 {
    CALLS[REQUEST_VEHICLE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = *regs;
        let a: [u32; 5] = stack_args(args);
        if PUMP_RUST {
            request_vehicle(r[2], r[1], &a);
        } else {
            game_call_n(orig(REQUEST_VEHICLE), r[0], r[1], r[2], 0, &a);
        }
        event(describe_request("vehicle", a[0] as usize));
        a[0]
    }
}

extern "C" fn request_geom_hook(regs: *const [u32; 3], args: *const u32) -> u32 {
    CALLS[REQUEST_GEOM].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = *regs;
        let a: [u32; 1] = stack_args(args);
        if PUMP_RUST {
            request_geom(r[0] as usize, a[0] as usize);
        } else {
            game_call_n(orig(REQUEST_GEOM), r[0], r[1], r[2], 0, &a);
        }
        event(describe_request("geom", a[0] as usize));
        a[0]
    }
}

extern "C" fn request_peg_hook(out: u32, req: u32) -> u32 {
    CALLS[REQUEST_PEG].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if PUMP_RUST {
            request_peg(out as usize, req as usize);
        } else {
            let f: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(orig(REQUEST_PEG));
            f(out, req);
        }
        event(describe_request("peg", out as usize));
        out
    }
}

/// `step_init`: ESI = step, EAX = file, CX = alignment, stdcall 4 arguments; ESI kept.
#[unsafe(naked)]
extern "C" fn step_init_entry() {
    std::arch::naked_asm!(
        "push esi",
        "push ecx",
        "push eax",
        "push esi",
        "lea eax, [esp + 20]",
        "push eax",
        "call {h}",
        "add esp, 16",
        "pop esi",
        "ret 16",
        h = sym step_init_hook,
    )
}

/// (args, step, file, align): args = [begin, issue, progress, after].
extern "C" fn step_init_hook(args: *const u32, step: u32, file: u32, align: u32) {
    CALLS[STEP_INIT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let a: [u32; 4] = stack_args(args);
        if PUMP_RUST {
            step_init(step as usize, file, align as u16, a[0], a[1], a[2], a[3]);
        } else {
            game_call_n(orig(STEP_INIT), file, align, 0, step, &a);
        }
    }
}

extern "C" fn unload_chunk_pegs_extras_hook(id: u32) {
    CALLS[UNLOAD_CHUNK_PEGS_EXTRAS].fetch_add(1, Ordering::Relaxed);
    event("unload_chunk_pegs_extras".into());
    unsafe {
        if PUMP_RUST {
            unload_chunk_pegs_extras(id as u16);
        } else {
            let f: extern "C" fn(u32) = std::mem::transmute(orig(UNLOAD_CHUNK_PEGS_EXTRAS));
            f(id);
        }
    }
}

/// `streamman_any_chunk_busy` (0xA80B40) is a pure query: shadow-checked.
extern "C" fn any_chunk_busy_hook() -> u32 {
    CALLS[ANY_CHUNK_BUSY].fetch_add(1, Ordering::Relaxed);
    let rust = unsafe { !busy_chunk_slots().is_empty() } as u32;
    let tramp = orig(ANY_CHUNK_BUSY);
    if tramp != 0 {
        let f: extern "C" fn() -> u32 = unsafe { std::mem::transmute(tramp) };
        let o = f() & 0xFF;
        if o != rust && o != unsafe { !busy_chunk_slots().is_empty() } as u32 {
            mismatch(ANY_CHUNK_BUSY, format!("original {o}, rust {rust}"));
            return o;
        }
    }
    rust
}

extern "C" fn cancel_chunk_loads_hook() {
    CALLS[CANCEL_CHUNK_LOADS].fetch_add(1, Ordering::Relaxed);
    event("cancel_chunk_loads".into());
    unsafe {
        if PUMP_RUST {
            cancel_chunk_loads();
        } else {
            let f: extern "C" fn() = std::mem::transmute(orig(CANCEL_CHUNK_LOADS));
            f();
        }
    }
}

#[unsafe(naked)]
extern "C" fn unload_flagged_geom_entry() {
    std::arch::naked_asm!("push eax", "call {h}", "add esp, 4", "ret", h = sym unload_flagged_geom_hook)
}

extern "C" fn unload_flagged_geom_hook(flag: u32) {
    CALLS[UNLOAD_FLAGGED_GEOM].fetch_add(1, Ordering::Relaxed);
    event(format!("unload_flagged_geom {}", flag as u8));
    unsafe {
        if PUMP_RUST {
            unload_flagged_geom(flag as u8);
        } else {
            game_call_n(orig(UNLOAD_FLAGGED_GEOM), flag, 0, 0, 0, &[]);
        }
    }
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
        CHECK_BATCH => check_batch_hook as extern "C" fn(u32) as usize,
        FINISH => finish_current_hook as extern "C" fn() -> u32 as usize,
        START_EXTRAS => start_extras_hook as extern "C" fn(u32) as usize,
        PROCESS_SEQ => process_sequential_hook as extern "C" fn() as usize,
        POLL_EXTRAS => poll_extras_hook as extern "C" fn(u32) as usize,
        STEP_INIT => step_init_entry as extern "C" fn() as usize,
        UNLOAD_CHUNK_PEGS_EXTRAS => unload_chunk_pegs_extras_hook as extern "C" fn(u32) as usize,
        ANY_CHUNK_BUSY => any_chunk_busy_hook as extern "C" fn() -> u32 as usize,
        CANCEL_CHUNK_LOADS => cancel_chunk_loads_hook as extern "C" fn() as usize,
        UNLOAD_FLAGGED_GEOM => unload_flagged_geom_entry as extern "C" fn() as usize,
        REQUEST_ANIMATION => request_animation_entry as extern "C" fn() as usize,
        REQUEST_CHARACTER => request_character_entry as extern "C" fn() as usize,
        REQUEST_VEHICLE => request_vehicle_entry as extern "C" fn() as usize,
        REQUEST_GEOM => request_geom_entry as extern "C" fn() as usize,
        REQUEST_PEG => request_peg_hook as extern "C" fn(u32, u32) -> u32 as usize,
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
