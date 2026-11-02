//! Whole-function differential tests for the translated camera functions of `port_big.rs`: the
//! original machine code (mapped from bin/SR2_pc.unpacked.exe at its real addresses) against the
//! Rust translation, on the same random registers / memory / stack arguments.
//!
//! How it works (all in one process, native, single thread):
//! - The exe image is mapped at 0x400000. `.text` / `.rdata` are copied in as they are; every page of
//!   `.data` / `.bss` and of a 64 MB "arena" (pointer targets) starts as PAGE_NOACCESS and is filled
//!   on its first touch by a vectored exception handler: with the exe's bytes or, depending on the
//!   scenario, with hash-generated words (zeros, small ints, floats, pool values, pointers into the
//!   arena). The fill is a pure function of (scenario, address), so both runs see the same memory.
//!   Touching an address outside the universe aborts the run (the abort address must match).
//! - Callees stay the original code. Every callee address of the ported functions is tapped: the first
//!   bytes are replaced by a jump into a logger that records the registers, the x87 stack and the
//!   stack arguments, then continues in the real callee through a trampoline and logs the return state
//!   (registers, x87 stack, the number of bytes the callee popped). A few callees can be replaced by
//!   fakes (OS imports, mode update functions, virtual methods).
//! - The original runs on the same emulated stack the translation uses (a fixed address, switched in
//!   `STK_P`), so stack pointers, the call stack window and the frame are directly comparable.
//! - Compared: every call (target, registers, xmm0, the logical x87 stack, 16 stack dwords at ESP),
//!   every return (registers, xmm0, x87, popped bytes, and the pop count the translation assumed), the
//!   exit registers, ESP balance, ST0 / xmm0, every page written, the arguments area, and for aborted
//!   runs the fault address.
//!
//! Run (needs the test exe linked at a base that leaves 0x400000..0x35B0000 free):
//!   RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release -p sr2-hook
//!   --target i686-pc-windows-msvc port_big::difftest -- --ignored --nocapture
//! Environment: DT_FN=name (substring), DT_N=scenarios per function, DT_SEED=base seed.

#![allow(static_mut_refs, clippy::all, dead_code)]

use super::*;
use std::arch::naked_asm;
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc, VirtualFree, VirtualProtect, VirtualQuery,
};

include!("port_big_difftest_data.rs");

pub const STK_P: usize = 0x0800_0000;
const ESP0: u32 = (STK_P + 4096 - 128) as u32;
const STK_LO: usize = STK_P - 0x1_0000;
const STK_HI: usize = STK_P + 0x1_0000;
const ARENA: usize = 0x3000_0000;
const ARENA_SIZE: usize = 0x0400_0000;
const IMG_LO: usize = 0x0040_0000;
const IMG_HI: usize = 0x035B_0000;
const LAZY_LO: usize = 0x00E8_0000;
const COV_BASE: usize = 0x0049_0000;
const COV_LEN: usize = 0x2_0000;
const STICKY: u32 = 0x02FF;
const RETN_BASE: usize = TH_BASE + 0x8000;
const NCALLS_MAX: usize = 20_000;

// ---------------------------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------------------------

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn mix2(a: u64, b: u64) -> u64 {
    mix(a ^ mix(b.wrapping_add(0x1234_5678_9ABC)))
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = mix(self.0);
        self.0
    }
    fn u32(&mut self) -> u32 {
        (self.next() >> 20) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 24) as u32 % n
    }
    fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }
}

const FLOATS: [f32; 16] = [
    0.0, 1.0, -1.0, 0.5, 2.0, 0.016666, 100.0, 3.14159, 1000.0, -0.5, 0.001, 10.0, 0.25, -100.0, 45.0, 1e-8,
];

fn arena_ptr(h: u64) -> u32 {
    (ARENA + (((h >> 8) as usize % (ARENA_SIZE - 0x1000)) & !0xF)) as u32
}

// ---------------------------------------------------------------------------------------------
// scenarios
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Scn {
    seed: u64,
    /// cumulative thresholds (of 1000) for: zero, small, pool, float, arena ptr, image ptr, random
    w: [u32; 7],
    pool: [u32; 8],
    keep_file: u32,
    regs: [u32; 7],
    xmm0: u32,
    args: [u32; 16],
    callee_seed: u64,
    over: Vec<(u32, u32, u8)>,
    pageseed: Vec<(u32, u64)>,
}

fn rand_pool(rng: &mut Rng) -> u32 {
    match rng.below(10) {
        0 => 0,
        1 => 1,
        2 => 0xFFFF_FFFF,
        3 => 2,
        4 => FLOATS[rng.below(16) as usize].to_bits(),
        5 => arena_ptr(rng.next()),
        6 => arena_ptr(rng.next()),
        7 => 4,
        8 => 0x3F80_0000,
        _ => rng.u32() & 0xFF,
    }
}

fn rand_reg(rng: &mut Rng, ptr_pct: u32) -> u32 {
    if rng.chance(ptr_pct) {
        return arena_ptr(rng.next());
    }
    match rng.below(6) {
        0 => 0,
        1 => rng.below(9),
        2 => FLOATS[rng.below(16) as usize].to_bits(),
        3 => rng.u32(),
        4 => rand_pool(rng),
        _ => 1,
    }
}

fn gen_scn(seed: u64, spec: &FnSpec) -> Scn {
    let mut rng = Rng(mix(seed));
    // weights: zero, small, pool, float, arena, image, random
    let raw = [
        8 + rng.below(25),
        8 + rng.below(18),
        8 + rng.below(25),
        4 + rng.below(18),
        6 + rng.below(25),
        1 + rng.below(6),
        1 + rng.below(6),
    ];
    let total: u32 = raw.iter().sum();
    let mut w = [0u32; 7];
    let mut acc = 0;
    for i in 0..7 {
        acc += raw[i];
        w[i] = acc * 1000 / total;
    }
    w[6] = 1000;
    let mut pool = [0u32; 8];
    for p in pool.iter_mut() {
        *p = rand_pool(&mut rng);
    }
    let ptr_pct = 10 + rng.below(85);
    let mut regs = [0u32; 7];
    for r in regs.iter_mut() {
        *r = rand_reg(&mut rng, ptr_pct);
    }
    let mut args = [0u32; 16];
    for a in args.iter_mut() {
        *a = if rng.chance(45) { FLOATS[rng.below(16) as usize].to_bits() } else { rand_reg(&mut rng, ptr_pct) };
    }
    if spec.dt_arg {
        args[0] = [0.016666f32, 0.033333, 0.0, 0.5, 1.0, 10.0, -0.01, f32::NAN][rng.below(8) as usize].to_bits();
    }
    Scn {
        seed: rng.next(),
        w,
        pool,
        keep_file: rng.below(101),
        regs,
        xmm0: FLOATS[rng.below(16) as usize].to_bits(),
        args,
        callee_seed: rng.next(),
        over: Vec::new(),
        pageseed: Vec::new(),
    }
}

fn mutate_scn(base: &Scn, rng: &mut Rng, touched: &[u32], spec: &FnSpec) -> Scn {
    let mut s = base.clone();
    let n = 1 + rng.below(3);
    for _ in 0..n {
        match rng.below(9) {
            0 => s.callee_seed = rng.next(),
            1 | 2 => {
                if !touched.is_empty() {
                    let p = touched[rng.below(touched.len() as u32) as usize];
                    s.pageseed.push((p, rng.next()));
                }
            }
            3 | 4 | 5 => {
                if !touched.is_empty() {
                    let p = touched[rng.below(touched.len() as u32) as usize];
                    let a = p + (rng.below(1024) << 2);
                    let v = if rng.chance(50) { s.pool[rng.below(8) as usize] } else { rand_pool(rng) };
                    s.over.push((a, v, 4));
                }
            }
            6 => {
                let k = rng.below(7) as usize;
                let pp = 10 + rng.below(80);
                s.regs[k] = rand_reg(rng, pp);
            }
            7 => s.pool[rng.below(8) as usize] = rand_pool(rng),
            _ => {
                let i = rng.below(16) as usize;
                s.args[i] = if rng.chance(50) { FLOATS[rng.below(16) as usize].to_bits() } else { rand_reg(rng, 70) };
                if spec.dt_arg && i == 0 {
                    s.args[0] = [0.016666f32, 0.033333, 0.0, 0.5, 1.0, 10.0, -0.01][rng.below(7) as usize].to_bits();
                }
            }
        }
    }
    if s.over.len() > 400 {
        s.over.drain(0..200);
    }
    if s.pageseed.len() > 200 {
        s.pageseed.drain(0..100);
    }
    s
}

