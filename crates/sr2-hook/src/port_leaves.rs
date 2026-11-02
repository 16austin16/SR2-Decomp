//! Port module: small leaf functions of the exe (no calls, one `ret`), found by scanning
//! `docs/functions.csv` for bodies that end at their first `ret`, hold no call / outside jump /
//! x87 / SSE, and are not in another port file. Mostly vtable stubs (constant results, field
//! getters, `ret N` no-ops), Lua binding flags, cheat toggles and a few hot LTCG helpers.
//!
//! Every port goes through a `pushad` thunk (`ret` or `ret N` as the original), so every
//! register comes back as the caller left it except those the Rust body sets. The Rust bodies
//! reproduce the original's final register values exactly (scratch ECX / EDX included), because
//! the shadow compares all seven registers.
//!
//! Shadow check: for the first `SHADOW_CALLS` calls of each port a relocated copy of the original
//! runs on the same registers and stack arguments. For the ports that write memory, the exact
//! bytes they write are snapshotted first: original runs, its result is saved, the snapshot is
//! restored, the Rust version runs, and registers and written bytes are compared. On a mismatch
//! the original's registers and writes are kept.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;
const RAND_INDEX: usize = 0x00EC_3ABC;
const RAND_TABLE: usize = 0x00EC_3AC0;

// --- registers ------------------------------------------------------------------------------------

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
struct Pushad {
    edi: u32,
    esi: u32,
    ebp: u32,
    #[allow(dead_code)] // never read
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// The seven general registers.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Regs {
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

impl Regs {
    fn of(p: &Pushad) -> Regs {
        Regs { eax: p.eax, ecx: p.ecx, edx: p.edx, ebx: p.ebx, esi: p.esi, edi: p.edi, ebp: p.ebp }
    }
    fn apply(&self, p: &mut Pushad) {
        p.eax = self.eax;
        p.ecx = self.ecx;
        p.edx = self.edx;
        p.ebx = self.ebx;
        p.esi = self.esi;
        p.edi = self.edi;
        p.ebp = self.ebp;
    }
}

/// Offsets used by the `call_full_*` asm: f 0, regs 4..32, args 32..64, saved esp 64.
#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    args: [u32; 8],
    esp: u32,
}

// --- calling the original copy ---------------------------------------------------------------
/// Call `f` with all seven registers from `r` and 0 stack argument dword(s) (the callee pops 0 bytes), then capture all seven registers into `r`.
unsafe fn call_full_0_0(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..0 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 0]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 1 stack argument dword(s) (the callee pops 0 bytes), then capture all seven registers into `r`.
unsafe fn call_full_1_0(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..1 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 4]",
            "push ebx",
            "mov ebx, [esp + 12]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 1 stack argument dword(s) (the callee pops 4 bytes), then capture all seven registers into `r`.
unsafe fn call_full_1_4(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..1 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 4]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 2 stack argument dword(s) (the callee pops 0 bytes), then capture all seven registers into `r`.
unsafe fn call_full_2_0(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..2 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 8]",
            "push ebx",
            "mov ebx, [esp + 16]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 2 stack argument dword(s) (the callee pops 8 bytes), then capture all seven registers into `r`.
unsafe fn call_full_2_8(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..2 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 8]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 3 stack argument dword(s) (the callee pops 12 bytes), then capture all seven registers into `r`.
unsafe fn call_full_3_12(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..3 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 12]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 4 stack argument dword(s) (the callee pops 16 bytes), then capture all seven registers into `r`.
unsafe fn call_full_4_16(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..4 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 44]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 16]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 5 stack argument dword(s) (the callee pops 20 bytes), then capture all seven registers into `r`.
unsafe fn call_full_5_20(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..5 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 48]",
            "push dword ptr [eax + 44]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 20]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 6 stack argument dword(s) (the callee pops 24 bytes), then capture all seven registers into `r`.
unsafe fn call_full_6_24(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..6 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 52]",
            "push dword ptr [eax + 48]",
            "push dword ptr [eax + 44]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 24]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// Call `f` with all seven registers from `r` and 8 stack argument dword(s) (the callee pops 32 bytes), then capture all seven registers into `r`.
unsafe fn call_full_8_32(f: usize, r: &mut Regs, args: *const u32) {
    let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0 };
    for i in 0..8 {
        b.args[i] = unsafe { *args.add(i) };
    }
    unsafe {
        std::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "mov [eax + 64], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 60]",
            "push dword ptr [eax + 56]",
            "push dword ptr [eax + 52]",
            "push dword ptr [eax + 48]",
            "push dword ptr [eax + 44]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]",
            "mov edx, [eax + 12]",
            "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]",
            "mov edi, [eax + 24]",
            "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 32]",
            "push ebx",
            "mov ebx, [esp + 8]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "mov [ebx + 20], esi",
            "mov [ebx + 24], edi",
            "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 64]",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

unsafe fn call_orig(nargs: usize, pop: u32, f: usize, r: &mut Regs, args: *const u32) {
    match (nargs, pop) {
        (0, 0) => unsafe { call_full_0_0(f, r, args) },
        (1, 0) => unsafe { call_full_1_0(f, r, args) },
        (1, 4) => unsafe { call_full_1_4(f, r, args) },
        (2, 0) => unsafe { call_full_2_0(f, r, args) },
        (2, 8) => unsafe { call_full_2_8(f, r, args) },
        (3, 12) => unsafe { call_full_3_12(f, r, args) },
        (4, 16) => unsafe { call_full_4_16(f, r, args) },
        (5, 20) => unsafe { call_full_5_20(f, r, args) },
        (6, 24) => unsafe { call_full_6_24(f, r, args) },
        (8, 32) => unsafe { call_full_8_32(f, r, args) },
        _ => unreachable!(),
    }
}

// --- the port table -------------------------------------------------------------------------------

/// A Rust body: sets the registers the original leaves changed (reads the stack arguments from
/// `args`), and does the original's memory writes.
type Run = fn(&mut Pushad, *const u32);
/// The memory the original may write for this call (address, length); (0, 0) = unused.
type Writes = fn(&Pushad, *const u32) -> [(usize, usize); 4];

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    /// Stack argument dwords handed to the shadow copy, and the bytes the original pops.
    nargs: usize,
    pop: u32,
    run: Run,
    writes: Option<Writes>,
    thunk: extern "C" fn(),
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

#[allow(clippy::too_many_arguments)]
const fn port(
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    nargs: usize,
    pop: u32,
    run: Run,
    writes: Option<Writes>,
    thunk: extern "C" fn(),
) -> Port {
    Port {
        name,
        addr,
        body,
        nargs,
        pop,
        run,
        writes,
        thunk,
        calls: AtomicU32::new(0),
        mismatches: AtomicU32::new(0),
        original: AtomicUsize::new(0),
    }
}

/// A naked thunk: `pushad`, then `entry(id, &mut block, &stack args)`, then `popad` and the
/// original's `ret` / `ret N`.
macro_rules! thunk {
    ($name:ident, $id:expr, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "push {id}",
                "call {f}",
                "add esp, 12",
                "popad",
                $ret,
                id = const $id,
                f = sym entry,
            )
        }
    };
}

/// Count a call; run the Rust body, shadowed against the original while the budget lasts.
extern "C" fn entry(id: usize, r: &mut Pushad, args: *const u32) {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_leaves: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    if orig == 0 || n >= SHADOW_CALLS {
        (p.run)(r, args);
        return;
    }
    shadow(p, n, orig, r, args);
}

fn snapshot(regions: &[(usize, usize); 4]) -> Vec<Vec<u8>> {
    regions
        .iter()
        .map(|&(a, l)| if l == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(a as *const u8, l).to_vec() } })
        .collect()
}

fn restore(regions: &[(usize, usize); 4], saved: &[Vec<u8>]) {
    for (&(a, l), bytes) in regions.iter().zip(saved) {
        if l != 0 {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), a as *mut u8, l) };
        }
    }
}

/// Run the original copy, then the Rust body, from the same state; compare registers and the
/// bytes written. On a mismatch keep the original's registers and writes.
fn shadow(p: &Port, n: u32, orig: usize, r: &mut Pushad, args: *const u32) {
    let inp = Regs::of(r);
    let regions = match p.writes {
        Some(w) => w(r, args),
        None => [(0, 0); 4],
    };
    let pre = snapshot(&regions);
    let mut theirs = inp;
    unsafe { call_orig(p.nargs, p.pop, orig, &mut theirs, args) };
    let post_theirs = snapshot(&regions);
    restore(&regions, &pre);
    let mut mine = *r;
    (p.run)(&mut mine, args);
    let post_mine = snapshot(&regions);
    let ours = Regs::of(&mine);
    if ours != theirs || post_mine != post_theirs {
        let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
        if m <= 10 {
            log(&format!(
                "port_leaves: {} mismatch (call {n}): in {inp:x?}: original {theirs:x?}, Rust {ours:x?}, writes equal {}",
                p.name,
                post_mine == post_theirs
            ));
        }
        restore(&regions, &post_theirs);
        theirs.apply(r);
    } else {
        *r = mine;
    }
}

// --- Rust bodies: generic shapes ----------------------------------------------------------------

fn ea(base: u32, off: u32) -> usize {
    base.wrapping_add(off) as usize
}

unsafe fn ww(p: usize, v: u16) {
    unsafe { *(p as *mut u16) = v }
}

/// `ret` / `ret N`: nothing changes.
fn nop(_: &mut Pushad, _: *const u32) {}

/// `xor eax, eax`.
fn zero_eax(p: &mut Pushad, _: *const u32) {
    p.eax = 0;
}

/// `xor ax, ax`.
fn ax_zero(p: &mut Pushad, _: *const u32) {
    p.eax &= !0xFFFF;
}

/// `mov al, V` / `xor al, al`: the rest of EAX is the caller's.
fn al_const<const V: u32>(p: &mut Pushad, _: *const u32) {
    p.eax = (p.eax & !0xFF) | V;
}

/// `mov eax, V`.
fn eax_const<const V: u32>(p: &mut Pushad, _: *const u32) {
    p.eax = V;
}

/// `mov eax, [ecx + D]`.
fn load_ecx<const D: u32>(p: &mut Pushad, _: *const u32) {
    p.eax = unsafe { rd(ea(p.ecx, D)) };
}

/// `mov al, [ecx + D]`.
fn load_ecx_b<const D: u32>(p: &mut Pushad, _: *const u32) {
    p.eax = (p.eax & !0xFF) | unsafe { rb(ea(p.ecx, D)) } as u32;
}

/// `xor eax, eax; cmp [ecx + D], eax; setne al`.
fn is_set<const D: u32>(p: &mut Pushad, _: *const u32) {
    p.eax = (unsafe { rd(ea(p.ecx, D)) } != 0) as u32;
}

/// `mov eax, [A]; mov eax, [eax]`.
fn global_deref<const A: usize>(p: &mut Pushad, _: *const u32) {
    p.eax = unsafe { rd(rd(A) as usize) };
}

/// A Lua binding that pushes the boolean V: the Lua state L is the stack argument, its top
/// (L + 8) gets tag 1 and the value at +8 and moves up by 0x10; returns 1 value; ECX = the old top.
fn lua_bool<const V: u32>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let l = *a as usize;
        let top = rd(l + 8);
        wd(top as usize, 1);
        wd(top as usize + 8, V);
        wd(l + 8, rd(l + 8).wrapping_add(0x10));
        p.ecx = top;
        p.eax = 1;
    }
}

fn lua_bool_w(_: &Pushad, a: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let l = *a as usize;
        [(rd(l + 8) as usize, 12), (l + 8, 4), (0, 0), (0, 0)]
    }
}

/// `mov byte [A], V; xor eax, eax`.
fn set_byte_z<const A: usize, const V: u8>(p: &mut Pushad, _: *const u32) {
    unsafe { wb(A, V) };
    p.eax = 0;
}

fn set_byte_w<const A: usize>(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(A, 1), (0, 0), (0, 0), (0, 0)]
}

/// `mov dword [A], V; xor eax, eax`.
fn set_dword_z<const A: usize, const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe { wd(A, V) };
    p.eax = 0;
}

/// `or dword [A], M`.
fn or_dword<const A: usize, const M: u32>(_: &mut Pushad, _: *const u32) {
    unsafe { wd(A, rd(A) | M) };
}

/// `and dword [A], M`.
fn and_dword<const A: usize, const M: u32>(_: &mut Pushad, _: *const u32) {
    unsafe { wd(A, rd(A) & M) };
}

fn set_dword_w<const A: usize>(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(A, 4), (0, 0), (0, 0), (0, 0)]
}

/// `mov byte [ecx + O], V`.
fn set_byte_ecx<const O: u32, const V: u8>(p: &mut Pushad, _: *const u32) {
    unsafe { wb(ea(p.ecx, O), V) };
}

fn ecx_byte_w<const O: u32>(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(ea(p.ecx, O), 1), (0, 0), (0, 0), (0, 0)]
}

/// `mov dword [ecx + O], V`.
fn set_dword_ecx<const O: u32, const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe { wd(ea(p.ecx, O), V) };
}

fn ecx_dword_w<const O: u32>(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(ea(p.ecx, O), 4), (0, 0), (0, 0), (0, 0)]
}

/// `vint_object_factory_imp<>::index_of(this = ECX, ptr)`: (ptr - base [+0x14]) / element size by
/// the compiler's multiply-high (M, shift S, and the extra add of the difference when B); -1 when
/// ptr is below the base or the index is not below the count (+0x10). EDX = the shifted high
/// word.
fn index_of<const M: u32, const S: u32, const B: bool>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let x = *a;
        let base = rd(ea(p.ecx, 0x14));
        let diff = x.wrapping_sub(base) as i32;
        let mut hi = ((M as i32 as i64 * diff as i64) >> 32) as i32;
        if B {
            hi = hi.wrapping_add(diff);
        }
        let q = (hi >> S) as u32;
        p.edx = q;
        let mut eax = q.wrapping_add(q >> 31);
        if x < base || eax >= rd(ea(p.ecx, 0x10)) {
            eax = 0xFFFF_FFFF;
        }
        p.eax = eax;
    }
}

// --- Rust bodies: individual functions -------------------------------------------------------------

/// `v % d` with the original's unsigned `div` (a zero divisor faults the same way).
fn udiv_rem(v: u32, d: u32) -> u32 {
    let rem: u32;
    unsafe {
        std::arch::asm!("div {d}", d = in(reg) d, inout("eax") v => _, inout("edx") 0u32 => rem, options(nomem, nostack));
    }
    rem
}

/// 0xBDCF50 rand_int_range (ESI = min, ECX = max -> EAX in [min, max]): the next value of the
/// shared random table (index 0xEC3ABC, wraps at 0x800) modulo max - min + 1, plus min.
/// ECX ends as the modulus and EDX as the remainder.
fn rand_int_range(p: &mut Pushad, _: *const u32) {
    unsafe {
        let k = rd(RAND_INDEX);
        let v = rd(RAND_TABLE.wrapping_add((k as usize).wrapping_mul(4)));
        let k1 = k.wrapping_add(1);
        wd(RAND_INDEX, if (k1 as i32) < 0x800 { k1 } else { 0 });
        let d = p.ecx.wrapping_sub(p.esi).wrapping_add(1);
        let rem = udiv_rem(v, d);
        p.edx = rem;
        p.ecx = d;
        p.eax = rem.wrapping_add(p.esi);
    }
}

fn rand_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(RAND_INDEX, 4), (0, 0), (0, 0), (0, 0)]
}

const WEAPON_INFOS: usize = 0x022D_7BCC;
const WEAPON_INFO_COUNT: usize = 0x022D_7BD0;

/// 0xB71760 weapon_info_find_by_hash (ESI = hash -> EAX entry or 0): the first of the 0x498-byte
/// weapon info entries whose +0x40 equals the hash. EDX = the count, ECX = the index reached.
fn weapon_info_find_by_hash(p: &mut Pushad, _: *const u32) {
    unsafe {
        let count = rd(WEAPON_INFO_COUNT) as i32;
        let base = rd(WEAPON_INFOS);
        p.edx = count as u32;
        p.ecx = 0;
        if count > 0 {
            let mut ptr = base;
            let mut i = 0u32;
            loop {
                if rd(ptr as usize + 0x40) == p.esi {
                    p.eax = ptr;
                    p.ecx = i;
                    return;
                }
                i += 1;
                ptr = ptr.wrapping_add(0x498);
                if (i as i32) >= count {
                    break;
                }
            }
            p.ecx = i;
        }
        p.eax = 0;
    }
}

const VINT_DATAGROUPS: usize = 0x0252_A204;

/// 0xB93D30 vint_datagroup_find_by_hash (EDX = hash -> EAX): walks the circular list headed at
/// 0x252A204 for a node with +8 == hash and +0x1C == 0; 0 for a zero hash or no match.
/// ECX = the head once the hash is non-zero.
fn vint_datagroup_find_by_hash(p: &mut Pushad, _: *const u32) {
    unsafe {
        if p.edx == 0 {
            p.eax = 0;
            return;
        }
        let head = rd(VINT_DATAGROUPS);
        p.ecx = head;
        let mut n = head;
        if n == 0 {
            p.eax = 0;
            return;
        }
        loop {
            if rd(n as usize + 8) == p.edx && rb(n as usize + 0x1C) == 0 {
                p.eax = n;
                return;
            }
            n = rd(n as usize);
            if n == head || n == 0 {
                p.eax = 0;
                return;
            }
        }
    }
}

const AMMO_COUNT: usize = 0x022D_7B50;

/// 0xB693E0 ammo_store_clear (EDI = &entries): zeroes dwords +4 and +8 of each of the 12-byte
/// entries (count at 0x22D7B50). EAX ends at the last entry, ECX = count, EDX = count * 12.
fn ammo_store_clear(p: &mut Pushad, _: *const u32) {
    unsafe {
        let count = rd(AMMO_COUNT) as i32;
        p.ecx = 0;
        if count > 0 {
            let base = rd(p.edi as usize);
            for i in 0..count as u32 {
                let e = base.wrapping_add(i.wrapping_mul(12)) as usize;
                wd(e + 4, 0);
                wd(e + 8, 0);
            }
            p.ecx = count as u32;
            p.edx = (count as u32).wrapping_mul(12);
            p.eax = base.wrapping_add((count as u32 - 1).wrapping_mul(12));
        }
    }
}

fn ammo_clear_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let count = rd(AMMO_COUNT) as i32;
        if count > 0 && count <= 0x1_0000 {
            [(rd(p.edi as usize) as usize, count as usize * 12), (0, 0), (0, 0), (0, 0)]
        } else {
            [(0, 0); 4]
        }
    }
}

/// 0xA02A50 nav_tri_edge_verts (stack: edge index; EDI = triangle vertex pointers, EDX / ECX =
/// out vertices): copies vertex[idx] to [EDX] and vertex[(idx + 1) % 3] to [ECX] (three dwords
/// each, in the original's order). EAX / EDX end as the last two dwords copied to [ECX].
fn nav_tri_edge_verts(p: &mut Pushad, a: *const u32) {
    unsafe {
        let idx = *a;
        let v1 = rd(ea(p.edi, idx.wrapping_mul(4))) as usize;
        let o1 = p.edx as usize;
        wd(o1, rd(v1));
        wd(o1 + 4, rd(v1 + 4));
        wd(o1 + 8, rd(v1 + 8));
        let rem = (idx as i32).wrapping_add(1).wrapping_rem(3) as u32;
        let v2 = rd(ea(p.edi, rem.wrapping_mul(4))) as usize;
        let o2 = p.ecx as usize;
        let w0 = rd(v2);
        wd(o2, w0);
        let w1 = rd(v2 + 4);
        wd(o2 + 4, w1);
        let w2 = rd(v2 + 8);
        wd(o2 + 8, w2);
        p.eax = w1;
        p.edx = w2;
    }
}

