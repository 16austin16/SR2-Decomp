//! World streamer (`streamman`), docs/analysis/05_streamer.md.
//!
//! Ported so far: the request queue the streamer uses for its requests and
//! read batches (a fixed-capacity, priority-picked list), and the slot-state
//! queries behind resource handles. Each is an inline detour that runs the
//! port and shadow-checks it against the original on every call:
//! - queries: both run, results compared (re-reading once on a difference);
//! - queue methods: snapshot the queue, run the original, rewind, run the port,
//!   compare the whole queue and the result, and keep the original's outcome on
//!   a mismatch.
//!
//! Queue layout (0x18 bytes): +0x00 count, +0x04 capacity, +0x08 node array,
//! +0x0C used list, +0x10 free list, +0x14 compare callback
//! `int(*)(u32 best, u32 candidate)`. Nodes are 12 bytes `{value, next, prev}`
//! on circular doubly linked lists; a node's index is its position in the array.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// `0x2225B3C[type]`: that type's array of 0x28-byte slot records.
const SLOT_ARRAYS: usize = 0x0222_5B3C;
const SLOT_SIZE: usize = 0x28;

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
const COUNT: usize = 10;

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
    log(&format!("replaced {ok}/{COUNT} streamman functions with Rust (inline detours, shadow-checked)"));
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
    format!("streamman live: {} calls, {} mismatches | {}", total.0, total.1, parts.join(", "))
}
