//! Port module: small data / save / camera-config functions, checked against the disassembly.
//!
//! - `gameplay_constants_parse_*` (and the `camera_free_*` readers): the XML section readers. All
//!   take the section's XML node in EAX (an LTCG parameter; the `camera_free_*` ones take it in EDI
//!   or EAX), find the child named by a string, and call the float / int parsers
//!   0xB74EF0 / 0xB747D0 / 0xB74910 (ECX = the field's name string, ESI = the destination, the
//!   child node on the stack) once per field. The fixed part is table-driven (`parse_fields`).
//! - `gamesave_sNN_*`: the per-section save / load functions that `gamesave_collect` and
//!   `gamesave_apply` call through the table at 0xDD3050 (cdecl, buffer pointer on the stack, no
//!   register parameters). The ones that end in a tail `jmp` are ported as a call of the callee
//!   with the same stack arguments (its other register results are not passed on; the table
//!   callers use none).
//!
//! Every port runs behind a `pushad` thunk: all registers come back as the caller left them,
//! except EAX (the result, or the last callee's result for the void functions), which is a
//! superset of what each original preserves. Callees are called by address with exactly the
//! registers the original had set up.
//!
//! Checks, for the first `SHADOW_CALLS` calls of each port (see `Mode`):
//! - `Snap`: the written ranges are saved, the original (a relocated copy) runs, its writes are
//!   kept, the ranges restored, the Rust version runs, and both are compared. The original's
//!   writes win on a mismatch. Only used where the callees are idempotent.
//! - `Pure`: the original's EAX is compared with Rust's.
//! - `Count`: the callees have side effects (or write too much), so there is no shadow: per-port
//!   call counters only.
//! After the window a port is the bare Rust body (one relaxed load).
//!
//! Threads: all state is atomics; the shadow keeps everything in locals.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

// Memory accesses are volatile here: the originals' loops store in program order and fault at a definite
// element, and the compiler must not merge or reorder the stores (a partial copy up to a fault would differ).
unsafe fn rd(p: usize) -> u32 {
    unsafe { std::ptr::read_volatile(p as *const u32) }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { std::ptr::read_volatile(p as *const u8) }
}
unsafe fn wd(p: usize, v: u32) {
    unsafe { std::ptr::write_volatile(p as *mut u32, v) }
}
unsafe fn wb(p: usize, v: u8) {
    unsafe { std::ptr::write_volatile(p as *mut u8, v) }
}

/// The shadow check (original vs Rust, first N calls) is off: `sr2-judge` is the test now, and the shadow
/// made its runs depend on where the relocated copy landed.
const SHADOW_CALLS: u32 = 0;

// --- registers -------------------------------------------------------------------------------------

/// The state a thunk hands to its handler: XMM0-7, then the `pushad` block (lowest address first).
/// The handler plays the original's register effects on this block (calls through `cx` keep it
/// current, XMM0-7 included); the thunk loads the block back into the CPU on return.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)] // esp is never read
pub(crate) struct Pushad {
    xmm: [[u32; 4]; 8],
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// Stack words handed to every callee: its own arguments, then what the original's frame holds above
/// them (saved registers, locals, the return address, the caller's arguments).
const FW: usize = 12;

/// The words the original has on the stack at a call: `own` (top first: arguments, then pushed
/// registers and locals), then the original's return address and the caller's own stack arguments.
unsafe fn fr(a: *const u32, own: &[u32]) -> [u32; FW] {
    let mut w = [0u32; FW];
    let mut n = 0;
    for &v in own {
        w[n] = v;
        n += 1;
    }
    w[n] = unsafe { *a.sub(1) };
    n += 1;
    let mut i = 0;
    while n < FW {
        w[n] = unsafe { *a.add(i) };
        n += 1;
        i += 1;
    }
    w
}

/// The words a callee entered by a tail `jmp` sees: the original's own stack arguments.
unsafe fn fr_tail(a: *const u32) -> [u32; FW] {
    let mut w = [0u32; FW];
    for (i, v) in w.iter_mut().enumerate() {
        *v = unsafe { *a.add(i) };
    }
    w
}

/// The address of the original's stack slot `k` dwords below its return address (1 = the first
/// word it pushes). The thunk reserves the original's frame there, so locals can live at the same
/// addresses and a pointer to one can be handed to a callee unchanged.
fn slot(a: *const u32, k: usize) -> usize {
    (a as usize).wrapping_sub(4 + 4 * k)
}

/// Stack argument `i` of the call (0 = the first, just above the return address).
unsafe fn arg(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

macro_rules! cx_fn {
    ($name:ident, $skip:literal) => {
        /// Call `f` with every register and XMM0-7 of `c`, the stack words `w` pushed, and
        /// load the registers and XMM0-7 the callee leaves back into `c`.
        unsafe fn $name(f: usize, c: &mut Pushad, w: &[u32; FW]) {
            let blk: [u32; 3] = [f as u32, w.as_ptr() as u32, c as *mut Pushad as u32];
            unsafe {
                std::arch::asm!(
                    "push ebx", "push esi", "push edi", "push ebp",
                    "mov ebx, eax",
                    "push dword ptr [ebx + 8]",
                    "push dword ptr [ebx]",
                    "mov esi, [ebx + 4]",
                    "mov ecx, 12",
                    "2:",
                    "push dword ptr [esi + ecx*4 - 4]",
                    "dec ecx",
                    "jnz 2b",
                    "mov eax, [ebx + 8]",
                    "movups xmm0, [eax]", "movups xmm1, [eax + 16]", "movups xmm2, [eax + 32]", "movups xmm3, [eax + 48]",
                    "movups xmm4, [eax + 64]", "movups xmm5, [eax + 80]", "movups xmm6, [eax + 96]", "movups xmm7, [eax + 112]",
                    "mov edi, [eax + 128]", "mov esi, [eax + 132]", "mov ebp, [eax + 136]", "mov ebx, [eax + 144]",
                    "mov edx, [eax + 148]", "mov ecx, [eax + 152]", "mov eax, [eax + 156]",
                    "call dword ptr [esp + 48]",
                    "add esp, {skip}",
                    "push eax",
                    "mov eax, [esp + 8]",
                    "movups [eax], xmm0", "movups [eax + 16], xmm1", "movups [eax + 32], xmm2", "movups [eax + 48], xmm3",
                    "movups [eax + 64], xmm4", "movups [eax + 80], xmm5", "movups [eax + 96], xmm6", "movups [eax + 112], xmm7",
                    "mov [eax + 128], edi", "mov [eax + 132], esi", "mov [eax + 136], ebp", "mov [eax + 144], ebx",
                    "mov [eax + 148], edx", "mov [eax + 152], ecx",
                    "pop ecx",
                    "mov [eax + 156], ecx",
                    "add esp, 8",
                    "pop ebp", "pop edi", "pop esi", "pop ebx",
                    skip = const $skip,
                    inout("eax") blk.as_ptr() => _,
                    out("ecx") _, out("edx") _,
                    clobber_abi("C"),
                );
            }
        }
    };
}
// The callee pops 0 / 1 / 2 / 3 of the words (a cdecl callee, or one ending in `ret 4 / 8 / 12`).
cx_fn!(cx, 48);
cx_fn!(cx_p1, 44);
cx_fn!(cx_p2, 40);
cx_fn!(cx_p3, 36);

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// `fld dword [lo]; fld dword [hi]; fcom st(1); fnstsw ax` and the two pops: the status word.
unsafe fn fcom_status(lo: usize, hi: usize) -> u32 {
    let sw: u32;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{lo}]",
            "fld dword ptr [{hi}]",
            "fcom st(1)",
            "fnstsw ax",
            "fstp st(0)",
            "fstp st(0)",
            lo = in(reg) lo,
            hi = in(reg) hi,
            out("eax") sw,
            options(nostack),
        );
    }
    sw & 0xFFFF
}

// --- the port table ----------------------------------------------------------------------------------

type Handler = unsafe fn(&mut Pushad, *const u32);
type Ranges = fn(&Pushad, *const u32, &mut Vec<(usize, usize)>);

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum Mode {
    /// Reads only: compare the result registers.
    Pure,
    /// Writes known ranges (and its callees are idempotent): snapshot shadow.
    Snap,
    /// Callees with side effects: counters only.
    Count,
}

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    /// Offsets of the `call` / `jmp rel32` instructions whose target is outside the body.
    ext: &'static [usize],
    mode: Mode,
    /// EAX is a result (compared in the shadow).
    eax_out: bool,
    ranges: Option<Ranges>,
    handler: Handler,
    thunk: extern "C" fn(),
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

impl Port {
    #[allow(clippy::too_many_arguments)]
    const fn new(
        name: &'static str,
        addr: usize,
        body: &'static [u8],
        ext: &'static [usize],
        mode: Mode,
        eax_out: bool,
        ranges: Option<Ranges>,
        handler: Handler,
        thunk: extern "C" fn(),
    ) -> Port {
        Port {
            name,
            addr,
            body,
            ext,
            mode,
            eax_out,
            ranges,
            handler,
            thunk,
            calls: AtomicU32::new(0),
            mismatches: AtomicU32::new(0),
            original: AtomicUsize::new(0),
        }
    }
}

/// A naked thunk: reserve the original's frame ($fr bytes), `pushad`, save XMM0-7, then
/// `f(&mut block, &stack args)`, then reload XMM0-7 and `popad`. The Rust body plays the original's
/// register effects on the block.
macro_rules! regs_thunk {
    ($f:ident, $ret:literal, $fr:literal) => {
        #[unsafe(naked)]
        pub extern "C" fn thunk() {
            std::arch::naked_asm!(
                "sub esp, {fr}",
                "pushad",
                "sub esp, 128",
                "movups [esp], xmm0", "movups [esp + 16], xmm1", "movups [esp + 32], xmm2", "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4", "movups [esp + 80], xmm5", "movups [esp + 96], xmm6", "movups [esp + 112], xmm7",
                "mov eax, esp",
                "lea ecx, [esp + 164 + {fr}]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp]", "movups xmm1, [esp + 16]", "movups xmm2, [esp + 32]", "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]", "movups xmm5, [esp + 80]", "movups xmm6, [esp + 96]", "movups xmm7, [esp + 112]",
                "add esp, 128",
                "popad",
                "add esp, {fr}",
                $ret,
                f = sym $f,
                fr = const $fr,
            )
        }
    };
}

macro_rules! ports {
    ($([$id:ident, $name:literal, $addr:expr, $body:ident, $ext:ident, $mode:expr, $eax_out:expr, $ranges:expr, $h:ident, $fr:literal]),* $(,)?) => {
        #[allow(non_camel_case_types, dead_code, clippy::enum_variant_names)]
        #[repr(usize)]
        enum Id { $($id),* }
        $(
            #[allow(non_snake_case)]
            mod $id {
                pub extern "C" fn entry(r: &mut super::Pushad, a: *const u32) {
                    unsafe { super::dispatch(super::Id::$id as usize, r, a) }
                }
                regs_thunk!(entry, "ret", $fr);
            }
        )*
        const COUNT: usize = [$(stringify!($id)),*].len();
        static PORTS: [Port; COUNT] = [
            $(Port::new($name, $addr, &$body, &$ext, $mode, $eax_out, $ranges, $h, $id::thunk)),*
        ];
    };
}

/// The entry of every port.
unsafe fn dispatch(id: usize, p: &mut Pushad, a: *const u32) {
    let port = &PORTS[id];
    let n = port.calls.load(Ordering::Relaxed);
    if port.mode == Mode::Count {
        port.calls.store(n.wrapping_add(1), Ordering::Relaxed);
        unsafe { (port.handler)(p, a) };
        return;
    }
    let orig = port.original.load(Ordering::Relaxed);
    if n >= SHADOW_CALLS || orig == 0 {
        unsafe { (port.handler)(p, a) };
        return;
    }
    port.calls.store(n + 1, Ordering::Relaxed);
    if n + 1 == SHADOW_CALLS {
        log(&format!("port_small_data: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", port.name, port.mismatches.load(Ordering::Relaxed)));
    }
    unsafe { shadow(port, n, orig, p, a) };
}

fn mismatch(port: &Port, n: u32, what: String) {
    let m = port.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_small_data: {} mismatch (call {n}): {what}", port.name));
    }
}

unsafe fn bytes_of(r: (usize, usize)) -> Vec<u8> {
    unsafe { std::slice::from_raw_parts(r.0 as *const u8, r.1).to_vec() }
}

unsafe fn put_back(r: (usize, usize), b: &[u8]) {
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), r.0 as *mut u8, r.1) };
}

/// The shadow (see the module comment): the original's result and writes win on a mismatch.
unsafe fn shadow(port: &Port, n: u32, orig: usize, p: &mut Pushad, a: *const u32) {
    let args = unsafe { fr_tail(a) };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    if let Some(f) = port.ranges {
        f(p, a, &mut ranges);
    }
    let saved: Vec<Vec<u8>> = ranges.iter().map(|&r| unsafe { bytes_of(r) }).collect();
    let o_eax = unsafe { call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, &args) };
    let o_mem: Vec<Vec<u8>> = ranges.iter().map(|&r| unsafe { bytes_of(r) }).collect();
    for (r, b) in ranges.iter().zip(&saved) {
        unsafe { put_back(*r, b) };
    }
    let mut q = *p;
    unsafe { (port.handler)(&mut q, a) };
    let x_mem: Vec<Vec<u8>> = ranges.iter().map(|&r| unsafe { bytes_of(r) }).collect();
    let mem_ok = o_mem == x_mem;
    let eax_ok = !port.eax_out || q.eax == o_eax;
    if mem_ok && eax_ok {
        *p = q;
        return;
    }
    mismatch(port, n, format!("eax original {o_eax:#x} Rust {:#x}, memory equal: {mem_ok}", q.eax));
    for (r, b) in ranges.iter().zip(&o_mem) {
        unsafe { put_back(*r, b) };
    }
    p.eax = o_eax;
}

/// A relocated copy of an original body: the external `call` / `jmp rel32`s (at `ext`) keep their
/// destinations.
unsafe fn relocate(addr: usize, body: &[u8], ext: &[usize]) -> Result<usize, detour::DetourError> {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
    let cur = unsafe { std::slice::from_raw_parts(addr as *const u8, body.len()) };
    if cur != body {
        return Err(detour::DetourError::PrologueMismatch { found: cur.to_vec() });
    }
    let copy = unsafe { VirtualAlloc(std::ptr::null(), body.len(), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
    if copy.is_null() {
        return Err(detour::DetourError::Alloc);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(body.as_ptr(), copy, body.len());
        for &o in ext {
            let rel = i32::from_le_bytes(body[o + 1..o + 5].try_into().unwrap());
            let dest = (addr + o + 5).wrapping_add(rel as usize);
            let new = (dest as i32).wrapping_sub((copy as usize + o + 5) as i32);
            std::ptr::copy_nonoverlapping(new.to_le_bytes().as_ptr(), copy.add(o + 1), 4);
        }
    }
    Ok(copy as usize)
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    // The shadow copies first: replacing patches the entry.
    for port in PORTS.iter() {
        if port.mode == Mode::Count || SHADOW_CALLS == 0 {
            continue;
        }
        match unsafe { relocate(port.addr, port.body, port.ext) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_small_data: {}: no shadow copy: {e:?}", port.name)),
        }
    }
    let mut ok = 0;
    for port in PORTS.iter() {
        if replace_pinned(port.name, port.addr, port.body, port.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_small_data: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_small_data: {}", parts.join(", ")) }
}

// --- the originals' bodies (pinned by replace_pinned) ---
// gameplay_constants_parse_death_and_busted @ 0x54d150, 151 bytes
const B_GC_DEATH: [u8; 151] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xF0, 0x45, 0xDE, 0x00, 0x50, 0xE8, 0xC3, 0xBE, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x6F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x04,
    0x46, 0xDE, 0x00, 0xBE, 0x3C, 0x74, 0x79, 0x02, 0xE8, 0x63, 0x7D, 0x62, 0x00, 0x57, 0xB9, 0x10,
    0x46, 0xDE, 0x00, 0xBE, 0x40, 0x74, 0x79, 0x02, 0xE8, 0x53, 0x7D, 0x62, 0x00, 0x57, 0xB9, 0x1C,
    0x46, 0xDE, 0x00, 0xBE, 0x44, 0x74, 0x79, 0x02, 0xE8, 0x23, 0x76, 0x62, 0x00, 0x57, 0xB9, 0x2C,
    0x46, 0xDE, 0x00, 0xBE, 0x48, 0x74, 0x79, 0x02, 0xE8, 0x13, 0x76, 0x62, 0x00, 0x57, 0xB9, 0x40,
    0x46, 0xDE, 0x00, 0xBE, 0x4C, 0x74, 0x79, 0x02, 0xE8, 0x03, 0x76, 0x62, 0x00, 0x57, 0xB9, 0x54,
    0x46, 0xDE, 0x00, 0xBE, 0x50, 0x74, 0x79, 0x02, 0xE8, 0xF3, 0x75, 0x62, 0x00, 0x83, 0xC4, 0x18,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0x96,
];
const X_GC_DEATH: [usize; 7] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x78, 0x88];
// gameplay_constants_parse_fat_bones @ 0x54d1f0, 123 bytes
const B_GC_FAT_BONES: [u8; 123] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x74, 0x46, 0xDE, 0x00, 0x50, 0xE8, 0x23, 0xBE, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x53, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x80,
    0x46, 0xDE, 0x00, 0xBE, 0x58, 0x74, 0x79, 0x02, 0xE8, 0xC3, 0x7C, 0x62, 0x00, 0xD9, 0x05, 0x58,
    0x74, 0x79, 0x02, 0xDC, 0x0D, 0xE8, 0x50, 0xDD, 0x00, 0x57, 0xB9, 0x90, 0x46, 0xDE, 0x00, 0xBE,
    0x54, 0x74, 0x79, 0x02, 0xD9, 0x1D, 0x58, 0x74, 0x79, 0x02, 0xE8, 0xA1, 0x7C, 0x62, 0x00, 0xD9,
    0x05, 0x54, 0x74, 0x79, 0x02, 0x83, 0xC4, 0x08, 0xDC, 0x0D, 0xE8, 0x50, 0xDD, 0x00, 0x5F, 0x5E,
    0xD9, 0x1D, 0x54, 0x74, 0x79, 0x02, 0xC3, 0x8B, 0xFE, 0xEB, 0xB2,
];
const X_GC_FAT_BONES: [usize; 3] = [0x18, 0x38, 0x5a];
// gameplay_constants_parse_drunk_sway @ 0x54d270, 103 bytes
const B_GC_DRUNK: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xA0, 0x46, 0xDE, 0x00, 0x50, 0xE8, 0xA3, 0xBD, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x10,
    0x36, 0xDE, 0x00, 0xBE, 0x30, 0x74, 0x79, 0x02, 0xE8, 0x43, 0x7C, 0x62, 0x00, 0x57, 0xB9, 0xAC,
    0x46, 0xDE, 0x00, 0xBE, 0x38, 0x74, 0x79, 0x02, 0xE8, 0x33, 0x7C, 0x62, 0x00, 0x57, 0xB9, 0xC0,
    0x46, 0xDE, 0x00, 0xBE, 0x34, 0x74, 0x79, 0x02, 0xE8, 0x23, 0x7C, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_DRUNK: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_hit_by_car @ 0x54d2e0, 190 bytes
const B_GC_HIT_CAR: [u8; 190] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x29, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x22, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xD0, 0x46, 0xDE, 0x00, 0x50, 0xE8, 0x33, 0xBD, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x0F, 0x84, 0x8F, 0x00, 0x00, 0x00, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE1, 0x33,
    0xFF, 0x57, 0xB9, 0xDC, 0x46, 0xDE, 0x00, 0xBE, 0x5C, 0x74, 0x79, 0x02, 0xE8, 0xCF, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0xFC, 0x46, 0xDE, 0x00, 0xBE, 0x60, 0x74, 0x79, 0x02, 0xE8, 0xBF, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x1C, 0x47, 0xDE, 0x00, 0xBE, 0x64, 0x74, 0x79, 0x02, 0xE8, 0xAF, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x3C, 0x47, 0xDE, 0x00, 0xBE, 0x68, 0x74, 0x79, 0x02, 0xE8, 0x9F, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x54, 0x47, 0xDE, 0x00, 0xBE, 0x6C, 0x74, 0x79, 0x02, 0xE8, 0x8F, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x6C, 0x47, 0xDE, 0x00, 0xBE, 0x70, 0x74, 0x79, 0x02, 0xE8, 0x7F, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x80, 0x47, 0xDE, 0x00, 0xBE, 0x74, 0x74, 0x79, 0x02, 0xE8, 0x6F, 0x7B, 0x62,
    0x00, 0x57, 0xB9, 0x98, 0x47, 0xDE, 0x00, 0xBE, 0x78, 0x74, 0x79, 0x02, 0xE8, 0x5F, 0x7B, 0x62,
    0x00, 0x83, 0xC4, 0x20, 0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xE9, 0x73, 0xFF, 0xFF, 0xFF,
];
const X_GC_HIT_CAR: [usize; 9] = [0x18, 0x3c, 0x4c, 0x5c, 0x6c, 0x7c, 0x8c, 0x9c, 0xac];
// gameplay_constants_parse_vehicle_collision @ 0x54d3a0, 119 bytes
const B_GC_VEH_COLL: [u8; 119] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xA8, 0x47, 0xDE, 0x00, 0x50, 0xE8, 0x73, 0xBC, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x4F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xBC,
    0x47, 0xDE, 0x00, 0xBE, 0x7C, 0x74, 0x79, 0x02, 0xE8, 0x13, 0x7B, 0x62, 0x00, 0x57, 0xB9, 0xE0,
    0x47, 0xDE, 0x00, 0xBE, 0x80, 0x74, 0x79, 0x02, 0xE8, 0x03, 0x7B, 0x62, 0x00, 0x57, 0xB9, 0x00,
    0x48, 0xDE, 0x00, 0xBE, 0x84, 0x74, 0x79, 0x02, 0xE8, 0xF3, 0x7A, 0x62, 0x00, 0x57, 0xB9, 0x24,
    0x48, 0xDE, 0x00, 0xBE, 0x88, 0x74, 0x79, 0x02, 0xE8, 0xE3, 0x7A, 0x62, 0x00, 0x83, 0xC4, 0x10,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xB6,
];
const X_GC_VEH_COLL: [usize; 5] = [0x18, 0x38, 0x48, 0x58, 0x68];
// gameplay_constants_parse_fire @ 0x54d420, 190 bytes
const B_GC_FIRE: [u8; 190] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x29, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x22, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x40, 0x48, 0xDE, 0x00, 0x50, 0xE8, 0xF3, 0xBB, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x0F, 0x84, 0x8F, 0x00, 0x00, 0x00, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE1, 0x33,
    0xFF, 0x57, 0xB9, 0x48, 0x48, 0xDE, 0x00, 0xBE, 0x8C, 0x74, 0x79, 0x02, 0xE8, 0x8F, 0x7A, 0x62,
    0x00, 0x57, 0xB9, 0x60, 0x48, 0xDE, 0x00, 0xBE, 0x90, 0x74, 0x79, 0x02, 0xE8, 0x5F, 0x73, 0x62,
    0x00, 0x57, 0xB9, 0x78, 0x48, 0xDE, 0x00, 0xBE, 0x94, 0x74, 0x79, 0x02, 0xE8, 0x4F, 0x73, 0x62,
    0x00, 0x57, 0xB9, 0x90, 0x48, 0xDE, 0x00, 0xBE, 0x98, 0x74, 0x79, 0x02, 0xE8, 0x3F, 0x73, 0x62,
    0x00, 0x57, 0xB9, 0xA8, 0x48, 0xDE, 0x00, 0xBE, 0x9C, 0x74, 0x79, 0x02, 0xE8, 0x4F, 0x7A, 0x62,
    0x00, 0x57, 0xB9, 0xC4, 0x48, 0xDE, 0x00, 0xBE, 0xA0, 0x74, 0x79, 0x02, 0xE8, 0x1F, 0x73, 0x62,
    0x00, 0x57, 0xB9, 0xE0, 0x48, 0xDE, 0x00, 0xBE, 0xA4, 0x74, 0x79, 0x02, 0xE8, 0x0F, 0x73, 0x62,
    0x00, 0x57, 0xB9, 0xF8, 0x48, 0xDE, 0x00, 0xBE, 0xA8, 0x74, 0x79, 0x02, 0xE8, 0xFF, 0x72, 0x62,
    0x00, 0x83, 0xC4, 0x20, 0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xE9, 0x73, 0xFF, 0xFF, 0xFF,
];
const X_GC_FIRE: [usize; 9] = [0x18, 0x3c, 0x4c, 0x5c, 0x6c, 0x7c, 0x8c, 0x9c, 0xac];
// gameplay_constants_parse_sticky_fire @ 0x54d5f0, 125 bytes
const B_GC_STICKY: [u8; 125] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xFC, 0x49, 0xDE, 0x00, 0x50, 0xE8, 0x23, 0xBA, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x55, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xC0,
    0xBE, 0xDD, 0x00, 0xBE, 0xC4, 0x74, 0x79, 0x02, 0xE8, 0xC3, 0x78, 0x62, 0x00, 0x57, 0xB9, 0xF8,
    0xBE, 0xDD, 0x00, 0xBE, 0xC8, 0x74, 0x79, 0x02, 0xE8, 0xB3, 0x78, 0x62, 0x00, 0x57, 0xB9, 0x30,
    0xBF, 0xDD, 0x00, 0xBE, 0xCC, 0x74, 0x79, 0x02, 0xE8, 0xA3, 0x78, 0x62, 0x00, 0x57, 0xB9, 0x68,
    0xBF, 0xDD, 0x00, 0xBE, 0xD0, 0x74, 0x79, 0x02, 0xE8, 0x93, 0x78, 0x62, 0x00, 0x83, 0xC4, 0x10,
    0x8B, 0xC7, 0x5F, 0x5E, 0xE9, 0x17, 0xFF, 0xFF, 0xFF, 0x8B, 0xFE, 0xEB, 0xB0,
];
const X_GC_STICKY: [usize; 6] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x74];
// gameplay_constants_parse_ho_pimp_ai @ 0x54d8d0, 103 bytes
const B_GC_PIMP_AI: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x60, 0x4B, 0xDE, 0x00, 0x50, 0xE8, 0x43, 0xB7, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x6C,
    0x4B, 0xDE, 0x00, 0xBE, 0xC0, 0x76, 0x79, 0x02, 0xE8, 0xE3, 0x75, 0x62, 0x00, 0x57, 0xB9, 0x78,
    0x4B, 0xDE, 0x00, 0xBE, 0xC4, 0x76, 0x79, 0x02, 0xE8, 0xD3, 0x75, 0x62, 0x00, 0x57, 0xB9, 0x90,
    0x4B, 0xDE, 0x00, 0xBE, 0xC8, 0x76, 0x79, 0x02, 0xE8, 0xC3, 0x75, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_PIMP_AI: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_throwing_constants @ 0x54db20, 119 bytes