fn nav_edge_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(p.edx as usize, 12), (p.ecx as usize, 12), (0, 0), (0, 0)]
}

/// 0xCDFCB0 script_thread_push_bool (ECX = script thread, stack: bool, `ret 4`): pushes a boolean
/// (tag 1, value at +8) on the Lua stack of thread->state (+4) and counts it in thread +0x1C.
/// EAX = the state, EDX = the value.
fn push_bool(p: &mut Pushad, a: *const u32) {
    unsafe {
        let arg = *a;
        let this = p.ecx as usize;
        let st = rd(this + 4) as usize;
        let top = rd(st + 8) as usize;
        let b = ((arg as u8) != 0) as u32;
        wd(top, 1);
        wd(top + 8, b);
        wd(st + 8, (top as u32).wrapping_add(0x10));
        wd(this + 0x1C, rd(this + 0x1C).wrapping_add(1));
        p.eax = st as u32;
        p.edx = b;
    }
}

fn push_bool_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let this = p.ecx as usize;
        let st = rd(this + 4) as usize;
        let top = rd(st + 8) as usize;
        [(top, 12), (st + 8, 4), (this + 0x1C, 4), (0, 0)]
    }
}

const UI_MODE: usize = 0x00E8_CF7C;
const UI_FLAG: usize = 0x0252_8ADF;

/// 0x779AB0 pause_menu_open (stack: value): clears 0x2528356, sets 0x252836C = 2 and
/// 0x29C8E9C = value; unless the flag byte 0x2528ADF is 1, moves the UI mode (0xE8CF7C) to 0xD
/// when it is neither 0xD nor 0xB. EAX = the value, or the mode that was read.
fn pause_menu_open(p: &mut Pushad, a: *const u32) {
    unsafe {
        let v = *a;
        let flag = rb(UI_FLAG) == 1;
        wb(0x0252_8356, 0);
        wd(0x0252_836C, 2);
        wd(0x029C_8E9C, v);
        p.eax = v;
        if flag {
            return;
        }
        let s = rd(UI_MODE);
        p.eax = s;
        if s == 0xD || s == 0xB {
            return;
        }
        wd(UI_MODE, 0xD);
    }
}

fn pause_open_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(0x0252_8356, 1), (0x0252_836C, 4), (0x029C_8E9C, 4), (UI_MODE, 4)]
}

/// 0x7DE2F0 gang_customization_setup_cameras (EAX = value): stores it at 0x2528A18; unless the
/// flag byte is 1, moves the UI mode to 3 when it is neither 0xD nor 0xB. EAX = the mode read.
fn gang_cams(p: &mut Pushad, _: *const u32) {
    unsafe {
        let flag = rb(UI_FLAG) == 1;
        wd(0x0252_8A18, p.eax);
        if flag {
            return;
        }
        let s = rd(UI_MODE);
        p.eax = s;
        if s == 0xD || s == 0xB {
            return;
        }
        wd(UI_MODE, 3);
    }
}

fn gang_cams_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(0x0252_8A18, 4), (UI_MODE, 4), (0, 0), (0, 0)]
}

const OVERLAY_ARR: usize = 0x00EA_C4E0;
const OVERLAY_COUNT: usize = 0x00EA_C4E8;

/// 0x75B4E0 overlay_stack_remove (EDI = &overlay): removes every entry equal to *EDI from the
/// overlay pointer array (0xEAC4E0, count 0xEAC4E8) by shifting the rest down, rescanning from the
/// same index. A line-by-line transliteration so that EAX / ECX / EDX end as the original's.
fn overlay_remove(p: &mut Pushad, _: *const u32) {
    unsafe {
        let mut ecx = rd(OVERLAY_COUNT) as i32;
        let mut eax = p.eax;
        let mut edx = p.edx;
        let mut esi: i32 = 0;
        if ecx > 0 {
            edx = rd(OVERLAY_ARR);
            loop {
                eax = rd(ea(edx, (esi as u32).wrapping_mul(4)));
                if eax != rd(p.edi as usize) {
                    esi += 1;
                } else {
                    ecx -= 1;
                    eax = esi as u32;
                    wd(OVERLAY_COUNT, ecx as u32);
                    if esi >= ecx {
                        break;
                    }
                    loop {
                        let t = rd(ea(edx, eax.wrapping_mul(4).wrapping_add(4)));
                        wd(ea(edx, eax.wrapping_mul(4)), t);
                        ecx = rd(OVERLAY_COUNT) as i32;
                        edx = rd(OVERLAY_ARR);
                        eax = eax.wrapping_add(1);
                        if (eax as i32) >= ecx {
                            break;
                        }
                    }
                }
                if esi >= ecx {
                    break;
                }
            }
        }
        p.eax = eax;
        p.ecx = ecx as u32;
        p.edx = edx;
    }
}

fn overlay_remove_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let count = rd(OVERLAY_COUNT) as i32;
        if count > 0 && count <= 0x1_0000 {
            [(rd(OVERLAY_ARR) as usize, count as usize * 4), (OVERLAY_COUNT, 4), (0, 0), (0, 0)]
        } else {
            [(OVERLAY_COUNT, 4), (0, 0), (0, 0), (0, 0)]
        }
    }
}

/// 0xB9EA50 vint_tween_compare (cdecl a, b -> EAX): difference of the +0x78 field of what the two
/// pointers point at. ECX = the same difference, EDX = b.
fn tween_compare(p: &mut Pushad, a: *const u32) {
    unsafe {
        let x = *a as usize;
        let y = *a.add(1);
        let d = rd(rd(x) as usize + 0x78).wrapping_sub(rd(rd(y as usize) as usize + 0x78));
        p.edx = y;
        p.ecx = d;
        p.eax = d;
    }
}

const MULTI_CUSTOM: usize = 0x02A4_D298;
const MULTI_DIRTY: usize = 0x00E8_B218;

/// 0x8427F0 / 0x8427A0 multi_custom_set_sprint_impl / _ammo_impl (AL = value): if it differs from
/// the stored byte CUR, stores it and flags the change (0x2A4D2C0, 0x2A4D2C2); marks 0xE8B218 =
/// 0xFFFF; for value 1 clears the byte RST (and its companions) when it is set. CL = 1; EAX = 0
/// when the value was 1.
fn custom_sprint<const CUR: usize, const RST: usize>(p: &mut Pushad, _: *const u32) {
    unsafe {
        let al = p.eax as u8;
        p.ecx = (p.ecx & !0xFF) | 1;
        if al != rb(CUR) {
            wb(CUR, al);
            wb(0x02A4_D2C0, 1);
            wb(0x02A4_D2C2, 1);
        }
        ww(MULTI_DIRTY, 0xFFFF);
        if al != 1 {
            return;
        }
        p.eax = 0;
        if rb(RST) == 0 {
            return;
        }
        wb(RST, 0);
        wd(0x02A4_D2A0, 0);
        wb(0x02A4_D298, 1);
        wb(0x02A4_D29A, 1);
    }
}

fn custom_set_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(MULTI_CUSTOM, 0x40), (MULTI_DIRTY, 2), (0, 0), (0, 0)]
}

/// 0x842980 multi_custom_set_tag_status_impl (AL = value, ECX = index): stores the value in
/// byte [0x2A4D2A4 + index] when it differs (clearing 0x2A4D2A0 and flagging 0x2A4D298 / 29A),
/// marks 0xE8B218 = 0xFFFF and clears byte 0x2A4D29C (flagging again) when it was set. DL = 1.
fn custom_tag_status(p: &mut Pushad, _: *const u32) {
    unsafe {
        let al = p.eax as u8;
        p.edx = (p.edx & !0xFF) | 1;
        let slot = ea(p.ecx, 0x02A4_D2A4);
        if rb(slot) != al {
            wb(slot, al);
            wd(0x02A4_D2A0, 0);
            wb(0x02A4_D298, 1);
            wb(0x02A4_D29A, 1);
        }
        ww(MULTI_DIRTY, 0xFFFF);
        if rb(0x02A4_D29C) != 0 {
            wb(0x02A4_D29C, 0);
            wb(0x02A4_D298, 1);
            wb(0x02A4_D29A, 1);
        }
    }
}

fn custom_tag_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(MULTI_CUSTOM, 0x40), (ea(p.ecx, 0x02A4_D2A4), 1), (MULTI_DIRTY, 2), (0, 0)]
}

/// 0x842720 multi_custom_set_friendly_fire_impl (EAX = mode 0 / 1 / 2): maps it to ECX 1 / 2 / 0
/// (anything else 0), stores that at 0x2A4D134 when it differs (flagging 0x2A4D12C / 12E) and
/// marks 0xE8B218 = 0xFFFF. EAX ends as mode - 2 for modes above 2, else 0.
fn custom_friendly_fire(p: &mut Pushad, _: *const u32) {
    unsafe {
        let v = p.eax;
        let (ecx, eax) = match v {
            0 => (1, 0),
            1 => (2, 0),
            2 => (0, 0),
            _ => (0, v.wrapping_sub(2)),
        };
        p.ecx = ecx;
        p.eax = eax;
        let differs = rd(0x02A4_D134) != ecx;
        ww(MULTI_DIRTY, 0xFFFF);
        if differs {
            wd(0x02A4_D134, ecx);
            wb(0x02A4_D12C, 1);
            wb(0x02A4_D12E, 1);
        }
    }
}

fn custom_ff_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    [(0x02A4_D12C, 0x10), (MULTI_DIRTY, 2), (0, 0), (0, 0)]
}

// --- Rust bodies: second batch ----------------------------------------------------------------------

/// Up to four (address, length) regions for a `Writes` function.
fn regions(v: &[(usize, usize)]) -> [(usize, usize); 4] {
    let mut r = [(0, 0); 4];
    for (i, &x) in v.iter().enumerate() {
        r[i] = x;
    }
    r
}

/// The bytes `[ecx + O, ecx + O + L)`.
fn ecx_range_w<const O: u32, const L: usize>(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(ea(p.ecx, O), L)])
}

/// `mov eax, [esp + 4]; mov ecx, [G]; cmp eax, [ecx]; sete al` (`ret 4`): is the argument the type
/// id stored behind the pointer at G. AL = the answer, the rest of EAX is the argument's; ECX = the
/// pointer.
fn is_type_global<const A: usize>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let x = *a;
        let t = rd(A);
        p.ecx = t;
        let eq = (x == rd(t as usize)) as u32;
        p.eax = (x & !0xFF) | eq;
    }
}

/// `vehicle_turn_signal_manager::get_light_type`: 3, or 4 when the byte at +0x2c is not 1.
fn turn_signal_light_type(p: &mut Pushad, _: *const u32) {
    p.eax = 3 + (unsafe { rb(ea(p.ecx, 0x2c)) } != 1) as u32;
}

/// `vehicle_turn_signal_manager::get_light_name`: 0xE46078 when the byte at +0x2c is 1, else 0xE4608C.
fn turn_signal_light_name(p: &mut Pushad, _: *const u32) {
    p.eax = if unsafe { rb(ea(p.ecx, 0x2c)) } == 1 { 0x00E4_6078 } else { 0x00E4_608C };
}

/// `skirmish_spawn_group::is_active`: ECX = the dword at +0x1f0 minus 2; AL = that is at most 2
/// (unsigned), i.e. the dword is 2, 3 or 4. The rest of EAX is the caller's.
fn skirmish_is_active(p: &mut Pushad, _: *const u32) {
    let v = unsafe { rd(ea(p.ecx, 0x1f0)) }.wrapping_sub(2);
    p.ecx = v;
    p.eax = (p.eax & !0xFF) | (v <= 2) as u32;
}

/// `tow_truck_diversion_type_info::get_start_mode`: 2 unless the dword at 0xE96C00 is 3 (then 0).
fn tow_truck_start_mode(p: &mut Pushad, _: *const u32) {
    p.eax = if unsafe { rd(0x00E9_6C00) } != 3 { 2 } else { 0 };
}

/// `mov al, [esp + 4]; mov [ecx + O], al` (`ret 4`).
fn set_byte_arg_ecx<const O: u32>(p: &mut Pushad, a: *const u32) {
    let v = unsafe { *a } as u8;
    p.eax = (p.eax & !0xFF) | v as u32;
    unsafe { wb(ea(p.ecx, O), v) };
}

/// `mov eax, [ecx + 0xc]; mov [ecx + 0xa4], eax`.
fn script_trigger_spawned(p: &mut Pushad, _: *const u32) {
    unsafe {
        let v = rd(ea(p.ecx, 0xc));
        p.eax = v;
        wd(ea(p.ecx, 0xa4), v);
    }
}

/// `mov eax, ecx; xor ecx, ecx; mov [eax + 0x28], ecx; mov [eax + 0x2c], ecx; mov [eax], V`.
fn vint_ctor<const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe {
        let t = p.ecx as usize;
        p.eax = p.ecx;
        p.ecx = 0;
        wd(t + 0x28, 0);
        wd(t + 0x2c, 0);
        wd(t, V);
    }
}

fn vint_ctor_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(p.ecx as usize, 4), (ea(p.ecx, 0x28), 8)])
}

/// The bitmap-style constructors: as `vint_ctor`, then `cmp ecx, ecx` (always equal, so the `jne`
/// is never taken) leaves [+0x98] = 9 and [+0x9c] = 0. V2 is the final vtable (the gradient one
/// stores V first, then V2).
fn vint_ctor_bitmap<const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe {
        let t = p.ecx as usize;
        p.eax = p.ecx;
        p.ecx = 0;
        wd(t + 0x28, 0);
        wd(t + 0x2c, 0);
        wd(t, V);
        wd(t + 0x98, 9);
        wd(t + 0x9c, 0);
    }
}

fn vint_ctor_gradient(p: &mut Pushad, _: *const u32) {
    unsafe {
        let t = p.ecx as usize;
        p.eax = p.ecx;
        p.ecx = 0;
        wd(t + 0x28, 0);
        wd(t + 0x2c, 0);
        wd(t, 0x00E4_D85C);
        wd(t + 0x98, 9);
        wd(t + 0x9c, 0);
        wd(t, 0x00E4_D9CC);
    }
}

fn vint_ctor_bitmap_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(p.ecx as usize, 4), (ea(p.ecx, 0x28), 8), (ea(p.ecx, 0x98), 8)])
}

/// `vint_value_ctor`: `mov eax, ecx; mov dword [eax], 0`.
fn vint_value_ctor(p: &mut Pushad, _: *const u32) {
    p.eax = p.ecx;
    unsafe { wd(p.ecx as usize, 0) };
}

/// `mugging_diversion_type::init`: zeroes bytes +0x18..+0x1a and the dword at +0x1c; EAX = 0.
fn mugging_init(p: &mut Pushad, _: *const u32) {
    unsafe {
        p.eax = 0;
        wb(ea(p.ecx, 0x18), 0);
        wb(ea(p.ecx, 0x19), 0);
        wb(ea(p.ecx, 0x1a), 0);
        wd(ea(p.ecx, 0x1c), 0);
    }
}

/// `luafn_gang_customization_revert_gang_sign`: copies the dword at 0x2528A68 to 0x1FCA504; EAX = 0.
fn gang_sign_revert(p: &mut Pushad, _: *const u32) {
    unsafe {
        wd(0x01FC_A504, rd(0x0252_8A68));
        p.eax = 0;
    }
}

fn gang_sign_revert_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x01FC_A504, 4)])
}

/// `detour_multi_registration::write_state_delta` (`ret 0x18`): EAX = the third argument, whose
/// dword at +0xc is cleared.
fn write_state_delta(p: &mut Pushad, a: *const u32) {
    unsafe {
        let e = *a.add(2);
        p.eax = e;
        wd(ea(e, 0xc), 0);
    }
}

fn write_state_delta_w(_: &Pushad, a: *const u32) -> [(usize, usize); 4] {
    regions(&[(unsafe { ea(*a.add(2), 0xc) }, 4)])
}

/// `detour_multi_registration::capture_snapshot` (`ret 8`): EAX = the second argument, whose dwords
/// at +0xc and +0x10 are cleared; ECX = 0.
fn capture_snapshot(p: &mut Pushad, a: *const u32) {
    unsafe {
        let e = *a.add(1);
        p.eax = e;
        p.ecx = 0;
        wd(ea(e, 0x10), 0);
        wd(ea(e, 0xc), 0);
    }
}

fn capture_snapshot_w(_: &Pushad, a: *const u32) -> [(usize, usize); 4] {
    regions(&[(unsafe { ea(*a.add(1), 0xc) }, 8)])
}

/// `luafn_reset_mp_tutorial_count`: zeroes the dwords 0x2529DCC / 0x2529DD0 and the byte 0x2529DD4; EAX = 0.
fn reset_mp_tutorial_count(p: &mut Pushad, _: *const u32) {
    unsafe {
        p.eax = 0;
        wd(0x0252_9DCC, 0);
        wd(0x0252_9DD0, 0);
        wb(0x0252_9DD4, 0);
    }
}

fn reset_mp_tutorial_count_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x0252_9DCC, 8), (0x0252_9DD4, 1)])
}

/// `cheat_weather_off`: clears bit 3 of 0x27A6A40 and zeroes the dword 0x2526E14.
fn cheat_weather_off(_: &mut Pushad, _: *const u32) {
    unsafe {
        wd(0x027A_6A40, rd(0x027A_6A40) & 0xFFFF_FFF7);
        wd(0x0252_6E14, 0);
    }
}

fn cheat_weather_off_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x027A_6A40, 4), (0x0252_6E14, 4)])
}

/// `luafn_pause_menu_objective_force_show`: zeroes the dword 0x29DACF0, sets the byte 0x25283C1; EAX = 0.
fn objective_force_show(p: &mut Pushad, _: *const u32) {
    unsafe {
        wd(0x029D_ACF0, 0);
        wb(0x0252_83C1, 1);
        p.eax = 0;
    }
}

fn objective_force_show_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x029D_ACF0, 4), (0x0252_83C1, 1)])
}

/// `luafn_sprint_reset_local_overrides`: stores -1 in the three dwords from 0xE96AA8; EAX = 0.
fn sprint_reset_overrides(p: &mut Pushad, _: *const u32) {
    unsafe {
        wd(0x00E9_6AA8, 0xFFFF_FFFF);
        wd(0x00E9_6AAC, 0xFFFF_FFFF);
        wd(0x00E9_6AB0, 0xFFFF_FFFF);
        p.eax = 0;
    }
}

fn sprint_reset_overrides_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x00E9_6AA8, 12)])
}

/// Nine dwords from A set to V; EAX = 0 (`luafn_character_slots_clear_reservations` / `_caps`).
fn slots_fill<const A: usize, const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe {
        for i in 0..9 {
            wd(A + 4 * i, V);
        }
        p.eax = 0;
    }
}

fn slots_fill_w<const A: usize>(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(A, 36)])
}

/// `luafn_player_creation_set_opening`: EAX = 0; sets the byte 0x2528989 when the byte 0xE8D6BF is set.
fn player_creation_set_opening(p: &mut Pushad, _: *const u32) {
    unsafe {
        p.eax = 0;
        if rb(0x00E8_D6BF) != 0 {
            wb(0x0252_8989, 1);
        }
    }
}

