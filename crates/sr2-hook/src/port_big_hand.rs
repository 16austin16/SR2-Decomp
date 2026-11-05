//! Hand-written camera ports (replacing the machine translations of `port_big.rs`).
//!
//! Every function here has the same shape as the translated ones: `body(ent, args)` where `ent` carries the entry
//! registers (and takes the exit registers) and `args` points at the stack arguments, so the same entry thunk
//! (`ent_thunk!`) and the same whole-function differential test (port_big_difftest.rs, which runs this body against the
//! original bytes) apply. Callees stay the original functions and are called by address through `hcall`.
//!
//! The registers a function leaves behind are only reproduced where a caller can read them: each port states the
//! live exit registers (`FnSpec::live` in the difftest) after reading the callers. Registers handed to a callee are
//! reproduced where the callee reads them (declared per call as an input mask, checked by the difftest against the
//! original at every call; the mask of outputs says which registers the port goes on to use).

#![allow(unsafe_op_in_unsafe_fn, dead_code, clippy::all)]

use super::*;

pub(super) const EAX: u8 = 0x01;
pub(super) const ECX: u8 = 0x02;
pub(super) const EDX: u8 = 0x04;
pub(super) const EBX: u8 = 0x08;
pub(super) const ESI: u8 = 0x10;
pub(super) const EDI: u8 = 0x20;
pub(super) const XMM0: u8 = 0x40;
pub(super) const NONE: u8 = 0;
/// output mask only: just the low byte of eax is used
pub(super) const AL: u8 = 0x80;

/// The general registers and xmm0 as the next callee will see them.
#[derive(Clone, Copy, Default)]
pub(super) struct Rg {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esi: u32,
    pub edi: u32,
    pub xmm0: u32,
}

impl Rg {
    pub fn entry(e: &Ent) -> Rg {
        Rg { eax: e.eax, ecx: e.ecx, edx: e.edx, ebx: e.ebx, esi: e.esi, edi: e.edi, xmm0: e.xmm0 }
    }
}

/// What a callee left on the x87 stack (`st[0]` = ST0), popped by `hcall`.
pub(super) struct Ret {
    pub n: usize,
    pub st: [f64; 4],
}

/// Call the game function `f` with the registers in `r` and `args` pushed (args[0] at the lowest address, i.e. the
/// last push); `r` takes the callee's exit registers. `pop` is the number of stack dwords the callee itself pops
/// (stdcall), `st_in` the values passed on the x87 stack (ST0 first), `live_st` the values the original keeps on its
/// x87 stack across the call (ST0 first). `inm` / `outm`: registers the callee reads / the port reads afterwards.
pub(super) unsafe fn hcall(r: &mut Rg, f: usize, inm: u8, outm: u8, args: &[u32], pop: u32, st_in: &[f64], live_st: &[f64]) -> Ret {
    let mut b = Blk {
        f: f as u32,
        eax: r.eax,
        ecx: r.ecx,
        edx: r.edx,
        ebx: r.ebx,
        esi: r.esi,
        edi: r.edi,
        args: args.as_ptr() as u32,
        nargs: args.len() as u32,
        xmm0: r.xmm0,
        sw_before: 0,
        sw_after: 0,
        has_in: st_in.len() as u32,
        pad: 0,
        st_in: [st_in.first().copied().unwrap_or(0.0), st_in.get(1).copied().unwrap_or(0.0)],
        st_out: [0.0; 4],
    };
    #[cfg(test)]
    {
        difftest::H_IN = inm;
        difftest::H_OUT = outm;
        let mut fx = X87::new();
        for v in live_st.iter().rev() {
            fx.push(*v);
        }
        difftest::pre_call(&b, &fx, pop);
        if std::env::var("DT_PCTRACE").is_ok() {
            eprintln!("    hcall {:#x} eax {:#x} ecx {:#x} args {:x?}", f, r.eax, r.ecx, args);
        }
    }
    let _ = (inm, outm, live_st, pop);
    let n = callx_raw(&mut b);
    r.eax = b.eax;
    r.ecx = b.ecx;
    r.edx = b.edx;
    r.ebx = b.ebx;
    r.esi = b.esi;
    r.edi = b.edi;
    r.xmm0 = b.xmm0;
    Ret { n, st: b.st_out }
}

/// Call with no x87 inputs and nothing live on the x87 stack.
#[inline(always)]
pub(super) unsafe fn call(r: &mut Rg, f: usize, inm: u8, outm: u8, args: &[u32], pop: u32) -> Ret {
    hcall(r, f, inm, outm, args, pop, &[], &[])
}

/// The function's local area: `size_of::<T>()` bytes ending `above` bytes below the entry esp. Under the difftest it
/// is the emulated stack frame the original uses (same addresses, same stack garbage for reads of uninitialised
/// locals); in the game it is a local buffer.
#[inline(always)]
pub(super) unsafe fn local_frame<T>(_buf: &mut core::mem::MaybeUninit<T>, _above: usize) -> *mut T {
    #[cfg(test)]
    {
        return (difftest::ESP0 as usize - _above - core::mem::size_of::<T>()) as *mut T;
    }
    #[cfg(not(test))]
    {
        _buf.as_mut_ptr()
    }
}

/// `a == b`, telling the difftest which value the comparison was against (it steers later scenarios to the equal case).
#[inline(always)]
pub(super) fn eq(a: u32, b: u32) -> bool {
    #[cfg(test)]
    difftest::cmp_hint(a, b, 32);
    a == b
}

/// Difftest coverage marker (the original address of the block this code corresponds to).
#[inline(always)]
pub(super) fn cov(_pc: usize) {
    #[cfg(test)]
    difftest::cov(_pc);
}

/// `fld dword` + `fstp dword` of raw memory: a signalling NaN comes back quiet.
#[inline(always)]
pub(super) fn q(v: u32) -> u32 {
    if v & 0x7F80_0000 == 0x7F80_0000 && v & 0x007F_FFFF != 0 { v | 0x0040_0000 } else { v }
}

/// x87 `fsub` of two f32 values (loaded as doubles), stored back as an f32: `(a - b) as f32`.
#[inline(always)]
pub(super) fn sub32(a: u32, b: u32) -> u32 {
    (((f32::from_bits(a) as f64) - (f32::from_bits(b) as f64)) as f32).to_bits()
}

// --- camera_cut (0x492DF0..0x492FD3, 483 bytes) -----------------------------------------------------
//
// Camera teleport: re-commits the follow context, recomputes the follow camera's desired position, and puts the
// camera there with no smoothing (copies the target orientation over the three camera matrices).
// No arguments, plain `ret`, game thread. Exit registers: ebx / esi / edi / ebp unchanged; eax / ecx / edx are
// scratch for every caller.
//
// Quirk kept: when the target entity handle does not resolve (but the position is valid) the original copies the
// orientation from address 0x20 (null entity + 0x20), which faults; so does this port.

const CAM_FROZEN: usize = 0x25f5ae8;
const CAM_MODE: usize = 0x25f5c54;
const CAM_TARGET_HANDLE: usize = 0x25f5bb0;
const CAM_DESIRED: usize = 0xe9a664;
const CAM_POS: usize = 0x25f5b14;
const CAM_POS_PREV: usize = 0x25f5b20;
const CAM_POS_COPY: usize = 0x25f5b2c;
const MAX_CUT_DIST: usize = 0x2526be8;

pub(super) unsafe fn camera_cut_body(ent: &mut Ent, _args: *const u32) {
    cov(0x492df0);
    if !eq(rd(CAM_FROZEN), 0) {
        return;
    }
    cov(0x492e02);
    let mut r = Rg::entry(ent);
    wb(0x2526b5a, 1);
    // camera_follow_commit_context, then camera_follow_compute_desired_position (no inputs)
    call(&mut r, 0x49a670, NONE, NONE, &[], 0);
    if eq(rd(CAM_FROZEN), 0) && eq(rd(CAM_MODE), 1) {
        cov(0x492e20);
        wd(CAM_MODE, 0);
    }
    call(&mut r, 0x499b50, NONE, NONE, &[], 0);

    let mut want = [rd(CAM_DESIRED), rd(CAM_DESIRED + 4), rd(CAM_DESIRED + 8)];
    // 0x49d140 writes the follow target point through esi
    let mut target = [0u32; 3];
    r.esi = target.as_mut_ptr() as u32;
    call(&mut r, 0x49d140, ESI, NONE, &[], 0);
    let mut cur = [q(rd(CAM_POS)), q(rd(CAM_POS + 4)), q(rd(CAM_POS + 8))];
    // 0x4938c0(eax = &want, edi = &target, &out, callback 0x4941F0) -> eax = the vector to cut to
    let mut out = [0u32; 3];
    r.eax = want.as_mut_ptr() as u32;
    r.edi = r.esi;
    call(&mut r, 0x4938c0, EAX | EDI, EAX, &[out.as_mut_ptr() as u32, 0x4941f0], 0);
    std::hint::black_box((&mut want, &mut target, &mut out));
    let p = r.eax as usize;
    let v = [rd(p), rd(p + 4), rd(p + 8)];
    for k in 0..3 {
        wd(CAM_POS_PREV + 4 * k, v[k]);
        wd(CAM_POS + 4 * k, v[k]);
        wd(CAM_POS_COPY + 4 * k, v[k]);
    }
    // distance from the cut point to the follow target point; distance the camera jumped
    let d = call(&mut r, 0xbde980, NONE, NONE, &[CAM_POS_PREV as u32, target.as_ptr() as u32], 0);
    wf(0x25f5d04, d.st[0] as f32);
    wf(0x25f5c78, 0.0);
    let d = call(&mut r, 0xbde980, NONE, NONE, &[cur.as_ptr() as u32, CAM_POS as u32], 0);
    let jump = d.st[0] as f32;
    if (rf(MAX_CUT_DIST) as f64) < jump as f64 {
        cov(0x492f13);
        wf(MAX_CUT_DIST, jump);
    } else {
        cov(0x492f1b);
    }
    // direction from the new camera position back to the desired position
    for k in 0..3 {
        cur[k] = sub32(q(want[k]), rd(CAM_POS_PREV + 4 * k));
    }
    r.ecx = cur.as_ptr() as u32;
    call(&mut r, 0xbdda80, ECX, AL, &[], 0);
    let mut mat = [0u32; 9];
    if r.eax as u8 != 0 {
        cov(0x492f54);
        // orientation of the follow target entity (handle -> slot table -> entity with matching id at +0x44)
        let h = rd(CAM_TARGET_HANDLE);
        let mut e = 0usize;
        if h != 0 {
            let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
            if slot != 0 {
                cov(0x492f5e);
                if eq(rd(slot + 0x44), h) {
                    cov(0x492f72);
                    e = slot;
                }
            }
        }
        for k in 0..9 {
            mat[k] = rd(e + 0x20 + 4 * k);
        }
    } else {
        cov(0x492f89);
        call(&mut r, 0xbe4c80, NONE, NONE, &[mat.as_mut_ptr() as u32, cur.as_ptr() as u32, 0xec5acc], 3);
    }
    cov(0x492f9d);
    for dst in [0x25f5b5cusize, 0x25f5b38, 0x25f5b80] {
        for k in 0..9 {
            wd(dst + 4 * k, mat[k]);
        }
    }
}

// --- camera_follow_context_update (0x49AE60..0x49AFEE, 398 bytes) ------------------------------------
//
// Per-frame upkeep of the follow camera's context: while the "settle stamp" 0xE9A68C is stale it asks the target
// what kind of thing it is (0x49AAB0), and when that kind (or the target) changed it runs the context-switch
// code (0x49ACD0 sets the camera flags 0xE9A684 for the kind, 0x49A6E0 starts the blend with the new kind's
// blend time), then clears an expired stamp and runs the follow-context step (0x49A4C0) with dt.
// stdcall (f32 dt) `ret 4`, game thread. Exit registers: ebx / esi / edi / ebp unchanged; eax / ecx / edx are
// scratch for the callers.
//
// The stamp tests are signed 32-bit tick differences against the clock 0xE843AC with a window of 900,000,000.

const STAMP: usize = 0xe9a68c;
const CLOCK: usize = 0xe843ac;
const STAMP_WINDOW: i32 = 0x35a4_e900;

pub(super) unsafe fn camera_follow_context_update_body(ent: &mut Ent, args: *const u32) {
    cov(0x49ae60);
    let dt = q(rd(args as usize));
    let mut r = Rg::entry(ent);
    let now = rd(CLOCK) as i32;
    let handle = rd(CAM_TARGET_HANDLE);
    r.esi = now as u32;
    r.edi = handle;

    // the first test: a stamp that is >= 0 and "recent" skips the whole refresh
    let t = rd(STAMP) as i32;
    let skip = t >= 0 && if t > now { t.wrapping_sub(now) < STAMP_WINDOW } else { now.wrapping_sub(t) >= STAMP_WINDOW };
    if !skip {
        cov(0x49ae9e);
        r.ecx = handle;
        call(&mut r, 0x49aab0, ECX, EAX, &[], 0);
        let kind = r.eax;
        let changed = !eq(kind, rd(0xe9a6a8));
        if changed {
            cov(0x49aeb5);
        }
        wb(0xe9a6a4, changed as u8);
        wd(0xe9a6a8, kind);
        // full refresh when the kind or the target changed, or the stamp is invalid / stale
        let t2 = rd(STAMP) as i32;
        let full = changed
            || !eq(handle, rd(0xe9a6a0))
            || t2 >= 0 && if t2 > now { t2.wrapping_sub(now) >= STAMP_WINDOW } else { now.wrapping_sub(t2) < STAMP_WINDOW };
        if full {
            cov(0x49aee8);
            r.eax = kind;
            call(&mut r, 0x49acd0, EAX, NONE, &[rd(0xe9a5bc)], 1);
            let idx = rd(0xe9a680);
            let state = rd(0xe9a6a8);
            let sel = if rb(0x25f633c + (idx.wrapping_mul(0x2c) as usize)) != 0 { idx } else { state };
            let mut blend = q(rd(0x25f6338 + (sel.wrapping_mul(0x2c) as usize)));
            let ctx = rd(0xe9a67c);
            if (eq(ctx, 0) && eq(state, 1)) || (eq(ctx, 1) && eq(state, 0)) {
                cov(0x49af57);
                blend = q(rd(0xe99860));
            }
            let flagged = rd(0x21703d4) as usize;
            if flagged != 0 && rb(flagged + 0x595) != 0 {
                cov(0x49af73);
                blend = 1.0f32.to_bits();
            }
            r.eax = handle;
            r.edx = rd(0xe9a694);
            call(&mut r, 0x49a6e0, EAX | EDX, NONE, &[state, blend], 2);
            wd(0xe9a69c, q(rd(0xe9a698)));
            wd(0xe9a694, rd(0xe9a690));
            wd(0xe9a6a0, handle);
        } else {
            // same kind and target, stamp usable: only the context id may have moved
            let cur = rd(0xe9a5bc);
            let want = rd(0xe9a67c);
            if !eq(cur, want) {
                cov(0x49af46);
                r.eax = cur;
                call(&mut r, 0x49acd0, EAX, NONE, &[want], 1);
            }
        }
    } else {
        cov(0x49afb3);
    }
    // an expired stamp is cleared
    let t = rd(STAMP) as i32;
    if t >= 0 && if t > now { t.wrapping_sub(now) >= STAMP_WINDOW } else { now.wrapping_sub(t) < STAMP_WINDOW } {
        cov(0x49afd1);
        wd(STAMP, 0xffff_ffff);
    }
    cov(0x49afdb);
    call(&mut r, 0x49a4c0, NONE, NONE, &[dt], 1);
}