const B_GC_THROWING: [u8; 119] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x6C, 0x4C, 0xDE, 0x00, 0x50, 0xE8, 0xF3, 0xB4, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x4F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x80,
    0x4C, 0xDE, 0x00, 0xBE, 0xFC, 0x76, 0x79, 0x02, 0xE8, 0x93, 0x73, 0x62, 0x00, 0x57, 0xB9, 0x98,
    0x4C, 0xDE, 0x00, 0xBE, 0x00, 0x77, 0x79, 0x02, 0xE8, 0x83, 0x73, 0x62, 0x00, 0x57, 0xB9, 0xAC,
    0x4C, 0xDE, 0x00, 0xBE, 0x04, 0x77, 0x79, 0x02, 0xE8, 0x73, 0x73, 0x62, 0x00, 0x57, 0xB9, 0xBC,
    0x4C, 0xDE, 0x00, 0xBE, 0x08, 0x77, 0x79, 0x02, 0xE8, 0x63, 0x73, 0x62, 0x00, 0x83, 0xC4, 0x10,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xB6,
];
const X_GC_THROWING: [usize; 5] = [0x18, 0x38, 0x48, 0x58, 0x68];
// gameplay_constants_parse_item_drops @ 0x54dba0, 141 bytes
const B_GC_ITEM_DROPS: [u8; 141] = [
    0x51, 0x85, 0xC0, 0x56, 0x57, 0x74, 0x24, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1D, 0x8B, 0xFF,
    0x8B, 0x06, 0x68, 0xD0, 0x4C, 0xDE, 0x00, 0x50, 0xE8, 0x73, 0xB4, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x65, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xDC,
    0x4C, 0xDE, 0x00, 0x8D, 0x74, 0x24, 0x0C, 0xE8, 0xF4, 0x6B, 0x62, 0x00, 0xDB, 0x44, 0x24, 0x0C,
    0x57, 0xB9, 0xF0, 0x4C, 0xDE, 0x00, 0xDC, 0x0D, 0xF0, 0xC4, 0xE5, 0x00, 0xD9, 0x1D, 0x0C, 0x77,
    0x79, 0x02, 0xE8, 0xD9, 0x6B, 0x62, 0x00, 0xDB, 0x44, 0x24, 0x10, 0x57, 0xB9, 0x08, 0x4D, 0xDE,
    0x00, 0xDC, 0x0D, 0xF0, 0xC4, 0xE5, 0x00, 0xD9, 0x1D, 0x10, 0x77, 0x79, 0x02, 0xE8, 0xBE, 0x6B,
    0x62, 0x00, 0xDB, 0x44, 0x24, 0x14, 0x83, 0xC4, 0x0C, 0x5F, 0x5E, 0xDC, 0x0D, 0xF0, 0xC4, 0xE5,
    0x00, 0xD9, 0x1D, 0x14, 0x77, 0x79, 0x02, 0x59, 0xC3, 0x8B, 0xFE, 0xEB, 0xA0,
];
const X_GC_ITEM_DROPS: [usize; 4] = [0x18, 0x37, 0x52, 0x6d];
// gameplay_constants_parse_shader_lod @ 0x54df50, 87 bytes
const B_GC_SHADER_LOD: [u8; 87] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xF0, 0x4E, 0xDE, 0x00, 0x50, 0xE8, 0xC3, 0xB0, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x2F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x00,
    0xE0, 0xDD, 0x00, 0xBE, 0x84, 0x77, 0x79, 0x02, 0xE8, 0x43, 0x68, 0x62, 0x00, 0x57, 0xB9, 0xFC,
    0x4E, 0xDE, 0x00, 0xBE, 0x88, 0x77, 0x79, 0x02, 0xE8, 0x33, 0x68, 0x62, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xD6,
];
const X_GC_SHADER_LOD: [usize; 3] = [0x18, 0x38, 0x48];
// gameplay_constants_parse_shadows @ 0x54dfb0, 151 bytes
const B_GC_SHADOWS: [u8; 151] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x08, 0x4F, 0xDE, 0x00, 0x50, 0xE8, 0x63, 0xB0, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x6F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x10,
    0x4F, 0xDE, 0x00, 0xBE, 0x8C, 0x77, 0x79, 0x02, 0xE8, 0xE3, 0x67, 0x62, 0x00, 0x57, 0xB9, 0x20,
    0x4F, 0xDE, 0x00, 0xBE, 0x90, 0x77, 0x79, 0x02, 0xE8, 0xD3, 0x67, 0x62, 0x00, 0x57, 0xB9, 0x34,
    0x4F, 0xDE, 0x00, 0xBE, 0x94, 0x77, 0x79, 0x02, 0xE8, 0xC3, 0x67, 0x62, 0x00, 0x57, 0xB9, 0x50,
    0x4F, 0xDE, 0x00, 0xBE, 0x98, 0x77, 0x79, 0x02, 0xE8, 0xB3, 0x67, 0x62, 0x00, 0x57, 0xB9, 0x6C,
    0x4F, 0xDE, 0x00, 0xBE, 0x9C, 0x77, 0x79, 0x02, 0xE8, 0xA3, 0x67, 0x62, 0x00, 0x57, 0xB9, 0x88,
    0x4F, 0xDE, 0x00, 0xBE, 0xA0, 0x77, 0x79, 0x02, 0xE8, 0x93, 0x67, 0x62, 0x00, 0x83, 0xC4, 0x18,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0x96,
];
const X_GC_SHADOWS: [usize; 7] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x78, 0x88];
// gameplay_constants_parse_controlpatterns @ 0x54e140, 87 bytes
const B_GC_CTRLPAT: [u8; 87] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xCC, 0x50, 0xDE, 0x00, 0x50, 0xE8, 0xD3, 0xAE, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x2F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xDC,
    0x50, 0xDE, 0x00, 0xBE, 0xD0, 0x77, 0x79, 0x02, 0xE8, 0x93, 0x67, 0x62, 0x00, 0x57, 0xB9, 0xEC,
    0x50, 0xDE, 0x00, 0xBE, 0xD4, 0x77, 0x79, 0x02, 0xE8, 0x63, 0x6D, 0x62, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xD6,
];
const X_GC_CTRLPAT: [usize; 3] = [0x18, 0x38, 0x48];
// gameplay_constants_parse_tauntreactions @ 0x54e1a0, 103 bytes
const B_GC_TAUNT: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x00, 0x51, 0xDE, 0x00, 0x50, 0xE8, 0x73, 0xAE, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x10,
    0x51, 0xDE, 0x00, 0xBE, 0xD8, 0x77, 0x79, 0x02, 0xE8, 0x13, 0x6D, 0x62, 0x00, 0x57, 0xB9, 0x20,
    0x51, 0xDE, 0x00, 0xBE, 0xDC, 0x77, 0x79, 0x02, 0xE8, 0x03, 0x6D, 0x62, 0x00, 0x57, 0xB9, 0x30,
    0x51, 0xDE, 0x00, 0xBE, 0xE0, 0x77, 0x79, 0x02, 0xE8, 0xF3, 0x6C, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_TAUNT: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_human_fine_collision @ 0x54e210, 190 bytes
const B_GC_FINE_COLL: [u8; 190] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x29, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x22, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x40, 0x51, 0xDE, 0x00, 0x50, 0xE8, 0x03, 0xAE, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x0F, 0x84, 0x8F, 0x00, 0x00, 0x00, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE1, 0x33,
    0xFF, 0x57, 0xB9, 0x58, 0x51, 0xDE, 0x00, 0xBE, 0xD4, 0x84, 0xE8, 0x00, 0xE8, 0x9F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0x64, 0x51, 0xDE, 0x00, 0xBE, 0xD8, 0x84, 0xE8, 0x00, 0xE8, 0x8F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0x70, 0x51, 0xDE, 0x00, 0xBE, 0xE0, 0x84, 0xE8, 0x00, 0xE8, 0x7F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0x7C, 0x51, 0xDE, 0x00, 0xBE, 0xDC, 0x84, 0xE8, 0x00, 0xE8, 0x6F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0x88, 0x51, 0xDE, 0x00, 0xBE, 0xEC, 0x84, 0xE8, 0x00, 0xE8, 0x5F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0x98, 0x51, 0xDE, 0x00, 0xBE, 0xF0, 0x84, 0xE8, 0x00, 0xE8, 0x4F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0xA8, 0x51, 0xDE, 0x00, 0xBE, 0xE4, 0x84, 0xE8, 0x00, 0xE8, 0x3F, 0x6C, 0x62,
    0x00, 0x57, 0xB9, 0xB8, 0x51, 0xDE, 0x00, 0xBE, 0xE8, 0x84, 0xE8, 0x00, 0xE8, 0x2F, 0x6C, 0x62,
    0x00, 0x83, 0xC4, 0x20, 0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xE9, 0x73, 0xFF, 0xFF, 0xFF,
];
const X_GC_FINE_COLL: [usize; 9] = [0x18, 0x3c, 0x4c, 0x5c, 0x6c, 0x7c, 0x8c, 0x9c, 0xac];
// gameplay_constants_parse_flamethrower @ 0x54f430, 103 bytes
const B_GC_FLAME: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xBC, 0x53, 0xDE, 0x00, 0x50, 0xE8, 0xE3, 0x9B, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xCC,
    0x53, 0xDE, 0x00, 0xBE, 0xE8, 0x79, 0x79, 0x02, 0xE8, 0x63, 0x53, 0x62, 0x00, 0x57, 0xB9, 0xE8,
    0x53, 0xDE, 0x00, 0xBE, 0xEC, 0x79, 0x79, 0x02, 0xE8, 0x53, 0x53, 0x62, 0x00, 0x57, 0xB9, 0xFC,
    0x53, 0xDE, 0x00, 0xBE, 0xF0, 0x79, 0x79, 0x02, 0xE8, 0x43, 0x53, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_FLAME: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_vehicle_interaction @ 0x54f6d0, 87 bytes
const B_GC_VEH_INTER: [u8; 87] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x88, 0x54, 0xDE, 0x00, 0x50, 0xE8, 0x43, 0x99, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x2F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x9C,
    0x54, 0xDE, 0x00, 0xBE, 0x30, 0x7A, 0x79, 0x02, 0xE8, 0xC3, 0x50, 0x62, 0x00, 0x57, 0xB9, 0xB8,
    0x54, 0xDE, 0x00, 0xBE, 0x34, 0x7A, 0x79, 0x02, 0xE8, 0xB3, 0x50, 0x62, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xD6,
];
const X_GC_VEH_INTER: [usize; 3] = [0x18, 0x38, 0x48];
// gameplay_constants_parse_controls @ 0x54f730, 66 bytes
const B_GC_CONTROLS: [u8; 66] = [
    0x85, 0xC0, 0x56, 0x74, 0x26, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1F, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x06, 0x68, 0xD4, 0x54, 0xDE, 0x00, 0x50, 0xE8, 0xE3, 0x98, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xF6, 0x56, 0xB9, 0xE0,
    0x54, 0xDE, 0x00, 0xBE, 0x38, 0x7A, 0x79, 0x02, 0xE8, 0x63, 0x50, 0x62, 0x00, 0x83, 0xC4, 0x04,
    0x5E, 0xC3,
];
const X_GC_CONTROLS: [usize; 2] = [0x18, 0x38];
// gameplay_constants_parse_stungun @ 0x54f780, 87 bytes
const B_GC_STUNGUN: [u8; 87] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xF8, 0x54, 0xDE, 0x00, 0x50, 0xE8, 0x93, 0x98, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x2F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x00,
    0x55, 0xDE, 0x00, 0xBE, 0x3C, 0x7A, 0x79, 0x02, 0xE8, 0x13, 0x50, 0x62, 0x00, 0x57, 0xB9, 0x10,
    0x55, 0xDE, 0x00, 0xBE, 0x40, 0x7A, 0x79, 0x02, 0xE8, 0x03, 0x50, 0x62, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xD6,
];
const X_GC_STUNGUN: [usize; 3] = [0x18, 0x38, 0x48];
// gameplay_constants_parse_rocket_accuracy @ 0x54f7e0, 103 bytes
const B_GC_ROCKET: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x24, 0x55, 0xDE, 0x00, 0x50, 0xE8, 0x33, 0x98, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x34,
    0x55, 0xDE, 0x00, 0xBE, 0x44, 0x7A, 0x79, 0x02, 0xE8, 0xD3, 0x56, 0x62, 0x00, 0x57, 0xB9, 0x50,
    0x55, 0xDE, 0x00, 0xBE, 0x48, 0x7A, 0x79, 0x02, 0xE8, 0xC3, 0x56, 0x62, 0x00, 0x57, 0xB9, 0x6C,
    0x55, 0xDE, 0x00, 0xBE, 0x4C, 0x7A, 0x79, 0x02, 0xE8, 0xB3, 0x56, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_ROCKET: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_activity_constants @ 0x54f850, 119 bytes