fn player_creation_set_opening_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x0252_8989, 1)])
}

const HOMIE_COUNT: usize = 0x0252_8400;
const HOMIE_FIRST: usize = 0x029D_C318;

/// `luafn_homie_mission_lock_all` / `_reset`: stores V in the first dword of each of the
/// 0x3AC-byte entries from 0x29DC318 (count at 0x2528400); EAX = 0, ECX = 0 when the count was
/// positive, else the count.
fn homie_set<const V: u32>(p: &mut Pushad, _: *const u32) {
    unsafe {
        let count = rd(HOMIE_COUNT) as i32;
        if count > 0 {
            for i in 0..count as usize {
                wd(HOMIE_FIRST + i * 0x3AC, V);
            }
            p.ecx = 0;
        } else {
            p.ecx = count as u32;
        }
        p.eax = 0;
    }
}

fn homie_set_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    let count = unsafe { rd(HOMIE_COUNT) } as i32;
    if count > 0 && count <= 64 {
        regions(&[(HOMIE_FIRST, (count as usize - 1) * 0x3AC + 4)])
    } else {
        [(0, 0); 4]
    }
}

const ENUM_COUNT: usize = 0x0252_A274;
const ENUM_TABLE: usize = 0x022F_2170;

/// 0xB9D7E0 vint_enum_register (EDX = value, stack: two values): while fewer than 0x20 entries
/// are registered, stores EDX, arg0 and arg1 in the 16-byte entry at [+0], [+4] and [+0xc] of the
/// table at 0x22F2170 and counts it (0x252A274). ECX = the old count (+1 when stored); when
/// stored EAX = count * 16 and EDX = arg1.
fn vint_enum_register(p: &mut Pushad, a: *const u32) {
    unsafe {
        let n = rd(ENUM_COUNT);
        p.ecx = n;
        if n >= 0x20 {
            return;
        }
        let e = ENUM_TABLE + (n as usize) * 16;
        p.eax = n << 4;
        wd(e, p.edx);
        wd(e + 4, *a);
        p.edx = *a.add(1);
        p.ecx = n + 1;
        wd(e + 0xc, p.edx);
        wd(ENUM_COUNT, n + 1);
    }
}

fn vint_enum_register_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    let n = unsafe { rd(ENUM_COUNT) };
    if n < 0x20 {
        regions(&[(ENUM_TABLE + n as usize * 16, 16), (ENUM_COUNT, 4)])
    } else {
        [(0, 0); 4]
    }
}

/// 0xB990F0 vint_writer_write_16_bytes (EAX = writer, EDX = source): copies the 16 bytes at EDX to
/// [writer] + [writer + 0xc] and advances [writer + 0xc] by 0x10. ECX = the destination, EDX = the
/// source's last dword.
fn vint_writer_write_16(p: &mut Pushad, _: *const u32) {
    unsafe {
        let w = p.eax as usize;
        let dst = rd(w).wrapping_add(rd(w + 0xc)) as usize;
        let src = p.edx as usize;
        p.ecx = dst as u32;
        wd(dst, rd(src));
        wd(dst + 4, rd(src + 4));
        wd(dst + 8, rd(src + 8));
        let last = rd(src + 0xc);
        p.edx = last;
        wd(dst + 0xc, last);
        wd(w + 0xc, rd(w + 0xc).wrapping_add(0x10));
    }
}

fn vint_writer_write_16_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let w = p.eax as usize;
        regions(&[(rd(w).wrapping_add(rd(w + 0xc)) as usize, 16), (w + 0xc, 4)])
    }
}

/// 0xB8CF10 vint_value_set_color3 (EAX = value, EDX = source vector): when the value's type (+0)
/// is 0 or 6, sets it to 6 and copies the three dwords at EDX to +4..+0xc. ECX = the type read
/// (when left alone), else the source's second dword; EDX = its third (when copied).
fn vint_value_set_color3(p: &mut Pushad, _: *const u32) {
    unsafe {
        let v = p.eax as usize;
        let s = p.edx as usize;
        let ty = rd(v);
        p.ecx = ty;
        if ty != 0 && ty != 6 {
            return;
        }
        wd(v, 6);
        wd(v + 4, rd(s));
        let c1 = rd(s + 4);
        p.ecx = c1;
        wd(v + 8, c1);
        let c2 = rd(s + 8);
        p.edx = c2;
        wd(v + 0xc, c2);
    }
}

fn vint_value_set_color3_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(p.eax as usize, 16)])
}

/// The record for a handle in the 0x31BACF0 table of 0x20 records (`vint_datagroup_find_by_handle`,
/// ECX = handle): the record when the handle's index (low 16 bits) is below 0x20, its +0x14 is the
/// handle and its byte +0x1c is 0; else 0.
fn vint_datagroup_find_by_handle(p: &mut Pushad, _: *const u32) {
    unsafe {
        let h = p.ecx;
        if h == 0 {
            p.eax = 0;
            return;
        }
        let i = h & 0xFFFF;
        p.eax = i;
        if i >= 0x20 {
            p.eax = 0;
            return;
        }
        let rec = (i << 5).wrapping_add(0x031B_ACF0);
        p.eax = rec;
        if rd(rec as usize + 0x14) != h || rb(rec as usize + 0x1c) != 0 {
            p.eax = 0;
        }
    }
}

const VINT_DATAITEMS: usize = 0x0252_A1F4;

/// 0xB951B0 vint_dataitem_find_by_hash (EDX = hash -> EAX): walks the circular list headed at
/// 0x252A1F4 (next at +0xc) for a node with +0x10 == hash; 0 for a zero hash or no match. ECX = the
/// head once the hash is non-zero.
fn vint_dataitem_find_by_hash(p: &mut Pushad, _: *const u32) {
    unsafe {
        if p.edx == 0 {
            p.eax = 0;
            return;
        }
        let head = rd(VINT_DATAITEMS);
        p.ecx = head;
        let mut n = head;
        if n == 0 {
            p.eax = 0;
            return;
        }
        loop {
            if rd(n as usize + 0x10) == p.edx {
                p.eax = n;
                return;
            }
            n = rd(n as usize + 0xc);
            if n == head || n == 0 {
                p.eax = 0;
                return;
            }
        }
    }
}

const VINT_CALLBACK_BUCKETS: usize = 0x022F_2670;

/// The shared tail of the callback lookups: EDX = handle (non-zero); the bucket at
/// 0x22F2670 + (handle & 0x3f) * 4 heads a circular list (next at +8) of callbacks whose +0x18 is
/// the handle. ECX = the bucket head; EAX = the match or 0.
fn callback_bucket_find(p: &mut Pushad) {
    unsafe {
        let h = p.edx;
        let head = rd(VINT_CALLBACK_BUCKETS + ((h & 0x3F) as usize) * 4);
        p.ecx = head;
        let mut n = head;
        if n == 0 {
            p.eax = 0;
            return;
        }
        loop {
            if rd(n as usize + 0x18) == h {
                p.eax = n;
                return;
            }
            n = rd(n as usize + 8);
            if n == head || n == 0 {
                p.eax = 0;
                return;
            }
        }
    }
}

/// 0xBA0980 vint_callback_find_by_handle (EDX = handle -> EAX).
fn vint_callback_find_by_handle(p: &mut Pushad, _: *const u32) {
    if p.edx == 0 {
        p.eax = 0;
        return;
    }
    callback_bucket_find(p);
}

/// 0xB93750 vint_callback_from_ref (EAX = a value reference): when its type (+0) is 8, the
/// callback for the handle at +4 (EDX = the handle); else 0.
fn vint_callback_from_ref(p: &mut Pushad, _: *const u32) {
    unsafe {
        let r = p.eax as usize;
        if rd(r) != 8 {
            p.eax = 0;
            return;
        }
        p.edx = rd(r + 4);
        if p.edx == 0 {
            p.eax = 0;
            return;
        }
        callback_bucket_find(p);
    }
}

const PLAYER_LIST: usize = 0x0213_6B78;

/// 0x7E2BD0 mp_players_kick_player (ECX = n -> EAX): the n-th node after the head of the circular
/// list at 0x2136B78 (next at +0x24); 0 when the list is empty or comes round first. EDX = the
/// head, ECX = what is left of n.
fn nth_player(p: &mut Pushad, _: *const u32) {
    unsafe {
        let head = rd(PLAYER_LIST);
        p.edx = head;
        p.eax = head;
        if head == 0 {
            p.eax = 0;
            return;
        }
        loop {
            if p.ecx == 0 {
                return;
            }
            p.eax = rd(p.eax as usize + 0x24);
            p.ecx = p.ecx.wrapping_sub(1);
            if p.eax == head || p.eax == 0 {
                p.eax = 0;
                return;
            }
        }
    }
}

/// 0xB8CE20 vint_args_consume (EAX = counter, stack: flag byte): decrements the counter when the
/// flag is set or the counter is above 0. AL = the byte at 0xDC59F5.
fn vint_args_consume(p: &mut Pushad, a: *const u32) {
    unsafe {
        let c = p.eax as usize;
        if (*a as u8) != 0 || (rd(c) as i32) > 0 {
            wd(c, rd(c).wrapping_sub(1));
        }
        p.eax = (p.eax & !0xFF) | rb(0x00DC_59F5) as u32;
    }
}

fn vint_args_consume_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(p.eax as usize, 4)])
}

/// 0x685270 cheat_vehicle_drop (stack: pointer to a pair): unless the first value is -1 or the
/// state at 0xEAA8AC is 2, stores the pair at 0xEAA8B0 / 0xEAA8B4 and sets the state to 1.
/// EAX = the first value; ECX = the pointer, or the second value once the first was not -1.
fn cheat_vehicle_drop(p: &mut Pushad, a: *const u32) {
    unsafe {
        let ptr = *a as usize;
        let first = rd(ptr);
        p.eax = first;
        p.ecx = ptr as u32;
        if first == 0xFFFF_FFFF {
            return;
        }
        let second = rd(ptr + 4);
        p.ecx = second;
        if rd(0x00EA_A8AC) == 2 {
            return;
        }
        wd(0x00EA_A8B0, first);
        wd(0x00EA_A8B4, second);
        wd(0x00EA_A8AC, 1);
    }
}

fn cheat_vehicle_drop_w(_: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(0x00EA_A8AC, 12)])
}

/// `vehicle_light_manager::init` (stack: arg0, two more, `ret 0xc`): [+0x10] = arg0, zeroes +8,
/// +0xc and the byte +4, sets +0x14..+0x24 to -1. EAX = -1.
fn light_manager_init(p: &mut Pushad, a: *const u32) {
    unsafe {
        wd(ea(p.ecx, 0x10), *a);
        wd(ea(p.ecx, 8), 0);
        wd(ea(p.ecx, 0xc), 0);
        wb(ea(p.ecx, 4), 0);
        for o in [0x14u32, 0x18, 0x1c, 0x20, 0x24] {
            wd(ea(p.ecx, o), 0xFFFF_FFFF);
        }
        p.eax = 0xFFFF_FFFF;
    }
}

/// `stunt_peel_out_diversion_type::init`: clears the byte +0x18, sets [+0xb0] = -1 and
/// [+0xac] = the dword at 0xE843AC, plus 0x94B62E00 when that is above 0x6B49D200 (signed). EAX =
/// the value stored.
fn peel_out_init(p: &mut Pushad, _: *const u32) {
    unsafe {
        let mut v = rd(0x00E8_43AC);
        let big = (v as i32) > 0x6B49_D200;
        wb(ea(p.ecx, 0x18), 0);
        wd(ea(p.ecx, 0xb0), 0xFFFF_FFFF);
        if big {
            v = v.wrapping_add(0x94B6_2E00);
        }
        wd(ea(p.ecx, 0xac), v);
        p.eax = v;
    }
}

fn peel_out_init_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(ea(p.ecx, 0x18), 1), (ea(p.ecx, 0xac), 8)])
}

/// `driver_bailout_diversion_type_info::on_game_event_15` (`ret 0x14`): when [+0x18] is set and the
/// fourth argument equals [+0x24], adds the third argument to [+0x44]. EAX = the fourth argument,
/// EDX = the third (as far as reached).
fn bailout_event_15(p: &mut Pushad, a: *const u32) {
    unsafe {
        if rd(ea(p.ecx, 0x18)) == 0 {
            return;
        }
        let e = *a.add(3);
        p.eax = e;
        if e != rd(ea(p.ecx, 0x24)) {
            return;
        }
        let d = *a.add(2);
        p.edx = d;
        wd(ea(p.ecx, 0x44), rd(ea(p.ecx, 0x44)).wrapping_add(d));
    }
}

/// `driver_bailout_diversion_type_info::on_vehicle_destroyed` (`ret 0x10`): when [+0x18] is set and
/// the third argument equals [+0x24], adds the dword at [[arg0 + 0x84e4] + 0x2a8] to [+0x44].
/// EAX = the fourth argument, then [arg0 + 0x84e4]; EDX = arg0, then the added dword.
fn bailout_vehicle_destroyed(p: &mut Pushad, a: *const u32) {
    unsafe {
        if rd(ea(p.ecx, 0x18)) == 0 {
            return;
        }
        let e = *a.add(2);
        p.eax = e;
        if e != rd(ea(p.ecx, 0x24)) {
            return;
        }
        let v = *a;
        p.edx = v;
        let o = rd(ea(v, 0x84e4));
        p.eax = o;
        let d = rd(ea(o, 0x2a8));
        p.edx = d;
        wd(ea(p.ecx, 0x44), rd(ea(p.ecx, 0x44)).wrapping_add(d));
    }
}

fn bailout_add_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    regions(&[(ea(p.ecx, 0x44), 4)])
}

const HANDLE_TABLE: u32 = 0x0214_9C64;

/// The shared head of the registration handlers (stack arg0 = handle): ECX = the handle; the entry
/// at 0x2149C64 + (handle & 0xfff) * 16 is EAX; returns the entry when it is non-null and its +0x44
/// is the handle. A zero handle leaves EAX alone.
fn registration_entry(p: &mut Pushad, a: *const u32) -> Option<u32> {
    unsafe {
        let h = *a;
        p.ecx = h;
        if h == 0 {
            return None;
        }
        let e = rd(ea((h & 0xFFF) << 4, HANDLE_TABLE));
        p.eax = e;
        if e == 0 || rd(ea(e, 0x44)) != h {
            return None;
        }
        Some(e)
    }
}

/// `*_multi_registration::on_register`: sets 0x180 in the entry's +0x80.
fn registration_on_register(p: &mut Pushad, a: *const u32) {
    if let Some(e) = registration_entry(p, a) {
        unsafe { wd(ea(e, 0x80), rd(ea(e, 0x80)) | 0x180) };
    }
}

fn registration_on_register_w(p: &Pushad, a: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let h = *a;
        if h == 0 {
            return [(0, 0); 4];
        }
        let e = rd(ea((h & 0xFFF) << 4, HANDLE_TABLE));
        if e == 0 || rd(ea(e, 0x44)) != h {
            return [(0, 0); 4];
        }
        let _ = p;
        regions(&[(ea(e, 0x80), 4)])
    }
}

/// `*_multi_registration::on_ownership_taken` (`ret 0xc`): when the entry's +0x4c is K, EAX =
/// its pointer at +OFF, whose dword +4 and byte +0 are cleared.
fn registration_ownership_taken<const K: u32, const OFF: u32>(p: &mut Pushad, a: *const u32) {
    unsafe {
        if let Some(e) = registration_entry(p, a) {
            if rd(ea(e, 0x4c)) != K {
                return;
            }
            let t = rd(ea(e, OFF));
            p.eax = t;
            wd(ea(t, 4), 0);
            wb(t as usize, 0);
        }
    }
}

fn registration_ownership_taken_w<const K: u32, const OFF: u32>(_: &Pushad, a: *const u32) -> [(usize, usize); 4] {
    unsafe {
        let h = *a;
        if h == 0 {
            return [(0, 0); 4];
        }
        let e = rd(ea((h & 0xFFF) << 4, HANDLE_TABLE));
        if e == 0 || rd(ea(e, 0x44)) != h || rd(ea(e, 0x4c)) != K {
            return [(0, 0); 4];
        }
        regions(&[(rd(ea(e, OFF)) as usize, 8)])
    }
}

/// The three handle-indexed reference counters (ECX = handle, low 16 bits the index): when the
/// handle is non-zero and the index below LIM, EAX = the record at BASE + index * SIZE; when the
/// record's key (at KEY) is the handle, its count (at CNT) goes up by one.
fn handle_addref<const LIM: u32, const SIZE: u32, const BASE: u32, const KEY: u32, const CNT: u32>(p: &mut Pushad, _: *const u32) {
    unsafe {
        let h = p.ecx;
        if h == 0 {
            return;
        }
        let i = h & 0xFFFF;
        p.eax = i;
        if i >= LIM {
            return;
        }
        let rec = BASE.wrapping_add(i.wrapping_mul(SIZE));
        p.eax = rec;
        if rd(ea(rec, KEY)) == h {
            wd(ea(rec, CNT), rd(ea(rec, CNT)).wrapping_add(1));
        }
    }
}

fn handle_addref_w<const LIM: u32, const SIZE: u32, const BASE: u32, const KEY: u32, const CNT: u32>(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    let h = p.ecx;
    let i = h & 0xFFFF;
    if h == 0 || i >= LIM {
        return [(0, 0); 4];
    }
    regions(&[(ea(BASE.wrapping_add(i.wrapping_mul(SIZE)), CNT), 4)])
}

/// 0xB92DA0 vint_font_addref: as `handle_addref`, but the limit is a 16-bit compare of CX against
/// 10 before the index is widened, and EDX = index * 7 once it passes.
fn vint_font_addref(p: &mut Pushad, _: *const u32) {
    unsafe {
        let h = p.ecx;
        if h == 0 || (h as u16) >= 0xA {
            return;
        }
        let i = h & 0xFFFF;
        p.eax = i;
        let k = i.wrapping_mul(7);
        p.edx = k;
        let rec = k.wrapping_mul(4).wrapping_add(0x031B_C400);
        p.eax = rec;
        if rd(ea(rec, 8)) == h {
            wd(ea(rec, 0x10), rd(ea(rec, 0x10)).wrapping_add(1));
        }
    }
}

fn vint_font_addref_w(p: &Pushad, _: *const u32) -> [(usize, usize); 4] {
    let h = p.ecx;
    if h == 0 || (h as u16) >= 0xA {
        return [(0, 0); 4];
    }
    regions(&[(ea((h & 0xFFFF).wrapping_mul(28).wrapping_add(0x031B_C400), 0x10), 4)])
}

// Lua boolean-returning bindings: the Lua state is the stack argument, its top (+8) receives tag 1
// and the value at +8 and moves up by 0x10; the binding returns 1 value. ECX = the old top, EDX =
// the value (the `cl` shapes leave ECX's upper bytes alone and set CL).

/// Push `flag` on the Lua state `l`; returns the old top.
unsafe fn lua_push_flag(l: usize, flag: u32) -> u32 {
    unsafe {
        let top = rd(l + 8);
        wd(top as usize, 1);
        wd(top as usize + 8, flag);
        wd(l + 8, top.wrapping_add(0x10));
        top
    }
}

/// Pushes (byte at A != 0).
fn lua_flag_byte<const A: usize>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let f = (rb(A) != 0) as u32;
        p.ecx = lua_push_flag(*a as usize, f);
        p.edx = f;
        p.eax = 1;
    }
}