// ---------------------------------------------------------------------------------------------
// the memory universe
// ---------------------------------------------------------------------------------------------

struct Sect {
    va: usize,
    vsize: usize,
    raw: usize,
    rsize: usize,
}

static mut EXE: Vec<u8> = Vec::new();
static mut SECTS: Vec<Sect> = Vec::new();
static mut SCN: Option<Scn> = None;
static mut TOUCHED: Vec<u32> = Vec::new();
static mut ACTIVE: bool = false;
static mut ABORT_ADDR: u32 = 0;
static mut ABORT_KIND: u32 = 0;
static mut ABORT_ACCESS: u32 = 0;
static mut ABORT_IN: i32 = -1;
static mut DYN_MAP: bool = true;
static mut CALLEE_STATS: Vec<(u32, u32)> = Vec::new();
static mut GUARD_ESP: u32 = 0;
static mut IN_GUARD: u32 = 0;
static mut DEBUG_ON: bool = false;
static mut DBG: Vec<String> = Vec::new();
static mut ORIG_SAVED_ESP: u32 = 0;
static mut ORIG_TARGET: u32 = 0;
static mut TAP_TARGET: u32 = 0;
static mut POST_RET: u32 = 0;
static mut MODE: u32 = 0; // 1 = original, 2 = Rust
static mut OUT: OutRegs = OutRegs { r: [0; 7], xmm0: 0, esp: 0, sw: 0, st0: [0; 2] };
static mut COVMAP: [u8; COV_LEN] = [0; COV_LEN];
static mut COV_NEW: u32 = 0;
static mut TRACE: Vec<u32> = Vec::new();
static RUN_START: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static mut MAIN_THREAD: isize = 0;
const RUN_LIMIT_MS: u64 = 150;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Hang guard: a run that takes longer than RUN_LIMIT_MS (a loop over garbage data in the original code,
/// or in the translation) is redirected to the abort pad. Both sides must then time out the same way.
unsafe fn start_watchdog() {
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows_sys::Win32::System::Diagnostics::Debug::{GetThreadContext, SetThreadContext, CONTEXT};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, ResumeThread, SuspendThread};
    let mut h = std::ptr::null_mut();
    DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut h, 0, 0, DUPLICATE_SAME_ACCESS);
    MAIN_THREAD = h as isize;
    std::thread::spawn(|| unsafe {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(25));
            let st = RUN_START.load(std::sync::atomic::Ordering::SeqCst);
            if st != 0 && now_ms() - st > RUN_LIMIT_MS {
                let h = MAIN_THREAD as *mut std::ffi::c_void;
                SuspendThread(h);
                if RUN_START.load(std::sync::atomic::Ordering::SeqCst) == st && ACTIVE && IN_GUARD == 1 {
                    let mut ctx: CONTEXT = std::mem::zeroed();
                    ctx.ContextFlags = 0x0001_0001; // CONTEXT_CONTROL
                    GetThreadContext(h, &mut ctx);
                    ctx.Eip = abort_pad as *const () as usize as u32;
                    ctx.Esp = GUARD_ESP;
                    ABORT_KIND = 3;
                    ABORT_ADDR = 0;
                    ABORT_IN = SWSTACK.last().map(|e| e.idx as i32).unwrap_or(-1);
                    SetThreadContext(h, &ctx);
                }
                ResumeThread(h);
            }
        }
    });
}

#[repr(C)]
struct OutRegs {
    r: [u32; 7],
    xmm0: u32,
    esp: u32,
    sw: u32,
    st0: [u32; 2],
}

#[repr(C)]
struct OrigCtx {
    r: [u32; 7],
    xmm0: u32,
    esp: u32,
}

unsafe fn alloc_fixed(addr: usize, len: usize, prot: u32) {
    let p = VirtualAlloc(addr as *const _, len, MEM_COMMIT, prot);
    assert_eq!(p as usize, addr, "cannot map {addr:#x}+{len:#x}");
}

unsafe fn rd_exe(raw: usize, n: usize) -> &'static [u8] {
    &EXE[raw..raw + n]
}

unsafe fn init_universe() {
    static mut DONE: bool = false;
    if DONE {
        return;
    }
    DONE = true;
    let path = std::env::var("DT_EXE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/SR2_pc.unpacked.exe").to_string());
    EXE = std::fs::read(&path).expect("bin/SR2_pc.unpacked.exe");
    let e = &EXE;
    let pe = u32::from_le_bytes(e[0x3C..0x40].try_into().unwrap()) as usize;
    let nsec = u16::from_le_bytes(e[pe + 6..pe + 8].try_into().unwrap()) as usize;
    let opt = u16::from_le_bytes(e[pe + 20..pe + 22].try_into().unwrap()) as usize;
    for i in 0..nsec {
        let o = pe + 24 + opt + i * 40 + 8;
        let g = |k: usize| u32::from_le_bytes(e[o + 4 * k..o + 4 * k + 4].try_into().unwrap()) as usize;
        SECTS.push(Sect { vsize: g(0), va: g(1) + 0x40_0000, rsize: g(2), raw: g(3) });
    }
    // the launcher reserved the ranges; commit inside them
    let p = VirtualAlloc(IMG_LO as *const _, LAZY_LO - IMG_LO, MEM_COMMIT, PAGE_EXECUTE_READWRITE);
    assert_eq!(p as usize, IMG_LO);
    for s in SECTS.iter() {
        if s.va >= LAZY_LO {
            continue;
        }
        let n = s.rsize.min(s.vsize);
        std::ptr::copy_nonoverlapping(EXE.as_ptr().add(s.raw), s.va as *mut u8, n);
    }
    let p = VirtualAlloc(LAZY_LO as *const _, IMG_HI - LAZY_LO, MEM_COMMIT, PAGE_NOACCESS);
    assert_eq!(p as usize, LAZY_LO);
    let p = VirtualAlloc(ARENA as *const _, ARENA_SIZE, MEM_COMMIT, PAGE_NOACCESS);
    assert_eq!(p as usize, ARENA, "cannot map the arena");
    alloc_fixed(STK_LO, STK_HI - STK_LO, PAGE_READWRITE);
    alloc_fixed(TH_BASE, 0x20000, PAGE_EXECUTE_READWRITE);
    AddVectoredExceptionHandler(1, Some(veh));
    start_watchdog();
    CALLS.reserve(NCALLS_MAX + 16);
    HINTS.reserve(512);
    CALLEE_STATS.resize(TAPS.len() + FAKES.len(), (0, 0));
    SWSTACK.reserve(1024);
    TOUCHED.reserve(65536);
    // thunks and trampolines
    for (k, &(_a, _n, tr)) in TAPS.iter().enumerate() {
        std::ptr::copy_nonoverlapping(tr.as_ptr(), (TR_BASE + 64 * k) as *mut u8, tr.len());
    }
    for n in 0..=24usize {
        let p = (RETN_BASE + 8 * n) as *mut u8;
        if n == 0 {
            *p = 0xC3;
        } else {
            *p = 0xC2;
            *(p.add(1) as *mut u16) = (4 * n) as u16;
        }
    }
    for k in 0..(TAPS.len() + FAKES.len()) {
        let p = (TH_BASE + 16 * k) as *mut u8;
        *p = 0x68;
        *(p.add(1) as *mut u32) = k as u32;
        *p.add(5) = 0xE9;
        let rel = (tap_common as *const () as usize as i64 - (p as usize as i64 + 10)) as i32;
        *(p.add(6) as *mut i32) = rel;
    }
}

fn image_section(page: usize) -> Option<&'static Sect> {
    unsafe { SECTS.iter().find(|s| page >= s.va & !0xFFF && page < (s.va + s.vsize + 0xFFF) & !0xFFF) }
}

