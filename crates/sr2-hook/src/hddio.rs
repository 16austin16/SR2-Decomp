//! The read scheduler ("hdd thread"), docs/analysis/05_streamer.md "Read
//! scheduler" and "Read scheduler API, exact behaviour".
//!
//! One thread services a priority-sorted list of read requests through the
//! file API (already Rust). The manager is `*(*0xEB80E0)`; everything below
//! +8 (its CRITICAL_SECTION) is guarded by that lock. Requests are 0x48-byte
//! records (256 at mgr+0x40) on circular lists through +0x40 (prev) / +0x44
//! (next): free (mgr+0x30), pending (mgr+0x34, highest priority first, FIFO
//! among equals) and finished (mgr+0x38).
//!
//! Ported: submit, set_state, finish, cancel(_impl), free/release, status,
//! error code, bytes read and the service loop the thread runs. The pure
//! queries are shadow-checked on every call; the rest run in Rust when `IO_RUST`
//! is set, else the originals, and both modes log the same events to
//! `sr2-io.log` (one per finished or cancelled request) for comparing runs.
//! Still original: the wait/urgent helpers (TLS and pooled wait events), the
//! read cache, creation and the thread procedure itself.
//!
//! The scattered `*0x2529E24 -> 0x2529E28` stores are the original's debug
//! checks (they change no state); they are kept where they happen.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// Run the scheduler in Rust (false: the originals, with the same event log).
const IO_RUST: bool = true;

const IAT_ENTER_CS: usize = 0x00DB_80CC;
const IAT_LEAVE_CS: usize = 0x00DB_80D0;
const IAT_SET_EVENT: usize = 0x00DB_81DC;
const IAT_WAIT_FOR_SINGLE_OBJECT: usize = 0x00DB_8100;
const IAT_SLEEP: usize = 0x00DB_80B8;
const DEBUG_SRC: usize = 0x0252_9E24;
const DEBUG_DST: usize = 0x0252_9E28;
const FORCE_FREE_MODE_2: usize = 0x0252_9DD5;
const FORCE_FREE_MODE_1: usize = 0x0252_9DD6;
const FREE_POLLS: usize = 0x0252_9DD7;
const PLAYER_CVTF_FROM_MEMORY: usize = 0x0252_9DDB;
const PLAYER_CVTF_DATA: usize = 0x0222_3CC0;
const RAW_OPEN: usize = 0x0252_9DDA;
const STR_PLAYER_CVTF: usize = 0x00E4_4150;
const STR_NOT_ENOUGH_MEMORY: usize = 0x00E4_4168;
const STR_RB: usize = 0x00DD_88E4;
const ATTACH_WAIT_EVENT: usize = 0x00A7_3240; // EAX = mgr, stdcall (request)
const WAIT_DONE: usize = 0x00A7_3900; // stdcall (mgr, request)
const READ_CACHE_LOOKUP: usize = 0x00A7_4A40; // ESI = request+4 -> AL
const READ_CACHE_STORE: usize = 0x00A7_4AB0; // EAX = request+4
const FILE_OPEN: usize = 0x00BF_DC20; // ECX = name, cdecl (mode, flag) -> handle
const FILE_READ: usize = 0x00BF_E170; // EAX = size, cdecl (dest, handle) -> bytes
const FILE_CLOSE: usize = 0x00BF_DF30; // cdecl (handle)
const FILE_ABORT_READ: usize = 0x00BF_DAF0; // EAX = handle
const FILE_AFTER_READ: usize = 0x00BF_E600; // EAX = handle
const OS_FILE_SEEK: usize = 0x00C1_8590; // EAX = 0, cdecl (os handle, offset) -> AL
const PATH_CHANGE_EXTENSION: usize = 0x00BF_F930; // ECX = src, cdecl (out, size, ext)
const CRT_STRICMP: usize = 0x00C9_9030;
const CRT_MEMCPY: usize = 0x00CA_0850;

