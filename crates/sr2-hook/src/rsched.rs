//! Resource slot scheduler ("rsched"), docs/analysis/05_streamer.md "Resource
//! slot scheduler". Six schedulers at 0x2F9B2E8 decide which characters,
//! vehicles and animations stay resident in a fixed number of memory slots, and
//! feed the streamer one load ("job") at a time.
//!
//! Ported: the whole scheduler (requests, quotas, group pairing, slots), the job
//! layer and the per-frame entry; still original: init/reset and slot_free. Jobs are 0x20-byte
//! records {id, dest, dest2, size, size2, level, next +0x18, prev +0x1C}: at
//! most 50 queued (count 0x2529E4C) in the `streamman_queue` at 0x2F9B2D0, with
//! a free list (0x2529E54) and an active list (0x2529E50). Only one job is
//! current (0x2529E58); its streamer handle is at 0xEB80F8 (0xFFFF0000 = none).
//! Behind `RSCHED_RUST`; both modes log the same job events (`sr2-rsched.log`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// Run the scheduler in Rust (false: the originals, with the same event log).
const RSCHED_RUST: bool = true;

const SCHEDULERS: usize = 0x02F9_B2E8; // 6 x 0xD4
const SCHED_SIZE: usize = 0xD4;
const JOB_QUEUE: usize = 0x02F9_B2D0;
const JOB_COUNT: usize = 0x0252_9E4C;
const JOBS_ACTIVE: usize = 0x0252_9E50;
const JOBS_FREE: usize = 0x0252_9E54;
const JOB_CURRENT: usize = 0x0252_9E58;
const JOB_HANDLE: usize = 0x00EB_80F8;
const MAX_JOBS: u32 = 0x32;
const REGISTRY: usize = 0x0252_9E3C; // [] 0xC-byte entries {flags, name list, extra}
const REGISTRY_COUNT: usize = 0x0252_9E38;
const REGISTRY_DEBUG_NAME: usize = 0x0347_B088;
const STR_EMPTY: usize = 0x00E4_41CC;
const SLOT_ARRAYS: usize = 0x0222_5B3C;

const Q_PUSH: usize = 0x00A7_2C70; // ECX = queue, stack (value) -> index
const Q_REMOVE: usize = 0x00A7_2D10; // EAX = index, EDX = queue -> value
const Q_POP_BEST: usize = 0x00A7_2DA0; // stdcall (queue) -> value
const STREAMMAN_IS_IDLE: usize = 0x00A7_D560; // cdecl (handle) -> AL
const JOB_BEGIN_SLOT: usize = 0x00A7_5F00; // EAX = id, ECX = scheduler
const JOB_REQUEST_CHARACTER: usize = 0x00A7_8030; // cdecl (out, id, dest, size, dest2, size2) -> out
const JOB_REQUEST_VEHICLE: usize = 0x00A7_85F0; // EAX = id, cdecl (out, dest, size, dest2, size2) -> out
const REQUEST_ANIMATION: usize = 0x00A8_8240; // EDX = dest, cdecl (out, name, size, extra) -> out
const JOB_FINISH_LOADED: usize = 0x00A7_8950; // EDX = size, cdecl (id, data, 1)
const JOB_FINISH_FAILED: usize = 0x00A7_89B0; // cdecl (id, 1)
const RSCHED_UPDATE: usize = 0x00A7_5730; // EAX = scheduler
const RSCHED_NETWORK_UPDATE: usize = 0x00A7_8AC0;

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}
unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

/// Call a game function with EAX/ECX/EDX/ESI set and stack arguments, saving
/// EBX/ESI/EDI/EBP; ESP is restored afterwards (cdecl or callee-clean).
unsafe fn game_call(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, args: &[u32]) -> u32 {
    let block: [u32; 7] = [f as u32, eax, ecx, edx, esi, args.as_ptr() as u32, args.len() as u32];
    let r: u32;
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
            inout("edx") block.as_ptr() => _, lateout("eax") r, lateout("ecx") _,
            clobber_abi("C"),
        );
    }
    r
}

/// Unlink `n` from a {next +0x18, prev +0x1C} list whose head pointer is at
/// `head`, and clear its links.
unsafe fn unlink(head: usize, n: usize) {
    unsafe {
        if rd(head) as usize == n && rd(n + 0x18) as usize == n {
            wr(head, 0);
        } else {
            if rd(head) as usize == n {
                wr(head, rd(n + 0x18));
            }
            let (next, prev) = (rd(n + 0x18), rd(n + 0x1C));
            if prev != 0 {
                wr(prev as usize + 0x18, next);
            }
            if next != 0 {
                wr(next as usize + 0x1C, prev);
            }
        }
        wr(n + 0x1C, 0);
        wr(n + 0x18, 0);
    }
}

/// Append `n` at the tail of such a list.
unsafe fn append(head: usize, n: usize) {
    unsafe {
        let h = rd(head) as usize;
        if h == 0 {
            wr(n + 0x18, n as u32);
            wr(n + 0x1C, n as u32);
            wr(head, n as u32);
        } else {
            let tail = rd(h + 0x1C) as usize;
            wr(n + 0x1C, tail as u32);
            wr(n + 0x18, h as u32);
            wr(tail + 0x18, n as u32);
            wr(h + 0x1C, n as u32);
        }
    }
}

/// `rsched_job_add` (0xA77DD0; cdecl (id, dest, size, dest2, size2, level) ->
/// queue index, or -1 with 50 jobs queued).
unsafe fn job_add(a: [u32; 6]) -> i32 {
    unsafe {
        if rd(JOB_COUNT) >= MAX_JOBS {
            return -1;
        }
        let job = rd(JOBS_FREE) as usize;
        unlink(JOBS_FREE, job);
        append(JOBS_ACTIVE, job);
        wr(JOB_COUNT, rd(JOB_COUNT) + 1);
        let [id, dest, size, dest2, size2, level] = a;
        wr(job, id);
        wr(job + 4, dest);
        wr(job + 8, dest2);
        wr(job + 0x0C, size);
        wr(job + 0x10, size2);
        wr(job + 0x14, level);
        let push: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(Q_PUSH);
        push(JOB_QUEUE as u32, job as u32)
    }
}

/// `rsched_job_cancel` (0xA77E90; EAX = queue index -> EAX = job): take a queued
/// job off the queue and the active list and free it.
unsafe fn job_cancel(index: u32) -> u32 {
    unsafe {
        let job = game_call(Q_REMOVE, index, 0, JOB_QUEUE as u32, 0, &[]) as usize;
        unlink(JOBS_ACTIVE, job);
        wr(JOB_COUNT, rd(JOB_COUNT).wrapping_sub(1));
        append(JOBS_FREE, job);
        job as u32
    }
}

/// A registry entry's first name, the way job_start_next looks it up for
/// animations ("Empty" for id -1, NULL past the end or without names).
unsafe fn registry_name(id: u32) -> u32 {
    unsafe {
        if id == u32::MAX {
            return STR_EMPTY as u32;
        }
        let i = id & 0xFF_FFFF;
        if i >= rd(REGISTRY_COUNT) {
            return 0;
        }
        let list = rd(rd(REGISTRY) as usize + i as usize * 0xC + 4);
        wr(REGISTRY_DEBUG_NAME, list);
        if list == 0 { 0 } else { rd(list as usize) }
    }
}