const B_GC_ACTIVITY: [u8; 119] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x88, 0x55, 0xDE, 0x00, 0x50, 0xE8, 0xC3, 0x97, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x4F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x9C,
    0x55, 0xDE, 0x00, 0xBE, 0x54, 0x7A, 0x79, 0x02, 0xE8, 0x63, 0x56, 0x62, 0x00, 0x57, 0xB9, 0xB4,
    0x55, 0xDE, 0x00, 0xBE, 0x58, 0x7A, 0x79, 0x02, 0xE8, 0x53, 0x56, 0x62, 0x00, 0x57, 0xB9, 0xD4,
    0x55, 0xDE, 0x00, 0xBE, 0x5C, 0x7A, 0x79, 0x02, 0xE8, 0x43, 0x56, 0x62, 0x00, 0x57, 0xB9, 0xEC,
    0x55, 0xDE, 0x00, 0xBE, 0x60, 0x7A, 0x79, 0x02, 0xE8, 0x33, 0x56, 0x62, 0x00, 0x83, 0xC4, 0x10,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xB6,
];
const X_GC_ACTIVITY: [usize; 5] = [0x18, 0x38, 0x48, 0x58, 0x68];
// gameplay_constants_parse_kneecapper @ 0x54f9a0, 119 bytes
const B_GC_KNEECAP: [u8; 119] = [
    0x51, 0x85, 0xC0, 0x56, 0x57, 0x74, 0x24, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1D, 0x8B, 0xFF,
    0x8B, 0x06, 0x68, 0x7C, 0x56, 0xDE, 0x00, 0x50, 0xE8, 0x73, 0x96, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x4F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x88,
    0x56, 0xDE, 0x00, 0x8D, 0x74, 0x24, 0x0C, 0xE8, 0x14, 0x55, 0x62, 0x00, 0xD9, 0x44, 0x24, 0x0C,
    0xDC, 0x0D, 0x98, 0xCA, 0xE5, 0x00, 0x57, 0xB9, 0x98, 0x56, 0xDE, 0x00, 0xBE, 0x80, 0x7A, 0x79,
    0x02, 0xD9, 0x1D, 0x7C, 0x7A, 0x79, 0x02, 0xE8, 0x14, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0xA4, 0x56,
    0xDE, 0x00, 0xBE, 0x84, 0x7A, 0x79, 0x02, 0xE8, 0x04, 0x4F, 0x62, 0x00, 0x83, 0xC4, 0x0C, 0x5F,
    0x5E, 0x59, 0xC3, 0x8B, 0xFE, 0xEB, 0xB6,
];
const X_GC_KNEECAP: [usize; 4] = [0x18, 0x37, 0x57, 0x67];
// gameplay_constants_parse_vehicle_evade_ai @ 0x54fc20, 119 bytes
const B_GC_EVADE: [u8; 119] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xF8, 0x57, 0xDE, 0x00, 0x50, 0xE8, 0xF3, 0x93, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x4F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x0C,
    0x58, 0xDE, 0x00, 0xBE, 0xC8, 0x7A, 0x79, 0x02, 0xE8, 0x93, 0x52, 0x62, 0x00, 0x57, 0xB9, 0x1C,
    0x58, 0xDE, 0x00, 0xBE, 0xCC, 0x7A, 0x79, 0x02, 0xE8, 0x83, 0x52, 0x62, 0x00, 0x57, 0xB9, 0x28,
    0x58, 0xDE, 0x00, 0xBE, 0xD0, 0x7A, 0x79, 0x02, 0xE8, 0x73, 0x52, 0x62, 0x00, 0x57, 0xB9, 0x38,
    0x58, 0xDE, 0x00, 0xBE, 0xD4, 0x7A, 0x79, 0x02, 0xE8, 0x63, 0x52, 0x62, 0x00, 0x83, 0xC4, 0x10,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xB6,
];
const X_GC_EVADE: [usize; 5] = [0x18, 0x38, 0x48, 0x58, 0x68];
// gameplay_constants_parse_pepperspray @ 0x54fec0, 151 bytes
const B_GC_PEPPER: [u8; 151] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0xB4, 0x59, 0xDE, 0x00, 0x50, 0xE8, 0x53, 0x91, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x6F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xC0,
    0x59, 0xDE, 0x00, 0xBE, 0x18, 0x7B, 0x79, 0x02, 0xE8, 0xF3, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0xD0,
    0x59, 0xDE, 0x00, 0xBE, 0x1C, 0x7B, 0x79, 0x02, 0xE8, 0xE3, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0xE0,
    0x59, 0xDE, 0x00, 0xBE, 0x28, 0x7B, 0x79, 0x02, 0xE8, 0xD3, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0xF0,
    0x59, 0xDE, 0x00, 0xBE, 0x2C, 0x7B, 0x79, 0x02, 0xE8, 0xC3, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0x00,
    0x5A, 0xDE, 0x00, 0xBE, 0x20, 0x7B, 0x79, 0x02, 0xE8, 0xB3, 0x4F, 0x62, 0x00, 0x57, 0xB9, 0x14,
    0x5A, 0xDE, 0x00, 0xBE, 0x24, 0x7B, 0x79, 0x02, 0xE8, 0xA3, 0x4F, 0x62, 0x00, 0x83, 0xC4, 0x18,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0x96,
];
const X_GC_PEPPER: [usize; 7] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x78, 0x88];
// gameplay_constants_parse_wieldable_prop_throw @ 0x5501a0, 151 bytes
const B_GC_PROP_THROW: [u8; 151] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x98, 0x5B, 0xDE, 0x00, 0x50, 0xE8, 0x73, 0x8E, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x6F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0xB0,
    0x5B, 0xDE, 0x00, 0xBE, 0x84, 0x7B, 0x79, 0x02, 0xE8, 0x13, 0x4D, 0x62, 0x00, 0x57, 0xB9, 0xCC,
    0x5B, 0xDE, 0x00, 0xBE, 0x88, 0x7B, 0x79, 0x02, 0xE8, 0x03, 0x4D, 0x62, 0x00, 0x57, 0xB9, 0xE8,
    0x5B, 0xDE, 0x00, 0xBE, 0x8C, 0x7B, 0x79, 0x02, 0xE8, 0xF3, 0x4C, 0x62, 0x00, 0x57, 0xB9, 0xF4,
    0x5B, 0xDE, 0x00, 0xBE, 0x90, 0x7B, 0x79, 0x02, 0xE8, 0xE3, 0x4C, 0x62, 0x00, 0x57, 0xB9, 0x00,
    0x5C, 0xDE, 0x00, 0xBE, 0x94, 0x7B, 0x79, 0x02, 0xE8, 0xD3, 0x4C, 0x62, 0x00, 0x57, 0xB9, 0x10,
    0x5C, 0xDE, 0x00, 0xBE, 0x98, 0x7B, 0x79, 0x02, 0xE8, 0xC3, 0x4C, 0x62, 0x00, 0x83, 0xC4, 0x18,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0x96,
];
const X_GC_PROP_THROW: [usize; 7] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x78, 0x88];
// gameplay_constants_parse_watercraft_params @ 0x550240, 135 bytes
const B_GC_WATERCRAFT: [u8; 135] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x20, 0x5C, 0xDE, 0x00, 0x50, 0xE8, 0xD3, 0x8D, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x5C, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x34,
    0x5C, 0xDE, 0x00, 0xBE, 0x9C, 0x7B, 0x79, 0x02, 0xE8, 0x73, 0x4C, 0x62, 0x00, 0x57, 0xB9, 0x48,
    0x5C, 0xDE, 0x00, 0xBE, 0xA0, 0x7B, 0x79, 0x02, 0xE8, 0x63, 0x4C, 0x62, 0x00, 0x57, 0xB9, 0x60,
    0x5C, 0xDE, 0x00, 0xBE, 0xA4, 0x7B, 0x79, 0x02, 0xE8, 0x53, 0x4C, 0x62, 0x00, 0xD9, 0x05, 0xA0,
    0x7B, 0x79, 0x02, 0xD9, 0x05, 0xA4, 0x7B, 0x79, 0x02, 0x83, 0xC4, 0x0C, 0xD8, 0xD1, 0x5F, 0x5E,
    0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x05, 0x7A, 0x0B, 0xD9, 0x1D, 0xA0, 0x7B, 0x79, 0x02, 0xC3,
    0x8B, 0xFE, 0xEB, 0xA9, 0xDD, 0xD8, 0xC3,
];
const X_GC_WATERCRAFT: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// gameplay_constants_parse_siren_whoop_params @ 0x5502d0, 151 bytes
const B_GC_SIREN: [u8; 151] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x78, 0x5C, 0xDE, 0x00, 0x50, 0xE8, 0x43, 0x8D, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x6F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x8C,
    0x5C, 0xDE, 0x00, 0xBE, 0xA8, 0x7B, 0x79, 0x02, 0xE8, 0xC3, 0x44, 0x62, 0x00, 0x57, 0xB9, 0x9C,
    0x5C, 0xDE, 0x00, 0xBE, 0xAC, 0x7B, 0x79, 0x02, 0xE8, 0xB3, 0x44, 0x62, 0x00, 0x57, 0xB9, 0xAC,
    0x5C, 0xDE, 0x00, 0xBE, 0xB0, 0x7B, 0x79, 0x02, 0xE8, 0xC3, 0x4B, 0x62, 0x00, 0x57, 0xB9, 0xC0,
    0x5C, 0xDE, 0x00, 0xBE, 0xB4, 0x7B, 0x79, 0x02, 0xE8, 0xB3, 0x4B, 0x62, 0x00, 0x57, 0xB9, 0xD4,
    0x5C, 0xDE, 0x00, 0xBE, 0xB8, 0x7B, 0x79, 0x02, 0xE8, 0xA3, 0x4B, 0x62, 0x00, 0x57, 0xB9, 0xF0,
    0x5C, 0xDE, 0x00, 0xBE, 0xBC, 0x7B, 0x79, 0x02, 0xE8, 0x93, 0x4B, 0x62, 0x00, 0x83, 0xC4, 0x18,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0x96,
];
const X_GC_SIREN: [usize; 7] = [0x18, 0x38, 0x48, 0x58, 0x68, 0x78, 0x88];
// gameplay_constants_parse_garage @ 0x550370, 66 bytes
const B_GC_GARAGE: [u8; 66] = [
    0x85, 0xC0, 0x56, 0x74, 0x26, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1F, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x06, 0x68, 0xF8, 0x3B, 0xDE, 0x00, 0x50, 0xE8, 0xA3, 0x8C, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xF6, 0x56, 0xB9, 0x08,
    0x5D, 0xDE, 0x00, 0xBE, 0xC0, 0x7B, 0x79, 0x02, 0xE8, 0x43, 0x4B, 0x62, 0x00, 0x83, 0xC4, 0x04,
    0x5E, 0xC3,
];
const X_GC_GARAGE: [usize; 2] = [0x18, 0x38];
// gameplay_constants_parse_fall_damage @ 0x5503c0, 103 bytes
const B_GC_FALL: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x57, 0x74, 0x25, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1E, 0x8D, 0x49, 0x00,
    0x8B, 0x06, 0x68, 0x1C, 0x5D, 0xDE, 0x00, 0x50, 0xE8, 0x53, 0x8C, 0x74, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x3F, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xFF, 0x57, 0xB9, 0x28,
    0x5D, 0xDE, 0x00, 0xBE, 0xC4, 0x7B, 0x79, 0x02, 0xE8, 0xF3, 0x4A, 0x62, 0x00, 0x57, 0xB9, 0x38,
    0x5D, 0xDE, 0x00, 0xBE, 0xC8, 0x7B, 0x79, 0x02, 0xE8, 0xE3, 0x4A, 0x62, 0x00, 0x57, 0xB9, 0x54,
    0x5D, 0xDE, 0x00, 0xBE, 0xCC, 0x7B, 0x79, 0x02, 0xE8, 0xD3, 0x4A, 0x62, 0x00, 0x83, 0xC4, 0x0C,
    0x5F, 0x5E, 0xC3, 0x8B, 0xFE, 0xEB, 0xC6,
];
const X_GC_FALL: [usize; 4] = [0x18, 0x38, 0x48, 0x58];
// camera_free_read_stationary_times @ 0x496110, 54 bytes
const B_CAM_STATIONARY: [u8; 54] = [
    0x56, 0x57, 0xB9, 0xD8, 0x9A, 0xDD, 0x00, 0xBE, 0xBC, 0x99, 0xE9, 0x00, 0xE8, 0xCF, 0xED, 0x6D,
    0x00, 0x57, 0xB9, 0xE8, 0x9A, 0xDD, 0x00, 0xBE, 0xC0, 0x99, 0xE9, 0x00, 0xE8, 0xBF, 0xED, 0x6D,
    0x00, 0x57, 0xB9, 0xF8, 0x9A, 0xDD, 0x00, 0xBE, 0xC4, 0x99, 0xE9, 0x00, 0xE8, 0xAF, 0xED, 0x6D,
    0x00, 0x83, 0xC4, 0x0C, 0x5E, 0xC3,
];
const X_CAM_STATIONARY: [usize; 3] = [0xc, 0x1c, 0x2c];
// camera_free_parse_crouch_y_offset @ 0x496150, 70 bytes
const B_CAM_CROUCH: [u8; 70] = [
    0x56, 0x57, 0xB9, 0x10, 0x9B, 0xDD, 0x00, 0xBE, 0xD4, 0x99, 0xE9, 0x00, 0xE8, 0x8F, 0xED, 0x6D,
    0x00, 0x57, 0xB9, 0x20, 0x9B, 0xDD, 0x00, 0xBE, 0xCC, 0x99, 0xE9, 0x00, 0xE8, 0x7F, 0xED, 0x6D,
    0x00, 0x57, 0xB9, 0x30, 0x9B, 0xDD, 0x00, 0xBE, 0xD0, 0x99, 0xE9, 0x00, 0xE8, 0x6F, 0xED, 0x6D,
    0x00, 0x57, 0xB9, 0x44, 0x9B, 0xDD, 0x00, 0xBE, 0xC8, 0x99, 0xE9, 0x00, 0xE8, 0x5F, 0xED, 0x6D,
    0x00, 0x83, 0xC4, 0x10, 0x5E, 0xC3,
];
const X_CAM_CROUCH: [usize; 4] = [0xc, 0x1c, 0x2c, 0x3c];
// camera_free_read_default_swing_rate @ 0x4961a0, 22 bytes
const B_CAM_SWING: [u8; 22] = [
    0x56, 0x50, 0xB9, 0x58, 0x9B, 0xDD, 0x00, 0xBE, 0xDC, 0x99, 0xE9, 0x00, 0xE8, 0x3F, 0xED, 0x6D,
    0x00, 0x83, 0xC4, 0x04, 0x5E, 0xC3,
];
const X_CAM_SWING: [usize; 1] = [0xc];
// camera_free_read_default_aggression @ 0x4961c0, 22 bytes
const B_CAM_AGGR: [u8; 22] = [
    0x56, 0x50, 0xB9, 0x6C, 0x9B, 0xDD, 0x00, 0xBE, 0xE0, 0x99, 0xE9, 0x00, 0xE8, 0x1F, 0xED, 0x6D,
    0x00, 0x83, 0xC4, 0x04, 0x5E, 0xC3,
];
const X_CAM_AGGR: [usize; 1] = [0xc];
// camera_free_parse_pitch_reset_time @ 0x4961e0, 170 bytes
const B_CAM_PITCH: [u8; 170] = [
    0x56, 0x57, 0xB9, 0x80, 0x9B, 0xDD, 0x00, 0xBE, 0xE4, 0x99, 0xE9, 0x00, 0xE8, 0xFF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0x94, 0x9B, 0xDD, 0x00, 0xBE, 0xE8, 0x99, 0xE9, 0x00, 0xE8, 0xEF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0xA4, 0x9B, 0xDD, 0x00, 0xBE, 0xD8, 0x39, 0xF5, 0x00, 0xE8, 0xDF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0xC0, 0x9B, 0xDD, 0x00, 0xBE, 0xA8, 0x99, 0xE9, 0x00, 0xE8, 0xCF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0xDC, 0x9B, 0xDD, 0x00, 0xBE, 0xAC, 0x99, 0xE9, 0x00, 0xE8, 0xBF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0xFC, 0x9B, 0xDD, 0x00, 0xBE, 0xB0, 0x99, 0xE9, 0x00, 0xE8, 0xAF, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0x1C, 0x9C, 0xDD, 0x00, 0xBE, 0x5C, 0x98, 0xE9, 0x00, 0xE8, 0x9F, 0xEC, 0x6D,
    0x00, 0x57, 0xB9, 0x34, 0x9C, 0xDD, 0x00, 0xBE, 0x60, 0x98, 0xE9, 0x00, 0xE8, 0x8F, 0xEC, 0x6D,
    0x00, 0xD9, 0x05, 0x5C, 0x98, 0xE9, 0x00, 0xDC, 0x0D, 0xE8, 0xC4, 0xE5, 0x00, 0x83, 0xC4, 0x20,
    0x5E, 0xD9, 0x1D, 0x5C, 0x98, 0xE9, 0x00, 0xD9, 0x05, 0xD8, 0x39, 0xF5, 0x00, 0xDC, 0x0D, 0xA8,
    0xC2, 0xE5, 0x00, 0xD9, 0x1D, 0xD8, 0x39, 0xF5, 0x00, 0xC3,
];
const X_CAM_PITCH: [usize; 8] = [0xc, 0x1c, 0x2c, 0x3c, 0x4c, 0x5c, 0x6c, 0x7c];
// gamesave_s04_districts_save_coop @ 0x6924f0, 122 bytes
const B_GS04C: [u8; 122] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x00, 0xBD, 0x7A, 0x02, 0x55, 0x33, 0xED, 0x89, 0x88, 0x04,
    0x73, 0x01, 0x00, 0x39, 0x2D, 0x00, 0xBD, 0x7A, 0x02, 0x7E, 0x5D, 0x56, 0x57, 0x8B, 0x3D, 0x94,
    0x6C, 0x52, 0x02, 0x05, 0x10, 0x73, 0x01, 0x00, 0x83, 0xC7, 0x59, 0xBE, 0x0C, 0xBD, 0x7A, 0x02,
    0x8B, 0x56, 0xF8, 0x89, 0x50, 0xF8, 0x8B, 0x4E, 0xFC, 0x89, 0x48, 0xFC, 0x0F, 0xB6, 0x16, 0x88,
    0x10, 0x0F, 0xB6, 0x4F, 0xFF, 0xC0, 0xE9, 0x06, 0x32, 0xCA, 0x80, 0xE1, 0x01, 0x30, 0x08, 0x0F,
    0xB6, 0x17, 0x8A, 0x08, 0x02, 0xD2, 0x32, 0xD1, 0x80, 0xE2, 0x02, 0x32, 0xD1, 0x88, 0x10, 0x83,
    0xC5, 0x01, 0x83, 0xC6, 0x0C, 0x83, 0xC0, 0x0C, 0x81, 0xC7, 0xA8, 0x00, 0x00, 0x00, 0x3B, 0x2D,
    0x00, 0xBD, 0x7A, 0x02, 0x7C, 0xBA, 0x5F, 0x5E, 0x5D, 0xC3,
];
const X_GS04C: [usize; 0] = [];
// gamesave_s09_activity_levels_save_coop @ 0x692b00, 52 bytes
const B_GS09C_SAVE: [u8; 52] = [
    0xA1, 0x88, 0x7C, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x17, 0xA1, 0x0C, 0x63, 0x3D, 0x03, 0x56, 0x8B,
    0x74, 0x24, 0x08, 0x89, 0x46, 0x74, 0xE8, 0x35, 0x40, 0x00, 0x00, 0x89, 0x46, 0x78, 0x5E, 0xC3,
    0x8B, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x89, 0x46, 0x74, 0xE8, 0x21, 0x40, 0x00, 0x00, 0x89,
    0x46, 0x78, 0x5E, 0xC3,
];
const X_GS09C_SAVE: [usize; 2] = [0x16, 0x2a];
// gamesave_s09_activity_levels_save @ 0x692b40, 52 bytes
const B_GS09_SAVE: [u8; 52] = [
    0xA1, 0x88, 0x7C, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x17, 0xA1, 0x0C, 0x63, 0x3D, 0x03, 0x56, 0x8B,
    0x74, 0x24, 0x08, 0x89, 0x46, 0x74, 0xE8, 0xF5, 0x3F, 0x00, 0x00, 0x89, 0x46, 0x78, 0x5E, 0xC3,
    0x8B, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x89, 0x46, 0x74, 0xE8, 0xE1, 0x3F, 0x00, 0x00, 0x89,
    0x46, 0x78, 0x5E, 0xC3,
];
const X_GS09_SAVE: [usize; 2] = [0x16, 0x2a];
// gamesave_s10_script_triggers_save @ 0x692b80, 34 bytes
const B_GS10_SAVE: [u8; 34] = [
    0x83, 0xEC, 0x08, 0x8B, 0x44, 0x24, 0x0C, 0x57, 0x8D, 0x7C, 0x24, 0x04, 0xC7, 0x44, 0x24, 0x04,
    0xD0, 0x08, 0xE0, 0x00, 0x89, 0x44, 0x24, 0x08, 0xE8, 0x63, 0xBE, 0x3D, 0x00, 0x5F, 0x83, 0xC4,
    0x08, 0xC3,
];
const X_GS10_SAVE: [usize; 1] = [0x18];
// gamesave_s11_cribs_save_coop @ 0x692c60, 146 bytes
const B_GS11C_SAVE: [u8; 146] = [
    0x66, 0x0F, 0xB6, 0x05, 0x38, 0xC2, 0x7A, 0x02, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x66, 0x89,
    0x85, 0xA4, 0x82, 0x03, 0x00, 0x66, 0x8B, 0x0D, 0xDC, 0xBF, 0x7A, 0x02, 0x33, 0xDB, 0x66, 0x89,
    0x8D, 0xA6, 0x82, 0x03, 0x00, 0x39, 0x1D, 0xDC, 0xBF, 0x7A, 0x02, 0x76, 0x2F, 0x56, 0x57, 0xBE,
    0xE0, 0xBF, 0x7A, 0x02, 0x8D, 0xBD, 0xA8, 0x82, 0x03, 0x00, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0xCE, 0x8B, 0xC7, 0xE8, 0x57, 0x00, 0x00, 0x00, 0x83, 0xC3, 0x01, 0x83, 0xC6, 0x28, 0x83,
    0xC7, 0x28, 0x3B, 0x1D, 0xDC, 0xBF, 0x7A, 0x02, 0x72, 0xE6, 0x5F, 0x5E, 0xA1, 0xCC, 0x73, 0x79,
    0x02, 0x3D, 0x00, 0x94, 0x35, 0x77, 0x7E, 0x0D, 0xC7, 0x85, 0x00, 0x85, 0x03, 0x00, 0x00, 0x94,
    0x35, 0x77, 0x5D, 0x5B, 0xC3, 0x3D, 0x00, 0x6C, 0xCA, 0x88, 0x7D, 0x0D, 0xC7, 0x85, 0x00, 0x85,
    0x03, 0x00, 0x00, 0x6C, 0xCA, 0x88, 0x5D, 0x5B, 0xC3, 0x89, 0x85, 0x00, 0x85, 0x03, 0x00, 0x5D,
    0x5B, 0xC3,
];
const X_GS11C_SAVE: [usize; 1] = [0x44];
// gamesave_s00_timers_save @ 0x692e80, 28 bytes
const B_GS00_SAVE: [u8; 28] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x8D, 0x86, 0x7C, 0x76, 0x03, 0x00, 0x50, 0xE8, 0xAF, 0xFA, 0xEA,
    0xFF, 0x83, 0xC4, 0x04, 0x89, 0x86, 0x78, 0x76, 0x03, 0x00, 0x5E, 0xC3,
];
const X_GS00_SAVE: [usize; 1] = [0xc];
// gamesave_s14_playlists_save @ 0x692ea0, 18 bytes
const B_GS14_SAVE: [u8; 18] = [
    0x57, 0x8B, 0x7C, 0x24, 0x08, 0x81, 0xC7, 0x38, 0x8C, 0x03, 0x00, 0xE8, 0x10, 0x87, 0x01, 0x00,
    0x5F, 0xC3,
];
const X_GS14_SAVE: [usize; 1] = [0xb];
// gamesave_s15_cutscene_seen_save @ 0x692ec0, 56 bytes
const B_GS15_SAVE: [u8; 56] = [
    0x83, 0xEC, 0x0C, 0x81, 0x3D, 0xAC, 0xA4, 0x45, 0x01, 0x00, 0x01, 0x00, 0x00, 0x7D, 0x25, 0x8B,
    0x44, 0x24, 0x10, 0x56, 0x05, 0x68, 0x9B, 0x03, 0x00, 0x8D, 0x74, 0x24, 0x04, 0xC7, 0x44, 0x24,
    0x08, 0x20, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x04, 0xC6, 0x44, 0x24, 0x0C, 0x00, 0xE8, 0xDD,
    0x0B, 0x03, 0x00, 0x5E, 0x83, 0xC4, 0x0C, 0xC3,
];
const X_GS15_SAVE: [usize; 1] = [0x2e];
// gamesave_s16_persistent_objects_save @ 0x692f00, 42 bytes
const B_GS16_SAVE: [u8; 42] = [
    0x83, 0xEC, 0x0C, 0x8B, 0x44, 0x24, 0x10, 0x8D, 0x0C, 0x24, 0x05, 0x88, 0x9B, 0x03, 0x00, 0x51,
    0xC7, 0x44, 0x24, 0x08, 0xED, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x04, 0xC6, 0x44, 0x24, 0x0C,
    0x00, 0xE8, 0xEA, 0x9E, 0x3F, 0x00, 0x83, 0xC4, 0x10, 0xC3,
];
const X_GS16_SAVE: [usize; 1] = [0x21];
// gamesave_s17_rewards_save @ 0x692f30, 65 bytes
const B_GS17_SAVE: [u8; 65] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x04, 0xB2, 0x40, 0x01, 0x89, 0x88, 0xB8, 0x9C, 0x03, 0x00,
    0xD9, 0x05, 0x10, 0xB2, 0x40, 0x01, 0xD9, 0x98, 0xC8, 0xA0, 0x03, 0x00, 0xD9, 0x05, 0x20, 0xB2,
    0x40, 0x01, 0xD9, 0x98, 0xCC, 0xA0, 0x03, 0x00, 0xD9, 0x05, 0x30, 0xB2, 0x40, 0x01, 0xD9, 0x98,
    0xD0, 0xA0, 0x03, 0x00, 0xD9, 0x05, 0x40, 0xB2, 0x40, 0x01, 0xD9, 0x98, 0xD4, 0xA0, 0x03, 0x00,
    0xC3,
];
const X_GS17_SAVE: [usize; 0] = [];
// gamesave_s18_activity_instances_save @ 0x692fb0, 20 bytes
const B_GS18_SAVE: [u8; 20] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x81, 0xC6, 0xB8, 0x9D, 0x03, 0x00, 0x74, 0x05, 0xE8, 0x7E, 0x00,
    0xED, 0xFF, 0x5E, 0xC3,
];
const X_GS18_SAVE: [usize; 1] = [0xd];
// gamesave_s20_cheats_save @ 0x693450, 24 bytes
const B_GS20_SAVE: [u8; 24] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x8D, 0x96, 0xC8, 0x9D, 0x03, 0x00, 0xE8, 0xC0, 0x4D, 0xFF, 0xFF,
    0x89, 0x86, 0xC4, 0x9D, 0x03, 0x00, 0x5E, 0xC3,
];
const X_GS20_SAVE: [usize; 1] = [0xb];
// gamesave_s12_tutorials_save @ 0x693470, 38 bytes
const B_GS12_SAVE: [u8; 38] = [
    0x8B, 0x4C, 0x24, 0x04, 0x81, 0xC1, 0xF8, 0x85, 0x03, 0x00, 0x33, 0xC0, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x90, 0xA4, 0xAE, 0x7D, 0x02, 0x89, 0x11, 0x83, 0xC0, 0x20, 0x83, 0xC1, 0x04, 0x3D, 0xC0,
    0x10, 0x00, 0x00, 0x72, 0xEB, 0xC3,
];
const X_GS12_SAVE: [usize; 0] = [];
// gamesave_s01_world_grid_save @ 0x693570, 24 bytes
const B_GS01_SAVE: [u8; 24] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x8D, 0x96, 0x08, 0x85, 0x03, 0x00, 0xE8, 0x90, 0x93, 0x3E, 0x00,
    0x89, 0x86, 0x04, 0x85, 0x03, 0x00, 0x5E, 0xC3,
];
const X_GS01_SAVE: [usize; 1] = [0xb];
// gamesave_s05_options_save @ 0x6935c0, 15 bytes
const B_GS05_SAVE: [u8; 15] = [
    0x8B, 0x4C, 0x24, 0x04, 0x81, 0xC1, 0x48, 0xA1, 0x03, 0x00, 0xE9, 0x01, 0xB7, 0x14, 0x00,
];
const X_GS05_SAVE: [usize; 1] = [0xa];
// gamesave_s23_audio_a_save @ 0x693720, 14 bytes
const B_GS23_SAVE: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0x6C, 0xA1, 0x03, 0x00, 0xE9, 0xA2, 0x80, 0xDF, 0xFF,
];
const X_GS23_SAVE: [usize; 1] = [0x9];
// gamesave_s24_audio_b_save @ 0x693730, 14 bytes
const B_GS24_SAVE: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0xCC, 0xA1, 0x03, 0x00, 0xE9, 0x02, 0xDA, 0xDF, 0xFF,
];
const X_GS24_SAVE: [usize; 1] = [0x9];
// gamesave_s25_audio_c_save @ 0x693740, 18 bytes
const B_GS25_SAVE: [u8; 18] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x81, 0xC6, 0xCC, 0xA9, 0x03, 0x00, 0xE8, 0xB0, 0xDA, 0xDF, 0xFF,
    0x5E, 0xC3,
];
const X_GS25_SAVE: [usize; 1] = [0xb];
// gamesave_s26_music_store_save @ 0x693760, 32 bytes
const B_GS26_SAVE: [u8; 32] = [
    0x51, 0x8B, 0x44, 0x24, 0x08, 0x05, 0x0C, 0xAA, 0x03, 0x00, 0x50, 0x8D, 0x4C, 0x24, 0x04, 0xC7,
    0x44, 0x24, 0x04, 0x5A, 0x00, 0x00, 0x00, 0xE8, 0x54, 0xB4, 0x11, 0x00, 0x83, 0xC4, 0x08, 0xC3,
];
const X_GS26_SAVE: [usize; 1] = [0x17];
// gamesave_s27_store_discounts_save @ 0x6937a0, 18 bytes
const B_GS27_SAVE: [u8; 18] = [
    0x57, 0x8B, 0x7C, 0x24, 0x08, 0x81, 0xC7, 0x68, 0xAA, 0x03, 0x00, 0xE8, 0x40, 0xD0, 0x11, 0x00,
    0x5F, 0xC3,
];
const X_GS27_SAVE: [usize; 1] = [0xb];
// gamesave_s07_garage_load @ 0x694470, 74 bytes
const B_GS07_LOAD: [u8; 74] = [
    0x53, 0x56, 0xE8, 0x79, 0x14, 0xEC, 0xFF, 0x8B, 0x5C, 0x24, 0x0C, 0x8B, 0x83, 0x70, 0x72, 0x03,
    0x00, 0x33, 0xF6, 0x39, 0xB3, 0xD8, 0x75, 0x01, 0x00, 0xA3, 0x98, 0x99, 0x40, 0x01, 0x7E, 0x27,
    0x57, 0x8D, 0xBB, 0xDC, 0x75, 0x01, 0x00, 0x83, 0x3F, 0x00, 0x75, 0x09, 0x57, 0xE8, 0x4E, 0x12,
    0xEC, 0xFF, 0x83, 0xC4, 0x04, 0x83, 0xC6, 0x01, 0x81, 0xC7, 0x34, 0x04, 0x00, 0x00, 0x3B, 0xB3,
    0xD8, 0x75, 0x01, 0x00, 0x7C, 0xE1, 0x5F, 0x5E, 0x5B, 0xC3,
];
const X_GS07_LOAD: [usize; 2] = [0x2, 0x2d];
// gamesave_s22_wiretap_load @ 0x6944c0, 21 bytes
const B_GS22_LOAD: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x88, 0xC8, 0x79, 0x03, 0x00, 0x88, 0x0D, 0x9B, 0x23, 0xFA, 0x00,
    0xE9, 0xCB, 0x87, 0xEB, 0xFF,
];
const X_GS22_LOAD: [usize; 1] = [0x10];
// gamesave_s09_activity_levels_load_coop @ 0x6947c0, 28 bytes
const B_GS09C_LOAD: [u8; 28] = [
    0x8B, 0x4C, 0x24, 0x04, 0x83, 0xC1, 0x74, 0xE8, 0xF4, 0x99, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x06,
    0x83, 0x78, 0x3C, 0x00, 0x75, 0x05, 0xA3, 0x88, 0x7C, 0x52, 0x02, 0xC3,
];
const X_GS09C_LOAD: [usize; 1] = [0x7];
// gamesave_s09_activity_levels_load @ 0x6947e0, 28 bytes
const B_GS09_LOAD: [u8; 28] = [
    0x8B, 0x4C, 0x24, 0x04, 0x83, 0xC1, 0x74, 0xE8, 0xD4, 0x99, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x06,
    0x83, 0x78, 0x3C, 0x00, 0x75, 0x05, 0xA3, 0x88, 0x7C, 0x52, 0x02, 0xC3,
];
const X_GS09_LOAD: [usize; 1] = [0x7];
// gamesave_s10_script_triggers_load @ 0x694800, 163 bytes
const B_GS10_LOAD: [u8; 163] = [
    0x55, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x33, 0xED, 0x39, 0xAF, 0xD0, 0x7A, 0x03, 0x00, 0x0F, 0x8E,
    0x8C, 0x00, 0x00, 0x00, 0xA1, 0x8C, 0xE8, 0xF8, 0x02, 0x53, 0x56, 0x8D, 0x9F, 0xD8, 0x7A, 0x03,
    0x00, 0x85, 0xC0, 0x8B, 0xF0, 0x74, 0x69, 0x8B, 0x4B, 0xFC, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x8E, 0xC0, 0x00, 0x00, 0x00, 0x74, 0x10, 0x8B, 0xB6, 0x88, 0x01, 0x00, 0x00, 0x3B, 0xF0,
    0x74, 0x4E, 0x85, 0xF6, 0x75, 0xEA, 0xEB, 0x48, 0x85, 0xF6, 0x74, 0x44, 0xF6, 0x03, 0x02, 0x74,
    0x27, 0x8A, 0x86, 0x10, 0x01, 0x00, 0x00, 0xA8, 0x10, 0x75, 0x1D, 0x8B, 0xBE, 0x7C, 0x01, 0x00,
    0x00, 0x0C, 0x10, 0x88, 0x86, 0x10, 0x01, 0x00, 0x00, 0x8B, 0x86, 0x80, 0x01, 0x00, 0x00, 0xE8,
    0x4C, 0x05, 0x11, 0x00, 0x8B, 0x7C, 0x24, 0x14, 0xF6, 0x03, 0x01, 0x74, 0x09, 0x8B, 0xC6, 0xE8,
    0xDC, 0x74, 0x3D, 0x00, 0xEB, 0x05, 0xE8, 0x55, 0x74, 0x3D, 0x00, 0xA1, 0x8C, 0xE8, 0xF8, 0x02,
    0x83, 0xC5, 0x01, 0x83, 0xC3, 0x08, 0x3B, 0xAF, 0xD0, 0x7A, 0x03, 0x00, 0x7C, 0x83, 0x5E, 0x5B,
    0x5F, 0x5D, 0xC3,
];
const X_GS10_LOAD: [usize; 3] = [0x6f, 0x7f, 0x86];
// gamesave_s11_cribs_load_coop @ 0x6948b0, 99 bytes
const B_GS11C_LOAD: [u8; 99] = [
    0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x66, 0x83, 0xBD, 0xA4, 0x82, 0x03, 0x00, 0x00, 0x0F, 0x95,
    0xC0, 0xA2, 0x38, 0xC2, 0x7A, 0x02, 0x0F, 0xB7, 0x85, 0xA6, 0x82, 0x03, 0x00, 0x33, 0xDB, 0x85,
    0xC0, 0xA3, 0xDC, 0xBF, 0x7A, 0x02, 0x76, 0x29, 0x56, 0x57, 0xBF, 0xE0, 0xBF, 0x7A, 0x02, 0x8D,
    0xB5, 0xA8, 0x82, 0x03, 0x00, 0x8B, 0xCE, 0x8B, 0xC7, 0xE8, 0x12, 0xE4, 0xFF, 0xFF, 0x83, 0xC3,
    0x01, 0x83, 0xC6, 0x28, 0x83, 0xC7, 0x28, 0x3B, 0x1D, 0xDC, 0xBF, 0x7A, 0x02, 0x72, 0xE6, 0x5F,
    0x5E, 0x8B, 0x8D, 0x00, 0x85, 0x03, 0x00, 0x51, 0xE8, 0xF3, 0x45, 0xEB, 0xFF, 0x83, 0xC4, 0x04,
    0x5D, 0x5B, 0xC3,
];
const X_GS11C_LOAD: [usize; 2] = [0x39, 0x58];
// gamesave_s00_timers_load @ 0x694ab0, 21 bytes
const B_GS00_LOAD: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x8D, 0x88, 0x7C, 0x76, 0x03, 0x00, 0x8B, 0x80, 0x78, 0x76, 0x03, 0x00,
    0xE9, 0x4B, 0xDF, 0xEA, 0xFF,
];
const X_GS00_LOAD: [usize; 1] = [0x10];
// gamesave_s16_persistent_objects_load @ 0x694ae0, 42 bytes
const B_GS16_LOAD: [u8; 42] = [
    0x83, 0xEC, 0x0C, 0x8B, 0x44, 0x24, 0x10, 0x8D, 0x0C, 0x24, 0x05, 0x88, 0x9B, 0x03, 0x00, 0x51,
    0xC7, 0x44, 0x24, 0x08, 0xED, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x04, 0xC6, 0x44, 0x24, 0x0C,
    0x00, 0xE8, 0xAA, 0x83, 0x3F, 0x00, 0x83, 0xC4, 0x10, 0xC3,
];
const X_GS16_LOAD: [usize; 1] = [0x21];
// gamesave_s15_cutscene_seen_load @ 0x694b10, 63 bytes
const B_GS15_LOAD: [u8; 63] = [
    0x83, 0xEC, 0x0C, 0x80, 0x7C, 0x24, 0x14, 0x00, 0x74, 0x31, 0x81, 0x3D, 0xAC, 0xA4, 0x45, 0x01,
    0x00, 0x01, 0x00, 0x00, 0x7D, 0x25, 0x8B, 0x44, 0x24, 0x10, 0x56, 0x05, 0x68, 0x9B, 0x03, 0x00,
    0x8D, 0x74, 0x24, 0x04, 0xC7, 0x44, 0x24, 0x08, 0x20, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x04,
    0xC6, 0x44, 0x24, 0x0C, 0x00, 0xE8, 0xE6, 0xEF, 0x02, 0x00, 0x5E, 0x83, 0xC4, 0x0C, 0xC3,
];
const X_GS15_LOAD: [usize; 1] = [0x35];
// gamesave_s17_rewards_load @ 0x694b50, 70 bytes
const B_GS17_LOAD: [u8; 70] = [
    0x80, 0x7C, 0x24, 0x08, 0x00, 0x74, 0x3E, 0x8B, 0x44, 0x24, 0x04, 0x8B, 0x88, 0xB8, 0x9C, 0x03,
    0x00, 0x56, 0x57, 0x89, 0x0D, 0x04, 0xB2, 0x40, 0x01, 0x33, 0xF6, 0x8D, 0xB8, 0xC8, 0xA0, 0x03,
    0x00, 0x83, 0xFE, 0xFF, 0x7E, 0x12, 0xD9, 0x07, 0x6A, 0x01, 0x51, 0x8B, 0xC6, 0xD9, 0x1C, 0x24,
    0xE8, 0x1B, 0x11, 0xF7, 0xFF, 0x83, 0xC4, 0x08, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x04, 0x83, 0xFE,
    0x04, 0x72, 0xDE, 0x5F, 0x5E, 0xC3,
];
const X_GS17_LOAD: [usize; 1] = [0x30];
// gamesave_s18_activity_instances_load @ 0x694bd0, 18 bytes
const B_GS18_LOAD: [u8; 18] = [
    0x8B, 0x4C, 0x24, 0x04, 0x81, 0xC1, 0xB8, 0x9D, 0x03, 0x00, 0x74, 0x05, 0xE9, 0xEF, 0xE4, 0xEC,
    0xFF, 0xC3,
];
const X_GS18_LOAD: [usize; 1] = [0xc];
// gamesave_s20_cheats_load @ 0x694e20, 23 bytes
const B_GS20_LOAD: [u8; 23] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x88, 0xC4, 0x9D, 0x03, 0x00, 0x51, 0x05, 0xC8, 0x9D, 0x03, 0x00,
    0xE8, 0x2B, 0x34, 0xFF, 0xFF, 0x59, 0xC3,
];
const X_GS20_LOAD: [usize; 1] = [0x10];
// gamesave_s12_tutorials_load @ 0x694e40, 38 bytes
const B_GS12_LOAD: [u8; 38] = [
    0x8B, 0x4C, 0x24, 0x04, 0x81, 0xC1, 0xF8, 0x85, 0x03, 0x00, 0x33, 0xC0, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x11, 0x89, 0x90, 0xA4, 0xAE, 0x7D, 0x02, 0x83, 0xC0, 0x20, 0x83, 0xC1, 0x04, 0x3D, 0xC0,
    0x10, 0x00, 0x00, 0x72, 0xEB, 0xC3,
];
const X_GS12_LOAD: [usize; 0] = [];
// gamesave_s13_debug_display_options_load @ 0x694e70, 101 bytes
const B_GS13_LOAD: [u8; 101] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x88, 0x10, 0x88, 0x03, 0x00, 0x89, 0x0D, 0x94, 0xBA, 0x7A, 0x02,
    0x8B, 0x90, 0x14, 0x88, 0x03, 0x00, 0x89, 0x15, 0xAC, 0xBA, 0x7A, 0x02, 0x8B, 0x88, 0x18, 0x88,
    0x03, 0x00, 0x89, 0x0D, 0xC4, 0xBA, 0x7A, 0x02, 0x8B, 0x90, 0x1C, 0x88, 0x03, 0x00, 0x89, 0x15,
    0xDC, 0xBA, 0x7A, 0x02, 0x8B, 0x88, 0x20, 0x88, 0x03, 0x00, 0x89, 0x0D, 0xF4, 0xBA, 0x7A, 0x02,
    0x8B, 0x90, 0x24, 0x88, 0x03, 0x00, 0x89, 0x15, 0x0C, 0xBB, 0x7A, 0x02, 0x8B, 0x88, 0x28, 0x88,
    0x03, 0x00, 0x89, 0x0D, 0x24, 0xBB, 0x7A, 0x02, 0x8B, 0x90, 0x2C, 0x88, 0x03, 0x00, 0x89, 0x15,
    0x3C, 0xBB, 0x7A, 0x02, 0xC3,
];
const X_GS13_LOAD: [usize; 0] = [];
// gamesave_s01_world_grid_load_coop @ 0x694ee0, 66 bytes
const B_GS01C_LOAD: [u8; 66] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x8A, 0x04, 0x85, 0x03, 0x00, 0x33, 0xC0, 0x85, 0xC9, 0x89, 0x0D,
    0x3C, 0xC2, 0x7A, 0x02, 0x7E, 0x2B, 0x8D, 0x8A, 0x08, 0x85, 0x03, 0x00, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x11, 0x89, 0x14, 0xC5, 0x40, 0xC2, 0x7A, 0x02, 0x8B, 0x51, 0x04, 0x89, 0x14, 0xC5, 0x44,
    0xC2, 0x7A, 0x02, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x08, 0x3B, 0x05, 0x3C, 0xC2, 0x7A, 0x02, 0x7C,
    0xDF, 0xC3,
];
const X_GS01C_LOAD: [usize; 0] = [];
// gamesave_s01_world_grid_load @ 0x694f30, 21 bytes
const B_GS01_LOAD: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x90, 0x04, 0x85, 0x03, 0x00, 0x8D, 0x88, 0x08, 0x85, 0x03, 0x00,
    0xE9, 0x8B, 0x7A, 0x3E, 0x00,
];
const X_GS01_LOAD: [usize; 1] = [0x10];
// gamesave_s05_options_load @ 0x694f80, 14 bytes
const B_GS05_LOAD: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0x48, 0xA1, 0x03, 0x00, 0xE9, 0xF2, 0x9D, 0x14, 0x00,
];
const X_GS05_LOAD: [usize; 1] = [0x9];
// gamesave_s23_audio_a_load @ 0x6950c0, 14 bytes
const B_GS23_LOAD: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0x6C, 0xA1, 0x03, 0x00, 0xE9, 0xA2, 0x67, 0xDF, 0xFF,
];
const X_GS23_LOAD: [usize; 1] = [0x9];
// gamesave_s24_audio_b_load @ 0x6950d0, 14 bytes
const B_GS24_LOAD: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0xCC, 0xA1, 0x03, 0x00, 0xE9, 0xA2, 0xC0, 0xDF, 0xFF,
];
const X_GS24_LOAD: [usize; 1] = [0x9];
// gamesave_s25_audio_c_load @ 0x6950e0, 14 bytes
const B_GS25_LOAD: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0xCC, 0xA9, 0x03, 0x00, 0xE9, 0x72, 0xC1, 0xDF, 0xFF,
];
const X_GS25_LOAD: [usize; 1] = [0x9];
// gamesave_s26_music_store_load @ 0x6950f0, 17 bytes
const B_GS26_LOAD: [u8; 17] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0x0C, 0xAA, 0x03, 0x00, 0x50, 0xE8, 0x61, 0x9C, 0x11, 0x00, 0x59,
    0xC3,
];
const X_GS26_LOAD: [usize; 1] = [0xa];
// gamesave_s27_store_discounts_load @ 0x695130, 17 bytes
const B_GS27_LOAD: [u8; 17] = [
    0x8B, 0x44, 0x24, 0x04, 0x05, 0x68, 0xAA, 0x03, 0x00, 0x50, 0xE8, 0x81, 0xB7, 0x11, 0x00, 0x59,
    0xC3,
];
const X_GS27_LOAD: [usize; 1] = [0xa];
// net_recv_qos @ 0x80b5b0, 20 bytes
const B_RECV_QOS: [u8; 20] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x48, 0x04, 0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x89, 0x81, 0xC0,
    0x1D, 0x00, 0x00, 0xC3,
];
const X_RECV_QOS: [usize; 0] = [];
// net_recv_bw_info @ 0x80b5e0, 45 bytes
const B_RECV_BW_INFO: [u8; 45] = [
    0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x41, 0x04, 0x85, 0xC0, 0x74, 0x21, 0x8B, 0x54, 0x24, 0x04, 0x8B,
    0x12, 0x89, 0x90, 0xBC, 0x1D, 0x00, 0x00, 0x8B, 0x41, 0x04, 0x83, 0xB8, 0xA8, 0x01, 0x00, 0x00,
    0x00, 0x74, 0x09, 0x57, 0x8B, 0xF8, 0xE8, 0x45, 0xB5, 0x00, 0x00, 0x5F, 0xC3,
];
const X_RECV_BW_INFO: [usize; 1] = [0x26];
// net_recv_next_srv @ 0x810a20, 32 bytes
const B_RECV_NEXT_SRV: [u8; 32] = [
    0x8B, 0x44, 0x24, 0x08, 0x83, 0x78, 0x04, 0x00, 0x74, 0x15, 0x8B, 0x40, 0x04, 0x8B, 0x80, 0xAC,
    0x01, 0x00, 0x00, 0x8B, 0x88, 0x08, 0x04, 0x00, 0x00, 0x89, 0x88, 0x10, 0x0A, 0x00, 0x00, 0xC3,
];
const X_RECV_NEXT_SRV: [usize; 0] = [];
// net_recv_modified_session @ 0x810bd0, 73 bytes
const B_RECV_MOD_SESS: [u8; 73] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x48, 0x04, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0x83, 0x78, 0x2C,
    0x02, 0x75, 0x35, 0x53, 0x8B, 0x99, 0x70, 0x01, 0x00, 0x00, 0x56, 0x8D, 0xB1, 0x4C, 0x01, 0x00,
    0x00, 0x8B, 0x88, 0xF8, 0x0B, 0x00, 0x00, 0x8D, 0x0C, 0x89, 0x57, 0x8D, 0x94, 0xC8, 0x18, 0x0A,
    0x00, 0x00, 0xB9, 0x09, 0x00, 0x00, 0x00, 0x8B, 0xFA, 0xF3, 0xA5, 0x5F, 0x89, 0x5A, 0x24, 0x83,
    0x80, 0xF8, 0x0B, 0x00, 0x00, 0x01, 0x5E, 0x5B, 0xC3,
];
const X_RECV_MOD_SESS: [usize; 0] = [];
// net_recv_notor_active @ 0x924d30, 51 bytes
const B_RECV_NOTOR: [u8; 51] = [
    0x8B, 0x44, 0x24, 0x04, 0x0F, 0xB7, 0x00, 0x33, 0xC9, 0x0F, 0xB7, 0xC0, 0x56, 0x8D, 0x49, 0x00,
    0x8B, 0x34, 0x8D, 0x68, 0x8B, 0xE8, 0x00, 0xBA, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE2, 0x85, 0xD0,
    0x0F, 0x95, 0xC2, 0x83, 0xC1, 0x01, 0x83, 0xF9, 0x0E, 0x88, 0x96, 0xA4, 0x02, 0x00, 0x00, 0x7C,
    0xDF, 0x5E, 0xC3,
];
const X_RECV_NOTOR: [usize; 0] = [];
// misc_packet_sub_05 @ 0x548580, 15 bytes
const B_MISC_PKT_05: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x6A, 0x01, 0xE8, 0x03, 0x00, 0x00, 0x00, 0x59, 0xC3,
];
const X_MISC_PKT_05: [usize; 1] = [0x8];
// misc_packet_sub_17 @ 0xa2c670, 8 bytes
const B_MISC_PKT_17: [u8; 8] = [
    0xC6, 0x05, 0x38, 0xC2, 0x7A, 0x02, 0x01, 0xC3,
];
const X_MISC_PKT_17: [usize; 0] = [];
// hud_pkt_sub_16 @ 0xa2ddb0, 12 bytes
const B_HUD_PKT_16: [u8; 12] = [
    0xC6, 0x05, 0x9F, 0x84, 0x52, 0x02, 0x00, 0xE9, 0x74, 0x64, 0xD6, 0xFF,
];
const X_HUD_PKT_16: [usize; 1] = [0x7];
// net_recv_acks @ 0x7fb150, 56 bytes
const B_RECV_ACKS: [u8; 56] = [
    0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x83, 0x7D, 0x04, 0x00, 0x74, 0x2B, 0x8B, 0x4C, 0x24, 0x08, 0x8A,
    0x01, 0x84, 0xC0, 0x76, 0x21, 0x53, 0x56, 0x57, 0x8D, 0x79, 0x01, 0x0F, 0xB6, 0xD8, 0x8B, 0xFF,
    0x0F, 0xB7, 0x37, 0x8B, 0x45, 0x04, 0x83, 0xC7, 0x02, 0xE8, 0xF2, 0xD0, 0xFF, 0xFF, 0x83, 0xEB,
    0x01, 0x75, 0xED, 0x5F, 0x5E, 0x5B, 0x5D, 0xC3,
];
const X_RECV_ACKS: [usize; 1] = [0x29];
// net_recv_sessionstart @ 0x835930, 18 bytes
const B_RECV_SESSSTART: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x48, 0x04, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0xE9, 0x5E, 0x70,
    0xFD, 0xFF,
];
const X_RECV_SESSSTART: [usize; 1] = [0xd];
// act_packet_sub_3f @ 0x5af3f0, 23 bytes
const B_ACT_3F: [u8; 23] = [
    0xA1, 0x88, 0x8E, 0x40, 0x01, 0x85, 0xC0, 0x74, 0x0D, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x06,
    0x8B, 0x48, 0x40, 0x89, 0x48, 0x44, 0xC3,
];
const X_ACT_3F: [usize; 0] = [];
// act_packet_sub_3e @ 0x5af390, 92 bytes
const B_ACT_3E: [u8; 92] = [
    0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74, 0x52, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x4C, 0x57,
    0x8B, 0xB8, 0x90, 0x00, 0x00, 0x00, 0x85, 0xFF, 0x74, 0x40, 0x8B, 0x4C, 0x24, 0x08, 0x53, 0x56,
    0x8B, 0x31, 0x8D, 0x51, 0x04, 0x33, 0xC0, 0x8D, 0x8F, 0x9C, 0x01, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x3B, 0xC6, 0x66, 0xC7, 0x01, 0xFF, 0xFF, 0x7D, 0x09, 0x66, 0x8B, 0x1A, 0x66, 0x89, 0x19, 0x83,
    0xC2, 0x02, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x02, 0x83, 0xF8, 0x0C, 0x7C, 0xE3, 0x85, 0xF6, 0x0F,
    0x9F, 0xC0, 0x5E, 0x88, 0x87, 0x9A, 0x01, 0x00, 0x00, 0x5B, 0x5F, 0xC3,
];
const X_ACT_3E: [usize; 0] = [];
// act_packet_sub_44 @ 0x5bbba0, 84 bytes
const B_ACT_44: [u8; 84] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC9, 0x8B, 0x10, 0x56, 0x8B,
    0x70, 0x04, 0x57, 0x8B, 0x78, 0x08, 0x8B, 0x40, 0x0C, 0x74, 0x36, 0x83, 0x79, 0x3C, 0x01, 0x75,
    0x30, 0x8B, 0x89, 0x90, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x26, 0x39, 0x91, 0xC0, 0x02, 0x00,
    0x00, 0x89, 0x91, 0xBC, 0x02, 0x00, 0x00, 0x7D, 0x06, 0x89, 0x91, 0xC0, 0x02, 0x00, 0x00, 0x89,
    0xB1, 0xC4, 0x02, 0x00, 0x00, 0x89, 0xB9, 0xC8, 0x02, 0x00, 0x00, 0x89, 0x81, 0xCC, 0x02, 0x00,
    0x00, 0x5F, 0x5E, 0xC3,
];
const X_ACT_44: [usize; 0] = [];
// act_packet_sub_4a @ 0x5f7e30, 59 bytes
const B_ACT_4A: [u8; 59] = [
    0x83, 0xEC, 0x08, 0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x08, 0x8B, 0x50, 0x04, 0x8B, 0x40, 0x08, 0x89,
    0x4C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x1D, 0x7C, 0xA6, 0x52, 0x02, 0x89, 0x14, 0x24,
    0xD9, 0x04, 0x24, 0x89, 0x44, 0x24, 0x04, 0xD9, 0x1D, 0x80, 0xA6, 0x52, 0x02, 0xD9, 0x44, 0x24,
    0x04, 0xD9, 0x1D, 0x84, 0xA6, 0x52, 0x02, 0x83, 0xC4, 0x08, 0xC3,
];
const X_ACT_4A: [usize; 0] = [];
// act_packet_sub_1a @ 0x5e7f10, 90 bytes
const B_ACT_1A: [u8; 90] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8A, 0x01, 0x3C, 0x05, 0x75, 0x08, 0xC6, 0x05, 0x3E, 0x77, 0x52, 0x02,
    0x01, 0xC3, 0x3C, 0x09, 0x75, 0x06, 0xA2, 0x1D, 0x77, 0x52, 0x02, 0xC3, 0x3C, 0x01, 0x75, 0x26,
    0xA2, 0x1D, 0x77, 0x52, 0x02, 0x0F, 0xBE, 0x41, 0x01, 0x83, 0xF8, 0x02, 0xA3, 0x04, 0x7D, 0xE9,
    0x00, 0x75, 0x09, 0x8B, 0x41, 0x02, 0xA3, 0xE4, 0xA5, 0x40, 0x01, 0xC3, 0x8B, 0x49, 0x02, 0x89,
    0x0D, 0x20, 0x78, 0x52, 0x02, 0xC3, 0x3C, 0x07, 0x75, 0x06, 0xA2, 0x1D, 0x77, 0x52, 0x02, 0xC3,
    0x3C, 0x08, 0x75, 0x05, 0xA2, 0x1D, 0x77, 0x52, 0x02, 0xC3,
];
const X_ACT_1A: [usize; 0] = [];
// act_packet_sub_48 @ 0x609790, 23 bytes
const B_ACT_48: [u8; 23] = [
    0x8B, 0x44, 0x24, 0x04, 0x0F, 0xBE, 0x00, 0x83, 0xF8, 0xFF, 0x7C, 0x0A, 0x83, 0xF8, 0x02, 0x7F,
    0x05, 0xA3, 0x68, 0x7B, 0xE9, 0x00, 0xC3,
];
const X_ACT_48: [usize; 0] = [];
// act_packet_sub_46 @ 0x5bbd20, 21 bytes
const B_ACT_46: [u8; 21] = [
    0xA1, 0x88, 0x8E, 0x40, 0x01, 0x85, 0xC0, 0x74, 0x0B, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x04,
    0xC6, 0x40, 0x1D, 0x01, 0xC3,
];
const X_ACT_46: [usize; 0] = [];
// net_recv_no_connect @ 0x7faf30, 94 bytes
const B_RECV_NO_CONNECT: [u8; 94] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x54, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00,
    0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x42, 0x80, 0xB8, 0xF8, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x39, 0x8B, 0x44, 0x24, 0x08, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0x2E, 0x8B,
    0x4C, 0x24, 0x04, 0x8B, 0x80, 0xAC, 0x01, 0x00, 0x00, 0x8A, 0x11, 0x8B, 0x88, 0x00, 0x04, 0x00,
    0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x16, 0x38, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x74, 0x0E, 0x8B,
    0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xEA, 0xC3,
];
const X_RECV_NO_CONNECT: [usize; 0] = [];
// act_packet_sub_1d @ 0x677bb0, 17 bytes
const B_ACT_1D: [u8; 17] = [
    0xA1, 0xD4, 0x03, 0x17, 0x02, 0x50, 0x8B, 0x44, 0x24, 0x08, 0xE8, 0xC1, 0x7F, 0xFF, 0xFF, 0x59,
    0xC3,
];
const X_ACT_1D: [usize; 1] = [0xa];
// act_packet_sub_1e @ 0x677bd0, 24 bytes
const B_ACT_1E: [u8; 24] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x38, 0x00, 0x0F, 0x95, 0xC1, 0x51, 0x8B, 0x0D, 0xD4, 0x03, 0x17,
    0x02, 0xE8, 0x4A, 0x80, 0xFF, 0xFF, 0x59, 0xC3,
];
const X_ACT_1E: [usize; 1] = [0x11];
// act_packet_sub_15 @ 0x566b70, 26 bytes
const B_ACT_15: [u8; 26] = [
    0xA1, 0xF8, 0xE3, 0xE9, 0x00, 0x83, 0xF8, 0x01, 0x74, 0x05, 0x83, 0xF8, 0x02, 0x75, 0x0A, 0x68,
    0xF4, 0xE3, 0xE9, 0x00, 0xE8, 0x27, 0xD9, 0xFF, 0xFF, 0xC3,
];
const X_ACT_15: [usize; 1] = [0x14];
// act_packet_sub_18 @ 0x566c00, 33 bytes
const B_ACT_18: [u8; 33] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x08, 0xA1, 0xF8, 0xE3, 0xE9, 0x00, 0x83, 0xF8, 0x01, 0x74, 0x05,
    0x83, 0xF8, 0x02, 0x75, 0x0B, 0x51, 0x68, 0xF4, 0xE3, 0xE9, 0x00, 0xE8, 0x90, 0xC5, 0xFF, 0xFF,
    0xC3,
];
const X_ACT_18: [usize; 1] = [0x1b];
// act_packet_sub_17 @ 0x566bd0, 45 bytes
const B_ACT_17: [u8; 45] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x11, 0x0F, 0xBE, 0x41, 0x04, 0x0F, 0xB7, 0x49, 0x05, 0x56, 0x8B,
    0x35, 0xF8, 0xE3, 0xE9, 0x00, 0x83, 0xFE, 0x01, 0x74, 0x05, 0x83, 0xFE, 0x02, 0x75, 0x0C, 0x51,
    0x52, 0x68, 0xF4, 0xE3, 0xE9, 0x00, 0xE8, 0xD5, 0xC6, 0xFF, 0xFF, 0x5E, 0xC3,
];
const X_ACT_17: [usize; 1] = [0x26];
// act_packet_sub_19 @ 0x566c30, 28 bytes
const B_ACT_19: [u8; 28] = [
    0xA1, 0xF8, 0xE3, 0xE9, 0x00, 0x83, 0xF8, 0x01, 0x74, 0x05, 0x83, 0xF8, 0x02, 0x75, 0x0C, 0x56,
    0xBE, 0xF4, 0xE3, 0xE9, 0x00, 0xE8, 0x56, 0xD6, 0xFF, 0xFF, 0x5E, 0xC3,
];
const X_ACT_19: [usize; 1] = [0x15];
// act_packet_sub_41 @ 0x5c2600, 14 bytes
const B_ACT_41: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x30, 0xE8, 0x54, 0xFF, 0xFF, 0xFF, 0x5E, 0xC3,
];
const X_ACT_41: [usize; 1] = [0x7];