// Request fields.
const R_SEQ: usize = 0x00;
const R_NAME: usize = 0x04;
const R_SUFFIX: usize = 0x08;
const R_OFFSET: usize = 0x0C;
const R_SIZE: usize = 0x10;
const R_DEST: usize = 0x14;
const R_PRIO: usize = 0x18;
const R_FILE: usize = 0x24;
const R_READ: usize = 0x28;
const R_STATE: usize = 0x2C;
const R_WAITER: usize = 0x30;
const R_ATTACHED: usize = 0x34;
const R_RELEASED: usize = 0x38;
const R_MGR: usize = 0x3C;
const R_PREV: usize = 0x40;
const R_NEXT: usize = 0x44;
// Manager fields.
const M_EVENT: usize = 0x00;
const M_LOCK: usize = 0x08;
const M_STOP: usize = 0x2C;
const M_LOW_PRIO_OK: usize = 0x2D;
const M_PAUSED: usize = 0x2F;
const M_FREE: usize = 0x30;
const M_PENDING: usize = 0x34;
const M_FINISHED: usize = 0x38;
const M_CURRENT: usize = 0x3C;

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}
unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}

unsafe fn api(iat: usize) -> extern "system" fn(u32) -> u32 {
    unsafe { std::mem::transmute(rd(iat) as usize) }
}
unsafe fn lock(mgr: usize) {
    unsafe { api(IAT_ENTER_CS)((mgr + M_LOCK) as u32) };
}
unsafe fn unlock(mgr: usize) {
    unsafe { api(IAT_LEAVE_CS)((mgr + M_LOCK) as u32) };
}
unsafe fn set_event(handle: u32) {
    unsafe { api(IAT_SET_EVENT)(handle) };
}

/// The original's debug check store.
unsafe fn debug_check() {
    unsafe { wr(DEBUG_DST, rd(rd(DEBUG_SRC) as usize)) }
}

/// Call a game function with EAX/ECX/EDX/ESI/EDI set and stack arguments,
/// saving EBX/ESI/EDI/EBP; ESP is restored afterwards (cdecl or callee-clean).
unsafe fn game_call(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> (u32, u32) {
    let block: [u32; 8] = [f as u32, eax, ecx, edx, esi, args.as_ptr() as u32, args.len() as u32, edi];
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
            "mov edi, [ebx + 28]",
            "call dword ptr [ebx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("edx") block.as_ptr() => hi, lateout("eax") lo, lateout("ecx") _,
            clobber_abi("C"),
        );
    }
    (lo, hi)
}

// --- Lists --------------------------------------------------------------------

/// `0xA73110` (ECX = head, EDX = node -> AL): is the node on that list.
unsafe fn on_list(head: u32, n: usize) -> bool {
    unsafe {
        let mut p = head as usize;
        while p != 0 {
            if p == n {
                return true;
            }
            p = rd(p + R_NEXT) as usize;
            if p == head as usize {
                break;
            }
        }
        false
    }
}

/// Unlink `n` from the list whose head pointer is at `head` and clear its links.
unsafe fn unlink(head: usize, n: usize) {
    unsafe {
        if rd(head) as usize == n && rd(n + R_NEXT) as usize == n {
            wr(n + R_NEXT, 0);
            wr(n + R_PREV, 0);
            wr(head, 0);
            return;
        }
        if rd(head) as usize == n {
            wr(head, rd(n + R_NEXT));
        }
        let (prev, next) = (rd(n + R_PREV), rd(n + R_NEXT));
        if prev != 0 {
            wr(prev as usize + R_NEXT, next);
        }
        if next != 0 {
            wr(next as usize + R_PREV, prev);
        }
        wr(n + R_NEXT, 0);
        wr(n + R_PREV, 0);
    }
}

/// Append `n` at the tail of the list whose head pointer is at `head`.
unsafe fn append(head: usize, n: usize) {
    unsafe {
        let h = rd(head) as usize;
        if h == 0 {
            wr(n + R_NEXT, n as u32);
            wr(n + R_PREV, n as u32);
            wr(head, n as u32);
        } else {
            let tail = rd(h + R_PREV) as usize;
            wr(n + R_PREV, tail as u32);
            wr(n + R_NEXT, h as u32);
            wr(tail + R_NEXT, n as u32);
            wr(h + R_PREV, n as u32);
        }
    }
}

fn in_flight(state: u32) -> bool {
    matches!(state, 2..=4)
}

// --- The API -----------------------------------------------------------------------