/// `rsched_job_start_next` (0xA77B90): make the best queued job current, prepare
/// its slot, and request it from the streamer by kind (id >> 24): characters
/// (0, 2, 3), vehicles (1, 5) or an animation (4).
unsafe fn job_start_next() {
    unsafe {
        if rd(JOB_QUEUE) == 0 {
            return;
        }
        let pop: extern "stdcall" fn(u32) -> u32 = std::mem::transmute(Q_POP_BEST);
        let job = pop(JOB_QUEUE as u32) as usize;
        unlink(JOBS_ACTIVE, job);
        wr(JOB_CURRENT, job as u32);
        let id = rd(job);
        let kind = id >> 24;
        game_call(JOB_BEGIN_SLOT, id, (SCHEDULERS + kind as usize * SCHED_SIZE) as u32, 0, 0, &[]);
        let f = |o: usize| rd(job + o);
        let mut out = 0u32;
        let out_p = &mut out as *mut u32 as u32;
        match kind {
            0 | 2 | 3 => {
                game_call(JOB_REQUEST_CHARACTER, 0, 0, 0, 0, &[out_p, id, f(4), f(0x0C), f(8), f(0x10)]);
            }
            1 | 5 => {
                game_call(JOB_REQUEST_VEHICLE, id, 0, 0, 0, &[out_p, f(4), f(0x0C), f(8), f(0x10)]);
            }
            4 => {
                let name = registry_name(id);
                let extra = rd(rd(REGISTRY) as usize + (id & 0xFF_FFFF) as usize * 0xC + 8);
                game_call(REQUEST_ANIMATION, 0, 0, f(4), 0, &[out_p, name, f(0x0C), extra]);
            }
            _ => return,
        }
        wr(JOB_HANDLE, out);
        event(format!("job_start k{kind} n{:x} n2{:x} level {} -> {}", f(0x0C), f(0x10), f(0x14), handle_text(out)));
    }
}

fn handle_text(h: u32) -> String {
    if h >> 16 == 0xFFFF { "none".into() } else { format!("{h:#x}") }
}

/// `rsched_job_pump` (0xA77F10): once the current job's streamer slot is idle,
/// hand the result to the scheduler (loaded: data and size; else failed), clear
/// the slot, free the job and start the next one.
unsafe fn job_pump() {
    unsafe {
        let h = rd(JOB_HANDLE);
        if !(h >> 16 == 0xFFFF && h & 0xFFFF == 0) {
            if game_call(STREAMMAN_IS_IDLE, 0, 0, 0, 0, &[h]) as u8 != 1 {
                return;
            }
            let slot = (rd(SLOT_ARRAYS + (h >> 16) as usize * 4) as usize).wrapping_add((h & 0xFFFF) as usize * 0x28);
            let job = rd(JOB_CURRENT) as usize;
            let loaded = slot != 0 && rd(slot + 4) == 4;
            if loaded {
                game_call(JOB_FINISH_LOADED, 0, 0, rd(slot + 0x0C), 0, &[rd(job), rd(slot + 8), 1]);
            } else {
                game_call(JOB_FINISH_FAILED, 0, 0, 0, 0, &[rd(job), 1]);
            }
            event(format!("job_done k{} -> {}", rd(job) >> 24, if loaded { "loaded" } else { "failed" }));
            let h = rd(JOB_HANDLE);
            let slot = (rd(SLOT_ARRAYS + (h >> 16) as usize * 4) as usize).wrapping_add((h & 0xFFFF) as usize * 0x28);
            wr(slot + 4, 0);
            wr(slot + 8, 0);
            wr(slot + 0x0C, 0);
            let job = rd(JOB_CURRENT) as usize;
            wr(job, u32::MAX);
            for o in (4..0x20).step_by(4) {
                wr(job + o, 0);
            }
            append(JOBS_FREE, job);
            wr(JOB_COUNT, rd(JOB_COUNT).wrapping_sub(1));
            wr(JOB_CURRENT, 0);
            wr(JOB_HANDLE, 0xFFFF_0000);
        }
        job_start_next_hook();
    }
}

/// `rsched_update_all` (0xA79080): update the six schedulers, pump the jobs, and
/// run the network-side update when this machine is a host in a running game.
unsafe fn update_all() {
    unsafe {
        for k in 0..6 {
            game_call(RSCHED_UPDATE, (SCHEDULERS + k * SCHED_SIZE) as u32, 0, 0, 0, &[]);
        }
        job_pump_hook();
        let streaming_off = rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0;
        let net = rd(0x0252_8C14) as usize;
        if !streaming_off
            && net != 0
            && !(rb(0x0252_8B56) != 0 && rb(0x0252_8B2B) == 0)
            && rb(net + 0x3F8) != 0
            && (rd(0x0252_8BB4) as i32) < 1
        {
            game_call(RSCHED_NETWORK_UPDATE, 0, 0, 0, 0, &[]);
        }
    }
}

// --- Schedulers, requests and slots ---------------------------------------------------
//
// Scheduler (0xD4): +0 slot count, +4 group count, +8 slots (0x3C each), +0xC idle
// list, +0x10 loading list, +0x14 slot of the current job, +0x18 in-use list, +0x1C
// free list, +0x20 stride A, +0x24 buffer A, +0x28 stride B, +0x2C buffer B, +0x38
// active requests, +0x3C free requests, +0x40 / +0x44 per-group request / slot queues
// (0x18 each), +0x48 deferred release count, +0x4C deferred releases (0x10 each),
// +0xCC -> per-group slot quotas, +0xD0 flags (1 disabled, 2 dirty, 8 updating).
//
// Request (0x48): +0 slot, +4 id, +8 group count, +0xC slot in use, +0x10 queue node,
// +0x14 -> per-group counts, +0x18 flags (1 evaluated, 2 load now), +0x1C/+0x20
// links, +0x24/+0x28 temp links, +0x2C..+0x44 counts per level 0..6.
//
// Slot (0x3C): +0 request, +4 id, +8 data, +0xC size, +0x10 loaded data, +0x14 time,
// +0x18 request using it, +0x1C job, +0x20 queue node, +0x24 group, +0x28 state (0
// idle, 1 queued, 2 loading, 3 in use, 4 free), +0x2C/+0x30 links, +0x34/+0x38 temp
// links.

const CALLBACKS: usize = 0x0222_48F8; // per kind 0x10: +0 compare, +4 available, +8 failed, +0xC unload
const CB_AVAILABLE: usize = 4;
const CB_FAILED: usize = 8;
const CB_UNLOAD: usize = 0xC;

unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}

unsafe fn set_dirty(s: usize) {
    unsafe { wb(s + 0xD0, rb(s + 0xD0) | 2) }
}

fn req_q(s: usize, g: u32) -> usize {
    unsafe { rd(s + 0x40) as usize + g as usize * 0x18 }
}

fn slot_q(s: usize, g: u32) -> usize {
    unsafe { rd(s + 0x44) as usize + g as usize * 0x18 }
}

unsafe fn q_push(q: usize, v: usize) -> u32 {
    unsafe {
        let push: extern "thiscall" fn(u32, u32) -> i32 = std::mem::transmute(Q_PUSH);
        push(q as u32, v as u32) as u32
    }
}