// --- game addresses ------------------------------------------------------------------------------------

/// `stricmp`-style compare: cdecl (a, b) -> 0 when equal.
const STRCMP: usize = 0x00C9_9030;
/// XML field parsers: ECX = field name string, ESI = destination, the node on the stack.
const PARSE_F: usize = 0x00B7_4EF0; // float
const PARSE_I: usize = 0x00B7_47D0; // int
const PARSE_X: usize = 0x00B7_4910; // the third parser (controlpatterns / kneecapper)

/// The double constant at `a` (the original reads it with `fmul qword`).
unsafe fn dbl(a: usize) -> f64 {
    unsafe { f64::from_bits(rd(a) as u64 | (rd(a + 4) as u64) << 32) }
}

//// The child lookup shared by the readers: the child of `node` (list at +8, next at +4) whose name
/// (first dword) equals `find`, or 0. `tail` is what the original has pushed below its return address
/// (saved registers, top first); `s` ends as the original's registers do after the loop.
unsafe fn find_child(s: &mut Pushad, a: *const u32, find: u32, tail: &[u32]) -> u32 {
    unsafe {
        if s.eax == 0 {
            return 0;
        }
        let mut e = rd(s.eax as usize + 8);
        s.esi = e;
        while e != 0 {
            let name = rd(e as usize);
            s.eax = name;
            let mut own = [0u32; 8];
            own[0] = name;
            own[1] = find;
            own[2..2 + tail.len()].copy_from_slice(tail);
            cx(STRCMP, s, &fr(a, &own[..2 + tail.len()]));
            if s.eax == 0 {
                return e;
            }
            e = rd(e as usize + 4);
            s.esi = e;
        }
        0
    }
}

/// One parser call per field: (parser, ECX name string, ESI destination); the node is pushed once more
/// for every call (the original pops them all at the end). `mid(k, s)` runs after call `k`.
unsafe fn seq(s: &mut Pushad, a: *const u32, node: u32, tail: &[u32], ops: &[(usize, u32, u32)], mid: &mut dyn FnMut(usize, &mut Pushad)) {
    for (k, &(f, ecx, esi)) in ops.iter().enumerate() {
        s.ecx = ecx;
        s.esi = esi;
        let mut own = [0u32; FW];
        let mut n = 0;
        for _ in 0..=k {
            own[n] = node;
            n += 1;
        }
        own[n..n + tail.len()].copy_from_slice(tail);
        n += tail.len();
        unsafe {
            cx(f, s, &fr(a, &own[..n]));
        }
        mid(k, s);
    }
}

/// A `gameplay_constants_parse_*` reader: find the child named `find`, then the fixed field list.
/// `two` is true when the original keeps the found node in EDI (two saved registers).
macro_rules! gc {
    ($h:ident, $rg:ident, $find:expr, $two:expr, [$(($f:expr, $s:expr, $d:expr)),* $(,)?]) => {
        unsafe fn $h(p: &mut Pushad, a: *const u32) {
            unsafe {
                let mut s = *p;
                let tail: &[u32] = if $two { &[p.edi, p.esi] } else { &[p.esi] };
                let node = find_child(&mut s, a, $find, tail);
                if $two {
                    s.edi = node;
                } else {
                    s.esi = node;
                }
                seq(&mut s, a, node, tail, &[$(($f, $s, $d)),*], &mut |_, _| {});
                s.esi = p.esi;
                s.edi = p.edi;
                *p = s;
            }
        }
        fn $rg(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
            $( v.push(($d as usize, 4)); )*
        }
    };
}

gc!(h_gc_death, rg_gc_death, 0xDE45F0, true, [
    (PARSE_F, 0xDE4604, 0x279743C), (PARSE_F, 0xDE4610, 0x2797440), (PARSE_I, 0xDE461C, 0x2797444),
    (PARSE_I, 0xDE462C, 0x2797448), (PARSE_I, 0xDE4640, 0x279744C), (PARSE_I, 0xDE4654, 0x2797450),
]);
gc!(h_gc_drunk, rg_gc_drunk, 0xDE46A0, true, [
    (PARSE_F, 0xDE3610, 0x2797430), (PARSE_F, 0xDE46AC, 0x2797438), (PARSE_F, 0xDE46C0, 0x2797434),
]);
gc!(h_gc_hit_car, rg_gc_hit_car, 0xDE46D0, true, [
    (PARSE_F, 0xDE46DC, 0x279745C), (PARSE_F, 0xDE46FC, 0x2797460), (PARSE_F, 0xDE471C, 0x2797464),
    (PARSE_F, 0xDE473C, 0x2797468), (PARSE_F, 0xDE4754, 0x279746C), (PARSE_F, 0xDE476C, 0x2797470),
    (PARSE_F, 0xDE4780, 0x2797474), (PARSE_F, 0xDE4798, 0x2797478),
]);
gc!(h_gc_veh_coll, rg_gc_veh_coll, 0xDE47A8, true, [
    (PARSE_F, 0xDE47BC, 0x279747C), (PARSE_F, 0xDE47E0, 0x2797480), (PARSE_F, 0xDE4800, 0x2797484),
    (PARSE_F, 0xDE4824, 0x2797488),
]);
gc!(h_gc_fire, rg_gc_fire, 0xDE4840, true, [
    (PARSE_F, 0xDE4848, 0x279748C), (PARSE_I, 0xDE4860, 0x2797490), (PARSE_I, 0xDE4878, 0x2797494),
    (PARSE_I, 0xDE4890, 0x2797498), (PARSE_F, 0xDE48A8, 0x279749C), (PARSE_I, 0xDE48C4, 0x27974A0),
    (PARSE_I, 0xDE48E0, 0x27974A4), (PARSE_I, 0xDE48F8, 0x27974A8),
]);
gc!(h_gc_pimp_ai, rg_gc_pimp_ai, 0xDE4B60, true, [
    (PARSE_F, 0xDE4B6C, 0x27976C0), (PARSE_F, 0xDE4B78, 0x27976C4), (PARSE_F, 0xDE4B90, 0x27976C8),
]);
gc!(h_gc_throwing, rg_gc_throwing, 0xDE4C6C, true, [
    (PARSE_F, 0xDE4C80, 0x27976FC), (PARSE_F, 0xDE4C98, 0x2797700), (PARSE_F, 0xDE4CAC, 0x2797704),
    (PARSE_F, 0xDE4CBC, 0x2797708),
]);
gc!(h_gc_shader_lod, rg_gc_shader_lod, 0xDE4EF0, true, [
    (PARSE_I, 0xDDE000, 0x2797784), (PARSE_I, 0xDE4EFC, 0x2797788),
]);
gc!(h_gc_shadows, rg_gc_shadows, 0xDE4F08, true, [
    (PARSE_I, 0xDE4F10, 0x279778C), (PARSE_I, 0xDE4F20, 0x2797790), (PARSE_I, 0xDE4F34, 0x2797794),
    (PARSE_I, 0xDE4F50, 0x2797798), (PARSE_I, 0xDE4F6C, 0x279779C), (PARSE_I, 0xDE4F88, 0x27977A0),
]);
gc!(h_gc_ctrlpat, rg_gc_ctrlpat, 0xDE50CC, true, [
    (PARSE_X, 0xDE50DC, 0x27977D0), (PARSE_F, 0xDE50EC, 0x27977D4),
]);
gc!(h_gc_taunt, rg_gc_taunt, 0xDE5100, true, [
    (PARSE_F, 0xDE5110, 0x27977D8), (PARSE_F, 0xDE5120, 0x27977DC), (PARSE_F, 0xDE5130, 0x27977E0),
]);
gc!(h_gc_fine_coll, rg_gc_fine_coll, 0xDE5140, true, [
    (PARSE_F, 0xDE5158, 0xE884D4), (PARSE_F, 0xDE5164, 0xE884D8), (PARSE_F, 0xDE5170, 0xE884E0),
    (PARSE_F, 0xDE517C, 0xE884DC), (PARSE_F, 0xDE5188, 0xE884EC), (PARSE_F, 0xDE5198, 0xE884F0),
    (PARSE_F, 0xDE51A8, 0xE884E4), (PARSE_F, 0xDE51B8, 0xE884E8),
]);
gc!(h_gc_flame, rg_gc_flame, 0xDE53BC, true, [
    (PARSE_I, 0xDE53CC, 0x27979E8), (PARSE_I, 0xDE53E8, 0x27979EC), (PARSE_I, 0xDE53FC, 0x27979F0),
]);
gc!(h_gc_veh_inter, rg_gc_veh_inter, 0xDE5488, true, [
    (PARSE_I, 0xDE549C, 0x2797A30), (PARSE_I, 0xDE54B8, 0x2797A34),
]);
gc!(h_gc_controls, rg_gc_controls, 0xDE54D4, false, [(PARSE_I, 0xDE54E0, 0x2797A38)]);
gc!(h_gc_stungun, rg_gc_stungun, 0xDE54F8, true, [
    (PARSE_I, 0xDE5500, 0x2797A3C), (PARSE_I, 0xDE5510, 0x2797A40),
]);
gc!(h_gc_rocket, rg_gc_rocket, 0xDE5524, true, [
    (PARSE_F, 0xDE5534, 0x2797A44), (PARSE_F, 0xDE5550, 0x2797A48), (PARSE_F, 0xDE556C, 0x2797A4C),
]);
gc!(h_gc_activity, rg_gc_activity, 0xDE5588, true, [
    (PARSE_F, 0xDE559C, 0x2797A54), (PARSE_F, 0xDE55B4, 0x2797A58), (PARSE_F, 0xDE55D4, 0x2797A5C),
    (PARSE_F, 0xDE55EC, 0x2797A60),
]);
gc!(h_gc_evade, rg_gc_evade, 0xDE57F8, true, [
    (PARSE_F, 0xDE580C, 0x2797AC8), (PARSE_F, 0xDE581C, 0x2797ACC), (PARSE_F, 0xDE5828, 0x2797AD0),
    (PARSE_F, 0xDE5838, 0x2797AD4),
]);
gc!(h_gc_pepper, rg_gc_pepper, 0xDE59B4, true, [
    (PARSE_F, 0xDE59C0, 0x2797B18), (PARSE_F, 0xDE59D0, 0x2797B1C), (PARSE_F, 0xDE59E0, 0x2797B28),
    (PARSE_F, 0xDE59F0, 0x2797B2C), (PARSE_F, 0xDE5A00, 0x2797B20), (PARSE_F, 0xDE5A14, 0x2797B24),
]);
gc!(h_gc_prop_throw, rg_gc_prop_throw, 0xDE5B98, true, [
    (PARSE_F, 0xDE5BB0, 0x2797B84), (PARSE_F, 0xDE5BCC, 0x2797B88), (PARSE_F, 0xDE5BE8, 0x2797B8C),
    (PARSE_F, 0xDE5BF4, 0x2797B90), (PARSE_F, 0xDE5C00, 0x2797B94), (PARSE_F, 0xDE5C10, 0x2797B98),
]);
gc!(h_gc_siren, rg_gc_siren, 0xDE5C78, true, [
    (PARSE_I, 0xDE5C8C, 0x2797BA8), (PARSE_I, 0xDE5C9C, 0x2797BAC), (PARSE_F, 0xDE5CAC, 0x2797BB0),
    (PARSE_F, 0xDE5CC0, 0x2797BB4), (PARSE_F, 0xDE5CD4, 0x2797BB8), (PARSE_F, 0xDE5CF0, 0x2797BBC),
]);
gc!(h_gc_garage, rg_gc_garage, 0xDE3BF8, false, [(PARSE_F, 0xDE5D08, 0x2797BC0)]);
gc!(h_gc_fall, rg_gc_fall, 0xDE5D1C, true, [
    (PARSE_F, 0xDE5D28, 0x2797BC4), (PARSE_F, 0xDE5D38, 0x2797BC8), (PARSE_F, 0xDE5D54, 0x2797BCC),
]);

/// 0x54d1f0: two float fields, each scaled by the double at 0xDD50E8 after its parse.
unsafe fn h_gc_fat_bones(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let tail = [p.edi, p.esi];
        let node = find_child(&mut s, a, 0xDE4674, &tail);
        s.edi = node;
        let c = dbl(0xDD50E8);
        seq(&mut s, a, node, &tail, &[(PARSE_F, 0xDE4680, 0x2797458), (PARSE_F, 0xDE4690, 0x2797454)], &mut |k, _| {
            let d = if k == 0 { 0x2797458 } else { 0x2797454 };
            wf(d, (rf(d) as f64 * c) as f32);
        });
        s.esi = p.esi;
        s.edi = p.edi;
        *p = s;
    }
}
fn rg_gc_fat_bones(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x2797454, 8));
}