/// `async_io_set_state` (0xA739A0; EAX = request, stdcall (mgr, state)): set the
/// state; a state that is not in flight signals the one-shot waiter (then cleared)
/// and the attached event.
unsafe fn set_state(req: usize, mgr: usize, state: u32) {
    unsafe {
        lock(mgr);
        wr(req + R_STATE, state);
        let busy = in_flight(rd(req + R_STATE));
        let waiter = rd(req + R_WAITER);
        if waiter != 0 && !busy {
            set_event(rd(waiter as usize));
            wr(req + R_WAITER, 0);
        }
        let attached = rd(req + R_ATTACHED);
        if attached != 0 && !busy {
            set_event(rd(attached as usize));
        }
        unlock(mgr);
    }
}

/// `hdd_finish_current` (0xA73150; ESI = request, EDI = mgr, stdcall (state)):
/// set the final state; a request its owner already released goes straight back
/// to the free list, otherwise onto the finished list.
unsafe fn finish(req: usize, mgr: usize, state: u32) {
    unsafe {
        lock(mgr);
        if rd(mgr + M_CURRENT) as usize != req {
            debug_check();
        }
        set_state_hook(req, mgr, state);
        if rb(req + R_RELEASED) != 0 {
            wr(req + R_SEQ, rd(req + R_SEQ).wrapping_add(1));
            wr(req + R_STATE, 1);
            append(mgr + M_FREE, req);
        } else {
            append(mgr + M_FINISHED, req);
        }
        unlock(mgr);
    }
}

/// `async_io_submit` (0xA73290; stdcall (mgr, desc[8], u8 attach_event) ->
/// EDX:EAX = sequence:request): take a free record (no empty check, as the
/// original), copy the description in, queue it by priority and wake the thread.
unsafe fn submit(mgr: usize, desc: usize, attach: u32) -> (u32, u32) {
    unsafe {
        lock(mgr);
        let n = rd(mgr + M_FREE) as usize;
        if n == 0 {
            debug_check();
        }
        unlink(mgr + M_FREE, n);
        wr(n + R_SEQ, rd(n + R_SEQ).wrapping_add(1));
        if rd(n + R_STATE) != 1 {
            debug_check();
        }
        wr(n + R_FILE, 0);
        wr(n + R_WAITER, 0);
        wr(n + R_READ, 0);
        wr(n + R_MGR, 0);
        wb(n + R_RELEASED, 0);
        wr(n + R_STATE, 1);
        std::ptr::copy_nonoverlapping(desc as *const u32, (n + R_NAME) as *mut u32, 8);
        wr(n + R_MGR, mgr as u32);
        set_state_hook(n, mgr, 2);
        if rd(n + R_ATTACHED) != 0 {
            debug_check();
        }
        if attach as u8 != 0 {
            game_call(ATTACH_WAIT_EVENT, mgr as u32, 0, 0, 0, 0, &[n as u32]);
        }
        let head = rd(mgr + M_PENDING) as usize;
        let prio = rb(n + R_PRIO);
        let mut before = 0usize;
        let mut p = head;
        while p != 0 {
            if rb(p + R_PRIO) < prio {
                before = p;
                break;
            }
            p = rd(p + R_NEXT) as usize;
            if p == head {
                break;
            }
        }
        if before != 0 {
            let prev = rd(before + R_PREV) as usize;
            wr(prev + R_NEXT, n as u32);
            wr(n + R_PREV, prev as u32);
            wr(before + R_PREV, n as u32);
            wr(n + R_NEXT, before as u32);
            if rd(mgr + M_PENDING) as usize == before {
                wr(mgr + M_PENDING, n as u32);
            }
        } else {
            if head != 0 {
                let tail = rd(head + R_PREV) as usize;
                if tail != 0 && rb(tail + R_PRIO) < prio {
                    debug_check();
                }
            }
            append(mgr + M_PENDING, n);
        }
        if !on_list(rd(mgr + M_PENDING), n) {
            debug_check();
        }
        set_event(rd(mgr + M_EVENT));
        let seq = rd(n + R_SEQ);
        unlock(mgr);
        (n as u32, seq)
    }
}

