//! Offline differential test rig for port_physics.rs (test builds only).
//!
//! The game's data lives at `DELTA` above its real addresses (the low address space of a test
//! process is taken by system mappings). The original function bytes come from the unpacked exe,
//! relocated to a private copy whose absolute data references are shifted by `DELTA` and whose
//! calls are aimed at recording stubs (or at other relocated originals); the Rust ports see the
//! same world through `g()`-shifted constants. The original and the port (through its real naked
//! thunk) run on the same random world and are compared: return registers, ST0, every mapped
//! byte, and the stream of calls into the rest of the game.
//!
//! Run:  RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//!       -p sr2-hook --target i686-pc-windows-msvc port_physics -- --test-threads=1 --nocapture

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

pub const EAX: u8 = 1;
pub const ECX: u8 = 2;
pub const EDX: u8 = 4;
pub const EBX: u8 = 8;
pub const ESI: u8 = 16;
pub const EDI: u8 = 32;
pub const EBP: u8 = 64;
const ALL_SAVED: u8 = EBX | ESI | EDI | EBP;

pub struct Rng(pub u64);
impl Rng {
    pub fn u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn below(&mut self, n: u32) -> u32 {
        self.u32() % n
    }
    pub fn one_in(&mut self, n: u32) -> bool {
        self.below(n) == 0
    }
    /// One of `v`, or one time in four a random word.
    pub fn mix(&mut self, v: &[u32]) -> u32 {
        if self.below(4) == 0 { self.u32() } else { self.pick(v) }
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u32) as usize]
    }
    /// A float bit pattern with the awkward cases over-represented.
    pub fn float_bits(&mut self) -> u32 {
        match self.below(14) {
            0 => 0,
            1 => 0x8000_0000,
            2 => 0x3F80_0000,
            3 => 0xBF80_0000,
            4 => 0x7FC0_0000,
            5 => 0x7F80_0001,
            6 => 0x7F80_0000,
            7 => 0x0000_0001,
            8 | 9 => (self.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
            10 | 11 => (self.below(20000) as f32 / 10.0 - 100.0).to_bits(),
            12 => 0x3727_C5AC, // 1e-5
            _ => self.u32(),
        }
    }
}

// --- the mapped world ---------------------------------------------------------------------------

struct Chunk {
    base: usize,
    data: bool,
}
static CHUNKS: Mutex<Vec<Chunk>> = Mutex::new(Vec::new());

const CH: usize = 0x1_0000;

fn ensure(addr: usize, len: usize, data: bool) {
    let mut c = CHUNKS.lock().unwrap();
    let mut p = addr & !(CH - 1);
    while p < addr + len {
        if !c.iter().any(|x| x.base == p) {
            let q = unsafe { VirtualAlloc(p as *const _, CH, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
            assert_eq!(q, p, "cannot map {p:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
            c.push(Chunk { base: p, data });
        } else if data {
            c.iter_mut().find(|x| x.base == p).unwrap().data = true;
        }
        p += CH;
    }
}

/// Where the game's data lives in the test process: real address + DELTA.
pub const DELTA: usize = 0x5000_0000;

/// Map `[addr, addr+len)` (already shifted); it is filled with random words by `randomize`.
pub fn map_data(addr: usize, len: usize) {
    ensure(addr, len, true);
}

fn exe() -> &'static Vec<u8> {
    static EXE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::fs::read(r"F:\Saints Row 2 Decomp\bin\SR2_pc.unpacked.exe").unwrap())
}

/// The bytes of the unpacked exe at the virtual address `va`.
pub fn exe_bytes(va: usize, len: usize) -> Vec<u8> {
    let e = exe();
    let pe = u32::from_le_bytes(e[0x3C..0x40].try_into().unwrap()) as usize;
    let nsec = u16::from_le_bytes(e[pe + 6..pe + 8].try_into().unwrap()) as usize;
    let opt = u16::from_le_bytes(e[pe + 20..pe + 22].try_into().unwrap()) as usize;
    for i in 0..nsec {
        let s = pe + 24 + opt + i * 40;
        let g = |o: usize| u32::from_le_bytes(e[s + o..s + o + 4].try_into().unwrap()) as usize;
        let (vsz, va0, rsz, raw) = (g(8), g(12), g(16), g(20));
        let a = 0x400000 + va0;
        if va >= a && va + len <= a + vsz.max(rsz) {
            return e[raw + va - a..raw + va - a + len].to_vec();
        }
    }
    panic!("{va:#x} is not in a section");
}

pub unsafe fn put(a: usize, v: u32) {
    unsafe { *(a as *mut u32) = v }
}
pub unsafe fn putb(a: usize, v: u8) {
    unsafe { *(a as *mut u8) = v }
}
pub unsafe fn puth(a: usize, v: u16) {
    unsafe { *(a as *mut u16) = v }
}
pub unsafe fn get(a: usize) -> u32 {
    unsafe { *(a as *const u32) }
}

fn snapshot() -> Vec<u8> {
    let c = CHUNKS.lock().unwrap();
    let mut v = Vec::with_capacity(c.len() * CH);
    for x in c.iter() {
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(x.base as *const u8, CH) });
    }
    v
}