// 0x54dba0: three ints parsed into one stack local (the slot of the pushed ECX), each converted
// (int * 0xE5C4F0) into a float; the final `pop ecx` takes the local back.
unsafe fn h_gc_item_drops(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let loc = slot(a, 1);
        wd(loc, p.ecx);
        let tail = [p.edi, p.esi, p.ecx];
        let node = find_child(&mut s, a, 0xDE4CD0, &tail);
        s.edi = node;
        let c = dbl(0xE5C4F0);
        let dests = [0x279770Cusize, 0x2797710, 0x2797714];
        seq(
            &mut s,
            a,
            node,
            &tail,
            &[(PARSE_I, 0xDE4CDC, loc as u32), (PARSE_I, 0xDE4CF0, loc as u32), (PARSE_I, 0xDE4D08, loc as u32)],
            &mut |k, _| wf(dests[k], (rd(loc) as i32 as f64 * c) as f32),
        );
        s.esi = p.esi;
        s.edi = p.edi;
        s.ecx = rd(loc);
        *p = s;
    }
}
fn rg_gc_item_drops(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x279770C, 12));
}

// 0x54f9a0: a float parsed into a stack local and scaled (0xE5CA98) into 0x2797A7C, then two more.
unsafe fn h_gc_kneecap(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let loc = slot(a, 1);
        wd(loc, p.ecx);
        let tail = [p.edi, p.esi, p.ecx];
        let node = find_child(&mut s, a, 0xDE567C, &tail);
        s.edi = node;
        seq(
            &mut s,
            a,
            node,
            &tail,
            &[(PARSE_F, 0xDE5688, loc as u32), (PARSE_X, 0xDE5698, 0x2797A80), (PARSE_X, 0xDE56A4, 0x2797A84)],
            &mut |k, _| {
                if k == 0 {
                    wf(0x2797A7C, (f32::from_bits(rd(loc)) as f64 * dbl(0xE5CA98)) as f32);
                }
            },
        );
        s.esi = p.esi;
        s.edi = p.edi;
        s.ecx = rd(loc);
        *p = s;
    }
}
fn rg_gc_kneecap(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x2797A7C, 12));
}

// 0x550240: three floats; then the second (0x2797BA0) is clamped down to the third (0x2797BA4).
unsafe fn h_gc_watercraft(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let tail = [p.edi, p.esi];
        let node = find_child(&mut s, a, 0xDE5C20, &tail);
        s.edi = node;
        seq(
            &mut s,
            a,
            node,
            &tail,
            &[(PARSE_F, 0xDE5C34, 0x2797B9C), (PARSE_F, 0xDE5C48, 0x2797BA0), (PARSE_F, 0xDE5C60, 0x2797BA4)],
            &mut |_, _| {},
        );
        // fcom: the store happens only when 0x2797BA4 < 0x2797BA0 (not on equal, greater or NaN);
        // `fnstsw ax` leaves the status word in AX.
        let sw = fcom_status(0x2797BA0, 0x2797BA4);
        s.eax = (s.eax & 0xFFFF_0000) | sw;
        if rf(0x2797BA4) < rf(0x2797BA0) {
            wd(0x2797BA0, q32(rd(0x2797BA4)));
        }
        s.esi = p.esi;
        s.edi = p.edi;
        *p = s;
    }
}
fn rg_gc_watercraft(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x2797B9C, 12));
}

// 0x54d5f0: four floats, then a tail jump to 0x54d580 with EAX = the node.
unsafe fn h_gc_sticky(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let tail = [p.edi, p.esi];
        let node = find_child(&mut s, a, 0xDE49FC, &tail);
        s.edi = node;
        seq(
            &mut s,
            a,
            node,
            &tail,
            &[(PARSE_F, 0xDDBEC0, 0x27974C4), (PARSE_F, 0xDDBEF8, 0x27974C8), (PARSE_F, 0xDDBF30, 0x27974CC), (PARSE_F, 0xDDBF68, 0x27974D0)],
            &mut |_, _| {},
        );
        // mov eax, edi; pop edi; pop esi; jmp: the callee sees the caller's ESI / EDI and arguments
        s.eax = node;
        s.esi = p.esi;
        s.edi = p.edi;
        cx(0x0054_D580, &mut s, &fr_tail(a));
        *p = s;
    }
}

// camera_free_*: the node is in EDI (or EAX) already; the fields are fixed.
macro_rules! cam {
    ($h:ident, $rg:ident, $node:ident, [$(($f:expr, $s:expr, $d:expr)),* $(,)?]) => {
        unsafe fn $h(p: &mut Pushad, a: *const u32) {
            unsafe {
                let mut s = *p;
                let node = p.$node;
                // `push esi` first; the one-field readers push EAX, the others EDI (the node)
                seq(&mut s, a, node, &[p.esi], &[$(($f, $s, $d)),*], &mut |_, _| {});
                s.esi = p.esi;
                *p = s;
            }
        }
        fn $rg(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
            $( v.push(($d as usize, 4)); )*
        }
    };
}

cam!(h_cam_stationary, rg_cam_stationary, edi, [
    (PARSE_F, 0xDD9AD8, 0xE999BC), (PARSE_F, 0xDD9AE8, 0xE999C0), (PARSE_F, 0xDD9AF8, 0xE999C4),
]);
cam!(h_cam_crouch, rg_cam_crouch, edi, [
    (PARSE_F, 0xDD9B10, 0xE999D4), (PARSE_F, 0xDD9B20, 0xE999CC), (PARSE_F, 0xDD9B30, 0xE999D0),
    (PARSE_F, 0xDD9B44, 0xE999C8),
]);
cam!(h_cam_swing, rg_cam_swing, eax, [(PARSE_F, 0xDD9B58, 0xE999DC)]);
cam!(h_cam_aggr, rg_cam_aggr, eax, [(PARSE_F, 0xDD9B6C, 0xE999E0)]);

// 0x4961e0: eight floats, then two of them scaled (doubles at 0xE5C4E8 / 0xE5C2A8).
unsafe fn h_cam_pitch(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        seq(
            &mut s,
            a,
            p.edi,
            &[p.esi],
            &[
                (PARSE_F, 0xDD9B80, 0xE999E4), (PARSE_F, 0xDD9B94, 0xE999E8), (PARSE_F, 0xDD9BA4, 0xF539D8),
                (PARSE_F, 0xDD9BC0, 0xE999A8), (PARSE_F, 0xDD9BDC, 0xE999AC), (PARSE_F, 0xDD9BFC, 0xE999B0),
                (PARSE_F, 0xDD9C1C, 0xE9985C), (PARSE_F, 0xDD9C34, 0xE99860),
            ],
            &mut |_, _| {},
        );
        wf(0xE9985C, (rf(0xE9985C) as f64 * dbl(0xE5C4E8)) as f32);
        wf(0xF539D8, (rf(0xF539D8) as f64 * dbl(0xE5C2A8)) as f32);
        s.esi = p.esi;
        *p = s;
    }
}
fn rg_cam_pitch(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    for a in [0xE999E4usize, 0xE999E8, 0xF539D8, 0xE999A8, 0xE999AC, 0xE999B0, 0xE9985C, 0xE99860] {
        v.push((a, 4));
    }
}

// --- gamesave sections -----------------------------------------------------------------------------------
// Every `h_gs*` takes the buffer pointer as stack argument 0. Each handler plays the original's register
// effects on `s` (the pushad block plus XMM0-7): what the original loads into a register is set there, what
// it saves and restores comes back from `p`, and every callee is called by address through `cx` with the
// stack words the original has at that call (`fr`).

fn rg_buf(a: *const u32, v: &mut Vec<(usize, usize)>, off: usize, len: usize) {
    let dst = unsafe { *a } as usize;
    v.push((dst.wrapping_add(off), len));
}

// 0x6924f0: the district list from the replicated copy 0x27ABD00 (count) / 0x27ABD0C (entries).
unsafe fn h_gs04c(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        let count = rd(0x27ABD00);
        wd(dst + 0x17304, count);
        p.ecx = count;
        p.eax = dst as u32;
        if (rd(0x27ABD00) as i32) > 0 {
            let mut edi = (rd(0x2526C94) as usize).wrapping_add(0x59);
            let mut eax = dst + 0x17310;
            let mut esi = 0x27ABD0Cusize;
            let mut ebp = 0i32;
            let (mut ecx, mut edx);
            loop {
                wd(eax - 8, rd(esi - 8));
                wd(eax - 4, rd(esi - 4));
                let dl = rb(esi);
                wb(eax, dl);
                let mut cl = rb(edi - 1) >> 6;
                cl ^= dl;
                cl &= 1;
                wb(eax, rb(eax) ^ cl);
                let mut dl2 = rb(edi);
                let cl2 = rb(eax);
                dl2 = dl2.wrapping_add(dl2);
                dl2 ^= cl2;
                dl2 &= 2;
                dl2 ^= cl2;
                wb(eax, dl2);
                ecx = cl2 as u32;
                edx = dl2 as u32;
                ebp += 1;
                esi += 12;
                eax += 12;
                edi = edi.wrapping_add(0xA8);
                if ebp >= rd(0x27ABD00) as i32 {
                    break;
                }
            }
            p.eax = eax as u32;
            p.ecx = ecx;
            p.edx = edx;
        }
    }
}
fn rg_gs04c(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    let n = unsafe { rd(0x27ABD00) as i32 }.clamp(0, 4096) as usize;
    rg_buf(a, v, 0x17304, 12 + n * 12);
}

// 0x692b00 / 0x692b40 (identical bodies): the activity-levels pointer (or the default block).
unsafe fn h_gs09_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0) as usize;
        let ptr = rd(0x2527C88);
        s.eax = if ptr == 0 { rd(0x33D630C) } else { rd(ptr as usize) };
        s.esi = dst as u32;
        wd(dst + 0x74, s.eax);
        cx(0x696B50, &mut s, &fr(a, &[p.esi]));
        wd(dst + 0x78, s.eax);
        s.esi = p.esi;
        *p = s;
    }
}

// 0x692b80: a two-dword local {0xE008D0, buffer} (the original's frame) is passed in EDI.
unsafe fn h_gs10_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let buf = arg(a, 0);
        wd(slot(a, 2), 0x00E0_08D0);
        wd(slot(a, 1), buf);
        s.eax = buf;
        s.edi = slot(a, 2) as u32;
        cx(0x00A6_EA00, &mut s, &fr(a, &[p.edi, 0x00E0_08D0, buf]));
        s.edi = p.edi;
        *p = s;
    }
}

// 0x692c60: the cribs (co-op client copy) and the clamped cash.
unsafe fn h_gs11c_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0) as usize;
        (dst + 0x382A4).wr16(rb(0x27AC238) as u16);
        let cnt16 = (0x27ABFDCusize).rd16();
        (dst + 0x382A6).wr16(cnt16);
        s.ecx = (p.ecx & 0xFFFF_0000) | cnt16 as u32;
        s.ebx = 0;
        s.ebp = dst as u32;
        if rd(0x27ABFDC) != 0 {
            let mut esi = 0x27ABFE0u32;
            let mut edi = (dst + 0x382A8) as u32;
            loop {
                s.ecx = esi;
                s.eax = edi;
                s.esi = esi;
                s.edi = edi;
                cx(0x692D00, &mut s, &fr(a, &[p.edi, p.esi, p.ebp, p.ebx]));
                s.ebx = s.ebx.wrapping_add(1);
                esi += 0x28;
                edi += 0x28;
                if s.ebx >= rd(0x27ABFDC) {
                    break;
                }
            }
            s.esi = p.esi;
            s.edi = p.edi;
        }
        let cash = rd(0x27973CC) as i32;
        let v = cash.clamp(0x88CA_6C00u32 as i32, 0x7735_9400);
        wd(dst + 0x38500, v as u32);
        s.eax = cash as u32;
        s.ebx = p.ebx;
        s.ebp = p.ebp;
        *p = s;
    }
}

trait Mem16 {
    unsafe fn rd16(self) -> u16;
    unsafe fn wr16(self, v: u16);
}
impl Mem16 for usize {
    unsafe fn rd16(self) -> u16 {
        unsafe { *(self as *const u16) }
    }
    unsafe fn wr16(self, v: u16) {
        unsafe { *(self as *mut u16) = v }
    }
}

// 0x692e80
unsafe fn h_gs00_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        let ptr = dst + 0x3767C;
        s.esi = dst;
        s.eax = ptr;
        cx(0x542940, &mut s, &fr(a, &[ptr, p.esi]));
        wd(dst as usize + 0x37678, s.eax);
        s.esi = p.esi;
        *p = s;
    }
}

// 0x692ea0: EDI = buffer + 0x38C38
unsafe fn h_gs14_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.edi = arg(a, 0).wrapping_add(0x38C38);
        cx(0x6AB5C0, &mut s, &fr(a, &[p.edi]));
        s.edi = p.edi;
        *p = s;
    }
}

// 0x692ec0 / 0x694b10: the cutscene-seen list through a 3-dword local {ptr, 0x20, 0}, ESI = its address.
unsafe fn cutscene_seen(p: &mut Pushad, a: *const u32, f: usize, gate: bool) {
    unsafe {
        if gate && rb(a.add(1) as usize) == 0 {
            return;
        }
        if (rd(0x145A4AC) as i32) >= 0x100 {
            return;
        }
        let mut s = *p;
        let eax = arg(a, 0).wrapping_add(0x39B68);
        wd(slot(a, 3), eax);
        wd(slot(a, 2), 0x20);
        wb(slot(a, 1), 0);
        s.eax = eax;
        s.esi = slot(a, 3) as u32;
        cx(f, &mut s, &fr(a, &[p.esi, eax, 0x20, rd(slot(a, 1))]));
        s.esi = p.esi;
        *p = s;
    }
}
unsafe fn h_gs15_save(p: &mut Pushad, a: *const u32) {
    unsafe { cutscene_seen(p, a, 0x6C3AD0, false) }
}
unsafe fn h_gs15_load(p: &mut Pushad, a: *const u32) {
    unsafe { cutscene_seen(p, a, 0x6C3B30, true) }
}

// 0x692f00 / 0x694ae0: the persistent objects through a 3-dword local {ptr, 0xED, 0}, whose address
// is the stack argument (and ECX).
unsafe fn persistent(p: &mut Pushad, a: *const u32, f: usize) {
    unsafe {
        let mut s = *p;
        let eax = arg(a, 0).wrapping_add(0x39B88);
        let ptr = slot(a, 3) as u32;
        wd(slot(a, 3), eax);
        wd(slot(a, 2), 0xED);
        wb(slot(a, 1), 0);
        s.eax = eax;
        s.ecx = ptr;
        cx(f, &mut s, &fr(a, &[ptr, eax, 0xED, rd(slot(a, 1))]));
        *p = s;
    }
}
unsafe fn h_gs16_save(p: &mut Pushad, a: *const u32) {
    unsafe { persistent(p, a, 0xA8CE10) }
}
unsafe fn h_gs16_load(p: &mut Pushad, a: *const u32) {
    unsafe { persistent(p, a, 0xA8CEB0) }
}

// 0x692f30: the reward counters (x87 dword copies: a signalling NaN comes out quiet).
unsafe fn h_gs17_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        let ecx = rd(0x140B204);
        wd(dst + 0x39CB8, ecx);
        wd(dst + 0x3A0C8, q32(rd(0x140B210)));
        wd(dst + 0x3A0CC, q32(rd(0x140B220)));
        wd(dst + 0x3A0D0, q32(rd(0x140B230)));
        wd(dst + 0x3A0D4, q32(rd(0x140B240)));
        p.eax = dst as u32;
        p.ecx = ecx;
    }
}
fn rg_gs17_save(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    rg_buf(a, v, 0x39CB8, 4);
    rg_buf(a, v, 0x3A0C8, 16);
}

// 0x692fb0: ESI = buffer + 0x39DB8, skipped when that wraps to zero.
unsafe fn h_gs18_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let esi = arg(a, 0).wrapping_add(0x39DB8);
        if esi != 0 {
            let mut s = *p;
            s.esi = esi;
            cx(0x563040, &mut s, &fr(a, &[p.esi]));
            s.esi = p.esi;
            *p = s;
        }
    }
}

// 0x693450
unsafe fn h_gs20_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        s.esi = dst;
        s.edx = dst.wrapping_add(0x39DC8);
        cx(0x688220, &mut s, &fr(a, &[p.esi]));
        wd(dst as usize + 0x39DC4, s.eax);
        s.esi = p.esi;
        *p = s;
    }
}

// 0x693470: 134 dwords, every 0x20 bytes from 0x27DAEA4.
unsafe fn h_gs12_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        let mut ecx = dst.wrapping_add(0x385F8);
        let mut eax = 0usize;
        let mut edx;
        loop {
            edx = rd(eax.wrapping_add(0x27DAEA4));
            wd(ecx, edx);
            eax += 0x20;
            ecx = ecx.wrapping_add(4);
            if eax >= 0x10C0 {
                break;
            }
        }
        p.eax = eax as u32;
        p.ecx = ecx as u32;
        p.edx = edx;
    }
}
fn rg_gs12_save(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    rg_buf(a, v, 0x385F8, 134 * 4);
}

// 0x693570
unsafe fn h_gs01_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        s.esi = dst;
        s.edx = dst.wrapping_add(0x38508);
        cx(0xA7C910, &mut s, &fr(a, &[p.esi]));
        wd(dst as usize + 0x38504, s.eax);
        s.esi = p.esi;
        *p = s;
    }
}

/// A tail `jmp callee` after the register set-up in `s`: the callee gets the original stack arguments.
unsafe fn tail(p: &mut Pushad, a: *const u32, f: usize, mut s: Pushad) {
    unsafe {
        cx(f, &mut s, &fr_tail(a));
        *p = s;
    }
}

unsafe fn h_gs05_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.ecx = arg(a, 0).wrapping_add(0x3A148);
        tail(p, a, 0x7DECD0, s)
    }
}
unsafe fn h_gs23_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A16C);
        tail(p, a, 0x48B7D0, s)
    }
}
unsafe fn h_gs24_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A1CC);
        tail(p, a, 0x491140, s)
    }
}

// 0x693740: ESI = buffer + 0x3A9CC
unsafe fn h_gs25_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.esi = arg(a, 0).wrapping_add(0x3A9CC);
        cx(0x491200, &mut s, &fr(a, &[p.esi]));
        s.esi = p.esi;
        *p = s;
    }
}

// 0x693760: ECX = the address of a local holding 0x5A; the pointer is the stack argument.
unsafe fn h_gs26_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ptr = arg(a, 0).wrapping_add(0x3AA0C);
        wd(slot(a, 1), 0x5A);
        s.eax = ptr;
        s.ecx = slot(a, 1) as u32;
        cx(0x7AEBD0, &mut s, &fr(a, &[ptr, 0x5A]));
        *p = s;
    }
}

// 0x6937a0: EDI = buffer + 0x3AA68
unsafe fn h_gs27_save(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.edi = arg(a, 0).wrapping_add(0x3AA68);
        cx(0x7B07F0, &mut s, &fr(a, &[p.edi]));
        s.edi = p.edi;
        *p = s;
    }
}

// 0x694470: the garage load: free records (first dword zero) get 0x5556F0.
unsafe fn h_gs07_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        cx(0x5558F0, &mut s, &fr(a, &[p.esi, p.ebx]));
        let ebx = arg(a, 0) as usize;
        s.ebx = ebx as u32;
        s.eax = rd(ebx + 0x37270);
        s.esi = 0;
        wd(0x1409998, s.eax);
        if (rd(ebx + 0x175D8) as i32) > 0 {
            let mut edi = ebx + 0x175DC;
            loop {
                s.edi = edi as u32;
                if rd(edi) == 0 {
                    cx(0x5556F0, &mut s, &fr(a, &[edi as u32, p.edi, p.esi, p.ebx]));
                }
                s.esi = s.esi.wrapping_add(1);
                edi += 0x434;
                if (s.esi as i32) >= rd(ebx + 0x175D8) as i32 {
                    break;
                }
            }
            s.edi = p.edi;
        }
        s.esi = p.esi;
        s.ebx = p.ebx;
        *p = s;
    }
}

// 0x6944c0: the wiretap flag byte, then a tail jump (EAX = the buffer).
unsafe fn h_gs22_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        let cl_ = rb(dst as usize + 0x379C8);
        wb(0xFA239B, cl_);
        s.eax = dst;
        s.ecx = (p.ecx & !0xFF) | cl_ as u32;
        tail(p, a, 0x54CCA0, s);
    }
}

// 0x6947c0 / 0x6947e0: the activity-levels object from the buffer; kept unless it has +0x3C set.
unsafe fn h_gs09_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.ecx = arg(a, 0).wrapping_add(0x74);
        cx(0x69E1C0, &mut s, &fr(a, &[]));
        if s.eax == 0 || rd(s.eax as usize + 0x3C) == 0 {
            wd(0x2527C88, s.eax);
        }
        *p = s;
    }
}

// 0x694800: script-trigger records re-applied to the objects in the circular list at 0x2F8E88C.
unsafe fn h_gs10_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        if (rd(dst + 0x37AD0) as i32) <= 0 {
            return;
        }
        let mut s = *p;
        let saved = [p.esi, p.ebx, p.edi, p.ebp];
        s.edi = dst as u32;
        s.ebp = 0;
        s.eax = rd(0x2F8E88C);
        s.ebx = (dst + 0x37AD8) as u32;
        loop {
            // 0x694821
            s.esi = s.eax;
            'body: {
                if s.eax == 0 {
                    break 'body;
                }
                s.ecx = rd(s.ebx as usize - 4);
                loop {
                    if rd(s.esi as usize + 0xC0) == s.ecx {
                        break;
                    }
                    s.esi = rd(s.esi as usize + 0x188);
                    if s.esi == s.eax || s.esi == 0 {
                        break 'body;
                    }
                }
                // 0x694848
                if rb(s.ebx as usize) & 2 != 0 {
                    let al = rb(s.esi as usize + 0x110);
                    s.eax = (s.eax & !0xFF) | al as u32;
                    if al & 0x10 == 0 {
                        s.edi = rd(s.esi as usize + 0x17C);
                        wb(s.esi as usize + 0x110, al | 0x10);
                        s.eax = rd(s.esi as usize + 0x180);
                        cx(0x7A4DC0, &mut s, &fr(a, &saved_frame(&saved)));
                        s.edi = dst as u32;
                    }
                }
                if rb(s.ebx as usize) & 1 != 0 {
                    s.eax = s.esi;
                    cx(0xA6BD60, &mut s, &fr(a, &saved_frame(&saved)));
                } else {
                    cx(0xA6BCE0, &mut s, &fr(a, &saved_frame(&saved)));
                }
                s.eax = rd(0x2F8E88C);
            }
            // 0x694890
            s.ebp = s.ebp.wrapping_add(1);
            s.ebx = s.ebx.wrapping_add(8);
            if (s.ebp as i32) >= rd(dst + 0x37AD0) as i32 {
                break;
            }
        }
        s.esi = saved[0];
        s.ebx = saved[1];
        s.edi = saved[2];
        s.ebp = saved[3];
        *p = s;
    }
}

/// `push ebp; push edi; push ebx; push esi` as the original's frame at a call, top first.
fn saved_frame(s: &[u32; 4]) -> [u32; 4] {
    [s[0], s[1], s[2], s[3]]
}

// 0x6948b0: the co-op client cribs.
unsafe fn h_gs11c_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0) as usize;
        wb(0x27AC238, ((dst + 0x382A4).rd16() != 0) as u8);
        let cnt = (dst + 0x382A6).rd16() as u32;
        s.eax = cnt;
        s.ebx = 0;
        s.ebp = dst as u32;
        wd(0x27ABFDC, cnt);
        if cnt != 0 {
            let mut esi = (dst + 0x382A8) as u32;
            let mut edi = 0x27ABFE0u32;
            loop {
                s.ecx = esi;
                s.eax = edi;
                s.esi = esi;
                s.edi = edi;
                cx(0x692D00, &mut s, &fr(a, &[p.edi, p.esi, p.ebp, p.ebx]));
                s.ebx = s.ebx.wrapping_add(1);
                esi += 0x28;
                edi += 0x28;
                if s.ebx >= rd(0x27ABFDC) {
                    break;
                }
            }
            s.esi = p.esi;
            s.edi = p.edi;
        }
        let ecx = rd(dst + 0x38500);
        s.ecx = ecx;
        cx(0x548F00, &mut s, &fr(a, &[ecx, p.ebp, p.ebx]));
        s.ebx = p.ebx;
        s.ebp = p.ebp;
        *p = s;
    }
}

// 0x694ab0
unsafe fn h_gs00_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        s.eax = rd(dst as usize + 0x37678);
        s.ecx = dst.wrapping_add(0x3767C);
        tail(p, a, 0x542A10, s);
    }
}

// 0x694b50: the reward counters back through 0x605CA0(index) (float, 1).
unsafe fn h_gs17_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        if rb(a.add(1) as usize) == 0 {
            return;
        }
        let mut s = *p;
        let dst = arg(a, 0) as usize;
        let ecx = rd(dst + 0x39CB8);
        wd(0x140B204, ecx);
        s.eax = dst as u32;
        s.ecx = ecx;
        s.esi = 0;
        let mut edi = dst + 0x3A0C8;
        loop {
            s.edi = edi as u32;
            if (s.esi as i32) > -1 {
                s.eax = s.esi;
                cx(0x605CA0, &mut s, &fr(a, &[q32(rd(edi)), 1, p.edi, p.esi]));
            }
            s.esi = s.esi.wrapping_add(1);
            edi += 4;
            if s.esi >= 4 {
                break;
            }
        }
        s.esi = p.esi;
        s.edi = p.edi;
        *p = s;
    }
}

// 0x694bd0: ECX = buffer + 0x39DB8, a tail jump unless that wraps to zero.
unsafe fn h_gs18_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.ecx = arg(a, 0).wrapping_add(0x39DB8);
        if s.ecx != 0 {
            tail(p, a, 0x5630D0, s);
        } else {
            *p = s;
        }
    }
}

// 0x694e20
unsafe fn h_gs20_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        let ecx = rd(dst as usize + 0x39DC4);
        s.eax = dst.wrapping_add(0x39DC8);
        s.ecx = ecx;
        cx(0x688260, &mut s, &fr(a, &[ecx]));
        s.ecx = ecx; // pop ecx: the pushed word
        *p = s;
    }
}

// 0x694e40: the reverse of the tutorials copy.
unsafe fn h_gs12_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        let mut ecx = dst.wrapping_add(0x385F8);
        let mut eax = 0usize;
        let mut edx;
        loop {
            edx = rd(ecx);
            wd(eax.wrapping_add(0x27DAEA4), edx);
            eax += 0x20;
            ecx = ecx.wrapping_add(4);
            if eax >= 0x10C0 {
                break;
            }
        }
        p.eax = eax as u32;
        p.ecx = ecx as u32;
        p.edx = edx;
    }
}
fn rg_gs12_load(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    for i in 0..134usize {
        v.push((0x27DAEA4 + i * 0x20, 4));
    }
}

// 0x694e70: eight debug-display option words (ECX / EDX alternate).
unsafe fn h_gs13_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        const D: [usize; 8] = [0x27ABA94, 0x27ABAAC, 0x27ABAC4, 0x27ABADC, 0x27ABAF4, 0x27ABB0C, 0x27ABB24, 0x27ABB3C];
        for (i, d) in D.iter().enumerate() {
            let v = rd(dst + 0x38810 + 4 * i);
            wd(*d, v);
            if i % 2 == 0 {
                p.ecx = v;
            } else {
                p.edx = v;
            }
        }
        p.eax = dst as u32;
    }
}
fn rg_gs13_load(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    for d in [0x27ABA94usize, 0x27ABAAC, 0x27ABAC4, 0x27ABADC, 0x27ABAF4, 0x27ABB0C, 0x27ABB24, 0x27ABB3C] {
        v.push((d, 4));
    }
}

// 0x694ee0: the world-grid pairs of the co-op copy (count at +0x38504, pairs from +0x38508).
unsafe fn h_gs01c_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let dst = arg(a, 0) as usize;
        let count = rd(dst + 0x38504);
        wd(0x27AC23C, count);
        let mut eax = 0usize;
        p.edx = dst as u32;
        p.ecx = count;
        if (count as i32) > 0 {
            let mut ecx = dst + 0x38508;
            let mut edx;
            loop {
                edx = rd(ecx);
                wd(eax * 8 + 0x27AC240, edx);
                edx = rd(ecx + 4);
                wd(eax * 8 + 0x27AC244, edx);
                eax += 1;
                ecx += 8;
                if eax as i32 >= rd(0x27AC23C) as i32 {
                    break;
                }
            }
            p.ecx = ecx as u32;
            p.edx = edx;
        }
        p.eax = eax as u32;
    }
}
fn rg_gs01c_load(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    let dst = unsafe { *a } as usize;
    let n = unsafe { rd(dst + 0x38504) as i32 }.clamp(0, 4096) as usize;
    v.push((0x27AC23C, 4));
    v.push((0x27AC240, n.max(1) * 8));
}