/// `async_io_cancel_impl` (0xA73430; EDI = mgr, stdcall (request, sequence)): a
/// queued request moves to the finished list as cancelled (5); the one being
/// read is marked cancelling (4) and its file read aborted.
unsafe fn cancel_impl(mgr: usize, req: usize, seq: u32) {
    unsafe {
        if req == 0 {
            return;
        }
        lock(mgr);
        if seq != rd(req + R_SEQ) {
            debug_check();
        }
        match rd(req + R_STATE) {
            1 => {
                if !on_list(rd(mgr + M_FREE), req) {
                    debug_check();
                }
            }
            2 => {
                if !on_list(rd(mgr + M_PENDING), req) {
                    debug_check();
                }
                unlink(mgr + M_PENDING, req);
                append(mgr + M_FINISHED, req);
                set_state_hook(req, mgr, 5);
            }
            3 => {
                if rd(mgr + M_CURRENT) as usize != req {
                    debug_check();
                }
                if rd(req + R_FILE) != 0 {
                    game_call(FILE_ABORT_READ, rd(req + R_FILE), 0, 0, 0, 0, &[]);
                }
                set_state_hook(req, mgr, 4);
            }
            _ => {}
        }
        unlock(mgr);
    }
}

/// `async_io_free` (0xA73650; stdcall (mgr, request, sequence, mode)): return a
/// record to the free list. While it is still in flight, modes 0/1 only mark it
/// released (the thread recycles it when done) and mode 2 waits for it.
unsafe fn free(mgr: usize, req: usize, seq: u32, mut mode: u32) {
    unsafe {
        if req == 0 {
            return;
        }
        if seq != rd(req + R_SEQ) {
            debug_check();
        }
        if rb(req + R_RELEASED) != 0 {
            debug_check();
        }
        lock(mgr);
        if rb(FORCE_FREE_MODE_2) != 0 {
            mode = 2;
        }
        if rb(FORCE_FREE_MODE_1) != 0 {
            mode = 1;
        }
        let busy = in_flight(rd(req + R_STATE));
        if rb(FREE_POLLS) != 0 {
            while in_flight(rd(req + R_STATE)) {
                unlock(mgr);
                api(IAT_SLEEP)(1);
                lock(mgr);
            }
        } else if busy {
            if !on_list(rd(mgr + M_PENDING), req) && rd(mgr + M_CURRENT) as usize != req {
                debug_check();
            }
            match mode {
                0 | 1 => {
                    wb(req + R_RELEASED, 1);
                    unlock(mgr);
                    return;
                }
                2 => {
                    unlock(mgr);
                    game_call(WAIT_DONE, 0, 0, 0, 0, 0, &[mgr as u32, req as u32]);
                    lock(mgr);
                }
                _ => {}
            }
        }
        if !on_list(rd(mgr + M_FINISHED), req) {
            debug_check();
        }
        unlink(mgr + M_FINISHED, req);
        wr(req + R_SEQ, rd(req + R_SEQ).wrapping_add(1));
        wr(req + R_STATE, 1);
        append(mgr + M_FREE, req);
        wb(req + R_RELEASED, 1);
        unlock(mgr);
    }
}

/// `async_io_status` (0xA741A0): 1 queued, 2 being read/cancelling, 3 done, 4
/// cancelled, 5 failed, 0 otherwise.
unsafe fn status(req: usize, _seq: u32) -> u32 {
    unsafe {
        if req == 0 {
            return 0;
        }
        match rd(req + R_STATE) {
            2 => 1,
            3 | 4 => 2,
            6 => 3,
            5 => 4,
            7..=9 => 5,
            _ => 0,
        }
    }
}

/// `async_io_error_code` (0xA74340): 11 for no request, 10 if the name is "not
/// enough memory", else the state (an index into the state-name table).
unsafe fn error_code(req: usize) -> u32 {
    unsafe {
        if req == 0 {
            return 11;
        }
        let r = game_call(CRT_STRICMP, 0, 0, 0, 0, 0, &[rd(req + R_NAME), STR_NOT_ENOUGH_MEMORY as u32]).0;
        if r == 0 { 10 } else { rd(req + R_STATE) }
    }
}

// --- The service loop ----------------------------------------------------------------