fn restore(s: &[u8]) {
    let c = CHUNKS.lock().unwrap();
    for (i, x) in c.iter().enumerate() {
        unsafe { std::ptr::copy_nonoverlapping(s[i * CH..(i + 1) * CH].as_ptr(), x.base as *mut u8, CH) };
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<(usize, u8, u8, usize)> {
    let c = CHUNKS.lock().unwrap();
    let mut first = None;
    let mut n = 0;
    for (i, x) in c.iter().enumerate() {
        for k in 0..CH {
            if a[i * CH + k] != b[i * CH + k] {
                n += 1;
                if n <= 24 {
                    eprintln!("  diff at {:#x}: original {:#04x} port {:#04x}", x.base + k, a[i * CH + k], b[i * CH + k]);
                }
                if first.is_none() {
                    first = Some((x.base + k, a[i * CH + k], b[i * CH + k]));
                }
            }
        }
    }
    first.map(|(p, x, y)| (p, x, y, n))
}

/// Refill every data chunk with random words.
pub fn randomize(rng: &mut Rng) {
    let c = CHUNKS.lock().unwrap();
    for x in c.iter().filter(|x| x.data) {
        let s = unsafe { std::slice::from_raw_parts_mut(x.base as *mut u32, CH / 4) };
        for w in s.iter_mut() {
            *w = rng.u32();
        }
    }
}

/// Zero `len` bytes at `a`.
pub fn zero(a: usize, len: usize) {
    unsafe { std::ptr::write_bytes(a as *mut u8, 0, len) };
}

// --- stubs for the callees ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum Ret {
    /// A random value comes back.
    Any,
    Zero,
    Const(u32),
    /// EAX comes back unchanged (the cookie check).
    Keep,
    /// A flag: 0, 1, 0x101 (AL set, the rest not), 0x100, 2, or a random word.
    Flag,
}

#[derive(Clone)]
pub struct Callee {
    pub addr: usize,
    /// Stack arguments logged.
    pub nargs: usize,
    /// Bytes the callee pops.
    pub pop: u16,
    /// Registers logged (the ones the callee reads).
    pub reads: u8,
    pub ret: Ret,
    /// ST0 comes back (a random f32).
    pub st0: bool,
    /// Random bytes written at the pointer in stack argument `.0` (or a register when `.0` >= 100:
    /// 100 + index in the order eax, ecx, edx, ebx, esi, edi), `.1` bytes.
    pub fill: Vec<(usize, usize)>,
    /// Memory logged too: (stack argument index, dwords) at the pointer in that argument.
    pub peek: Vec<(usize, usize)>,
    /// Stack arguments (bit k = argument k) the callee reads as a byte: only the low byte is logged.
    pub byte_args: u32,
    /// Runs after the log: may write memory and replace the result.
    pub hook: Option<fn(&Pushad, *const u32) -> Option<u32>>,
    pub silent: bool,
    /// XMM0 comes back (a random f32), not ST0.
    pub xmm0: bool,
    /// With `st0`: a small value (-1.9 .. 28, a few NaN / infinities).
    pub st0_small: bool,
}

pub fn callee(addr: usize, nargs: usize, pop: u16, reads: u8, ret: Ret) -> Callee {
    Callee { addr, nargs, pop, reads, ret, st0: false, fill: Vec::new(), peek: Vec::new(), byte_args: 0, hook: None, silent: false, xmm0: false, st0_small: false }
}

static CALLEES: Mutex<Vec<Callee>> = Mutex::new(Vec::new());
static LOG: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
static SRNG: AtomicU64 = AtomicU64::new(1);
static ST0_VAL: AtomicU32 = AtomicU32::new(0);

/// Original address -> where a call to it really goes (a stub, or a relocated original).
static REDIRECT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
/// The stub address of each callee, in `CALLEES` order.
static STUBS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Where a call to the game address `f` should really go.
pub fn redirect(f: usize) -> usize {
    REDIRECT.lock().unwrap().iter().find(|x| x.0 == f).map(|x| x.1).unwrap_or(f)
}

/// The address of the stub for the callee registered for game address `addr` (to store in the
/// world as a function pointer).
pub fn stub_of(addr: usize) -> usize {
    redirect(addr)
}

fn exec_alloc(len: usize) -> usize {
    let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
    assert!(p != 0);
    p
}

unsafe extern "system" fn veh(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    unsafe {
        let rec = &*(*info).ExceptionRecord;
        let ctx = &*(*info).ContextRecord;
        eprintln!(
            "EXCEPTION {:#x} at eip {:#x} access {:x?} eax {:#x} ecx {:#x} edx {:#x} ebx {:#x} esi {:#x} edi {:#x} ebp {:#x} esp {:#x}",
            rec.ExceptionCode, ctx.Eip, &rec.ExceptionInformation[..2], ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx, ctx.Esi, ctx.Edi, ctx.Ebp, ctx.Esp
        );
        if std::env::var("BT").is_ok() {
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
        }
        if std::env::var("DUMPF").is_ok() {
            for k in 0..24usize {
                eprintln!("  F+{:#x}: {:#x}", 0x10 + k * 4, *((ctx.Esp as usize + 12 + 0x10 + k * 4) as *const u32));
            }
        }
    }
    0
}

/// Forget the previous test's stubs and loaded originals.
pub fn reset() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(veh));
    });
    REDIRECT.lock().unwrap().clear();
    CALLEES.lock().unwrap().clear();
    STUBS.lock().unwrap().clear();
}