unsafe fn q_remove(q: usize, node: u32) {
    unsafe {
        game_call(Q_REMOVE, node, 0, q as u32, 0, &[]);
    }
}

unsafe fn q_pop_best(q: usize) -> usize {
    unsafe {
        let pop: extern "stdcall" fn(u32) -> u32 = std::mem::transmute(Q_POP_BEST);
        pop(q as u32) as usize
    }
}

/// Unlink `n` from the circular list whose head pointer is at `head` (links at
/// `next`/`prev`), the way every rsched list does it, and clear its links.
unsafe fn list_unlink(head: usize, n: usize, next: usize, prev: usize) {
    unsafe {
        if rd(head) as usize == n {
            let nx = rd(n + next);
            if nx as usize == n {
                wr(n + next, 0);
                wr(n + prev, 0);
                wr(head, 0);
                return;
            }
            wr(head, nx);
        }
        let (nx, pv) = (rd(n + next), rd(n + prev));
        wr(pv as usize + next, nx);
        wr(nx as usize + prev, pv);
        wr(n + next, 0);
        wr(n + prev, 0);
    }
}

/// Append `n` at the tail of such a list.
unsafe fn list_append(head: usize, n: usize, next: usize, prev: usize) {
    unsafe {
        let h = rd(head) as usize;
        if h == 0 {
            wr(n + next, n as u32);
            wr(n + prev, n as u32);
            wr(head, n as u32);
        } else {
            let tail = rd(h + prev) as usize;
            wr(n + prev, tail as u32);
            wr(n + next, h as u32);
            wr(tail + next, n as u32);
            wr(h + prev, n as u32);
        }
    }
}

/// The element after `n`, or 0 at the end of the circular list headed by `head`.
unsafe fn next_or_end(n: usize, next: usize, head: usize) -> usize {
    unsafe {
        let x = rd(n + next) as usize;
        if x == head { 0 } else { x }
    }
}

/// A request's best level: the highest of levels 0..6 with a count (0x00A75080).
unsafe fn best_level(req: usize) -> u32 {
    unsafe { (0..7u32).rev().find(|&i| rd(req + 0x2C + i as usize * 4) != 0).unwrap_or(0) }
}

/// A request's best group: the highest group with a count (0x00A750A0).
unsafe fn best_group(req: usize) -> u32 {
    unsafe {
        let counts = rd(req + 0x14) as usize;
        (0..rd(req + 8)).rev().find(|&g| rd(counts + g as usize * 4) != 0).unwrap_or(0)
    }
}

/// The registry's "load failed" bit for an id (none for -1 or past the end).
unsafe fn id_failed(id: u32) -> bool {
    unsafe {
        let i = id & 0xFF_FFFF;
        i < rd(REGISTRY_COUNT) && rd(rd(REGISTRY) as usize + i as usize * 0xC) >> 3 & 1 != 0
    }
}

/// 0x00A75170: 2 if only `a` failed, 0 if only `b` failed, else 1.
unsafe fn compare_failed(a: u32, b: u32) -> i32 {
    unsafe {
        let fa = a != u32::MAX && id_failed(a);
        let fb = b != u32::MAX && id_failed(b);
        match (fa, fb) {
            (true, false) => 2,
            (false, true) => 0,
            _ => 1,
        }
    }
}

/// 0x00A78E30: the kind's compare callback for two slots' ids (1 without one).
unsafe fn compare_callback(a: usize, b: usize) -> i32 {
    unsafe {
        let (ia, ib) = (rd(a + 4), rd(b + 4));
        if ia == u32::MAX || ib == u32::MAX {
            return 1;
        }
        let f = rd(CALLBACKS + (ia >> 24) as usize * 0x10);
        if f == 0 {
            return 1;
        }
        let f: extern "C" fn(u32, u32) -> i32 = std::mem::transmute(f as usize);
        f(ia, ib)
    }
}

/// `rsched_slot_compare` (0xA751E0): failed slots last, then by the bound request's
/// best level (2 without one), then time, then loaded data, then the kind callback.
unsafe fn slot_compare(a: usize, b: usize) -> i32 {
    unsafe {
        let r = compare_failed(rd(a + 4), rd(b + 4));
        if r != 1 {
            return if r == 2 { 0 } else { 2 };
        }
        let level = |s: usize| if rd(s) != 0 { best_level(rd(s) as usize) } else { 2 };
        let (la, lb) = (level(a), level(b));
        if la != lb {
            return if la < lb { 0 } else { 2 };
        }
        let (ta, tb) = (*((a + 0x14) as *const f32), *((b + 0x14) as *const f32));
        if tb > ta {
            return 0;
        }
        if tb < ta {
            return 2;
        }
        let (ua, ub) = (rd(a + 0x10), rd(b + 0x10));
        if ua != ub {
            return if ua < ub { 0 } else { 2 };
        }
        compare_callback(a, b)
    }
}

/// `rsched_request_compare` (0xA752A0): evaluated first, failed last, then by best
/// level, then the bound slots (time, then loaded data).
unsafe fn request_compare(a: usize, b: usize) -> i32 {
    unsafe {
        let (ea, eb) = (rb(a + 0x18) & 1 != 0, rb(b + 0x18) & 1 != 0);
        if ea && !eb {
            return 2;
        }
        if !ea && eb {
            return 0;
        }
        let r = compare_failed(rd(a + 4), rd(b + 4));
        if r != 1 {
            return r;
        }
        let (la, lb) = (best_level(a), best_level(b));
        if la != lb {
            return if la < lb { 2 } else { 0 };
        }
        let (sa, sb) = (rd(a) as usize, rd(b) as usize);
        match (sa != 0, sb != 0) {
            (true, false) => return 0,
            (false, true) => return 2,
            (false, false) => return 1,
            _ => {}
        }
        let (ta, tb) = (*((sa + 0x14) as *const f32), *((sb + 0x14) as *const f32));
        if tb > ta {
            return 2;
        }
        if tb < ta {
            return 0;
        }
        let (ua, ub) = (rd(sa + 0x10), rd(sb + 0x10));
        if ua < ub {
            2
        } else if ua > ub {
            0
        } else {
            1
        }
    }
}

/// Call a kind callback (+4 available, +8 failed, +0xC unload) with an id.
unsafe fn callback(off: usize, id: u32) {
    unsafe {
        let f = rd(CALLBACKS + (id >> 24) as usize * 0x10 + off);
        if f != 0 {
            let f: extern "C" fn(u32) -> u32 = std::mem::transmute(f as usize);
            f(id);
        }
    }
}

/// 0x00A75030: reset a request (id -1, no node, no counts).
unsafe fn request_clear(req: usize) {
    unsafe {
        wr(req, 0);
        wr(req + 0x0C, 0);
        wr(req + 4, u32::MAX);
        wr(req + 0x10, u32::MAX);
        wb(req + 0x18, 0);
        for o in (0x1C..=0x44).step_by(4) {
            wr(req + o, 0);
        }
        std::ptr::write_bytes(rd(req + 0x14) as *mut u8, 0, rd(req + 8) as usize * 4);
    }
}

/// The active request with this id, or 0.
unsafe fn find_request(s: usize, id: u32) -> usize {
    unsafe {
        let head = rd(s + 0x38) as usize;
        let mut r = head;
        while r != 0 {
            if rd(r + 4) == id {
                return r;
            }
            r = rd(r + 0x1C) as usize;
            if r == head {
                break;
            }
        }
        0
    }
}