/// `hdd_service` (0xA73C50; ECX = mgr -> AL), run by the thread with Sleep(5)
/// between calls until it returns false (stop). Waits for work, then services
/// the pending list in priority order until it is empty, its head is a low
/// priority read (<= 0x21) while those are held back, or the scheduler pauses.
/// Each request: a cancel before it starts gives 5; the debug player.cvtf copy or
/// a read-cache hit give 6; otherwise open (7 on failure), seek to the start
/// offset, read, close, and the handle's error picks 6, 5 (error 3) or 9.
unsafe fn service(mgr: usize) -> bool {
    unsafe {
        let wait: extern "system" fn(u32, u32) -> u32 = std::mem::transmute(rd(IAT_WAIT_FOR_SINGLE_OBJECT) as usize);
        wait(rd(mgr + M_EVENT), u32::MAX);
        if rb(mgr + M_PAUSED) != 0 {
            return true;
        }
        if rb(mgr + M_STOP) != 0 {
            return false;
        }
        'service: loop {
            if rb(mgr + M_PAUSED) != 0 {
                return true;
            }
            lock(mgr);
            let cur = rd(mgr + M_PENDING) as usize;
            wr(mgr + M_CURRENT, cur as u32);
            if cur == 0 || (rb(cur + R_PRIO) <= 0x21 && rb(mgr + M_LOW_PRIO_OK) == 0) {
                unlock(mgr);
                return true;
            }
            unlink(mgr + M_PENDING, cur);
            set_state_hook(cur, mgr, 3);
            unlock(mgr);

            let mut buf = [0u8; 0x40];
            let mut name = rd(cur + R_NAME);
            if rd(cur + R_SUFFIX) != 0 {
                game_call(
                    PATH_CHANGE_EXTENSION,
                    0,
                    name,
                    0,
                    0,
                    0,
                    &[buf.as_mut_ptr() as u32, 0x40, rd(cur + R_SUFFIX)],
                );
                name = buf.as_ptr() as u32;
            }
            let state = 'read: {
                if rd(cur + R_STATE) == 4 {
                    break 'read 5;
                }
                let raw = rd(cur + R_NAME);
                if rb(PLAYER_CVTF_FROM_MEMORY) != 0
                    && raw != 0
                    && game_call(CRT_STRICMP, 0, 0, 0, 0, 0, &[raw, STR_PLAYER_CVTF as u32]).0 == 0
                {
                    game_call(
                        CRT_MEMCPY,
                        0,
                        0,
                        0,
                        0,
                        0,
                        &[rd(cur + R_DEST), PLAYER_CVTF_DATA as u32, rd(cur + R_SIZE)],
                    );
                    break 'read 6;
                }
                if game_call(READ_CACHE_LOOKUP, 0, 0, 0, (cur + R_NAME) as u32, 0, &[]).0 as u8 != 0 {
                    break 'read 6;
                }
                let flag = (rb(RAW_OPEN) == 0) as u32;
                let h = game_call(FILE_OPEN, 0, name, flag, 0, 0, &[STR_RB as u32, flag]).0 as usize;
                wr(cur + R_FILE, h as u32);
                if h == 0 {
                    break 'read 7;
                }
                if rd(cur + R_STATE) == 4 {
                    lock(mgr);
                    game_call(FILE_CLOSE, 0, 0, 0, 0, 0, &[h as u32]);
                    wr(cur + R_FILE, 0);
                    finish_hook_call(cur, mgr, 5);
                    unlock(mgr);
                    if rb(mgr + M_STOP) != 0 {
                        return false;
                    }
                    continue 'service;
                }
                let offset = rd(cur + R_OFFSET);
                if offset != 0 && (offset as i32) >= 0 && !(rd(h + 0x154) == 1 && offset > rd(h + 0x160)) {
                    match rd(h + 0x148) {
                        0 => {
                            if rd(h + 0x150) == 0
                                && game_call(OS_FILE_SEEK, 0, 0, 0, 0, 0, &[rd(h + 0x14C), offset]).0 as u8 != 0
                            {
                                wr(h + 0x15C, offset);
                            }
                        }
                        2 => wr(h + 0x15C, offset),
                        _ => {}
                    }
                }
                let read = game_call(FILE_READ, rd(cur + R_SIZE), 0, 0, 0, 0, &[rd(cur + R_DEST), h as u32]).0;
                wr(cur + R_READ, read);
                game_call(READ_CACHE_STORE, (cur + R_NAME) as u32, 0, 0, 0, 0, &[]);
                if rb(RAW_OPEN) == 0 {
                    game_call(FILE_AFTER_READ, h as u32, 0, 0, 0, 0, &[]);
                }
                lock(mgr);
                game_call(FILE_CLOSE, 0, 0, 0, 0, 0, &[h as u32]);
                let err = rd(h + 0x158);
                wr(cur + R_FILE, 0);
                unlock(mgr);
                match err {
                    0 => 6,
                    3 => 5,
                    _ => 9,
                }
            };
            finish_hook_call(cur, mgr, state);
            if rb(mgr + M_STOP) != 0 {
                return false;
            }
        }
    }
}