/// Register the callees: each gets a stub, and calls to its game address are redirected to it.
pub fn set_callees(c: Vec<Callee>) {
    let mem = exec_alloc(c.len() * 16 + 16);
    let mut stubs = Vec::new();
    for (i, x) in c.iter().enumerate() {
        let a = mem + i * 16;
        let mut code = [0x90u8; 16];
        code[0] = 0x68;
        code[1..5].copy_from_slice(&(i as u32).to_le_bytes());
        code[5] = 0xE8;
        let target = if x.xmm0 { stub_common_xmm0 as usize } else if x.st0 { stub_common_st0 as usize } else { stub_common as usize };
        let rel = (target as i32).wrapping_sub((a + 10) as i32);
        code[6..10].copy_from_slice(&rel.to_le_bytes());
        if x.pop == 0 {
            code[10] = 0xC3;
        } else {
            code[10] = 0xC2;
            code[11..13].copy_from_slice(&x.pop.to_le_bytes());
        }
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), a as *mut u8, 16) };
        stubs.push(a);
        REDIRECT.lock().unwrap().push((x.addr, a));
    }
    *STUBS.lock().unwrap() = stubs;
    *CALLEES.lock().unwrap() = c;
}

/// Load the original function at game address `base` into a private copy: `fixes` are offsets of
/// absolute data addresses (shifted by DELTA), `calls` the `call rel32`s and `tails` the `jmp
/// rel32`s out of the body ((offset, target)) re-aimed through `redirect`, `tables` the (offset of
/// the `jmp [reg*4 + table]` displacement, entry count) whose tables are copied with the entries
/// moved into the copy. Calls to this function are redirected to the copy too.
pub fn load_orig(base: usize, body: &[u8], calls: &[usize], fixes: &[usize], tails: &[(usize, usize)], tables: &[(usize, usize)]) -> usize {
    let copy = exec_alloc(body.len() + 16);
    unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), copy as *mut u8, body.len()) };
    REDIRECT.lock().unwrap().push((base, copy)); // recursion goes to the copy
    let rd32 = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap());
    for &o in fixes {
        unsafe { std::ptr::write_unaligned((copy + o) as *mut u32, rd32(o).wrapping_add(DELTA as u32)) };
    }
    let aim = |o: usize, dest: usize| {
        let r = (dest as i32).wrapping_sub((copy + o + 5) as i32);
        unsafe { std::ptr::write_unaligned((copy + o + 1) as *mut i32, r) };
    };
    for &o in calls {
        let target = (base + o + 5).wrapping_add(rd32(o + 1) as usize);
        let d = redirect(target);
        assert!(d != target, "call at {o:#x} to {target:#x} has no callee entry");
        aim(o, d);
    }
    for &(o, t) in tails {
        let d = redirect(t);
        assert!(d != t, "tail jump at {o:#x} to {t:#x} has no callee entry");
        aim(o, d);
    }
    for &(o, n) in tables {
        let table = rd32(o) as usize;
        let tb = exe_bytes(table, n * 4);
        let mem = exec_alloc(n * 4 + 16);
        for k in 0..n {
            let e = u32::from_le_bytes(tb[k * 4..k * 4 + 4].try_into().unwrap()) as usize;
            assert!(e >= base && e < base + body.len(), "table entry {e:#x} outside the body");
            unsafe { std::ptr::write_unaligned((mem + k * 4) as *mut u32, (copy + (e - base)) as u32) };
        }
        unsafe { std::ptr::write_unaligned((copy + o) as *mut u32, mem as u32) };
    }
    REDIRECT.lock().unwrap().push((base, copy));
    copy
}

