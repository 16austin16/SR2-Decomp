//! Crash logger: a vectored exception handler that logs the first few access
//! violations (address, registers, and the stack's likely return addresses into the
//! exe or this DLL), then lets the exception continue as normal.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::log;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: extern "system" fn(*const [usize; 2]) -> i32) -> usize;
    fn IsBadReadPtr(p: *const u8, n: usize) -> i32;
}

static LOGGED: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" {
    static __ImageBase: u8;
}

/// This DLL's base and end (SizeOfImage from its PE header).
fn dll_range() -> (usize, usize) {
    unsafe {
        let base = &__ImageBase as *const u8 as usize;
        let pe = base + *((base + 0x3C) as *const u32) as usize;
        let size = *((pe + 0x50) as *const u32) as usize;
        (base, base + size)
    }
}

/// The most recent port entries made by this thread, newest first (see portlib's trace ring).
fn recent_ports() -> String {
    unsafe {
        let tid = { let t: u32; std::arch::asm!("mov {0}, fs:[0x24]", out(reg) t); t };
        let n = crate::portlib::TRACE_LEN;
        let end = crate::portlib::TRACE_IDX.load(Ordering::Relaxed) as usize;
        let ring = std::ptr::addr_of!(crate::portlib::TRACE_RING);
        let names = match crate::portlib::TRACE_NAMES.try_lock() {
            Ok(v) => v.clone(),
            Err(_) => Vec::new(),
        };
        let mut out = Vec::new();
        for k in 1..=n.min(end) {
            let e = (*ring)[(end - k) % n];
            if e[0] == 0 || e[1] != tid {
                continue;
            }
            let id = (e[0] - 1) as usize;
            let s = match names.get(id) {
                Some((nm, a)) => format!("{nm}@{a:#x}"),
                None => format!("port#{id}"),
            };
            if out.last() != Some(&s) {
                out.push(s);
            }
            if out.len() >= 40 {
                break;
            }
        }
        if out.is_empty() { "(none on this thread)".to_string() } else { out.join(" <- ") }
    }
}

extern "system" fn handler(info: *const [usize; 2]) -> i32 {
    unsafe {
        let [record, context] = *info;
        let code = *(record as *const u32);
        if code != 0xC000_0005 || LOGGED.fetch_add(1, Ordering::Relaxed) >= 3 {
            return 0; // EXCEPTION_CONTINUE_SEARCH
        }
        let r = |o: usize| *((context + o) as *const u32);
        let (eip, esp) = (r(0xB8), r(0xC4));
        let target = *((record + 0x18) as *const u32); // ExceptionInformation[1]
        let (lo, hi) = dll_range();
        let mut stack = Vec::new();
        for i in 0..1024usize {
            let p = esp as usize + i * 4;
            if IsBadReadPtr(p as *const u8, 4) != 0 {
                break;
            }
            let v = *(p as *const u32) as usize;
            if (0x0040_1000..0x00DB_0000).contains(&v) || (lo..hi).contains(&v) {
                stack.push(format!("+{:#x}:{v:#x}", i * 4));
            }
        }
        log(&format!(
            "CRASH: access violation at eip {eip:#x} (address {target:#x}) eax {:#x} ebx {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} ebp {:#x} esp {esp:#x} | dll {lo:#x} | stack code refs: {}",
            r(0xB0),
            r(0xA4),
            r(0xAC),
            r(0xA8),
            r(0xA0),
            r(0x9C),
            r(0xB4),
            stack.join(" ")
        ));
        log(&format!("CRASH: last ports entered on this thread, newest first: {}", recent_ports()));
        log("CRASH: resolve the addresses with: python tools/crash_report.py <game folder>/sr2-hook.log");
        crate::missionlog::flush("a crash");
    }
    0
}

pub fn install() {
    unsafe { AddVectoredExceptionHandler(1, handler) };
}