// --- Hooks ----------------------------------------------------------------------------

const SET_STATE: usize = 0;
const FINISH: usize = 1;
const SUBMIT: usize = 2;
const CANCEL_IMPL: usize = 3;
const FREE: usize = 4;
const STATUS: usize = 5;
const CANCEL: usize = 6;
const RELEASE: usize = 7;
const ERROR_CODE: usize = 8;
const BYTES_READ: usize = 9;
const SERVICE: usize = 10;
const COUNT: usize = 11;

const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("async_io_set_state", 0x00A7_39A0, &[0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C]),
    ("hdd_finish_current", 0x00A7_3150, &[0x53, 0x8D, 0x5F, 0x08, 0x53]),
    ("async_io_submit", 0x00A7_3290, &[0x53, 0x8B, 0x5C, 0x24, 0x08]),
    ("async_io_cancel_impl", 0x00A7_3430, &[0x51, 0x53, 0x56, 0x8B, 0x74, 0x24, 0x10]),
    ("async_io_free", 0x00A7_3650, &[0x56, 0x8B, 0x74, 0x24, 0x0C]),
    ("async_io_status", 0x00A7_41A0, &[0x53, 0x8B, 0x5C, 0x24, 0x08]),
    ("async_io_cancel", 0x00A7_4150, &[0x8B, 0x54, 0x24, 0x04, 0x53]),
    ("async_io_release", 0x00A7_4270, &[0x8B, 0x4C, 0x24, 0x04, 0x53]),
    ("async_io_error_code", 0x00A7_4340, &[0x56, 0x8B, 0x74, 0x24, 0x08]),
    ("async_io_bytes_read", 0x00A7_35E0, &[0x53, 0x8B, 0x5C, 0x24, 0x08]),
    ("hdd_service", 0x00A7_3C50, &[0x83, 0xEC, 0x54, 0xA1, 0x50, 0x0B, 0xE8, 0x00]),
];

static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

static EVENTS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

fn event(e: String) {
    *EVENTS.lock().unwrap_or_else(|p| p.into_inner()).entry(e).or_insert(0) += 1;
}

unsafe fn cstr(p: u32) -> String {
    unsafe {
        if p == 0 {
            return "-".into();
        }
        let b = std::slice::from_raw_parts(p as *const u8, 64);
        let n = b.iter().position(|&c| c == 0).unwrap_or(64);
        String::from_utf8_lossy(&b[..n]).into_owned()
    }
}

/// A request for the event log: name (+ suffix), priority and size.
unsafe fn describe(req: usize) -> String {
    unsafe {
        let suffix = rd(req + R_SUFFIX);
        let name = cstr(rd(req + R_NAME));
        let name = if suffix != 0 { format!("{name}{}", cstr(suffix)) } else { name };
        format!("{name} p{:x} n{:x}", rb(req + R_PRIO), rd(req + R_SIZE))
    }
}

/// EAX = request; stdcall (mgr, state).
#[unsafe(naked)]
extern "C" fn set_state_entry() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 8]",
        "push dword ptr [esp + 8]",
        "push eax",
        "call {h}",
        "add esp, 12",
        "ret 8",
        h = sym set_state_hook_c,
    )
}

extern "C" fn set_state_hook_c(req: u32, mgr: u32, state: u32) {
    unsafe { set_state_hook(req as usize, mgr as usize, state) }
}

unsafe fn set_state_hook(req: usize, mgr: usize, state: u32) {
    CALLS[SET_STATE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if IO_RUST {
            set_state(req, mgr, state);
        } else {
            game_call(orig(SET_STATE), req as u32, 0, 0, 0, 0, &[mgr as u32, state]);
        }
    }
}

/// ESI = request, EDI = mgr; stdcall (state).
#[unsafe(naked)]
extern "C" fn finish_entry() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 4]",
        "push edi",
        "push esi",
        "call {h}",
        "add esp, 12",
        "ret 4",
        h = sym finish_hook_c,
    )
}