fn srng() -> u32 {
    let mut r = Rng(SRNG.load(Ordering::Relaxed));
    let v = r.u32();
    SRNG.store(r.0, Ordering::Relaxed);
    v
}

#[repr(C)]
pub struct Pushad {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
}

/// Add a line to the call log (a hook records memory the callee would read).
pub fn log_extra(v: Vec<u32>) {
    LOG.lock().unwrap().push(v);
}

/// A pseudo-random word for the hooks (the same sequence in the original's run and the port's).
pub fn hook_rand() -> u32 {
    srng()
}

static STACK_LO: AtomicU32 = AtomicU32::new(0);
static STACK_HI: AtomicU32 = AtomicU32::new(0);

extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
    let c = CALLEES.lock().unwrap()[idx as usize].clone();
    let r = unsafe { &*regs };
    if std::env::var("STUBV").is_ok() {
        eprintln!("  stub {:#x} eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} a0 {:x?}", c.addr, r.eax, r.ecx, r.edx, r.esi, r.edi, unsafe { [*args, *args.add(1), *args.add(2), *args.add(3)] });
    }
    let (lo, hi) = (STACK_LO.load(Ordering::Relaxed), STACK_HI.load(Ordering::Relaxed));
    let mask = |v: u32| {
        if v >= lo && v < hi {
            0x57AC_0000
        } else if (v as usize) >= DELTA + 0xD0_0000 && (v as usize) < DELTA + 0x340_0000 {
            v - DELTA as u32
        } else {
            v
        }
    };
    if !c.silent {
        let mut e = vec![c.addr as u32];
        for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (EBX, r.ebx), (ESI, r.esi), (EDI, r.edi)] {
            e.push(if c.reads & bit != 0 { mask(v) } else { 0 });
        }
        for k in 0..c.nargs {
            let v = unsafe { *args.add(k) };
            e.push(if c.byte_args >> k & 1 != 0 { v & 0xFF } else { mask(v) });
        }
        for &(a, n) in &c.peek {
            let p = unsafe { *args.add(a) } as usize;
            for k in 0..n {
                e.push(unsafe { *((p + k * 4) as *const u32) });
            }
        }
        LOG.lock().unwrap().push(e);
    }
    for &(src, len) in &c.fill {
        let p = if src >= 100 {
            [r.eax, r.ecx, r.edx, r.ebx, r.esi, r.edi][src - 100]
        } else {
            unsafe { *args.add(src) }
        };
        if len % 4 == 0 {
            // dwords: floats with the awkward cases over-represented
            let mut g = Rng(srng() as u64 | 1 << 40);
            for k in 0..len / 4 {
                unsafe { *((p as usize + k * 4) as *mut u32) = g.float_bits() };
            }
        } else {
            for k in 0..len {
                unsafe { *((p as usize + k) as *mut u8) = srng() as u8 };
            }
        }
    }
    if c.st0 || c.xmm0 {
        let mut g = Rng(srng() as u64 | 1 << 40);
        let b = if c.st0_small {
            match g.below(16) {
                0 => f32::NAN.to_bits(),
                1 => f32::INFINITY.to_bits(),
                2 => f32::NEG_INFINITY.to_bits(),
                _ => (g.below(300) as f32 / 10.0 - 1.9).to_bits(),
            }
        } else {
            match g.below(4) {
            0 => g.float_bits(),
            _ => (g.below(4000) as f32 / 100.0 - 20.0).to_bits(),
            }
        };
        ST0_VAL.store(b, Ordering::Relaxed);
    }
    if let Some(h) = c.hook {
        if let Some(v) = h(r, args) {
            return v;
        }
    }
    match c.ret {
        Ret::Zero => 0,
        Ret::Any => srng(),
        Ret::Const(v) => v,
        Ret::Keep => r.eax,
        Ret::Flag => match srng() % 6 {
            0 => 0,
            1 | 2 => 1,
            3 => 0x101,
            4 => 0x100,
            _ => srng(),
        },
    }
}