/// `rsched_slot_set_group` (0xA76F00): move a slot to another group's queue (the
/// current job's slot is in no queue).
unsafe fn slot_set_group(s: usize, slot: usize, old: u32, new: u32) {
    unsafe {
        if old == new {
            return;
        }
        set_dirty(s);
        wr(slot + 0x24, new);
        if rd(s + 0x14) as usize != slot {
            q_remove(slot_q(s, old), rd(slot + 0x20));
            wr(slot + 0x20, q_push(slot_q(s, new), slot));
        }
    }
}

/// `rsched_bind_loaded_slot` (0xA76C20; EDI = scheduler, stack req): bind the
/// request to the first non-free slot holding its id, in the request's best group.
unsafe fn bind_loaded_slot(s: usize, req: usize) {
    unsafe {
        let group = best_group(req);
        let id = rd(req + 4);
        for i in 0..rd(s) as usize {
            let slot = rd(s + 8) as usize + i * 0x3C;
            if rd(slot + 4) == id && rd(slot + 0x28) != 4 {
                wr(slot, req as u32);
                wr(req, slot as u32);
                slot_set_group(s, slot, rd(slot + 0x24), group);
                return;
            }
        }
    }
}

/// `rsched_slot_use` (0xA76CD0): an idle slot goes in use by a request.
unsafe fn slot_use(s: usize, slot: usize, req: usize) {
    unsafe {
        list_unlink(s + 0x0C, slot, 0x2C, 0x30);
        list_append(s + 0x18, slot, 0x2C, 0x30);
        wr(slot + 0x28, 3);
        wr(slot + 0x18, req as u32);
        wr(req + 0x0C, slot as u32);
    }
}

/// `rsched_slot_make_idle` (0xA76D50): an in-use slot goes idle; tell the kind.
unsafe fn slot_make_idle(s: usize, slot: usize) {
    unsafe {
        list_unlink(s + 0x18, slot, 0x2C, 0x30);
        list_append(s + 0x0C, slot, 0x2C, 0x30);
        let user = rd(slot + 0x18) as usize;
        wr(slot + 0x28, 0);
        wr(user + 0x0C, 0);
        wr(slot + 0x18, 0);
        callback(CB_AVAILABLE, rd(slot + 4));
    }
}

/// `rsched_slot_evict` (0xA76DE0; ESI = slot, stack scheduler): unbind the slot,
/// cancel its job, take it off its state list, and unload an idle slot's data.
unsafe fn slot_evict(s: usize, slot: usize) {
    unsafe {
        let unbind = |slot: usize| {
            let r = rd(slot) as usize;
            if r != 0 {
                wr(r, 0);
                wr(slot, 0);
            }
        };
        unbind(slot);
        if rd(slot + 0x18) != 0 {
            slot_make_idle(s, slot);
        }
        let list = match rd(slot + 0x28) {
            0 => s + 0x0C,
            1 => {
                let job = rd(slot + 0x1C);
                if job != u32::MAX {
                    job_cancel_hook(job);
                    wr(slot + 0x1C, u32::MAX);
                }
                s + 0x10
            }
            4 => s + 0x1C,
            _ => 0,
        };
        list_unlink(list, slot, 0x2C, 0x30);
        if rd(slot + 0x28) == 0 {
            callback(CB_UNLOAD, rd(slot + 4));
            unbind(slot);
        }
    }
}

/// `rsched_request_requeue` (0xA76EA0): move a request (and its slot) between groups.
unsafe fn request_requeue(s: usize, req: usize, old: u32, new: u32) {
    unsafe {
        if old == new {
            return;
        }
        set_dirty(s);
        q_remove(req_q(s, old), rd(req + 0x10));
        wr(req + 0x10, q_push(req_q(s, new), req));
        let slot = rd(req) as usize;
        if slot != 0 {
            slot_set_group(s, slot, old, new);
        }
    }
}

/// `rsched_add_ref` (0xA757F0; EAX = scheduler, stack (id, level, group, load_now)).
unsafe fn add_ref(s: usize, id: u32, level: u32, group: u32, load_now: u32) {
    unsafe {
        let req = find_request(s, id);
        if req != 0 {
            let (old_level, old_group) = (best_level(req), best_group(req));
            wr(req + 0x2C + level as usize * 4, rd(req + 0x2C + level as usize * 4).wrapping_add(1));
            let counts = rd(req + 0x14) as usize + group as usize * 4;
            wr(counts, rd(counts).wrapping_add(1));
            let (new_level, new_group) = (best_level(req), best_group(req));
            if old_level != new_level {
                set_dirty(s);
            }
            if old_group != new_group {
                set_dirty(s);
                q_remove(req_q(s, old_group), rd(req + 0x10));
                wr(req + 0x10, q_push(req_q(s, new_group), req));
                let slot = rd(req) as usize;
                if slot != 0 {
                    set_dirty(s);
                    wr(slot + 0x24, new_group);
                    if rd(s + 0x14) as usize != slot {
                        q_remove(slot_q(s, old_group), rd(slot + 0x20));
                        wr(slot + 0x20, q_push(slot_q(s, new_group), slot));
                    }
                }
            }
            if rd(req) == 0 {
                bind_loaded_slot(s, req);
            }
            return;
        }
        let req = rd(s + 0x3C) as usize;
        if req == 0 {
            return;
        }
        list_unlink(s + 0x3C, req, 0x1C, 0x20);
        request_clear(req);
        wr(req + 4, id);
        wb(req + 0x18, (load_now as u8) << 1 & 2);
        wr(req + 0x2C + level as usize * 4, rd(req + 0x2C + level as usize * 4).wrapping_add(1));
        let counts = rd(req + 0x14) as usize + group as usize * 4;
        wr(counts, rd(counts).wrapping_add(1));
        list_append(s + 0x38, req, 0x1C, 0x20);
        wr(req + 0x10, q_push(req_q(s, group), req));
        bind_loaded_slot(s, req);
        set_dirty(s);
    }
}

/// `rsched_release` (0xA75A60; EAX = scheduler, EDX = id, stack (level, group, all)).
/// Deferred while the scheduler updates; frees the request when no level is left.
unsafe fn release(s: usize, id: u32, level: u32, group: u32, all: u32) {
    unsafe {
        if rb(s + 0xD0) & 8 != 0 {
            let n = rd(s + 0x48);
            let rec = s + 0x4C + n as usize * 0x10;
            wr(s + 0x48, n + 1);
            wr(rec + 4, level);
            wr(rec + 0x0C, group);
            wr(rec, id);
            wb(rec + 8, all as u8);
            return;
        }
        let req = find_request(s, id);
        if req == 0 {
            let streaming_off = rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0;
            if !streaming_off && id != u32::MAX && (id & 0xFF_FFFF) < rd(REGISTRY_COUNT) {
                wr(REGISTRY_DEBUG_NAME, rd(rd(REGISTRY) as usize + (id & 0xFF_FFFF) as usize * 0xC + 4));
            }
            return;
        }
        let old_group = best_group(req);
        if all as u8 != 0 {
            for l in 0..7 {
                wr(req + 0x2C + l * 4, 0);
            }
            std::ptr::write_bytes(rd(req + 0x14) as *mut u8, 0, rd(req + 8) as usize * 4);
        } else {
            wr(req + 0x2C + level as usize * 4, rd(req + 0x2C + level as usize * 4).wrapping_sub(1));
            let counts = rd(req + 0x14) as usize + group as usize * 4;
            wr(counts, rd(counts).wrapping_sub(1));
        }
        if (0..7).any(|l| rd(req + 0x2C + l * 4) != 0) {
            request_requeue(s, req, old_group, best_group(req));
            if rd(req) == 0 {
                bind_loaded_slot(s, req);
            }
            return;
        }
        let slot = rd(req) as usize;
        if slot != 0 {
            let r = rd(slot) as usize;
            if r != 0 {
                wr(r, 0);
                wr(slot, 0);
            }
        }
        let using = rd(req + 0x0C) as usize;
        if using != 0 {
            slot_make_idle(s, using);
        }
        list_unlink(s + 0x38, req, 0x1C, 0x20);
        q_remove(req_q(s, old_group), rd(req + 0x10));
        request_clear(req);
        list_append(s + 0x3C, req, 0x1C, 0x20);
        set_dirty(s);
    }
}

