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
        calls: AtomicU32::new(0),
        mismatches: AtomicU32::new(0),
        original: AtomicUsize::new(0),
    }
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
    let regs_ok = ours.ecx == theirs.ecx
        && ours.edx == theirs.edx
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

// --- the table ------------------------------------------------------------------------------------

const COUNT: usize = 84;

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
];

// --- install ------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for port in PORTS.iter() {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, port.body, &[]) } {
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