#[unsafe(naked)]
extern "C" fn stub_common() {
    std::arch::naked_asm!(
        "pushad",
        "lea eax, [esp + 44]",
        "push eax",
        "push dword ptr [esp + 40]",
        "lea eax, [esp + 8]",
        "push eax",
        "call {f}",
        "add esp, 12",
        "mov [esp + 28], eax",
        "popad",
        "ret 4",
        f = sym stub_rs,
    )
}

#[unsafe(naked)]
extern "C" fn stub_common_st0() {
    std::arch::naked_asm!(
        "pushad",
        "lea eax, [esp + 44]",
        "push eax",
        "push dword ptr [esp + 40]",
        "lea eax, [esp + 8]",
        "push eax",
        "call {f}",
        "add esp, 12",
        "mov [esp + 28], eax",
        "popad",
        "fld dword ptr [{v}]",
        "ret 4",
        f = sym stub_rs,
        v = sym ST0_VAL,
    )
}

#[unsafe(naked)]
extern "C" fn stub_common_xmm0() {
    std::arch::naked_asm!(
        "pushad",
        "lea eax, [esp + 44]",
        "push eax",
        "push dword ptr [esp + 40]",
        "lea eax, [esp + 8]",
        "push eax",
        "call {f}",
        "add esp, 12",
        "mov [esp + 28], eax",
        "popad",
        "movss xmm0, dword ptr [{v}]",
        "ret 4",
        f = sym stub_rs,
        v = sym ST0_VAL,
    )
}

