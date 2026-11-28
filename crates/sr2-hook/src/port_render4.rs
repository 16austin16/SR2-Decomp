//! Render-thread functions the last round skipped, plus the next hottest ones from doc 58
//! section 5:
//!
//! - the three city chunk tree walks `0x4B6A20` (frustum), `0x4B6B60` (dual frustum) and
//!   `0x4B6CC0` (view bounds), explicit 32-entry stacks like the original's;
//! - `0xBF65F0` `aabb_point_dist_sq_x87` (mixed x87 / SSE, also writes three floats through ESI,
//!   result on ST0);
//! - `0x4C5180` `glare_occlusion_query_update` (D3D query polling);
//! - `0xD19E30` `shader_param_write_floats`, `0xD1ACA0` `shader_param_set_by_id`, `0xD1AD50`
//!   `shader_param_set`;
//! - `0xD22A00` `gfx_eye_facing_test`, `0xBF60D0` `vec3_sign_octant`, `0xD18E50`
//!   `gfx_cmd_set_const5_vec3_cached`.
//!
//! Checks (see `install`):
//! - `0xBF65F0`: snapshot shadow for the first `SHADOW_CALLS` calls (the three floats at ESI and
//!   ST0; the original's results are kept on a mismatch).
//! - `0xD22A00`: relocated-original shadow (pure), first `SHADOW_CALLS` calls.
//! - `0xBF60D0`: snapshot shadow (two dword writes).
//! - Everything else writes through callees with side effects or over big tables (tree walks,
//!   glare, shader params, command stream): per-branch counters for the first `SHADOW_CALLS`
//!   calls only.
//! - All of them are also checked offline against the original bytes (tests at the bottom).
//!
//! Every thunk keeps ECX / EDX / XMM0-7 as the caller left them where the original does not touch
//! them (callers of LTCG leaves may hold live values there). `0xBF60D0` and `0xD22A00`
//! additionally keep EAX / EDX where the original does.
//!
//! Deviations: the tree walks' explicit stacks hold 32 entries like the original's frame; the
//! original's 33rd push would overwrite its own saved registers / return address, the port drops
//! such a push instead (never reached with real trees: the stack grows by one per level of right
//! children still pending).

#![allow(unused_imports)]

use std::arch::x86::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const TREE_FRUSTUM: usize = 0x004B_6A20;
const TREE_DUAL: usize = 0x004B_6B60;
const TREE_VIEW: usize = 0x004B_6CC0;
const DIST_X87: usize = 0x00BF_65F0;
const GLARE_UPDATE: usize = 0x004C_5180;
const PARAM_WRITE_FLOATS: usize = 0x00D1_9E30;
const PARAM_SET_BY_ID: usize = 0x00D1_ACA0;
const PARAM_SET: usize = 0x00D1_AD50;
const EYE_FACING: usize = 0x00D2_2A00;
const SIGN_OCTANT: usize = 0x00BF_60D0;
const SET_CONST5: usize = 0x00D1_8E50;

const TREE_LEAF: usize = 0x004B_6970;
const FRUSTUM_TEST_AABB: usize = 0x0053_0270;
const VIEW_BOUNDS_VISIBLE: usize = 0x0053_0450;
#[cfg_attr(test, allow(dead_code))]
const SQRTF_WRAPPER: usize = 0x0040_EA20;
/// Where the glare port calls the square root wrapper (the tests cannot map the page it is on).
#[cfg(not(test))]
#[inline(always)]
fn sqrtf_addr() -> usize {
    SQRTF_WRAPPER
}
#[cfg(test)]
fn sqrtf_addr() -> usize {
    tests::sqrt_stub_addr()
}
const THREAD_STATE_COPY: usize = 0x00D2_3840;
const VEC3_NORMALIZE: usize = 0x00BD_DFF0;
const MAT33_FROM_FORWARD: usize = 0x00BE_4B20;
const GLARE_PROJECT: usize = 0x00D2_2C80;
const GFX_STATE: usize = 0x00D2_2790;
const LOWER_BOUND_KEY64: usize = 0x00BD_5C30;

const CHUNK_COUNT: usize = 0x00F6_FBC4;
const CHUNK_PTRS: usize = 0x00F6_FBD8;
const CHUNK_FLAGS: usize = 0x00F8_3B08;
const PLANE_COUNT: usize = 0x0252_7398;
const SHADOW_PLANE_COUNT: usize = 0x0252_739C;
const SHADOW_FLAG: usize = 0x0261_2D74; // byte
const SHADOW_PLANES: u32 = 0x0261_2D80;
const SHADOW_CELLS: u32 = 0x0277_85D0;
const VIEW_CELLS: u32 = 0x0277_41D0;

const QUERY_TABLE: usize = 0x00F8_49A8;
const QUERY_PIXELS_MIN: usize = 0x0252_6D2C;
const GLARE_COUNT: usize = 0x0252_6D38;
const GLARE_ENTRIES: usize = 0x0264_8558; // 0x38 bytes each
const GLARE_FALLBACK: usize = 0x0252_A6D0; // vec3
const GLARE_SCALE_A: usize = 0x00E5_C0D8; // f64
const GLARE_SCALE_B: usize = 0x00E5_CAE0; // f64
const GLARE_SCALE_C: usize = 0x00E5_C0F8; // f64
const GLARE_SCALE_W: usize = 0x00E5_C180; // f64

const DIST_ZERO: usize = 0x00E5_BFB0; // f32 (-0.0)
const EYE_FACING_MIN: usize = 0x00DD_32C0; // f32

const CMD_CURSOR: usize = 0x033D_62F8;
const CONST5_CACHE: usize = 0x0230_4DAC; // 4 floats

// --- small helpers ----------------------------------------------------------------------------------

#[inline(always)]
unsafe fn rh(p: usize) -> u32 {
    unsafe { *(p as *const u16) as u32 }
}

#[inline(always)]
unsafe fn rdbl(p: usize) -> f64 {
    unsafe { std::ptr::read_unaligned(p as *const f64) }
}

/// `fs:[0x2c]` -> slot 0: the render thread's state block.
#[inline(always)]
unsafe fn thread_state() -> usize {
    let tls: usize;
    unsafe {
        std::arch::asm!("mov {}, fs:[0x2c]", out(reg) tls, options(nostack, readonly, preserves_flags));
        rd(tls) as usize
    }
}

/// What `fld dword` / `fstp dword` does to a float's bits: a signalling NaN comes out quiet.
#[inline(always)]
fn quiet(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// An x87 result stored as `fstp dword`: rounded to f32, a NaN left quiet.
#[inline(always)]
fn st(d: f64) -> u32 {
    quiet((d as f32).to_bits())
}

/// The x87 rule for an operation whose two operands are both NaN: the larger significand wins
/// (SSE would take the first operand); `r` is the SSE result otherwise.
#[inline(always)]
fn xn(a: f64, b: f64, r: f64) -> f64 {
    if a.is_nan() && b.is_nan() {
        let m = 0x000F_FFFF_FFFF_FFFFu64;
        let (ma, mb) = (a.to_bits() & m, b.to_bits() & m);
        if ma > mb || (ma == mb && !a.is_sign_negative()) { a } else { b }
    } else {
        r
    }
}

/// `frustum_test_aabb` (ECX = min corner, EDX = box, cdecl (planes, cells, count)) -> EAX.
#[inline(always)]
unsafe fn frustum(ecx: usize, edx: usize, a0: u32, a1: u32, a2: u32) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push {a2}",
            "push {a1}",
            "push {a0}",
            "call {f}",
            "add esp, 12",
            a0 = in(reg) a0,
            a1 = in(reg) a1,
            a2 = in(reg) a2,
            f = in(reg) FRUSTUM_TEST_AABB,
            in("ecx") ecx,
            in("edx") edx,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

/// A game function that returns on the x87 stack, as the f64 it holds.
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    let mut out = 0f64;
    let block: [u32; 9] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "mov ebx, eax",
            "mov ecx, [ebx + 24]",
            "mov esi, [ebx + 20]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]", "mov esi, [ebx + 12]", "mov edi, [ebx + 16]",
            "mov eax, [ebx + 28]",
            "call dword ptr [ebx]",
            "mov ecx, [ebx + 32]",
            "fstp qword ptr [ecx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    out
}

// --- shadow bookkeeping -----------------------------------------------------------------------------

struct Shadow {
    name: &'static str,
    orig: AtomicUsize,
    /// Calls seen while the window is open (stops counting at `SHADOW_CALLS`).
    calls: AtomicU32,
    bad: AtomicU32,
    /// All calls, counted without a lock prefix (approximate across threads).
    total: AtomicU32,
}

impl Shadow {
    const fn new(name: &'static str) -> Shadow {
        Shadow { name, orig: AtomicUsize::new(0), calls: AtomicU32::new(0), bad: AtomicU32::new(0), total: AtomicU32::new(0) }
    }
    #[inline(always)]
    fn active(&self) -> bool {
        let t = self.total.load(Ordering::Relaxed);
        self.total.store(t.wrapping_add(1), Ordering::Relaxed);
        self.calls.load(Ordering::Relaxed) < SHADOW_CALLS && self.orig.load(Ordering::Relaxed) != 0
    }
}

static S_DIST: Shadow = Shadow::new("aabb_point_dist_sq_x87");
static S_EYE: Shadow = Shadow::new("gfx_eye_facing_test");
static S_OCT: Shadow = Shadow::new("vec3_sign_octant");

/// Per-function counters for functions checked by branch only. `[0]` = calls (all, no lock
/// prefix), `[1..]` = branch counters, counted for the first `SHADOW_CALLS` calls only.
const NB: usize = 8;
const NFUNC: usize = 7;
static BR_SEEN: [AtomicU32; NFUNC] = [const { AtomicU32::new(0) }; NFUNC];
static BR_TOTAL: [AtomicU32; NFUNC] = [const { AtomicU32::new(0) }; NFUNC];
static BR: [[AtomicU32; NB]; NFUNC] = [const { [const { AtomicU32::new(0) }; NB] }; NFUNC];
const BR_NAMES: [&str; NFUNC] = [
    "city_chunk_tree_cull_frustum",
    "city_chunk_tree_cull_frustum_dual",
    "city_chunk_tree_cull_view_bounds",
    "glare_occlusion_query_update",
    "shader_param_write_floats",
    "shader_param_set_by_id",
    "shader_param_set",
];
const F_TREE_A: usize = 0;
const F_TREE_B: usize = 1;
const F_TREE_C: usize = 2;
const F_GLARE: usize = 3;
const F_PWRITE: usize = 4;
const F_PBYID: usize = 5;
const F_PSET: usize = 6;