// --- camera_aim_pitch_step (0x497A20..0x497C7A, 602 bytes) --------------------------------------------
//
// One step of the aim camera's pitch follow: builds an orientation from the player's aim direction (the human's
// facing at +0x38, a supplied direction, or a direction derived from the human's weapon vectors at +0x2C), takes its
// pitch and rate-limits the camera's pitch (cam+0x20) toward it with 0x4970C0.
// Registers: EAX = &direction vector (3 f32), ECX = human, stack: cdecl (cam, f32 rate), plain `ret`; the original
// uses the first argument slot as an f32 scratch cell (reproduced, the difftest compares it). Single caller (the
// follow update), which does not read eax / ecx / edx afterwards; ebx / esi / edi / ebp are preserved.

#[repr(C)]
struct AimFrame {
    _unused: u32,
    /// first cross product / spill cell of the angle (an f64 is stored over the first two dwords)
    b: [u32; 4],
    c: [u32; 3],
    mat: [u32; 9],
    _tail: u32,
}

/// `(a*x + b*y) + c*z` with the constant vector 0xEC5ACC (x87 order: products in f64, sums in f64, one f32 store).
#[inline(always)]
unsafe fn dot_up(x: u32, y: u32, z: u32) -> f32 {
    let (a, b, c) = (rf(0xec5acc) as f64, rf(0xec5ad0) as f64, rf(0xec5ad4) as f64);
    (((f32::from_bits(x) as f64) * a + (f32::from_bits(y) as f64) * b) + (f32::from_bits(z) as f64) * c) as f32
}

pub(super) unsafe fn camera_aim_pitch_step_body(ent: &mut Ent, args: *const u32) {
    cov(0x497a20);
    let a = args as usize;
    let cam = rd(a) as usize;
    let rate = rd(a + 4);
    let dir = ent.eax as usize;
    let human = ent.ecx as usize;
    wf(a, 0.0);
    let mut r = Rg::entry(ent);
    r.ebx = cam as u32;
    r.esi = human as u32;
    r.edi = dir as u32;
    let mut scratch = 0.0f32;

    let mut buf = core::mem::MaybeUninit::<AimFrame>::uninit();
    let fr = local_frame(&mut buf, 0);
    let vb = core::ptr::addr_of_mut!((*fr).b) as u32;
    let vc = core::ptr::addr_of_mut!((*fr).c) as u32;
    let mat = core::ptr::addr_of_mut!((*fr).mat) as u32;

    #[derive(PartialEq)]
    enum Src {
        /// the human's own vector (+0x38)
        Human,
        /// the supplied direction, if it has a length
        Dir,
        /// the human's weapon vectors
        Weapon,
        /// the vector already in eax
        Built,
        Nothing,
    }
    let mut src = Src::Nothing;
    if human != 0 && rd(human + 0x4c) == 5 {
        cov(0x497a46);
        r.eax = human as u32;
        call(&mut r, 0xab5340, EAX, AL, &[], 0);
        if r.eax as u8 == 0 && rd(rd(human + 0x84e4) as usize + 0x9c) != 3 {
            cov(0x497a6d);
            r.ecx = 0xec5ae4;
            r.eax = mat;
            call(&mut r, 0xbe2c60, EAX | ECX, NONE, &[], 0);
            let mut aac = true;
            if rd(rd(human + 0x84e4) as usize + 0x9c) == 1 {
                r.eax = human as u32;
                call(&mut r, 0xadd870, EAX, AL, &[], 0);
                if r.eax as u8 != 0 {
                    aac = false;
                    src = Src::Dir;
                } else {
                    call(&mut r, 0xaab8e0, NONE, EAX, &[vc], 0);
                    if r.eax <= 1 {
                        aac = false;
                        src = Src::Dir;
                    }
                }
            }
            if aac {
                cov(0x497aac);
                if (rd(human + 0x8f50) >> 12) & 1 != 0 {
                    src = Src::Human;
                } else {
                    scratch = dot_up(rd(human + 0x38), rd(human + 0x3c), rd(human + 0x40));
                    wf(a, scratch);
                    call(&mut r, 0xbf1670, NONE, AL, &[q(scratch.to_bits()), 0, q(rd(0xe5c584))], 0);
                    src = if r.eax as u8 != 0 { Src::Human } else { Src::Dir };
                }
            }
            if src == Src::Human {
                cov(0x497b02);
                r.eax = (human + 0x38) as u32;
                src = Src::Built;
            }
            if src == Src::Dir {
                cov(0x497b0a);
                // the supplied direction, when it is long enough to point somewhere
                let (x, y, z) = (rf(dir) as f64, rf(dir + 4) as f64, rf(dir + 8) as f64);
                let len2 = ((x * x + y * y) + z * z) as f32;
                scratch = len2;
                wf(a, scratch);
                src = Src::Weapon;
                if len2 > rf(0xdd518c) {
                    cov(0x497b37);
                    scratch = dot_up(rd(dir), rd(dir + 4), rd(dir + 8));
                    wf(a, scratch);
                    call(&mut r, 0xbf1670, NONE, AL, &[q(scratch.to_bits()), 0, q(rd(0xe885a8))], 0);
                    if r.eax as u8 != 0 {
                        cov(0x497b7f);
                        r.esi = vc;
                        call(&mut r, 0xbddba0, ESI | EDI, EAX, &[], 0);
                        src = Src::Built;
                    }
                }
            }
            if src == Src::Weapon {
                cov(0x497b8a);
                // the weapon vectors at +0x2C: a direction exists when their projection is large enough
                let w = human + 0x2c;
                scratch = dot_up(rd(w), rd(w + 4), rd(w + 8));
                wf(a, scratch);
                r.edi = w as u32;
                if (scratch as f64) > rdf64(0xe5cdb8) {
                    cov(0x497bc0);
                    r.ecx = w as u32;
                    r.eax = vb;
                    call(&mut r, 0x42eea0, EAX | ECX, NONE, &[0xec5acc], 1);
                    r.esi = vb;
                    call(&mut r, 0xbde150, ESI, NONE, &[], 0);
                    r.ecx = r.esi;
                    r.eax = vc;
                    call(&mut r, 0x42eea0, EAX | ECX, NONE, &[w as u32], 1);
                    r.esi = vc;
                    call(&mut r, 0xbde150, ESI, NONE, &[], 0);
                    r.eax = r.esi;
                    src = Src::Built;
                }
            }
            if src == Src::Built {
                cov(0x497bf0);
                r.ecx = mat;
                call(&mut r, 0xbe4b20, EAX | ECX, NONE, &[], 0);
            }
            cov(0x497bf9);
            // pitch error: camera angle minus the orientation's angle, wrapped; scaled by the matrix's pitch term
            let angle = rf(cam + 4);
            core::ptr::write_unaligned(vb as *mut f64, angle as f64);
            r.ecx = mat;
            call(&mut r, 0xbe3380, ECX, XMM0, &[], 0);
            let turn = f32::from_bits(r.xmm0);
            scratch = ((angle as f64) - (turn as f64)) as f32;
            wf(a, scratch);
            let wrapped = call(&mut r, 0x40b820, NONE, NONE, &[q(scratch.to_bits())], 0);
            scratch = wrapped.st[0] as f32;
            wf(a, scratch);
            let asin = call(&mut r, 0x4f1ac0, NONE, NONE, &[q(rd(mat as usize + 28))], 0);
            scratch = ((-asin.st[0]) * (scratch as f64)) as f32;
            wf(a, scratch);
        }
    }
    cov(0x497c45);
    let out = call(&mut r, 0x4970c0, NONE, NONE, &[q(rd(cam + 0x20)), q(scratch.to_bits()), q(rd(0xe999e0)), q(rate)], 0);
    wf(cam + 0x20, out.st[0] as f32);
}

// --- camera_orient_follow_step (0x499190..0x499410, 640 bytes) -----------------------------------------
//
// One step of the orientation follow of the camera context `ctx` (stack argument, cdecl, plain `ret`): blends the
// context's stored orientation toward the followed entity's (as quaternions through 0xC05E80 / 0xC057D0), turns the
// result into a yaw/pitch correction (+0x0C, +0x24, 0x25F5C58) and clears the context's request flag (+0x00) when
// the request has expired. The original uses the argument slot as an f32 scratch cell (reproduced; the difftest
// compares it). Single caller (the follow update), which does not read eax / ecx / edx afterwards.
// (The translated port covered 742 bytes; the function really ends at the `ret` at 0x49940F, the bytes after it are
// the next function.)

#[repr(C)]
struct OrientFrame {
    _unused: u32,
    /// the camera angle (+4) stored as an f32 for the yaw difference
    spill: u32,
    from: [u32; 4],
    to: [u32; 4],
    blend: [u32; 4],
    mat: [u32; 9],
}

pub(super) unsafe fn camera_orient_follow_step_body(ent: &mut Ent, args: *const u32) {
    cov(0x499190);
    let a = args as usize;
    let cx = rd(a) as usize;
    let mut r = Rg::entry(ent);
    r.ebx = cx as u32;
    let mut buf = core::mem::MaybeUninit::<OrientFrame>::uninit();
    let fr = local_frame(&mut buf, 0);
    let from = core::ptr::addr_of_mut!((*fr).from) as u32;
    let to = core::ptr::addr_of_mut!((*fr).to) as u32;
    let blend = core::ptr::addr_of_mut!((*fr).blend) as u32;
    let mat = core::ptr::addr_of_mut!((*fr).mat) as u32;

    // the followed entity (handle at +0x90 -> slot table; must be a kind-5 object carrying the same handle)
    let h = rd(cx + 0x90);
    let mut e = 0usize;
    if h != 0 {
        let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
        if slot != 0 && eq(rd(slot + 0x44), h) && eq(rd(slot + 0x4c), 5) {
            e = slot;
        }
    }
    if e != 0 {
        cov(0x4991d3);
        r.ecx = (e + 0x20) as u32;
        r.edx = (cx + 0x98) as u32;
        call(&mut r, 0x406810, ECX | EDX, NONE, &[from], 0);
        r.eax = from;
        r.ecx = mat;
        call(&mut r, 0xbe4b20, EAX | ECX, NONE, &[], 0);
        r.edi = 0x25f5b5c;
        r.eax = to;
        call(&mut r, 0xc053e0, EAX | EDI, NONE, &[], 0);
        r.edi = mat;
        r.eax = from;
        call(&mut r, 0xc053e0, EAX | EDI, NONE, &[], 0);
        // blend factor: the context's rate times a constant, at most 1.0
        let rate = ((rf(cx + 0x94) as f64) * (rf(0xe8437c) as f64)) as f32;
        wf(a, rate);
        let mut t = rate;
        if !(1.0f64 > rate as f64) {
            cov(0x49922e);
            t = 1.0;
            wf(a, t);
        }
        r.ecx = from;
        r.esi = to;
        r.edi = blend;
        call(&mut r, 0xc05e80, ECX | ESI | EDI, NONE, &[q(t.to_bits())], 0);
        r.ecx = mat;
        r.eax = r.edi;
        call(&mut r, 0xc057d0, EAX | ECX, NONE, &[], 0);
        let angle = q(rd(cx + 4));
        (*fr).spill = angle;

        if rb(cx + 0xa8) != 0 {
            cov(0x49926d);
            call(&mut r, 0xbe3380, ECX, XMM0, &[], 0);
            wd(a, r.xmm0);
            let v = q(r.xmm0);
            wd(a, v);
            call(&mut r, 0xbf14d0, NONE, XMM0, &[v, 0], 0);
            wd(a, r.xmm0);
            wd(0x25f5c58, q(r.xmm0));
        }
        if rb(cx + 0xa9) != 0 {
            cov(0x4992b1);
            let asin = call(&mut r, 0x4f1ac0, NONE, NONE, &[q(rd(mat as usize + 28))], 0);
            let s = (-asin.st[0]) as f32;
            wf(a, s);
            wf(cx + 0x24, s);
            wf(cx + 0x24, ((s as f64) - ((rf(cx + 0x20) as f64) + (rf(0xe9a650) as f64))) as f32);
        }
        // yaw correction: wrapped angle difference scaled by the context's weight, clamped
        let w = rf(cx + 0xac);
        if !(w as f64 == 0.0) {
            cov(0x4992ef);
            let diff = ((rf(cx + 4) as f64) - (f32::from_bits((*fr).spill) as f64)) as f32;
            wf(a, diff);
            let (top, bottom, turn) = (rdf64(0xe5c160), rdf64(0xe5c170), rdf64(0xe5bf78));
            let mut v = diff as f64;
            if top < v {
                cov(0x49930f);
                let f = (v - turn) as f32;
                wf(a, f);
                v = f as f64;
            } else if v < bottom {
                cov(0x499324);
                let f = (v + turn) as f32;
                wf(a, f);
                v = f as f64;
            }
            if rb(0x2526b9f) != 0 {
                cov(0x49933b);
                wf(cx + 0xc, 0.0);
            } else {
                cov(0x499344);
                let x = (((-(w as f64)) * v) / top) as f32;
                wf(a, x);
                wf(cx + 0xc, x);
                let (lo, hi) = (rf(0xe5cf30), rf(0xdd347c));
                if !((lo as f64) < x as f64) {
                    cov(0x49938c);
                    wf(a, lo);
                    wf(cx + 0xc, lo);
                } else if (hi as f64) <= x as f64 {
                    cov(0x49937d);
                    wf(a, hi);
                    wf(cx + 0xc, hi);
                } else {
                    cov(0x49937f);
                    wf(a, x);
                    wf(cx + 0xc, x);
                }
            }
        }
    }
    // the request flag (+0x00) is cleared when its stamp (+0xa4) is unset, or the stamp is a long way out on the
    // future side, or inside the window on the past side; otherwise only when the camera is moving (0x25F5CA0..)
    cov(0x49939d);
    let stamp = rd(cx + 0xa4) as i32;
    let now = rd(CLOCK) as i32;
    let moving = |a: usize| -> bool {
        let (x, y) = (rf(0x25f5ca0) as f64, rf(0x25f5ca4) as f64);
        let len2 = (y * y + x * x) as f32;
        wf(a, len2);
        !((len2 as f64) < rf(0xe5c6a4) as f64 || len2.is_nan())
    };
    let clear = if stamp < 0 {
        true
    } else if stamp > now {
        if stamp.wrapping_sub(now) < STAMP_WINDOW {
            cov(0x4993da);
            moving(a)
        } else {
            cov(0x4993c3);
            true
        }
    } else if now.wrapping_sub(stamp) < STAMP_WINDOW {
        true
    } else {
        cov(0x4993da);
        moving(a)
    };
    if clear {
        cov(0x499403);
        wd(cx, 0);
    }
}