// --- running --------------------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Regs {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
}

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    esp: u32,
    st0: [u32; 2],
    want_st0: u32,
    n: u32,
    args: [u32; 16],
}

static mut TARGET: u32 = 0;
static mut BLKP: u32 = 0;

/// Call `f` with all seven registers and the stack arguments; capture the registers after (and
/// ST0 as an f64 when `want_st0`). `tls`: the value for `fs:[0x2c]`'s first slot (0: untouched).
pub unsafe fn call_with(f: usize, r: &mut Regs, args: &[u32], want_st0: bool, tls: u32) -> f64 {
    let mut b = Block { f: f as u32, r: *r, esp: 0, st0: [0; 2], want_st0: want_st0 as u32, n: args.len() as u32, args: [0; 16] };
    b.args[..args.len()].copy_from_slice(args);
    let local = 0u32;
    STACK_LO.store((&local as *const u32 as u32).wrapping_sub(0x40000), Ordering::Relaxed);
    STACK_HI.store((&local as *const u32 as u32).wrapping_add(0x40000), Ordering::Relaxed);
    // fs:[0x2c] -> a one-slot array holding `tls`
    let slot = [tls];
    let saved_tls: u32;
    unsafe {
        std::arch::asm!("mov {0}, fs:[0x2c]", out(reg) saved_tls);
        if tls != 0 {
            std::arch::asm!("mov fs:[0x2c], {0}", in(reg) slot.as_ptr());
        }
        TARGET = f as u32;
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 32], esp",
            "mov [{blkp}], eax",
            "mov ecx, [eax + 48]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [eax + 48 + ecx*4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [{target}]",
            "xchg ebx, [{blkp}]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "mov eax, [{blkp}]",
            "mov [ebx + 16], eax",
            "cmp dword ptr [ebx + 44], 0",
            "je 4f",
            "fstp qword ptr [ebx + 36]",
            "4:",
            "mov esp, [ebx + 32]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            target = sym TARGET,
            blkp = sym BLKP,
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
        if tls != 0 {
            std::arch::asm!("mov fs:[0x2c], {0}", in(reg) saved_tls);
        }
    }
    *r = b.r;
    f64::from_bits(b.st0[0] as u64 | (b.st0[1] as u64) << 32)
}

pub struct Case {
    pub regs: Regs,
    pub args: Vec<u32>,
    pub want_st0: bool,
    pub tls: u32,
    /// Registers (besides EBX/ESI/EDI/EBP, always compared with their inputs) whose values after
    /// the call must match between the original and the port.
    pub out: u8,
    /// Seed for the stubs' random results.
    pub seed: u64,
}

impl Case {
    pub fn new(seed: u64) -> Case {
        Case {
            regs: Regs { eax: 0xAAAA_0001, ecx: 0xCCCC_0002, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 },
            args: Vec::new(),
            want_st0: false,
            tls: 0,
            out: EAX,
            seed,
        }
    }
}

fn run1(f: usize, c: &Case) -> (Regs, f64, Vec<Vec<u32>>) {
    SRNG.store(c.seed | 1, Ordering::Relaxed);
    LOG.lock().unwrap().clear();
    let mut r = c.regs;
    let st = unsafe { call_with(f, &mut r, &c.args, c.want_st0, c.tls) };
    let log = std::mem::take(&mut *LOG.lock().unwrap());
    (r, st, log)
}