/// `rsched_update` (0xA75730; EAX = scheduler): when dirty, bind requests to
/// loaded slots, share the slots out by group quota, pair requests with slots per
/// group, then replay the releases deferred meanwhile.
unsafe fn update(s: usize) {
    unsafe {
        let f = rb(s + 0xD0);
        if f & 1 != 0 || f & 2 == 0 {
            return;
        }
        wb(s + 0xD0, (f & !2) | 8);
        let mut r = rd(s + 0x38) as usize;
        while r != 0 {
            if rd(r) == 0 {
                bind_loaded_slot(s, r);
            }
            r = rd(r + 0x1C) as usize;
            if r == rd(s + 0x38) as usize {
                break;
            }
        }
        assign_group_quotas(s);
        for g in 0..rd(s + 4) {
            update_group(s, g);
        }
        let mut r = rd(s + 0x38) as usize;
        while r != 0 {
            wb(r + 0x18, rb(r + 0x18) & !1);
            r = rd(r + 0x1C) as usize;
            if r == rd(s + 0x38) as usize {
                break;
            }
        }
        wb(s + 0xD0, rb(s + 0xD0) & !8);
        let mut i = 0;
        while i < rd(s + 0x48) as usize {
            let rec = s + 0x4C + i * 0x10;
            release_entry(s, rd(rec), rd(rec + 4), rd(rec + 0x0C), rb(rec + 8) as u32);
            i += 1;
        }
        wr(s + 0x48, 0);
    }
}

/// `rsched_assign_group_quotas` (0xA76220): trim each group's slots to its quota
/// (the excess from the front of its queue order goes spare), then hand spare
/// slots to groups below quota, unbinding them from their requests.
unsafe fn assign_group_quotas(s: usize) {
    unsafe {
        let groups = rd(s + 4) as usize;
        let mut counts = [0u32; 0x40];
        let mut spare = 0u32;
        let spare_h = &mut spare as *mut u32 as usize;
        let quota = |g: usize| rd(rd(s + 0xCC) as usize + g * 4);
        let current_in = |g: usize| {
            let cur = rd(s + 0x14) as usize;
            cur != 0 && rd(cur + 0x24) as usize == g
        };
        for g in 0..groups {
            counts[g] = 0;
            let mut temp = 0u32;
            let temp_h = &mut temp as *mut u32 as usize;
            let mut n = 0u32;
            loop {
                let slot = q_pop_best(slot_q(s, g as u32));
                if slot == 0 {
                    break;
                }
                n += 1;
                wr(slot + 0x20, u32::MAX);
                list_append(temp_h, slot, 0x34, 0x38);
            }
            if current_in(g) {
                n += 1;
            }
            if n > quota(g) {
                for _ in 0..n - quota(g) {
                    let slot = rd(temp_h) as usize;
                    if slot == 0 {
                        break;
                    }
                    list_unlink(temp_h, slot, 0x34, 0x38);
                    list_append(spare_h, slot, 0x34, 0x38);
                }
            }
            loop {
                let slot = rd(temp_h) as usize;
                if slot == 0 {
                    break;
                }
                list_unlink(temp_h, slot, 0x34, 0x38);
                wr(slot + 0x20, q_push(slot_q(s, g as u32), slot));
                counts[g] += 1;
            }
            if current_in(g) {
                counts[g] += 1;
            }
        }
        for g in 0..groups {
            if counts[g] >= quota(g) {
                continue;
            }
            for _ in 0..quota(g) - counts[g] {
                let slot = rd(spare_h) as usize;
                if slot == 0 {
                    break;
                }
                list_unlink(spare_h, slot, 0x34, 0x38);
                let r = rd(slot) as usize;
                if r != 0 {
                    wb(r + 0x18, rb(r + 0x18) | 1);
                    wr(r, 0);
                    wr(slot, 0);
                }
                wr(slot + 0x20, q_push(slot_q(s, g as u32), slot));
                counts[g] += 1;
                wr(slot + 0x24, g as u32);
            }
        }
    }
}