// --- camera_clip_segment (0x4938C0..0x493AE8, 552 bytes) -----------------------------------------------
//
// Clips the camera's move from point A to point B against the world: casts the segment through the collision
// query 0x6FA820 and returns, in the output vector, either B (nothing in the way, or only things the camera
// may pass through) or the position of the first blocking hit (which also sets the "camera was blocked" flag
// 0x2526B5A).
// Registers: EAX = &A (3 f32), EDI = &B; stack (cdecl): &out (3 f32), a callback id for the query (0x4941F0 from
// the follow camera); returns eax = &out. Plain `ret`, game thread. Exit registers: eax = &out; ebx / esi / edi /
// ebp unchanged; ecx / edx are scratch for all six callers.
// The original aligns esp to 8 after its frame pointer; the frame below is the query block (88 bytes), the hit list
// header and the loop counter.

#[repr(C)]
struct ClipFrame {
    /// hit index
    i: u32,
    /// hit list: pointer to 60-byte records, count, capacity word (bit 31 set = no buffer to free)
    list: u32,
    count: u32,
    cap: u32,
    /// the query block: from, to, radius, ..., callback, mode words
    q: [u32; 22],
}

pub(super) unsafe fn camera_clip_segment_body(ent: &mut Ent, args: *const u32) {
    cov(0x4938c0);
    let out = rd(args as usize) as usize;
    let callback = rd(args as usize + 4);
    let from = ent.eax as usize;
    let to = ent.edi as usize;
    let mut r = Rg::entry(ent);
    r.ebx = out as u32;
    r.esi = 0;
    // the point written to `out` when nothing blocks (fld/fstp: quiet NaNs)
    let put = |v: usize| {
        for k in 0..3 {
            wd(out + 4 * k, q(rd(v + 4 * k)));
        }
    };
    ent.eax = out as u32;
    if !eq(rd(0x252864c), 0) {
        cov(0x4938d8);
        put(to);
        return;
    }
    cov(0x4938f0);
    let mut buf = core::mem::MaybeUninit::<ClipFrame>::uninit();
    let fr = local_frame(&mut buf, 8);
    let f = &mut *fr;
    f.q[6] = q(rd(0xe99a70));
    for k in 0..3 {
        f.q[k] = rd(from + 4 * k);
        f.q[3 + k] = rd(to + 4 * k);
    }
    f.q[7] = 0;
    f.q[17] = 0;
    f.q[18] = callback;
    f.q[19] = 3;
    f.q[20] = 1;
    f.q[21] = 1;
    f.list = 0;
    f.count = 0;
    f.cap = 0x8000_0000;
    let res = core::ptr::addr_of_mut!(f.list) as u32;
    r.eax = 1;
    r.ecx = core::ptr::addr_of_mut!(f.q) as u32;
    r.edx = res;
    r.edi = to as u32;
    call(&mut r, 0x6fa820, EAX | ECX | EDX | EBX | ESI | EDI, AL, &[res, 1, 0], 0);
    if r.eax as u8 == 0 {
        cov(0x493ac7);
        put(to);
        r.ecx = res;
        call(&mut r, 0x4080b0, ECX, NONE, &[], 0);
        return;
    }
    cov(0x493971);
    f.i = 0;
    // the first blocking hit, if any
    let mut hit = 0usize;
    let mut blocked = false;
    if (f.count as i32) > 0 {
        let mut p = f.list as usize;
        loop {
            cov(0x493983);
            let world = rd(0x21703d4);
            call(&mut r, 0x9b9320, NONE, AL, &[world], 0);
            if r.eax as u8 == 0 {
                cov(0x493a6f);
                hit = p;
                break;
            }
            let id = rd(p + 0x28);
            if !eq(rd(world as usize + 0xd74), id) {
                hit = p;
                break;
            }
            // the object the hit belongs to (must be a live kind-5 object), then its collision class
            let mut obj = 0usize;
            if id != 0 {
                let slot = rd(0x2149c64 + (((id & 0xfff) << 4) as usize)) as usize;
                if slot != 0 && eq(rd(slot + 0x44), id) && eq(rd(slot + 0x4c), 5) {
                    cov(0x4939cf);
                    obj = slot;
                }
            }
            r.esi = p as u32;
            call(&mut r, 0xaad320, NONE, EAX, &[obj as u32, p as u32], 0);
            if r.eax == 0 {
                hit = p;
                break;
            }
            let class = rd(rd(r.eax as usize + 0xb4) as usize + 4);
            if !(eq(class, 4) || eq(class, 8) || eq(class, 1)) {
                hit = p;
                break;
            }
            cov(0x493a07);
            f.i += 1;
            p += 0x3c;
            if (f.i as i32) >= (f.count as i32) {
                break;
            }
        }
        blocked = hit != 0;
    }
    if blocked {
        cov(0x493a73);
        for k in 0..3 {
            wd(out + 4 * k, q(rd(hit + 0xc + 4 * k)));
        }
        wb(0x2526b5a, 1);
    } else {
        cov(0x493a1f);
        put(to);
    }
    // give the hit list back to the pool
    let cap = f.cap;
    if (cap as i32) >= 0 {
        cov(0x493a3b);
        let key = rd(0x31bf2bc);
        r.eax = key;
        call(&mut r, rd(0xdb807c) as usize, NONE, EAX, &[key], 1);
        let tls = r.eax;
        let bytes = (cap & 0x3fff_ffff).wrapping_mul(60);
        r.ecx = tls;
        call(&mut r, 0xd2b7d0, ECX, NONE, &[f.list, bytes, 0x17], 3);
    }
}

// --- camera_follow_idle_step (0x499480..0x4996F0, 624 bytes) --------------------------------------------
//
// Upkeep of the follow camera's "idle" state for the context `ctx`: updates the two context counters (0x498770,
// 0x4988C0 with `rate` and `mode`), clears the idle yaw (+0x0C), and decides whether the idle timer (+0x68 / +0x6C)
// restarts and the request flag (+0x00) clears: it does when the camera is not blocked by a script / cutscene /
// the player's vehicle state (speeds and steering below thresholds).
// Stack (cdecl): ctx, f32 rate, mode; plain `ret`; single caller (the follow update). ESI = ctx is passed to both
// counters. The first argument slot is used as an f32 scratch cell in the vehicle case (reproduced). Exit registers:
// ebx / esi / edi / ebp unchanged, eax / ecx / edx are scratch for the caller.
// (The translated port covered 799 bytes; the function ends at the `ret` at 0x4996EF, the rest is the next function.)

#[repr(C)]
struct IdleFrame {
    _unused: u32,
    speed: u32,
    dot: u32,
    vec: [u32; 3],
}

/// `t + delta` as the timer words are kept: a sum past 0x6B49D200 wraps by 0x94B62E00.
fn timer_add(t: u32, delta: u32) -> u32 {
    let v = t.wrapping_add(delta);
    if (v as i32) > 0x6b49_d200 { v.wrapping_add(0x94b6_2e00) } else { v }
}

pub(super) unsafe fn camera_follow_idle_step_body(ent: &mut Ent, args: *const u32) {
    cov(0x499480);
    let a = args as usize;
    let ctx = rd(a) as usize;
    let rate = q(rd(a + 4));
    let mode = rd(a + 8);
    let mut r = Rg::entry(ent);
    r.ebx = ctx as u32;
    r.esi = ctx as u32;
    r.edi = mode;
    call(&mut r, 0x498770, ESI, NONE, &[rate, mode], 0);
    call(&mut r, 0x4988c0, ESI, NONE, &[rate, mode], 0);
    wf(ctx + 0xc, 0.0);

    // the followed object, when it is a live kind-5 object
    let h = rd(CAM_TARGET_HANDLE);
    let mut e = 0usize;
    if h != 0 {
        let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
        if slot != 0 && eq(rd(slot + 0x44), h) && eq(rd(slot + 0x4c), 5) {
            e = slot;
        }
    }
    if rd(ctx + 0xe8) != 0 {
        return;
    }
    let stamp = rd(0xe9a6b0) as i32;
    let now = rd(CLOCK);
    if stamp < 0 {
        cov(0x499505);
        wd(ctx, 0);
        wd(ctx + 0x68, timer_add(now, 0x2710));
        return;
    }
    // only while the stamp is out of its window
    let inside = if stamp > now as i32 { stamp.wrapping_sub(now as i32) < STAMP_WINDOW } else { now.wrapping_sub(stamp as u32) as i32 >= STAMP_WINDOW };
    if inside {
        return;
    }
    cov(0x49955c);
    let world = rd(0x21703d4) as usize;
    let flags = rd(world + 0xc0);
    r.esi = world as u32;
    r.ecx = flags >> 30;
    r.eax = flags >> 29;
    if (flags >> 30) & 1 != 0 || (flags >> 29) & 1 != 0 || rb(0x2529771) != 0 || eq(rd(0xe9a5bc), 7) {
        return;
    }
    // is the player in a vehicle state that keeps the camera from idling?
    let mut restart = true;
    if e != 0 && eq(rb(e + 0x84d8) as u32, 1) {
        cov(0x4995a3);
        let state = rd(rd(e + 0x84e4) as usize + 0x9c);
        r.edx = rd(e + 0x84e4);
        r.eax = state.wrapping_sub(3);
        r.edi = e as u32;
        if eq(state, 3) {
            cov(0x49960d);
            let v = rd(e + 0x540) as usize;
            if v != 0 {
                let lean = q(rd(v + 0x24));
                wd(a, lean);
                r.eax = v as u32;
                r.ecx = e as u32;
                let speed = call(&mut r, 0xaa6a90, EAX | ECX | EDX | ESI | EDI, NONE, &[], 0);
                let speed = speed.st[0] as f32;
                let mut buf = core::mem::MaybeUninit::<IdleFrame>::uninit();
                let fr = local_frame(&mut buf, 0);
                (*fr).speed = speed.to_bits();
                r.ecx = e as u32;
                let vec = core::ptr::addr_of_mut!((*fr).vec) as u32;
                r.eax = vec;
                call(&mut r, 0xaa69d0, EAX | ECX | EDX | ESI | EDI, NONE, &[vec], 0);
                let vv = (*fr).vec;
                let dot = (((rf(e + 0x30) as f64) * (f32::from_bits(vv[1]) as f64) + (f32::from_bits(vv[0]) as f64) * (rf(e + 0x2c) as f64))
                    + (rf(e + 0x34) as f64) * (f32::from_bits(vv[2]) as f64)) as f32;
                (*fr).dot = dot.to_bits();
                let lean_abs = f32::from_bits(lean & 0x7fff_ffff);
                wf(a, lean_abs);
                // a lean below the threshold and a speed under the vehicle's limit: leave the idle state alone
                if (lean_abs as f64) < rdf64(0xe5c1a0) && (rf(rd(e + 0x84e4) as usize + 0x4f4) as f64) > speed as f64 {
                    cov(0x49968b);
                    return;
                }
                let dot_abs = f32::from_bits(dot.to_bits() & 0x7fff_ffff);
                wf(a, dot_abs);
                if (dot_abs as f64) > rdf64(0xe5c0f8) {
                    return;
                }
            }
        } else if eq(state, 4) {
            cov(0x4995ca);
            r.eax = state.wrapping_sub(4);
            let v = rd(e + 0x550) as usize;
            if v != 0 {
                r.edi = v as u32;
                let abs = call(&mut r, 0x49ee60, NONE, NONE, &[q(rd(v + 0x68))], 0);
                if abs.st[0] < rdf64(0xe5c1a0) {
                    return;
                }
                call(&mut r, 0x960c30, ECX | EDX | ESI | EDI, AL, &[], 0);
                if r.eax as u8 != 0 {
                    cov(0x499605);
                    restart = false;
                }
            }
        }
    }
    if restart {
        cov(0x4996a8);
        wd(ctx + 0x68, timer_add(now, 0x2710));
        wd(0xe9a6b0, 0xffff_ffff);
        wd(ctx + 0x6c, timer_add(now, 0x7d0));
        wd(ctx, 0);
    }
}

// --- camera_context_orient_step (0x498D40..0x499112, 979 bytes) ---------------------------------------
//
// One step of a camera context `cam` (stack argument 0; the follow update passes its own context blocks) that
// follows the player's orientation. When the player (the camera target 0x25F5BB0, a kind-5 entity) is in a state
// that allows it and is the world's human in its seat, the context's angles (+0x24, +0x10, +0x04) are blended toward
// the player's matrix (quaternion slerp through 0xC05E80; the weight comes from the player's state record, a running
// timer at cam+0x68 and the angle between the two headings). The resulting corrections go to the camera angle
// cell 0x25F5C58 and to cam+0x0C / +0x10. Otherwise the older step 0x498A20 (cam, dt, mode in eax) runs.
// At the end the request word cam+0x00 is set to 4 when the idle stamp 0xE9A6B0 has run out, or when the player
// is in a state-4 seat of the world human.
// Arguments (cdecl, plain `ret`): cam, f32 dt, mode (eax for 0x498A20). Single caller (the follow update), which
// reads none of eax / ecx / edx afterwards; ebx / esi / edi / ebp are preserved.
// The original reads one stack cell that no code path wrote when neither blend weight branch ran (the "heading"
// cell +0x1C, stored into cam+0x0C / +0x10); the frame below sits at the same stack address, so the garbage is the
// same.

