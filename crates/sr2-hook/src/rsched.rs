//! Resource slot scheduler ("rsched"), docs/analysis/05_streamer.md "Resource
//! slot scheduler". Six schedulers at 0x2F9B2E8 decide which characters,
//! vehicles and animations stay resident in a fixed number of memory slots, and
//! feed the streamer one load ("job") at a time.
//!
//! Ported so far: the job layer and the per-frame entry. Jobs are 0x20-byte
//! records {id, dest, dest2, size, size2, level, next +0x18, prev +0x1C}: at
//! most 50 queued (count 0x2529E4C) in the `streamman_queue` at 0x2F9B2D0, with
//! a free list (0x2529E54) and an active list (0x2529E50). Only one job is
//! current (0x2529E58); its streamer handle is at 0xEB80F8 (0xFFFF0000 = none).
//! Behind `RSCHED_RUST`; both modes log the same job events (`sr2-rsched.log`).
//! Still original: requests (add_ref/release), quotas, group pairing, slots and
//! the job finish handlers.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// Run the job layer in Rust (false: the originals, with the same event log).
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

// --- Hooks and report -----------------------------------------------------------------

const JOB_ADD: usize = 0;
const JOB_CANCEL: usize = 1;
const JOB_START_NEXT: usize = 2;
const JOB_PUMP: usize = 3;
const UPDATE_ALL: usize = 4;
const COUNT: usize = 5;

const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("rsched_job_add", 0x00A7_7DD0, &[0x83, 0x3D, 0x4C, 0x9E, 0x52, 0x02, 0x32]),
    ("rsched_job_cancel", 0x00A7_7E90, &[0xBA, 0xD0, 0xB2, 0xF9, 0x02]),
    ("rsched_job_start_next", 0x00A7_7B90, &[0x83, 0xEC, 0x10, 0x53, 0x33, 0xDB]),
    ("rsched_job_pump", 0x00A7_7F10, &[0x51, 0x56, 0x8B, 0x35, 0xF8, 0x80, 0xEB, 0x00]),
    ("rsched_update_all", 0x00A7_9080, &[0x51, 0x56, 0x57, 0xBE, 0xE8, 0xB2, 0xF9, 0x02]),
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
    unsafe {
        if RSCHED_RUST {
            update_all();
        } else {
            let f: extern "C" fn() = std::mem::transmute(orig(UPDATE_ALL));
            f();
        }
    }
}

fn hook_address(i: usize) -> usize {
    match i {
        JOB_ADD => job_add_hook as extern "C" fn(u32, u32, u32, u32, u32, u32) -> i32 as usize,
        JOB_CANCEL => job_cancel_entry as extern "C" fn() as usize,
        JOB_START_NEXT => job_start_next_hook as extern "C" fn() as usize,
        JOB_PUMP => job_pump_hook as extern "C" fn() as usize,
        UPDATE_ALL => update_all_hook as extern "C" fn() as usize,
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
    log(&format!("replaced {ok}/{COUNT} rsched job functions ({mode})"));
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
    format!("rsched live: {}", parts.join(", "))
}