/// `rsched_update_group` (0xA764D0; EAX = group, stack scheduler): pair the group's
/// best requests with its worst slots, starting loads (jobs) where needed.
unsafe fn update_group(s: usize, g: u32) {
    unsafe {
        let (rq, sq) = (req_q(s, g), slot_q(s, g));
        let mut slots = 0u32;
        let mut reqs = 0u32;
        let sh = &mut slots as *mut u32 as usize;
        let rh = &mut reqs as *mut u32 as usize;
        let mut n = 0u32;
        loop {
            let slot = q_pop_best(sq);
            if slot == 0 {
                break;
            }
            wr(slot + 0x20, u32::MAX);
            list_append(sh, slot, 0x34, 0x38);
            n += 1;
        }
        if n > 0 {
            let mut i = 0;
            let mut r = q_pop_best(rq);
            while r != 0 {
                wr(r + 0x10, u32::MAX);
                list_append(rh, r, 0x24, 0x28);
                i += 1;
                if i >= n {
                    break;
                }
                r = q_pop_best(rq);
            }
        }
        // Requests whose resource failed to load go straight back.
        let mut r = rd(rh) as usize;
        while r != 0 {
            let next = next_or_end(r, 0x24, rd(rh) as usize);
            if id_failed(rd(r + 4)) {
                list_unlink(rh, r, 0x24, 0x28);
                wr(r + 0x10, q_push(rq, r));
            }
            r = next;
        }
        // Requests already holding a slot with their id keep it.
        let mut r = rd(rh) as usize;
        while r != 0 {
            let next = next_or_end(r, 0x24, rd(rh) as usize);
            let mut slot = rd(sh) as usize;
            while slot != 0 {
                let snext = next_or_end(slot, 0x34, rd(sh) as usize);
                if rd(r + 4) == rd(slot + 4) {
                    break;
                }
                slot = snext;
            }
            if slot != 0 {
                list_unlink(rh, r, 0x24, 0x28);
                list_unlink(sh, slot, 0x34, 0x38);
                if best_level(r) >= 5 && rd(slot + 0x18) != 0 {
                    slot_make_idle(s, slot);
                }
                wr(r + 0x10, q_push(rq, r));
                wr(slot + 0x20, q_push(sq, slot));
            }
            let cur = rd(s + 0x14) as usize;
            if cur != 0 && rd(r + 4) == rd(cur + 4) {
                list_unlink(rh, r, 0x24, 0x28);
                wr(r + 0x10, q_push(rq, r));
            }
            r = next;
        }
        // Requests whose id sits in a slot of another group wait for it.
        let mut r = rd(rh) as usize;
        while r != 0 {
            let next = next_or_end(r, 0x24, rd(rh) as usize);
            for i in 0..rd(s) as usize {
                let slot = rd(s + 8) as usize + i * 0x3C;
                if rd(r + 4) == rd(slot + 4) && rd(slot + 0x24) != best_group(r) {
                    list_unlink(rh, r, 0x24, 0x28);
                    if best_level(r) >= 5 && rd(slot + 0x18) != 0 {
                        slot_make_idle(s, slot);
                    }
                    wr(r + 0x10, q_push(rq, r));
                }
            }
            r = next;
        }
        // Pair the rest, best request with worst slot.
        let (s_head, r_head) = (rd(sh) as usize, rd(rh) as usize);
        let (mut slot, mut r) = (s_head, r_head);
        while slot != 0 && r != 0 {
            let snext = next_or_end(slot, 0x34, s_head);
            let rnext = next_or_end(r, 0x24, r_head);
            let level = best_level(r);
            if level >= 5 || rd(slot + 0x10) == 0 {
                slot_evict(s, slot);
                list_append(s + 0x10, slot, 0x2C, 0x30);
                wr(slot + 0x28, 1);
                wr(slot + 0x14, 0);
                wr(slot + 4, rd(r + 4));
                wr(slot + 8, 0);
                wr(slot + 0x0C, 0);
                wr(slot + 0x10, 0);
                wr(slot, r as u32);
                wr(r, slot as u32);
                if rb(r + 0x18) & 2 != 0 {
                    let i = ((slot - rd(s + 8) as usize) / 0x3C) as u32;
                    let (size_a, size_b) = (rd(s + 0x20), rd(s + 0x28));
                    let dest_a = rd(s + 0x24).wrapping_add(i.wrapping_mul(size_a));
                    let dest_b = rd(s + 0x2C).wrapping_add(i.wrapping_mul(size_b));
                    let job = job_add_hook(rd(slot + 4), dest_a, size_a, dest_b, size_b, level);
                    wr(slot + 0x1C, job as u32);
                } else {
                    wr(slot + 0x1C, u32::MAX);
                }
                let using = rd(r + 0x0C) as usize;
                if using != 0 {
                    slot_make_idle(s, using);
                }
            } else if rd(r + 0x0C) == 0 && rd(slot + 0x18) == 0 {
                slot_use(s, slot, r);
            }
            slot = snext;
            r = rnext;
        }
        // Everything left goes back on the queues (temp links are left as they are).
        let mut r = r_head;
        while r != 0 {
            wr(r + 0x10, q_push(rq, r));
            r = rd(r + 0x24) as usize;
            if r == r_head {
                break;
            }
        }
        let s_head = rd(sh) as usize;
        let mut slot = s_head;
        while slot != 0 {
            wr(slot + 0x20, q_push(sq, slot));
            slot = rd(slot + 0x34) as usize;
            if slot == s_head {
                break;
            }
        }
    }
}

/// `rsched_job_begin_slot` (0xA75F00; EAX = id, ECX = scheduler): the queued slot
/// with this id becomes the one loading (state 2), out of its queue.
unsafe fn job_begin_slot(id: u32, s: usize) {
    unsafe {
        let head = rd(s + 0x10) as usize;
        let mut slot = head;
        while slot != 0 {
            if rd(slot + 4) == id {
                list_unlink(s + 0x10, slot, 0x2C, 0x30);
                wr(s + 0x14, slot as u32);
                wr(slot + 0x28, 2);
                q_remove(slot_q(s, rd(slot + 0x24)), rd(slot + 0x20));
                wr(slot + 0x20, u32::MAX);
                return;
            }
            slot = rd(slot + 0x2C) as usize;
            if slot == head {
                return;
            }
        }
    }
}

/// `rsched_slot_loaded` (0xA75F80): the loading slot goes idle with its data.
unsafe fn slot_loaded(s: usize, data: u32, size: u32) {
    unsafe {
        let slot = rd(s + 0x14) as usize;
        list_append(s + 0x0C, slot, 0x2C, 0x30);
        wr(slot + 0x28, 0);
        wr(slot + 8, data);
        wr(slot + 0x0C, size);
        wr(slot + 0x20, q_push(slot_q(s, rd(slot + 0x24)), slot));
        set_dirty(s);
        wr(s + 0x14, 0);
    }
}

/// `rsched_slot_load_failed` (0xA76010): the loading slot is reset to free.
unsafe fn slot_load_failed(s: usize) {
    unsafe {
        let slot = rd(s + 0x14) as usize;
        let group = rd(slot + 0x24);
        let r = rd(slot) as usize;
        if r != 0 {
            wr(r, 0);
            wr(slot, 0);
        }
        wr(slot + 0x14, 0);
        wr(slot, 0);
        wr(slot + 4, u32::MAX);
        for o in [8, 0x0C, 0x10, 0x18] {
            wr(slot + o, 0);
        }
        wr(slot + 0x1C, u32::MAX);
        wr(slot + 0x20, u32::MAX);
        wr(slot + 0x24, 0);
        wr(slot + 0x28, 4);
        for o in (0x2C..=0x38).step_by(4) {
            wr(slot + o, 0);
        }
        list_append(s + 0x1C, slot, 0x2C, 0x30);
        wr(slot + 0x24, group);
        wr(slot + 0x20, q_push(slot_q(s, group), slot));
        wr(s + 0x14, 0);
    }
}

/// `rsched_job_finish_loaded` (0xA78950; EDX = size, cdecl (id, data, update)).
unsafe fn job_finish_loaded(id: u32, data: u32, update: u32, size: u32) {
    unsafe {
        let e = rd(REGISTRY) as usize + (id & 0xFF_FFFF) as usize * 0xC;
        wr(e, rd(e) & !8);
        if update as u8 == 1 {
            slot_loaded(SCHEDULERS + (id >> 24) as usize * SCHED_SIZE, data, size);
        }
        callback(CB_AVAILABLE, id);
    }
}

/// `rsched_job_finish_failed` (0xA789B0; cdecl (id, update)).
unsafe fn job_finish_failed(id: u32, update: u32) {
    unsafe {
        let e = rd(REGISTRY) as usize + (id & 0xFF_FFFF) as usize * 0xC;
        wr(e, rd(e) | 8);
        if update as u8 == 1 {
            slot_load_failed(SCHEDULERS + (id >> 24) as usize * SCHED_SIZE);
        }
        callback(CB_FAILED, id);
    }
}

// --- Hooks and report -----------------------------------------------------------------

const JOB_ADD: usize = 0;
const JOB_CANCEL: usize = 1;
const JOB_START_NEXT: usize = 2;
const JOB_PUMP: usize = 3;
const UPDATE_ALL: usize = 4;
const ADD_REF: usize = 5;
const RELEASE: usize = 6;
const UPDATE: usize = 7;
const BEGIN_SLOT: usize = 8;
const FINISH_LOADED: usize = 9;
const FINISH_FAILED: usize = 10;
const SLOT_COMPARE: usize = 11;
const REQUEST_COMPARE: usize = 12;
const COUNT: usize = 13;