/// Pushes (dword at A != 0).
fn lua_flag_dword_ne<const A: usize>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let f = (rd(A) != 0) as u32;
        p.ecx = lua_push_flag(*a as usize, f);
        p.edx = f;
        p.eax = 1;
    }
}

/// Pushes (dword at A == K).
fn lua_flag_dword_eq<const A: usize, const K: u32>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let f = (rd(A) == K) as u32;
        p.ecx = lua_push_flag(*a as usize, f);
        p.edx = f;
        p.eax = 1;
    }
}

/// Pushes (dword at A == K), leaving ECX's upper bytes alone and CL = the flag.
fn lua_flag_dword_eq_cl<const A: usize, const K: u32>(p: &mut Pushad, a: *const u32) {
    unsafe {
        let f = (rd(A) == K) as u32;
        lua_push_flag(*a as usize, f);
        p.ecx = (p.ecx & !0xFF) | f;
        p.edx = f;
        p.eax = 1;
    }
}

/// `luafn_get_is_team_game`: pushes (dword at 0xE8B210 is 0xD or 0xC), shape as above.
fn lua_team_game(p: &mut Pushad, a: *const u32) {
    unsafe {
        let v = rd(0x00E8_B210);
        let f = (v == 0xD || v == 0xC) as u32;
        lua_push_flag(*a as usize, f);
        p.ecx = (p.ecx & !0xFF) | f;
        p.edx = f;
        p.eax = 1;
    }
}

/// `luafn_weapon_store_is_melee_store`: pushes (the dword at +0x13c of the record pointed to by
/// 0x2528834 is 0x27); ECX = the old top.
fn lua_melee_store(p: &mut Pushad, a: *const u32) {
    unsafe {
        let f = (rd(rd(0x0252_8834) as usize + 0x13c) == 0x27) as u32;
        p.ecx = lua_push_flag(*a as usize, f);
        p.edx = f;
        p.eax = 1;
    }
}

// --- thunks -------------------------------------------------------------------------------------
thunk!(t0, 0, "ret");
thunk!(t1, 1, "ret");
thunk!(t2, 2, "ret");
thunk!(t3, 3, "ret");
thunk!(t4, 4, "ret");
thunk!(t5, 5, "ret");
thunk!(t6, 6, "ret");
thunk!(t7, 7, "ret");
thunk!(t8, 8, "ret");
thunk!(t9, 9, "ret 12");
thunk!(t10, 10, "ret 12");
thunk!(t11, 11, "ret 8");
thunk!(t12, 12, "ret");
thunk!(t13, 13, "ret");
thunk!(t14, 14, "ret 8");
thunk!(t15, 15, "ret 12");
thunk!(t16, 16, "ret 12");
thunk!(t17, 17, "ret");
thunk!(t18, 18, "ret 8");
thunk!(t19, 19, "ret 4");
thunk!(t20, 20, "ret");
thunk!(t21, 21, "ret");
thunk!(t22, 22, "ret");
thunk!(t23, 23, "ret 4");
thunk!(t24, 24, "ret");
thunk!(t25, 25, "ret");
thunk!(t26, 26, "ret");
thunk!(t27, 27, "ret");
thunk!(t28, 28, "ret");
thunk!(t29, 29, "ret");
thunk!(t30, 30, "ret");
thunk!(t31, 31, "ret");
thunk!(t32, 32, "ret");
thunk!(t33, 33, "ret 20");
thunk!(t34, 34, "ret 16");
thunk!(t35, 35, "ret 16");
thunk!(t36, 36, "ret 24");
thunk!(t37, 37, "ret");
thunk!(t38, 38, "ret 4");
thunk!(t39, 39, "ret 4");
thunk!(t40, 40, "ret");
thunk!(t41, 41, "ret");
thunk!(t42, 42, "ret");
thunk!(t43, 43, "ret");
thunk!(t44, 44, "ret");
thunk!(t45, 45, "ret 4");
thunk!(t46, 46, "ret");
thunk!(t47, 47, "ret");
thunk!(t48, 48, "ret");
thunk!(t49, 49, "ret");
thunk!(t50, 50, "ret");
thunk!(t51, 51, "ret");
thunk!(t52, 52, "ret");
thunk!(t53, 53, "ret");
thunk!(t54, 54, "ret");
thunk!(t55, 55, "ret");
thunk!(t56, 56, "ret");
thunk!(t57, 57, "ret");
thunk!(t58, 58, "ret");
thunk!(t59, 59, "ret");
thunk!(t60, 60, "ret");
thunk!(t61, 61, "ret 12");
thunk!(t62, 62, "ret 32");
thunk!(t63, 63, "ret 4");
thunk!(t64, 64, "ret");
thunk!(t65, 65, "ret");
thunk!(t66, 66, "ret");
thunk!(t67, 67, "ret");
thunk!(t68, 68, "ret 4");
thunk!(t69, 69, "ret 4");
thunk!(t70, 70, "ret");
thunk!(t71, 71, "ret");
thunk!(t72, 72, "ret");
thunk!(t73, 73, "ret");
thunk!(t74, 74, "ret");
thunk!(t75, 75, "ret");
thunk!(t76, 76, "ret");
thunk!(t77, 77, "ret");
thunk!(t78, 78, "ret");
thunk!(t79, 79, "ret");
thunk!(t80, 80, "ret");
thunk!(t81, 81, "ret");
thunk!(t82, 82, "ret");
thunk!(t83, 83, "ret");
thunk!(t84, 84, "ret");
thunk!(t85, 85, "ret");
thunk!(t86, 86, "ret");
thunk!(t87, 87, "ret");
thunk!(t88, 88, "ret");
thunk!(t89, 89, "ret");
thunk!(t90, 90, "ret");
thunk!(t91, 91, "ret");
thunk!(t92, 92, "ret");
thunk!(t93, 93, "ret 8");
thunk!(t94, 94, "ret 8");
thunk!(t95, 95, "ret");
thunk!(t96, 96, "ret");
thunk!(t97, 97, "ret 4");
thunk!(t98, 98, "ret");
thunk!(t99, 99, "ret 4");
thunk!(t100, 100, "ret");
thunk!(t101, 101, "ret");
thunk!(t102, 102, "ret 4");
thunk!(t103, 103, "ret");
thunk!(t104, 104, "ret 4");
thunk!(t105, 105, "ret 4");
thunk!(t106, 106, "ret 4");
thunk!(t107, 107, "ret 4");
thunk!(t108, 108, "ret 4");
thunk!(t109, 109, "ret 4");
thunk!(t110, 110, "ret 4");
thunk!(t111, 111, "ret 4");
thunk!(t112, 112, "ret");
thunk!(t113, 113, "ret 4");
thunk!(t114, 114, "ret 4");
thunk!(t115, 115, "ret");
thunk!(t116, 116, "ret");
thunk!(t117, 117, "ret");
thunk!(t118, 118, "ret 8");
thunk!(t119, 119, "ret");
thunk!(t120, 120, "ret");
thunk!(t121, 121, "ret 8");
thunk!(t122, 122, "ret");
thunk!(t123, 123, "ret");
thunk!(t124, 124, "ret 20");
thunk!(t125, 125, "ret 4");
thunk!(t126, 126, "ret");
thunk!(t127, 127, "ret");
thunk!(t128, 128, "ret");
thunk!(t129, 129, "ret");
thunk!(t130, 130, "ret");
thunk!(t131, 131, "ret");
thunk!(t132, 132, "ret 4");
thunk!(t133, 133, "ret");
thunk!(t134, 134, "ret 20");
thunk!(t135, 135, "ret 16");
thunk!(t136, 136, "ret");
thunk!(t137, 137, "ret");
thunk!(t138, 138, "ret");
thunk!(t139, 139, "ret 4");
thunk!(t140, 140, "ret");
thunk!(t141, 141, "ret");
thunk!(t142, 142, "ret");
thunk!(t143, 143, "ret");
thunk!(t144, 144, "ret");
thunk!(t145, 145, "ret");
thunk!(t146, 146, "ret");
thunk!(t147, 147, "ret 4");
thunk!(t148, 148, "ret");
thunk!(t149, 149, "ret");
thunk!(t150, 150, "ret");
thunk!(t151, 151, "ret");
thunk!(t152, 152, "ret");
thunk!(t153, 153, "ret");
thunk!(t154, 154, "ret");
thunk!(t155, 155, "ret");
thunk!(t156, 156, "ret");
thunk!(t157, 157, "ret");
thunk!(t158, 158, "ret");
thunk!(t159, 159, "ret");
thunk!(t160, 160, "ret");
thunk!(t161, 161, "ret");
thunk!(t162, 162, "ret");
thunk!(t163, 163, "ret");
thunk!(t164, 164, "ret");
thunk!(t165, 165, "ret");
thunk!(t166, 166, "ret");
thunk!(t167, 167, "ret");
thunk!(t168, 168, "ret");
thunk!(t169, 169, "ret");
thunk!(t170, 170, "ret");
thunk!(t171, 171, "ret");
thunk!(t172, 172, "ret");
thunk!(t173, 173, "ret");
thunk!(t174, 174, "ret 4");
thunk!(t175, 175, "ret 12");
thunk!(t176, 176, "ret 8");
thunk!(t177, 177, "ret 4");
thunk!(t178, 178, "ret 24");
thunk!(t179, 179, "ret 4");
thunk!(t180, 180, "ret 12");
thunk!(t181, 181, "ret 12");
thunk!(t182, 182, "ret 4");
thunk!(t183, 183, "ret 12");
thunk!(t184, 184, "ret 12");
thunk!(t185, 185, "ret 4");
thunk!(t186, 186, "ret");
thunk!(t187, 187, "ret");
thunk!(t188, 188, "ret");
thunk!(t189, 189, "ret");
thunk!(t190, 190, "ret");
thunk!(t191, 191, "ret");
thunk!(t192, 192, "ret");
thunk!(t193, 193, "ret");
thunk!(t194, 194, "ret");
thunk!(t195, 195, "ret");
thunk!(t196, 196, "ret");
thunk!(t197, 197, "ret");
thunk!(t198, 198, "ret");
thunk!(t199, 199, "ret");
thunk!(t200, 200, "ret");
thunk!(t201, 201, "ret");
thunk!(t202, 202, "ret");
thunk!(t203, 203, "ret 8");
thunk!(t204, 204, "ret");
thunk!(t205, 205, "ret");
thunk!(t206, 206, "ret");
thunk!(t207, 207, "ret 12");
thunk!(t208, 208, "ret");
thunk!(t209, 209, "ret");
thunk!(t210, 210, "ret");
thunk!(t211, 211, "ret");
thunk!(t212, 212, "ret");
thunk!(t213, 213, "ret");
thunk!(t214, 214, "ret");
thunk!(t215, 215, "ret");
thunk!(t216, 216, "ret");
thunk!(t217, 217, "ret");
thunk!(t218, 218, "ret");
thunk!(t219, 219, "ret");
thunk!(t220, 220, "ret");
thunk!(t221, 221, "ret");
thunk!(t222, 222, "ret");
thunk!(t223, 223, "ret");
thunk!(t224, 224, "ret");
thunk!(t225, 225, "ret 4");
thunk!(t226, 226, "ret 4");
thunk!(t227, 227, "ret");
thunk!(t228, 228, "ret");
thunk!(t229, 229, "ret");
thunk!(t230, 230, "ret");
thunk!(t231, 231, "ret");
thunk!(t232, 232, "ret");
thunk!(t233, 233, "ret 4");
thunk!(t234, 234, "ret");
thunk!(t235, 235, "ret 4");
thunk!(t236, 236, "ret");
thunk!(t237, 237, "ret");
thunk!(t238, 238, "ret");
thunk!(t239, 239, "ret");
thunk!(t240, 240, "ret");
thunk!(t241, 241, "ret");
thunk!(t242, 242, "ret");
thunk!(t243, 243, "ret");
thunk!(t244, 244, "ret");
thunk!(t245, 245, "ret");
thunk!(t246, 246, "ret");
thunk!(t247, 247, "ret");
thunk!(t248, 248, "ret");
thunk!(t249, 249, "ret");
thunk!(t250, 250, "ret");
thunk!(t251, 251, "ret");
thunk!(t252, 252, "ret");
thunk!(t253, 253, "ret");
thunk!(t254, 254, "ret");

// --- the table ------------------------------------------------------------------------------------

const COUNT: usize = 255;