extern "C" fn finish_hook_c(req: u32, mgr: u32, state: u32) {
    unsafe { finish_hook_call(req as usize, mgr as usize, state) }
}

unsafe fn finish_hook_call(req: usize, mgr: usize, state: u32) {
    CALLS[FINISH].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let e = describe(req);
        if IO_RUST {
            finish(req, mgr, state);
        } else {
            game_call(orig(FINISH), 0, 0, 0, req as u32, mgr as u32, &[state]);
        }
        let read = rd(req + R_READ);
        event(format!("io {e} -> s{state} read {read:x}"));
    }
}

extern "stdcall" fn submit_hook(mgr: u32, desc: u32, attach: u32) -> u64 {
    CALLS[SUBMIT].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let (req, seq) = if IO_RUST {
            submit(mgr as usize, desc as usize, attach)
        } else {
            let f: extern "stdcall" fn(u32, u32, u32) -> u64 = std::mem::transmute(orig(SUBMIT));
            let r = f(mgr, desc, attach);
            (r as u32, (r >> 32) as u32)
        };
        (seq as u64) << 32 | req as u64
    }
}

/// EDI = mgr; stdcall (request, sequence).
#[unsafe(naked)]
extern "C" fn cancel_impl_entry() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 8]",
        "push dword ptr [esp + 8]",
        "push edi",
        "call {h}",
        "add esp, 12",
        "ret 8",
        h = sym cancel_impl_hook,
    )
}

extern "C" fn cancel_impl_hook(mgr: u32, req: u32, seq: u32) {
    CALLS[CANCEL_IMPL].fetch_add(1, Ordering::Relaxed);
    unsafe {
        let before = if req != 0 { rd(req as usize + R_STATE) } else { 0 };
        if IO_RUST {
            cancel_impl(mgr as usize, req as usize, seq);
        } else {
            game_call(orig(CANCEL_IMPL), 0, 0, 0, 0, mgr, &[req, seq]);
        }
        if req != 0 {
            event(format!("io_cancel {} s{before} -> s{}", describe(req as usize), rd(req as usize + R_STATE)));
        }
    }
}

extern "stdcall" fn free_hook(mgr: u32, req: u32, seq: u32, mode: u32) {
    CALLS[FREE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if IO_RUST {
            free(mgr as usize, req as usize, seq, mode);
        } else {
            let f: extern "stdcall" fn(u32, u32, u32, u32) = std::mem::transmute(orig(FREE));
            f(mgr, req, seq, mode);
        }
    }
}

/// Pure query: shadow-checked against the original on every call.
extern "C" fn status_hook(req: u32, seq: u32) -> u32 {
    CALLS[STATUS].fetch_add(1, Ordering::Relaxed);
    let rust = unsafe { status(req as usize, seq) };
    let f: extern "C" fn(u32, u32) -> u32 = unsafe { std::mem::transmute(orig(STATUS)) };
    let o = f(req, seq);
    if o != rust && o != unsafe { status(req as usize, seq) } {
        if MISMATCHES[STATUS].fetch_add(1, Ordering::Relaxed) < 5 {
            log(&format!("hddio status mismatch: request {req:#x}: original {o}, rust {rust}"));
        }
        return o;
    }
    rust
}

/// `async_io_cancel` (0xA74150, cdecl (request, sequence)): cancel_impl with the request's manager.
extern "C" fn cancel_hook(req: u32, seq: u32) {
    CALLS[CANCEL].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if IO_RUST {
            if req == 0 || seq != rd(req as usize + R_SEQ) {
                debug_check();
            }
            cancel_impl_hook(rd(req as usize + R_MGR), req, seq);
        } else {
            let f: extern "C" fn(u32, u32) = std::mem::transmute(orig(CANCEL));
            f(req, seq);
        }
    }
}

/// `async_io_release` (0xA74270, cdecl (request, sequence, mode)): free with the request's manager.
extern "C" fn release_hook(req: u32, seq: u32, mode: u32) {
    CALLS[RELEASE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if IO_RUST {
            if req == 0 || seq != rd(req as usize + R_SEQ) {
                debug_check();
            }
            free_hook(rd(req as usize + R_MGR), req, seq, mode);
        } else {
            let f: extern "C" fn(u32, u32, u32) = std::mem::transmute(orig(RELEASE));
            f(req, seq, mode);
        }
    }
}