const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("rsched_job_add", 0x00A7_7DD0, &[0x83, 0x3D, 0x4C, 0x9E, 0x52, 0x02, 0x32]),
    ("rsched_job_cancel", 0x00A7_7E90, &[0xBA, 0xD0, 0xB2, 0xF9, 0x02]),
    ("rsched_job_start_next", 0x00A7_7B90, &[0x83, 0xEC, 0x10, 0x53, 0x33, 0xDB]),
    ("rsched_job_pump", 0x00A7_7F10, &[0x51, 0x56, 0x8B, 0x35, 0xF8, 0x80, 0xEB, 0x00]),
    ("rsched_update_all", 0x00A7_9080, &[0x51, 0x56, 0x57, 0xBE, 0xE8, 0xB2, 0xF9, 0x02]),
    ("rsched_add_ref", 0x00A7_57F0, &[0x53, 0x8A, 0x5C, 0x24, 0x14]),
    ("rsched_release", 0x00A7_5A60, &[0x53, 0x8B, 0x5C, 0x24, 0x0C]),
    ("rsched_update", 0x00A7_5730, &[0x57, 0x8B, 0xF8, 0x8A, 0x87, 0xD0, 0x00, 0x00, 0x00]),
    ("rsched_job_begin_slot", 0x00A7_5F00, &[0x56, 0x57, 0x8B, 0xF8, 0x8B, 0x41, 0x10]),
    ("rsched_job_finish_loaded", 0x00A7_8950, &[0x8B, 0x0D, 0x3C, 0x9E, 0x52, 0x02]),
    ("rsched_job_finish_failed", 0x00A7_89B0, &[0x8B, 0x0D, 0x3C, 0x9E, 0x52, 0x02]),
    ("rsched_slot_compare", 0x00A7_51E0, &[0x56, 0x8B, 0x74, 0x24, 0x08]),
    ("rsched_request_compare", 0x00A7_52A0, &[0x56, 0x8B, 0x74, 0x24, 0x0C]),
];

static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

static EVENTS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

fn event(e: String) {
    *EVENTS.lock().unwrap_or_else(|p| p.into_inner()).entry(e).or_insert(0) += 1;
}

extern "C" fn job_add_hook(id: u32, dest: u32, size: u32, dest2: u32, size2: u32, level: u32) -> i32 {
    CALLS[JOB_ADD].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = if RSCHED_RUST {
            job_add([id, dest, size, dest2, size2, level])
        } else {
            let f: extern "C" fn(u32, u32, u32, u32, u32, u32) -> i32 = std::mem::transmute(orig(JOB_ADD));
            f(id, dest, size, dest2, size2, level)
        };
        event(format!("job_add k{} n{size:x} n2{size2:x} level {level} -> {}", id >> 24, (r >= 0) as u8));
        r
    }
}

#[unsafe(naked)]
extern "C" fn job_cancel_entry() {
    std::arch::naked_asm!("push eax", "call {h}", "add esp, 4", "ret", h = sym job_cancel_hook)
}

extern "C" fn job_cancel_hook(index: u32) -> u32 {
    CALLS[JOB_CANCEL].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let job = if RSCHED_RUST { job_cancel(index) } else { game_call(orig(JOB_CANCEL), index, 0, 0, 0, &[]) };
        event(format!("job_cancel k{}", rd(job as usize) >> 24));
        job
    }
}

extern "C" fn job_start_next_hook() {
    CALLS[JOB_START_NEXT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            job_start_next();
        } else {
            let job_before = rd(JOB_CURRENT);
            let f: extern "C" fn() = std::mem::transmute(orig(JOB_START_NEXT));
            f();
            let job = rd(JOB_CURRENT) as usize;
            if job != 0 && job as u32 != job_before {
                let id = rd(job);
                event(format!(
                    "job_start k{} n{:x} n2{:x} level {} -> {}",
                    id >> 24,
                    rd(job + 0x0C),
                    rd(job + 0x10),
                    rd(job + 0x14),
                    handle_text(rd(JOB_HANDLE))
                ));
            }
        }
    }
}

extern "C" fn job_pump_hook() {
    CALLS[JOB_PUMP].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            job_pump();
        } else {
            let (job, h) = (rd(JOB_CURRENT) as usize, rd(JOB_HANDLE));
            let kind = if job != 0 { rd(job) >> 24 } else { 0 };
            let slot = if h >> 16 == 0xFFFF {
                0
            } else {
                (rd(SLOT_ARRAYS + (h >> 16) as usize * 4) as usize).wrapping_add((h & 0xFFFF) as usize * 0x28)
            };
            let loaded = slot != 0 && rd(slot + 4) == 4;
            let f: extern "C" fn() = std::mem::transmute(orig(JOB_PUMP));
            f();
            // Finished when the job left the current slot (it is freed to id -1; the next
            // job can reuse the same streamer handle, so the handle alone can't tell).
            if job != 0 && (rd(JOB_CURRENT) as usize != job || rd(job) == u32::MAX) {
                event(format!("job_done k{kind} -> {}", if loaded { "loaded" } else { "failed" }));
            }
        }
    }
}

extern "C" fn update_all_hook() {
    CALLS[UPDATE_ALL].fetch_add(1, Ordering::Relaxed);
    wrap_callbacks();
    unsafe {
        if RSCHED_RUST {
            update_all();
        } else {
            let f: extern "C" fn() = std::mem::transmute(orig(UPDATE_ALL));
            f();
        }
    }
}

#[unsafe(naked)]
extern "C" fn add_ref_entry() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 16]",
        "push dword ptr [esp + 16]",
        "push dword ptr [esp + 16]",
        "push dword ptr [esp + 16]",
        "push eax",
        "call {h}",
        "add esp, 20",
        "ret 16",
        h = sym add_ref_hook,
    )
}

extern "C" fn add_ref_hook(s: u32, id: u32, level: u32, group: u32, load_now: u32) {
    CALLS[ADD_REF].fetch_add(1, Ordering::Relaxed);
    event(format!("add_ref k{} level {level} group {group} now {}", id >> 24, load_now & 0xFF));
    unsafe {
        if RSCHED_RUST {
            add_ref(s as usize, id, level, group, load_now);
        } else {
            game_call(orig(ADD_REF), s, 0, 0, 0, &[id, level, group, load_now]);
        }
    }
}

#[unsafe(naked)]
extern "C" fn release_entry_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 12]",
        "push dword ptr [esp + 12]",
        "push dword ptr [esp + 12]",
        "push edx",
        "push eax",
        "call {h}",
        "add esp, 20",
        "ret 12",
        h = sym release_hook,
    )
}

extern "C" fn release_hook(s: u32, id: u32, level: u32, group: u32, all: u32) {
    unsafe { release_entry(s as usize, id, level, group, all) }
}

/// What a call to `rsched_release` does in either mode (also the deferred replay).
unsafe fn release_entry(s: usize, id: u32, level: u32, group: u32, all: u32) {
    CALLS[RELEASE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if rb(s + 0xD0) & 8 == 0 {
            event(format!("release k{} level {level} group {group} all {}", id >> 24, all & 0xFF));
        }
        if RSCHED_RUST {
            release(s, id, level, group, all);
        } else {
            game_call(orig(RELEASE), s as u32, 0, id, 0, &[level, group, all]);
        }
    }
}