/// Offsets of the 0x68-byte local area (the original's esp+0x10 .. esp+0x78 after its prologue).
mod of {
    pub const T14: usize = 0x04;
    pub const T18: usize = 0x08;
    pub const T1C: usize = 0x0c;
    pub const T20: usize = 0x10;
    pub const Q2: usize = 0x14;
    pub const Q1: usize = 0x24;
    pub const Q3: usize = 0x34;
    pub const MAT: usize = 0x44;
}

#[repr(C)]
struct OrientCtxFrame {
    w: [u32; 26],
}

/// The kind-5 entity of the handle `h` (handle -> slot table -> entity with the same id at +0x44), else 0.
#[inline(always)]
unsafe fn kind5_entity(h: u32) -> usize {
    if h == 0 {
        return 0;
    }
    let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
    if slot == 0 || !eq(rd(slot + 0x44), h) {
        return 0;
    }
    if eq(rd(slot + 0x4c), 5) { slot } else { 0 }
}

/// The stamp is set (>= 0) and the clock has passed it by the 900,000,000 window (see timer_elapsed).
#[inline(always)]
fn stamp_run_out(stamp: i32, now: i32) -> bool {
    stamp >= 0 && if stamp > now { stamp.wrapping_sub(now) < STAMP_WINDOW } else { now.wrapping_sub(stamp) >= STAMP_WINDOW }
}

pub(super) unsafe fn camera_context_orient_step_body(ent: &mut Ent, args: *const u32) {
    cov(0x498d40);
    let a = args as usize;
    let cam = rd(a) as usize;
    let dt = q(rd(a + 4));
    let mode = rd(a + 8);
    let mut r = Rg::entry(ent);
    let mut buf = core::mem::MaybeUninit::<OrientCtxFrame>::uninit();
    let fr = local_frame(&mut buf, 0) as usize;

    let ply = kind5_entity(rd(CAM_TARGET_HANDLE));
    wf(cam + 0xc, 0.0);
    // `ok`: the player's state allows the blend; `mine`: the occupant of the player's seat is the world's human
    let mut ok = false;
    let mut mine = false;
    if ply != 0 {
        cov(0x498d8d);
        let st = rd(ply + 0x84e4) as usize;
        if eq(rd(st + 0x9c), 2) {
            cov(0x498d9c);
            r.edi = ply as u32;
            call(&mut r, 0xaab6b0, EDI, AL, &[], 0);
            ok = r.eax as u8 == 0;
        } else {
            cov(0x498dbe);
            ok = (rd(st + 0x5b0) >> 30) & 1 != 0;
        }
        let seat = if eq(rb(ply + 0x84d8) as u32, 1) {
            let v = rd(ply + 0x6ce8) as i32;
            if v > 0 {
                cov(0x498dd0);
                Some(())
            } else {
                None
            }
        } else {
            cov(0x498dd6);
            Some(())
        };
        let occupant = if seat.is_some() {
            r.eax = rd(ply + 0x8abc);
            call(&mut r, 0x960d60, EAX, EAX, &[], 0);
            r.eax
        } else {
            cov(0x498dd2);
            0
        };
        mine = eq(occupant, rd(0x21703d4));
    }

    if !ok {
        cov(0x499044);
        r.eax = mode;
        call(&mut r, 0x498a20, EAX, NONE, &[cam as u32, dt], 0);
    } else if !mine {
        cov(0x49903d);
    } else {
        cov(0x498e06);
        // the context's angles -> matrix -> quaternion
        let mat = fr + of::MAT;
        call(&mut r, 0xbe2d90, NONE, NONE, &[mat as u32, q(rd(cam + 0x24)), q(rd(cam + 0x10)), q(rd(cam + 4))], 4);
        r.eax = (fr + of::Q1) as u32;
        r.edi = mat as u32;
        call(&mut r, 0xc053e0, EAX | EDI, NONE, &[], 0);

        // blend weight (t14): a running timer at cam+0x68 shapes it
        let stamp = cam + 0x68;
        let mut st: usize;
        let mut running = false;
        if (rd(stamp) as i32) >= 0 {
            r.eax = stamp as u32;
            call(&mut r, 0xbdd230, EAX, AL, &[], 0);
            running = r.eax as u8 == 0;
        }
        let w: f64;
        if running {
            cov(0x498e5a);
            st = rd(ply + 0x84e4) as usize;
            wd(fr + of::T18, q(rd(st + 0x4d0)));
            r.eax = stamp as u32;
            call(&mut r, 0xbdd260, EAX, EAX, &[], 0);
            let remaining = r.eax;
            wd(fr + of::T14, remaining);
            wf(fr + of::T1C, ((remaining as i32 as f64) / rdf64(0xe5bf98)) as f32);
            if eq(rd(st + 0x9c), 3) {
                cov(0x498e8c);
                r.ecx = ply as u32;
                let sp = call(&mut r, 0xaa6a90, ECX, NONE, &[], 0);
                wf(fr + of::T14, sp.st[0] as f32);
                st = rd(ply + 0x84e4) as usize;
                // speed over the vehicle's top speed; under the limit constant it counts as 1
                wf(fr + of::T14, ((rf(fr + of::T14) as f64) / (rf(st + 0x4f4) as f64)) as f32);
                if (rf(fr + of::T14) as f64) < rdf64(0xe5bf40) {
                    cov(0x498ebc);
                    wf(fr + of::T14, 1.0);
                }
                wf(fr + of::T18, ((rf(fr + of::T14) as f64) * (rf(fr + of::T18) as f64)) as f32);
            }
            let st3 = rd(ply + 0x84e4) as usize;
            r.eax = st3 as u32;
            let lo = rf(st3 + 0x4cc) as f64;
            w = (((rf(fr + of::T18) as f64) - lo) * (rf(fr + of::T1C) as f64)) + lo;
        } else {
            cov(0x498eea);
            st = rd(ply + 0x84e4) as usize;
            w = rf(st + 0x4cc) as f64;
        }
        wf(fr + of::T14, w as f32);
        let k = rf(0xe8437c);
        wf(fr + of::T14, ((k as f64) * (rf(fr + of::T14) as f64)) as f32);
        if rd(st + 0x5b0) & 0x8000_0000 != 0 {
            cov(0x498f1a);
            wf(fr + of::T20, ((k as f64) * (rf(st + 0x4d4) as f64)) as f32);
            // angle between the context's heading (matrix row 2) and the player's heading
            let m = |i: usize| rf(fr + of::MAT + 4 * i) as f64;
            let dot = (((rf(ply + 0x3c) as f64) * m(7) + (rf(ply + 0x38) as f64) * m(6)) + (rf(ply + 0x40) as f64) * m(8)) as f32;
            wf(fr + of::T1C, dot);
            let angle: f32;
            if (dot as f64) < 1.0 || dot.is_nan() {
                cov(0x498f58);
                // the library's acos reads the upper half of eax (the original has `fnstsw ax` here: ax = 0x3100)
                r.eax = (r.eax & 0xffff_0000) | 0x3100;
                let ac = call(&mut r, 0x42ef30, EAX, NONE, &[q(dot.to_bits())], 0);
                angle = ac.st[0].abs() as f32;
                wf(fr + of::T1C, angle);
            } else {
                cov(0x498f52);
                angle = 0.0;
            }
            wf(fr + of::T18, angle);
            let lim = rf(fr + of::T20);
            if (angle as f64) < (lim as f64) {
                cov(0x498f83);
                wf(fr + of::T14, 1.0);
            } else {
                cov(0x498f8f);
                let ratio = ((lim as f64) / (angle as f64)) as f32;
                wf(fr + of::T20, ratio);
                if (ratio as f64) > (rf(fr + of::T14) as f64) {
                    cov(0x498fa8);
                    wf(fr + of::T14, ratio);
                }
            }
        }
        cov(0x498fb0);
        // slerp from the context's quaternion toward the player's by t14, back to a matrix, then to angles
        r.eax = (fr + of::Q2) as u32;
        r.edi = (ply + 0x20) as u32;
        call(&mut r, 0xc053e0, EAX | EDI, NONE, &[], 0);
        r.ecx = (fr + of::Q2) as u32;
        r.esi = (fr + of::Q1) as u32;
        r.edi = (fr + of::Q3) as u32;
        call(&mut r, 0xc05e80, ECX | ESI | EDI, NONE, &[q(rd(fr + of::T14))], 0);
        r.eax = (fr + of::Q3) as u32;
        r.ecx = mat as u32;
        call(&mut r, 0xc057d0, EAX | ECX, NONE, &[], 0);
        // cam+0x24 gets the first angle, t20 / t1c the others
        r.esi = mat as u32;
        r.edi = (fr + of::T1C) as u32;
        call(&mut r, 0xbe3150, ESI | EDI, NONE, &[(cam + 0x24) as u32, (fr + of::T20) as u32], 2);
        call(&mut r, 0xbf14d0, NONE, XMM0, &[q(rd(fr + of::T20)), 0], 0);
        wd(fr + of::T20, r.xmm0);
        wd(0x25f5c58, q(rd(fr + of::T20)));
        if !eq(rd(rd(ply + 0x84e4) as usize + 0x7c8), 1) {
            cov(0x49902b);
            let v = q(rd(fr + of::T1C));
            wd(cam + 0x10, v);
            wd(cam + 0xc, v);
        } else {
            cov(0x499061);
        }
    }

    // the player's state record says the blend runs "by heading": cam+0x0C = scale * the heading angle
    if ply != 0 {
        let st = rd(ply + 0x84e4) as usize;
        if eq(rd(st + 0x7c8), 1) && mine {
            cov(0x49907f);
            r.esi = (ply + 0x20) as u32;
            r.edi = (fr + of::T20) as u32;
            call(&mut r, 0xbe3150, ESI | EDI, NONE, &[(fr + of::T18) as u32, (fr + of::T1C) as u32], 2);
            wf(cam + 0xc, ((rf(st + 0x7cc) as f64) * (rf(fr + of::T20) as f64)) as f32);
        }
    }
    if stamp_run_out(rd(0xe9a6b0) as i32, rd(CLOCK) as i32) {
        cov(0x4990d0);
        wd(cam, 4);
    }
    if ply != 0 {
        let world = rd(0x21703d4) as usize;
        if world != 0 && eq(rd(rd(ply + 0x84e4) as usize + 0x9c), 4) {
            r.esi = world as u32;
            call(&mut r, 0x960c30, ESI, AL, &[], 0);
            if r.eax as u8 != 0 {
                cov(0x499105);
                wd(cam, 4);
            }
        }
    }
}

// --- camera_set_follow_submode (0x49D7A0..0x49D8AB, 268 bytes) ----------------------------------------
//
// Switches the follow camera's sub-mode (EDI = the new sub-mode, no stack arguments, plain `ret`) while the camera
// target entity (0x25F5BB0, any kind) exists and the sub-mode differs from the current one (0x25F5D3C). Leaving a
// sub-mode other than 1 re-aims the camera along the target's heading (0x49DF50 with the vector at +0x38) and
// clears the angle cell 0x25F5C78; leaving sub-mode 1 only raises the flag byte 0x25F5D34. Sub-modes 0..5 then pick
// the camera flag word 0x25F5C54 (0 -> 0, 1 -> 3, 2..5 -> 4) and the vector handed to 0x49DF50: 2 the negated
// heading (+0x38), 3 the heading, 4 the negated side vector (+0x20), 5 the side vector. The new sub-mode is stored
// for every value (also above 5, which changes nothing else).
// Callers (the follow update and three others) read none of eax / ecx / edx afterwards; ebx / esi / edi / ebp are
// preserved.

pub(super) unsafe fn camera_set_follow_submode_body(ent: &mut Ent, _args: *const u32) {
    cov(0x49d7a0);
    let mode = ent.edi;
    let h = rd(CAM_TARGET_HANDLE);
    if h == 0 {
        return;
    }
    let e = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
    if e == 0 || !eq(rd(e + 0x44), h) {
        return;
    }
    cov(0x49d7d3);
    let cur = rd(0x25f5d3c);
    if eq(mode, cur) {
        return;
    }
    let mut r = Rg::entry(ent);
    if !eq(cur, 1) {
        cov(0x49d7e5);
        r.eax = (e + 0x38) as u32;
        call(&mut r, 0x49df50, EAX, NONE, &[], 0);
        wf(0x25f5c78, 0.0);
    } else {
        cov(0x49d7f7);
        wb(0x25f5d34, 1);
    }
    // the vector for 0x49DF50: (source, negated); None = the mode only stores its number
    let pick = match mode {
        0 => {
            cov(0x49d80e);
            wd(0x25f5c54, 0);
            None
        }
        1 => {
            cov(0x49d823);
            wd(0x25f5c54, 3);
            None
        }
        2 => Some((e + 0x38, true)),
        3 => Some((e + 0x38, false)),
        4 => Some((e + 0x20, true)),
        5 => Some((e + 0x20, false)),
        _ => None,
    };
    if let Some((src, negate)) = pick {
        cov(0x49d838);
        let mut v = [0u32; 4];
        let mut p = src;
        if negate {
            for k in 0..3 {
                v[k] = q(rd(src + 4 * k)) ^ 0x8000_0000;
            }
            p = v.as_ptr() as usize;
        }
        std::hint::black_box(&mut v);
        r.eax = p as u32;
        call(&mut r, 0x49df50, EAX, NONE, &[], 0);
        wd(0x25f5c54, 4);
    }
    cov(0x49d887);
    wd(0x25f5d3c, mode);
}

