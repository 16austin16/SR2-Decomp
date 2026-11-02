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
    if orig == 0 || n >= SHADOW_CALLS || (BIG[id].load(Ordering::Relaxed) && n >= BIG_SHADOW_CALLS) {
        (p.run)(r, args);
        return;
    }
    shadow(id, p, n, orig, r, args);
}

/// Ports whose writes cover more than BIG_WRITES bytes (bulk clears such as vhashtable
/// clear_buckets): recording and snapshotting them costs ~0.15 s a call, so they are shadowed for
/// the first BIG_SHADOW_CALLS calls only.
static BIG: [std::sync::atomic::AtomicBool; COUNT] = [const { std::sync::atomic::AtomicBool::new(false) }; COUNT];
const BIG_WRITES: usize = 0x1_0000;
const BIG_SHADOW_CALLS: u32 = 16;

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
fn shadow(id: usize, p: &Port, n: u32, orig: usize, r: &mut Pushad, args: *const u32) {
    let inp = Regs::of(r);
    let regions = match p.writes {
        Some(w) => w(r, args),
        None => [(0, 0); 4],
    };
    if regions.iter().map(|&(_, l)| l).sum::<usize>() > BIG_WRITES {
        BIG[id].store(true, Ordering::Relaxed);
    }
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

// --- third sweep: leaves translated from the disassembly --------------------------------------------
//
// Bodies below are the original instruction streams written out one instruction at a time: general
// registers live in the `Pushad` block, the arithmetic flags in four locals (set exactly as the CPU
// sets them for the instructions used), memory goes through `Mem`, and a body with branches is a
// small state machine over its basic blocks. `Mem` is the real memory for the run, and a recording
// overlay for the dry run that finds the bytes a call will write (for the shadow's snapshot).

trait Mem {
    fn rd(&mut self, a: usize) -> u32;
    fn rw(&mut self, a: usize) -> u32;
    fn rb(&mut self, a: usize) -> u32;
    fn wd(&mut self, a: usize, v: u32);
    fn ww(&mut self, a: usize, v: u32);
    fn wb(&mut self, a: usize, v: u32);
}

struct RealMem;

impl Mem for RealMem {
    fn rd(&mut self, a: usize) -> u32 {
        unsafe { rd(a) }
    }
    fn rw(&mut self, a: usize) -> u32 {
        unsafe { *(a as *const u16) as u32 }
    }
    fn rb(&mut self, a: usize) -> u32 {
        unsafe { rb(a) as u32 }
    }
    fn wd(&mut self, a: usize, v: u32) {
        unsafe { wd(a, v) }
    }
    fn ww(&mut self, a: usize, v: u32) {
        unsafe { ww(a, v as u16) }
    }
    fn wb(&mut self, a: usize, v: u32) {
        unsafe { wb(a, v as u8) }
    }
}

/// Dry-run memory: reads see earlier writes of the same run, nothing reaches the real memory.
struct Rec {
    over: std::collections::HashMap<usize, u8>,
}

impl Rec {
    fn get(&self, a: usize) -> u8 {
        match self.over.get(&a) {
            Some(e) => *e,
            None => unsafe { rb(a) },
        }
    }
    fn put(&mut self, a: usize, n: usize, v: u32) {
        for i in 0..n {
            self.over.insert(a.wrapping_add(i), (v >> (8 * i)) as u8);
        }
    }
    fn read(&self, a: usize, n: usize) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            v |= (self.get(a.wrapping_add(i)) as u32) << (8 * i);
        }
        v
    }
}

impl Mem for Rec {
    fn rd(&mut self, a: usize) -> u32 {
        self.read(a, 4)
    }
    fn rw(&mut self, a: usize) -> u32 {
        self.read(a, 2)
    }
    fn rb(&mut self, a: usize) -> u32 {
        self.read(a, 1)
    }
    fn wd(&mut self, a: usize, v: u32) {
        self.put(a, 4, v)
    }
    fn ww(&mut self, a: usize, v: u32) {
        self.put(a, 2, v)
    }
    fn wb(&mut self, a: usize, v: u32) {
        self.put(a, 1, v)
    }
}

/// Calls whose dry run wrote to more than four separate places (the closest were joined).
static SPAN_OVERFLOW: AtomicU32 = AtomicU32::new(0);

/// The (at most four) byte ranges a dry run wrote. Ranges less than 0x40 bytes apart are joined
/// (the gap is snapshotted and restored unchanged); more than four after that is counted in
/// `SPAN_OVERFLOW` and the closest ranges are joined too.
fn rec_writes(f: impl FnOnce(&mut Rec)) -> [(usize, usize); 4] {
    let mut r = Rec { over: std::collections::HashMap::new() };
    f(&mut r);
    let mut addrs: Vec<usize> = r.over.keys().copied().collect();
    addrs.sort_unstable();
    addrs.dedup();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for a in addrs {
        match spans.last_mut() {
            Some(s) if a - (s.0 + s.1) < 0x40 => s.1 = a + 1 - s.0,
            _ => spans.push((a, 1)),
        }
    }
    if spans.len() > 4 {
        SPAN_OVERFLOW.fetch_add(1, Ordering::Relaxed);
    }
    while spans.len() > 4 {
        let mut best = 0;
        let mut gap = usize::MAX;
        for i in 0..spans.len() - 1 {
            let g = spans[i + 1].0 - (spans[i].0 + spans[i].1);
            if g < gap {
                gap = g;
                best = i;
            }
        }
        let next = spans.remove(best + 1);
        spans[best].1 = next.0 + next.1 - spans[best].0;
    }
    let mut out = [(0, 0); 4];
    for (i, s) in spans.iter().enumerate() {
        out[i] = *s;
    }
    out
}

#[derive(Clone, Copy)]
struct Fl {
    r: u32,
    zf: bool,
    sf: bool,
    cf: bool,
    of: bool,
}

fn wmask(w: u32) -> u32 {
    if w == 32 { u32::MAX } else { (1u32 << w) - 1 }
}

fn fl_logic(r: u32, w: u32) -> Fl {
    let r = r & wmask(w);
    Fl { r, zf: r == 0, sf: ((r >> (w - 1)) & 1) != 0, cf: false, of: false }
}

fn fl_add(a: u32, b: u32, c: u32, w: u32) -> Fl {
    let mk = wmask(w);
    let (a, b) = (a & mk, b & mk);
    let s = a as u64 + b as u64 + c as u64;
    let r = (s as u32) & mk;
    Fl { r, zf: r == 0, sf: ((r >> (w - 1)) & 1) != 0, cf: s > mk as u64, of: ((((a ^ r) & (b ^ r)) >> (w - 1)) & 1) != 0 }
}

fn fl_sub(a: u32, b: u32, c: u32, w: u32) -> Fl {
    let mk = wmask(w);
    let (a, b) = (a & mk, b & mk);
    let r = a.wrapping_sub(b).wrapping_sub(c) & mk;
    Fl {
        r,
        zf: r == 0,
        sf: ((r >> (w - 1)) & 1) != 0,
        cf: (a as u64) < b as u64 + c as u64,
        of: ((((a ^ b) & (a ^ r)) >> (w - 1)) & 1) != 0,
    }
}

fn fl_shl(x: u32, n: u32, w: u32) -> Fl {
    let mk = wmask(w);
    let x = x & mk;
    let r = (x << n) & mk;
    let cf = ((x >> (w - n)) & 1) != 0;
    Fl { r, zf: r == 0, sf: ((r >> (w - 1)) & 1) != 0, cf, of: (((r >> (w - 1)) & 1) != 0) != cf }
}

fn fl_shr(x: u32, n: u32, w: u32) -> Fl {
    let mk = wmask(w);
    let x = x & mk;
    let r = x >> n;
    Fl { r, zf: r == 0, sf: ((r >> (w - 1)) & 1) != 0, cf: ((x >> (n - 1)) & 1) != 0, of: ((x >> (w - 1)) & 1) != 0 }
}

fn fl_sar(x: u32, n: u32, w: u32) -> Fl {
    let mk = wmask(w);
    let sx = ((x << (32 - w)) as i32) >> (32 - w);
    let r = ((sx >> n) as u32) & mk;
    Fl { r, zf: r == 0, sf: ((r >> (w - 1)) & 1) != 0, cf: ((sx >> (n - 1)) & 1) != 0, of: false }
}

fn fl_imul(a: u32, b: u32) -> Fl {
    let full = (a as i32 as i64) * (b as i32 as i64);
    let r = full as i32;
    let ov = full != r as i64;
    Fl { r: r as u32, zf: r == 0, sf: r < 0, cf: ov, of: ov }
}

/// Make the run / dry-run entry points for a translated read-only body.
macro_rules! leaf_ro {
    ($f:ident, $run:ident) => {
        fn $run(p: &mut Pushad, a: *const u32) {
            $f(p, a, &mut RealMem)
        }
    };
}

/// Make the run / written-bytes entry points for a translated body that writes memory.
macro_rules! leaf_rw {
    ($f:ident, $run:ident, $w:ident) => {
        fn $run(p: &mut Pushad, a: *const u32) {
            $f(p, a, &mut RealMem)
        }
        fn $w(p: &Pushad, a: *const u32) -> [(usize, usize); 4] {
            rec_writes(|m| {
                let mut q = *p;
                $f(&mut q, a, m)
            })
        }
    };
}

// --- vhashtable<>::clear_buckets, hand-written (the machine translation was ~100x too slow) ----------
// Original: eax = 0; if [ecx+4] != 0 { do { edx = [ecx+8]; elem(edx, eax) = fill; eax += 1 } while eax < [ecx+4] }.
// Count and bucket pointer are re-read every iteration, like the original.

#[inline(always)]
unsafe fn cb_rd(a: u32) -> u32 {
    unsafe { std::ptr::read_volatile(a as usize as *const u32) }
}

/// Fills 16-bit buckets with 0xFFFF; ends with eax = index, edx = bucket pointer (if it looped).
unsafe fn cb_fill16(p: &mut Pushad) {
    unsafe {
        let c = p.ecx;
        let mut i = 0u32;
        if cb_rd(c.wrapping_add(4)) != 0 {
            let mut d;
            loop {
                d = cb_rd(c.wrapping_add(8));
                std::ptr::write_volatile(d.wrapping_add(i.wrapping_mul(2)) as usize as *mut u16, 0xFFFF);
                i = i.wrapping_add(1);
                if i >= cb_rd(c.wrapping_add(4)) {
                    break;
                }
            }
            p.edx = d;
        }
        p.eax = i;
    }
}

/// 32-bit buckets filled with `v`.
unsafe fn cb_fill32(p: &mut Pushad, v: u32) {
    unsafe {
        let c = p.ecx;
        let mut i = 0u32;
        if cb_rd(c.wrapping_add(4)) != 0 {
            let mut d;
            loop {
                d = cb_rd(c.wrapping_add(8));
                std::ptr::write_volatile(d.wrapping_add(i.wrapping_mul(4)) as usize as *mut u32, v);
                i = i.wrapping_add(1);
                if i >= cb_rd(c.wrapping_add(4)) {
                    break;
                }
            }
            p.edx = d;
        }
        p.eax = i;
    }
}

/// 8-byte buckets filled from the global pair at 0xEB80F0 / 0xEB80F4 (read every iteration);
/// esi is saved and restored by the original.
unsafe fn cb_fill64(p: &mut Pushad) {
    unsafe {
        let c = p.ecx;
        let mut i = 0u32;
        if cb_rd(c.wrapping_add(4)) != 0 {
            let mut d;
            loop {
                let lo = cb_rd(0x00EB_80F0);
                d = cb_rd(c.wrapping_add(8));
                std::ptr::write_volatile(d.wrapping_add(i.wrapping_mul(8)) as usize as *mut u32, lo);
                let hi = cb_rd(0x00EB_80F4);
                std::ptr::write_volatile(d.wrapping_add(i.wrapping_mul(8)).wrapping_add(4) as usize as *mut u32, hi);
                i = i.wrapping_add(1);
                if i >= cb_rd(c.wrapping_add(4)) {
                    break;
                }
            }
            p.edx = d;
        }
        p.eax = i;
    }
}

/// The bytes a clear_buckets call writes: count buckets of `elem` bytes at [ecx+8].
fn cb_writes(p: &Pushad, elem: usize) -> [(usize, usize); 4] {
    unsafe {
        let n = cb_rd(p.ecx.wrapping_add(4)) as usize;
        let d = cb_rd(p.ecx.wrapping_add(8)) as usize;
        [(d, n * elem), (0, 0), (0, 0), (0, 0)]
    }
}