// 0x694f30: a tail jump with EDX = the count, ECX = the pairs, EAX = the buffer.
unsafe fn h_gs01_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let dst = arg(a, 0);
        s.eax = dst;
        s.edx = rd(dst as usize + 0x38504);
        s.ecx = dst.wrapping_add(0x38508);
        tail(p, a, 0xA7C9D0, s);
    }
}

unsafe fn h_gs05_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A148);
        tail(p, a, 0x7DED80, s)
    }
}
unsafe fn h_gs23_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A16C);
        tail(p, a, 0x48B870, s)
    }
}
unsafe fn h_gs24_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A1CC);
        tail(p, a, 0x491180, s)
    }
}
unsafe fn h_gs25_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0).wrapping_add(0x3A9CC);
        tail(p, a, 0x491260, s)
    }
}

// 0x6950f0 / 0x695130: EAX = the pointer, which is also the stack argument (popped into ECX after).
unsafe fn h_gs26_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ptr = arg(a, 0).wrapping_add(0x3AA0C);
        s.eax = ptr;
        cx(0x7AED60, &mut s, &fr(a, &[ptr]));
        s.ecx = ptr;
        *p = s;
    }
}
unsafe fn h_gs27_load(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ptr = arg(a, 0).wrapping_add(0x3AA68);
        s.eax = ptr;
        cx(0x7B08C0, &mut s, &fr(a, &[ptr]));
        s.ecx = ptr;
        *p = s;
    }
}


// --- packet handlers -------------------------------------------------------------------------------------------
// Called by pointer from the receive tables: cdecl (data pointer, connection-info pointer), no register
// parameters, no result.

unsafe fn rdu(p: usize) -> u32 {
    unsafe { std::ptr::read_unaligned(p as *const u32) }
}

unsafe fn rd16(p: usize) -> u16 {
    unsafe { (p as *const u16).read_unaligned() }
}

unsafe fn wr16(p: usize, v: u16) {
    unsafe { (p as *mut u16).write_unaligned(v) }
}

fn ag(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

// 0x80b5b0: the connection's QoS word.
unsafe fn h_recv_qos(p: &mut Pushad, a: *const u32) {
    unsafe {
        let edx = arg(a, 0);
        let ecx = rd(arg(a, 1) as usize + 4);
        let eax = rd(edx as usize);
        wd(ecx as usize + 0x1DC0, eax);
        p.eax = eax;
        p.ecx = ecx;
        p.edx = edx;
    }
}
fn rg_recv_qos(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe { v.push((rd(ag(a, 1) as usize + 4) as usize + 0x1DC0, 4)) }
}

// 0x80b5e0: the bandwidth word, then net_send_upd_conn (0x816B50, EDI = the connection) when it asks.
unsafe fn h_recv_bw_info(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ecx = arg(a, 1);
        s.ecx = ecx;
        s.eax = rd(ecx as usize + 4);
        if s.eax == 0 {
            *p = s;
            return;
        }
        s.edx = rd(arg(a, 0) as usize);
        wd(s.eax as usize + 0x1DBC, s.edx);
        s.eax = rd(ecx as usize + 4);
        if rd(s.eax as usize + 0x1A8) == 0 {
            *p = s;
            return;
        }
        s.edi = s.eax;
        cx(0x816B50, &mut s, &fr(a, &[p.edi]));
        s.edi = p.edi;
        *p = s;
    }
}

// 0x810a20 (32 bytes): copy a field of the session.
unsafe fn h_recv_next_srv(p: &mut Pushad, a: *const u32) {
    unsafe {
        let c = rd(arg(a, 1) as usize + 4);
        if c == 0 {
            p.eax = arg(a, 1);
            return;
        }
        let o = rd(c as usize + 0x1AC) as usize;
        let ecx = rd(o + 0x408);
        wd(o + 0xA10, ecx);
        p.eax = o as u32;
        p.ecx = ecx;
    }
}
fn rg_recv_next_srv(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let c = rd(ag(a, 1) as usize + 4) as usize;
        if c != 0 {
            v.push((rd(c + 0x1AC) as usize + 0xA10, 4));
        }
    }
}

// 0x810bd0: append the sender's 9-dword record (+ one more dword) to the session's list of 40-byte entries.
unsafe fn h_recv_modified_session(p: &mut Pushad, a: *const u32) {
    unsafe {
        let ecx = rd(arg(a, 1) as usize + 4) as usize;
        let eax = rd(ecx + 0x1AC) as usize;
        p.eax = eax as u32;
        p.ecx = ecx as u32;
        if rd(eax + 0x2C) != 2 {
            return;
        }
        let ebx = rd(ecx + 0x170);
        let n = rd(eax + 0xBF8);
        let dst = eax.wrapping_add((n.wrapping_mul(40)) as usize).wrapping_add(0xA18);
        for i in 0..9 {
            wd(dst.wrapping_add(i * 4), rd(ecx + 0x14C + i * 4));
        }
        wd(dst.wrapping_add(0x24), ebx);
        wd(eax + 0xBF8, rd(eax + 0xBF8).wrapping_add(1));
        p.ecx = 0; // rep movsd ran ECX down to zero
        p.edx = dst as u32;
    }
}
fn rg_recv_modified_session(_p: &Pushad, a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let ecx = rd(ag(a, 1) as usize + 4) as usize;
        let eax = rd(ecx + 0x1AC) as usize;
        if rd(eax + 0x2C) == 2 {
            let n = rd(eax + 0xBF8);
            v.push((eax.wrapping_add((n.wrapping_mul(40)) as usize).wrapping_add(0xA18), 40));
            v.push((eax + 0xBF8, 4));
        }
    }
}

// 0x924d30: bit i of the word marks entry i of the table at 0xE88B68 (byte +0x2A4).
unsafe fn h_recv_notor_active(p: &mut Pushad, a: *const u32) {
    unsafe {
        let eax = rd16(arg(a, 0) as usize) as u32;
        let mut edx = 0u32;
        for i in 0..14u32 {
            let esi = rd(0xE8_8B68 + i as usize * 4) as usize;
            let flag = (eax & (1 << i) != 0) as u32;
            wb(esi + 0x2A4, flag as u8);
            edx = (1u32 << i & !0xFF) | flag;
        }
        p.eax = eax;
        p.ecx = 14;
        p.edx = edx;
    }
}
fn rg_recv_notor_active(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    for i in 0..14usize {
        v.push((unsafe { rd(0xE8_8B68 + i * 4) } as usize + 0x2A4, 1));
    }
}

// 0x548580: EDX = the first dword of the data, one stack argument (1), popped into ECX after.
unsafe fn h_misc_pkt_05(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0);
        s.edx = rd(s.eax as usize);
        cx(0x548590, &mut s, &fr(a, &[1]));
        s.ecx = 1;
        *p = s;
    }
}

// 0xa2c670
unsafe fn h_misc_pkt_17(_p: &mut Pushad, _a: *const u32) {
    unsafe { wb(0x27AC238, 1) }
}
fn rg_misc_pkt_17(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x27AC238, 1));
}

// 0xa2ddb0: clear a flag, then a tail jump.
unsafe fn h_hud_pkt_16(p: &mut Pushad, a: *const u32) {
    unsafe {
        wb(0x252849F, 0);
        tail(p, a, 0x794230, *p);
    }
}

// 0x7fb150: each u16 in the packet (count byte first) is an ack for the connection (net_conn_ack_record).
unsafe fn h_recv_acks(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ebp = arg(a, 1) as usize;
        if rd(ebp + 4) == 0 {
            return;
        }
        let ecx = arg(a, 0);
        s.ecx = ecx;
        let al = rb(ecx as usize);
        s.eax = (p.eax & !0xFF) | al as u32;
        if al == 0 {
            *p = s;
            return;
        }
        s.ebp = ebp as u32;
        let mut edi = ecx as usize + 1;
        s.ebx = al as u32;
        loop {
            s.esi = rd16(edi) as u32;
            s.eax = rd(ebp + 4);
            edi += 2;
            s.edi = edi as u32;
            cx(0x7F8270, &mut s, &fr(a, &[p.edi, p.esi, p.ebx, p.ebp]));
            s.ebx = s.ebx.wrapping_sub(1);
            if s.ebx == 0 {
                break;
            }
        }
        s.edi = p.edi;
        s.esi = p.esi;
        s.ebx = p.ebx;
        s.ebp = p.ebp;
        *p = s;
    }
}

// 0x835930: a tail jump with EAX = the session, ECX = the connection.
unsafe fn h_recv_sessionstart(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ecx = rd(arg(a, 1) as usize + 4);
        s.ecx = ecx;
        s.eax = rd(ecx as usize + 0x1AC);
        tail(p, a, 0x80C9A0, s);
    }
}

// 0x5af3f0
unsafe fn h_act_3f(p: &mut Pushad, _a: *const u32) {
    unsafe {
        let e = rd(0x1408E88) as usize;
        p.eax = e as u32;
        if e == 0 {
            return;
        }
        let o = rd(e + 0x10) as usize;
        p.eax = o as u32;
        if o == 0 {
            return;
        }
        let ecx = rd(o + 0x40);
        wd(o + 0x44, ecx);
        p.ecx = ecx;
    }
}
fn rg_act_3f(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let e = rd(0x1408E88) as usize;
        if e != 0 {
            let o = rd(e + 0x10) as usize;
            if o != 0 {
                v.push((o + 0x44, 4));
            }
        }
    }
}

// 0x5af390: up to 12 u16 values (count first, then the values) into the local player's table at +0x19C.
unsafe fn h_act_3e(p: &mut Pushad, a: *const u32) {
    unsafe {
        let w = rd(0x27B3C60) as usize;
        p.eax = w as u32;
        if w == 0 || rd(w + 0x3C) != 1 {
            return;
        }
        let edi = rd(w + 0x90) as usize;
        if edi == 0 {
            return;
        }
        let args = arg(a, 0) as usize;
        let n = rd(args) as i32;
        let mut src = args + 4;
        let mut dst = edi + 0x19C;
        for i in 0..12i32 {
            wr16(dst, 0xFFFF);
            if i < n {
                wr16(dst, rd16(src));
                src += 2;
            }
            dst += 2;
        }
        wb(edi + 0x19A, (n > 0) as u8);
        p.eax = (n > 0) as u32; // eax = 12, then `setg al`
        p.ecx = dst as u32;
        p.edx = src as u32;
    }
}
fn rg_act_3e(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let w = rd(0x27B3C60) as usize;
        if w != 0 && rd(w + 0x3C) == 1 {
            let edi = rd(w + 0x90) as usize;
            if edi != 0 {
                v.push((edi + 0x19A, 2 + 24));
            }
        }
    }
}

// 0x5bbba0: four dwords into the local player's record at +0x2BC; +0x2C0 only ever grows.
unsafe fn h_act_44(p: &mut Pushad, a: *const u32) {
    unsafe {
        let m = arg(a, 0) as usize;
        let edx = rd(m);
        let ecx0 = rd(0x27B3C60) as usize;
        let (esi, edi, eax) = (rd(m + 4), rd(m + 8), rd(m + 12));
        p.eax = eax;
        p.edx = edx;
        p.ecx = ecx0 as u32;
        if ecx0 == 0 || rd(ecx0 + 0x3C) != 1 {
            return;
        }
        let ecx = rd(ecx0 + 0x90) as usize;
        p.ecx = ecx as u32;
        if ecx == 0 {
            return;
        }
        let big = (rd(ecx + 0x2C0) as i32) >= edx as i32;
        wd(ecx + 0x2BC, edx);
        if !big {
            wd(ecx + 0x2C0, edx);
        }
        wd(ecx + 0x2C4, esi);
        wd(ecx + 0x2C8, edi);
        wd(ecx + 0x2CC, eax);
    }
}
fn rg_act_44(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let w = rd(0x27B3C60) as usize;
        if w != 0 && rd(w + 0x3C) == 1 {
            let ecx = rd(w + 0x90) as usize;
            if ecx != 0 {
                v.push((ecx + 0x2BC, 0x14));
            }
        }
    }
}

// 0x5f7e30: three floats (through the x87: a signalling NaN comes out quiet). The original stages them in
// its frame (two locals and the caller's first argument slot).
unsafe fn h_act_4a(p: &mut Pushad, a: *const u32) {
    unsafe {
        let m = arg(a, 0) as usize;
        let (ecx, edx, eax) = (rd(m), rd(m + 4), rd(m + 8));
        *(a as *mut u32) = ecx;
        wd(slot(a, 2), edx);
        wd(slot(a, 1), eax);
        wd(0x252A67C, q32(ecx));
        wd(0x252A680, q32(edx));
        wd(0x252A684, q32(eax));
        p.eax = eax;
        p.ecx = ecx;
        p.edx = edx;
    }
}
fn rg_act_4a(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x252A67C, 12));
}

// 0x5e7f10: a one-byte command in the packet sets mode flags.
unsafe fn h_act_1a(p: &mut Pushad, a: *const u32) {
    unsafe {
        let m = arg(a, 0) as usize;
        let al = rb(m);
        p.ecx = m as u32;
        p.eax = (p.eax & !0xFF) | al as u32;
        match al {
            5 => wb(0x252773E, 1),
            9 => wb(0x252771D, al),
            1 => {
                wb(0x252771D, al);
                let eax = rb(m + 1) as i8 as i32 as u32;
                wd(0xE97D04, eax);
                p.eax = eax;
                if eax == 2 {
                    let v = rdu(m + 2);
                    wd(0x140A5E4, v);
                    p.eax = v;
                } else {
                    let v = rdu(m + 2);
                    wd(0x2527820, v);
                    p.ecx = v;
                }
            }
            7 | 8 => wb(0x252771D, al),
            _ => {}
        }
    }
}
fn rg_act_1a(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0x252771D, 1));
    v.push((0x252773E, 1));
    v.push((0xE97D04, 4));
    v.push((0x140A5E4, 4));
    v.push((0x2527820, 4));
}

// 0x609790: a signed byte in -1..=2 is stored.
unsafe fn h_act_48(p: &mut Pushad, a: *const u32) {
    unsafe {
        let eax = rb(arg(a, 0) as usize) as i8 as i32;
        p.eax = eax as u32;
        if (-1..=2).contains(&eax) {
            wd(0xE97B68, eax as u32);
        }
    }
}
fn rg_act_48(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    v.push((0xE97B68, 4));
}

// 0x5bbd20
unsafe fn h_act_46(p: &mut Pushad, _a: *const u32) {
    unsafe {
        let e = rd(0x1408E88) as usize;
        p.eax = e as u32;
        if e == 0 {
            return;
        }
        let o = rd(e + 0x10) as usize;
        p.eax = o as u32;
        if o != 0 {
            wb(o + 0x1D, 1);
        }
    }
}
fn rg_act_46(_p: &Pushad, _a: *const u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let e = rd(0x1408E88) as usize;
        if e != 0 {
            let o = rd(e + 0x10) as usize;
            if o != 0 {
                v.push((o + 0x1D, 1));
            }
        }
    }
}

// 0x7faf30: walks the connection's list looking for a matching id and does nothing with the result
// (reads only; kept so that a bad list faults the same way). Leaves EAX / ECX / EDX as the walk does.
unsafe fn h_recv_no_connect(p: &mut Pushad, a: *const u32) {
    unsafe {
        let s = rd(0x2528C14) as usize;
        p.eax = s as u32;
        if s == 0 {
            return;
        }
        if rb(0x2528B56) != 0 && rb(0x2528B2B) == 0 {
            return;
        }
        if rb(s + 0x3F8) == 0 {
            return;
        }
        let c = rd(arg(a, 1) as usize + 4) as usize;
        p.eax = c as u32;
        if c == 0 {
            return;
        }
        let sess = rd(c + 0x1AC) as usize;
        let dl = rb(arg(a, 0) as usize);
        p.ecx = rd(sess + 0x400);
        p.edx = (p.edx & !0xFF) | dl as u32;
        let ecx = p.ecx as usize;
        let mut eax = ecx;
        p.eax = eax as u32;
        if eax == 0 {
            return;
        }
        loop {
            if rb(eax + 0x1B0) == dl {
                break;
            }
            eax = rd(eax + 0x22CC) as usize;
            p.eax = eax as u32;
            if eax == ecx || eax == 0 {
                break;
            }
        }
    }
}

// 0x677bb0: (data) with the local player on the stack, popped into ECX after.
unsafe fn h_act_1d(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let pl = rd(0x21703D4);
        s.eax = arg(a, 0);
        cx(0x66FB80, &mut s, &fr(a, &[pl]));
        s.ecx = pl;
        *p = s;
    }
}

// 0x677bd0: ECX = the local player, the stack argument is ECX with its low byte replaced by "byte != 0".
unsafe fn h_act_1e(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let b = (rb(arg(a, 0) as usize) != 0) as u32;
        let pushed = (p.ecx & !0xFF) | b;
        s.eax = arg(a, 0);
        s.ecx = rd(0x21703D4);
        cx(0x66FC30, &mut s, &fr(a, &[pushed]));
        s.ecx = pushed;
        *p = s;
    }
}

// 0x566b70 / 0x566c00 / 0x566bd0 / 0x566c30: forward to the callee only when the mode (0xE9E3F8) is 1 or 2,
// with the object 0xE9E3F4 as an argument.
unsafe fn h_act_15(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let eax = rd(0xE9E3F8);
        s.eax = eax;
        if eax == 1 || eax == 2 {
            cx_p1(0x5644B0, &mut s, &fr(a, &[0xE9E3F4]));
        }
        *p = s;
    }
}
unsafe fn h_act_18(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let ecx = rd(arg(a, 0) as usize);
        let eax = rd(0xE9E3F8);
        s.ecx = ecx;
        s.eax = eax;
        if eax == 1 || eax == 2 {
            cx_p2(0x5631B0, &mut s, &fr(a, &[0xE9E3F4, ecx]));
        }
        *p = s;
    }
}
unsafe fn h_act_17(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let m = arg(a, 0) as usize;
        let edx = rd(m);
        let eax = rb(m + 4) as i8 as i32 as u32;
        let ecx = rd16(m + 5) as u32;
        let esi = rd(0xE9E3F8);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.esi = esi;
        if esi == 1 || esi == 2 {
            cx_p3(0x5632D0, &mut s, &fr(a, &[0xE9E3F4, edx, ecx, p.esi]));
        }
        s.esi = p.esi;
        *p = s;
    }
}
unsafe fn h_act_19(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        let eax = rd(0xE9E3F8);
        s.eax = eax;
        if eax == 1 || eax == 2 {
            s.esi = 0xE9E3F4;
            cx(0x5642A0, &mut s, &fr(a, &[p.esi]));
            s.esi = p.esi;
        }
        *p = s;
    }
}

// 0x5c2600: ESI = the first dword of the data.
unsafe fn h_act_41(p: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = *p;
        s.eax = arg(a, 0);
        s.esi = rd(s.eax as usize);
        cx(0x5C2560, &mut s, &fr(a, &[p.esi]));
        s.esi = p.esi;
        *p = s;
    }
}
// --- the table ---------------------------------------------------------------------------------------------

use Mode::{Count, Pure, Snap};

