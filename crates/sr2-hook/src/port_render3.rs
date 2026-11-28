//! Render-thread hot paths from the tss02 mission log (doc 58): the city chunk instance cull
//! callbacks (`0x4B5CE0`, `0x4B5C40`, `0x4B5FF0`, `0x4B54A0`), the animated texture entry resolve
//! (`0xC094E0`), the render item sort compare (`0x5240F0`) and a few math leaves (`0xBF63B0`,
//! `0xBE61B0`, `0xA90B10`) and `0x530520`.
//!
//! Checks (see `install`):
//! - Pure functions (`texture_entry_animated_resolve`, `render_item_less`,
//!   `city_view_instance_filter_test`, `mesh_material_flags_or`, `aabb_point_dist_sq_sse`,
//!   `plane_eval_xyz_ptrs`): relocated-original shadow for the first `SHADOW_CALLS` calls, the
//!   original's result kept on a mismatch.
//! - `mat44_mul_sse` (writes 64 bytes): snapshot shadow (save the output, run the original, save
//!   its result, restore, run Rust, compare; the original's bytes win on a mismatch).
//! - The three cull callbacks write the visibility bytes / words through callees with side effects
//!   (`frustum_test_aabb`, the interlocked add) from several job threads: per-branch counters for
//!   the first `SHADOW_CALLS` calls only.
//! - All of them are also checked offline against the original bytes (tests at the bottom).
//!
//! Every thunk keeps ECX / EDX / XMM0-7 as the caller left them where the original does not touch
//! them (callers of LTCG leaves may hold live values there); XMM0 carries the result of
//! `aabb_point_dist_sq_sse`, ST0 that of `plane_eval_xyz_ptrs`.
//!
//! Deviations from the docs: `0x4B5FF0` takes the object pointer at `rec+0x48` when it is non-zero
//! AND `rec+0x30 & 0x10000` is SET (doc 58 says clear); `0xBF65F0` (`aabb_point_dist_sq_x87`)
//! also writes three floats through ESI and was not ported.

#![allow(unused_imports)]

use std::arch::x86::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const CITY_CULL_SPLIT: usize = 0x004B_5CE0;
const CITY_CULL: usize = 0x004B_5C40;
const CITY_VIEW_CULL: usize = 0x004B_5FF0;
const CITY_FILTER: usize = 0x004B_54A0;
const TEXTURE_RESOLVE: usize = 0x00C0_94E0;
const ITEM_LESS: usize = 0x0052_40F0;
const AABB_DIST_SSE: usize = 0x00BF_63B0;
const MAT44_MUL: usize = 0x00BE_61B0;
const PLANE_EVAL: usize = 0x00A9_0B10;
const MESH_FLAGS_OR: usize = 0x0053_0520;

const FRUSTUM_TEST_AABB: usize = 0x0053_0270;
const VIEW_BOUNDS_VISIBLE: usize = 0x0053_0450;
const FTOL2_SSE: usize = 0x00DA_0140;

const SSE3_FLAG: usize = 0x00E8_E88A; // byte: the haddps variants are selected
const CULL_RANGE: usize = 0x00E8_499C; // float: squared on use
const DIST_MASK_SSE3: usize = 0x00E9_96F0; // 16-byte lane mask (w cleared)
const DIST_MASK: usize = 0x00E8_3E50; // 16-byte lane mask (w cleared)
const PLANE_COUNT: usize = 0x0252_7398;
const SHADOW_PLANE_COUNT: usize = 0x0252_739C;
const SHADOW_FLAG: usize = 0x0261_2D74; // byte
const SHADOW_PLANES: u32 = 0x0261_2D80;
const SHADOW_CELLS: u32 = 0x0277_85D0;
const VIEW_CELLS: u32 = 0x0277_41D0;
const VIEW_RECORDS: usize = 0x0261_2D10; // 0x280 bytes each
const INTERLOCKED_ADD_SLOT: usize = 0x00DB_809C;
const TEX_COUNT: usize = 0x0234_8908;
const TEX_TABLE: usize = 0x0234_890C;
const TEX_TICK: usize = 0x0252_A3C8;
const SSE2_FLAG: usize = 0x034A_536C;
/// `fmul qword [0xE5BFE0]`: 0.001f as a double.
const RATE_SCALE: f64 = 0.001f32 as f64;
/// `fadd dword [0xE5BFD8]`.
const TWO_32: f64 = 4294967296.0;

// --- small helpers ----------------------------------------------------------------------------------

#[inline(always)]
unsafe fn rh(p: usize) -> u32 {
    unsafe { *(p as *const u16) as u32 }
}

/// `fs:[0x2c]` -> slot 0: the render / job thread's state block.
#[inline(always)]
unsafe fn thread_state() -> usize {
    let tls: usize;
    unsafe {
        std::arch::asm!("mov {}, fs:[0x2c]", out(reg) tls, options(nostack, readonly, preserves_flags));
        rd(tls) as usize
    }
}

