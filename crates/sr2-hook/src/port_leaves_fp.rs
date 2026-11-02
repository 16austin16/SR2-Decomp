//! Port module: small floating-point leaf functions of the exe (one entry, one `ret`, no call):
//! Lua number getters that push one or two numbers on the Lua stack, widget size getters,
//! constant-returning virtual stubs, the vint element vec2 / colour / source accessors, the
//! polynomial vint ease functions, and a few vector / matrix helpers.
//!
//! Every port goes through a `pushad` thunk (`ret` or `ret N` as the original). The Rust body
//! gets the register block and the stack arguments, performs the memory writes through [`W`],
//! sets the registers the original leaves changed (scratch ones included) and returns the
//! value the original leaves in ST0 (`0.0` when it leaves nothing). The thunk then loads that
//! `f64` with `fld qword`, so a result the original rounds to f32 (`fstp dword` / `fld dword`)
//! is passed as the f32 widened to f64, and an unrounded one (53-bit double) as the exact f64.
//!
//! The game's x87 runs at 53-bit precision, so every x87 expression is evaluated in f64 and
//! rounded with `as f32` at each `fstp dword` / `fst dword`; `fld dword` is `f32 as f64`
//! (a signalling NaN becomes quiet, as on the FPU). Constants are read from the exe at run time.
//!
//! Branching ports (ids 84 and up): the vint property getters / setters with two or four `ret`s
//! and the `vint_ease_*` functions with a branch or a CRT call. Each body is a one-to-one
//! translation of the original (`loop { match pc { .. } }` over its basic blocks, a virtual
//! stack for the esp / ebp slots, an `f[]` array for the x87 stack, the integer flags in `Fl`).
//! The CRT calls go to the original code by address (`crt_call1` for the cdecl sin / cos / sqrt
//! wrappers, `crt_pow` for `crt_pow_x87` with its two x87 operands) and bring back EAX / ECX /
//! EDX as the CRT leaves them; the shadow ignores ECX for those (it holds a stack address). The
//! call to `vint_ease_out_bounce` from the other bounce functions goes to its Rust body. The x87
//! compare idioms keep the original polarity: `fnstsw ax` is rebuilt by `sw_ax` and the
//! `test ah, N` / `jp` branches read it through `Fl`.
//!
//! Shadow check: for the first `SHADOW_CALLS` calls of each port a relocated copy of the
//! original runs on the same registers and stack arguments. The Rust body first runs once with
//! an undo log to learn which bytes it writes, the log is rolled back, then the original runs,
//! its registers, ST0 and written bytes are saved, the bytes are restored, the Rust body runs for
//! real and everything is compared. On a mismatch the original's results are kept.

#![allow(clippy::too_many_arguments)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

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

/// Offsets used by the `call_full_*` asm: f 0, regs 4..32, args 32..64, saved esp 64, ST0 68.
#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    args: [u32; 8],
    esp: u32,
    st0: [u32; 2],
}

// --- memory writes ----------------------------------------------------------------------------------

/// The Rust bodies write through this: really, or really with an undo log (the dry run that finds
/// the written bytes).
struct W {
    log: Option<Vec<(usize, usize, u64)>>,
}

impl W {
    fn real() -> W {
        W { log: None }
    }
    fn dry() -> W {
        W { log: Some(Vec::new()) }
    }
    fn note(&mut self, a: usize, n: usize) {
        if let Some(l) = &mut self.log {
            let mut old = 0u64;
            unsafe { std::ptr::copy_nonoverlapping(a as *const u8, &mut old as *mut u64 as *mut u8, n) };
            l.push((a, n, old));
        }
    }
    fn b(&mut self, a: usize, v: u8) {
        self.note(a, 1);
        unsafe { *(a as *mut u8) = v };
    }
    #[allow(dead_code)] // for the translated bodies that write a word
    fn h(&mut self, a: usize, v: u16) {
        self.note(a, 2);
        unsafe { std::ptr::write_unaligned(a as *mut u16, v) };
    }
    fn d(&mut self, a: usize, v: u32) {
        self.note(a, 4);
        unsafe { std::ptr::write_unaligned(a as *mut u32, v) };
    }
    fn q(&mut self, a: usize, v: u64) {
        self.note(a, 8);
        unsafe { std::ptr::write_unaligned(a as *mut u64, v) };
    }
    /// Undo everything logged, newest first; return the written regions.
    fn undo(self) -> Vec<(usize, usize)> {
        let l = self.log.unwrap_or_default();
        for &(a, n, old) in l.iter().rev() {
            unsafe { std::ptr::copy_nonoverlapping(&old as *const u64 as *const u8, a as *mut u8, n) };
        }
        l.iter().map(|&(a, n, _)| (a, n)).collect()
    }
}

/// A dword read of game memory.
fn rd(a: usize) -> u32 {
    unsafe { std::ptr::read_unaligned(a as *const u32) }
}

/// A byte read of game memory.
fn rb(a: usize) -> u8 {
    unsafe { *(a as *const u8) }
}

/// A qword read (constants, Lua numbers).
fn rq(a: usize) -> u64 {
    unsafe { std::ptr::read_unaligned(a as *const u64) }
}

/// `fld dword` then `fstp dword`: the bits with a signalling NaN made quiet.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// `fld qword` then `fstp qword`.
fn q64(bits: u64) -> u64 {
    if bits & 0x7FF0_0000_0000_0000 == 0x7FF0_0000_0000_0000 && bits & 0x000F_FFFF_FFFF_FFFF != 0 {
        bits | 0x0008_0000_0000_0000
    } else {
        bits
    }
}

/// `fld dword [m]`.
fn ldf(bits: u32) -> f64 {
    f32::from_bits(bits) as f64
}

/// `fstp dword` followed by `fld dword`: the value rounded to f32.
fn r32(x: f64) -> f64 {
    (x as f32) as f64
}

/// The x87 condition codes (in the `ah` bit positions of the status word) after comparing `a`
/// (ST0) with `b`.
fn cc(a: f64, b: f64) -> u32 {
    if a.is_nan() || b.is_nan() {
        0x4500
    } else if a < b {
        0x0100
    } else if a == b {
        0x4000
    } else {
        0
    }
}

/// The low word `fnstsw ax` leaves after a compare: condition codes, the TOP field (the live
/// one lowered by `pushed` values the original keeps on the x87 stack) and the live sticky flags.
fn sw_ax(cc: u32, pushed: u32) -> u32 {
    let live: u32;
    unsafe { std::arch::asm!("fnstsw ax", "movzx eax, ax", out("eax") live, options(nomem, nostack)) };
    let top = ((live >> 11) & 7).wrapping_sub(pushed) & 7;
    cc | (top << 11) | (live & 0xFF)
}

// --- calling the original copy ---------------------------------------------------------------

macro_rules! call_full {
    ($name:ident, $off:literal, $fst:literal) => {
        /// Call `f` with all seven registers from `r` and `nargs` stack dwords (the callee pops a
        /// fixed number of bytes), then capture the seven registers and, for `fstp`, ST0.
        unsafe fn $name(f: usize, r: &mut Regs, args: *const u32, nargs: usize, st: &mut u64) {
            let mut b = Block { f: f as u32, r: *r, args: [0; 8], esp: 0, st0: [0; 2] };
            for i in 0..nargs.min(8) {
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
                    concat!("mov ebx, [esp + ", $off, "]"),
                    "mov [ebx + 4], eax",
                    "mov [ebx + 8], ecx",
                    "mov [ebx + 12], edx",
                    "mov [ebx + 20], esi",
                    "mov [ebx + 24], edi",
                    "mov [ebx + 28], ebp",
                    "pop dword ptr [ebx + 16]",
                    $fst,
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
            *st = (b.st0[1] as u64) << 32 | b.st0[0] as u64;
        }
    };
}

call_full!(cf_0, "40", "nop");
call_full!(cf_0s, "40", "fstp qword ptr [ebx + 68]");
call_full!(cf_4, "36", "nop");
call_full!(cf_4s, "36", "fstp qword ptr [ebx + 68]");
call_full!(cf_8, "32", "nop");
call_full!(cf_8s, "32", "fstp qword ptr [ebx + 68]");

unsafe fn call_orig(pop: u32, st0: bool, f: usize, r: &mut Regs, args: *const u32, nargs: usize, st: &mut u64) {
    unsafe {
        match (pop, st0) {
            (0, false) => cf_0(f, r, args, nargs, st),
            (0, true) => cf_0s(f, r, args, nargs, st),
            (4, false) => cf_4(f, r, args, nargs, st),
            (4, true) => cf_4s(f, r, args, nargs, st),
            (8, false) => cf_8(f, r, args, nargs, st),
            (8, true) => cf_8s(f, r, args, nargs, st),
            _ => unreachable!(),
        }
    }
}

// --- the port table -------------------------------------------------------------------------------

/// A Rust body: sets the registers the original leaves changed (reads the stack arguments from
/// `args`), does the original's memory writes through `W`, and returns what is left in ST0.
type Run = fn(&mut Pushad, *const u32, &mut W) -> f64;

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    /// Stack argument dwords handed to the shadow copy, and the bytes the original pops.
    nargs: usize,
    pop: u32,
    /// The original leaves a value in ST0.
    st0: bool,
    /// Bits of EAX compared by the shadow (the low byte after `fnstsw` holds sticky flags).
    eax_mask: u32,
    run: Run,
    thunk: extern "C" fn(),
    /// Offsets of the `call rel32` instructions inside `body` (for the relocated copy).
    call_offsets: &'static [usize],
    /// Registers the shadow does not compare (bit 1 = ECX, bit 2 = EDX): a CRT call leaves stack
    /// addresses in them, and the callers treat them as clobbered.
    ignore_regs: u32,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn port(
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    nargs: usize,
    pop: u32,
    st0: bool,
    eax_mask: u32,
    run: Run,
    thunk: extern "C" fn(),
) -> Port {
    Port {
        name,
        addr,
        body,
        nargs,
        pop,
        st0,
        eax_mask,
        run,
        thunk,
        call_offsets: &[],
        ignore_regs: 0,
        calls: AtomicU32::new(0),
        mismatches: AtomicU32::new(0),
        original: AtomicUsize::new(0),
    }
}

/// As [`port`], for a body that holds `call rel32` instructions (at `call_offsets`).
const fn port_c(
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    call_offsets: &'static [usize],
    nargs: usize,
    pop: u32,
    st0: bool,
    eax_mask: u32,
    ignore_regs: u32,
    run: Run,
    thunk: extern "C" fn(),
) -> Port {
    let mut p = port(name, addr, body, nargs, pop, st0, eax_mask, run, thunk);
    p.call_offsets = call_offsets;
    p.ignore_regs = ignore_regs;
    p
}

/// A naked thunk: `pushad`, then `entry(id, &mut block, &stack args, &mut st0)`, then (for the
/// ports that return on the x87 stack) `fld qword` of the result, `popad` and the original's
/// `ret` / `ret N`.
macro_rules! thunk {
    ($name:ident, $id:expr, $ret:literal, $fld:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 8",
                "mov eax, esp",
                "lea ecx, [esp + 8]",
                "lea edx, [esp + 44]",
                "push eax",
                "push edx",
                "push ecx",
                "push {id}",
                "call {f}",
                "add esp, 16",
                $fld,
                "add esp, 8",
                "popad",
                $ret,
                id = const $id,
                f = sym entry,
            )
        }
    };
}

/// Count a call; run the Rust body, shadowed against the original while the budget lasts.
extern "C" fn entry(id: usize, r: &mut Pushad, args: *const u32, st: &mut f64) {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_leaves_fp: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    if orig == 0 || n >= SHADOW_CALLS {
        *st = (p.run)(r, args, &mut W::real());
        return;
    }
    shadow(p, n, orig, r, args, st);
}

fn snapshot(regions: &[(usize, usize)]) -> Vec<Vec<u8>> {
    regions.iter().map(|&(a, l)| unsafe { std::slice::from_raw_parts(a as *const u8, l).to_vec() }).collect()
}

fn restore(regions: &[(usize, usize)], saved: &[Vec<u8>]) {
    for (&(a, l), bytes) in regions.iter().zip(saved) {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), a as *mut u8, l) };
    }
}

/// Run the original copy, then the Rust body, from the same state; compare registers, ST0 and
/// the bytes written. On a mismatch keep the original's results.
fn shadow(p: &Port, n: u32, orig: usize, r: &mut Pushad, args: *const u32, st: &mut f64) {
    let inp = Regs::of(r);
    // Dry run of the Rust body: which bytes does it write?
    let regions = {
        let mut probe = *r;
        let mut w = W::dry();
        let _ = (p.run)(&mut probe, args, &mut w);
        w.undo()
    };
    let pre = snapshot(&regions);
    let mut theirs = inp;
    let mut theirs_st = 0u64;
    unsafe { call_orig(p.pop, p.st0, orig, &mut theirs, args, p.nargs, &mut theirs_st) };
    let post_theirs = snapshot(&regions);
    restore(&regions, &pre);
    let mut mine = *r;
    let mine_st = (p.run)(&mut mine, args, &mut W::real());
    let post_mine = snapshot(&regions);
    let ours = Regs::of(&mine);
    let regs_ok = (p.ignore_regs & 2 != 0 || ours.ecx == theirs.ecx)
        && (p.ignore_regs & 4 != 0 || ours.edx == theirs.edx)
        && ours.ebx == theirs.ebx
        && ours.esi == theirs.esi
        && ours.edi == theirs.edi
        && ours.ebp == theirs.ebp
        && (ours.eax ^ theirs.eax) & p.eax_mask == 0;
    let st_ok = !p.st0 || mine_st.to_bits() == theirs_st;
    if !regs_ok || !st_ok || post_mine != post_theirs {
        let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
        if m <= 10 {
            log(&format!(
                "port_leaves_fp: {} mismatch (call {n}): in {inp:x?}: original {theirs:x?} st0 {theirs_st:#x}, Rust {ours:x?} st0 {:#x}, writes equal {}",
                p.name,
                mine_st.to_bits(),
                post_mine == post_theirs
            ));
        }
        restore(&regions, &post_theirs);
        theirs.apply(r);
        *st = f64::from_bits(theirs_st);
    } else {
        *r = mine;
        *st = mine_st;
    }
}


// --- Rust bodies: helpers -----------------------------------------------------------------------------