ports! {
    [gc_death, "gameplay_constants_parse_death_and_busted", 0x54D150, B_GC_DEATH, X_GC_DEATH, Snap, false, Some(rg_gc_death), h_gc_death, 0],
    [gc_fat_bones, "gameplay_constants_parse_fat_bones", 0x54D1F0, B_GC_FAT_BONES, X_GC_FAT_BONES, Snap, false, Some(rg_gc_fat_bones), h_gc_fat_bones, 0],
    [gc_drunk, "gameplay_constants_parse_drunk_sway", 0x54D270, B_GC_DRUNK, X_GC_DRUNK, Snap, false, Some(rg_gc_drunk), h_gc_drunk, 0],
    [gc_hit_car, "gameplay_constants_parse_hit_by_car", 0x54D2E0, B_GC_HIT_CAR, X_GC_HIT_CAR, Snap, false, Some(rg_gc_hit_car), h_gc_hit_car, 0],
    [gc_veh_coll, "gameplay_constants_parse_vehicle_collision", 0x54D3A0, B_GC_VEH_COLL, X_GC_VEH_COLL, Snap, false, Some(rg_gc_veh_coll), h_gc_veh_coll, 0],
    [gc_fire, "gameplay_constants_parse_fire", 0x54D420, B_GC_FIRE, X_GC_FIRE, Snap, false, Some(rg_gc_fire), h_gc_fire, 0],
    [gc_sticky, "gameplay_constants_parse_sticky_fire", 0x54D5F0, B_GC_STICKY, X_GC_STICKY, Count, true, None, h_gc_sticky, 0],
    [gc_pimp_ai, "gameplay_constants_parse_ho_pimp_ai", 0x54D8D0, B_GC_PIMP_AI, X_GC_PIMP_AI, Snap, false, Some(rg_gc_pimp_ai), h_gc_pimp_ai, 0],
    [gc_throwing, "gameplay_constants_parse_throwing_constants", 0x54DB20, B_GC_THROWING, X_GC_THROWING, Snap, false, Some(rg_gc_throwing), h_gc_throwing, 0],
    [gc_item_drops, "gameplay_constants_parse_item_drops", 0x54DBA0, B_GC_ITEM_DROPS, X_GC_ITEM_DROPS, Snap, false, Some(rg_gc_item_drops), h_gc_item_drops, 4],
    [gc_shader_lod, "gameplay_constants_parse_shader_lod", 0x54DF50, B_GC_SHADER_LOD, X_GC_SHADER_LOD, Snap, false, Some(rg_gc_shader_lod), h_gc_shader_lod, 0],
    [gc_shadows, "gameplay_constants_parse_shadows", 0x54DFB0, B_GC_SHADOWS, X_GC_SHADOWS, Snap, false, Some(rg_gc_shadows), h_gc_shadows, 0],
    [gc_ctrlpat, "gameplay_constants_parse_controlpatterns", 0x54E140, B_GC_CTRLPAT, X_GC_CTRLPAT, Snap, false, Some(rg_gc_ctrlpat), h_gc_ctrlpat, 0],
    [gc_taunt, "gameplay_constants_parse_tauntreactions", 0x54E1A0, B_GC_TAUNT, X_GC_TAUNT, Snap, false, Some(rg_gc_taunt), h_gc_taunt, 0],
    [gc_fine_coll, "gameplay_constants_parse_human_fine_collision", 0x54E210, B_GC_FINE_COLL, X_GC_FINE_COLL, Snap, false, Some(rg_gc_fine_coll), h_gc_fine_coll, 0],
    [gc_flame, "gameplay_constants_parse_flamethrower", 0x54F430, B_GC_FLAME, X_GC_FLAME, Snap, false, Some(rg_gc_flame), h_gc_flame, 0],
    [gc_veh_inter, "gameplay_constants_parse_vehicle_interaction", 0x54F6D0, B_GC_VEH_INTER, X_GC_VEH_INTER, Snap, false, Some(rg_gc_veh_inter), h_gc_veh_inter, 0],
    [gc_controls, "gameplay_constants_parse_controls", 0x54F730, B_GC_CONTROLS, X_GC_CONTROLS, Snap, false, Some(rg_gc_controls), h_gc_controls, 0],
    [gc_stungun, "gameplay_constants_parse_stungun", 0x54F780, B_GC_STUNGUN, X_GC_STUNGUN, Snap, false, Some(rg_gc_stungun), h_gc_stungun, 0],
    [gc_rocket, "gameplay_constants_parse_rocket_accuracy", 0x54F7E0, B_GC_ROCKET, X_GC_ROCKET, Snap, false, Some(rg_gc_rocket), h_gc_rocket, 0],
    [gc_activity, "gameplay_constants_parse_activity_constants", 0x54F850, B_GC_ACTIVITY, X_GC_ACTIVITY, Snap, false, Some(rg_gc_activity), h_gc_activity, 0],
    [gc_kneecap, "gameplay_constants_parse_kneecapper", 0x54F9A0, B_GC_KNEECAP, X_GC_KNEECAP, Snap, false, Some(rg_gc_kneecap), h_gc_kneecap, 4],
    [gc_evade, "gameplay_constants_parse_vehicle_evade_ai", 0x54FC20, B_GC_EVADE, X_GC_EVADE, Snap, false, Some(rg_gc_evade), h_gc_evade, 0],
    [gc_pepper, "gameplay_constants_parse_pepperspray", 0x54FEC0, B_GC_PEPPER, X_GC_PEPPER, Snap, false, Some(rg_gc_pepper), h_gc_pepper, 0],
    [gc_prop_throw, "gameplay_constants_parse_wieldable_prop_throw", 0x5501A0, B_GC_PROP_THROW, X_GC_PROP_THROW, Snap, false, Some(rg_gc_prop_throw), h_gc_prop_throw, 0],
    [gc_watercraft, "gameplay_constants_parse_watercraft_params", 0x550240, B_GC_WATERCRAFT, X_GC_WATERCRAFT, Snap, false, Some(rg_gc_watercraft), h_gc_watercraft, 0],
    [gc_siren, "gameplay_constants_parse_siren_whoop_params", 0x5502D0, B_GC_SIREN, X_GC_SIREN, Snap, false, Some(rg_gc_siren), h_gc_siren, 0],
    [gc_garage, "gameplay_constants_parse_garage", 0x550370, B_GC_GARAGE, X_GC_GARAGE, Snap, false, Some(rg_gc_garage), h_gc_garage, 0],
    [gc_fall, "gameplay_constants_parse_fall_damage", 0x5503C0, B_GC_FALL, X_GC_FALL, Snap, false, Some(rg_gc_fall), h_gc_fall, 0],
    [cam_stationary, "camera_free_read_stationary_times", 0x496110, B_CAM_STATIONARY, X_CAM_STATIONARY, Snap, false, Some(rg_cam_stationary), h_cam_stationary, 0],
    [cam_crouch, "camera_free_parse_crouch_y_offset", 0x496150, B_CAM_CROUCH, X_CAM_CROUCH, Snap, false, Some(rg_cam_crouch), h_cam_crouch, 0],
    [cam_swing, "camera_free_read_default_swing_rate", 0x4961A0, B_CAM_SWING, X_CAM_SWING, Snap, false, Some(rg_cam_swing), h_cam_swing, 0],
    [cam_aggr, "camera_free_read_default_aggression", 0x4961C0, B_CAM_AGGR, X_CAM_AGGR, Snap, false, Some(rg_cam_aggr), h_cam_aggr, 0],
    [cam_pitch, "camera_free_parse_pitch_reset_time", 0x4961E0, B_CAM_PITCH, X_CAM_PITCH, Snap, false, Some(rg_cam_pitch), h_cam_pitch, 0],
    [gs04c, "gamesave_s04_districts_save_coop", 0x6924F0, B_GS04C, X_GS04C, Snap, false, Some(rg_gs04c), h_gs04c, 0],
    [gs09c_save, "gamesave_s09_activity_levels_save_coop", 0x692B00, B_GS09C_SAVE, X_GS09C_SAVE, Count, true, None, h_gs09_save, 0],
    [gs09_save, "gamesave_s09_activity_levels_save", 0x692B40, B_GS09_SAVE, X_GS09_SAVE, Count, true, None, h_gs09_save, 0],
    [gs10_save, "gamesave_s10_script_triggers_save", 0x692B80, B_GS10_SAVE, X_GS10_SAVE, Count, true, None, h_gs10_save, 8],
    [gs11c_save, "gamesave_s11_cribs_save_coop", 0x692C60, B_GS11C_SAVE, X_GS11C_SAVE, Count, true, None, h_gs11c_save, 0],
    [gs00_save, "gamesave_s00_timers_save", 0x692E80, B_GS00_SAVE, X_GS00_SAVE, Count, true, None, h_gs00_save, 0],
    [gs14_save, "gamesave_s14_playlists_save", 0x692EA0, B_GS14_SAVE, X_GS14_SAVE, Count, true, None, h_gs14_save, 0],
    [gs15_save, "gamesave_s15_cutscene_seen_save", 0x692EC0, B_GS15_SAVE, X_GS15_SAVE, Count, true, None, h_gs15_save, 12],
    [gs16_save, "gamesave_s16_persistent_objects_save", 0x692F00, B_GS16_SAVE, X_GS16_SAVE, Count, true, None, h_gs16_save, 12],
    [gs17_save, "gamesave_s17_rewards_save", 0x692F30, B_GS17_SAVE, X_GS17_SAVE, Snap, false, Some(rg_gs17_save), h_gs17_save, 0],
    [gs18_save, "gamesave_s18_activity_instances_save", 0x692FB0, B_GS18_SAVE, X_GS18_SAVE, Count, true, None, h_gs18_save, 0],
    [gs20_save, "gamesave_s20_cheats_save", 0x693450, B_GS20_SAVE, X_GS20_SAVE, Count, true, None, h_gs20_save, 0],
    [gs12_save, "gamesave_s12_tutorials_save", 0x693470, B_GS12_SAVE, X_GS12_SAVE, Snap, false, Some(rg_gs12_save), h_gs12_save, 0],
    [gs01_save, "gamesave_s01_world_grid_save", 0x693570, B_GS01_SAVE, X_GS01_SAVE, Count, true, None, h_gs01_save, 0],
    [gs05_save, "gamesave_s05_options_save", 0x6935C0, B_GS05_SAVE, X_GS05_SAVE, Count, true, None, h_gs05_save, 0],
    [gs23_save, "gamesave_s23_audio_a_save", 0x693720, B_GS23_SAVE, X_GS23_SAVE, Count, true, None, h_gs23_save, 0],
    [gs24_save, "gamesave_s24_audio_b_save", 0x693730, B_GS24_SAVE, X_GS24_SAVE, Count, true, None, h_gs24_save, 0],
    [gs25_save, "gamesave_s25_audio_c_save", 0x693740, B_GS25_SAVE, X_GS25_SAVE, Count, true, None, h_gs25_save, 0],
    [gs26_save, "gamesave_s26_music_store_save", 0x693760, B_GS26_SAVE, X_GS26_SAVE, Count, true, None, h_gs26_save, 4],
    [gs27_save, "gamesave_s27_store_discounts_save", 0x6937A0, B_GS27_SAVE, X_GS27_SAVE, Count, true, None, h_gs27_save, 0],
    [gs07_load, "gamesave_s07_garage_load", 0x694470, B_GS07_LOAD, X_GS07_LOAD, Count, true, None, h_gs07_load, 0],
    [gs22_load, "gamesave_s22_wiretap_load", 0x6944C0, B_GS22_LOAD, X_GS22_LOAD, Count, true, None, h_gs22_load, 0],
    [gs09c_load, "gamesave_s09_activity_levels_load_coop", 0x6947C0, B_GS09C_LOAD, X_GS09C_LOAD, Count, true, None, h_gs09_load, 0],
    [gs09_load, "gamesave_s09_activity_levels_load", 0x6947E0, B_GS09_LOAD, X_GS09_LOAD, Count, true, None, h_gs09_load, 0],
    [gs10_load, "gamesave_s10_script_triggers_load", 0x694800, B_GS10_LOAD, X_GS10_LOAD, Count, true, None, h_gs10_load, 0],
    [gs11c_load, "gamesave_s11_cribs_load_coop", 0x6948B0, B_GS11C_LOAD, X_GS11C_LOAD, Count, true, None, h_gs11c_load, 0],
    [gs00_load, "gamesave_s00_timers_load", 0x694AB0, B_GS00_LOAD, X_GS00_LOAD, Count, true, None, h_gs00_load, 0],
    [gs16_load, "gamesave_s16_persistent_objects_load", 0x694AE0, B_GS16_LOAD, X_GS16_LOAD, Count, true, None, h_gs16_load, 12],
    [gs15_load, "gamesave_s15_cutscene_seen_load", 0x694B10, B_GS15_LOAD, X_GS15_LOAD, Count, true, None, h_gs15_load, 12],
    [gs17_load, "gamesave_s17_rewards_load", 0x694B50, B_GS17_LOAD, X_GS17_LOAD, Count, true, None, h_gs17_load, 0],
    [gs18_load, "gamesave_s18_activity_instances_load", 0x694BD0, B_GS18_LOAD, X_GS18_LOAD, Count, true, None, h_gs18_load, 0],
    [gs20_load, "gamesave_s20_cheats_load", 0x694E20, B_GS20_LOAD, X_GS20_LOAD, Count, true, None, h_gs20_load, 0],
    [gs12_load, "gamesave_s12_tutorials_load", 0x694E40, B_GS12_LOAD, X_GS12_LOAD, Snap, false, Some(rg_gs12_load), h_gs12_load, 0],
    [gs13_load, "gamesave_s13_debug_display_options_load", 0x694E70, B_GS13_LOAD, X_GS13_LOAD, Snap, false, Some(rg_gs13_load), h_gs13_load, 0],
    [gs01c_load, "gamesave_s01_world_grid_load_coop", 0x694EE0, B_GS01C_LOAD, X_GS01C_LOAD, Snap, false, Some(rg_gs01c_load), h_gs01c_load, 0],
    [gs01_load, "gamesave_s01_world_grid_load", 0x694F30, B_GS01_LOAD, X_GS01_LOAD, Count, true, None, h_gs01_load, 0],
    [gs05_load, "gamesave_s05_options_load", 0x694F80, B_GS05_LOAD, X_GS05_LOAD, Count, true, None, h_gs05_load, 0],
    [gs23_load, "gamesave_s23_audio_a_load", 0x6950C0, B_GS23_LOAD, X_GS23_LOAD, Count, true, None, h_gs23_load, 0],
    [gs24_load, "gamesave_s24_audio_b_load", 0x6950D0, B_GS24_LOAD, X_GS24_LOAD, Count, true, None, h_gs24_load, 0],
    [gs25_load, "gamesave_s25_audio_c_load", 0x6950E0, B_GS25_LOAD, X_GS25_LOAD, Count, true, None, h_gs25_load, 0],
    [gs26_load, "gamesave_s26_music_store_load", 0x6950F0, B_GS26_LOAD, X_GS26_LOAD, Count, true, None, h_gs26_load, 0],
    [gs27_load, "gamesave_s27_store_discounts_load", 0x695130, B_GS27_LOAD, X_GS27_LOAD, Count, true, None, h_gs27_load, 0],
    [recv_qos, "net_recv_qos", 0x80B5B0, B_RECV_QOS, X_RECV_QOS, Snap, false, Some(rg_recv_qos), h_recv_qos, 0],
    [recv_bw_info, "net_recv_bw_info", 0x80B5E0, B_RECV_BW_INFO, X_RECV_BW_INFO, Count, true, None, h_recv_bw_info, 0],
    [recv_next_srv, "net_recv_next_srv", 0x810A20, B_RECV_NEXT_SRV, X_RECV_NEXT_SRV, Snap, false, Some(rg_recv_next_srv), h_recv_next_srv, 0],
    [recv_mod_sess, "net_recv_modified_session", 0x810BD0, B_RECV_MOD_SESS, X_RECV_MOD_SESS, Snap, false, Some(rg_recv_modified_session), h_recv_modified_session, 0],
    [recv_notor, "net_recv_notor_active", 0x924D30, B_RECV_NOTOR, X_RECV_NOTOR, Snap, false, Some(rg_recv_notor_active), h_recv_notor_active, 0],
    [misc_pkt_05, "misc_packet_sub_05", 0x548580, B_MISC_PKT_05, X_MISC_PKT_05, Count, true, None, h_misc_pkt_05, 0],
    [misc_pkt_17, "misc_packet_sub_17", 0xA2C670, B_MISC_PKT_17, X_MISC_PKT_17, Snap, false, Some(rg_misc_pkt_17), h_misc_pkt_17, 0],
    [hud_pkt_16, "hud_pkt_sub_16", 0xA2DDB0, B_HUD_PKT_16, X_HUD_PKT_16, Count, true, None, h_hud_pkt_16, 0],
    [recv_acks, "net_recv_acks", 0x7FB150, B_RECV_ACKS, X_RECV_ACKS, Count, true, None, h_recv_acks, 0],
    [recv_sessstart, "net_recv_sessionstart", 0x835930, B_RECV_SESSSTART, X_RECV_SESSSTART, Count, true, None, h_recv_sessionstart, 0],
    [act_3f, "act_packet_sub_3f", 0x5AF3F0, B_ACT_3F, X_ACT_3F, Snap, false, Some(rg_act_3f), h_act_3f, 0],
    [act_3e, "act_packet_sub_3e", 0x5AF390, B_ACT_3E, X_ACT_3E, Snap, false, Some(rg_act_3e), h_act_3e, 0],
    [act_44, "act_packet_sub_44", 0x5BBBA0, B_ACT_44, X_ACT_44, Snap, false, Some(rg_act_44), h_act_44, 0],
    [act_4a, "act_packet_sub_4a", 0x5F7E30, B_ACT_4A, X_ACT_4A, Snap, false, Some(rg_act_4a), h_act_4a, 8],
    [act_1a, "act_packet_sub_1a", 0x5E7F10, B_ACT_1A, X_ACT_1A, Snap, false, Some(rg_act_1a), h_act_1a, 0],
    [act_48, "act_packet_sub_48", 0x609790, B_ACT_48, X_ACT_48, Snap, false, Some(rg_act_48), h_act_48, 0],
    [act_46, "act_packet_sub_46", 0x5BBD20, B_ACT_46, X_ACT_46, Snap, false, Some(rg_act_46), h_act_46, 0],
    [recv_no_connect, "net_recv_no_connect", 0x7FAF30, B_RECV_NO_CONNECT, X_RECV_NO_CONNECT, Pure, false, None, h_recv_no_connect, 0],
    [act_1d, "act_packet_sub_1d", 0x677BB0, B_ACT_1D, X_ACT_1D, Count, true, None, h_act_1d, 0],
    [act_1e, "act_packet_sub_1e", 0x677BD0, B_ACT_1E, X_ACT_1E, Count, true, None, h_act_1e, 0],
    [act_15, "act_packet_sub_15", 0x566B70, B_ACT_15, X_ACT_15, Count, true, None, h_act_15, 0],
    [act_18, "act_packet_sub_18", 0x566C00, B_ACT_18, X_ACT_18, Count, true, None, h_act_18, 0],
    [act_17, "act_packet_sub_17", 0x566BD0, B_ACT_17, X_ACT_17, Count, true, None, h_act_17, 0],
    [act_19, "act_packet_sub_19", 0x566C30, B_ACT_19, X_ACT_19, Count, true, None, h_act_19, 0],
    [act_41, "act_packet_sub_41", 0x5C2600, B_ACT_41, X_ACT_41, Count, true, None, h_act_41, 0],
}