/// The game's stdcall interlocked add `(int *p, int v)` through its import slot.
#[inline(always)]
unsafe fn xadd(p: usize, v: u32) {
    unsafe {
        std::arch::asm!(
            "push {v}",
            "push {p}",
            "call dword ptr [{s}]",
            v = in(reg) v,
            p = in(reg) p,
            s = in(reg) INTERLOCKED_ADD_SLOT,
            clobber_abi("C"),
        );
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

/// `aabb_point_dist_sq_sse` by address: ECX = &max, EDX = &point, stack (&min) -> XMM0.x as bits.
#[inline(always)]
unsafe fn dist_call(f: usize, max: usize, point: usize, min: usize) -> f32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push {m}",
            "call {f}",
            "add esp, 4",
            "movd eax, xmm0",
            m = in(reg) min,
            f = in(reg) f,
            in("ecx") max,
            in("edx") point,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    f32::from_bits(r)
}

/// `_ftol2_sse`: truncate to int32, 0x80000000 for NaN / out of range. The SSE2 path inline, the
/// x87 path (flag clear) through the original.
#[inline(always)]
unsafe fn ftol2(d: f64) -> i32 {
    unsafe {
        if rd(SSE2_FLAG) != 0 {
            _mm_cvttsd_si32(_mm_set_sd(d))
        } else {
            let r: i32;
            std::arch::asm!(
                "fld qword ptr [{p}]",
                "call {f}",
                p = in(reg) &d as *const f64,
                f = in(reg) FTOL2_SSE,
                lateout("eax") r,
                clobber_abi("C"),
            );
            r
        }
    }
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

static S_TEX: Shadow = Shadow::new("texture_entry_animated_resolve");
static S_LESS: Shadow = Shadow::new("render_item_less");
static S_FILTER: Shadow = Shadow::new("city_view_instance_filter_test");
static S_FLAGS: Shadow = Shadow::new("mesh_material_flags_or");
static S_DIST: Shadow = Shadow::new("aabb_point_dist_sq_sse");
static S_PLANE: Shadow = Shadow::new("plane_eval_xyz_ptrs");
static S_MAT: Shadow = Shadow::new("mat44_mul_sse");

/// Branch counters of the cull callbacks, first `SHADOW_CALLS` calls only:
/// 0 calls, 1 instances, 2 skipped by flags, 3 frustum passed, 4 second test run, 5 add 0x10000,
/// 6 distance add, 7 filtered (view cull), 8 not visible (view cull), 9 view add.
static CULL_SEEN: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static CULL_BRANCH: [[AtomicU32; 10]; 3] = [const { [const { AtomicU32::new(0) }; 10] }; 3];

#[inline(always)]
fn bump(which: usize, i: usize) {
    if CULL_SEEN[which].load(Ordering::Relaxed) < SHADOW_CALLS {
        CULL_BRANCH[which][i].fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(always)]
fn seen(which: usize) {
    let c = CULL_SEEN[which].load(Ordering::Relaxed);
    if c < SHADOW_CALLS {
        CULL_SEEN[which].store(c + 1, Ordering::Relaxed);
        CULL_BRANCH[which][0].fetch_add(1, Ordering::Relaxed);
    }
}

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

// =================================================================================================
// 0x4B5CE0 city_cull_chunk_instances_split  (cdecl (entry*), plain ret)
// =================================================================================================

/// `entry`: +4 ctx, +8 chunk node index. ctx: +0xC records (0x50 each), +0x10 chunk set, +0x18
/// visibility array (8 bytes per record: dword at +0 counts views, byte at +6 = in frustum).
unsafe fn split_impl(entry: usize) {
    unsafe {
        let ctx0 = rd(entry + 4) as usize;
        let set = rd(ctx0 + 0x10) as usize;
        let node = (rd(entry + 8) as usize).wrapping_mul(0x30).wrapping_add(rd(set + 4) as usize);
        let list = (rd(set + 0xC) as usize).wrapping_add((rd(node + 0x1C) as usize).wrapping_mul(2));
        let sse = rb(SSE3_FLAG) != 0;
        if sse {
            let th = thread_state();
            let t = rf(CULL_RANGE);
            let thresh2 = t * t;
            let eye = (rd(th + 0x58) as usize).wrapping_add(0x710);
            if rd(node + 0xC) == 0 {
                return;
            }
            let mask = _mm_load_ps(DIST_MASK_SSE3 as *const f32);
            let mut i = 0usize;
            loop {
                let idx = rh(list + i * 2) as usize;
                let ctx = rd(entry + 4) as usize;
                let rec = idx.wrapping_mul(0x50).wrapping_add(rd(ctx + 0xC) as usize);
                let f30 = rd(rec + 0x30);
                let f34 = rd(rec + 0x34);
                bump(0, 1);
                if (f30 & 0x8000) | (f34 & 0x40) == 0 {
                    let cnt = rd(PLANE_COUNT);
                    let planes = rd(rd(th + 0x58) as usize + 0x740);
                    let r = frustum(rec + 0x10, rec, planes, VIEW_CELLS, cnt);
                    let vis = (rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8);
                    wb(vis + 6, r as u8);
                    if r as u8 != 0 {
                        bump(0, 3);
                    }
                    if f34 & 0x200 == 0 {
                        let go = if rb(SHADOW_FLAG) == 0 {
                            bump(0, 4);
                            frustum(rec + 0x10, rec, SHADOW_PLANES, SHADOW_CELLS, rd(SHADOW_PLANE_COUNT)) as u8 != 0
                        } else {
                            true
                        };
                        if go {
                            bump(0, 5);
                            xadd((rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8), 0x10000);
                        }
                    }
                    let lo = _mm_loadu_ps(rec as *const f32);
                    let hi = _mm_loadu_ps((rec + 0x10) as *const f32);
                    let pt = _mm_loadu_ps(eye as *const f32);
                    let zero = _mm_setzero_ps();
                    let a = _mm_max_ps(_mm_sub_ps(lo, pt), zero);
                    let b = _mm_max_ps(_mm_sub_ps(pt, hi), zero);
                    let d = _mm_add_ps(a, b);
                    let sq = _mm_and_ps(_mm_mul_ps(d, d), mask);
                    // haddps twice: ((x2 + y2) + (z2 + w2))
                    let p = _mm_add_ps(sq, _mm_shuffle_ps::<0xB1>(sq, sq));
                    let q = _mm_add_ps(p, _mm_movehl_ps(p, p));
                    let dist = _mm_cvtss_f32(q);
                    // fcompp thresh2, dist; test ah,1: add unless thresh2 < dist or unordered
                    if dist <= thresh2 {
                        bump(0, 6);
                        xadd((rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8), 1);
                    }
                } else {
                    bump(0, 2);
                }
                i += 1;
                if i >= rd(node + 0xC) as usize {
                    break;
                }
            }
        } else {
            if rd(node + 0xC) == 0 {
                return;
            }
            let mut i = 0usize;
            loop {
                let idx = rh(list + i * 2) as usize;
                let ctx = rd(entry + 4) as usize;
                let rec = idx.wrapping_mul(0x50).wrapping_add(rd(ctx + 0xC) as usize);
                let f30 = rd(rec + 0x30);
                let f34 = rd(rec + 0x34);
                bump(0, 1);
                if (f30 & 0x8000) | (f34 & 0x40) == 0 {
                    let th = thread_state();
                    let cnt = rd(PLANE_COUNT);
                    let planes = rd(rd(th + 0x58) as usize + 0x740);
                    let r = frustum(rec + 0x10, rec, planes, VIEW_CELLS, cnt);
                    let vis = (rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8);
                    wb(vis + 6, r as u8);
                    if r as u8 != 0 {
                        bump(0, 3);
                    }
                    if f34 & 0x200 == 0 {
                        let go = if rb(SHADOW_FLAG) == 0 {
                            bump(0, 4);
                            frustum(rec + 0x10, rec, SHADOW_PLANES, SHADOW_CELLS, rd(SHADOW_PLANE_COUNT)) as u8 != 0
                        } else {
                            true
                        };
                        if go {
                            bump(0, 5);
                            xadd((rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8), 0x10000);
                        }
                    }
                    let eye = (rd(th + 0x58) as usize).wrapping_add(0x710);
                    let d = dist_call(AABB_DIST_SSE, rec + 0x10, eye, rec);
                    let t = rf(CULL_RANGE) as f64;
                    // x87: dist (f32) against thresh * thresh kept at double precision
                    if (d as f64) <= t * t {
                        bump(0, 6);
                        xadd((rd(rd(entry + 4) as usize + 0x18) as usize).wrapping_add(idx * 8), 1);
                    }
                } else {
                    bump(0, 2);
                }
                i += 1;
                if i >= rd(node + 0xC) as usize {
                    break;
                }
            }
        }
    }
}

extern "C" fn split_rs(entry: u32) {
    unsafe {
        seen(0);
        split_impl(entry as usize)
    }
}

#[unsafe(naked)]
extern "C" fn split_thunk() {
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
        f = sym split_rs,
    )
}

// =================================================================================================
// 0x4B5C40 city_cull_chunk_instances  (EDI = entry*, plain ret)
// =================================================================================================

/// Returns the list pointer: the original stores it over its saved ECX, so it leaves it in ECX.
unsafe fn cull_impl(entry: usize) -> u32 {
    unsafe {
        let ctx0 = rd(entry + 4) as usize;
        let set = rd(ctx0 + 0x10) as usize;
        let base = rd(set + 0xC) as usize;
        let node = (rd(entry + 8) as usize).wrapping_mul(0x30).wrapping_add(rd(set + 4) as usize);
        let list = base.wrapping_add((rd(node + 0x1C) as usize).wrapping_mul(2));
        let mut i = 0usize;
        if rd(node + 0xC) == 0 {
            return list as u32;
        }
        loop {
            let idx = rh(list + i * 2) as usize;
            let ctx = rd(entry + 4) as usize;
            let rec = idx.wrapping_mul(0x50).wrapping_add(rd(ctx + 0xC) as usize);
            bump(1, 1);
            if (rd(rec + 0x30) & 0x8000) | (rd(rec + 0x34) & 0x40) == 0 {
                let cnt = rd(PLANE_COUNT);
                let th = thread_state();
                let planes = rd(rd(th + 0x58) as usize + 0x740);
                let r = frustum(rec + 0x10, rec, planes, VIEW_CELLS, cnt);
                let vis = rd(rd(entry + 4) as usize + 0x18) as usize;
                wb(vis.wrapping_add(idx * 8 + 6), r as u8);
                if r as u8 != 0 {
                    bump(1, 3);
                }
            } else {
                bump(1, 2);
            }
            i += 1;
            if i >= rd(node + 0xC) as usize {
                break;
            }
        }
        list as u32
    }
}

extern "C" fn cull_rs(entry: u32) -> u32 {
    unsafe {
        seen(1);
        cull_impl(entry as usize)
    }
}

/// EDI = entry. The original leaves the chunk list pointer in ECX (it overwrites its saved ECX).
#[unsafe(naked)]
extern "C" fn cull_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push edi",
        "call {f}",
        "add esp, 4",
        xmm_load!(),
        "pop edx",
        "add esp, 4",
        "mov ecx, eax",
        "ret",
        f = sym cull_rs,
    )
}

// =================================================================================================
// 0x4B54A0 city_view_instance_filter_test  (ESI = instance, EDX = view record, stack u16 key)
// =================================================================================================

/// Returns (EAX, ECX, EDX) as the original leaves them (callers test AL): the loop counter and the
/// advanced list pointer stay in ECX / EDX.
unsafe fn filter_impl(esi: u32, view: usize, key: u32, ecx0: u32) -> (u32, u32, u32) {
    unsafe {
        let count = rd(view + 0x58);
        if count == 0 {
            return (0, ecx0, view as u32);
        }
        let cx = rh(view + 0x60) as u32;
        if cx as u16 != key as u16 {
            let neg = ((count as i32) < 0) as u32;
            return ((count & !0xFF) | neg, (ecx0 & 0xFFFF_0000) | cx, neg);
        }
        let list = rd(view + 0x5C) as usize;
        if (count as i32) > 0 {
            let n = count as i32;
            let mut k = 0i32;
            while k < n {
                if rh(list + k as usize * 2) == esi {
                    return ((count & !0xFF) | 1, k as u32, (list + k as usize * 2) as u32);
                }
                k += 1;
            }
            (count & !0xFF, k as u32, (list + k as usize * 2) as u32)
        } else {
            let n = count.wrapping_neg();
            if (n as i32) <= 0 {
                return ((n & !0xFF) | 1, 0, view as u32);
            }
            let mut k = 0i32;
            while k < n as i32 {
                if rh(list + k as usize * 2) == esi {
                    return (n & !0xFF, k as u32, (list + k as usize * 2) as u32);
                }
                k += 1;
            }
            ((n & !0xFF) | 1, k as u32, (list + k as usize * 2) as u32)
        }
    }
}