/// Pure query: shadow-checked.
extern "C" fn error_code_hook(req: u32, seq: u32) -> u32 {
    CALLS[ERROR_CODE].fetch_add(1, Ordering::Relaxed);
    let rust = unsafe { error_code(req as usize) };
    let f: extern "C" fn(u32, u32) -> u32 = unsafe { std::mem::transmute(orig(ERROR_CODE)) };
    let o = f(req, seq);
    if o != rust && o != unsafe { error_code(req as usize) } {
        if MISMATCHES[ERROR_CODE].fetch_add(1, Ordering::Relaxed) < 5 {
            log(&format!("hddio error_code mismatch: request {req:#x}: original {o}, rust {rust}"));
        }
        return o;
    }
    rust
}

/// `async_io_bytes_read` (0xA735E0, stdcall (mgr, request, sequence)): takes the
/// lock and reads the request's file, but always returns 0 on PC.
extern "stdcall" fn bytes_read_hook(mgr: u32, req: u32, seq: u32) -> u32 {
    CALLS[BYTES_READ].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if !IO_RUST {
            let f: extern "stdcall" fn(u32, u32, u32) -> u32 = std::mem::transmute(orig(BYTES_READ));
            return f(mgr, req, seq);
        }
        lock(mgr as usize);
        if req != 0 && seq != rd(req as usize + R_SEQ) {
            debug_check();
        }
        unlock(mgr as usize);
        0
    }
}

extern "thiscall" fn service_hook(mgr: u32) -> u32 {
    CALLS[SERVICE].fetch_add(1, Ordering::Relaxed);
    unsafe {
        if IO_RUST {
            service(mgr as usize) as u32
        } else {
            let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(orig(SERVICE));
            f(mgr) & 0xFF
        }
    }
}

fn hook_address(i: usize) -> usize {
    match i {
        SET_STATE => set_state_entry as extern "C" fn() as usize,
        FINISH => finish_entry as extern "C" fn() as usize,
        SUBMIT => submit_hook as extern "stdcall" fn(u32, u32, u32) -> u64 as usize,
        CANCEL_IMPL => cancel_impl_entry as extern "C" fn() as usize,
        FREE => free_hook as extern "stdcall" fn(u32, u32, u32, u32) as usize,
        STATUS => status_hook as extern "C" fn(u32, u32) -> u32 as usize,
        CANCEL => cancel_hook as extern "C" fn(u32, u32) as usize,
        RELEASE => release_hook as extern "C" fn(u32, u32, u32) as usize,
        ERROR_CODE => error_code_hook as extern "C" fn(u32, u32) -> u32 as usize,
        BYTES_READ => bytes_read_hook as extern "stdcall" fn(u32, u32, u32) -> u32 as usize,
        SERVICE => service_hook as extern "thiscall" fn(u32) -> u32 as usize,
        _ => unreachable!(),
    }
}

/// Detour the scheduler. The hdd thread is normally blocked inside hdd_service's
/// wait, past the patched prologue; its next call goes to the port.
pub fn install() {
    let mut ok = 0;
    for (i, &(name, addr, prologue)) in PORTS.iter().enumerate() {
        match unsafe { detour::install(addr, prologue, hook_address(i)) } {
            Ok(tramp) => {
                ORIG[i].store(tramp, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("hddio: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    let mode = if IO_RUST { "Rust" } else { "original, observed" };
    log(&format!("replaced {ok}/{COUNT} read scheduler functions ({mode}; status/error code shadow-checked)"));
}

pub fn summary() -> String {
    let events = EVENTS.lock().unwrap_or_else(|p| p.into_inner());
    let text: String = events.iter().map(|(e, n)| format!("{e}\t{n}\n")).collect();
    let _ = std::fs::write("sr2-io.log", text);
    let parts: Vec<String> = PORTS
        .iter()
        .enumerate()
        .filter_map(|(i, &(name, ..))| {
            let (c, m) = (CALLS[i].load(Ordering::Relaxed), MISMATCHES[i].load(Ordering::Relaxed));
            (c > 0).then(|| format!("{name} {c}{}", if m > 0 { format!(" ({m} MISMATCH)") } else { String::new() }))
        })
        .collect();
    format!("hddio live: {}", parts.join(", "))
}