static PORTS: [Port; COUNT] = [
    port("allocator<>::element_size", 0x004cb100, &B0, 0, 0, eax_const::<0x68> as Run, None, t0),
    port("allocator<>::element_size_2", 0x004cb5b0, &B1, 0, 0, eax_const::<0x48> as Run, None, t1),
    port("IAllocator::init_default_pool", 0x004cb760, &B2, 0, 0, nop as Run, None, t2),
    port("IAllocator::release", 0x004cb770, &B3, 0, 0, nop as Run, None, t3),
    port("IAllocator::num_used", 0x004cb790, &B4, 0, 0, zero_eax as Run, None, t4),
    port("IAllocator::num_free", 0x004cb7a0, &B5, 0, 0, zero_eax as Run, None, t5),
    port("ovl_base::is_topmost", 0x00501990, &B6, 0, 0, load_ecx_b::<0x5> as Run, None, t6),
    port("ovl_base::takes_hud_focus", 0x005019a0, &B7, 0, 0, al_const::<1> as Run, None, t7),
    port("r_particle_inst::casts_shadow", 0x0050eea0, &B8, 0, 0, al_const::<0> as Run, None, t8),
    port("r_particle_inst::compute_pass_data", 0x0050eeb0, &B9, 3, 12, al_const::<1> as Run, None, t9),
    port("r_particle_inst::prepare_pass", 0x0050eec0, &B10, 3, 12, al_const::<1> as Run, None, t10),
    port("r_particle_inst::render_shadow", 0x0050eed0, &B11, 2, 8, nop as Run, None, t11),
    port("r_particle_inst::is_ready", 0x0050eee0, &B12, 0, 0, al_const::<1> as Run, None, t12),
    port("r_particle_ribbon_inst::casts_shadow", 0x0050ef00, &B13, 0, 0, al_const::<0> as Run, None, t13),
    port("r_particle_ribbon_inst::render_shadow", 0x0050ef30, &B14, 2, 8, nop as Run, None, t14),
    port("rl_decal_inst::compute_pass_data", 0x0052e410, &B15, 3, 12, al_const::<0> as Run, None, t15),
    port("rl_decal_inst::prepare_pass", 0x0052e420, &B16, 3, 12, al_const::<0> as Run, None, t16),
    port("rl_mesh_inst::get_env_handle", 0x00530100, &B17, 0, 0, load_ecx::<0x78> as Run, None, t17),
    port("activity_instance::load_from_table", 0x0053e220, &B18, 2, 8, set_byte_ecx::<0x4,1> as Run, Some(ecx_byte_w::<0x4> as Writes), t18),
    port("activity_level_instance::load_xtbl", 0x0053f7e0, &B19, 1, 4, nop as Run, None, t19),
    port("allocator<>::element_size_3", 0x00540bb0, &B20, 0, 0, eax_const::<0x24> as Run, None, t20),
    port("allocator<>::element_size_4", 0x00557420, &B21, 0, 0, eax_const::<0x43c> as Run, None, t21),
    port("guardian_angel_info::get_activity_type", 0x0056c8a0, &B22, 0, 0, eax_const::<0x6> as Run, None, t22),
    port("guardian_angel_level_instance::load_xtbl", 0x00572140, &B23, 1, 4, nop as Run, None, t23),
    port("heli_info::get_activity_type", 0x005752a0, &B24, 0, 0, eax_const::<0xf> as Run, None, t24),
    port("human_torch_info::get_activity_type", 0x0058ce60, &B25, 0, 0, eax_const::<0x7> as Run, None, t25),
    port("piracy_info::get_activity_type", 0x00591a90, &B26, 0, 0, eax_const::<0x9> as Run, None, t26),
    port("racing_activity_info::get_activity_type", 0x00598000, &B27, 0, 0, eax_const::<0xe> as Run, None, t27),
    port("security_info::get_activity_type", 0x00598300, &B28, 0, 0, eax_const::<0xb> as Run, None, t28),
    port("sewage_info::get_activity_type", 0x005b4200, &B29, 0, 0, eax_const::<0xc> as Run, None, t29),
    port("zombie_info::get_activity_type", 0x005b6df0, &B30, 0, 0, eax_const::<0xd> as Run, None, t30),
    port("fuzz_info::get_activity_type", 0x005bc370, &B31, 0, 0, eax_const::<0x5> as Run, None, t31),
    port("fight_club_activity_info::get_activity_type", 0x005d7f00, &B32, 0, 0, eax_const::<0x3> as Run, None, t32),
    port("diversion_type_info::on_human_killed", 0x005f1510, &B33, 5, 20, nop as Run, None, t33),
    port("diversion_type_info::on_object_killed", 0x005f1520, &B34, 4, 16, nop as Run, None, t34),
    port("diversion_type_info::on_vehicle_destroyed", 0x005f1530, &B35, 4, 16, nop as Run, None, t35),
    port("diversion_type_info::on_game_event_5", 0x005f1540, &B36, 6, 24, nop as Run, None, t36),
    port("ambulance_diversion_type_info::get_hud_icon_name", 0x005f16f0, &B37, 0, 0, eax_const::<0xdf12f0> as Run, None, t37),
    port("chop_shop_diversion_type::create_instance", 0x00600f80, &B38, 1, 4, zero_eax as Run, None, t38),
    port("combat_tricks_diversion_type::create_instance", 0x00601620, &B39, 1, 4, zero_eax as Run, None, t39),
    port("hostage_diversion_type_info::can_start", 0x00612860, &B40, 0, 0, load_ecx_b::<0x1d> as Run, None, t40),
    port("cheat_infinite_ammo_on", 0x00685470, &B41, 0, 0, or_dword::<0x27a6a40,0x40> as Run, Some(set_dword_w::<0x27a6a40> as Writes), t41),
    port("cheat_unlimited_clip_on", 0x00685690, &B42, 0, 0, or_dword::<0x27a6a40,0x1000> as Run, Some(set_dword_w::<0x27a6a40> as Writes), t42),
    port("cheat_infinite_ammo_off", 0x00685b10, &B43, 0, 0, and_dword::<0x27a6a40,0xffffffbf> as Run, Some(set_dword_w::<0x27a6a40> as Writes), t43),
    port("cheat_unlimited_clip_off", 0x00685d40, &B44, 0, 0, and_dword::<0x27a6a40,0xffffefff> as Run, Some(set_dword_w::<0x27a6a40> as Writes), t44),
    port("character::on_loaded", 0x006e8710, &B45, 1, 4, al_const::<0> as Run, None, t45),
    port("luafn_Completion_reset_respect_screen", 0x00750260, &B46, 0, 0, set_byte_z::<0x2527f9a,0> as Run, Some(set_byte_w::<0x2527f9a> as Writes), t46),
    port("luafn_credits_are_finished", 0x00751820, &B47, 0, 0, set_byte_z::<0xe8d56b,1> as Run, Some(set_byte_w::<0xe8d56b> as Writes), t47),
    port("overlay_stack_remove", 0x0075b4e0, &B48, 0, 0, overlay_remove as Run, Some(overlay_remove_w as Writes), t48),
    port("ovl_chat_message::hide", 0x0075d280, &B49, 0, 0, set_dword_ecx::<0x250,0xffffffff> as Run, Some(ecx_dword_w::<0x250> as Writes), t49),
    port("ovl_text_entry::show", 0x007617d0, &B50, 0, 0, set_byte_ecx::<0xc,1> as Run, Some(ecx_byte_w::<0xc> as Writes), t50),
    port("pause_menu_open", 0x00779ab0, &B51, 1, 0, pause_menu_open as Run, Some(pause_open_w as Writes), t51),
    port("luafn_vcust_using_lightset", 0x007b81f0, &B52, 1, 0, lua_bool::<1> as Run, Some(lua_bool_w as Writes), t52),
    port("luafn_pcr_mark_changed", 0x007d3a00, &B53, 0, 0, set_byte_z::<0x25289fa,1> as Run, Some(set_byte_w::<0x25289fa> as Writes), t53),
    port("gang_customization_setup_cameras", 0x007de2f0, &B54, 0, 0, gang_cams as Run, Some(gang_cams_w as Writes), t54),
    port("luafn_get_ps3_button_swap", 0x007ecc80, &B55, 1, 0, lua_bool::<0> as Run, Some(lua_bool_w as Writes), t55),
    port("luafn_is_german_build", 0x007ed790, &B56, 1, 0, lua_bool::<0> as Run, Some(lua_bool_w as Writes), t56),
    port("luafn_coop_start_new_live", 0x007eec70, &B57, 0, 0, set_byte_z::<0x2527b9e,1> as Run, Some(set_byte_w::<0x2527b9e> as Writes), t57),
    port("luafn_coop_start_new_syslink", 0x007eec80, &B58, 0, 0, set_byte_z::<0x2527b9f,1> as Run, Some(set_byte_w::<0x2527b9f> as Writes), t58),
    port("luafn_user_has_online_privlage", 0x007eed30, &B59, 1, 0, lua_bool::<1> as Run, Some(lua_bool_w as Writes), t59),
    port("luafn_user_has_voice_privlage", 0x007eed50, &B60, 1, 0, lua_bool::<0> as Run, Some(lua_bool_w as Writes), t60),
    port("detour_multi_registration::get_dirty_fields", 0x008278a0, &B61, 3, 12, ax_zero as Run, None, t61),
    port("detour_multi_registration::get_send_priority", 0x00827d20, &B62, 8, 32, ax_zero as Run, None, t62),
    port("detour_multi_registration::can_migrate", 0x00827d60, &B63, 1, 4, al_const::<0> as Run, None, t63),
    port("multi_custom_set_friendly_fire_impl", 0x00842720, &B64, 0, 0, custom_friendly_fire as Run, Some(custom_ff_w as Writes), t64),
    port("multi_custom_set_ammo_impl", 0x008427a0, &B65, 0, 0, custom_sprint::<0x2a4d2c5,0x2a4d2ab> as Run, Some(custom_set_w as Writes), t65),
    port("multi_custom_set_sprint_impl", 0x008427f0, &B66, 0, 0, custom_sprint::<0x2a4d2c4,0x2a4d2a6> as Run, Some(custom_set_w as Writes), t66),
    port("multi_custom_set_tag_status_impl", 0x00842980, &B67, 0, 0, custom_tag_status as Run, Some(custom_tag_w as Writes), t67),
    port("pipebomb_multi_registration::can_migrate", 0x0085e320, &B68, 1, 4, al_const::<1> as Run, None, t68),
    port("rocket_multi_registration::can_migrate", 0x008705a0, &B69, 1, 4, al_const::<1> as Run, None, t69),
    port("luafn_mp_completion_return_to_lobby", 0x00890fb0, &B70, 0, 0, set_byte_z::<0x2529292,1> as Run, Some(set_byte_w::<0x2529292> as Writes), t70),
    port("luafn_mp_completion_quit_game", 0x00890fc0, &B71, 0, 0, set_byte_z::<0x2529293,1> as Run, Some(set_byte_w::<0x2529293> as Writes), t71),
    port("notoriety_spawn_group::is_active", 0x00923070, &B72, 0, 0, load_ecx_b::<0x2a4> as Run, None, t72),
    port("nav_tri_edge_verts", 0x00a02a50, &B73, 1, 0, nav_tri_edge_verts as Run, Some(nav_edge_w as Writes), t73),
    port("luafn_audio_is_xmp_playing", 0x00a28620, &B74, 1, 0, lua_bool::<0> as Run, Some(lua_bool_w as Writes), t74),
    port("luafn_cellphone_clear_between_call_delay", 0x00a2a8c0, &B75, 0, 0, set_dword_z::<0xeb8088,0xffffffff> as Run, Some(set_dword_w::<0xeb8088> as Writes), t75),
    port("luafn_npc_max_corpses_reset", 0x00a3cdf0, &B76, 0, 0, set_dword_z::<0xe881c8,0xffffffff> as Run, Some(set_dword_w::<0xe881c8> as Writes), t76),
    port("luafn_screenshot_check_done", 0x00a432a0, &B77, 1, 0, lua_bool::<1> as Run, Some(lua_bool_w as Writes), t77),
    port("luafn_finishers_disable", 0x00a46ef0, &B78, 0, 0, set_byte_z::<0x2527c84,0> as Run, Some(set_byte_w::<0x2527c84> as Writes), t78),
    port("luafn_bink_movie_done", 0x00a47b90, &B79, 1, 0, lua_bool::<1> as Run, Some(lua_bool_w as Writes), t79),
    port("script_item::get_object", 0x00a595e0, &B80, 0, 0, load_ecx::<0x64> as Run, None, t80),
    port("script_item::is_spawned", 0x00a59db0, &B81, 0, 0, is_set::<0x64> as Run, None, t81),
    port("script_mover::is_spawned", 0x00a5af90, &B82, 0, 0, is_set::<0x90> as Run, None, t82),
    port("script_npc::get_object", 0x00a60040, &B83, 0, 0, load_ecx::<0x174> as Run, None, t83),
    port("script_npc::is_spawned", 0x00a60df0, &B84, 0, 0, is_set::<0x174> as Run, None, t84),
    port("script_searchlight::is_spawned", 0x00a64380, &B85, 0, 0, is_set::<0x54> as Run, None, t85),
    port("script_searchlight::get_object", 0x00a64390, &B86, 0, 0, load_ecx::<0x54> as Run, None, t86),
    port("script_vehicle::get_object", 0x00a6ecf0, &B87, 0, 0, load_ecx::<0xb8> as Run, None, t87),
    port("ammo_store_clear", 0x00b693e0, &B88, 0, 0, ammo_store_clear as Run, Some(ammo_clear_w as Writes), t88),
    port("weapon_info_find_by_hash", 0x00b71760, &B89, 0, 0, weapon_info_find_by_hash as Run, None, t89),
    port("r_dynamic_water_detailed_inst::get_env_handle", 0x00b75c20, &B90, 0, 0, load_ecx::<0x6fc> as Run, None, t90),
    port("vint_callback::scalar_deleting_dtor", 0x00b8cea0, &B91, 0, 0, set_dword_ecx::<0x18,0x0> as Run, Some(ecx_dword_w::<0x18> as Writes), t91),
    port("vint_datagroup_find_by_hash", 0x00b93d30, &B92, 0, 0, vint_datagroup_find_by_hash as Run, None, t92),
    port("vint_element_base::set_screen_nw", 0x00b96a90, &B93, 2, 8, al_const::<0> as Run, None, t93),
    port("vint_element_base::set_screen_se", 0x00b96b20, &B94, 2, 8, al_const::<0> as Run, None, t94),
    port("vint_element_base::get_type", 0x00b9c310, &B95, 0, 0, global_deref::<0xebdf6c> as Run, None, t95),
    port("vint_element_group::get_type", 0x00b9c3f0, &B96, 0, 0, global_deref::<0xebe010> as Run, None, t96),
    port("vint_object_factory_imp<>::index_of", 0x00b9cc80, &B97, 1, 4, index_of::<0x6bca1af3,6,false> as Run, None, t97),
    port("vint_object_animation::get_type", 0x00b9ce00, &B98, 0, 0, global_deref::<0xebe09c> as Run, None, t98),
    port("vint_object_factory_imp<>::index_of_2", 0x00b9d580, &B99, 1, 4, index_of::<0x51eb851f,5,false> as Run, None, t99),
    port("vint_object_tween::get_type", 0x00b9dcf0, &B100, 0, 0, global_deref::<0xebe2f8> as Run, None, t100),
    port("vint_tween_compare", 0x00b9ea50, &B101, 2, 0, tween_compare as Run, None, t101),
    port("vint_object_factory_imp<>::index_of_3", 0x00ba01a0, &B102, 1, 4, index_of::<0x8d3dcb09,7,true> as Run, None, t102),
    port("vint_element_bitmap::get_type", 0x00ba0c80, &B103, 0, 0, global_deref::<0xebe3f4> as Run, None, t103),
    port("vint_object_factory_imp<>::index_of_4", 0x00ba16d0, &B104, 1, 4, index_of::<0x5397829d,6,false> as Run, None, t104),
    port("vint_object_factory_imp<>::index_of_5", 0x00ba25f0, &B105, 1, 4, index_of::<0x7e07e07f,7,false> as Run, None, t105),
    port("vint_object_factory_imp<>::index_of_6", 0x00ba4570, &B106, 1, 4, index_of::<0x4325c53f,6,false> as Run, None, t106),
    port("vint_object_factory_imp<>::index_of_7", 0x00ba4b40, &B107, 1, 4, index_of::<0x6bca1af3,6,false> as Run, None, t107),
    port("vint_object_factory_imp<>::index_of_8", 0x00ba5340, &B108, 1, 4, index_of::<0x63e7063f,6,false> as Run, None, t108),
    port("vint_object_factory_imp<>::index_of_9", 0x00ba6050, &B109, 1, 4, index_of::<0xa0a0a0a1,7,true> as Run, None, t109),
    port("vint_object_factory_imp<>::index_of_10", 0x00ba6830, &B110, 1, 4, index_of::<0x66666667,6,false> as Run, None, t110),
    port("vint_object_factory_imp<>::index_of_11", 0x00ba6f00, &B111, 1, 4, index_of::<0xd20d20d3,7,true> as Run, None, t111),
    port("rand_int_range", 0x00bdcf50, &B112, 0, 0, rand_int_range as Run, Some(rand_w as Writes), t112),
    port("script_thread_push_bool", 0x00cdfcb0, &B113, 1, 4, push_bool as Run, Some(push_bool_w as Writes), t113),
    port("ovl_base::set_topmost", 0x005019b0, &B114, 1, 4, set_byte_arg_ecx::<5> as Run, Some(ecx_byte_w::<5> as Writes), t114),
    port("r_particle_inst::get_env_handle", 0x0050eef0, &B115, 0, 0, zero_eax as Run, None, t115),
    port("r_particle_ribbon_inst::is_ready", 0x0050ef40, &B116, 0, 0, al_const::<1> as Run, None, t116),
    port("rl_animated_mesh_inst::is_ready", 0x0052e3a0, &B117, 0, 0, al_const::<0> as Run, None, t117),
    port("rl_decal_inst::render_shadow", 0x0052e400, &B118, 2, 8, nop as Run, None, t118),
    port("rl_renderable_inst::is_ready", 0x00530120, &B119, 0, 0, al_const::<1> as Run, None, t119),
    port("skirmish_spawn_group::is_active", 0x00567740, &B120, 0, 0, skirmish_is_active as Run, None, t120),
    port("piracy_instance::load_from_table", 0x00591aa0, &B121, 2, 8, set_byte_ecx::<4,1> as Run, Some(ecx_byte_w::<4> as Writes), t121),
    port("fuzz_crime::get_bar_title", 0x005bc230, &B122, 0, 0, zero_eax as Run, None, t122),
    port("demo_derby_sp_info::get_activity_type", 0x005f0df0, &B123, 0, 0, eax_const::<0x1> as Run, None, t123),
    port("diversion_type_info::on_game_event_15", 0x005f1560, &B124, 5, 20, nop as Run, None, t124),
    port("diversion_type_info::on_game_event_21", 0x005f1580, &B125, 1, 4, nop as Run, None, t125),
    port("barnstorming_diversion_type::get_type", 0x005f7f70, &B126, 0, 0, eax_const::<0x7> as Run, None, t126),
    port("base_jumping_diversion_type_info::get_type", 0x005f9580, &B127, 0, 0, eax_const::<0x2> as Run, None, t127),
    port("base_jumping_diversion_type_info::can_start", 0x005f95f0, &B128, 0, 0, al_const::<0> as Run, None, t128),
    port("chop_shop_diversion_type::get_type", 0x005fff20, &B129, 0, 0, eax_const::<0x4> as Run, None, t129),
    port("combat_tricks_diversion_type::get_type", 0x00601630, &B130, 0, 0, eax_const::<0x23> as Run, None, t130),
    port("diversion_type_info::can_start", 0x00602ae0, &B131, 0, 0, al_const::<0> as Run, None, t131),
    port("diversion_type_info::create_instance", 0x00602af0, &B132, 1, 4, zero_eax as Run, None, t132),
    port("driver_bailout_diversion_type_info::get_type", 0x00609900, &B133, 0, 0, eax_const::<0x5> as Run, None, t133),
    port("driver_bailout_diversion_type_info::on_game_event_15", 0x00609dd0, &B134, 5, 20, bailout_event_15 as Run, Some(bailout_add_w as Writes), t134),
    port("driver_bailout_diversion_type_info::on_vehicle_destroyed", 0x00609df0, &B135, 4, 16, bailout_vehicle_destroyed as Run, Some(bailout_add_w as Writes), t135),
    port("firetruck_diversion_type_info::get_type", 0x0060b8c0, &B136, 0, 0, eax_const::<0x8> as Run, None, t136),
    port("hostage_diversion_type_info::get_type", 0x006124b0, &B137, 0, 0, eax_const::<0xa> as Run, None, t137),
    port("mugging_diversion_type::init", 0x00614200, &B138, 0, 0, mugging_init as Run, Some(ecx_range_w::<0x18,8> as Writes), t138),
    port("safecracking_diversion_type_info::create_instance", 0x00615db0, &B139, 1, 4, zero_eax as Run, None, t139),
    port("safecracking_diversion_type_info::get_type", 0x00615e10, &B140, 0, 0, eax_const::<0xe> as Run, None, t140),
    port("stunt_peel_out_diversion_type::init", 0x0061dab0, &B141, 0, 0, peel_out_init as Run, Some(peel_out_init_w as Writes), t141),
    port("survival_diversion_type_info::get_type", 0x0061f0d0, &B142, 0, 0, eax_const::<0xf> as Run, None, t142),
    port("tow_truck_diversion_type_info::get_start_mode", 0x0062e950, &B143, 0, 0, tow_truck_start_mode as Run, None, t143),
    port("fraud_info::get_activity_type", 0x0064bf90, &B144, 0, 0, eax_const::<0x4> as Run, None, t144),
    port("trafficking_info::get_activity_type", 0x0065e370, &B145, 0, 0, eax_const::<0xa> as Run, None, t145),
    port("escort_info::get_activity_type", 0x0066d940, &B146, 0, 0, eax_const::<0x2> as Run, None, t146),
    port("escort_level_instance::load_xtbl", 0x006770a0, &B147, 1, 4, nop as Run, None, t147),
    port("cheat_vehicle_drop", 0x00685270, &B148, 1, 0, cheat_vehicle_drop as Run, Some(cheat_vehicle_drop_w as Writes), t148),
    port("cheat_weather_off", 0x00685b20, &B149, 0, 0, cheat_weather_off as Run, Some(cheat_weather_off_w as Writes), t149),
    port("luafn_Completion_show_reward_screen", 0x00750270, &B150, 1, 0, lua_flag_byte::<0x2527f9a> as Run, Some(lua_bool_w as Writes), t150),
    port("ovl_login::hide", 0x0075f0a0, &B151, 0, 0, set_dword_ecx::<0x250,0xffffffff> as Run, Some(ecx_dword_w::<0x250> as Writes), t151),
    port("ovl_message::hide", 0x0075fa40, &B152, 0, 0, set_dword_ecx::<0x250,0xffffffff> as Run, Some(ecx_dword_w::<0x250> as Writes), t152),
    port("ovl_text_entry::render", 0x00761830, &B153, 0, 0, nop as Run, None, t153),
    port("luafn_pause_menu_open_achievements", 0x0077aa50, &B154, 0, 0, zero_eax as Run, None, t154),
    port("vint_value_ctor", 0x007903c0, &B155, 0, 0, vint_value_ctor as Run, Some(ecx_dword_w::<0> as Writes), t155),
    port("luafn_weapon_store_get_is_cache", 0x007b3550, &B156, 1, 0, lua_flag_byte::<0x25287cb> as Run, Some(lua_bool_w as Writes), t156),
    port("luafn_weapon_store_is_melee_store", 0x007b3580, &B157, 1, 0, lua_melee_store as Run, Some(lua_bool_w as Writes), t157),
    port("luafn_pcr_is_plastic_surgeon", 0x007d22e0, &B158, 1, 0, lua_flag_byte::<0x25289f8> as Run, Some(lua_bool_w as Writes), t158),
    port("luafn_pcr_avatar_changed", 0x007d35e0, &B159, 1, 0, lua_flag_byte::<0x25289fa> as Run, Some(lua_bool_w as Writes), t159),
    port("luafn_gang_customization_revert_gang_sign", 0x007de180, &B160, 0, 0, gang_sign_revert as Run, Some(gang_sign_revert_w as Writes), t160),
    port("mp_players_kick_player", 0x007e2bd0, &B161, 0, 0, nth_player as Run, None, t161),
    port("luafn_use_imperial_units", 0x007ec9a0, &B162, 1, 0, lua_flag_dword_eq::<0xe8c470,0> as Run, Some(lua_bool_w as Writes), t162),
    port("luafn_get_is_matchmaking", 0x007ee160, &B163, 1, 0, lua_flag_byte::<0x2527e8f> as Run, Some(lua_bool_w as Writes), t163),
    port("luafn_get_is_strongarm", 0x007ee290, &B164, 1, 0, lua_flag_dword_eq_cl::<0xe8b210,0xd> as Run, Some(lua_bool_w as Writes), t164),
    port("luafn_get_is_team_game", 0x007ee2c0, &B165, 1, 0, lua_team_game as Run, Some(lua_bool_w as Writes), t165),
    port("luafn_get_is_custom_game", 0x007ee5f0, &B166, 1, 0, lua_flag_dword_eq_cl::<0xe8b20c,3> as Run, Some(lua_bool_w as Writes), t166),
    port("luafn_is_coop_start_screen", 0x007eeab0, &B167, 1, 0, lua_flag_byte::<0x2527ba0> as Run, Some(lua_bool_w as Writes), t167),
    port("luafn_is_connected_to_network", 0x007eec90, &B168, 1, 0, lua_flag_dword_eq::<0x2529334,4> as Run, Some(lua_bool_w as Writes), t168),
    port("luafn_is_connected_to_service", 0x007eed00, &B169, 1, 0, lua_flag_dword_eq::<0x2529334,4> as Run, Some(lua_bool_w as Writes), t169),
    port("luafn_is_mp_tutorial_playing", 0x007ef430, &B170, 1, 0, lua_flag_byte::<0x2527ebd> as Run, Some(lua_bool_w as Writes), t170),
    port("luafn_get_is_tutorial", 0x007ef460, &B171, 1, 0, lua_flag_dword_eq_cl::<0xe96c00,9> as Run, Some(lua_bool_w as Writes), t171),
    port("luafn_is_mp_storage_blank", 0x007ef6e0, &B172, 1, 0, lua_flag_dword_eq::<0x2528c24,1> as Run, Some(lua_bool_w as Writes), t172),
    port("luafn_event_tracking_interface_exit", 0x007ef740, &B173, 0, 0, zero_eax as Run, None, t173),
    port("detour_multi_registration::on_register", 0x008278b0, &B174, 1, 4, registration_on_register as Run, Some(registration_on_register_w as Writes), t174),
    port("detour_multi_registration::write_destroy_data", 0x00827c20, &B175, 3, 12, nop as Run, None, t175),
    port("detour_multi_registration::capture_snapshot", 0x00827cb0, &B176, 2, 8, capture_snapshot as Run, Some(capture_snapshot_w as Writes), t176),
    port("detour_multi_registration::free_snapshot", 0x00827cc0, &B177, 1, 4, nop as Run, None, t177),
    port("detour_multi_registration::write_state_delta", 0x00827cd0, &B178, 6, 24, write_state_delta as Run, Some(write_state_delta_w as Writes), t178),
    port("detour_multi_registration::on_owner_left", 0x00827d50, &B179, 1, 4, eax_const::<0x1> as Run, None, t179),
    port("detour_multi_registration::on_ownership_taken", 0x00827d70, &B180, 3, 12, nop as Run, None, t180),
    port("pipebomb_multi_registration::on_ownership_taken", 0x0085e330, &B181, 3, 12, registration_ownership_taken::<4,0x11c> as Run, Some(registration_ownership_taken_w::<4,0x11c> as Writes), t181),
    port("player_multi_registration::can_migrate", 0x00869a80, &B182, 1, 4, al_const::<0> as Run, None, t182),
    port("prop_multi_registration::on_ownership_taken", 0x0086d570, &B183, 3, 12, registration_ownership_taken::<0xc,0x118> as Run, Some(registration_ownership_taken_w::<0xc,0x118> as Writes), t183),
    port("rocket_multi_registration::on_ownership_taken", 0x008705b0, &B184, 3, 12, registration_ownership_taken::<4,0x11c> as Run, Some(registration_ownership_taken_w::<4,0x11c> as Writes), t184),
    port("trigger_multi_registration::on_register", 0x00871aa0, &B185, 1, 4, registration_on_register as Run, Some(registration_on_register_w as Writes), t185),
    port("luafn_pause_menu_objective_force_show", 0x00a27e40, &B186, 0, 0, objective_force_show as Run, Some(objective_force_show_w as Writes), t186),
    port("luafn_character_slots_clear_reservations", 0x00a2b440, &B187, 0, 0, slots_fill::<0x215dfdc,0> as Run, Some(slots_fill_w::<0x215dfdc> as Writes), t187),
    port("luafn_character_slots_clear_caps", 0x00a2b4f0, &B188, 0, 0, slots_fill::<0x215dfb8,0xa> as Run, Some(slots_fill_w::<0x215dfb8> as Writes), t188),
    port("luafn_homie_mission_lock_all", 0x00a2d8a0, &B189, 0, 0, homie_set::<1> as Run, Some(homie_set_w as Writes), t189),
    port("luafn_homie_mission_reset", 0x00a2d8d0, &B190, 0, 0, homie_set::<0> as Run, Some(homie_set_w as Writes), t190),
    port("luafn_human_shielding_disabled", 0x00a31960, &B191, 1, 0, lua_bool::<0> as Run, Some(lua_bool_w as Writes), t191),
    port("luafn_is_compressed_prologue", 0x00a356d0, &B192, 1, 0, lua_flag_byte::<0x252981f> as Run, Some(lua_bool_w as Writes), t192),
    port("luafn_never_die", 0x00a3b6d0, &B193, 0, 0, zero_eax as Run, None, t193),
    port("luafn_player_creation_is_open_do", 0x00a40d30, &B194, 1, 0, lua_flag_dword_ne::<0x25289cc> as Run, Some(lua_bool_w as Writes), t194),
    port("luafn_player_creation_set_opening", 0x00a40d60, &B195, 0, 0, player_creation_set_opening as Run, Some(player_creation_set_opening_w as Writes), t195),
    port("luafn_reset_mp_tutorial_count", 0x00a431a0, &B196, 0, 0, reset_mp_tutorial_count as Run, Some(reset_mp_tutorial_count_w as Writes), t196),
    port("luafn_set_mission_author", 0x00a44f10, &B197, 0, 0, zero_eax as Run, None, t197),
    port("luafn_sprint_reset_local_overrides", 0x00a46320, &B198, 0, 0, sprint_reset_overrides as Run, Some(sprint_reset_overrides_w as Writes), t198),
    port("luafn_finishers_enable", 0x00a46f00, &B199, 0, 0, set_byte_z::<0x2527c84,1> as Run, Some(set_byte_w::<0x2527c84> as Writes), t199),
    port("luafn_parachute_disable", 0x00a46f10, &B200, 0, 0, set_byte_z::<0x2527c85,0> as Run, Some(set_byte_w::<0x2527c85> as Writes), t200),
    port("luafn_parachute_enable", 0x00a46f20, &B201, 0, 0, set_byte_z::<0x2527c85,1> as Run, Some(set_byte_w::<0x2527c85> as Writes), t201),
    port("luafn_is_demo_execution", 0x00a4e610, &B202, 1, 0, lua_flag_byte::<0x25272cf> as Run, Some(lua_bool_w as Writes), t202),
    port("script_mover::on_object_spawned", 0x00a5af80, &B203, 2, 8, nop as Run, None, t203),
    port("script_trigger::on_object_spawned", 0x00a67ea0, &B204, 0, 0, script_trigger_spawned as Run, Some(ecx_dword_w::<0xa4> as Writes), t204),
    port("script_vehicle::is_spawned", 0x00a6f780, &B205, 0, 0, is_set::<0xb8> as Run, None, t205),
    port("r_tree_inst::casts_shadow", 0x00a8da40, &B206, 0, 0, al_const::<1> as Run, None, t206),
    port("vehicle_light_manager::init", 0x00ad47c0, &B207, 3, 12, light_manager_init as Run, Some(ecx_range_w::<4,0x24> as Writes), t207),
    port("vehicle_light_manager::get_light_type", 0x00ad4d70, &B208, 0, 0, eax_const::<0xffffffff> as Run, None, t208),
    port("vehicle_light_manager::tag_prefix", 0x00ad4e10, &B209, 0, 0, zero_eax as Run, None, t209),
    port("vehicle_headlight_manager::get_light_type", 0x00ad59a0, &B210, 0, 0, eax_const::<0x1> as Run, None, t210),
    port("vehicle_brakelight_manager::tag_prefix", 0x00ad5e20, &B211, 0, 0, eax_const::<0x2> as Run, None, t211),
    port("vehicle_turn_signal_manager::get_light_name", 0x00ad6090, &B212, 0, 0, turn_signal_light_name as Run, None, t212),
    port("vehicle_turn_signal_manager::get_light_type", 0x00ad60b0, &B213, 0, 0, turn_signal_light_type as Run, None, t213),
    port("vehicle_reverselight_manager::get_light_type", 0x00ad61e0, &B214, 0, 0, eax_const::<0x5> as Run, None, t214),
    port("vehicle_runninglight_manager::get_light_type", 0x00ad6640, &B215, 0, 0, eax_const::<0x7> as Run, None, t215),
    port("vint_args_consume", 0x00b8ce20, &B216, 1, 0, vint_args_consume as Run, Some(vint_args_consume_w as Writes), t216),
    port("vint_value_set_color3", 0x00b8cf10, &B217, 0, 0, vint_value_set_color3 as Run, Some(vint_value_set_color3_w as Writes), t217),
    port("vint_bitmap_addref", 0x00b92d30, &B218, 0, 0, handle_addref::<0xf0,36,0x31bc518,0x14,0x18> as Run, Some(handle_addref_w::<0xf0,36,0x31bc518,0x14,0x18> as Writes), t218),
    port("vint_font_addref", 0x00b92da0, &B219, 0, 0, vint_font_addref as Run, Some(vint_font_addref_w as Writes), t219),
    port("vint_sound_addref", 0x00b92e20, &B220, 0, 0, handle_addref::<0x32,24,0x31be700,8,0xc> as Run, Some(handle_addref_w::<0x32,24,0x31be700,8,0xc> as Writes), t220),
    port("vint_callback_from_ref", 0x00b93750, &B221, 0, 0, vint_callback_from_ref as Run, None, t221),
    port("vint_datagroup_find_by_handle", 0x00b94260, &B222, 0, 0, vint_datagroup_find_by_handle as Run, None, t222),
    port("vint_dataitem_find_by_hash", 0x00b951b0, &B223, 0, 0, vint_dataitem_find_by_hash as Run, None, t223),
    port("vint_writer_write_16_bytes", 0x00b990f0, &B224, 0, 0, vint_writer_write_16 as Run, Some(vint_writer_write_16_w as Writes), t224),
    port("vint_object_base::is_type", 0x00b9c250, &B225, 1, 4, al_const::<0> as Run, None, t225),
    port("vint_element_base::is_type", 0x00b9c2f0, &B226, 1, 4, is_type_global::<0xebdf6c> as Run, None, t226),
    port("vint_element_base::pre_render", 0x00b9c330, &B227, 0, 0, nop as Run, None, t227),
    port("vint_element_group_ctor", 0x00b9c390, &B228, 0, 0, vint_ctor::<0xe4d334> as Run, Some(vint_ctor_w as Writes), t228),
    port("vint_element_group_dtor", 0x00b9c3b0, &B229, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t229),
    port("vint_element_group::update", 0x00b9c520, &B230, 0, 0, nop as Run, None, t230),
    port("vint_object_animation_ctor", 0x00b9cdb0, &B231, 0, 0, vint_ctor::<0xe4d454> as Run, Some(vint_ctor_w as Writes), t231),
    port("vint_object_animation_dtor", 0x00b9cdd0, &B232, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t232),
    port("vint_object_animation::is_type", 0x00b9cde0, &B233, 1, 4, is_type_global::<0xebe09c> as Run, None, t233),
    port("vint_enum_register", 0x00b9d7e0, &B234, 2, 0, vint_enum_register as Run, Some(vint_enum_register_w as Writes), t234),
    port("vint_object_tween::is_type", 0x00b9dcd0, &B235, 1, 4, is_type_global::<0xebe2f8> as Run, None, t235),
    port("vint_callback_find_by_handle", 0x00ba0980, &B236, 0, 0, vint_callback_find_by_handle as Run, None, t236),
    port("vint_element_bitmap_ctor", 0x00ba0bf0, &B237, 0, 0, vint_ctor_bitmap::<0xe4d85c> as Run, Some(vint_ctor_bitmap_w as Writes), t237),
    port("vint_element_gradient_ctor", 0x00ba1870, &B238, 0, 0, vint_ctor_gradient as Run, Some(vint_ctor_bitmap_w as Writes), t238),
    port("vint_element_gradient::get_type", 0x00ba1910, &B239, 0, 0, global_deref::<0xebe530> as Run, None, t239),
    port("vint_element_text::get_type", 0x00ba2850, &B240, 0, 0, global_deref::<0xebe790> as Run, None, t240),
    port("vint_element_point_ctor", 0x00ba46b0, &B241, 0, 0, vint_ctor::<0xe4dd4c> as Run, Some(vint_ctor_w as Writes), t241),
    port("vint_element_point_dtor", 0x00ba46d0, &B242, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t242),
    port("vint_element_point::get_type", 0x00ba4710, &B243, 0, 0, global_deref::<0xebe7e8> as Run, None, t243),
    port("vint_element_clip_ctor", 0x00ba4c60, &B244, 0, 0, vint_ctor::<0xe4de34> as Run, Some(vint_ctor_w as Writes), t244),
    port("vint_element_clip_dtor", 0x00ba4c80, &B245, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t245),
    port("vint_element_clip::get_type", 0x00ba4cc0, &B246, 0, 0, global_deref::<0xebe850> as Run, None, t246),
    port("vint_element_bitmap_circle_ctor", 0x00ba5460, &B247, 0, 0, vint_ctor_bitmap::<0xe4df54> as Run, Some(vint_ctor_bitmap_w as Writes), t247),
    port("vint_element_bitmap_circle::get_type", 0x00ba54f0, &B248, 0, 0, global_deref::<0xebe96c> as Run, None, t248),
    port("vint_element_sr2_map_ctor", 0x00ba61a0, &B249, 0, 0, vint_ctor::<0xe4e074> as Run, Some(vint_ctor_w as Writes), t249),
    port("vint_element_sr2_map_dtor", 0x00ba61c0, &B250, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t250),
    port("vint_element_sr2_map::get_type", 0x00ba6200, &B251, 0, 0, global_deref::<0xebe9d0> as Run, None, t251),
    port("vint_element_video_ctor", 0x00ba6950, &B252, 0, 0, vint_ctor::<0xe4e174> as Run, Some(vint_ctor_w as Writes), t252),
    port("vint_element_video_dtor", 0x00ba6970, &B253, 0, 0, set_dword_ecx::<0,0xe4d240> as Run, Some(ecx_dword_w::<0> as Writes), t253),
    port("vint_element_video::get_type", 0x00ba69b0, &B254, 0, 0, global_deref::<0xebea18> as Run, None, t254),
];