// --- camera_push_target (0x4969B0..0x496AF6, 327 bytes) -----------------------------------------------
//
// Pushes the camera target entity `ent` (EAX) by the vector `v` (ESI, 3 f32, also an output) the way its kind wants:
// kind 1 hands the vector to 0x983540 (ecx = the entity), kind 5 (a human) runs the code below, every other kind goes
// through virtual method 0 of the entity (this = ecx, stack (v, -1), callee pops 8).
// Kind 5: the vector is first filled with the entity's velocity (0x49AA50); then, only while the world's session
// object 0x27B3C60 is in state 1, its virtual method 1 answers 0 and the flag byte 0x2526B9A is set, the vector is
// flipped (x -1) when (0xAA5F70 value of the entity) * (vector . heading at +0x38 of the entity) is below 0, and
// is scaled to the length 40 / sqrt(|v|) when its squared length is under 1600 and above 1e-6 (0x40B790).
// No stack arguments, plain `ret`; no result. ESI and EDI are preserved (the original saves edi), eax / ecx / edx
// are scratch. The x87 stack is empty on exit on every path.

pub(super) unsafe fn camera_push_target_body(ent: &mut Ent, _args: *const u32) {
    cov(0x4969b0);
    let e = ent.eax as usize;
    let v = ent.esi as usize;
    let mut r = Rg::entry(ent);
    let kind = rd(e + 0x4c);
    if eq(kind, 1) {
        cov(0x496ae7);
        r.ecx = e as u32;
        call(&mut r, 0x983540, ECX, NONE, &[v as u32], 0);
        return;
    }
    if !eq(kind, 5) {
        cov(0x4969c7);
        let method = rd(rd(e) as usize) as usize;
        r.ecx = e as u32;
        call(&mut r, method, ECX, NONE, &[v as u32, 0xffff_ffff], 2);
        return;
    }
    cov(0x4969d7);
    let handle = rd(e + 0x44);
    r.eax = handle;
    r.edx = v as u32;
    call(&mut r, 0xaa6a50, EAX | EDX, NONE, &[], 0);
    let world = rd(0x27b3c60) as usize;
    if world == 0 || !eq(rd(world + 0x3c), 1) {
        return;
    }
    cov(0x4969f8);
    let obj = rd(world + 0x90) as usize;
    let method = rd(rd(obj) as usize + 4) as usize;
    r.ecx = obj as u32;
    call(&mut r, method, ECX, EAX, &[], 0);
    if r.eax != 0 || rb(0x2526b9a) == 0 {
        return;
    }
    cov(0x496a19);
    // the entity the handle names (kind 5 only): its heading at +0x38
    r.edx = handle;
    r.ecx = handle;
    call(&mut r, 0xae28f0, ECX, EAX, &[], 0);
    let heading = (r.eax as usize).wrapping_add(0x38);
    r.ecx = handle;
    call(&mut r, 0xae28f0, ECX, EAX, &[], 0);
    let a = call(&mut r, 0xaa5f70, EAX, NONE, &[], 0).st[0];
    let dot = (((rf(heading + 4) as f64) * (rf(v + 4) as f64) + (rf(heading) as f64) * (rf(v) as f64)) + (rf(heading + 8) as f64) * (rf(v + 8) as f64)) as f32;
    if a * (dot as f64) < rdf64(0xe5bfa0) {
        cov(0x496a5d);
        // against the heading: flip the vector
        let m = rdf64(0xe5c180);
        for k in 0..3 {
            wf(v + 4 * k, ((rf(v + 4 * k) as f64) * m) as f32);
        }
    }
    cov(0x496a79);
    let (x, y, z) = (rf(v) as f64, rf(v + 4) as f64, rf(v + 8) as f64);
    let len2 = (((y * y) + (x * x)) + z * z) as f32;
    if !((len2 as f64) < rdf64(0xe5c688)) {
        return;
    }
    cov(0x496aa6);
    r.eax = v as u32;
    let l2 = call(&mut r, 0x40ea60, EAX, NONE, &[], 0).st[0];
    // (a squared length that is not above 1e-6, or not a number, leaves the vector alone)
    if !(l2 > rdf64(0xe5cf38)) {
        return;
    }
    cov(0x496aba);
    r.eax = v as u32;
    let l2 = call(&mut r, 0x40ea60, EAX, NONE, &[], 0).st[0];
    let len = call(&mut r, 0x40ea20, NONE, NONE, &[q((l2 as f32).to_bits())], 0).st[0];
    let scale = (rdf64(0xe5c328) / len) as f32;
    r.eax = v as u32;
    call(&mut r, 0x40b790, EAX, NONE, &[q(scale.to_bits())], 1);
}

// --- camera_chase_blend (0x497220..0x497777, 1,368 bytes) ----------------------------------------------
//
// Smoothing step of the vehicle-chase camera: works out where the camera's aim point should be this frame and
// writes it to `out`. EDX = &point (3 f32, the current aim point); cdecl stack arguments (out: &3 f32, then the
// direction (vx, vy, vz) and an f32 `t`, the frame time), plain `ret`. The original uses the three direction slots
// (args 1..3) as working storage and leaves the final direction there: the chase direction blended toward the camera
// offset (0x25F5B50 * 0xE9A654) by its squared length, normalised (0xBDE150) and scaled by 0xE9A654 -- or the
// target's own heading (+0x38) when the target is a kind-5 entity in state 2..4; negated when the target's seat
// check (0xAA8290 / 0xAA63A0) asks for it.
// Result: out = base + (target - base) * (1 - (1 - k)^(t / c)), where k is a smoothing constant (0x25F5CB8, or the
// target's state record +0x4D8 / +0x4DC blended in while the timer 0x25F5CBC runs) clamped to 0..1 after being
// scaled by the mode factor (0x25F5CB0 / 0x25F5CB4, taken by its magnitude); the lerp runs from the offset-relative
// point toward the camera offset (negative mode factor) or toward the direction (otherwise).
// Single caller (the vehicle chase code), which reads none of eax / ecx / edx afterwards; ebx / esi / edi / ebp are
// preserved. The x87 stack is empty on exit.

/// `base + f32(f32(target - base) * w)` per component, with the stores of the x87 code between the steps.
#[inline(always)]
fn blend3(base: [f32; 3], target: [f32; 3], w: f32) -> [f32; 3] {
    let mut o = [0f32; 3];
    for k in 0..3 {
        let d = ((target[k] as f64) - (base[k] as f64)) as f32;
        let p = ((d as f64) * (w as f64)) as f32;
        o[k] = ((base[k] as f64) + (p as f64)) as f32;
    }
    o
}

pub(super) unsafe fn camera_chase_blend_body(ent: &mut Ent, args: *const u32) {
    cov(0x497220);
    let a = args as usize;
    let out = rd(a) as usize;
    let dir = a + 4;
    let t = rf(a + 0x10);
    let inp = ent.edx as usize;
    let mut r = Rg::entry(ent);

    // the target: a kind-5 entity; `on_foot_or_seat` when its state record says 2..=4
    let tgt = kind5_entity(rd(CAM_TARGET_HANDLE));
    let mut seat = false;
    if tgt != 0 {
        cov(0x4972c6);
        seat = rd(rd(tgt + 0x84e4) as usize + 0x9c).wrapping_sub(2) <= 2;
    }

    let s = rf(0xe9a654) as f64;
    let off = [rf(0x25f5b50), rf(0x25f5b54), rf(0x25f5b58)];
    let scaled = [((off[0] as f64) * s) as f32, ((off[1] as f64) * s) as f32, ((off[2] as f64) * s) as f32];
    let mut rel = [0f32; 3];
    for k in 0..3 {
        rel[k] = ((rf(inp + 4 * k) as f64) - (rf(0x25f5b14 + 4 * k) as f64)) as f32;
    }

    let mut normalise = true;
    if seat {
        cov(0x4972a9);
        for k in 0..3 {
            wd(dir + 4 * k, rd(tgt + 0x38 + 4 * k));
        }
    } else {
        cov(0x4972e2);
        // k: how much of the offset counts; 1, or the constant 0xDD14C4 while the world is in state 1 and flagged
        let mut k = 1.0f32;
        let world = rd(0x27b3c60) as usize;
        if world != 0 && eq(rd(world + 0x3c), 1) {
            let obj = rd(world + 0x90) as usize;
            let method = rd(rd(obj) as usize + 4) as usize;
            r.ecx = obj as u32;
            call(&mut r, method, ECX, EAX, &[], 0);
            if r.eax == 0 && rb(0x2526b9a) != 0 {
                cov(0x497310);
                k = rf(0xdd14c4);
            }
        }
        let (x, y, z) = (rf(dir) as f64, rf(dir + 4) as f64, rf(dir + 8) as f64);
        let xz = ((x * x) + (z * z)) as f32;
        if (xz as f64) < (rf(0xdd0584) as f64) {
            // short direction: take the camera offset as it is
            cov(0x497345);
            for i in 0..3 {
                wf(dir + 4 * i, scaled[i]);
            }
            normalise = false;
        } else {
            let len2 = (((x * x) + (y * y)) + (z * z)) as f32;
            if (k as f64) > (len2 as f64) {
                cov(0x49738d);
                // blend the direction (weight len2) with the offset (weight k - len2)
                let rest = ((k as f64) - (len2 as f64)) as f32;
                let mut v = [0f32; 3];
                for i in 0..3 {
                    let own = ((rf(dir + 4 * i) as f64) * (len2 as f64)) as f32;
                    let off_part = ((scaled[i] as f64) * (rest as f64)) as f32;
                    v[i] = ((own as f64) + (off_part as f64)) as f32;
                }
                for i in 0..3 {
                    wf(dir + 4 * i, v[i]);
                }
            } else {
                cov(0x497416);
            }
        }
    }
    if normalise {
        cov(0x497420);
        r.esi = dir as u32;
        call(&mut r, 0xbde150, ESI, NONE, &[], 0);
        for i in 0..3 {
            wf(dir + 4 * i, ((s) * (rf(dir + 4 * i) as f64)) as f32);
        }
    }

    // does the target's seat check ask for the direction to be flipped?
    let mut flip = false;
    if tgt != 0 {
        cov(0x49744a);
        let st = rd(tgt + 0x84e4) as usize;
        let state4 = eq(rd(st + 0x9c), 4);
        r.ecx = tgt as u32;
        call(&mut r, 0xaa8290, ECX, AL, &[], 0);
        if r.eax as u8 != 0 {
            r.eax = rd(tgt + 0x44);
            let x = call(&mut r, 0xaa63a0, EAX, NONE, &[], 0).st[0];
            flip = if state4 { x < 0.0 } else { !(x == 0.0) };
        }
    }
    if seat && flip {
        cov(0x4974ad);
        for i in 0..3 {
            wd(dir + 4 * i, q(rd(dir + 4 * i)) ^ 0x8000_0000);
        }
    }

    // smoothing exponent and the mode factor
    let l14 = ((t as f64) / rdf64(0xe5cca8)) as f32;
    let mode = if (rd(0x2527c08) as i32) > 0 { rf(0x25f5cb4) } else { rf(0x25f5cb0) };
    let k_of_state;
    let timer_on = tgt != 0 && stamp_run_out(rd(0x25f5cbc) as i32, rd(CLOCK) as i32);
    if timer_on {
        cov(0x497574);
        let st = rd(tgt + 0x84e4) as usize;
        let own = if flip { rf(st + 0x4dc) } else { rf(st + 0x4d8) };
        r.eax = 0x25f5cbc;
        hcall(&mut r, 0xbdd260, EAX, EAX, &[], 0, &[], &[0.0]);
        let frac = (((r.eax as i32) as f64) / rdf64(0xe5bf98)) as f32;
        let base = rf(0x25f5cb8) as f64;
        k_of_state = ((((own as f64) - base) * (frac as f64)) + base) as f32;
    } else {
        cov(0x497525);
        k_of_state = rf(0x25f5cb8);
    }
    // k scaled by the mode factor (negative factors by their magnitude), clamped to 0..1
    let kk = if mode < 0.0 {
        cov(0x497540);
        ((-(mode as f64)) * (k_of_state as f64)) as f32
    } else {
        cov(0x497690);
        ((mode as f64) * (k_of_state as f64)) as f32
    };
    let kc = if !((kk as f64) > 0.0) {
        0.0f32
    } else if !(1.0 > (kk as f64)) {
        1.0f32
    } else {
        kk
    };
    // alpha = 1 - (1 - kc)^l14
    let base_w = ((1.0 - (kc as f64)) as f32) as f64;
    let pw = hcall(&mut r, 0xca05d0, NONE, NONE, &[], 0, &[l14 as f64, base_w], &[]).st[0] as f32;
    let alpha = (1.0 - (pw as f64)) as f32;

    let dv = [rf(dir), rf(dir + 4), rf(dir + 8)];
    let res = if mode < 0.0 {
        cov(0x4975d3);
        blend3(rel, scaled, alpha)
    } else {
        cov(0x4976be);
        // (this tail runs after the original has popped its saved registers, so its frame offsets point at the
        // offset-relative point again: the blend starts from `rel`, not from the camera offset)
        blend3(rel, dv, alpha)
    };
    for i in 0..3 {
        wf(out + 4 * i, res[i]);
    }
}

// --- camera_shake_apply (0x4A01B0..0x4A06F5, 1,350 bytes) ---------------------------------------------
//
// Camera shake: four random channels (offset 0x25F5C50, two rotations applied through 0xBE50F0 on the block 0x25F5C2C,
// and a roll value 0x25F5C68) that each re-roll a random target (-amplitude..amplitude, the rotations scaled by
// 2*pi / 0xE5C120) when their timer (0x25F6888 / 0x25F688C / 0x25F6890 / 0x25F6894) has run out and then glide from
// the previous target to the new one over the shake's duration. No arguments, plain `ret`; ebx / esi / edi / ebp are
// preserved, eax / ecx / edx are scratch for the caller (camera_effects_update). The x87 stack is empty on exit.
//
// Where the amplitude and duration come from:
//  - a shake request is running (stamp 0xE9A6BC set and not elapsed) and its start-up stamp (0xE9A6C4) has elapsed:
//    amplitude = 0xF53ABC (+ an eased fraction of the remaining time when 0xF53AB4 is set), duration 0x2526C24;
//    an elapsed request clears both stamps and ends the shake;
//  - no request: the world's flags (+0xC0): bit 29 without bit 30 gives the 0x9A0270 gain and duration 0 (the
//    channels then keep rolling every frame); otherwise, when 0x960120 says the world's human is in the state
//    that shakes, amplitude 0xE997FC and a duration of 250 ms (and the "alternate sign" mode of the roll).
// Timer words wrap by 0x6B49D200 (a negative duration wraps the other way).

