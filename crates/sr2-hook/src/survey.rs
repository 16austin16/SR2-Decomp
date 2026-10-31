//! Runtime survey of the memory-pool classes (temporary instrumentation).
//!
//! Every vtable slot of the mempool family is redirected through a 13-byte stub,
//! `lock inc dword [counter]; jmp dword [original]`, so the call counts per
//! method can be logged without touching any code bytes or registers (the stub
//! only changes flags, which nothing relies on at a function entry). The `.data`
//! section is also scanned for pool objects (a dword equal to a pool vtable).

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
};

use crate::log;

/// (class, vtable, slot count) from docs/rtti_classes.csv.
const POOLS: [(&str, usize, usize); 11] = [
    ("mempool", 0x00E5_8CD4, 25),
    ("virtual_mempool", 0x00E5_93A4, 25),
    ("page_mempool", 0x00E5_AC64, 25),
    ("split_page_mempool", 0x00E5_A6C4, 30),
    ("async_mempool", 0x00DE_1B94, 22),
    ("async_virtual_mempool", 0x00E5_A744, 25),
    ("compacting_mempool", 0x00DE_18D4, 21),
    ("tracked_mempool", 0x00E5_94DC, 29),
    ("string_pool", 0x00DD_A834, 25),
    ("string_hash_table", 0x00E5_959C, 25),
    ("split_mempool_template<mempool>", 0x00DE_186C, 25),
];
const MAX_SLOTS: usize = 32;
const DATA_START: usize = 0x00E8_0000;
const DATA_SIZE: usize = 0x0262_639C;

struct Survey {
    counts: Vec<[AtomicU32; MAX_SLOTS]>,
    /// Original function per class and slot.
    originals: Vec<Vec<usize>>,
}

static SURVEY: OnceLock<Survey> = OnceLock::new();

pub fn install() {
    let survey = SURVEY.get_or_init(|| Survey {
        counts: POOLS.iter().map(|_| std::array::from_fn(|_| AtomicU32::new(0))).collect(),
        originals: POOLS
            .iter()
            .map(|&(_, vt, n)| (0..n).map(|i| unsafe { *((vt + 4 * i) as *const usize) }).collect())
            .collect(),
    });
    let total: usize = POOLS.iter().map(|p| p.2).sum();
    // Stubs plus one pointer cell per slot for the indirect jmp.
    let mem = unsafe { VirtualAlloc(std::ptr::null(), total * 20, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) }
        as *mut u8;
    if mem.is_null() {
        log("survey: VirtualAlloc failed");
        return;
    }
    let mut stub = mem;
    for (c, &(_, vt, n)) in POOLS.iter().enumerate() {
        let mut old = 0u32;
        if unsafe { VirtualProtect(vt as *const c_void, n * 4, PAGE_READWRITE, &mut old) } == 0 {
            log("survey: VirtualProtect failed");
            return;
        }
        for i in 0..n {
            unsafe {
                let cell = stub.add(16) as *mut usize;
                *cell = survey.originals[c][i];
                let counter = &survey.counts[c][i] as *const AtomicU32 as usize;
                let code: [u8; 13] = {
                    let a = counter.to_le_bytes();
                    let b = (cell as usize).to_le_bytes();
                    [0xF0, 0xFF, 0x05, a[0], a[1], a[2], a[3], 0xFF, 0x25, b[0], b[1], b[2], b[3]]
                };
                std::ptr::copy_nonoverlapping(code.as_ptr(), stub, code.len());
                *((vt + 4 * i) as *mut usize) = stub as usize;
                stub = stub.add(20);
            }
        }
        unsafe { VirtualProtect(vt as *const c_void, n * 4, old, &mut old) };
    }
    log(&format!("survey: counting {total} mempool vtable slots across {} classes", POOLS.len()));
}

pub fn report(label: &str) {
    let Some(survey) = SURVEY.get() else { return };
    let mut lines = vec![format!("survey [{label}] mempool method calls (class.slot +offset fn: count):")];
    for (c, &(name, _, n)) in POOLS.iter().enumerate() {
        let hits: Vec<String> = (0..n)
            .filter_map(|i| {
                let k = survey.counts[c][i].load(Ordering::Relaxed);
                (k > 0).then(|| format!("{i}+{:02x} {:#x}: {k}", 4 * i, survey.originals[c][i]))
            })
            .collect();
        if !hits.is_empty() {
            lines.push(format!("  {name}: {}", hits.join(", ")));
        }
    }
    log(&lines.join("\n"));
}

/// Pool objects stored directly in `.data`, with their first 16 dwords.
pub fn report_instances() {
    let mut lines = vec!["survey: mempool objects in .data (vtable match):".to_string()];
    let words = unsafe { std::slice::from_raw_parts(DATA_START as *const usize, DATA_SIZE / 4) };
    for (k, &w) in words.iter().enumerate() {
        if let Some(&(name, _, _)) = POOLS.iter().find(|p| p.1 == w) {
            let at = DATA_START + 4 * k;
            let fields = &words[k + 1..(k + 16).min(words.len())];
            let text = fields.iter().map(|f| format!("{f:08x}")).collect::<Vec<_>>().join(" ");
            // The first field that points at a short printable string is probably the pool's name.
            let label = fields
                .iter()
                .find_map(|&p| printable(p))
                .map(|s| format!(" name {s:?}"))
                .unwrap_or_default();
            lines.push(format!("  {at:#010x} {name}{label} | {text}"));
        }
    }
    log(&lines.join("\n"));
}

fn printable(p: usize) -> Option<String> {
    // Only read inside the image's read-only and data sections.
    if !(0x00DB_8000..0x034A_7000).contains(&p) {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(p as *const u8, 48) };
    let end = bytes.iter().position(|&b| b == 0)?;
    (end >= 3 && bytes[..end].iter().all(|b| b.is_ascii_graphic() || *b == b' '))
        .then(|| String::from_utf8_lossy(&bytes[..end]).into_owned())
}