/// Opens the counting window for one call; returns whether this call is counted.
#[inline(always)]
fn br_open(f: usize) -> bool {
    let t = BR_TOTAL[f].load(Ordering::Relaxed);
    BR_TOTAL[f].store(t.wrapping_add(1), Ordering::Relaxed);
    let c = BR_SEEN[f].load(Ordering::Relaxed);
    if c < SHADOW_CALLS {
        BR_SEEN[f].store(c + 1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

#[inline(always)]
fn br_flush(f: usize, cnt: &[u32; NB]) {
    for (i, &c) in cnt.iter().enumerate() {
        if c != 0 {
            BR[f][i].fetch_add(c, Ordering::Relaxed);
        }
    }
}

static CONST5_TOTAL: AtomicU32 = AtomicU32::new(0);
static CONST5_HIT: AtomicU32 = AtomicU32::new(0);
static CONST5_PUSH: AtomicU32 = AtomicU32::new(0);

// --- thunk pieces -----------------------------------------------------------------------------------

/// Saves XMM0-7 below the stack pointer (128 bytes).
macro_rules! xmm_save {
    () => {
        concat!(
            "sub esp, 128\n",
            "movups xmmword ptr [esp], xmm0\n",
            "movups xmmword ptr [esp + 16], xmm1\n",
            "movups xmmword ptr [esp + 32], xmm2\n",
            "movups xmmword ptr [esp + 48], xmm3\n",
            "movups xmmword ptr [esp + 64], xmm4\n",
            "movups xmmword ptr [esp + 80], xmm5\n",
            "movups xmmword ptr [esp + 96], xmm6\n",
            "movups xmmword ptr [esp + 112], xmm7\n",
        )
    };
}

macro_rules! xmm_load {
    () => {
        concat!(
            "movups xmm0, xmmword ptr [esp]\n",
            "movups xmm1, xmmword ptr [esp + 16]\n",
            "movups xmm2, xmmword ptr [esp + 32]\n",
            "movups xmm3, xmmword ptr [esp + 48]\n",
            "movups xmm4, xmmword ptr [esp + 64]\n",
            "movups xmm5, xmmword ptr [esp + 80]\n",
            "movups xmm6, xmmword ptr [esp + 96]\n",
            "movups xmm7, xmmword ptr [esp + 112]\n",
            "add esp, 128\n",
        )
    };
}

/// Saves XMM4-7 (the original clobbers XMM0-3 itself).
macro_rules! xmm_save_hi {
    () => {
        concat!(
            "sub esp, 64\n",
            "movups xmmword ptr [esp], xmm4\n",
            "movups xmmword ptr [esp + 16], xmm5\n",
            "movups xmmword ptr [esp + 32], xmm6\n",
            "movups xmmword ptr [esp + 48], xmm7\n",
        )
    };
}

macro_rules! xmm_load_hi {
    () => {
        concat!(
            "movups xmm4, xmmword ptr [esp]\n",
            "movups xmm5, xmmword ptr [esp + 16]\n",
            "movups xmm6, xmmword ptr [esp + 32]\n",
            "movups xmm7, xmmword ptr [esp + 48]\n",
            "add esp, 64\n",
        )
    };
}

// =================================================================================================
// 0x4B6A20 / 0x4B6B60 / 0x4B6CC0 city chunk tree walks
// =================================================================================================
//
// For every streamed chunk (`[0xF6FBD8]`, count `[0xF6FBC4]`) whose flag byte `[0xF83B08 + id]` is
// set and that has a tree (`chunk+0x50` -> `+8` non-zero), the nodes (0x30 bytes at `tree[+4]`;
// box at `+0x10`, leaf count at `+0xC`, left child `+0x28`, right child `+0x2C`) are walked with an
// explicit stack of 32 entries. A node that passes its visibility test and has a count hands the
// work item `{counter | id << 16, chunk + 0x40, node, extra}` (ESI = &item, EAX = the test's
// result) to `0x4B6970`; one without a count descends: right child pushed when both exist.
//   A: frustum test on the thread view's planes, extra = -1.
//   B: first the second plane set (unless `[0x2612D74]` is set, then the node is taken without a
//      test), then the thread view's planes; extra = 0.
//   C: `view_bounds_visible(node)` with ESI = the view index (the cdecl argument), extra = view.

#[inline(always)]
unsafe fn tree_impl<const MODE: u32>(view: u32, cnt: &mut [u32; NB]) {
    unsafe {
        if (rd(CHUNK_COUNT) as i32) <= 0 {
            return;
        }
        let mut i: u32 = 0;
        loop {
            let chunk = rd(CHUNK_PTRS + i as usize * 4) as usize;
            let id = rh(chunk + 8);
            if rb(CHUNK_FLAGS + id as usize) != 0 {
                let t = rd(chunk + 0x50) as usize;
                if t != 0 && rd(t + 8) != 0 {
                    cnt[1] += 1;
                    let state = if MODE == 0 { thread_state() } else { 0 };
                    let mut item: [u32; 4] =
                        [(i & 0xFFFF) | (id << 16), (chunk + 0x40) as u32, 0, match MODE { 0 => 0xFFFF_FFFF, 1 => 0, _ => view }];
                    let mut stack = [0u32; 32];
                    let mut sp = 0usize;
                    let mut idx = 0u32;
                    loop {
                        let t = rd(chunk + 0x50) as usize;
                        let node = rd(t + 4).wrapping_add(idx.wrapping_mul(0x30)) as usize;
                        cnt[2] += 1;
                        let (ok, eax): (bool, u32) = match MODE {
                            0 => {
                                let planes = rd(rd(state + 0x58) as usize + 0x740);
                                let r = frustum(node + 0x10, node, planes, VIEW_CELLS, rd(PLANE_COUNT));
                                (r as u8 != 0, r)
                            }
                            1 => {
                                if rb(SHADOW_FLAG) == 0 {
                                    let r1 = frustum(node + 0x10, node, SHADOW_PLANES, SHADOW_CELLS, rd(SHADOW_PLANE_COUNT));
                                    if r1 as u8 != 0 {
                                        (true, r1)
                                    } else {
                                        let st = thread_state();
                                        let planes = rd(rd(st + 0x58) as usize + 0x740);
                                        let r2 = frustum(node + 0x10, node, planes, VIEW_CELLS, rd(PLANE_COUNT));
                                        (r2 as u8 != 0, r2)
                                    }
                                } else {
                                    (true, chunk as u32)
                                }
                            }
                            _ => {
                                let r = call_regs(VIEW_BOUNDS_VISIBLE, 0, chunk as u32, t as u32, view, (node + 0x10) as u32, &[node as u32]);
                                (r as u8 != 0, r)
                            }
                        };
                        if !ok {
                            cnt[3] += 1;
                        } else if rd(node + 0xC) != 0 {
                            cnt[4] += 1;
                            item[2] = idx;
                            call_regs(TREE_LEAF, eax, 0, 0, item.as_ptr() as u32, 0, &[]);
                        } else {
                            let right = rd(node + 0x2C);
                            if right == 0 {
                                idx = rd(node + 0x28);
                            } else {
                                let left = rd(node + 0x28);
                                if left == 0 {
                                    idx = right;
                                } else {
                                    if sp < 32 {
                                        stack[sp] = right;
                                        sp += 1;
                                    } else {
                                        cnt[5] += 1;
                                    }
                                    idx = left;
                                }
                            }
                            continue;
                        }
                        // rejected or leaf: next pending node, or this chunk is done
                        if sp == 0 {
                            break;
                        }
                        sp -= 1;
                        idx = stack[sp];
                    }
                }
            }
            i = i.wrapping_add(1);
            if (i as i32) >= rd(CHUNK_COUNT) as i32 {
                break;
            }
        }
    }
}

extern "C" fn tree_a_rs() {
    let mut cnt = [0u32; NB];
    let counted = br_open(F_TREE_A);
    unsafe { tree_impl::<0>(0, &mut cnt) };
    if counted {
        cnt[0] = 1;
        br_flush(F_TREE_A, &cnt);
    }
}

extern "C" fn tree_b_rs() {
    let mut cnt = [0u32; NB];
    let counted = br_open(F_TREE_B);
    unsafe { tree_impl::<1>(0, &mut cnt) };
    if counted {
        cnt[0] = 1;
        br_flush(F_TREE_B, &cnt);
    }
}

extern "C" fn tree_c_rs(view: u32) {
    let mut cnt = [0u32; NB];
    let counted = br_open(F_TREE_C);
    unsafe { tree_impl::<2>(view, &mut cnt) };
    if counted {
        cnt[0] = 1;
        br_flush(F_TREE_C, &cnt);
    }
}

#[unsafe(naked)]
extern "C" fn tree_a_thunk() {
    std::arch::naked_asm!("push ecx", "push edx", xmm_save!(), "call {f}", xmm_load!(), "pop edx", "pop ecx", "ret", f = sym tree_a_rs)
}

#[unsafe(naked)]
extern "C" fn tree_b_thunk() {
    std::arch::naked_asm!("push ecx", "push edx", xmm_save!(), "call {f}", xmm_load!(), "pop edx", "pop ecx", "ret", f = sym tree_b_rs)
}

#[unsafe(naked)]
extern "C" fn tree_c_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push dword ptr [esp + 140]",
        "call {f}",
        "add esp, 4",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym tree_c_rs,
    )
}

// =================================================================================================
// 0xBF65F0 aabb_point_dist_sq_x87  (EAX = &point, ECX = &A, stack &B, ESI = &out[3] -> ST0)
// =================================================================================================

/// Per axis: `d1 = f32(p - a)`, `d2 = f32(b - p)`; `m4 = d1 >= 0 ? -0.0 - d1 : 0` (SSE),
/// `e = d2 >= 0 ? d2 : 0`, `a = d1 >= 0 ? d1 : 0`; `out = f32(p + (e + m4))` (written through ESI),
/// `d = f32(e + a)`; result `f32((dx^2 + dy^2) + dz^2)` on the x87 stack (53-bit).
#[inline(always)]
unsafe fn dist_impl(p: usize, a: usize, b: usize, out: usize) -> f32 {
    unsafe {
        let k = rf(DIST_ZERO);
        let mut o = [0f32; 3];
        let mut d = [0f32; 3];
        for i in 0..3 {
            let pv = rf(p + i * 4);
            let av = rf(a + i * 4);
            let bv = rf(b + i * 4);
            let d1 = (pv as f64 - av as f64) as f32;
            let d2 = (bv as f64 - pv as f64) as f32;
            let m4 = if d1 >= 0.0 { k - d1 } else { 0.0 };
            let lo = if d1 >= 0.0 { d1 } else { 0.0 };
            let e = if d2 >= 0.0 { d2 } else { 0.0 };
            o[i] = (pv as f64 + (e as f64 + m4 as f64)) as f32;
            d[i] = (e as f64 + lo as f64) as f32;
        }
        wf(out, o[0]);
        wf(out + 4, o[1]);
        wf(out + 8, o[2]);
        let (dx, dy, dz) = (d[0] as f64, d[1] as f64, d[2] as f64);
        let s = (dx * dx + dy * dy) + dz * dz;
        s as f32
    }
}

/// The original (relocated copy), returning its ST0.
unsafe fn dist_call(f: usize, p: usize, a: usize, b: usize, out: usize) -> f32 {
    let mut r = 0f32;
    unsafe {
        std::arch::asm!(
            "push {r}",
            "push esi",
            "mov esi, {out}",
            "push {b}",
            "call {f}",
            "add esp, 4",
            "pop esi",
            "pop edx",
            "fstp dword ptr [edx]",
            b = in(reg) b,
            f = in(reg) f,
            r = in(reg) &mut r as *mut f32,
            out = in(reg) out,
            in("eax") p,
            in("ecx") a,
            clobber_abi("C"),
        );
    }
    r
}

#[inline(always)]
fn same_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

extern "C" fn dist_rs(p: u32, a: u32, b: u32, out: u32) -> f32 {
    unsafe {
        if !S_DIST.active() {
            return dist_impl(p as usize, a as usize, b as usize, out as usize);
        }
        S_DIST.calls.fetch_add(1, Ordering::Relaxed);
        let o = out as usize;
        let saved = [rd(o), rd(o + 4), rd(o + 8)];
        let orig = dist_call(S_DIST.orig.load(Ordering::Relaxed), p as usize, a as usize, b as usize, o);
        let orig_out = [rd(o), rd(o + 4), rd(o + 8)];
        wd(o, saved[0]);
        wd(o + 4, saved[1]);
        wd(o + 8, saved[2]);
        let mine = dist_impl(p as usize, a as usize, b as usize, o);
        let mine_out = [rd(o), rd(o + 4), rd(o + 8)];
        let out_same = (0..3).all(|i| mine_out[i] == orig_out[i] || (f32::from_bits(mine_out[i]).is_nan() && f32::from_bits(orig_out[i]).is_nan()));
        if !same_f32(mine, orig) || !out_same {
            S_DIST.bad.fetch_add(1, Ordering::Relaxed);
        }
        wd(o, orig_out[0]);
        wd(o + 4, orig_out[1]);
        wd(o + 8, orig_out[2]);
        orig
    }
}

#[unsafe(naked)]
extern "C" fn dist_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save_hi!(),
        "push esi",
        "push dword ptr [esp + 80]",
        "push ecx",
        "push eax",
        "call {f}",
        "add esp, 16",
        xmm_load_hi!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym dist_rs,
    )
}

// =================================================================================================
// 0x4C5180 glare_occlusion_query_update  (EAX = &camera, stack (record*, byte, byte), plain ret)
// =================================================================================================
//
// record: +0xC position, +0x3E flags, +0x80 query pending, +0x81 visible, +0x82 query id (u16).
// Polls the query (COM `GetData` through vtable slot 7, 4 bytes), then, when the last byte
// argument is set and no query is pending, starts the next one by appending an entry (0x38 bytes
// at `0x2648558 + [0x2526D38] * 0x38`) with the direction / position the byte argument picks.

unsafe fn glare_impl(cam: usize, rec: usize, from_rec: u8, start: u8, cnt: &mut [u32; NB]) {
    unsafe {
        if rb(rec + 0x80) == 0 {
            wb(rec + 0x81, 0);
        } else {
            let obj = rd(rd(QUERY_TABLE + rh(rec + 0x82) as usize * 4) as usize) as usize;
            let mut res: u32 = 0;
            if obj != 0 {
                cnt[1] += 1;
                let vt = rd(obj) as usize;
                let r = call_regs(rd(vt + 0x1C) as usize, rd(vt + 0x1C), vt as u32, &mut res as *mut u32 as u32, 0, 0, &[obj as u32, &mut res as *mut u32 as u32, 4, 0]);
                if r != 1 {
                    wb(rec + 0x80, 0);
                    if r != 0 {
                        cnt[2] += 1;
                        wb(rec + 0x81, 0);
                    } else if res <= 0x2710 {
                        cnt[3] += 1;
                        wb(rec + 0x81, (res > rd(QUERY_PIXELS_MIN)) as u8);
                    }
                }
            }
        }
        if start == 0 || rb(rec + 0x80) != 0 {
            return;
        }
        cnt[4] += 1;
        wb(rec + 0x80, 1);
        // the glare position (bits), or the camera-relative offset
        let (x, y, z) = if from_rec != 0 {
            (rd(rec + 0xC), rd(rec + 0x10), rd(rec + 0x14))
        } else {
            (
                st(rf(cam) as f64 - rf(rec + 0xC) as f64),
                st(rf(cam + 4) as f64 - rf(rec + 0x10) as f64),
                st(rf(cam + 8) as f64 - rf(rec + 0x14) as f64),
            )
        };
        // v = normalize(v), or the fallback direction when its length is 0
        let (fx, fy, fz) = (f32::from_bits(x) as f64, f32::from_bits(y) as f64, f32::from_bits(z) as f64);
        let (qx, qy, qz) = (fx * fx, fy * fy, fz * fz);
        let s1 = xn(qx, qy, qx + qy);
        let l2 = f32::from_bits(st(xn(s1, qz, s1 + qz)));
        let len = call_regs_st0(sqrtf_addr(), 0, 0, 0, 0, 0, &[l2.to_bits()]) as f32;
        let mut v: [u32; 3];
        if len == 0.0 {
            cnt[5] += 1;
            v = [rd(GLARE_FALLBACK), rd(GLARE_FALLBACK + 4), rd(GLARE_FALLBACK + 8)];
        } else {
            let inv = (1.0f64 / len as f64) as f32 as f64;
            v = [st(xn(fx, inv, fx * inv)), st(xn(fy, inv, fy * inv)), st(xn(inv, fz, inv * fz))];
        }
        // the point on the glare (pos -/+ direction * scale)
        let (vx, vy, vz) = (f32::from_bits(v[0]) as f64, f32::from_bits(v[1]) as f64, f32::from_bits(v[2]) as f64);
        let (px, py, pz) = (rf(rec + 0xC) as f64, rf(rec + 0x10) as f64, rf(rec + 0x14) as f64);
        if from_rec != 0 {
            let k = rdbl(GLARE_SCALE_A);
            let t = [(vx * k) as f32 as f64, (vy * k) as f32 as f64, (k * vz) as f32 as f64];
            v = [st(xn(px, t[0], px - t[0])), st(xn(py, t[1], py - t[1])), st(xn(pz, t[2], pz - t[2]))];
        } else {
            let k = if rh(rec + 0x3E) & 0x4000 != 0 { rdbl(GLARE_SCALE_B) } else { rdbl(GLARE_SCALE_C) };
            let t = [(vx * k) as f32 as f64, (vy * k) as f32 as f64, (k * vz) as f32 as f64];
            v = [st(xn(t[0], px, t[0] + px)), st(xn(t[1], py, t[1] + py)), st(xn(t[2], pz, t[2] + pz))];
        }
        // the camera matrix rows and the basis built from its forward axis
        let mut m = [0u32; 9];
        call_regs(THREAD_STATE_COPY, 0, 0, 0, m.as_mut_ptr() as u32, 0, &[]);
        let kw = rdbl(GLARE_SCALE_W);
        let mut w = [
            st(f32::from_bits(m[6]) as f64 * kw),
            st(f32::from_bits(m[7]) as f64 * kw),
            st(kw * f32::from_bits(m[8]) as f64),
        ];
        call_regs(VEC3_NORMALIZE, 0, 0, 0, w.as_mut_ptr() as u32, 0, &[]);
        call_regs(MAT33_FROM_FORWARD, w.as_ptr() as u32, m.as_mut_ptr() as u32, 0, 0, 0, &[]);
        let proj = call_regs_st0(GLARE_PROJECT, 0, 0, 0, v.as_ptr() as u32, 0, &[1.0f32.to_bits()]);
        let size = f32::from_bits(st(proj.abs()));
        // append the entry
        let n = rd(GLARE_COUNT) as usize;
        let e = GLARE_ENTRIES.wrapping_add(n.wrapping_mul(0x38));
        wf(e + 0xC, size);
        wd(e, v[0]);
        wd(GLARE_COUNT, (n as u32).wrapping_add(1));
        wd(e + 4, v[1]);
        wd(e + 8, v[2]);
        for (k, &word) in m.iter().enumerate() {
            wd(e + 0x10 + k * 4, word);
        }
        if rb(rec + 0x3E) & 4 != 0 {
            wd(e + 0x34, 0);
        } else {
            wd(e + 0x34, rd(QUERY_TABLE + rh(rec + 0x82) as usize * 4));
        }
    }
}

extern "C" fn glare_rs(cam: u32, rec: u32, a2: u32, a3: u32) {
    let mut cnt = [0u32; NB];
    let counted = br_open(F_GLARE);
    unsafe { glare_impl(cam as usize, rec as usize, a2 as u8, a3 as u8, &mut cnt) };
    if counted {
        cnt[0] = 1;
        br_flush(F_GLARE, &cnt);
    }
}

#[unsafe(naked)]
extern "C" fn glare_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push dword ptr [esp + 148]",
        "push dword ptr [esp + 148]",
        "push dword ptr [esp + 148]",
        "push eax",
        "call {f}",
        "add esp, 16",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym glare_rs,
    )
}