extern "C" fn filter_rs(esi: u32, view: u32, key: u32, out: *mut [u32; 2]) -> u32 {
    unsafe {
        let (eax, ecx, edx) = filter_impl(esi, view as usize, key, (*out)[0]);
        (*out)[0] = ecx;
        (*out)[1] = edx;
        if !S_FILTER.active() {
            return eax;
        }
        S_FILTER.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_FILTER.orig.load(Ordering::Relaxed), 0, 0, view, esi, 0, &[key]);
        if eax & 0xFF != orig & 0xFF {
            S_FILTER.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn filter_thunk() {
    std::arch::naked_asm!(
        "sub esp, 8",
        "mov [esp], ecx",
        "mov [esp + 4], edx",
        xmm_save!(),
        "lea eax, [esp + 128]",
        "push eax",
        "push dword ptr [esp + 144]",
        "push edx",
        "push esi",
        "call {f}",
        "add esp, 16",
        xmm_load!(),
        "mov ecx, [esp]",
        "mov edx, [esp + 4]",
        "add esp, 8",
        "ret",
        f = sym filter_rs,
    )
}

// =================================================================================================
// 0x4B5FF0 city_view_chunk_instances_cull  (cdecl (entry*), plain ret)
// =================================================================================================

/// `entry`: +2 u16 chunk id, +4 ctx, +8 node index, +0xC view index.
unsafe fn view_cull_impl(entry: usize) {
    unsafe {
        let ctx0 = rd(entry + 4) as usize;
        let set = rd(ctx0 + 0x10) as usize;
        let node = (rd(entry + 8) as usize).wrapping_mul(0x30).wrapping_add(rd(set + 4) as usize);
        let list = (rd(set + 0xC) as usize).wrapping_add((rd(node + 0x1C) as usize).wrapping_mul(2));
        if rd(node + 0xC) == 0 {
            return;
        }
        let mut i = 0usize;
        loop {
            let idx = rh(list + i * 2) as usize;
            let ctx = rd(entry + 4) as usize;
            let rec = idx.wrapping_mul(0x50).wrapping_add(rd(ctx + 0xC) as usize);
            let f34 = rd(rec + 0x34);
            let f30 = rd(rec + 0x30);
            let plain = f34 & 0x200 == 0;
            bump(2, 1);
            if (f30 & 0x8000) | (f34 & 0x40) == 0 {
                let p48 = rd(rec + 0x48);
                let obj = if p48 != 0 && f30 & 0x10000 != 0 { p48 } else { rd(rec + 0x40) };
                let diff = obj.wrapping_sub(rd(ctx + 4)) as i32;
                let hi = ((diff as i64 * 0x2AAA_AAABi64) >> 32) as i32;
                let q = hi >> 4;
                let key = (q.wrapping_add(((q as u32) >> 31) as i32)) as u16 as u32;
                let view = rd(entry + 0xC);
                let vrec = (view as usize).wrapping_mul(0x280).wrapping_add(VIEW_RECORDS);
                let filtered = call_regs(CITY_FILTER, 0, 0, vrec as u32, key, 0, &[rh(entry + 2)]);
                if filtered as u8 != 0 {
                    bump(2, 7);
                } else {
                    let vb = call_regs(VIEW_BOUNDS_VISIBLE, 0, 0, 0, rd(entry + 0xC), (rec + 0x10) as u32, &[rec as u32]);
                    if vb as u8 == 0 {
                        bump(2, 8);
                    } else {
                        bump(2, 9);
                        let view = rd(entry + 0xC);
                        let vrec = (view as usize).wrapping_mul(0x280);
                        let add = if plain && rd(vrec + 0x0261_2D18) & 0x400 != 0 {
                            0x10001u32.wrapping_shl(view & 31)
                        } else {
                            1u32.wrapping_shl(view & 31)
                        };
                        let vis = rd(rd(entry + 4) as usize + 0x18) as usize;
                        xadd(vis.wrapping_add(idx * 8), add);
                    }
                }
            } else {
                bump(2, 2);
            }
            i += 1;
            if i >= rd(node + 0xC) as usize {
                break;
            }
        }
    }
}

extern "C" fn view_cull_rs(entry: u32) {
    unsafe {
        seen(2);
        view_cull_impl(entry as usize)
    }
}

#[unsafe(naked)]
extern "C" fn view_cull_thunk() {
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
        f = sym view_cull_rs,
    )
}

// =================================================================================================
// 0xC094E0 texture_entry_animated_resolve  (EAX = index -> EAX = entry or 0)
// =================================================================================================

unsafe fn texres_impl(idx: i32, out: &mut [u32; 2]) -> u32 {
    unsafe {
        if idx < 0 || idx >= rd(TEX_COUNT) as i32 {
            return 0;
        }
        let base = rd(TEX_TABLE);
        let entry = (base as usize).wrapping_add((idx as usize).wrapping_mul(12));
        let info = rd(entry + 4) as usize;
        let frames = rh(info + 0x10);
        let mut i = idx;
        if frames > 1 && rh(entry + 8) == 0 {
            let rate = rb(info + 0x1A) as f64;
            let sign = rd(TEX_TICK) as i32;
            let step32 = (rate * RATE_SCALE) as f32;
            // `fstp dword [esp + 0xC]` overwrites the pushed ECX; the final `pop ecx` leaves these bits
            out[0] = step32.to_bits();
            let step = step32 as f64;
            let mut tick = rd(TEX_TICK) as i32 as f64;
            if sign < 0 {
                tick += TWO_32;
            }
            let v = ftol2(step * tick);
            i = i.wrapping_add(v.wrapping_rem(frames as i32));
        }
        out[1] = (i as u32).wrapping_mul(3);
        base.wrapping_add((i as u32).wrapping_mul(12))
    }
}

extern "C" fn texres_rs(idx: u32, out: *mut [u32; 2]) -> u32 {
    unsafe {
        let mut o = *out;
        let mine = texres_impl(idx as i32, &mut o);
        *out = o;
        if !S_TEX.active() {
            return mine;
        }
        S_TEX.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_TEX.orig.load(Ordering::Relaxed), idx, 0, 0, 0, 0, &[]);
        if mine != orig {
            S_TEX.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn texres_thunk() {
    std::arch::naked_asm!(
        "sub esp, 8",
        "mov [esp], ecx",
        "mov [esp + 4], edx",
        xmm_save!(),
        "lea ecx, [esp + 128]",
        "push ecx",
        "push eax",
        "call {f}",
        "add esp, 8",
        xmm_load!(),
        "mov ecx, [esp]",
        "mov edx, [esp + 4]",
        "add esp, 8",
        "ret",
        f = sym texres_rs,
    )
}

// =================================================================================================
// 0x5240F0 render_item_less  (EDI = a, ESI = b -> AL)
// =================================================================================================

unsafe fn less_impl(a: usize, b: usize) -> u32 {
    unsafe {
        let fa = rd(a + 4);
        let fb = rd(b + 4);
        let ka = ((fa & 0x180) | ((fa >> 4) & 0x200)) >> 7;
        let kb = ((fb & 0x180) | ((fb >> 4) & 0x200)) >> 7;
        if (ka as i32) < kb as i32 {
            return 1;
        }
        if (ka as i32) > kb as i32 {
            return 0;
        }
        let a_has = fa as u8 & 0x20 != 0;
        let b_has = fb as u8 & 0x20 != 0;
        if !a_has && b_has {
            return 1;
        }
        if a_has && !b_has {
            return 0;
        }
        // fcompp on (b, a): sorts before only when a.key < b.key, both ordered
        (rf(a + 0x10) < rf(b + 0x10)) as u32
    }
}

extern "C" fn less_rs(a: u32, b: u32, out: *mut [u32; 2]) -> u32 {
    unsafe {
        // the original leaves ECX = b's sort key and EDX = b's (flags & 0x180)
        let fb = rd(b as usize + 4);
        (*out)[0] = ((fb & 0x180) | ((fb >> 4) & 0x200)) >> 7;
        (*out)[1] = fb & 0x180;
        if !S_LESS.active() {
            return less_impl(a as usize, b as usize);
        }
        S_LESS.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_LESS.orig.load(Ordering::Relaxed), 0, 0, 0, b, a, &[]);
        let mine = less_impl(a as usize, b as usize);
        if mine & 0xFF != orig & 0xFF {
            S_LESS.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn less_thunk() {
    std::arch::naked_asm!(
        "sub esp, 8",
        xmm_save!(),
        "lea eax, [esp + 128]",
        "push eax",
        "push esi",
        "push edi",
        "call {f}",
        "add esp, 12",
        xmm_load!(),
        "mov ecx, [esp]",
        "mov edx, [esp + 4]",
        "add esp, 8",
        "ret",
        f = sym less_rs,
    )
}

// =================================================================================================
// 0x530520 mesh_material_flags_or  (ECX = mesh ref -> EAX)
// =================================================================================================

unsafe fn flags_impl(r: usize) -> u32 {
    unsafe {
        let mesh = rd(r) as usize;
        let (count, list) = if rd(r + 4) != 0 {
            (rh(r + 8), rd(r + 0xC) as usize)
        } else {
            (rh(mesh + 2), rd(mesh + 4) as usize)
        };
        if count == 0 {
            return 0;
        }
        let mats = rd(rd(mesh + 0xC) as usize + 4) as usize;
        let mut acc = 0u32;
        let mut p = list.wrapping_add(0xE);
        for _ in 0..count {
            let m = rh(p) as usize * 0x18;
            acc |= rb(mats.wrapping_add(m + 0x12)) as u32 & 0x28;
            p = p.wrapping_add(0x10);
        }
        acc
    }
}

extern "C" fn flags_rs(r: u32) -> u32 {
    unsafe {
        if !S_FLAGS.active() {
            return flags_impl(r as usize);
        }
        S_FLAGS.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_FLAGS.orig.load(Ordering::Relaxed), 0, r, 0, 0, 0, &[]);
        let mine = flags_impl(r as usize);
        if mine != orig {
            S_FLAGS.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn flags_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push ecx",
        "call {f}",
        "add esp, 4",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "ret",
        f = sym flags_rs,
    )
}

// =================================================================================================
// 0xBF63B0 aabb_point_dist_sq_sse  (ECX = &max, EDX = &point, stack &min -> XMM0.x)
// =================================================================================================

#[inline(always)]
unsafe fn dist_impl(max: usize, point: usize, min: usize) -> f32 {
    unsafe {
        let p = _mm_loadu_ps(point as *const f32);
        let hi = _mm_loadu_ps(max as *const f32);
        let zero = _mm_setzero_ps();
        let a = _mm_max_ps(_mm_sub_ps(p, hi), zero);
        let lo = _mm_loadu_ps(min as *const f32);
        let b = _mm_max_ps(_mm_sub_ps(lo, p), zero);
        let d = _mm_add_ps(a, b);
        let sq = _mm_and_ps(_mm_mul_ps(d, d), _mm_load_ps(DIST_MASK as *const f32));
        let hl = _mm_movehl_ps(sq, sq);
        let s = _mm_add_ps(hl, sq);
        let t = _mm_shuffle_ps::<0xE5>(s, s);
        _mm_cvtss_f32(_mm_add_ps(t, s))
    }
}

/// Same value, NaNs equal (the payload of a NaN result is not part of the contract).
#[inline(always)]
fn same_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// Returns the f32 bits; the thunk puts them in XMM0.
extern "C" fn dist_rs(max: u32, point: u32, min: u32) -> u32 {
    unsafe {
        if !S_DIST.active() {
            return dist_impl(max as usize, point as usize, min as usize).to_bits();
        }
        S_DIST.calls.fetch_add(1, Ordering::Relaxed);
        let orig = dist_call(S_DIST.orig.load(Ordering::Relaxed), max as usize, point as usize, min as usize);
        let mine = dist_impl(max as usize, point as usize, min as usize);
        if !same_f32(mine, orig) {
            S_DIST.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig.to_bits()
    }
}

#[unsafe(naked)]
extern "C" fn dist_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push dword ptr [esp + 140]",
        "push edx",
        "push ecx",
        "call {f}",
        "add esp, 12",
        xmm_load!(),
        "movd xmm0, eax",
        "pop edx",
        "pop ecx",
        "mov eax, dword ptr [esp + 4]",
        "ret",
        f = sym dist_rs,
    )
}

// =================================================================================================
// 0xBE61B0 mat44_mul_sse  (ECX = &A, EDX = &B, stack &out, ret 4)
// =================================================================================================

#[inline(always)]
unsafe fn mat44_impl(a: usize, b: usize, out: usize, xs: *mut f32) {
    unsafe {
        let b0 = _mm_loadu_ps(b as *const f32);
        let b1 = _mm_loadu_ps((b + 0x10) as *const f32);
        let b2 = _mm_loadu_ps((b + 0x20) as *const f32);
        let b3 = _mm_loadu_ps((b + 0x30) as *const f32);
        let mut row = 0usize;
        while row < 4 {
            let r = _mm_loadu_ps((a + row * 16) as *const f32);
            let mut acc = _mm_mul_ps(_mm_shuffle_ps::<0x55>(r, r), b1);
            acc = _mm_add_ps(acc, _mm_mul_ps(_mm_shuffle_ps::<0x00>(r, r), b0));
            acc = _mm_add_ps(acc, _mm_mul_ps(_mm_shuffle_ps::<0xAA>(r, r), b2));
            acc = _mm_add_ps(acc, _mm_mul_ps(_mm_shuffle_ps::<0xFF>(r, r), b3));
            _mm_storeu_ps((out + row * 16) as *mut f32, acc);
            if row == 2 {
                // XMM6 keeps the last term of row 2
                _mm_storeu_ps(xs.add(24), _mm_mul_ps(_mm_shuffle_ps::<0xFF>(r, r), b3));
            }
            if row == 3 {
                // the original leaves XMM0 = row 3 of A, XMM1 = w * B3, XMM2 = x * B0, XMM3/4 = B2/B3,
                // XMM5 = the result row (XMM7 untouched)
                _mm_storeu_ps(xs, r);
                _mm_storeu_ps(xs.add(4), _mm_mul_ps(_mm_shuffle_ps::<0xFF>(r, r), b3));
                _mm_storeu_ps(xs.add(8), _mm_mul_ps(_mm_shuffle_ps::<0x00>(r, r), b0));
                _mm_storeu_ps(xs.add(12), b2);
                _mm_storeu_ps(xs.add(16), b3);
                _mm_storeu_ps(xs.add(20), acc);
            }
            row += 1;
        }
    }
}

extern "C" fn mat44_rs(a: u32, b: u32, out: u32, xs: *mut f32) {
    unsafe {
        if !S_MAT.active() {
            return mat44_impl(a as usize, b as usize, out as usize, xs);
        }
        S_MAT.calls.fetch_add(1, Ordering::Relaxed);
        let mut before = [0u8; 64];
        let mut theirs = [0u8; 64];
        let mut mine = [0u8; 64];
        std::ptr::copy_nonoverlapping(out as *const u8, before.as_mut_ptr(), 64);
        call_regs(S_MAT.orig.load(Ordering::Relaxed), 0, a, b, 0, 0, &[out]);
        std::ptr::copy_nonoverlapping(out as *const u8, theirs.as_mut_ptr(), 64);
        std::ptr::copy_nonoverlapping(before.as_ptr(), out as *mut u8, 64);
        let mut scratch = [0f32; 28];
        mat44_impl(a as usize, b as usize, out as usize, scratch.as_mut_ptr());
        mat44_impl(a as usize, b as usize, out as usize, xs);
        std::ptr::copy_nonoverlapping(out as *const u8, mine.as_mut_ptr(), 64);
        let same = (0..16).all(|k| {
            let x = f32::from_le_bytes(theirs[k * 4..k * 4 + 4].try_into().unwrap());
            let y = f32::from_le_bytes(mine[k * 4..k * 4 + 4].try_into().unwrap());
            same_f32(x, y)
        });
        if !same {
            S_MAT.bad.fetch_add(1, Ordering::Relaxed);
        }
        std::ptr::copy_nonoverlapping(theirs.as_ptr(), out as *mut u8, 64);
    }
}

#[unsafe(naked)]
extern "C" fn mat44_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        xmm_save!(),
        "push ebp",
        "mov ebp, esp",
        "and esp, 0xfffffff0",
        "lea eax, [ebp + 4]",
        "push eax",
        "push dword ptr [ebp + 144]",
        "push edx",
        "push ecx",
        "call {f}",
        "mov esp, ebp",
        "pop ebp",
        xmm_load!(),
        "pop edx",
        "pop ecx",
        "mov eax, dword ptr [esp + 4]",
        "ret 4",
        f = sym mat44_rs,
    )
}

// =================================================================================================
// 0xA90B10 plane_eval_xyz_ptrs  (EAX = plane, ECX = &y, stack (&x, &z), ret 8 -> ST0)
// =================================================================================================

/// `plane.b * y + plane.a * x + plane.c * z` stored as f32, plus `plane.d`, stored as f32 again.
#[inline(always)]
unsafe fn plane_impl(plane: usize, y: usize, x: usize, z: usize) -> f32 {
    unsafe {
        let mut s = rf(plane + 4) as f64 * rf(y) as f64;
        s += rf(x) as f64 * rf(plane) as f64;
        s += rf(plane + 8) as f64 * rf(z) as f64;
        let t = s as f32;
        (t as f64 + rf(plane + 0xC) as f64) as f32
    }
}

/// The original, returning its ST0 as a double.
unsafe fn plane_call(f: usize, plane: usize, y: usize, x: usize, z: usize) -> f32 {
    let lo: u32;
    let hi: u32;
    unsafe {
        std::arch::asm!(
            "push {z}",
            "push {x}",
            "call {f}",
            "sub esp, 8",
            "fstp qword ptr [esp]",
            "pop eax",
            "pop edx",
            x = in(reg) x,
            z = in(reg) z,
            f = in(reg) f,
            inout("eax") plane => lo,
            in("ecx") y,
            lateout("edx") hi,
            clobber_abi("C"),
        );
    }
    f64::from_bits(lo as u64 | (hi as u64) << 32) as f32
}

extern "C" fn plane_rs(plane: u32, y: u32, x: u32, z: u32) -> f32 {
    unsafe {
        if !S_PLANE.active() {
            return plane_impl(plane as usize, y as usize, x as usize, z as usize);
        }
        S_PLANE.calls.fetch_add(1, Ordering::Relaxed);
        let orig = plane_call(S_PLANE.orig.load(Ordering::Relaxed), plane as usize, y as usize, x as usize, z as usize);
        let mine = plane_impl(plane as usize, y as usize, x as usize, z as usize);
        if !same_f32(mine, orig) {
            S_PLANE.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

#[unsafe(naked)]
extern "C" fn plane_thunk() {
    // The original leaves EAX alone, ECX / EDX = the two stack pointers, and overwrites its first
    // stack argument slot with the result (`fstp dword [esp + 4]`).
    std::arch::naked_asm!(
        "push eax",
        xmm_save!(),
        "push dword ptr [esp + 140]",
        "push dword ptr [esp + 140]",
        "push ecx",
        "push eax",
        "call {f}",
        "add esp, 16",
        xmm_load!(),
        "mov edx, [esp + 8]",
        "mov ecx, [esp + 12]",
        "fst dword ptr [esp + 8]",
        "pop eax",
        "ret 8",
        f = sym plane_rs,
    )
}

// =================================================================================================
// install / summary
// =================================================================================================

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let shadowed: [(&Shadow, usize, &[u8], &[usize], usize); 7] = [
        (&S_TEX, TEXTURE_RESOLVE, &TEXRES_BODY, &TEXRES_CALLS, texres_thunk as *const () as usize),
        (&S_LESS, ITEM_LESS, &LESS_BODY, &LESS_CALLS, less_thunk as *const () as usize),
        (&S_FILTER, CITY_FILTER, &FILTER_BODY, &FILTER_CALLS, filter_thunk as *const () as usize),
        (&S_FLAGS, MESH_FLAGS_OR, &FLAGS_BODY, &FLAGS_CALLS, flags_thunk as *const () as usize),
        (&S_DIST, AABB_DIST_SSE, &DIST_BODY, &DIST_CALLS, dist_thunk as *const () as usize),
        (&S_PLANE, PLANE_EVAL, &PLANE_BODY, &PLANE_CALLS, plane_thunk as *const () as usize),
        (&S_MAT, MAT44_MUL, &MAT44_BODY, &MAT44_CALLS, mat44_thunk as *const () as usize),
    ];
    let mut ok = 0;
    let mut total = shadowed.len();
    for (s, addr, body, calls, thunk) in shadowed {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(addr, body, calls) } {
            Ok(copy) => s.orig.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_render3: {}: no shadow copy: {e:?}", s.name)),
        }
        if replace_pinned(s.name, addr, body, thunk) {
            ok += 1;
        }
    }
    for (name, addr, body, thunk) in [
        ("city_cull_chunk_instances_split", CITY_CULL_SPLIT, &SPLIT_BODY[..], split_thunk as *const () as usize),
        ("city_cull_chunk_instances", CITY_CULL, &CULL_BODY[..], cull_thunk as *const () as usize),
        ("city_view_chunk_instances_cull", CITY_VIEW_CULL, &VIEWCULL_BODY[..], view_cull_thunk as *const () as usize),
    ] {
        total += 1;
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_render3: replaced {ok}/{total} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut parts = Vec::new();
    for s in [&S_TEX, &S_LESS, &S_FILTER, &S_FLAGS, &S_DIST, &S_PLANE, &S_MAT] {
        let c = s.total.load(Ordering::Relaxed);
        if c > 0 {
            parts.push(format!("{} {} ({})", s.name, c, s.bad.load(Ordering::Relaxed)));
        }
    }
    for (k, name) in ["city_cull_chunk_instances_split", "city_cull_chunk_instances", "city_view_chunk_instances_cull"].iter().enumerate() {
        let br: Vec<u32> = CULL_BRANCH[k].iter().map(|c| c.load(Ordering::Relaxed)).collect();
        if br[0] > 0 {
            parts.push(format!("{name} {} (branches {:?})", br[0], &br[1..]));
        }
    }
    if parts.is_empty() { String::new() } else { format!("port_render3: {}", parts.join(", ")) }
}

// --- bodies (python tools/body_bytes.py) --------------------------------------------------------

const SPLIT_BODY: [u8; 769] = [
    0x83, 0xEC, 0x2C, 0x53, 0x55, 0x8B, 0x6C, 0x24, 0x38, 0x8B, 0x45, 0x04, 0x8B, 0x48, 0x10, 0x8B,
    0x45, 0x08, 0x8D, 0x04, 0x40, 0xC1, 0xE0, 0x04, 0x03, 0x41, 0x04, 0x80, 0x3D, 0x8A, 0xE8, 0xE8,
    0x00, 0x00, 0x8B, 0x50, 0x1C, 0x8B, 0x49, 0x0C, 0x56, 0x8D, 0x14, 0x51, 0x57, 0x89, 0x44, 0x24,
    0x18, 0x89, 0x54, 0x24, 0x14, 0xC7, 0x44, 0x24, 0x10, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x7E,
    0x01, 0x00, 0x00, 0x64, 0x8B, 0x0D, 0x2C, 0x00, 0x00, 0x00, 0xD9, 0x05, 0x9C, 0x49, 0xE8, 0x00,
    0x8B, 0x09, 0xDC, 0xC8, 0x8B, 0x51, 0x58, 0x81, 0xC2, 0x10, 0x07, 0x00, 0x00, 0x83, 0x78, 0x0C,
    0x00, 0xD9, 0x5C, 0x24, 0x28, 0x89, 0x4C, 0x24, 0x1C, 0x89, 0x54, 0x24, 0x20, 0x0F, 0x86, 0x86,
    0x02, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x10, 0x8B, 0x44, 0x24, 0x14, 0x0F, 0xB7, 0x04, 0x48, 0x0F,
    0xB7, 0xD0, 0x8B, 0x45, 0x04, 0x8D, 0x34, 0x92, 0xC1, 0xE6, 0x04, 0x03, 0x70, 0x0C, 0x33, 0xC0,
    0x8B, 0x7E, 0x34, 0x8B, 0x5E, 0x30, 0x8B, 0xCF, 0x81, 0xE1, 0x00, 0x02, 0x00, 0x00, 0x0B, 0xC1,
    0x89, 0x74, 0x24, 0x24, 0xC6, 0x44, 0x24, 0x40, 0x01, 0x74, 0x05, 0xC6, 0x44, 0x24, 0x40, 0x00,
    0x81, 0xE3, 0x00, 0x80, 0x00, 0x00, 0x83, 0xE7, 0x40, 0x0B, 0xDF, 0x0F, 0x85, 0xE0, 0x00, 0x00,
    0x00, 0x8B, 0x0D, 0x98, 0x73, 0x52, 0x02, 0x8D, 0x3C, 0xD5, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x54,
    0x24, 0x1C, 0x8B, 0x42, 0x58, 0x51, 0x8B, 0x88, 0x40, 0x07, 0x00, 0x00, 0x68, 0xD0, 0x41, 0x77,
    0x02, 0x8D, 0x5E, 0x10, 0x51, 0x8B, 0xCB, 0x8B, 0xD6, 0xE8, 0xA2, 0xA4, 0x07, 0x00, 0x8B, 0x55,
    0x04, 0x8B, 0x4A, 0x18, 0x83, 0xC4, 0x0C, 0x80, 0x7C, 0x24, 0x40, 0x00, 0x88, 0x44, 0x39, 0x06,
    0x74, 0x3E, 0x80, 0x3D, 0x74, 0x2D, 0x61, 0x02, 0x00, 0x75, 0x21, 0x8B, 0x15, 0x9C, 0x73, 0x52,
    0x02, 0x52, 0x68, 0xD0, 0x85, 0x77, 0x02, 0x68, 0x80, 0x2D, 0x61, 0x02, 0x8B, 0xCB, 0x8B, 0xD6,
    0xE8, 0x6B, 0xA4, 0x07, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x74, 0x14, 0x8B, 0x45, 0x04, 0x8B,
    0x48, 0x18, 0x68, 0x00, 0x00, 0x01, 0x00, 0x03, 0xCF, 0x51, 0xFF, 0x15, 0x9C, 0x80, 0xDB, 0x00,
    0x8B, 0x44, 0x24, 0x24, 0x8B, 0x4C, 0x24, 0x20, 0x0F, 0x10, 0x00, 0x0F, 0x10, 0x48, 0x10, 0x0F,
    0x10, 0x11, 0x0F, 0x5C, 0xC2, 0x0F, 0x5C, 0xD1, 0x0F, 0x57, 0xC9, 0x0F, 0x5F, 0xC1, 0x0F, 0x5F,
    0xD1, 0x0F, 0x58, 0xC2, 0x0F, 0x28, 0x1D, 0xF0, 0x96, 0xE9, 0x00, 0x0F, 0x59, 0xC0, 0x0F, 0x54,
    0xC3, 0xF2, 0x0F, 0x7C, 0xC0, 0xF2, 0x0F, 0x7C, 0xC0, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x2C, 0xD9,
    0x44, 0x24, 0x2C, 0xD9, 0x44, 0x24, 0x28, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x75, 0x11,
    0x8B, 0x55, 0x04, 0x8B, 0x42, 0x18, 0x6A, 0x01, 0x03, 0xC7, 0x50, 0xFF, 0x15, 0x9C, 0x80, 0xDB,
    0x00, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x18, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x0C, 0x89,
    0x44, 0x24, 0x10, 0x0F, 0x82, 0xBA, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x2C,
    0xC3, 0x83, 0x78, 0x0C, 0x00, 0x0F, 0x86, 0x2E, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x14, 0x8B,
    0x44, 0x24, 0x10, 0x0F, 0xB7, 0x04, 0x42, 0x8B, 0x4D, 0x04, 0x0F, 0xB7, 0xD0, 0x8D, 0x34, 0x92,
    0xC1, 0xE6, 0x04, 0x03, 0x71, 0x0C, 0x33, 0xC0, 0x8B, 0x7E, 0x34, 0x8B, 0x5E, 0x30, 0x8B, 0xCF,
    0x81, 0xE1, 0x00, 0x02, 0x00, 0x00, 0x0B, 0xC1, 0xC6, 0x44, 0x24, 0x40, 0x01, 0x74, 0x05, 0xC6,
    0x44, 0x24, 0x40, 0x00, 0x81, 0xE3, 0x00, 0x80, 0x00, 0x00, 0x83, 0xE7, 0x40, 0x0B, 0xDF, 0x0F,
    0x85, 0xCC, 0x00, 0x00, 0x00, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x00, 0x8B, 0x0D, 0x98,
    0x73, 0x52, 0x02, 0x8D, 0x1C, 0xD5, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x50, 0x58, 0x51, 0x89, 0x44,
    0x24, 0x2C, 0x8B, 0x82, 0x40, 0x07, 0x00, 0x00, 0x8D, 0x7E, 0x10, 0x68, 0xD0, 0x41, 0x77, 0x02,
    0x50, 0x8B, 0xCF, 0x8B, 0xD6, 0xE8, 0x46, 0xA3, 0x07, 0x00, 0x8B, 0x4D, 0x04, 0x8B, 0x51, 0x18,
    0x83, 0xC4, 0x0C, 0x80, 0x7C, 0x24, 0x40, 0x00, 0x88, 0x44, 0x1A, 0x06, 0x74, 0x3D, 0x80, 0x3D,
    0x74, 0x2D, 0x61, 0x02, 0x00, 0x75, 0x20, 0xA1, 0x9C, 0x73, 0x52, 0x02, 0x50, 0x68, 0xD0, 0x85,
    0x77, 0x02, 0x68, 0x80, 0x2D, 0x61, 0x02, 0x8B, 0xCF, 0x8B, 0xD6, 0xE8, 0x10, 0xA3, 0x07, 0x00,
    0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x74, 0x14, 0x8B, 0x4D, 0x04, 0x8B, 0x51, 0x18, 0x68, 0x00, 0x00,
    0x01, 0x00, 0x03, 0xD3, 0x52, 0xFF, 0x15, 0x9C, 0x80, 0xDB, 0x00, 0x8B, 0x44, 0x24, 0x28, 0xD9,
    0x05, 0x9C, 0x49, 0xE8, 0x00, 0x8B, 0x50, 0x58, 0x81, 0xC2, 0x10, 0x07, 0x00, 0x00, 0x56, 0x8B,
    0xCF, 0xE8, 0x1A, 0x04, 0x74, 0x00, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x28,
    0xD9, 0xC1, 0x83, 0xC4, 0x04, 0xDE, 0xCA, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7A, 0x11,
    0x8B, 0x4D, 0x04, 0x8B, 0x51, 0x18, 0x6A, 0x01, 0x03, 0xD3, 0x52, 0xFF, 0x15, 0x9C, 0x80, 0xDB,
    0x00, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x18, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x0C, 0x89,
    0x44, 0x24, 0x10, 0x0F, 0x82, 0xD2, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x2C,
    0xC3,
];
const CULL_BODY: [u8; 148] = [
    0x51, 0x8B, 0x47, 0x04, 0x8B, 0x40, 0x10, 0x8B, 0x50, 0x0C, 0x53, 0x55, 0x8B, 0x6F, 0x08, 0x8D,
    0x6C, 0x6D, 0x00, 0xC1, 0xE5, 0x04, 0x03, 0x68, 0x04, 0x33, 0xDB, 0x39, 0x5D, 0x0C, 0x8B, 0x4D,
    0x1C, 0x8D, 0x04, 0x4A, 0x56, 0x89, 0x44, 0x24, 0x0C, 0x76, 0x64, 0x8B, 0x4C, 0x24, 0x0C, 0x0F,
    0xB7, 0x04, 0x59, 0x0F, 0xB7, 0xF0, 0x8B, 0x47, 0x04, 0x8D, 0x14, 0xB6, 0xC1, 0xE2, 0x04, 0x03,
    0x50, 0x0C, 0x8B, 0x42, 0x30, 0x8B, 0x4A, 0x34, 0x25, 0x00, 0x80, 0x00, 0x00, 0x83, 0xE1, 0x40,
    0x0B, 0xC1, 0x75, 0x33, 0x8B, 0x0D, 0x98, 0x73, 0x52, 0x02, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00,
    0x51, 0x8B, 0x08, 0x8B, 0x41, 0x58, 0x8B, 0x88, 0x40, 0x07, 0x00, 0x00, 0x68, 0xD0, 0x41, 0x77,
    0x02, 0x51, 0x8D, 0x4A, 0x10, 0xE8, 0xB6, 0xA5, 0x07, 0x00, 0x8B, 0x57, 0x04, 0x8B, 0x4A, 0x18,
    0x83, 0xC4, 0x0C, 0x88, 0x44, 0xF1, 0x06, 0x83, 0xC3, 0x01, 0x3B, 0x5D, 0x0C, 0x72, 0x9C, 0x5E,
    0x5D, 0x5B, 0x59, 0xC3,
];
const VIEWCULL_BODY: [u8; 357] = [
    0x83, 0xEC, 0x10, 0x53, 0x8B, 0x5C, 0x24, 0x18, 0x8B, 0x43, 0x04, 0x8B, 0x48, 0x10, 0x8B, 0x43,
    0x08, 0x8D, 0x04, 0x40, 0xC1, 0xE0, 0x04, 0x03, 0x41, 0x04, 0x8B, 0x49, 0x0C, 0x83, 0x78, 0x0C,
    0x00, 0x8B, 0x50, 0x1C, 0x55, 0x56, 0x8D, 0x14, 0x51, 0x57, 0x89, 0x44, 0x24, 0x1C, 0x89, 0x54,
    0x24, 0x18, 0xC7, 0x44, 0x24, 0x10, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x86, 0x1D, 0x01, 0x00, 0x00,
    0x8B, 0x4C, 0x24, 0x10, 0x8B, 0x44, 0x24, 0x18, 0x0F, 0xB7, 0x04, 0x48, 0x8B, 0x7B, 0x04, 0x0F,
    0xB7, 0xE8, 0x89, 0x6C, 0x24, 0x14, 0x8D, 0x6C, 0xAD, 0x00, 0xC1, 0xE5, 0x04, 0x03, 0x6F, 0x0C,
    0x33, 0xC0, 0x8B, 0x55, 0x34, 0x8B, 0x75, 0x30, 0x8B, 0xCA, 0x81, 0xE1, 0x00, 0x02, 0x00, 0x00,
    0x0B, 0xC1, 0xC6, 0x44, 0x24, 0x24, 0x01, 0x74, 0x05, 0xC6, 0x44, 0x24, 0x24, 0x00, 0x8B, 0xC6,
    0x25, 0x00, 0x80, 0x00, 0x00, 0x83, 0xE2, 0x40, 0x0B, 0xC2, 0x0F, 0x85, 0xB5, 0x00, 0x00, 0x00,
    0x8B, 0x45, 0x48, 0x85, 0xC0, 0x74, 0x0C, 0x81, 0xE6, 0x00, 0x00, 0x01, 0x00, 0x33, 0xC9, 0x0B,
    0xF1, 0x75, 0x03, 0x8B, 0x45, 0x40, 0x2B, 0x47, 0x04, 0x0F, 0xB7, 0x53, 0x02, 0x8B, 0x4B, 0x0C,
    0x52, 0x8B, 0xD0, 0xB8, 0xAB, 0xAA, 0xAA, 0x2A, 0xF7, 0xEA, 0xC1, 0xFA, 0x04, 0x8B, 0xC2, 0xC1,
    0xE8, 0x1F, 0x03, 0xC2, 0x8D, 0x14, 0x89, 0xC1, 0xE2, 0x07, 0x0F, 0xB7, 0xF0, 0x81, 0xC2, 0x10,
    0x2D, 0x61, 0x02, 0xE8, 0xD8, 0xF3, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x66, 0x8B,
    0x73, 0x0C, 0x8D, 0x7D, 0x10, 0x55, 0xE8, 0x75, 0xA3, 0x07, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0,
    0x74, 0x53, 0x80, 0x7C, 0x24, 0x24, 0x00, 0x74, 0x2D, 0x8B, 0x4B, 0x0C, 0x8D, 0x14, 0x89, 0xC1,
    0xE2, 0x07, 0xF7, 0x82, 0x18, 0x2D, 0x61, 0x02, 0x00, 0x04, 0x00, 0x00, 0x74, 0x18, 0xB8, 0x01,
    0x00, 0x01, 0x00, 0xD3, 0xE0, 0x8B, 0x4B, 0x04, 0x8B, 0x51, 0x18, 0x50, 0x8B, 0x44, 0x24, 0x18,
    0x8D, 0x0C, 0xC2, 0x51, 0xEB, 0x19, 0x8B, 0x4B, 0x0C, 0x8B, 0x43, 0x04, 0xBA, 0x01, 0x00, 0x00,
    0x00, 0xD3, 0xE2, 0x8B, 0x48, 0x18, 0x52, 0x8B, 0x54, 0x24, 0x18, 0x8D, 0x04, 0xD1, 0x50, 0xFF,
    0x15, 0x9C, 0x80, 0xDB, 0x00, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x1C, 0x83, 0xC0, 0x01,
    0x3B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x10, 0x0F, 0x82, 0xE3, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D,
    0x5B, 0x83, 0xC4, 0x10, 0xC3,
];
const FILTER_BODY: [u8; 101] = [
    0x8B, 0x42, 0x58, 0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x66, 0x8B, 0x4A, 0x60, 0x66, 0x3B,
    0x4C, 0x24, 0x04, 0x74, 0x0A, 0x33, 0xD2, 0x85, 0xC0, 0x0F, 0x9C, 0xC2, 0x8A, 0xC2, 0xC3, 0x33,
    0xC9, 0x85, 0xC0, 0x57, 0x7D, 0x1F, 0xF7, 0xD8, 0x85, 0xC0, 0x7E, 0x15, 0x8B, 0x52, 0x5C, 0x90,
    0x0F, 0xB7, 0x3A, 0x3B, 0xFE, 0x74, 0x2A, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x02, 0x3B, 0xC8, 0x7C,
    0xEF, 0xB0, 0x01, 0x5F, 0xC3, 0x7E, 0x1A, 0x8B, 0x52, 0x5C, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0xB7, 0x3A, 0x3B, 0xFE, 0x74, 0xEA, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x02, 0x3B, 0xC8, 0x7C,
    0xEF, 0x32, 0xC0, 0x5F, 0xC3,
];
const FILTER_CALLS: [usize; 0] = [];
const TEXRES_BODY: [u8; 130] = [
    0x51, 0x56, 0x8B, 0xF0, 0x85, 0xF6, 0x7C, 0x75, 0x3B, 0x35, 0x08, 0x89, 0x34, 0x02, 0x7D, 0x6D,
    0x53, 0x8B, 0x1D, 0x0C, 0x89, 0x34, 0x02, 0x8D, 0x04, 0x76, 0x8B, 0x4C, 0x83, 0x04, 0x8D, 0x04,
    0x83, 0x57, 0x0F, 0xB7, 0x79, 0x10, 0x66, 0x83, 0xFF, 0x01, 0x76, 0x46, 0x66, 0x83, 0x78, 0x08,
    0x00, 0x75, 0x3F, 0x0F, 0xB6, 0x49, 0x1A, 0x8B, 0x15, 0xC8, 0xA3, 0x52, 0x02, 0x85, 0xD2, 0x89,
    0x4C, 0x24, 0x0C, 0xDB, 0x44, 0x24, 0x0C, 0xDC, 0x0D, 0xE0, 0xBF, 0xE5, 0x00, 0xD9, 0x5C, 0x24,
    0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xDB, 0x05, 0xC8, 0xA3, 0x52, 0x02, 0x7D, 0x06, 0xD8, 0x05, 0xD8,
    0xBF, 0xE5, 0x00, 0xDE, 0xC9, 0xE8, 0xF6, 0x6B, 0x19, 0x00, 0x0F, 0xB7, 0xCF, 0x99, 0xF7, 0xF9,
    0x03, 0xF2, 0x8D, 0x14, 0x76, 0x5F, 0x8D, 0x04, 0x93, 0x5B, 0x5E, 0x59, 0xC3, 0x33, 0xC0, 0x5E,
    0x59, 0xC3,
];
const TEXRES_CALLS: [usize; 1] = [0x65];
const LESS_BODY: [u8; 102] = [
    0x8B, 0x4F, 0x04, 0x8B, 0x56, 0x04, 0x8B, 0xC1, 0x81, 0xE1, 0x80, 0x01, 0x00, 0x00, 0xC1, 0xE8,
    0x04, 0x25, 0x00, 0x02, 0x00, 0x00, 0x0B, 0xC1, 0x8B, 0xCA, 0xC1, 0xE9, 0x04, 0x81, 0xE1, 0x00,
    0x02, 0x00, 0x00, 0x81, 0xE2, 0x80, 0x01, 0x00, 0x00, 0x0B, 0xCA, 0xC1, 0xE8, 0x07, 0xC1, 0xE9,
    0x07, 0x3B, 0xC1, 0x7D, 0x03, 0xB0, 0x01, 0xC3, 0x7F, 0x0C, 0xB0, 0x20, 0x84, 0x47, 0x04, 0x74,
    0x08, 0x84, 0x46, 0x04, 0x75, 0x08, 0x32, 0xC0, 0xC3, 0x84, 0x46, 0x04, 0x75, 0xE7, 0xD9, 0x47,
    0x10, 0xD9, 0x46, 0x10, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x06, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const LESS_CALLS: [usize; 0] = [];
const DIST_BODY: [u8; 94] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x83, 0xEC, 0x10, 0x0F, 0x10, 0x0A, 0x0F, 0x10, 0x19, 0x8B,
    0x45, 0x08, 0x0F, 0x57, 0xC0, 0x0F, 0x28, 0xD0, 0x0F, 0x28, 0xC1, 0x0F, 0x5C, 0xC3, 0x0F, 0x10,
    0x18, 0x0F, 0x5C, 0xD9, 0x0F, 0x5F, 0xC2, 0x0F, 0x5F, 0xDA, 0x0F, 0x58, 0xC3, 0x0F, 0x28, 0xC8,
    0x0F, 0x59, 0xC8, 0x0F, 0x28, 0x05, 0x50, 0x3E, 0xE8, 0x00, 0x0F, 0x54, 0xC8, 0x0F, 0x28, 0xC1,
    0x0F, 0x12, 0xC1, 0x0F, 0x58, 0xC1, 0x0F, 0x28, 0xC8, 0x0F, 0xC6, 0xC8, 0xE5, 0x0F, 0x58, 0xC8,
    0xF3, 0x0F, 0x11, 0x0C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0x8B, 0xE5, 0x5D, 0xC3,
];
const DIST_CALLS: [usize; 0] = [];
const MAT44_BODY: [u8; 256] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x0F, 0x10, 0x01, 0x0F, 0x10, 0x0A, 0x0F, 0x10, 0x52, 0x10,
    0x0F, 0x10, 0x5A, 0x20, 0x0F, 0x10, 0x62, 0x30, 0x8B, 0x45, 0x08, 0x0F, 0x28, 0xE8, 0x0F, 0xC6,
    0xE8, 0x55, 0x0F, 0x59, 0xEA, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0x00, 0x0F, 0x59, 0xF1, 0x0F,
    0x58, 0xEE, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0xAA, 0x0F, 0x59, 0xF3, 0x0F, 0x58, 0xEE, 0x0F,
    0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0xFF, 0x0F, 0x59, 0xF4, 0x0F, 0x58, 0xEE, 0x0F, 0x11, 0x28, 0x0F,
    0x10, 0x41, 0x10, 0x0F, 0x28, 0xE8, 0x0F, 0xC6, 0xE8, 0x55, 0x0F, 0x59, 0xEA, 0x0F, 0x28, 0xF0,
    0x0F, 0xC6, 0xF0, 0x00, 0x0F, 0x59, 0xF1, 0x0F, 0x58, 0xEE, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0,
    0xAA, 0x0F, 0x59, 0xF3, 0x0F, 0x58, 0xEE, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0xFF, 0x0F, 0x59,
    0xF4, 0x0F, 0x58, 0xEE, 0x0F, 0x11, 0x68, 0x10, 0x0F, 0x10, 0x41, 0x20, 0x0F, 0x28, 0xE8, 0x0F,
    0xC6, 0xE8, 0x55, 0x0F, 0x59, 0xEA, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0x00, 0x0F, 0x59, 0xF1,
    0x0F, 0x58, 0xEE, 0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0xAA, 0x0F, 0x59, 0xF3, 0x0F, 0x58, 0xEE,
    0x0F, 0x28, 0xF0, 0x0F, 0xC6, 0xF0, 0xFF, 0x0F, 0x59, 0xF4, 0x0F, 0x58, 0xEE, 0x0F, 0x11, 0x68,
    0x20, 0x0F, 0x10, 0x41, 0x30, 0x0F, 0x28, 0xE8, 0x0F, 0xC6, 0xE8, 0x55, 0x0F, 0x59, 0xEA, 0x0F,
    0x28, 0xD0, 0x0F, 0xC6, 0xD0, 0x00, 0x0F, 0x59, 0xD1, 0x0F, 0x28, 0xC8, 0x0F, 0xC6, 0xC8, 0xAA,
    0x0F, 0x59, 0xCB, 0x0F, 0x58, 0xEA, 0x0F, 0x58, 0xE9, 0x0F, 0x28, 0xC8, 0x0F, 0xC6, 0xC8, 0xFF,
    0x0F, 0x59, 0xCC, 0x0F, 0x58, 0xE9, 0x0F, 0x11, 0x68, 0x30, 0x8B, 0xE5, 0x5D, 0xC2, 0x04, 0x00,
];
const MAT44_CALLS: [usize; 0] = [];
const PLANE_BODY: [u8; 48] = [
    0x8B, 0x54, 0x24, 0x04, 0xD9, 0x40, 0x04, 0xD8, 0x09, 0x8B, 0x4C, 0x24, 0x08, 0xD9, 0x02, 0xD8,
    0x08, 0xDE, 0xC1, 0xD9, 0x40, 0x08, 0xD8, 0x09, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0xD8, 0x40, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC2, 0x08, 0x00,
];
const PLANE_CALLS: [usize; 0] = [];
const FLAGS_BODY: [u8; 91] = [
    0x8B, 0x51, 0x04, 0x33, 0xC0, 0x85, 0xD2, 0x56, 0x74, 0x06, 0x0F, 0xB7, 0x71, 0x08, 0xEB, 0x06,
    0x8B, 0x31, 0x0F, 0xB7, 0x76, 0x02, 0x85, 0xD2, 0x0F, 0xB7, 0xF6, 0x74, 0x05, 0x8B, 0x51, 0x0C,
    0xEB, 0x05, 0x8B, 0x11, 0x8B, 0x52, 0x04, 0x85, 0xF6, 0x8B, 0x09, 0x7E, 0x2C, 0x8B, 0x49, 0x0C,
    0x57, 0x8B, 0x79, 0x04, 0x83, 0xC2, 0x0E, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0xB7, 0x0A, 0x8D, 0x0C, 0x49, 0x0F, 0xB6, 0x4C, 0xCF, 0x12, 0x83, 0xE1, 0x28, 0x0B, 0xC1,
    0x83, 0xC2, 0x10, 0x83, 0xEE, 0x01, 0x75, 0xE8, 0x5F, 0x5E, 0xC3,
];
const FLAGS_CALLS: [usize; 0] = [];

// --- offline differential tests -----------------------------------------------------------------
//
// The test process maps windows of the unpacked exe (code, constants and the globals the originals
// read) at their real addresses, so the ORIGINAL bytes run in place; the callees `frustum_test_aabb`
// and `view_bounds_visible` are replaced by recording stubs and the interlocked-add import slot by
// a native add. Each case builds a random world, runs the original, restores the world, runs the
// Rust thunk and compares: EAX where it is a result, ECX / EDX where the original keeps them, the
// callee-saved registers, XMM0 (where it is a result), every XMM register the original leaves
// alone, ST0 and the x87 depth, all world memory and the stream of calls the code made.
//
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" \
//   cargo test --release -p sr2-hook --target i686-pc-windows-msvc port_render3 -- --ignored --test-threads=1 --nocapture

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
            assert_eq!(c as usize, s, "cannot map {s:#x}..{e:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
            blk += 0x1_0000;
        }
        let bytes = exe_bytes(lo, hi - lo);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), lo as *mut u8, bytes.len()) };
    }

    // --- stubs --------------------------------------------------------------------------------------

    static mut LOG: Vec<[u32; 7]> = Vec::new();
    static mut STUB_RNG: u64 = 1;

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

    extern "system" fn xadd_stub(p: *mut i32, v: i32) -> i32 {
        log_push([3, p as u32, v as u32, 0, 0, 0, 0]);
        unsafe {
            let old = *p;
            *p = old.wrapping_add(v);
            old
        }
    }

    fn patch_jmp(at: usize, to: usize) {
        let rel = (to as i32).wrapping_sub((at + 5) as i32);
        unsafe {
            *(at as *mut u8) = 0xE9;
            std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (at + 1) as *mut u8, 4);
        }
    }

    fn setup() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            for (lo, hi) in [
                (0x4B_4000, 0x4B_8000),
                (0x52_4000, 0x52_5000),
                (0x53_0000, 0x53_1000),
                (0xA9_0000, 0xA9_1000),
                (0xBE_6000, 0xBE_7000),
                (0xBF_6000, 0xBF_7000),
                (0xC0_9000, 0xC0_A000),
                (0xDA_0000, 0xDA_1000),
                (0xDB_8000, 0xDB_9000),
                (0xE5_B000, 0xE5_D000),
                (0xE8_0000, 0xEA_0000),
                (0x234_8000, 0x234_9000),
                (0x252_7000, 0x252_8000),
                (0x252_A000, 0x252_B000),
                (0x261_2000, 0x261_6000),
                (0x34A_5000, 0x34A_6000),
            ] {
                map_window(lo, hi);
            }
            patch_jmp(FRUSTUM_TEST_AABB, frustum_stub as *const () as usize);
            patch_jmp(VIEW_BOUNDS_VISIBLE, vbv_stub as *const () as usize);
            unsafe { *(INTERLOCKED_ADD_SLOT as *mut u32) = xadd_stub as *const () as u32 };
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

    // --- the cull world --------------------------------------------------------------------------------

    const E: usize = 0x000; // entry
    const CTX: usize = 0x100;
    const SET: usize = 0x200;
    const NODES: usize = 0x300;
    const LIST: usize = 0x400;
    const RECS: usize = 0x1000;
    const VIS: usize = 0x3000;
    const VSTATE: usize = 0x4000;
    const PLANES: usize = 0x4800;
    const TLSBLK: usize = 0x4900;
    const OBJS: usize = 0x5000;
    const INCL: usize = 0x5800;
    const NREC: u32 = 48;

    fn cull_extra() -> Vec<(usize, usize)> {
        vec![(VIEW_RECORDS, 0xA00)]
    }

    fn cull_world(rng: &mut Rng, w: &mut World, view_cull: bool) -> Vec<(usize, usize)> {
        w.fill_random(rng);
        // the globals
        gb(SSE3_FLAG, rng.below(2) as u8);
        g(CULL_RANGE, match rng.below(6) {
            0 => 0x7FC0_0000,
            1 => 0,
            2 => (rng.below(20) as f32).to_bits(),
            _ => ((rng.below(4000) as f32) / 10.0).to_bits(),
        });
        g(PLANE_COUNT, rng.u32());
        g(SHADOW_PLANE_COUNT, rng.u32());
        gb(SHADOW_FLAG, if rng.one_in(3) { rng.u32() as u8 } else { 0 });
        // view records: fill the fields the filter reads
        for v in 0..4usize {
            let vr = VIEW_RECORDS + v * 0x280;
            g(vr + 8, if rng.one_in(2) { 0x400 | rng.u32() } else { rng.u32() & !0x400 });
            let cnt: i32 = match rng.below(6) {
                0 => 0,
                1 => i32::MIN,
                2 => rng.below(9) as i32 + 1,
                3 => -(rng.below(9) as i32 + 1),
                _ => rng.u32() as i32 % 6,
            };
            g(vr + 0x58, cnt as u32);
            g(vr + 0x5C, w.at(INCL));
            unsafe { *((vr + 0x60) as *mut u16) = rng.below(4) as u16 };
        }
        for k in 0..16usize {
            w.put16(INCL + k * 2, if rng.one_in(4) { rng.u32() as u16 } else { rng.below(NREC) as u16 });
        }
        // thread state
        let th = w.at(TLSBLK);
        w.put(TLSBLK + 0x58, w.at(VSTATE));
        w.put(VSTATE + 0x740, w.at(PLANES));
        for k in 0..4 {
            w.put(VSTATE + 0x710 + k * 4, fbits(rng, 40.0));
        }
        w.tls_arr[0] = th;
        // records
        for r in 0..NREC as usize {
            let rec = RECS + r * 0x50;
            for k in 0..8usize {
                let v = fbits(rng, 40.0);
                w.put(rec + k * 4, v);
            }
            if rng.below(3) != 0 {
                // a sane box (min <= max)
                for k in 0..3usize {
                    let lo = ((rng.below(200) as f32) - 100.0) / 2.0;
                    let hi = lo + (rng.below(60) as f32) / 3.0;
                    w.put(rec + k * 4, lo.to_bits());
                    w.put(rec + 0x10 + k * 4, hi.to_bits());
                }
            }
            let mut f30 = rng.u32() & !0x8018000;
            if rng.one_in(5) {
                f30 |= 0x8000;
            }
            if rng.one_in(2) {
                f30 |= 0x10000;
            }
            let mut f34 = rng.u32() & !0x240;
            if rng.one_in(5) {
                f34 |= 0x40;
            }
            if rng.one_in(2) {
                f34 |= 0x200;
            }
            w.put(rec + 0x30, f30);
            w.put(rec + 0x34, f34);
            let base = w.at(OBJS);
            let obj = |rng: &mut Rng| match rng.below(5) {
                0 => 0,
                1 => rng.u32(),
                _ => base.wrapping_add(rng.below(40) * 0x60 + if rng.one_in(4) { rng.below(0x60) } else { 0 }),
            };
            w.put(rec + 0x40, obj(rng));
            w.put(rec + 0x48, obj(rng));
        }
        // pointers: entry / ctx / set / nodes / list / vis
        let key = if rng.one_in(2) { rng.below(4) } else { rng.below(0x10000) };
        w.put(E, key << 16 | (rng.u32() & 0xFFFF));
        w.put(E + 4, w.at(CTX));
        let node_idx = rng.below(4);
        w.put(E + 8, node_idx);
        w.put(E + 0xC, rng.below(4));
        w.put(CTX + 4, w.at(OBJS));
        w.put(CTX + 0xC, w.at(RECS));
        w.put(CTX + 0x10, w.at(SET));
        w.put(CTX + 0x18, w.at(VIS));
        w.put(SET + 4, w.at(NODES));
        let list_off = rng.below(8);
        w.put(SET + 0xC, w.at(LIST));
        for n in 0..4usize {
            let count = match rng.below(8) {
                0 => 0,
                _ => rng.below(14),
            };
            w.put(NODES + n * 0x30 + 0xC, count);
            w.put(NODES + n * 0x30 + 0x1C, list_off + rng.below(8));
        }
        for k in 0..0x80usize {
            w.put16(LIST + k * 2, if rng.one_in(30) { rng.u32() as u16 & 0x3F } else { rng.below(NREC) as u16 });
        }
        let _ = view_cull;
        cull_extra()
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn split_matches_original() {
        run_cases(
            "city_cull_chunk_instances_split",
            |rng, w| {
                let extra = cull_world(rng, w, false);
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![w.at(E)] }, Opts { tls: true, ..Opts::default() }, extra)
            },
            0x8000,
            CITY_CULL_SPLIT,
            split_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn cull_matches_original() {
        run_cases(
            "city_cull_chunk_instances",
            |rng, w| {
                let extra = cull_world(rng, w, false);
                let ecx = rng.u32();
                (Case { eax: rng.u32(), ecx, edx: rng.u32(), esi: 0x5151_5151, edi: w.at(E), args: vec![] }, Opts { tls: true, cmp_ecx: true, ..Opts::default() }, extra)
            },
            0x8000,
            CITY_CULL,
            cull_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn view_cull_matches_original() {
        run_cases(
            "city_view_chunk_instances_cull",
            |rng, w| {
                let extra = cull_world(rng, w, true);
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![w.at(E)] }, Opts { tls: true, ..Opts::default() }, extra)
            },
            0x8000,
            CITY_VIEW_CULL,
            view_cull_thunk as *const () as usize,
        );
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn filter_matches_original() {
        run_cases(
            "city_view_instance_filter_test",
            |rng, w| {
                let extra = cull_world(rng, w, true);
                let view = rng.below(4) as usize;
                let key = match rng.below(3) {
                    0 => rng.below(4),
                    1 => rng.u32(),
                    _ => 0xFFFF_0000 | rng.below(4),
                };
                let esi = if rng.one_in(8) { rng.u32() } else { rng.below(NREC) };
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: (VIEW_RECORDS + view * 0x280) as u32, esi, edi: 0xD1D1_D1D1, args: vec![key] }, Opts { eax: true, ..Opts::default() }, extra)
            },
            0x8000,
            CITY_FILTER,
            filter_thunk as *const () as usize,
        );
    }

    // --- texture_entry_animated_resolve ---------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn texres_matches_original() {
        run_cases(
            "texture_entry_animated_resolve",
            |rng, w| {
                w.fill_random(rng);
                let n = rng.below(12);
                g(TEX_COUNT, n);
                g(TEX_TABLE, w.at(0x100));
                g(SSE2_FLAG, rng.below(2));
                g(TEX_TICK, match rng.below(6) {
                    0 => 0,
                    1 => 0x8000_0000,
                    2 => rng.u32(),
                    3 => rng.u32() | 0x8000_0000,
                    _ => rng.below(100000),
                });
                for k in 0..12usize {
                    w.put(0x100 + k * 12 + 4, w.at(0x400 + (k % 5) * 0x40));
                    w.put16(0x100 + k * 12 + 8, if rng.one_in(3) { rng.u32() as u16 } else { 0 });
                }
                for k in 0..5usize {
                    w.put16(0x400 + k * 0x40 + 0x10, match rng.below(5) {
                        0 => 0,
                        1 => 1,
                        2 => rng.u32() as u16,
                        _ => 2 + rng.below(9) as u16,
                    });
                    w.put8(0x400 + k * 0x40 + 0x1A, match rng.below(4) {
                        0 => 0,
                        1 => 255,
                        _ => rng.u32() as u8,
                    });
                }
                let idx = match rng.below(8) {
                    0 => rng.u32(),
                    1 => 0xFFFF_FFFF,
                    2 => n,
                    _ => rng.below(14).wrapping_sub(1),
                };
                (Case { eax: idx, ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![] }, Opts { eax: true, ..Opts::default() }, vec![(TEX_COUNT, 8), (TEX_TICK, 4), (SSE2_FLAG, 4)])
            },
            0x1000,
            TEXTURE_RESOLVE,
            texres_thunk as *const () as usize,
        );
    }

    // --- render_item_less ------------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn less_matches_original() {
        run_cases(
            "render_item_less",
            |rng, w| {
                w.fill_random(rng);
                let cls_a = rng.pick(&[0u32, 0x80, 0x100, 0x180]) | rng.pick(&[0u32, 0x2000]);
                let cls_b = if rng.below(3) != 0 { cls_a } else { rng.pick(&[0u32, 0x80, 0x100, 0x180]) | rng.pick(&[0u32, 0x2000]) };
                let key_a = fbits(rng, 5.0);
                let key_b = if rng.one_in(4) { key_a } else { fbits(rng, 5.0) };
                for (item, cls, key) in [(0usize, cls_a, key_a), (1, cls_b, key_b)] {
                    let o = item * 0x40;
                    let flags = (rng.u32() & !0x21A0) | cls | if rng.one_in(2) { 0x20 } else { 0 };
                    w.put(o + 4, flags);
                    w.put(o + 0x10, key);
                }
                let a = w.at(0);
                let b = w.at(0x40);
                (Case { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: b, edi: a, args: vec![] }, Opts { eax: true, al_only: true, ..Opts::default() }, vec![])
            },
            0x1000,
            ITEM_LESS,
            less_thunk as *const () as usize,
        );
    }

    // --- mesh_material_flags_or ------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn flags_matches_original() {
        run_cases(
            "mesh_material_flags_or",
            |rng, w| {
                w.fill_random(rng);
                let own = rng.one_in(2);
                // ref at 0x100: mesh, own flag, count, list
                w.put(0x100, w.at(0x200));
                w.put(0x104, if own { rng.u32() | 1 } else { 0 });
                let count = match rng.below(5) {
                    0 => 0,
                    _ => rng.below(9),
                };
                if own {
                    w.put16(0x108, count as u16);
                    w.put(0x10C, w.at(0x800));
                    w.put16(0x200 + 2, rng.u32() as u16);
                    w.put(0x204, 0xDEAD_0000);
                } else {
                    w.put16(0x200 + 2, count as u16);
                    w.put(0x204, w.at(0x800));
                    w.put(0x10C, 0xDEAD_0000);
                    w.put16(0x108, rng.u32() as u16);
                }
                // mesh +0xC -> X; X +4 -> material table
                if count == 0 {
                    w.put(0x20C, 0xDEAD_1000 + rng.below(0x100));
                } else {
                    w.put(0x20C, w.at(0x300));
                    w.put(0x304, w.at(0xC00));
                }
                for k in 0..10usize {
                    w.put16(0x800 + k * 0x10 + 0xE, rng.below(16) as u16);
                }
                (Case { eax: rng.u32(), ecx: w.at(0x100), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![] }, Opts { eax: true, ..Opts::default() }, vec![])
            },
            0x1000,
            MESH_FLAGS_OR,
            flags_thunk as *const () as usize,
        );
    }

    // --- aabb_point_dist_sq_sse ------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn dist_matches_original() {
        run_cases(
            "aabb_point_dist_sq_sse",
            |rng, w| {
                w.fill_random(rng);
                // three unaligned vec4: max at 0x103, point at 0x153, min at 0x1A3 (odd offsets)
                for (base, _) in [(0x103usize, 0), (0x153, 1), (0x1A3, 2)] {
                    for k in 0..4usize {
                        let v = fbits(rng, 50.0);
                        unsafe { std::ptr::write_unaligned((w.base() + base + k * 4) as *mut u32, v) };
                    }
                }
                let a = (w.at(0x103), w.at(0x153), w.at(0x1A3));
                (Case { eax: rng.u32(), ecx: a.0, edx: a.1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![a.2] }, Opts { xmm0: true, keep_ecx: true, keep_edx: true, ..Opts::default() }, vec![])
            },
            0x1000,
            AABB_DIST_SSE,
            dist_thunk as *const () as usize,
        );
    }

    // --- mat44_mul_sse ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn mat44_matches_original() {
        run_cases(
            "mat44_mul_sse",
            |rng, w| {
                w.fill_random(rng);
                for k in 0..0x60usize {
                    // two matrices and an output area at odd offsets
                    let v = if rng.one_in(8) { rng.pick(&[0x7F80_0000u32, 0xFF80_0000, 0, 0x8000_0000, 0x7000_0000, 0x0000_0001, 0x3F80_0000]) } else { (((rng.below(2001) as f32) - 1000.0) / 100.0).to_bits() };
                    unsafe { std::ptr::write_unaligned((w.base() + 0x101 + k * 4) as *mut u32, v) };
                }
                let a = w.at(0x101);
                let b = w.at(0x101 + 0x40);
                // the output: apart, on A, on B, or half-overlapping A
                let out = match rng.below(5) {
                    0 => a,
                    1 => b,
                    2 => a + 0x10,
                    _ => w.at(0x101 + 0x80),
                };
                (Case { eax: rng.u32(), ecx: a, edx: b, esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![out] }, Opts { keep_ecx: true, keep_edx: true, ..Opts::default() }, vec![])
            },
            0x1000,
            MAT44_MUL,
            mat44_thunk as *const () as usize,
        );
    }

    // --- plane_eval_xyz_ptrs ---------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn plane_matches_original() {
        run_cases(
            "plane_eval_xyz_ptrs",
            |rng, w| {
                w.fill_random(rng);
                for k in 0..0x20usize {
                    let v = fbits(rng, 100.0);
                    w.put(0x100 + k * 4, v);
                }
                let (plane, y, x, z) = (w.at(0x100), w.at(0x140), w.at(0x150), w.at(0x160));
                (Case { eax: plane, ecx: y, edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1, args: vec![x, z] }, Opts { st0: true, ..Opts::default() }, vec![])
            },
            0x1000,
            PLANE_EVAL,
            plane_thunk as *const () as usize,
        );
    }
}