fn r_cb16(p: &mut Pushad, _a: *const u32) {
    unsafe { cb_fill16(p) }
}
fn r_cb32_0(p: &mut Pushad, _a: *const u32) {
    unsafe { cb_fill32(p, 0) }
}
fn r_cb32_ffff(p: &mut Pushad, _a: *const u32) {
    unsafe { cb_fill32(p, 0xFFFF) }
}
fn r_cb64(p: &mut Pushad, _a: *const u32) {
    unsafe { cb_fill64(p) }
}
fn w_cb2(p: &Pushad, _a: *const u32) -> [(usize, usize); 4] {
    cb_writes(p, 2)
}
fn w_cb4(p: &Pushad, _a: *const u32) -> [(usize, usize); 4] {
    cb_writes(p, 4)
}
fn w_cb8(p: &Pushad, _a: *const u32) -> [(usize, usize); 4] {
    cb_writes(p, 8)
}

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4660b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((p.ecx) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0xFFu32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.ecx = m.rd((p.ecx) as usize);
                let x = p.ecx; let t = fl_shr(x, 8, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.eax & 0xFF);
                let t = fl_imul(p.eax, 0x19Cu32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.ecx; let y = m.rd((p.eax.wrapping_add(0xEEB9AC)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0xEEB8F8)) as usize); let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            5 => {
                p.eax = (p.eax & 0xFFFF0000) | ((m.rw((p.eax.wrapping_add(0xEEBA8E)) as usize)) & 0xFFFF);
                let x = (p.eax & 0xFFFF); let t = fl_shr(x, 11, 16);
                p.eax = (p.eax & 0xFFFF0000) | ((t.r) & 0xFFFF);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | ((!((p.eax & 0xFF))) & 0xFF);
                let r = ((p.eax & 0xFF)) & (0x1u32);
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_4660b0, r_4660b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_493860(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = m.rb((0xE8825Fu32) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                return; // unreachable
            }
            3 => {
                p.ecx = m.rd((p.eax.wrapping_mul(4).wrapping_add(0xF53914)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = p.ecx; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = p.eax; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x3u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf != of) { pc = 3; continue; }
                pc = 6; continue;
            }
            6 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            7 => {
                let r = (p.esi) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                m.wd((p.esi) as usize, p.eax);
                pc = 9; continue;
            }
            9 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_493860, r_493860, w_493860);


#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4b26e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = 0x77359400u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (zf || sf != of) { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx) as usize, 0x77359400u32);
                return;
            }
            2 => {
                let x = p.eax; let y = 0x88CA6C00u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                m.wd((p.ecx) as usize, 0x88CA6C00u32);
                return;
            }
            4 => {
                m.wd((p.ecx) as usize, p.eax);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_4b26e0, r_4b26e0, w_4b26e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4ff4b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0xF9EA38u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x8)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xC)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.ecx = m.rd((p.ecx.wrapping_add(0x10)) as usize);
                m.wd((p.eax.wrapping_add(0xC)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_4ff4b0, r_4ff4b0, w_4ff4b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4ff4e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if (zf || sf != of) { pc = 5; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.ecx; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                pc = 3; continue;
            }
            2 => {
                return; // unreachable
            }
            3 => {
                p.eax = m.rd((p.ecx.wrapping_add(0xFFFFFFF8)) as usize);
                p.esi = m.rd((p.ecx.wrapping_add(0xFFFFFFFC)) as usize);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0xF9EA38u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax) as usize, p.esi);
                p.esi = m.rd((p.ecx) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.esi);
                p.esi = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.esi);
                p.esi = m.rd((p.ecx.wrapping_add(0x8)) as usize);
                let x = p.ecx; let y = 0x14u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax.wrapping_add(0xC)) as usize, p.esi);
                if !zf { pc = 3; continue; }
                pc = 4; continue;
            }
            4 => {
                p.esi = stk[0];
                pc = 5; continue;
            }
            5 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_4ff4e0, r_4ff4e0, w_4ff4e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4ff520(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0xF9F278u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax) as usize, p.edx);
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((p.ecx.wrapping_add(0x8)) as usize)) & 0xFF);
                m.wb((p.eax.wrapping_add(0x4)) as usize, (p.edx & 0xFF));
                p.edx = m.rd((p.ecx.wrapping_add(0xC)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((p.ecx.wrapping_add(0x10)) as usize)) & 0xFF);
                m.wb((p.eax.wrapping_add(0xC)) as usize, (p.ecx & 0xFF));
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_4ff520, r_4ff520, w_4ff520);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_4ff550(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.esi;
                p.esi = p.eax;
                let r = (p.esi) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if (zf || sf != of) { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.ecx; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 2; continue;
            }
            2 => {
                p.eax = m.rd((p.ecx.wrapping_add(0xFFFFFFF8)) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(0xFFFFFFFC)) as usize);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0xF9F278u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rb((p.ecx) as usize);
                m.wb((p.eax.wrapping_add(0x4)) as usize, (p.edx & 0xFF));
                p.edx = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.edx = m.rb((p.ecx.wrapping_add(0x8)) as usize);
                let x = p.ecx; let y = 0x14u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.esi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wb((p.eax.wrapping_add(0xC)) as usize, (p.edx & 0xFF));
                if !zf { pc = 2; continue; }
                pc = 3; continue;
            }
            3 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_4ff550, r_4ff550, w_4ff550);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_558d90(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.eax.wrapping_add(0xC4)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x8)) as usize);
                p.eax = m.rd((p.ecx.wrapping_add(0x3C)) as usize);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_558d90, r_558d90);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_574c70(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                m.wd((p.eax) as usize, 0xDE874Cu32);
                m.wd((p.eax.wrapping_add(0x18)) as usize, 0xFFFFFFFFu32);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_574c70, r_574c70, w_574c70);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_621890(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B48u32) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.edx & 0xFF)) & ((p.edx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = unsafe { *a.add(0) };
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((p.eax.wrapping_add(0xFA8)) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0xFFu32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                p.edx = (((p.eax & 0xFF)) as u8 as i8 as i32 as u32);
                let x = m.rd((p.ecx.wrapping_add(p.edx.wrapping_mul(4)).wrapping_add(0x1C)) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.ecx.wrapping_add(p.edx.wrapping_mul(4)).wrapping_add(0x1C)) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.ecx.wrapping_add(p.edx.wrapping_mul(4)).wrapping_add(0x1C));
                pc = 4; continue;
            }
            4 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_621890, r_621890, w_621890);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_6417e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                m.wd((p.eax) as usize, 0xDFAB94u32);
                let r = (p.ecx) | (0xFFFFFFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x30)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x34)) as usize, p.ecx);
                let r = (p.edx) ^ (p.edx);
                p.edx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x3C)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0x40)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0x44)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0x48)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0x4C)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0x50)) as usize, p.edx);
                m.wb((p.eax.wrapping_add(0x64)) as usize, 0x1u32);
                m.wd((p.eax.wrapping_add(0x68)) as usize, p.ecx);
                p.edx = m.rd((0x33D630Cu32) as usize);
                m.wd((p.eax.wrapping_add(0x88)) as usize, p.edx);
                p.edx = m.rd((0x33D630Cu32) as usize);
                m.wd((p.eax.wrapping_add(0x90)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0xD0)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xD4)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xD8)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xDC)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_6417e0, r_6417e0, w_6417e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_6dfc60(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.ebx;
                p.ebx = unsafe { *a.add(0) };
                stk[1] = p.esi;
                p.esi = p.eax;
                p.edx = m.rd((p.ecx.wrapping_add(p.esi.wrapping_mul(4)).wrapping_add(0x3808)) as usize);
                stk[2] = p.edi;
                p.eax = p.edx;
                let r = (p.edi) ^ (p.edi);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = m.rd((p.eax) as usize); let y = p.ebx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 6; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = m.rd((p.eax.wrapping_add(0x4)) as usize);
                let x = p.eax; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.eax; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 1; continue; }
                pc = 4; continue;
            }
            4 => {
                pc = 6; continue;
            }
            5 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 6; continue;
            }
            6 => {
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 7; continue;
            }
            7 => {
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edi);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edi);
                m.wd((p.ecx.wrapping_add(p.esi.wrapping_mul(4)).wrapping_add(0x3808)) as usize, p.edi);
                pc = 11; continue;
            }
            9 => {
                m.wd((p.ecx.wrapping_add(p.esi.wrapping_mul(4)).wrapping_add(0x3808)) as usize, p.edx);
                pc = 10; continue;
            }
            10 => {
                p.edx = m.rd((p.eax.wrapping_add(0x8)) as usize);
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.esi);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.esi = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.edx.wrapping_add(0x8)) as usize, p.esi);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edi);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edi);
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0xFFFFFFFFu32);
                p.edx = m.rd((p.ecx.wrapping_add(0x3888)) as usize);
                let x = p.edx; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edi = stk[2];
                p.esi = stk[1];
                p.ebx = stk[0];
                if !zf { pc = 13; continue; }
                pc = 12; continue;
            }
            12 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.eax);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.eax);
                m.wd((p.ecx.wrapping_add(0x3888)) as usize, p.eax);
                return;
            }
            13 => {
                p.edx = m.rd((p.edx.wrapping_add(0x8)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x3888)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x3888)) as usize);
                p.edx = m.rd((p.edx.wrapping_add(0x8)) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.eax);
                p.ecx = m.rd((p.ecx.wrapping_add(0x3888)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_6dfc60, r_6dfc60, w_6dfc60);



#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_7683b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x1F7A434u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.ecx);
                m.wd((0x1F7A434u32) as usize, p.ecx);
                return;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.edx);
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.eax);
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.edx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_7683b0, r_7683b0, w_7683b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_7686b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x1F7A430u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.ecx);
                m.wd((0x1F7A430u32) as usize, p.ecx);
                return;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.edx);
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.eax);
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.edx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_7686b0, r_7686b0, w_7686b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_768b70(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x1F7A42Cu32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.ecx);
                m.wd((0x1F7A42Cu32) as usize, p.ecx);
                return;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.edx);
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.eax);
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.edx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_768b70, r_768b70, w_768b70);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_769470(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x1F7A424u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.ecx);
                m.wd((0x1F7A424u32) as usize, p.ecx);
                return;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.edx);
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.eax);
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.edx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_769470, r_769470, w_769470);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_76a800(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x1F7A41Cu32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.ecx);
                m.wd((0x1F7A41Cu32) as usize, p.ecx);
                return;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.edx);
                m.wd((p.ecx.wrapping_add(0x18)) as usize, p.eax);
                p.edx = m.rd((p.eax.wrapping_add(0x1C)) as usize);
                m.wd((p.edx.wrapping_add(0x18)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_76a800, r_76a800, w_76a800);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_773e00(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.ebx;
                p.eax = (p.eax.wrapping_add(p.eax.wrapping_mul(2)));
                p.ecx = m.rw((p.eax.wrapping_mul(4).wrapping_add(0xE8DEA0)) as usize);
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((p.eax.wrapping_mul(4).wrapping_add(0xE8DE9C)) as usize)) & 0xFF);
                p.eax = (p.eax.wrapping_mul(4).wrapping_add(0xE8DE9C));
                stk[1] = p.esi;
                p.esi = m.rd((0x1F7A9B0u32) as usize);
                let r = (p.ecx) ^ (p.esi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) & (0xF000u32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.esi) ^ (p.ecx);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((0x1F7A9ADu32) as usize)) & 0xFF);
                p.ebx = (p.ebx & 0xFFFFFF00) | (((p.edx & 0xFF)) & 0xFF);
                let x = (p.ebx & 0xFF); let y = (p.ebx & 0xFF); let t = fl_add(x, y, 0, 8);
                p.ebx = (p.ebx & 0xFFFFFF00) | ((t.r) & 0xFF);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = ((p.ebx & 0xFF)) ^ ((p.ecx & 0xFF));
                p.ebx = (p.ebx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = ((p.ebx & 0xFF)) & (0x2u32);
                p.ebx = (p.ebx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = ((p.ecx & 0xFF)) ^ ((p.ebx & 0xFF));
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebx = (p.ebx & 0xFFFFFF00) | ((m.rb((p.eax.wrapping_add(0x2)) as usize)) & 0xFF);
                m.wb((0x1F7A9ADu32) as usize, (p.ecx & 0xFF));
                p.ecx = m.rb((p.eax.wrapping_add(0x1)) as usize);
                stk[2] = p.edi;
                m.wb((0x1F7A9AEu32) as usize, (p.ecx & 0xFF));
                p.ecx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.edi = p.ecx;
                let x = p.edi; let t = fl_shl(x, 20, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edi; let t = fl_sar(x, 20, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.edi) ^ (p.esi);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 28, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.edi) & (0xF00u32);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.esi) ^ (p.edi);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_sar(x, 28, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.ecx) ^ (p.esi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) & (0xFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.esi) ^ (p.ecx);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ecx = m.rd((p.eax.wrapping_add(0x8)) as usize);
                p.eax = m.rb((p.eax.wrapping_add(0x4)) as usize);
                let r = (p.eax) ^ (p.esi);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.eax) & (0xF0u32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.esi) ^ (p.eax);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x1F7A9ADu32) as usize)) & 0xFF);
                let x = (p.edx & 0xFF); let y = (p.edx & 0xFF); let t = fl_add(x, y, 0, 8);
                p.edx = (p.edx & 0xFFFFFF00) | ((t.r) & 0xFF);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = ((p.edx & 0xFF)) ^ ((p.eax & 0xFF));
                p.edx = (p.edx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.edi = stk[2];
                let r = ((p.edx & 0xFF)) & (0xCu32);
                p.edx = (p.edx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((0x1F7A9B0u32) as usize, p.esi);
                let r = ((p.eax & 0xFF)) ^ ((p.edx & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.esi = stk[1];
                m.wb((0x1F7A9AFu32) as usize, (p.ebx & 0xFF));
                m.wd((0x1F7A9B4u32) as usize, p.ecx);
                m.wb((0x1F7A9ADu32) as usize, (p.eax & 0xFF));
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_773e00, r_773e00, w_773e00);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_77dba0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0xE96B7Cu32) as usize);
                let x = p.eax; let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (zf || sf != of) { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.eax.wrapping_mul(4).wrapping_add(0x144E760)) as usize);
                let x = p.eax; let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 11; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = p.eax; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 11; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.ecx; let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 11; continue; }
                pc = 4; continue;
            }
            4 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B48u32) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.edx & 0xFF)) & ((p.edx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            7 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B48u32) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 8; continue;
            }
            8 => {
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let x = (p.edx & 0xFF); let y = (p.eax & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 9; continue;
            }
            9 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            10 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            11 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_77dba0, r_77dba0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_784600(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = m.rb((p.ecx.wrapping_add(0x204)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = m.rd((p.ecx.wrapping_add(0x200)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            3 => {
                p.eax = 0x1u32;
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_784600, r_784600);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_78e400(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = p.eax; let y = 0x4u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 15; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let t = fl_shl(x, 6, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x29E3F18u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = p.eax;
                let x = m.rb((p.edx.wrapping_add(0xD)) as usize); let y = (p.ecx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.ebx;
                if !zf { pc = 11; continue; }
                pc = 2; continue;
            }
            2 => {
                let r = ((p.ecx & 0xFF)) & ((p.ecx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebx = (p.ebx & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                let x = (p.ecx & 0xFF); let y = (p.ebx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wb((p.edx.wrapping_add(0xD)) as usize, (p.eax & 0xFF));
                m.wb((p.edx.wrapping_add(0x1C)) as usize, (p.ebx & 0xFF));
                if !zf { pc = 10; continue; }
                pc = 3; continue;
            }
            3 => {
                let r = ((p.ecx & 0xFF)) ^ ((p.ecx & 0xFF));
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 4; continue;
            }
            4 => {
                let x = m.rb((p.eax.wrapping_add(0x29E3F25)) as usize); let y = (p.ebx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 9; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rb((p.eax.wrapping_add(0x29E3F24)) as usize); let y = (p.ebx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = (p.ecx & 0xFFFFFF00) | (((p.ebx & 0xFF)) & 0xFF);
                pc = 7; continue;
            }
            7 => {
                let x = p.eax; let y = 0x40u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x100u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if cf { pc = 4; continue; }
                pc = 8; continue;
            }
            8 => {
                let x = (p.ecx & 0xFF); let y = (p.ebx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = (p.ecx & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                m.wb((0x252849Cu32) as usize, (p.ecx & 0xFF));
                pc = 11; continue;
            }
            9 => {
                m.wb((0x252849Cu32) as usize, 0x0u32);
                p.ebx = stk[0];
                return;
            }
            10 => {
                m.wb((0x252849Cu32) as usize, 0x0u32);
                pc = 11; continue;
            }
            11 => {
                let x = m.rb((p.edx.wrapping_add(0xC)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 14; continue; }
                pc = 12; continue;
            }
            12 => {
                let x = m.rd((0x29E3EF0u32) as usize); let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 14; continue; }
                pc = 13; continue;
            }
            13 => {
                m.wd((0x29E3EF0u32) as usize, 0x4u32);
                pc = 14; continue;
            }
            14 => {
                p.ebx = stk[0];
                pc = 15; continue;
            }
            15 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_78e400, r_78e400, w_78e400);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_827ce0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0xEu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                p.eax = 0x2u32;
                return;
            }
            5 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_827ce0, r_827ce0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_8309f0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.esi;
                p.esi = p.eax;
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rb((0xEB2231u32) as usize); let y = (p.eax & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (m.rb((p.esi.wrapping_add(0x9C)) as usize)) & (0x2u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                stk[1] = p.edi;
                p.ecx = 0x2Cu32;
                p.edi = 0xEB2230u32;
                while p.ecx != 0 { let v = m.rd(p.esi as usize); m.wd(p.edi as usize, v); p.esi = p.esi.wrapping_add(4); p.edi = p.edi.wrapping_add(4); p.ecx = p.ecx.wrapping_sub(1); }
                m.wb((0xEB2231u32) as usize, (p.eax & 0xFF));
                m.wb((0xEB2232u32) as usize, (p.eax & 0xFF));
                m.wb((0xEB2230u32) as usize, 0x1u32);
                p.edi = stk[1];
                pc = 3; continue;
            }
            3 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_8309f0, r_8309f0, w_8309f0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_8429f0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.ebx;
                stk[1] = p.esi;
                p.esi = m.rd((0xE8B210u32) as usize);
                let r = ((p.edx & 0xFF)) ^ ((p.edx & 0xFF));
                p.edx = (p.edx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.esi; let y = 0xDu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = 0x9u32;
                p.eax = 0xE8B21Cu32;
                p.ebx = (p.ebx & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.ecx = 0x6u32;
                p.eax = 0xE8B240u32;
                p.edx = (p.edx & 0xFFFFFF00) | (((p.ebx & 0xFF)) & 0xFF);
                pc = 2; continue;
            }
            2 => {
                let x = p.edi; let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 10; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.esi; let y = 0xDu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(p.edi.wrapping_mul(4))) as usize);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.eax;
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = p.ecx;
                pc = 5; continue;
            }
            5 => {
                let r = ((p.edx & 0xFF)) & ((p.edx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 8; continue; }
                pc = 6; continue;
            }
            6 => {
                let x = (p.eax & 0xFF); let y = m.rb((0x2A4D14Au32) as usize); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 10; continue; }
                pc = 7; continue;
            }
            7 => {
                p.esi = stk[1];
                m.wb((0x2A4D12Eu32) as usize, (p.ebx & 0xFF));
                m.wb((0x2A4D12Cu32) as usize, (p.ebx & 0xFF));
                m.wb((0x2A4D14Au32) as usize, (p.eax & 0xFF));
                m.ww((0x2A4D148u32) as usize, 0x0u32);
                p.ebx = stk[0];
                return;
            }
            8 => {
                let x = (p.eax & 0xFFFF); let y = m.rw((0x2A4D148u32) as usize); let t = fl_sub(x, y, 0, 16);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 10; continue; }
                pc = 9; continue;
            }
            9 => {
                m.ww((0x2A4D148u32) as usize, (p.eax & 0xFFFF));
                m.wb((0x2A4D14Au32) as usize, 0x0u32);
                m.wb((0x2A4D12Eu32) as usize, (p.ebx & 0xFF));
                m.wb((0x2A4D12Cu32) as usize, (p.ebx & 0xFF));
                pc = 10; continue;
            }
            10 => {
                p.esi = stk[1];
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_8429f0, r_8429f0, w_8429f0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_842a80(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = ((p.ecx & 0xFF)) ^ ((p.ecx & 0xFF));
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rd((0xE8B210u32) as usize); let y = 0xDu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.ebx;
                p.eax = 0xE8B258u32;
                p.ebx = (p.ebx & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = 0xE8B270u32;
                p.ecx = (p.ecx & 0xFFFFFF00) | (((p.ebx & 0xFF)) & 0xFF);
                pc = 2; continue;
            }
            2 => {
                let x = p.edx; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 8; continue; }
                pc = 3; continue;
            }
            3 => {
                let r = ((p.ecx & 0xFF)) & ((p.ecx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.eax = m.rd((p.eax.wrapping_add(p.edx.wrapping_mul(4))) as usize);
                if zf { pc = 6; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = p.eax; let y = m.rd((0x2A4D144u32) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 8; continue; }
                pc = 5; continue;
            }
            5 => {
                m.wb((0x2A4D12Eu32) as usize, (p.ebx & 0xFF));
                m.wb((0x2A4D12Cu32) as usize, (p.ebx & 0xFF));
                m.wd((0x2A4D144u32) as usize, p.eax);
                p.ebx = stk[0];
                return;
            }
            6 => {
                let x = p.eax; let y = m.rd((0x2A4D140u32) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                m.wd((0x2A4D140u32) as usize, p.eax);
                m.wb((0x2A4D12Eu32) as usize, (p.ebx & 0xFF));
                m.wb((0x2A4D12Cu32) as usize, (p.ebx & 0xFF));
                pc = 8; continue;
            }
            8 => {
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_842a80, r_842a80, w_842a80);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_845470(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                p.ecx = unsafe { *a.add(1) };
                p.eax = m.rb((p.ecx.wrapping_add(0x1B0)) as usize);
                pc = 3; continue;
            }
            3 => {
                p.edx = unsafe { *a.add(0) };
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.edx;
                let r = (p.ecx) & (0xFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = m.rd((p.ecx.wrapping_add(0x2149C64)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.ecx.wrapping_add(0x44)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0xB0)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 9; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x6Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.eax; let t = fl_shl(x, 7, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = (unsafe { *a.add(3) } & 0xFFFF);
                p.ecx = 0xE8B1F8u32;
                let x = p.eax; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0x4u32;
                pc = 10; continue;
            }
            10 => {
                let r = (m.rd((p.ecx) as usize)) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0x0u32);
                pc = 13; continue;
            }
            12 => {
                let x = m.rd((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 13; continue;
            }
            13 => {
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 14; continue;
            }
            14 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_845470, r_845470, w_845470);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_845830(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 5; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 5; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 5; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x80)) as usize);
                p.eax = unsafe { *a.add(2) };
                p.edx = m.rw((p.eax) as usize);
                let x = p.ecx; let t = fl_shr(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = unsafe { *a.add(1) };
                let r = ((p.ecx & 0xFF)) & (0x1u32);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wb((p.edx.wrapping_add(p.esi)) as usize, (p.ecx & 0xFF));
                let x = m.rw((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 16);
                m.ww((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.esi = stk[0];
                pc = 5; continue;
            }
            5 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_845830, r_845830, w_845830);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_84e000(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0x3u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 5; continue;
            }
            5 => {
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((p.eax.wrapping_add(0xCC)) as usize)) & 0xFF);
                let r = ((p.ecx & 0xFF)) & (0x10u32);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = (p.ecx & 0xFF); let y = 0x10u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((!zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_84e000, r_84e000);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_84e120(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 3; continue;
            }
            3 => {
                p.ecx = 0x3u32;
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = m.rd((p.eax.wrapping_add(0xD0)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x80)) as usize);
                p.eax = unsafe { *a.add(2) };
                p.edx = m.rw((p.eax) as usize);
                let x = p.ecx; let t = fl_shr(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = unsafe { *a.add(1) };
                let r = ((p.ecx & 0xFF)) & (0x1u32);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wb((p.edx.wrapping_add(p.esi)) as usize, (p.ecx & 0xFF));
                let x = m.rw((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 16);
                m.ww((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.esi = stk[0];
                pc = 6; continue;
            }
            6 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_84e120, r_84e120, w_84e120);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_857990(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                p.ecx = unsafe { *a.add(1) };
                p.eax = m.rb((p.ecx.wrapping_add(0x1B0)) as usize);
                pc = 3; continue;
            }
            3 => {
                p.edx = unsafe { *a.add(0) };
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.edx;
                let r = (p.ecx) & (0xFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = m.rd((p.ecx.wrapping_add(0x2149C64)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.ecx.wrapping_add(0x44)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0xB0)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 9; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x6Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.eax; let t = fl_shl(x, 7, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = (unsafe { *a.add(3) } & 0xFFFF);
                p.ecx = 0xE8B19Cu32;
                let x = p.eax; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0xAu32;
                pc = 10; continue;
            }
            10 => {
                let r = (m.rd((p.ecx) as usize)) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0x0u32);
                pc = 13; continue;
            }
            12 => {
                let x = m.rd((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 13; continue;
            }
            13 => {
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 14; continue;
            }
            14 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_857990, r_857990, w_857990);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_85e1b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                p.ecx = unsafe { *a.add(1) };
                p.eax = m.rb((p.ecx.wrapping_add(0x1B0)) as usize);
                pc = 3; continue;
            }
            3 => {
                p.edx = unsafe { *a.add(0) };
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.edx;
                let r = (p.ecx) & (0xFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = m.rd((p.ecx.wrapping_add(0x2149C64)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.ecx.wrapping_add(0x44)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0xB0)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 9; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x6Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.eax; let t = fl_shl(x, 7, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = (unsafe { *a.add(3) } & 0xFFFF);
                p.ecx = 0xE8B164u32;
                let x = p.eax; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0x4u32;
                pc = 10; continue;
            }
            10 => {
                let r = (m.rd((p.ecx) as usize)) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0x0u32);
                pc = 13; continue;
            }
            12 => {
                let x = m.rd((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 13; continue;
            }
            13 => {
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 14; continue;
            }
            14 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_85e1b0, r_85e1b0, w_85e1b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_8699b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                p.ecx = unsafe { *a.add(1) };
                p.eax = m.rb((p.ecx.wrapping_add(0x1B0)) as usize);
                pc = 3; continue;
            }
            3 => {
                p.edx = unsafe { *a.add(0) };
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.edx;
                let r = (p.ecx) & (0xFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = m.rd((p.ecx.wrapping_add(0x2149C64)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.ecx.wrapping_add(0x44)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0xB0)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 9; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x6Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.eax; let t = fl_shl(x, 7, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = (unsafe { *a.add(3) } & 0xFFFF);
                p.ecx = 0xE8B128u32;
                let x = p.eax; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0xBu32;
                pc = 10; continue;
            }
            10 => {
                let r = (m.rd((p.ecx) as usize)) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0x0u32);
                pc = 13; continue;
            }
            12 => {
                let x = m.rd((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 13; continue;
            }
            13 => {
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 14; continue;
            }
            14 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_8699b0, r_8699b0, w_8699b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_869a90(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 3; continue;
            }
            3 => {
                p.ecx = 0x1u32;
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = m.rb((p.eax.wrapping_add(0x3CC)) as usize); let y = (p.ecx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x1360)) as usize);
                m.wd((p.ecx) as usize, 0x0u32);
                pc = 6; continue;
            }
            6 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_869a90, r_869a90, w_869a90);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_86d430(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            2 => {
                p.ecx = unsafe { *a.add(1) };
                p.eax = m.rb((p.ecx.wrapping_add(0x1B0)) as usize);
                pc = 3; continue;
            }
            3 => {
                p.edx = unsafe { *a.add(0) };
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = p.edx;
                let r = (p.ecx) & (0xFFFu32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let t = fl_shl(x, 4, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = m.rd((p.ecx.wrapping_add(0x2149C64)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.ecx.wrapping_add(0x44)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0xB0)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 9; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x6Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.eax; let t = fl_shl(x, 7, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.ecx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                p.esi = (unsafe { *a.add(3) } & 0xFFFF);
                p.ecx = 0xE8B0ECu32;
                let x = p.eax; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0x3u32;
                pc = 10; continue;
            }
            10 => {
                let r = (m.rd((p.ecx) as usize)) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                m.wd((p.eax) as usize, 0x0u32);
                pc = 13; continue;
            }
            12 => {
                let x = m.rd((p.eax) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.eax) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 13; continue;
            }
            13 => {
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x8u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 14; continue;
            }
            14 => {
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_86d430, r_86d430, w_86d430);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_86d4f0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x2528C14u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 5; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = m.rb((0x2528B56u32) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rb((0x2528B2Bu32) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rb((p.eax.wrapping_add(0x3F8)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            5 => {
                p.eax = 0x1u32;
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_86d4f0, r_86d4f0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_86d530(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = unsafe { *a.add(0) };
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0xCu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 5; continue;
            }
            5 => {
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((p.eax.wrapping_add(0x120)) as usize)) & 0xFF);
                let r = ((p.ecx & 0xFF)) & (0x1u32);
                p.ecx = (p.ecx & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = (p.ecx & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((!zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_86d530, r_86d530);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_8742c0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0xEB3958u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ecx = m.rd((0xE843ACu32) as usize);
                if (sf != of) { pc = 5; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (zf || sf != of) { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                let x = p.eax; let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x35A4E900u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf != of) { pc = 7; continue; }
                pc = 3; continue;
            }
            3 => {
                pc = 5; continue;
            }
            4 => {
                p.edx = p.ecx;
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x35A4E900u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                p.eax = (p.ecx.wrapping_add(0x1F4));
                let x = p.eax; let y = 0x6B49D200u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((0xEB3958u32) as usize, p.eax);
                if (zf || sf != of) { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                let x = p.eax; let y = 0x6B49D200u32; let t = fl_sub(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((0xEB3958u32) as usize, p.eax);
                pc = 7; continue;
            }
            7 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_8742c0, r_8742c0, w_8742c0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_9195d0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0xBC)) as usize);
                p.eax = unsafe { *a.add(0) };
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xC0)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xC4)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xC8)) as usize);
                p.eax = unsafe { *a.add(1) };
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xCC)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.ecx = m.rd((p.ecx.wrapping_add(0xD0)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_9195d0, r_9195d0, w_9195d0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_9335c0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx.wrapping_add(0xC4)) as usize);
                p.edx = m.rd((p.eax.wrapping_add(0x44)) as usize);
                p.eax = m.rd((p.edx.wrapping_add(0x30)) as usize);
                p.edx = unsafe { *a.add(0) };
                stk[0] = p.esi;
                p.esi = m.rd((p.eax) as usize);
                m.wd((p.edx) as usize, p.esi);
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.esi);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.edx.wrapping_add(0x8)) as usize, p.eax);
                p.ecx = m.rd((p.ecx.wrapping_add(0xC4)) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(0x44)) as usize);
                p.eax = m.rd((p.edx.wrapping_add(0x30)) as usize);
                p.edx = m.rd((p.eax.wrapping_add(0xC)) as usize);
                p.ecx = unsafe { *a.add(1) };
                let x = p.eax; let y = 0xCu32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_9335c0, r_9335c0, w_9335c0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_934a70(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (m.rb((p.ecx.wrapping_add(0x120)) as usize)) & (0x10u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.ecx.wrapping_add(0xE8)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = m.rd((p.eax.wrapping_add(0x44)) as usize);
                p.eax = m.rd((p.eax.wrapping_add(0x30)) as usize);
                p.edx = unsafe { *a.add(0) };
                stk[0] = p.esi;
                p.esi = m.rd((p.eax) as usize);
                m.wd((p.edx) as usize, p.esi);
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.esi);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.edx.wrapping_add(0x8)) as usize, p.eax);
                p.ecx = m.rd((p.ecx.wrapping_add(0xE8)) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(0x44)) as usize);
                p.eax = m.rd((p.edx.wrapping_add(0x30)) as usize);
                p.edx = m.rd((p.eax.wrapping_add(0xC)) as usize);
                p.ecx = unsafe { *a.add(1) };
                let x = p.eax; let y = 0xCu32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                p.esi = stk[0];
                pc = 3; continue;
            }
            3 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_934a70, r_934a70, w_934a70);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_960200(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0xDC)) as usize);
                p.eax = unsafe { *a.add(0) };
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xE0)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xE4)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xE8)) as usize);
                p.eax = unsafe { *a.add(1) };
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0xEC)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.ecx = m.rd((p.ecx.wrapping_add(0xF0)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_960200, r_960200, w_960200);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_98e750(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                p.ecx = m.rd((p.edx.wrapping_add(0x2A8)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 3; continue;
            }
            3 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                let r = (m.rb((p.eax.wrapping_add(0x80)) as usize)) & (0xBu32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 13; continue; }
                pc = 7; continue;
            }
            7 => {
                p.ecx = m.rd((p.edx.wrapping_add(0x2A4)) as usize);
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 8; continue;
            }
            8 => {
                p.eax = p.ecx;
                let r = (p.eax) & (0xFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = m.rd((p.eax.wrapping_add(0x2149C64)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 9; continue;
            }
            9 => {
                let x = m.rd((p.eax.wrapping_add(0x44)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 12; continue; }
                pc = 10; continue;
            }
            10 => {
                let x = m.rd((p.eax.wrapping_add(0x4C)) as usize); let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 12; continue; }
                pc = 11; continue;
            }
            11 => {
                let r = (m.rb((p.eax.wrapping_add(0x80)) as usize)) & (0xBu32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 13; continue; }
                pc = 12; continue;
            }
            12 => {
                p.eax = p.edx;
                pc = 13; continue;
            }
            13 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_98e750, r_98e750);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_9c6060(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.eax.wrapping_add(0x10CC)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = m.rd((p.eax.wrapping_add(0xC4)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 3; continue;
            }
            3 => {
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = m.rd((p.eax.wrapping_add(0x34)) as usize); let y = 0x5u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                let r = ((p.eax & 0xFF)) & ((p.eax & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                p.eax = 0x1u32;
                return;
            }
            6 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_9c6060, r_9c6060);



#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_a57b30(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                m.wd((p.eax) as usize, 0xE3E3BCu32);
                p.ecx = m.rd((0x33D6308u32) as usize);
                m.wd((p.eax.wrapping_add(0xC)) as usize, p.ecx);
                p.edx = m.rd((0x33D6308u32) as usize);
                m.wd((p.eax.wrapping_add(0xC)) as usize, p.edx);
                let r = (m.rb((p.eax.wrapping_add(0x18)) as usize)) & (0xFCu32);
                m.wb((p.eax.wrapping_add(0x18)) as usize, r);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x10)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.ecx);
                m.wb((p.eax.wrapping_add(0x14)) as usize, (p.ecx & 0xFF));
                m.wb((p.eax.wrapping_add(0x15)) as usize, 0xFFu32);
                m.wd((p.eax) as usize, 0xE42FB8u32);
                m.wd((p.eax.wrapping_add(0x4)) as usize, 0xA62960u32);
                m.wd((p.eax.wrapping_add(0x20)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x24)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x28)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_a57b30, r_a57b30, w_a57b30);


#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_a63700(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = p.ecx;
                p.ecx = unsafe { *a.add(0) };
                m.wd((p.eax.wrapping_add(0x18)) as usize, 0x0u32);
                p.edx = m.rd((p.ecx) as usize);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                stk[0] = p.esi;
                p.esi = unsafe { *a.add(1) };
                m.wd((p.eax.wrapping_add(0x20)) as usize, p.edx);
                p.ecx = m.rd((p.ecx.wrapping_add(0x8)) as usize);
                p.edx = unsafe { *a.add(2) };
                stk[1] = p.edi;
                m.wd((p.eax.wrapping_add(0x24)) as usize, p.ecx);
                p.edi = (p.eax.wrapping_add(0x28));
                p.ecx = 0x9u32;
                while p.ecx != 0 { let v = m.rd(p.esi as usize); m.wd(p.edi as usize, v); p.esi = p.esi.wrapping_add(4); p.edi = p.edi.wrapping_add(4); p.ecx = p.ecx.wrapping_sub(1); }
                p.edi = stk[1];
                m.wd((p.eax.wrapping_add(0x4C)) as usize, p.edx);
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_a63700, r_a63700, w_a63700);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_a63740(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = p.ecx;
                m.wd((p.eax.wrapping_add(0x18)) as usize, p.edx);
                p.ecx = m.rd((p.edx.wrapping_add(0x1C)) as usize);
                m.wd((p.eax.wrapping_add(0x1C)) as usize, p.ecx);
                p.ecx = m.rd((p.edx.wrapping_add(0x20)) as usize);
                m.wd((p.eax.wrapping_add(0x20)) as usize, p.ecx);
                p.ecx = m.rd((p.edx.wrapping_add(0x24)) as usize);
                stk[0] = p.esi;
                stk[1] = p.edi;
                m.wd((p.eax.wrapping_add(0x24)) as usize, p.ecx);
                p.esi = (p.edx.wrapping_add(0x28));
                p.edi = (p.eax.wrapping_add(0x28));
                p.ecx = 0x9u32;
                while p.ecx != 0 { let v = m.rd(p.esi as usize); m.wd(p.edi as usize, v); p.esi = p.esi.wrapping_add(4); p.edi = p.edi.wrapping_add(4); p.ecx = p.ecx.wrapping_sub(1); }
                p.edx = m.rd((p.edx.wrapping_add(0x4C)) as usize);
                p.edi = stk[1];
                m.wd((p.eax.wrapping_add(0x4C)) as usize, p.edx);
                p.esi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_a63740, r_a63740, w_a63740);



#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ad5780(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = m.rd((p.ecx.wrapping_add(0x10)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 9; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.ecx.wrapping_add(0xC)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 9; continue; }
                pc = 2; continue;
            }
            2 => {
                p.edx = unsafe { *a.add(0) };
                stk[0] = p.esi;
                stk[1] = p.edi;
                pc = 3; continue;
            }
            3 => {
                p.edi = m.rd((p.eax.wrapping_add(0x58)) as usize);
                let x = p.edi; let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 6; continue; }
                pc = 4; continue;
            }
            4 => {
                p.esi = m.rd((p.edx.wrapping_add(0x40)) as usize);
                let x = p.esi; let y = 0x10u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 8; continue; }
                pc = 5; continue;
            }
            5 => {
                m.wd((p.edx.wrapping_add(p.esi.wrapping_mul(4))) as usize, p.edi);
                let x = m.rd((p.edx.wrapping_add(0x40)) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((p.edx.wrapping_add(0x40)) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 6; continue;
            }
            6 => {
                p.eax = m.rd((p.eax.wrapping_add(0x5C)) as usize);
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0xC)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 3; continue; }
                pc = 8; continue;
            }
            8 => {
                p.edi = stk[1];
                p.esi = stk[0];
                pc = 9; continue;
            }
            9 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_ad5780, r_ad5780, w_ad5780);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b2e6e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.esi.wrapping_add(0x856C)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 10; continue; }
                pc = 1; continue;
            }
            1 => {
                stk[0] = p.ebx;
                p.ebx = m.rd((0x252A110u32) as usize);
                stk[1] = p.ebp;
                p.ebp = (p.ebx.wrapping_add(p.ebx.wrapping_mul(2)));
                p.ebp = (p.ebp.wrapping_mul(8).wrapping_add(0x3162350));
                stk[2] = p.edi;
                pc = 2; continue;
            }
            2 => {
                let x = p.ebp; let y = 0x3165350u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 9; continue; }
                pc = 3; continue;
            }
            3 => {
                p.eax = p.ebp;
                let r = (m.rd((p.eax.wrapping_add(0x14)) as usize)) | (0x1u32);
                m.wd((p.eax.wrapping_add(0x14)) as usize, r);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ecx = m.rd((p.edx.wrapping_add(0x10)) as usize);
                p.edi = m.rd((p.eax.wrapping_add(0x14)) as usize);
                let r = (p.ecx) ^ (p.edi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) & (0x2u32);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.ecx) ^ (p.edi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x14)) as usize, p.ecx);
                p.edi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                let r = (p.edi) ^ (p.ecx);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.edi) & (0x4u32);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.edi) ^ (p.ecx);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x14)) as usize, p.edi);
                p.ecx = m.rd((p.esi.wrapping_add(0x14)) as usize);
                m.wd((p.eax) as usize, p.ecx);
                p.ecx = m.rd((p.esi.wrapping_add(0x18)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.ecx);
                p.ecx = m.rd((p.esi.wrapping_add(0x1C)) as usize);
                m.wd((p.eax.wrapping_add(0x8)) as usize, p.ecx);
                p.ecx = m.rd((p.esi.wrapping_add(0x44)) as usize);
                m.wd((p.eax.wrapping_add(0xC)) as usize, p.ecx);
                p.ecx = m.rd((p.edx.wrapping_add(0xC)) as usize);
                let x = p.ebx; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.ebx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ebp; let y = 0x18u32; let t = fl_add(x, y, 0, 32);
                p.ebp = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.eax.wrapping_add(0x10)) as usize, p.ecx);
                let r = (m.rb((p.edx.wrapping_add(0x10)) as usize)) & (0x2u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.eax = m.rd((p.edx.wrapping_add(0xC)) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                let r = (m.rd((p.eax.wrapping_add(0x10)) as usize)) | (0x100000u32);
                m.wd((p.eax.wrapping_add(0x10)) as usize, r);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 6; continue;
            }
            6 => {
                let x = m.rd((0x252A114u32) as usize); let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                m.wd((0x252A114u32) as usize, t.r);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 7; continue;
            }
            7 => {
                p.edx = m.rd((p.edx.wrapping_add(0x14)) as usize);
                let x = p.edx; let y = m.rd((p.esi.wrapping_add(0x856C)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 9; continue;
            }
            9 => {
                p.edi = stk[2];
                p.ebp = stk[1];
                m.wd((0x252A110u32) as usize, p.ebx);
                p.ebx = stk[0];
                pc = 10; continue;
            }
            10 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b2e6e0, r_b2e6e0, w_b2e6e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b68c20(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((0x2528B48u32) as usize)) & 0xFF);
                let x = (p.ecx & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.ebx;
                p.ebx = (p.ebx & 0xFFFFFF00) | (((unsafe { *a.add(0) } & 0xFF)) & 0xFF);
                stk[1] = p.edi;
                p.edi = p.eax;
                p.eax = m.rd((p.esi) as usize);
                if !zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.edx & 0xFF)) & ((p.edx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.edx = m.rd((p.eax.wrapping_add(0x10)) as usize);
                pc = 4; continue;
            }
            3 => {
                p.edx = m.rd((p.eax.wrapping_add(0xC)) as usize);
                pc = 4; continue;
            }
            4 => {
                p.eax = m.rd((p.esi.wrapping_add(0x4)) as usize);
                p.ecx = m.rd((p.esi.wrapping_add(0x8)) as usize);
                let x = p.ecx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = ((p.ebx & 0xFF)) & ((p.ebx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                p.edx = p.edi;
                pc = 7; continue;
            }
            6 => {
                let x = p.edi; let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = p.ecx;
                if (sf != of) { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                p.edi = p.edx;
                pc = 8; continue;
            }
            8 => {
                let x = p.eax; let y = p.edi; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = ((p.ebx & 0xFF)) & ((p.ebx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.esi.wrapping_add(0x4)) as usize, p.eax);
                p.eax = p.edi;
                if zf { pc = 11; continue; }
                pc = 9; continue;
            }
            9 => {
                let x = p.ecx; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 11; continue; }
                pc = 10; continue;
            }
            10 => {
                let x = p.edx; let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.esi.wrapping_add(0x8)) as usize, p.edx);
                pc = 11; continue;
            }
            11 => {
                p.edi = stk[1];
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b68c20, r_b68c20, w_b68c20);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b69190(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((0x22D7B50u32) as usize);
                stk[0] = p.ebx;
                p.ebx = unsafe { *a.add(0) };
                stk[1] = p.ebp;
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                stk[2] = p.esi;
                if (zf || sf != of) { pc = 6; continue; }
                pc = 1; continue;
            }
            1 => {
                p.esi = m.rd((p.ecx) as usize);
                p.ecx = p.esi;
                pc = 2; continue;
            }
            2 => {
                p.ebp = m.rd((p.ecx) as usize);
                let x = m.rd((p.ebp.wrapping_add(0x4)) as usize); let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 5; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.eax; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0xCu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf != of) { pc = 2; continue; }
                pc = 4; continue;
            }
            4 => {
                p.esi = stk[2];
                p.ebp = stk[1];
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebx = stk[0];
                return;
            }
            5 => {
                p.edx = (p.eax.wrapping_add(p.eax.wrapping_mul(2)));
                p.ecx = (p.esi.wrapping_add(p.edx.wrapping_mul(4)));
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.esi = stk[2];
                p.ebp = stk[1];
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebx = stk[0];
                return;
            }
            7 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x4)) as usize);
                let x = p.ebx; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = p.ebx;
                if (sf != of) { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                p.eax = p.edx;
                pc = 9; continue;
            }
            9 => {
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x8)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if (cf || zf) { pc = 11; continue; }
                pc = 10; continue;
            }
            10 => {
                let r = (p.esi) ^ (p.esi);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = p.esi; let t = fl_sub(x, y, cf as u32, 32);
                p.esi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.esi) & (p.edx);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.esi);
                pc = 11; continue;
            }
            11 => {
                p.esi = stk[2];
                p.ebp = stk[1];
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b69190, r_b69190, w_b69190);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b6b500(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.esi) & (p.esi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 10; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.edi) & (p.edi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 10; continue; }
                pc = 2; continue;
            }
            2 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rb((p.esi.wrapping_add(0x84D8)) as usize); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 8; continue; }
                pc = 3; continue;
            }
            3 => {
                p.edx = m.rd((p.esi.wrapping_add(0x84D0)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                stk[0] = p.ebx;
                if (cf || zf) { pc = 7; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = (p.esi.wrapping_add(0x84C8));
                pc = 5; continue;
            }
            5 => {
                p.ebx = m.rd((p.ecx) as usize);
                let x = m.rd((p.ebx) as usize); let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 9; continue; }
                pc = 6; continue;
            }
            6 => {
                let x = p.eax; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0x4u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if cf { pc = 5; continue; }
                pc = 7; continue;
            }
            7 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebx = stk[0];
                pc = 8; continue;
            }
            8 => {
                return;
            }
            9 => {
                p.eax = m.rd((p.esi.wrapping_add(p.eax.wrapping_mul(4)).wrapping_add(0x84C8)) as usize);
                p.ebx = stk[0];
                return;
            }
            10 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b6b500, r_b6b500);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b72c40(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rd((p.eax.wrapping_add(0x34)) as usize); let y = 0x5u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = (p.ecx & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                p.eax = (p.eax & 0xFFFFFF00) | (((p.ecx & 0xFF)) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b72c40, r_b72c40);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b72ce0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                let x = m.rb((0x252973Au32) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 11; continue; }
                pc = 3; continue;
            }
            3 => {
                let r = (m.rb((0x27A6A40u32) as usize)) & (0x40u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 11; continue; }
                pc = 4; continue;
            }
            4 => {
                p.ecx = (p.ecx & 0xFFFFFF00) | ((m.rb((0x2528B48u32) as usize)) & 0xFF);
                let x = (p.ecx & 0xFF); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                p.edx = (p.edx & 0xFFFFFF00) | ((m.rb((0x2528B49u32) as usize)) & 0xFF);
                let r = ((p.edx & 0xFF)) & ((p.edx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                let x = m.rb((0x2A4D2C5u32) as usize); let y = (p.edx & 0xFF); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 11; continue; }
                pc = 7; continue;
            }
            7 => {
                let x = (unsafe { *a.add(0) } & 0xFF); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                let r = (m.rb((p.eax.wrapping_add(0xC)) as usize)) & (0x10u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 10; continue;
            }
            9 => {
                let r = (m.rb((p.eax.wrapping_add(0xC)) as usize)) & (0x20u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 10; continue;
            }
            10 => {
                if zf { pc = 1; continue; }
                pc = 11; continue;
            }
            11 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b72ce0, r_b72ce0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b73ca0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                p.ecx = m.rd((p.eax.wrapping_add(0x34)) as usize);
                let x = p.ecx; let y = 0xFFFFFFF3u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (!cf && !zf) { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.eax.wrapping_add(0xC)) as usize);
                let x = p.eax; let t = fl_shr(x, 22, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.eax) & (0x1u32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b73ca0, r_b73ca0);


#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b8d100(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx.wrapping_add(0x24)) as usize);
                let x = p.eax; let y = 0x0u32; let t = fl_sub(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x1u32; let t = fl_sub(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = m.rd((p.ecx.wrapping_add(0x30)) as usize);
                let x = p.eax; let y = m.rd((0x252A23Cu32) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 7; continue; }
                pc = 3; continue;
            }
            3 => {
                p.ecx = m.rd((0x252A240u32) as usize);
                p.eax = (p.eax.wrapping_add(p.eax.wrapping_mul(2)));
                p.eax = m.rd((p.ecx.wrapping_add(p.eax.wrapping_mul(8)).wrapping_add(0x14)) as usize);
                return;
            }
            4 => {
                p.ecx = m.rd((p.ecx.wrapping_add(0x2C)) as usize);
                let x = p.ecx; let y = 0x22u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf == of) { pc = 7; continue; }
                pc = 5; continue;
            }
            5 => {
                p.eax = m.rd((0x252A238u32) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                p.eax = m.rd((p.eax.wrapping_add(p.ecx.wrapping_mul(4))) as usize);
                return;
            }
            7 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b8d100, r_b8d100);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b91e70(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rw((p.esi) as usize);
                let x = (p.eax & 0xFFFF); let y = 0x30u32; let t = fl_sub(x, y, 0, 16);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.ecx = p.esi;
                if cf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFF);
                pc = 2; continue;
            }
            2 => {
                let x = (p.eax & 0xFFFF); let y = 0x39u32; let t = fl_sub(x, y, 0, 16);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (!cf && !zf) { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                p.eax = m.rw((p.ecx.wrapping_add(0x2)) as usize);
                let x = p.ecx; let y = 0x2u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = (p.eax & 0xFFFF); let y = 0x30u32; let t = fl_sub(x, y, 0, 16);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 2; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = p.ecx; let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.ecx; let y = p.esi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = 0x1u32;
                if cf { pc = 8; continue; }
                pc = 5; continue;
            }
            5 => {
                stk[0] = p.edi;
                pc = 6; continue;
            }
            6 => {
                p.edi = m.rw((p.ecx) as usize);
                let x = p.edi; let y = 0x30u32; let t = fl_sub(x, y, 0, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let t = fl_imul(p.edi, p.edx);
                p.edi = t.r;
                (cf, of) = (t.cf, t.of);
                p.edx = (p.edx.wrapping_add(p.edx.wrapping_mul(4)));
                let x = p.ecx; let y = 0x2u32; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.edi; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.esi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 6; continue; }
                pc = 7; continue;
            }
            7 => {
                p.edi = stk[0];
                pc = 8; continue;
            }
            8 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b91e70, r_b91e70);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b93b40(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.edx.wrapping_add(0xC)) as usize);
                stk[0] = p.ebx;
                stk[1] = p.esi;
                stk[2] = p.edi;
                let r = (p.ebx) ^ (p.ebx);
                p.ebx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = p.ebx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 10; continue; }
                pc = 2; continue;
            }
            2 => {
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.edi = m.rd((p.edx.wrapping_add(0xC)) as usize);
                p.ecx = p.esi;
                let x = p.ecx; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let t = fl_sub(0, x, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.ecx; let t = fl_sub(x, y, cf as u32, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.ecx) & (p.esi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.edi; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.esi; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.ebx);
                m.wd((p.eax) as usize, p.ebx);
                m.wd((p.edx.wrapping_add(0xC)) as usize, p.ebx);
                pc = 7; continue;
            }
            5 => {
                m.wd((p.edx.wrapping_add(0xC)) as usize, p.esi);
                pc = 6; continue;
            }
            6 => {
                p.esi = m.rd((p.eax) as usize);
                p.edi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.esi.wrapping_add(0x4)) as usize, p.edi);
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.edi = m.rd((p.eax) as usize);
                m.wd((p.esi) as usize, p.edi);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.ebx);
                m.wd((p.eax) as usize, p.ebx);
                pc = 7; continue;
            }
            7 => {
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                let x = p.esi; let y = p.ebx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.eax);
                m.wd((p.eax) as usize, p.eax);
                m.wd((p.edx.wrapping_add(0x10)) as usize, p.eax);
                p.eax = p.ecx;
                pc = 1; continue;
            }
            9 => {
                p.esi = m.rd((p.esi) as usize);
                m.wd((p.esi.wrapping_add(0x4)) as usize, p.eax);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                p.esi = m.rd((p.esi) as usize);
                m.wd((p.eax) as usize, p.esi);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.esi) as usize, p.eax);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.esi);
                m.wd((p.edx.wrapping_add(0x10)) as usize, p.eax);
                p.eax = p.ecx;
                pc = 1; continue;
            }
            10 => {
                p.edi = stk[2];
                p.esi = stk[1];
                m.wb((p.edx.wrapping_add(0x1C)) as usize, 0x1u32);
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b93b40, r_b93b40, w_b93b40);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b93da0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = m.rb((p.eax.wrapping_add(0x1C)) as usize); let y = 0x1u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                p.eax = m.rd((p.eax.wrapping_add(0xC)) as usize);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b93da0, r_b93da0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b942d0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 12; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.edx & 0xFFFF);
                let x = p.eax; let y = 0x80u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 12; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x22Cu32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = 0x31A96F0u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = m.rd((p.eax.wrapping_add(0x224)) as usize); let y = p.edx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 12; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = m.rd((p.eax.wrapping_add(0x21C)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 12; continue; }
                pc = 4; continue;
            }
            4 => {
                let x = m.rd((p.ecx.wrapping_add(0xC)) as usize); let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 8; continue; }
                pc = 5; continue;
            }
            5 => {
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                let x = p.edx; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, 0x0u32);
                m.wd((p.eax) as usize, 0x0u32);
                m.wd((p.ecx.wrapping_add(0xC)) as usize, 0x0u32);
                pc = 9; continue;
            }
            7 => {
                m.wd((p.ecx.wrapping_add(0xC)) as usize, p.edx);
                pc = 8; continue;
            }
            8 => {
                p.edx = m.rd((p.eax) as usize);
                stk[0] = p.edi;
                p.edi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.edi);
                p.edi = m.rd((p.eax) as usize);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.edx) as usize, p.edi);
                m.wd((p.eax.wrapping_add(0x4)) as usize, 0x0u32);
                m.wd((p.eax) as usize, 0x0u32);
                p.edi = stk[0];
                pc = 9; continue;
            }
            9 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x10)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 11; continue; }
                pc = 10; continue;
            }
            10 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.eax);
                m.wd((p.eax) as usize, p.eax);
                m.wd((p.ecx.wrapping_add(0x10)) as usize, p.eax);
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            11 => {
                p.edx = m.rd((p.edx) as usize);
                m.wd((p.eax) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x10)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.edx);
                p.edx = m.rd((p.ecx.wrapping_add(0x10)) as usize);
                p.edx = m.rd((p.edx) as usize);
                m.wd((p.edx.wrapping_add(0x4)) as usize, p.eax);
                p.ecx = m.rd((p.ecx.wrapping_add(0x10)) as usize);
                m.wd((p.ecx) as usize, p.eax);
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            12 => {
                let r = ((p.eax & 0xFF)) ^ ((p.eax & 0xFF));
                p.eax = (p.eax & 0xFFFFFF00) | ((r) & 0xFF);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b942d0, r_b942d0, w_b942d0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b94380(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.edx.wrapping_add(0xC)) as usize);
                stk[0] = p.ebx;
                stk[1] = p.esi;
                stk[2] = p.edi;
                let r = (p.ebx) ^ (p.ebx);
                p.ebx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = p.ebx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 10; continue; }
                pc = 2; continue;
            }
            2 => {
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.edi = m.rd((p.edx.wrapping_add(0xC)) as usize);
                p.ecx = p.esi;
                let x = p.ecx; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let t = fl_sub(0, x, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.ecx; let t = fl_sub(x, y, cf as u32, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.ecx) & (p.esi);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = p.edi; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 3; continue;
            }
            3 => {
                let x = p.esi; let y = p.eax; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.ebx);
                m.wd((p.eax) as usize, p.ebx);
                m.wd((p.edx.wrapping_add(0xC)) as usize, p.ebx);
                pc = 7; continue;
            }
            5 => {
                m.wd((p.edx.wrapping_add(0xC)) as usize, p.esi);
                pc = 6; continue;
            }
            6 => {
                p.esi = m.rd((p.eax) as usize);
                p.edi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.esi.wrapping_add(0x4)) as usize, p.edi);
                p.esi = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.edi = m.rd((p.eax) as usize);
                m.wd((p.esi) as usize, p.edi);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.ebx);
                m.wd((p.eax) as usize, p.ebx);
                pc = 7; continue;
            }
            7 => {
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                let x = p.esi; let y = p.ebx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 9; continue; }
                pc = 8; continue;
            }
            8 => {
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.eax);
                m.wd((p.eax) as usize, p.eax);
                m.wd((p.edx.wrapping_add(0x10)) as usize, p.eax);
                p.eax = p.ecx;
                pc = 1; continue;
            }
            9 => {
                p.esi = m.rd((p.esi) as usize);
                m.wd((p.eax) as usize, p.esi);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.eax.wrapping_add(0x4)) as usize, p.esi);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                p.esi = m.rd((p.esi) as usize);
                m.wd((p.esi.wrapping_add(0x4)) as usize, p.eax);
                p.esi = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.esi) as usize, p.eax);
                p.eax = p.ecx;
                pc = 1; continue;
            }
            10 => {
                p.edi = stk[2];
                p.esi = stk[1];
                p.ebx = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b94380, r_b94380, w_b94380);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b967a0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = m.rd((p.edx) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.ecx.wrapping_add(0x7C));
                p.ecx = (p.edx.wrapping_add(0x4));
                m.wd((p.edx) as usize, 0x6u32);
                p.edx = m.rd((p.eax) as usize);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                pc = 3; continue;
            }
            3 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b967a0, r_b967a0, w_b967a0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b97220(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 1];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((0x252A214u32) as usize);
                let x = p.eax; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[0] = p.esi;
                let r = (p.esi) ^ (p.esi);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.ecx.wrapping_add(0x28)) as usize, p.eax);
                m.wd((0x252A214u32) as usize, p.eax);
                let r = (p.eax) & (0x3Fu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.ecx.wrapping_add(0x2C)) as usize, p.esi);
                m.wd((p.ecx.wrapping_add(0x30)) as usize, p.esi);
                m.wd((p.ecx.wrapping_add(0x24)) as usize, p.esi);
                m.wd((p.ecx.wrapping_add(0x1C)) as usize, p.esi);
                m.wd((p.ecx.wrapping_add(0x20)) as usize, p.esi);
                p.edx = m.rd((p.eax.wrapping_mul(4).wrapping_add(0x22EF2B8)) as usize);
                let x = p.edx; let y = p.esi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.esi = stk[0];
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.ecx.wrapping_add(0xC)) as usize, p.ecx);
                m.wd((p.ecx.wrapping_add(0x10)) as usize, p.ecx);
                m.wd((p.eax.wrapping_mul(4).wrapping_add(0x22EF2B8)) as usize, p.ecx);
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.ecx.wrapping_add(0x10)) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_mul(4).wrapping_add(0x22EF2B8)) as usize);
                m.wd((p.ecx.wrapping_add(0xC)) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_mul(4).wrapping_add(0x22EF2B8)) as usize);
                p.edx = m.rd((p.edx.wrapping_add(0x10)) as usize);
                m.wd((p.edx.wrapping_add(0xC)) as usize, p.ecx);
                p.eax = m.rd((p.eax.wrapping_mul(4).wrapping_add(0x22EF2B8)) as usize);
                m.wd((p.eax.wrapping_add(0x10)) as usize, p.ecx);
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b97220, r_b97220, w_b97220);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9ac60(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 4];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.ebp;
                p.ebp = unsafe { *a.add(0) };
                let r = (p.ebp) & (p.ebp);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) | (0xFFFFFFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.ebp = stk[0];
                return;
            }
            2 => {
                stk[1] = p.ebx;
                stk[2] = p.esi;
                stk[3] = p.edi;
                p.edi = m.rd((0x252A248u32) as usize);
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let r = (p.edi) & (p.edi);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if (zf || sf != of) { pc = 13; continue; }
                pc = 3; continue;
            }
            3 => {
                p.esi = 0x22EF578u32;
                pc = 4; continue;
            }
            4 => {
                p.edx = p.esi;
                p.ecx = p.ebp;
                pc = 5; continue;
            }
            5 => {
                p.ebx = (p.ebx & 0xFFFFFF00) | ((m.rb((p.ecx) as usize)) & 0xFF);
                let x = (p.ebx & 0xFF); let y = m.rb((p.edx) as usize); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 6; continue;
            }
            6 => {
                let r = ((p.ebx & 0xFF)) & ((p.ebx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 9; continue; }
                pc = 7; continue;
            }
            7 => {
                p.ebx = (p.ebx & 0xFFFFFF00) | ((m.rb((p.ecx.wrapping_add(0x1)) as usize)) & 0xFF);
                let x = (p.ebx & 0xFF); let y = m.rb((p.edx.wrapping_add(0x1)) as usize); let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 10; continue; }
                pc = 8; continue;
            }
            8 => {
                let x = p.ecx; let y = 0x2u32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0x2u32; let t = fl_add(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = ((p.ebx & 0xFF)) & ((p.ebx & 0xFF));
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 5; continue; }
                pc = 9; continue;
            }
            9 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 11; continue;
            }
            10 => {
                let x = p.ecx; let y = p.ecx; let t = fl_sub(x, y, cf as u32, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, cf as u32, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 11; continue;
            }
            11 => {
                let r = (p.ecx) & (p.ecx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 14; continue; }
                pc = 12; continue;
            }
            12 => {
                let x = p.eax; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = 0x2Cu32; let t = fl_add(x, y, 0, 32);
                p.esi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (sf != of) { pc = 4; continue; }
                pc = 13; continue;
            }
            13 => {
                let r = (p.eax) | (0xFFFFFFFFu32);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 14; continue;
            }
            14 => {
                p.edi = stk[3];
                p.esi = stk[2];
                p.ebx = stk[1];
                p.ebp = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b9ac60, r_b9ac60);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9c3c0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE010u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b9c3c0, r_b9c3c0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9ccb0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x98u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b9ccb0, r_b9ccb0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9d5b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x64u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b9d5b0, r_b9d5b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9d760(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let x = p.eax; let y = m.rd((0x252A270u32) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let t = fl_shl(x, 4, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = 0x22F2370u32; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            2 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_b9d760, r_b9d760);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_b9e6b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = p.ecx;
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                m.wd((p.eax.wrapping_add(0x28)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x2C)) as usize, p.ecx);
                m.wd((p.eax) as usize, 0xE4D734u32);
                m.wd((p.eax.wrapping_add(0x54)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0x64)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xAC)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xBC)) as usize, p.ecx);
                m.wd((p.eax.wrapping_add(0xCC)) as usize, p.ecx);
                let x = m.rd((p.eax.wrapping_add(0xAC)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = (p.ecx.wrapping_add(0x8));
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                m.wd((p.eax.wrapping_add(0xAC)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0xB0)) as usize, p.ecx);
                pc = 2; continue;
            }
            2 => {
                let x = m.rd((p.eax.wrapping_add(0xBC)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                m.wd((p.eax.wrapping_add(0xBC)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0xC0)) as usize, p.ecx);
                pc = 4; continue;
            }
            4 => {
                let x = m.rd((p.eax.wrapping_add(0xCC)) as usize); let y = p.ecx; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 6; continue; }
                pc = 5; continue;
            }
            5 => {
                m.wd((p.eax.wrapping_add(0xCC)) as usize, p.edx);
                m.wd((p.eax.wrapping_add(0xD0)) as usize, p.ecx);
                pc = 6; continue;
            }
            6 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_b9e6b0, r_b9e6b0, w_b9e6b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba0c50(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE3F4u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba0c50, r_ba0c50);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba1700(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0xC4u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba1700, r_ba1700);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba18e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE530u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBE3F4u32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if zf { pc = 1; continue; }
                pc = 3; continue;
            }
            3 => {
                p.ecx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba18e0, r_ba18e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba1c30(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = m.rd((p.edx) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.ecx.wrapping_add(0xC4));
                p.ecx = (p.edx.wrapping_add(0x4));
                m.wd((p.edx) as usize, 0x6u32);
                p.edx = m.rd((p.eax) as usize);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                pc = 3; continue;
            }
            3 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_ba1c30, r_ba1c30, w_ba1c30);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba1d00(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = m.rd((p.edx) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.ecx.wrapping_add(0xD0));
                p.ecx = (p.edx.wrapping_add(0x4));
                m.wd((p.edx) as usize, 0x6u32);
                p.edx = m.rd((p.eax) as usize);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                pc = 3; continue;
            }
            3 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_ba1d00, r_ba1d00, w_ba1d00);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba1dd0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = m.rd((p.edx) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.ecx.wrapping_add(0xDC));
                p.ecx = (p.edx.wrapping_add(0x4));
                m.wd((p.edx) as usize, 0x6u32);
                p.edx = m.rd((p.eax) as usize);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                pc = 3; continue;
            }
            3 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_ba1dd0, r_ba1dd0, w_ba1dd0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba1ea0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = unsafe { *a.add(0) };
                p.eax = m.rd((p.edx) as usize);
                let r = (p.eax) & (p.eax);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let x = p.eax; let y = 0x6u32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.ecx.wrapping_add(0xE8));
                p.ecx = (p.edx.wrapping_add(0x4));
                m.wd((p.edx) as usize, 0x6u32);
                p.edx = m.rd((p.eax) as usize);
                m.wd((p.ecx) as usize, p.edx);
                p.edx = m.rd((p.eax.wrapping_add(0x4)) as usize);
                m.wd((p.ecx.wrapping_add(0x4)) as usize, p.edx);
                p.eax = m.rd((p.eax.wrapping_add(0x8)) as usize);
                m.wd((p.ecx.wrapping_add(0x8)) as usize, p.eax);
                pc = 3; continue;
            }
            3 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_ba1ea0, r_ba1ea0, w_ba1ea0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba2620(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x104u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba2620, r_ba2620);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba2820(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE790u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba2820, r_ba2820);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba45a0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0xF4u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba45a0, r_ba45a0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba46e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE7E8u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba46e0, r_ba46e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba4b70(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x98u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba4b70, r_ba4b70);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba4c90(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE850u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba4c90, r_ba4c90);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba5370(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0xA4u32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba5370, r_ba5370);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba54c0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE96Cu32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba54c0, r_ba54c0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba6090(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0xCCu32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba6090, r_ba6090);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba61d0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBE9D0u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba61d0, r_ba61d0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba6860(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                p.eax = (p.eax.wrapping_add(p.eax.wrapping_mul(4)));
                let x = p.eax; let t = fl_shl(x, 5, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba6860, r_ba6860);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba6980(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                p.ecx = m.rd((0xEBEA18u32) as usize);
                let x = p.eax; let y = m.rd((p.ecx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((0x1u32) & 0xFF);
                return;
            }
            2 => {
                p.edx = m.rd((0xEBDF6Cu32) as usize);
                let x = p.eax; let y = m.rd((p.edx) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.eax = (p.eax & 0xFFFFFF00) | (((zf) as u32) & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba6980, r_ba6980);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_ba6f40(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.edx = m.rd((p.ecx.wrapping_add(0x14)) as usize);
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if zf { pc = 3; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = unsafe { *a.add(0) };
                let x = p.eax; let y = m.rd((p.ecx.wrapping_add(0x10)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !cf { pc = 3; continue; }
                pc = 2; continue;
            }
            2 => {
                let t = fl_imul(p.eax, 0x9Cu32);
                p.eax = t.r;
                (cf, of) = (t.cf, t.of);
                let x = p.eax; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.eax = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                return;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_ba6f40, r_ba6f40);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_c07030(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx.wrapping_add(0x44)) as usize);
                m.wd((p.ecx.wrapping_add(0x48)) as usize, p.eax);
                p.edx = m.rd((p.ecx.wrapping_add(0x54)) as usize);
                m.wd((p.ecx.wrapping_add(0x58)) as usize, p.edx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_c07030, r_c07030, w_c07030);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_c078b0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = m.rd((p.ecx.wrapping_add(0x44)) as usize);
                m.wd((p.ecx.wrapping_add(0x48)) as usize, p.eax);
                p.edx = m.rd((p.ecx.wrapping_add(0x54)) as usize);
                m.wd((p.ecx.wrapping_add(0x58)) as usize, p.edx);
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_c078b0, r_c078b0, w_c078b0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_c0a1e0(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 3];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                stk[0] = p.edi;
                let r = (p.edi) ^ (p.edi);
                p.edi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rd((p.eax.wrapping_add(0x154)) as usize); let y = p.edi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if (cf || zf) { pc = 10; continue; }
                pc = 1; continue;
            }
            1 => {
                stk[1] = p.ebx;
                stk[2] = p.esi;
                let r = (p.esi) ^ (p.esi);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 2; continue;
            }
            2 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x170)) as usize);
                p.edx = m.rd((p.ecx.wrapping_add(p.esi)) as usize);
                let x = p.ecx; let y = p.esi; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                m.wd((p.ecx) as usize, 0x0u32);
                pc = 5; continue;
            }
            4 => {
                p.ebx = m.rd((p.eax.wrapping_add(0x174)) as usize);
                let x = p.ebx; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.ebx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.ecx) as usize, p.ebx);
                pc = 5; continue;
            }
            5 => {
                p.edx = m.rd((p.eax.wrapping_add(0x170)) as usize);
                p.ecx = (p.esi.wrapping_add(p.edx).wrapping_add(0x4));
                p.edx = m.rd((p.ecx) as usize);
                let x = p.edx; let y = 0xFFFFFFFFu32; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 7; continue; }
                pc = 6; continue;
            }
            6 => {
                m.wd((p.ecx) as usize, 0x0u32);
                pc = 8; continue;
            }
            7 => {
                p.ebx = m.rd((p.eax.wrapping_add(0x178)) as usize);
                let x = p.ebx; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.ebx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                m.wd((p.ecx) as usize, p.ebx);
                pc = 8; continue;
            }
            8 => {
                let x = p.edi; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = 0x1Cu32; let t = fl_add(x, y, 0, 32);
                p.esi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edi; let y = m.rd((p.eax.wrapping_add(0x154)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if cf { pc = 2; continue; }
                pc = 9; continue;
            }
            9 => {
                p.esi = stk[2];
                p.ebx = stk[1];
                pc = 10; continue;
            }
            10 => {
                p.edi = stk[0];
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_c0a1e0, r_c0a1e0, w_c0a1e0);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_c0a290(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut stk = [0u32; 2];
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                let r = (m.rb((p.eax.wrapping_add(0x14C)) as usize)) & (0x2u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 11; continue; }
                pc = 1; continue;
            }
            1 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x164)) as usize);
                p.edx = m.rd((p.eax.wrapping_add(0x160)) as usize);
                let x = p.edx; let y = 0x7FFu32; let t = fl_add(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0x7FFu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let t = fl_shr(x, 11, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let t = fl_shr(x, 11, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                p.edx = m.rd((p.eax.wrapping_add(0x15C)) as usize);
                stk[0] = p.esi;
                let x = p.edx; let y = 0x7FFu32; let t = fl_add(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let t = fl_shr(x, 11, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.esi) ^ (p.esi);
                p.esi = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                let x = m.rd((p.eax.wrapping_add(0x154)) as usize); let y = p.esi; let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                stk[1] = p.edi;
                p.edi = (p.ecx.wrapping_add(p.edx).wrapping_add(0x1));
                if (cf || zf) { pc = 10; continue; }
                pc = 2; continue;
            }
            2 => {
                let r = (p.ecx) ^ (p.ecx);
                p.ecx = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                pc = 3; continue;
            }
            3 => {
                p.edx = m.rd((p.eax.wrapping_add(0x170)) as usize);
                m.wd((p.edx.wrapping_add(p.ecx).wrapping_add(0x8)) as usize, p.edi);
                let r = (m.rb((p.eax.wrapping_add(0x14C)) as usize)) & (0x1u32);
                let t = fl_logic(r, 8); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                p.edx = m.rd((p.eax.wrapping_add(0x170)) as usize);
                if zf { pc = 5; continue; }
                pc = 4; continue;
            }
            4 => {
                p.edx = m.rd((p.edx.wrapping_add(p.ecx).wrapping_add(0x14)) as usize);
                pc = 6; continue;
            }
            5 => {
                p.edx = m.rd((p.edx.wrapping_add(p.ecx).wrapping_add(0x10)) as usize);
                pc = 6; continue;
            }
            6 => {
                let r = (p.edx) & (p.edx);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 8; continue; }
                pc = 7; continue;
            }
            7 => {
                let x = p.edi; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            8 => {
                let x = p.edx; let y = 0x7FFu32; let t = fl_add(x, y, 0, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edx; let t = fl_shr(x, 11, 32);
                p.edx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.edi; let y = p.edx; let t = fl_add(x, y, 0, 32);
                p.edi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                pc = 9; continue;
            }
            9 => {
                let x = p.esi; let y = 0x1u32; let t = fl_add(x, y, 0, 32);
                p.esi = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.ecx; let y = 0x1Cu32; let t = fl_add(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let x = p.esi; let y = m.rd((p.eax.wrapping_add(0x154)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if cf { pc = 3; continue; }
                pc = 10; continue;
            }
            10 => {
                p.edi = stk[1];
                p.esi = stk[0];
                pc = 11; continue;
            }
            11 => {
                return;
            }
            _ => return,
        }
    }
}
leaf_rw!(f_c0a290, r_c0a290, w_c0a290);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_cdfd40(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = unsafe { *a.add(0) };
                let x = m.rb((p.eax.wrapping_add(0xC)) as usize); let y = 0x0u32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 1; continue;
            }
            1 => {
                p.eax = m.rd((p.eax.wrapping_add(0x4)) as usize);
                p.ecx = m.rd((p.eax.wrapping_add(0x14)) as usize);
                let x = p.ecx; let y = m.rd((p.eax.wrapping_add(0x28)) as usize); let t = fl_sub(x, y, 0, 32);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 4; continue; }
                pc = 2; continue;
            }
            2 => {
                p.ecx = m.rd((p.eax.wrapping_add(0x8)) as usize);
                let x = p.ecx; let y = m.rd((p.eax.wrapping_add(0xC)) as usize); let t = fl_sub(x, y, 0, 32);
                p.ecx = t.r;
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                let r = (p.ecx) & (0xFFFFFFF0u32);
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                if !zf { pc = 4; continue; }
                pc = 3; continue;
            }
            3 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            4 => {
                p.eax = 0x1u32;
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_cdfd40, r_cdfd40);

#[allow(unused_assignments, unused_variables, unused_mut, unused_parens)]
fn f_d09810(p: &mut Pushad, a: *const u32, m: &mut dyn Mem) {
    let (mut zf, mut sf, mut cf, mut of) = (false, false, false, false);
    let mut pc = 0u32;
    loop {
        match pc {
            0 => {
                p.eax = (p.eax & 0xFFFFFF00) | ((m.rb((p.ecx.wrapping_add(0x348)) as usize)) & 0xFF);
                let x = (p.eax & 0xFF); let y = 0xFFu32; let t = fl_sub(x, y, 0, 8);
                (zf, sf, cf, of) = (t.zf, t.sf, t.cf, t.of);
                if !zf { pc = 2; continue; }
                pc = 1; continue;
            }
            1 => {
                let r = (p.eax) ^ (p.eax);
                p.eax = r;
                let t = fl_logic(r, 32); (zf, sf, cf, of) = (t.zf, t.sf, false, false);
                return;
            }
            2 => {
                p.eax = (p.eax & 0xFF);
                return;
            }
            _ => return,
        }
    }
}
leaf_ro!(f_d09810, r_d09810);

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
thunk!(t255, 255, "ret");
thunk!(t256, 256, "ret");
thunk!(t257, 257, "ret");
thunk!(t258, 258, "ret");
thunk!(t259, 259, "ret");
thunk!(t260, 260, "ret 4");
thunk!(t261, 261, "ret");
thunk!(t262, 262, "ret");
thunk!(t263, 263, "ret");
thunk!(t264, 264, "ret");
thunk!(t265, 265, "ret");
thunk!(t266, 266, "ret 12");
thunk!(t267, 267, "ret 12");
thunk!(t268, 268, "ret");
thunk!(t269, 269, "ret");
thunk!(t270, 270, "ret");
thunk!(t271, 271, "ret 12");
thunk!(t272, 272, "ret 12");
thunk!(t273, 273, "ret 8");
thunk!(t274, 274, "ret");
thunk!(t275, 275, "ret");
thunk!(t276, 276, "ret");
thunk!(t277, 277, "ret");
thunk!(t278, 278, "ret");
thunk!(t279, 279, "ret");
thunk!(t280, 280, "ret");
thunk!(t281, 281, "ret 16");
thunk!(t282, 282, "ret 4");
thunk!(t283, 283, "ret 4");
thunk!(t284, 284, "ret 4");
thunk!(t285, 285, "ret");
thunk!(t286, 286, "ret 8");
thunk!(t287, 287, "ret");
thunk!(t288, 288, "ret");
thunk!(t289, 289, "ret");
thunk!(t290, 290, "ret");
thunk!(t291, 291, "ret");
thunk!(t292, 292, "ret");
thunk!(t293, 293, "ret");
thunk!(t294, 294, "ret");
thunk!(t295, 295, "ret");
thunk!(t296, 296, "ret 8");
thunk!(t297, 297, "ret");
thunk!(t298, 298, "ret");
thunk!(t299, 299, "ret");
thunk!(t300, 300, "ret 8");
thunk!(t301, 301, "ret");
thunk!(t302, 302, "ret");
thunk!(t303, 303, "ret");
thunk!(t304, 304, "ret");
thunk!(t305, 305, "ret 4");
thunk!(t306, 306, "ret");
thunk!(t307, 307, "ret");
thunk!(t308, 308, "ret");
thunk!(t309, 309, "ret");
thunk!(t310, 310, "ret");
thunk!(t311, 311, "ret");
thunk!(t312, 312, "ret");
thunk!(t313, 313, "ret");
thunk!(t314, 314, "ret 4");
thunk!(t315, 315, "ret");
thunk!(t316, 316, "ret");
thunk!(t317, 317, "ret");
thunk!(t318, 318, "ret");
thunk!(t319, 319, "ret");
thunk!(t320, 320, "ret");
thunk!(t321, 321, "ret");
thunk!(t322, 322, "ret");
thunk!(t323, 323, "ret");
thunk!(t324, 324, "ret");
thunk!(t325, 325, "ret");
thunk!(t326, 326, "ret");
thunk!(t327, 327, "ret");
thunk!(t328, 328, "ret");
thunk!(t329, 329, "ret");
thunk!(t330, 330, "ret");
thunk!(t331, 331, "ret");
thunk!(t332, 332, "ret");
thunk!(t333, 333, "ret");
thunk!(t334, 334, "ret");
thunk!(t335, 335, "ret 4");
thunk!(t336, 336, "ret 4");
thunk!(t337, 337, "ret 8");
thunk!(t338, 338, "ret");
thunk!(t339, 339, "ret");
thunk!(t340, 340, "ret");
thunk!(t341, 341, "ret");
thunk!(t342, 342, "ret");
thunk!(t343, 343, "ret");
thunk!(t344, 344, "ret");
thunk!(t345, 345, "ret");
thunk!(t346, 346, "ret");
thunk!(t347, 347, "ret");
thunk!(t348, 348, "ret");
thunk!(t349, 349, "ret");
thunk!(t350, 350, "ret");
thunk!(t351, 351, "ret");
thunk!(t352, 352, "ret 4");
thunk!(t353, 353, "ret");
thunk!(t354, 354, "ret 4");
thunk!(t355, 355, "ret");
thunk!(t356, 356, "ret");
thunk!(t357, 357, "ret");
thunk!(t358, 358, "ret");
thunk!(t359, 359, "ret");
thunk!(t360, 360, "ret");
thunk!(t361, 361, "ret");
thunk!(t362, 362, "ret");
thunk!(t363, 363, "ret");
thunk!(t364, 364, "ret");
thunk!(t365, 365, "ret");
thunk!(t366, 366, "ret");
thunk!(t367, 367, "ret");
thunk!(t368, 368, "ret");
thunk!(t369, 369, "ret");
thunk!(t370, 370, "ret 4");
thunk!(t371, 371, "ret");
thunk!(t372, 372, "ret");
thunk!(t373, 373, "ret");
thunk!(t374, 374, "ret");
thunk!(t375, 375, "ret");
thunk!(t376, 376, "ret");
thunk!(t377, 377, "ret");
thunk!(t378, 378, "ret");
thunk!(t379, 379, "ret");
thunk!(t380, 380, "ret");
thunk!(t381, 381, "ret");
thunk!(t382, 382, "ret");
thunk!(t383, 383, "ret");
thunk!(t384, 384, "ret");
thunk!(t385, 385, "ret");
thunk!(t386, 386, "ret");
thunk!(t387, 387, "ret");
thunk!(t388, 388, "ret");
thunk!(t389, 389, "ret 8");
thunk!(t390, 390, "ret 4");
thunk!(t391, 391, "ret");
thunk!(t392, 392, "ret 4");
thunk!(t393, 393, "ret 20");
thunk!(t394, 394, "ret 16");
thunk!(t395, 395, "ret 12");
thunk!(t396, 396, "ret");
thunk!(t397, 397, "ret");
thunk!(t398, 398, "ret");
thunk!(t399, 399, "ret 12");
thunk!(t400, 400, "ret");
thunk!(t401, 401, "ret 16");
thunk!(t402, 402, "ret 12");
thunk!(t403, 403, "ret 12");
thunk!(t404, 404, "ret");
thunk!(t405, 405, "ret 4");
thunk!(t406, 406, "ret 12");
thunk!(t407, 407, "ret 12");
thunk!(t408, 408, "ret 16");
thunk!(t409, 409, "ret 12");
thunk!(t410, 410, "ret");
thunk!(t411, 411, "ret 16");
thunk!(t412, 412, "ret 12");
thunk!(t413, 413, "ret 12");
thunk!(t414, 414, "ret 16");
thunk!(t415, 415, "ret 4");
thunk!(t416, 416, "ret 12");
thunk!(t417, 417, "ret 12");
thunk!(t418, 418, "ret 12");
thunk!(t419, 419, "ret");
thunk!(t420, 420, "ret 16");
thunk!(t421, 421, "ret 4");
thunk!(t422, 422, "ret 4");
thunk!(t423, 423, "ret 12");
thunk!(t424, 424, "ret 12");
thunk!(t425, 425, "ret");
thunk!(t426, 426, "ret 12");
thunk!(t427, 427, "ret 4");
thunk!(t428, 428, "ret 12");
thunk!(t429, 429, "ret");
thunk!(t430, 430, "ret 4");
thunk!(t431, 431, "ret 16");
thunk!(t432, 432, "ret 4");
thunk!(t433, 433, "ret 12");
thunk!(t434, 434, "ret 12");
thunk!(t435, 435, "ret 12");
thunk!(t436, 436, "ret");
thunk!(t437, 437, "ret 4");
thunk!(t438, 438, "ret 8");
thunk!(t439, 439, "ret 8");
thunk!(t440, 440, "ret 8");
thunk!(t441, 441, "ret");
thunk!(t442, 442, "ret");
thunk!(t443, 443, "ret");
thunk!(t444, 444, "ret 8");
thunk!(t445, 445, "ret 8");
thunk!(t446, 446, "ret 8");
thunk!(t447, 447, "ret");
thunk!(t448, 448, "ret 12");
thunk!(t449, 449, "ret 4");
thunk!(t450, 450, "ret");
thunk!(t451, 451, "ret");
thunk!(t452, 452, "ret");
thunk!(t453, 453, "ret");
thunk!(t454, 454, "ret 4");
thunk!(t455, 455, "ret");
thunk!(t456, 456, "ret 12");
thunk!(t457, 457, "ret 4");
thunk!(t458, 458, "ret 4");
thunk!(t459, 459, "ret 8");
thunk!(t460, 460, "ret");
thunk!(t461, 461, "ret 12");
thunk!(t462, 462, "ret 4");
thunk!(t463, 463, "ret 12");
thunk!(t464, 464, "ret 8");
thunk!(t465, 465, "ret");
thunk!(t466, 466, "ret");
thunk!(t467, 467, "ret");
thunk!(t468, 468, "ret");
thunk!(t469, 469, "ret 4");
thunk!(t470, 470, "ret 4");
thunk!(t471, 471, "ret");
thunk!(t472, 472, "ret");
thunk!(t473, 473, "ret");
thunk!(t474, 474, "ret");
thunk!(t475, 475, "ret");
thunk!(t476, 476, "ret");
thunk!(t477, 477, "ret");
thunk!(t478, 478, "ret");
thunk!(t479, 479, "ret");
thunk!(t480, 480, "ret");
thunk!(t481, 481, "ret");
thunk!(t482, 482, "ret");
thunk!(t483, 483, "ret 12");
thunk!(t484, 484, "ret 12");
thunk!(t485, 485, "ret 8");
thunk!(t486, 486, "ret");
thunk!(t487, 487, "ret");
thunk!(t488, 488, "ret 4");
thunk!(t489, 489, "ret");
thunk!(t490, 490, "ret 4");
thunk!(t491, 491, "ret 4");
thunk!(t492, 492, "ret");
thunk!(t493, 493, "ret");
thunk!(t494, 494, "ret");
thunk!(t495, 495, "ret");
thunk!(t496, 496, "ret");
thunk!(t497, 497, "ret 12");
thunk!(t498, 498, "ret 12");
thunk!(t499, 499, "ret 8");
thunk!(t500, 500, "ret");
thunk!(t501, 501, "ret 12");
thunk!(t502, 502, "ret 12");
thunk!(t503, 503, "ret 8");
thunk!(t504, 504, "ret");
thunk!(t505, 505, "ret");
thunk!(t506, 506, "ret");
thunk!(t507, 507, "ret 12");
thunk!(t508, 508, "ret 12");
thunk!(t509, 509, "ret 8");
thunk!(t510, 510, "ret");
thunk!(t511, 511, "ret");
thunk!(t512, 512, "ret");
thunk!(t513, 513, "ret");
thunk!(t514, 514, "ret");
thunk!(t515, 515, "ret");
thunk!(t516, 516, "ret");
thunk!(t517, 517, "ret");
thunk!(t518, 518, "ret");
thunk!(t519, 519, "ret 8");
thunk!(t520, 520, "ret 8");
thunk!(t521, 521, "ret");
thunk!(t522, 522, "ret");
thunk!(t523, 523, "ret");
thunk!(t524, 524, "ret");
thunk!(t525, 525, "ret");
thunk!(t526, 526, "ret");
thunk!(t527, 527, "ret 4");
thunk!(t528, 528, "ret");
thunk!(t529, 529, "ret 4");
thunk!(t530, 530, "ret 8");
thunk!(t531, 531, "ret 8");
thunk!(t532, 532, "ret 8");
thunk!(t533, 533, "ret 4");
thunk!(t534, 534, "ret");
thunk!(t535, 535, "ret");
thunk!(t536, 536, "ret 8");
thunk!(t537, 537, "ret 4");
thunk!(t538, 538, "ret");
thunk!(t539, 539, "ret");
thunk!(t540, 540, "ret");
thunk!(t541, 541, "ret 4");
thunk!(t542, 542, "ret");
thunk!(t543, 543, "ret 8");
thunk!(t544, 544, "ret 4");
thunk!(t545, 545, "ret 4");
thunk!(t546, 546, "ret");
thunk!(t547, 547, "ret 8");
thunk!(t548, 548, "ret 8");
thunk!(t549, 549, "ret 8");
thunk!(t550, 550, "ret 8");
thunk!(t551, 551, "ret 4");
thunk!(t552, 552, "ret 4");
thunk!(t553, 553, "ret");
thunk!(t554, 554, "ret 4");
thunk!(t555, 555, "ret 4");
thunk!(t556, 556, "ret");
thunk!(t557, 557, "ret");
thunk!(t558, 558, "ret 4");
thunk!(t559, 559, "ret 8");
thunk!(t560, 560, "ret 4");
thunk!(t561, 561, "ret 4");
thunk!(t562, 562, "ret");
thunk!(t563, 563, "ret");
thunk!(t564, 564, "ret 4");
thunk!(t565, 565, "ret 4");
thunk!(t566, 566, "ret");
thunk!(t567, 567, "ret 4");
thunk!(t568, 568, "ret 4");
thunk!(t569, 569, "ret");
thunk!(t570, 570, "ret");
thunk!(t571, 571, "ret 4");
thunk!(t572, 572, "ret 4");
thunk!(t573, 573, "ret");
thunk!(t574, 574, "ret");
thunk!(t575, 575, "ret");
thunk!(t576, 576, "ret 4");
thunk!(t577, 577, "ret");
thunk!(t578, 578, "ret");
thunk!(t579, 579, "ret");
thunk!(t580, 580, "ret");
thunk!(t581, 581, "ret");
thunk!(t582, 582, "ret");
thunk!(t583, 583, "ret");

// --- the table ------------------------------------------------------------------------------------

const COUNT: usize = 584;

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
    port("audio_voice_is_2d", 0x004660b0, &B255, 0, 0, r_4660b0 as Run, None, t255),
    port("human_slowmo_rise_list_find", 0x00493860, &B256, 0, 0, r_493860 as Run, Some(w_493860 as Writes), t256),
    port("string_pool::vf0", 0x004a4a80, &B257, 0, 0, nop as Run, None, t257),
    port("string_pool::size", 0x004a4b00, &B258, 0, 0, load_ecx::<0x3c> as Run, None, t258),
    port("vhashtable<>::clear_buckets", 0x004ac100, &B259, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t259),
    port("lighting_ambient_debug_print", 0x004b26e0, &B260, 1, 4, r_4b26e0 as Run, Some(w_4b26e0 as Writes), t260),
    port("std::codecvt_base::scalar_deleting_dtor", 0x004e8380, &B261, 0, 0, eax_const::<1> as Run, None, t261),
    port("input_action_set", 0x004ff4b0, &B262, 0, 0, r_4ff4b0 as Run, Some(w_4ff4b0 as Writes), t262),
    port("input_actions_set_many", 0x004ff4e0, &B263, 0, 0, r_4ff4e0 as Run, Some(w_4ff4e0 as Writes), t263),
    port("input_stick_action_set", 0x004ff520, &B264, 0, 0, r_4ff520 as Run, Some(w_4ff520 as Writes), t264),
    port("input_stick_actions_set_many", 0x004ff550, &B265, 0, 0, r_4ff550 as Run, Some(w_4ff550 as Writes), t265),
    port("r_particle_ribbon_inst::compute_pass_data", 0x0050ef10, &B266, 3, 12, al_const::<1> as Run, None, t266),
    port("r_particle_ribbon_inst::prepare_pass", 0x0050ef20, &B267, 3, 12, al_const::<1> as Run, None, t267),
    port("r_particle_ribbon_inst::get_env_handle", 0x0050ef50, &B268, 0, 0, zero_eax as Run, None, t268),
    port("rl_decal_inst::casts_shadow", 0x0052e3f0, &B269, 0, 0, al_const::<0> as Run, None, t269),
    port("r_distant_ped_mesh_inst::casts_shadow", 0x0052e430, &B270, 0, 0, al_const::<0> as Run, None, t270),
    port("r_distant_ped_mesh_inst::compute_pass_data", 0x0052e440, &B271, 3, 12, al_const::<1> as Run, None, t271),
    port("r_distant_ped_mesh_inst::prepare_pass", 0x0052e450, &B272, 3, 12, al_const::<1> as Run, None, t272),
    port("r_distant_ped_mesh_inst::render_shadow", 0x0052e460, &B273, 2, 8, nop as Run, None, t273),
    port("r_distant_ped_mesh_inst::is_ready", 0x0052e470, &B274, 0, 0, al_const::<1> as Run, None, t274),
    port("rl_renderable_inst::get_env_handle", 0x00530110, &B275, 0, 0, zero_eax as Run, None, t275),
    port("inventory_item_weapon_slot", 0x00558d90, &B276, 0, 0, r_558d90 as Run, None, t276),
    port("activity_level_instance::dtor", 0x00574c70, &B277, 0, 0, r_574c70 as Run, Some(w_574c70 as Writes), t277),
    port("heli_objective::get_start_message", 0x00574fd0, &B278, 0, 0, zero_eax as Run, None, t278),
    port("demo_derby_info::get_activity_type", 0x005e6bc0, &B279, 0, 0, zero_eax as Run, None, t279),
    port("diversion_type_info::get_start_mode", 0x005f14c0, &B280, 0, 0, zero_eax as Run, None, t280),
    port("diversion_type_info::on_object_damaged", 0x005f1550, &B281, 4, 16, nop as Run, None, t281),
    port("diversion_type_info::on_game_event_20", 0x005f1570, &B282, 1, 4, nop as Run, None, t282),
    port("diversion_type_info::on_game_event_22", 0x005f1590, &B283, 1, 4, nop as Run, None, t283),
    port("diversion_type_info::on_game_event_18", 0x005f15d0, &B284, 1, 4, nop as Run, None, t284),
    port("ambulance_diversion_type_info::get_type", 0x005f16c0, &B285, 0, 0, eax_const::<1> as Run, None, t285),
    port("barnstorming_diversion_type::get_start_message", 0x005f7f60, &B286, 2, 8, nop as Run, None, t286),
    port("barnstorming_diversion_type::get_hud_icon_name", 0x005f7fb0, &B287, 0, 0, eax_const::<0xdf12f0> as Run, None, t287),
    port("base_jumping_diversion_type_info::get_hud_icon_name", 0x005f95b0, &B288, 0, 0, eax_const::<0xdf12f0> as Run, None, t288),
    port("blackjack_diversion_type_info::update", 0x005fba70, &B289, 0, 0, nop as Run, None, t289),
    port("blackjack_diversion_type_info::get_hud_icon_name", 0x005fbb00, &B290, 0, 0, eax_const::<0xdf12f0> as Run, None, t290),
    port("blackjack_diversion_type_info::get_type", 0x005fbca0, &B291, 0, 0, eax_const::<3> as Run, None, t291),
    port("chop_shop_diversion_type::can_start", 0x00600f00, &B292, 0, 0, al_const::<0> as Run, None, t292),
    port("chop_shop_diversion_type::get_title", 0x00600f90, &B293, 0, 0, eax_const::<0xdf2c84> as Run, None, t293),
    port("chop_shop_diversion_type::get_hud_icon_name", 0x00600fa0, &B294, 0, 0, eax_const::<0xdf12f0> as Run, None, t294),
    port("combat_tricks_diversion_type::can_start", 0x00601600, &B295, 0, 0, al_const::<0> as Run, None, t295),
    port("combat_tricks_diversion_type::get_start_message", 0x00601610, &B296, 2, 8, nop as Run, None, t296),
    port("combat_tricks_diversion_type::get_hud_icon_name", 0x00601650, &B297, 0, 0, eax_const::<0xdc5797> as Run, None, t297),
    port("diversion_type_info::update", 0x00602ab0, &B298, 0, 0, nop as Run, None, t298),
    port("diversion_type_info::get_hud_icon_name", 0x00602ac0, &B299, 0, 0, zero_eax as Run, None, t299),
    port("diversion_type_info::get_start_message", 0x00602b00, &B300, 2, 8, nop as Run, None, t300),
    port("driveby_diversion_type_info::get_type", 0x00606400, &B301, 0, 0, eax_const::<0x16> as Run, None, t301),
    port("driveby_diversion_type_info::get_hud_icon_name", 0x00606440, &B302, 0, 0, eax_const::<0xdf12f0> as Run, None, t302),
    port("exploration_diversion_type::get_type", 0x0060aa90, &B303, 0, 0, eax_const::<6> as Run, None, t303),
    port("exploration_diversion_type::can_start", 0x0060aaa0, &B304, 0, 0, al_const::<0> as Run, None, t304),
    port("exploration_diversion_type::create_instance", 0x0060aae0, &B305, 1, 4, zero_eax as Run, None, t305),
    port("exploration_diversion_type::get_hud_icon_name", 0x0060ab10, &B306, 0, 0, eax_const::<0xdf12f0> as Run, None, t306),
    port("firetruck_diversion_type_info::get_hud_icon_name", 0x0060b8f0, &B307, 0, 0, eax_const::<0xdf12f0> as Run, None, t307),
    port("flashing_diversion_type::get_type", 0x00611000, &B308, 0, 0, eax_const::<9> as Run, None, t308),
    port("flashing_diversion_type::get_hud_icon_name", 0x00611290, &B309, 0, 0, eax_const::<0xdf12f0> as Run, None, t309),
    port("flashing_diversion_type::get_start_mode", 0x006112c0, &B310, 0, 0, eax_const::<1> as Run, None, t310),
    port("hostage_diversion_type_info::get_hud_icon_name", 0x00612510, &B311, 0, 0, eax_const::<0xdf12f0> as Run, None, t311),
    port("mugging_diversion_type::get_type", 0x00614230, &B312, 0, 0, eax_const::<0xb> as Run, None, t312),
    port("mugging_diversion_type::can_start", 0x00614240, &B313, 0, 0, al_const::<0> as Run, None, t313),
    port("mugging_diversion_type::create_instance", 0x006142b0, &B314, 1, 4, zero_eax as Run, None, t314),
    port("mugging_diversion_type::get_hud_icon_name", 0x006142e0, &B315, 0, 0, eax_const::<0xdf12f0> as Run, None, t315),
    port("racing_diversion_type_info::get_type", 0x00615040, &B316, 0, 0, zero_eax as Run, None, t316),
    port("racing_diversion_type_info::get_hud_icon_name", 0x006150c0, &B317, 0, 0, eax_const::<0xdf12f0> as Run, None, t317),
    port("safecracking_diversion_type_info::get_hud_icon_name", 0x00615da0, &B318, 0, 0, eax_const::<0xdf12f0> as Run, None, t318),
    port("safecracking_diversion_type_info::can_start", 0x00615dc0, &B319, 0, 0, al_const::<0> as Run, None, t319),
    port("streaking_diversion_type::get_type", 0x006163f0, &B320, 0, 0, eax_const::<0xd> as Run, None, t320),
    port("streaking_diversion_type::get_hud_icon_name", 0x00616540, &B321, 0, 0, eax_const::<0xdf12f0> as Run, None, t321),
    port("stunt_back_seat_driver_diversion_type::get_type", 0x00617000, &B322, 0, 0, eax_const::<0x1b> as Run, None, t322),
    port("stunt_jumping_diversion_type::get_type", 0x00618610, &B323, 0, 0, eax_const::<0x1c> as Run, None, t323),
    port("stunt_drifting_diversion_type::get_type", 0x00619b90, &B324, 0, 0, eax_const::<0x1a> as Run, None, t324),
    port("stunt_hydraulics_diversion_type::get_type", 0x0061a450, &B325, 0, 0, eax_const::<0x1d> as Run, None, t325),
    port("stunt_low_flying_diversion_type::get_type", 0x0061be60, &B326, 0, 0, eax_const::<0x22> as Run, None, t326),
    port("stunt_near_miss_diversion_type::get_type", 0x0061c770, &B327, 0, 0, eax_const::<0x1e> as Run, None, t327),
    port("stunt_oncoming_traffic_diversion_type::get_type", 0x0061d3a0, &B328, 0, 0, eax_const::<0x1f> as Run, None, t328),
    port("stunt_peel_out_diversion_type::get_type", 0x0061db40, &B329, 0, 0, eax_const::<0x20> as Run, None, t329),
    port("stunt_wheels_diversion_type::get_type", 0x0061e290, &B330, 0, 0, eax_const::<0x21> as Run, None, t330),
    port("survival_diversion_type_info::update", 0x0061f0e0, &B331, 0, 0, nop as Run, None, t331),
    port("survival_diversion_type_info::get_hud_icon_name", 0x0061f110, &B332, 0, 0, eax_const::<0xdf12f0> as Run, None, t332),
    port("tagging_diversion_type_info::get_type", 0x006207c0, &B333, 0, 0, eax_const::<0x17> as Run, None, t333),
    port("tagging_diversion_type_info::get_hud_icon_name", 0x006207f0, &B334, 0, 0, eax_const::<0xdf12f0> as Run, None, t334),
    port("tagging_diversion_type_info::create_instance", 0x00620840, &B335, 1, 4, zero_eax as Run, None, t335),
    port("tagging_diversion_type_info::on_game_event_18", 0x00621890, &B336, 1, 4, r_621890 as Run, Some(w_621890 as Writes), t336),
    port("taunting_diversion_type::get_start_message", 0x00627310, &B337, 2, 8, nop as Run, None, t337),
    port("taunting_diversion_type::get_type", 0x00627320, &B338, 0, 0, eax_const::<0x15> as Run, None, t338),
    port("taunting_diversion_type::get_hud_icon_name", 0x00627360, &B339, 0, 0, eax_const::<0xdf12f0> as Run, None, t339),
    port("taxi_diversion_type_info::get_type", 0x00628e70, &B340, 0, 0, eax_const::<0x10> as Run, None, t340),
    port("taxi_diversion_type_info::get_hud_icon_name", 0x00628fa0, &B341, 0, 0, eax_const::<0xdf12f0> as Run, None, t341),
    port("tow_truck_diversion_type_info::get_type", 0x0062db80, &B342, 0, 0, eax_const::<0x11> as Run, None, t342),
    port("tow_truck_diversion_type_info::get_hud_icon_name", 0x0062e940, &B343, 0, 0, eax_const::<0xdf12f0> as Run, None, t343),
    port("vehicle_surfing_style_two_diversion_type_info::get_type", 0x00633140, &B344, 0, 0, eax_const::<0x24> as Run, None, t344),
    port("vehicle_surfing_style_two_diversion_type_info::can_start", 0x006331d0, &B345, 0, 0, al_const::<0> as Run, None, t345),
    port("vehicle_surfing_style_two_diversion_type_info::get_hud_icon_name", 0x00633270, &B346, 0, 0, eax_const::<0xdf12f0> as Run, None, t346),
    port("poker_diversion_type_info::update", 0x00635730, &B347, 0, 0, nop as Run, None, t347),
    port("poker_diversion_type_info::get_hud_icon_name", 0x006357c0, &B348, 0, 0, eax_const::<0xdf12f0> as Run, None, t348),
    port("poker_diversion_type_info::get_type", 0x006358b0, &B349, 0, 0, eax_const::<0xc> as Run, None, t349),
    port("whoring_diversion_type_info::get_type", 0x00638ae0, &B350, 0, 0, eax_const::<0x12> as Run, None, t350),
    port("whoring_diversion_type_info::get_hud_icon_name", 0x00638d30, &B351, 0, 0, eax_const::<0xdf12f0> as Run, None, t351),
    port("whoring_diversion_type_info::create_instance", 0x00638d40, &B352, 1, 4, zero_eax as Run, None, t352),
    port("windshield_cannon_diversion_type_info::get_type", 0x0063a7b0, &B353, 0, 0, eax_const::<0x13> as Run, None, t353),
    port("hitman_diversion_type::create_instance", 0x0063ab50, &B354, 1, 4, zero_eax as Run, None, t354),
    port("hitman_diversion_type::get_type", 0x0063ab60, &B355, 0, 0, eax_const::<0x14> as Run, None, t355),
    port("hitman_diversion_type::get_hud_icon_name", 0x0063ab90, &B356, 0, 0, eax_const::<0xdc5797> as Run, None, t356),
    port("cat_and_mouse_diversion_type_info::get_type", 0x00641730, &B357, 0, 0, eax_const::<0x19> as Run, None, t357),
    port("cat_and_mouse_diversion_type_info::get_hud_icon_name", 0x00641760, &B358, 0, 0, eax_const::<0xdc5797> as Run, None, t358),
    port("cat_and_mouse_diversion_type_info::get_start_mode", 0x006417a0, &B359, 0, 0, eax_const::<2> as Run, None, t359),
    port("cat_and_mouse_diversion_instance::ctor", 0x006417e0, &B360, 0, 0, r_6417e0 as Run, Some(w_6417e0 as Writes), t360),
    port("death_tag_diversion_type_info::get_type", 0x006457d0, &B361, 0, 0, eax_const::<0x18> as Run, None, t361),
    port("death_tag_diversion_type_info::get_hud_icon_name", 0x00645800, &B362, 0, 0, eax_const::<0xdc5797> as Run, None, t362),
    port("death_tag_diversion_type_info::get_start_mode", 0x00645840, &B363, 0, 0, eax_const::<2> as Run, None, t363),
    port("pimping_info::get_activity_type", 0x00655ca0, &B364, 0, 0, eax_const::<8> as Run, None, t364),
    port("vandalism_news_spawn_group::on_table_loaded", 0x00667cf0, &B365, 0, 0, nop as Run, None, t365),
    port("vandalism_info::get_activity_type", 0x00667d30, &B366, 0, 0, eax_const::<0x10> as Run, None, t366),
    port("escort_spawn_group::on_table_loaded", 0x00678590, &B367, 0, 0, nop as Run, None, t367),
    port("escort_enemy_spawn_group::on_table_loaded", 0x006785b0, &B368, 0, 0, nop as Run, None, t368),
    port("critical_locked_msg::is_topmost", 0x0069d1f0, &B369, 0, 0, al_const::<1> as Run, None, t369),
    port("character_table_load", 0x006dfc60, &B370, 1, 4, r_6dfc60 as Run, Some(w_6dfc60 as Writes), t370),
    port("string_hash_table::vf0", 0x006f2d30, &B371, 0, 0, nop as Run, None, t371),
    port("vhashtable<>::clear_buckets_2", 0x0070bc20, &B372, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t372),
    port("vhashtable<>::clear_buckets_3", 0x0070be50, &B373, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t373),
    port("ovl_buysell::show", 0x0075c640, &B374, 0, 0, nop as Run, None, t374),
    port("ovl_buysell::hide", 0x0075c650, &B375, 0, 0, nop as Run, None, t375),
    port("ovl_buysell::update", 0x0075c660, &B376, 0, 0, al_const::<0> as Run, None, t376),
    port("ovl_buysell::render", 0x0075c670, &B377, 0, 0, nop as Run, None, t377),
    port("thin_control::release", 0x00767ad0, &B378, 0, 0, nop as Run, None, t378),
    port("thin_control_text::release", 0x007683b0, &B379, 0, 0, r_7683b0 as Run, Some(w_7683b0 as Writes), t379),
    port("thin_control_text_button::release", 0x007686b0, &B380, 0, 0, r_7686b0 as Run, Some(w_7686b0 as Writes), t380),
    port("thin_control_toggle_button::release", 0x00768b70, &B381, 0, 0, r_768b70 as Run, Some(w_768b70 as Writes), t381),
    port("thin_control_slider::release", 0x00769470, &B382, 0, 0, r_769470 as Run, Some(w_769470 as Writes), t382),
    port("thin_control_list_box::release", 0x0076a800, &B383, 0, 0, r_76a800 as Run, Some(w_76a800 as Writes), t383),
    port("options_apply_quality_preset", 0x00773e00, &B384, 0, 0, r_773e00 as Run, Some(w_773e00 as Writes), t384),
    port("help_screen_categories_parse_displaymask", 0x0077dba0, &B385, 0, 0, r_77dba0 as Run, None, t385),
    port("cell_spawn_group::on_table_loaded", 0x00784550, &B386, 0, 0, nop as Run, None, t386),
    port("cell_spawn_group::is_active", 0x00784600, &B387, 0, 0, r_784600 as Run, None, t387),
    port("hud_timer_hide_impl", 0x0078e400, &B388, 0, 0, r_78e400 as Run, Some(w_78e400 as Writes), t388),
    port("multi_core_object_registration::on_ownership_given", 0x00827790, &B389, 2, 8, nop as Run, None, t389),
    port("detour_multi_registration::on_unregister", 0x00827c10, &B390, 1, 4, nop as Run, None, t390),
    port("detour_multi_registration::on_net_tick", 0x00827c90, &B391, 0, 0, nop as Run, None, t391),
    port("detour_multi_registration::remote_do_frame", 0x00827ca0, &B392, 1, 4, nop as Run, None, t392),
    port("detour_multi_registration::read_state_delta", 0x00827ce0, &B393, 5, 20, r_827ce0 as Run, None, t393),
    port("detour_multi_registration::on_state_sent", 0x00827d30, &B394, 4, 16, nop as Run, None, t394),
    port("detour_multi_registration::write_migration_data", 0x00827d80, &B395, 3, 12, nop as Run, None, t395),
    port("coop_wait_text", 0x008309f0, &B396, 0, 0, r_8309f0 as Run, Some(w_8309f0 as Writes), t396),
    port("multi_custom_set_time_limit_impl", 0x008429f0, &B397, 0, 0, r_8429f0 as Run, Some(w_8429f0 as Writes), t397),
    port("multi_custom_set_score_limit_impl", 0x00842a80, &B398, 0, 0, r_842a80 as Run, Some(w_842a80 as Writes), t398),
    port("item_multi_registration::write_destroy_data", 0x00843ba0, &B399, 3, 12, nop as Run, None, t399),
    port("item_multi_registration::on_net_tick", 0x00843c10, &B400, 0, 0, nop as Run, None, t400),
    port("item_multi_registration::on_state_sent", 0x00845470, &B401, 4, 16, r_845470 as Run, Some(w_845470 as Writes), t401),
    port("item_multi_registration::write_migration_data", 0x00845830, &B402, 3, 12, r_845830 as Run, Some(w_845830 as Writes), t402),
    port("mover_multi_registration::write_destroy_data", 0x0084bf70, &B403, 3, 12, nop as Run, None, t403),
    port("mover_multi_registration::on_net_tick", 0x0084c090, &B404, 0, 0, nop as Run, None, t404),
    port("mover_multi_registration::can_migrate", 0x0084e000, &B405, 1, 4, r_84e000 as Run, None, t405),
    port("mover_multi_registration::write_migration_data", 0x0084e120, &B406, 3, 12, r_84e120 as Run, Some(w_84e120 as Writes), t406),
    port("npc_multi_registration::write_destroy_data", 0x008524a0, &B407, 3, 12, nop as Run, None, t407),
    port("npc_multi_registration::on_state_sent", 0x00857990, &B408, 4, 16, r_857990 as Run, Some(w_857990 as Writes), t408),
    port("pipebomb_multi_registration::write_destroy_data", 0x0085cc40, &B409, 3, 12, nop as Run, None, t409),
    port("pipebomb_multi_registration::on_net_tick", 0x0085cc90, &B410, 0, 0, nop as Run, None, t410),
    port("pipebomb_multi_registration::on_state_sent", 0x0085e1b0, &B411, 4, 16, r_85e1b0 as Run, Some(w_85e1b0 as Writes), t411),
    port("pipebomb_multi_registration::write_migration_data", 0x0085e370, &B412, 3, 12, nop as Run, None, t412),
    port("player_multi_registration::write_destroy_data", 0x00863340, &B413, 3, 12, nop as Run, None, t413),
    port("player_multi_registration::on_state_sent", 0x008699b0, &B414, 4, 16, r_8699b0 as Run, Some(w_8699b0 as Writes), t414),
    port("player_multi_registration::on_owner_left", 0x00869a70, &B415, 1, 4, eax_const::<1> as Run, None, t415),
    port("player_multi_registration::on_ownership_taken", 0x00869a90, &B416, 3, 12, r_869a90 as Run, Some(w_869a90 as Writes), t416),
    port("player_multi_registration::write_migration_data", 0x00869ae0, &B417, 3, 12, nop as Run, None, t417),
    port("prop_multi_registration::write_destroy_data", 0x0086c090, &B418, 3, 12, nop as Run, None, t418),
    port("prop_multi_registration::on_net_tick", 0x0086c0e0, &B419, 0, 0, nop as Run, None, t419),
    port("prop_multi_registration::on_state_sent", 0x0086d430, &B420, 4, 16, r_86d430 as Run, Some(w_86d430 as Writes), t420),
    port("prop_multi_registration::on_owner_left", 0x0086d4f0, &B421, 1, 4, r_86d4f0 as Run, None, t421),
    port("prop_multi_registration::can_migrate", 0x0086d530, &B422, 1, 4, r_86d530 as Run, None, t422),
    port("prop_multi_registration::write_migration_data", 0x0086d5b0, &B423, 3, 12, nop as Run, None, t423),
    port("rocket_multi_registration::write_destroy_data", 0x0086f2b0, &B424, 3, 12, nop as Run, None, t424),
    port("rocket_multi_registration::on_net_tick", 0x0086f300, &B425, 0, 0, nop as Run, None, t425),
    port("rocket_multi_registration::write_migration_data", 0x008705f0, &B426, 3, 12, nop as Run, None, t426),
    port("trigger_multi_registration::on_unregister", 0x00871fe0, &B427, 1, 4, nop as Run, None, t427),
    port("trigger_multi_registration::write_destroy_data", 0x00871ff0, &B428, 3, 12, nop as Run, None, t428),
    port("trigger_multi_registration::on_net_tick", 0x00872040, &B429, 0, 0, nop as Run, None, t429),
    port("trigger_multi_registration::remote_do_frame", 0x00872050, &B430, 1, 4, nop as Run, None, t430),
    port("trigger_multi_registration::on_state_sent", 0x00872680, &B431, 4, 16, nop as Run, None, t431),
    port("trigger_multi_registration::can_migrate", 0x00872700, &B432, 1, 4, al_const::<0> as Run, None, t432),
    port("trigger_multi_registration::on_ownership_taken", 0x00872710, &B433, 3, 12, nop as Run, None, t433),
    port("trigger_multi_registration::write_migration_data", 0x00872720, &B434, 3, 12, nop as Run, None, t434),
    port("vehicle_multi_registration::write_destroy_data", 0x00874270, &B435, 3, 12, nop as Run, None, t435),
    port("vehicle_multi_registration::on_net_tick", 0x008742c0, &B436, 0, 0, r_8742c0 as Run, Some(w_8742c0 as Writes), t436),
    port("mp_bh_custom_match_category::load_from_table", 0x00896330, &B437, 1, 4, nop as Run, None, t437),
    port("detour_hull_object::get_local_bbox", 0x009195d0, &B438, 2, 8, r_9195d0 as Run, Some(w_9195d0 as Writes), t438),
    port("inventory_item::get_world_bbox", 0x0091d6d0, &B439, 2, 8, nop as Run, None, t439),
    port("inventory_item::get_local_bbox", 0x0091d6e0, &B440, 2, 8, nop as Run, None, t440),
    port("notoriety_spawn_group_police_heli::on_table_loaded", 0x00923da0, &B441, 0, 0, nop as Run, None, t441),
    port("notoriety_spawn_group_police::on_table_loaded", 0x00923f90, &B442, 0, 0, nop as Run, None, t442),
    port("notoriety_spawn_group_gang::on_table_loaded", 0x00924220, &B443, 0, 0, nop as Run, None, t443),
    port("object_item::get_local_bbox", 0x009335c0, &B444, 2, 8, r_9335c0 as Run, Some(w_9335c0 as Writes), t444),
    port("object_prop::get_local_bbox", 0x00934a70, &B445, 2, 8, r_934a70 as Run, Some(w_934a70 as Writes), t445),
    port("human::get_local_bbox", 0x00960200, &B446, 2, 8, r_960200 as Run, Some(w_960200 as Writes), t446),
    port("human_party_root_leader", 0x0098e750, &B447, 0, 0, r_98e750 as Run, None, t447),
    port("rl_container_inst::compute_pass_data", 0x009af500, &B448, 3, 12, al_const::<0> as Run, None, t448),
    port("rl_container_inst::pre_render", 0x009af510, &B449, 1, 4, nop as Run, None, t449),
    port("human_has_thrown_weapon", 0x009c6060, &B450, 0, 0, r_9c6060 as Run, None, t450),
    port("vhashtable<>::clear_buckets_4", 0x009fe7d0, &B451, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t451),
    port("vhashtable<>::clear_buckets_5", 0x009feb30, &B452, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t452),
    port("script_object::dtor_2", 0x00a57b30, &B453, 0, 0, r_a57b30 as Run, Some(w_a57b30 as Writes), t453),
    port("script_interior::parse", 0x00a58aa0, &B454, 1, 4, al_const::<0> as Run, None, t454),
    port("vhashtable<>::clear_buckets_6", 0x00a5fdf0, &B455, 0, 0, r_cb32_0 as Run, Some(w_cb4 as Writes), t455),
    port("script_positionable_object::set_transform", 0x00a63700, &B456, 3, 12, r_a63700 as Run, Some(w_a63700 as Writes), t456),
    port("script_positionable_object::copy_transform_from", 0x00a63740, &B457, 1, 4, r_a63740 as Run, Some(w_a63740 as Writes), t457),
    port("script_respawn::parse", 0x00a64240, &B458, 1, 4, al_const::<0> as Run, None, t458),
    port("script_searchlight::on_object_spawned", 0x00a64370, &B459, 2, 8, nop as Run, None, t459),
    port("vhashtable<>::clear_buckets_7", 0x00a77940, &B460, 0, 0, r_cb64 as Run, Some(w_cb8 as Writes), t460),
    port("r_tree_inst::compute_pass_data", 0x00a8da50, &B461, 3, 12, al_const::<1> as Run, None, t461),
    port("r_tree_inst::pre_render", 0x00a8da60, &B462, 1, 4, nop as Run, None, t462),
    port("r_tree_inst::prepare_pass", 0x00a8da70, &B463, 3, 12, al_const::<1> as Run, None, t463),
    port("r_tree_inst::render_shadow", 0x00a8da80, &B464, 2, 8, nop as Run, None, t464),
    port("r_tree_inst::is_ready", 0x00a8da90, &B465, 0, 0, al_const::<1> as Run, None, t465),
    port("r_tree_inst::get_env_handle", 0x00a8daa0, &B466, 0, 0, zero_eax as Run, None, t466),
    port("vhashtable<>::clear_buckets_8", 0x00ad33d0, &B467, 0, 0, r_cb16 as Run, Some(w_cb2 as Writes), t467),
    port("vehicle_light_manager::get_light_name", 0x00ad4d60, &B468, 0, 0, zero_eax as Run, None, t468),
    port("vehicle_light_manager::update", 0x00ad4f50, &B469, 1, 4, nop as Run, None, t469),
    port("vehicle_light_manager::update_time_of_day", 0x00ad5780, &B470, 1, 4, r_ad5780 as Run, Some(w_ad5780 as Writes), t470),
    port("vehicle_headlight_manager::get_light_name", 0x00ad5990, &B471, 0, 0, eax_const::<0xe4605c> as Run, None, t471),
    port("vehicle_brakelight_manager::get_light_name", 0x00ad5e00, &B472, 0, 0, eax_const::<0xe4606c> as Run, None, t472),
    port("vehicle_brakelight_manager::get_light_type", 0x00ad5e10, &B473, 0, 0, eax_const::<2> as Run, None, t473),
    port("vehicle_turn_signal_manager::tag_prefix", 0x00ad60c0, &B474, 0, 0, eax_const::<3> as Run, None, t474),
    port("vehicle_reverselight_manager::get_light_name", 0x00ad61d0, &B475, 0, 0, eax_const::<0xe460a0> as Run, None, t475),
    port("vehicle_sirenlight_manager::get_light_name", 0x00ad64e0, &B476, 0, 0, eax_const::<0xe460b0> as Run, None, t476),
    port("vehicle_sirenlight_manager::get_light_type", 0x00ad64f0, &B477, 0, 0, eax_const::<6> as Run, None, t477),
    port("vehicle_runninglight_manager::get_light_name", 0x00ad6630, &B478, 0, 0, eax_const::<0xe460b8> as Run, None, t478),
    port("vehicle_runninglight_manager::tag_prefix", 0x00ad6650, &B479, 0, 0, zero_eax as Run, None, t479),
    port("vehicle_aerolight_manager::get_light_name", 0x00ad6940, &B480, 0, 0, eax_const::<0xe460cc> as Run, None, t480),
    port("vehicle_aerolight_manager::get_light_type", 0x00ad6950, &B481, 0, 0, eax_const::<8> as Run, None, t481),
    port("r_dynamic_license_plate_inst::casts_shadow", 0x00aefe00, &B482, 0, 0, al_const::<0> as Run, None, t482),
    port("r_dynamic_license_plate_inst::compute_pass_data", 0x00aefe10, &B483, 3, 12, al_const::<1> as Run, None, t483),
    port("r_dynamic_license_plate_inst::prepare_pass", 0x00aefe20, &B484, 3, 12, al_const::<1> as Run, None, t484),
    port("r_dynamic_license_plate_inst::render_shadow", 0x00aefe30, &B485, 2, 8, nop as Run, None, t485),
    port("r_dynamic_license_plate_inst::is_ready", 0x00aefe40, &B486, 0, 0, al_const::<1> as Run, None, t486),
    port("r_dynamic_license_plate_inst::get_env_handle", 0x00aefe50, &B487, 0, 0, zero_eax as Run, None, t487),
    port("r_dynamic_license_plate_inst::pre_render", 0x00af07b0, &B488, 1, 4, nop as Run, None, t488),
    port("vehicle_obstacle_records_add", 0x00b2e6e0, &B489, 0, 0, r_b2e6e0 as Run, Some(w_b2e6e0 as Writes), t489),
    port("ammo_store_entry_add", 0x00b68c20, &B490, 1, 4, r_b68c20 as Run, Some(w_b68c20 as Writes), t490),
    port("ammo_store_remove", 0x00b69190, &B491, 1, 4, r_b69190 as Run, Some(w_b69190 as Writes), t491),
    port("vehicle_weapon_find", 0x00b6b500, &B492, 0, 0, r_b6b500 as Run, None, t492),
    port("weapon_info_is_thrown", 0x00b72c40, &B493, 0, 0, r_b72c40 as Run, None, t493),
    port("weapon_has_unlimited_ammo", 0x00b72ce0, &B494, 1, 0, r_b72ce0 as Run, None, t494),
    port("weapon_instance_revives_humans", 0x00b73ca0, &B495, 0, 0, r_b73ca0 as Run, None, t495),
    port("r_dynamic_water_detailed_inst::casts_shadow", 0x00b75bd0, &B496, 0, 0, al_const::<0> as Run, None, t496),
    port("r_dynamic_water_detailed_inst::compute_pass_data", 0x00b75be0, &B497, 3, 12, al_const::<1> as Run, None, t497),
    port("r_dynamic_water_detailed_inst::prepare_pass", 0x00b75bf0, &B498, 3, 12, al_const::<1> as Run, None, t498),
    port("r_dynamic_water_detailed_inst::render_shadow", 0x00b75c00, &B499, 2, 8, nop as Run, None, t499),
    port("r_dynamic_water_simple_inst::casts_shadow", 0x00b75c40, &B500, 0, 0, al_const::<0> as Run, None, t500),
    port("r_dynamic_water_simple_inst::compute_pass_data", 0x00b75c50, &B501, 3, 12, al_const::<1> as Run, None, t501),
    port("r_dynamic_water_simple_inst::prepare_pass", 0x00b75c60, &B502, 3, 12, al_const::<1> as Run, None, t502),
    port("r_dynamic_water_simple_inst::render_shadow", 0x00b75c70, &B503, 2, 8, nop as Run, None, t503),
    port("r_dynamic_water_simple_inst::is_ready", 0x00b75c80, &B504, 0, 0, al_const::<1> as Run, None, t504),
    port("r_dynamic_water_simple_inst::get_env_handle", 0x00b75c90, &B505, 0, 0, load_ecx::<0x1c> as Run, None, t505),
    port("r_distant_vehicle_headlight_inst::casts_shadow", 0x00b7ee50, &B506, 0, 0, al_const::<0> as Run, None, t506),
    port("r_distant_vehicle_headlight_inst::compute_pass_data", 0x00b7ee60, &B507, 3, 12, al_const::<1> as Run, None, t507),
    port("r_distant_vehicle_headlight_inst::prepare_pass", 0x00b7ee70, &B508, 3, 12, al_const::<1> as Run, None, t508),
    port("r_distant_vehicle_headlight_inst::render_shadow", 0x00b7ee80, &B509, 2, 8, nop as Run, None, t509),
    port("r_distant_vehicle_headlight_inst::is_ready", 0x00b7ee90, &B510, 0, 0, al_const::<1> as Run, None, t510),
    port("r_distant_vehicle_headlight_inst::get_env_handle", 0x00b7eea0, &B511, 0, 0, zero_eax as Run, None, t511),
    port("vhashtable<>::clear_buckets_9", 0x00b8a070, &B512, 0, 0, r_cb32_ffff as Run, Some(w_cb4 as Writes), t512),
    port("vint_subscription_event_name", 0x00b8d100, &B513, 0, 0, r_b8d100 as Run, None, t513),
    port("vint_parse_decimal_wide", 0x00b91e70, &B514, 0, 0, r_b91e70 as Run, None, t514),
    port("vint_datagroup_close", 0x00b93b40, &B515, 0, 0, r_b93b40 as Run, Some(w_b93b40 as Writes), t515),
    port("vint_datagroup_first_item", 0x00b93da0, &B516, 0, 0, r_b93da0 as Run, None, t516),
    port("vint_datagroup_queue_item", 0x00b942d0, &B517, 0, 0, r_b942d0 as Run, Some(w_b942d0 as Writes), t517),
    port("vint_datagroup_splice_open_to_queue", 0x00b94380, &B518, 0, 0, r_b94380 as Run, Some(w_b94380 as Writes), t518),
    port("vint_element_base::get_tint", 0x00b967a0, &B519, 2, 8, r_b967a0 as Run, Some(w_b967a0 as Writes), t519),
    port("vint_element_base::set_unscaled_size", 0x00b97090, &B520, 2, 8, al_const::<0> as Run, None, t520),
    port("vint_object_register_handle", 0x00b97220, &B521, 0, 0, r_b97220 as Run, Some(w_b97220 as Writes), t521),
    port("vint_object_base::update", 0x00b97560, &B522, 0, 0, nop as Run, None, t522),
    port("vint_project_find_resolution", 0x00b9ac60, &B523, 1, 0, r_b9ac60 as Run, None, t523),
    port("vint_object_base::on_destroy", 0x00b9c240, &B524, 0, 0, nop as Run, None, t524),
    port("vint_element_base::on_destroy", 0x00b9c320, &B525, 0, 0, nop as Run, None, t525),
    port("vint_element_base::post_render", 0x00b9c340, &B526, 0, 0, nop as Run, None, t526),
    port("vint_element_group::is_type", 0x00b9c3c0, &B527, 1, 4, r_b9c3c0 as Run, None, t527),
    port("vint_element_group::on_destroy", 0x00b9c510, &B528, 0, 0, nop as Run, None, t528),
    port("vint_element_group::render", 0x00b9c530, &B529, 1, 4, nop as Run, None, t529),
    port("vint_element_group::set_screen_size", 0x00b9c8a0, &B530, 2, 8, al_const::<0> as Run, None, t530),
    port("vint_element_group::set_screen_se", 0x00b9c930, &B531, 2, 8, al_const::<0> as Run, None, t531),
    port("vint_element_group::set_screen_nw", 0x00b9c9c0, &B532, 2, 8, al_const::<0> as Run, None, t532),
    port("vint_object_factory_imp<>::get", 0x00b9ccb0, &B533, 1, 4, r_b9ccb0 as Run, None, t533),
    port("vint_object_animation::update", 0x00b9ced0, &B534, 0, 0, nop as Run, None, t534),
    port("vint_object_animation::on_destroy", 0x00b9cee0, &B535, 0, 0, nop as Run, None, t535),
    port("vint_object_animation::get_target_handle", 0x00b9d2d0, &B536, 2, 8, al_const::<0> as Run, None, t536),
    port("vint_object_factory_imp<>::get_2", 0x00b9d5b0, &B537, 1, 4, r_b9d5b0 as Run, None, t537),
    port("vint_type_by_index", 0x00b9d760, &B538, 0, 0, r_b9d760 as Run, None, t538),
    port("vint_object_tween_ctor", 0x00b9e6b0, &B539, 0, 0, r_b9e6b0 as Run, Some(w_b9e6b0 as Writes), t539),
    port("vint_object_tween::update", 0x00b9e9f0, &B540, 0, 0, nop as Run, None, t540),
    port("vint_element_bitmap::is_type", 0x00ba0c50, &B541, 1, 4, r_ba0c50 as Run, None, t541),
    port("vint_element_bitmap::update", 0x00ba0dc0, &B542, 0, 0, nop as Run, None, t542),
    port("vint_element_bitmap::get_image_crc", 0x00ba0ff0, &B543, 2, 8, al_const::<0> as Run, None, t543),
    port("vint_object_factory_imp<>::get_4", 0x00ba1700, &B544, 1, 4, r_ba1700 as Run, None, t544),
    port("vint_element_gradient::is_type", 0x00ba18e0, &B545, 1, 4, r_ba18e0 as Run, None, t545),
    port("vint_element_gradient::update", 0x00ba1a80, &B546, 0, 0, nop as Run, None, t546),
    port("vint_element_gradient::get_gradient_nw", 0x00ba1c30, &B547, 2, 8, r_ba1c30 as Run, Some(w_ba1c30 as Writes), t547),
    port("vint_element_gradient::get_gradient_ne", 0x00ba1d00, &B548, 2, 8, r_ba1d00 as Run, Some(w_ba1d00 as Writes), t548),
    port("vint_element_gradient::get_gradient_sw", 0x00ba1dd0, &B549, 2, 8, r_ba1dd0 as Run, Some(w_ba1dd0 as Writes), t549),
    port("vint_element_gradient::get_gradient_se", 0x00ba1ea0, &B550, 2, 8, r_ba1ea0 as Run, Some(w_ba1ea0 as Writes), t550),
    port("vint_object_factory_imp<>::get_5", 0x00ba2620, &B551, 1, 4, r_ba2620 as Run, None, t551),
    port("vint_element_text::is_type", 0x00ba2820, &B552, 1, 4, r_ba2820 as Run, None, t552),
    port("vint_element_text::update", 0x00ba2c30, &B553, 0, 0, nop as Run, None, t553),
    port("vint_object_factory_imp<>::get_6", 0x00ba45a0, &B554, 1, 4, r_ba45a0 as Run, None, t554),
    port("vint_element_point::is_type", 0x00ba46e0, &B555, 1, 4, r_ba46e0 as Run, None, t555),
    port("vint_element_point::on_destroy", 0x00ba47f0, &B556, 0, 0, nop as Run, None, t556),
    port("vint_element_point::update", 0x00ba4800, &B557, 0, 0, nop as Run, None, t557),
    port("vint_element_point::render", 0x00ba4810, &B558, 1, 4, nop as Run, None, t558),
    port("vint_element_point::set_screen_size", 0x00ba4890, &B559, 2, 8, al_const::<0> as Run, None, t559),
    port("vint_object_factory_imp<>::get_7", 0x00ba4b70, &B560, 1, 4, r_ba4b70 as Run, None, t560),
    port("vint_element_clip::is_type", 0x00ba4c90, &B561, 1, 4, r_ba4c90 as Run, None, t561),
    port("vint_element_clip::on_destroy", 0x00ba4db0, &B562, 0, 0, nop as Run, None, t562),
    port("vint_element_clip::update", 0x00ba4dc0, &B563, 0, 0, nop as Run, None, t563),
    port("vint_object_factory_imp<>::get_8", 0x00ba5370, &B564, 1, 4, r_ba5370 as Run, None, t564),
    port("vint_element_bitmap_circle::is_type", 0x00ba54c0, &B565, 1, 4, r_ba54c0 as Run, None, t565),
    port("vint_element_bitmap_circle::update", 0x00ba5650, &B566, 0, 0, nop as Run, None, t566),
    port("vint_object_factory_imp<>::get_9", 0x00ba6090, &B567, 1, 4, r_ba6090 as Run, None, t567),
    port("vint_element_sr2_map::is_type", 0x00ba61d0, &B568, 1, 4, r_ba61d0 as Run, None, t568),
    port("vint_element_sr2_map::on_destroy", 0x00ba6300, &B569, 0, 0, nop as Run, None, t569),
    port("vint_element_sr2_map::update", 0x00ba6310, &B570, 0, 0, nop as Run, None, t570),
    port("vint_object_factory_imp<>::get_10", 0x00ba6860, &B571, 1, 4, r_ba6860 as Run, None, t571),
    port("vint_element_video::is_type", 0x00ba6980, &B572, 1, 4, r_ba6980 as Run, None, t572),
    port("vint_element_video::on_destroy", 0x00ba6a90, &B573, 0, 0, nop as Run, None, t573),
    port("vint_element_video::update", 0x00ba6aa0, &B574, 0, 0, nop as Run, None, t574),
    port("vint_element_video::post_render", 0x00ba6b80, &B575, 0, 0, nop as Run, None, t575),
    port("vint_object_factory_imp<>::get_11", 0x00ba6f40, &B576, 1, 4, r_ba6f40 as Run, None, t576),
    port("rl_morph_mesh_inst::is_ready", 0x00bdb440, &B577, 0, 0, al_const::<0> as Run, None, t577),
    port("string_pool::mark", 0x00c07030, &B578, 0, 0, r_c07030 as Run, Some(w_c07030 as Writes), t578),
    port("string_hash_table::mark", 0x00c078b0, &B579, 0, 0, r_c078b0 as Run, Some(w_c078b0 as Writes), t579),
    port("packfile_fixup_string_ptrs", 0x00c0a1e0, &B580, 0, 0, r_c0a1e0 as Run, Some(w_c0a1e0 as Writes), t580),
    port("packfile_compute_entry_sectors", 0x00c0a290, &B581, 0, 0, r_c0a290 as Run, Some(w_c0a290 as Writes), t581),
    port("script_thread_is_suspended", 0x00cdfd40, &B582, 1, 0, r_cdfd40 as Run, None, t582),
    port("rl_lod_mesh_inst::get_lod", 0x00d09810, &B583, 0, 0, r_d09810 as Run, None, t583),
];

// --- install ------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
/// Addresses port_weapons_audio.rs also ports (with its own shadows); left to that module.
const OWNED_ELSEWHERE: [usize; 5] = [0x00b6_93e0, 0x0046_60b0, 0x00b6_8c20, 0x00b6_9190, 0x00b7_2c40];

pub fn install() {
    let mut ok = 0;
    for (id, port) in PORTS.iter().enumerate() {
        if port.name.starts_with("vhashtable<>::clear_buckets") {
            // Bulk clears: shadow only the first BIG_SHADOW_CALLS calls.
            BIG[id].store(true, Ordering::Relaxed);
        }
        if OWNED_ELSEWHERE.contains(&port.addr) {
            continue;
        }
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, port.body, &[]) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_leaves: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, port.body, port.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_leaves: replaced {ok}/{} with Rust", COUNT - OWNED_ELSEWHERE.len()));
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
    let ov = SPAN_OVERFLOW.load(Ordering::Relaxed);
    if parts.is_empty() {
        String::new()
    } else if ov > 0 {
        format!("port_leaves: {} (write dry runs over 4 spans: {ov})", parts.join(", "))
    } else {
        format!("port_leaves: {}", parts.join(", "))
    }
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
const B255: [u8; 58] = [
    0x8A, 0x01, 0x3C, 0xFF, 0x74, 0x21, 0x8B, 0x09, 0xC1, 0xE9, 0x08, 0x74, 0x1A, 0x0F, 0xB6, 0xC0,
    0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x09, 0x83, 0xB8,
    0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x66, 0x8B, 0x80, 0x8E, 0xBA, 0xEE,
    0x00, 0x66, 0xC1, 0xE8, 0x0B, 0xF6, 0xD0, 0x24, 0x01, 0xC3,
];
const B256: [u8; 51] = [
    0x80, 0x3D, 0x5F, 0x82, 0xE8, 0x00, 0x00, 0x74, 0x1E, 0x33, 0xC0, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x0C, 0x85, 0x14, 0x39, 0xF5, 0x00, 0x85, 0xC9, 0x74, 0x04, 0x3B, 0xCA, 0x74, 0x0B, 0x83,
    0xC0, 0x01, 0x83, 0xF8, 0x03, 0x7C, 0xE9, 0x32, 0xC0, 0xC3, 0x85, 0xF6, 0x74, 0x02, 0x89, 0x06,
    0xB0, 0x01, 0xC3,
];
const B257: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B258: [u8; 5] = [
    0x8B, 0x41, 0x3C, 0xC3, 0xCC,
];
const B259: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B260: [u8; 41] = [
    0x8B, 0x44, 0x24, 0x04, 0x3D, 0x00, 0x94, 0x35, 0x77, 0x7E, 0x09, 0xC7, 0x01, 0x00, 0x94, 0x35,
    0x77, 0xC2, 0x04, 0x00, 0x3D, 0x00, 0x6C, 0xCA, 0x88, 0x7D, 0x09, 0xC7, 0x01, 0x00, 0x6C, 0xCA,
    0x88, 0xC2, 0x04, 0x00, 0x89, 0x01, 0xC2, 0x04, 0x00,
];
const B261: [u8; 6] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B262: [u8; 34] = [
    0x8B, 0x01, 0x8B, 0x51, 0x04, 0xC1, 0xE0, 0x04, 0x05, 0x38, 0xEA, 0xF9, 0x00, 0x89, 0x10, 0x8B,
    0x51, 0x08, 0x89, 0x50, 0x04, 0x8B, 0x51, 0x0C, 0x89, 0x50, 0x08, 0x8B, 0x49, 0x10, 0x89, 0x48,
    0x0C, 0xC3,
];
const B263: [u8; 59] = [
    0x85, 0xD2, 0x7E, 0x36, 0x83, 0xC1, 0x08, 0x56, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x41, 0xF8, 0x8B, 0x71, 0xFC, 0xC1, 0xE0, 0x04, 0x05, 0x38, 0xEA, 0xF9, 0x00, 0x89, 0x30,
    0x8B, 0x31, 0x89, 0x70, 0x04, 0x8B, 0x71, 0x04, 0x89, 0x70, 0x08, 0x8B, 0x71, 0x08, 0x83, 0xC1,
    0x14, 0x83, 0xEA, 0x01, 0x89, 0x70, 0x0C, 0x75, 0xD7, 0x5E, 0xC3,
];
const B264: [u8; 34] = [
    0x8B, 0x01, 0x8B, 0x51, 0x04, 0xC1, 0xE0, 0x04, 0x05, 0x78, 0xF2, 0xF9, 0x00, 0x89, 0x10, 0x8A,
    0x51, 0x08, 0x88, 0x50, 0x04, 0x8B, 0x51, 0x0C, 0x89, 0x50, 0x08, 0x8A, 0x49, 0x10, 0x88, 0x48,
    0x0C, 0xC3,
];
const B265: [u8; 61] = [
    0x56, 0x8B, 0xF0, 0x85, 0xF6, 0x7E, 0x34, 0x83, 0xC1, 0x08, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x41, 0xF8, 0x8B, 0x51, 0xFC, 0xC1, 0xE0, 0x04, 0x05, 0x78, 0xF2, 0xF9, 0x00, 0x89, 0x10,
    0x0F, 0xB6, 0x11, 0x88, 0x50, 0x04, 0x8B, 0x51, 0x04, 0x89, 0x50, 0x08, 0x0F, 0xB6, 0x51, 0x08,
    0x83, 0xC1, 0x14, 0x83, 0xEE, 0x01, 0x88, 0x50, 0x0C, 0x75, 0xD5, 0x5E, 0xC3,
];
const B266: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B267: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B268: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B269: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B270: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B271: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B272: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B273: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B274: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B275: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B276: [u8; 27] = [
    0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x11, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74,
    0x07, 0x8B, 0x48, 0x08, 0x8B, 0x41, 0x3C, 0xC3, 0x33, 0xC0, 0xC3,
];
const B277: [u8; 14] = [
    0xC7, 0x00, 0x4C, 0x87, 0xDE, 0x00, 0xC7, 0x40, 0x18, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B278: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B279: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B280: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B281: [u8; 5] = [
    0xC2, 0x10, 0x00, 0xCC, 0xCC,
];
const B282: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B283: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B284: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B285: [u8; 6] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B286: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B287: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B288: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B289: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B290: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B291: [u8; 6] = [
    0xB8, 0x03, 0x00, 0x00, 0x00, 0xC3,
];
const B292: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B293: [u8; 6] = [
    0xB8, 0x84, 0x2C, 0xDF, 0x00, 0xC3,
];
const B294: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B295: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B296: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B297: [u8; 6] = [
    0xB8, 0x97, 0x57, 0xDC, 0x00, 0xC3,
];
const B298: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B299: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B300: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B301: [u8; 6] = [
    0xB8, 0x16, 0x00, 0x00, 0x00, 0xC3,
];
const B302: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B303: [u8; 6] = [
    0xB8, 0x06, 0x00, 0x00, 0x00, 0xC3,
];
const B304: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B305: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B306: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B307: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B308: [u8; 6] = [
    0xB8, 0x09, 0x00, 0x00, 0x00, 0xC3,
];
const B309: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B310: [u8; 6] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B311: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B312: [u8; 6] = [
    0xB8, 0x0B, 0x00, 0x00, 0x00, 0xC3,
];
const B313: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B314: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B315: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B316: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B317: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B318: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B319: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B320: [u8; 6] = [
    0xB8, 0x0D, 0x00, 0x00, 0x00, 0xC3,
];
const B321: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B322: [u8; 6] = [
    0xB8, 0x1B, 0x00, 0x00, 0x00, 0xC3,
];
const B323: [u8; 6] = [
    0xB8, 0x1C, 0x00, 0x00, 0x00, 0xC3,
];
const B324: [u8; 6] = [
    0xB8, 0x1A, 0x00, 0x00, 0x00, 0xC3,
];
const B325: [u8; 6] = [
    0xB8, 0x1D, 0x00, 0x00, 0x00, 0xC3,
];
const B326: [u8; 6] = [
    0xB8, 0x22, 0x00, 0x00, 0x00, 0xC3,
];
const B327: [u8; 6] = [
    0xB8, 0x1E, 0x00, 0x00, 0x00, 0xC3,
];
const B328: [u8; 6] = [
    0xB8, 0x1F, 0x00, 0x00, 0x00, 0xC3,
];
const B329: [u8; 6] = [
    0xB8, 0x20, 0x00, 0x00, 0x00, 0xC3,
];
const B330: [u8; 6] = [
    0xB8, 0x21, 0x00, 0x00, 0x00, 0xC3,
];
const B331: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B332: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B333: [u8; 6] = [
    0xB8, 0x17, 0x00, 0x00, 0x00, 0xC3,
];
const B334: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B335: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B336: [u8; 48] = [
    0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x24, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84,
    0xD2, 0x75, 0x1A, 0x8B, 0x44, 0x24, 0x04, 0x8A, 0x80, 0xA8, 0x0F, 0x00, 0x00, 0x3C, 0xFF, 0x74,
    0x0C, 0x0F, 0xBE, 0xD0, 0x83, 0x44, 0x91, 0x1C, 0x01, 0x8D, 0x44, 0x91, 0x1C, 0xC2, 0x04, 0x00,
];
const B337: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B338: [u8; 6] = [
    0xB8, 0x15, 0x00, 0x00, 0x00, 0xC3,
];
const B339: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B340: [u8; 6] = [
    0xB8, 0x10, 0x00, 0x00, 0x00, 0xC3,
];
const B341: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B342: [u8; 6] = [
    0xB8, 0x11, 0x00, 0x00, 0x00, 0xC3,
];
const B343: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B344: [u8; 6] = [
    0xB8, 0x24, 0x00, 0x00, 0x00, 0xC3,
];
const B345: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B346: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B347: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B348: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B349: [u8; 6] = [
    0xB8, 0x0C, 0x00, 0x00, 0x00, 0xC3,
];
const B350: [u8; 6] = [
    0xB8, 0x12, 0x00, 0x00, 0x00, 0xC3,
];
const B351: [u8; 6] = [
    0xB8, 0xF0, 0x12, 0xDF, 0x00, 0xC3,
];
const B352: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B353: [u8; 6] = [
    0xB8, 0x13, 0x00, 0x00, 0x00, 0xC3,
];
const B354: [u8; 5] = [
    0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B355: [u8; 6] = [
    0xB8, 0x14, 0x00, 0x00, 0x00, 0xC3,
];
const B356: [u8; 6] = [
    0xB8, 0x97, 0x57, 0xDC, 0x00, 0xC3,
];
const B357: [u8; 6] = [
    0xB8, 0x19, 0x00, 0x00, 0x00, 0xC3,
];
const B358: [u8; 6] = [
    0xB8, 0x97, 0x57, 0xDC, 0x00, 0xC3,
];
const B359: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B360: [u8; 91] = [
    0xC7, 0x00, 0x94, 0xAB, 0xDF, 0x00, 0x83, 0xC9, 0xFF, 0x89, 0x48, 0x30, 0x89, 0x48, 0x34, 0x33,
    0xD2, 0x89, 0x50, 0x3C, 0x89, 0x50, 0x40, 0x89, 0x50, 0x44, 0x89, 0x50, 0x48, 0x89, 0x50, 0x4C,
    0x89, 0x50, 0x50, 0xC6, 0x40, 0x64, 0x01, 0x89, 0x48, 0x68, 0x8B, 0x15, 0x0C, 0x63, 0x3D, 0x03,
    0x89, 0x90, 0x88, 0x00, 0x00, 0x00, 0x8B, 0x15, 0x0C, 0x63, 0x3D, 0x03, 0x89, 0x90, 0x90, 0x00,
    0x00, 0x00, 0x89, 0x88, 0xD0, 0x00, 0x00, 0x00, 0x89, 0x88, 0xD4, 0x00, 0x00, 0x00, 0x89, 0x88,
    0xD8, 0x00, 0x00, 0x00, 0x89, 0x88, 0xDC, 0x00, 0x00, 0x00, 0xC3,
];
const B361: [u8; 6] = [
    0xB8, 0x18, 0x00, 0x00, 0x00, 0xC3,
];
const B362: [u8; 6] = [
    0xB8, 0x97, 0x57, 0xDC, 0x00, 0xC3,
];
const B363: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B364: [u8; 6] = [
    0xB8, 0x08, 0x00, 0x00, 0x00, 0xC3,
];
const B365: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B366: [u8; 6] = [
    0xB8, 0x10, 0x00, 0x00, 0x00, 0xC3,
];
const B367: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B368: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B369: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B370: [u8; 173] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x8B, 0xF0, 0x8B, 0x94, 0xB1, 0x08, 0x38, 0x00, 0x00, 0x57,
    0x8B, 0xC2, 0x33, 0xFF, 0x3B, 0xC7, 0x74, 0x13, 0x39, 0x18, 0x74, 0x0F, 0x8B, 0x40, 0x04, 0x3B,
    0xC2, 0x74, 0x06, 0x3B, 0xC7, 0x75, 0xF1, 0xEB, 0x02, 0x33, 0xC0, 0x3B, 0xD0, 0x75, 0x1D, 0x8B,
    0x50, 0x04, 0x3B, 0xD0, 0x75, 0x0F, 0x89, 0x78, 0x04, 0x89, 0x78, 0x08, 0x89, 0xBC, 0xB1, 0x08,
    0x38, 0x00, 0x00, 0xEB, 0x1F, 0x89, 0x94, 0xB1, 0x08, 0x38, 0x00, 0x00, 0x8B, 0x50, 0x08, 0x8B,
    0x70, 0x04, 0x89, 0x72, 0x04, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x89, 0x72, 0x08, 0x89, 0x78,
    0x04, 0x89, 0x78, 0x08, 0xC7, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0x91, 0x88, 0x38, 0x00, 0x00,
    0x3B, 0xD7, 0x5F, 0x5E, 0x5B, 0x75, 0x0F, 0x89, 0x40, 0x04, 0x89, 0x40, 0x08, 0x89, 0x81, 0x88,
    0x38, 0x00, 0x00, 0xC2, 0x04, 0x00, 0x8B, 0x52, 0x08, 0x89, 0x50, 0x08, 0x8B, 0x91, 0x88, 0x38,
    0x00, 0x00, 0x89, 0x50, 0x04, 0x8B, 0x91, 0x88, 0x38, 0x00, 0x00, 0x8B, 0x52, 0x08, 0x89, 0x42,
    0x04, 0x8B, 0x89, 0x88, 0x38, 0x00, 0x00, 0x89, 0x41, 0x08, 0xC2, 0x04, 0x00,
];
const B371: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B372: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B373: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B374: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B375: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B376: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B377: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B378: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B379: [u8; 41] = [
    0xA1, 0x34, 0xA4, 0xF7, 0x01, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x49, 0x18, 0x89, 0x49, 0x1C, 0x89,
    0x0D, 0x34, 0xA4, 0xF7, 0x01, 0xC3, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x89, 0x41, 0x18, 0x8B,
    0x50, 0x1C, 0x89, 0x4A, 0x18, 0x89, 0x48, 0x1C, 0xC3,
];
const B380: [u8; 41] = [
    0xA1, 0x30, 0xA4, 0xF7, 0x01, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x49, 0x18, 0x89, 0x49, 0x1C, 0x89,
    0x0D, 0x30, 0xA4, 0xF7, 0x01, 0xC3, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x89, 0x41, 0x18, 0x8B,
    0x50, 0x1C, 0x89, 0x4A, 0x18, 0x89, 0x48, 0x1C, 0xC3,
];
const B381: [u8; 41] = [
    0xA1, 0x2C, 0xA4, 0xF7, 0x01, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x49, 0x18, 0x89, 0x49, 0x1C, 0x89,
    0x0D, 0x2C, 0xA4, 0xF7, 0x01, 0xC3, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x89, 0x41, 0x18, 0x8B,
    0x50, 0x1C, 0x89, 0x4A, 0x18, 0x89, 0x48, 0x1C, 0xC3,
];
const B382: [u8; 41] = [
    0xA1, 0x24, 0xA4, 0xF7, 0x01, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x49, 0x18, 0x89, 0x49, 0x1C, 0x89,
    0x0D, 0x24, 0xA4, 0xF7, 0x01, 0xC3, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x89, 0x41, 0x18, 0x8B,
    0x50, 0x1C, 0x89, 0x4A, 0x18, 0x89, 0x48, 0x1C, 0xC3,
];
const B383: [u8; 41] = [
    0xA1, 0x1C, 0xA4, 0xF7, 0x01, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x49, 0x18, 0x89, 0x49, 0x1C, 0x89,
    0x0D, 0x1C, 0xA4, 0xF7, 0x01, 0xC3, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x89, 0x41, 0x18, 0x8B,
    0x50, 0x1C, 0x89, 0x4A, 0x18, 0x89, 0x48, 0x1C, 0xC3,
];
const B384: [u8; 171] = [
    0x53, 0x8D, 0x04, 0x40, 0x0F, 0xB7, 0x0C, 0x85, 0xA0, 0xDE, 0xE8, 0x00, 0x8A, 0x14, 0x85, 0x9C,
    0xDE, 0xE8, 0x00, 0x8D, 0x04, 0x85, 0x9C, 0xDE, 0xE8, 0x00, 0x56, 0x8B, 0x35, 0xB0, 0xA9, 0xF7,
    0x01, 0x33, 0xCE, 0x81, 0xE1, 0x00, 0xF0, 0x00, 0x00, 0x33, 0xF1, 0x8A, 0x0D, 0xAD, 0xA9, 0xF7,
    0x01, 0x8A, 0xDA, 0x02, 0xDB, 0x32, 0xD9, 0x80, 0xE3, 0x02, 0x32, 0xCB, 0x8A, 0x58, 0x02, 0x88,
    0x0D, 0xAD, 0xA9, 0xF7, 0x01, 0x0F, 0xB6, 0x48, 0x01, 0x57, 0x88, 0x0D, 0xAE, 0xA9, 0xF7, 0x01,
    0x8B, 0x48, 0x04, 0x8B, 0xF9, 0xC1, 0xE7, 0x14, 0xC1, 0xFF, 0x14, 0x33, 0xFE, 0xC1, 0xE1, 0x1C,
    0x81, 0xE7, 0x00, 0x0F, 0x00, 0x00, 0x33, 0xF7, 0xC1, 0xF9, 0x1C, 0x33, 0xCE, 0x83, 0xE1, 0x0F,
    0x33, 0xF1, 0x8B, 0x48, 0x08, 0x0F, 0xB6, 0x40, 0x04, 0x33, 0xC6, 0x25, 0xF0, 0x00, 0x00, 0x00,
    0x33, 0xF0, 0xA0, 0xAD, 0xA9, 0xF7, 0x01, 0x02, 0xD2, 0x32, 0xD0, 0x5F, 0x80, 0xE2, 0x0C, 0x89,
    0x35, 0xB0, 0xA9, 0xF7, 0x01, 0x32, 0xC2, 0x5E, 0x88, 0x1D, 0xAF, 0xA9, 0xF7, 0x01, 0x89, 0x0D,
    0xB4, 0xA9, 0xF7, 0x01, 0xA2, 0xAD, 0xA9, 0xF7, 0x01, 0x5B, 0xC3,
];
const B385: [u8; 93] = [
    0xA1, 0x7C, 0x6B, 0xE9, 0x00, 0x83, 0xF8, 0xFF, 0x7E, 0x11, 0x8B, 0x04, 0x85, 0x60, 0xE7, 0x44,
    0x01, 0x83, 0xF8, 0x02, 0x74, 0x44, 0x83, 0xF8, 0x01, 0x74, 0x3F, 0x83, 0xF9, 0x02, 0x74, 0x3A,
    0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x13, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84,
    0xD2, 0x75, 0x09, 0x33, 0xC0, 0x83, 0xF9, 0x01, 0x0F, 0x94, 0xC0, 0xC3, 0xA0, 0x48, 0x8B, 0x52,
    0x02, 0x3C, 0x01, 0x75, 0x12, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x3A, 0xD0, 0x75, 0x08, 0x33,
    0xC0, 0x85, 0xC9, 0x0F, 0x94, 0xC0, 0xC3, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const B386: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B387: [u8; 27] = [
    0x80, 0xB9, 0x04, 0x02, 0x00, 0x00, 0x00, 0x75, 0x0C, 0x83, 0xB9, 0x00, 0x02, 0x00, 0x00, 0x00,
    0x75, 0x03, 0x33, 0xC0, 0xC3, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B388: [u8; 132] = [
    0x83, 0xF8, 0x04, 0x73, 0x7E, 0xC1, 0xE0, 0x06, 0x05, 0x18, 0x3F, 0x9E, 0x02, 0x8B, 0xD0, 0x38,
    0x4A, 0x0D, 0x53, 0x75, 0x54, 0x84, 0xC9, 0xB3, 0x01, 0x0F, 0x94, 0xC0, 0x3A, 0xCB, 0x88, 0x42,
    0x0D, 0x88, 0x5A, 0x1C, 0x75, 0x3C, 0x32, 0xC9, 0x33, 0xC0, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x38, 0x98, 0x25, 0x3F, 0x9E, 0x02, 0x74, 0x21, 0x38, 0x98, 0x24, 0x3F, 0x9E, 0x02, 0x75, 0x02,
    0x8A, 0xCB, 0x83, 0xC0, 0x40, 0x3D, 0x00, 0x01, 0x00, 0x00, 0x72, 0xE4, 0x3A, 0xCB, 0x0F, 0x94,
    0xC1, 0x88, 0x0D, 0x9C, 0x84, 0x52, 0x02, 0xEB, 0x10, 0xC6, 0x05, 0x9C, 0x84, 0x52, 0x02, 0x00,
    0x5B, 0xC3, 0xC6, 0x05, 0x9C, 0x84, 0x52, 0x02, 0x00, 0x80, 0x7A, 0x0C, 0x00, 0x74, 0x13, 0x83,
    0x3D, 0xF0, 0x3E, 0x9E, 0x02, 0x02, 0x75, 0x0A, 0xC7, 0x05, 0xF0, 0x3E, 0x9E, 0x02, 0x04, 0x00,
    0x00, 0x00, 0x5B, 0xC3,
];
const B389: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B390: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B391: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B392: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B393: [u8; 52] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x1F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48, 0x44, 0x75,
    0x06, 0x83, 0x78, 0x4C, 0x0E, 0x74, 0x08, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC2, 0x14, 0x00, 0x33,
    0xC0, 0xC2, 0x14, 0x00,
];
const B394: [u8; 5] = [
    0xC2, 0x10, 0x00, 0xCC, 0xCC,
];
const B395: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B396: [u8; 55] = [
    0x56, 0x8B, 0xF0, 0x32, 0xC0, 0x38, 0x05, 0x31, 0x22, 0xEB, 0x00, 0x75, 0x28, 0xF6, 0x86, 0x9C,
    0x00, 0x00, 0x00, 0x02, 0x74, 0x1F, 0x57, 0xB9, 0x2C, 0x00, 0x00, 0x00, 0xBF, 0x30, 0x22, 0xEB,
    0x00, 0xF3, 0xA5, 0xA2, 0x31, 0x22, 0xEB, 0x00, 0xA2, 0x32, 0x22, 0xEB, 0x00, 0xC6, 0x05, 0x30,
    0x22, 0xEB, 0x00, 0x01, 0x5F, 0x5E, 0xC3,
];
const B397: [u8; 142] = [
    0x53, 0x56, 0x8B, 0x35, 0x10, 0xB2, 0xE8, 0x00, 0x32, 0xD2, 0x83, 0xFE, 0x0D, 0xB9, 0x09, 0x00,
    0x00, 0x00, 0xB8, 0x1C, 0xB2, 0xE8, 0x00, 0xB3, 0x01, 0x75, 0x0C, 0xB9, 0x06, 0x00, 0x00, 0x00,
    0xB8, 0x40, 0xB2, 0xE8, 0x00, 0x8A, 0xD3, 0x3B, 0xF9, 0x7D, 0x60, 0x83, 0xFE, 0x0D, 0x8B, 0x04,
    0xB8, 0x74, 0x0D, 0x8B, 0xC8, 0xC1, 0xE1, 0x04, 0x2B, 0xC8, 0x03, 0xC9, 0x03, 0xC9, 0x8B, 0xC1,
    0x84, 0xD2, 0x74, 0x25, 0x3A, 0x05, 0x4A, 0xD1, 0xA4, 0x02, 0x74, 0x3F, 0x5E, 0x88, 0x1D, 0x2E,
    0xD1, 0xA4, 0x02, 0x88, 0x1D, 0x2C, 0xD1, 0xA4, 0x02, 0xA2, 0x4A, 0xD1, 0xA4, 0x02, 0x66, 0xC7,
    0x05, 0x48, 0xD1, 0xA4, 0x02, 0x00, 0x00, 0x5B, 0xC3, 0x66, 0x3B, 0x05, 0x48, 0xD1, 0xA4, 0x02,
    0x74, 0x19, 0x66, 0xA3, 0x48, 0xD1, 0xA4, 0x02, 0xC6, 0x05, 0x4A, 0xD1, 0xA4, 0x02, 0x00, 0x88,
    0x1D, 0x2E, 0xD1, 0xA4, 0x02, 0x88, 0x1D, 0x2C, 0xD1, 0xA4, 0x02, 0x5E, 0x5B, 0xC3,
];
const B398: [u8; 92] = [
    0x32, 0xC9, 0x83, 0x3D, 0x10, 0xB2, 0xE8, 0x00, 0x0D, 0x53, 0xB8, 0x58, 0xB2, 0xE8, 0x00, 0xB3,
    0x01, 0x75, 0x07, 0xB8, 0x70, 0xB2, 0xE8, 0x00, 0x8A, 0xCB, 0x83, 0xFA, 0x06, 0x7D, 0x3B, 0x84,
    0xC9, 0x8B, 0x04, 0x90, 0x74, 0x1B, 0x3B, 0x05, 0x44, 0xD1, 0xA4, 0x02, 0x74, 0x2C, 0x88, 0x1D,
    0x2E, 0xD1, 0xA4, 0x02, 0x88, 0x1D, 0x2C, 0xD1, 0xA4, 0x02, 0xA3, 0x44, 0xD1, 0xA4, 0x02, 0x5B,
    0xC3, 0x3B, 0x05, 0x40, 0xD1, 0xA4, 0x02, 0x74, 0x11, 0xA3, 0x40, 0xD1, 0xA4, 0x02, 0x88, 0x1D,
    0x2E, 0xD1, 0xA4, 0x02, 0x88, 0x1D, 0x2C, 0xD1, 0xA4, 0x02, 0x5B, 0xC3,
];
const B399: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B400: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B401: [u8; 129] = [
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x4C, 0x24,
    0x08, 0x0F, 0xB6, 0x81, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x85, 0xD2, 0x74, 0x24,
    0x8B, 0xCA, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x51, 0x44, 0x75, 0x0A, 0x8B, 0x89, 0xB0, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x83, 0xC1, 0x6C, 0xC1, 0xE0, 0x07, 0x03, 0xC1,
    0x56, 0x0F, 0xB7, 0x74, 0x24, 0x14, 0xB9, 0xF8, 0xB1, 0xE8, 0x00, 0x83, 0xC0, 0x04, 0xBA, 0x04,
    0x00, 0x00, 0x00, 0x85, 0x31, 0x74, 0x08, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0x00, 0x01, 0x83, 0xC1, 0x04, 0x83, 0xC0, 0x08, 0x83, 0xEA, 0x01, 0x75, 0xE6, 0x5E, 0xC2, 0x10,
    0x00,
];
const B402: [u8; 74] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x3F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x2B, 0x39, 0x48, 0x44, 0x75,
    0x26, 0x83, 0x78, 0x4C, 0x02, 0x75, 0x20, 0x8B, 0x88, 0x80, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24,
    0x0C, 0x0F, 0xB7, 0x10, 0xC1, 0xE9, 0x04, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x80, 0xE1, 0x01, 0x88,
    0x0C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x5E, 0xC2, 0x0C, 0x00,
];
const B403: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B404: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B405: [u8; 59] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x1F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48, 0x44, 0x75,
    0x06, 0x83, 0x78, 0x4C, 0x03, 0x74, 0x02, 0x33, 0xC0, 0x8A, 0x88, 0xCC, 0x00, 0x00, 0x00, 0x80,
    0xE1, 0x10, 0x80, 0xF9, 0x10, 0x0F, 0x95, 0xC0, 0xC2, 0x04, 0x00,
];
const B406: [u8; 86] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x4B, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x37, 0x39, 0x48, 0x44, 0x75,
    0x32, 0xB9, 0x03, 0x00, 0x00, 0x00, 0x39, 0x48, 0x4C, 0x75, 0x28, 0x39, 0x88, 0xD0, 0x00, 0x00,
    0x00, 0x75, 0x20, 0x8B, 0x88, 0x80, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10,
    0xC1, 0xE9, 0x04, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x80, 0xE1, 0x01, 0x88, 0x0C, 0x32, 0x66, 0x83,
    0x00, 0x01, 0x5E, 0xC2, 0x0C, 0x00,
];
const B407: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B408: [u8; 129] = [
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x4C, 0x24,
    0x08, 0x0F, 0xB6, 0x81, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x85, 0xD2, 0x74, 0x24,
    0x8B, 0xCA, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x51, 0x44, 0x75, 0x0A, 0x8B, 0x89, 0xB0, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x83, 0xC1, 0x6C, 0xC1, 0xE0, 0x07, 0x03, 0xC1,
    0x56, 0x0F, 0xB7, 0x74, 0x24, 0x14, 0xB9, 0x9C, 0xB1, 0xE8, 0x00, 0x83, 0xC0, 0x04, 0xBA, 0x0A,
    0x00, 0x00, 0x00, 0x85, 0x31, 0x74, 0x08, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0x00, 0x01, 0x83, 0xC1, 0x04, 0x83, 0xC0, 0x08, 0x83, 0xEA, 0x01, 0x75, 0xE6, 0x5E, 0xC2, 0x10,
    0x00,
];
const B409: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B410: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B411: [u8; 129] = [
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x4C, 0x24,
    0x08, 0x0F, 0xB6, 0x81, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x85, 0xD2, 0x74, 0x24,
    0x8B, 0xCA, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x51, 0x44, 0x75, 0x0A, 0x8B, 0x89, 0xB0, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x83, 0xC1, 0x6C, 0xC1, 0xE0, 0x07, 0x03, 0xC1,
    0x56, 0x0F, 0xB7, 0x74, 0x24, 0x14, 0xB9, 0x64, 0xB1, 0xE8, 0x00, 0x83, 0xC0, 0x04, 0xBA, 0x04,
    0x00, 0x00, 0x00, 0x85, 0x31, 0x74, 0x08, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0x00, 0x01, 0x83, 0xC1, 0x04, 0x83, 0xC0, 0x08, 0x83, 0xEA, 0x01, 0x75, 0xE6, 0x5E, 0xC2, 0x10,
    0x00,
];
const B412: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B413: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B414: [u8; 129] = [
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x4C, 0x24,
    0x08, 0x0F, 0xB6, 0x81, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x85, 0xD2, 0x74, 0x24,
    0x8B, 0xCA, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x51, 0x44, 0x75, 0x0A, 0x8B, 0x89, 0xB0, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x83, 0xC1, 0x6C, 0xC1, 0xE0, 0x07, 0x03, 0xC1,
    0x56, 0x0F, 0xB7, 0x74, 0x24, 0x14, 0xB9, 0x28, 0xB1, 0xE8, 0x00, 0x83, 0xC0, 0x04, 0xBA, 0x0B,
    0x00, 0x00, 0x00, 0x85, 0x31, 0x74, 0x08, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0x00, 0x01, 0x83, 0xC1, 0x04, 0x83, 0xC0, 0x08, 0x83, 0xEA, 0x01, 0x75, 0xE6, 0x5E, 0xC2, 0x10,
    0x00,
];
const B415: [u8; 8] = [
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
const B416: [u8; 66] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x37, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x23, 0x39, 0x48, 0x44, 0x75,
    0x1E, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x39, 0x48, 0x4C, 0x75, 0x14, 0x38, 0x88, 0xCC, 0x03, 0x00,
    0x00, 0x75, 0x0C, 0x8B, 0x88, 0x60, 0x13, 0x00, 0x00, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x00, 0xC2,
    0x0C, 0x00,
];
const B417: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B418: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B419: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B420: [u8; 129] = [
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x4C, 0x24,
    0x08, 0x0F, 0xB6, 0x81, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x85, 0xD2, 0x74, 0x24,
    0x8B, 0xCA, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x51, 0x44, 0x75, 0x0A, 0x8B, 0x89, 0xB0, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x83, 0xC1, 0x6C, 0xC1, 0xE0, 0x07, 0x03, 0xC1,
    0x56, 0x0F, 0xB7, 0x74, 0x24, 0x14, 0xB9, 0xEC, 0xB0, 0xE8, 0x00, 0x83, 0xC0, 0x04, 0xBA, 0x03,
    0x00, 0x00, 0x00, 0x85, 0x31, 0x74, 0x08, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0x00, 0x01, 0x83, 0xC1, 0x04, 0x83, 0xC0, 0x08, 0x83, 0xEA, 0x01, 0x75, 0xE6, 0x5E, 0xC2, 0x10,
    0x00,
];
const B421: [u8; 49] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x20, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00,
    0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x0E, 0x80, 0xB8, 0xF8, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x05, 0x33, 0xC0, 0xC2, 0x04, 0x00, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x04,
    0x00,
];
const B422: [u8; 59] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x1F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48, 0x44, 0x75,
    0x06, 0x83, 0x78, 0x4C, 0x0C, 0x74, 0x02, 0x33, 0xC0, 0x8A, 0x88, 0x20, 0x01, 0x00, 0x00, 0x80,
    0xE1, 0x01, 0x80, 0xF9, 0x01, 0x0F, 0x95, 0xC0, 0xC2, 0x04, 0x00,
];
const B423: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B424: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B425: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B426: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B427: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B428: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B429: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B430: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B431: [u8; 5] = [
    0xC2, 0x10, 0x00, 0xCC, 0xCC,
];
const B432: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B433: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B434: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B435: [u8; 5] = [
    0xC2, 0x0C, 0x00, 0xCC, 0xCC,
];
const B436: [u8; 71] = [
    0xA1, 0x58, 0x39, 0xEB, 0x00, 0x85, 0xC0, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x7C, 0x1B, 0x3B,
    0xC1, 0x7E, 0x0B, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x7C, 0x2A, 0xEB, 0x0C, 0x8B, 0xD1,
    0x2B, 0xD0, 0x81, 0xFA, 0x00, 0xE9, 0xA4, 0x35, 0x7D, 0x1C, 0x8D, 0x81, 0xF4, 0x01, 0x00, 0x00,
    0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x58, 0x39, 0xEB, 0x00, 0x7E, 0x0A, 0x2D, 0x00, 0xD2, 0x49,
    0x6B, 0xA3, 0x58, 0x39, 0xEB, 0x00, 0xC3,
];
const B437: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B438: [u8; 63] = [
    0x8B, 0x91, 0xBC, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x04, 0x89, 0x10, 0x8B, 0x91, 0xC0, 0x00,
    0x00, 0x00, 0x89, 0x50, 0x04, 0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x89, 0x50, 0x08, 0x8B, 0x91,
    0xC8, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x08, 0x89, 0x10, 0x8B, 0x91, 0xCC, 0x00, 0x00, 0x00,
    0x89, 0x50, 0x04, 0x8B, 0x89, 0xD0, 0x00, 0x00, 0x00, 0x89, 0x48, 0x08, 0xC2, 0x08, 0x00,
];
const B439: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B440: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B441: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B442: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B443: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B444: [u8; 73] = [
    0x8B, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0x50, 0x44, 0x8B, 0x42, 0x30, 0x8B, 0x54, 0x24, 0x04,
    0x56, 0x8B, 0x30, 0x89, 0x32, 0x8B, 0x70, 0x04, 0x89, 0x72, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x42,
    0x08, 0x8B, 0x89, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0x51, 0x44, 0x8B, 0x42, 0x30, 0x8B, 0x50, 0x0C,
    0x8B, 0x4C, 0x24, 0x0C, 0x83, 0xC0, 0x0C, 0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B,
    0x40, 0x08, 0x89, 0x41, 0x08, 0x5E, 0xC2, 0x08, 0x00,
];
const B445: [u8; 86] = [
    0xF6, 0x81, 0x20, 0x01, 0x00, 0x00, 0x10, 0x74, 0x4A, 0x8B, 0x81, 0xE8, 0x00, 0x00, 0x00, 0x85,
    0xC0, 0x74, 0x40, 0x8B, 0x40, 0x44, 0x8B, 0x40, 0x30, 0x8B, 0x54, 0x24, 0x04, 0x56, 0x8B, 0x30,
    0x89, 0x32, 0x8B, 0x70, 0x04, 0x89, 0x72, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x42, 0x08, 0x8B, 0x89,
    0xE8, 0x00, 0x00, 0x00, 0x8B, 0x51, 0x44, 0x8B, 0x42, 0x30, 0x8B, 0x50, 0x0C, 0x8B, 0x4C, 0x24,
    0x0C, 0x83, 0xC0, 0x0C, 0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89,
    0x41, 0x08, 0x5E, 0xC2, 0x08, 0x00,
];
const B446: [u8; 63] = [
    0x8B, 0x91, 0xDC, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x04, 0x89, 0x10, 0x8B, 0x91, 0xE0, 0x00,
    0x00, 0x00, 0x89, 0x50, 0x04, 0x8B, 0x91, 0xE4, 0x00, 0x00, 0x00, 0x89, 0x50, 0x08, 0x8B, 0x91,
    0xE8, 0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x08, 0x89, 0x10, 0x8B, 0x91, 0xEC, 0x00, 0x00, 0x00,
    0x89, 0x50, 0x04, 0x8B, 0x89, 0xF0, 0x00, 0x00, 0x00, 0x89, 0x48, 0x08, 0xC2, 0x08, 0x00,
];
const B447: [u8; 110] = [
    0x85, 0xD2, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x8A, 0xA8, 0x02, 0x00, 0x00, 0x85, 0xC9, 0x74,
    0x28, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14,
    0x02, 0x85, 0xC0, 0x74, 0x14, 0x39, 0x48, 0x44, 0x75, 0x0F, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x09,
    0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x74, 0x34, 0x8B, 0x8A, 0xA4, 0x02, 0x00, 0x00, 0x85,
    0xC9, 0x74, 0x28, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x14, 0x39, 0x48, 0x44, 0x75, 0x0F, 0x83, 0x78, 0x4C, 0x01,
    0x75, 0x09, 0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x74, 0x02, 0x8B, 0xC2, 0xC3,
];
const B448: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x0C, 0x00,
];
const B449: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B450: [u8; 51] = [
    0x85, 0xC0, 0x74, 0x2C, 0x8B, 0x80, 0xCC, 0x10, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x22, 0x8B, 0x80,
    0xC4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x18, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x11, 0x83,
    0x78, 0x34, 0x05, 0x0F, 0x94, 0xC0, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
    0x33, 0xC0, 0xC3,
];
const B451: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B452: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B453: [u8; 69] = [
    0xC7, 0x00, 0xBC, 0xE3, 0xE3, 0x00, 0x8B, 0x0D, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x48, 0x0C, 0x8B,
    0x15, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x50, 0x0C, 0x80, 0x60, 0x18, 0xFC, 0x33, 0xC9, 0x89, 0x48,
    0x10, 0x89, 0x48, 0x08, 0x88, 0x48, 0x14, 0xC6, 0x40, 0x15, 0xFF, 0xC7, 0x00, 0xB8, 0x2F, 0xE4,
    0x00, 0xC7, 0x40, 0x04, 0x60, 0x29, 0xA6, 0x00, 0x89, 0x48, 0x20, 0x89, 0x48, 0x24, 0x89, 0x48,
    0x28, 0x89, 0x48, 0x1C, 0xC3,
];
const B454: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B455: [u8; 35] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1B, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0xC7, 0x04, 0x82, 0x00, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04,
    0x72, 0xEE, 0xC3,
];
const B456: [u8; 58] = [
    0x8B, 0xC1, 0x8B, 0x4C, 0x24, 0x04, 0xC7, 0x40, 0x18, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x11, 0x89,
    0x50, 0x1C, 0x8B, 0x51, 0x04, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x89, 0x50, 0x20, 0x8B, 0x49, 0x08,
    0x8B, 0x54, 0x24, 0x10, 0x57, 0x89, 0x48, 0x24, 0x8D, 0x78, 0x28, 0xB9, 0x09, 0x00, 0x00, 0x00,
    0xF3, 0xA5, 0x5F, 0x89, 0x50, 0x4C, 0x5E, 0xC2, 0x0C, 0x00,
];
const B457: [u8; 53] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0xC1, 0x89, 0x50, 0x18, 0x8B, 0x4A, 0x1C, 0x89, 0x48, 0x1C, 0x8B,
    0x4A, 0x20, 0x89, 0x48, 0x20, 0x8B, 0x4A, 0x24, 0x56, 0x57, 0x89, 0x48, 0x24, 0x8D, 0x72, 0x28,
    0x8D, 0x78, 0x28, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x8B, 0x52, 0x4C, 0x5F, 0x89, 0x50,
    0x4C, 0x5E, 0xC2, 0x04, 0x00,
];
const B458: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x04, 0x00,
];
const B459: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B460: [u8; 48] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x28, 0x56, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x35, 0xF0, 0x80, 0xEB, 0x00, 0x8B, 0x51, 0x08, 0x89, 0x34, 0xC2, 0x8B, 0x35, 0xF4, 0x80,
    0xEB, 0x00, 0x89, 0x74, 0xC2, 0x04, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72, 0xE2, 0x5E, 0xC3,
];
const B461: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B462: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B463: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B464: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B465: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B466: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B467: [u8; 34] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1A, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x66, 0xC7, 0x04, 0x42, 0xFF, 0xFF, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04, 0x72,
    0xEF, 0xC3,
];
const B468: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B469: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B470: [u8; 59] = [
    0x83, 0x79, 0x10, 0x00, 0x74, 0x32, 0x8B, 0x41, 0x0C, 0x85, 0xC0, 0x74, 0x2B, 0x8B, 0x54, 0x24,
    0x04, 0x56, 0x57, 0x8B, 0x78, 0x58, 0x83, 0xFF, 0xFF, 0x74, 0x0F, 0x8B, 0x72, 0x40, 0x83, 0xFE,
    0x10, 0x74, 0x13, 0x89, 0x3C, 0xB2, 0x83, 0x42, 0x40, 0x01, 0x8B, 0x40, 0x5C, 0x3B, 0x41, 0x0C,
    0x74, 0x04, 0x85, 0xC0, 0x75, 0xDD, 0x5F, 0x5E, 0xC2, 0x04, 0x00,
];
const B471: [u8; 6] = [
    0xB8, 0x5C, 0x60, 0xE4, 0x00, 0xC3,
];
const B472: [u8; 6] = [
    0xB8, 0x6C, 0x60, 0xE4, 0x00, 0xC3,
];
const B473: [u8; 6] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
];
const B474: [u8; 6] = [
    0xB8, 0x03, 0x00, 0x00, 0x00, 0xC3,
];
const B475: [u8; 6] = [
    0xB8, 0xA0, 0x60, 0xE4, 0x00, 0xC3,
];
const B476: [u8; 6] = [
    0xB8, 0xB0, 0x60, 0xE4, 0x00, 0xC3,
];
const B477: [u8; 6] = [
    0xB8, 0x06, 0x00, 0x00, 0x00, 0xC3,
];
const B478: [u8; 6] = [
    0xB8, 0xB8, 0x60, 0xE4, 0x00, 0xC3,
];
const B479: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B480: [u8; 6] = [
    0xB8, 0xCC, 0x60, 0xE4, 0x00, 0xC3,
];
const B481: [u8; 6] = [
    0xB8, 0x08, 0x00, 0x00, 0x00, 0xC3,
];
const B482: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B483: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B484: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B485: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B486: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B487: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B488: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B489: [u8; 163] = [
    0x8B, 0x96, 0x6C, 0x85, 0x00, 0x00, 0x85, 0xD2, 0x0F, 0x84, 0x94, 0x00, 0x00, 0x00, 0x53, 0x8B,
    0x1D, 0x10, 0xA1, 0x52, 0x02, 0x55, 0x8D, 0x2C, 0x5B, 0x8D, 0x2C, 0xED, 0x50, 0x23, 0x16, 0x03,
    0x57, 0x81, 0xFD, 0x50, 0x53, 0x16, 0x03, 0x7D, 0x70, 0x8B, 0xC5, 0x83, 0x48, 0x14, 0x01, 0x8B,
    0x4A, 0x10, 0x8B, 0x78, 0x14, 0x33, 0xCF, 0x83, 0xE1, 0x02, 0x33, 0xCF, 0x89, 0x48, 0x14, 0x8B,
    0x7A, 0x10, 0x33, 0xF9, 0x83, 0xE7, 0x04, 0x33, 0xF9, 0x89, 0x78, 0x14, 0x8B, 0x4E, 0x14, 0x89,
    0x08, 0x8B, 0x4E, 0x18, 0x89, 0x48, 0x04, 0x8B, 0x4E, 0x1C, 0x89, 0x48, 0x08, 0x8B, 0x4E, 0x44,
    0x89, 0x48, 0x0C, 0x8B, 0x4A, 0x0C, 0x83, 0xC3, 0x01, 0x83, 0xC5, 0x18, 0x89, 0x48, 0x10, 0xF6,
    0x42, 0x10, 0x02, 0x74, 0x15, 0x8B, 0x42, 0x0C, 0x85, 0xC0, 0x74, 0x07, 0x81, 0x48, 0x10, 0x00,
    0x00, 0x10, 0x00, 0x83, 0x05, 0x14, 0xA1, 0x52, 0x02, 0x01, 0x8B, 0x52, 0x14, 0x3B, 0x96, 0x6C,
    0x85, 0x00, 0x00, 0x74, 0x04, 0x85, 0xD2, 0x75, 0x88, 0x5F, 0x5D, 0x89, 0x1D, 0x10, 0xA1, 0x52,
    0x02, 0x5B, 0xC3,
];
const B490: [u8; 90] = [
    0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01, 0x53, 0x8A, 0x5C, 0x24, 0x08, 0x57, 0x8B,
    0xF8, 0x8B, 0x06, 0x75, 0x0F, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x75, 0x05, 0x8B,
    0x50, 0x10, 0xEB, 0x03, 0x8B, 0x50, 0x0C, 0x8B, 0x46, 0x04, 0x8B, 0x4E, 0x08, 0x2B, 0xC8, 0x03,
    0xCA, 0x84, 0xDB, 0x74, 0x04, 0x8B, 0xD7, 0xEB, 0x06, 0x3B, 0xF9, 0x8B, 0xD1, 0x7C, 0x02, 0x8B,
    0xFA, 0x03, 0xC7, 0x84, 0xDB, 0x89, 0x46, 0x04, 0x8B, 0xC7, 0x74, 0x09, 0x3B, 0xCA, 0x7D, 0x05,
    0x2B, 0xD1, 0x89, 0x56, 0x08, 0x5F, 0x5B, 0xC2, 0x04, 0x00,
];
const B491: [u8; 108] = [
    0x8B, 0x15, 0x50, 0x7B, 0x2D, 0x02, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x33, 0xC0, 0x85, 0xD2,
    0x56, 0x7E, 0x27, 0x8B, 0x31, 0x8B, 0xCE, 0x8B, 0x29, 0x39, 0x7D, 0x04, 0x74, 0x12, 0x83, 0xC0,
    0x01, 0x83, 0xC1, 0x0C, 0x3B, 0xC2, 0x7C, 0xEF, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC2, 0x04, 0x00,
    0x8D, 0x14, 0x40, 0x8D, 0x0C, 0x96, 0x85, 0xC9, 0x75, 0x08, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC2,
    0x04, 0x00, 0x8B, 0x51, 0x04, 0x3B, 0xDA, 0x8B, 0xC3, 0x7C, 0x02, 0x8B, 0xC2, 0x2B, 0xD0, 0x89,
    0x51, 0x04, 0x8B, 0x51, 0x08, 0x85, 0xD2, 0x76, 0x0D, 0x33, 0xF6, 0x2B, 0xD0, 0x3B, 0xF2, 0x1B,
    0xF6, 0x23, 0xF2, 0x89, 0x71, 0x08, 0x5E, 0x5D, 0x5B, 0xC2, 0x04, 0x00,
];
const B492: [u8; 68] = [
    0x85, 0xF6, 0x74, 0x3D, 0x85, 0xFF, 0x74, 0x39, 0x33, 0xC0, 0x80, 0xBE, 0xD8, 0x84, 0x00, 0x00,
    0x01, 0x75, 0x24, 0x8B, 0x96, 0xD0, 0x84, 0x00, 0x00, 0x85, 0xD2, 0x53, 0x76, 0x16, 0x8D, 0x8E,
    0xC8, 0x84, 0x00, 0x00, 0x8B, 0x19, 0x39, 0x3B, 0x74, 0x0E, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x04,
    0x3B, 0xC2, 0x72, 0xF0, 0x33, 0xC0, 0x5B, 0xC3, 0x8B, 0x84, 0x86, 0xC8, 0x84, 0x00, 0x00, 0x5B,
    0xC3, 0x33, 0xC0, 0xC3,
];
const B493: [u8; 19] = [
    0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x33, 0xC9, 0x83, 0x78, 0x34, 0x05, 0x0F, 0x94, 0xC1,
    0x8A, 0xC1, 0xC3,
];
const B494: [u8; 76] = [
    0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x80, 0x3D, 0x3A, 0x97, 0x52, 0x02, 0x00, 0x75, 0x39,
    0xF6, 0x05, 0x40, 0x6A, 0x7A, 0x02, 0x40, 0x75, 0x30, 0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80,
    0xF9, 0x01, 0x75, 0x12, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x75, 0x08, 0x38, 0x15,
    0xC5, 0xD2, 0xA4, 0x02, 0x75, 0x13, 0x80, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x06, 0xF6, 0x40, 0x0C,
    0x10, 0xEB, 0x04, 0xF6, 0x40, 0x0C, 0x20, 0x74, 0xBB, 0xB0, 0x01, 0xC3,
];
const B495: [u8; 27] = [
    0x8B, 0x40, 0x08, 0x8B, 0x48, 0x34, 0x83, 0xC1, 0xF3, 0x83, 0xF9, 0x01, 0x77, 0x0A, 0x8B, 0x40,
    0x0C, 0xC1, 0xE8, 0x16, 0x83, 0xE0, 0x01, 0xC3, 0x32, 0xC0, 0xC3,
];
const B496: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B497: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B498: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B499: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B500: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B501: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B502: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B503: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B504: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B505: [u8; 5] = [
    0x8B, 0x41, 0x1C, 0xC3, 0xCC,
];
const B506: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B507: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B508: [u8; 5] = [
    0xB0, 0x01, 0xC2, 0x0C, 0x00,
];
const B509: [u8; 5] = [
    0xC2, 0x08, 0x00, 0xCC, 0xCC,
];
const B510: [u8; 5] = [
    0xB0, 0x01, 0xC3, 0xCC, 0xCC,
];
const B511: [u8; 5] = [
    0x33, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B512: [u8; 35] = [
    0x33, 0xC0, 0x39, 0x41, 0x04, 0x76, 0x1B, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0xC7, 0x04, 0x82, 0xFF, 0xFF, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x04,
    0x72, 0xEE, 0xC3,
];
const B513: [u8; 62] = [
    0x8B, 0x41, 0x24, 0x83, 0xE8, 0x00, 0x74, 0x1E, 0x83, 0xE8, 0x01, 0x75, 0x2E, 0x8B, 0x41, 0x30,
    0x3B, 0x05, 0x3C, 0xA2, 0x52, 0x02, 0x73, 0x23, 0x8B, 0x0D, 0x40, 0xA2, 0x52, 0x02, 0x8D, 0x04,
    0x40, 0x8B, 0x44, 0xC1, 0x14, 0xC3, 0x8B, 0x49, 0x2C, 0x83, 0xF9, 0x22, 0x7D, 0x0D, 0xA1, 0x38,
    0xA2, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x04, 0x8B, 0x04, 0x88, 0xC3, 0x33, 0xC0, 0xC3,
];
const B514: [u8; 75] = [
    0x0F, 0xB7, 0x06, 0x66, 0x3D, 0x30, 0x00, 0x8B, 0xCE, 0x72, 0x18, 0x0F, 0xB7, 0xC0, 0x8B, 0xFF,
    0x66, 0x3D, 0x39, 0x00, 0x77, 0x0D, 0x0F, 0xB7, 0x41, 0x02, 0x83, 0xC1, 0x02, 0x66, 0x3D, 0x30,
    0x00, 0x73, 0xED, 0x83, 0xE9, 0x02, 0x33, 0xC0, 0x3B, 0xCE, 0xBA, 0x01, 0x00, 0x00, 0x00, 0x72,
    0x19, 0x57, 0x0F, 0xB7, 0x39, 0x83, 0xEF, 0x30, 0x0F, 0xAF, 0xFA, 0x8D, 0x14, 0x92, 0x83, 0xE9,
    0x02, 0x03, 0xC7, 0x03, 0xD2, 0x3B, 0xCE, 0x73, 0xE9, 0x5F, 0xC3,
];
const B515: [u8; 126] = [
    0x8B, 0x42, 0x0C, 0x53, 0x56, 0x57, 0x33, 0xDB, 0x3B, 0xC3, 0x74, 0x6A, 0x8B, 0x70, 0x04, 0x8B,
    0x7A, 0x0C, 0x8B, 0xCE, 0x2B, 0xCF, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xCE, 0x3B, 0xF8, 0x75, 0x11,
    0x3B, 0xF0, 0x75, 0x0A, 0x89, 0x58, 0x04, 0x89, 0x18, 0x89, 0x5A, 0x0C, 0xEB, 0x17, 0x89, 0x72,
    0x0C, 0x8B, 0x30, 0x8B, 0x78, 0x04, 0x89, 0x7E, 0x04, 0x8B, 0x70, 0x04, 0x8B, 0x38, 0x89, 0x3E,
    0x89, 0x58, 0x04, 0x89, 0x18, 0x8B, 0x72, 0x10, 0x3B, 0xF3, 0x75, 0x0C, 0x89, 0x40, 0x04, 0x89,
    0x00, 0x89, 0x42, 0x10, 0x8B, 0xC1, 0xEB, 0xB0, 0x8B, 0x36, 0x89, 0x46, 0x04, 0x8B, 0x72, 0x10,
    0x8B, 0x36, 0x89, 0x30, 0x8B, 0x72, 0x10, 0x89, 0x06, 0x8B, 0x72, 0x10, 0x89, 0x70, 0x04, 0x89,
    0x42, 0x10, 0x8B, 0xC1, 0xEB, 0x92, 0x5F, 0x5E, 0xC6, 0x42, 0x1C, 0x01, 0x5B, 0xC3,
];
const B516: [u8; 13] = [
    0x80, 0x78, 0x1C, 0x01, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x40, 0x0C, 0xC3,
];
const B517: [u8; 163] = [
    0x85, 0xD2, 0x0F, 0x84, 0x98, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0xC2, 0x3D, 0x80, 0x00, 0x00, 0x00,
    0x0F, 0x83, 0x8A, 0x00, 0x00, 0x00, 0x69, 0xC0, 0x2C, 0x02, 0x00, 0x00, 0x05, 0xF0, 0x96, 0x1A,
    0x03, 0x39, 0x90, 0x24, 0x02, 0x00, 0x00, 0x75, 0x77, 0x39, 0x88, 0x1C, 0x02, 0x00, 0x00, 0x75,
    0x6F, 0x39, 0x41, 0x0C, 0x75, 0x20, 0x8B, 0x50, 0x04, 0x3B, 0xD0, 0x75, 0x16, 0xC7, 0x40, 0x04,
    0x00, 0x00, 0x00, 0x00, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x0C, 0x00, 0x00, 0x00,
    0x00, 0xEB, 0x21, 0x89, 0x51, 0x0C, 0x8B, 0x10, 0x57, 0x8B, 0x78, 0x04, 0x89, 0x7A, 0x04, 0x8B,
    0x38, 0x8B, 0x50, 0x04, 0x89, 0x3A, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x5F, 0x8B, 0x51, 0x10, 0x85, 0xD2, 0x75, 0x0B, 0x89, 0x40, 0x04, 0x89, 0x00,
    0x89, 0x41, 0x10, 0xB0, 0x01, 0xC3, 0x8B, 0x12, 0x89, 0x10, 0x8B, 0x51, 0x10, 0x89, 0x50, 0x04,
    0x8B, 0x51, 0x10, 0x8B, 0x12, 0x89, 0x42, 0x04, 0x8B, 0x49, 0x10, 0x89, 0x01, 0xB0, 0x01, 0xC3,
    0x32, 0xC0, 0xC3,
];
const B518: [u8; 119] = [
    0x8B, 0x42, 0x0C, 0x53, 0x56, 0x57, 0x33, 0xDB, 0x3B, 0xC3, 0x74, 0x67, 0x8B, 0x70, 0x04, 0x8B,
    0x7A, 0x0C, 0x8B, 0xCE, 0x2B, 0xCF, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xCE, 0x3B, 0xF8, 0x75, 0x11,
    0x3B, 0xF0, 0x75, 0x0A, 0x89, 0x58, 0x04, 0x89, 0x18, 0x89, 0x5A, 0x0C, 0xEB, 0x17, 0x89, 0x72,
    0x0C, 0x8B, 0x30, 0x8B, 0x78, 0x04, 0x89, 0x7E, 0x04, 0x8B, 0x70, 0x04, 0x8B, 0x38, 0x89, 0x3E,
    0x89, 0x58, 0x04, 0x89, 0x18, 0x8B, 0x72, 0x10, 0x3B, 0xF3, 0x75, 0x0C, 0x89, 0x40, 0x04, 0x89,
    0x00, 0x89, 0x42, 0x10, 0x8B, 0xC1, 0xEB, 0xB0, 0x8B, 0x36, 0x89, 0x30, 0x8B, 0x72, 0x10, 0x89,
    0x70, 0x04, 0x8B, 0x72, 0x10, 0x8B, 0x36, 0x89, 0x46, 0x04, 0x8B, 0x72, 0x10, 0x89, 0x06, 0x8B,
    0xC1, 0xEB, 0x95, 0x5F, 0x5E, 0x5B, 0xC3,
];
const B519: [u8; 48] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xF8, 0x06, 0x75, 0x1C, 0x8D,
    0x41, 0x7C, 0x8D, 0x4A, 0x04, 0xC7, 0x02, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x10, 0x89, 0x11, 0x8B,
    0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x41, 0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B520: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B521: [u8; 107] = [
    0xA1, 0x14, 0xA2, 0x52, 0x02, 0x83, 0xC0, 0x01, 0x56, 0x33, 0xF6, 0x89, 0x41, 0x28, 0xA3, 0x14,
    0xA2, 0x52, 0x02, 0x83, 0xE0, 0x3F, 0x89, 0x71, 0x2C, 0x89, 0x71, 0x30, 0x89, 0x71, 0x24, 0x89,
    0x71, 0x1C, 0x89, 0x71, 0x20, 0x8B, 0x14, 0x85, 0xB8, 0xF2, 0x2E, 0x02, 0x3B, 0xD6, 0x5E, 0x75,
    0x10, 0x89, 0x49, 0x0C, 0x89, 0x49, 0x10, 0x89, 0x0C, 0x85, 0xB8, 0xF2, 0x2E, 0x02, 0xB0, 0x01,
    0xC3, 0x8B, 0x52, 0x10, 0x89, 0x51, 0x10, 0x8B, 0x14, 0x85, 0xB8, 0xF2, 0x2E, 0x02, 0x89, 0x51,
    0x0C, 0x8B, 0x14, 0x85, 0xB8, 0xF2, 0x2E, 0x02, 0x8B, 0x52, 0x10, 0x89, 0x4A, 0x0C, 0x8B, 0x04,
    0x85, 0xB8, 0xF2, 0x2E, 0x02, 0x89, 0x48, 0x10, 0xB0, 0x01, 0xC3,
];
const B522: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B523: [u8; 97] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x85, 0xED, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0x5D, 0xC3, 0x53, 0x56,
    0x57, 0x8B, 0x3D, 0x48, 0xA2, 0x52, 0x02, 0x33, 0xC0, 0x85, 0xFF, 0x7E, 0x3C, 0xBE, 0x78, 0xF5,
    0x2E, 0x02, 0x8B, 0xD6, 0x8B, 0xCD, 0x8A, 0x19, 0x3A, 0x1A, 0x75, 0x1A, 0x84, 0xDB, 0x74, 0x12,
    0x8A, 0x59, 0x01, 0x3A, 0x5A, 0x01, 0x75, 0x0E, 0x83, 0xC1, 0x02, 0x83, 0xC2, 0x02, 0x84, 0xDB,
    0x75, 0xE4, 0x33, 0xC9, 0xEB, 0x05, 0x1B, 0xC9, 0x83, 0xD9, 0xFF, 0x85, 0xC9, 0x74, 0x0D, 0x83,
    0xC0, 0x01, 0x83, 0xC6, 0x2C, 0x3B, 0xC7, 0x7C, 0xC9, 0x83, 0xC8, 0xFF, 0x5F, 0x5E, 0x5B, 0x5D,
    0xC3,
];
const B524: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B525: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B526: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B527: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x10, 0xE0, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B528: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B529: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B530: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B531: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B532: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B533: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0x98, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B534: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B535: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B536: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B537: [u8; 29] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x11, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x08,
    0x6B, 0xC0, 0x64, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B538: [u8; 20] = [
    0x3B, 0x05, 0x70, 0xA2, 0x52, 0x02, 0x73, 0x09, 0xC1, 0xE0, 0x04, 0x05, 0x70, 0x23, 0x2F, 0x02,
    0xC3, 0x33, 0xC0, 0xC3,
];
const B539: [u8; 104] = [
    0x8B, 0xC1, 0x33, 0xC9, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC7, 0x00, 0x34, 0xD7, 0xE4, 0x00,
    0x89, 0x48, 0x54, 0x89, 0x48, 0x64, 0x89, 0x88, 0xAC, 0x00, 0x00, 0x00, 0x89, 0x88, 0xBC, 0x00,
    0x00, 0x00, 0x89, 0x88, 0xCC, 0x00, 0x00, 0x00, 0x39, 0x88, 0xAC, 0x00, 0x00, 0x00, 0x8D, 0x51,
    0x08, 0x75, 0x0C, 0x89, 0x90, 0xAC, 0x00, 0x00, 0x00, 0x89, 0x88, 0xB0, 0x00, 0x00, 0x00, 0x39,
    0x88, 0xBC, 0x00, 0x00, 0x00, 0x75, 0x0C, 0x89, 0x90, 0xBC, 0x00, 0x00, 0x00, 0x89, 0x88, 0xC0,
    0x00, 0x00, 0x00, 0x39, 0x88, 0xCC, 0x00, 0x00, 0x00, 0x75, 0x0C, 0x89, 0x90, 0xCC, 0x00, 0x00,
    0x00, 0x89, 0x88, 0xD0, 0x00, 0x00, 0x00, 0xC3,
];
const B540: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B541: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0xF4, 0xE3, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B542: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B543: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B544: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0xC4, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B545: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x30, 0xE5, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0xF4, 0xE3, 0xEB, 0x00, 0x3B, 0x02, 0x74, 0xF1, 0x8B, 0x0D, 0x6C,
    0xDF, 0xEB, 0x00, 0x3B, 0x01, 0x0F, 0x94, 0xC0, 0xC2, 0x04, 0x00,
];
const B546: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B547: [u8; 51] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xF8, 0x06, 0x75, 0x1F, 0x8D,
    0x81, 0xC4, 0x00, 0x00, 0x00, 0x8D, 0x4A, 0x04, 0xC7, 0x02, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x10,
    0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x41, 0x08, 0xB0, 0x01,
    0xC2, 0x08, 0x00,
];
const B548: [u8; 51] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xF8, 0x06, 0x75, 0x1F, 0x8D,
    0x81, 0xD0, 0x00, 0x00, 0x00, 0x8D, 0x4A, 0x04, 0xC7, 0x02, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x10,
    0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x41, 0x08, 0xB0, 0x01,
    0xC2, 0x08, 0x00,
];
const B549: [u8; 51] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xF8, 0x06, 0x75, 0x1F, 0x8D,
    0x81, 0xDC, 0x00, 0x00, 0x00, 0x8D, 0x4A, 0x04, 0xC7, 0x02, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x10,
    0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x41, 0x08, 0xB0, 0x01,
    0xC2, 0x08, 0x00,
];
const B550: [u8; 51] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xF8, 0x06, 0x75, 0x1F, 0x8D,
    0x81, 0xE8, 0x00, 0x00, 0x00, 0x8D, 0x4A, 0x04, 0xC7, 0x02, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x10,
    0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08, 0x89, 0x41, 0x08, 0xB0, 0x01,
    0xC2, 0x08, 0x00,
];
const B551: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0x04, 0x01, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B552: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x90, 0xE7, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B553: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B554: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0xF4, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B555: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0xE8, 0xE7, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B556: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B557: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B558: [u8; 5] = [
    0xC2, 0x04, 0x00, 0xCC, 0xCC,
];
const B559: [u8; 5] = [
    0x32, 0xC0, 0xC2, 0x08, 0x00,
];
const B560: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0x98, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B561: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x50, 0xE8, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B562: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B563: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B564: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0xA4, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B565: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x6C, 0xE9, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B566: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B567: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0xCC, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B568: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0xD0, 0xE9, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B569: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B570: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B571: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x8D, 0x04, 0x80, 0xC1, 0xE0, 0x05, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B572: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x18, 0xEA, 0xEB, 0x00, 0x3B, 0x01, 0x75, 0x05, 0xB0, 0x01,
    0xC2, 0x04, 0x00, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x3B, 0x02, 0x0F, 0x94, 0xC0, 0xC2, 0x04,
    0x00,
];
const B573: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B574: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B575: [u8; 5] = [
    0xC3, 0xCC, 0xCC, 0xCC, 0xCC,
];
const B576: [u8; 32] = [
    0x8B, 0x51, 0x14, 0x85, 0xD2, 0x74, 0x14, 0x8B, 0x44, 0x24, 0x04, 0x3B, 0x41, 0x10, 0x73, 0x0B,
    0x69, 0xC0, 0x9C, 0x00, 0x00, 0x00, 0x03, 0xC2, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,
];
const B577: [u8; 5] = [
    0x32, 0xC0, 0xC3, 0xCC, 0xCC,
];
const B578: [u8; 13] = [
    0x8B, 0x41, 0x44, 0x89, 0x41, 0x48, 0x8B, 0x51, 0x54, 0x89, 0x51, 0x58, 0xC3,
];
const B579: [u8; 13] = [
    0x8B, 0x41, 0x44, 0x89, 0x41, 0x48, 0x8B, 0x51, 0x54, 0x89, 0x51, 0x58, 0xC3,
];
const B580: [u8; 103] = [
    0x57, 0x33, 0xFF, 0x39, 0xB8, 0x54, 0x01, 0x00, 0x00, 0x76, 0x5A, 0x53, 0x56, 0x33, 0xF6, 0x90,
    0x8B, 0x88, 0x70, 0x01, 0x00, 0x00, 0x8B, 0x14, 0x31, 0x03, 0xCE, 0x83, 0xFA, 0xFF, 0x75, 0x08,
    0xC7, 0x01, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x0A, 0x8B, 0x98, 0x74, 0x01, 0x00, 0x00, 0x03, 0xDA,
    0x89, 0x19, 0x8B, 0x90, 0x70, 0x01, 0x00, 0x00, 0x8D, 0x4C, 0x16, 0x04, 0x8B, 0x11, 0x83, 0xFA,
    0xFF, 0x75, 0x08, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x0A, 0x8B, 0x98, 0x78, 0x01, 0x00,
    0x00, 0x03, 0xDA, 0x89, 0x19, 0x83, 0xC7, 0x01, 0x83, 0xC6, 0x1C, 0x3B, 0xB8, 0x54, 0x01, 0x00,
    0x00, 0x72, 0xAD, 0x5E, 0x5B, 0x5F, 0xC3,
];
const B581: [u8; 152] = [
    0xF6, 0x80, 0x4C, 0x01, 0x00, 0x00, 0x02, 0x0F, 0x85, 0x8A, 0x00, 0x00, 0x00, 0x8B, 0x88, 0x64,
    0x01, 0x00, 0x00, 0x8B, 0x90, 0x60, 0x01, 0x00, 0x00, 0x81, 0xC2, 0xFF, 0x07, 0x00, 0x00, 0x81,
    0xC1, 0xFF, 0x07, 0x00, 0x00, 0xC1, 0xEA, 0x0B, 0xC1, 0xE9, 0x0B, 0x03, 0xCA, 0x8B, 0x90, 0x5C,
    0x01, 0x00, 0x00, 0x56, 0x81, 0xC2, 0xFF, 0x07, 0x00, 0x00, 0xC1, 0xEA, 0x0B, 0x33, 0xF6, 0x39,
    0xB0, 0x54, 0x01, 0x00, 0x00, 0x57, 0x8D, 0x7C, 0x11, 0x01, 0x76, 0x49, 0x33, 0xC9, 0x8B, 0xFF,
    0x8B, 0x90, 0x70, 0x01, 0x00, 0x00, 0x89, 0x7C, 0x0A, 0x08, 0xF6, 0x80, 0x4C, 0x01, 0x00, 0x00,
    0x01, 0x8B, 0x90, 0x70, 0x01, 0x00, 0x00, 0x74, 0x06, 0x8B, 0x54, 0x0A, 0x14, 0xEB, 0x04, 0x8B,
    0x54, 0x0A, 0x10, 0x85, 0xD2, 0x75, 0x05, 0x83, 0xC7, 0x01, 0xEB, 0x0B, 0x81, 0xC2, 0xFF, 0x07,
    0x00, 0x00, 0xC1, 0xEA, 0x0B, 0x03, 0xFA, 0x83, 0xC6, 0x01, 0x83, 0xC1, 0x1C, 0x3B, 0xB0, 0x54,
    0x01, 0x00, 0x00, 0x72, 0xBB, 0x5F, 0x5E, 0xC3,
];
const B582: [u8; 44] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x0C, 0x00, 0x75, 0x1C, 0x8B, 0x40, 0x04, 0x8B, 0x48, 0x14,
    0x3B, 0x48, 0x28, 0x75, 0x11, 0x8B, 0x48, 0x08, 0x2B, 0x48, 0x0C, 0xF7, 0xC1, 0xF0, 0xFF, 0xFF,
    0xFF, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B583: [u8; 17] = [
    0x8A, 0x81, 0x48, 0x03, 0x00, 0x00, 0x3C, 0xFF, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x0F, 0xB6, 0xC0,
    0xC3,
];