/// `timer_remaining(stamp)` as the game computes it (0xBDD260), through the original function.
#[inline(always)]
unsafe fn game_timer_remaining(r: &mut Rg, stamp: usize, live: &[f64]) -> i32 {
    r.eax = stamp as u32;
    hcall(r, 0xbdd260, EAX, EAX, &[], 0, &[], live);
    r.eax as i32
}

/// A timer stamp for `now + duration`, wrapped the way 0x4A01B0 does.
#[inline(always)]
fn shake_stamp(now: u32, dur: i32) -> u32 {
    let v = now.wrapping_add(dur as u32);
    if dur < 0 {
        if (v as i32) < 0 { v.wrapping_add(0x6b49_d200) } else { v }
    } else if (v as i32) > 0x6b49_d200 {
        v.wrapping_sub(0x6b49_d200)
    } else {
        v
    }
}

/// `lo + (hi - lo) * r` as the x87 code evaluates it, stored as an f32.
#[inline(always)]
fn shake_pick(lo: f32, hi: f32, r: f64) -> f32 {
    ((lo as f64) + (((hi as f64) - (lo as f64)) * r)) as f32
}

/// One channel: re-roll when its timer is not running. Returns the progress fraction `u` as an f32 and leaves the
/// channel's current / previous values in memory. `lo` / `hi` are the bounds of the random target.
unsafe fn shake_channel(r: &mut Rg, live: &[f64], pc: usize, now: u32, dur: i32, timer: usize, cur: usize, prev: usize, lo: f32, hi: f32) -> f32 {
    if !stamp_run_out(rd(timer) as i32, now as i32) {
        cov(pc);
        wd(prev, q(rd(cur)));
        r.eax = 0xec30f8;
        let roll = call(r, 0xbdce30, EAX, NONE, &[], 0).st[0];
        wf(cur, shake_pick(lo, hi, roll));
        wd(timer, shake_stamp(now, dur));
    }
    if dur != 0 {
        let rem = game_timer_remaining(r, timer, live);
        (1.0 - (rem as f64) / (dur as f64)) as f32
    } else {
        1.0
    }
}

/// `prev + (cur - prev) * u`
#[inline(always)]
unsafe fn shake_glide(cur: usize, prev: usize, u: f32) -> f32 {
    ((rf(prev) as f64) + (((rf(cur) as f64) - (rf(prev) as f64)) * (u as f64))) as f32
}

pub(super) unsafe fn camera_shake_apply_body(ent: &mut Ent, _args: *const u32) {
    cov(0x4a01b0);
    let mut r = Rg::entry(ent);
    wf(0x25f5c50, 0.0);
    wf(0x25f5c68, 0.0);
    let now = rd(CLOCK);
    let req = rd(0xe9a6bc) as i32;
    let mut alternate = false;
    let mut amp: f32;
    let dur: i32;
    if req >= 0 {
        // a request is set: an elapsed one ends the shake
        let elapsed = if req > now as i32 { req.wrapping_sub(now as i32) >= STAMP_WINDOW } else { (now as i32).wrapping_sub(req) < STAMP_WINDOW };
        if elapsed {
            cov(0x4a0219);
            wd(0xe9a6bc, 0xffff_ffff);
            wd(0xe9a6c4, 0xffff_ffff);
            return;
        }
        // the start-up stamp must have elapsed too
        let start = rd(0xe9a6c4) as i32;
        let start_elapsed = start >= 0 && if start > now as i32 { start.wrapping_sub(now as i32) >= STAMP_WINDOW } else { (now as i32).wrapping_sub(start) < STAMP_WINDOW };
        if !start_elapsed {
            cov(0x4a06ef);
            return;
        }
        cov(0x4a0243);
        let div = rd(0xf53ab4) as i32;
        if div != 0 {
            cov(0x4a024c);
            let rem = game_timer_remaining(&mut r, 0xe9a6bc, &[]);
            let ratio = ((rem as f64) / (div as f64)) as f32;
            let eased = call(&mut r, 0xbf0f50, NONE, NONE, &[q(ratio.to_bits()), q(1.0f32.to_bits()), q(rd(0xdd1c8c))], 0).st[0];
            amp = ((eased * (rf(0xf53ab8) as f64)) + (rf(0xf53abc) as f64)) as f32;
        } else {
            amp = f32::from_bits(q(rd(0xf53abc)));
        }
        dur = rd(0x2526c24) as i32;
    } else {
        let world = rd(0x21703d4) as usize;
        let flags = rd(world + 0xc0);
        if (flags >> 29) & 1 != 0 && (flags >> 30) & 1 == 0 {
            cov(0x4a02ca);
            r.ecx = world as u32;
            amp = call(&mut r, 0x9a0270, ECX, NONE, &[], 0).st[0] as f32;
            dur = 0;
        } else {
            cov(0x4a02d3);
            r.esi = world as u32;
            call(&mut r, 0x960120, ESI, AL, &[], 0);
            if r.eax as u8 == 0 {
                return;
            }
            cov(0x4a02e2);
            amp = f32::from_bits(q(rd(0xe997fc)));
            dur = 250;
            alternate = true;
        }
    }

    cov(0x4a02f1);
    // channel 1: the offset
    let u = shake_channel(&mut r, &[amp as f64], 0x4a0321, now, dur, 0x25f6888, 0x2526c28, 0x2526c38, -amp, amp);
    wf(0x25f5c50, shake_glide(0x2526c28, 0x2526c38, u));
    // channel 2: a rotation of the block 0x25F5C2C
    cov(0x4a03a3);
    let u = shake_channel(&mut r, &[], 0x4a040d, now, dur, 0x25f688c, 0x2526c2c, 0x2526c3c, -amp, amp);
    let v = shake_glide(0x2526c2c, 0x2526c3c, u);
    call(&mut r, 0xbe50f0, NONE, NONE, &[0x25f5c2c, q(v.to_bits())], 2);
    // channel 3: a second rotation, scaled by 2*pi
    cov(0x4a04ae);
    let two_pi = rdf64(0xe5bf70);
    let (lo3, hi3) = ((-(amp as f64) * two_pi) as f32, ((amp as f64) * two_pi) as f32);
    let u = shake_channel(&mut r, &[], 0x4a04d6, now, dur, 0x25f6890, 0x2526c30, 0x2526c40, lo3, hi3);
    let v = shake_glide(0x2526c30, 0x2526c40, u);
    call(&mut r, 0xbe50f0, NONE, NONE, &[0x25f5c2c, q(v.to_bits())], 2);
    // channel 4: the roll; the amplitude is capped, and in the alternate mode every new target flips sign
    cov(0x4a059e);
    let running = stamp_run_out(rd(0x25f6894) as i32, now as i32);
    if !running {
        cov(0x4a05ca);
        if (amp as f64) > rdf64(0xe5c2c0) {
            amp = f32::from_bits(q(rd(0xe8b048)));
        }
        let old = rf(0x2526c34);
        wd(0x2526c44, q(rd(0x2526c34)));
        let positive = (old as f64) > 0.0;
        let c = rdf64(0xe5c120);
        let (lo4, hi4) = ((-(amp as f64) * c) as f32, ((amp as f64) * c) as f32);
        r.eax = 0xec30f8;
        let roll = call(&mut r, 0xbdce30, EAX, NONE, &[], 0).st[0];
        let mut new = shake_pick(lo4, hi4, roll);
        wf(0x2526c34, new);
        if alternate {
            cov(0x4a0647);
            if new > 0.0 && positive || new < 0.0 && !positive {
                new = -new;
                wf(0x2526c34, new);
            }
        }
        wd(0x25f6894, shake_stamp(now, dur));
    }
    cov(0x4a06ab);
    let u = if dur != 0 {
        let rem = game_timer_remaining(&mut r, 0x25f6894, &[]);
        (1.0 - (rem as f64) / (dur as f64)) as f32
    } else {
        1.0
    };
    wf(0x25f5c68, shake_glide(0x2526c34, 0x2526c44, u));
}

// --- camera_script_build_track (0x49F320..0x49FC95, 2,422 bytes) --------------------------------------
//
// Bakes a scripted camera track. Inputs: EAX = n (number of key nodes), ESI = the track block to fill (0x4B0 bytes,
// cleared first), cdecl stack arguments (nodes: &[*node; n], durations: &[f32; n-1]), plain `ret`, no result. A node
// holds three vectors: position (+0x1C), aim point (+0x40) and a third channel (+0x28). Each channel gets a start
// and an end control point extrapolated from its two outer nodes (p0 + (p0 - p1) * 0.1 and the same at the end), the
// three control arrays are handed to the spline evaluator 0xC022C0, and the track is sampled once per `step`:
//   m = min(ceil(total duration), 30) samples (stored at +0x00, the step total / (m-1) at +0x04), sample j of the
//   position / aim / third channel at +0x14 / +0x194 / +0x314 + 12 j, found by walking the segment durations; the
//   last sample is the last node itself. Start / end points are extrapolated again from the samples (at +0x08 and
//   after the last sample), and three {pointer, count, 0, ...} headers (at +0x488, +0x494, +0x4A0) describe the
//   channels.
// The control arrays live in the function's own frame, in one run (position at +0x4C, aim at +0x1CC, third at +0x34C,
// each with the start point 12 bytes before it), exactly as in the original, so more than 29 nodes overrun into the
// next array there too. Deviation: more than 30 nodes would write the return address (the original crashes), fewer than -5 would run
// below the frame; the port returns without doing anything in both cases.
// ebx / esi / edi / ebp are preserved, eax / ecx / edx are scratch; the x87 stack is empty on exit.

/// `base + (base - other) * c` per component with the f32 stores of the x87 code into the frame cells (the
/// difference at `dc`, the product at `tc`, the sum at `pc`); returns the sum's bits.
#[inline(always)]
unsafe fn track_extrap(cell: &dyn Fn(usize) -> usize, base: usize, other: usize, c: f64, dc: usize, tc: usize, pc: usize) -> [u32; 3] {
    let mut res = [0u32; 3];
    for k in 0..3 {
        let d = ((rf(base + 4 * k) as f64) - (rf(other + 4 * k) as f64)) as f32;
        wf(cell(dc + 4 * k), d);
        let t = ((d as f64) * c) as f32;
        wf(cell(tc + 4 * k), t);
        let p = ((rf(base + 4 * k) as f64) + (t as f64)) as f32;
        wf(cell(pc + 4 * k), p);
        res[k] = p.to_bits();
    }
    res
}