fn argv(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

fn rdd(a: u32) -> u32 {
    rd(a as usize)
}

fn at(base: u32, off: u32) -> u32 {
    rdd(base.wrapping_add(off))
}

fn ad(base: u32, off: u32) -> usize {
    base.wrapping_add(off) as usize
}

/// Push a Lua number (tag 3) on the Lua state `l`; returns the old top.
fn push_num(w: &mut W, l: u32, v: f64) -> u32 {
    let top = at(l, 8);
    w.d(top as usize, 3);
    w.q(ad(top, 8), v.to_bits());
    w.d(ad(l, 8), top.wrapping_add(16));
    top
}

/// Push a Lua boolean (tag 1) on the Lua state `l`; returns the old top.
fn push_bool(w: &mut W, l: u32, v: u32) -> u32 {
    let top = at(l, 8);
    w.d(top as usize, 1);
    w.d(ad(top, 8), v);
    w.d(ad(l, 8), top.wrapping_add(16));
    top
}

// --- Rust bodies: widgets ---------------------------------------------------------------------------------

/// `widget_tabbar::get_height` (ECX = widget, `ret 4`): (unsigned count at +0x1c) * f32 at +0x28.
/// ECX comes back as the result's bits (the `push ecx` slot is the float spill slot).
fn tabbar_get_height(p: &mut Pushad, _: *const u32, _: &mut W) -> f64 {
    let v = at(p.ecx, 0x1c);
    p.eax = v;
    let mut x = (v as i32) as f64;
    if (v as i32) < 0 {
        x += ldf(rdd(0xe5bfd8));
    }
    let res = r32(x * ldf(at(p.ecx, 0x28)));
    p.ecx = (res as f32).to_bits();
    res
}

/// `pending_menu_widget::get_width` / `get_height` (ECX = widget, `ret 4`): the larger of the
/// f32s at +0x18 / +0x1c (the second on a tie or NaN), then the larger of that and the f32 at `O`
/// (`O` on a tie or NaN). `fnstsw ax` leaves the last compare in AX; ECX is the spill slot.
fn pending_dim<const O: u32>(p: &mut Pushad, _: *const u32, _: &mut W) -> f64 {
    let a = ldf(at(p.ecx, 0x18));
    let b = ldf(at(p.ecx, 0x1c));
    let m = if b < a { a } else { b };
    let m = r32(m);
    let c = ldf(at(p.ecx, O));
    let res = r32(if c < m { m } else { c });
    p.eax = (p.eax & 0xFFFF_0000) | sw_ax(cc(c, m), 1);
    p.ecx = (res as f32).to_bits();
    res
}

// --- Rust bodies: random float ----------------------------------------------------------------------------

/// `rng_float01` (EAX = generator state): the next table value as an unsigned number times a
/// double constant, rounded to f32. ECX / EDX end up as the value (the spill slot is `pop ecx`).
fn rng_float01(p: &mut Pushad, _: *const u32, w: &mut W) -> f64 {
    let g = p.eax;
    let idx = at(g, 0x9c4);
    let v = rdd(g.wrapping_add(idx.wrapping_mul(4)).wrapping_add(0x9c8));
    let n = idx.wrapping_add(1);
    w.d(ad(g, 0x9c4), n);
    if n == 0x800 {
        w.d(ad(g, 0x9c4), 0);
    }
    let mut x = (v as i32) as f64;
    if (v as i32) < 0 {
        x += ldf(rdd(0xe5bfd8));
    }
    let res = r32(x * f64::from_bits(rq(0xe5c030)));
    p.ecx = (res as f32).to_bits();
    p.edx = v;
    res
}

// --- Rust bodies: Lua getters with branches ------------------------------------------------------------

/// `luafn_get_mp_mode_index`: byte at 0xe8b210 minus 0xb (8-bit wrap).
fn lua_mp_mode_index(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let dl = rb(0xe8b210).wrapping_sub(0xb) as u32;
    p.edx = dl;
    p.ecx = push_num(w, argv(a, 0), dl as f64);
    p.eax = 1;
    0.0
}

/// `luafn_get_mp_map_index`: a table entry for the current mode, or a global, as unsigned.
fn lua_mp_map_index(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let v = if unsafe { std::ptr::read_unaligned(0xe8b214usize as *const u16) } == 0xFFFF {
        let i = (rb(0xe8b210).wrapping_sub(0xb)) as u32;
        rdd(i.wrapping_mul(4).wrapping_add(0x2130064))
    } else {
        rdd(0x2528d94)
    };
    let mut x = (v as i32) as f64;
    if (v as i32) < 0 {
        x += f64::from_bits(rq(0xe5c0e0));
    }
    p.edx = v;
    p.ecx = push_num(w, argv(a, 0), x);
    p.eax = 1;
    0.0
}

/// `luafn_get_coop_join_type`: 0 -> 0, 1 -> 1, 2 -> 2, anything else 0.
fn lua_coop_join_type(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let v = match rdd(0xe97310) {
        1 => 1.0,
        2 => 2.0,
        _ => 0.0,
    };
    p.ecx = push_num(w, argv(a, 0), v);
    p.eax = 1;
    0.0
}

/// `luafn_get_coop_friendly_fire`: 0 -> 2, 2 -> 1, anything else 0.
fn lua_coop_friendly_fire(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let v = match rdd(0xe97314) {
        0 => 2.0,
        2 => 1.0,
        _ => 0.0,
    };
    p.ecx = push_num(w, argv(a, 0), v);
    p.eax = 1;
    0.0
}

/// `luafn_get_multi_clock_time`: field +8 of the object at 0x2528c14, or the global 0x212b160,
/// as an unsigned number.
fn lua_multi_clock_time(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let obj = rdd(0x2528c14);
    let v = if obj != 0 { at(obj, 8) } else { rdd(0x212b160) };
    let mut x = (v as i32) as f64;
    if (v as i32) < 0 {
        x += f64::from_bits(rq(0xe5c0e0));
    }
    p.edx = v;
    p.ecx = push_num(w, argv(a, 0), x);
    p.eax = 1;
    0.0
}

/// `luafn_demo_derby_get_menu_type`: 2.0 (setting the mode to 2) while a flag is set, else the
/// mode 2 -> 2.0, 1 -> 1.0, other -> 0.0.
fn lua_demo_derby_menu_type(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let two = f64::from_bits(rq(0xe5bfe8));
    let v = if rb(0x2527809) != 0 {
        w.d(0xe97d04, 2);
        two
    } else {
        match rdd(0xe97d04) {
            2 => two,
            1 => 1.0,
            _ => 0.0,
        }
    };
    p.ecx = push_num(w, argv(a, 0), v);
    p.eax = 1;
    0.0
}

/// `luafn_pcu_bra_required`: pushes a boolean, true when the player-controller data is missing
/// or its constant is greater than the f32 at index `[0x25297b8]+0x1c` of the table.
fn lua_pcu_bra_required(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let g = rdd(0x21703d4);
    let tbl = at(g, 0x13b0);
    let mut cl = true;
    if tbl != 0 {
        let sel = rdd(0x25297b8);
        if sel != 0 {
            let idx = at(sel, 0x1c);
            let fbits = q32(rdd(at(tbl, 4).wrapping_add(idx.wrapping_mul(12)).wrapping_add(4)));
            p.ecx = fbits;
            cl = ldf(rdd(0xdd1c8c)) > ldf(fbits);
        }
    }
    p.edx = cl as u32;
    push_bool(w, argv(a, 0), cl as u32);
    p.eax = 1;
    0.0
}

/// `luafn_vint_hack_is_std_res`: pushes a boolean, `1.0 > f32 at 0x22dbfac`; CL carries it.
fn lua_hack_is_std_res(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let cl = (1.0 > ldf(rdd(0x22dbfac))) as u32;
    p.ecx = (p.ecx & !0xff) | cl;
    p.edx = cl;
    push_bool(w, argv(a, 0), cl);
    p.eax = 1;
    0.0
}

// --- Rust bodies: vint value / element accessors ------------------------------------------------------------

/// `vint_value_get_vec2` (ECX = value, EAX = out floats): zeroes the pair, then copies a vec2.
fn vint_value_get_vec2(p: &mut Pushad, _: *const u32, w: &mut W) -> f64 {
    let tag = rdd(p.ecx);
    w.d(p.eax as usize, 0);
    w.d(ad(p.eax, 4), 0);
    if tag == 7 {
        let x = q32(at(p.ecx, 4));
        w.d(p.eax as usize, x);
        let y = q32(at(p.ecx, 8));
        w.d(ad(p.eax, 4), y);
    }
    0.0
}

/// Element getters of a vec2 property (ECX = element, stdcall `(value*, extra)`): an empty or
/// vec2 value (tag 0 or 7) becomes the vec2 of the f32 fields `A` and `B`. EAX = value | 1 in
/// the low byte, EDX = the old tag.
fn get_vec2<const A: u32, const B: u32>(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let ptr = argv(a, 0);
    let tag = rdd(ptr);
    if tag == 0 || tag == 7 {
        w.d(ptr as usize, 7);
        let vb = q32(at(p.ecx, B));
        w.d(ad(ptr, 4), q32(at(p.ecx, A)));
        w.d(ad(ptr, 8), vb);
    }
    p.eax = (ptr & !0xff) | 1;
    p.edx = tag;
    0.0
}

/// Element setters of a vec2 property: the fields `A` / `B` take the value's vec2 (0, 0 when
/// the tag is not 7); `OR` sets bit 1 of the flag byte at +0xf0 first (text scale).
fn set_vec2<const A: u32, const B: u32, const OR: bool>(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let ptr = argv(a, 0);
    let (v0, v1) = if rdd(ptr) == 7 { (q32(at(ptr, 4)), q32(at(ptr, 8))) } else { (0, 0) };
    if OR {
        let f = rb(ad(p.ecx, 0xf0));
        w.b(ad(p.ecx, 0xf0), f | 2);
    }
    w.d(ad(p.ecx, A), v0);
    w.d(ad(p.ecx, B), v1);
    p.eax = (v0 & !0xff) | 1;
    p.edx = v1;
    0.0
}

/// Element setters of a colour property (three f32 fields at `O0..O2`): a tag-6 value is copied
/// as raw dwords, anything else gives 0.0 for each. EAX = third | 1 in the low byte, EDX = first.
fn set_color3<const O0: u32, const O1: u32, const O2: u32>(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let ptr = argv(a, 0);
    let (r, g, b) = if rdd(ptr) == 6 { (at(ptr, 4), at(ptr, 8), at(ptr, 0xc)) } else { (0, 0, 0) };
    w.d(ad(p.ecx, O0), r);
    w.d(ad(p.ecx, O1), g);
    w.d(ad(p.ecx, O2), b);
    p.eax = (b & !0xff) | 1;
    p.edx = r;
    0.0
}

/// Bitmap element source-corner getters (ECX = element, stdcall `(value*, extra)`): the integer
/// fields `A` / `B` as the f32 pair of an empty or vec2 value. ECX = the value pointer on return,
/// EAX = old tag | 1 in the low byte.
fn get_source<const A: u32, const B: u32>(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let fa = ((rd(ad(p.ecx, A)) as i32) as f64 as f32).to_bits();
    let fb = ((rd(ad(p.ecx, B)) as i32) as f64 as f32).to_bits();
    let ptr = argv(a, 0);
    let tag = rdd(ptr);
    if tag == 0 || tag == 7 {
        w.d(ptr as usize, 7);
        w.d(ad(ptr, 4), fa);
        w.d(ad(ptr, 8), fb);
    }
    p.ecx = ptr;
    p.eax = (tag & !0xff) | 1;
    0.0
}

/// `vint_element_point::get_screen_size`: an empty or vec2 value becomes (1.0, 1.0). ECX = old
/// tag, EAX = value | 1 in the low byte.
fn point_get_screen_size(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let ptr = argv(a, 0);
    let tag = rdd(ptr);
    if tag == 0 || tag == 7 {
        w.d(ptr as usize, 7);
        w.d(ad(ptr, 4), 1.0f32.to_bits());
        w.d(ad(ptr, 8), 1.0f32.to_bits());
    }
    p.ecx = tag;
    p.eax = (ptr & !0xff) | 1;
    0.0
}

/// `vint_element_text::set_wrap_width` (ECX = element, stdcall `(value*, extra)`): the number
/// value (tag 1 int, 2 unsigned int, 3 f32; else 0) rounded to f32 goes to +0xec, replaced by the
/// f32 at 0xdd071c when it is below the double at 0xe5c120; bit 1 of +0xf0 is set. EAX keeps
/// the `fnstsw` condition codes in its second byte.
fn text_set_wrap_width(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let ptr = argv(a, 0);
    let tag = rdd(ptr);
    let (x, e) = match tag {
        1 => ((at(ptr, 4) as i32) as f64, 0),
        2 => {
            let raw = at(ptr, 4);
            let mut x = (raw as i32) as f64;
            if (raw as i32) < 0 {
                x += ldf(rdd(0xe5bfd8));
            }
            (x, raw)
        }
        3 => (ldf(at(ptr, 4)), 0),
        _ => (0.0, tag.wrapping_sub(3)),
    };
    let v = r32(x);
    w.d(ad(p.ecx, 0xec), (v as f32).to_bits());
    let limit = f64::from_bits(rq(0xe5c120));
    let c = cc(v, limit);
    let f = rb(ad(p.ecx, 0xf0));
    let fin = if v < limit { ldf(rdd(0xdd071c)) } else { v };
    w.b(ad(p.ecx, 0xf0), f | 2);
    w.d(ad(p.ecx, 0xec), (fin as f32).to_bits());
    p.eax = (e & 0xFFFF_0000) | sw_ax(c, 1) & 0xFF00 | 1;
    p.edx = ptr;
    0.0
}


// --- Rust bodies: translated straight-line functions (one per exe function) -----------------------------

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_753410(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 753410 push ebp
    s[23] = ebp;
    // 753411 mov ebp, esp
    // 753413 and esp, 0xfffffff8
    // 753416 fild dword ptr [0x2528030]
    let t1: f64 = (rd(0x2528030usize) as i32) as f64;
    // 75341c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 75341f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 753422 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 753428 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 75342b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 75342f mov eax, 1
    eax = 0x1u32;
    // 753434 mov esp, ebp
    // 753436 pop ebp
    ebp = s[23];
    // 753437 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_775820(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 775820 push ebp
    s[23] = ebp;
    // 775821 mov ebp, esp
    // 775823 and esp, 0xfffffff8
    // 775826 fild dword ptr [0xe986f4]
    let t1: f64 = (rd(0xe986f4usize) as i32) as f64;
    // 77582c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 77582f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 775832 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 775838 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 77583b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 77583f mov eax, 1
    eax = 0x1u32;
    // 775844 mov esp, ebp
    // 775846 pop ebp
    ebp = s[23];
    // 775847 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_77aa70(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 77aa70 push ebp
    s[23] = ebp;
    // 77aa71 mov ebp, esp
    // 77aa73 and esp, 0xfffffff8
    // 77aa76 fild dword ptr [0x252836c]
    let t1: f64 = (rd(0x252836cusize) as i32) as f64;
    // 77aa7c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 77aa7f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 77aa82 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 77aa88 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 77aa8b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 77aa8f mov eax, 1
    eax = 0x1u32;
    // 77aa94 mov esp, ebp
    // 77aa96 pop ebp
    ebp = s[23];
    // 77aa97 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_7ee590(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 7ee590 push ebp
    s[23] = ebp;
    // 7ee591 mov ebp, esp
    // 7ee593 and esp, 0xfffffff8
    // 7ee596 fild dword ptr [0x252934c]
    let t1: f64 = (rd(0x252934cusize) as i32) as f64;
    // 7ee59c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 7ee59f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 7ee5a2 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 7ee5a8 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 7ee5ab add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 7ee5af mov eax, 1
    eax = 0x1u32;
    // 7ee5b4 mov esp, ebp
    // 7ee5b6 pop ebp
    ebp = s[23];
    // 7ee5b7 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_7ef9a0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 7ef9a0 push ebp
    s[23] = ebp;
    // 7ef9a1 mov ebp, esp
    // 7ef9a3 and esp, 0xfffffff8
    // 7ef9a6 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 7ef9a9 fild dword ptr [0x212a984]
    let t1: f64 = (rd(0x212a984usize) as i32) as f64;
    // 7ef9af mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 7ef9b2 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 7ef9b8 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 7ef9bb add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 7ef9bf mov eax, 1
    eax = 0x1u32;
    // 7ef9c4 mov esp, ebp
    // 7ef9c6 pop ebp
    ebp = s[23];
    // 7ef9c7 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a3d990(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // a3d990 push ebp
    s[23] = ebp;
    // a3d991 mov ebp, esp
    // a3d993 and esp, 0xfffffff8
    // a3d996 fild dword ptr [0xe87d20]
    let t1: f64 = (rd(0xe87d20usize) as i32) as f64;
    // a3d99c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // a3d99f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // a3d9a2 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // a3d9a8 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // a3d9ab add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // a3d9af mov eax, 1
    eax = 0x1u32;
    // a3d9b4 mov esp, ebp
    // a3d9b6 pop ebp
    ebp = s[23];
    // a3d9b7 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b90c60(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // b90c60 push ebp
    s[23] = ebp;
    // b90c61 mov ebp, esp
    // b90c63 and esp, 0xfffffff8
    // b90c66 fild dword ptr [0xe82e0c]
    let t1: f64 = (rd(0xe82e0cusize) as i32) as f64;
    // b90c6c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // b90c6f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // b90c72 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // b90c78 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // b90c7b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // b90c7f mov eax, 1
    eax = 0x1u32;
    // b90c84 mov esp, ebp
    // b90c86 pop ebp
    ebp = s[23];
    // b90c87 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b90d70(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // b90d70 push ebp
    s[23] = ebp;
    // b90d71 mov ebp, esp
    // b90d73 and esp, 0xfffffff8
    // b90d76 fild dword ptr [0xe82e0c]
    let t1: f64 = (rd(0xe82e0cusize) as i32) as f64;
    // b90d7c mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // b90d7f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // b90d82 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // b90d88 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // b90d8b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // b90d8f mov eax, 1
    eax = 0x1u32;
    // b90d94 mov esp, ebp
    // b90d96 pop ebp
    ebp = s[23];
    // b90d97 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_7d28e0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 7d28e0 push ebp
    s[23] = ebp;
    // 7d28e1 mov ebp, esp
    // 7d28e3 and esp, 0xfffffff8
    // 7d28e6 mov eax, dword ptr [0x21703d4]
    eax = rd(0x21703d4usize);
    // 7d28eb mov ecx, dword ptr [eax + 0x13b0]
    ecx = rd((eax.wrapping_add(0x13b0)) as usize);
    // 7d28f1 fild dword ptr [ecx + 0xc]
    let t1: f64 = (rd((ecx.wrapping_add(0xc)) as usize) as i32) as f64;
    // 7d28f4 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 7d28f7 mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 7d28fa mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 7d2900 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 7d2903 add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // 7d2907 mov eax, 1
    eax = 0x1u32;
    // 7d290c mov esp, ebp
    // 7d290e pop ebp
    ebp = s[23];
    // 7d290f ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a41850(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // a41850 push ebp
    s[23] = ebp;
    // a41851 mov ebp, esp
    // a41853 and esp, 0xfffffff8
    // a41856 mov eax, dword ptr [0x21703d4]
    eax = rd(0x21703d4usize);
    // a4185b fild dword ptr [eax + 0x1200]
    let t1: f64 = (rd((eax.wrapping_add(0x1200)) as usize) as i32) as f64;
    // a41861 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // a41864 mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // a41867 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // a4186d fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // a41870 add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // a41874 mov eax, 1
    eax = 0x1u32;
    // a41879 mov esp, ebp
    // a4187b pop ebp
    ebp = s[23];
    // a4187c ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a2e510(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // a2e510 push ebp
    s[23] = ebp;
    // a2e511 mov ebp, esp
    // a2e513 and esp, 0xfffffff8
    // a2e516 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // a2e519 fld dword ptr [0x2526da0]
    let t1: f64 = f32::from_bits(rd(0x2526da0usize)) as f64;
    // a2e51f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // a2e522 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // a2e525 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // a2e52b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // a2e52f mov eax, 1
    eax = 0x1u32;
    // a2e534 mov esp, ebp
    // a2e536 pop ebp
    ebp = s[23];
    // a2e537 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a398c0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // a398c0 push ebp
    s[23] = ebp;
    // a398c1 mov ebp, esp
    // a398c3 and esp, 0xfffffff8
    // a398c6 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // a398c9 fldz 
    let t1: f64 = 0.0;
    // a398cb mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // a398ce fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // a398d1 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // a398d7 add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // a398db mov eax, 1
    eax = 0x1u32;
    // a398e0 mov esp, ebp
    // a398e2 pop ebp
    ebp = s[23];
    // a398e3 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a47b50(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // a47b50 push ebp
    s[23] = ebp;
    // a47b51 mov ebp, esp
    // a47b53 and esp, 0xfffffff8
    // a47b56 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // a47b59 fld1 
    let t1: f64 = 1.0;
    // a47b5b mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // a47b5e fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // a47b61 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // a47b67 add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // a47b6b mov eax, 1
    eax = 0x1u32;
    // a47b70 mov esp, ebp
    // a47b72 pop ebp
    ebp = s[23];
    // a47b73 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_d74500(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // d74500 push ebp
    s[23] = ebp;
    // d74501 mov ebp, esp
    // d74503 and esp, 0xfffffff8
    // d74506 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // d74509 fld dword ptr [0xe84380]
    let t1: f64 = f32::from_bits(rd(0xe84380usize)) as f64;
    // d7450f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // d74512 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // d74515 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // d7451b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // d7451f mov eax, 1
    eax = 0x1u32;
    // d74524 mov esp, ebp
    // d74526 pop ebp
    ebp = s[23];
    // d74527 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_d74530(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // d74530 push ebp
    s[23] = ebp;
    // d74531 mov ebp, esp
    // d74533 and esp, 0xfffffff8
    // d74536 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // d74539 fld dword ptr [0xe84380]
    let t1: f64 = f32::from_bits(rd(0xe84380usize)) as f64;
    // d7453f mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // d74542 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // d74545 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // d7454b add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // d7454f mov eax, 1
    eax = 0x1u32;
    // d74554 mov esp, ebp
    // d74556 pop ebp
    ebp = s[23];
    // d74557 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_d71d50(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // d71d50 mov eax, dword ptr [esp + 4]
    eax = s[25];
    // d71d54 mov ecx, dword ptr [eax + 0x10]
    ecx = rd((eax.wrapping_add(0x10)) as usize);
    // d71d57 mov edx, dword ptr [ecx + 0x24]
    edx = rd((ecx.wrapping_add(0x24)) as usize);
    // d71d5a mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // d71d5d shr edx, 0xa
    edx = edx >> 10;
    // d71d60 mov dword ptr [esp + 4], edx
    s[25] = edx;
    // d71d64 fild dword ptr [esp + 4]
    let t1: f64 = (s[25] as i32) as f64;
    // d71d68 mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // d71d6e push esi
    s[23] = esi;
    // d71d6f mov edx, 0x10
    edx = 0x10u32;
    // d71d74 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // d71d77 add dword ptr [eax + 8], edx
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(edx)); }
    // d71d7a mov esi, dword ptr [eax + 0x10]
    esi = rd((eax.wrapping_add(0x10)) as usize);
    // d71d7d mov esi, dword ptr [esi + 0x20]
    esi = rd((esi.wrapping_add(0x20)) as usize);
    // d71d80 mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // d71d83 shr esi, 0xa
    esi = esi >> 10;
    // d71d86 mov dword ptr [esp + 8], esi
    s[25] = esi;
    // d71d8a fild dword ptr [esp + 8]
    let t2: f64 = (s[25] as i32) as f64;
    // d71d8e mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // d71d94 pop esi
    esi = s[23];
    // d71d95 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t2.to_bits());
    // d71d98 add dword ptr [eax + 8], edx
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(edx)); }
    // d71d9b lea eax, [edx - 0xe]
    eax = ((edx.wrapping_add(0xfffffff2)) as usize) as u32;
    // d71d9e ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_789480(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 789480 push ebp
    s[23] = ebp;
    // 789481 mov ebp, esp
    // 789483 and esp, 0xfffffff8
    // 789486 push ecx
    s[22] = ecx;
    // 789487 movzx eax, byte ptr [0x140e72c]
    eax = rb(0x140e72cusize) as u32;
    // 78948e mov dword ptr [esp], eax
    s[22] = eax;
    // 789491 mov eax, dword ptr [ebp + 8]
    eax = s[25];
    // 789494 mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 789497 fild dword ptr [esp]
    let t1: f64 = (s[22] as i32) as f64;
    // 78949a mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 7894a0 mov edx, 0x10
    edx = 0x10u32;
    // 7894a5 push esi
    s[21] = esi;
    // 7894a6 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t1.to_bits());
    // 7894a9 add dword ptr [eax + 8], edx
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(edx)); }
    // 7894ac movzx esi, byte ptr [0x140e72d]
    esi = rb(0x140e72dusize) as u32;
    // 7894b3 mov ecx, dword ptr [eax + 8]
    ecx = rd((eax.wrapping_add(0x8)) as usize);
    // 7894b6 mov dword ptr [esp + 4], esi
    s[22] = esi;
    // 7894ba mov dword ptr [ecx], 3
    w.d((ecx) as usize, 0x3u32);
    // 7894c0 pop esi
    esi = s[21];
    // 7894c1 fild dword ptr [esp]
    let t2: f64 = (s[22] as i32) as f64;
    // 7894c4 fstp qword ptr [ecx + 8]
    w.q((ecx.wrapping_add(0x8)) as usize, t2.to_bits());
    // 7894c7 add dword ptr [eax + 8], edx
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(edx)); }
    // 7894ca lea eax, [edx - 0xe]
    eax = ((edx.wrapping_add(0xfffffff2)) as usize) as u32;
    // 7894cd mov esp, ebp
    // 7894cf pop ebp
    ebp = s[23];
    // 7894d0 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a3cfe0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // a3cfe0 fldz 
    let t1: f64 = 0.0;
    // a3cfe2 xor eax, eax
    eax = 0;
    // a3cfe4 fstp dword ptr [0x2529740]
    w.d(0x2529740usize, (t1 as f32).to_bits());
    // a3cfea ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_cdfc60(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    // cdfc60 mov eax, dword ptr [ecx + 4]
    eax = rd((ecx.wrapping_add(0x4)) as usize);
    // cdfc63 fld qword ptr [esp + 4]
    let t1: f64 = f64::from_bits(q64((s[26] as u64) << 32 | s[25] as u64));
    // cdfc67 mov edx, dword ptr [eax + 8]
    edx = rd((eax.wrapping_add(0x8)) as usize);
    // cdfc6a fstp qword ptr [edx + 8]
    w.q((edx.wrapping_add(0x8)) as usize, t1.to_bits());
    // cdfc6d mov dword ptr [edx], 3
    w.d((edx) as usize, 0x3u32);
    // cdfc73 add dword ptr [eax + 8], 0x10
    { let ad = (eax.wrapping_add(0x8)) as usize; w.d(ad, rd(ad).wrapping_add(0x10u32)); }
    // cdfc77 add dword ptr [ecx + 0x1c], 1
    { let ad = (ecx.wrapping_add(0x1c)) as usize; w.d(ad, rd(ad).wrapping_add(0x1u32)); }
    // cdfc7b ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_5bc250(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 5bc250 fldz 
    let t1: f64 = 0.0;
    // 5bc252 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_761860(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 761860 fld dword ptr [ecx + 0x10]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 761863 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_761870(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 761870 fld dword ptr [ecx + 0x14]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0x14)) as usize)) as f64;
    // 761873 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_77b2e0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 77b2e0 push ecx
    s[23] = ecx;
    // 77b2e1 mov eax, dword ptr [ecx + 0x14]
    eax = rd((ecx.wrapping_add(0x14)) as usize);
    // 77b2e4 add eax, dword ptr [ecx + 0x10]
    eax = eax.wrapping_add(rd((ecx.wrapping_add(0x10)) as usize));
    // 77b2e7 mov dword ptr [esp], eax
    s[23] = eax;
    // 77b2ea fild dword ptr [esp]
    let t1: f64 = (s[23] as i32) as f64;
    // 77b2ed fmul qword ptr [0xe5c228]
    let t2: f64 = f64::from_bits(q64(rq(0xe5c228usize)));
    let t3: f64 = t1 * t2;
    // 77b2f3 fadd qword ptr [0xe5bf68]
    let t4: f64 = f64::from_bits(q64(rq(0xe5bf68usize)));
    let t5: f64 = t3 + t4;
    // 77b2f9 fstp dword ptr [esp]
    s[23] = (t5 as f32).to_bits();
    // 77b2fc fld dword ptr [esp]
    let t6: f64 = f32::from_bits(s[23]) as f64;
    // 77b2ff pop ecx
    ecx = s[23];
    // 77b300 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t6
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_781df0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 781df0 fld1 
    let t1: f64 = 1.0;
    // 781df2 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_781f40(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 781f40 fld dword ptr [0xe5c220]
    let t1: f64 = f32::from_bits(rd(0xe5c220usize)) as f64;
    // 781f46 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_781f50(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // 781f50 fld dword ptr [ecx + 0x974]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0x974)) as usize)) as f64;
    // 781f56 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_aefe60(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // aefe60 fld dword ptr [0xe8725c]
    let t1: f64 = f32::from_bits(rd(0xe8725cusize)) as f64;
    // aefe66 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b75c30(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // b75c30 fld dword ptr [0xe8725c]
    let t1: f64 = f32::from_bits(rd(0xe8725cusize)) as f64;
    // b75c36 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b75ca0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // b75ca0 fld dword ptr [0xe5c0f0]
    let t1: f64 = f32::from_bits(rd(0xe5c0f0usize)) as f64;
    // b75ca6 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b7eeb0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // b7eeb0 fld dword ptr [0xe8725c]
    let t1: f64 = f32::from_bits(rd(0xe8725cusize)) as f64;
    // b7eeb6 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_cf5350(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let _ = a;
    // cf5350 fldz 
    let t1: f64 = 0.0;
    // cf5352 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t1
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_404a20(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    // 404a20 sub esp, 8
    // 404a23 fld dword ptr [ecx]
    let t1: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    // 404a25 fsub dword ptr [eax]
    let t2: f64 = f32::from_bits(rd((eax) as usize)) as f64;
    let t3: f64 = t1 - t2;
    // 404a27 fstp dword ptr [esp]
    s[22] = (t3 as f32).to_bits();
    // 404a2a fld dword ptr [esp]
    let t4: f64 = f32::from_bits(s[22]) as f64;
    // 404a2d fmul st(0), st(0)
    let t5: f64 = t4 * t4;
    // 404a2f fstp dword ptr [esp + 4]
    s[23] = (t5 as f32).to_bits();
    // 404a33 fld dword ptr [ecx + 4]
    let t6: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    // 404a36 fsub dword ptr [eax + 4]
    let t7: f64 = f32::from_bits(rd((eax.wrapping_add(0x4)) as usize)) as f64;
    let t8: f64 = t6 - t7;
    // 404a39 fstp dword ptr [esp]
    s[22] = (t8 as f32).to_bits();
    // 404a3c fld dword ptr [esp]
    let t9: f64 = f32::from_bits(s[22]) as f64;
    // 404a3f fmul st(0), st(0)
    let t10: f64 = t9 * t9;
    // 404a41 fadd dword ptr [esp + 4]
    let t11: f64 = f32::from_bits(s[23]) as f64;
    let t12: f64 = t10 + t11;
    // 404a45 fstp dword ptr [esp + 4]
    s[23] = (t12 as f32).to_bits();
    // 404a49 fld dword ptr [ecx + 8]
    let t13: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    // 404a4c fsub dword ptr [eax + 8]
    let t14: f64 = f32::from_bits(rd((eax.wrapping_add(0x8)) as usize)) as f64;
    let t15: f64 = t13 - t14;
    // 404a4f fstp dword ptr [esp]
    s[22] = (t15 as f32).to_bits();
    // 404a52 fld dword ptr [esp]
    let t16: f64 = f32::from_bits(s[22]) as f64;
    // 404a55 fmul st(0), st(0)
    let t17: f64 = t16 * t16;
    // 404a57 fadd dword ptr [esp + 4]
    let t18: f64 = f32::from_bits(s[23]) as f64;
    let t19: f64 = t17 + t18;
    // 404a5b fstp dword ptr [esp + 4]
    s[23] = (t19 as f32).to_bits();
    // 404a5f fld dword ptr [esp + 4]
    let t20: f64 = f32::from_bits(s[23]) as f64;
    // 404a63 add esp, 8
    // 404a66 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t20
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_bde570(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    // bde570 sub esp, 0xc
    // bde573 fld dword ptr [ecx]
    let t1: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    // bde575 fstp dword ptr [esp + 4]
    s[22] = (t1 as f32).to_bits();
    // bde579 fld dword ptr [ecx + 4]
    let t2: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    // bde57c fstp dword ptr [esp]
    s[21] = (t2 as f32).to_bits();
    // bde57f fld dword ptr [ecx + 8]
    let t3: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    // bde582 fstp dword ptr [esp + 8]
    s[23] = (t3 as f32).to_bits();
    // bde586 fld dword ptr [eax + 0xc]
    let t4: f64 = f32::from_bits(rd((eax.wrapping_add(0xc)) as usize)) as f64;
    // bde589 fld dword ptr [esp]
    let t5: f64 = f32::from_bits(s[21]) as f64;
    // bde58c fld st(0)
    // bde58e fmulp st(2)
    let t6: f64 = t4 * t5;
    // bde590 fld dword ptr [eax]
    let t7: f64 = f32::from_bits(rd((eax) as usize)) as f64;
    // bde592 fld dword ptr [esp + 4]
    let t8: f64 = f32::from_bits(s[22]) as f64;
    // bde596 fld st(0)
    // bde598 fmulp st(2)
    let t9: f64 = t7 * t8;
    // bde59a fxch st(3)
    // bde59c faddp st(1)
    let t10: f64 = t9 + t6;
    // bde59e fld dword ptr [eax + 0x18]
    let t11: f64 = f32::from_bits(rd((eax.wrapping_add(0x18)) as usize)) as f64;
    // bde5a1 fld dword ptr [esp + 8]
    let t12: f64 = f32::from_bits(s[23]) as f64;
    // bde5a5 fld st(0)
    // bde5a7 fmulp st(2)
    let t13: f64 = t11 * t12;
    // bde5a9 fxch st(2)
    // bde5ab faddp st(1)
    let t14: f64 = t13 + t10;
    // bde5ad fstp dword ptr [ecx]
    w.d((ecx) as usize, (t14 as f32).to_bits());
    // bde5af fld dword ptr [eax + 0x10]
    let t15: f64 = f32::from_bits(rd((eax.wrapping_add(0x10)) as usize)) as f64;
    // bde5b2 fmul st(2)
    let t16: f64 = t15 * t5;
    // bde5b4 fld dword ptr [eax + 4]
    let t17: f64 = f32::from_bits(rd((eax.wrapping_add(0x4)) as usize)) as f64;
    // bde5b7 fmul st(4)
    let t18: f64 = t17 * t8;
    // bde5b9 faddp st(1)
    let t19: f64 = t16 + t18;
    // bde5bb fld dword ptr [eax + 0x1c]
    let t20: f64 = f32::from_bits(rd((eax.wrapping_add(0x1c)) as usize)) as f64;
    // bde5be fmul st(2)
    let t21: f64 = t20 * t12;
    // bde5c0 faddp st(1)
    let t22: f64 = t19 + t21;
    // bde5c2 fstp dword ptr [ecx + 4]
    w.d((ecx.wrapping_add(0x4)) as usize, (t22 as f32).to_bits());
    // bde5c5 fld dword ptr [eax + 0x14]
    let t23: f64 = f32::from_bits(rd((eax.wrapping_add(0x14)) as usize)) as f64;
    // bde5c8 fmulp st(2)
    let t24: f64 = t5 * t23;
    // bde5ca fld dword ptr [eax + 8]
    let t25: f64 = f32::from_bits(rd((eax.wrapping_add(0x8)) as usize)) as f64;
    // bde5cd fmulp st(3)
    let t26: f64 = t8 * t25;
    // bde5cf fxch st(1)
    // bde5d1 faddp st(2)
    let t27: f64 = t26 + t24;
    // bde5d3 fmul dword ptr [eax + 0x20]
    let t28: f64 = f32::from_bits(rd((eax.wrapping_add(0x20)) as usize)) as f64;
    let t29: f64 = t12 * t28;
    // bde5d6 faddp st(1)
    let t30: f64 = t27 + t29;
    // bde5d8 fstp dword ptr [ecx + 8]
    w.d((ecx.wrapping_add(0x8)) as usize, (t30 as f32).to_bits());
    // bde5db add esp, 0xc
    // bde5de ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_bdde20(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // bdde20 sub esp, 8
    // bdde23 mov edx, dword ptr [esp + 0xc]
    edx = s[25];
    // bdde27 fld dword ptr [edx + 4]
    let t1: f64 = f32::from_bits(rd((edx.wrapping_add(0x4)) as usize)) as f64;
    // bdde2a fstp dword ptr [esp + 0xc]
    s[25] = (t1 as f32).to_bits();
    // bdde2e fld dword ptr [edx]
    let t2: f64 = f32::from_bits(rd((edx) as usize)) as f64;
    // bdde30 fstp dword ptr [esp]
    s[22] = (t2 as f32).to_bits();
    // bdde33 fld dword ptr [edx + 8]
    let t3: f64 = f32::from_bits(rd((edx.wrapping_add(0x8)) as usize)) as f64;
    // bdde36 fstp dword ptr [esp + 4]
    s[23] = (t3 as f32).to_bits();
    // bdde3a fld dword ptr [ecx + 0xc]
    let t4: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // bdde3d fld dword ptr [esp + 0xc]
    let t5: f64 = f32::from_bits(s[25]) as f64;
    // bdde41 fld st(0)
    // bdde43 fmulp st(2)
    let t6: f64 = t4 * t5;
    // bdde45 fld dword ptr [ecx]
    let t7: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    // bdde47 fld dword ptr [esp]
    let t8: f64 = f32::from_bits(s[22]) as f64;
    // bdde4a fld st(0)
    // bdde4c fmulp st(2)
    let t9: f64 = t7 * t8;
    // bdde4e fxch st(3)
    // bdde50 faddp st(1)
    let t10: f64 = t9 + t6;
    // bdde52 fld dword ptr [ecx + 0x18]
    let t11: f64 = f32::from_bits(rd((ecx.wrapping_add(0x18)) as usize)) as f64;
    // bdde55 fld dword ptr [esp + 4]
    let t12: f64 = f32::from_bits(s[23]) as f64;
    // bdde59 fld st(0)
    // bdde5b fmulp st(2)
    let t13: f64 = t11 * t12;
    // bdde5d fxch st(2)
    // bdde5f faddp st(1)
    let t14: f64 = t13 + t10;
    // bdde61 fstp dword ptr [eax]
    w.d((eax) as usize, (t14 as f32).to_bits());
    // bdde63 fld dword ptr [ecx + 4]
    let t15: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    // bdde66 fmul st(3)
    let t16: f64 = t15 * t8;
    // bdde68 fld dword ptr [ecx + 0x10]
    let t17: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // bdde6b fmul st(3)
    let t18: f64 = t17 * t5;
    // bdde6d faddp st(1)
    let t19: f64 = t16 + t18;
    // bdde6f fld dword ptr [ecx + 0x1c]
    let t20: f64 = f32::from_bits(rd((ecx.wrapping_add(0x1c)) as usize)) as f64;
    // bdde72 fmul st(2)
    let t21: f64 = t20 * t12;
    // bdde74 faddp st(1)
    let t22: f64 = t19 + t21;
    // bdde76 fstp dword ptr [eax + 4]
    w.d((eax.wrapping_add(0x4)) as usize, (t22 as f32).to_bits());
    // bdde79 fld dword ptr [ecx + 8]
    let t23: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    // bdde7c fmulp st(3)
    let t24: f64 = t8 * t23;
    // bdde7e fld dword ptr [ecx + 0x14]
    let t25: f64 = f32::from_bits(rd((ecx.wrapping_add(0x14)) as usize)) as f64;
    // bdde81 fmulp st(2)
    let t26: f64 = t5 * t25;
    // bdde83 fxch st(2)
    // bdde85 faddp st(1)
    let t27: f64 = t26 + t24;
    // bdde87 fld dword ptr [ecx + 0x20]
    let t28: f64 = f32::from_bits(rd((ecx.wrapping_add(0x20)) as usize)) as f64;
    // bdde8a fmulp st(2)
    let t29: f64 = t12 * t28;
    // bdde8c faddp st(1)
    let t30: f64 = t29 + t27;
    // bdde8e fstp dword ptr [eax + 8]
    w.d((eax.wrapping_add(0x8)) as usize, (t30 as f32).to_bits());
    // bdde91 add esp, 8
    // bdde94 ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_a0fe30(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    // a0fe30 sub esp, 0x18
    // a0fe33 mov eax, dword ptr [edx + 4]
    eax = rd((edx.wrapping_add(0x4)) as usize);
    // a0fe36 mov ecx, dword ptr [edx]
    ecx = rd((edx) as usize);
    // a0fe38 fld dword ptr [eax]
    let t1: f64 = f32::from_bits(rd((eax) as usize)) as f64;
    // a0fe3a fadd dword ptr [ecx]
    let t2: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    let t3: f64 = t1 + t2;
    // a0fe3c fstp dword ptr [esp]
    s[18] = (t3 as f32).to_bits();
    // a0fe3f fld dword ptr [eax + 4]
    let t4: f64 = f32::from_bits(rd((eax.wrapping_add(0x4)) as usize)) as f64;
    // a0fe42 fadd dword ptr [ecx + 4]
    let t5: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t6: f64 = t4 + t5;
    // a0fe45 fstp dword ptr [esp + 4]
    s[19] = (t6 as f32).to_bits();
    // a0fe49 fld dword ptr [eax + 8]
    let t7: f64 = f32::from_bits(rd((eax.wrapping_add(0x8)) as usize)) as f64;
    // a0fe4c mov eax, dword ptr [edx + 8]
    eax = rd((edx.wrapping_add(0x8)) as usize);
    // a0fe4f fadd dword ptr [ecx + 8]
    let t8: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    let t9: f64 = t7 + t8;
    // a0fe52 fstp dword ptr [esp + 8]
    s[20] = (t9 as f32).to_bits();
    // a0fe56 fld dword ptr [eax]
    let t10: f64 = f32::from_bits(rd((eax) as usize)) as f64;
    // a0fe58 fadd dword ptr [esp]
    let t11: f64 = f32::from_bits(s[18]) as f64;
    let t12: f64 = t10 + t11;
    // a0fe5b fstp dword ptr [esp + 0xc]
    s[21] = (t12 as f32).to_bits();
    // a0fe5f fld dword ptr [eax + 4]
    let t13: f64 = f32::from_bits(rd((eax.wrapping_add(0x4)) as usize)) as f64;
    // a0fe62 fadd dword ptr [esp + 4]
    let t14: f64 = f32::from_bits(s[19]) as f64;
    let t15: f64 = t13 + t14;
    // a0fe66 fstp dword ptr [esp + 0x10]
    s[22] = (t15 as f32).to_bits();
    // a0fe6a fld dword ptr [eax + 8]
    let t16: f64 = f32::from_bits(rd((eax.wrapping_add(0x8)) as usize)) as f64;
    // a0fe6d fadd dword ptr [esp + 8]
    let t17: f64 = f32::from_bits(s[20]) as f64;
    let t18: f64 = t16 + t17;
    // a0fe71 fstp dword ptr [esp + 0x14]
    s[23] = (t18 as f32).to_bits();
    // a0fe75 fld dword ptr [esp + 0xc]
    let t19: f64 = f32::from_bits(s[21]) as f64;
    // a0fe79 fld qword ptr [0xe5c4c0]
    let t20: f64 = f64::from_bits(q64(rq(0xe5c4c0usize)));
    // a0fe7f fmul st(1), st(0)
    let t21: f64 = t19 * t20;
    // a0fe81 fxch st(1)
    // a0fe83 fstp dword ptr [esp]
    s[18] = (t21 as f32).to_bits();
    // a0fe86 mov eax, dword ptr [esp]
    eax = s[18];
    // a0fe89 fld dword ptr [esp + 0x10]
    let t22: f64 = f32::from_bits(s[22]) as f64;
    // a0fe8d mov dword ptr [esi], eax
    w.d((esi) as usize, eax);
    // a0fe8f fmul st(1)
    let t23: f64 = t22 * t20;
    // a0fe91 fstp dword ptr [esp + 4]
    s[19] = (t23 as f32).to_bits();
    // a0fe95 mov ecx, dword ptr [esp + 4]
    ecx = s[19];
    // a0fe99 mov dword ptr [esi + 4], ecx
    w.d((esi.wrapping_add(0x4)) as usize, ecx);
    // a0fe9c fmul dword ptr [esp + 0x14]
    let t24: f64 = f32::from_bits(s[23]) as f64;
    let t25: f64 = t20 * t24;
    // a0fea0 fstp dword ptr [esp + 8]
    s[20] = (t25 as f32).to_bits();
    // a0fea4 mov edx, dword ptr [esp + 8]
    edx = s[20];
    // a0fea8 mov dword ptr [esi + 8], edx
    w.d((esi.wrapping_add(0x8)) as usize, edx);
    // a0feab add esp, 0x18
    // a0feae ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_729f30(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // 729f30 mov ecx, dword ptr [esp + 4]
    ecx = s[25];
    // 729f34 fld dword ptr [edx]
    let t1: f64 = f32::from_bits(rd((edx) as usize)) as f64;
    // 729f36 fmul dword ptr [ecx]
    let t2: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    let t3: f64 = t1 * t2;
    // 729f38 fld dword ptr [edx + 0xc]
    let t4: f64 = f32::from_bits(rd((edx.wrapping_add(0xc)) as usize)) as f64;
    // 729f3b fmul dword ptr [ecx + 4]
    let t5: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t6: f64 = t4 * t5;
    // 729f3e faddp st(1)
    let t7: f64 = t3 + t6;
    // 729f40 fstp dword ptr [eax]
    w.d((eax) as usize, (t7 as f32).to_bits());
    // 729f42 fld dword ptr [edx + 0x10]
    let t8: f64 = f32::from_bits(rd((edx.wrapping_add(0x10)) as usize)) as f64;
    // 729f45 fmul dword ptr [ecx + 4]
    let t9: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t10: f64 = t8 * t9;
    // 729f48 fld dword ptr [edx + 4]
    let t11: f64 = f32::from_bits(rd((edx.wrapping_add(0x4)) as usize)) as f64;
    // 729f4b fmul dword ptr [ecx]
    let t12: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    let t13: f64 = t11 * t12;
    // 729f4d faddp st(1)
    let t14: f64 = t10 + t13;
    // 729f4f fstp dword ptr [eax + 4]
    w.d((eax.wrapping_add(0x4)) as usize, (t14 as f32).to_bits());
    // 729f52 fld dword ptr [edx + 0x14]
    let t15: f64 = f32::from_bits(rd((edx.wrapping_add(0x14)) as usize)) as f64;
    // 729f55 fmul dword ptr [ecx + 4]
    let t16: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t17: f64 = t15 * t16;
    // 729f58 fld dword ptr [edx + 8]
    let t18: f64 = f32::from_bits(rd((edx.wrapping_add(0x8)) as usize)) as f64;
    // 729f5b fmul dword ptr [ecx]
    let t19: f64 = f32::from_bits(rd((ecx) as usize)) as f64;
    let t20: f64 = t18 * t19;
    // 729f5d faddp st(1)
    let t21: f64 = t17 + t20;
    // 729f5f fadd dword ptr [ecx + 8]
    let t22: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    let t23: f64 = t21 + t22;
    // 729f62 fstp dword ptr [eax + 8]
    w.d((eax.wrapping_add(0x8)) as usize, (t23 as f32).to_bits());
    // 729f65 fld dword ptr [ecx + 0x10]
    let t24: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 729f68 fmul dword ptr [edx + 0xc]
    let t25: f64 = f32::from_bits(rd((edx.wrapping_add(0xc)) as usize)) as f64;
    let t26: f64 = t24 * t25;
    // 729f6b fld dword ptr [edx]
    let t27: f64 = f32::from_bits(rd((edx) as usize)) as f64;
    // 729f6d fmul dword ptr [ecx + 0xc]
    let t28: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    let t29: f64 = t27 * t28;
    // 729f70 faddp st(1)
    let t30: f64 = t26 + t29;
    // 729f72 fstp dword ptr [eax + 0xc]
    w.d((eax.wrapping_add(0xc)) as usize, (t30 as f32).to_bits());
    // 729f75 fld dword ptr [edx + 4]
    let t31: f64 = f32::from_bits(rd((edx.wrapping_add(0x4)) as usize)) as f64;
    // 729f78 fmul dword ptr [ecx + 0xc]
    let t32: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    let t33: f64 = t31 * t32;
    // 729f7b fld dword ptr [ecx + 0x10]
    let t34: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 729f7e fmul dword ptr [edx + 0x10]
    let t35: f64 = f32::from_bits(rd((edx.wrapping_add(0x10)) as usize)) as f64;
    let t36: f64 = t34 * t35;
    // 729f81 faddp st(1)
    let t37: f64 = t33 + t36;
    // 729f83 fstp dword ptr [eax + 0x10]
    w.d((eax.wrapping_add(0x10)) as usize, (t37 as f32).to_bits());
    // 729f86 fld dword ptr [edx + 8]
    let t38: f64 = f32::from_bits(rd((edx.wrapping_add(0x8)) as usize)) as f64;
    // 729f89 fmul dword ptr [ecx + 0xc]
    let t39: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    let t40: f64 = t38 * t39;
    // 729f8c fld dword ptr [ecx + 0x10]
    let t41: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 729f8f fmul dword ptr [edx + 0x14]
    let t42: f64 = f32::from_bits(rd((edx.wrapping_add(0x14)) as usize)) as f64;
    let t43: f64 = t41 * t42;
    // 729f92 faddp st(1)
    let t44: f64 = t40 + t43;
    // 729f94 fadd dword ptr [ecx + 0x14]
    let t45: f64 = f32::from_bits(rd((ecx.wrapping_add(0x14)) as usize)) as f64;
    let t46: f64 = t44 + t45;
    // 729f97 fstp dword ptr [eax + 0x14]
    w.d((eax.wrapping_add(0x14)) as usize, (t46 as f32).to_bits());
    // 729f9a ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_bdb4f0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    // bdb4f0 push ecx
    s[23] = ecx;
    // bdb4f1 movzx edx, byte ptr [eax]
    edx = rb((eax) as usize) as u32;
    // bdb4f4 mov dword ptr [esp], edx
    s[23] = edx;
    // bdb4f7 fild dword ptr [esp]
    let t1: f64 = (s[23] as i32) as f64;
    // bdb4fa fld qword ptr [0xe5c038]
    let t2: f64 = f64::from_bits(q64(rq(0xe5c038usize)));
    // bdb500 fdiv st(1), st(0)
    let t3: f64 = t1 / t2;
    // bdb502 fld qword ptr [0xe5bfe8]
    let t4: f64 = f64::from_bits(q64(rq(0xe5bfe8usize)));
    // bdb508 fmul st(2), st(0)
    let t5: f64 = t3 * t4;
    // bdb50a fld1 
    let t6: f64 = 1.0;
    // bdb50c fsub st(3), st(0)
    let t7: f64 = t5 - t6;
    // bdb50e fxch st(3)
    // bdb510 fstp dword ptr [ecx]
    w.d((ecx) as usize, (t7 as f32).to_bits());
    // bdb512 movzx edx, byte ptr [eax + 1]
    edx = rb((eax.wrapping_add(0x1)) as usize) as u32;
    // bdb516 mov dword ptr [esp], edx
    s[23] = edx;
    // bdb519 fild dword ptr [esp]
    let t8: f64 = (s[23] as i32) as f64;
    // bdb51c fdiv st(2)
    let t9: f64 = t8 / t2;
    // bdb51e fmul st(1)
    let t10: f64 = t9 * t4;
    // bdb520 fsub st(3)
    let t11: f64 = t10 - t6;
    // bdb522 fstp dword ptr [ecx + 4]
    w.d((ecx.wrapping_add(0x4)) as usize, (t11 as f32).to_bits());
    // bdb525 movzx edx, byte ptr [eax + 2]
    edx = rb((eax.wrapping_add(0x2)) as usize) as u32;
    // bdb529 mov dword ptr [esp], edx
    s[23] = edx;
    // bdb52c fild dword ptr [esp]
    let t12: f64 = (s[23] as i32) as f64;
    // bdb52f fdiv st(2)
    let t13: f64 = t12 / t2;
    // bdb531 fmul st(1)
    let t14: f64 = t13 * t4;
    // bdb533 fsub st(3)
    let t15: f64 = t14 - t6;
    // bdb535 fstp dword ptr [ecx + 8]
    w.d((ecx.wrapping_add(0x8)) as usize, (t15 as f32).to_bits());
    // bdb538 movzx eax, byte ptr [eax + 3]
    eax = rb((eax.wrapping_add(0x3)) as usize) as u32;
    // bdb53c mov dword ptr [esp], eax
    s[23] = eax;
    // bdb53f mov ecx, dword ptr [esp + 8]
    ecx = s[25];
    // bdb543 fild dword ptr [esp]
    let t16: f64 = (s[23] as i32) as f64;
    // bdb546 fdivrp st(2)
    let t17: f64 = t16 / t2;
    // bdb548 fmulp st(1)
    let t18: f64 = t17 * t4;
    // bdb54a fsubrp st(1)
    let t19: f64 = t18 - t6;
    // bdb54c fstp dword ptr [ecx]
    w.d((ecx) as usize, (t19 as f32).to_bits());
    // bdb54e pop ecx
    ecx = s[23];
    // bdb54f ret 4
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_50b440(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    // 50b440 push ecx
    s[23] = ecx;
    // 50b441 mov eax, dword ptr [ecx + 0xd0]
    eax = rd((ecx.wrapping_add(0xd0)) as usize);
    // 50b447 fld dword ptr [eax + 0x1c]
    let t1: f64 = f32::from_bits(rd((eax.wrapping_add(0x1c)) as usize)) as f64;
    // 50b44a mov eax, dword ptr [esp + 8]
    eax = s[25];
    // 50b44e fstp dword ptr [esp]
    s[23] = (t1 as f32).to_bits();
    // 50b451 fld dword ptr [esp]
    let t2: f64 = f32::from_bits(s[23]) as f64;
    // 50b454 fld st(0)
    // 50b456 fchs 
    let t3: f64 = -t2;
    // 50b458 fstp dword ptr [esp]
    s[23] = (t3 as f32).to_bits();
    // 50b45b fld dword ptr [esp]
    let t4: f64 = f32::from_bits(s[23]) as f64;
    // 50b45e fst dword ptr [eax]
    w.d((eax) as usize, (t4 as f32).to_bits());
    // 50b460 fst dword ptr [eax + 4]
    w.d((eax.wrapping_add(0x4)) as usize, (t4 as f32).to_bits());
    // 50b463 fstp dword ptr [eax + 8]
    w.d((eax.wrapping_add(0x8)) as usize, (t4 as f32).to_bits());
    // 50b466 mov eax, dword ptr [esp + 0xc]
    eax = s[26];
    // 50b46a fst dword ptr [eax]
    w.d((eax) as usize, (t2 as f32).to_bits());
    // 50b46c fst dword ptr [eax + 4]
    w.d((eax.wrapping_add(0x4)) as usize, (t2 as f32).to_bits());
    // 50b46f fstp dword ptr [eax + 8]
    w.d((eax.wrapping_add(0x8)) as usize, (t2 as f32).to_bits());
    // 50b472 pop ecx
    ecx = s[23];
    // 50b473 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_7685c0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    // 7685c0 fld dword ptr [ecx + 0xc]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 7685c3 fsub dword ptr [ecx + 4]
    let t2: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t3: f64 = t1 - t2;
    // 7685c6 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t3 as f32).to_bits());
    // 7685c9 fld dword ptr [ecx + 0x10]
    let t4: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 7685cc fsub dword ptr [ecx + 8]
    let t5: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    let t6: f64 = t4 - t5;
    // 7685cf fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t6 as f32).to_bits());
    // 7685d2 fild dword ptr [esp + 4]
    let t7: f64 = (s[25] as i32) as f64;
    // 7685d6 fstp dword ptr [esp + 4]
    s[25] = (t7 as f32).to_bits();
    // 7685da fld dword ptr [ecx + 0xc]
    let t8: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 7685dd fld dword ptr [esp + 4]
    let t9: f64 = f32::from_bits(s[25]) as f64;
    // 7685e1 fld st(0)
    // 7685e3 faddp st(2)
    let t10: f64 = t8 + t9;
    // 7685e5 fxch st(1)
    // 7685e7 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t10 as f32).to_bits());
    // 7685ea fstp dword ptr [ecx + 4]
    w.d((ecx.wrapping_add(0x4)) as usize, (t9 as f32).to_bits());
    // 7685ed fild dword ptr [esp + 8]
    let t11: f64 = (s[26] as i32) as f64;
    // 7685f1 fstp dword ptr [esp + 4]
    s[25] = (t11 as f32).to_bits();
    // 7685f5 fld dword ptr [ecx + 0x10]
    let t12: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 7685f8 fld dword ptr [esp + 4]
    let t13: f64 = f32::from_bits(s[25]) as f64;
    // 7685fc fld st(0)
    // 7685fe faddp st(2)
    let t14: f64 = t12 + t13;
    // 768600 fxch st(1)
    // 768602 fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t14 as f32).to_bits());
    // 768605 fstp dword ptr [ecx + 8]
    w.d((ecx.wrapping_add(0x8)) as usize, (t13 as f32).to_bits());
    // 768608 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_768a70(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    // 768a70 fld dword ptr [ecx + 0xc]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 768a73 fsub dword ptr [ecx + 4]
    let t2: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t3: f64 = t1 - t2;
    // 768a76 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t3 as f32).to_bits());
    // 768a79 fld dword ptr [ecx + 0x10]
    let t4: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 768a7c fsub dword ptr [ecx + 8]
    let t5: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    let t6: f64 = t4 - t5;
    // 768a7f fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t6 as f32).to_bits());
    // 768a82 fild dword ptr [esp + 4]
    let t7: f64 = (s[25] as i32) as f64;
    // 768a86 fstp dword ptr [esp + 4]
    s[25] = (t7 as f32).to_bits();
    // 768a8a fld dword ptr [ecx + 0xc]
    let t8: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 768a8d fld dword ptr [esp + 4]
    let t9: f64 = f32::from_bits(s[25]) as f64;
    // 768a91 fld st(0)
    // 768a93 faddp st(2)
    let t10: f64 = t8 + t9;
    // 768a95 fxch st(1)
    // 768a97 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t10 as f32).to_bits());
    // 768a9a fstp dword ptr [ecx + 4]
    w.d((ecx.wrapping_add(0x4)) as usize, (t9 as f32).to_bits());
    // 768a9d fild dword ptr [esp + 8]
    let t11: f64 = (s[26] as i32) as f64;
    // 768aa1 fstp dword ptr [esp + 4]
    s[25] = (t11 as f32).to_bits();
    // 768aa5 fld dword ptr [ecx + 0x10]
    let t12: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 768aa8 fld dword ptr [esp + 4]
    let t13: f64 = f32::from_bits(s[25]) as f64;
    // 768aac fld st(0)
    // 768aae faddp st(2)
    let t14: f64 = t12 + t13;
    // 768ab0 fxch st(1)
    // 768ab2 fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t14 as f32).to_bits());
    // 768ab5 fstp dword ptr [ecx + 8]
    w.d((ecx.wrapping_add(0x8)) as usize, (t13 as f32).to_bits());
    // 768ab8 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_768ea0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    // 768ea0 fld dword ptr [ecx + 0xc]
    let t1: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 768ea3 fsub dword ptr [ecx + 4]
    let t2: f64 = f32::from_bits(rd((ecx.wrapping_add(0x4)) as usize)) as f64;
    let t3: f64 = t1 - t2;
    // 768ea6 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t3 as f32).to_bits());
    // 768ea9 fld dword ptr [ecx + 0x10]
    let t4: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 768eac fsub dword ptr [ecx + 8]
    let t5: f64 = f32::from_bits(rd((ecx.wrapping_add(0x8)) as usize)) as f64;
    let t6: f64 = t4 - t5;
    // 768eaf fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t6 as f32).to_bits());
    // 768eb2 fild dword ptr [esp + 4]
    let t7: f64 = (s[25] as i32) as f64;
    // 768eb6 fstp dword ptr [esp + 4]
    s[25] = (t7 as f32).to_bits();
    // 768eba fld dword ptr [ecx + 0xc]
    let t8: f64 = f32::from_bits(rd((ecx.wrapping_add(0xc)) as usize)) as f64;
    // 768ebd fld dword ptr [esp + 4]
    let t9: f64 = f32::from_bits(s[25]) as f64;
    // 768ec1 fld st(0)
    // 768ec3 faddp st(2)
    let t10: f64 = t8 + t9;
    // 768ec5 fxch st(1)
    // 768ec7 fstp dword ptr [ecx + 0xc]
    w.d((ecx.wrapping_add(0xc)) as usize, (t10 as f32).to_bits());
    // 768eca fstp dword ptr [ecx + 4]
    w.d((ecx.wrapping_add(0x4)) as usize, (t9 as f32).to_bits());
    // 768ecd fild dword ptr [esp + 8]
    let t11: f64 = (s[26] as i32) as f64;
    // 768ed1 fstp dword ptr [esp + 4]
    s[25] = (t11 as f32).to_bits();
    // 768ed5 fld dword ptr [ecx + 0x10]
    let t12: f64 = f32::from_bits(rd((ecx.wrapping_add(0x10)) as usize)) as f64;
    // 768ed8 fld dword ptr [esp + 4]
    let t13: f64 = f32::from_bits(s[25]) as f64;
    // 768edc fld st(0)
    // 768ede faddp st(2)
    let t14: f64 = t12 + t13;
    // 768ee0 fxch st(1)
    // 768ee2 fstp dword ptr [ecx + 0x10]
    w.d((ecx.wrapping_add(0x10)) as usize, (t14 as f32).to_bits());
    // 768ee5 fstp dword ptr [ecx + 8]
    w.d((ecx.wrapping_add(0x8)) as usize, (t13 as f32).to_bits());
    // 768ee8 ret 8
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    0.0
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9dd80(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9dd80 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9dd84 fmul dword ptr [esp + 0xc]
    let t2: f64 = f32::from_bits(s[27]) as f64;
    let t3: f64 = t1 * t2;
    // b9dd88 fadd dword ptr [esp + 8]
    let t4: f64 = f32::from_bits(s[26]) as f64;
    let t5: f64 = t3 + t4;
    // b9dd8c fstp dword ptr [esp + 4]
    s[25] = (t5 as f32).to_bits();
    // b9dd90 fld dword ptr [esp + 4]
    let t6: f64 = f32::from_bits(s[25]) as f64;
    // b9dd94 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t6
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9df70(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9df70 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9df74 fld st(0)
    // b9df76 fmul dword ptr [esp + 0xc]
    let t2: f64 = f32::from_bits(s[27]) as f64;
    let t3: f64 = t1 * t2;
    // b9df7a fmulp st(1)
    let t4: f64 = t1 * t3;
    // b9df7c fadd dword ptr [esp + 8]
    let t5: f64 = f32::from_bits(s[26]) as f64;
    let t6: f64 = t4 + t5;
    // b9df80 fstp dword ptr [esp + 4]
    s[25] = (t6 as f32).to_bits();
    // b9df84 fld dword ptr [esp + 4]
    let t7: f64 = f32::from_bits(s[25]) as f64;
    // b9df88 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t7
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9df90(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9df90 fld dword ptr [esp + 8]
    let t1: f64 = f32::from_bits(s[26]) as f64;
    // b9df94 fld dword ptr [esp + 4]
    let t2: f64 = f32::from_bits(s[25]) as f64;
    // b9df98 fld st(0)
    // b9df9a fmul dword ptr [esp + 0xc]
    let t3: f64 = f32::from_bits(s[27]) as f64;
    let t4: f64 = t2 * t3;
    // b9df9e fxch st(1)
    // b9dfa0 fsub qword ptr [0xe5bfe8]
    let t5: f64 = f64::from_bits(q64(rq(0xe5bfe8usize)));
    let t6: f64 = t2 - t5;
    // b9dfa6 fmulp st(1)
    let t7: f64 = t4 * t6;
    // b9dfa8 fsubp st(1)
    let t8: f64 = t1 - t7;
    // b9dfaa fstp dword ptr [esp + 8]
    s[26] = (t8 as f32).to_bits();
    // b9dfae fld dword ptr [esp + 8]
    let t9: f64 = f32::from_bits(s[26]) as f64;
    // b9dfb2 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t9
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e030(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e030 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e034 fld st(0)
    // b9e036 fmul dword ptr [esp + 0xc]
    let t2: f64 = f32::from_bits(s[27]) as f64;
    let t3: f64 = t1 * t2;
    // b9e03a fmul st(1)
    let t4: f64 = t3 * t1;
    // b9e03c fmulp st(1)
    let t5: f64 = t1 * t4;
    // b9e03e fadd dword ptr [esp + 8]
    let t6: f64 = f32::from_bits(s[26]) as f64;
    let t7: f64 = t5 + t6;
    // b9e042 fstp dword ptr [esp + 4]
    s[25] = (t7 as f32).to_bits();
    // b9e046 fld dword ptr [esp + 4]
    let t8: f64 = f32::from_bits(s[25]) as f64;
    // b9e04a ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t8
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e050(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e050 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e054 fsub qword ptr [0xe5bf40]
    let t2: f64 = f64::from_bits(q64(rq(0xe5bf40usize)));
    let t3: f64 = t1 - t2;
    // b9e05a fstp dword ptr [esp + 4]
    s[25] = (t3 as f32).to_bits();
    // b9e05e fld dword ptr [esp + 4]
    let t4: f64 = f32::from_bits(s[25]) as f64;
    // b9e062 fld st(0)
    // b9e064 fmul dword ptr [esp + 0xc]
    let t5: f64 = f32::from_bits(s[27]) as f64;
    let t6: f64 = t4 * t5;
    // b9e068 fmul st(1)
    let t7: f64 = t6 * t4;
    // b9e06a fmulp st(1)
    let t8: f64 = t4 * t7;
    // b9e06c fadd dword ptr [esp + 8]
    let t9: f64 = f32::from_bits(s[26]) as f64;
    let t10: f64 = t8 + t9;
    // b9e070 fstp dword ptr [esp + 4]
    s[25] = (t10 as f32).to_bits();
    // b9e074 fld dword ptr [esp + 4]
    let t11: f64 = f32::from_bits(s[25]) as f64;
    // b9e078 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t11
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e100(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e100 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e104 fld st(0)
    // b9e106 fmul dword ptr [esp + 0xc]
    let t2: f64 = f32::from_bits(s[27]) as f64;
    let t3: f64 = t1 * t2;
    // b9e10a fmul st(1)
    let t4: f64 = t3 * t1;
    // b9e10c fmul st(1)
    let t5: f64 = t4 * t1;
    // b9e10e fmulp st(1)
    let t6: f64 = t1 * t5;
    // b9e110 fadd dword ptr [esp + 8]
    let t7: f64 = f32::from_bits(s[26]) as f64;
    let t8: f64 = t6 + t7;
    // b9e114 fstp dword ptr [esp + 4]
    s[25] = (t8 as f32).to_bits();
    // b9e118 fld dword ptr [esp + 4]
    let t9: f64 = f32::from_bits(s[25]) as f64;
    // b9e11c ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t9
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e120(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e120 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e124 fld1 
    let t2: f64 = 1.0;
    // b9e126 fsub st(1), st(0)
    let t3: f64 = t1 - t2;
    // b9e128 fxch st(1)
    // b9e12a fstp dword ptr [esp + 4]
    s[25] = (t3 as f32).to_bits();
    // b9e12e fld dword ptr [esp + 4]
    let t4: f64 = f32::from_bits(s[25]) as f64;
    // b9e132 fld st(0)
    // b9e134 fld st(0)
    // b9e136 fmulp st(1)
    let t5: f64 = t4 * t4;
    // b9e138 fld st(1)
    // b9e13a fmulp st(1)
    let t6: f64 = t5 * t4;
    // b9e13c fmulp st(1)
    let t7: f64 = t4 * t6;
    // b9e13e fsubrp st(1)
    let t8: f64 = t7 - t2;
    // b9e140 fld dword ptr [esp + 0xc]
    let t9: f64 = f32::from_bits(s[27]) as f64;
    // b9e144 fmulp st(1)
    let t10: f64 = t8 * t9;
    // b9e146 fsubr dword ptr [esp + 8]
    let t11: f64 = f32::from_bits(s[26]) as f64;
    let t12: f64 = t11 - t10;
    // b9e14a fstp dword ptr [esp + 4]
    s[25] = (t12 as f32).to_bits();
    // b9e14e fld dword ptr [esp + 4]
    let t13: f64 = f32::from_bits(s[25]) as f64;
    // b9e152 ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t13
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e1e0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e1e0 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e1e4 fld st(0)
    // b9e1e6 fmul dword ptr [esp + 0xc]
    let t2: f64 = f32::from_bits(s[27]) as f64;
    let t3: f64 = t1 * t2;
    // b9e1ea fmul st(1)
    let t4: f64 = t3 * t1;
    // b9e1ec fmul st(1)
    let t5: f64 = t4 * t1;
    // b9e1ee fmul st(1)
    let t6: f64 = t5 * t1;
    // b9e1f0 fmulp st(1)
    let t7: f64 = t1 * t6;
    // b9e1f2 fadd dword ptr [esp + 8]
    let t8: f64 = f32::from_bits(s[26]) as f64;
    let t9: f64 = t7 + t8;
    // b9e1f6 fstp dword ptr [esp + 4]
    s[25] = (t9 as f32).to_bits();
    // b9e1fa fld dword ptr [esp + 4]
    let t10: f64 = f32::from_bits(s[25]) as f64;
    // b9e1fe ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t10
}

#[allow(unused_assignments, unused_mut, unused_variables, clippy::needless_late_init)]
fn g_b9e200(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    // b9e200 fld dword ptr [esp + 4]
    let t1: f64 = f32::from_bits(s[25]) as f64;
    // b9e204 fld1 
    let t2: f64 = 1.0;
    // b9e206 fsub st(1), st(0)
    let t3: f64 = t1 - t2;
    // b9e208 fxch st(1)
    // b9e20a fstp dword ptr [esp + 4]
    s[25] = (t3 as f32).to_bits();
    // b9e20e fld dword ptr [esp + 4]
    let t4: f64 = f32::from_bits(s[25]) as f64;
    // b9e212 fld st(0)
    // b9e214 fmul st(0), st(0)
    let t5: f64 = t4 * t4;
    // b9e216 fmul st(1)
    let t6: f64 = t5 * t4;
    // b9e218 fmul st(1)
    let t7: f64 = t6 * t4;
    // b9e21a fmulp st(1)
    let t8: f64 = t4 * t7;
    // b9e21c faddp st(1)
    let t9: f64 = t2 + t8;
    // b9e21e fmul dword ptr [esp + 0xc]
    let t10: f64 = f32::from_bits(s[27]) as f64;
    let t11: f64 = t9 * t10;
    // b9e222 fadd dword ptr [esp + 8]
    let t12: f64 = f32::from_bits(s[26]) as f64;
    let t13: f64 = t11 + t12;
    // b9e226 fstp dword ptr [esp + 4]
    s[25] = (t13 as f32).to_bits();
    // b9e22a fld dword ptr [esp + 4]
    let t14: f64 = f32::from_bits(s[25]) as f64;
    // b9e22e ret 
    (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp) = (eax, ecx, edx, ebx, esi, edi, ebp);
    t14
}

// --- helpers for the branching ports (two-exit leaves, ease functions) -----------------------------

/// The integer flags a branch reads.
#[derive(Clone, Copy, Default)]
#[allow(dead_code)] // CF is only read by `jb` / `jae` style branches
struct Fl {
    zf: bool,
    sf: bool,
    cf: bool,
    of: bool,
    pf: bool,
}

fn width_mask(bits: u32) -> u32 {
    if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 }
}

/// PF: the low byte has an even number of set bits.
fn parity(r: u32) -> bool {
    (r as u8).count_ones() % 2 == 0
}

/// `and` / `or` / `xor` / `test` flags of the result `r` at `bits` width (CF = OF = 0).
fn fl_logic(r: u32, bits: u32) -> Fl {
    let r = r & width_mask(bits);
    Fl { zf: r == 0, sf: (r >> (bits - 1)) & 1 != 0, cf: false, of: false, pf: parity(r) }
}

/// `add`: the result and the flags.
#[allow(dead_code)]
fn fl_add(a: u32, b: u32, bits: u32) -> (u32, Fl) {
    let m = width_mask(bits);
    let (a, b) = (a & m, b & m);
    let r = a.wrapping_add(b) & m;
    let sb = 1u32 << (bits - 1);
    let fl = Fl {
        zf: r == 0,
        sf: r & sb != 0,
        cf: a as u64 + b as u64 > m as u64,
        of: (!(a ^ b) & (a ^ r)) & sb != 0,
        pf: parity(r),
    };
    (r, fl)
}

/// `sub` / `cmp` / `neg`: the result and the flags (CF = unsigned borrow).
fn fl_sub(a: u32, b: u32, bits: u32) -> (u32, Fl) {
    let m = width_mask(bits);
    let (a, b) = (a & m, b & m);
    let r = a.wrapping_sub(b) & m;
    let sb = 1u32 << (bits - 1);
    let fl = Fl { zf: r == 0, sf: r & sb != 0, cf: a < b, of: ((a ^ b) & (a ^ r)) & sb != 0, pf: parity(r) };
    (r, fl)
}

/// `shl` / `shr` / `sar` by an immediate `n` (1..bits): the result and the flags (OF as for a
/// shift by one).
fn fl_shift(kind: &str, a: u32, n: u32, bits: u32) -> (u32, Fl) {
    let m = width_mask(bits);
    let a = a & m;
    let sb = 1u32 << (bits - 1);
    let (r, cf) = match kind {
        "shl" => (a.checked_shl(n).unwrap_or(0) & m, (a >> (bits - n)) & 1 != 0),
        "shr" => (a >> n, (a >> (n - 1)) & 1 != 0),
        _ => {
            let sa = ((a << (32 - bits)) as i32) >> (32 - bits);
            (((sa >> n.min(31)) as u32) & m, ((sa >> (n - 1).min(31)) as u32) & 1 != 0)
        }
    };
    let of = match kind {
        "shl" => (r & sb != 0) != cf,
        "shr" => a & sb != 0,
        _ => false,
    };
    (r, Fl { zf: r == 0, sf: r & sb != 0, cf, of, pf: parity(r) })
}

/// `fcomi` / `fucomi`: ZF / PF / CF from comparing `a` (ST0) with `b`.
#[allow(dead_code)]
fn fl_fcomi(a: f64, b: f64) -> Fl {
    if a.is_nan() || b.is_nan() {
        Fl { zf: true, pf: true, cf: true, ..Fl::default() }
    } else {
        Fl { zf: a == b, cf: a < b, ..Fl::default() }
    }
}

/// Replace the bits `mask << shift` of `reg` with the low bits of `v` (a write to AL / AH / AX).
#[inline(never)]
fn set_part(reg: u32, shift: u32, mask: u32, v: u32) -> u32 {
    (reg & !(mask << shift)) | ((v & mask) << shift)
}

/// `fld dword`: a signalling NaN becomes quiet, as on the FPU (a plain `as f64` can be folded
/// away together with the `as f32` of the matching `fstp dword`).
fn ldq(bits: u32) -> f64 {
    f32::from_bits(q32(bits)) as f64
}

/// The result `r` of an x87 add / sub / mul / div of `a` and `b`: with two NaN operands the FPU
/// returns the one with the larger significand (SSE returns the first operand).
fn xn(a: f64, b: f64, r: f64) -> f64 {
    if a.is_nan() && b.is_nan() {
        const SIG: u64 = 0x000F_FFFF_FFFF_FFFF;
        if (b.to_bits() & SIG) > (a.to_bits() & SIG) { b } else { a }
    } else {
        r
    }
}

/// A word read of game memory.
#[allow(dead_code)]
fn rw(a: usize) -> u16 {
    unsafe { std::ptr::read_unaligned(a as *const u16) }
}

/// The registers, the stack argument and the x87 values of a call into the CRT, and what comes
/// back. Offsets used by the asm: f 0, eax 4, ecx 8, edx 12, arg 16, out 20, st1 28, st0 36.
#[repr(C)]
struct CrtBlock {
    f: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    arg: u32,
    out: [u32; 2],
    in1: [u32; 2],
    in0: [u32; 2],
}

/// Call the CRT function `f` (cdecl, one dword on the stack, result in ST0) with these EAX /
/// ECX / EDX; returns the three registers and ST0 as the original code would see them.
fn crt_call1(f: usize, arg: u32, eax: u32, ecx: u32, edx: u32) -> (u32, u32, u32, f64) {
    let mut b = CrtBlock { f: f as u32, eax, ecx, edx, arg, out: [0; 2], in1: [0; 2], in0: [0; 2] };
    unsafe {
        std::arch::asm!(
            "push ebx",
            "mov ebx, eax",
            "push dword ptr [ebx + 16]",
            "mov ecx, [ebx + 8]",
            "mov edx, [ebx + 12]",
            "mov eax, [ebx + 4]",
            "call dword ptr [ebx]",
            "add esp, 4",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "fstp qword ptr [ebx + 20]",
            "pop ebx",
            inout("eax") &mut b as *mut CrtBlock => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    (b.eax, b.ecx, b.edx, f64::from_bits((b.out[1] as u64) << 32 | b.out[0] as u64))
}

/// Call `crt_pow_x87` (0xCA05D0): `st1 ^ st0` on the x87 stack, result in ST0, both inputs
/// consumed. Returns the three registers and the result.
fn crt_pow(eax: u32, ecx: u32, edx: u32, st1: f64, st0: f64) -> (u32, u32, u32, f64) {
    let mut b = CrtBlock {
        f: 0xca05d0,
        eax,
        ecx,
        edx,
        arg: 0,
        out: [0; 2],
        in1: [st1.to_bits() as u32, (st1.to_bits() >> 32) as u32],
        in0: [st0.to_bits() as u32, (st0.to_bits() >> 32) as u32],
    };
    unsafe {
        std::arch::asm!(
            "push ebx",
            "mov ebx, eax",
            "fld qword ptr [ebx + 28]",
            "fld qword ptr [ebx + 36]",
            "mov ecx, [ebx + 8]",
            "mov edx, [ebx + 12]",
            "mov eax, [ebx + 4]",
            "call dword ptr [ebx]",
            "mov [ebx + 4], eax",
            "mov [ebx + 8], ecx",
            "mov [ebx + 12], edx",
            "fstp qword ptr [ebx + 20]",
            "pop ebx",
            inout("eax") &mut b as *mut CrtBlock => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    (b.eax, b.ecx, b.edx, f64::from_bits((b.out[1] as u64) << 32 | b.out[0] as u64))
}

// --- Rust bodies: two-exit leaves and ease functions (translated one per exe function) ----------

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b96520(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb96520u32;
    loop {
        match pc {
            0xb96520 => {
                // b96520 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b96524 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b96526 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b96528 mov cl, byte ptr [ecx + 0x59]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0x59)) as usize) as u32));
                // b9652b je 0xb9653a
                pc = if fl.zf { 0xb9653a } else { 0xb9652d };
            }
            0xb9652d => {
                // b9652d cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // b96530 jne 0xb96543
                pc = if !fl.zf { 0xb96543 } else { 0xb96532 };
            }
            0xb96532 => {
                // b96532 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // b96535 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96537 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9653a => {
                // b9653a mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // b96540 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xb96543;
            }
            0xb96543 => {
                // b96543 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96545 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b965b0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb965b0u32;
    loop {
        match pc {
            0xb965b0 => {
                // b965b0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b965b4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b965b6 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b965b8 mov cl, byte ptr [ecx + 0x58]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0x58)) as usize) as u32));
                // b965bb je 0xb965ca
                pc = if fl.zf { 0xb965ca } else { 0xb965bd };
            }
            0xb965bd => {
                // b965bd cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // b965c0 jne 0xb965d3
                pc = if !fl.zf { 0xb965d3 } else { 0xb965c2 };
            }
            0xb965c2 => {
                // b965c2 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // b965c5 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b965c7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb965ca => {
                // b965ca mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // b965d0 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xb965d3;
            }
            0xb965d3 => {
                // b965d3 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b965d5 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b96860(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb96860u32;
    loop {
        match pc {
            0xb96860 => {
                // b96860 push ecx
                s[23] = ecx;
                // b96861 fld dword ptr [ecx + 0x88]
                f[0] = ldq(rd((ecx.wrapping_add(0x88)) as usize));
                // b96867 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b9686b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b9686d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b96870 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b96872 je 0xb96885
                pc = if fl.zf { 0xb96885 } else { 0xb96874 };
            }
            0xb96874 => {
                // b96874 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b96877 jne 0xb96891
                pc = if !fl.zf { 0xb96891 } else { 0xb96879 };
            }
            0xb96879 => {
                // b96879 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9687c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9687e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b96881 pop ecx
                ecx = s[23];
                // b96882 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96885 => {
                // b96885 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b96888 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b9688e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb96891;
            }
            0xb96891 => {
                // b96891 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96893 pop ecx
                ecx = s[23];
                // b96894 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b968c0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb968c0u32;
    loop {
        match pc {
            0xb968c0 => {
                // b968c0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b968c4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b968c6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b968c9 je 0xb96923
                pc = if fl.zf { 0xb96923 } else { 0xb968cb };
            }
            0xb968cb => {
                // b968cb sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b968ce je 0xb96900
                pc = if fl.zf { 0xb96900 } else { 0xb968d0 };
            }
            0xb968d0 => {
                // b968d0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b968d3 je 0xb968ea
                pc = if fl.zf { 0xb968ea } else { 0xb968d5 };
            }
            0xb968d5 => {
                // b968d5 fldz 
                f[0] = 0.0;
                // b968d7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b968d9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b968dd fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b968e1 fstp dword ptr [ecx + 0x88]
                w.d((ecx.wrapping_add(0x88)) as usize, (f[0] as f32).to_bits());
                // b968e7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb968ea => {
                // b968ea fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // b968ed mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b968ef fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b968f3 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b968f7 fstp dword ptr [ecx + 0x88]
                w.d((ecx.wrapping_add(0x88)) as usize, (f[0] as f32).to_bits());
                // b968fd ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96900 => {
                // b96900 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // b96903 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // b96906 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b96908 jge 0xb96926
                pc = if (fl.sf == fl.of) { 0xb96926 } else { 0xb9690a };
            }
            0xb9690a => {
                // b9690a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // b96910 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96912 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b96916 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9691a fstp dword ptr [ecx + 0x88]
                w.d((ecx.wrapping_add(0x88)) as usize, (f[0] as f32).to_bits());
                // b96920 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96923 => {
                // b96923 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xb96926;
            }
            0xb96926 => {
                // b96926 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9692a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9692c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b96930 fstp dword ptr [ecx + 0x88]
                w.d((ecx.wrapping_add(0x88)) as usize, (f[0] as f32).to_bits());
                // b96936 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b96dc0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb96dc0u32;
    loop {
        match pc {
            0xb96dc0 => {
                // b96dc0 push ecx
                s[23] = ecx;
                // b96dc1 fld dword ptr [ecx + 0x78]
                f[0] = ldq(rd((ecx.wrapping_add(0x78)) as usize));
                // b96dc4 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b96dc8 mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b96dca fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b96dcd test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b96dcf je 0xb96de2
                pc = if fl.zf { 0xb96de2 } else { 0xb96dd1 };
            }
            0xb96dd1 => {
                // b96dd1 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b96dd4 jne 0xb96dee
                pc = if !fl.zf { 0xb96dee } else { 0xb96dd6 };
            }
            0xb96dd6 => {
                // b96dd6 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b96dd9 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96ddb fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b96dde pop ecx
                ecx = s[23];
                // b96ddf ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96de2 => {
                // b96de2 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b96de5 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b96deb fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb96dee;
            }
            0xb96dee => {
                // b96dee mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96df0 pop ecx
                ecx = s[23];
                // b96df1 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b96e20(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb96e20u32;
    loop {
        match pc {
            0xb96e20 => {
                // b96e20 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b96e24 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b96e26 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b96e29 je 0xb96e7a
                pc = if fl.zf { 0xb96e7a } else { 0xb96e2b };
            }
            0xb96e2b => {
                // b96e2b sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b96e2e je 0xb96e5a
                pc = if fl.zf { 0xb96e5a } else { 0xb96e30 };
            }
            0xb96e30 => {
                // b96e30 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b96e33 je 0xb96e47
                pc = if fl.zf { 0xb96e47 } else { 0xb96e35 };
            }
            0xb96e35 => {
                // b96e35 fldz 
                f[0] = 0.0;
                // b96e37 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96e39 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b96e3d fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b96e41 fstp dword ptr [ecx + 0x78]
                w.d((ecx.wrapping_add(0x78)) as usize, (f[0] as f32).to_bits());
                // b96e44 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96e47 => {
                // b96e47 fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // b96e4a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96e4c fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b96e50 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b96e54 fstp dword ptr [ecx + 0x78]
                w.d((ecx.wrapping_add(0x78)) as usize, (f[0] as f32).to_bits());
                // b96e57 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96e5a => {
                // b96e5a mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // b96e5d fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // b96e60 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b96e62 jge 0xb96e7d
                pc = if (fl.sf == fl.of) { 0xb96e7d } else { 0xb96e64 };
            }
            0xb96e64 => {
                // b96e64 fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // b96e6a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96e6c fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b96e70 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b96e74 fstp dword ptr [ecx + 0x78]
                w.d((ecx.wrapping_add(0x78)) as usize, (f[0] as f32).to_bits());
                // b96e77 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb96e7a => {
                // b96e7a fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xb96e7d;
            }
            0xb96e7d => {
                // b96e7d fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b96e81 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b96e83 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b96e87 fstp dword ptr [ecx + 0x78]
                w.d((ecx.wrapping_add(0x78)) as usize, (f[0] as f32).to_bits());
                // b96e8a ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9cf10(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9cf10u32;
    loop {
        match pc {
            0xb9cf10 => {
                // b9cf10 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b9cf14 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b9cf16 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9cf19 je 0xb9cf73
                pc = if fl.zf { 0xb9cf73 } else { 0xb9cf1b };
            }
            0xb9cf1b => {
                // b9cf1b sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9cf1e je 0xb9cf50
                pc = if fl.zf { 0xb9cf50 } else { 0xb9cf20 };
            }
            0xb9cf20 => {
                // b9cf20 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9cf23 je 0xb9cf3a
                pc = if fl.zf { 0xb9cf3a } else { 0xb9cf25 };
            }
            0xb9cf25 => {
                // b9cf25 fldz 
                f[0] = 0.0;
                // b9cf27 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cf29 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9cf2d fld dword ptr [ecx + 0x5c]
                f[0] = ldq(rd((ecx.wrapping_add(0x5c)) as usize));
                // b9cf30 fadd dword ptr [esp + 4]
                f[0] = xn(f[0], ldq(s[25]), f[0] + ldq(s[25]));
                // b9cf34 fstp dword ptr [ecx + 0x54]
                w.d((ecx.wrapping_add(0x54)) as usize, (f[0] as f32).to_bits());
                // b9cf37 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9cf3a => {
                // b9cf3a fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // b9cf3d mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cf3f fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9cf43 fld dword ptr [ecx + 0x5c]
                f[0] = ldq(rd((ecx.wrapping_add(0x5c)) as usize));
                // b9cf46 fadd dword ptr [esp + 4]
                f[0] = xn(f[0], ldq(s[25]), f[0] + ldq(s[25]));
                // b9cf4a fstp dword ptr [ecx + 0x54]
                w.d((ecx.wrapping_add(0x54)) as usize, (f[0] as f32).to_bits());
                // b9cf4d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9cf50 => {
                // b9cf50 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // b9cf53 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // b9cf56 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9cf58 jge 0xb9cf76
                pc = if (fl.sf == fl.of) { 0xb9cf76 } else { 0xb9cf5a };
            }
            0xb9cf5a => {
                // b9cf5a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // b9cf60 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cf62 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9cf66 fld dword ptr [ecx + 0x5c]
                f[0] = ldq(rd((ecx.wrapping_add(0x5c)) as usize));
                // b9cf69 fadd dword ptr [esp + 4]
                f[0] = xn(f[0], ldq(s[25]), f[0] + ldq(s[25]));
                // b9cf6d fstp dword ptr [ecx + 0x54]
                w.d((ecx.wrapping_add(0x54)) as usize, (f[0] as f32).to_bits());
                // b9cf70 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9cf73 => {
                // b9cf73 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xb9cf76;
            }
            0xb9cf76 => {
                // b9cf76 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9cf7a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cf7c fld dword ptr [ecx + 0x5c]
                f[0] = ldq(rd((ecx.wrapping_add(0x5c)) as usize));
                // b9cf7f fadd dword ptr [esp + 4]
                f[0] = xn(f[0], ldq(s[25]), f[0] + ldq(s[25]));
                // b9cf83 fstp dword ptr [ecx + 0x54]
                w.d((ecx.wrapping_add(0x54)) as usize, (f[0] as f32).to_bits());
                // b9cf86 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9cfb0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9cfb0u32;
    loop {
        match pc {
            0xb9cfb0 => {
                // b9cfb0 push ecx
                s[23] = ecx;
                // b9cfb1 fld dword ptr [ecx + 0x5c]
                f[0] = ldq(rd((ecx.wrapping_add(0x5c)) as usize));
                // b9cfb4 fadd dword ptr [ecx + 0x54]
                f[0] = xn(f[0], ldq(rd((ecx.wrapping_add(0x54)) as usize)), f[0] + ldq(rd((ecx.wrapping_add(0x54)) as usize)));
                // b9cfb7 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b9cfbb mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b9cfbd test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9cfbf fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9cfc2 je 0xb9cfd5
                pc = if fl.zf { 0xb9cfd5 } else { 0xb9cfc4 };
            }
            0xb9cfc4 => {
                // b9cfc4 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b9cfc7 jne 0xb9cfe1
                pc = if !fl.zf { 0xb9cfe1 } else { 0xb9cfc9 };
            }
            0xb9cfc9 => {
                // b9cfc9 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9cfcc mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cfce fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b9cfd1 pop ecx
                ecx = s[23];
                // b9cfd2 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9cfd5 => {
                // b9cfd5 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9cfd8 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b9cfde fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb9cfe1;
            }
            0xb9cfe1 => {
                // b9cfe1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9cfe3 pop ecx
                ecx = s[23];
                // b9cfe4 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9d160(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9d160u32;
    loop {
        match pc {
            0xb9d160 => {
                // b9d160 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b9d164 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b9d166 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9d168 mov cl, byte ptr [ecx + 0x58]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0x58)) as usize) as u32));
                // b9d16b je 0xb9d17a
                pc = if fl.zf { 0xb9d17a } else { 0xb9d16d };
            }
            0xb9d16d => {
                // b9d16d cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // b9d170 jne 0xb9d183
                pc = if !fl.zf { 0xb9d183 } else { 0xb9d172 };
            }
            0xb9d172 => {
                // b9d172 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // b9d175 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9d177 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9d17a => {
                // b9d17a mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // b9d180 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xb9d183;
            }
            0xb9d183 => {
                // b9d183 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9d185 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9f670(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9f670u32;
    loop {
        match pc {
            0xb9f670 => {
                // b9f670 push ecx
                s[23] = ecx;
                // b9f671 fld dword ptr [ecx + 0x7c]
                f[0] = ldq(rd((ecx.wrapping_add(0x7c)) as usize));
                // b9f674 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b9f678 mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b9f67a fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9f67d test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9f67f je 0xb9f692
                pc = if fl.zf { 0xb9f692 } else { 0xb9f681 };
            }
            0xb9f681 => {
                // b9f681 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b9f684 jne 0xb9f69e
                pc = if !fl.zf { 0xb9f69e } else { 0xb9f686 };
            }
            0xb9f686 => {
                // b9f686 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9f689 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9f68b fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b9f68e pop ecx
                ecx = s[23];
                // b9f68f ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9f692 => {
                // b9f692 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9f695 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b9f69b fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb9f69e;
            }
            0xb9f69e => {
                // b9f69e mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9f6a0 pop ecx
                ecx = s[23];
                // b9f6a1 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9f760(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9f760u32;
    loop {
        match pc {
            0xb9f760 => {
                // b9f760 push ecx
                s[23] = ecx;
                // b9f761 fld dword ptr [ecx + 0x74]
                f[0] = ldq(rd((ecx.wrapping_add(0x74)) as usize));
                // b9f764 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b9f768 mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b9f76a fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9f76d test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9f76f je 0xb9f782
                pc = if fl.zf { 0xb9f782 } else { 0xb9f771 };
            }
            0xb9f771 => {
                // b9f771 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b9f774 jne 0xb9f78e
                pc = if !fl.zf { 0xb9f78e } else { 0xb9f776 };
            }
            0xb9f776 => {
                // b9f776 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9f779 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9f77b fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b9f77e pop ecx
                ecx = s[23];
                // b9f77f ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9f782 => {
                // b9f782 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9f785 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b9f78b fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb9f78e;
            }
            0xb9f78e => {
                // b9f78e mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9f790 pop ecx
                ecx = s[23];
                // b9f791 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9fb00(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9fb00u32;
    loop {
        match pc {
            0xb9fb00 => {
                // b9fb00 push ecx
                s[23] = ecx;
                // b9fb01 fld dword ptr [ecx + 0xa0]
                f[0] = ldq(rd((ecx.wrapping_add(0xa0)) as usize));
                // b9fb07 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // b9fb0b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b9fb0d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9fb10 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9fb12 je 0xb9fb25
                pc = if fl.zf { 0xb9fb25 } else { 0xb9fb14 };
            }
            0xb9fb14 => {
                // b9fb14 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // b9fb17 jne 0xb9fb31
                pc = if !fl.zf { 0xb9fb31 } else { 0xb9fb19 };
            }
            0xb9fb19 => {
                // b9fb19 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9fb1c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fb1e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b9fb21 pop ecx
                ecx = s[23];
                // b9fb22 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9fb25 => {
                // b9fb25 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b9fb28 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // b9fb2e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xb9fb31;
            }
            0xb9fb31 => {
                // b9fb31 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fb33 pop ecx
                ecx = s[23];
                // b9fb34 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9fb60(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9fb60u32;
    loop {
        match pc {
            0xb9fb60 => {
                // b9fb60 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b9fb64 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // b9fb66 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9fb69 je 0xb9fbc3
                pc = if fl.zf { 0xb9fbc3 } else { 0xb9fb6b };
            }
            0xb9fb6b => {
                // b9fb6b sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9fb6e je 0xb9fba0
                pc = if fl.zf { 0xb9fba0 } else { 0xb9fb70 };
            }
            0xb9fb70 => {
                // b9fb70 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b9fb73 je 0xb9fb8a
                pc = if fl.zf { 0xb9fb8a } else { 0xb9fb75 };
            }
            0xb9fb75 => {
                // b9fb75 fldz 
                f[0] = 0.0;
                // b9fb77 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fb79 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9fb7d fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9fb81 fstp dword ptr [ecx + 0xa0]
                w.d((ecx.wrapping_add(0xa0)) as usize, (f[0] as f32).to_bits());
                // b9fb87 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9fb8a => {
                // b9fb8a fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // b9fb8d mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fb8f fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9fb93 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9fb97 fstp dword ptr [ecx + 0xa0]
                w.d((ecx.wrapping_add(0xa0)) as usize, (f[0] as f32).to_bits());
                // b9fb9d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9fba0 => {
                // b9fba0 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // b9fba3 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // b9fba6 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b9fba8 jge 0xb9fbc6
                pc = if (fl.sf == fl.of) { 0xb9fbc6 } else { 0xb9fbaa };
            }
            0xb9fbaa => {
                // b9fbaa fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // b9fbb0 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fbb2 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9fbb6 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9fbba fstp dword ptr [ecx + 0xa0]
                w.d((ecx.wrapping_add(0xa0)) as usize, (f[0] as f32).to_bits());
                // b9fbc0 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb9fbc3 => {
                // b9fbc3 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xb9fbc6;
            }
            0xb9fbc6 => {
                // b9fbc6 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9fbca mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // b9fbcc fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9fbd0 fstp dword ptr [ecx + 0xa0]
                w.d((ecx.wrapping_add(0xa0)) as usize, (f[0] as f32).to_bits());
                // b9fbd6 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba1400(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba1400u32;
    loop {
        match pc {
            0xba1400 => {
                // ba1400 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba1404 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba1406 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba1408 mov cl, byte ptr [ecx + 0xc0]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0xc0)) as usize) as u32));
                // ba140e je 0xba141d
                pc = if fl.zf { 0xba141d } else { 0xba1410 };
            }
            0xba1410 => {
                // ba1410 cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // ba1413 jne 0xba1426
                pc = if !fl.zf { 0xba1426 } else { 0xba1415 };
            }
            0xba1415 => {
                // ba1415 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // ba1418 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba141a ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba141d => {
                // ba141d mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // ba1423 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xba1426;
            }
            0xba1426 => {
                // ba1426 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba1428 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba1f70(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba1f70u32;
    loop {
        match pc {
            0xba1f70 => {
                // ba1f70 push ecx
                s[23] = ecx;
                // ba1f71 fld dword ptr [ecx + 0xf4]
                f[0] = ldq(rd((ecx.wrapping_add(0xf4)) as usize));
                // ba1f77 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba1f7b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba1f7d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba1f80 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba1f82 je 0xba1f95
                pc = if fl.zf { 0xba1f95 } else { 0xba1f84 };
            }
            0xba1f84 => {
                // ba1f84 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba1f87 jne 0xba1fa1
                pc = if !fl.zf { 0xba1fa1 } else { 0xba1f89 };
            }
            0xba1f89 => {
                // ba1f89 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba1f8c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba1f8e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba1f91 pop ecx
                ecx = s[23];
                // ba1f92 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba1f95 => {
                // ba1f95 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba1f98 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba1f9e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba1fa1;
            }
            0xba1fa1 => {
                // ba1fa1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba1fa3 pop ecx
                ecx = s[23];
                // ba1fa4 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba1fd0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba1fd0u32;
    loop {
        match pc {
            0xba1fd0 => {
                // ba1fd0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba1fd4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba1fd6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba1fd9 je 0xba2033
                pc = if fl.zf { 0xba2033 } else { 0xba1fdb };
            }
            0xba1fdb => {
                // ba1fdb sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba1fde je 0xba2010
                pc = if fl.zf { 0xba2010 } else { 0xba1fe0 };
            }
            0xba1fe0 => {
                // ba1fe0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba1fe3 je 0xba1ffa
                pc = if fl.zf { 0xba1ffa } else { 0xba1fe5 };
            }
            0xba1fe5 => {
                // ba1fe5 fldz 
                f[0] = 0.0;
                // ba1fe7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba1fe9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba1fed fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba1ff1 fstp dword ptr [ecx + 0xf4]
                w.d((ecx.wrapping_add(0xf4)) as usize, (f[0] as f32).to_bits());
                // ba1ff7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba1ffa => {
                // ba1ffa fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba1ffd mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba1fff fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2003 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2007 fstp dword ptr [ecx + 0xf4]
                w.d((ecx.wrapping_add(0xf4)) as usize, (f[0] as f32).to_bits());
                // ba200d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2010 => {
                // ba2010 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba2013 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba2016 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2018 jge 0xba2036
                pc = if (fl.sf == fl.of) { 0xba2036 } else { 0xba201a };
            }
            0xba201a => {
                // ba201a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba2020 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba2022 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2026 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba202a fstp dword ptr [ecx + 0xf4]
                w.d((ecx.wrapping_add(0xf4)) as usize, (f[0] as f32).to_bits());
                // ba2030 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2033 => {
                // ba2033 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba2036;
            }
            0xba2036 => {
                // ba2036 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba203a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba203c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2040 fstp dword ptr [ecx + 0xf4]
                w.d((ecx.wrapping_add(0xf4)) as usize, (f[0] as f32).to_bits());
                // ba2046 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba2070(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba2070u32;
    loop {
        match pc {
            0xba2070 => {
                // ba2070 push ecx
                s[23] = ecx;
                // ba2071 fld dword ptr [ecx + 0xf8]
                f[0] = ldq(rd((ecx.wrapping_add(0xf8)) as usize));
                // ba2077 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba207b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba207d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba2080 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2082 je 0xba2095
                pc = if fl.zf { 0xba2095 } else { 0xba2084 };
            }
            0xba2084 => {
                // ba2084 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba2087 jne 0xba20a1
                pc = if !fl.zf { 0xba20a1 } else { 0xba2089 };
            }
            0xba2089 => {
                // ba2089 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba208c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba208e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba2091 pop ecx
                ecx = s[23];
                // ba2092 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2095 => {
                // ba2095 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba2098 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba209e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba20a1;
            }
            0xba20a1 => {
                // ba20a1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba20a3 pop ecx
                ecx = s[23];
                // ba20a4 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba20d0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba20d0u32;
    loop {
        match pc {
            0xba20d0 => {
                // ba20d0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba20d4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba20d6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba20d9 je 0xba2133
                pc = if fl.zf { 0xba2133 } else { 0xba20db };
            }
            0xba20db => {
                // ba20db sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba20de je 0xba2110
                pc = if fl.zf { 0xba2110 } else { 0xba20e0 };
            }
            0xba20e0 => {
                // ba20e0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba20e3 je 0xba20fa
                pc = if fl.zf { 0xba20fa } else { 0xba20e5 };
            }
            0xba20e5 => {
                // ba20e5 fldz 
                f[0] = 0.0;
                // ba20e7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba20e9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba20ed fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba20f1 fstp dword ptr [ecx + 0xf8]
                w.d((ecx.wrapping_add(0xf8)) as usize, (f[0] as f32).to_bits());
                // ba20f7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba20fa => {
                // ba20fa fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba20fd mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba20ff fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2103 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2107 fstp dword ptr [ecx + 0xf8]
                w.d((ecx.wrapping_add(0xf8)) as usize, (f[0] as f32).to_bits());
                // ba210d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2110 => {
                // ba2110 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba2113 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba2116 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2118 jge 0xba2136
                pc = if (fl.sf == fl.of) { 0xba2136 } else { 0xba211a };
            }
            0xba211a => {
                // ba211a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba2120 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba2122 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2126 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba212a fstp dword ptr [ecx + 0xf8]
                w.d((ecx.wrapping_add(0xf8)) as usize, (f[0] as f32).to_bits());
                // ba2130 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2133 => {
                // ba2133 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba2136;
            }
            0xba2136 => {
                // ba2136 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba213a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba213c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2140 fstp dword ptr [ecx + 0xf8]
                w.d((ecx.wrapping_add(0xf8)) as usize, (f[0] as f32).to_bits());
                // ba2146 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba2170(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba2170u32;
    loop {
        match pc {
            0xba2170 => {
                // ba2170 push ecx
                s[23] = ecx;
                // ba2171 fld dword ptr [ecx + 0xfc]
                f[0] = ldq(rd((ecx.wrapping_add(0xfc)) as usize));
                // ba2177 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba217b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba217d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba2180 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2182 je 0xba2195
                pc = if fl.zf { 0xba2195 } else { 0xba2184 };
            }
            0xba2184 => {
                // ba2184 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba2187 jne 0xba21a1
                pc = if !fl.zf { 0xba21a1 } else { 0xba2189 };
            }
            0xba2189 => {
                // ba2189 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba218c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba218e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba2191 pop ecx
                ecx = s[23];
                // ba2192 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2195 => {
                // ba2195 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba2198 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba219e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba21a1;
            }
            0xba21a1 => {
                // ba21a1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba21a3 pop ecx
                ecx = s[23];
                // ba21a4 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba21d0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba21d0u32;
    loop {
        match pc {
            0xba21d0 => {
                // ba21d0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba21d4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba21d6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba21d9 je 0xba2233
                pc = if fl.zf { 0xba2233 } else { 0xba21db };
            }
            0xba21db => {
                // ba21db sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba21de je 0xba2210
                pc = if fl.zf { 0xba2210 } else { 0xba21e0 };
            }
            0xba21e0 => {
                // ba21e0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba21e3 je 0xba21fa
                pc = if fl.zf { 0xba21fa } else { 0xba21e5 };
            }
            0xba21e5 => {
                // ba21e5 fldz 
                f[0] = 0.0;
                // ba21e7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba21e9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba21ed fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba21f1 fstp dword ptr [ecx + 0xfc]
                w.d((ecx.wrapping_add(0xfc)) as usize, (f[0] as f32).to_bits());
                // ba21f7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba21fa => {
                // ba21fa fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba21fd mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba21ff fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2203 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2207 fstp dword ptr [ecx + 0xfc]
                w.d((ecx.wrapping_add(0xfc)) as usize, (f[0] as f32).to_bits());
                // ba220d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2210 => {
                // ba2210 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba2213 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba2216 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2218 jge 0xba2236
                pc = if (fl.sf == fl.of) { 0xba2236 } else { 0xba221a };
            }
            0xba221a => {
                // ba221a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba2220 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba2222 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2226 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba222a fstp dword ptr [ecx + 0xfc]
                w.d((ecx.wrapping_add(0xfc)) as usize, (f[0] as f32).to_bits());
                // ba2230 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2233 => {
                // ba2233 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba2236;
            }
            0xba2236 => {
                // ba2236 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba223a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba223c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2240 fstp dword ptr [ecx + 0xfc]
                w.d((ecx.wrapping_add(0xfc)) as usize, (f[0] as f32).to_bits());
                // ba2246 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba2270(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba2270u32;
    loop {
        match pc {
            0xba2270 => {
                // ba2270 push ecx
                s[23] = ecx;
                // ba2271 fld dword ptr [ecx + 0x100]
                f[0] = ldq(rd((ecx.wrapping_add(0x100)) as usize));
                // ba2277 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba227b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba227d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba2280 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2282 je 0xba2295
                pc = if fl.zf { 0xba2295 } else { 0xba2284 };
            }
            0xba2284 => {
                // ba2284 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba2287 jne 0xba22a1
                pc = if !fl.zf { 0xba22a1 } else { 0xba2289 };
            }
            0xba2289 => {
                // ba2289 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba228c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba228e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba2291 pop ecx
                ecx = s[23];
                // ba2292 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2295 => {
                // ba2295 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba2298 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba229e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba22a1;
            }
            0xba22a1 => {
                // ba22a1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba22a3 pop ecx
                ecx = s[23];
                // ba22a4 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba22d0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba22d0u32;
    loop {
        match pc {
            0xba22d0 => {
                // ba22d0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba22d4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba22d6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba22d9 je 0xba2333
                pc = if fl.zf { 0xba2333 } else { 0xba22db };
            }
            0xba22db => {
                // ba22db sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba22de je 0xba2310
                pc = if fl.zf { 0xba2310 } else { 0xba22e0 };
            }
            0xba22e0 => {
                // ba22e0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba22e3 je 0xba22fa
                pc = if fl.zf { 0xba22fa } else { 0xba22e5 };
            }
            0xba22e5 => {
                // ba22e5 fldz 
                f[0] = 0.0;
                // ba22e7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba22e9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba22ed fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba22f1 fstp dword ptr [ecx + 0x100]
                w.d((ecx.wrapping_add(0x100)) as usize, (f[0] as f32).to_bits());
                // ba22f7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba22fa => {
                // ba22fa fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba22fd mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba22ff fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2303 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2307 fstp dword ptr [ecx + 0x100]
                w.d((ecx.wrapping_add(0x100)) as usize, (f[0] as f32).to_bits());
                // ba230d ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2310 => {
                // ba2310 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba2313 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba2316 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba2318 jge 0xba2336
                pc = if (fl.sf == fl.of) { 0xba2336 } else { 0xba231a };
            }
            0xba231a => {
                // ba231a fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba2320 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba2322 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba2326 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba232a fstp dword ptr [ecx + 0x100]
                w.d((ecx.wrapping_add(0x100)) as usize, (f[0] as f32).to_bits());
                // ba2330 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba2333 => {
                // ba2333 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba2336;
            }
            0xba2336 => {
                // ba2336 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba233a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba233c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba2340 fstp dword ptr [ecx + 0x100]
                w.d((ecx.wrapping_add(0x100)) as usize, (f[0] as f32).to_bits());
                // ba2346 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba35a0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba35a0u32;
    loop {
        match pc {
            0xba35a0 => {
                // ba35a0 mov al, byte ptr [ecx + 0xf0]
                eax = set_part(eax, 0, 0xff, (rb((ecx.wrapping_add(0xf0)) as usize) as u32));
                // ba35a6 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba35aa mov ecx, dword ptr [edx]
                ecx = rd((edx) as usize);
                // ba35ac and al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba35ae test ecx, ecx
                let r_ = (ecx) & (ecx);
                fl = fl_logic(r_, 32);
                // ba35b0 je 0xba35bf
                pc = if fl.zf { 0xba35bf } else { 0xba35b2 };
            }
            0xba35b2 => {
                // ba35b2 cmp ecx, 5
                fl = fl_sub(ecx, 0x5u32, 32).1;
                // ba35b5 jne 0xba35c8
                pc = if !fl.zf { 0xba35c8 } else { 0xba35b7 };
            }
            0xba35b7 => {
                // ba35b7 mov byte ptr [edx + 4], al
                w.b((edx.wrapping_add(0x4)) as usize, ((eax & 0xff)) as u8);
                // ba35ba mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba35bc ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba35bf => {
                // ba35bf mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // ba35c5 mov byte ptr [edx + 4], al
                w.b((edx.wrapping_add(0x4)) as usize, ((eax & 0xff)) as u8);
                pc = 0xba35c8;
            }
            0xba35c8 => {
                // ba35c8 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba35ca ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba3640(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba3640u32;
    loop {
        match pc {
            0xba3640 => {
                // ba3640 push ecx
                s[23] = ecx;
                // ba3641 fld dword ptr [ecx + 0xec]
                f[0] = ldq(rd((ecx.wrapping_add(0xec)) as usize));
                // ba3647 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba364b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba364d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba3650 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba3652 je 0xba3665
                pc = if fl.zf { 0xba3665 } else { 0xba3654 };
            }
            0xba3654 => {
                // ba3654 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba3657 jne 0xba3671
                pc = if !fl.zf { 0xba3671 } else { 0xba3659 };
            }
            0xba3659 => {
                // ba3659 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba365c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba365e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba3661 pop ecx
                ecx = s[23];
                // ba3662 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba3665 => {
                // ba3665 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba3668 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba366e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba3671;
            }
            0xba3671 => {
                // ba3671 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba3673 pop ecx
                ecx = s[23];
                // ba3674 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba3cf0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba3cf0u32;
    loop {
        match pc {
            0xba3cf0 => {
                // ba3cf0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba3cf4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba3cf6 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba3cf8 mov cl, byte ptr [ecx + 0xc4]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0xc4)) as usize) as u32));
                // ba3cfe je 0xba3d0d
                pc = if fl.zf { 0xba3d0d } else { 0xba3d00 };
            }
            0xba3d00 => {
                // ba3d00 cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // ba3d03 jne 0xba3d16
                pc = if !fl.zf { 0xba3d16 } else { 0xba3d05 };
            }
            0xba3d05 => {
                // ba3d05 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // ba3d08 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba3d0a ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba3d0d => {
                // ba3d0d mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // ba3d13 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xba3d16;
            }
            0xba3d16 => {
                // ba3d16 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba3d18 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba5030(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba5030u32;
    loop {
        match pc {
            0xba5030 => {
                // ba5030 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba5034 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba5036 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba5038 mov cl, byte ptr [ecx + 0xa0]
                ecx = set_part(ecx, 0, 0xff, (rb((ecx.wrapping_add(0xa0)) as usize) as u32));
                // ba503e je 0xba504d
                pc = if fl.zf { 0xba504d } else { 0xba5040 };
            }
            0xba5040 => {
                // ba5040 cmp eax, 5
                fl = fl_sub(eax, 0x5u32, 32).1;
                // ba5043 jne 0xba5056
                pc = if !fl.zf { 0xba5056 } else { 0xba5045 };
            }
            0xba5045 => {
                // ba5045 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                // ba5048 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba504a ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba504d => {
                // ba504d mov dword ptr [edx], 5
                w.d((edx) as usize, 0x5u32);
                // ba5053 mov byte ptr [edx + 4], cl
                w.b((edx.wrapping_add(0x4)) as usize, ((ecx & 0xff)) as u8);
                pc = 0xba5056;
            }
            0xba5056 => {
                // ba5056 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5058 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba5b50(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba5b50u32;
    loop {
        match pc {
            0xba5b50 => {
                // ba5b50 push ecx
                s[23] = ecx;
                // ba5b51 fld dword ptr [ecx + 0xc0]
                f[0] = ldq(rd((ecx.wrapping_add(0xc0)) as usize));
                // ba5b57 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba5b5b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba5b5d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba5b60 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba5b62 je 0xba5b75
                pc = if fl.zf { 0xba5b75 } else { 0xba5b64 };
            }
            0xba5b64 => {
                // ba5b64 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba5b67 jne 0xba5b81
                pc = if !fl.zf { 0xba5b81 } else { 0xba5b69 };
            }
            0xba5b69 => {
                // ba5b69 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba5b6c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5b6e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba5b71 pop ecx
                ecx = s[23];
                // ba5b72 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5b75 => {
                // ba5b75 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba5b78 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba5b7e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba5b81;
            }
            0xba5b81 => {
                // ba5b81 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5b83 pop ecx
                ecx = s[23];
                // ba5b84 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba5bb0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba5bb0u32;
    loop {
        match pc {
            0xba5bb0 => {
                // ba5bb0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba5bb4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba5bb6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5bb9 je 0xba5c13
                pc = if fl.zf { 0xba5c13 } else { 0xba5bbb };
            }
            0xba5bbb => {
                // ba5bbb sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5bbe je 0xba5bf0
                pc = if fl.zf { 0xba5bf0 } else { 0xba5bc0 };
            }
            0xba5bc0 => {
                // ba5bc0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5bc3 je 0xba5bda
                pc = if fl.zf { 0xba5bda } else { 0xba5bc5 };
            }
            0xba5bc5 => {
                // ba5bc5 fldz 
                f[0] = 0.0;
                // ba5bc7 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5bc9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5bcd fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5bd1 fstp dword ptr [ecx + 0xc0]
                w.d((ecx.wrapping_add(0xc0)) as usize, (f[0] as f32).to_bits());
                // ba5bd7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5bda => {
                // ba5bda fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba5bdd mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5bdf fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5be3 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5be7 fstp dword ptr [ecx + 0xc0]
                w.d((ecx.wrapping_add(0xc0)) as usize, (f[0] as f32).to_bits());
                // ba5bed ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5bf0 => {
                // ba5bf0 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba5bf3 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba5bf6 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba5bf8 jge 0xba5c16
                pc = if (fl.sf == fl.of) { 0xba5c16 } else { 0xba5bfa };
            }
            0xba5bfa => {
                // ba5bfa fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba5c00 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5c02 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5c06 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5c0a fstp dword ptr [ecx + 0xc0]
                w.d((ecx.wrapping_add(0xc0)) as usize, (f[0] as f32).to_bits());
                // ba5c10 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5c13 => {
                // ba5c13 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba5c16;
            }
            0xba5c16 => {
                // ba5c16 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5c1a mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5c1c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5c20 fstp dword ptr [ecx + 0xc0]
                w.d((ecx.wrapping_add(0xc0)) as usize, (f[0] as f32).to_bits());
                // ba5c26 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba5c50(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba5c50u32;
    loop {
        match pc {
            0xba5c50 => {
                // ba5c50 push ecx
                s[23] = ecx;
                // ba5c51 fld dword ptr [ecx + 0xc4]
                f[0] = ldq(rd((ecx.wrapping_add(0xc4)) as usize));
                // ba5c57 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba5c5b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba5c5d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba5c60 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba5c62 je 0xba5c75
                pc = if fl.zf { 0xba5c75 } else { 0xba5c64 };
            }
            0xba5c64 => {
                // ba5c64 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba5c67 jne 0xba5c81
                pc = if !fl.zf { 0xba5c81 } else { 0xba5c69 };
            }
            0xba5c69 => {
                // ba5c69 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba5c6c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5c6e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba5c71 pop ecx
                ecx = s[23];
                // ba5c72 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5c75 => {
                // ba5c75 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba5c78 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba5c7e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba5c81;
            }
            0xba5c81 => {
                // ba5c81 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba5c83 pop ecx
                ecx = s[23];
                // ba5c84 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba5cb0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba5cb0u32;
    loop {
        match pc {
            0xba5cb0 => {
                // ba5cb0 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba5cb4 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba5cb6 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5cb9 je 0xba5d13
                pc = if fl.zf { 0xba5d13 } else { 0xba5cbb };
            }
            0xba5cbb => {
                // ba5cbb sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5cbe je 0xba5cf0
                pc = if fl.zf { 0xba5cf0 } else { 0xba5cc0 };
            }
            0xba5cc0 => {
                // ba5cc0 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba5cc3 je 0xba5cda
                pc = if fl.zf { 0xba5cda } else { 0xba5cc5 };
            }
            0xba5cc5 => {
                // ba5cc5 fldz 
                f[0] = 0.0;
                // ba5cc7 xor al, al
                let r_ = ((eax & 0xff)) ^ ((eax & 0xff));
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba5cc9 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5ccd fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5cd1 fstp dword ptr [ecx + 0xc4]
                w.d((ecx.wrapping_add(0xc4)) as usize, (f[0] as f32).to_bits());
                // ba5cd7 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5cda => {
                // ba5cda fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba5cdd xor al, al
                let r_ = ((eax & 0xff)) ^ ((eax & 0xff));
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba5cdf fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5ce3 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5ce7 fstp dword ptr [ecx + 0xc4]
                w.d((ecx.wrapping_add(0xc4)) as usize, (f[0] as f32).to_bits());
                // ba5ced ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5cf0 => {
                // ba5cf0 mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba5cf3 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba5cf6 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba5cf8 jge 0xba5d16
                pc = if (fl.sf == fl.of) { 0xba5d16 } else { 0xba5cfa };
            }
            0xba5cfa => {
                // ba5cfa fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba5d00 xor al, al
                let r_ = ((eax & 0xff)) ^ ((eax & 0xff));
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba5d02 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5d06 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5d0a fstp dword ptr [ecx + 0xc4]
                w.d((ecx.wrapping_add(0xc4)) as usize, (f[0] as f32).to_bits());
                // ba5d10 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba5d13 => {
                // ba5d13 fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba5d16;
            }
            0xba5d16 => {
                // ba5d16 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba5d1a xor al, al
                let r_ = ((eax & 0xff)) ^ ((eax & 0xff));
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba5d1c fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // ba5d20 fstp dword ptr [ecx + 0xc4]
                w.d((ecx.wrapping_add(0xc4)) as usize, (f[0] as f32).to_bits());
                // ba5d26 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba6400(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba6400u32;
    loop {
        match pc {
            0xba6400 => {
                // ba6400 push ecx
                s[23] = ecx;
                // ba6401 fld dword ptr [ecx + 0x98]
                f[0] = ldq(rd((ecx.wrapping_add(0x98)) as usize));
                // ba6407 mov ecx, dword ptr [esp + 8]
                ecx = s[25];
                // ba640b mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // ba640d fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // ba6410 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba6412 je 0xba6425
                pc = if fl.zf { 0xba6425 } else { 0xba6414 };
            }
            0xba6414 => {
                // ba6414 cmp eax, 3
                fl = fl_sub(eax, 0x3u32, 32).1;
                // ba6417 jne 0xba6431
                pc = if !fl.zf { 0xba6431 } else { 0xba6419 };
            }
            0xba6419 => {
                // ba6419 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba641c mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba641e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // ba6421 pop ecx
                ecx = s[23];
                // ba6422 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba6425 => {
                // ba6425 fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // ba6428 mov dword ptr [ecx], 3
                w.d((ecx) as usize, 0x3u32);
                // ba642e fstp dword ptr [ecx + 4]
                w.d((ecx.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                pc = 0xba6431;
            }
            0xba6431 => {
                // ba6431 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba6433 pop ecx
                ecx = s[23];
                // ba6434 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_ba6460(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xba6460u32;
    loop {
        match pc {
            0xba6460 => {
                // ba6460 mov edx, dword ptr [esp + 4]
                edx = s[25];
                // ba6464 mov eax, dword ptr [edx]
                eax = rd((edx) as usize);
                // ba6466 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba6469 je 0xba648c
                pc = if fl.zf { 0xba648c } else { 0xba646b };
            }
            0xba646b => {
                // ba646b sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba646e je 0xba647a
                pc = if fl.zf { 0xba647a } else { 0xba6470 };
            }
            0xba6470 => {
                // ba6470 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // ba6473 jne 0xba64a6
                pc = if !fl.zf { 0xba64a6 } else { 0xba6475 };
            }
            0xba6475 => {
                // ba6475 fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // ba6478 jmp 0xba648f
                pc = 0xba648f;
            }
            0xba647a => {
                // ba647a mov eax, dword ptr [edx + 4]
                eax = rd((edx.wrapping_add(0x4)) as usize);
                // ba647d fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // ba6480 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // ba6482 jge 0xba648f
                pc = if (fl.sf == fl.of) { 0xba648f } else { 0xba6484 };
            }
            0xba6484 => {
                // ba6484 fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // ba648a jmp 0xba648f
                pc = 0xba648f;
            }
            0xba648c => {
                // ba648c fild dword ptr [edx + 4]
                f[0] = (((rd((edx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xba648f;
            }
            0xba648f => {
                // ba648f fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // ba6493 fldz 
                f[0] = 0.0;
                // ba6495 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // ba6499 fcom st(1)
                cc87 = cc(f[1], f[0]);
                // ba649b fnstsw ax
                eax = (eax & 0xffff0000) | sw_ax(cc87, 2);
                // ba649d fstp st(1)
                f[0] = f[1];
                // ba649f test ah, 0x41
                let r_ = (((eax >> 8) & 0xff)) & (0x41u32);
                fl = fl_logic(r_, 8);
                // ba64a2 jp 0xba64ab
                pc = if fl.pf { 0xba64ab } else { 0xba64a4 };
            }
            0xba64a4 => {
                // ba64a4 fstp st(0)
                pc = 0xba64a6;
            }
            0xba64a6 => {
                // ba64a6 xor al, al
                let r_ = ((eax & 0xff)) ^ ((eax & 0xff));
                fl = fl_logic(r_, 8);
                eax = set_part(eax, 0, 0xff, r_);
                // ba64a8 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xba64ab => {
                // ba64ab fstp dword ptr [ecx + 0x98]
                w.d((ecx.wrapping_add(0x98)) as usize, (f[0] as f32).to_bits());
                // ba64b1 mov al, 1
                eax = set_part(eax, 0, 0xff, 0x1u32);
                // ba64b3 ret 8
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_7903d0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0x7903d0u32;
    loop {
        match pc {
            0x7903d0 => {
                // 7903d0 mov ecx, dword ptr [eax]
                ecx = rd((eax) as usize);
                // 7903d2 test ecx, ecx
                let r_ = (ecx) & (ecx);
                fl = fl_logic(r_, 32);
                // 7903d4 je 0x7903de
                pc = if fl.zf { 0x7903de } else { 0x7903d6 };
            }
            0x7903d6 => {
                // 7903d6 cmp ecx, 3
                fl = fl_sub(ecx, 0x3u32, 32).1;
                // 7903d9 je 0x7903e4
                pc = if fl.zf { 0x7903e4 } else { 0x7903db };
            }
            0x7903db => {
                // 7903db ret 4
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0x7903de => {
                // 7903de mov dword ptr [eax], 3
                w.d((eax) as usize, 0x3u32);
                pc = 0x7903e4;
            }
            0x7903e4 => {
                // 7903e4 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // 7903e8 fstp dword ptr [eax + 4]
                w.d((eax.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // 7903eb ret 4
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_5e4970(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0x5e4970u32;
    loop {
        match pc {
            0x5e4970 => {
                // 5e4970 push ecx
                s[23] = ecx;
                // 5e4971 mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // 5e4973 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // 5e4976 je 0x5e499d
                pc = if fl.zf { 0x5e499d } else { 0x5e4978 };
            }
            0x5e4978 => {
                // 5e4978 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // 5e497b je 0x5e498b
                pc = if fl.zf { 0x5e498b } else { 0x5e497d };
            }
            0x5e497d => {
                // 5e497d sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // 5e4980 je 0x5e4986
                pc = if fl.zf { 0x5e4986 } else { 0x5e4982 };
            }
            0x5e4982 => {
                // 5e4982 fldz 
                f[0] = 0.0;
                // 5e4984 pop ecx
                ecx = s[23];
                // 5e4985 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0x5e4986 => {
                // 5e4986 fld dword ptr [ecx + 4]
                f[0] = ldq(rd((ecx.wrapping_add(0x4)) as usize));
                // 5e4989 pop ecx
                ecx = s[23];
                // 5e498a ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0x5e498b => {
                // 5e498b mov eax, dword ptr [ecx + 4]
                eax = rd((ecx.wrapping_add(0x4)) as usize);
                // 5e498e fild dword ptr [ecx + 4]
                f[0] = (((rd((ecx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // 5e4991 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // 5e4993 jge 0x5e49a0
                pc = if (fl.sf == fl.of) { 0x5e49a0 } else { 0x5e4995 };
            }
            0x5e4995 => {
                // 5e4995 fadd dword ptr [0xe5bfd8]
                f[0] = xn(f[0], ldq(rd((0xe5bfd8u32) as usize)), f[0] + ldq(rd((0xe5bfd8u32) as usize)));
                // 5e499b pop ecx
                ecx = s[23];
                // 5e499c ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0x5e499d => {
                // 5e499d fild dword ptr [ecx + 4]
                f[0] = (((rd((ecx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0x5e49a0;
            }
            0x5e49a0 => {
                // 5e49a0 pop ecx
                ecx = s[23];
                // 5e49a1 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b8cf40(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb8cf40u32;
    loop {
        match pc {
            0xb8cf40 => {
                // b8cf40 push ecx
                s[23] = ecx;
                // b8cf41 mov eax, dword ptr [ecx]
                eax = rd((ecx) as usize);
                // b8cf43 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b8cf46 je 0xb8cf6d
                pc = if fl.zf { 0xb8cf6d } else { 0xb8cf48 };
            }
            0xb8cf48 => {
                // b8cf48 sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b8cf4b je 0xb8cf5b
                pc = if fl.zf { 0xb8cf5b } else { 0xb8cf4d };
            }
            0xb8cf4d => {
                // b8cf4d sub eax, 1
                let (r_, f_) = fl_sub(eax, 0x1u32, 32); fl = f_;
                eax = r_;
                // b8cf50 je 0xb8cf56
                pc = if fl.zf { 0xb8cf56 } else { 0xb8cf52 };
            }
            0xb8cf52 => {
                // b8cf52 fldz 
                f[0] = 0.0;
                // b8cf54 pop ecx
                ecx = s[23];
                // b8cf55 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb8cf56 => {
                // b8cf56 fld dword ptr [ecx + 4]
                f[0] = ldq(rd((ecx.wrapping_add(0x4)) as usize));
                // b8cf59 pop ecx
                ecx = s[23];
                // b8cf5a ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb8cf5b => {
                // b8cf5b mov eax, dword ptr [ecx + 4]
                eax = rd((ecx.wrapping_add(0x4)) as usize);
                // b8cf5e fild dword ptr [ecx + 4]
                f[0] = (((rd((ecx.wrapping_add(0x4)) as usize)) as i32) as f64);
                // b8cf61 test eax, eax
                let r_ = (eax) & (eax);
                fl = fl_logic(r_, 32);
                // b8cf63 jge 0xb8cf70
                pc = if (fl.sf == fl.of) { 0xb8cf70 } else { 0xb8cf65 };
            }
            0xb8cf65 => {
                // b8cf65 fadd qword ptr [0xe5c0e0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c0e0u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5c0e0u32) as usize))));
                // b8cf6b pop ecx
                ecx = s[23];
                // b8cf6c ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb8cf6d => {
                // b8cf6d fild dword ptr [ecx + 4]
                f[0] = (((rd((ecx.wrapping_add(0x4)) as usize)) as i32) as f64);
                pc = 0xb8cf70;
            }
            0xb8cf70 => {
                // b8cf70 pop ecx
                ecx = s[23];
                // b8cf71 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b8a150(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb8a150u32;
    loop {
        match pc {
            0xb8a150 => {
                // b8a150 push ecx
                s[23] = ecx;
                // b8a151 mov ecx, dword ptr [eax]
                ecx = rd((eax) as usize);
                // b8a153 test ecx, ecx
                let r_ = (ecx) & (ecx);
                fl = fl_logic(r_, 32);
                // b8a155 je 0xb8a15e
                pc = if fl.zf { 0xb8a15e } else { 0xb8a157 };
            }
            0xb8a157 => {
                // b8a157 cmp ecx, 7
                fl = fl_sub(ecx, 0x7u32, 32).1;
                // b8a15a je 0xb8a164
                pc = if fl.zf { 0xb8a164 } else { 0xb8a15c };
            }
            0xb8a15c => {
                // b8a15c pop ecx
                ecx = s[23];
                // b8a15d ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb8a15e => {
                // b8a15e mov dword ptr [eax], 7
                w.d((eax) as usize, 0x7u32);
                pc = 0xb8a164;
            }
            0xb8a164 => {
                // b8a164 fld dword ptr [edx + 4]
                f[0] = ldq(rd((edx.wrapping_add(0x4)) as usize));
                // b8a167 fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b8a16a fld dword ptr [edx]
                f[0] = ldq(rd((edx) as usize));
                // b8a16c fstp dword ptr [eax + 4]
                w.d((eax.wrapping_add(0x4)) as usize, (f[0] as f32).to_bits());
                // b8a16f fld dword ptr [esp]
                f[0] = ldq(s[23]);
                // b8a172 fstp dword ptr [eax + 8]
                w.d((eax.wrapping_add(0x8)) as usize, (f[0] as f32).to_bits());
                // b8a175 pop ecx
                ecx = s[23];
                // b8a176 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b956b0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb956b0u32;
    loop {
        match pc {
            0xb956b0 => {
                // b956b0 sub esp, 0xc
                // b956b3 cmp dword ptr [ecx], 6
                fl = fl_sub(rd((ecx) as usize), 0x6u32, 32).1;
                // b956b6 jne 0xb956cd
                pc = if !fl.zf { 0xb956cd } else { 0xb956b8 };
            }
            0xb956b8 => {
                // b956b8 mov edx, dword ptr [ecx + 4]
                edx = rd((ecx.wrapping_add(0x4)) as usize);
                // b956bb mov dword ptr [eax], edx
                w.d((eax) as usize, edx);
                // b956bd mov edx, dword ptr [ecx + 8]
                edx = rd((ecx.wrapping_add(0x8)) as usize);
                // b956c0 mov ecx, dword ptr [ecx + 0xc]
                ecx = rd((ecx.wrapping_add(0xc)) as usize);
                // b956c3 mov dword ptr [eax + 4], edx
                w.d((eax.wrapping_add(0x4)) as usize, edx);
                // b956c6 mov dword ptr [eax + 8], ecx
                w.d((eax.wrapping_add(0x8)) as usize, ecx);
                // b956c9 add esp, 0xc
                // b956cc ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            0xb956cd => {
                // b956cd fldz 
                f[0] = 0.0;
                // b956cf fst dword ptr [esp]
                s[21] = (f[0] as f32).to_bits();
                // b956d2 mov edx, dword ptr [esp]
                edx = s[21];
                // b956d5 fst dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b956d9 mov ecx, dword ptr [esp + 4]
                ecx = s[22];
                // b956dd fstp dword ptr [esp + 8]
                s[23] = (f[0] as f32).to_bits();
                // b956e1 mov dword ptr [eax], edx
                w.d((eax) as usize, edx);
                // b956e3 mov edx, dword ptr [esp + 8]
                edx = s[23];
                // b956e7 mov dword ptr [eax + 4], ecx
                w.d((eax.wrapping_add(0x4)) as usize, ecx);
                // b956ea mov dword ptr [eax + 8], edx
                w.d((eax.wrapping_add(0x8)) as usize, edx);
                // b956ed add esp, 0xc
                // b956f0 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return 0.0;
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9dda0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9dda0u32;
    loop {
        match pc {
            0xb9dda0 => {
                // b9dda0 push ebp
                s[23] = ebp;
                // b9dda1 mov ebp, esp
                // mov ebp, esp (frame)
                // b9dda3 and esp, 0xfffffff8
                // and esp, -8 (alignment is not modelled)
                // b9dda6 sub esp, 8
                // b9dda9 fld dword ptr [ebp + 8]
                f[0] = ldq(s[25]);
                // b9ddac fst dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9ddb0 mov eax, dword ptr [esp + 4]
                eax = s[22];
                // b9ddb4 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9ddb7 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9ddb9 jne 0xb9ddff
                pc = if !fl.zf { 0xb9ddff } else { 0xb9ddbb };
            }
            0xb9ddbb => {
                // b9ddbb fst dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9ddbf cmp dword ptr [esp + 4], 0
                fl = fl_sub(s[22], 0x0u32, 32).1;
                // b9ddc4 je 0xb9ddff
                pc = if fl.zf { 0xb9ddff } else { 0xb9ddc6 };
            }
            0xb9ddc6 => {
                // b9ddc6 fsub qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9ddcc fmul qword ptr [0xe5c1b8]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c1b8u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c1b8u32) as usize))));
                // b9ddd2 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9ddd6 fld qword ptr [0xe5bfe8]
                f[0] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9dddc fld dword ptr [esp + 4]
                f[1] = ldq(s[22]);
                // b9dde0 call 0xca05d0
                { let (ra, rc, rd_, r) = crt_pow(eax, ecx, edx, f[0], f[1]); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9dde5 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9dde9 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9dded fmul dword ptr [ebp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9ddf0 fadd dword ptr [ebp + 0xc]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9ddf3 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9ddf7 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9ddfb mov esp, ebp
                // mov esp, ebp
                // b9ddfd pop ebp
                ebp = s[23];
                // b9ddfe ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9ddff => {
                // b9ddff fstp st(0)
                // b9de01 fld dword ptr [ebp + 0xc]
                f[0] = ldq(s[26]);
                // b9de04 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9de08 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9de0c mov esp, ebp
                // mov esp, ebp
                // b9de0e pop ebp
                ebp = s[23];
                // b9de0f ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9de10(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9de10u32;
    loop {
        match pc {
            0xb9de10 => {
                // b9de10 push ebp
                s[23] = ebp;
                // b9de11 mov ebp, esp
                // mov ebp, esp (frame)
                // b9de13 and esp, 0xfffffff8
                // and esp, -8 (alignment is not modelled)
                // b9de16 sub esp, 8
                // b9de19 fld dword ptr [ebp + 8]
                f[0] = ldq(s[25]);
                // b9de1c fld st(0)
                f[1] = f[0];
                // b9de1e fld1 
                f[2] = 1.0;
                // b9de20 fld st(0)
                f[3] = f[2];
                // b9de22 fsubrp st(2)
                f[1] = xn(f[1], f[3], f[3] - f[1]);
                // b9de24 fxch st(1)
                f.swap(2, 1);
                // b9de26 fstp dword ptr [esp + 4]
                s[22] = (f[2] as f32).to_bits();
                // b9de2a mov eax, dword ptr [esp + 4]
                eax = s[22];
                // b9de2e shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9de31 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9de33 jne 0xb9de79
                pc = if !fl.zf { 0xb9de79 } else { 0xb9de35 };
            }
            0xb9de35 => {
                // b9de35 fsubr st(1)
                f[1] = xn(f[1], f[0], f[0] - f[1]);
                // b9de37 fstp dword ptr [esp + 4]
                s[22] = (f[1] as f32).to_bits();
                // b9de3b cmp dword ptr [esp + 4], 0
                fl = fl_sub(s[22], 0x0u32, 32).1;
                // b9de40 je 0xb9de7b
                pc = if fl.zf { 0xb9de7b } else { 0xb9de42 };
            }
            0xb9de42 => {
                // b9de42 fmul qword ptr [0xe5c9c0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c9c0u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c9c0u32) as usize))));
                // b9de48 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9de4c fld qword ptr [0xe5bfe8]
                f[0] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9de52 fld dword ptr [esp + 4]
                f[1] = ldq(s[22]);
                // b9de56 call 0xca05d0
                { let (ra, rc, rd_, r) = crt_pow(eax, ecx, edx, f[0], f[1]); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9de5b fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9de5f fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9de63 fld1 
                f[1] = 1.0;
                // b9de65 fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9de67 fmul dword ptr [ebp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9de6a fadd dword ptr [ebp + 0xc]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9de6d fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9de71 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9de75 mov esp, ebp
                // mov esp, ebp
                // b9de77 pop ebp
                ebp = s[23];
                // b9de78 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9de79 => {
                // b9de79 fstp st(1)
                f[0] = f[1];
                pc = 0xb9de7b;
            }
            0xb9de7b => {
                // b9de7b fstp st(0)
                // b9de7d fld dword ptr [ebp + 0xc]
                f[0] = ldq(s[26]);
                // b9de80 fadd dword ptr [ebp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] + ldq(s[27]));
                // b9de83 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9de87 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9de8b mov esp, ebp
                // mov esp, ebp
                // b9de8d pop ebp
                ebp = s[23];
                // b9de8e ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9de90(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9de90u32;
    loop {
        match pc {
            0xb9de90 => {
                // b9de90 push ebp
                s[23] = ebp;
                // b9de91 mov ebp, esp
                // mov ebp, esp (frame)
                // b9de93 and esp, 0xfffffff8
                // and esp, -8 (alignment is not modelled)
                // b9de96 sub esp, 8
                // b9de99 fld dword ptr [ebp + 8]
                f[0] = ldq(s[25]);
                // b9de9c fst dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9dea0 cmp dword ptr [esp + 4], 0
                fl = fl_sub(s[22], 0x0u32, 32).1;
                // b9dea5 jne 0xb9deb0
                pc = if !fl.zf { 0xb9deb0 } else { 0xb9dea7 };
            }
            0xb9dea7 => {
                // b9dea7 fstp st(0)
                // b9dea9 fld dword ptr [ebp + 0xc]
                f[0] = ldq(s[26]);
                // b9deac mov esp, ebp
                // mov esp, ebp
                // b9deae pop ebp
                ebp = s[23];
                // b9deaf ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9deb0 => {
                // b9deb0 fld st(0)
                f[1] = f[0];
                // b9deb2 fld1 
                f[2] = 1.0;
                // b9deb4 fsub st(1), st(0)
                f[1] = xn(f[1], f[2], f[1] - f[2]);
                // b9deb6 fxch st(1)
                f.swap(2, 1);
                // b9deb8 fstp dword ptr [esp + 4]
                s[22] = (f[2] as f32).to_bits();
                // b9debc cmp dword ptr [esp + 4], 0
                fl = fl_sub(s[22], 0x0u32, 32).1;
                // b9dec1 jne 0xb9ded9
                pc = if !fl.zf { 0xb9ded9 } else { 0xb9dec3 };
            }
            0xb9dec3 => {
                // b9dec3 fstp st(1)
                f[0] = f[1];
                // b9dec5 fstp st(0)
                // b9dec7 fld dword ptr [ebp + 0xc]
                f[0] = ldq(s[26]);
                // b9deca fadd dword ptr [ebp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] + ldq(s[27]));
                // b9decd fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9ded1 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9ded5 mov esp, ebp
                // mov esp, ebp
                // b9ded7 pop ebp
                ebp = s[23];
                // b9ded8 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9ded9 => {
                // b9ded9 fld qword ptr [0xe5bfe8]
                f[2] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9dedf fmul st(2), st(0)
                f[0] = xn(f[0], f[2], f[0] * f[2]);
                // b9dee1 fxch st(2)
                f.swap(2, 0);
                // b9dee3 fsubrp st(1)
                f[1] = xn(f[1], f[2], f[2] - f[1]);
                // b9dee5 fstp dword ptr [ebp + 8]
                s[25] = (f[1] as f32).to_bits();
                // b9dee8 fld dword ptr [ebp + 8]
                f[1] = ldq(s[25]);
                // b9deeb fst dword ptr [esp + 4]
                s[22] = (f[1] as f32).to_bits();
                // b9deef mov eax, dword ptr [esp + 4]
                eax = s[22];
                // b9def3 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9def6 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9def8 je 0xb9df2f
                pc = if fl.zf { 0xb9df2f } else { 0xb9defa };
            }
            0xb9defa => {
                // b9defa fmul qword ptr [0xe5c1b8]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5c1b8u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5c1b8u32) as usize))));
                // b9df00 fstp dword ptr [esp + 4]
                s[22] = (f[1] as f32).to_bits();
                // b9df04 fld dword ptr [esp + 4]
                f[1] = ldq(s[22]);
                // b9df08 call 0xca05d0
                { let (ra, rc, rd_, r) = crt_pow(eax, ecx, edx, f[0], f[1]); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9df0d fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9df11 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9df15 fld dword ptr [ebp + 0x10]
                f[1] = ldq(s[27]);
                // b9df18 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9df1e fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9df20 fadd dword ptr [ebp + 0xc]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9df23 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9df27 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9df2b mov esp, ebp
                // mov esp, ebp
                // b9df2d pop ebp
                ebp = s[23];
                // b9df2e ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9df2f => {
                // b9df2f fmul qword ptr [0xe5c9c0]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5c9c0u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5c9c0u32) as usize))));
                // b9df35 fstp dword ptr [esp + 4]
                s[22] = (f[1] as f32).to_bits();
                // b9df39 fld dword ptr [esp + 4]
                f[1] = ldq(s[22]);
                // b9df3d call 0xca05d0
                { let (ra, rc, rd_, r) = crt_pow(eax, ecx, edx, f[0], f[1]); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9df42 fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9df46 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9df4a fsubr qword ptr [0xe5bfe8]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bfe8u32) as usize))), f64::from_bits(q64(rq((0xe5bfe8u32) as usize))) - f[0]);
                // b9df50 fld dword ptr [ebp + 0x10]
                f[1] = ldq(s[27]);
                // b9df53 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9df59 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9df5b fadd dword ptr [ebp + 0xc]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9df5e fstp dword ptr [esp + 4]
                s[22] = (f[0] as f32).to_bits();
                // b9df62 fld dword ptr [esp + 4]
                f[0] = ldq(s[22]);
                // b9df66 mov esp, ebp
                // mov esp, ebp
                // b9df68 pop ebp
                ebp = s[23];
                // b9df69 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9dfc0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9dfc0u32;
    loop {
        match pc {
            0xb9dfc0 => {
                // b9dfc0 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9dfc4 fadd st(0), st(0)
                f[0] = xn(f[0], f[0], f[0] + f[0]);
                // b9dfc6 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9dfca fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9dfce fld st(0)
                f[1] = f[0];
                // b9dfd0 fld1 
                f[2] = 1.0;
                // b9dfd2 fsub st(1), st(0)
                f[1] = xn(f[1], f[2], f[1] - f[2]);
                // b9dfd4 fxch st(1)
                f.swap(2, 1);
                // b9dfd6 fst dword ptr [esp + 4]
                s[25] = (f[2] as f32).to_bits();
                // b9dfda mov eax, dword ptr [esp + 4]
                eax = s[25];
                // b9dfde shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9dfe1 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9dfe3 je 0xb9e004
                pc = if fl.zf { 0xb9e004 } else { 0xb9dfe5 };
            }
            0xb9dfe5 => {
                // b9dfe5 fstp st(0)
                // b9dfe7 fstp st(0)
                // b9dfe9 fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9dfed fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9dff3 fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9dff5 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9dff7 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9dffb fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9dfff fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e003 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e004 => {
                // b9e004 fld dword ptr [esp + 8]
                f[3] = ldq(s[26]);
                // b9e008 fxch st(3)
                f.swap(3, 0);
                // b9e00a fsub qword ptr [0xe5bff8]
                f[3] = xn(f[3], f64::from_bits(q64(rq((0xe5bff8u32) as usize))), f[3] - f64::from_bits(q64(rq((0xe5bff8u32) as usize))));
                // b9e010 fmulp st(1)
                f[2] = xn(f[2], f[3], f[2] * f[3]);
                // b9e012 fsubrp st(1)
                f[1] = xn(f[1], f[2], f[2] - f[1]);
                // b9e014 fld dword ptr [esp + 0xc]
                f[2] = ldq(s[27]);
                // b9e018 fmul qword ptr [0xe5bf70]
                f[2] = xn(f[2], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[2] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e01e fmulp st(1)
                f[1] = xn(f[1], f[2], f[1] * f[2]);
                // b9e020 fsubp st(1)
                f[0] = xn(f[0], f[1], f[0] - f[1]);
                // b9e022 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e026 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e02a ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e080(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e080u32;
    loop {
        match pc {
            0xb9e080 => {
                // b9e080 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e084 fld qword ptr [0xe5bfe8]
                f[1] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9e08a fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e08c fxch st(1)
                f.swap(1, 0);
                // b9e08e fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e092 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e096 fld qword ptr [0xe5bf40]
                f[2] = f64::from_bits(q64(rq((0xe5bf40u32) as usize)));
                // b9e09c fsubr st(1)
                f[2] = xn(f[2], f[1], f[1] - f[2]);
                // b9e09e fstp dword ptr [esp + 4]
                s[25] = (f[2] as f32).to_bits();
                // b9e0a2 mov eax, dword ptr [esp + 4]
                eax = s[25];
                // b9e0a6 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e0a9 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e0ab je 0xb9e0cc
                pc = if fl.zf { 0xb9e0cc } else { 0xb9e0ad };
            }
            0xb9e0ad => {
                // b9e0ad fstp st(1)
                f[0] = f[1];
                // b9e0af fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e0b3 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e0b9 fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e0bb fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e0bd fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e0bf fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e0c3 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e0c7 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e0cb ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e0cc => {
                // b9e0cc fsub st(1)
                f[1] = xn(f[1], f[0], f[1] - f[0]);
                // b9e0ce fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e0d2 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e0d6 fld st(0)
                f[2] = f[1];
                // b9e0d8 fmul st(0), st(0)
                f[2] = xn(f[2], f[2], f[2] * f[2]);
                // b9e0da fmulp st(1)
                f[1] = xn(f[1], f[2], f[1] * f[2]);
                // b9e0dc faddp st(1)
                f[0] = xn(f[0], f[1], f[0] + f[1]);
                // b9e0de fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e0e2 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e0e8 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e0ea fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e0ee fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e0f2 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e0f6 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e160(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e160u32;
    loop {
        match pc {
            0xb9e160 => {
                // b9e160 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e164 fld qword ptr [0xe5bfe8]
                f[1] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9e16a fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e16c fxch st(1)
                f.swap(1, 0);
                // b9e16e fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e172 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e176 fld qword ptr [0xe5bf40]
                f[2] = f64::from_bits(q64(rq((0xe5bf40u32) as usize)));
                // b9e17c fsubr st(1)
                f[2] = xn(f[2], f[1], f[1] - f[2]);
                // b9e17e fstp dword ptr [esp + 4]
                s[25] = (f[2] as f32).to_bits();
                // b9e182 mov eax, dword ptr [esp + 4]
                eax = s[25];
                // b9e186 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e189 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e18b je 0xb9e1ae
                pc = if fl.zf { 0xb9e1ae } else { 0xb9e18d };
            }
            0xb9e18d => {
                // b9e18d fstp st(1)
                f[0] = f[1];
                // b9e18f fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e193 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e199 fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e19b fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e19d fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e19f fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e1a1 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e1a5 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e1a9 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e1ad ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e1ae => {
                // b9e1ae fsub st(1)
                f[1] = xn(f[1], f[0], f[1] - f[0]);
                // b9e1b0 fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e1b4 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e1b8 fld st(0)
                f[2] = f[1];
                // b9e1ba fld st(0)
                f[3] = f[2];
                // b9e1bc fmulp st(1)
                f[2] = xn(f[2], f[3], f[2] * f[3]);
                // b9e1be fld st(1)
                f[3] = f[1];
                // b9e1c0 fmulp st(1)
                f[2] = xn(f[2], f[3], f[2] * f[3]);
                // b9e1c2 fmulp st(1)
                f[1] = xn(f[1], f[2], f[1] * f[2]);
                // b9e1c4 fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9e1c6 fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e1ca fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e1d0 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e1d2 fsubr dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), ldq(s[26]) - f[0]);
                // b9e1d6 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e1da fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e1de ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e230(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e230u32;
    loop {
        match pc {
            0xb9e230 => {
                // b9e230 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e234 fld qword ptr [0xe5bfe8]
                f[1] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9e23a fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e23c fxch st(1)
                f.swap(1, 0);
                // b9e23e fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e242 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e246 fld qword ptr [0xe5bf40]
                f[2] = f64::from_bits(q64(rq((0xe5bf40u32) as usize)));
                // b9e24c fsubr st(1)
                f[2] = xn(f[2], f[1], f[1] - f[2]);
                // b9e24e fstp dword ptr [esp + 4]
                s[25] = (f[2] as f32).to_bits();
                // b9e252 mov eax, dword ptr [esp + 4]
                eax = s[25];
                // b9e256 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e259 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e25b je 0xb9e280
                pc = if fl.zf { 0xb9e280 } else { 0xb9e25d };
            }
            0xb9e25d => {
                // b9e25d fstp st(1)
                f[0] = f[1];
                // b9e25f fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e263 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e269 fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e26b fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e26d fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e26f fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e271 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e273 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e277 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e27b fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e27f ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e280 => {
                // b9e280 fsub st(1)
                f[1] = xn(f[1], f[0], f[1] - f[0]);
                // b9e282 fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e286 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e28a fld st(0)
                f[2] = f[1];
                // b9e28c fmul st(0), st(0)
                f[2] = xn(f[2], f[2], f[2] * f[2]);
                // b9e28e fmul st(1)
                f[2] = xn(f[2], f[1], f[2] * f[1]);
                // b9e290 fmul st(1)
                f[2] = xn(f[2], f[1], f[2] * f[1]);
                // b9e292 fmulp st(1)
                f[1] = xn(f[1], f[2], f[1] * f[2]);
                // b9e294 faddp st(1)
                f[0] = xn(f[0], f[1], f[0] + f[1]);
                // b9e296 fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e29a fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e2a0 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e2a2 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e2a6 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e2aa fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e2ae ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e2b0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e2b0u32;
    loop {
        match pc {
            0xb9e2b0 => {
                // b9e2b0 sub esp, 8
                // b9e2b3 fld dword ptr [esp + 0x14]
                f[0] = ldq(s[27]);
                // b9e2b7 push ecx
                s[21] = ecx;
                // b9e2b8 fstp qword ptr [esp + 4]
                { let b_ = f[0].to_bits(); s[22] = b_ as u32; s[23] = (b_ >> 32) as u32; }
                // b9e2bc fld dword ptr [esp + 0x10]
                f[0] = ldq(s[25]);
                // b9e2c0 fmul qword ptr [0xe5c448]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c448u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c448u32) as usize))));
                // b9e2c6 fstp dword ptr [esp + 0x18]
                s[27] = (f[0] as f32).to_bits();
                // b9e2ca fld dword ptr [esp + 0x18]
                f[0] = ldq(s[27]);
                // b9e2ce fstp dword ptr [esp]
                s[21] = (f[0] as f32).to_bits();
                // b9e2d1 call 0x40b820
                { let (ra, rc, rd_, r) = crt_call1(0x40b820, s[21], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e2d6 fld qword ptr [esp + 4]
                f[1] = f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32)));
                // b9e2da fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e2dc fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9e2de fadd dword ptr [esp + 0x14]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e2e2 fstp dword ptr [esp + 0x18]
                s[27] = (f[0] as f32).to_bits();
                // b9e2e6 fld dword ptr [esp + 0x18]
                f[0] = ldq(s[27]);
                // b9e2ea add esp, 0xc
                // b9e2ed ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e2f0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e2f0u32;
    loop {
        match pc {
            0xb9e2f0 => {
                // b9e2f0 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e2f4 push ecx
                s[23] = ecx;
                // b9e2f5 fmul qword ptr [0xe5c448]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c448u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c448u32) as usize))));
                // b9e2fb fstp dword ptr [esp + 8]
                s[25] = (f[0] as f32).to_bits();
                // b9e2ff fld dword ptr [esp + 8]
                f[0] = ldq(s[25]);
                // b9e303 fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9e306 call 0x428cd0
                { let (ra, rc, rd_, r) = crt_call1(0x428cd0, s[23], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e30b fmul dword ptr [esp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9e30f add esp, 4
                // b9e312 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e316 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e31a fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e31e ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e320(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e320u32;
    loop {
        match pc {
            0xb9e320 => {
                // b9e320 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e324 push ecx
                s[23] = ecx;
                // b9e325 fmul qword ptr [0xe5c160]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c160u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c160u32) as usize))));
                // b9e32b fstp dword ptr [esp + 8]
                s[25] = (f[0] as f32).to_bits();
                // b9e32f fld dword ptr [esp + 8]
                f[0] = ldq(s[25]);
                // b9e333 fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9e336 call 0x40b820
                { let (ra, rc, rd_, r) = crt_call1(0x40b820, s[23], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e33b fsub qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9e341 add esp, 4
                // b9e344 fld dword ptr [esp + 0xc]
                f[1] = ldq(s[27]);
                // b9e348 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e34e fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e350 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e354 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e358 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e35c ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e360(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e360u32;
    loop {
        match pc {
            0xb9e360 => {
                // b9e360 sub esp, 8
                // b9e363 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e367 push ecx
                s[21] = ecx;
                // b9e368 fld dword ptr [esp + 0x14]
                f[1] = ldq(s[26]);
                // b9e36c fstp qword ptr [esp + 4]
                { let b_ = f[1].to_bits(); s[22] = b_ as u32; s[23] = (b_ >> 32) as u32; }
                // b9e370 fmul st(0), st(0)
                f[0] = xn(f[0], f[0], f[0] * f[0]);
                // b9e372 fld1 
                f[1] = 1.0;
                // b9e374 fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9e376 fstp dword ptr [esp + 0x10]
                s[25] = (f[0] as f32).to_bits();
                // b9e37a fld dword ptr [esp + 0x10]
                f[0] = ldq(s[25]);
                // b9e37e fstp dword ptr [esp]
                s[21] = (f[0] as f32).to_bits();
                // b9e381 call 0x40ea20
                { let (ra, rc, rd_, r) = crt_call1(0x40ea20, s[21], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e386 fsub qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9e38c fmul dword ptr [esp + 0x18]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9e390 fsubr qword ptr [esp + 4]
                f[0] = xn(f[0], f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))), f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))) - f[0]);
                // b9e394 fstp dword ptr [esp + 0x10]
                s[25] = (f[0] as f32).to_bits();
                // b9e398 fld dword ptr [esp + 0x10]
                f[0] = ldq(s[25]);
                // b9e39c add esp, 0xc
                // b9e39f ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e3a0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e3a0u32;
    loop {
        match pc {
            0xb9e3a0 => {
                // b9e3a0 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e3a4 push ecx
                s[23] = ecx;
                // b9e3a5 fld1 
                f[1] = 1.0;
                // b9e3a7 fsub st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] - f[1]);
                // b9e3a9 fxch st(1)
                f.swap(1, 0);
                // b9e3ab fstp dword ptr [esp + 8]
                s[25] = (f[1] as f32).to_bits();
                // b9e3af fld dword ptr [esp + 8]
                f[1] = ldq(s[25]);
                // b9e3b3 fmul st(0), st(0)
                f[1] = xn(f[1], f[1], f[1] * f[1]);
                // b9e3b5 fsubp st(1)
                f[0] = xn(f[0], f[1], f[0] - f[1]);
                // b9e3b7 fstp dword ptr [esp + 8]
                s[25] = (f[0] as f32).to_bits();
                // b9e3bb fld dword ptr [esp + 8]
                f[0] = ldq(s[25]);
                // b9e3bf fstp dword ptr [esp]
                s[23] = (f[0] as f32).to_bits();
                // b9e3c2 call 0x40ea20
                { let (ra, rc, rd_, r) = crt_call1(0x40ea20, s[23], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e3c7 fmul dword ptr [esp + 0x10]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9e3cb add esp, 4
                // b9e3ce fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e3d2 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e3d6 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e3da ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e3e0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e3e0u32;
    loop {
        match pc {
            0xb9e3e0 => {
                // b9e3e0 sub esp, 8
                // b9e3e3 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e3e7 push ecx
                s[21] = ecx;
                // b9e3e8 fld qword ptr [0xe5bfe8]
                f[1] = f64::from_bits(q64(rq((0xe5bfe8u32) as usize)));
                // b9e3ee fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e3f0 fxch st(1)
                f.swap(1, 0);
                // b9e3f2 fstp dword ptr [esp + 0x10]
                s[25] = (f[1] as f32).to_bits();
                // b9e3f6 fld dword ptr [esp + 0x10]
                f[1] = ldq(s[25]);
                // b9e3fa fld st(0)
                f[2] = f[1];
                // b9e3fc fld1 
                f[3] = 1.0;
                // b9e3fe fsub st(1), st(0)
                f[2] = xn(f[2], f[3], f[2] - f[3]);
                // b9e400 fxch st(1)
                f.swap(3, 2);
                // b9e402 fstp dword ptr [esp + 0x10]
                s[25] = (f[3] as f32).to_bits();
                // b9e406 mov eax, dword ptr [esp + 0x10]
                eax = s[25];
                // b9e40a shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e40d test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e40f je 0xb9e453
                pc = if fl.zf { 0xb9e453 } else { 0xb9e411 };
            }
            0xb9e411 => {
                // b9e411 fstp st(2)
                f[0] = f[2];
                // b9e413 fld dword ptr [esp + 0x14]
                f[2] = ldq(s[26]);
                // b9e417 fstp qword ptr [esp + 4]
                { let b_ = f[2].to_bits(); s[22] = b_ as u32; s[23] = (b_ >> 32) as u32; }
                // b9e41b fmul st(0), st(0)
                f[1] = xn(f[1], f[1], f[1] * f[1]);
                // b9e41d fsubp st(1)
                f[0] = xn(f[0], f[1], f[0] - f[1]);
                // b9e41f fstp dword ptr [esp + 0x10]
                s[25] = (f[0] as f32).to_bits();
                // b9e423 fld dword ptr [esp + 0x10]
                f[0] = ldq(s[25]);
                // b9e427 fstp dword ptr [esp]
                s[21] = (f[0] as f32).to_bits();
                // b9e42a call 0x40ea20
                { let (ra, rc, rd_, r) = crt_call1(0x40ea20, s[21], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e42f fsub qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9e435 add esp, 4
                // b9e438 fld dword ptr [esp + 0x14]
                f[1] = ldq(s[27]);
                // b9e43c fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e442 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e444 fsubr qword ptr [esp]
                f[0] = xn(f[0], f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))), f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))) - f[0]);
                // b9e447 fstp dword ptr [esp + 0xc]
                s[25] = (f[0] as f32).to_bits();
                // b9e44b fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e44f add esp, 8
                // b9e452 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e453 => {
                // b9e453 fxch st(1)
                f.swap(2, 1);
                // b9e455 faddp st(2)
                f[0] = xn(f[0], f[2], f[0] + f[2]);
                // b9e457 fxch st(1)
                f.swap(1, 0);
                // b9e459 fstp dword ptr [esp + 0x10]
                s[25] = (f[1] as f32).to_bits();
                // b9e45d fld dword ptr [esp + 0x10]
                f[1] = ldq(s[25]);
                // b9e461 fmul st(0), st(0)
                f[1] = xn(f[1], f[1], f[1] * f[1]);
                // b9e463 fsubp st(1)
                f[0] = xn(f[0], f[1], f[0] - f[1]);
                // b9e465 fstp dword ptr [esp + 0x10]
                s[25] = (f[0] as f32).to_bits();
                // b9e469 fld dword ptr [esp + 0x10]
                f[0] = ldq(s[25]);
                // b9e46d fstp dword ptr [esp]
                s[21] = (f[0] as f32).to_bits();
                // b9e470 call 0x40ea20
                { let (ra, rc, rd_, r) = crt_call1(0x40ea20, s[21], eax, ecx, edx); eax = ra; ecx = rc; edx = rd_; f[0] = r; }
                // b9e475 fadd qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9e47b add esp, 4
                // b9e47e fld dword ptr [esp + 0x14]
                f[1] = ldq(s[27]);
                // b9e482 fmul qword ptr [0xe5bf70]
                f[1] = xn(f[1], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[1] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e488 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e48a fadd dword ptr [esp + 0x10]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e48e fstp dword ptr [esp + 0xc]
                s[25] = (f[0] as f32).to_bits();
                // b9e492 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e496 add esp, 8
                // b9e499 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e4a0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e4a0u32;
    loop {
        match pc {
            0xb9e4a0 => {
                // b9e4a0 sub esp, 8
                // b9e4a3 fld dword ptr [esp + 0x14]
                f[0] = ldq(s[27]);
                // b9e4a7 sub esp, 0xc
                // b9e4aa fst qword ptr [esp + 0xc]
                { let b_ = f[0].to_bits(); s[22] = b_ as u32; s[23] = (b_ >> 32) as u32; }
                // b9e4ae fstp dword ptr [esp + 8]
                s[21] = (f[0] as f32).to_bits();
                // b9e4b2 fldz 
                f[0] = 0.0;
                // b9e4b4 fstp dword ptr [esp + 4]
                s[20] = (f[0] as f32).to_bits();
                // b9e4b8 fld dword ptr [esp + 0x18]
                f[0] = ldq(s[25]);
                // b9e4bc fld1 
                f[1] = 1.0;
                // b9e4be fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9e4c0 fstp dword ptr [esp + 0x20]
                s[27] = (f[0] as f32).to_bits();
                // b9e4c4 fld dword ptr [esp + 0x20]
                f[0] = ldq(s[27]);
                // b9e4c8 fstp dword ptr [esp]
                s[19] = (f[0] as f32).to_bits();
                // b9e4cb call 0xb9e4f0
                { let mut q = Pushad { edi, esi, ebp, esp: 0, ebx, edx, ecx, eax }; let r = g2_b9e4f0(&mut q, s[19..].as_ptr(), w); eax = q.eax; ecx = q.ecx; edx = q.edx; f[0] = r; }
                // b9e4d0 fsubr qword ptr [esp + 0xc]
                f[0] = xn(f[0], f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))), f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))) - f[0]);
                // b9e4d4 fadd dword ptr [esp + 0x1c]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e4d8 fstp dword ptr [esp + 0x20]
                s[27] = (f[0] as f32).to_bits();
                // b9e4dc fld dword ptr [esp + 0x20]
                f[0] = ldq(s[27]);
                // b9e4e0 add esp, 0x14
                // b9e4e3 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e4f0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e4f0u32;
    loop {
        match pc {
            0xb9e4f0 => {
                // b9e4f0 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e4f4 fld qword ptr [0xe5c9b8]
                f[1] = f64::from_bits(q64(rq((0xe5c9b8u32) as usize)));
                // b9e4fa fsubr st(1)
                f[1] = xn(f[1], f[0], f[0] - f[1]);
                // b9e4fc fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e500 mov eax, dword ptr [esp + 4]
                eax = s[25];
                // b9e504 shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e507 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e509 je 0xb9e526
                pc = if fl.zf { 0xb9e526 } else { 0xb9e50b };
            }
            0xb9e50b => {
                // b9e50b fld qword ptr [0xe5c9b0]
                f[1] = f64::from_bits(q64(rq((0xe5c9b0u32) as usize)));
                // b9e511 fmul st(1)
                f[1] = xn(f[1], f[0], f[1] * f[0]);
                // b9e513 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e515 fmul dword ptr [esp + 0xc]
                f[0] = xn(f[0], ldq(s[27]), f[0] * ldq(s[27]));
                // b9e519 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e51d fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e521 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e525 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e526 => {
                // b9e526 fld qword ptr [0xe5c9a8]
                f[1] = f64::from_bits(q64(rq((0xe5c9a8u32) as usize)));
                // b9e52c fsubr st(1)
                f[1] = xn(f[1], f[0], f[0] - f[1]);
                // b9e52e fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e532 mov ecx, dword ptr [esp + 4]
                ecx = s[25];
                // b9e536 shr ecx, 0x1f
                let (r_, f_) = fl_shift("shr", ecx, 31, 32); fl = f_;
                ecx = r_;
                // b9e539 test cl, 1
                let r_ = ((ecx & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e53c je 0xb9e56f
                pc = if fl.zf { 0xb9e56f } else { 0xb9e53e };
            }
            0xb9e53e => {
                // b9e53e fsub qword ptr [0xe5c9a0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c9a0u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5c9a0u32) as usize))));
                // b9e544 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e548 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[27]);
                // b9e54c fmul qword ptr [0xe5c9b0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c9b0u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c9b0u32) as usize))));
                // b9e552 fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e556 fld st(0)
                f[2] = f[1];
                // b9e558 fmulp st(2)
                f[0] = xn(f[0], f[2], f[0] * f[2]);
                // b9e55a fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e55c fadd qword ptr [0xe5c370]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c370u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5c370u32) as usize))));
                // b9e562 fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e566 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e56a fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e56e ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e56f => {
                // b9e56f fld qword ptr [0xe5c998]
                f[1] = f64::from_bits(q64(rq((0xe5c998u32) as usize)));
                // b9e575 fsubr st(1)
                f[1] = xn(f[1], f[0], f[0] - f[1]);
                // b9e577 fstp dword ptr [esp + 4]
                s[25] = (f[1] as f32).to_bits();
                // b9e57b mov edx, dword ptr [esp + 4]
                edx = s[25];
                // b9e57f shr edx, 0x1f
                let (r_, f_) = fl_shift("shr", edx, 31, 32); fl = f_;
                edx = r_;
                // b9e582 test dl, 1
                let r_ = ((edx & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e585 je 0xb9e5b8
                pc = if fl.zf { 0xb9e5b8 } else { 0xb9e587 };
            }
            0xb9e587 => {
                // b9e587 fsub qword ptr [0xe5c990]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c990u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5c990u32) as usize))));
                // b9e58d fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e591 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[27]);
                // b9e595 fmul qword ptr [0xe5c9b0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c9b0u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c9b0u32) as usize))));
                // b9e59b fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e59f fld st(0)
                f[2] = f[1];
                // b9e5a1 fmulp st(2)
                f[0] = xn(f[0], f[2], f[0] * f[2]);
                // b9e5a3 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e5a5 fadd qword ptr [0xe5c988]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c988u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5c988u32) as usize))));
                // b9e5ab fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e5af fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e5b3 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e5b7 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e5b8 => {
                // b9e5b8 fsub qword ptr [0xe5c980]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c980u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5c980u32) as usize))));
                // b9e5be fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e5c2 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[27]);
                // b9e5c6 fmul qword ptr [0xe5c9b0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c9b0u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5c9b0u32) as usize))));
                // b9e5cc fld dword ptr [esp + 4]
                f[1] = ldq(s[25]);
                // b9e5d0 fld st(0)
                f[2] = f[1];
                // b9e5d2 fmulp st(2)
                f[0] = xn(f[0], f[2], f[0] * f[2]);
                // b9e5d4 fmulp st(1)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e5d6 fadd qword ptr [0xe5c978]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5c978u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5c978u32) as usize))));
                // b9e5dc fadd dword ptr [esp + 8]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e5e0 fstp dword ptr [esp + 4]
                s[25] = (f[0] as f32).to_bits();
                // b9e5e4 fld dword ptr [esp + 4]
                f[0] = ldq(s[25]);
                // b9e5e8 ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_assignments, unused_mut, unused_variables, unused_parens, clippy::all)]
fn g2_b9e5f0(p: &mut Pushad, a: *const u32, w: &mut W) -> f64 {
    let (mut eax, mut ecx, mut edx, mut ebx, mut esi, mut edi, mut ebp) = (p.eax, p.ecx, p.edx, p.ebx, p.esi, p.edi, p.ebp);
    let mut s = [0u32; 40];
    s[25] = unsafe { *a.add(0) };
    s[26] = unsafe { *a.add(1) };
    s[27] = unsafe { *a.add(2) };
    let mut f = [0f64; 8];
    let mut fl = Fl::default();
    let mut cc87 = 0u32;
    let mut pc = 0xb9e5f0u32;
    loop {
        match pc {
            0xb9e5f0 => {
                // b9e5f0 sub esp, 8
                // b9e5f3 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e5f7 sub esp, 0xc
                // b9e5fa fld qword ptr [0xe5bf70]
                f[1] = f64::from_bits(q64(rq((0xe5bf70u32) as usize)));
                // b9e600 fsubr st(1)
                f[1] = xn(f[1], f[0], f[0] - f[1]);
                // b9e602 fstp dword ptr [esp + 0x18]
                s[25] = (f[1] as f32).to_bits();
                // b9e606 mov eax, dword ptr [esp + 0x18]
                eax = s[25];
                // b9e60a fld dword ptr [esp + 0x20]
                f[1] = ldq(s[27]);
                // b9e60e shr eax, 0x1f
                let (r_, f_) = fl_shift("shr", eax, 31, 32); fl = f_;
                eax = r_;
                // b9e611 test al, 1
                let r_ = ((eax & 0xff)) & (0x1u32);
                fl = fl_logic(r_, 8);
                // b9e613 je 0xb9e66c
                pc = if fl.zf { 0xb9e66c } else { 0xb9e615 };
            }
            0xb9e615 => {
                // b9e615 fst qword ptr [esp + 0xc]
                { let b_ = f[1].to_bits(); s[22] = b_ as u32; s[23] = (b_ >> 32) as u32; }
                // b9e619 fstp dword ptr [esp + 8]
                s[21] = (f[1] as f32).to_bits();
                // b9e61d fldz 
                f[1] = 0.0;
                // b9e61f fstp dword ptr [esp + 4]
                s[20] = (f[1] as f32).to_bits();
                // b9e623 fadd st(0), st(0)
                f[0] = xn(f[0], f[0], f[0] + f[0]);
                // b9e625 fstp dword ptr [esp + 0x18]
                s[25] = (f[0] as f32).to_bits();
                // b9e629 fld dword ptr [esp + 0x18]
                f[0] = ldq(s[25]);
                // b9e62d fld1 
                f[1] = 1.0;
                // b9e62f fsubrp st(1)
                f[0] = xn(f[0], f[1], f[1] - f[0]);
                // b9e631 fstp dword ptr [esp + 0x18]
                s[25] = (f[0] as f32).to_bits();
                // b9e635 fld dword ptr [esp + 0x18]
                f[0] = ldq(s[25]);
                // b9e639 fstp dword ptr [esp]
                s[19] = (f[0] as f32).to_bits();
                // b9e63c call 0xb9e4f0
                { let mut q = Pushad { edi, esi, ebp, esp: 0, ebx, edx, ecx, eax }; let r = g2_b9e4f0(&mut q, s[19..].as_ptr(), w); eax = q.eax; ecx = q.ecx; edx = q.edx; f[0] = r; }
                // b9e641 fsubr qword ptr [esp + 0xc]
                f[0] = xn(f[0], f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))), f64::from_bits(q64((s[22] as u64) | ((s[23] as u64) << 32))) - f[0]);
                // b9e645 add esp, 0xc
                // b9e648 fadd qword ptr [0xe5bfa0]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bfa0u32) as usize))), f[0] + f64::from_bits(q64(rq((0xe5bfa0u32) as usize))));
                // b9e64e fstp dword ptr [esp + 0xc]
                s[25] = (f[0] as f32).to_bits();
                // b9e652 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e656 fmul qword ptr [0xe5bf70]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf70u32) as usize))), f[0] * f64::from_bits(q64(rq((0xe5bf70u32) as usize))));
                // b9e65c fadd dword ptr [esp + 0x10]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e660 fstp dword ptr [esp + 0xc]
                s[25] = (f[0] as f32).to_bits();
                // b9e664 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e668 add esp, 8
                // b9e66b ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            0xb9e66c => {
                // b9e66c fstp dword ptr [esp + 8]
                s[21] = (f[1] as f32).to_bits();
                // b9e670 fldz 
                f[1] = 0.0;
                // b9e672 fstp dword ptr [esp + 4]
                s[20] = (f[1] as f32).to_bits();
                // b9e676 fadd st(0), st(0)
                f[0] = xn(f[0], f[0], f[0] + f[0]);
                // b9e678 fsub qword ptr [0xe5bf40]
                f[0] = xn(f[0], f64::from_bits(q64(rq((0xe5bf40u32) as usize))), f[0] - f64::from_bits(q64(rq((0xe5bf40u32) as usize))));
                // b9e67e fstp dword ptr [esp + 0x18]
                s[25] = (f[0] as f32).to_bits();
                // b9e682 fld dword ptr [esp + 0x18]
                f[0] = ldq(s[25]);
                // b9e686 fstp dword ptr [esp]
                s[19] = (f[0] as f32).to_bits();
                // b9e689 call 0xb9e4f0
                { let mut q = Pushad { edi, esi, ebp, esp: 0, ebx, edx, ecx, eax }; let r = g2_b9e4f0(&mut q, s[19..].as_ptr(), w); eax = q.eax; ecx = q.ecx; edx = q.edx; f[0] = r; }
                // b9e68e fld qword ptr [0xe5bf70]
                f[1] = f64::from_bits(q64(rq((0xe5bf70u32) as usize)));
                // b9e694 add esp, 0xc
                // b9e697 fmul st(1), st(0)
                f[0] = xn(f[0], f[1], f[0] * f[1]);
                // b9e699 fmul dword ptr [esp + 0x14]
                f[1] = xn(f[1], ldq(s[27]), f[1] * ldq(s[27]));
                // b9e69d faddp st(1)
                f[0] = xn(f[0], f[1], f[0] + f[1]);
                // b9e69f fadd dword ptr [esp + 0x10]
                f[0] = xn(f[0], ldq(s[26]), f[0] + ldq(s[26]));
                // b9e6a3 fstp dword ptr [esp + 0xc]
                s[25] = (f[0] as f32).to_bits();
                // b9e6a7 fld dword ptr [esp + 0xc]
                f[0] = ldq(s[25]);
                // b9e6ab add esp, 8
                // b9e6ae ret 
                p.eax = eax; p.ecx = ecx; p.edx = edx; p.ebx = ebx; p.esi = esi; p.edi = edi; p.ebp = ebp;
                return f[0];
            }
            _ => unreachable!(),
        }
    }
}

// --- thunks -------------------------------------------------------------------------------------

thunk!(t0, 0, "ret", "nop");
thunk!(t1, 1, "ret", "nop");
thunk!(t2, 2, "ret", "nop");
thunk!(t3, 3, "ret", "nop");
thunk!(t4, 4, "ret", "nop");
thunk!(t5, 5, "ret", "nop");
thunk!(t6, 6, "ret", "nop");
thunk!(t7, 7, "ret", "nop");
thunk!(t8, 8, "ret", "nop");
thunk!(t9, 9, "ret", "nop");
thunk!(t10, 10, "ret", "nop");
thunk!(t11, 11, "ret", "nop");
thunk!(t12, 12, "ret", "nop");
thunk!(t13, 13, "ret", "nop");
thunk!(t14, 14, "ret", "nop");
thunk!(t15, 15, "ret", "nop");
thunk!(t16, 16, "ret", "nop");
thunk!(t17, 17, "ret", "nop");
thunk!(t18, 18, "ret 8", "nop");
thunk!(t19, 19, "ret", "fld qword ptr [esp]");
thunk!(t20, 20, "ret 4", "fld qword ptr [esp]");
thunk!(t21, 21, "ret 4", "fld qword ptr [esp]");
thunk!(t22, 22, "ret 4", "fld qword ptr [esp]");
thunk!(t23, 23, "ret 4", "fld qword ptr [esp]");
thunk!(t24, 24, "ret 4", "fld qword ptr [esp]");
thunk!(t25, 25, "ret 4", "fld qword ptr [esp]");
thunk!(t26, 26, "ret 8", "fld qword ptr [esp]");
thunk!(t27, 27, "ret 8", "fld qword ptr [esp]");
thunk!(t28, 28, "ret 8", "fld qword ptr [esp]");
thunk!(t29, 29, "ret 8", "fld qword ptr [esp]");
thunk!(t30, 30, "ret 4", "fld qword ptr [esp]");
thunk!(t31, 31, "ret", "fld qword ptr [esp]");
thunk!(t32, 32, "ret", "nop");
thunk!(t33, 33, "ret 4", "nop");
thunk!(t34, 34, "ret", "nop");
thunk!(t35, 35, "ret 4", "nop");
thunk!(t36, 36, "ret 4", "nop");
thunk!(t37, 37, "ret 8", "nop");
thunk!(t38, 38, "ret 8", "nop");
thunk!(t39, 39, "ret 8", "nop");
thunk!(t40, 40, "ret 8", "nop");
thunk!(t41, 41, "ret", "fld qword ptr [esp]");
thunk!(t42, 42, "ret", "fld qword ptr [esp]");
thunk!(t43, 43, "ret", "fld qword ptr [esp]");
thunk!(t44, 44, "ret", "fld qword ptr [esp]");
thunk!(t45, 45, "ret", "fld qword ptr [esp]");
thunk!(t46, 46, "ret", "fld qword ptr [esp]");
thunk!(t47, 47, "ret", "fld qword ptr [esp]");
thunk!(t48, 48, "ret", "fld qword ptr [esp]");
thunk!(t49, 49, "ret", "fld qword ptr [esp]");
thunk!(t50, 50, "ret 4", "fld qword ptr [esp]");
thunk!(t51, 51, "ret 4", "fld qword ptr [esp]");
thunk!(t52, 52, "ret 4", "fld qword ptr [esp]");
thunk!(t53, 53, "ret", "fld qword ptr [esp]");
thunk!(t54, 54, "ret", "nop");
thunk!(t55, 55, "ret", "nop");
thunk!(t56, 56, "ret", "nop");
thunk!(t57, 57, "ret", "nop");
thunk!(t58, 58, "ret", "nop");
thunk!(t59, 59, "ret", "nop");
thunk!(t60, 60, "ret", "nop");
thunk!(t61, 61, "ret", "nop");
thunk!(t62, 62, "ret", "nop");
thunk!(t63, 63, "ret 8", "nop");
thunk!(t64, 64, "ret 8", "nop");
thunk!(t65, 65, "ret 8", "nop");
thunk!(t66, 66, "ret 8", "nop");
thunk!(t67, 67, "ret 8", "nop");
thunk!(t68, 68, "ret 8", "nop");
thunk!(t69, 69, "ret 8", "nop");
thunk!(t70, 70, "ret 8", "nop");
thunk!(t71, 71, "ret 8", "nop");
thunk!(t72, 72, "ret 8", "nop");
thunk!(t73, 73, "ret 8", "nop");
thunk!(t74, 74, "ret 8", "nop");
thunk!(t75, 75, "ret 8", "nop");
thunk!(t76, 76, "ret 8", "nop");
thunk!(t77, 77, "ret 8", "nop");
thunk!(t78, 78, "ret 8", "nop");
thunk!(t79, 79, "ret 8", "nop");
thunk!(t80, 80, "ret 8", "nop");
thunk!(t81, 81, "ret 8", "nop");
thunk!(t82, 82, "ret 8", "nop");
thunk!(t83, 83, "ret 8", "nop");
thunk!(t84, 84, "ret 8", "nop");
thunk!(t85, 85, "ret 8", "nop");
thunk!(t86, 86, "ret 8", "nop");
thunk!(t87, 87, "ret 8", "nop");
thunk!(t88, 88, "ret 8", "nop");
thunk!(t89, 89, "ret 8", "nop");
thunk!(t90, 90, "ret 8", "nop");
thunk!(t91, 91, "ret 8", "nop");
thunk!(t92, 92, "ret 8", "nop");
thunk!(t93, 93, "ret 8", "nop");
thunk!(t94, 94, "ret 8", "nop");
thunk!(t95, 95, "ret 8", "nop");
thunk!(t96, 96, "ret 8", "nop");
thunk!(t97, 97, "ret 8", "nop");
thunk!(t98, 98, "ret 8", "nop");
thunk!(t99, 99, "ret 8", "nop");
thunk!(t100, 100, "ret 8", "nop");
thunk!(t101, 101, "ret 8", "nop");
thunk!(t102, 102, "ret 8", "nop");
thunk!(t103, 103, "ret 8", "nop");
thunk!(t104, 104, "ret 8", "nop");
thunk!(t105, 105, "ret 8", "nop");
thunk!(t106, 106, "ret 8", "nop");
thunk!(t107, 107, "ret 8", "nop");
thunk!(t108, 108, "ret 8", "nop");
thunk!(t109, 109, "ret 8", "nop");
thunk!(t110, 110, "ret 8", "nop");
thunk!(t111, 111, "ret 8", "nop");
thunk!(t112, 112, "ret 8", "nop");
thunk!(t113, 113, "ret 8", "nop");
thunk!(t114, 114, "ret 8", "nop");
thunk!(t115, 115, "ret 8", "nop");
thunk!(t116, 116, "ret 4", "nop");
thunk!(t117, 117, "ret", "fld qword ptr [esp]");
thunk!(t118, 118, "ret", "fld qword ptr [esp]");
thunk!(t119, 119, "ret", "nop");
thunk!(t120, 120, "ret", "nop");
thunk!(t121, 121, "ret", "fld qword ptr [esp]");
thunk!(t122, 122, "ret", "fld qword ptr [esp]");
thunk!(t123, 123, "ret", "fld qword ptr [esp]");
thunk!(t124, 124, "ret", "fld qword ptr [esp]");
thunk!(t125, 125, "ret", "fld qword ptr [esp]");
thunk!(t126, 126, "ret", "fld qword ptr [esp]");
thunk!(t127, 127, "ret", "fld qword ptr [esp]");
thunk!(t128, 128, "ret", "fld qword ptr [esp]");
thunk!(t129, 129, "ret", "fld qword ptr [esp]");
thunk!(t130, 130, "ret", "fld qword ptr [esp]");
thunk!(t131, 131, "ret", "fld qword ptr [esp]");
thunk!(t132, 132, "ret", "fld qword ptr [esp]");
thunk!(t133, 133, "ret", "fld qword ptr [esp]");
thunk!(t134, 134, "ret", "fld qword ptr [esp]");
thunk!(t135, 135, "ret", "fld qword ptr [esp]");
thunk!(t136, 136, "ret", "fld qword ptr [esp]");

// --- the table ------------------------------------------------------------------------------------

const COUNT: usize = 137;

static PORTS: [Port; COUNT] = [
    port("luafn_menu_open_enter_name_result", 0x00753410, &B0, 1, 0, false, 0xffffffff, g_753410 as Run, t0),
    port("luafn_get_current_difficulty", 0x00775820, &B1, 1, 0, false, 0xffffffff, g_775820 as Run, t1),
    port("luafn_pause_menu_get_state", 0x0077aa70, &B2, 1, 0, false, 0xffffffff, g_77aa70 as Run, t2),
    port("luafn_get_disconnected_reason", 0x007ee590, &B3, 1, 0, false, 0xffffffff, g_7ee590 as Run, t3),
    port("luafn_multi_get_sort_method", 0x007ef9a0, &B4, 1, 0, false, 0xffffffff, g_7ef9a0 as Run, t4),
    port("luafn_open_vint_dialog_check_done", 0x00a3d990, &B5, 1, 0, false, 0xffffffff, g_a3d990 as Run, t5),
    port("luafn_pc_capture_input_key_get_result", 0x00b90c60, &B6, 1, 0, false, 0xffffffff, g_b90c60 as Run, t6),
    port("luafn_pc_capture_input_pad_get_result", 0x00b90d70, &B7, 1, 0, false, 0xffffffff, g_b90d70 as Run, t7),
    port("luafn_pcr_get_current_voice_number", 0x007d28e0, &B8, 1, 0, false, 0xffffffff, g_7d28e0 as Run, t8),
    port("luafn_get_player_respect", 0x00a41850, &B9, 1, 0, false, 0xffffffff, g_a41850 as Run, t9),
    port("luafn_fade_get_percent", 0x00a2e510, &B10, 1, 0, false, 0xffffffff, g_a2e510 as Run, t10),
    port("luafn_mission_debug", 0x00a398c0, &B11, 1, 0, false, 0xffffffff, g_a398c0 as Run, t11),
    port("luafn_build_get_build_type", 0x00a47b50, &B12, 1, 0, false, 0xffffffff, g_a47b50 as Run, t12),
    port("luafn_get_frame_time", 0x00d74500, &B13, 1, 0, false, 0xffffffff, g_d74500 as Run, t13),
    port("luafn_get_frame_time_real", 0x00d74530, &B14, 1, 0, false, 0xffffffff, g_d74530 as Run, t14),
    port("luaB_gcinfo", 0x00d71d50, &B15, 1, 0, false, 0xffffffff, g_d71d50 as Run, t15),
    port("luafn_cellphone_get_time", 0x00789480, &B16, 1, 0, false, 0xffffffff, g_789480 as Run, t16),
    port("luafn_npc_respawn_dist_reset", 0x00a3cfe0, &B17, 0, 0, false, 0xffffffff, g_a3cfe0 as Run, t17),
    port("script_thread_push_number", 0x00cdfc60, &B18, 2, 8, false, 0xffffffff, g_cdfc60 as Run, t18),
    port("fuzz_crime::get_bar_progress", 0x005bc250, &B19, 0, 0, true, 0xffffffff, g_5bc250 as Run, t19),
    port("deprecated_slider_widget::get_width", 0x00761860, &B20, 1, 4, true, 0xffffffff, g_761860 as Run, t20),
    port("deprecated_slider_widget::get_height", 0x00761870, &B21, 1, 4, true, 0xffffffff, g_761870 as Run, t21),
    port("widget_activities::get_width", 0x0077b2e0, &B22, 1, 4, true, 0xffffffff, g_77b2e0 as Run, t22),
    port("widget_divider::get_width", 0x00781df0, &B23, 1, 4, true, 0xffffffff, g_781df0 as Run, t23),
    port("widget_story::get_width", 0x00781f40, &B24, 1, 4, true, 0xffffffff, g_781f40 as Run, t24),
    port("widget_story::get_height", 0x00781f50, &B25, 1, 4, true, 0xffffffff, g_781f50 as Run, t25),
    port("r_dynamic_license_plate_inst::min_plane_distance", 0x00aefe60, &B26, 2, 8, true, 0xffffffff, g_aefe60 as Run, t26),
    port("r_dynamic_water_detailed_inst::min_plane_distance", 0x00b75c30, &B27, 2, 8, true, 0xffffffff, g_b75c30 as Run, t27),
    port("r_dynamic_water_simple_inst::min_plane_distance", 0x00b75ca0, &B28, 2, 8, true, 0xffffffff, g_b75ca0 as Run, t28),
    port("r_distant_vehicle_headlight_inst::min_plane_distance", 0x00b7eeb0, &B29, 2, 8, true, 0xffffffff, g_b7eeb0 as Run, t29),
    port("IDirect3DDevice9Lost::GetNPatchMode", 0x00cf5350, &B30, 1, 4, true, 0xffffffff, g_cf5350 as Run, t30),
    port("vec3_distance_sq_dup", 0x00404a20, &B31, 0, 0, true, 0xffffffff, g_404a20 as Run, t31),
    port("vec3_mul_mat33_in_place", 0x00bde570, &B32, 0, 0, false, 0xffffffff, g_bde570 as Run, t32),
    port("vec3_mul_mat33", 0x00bdde20, &B33, 1, 4, false, 0xffffffff, g_bdde20 as Run, t33),
    port("nav_tri_centroid", 0x00a0fe30, &B34, 0, 0, false, 0xffffffff, g_a0fe30 as Run, t34),
    port("matrix2x3_mul", 0x00729f30, &B35, 1, 4, false, 0xffffffff, g_729f30 as Run, t35),
    port("normal_unpack_ubyte4", 0x00bdb4f0, &B36, 1, 4, false, 0xffffffff, g_bdb4f0 as Run, t36),
    port("effect_object::get_local_bbox", 0x0050b440, &B37, 2, 8, false, 0xffffffff, g_50b440 as Run, t37),
    port("thin_control_text::set_position", 0x007685c0, &B38, 2, 8, false, 0xffffffff, g_7685c0 as Run, t38),
    port("thin_control_text_button::set_position", 0x00768a70, &B39, 2, 8, false, 0xffffffff, g_768a70 as Run, t39),
    port("thin_control_toggle_button::set_position", 0x00768ea0, &B40, 2, 8, false, 0xffffffff, g_768ea0 as Run, t40),
    port("vint_ease_linear", 0x00b9dd80, &B41, 3, 0, true, 0xffffffff, g_b9dd80 as Run, t41),
    port("vint_ease_in_quad", 0x00b9df70, &B42, 3, 0, true, 0xffffffff, g_b9df70 as Run, t42),
    port("vint_ease_out_quad", 0x00b9df90, &B43, 3, 0, true, 0xffffffff, g_b9df90 as Run, t43),
    port("vint_ease_in_cubic", 0x00b9e030, &B44, 3, 0, true, 0xffffffff, g_b9e030 as Run, t44),
    port("vint_ease_out_cubic", 0x00b9e050, &B45, 3, 0, true, 0xffffffff, g_b9e050 as Run, t45),
    port("vint_ease_in_quart", 0x00b9e100, &B46, 3, 0, true, 0xffffffff, g_b9e100 as Run, t46),
    port("vint_ease_out_quart", 0x00b9e120, &B47, 3, 0, true, 0xffffffff, g_b9e120 as Run, t47),
    port("vint_ease_in_quint", 0x00b9e1e0, &B48, 3, 0, true, 0xffffffff, g_b9e1e0 as Run, t48),
    port("vint_ease_out_quint", 0x00b9e200, &B49, 3, 0, true, 0xffffffff, g_b9e200 as Run, t49),
    port("widget_tabbar::get_height", 0x0074e8d0, &B50, 1, 4, true, 0xffffffff, tabbar_get_height as Run, t50),
    port("pending_menu_widget::get_width", 0x007e32d0, &B51, 1, 4, true, 0xffffff00, pending_dim::<0x20> as Run, t51),
    port("pending_menu_widget::get_height", 0x007e3310, &B52, 1, 4, true, 0xffffff00, pending_dim::<0x24> as Run, t52),
    port("rng_float01", 0x00bdce30, &B53, 0, 0, true, 0xffffffff, rng_float01 as Run, t53),
    port("luafn_get_mp_mode_index", 0x007ee300, &B54, 1, 0, false, 0xffffffff, lua_mp_mode_index as Run, t54),
    port("luafn_get_mp_map_index", 0x007ee340, &B55, 1, 0, false, 0xffffffff, lua_mp_map_index as Run, t55),
    port("luafn_get_coop_join_type", 0x007ee970, &B56, 1, 0, false, 0xffffffff, lua_coop_join_type as Run, t56),
    port("luafn_get_coop_friendly_fire", 0x007eea50, &B57, 1, 0, false, 0xffffffff, lua_coop_friendly_fire as Run, t57),
    port("luafn_get_multi_clock_time", 0x0089d8c0, &B58, 1, 0, false, 0xffffffff, lua_multi_clock_time as Run, t58),
    port("luafn_demo_derby_get_menu_type", 0x005ee840, &B59, 1, 0, false, 0xffffffff, lua_demo_derby_menu_type as Run, t59),
    port("luafn_pcu_bra_required", 0x007ca4f0, &B60, 1, 0, false, 0xffffffff, lua_pcu_bra_required as Run, t60),
    port("luafn_vint_hack_is_std_res", 0x00b907b0, &B61, 1, 0, false, 0xffffffff, lua_hack_is_std_res as Run, t61),
    port("vint_value_get_vec2", 0x00b8cf80, &B62, 0, 0, false, 0xffffffff, vint_value_get_vec2 as Run, t62),
    port("vint_element_base::get_anchor", 0x00b96640, &B63, 2, 8, false, 0xffffffff, get_vec2::<0x5c, 0x60> as Run, t63),
    port("vint_element_base::get_offset", 0x00b966f0, &B64, 2, 8, false, 0xffffffff, get_vec2::<0x64, 0x68> as Run, t64),
    port("vint_element_base::get_scale", 0x00b96eb0, &B65, 2, 8, false, 0xffffffff, get_vec2::<0x90, 0x94> as Run, t65),
    port("vint_element_text::get_text_scale", 0x00ba3370, &B66, 2, 8, false, 0xffffffff, get_vec2::<0xac, 0xb0> as Run, t66),
    port("vint_element_clip::get_clip_size", 0x00ba4f70, &B67, 2, 8, false, 0xffffffff, get_vec2::<0x98, 0x9c> as Run, t67),
    port("vint_element_base::set_anchor", 0x00b96690, &B68, 2, 8, false, 0xffffffff, set_vec2::<0x5c, 0x60, false> as Run, t68),
    port("vint_element_base::set_offset", 0x00b96740, &B69, 2, 8, false, 0xffffffff, set_vec2::<0x64, 0x68, false> as Run, t69),
    port("vint_element_base::set_scale", 0x00b96f10, &B70, 2, 8, false, 0xffffffff, set_vec2::<0x90, 0x94, false> as Run, t70),
    port("vint_element_text::set_text_scale", 0x00ba33d0, &B71, 2, 8, false, 0xffffffff, set_vec2::<0xac, 0xb0, true> as Run, t71),
    port("vint_element_clip::set_clip_size", 0x00ba4fd0, &B72, 2, 8, false, 0xffffffff, set_vec2::<0x98, 0x9c, false> as Run, t72),
    port("vint_element_base::set_tint", 0x00b967f0, &B73, 2, 8, false, 0xffffffff, set_color3::<0x7c, 0x80, 0x84> as Run, t73),
    port("vint_element_gradient::set_gradient_nw", 0x00ba1c90, &B74, 2, 8, false, 0xffffffff, set_color3::<0xc4, 0xc8, 0xcc> as Run, t74),
    port("vint_element_gradient::set_gradient_ne", 0x00ba1d60, &B75, 2, 8, false, 0xffffffff, set_color3::<0xd0, 0xd4, 0xd8> as Run, t75),
    port("vint_element_gradient::set_gradient_sw", 0x00ba1e30, &B76, 2, 8, false, 0xffffffff, set_color3::<0xdc, 0xe0, 0xe4> as Run, t76),
    port("vint_element_gradient::set_gradient_se", 0x00ba1f00, &B77, 2, 8, false, 0xffffffff, set_color3::<0xe8, 0xec, 0xf0> as Run, t77),
    port("vint_element_bitmap::get_source_nw", 0x00ba1100, &B78, 2, 8, false, 0xffffffff, get_source::<0xa8, 0xac> as Run, t78),
    port("vint_element_bitmap::get_source_se", 0x00ba1260, &B79, 2, 8, false, 0xffffffff, get_source::<0xb0, 0xb4> as Run, t79),
    port("vint_element_bitmap_circle::get_source_nw", 0x00ba5890, &B80, 2, 8, false, 0xffffffff, get_source::<0xa8, 0xac> as Run, t80),
    port("vint_element_bitmap_circle::get_source_se", 0x00ba59f0, &B81, 2, 8, false, 0xffffffff, get_source::<0xb0, 0xb4> as Run, t81),
    port("vint_element_text::set_wrap_width", 0x00ba36a0, &B82, 2, 8, false, 0xffffffff, text_set_wrap_width as Run, t82),
    port("vint_element_point::get_screen_size", 0x00ba4840, &B83, 2, 8, false, 0xffffffff, point_get_screen_size as Run, t83),
    port_c("vint_element_base::get_visible", 0x00b96520, &B84, &C84, 2, 8, false, 0xffffffff, 0, g2_b96520 as Run, t84),
    port_c("vint_element_base::get_mask", 0x00b965b0, &B85, &C85, 2, 8, false, 0xffffffff, 0, g2_b965b0 as Run, t85),
    port_c("vint_element_base::get_alpha", 0x00b96860, &B86, &C86, 2, 8, false, 0xffffffff, 0, g2_b96860 as Run, t86),
    port_c("vint_element_base::set_alpha", 0x00b968c0, &B87, &C87, 2, 8, false, 0xffffffff, 0, g2_b968c0 as Run, t87),
    port_c("vint_element_base::get_rotation", 0x00b96dc0, &B88, &C88, 2, 8, false, 0xffffffff, 0, g2_b96dc0 as Run, t88),
    port_c("vint_element_base::set_rotation", 0x00b96e20, &B89, &C89, 2, 8, false, 0xffffffff, 0, g2_b96e20 as Run, t89),
    port_c("vint_object_animation::set_start_time", 0x00b9cf10, &B90, &C90, 2, 8, false, 0xffffffff, 0, g2_b9cf10 as Run, t90),
    port_c("vint_object_animation::get_start_time", 0x00b9cfb0, &B91, &C91, 2, 8, false, 0xffffffff, 0, g2_b9cfb0 as Run, t91),
    port_c("vint_object_animation::get_is_paused", 0x00b9d160, &B92, &C92, 2, 8, false, 0xffffffff, 0, g2_b9d160 as Run, t92),
    port_c("vint_object_tween::get_duration", 0x00b9f670, &B93, &C93, 2, 8, false, 0xffffffff, 0, g2_b9f670 as Run, t93),
    port_c("vint_object_tween::get_start_time", 0x00b9f760, &B94, &C94, 2, 8, false, 0xffffffff, 0, g2_b9f760 as Run, t94),
    port_c("vint_object_tween::get_max_loops", 0x00b9fb00, &B95, &C95, 2, 8, false, 0xffffffff, 0, g2_b9fb00 as Run, t95),
    port_c("vint_object_tween::set_max_loops", 0x00b9fb60, &B96, &C96, 2, 8, false, 0xffffffff, 0, g2_b9fb60 as Run, t96),
    port_c("vint_element_bitmap::get_custom_source_coords", 0x00ba1400, &B97, &C97, 2, 8, false, 0xffffffff, 0, g2_ba1400 as Run, t97),
    port_c("vint_element_gradient::get_alpha_nw", 0x00ba1f70, &B98, &C98, 2, 8, false, 0xffffffff, 0, g2_ba1f70 as Run, t98),
    port_c("vint_element_gradient::set_alpha_nw", 0x00ba1fd0, &B99, &C99, 2, 8, false, 0xffffffff, 0, g2_ba1fd0 as Run, t99),
    port_c("vint_element_gradient::get_alpha_ne", 0x00ba2070, &B100, &C100, 2, 8, false, 0xffffffff, 0, g2_ba2070 as Run, t100),
    port_c("vint_element_gradient::set_alpha_ne", 0x00ba20d0, &B101, &C101, 2, 8, false, 0xffffffff, 0, g2_ba20d0 as Run, t101),
    port_c("vint_element_gradient::get_alpha_sw", 0x00ba2170, &B102, &C102, 2, 8, false, 0xffffffff, 0, g2_ba2170 as Run, t102),
    port_c("vint_element_gradient::set_alpha_sw", 0x00ba21d0, &B103, &C103, 2, 8, false, 0xffffffff, 0, g2_ba21d0 as Run, t103),
    port_c("vint_element_gradient::get_alpha_se", 0x00ba2270, &B104, &C104, 2, 8, false, 0xffffffff, 0, g2_ba2270 as Run, t104),
    port_c("vint_element_gradient::set_alpha_se", 0x00ba22d0, &B105, &C105, 2, 8, false, 0xffffffff, 0, g2_ba22d0 as Run, t105),
    port_c("vint_element_text::get_word_wrap", 0x00ba35a0, &B106, &C106, 2, 8, false, 0xffffffff, 0, g2_ba35a0 as Run, t106),
    port_c("vint_element_text::get_wrap_width", 0x00ba3640, &B107, &C107, 2, 8, false, 0xffffffff, 0, g2_ba3640 as Run, t107),
    port_c("vint_element_text::get_line_frame_enable", 0x00ba3cf0, &B108, &C108, 2, 8, false, 0xffffffff, 0, g2_ba3cf0 as Run, t108),
    port_c("vint_element_clip::get_clip_enabled", 0x00ba5030, &B109, &C109, 2, 8, false, 0xffffffff, 0, g2_ba5030 as Run, t109),
    port_c("vint_element_bitmap_circle::get_start_angle", 0x00ba5b50, &B110, &C110, 2, 8, false, 0xffffffff, 0, g2_ba5b50 as Run, t110),
    port_c("vint_element_bitmap_circle::set_start_angle", 0x00ba5bb0, &B111, &C111, 2, 8, false, 0xffffffff, 0, g2_ba5bb0 as Run, t111),
    port_c("vint_element_bitmap_circle::get_end_angle", 0x00ba5c50, &B112, &C112, 2, 8, false, 0xffffffff, 0, g2_ba5c50 as Run, t112),
    port_c("vint_element_bitmap_circle::set_end_angle", 0x00ba5cb0, &B113, &C113, 2, 8, false, 0xffffffff, 0, g2_ba5cb0 as Run, t113),
    port_c("vint_element_sr2_map::get_zoom", 0x00ba6400, &B114, &C114, 2, 8, false, 0xffffffff, 0, g2_ba6400 as Run, t114),
    port_c("vint_element_sr2_map::set_zoom", 0x00ba6460, &B115, &C115, 2, 8, false, 0xffffffff, 0, g2_ba6460 as Run, t115),
    port_c("vint_value_set_float", 0x007903d0, &B116, &C116, 1, 4, false, 0xffffffff, 0, g2_7903d0 as Run, t116),
    port_c("vint_value_to_float", 0x005e4970, &B117, &C117, 0, 0, true, 0xffffffff, 0, g2_5e4970 as Run, t117),
    port_c("vint_value_to_double", 0x00b8cf40, &B118, &C118, 0, 0, true, 0xffffffff, 0, g2_b8cf40 as Run, t118),
    port_c("vint_value_set_vec2", 0x00b8a150, &B119, &C119, 0, 0, false, 0xffffffff, 0, g2_b8a150 as Run, t119),
    port_c("vint_value_get_color3", 0x00b956b0, &B120, &C120, 0, 0, false, 0xffffffff, 0, g2_b956b0 as Run, t120),
    port_c("vint_ease_in_exp", 0x00b9dda0, &B121, &C121, 3, 0, true, 0xffffffff, 2, g2_b9dda0 as Run, t121),
    port_c("vint_ease_out_exp", 0x00b9de10, &B122, &C122, 3, 0, true, 0xffffffff, 2, g2_b9de10 as Run, t122),
    port_c("vint_ease_in_out_exp", 0x00b9de90, &B123, &C123, 3, 0, true, 0xffffffff, 2, g2_b9de90 as Run, t123),
    port_c("vint_ease_in_out_quad", 0x00b9dfc0, &B124, &C124, 3, 0, true, 0xffffffff, 0, g2_b9dfc0 as Run, t124),
    port_c("vint_ease_in_out_cubic", 0x00b9e080, &B125, &C125, 3, 0, true, 0xffffffff, 0, g2_b9e080 as Run, t125),
    port_c("vint_ease_in_out_quart", 0x00b9e160, &B126, &C126, 3, 0, true, 0xffffffff, 0, g2_b9e160 as Run, t126),
    port_c("vint_ease_in_out_quint", 0x00b9e230, &B127, &C127, 3, 0, true, 0xffffffff, 0, g2_b9e230 as Run, t127),
    port_c("vint_ease_in_sin", 0x00b9e2b0, &B128, &C128, 3, 0, true, 0xffffffff, 2, g2_b9e2b0 as Run, t128),
    port_c("vint_ease_out_sin", 0x00b9e2f0, &B129, &C129, 3, 0, true, 0xffffffff, 2, g2_b9e2f0 as Run, t129),
    port_c("vint_ease_in_out_sin", 0x00b9e320, &B130, &C130, 3, 0, true, 0xffffffff, 2, g2_b9e320 as Run, t130),
    port_c("vint_ease_in_circ", 0x00b9e360, &B131, &C131, 3, 0, true, 0xffffffff, 2, g2_b9e360 as Run, t131),
    port_c("vint_ease_out_circ", 0x00b9e3a0, &B132, &C132, 3, 0, true, 0xffffffff, 2, g2_b9e3a0 as Run, t132),
    port_c("vint_ease_in_out_circ", 0x00b9e3e0, &B133, &C133, 3, 0, true, 0xffffffff, 2, g2_b9e3e0 as Run, t133),
    port_c("vint_ease_in_bounce", 0x00b9e4a0, &B134, &C134, 3, 0, true, 0xffffffff, 0, g2_b9e4a0 as Run, t134),
    port_c("vint_ease_out_bounce", 0x00b9e4f0, &B135, &C135, 3, 0, true, 0xffffffff, 0, g2_b9e4f0 as Run, t135),
    port_c("vint_ease_in_out_bounce", 0x00b9e5f0, &B136, &C136, 3, 0, true, 0xffffffff, 0, g2_b9e5f0 as Run, t136),
];

// --- install ------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for port in PORTS.iter() {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, port.body, port.call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_leaves_fp: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, port.body, port.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_leaves_fp: replaced {ok}/{COUNT} with Rust"));
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
    if parts.is_empty() { String::new() } else { format!("port_leaves_fp: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const B0: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x30, 0x80, 0x52, 0x02,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B1: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0xF4, 0x86, 0xE9, 0x00,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B2: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x6C, 0x83, 0x52, 0x02,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B3: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x4C, 0x93, 0x52, 0x02,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B4: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xDB, 0x05, 0x84,
    0xA9, 0x12, 0x02, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B5: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x20, 0x7D, 0xE8, 0x00,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B6: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x0C, 0x2E, 0xE8, 0x00,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B7: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xDB, 0x05, 0x0C, 0x2E, 0xE8, 0x00,
    0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B8: [u8; 48] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B,
    0x88, 0xB0, 0x13, 0x00, 0x00, 0xDB, 0x41, 0x0C, 0x8B, 0x45, 0x08, 0x8B,
    0x48, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xDD, 0x59, 0x08, 0x83,
    0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B9: [u8; 45] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0xDB,
    0x80, 0x00, 0x12, 0x00, 0x00, 0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7,
    0x01, 0x03, 0x00, 0x00, 0x00, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B10: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xD9, 0x05, 0xA0,
    0x6D, 0x52, 0x02, 0x8B, 0x48, 0x08, 0xDD, 0x59, 0x08, 0xC7, 0x01, 0x03,
    0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B11: [u8; 36] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xD9, 0xEE, 0x8B,
    0x48, 0x08, 0xDD, 0x59, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0x83,
    0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B12: [u8; 36] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xD9, 0xE8, 0x8B,
    0x48, 0x08, 0xDD, 0x59, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0x83,
    0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B13: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xD9, 0x05, 0x80,
    0x43, 0xE8, 0x00, 0x8B, 0x48, 0x08, 0xDD, 0x59, 0x08, 0xC7, 0x01, 0x03,
    0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B14: [u8; 40] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0xD9, 0x05, 0x80,
    0x43, 0xE8, 0x00, 0x8B, 0x48, 0x08, 0xDD, 0x59, 0x08, 0xC7, 0x01, 0x03,
    0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const B15: [u8; 79] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x51, 0x24, 0x8B, 0x48,
    0x08, 0xC1, 0xEA, 0x0A, 0x89, 0x54, 0x24, 0x04, 0xDB, 0x44, 0x24, 0x04,
    0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0x56, 0xBA, 0x10, 0x00, 0x00, 0x00,
    0xDD, 0x59, 0x08, 0x01, 0x50, 0x08, 0x8B, 0x70, 0x10, 0x8B, 0x76, 0x20,
    0x8B, 0x48, 0x08, 0xC1, 0xEE, 0x0A, 0x89, 0x74, 0x24, 0x08, 0xDB, 0x44,
    0x24, 0x08, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0x5E, 0xDD, 0x59, 0x08,
    0x01, 0x50, 0x08, 0x8D, 0x42, 0xF2, 0xC3,
];
const B16: [u8; 81] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x51, 0x0F, 0xB6, 0x05, 0x2C, 0xE7,
    0x40, 0x01, 0x89, 0x04, 0x24, 0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xDB,
    0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xBA, 0x10, 0x00, 0x00,
    0x00, 0x56, 0xDD, 0x59, 0x08, 0x01, 0x50, 0x08, 0x0F, 0xB6, 0x35, 0x2D,
    0xE7, 0x40, 0x01, 0x8B, 0x48, 0x08, 0x89, 0x74, 0x24, 0x04, 0xC7, 0x01,
    0x03, 0x00, 0x00, 0x00, 0x5E, 0xDB, 0x04, 0x24, 0xDD, 0x59, 0x08, 0x01,
    0x50, 0x08, 0x8D, 0x42, 0xF2, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B17: [u8; 11] = [
    0xD9, 0xEE, 0x33, 0xC0, 0xD9, 0x1D, 0x40, 0x97, 0x52, 0x02, 0xC3,
];
const B18: [u8; 30] = [
    0x8B, 0x41, 0x04, 0xDD, 0x44, 0x24, 0x04, 0x8B, 0x50, 0x08, 0xDD, 0x5A,
    0x08, 0xC7, 0x02, 0x03, 0x00, 0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0x83,
    0x41, 0x1C, 0x01, 0xC2, 0x08, 0x00,
];
const B19: [u8; 5] = [
    0xD9, 0xEE, 0xC3, 0xCC, 0xCC,
];
const B20: [u8; 6] = [
    0xD9, 0x41, 0x10, 0xC2, 0x04, 0x00,
];
const B21: [u8; 6] = [
    0xD9, 0x41, 0x14, 0xC2, 0x04, 0x00,
];
const B22: [u8; 35] = [
    0x51, 0x8B, 0x41, 0x14, 0x03, 0x41, 0x10, 0x89, 0x04, 0x24, 0xDB, 0x04,
    0x24, 0xDC, 0x0D, 0x28, 0xC2, 0xE5, 0x00, 0xDC, 0x05, 0x68, 0xBF, 0xE5,
    0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00,
];
const B23: [u8; 5] = [
    0xD9, 0xE8, 0xC2, 0x04, 0x00,
];
const B24: [u8; 9] = [
    0xD9, 0x05, 0x20, 0xC2, 0xE5, 0x00, 0xC2, 0x04, 0x00,
];
const B25: [u8; 9] = [
    0xD9, 0x81, 0x74, 0x09, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
const B26: [u8; 9] = [
    0xD9, 0x05, 0x5C, 0x72, 0xE8, 0x00, 0xC2, 0x08, 0x00,
];
const B27: [u8; 9] = [
    0xD9, 0x05, 0x5C, 0x72, 0xE8, 0x00, 0xC2, 0x08, 0x00,
];
const B28: [u8; 9] = [
    0xD9, 0x05, 0xF0, 0xC0, 0xE5, 0x00, 0xC2, 0x08, 0x00,
];
const B29: [u8; 9] = [
    0xD9, 0x05, 0x5C, 0x72, 0xE8, 0x00, 0xC2, 0x08, 0x00,
];
const B30: [u8; 5] = [
    0xD9, 0xEE, 0xC2, 0x04, 0x00,
];
const B31: [u8; 71] = [
    0x83, 0xEC, 0x08, 0xD9, 0x01, 0xD8, 0x20, 0xD9, 0x1C, 0x24, 0xD9, 0x04,
    0x24, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04, 0xD8, 0x60,
    0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24,
    0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD8, 0x60, 0x08, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x83, 0xC4, 0x08, 0xC3,
];
const B32: [u8; 111] = [
    0x83, 0xEC, 0x0C, 0xD9, 0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x41, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x40,
    0x0C, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0x00, 0xD9, 0x44,
    0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9, 0x40,
    0x18, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE,
    0xC1, 0xD9, 0x19, 0xD9, 0x40, 0x10, 0xD8, 0xCA, 0xD9, 0x40, 0x04, 0xD8,
    0xCC, 0xDE, 0xC1, 0xD9, 0x40, 0x1C, 0xD8, 0xCA, 0xDE, 0xC1, 0xD9, 0x59,
    0x04, 0xD9, 0x40, 0x14, 0xDE, 0xCA, 0xD9, 0x40, 0x08, 0xDE, 0xCB, 0xD9,
    0xC9, 0xDE, 0xC2, 0xD8, 0x48, 0x20, 0xDE, 0xC1, 0xD9, 0x59, 0x08, 0x83,
    0xC4, 0x0C, 0xC3,
];
const B33: [u8; 119] = [
    0x83, 0xEC, 0x08, 0x8B, 0x54, 0x24, 0x0C, 0xD9, 0x42, 0x04, 0xD9, 0x5C,
    0x24, 0x0C, 0xD9, 0x02, 0xD9, 0x1C, 0x24, 0xD9, 0x42, 0x08, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x41, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xDE,
    0xCA, 0xD9, 0x01, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCB,
    0xDE, 0xC1, 0xD9, 0x41, 0x18, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE,
    0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9, 0x18, 0xD9, 0x41, 0x04, 0xD8, 0xCB,
    0xD9, 0x41, 0x10, 0xD8, 0xCB, 0xDE, 0xC1, 0xD9, 0x41, 0x1C, 0xD8, 0xCA,
    0xDE, 0xC1, 0xD9, 0x58, 0x04, 0xD9, 0x41, 0x08, 0xDE, 0xCB, 0xD9, 0x41,
    0x14, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9, 0x41, 0x20, 0xDE, 0xCA,
    0xDE, 0xC1, 0xD9, 0x58, 0x08, 0x83, 0xC4, 0x08, 0xC2, 0x04, 0x00,
];
const B34: [u8; 127] = [
    0x83, 0xEC, 0x18, 0x8B, 0x42, 0x04, 0x8B, 0x0A, 0xD9, 0x00, 0xD8, 0x01,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x04, 0xD8, 0x41, 0x04, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x40, 0x08, 0x8B, 0x42, 0x08, 0xD8, 0x41, 0x08, 0xD9, 0x5C,
    0x24, 0x08, 0xD9, 0x00, 0xD8, 0x04, 0x24, 0xD9, 0x5C, 0x24, 0x0C, 0xD9,
    0x40, 0x04, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x40,
    0x08, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x0C, 0xDD, 0x05, 0xC0, 0xC4, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9,
    0x1C, 0x24, 0x8B, 0x04, 0x24, 0xD9, 0x44, 0x24, 0x10, 0x89, 0x06, 0xD8,
    0xC9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x04, 0x89, 0x4E, 0x04,
    0xD8, 0x4C, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x08, 0x8B, 0x54, 0x24, 0x08,
    0x89, 0x56, 0x08, 0x83, 0xC4, 0x18, 0xC3,
];
const B35: [u8; 109] = [
    0x8B, 0x4C, 0x24, 0x04, 0xD9, 0x02, 0xD8, 0x09, 0xD9, 0x42, 0x0C, 0xD8,
    0x49, 0x04, 0xDE, 0xC1, 0xD9, 0x18, 0xD9, 0x42, 0x10, 0xD8, 0x49, 0x04,
    0xD9, 0x42, 0x04, 0xD8, 0x09, 0xDE, 0xC1, 0xD9, 0x58, 0x04, 0xD9, 0x42,
    0x14, 0xD8, 0x49, 0x04, 0xD9, 0x42, 0x08, 0xD8, 0x09, 0xDE, 0xC1, 0xD8,
    0x41, 0x08, 0xD9, 0x58, 0x08, 0xD9, 0x41, 0x10, 0xD8, 0x4A, 0x0C, 0xD9,
    0x02, 0xD8, 0x49, 0x0C, 0xDE, 0xC1, 0xD9, 0x58, 0x0C, 0xD9, 0x42, 0x04,
    0xD8, 0x49, 0x0C, 0xD9, 0x41, 0x10, 0xD8, 0x4A, 0x10, 0xDE, 0xC1, 0xD9,
    0x58, 0x10, 0xD9, 0x42, 0x08, 0xD8, 0x49, 0x0C, 0xD9, 0x41, 0x10, 0xD8,
    0x4A, 0x14, 0xDE, 0xC1, 0xD8, 0x41, 0x14, 0xD9, 0x58, 0x14, 0xC2, 0x04,
    0x00,
];
const B36: [u8; 98] = [
    0x51, 0x0F, 0xB6, 0x10, 0x89, 0x14, 0x24, 0xDB, 0x04, 0x24, 0xDD, 0x05,
    0x38, 0xC0, 0xE5, 0x00, 0xDC, 0xF9, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00,
    0xDC, 0xCA, 0xD9, 0xE8, 0xDC, 0xEB, 0xD9, 0xCB, 0xD9, 0x19, 0x0F, 0xB6,
    0x50, 0x01, 0x89, 0x14, 0x24, 0xDB, 0x04, 0x24, 0xD8, 0xF2, 0xD8, 0xC9,
    0xD8, 0xE3, 0xD9, 0x59, 0x04, 0x0F, 0xB6, 0x50, 0x02, 0x89, 0x14, 0x24,
    0xDB, 0x04, 0x24, 0xD8, 0xF2, 0xD8, 0xC9, 0xD8, 0xE3, 0xD9, 0x59, 0x08,
    0x0F, 0xB6, 0x40, 0x03, 0x89, 0x04, 0x24, 0x8B, 0x4C, 0x24, 0x08, 0xDB,
    0x04, 0x24, 0xDE, 0xF2, 0xDE, 0xC9, 0xDE, 0xE1, 0xD9, 0x19, 0x59, 0xC2,
    0x04, 0x00,
];
const B37: [u8; 54] = [
    0x51, 0x8B, 0x81, 0xD0, 0x00, 0x00, 0x00, 0xD9, 0x40, 0x1C, 0x8B, 0x44,
    0x24, 0x08, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xD9, 0xE0,
    0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x10, 0xD9, 0x50, 0x04, 0xD9,
    0x58, 0x08, 0x8B, 0x44, 0x24, 0x0C, 0xD9, 0x10, 0xD9, 0x50, 0x04, 0xD9,
    0x58, 0x08, 0x59, 0xC2, 0x08, 0x00,
];
const B38: [u8; 75] = [
    0xD9, 0x41, 0x0C, 0xD8, 0x61, 0x04, 0xD9, 0x59, 0x0C, 0xD9, 0x41, 0x10,
    0xD8, 0x61, 0x08, 0xD9, 0x59, 0x10, 0xDB, 0x44, 0x24, 0x04, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x41, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE,
    0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x0C, 0xD9, 0x59, 0x04, 0xDB, 0x44, 0x24,
    0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x10, 0xD9, 0x44, 0x24, 0x04,
    0xD9, 0xC0, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x10, 0xD9, 0x59, 0x08,
    0xC2, 0x08, 0x00,
];
const B39: [u8; 75] = [
    0xD9, 0x41, 0x0C, 0xD8, 0x61, 0x04, 0xD9, 0x59, 0x0C, 0xD9, 0x41, 0x10,
    0xD8, 0x61, 0x08, 0xD9, 0x59, 0x10, 0xDB, 0x44, 0x24, 0x04, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x41, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE,
    0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x0C, 0xD9, 0x59, 0x04, 0xDB, 0x44, 0x24,
    0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x10, 0xD9, 0x44, 0x24, 0x04,
    0xD9, 0xC0, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x10, 0xD9, 0x59, 0x08,
    0xC2, 0x08, 0x00,
];
const B40: [u8; 75] = [
    0xD9, 0x41, 0x0C, 0xD8, 0x61, 0x04, 0xD9, 0x59, 0x0C, 0xD9, 0x41, 0x10,
    0xD8, 0x61, 0x08, 0xD9, 0x59, 0x10, 0xDB, 0x44, 0x24, 0x04, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x41, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE,
    0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x0C, 0xD9, 0x59, 0x04, 0xDB, 0x44, 0x24,
    0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x10, 0xD9, 0x44, 0x24, 0x04,
    0xD9, 0xC0, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x59, 0x10, 0xD9, 0x59, 0x08,
    0xC2, 0x08, 0x00,
];
const B41: [u8; 21] = [
    0xD9, 0x44, 0x24, 0x04, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0x44, 0x24, 0x08,
    0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const B42: [u8; 25] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x0C, 0xDE, 0xC9,
    0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04,
    0xC3,
];
const B43: [u8; 35] = [
    0xD9, 0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C,
    0x24, 0x0C, 0xD9, 0xC9, 0xDC, 0x25, 0xE8, 0xBF, 0xE5, 0x00, 0xDE, 0xC9,
    0xDE, 0xE9, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xC3,
];
const B44: [u8; 27] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0xC9,
    0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0xC3,
];
const B45: [u8; 41] = [
    0xD9, 0x44, 0x24, 0x04, 0xDC, 0x25, 0x40, 0xBF, 0xE5, 0x00, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x0C,
    0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const B46: [u8; 29] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0xC9,
    0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const B47: [u8; 51] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD9, 0xC0, 0xDE, 0xC9,
    0xD9, 0xC1, 0xDE, 0xC9, 0xDE, 0xC9, 0xDE, 0xE1, 0xD9, 0x44, 0x24, 0x0C,
    0xDE, 0xC9, 0xD8, 0x6C, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0xC3,
];
const B48: [u8; 31] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0xC9,
    0xD8, 0xC9, 0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const B49: [u8; 47] = [
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDC, 0xC8, 0xD8, 0xC9,
    0xD8, 0xC9, 0xDE, 0xC9, 0xDE, 0xC1, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0x44,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const B50: [u8; 30] = [
    0x51, 0x8B, 0x41, 0x1C, 0xDB, 0x41, 0x1C, 0x85, 0xC0, 0x7D, 0x06, 0xD8,
    0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xD8, 0x49, 0x28, 0xD9, 0x1C, 0x24, 0xD9,
    0x04, 0x24, 0x59, 0xC2, 0x04, 0x00,
];
const B51: [u8; 57] = [
    0x51, 0xD9, 0x41, 0x18, 0xD9, 0x41, 0x1C, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6,
    0xC4, 0x05, 0x7A, 0x05, 0xD9, 0x41, 0x18, 0xEB, 0x03, 0xD9, 0x41, 0x1C,
    0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x41, 0x20, 0xD8, 0xD9, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x05, 0xDD, 0xD8, 0xD9, 0x41, 0x20, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00,
];
const B52: [u8; 57] = [
    0x51, 0xD9, 0x41, 0x18, 0xD9, 0x41, 0x1C, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6,
    0xC4, 0x05, 0x7A, 0x05, 0xD9, 0x41, 0x18, 0xEB, 0x03, 0xD9, 0x41, 0x1C,
    0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x41, 0x24, 0xD8, 0xD9, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x05, 0xDD, 0xD8, 0xD9, 0x41, 0x24, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00,
];
const B53: [u8; 71] = [
    0x51, 0x8B, 0x88, 0xC4, 0x09, 0x00, 0x00, 0x8B, 0x94, 0x88, 0xC8, 0x09,
    0x00, 0x00, 0x83, 0xC1, 0x01, 0x81, 0xF9, 0x00, 0x08, 0x00, 0x00, 0x89,
    0x88, 0xC4, 0x09, 0x00, 0x00, 0x75, 0x0A, 0xC7, 0x80, 0xC4, 0x09, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x85, 0xD2, 0x89, 0x14, 0x24, 0xDB, 0x04,
    0x24, 0x7D, 0x06, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xDC, 0x0D, 0x30,
    0xC0, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
const B54: [u8; 57] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0x8A, 0x15, 0x10,
    0xB2, 0xE8, 0x00, 0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0x80, 0xEA, 0x0B,
    0x0F, 0xB6, 0xD2, 0x89, 0x54, 0x24, 0x04, 0xC7, 0x01, 0x03, 0x00, 0x00,
    0x00, 0xDB, 0x44, 0x24, 0x04, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B55: [u8; 90] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0x66, 0x83, 0x3D,
    0x14, 0xB2, 0xE8, 0x00, 0xFF, 0x75, 0x13, 0xA0, 0x10, 0xB2, 0xE8, 0x00,
    0x2C, 0x0B, 0x0F, 0xB6, 0xC8, 0x8B, 0x14, 0x8D, 0x64, 0x00, 0x13, 0x02,
    0xEB, 0x06, 0x8B, 0x15, 0x94, 0x8D, 0x52, 0x02, 0x85, 0xD2, 0x8B, 0x45,
    0x08, 0x8B, 0x48, 0x08, 0x89, 0x54, 0x24, 0x04, 0xDB, 0x44, 0x24, 0x04,
    0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0x7D, 0x06, 0xDC, 0x05, 0xE0, 0xC0,
    0xE5, 0x00, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B56: [u8; 90] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xA1, 0x10, 0x73,
    0xE9, 0x00, 0x33, 0xC9, 0x2B, 0xC1, 0x89, 0x4C, 0x24, 0x04, 0x74, 0x1E,
    0x83, 0xE8, 0x01, 0x74, 0x0F, 0x83, 0xE8, 0x01, 0x75, 0x18, 0xC7, 0x44,
    0x24, 0x04, 0x02, 0x00, 0x00, 0x00, 0xEB, 0x0E, 0xC7, 0x44, 0x24, 0x04,
    0x01, 0x00, 0x00, 0x00, 0xEB, 0x04, 0x89, 0x4C, 0x24, 0x04, 0x8B, 0x45,
    0x08, 0xDB, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00,
    0x00, 0x00, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B57: [u8; 90] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xA1, 0x14, 0x73,
    0xE9, 0x00, 0x33, 0xC9, 0x2B, 0xC1, 0x89, 0x4C, 0x24, 0x04, 0x74, 0x1A,
    0x83, 0xE8, 0x01, 0x74, 0x0F, 0x83, 0xE8, 0x01, 0x75, 0x18, 0xC7, 0x44,
    0x24, 0x04, 0x01, 0x00, 0x00, 0x00, 0xEB, 0x0E, 0x89, 0x4C, 0x24, 0x04,
    0xEB, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x02, 0x00, 0x00, 0x00, 0x8B, 0x45,
    0x08, 0xDB, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00,
    0x00, 0x00, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B58: [u8; 75] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x83,
    0xEC, 0x08, 0x85, 0xC0, 0x74, 0x05, 0x8B, 0x50, 0x08, 0xEB, 0x06, 0x8B,
    0x15, 0x60, 0xB1, 0x12, 0x02, 0x85, 0xD2, 0x8B, 0x45, 0x08, 0x8B, 0x48,
    0x08, 0x89, 0x54, 0x24, 0x04, 0xDB, 0x44, 0x24, 0x04, 0xC7, 0x01, 0x03,
    0x00, 0x00, 0x00, 0x7D, 0x06, 0xDC, 0x05, 0xE0, 0xC0, 0xE5, 0x00, 0xDD,
    0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x8B,
    0xE5, 0x5D, 0xC3,
];
const B59: [u8; 90] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x80, 0x3D, 0x09, 0x78, 0x52, 0x02,
    0x00, 0x74, 0x12, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xC7, 0x05, 0x04,
    0x7D, 0xE9, 0x00, 0x02, 0x00, 0x00, 0x00, 0xEB, 0x1D, 0xA1, 0x04, 0x7D,
    0xE9, 0x00, 0x83, 0xF8, 0x02, 0x75, 0x08, 0xDD, 0x05, 0xE8, 0xBF, 0xE5,
    0x00, 0xEB, 0x0B, 0x83, 0xF8, 0x01, 0x75, 0x04, 0xD9, 0xE8, 0xEB, 0x02,
    0xD9, 0xEE, 0x8B, 0x45, 0x08, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x03, 0x00,
    0x00, 0x00, 0xDD, 0x59, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B60: [u8; 99] = [
    0x51, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x88, 0xB0, 0x13, 0x00, 0x00,
    0x85, 0xC9, 0x74, 0x29, 0xA1, 0xB8, 0x97, 0x52, 0x02, 0x85, 0xC0, 0x74,
    0x20, 0x8B, 0x40, 0x1C, 0x8D, 0x14, 0x40, 0x8B, 0x41, 0x04, 0xD9, 0x44,
    0x90, 0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x05, 0x8C, 0x1C, 0xDD, 0x00, 0xD8,
    0x1C, 0x24, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x04, 0xB1, 0x01, 0xEB,
    0x02, 0x32, 0xC9, 0x8B, 0x44, 0x24, 0x08, 0x33, 0xD2, 0x84, 0xC9, 0x0F,
    0x95, 0xC2, 0x56, 0x8B, 0x70, 0x08, 0xC7, 0x06, 0x01, 0x00, 0x00, 0x00,
    0x89, 0x56, 0x08, 0x83, 0x40, 0x08, 0x10, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x5E, 0x59, 0xC3,
];
const B61: [u8; 56] = [
    0xD9, 0xE8, 0xD8, 0x1D, 0xAC, 0xBF, 0x2D, 0x02, 0xDF, 0xE0, 0xF6, 0xC4,
    0x41, 0x75, 0x04, 0xB1, 0x01, 0xEB, 0x02, 0x32, 0xC9, 0x8B, 0x44, 0x24,
    0x04, 0x33, 0xD2, 0x84, 0xC9, 0x0F, 0x95, 0xC2, 0x56, 0x8B, 0x70, 0x08,
    0xC7, 0x06, 0x01, 0x00, 0x00, 0x00, 0x89, 0x56, 0x08, 0x83, 0x40, 0x08,
    0x10, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B62: [u8; 24] = [
    0x83, 0x39, 0x07, 0xD9, 0xEE, 0xD9, 0x10, 0xD9, 0x58, 0x04, 0x75, 0x0B,
    0xD9, 0x41, 0x04, 0xD9, 0x18, 0xD9, 0x41, 0x08, 0xD9, 0x58, 0x04, 0xC3,
];
const B63: [u8; 48] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x07, 0x83, 0xFA,
    0x07, 0x75, 0x1C, 0xEB, 0x06, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x41, 0x60, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x5C, 0xD9, 0x58, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58, 0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B64: [u8; 48] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x07, 0x83, 0xFA,
    0x07, 0x75, 0x1C, 0xEB, 0x06, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x41, 0x68, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x64, 0xD9, 0x58, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58, 0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B65: [u8; 54] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x07, 0x83, 0xFA,
    0x07, 0x75, 0x22, 0xEB, 0x06, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x81, 0x94, 0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x81, 0x90,
    0x00, 0x00, 0x00, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58,
    0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B66: [u8; 54] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x07, 0x83, 0xFA,
    0x07, 0x75, 0x22, 0xEB, 0x06, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x81, 0xB0, 0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x81, 0xAC,
    0x00, 0x00, 0x00, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58,
    0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B67: [u8; 54] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x07, 0x83, 0xFA,
    0x07, 0x75, 0x22, 0xEB, 0x06, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x81, 0x9C, 0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x81, 0x98,
    0x00, 0x00, 0x00, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58,
    0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B68: [u8; 55] = [
    0x83, 0xEC, 0x08, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x0C, 0x83, 0x38, 0x07,
    0xD9, 0x14, 0x24, 0xD9, 0x5C, 0x24, 0x04, 0x75, 0x0D, 0xD9, 0x40, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x04,
    0x24, 0x8B, 0x54, 0x24, 0x04, 0x89, 0x41, 0x5C, 0x89, 0x51, 0x60, 0xB0,
    0x01, 0x83, 0xC4, 0x08, 0xC2, 0x08, 0x00,
];
const B69: [u8; 55] = [
    0x83, 0xEC, 0x08, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x0C, 0x83, 0x38, 0x07,
    0xD9, 0x14, 0x24, 0xD9, 0x5C, 0x24, 0x04, 0x75, 0x0D, 0xD9, 0x40, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x04,
    0x24, 0x8B, 0x54, 0x24, 0x04, 0x89, 0x41, 0x64, 0x89, 0x51, 0x68, 0xB0,
    0x01, 0x83, 0xC4, 0x08, 0xC2, 0x08, 0x00,
];
const B70: [u8; 61] = [
    0x83, 0xEC, 0x08, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x0C, 0x83, 0x38, 0x07,
    0xD9, 0x14, 0x24, 0xD9, 0x5C, 0x24, 0x04, 0x75, 0x0D, 0xD9, 0x40, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x04,
    0x24, 0x8B, 0x54, 0x24, 0x04, 0x89, 0x81, 0x90, 0x00, 0x00, 0x00, 0x89,
    0x91, 0x94, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x83, 0xC4, 0x08, 0xC2, 0x08,
    0x00,
];
const B71: [u8; 68] = [
    0x83, 0xEC, 0x08, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x0C, 0x83, 0x38, 0x07,
    0xD9, 0x14, 0x24, 0xD9, 0x5C, 0x24, 0x04, 0x75, 0x0D, 0xD9, 0x40, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x04,
    0x24, 0x8B, 0x54, 0x24, 0x04, 0x80, 0x89, 0xF0, 0x00, 0x00, 0x00, 0x02,
    0x89, 0x81, 0xAC, 0x00, 0x00, 0x00, 0x89, 0x91, 0xB0, 0x00, 0x00, 0x00,
    0xB0, 0x01, 0x83, 0xC4, 0x08, 0xC2, 0x08, 0x00,
];
const B72: [u8; 61] = [
    0x83, 0xEC, 0x08, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x0C, 0x83, 0x38, 0x07,
    0xD9, 0x14, 0x24, 0xD9, 0x5C, 0x24, 0x04, 0x75, 0x0D, 0xD9, 0x40, 0x04,
    0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x04,
    0x24, 0x8B, 0x54, 0x24, 0x04, 0x89, 0x81, 0x98, 0x00, 0x00, 0x00, 0x89,
    0x91, 0x9C, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x83, 0xC4, 0x08, 0xC2, 0x08,
    0x00,
];
const B73: [u8; 74] = [
    0x8B, 0x44, 0x24, 0x04, 0x83, 0xEC, 0x0C, 0x83, 0x38, 0x06, 0x56, 0x75,
    0x0B, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x8B, 0x40, 0x0C, 0xEB, 0x1A,
    0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x54,
    0x24, 0x08, 0x8B, 0x74, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44,
    0x24, 0x0C, 0x89, 0x51, 0x7C, 0x89, 0xB1, 0x80, 0x00, 0x00, 0x00, 0x89,
    0x81, 0x84, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0x83, 0xC4, 0x0C, 0xC2,
    0x08, 0x00,
];
const B74: [u8; 77] = [
    0x8B, 0x44, 0x24, 0x04, 0x83, 0xEC, 0x0C, 0x83, 0x38, 0x06, 0x56, 0x75,
    0x0B, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x8B, 0x40, 0x0C, 0xEB, 0x1A,
    0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x54,
    0x24, 0x08, 0x8B, 0x74, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44,
    0x24, 0x0C, 0x89, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x89, 0xB1, 0xC8, 0x00,
    0x00, 0x00, 0x89, 0x81, 0xCC, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0x83,
    0xC4, 0x0C, 0xC2, 0x08, 0x00,
];
const B75: [u8; 77] = [
    0x8B, 0x44, 0x24, 0x04, 0x83, 0xEC, 0x0C, 0x83, 0x38, 0x06, 0x56, 0x75,
    0x0B, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x8B, 0x40, 0x0C, 0xEB, 0x1A,
    0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x54,
    0x24, 0x08, 0x8B, 0x74, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44,
    0x24, 0x0C, 0x89, 0x91, 0xD0, 0x00, 0x00, 0x00, 0x89, 0xB1, 0xD4, 0x00,
    0x00, 0x00, 0x89, 0x81, 0xD8, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0x83,
    0xC4, 0x0C, 0xC2, 0x08, 0x00,
];
const B76: [u8; 77] = [
    0x8B, 0x44, 0x24, 0x04, 0x83, 0xEC, 0x0C, 0x83, 0x38, 0x06, 0x56, 0x75,
    0x0B, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x8B, 0x40, 0x0C, 0xEB, 0x1A,
    0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x54,
    0x24, 0x08, 0x8B, 0x74, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44,
    0x24, 0x0C, 0x89, 0x91, 0xDC, 0x00, 0x00, 0x00, 0x89, 0xB1, 0xE0, 0x00,
    0x00, 0x00, 0x89, 0x81, 0xE4, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0x83,
    0xC4, 0x0C, 0xC2, 0x08, 0x00,
];
const B77: [u8; 77] = [
    0x8B, 0x44, 0x24, 0x04, 0x83, 0xEC, 0x0C, 0x83, 0x38, 0x06, 0x56, 0x75,
    0x0B, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x8B, 0x40, 0x0C, 0xEB, 0x1A,
    0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x54,
    0x24, 0x08, 0x8B, 0x74, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44,
    0x24, 0x0C, 0x89, 0x91, 0xE8, 0x00, 0x00, 0x00, 0x89, 0xB1, 0xEC, 0x00,
    0x00, 0x00, 0x89, 0x81, 0xF0, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0x83,
    0xC4, 0x0C, 0xC2, 0x08, 0x00,
];
const B78: [u8; 64] = [
    0x83, 0xEC, 0x08, 0xDB, 0x81, 0xA8, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24,
    0xDB, 0x81, 0xAC, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x0C, 0x8B, 0x01,
    0x85, 0xC0, 0xD9, 0x5C, 0x24, 0x04, 0x74, 0x05, 0x83, 0xF8, 0x07, 0x75,
    0x13, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x07, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x08, 0xB0, 0x01, 0x83, 0xC4,
    0x08, 0xC2, 0x08, 0x00,
];
const B79: [u8; 64] = [
    0x83, 0xEC, 0x08, 0xDB, 0x81, 0xB0, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24,
    0xDB, 0x81, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x0C, 0x8B, 0x01,
    0x85, 0xC0, 0xD9, 0x5C, 0x24, 0x04, 0x74, 0x05, 0x83, 0xF8, 0x07, 0x75,
    0x13, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x07, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x08, 0xB0, 0x01, 0x83, 0xC4,
    0x08, 0xC2, 0x08, 0x00,
];
const B80: [u8; 64] = [
    0x83, 0xEC, 0x08, 0xDB, 0x81, 0xA8, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24,
    0xDB, 0x81, 0xAC, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x0C, 0x8B, 0x01,
    0x85, 0xC0, 0xD9, 0x5C, 0x24, 0x04, 0x74, 0x05, 0x83, 0xF8, 0x07, 0x75,
    0x13, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x07, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x08, 0xB0, 0x01, 0x83, 0xC4,
    0x08, 0xC2, 0x08, 0x00,
];
const B81: [u8; 64] = [
    0x83, 0xEC, 0x08, 0xDB, 0x81, 0xB0, 0x00, 0x00, 0x00, 0xD9, 0x1C, 0x24,
    0xDB, 0x81, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x0C, 0x8B, 0x01,
    0x85, 0xC0, 0xD9, 0x5C, 0x24, 0x04, 0x74, 0x05, 0x83, 0xF8, 0x07, 0x75,
    0x13, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x07, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x08, 0xB0, 0x01, 0x83, 0xC4,
    0x08, 0xC2, 0x08, 0x00,
];
const B82: [u8; 112] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x25, 0x83,
    0xE8, 0x01, 0x74, 0x0E, 0x83, 0xE8, 0x01, 0x74, 0x04, 0xD9, 0xEE, 0xEB,
    0x1A, 0xD9, 0x42, 0x04, 0xEB, 0x15, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04,
    0x85, 0xC0, 0x7D, 0x0B, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xEB, 0x03,
    0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9,
    0x91, 0xEC, 0x00, 0x00, 0x00, 0xDC, 0x15, 0x20, 0xC1, 0xE5, 0x00, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x08, 0xDD, 0xD8, 0xD9, 0x05, 0x1C, 0x07,
    0xDD, 0x00, 0x80, 0x89, 0xF0, 0x00, 0x00, 0x00, 0x02, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xB0, 0x01, 0xD9, 0x99, 0xEC, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00,
];
const B83: [u8; 34] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x08, 0x85, 0xC9, 0x74, 0x05, 0x83, 0xF9,
    0x07, 0x75, 0x0E, 0xD9, 0xE8, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9,
    0x50, 0x04, 0xD9, 0x58, 0x08, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B84: [u8; 40] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x49, 0x59, 0x74,
    0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04, 0xB0, 0x01, 0xC2,
    0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88, 0x4A, 0x04, 0xB0,
    0x01, 0xC2, 0x08, 0x00,
];
const C84: [usize; 0] = [];
const B85: [u8; 40] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x49, 0x58, 0x74,
    0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04, 0xB0, 0x01, 0xC2,
    0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88, 0x4A, 0x04, 0xB0,
    0x01, 0xC2, 0x08, 0x00,
];
const C85: [usize; 0] = [];
const B86: [u8; 55] = [
    0x51, 0xD9, 0x81, 0x88, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C86: [usize; 0] = [];
const B87: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x88,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x88, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x88, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x88, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C87: [usize; 0] = [];
const B88: [u8; 52] = [
    0x51, 0xD9, 0x41, 0x78, 0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x01, 0xD9, 0x1C,
    0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75, 0x18, 0xD9, 0x04,
    0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08, 0x00, 0xD9, 0x04,
    0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59, 0x04, 0xB0, 0x01,
    0x59, 0xC2, 0x08, 0x00,
];
const C88: [usize; 0] = [];
const B89: [u8; 109] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x4F, 0x83,
    0xE8, 0x01, 0x74, 0x2A, 0x83, 0xE8, 0x01, 0x74, 0x12, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x78,
    0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x78, 0xC2, 0x08, 0x00, 0x8B, 0x42,
    0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0, 0x7D, 0x19, 0xD8, 0x05, 0xD8, 0xBF,
    0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04,
    0xD9, 0x59, 0x78, 0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24,
    0x04, 0xB0, 0x01, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x78, 0xC2, 0x08,
    0x00,
];
const C89: [usize; 0] = [];
const B90: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x5C, 0xD8, 0x44, 0x24, 0x04,
    0xD9, 0x59, 0x54, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x41, 0x5C, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x59,
    0x54, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x41, 0x5C, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x54,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x41, 0x5C, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x59, 0x54, 0xC2, 0x08,
    0x00,
];
const C90: [usize; 0] = [];
const B91: [u8; 55] = [
    0x51, 0xD9, 0x41, 0x5C, 0xD8, 0x41, 0x54, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0x85, 0xC0, 0xD9, 0x1C, 0x24, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C91: [usize; 0] = [];
const B92: [u8; 40] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x49, 0x58, 0x74,
    0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04, 0xB0, 0x01, 0xC2,
    0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88, 0x4A, 0x04, 0xB0,
    0x01, 0xC2, 0x08, 0x00,
];
const C92: [usize; 0] = [];
const B93: [u8; 52] = [
    0x51, 0xD9, 0x41, 0x7C, 0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x01, 0xD9, 0x1C,
    0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75, 0x18, 0xD9, 0x04,
    0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08, 0x00, 0xD9, 0x04,
    0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59, 0x04, 0xB0, 0x01,
    0x59, 0xC2, 0x08, 0x00,
];
const C93: [usize; 0] = [];
const B94: [u8; 52] = [
    0x51, 0xD9, 0x41, 0x74, 0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x01, 0xD9, 0x1C,
    0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75, 0x18, 0xD9, 0x04,
    0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08, 0x00, 0xD9, 0x04,
    0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59, 0x04, 0xB0, 0x01,
    0x59, 0xC2, 0x08, 0x00,
];
const C94: [usize; 0] = [];
const B95: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xA0, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C95: [usize; 0] = [];
const B96: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xA0,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xA0, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xA0, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xA0, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C96: [usize; 0] = [];
const B97: [u8; 43] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x89, 0xC0, 0x00,
    0x00, 0x00, 0x74, 0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04,
    0xB0, 0x01, 0xC2, 0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88,
    0x4A, 0x04, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const C97: [usize; 0] = [];
const B98: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xF4, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C98: [usize; 0] = [];
const B99: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF4,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF4, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF4, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF4, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C99: [usize; 0] = [];
const B100: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xF8, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C100: [usize; 0] = [];
const B101: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF8,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF8, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF8, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xF8, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C101: [usize; 0] = [];
const B102: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xFC, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C102: [usize; 0] = [];
const B103: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xFC,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xFC, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xFC, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xFC, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C103: [usize; 0] = [];
const B104: [u8; 55] = [
    0x51, 0xD9, 0x81, 0x00, 0x01, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C104: [usize; 0] = [];
const B105: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x00,
    0x01, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x00, 0x01, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x00, 0x01, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x00, 0x01, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C105: [usize; 0] = [];
const B106: [u8; 45] = [
    0x8A, 0x81, 0xF0, 0x00, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x04, 0x8B, 0x0A,
    0x24, 0x01, 0x85, 0xC9, 0x74, 0x0D, 0x83, 0xF9, 0x05, 0x75, 0x11, 0x88,
    0x42, 0x04, 0xB0, 0x01, 0xC2, 0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00,
    0x00, 0x88, 0x42, 0x04, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const C106: [usize; 0] = [];
const B107: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xEC, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C107: [usize; 0] = [];
const B108: [u8; 43] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x89, 0xC4, 0x00,
    0x00, 0x00, 0x74, 0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04,
    0xB0, 0x01, 0xC2, 0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88,
    0x4A, 0x04, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const C108: [usize; 0] = [];
const B109: [u8; 43] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x85, 0xC0, 0x8A, 0x89, 0xA0, 0x00,
    0x00, 0x00, 0x74, 0x0D, 0x83, 0xF8, 0x05, 0x75, 0x11, 0x88, 0x4A, 0x04,
    0xB0, 0x01, 0xC2, 0x08, 0x00, 0xC7, 0x02, 0x05, 0x00, 0x00, 0x00, 0x88,
    0x4A, 0x04, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const C109: [usize; 0] = [];
const B110: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C110: [usize; 0] = [];
const B111: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0xB0,
    0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC0,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0xB0, 0x01, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC0, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xB0, 0x01, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC0, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xB0, 0x01,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC0, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C111: [usize; 0] = [];
const B112: [u8; 55] = [
    0x51, 0xD9, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C112: [usize; 0] = [];
const B113: [u8; 121] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x58, 0x83,
    0xE8, 0x01, 0x74, 0x30, 0x83, 0xE8, 0x01, 0x74, 0x15, 0xD9, 0xEE, 0x32,
    0xC0, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC4,
    0x00, 0x00, 0x00, 0xC2, 0x08, 0x00, 0xD9, 0x42, 0x04, 0x32, 0xC0, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC4, 0x00, 0x00,
    0x00, 0xC2, 0x08, 0x00, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0,
    0x7D, 0x1C, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0x32, 0xC0, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC4, 0x00, 0x00, 0x00,
    0xC2, 0x08, 0x00, 0xDB, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0x32, 0xC0,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0xC4, 0x00, 0x00, 0x00, 0xC2, 0x08,
    0x00,
];
const C113: [usize; 0] = [];
const B114: [u8; 55] = [
    0x51, 0xD9, 0x81, 0x98, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x8B,
    0x01, 0xD9, 0x1C, 0x24, 0x85, 0xC0, 0x74, 0x11, 0x83, 0xF8, 0x03, 0x75,
    0x18, 0xD9, 0x04, 0x24, 0xB0, 0x01, 0xD9, 0x59, 0x04, 0x59, 0xC2, 0x08,
    0x00, 0xD9, 0x04, 0x24, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x59,
    0x04, 0xB0, 0x01, 0x59, 0xC2, 0x08, 0x00,
];
const C114: [usize; 0] = [];
const B115: [u8; 86] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x02, 0x83, 0xE8, 0x01, 0x74, 0x21, 0x83,
    0xE8, 0x01, 0x74, 0x0A, 0x83, 0xE8, 0x01, 0x75, 0x31, 0xD9, 0x42, 0x04,
    0xEB, 0x15, 0x8B, 0x42, 0x04, 0xDB, 0x42, 0x04, 0x85, 0xC0, 0x7D, 0x0B,
    0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xEB, 0x03, 0xDB, 0x42, 0x04, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xD1, 0xDF,
    0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0x7A, 0x07, 0xDD, 0xD8, 0x32, 0xC0,
    0xC2, 0x08, 0x00, 0xD9, 0x99, 0x98, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xC2,
    0x08, 0x00,
];
const C115: [usize; 0] = [];
const B116: [u8; 30] = [
    0x8B, 0x08, 0x85, 0xC9, 0x74, 0x08, 0x83, 0xF9, 0x03, 0x74, 0x09, 0xC2,
    0x04, 0x00, 0xC7, 0x00, 0x03, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x04,
    0xD9, 0x58, 0x04, 0xC2, 0x04, 0x00,
];
const C116: [usize; 0] = [];
const B117: [u8; 50] = [
    0x51, 0x8B, 0x01, 0x83, 0xE8, 0x01, 0x74, 0x25, 0x83, 0xE8, 0x01, 0x74,
    0x0E, 0x83, 0xE8, 0x01, 0x74, 0x04, 0xD9, 0xEE, 0x59, 0xC3, 0xD9, 0x41,
    0x04, 0x59, 0xC3, 0x8B, 0x41, 0x04, 0xDB, 0x41, 0x04, 0x85, 0xC0, 0x7D,
    0x0B, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0x59, 0xC3, 0xDB, 0x41, 0x04,
    0x59, 0xC3,
];
const C117: [usize; 0] = [];
const B118: [u8; 50] = [
    0x51, 0x8B, 0x01, 0x83, 0xE8, 0x01, 0x74, 0x25, 0x83, 0xE8, 0x01, 0x74,
    0x0E, 0x83, 0xE8, 0x01, 0x74, 0x04, 0xD9, 0xEE, 0x59, 0xC3, 0xD9, 0x41,
    0x04, 0x59, 0xC3, 0x8B, 0x41, 0x04, 0xDB, 0x41, 0x04, 0x85, 0xC0, 0x7D,
    0x0B, 0xDC, 0x05, 0xE0, 0xC0, 0xE5, 0x00, 0x59, 0xC3, 0xDB, 0x41, 0x04,
    0x59, 0xC3,
];
const C118: [usize; 0] = [];
const B119: [u8; 39] = [
    0x51, 0x8B, 0x08, 0x85, 0xC9, 0x74, 0x07, 0x83, 0xF9, 0x07, 0x74, 0x08,
    0x59, 0xC3, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xD9, 0x42, 0x04, 0xD9,
    0x1C, 0x24, 0xD9, 0x02, 0xD9, 0x58, 0x04, 0xD9, 0x04, 0x24, 0xD9, 0x58,
    0x08, 0x59, 0xC3,
];
const C119: [usize; 0] = [];
const B120: [u8; 65] = [
    0x83, 0xEC, 0x0C, 0x83, 0x39, 0x06, 0x75, 0x15, 0x8B, 0x51, 0x04, 0x89,
    0x10, 0x8B, 0x51, 0x08, 0x8B, 0x49, 0x0C, 0x89, 0x50, 0x04, 0x89, 0x48,
    0x08, 0x83, 0xC4, 0x0C, 0xC3, 0xD9, 0xEE, 0xD9, 0x14, 0x24, 0x8B, 0x14,
    0x24, 0xD9, 0x54, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x04, 0xD9, 0x5C, 0x24,
    0x08, 0x89, 0x10, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x48, 0x04, 0x89, 0x50,
    0x08, 0x83, 0xC4, 0x0C, 0xC3,
];
const C120: [usize; 0] = [];
const B121: [u8; 112] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08,
    0xD9, 0x54, 0x24, 0x04, 0x8B, 0x44, 0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8,
    0x01, 0x75, 0x44, 0xD9, 0x54, 0x24, 0x04, 0x83, 0x7C, 0x24, 0x04, 0x00,
    0x74, 0x39, 0xDC, 0x25, 0x40, 0xBF, 0xE5, 0x00, 0xDC, 0x0D, 0xB8, 0xC1,
    0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00,
    0xD9, 0x44, 0x24, 0x04, 0xE8, 0xEB, 0x27, 0x10, 0x00, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x4D, 0x10, 0xD8, 0x45, 0x0C, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D, 0xC3, 0xDD,
    0xD8, 0xD9, 0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04,
    0x8B, 0xE5, 0x5D, 0xC3,
];
const C121: [usize; 1] = [0x40];
const B122: [u8; 127] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08,
    0xD9, 0xC0, 0xD9, 0xE8, 0xD9, 0xC0, 0xDE, 0xE2, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x04, 0x8B, 0x44, 0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x75,
    0x44, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x83, 0x7C, 0x24, 0x04, 0x00,
    0x74, 0x39, 0xDC, 0x0D, 0xC0, 0xC9, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x04,
    0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xE8, 0x75,
    0x27, 0x10, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9,
    0xE8, 0xDE, 0xE1, 0xD8, 0x4D, 0x10, 0xD8, 0x45, 0x0C, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D, 0xC3, 0xDD, 0xD9, 0xDD,
    0xD8, 0xD9, 0x45, 0x0C, 0xD8, 0x45, 0x10, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D, 0xC3,
];
const C122: [usize; 1] = [0x46];
const B123: [u8; 218] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08,
    0xD9, 0x54, 0x24, 0x04, 0x83, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x09, 0xDD,
    0xD8, 0xD9, 0x45, 0x0C, 0x8B, 0xE5, 0x5D, 0xC3, 0xD9, 0xC0, 0xD9, 0xE8,
    0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x04, 0x83, 0x7C, 0x24, 0x04,
    0x00, 0x75, 0x16, 0xDD, 0xD9, 0xDD, 0xD8, 0xD9, 0x45, 0x0C, 0xD8, 0x45,
    0x10, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D,
    0xC3, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xDC, 0xCA, 0xD9, 0xCA, 0xDE,
    0xE1, 0xD9, 0x5D, 0x08, 0xD9, 0x45, 0x08, 0xD9, 0x54, 0x24, 0x04, 0x8B,
    0x44, 0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74, 0x35, 0xDC, 0x0D,
    0xB8, 0xC1, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04,
    0xE8, 0xC3, 0x26, 0x10, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24,
    0x04, 0xD9, 0x45, 0x10, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9,
    0xD8, 0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B,
    0xE5, 0x5D, 0xC3, 0xDC, 0x0D, 0xC0, 0xC9, 0xE5, 0x00, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xE8, 0x8E, 0x26, 0x10, 0x00, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDC, 0x2D, 0xE8, 0xBF, 0xE5, 0x00,
    0xD9, 0x45, 0x10, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xD8,
    0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5,
    0x5D, 0xC3,
];
const C123: [usize; 2] = [0x78, 0xad];
const B124: [u8; 107] = [
    0xD9, 0x44, 0x24, 0x04, 0xDC, 0xC0, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0xD9, 0xC0, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x54,
    0x24, 0x04, 0x8B, 0x44, 0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74,
    0x1F, 0xDD, 0xD8, 0xDD, 0xD8, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70,
    0xBF, 0xE5, 0x00, 0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xD9, 0x44, 0x24, 0x08,
    0xD9, 0xCB, 0xDC, 0x25, 0xF8, 0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xDE, 0xE1,
    0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9,
    0xDE, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C124: [usize; 0] = [];
const B125: [u8; 119] = [
    0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05,
    0x40, 0xBF, 0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x44,
    0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74, 0x1F, 0xDD, 0xD9, 0xD9,
    0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xD8, 0xC9, 0xD8,
    0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x04, 0xC3, 0xD8, 0xE1, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0xD9, 0xC0, 0xDC, 0xC8, 0xDE, 0xC9, 0xDE, 0xC1, 0xD9, 0x44,
    0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xD8, 0x44,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C125: [usize; 0] = [];
const B126: [u8; 127] = [
    0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05,
    0x40, 0xBF, 0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x44,
    0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74, 0x21, 0xDD, 0xD9, 0xD9,
    0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xD8, 0xC9, 0xD8,
    0xC9, 0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xD8, 0xE1, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD9, 0xC0, 0xDE, 0xC9, 0xD9, 0xC1,
    0xDE, 0xC9, 0xDE, 0xC9, 0xDE, 0xE1, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D,
    0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xD8, 0x6C, 0x24, 0x08, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C126: [usize; 0] = [];
const B127: [u8; 127] = [
    0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05,
    0x40, 0xBF, 0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x44,
    0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74, 0x23, 0xDD, 0xD9, 0xD9,
    0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xD8, 0xC9, 0xD8,
    0xC9, 0xD8, 0xC9, 0xD8, 0xC9, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xD8, 0xE1, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDC, 0xC8, 0xD8, 0xC9,
    0xD8, 0xC9, 0xDE, 0xC9, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D,
    0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C127: [usize; 0] = [];
const B128: [u8; 62] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x14, 0x51, 0xDD, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x10, 0xDC, 0x0D, 0x48, 0xC4, 0xE5, 0x00, 0xD9, 0x5C,
    0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0x1C, 0x24, 0xE8, 0x4A, 0xD5,
    0x86, 0xFF, 0xDD, 0x44, 0x24, 0x04, 0xDC, 0xC9, 0xDE, 0xE1, 0xD8, 0x44,
    0x24, 0x14, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0x83, 0xC4,
    0x0C, 0xC3,
];
const C128: [usize; 1] = [0x21];
const B129: [u8; 47] = [
    0xD9, 0x44, 0x24, 0x04, 0x51, 0xDC, 0x0D, 0x48, 0xC4, 0xE5, 0x00, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8, 0xC5,
    0xA9, 0x88, 0xFF, 0xD8, 0x4C, 0x24, 0x10, 0x83, 0xC4, 0x04, 0xD8, 0x44,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C129: [usize; 1] = [0x16];
const B130: [u8; 61] = [
    0xD9, 0x44, 0x24, 0x04, 0x51, 0xDC, 0x0D, 0x60, 0xC1, 0xE5, 0x00, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8, 0xE5,
    0xD4, 0x86, 0xFF, 0xDC, 0x25, 0x40, 0xBF, 0xE5, 0x00, 0x83, 0xC4, 0x04,
    0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xDE, 0xC9,
    0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04,
    0xC3,
];
const C130: [usize; 1] = [0x16];
const B131: [u8; 64] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x0C, 0x51, 0xD9, 0x44, 0x24, 0x14,
    0xDD, 0x5C, 0x24, 0x04, 0xDC, 0xC8, 0xD9, 0xE8, 0xDE, 0xE1, 0xD9, 0x5C,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x1C, 0x24, 0xE8, 0x9A, 0x06,
    0x87, 0xFF, 0xDC, 0x25, 0x40, 0xBF, 0xE5, 0x00, 0xD8, 0x4C, 0x24, 0x18,
    0xDC, 0x6C, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10,
    0x83, 0xC4, 0x0C, 0xC3,
];
const C131: [usize; 1] = [0x21];
const B132: [u8; 59] = [
    0xD9, 0x44, 0x24, 0x04, 0x51, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xDC, 0xC8, 0xDE, 0xE9, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8, 0x59,
    0x06, 0x87, 0xFF, 0xD8, 0x4C, 0x24, 0x10, 0x83, 0xC4, 0x04, 0xD8, 0x44,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C132: [usize; 1] = [0x22];
const B133: [u8; 186] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x0C, 0x51, 0xDD, 0x05, 0xE8, 0xBF,
    0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0xC0, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x10, 0x8B, 0x44, 0x24, 0x10, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74,
    0x42, 0xDD, 0xDA, 0xD9, 0x44, 0x24, 0x14, 0xDD, 0x5C, 0x24, 0x04, 0xDC,
    0xC8, 0xDE, 0xE9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9,
    0x1C, 0x24, 0xE8, 0xF1, 0x05, 0x87, 0xFF, 0xDC, 0x25, 0x40, 0xBF, 0xE5,
    0x00, 0x83, 0xC4, 0x04, 0xD9, 0x44, 0x24, 0x14, 0xDC, 0x0D, 0x70, 0xBF,
    0xE5, 0x00, 0xDE, 0xC9, 0xDC, 0x2C, 0x24, 0xD9, 0x5C, 0x24, 0x0C, 0xD9,
    0x44, 0x24, 0x0C, 0x83, 0xC4, 0x08, 0xC3, 0xD9, 0xC9, 0xDE, 0xC2, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDC, 0xC8, 0xDE,
    0xE9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x1C, 0x24,
    0xE8, 0xAB, 0x05, 0x87, 0xFF, 0xDC, 0x05, 0x40, 0xBF, 0xE5, 0x00, 0x83,
    0xC4, 0x04, 0xD9, 0x44, 0x24, 0x14, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00,
    0xDE, 0xC9, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44,
    0x24, 0x0C, 0x83, 0xC4, 0x08, 0xC3,
];
const C133: [usize; 2] = [0x4a, 0x90];
const B134: [u8; 68] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x14, 0x83, 0xEC, 0x0C, 0xDD, 0x54,
    0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0xEE, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x18, 0xD9, 0xE8, 0xDE, 0xE1, 0xD9, 0x5C, 0x24, 0x20,
    0xD9, 0x44, 0x24, 0x20, 0xD9, 0x1C, 0x24, 0xE8, 0x20, 0x00, 0x00, 0x00,
    0xDC, 0x6C, 0x24, 0x0C, 0xD8, 0x44, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x20,
    0xD9, 0x44, 0x24, 0x20, 0x83, 0xC4, 0x14, 0xC3,
];
const C134: [usize; 1] = [0x2b];
const B135: [u8; 249] = [
    0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05, 0xB8, 0xC9, 0xE5, 0x00, 0xD8, 0xE9,
    0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x44, 0x24, 0x04, 0xC1, 0xE8, 0x1F, 0xA8,
    0x01, 0x74, 0x1B, 0xDD, 0x05, 0xB0, 0xC9, 0xE5, 0x00, 0xD8, 0xC9, 0xDE,
    0xC9, 0xD8, 0x4C, 0x24, 0x0C, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xDD, 0x05, 0xA8, 0xC9, 0xE5, 0x00,
    0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x04, 0xC1, 0xE9,
    0x1F, 0xF6, 0xC1, 0x01, 0x74, 0x31, 0xDC, 0x25, 0xA0, 0xC9, 0xE5, 0x00,
    0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0xB0, 0xC9,
    0xE5, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xDE, 0xC9,
    0xDC, 0x05, 0x70, 0xC3, 0xE5, 0x00, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xDD, 0x05, 0x98, 0xC9, 0xE5,
    0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xC1,
    0xEA, 0x1F, 0xF6, 0xC2, 0x01, 0x74, 0x31, 0xDC, 0x25, 0x90, 0xC9, 0xE5,
    0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0xB0,
    0xC9, 0xE5, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xDE,
    0xC9, 0xDC, 0x05, 0x88, 0xC9, 0xE5, 0x00, 0xD8, 0x44, 0x24, 0x08, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3, 0xDC, 0x25, 0x80, 0xC9,
    0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D,
    0xB0, 0xC9, 0xE5, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA,
    0xDE, 0xC9, 0xDC, 0x05, 0x78, 0xC9, 0xE5, 0x00, 0xD8, 0x44, 0x24, 0x08,
    0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC3,
];
const C135: [usize; 0] = [];
const B136: [u8; 191] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x0C, 0x83, 0xEC, 0x0C, 0xDD, 0x05,
    0x70, 0xBF, 0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x18, 0x8B, 0x44,
    0x24, 0x18, 0xD9, 0x44, 0x24, 0x20, 0xC1, 0xE8, 0x1F, 0xA8, 0x01, 0x74,
    0x57, 0xDD, 0x54, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0xEE, 0xD9,
    0x5C, 0x24, 0x04, 0xDC, 0xC0, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24,
    0x18, 0xD9, 0xE8, 0xDE, 0xE1, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24,
    0x18, 0xD9, 0x1C, 0x24, 0xE8, 0xAF, 0xFE, 0xFF, 0xFF, 0xDC, 0x6C, 0x24,
    0x0C, 0x83, 0xC4, 0x0C, 0xDC, 0x05, 0xA0, 0xBF, 0xE5, 0x00, 0xD9, 0x5C,
    0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00,
    0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C,
    0x83, 0xC4, 0x08, 0xC3, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0xEE, 0xD9, 0x5C,
    0x24, 0x04, 0xDC, 0xC0, 0xDC, 0x25, 0x40, 0xBF, 0xE5, 0x00, 0xD9, 0x5C,
    0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0x1C, 0x24, 0xE8, 0x62, 0xFE,
    0xFF, 0xFF, 0xDD, 0x05, 0x70, 0xBF, 0xE5, 0x00, 0x83, 0xC4, 0x0C, 0xDC,
    0xC9, 0xD8, 0x4C, 0x24, 0x14, 0xDE, 0xC1, 0xD8, 0x44, 0x24, 0x10, 0xD9,
    0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0x83, 0xC4, 0x08, 0xC3,
];
const C136: [usize; 2] = [0x4c, 0x99];