// =================================================================================================
// 0xD19E30 shader_param_write_floats  (EAX = param block, stack (entry*, src*), plain ret)
// =================================================================================================
//
// entry: +8 u16 outer count, +0xA u16 inner count, +0xC u32 index table. For every outer index the
// inner floats are copied (through the x87, so signalling NaNs come out quiet) from `src` to
// `block[+0xC] + index * 4`.

/// Returns EAX as the original leaves it. With a non-empty outer count the original also overwrites its
/// first stack argument slot with the block's table pointer, and leaves ECX = EDX = 0 (the high halves
/// of its 64-bit loop counters).
unsafe fn pwrite_impl(block: usize, e: usize, src: usize, slot: *mut u32, out: *mut [u32; 2]) -> u32 {
    unsafe {
        if rh(e + 8) == 0 {
            return block as u32;
        }
        *slot = rd(block + 0xC);
        (*out)[0] = 0;
        (*out)[1] = 0;
        let base = rd(block + 0xC) as usize;
        let mut inner = rh(e + 0xA);
        let mut o: u32 = 0;
        loop {
            // volatile: the original always reads the index, even when the inner count is 0
            let idx = std::ptr::read_volatile((rd(e + 0xC) as usize + o as usize * 4) as *const u32) as usize;
            let mut dest = base.wrapping_add(idx.wrapping_mul(4));
            let mut s = src;
            if inner != 0 {
                let mut k: u32 = 0;
                loop {
                    wd(dest, quiet(rd(s)));
                    dest = dest.wrapping_add(4);
                    s = s.wrapping_add(4);
                    k += 1;
                    inner = rh(e + 0xA);
                    if k >= inner {
                        break;
                    }
                }
            }
            o += 1;
            if o >= rh(e + 8) {
                break;
            }
        }
        rh(e + 8)
    }
}

extern "C" fn pwrite_rs(block: u32, e: u32, src: u32, slot: *mut u32, out: *mut [u32; 2]) -> u32 {
    let counted = br_open(F_PWRITE);
    let r = unsafe { pwrite_impl(block as usize, e as usize, src as usize, slot, out) };
    if counted {
        BR[F_PWRITE][0].fetch_add(1, Ordering::Relaxed);
    }
    r
}

#[unsafe(naked)]
extern "C" fn pwrite_thunk() {
    std::arch::naked_asm!(
        "sub esp, 8",
        "mov [esp], ecx",
        "mov [esp + 4], edx",
        xmm_save!(),
        "lea ecx, [esp + 128]",
        "push ecx",
        "lea ecx, [esp + 144]",
        "push ecx",
        "push dword ptr [esp + 152]",
        "push dword ptr [esp + 152]",
        "push eax",
        "call {f}",
        "add esp, 20",
        xmm_load!(),
        "mov ecx, [esp]",
        "mov edx, [esp + 4]",
        "add esp, 8",
        "ret",
        f = sym pwrite_rs,
    )
}

// =================================================================================================
// 0xD1ACA0 shader_param_set_by_id  (ESI = list {count, entries (16 bytes) at +4}, stack (param, id, src))
// =================================================================================================

/// Returns EAX as the original leaves it: the entry EAX for an empty list, else the last entry pointer
/// or (after a match) the callee's result.
unsafe fn pbyid_impl(list: usize, param: u32, id: u32, src: u32, eax0: u32) -> u32 {
    unsafe {
        if rd(list) == 0 {
            return eax0;
        }
        let mut eax;
        let mut i: u32 = 0;
        let mut off: u32 = 0;
        loop {
            let ent = rd(list + 4).wrapping_add(off);
            eax = ent;
            if rd(ent as usize) == id {
                eax = call_regs(PARAM_WRITE_FLOATS, param, 0, 0, 0, 0, &[ent, src]);
            }
            i += 1;
            off = off.wrapping_add(0x10);
            if i >= rd(list) {
                break;
            }
        }
        eax
    }
}

extern "C" fn pbyid_rs(list: u32, param: u32, id: u32, src: u32, eax0: u32) -> u32 {
    let counted = br_open(F_PBYID);
    let r = unsafe { pbyid_impl(list as usize, param, id, src, eax0) };
    if counted {
        BR[F_PBYID][0].fetch_add(1, Ordering::Relaxed);
    }
    r
}

#[unsafe(naked)]
extern "C" fn pbyid_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push eax",
        "push dword ptr [esp + 152]",
        "push dword ptr [esp + 152]",
        "push dword ptr [esp + 152]",
        "push esi",
        "call {f}",
        "add esp, 20",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym pbyid_rs,
    )
}

// =================================================================================================
// 0xD1AD50 shader_param_set  (EAX = param block, EDI = flag, stack (a, b, c), plain ret)
// =================================================================================================
//
// flag == 0: `shader_param_set_by_id(ESI = block, a, b, c)`. Otherwise a binary search of the
// block's sorted 16-byte records `[block[+4], block[+4] + block[0] * 16)` for the key
// `{lo = b, hi = flag}` (`lower_bound_key64_rec16`, ECX = first, EAX = last); on an exact match
// `shader_param_write_floats(EAX = a, entry, c)`.

/// Returns EAX as the original leaves it (the last callee's result).
unsafe fn pset_impl(block: u32, flag: u32, a: u32, b: u32, c: u32) -> u32 {
    unsafe {
        if flag == 0 {
            return call_regs(PARAM_SET_BY_ID, c, a, 0, block, 0, &[a, b, c]);
        }
        let first = rd(block as usize + 4);
        let last = first.wrapping_add(rd(block as usize) << 4);
        let key: [u32; 2] = [b, flag];
        let r = call_regs(LOWER_BOUND_KEY64, last, first, 0, 0, 0, &[key.as_ptr() as u32]);
        if r != last && rd(r as usize + 4) == flag && rd(r as usize) == b {
            return call_regs(PARAM_WRITE_FLOATS, a, c, 0, 0, 0, &[r, c]);
        }
        r
    }
}

extern "C" fn pset_rs(block: u32, flag: u32, a: u32, b: u32, c: u32) -> u32 {
    let counted = br_open(F_PSET);
    let eax = unsafe { pset_impl(block, flag, a, b, c) };
    if counted {
        BR[F_PSET][0].fetch_add(1, Ordering::Relaxed);
        if flag == 0 {
            BR[F_PSET][1].fetch_add(1, Ordering::Relaxed);
        }
    }
    eax
}

#[unsafe(naked)]
extern "C" fn pset_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push dword ptr [esp + 148]",
        "push dword ptr [esp + 148]",
        "push dword ptr [esp + 148]",
        "push edi",
        "push eax",
        "call {f}",
        "add esp, 20",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym pset_rs,
    )
}

// =================================================================================================
// 0xD22A00 gfx_eye_facing_test  (ESI = point, EDI = normal -> EAX 0 / 1)
// =================================================================================================

#[inline(always)]
unsafe fn eye_impl(esi: usize, edi: usize) -> u32 {
    unsafe {
        let eye = call_regs(GFX_STATE, 0, 0, 0, 0, 0, &[]) as usize;
        let d = [
            (rf(eye) as f64 - rf(esi) as f64) as f32 as f64,
            (rf(eye + 4) as f64 - rf(esi + 4) as f64) as f32 as f64,
            (rf(eye + 8) as f64 - rf(esi + 8) as f64) as f32 as f64,
        ];
        let s = ((rf(edi + 4) as f64 * d[1] + rf(edi) as f64 * d[0]) + rf(edi + 8) as f64 * d[2]) as f32;
        // fcomp, test ah 0x41, jne: taken when s <= limit or unordered
        (s > rf(EYE_FACING_MIN)) as u32
    }
}

extern "C" fn eye_rs(esi: u32, edi: u32) -> u32 {
    unsafe {
        if !S_EYE.active() {
            return eye_impl(esi as usize, edi as usize);
        }
        S_EYE.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_EYE.orig.load(Ordering::Relaxed), 0, 0, 0, esi, edi, &[]);
        let mine = eye_impl(esi as usize, edi as usize);
        if mine != orig {
            S_EYE.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn eye_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push edi",
        "push esi",
        "call {f}",
        "add esp, 8",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym eye_rs,
    )
}

// =================================================================================================
// 0xBF60D0 vec3_sign_octant  (EAX = out A*, EDX = &vec3, stack out B*, plain ret)
// =================================================================================================

#[inline(always)]
unsafe fn octant_impl(a: usize, v: usize, b: usize) {
    unsafe {
        let x = rf(v) > 0.0;
        let y = rf(v + 4) > 0.0;
        let z = rf(v + 8) > 0.0;
        let (ra, rb_) = match (x, y, z) {
            (true, true, true) => (4, 2),
            (true, true, false) => (0, 6),
            (true, false, true) => (5, 3),
            (true, false, false) => (1, 7),
            (false, true, true) => (7, 1),
            (false, true, false) => (3, 5),
            (false, false, true) => (6, 0),
            (false, false, false) => (2, 4),
        };
        wd(a, ra);
        wd(b, rb_);
    }
}

extern "C" fn octant_rs(a: u32, v: u32, b: u32) {
    unsafe {
        if !S_OCT.active() {
            return octant_impl(a as usize, v as usize, b as usize);
        }
        S_OCT.calls.fetch_add(1, Ordering::Relaxed);
        let (pa, pb) = (a as usize, b as usize);
        let saved = [rd(pa), rd(pb)];
        call_regs(S_OCT.orig.load(Ordering::Relaxed), a, 0, v, 0, 0, &[b]);
        let orig = [rd(pa), rd(pb)];
        wd(pa, saved[0]);
        wd(pb, saved[1]);
        octant_impl(pa, v as usize, pb);
        if [rd(pa), rd(pb)] != orig {
            S_OCT.bad.fetch_add(1, Ordering::Relaxed);
        }
        wd(pa, orig[0]);
        wd(pb, orig[1]);
    }
}

#[unsafe(naked)]
extern "C" fn octant_thunk() {
    std::arch::naked_asm!(
        "push eax",
        "push ecx",
        "push edx",
        xmm_save!(),
        "push dword ptr [esp + 144]",
        "push edx",
        "push eax",
        "call {f}",
        "add esp, 12",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "pop eax",
        "ret",
        f = sym octant_rs,
    )
}

// =================================================================================================
// 0xD18E50 gfx_cmd_set_const5_vec3_cached  (EAX = &vec3, plain ret)
// =================================================================================================
//
// When the vec3's bits differ from the cached `{x, y, z, 1.0}` at 0x2304DAC, refreshes the cache
// (through the x87: signalling NaNs come out quiet) and appends the 7-dword command
// `{7, 5, 1, x, y, z, 1.0}` to the stream at `[0x33D62F8]`.

unsafe fn const5_impl(v: usize) {
    unsafe {
        if rd(v) == rd(CONST5_CACHE) && rd(v + 4) == rd(CONST5_CACHE + 4) && rd(v + 8) == rd(CONST5_CACHE + 8) {
            CONST5_HIT.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let (a, b, c) = (quiet(rd(v)), quiet(rd(v + 4)), quiet(rd(v + 8)));
        wd(CONST5_CACHE, a);
        wd(CONST5_CACHE + 4, b);
        wd(CONST5_CACHE + 8, c);
        wd(CONST5_CACHE + 12, 0x3F80_0000);
        let p = rd(CMD_CURSOR) as usize;
        wd(p, 7);
        wd(p + 4, 5);
        wd(p + 8, 1);
        wd(p + 0xC, a);
        wd(p + 0x10, b);
        wd(p + 0x14, c);
        wd(p + 0x18, 0x3F80_0000);
        wd(CMD_CURSOR, (p as u32).wrapping_add(0x1C));
        CONST5_PUSH.fetch_add(1, Ordering::Relaxed);
    }
}

extern "C" fn const5_rs(v: u32) {
    let t = CONST5_TOTAL.load(Ordering::Relaxed);
    CONST5_TOTAL.store(t.wrapping_add(1), Ordering::Relaxed);
    unsafe { const5_impl(v as usize) };
}

#[unsafe(naked)]
extern "C" fn const5_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push eax",
        "call {f}",
        "add esp, 4",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym const5_rs,
    )
}

// =================================================================================================
// install / summary
// =================================================================================================

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let shadowed: [(&Shadow, usize, &[u8], &[usize], usize); 3] = [
        (&S_DIST, DIST_X87, &DIST87_BODY, &DIST87_CALLS, dist_thunk as *const () as usize),
        (&S_EYE, EYE_FACING, &EYE_BODY, &EYE_CALLS, eye_thunk as *const () as usize),
        (&S_OCT, SIGN_OCTANT, &OCT_BODY, &OCT_CALLS, octant_thunk as *const () as usize),
    ];
    let mut ok = 0;
    let mut total = shadowed.len();
    for (s, addr, body, calls, thunk) in shadowed {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(addr, body, calls) } {
            Ok(copy) => s.orig.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_render4: {}: no shadow copy: {e:?}", s.name)),
        }
        if replace_pinned(s.name, addr, body, thunk) {
            ok += 1;
        }
    }
    for (name, addr, body, thunk) in [
        ("city_chunk_tree_cull_frustum", TREE_FRUSTUM, &TREE_A_BODY[..], tree_a_thunk as *const () as usize),
        ("city_chunk_tree_cull_frustum_dual", TREE_DUAL, &TREE_B_BODY[..], tree_b_thunk as *const () as usize),
        ("city_chunk_tree_cull_view_bounds", TREE_VIEW, &TREE_C_BODY[..], tree_c_thunk as *const () as usize),
        ("glare_occlusion_query_update", GLARE_UPDATE, &GLARE_BODY[..], glare_thunk as *const () as usize),
        ("shader_param_write_floats", PARAM_WRITE_FLOATS, &PWRITE_BODY[..], pwrite_thunk as *const () as usize),
        ("shader_param_set_by_id", PARAM_SET_BY_ID, &PBYID_BODY[..], pbyid_thunk as *const () as usize),
        ("shader_param_set", PARAM_SET, &PSET_BODY[..], pset_thunk as *const () as usize),
        ("gfx_cmd_set_const5_vec3_cached", SET_CONST5, &CONST5_BODY[..], const5_thunk as *const () as usize),
    ] {
        total += 1;
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_render4: replaced {ok}/{total} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut parts = Vec::new();
    for s in [&S_DIST, &S_EYE, &S_OCT] {
        let c = s.total.load(Ordering::Relaxed);
        if c > 0 {
            parts.push(format!("{} {} ({})", s.name, c, s.bad.load(Ordering::Relaxed)));
        }
    }
    for (k, name) in BR_NAMES.iter().enumerate() {
        let c = BR_TOTAL[k].load(Ordering::Relaxed);
        if c > 0 {
            let br: Vec<u32> = BR[k].iter().map(|c| c.load(Ordering::Relaxed)).collect();
            parts.push(format!("{name} {c} (branches {:?})", &br[1..]));
        }
    }
    let c = CONST5_TOTAL.load(Ordering::Relaxed);
    if c > 0 {
        parts.push(format!(
            "gfx_cmd_set_const5_vec3_cached {c} (cached {}, pushed {})",
            CONST5_HIT.load(Ordering::Relaxed),
            CONST5_PUSH.load(Ordering::Relaxed)
        ));
    }
    if parts.is_empty() { String::new() } else { format!("port_render4: {}", parts.join(", ")) }
}