pub(super) unsafe fn camera_script_build_track_body(ent: &mut Ent, args: *const u32) {
    cov(0x49f320);
    let a = args as usize;
    let nodes = rd(a) as usize;
    let durs = rd(a + 4) as usize;
    let n = ent.eax as i32;
    let out = ent.esi as usize;
    let mut r = Rg::entry(ent);
    if !(-5..=30).contains(&n) {
        return;
    }
    // the frame: the original's 0x4B4-byte local area (cell(x) = the original's [esp + x] after its prologue)
    let mut buf = core::mem::MaybeUninit::<[u32; 301]>::uninit();
    let fr = local_frame(&mut buf, 0) as usize;
    let cell = |x: usize| fr + x - 0xc;

    call(&mut r, 0xca0bc0, NONE, NONE, &[out as u32, 0, 0x4b0], 0);
    wf(cell(0x24), 0.0);

    // total duration of the n-1 segments, summed in order with an f32 store after each addition
    let count = n.wrapping_sub(1);
    let mut total = 0.0f32;
    if count >= 4 {
        cov(0x49f359);
    }
    let mut kk = 0;
    while kk < count {
        total = ((rf(durs + 4 * kk as usize) as f64) + (total as f64)) as f32;
        kk += 1;
    }
    wf(cell(0x24), total);

    // start control points: first node + (first - second) * 0.1 (the aim point and the third channel likewise)
    cov(0x49f3b4);
    let c = rdf64(0xdd50e8);
    let n0 = rd(nodes) as usize;
    let n1 = rd(nodes + 4) as usize;
    let pre_a = track_extrap(&cell, n0 + 0x1c, n1 + 0x1c, c, 0x18, 0x0c, 0x18);
    let pre_b = track_extrap(&cell, n0 + 0x40, n1 + 0x40, c, 0x0c, 0x18, 0x0c);
    let pre_c = track_extrap(&cell, n0 + 0x28, n1 + 0x28, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(cell(0x40 + 4 * k), pre_a[k]);
        wd(cell(0x1c0 + 4 * k), pre_b[k]);
        wd(cell(0x340 + 4 * k), pre_c[k]);
    }
    // the nodes' vectors, in order
    let mut i = 0i32;
    while i < n {
        cov(0x49f53b);
        let nd = rd(nodes + 4 * i as usize) as usize;
        for k in 0..3 {
            wd(cell(0x4c + 12 * i as usize + 4 * k), rd(nd + 0x1c + 4 * k));
            wd(cell(0x1cc + 12 * i as usize + 4 * k), rd(nd + 0x40 + 4 * k));
            wd(cell(0x34c + 12 * i as usize + 4 * k), rd(nd + 0x28 + 4 * k));
        }
        i += 1;
    }
    // end control points: last + (last - one before) * 0.1
    cov(0x49f59c);
    let ne = rd(nodes.wrapping_add((n.wrapping_sub(1) as u32).wrapping_mul(4) as usize)) as usize;
    let np = rd(nodes.wrapping_add((n.wrapping_sub(2) as u32).wrapping_mul(4) as usize)) as usize;
    let n12 = (n as u32).wrapping_mul(12) as usize;
    let post_a = track_extrap(&cell, ne + 0x1c, np + 0x1c, c, 0x0c, 0x18, 0x0c);
    let post_b = track_extrap(&cell, ne + 0x40, np + 0x40, c, 0x0c, 0x18, 0x0c);
    let post_c = track_extrap(&cell, ne + 0x28, np + 0x28, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(cell(0x4c).wrapping_add(n12 + 4 * k), post_a[k]);
        wd(cell(0x1cc).wrapping_add(n12 + 4 * k), post_b[k]);
        wd(cell(0x34c).wrapping_add(n12 + 4 * k), post_c[k]);
    }

    // the three control-array descriptors {pointer, count}; the cells between them keep what the original left
    let np2 = (n as u32).wrapping_add(2);
    wd(cell(0xc), cell(0x40) as u32);
    wf(cell(0x14), 0.0);
    wf(cell(0x20), 0.0);
    wf(cell(0x3c), 0.0);
    wd(cell(0x10), np2);
    wd(cell(0x1c), np2);
    wd(cell(0x18), cell(0x1c0) as u32);
    wd(cell(0x38), np2);
    wd(cell(0x34), cell(0x340) as u32);

    // m = min(ftol(ceil(total)), 30) -- the original evaluates the ceiling once more when it is under 30
    // (the library routines run with the registers the original has here: some of them read their scratch registers)
    r.eax = np2;
    r.ecx = cell(0x1c0) as u32;
    r.edx = cell(0x340) as u32;
    r.ebx = n12 as u32;
    r.esi = out as u32;
    r.edi = n as u32;
    let up = call(&mut r, 0x4912d0, EAX | ECX | EDX | EBX | ESI | EDI, NONE, &[q(total.to_bits())], 0).st[0];
    hcall(&mut r, 0xda0140, NONE, EAX, &[], 0, &[up], &[]);
    let mut m = r.eax as i32;
    if m >= 0x1e {
        cov(0x49f78c);
        m = 0x1e;
    } else {
        cov(0x49f775);
        let up = call(&mut r, 0x4912d0, NONE, NONE, &[q(total.to_bits())], 0).st[0];
        hcall(&mut r, 0xda0140, NONE, EAX, &[], 0, &[up], &[]);
        m = r.eax as i32;
    }
    cov(0x49f791);
    wd(cell(0x28), (m - 1) as u32);
    let step = ((total as f64) / ((m - 1) as f64)) as f32;
    wd(out, m as u32);
    wd(cell(0x28), 0);
    wf(out + 4, step);
    wf(cell(0x24), 0.0);

    // sample the three splines once per step, walking the segment durations
    let mut e = 1i32;
    if m > 1 {
        cov(0x49f7c0);
        let mut acc = 0.0f32;
        let mut seg = 0i32;
        let mut dst = out + 0x194;
        loop {
            cov(0x49f7d7);
            let dk = rf(durs + 4 * seg as usize);
            let t0 = ((((acc as f64) / (dk as f64)) + (seg as f64)) as f32) as f64;
            let t = (t0 + rdf64(0xe5bf40)) as f32;
            wd(cell(0x28), seg as u32);
            wf(cell(0x30), t);
            r.eax = cell(0xc) as u32;
            r.edx = (dst - 0x180) as u32;
            call(&mut r, 0xc022c0, EAX | EDX, NONE, &[q(t.to_bits())], 1);
            r.eax = cell(0x18) as u32;
            r.edx = dst as u32;
            call(&mut r, 0xc022c0, EAX | EDX, NONE, &[q(t.to_bits())], 1);
            r.eax = cell(0x34) as u32;
            r.edx = (dst + 0x180) as u32;
            call(&mut r, 0xc022c0, EAX | EDX, NONE, &[q(t.to_bits())], 1);
            acc = ((rf(out + 4) as f64) + (acc as f64)) as f32;
            while (rf(durs + 4 * seg as usize) as f64) < (acc as f64) {
                cov(0x49f850);
                acc = ((acc as f64) - (rf(durs + 4 * seg as usize) as f64)) as f32;
                seg += 1;
            }
            wf(cell(0x24), acc);
            dst += 12;
            e += 1;
            if !(e < (rd(out) as i32)) {
                break;
            }
        }
    }

    // the last sample is the last node itself
    cov(0x49f88e);
    let p = out + 12 * e as usize;
    for k in 0..3 {
        wd(p + 8 + 4 * k, rd(cell(0x40 + n12 + 4 * k)));
        wd(p + 0x188 + 4 * k, rd(cell(0x1c0 + n12 + 4 * k)));
        wd(p + 0x308 + 4 * k, rd(cell(0x340 + n12 + 4 * k)));
    }
    // start points again, from the samples (position, aim, third channel)
    let pre_a = track_extrap(&cell, out + 0x14, out + 0x20, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(out + 8 + 4 * k, pre_a[k]);
    }
    let pre_b = track_extrap(&cell, out + 0x194, out + 0x1a0, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(out + 0x188 + 4 * k, pre_b[k]);
    }
    let pre_c = track_extrap(&cell, out + 0x314, out + 0x320, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(out + 0x308 + 4 * k, pre_c[k]);
    }
    // end points from the last two samples
    let post_a = track_extrap(&cell, p + 8, p - 4, c, 0x0c, 0x18, 0x0c);
    let post_b = track_extrap(&cell, p + 0x188, p + 0x17c, c, 0x0c, 0x18, 0x0c);
    let post_c = track_extrap(&cell, p + 0x308, p + 0x2fc, c, 0x0c, 0x18, 0x0c);
    for k in 0..3 {
        wd(p + 0x14 + 4 * k, post_a[k]);
        wd(p + 0x194 + 4 * k, post_b[k]);
        wd(p + 0x314 + 4 * k, post_c[k]);
    }
    // the headers {pointer, count + 2, 0, ...}
    let cnt = rd(out).wrapping_add(2);
    wd(out + 0x488, (out + 8) as u32);
    wd(out + 0x48c, cnt);
    wf(out + 0x490, 0.0);
    wd(out + 0x494, (out + 0x188) as u32);
    wd(out + 0x498, rd(out).wrapping_add(2));
    wf(out + 0x49c, 0.0);
    wd(out + 0x4a0, (out + 0x308) as u32);
    wd(out + 0x4a4, rd(out).wrapping_add(2));
    wf(out + 0x4a8, 0.0);
    wf(out + 0x4ac, 0.0);
}

// --- camera_follow_compute_desired_position (0x499B50..0x49A07C, 1,325 bytes) -------------------------
//
// Works out where the follow camera wants to be: 0xE9A664 (3 f32). No arguments, no result, plain `ret`; ebx / esi /
// edi / ebp preserved, eax / ecx / edx scratch for the callers (camera_cut, the follow-state resets 0x49A260 /
// 0x49A330 / 0x49A4C0, 0x49D090); the x87 stack is empty on exit.
//  1. The offset vector is the active params 0xE9A638 (x, y, z) with y scaled by 0x27A6A4C when that is positive;
//     then, from the *params* again, y scaled by 0x27A6A50 when that is positive (else the vector stays as it is).
//  2. With the camera target entity (0xE9A5B8, any kind) the offset is turned into the target's frame (flag 8 of the
//     camera flags 0xE9A684 skips that; flag 0x10 uses the target's matrix at +0x20, else a yaw-only matrix built from
//     its heading) and added to the target position from 0x4968E0.
//  3. Flag 1 (target present): a ray cast from the target toward the desired point; a hit moves the point to the hit
//     and 0.5 down (0xE5C108).
//  4. When the target is a kind-1 entity in a state that wants it, flag 2: a ray cast straight down (distance
//     0xE999D4 * 0x2526C10) and the desired point goes to the last hit's position; without flag 2 the point just
//     drops by that distance.
//  5. Flag 4: a ray cast straight up (the hits of kind 0x14 move the point to the hit + 2 pi); the hit list is
//     released through the heap's free (0xD2B7D0) there; the other casts release it with 0x4080B0.
// The ray cast request (0x6FA330: ecx = request, stack (hit list, 1)) and the hit list {array, count, heap id} are
// built in the function's frame at the same addresses as in the original, field by field.

const RAY_LIST: usize = 12;
const RAY_REQ: usize = 44;

pub(super) unsafe fn camera_follow_compute_desired_body(ent: &mut Ent, _args: *const u32) {
    cov(0x499b50);
    const DES: usize = 0xe9a664;
    let mut r = Rg::entry(ent);
    // the frame: 0xA0 bytes of locals; cell(x) = the original's [F + x] (F = esp before its last `push edi`)
    let mut buf = core::mem::MaybeUninit::<[u32; 40]>::uninit();
    let fr = local_frame(&mut buf, 8) as usize;
    let cell = |x: usize| fr + x - 0xc;

    // 1. the offset vector from the params, y scaled twice (each time from the original y)
    let (px, py, pz) = (q(rd(0xe9a638)), rf(0xe9a63c), q(rd(0xe9a640)));
    let scale_y = |s: f32| ((s as f64) * (py as f64)) as f32;
    let a = rf(0x27a6a4c);
    let mut src = 0xe9a638usize;
    if (a as f64) > 0.0 {
        cov(0x499b83);
        wd(cell(28), px);
        wf(cell(32), scale_y(a));
        wd(cell(36), pz);
        src = cell(28);
    }
    for k in 0..3 {
        wd(DES + 4 * k, rd(src + 4 * k));
    }
    let b = rf(0x27a6a50);
    if (b as f64) > 0.0 {
        cov(0x499bd0);
        wd(cell(28), px);
        wf(cell(32), scale_y(b));
        wd(cell(36), pz);
        src = cell(28);
    } else {
        src = DES;
    }
    for k in 0..3 {
        wd(DES + 4 * k, rd(src + 4 * k));
    }

    // 2. into the target's frame, plus the target's position
    let h = rd(0xe9a5b8);
    let mut tgt = 0usize;
    if h != 0 {
        let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
        if slot != 0 && eq(rd(slot + 0x44), h) {
            cov(0x499c24);
            tgt = slot;
            let fl = rb(0xe9a684);
            if fl & 8 == 0 {
                let mat;
                if fl & 0x10 != 0 {
                    cov(0x499c51);
                    mat = slot + 0x20;
                } else {
                    cov(0x499c56);
                    r.ecx = (slot + 0x20) as u32;
                    call(&mut r, 0xbe3380, ECX, XMM0, &[], 0);
                    wd(cell(24), r.xmm0);
                    let yaw = q(r.xmm0);
                    call(&mut r, 0xbe2d90, NONE, NONE, &[cell(136) as u32, 0, 0, yaw], 4);
                    mat = cell(136);
                }
                r.eax = mat as u32;
                r.ecx = DES as u32;
                call(&mut r, 0xbde570, EAX | ECX, NONE, &[], 0);
            }
            cov(0x499c9c);
            r.esi = rd(0xe9a5b8);
            call(&mut r, 0x4968e0, ESI, NONE, &[cell(28) as u32], 0);
            for k in 0..3 {
                wf(DES + 4 * k, ((rf(cell(28 + 4 * k)) as f64) + (rf(DES + 4 * k) as f64)) as f32);
            }
        }
    }
    wd(cell(40), tgt as u32);

    // the entity ray 2 / 3 look at: only a kind-1 entity
    let mut veh = 0usize;
    if h != 0 {
        let slot = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
        if slot != 0 && eq(rd(slot + 0x44), h) && eq(rd(slot + 0x4c), 1) {
            veh = slot;
        }
    }

    // a ray cast request in the frame: start / end points (raw dwords), the float at +68, the flag word at +120
    let query = |r: &mut Rg, start: [u32; 3], end: [u32; 3], aux: u32, flags: u32| -> bool {
        wd(cell(72), 0);
        wd(cell(112), 0);
        wd(cell(116), 0);
        wd(cell(124), 0);
        wd(cell(128), 0);
        wd(cell(RAY_LIST), 0);
        wd(cell(RAY_LIST + 4), 0);
        wd(cell(RAY_LIST + 8), 0x8000_0000);
        for k in 0..3 {
            wd(cell(RAY_REQ + 4 * k), start[k]);
            wd(cell(RAY_REQ + 12 + 4 * k), end[k]);
        }
        wd(cell(68), aux);
        wd(cell(120), flags);
        r.ecx = cell(RAY_REQ) as u32;
        call(r, 0x6fa330, ECX, AL, &[cell(RAY_LIST) as u32, 1], 0);
        r.eax as u8 != 0
    };
    let release = |r: &mut Rg| {
        r.ecx = cell(RAY_LIST) as u32;
        call(r, 0x4080b0, ECX, NONE, &[], 0);
    };

    // 3. flag 1: from the target toward the desired point
    if rb(0xe9a684) & 1 != 0 && tgt != 0 {
        cov(0x499d31);
        r.esi = rd(0xe9a5b8);
        call(&mut r, 0x4968e0, ESI, NONE, &[cell(28) as u32], 0);
        let up = ((rf(cell(32)) as f64) + rdf64(0xe5c108)) as f32;
        let start = [rd(cell(28)), up.to_bits(), rd(cell(36))];
        let end = [rd(DES), rd(DES + 4), rd(DES + 8)];
        if query(&mut r, start, end, q(rd(0xdd1944)), 0x5d) {
            cov(0x499dd7);
            let hit = rd(cell(RAY_LIST)) as usize;
            wd(DES, rd(hit));
            wd(DES + 8, rd(hit + 8));
            wf(DES + 4, ((rf(hit + 4) as f64) - rdf64(0xe5c108)) as f32);
        }
        release(&mut r);
    }

    // 4. a kind-1 target in a state that wants it: drop the point onto what is below it
    let mut wants = false;
    if veh != 0 {
        cov(0x499e23);
        wants = true;
        if (rd(veh + 0xc0) >> 7) & 1 == 0 {
            let ctx = rd(veh + 0x5e0) as usize;
            r.eax = rd(ctx + 0x40);
            call(&mut r, 0x994cc0, EAX, AL, &[], 0);
            if r.eax as u8 == 0 && rd(ctx + 0x14) != 0x170 {
                r.eax = veh as u32;
                call(&mut r, 0x994e70, EAX, AL, &[], 0);
                wants = r.eax as u8 != 0;
            }
        }
    }
    if wants {
        let drop = ((rf(0xe999d4) as f64) * (rf(0x2526c10) as f64)) as f32;
        if rb(0xe9a684) & 2 != 0 {
            cov(0x499e5b);
            let end_y = ((rf(DES + 4) as f64) - (drop as f64)) as f32;
            let start = [rd(DES), rd(DES + 4), rd(DES + 8)];
            let end = [rd(DES), end_y.to_bits(), rd(DES + 8)];
            if query(&mut r, start, end, 0, 0x5d) {
                cov(0x499ef6);
                let n = rd(cell(RAY_LIST + 4));
                let hits = rd(cell(RAY_LIST)) as usize;
                let last = hits.wrapping_add((n.wrapping_mul(60)).wrapping_sub(0x30) as usize);
                for k in 0..3 {
                    wd(DES + 4 * k, rd(last + 4 * k));
                }
            }
            release(&mut r);
        } else {
            cov(0x499f39);
            wf(DES + 4, ((rf(DES + 4) as f64) - (drop as f64)) as f32);
        }
    }

    // 5. flag 4: a ray cast straight up
    if rb(0xe9a684) & 4 != 0 {
        cov(0x499f66);
        let y = rf(DES + 4);
        let start = [rd(DES), (((y as f64) - rdf64(0xe5bf70)) as f32).to_bits(), rd(DES + 8)];
        let end = [rd(DES), (((y as f64) + rdf64(0xe5c1b8)) as f32).to_bits(), rd(DES + 8)];
        let ok = query(&mut r, start, end, 0, 0x4d);
        let hits = rd(cell(RAY_LIST)) as usize;
        if ok {
            let n = rd(cell(RAY_LIST + 4)) as i32;
            if n > 0 {
                cov(0x49a000);
                let c = rdf64(0xe5bf70);
                for i in 0..n as usize {
                    let p = hits.wrapping_add(0x3c * i);
                    if eq(rd(p + 0x38), 0x14) {
                        cov(0x49a012);
                        wd(DES, rd(p));
                        wd(DES + 4, rd(p + 4));
                        wd(DES + 8, rd(p + 8));
                        wf(DES + 4, ((rf(DES + 4) as f64) + c) as f32);
                    }
                }
            }
        }
        // the list is released by the heap, not by 0x4080B0
        let heap = rd(cell(RAY_LIST + 8));
        if (heap as i32) >= 0 {
            cov(0x49a04c);
            let tls = hcall(&mut r, rd(0xdb807c) as usize, NONE, EAX, &[rd(0x31bf2bc)], 1, &[], &[]);
            let _ = tls;
            r.ecx = r.eax;
            let size = (heap & 0x3fff_ffff).wrapping_mul(60);
            call(&mut r, 0xd2b7d0, ECX, NONE, &[hits as u32, size, 0x17], 3);
        }
    }
}