fn is_lazy(addr: usize) -> bool {
    (addr >= LAZY_LO && addr < IMG_HI) || (addr >= ARENA && addr < ARENA + ARENA_SIZE)
}

/// One generated word of memory at `addr`. Pages come in three styles (pointer-rich, data-rich, mixed) so that
/// chains of pointers often survive while data fields still get the scenario's mix of small ints and floats.
fn gen_word(s: &Scn, pseed: u64, addr: u32) -> u32 {
    let h = mix2(pseed, addr as u64);
    let style = mix2(pseed, 0x5717E) % 10;
    let sel = (h & 0x3FF) as u32 % 1000;
    let r = (h >> 12) as u32;
    if style < 5 {
        // pointer-rich
        return match sel {
            0..=69 => 0,
            70..=169 => r % 9,
            170..=219 => s.pool[(r & 7) as usize],
            220..=269 => FLOATS[(r & 15) as usize].to_bits(),
            270..=969 => arena_ptr(h),
            _ => r,
        };
    }
    if sel < s.w[0] {
        0
    } else if sel < s.w[1] {
        r % 9
    } else if sel < s.w[2] {
        s.pool[(r & 7) as usize]
    } else if sel < s.w[3] {
        FLOATS[(r & 15) as usize].to_bits()
    } else if sel < s.w[4] {
        arena_ptr(h)
    } else if sel < s.w[5] {
        (0x00DB_8000 + ((r as usize) % 0x40_0000)) as u32 & !3
    } else {
        r
    }
}