// --- bodies (python tools/body_bytes.py) --------------------------------------------------------

const TREE_A_BODY: [u8; 305] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x81, 0xEC, 0xA4, 0x00, 0x00, 0x00, 0x53, 0x33, 0xC9, 0x39,
    0x0D, 0xC4, 0xFB, 0xF6, 0x00, 0x56, 0x57, 0x89, 0x4C, 0x24, 0x14, 0x0F, 0x8E, 0x09, 0x01, 0x00,
    0x00, 0x8B, 0x34, 0x8D, 0xD8, 0xFB, 0xF6, 0x00, 0x0F, 0xB7, 0x46, 0x08, 0x0F, 0xB7, 0xD0, 0x80,
    0xBA, 0x08, 0x3B, 0xF8, 0x00, 0x00, 0x89, 0x74, 0x24, 0x1C, 0x0F, 0x84, 0xD7, 0x00, 0x00, 0x00,
    0x8B, 0x56, 0x50, 0x85, 0xD2, 0x0F, 0x84, 0xCC, 0x00, 0x00, 0x00, 0x83, 0x7A, 0x08, 0x00, 0x0F,
    0x84, 0xC2, 0x00, 0x00, 0x00, 0x66, 0x89, 0x44, 0x24, 0x22, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00,
    0x66, 0x89, 0x4C, 0x24, 0x20, 0x8B, 0x08, 0x83, 0xC6, 0x40, 0x33, 0xDB, 0xC7, 0x44, 0x24, 0x2C,
    0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x74, 0x24, 0x24, 0x33, 0xFF, 0x89, 0x4C, 0x24, 0x18, 0x8B, 0xFF,
    0x8B, 0x54, 0x24, 0x1C, 0x8B, 0x42, 0x50, 0x8B, 0x0D, 0x98, 0x73, 0x52, 0x02, 0x8B, 0x54, 0x24,
    0x18, 0x8D, 0x34, 0x7F, 0x51, 0xC1, 0xE6, 0x04, 0x03, 0x70, 0x04, 0x8B, 0x42, 0x58, 0x8B, 0x88,
    0x40, 0x07, 0x00, 0x00, 0x68, 0xD0, 0x41, 0x77, 0x02, 0x51, 0x8D, 0x4E, 0x10, 0x8B, 0xD6, 0xE8,
    0x9C, 0x97, 0x07, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x75, 0x0D, 0x85, 0xDB, 0x74, 0x54, 0x8B,
    0x7C, 0x9C, 0x2C, 0x83, 0xEB, 0x01, 0xEB, 0xB8, 0x83, 0x7E, 0x0C, 0x00, 0x76, 0x1A, 0x8D, 0x74,
    0x24, 0x20, 0x89, 0x7C, 0x24, 0x28, 0xE8, 0x75, 0xFE, 0xFF, 0xFF, 0x85, 0xDB, 0x74, 0x34, 0x8B,
    0x7C, 0x9C, 0x2C, 0x83, 0xEB, 0x01, 0xEB, 0x98, 0x8B, 0x46, 0x2C, 0x85, 0xC0, 0x74, 0x1C, 0x8B,
    0x76, 0x28, 0x85, 0xF6, 0x74, 0x0E, 0x89, 0x44, 0x9C, 0x30, 0x83, 0xC3, 0x01, 0x8B, 0xFE, 0xE9,
    0x7C, 0xFF, 0xFF, 0xFF, 0x8B, 0xF8, 0xE9, 0x75, 0xFF, 0xFF, 0xFF, 0x8B, 0x7E, 0x28, 0xE9, 0x6D,
    0xFF, 0xFF, 0xFF, 0x8B, 0x4C, 0x24, 0x14, 0x83, 0xC1, 0x01, 0x3B, 0x0D, 0xC4, 0xFB, 0xF6, 0x00,
    0x89, 0x4C, 0x24, 0x14, 0x0F, 0x8C, 0xF7, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5B, 0x8B, 0xE5, 0x5D,
    0xC3,
];
const TREE_B_BODY: [u8; 339] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x81, 0xEC, 0xA4, 0x00, 0x00, 0x00, 0x53, 0x33, 0xC0, 0x39,
    0x05, 0xC4, 0xFB, 0xF6, 0x00, 0x56, 0x57, 0x89, 0x44, 0x24, 0x18, 0x0F, 0x8E, 0x2B, 0x01, 0x00,
    0x00, 0x8B, 0x34, 0x85, 0xD8, 0xFB, 0xF6, 0x00, 0x0F, 0xB7, 0x4E, 0x08, 0x0F, 0xB7, 0xD1, 0x80,
    0xBA, 0x08, 0x3B, 0xF8, 0x00, 0x00, 0x89, 0x74, 0x24, 0x1C, 0x0F, 0x84, 0xF9, 0x00, 0x00, 0x00,
    0x8B, 0x56, 0x50, 0x33, 0xFF, 0x3B, 0xD7, 0x0F, 0x84, 0xEC, 0x00, 0x00, 0x00, 0x39, 0x7A, 0x08,
    0x0F, 0x84, 0xE3, 0x00, 0x00, 0x00, 0x83, 0xC6, 0x40, 0x66, 0x89, 0x44, 0x24, 0x20, 0x66, 0x89,
    0x4C, 0x24, 0x22, 0x89, 0x7C, 0x24, 0x2C, 0x89, 0x74, 0x24, 0x24, 0x33, 0xDB, 0x8D, 0x49, 0x00,
    0x8B, 0x44, 0x24, 0x1C, 0x8B, 0x48, 0x50, 0x8D, 0x34, 0x7F, 0xC1, 0xE6, 0x04, 0x03, 0x71, 0x04,
    0x80, 0x3D, 0x74, 0x2D, 0x61, 0x02, 0x00, 0x75, 0x5E, 0x8B, 0x15, 0x9C, 0x73, 0x52, 0x02, 0x52,
    0x68, 0xD0, 0x85, 0x77, 0x02, 0x8D, 0x4E, 0x10, 0x68, 0x80, 0x2D, 0x61, 0x02, 0x8B, 0xD6, 0xE8,
    0x6C, 0x96, 0x07, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x75, 0x3C, 0xA1, 0x98, 0x73, 0x52, 0x02,
    0x64, 0x8B, 0x0D, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x11, 0x50, 0x8B, 0x42, 0x58, 0x8B, 0x88, 0x40,
    0x07, 0x00, 0x00, 0x68, 0xD0, 0x41, 0x77, 0x02, 0x51, 0x8D, 0x4E, 0x10, 0x8B, 0xD6, 0xE8, 0x3D,
    0x96, 0x07, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x75, 0x0D, 0x85, 0xDB, 0x74, 0x57, 0x8B, 0x7C,
    0x9C, 0x2C, 0x83, 0xEB, 0x01, 0xEB, 0x89, 0x83, 0x7E, 0x0C, 0x00, 0x76, 0x1D, 0x8D, 0x74, 0x24,
    0x20, 0x89, 0x7C, 0x24, 0x28, 0xE8, 0x16, 0xFD, 0xFF, 0xFF, 0x85, 0xDB, 0x74, 0x37, 0x8B, 0x7C,
    0x9C, 0x2C, 0x83, 0xEB, 0x01, 0xE9, 0x66, 0xFF, 0xFF, 0xFF, 0x8B, 0x46, 0x2C, 0x85, 0xC0, 0x74,
    0x1C, 0x8B, 0x76, 0x28, 0x85, 0xF6, 0x74, 0x0E, 0x89, 0x44, 0x9C, 0x30, 0x83, 0xC3, 0x01, 0x8B,
    0xFE, 0xE9, 0x4A, 0xFF, 0xFF, 0xFF, 0x8B, 0xF8, 0xE9, 0x43, 0xFF, 0xFF, 0xFF, 0x8B, 0x7E, 0x28,
    0xE9, 0x3B, 0xFF, 0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x18, 0x83, 0xC0, 0x01, 0x3B, 0x05, 0xC4, 0xFB,
    0xF6, 0x00, 0x89, 0x44, 0x24, 0x18, 0x0F, 0x8C, 0xD5, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5B, 0x8B,
    0xE5, 0x5D, 0xC3,
];
const TREE_C_BODY: [u8; 334] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x81, 0xEC, 0xA4, 0x00, 0x00, 0x00, 0x53, 0x56, 0x57, 0x33,
    0xFF, 0x33, 0xC9, 0x39, 0x3D, 0xC4, 0xFB, 0xF6, 0x00, 0x89, 0x4C, 0x24, 0x18, 0x0F, 0x8E, 0x24,
    0x01, 0x00, 0x00, 0xEB, 0x0B, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x34, 0x8D, 0xD8, 0xFB, 0xF6, 0x00, 0x0F, 0xB7, 0x46, 0x08, 0x0F, 0xB7, 0xD0, 0x80, 0xBA,
    0x08, 0x3B, 0xF8, 0x00, 0x00, 0x89, 0x74, 0x24, 0x1C, 0x0F, 0x84, 0xE5, 0x00, 0x00, 0x00, 0x8B,
    0x56, 0x50, 0x3B, 0xD7, 0x0F, 0x84, 0xDA, 0x00, 0x00, 0x00, 0x39, 0x7A, 0x08, 0x0F, 0x84, 0xD1,
    0x00, 0x00, 0x00, 0x66, 0x89, 0x44, 0x24, 0x22, 0x8B, 0x45, 0x08, 0x83, 0xC6, 0x40, 0x66, 0x89,
    0x4C, 0x24, 0x20, 0x89, 0x44, 0x24, 0x2C, 0x89, 0x74, 0x24, 0x24, 0x89, 0x7C, 0x24, 0x14, 0x89,
    0x7C, 0x24, 0x10, 0x8B, 0x5C, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x1C, 0x8B, 0x51, 0x50, 0x8B, 0x75,
    0x08, 0x8D, 0x1C, 0x5B, 0xC1, 0xE3, 0x04, 0x03, 0x5A, 0x04, 0x8D, 0x7B, 0x10, 0x53, 0xE8, 0xED,
    0x96, 0x07, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x19, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0,
    0x74, 0x7C, 0x83, 0xE8, 0x01, 0x89, 0x44, 0x24, 0x14, 0x8B, 0x44, 0x84, 0x30, 0x89, 0x44, 0x24,
    0x10, 0xEB, 0xC0, 0x83, 0x7B, 0x0C, 0x00, 0x76, 0x2A, 0x8B, 0x4C, 0x24, 0x10, 0x8D, 0x74, 0x24,
    0x20, 0x89, 0x4C, 0x24, 0x28, 0xE8, 0xD6, 0xFB, 0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0,
    0x74, 0x4C, 0x8B, 0x54, 0x84, 0x2C, 0x83, 0xE8, 0x01, 0x89, 0x44, 0x24, 0x14, 0x89, 0x54, 0x24,
    0x10, 0xEB, 0x90, 0x8B, 0x4B, 0x2C, 0x85, 0xC9, 0x74, 0x28, 0x8B, 0x5B, 0x28, 0x85, 0xDB, 0x74,
    0x18, 0x8B, 0x44, 0x24, 0x14, 0x89, 0x4C, 0x84, 0x30, 0x83, 0xC0, 0x01, 0x89, 0x44, 0x24, 0x14,
    0x89, 0x5C, 0x24, 0x10, 0xE9, 0x6A, 0xFF, 0xFF, 0xFF, 0x89, 0x4C, 0x24, 0x10, 0xE9, 0x61, 0xFF,
    0xFF, 0xFF, 0x8B, 0x43, 0x28, 0x89, 0x44, 0x24, 0x10, 0xE9, 0x55, 0xFF, 0xFF, 0xFF, 0x8B, 0x4C,
    0x24, 0x18, 0x33, 0xFF, 0x83, 0xC1, 0x01, 0x3B, 0x0D, 0xC4, 0xFB, 0xF6, 0x00, 0x89, 0x4C, 0x24,
    0x18, 0x0F, 0x8C, 0xE9, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5B, 0x8B, 0xE5, 0x5D, 0xC3,
];
const DIST87_BODY: [u8; 542] = [
    0x83, 0xEC, 0x18, 0xD9, 0x00, 0x8B, 0x54, 0x24, 0x1C, 0xF3, 0x0F, 0x10, 0x40, 0x04, 0xD9, 0x5C,
    0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xF3, 0x0F, 0x10, 0x1D, 0xB0, 0xBF, 0xE5, 0x00, 0xD9, 0xC0,
    0xF3, 0x0F, 0x11, 0x44, 0x24, 0x08, 0xD8, 0x21, 0xF3, 0x0F, 0x10, 0x40, 0x08, 0xF3, 0x0F, 0x11,
    0x44, 0x24, 0x0C, 0x0F, 0x57, 0xC0, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x02, 0xF3, 0x0F, 0x10, 0x54,
    0x24, 0x1C, 0xD8, 0xE1, 0xD9, 0x1C, 0x24, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x1C, 0xDF, 0xF1, 0x72,
    0x0F, 0x0F, 0x28, 0xCB, 0xF3, 0x0F, 0x5C, 0xCA, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x04, 0xEB, 0x06,
    0xF3, 0x0F, 0x11, 0x44, 0x24, 0x04, 0xF3, 0x0F, 0x10, 0x0C, 0x24, 0x0F, 0x2F, 0xC8, 0x72, 0x08,
    0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0x0F, 0x2F,
    0xD0, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x04, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x5C, 0x24,
    0x04, 0x72, 0x07, 0xF3, 0x0F, 0x11, 0x14, 0x24, 0xEB, 0x05, 0xF3, 0x0F, 0x11, 0x04, 0x24, 0x0F,
    0x2F, 0xC8, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44,
    0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x04, 0x24, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x08, 0xD9, 0xC0, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x42, 0x04, 0xF3, 0x0F, 0x10,
    0x54, 0x24, 0x1C, 0xD8, 0xE1, 0xD9, 0x1C, 0x24, 0xD9, 0x44, 0x24, 0x1C, 0xDF, 0xF2, 0x72, 0x0F,
    0x0F, 0x28, 0xCB, 0xF3, 0x0F, 0x5C, 0xCA, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x08, 0xEB, 0x06, 0xF3,
    0x0F, 0x11, 0x44, 0x24, 0x08, 0xF3, 0x0F, 0x10, 0x0C, 0x24, 0x0F, 0x2F, 0xC8, 0x72, 0x08, 0xF3,
    0x0F, 0x11, 0x4C, 0x24, 0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0x0F, 0x2F, 0xD0,
    0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x08, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x08, 0x72, 0x07,
    0xF3, 0x0F, 0x11, 0x14, 0x24, 0xEB, 0x05, 0xF3, 0x0F, 0x11, 0x04, 0x24, 0x0F, 0x2F, 0xC8, 0x72,
    0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0xD9,
    0x44, 0x24, 0x1C, 0xD8, 0x04, 0x24, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0,
    0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x42, 0x08, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x1C,
    0xD8, 0xE1, 0xD9, 0x1C, 0x24, 0xD9, 0x44, 0x24, 0x1C, 0xDF, 0xF2, 0xDD, 0xD9, 0x72, 0x0C, 0xF3,
    0x0F, 0x5C, 0xDA, 0xF3, 0x0F, 0x11, 0x5C, 0x24, 0x0C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24,
    0x0C, 0xF3, 0x0F, 0x10, 0x0C, 0x24, 0x0F, 0x2F, 0xC8, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24,
    0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0x0F, 0x2F, 0xD0, 0xD9, 0x44, 0x24, 0x1C,
    0xD8, 0x44, 0x24, 0x0C, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x0C, 0x72, 0x07, 0xF3, 0x0F, 0x11, 0x14,
    0x24, 0xEB, 0x05, 0xF3, 0x0F, 0x11, 0x04, 0x24, 0x0F, 0x2F, 0xC8, 0x72, 0x08, 0xF3, 0x0F, 0x11,
    0x4C, 0x24, 0x1C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xF3,
    0x0F, 0x10, 0x44, 0x24, 0x04, 0xD8, 0x04, 0x24, 0xF3, 0x0F, 0x11, 0x06, 0xF3, 0x0F, 0x10, 0x44,
    0x24, 0x08, 0xF3, 0x0F, 0x11, 0x46, 0x04, 0xD9, 0x5C, 0x24, 0x1C, 0xF3, 0x0F, 0x10, 0x44, 0x24,
    0x0C, 0xD9, 0x44, 0x24, 0x10, 0xF3, 0x0F, 0x11, 0x46, 0x08, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x44,
    0x24, 0x1C, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8,
    0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0x83, 0xC4, 0x18, 0xC3,
];
const DIST87_CALLS: [usize; 0] = [];
const GLARE_BODY: [u8; 834] = [
    0x83, 0xEC, 0x48, 0x53, 0x8A, 0x5C, 0x24, 0x54, 0x55, 0x8B, 0x6C, 0x24, 0x54, 0x80, 0xBD, 0x80,
    0x00, 0x00, 0x00, 0x00, 0x56, 0x57, 0x8B, 0xF0, 0x74, 0x59, 0x0F, 0xB7, 0x85, 0x82, 0x00, 0x00,
    0x00, 0x8B, 0x04, 0x85, 0xA8, 0x49, 0xF8, 0x00, 0xC7, 0x44, 0x24, 0x5C, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x00, 0x85, 0xC0, 0x74, 0x44, 0x8B, 0x08, 0x6A, 0x00, 0x6A, 0x04, 0x8D, 0x54, 0x24, 0x64,
    0x52, 0x50, 0x8B, 0x41, 0x1C, 0xFF, 0xD0, 0x83, 0xF8, 0x01, 0x74, 0x2E, 0x85, 0xC0, 0xC6, 0x85,
    0x80, 0x00, 0x00, 0x00, 0x00, 0x75, 0x1C, 0x8B, 0x44, 0x24, 0x5C, 0x3D, 0x10, 0x27, 0x00, 0x00,
    0x77, 0x18, 0x3B, 0x05, 0x2C, 0x6D, 0x52, 0x02, 0x0F, 0x97, 0xC1, 0x88, 0x8D, 0x81, 0x00, 0x00,
    0x00, 0xEB, 0x07, 0xC6, 0x85, 0x81, 0x00, 0x00, 0x00, 0x00, 0x80, 0x7C, 0x24, 0x64, 0x00, 0x0F,
    0x84, 0xB5, 0x02, 0x00, 0x00, 0x80, 0xBD, 0x80, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xA8, 0x02,
    0x00, 0x00, 0x84, 0xDB, 0xC6, 0x85, 0x80, 0x00, 0x00, 0x00, 0x01, 0x74, 0x0B, 0x8B, 0x55, 0x0C,
    0x8B, 0x45, 0x10, 0x8B, 0x4D, 0x14, 0xEB, 0x29, 0xD9, 0x06, 0xD8, 0x65, 0x0C, 0xD9, 0x5C, 0x24,
    0x1C, 0x8B, 0x54, 0x24, 0x1C, 0xD9, 0x46, 0x04, 0xD8, 0x65, 0x10, 0xD9, 0x5C, 0x24, 0x20, 0x8B,
    0x44, 0x24, 0x20, 0xD9, 0x46, 0x08, 0xD8, 0x65, 0x14, 0xD9, 0x5C, 0x24, 0x24, 0x8B, 0x4C, 0x24,
    0x24, 0x89, 0x44, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0x89, 0x54, 0x24, 0x10, 0xD9, 0x44, 0x24,
    0x10, 0x89, 0x4C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0xC1, 0x51, 0xDE, 0xCA, 0xD9, 0xC2,
    0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x68, 0xD9, 0x44,
    0x24, 0x68, 0xD9, 0x1C, 0x24, 0xE8, 0x96, 0x97, 0xF4, 0xFF, 0xD9, 0x5C, 0x24, 0x68, 0x83, 0xC4,
    0x04, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x64, 0xDD, 0xE1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x44,
    0x7B, 0x2A, 0xD9, 0xE8, 0xDE, 0xF1, 0xD9, 0x5C, 0x24, 0x64, 0xD9, 0x44, 0x24, 0x64, 0xD9, 0xC0,
    0xD8, 0x4C, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xC9, 0xD9, 0x5C,
    0x24, 0x14, 0xD8, 0x4C, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x18, 0xEB, 0x1F, 0x8B, 0x15, 0xD0, 0xA6,
    0x52, 0x02, 0xDD, 0xD8, 0xA1, 0xD4, 0xA6, 0x52, 0x02, 0x8B, 0x0D, 0xD8, 0xA6, 0x52, 0x02, 0x89,
    0x54, 0x24, 0x10, 0x89, 0x44, 0x24, 0x14, 0x89, 0x4C, 0x24, 0x18, 0x84, 0xDB, 0xD9, 0x44, 0x24,
    0x10, 0x74, 0x56, 0xDD, 0x05, 0xD8, 0xC0, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24,
    0x1C, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x20, 0xD8, 0x4C, 0x24, 0x18, 0xD9,
    0x5C, 0x24, 0x24, 0xD9, 0x45, 0x0C, 0xD8, 0x64, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x28, 0x8B, 0x7C,
    0x24, 0x28, 0xD9, 0x45, 0x10, 0xD8, 0x64, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x2C, 0x8B, 0x5C, 0x24,
    0x2C, 0xD9, 0x45, 0x14, 0xD8, 0x64, 0x24, 0x24, 0xD9, 0x5C, 0x24, 0x30, 0x8B, 0x54, 0x24, 0x30,
    0x89, 0x54, 0x24, 0x18, 0xE9, 0xA4, 0x00, 0x00, 0x00, 0x66, 0xF7, 0x45, 0x3E, 0x00, 0x40, 0x74,
    0x4B, 0xDD, 0x05, 0xE0, 0xCA, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x28, 0xD9,
    0x44, 0x24, 0x14, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x2C, 0xD8, 0x4C, 0x24, 0x18, 0xD9, 0x5C, 0x24,
    0x30, 0xD9, 0x44, 0x24, 0x28, 0xD8, 0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x45, 0x10, 0xD8,
    0x44, 0x24, 0x2C, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x45, 0x14, 0xD8, 0x44, 0x24, 0x30, 0xD9, 0x5C,
    0x24, 0x24, 0x8B, 0x44, 0x24, 0x24, 0x89, 0x44, 0x24, 0x18, 0xEB, 0x49, 0xDD, 0x05, 0xF8, 0xC0,
    0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xC9,
    0xD9, 0x5C, 0x24, 0x2C, 0xD8, 0x4C, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x28,
    0xD8, 0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x45, 0x10, 0xD8, 0x44, 0x24, 0x2C, 0xD9, 0x5C,
    0x24, 0x20, 0xD9, 0x45, 0x14, 0xD8, 0x44, 0x24, 0x30, 0xD9, 0x5C, 0x24, 0x24, 0x8B, 0x4C, 0x24,
    0x24, 0x89, 0x4C, 0x24, 0x18, 0x8B, 0x5C, 0x24, 0x20, 0x8B, 0x7C, 0x24, 0x1C, 0x8D, 0x74, 0x24,
    0x34, 0x89, 0x7C, 0x24, 0x10, 0x89, 0x5C, 0x24, 0x14, 0xE8, 0x42, 0xE4, 0x85, 0x00, 0xD9, 0x40,
    0x18, 0xDD, 0x05, 0x80, 0xC1, 0xE5, 0x00, 0x83, 0xC0, 0x18, 0xDC, 0xC9, 0x8D, 0x74, 0x24, 0x28,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x40, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x2C, 0xD8,
    0x48, 0x08, 0xD9, 0x5C, 0x24, 0x30, 0xE8, 0xC5, 0x8B, 0x71, 0x00, 0x8B, 0xC6, 0x8D, 0x4C, 0x24,
    0x34, 0xE8, 0xEA, 0xF6, 0x71, 0x00, 0xD9, 0xE8, 0x51, 0xD9, 0x1C, 0x24, 0x8D, 0x74, 0x24, 0x14,
    0xE8, 0x3B, 0xD8, 0x85, 0x00, 0xD9, 0xE1, 0x8B, 0x0D, 0x38, 0x6D, 0x52, 0x02, 0xD9, 0x5C, 0x24,
    0x68, 0xD9, 0x44, 0x24, 0x68, 0x8B, 0x54, 0x24, 0x1C, 0x8D, 0x04, 0xCD, 0x00, 0x00, 0x00, 0x00,
    0xD9, 0x5C, 0x24, 0x68, 0xD9, 0x44, 0x24, 0x68, 0x2B, 0xC1, 0x8D, 0x04, 0xC5, 0x58, 0x85, 0x64,
    0x02, 0x83, 0xC1, 0x01, 0xD9, 0x58, 0x0C, 0x89, 0x38, 0x89, 0x0D, 0x38, 0x6D, 0x52, 0x02, 0x89,
    0x58, 0x04, 0x8D, 0x78, 0x10, 0xB9, 0x09, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x38, 0x83, 0xC4,
    0x04, 0x89, 0x50, 0x08, 0xF3, 0xA5, 0xF6, 0x45, 0x3E, 0x04, 0x74, 0x0D, 0x33, 0xC9, 0x89, 0x48,
    0x34, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x48, 0xC3, 0x0F, 0xB7, 0x8D, 0x82, 0x00, 0x00, 0x00,
    0x8B, 0x0C, 0x8D, 0xA8, 0x49, 0xF8, 0x00, 0x89, 0x48, 0x34, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4,
    0x48, 0xC3,
];
const PWRITE_BODY: [u8; 174] = [
    0x83, 0xEC, 0x18, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x24, 0x56, 0x57, 0x33, 0xFF, 0x66, 0x39, 0x7D,
    0x08, 0x89, 0x7C, 0x24, 0x10, 0x89, 0x7C, 0x24, 0x14, 0x0F, 0x86, 0x87, 0x00, 0x00, 0x00, 0x8B,
    0x48, 0x0C, 0x0F, 0xB7, 0x45, 0x0A, 0x99, 0x89, 0x4C, 0x24, 0x2C, 0x89, 0x44, 0x24, 0x18, 0x8B,
    0xDA, 0xEB, 0x02, 0x33, 0xFF, 0x85, 0xDB, 0x8B, 0x55, 0x0C, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x0C,
    0x82, 0x8B, 0x54, 0x24, 0x2C, 0x8B, 0x74, 0x24, 0x30, 0x8D, 0x0C, 0x8A, 0x89, 0x7C, 0x24, 0x20,
    0x72, 0x33, 0x77, 0x07, 0x83, 0x7C, 0x24, 0x18, 0x00, 0x76, 0x2A, 0xD9, 0x06, 0x83, 0xC1, 0x04,
    0xD9, 0x59, 0xFC, 0x0F, 0xB7, 0x45, 0x0A, 0x83, 0xC6, 0x04, 0x83, 0x44, 0x24, 0x20, 0x01, 0x99,
    0x83, 0xD7, 0x00, 0x8B, 0xDA, 0x3B, 0xFB, 0x89, 0x44, 0x24, 0x18, 0x72, 0xDE, 0x77, 0x06, 0x39,
    0x44, 0x24, 0x20, 0x72, 0xD6, 0x83, 0x44, 0x24, 0x10, 0x01, 0x0F, 0xB7, 0x45, 0x08, 0x8B, 0x4C,
    0x24, 0x14, 0x83, 0xD1, 0x00, 0x99, 0x3B, 0xCA, 0x89, 0x4C, 0x24, 0x14, 0x72, 0x95, 0x77, 0x06,
    0x39, 0x44, 0x24, 0x10, 0x72, 0x8D, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x18, 0xC3,
];
const PBYID_BODY: [u8; 58] = [
    0x51, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x14, 0x57, 0x33, 0xFF, 0x39, 0x3E, 0x76, 0x27, 0x33, 0xDB,
    0x8B, 0x46, 0x04, 0x03, 0xC3, 0x39, 0x28, 0x75, 0x12, 0x8B, 0x4C, 0x24, 0x1C, 0x51, 0x50, 0x8B,
    0x44, 0x24, 0x1C, 0xE8, 0x68, 0xF1, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x83, 0xC7, 0x01, 0x83, 0xC3,
    0x10, 0x3B, 0x3E, 0x72, 0xDB, 0x5F, 0x5D, 0x5B, 0x59, 0xC3,
];
const PSET_BODY: [u8; 111] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x10, 0x85, 0xFF, 0x53, 0x8B, 0x5D, 0x0C, 0x56,
    0x8B, 0xF0, 0x75, 0x17, 0x8B, 0x45, 0x10, 0x8B, 0x4D, 0x08, 0x50, 0x53, 0x51, 0xE8, 0x2E, 0xFF,
    0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x5E, 0x5B, 0x8B, 0xE5, 0x5D, 0xC3, 0x8B, 0x4E, 0x04, 0x8B, 0x36,
    0xC1, 0xE6, 0x04, 0x03, 0xF1, 0x8D, 0x54, 0x24, 0x08, 0x52, 0x8B, 0xC6, 0x89, 0x7C, 0x24, 0x10,
    0x89, 0x5C, 0x24, 0x0C, 0xE8, 0x97, 0xAE, 0xEB, 0xFF, 0x83, 0xC4, 0x04, 0x3B, 0xC6, 0x74, 0x19,
    0x39, 0x78, 0x04, 0x75, 0x14, 0x39, 0x18, 0x75, 0x10, 0x8B, 0x4D, 0x10, 0x51, 0x50, 0x8B, 0x45,
    0x08, 0xE8, 0x7A, 0xF0, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0x5B, 0x8B, 0xE5, 0x5D, 0xC3,
];
const EYE_BODY: [u8; 94] = [
    0x83, 0xEC, 0x10, 0xE8, 0x88, 0xFD, 0xFF, 0xFF, 0xD9, 0x00, 0xD8, 0x26, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x40, 0x04, 0xD8, 0x66, 0x04, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x40, 0x08, 0xD8, 0x66, 0x08,
    0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x47, 0x04, 0xD8, 0x4C, 0x24, 0x08, 0xD9, 0x07, 0xD8, 0x4C, 0x24,
    0x04, 0xDE, 0xC1, 0xD9, 0x47, 0x08, 0xD8, 0x4C, 0x24, 0x0C, 0xDE, 0xC1, 0xD9, 0x1C, 0x24, 0xD9,
    0x04, 0x24, 0xD8, 0x1D, 0xC0, 0x32, 0xDD, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x09, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0x83, 0xC4, 0x10, 0xC3, 0x33, 0xC0, 0x83, 0xC4, 0x10, 0xC3,
];
const EYE_CALLS: [usize; 1] = [0x3];
const OCT_BODY: [u8; 165] = [
    0xF3, 0x0F, 0x10, 0x0A, 0x0F, 0x57, 0xC0, 0x8B, 0x4C, 0x24, 0x04, 0x0F, 0x2F, 0xC8, 0xF3, 0x0F,
    0x10, 0x4A, 0x04, 0x76, 0x48, 0x0F, 0x2F, 0xC8, 0xF3, 0x0F, 0x10, 0x4A, 0x08, 0x76, 0x1F, 0x0F,
    0x2F, 0xC8, 0x76, 0x0D, 0xC7, 0x00, 0x04, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x02, 0x00, 0x00, 0x00,
    0xC3, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x06, 0x00, 0x00, 0x00, 0xC3, 0x0F, 0x2F,
    0xC8, 0x76, 0x0D, 0xC7, 0x00, 0x05, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x03, 0x00, 0x00, 0x00, 0xC3,
    0xC7, 0x00, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x07, 0x00, 0x00, 0x00, 0xC3, 0x0F, 0x2F, 0xC8,
    0xF3, 0x0F, 0x10, 0x4A, 0x08, 0x76, 0x1F, 0x0F, 0x2F, 0xC8, 0x76, 0x0D, 0xC7, 0x00, 0x07, 0x00,
    0x00, 0x00, 0xC7, 0x01, 0x01, 0x00, 0x00, 0x00, 0xC3, 0xC7, 0x00, 0x03, 0x00, 0x00, 0x00, 0xC7,
    0x01, 0x05, 0x00, 0x00, 0x00, 0xC3, 0x0F, 0x2F, 0xC8, 0x76, 0x0D, 0xC7, 0x00, 0x06, 0x00, 0x00,
    0x00, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x00, 0xC3, 0xC7, 0x00, 0x02, 0x00, 0x00, 0x00, 0xC7, 0x01,
    0x04, 0x00, 0x00, 0x00, 0xC3,
];
const OCT_CALLS: [usize; 0] = [];
const CONST5_BODY: [u8; 135] = [
    0x8B, 0x08, 0x3B, 0x0D, 0xAC, 0x4D, 0x30, 0x02, 0x75, 0x16, 0x8B, 0x50, 0x04, 0x3B, 0x15, 0xB0,
    0x4D, 0x30, 0x02, 0x75, 0x0B, 0x8B, 0x48, 0x08, 0x3B, 0x0D, 0xB4, 0x4D, 0x30, 0x02, 0x74, 0x66,
    0xD9, 0x00, 0xD9, 0x1D, 0xAC, 0x4D, 0x30, 0x02, 0xD9, 0x40, 0x04, 0xD9, 0x1D, 0xB0, 0x4D, 0x30,
    0x02, 0xD9, 0x40, 0x08, 0xA1, 0xF8, 0x62, 0x3D, 0x03, 0xD9, 0x1D, 0xB4, 0x4D, 0x30, 0x02, 0xD9,
    0xE8, 0xD9, 0x1D, 0xB8, 0x4D, 0x30, 0x02, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0xC7, 0x40, 0x04,
    0x05, 0x00, 0x00, 0x00, 0xC7, 0x40, 0x08, 0x01, 0x00, 0x00, 0x00, 0x8B, 0x15, 0xAC, 0x4D, 0x30,
    0x02, 0x89, 0x50, 0x0C, 0x8B, 0x0D, 0xB0, 0x4D, 0x30, 0x02, 0x89, 0x48, 0x10, 0x8B, 0x15, 0xB4,
    0x4D, 0x30, 0x02, 0x89, 0x50, 0x14, 0x8B, 0x0D, 0xB8, 0x4D, 0x30, 0x02, 0x89, 0x48, 0x18, 0x83,
    0x05, 0xF8, 0x62, 0x3D, 0x03, 0x1C, 0xC3,
];