// --- install ------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for port in PORTS.iter() {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, port.body, &[]) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_leaves: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, port.body, port.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_leaves: replaced {ok}/{COUNT} with Rust"));
}

/// One line for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_leaves: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const B0: [u8; 6] = [
    0xB8, 0x68, 0x00, 0x00, 0x00, 0xC3,
];
const B1: [u8; 6] = [
    0xB8, 0x48, 0x00, 0x00, 0x00, 0xC3,
];
const B2: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B3: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B4: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B5: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B6: [u8; 5] = [
    0x8A, 0x41, 0x05, 0xC3, 0xCC,
];
const B7: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B8: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B9: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B10: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B11: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B12: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B13: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B14: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B15: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x0C, 0x00,
];
const B16: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x0C, 0x00,
];
const B17: [u8; 5] = [
    0x8B, 0x41, 0x78, 0xC3, 0xCC,
];
const B18: [u8; 7] = [
    0xC6, 0x41, 0x04, 0x01, 0xC2, 0x08, 0x00,
];
const B19: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B20: [u8; 6] = [
    0xB8, 0x24, 0x00, 0x00, 0x00, 0xC3,
];
const B21: [u8; 6] = [
    0xB8, 0x3C, 0x04, 0x00, 0x00, 0xC3,
];
const B22: [u8; 6] = [
    0xB8, 0x06, 0x00, 0x00, 0x00, 0xC3,
];
const B23: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B24: [u8; 6] = [
    0xB8, 0x0F, 0x00, 0x00, 0x00, 0xC3,
];
const B25: [u8; 6] = [
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3,
];
const B26: [u8; 6] = [
    0xB8, 0x09, 0x00, 0x00, 0x00, 0xC3,
];
const B27: [u8; 6] = [
    0xB8, 0x0E, 0x00, 0x00, 0x00, 0xC3,
];
const B28: [u8; 6] = [
    0xB8, 0x0B, 0x00, 0x00, 0x00, 0xC3,
];
const B29: [u8; 6] = [
    0xB8, 0x0C, 0x00, 0x00, 0x00, 0xC3,
];
const B30: [u8; 6] = [
    0xB8, 0x0D, 0x00, 0x00, 0x00, 0xC3,
];
const B31: [u8; 6] = [
    0xB8, 0x05, 0x00, 0x00, 0x00, 0xC3,
];
const B32: [u8; 6] = [
    0xB8, 0x03, 0x00, 0x00, 0x00, 0xC3,
];
const B33: [u8; 5] = [
    0xC2, 0x14, 0x00, 0xCC, 0xCC,
];
const B34: [u8; 5] = [
    0xC2, 0x10, 0x00, 0xCC, 0xCC,
];
const B35: [u8; 5] = [
    0xC2, 0x10, 0x00, 0xCC, 0xCC,
];
const B36: [u8; 5] = [
    0xC2, 0x18, 0x00, 0xCC, 0xCC,
];
const B37: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B38: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B39: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B40: [u8; 5] = [
    0x8A, 0x41, 0x1D, 0xC3, 0xCC,
];
const B41: [u8; 8] = [
    0x83, 0x0D, 0x40, 0x6A, 0x7A, 0x02, 0x40, 0xC3,
];
const B42: [u8; 11] = [
    0x81, 0x0D, 0x40, 0x6A, 0x7A, 0x02, 0x00, 0x10, 0x00, 0x00, 0xC3,
];
const B43: [u8; 8] = [
    0x83, 0x25, 0x40, 0x6A, 0x7A, 0x02, 0xBF, 0xC3,
];
const B44: [u8; 11] = [
    0x81, 0x25, 0x40, 0x6A, 0x7A, 0x02, 0xFF, 0xEF, 0xFF, 0xFF, 0xC3,
];
const B45: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B46: [u8; 10] = [
    0xC6, 0x05, 0x9A, 0x7F, 0x52, 0x02, 0x00, 0x33, 0xC0, 0xC3,
];
const B47: [u8; 10] = [
    0xC6, 0x05, 0x6B, 0xD5, 0xE8, 0x00, 0x01, 0x33, 0xC0, 0xC3,
];
const B48: [u8; 85] = [
    0x8B, 0x0D, 0xE8, 0xC4, 0xEA, 0x00, 0x56, 0x33, 0xF6, 0x85, 0xC9, 0x7E, 0x46, 0x8B, 0x15, 0xE0,
    0xC4, 0xEA, 0x00, 0x8B, 0x04, 0xB2, 0x3B, 0x07, 0x75, 0x32, 0x83, 0xE9, 0x01, 0x3B, 0xF1, 0x8B,
    0xC6, 0x89, 0x0D, 0xE8, 0xC4, 0xEA, 0x00, 0x7D, 0x2A, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x4C, 0x82, 0x04, 0x89, 0x0C, 0x82, 0x8B, 0x0D, 0xE8, 0xC4, 0xEA, 0x00, 0x8B, 0x15, 0xE0,
    0xC4, 0xEA, 0x00, 0x83, 0xC0, 0x01, 0x3B, 0xC1, 0x7C, 0xE6, 0xEB, 0x03, 0x83, 0xC6, 0x01, 0x3B,
    0xF1, 0x7C, 0xC0, 0x5E, 0xC3,
];
const B49: [u8; 11] = [
    0xC7, 0x81, 0x50, 0x02, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B50: [u8; 5] = [
    0xC6, 0x41, 0x0C, 0x01, 0xC3,
];
const B51: [u8; 61] = [
    0x80, 0x3D, 0xDF, 0x8A, 0x52, 0x02, 0x01, 0x8B, 0x44, 0x24, 0x04, 0xC6, 0x05, 0x56, 0x83, 0x52,
    0x02, 0x00, 0xC7, 0x05, 0x6C, 0x83, 0x52, 0x02, 0x02, 0x00, 0x00, 0x00, 0xA3, 0x9C, 0x8E, 0x9C,
    0x02, 0x74, 0x19, 0xA1, 0x7C, 0xCF, 0xE8, 0x00, 0x83, 0xF8, 0x0D, 0x74, 0x0F, 0x83, 0xF8, 0x0B,
    0x74, 0x0A, 0xC7, 0x05, 0x7C, 0xCF, 0xE8, 0x00, 0x0D, 0x00, 0x00, 0x00, 0xC3,
];
const B52: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x01, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B53: [u8; 10] = [
    0xC6, 0x05, 0xFA, 0x89, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B54: [u8; 40] = [
    0x80, 0x3D, 0xDF, 0x8A, 0x52, 0x02, 0x01, 0xA3, 0x18, 0x8A, 0x52, 0x02, 0x74, 0x19, 0xA1, 0x7C,
    0xCF, 0xE8, 0x00, 0x83, 0xF8, 0x0D, 0x74, 0x0F, 0x83, 0xF8, 0x0B, 0x74, 0x0A, 0xC7, 0x05, 0x7C,
    0xCF, 0xE8, 0x00, 0x03, 0x00, 0x00, 0x00, 0xC3,
];
const B55: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B56: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B57: [u8; 10] = [
    0xC6, 0x05, 0x9E, 0x7B, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B58: [u8; 10] = [
    0xC6, 0x05, 0x9F, 0x7B, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B59: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x01, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B60: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B61: [u8; 6] = [
    0x66, 0x33, 0xC0, 0xC2, 0x0C, 0x00,
];
const B62: [u8; 6] = [
    0x66, 0x33, 0xC0, 0xC2, 0x20, 0x00,
];
const B63: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B64: [u8; 68] = [
    0x33, 0xC9, 0x2B, 0xC1, 0x74, 0x13, 0x83, 0xE8, 0x01, 0x74, 0x07, 0x83, 0xE8, 0x01, 0x75, 0x0E,
    0xEB, 0x0C, 0xB9, 0x02, 0x00, 0x00, 0x00, 0xEB, 0x05, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x39, 0x0D,
    0x34, 0xD1, 0xA4, 0x02, 0x66, 0xC7, 0x05, 0x18, 0xB2, 0xE8, 0x00, 0xFF, 0xFF, 0x74, 0x14, 0x89,
    0x0D, 0x34, 0xD1, 0xA4, 0x02, 0xC6, 0x05, 0x2C, 0xD1, 0xA4, 0x02, 0x01, 0xC6, 0x05, 0x2E, 0xD1,
    0xA4, 0x02, 0x01, 0xC3,
];
const B65: [u8; 73] = [
    0x3A, 0x05, 0xC5, 0xD2, 0xA4, 0x02, 0xB1, 0x01, 0x74, 0x11, 0xA2, 0xC5, 0xD2, 0xA4, 0x02, 0x88,
    0x0D, 0xC0, 0xD2, 0xA4, 0x02, 0x88, 0x0D, 0xC2, 0xD2, 0xA4, 0x02, 0x3A, 0xC1, 0x66, 0xC7, 0x05,
    0x18, 0xB2, 0xE8, 0x00, 0xFF, 0xFF, 0x75, 0x20, 0x33, 0xC0, 0x38, 0x05, 0xAB, 0xD2, 0xA4, 0x02,
    0x74, 0x16, 0xA2, 0xAB, 0xD2, 0xA4, 0x02, 0xA3, 0xA0, 0xD2, 0xA4, 0x02, 0x88, 0x0D, 0x98, 0xD2,
    0xA4, 0x02, 0x88, 0x0D, 0x9A, 0xD2, 0xA4, 0x02, 0xC3,
];
const B66: [u8; 73] = [
    0x3A, 0x05, 0xC4, 0xD2, 0xA4, 0x02, 0xB1, 0x01, 0x74, 0x11, 0xA2, 0xC4, 0xD2, 0xA4, 0x02, 0x88,
    0x0D, 0xC0, 0xD2, 0xA4, 0x02, 0x88, 0x0D, 0xC2, 0xD2, 0xA4, 0x02, 0x3A, 0xC1, 0x66, 0xC7, 0x05,
    0x18, 0xB2, 0xE8, 0x00, 0xFF, 0xFF, 0x75, 0x20, 0x33, 0xC0, 0x38, 0x05, 0xA6, 0xD2, 0xA4, 0x02,
    0x74, 0x16, 0xA2, 0xA6, 0xD2, 0xA4, 0x02, 0xA3, 0xA0, 0xD2, 0xA4, 0x02, 0x88, 0x0D, 0x98, 0xD2,
    0xA4, 0x02, 0x88, 0x0D, 0x9A, 0xD2, 0xA4, 0x02, 0xC3,
];
const B67: [u8; 76] = [
    0x38, 0x81, 0xA4, 0xD2, 0xA4, 0x02, 0xB2, 0x01, 0x74, 0x1C, 0x88, 0x81, 0xA4, 0xD2, 0xA4, 0x02,
    0xC7, 0x05, 0xA0, 0xD2, 0xA4, 0x02, 0x00, 0x00, 0x00, 0x00, 0x88, 0x15, 0x98, 0xD2, 0xA4, 0x02,
    0x88, 0x15, 0x9A, 0xD2, 0xA4, 0x02, 0x80, 0x3D, 0x9C, 0xD2, 0xA4, 0x02, 0x00, 0x66, 0xC7, 0x05,
    0x18, 0xB2, 0xE8, 0x00, 0xFF, 0xFF, 0x74, 0x13, 0xC6, 0x05, 0x9C, 0xD2, 0xA4, 0x02, 0x00, 0x88,
    0x15, 0x98, 0xD2, 0xA4, 0x02, 0x88, 0x15, 0x9A, 0xD2, 0xA4, 0x02, 0xC3,
];
const B68: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x04, 0x00,
];
const B69: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x04, 0x00,
];
const B70: [u8; 10] = [
    0xC6, 0x05, 0x92, 0x92, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B71: [u8; 10] = [
    0xC6, 0x05, 0x93, 0x92, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B72: [u8; 7] = [
    0x8A, 0x81, 0xA4, 0x02, 0x00, 0x00, 0xC3,
];
const B73: [u8; 58] = [
    0x53, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x8B, 0x04, 0xB7, 0x8B, 0x18, 0x89, 0x1A, 0x8B, 0x58, 0x04,
    0x89, 0x5A, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x42, 0x08, 0x8D, 0x46, 0x01, 0x99, 0xBE, 0x03, 0x00,
    0x00, 0x00, 0xF7, 0xFE, 0x5E, 0x5B, 0x8B, 0x14, 0x97, 0x8B, 0x02, 0x89, 0x01, 0x8B, 0x42, 0x04,
    0x89, 0x41, 0x04, 0x8B, 0x52, 0x08, 0x89, 0x51, 0x08, 0xC3,
];
const B74: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B75: [u8; 13] = [
    0xC7, 0x05, 0x88, 0x80, 0xEB, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x33, 0xC0, 0xC3,
];
const B76: [u8; 13] = [
    0xC7, 0x05, 0xC8, 0x81, 0xE8, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x33, 0xC0, 0xC3,
];
const B77: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x01, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B78: [u8; 10] = [
    0xC6, 0x05, 0x84, 0x7C, 0x52, 0x02, 0x00, 0x33, 0xC0, 0xC3,
];
const B79: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x01, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B80: [u8; 5] = [
    0x8B, 0x41, 0x64, 0xC3, 0xCC,
];
const B81: [u8; 9] = [
    0x33, 0xC0, 0x39, 0x41, 0x64, 0x0F, 0x95, 0xC0, 0xC3,
];
const B82: [u8; 12] = [
    0x33, 0xC0, 0x39, 0x81, 0x90, 0x00, 0x00, 0x00, 0x0F, 0x95, 0xC0, 0xC3,
];
const B83: [u8; 7] = [
    0x8B, 0x81, 0x74, 0x01, 0x00, 0x00, 0xC3,
];
const B84: [u8; 12] = [
    0x33, 0xC0, 0x39, 0x81, 0x74, 0x01, 0x00, 0x00, 0x0F, 0x95, 0xC0, 0xC3,
];
const B85: [u8; 9] = [
    0x33, 0xC0, 0x39, 0x41, 0x54, 0x0F, 0x95, 0xC0, 0xC3,
];
const B86: [u8; 5] = [
    0x8B, 0x41, 0x54, 0xC3, 0xCC,
];
const B87: [u8; 7] = [
    0x8B, 0x81, 0xB8, 0x00, 0x00, 0x00, 0xC3,
];
const B88: [u8; 42] = [
    0x56, 0x33, 0xF6, 0x33, 0xC9, 0x39, 0x35, 0x50, 0x7B, 0x2D, 0x02, 0x7E, 0x1B, 0x33, 0xD2, 0x90,
    0x8B, 0x07, 0x03, 0xC2, 0x83, 0xC1, 0x01, 0x89, 0x70, 0x04, 0x89, 0x70, 0x08, 0x83, 0xC2, 0x0C,
    0x3B, 0x0D, 0x50, 0x7B, 0x2D, 0x02, 0x7C, 0xE8, 0x5E, 0xC3,
];
const B89: [u8; 37] = [
    0x8B, 0x15, 0xD0, 0x7B, 0x2D, 0x02, 0xA1, 0xCC, 0x7B, 0x2D, 0x02, 0x33, 0xC9, 0x85, 0xD2, 0x7E,
    0x11, 0x39, 0x70, 0x40, 0x74, 0x0E, 0x83, 0xC1, 0x01, 0x05, 0x98, 0x04, 0x00, 0x00, 0x3B, 0xCA,
    0x7C, 0xEF, 0x33, 0xC0, 0xC3,
];
const B90: [u8; 7] = [
    0x8B, 0x81, 0xFC, 0x06, 0x00, 0x00, 0xC3,
];
const B91: [u8; 8] = [
    0xC7, 0x41, 0x18, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const B92: [u8; 40] = [
    0x85, 0xD2, 0x74, 0x21, 0x8B, 0x0D, 0x04, 0xA2, 0x52, 0x02, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x15,
    0x39, 0x50, 0x08, 0x75, 0x06, 0x80, 0x78, 0x1C, 0x00, 0x74, 0x0C, 0x8B, 0x00, 0x3B, 0xC1, 0x74,
    0x04, 0x85, 0xC0, 0x75, 0xEB, 0x33, 0xC0, 0xC3,
];
const B93: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B94: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B95: [u8; 8] = [
    0xA1, 0x6C, 0xDF, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B96: [u8; 8] = [
    0xA1, 0x10, 0xE0, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B97: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0xF3, 0x1A,
    0xCA, 0x6B, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B98: [u8; 8] = [
    0xA1, 0x9C, 0xE0, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B99: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x1F, 0x85,
    0xEB, 0x51, 0xF7, 0xEA, 0xC1, 0xFA, 0x05, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B100: [u8; 8] = [
    0xA1, 0xF8, 0xE2, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B101: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x08, 0x8B, 0x54, 0x24, 0x08, 0x8B, 0x02, 0x8B, 0x49, 0x78, 0x2B,
    0x48, 0x78, 0x8B, 0xC1, 0xC3,
];
const B102: [u8; 51] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0x79, 0x14, 0x8B, 0xF3, 0x2B, 0xF7, 0xB8, 0x09,
    0xCB, 0x3D, 0x8D, 0xF7, 0xEE, 0x03, 0xD6, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03,
    0xC2, 0x3B, 0xDF, 0x5F, 0x5E, 0x5B, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF,
    0xC2, 0x04, 0x00,
];
const B103: [u8; 8] = [
    0xA1, 0xF4, 0xE3, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B104: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x9D, 0x82,
    0x97, 0x53, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B105: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x7F, 0xE0,
    0x07, 0x7E, 0xF7, 0xEA, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B106: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x3F, 0xC5,
    0x25, 0x43, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B107: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0xF3, 0x1A,
    0xCA, 0x6B, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B108: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x3F, 0x06,
    0xE7, 0x63, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B109: [u8; 51] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0x79, 0x14, 0x8B, 0xF3, 0x2B, 0xF7, 0xB8, 0xA1,
    0xA0, 0xA0, 0xA0, 0xF7, 0xEE, 0x03, 0xD6, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03,
    0xC2, 0x3B, 0xDF, 0x5F, 0x5E, 0x5B, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF,
    0xC2, 0x04, 0x00,
];
const B110: [u8; 47] = [
    0x56, 0x8B, 0x71, 0x14, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xD7, 0x2B, 0xD6, 0xB8, 0x67, 0x66,
    0x66, 0x66, 0xF7, 0xEA, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x3B, 0xFE,
    0x5F, 0x5E, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF, 0xC2, 0x04, 0x00,
];
const B111: [u8; 51] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0x79, 0x14, 0x8B, 0xF3, 0x2B, 0xF7, 0xB8, 0xD3,
    0x20, 0x0D, 0xD2, 0xF7, 0xEE, 0x03, 0xD6, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03,
    0xC2, 0x3B, 0xDF, 0x5F, 0x5E, 0x5B, 0x72, 0x05, 0x3B, 0x41, 0x10, 0x72, 0x03, 0x83, 0xC8, 0xFF,
    0xC2, 0x04, 0x00,
];
const B112: [u8; 54] = [
    0x8B, 0x15, 0xBC, 0x3A, 0xEC, 0x00, 0x8B, 0x04, 0x95, 0xC0, 0x3A, 0xEC, 0x00, 0x83, 0xC2, 0x01,
    0x81, 0xFA, 0x00, 0x08, 0x00, 0x00, 0x89, 0x15, 0xBC, 0x3A, 0xEC, 0x00, 0x7C, 0x0A, 0xC7, 0x05,
    0xBC, 0x3A, 0xEC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xCE, 0x83, 0xC1, 0x01, 0x33, 0xD2, 0xF7,
    0xF1, 0x8B, 0xC2, 0x03, 0xC6, 0xC3,
];
const B113: [u8; 37] = [
    0x8B, 0x41, 0x04, 0x33, 0xD2, 0x38, 0x54, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x0F, 0x95, 0xC2,
    0xC7, 0x06, 0x01, 0x00, 0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0x83, 0x41, 0x1C,
    0x01, 0x5E, 0xC2, 0x04, 0x00,
];
const B114: [u8; 10] = [
    0x8A, 0x44, 0x24, 0x04, 0x88, 0x41, 0x05, 0xC2, 0x04, 0x00,
];
const B115: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B116: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B117: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B118: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B119: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B120: [u8; 16] = [
    0x8B, 0x89, 0xF0, 0x01, 0x00, 0x00, 0x83, 0xC1, 0xFE, 0x83, 0xF9, 0x02, 0x0F, 0x96, 0xC0, 0xC3,
];
const B121: [u8; 7] = [
    0xC6, 0x41, 0x04, 0x01, 0xC2, 0x08, 0x00,
];
const B122: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B123: [u8; 6] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B124: [u8; 5] = [
    0xC2, 0x14, 0x00, 0xCC, 0xCC,
];
const B125: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B126: [u8; 6] = [
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3,
];
const B127: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B128: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B129: [u8; 6] = [
    0xB8, 0x04, 0x00, 0x00, 0x00, 0xC3,
];
const B130: [u8; 6] = [
    0xB8, 0x23, 0x00, 0x00, 0x00, 0xC3,
];
const B131: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B132: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B133: [u8; 6] = [
    0xB8, 0x05, 0x00, 0x00, 0x00, 0xC3,
];
const B134: [u8; 25] = [
    0x83, 0x79, 0x18, 0x00, 0x74, 0x10, 0x8B, 0x44, 0x24, 0x10, 0x3B, 0x41, 0x24, 0x75, 0x07, 0x8B,
    0x54, 0x24, 0x0C, 0x01, 0x51, 0x44, 0xC2, 0x14, 0x00,
];
const B135: [u8; 37] = [
    0x83, 0x79, 0x18, 0x00, 0x74, 0x1C, 0x8B, 0x44, 0x24, 0x0C, 0x3B, 0x41, 0x24, 0x75, 0x13, 0x8B,
    0x54, 0x24, 0x04, 0x8B, 0x82, 0xE4, 0x84, 0x00, 0x00, 0x8B, 0x90, 0xA8, 0x02, 0x00, 0x00, 0x01,
    0x51, 0x44, 0xC2, 0x10, 0x00,
];
const B136: [u8; 6] = [
    0xB8, 0x08, 0x00, 0x00, 0x00, 0xC3,
];
const B137: [u8; 6] = [
    0xB8, 0x0A, 0x00, 0x00, 0x00, 0xC3,
];
const B138: [u8; 15] = [
    0x33, 0xC0, 0x88, 0x41, 0x18, 0x88, 0x41, 0x19, 0x88, 0x41, 0x1A, 0x89, 0x41, 0x1C, 0xC3,
];
const B139: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B140: [u8; 6] = [
    0xB8, 0x0E, 0x00, 0x00, 0x00, 0xC3,
];
const B141: [u8; 44] = [
    0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xC6, 0x41, 0x18, 0x00, 0xC7, 0x81,
    0xB0, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x81, 0xAC, 0x00, 0x00, 0x00, 0x7E, 0x0B,
    0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x81, 0xAC, 0x00, 0x00, 0x00, 0xC3,
];
const B142: [u8; 6] = [
    0xB8, 0x0F, 0x00, 0x00, 0x00, 0xC3,
];
const B143: [u8; 16] = [
    0xA1, 0x00, 0x6C, 0xE9, 0x00, 0x83, 0xE8, 0x03, 0xF7, 0xD8, 0x1B, 0xC0, 0x83, 0xE0, 0x02, 0xC3,
];
const B144: [u8; 6] = [
    0xB8, 0x04, 0x00, 0x00, 0x00, 0xC3,
];
const B145: [u8; 6] = [
    0xB8, 0x0A, 0x00, 0x00, 0x00, 0xC3,
];
const B146: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B147: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B148: [u8; 45] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x83, 0xF8, 0xFF, 0x74, 0x21, 0x83, 0x3D, 0xAC, 0xA8, 0xEA,
    0x00, 0x02, 0x8B, 0x49, 0x04, 0x74, 0x15, 0xA3, 0xB0, 0xA8, 0xEA, 0x00, 0x89, 0x0D, 0xB4, 0xA8,
    0xEA, 0x00, 0xC7, 0x05, 0xAC, 0xA8, 0xEA, 0x00, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B149: [u8; 18] = [
    0x83, 0x25, 0x40, 0x6A, 0x7A, 0x02, 0xF7, 0xC7, 0x05, 0x14, 0x6E, 0x52, 0x02, 0x00, 0x00, 0x00,
    0x00, 0xC3,
];
const B150: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0x9A, 0x7F, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B151: [u8; 11] = [
    0xC7, 0x81, 0x50, 0x02, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B152: [u8; 11] = [
    0xC7, 0x81, 0x50, 0x02, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B153: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B154: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B155: [u8; 9] = [
    0x8B, 0xC1, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const B156: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0xCB, 0x87, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B157: [u8; 55] = [
    0x8B, 0x0D, 0x34, 0x88, 0x52, 0x02, 0x33, 0xC0, 0x83, 0xB9, 0x3C, 0x01, 0x00, 0x00, 0x27, 0x56,
    0x0F, 0x94, 0xC0, 0x33, 0xD2, 0x8B, 0xF0, 0x8B, 0x44, 0x24, 0x08, 0x8B, 0x48, 0x08, 0x85, 0xF6,
    0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08,
    0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B158: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0xF8, 0x89, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B159: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0xFA, 0x89, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B160: [u8; 13] = [
    0xA1, 0x68, 0x8A, 0x52, 0x02, 0xA3, 0x04, 0xA5, 0xFC, 0x01, 0x33, 0xC0, 0xC3,
];
const B161: [u8; 37] = [
    0x8B, 0x15, 0x78, 0x6B, 0x13, 0x02, 0x8B, 0xC2, 0x85, 0xC0, 0x74, 0x16, 0x8D, 0x64, 0x24, 0x00,
    0x85, 0xC9, 0x74, 0x10, 0x8B, 0x40, 0x24, 0x83, 0xE9, 0x01, 0x3B, 0xC2, 0x74, 0x04, 0x85, 0xC0,
    0x75, 0xEE, 0x33, 0xC0, 0xC3,
];
const B162: [u8; 37] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x39, 0x15, 0x70, 0xC4, 0xE8, 0x00, 0xC7,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x94, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B163: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0x8F, 0x7E, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B164: [u8; 48] = [
    0x32, 0xC9, 0x83, 0x3D, 0x10, 0xB2, 0xE8, 0x00, 0x0D, 0x75, 0x02, 0xB1, 0x01, 0x8B, 0x44, 0x24,
    0x04, 0x33, 0xD2, 0x84, 0xC9, 0x0F, 0x95, 0xC2, 0x56, 0x8B, 0x70, 0x08, 0xC7, 0x06, 0x01, 0x00,
    0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B165: [u8; 54] = [
    0xA1, 0x10, 0xB2, 0xE8, 0x00, 0x32, 0xC9, 0x83, 0xF8, 0x0D, 0x74, 0x05, 0x83, 0xF8, 0x0C, 0x75,
    0x02, 0xB1, 0x01, 0x8B, 0x44, 0x24, 0x04, 0x33, 0xD2, 0x84, 0xC9, 0x0F, 0x95, 0xC2, 0x56, 0x8B,
    0x70, 0x08, 0xC7, 0x06, 0x01, 0x00, 0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B166: [u8; 48] = [
    0x32, 0xC9, 0x83, 0x3D, 0x0C, 0xB2, 0xE8, 0x00, 0x03, 0x75, 0x02, 0xB1, 0x01, 0x8B, 0x44, 0x24,
    0x04, 0x33, 0xD2, 0x84, 0xC9, 0x0F, 0x95, 0xC2, 0x56, 0x8B, 0x70, 0x08, 0xC7, 0x06, 0x01, 0x00,
    0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B167: [u8; 37] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x38, 0x15, 0xA0, 0x7B, 0x52, 0x02, 0xC7,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x95, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B168: [u8; 38] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x83, 0x3D, 0x34, 0x93, 0x52, 0x02, 0x04,
    0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x94, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B169: [u8; 38] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x83, 0x3D, 0x34, 0x93, 0x52, 0x02, 0x04,
    0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x94, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B170: [u8; 37] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x38, 0x15, 0xBD, 0x7E, 0x52, 0x02, 0xC7,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x95, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B171: [u8; 48] = [
    0x32, 0xC9, 0x83, 0x3D, 0x00, 0x6C, 0xE9, 0x00, 0x09, 0x75, 0x02, 0xB1, 0x01, 0x8B, 0x44, 0x24,
    0x04, 0x33, 0xD2, 0x84, 0xC9, 0x0F, 0x95, 0xC2, 0x56, 0x8B, 0x70, 0x08, 0xC7, 0x06, 0x01, 0x00,
    0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B172: [u8; 38] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x83, 0x3D, 0x24, 0x8C, 0x52, 0x02, 0x01,
    0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x94, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B173: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B174: [u8; 46] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x23, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x39, 0x48, 0x44, 0x75,
    0x0A, 0x81, 0x88, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
const B175: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B176: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x08, 0x33, 0xC9, 0x89, 0x48, 0x10, 0x89, 0x48, 0x0C, 0xC2, 0x08, 0x00,
];
const B177: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B178: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x0C, 0xC7, 0x40, 0x0C, 0x00, 0x00, 0x00, 0x00, 0xC2, 0x18, 0x00,
];
const B179: [u8; 8] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
const B180: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B181: [u8; 58] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x2F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x1B, 0x39, 0x48, 0x44, 0x75,
    0x16, 0x83, 0x78, 0x4C, 0x04, 0x75, 0x10, 0x8B, 0x80, 0x1C, 0x01, 0x00, 0x00, 0xC7, 0x40, 0x04,
    0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const B182: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B183: [u8; 58] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x2F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x1B, 0x39, 0x48, 0x44, 0x75,
    0x16, 0x83, 0x78, 0x4C, 0x0C, 0x75, 0x10, 0x8B, 0x80, 0x18, 0x01, 0x00, 0x00, 0xC7, 0x40, 0x04,
    0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const B184: [u8; 58] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x2F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x1B, 0x39, 0x48, 0x44, 0x75,
    0x16, 0x83, 0x78, 0x4C, 0x04, 0x75, 0x10, 0x8B, 0x80, 0x1C, 0x01, 0x00, 0x00, 0xC7, 0x40, 0x04,
    0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0xC2, 0x0C, 0x00,
];
const B185: [u8; 46] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x23, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x39, 0x48, 0x44, 0x75,
    0x0A, 0x81, 0x88, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
const B186: [u8; 20] = [
    0xC7, 0x05, 0xF0, 0xAC, 0x9D, 0x02, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x05, 0xC1, 0x83, 0x52, 0x02,
    0x01, 0x33, 0xC0, 0xC3,
];
const B187: [u8; 48] = [
    0x33, 0xC0, 0xA3, 0xDC, 0xDF, 0x15, 0x02, 0xA3, 0xE0, 0xDF, 0x15, 0x02, 0xA3, 0xE4, 0xDF, 0x15,
    0x02, 0xA3, 0xE8, 0xDF, 0x15, 0x02, 0xA3, 0xEC, 0xDF, 0x15, 0x02, 0xA3, 0xF0, 0xDF, 0x15, 0x02,
    0xA3, 0xF4, 0xDF, 0x15, 0x02, 0xA3, 0xF8, 0xDF, 0x15, 0x02, 0xA3, 0xFC, 0xDF, 0x15, 0x02, 0xC3,
];
const B188: [u8; 53] = [
    0xB8, 0x0A, 0x00, 0x00, 0x00, 0xA3, 0xB8, 0xDF, 0x15, 0x02, 0xA3, 0xBC, 0xDF, 0x15, 0x02, 0xA3,
    0xC0, 0xDF, 0x15, 0x02, 0xA3, 0xC4, 0xDF, 0x15, 0x02, 0xA3, 0xC8, 0xDF, 0x15, 0x02, 0xA3, 0xCC,
    0xDF, 0x15, 0x02, 0xA3, 0xD0, 0xDF, 0x15, 0x02, 0xA3, 0xD4, 0xDF, 0x15, 0x02, 0xA3, 0xD8, 0xDF,
    0x15, 0x02, 0x33, 0xC0, 0xC3,
];
const B189: [u8; 35] = [
    0x8B, 0x0D, 0x00, 0x84, 0x52, 0x02, 0x85, 0xC9, 0x7E, 0x16, 0xB8, 0x18, 0xC3, 0x9D, 0x02, 0x90,
    0xC7, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0xAC, 0x03, 0x00, 0x00, 0x83, 0xE9, 0x01, 0x75, 0xF0,
    0x33, 0xC0, 0xC3,
];
const B190: [u8; 35] = [
    0x8B, 0x0D, 0x00, 0x84, 0x52, 0x02, 0x85, 0xC9, 0x7E, 0x16, 0xB8, 0x18, 0xC3, 0x9D, 0x02, 0x90,
    0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0xAC, 0x03, 0x00, 0x00, 0x83, 0xE9, 0x01, 0x75, 0xF0,
    0x33, 0xC0, 0xC3,
];
const B191: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B192: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0x1F, 0x98, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B193: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B194: [u8; 37] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x33, 0xD2, 0x39, 0x15, 0xCC, 0x89, 0x52, 0x02, 0xC7,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x95, 0xC2, 0x89, 0x51, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B195: [u8; 18] = [
    0x33, 0xC0, 0x38, 0x05, 0xBF, 0xD6, 0xE8, 0x00, 0x74, 0x07, 0xC6, 0x05, 0x89, 0x89, 0x52, 0x02,
    0x01, 0xC3,
];
const B196: [u8; 18] = [
    0x33, 0xC0, 0xA3, 0xCC, 0x9D, 0x52, 0x02, 0xA3, 0xD0, 0x9D, 0x52, 0x02, 0xA2, 0xD4, 0x9D, 0x52,
    0x02, 0xC3,
];
const B197: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B198: [u8; 21] = [
    0x83, 0xC8, 0xFF, 0xA3, 0xA8, 0x6A, 0xE9, 0x00, 0xA3, 0xAC, 0x6A, 0xE9, 0x00, 0xA3, 0xB0, 0x6A,
    0xE9, 0x00, 0x33, 0xC0, 0xC3,
];
const B199: [u8; 10] = [
    0xC6, 0x05, 0x84, 0x7C, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B200: [u8; 10] = [
    0xC6, 0x05, 0x85, 0x7C, 0x52, 0x02, 0x00, 0x33, 0xC0, 0xC3,
];
const B201: [u8; 10] = [
    0xC6, 0x05, 0x85, 0x7C, 0x52, 0x02, 0x01, 0x33, 0xC0, 0xC3,
];
const B202: [u8; 42] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0x56, 0x0F, 0xB6, 0x35, 0xCF, 0x72, 0x52, 0x02, 0x33,
    0xD2, 0x85, 0xF6, 0x0F, 0x95, 0xC2, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0x5E, 0x89, 0x51, 0x08,
    0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B203: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B204: [u8; 10] = [
    0x8B, 0x41, 0x0C, 0x89, 0x81, 0xA4, 0x00, 0x00, 0x00, 0xC3,
];
const B205: [u8; 12] = [
    0x33, 0xC0, 0x39, 0x81, 0xB8, 0x00, 0x00, 0x00, 0x0F, 0x95, 0xC0, 0xC3,
];
const B206: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B207: [u8; 39] = [
    0x8B, 0x44, 0x24, 0x04, 0x89, 0x41, 0x10, 0x33, 0xC0, 0x89, 0x41, 0x08, 0x89, 0x41, 0x0C, 0x88,
    0x41, 0x04, 0x83, 0xC8, 0xFF, 0x89, 0x41, 0x14, 0x89, 0x41, 0x18, 0x89, 0x41, 0x1C, 0x89, 0x41,
    0x20, 0x89, 0x41, 0x24, 0xC2, 0x0C, 0x00,
];
const B208: [u8; 5] = [
    0x83, 0xC8, 0xFF, 0xC3, 0xCC,
];
const B209: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B210: [u8; 6] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B211: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B212: [u8; 17] = [
    0x80, 0x79, 0x2C, 0x01, 0xB8, 0x78, 0x60, 0xE4, 0x00, 0x74, 0x05, 0xB8, 0x8C, 0x60, 0xE4, 0x00,
    0xC3,
];
const B213: [u8; 13] = [
    0x33, 0xC0, 0x80, 0x79, 0x2C, 0x01, 0x0F, 0x95, 0xC0, 0x83, 0xC0, 0x03, 0xC3,
];
const B214: [u8; 6] = [
    0xB8, 0x05, 0x00, 0x00, 0x00, 0xC3,
];
const B215: [u8; 6] = [
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3,
];
const B216: [u8; 21] = [
    0x80, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x05, 0x83, 0x38, 0x00, 0x7E, 0x03, 0x83, 0x00, 0xFF, 0xA0,
    0xF5, 0x59, 0xDC, 0x00, 0xC3,
];
const B217: [u8; 35] = [
    0x8B, 0x08, 0x85, 0xC9, 0x74, 0x05, 0x83, 0xF9, 0x06, 0x75, 0x17, 0xC7, 0x00, 0x06, 0x00, 0x00,
    0x00, 0x8B, 0x0A, 0x89, 0x48, 0x04, 0x8B, 0x4A, 0x04, 0x89, 0x48, 0x08, 0x8B, 0x52, 0x08, 0x89,
    0x50, 0x0C, 0xC3,
];
const B218: [u8; 38] = [
    0x85, 0xC9, 0x74, 0x21, 0x0F, 0xB7, 0xC1, 0x3D, 0xF0, 0x00, 0x00, 0x00, 0x73, 0x17, 0x8D, 0x04,
    0xC0, 0x39, 0x0C, 0x85, 0x2C, 0xC5, 0x1B, 0x03, 0x8D, 0x04, 0x85, 0x18, 0xC5, 0x1B, 0x03, 0x75,
    0x04, 0x83, 0x40, 0x18, 0x01, 0xC3,
];
const B219: [u8; 43] = [
    0x85, 0xC9, 0x74, 0x26, 0x66, 0x83, 0xF9, 0x0A, 0x73, 0x20, 0x0F, 0xB7, 0xC1, 0x8D, 0x14, 0xC5,
    0x00, 0x00, 0x00, 0x00, 0x2B, 0xD0, 0x39, 0x0C, 0x95, 0x08, 0xC4, 0x1B, 0x03, 0x8D, 0x04, 0x95,
    0x00, 0xC4, 0x1B, 0x03, 0x75, 0x04, 0x83, 0x40, 0x10, 0x01, 0xC3,
];
const B220: [u8; 36] = [
    0x85, 0xC9, 0x74, 0x1F, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x32, 0x73, 0x17, 0x8D, 0x04, 0x40, 0x39,
    0x0C, 0xC5, 0x08, 0xE7, 0x1B, 0x03, 0x8D, 0x04, 0xC5, 0x00, 0xE7, 0x1B, 0x03, 0x75, 0x04, 0x83,
    0x40, 0x0C, 0x01, 0xC3,
];
const B221: [u8; 51] = [
    0x83, 0x38, 0x08, 0x75, 0x2B, 0x8B, 0x50, 0x04, 0x85, 0xD2, 0x74, 0x24, 0x8B, 0xC2, 0x83, 0xE0,
    0x3F, 0x8B, 0x0C, 0x85, 0x70, 0x26, 0x2F, 0x02, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x12, 0x8B, 0xFF,
    0x39, 0x50, 0x18, 0x74, 0x0D, 0x8B, 0x40, 0x08, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xF0,
    0x33, 0xC0, 0xC3,
];
const B222: [u8; 34] = [
    0x85, 0xC9, 0x74, 0x1B, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x20, 0x73, 0x13, 0xC1, 0xE0, 0x05, 0x05,
    0xF0, 0xAC, 0x1B, 0x03, 0x39, 0x48, 0x14, 0x75, 0x06, 0x80, 0x78, 0x1C, 0x00, 0x74, 0x02, 0x33,
    0xC0, 0xC3,
];
const B223: [u8; 35] = [
    0x85, 0xD2, 0x74, 0x1C, 0x8B, 0x0D, 0xF4, 0xA1, 0x52, 0x02, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x10,
    0x39, 0x50, 0x10, 0x74, 0x0D, 0x8B, 0x40, 0x0C, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xF0,
    0x33, 0xC0, 0xC3,
];
const B224: [u8; 34] = [
    0x8B, 0x08, 0x03, 0x48, 0x0C, 0x56, 0x8B, 0x32, 0x89, 0x31, 0x8B, 0x72, 0x04, 0x89, 0x71, 0x04,
    0x8B, 0x72, 0x08, 0x89, 0x71, 0x08, 0x8B, 0x52, 0x0C, 0x89, 0x51, 0x0C, 0x83, 0x40, 0x0C, 0x10,
    0x5E, 0xC3,
];
const B225: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B226: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x01, 0x0F, 0x94, 0xC0, 0xC2,
    0x04, 0x00,
];
const B227: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B228: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x34, 0xD3, 0xE4, 0x00,
    0xC3,
];
const B229: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B230: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B231: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x54, 0xD4, 0xE4, 0x00,
    0xC3,
];
const B232: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B233: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x9C, 0xE0, 0xEB, 0x00, 0x3B, 0x01, 0x0F, 0x94, 0xC0, 0xC2,
    0x04, 0x00,
];
const B234: [u8; 52] = [
    0x8B, 0x0D, 0x74, 0xA2, 0x52, 0x02, 0x83, 0xF9, 0x20, 0x73, 0x28, 0x8B, 0xC1, 0xC1, 0xE0, 0x04,
    0x89, 0x90, 0x70, 0x21, 0x2F, 0x02, 0x8B, 0x54, 0x24, 0x04, 0x89, 0x90, 0x74, 0x21, 0x2F, 0x02,
    0x8B, 0x54, 0x24, 0x08, 0x83, 0xC1, 0x01, 0x89, 0x90, 0x7C, 0x21, 0x2F, 0x02, 0x89, 0x0D, 0x74,
    0xA2, 0x52, 0x02, 0xC3,
];
const B235: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0xF8, 0xE2, 0xEB, 0x00, 0x3B, 0x01, 0x0F, 0x94, 0xC0, 0xC2,
    0x04, 0x00,
];
const B236: [u8; 41] = [
    0x85, 0xD2, 0x74, 0x22, 0x8B, 0xC2, 0x83, 0xE0, 0x3F, 0x8B, 0x0C, 0x85, 0x70, 0x26, 0x2F, 0x02,
    0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x10, 0x39, 0x50, 0x18, 0x74, 0x0D, 0x8B, 0x40, 0x08, 0x3B, 0xC1,
    0x74, 0x04, 0x85, 0xC0, 0x75, 0xF0, 0x33, 0xC0, 0xC3,
];
const B237: [u8; 43] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x3B, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x5C, 0xD8,
    0xE4, 0x00, 0x89, 0x88, 0x98, 0x00, 0x00, 0x00, 0x75, 0x10, 0xC7, 0x80, 0x98, 0x00, 0x00, 0x00,
    0x09, 0x00, 0x00, 0x00, 0x89, 0x88, 0x9C, 0x00, 0x00, 0x00, 0xC3,
];
const B238: [u8; 49] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x3B, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x5C, 0xD8,
    0xE4, 0x00, 0x89, 0x88, 0x98, 0x00, 0x00, 0x00, 0x75, 0x10, 0xC7, 0x80, 0x98, 0x00, 0x00, 0x00,
    0x09, 0x00, 0x00, 0x00, 0x89, 0x88, 0x9C, 0x00, 0x00, 0x00, 0xC7, 0x00, 0xCC, 0xD9, 0xE4, 0x00,
    0xC3,
];
const B239: [u8; 8] = [
    0xA1, 0x30, 0xE5, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B240: [u8; 8] = [
    0xA1, 0x90, 0xE7, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B241: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x4C, 0xDD, 0xE4, 0x00,
    0xC3,
];
const B242: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B243: [u8; 8] = [
    0xA1, 0xE8, 0xE7, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B244: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x34, 0xDE, 0xE4, 0x00,
    0xC3,
];
const B245: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B246: [u8; 8] = [
    0xA1, 0x50, 0xE8, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B247: [u8; 43] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x3B, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x54, 0xDF,
    0xE4, 0x00, 0x89, 0x88, 0x98, 0x00, 0x00, 0x00, 0x75, 0x10, 0xC7, 0x80, 0x98, 0x00, 0x00, 0x00,
    0x09, 0x00, 0x00, 0x00, 0x89, 0x88, 0x9C, 0x00, 0x00, 0x00, 0xC3,
];
const B248: [u8; 8] = [
    0xA1, 0x6C, 0xE9, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B249: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x74, 0xE0, 0xE4, 0x00,
    0xC3,
];
const B250: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B251: [u8; 8] = [
    0xA1, 0xD0, 0xE9, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
const B252: [u8; 17] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x74, 0xE1, 0xE4, 0x00,
    0xC3,
];
const B253: [u8; 7] = [
    0xC7, 0x01, 0x40, 0xD2, 0xE4, 0x00, 0xC3,
];
const B254: [u8; 8] = [
    0xA1, 0x18, 0xEA, 0xEB, 0x00, 0x8B, 0x00, 0xC3,
];