// --- camera_update (0x4932F0..0x49380D, 1,310 bytes) ---------------------------------------------------
//
// The per-frame camera update (sim_frame calls it twice a frame). No arguments, no result, plain `ret`; ebx / esi / edi /
// ebp preserved, eax / ecx / edx scratch for the callers; the x87 stack is empty on exit. It does nothing without a
// camera target entity (0x25F5BB0).
//  1. Keeps the previous camera position / orientation (0x25F5B2C.., 0x25F5B80), takes the frame time (0xE8437C, or
//     the time scale of 0x6FF860 for a human / a flagged vehicle target) and, when it is positive, runs the update of
//     the active camera mode (table 0xDD5258, 5 dwords per mode, slot 0 takes dt; mode 0 is camera_follow_update) and,
//     with flag 2, the blend step 0x4A33A0; then the effects (0x4A0EA0).
//  2. Flag 4 of 0xE9A684: a camera cut (0x492DF0), and a long cut raises the 0x2529620 flag.
//  3. Moves the current position / orientation from the target block (0x25F5B20..), eases the field of view toward its
//     target (flag 8: by 0xE84384 * 0xE5C130, clamped to 0..1).
//  4. With 0xE8825F set: a collision query (0x6FA820) around the camera; every vehicle hit that is not allowed to stay
//     in view is hidden through 0x492D90 (the decision reads the camera attach state 0xEBCE9C / 0xEBCEA0 and the
//     vehicle's driver); the hit list is released through the heap (0xD2B7D0).
//  5. Publishes the target's position / matrix (0x25F5BB4.., 0x25F5BC0..) and the frame time (0x25F5BE4), measures how
//     far the camera moved (0xBDE980 + the pending jump 0x2526BE8, flag 0xE996E5 when it is small), and refreshes the
//     region id (0x4A9BE0 of the camera position); the one-shot flag 0x2526B5A is cleared.
// The collision query request lives in the frame at the same offsets as the original's.

pub(super) unsafe fn camera_update_body(ent: &mut Ent, _args: *const u32) {
    cov(0x4932f0);
    let mut r = Rg::entry(ent);
    // the frame: 0x8C bytes of locals; cell(x) = the original's [F + x] (F = esp after its four pushes)
    let mut buf = core::mem::MaybeUninit::<[u32; 35]>::uninit();
    let fr = local_frame(&mut buf, 0) as usize;
    let cell = |x: usize| fr + x - 0x10;

    let h = rd(CAM_TARGET_HANDLE);
    if h == 0 {
        return;
    }
    let tgt = rd(0x2149c64 + (((h & 0xfff) << 4) as usize)) as usize;
    wd(cell(24), tgt as u32);
    if tgt == 0 || !eq(rd(tgt + 0x44), h) {
        return;
    }
    cov(0x49332f);
    call(&mut r, 0x4a3540, NONE, NONE, &[], 0);
    let mut dt = f32::from_bits(q(rd(0xe8437c)));
    wf(cell(20), dt);
    if eq(rd(0xe96c00), 0xd) && eq(rd(0x213c444), 1) {
        let a = rd(0x213b4f8) as usize;
        if a != 0 {
            let b = rd(a + 4) as usize;
            if b != 0 {
                let c = rd(b + 4) as usize;
                if c != 0 {
                    let d = rd(c + 4) as usize;
                    if eq(rd(d), 2) {
                        cov(0x49336f);
                        call(&mut r, 0x8aaa80, NONE, NONE, &[], 0);
                    }
                }
            }
        }
    }
    // the previous position / orientation
    wd(0x25f5b30, rd(0x25f5b18));
    wd(0x25f5b2c, rd(0x25f5b14));
    wd(0x25f5b34, rd(0x25f5b1c));
    for k in 0..9 {
        wd(0x25f5b80 + 4 * k, rd(0x25f5b38 + 4 * k));
    }
    wd(0x25f5bac, q(rd(0x25f5ba4)));

    // the frame time: a human / a flagged vehicle target runs on its own time scale
    let h2 = rd(CAM_TARGET_HANDLE);
    let mut scaled = false;
    if h2 != 0 {
        let slot = rd(0x2149c64 + (((h2 & 0xfff) << 4) as usize)) as usize;
        if slot != 0 && eq(rd(slot + 0x44), h2) && rb(0x25f5be8) & 2 == 0 {
            let kind = rd(slot + 0x4c);
            if eq(kind, 5) || (eq(kind, 1) && rb(slot + 0x595) != 0) {
                cov(0x4933f4);
                dt = call(&mut r, 0x6ff860, NONE, NONE, &[], 0).st[0] as f32;
                scaled = true;
            }
        }
    }
    if !scaled {
        dt = f32::from_bits(q(dt.to_bits()));
    }
    wf(cell(20), dt);
    for k in 0..3 {
        wd(cell(40 + 4 * k), q(rd(0x25f5b14 + 4 * k)));
    }
    if (dt as f64) > 0.0 {
        cov(0x493432);
        let mode_fn = rd(0xdd5258 + 20 * rd(0x25f5ae8) as usize) as usize;
        if mode_fn != 0 {
            cov(0x493445);
            call(&mut r, mode_fn, NONE, NONE, &[q(dt.to_bits())], 0);
        }
        if rb(0x25f5be8) & 2 != 0 {
            cov(0x49345b);
            r.eax = 0x25f5b5c;
            r.ecx = 0x25f5b20;
            r.edi = 0x25f5ba8;
            call(&mut r, 0x4a33a0, EAX | ECX | EDI, NONE, &[0x25f5b20, 0x25f5ba8, 0x25f5b5c, q(dt.to_bits())], 0);
        }
    }
    cov(0x493489);
    call(&mut r, 0x4a0ea0, NONE, NONE, &[], 0);

    // flag 4: a camera cut
    if rb(0x25f5be8) & 4 != 0 {
        cov(0x493497);
        for k in 0..3 {
            wd(cell(52 + 4 * k), q(rd(0x25f5b14 + 4 * k)));
        }
        call(&mut r, 0x492df0, NONE, NONE, &[], 0);
        wd(0x25f5be8, rd(0x25f5be8) & 0xffff_fffb);
        r.eax = cell(52) as u32;
        r.ecx = 0x25f5b14;
        let jump = call(&mut r, 0xbdea30, EAX | ECX, NONE, &[], 0).st[0];
        if jump > rdf64(0xe5c540) && rb(0x2528b48) == 0 {
            cov(0x4934e6);
            wb(0x2529620, 1);
        }
    }
    // the position / orientation from the target block
    wd(0x25f5b18, rd(0x25f5b24));
    wd(0x25f5b14, rd(0x25f5b20));
    wd(0x25f5b1c, rd(0x25f5b28));
    for k in 0..9 {
        wd(0x25f5b38 + 4 * k, rd(0x25f5b5c + 4 * k));
    }
    // the field of view eases toward its target (flag 8) or takes it
    let fov_target = rf(0x25f5ba8);
    let fov = if rb(0x25f5be8) & 8 != 0 {
        cov(0x49352c);
        let mut k = ((rf(0xe84384) as f64) * rdf64(0xe5c130)) as f32;
        wf(cell(16), k);
        if !((k as f64) > 0.0) {
            cov(0x49355e);
            k = 0.0;
        } else if !(1.0 > (k as f64)) {
            cov(0x493560);
            k = 1.0;
        }
        wf(cell(16), k);
        ((fov_target as f64 - rf(0x25f5ba4) as f64) * (k as f64) + rf(0x25f5ba4) as f64) as f32
    } else {
        fov_target
    };
    wf(0x25f5ba4, fov);

    // the collision query around the camera
    if rb(0xe8825f) != 0 {
        cov(0x493596);
        let (cx, cy, cz) = (rd(0x25f5b20), rd(0x25f5b24), rd(0x25f5b28));
        wd(cell(88), q(rd(0xe5daa0)));
        wd(cell(72), cz);
        wd(cell(84), cz);
        for off in [92, 132, 136, 148, 28, 32] {
            wd(cell(off), 0);
        }
        wd(cell(36), 0x8000_0000);
        wd(cell(64), cx);
        wd(cell(68), cy);
        wd(cell(76), cx);
        wd(cell(80), cy);
        wd(cell(140), 0x50);
        wd(cell(144), 1);
        // (wait until the other thread is done with the hit objects)
        while rd(0xf53908) != 0 {
            cov(0x493612);
            call(&mut r, 0x492d40, NONE, NONE, &[], 0);
        }
        r.ecx = cell(64) as u32;
        call(&mut r, 0x6fa820, ECX, AL, &[cell(28) as u32, 1, 0], 0);
        if r.eax as u8 != 0 && (rd(cell(32)) as i32) > 0 {
            cov(0x49363d);
            let count = rd(cell(32));
            let hits = rd(cell(28)) as usize;
            let mut attach_mode = rd(0xebcea0);
            let mut attach_obj = rd(0xebce9c) as usize;
            for i in 0..count as usize {
                let handle = rd(hits + 0x28 + 0x3c * i);
                if handle == 0 {
                    continue;
                }
                let slot = rd(0x2149c64 + (((handle & 0xfff) << 4) as usize)) as usize;
                if slot == 0 || !eq(rd(slot + 0x44), handle) || !eq(rd(slot + 0x4c), 1) {
                    continue;
                }
                cov(0x493660);
                // is this vehicle one the camera is attached to (then it stays)?
                let check = if attach_obj == 0 || handle != rd(attach_obj + 0x44) { true } else { attach_mode == 7 || attach_mode == 8 };
                let mut hide = !check;
                if check {
                    r.eax = slot as u32;
                    call(&mut r, 0x9b92e0, EAX, AL, &[], 0);
                    if r.eax as u8 != 0 {
                        hide = true;
                    } else {
                        let driver = rd(slot + 0xd74);
                        r.ecx = driver;
                        call(&mut r, 0xae28f0, ECX, EAX, &[], 0);
                        if r.eax == 0 {
                            hide = true;
                        } else {
                            r.ecx = driver;
                            call(&mut r, 0xae28f0, ECX, EAX, &[], 0);
                            let human = r.eax as usize;
                            let seat = rd(slot + 0xd78);
                            if human == 0 || seat == 0xffff_ffff {
                                hide = true;
                            } else {
                                let rec = rd(human + 0x84e4) as usize;
                                if eq(rd(rec + 0x6f4 + 4 * seat as usize), rd(0x33d630c)) {
                                    continue;
                                }
                                r.eax = slot as u32;
                                call(&mut r, 0x926650, EAX, AL, &[], 0);
                                if r.eax as u8 != 0 {
                                    continue;
                                }
                                hide = true;
                            }
                        }
                    }
                }
                if hide {
                    cov(0x493702);
                    call(&mut r, 0x492d90, NONE, NONE, &[], 0);
                    attach_mode = rd(0xebcea0);
                    attach_obj = rd(0xebce9c) as usize;
                }
            }
        }
        // the hit list goes back to the heap
        let heap = rd(cell(36));
        if (heap as i32) >= 0 {
            cov(0x49372d);
            hcall(&mut r, rd(0xdb807c) as usize, NONE, EAX, &[rd(0x31bf2bc)], 1, &[], &[]);
            r.ecx = r.eax;
            let size = (heap & 0x3fff_ffff).wrapping_mul(60);
            call(&mut r, 0xd2b7d0, ECX, NONE, &[rd(cell(28)), size, 0x17], 3);
        }
    }

    // publish the target's position / matrix and the frame time; measure the jump
    cov(0x49375a);
    wd(0x25f5bb4, rd(tgt + 0x14));
    wd(0x25f5bb8, rd(tgt + 0x18));
    wd(0x25f5bbc, rd(tgt + 0x1c));
    for k in 0..9 {
        wd(0x25f5bc0 + 4 * k, rd(tgt + 0x20 + 4 * k));
    }
    wf(0x25f5be4, dt);
    let moved = call(&mut r, 0xbde980, NONE, NONE, &[cell(40) as u32, 0x25f5b14], 0).st[0];
    let jump = (moved + (rf(0x2526be8) as f64)) as f32;
    wb(0xe996e5, 1);
    wf(cell(20), jump);
    // (cleared unless jump >= limit as an ordered comparison: a NaN on either side clears it)
    if !((jump as f64) >= (rf(0xe88be8) as f64)) {
        cov(0x4937c1);
        wb(0xe996e5, 0);
    }
    let prev_region = rd(0x25f5bec) & 0xffff;
    wf(0x2526be8, 0.0);
    wd(0x25f5bf0, prev_region);
    r.edi = 0x25f5b14;
    call(&mut r, 0x4a9be0, EDI, EAX, &[], 0);
    wb(0x25f5bec, r.eax as u8);
    wb(0x25f5bed, (r.eax >> 8) as u8);
    if rb(0x2526b5a) != 0 {
        cov(0x4937f5);
        wb(0xfa239a, 0);
        wb(0x2526b5a, 0);
    }
}