// --- offline differential tests -----------------------------------------------------------------
//
// The test process maps windows of the unpacked exe (code, constants and the globals the originals
// read) at their real addresses, so the ORIGINAL bytes run in place; the callees with side effects
// (`frustum_test_aabb`, `view_bounds_visible`, the tree leaf `0x4B6970`, the D3D query, the camera
// state helpers, `shader_param_write_floats` for its callers) are replaced by recording stubs. Each
// case builds a random world, runs the original, restores the world, runs the Rust thunk and
// compares: EAX where it is a result, ECX / EDX where the original keeps them, the callee-saved
// registers, XMM0 (where it is a result), every XMM register the original leaves alone, ST0 and the
// x87 depth, all world memory and the stream of calls the code made.
//
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" //   cargo test --release -p sr2-hook --target i686-pc-windows-msvc port_render4 -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

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
        fn one_in(&mut self, n: u32) -> bool {
            self.below(n) == 0
        }
        fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[self.below(v.len() as u32) as usize]
        }
    }

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    /// A float bit pattern with the awkward cases over-represented.
    fn fbits(rng: &mut Rng, scale: f32) -> u32 {
        match rng.below(20) {
            0 => 0x7FC0_0000,
            1 => 0x7F80_0001,
            2 => 0x7F80_0000,
            3 => 0xFF80_0000,
            4 => 0,
            5 => 0x8000_0000,
            6 => 0x0000_0001,
            7 => rng.u32(),
            8 | 9 => ((rng.below(9) as f32) - 4.0).to_bits(),
            _ => (((rng.below(20001) as f32) - 10000.0) / 10000.0 * scale).to_bits(),
        }
    }

    // --- the exe image ------------------------------------------------------------------------------

    static EXE: std::sync::OnceLock<(Vec<u8>, Vec<[usize; 4]>)> = std::sync::OnceLock::new();

    fn exe() -> &'static (Vec<u8>, Vec<[usize; 4]>) {
        EXE.get_or_init(|| {
            let exe = std::fs::read("F:/Saints Row 2 Decomp/bin/SR2_pc.unpacked.exe").unwrap();
            let rd32 = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
            let pe = rd32(0x3C);
            let nsec = u16::from_le_bytes(exe[pe + 6..pe + 8].try_into().unwrap()) as usize;
            let opt = u16::from_le_bytes(exe[pe + 20..pe + 22].try_into().unwrap()) as usize;
            let secs = (0..nsec)
                .map(|i| {
                    let s = pe + 24 + opt + i * 40;
                    [0x40_0000 + rd32(s + 12), rd32(s + 8), rd32(s + 20), rd32(s + 16)]
                })
                .collect();
            (exe, secs)
        })
    }

    fn exe_bytes(va: usize, len: usize) -> Vec<u8> {
        let (exe, secs) = exe();
        (va..va + len)
            .map(|a| {
                for &[sva, vsz, raw, rsz] in secs {
                    if a >= sva && a < sva + vsz.max(rsz) {
                        return if a - sva < rsz { exe[raw + a - sva] } else { 0 };
                    }
                }
                0
            })
            .collect()
    }

    fn map_window(lo: usize, hi: usize) {
        let (lo, hi) = (lo & !0xFFF, hi.div_ceil(0x1000) * 0x1000);
        let mut blk = lo & !0xFFFF;
        while blk < hi {
            let _ = unsafe { VirtualAlloc(blk as *const c_void, 0x1_0000, MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
            let (s, e) = (blk.max(lo), (blk + 0x1_0000).min(hi));
            let c = unsafe { VirtualAlloc(s as *const c_void, e - s, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
            if c as usize != s {
                let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                let mut q = unsafe { std::mem::zeroed::<windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION>() };
                unsafe { windows_sys::Win32::System::Memory::VirtualQuery(s as *const c_void, &mut q, std::mem::size_of_val(&q)) };
                eprintln!("cannot map {s:#x}..{e:#x}: error {err}; base {:#x} alloc {:#x} size {:#x} state {:#x} prot {:#x} type {:#x}", q.BaseAddress as usize, q.AllocationBase as usize, q.RegionSize, q.State, q.AllocationProtect, q.Type);
                blk += 0x1_0000;
                continue;
            }
            blk += 0x1_0000;
        }
        let bytes = exe_bytes(lo, hi - lo);
        let mut q = unsafe { std::mem::zeroed::<windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION>() };
        let ok = unsafe { windows_sys::Win32::System::Memory::VirtualQuery(lo as *const c_void, &mut q, std::mem::size_of_val(&q)) };
        if ok == 0 || q.State != windows_sys::Win32::System::Memory::MEM_COMMIT {
            return;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), lo as *mut u8, bytes.len()) };
    }

    // --- stubs --------------------------------------------------------------------------------------
    //
    // The callees with side effects or with state the test cannot supply are replaced by recording
    // stubs (they log what they were called with and clobber the registers the real ones clobber);
    // `lower_bound_key64_rec16` and `shader_param_set_by_id` run as the original bytes.

    static mut LOG: Vec<[u32; 7]> = Vec::new();
    static mut STUB_RNG: u64 = 1;
    static mut GFX_STATE_PTR: u32 = 0;

    const STATE_MAT: usize = 0x252_A100; // 9 words the state-copy stub hands out

    fn stub_rand() -> u32 {
        unsafe {
            let r = &raw mut STUB_RNG;
            let mut g = Rng(*r);
            let v = g.u32();
            *r = g.0;
            v
        }
    }

    fn log_push(e: [u32; 7]) {
        unsafe { (*(&raw mut LOG)).push(e) }
    }

    extern "C" fn frustum_stub_rs(ecx: u32, edx: u32, a0: u32, a1: u32, a2: u32) -> u32 {
        log_push([1, ecx, edx, a0, a1, a2, 0]);
        match stub_rand() % 4 {
            0 => 0,
            1 => 1,
            2 => 0x100,
            _ => stub_rand(),
        }
    }

    /// cdecl (planes, cells, count), ECX / EDX inputs; clobbers ECX / EDX with garbage.
    #[unsafe(naked)]
    extern "C" fn frustum_stub() {
        std::arch::naked_asm!(
            "push dword ptr [esp + 12]",
            "push dword ptr [esp + 12]",
            "push dword ptr [esp + 12]",
            "push edx",
            "push ecx",
            "call {f}",
            "add esp, 20",
            "mov ecx, 0x0BADC0DE",
            "mov edx, 0x0BADD00D",
            "ret",
            f = sym frustum_stub_rs,
        )
    }

    extern "C" fn vbv_stub_rs(esi: u32, edi: u32, rec: u32) -> u32 {
        log_push([2, esi, edi, rec, 0, 0, 0]);
        match stub_rand() % 3 {
            0 => 0,
            1 => 1,
            _ => stub_rand(),
        }
    }

    #[unsafe(naked)]
    extern "C" fn vbv_stub() {
        std::arch::naked_asm!(
            "push dword ptr [esp + 4]",
            "push edi",
            "push esi",
            "call {f}",
            "add esp, 12",
            "mov ecx, 0x0BADC0DE",
            "mov edx, 0x0BADD00D",
            "ret",
            f = sym vbv_stub_rs,
        )
    }

    /// 0x4B6970: ESI = work item, EAX logged (the original stores it); keeps ECX.
    extern "C" fn leaf_stub_rs(esi: u32, eax: u32) {
        unsafe {
            let w = |k: usize| rd(esi as usize + k * 4);
            log_push([4, w(0), w(1), w(2), w(3), eax, 0]);
        }
    }

    #[unsafe(naked)]
    extern "C" fn leaf_stub() {
        std::arch::naked_asm!(
            "push ecx",
            "push eax",
            "push esi",
            "call {f}",
            "add esp, 8",
            "pop ecx",
            "mov eax, 0x0BADC0DE",
            "mov edx, 0x0BADD00D",
            "ret",
            f = sym leaf_stub_rs,
        )
    }

    /// The D3D query's `GetData` (stdcall).
    extern "system" fn getdata_stub(this: u32, p: *mut u32, size: u32, flags: u32) -> i32 {
        log_push([5, this, size, flags, 0, 0, 0]);
        unsafe {
            *p = match stub_rand() % 6 {
                0 => 0,
                1 => 0x2710,
                2 => 0x2711,
                3 => stub_rand() % 64,
                4 => stub_rand() % 0x4000,
                _ => stub_rand(),
            };
        }
        match stub_rand() % 6 {
            0 | 1 | 2 => 0,
            3 => 1,
            4 => 2,
            _ => 0x8876_086Cu32 as i32,
        }
    }

    extern "C" fn sqrt_stub(x: f32) -> f32 {
        log_push([6, x.to_bits(), 0, 0, 0, 0, 0]);
        (x as f64).sqrt() as f32
    }

    extern "C" fn state_copy_rs(buf: u32) {
        log_push([7, 0, 0, 0, 0, 0, 0]);
        unsafe {
            for k in 0..9usize {
                wd(buf as usize + k * 4, rd(STATE_MAT + k * 4));
            }
        }
    }

    /// 0xD23840: ESI = 9-dword buffer, returns EAX = ESI.
    #[unsafe(naked)]
    extern "C" fn state_copy_stub() {
        std::arch::naked_asm!("push esi", "push esi", "call {f}", "add esp, 4", "pop eax", "ret", f = sym state_copy_rs)
    }

    extern "C" fn normalize_rs(v: u32) {
        unsafe {
            let w = |k: usize| rd(v as usize + k * 4);
            log_push([8, w(0), w(1), w(2), 0, 0, 0]);
            for k in 0..3usize {
                wd(v as usize + k * 4, w(k) ^ (0x00FF_0000 >> (k * 4)));
            }
        }
    }

    /// 0xBDDFF0: ESI = vec3.
    #[unsafe(naked)]
    extern "C" fn normalize_stub() {
        std::arch::naked_asm!("push esi", "call {f}", "add esp, 4", "ret", f = sym normalize_rs)
    }

    extern "C" fn basis_rs(w: u32, mat: u32) {
        unsafe {
            let x = |k: usize| rd(w as usize + k * 4);
            log_push([9, x(0), x(1), x(2), 0, 0, 0]);
            for k in 0..9usize {
                wd(mat as usize + k * 4, x(k % 3).wrapping_add(k as u32 * 0x0101_0101));
            }
        }
    }

    /// 0xBE4B20: EAX = &forward, ECX = &matrix out.
    #[unsafe(naked)]
    extern "C" fn basis_stub() {
        std::arch::naked_asm!("push ecx", "push eax", "call {f}", "add esp, 8", "ret", f = sym basis_rs)
    }

    extern "C" fn project_rs(v: u32, arg: u32) -> f32 {
        unsafe {
            let w = |k: usize| rd(v as usize + k * 4);
            log_push([10, w(0), w(1), w(2), arg, 0, 0]);
            let f = |b: u32| f32::from_bits(b);
            f(w(0)) * 0.5 - f(w(1)) + f(arg) * f(w(2))
        }
    }

    /// 0xD22C80: ESI = vec3, stack f32 -> ST0.
    #[unsafe(naked)]
    extern "C" fn project_stub() {
        std::arch::naked_asm!("push dword ptr [esp + 4]", "push esi", "call {f}", "add esp, 8", "ret", f = sym project_rs)
    }

    extern "C" fn gfx_state_rs() -> u32 {
        log_push([11, 0, 0, 0, 0, 0, 0]);
        unsafe { *(&raw const GFX_STATE_PTR) }
    }

    /// 0xD22790 (keeps ESI as the Rust function does).
    #[unsafe(naked)]
    extern "C" fn gfx_state_stub() {
        std::arch::naked_asm!("call {f}", "ret", f = sym gfx_state_rs)
    }

    extern "C" fn pwrite_stub_rs(eax: u32, e: u32, src: u32) {
        log_push([12, eax, e, src, 0, 0, 0]);
    }

    /// 0xD19E30 as a recording stub: EAX = block, stack (entry, src).
    #[unsafe(naked)]
    extern "C" fn pwrite_stub() {
        std::arch::naked_asm!(
            "push dword ptr [esp + 8]",
            "push dword ptr [esp + 8]",
            "push eax",
            "call {f}",
            "add esp, 12",
            "mov ecx, 0x0BADC0DE",
            "mov edx, 0x0BADD00D",
            "ret",
            f = sym pwrite_stub_rs,
        )
    }

    pub(super) fn sqrt_stub_addr() -> usize {
        sqrt_stub as *const () as usize
    }

    fn patch_call(at: usize, to: usize) {
        let rel = (to as i32).wrapping_sub((at + 5) as i32);
        unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (at + 1) as *mut u8, 4) };
    }

    fn patch_jmp(at: usize, to: usize) {
        let rel = (to as i32).wrapping_sub((at + 5) as i32);
        unsafe {
            *(at as *mut u8) = 0xE9;
            std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (at + 1) as *mut u8, 4);
        }
    }

    static STUB_PW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// With `STUB_PW` set, 0xD19E30 is replaced by the recording stub (for the tests of its callers).
    fn setup() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            for (lo, hi) in [
                (0x4B_4000, 0x4B_8000),
                (0x4C_5000, 0x4C_6000),
                (0x53_0000, 0x53_1000),
                (0xBD_5000, 0xBD_6000),
                (0xBD_D000, 0xBD_E000),
                (0xBE_4000, 0xBE_5000),
                (0xBF_6000, 0xBF_7000),
                (0xD1_8000, 0xD1_B000),
                (0xD2_2000, 0xD2_4000),
                (0xDD_3000, 0xDD_4000),
                (0xE5_B000, 0xE5_D000),
                (0xF6_F000, 0xF7_0000),
                (0xF8_3000, 0xF8_5000),
                (0x230_4000, 0x230_5000),
                (0x252_6000, 0x252_8000),
                (0x252_A000, 0x252_B000),
                (0x261_2000, 0x261_6000),
                (0x264_8000, 0x264_9000),
                (0x33D_6000, 0x33D_7000),
            ] {
                map_window(lo, hi);
            }
            patch_jmp(FRUSTUM_TEST_AABB, frustum_stub as *const () as usize);
            patch_jmp(VIEW_BOUNDS_VISIBLE, vbv_stub as *const () as usize);
            patch_jmp(TREE_LEAF, leaf_stub as *const () as usize);
            // the original's `call 0x40EA20` (the wrapper's page cannot be mapped here)
            patch_call(GLARE_UPDATE + 0x105, sqrt_stub as *const () as usize);
            patch_jmp(THREAD_STATE_COPY, state_copy_stub as *const () as usize);
            patch_jmp(VEC3_NORMALIZE, normalize_stub as *const () as usize);
            patch_jmp(MAT33_FROM_FORWARD, basis_stub as *const () as usize);
            patch_jmp(GLARE_PROJECT, project_stub as *const () as usize);
            patch_jmp(GFX_STATE, gfx_state_stub as *const () as usize);
            if STUB_PW.load(Ordering::Relaxed) {
                patch_jmp(PARAM_WRITE_FLOATS, pwrite_stub as *const () as usize);
            }
        });
    }

    fn g(addr: usize, v: u32) {
        unsafe { *(addr as *mut u32) = v }
    }
    fn gb(addr: usize, v: u8) {
        unsafe { *(addr as *mut u8) = v }
    }

    // --- the probe: one call with every register set and read back -------------------------------------

    #[repr(C, align(16))]
    #[derive(Clone)]
    struct Probe {
        f: u32,
        eax: u32,
        ecx: u32,
        edx: u32,
        ebx: u32,
        esi: u32,
        edi: u32,
        ebp: u32,
        nargs: u32,
        args: u32,
        esp: u32,
        want_st0: u32,
        fsw0: u32,
        fsw1: u32,
        pad: [u32; 2],
        st0: f64,
        pad2: [u32; 2],
        xin: [[u32; 4]; 8],
        xout: [[u32; 4]; 8],
    }

    static mut FNP: usize = 0;
    static mut BLK: usize = 0;

    unsafe fn call_probe(p: &mut Probe, args: &[u32]) {
        p.nargs = args.len() as u32;
        p.args = args.as_ptr() as u32;
        unsafe {
            *(&raw mut FNP) = p.f as usize;
            *(&raw mut BLK) = p as *mut Probe as usize;
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov dword ptr [eax + 40], esp",
                "fnstsw word ptr [eax + 48]",
                "movups xmm0, xmmword ptr [eax + 80]",
                "movups xmm1, xmmword ptr [eax + 96]",
                "movups xmm2, xmmword ptr [eax + 112]",
                "movups xmm3, xmmword ptr [eax + 128]",
                "movups xmm4, xmmword ptr [eax + 144]",
                "movups xmm5, xmmword ptr [eax + 160]",
                "movups xmm6, xmmword ptr [eax + 176]",
                "movups xmm7, xmmword ptr [eax + 192]",
                "mov ecx, dword ptr [eax + 32]",
                "mov esi, dword ptr [eax + 36]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "mov ecx, dword ptr [eax + 8]",
                "mov edx, dword ptr [eax + 12]",
                "mov ebx, dword ptr [eax + 16]",
                "mov esi, dword ptr [eax + 20]",
                "mov edi, dword ptr [eax + 24]",
                "mov ebp, dword ptr [eax + 28]",
                "mov eax, dword ptr [eax + 4]",
                "call dword ptr [{fnp}]",
                "push eax",
                "mov eax, dword ptr [{blk}]",
                "pop dword ptr [eax + 4]",
                "mov dword ptr [eax + 8], ecx",
                "mov dword ptr [eax + 12], edx",
                "mov dword ptr [eax + 16], ebx",
                "mov dword ptr [eax + 20], esi",
                "mov dword ptr [eax + 24], edi",
                "mov dword ptr [eax + 28], ebp",
                "fnstsw word ptr [eax + 52]",
                "cmp dword ptr [eax + 44], 0",
                "je 4f",
                "fstp qword ptr [eax + 64]",
                "4:",
                "movups xmmword ptr [eax + 208], xmm0",
                "movups xmmword ptr [eax + 224], xmm1",
                "movups xmmword ptr [eax + 240], xmm2",
                "movups xmmword ptr [eax + 256], xmm3",
                "movups xmmword ptr [eax + 272], xmm4",
                "movups xmmword ptr [eax + 288], xmm5",
                "movups xmmword ptr [eax + 304], xmm6",
                "movups xmmword ptr [eax + 320], xmm7",
                "mov esp, dword ptr [eax + 40]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                fnp = sym FNP,
                blk = sym BLK,
                in("eax") p as *mut Probe,
                clobber_abi("C"),
            );
        }
    }

    /// Runs `f` with the thread's TLS array replaced by `arr` (slot 0 = the state block).
    unsafe fn with_tls<R>(arr: *mut u32, f: impl FnOnce() -> R) -> R {
        unsafe {
            let teb: usize;
            std::arch::asm!("mov {}, fs:[0x18]", out(reg) teb);
            let slot = (teb + 0x2C) as *mut usize;
            let old = *slot;
            *slot = arr as usize;
            let r = f();
            *slot = old;
            r
        }
    }

    // --- the world ------------------------------------------------------------------------------------

    /// Heap area the objects live in, plus the TLS fakes.
    struct World {
        heap: Vec<u128>,
        tls_arr: Vec<u32>,
    }

    impl World {
        fn new(size: usize) -> World {
            World { heap: vec![0u128; size / 16], tls_arr: vec![0u32; 8] }
        }
        fn base(&self) -> usize {
            self.heap.as_ptr() as usize
        }
        fn len(&self) -> usize {
            self.heap.len() * 16
        }
        fn fill_random(&mut self, rng: &mut Rng) {
            let s = unsafe { std::slice::from_raw_parts_mut(self.base() as *mut u32, self.len() / 4) };
            for w in s.iter_mut() {
                *w = rng.u32();
            }
        }
        fn put(&self, off: usize, v: u32) {
            unsafe { *((self.base() + off) as *mut u32) = v }
        }
        fn put16(&self, off: usize, v: u16) {
            unsafe { *((self.base() + off) as *mut u16) = v }
        }
        fn put8(&self, off: usize, v: u8) {
            unsafe { *((self.base() + off) as *mut u8) = v }
        }
        fn at(&self, off: usize) -> u32 {
            (self.base() + off) as u32
        }
        /// Everything the checks compare: the heap and the globals pages the code may touch.
        fn snapshot(&self, extra: &[(usize, usize)]) -> Vec<u8> {
            let mut v = unsafe { std::slice::from_raw_parts(self.base() as *const u8, self.len()).to_vec() };
            for &(a, l) in extra {
                v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, l) });
            }
            v
        }
        fn restore(&self, snap: &[u8], extra: &[(usize, usize)]) {
            unsafe { std::ptr::copy_nonoverlapping(snap.as_ptr(), self.base() as *mut u8, self.len()) };
            let mut at = self.len();
            for &(a, l) in extra {
                unsafe { std::ptr::copy_nonoverlapping(snap[at..at + l].as_ptr(), a as *mut u8, l) };
                at += l;
            }
        }
    }

    #[derive(Clone, Copy, Default)]
    struct Opts {
        eax: bool,
        /// Compare the low byte only (callers test AL).
        al_only: bool,
        keep_ecx: bool,
        cmp_ecx: bool,
        keep_edx: bool,
        xmm0: bool,
        st0: bool,
        tls: bool,
    }

    struct Case {
        /// Registers on entry (ebx / ebp are sentinels).
        eax: u32,
        ecx: u32,
        edx: u32,
        esi: u32,
        edi: u32,
        args: Vec<u32>,
    }

    fn xmm_sentinel(rng: &mut Rng) -> [[u32; 4]; 8] {
        let mut x = [[0u32; 4]; 8];
        for r in x.iter_mut() {
            for l in r.iter_mut() {
                *l = rng.u32();
            }
        }
        x
    }

    fn run_one(w: &World, orig: usize, thunk: usize, c: &Case, o: Opts, seed: u64, xin: [[u32; 4]; 8], extra: &[(usize, usize)]) -> Result<(), String> {
        let snap = w.snapshot(extra);
        let mk = |f: usize| Probe {
            f: f as u32,
            eax: c.eax,
            ecx: c.ecx,
            edx: c.edx,
            ebx: 0xB1B1_B1B1,
            esi: c.esi,
            edi: c.edi,
            ebp: 0xE9E9_E9E9,
            nargs: 0,
            args: 0,
            esp: 0,
            want_st0: o.st0 as u32,
            fsw0: 0,
            fsw1: 0,
            pad: [0; 2],
            st0: 0.0,
            pad2: [0; 2],
            xin,
            xout: [[0; 4]; 8],
        };
        let run = |f: usize| -> (Probe, Vec<[u32; 7]>, Vec<u8>) {
            unsafe {
                *(&raw mut STUB_RNG) = seed | 1;
                (*(&raw mut LOG)).clear();
            }
            let mut p = mk(f);
            if o.tls {
                unsafe { with_tls(w.tls_arr.as_ptr() as *mut u32, || call_probe(&mut p, &c.args)) };
            } else {
                unsafe { call_probe(&mut p, &c.args) };
            }
            let log = unsafe { (*(&raw mut LOG)).clone() };
            (p, log, w.snapshot(extra))
        };
        let (po, lo, mo) = run(orig);
        w.restore(&snap, extra);
        let (pr, lr, mr) = run(thunk);
        let top = |fsw: u32| (fsw >> 11) & 7;
        if top(po.fsw1) != top(pr.fsw1) {
            return Err(format!("x87 TOP: original {} rust {}", top(po.fsw1), top(pr.fsw1)));
        }
        if o.eax {
            let (a, b) = if o.al_only { (po.eax & 0xFF, pr.eax & 0xFF) } else { (po.eax, pr.eax) };
            if a != b {
                return Err(format!("eax: original {a:#x} rust {b:#x}"));
            }
        }
        if o.keep_ecx && (pr.ecx != c.ecx || po.ecx != c.ecx) {
            return Err(format!("ecx: in {:#x} original {:#x} rust {:#x}", c.ecx, po.ecx, pr.ecx));
        }
        if o.cmp_ecx && pr.ecx != po.ecx {
            return Err(format!("ecx: original {:#x} rust {:#x}", po.ecx, pr.ecx));
        }
        if o.keep_edx && pr.edx != c.edx {
            return Err(format!("edx: in {:#x} rust {:#x}", c.edx, pr.edx));
        }
        for (n, x, y, z) in [("ebx", po.ebx, pr.ebx, 0xB1B1_B1B1u32), ("ebp", po.ebp, pr.ebp, 0xE9E9_E9E9)] {
            if x != z || y != z {
                return Err(format!("{n}: in {z:#x} original {x:#x} rust {y:#x}"));
            }
        }
        for (n, x, y, z) in [("esi", po.esi, pr.esi, c.esi), ("edi", po.edi, pr.edi, c.edi)] {
            if x != z || y != z {
                return Err(format!("{n}: in {z:#x} original {x:#x} rust {y:#x}"));
            }
        }
        if o.st0 {
            let same = po.st0.to_bits() == pr.st0.to_bits() || (po.st0.is_nan() && pr.st0.is_nan());
            if !same {
                return Err(format!("st0: original {:?} ({:#x}) rust {:?} ({:#x})", po.st0, po.st0.to_bits(), pr.st0, pr.st0.to_bits()));
            }
        }
        for k in 0..8 {
            if po.xout[k] == xin[k] && pr.xout[k] != xin[k] {
                return Err(format!("xmm{k} was kept by the original and changed by the port"));
            }
            if k == 0 && o.xmm0 {
                let f = |b: u32| f32::from_bits(b);
                let same = po.xout[0][0] == pr.xout[0][0] || (f(po.xout[0][0]).is_nan() && f(pr.xout[0][0]).is_nan());
                if !same || po.xout[0][1..] != pr.xout[0][1..] {
                    return Err(format!("xmm0: original {:x?} rust {:x?}", po.xout[0], pr.xout[0]));
                }
            }
        }
        if lo != lr {
            let i = lo.iter().zip(&lr).position(|(a, b)| a != b).unwrap_or(lo.len().min(lr.len()));
            return Err(format!("calls differ at #{i} (original {} calls, rust {}): original {:x?} rust {:x?}", lo.len(), lr.len(), lo.get(i), lr.get(i)));
        }
        if let Some(i) = mo.iter().zip(&mr).position(|(a, b)| a != b) {
            return Err(format!("memory differs at snapshot offset {i:#x}: original {:#x} rust {:#x}", mo[i], mr[i]));
        }
        Ok(())
    }

    /// Floats are compared by value for the matrix test (NaN payloads of results may differ).
    fn run_cases(name: &str, mut f: impl FnMut(&mut Rng, &mut World) -> (Case, Opts, Vec<(usize, usize)>), size: usize, orig: usize, thunk: usize) {
        setup();
        let mut rng = Rng(0x1234_5678_9ABC_DEF1 ^ (name.len() as u64) << 20);
        let mut w = World::new(size);
        let n = cases();
        let mut bad = 0;
        for i in 0..n {
            let (c, o, extra) = f(&mut rng, &mut w);
            let xin = xmm_sentinel(&mut rng);
            if let Err(e) = run_one(&w, orig, thunk, &c, o, rng.u32() as u64, xin, &extra) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name} case {i}: regs {:#x} {:#x} {:#x} {:#x} {:#x} args {:x?}: {e}", c.eax, c.ecx, c.edx, c.esi, c.edi, c.args);
                }
            }
        }
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }


    // --- helpers for the worlds -------------------------------------------------------------------------

    fn sane_float(rng: &mut Rng, scale: f32) -> u32 {
        (((rng.below(20001) as f32) - 10000.0) / 10000.0 * scale).to_bits()
    }

    fn mark_pw() {
        STUB_PW.store(true, Ordering::Relaxed);
    }

    // --- the tree walks ---------------------------------------------------------------------------------

    const TLSBLK: usize = 0x5000;
    const VSTATE: usize = 0x5100;
    const CH: usize = 0x100; // chunks, 0x80 bytes each
    const TR: usize = 0x1000; // trees, 0x40 bytes each
    const ND: usize = 0x2000; // node arrays, 0x600 bytes per tree (up to 32 nodes of 0x30)

    fn tree_world(rng: &mut Rng, w: &mut World) {
        w.fill_random(rng);
        let n: u32 = match rng.below(10) {
            0 => 0,
            1 => 0x8000_0000 | rng.u32(),
            _ => 1 + rng.below(6),
        };
        g(CHUNK_COUNT, n);
        for id in 0..64usize {
            gb(CHUNK_FLAGS + id, if rng.one_in(4) { 0 } else { rng.u32() as u8 | 1 });
        }
        g(PLANE_COUNT, rng.u32());
        g(SHADOW_PLANE_COUNT, rng.u32());
        gb(SHADOW_FLAG, if rng.one_in(3) { rng.u32() as u8 | 1 } else { 0 });
        w.put(TLSBLK + 0x58, w.at(VSTATE));
        w.put(VSTATE + 0x740, rng.u32());
        w.tls_arr[0] = w.at(TLSBLK);
        for i in 0..6usize {
            g(CHUNK_PTRS + i * 4, w.at(CH + i * 0x80));
            let c = CH + i * 0x80;
            w.put16(c + 8, rng.below(64) as u16);
            w.put(c + 0x50, if rng.one_in(6) { 0 } else { w.at(TR + i * 0x40) });
            let t = TR + i * 0x40;
            w.put(t + 4, w.at(ND + i * 0x600));
            w.put(t + 8, if rng.one_in(8) { 0 } else { rng.u32() | 1 });
            let nn = 1 + rng.below(32) as usize;
            // a real tree: every node is somebody's child at most once
            let mut free: Vec<u32> = (1..nn as u32).collect();
            for j in 0..nn {
                let nd = ND + i * 0x600 + j * 0x30;
                let avail: Vec<u32> = free.iter().copied().filter(|&c| c > j as u32).collect();
                let leaf = avail.is_empty() || rng.one_in(3);
                w.put(nd + 0xC, if leaf { rng.u32() | 1 } else { 0 });
                if !leaf {
                    let take = |rng: &mut Rng, free: &mut Vec<u32>| -> u32 {
                        let cands: Vec<usize> = (0..free.len()).filter(|&k| free[k] > j as u32).collect();
                        if cands.is_empty() {
                            return 0;
                        }
                        free.remove(cands[rng.below(cands.len() as u32) as usize])
                    };
                    let a = take(rng, &mut free);
                    let b = if rng.one_in(2) { take(rng, &mut free) } else { 0 };
                    // right == 0 needs a left child; (left, right) = (a, b) or (b, a)
                    let (left, right) = if b == 0 { if rng.one_in(2) { (a, 0) } else { (0, a) } } else if rng.one_in(2) { (a, b) } else { (b, a) };
                    w.put(nd + 0x28, left);
                    w.put(nd + 0x2C, right);
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn tree_a_matches_original() {
        run_cases(
            "city_chunk_tree_cull_frustum",
            |rng, w| {
                tree_world(rng, w);
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![] }, Opts { tls: true, ..Opts::default() }, vec![])
            },
            0x6000,
            TREE_FRUSTUM,
            tree_a_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn tree_b_matches_original() {
        run_cases(
            "city_chunk_tree_cull_frustum_dual",
            |rng, w| {
                tree_world(rng, w);
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![] }, Opts { tls: true, ..Opts::default() }, vec![])
            },
            0x6000,
            TREE_DUAL,
            tree_b_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn tree_c_matches_original() {
        run_cases(
            "city_chunk_tree_cull_view_bounds",
            |rng, w| {
                tree_world(rng, w);
                let view = if rng.one_in(2) { rng.below(8) } else { rng.u32() };
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![view] }, Opts { tls: true, ..Opts::default() }, vec![])
            },
            0x6000,
            TREE_VIEW,
            tree_c_thunk as *const () as usize,
        );
    }

    // --- aabb_point_dist_sq_x87 ---------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn dist87_matches_original() {
        run_cases(
            "aabb_point_dist_sq_x87",
            |rng, w| {
                w.fill_random(rng);
                for base in [0x100usize, 0x140, 0x180] {
                    for k in 0..4usize {
                        let v = fbits(rng, 20.0);
                        w.put(base + k * 4, v);
                    }
                }
                if rng.below(3) != 0 {
                    // a sane box around the point
                    for k in 0..3usize {
                        let lo = ((rng.below(200) as f32) - 100.0) / 4.0;
                        let hi = lo + (rng.below(60) as f32) / 3.0;
                        let p = lo + (rng.below(80) as f32 - 10.0) / 3.0;
                        w.put(0x140 + k * 4, lo.to_bits());
                        w.put(0x180 + k * 4, hi.to_bits());
                        w.put(0x100 + k * 4, p.to_bits());
                    }
                }
                let out = match rng.below(8) {
                    0 => w.at(0x100),
                    1 => w.at(0x140),
                    2 => w.at(0x180),
                    3 => w.at(0x104),
                    _ => w.at(0x200),
                };
                (
                    Case { eax: w.at(0x100), ecx: w.at(0x140), edx: rng.u32(), esi: out, edi: 0xD1D1_D1D1, args: vec![w.at(0x180)] },
                    Opts { st0: true, ..Opts::default() },
                    vec![],
                )
            },
            0x1000,
            DIST_X87,
            dist_thunk as *const () as usize,
        );
    }

    // --- glare_occlusion_query_update -----------------------------------------------------------------------

    const REC: usize = 0x100;
    const CAM: usize = 0x200;
    const SLOTS: usize = 0x300;
    const OBJS: usize = 0x600;
    const VT: usize = 0x500;

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn glare_matches_original() {
        run_cases(
            "glare_occlusion_query_update",
            |rng, w| {
                w.fill_random(rng);
                w.put(VT + 7 * 4, getdata_stub as *const () as u32);
                for id in 0..64usize {
                    g(QUERY_TABLE + id * 4, w.at(SLOTS + id * 4));
                    w.put(SLOTS + id * 4, if rng.one_in(5) { 0 } else { w.at(OBJS + id * 8) });
                    w.put(OBJS + id * 8, w.at(VT));
                }
                g(QUERY_PIXELS_MIN, match rng.below(4) {
                    0 => 0,
                    1 => 0x2710,
                    2 => rng.below(64),
                    _ => rng.u32() >> rng.below(24),
                });
                g(GLARE_COUNT, rng.below(8));
                for k in 0..3usize {
                    g(GLARE_FALLBACK + k * 4, fbits(rng, 3.0));
                }
                for k in 0..9usize {
                    g(STATE_MAT + k * 4, fbits(rng, 3.0));
                }
                for k in 0..3usize {
                    let (c, p) = (fbits(rng, 50.0), fbits(rng, 50.0));
                    w.put(CAM + k * 4, c);
                    w.put(REC + 0xC + k * 4, p);
                }
                match rng.below(8) {
                    0 => {
                        for k in 0..3usize {
                            w.put(REC + 0xC + k * 4, 0);
                        }
                    }
                    1 => {
                        for k in 0..3usize {
                            let c = unsafe { *((w.base() + CAM + k * 4) as *const u32) };
                            w.put(REC + 0xC + k * 4, c);
                        }
                    }
                    2 => {
                        for k in 0..3usize {
                            let v = sane_float(rng, 100.0);
                            w.put(REC + 0xC + k * 4, v);
                        }
                    }
                    _ => {}
                }
                w.put8(REC + 0x80, rng.pick(&[0u8, 0, 1, 1, 1, 2, 0xFF]));
                let idn = if rng.one_in(8) { 64 } else { 8 };
                w.put16(REC + 0x82, rng.below(idn) as u16);
                let f1: u16 = if rng.one_in(2) { rng.u32() as u16 } else { 0 };
                let f2: u16 = if rng.one_in(2) { 0x4000 } else { 0 };
                w.put16(REC + 0x3E, f1 | f2);
                let a2 = if rng.one_in(2) { rng.below(2) } else { rng.u32() };
                let a3 = if rng.one_in(5) { rng.u32() & !0xFF } else if rng.one_in(2) { 1 } else { rng.u32() };
                (
                    Case { eax: w.at(CAM), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![w.at(REC), a2, a3] },
                    Opts::default(),
                    vec![(GLARE_COUNT, 4), (GLARE_ENTRIES, 0x38 * 10)],
                )
            },
            0x1000,
            GLARE_UPDATE,
            glare_thunk as *const () as usize,
        );
    }

    // --- shader_param_write_floats --------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn pwrite_matches_original() {
        run_cases(
            "shader_param_write_floats",
            |rng, w| {
                w.fill_random(rng);
                const BLK: usize = 0x100;
                const E: usize = 0x200;
                const TBL: usize = 0x300;
                const DEST: usize = 0x4000;
                const SRC: usize = 0x6000;
                w.put(BLK + 0xC, w.at(DEST));
                w.put16(E + 8, if rng.one_in(6) { 0 } else { rng.below(6) as u16 });
                w.put16(E + 0xA, if rng.one_in(6) { 0 } else { rng.below(9) as u16 });
                w.put(E + 0xC, w.at(TBL));
                for k in 0..8usize {
                    w.put(TBL + k * 4, rng.below(0x400));
                }
                for k in 0..10usize {
                    let v = fbits(rng, 10.0);
                    w.put(SRC + k * 4, v);
                }
                (
                    Case { eax: w.at(BLK), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![w.at(E), w.at(SRC)] },
                    Opts::default(),
                    vec![],
                )
            },
            0x8000,
            PARAM_WRITE_FLOATS,
            pwrite_thunk as *const () as usize,
        );
    }

    // --- shader_param_set_by_id and shader_param_set (0xD19E30 as a recording stub) ----------------------------

    fn param_list_world(rng: &mut Rng, w: &mut World) {
        w.fill_random(rng);
        const BLK: usize = 0x100;
        const ENT: usize = 0x200;
        let n = if rng.one_in(6) { 0 } else { rng.below(8) };
        w.put(BLK, n);
        w.put(BLK + 4, w.at(ENT));
        let sorted = !rng.one_in(4);
        let mut keys: Vec<(u32, u32)> = (0..n).map(|_| (rng.below(3), rng.below(5))).collect();
        if sorted {
            keys.sort();
        }
        for (k, &(hi, lo)) in keys.iter().enumerate() {
            w.put(ENT + k * 16, lo);
            w.put(ENT + k * 16 + 4, hi);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn pbyid_matches_original() {
        mark_pw();
        run_cases(
            "shader_param_set_by_id",
            |rng, w| {
                param_list_world(rng, w);
                // ids compare against the first dword of each record (the low key part)
                let id = rng.below(6);
                (
                    Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: w.at(0x100), edi: 0xD1D1_D1D1, args: vec![w.at(0x40), id, w.at(0x60)] },
                    Opts::default(),
                    vec![],
                )
            },
            0x1000,
            PARAM_SET_BY_ID,
            pbyid_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn pset_matches_original() {
        mark_pw();
        run_cases(
            "shader_param_set",
            |rng, w| {
                param_list_world(rng, w);
                let flag = if rng.one_in(3) { 0 } else { rng.below(4) };
                let b = rng.below(6);
                (
                    Case { eax: w.at(0x100), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: flag, args: vec![w.at(0x40), b, w.at(0x60)] },
                    Opts::default(),
                    vec![],
                )
            },
            0x1000,
            PARAM_SET,
            pset_thunk as *const () as usize,
        );
    }

    // --- gfx_eye_facing_test ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn eye_matches_original() {
        run_cases(
            "gfx_eye_facing_test",
            |rng, w| {
                w.fill_random(rng);
                for k in 0..3usize {
                    let (a, b, c) = (fbits(rng, 20.0), fbits(rng, 20.0), fbits(rng, 5.0));
                    w.put(0x400 + k * 4, a);
                    w.put(0x100 + k * 4, b);
                    w.put(0x200 + k * 4, c);
                }
                unsafe { *(&raw mut GFX_STATE_PTR) = w.at(0x400) };
                g(EYE_FACING_MIN, match rng.below(4) {
                    0 | 1 => 0,
                    2 => 0x7FC0_0000,
                    _ => sane_float(rng, 4.0),
                });
                (
                    Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: w.at(0x100), edi: w.at(0x200), args: vec![] },
                    Opts { eax: true, ..Opts::default() },
                    vec![(EYE_FACING_MIN, 4)],
                )
            },
            0x1000,
            EYE_FACING,
            eye_thunk as *const () as usize,
        );
    }

    // --- vec3_sign_octant ------------------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn octant_matches_original() {
        run_cases(
            "vec3_sign_octant",
            |rng, w| {
                w.fill_random(rng);
                for k in 0..3usize {
                    let v = fbits(rng, 5.0);
                    w.put(0x100 + k * 4, v);
                }
                let b = if rng.one_in(8) { w.at(0x200) } else { w.at(0x204) };
                (
                    Case { eax: w.at(0x200), ecx: rng.u32(), edx: w.at(0x100), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![b] },
                    Opts { eax: true, keep_edx: true, ..Opts::default() },
                    vec![],
                )
            },
            0x400,
            SIGN_OCTANT,
            octant_thunk as *const () as usize,
        );
    }

    // --- gfx_cmd_set_const5_vec3_cached ---------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn const5_matches_original() {
        run_cases(
            "gfx_cmd_set_const5_vec3_cached",
            |rng, w| {
                w.fill_random(rng);
                let mut v = [0u32; 3];
                for k in 0..3usize {
                    v[k] = fbits(rng, 5.0);
                    w.put(0x100 + k * 4, v[k]);
                }
                let mode = rng.below(5);
                for k in 0..4usize {
                    let same = mode == 0 || (mode == 1 && k != 1) || (mode == 2 && k != 2);
                    g(CONST5_CACHE + k * 4, if k < 3 && same { v[k] } else { rng.u32() });
                }
                g(CMD_CURSOR, w.at(0x400 + 4 * rng.below(16) as usize));
                (
                    Case { eax: w.at(0x100), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![] },
                    Opts::default(),
                    vec![(CONST5_CACHE, 16), (CMD_CURSOR, 4)],
                )
            },
            0x1000,
            SET_CONST5,
            const5_thunk as *const () as usize,
        );
    }
}