/// One case on the current memory: the original (at `orig`), then the port (at `port`) from the
/// same snapshot. Returns the original's registers.
pub fn check(orig: usize, port: usize, c: &Case) -> Result<Regs, String> {
    let snap = snapshot();
    let (ra, sa, la) = run1(orig, c);
    let after_a = snapshot();
    restore(&snap);
    let (rb, sb, lb) = run1(port, c);
    let after_b = snapshot();
    let names = ["eax", "ecx", "edx", "ebx", "esi", "edi", "ebp"];
    let get = |r: &Regs, i: usize| [r.eax, r.ecx, r.edx, r.ebx, r.esi, r.edi, r.ebp][i];
    for i in 0..7 {
        let bit = 1u8 << i;
        if (c.out | ALL_SAVED) & bit == 0 {
            continue;
        }
        if ALL_SAVED & bit != 0 && c.out & bit == 0 {
            if get(&ra, i) != get(&c.regs, i) {
                return Err(format!("the original changed {}: {:#x} -> {:#x}", names[i], get(&c.regs, i), get(&ra, i)));
            }
            if get(&rb, i) != get(&c.regs, i) {
                return Err(format!("the port changed {}: {:#x} -> {:#x}", names[i], get(&c.regs, i), get(&rb, i)));
            }
        } else if get(&ra, i) != get(&rb, i) {
            return Err(format!("{}: original {:#x}, port {:#x} (in {:#x})", names[i], get(&ra, i), get(&rb, i), get(&c.regs, i)));
        }
    }
    if c.want_st0 && sa.to_bits() != sb.to_bits() && !(sa.is_nan() && sb.is_nan()) {
        return Err(format!("ST0: original {sa:e} ({:#x}), port {sb:e} ({:#x})", sa.to_bits(), sb.to_bits()));
    }
    for (i, (a, b)) in la.iter().zip(&lb).enumerate() {
        if a != b {
            return Err(format!("call #{i} differs:\n  original {a:x?}\n  port     {b:x?}"));
        }
    }
    if la.len() != lb.len() {
        let longer = if la.len() > lb.len() { ("original", &la) } else { ("port", &lb) };
        return Err(format!("original made {} calls, port {}; {} next calls {:x?}", la.len(), lb.len(), longer.0, longer.1[la.len().min(lb.len())]));
    }
    if let Some((p, x, y, n)) = first_diff(&after_a, &after_b) {
        return Err(format!("memory differs at {p:#x}: original {x:#x}, port {y:#x} ({n} bytes differ)"));
    }
    // leave the world as the original left it
    restore(&after_a);
    Ok(ra)
}

/// Where the test world's objects live (outside the game's range).
pub const ARENA: usize = 0x6000_0000;
pub const ARENA_LEN: usize = 0x10_0000;

pub fn map_arena() {
    ensure(ARENA, ARENA_LEN, true);
}

/// A bump allocator over the arena.
pub struct Arena(pub usize);
impl Arena {
    pub fn new() -> Arena {
        Arena(ARENA)
    }
    pub fn alloc(&mut self, size: usize) -> usize {
        let p = self.0;
        self.0 += (size + 15) & !15;
        assert!(self.0 <= ARENA + ARENA_LEN, "arena full");
        p
    }
}

/// A short report of what occupies the address ranges the game needs.
pub fn dump_occupied() {
    use windows_sys::Win32::System::Memory::{MEM_FREE, MEMORY_BASIC_INFORMATION, VirtualQuery};
    let mut a = 0x40_0000usize;
    while a < 0x400_0000 {
        let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { VirtualQuery(a as *const _, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
            break;
        }
        if m.State != MEM_FREE {
            eprintln!("occupied {:#x}..{:#x} state {:#x} type {:#x} allocbase {:#x}", m.BaseAddress as usize, m.BaseAddress as usize + m.RegionSize, m.State, m.Type, m.AllocationBase as usize);
        }
        a = m.BaseAddress as usize + m.RegionSize;
    }
}

pub fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}
