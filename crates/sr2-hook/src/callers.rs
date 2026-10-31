//! Caller survey for naming virtual methods (temporary instrumentation, feature
//! `caller-survey`).
//!
//! Every target vtable cell (caller_targets.rs: the still-unnamed `vfN` slots of
//! game classes) is pointed at a stub that records the return address of each call
//! and then jumps to the original method:
//!
//! ```text
//! pushfd; pushad
//! push dword [esp+0x24]      ; the caller's return address
//! push <target index>
//! call record                ; cdecl, only touches registers pushad saved
//! add esp, 8
//! popad; popfd
//! jmp dword [cell]           ; the original method
//! ```
//!
//! Registers, flags and the stack are exactly as the caller left them when the
//! original runs, so every calling convention works. A cell whose current value
//! is not the original function (already replaced by a port) is left alone.
//! A thread rewrites `sr2-callers.log` every 30 s; tools/caller_report.py turns it
//! into per-slot caller contexts.

use std::ffi::c_void;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
};

use crate::caller_targets::TARGETS;
use crate::log;

const CALLERS: usize = 8;
const STUB: usize = 32;
const LOG_FILE: &str = "sr2-callers.log";
const LOG_EVERY_SECS: u64 = 30;

struct Entry {
    total: AtomicU32,
    other: AtomicU32,
    ret: [AtomicU32; CALLERS],
    count: [AtomicU32; CALLERS],
}

static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();

extern "C" fn record(index: u32, ret: u32) {
    let Some(e) = TABLE.get().and_then(|t| t.get(index as usize)) else { return };
    e.total.fetch_add(1, Ordering::Relaxed);
    for i in 0..CALLERS {
        let cur = e.ret[i].load(Ordering::Relaxed);
        if cur == ret {
            e.count[i].fetch_add(1, Ordering::Relaxed);
            return;
        }
        if cur == 0 {
            match e.ret[i].compare_exchange(0, ret, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    e.count[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(won) if won == ret => {
                    e.count[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => {}
            }
        }
    }
    e.other.fetch_add(1, Ordering::Relaxed);
}

pub fn install() {
    TABLE.get_or_init(|| {
        TARGETS
            .iter()
            .map(|_| Entry {
                total: AtomicU32::new(0),
                other: AtomicU32::new(0),
                ret: std::array::from_fn(|_| AtomicU32::new(0)),
                count: std::array::from_fn(|_| AtomicU32::new(0)),
            })
            .collect()
    });
    let mem = unsafe {
        VirtualAlloc(std::ptr::null(), TARGETS.len() * STUB, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
    } as *mut u8;
    if mem.is_null() {
        log("callers: VirtualAlloc failed");
        return;
    }
    let record_addr = record as *const () as usize;
    let (mut wrapped, mut skipped) = (0, 0);
    for (i, &(cell, _slot, original)) in TARGETS.iter().enumerate() {
        let cell = cell as usize;
        if unsafe { *(cell as *const u32) } != original {
            skipped += 1;
            continue;
        }
        unsafe {
            let stub = mem.add(i * STUB);
            let jmp_cell = stub.add(28) as *mut u32;
            *jmp_cell = original;
            let call_at = stub as usize + 11; // E8 rel32 follows 1+1+4+5 bytes
            let rel = (record_addr as i64 - (call_at as i64 + 5)) as i32;
            let mut code = Vec::with_capacity(27);
            code.extend_from_slice(&[0x9C, 0x60, 0xFF, 0x74, 0x24, 0x24, 0x68]);
            code.extend_from_slice(&(i as u32).to_le_bytes());
            code.push(0xE8);
            code.extend_from_slice(&rel.to_le_bytes());
            code.extend_from_slice(&[0x83, 0xC4, 0x08, 0x61, 0x9D, 0xFF, 0x25]);
            code.extend_from_slice(&(jmp_cell as u32).to_le_bytes());
            std::ptr::copy_nonoverlapping(code.as_ptr(), stub, code.len());
            let mut old = 0u32;
            if VirtualProtect(cell as *const c_void, 4, PAGE_READWRITE, &mut old) == 0 {
                skipped += 1;
                continue;
            }
            *(cell as *mut u32) = stub as u32;
            VirtualProtect(cell as *const c_void, 4, old, &mut old);
        }
        wrapped += 1;
    }
    log(&format!("callers: wrapped {wrapped} vtable cells ({skipped} skipped: already replaced)"));
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(LOG_EVERY_SECS));
        write_log();
    });
}

fn write_log() {
    let Some(table) = TABLE.get() else { return };
    let mut out = String::from("# index cell slot function total other | caller:count ...\n");
    for (i, e) in table.iter().enumerate() {
        let total = e.total.load(Ordering::Relaxed);
        if total == 0 {
            continue;
        }
        let (cell, slot, f) = TARGETS[i];
        out.push_str(&format!(
            "{i} {cell:#010x} {slot} {f:#010x} {total} {} |",
            e.other.load(Ordering::Relaxed)
        ));
        for k in 0..CALLERS {
            let r = e.ret[k].load(Ordering::Relaxed);
            if r != 0 {
                out.push_str(&format!(" {r:#010x}:{}", e.count[k].load(Ordering::Relaxed)));
            }
        }
        out.push('\n');
    }
    if let Ok(mut f) = std::fs::File::create(LOG_FILE) {
        let _ = f.write_all(out.as_bytes());
    }
}