fn fill_page(s: &Scn, page: usize, buf: &mut [u8; 4096]) {
    let mut pseed = s.seed ^ ((page as u64) << 8);
    for &(p, sd) in s.pageseed.iter() {
        if p as usize == page {
            pseed = sd;
        }
    }
    let hfile = mix2(s.seed ^ 0x77, page as u64) % 100;
    let mut done = false;
    if page < ARENA {
        if let Some(sec) = image_section(page) {
            if (hfile as u32) < s.keep_file {
                // the exe's bytes (zeros past the raw data)
                buf.fill(0);
                let off = page.wrapping_sub(sec.va);
                if page >= sec.va && off < sec.rsize.min(sec.vsize) {
                    let n = (sec.rsize.min(sec.vsize) - off).min(4096);
                    buf[..n].copy_from_slice(unsafe { rd_exe(sec.raw + off, n) });
                } else if page < sec.va {
                    let start = sec.va - page;
                    let n = (sec.rsize.min(sec.vsize)).min(4096 - start);
                    buf[start..start + n].copy_from_slice(unsafe { rd_exe(sec.raw, n) });
                }
                done = true;
            }
        }
    }
    if !done {
        for i in 0..1024 {
            let a = (page + i * 4) as u32;
            let v = gen_word(s, pseed, a);
            buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    for &(a, v, sz) in s.over.iter() {
        if a as usize & !0xFFF == page {
            let o = a as usize & 0xFFF;
            let n = (sz as usize).min(4096 - o);
            buf[o..o + n].copy_from_slice(&v.to_le_bytes()[..n]);
        }
    }
}

unsafe fn reset_pages() {
    for &p in TOUCHED.iter() {
        if is_lazy(p as usize) {
            let mut old = 0;
            VirtualProtect(p as *const _, 4096, PAGE_NOACCESS, &mut old);
        } else {
            VirtualFree(p as *mut _, 0, MEM_RELEASE);
        }
    }
    TOUCHED.clear();
}

/// Any other address (a garbage pointer) is mapped on demand too, except the null page and the top of the address space:
/// the run then reads generated memory instead of crashing. Released again by `reset_pages`.
unsafe fn map_dynamic(page: usize) -> bool {
    if page < 0x1_0000 || page >= 0x7FF0_0000 {
        return false;
    }
    let p = VirtualAlloc(page as *const _, 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if p as usize != page {
        return false;
    }
    let mut buf = [0u8; 4096];
    fill_page(SCN.as_ref().unwrap(), page, &mut buf);
    std::ptr::copy_nonoverlapping(buf.as_ptr(), page as *mut u8, 4096);
    TOUCHED.push(page as u32);
    true
}

unsafe extern "system" fn veh(info: *mut EXCEPTION_POINTERS) -> i32 {
    let rec = &*(*info).ExceptionRecord;
    let ctx = &mut *(*info).ContextRecord;
    if !ACTIVE {
        if rec.ExceptionCode == 0xC000_0005u32 as i32 {
            eprintln!("AV outside a run: eip {:#x} addr {:#x} esp {:#x}", ctx.Eip, rec.ExceptionInformation[1], ctx.Esp);
        }
        return 0;
    }
    if rec.ExceptionCode == 0xC000_0005u32 as i32 {
        let addr = rec.ExceptionInformation[1];
        if is_lazy(addr) {
            let page = addr & !0xFFF;
            let mut old = 0;
            VirtualProtect(page as *const _, 4096, PAGE_READWRITE, &mut old);
            let mut buf = [0u8; 4096];
            fill_page(SCN.as_ref().unwrap(), page, &mut buf);
            std::ptr::copy_nonoverlapping(buf.as_ptr(), page as *mut u8, 4096);
            TOUCHED.push(page as u32);
            return -1;
        }
        if DYN_MAP && map_dynamic(addr & !0xFFF) {
            return -1;
        }
        ABORT_ADDR = addr as u32;
        ABORT_IN = SWSTACK.last().map(|e| e.idx as i32).unwrap_or(-1);
        ABORT_KIND = 1;
        ABORT_ACCESS = rec.ExceptionInformation[0] as u32;
    } else if rec.ExceptionCode == 0x4000_0015u32 as i32 || rec.ExceptionCode == 0x4000_001Eu32 as i32 {
        return 0;
    } else {
        ABORT_ADDR = rec.ExceptionCode as u32;
        ABORT_KIND = 2;
    }
    ctx.Eip = abort_pad as *const () as usize as u32;
    ctx.Esp = GUARD_ESP;
    -1
}

// ---------------------------------------------------------------------------------------------
// running code: guarded call, original entry / exit pads
// ---------------------------------------------------------------------------------------------

#[unsafe(naked)]
unsafe extern "C" fn guarded(f: usize, a0: usize, a1: usize) -> u32 {
    naked_asm!(
        "push ebx", "push esi", "push edi", "push ebp",
        "mov dword ptr [{g}], esp",
        "mov dword ptr [{ig}], 1",
        "push dword ptr [esp + 28]",
        "push dword ptr [esp + 28]",
        "call dword ptr [esp + 28]",
        "mov dword ptr [{ig}], 0",
        "add esp, 8",
        "xor eax, eax",
        "pop ebp", "pop edi", "pop esi", "pop ebx",
        "ret",
        g = sym GUARD_ESP,
        ig = sym IN_GUARD,
    )
}

#[unsafe(naked)]
unsafe extern "C" fn abort_pad() {
    naked_asm!("mov dword ptr [{ig}], 0", "mov eax, 1", "pop ebp", "pop edi", "pop esi", "pop ebx", "ret", ig = sym IN_GUARD)
}

/// cdecl `orig_enter(ctx)`: loads the registers, switches to the emulated stack and jumps into the
/// original code; the original's `ret` lands in `orig_ret_pad`.
#[unsafe(naked)]
unsafe extern "C" fn orig_enter(_ctx: *mut OrigCtx, _unused: usize) {
    naked_asm!(
        "push ebx", "push esi", "push edi", "push ebp",
        "mov eax, [esp + 20]",
        "mov dword ptr [{saved}], esp",
        "mov ecx, [eax + 4]", "mov edx, [eax + 8]", "mov ebx, [eax + 12]", "mov esi, [eax + 16]",
        "mov edi, [eax + 20]", "mov ebp, [eax + 24]",
        "movss xmm0, dword ptr [eax + 28]",
        "mov esp, [eax + 32]",
        "mov eax, [eax]",
        "jmp dword ptr [{tgt}]",
        saved = sym ORIG_SAVED_ESP,
        tgt = sym ORIG_TARGET,
    )
}

#[unsafe(naked)]
unsafe extern "C" fn orig_ret_pad() {
    naked_asm!(
        "mov dword ptr [{o}], eax", "mov dword ptr [{o} + 4], ecx", "mov dword ptr [{o} + 8], edx",
        "mov dword ptr [{o} + 12], ebx", "mov dword ptr [{o} + 16], esi", "mov dword ptr [{o} + 20], edi",
        "mov dword ptr [{o} + 24], ebp",
        "movss dword ptr [{o} + 28], xmm0",
        "mov dword ptr [{o} + 32], esp",
        "fnstsw ax",
        "movzx eax, ax",
        "mov dword ptr [{o} + 36], eax",
        "shr eax, 11",
        "and eax, 7",
        "jz 3f",
        "fst qword ptr [{o} + 40]",
        "3:",
        "mov esp, dword ptr [{saved}]",
        "pop ebp", "pop edi", "pop esi", "pop ebx",
        "xor eax, eax",
        "ret",
        o = sym OUT,
        saved = sym ORIG_SAVED_ESP,
    )
}

// ---------------------------------------------------------------------------------------------
// call taps
// ---------------------------------------------------------------------------------------------

#[repr(C)]
struct TapFrame {
    xmm0: [u32; 4],
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
    idx: u32,
    ret: u32,
}

#[repr(C)]
struct PostFrame {
    xmm0: [u32; 4],
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

#[unsafe(naked)]
unsafe extern "C" fn tap_common() {
    naked_asm!(
        "push ebp", "push edi", "push esi", "push ebx", "push edx", "push ecx", "push eax",
        "sub esp, 16",
        "movss dword ptr [esp], xmm0",
        "mov eax, esp",
        "push eax",
        "call {log}",
        "add esp, 4",
        "movss xmm0, dword ptr [esp]",
        "add esp, 16",
        "pop eax", "pop ecx", "pop edx", "pop ebx", "pop esi", "pop edi", "pop ebp",
        "add esp, 4",
        "jmp dword ptr [{tgt}]",
        log = sym tap_log,
        tgt = sym TAP_TARGET,
    )
}

#[unsafe(naked)]
unsafe extern "C" fn post_pad() {
    naked_asm!(
        "push ebp", "push edi", "push esi", "push ebx", "push edx", "push ecx", "push eax",
        "sub esp, 16",
        "movss dword ptr [esp], xmm0",
        "mov eax, esp",
        "push eax",
        "call {log}",
        "add esp, 4",
        "movss xmm0, dword ptr [esp]",
        "add esp, 16",
        "pop eax", "pop ecx", "pop edx", "pop ebx", "pop esi", "pop edi", "pop ebp",
        "jmp dword ptr [{ret}]",
        log = sym post_log,
        ret = sym POST_RET,
    )
}

unsafe fn x87_dump(out: &mut [f64; 8]) -> usize {
    let sw: u16;
    std::arch::asm!("fnstsw ax", out("ax") sw, options(nostack, nomem));
    let depth = (8 - ((sw >> 11) & 7) as usize) & 7;
    for k in 0..depth {
        std::arch::asm!("fstp qword ptr [{p}]", p = in(reg) &mut out[k] as *mut f64, options(nostack));
    }
    for k in (0..depth).rev() {
        std::arch::asm!("fld qword ptr [{p}]", p = in(reg) &out[k] as *const f64, options(nostack));
    }
    depth
}

unsafe fn x87_pop() {
    std::arch::asm!("fstp st(0)", options(nostack));
}

unsafe fn x87_push(v: f64) {
    std::arch::asm!("fld qword ptr [{p}]", p = in(reg) &v as *const f64, options(nostack));
}

#[derive(Clone, Copy)]
struct CallRec {
    /// 0 = call, 1 = return
    kind: u8,
    depth: u8,
    idx: u32,
    vesp: u32,
    regs: [u32; 6],
    xmm0: u32,
    nst: u8,
    st: [f64; 8],
    stack: [u32; 16],
    /// call: the pop count (dwords) the translation assumed; return: bytes the callee popped
    pops: i32,
    assumed_pop: i32,
    fake: bool,
}

static mut CALLS: Vec<CallRec> = Vec::new();

pub struct Pre {
    valid: bool,
    vesp: u32,
    pop: u32,
    nin: u32,
    fxn: usize,
    fx: [f64; 8],
    nargs: u32,
    target: u32,
}
static mut PRE: Pre = Pre { valid: false, vesp: 0, pop: 0, nin: 0, fxn: 0, fx: [0.0; 8], nargs: 0, target: 0 };

struct SwEnt {
    idx: u32,
    ret: u32,
    args_ptr: u32,
    top: bool,
    fxn: usize,
    fx: [f64; 8],
    assumed_pop: i32,
    vesp: u32,
}
static mut SWSTACK: Vec<SwEnt> = Vec::new();
static mut CALL_NO: u32 = 0;
static mut EXTRA_ERR: Vec<String> = Vec::new();

/// Called by the translation (cfg(test)) right before it calls a callee.
pub(super) fn pre_call(b: &Blk, fx: &X87, pop: u32) {
    unsafe {
        PRE.valid = true;
        PRE.vesp = b.args;
        PRE.pop = pop;
        PRE.nin = b.has_in;
        PRE.nargs = b.nargs;
        PRE.target = b.f;
        let depth = (8 - fx.top) & 7;
        PRE.fxn = depth;
        for i in 0..depth {
            PRE.fx[i] = fx.s(i);
        }
    }
}

const RING: usize = 256;
static mut RING_BUF: [(u32, u32, u8); RING] = [(0, 0, 0); RING];
static mut RING_N: usize = 0;
static mut INIT_REGS: [u32; 7] = [0; 7];
static mut INIT_ARGS: [u32; 16] = [0; 16];

#[derive(Clone, Copy, PartialEq)]
enum Hint {
    Mem(u32, u32, u8),
    Reg(u8, u32),
    Arg(u8, u32),
}
static mut HINTS: Vec<Hint> = Vec::new();

/// Called from the shadowed memory readers of the translation (cfg(test)): remembers which lazily-filled memory the
/// last values were read from.
pub(super) fn note_read(p: usize, v: u32, size: u8) {
    unsafe {
        if MODE == 2 && (is_lazy(p) || (p >= 0x1_0000 && p < 0x7FF0_0000 && !(p >= IMG_LO && p < LAZY_LO) && !(p >= STK_LO && p < STK_HI) && !(p >= TH_BASE && p < TH_BASE + 0x20000))) {
            RING_BUF[RING_N % RING] = (p as u32, v, size);
            RING_N += 1;
        }
    }
}

unsafe fn hint_for(x: u32, y: u32, bits: u32) {
    if HINTS.len() >= 256 {
        return;
    }
    let n = RING_N.min(RING);
    for k in 0..n {
        let (ad, v, sz) = RING_BUF[(RING_N - 1 - k) % RING];
        if sz as u32 * 8 == bits && v == x {
            let h = Hint::Mem(ad, y, sz);
            if !HINTS.contains(&h) {
                HINTS.push(h);
            }
            return;
        }
    }
    if bits == 32 {
        for k in 0..7 {
            if INIT_REGS[k] == x {
                let h = Hint::Reg(k as u8, y);
                if !HINTS.contains(&h) {
                    HINTS.push(h);
                }
                return;
            }
        }
        for k in 0..16 {
            if INIT_ARGS[k] == x {
                let h = Hint::Arg(k as u8, y);
                if !HINTS.contains(&h) {
                    HINTS.push(h);
                }
                return;
            }
        }
    }
}

/// Called from `f_sub` (cfg(test)): the operands of every compare, to steer the next scenarios (an equal pair makes a
/// `je` flip).
pub(super) fn cmp_hint(a: u32, b: u32, bits: u32) {
    unsafe {
        if MODE != 2 || a == b {
            return;
        }
        let m = fl_mask(bits);
        hint_for(a & m, b & m, bits);
        hint_for(b & m, a & m, bits);
    }
}

fn apply_hint(s: &mut Scn, h: Hint) {
    match h {
        Hint::Mem(a, v, sz) => s.over.push((a, v, sz)),
        Hint::Reg(k, v) => s.regs[k as usize] = v,
        Hint::Arg(k, v) => s.args[k as usize] = v,
    }
}

/// A fault at `addr` (not in the universe): find the value it was computed from and make it a valid pointer.
unsafe fn repair_hint(addr: u32, rng: &mut Rng) -> Option<Hint> {
    let n = RING_N.min(RING);
    for k in 0..n {
        let (ad, v, sz) = RING_BUF[(RING_N - 1 - k) % RING];
        if sz == 4 && addr.wrapping_sub(v) < 0x4000 {
            return Some(Hint::Mem(ad, arena_ptr(rng.next()), 4));
        }
    }
    for k in 0..7 {
        if addr.wrapping_sub(INIT_REGS[k]) < 0x4000 {
            return Some(Hint::Reg(k as u8, arena_ptr(rng.next())));
        }
    }
    for k in 0..16 {
        if addr.wrapping_sub(INIT_ARGS[k]) < 0x4000 {
            return Some(Hint::Arg(k as u8, arena_ptr(rng.next())));
        }
    }
    None
}

pub(super) fn cov(pc: usize) {
    unsafe {
        if MODE == 2 && TRACE.len() < 30000 {
            TRACE.push(pc as u32);
        }
        if MODE == 2 && pc >= COV_BASE && pc - COV_BASE < COV_LEN {
            let c = &mut COVMAP[pc - COV_BASE];
            if *c == 0 {
                *c = 1;
                COV_NEW += 1;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FakeSpec {
    name: &'static str,
    ret_dw: usize,
    nin: usize,
    nout: usize,
    /// 0 = eax unchanged, 1 = small, 2 = arena pointer, 3 = random
    eax: u8,
}

/// Fakes: (patched address or import slot, spec). The first TAPS.len() thunks are the real taps; fake `k`
/// has index TAPS.len() + k and is called through its own thunk.
static FAKES: &[(u32, bool, FakeSpec)] = &[
    // (address, is_import_slot, spec)
    (0x00DB_807C, true, FakeSpec { name: "import 0xDB807C (TlsGetValue, stdcall 1)", ret_dw: 1, nin: 0, nout: 0, eax: 2 }),
    (0x00DD_5258, true, FakeSpec { name: "mode0 table", ret_dw: 0, nin: 0, nout: 0, eax: 0 }),
];

fn call_rng(salt: u64) -> u64 {
    unsafe { mix2(SCN.as_ref().unwrap().callee_seed ^ salt, CALL_NO as u64) }
}

extern "C" fn tap_log(fr: *mut TapFrame) {
    unsafe {
        let fr = &mut *fr;
        let idx = fr.idx as usize;
        CALL_NO += 1;
        if MODE == 2 && idx < CALLEE_STATS.len() {
            CALLEE_STATS[idx].0 += 1;
        }
        if DEBUG_ON {
            eprintln!("tap {} mode {} fr {:#x} window {:x?}", idx, MODE, fr as *const _ as usize, (0..8).map(|i| rd(ESP0 as usize - 0x14 + 4 * i)).collect::<Vec<_>>());
        }
        let mut st = [0f64; 8];
        let nreal = x87_dump(&mut st);
        let args_ptr = (&fr.ret as *const u32 as u32) + 4;
        let top = SWSTACK.is_empty();
        let rust = MODE == 2;
        let mut rec = CallRec {
            kind: 0,
            depth: SWSTACK.len() as u8,
            idx: idx as u32,
            vesp: if rust && top { PRE.vesp } else { args_ptr },
            regs: [fr.eax, fr.ecx, fr.edx, fr.ebx, fr.esi, fr.edi],
            xmm0: fr.xmm0[0],
            nst: nreal as u8,
            st,
            stack: [0; 16],
            pops: 0,
            assumed_pop: -1,
            fake: false,
        };
        let mut fxn = 0usize;
        let mut fx = [0f64; 8];
        if rust && top {
            if !PRE.valid || PRE.target != taps_addr(idx) && idx < TAPS.len() {
                EXTRA_ERR.push(format!("tap {idx} fired without matching pre_call (target {:#x})", PRE.target));
            }
            if nreal as u32 != PRE.nin {
                EXTRA_ERR.push(format!("call {:#x}: real x87 depth {nreal} at the callee, the translation passed {} inputs", taps_addr(idx), PRE.nin));
            }
            fxn = PRE.fxn;
            fx = PRE.fx;
            rec.assumed_pop = PRE.pop as i32;
            for i in 0..fxn {
                if nreal + i < 8 {
                    rec.st[nreal + i] = fx[i];
                }
            }
            rec.nst = (nreal + fxn).min(8) as u8;
            PRE.valid = false;
        }
        if top {
            for i in 0..16 {
                let a = rec.vesp as usize + 4 * i;
                rec.stack[i] = if a >= STK_LO && a + 4 <= STK_HI && a != ESP0 as usize { rd(a) } else { 0 };
            }
        }
        // decide: fake or real
        let fake: Option<FakeSpec> = if idx >= TAPS.len() { Some(FAKES[idx - TAPS.len()].2) } else { None };
        if let Some(f) = fake {
            rec.fake = true;
            CALLS.push(rec);
            // behaviour: pop inputs, set eax, push outputs
            for _ in 0..f.nin {
                x87_pop();
            }
            let r = call_rng(0xFA);
            match f.eax {
                1 => fr.eax = (r >> 8) as u32 % 6,
                2 => fr.eax = arena_ptr(r),
                3 => fr.eax = (r >> 8) as u32,
                _ => {}
            }
            for k in 0..f.nout {
                x87_push(FLOATS[((r >> (12 + 4 * k)) & 15) as usize] as f64);
            }
            TAP_TARGET = (RETN_BASE + 8 * f.ret_dw) as u32;
        } else {
            CALLS.push(rec);
            SWSTACK.push(SwEnt { idx: idx as u32, ret: fr.ret, args_ptr, top, fxn, fx, assumed_pop: rec.assumed_pop, vesp: rec.vesp });
            fr.ret = post_pad as *const () as usize as u32;
            TAP_TARGET = (TR_BASE + 64 * idx) as u32;
        }
        if CALLS.len() >= NCALLS_MAX {
            ABORT_KIND = 4;
            ABORT_ADDR = 0;
            std::arch::asm!("mov esp, dword ptr [{g}]", "jmp {pad}", g = sym GUARD_ESP, pad = sym abort_pad, options(noreturn));
        }
    }
}

fn taps_addr(idx: usize) -> u32 {
    if idx < TAPS.len() { TAPS[idx].0 } else { FAKES[idx - TAPS.len()].0 }
}

extern "C" fn post_log(fr: *mut PostFrame) {
    unsafe {
        let fr = &mut *fr;
        let ent = SWSTACK.pop().expect("post without tap");
        let mut st = [0f64; 8];
        let nreal = x87_dump(&mut st);
        let fr_end = (&fr.ebp as *const u32 as u32) + 4;
        let rust = MODE == 2;
        let mut rec = CallRec {
            kind: 1,
            depth: SWSTACK.len() as u8,
            idx: ent.idx,
            vesp: ent.vesp,
            regs: [fr.eax, fr.ecx, fr.edx, fr.ebx, fr.esi, fr.edi],
            xmm0: fr.xmm0[0],
            nst: nreal as u8,
            st,
            stack: [0; 16],
            pops: (fr_end - ent.args_ptr) as i32,
            assumed_pop: ent.assumed_pop,
            fake: false,
        };
        if rust && ent.top {
            for i in 0..ent.fxn {
                if nreal + i < 8 {
                    rec.st[nreal + i] = ent.fx[i];
                }
            }
            rec.nst = (nreal + ent.fxn).min(8) as u8;
        }
        CALLS.push(rec);
        POST_RET = ent.ret;
    }
}

// ---------------------------------------------------------------------------------------------
// tap installation
// ---------------------------------------------------------------------------------------------

static mut TAP_SAVED: Vec<[u8; 5]> = Vec::new();

unsafe fn set_tap(k: usize, on: bool) {
    let a = TAPS[k].0 as usize;
    if TAP_SAVED.is_empty() {
        for &(a, _, _) in TAPS.iter() {
            let mut b = [0u8; 5];
            std::ptr::copy_nonoverlapping(a as *const u8, b.as_mut_ptr(), 5);
            TAP_SAVED.push(b);
        }
    }
    if on {
        let p = a as *mut u8;
        *p = 0xE9;
        let rel = ((TH_BASE + 16 * k) as i64 - (a as i64 + 5)) as i32;
        *(p.add(1) as *mut i32) = rel;
    } else {
        std::ptr::copy_nonoverlapping(TAP_SAVED[k].as_ptr(), a as *mut u8, 5);
    }
}

unsafe fn install_taps(exclude: u32) {
    for k in 0..TAPS.len() {
        set_tap(k, TAPS[k].0 != exclude);
    }
    // import slot and the camera mode update table (slot 2 of each 5-dword mode entry)
    wd(0x00DB_807C, (TH_BASE + 16 * TAPS.len()) as u32);
    for k in 0..12usize {
        let slot = 0x00DD_5258 + 20 * k;
        if rd(slot) != 0 {
            wd(slot, (TH_BASE + 16 * (TAPS.len() + 1)) as u32);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// one execution
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct FnSpec {
    name: &'static str,
    start: u32,
    body: unsafe fn(&mut Ent, *const u32),
    ret_n: u32,
    dt_arg: bool,
}

struct RunOut {
    aborted: u32,
    kind: u32,
    abort_addr: u32,
    regs: [u32; 7],
    xmm0: u32,
    esp: u32,
    nst: u32,
    st0: u64,
    calls: Vec<CallRec>,
    pages: Vec<(u32, Vec<u8>)>,
    argarea: Vec<u8>,
    extra: Vec<String>,
}

unsafe fn fpu_init() {
    let cw: u16 = 0x027F;
    std::arch::asm!("fninit", "fldcw word ptr [{p}]", p = in(reg) &cw as *const u16, options(nostack));
}

unsafe fn prepare(s: &Scn, _spec: &FnSpec, preload: &[u32]) {
    reset_pages();
    SCN = Some(s.clone());
    for &p in preload {
        if is_lazy(p as usize) {
            let mut old = 0;
            VirtualProtect(p as *const _, 4096, PAGE_READWRITE, &mut old);
            let mut buf = [0u8; 4096];
            fill_page(s, p as usize, &mut buf);
            std::ptr::copy_nonoverlapping(buf.as_ptr(), p as *mut u8, 4096);
            TOUCHED.push(p);
        } else {
            map_dynamic(p as usize);
        }
    }
    CALLS.clear();
    TRACE.clear();
    SWSTACK.clear();
    EXTRA_ERR.clear();
    CALL_NO = 0;
    PRE.valid = false;
    // stack garbage and arguments
    let lo = ESP0 as usize - 0x1000;
    let hi = ESP0 as usize + 0x400;
    let mut a = lo;
    while a < hi {
        wd(a, mix2(s.seed ^ 0x57AC, a as u64) as u32);
        a += 4;
    }
    for i in 0..16 {
        wd(ESP0 as usize + 4 + 4 * i, s.args[i]);
    }
    fpu_init();
    if DEBUG_ON {
        eprintln!("prepare mode {} window {:x?}", MODE, (0..8).map(|i| rd(ESP0 as usize - 0x14 + 4 * i)).collect::<Vec<_>>());
    }
}

unsafe fn finish(aborted: u32, _spec: &FnSpec) -> RunOut {
    ACTIVE = false;
    RUN_START.store(0, std::sync::atomic::Ordering::SeqCst);
    let mut pages = Vec::with_capacity(TOUCHED.len());
    for &p in TOUCHED.iter() {
        pages.push((p, std::slice::from_raw_parts(p as *const u8, 4096).to_vec()));
    }
    let argarea = std::slice::from_raw_parts((ESP0 + 4) as *const u8, 0x300).to_vec();
    let out = RunOut {
        aborted,
        kind: ABORT_KIND,
        abort_addr: ABORT_ADDR,
        regs: [0; 7],
        xmm0: 0,
        esp: 0,
        nst: 0,
        st0: 0,
        calls: CALLS.clone(),
        pages,
        argarea,
        extra: EXTRA_ERR.clone(),
    };
    out
}

/// The original runs twice: the first run discovers the pages it touches (the exception handler that fills them
/// scribbles over the stack below ESP, which would differ from the translation's run); the second run has
/// them all filled in advance, so nothing disturbs the emulated stack.
unsafe fn exec_orig(s: &Scn, spec: &FnSpec) -> (RunOut, Vec<u32>) {
    let d = orig_once(s, spec, &[]);
    let pages: Vec<u32> = d.pages.iter().map(|p| p.0).collect();
    if d.kind >= 3 {
        return (d, pages);
    }
    let o = orig_once(s, spec, &pages);
    (o, pages)
}

unsafe fn orig_once(s: &Scn, spec: &FnSpec, preload: &[u32]) -> RunOut {
    prepare(s, spec, preload);
    MODE = 1;
    wd(ESP0 as usize, orig_ret_pad as *const () as usize as u32);
    let ctx = OrigCtx { r: s.regs, xmm0: s.xmm0, esp: ESP0 };
    ORIG_TARGET = spec.start;
    ABORT_ADDR = 0;
    ABORT_KIND = 0;
    ACTIVE = true;
    RUN_START.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
    let ab = guarded(orig_enter as *const () as usize, &ctx as *const _ as usize, 0);
    let mut o = finish(ab, spec);
    if ab == 0 {
        o.regs = OUT.r;
        o.xmm0 = OUT.xmm0;
        o.esp = OUT.esp;
        let depth = (8 - ((OUT.sw >> 11) & 7) as usize) & 7;
        o.nst = depth as u32;
        o.st0 = (OUT.st0[0] as u64) | ((OUT.st0[1] as u64) << 32);
    }
    o.abort_addr = ABORT_ADDR;
    reset_pages_after(&o);
    MODE = 0;
    fpu_init();
    o
}

unsafe fn reset_pages_after(_o: &RunOut) {
    reset_pages();
}

extern "C" fn rust_runner(ent: *mut Ent, spec: *const FnSpec) {
    unsafe { ((*spec).body)(&mut *ent, ARGS_COPY.as_ptr()) }
}

static mut ARGS_COPY: [u32; 16] = [0; 16];

unsafe fn exec_rust(s: &Scn, spec: &FnSpec, preload: &[u32]) -> RunOut {
    prepare(s, spec, preload);
    RING_N = 0;
    HINTS.clear();
    INIT_REGS = s.regs;
    INIT_ARGS = s.args;
    MODE = 2;
    ARGS_COPY = s.args;
    let mut ent = Ent {
        eax: s.regs[0],
        ecx: s.regs[1],
        edx: s.regs[2],
        ebx: s.regs[3],
        esi: s.regs[4],
        edi: s.regs[5],
        ebp: s.regs[6],
        xmm0: s.xmm0,
        nst: 0,
        pad: 0,
        st0: 0.0,
    };
    ABORT_ADDR = 0;
    ABORT_KIND = 0;
    ACTIVE = true;
    RUN_START.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
    let ab = guarded(rust_runner as *const () as usize, &mut ent as *mut _ as usize, spec as *const _ as usize);
    let mut o = finish(ab, spec);
    if ab == 0 {
        o.regs = [ent.eax, ent.ecx, ent.edx, ent.ebx, ent.esi, ent.edi, ent.ebp];
        o.xmm0 = ent.xmm0;
        // translated ESP is at the return address slot; the original's is one dword above it
        o.esp = ent.pad + 4;
        o.nst = ent.nst;
        o.st0 = ent.st0.to_bits();
    }
    o.abort_addr = ABORT_ADDR;
    reset_pages();
    MODE = 0;
    fpu_init();
    o
}

// ---------------------------------------------------------------------------------------------
// comparison
// ---------------------------------------------------------------------------------------------

fn thread_stack() -> (u32, u32) {
    let base: u32;
    unsafe { std::arch::asm!("mov {}, fs:[4]", out(reg) base) };
    (base.saturating_sub(0x80_0000), base)
}

fn fmt_f(v: f64) -> String {
    format!("{v:e}/{:#018x}", v.to_bits())
}

fn feq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

unsafe fn compare(s: &Scn, spec: &FnSpec, o: &RunOut, r: &RunOut) -> Result<(), String> {
    if o.kind >= 3 || r.kind >= 3 {
        return Err("TIMEOUT".into());
    }
    for e in r.extra.iter() {
        return Err(format!("harness: {e}"));
    }
    // calls
    let n = o.calls.len().min(r.calls.len());
    for i in 0..n {
        let (a, b) = (&o.calls[i], &r.calls[i]);
        let what = |m: &str| format!("record #{i} ({} {:#x}, depth {}): {m}", if a.kind == 0 { "call" } else { "return" }, taps_addr(a.idx as usize), a.depth);
        if a.kind != b.kind || a.idx != b.idx {
            return Err(what(&format!("sequence differs: original {} {:#x} vs Rust {} {:#x}", a.kind, taps_addr(a.idx as usize), b.kind, taps_addr(b.idx as usize))));
        }
        // callees run on the emulated stack in the original and on the thread stack under the translation, so
        // below the first level a stack pointer in a register only has to be a stack pointer in both
        let norm = |v: u32, rust: bool| -> u32 {
            if a.depth == 0 && a.kind == 0 {
                return v;
            }
            let (lo, hi) = if rust { thread_stack() } else { (STK_LO as u32, STK_HI as u32) };
            if v >= lo && v < hi { 0x57AC_0000 } else { v }
        };
        let ra: Vec<u32> = a.regs.iter().map(|&v| norm(v, false)).collect();
        let rb: Vec<u32> = b.regs.iter().map(|&v| norm(v, true)).collect();
        // eax after `fnstsw ax` carries the x87 sticky exception flags / C1, which the translation does not model
        let (mut ra, mut rb) = (ra, rb);
        ra[0] &= !STICKY;
        rb[0] &= !STICKY;
        if ra != rb {
            return Err(what(&format!("registers eax ecx edx ebx esi edi: original {:x?} vs Rust {:x?}", a.regs, b.regs)));
        }
        if a.xmm0 != b.xmm0 && !(f32::from_bits(a.xmm0).is_nan() && f32::from_bits(b.xmm0).is_nan()) {
            return Err(what(&format!("xmm0: original {:#x} vs Rust {:#x}", a.xmm0, b.xmm0)));
        }
        if a.depth == 0 {
            if a.nst != b.nst || !(0..a.nst as usize).all(|k| feq(a.st[k], b.st[k])) {
                return Err(what(&format!(
                    "x87 stack: original {:?} vs Rust {:?}",
                    (0..a.nst as usize).map(|k| fmt_f(a.st[k])).collect::<Vec<_>>(),
                    (0..b.nst as usize).map(|k| fmt_f(b.st[k])).collect::<Vec<_>>()
                )));
            }
            if a.kind == 0 {
                if a.vesp != b.vesp {
                    return Err(what(&format!("ESP at the call: original {:#x} vs Rust {:#x}", a.vesp, b.vesp)));
                }
                if a.stack != b.stack {
                    let k = (0..16).find(|&k| a.stack[k] != b.stack[k]).unwrap();
                    return Err(what(&format!("stack dword {k} at ESP {:#x}: original {:#x} vs Rust {:#x} (first 8 {:x?} vs {:x?})", a.vesp, a.stack[k], b.stack[k], &a.stack[..8], &b.stack[..8])));
                }
            } else {
                if a.pops != b.pops {
                    return Err(what("popped bytes differ between runs".into()));
                }
                if b.depth == 0 && b.assumed_pop >= 0 && b.assumed_pop * 4 != b.pops {
                    return Err(what(&format!("callee popped {} bytes, the translation assumes {}", b.pops, b.assumed_pop * 4)));
                }
            }
        }
    }
    if o.calls.len() != r.calls.len() {
        let (long, w) = if o.calls.len() > r.calls.len() { (&o.calls, "original") } else { (&r.calls, "Rust") };
        let c = &long[n];
        return Err(format!("call count: original {} vs Rust {} ({w} continues with {} {:#x})", o.calls.len(), r.calls.len(), c.kind, taps_addr(c.idx as usize)));
    }
    // abort
    if o.aborted != r.aborted {
        return Err(format!("original aborted={} (addr {:#x}) vs Rust aborted={} (addr {:#x})", o.aborted, o.abort_addr, r.aborted, r.abort_addr));
    }
    if o.aborted != 0 && o.abort_addr != r.abort_addr {
        return Err(format!("both abort, at different addresses: original {:#x} vs Rust {:#x}", o.abort_addr, r.abort_addr));
    }
    // memory
    let mut pages: Vec<u32> = o.pages.iter().map(|p| p.0).chain(r.pages.iter().map(|p| p.0)).collect();
    pages.sort();
    pages.dedup();
    let mut pr = [0u8; 4096];
    for p in pages {
        let a = o.pages.iter().find(|x| x.0 == p).map(|x| x.1.as_slice());
        let b = r.pages.iter().find(|x| x.0 == p).map(|x| x.1.as_slice());
        let (a, b) = match (a, b) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => {
                fill_page(s, p as usize, &mut pr);
                (a, &pr[..])
            }
            (None, Some(b)) => {
                fill_page(s, p as usize, &mut pr);
                (&pr[..], b)
            }
            _ => continue,
        };
        if a != b {
            let k = (0..4096).find(|&k| a[k] != b[k]).unwrap() & !3;
            return Err(format!(
                "memory at {:#x}: original {:#x} vs Rust {:#x}",
                p as usize + k,
                u32::from_le_bytes(a[k..k + 4].try_into().unwrap()),
                u32::from_le_bytes(b[k..k + 4].try_into().unwrap())
            ));
        }
    }
    if o.aborted != 0 {
        return Ok(());
    }
    // exit state
    if o.esp != r.esp || o.esp != ESP0 + 4 + spec.ret_n {
        return Err(format!("ESP at the return: original {:#x}, Rust {:#x}, expected {:#x}", o.esp, r.esp, ESP0 + 4 + spec.ret_n));
    }
    let names = ["eax", "ecx", "edx", "ebx", "esi", "edi", "ebp"];
    for k in 0..7 {
        let m = if k == 0 { !STICKY } else { u32::MAX };
        if o.regs[k] & m != r.regs[k] & m {
            return Err(format!("exit {}: original {:#x} vs Rust {:#x}", names[k], o.regs[k], r.regs[k]));
        }
    }
    if o.xmm0 != r.xmm0 && !(f32::from_bits(o.xmm0).is_nan() && f32::from_bits(r.xmm0).is_nan()) {
        return Err(format!("exit xmm0: original {:#x} vs Rust {:#x}", o.xmm0, r.xmm0));
    }
    if o.nst != r.nst {
        return Err(format!("exit x87 depth: original {} vs Rust {}", o.nst, r.nst));
    }
    if o.nst >= 1 && !feq(f64::from_bits(o.st0), f64::from_bits(r.st0)) {
        return Err(format!("exit ST0: original {} vs Rust {}", fmt_f(f64::from_bits(o.st0)), fmt_f(f64::from_bits(r.st0))));
    }
    if o.argarea != r.argarea {
        let k = (0..o.argarea.len()).find(|&k| o.argarea[k] != r.argarea[k]).unwrap();
        return Err(format!("arguments area byte +{k:#x}: original {:#x} vs Rust {:#x}", o.argarea[k], r.argarea[k]));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// the tests
// ---------------------------------------------------------------------------------------------

macro_rules! fns {
    ($(($n:expr, $a:expr, $b:ident, $dt:expr, $rn:expr),)*) => {
        static FNS: &[FnSpec] = &[$(FnSpec { name: $n, start: $a, body: $b, ret_n: $rn, dt_arg: $dt },)*];
    };
}

fns! {
    ("camera_follow_update", 0x49b4b0, camera_follow_update_body, true, 0),
    ("camera_update", 0x4932f0, camera_update_body, false, 0),
    ("cam_498d40", 0x498d40, cam_498d40_body, false, 0),
    ("cam_499480", 0x499480, cam_499480_body, false, 0),
    ("cam_499190", 0x499190, cam_499190_body, false, 0),
    ("cam_497a20", 0x497a20, cam_497a20_body, false, 0),
    ("cam_4938c0", 0x4938c0, cam_4938c0_body, false, 0),
    ("cam_496be0", 0x496be0, cam_496be0_body, false, 0),
    ("cam_492df0", 0x492df0, cam_492df0_body, false, 0),
    ("cam_49ae60", 0x49ae60, cam_49ae60_body, false, 4),
    ("cam_49f320", 0x49f320, cam_49f320_body, false, 0),
    ("cam_49d7a0", 0x49d7a0, cam_49d7a0_body, false, 0),
}

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// The game's image range is usually taken by the test process's own heaps. This re-runs the test exe
/// as a suspended child, reserves the range in it before the loader creates the heaps, and relays the
/// child's exit code. Returns true when this process is the child (or the launch is unnecessary).
unsafe fn relaunch_with_reserved_range() -> Option<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::VirtualAllocEx;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, ResumeThread, WaitForSingleObject};
    #[repr(C)]
    struct ProcInfo {
        h_process: *mut std::ffi::c_void,
        h_thread: *mut std::ffi::c_void,
        pid: u32,
        tid: u32,
    }
    unsafe extern "system" {
        fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const u8, ta: *const u8, inherit: i32, flags: u32, env: *const u8, dir: *const u16, si: *const u32, pi: *mut ProcInfo) -> i32;
        fn GetCommandLineW() -> *mut u16;
    }
    if std::env::var("DT_CHILD").is_ok() {
        return None;
    }
    std::env::set_var("DT_CHILD", "1");
    let mut si = [0u32; 17];
    si[0] = 68;
    let mut pi = ProcInfo { h_process: std::ptr::null_mut(), h_thread: std::ptr::null_mut(), pid: 0, tid: 0 };
    let ok = CreateProcessW(std::ptr::null(), GetCommandLineW(), std::ptr::null(), std::ptr::null(), 1, 4, std::ptr::null(), std::ptr::null(), si.as_ptr(), &mut pi);
    assert!(ok != 0, "CreateProcess failed");
    for (lo, hi) in [(IMG_LO, IMG_HI), (ARENA, ARENA + ARENA_SIZE), (STK_LO, STK_HI), (TH_BASE, TH_BASE + 0x20000)] {
        let p = VirtualAllocEx(pi.h_process, lo as *const _, hi - lo, MEM_RESERVE, PAGE_NOACCESS);
        assert_eq!(p as usize, lo, "cannot reserve {lo:#x} in the child");
    }
    ResumeThread(pi.h_thread);
    WaitForSingleObject(pi.h_process, INFINITE);
    let mut code = 0u32;
    GetExitCodeProcess(pi.h_process, &mut code);
    CloseHandle(pi.h_process);
    CloseHandle(pi.h_thread);
    Some(code)
}

#[test]
#[ignore = "needs the test exe linked at /BASE:0x10000000 (see the module doc)"]
fn camera_functions_match_original() {
    unsafe {
        if let Some(code) = relaunch_with_reserved_range() {
            assert_eq!(code, 0, "the test child process failed (exit code {code:#x})");
            return;
        }
        init_universe();
        let filt = std::env::var("DT_FN").unwrap_or_default();
        let nscn = env_u64("DT_N", 2000);
        let base_seed = env_u64("DT_SEED", 1);
        let mut total_bad = 0;
        let verbose = std::env::var("DT_VERBOSE").is_ok();
        for spec in FNS.iter().filter(|f| filt.is_empty() || f.name.contains(&filt)) {
            install_taps(spec.start);
            COVMAP.fill(0);
            let mut corpus: Vec<(Scn, Vec<u32>)> = Vec::new();
            let (mut ok, mut ab_ok, mut bad, mut calls_total, mut timeouts) = (0u32, 0u32, 0u32, 0usize, 0u32);
            let mut first_bad: Option<(u64, String)> = None;
            let mut rng = Rng(mix(base_seed ^ spec.start as u64));
            let mut queue: Vec<Scn> = Vec::new();
            for it in 0..nscn {
                let seed = base_seed * 1_000_003 + it;
                let mut s = if !queue.is_empty() && rng.chance(70) {
                    queue.pop().unwrap()
                } else if !corpus.is_empty() && rng.chance(60) {
                    let (b, t) = &corpus[rng.below(corpus.len() as u32) as usize];
                    mutate_scn(b, &mut rng, t, spec)
                } else {
                    gen_scn(seed, spec)
                };
                COV_NEW = 0;
                if verbose {
                    eprintln!("{} scenario {it}", spec.name);
                }
                // repair loop on the translation alone: a fault through a bad pointer is turned into a valid
                // pointer at the place the value came from, so that the run gets further
                let mut repairs = 0;
                loop {
                    let r = exec_rust(&s, spec, &[]);
                    if r.aborted != 0 && ABORT_IN >= 0 && (ABORT_IN as usize) < CALLEE_STATS.len() {
                        CALLEE_STATS[ABORT_IN as usize].1 += 1;
                    }
                    if r.kind == 1 && repairs < 10 {
                        if let Some(h) = repair_hint(r.abort_addr, &mut rng) {
                            apply_hint(&mut s, h);
                            repairs += 1;
                            continue;
                        }
                    }
                    break;
                }
                // children from the compare hints of the final run
                let r_hints = HINTS.clone();
                let (o, pre) = exec_orig(&s, spec);
                if verbose {
                    eprintln!("  orig done: aborted {} kind {} calls {}", o.aborted, o.kind, o.calls.len());
                }
                let r = exec_rust(&s, spec, &pre);
                if verbose {
                    eprintln!("  rust done: aborted {} kind {} calls {}", r.aborted, r.kind, r.calls.len());
                }
                calls_total += o.calls.len();
                match compare(&s, spec, &o, &r) {
                    Ok(()) => {
                        if o.aborted != 0 {
                            ab_ok += 1;
                        } else {
                            ok += 1;
                        }
                    }
                    Err(e) if e == "TIMEOUT" => timeouts += 1,
                    Err(e) => {
                        bad += 1;
                        if first_bad.is_none() {
                            if std::env::var("DT_TRACE").is_ok() {
                                println!("   regs {:x?}\n   args {:x?}\n   rust trace {:x?}", s.regs, &s.args[..6], TRACE);
                            }
                            first_bad = Some((it, e));
                        }
                    }
                }
                if !r_hints.is_empty() && queue.len() < 200 {
                    for _ in 0..3 {
                        let mut c = s.clone();
                        for _ in 0..(1 + rng.below(2)) {
                            apply_hint(&mut c, r_hints[rng.below(r_hints.len() as u32) as usize]);
                        }
                        queue.push(c);
                    }
                }
                if COV_NEW > 0 {
                    let t: Vec<u32> = o.pages.iter().chain(r.pages.iter()).map(|p| p.0).collect();
                    corpus.push((s, t));
                    if corpus.len() > 300 {
                        corpus.remove(0);
                    }
                }
            }
            let arms = ARMS.iter().find(|a| a.0 == spec.name).map(|a| a.2).unwrap_or(&[]);
            let hit = arms.iter().filter(|&&a| COVMAP[a as usize - COV_BASE] != 0).count();
            let missing: Vec<String> = arms.iter().filter(|&&a| COVMAP[a as usize - COV_BASE] == 0).map(|a| format!("{a:#x}")).collect();
            println!(
                "{:22} scenarios {nscn}: pass {ok} (+{ab_ok} consistent aborts), FAIL {bad}, timeouts {timeouts}, calls {calls_total}, blocks {hit}/{}",
                spec.name,
                arms.len()
            );
            if std::env::var("DT_STATS").is_ok() {
                let mut v: Vec<(usize, (u32, u32))> = CALLEE_STATS.iter().cloned().enumerate().filter(|x| x.1.1 > 0).collect();
                v.sort_by_key(|x| std::cmp::Reverse(x.1.1));
                for (i, (c, a)) in v.iter().take(12) {
                    println!("   callee {:#x}: {c} calls, {a} runs aborted inside", taps_addr(*i));
                }
                for c in CALLEE_STATS.iter_mut() {
                    *c = (0, 0);
                }
            }
            if std::env::var("DT_MISSING").is_ok() {
                println!("   missing blocks: {}", missing.join(" "));
            }
            if let Some((seed, e)) = first_bad {
                println!("   first failure (scenario #{seed}): {e}");
            }
            total_bad += bad;
        }
        assert_eq!(total_bad, 0, "mismatches found");
    }
}