#[unsafe(naked)]
extern "C" fn update_entry() {
    std::arch::naked_asm!("push eax", "call {h}", "add esp, 4", "ret", h = sym update_hook)
}

extern "C" fn update_hook(s: u32) {
    CALLS[UPDATE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            update(s as usize);
        } else {
            game_call(orig(UPDATE), s, 0, 0, 0, &[]);
        }
    }
}

#[unsafe(naked)]
extern "C" fn begin_slot_entry() {
    std::arch::naked_asm!(
        "push ecx",
        "push ecx",
        "push eax",
        "call {h}",
        "add esp, 8",
        "pop ecx",
        "ret",
        h = sym begin_slot_hook,
    )
}

extern "C" fn begin_slot_hook(id: u32, s: u32) {
    CALLS[BEGIN_SLOT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            job_begin_slot(id, s as usize);
        } else {
            game_call(orig(BEGIN_SLOT), id, s, 0, 0, &[]);
        }
    }
}

#[unsafe(naked)]
extern "C" fn finish_loaded_entry() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 12]",
        "push dword ptr [esp + 12]",
        "push dword ptr [esp + 12]",
        "push edx",
        "call {h}",
        "add esp, 16",
        "ret",
        h = sym finish_loaded_hook,
    )
}

extern "C" fn finish_loaded_hook(size: u32, id: u32, data: u32, update: u32) {
    CALLS[FINISH_LOADED].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            job_finish_loaded(id, data, update, size);
        } else {
            game_call(orig(FINISH_LOADED), 0, 0, size, 0, &[id, data, update]);
        }
    }
}

extern "C" fn finish_failed_hook(id: u32, update: u32) {
    CALLS[FINISH_FAILED].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if RSCHED_RUST {
            job_finish_failed(id, update);
        } else {
            let f: extern "C" fn(u32, u32) = std::mem::transmute(orig(FINISH_FAILED));
            f(id, update);
        }
    }
}

static COMPARE_MISMATCHES: AtomicU32 = AtomicU32::new(0);

/// The two queue compares are pure: run both and count disagreements; the Rust
/// result is used in Rust mode.
fn shadow_compare(i: usize, a: u32, b: u32, rust: unsafe fn(usize, usize) -> i32) -> i32 {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let f: extern "C" fn(u32, u32) -> i32 = std::mem::transmute(orig(i));
        let o = f(a, b);
        let r = rust(a as usize, b as usize);
        if o != r && COMPARE_MISMATCHES.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!("rsched: {} mismatch ({a:#x}, {b:#x}): original {o}, Rust {r}", PORTS[i].0));
        }
        if RSCHED_RUST { r } else { o }
    }
}

extern "C" fn slot_compare_hook(a: u32, b: u32) -> i32 {
    shadow_compare(SLOT_COMPARE, a, b, slot_compare)
}

extern "C" fn request_compare_hook(a: u32, b: u32) -> i32 {
    shadow_compare(REQUEST_COMPARE, a, b, request_compare)
}

// The kinds' available / failed / unload callbacks are wrapped (in both modes) to log
// what the scheduler tells the game.
static CB_ORIG: [AtomicUsize; 24] = [const { AtomicUsize::new(0) }; 24];
static CB_WRAPPED: AtomicBool = AtomicBool::new(false);

fn wrap_callbacks() {
    if CB_WRAPPED.swap(true, Ordering::AcqRel) {
        return;
    }
    let wrappers: [(usize, usize); 3] = [
        (1, cb_available as extern "C" fn(u32) -> u32 as usize),
        (2, cb_failed as extern "C" fn(u32) -> u32 as usize),
        (3, cb_unload as extern "C" fn(u32) -> u32 as usize),
    ];
    for k in 0..6 {
        for &(j, w) in &wrappers {
            let p = CALLBACKS + k * 0x10 + j * 4;
            unsafe {
                let f = rd(p);
                if f != 0 {
                    CB_ORIG[k * 4 + j].store(f as usize, Ordering::Release);
                    wr(p, w as u32);
                }
            }
        }
    }
}

fn cb_forward(j: usize, name: &str, id: u32) -> u32 {
    let k = (id >> 24) as usize;
    event(format!("cb_{name} k{k}"));
    let f = if k < 6 { CB_ORIG[k * 4 + j].load(Ordering::Acquire) } else { 0 };
    if f == 0 {
        return 0;
    }
    let f: extern "C" fn(u32) -> u32 = unsafe { std::mem::transmute(f) };
    f(id)
}

extern "C" fn cb_available(id: u32) -> u32 {
    cb_forward(1, "available", id)
}

extern "C" fn cb_failed(id: u32) -> u32 {
    cb_forward(2, "failed", id)
}

extern "C" fn cb_unload(id: u32) -> u32 {
    cb_forward(3, "unload", id)
}

fn hook_address(i: usize) -> usize {
    match i {
        JOB_ADD => job_add_hook as extern "C" fn(u32, u32, u32, u32, u32, u32) -> i32 as usize,
        JOB_CANCEL => job_cancel_entry as extern "C" fn() as usize,
        JOB_START_NEXT => job_start_next_hook as extern "C" fn() as usize,
        JOB_PUMP => job_pump_hook as extern "C" fn() as usize,
        UPDATE_ALL => update_all_hook as extern "C" fn() as usize,
        ADD_REF => add_ref_entry as extern "C" fn() as usize,
        RELEASE => release_entry_thunk as extern "C" fn() as usize,
        UPDATE => update_entry as extern "C" fn() as usize,
        BEGIN_SLOT => begin_slot_entry as extern "C" fn() as usize,
        FINISH_LOADED => finish_loaded_entry as extern "C" fn() as usize,
        FINISH_FAILED => finish_failed_hook as extern "C" fn(u32, u32) as usize,
        SLOT_COMPARE => slot_compare_hook as extern "C" fn(u32, u32) -> i32 as usize,
        REQUEST_COMPARE => request_compare_hook as extern "C" fn(u32, u32) -> i32 as usize,
        _ => unreachable!(),
    }
}

pub fn install() {
    let mut ok = 0;
    for (i, &(name, addr, prologue)) in PORTS.iter().enumerate() {
        match unsafe { detour::install(addr, prologue, hook_address(i)) } {
            Ok(tramp) => {
                ORIG[i].store(tramp, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("rsched: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    let mode = if RSCHED_RUST { "Rust" } else { "original, observed" };
    log(&format!("replaced {ok}/{COUNT} rsched functions ({mode})"));
}

pub fn summary() -> String {
    let events = EVENTS.lock().unwrap_or_else(|p| p.into_inner());
    let text: String = events.iter().map(|(e, n)| format!("{e}\t{n}\n")).collect();
    let _ = std::fs::write("sr2-rsched.log", text);
    let parts: Vec<String> = PORTS
        .iter()
        .enumerate()
        .filter_map(|(i, &(name, ..))| {
            let c = CALLS[i].load(Ordering::Relaxed);
            (c > 0).then(|| format!("{name} {c}"))
        })
        .collect();
    format!("rsched live: {} | compare mismatches {}", parts.join(", "), COMPARE_MISMATCHES.load(Ordering::Relaxed))
}
