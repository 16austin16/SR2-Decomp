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