// =================================================================================================
// Offline differential test
// =================================================================================================
//
// The original exe is mapped at its own addresses (0x400000) inside the test process, so the ORIGINAL
// bytes run in place, with their absolute data references, against the real thunk (and the Rust
// body behind it). Every callee of the function under test is replaced in the mapped image by a
// logging stub (a `push id; call common` trampoline): the stub records the registers and stack
// arguments it was called with and returns a value chosen by the test. The original and the thunk
// run on the same memory (snapshot restored in between), and the test compares: the callee logs
// (so every register and stack argument handed to a callee is checked), every watched byte of
// memory, EAX where it is a result, and the registers the original preserves.
//
// Run each test in its own process:
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_small_data -- --ignored --test-threads=1 --nocapture
// (CASES=n changes the number of random cases per function, default 1500.)

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE};

    // --- the mapped image ---------------------------------------------------------------------------
    //
    // The test process must own the game's address range 0x400000..0x35AD000, but the system maps files into
    // it while the process starts. So the test re-runs itself as a child process created suspended, and the
    // parent reserves the range in the child before it runs (`child_gate`). In the child `map_image` only
    // copies the exe's sections in.

    const IMAGE_BASE: usize = 0x40_0000;
    const IMAGE_SIZE: usize = 0x31A_D000;

    mod sys {
        use std::ffi::c_void;
        #[repr(C)]
        pub struct StartupInfo {
            pub cb: u32,
            pub reserved: *mut u16,
            pub desktop: *mut u16,
            pub title: *mut u16,
            pub x: u32,
            pub y: u32,
            pub xs: u32,
            pub ys: u32,
            pub xc: u32,
            pub yc: u32,
            pub fill: u32,
            pub flags: u32,
            pub show: u16,
            pub cb2: u16,
            pub reserved2: *mut u8,
            pub stdin: *mut c_void,
            pub stdout: *mut c_void,
            pub stderr: *mut c_void,
        }
        #[repr(C)]
        pub struct ProcInfo {
            pub process: *mut c_void,
            pub thread: *mut c_void,
            pub pid: u32,
            pub tid: u32,
        }
        #[link(name = "kernel32")]
        #[allow(clashing_extern_declarations)]
        unsafe extern "system" {
            pub fn CreateProcessW(
                app: *const u16,
                cmd: *mut u16,
                pa: *const c_void,
                ta: *const c_void,
                inherit: i32,
                flags: u32,
                env: *const c_void,
                dir: *const u16,
                si: *const StartupInfo,
                pi: *mut ProcInfo,
            ) -> i32;
            pub fn VirtualAllocEx(p: *mut c_void, a: *const c_void, n: usize, t: u32, prot: u32) -> *mut c_void;
            pub fn ResumeThread(t: *mut c_void) -> u32;
            pub fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
            pub fn GetExitCodeProcess(h: *mut c_void, code: *mut u32) -> i32;
            pub fn CloseHandle(h: *mut c_void) -> i32;
            pub fn TerminateProcess(h: *mut c_void, code: u32) -> i32;
        }
    }

    /// True in the child (go on); in the parent, run `name` in a child with the range reserved and report.
    fn child_gate(name: &str) -> bool {
        if std::env::var("SD_CHILD").is_ok() {
            return true;
        }
        let exe = std::env::current_exe().unwrap();
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let app = wide(exe.to_str().unwrap());
        let mut cmd = wide(&format!("\"{}\" --ignored --test-threads=1 --nocapture --exact port_small_data::tests::{name}", exe.to_str().unwrap()));
        unsafe { std::env::set_var("SD_CHILD", "1") };
        let si = sys::StartupInfo {
            cb: std::mem::size_of::<sys::StartupInfo>() as u32,
            reserved: std::ptr::null_mut(),
            desktop: std::ptr::null_mut(),
            title: std::ptr::null_mut(),
            x: 0,
            y: 0,
            xs: 0,
            ys: 0,
            xc: 0,
            yc: 0,
            fill: 0,
            flags: 0,
            show: 0,
            cb2: 0,
            reserved2: std::ptr::null_mut(),
            stdin: std::ptr::null_mut(),
            stdout: std::ptr::null_mut(),
            stderr: std::ptr::null_mut(),
        };
        let mut pi = sys::ProcInfo { process: std::ptr::null_mut(), thread: std::ptr::null_mut(), pid: 0, tid: 0 };
        unsafe {
            let ok = sys::CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 1, 4 /* CREATE_SUSPENDED */, std::ptr::null(), std::ptr::null(), &si, &mut pi);
            assert!(ok != 0, "CreateProcessW failed");
            let p = sys::VirtualAllocEx(pi.process, IMAGE_BASE as *const _, IMAGE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
            if p as usize != IMAGE_BASE {
                sys::TerminateProcess(pi.process, 99);
                panic!("cannot reserve the game's address range in the child");
            }
            sys::ResumeThread(pi.thread);
            sys::WaitForSingleObject(pi.process, 0xFFFF_FFFF);
            let mut code = 0u32;
            sys::GetExitCodeProcess(pi.process, &mut code);
            sys::CloseHandle(pi.thread);
            sys::CloseHandle(pi.process);
            assert_eq!(code, 0, "the child test process failed (exit code {code:#x})");
        }
        false
    }

    fn map_image() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            assert!(std::env::var("SD_CHILD").is_ok(), "must run through child_gate");
            let path = "F:/Saints Row 2 Decomp/bin/SR2_pc.unpacked.exe";
            let exe = std::fs::read(path).expect("read the unpacked exe");
            let u32at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
            let u16at = |o: usize| u16::from_le_bytes(exe[o..o + 2].try_into().unwrap()) as usize;
            let pe = u32at(0x3C);
            let nsec = u16at(pe + 6);
            let opt = u16at(pe + 20);
            for i in 0..nsec {
                let o = pe + 24 + opt + i * 40;
                let (vsz, va, rsz, raw) = (u32at(o + 8), u32at(o + 12), u32at(o + 16), u32at(o + 20));
                let n = rsz.min(vsz);
                unsafe { std::ptr::copy_nonoverlapping(exe[raw..raw + n].as_ptr(), (IMAGE_BASE + va) as *mut u8, n) };
            }
        });
    }

    // --- logging stubs -------------------------------------------------------------------------------

    #[derive(Clone, Default, PartialEq, Debug)]
    struct Entry {
        id: u32,
        eax: u32,
        ecx: u32,
        edx: u32,
        esi: u32,
        edi: u32,
        stk: [u32; 6],
        extra: Vec<u32>,
        /// Bits: 0 eax, 1 ecx, 2 edx, 3 esi, 4 edi, 8+i stack argument i (not compared).
        ignore: u32,
    }

    fn norm(e: &Entry) -> Entry {
        let mut n = e.clone();
        let z = |bit: u32, v: &mut u32| {
            if e.ignore & (1 << bit) != 0 {
                *v = 0;
            }
        };
        z(0, &mut n.eax);
        z(1, &mut n.ecx);
        z(2, &mut n.edx);
        z(3, &mut n.esi);
        z(4, &mut n.edi);
        for i in 0..6 {
            z(8 + i as u32, &mut n.stk[i]);
        }
        n
    }

    /// The number of stack arguments each stubbed callee takes (the rest of the logged stack is stale).
    fn nargs_of(addr: usize) -> usize {
        match addr {
            STRCMP => 2,
            PARSE_F | PARSE_I | PARSE_X => 1,
            0x542940 | 0xA8CE10 | 0xA8CEB0 | 0x7AEBD0 | 0x5556F0 | 0x548F00 | 0x688260 | 0x7AED60 | 0x7B08C0 | 0x548590 | 0x66FB80 | 0x66FC30 | 0x5644B0 => 1,
            0x605CA0 | 0x5631B0 => 2,
            0x5632D0 => 3,
            // tail jumps: the callee sees the caller's stack (the two test arguments)
            0x7DECD0 | 0x48B7D0 | 0x491140 | 0x54CCA0 | 0x542A10 | 0x5630D0 | 0x7DED80 | 0x48B870 | 0x491180 | 0x491260 | 0xA7C9D0 | 0x794230 | 0x80C9A0 => 2,
            _ => 0,
        }
    }

    type Cb = fn(&mut Pushad, &mut Entry, u32);
    static LOG: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    static CB: Mutex<Option<Cb>> = Mutex::new(None);
    static NTH: Mutex<[u32; 64]> = Mutex::new([0; 64]);
    static IDS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    /// Test parameters the callbacks read.
    static P: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

    fn p(i: usize) -> u32 {
        P[i].load(Ordering::Relaxed)
    }
    fn set_p(i: usize, v: u32) {
        P[i].store(v, Ordering::Relaxed)
    }

    extern "C" fn record(r: &mut Pushad, stk: *const u32) {
        let (id, nth);
        let mut e = Entry::default();
        unsafe {
            id = *stk.add(1);
            let mut n = NTH.lock().unwrap();
            nth = n[id as usize];
            n[id as usize] += 1;
            e.id = id;
            e.eax = r.eax;
            e.ecx = r.ecx;
            e.edx = r.edx;
            e.esi = r.esi;
            e.edi = r.edi;
            let na = nargs_of(IDS.lock().unwrap()[id as usize]);
            for i in 0..na {
                e.stk[i] = *stk.add(3 + i);
            }
        }
        let cb = *CB.lock().unwrap();
        if let Some(cb) = cb {
            cb(r, &mut e, nth);
        }
        LOG.lock().unwrap().push(e);
    }

    #[unsafe(naked)]
    extern "C" fn stub_common() {
        std::arch::naked_asm!(
            "pushad",
            "mov eax, esp",
            "lea ecx, [esp + 32]",
            "push ecx",
            "push eax",
            "call {f}",
            "add esp, 8",
            "popad",
            "ret",
            f = sym record,
        )
    }

    /// Replace the function at `addr` in the mapped image by a logging stub (`ret_n` bytes popped).
    fn stub(addr: usize, ret_n: u16) {
        map_image();
        let id = {
            let mut ids = IDS.lock().unwrap();
            match ids.iter().position(|&a| a == addr) {
                Some(i) => i as u32,
                None => {
                    ids.push(addr);
                    (ids.len() - 1) as u32
                }
            }
        };
        let mut code = vec![0x68u8];
        code.extend_from_slice(&id.to_le_bytes());
        code.push(0xE8);
        let rel = (stub_common as *const () as usize as i32).wrapping_sub((addr + 10) as i32);
        code.extend_from_slice(&rel.to_le_bytes());
        code.extend_from_slice(&[0x83, 0xC4, 0x04]);
        if ret_n == 0 {
            code.push(0xC3);
        } else {
            code.extend_from_slice(&[0xC2, ret_n as u8, (ret_n >> 8) as u8]);
        }
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), addr as *mut u8, code.len()) };
    }

    fn stub_id(addr: usize) -> u32 {
        IDS.lock().unwrap().iter().position(|&a| a == addr).unwrap() as u32
    }

    // --- the call harness --------------------------------------------------------------------------------

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct TB {
        f: u32,
        eax: u32,
        ecx: u32,
        edx: u32,
        ebx: u32,
        esi: u32,
        edi: u32,
        ebp: u32,
        args: u32,
        nargs: u32,
        o_eax: u32,
        o_ecx: u32,
        o_edx: u32,
        o_ebx: u32,
        o_esi: u32,
        o_edi: u32,
        o_ebp: u32,
        esp: u32,
    }

    static SLOTS: Mutex<()> = Mutex::new(());
    static mut SLOT: [u32; 2] = [0; 2];

    #[derive(Clone, Copy, Debug)]
    struct In {
        eax: u32,
        ecx: u32,
        edx: u32,
        esi: u32,
        edi: u32,
    }

    const EBX_IN: u32 = 0xB1B1_B1B1;
    const EBP_IN: u32 = 0xE9E9_E9E9;

    unsafe fn call_t(f: usize, r: In, args: &[u32]) -> TB {
        let _g = SLOTS.lock().unwrap();
        let mut b = TB {
            f: f as u32,
            eax: r.eax,
            ecx: r.ecx,
            edx: r.edx,
            ebx: EBX_IN,
            esi: r.esi,
            edi: r.edi,
            ebp: EBP_IN,
            args: args.as_ptr() as u32,
            nargs: args.len() as u32,
            ..Default::default()
        };
        unsafe {
            let slot = std::ptr::addr_of_mut!(SLOT);
            (*slot)[0] = &mut b as *mut TB as u32;
            (*slot)[1] = f as u32;
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 68], esp",
                "mov [{s}], eax",
                "mov ebx, eax",
                "fninit",
                "push 0x27F",
                "fldcw word ptr [esp]",
                "add esp, 4",
                "mov ecx, [ebx + 36]",
                "mov esi, [ebx + 32]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 20]", "mov edi, [ebx + 24]",
                "mov ebp, [ebx + 28]", "mov eax, [ebx + 4]", "mov ebx, [ebx + 16]",
                "call dword ptr [{s} + 4]",
                "push eax",
                "mov eax, dword ptr [{s}]",
                "mov [eax + 44], ecx", "mov [eax + 48], edx", "mov [eax + 52], ebx", "mov [eax + 56], esi",
                "mov [eax + 60], edi", "mov [eax + 64], ebp",
                "pop dword ptr [eax + 40]",
                "mov esp, [eax + 68]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                s = sym SLOT,
                inout("eax") &mut b as *mut TB => _,
                clobber_abi("C"),
            );
        }
        b
    }

    // --- random numbers ------------------------------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.u32() % n
        }
        /// Any float bits: NaNs (quiet and signalling), infinities, zeros, denormals included.
        fn any_f(&mut self) -> u32 {
            match self.below(12) {
                0 => 0x7F80_0000,
                1 => 0xFF80_0000,
                2 => 0x7FC0_0000 | self.u32() & 0x3F_FFFF,
                3 => 0x7F80_0000 | (self.u32() & 0x7F_FFFF) | 1,
                4 => 0,
                5 => 0x8000_0000,
                6 => self.u32() & 0x807F_FFFF,
                7 => ((self.below(21) as f32 - 10.0) / 4.0).to_bits(),
                _ => self.u32(),
            }
        }
    }

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(1500)
    }

    unsafe fn snap(watch: &[(usize, usize)]) -> Vec<Vec<u8>> {
        watch.iter().map(|&r| unsafe { bytes_of(r) }).collect()
    }

    unsafe fn restore(watch: &[(usize, usize)], s: &[Vec<u8>]) {
        for (r, b) in watch.iter().zip(s) {
            unsafe { put_back(*r, b) };
        }
    }

    static COPIES: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

    /// The relocated copy `install` would make of port `pid`'s original (cached).
    fn reloc_copy(pid: usize) -> usize {
        let mut c = COPIES.lock().unwrap();
        if let Some(&(_, copy)) = c.iter().find(|e| e.0 == pid) {
            return copy;
        }
        let port = &PORTS[pid];
        let copy = unsafe { relocate(port.addr, port.body, port.ext) }.unwrap_or_else(|e| panic!("relocate {}: {e:?}", port.name));
        c.push((pid, copy));
        copy
    }

    /// One case: the original at `orig`, the thunk of port `pid`.
    unsafe fn run_case(pid: usize, r: In, args: &[u32], watch: &[(usize, usize)], cb: Cb) -> Result<(), String> {
        let port = &PORTS[pid];
        let (orig, thunk) = (port.addr, port.thunk as usize);
        unsafe {
            *CB.lock().unwrap() = Some(cb);
            let before = snap(watch);
            *NTH.lock().unwrap() = [0; 64];
            LOG.lock().unwrap().clear();
            let o = call_t(orig, r, args);
            let o_mem = snap(watch);
            let o_log: Vec<Entry> = LOG.lock().unwrap().iter().map(norm).collect();
            restore(watch, &before);
            *NTH.lock().unwrap() = [0; 64];
            LOG.lock().unwrap().clear();
            PORTS[pid].original.store(0, Ordering::Relaxed);
            let t = call_t(thunk, r, args);
            let t_mem = snap(watch);
            let t_log: Vec<Entry> = LOG.lock().unwrap().iter().map(norm).collect();
            restore(watch, &before);
            // registers
            if t.o_ebx != EBX_IN || t.o_esi != r.esi || t.o_edi != r.edi || t.o_ebp != EBP_IN || t.o_ecx != r.ecx || t.o_edx != r.edx {
                return Err(format!("the thunk changed a register: ebx {:#x} esi {:#x} edi {:#x} ebp {:#x} ecx {:#x} edx {:#x}", t.o_ebx, t.o_esi, t.o_edi, t.o_ebp, t.o_ecx, t.o_edx));
            }
            if o.o_ebx != EBX_IN || o.o_esi != r.esi || o.o_edi != r.edi || o.o_ebp != EBP_IN {
                return Err("the ORIGINAL changed a callee-saved register".into());
            }
            if o.o_eax == r.eax && t.o_eax != r.eax {
                return Err(format!("the thunk changed EAX ({:#x} -> {:#x}), which the original preserved", r.eax, t.o_eax));
            }
            if port.eax_out && o.o_eax != t.o_eax {
                return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
            }
            if o_log != t_log {
                let n = o_log.iter().zip(&t_log).position(|(a, b)| a != b);
                return Err(format!("callee logs differ ({} vs {} calls), first difference at {:?}: original {:x?} Rust {:x?}", o_log.len(), t_log.len(), n, n.map(|i| &o_log[i]), n.map(|i| &t_log[i])));
            }
            for (i, (a, b)) in o_mem.iter().zip(&t_mem).enumerate() {
                if a != b {
                    let at = a.iter().zip(b).position(|(x, y)| x != y).unwrap();
                    return Err(format!("memory range {i} ({:#x}+{at:#x}): original {:#x} Rust {:#x}", watch[i].0, a[at], b[at]));
                }
            }
            // the in-game shadow path itself: it must agree (no mismatch) and leave the original's result
            if port.mode != Mode::Count {
                port.original.store(reloc_copy(pid), Ordering::Relaxed);
                port.calls.store(0, Ordering::Relaxed);
                let m0 = port.mismatches.load(Ordering::Relaxed);
                *NTH.lock().unwrap() = [0; 64];
                LOG.lock().unwrap().clear();
                let s = call_t(thunk, r, args);
                let s_mem = snap(watch);
                restore(watch, &before);
                port.original.store(0, Ordering::Relaxed);
                if port.mismatches.load(Ordering::Relaxed) != m0 {
                    return Err("the in-game shadow reported a mismatch".into());
                }
                if s_mem != o_mem || (port.eax_out && s.o_eax != o.o_eax) {
                    return Err("the shadow path left a different result".into());
                }
            }
        }
        Ok(())
    }

    fn arena(words: usize) -> &'static mut [u32] {
        Box::leak(vec![0u32; words].into_boxed_slice())
    }

    fn pid(name: &str) -> usize {
        PORTS.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no port {name}"))
    }

    fn report(name: &str, n: u32, bad: u32) {
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }

    /// The default callback: a pseudo-random result per call.
    fn cb_default(r: &mut Pushad, e: &mut Entry, nth: u32) {
        r.eax = (0x9E37_79B9u32).wrapping_mul(e.id + 1).wrapping_add(nth.wrapping_mul(0x85EB_CA6B)) ^ 0x1234_5678;
    }

    /// Fill `[a, a+len)` of game memory with random bytes.
    unsafe fn fill(a: usize, len: usize, rng: &mut Rng) {
        for i in 0..len / 4 {
            unsafe { wd(a + i * 4, rng.u32()) };
        }
    }

    unsafe fn fill_f(a: usize, len: usize, rng: &mut Rng) {
        for i in 0..len / 4 {
            unsafe { wd(a + i * 4, rng.any_f()) };
        }
    }

    // --- gameplay_constants_parse_* / camera_free_* ---------------------------------------------------------------

    const PARSERS: [usize; 3] = [PARSE_F, PARSE_I, PARSE_X];

    /// Parser stubs: a result per call; with P[0] set they also write a value through ESI (a stack
    /// local the original and Rust place at different addresses).
    fn cb_gc(r: &mut Pushad, e: &mut Entry, nth: u32) {
        if e.id == stub_id(STRCMP) {
            r.eax = if nth == p(1) { 0 } else { 1 + nth };
            return;
        }
        r.eax = 0x5000 + nth * 7;
        if p(0) != 0 {
            let k = {
                let mut n = NTH.lock().unwrap();
                n[62] += 1;
                n[62] - 1
            };
            let v = ((k % 3) + 1).wrapping_mul(0x0123_4567) ^ p(2);
            unsafe { wd(r.esi as usize, v) };
            e.ignore |= 1 << 3;
            e.extra.push(v);
        }
    }

    fn gc_test(names: &[&str], node_in_edi: bool, with_find: bool) {
        map_image();
        stub(STRCMP, 0);
        stub(0x54D580, 0);
        for f in PARSERS {
            stub(f, 0);
        }
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for name in names {
            if let Ok(only) = std::env::var("ONLY") {
                if only != *name {
                    continue;
                }
            }
            let id = pid(name);
            let local = matches!(*name, "gameplay_constants_parse_item_drops" | "gameplay_constants_parse_kneecapper");
            set_p(0, local as u32);
            let ar = arena(256);
            let base = ar.as_ptr() as usize;
            let mut ranges = Vec::new();
            match PORTS[id].ranges {
                Some(f) => f(&Pushad { xmm: [[0; 4]; 8], edi: 0, esi: 0, ebp: 0, esp: 0, ebx: 0, edx: 0, ecx: 0, eax: 0 }, std::ptr::null(), &mut ranges),
                None => ranges.push((0x27974C4, 16)), // sticky_fire: Count mode, no range function
            }
            let mut watch = ranges.clone();
            watch.push((base, 256 * 4));
            let (n, mut bad) = (cases(), 0);
            for i in 0..n {
                // a node with m children; the compare stub says "equal" on the k-th
                let m = rng.below(6);
                for w in ar.iter_mut() {
                    *w = rng.u32();
                }
                let node = base as u32 + 0x20;
                let mut next = 0u32;
                for c in (0..m).rev() {
                    let ch = base as u32 + 0x100 + c * 0x20;
                    unsafe {
                        wd(ch as usize, base as u32 + 0x300 + c * 4);
                        wd(ch as usize + 4, next);
                    }
                    next = ch;
                }
                unsafe { wd(node as usize + 8, next) };
                let node_ptr = if rng.below(8) == 0 { 0 } else { node };
                set_p(1, if rng.below(4) == 0 { 99 } else { rng.below(m + 1) });
                set_p(2, rng.u32());
                for &(a, l) in &ranges {
                    unsafe { fill_f(a, l, &mut rng) };
                }
                let reg = |rng: &mut Rng| rng.u32();
                let mut r = In { eax: reg(&mut rng), ecx: reg(&mut rng), edx: reg(&mut rng), esi: reg(&mut rng), edi: reg(&mut rng) };
                if with_find {
                    r.eax = node_ptr;
                } else if node_in_edi {
                    r.edi = node_ptr.wrapping_add(rng.below(3) * 0x1111_0000);
                } else {
                    r.eax = node_ptr.wrapping_add(rng.below(3) * 0x1111_0000);
                }
                if let Err(m) = unsafe { run_case(id, r, &[], &watch, cb_gc) } {
                    bad += 1;
                    if bad <= 3 {
                        eprintln!("{name} case {i}: {m}");
                    }
                }
            }
            report(name, n, bad);
        }
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn gc_templates_match_original() {
        if !child_gate("gc_templates_match_original") {
            return;
        }
        let names: Vec<&str> = PORTS.iter().map(|p| p.name).filter(|n| n.starts_with("gameplay_constants_")).collect();
        gc_test(&names, false, true);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn camera_free_matches_original() {
        if !child_gate("camera_free_matches_original") {
            return;
        }
        gc_test(&["camera_free_read_stationary_times", "camera_free_parse_crouch_y_offset", "camera_free_parse_pitch_reset_time"], true, false);
        gc_test(&["camera_free_read_default_swing_rate", "camera_free_read_default_aggression"], false, false);
    }

    // --- gamesave sections ------------------------------------------------------------------------------------

    struct Ctx {
        base: usize,
        aux: usize,
        watch: Vec<(usize, usize)>,
        args: Vec<u32>,
        r: In,
    }

    fn try_id(addr: usize) -> Option<u32> {
        IDS.lock().unwrap().iter().position(|&a| a == addr).map(|i| i as u32)
    }

    /// The gamesave callback: a result per call; P[3] = register whose pointee (P[4] words, the last
    /// masked with P[5]) is logged instead of the (stack) pointer; P[6] = more ignored bits; P[1] =
    /// what 0x69E1C0 returns.
    fn cb_gs(r: &mut Pushad, e: &mut Entry, nth: u32) {
        cb_default(r, e, nth);
        let (reg, n, last) = (p(3), p(4), p(5));
        if reg != 0 {
            let ptr = match reg {
                1 => e.ecx,
                2 => e.esi,
                _ => e.edi,
            } as usize;
            for i in 0..n as usize {
                let mut v = unsafe { rd(ptr + i * 4) };
                if i + 1 == n as usize {
                    v &= last;
                }
                e.extra.push(v);
            }
            e.ignore |= 1 << match reg {
                1 => 1,
                2 => 3,
                _ => 4,
            };
        }
        e.ignore |= p(6);
        if try_id(0x69E1C0) == Some(e.id) {
            r.eax = p(1);
        }
    }

    const NOSTUB: &[(usize, u16)] = &[];

    fn gs_test(name: &str, stubs: &[(usize, u16)], deref: (u32, u32, u32, u32), setup: fn(&mut Rng, &mut Ctx)) {
        if let Ok(only) = std::env::var("ONLY") {
            if only != name {
                return;
            }
        }
        map_image();
        for &(a, n) in stubs {
            stub(a, n);
        }
        let id = pid(name);
        set_p(1, 0);
        set_p(3, deref.0);
        set_p(4, deref.1);
        set_p(5, deref.2);
        set_p(6, deref.3);
        let words = 0x3B000 / 4;
        let ar = arena(words);
        let aux = arena(0x4000);
        let (base, auxp) = (ar.as_ptr() as usize, aux.as_ptr() as usize);
        let mut rng = Rng(0x0DDB_A11C_AFE5_1234 ^ id as u64);
        for w in ar.iter_mut() {
            *w = rng.u32();
        }
        let (n, mut bad) = (cases(), 0);
        for i in 0..n {
            for _ in 0..600 {
                ar[rng.below(words as u32) as usize] = rng.u32();
            }
            let reg = |rng: &mut Rng| rng.u32();
            let mut c = Ctx {
                base,
                aux: auxp,
                watch: vec![(base, words * 4), (auxp, 0x10000)],
                args: vec![base as u32, rng.u32()],
                r: In { eax: reg(&mut rng), ecx: reg(&mut rng), edx: reg(&mut rng), esi: reg(&mut rng), edi: reg(&mut rng) },
            };
            setup(&mut rng, &mut c);
            if let Err(m) = unsafe { run_case(id, c.r, &c.args, &c.watch, cb_gs) } {
                bad += 1;
                if bad <= 3 {
                    eprintln!("{name} case {i}: {m}");
                }
            }
        }
        report(name, n, bad);
    }

    fn su_none(_: &mut Rng, _: &mut Ctx) {}

    fn su_gs04c(rng: &mut Rng, c: &mut Ctx) {
        let n = if rng.below(10) == 0 { rng.u32() } else { rng.below(14) };
        let n = if rng.below(12) == 0 { 0xFFFF_FFF0 } else { n.min(13) };
        unsafe {
            wd(0x27ABD00, n);
            fill(0x27ABD04, 12 * 16, rng);
            wd(0x2526C94, c.aux as u32);
            fill(c.aux, 0x4000, rng);
        }
        c.watch.push((0x27ABD00, 4 + 12 * 16));
        c.watch.push((0x2526C94, 4));
    }

    fn su_gs09_save(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            wd(0x2527C88, if rng.below(2) == 0 { 0 } else { c.aux as u32 });
            wd(c.aux, rng.u32());
            wd(0x33D630C, rng.u32());
        }
        c.watch.push((0x2527C88, 4));
        c.watch.push((0x33D630C, 4));
    }

    fn su_gs11c_save(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            wd(0x27AC238, rng.u32());
            wd(0x27ABFDC, if rng.below(4) == 0 { 0 } else { rng.below(7) });
            fill(0x27ABFE0, 0x28 * 8, rng);
            wd(
                0x27973CC,
                match rng.below(8) {
                    0 => 2_000_000_000,
                    1 => 2_000_000_001,
                    2 => (-2_000_000_000i32) as u32,
                    3 => (-2_000_000_001i32) as u32,
                    4 => 0,
                    _ => rng.u32(),
                },
            );
        }
        c.watch.push((0x27AC238, 4));
        c.watch.push((0x27ABFDC, 4));
        c.watch.push((0x27973CC, 4));
    }

    fn su_gs15(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            wd(0x145A4AC, [0xFF, 0x100, 0x101, 0xFFFF_FFFF, 0, 0x7FFF_FFFF, 0x8000_0000][rng.below(7) as usize] ^ if rng.below(5) == 0 { rng.below(3) } else { 0 });
        }
        if rng.below(3) == 0 {
            c.args[1] &= !0xFF;
        }
    }

    fn su_gs17_save(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            wd(0x140B204, rng.u32());
            for a in [0x140B210usize, 0x140B220, 0x140B230, 0x140B240] {
                wd(a, rng.any_f());
            }
        }
        let _ = c;
    }

    fn su_gs17_load(rng: &mut Rng, c: &mut Ctx) {
        for i in 0..4 {
            unsafe { wd(c.base + 0x3A0C8 + i * 4, rng.any_f()) };
        }
        if rng.below(3) == 0 {
            c.args[1] &= !0xFF;
        }
        c.watch.push((0x140B204, 4));
    }

    fn su_gs12(rng: &mut Rng, c: &mut Ctx) {
        unsafe { fill(0x27DAEA4, 0x10C0 + 0x20, rng) };
        c.watch.push((0x27DAEA4, 0x10C0 + 0x20));
    }

    fn su_gs13(_: &mut Rng, c: &mut Ctx) {
        c.watch.push((0x27ABA94, 0x27ABB40 - 0x27ABA94));
    }

    fn su_gs01c_load(rng: &mut Rng, c: &mut Ctx) {
        let n = match rng.below(8) {
            0 => 0u32,
            1 => 0xFFFF_FFFF,
            _ => rng.below(41),
        };
        unsafe { wd(c.base + 0x38504, n) };
        c.watch.push((0x27AC23C, 8 + 41 * 8));
    }

    fn su_gs07_load(rng: &mut Rng, c: &mut Ctx) {
        let n = if rng.below(10) == 0 { 0xFFFF_FFFF } else { rng.below(7) };
        unsafe {
            wd(c.base + 0x175D8, n);
            for i in 0..8 {
                wd(c.base + 0x175DC + i * 0x434, if rng.below(2) == 0 { 0 } else { rng.u32() });
            }
        }
        c.watch.push((0x1409998, 4));
    }

    fn su_gs22(_: &mut Rng, c: &mut Ctx) {
        c.watch.push((0xFA2398, 8));
    }

    fn su_gs09_load(rng: &mut Rng, c: &mut Ctx) {
        let ptr = if rng.below(3) == 0 { 0 } else { c.aux as u32 };
        set_p(1, ptr);
        unsafe { wd(c.aux + 0x3C, if rng.below(2) == 0 { 0 } else { rng.u32() }) };
        c.watch.push((0x2527C88, 4));
    }

    fn su_gs10_load(rng: &mut Rng, c: &mut Ctx) {
        let m = rng.below(5);
        unsafe {
            fill(c.aux, 0x1000, rng);
            for i in 0..m {
                let nd = c.aux + i as usize * 0x200;
                wd(nd + 0xC0, 1 + rng.below(4));
                let next = if i + 1 < m {
                    nd as u32 + 0x200
                } else if rng.below(4) == 0 {
                    0
                } else {
                    c.aux as u32
                };
                wd(nd + 0x188, next);
                wb(nd + 0x110, rng.u32() as u8);
            }
            wd(0x2F8E88C, if m == 0 { 0 } else { c.aux as u32 });
            let cnt = if rng.below(10) == 0 { 0xFFFF_FFFF } else { rng.below(6) };
            wd(c.base + 0x37AD0, cnt);
            for i in 0..6 {
                wd(c.base + 0x37AD4 + i * 8, 1 + rng.below(5));
                wd(c.base + 0x37AD8 + i * 8, rng.u32());
            }
        }
        c.watch.push((0x2F8E88C, 4));
    }

    fn su_gs11c_load(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            (c.base + 0x382A4).wr16(if rng.below(3) == 0 { 0 } else { rng.u32() as u16 });
            (c.base + 0x382A6).wr16(if rng.below(4) == 0 { 0 } else { rng.below(7) as u16 });
        }
        c.watch.push((0x27AC238, 4));
        c.watch.push((0x27ABFDC, 4));
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn gamesave_sections_match_original() {
        if !child_gate("gamesave_sections_match_original") {
            return;
        }
        let c0 = (0, 0, 0xFFFF_FFFF, 0);
        // a 3-dword local {ptr, n, byte}: ESI points at it
        let loc_esi = (2, 3, 0xFF, 0);
        // the same through ECX, which is also stack argument 0
        let loc_ecx = (1, 3, 0xFF, 1 << 8);
        let t: &[(&str, &[(usize, u16)], (u32, u32, u32, u32), fn(&mut Rng, &mut Ctx))] = &[
            ("gamesave_s04_districts_save_coop", NOSTUB, c0, su_gs04c),
            ("gamesave_s09_activity_levels_save_coop", &[(0x696B50, 0)], c0, su_gs09_save),
            ("gamesave_s09_activity_levels_save", &[(0x696B50, 0)], c0, su_gs09_save),
            ("gamesave_s10_script_triggers_save", &[(0xA6EA00, 0)], (3, 2, 0xFFFF_FFFF, 0), su_none),
            ("gamesave_s11_cribs_save_coop", &[(0x692D00, 0)], c0, su_gs11c_save),
            ("gamesave_s00_timers_save", &[(0x542940, 0)], c0, su_none),
            ("gamesave_s14_playlists_save", &[(0x6AB5C0, 0)], c0, su_none),
            ("gamesave_s15_cutscene_seen_save", &[(0x6C3AD0, 0)], loc_esi, su_gs15),
            ("gamesave_s16_persistent_objects_save", &[(0xA8CE10, 0)], loc_ecx, su_none),
            ("gamesave_s17_rewards_save", NOSTUB, c0, su_gs17_save),
            ("gamesave_s18_activity_instances_save", &[(0x563040, 0)], c0, su_none),
            ("gamesave_s20_cheats_save", &[(0x688220, 0)], c0, su_none),
            ("gamesave_s12_tutorials_save", NOSTUB, c0, su_gs12),
            ("gamesave_s01_world_grid_save", &[(0xA7C910, 0)], c0, su_none),
            ("gamesave_s05_options_save", &[(0x7DECD0, 0)], c0, su_none),
            ("gamesave_s23_audio_a_save", &[(0x48B7D0, 0)], c0, su_none),
            ("gamesave_s24_audio_b_save", &[(0x491140, 0)], c0, su_none),
            ("gamesave_s25_audio_c_save", &[(0x491200, 0)], c0, su_none),
            ("gamesave_s26_music_store_save", &[(0x7AEBD0, 0)], (1, 1, 0xFFFF_FFFF, 0), su_none),
            ("gamesave_s27_store_discounts_save", &[(0x7B07F0, 0)], c0, su_none),
            ("gamesave_s07_garage_load", &[(0x5558F0, 0), (0x5556F0, 0)], c0, su_gs07_load),
            ("gamesave_s22_wiretap_load", &[(0x54CCA0, 0)], c0, su_gs22),
            ("gamesave_s09_activity_levels_load_coop", &[(0x69E1C0, 0)], c0, su_gs09_load),
            ("gamesave_s09_activity_levels_load", &[(0x69E1C0, 0)], c0, su_gs09_load),
            ("gamesave_s10_script_triggers_load", &[(0x7A4DC0, 0), (0xA6BD60, 0), (0xA6BCE0, 0)], c0, su_gs10_load),
            ("gamesave_s11_cribs_load_coop", &[(0x692D00, 0), (0x548F00, 0)], c0, su_gs11c_load),
            ("gamesave_s00_timers_load", &[(0x542A10, 0)], c0, su_none),
            ("gamesave_s16_persistent_objects_load", &[(0xA8CEB0, 0)], loc_ecx, su_none),
            ("gamesave_s15_cutscene_seen_load", &[(0x6C3B30, 0)], loc_esi, su_gs15),
            ("gamesave_s17_rewards_load", &[(0x605CA0, 0)], c0, su_gs17_load),
            ("gamesave_s18_activity_instances_load", &[(0x5630D0, 0)], c0, su_none),
            ("gamesave_s20_cheats_load", &[(0x688260, 0)], c0, su_none),
            ("gamesave_s12_tutorials_load", NOSTUB, c0, su_gs12),
            ("gamesave_s13_debug_display_options_load", NOSTUB, c0, su_gs13),
            ("gamesave_s01_world_grid_load_coop", NOSTUB, c0, su_gs01c_load),
            ("gamesave_s01_world_grid_load", &[(0xA7C9D0, 0)], c0, su_none),
            ("gamesave_s05_options_load", &[(0x7DED80, 0)], c0, su_none),
            ("gamesave_s23_audio_a_load", &[(0x48B870, 0)], c0, su_none),
            ("gamesave_s24_audio_b_load", &[(0x491180, 0)], c0, su_none),
            ("gamesave_s25_audio_c_load", &[(0x491260, 0)], c0, su_none),
            ("gamesave_s26_music_store_load", &[(0x7AED60, 0)], c0, su_none),
            ("gamesave_s27_store_discounts_load", &[(0x7B08C0, 0)], c0, su_none),
        ];
        for (name, stubs, deref, setup) in t {
            gs_test(name, stubs, *deref, *setup);
        }
    }

    // --- packet handlers ----------------------------------------------------------------------------------------

    /// A connection `conn` (aux + 0x1000, null with 1/8) reachable from argument 1 (aux + 0x100, +4), whose
    /// session is aux + 0x8000. The whole aux block is random first.
    unsafe fn conn_world(rng: &mut Rng, c: &mut Ctx, null_ok: bool) -> (usize, usize) {
        let a = c.aux;
        unsafe {
            fill(a, 0x10000, rng);
            let conn = a + 0x1000;
            let sess = a + 0x8000;
            c.args[1] = (a + 0x100) as u32;
            wd(a + 0x104, if null_ok && rng.below(8) == 0 { 0 } else { conn as u32 });
            wd(conn + 0x1AC, sess as u32);
            (conn, sess)
        }
    }

    fn su_recv_qos(rng: &mut Rng, c: &mut Ctx) {
        unsafe { conn_world(rng, c, false) };
    }

    fn su_recv_next_srv(rng: &mut Rng, c: &mut Ctx) {
        unsafe { conn_world(rng, c, true) };
    }

    fn su_recv_bw_info(rng: &mut Rng, c: &mut Ctx) {
        let (conn, _) = unsafe { conn_world(rng, c, true) };
        unsafe { wd(conn + 0x1A8, if rng.below(3) == 0 { 0 } else { rng.u32() }) };
    }

    fn su_recv_mod_sess(rng: &mut Rng, c: &mut Ctx) {
        let (_, sess) = unsafe { conn_world(rng, c, false) };
        unsafe {
            wd(sess + 0x2C, if rng.below(4) == 0 { rng.below(4) } else { 2 });
            wd(sess + 0xBF8, rng.below(20));
        }
    }

    fn su_recv_notor(rng: &mut Rng, c: &mut Ctx) {
        unsafe {
            fill(c.aux, 0x10000, rng);
            for i in 0..14usize {
                wd(0xE8_8B68 + i * 4, (c.aux + i * 0x400) as u32);
            }
        }
        c.watch.push((0xE8_8B68, 56));
    }

    fn su_recv_acks(rng: &mut Rng, c: &mut Ctx) {
        unsafe { conn_world(rng, c, true) };
        c.args[0] = (c.aux + 0x2000) as u32;
        unsafe { wb(c.aux + 0x2000, if rng.below(5) == 0 { 0 } else { 1 + rng.below(4) as u8 }) };
    }

    fn su_misc17(_: &mut Rng, c: &mut Ctx) {
        c.watch.push((0x27AC238, 4));
    }

    fn su_hud16(_: &mut Rng, c: &mut Ctx) {
        c.watch.push((0x252_849C, 8));
    }

    fn su_act_obj(rng: &mut Rng, c: &mut Ctx) {
        let a = c.aux;
        unsafe {
            fill(a, 0x10000, rng);
            wd(0x1408E88, if rng.below(6) == 0 { 0 } else { (a + 0x100) as u32 });
            wd(a + 0x110, if rng.below(5) == 0 { 0 } else { (a + 0x1000) as u32 });
        }
        c.watch.push((0x1408E88, 4));
    }

    /// The local-player record: [0x27B3C60] -> world (+0x3C = 1, +0x90 -> record).
    fn su_act_world(rng: &mut Rng, c: &mut Ctx) {
        let a = c.aux;
        unsafe {
            fill(a, 0x10000, rng);
            wd(0x27B3C60, if rng.below(6) == 0 { 0 } else { (a + 0x200) as u32 });
            wd(a + 0x200 + 0x3C, if rng.below(5) == 0 { rng.u32() } else { 1 });
            wd(a + 0x290, if rng.below(5) == 0 { 0 } else { (a + 0x1000) as u32 });
            c.args[0] = (a + 0x3000) as u32;
            wd(
                a + 0x3000,
                match rng.below(5) {
                    0 => 0xFFFF_FFFF,
                    1 => 0,
                    2 => 20,
                    _ => rng.below(14),
                },
            );
            // 0x2C0 compared against the first message dword
            let v = rd(a + 0x3000);
            wd(a + 0x1000 + 0x2C0, if rng.below(3) == 0 { v } else { rng.u32() });
        }
        c.watch.push((0x27B3C60, 4));
    }

    fn su_act_4a(rng: &mut Rng, c: &mut Ctx) {
        for i in 0..3 {
            unsafe { wd(c.base + i * 4, rng.any_f()) };
        }
        c.args[0] = c.base as u32;
    }

    fn su_act_1a(rng: &mut Rng, c: &mut Ctx) {
        let cmd = [5u8, 9, 1, 7, 8, 1, 1, rng.u32() as u8][rng.below(8) as usize];
        unsafe {
            wb(c.base, cmd);
            wb(c.base + 1, [2u8, 2, 0, 1, 0xFF, rng.u32() as u8][rng.below(6) as usize]);
        }
        c.args[0] = c.base as u32;
    }

    fn su_act_48(rng: &mut Rng, c: &mut Ctx) {
        unsafe { wb(c.base, [0xFDu8, 0xFE, 0xFF, 0, 1, 2, 3, 4, rng.u32() as u8][rng.below(9) as usize]) };
        c.args[0] = c.base as u32;
    }

    fn su_no_connect(rng: &mut Rng, c: &mut Ctx) {
        let (_, sess) = unsafe { conn_world(rng, c, true) };
        let a = c.aux;
        unsafe {
            wd(0x2528C14, if rng.below(6) == 0 { 0 } else { (a + 0x400) as u32 });
            wb(0x2528B56, rng.below(2) as u8);
            wb(0x2528B2B, rng.below(2) as u8);
            wb(a + 0x400 + 0x3F8, rng.below(5) as u8);
            let m = rng.below(4);
            for i in 0..m as usize {
                let nd = a + 0x9000 + i * 0x2400;
                wb(nd + 0x1B0, 1 + rng.below(4) as u8);
                let next = if i + 1 < m as usize {
                    nd + 0x2400
                } else if rng.below(3) == 0 {
                    0
                } else {
                    a + 0x9000
                };
                wd(nd + 0x22CC, next as u32);
            }
            wd(sess + 0x400, if m == 0 { 0 } else { (a + 0x9000) as u32 });
            wb(c.base, 1 + rng.below(5) as u8);
        }
        c.args[0] = c.base as u32;
    }

    fn su_act_1d(rng: &mut Rng, _c: &mut Ctx) {
        unsafe { wd(0x21703D4, rng.u32()) };
    }

    fn su_act_mode(rng: &mut Rng, _c: &mut Ctx) {
        unsafe { wd(0xE9E3F8, [0u32, 1, 2, 3, 0xFFFF_FFFF, 1, 2, rng.u32()][rng.below(8) as usize]) };
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn packet_handlers_match_original() {
        if !child_gate("packet_handlers_match_original") {
            return;
        }
        let c0 = (0, 0, 0xFFFF_FFFF, 0);
        let t: &[(&str, &[(usize, u16)], (u32, u32, u32, u32), fn(&mut Rng, &mut Ctx))] = &[
            ("net_recv_qos", NOSTUB, c0, su_recv_qos),
            ("net_recv_bw_info", &[(0x816B50, 0)], c0, su_recv_bw_info),
            ("net_recv_next_srv", NOSTUB, c0, su_recv_next_srv),
            ("net_recv_modified_session", NOSTUB, c0, su_recv_mod_sess),
            ("net_recv_notor_active", NOSTUB, c0, su_recv_notor),
            ("misc_packet_sub_05", &[(0x548590, 0)], c0, su_none),
            ("misc_packet_sub_17", NOSTUB, c0, su_misc17),
            ("hud_pkt_sub_16", &[(0x794230, 0)], c0, su_hud16),
            ("net_recv_acks", &[(0x7F8270, 0)], c0, su_recv_acks),
            ("net_recv_sessionstart", &[(0x80C9A0, 0)], c0, su_recv_qos),
            ("act_packet_sub_3f", NOSTUB, c0, su_act_obj),
            ("act_packet_sub_3e", NOSTUB, c0, su_act_world),
            ("act_packet_sub_44", NOSTUB, c0, su_act_world),
            ("act_packet_sub_4a", NOSTUB, c0, su_act_4a),
            ("act_packet_sub_1a", NOSTUB, c0, su_act_1a),
            ("act_packet_sub_48", NOSTUB, c0, su_act_48),
            ("act_packet_sub_46", NOSTUB, c0, su_act_obj),
            ("net_recv_no_connect", NOSTUB, c0, su_no_connect),
            ("act_packet_sub_1d", &[(0x66FB80, 0)], c0, su_act_1d),
            ("act_packet_sub_1e", &[(0x66FC30, 0)], c0, su_act_1d),
            ("act_packet_sub_15", &[(0x5644B0, 4)], c0, su_act_mode),
            ("act_packet_sub_18", &[(0x5631B0, 8)], c0, su_act_mode),
            ("act_packet_sub_17", &[(0x5632D0, 12)], c0, su_act_mode),
            ("act_packet_sub_19", &[(0x5642A0, 0)], c0, su_act_mode),
            ("act_packet_sub_41", &[(0x5C2560, 0)], c0, su_none),
        ];
        for (name, stubs, deref, setup) in t {
            gs_test(name, stubs, *deref, *setup);
        }
    }
}
