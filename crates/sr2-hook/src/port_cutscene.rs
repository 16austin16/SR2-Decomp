//! Port module: the cutscene shot functions (table 0xDD28A0) and cutscene actions (table 0xDD2B28)
//! of doc 51, each checked against the disassembly.
//!
//! Every port is a cdecl thunk that keeps ECX / EDX (`keep_ecx_edx_arg1`) around the Rust body,
//! which is a superset of what each original preserves (the originals are cdecl, void, and callers
//! of the tables ignore EAX).
//!
//! Checks:
//! - `shadow`: for the first `SHADOW_CALLS` calls a relocated copy of the original runs first on
//!   the real record, the bytes it wrote (the regions listed per port) are saved and restored, then
//!   the Rust version runs and the regions are compared; on a mismatch the original's bytes are
//!   put back (and counted). Used for the shot functions that only write globals / player fields
//!   and call nothing.
//! - counters: ports that call the game's functions (whose side effects cannot be run twice) keep
//!   a per-port call counter and per-branch counters, no shadow.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;
const PLAYER_PTR: usize = 0x0252_7D10; // cutscene player object
const GAME_NOW: usize = 0x00E8_43AC;
const WRAP: i32 = 0x6B49_D200;

struct St {
    calls: AtomicU32,
    mism: AtomicU32,
    orig: AtomicUsize,
}
#[allow(clippy::declare_interior_mutable_const)]
const ST0: St = St { calls: AtomicU32::new(0), mism: AtomicU32::new(0), orig: AtomicUsize::new(0) };
static ST: [St; NPORTS] = [ST0; NPORTS];

type Regions = unsafe fn(u32) -> Vec<(usize, usize)>;

/// Run the original and the Rust body on the same state (first SHADOW_CALLS calls) and keep the
/// original's writes on a mismatch.
unsafe fn shadow(i: usize, rec: u32, rust: unsafe fn(u32), regions: Regions) {
    let st = &ST[i];
    let n = st.calls.fetch_add(1, Ordering::Relaxed);
    if n >= SHADOW_CALLS {
        unsafe { rust(rec) };
        return;
    }
    let orig = st.orig.load(Ordering::Relaxed);
    if orig == 0 {
        unsafe { rust(rec) };
        return;
    }
    unsafe {
        let regs = regions(rec);
        let snap: Vec<Vec<u8>> =
            regs.iter().map(|&(a, l)| std::slice::from_raw_parts(a as *const u8, l).to_vec()).collect();
        call_regs(orig, 0, 0, 0, 0, 0, &[rec]);
        let after: Vec<Vec<u8>> =
            regs.iter().map(|&(a, l)| std::slice::from_raw_parts(a as *const u8, l).to_vec()).collect();
        for (k, &(a, l)) in regs.iter().enumerate() {
            std::ptr::copy_nonoverlapping(snap[k].as_ptr(), a as *mut u8, l);
        }
        rust(rec);
        let mut bad = false;
        for (k, &(a, l)) in regs.iter().enumerate() {
            let now = std::slice::from_raw_parts(a as *const u8, l);
            if now != &after[k][..] {
                bad = true;
                std::ptr::copy_nonoverlapping(after[k].as_ptr(), a as *mut u8, l);
            }
        }
        if bad {
            st.mism.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// `fld dword; fstp dword`: quiets a signalling NaN.
fn q(x: f32) -> f32 {
    std::hint::black_box(x as f64) as f32
}

unsafe fn pl() -> usize {
    unsafe { rd(PLAYER_PTR) as usize }
}

unsafe fn b8(rec: u32) -> u8 {
    unsafe { rb(rec as usize + 8) }
}

unsafe fn f8(rec: u32) -> f32 {
    unsafe { rf(rec as usize + 8) }
}

/// `now + ms`, wrapped with the signed `jle` test the shot functions use.
fn now_plus(ms: i32) -> i32 {
    let v = unsafe { rd(GAME_NOW) } as i32 + ms;
    if v > WRAP { v - WRAP } else { v }
}

/// Defines the extern wrapper and the ECX / EDX keeping thunk of a shadowed port.
macro_rules! sp {
    ($w:ident, $t:ident, $idx:expr, $rust:ident, $reg:ident) => {
        extern "C" fn $w(rec: u32) {
            unsafe { shadow($idx, rec, $rust, $reg) }
        }
        keep_ecx_edx_arg1!($t, $w);
    };
}

// === level_lights 0x6D7590 =========================================================================
unsafe fn level_lights(rec: u32) {
    unsafe { wb(0x0252_95C9, (b8(rec) == 0) as u8) }
}
unsafe fn level_lights_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0252_95C9, 1)]
}
sp!(level_lights_w, level_lights_t, 0, level_lights, level_lights_r);

// === tree_shadows 0x6D75B0 =========================================================================
unsafe fn tree_shadows(rec: u32) {
    unsafe { wd(0x00E9_8994, (b8(rec) != 0) as u32 + 1) }
}
unsafe fn tree_shadows_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_8994, 4)]
}
sp!(tree_shadows_w, tree_shadows_t, 1, tree_shadows, tree_shadows_r);

// === tod_character_shadows 0x6D75D0 ================================================================
unsafe fn tod_char_shadows(rec: u32) {
    unsafe { wb(0x00E9_7D4E, b8(rec)) }
}
unsafe fn tod_char_shadows_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_7D4E, 1)]
}
sp!(tod_char_shadows_w, tod_char_shadows_t, 2, tod_char_shadows, tod_char_shadows_r);

// === motion_blur 0x6D75E0 ==========================================================================
unsafe fn motion_blur(rec: u32) {
    unsafe {
        let b = (b8(rec) == 0) as u8;
        wb(pl() + 0x15E0, b);
    }
}
unsafe fn motion_blur_r(_: u32) -> Vec<(usize, usize)> {
    vec![(unsafe { pl() } + 0x15E0, 1)]
}
sp!(motion_blur_w, motion_blur_t, 3, motion_blur, motion_blur_r);

// === motion_blur_override 0x6D7600 =================================================================
unsafe fn motion_blur_override(rec: u32) {
    unsafe {
        let p = rd(rec as usize + 8) as usize;
        let d = pl() + 0x15E4;
        let (a, b, c) = (rd(p), rd(p + 4), rd(p + 8));
        wd(d, a);
        wd(d + 4, b);
        wd(d + 8, c);
    }
}
unsafe fn motion_blur_override_r(_: u32) -> Vec<(usize, usize)> {
    vec![(unsafe { pl() } + 0x15E4, 12)]
}
sp!(motion_blur_override_w, motion_blur_override_t, 4, motion_blur_override, motion_blur_override_r);

// === camera_moves_with_target 0x6D7630 =============================================================
unsafe fn camera_moves(rec: u32) {
    unsafe { wb(0x025F_5D90, b8(rec)) }
}
unsafe fn camera_moves_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x025F_5D90, 1)]
}
sp!(camera_moves_w, camera_moves_t, 5, camera_moves, camera_moves_r);

// === disable_trees 0x6D7640 ========================================================================
unsafe fn disable_trees(rec: u32) {
    unsafe { wb(0x00E8_78BB, (b8(rec) == 0) as u8) }
}
unsafe fn disable_trees_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E8_78BB, 1)]
}
sp!(disable_trees_w, disable_trees_t, 6, disable_trees, disable_trees_r);

// === city_no_render_al 0x6D7660 ====================================================================
unsafe fn city_no_render_al(rec: u32) {
    unsafe { wb(0x0252_6CC5, b8(rec)) }
}
unsafe fn city_no_render_al_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0252_6CC5, 1)]
}
sp!(city_no_render_al_w, city_no_render_al_t, 7, city_no_render_al, city_no_render_al_r);

// === shader_lod_force_highest 0x6D7670 =============================================================
unsafe fn shader_lod_force(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            let p = pl();
            if rd(p + 0x165C) as i32 == -2 {
                wd(p + 0x165C, rd(0x00E8_49D0));
            }
            wb(0x0252_6DC4, 0);
            wd(0x00E8_49D0, 0);
        }
    }
}
unsafe fn shader_lod_force_r(_: u32) -> Vec<(usize, usize)> {
    vec![(unsafe { pl() } + 0x165C, 4), (0x0252_6DC4, 1), (0x00E8_49D0, 4)]
}
sp!(shader_lod_force_w, shader_lod_force_t, 8, shader_lod_force, shader_lod_force_r);

// === city_shadow_dist 0x6D76B0 =====================================================================
unsafe fn city_shadow_dist(rec: u32) {
    unsafe { wf(0x00E8_49A0, q(f8(rec))) }
}
unsafe fn city_shadow_dist_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E8_49A0, 4)]
}
sp!(city_shadow_dist_w, city_shadow_dist_t, 9, city_shadow_dist, city_shadow_dist_r);

// === far_clip 0x6D7720 =============================================================================
// fldz; fcomp [v]; test ah, 5; jp skip -> stores only when 0.0 < v.
unsafe fn far_clip(rec: u32) {
    unsafe {
        let v = f8(rec);
        if 0.0f64 < v as f64 {
            wf(0x00E9_8988, q(v));
        }
    }
}
unsafe fn far_clip_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_8988, 4)]
}
sp!(far_clip_w, far_clip_t, 10, far_clip, far_clip_r);

// === near_clip_danger 0x6D7740 =====================================================================
unsafe fn near_clip(rec: u32) {
    unsafe { wf(0x00E9_898C, q(f8(rec))) }
}
unsafe fn near_clip_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_898C, 4)]
}
sp!(near_clip_w, near_clip_t, 11, near_clip, near_clip_r);

// === shot_fade_out 0x6D77A0 ========================================================================
unsafe fn shot_fade_out(rec: u32) {
    unsafe {
        let v = f8(rec);
        if 0.0f64 < v as f64 {
            let s = rd(pl() + 0x1540) as usize;
            wf(s + 0x164, q(v));
            let s = rd(pl() + 0x1540) as usize;
            wf(s + 0x168, q(v));
        }
    }
}
unsafe fn shot_fade_out_r(_: u32) -> Vec<(usize, usize)> {
    unsafe { vec![(rd(pl() + 0x1540) as usize + 0x164, 8)] }
}
sp!(shot_fade_out_w, shot_fade_out_t, 12, shot_fade_out, shot_fade_out_r);

/// `([0x27E6714] - [0x27E6710])` stored as f32, minus `v` stored as f32 (the midway fades).
unsafe fn midway_value(v: f32) -> f32 {
    unsafe {
        let a = (rf(0x027E_6714) as f64 - rf(0x027E_6710) as f64) as f32;
        (a as f64 - v as f64) as f32
    }
}

// === shot_fade_out_midway 0x6D77E0 =================================================================
unsafe fn shot_fade_out_midway(rec: u32) {
    unsafe {
        let v = f8(rec);
        if 0.0f64 < v as f64 {
            let p = pl();
            let b = midway_value(v);
            let s = rd(p + 0x1540) as usize;
            wf(s + 0x164, b);
            let s = rd(p + 0x1540) as usize;
            wf(s + 0x168, q(rf(0x00DD_1C8C)));
        }
    }
}
unsafe fn shot_fade_out_midway_r(_: u32) -> Vec<(usize, usize)> {
    unsafe { vec![(rd(pl() + 0x1540) as usize + 0x164, 8)] }
}
sp!(shot_fade_out_midway_w, shot_fade_out_midway_t, 13, shot_fade_out_midway, shot_fade_out_midway_r);

// === shot_cut_out_midway 0x6D7840 ==================================================================
// Same, but the second field gets the 0.0 still on the x87 stack.
unsafe fn shot_cut_out_midway(rec: u32) {
    unsafe {
        let v = f8(rec);
        if 0.0f64 < v as f64 {
            let p = pl();
            let b = midway_value(v);
            let s = rd(p + 0x1540) as usize;
            wf(s + 0x164, b);
            let s = rd(p + 0x1540) as usize;
            wf(s + 0x168, 0.0);
        }
    }
}
sp!(shot_cut_out_midway_w, shot_cut_out_midway_t, 14, shot_cut_out_midway, shot_fade_out_midway_r);

// === letterbox_fade_in 0x6D7890 ====================================================================
unsafe fn letterbox_fade_in(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            wb(0x0140_C85A, 1);
            wf(0x01F7_6904, 0.0);
            wd(0x00EA_C468, now_plus(0x3E8) as u32);
        }
    }
}
unsafe fn letterbox_fade_in_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0140_C85A, 1), (0x01F7_6904, 4), (0x00EA_C468, 4)]
}
sp!(letterbox_fade_in_w, letterbox_fade_in_t, 15, letterbox_fade_in, letterbox_fade_in_r);

// === letterbox_fade_out 0x6D78D0 ===================================================================
unsafe fn letterbox_fade_out(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            let s = rd(pl() + 0x1540) as usize;
            wb(s + 0x16C, 1);
        }
    }
}
unsafe fn letterbox_fade_out_r(_: u32) -> Vec<(usize, usize)> {
    unsafe { vec![(rd(pl() + 0x1540) as usize + 0x16C, 1)] }
}
sp!(letterbox_fade_out_w, letterbox_fade_out_t, 16, letterbox_fade_out, letterbox_fade_out_r);

// === framebuffer_tint 0x6D78F0 =====================================================================
// Builds a 9-dword record on the stack and `rep movsd`s it to player+0x155C. The record's byte at
// +4 is 1 and its other three bytes are stale stack: only that byte is written here (the shadow
// leaves the three bytes out of the comparison).
unsafe fn framebuffer_tint(rec: u32) {
    unsafe {
        let p = rd(rec as usize + 8) as usize;
        let (r, g, b) = (rb(p), rb(p + 1), rb(p + 2));
        if r == 0 && g == 0 && b == 0 {
            return;
        }
        let k = *(0x00E5_C038 as *const f64);
        let a = q(rf(p + 4));
        let rf_ = (r as f64 / k) as f32;
        let gf_ = (g as f64 / k) as f32;
        let bf_ = (b as f64 / k) as f32;
        let big_r = (rf_ as f64 * a as f64) as f32;
        let big_g = (gf_ as f64 * a as f64) as f32;
        let big_b = (a as f64 * bf_ as f64) as f32;
        let d = pl() + 0x155C;
        wd(d, 3);
        wb(d + 4, 1);
        wf(d + 8, big_r);
        wf(d + 0xC, big_g);
        wf(d + 0x10, big_b);
        wf(d + 0x14, 1.0);
        wf(d + 0x18, q(rf(p + 0xC)));
        wf(d + 0x1C, q(rf(p + 8)));
        wf(d + 0x20, q(rf(p + 0x10)));
    }
}
unsafe fn framebuffer_tint_r(_: u32) -> Vec<(usize, usize)> {
    let d = unsafe { pl() } + 0x155C;
    vec![(d, 4), (d + 4, 1), (d + 8, 0x1C)]
}
sp!(framebuffer_tint_w, framebuffer_tint_t, 17, framebuffer_tint, framebuffer_tint_r);

// === force_tod_lights 0x6D79E0 =====================================================================
unsafe fn force_tod_lights(rec: u32) {
    unsafe {
        let c = b8(rec);
        wb(0x0252_6CC3, 1);
        wb(0x0252_6CC4, c);
    }
}
unsafe fn force_tod_lights_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0252_6CC3, 2)]
}
sp!(force_tod_lights_w, force_tod_lights_t, 18, force_tod_lights, force_tod_lights_r);

// === lightning 0x6D7D70 ============================================================================
unsafe fn lightning(rec: u32) {
    unsafe {
        if b8(rec) != 0 {
            wd(0x00E9_A848, now_plus(0x1388) as u32);
        } else {
            wd(0x00E9_A848, 0xFFFF_FFFF);
        }
    }
}
unsafe fn lightning_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_A848, 4)]
}
sp!(lightning_w, lightning_t, 19, lightning, lightning_r);

// === glares 0x6D8120 ===============================================================================
unsafe fn glares(rec: u32) {
    unsafe { wb(0x00E9_7D56, (b8(rec) != 0) as u8) }
}
unsafe fn glares_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_7D56, 1)]
}
sp!(glares_w, glares_t, 20, glares, glares_r);

// === fog_distance 0x6D8170 =========================================================================
// Each of the three floats is tested with `fcom`; `test ah, 0x41 / jp` falls through when
// 0.0 <= x (ordered), otherwise the enable byte is cleared.
unsafe fn fog_distance(rec: u32) {
    unsafe {
        let p = rd(rec as usize + 8) as usize;
        let (x, y, z) = (rf(p) as f64, rf(p + 4) as f64, rf(p + 8) as f64);
        if 0.0f64 <= x && 0.0f64 <= y && 0.0f64 <= z {
            let (a, b, c) = (rd(p), rd(p + 4), rd(p + 8));
            wb(0x0265_8CA4, 1);
            wd(0x0265_8CA8, a);
            wd(0x0265_8CAC, b);
            wd(0x0265_8CB0, c);
        } else {
            wb(0x0265_8CA4, 0);
        }
    }
}
unsafe fn fog_distance_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0265_8CA4, 1), (0x0265_8CA8, 12)]
}
sp!(fog_distance_w, fog_distance_t, 21, fog_distance, fog_distance_r);

// === city_region_blend 0x6D81E0 ====================================================================
unsafe fn city_region_blend(rec: u32) {
    unsafe { wb(0x00E9_96E6, b8(rec)) }
}
unsafe fn city_region_blend_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_96E6, 1)]
}
sp!(city_region_blend_w, city_region_blend_t, 22, city_region_blend, city_region_blend_r);

// === city_render_fade_dist_scale 0x6D81F0 ==========================================================
unsafe fn city_render_fade(rec: u32) {
    unsafe { wf(0x00E9_96B4, q(f8(rec))) }
}
unsafe fn city_render_fade_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x00E9_96B4, 4)]
}
sp!(city_render_fade_w, city_render_fade_t, 23, city_render_fade, city_render_fade_r);

// === drug_effect 0x6D8200 ==========================================================================
unsafe fn drug_effect(rec: u32) {
    unsafe {
        let p = rd(rec as usize + 8) as usize;
        let pp = pl();
        wf(pp + 0x15D8, q(rf(p)));
        wf(pp + 0x15DC, q(rf(p + 4)));
    }
}
unsafe fn drug_effect_r(_: u32) -> Vec<(usize, usize)> {
    vec![(unsafe { pl() } + 0x15D8, 8)]
}
sp!(drug_effect_w, drug_effect_t, 24, drug_effect, drug_effect_r);

// === rain_override 0x6D8240 ========================================================================
// v < 0.0 or NaN (the `test ah, 1` of an fcom with 0.0 under it): flag 0 and 0.0, else flag 1 and v.
unsafe fn rain_override(rec: u32) {
    unsafe {
        let v = q(f8(rec));
        if v < 0.0 || v.is_nan() {
            wb(0x0252_6D78, 0);
            wf(0x0252_6D7C, 0.0);
        } else {
            wb(0x0252_6D78, 1);
            wf(0x0252_6D7C, v);
        }
    }
}
unsafe fn rain_override_r(_: u32) -> Vec<(usize, usize)> {
    vec![(0x0252_6D78, 1), (0x0252_6D7C, 4)]
}
sp!(rain_override_w, rain_override_t, 25, rain_override, rain_override_r);

// === credits_roll 0x6D82D0 =========================================================================
unsafe fn credits_roll(rec: u32) {
    unsafe {
        let v = (f8(rec) as f64 + rf(0x027E_66FC) as f64) as f32;
        wf(pl() + 0xB08, v);
    }
}
unsafe fn credits_roll_r(_: u32) -> Vec<(usize, usize)> {
    vec![(unsafe { pl() } + 0xB08, 4)]
}
sp!(credits_roll_w, credits_roll_t, 26, credits_roll, credits_roll_r);


// ====================================================================================================
// Ports that call the game's functions (counters only)
// ====================================================================================================

/// Per-port branch counters (four per port).
static BR: [AtomicU32; NPORTS * 4] = {
    #[allow(clippy::declare_interior_mutable_const)]
    const Z: AtomicU32 = AtomicU32::new(0);
    [Z; NPORTS * 4]
};

fn hit(i: usize, k: usize) {
    BR[i * 4 + k].fetch_add(1, Ordering::Relaxed);
}

/// Entry counter + wrapper + ECX / EDX keeping thunk of a counter port taking the record.
macro_rules! cp {
    ($w:ident, $t:ident, $idx:expr, $rust:ident) => {
        extern "C" fn $w(rec: u32) {
            ST[$idx].calls.fetch_add(1, Ordering::Relaxed);
            unsafe { $rust(rec) }
        }
        keep_ecx_edx_arg1!($t, $w);
    };
}

/// `call_regs` for a callee that returns on the x87 stack: the result is popped as a qword.
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    let mut out = 0f64;
    let block: [u32; 9] =
        [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
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

/// `fld qword [x]; call 0xDA0140` (the game's float -> int, truncating): the int result in EAX.
unsafe fn ftol(x: f64) -> i32 {
    let r: i32;
    unsafe {
        std::arch::asm!(
            "fld qword ptr [ecx]",
            "call edx",
            in("ecx") &x as *const f64,
            in("edx") 0x00DA_0140usize,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

/// `push s; call _atof (0xC984F5); add esp, 4` with the result popped from ST0.
unsafe fn atof(s: u32) -> f64 {
    let mut out = 0f64;
    unsafe {
        std::arch::asm!(
            "push ecx",
            "call eax",
            "add esp, 4",
            "fstp qword ptr [edi]",
            in("ecx") s,
            in("eax") 0x00C9_84F5usize,
            in("edi") &mut out as *mut f64,
            clobber_abi("C"),
        );
    }
    out
}

/// `script_find_object_by_name` (0xA52BB0, ECX = name, EAX = &found) and, when it did not find
/// it, `script_object_to_human` (0xA52C70, ESI = name). Returns EAX as the callers test it.
unsafe fn find_obj(name: u32) -> u32 {
    unsafe {
        let mut found = 0u32;
        let o = call_regs(0x00A5_2BB0, &mut found as *mut u32 as u32, name, 0, 0, 0, &[]);
        if found & 0xFF != 0 {
            o
        } else {
            call_regs(0x00A5_2C70, o, 0, 0, name, 0, &[])
        }
    }
}

/// `string_crc32` (0xBDCA30, stdcall, EAX = &out, EDX = 0) then `navpoint_find` (0xA5EB50,
/// EAX = &hash, still the pointer the crc call received).
unsafe fn navpoint(name: u32) -> u32 {
    unsafe {
        let mut h = 0u32;
        let pa = &mut h as *mut u32 as u32;
        call_regs(0x00BD_CA30, pa, 0, 0, 0, 0, &[name]);
        call_regs(0x00A5_EB50, pa, 0, 0, 0, 0, &[])
    }
}

/// A virtual call `this->vtable[slot](...)`: ECX = this, EDX = vtable, EAX = the function.
unsafe fn vcall(this: u32, slot: usize) -> u32 {
    unsafe {
        let vt = rd(this as usize);
        let f = rd(vt as usize + slot * 4);
        call_regs(f as usize, f, this, vt, 0, 0, &[])
    }
}

// --- shot functions with callees ------------------------------------------------------------------

// 0x6D7510 spawning_peds_disabled: b8 == 1 -> clear the ped flags, run b7da30, set the dirty byte.
unsafe fn spawning_peds(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            hit(27, 0);
            wb(0x0215_DA25, 0);
            wb(0x0215_DA05, 0);
            call(0x00B7_DA30);
            wb(0x0252_9620, 1);
        } else {
            hit(27, 1);
            wb(0x0215_DA25, 1);
            wb(0x0215_DA05, 1);
            call(0x00B7_DA30);
        }
    }
}
cp!(spawning_peds_w, spawning_peds_t, 27, spawning_peds);

// 0x6D7550 spawning_cars_disabled
unsafe fn spawning_cars(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            hit(28, 0);
            wb(0x0215_DA1D, 0);
            call(0x0093_C660);
            wb(0x0215_D9FD, 0);
            call(0x00B8_1130);
            wb(0x0252_9620, 1);
        } else {
            hit(28, 1);
            wb(0x0215_D9FD, 1);
            call(0x00B8_1130);
            wb(0x0252_9620, 1);
        }
    }
}
cp!(spawning_cars_w, spawning_cars_t, 28, spawning_cars);

// 0x6D76D0 spawning_remove_all
unsafe fn spawning_remove_all(rec: u32) {
    unsafe {
        if b8(rec) == 1 {
            hit(29, 0);
            wb(0x0215_DA25, 0);
            wb(0x0215_DA1D, 0);
            call(0x0093_C660);
            wb(0x0215_DA05, 0);
            call(0x00B7_DA30);
            wb(0x0215_D9FD, 0);
            call(0x00B8_1130);
            wb(0x0252_9620, 1);
        }
    }
}
cp!(spawning_remove_all_w, spawning_remove_all_t, 29, spawning_remove_all);

// 0x6D7A60 hide_player_compnt: EDI = name -> 0x9E7530 (index or -1); then 0x9EA4E0 / 0x9EA660.
unsafe fn hide_player_compnt(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        let idx = call_regs(0x009E_7530, 0, 0, 0, 0, name, &[]);
        if idx != 0xFFFF_FFFF {
            hit(30, 0);
            let p = rd(0x0217_03D4);
            let v = rd(p as usize + 0x13B0);
            call_regs(0x009E_A4E0, idx, p, v, 0, 0, &[v, idx]);
        } else {
            hit(30, 1);
            let p = rd(0x0217_03D4);
            let v = rd(p as usize + 0x13B0);
            call_regs(0x009E_A660, p, v, 0, 0, 0, &[v]);
        }
    }
}
cp!(hide_player_compnt_w, hide_player_compnt_t, 30, hide_player_compnt);

// 0x6D8140 time_of_day_set: ECX = int(v) for 0x68A5C0.
unsafe fn time_of_day_set(rec: u32) {
    unsafe {
        let n = ftol(f8(rec) as f64) as u32;
        call_regs(0x0068_A5C0, n, n, 0, 0, 0, &[]);
    }
}
cp!(time_of_day_set_w, time_of_day_set_t, 31, time_of_day_set);

// 0x6D7AA0 bink_play: n = int(v); n >= 1 -> set volume (import 0xDB8400) and pause(0)
// (0xDB83D0); n < 0 -> pause(1); n == 0 -> nothing. Handle at 0x140E670.
unsafe fn bink_play(rec: u32) {
    unsafe {
        let n = ftol(f8(rec) as f64);
        let h = rd(0x0140_E670);
        if n >= 1 {
            hit(32, 0);
            if h != 0 {
                call_regs(import(0x00DB_8400), 0, 0, 0, 0, 0, &[h, n as u32, 1]);
                call_regs(import(0x00DB_83D0), 0, 0, 0, 0, 0, &[h, 0]);
            }
        } else if n < 0 {
            hit(32, 1);
            if h != 0 {
                call_regs(import(0x00DB_83D0), 0, 0, 0, 0, 0, &[h, 1]);
            }
        }
    }
}
cp!(bink_play_w, bink_play_t, 32, bink_play);

// 0x6D7AF0 / 0x6D7B10 mesh_mover_hide / show: door by name (0xA5B6E0, EAX = name) -> ESI = door.
unsafe fn mesh_mover_hide(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        if name != 0 {
            let d = call_regs(0x00A5_B6E0, name, 0, 0, 0, 0, &[]);
            if d != 0 {
                hit(33, 0);
                call_regs(0x00A5_B800, d, 0, 0, d, 0, &[]);
            }
        }
    }
}
cp!(mesh_mover_hide_w, mesh_mover_hide_t, 33, mesh_mover_hide);

unsafe fn mesh_mover_show(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        if name != 0 {
            let d = call_regs(0x00A5_B6E0, name, 0, 0, 0, 0, &[]);
            if d != 0 {
                hit(34, 0);
                call_regs(0x00A5_B970, d, 0, 0, d, 0, &[]);
            }
        }
    }
}
cp!(mesh_mover_show_w, mesh_mover_show_t, 34, mesh_mover_show);

// 0x6D8280 chunk_hide: EDI = name -> 0x4A9770 (chunk); bit 0 of chunk+0x1D set; the name pointer
// (rec + 8) is added to the player's list at +0x1EE4 (0x6D8A10, ECX = list, stdcall 1 arg).
unsafe fn chunk_hide(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        let c = call_regs(0x004A_9770, 0, 0, 0, 0, name, &[]);
        if c != 0 {
            hit(35, 0);
            let list = rd(PLAYER_PTR) + 0x1EE4;
            wb(c as usize + 0x1D, rb(c as usize + 0x1D) | 1);
            call_regs(0x006D_8A10, c, list, 0, 0, 0, &[rec + 8]);
        }
    }
}
cp!(chunk_hide_w, chunk_hide_t, 35, chunk_hide);

// 0x6D82B0 chunk_show
unsafe fn chunk_show(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        let c = call_regs(0x004A_9770, 0, 0, 0, 0, name, &[]);
        if c != 0 {
            hit(36, 0);
            wb(c as usize + 0x1D, rb(c as usize + 0x1D) & 0xFE);
        }
    }
}
cp!(chunk_show_w, chunk_show_t, 36, chunk_show);

// 0x6D7DB0 weather_stage: index = 0x4DEE30(name) (-1 when unknown); player+0x43D4 = 1; the forced
// weather is applied through 0x4DEC00 (EAX = index, CL = 1).
unsafe fn weather_stage(rec: u32) {
    unsafe {
        let name = rd(rec as usize + 8);
        if name != 0 {
            let idx = call_regs(0x004D_EE30, 0, 0, 0, 0, 0, &[name]);
            if idx != 0xFFFF_FFFF {
                hit(37, 0);
                wb(rd(PLAYER_PTR) as usize + 0x43D4, 1);
                call_regs(0x004D_EC00, idx, 1, 0, 0, 0, &[]);
            }
        }
    }
}
cp!(weather_stage_w, weather_stage_t, 37, weather_stage);

// 0x6D7750 shot_fade_in: when 0.0 < v: 0x4D1270(int(v * 1000.0), 0.0f) with DL = 0, then
// player+0xB03 = 1 unless player+0xB02 is set.
unsafe fn shot_fade_in(rec: u32) {
    unsafe {
        let v = f8(rec);
        if 0.0f64 < v as f64 {
            hit(38, 0);
            let ms = ftol(v as f64 * *(0x00E5_C0D8 as *const f64));
            call_regs(0x004D_1270, 0, 0, 0, 0, 0, &[ms as u32, 0]);
            let p = pl();
            if rb(p + 0xB02) == 0 {
                hit(38, 1);
                wb(p + 0xB03, 1);
            }
        }
    }
}
cp!(shot_fade_in_w, shot_fade_in_t, 38, shot_fade_in);

// --- actions --------------------------------------------------------------------------------------

const CAM_SET: usize = 0x004A_29E0; // EAX = pos ptr, ECX = flag/ptr, ESI = target object, EDI = orient ptr
const CAM_POS: u32 = 0x025F_5B20;

// 0x6CB680 cam_stop_track
unsafe fn act_cam_stop_track(_rec: u32) {
    unsafe {
        call_regs(CAM_SET, CAM_POS, 0, 0, 0, 0, &[]);
    }
}
cp!(act_cam_stop_track_w, act_cam_stop_track_t, 39, act_cam_stop_track);

// 0x6CB2E0 cam_track_char
unsafe fn act_cam_track_char(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 0xC);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(40, 0);
                call_regs(CAM_SET, CAM_POS, 0x00DD_80C8, 0, o, 0, &[]);
            }
        }
    }
}
cp!(act_cam_track_char_w, act_cam_track_char_t, 40, act_cam_track_char);

// 0x6CB330 cam_look_at_char
unsafe fn act_cam_look_at_char(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 0xC);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(41, 0);
                call_regs(CAM_SET, CAM_POS, 0x00DD_80C8, 0, o, 0, &[]);
                call_regs(CAM_SET, CAM_POS, 0, 0, 0, 0, &[]);
            }
        }
    }
}
cp!(act_cam_look_at_char_w, act_cam_look_at_char_t, 41, act_cam_look_at_char);

// 0x6CB6E0 / 0x6CB730 hide_character / show_character: 0x926860 (ECX = human, args (a, 1, b)).
unsafe fn act_hide_character(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(42, 0);
                call_regs(0x0092_6860, 0, o, 0, n, 0, &[1, 1, 0]);
            }
        }
    }
}
cp!(act_hide_character_w, act_hide_character_t, 42, act_hide_character);

unsafe fn act_show_character(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(43, 0);
                call_regs(0x0092_6860, 0, o, 0, n, 0, &[0, 1, 0]);
            }
        }
    }
}
cp!(act_show_character_w, act_show_character_t, 43, act_show_character);

// 0x6CB780 disable_physics
unsafe fn act_disable_physics(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(44, 0);
                call_regs(0x006F_C610, 0, 0, 0, o, 0, &[0x14, 0]);
                let e = rd(o as usize + 0x570);
                if e != 0 {
                    wb(e as usize + 0x130, rb(e as usize + 0x130) | 2);
                }
                wd(o as usize + 0xCC, rd(o as usize + 0xCC) | 0x40_0000);
            }
        }
    }
}
cp!(act_disable_physics_w, act_disable_physics_t, 44, act_disable_physics);

// 0x6CB7E0 enable_physics
unsafe fn act_enable_physics(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(45, 0);
                call_regs(0x006F_C610, 0, 0, 0, o, 0, &[0xF, 0]);
                let e = rd(o as usize + 0x570);
                if e != 0 {
                    wb(e as usize + 0x130, rb(e as usize + 0x130) & 0xFD);
                }
                call_regs(0x0095_F090, o, 0, 0, 0, 0, &[o + 0x14, 0, 0xFFFF_FFFF, 0]);
                wd(o as usize + 0xCC, rd(o as usize + 0xCC) & 0xFFBF_FFFF);
            }
        }
    }
}
cp!(act_enable_physics_w, act_enable_physics_t, 45, act_enable_physics);

// 0x6CB850 hide_vehicle: 0xA70030 (EAX = name) -> object; vslot 0x2C (is-vehicle-like, AL) then
// vslot 0x34 -> the vehicle; 0xAACC60(handle, 1, 0).
unsafe fn act_hide_vehicle(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        let o = call_regs(0x00A7_0030, n, 0, 0, 0, 0, &[]);
        if o != 0 && (vcall(o, 0x2C / 4) & 0xFF) != 0 {
            hit(46, 0);
            let v = vcall(o, 0x34 / 4);
            let h = rd(v as usize + 0x44);
            call_regs(0x00AA_CC60, 0, 0, 0, 0, 0, &[h, 1, 0]);
        }
    }
}
cp!(act_hide_vehicle_w, act_hide_vehicle_t, 46, act_hide_vehicle);

// 0x6CB890 show_vehicle: as hide, 0xAACC60(handle, 0, 0), then a tail call of vslot 0x20 of the vehicle.
unsafe fn act_show_vehicle(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        let o = call_regs(0x00A7_0030, n, 0, 0, 0, 0, &[]);
        if o != 0 && (vcall(o, 0x2C / 4) & 0xFF) != 0 {
            hit(47, 0);
            let v = vcall(o, 0x34 / 4);
            let h = rd(v as usize + 0x44);
            call_regs(0x00AA_CC60, 0, 0, 0, 0, 0, &[h, 0, 0]);
            vcall(v, 0x20 / 4);
        }
    }
}
cp!(act_show_vehicle_w, act_show_vehicle_t, 47, act_show_vehicle);

// 0x6CB8E0 / 0x6CB900 hide_group / show_group: 0xA57730 (EAX = name) -> group; 0xA58070 (EDI = group, 1 arg).
unsafe fn act_hide_group(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let g = call_regs(0x00A5_7730, n, 0, 0, 0, 0, &[]);
            if g != 0 {
                hit(48, 0);
                call_regs(0x00A5_8070, 0, 0, 0, 0, g, &[1]);
            }
        }
    }
}
cp!(act_hide_group_w, act_hide_group_t, 48, act_hide_group);

unsafe fn act_show_group(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let g = call_regs(0x00A5_7730, n, 0, 0, 0, 0, &[]);
            if g != 0 {
                hit(49, 0);
                call_regs(0x00A5_8070, 0, 0, 0, 0, g, &[0]);
            }
        }
    }
}
cp!(act_show_group_w, act_show_group_t, 49, act_show_group);

// 0x6CB250 look_at_nothing
unsafe fn act_look_at_nothing(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n != 0 {
            let o = find_obj(n);
            if o != 0 {
                hit(50, 0);
                wd(o as usize + 0x758, rd(o as usize + 0x758) | 1);
                if rb(o as usize + 0x3CC) == 2 {
                    hit(50, 1);
                    wb(o as usize + 0x140, rb(o as usize + 0x140) & 0xFB);
                }
            }
        }
    }
}
cp!(act_look_at_nothing_w, act_look_at_nothing_t, 50, act_look_at_nothing);

// 0x6CB6A0 cam_follow_target: v = f32(_atof(rec[+0x10])); [25F5D90] = (v > 0.0); [25F5D94] = v.
unsafe fn act_cam_follow_target(rec: u32) {
    unsafe {
        let v = atof(rd(rec as usize + 0x10)) as f32;
        wb(0x025F_5D90, 1);
        wf(0x025F_5D94, v);
        if !(v as f64 > 0.0) {
            hit(51, 0);
            wb(0x025F_5D90, 0);
        }
    }
}
cp!(act_cam_follow_target_w, act_cam_follow_target_t, 51, act_cam_follow_target);

// 0x6CB2A0 cam_cut_to: navpoint (rec[+0xC]) -> camera set (EAX = nav+0x1C, EDI = nav+0x28, ECX =
// ESI = 0), then cam_stop_slide (0x6CB5E0) with the record as its (ignored) argument.
unsafe fn act_cam_cut_to(rec: u32) {
    unsafe {
        let nav = navpoint(rd(rec as usize + 0xC));
        if nav != 0 {
            hit(52, 0);
            call_regs(CAM_SET, nav + 0x1C, 0, 0, 0, nav + 0x28, &[]);
            call_regs(0x006C_B5E0, 0, 0, 0, 0, 0, &[rec]);
        }
    }
}
cp!(act_cam_cut_to_w, act_cam_cut_to_t, 52, act_cam_cut_to);

// 0x6CB110 look_at: both objects by name; fields of the first's look record at +0x730.
unsafe fn act_look_at(rec: u32) {
    unsafe {
        let n1 = rd(rec as usize + 8);
        if n1 == 0 {
            return;
        }
        let a = find_obj(n1);
        if a == 0 {
            return;
        }
        let n2 = rd(rec as usize + 0xC);
        if n2 == 0 {
            return;
        }
        let b = find_obj(n2);
        if b == 0 {
            return;
        }
        hit(53, 0);
        let (a, b) = (a as usize, b as usize);
        wd(a + 0x758, rd(a + 0x758) & 0xFFFF_FFFE);
        let h = rd(b + 0x44);
        let p = a + 0x730;
        wd(p + 0x30, rd(p + 0x30) & 0xFFFF_FFFE);
        wf(p, 0.0);
        wd(p + 0xC, h);
        wf(p + 4, 0.0);
        wf(p + 8, 0.0);
        wb(p + 0x2C, rb(p + 0x2C) | 1);
        wf(p + 0x1C, q(rf(0x00DD_0C54)));
        wd(p + 0x18, 0xFFFF_FFFF);
        if rb(a + 0x3CC) == 2 {
            hit(53, 1);
            wb(a + 0x140, rb(a + 0x140) | 4);
        }
    }
}
cp!(act_look_at_w, act_look_at_t, 53, act_look_at);

// 0x6CB1D0 look_at_nav: object + navpoint; 0x9B5C00(obj, -1.0f) with EAX = nav & ~0xFF,
// EDX = nav + 0x1C, ESI = 0.
unsafe fn act_look_at_nav(rec: u32) {
    unsafe {
        let n1 = rd(rec as usize + 8);
        if n1 == 0 {
            return;
        }
        let a = find_obj(n1);
        if a == 0 {
            return;
        }
        let n2 = rd(rec as usize + 0xC);
        if n2 == 0 {
            return;
        }
        let nav = navpoint(n2);
        if nav == 0 {
            return;
        }
        hit(54, 0);
        let m1 = q(rf(0x00E8_7998));
        wd(a as usize + 0x758, rd(a as usize + 0x758) & 0xFFFF_FFFE);
        call_regs(0x009B_5C00, nav & !0xFF, 0, nav + 0x1C, 0, 0, &[a, m1.to_bits()]);
    }
}
cp!(act_look_at_nav_w, act_look_at_nav_t, 54, act_look_at_nav);

// 0x6CAA80 teleports_to
unsafe fn act_teleports_to(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n == 0 {
            return;
        }
        let o = find_obj(n);
        if o == 0 {
            return;
        }
        let nav = navpoint(rd(rec as usize + 0xC));
        if nav == 0 {
            return;
        }
        hit(55, 0);
        let mut eax = nav;
        if rb(o as usize + 0x3CC) == 2 {
            hit(55, 1);
            eax = call_regs(0x0044_7CD0, 0, 0, 0, 0, 0, &[o, 0, 0]);
        }
        call_regs(0x009A_0B00, eax, nav + 0x28, nav + 0x1C, nav, o, &[o, 0, 1, 0, 0]);
    }
}
cp!(act_teleports_to_w, act_teleports_to_t, 55, act_teleports_to);

// 0x6CAB00 walk_to / 0x6CAB70 run_to: object, navpoint; [0x2527D28] = 3 / 7; 0x6CA560(nav, 1 / 2)
// with ESI = object, EDI = the record.
unsafe fn walk_run(rec: u32, idx: usize, state: u32, mode: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n == 0 {
            return;
        }
        let o = find_obj(n);
        if o == 0 {
            return;
        }
        let nav = navpoint(rd(rec as usize + 0xC));
        if nav == 0 {
            return;
        }
        hit(idx, 0);
        wd(0x0252_7D28, state);
        call_regs(0x006C_A560, nav, 0, 0, o, rec, &[nav, mode]);
    }
}
unsafe fn act_walk_to(rec: u32) {
    unsafe { walk_run(rec, 56, 3, 1) }
}
cp!(act_walk_to_w, act_walk_to_t, 56, act_walk_to);
unsafe fn act_run_to(rec: u32) {
    unsafe { walk_run(rec, 57, 7, 2) }
}
cp!(act_run_to_w, act_run_to_t, 57, act_run_to);

// 0x6CACD0 suicides: 0x9847A0(obj, -1.0f, 0, 0) with EDX = ECX = 0; its ST0 result is popped.
unsafe fn act_suicides(rec: u32) {
    unsafe {
        let n = rd(rec as usize + 8);
        if n == 0 {
            return;
        }
        let o = find_obj(n);
        if o == 0 {
            return;
        }
        hit(58, 0);
        let m1 = q(rf(0x00E8_7998));
        call_regs_st0(0x0098_47A0, o, 0, 0, n, 0, &[o, m1.to_bits(), 0, 0]);
    }
}
cp!(act_suicides_w, act_suicides_t, 58, act_suicides);

// 0x6CB5E0 cam_stop_slide: resets the camera-slide record (player+0x1540 + 0x1C) and the two
// 9-dword blocks at player+0x158C / +0x15B4. Shadowed.
unsafe fn act_cam_stop_slide(_rec: u32) {
    unsafe {
        let p = rd(PLAYER_PTR) as usize;
        if p == 0 {
            return;
        }
        let s = rd(p + 0x1540) as usize + 0x1C;
        wb(s, 0);
        let (a, b, c) = (rd(0x025F_5B20), rd(0x025F_5B24), rd(0x025F_5B28));
        wd(s + 0xC, a);
        wd(s + 0x10, b);
        wd(s + 0x14, c);
        wd(s + 0x1C, a);
        wd(s + 0x20, b);
        wd(s + 0x24, c);
        wf(s + 0x18, 0.0);
        wf(s + 8, 0.0);
        wb(p + 0x1580, 0);
        copy_dwords(p + 0x158C, 0x025F_5B5C, 9);
        copy_dwords(p + 0x15B4, 0x025F_5B5C, 9);
        wf(p + 0x15B0, 0.0);
        wf(p + 0x1588, 0.0);
    }
}
unsafe fn act_cam_stop_slide_r(_: u32) -> Vec<(usize, usize)> {
    unsafe {
        let p = rd(PLAYER_PTR) as usize;
        if p == 0 {
            return vec![];
        }
        vec![(rd(p + 0x1540) as usize + 0x1C, 0x28), (p + 0x1580, 0x58)]
    }
}
sp!(act_cam_stop_slide_w, act_cam_stop_slide_t, 59, act_cam_stop_slide, act_cam_stop_slide_r);

// 0x6CB590 cam_slide_to_update: AL = 0 only when the slide is active and [s+0x18] > [0x27E6710]
// (or unordered); EAX is exactly 0 / 1.
unsafe fn act_cam_slide_update(_rec: u32) -> u32 {
    unsafe {
        let p = rd(PLAYER_PTR) as usize;
        if p == 0 {
            return 1;
        }
        let s = rd(p + 0x1540) as usize + 0x1C;
        if rb(s) == 0 {
            return 1;
        }
        let a = rf(s + 0x18) as f64;
        let b = rf(0x027E_6710) as f64;
        // fcompp ST0 = [s+0x18], ST1 = [..6710]; test ah, 0x41; jnp -> 1: not greater and ordered.
        if a > b || a.is_nan() || b.is_nan() { 0 } else { 1 }
    }
}
extern "C" fn act_cam_slide_update_w(rec: u32) -> u32 {
    let st = &ST[60];
    let n = st.calls.fetch_add(1, Ordering::Relaxed);
    unsafe {
        let r = act_cam_slide_update(rec);
        if n < SHADOW_CALLS {
            let orig = st.orig.load(Ordering::Relaxed);
            if orig != 0 {
                let o = call_regs(orig, 0, 0, 0, 0, 0, &[rec]);
                if o != r {
                    st.mism.fetch_add(1, Ordering::Relaxed);
                    return o;
                }
            }
        }
        r
    }
}
keep_ecx_edx_arg1!(act_cam_slide_update_t, act_cam_slide_update_w);

// level_lights 0x6d7590..0x6d75a2 size 0x12
const B_LEVEL_LIGHTS: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x00, 0x0F, 0x94, 0xC1, 0x88, 0x0D, 0xC9, 0x95, 0x52,
    0x02, 0xC3,
];
const B_LEVEL_LIGHTS_CALLS: [usize; 0] = [];
// tree_shadows 0x6d75b0..0x6d75c5 size 0x15
const B_TREE_SHADOWS: [u8; 21] = [
    0x8B, 0x4C, 0x24, 0x04, 0x33, 0xC0, 0x38, 0x41, 0x08, 0x0F, 0x95, 0xC0, 0x83, 0xC0, 0x01, 0xA3,
    0x94, 0x89, 0xE9, 0x00, 0xC3,
];
const B_TREE_SHADOWS_CALLS: [usize; 0] = [];
// tod_char_shadows 0x6d75d0..0x6d75de size 0xe
const B_TOD_CHAR_SHADOWS: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x08, 0x88, 0x0D, 0x4E, 0x7D, 0xE9, 0x00, 0xC3,
];
const B_TOD_CHAR_SHADOWS_CALLS: [usize; 0] = [];
// motion_blur 0x6d75e0..0x6d75f8 size 0x18
const B_MOTION_BLUR: [u8; 24] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x00, 0x8B, 0x15, 0x10, 0x7D, 0x52, 0x02, 0x0F, 0x94,
    0xC1, 0x88, 0x8A, 0xE0, 0x15, 0x00, 0x00, 0xC3,
];
const B_MOTION_BLUR_CALLS: [usize; 0] = [];
// motion_blur_override 0x6d7600..0x6d7624 size 0x24
const B_MOTION_BLUR_OVERRIDE: [u8; 36] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x8B, 0x10, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0x81,
    0xC1, 0xE4, 0x15, 0x00, 0x00, 0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40, 0x08,
    0x89, 0x41, 0x08, 0xC3,
];
const B_MOTION_BLUR_OVERRIDE_CALLS: [usize; 0] = [];
// camera_moves 0x6d7630..0x6d763e size 0xe
const B_CAMERA_MOVES: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x08, 0x88, 0x0D, 0x90, 0x5D, 0x5F, 0x02, 0xC3,
];
const B_CAMERA_MOVES_CALLS: [usize; 0] = [];
// disable_trees 0x6d7640..0x6d7652 size 0x12
const B_DISABLE_TREES: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x00, 0x0F, 0x94, 0xC1, 0x88, 0x0D, 0xBB, 0x78, 0xE8,
    0x00, 0xC3,
];
const B_DISABLE_TREES_CALLS: [usize; 0] = [];
// city_no_render_al 0x6d7660..0x6d766e size 0xe
const B_CITY_NO_RENDER_AL: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x08, 0x88, 0x0D, 0xC5, 0x6C, 0x52, 0x02, 0xC3,
];
const B_CITY_NO_RENDER_AL_CALLS: [usize; 0] = [];
// shader_lod_force 0x6d7670..0x6d76a1 size 0x31
const B_SHADER_LOD_FORCE: [u8; 49] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x01, 0x75, 0x26, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0x83,
    0xB8, 0x5C, 0x16, 0x00, 0x00, 0xFE, 0x75, 0x0C, 0x8B, 0x0D, 0xD0, 0x49, 0xE8, 0x00, 0x89, 0x88,
    0x5C, 0x16, 0x00, 0x00, 0x33, 0xC0, 0xA2, 0xC4, 0x6D, 0x52, 0x02, 0xA3, 0xD0, 0x49, 0xE8, 0x00,
    0xC3,
];
const B_SHADER_LOD_FORCE_CALLS: [usize; 0] = [];
// city_shadow_dist 0x6d76b0..0x6d76be size 0xe
const B_CITY_SHADOW_DIST: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xD9, 0x1D, 0xA0, 0x49, 0xE8, 0x00, 0xC3,
];
const B_CITY_SHADOW_DIST_CALLS: [usize; 0] = [];
// far_clip 0x6d7720..0x6d773a size 0x1a
const B_FAR_CLIP: [u8; 26] = [
    0xD9, 0xEE, 0x8B, 0x4C, 0x24, 0x04, 0xD8, 0x59, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x09,
    0xD9, 0x41, 0x08, 0xD9, 0x1D, 0x88, 0x89, 0xE9, 0x00, 0xC3,
];
const B_FAR_CLIP_CALLS: [usize; 0] = [];
// near_clip 0x6d7740..0x6d774e size 0xe
const B_NEAR_CLIP: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xD9, 0x1D, 0x8C, 0x89, 0xE9, 0x00, 0xC3,
];
const B_NEAR_CLIP_CALLS: [usize; 0] = [];
// shot_fade_out 0x6d77a0..0x6d77d4 size 0x34
const B_SHOT_FADE_OUT: [u8; 52] = [
    0xD9, 0xEE, 0x8B, 0x4C, 0x24, 0x04, 0xD8, 0x59, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x23,
    0xA1, 0x10, 0x7D, 0x52, 0x02, 0xD9, 0x41, 0x08, 0x8B, 0x90, 0x40, 0x15, 0x00, 0x00, 0xD9, 0x9A,
    0x64, 0x01, 0x00, 0x00, 0x8B, 0x80, 0x40, 0x15, 0x00, 0x00, 0xD9, 0x41, 0x08, 0xD9, 0x98, 0x68,
    0x01, 0x00, 0x00, 0xC3,
];
const B_SHOT_FADE_OUT_CALLS: [usize; 0] = [];
// shot_fade_out_midway 0x6d77e0..0x6d7833 size 0x53
const B_SHOT_FADE_OUT_MIDWAY: [u8; 83] = [
    0xD9, 0xEE, 0x8B, 0x4C, 0x24, 0x04, 0xD8, 0x59, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x42,
    0xD9, 0x05, 0x14, 0x67, 0x7E, 0x02, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0xD8, 0x25, 0x10, 0x67, 0x7E,
    0x02, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x61, 0x08, 0x8B, 0x88, 0x40, 0x15,
    0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x64, 0x01, 0x00, 0x00,
    0x8B, 0x90, 0x40, 0x15, 0x00, 0x00, 0xD9, 0x05, 0x8C, 0x1C, 0xDD, 0x00, 0xD9, 0x9A, 0x68, 0x01,
    0x00, 0x00, 0xC3,
];
const B_SHOT_FADE_OUT_MIDWAY_CALLS: [usize; 0] = [];
// shot_cut_out_midway 0x6d7840..0x6d7890 size 0x50
const B_SHOT_CUT_OUT_MIDWAY: [u8; 80] = [
    0xD9, 0xEE, 0x8B, 0x4C, 0x24, 0x04, 0xD8, 0x51, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x3D,
    0xD9, 0x05, 0x14, 0x67, 0x7E, 0x02, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0xD8, 0x25, 0x10, 0x67, 0x7E,
    0x02, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x61, 0x08, 0x8B, 0x88, 0x40, 0x15,
    0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x99, 0x64, 0x01, 0x00, 0x00,
    0x8B, 0x90, 0x40, 0x15, 0x00, 0x00, 0xD9, 0x9A, 0x68, 0x01, 0x00, 0x00, 0xC3, 0xDD, 0xD8, 0xC3,
];
const B_SHOT_CUT_OUT_MIDWAY_CALLS: [usize; 0] = [];
// letterbox_fade_in 0x6d7890..0x6d78c9 size 0x39
const B_LETTERBOX_FADE_IN: [u8; 57] = [
    0x8B, 0x4C, 0x24, 0x04, 0xB0, 0x01, 0x38, 0x41, 0x08, 0x75, 0x2D, 0xD9, 0xEE, 0xA2, 0x5A, 0xC8,
    0x40, 0x01, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0xD9, 0x1D, 0x04, 0x69, 0xF7, 0x01, 0x05, 0xE8, 0x03,
    0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x68, 0xC4, 0xEA, 0x00, 0x7E, 0x0A, 0x2D, 0x00,
    0xD2, 0x49, 0x6B, 0xA3, 0x68, 0xC4, 0xEA, 0x00, 0xC3,
];
const B_LETTERBOX_FADE_IN_CALLS: [usize; 0] = [];
// letterbox_fade_out 0x6d78d0..0x6d78ee size 0x1e
const B_LETTERBOX_FADE_OUT: [u8; 30] = [
    0x8B, 0x4C, 0x24, 0x04, 0xB0, 0x01, 0x38, 0x41, 0x08, 0x75, 0x12, 0x8B, 0x15, 0x10, 0x7D, 0x52,
    0x02, 0x8B, 0x8A, 0x40, 0x15, 0x00, 0x00, 0x88, 0x81, 0x6C, 0x01, 0x00, 0x00, 0xC3,
];
const B_LETTERBOX_FADE_OUT_CALLS: [usize; 0] = [];
// framebuffer_tint 0x6d78f0..0x6d79da size 0xea
const B_FRAMEBUFFER_TINT: [u8; 234] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x41, 0x08, 0x83, 0xEC, 0x34, 0x80, 0x38, 0x00, 0x75, 0x10, 0x80,
    0x78, 0x01, 0x00, 0x75, 0x0A, 0x80, 0x78, 0x02, 0x00, 0x0F, 0x84, 0xC7, 0x00, 0x00, 0x00, 0x0F,
    0xB6, 0x10, 0xD9, 0x40, 0x04, 0xD9, 0x1C, 0x24, 0x89, 0x54, 0x24, 0x38, 0x0F, 0xB6, 0x50, 0x01,
    0xDB, 0x44, 0x24, 0x38, 0xDD, 0x05, 0x38, 0xC0, 0xE5, 0x00, 0x0F, 0xB6, 0x40, 0x02, 0xDC, 0xF9,
    0x56, 0x57, 0x8B, 0x3D, 0x10, 0x7D, 0x52, 0x02, 0x81, 0xC7, 0x5C, 0x15, 0x00, 0x00, 0x8D, 0x74,
    0x24, 0x18, 0xC6, 0x44, 0x24, 0x1C, 0x01, 0xC7, 0x44, 0x24, 0x18, 0x03, 0x00, 0x00, 0x00, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x44, 0x24, 0x40, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0x89,
    0x54, 0x24, 0x40, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x54, 0x24, 0x0C, 0xDB,
    0x44, 0x24, 0x40, 0x89, 0x54, 0x24, 0x20, 0xD8, 0xF2, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x44, 0x24,
    0x40, 0xD8, 0xC9, 0x89, 0x44, 0x24, 0x40, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x44, 0x24, 0x10, 0xDB,
    0x44, 0x24, 0x40, 0x89, 0x44, 0x24, 0x24, 0x8B, 0x41, 0x08, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xDE,
    0xF2, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x40, 0xD8, 0x4C, 0x24, 0x40, 0xD9, 0x5C, 0x24, 0x14, 0x8B,
    0x54, 0x24, 0x14, 0xD9, 0xE8, 0x89, 0x54, 0x24, 0x28, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x40, 0x0C,
    0xD9, 0x5C, 0x24, 0x30, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x40, 0x10, 0xD9, 0x5C,
    0x24, 0x38, 0xF3, 0xA5, 0x5F, 0x5E, 0x83, 0xC4, 0x34, 0xC3,
];
const B_FRAMEBUFFER_TINT_CALLS: [usize; 0] = [];
// force_tod_lights 0x6d79e0..0x6d79f5 size 0x15
const B_FORCE_TOD_LIGHTS: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x08, 0xC6, 0x05, 0xC3, 0x6C, 0x52, 0x02, 0x01, 0x88, 0x0D,
    0xC4, 0x6C, 0x52, 0x02, 0xC3,
];
const B_FORCE_TOD_LIGHTS_CALLS: [usize; 0] = [];
// lightning 0x6d7d70..0x6d7da6 size 0x36
const B_LIGHTNING: [u8; 54] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x00, 0x74, 0x21, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05,
    0x88, 0x13, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x48, 0xA8, 0xE9, 0x00, 0x7E, 0x15,
    0x2D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x48, 0xA8, 0xE9, 0x00, 0xC3, 0xC7, 0x05, 0x48, 0xA8, 0xE9,
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B_LIGHTNING_CALLS: [usize; 0] = [];
// glares 0x6d8120..0x6d8132 size 0x12
const B_GLARES: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x00, 0x0F, 0x95, 0xC1, 0x88, 0x0D, 0x56, 0x7D, 0xE9,
    0x00, 0xC3,
];
const B_GLARES_CALLS: [usize; 0] = [];
// fog_distance 0x6d8170..0x6d81e0 size 0x70
const B_FOG_DISTANCE: [u8; 112] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x8B, 0x48, 0x08, 0xD8, 0x11, 0x83, 0xEC, 0x0C, 0xDF, 0xE0,
    0xF6, 0xC4, 0x41, 0x7A, 0x4E, 0xD8, 0x51, 0x04, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7A, 0x44, 0xD8,
    0x59, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7A, 0x3C, 0xD9, 0x01, 0xD9, 0x1C, 0x24, 0xD9, 0x41,
    0x04, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x04, 0xD9, 0x41, 0x08, 0x8B, 0x0C, 0x24, 0xD9,
    0x5C, 0x24, 0x08, 0x8B, 0x44, 0x24, 0x08, 0xC6, 0x05, 0xA4, 0x8C, 0x65, 0x02, 0x01, 0x89, 0x0D,
    0xA8, 0x8C, 0x65, 0x02, 0x89, 0x15, 0xAC, 0x8C, 0x65, 0x02, 0xA3, 0xB0, 0x8C, 0x65, 0x02, 0x83,
    0xC4, 0x0C, 0xC3, 0xDD, 0xD8, 0xC6, 0x05, 0xA4, 0x8C, 0x65, 0x02, 0x00, 0x83, 0xC4, 0x0C, 0xC3,
];
const B_FOG_DISTANCE_CALLS: [usize; 0] = [];
// city_region_blend 0x6d81e0..0x6d81ee size 0xe
const B_CITY_REGION_BLEND: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x08, 0x88, 0x0D, 0xE6, 0x96, 0xE9, 0x00, 0xC3,
];
const B_CITY_REGION_BLEND_CALLS: [usize; 0] = [];
// city_render_fade 0x6d81f0..0x6d81fe size 0xe
const B_CITY_RENDER_FADE: [u8; 14] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xD9, 0x1D, 0xB4, 0x96, 0xE9, 0x00, 0xC3,
];
const B_CITY_RENDER_FADE_CALLS: [usize; 0] = [];
// drug_effect 0x6d8200..0x6d8222 size 0x22
const B_DRUG_EFFECT: [u8; 34] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x08, 0xD9, 0x01, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0xD9,
    0x99, 0xD8, 0x15, 0x00, 0x00, 0x8B, 0x50, 0x08, 0xD9, 0x42, 0x04, 0xD9, 0x99, 0xDC, 0x15, 0x00,
    0x00, 0xC3,
];
const B_DRUG_EFFECT_CALLS: [usize; 0] = [];
// rain_override 0x6d8240..0x6d827a size 0x3a
const B_RAIN_OVERRIDE: [u8; 58] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0xEE, 0xD9, 0x44, 0x24,
    0x04, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x75, 0x10, 0xDD, 0xD9, 0xC6, 0x05, 0x78, 0x6D,
    0x52, 0x02, 0x01, 0xD9, 0x1D, 0x7C, 0x6D, 0x52, 0x02, 0xC3, 0xDD, 0xD8, 0xC6, 0x05, 0x78, 0x6D,
    0x52, 0x02, 0x00, 0xD9, 0x1D, 0x7C, 0x6D, 0x52, 0x02, 0xC3,
];
const B_RAIN_OVERRIDE_CALLS: [usize; 0] = [];
// credits_roll 0x6d82d0..0x6d82ea size 0x1a
const B_CREDITS_ROLL: [u8; 26] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0xD8, 0x05, 0xFC,
    0x66, 0x7E, 0x02, 0xD9, 0x99, 0x08, 0x0B, 0x00, 0x00, 0xC3,
];
const B_CREDITS_ROLL_CALLS: [usize; 0] = [];
// spawning_peds 0x6d7510..0x6d7547 size 0x37
const B_SPAWNING_PEDS: [u8; 55] = [
    0x8B, 0x44, 0x24, 0x04, 0x53, 0xB3, 0x01, 0x38, 0x58, 0x08, 0x75, 0x19, 0x32, 0xC0, 0xA2, 0x25,
    0xDA, 0x15, 0x02, 0xA2, 0x05, 0xDA, 0x15, 0x02, 0xE8, 0x03, 0x65, 0x4A, 0x00, 0x88, 0x1D, 0x20,
    0x96, 0x52, 0x02, 0x5B, 0xC3, 0x88, 0x1D, 0x25, 0xDA, 0x15, 0x02, 0x88, 0x1D, 0x05, 0xDA, 0x15,
    0x02, 0x5B, 0xE9, 0xE9, 0x64, 0x4A, 0x00,
];
const B_SPAWNING_PEDS_CALLS: [usize; 1] = [0x18];
// spawning_cars 0x6d7550..0x6d758f size 0x3f
const B_SPAWNING_CARS: [u8; 63] = [
    0x8B, 0x44, 0x24, 0x04, 0x53, 0xB3, 0x01, 0x38, 0x58, 0x08, 0x75, 0x20, 0xC6, 0x05, 0x1D, 0xDA,
    0x15, 0x02, 0x00, 0xE8, 0xF8, 0x50, 0x26, 0x00, 0xC6, 0x05, 0xFD, 0xD9, 0x15, 0x02, 0x00, 0xE8,
    0xBC, 0x9B, 0x4A, 0x00, 0x88, 0x1D, 0x20, 0x96, 0x52, 0x02, 0x5B, 0xC3, 0x88, 0x1D, 0xFD, 0xD9,
    0x15, 0x02, 0xE8, 0xA9, 0x9B, 0x4A, 0x00, 0x88, 0x1D, 0x20, 0x96, 0x52, 0x02, 0x5B, 0xC3,
];
const B_SPAWNING_CARS_CALLS: [usize; 3] = [0x13, 0x1f, 0x32];
// spawning_remove_all 0x6d76d0..0x6d7711 size 0x41
const B_SPAWNING_REMOVE_ALL: [u8; 65] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x78, 0x08, 0x01, 0x75, 0x36, 0x53, 0x32, 0xDB, 0x88, 0x1D, 0x25,
    0xDA, 0x15, 0x02, 0x88, 0x1D, 0x1D, 0xDA, 0x15, 0x02, 0xE8, 0x72, 0x4F, 0x26, 0x00, 0x88, 0x1D,
    0x05, 0xDA, 0x15, 0x02, 0xE8, 0x37, 0x63, 0x4A, 0x00, 0x88, 0x1D, 0xFD, 0xD9, 0x15, 0x02, 0xE8,
    0x2C, 0x9A, 0x4A, 0x00, 0xC6, 0x05, 0x20, 0x96, 0x52, 0x02, 0x01, 0x5B, 0x8D, 0x64, 0x24, 0x00,
    0xC3,
];
const B_SPAWNING_REMOVE_ALL_CALLS: [usize; 3] = [0x19, 0x24, 0x2f];
// hide_player_compnt 0x6d7a60..0x6d7a9e size 0x3e
const B_HIDE_PLAYER_COMPNT: [u8; 62] = [
    0x8B, 0x44, 0x24, 0x04, 0x57, 0x8B, 0x78, 0x08, 0xE8, 0xC3, 0xFA, 0x30, 0x00, 0x83, 0xF8, 0xFF,
    0x5F, 0x74, 0x17, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x91, 0xB0, 0x13, 0x00, 0x00, 0x50,
    0x52, 0xE8, 0x5A, 0x2A, 0x31, 0x00, 0x83, 0xC4, 0x08, 0xC3, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B,
    0x88, 0xB0, 0x13, 0x00, 0x00, 0x89, 0x4C, 0x24, 0x04, 0xE9, 0xC2, 0x2B, 0x31, 0x00,
];
const B_HIDE_PLAYER_COMPNT_CALLS: [usize; 2] = [0x8, 0x21];
// time_of_day_set 0x6d8140..0x6d8153 size 0x13
const B_TIME_OF_DAY_SET: [u8; 19] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xE8, 0xF4, 0x7F, 0x6C, 0x00, 0x8B, 0xC8, 0xE9, 0x6D,
    0x24, 0xFB, 0xFF,
];
const B_TIME_OF_DAY_SET_CALLS: [usize; 1] = [0x7];
// bink_play 0x6d7aa0..0x6d7aea size 0x4a
const B_BINK_PLAY: [u8; 74] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0x40, 0x08, 0xE8, 0x94, 0x86, 0x6C, 0x00, 0x83, 0xF8, 0x01, 0x7C,
    0x22, 0x8B, 0x0D, 0x70, 0xE6, 0x40, 0x01, 0x85, 0xC9, 0x56, 0x8B, 0xF1, 0x74, 0x13, 0x6A, 0x01,
    0x50, 0x51, 0xFF, 0x15, 0x00, 0x84, 0xDB, 0x00, 0x6A, 0x00, 0x56, 0xFF, 0x15, 0xD0, 0x83, 0xDB,
    0x00, 0x5E, 0xC3, 0x85, 0xC0, 0x7D, 0xFB, 0xA1, 0x70, 0xE6, 0x40, 0x01, 0x85, 0xC0, 0x74, 0xF2,
    0x6A, 0x01, 0x50, 0xFF, 0x15, 0xD0, 0x83, 0xDB, 0x00, 0xC3,
];
const B_BINK_PLAY_CALLS: [usize; 1] = [0x7];
// mesh_mover_hide 0x6d7af0..0x6d7b0e size 0x1e
const B_MESH_MOVER_HIDE: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x12, 0xE8, 0xE0, 0x3B, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x09, 0x56, 0x8B, 0xF0, 0xE8, 0xF4, 0x3C, 0x38, 0x00, 0x5E, 0xC3,
];
const B_MESH_MOVER_HIDE_CALLS: [usize; 2] = [0xb, 0x17];
// mesh_mover_show 0x6d7b10..0x6d7b2e size 0x1e
const B_MESH_MOVER_SHOW: [u8; 30] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x12, 0xE8, 0xC0, 0x3B, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x09, 0x56, 0x8B, 0xF0, 0xE8, 0x44, 0x3E, 0x38, 0x00, 0x5E, 0xC3,
];
const B_MESH_MOVER_SHOW_CALLS: [usize; 2] = [0xb, 0x17];
// chunk_hide 0x6d8280..0x6d82ae size 0x2e
const B_CHUNK_HIDE: [u8; 46] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x8B, 0x7E, 0x08, 0x83, 0xC6, 0x08, 0xE8, 0xDF, 0x14, 0xDD,
    0xFF, 0x85, 0xC0, 0x74, 0x16, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0x80, 0x48, 0x1D, 0x01, 0x56,
    0x81, 0xC1, 0xE4, 0x1E, 0x00, 0x00, 0xE8, 0x65, 0x07, 0x00, 0x00, 0x5F, 0x5E, 0xC3,
];
const B_CHUNK_HIDE_CALLS: [usize; 2] = [0xc, 0x26];
// chunk_show 0x6d82b0..0x6d82c7 size 0x17
const B_CHUNK_SHOW: [u8; 23] = [
    0x8B, 0x44, 0x24, 0x04, 0x57, 0x8B, 0x78, 0x08, 0xE8, 0xB3, 0x14, 0xDD, 0xFF, 0x85, 0xC0, 0x5F,
    0x74, 0x04, 0x80, 0x60, 0x1D, 0xFE, 0xC3,
];
const B_CHUNK_SHOW_CALLS: [usize; 1] = [0x8];
// weather_stage 0x6d7db0..0x6d7ddd size 0x2d
const B_WEATHER_STAGE: [u8; 45] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x21, 0x50, 0xE8, 0x6F, 0x70, 0xE0,
    0xFF, 0x83, 0xC4, 0x04, 0x83, 0xF8, 0xFF, 0x74, 0x13, 0x8B, 0x15, 0x10, 0x7D, 0x52, 0x02, 0xB1,
    0x01, 0x88, 0x8A, 0xD4, 0x43, 0x00, 0x00, 0xE9, 0x24, 0x6E, 0xE0, 0xFF, 0xC3,
];
const B_WEATHER_STAGE_CALLS: [usize; 1] = [0xc];
// shot_fade_in 0x6d7750..0x6d7796 size 0x46
const B_SHOT_FADE_IN: [u8; 70] = [
    0xD9, 0xEE, 0x8B, 0x4C, 0x24, 0x04, 0xD8, 0x51, 0x08, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x33,
    0x51, 0xD9, 0x1C, 0x24, 0xD9, 0x41, 0x08, 0xDC, 0x0D, 0xD8, 0xC0, 0xE5, 0x00, 0xE8, 0xCE, 0x89,
    0x6C, 0x00, 0x50, 0x32, 0xD2, 0xE8, 0xF6, 0x9A, 0xDF, 0xFF, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0x83,
    0xC4, 0x08, 0x80, 0xB8, 0x02, 0x0B, 0x00, 0x00, 0x00, 0x75, 0x0A, 0xC6, 0x80, 0x03, 0x0B, 0x00,
    0x00, 0x01, 0xC3, 0xDD, 0xD8, 0xC3,
];
const B_SHOT_FADE_IN_CALLS: [usize; 2] = [0x1d, 0x25];
// act_cam_stop_track 0x6cb680..0x6cb695 size 0x15
const B_ACT_CAM_STOP_TRACK: [u8; 21] = [
    0x56, 0x57, 0x33, 0xFF, 0xB8, 0x20, 0x5B, 0x5F, 0x02, 0x33, 0xC9, 0x33, 0xF6, 0xE8, 0x4E, 0x73,
    0xDD, 0xFF, 0x5F, 0x5E, 0xC3,
];
const B_ACT_CAM_STOP_TRACK_CALLS: [usize; 1] = [0xd];
// act_cam_track_char 0x6cb2e0..0x6cb327 size 0x47
const B_ACT_CAM_TRACK_CHAR: [u8; 71] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x0C, 0x85, 0xF6, 0x74, 0x39, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0xB4, 0x78, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0x68, 0x79, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x19, 0x8B, 0xF0, 0x85, 0xF6,
    0x74, 0x13, 0x57, 0x33, 0xFF, 0xB8, 0x20, 0x5B, 0x5F, 0x02, 0xB9, 0xC8, 0x80, 0xDD, 0x00, 0xE8,
    0xBC, 0x76, 0xDD, 0xFF, 0x5F, 0x5E, 0xC3,
];
const B_ACT_CAM_TRACK_CHAR_CALLS: [usize; 3] = [0x17, 0x23, 0x3f];
// act_cam_look_at_char 0x6cb330..0x6cb390 size 0x60
const B_ACT_CAM_LOOK_AT_CHAR: [u8; 96] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x8B, 0x45, 0x08, 0x83, 0xEC, 0x08, 0x56, 0x8B, 0x70, 0x0C,
    0x85, 0xF6, 0x57, 0x74, 0x45, 0x8D, 0x44, 0x24, 0x0F, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x0F, 0x00,
    0xE8, 0x5B, 0x78, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x0F, 0x00, 0x75, 0x09, 0xE8, 0x0F, 0x79, 0x38,
    0x00, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0xF0, 0x85, 0xF6, 0x74, 0x1F, 0x33, 0xFF, 0xB8, 0x20, 0x5B,
    0x5F, 0x02, 0xB9, 0xC8, 0x80, 0xDD, 0x00, 0xE8, 0x64, 0x76, 0xDD, 0xFF, 0xB8, 0x20, 0x5B, 0x5F,
    0x02, 0x33, 0xC9, 0x33, 0xF6, 0xE8, 0x56, 0x76, 0xDD, 0xFF, 0x5F, 0x5E, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B_ACT_CAM_LOOK_AT_CHAR_CALLS: [usize; 4] = [0x20, 0x2c, 0x47, 0x55];
// act_hide_character 0x6cb6e0..0x6cb722 size 0x42
const B_ACT_HIDE_CHARACTER: [u8; 66] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x34, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0xB4, 0x74, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0x68, 0x75, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x14, 0x8B, 0xC8, 0x85, 0xC9,
    0x74, 0x0E, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x01, 0xE8, 0x43, 0xB1, 0x25, 0x00, 0x83, 0xC4, 0x0C,
    0x5E, 0xC3,
];
const B_ACT_HIDE_CHARACTER_CALLS: [usize; 3] = [0x17, 0x23, 0x38];
// act_show_character 0x6cb730..0x6cb772 size 0x42
const B_ACT_SHOW_CHARACTER: [u8; 66] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x34, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0x64, 0x74, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0x18, 0x75, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x14, 0x8B, 0xC8, 0x85, 0xC9,
    0x74, 0x0E, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0xE8, 0xF3, 0xB0, 0x25, 0x00, 0x83, 0xC4, 0x0C,
    0x5E, 0xC3,
];
const B_ACT_SHOW_CHARACTER_CALLS: [usize; 3] = [0x17, 0x23, 0x38];
// act_disable_physics 0x6cb780..0x6cb7db size 0x5b
const B_ACT_DISABLE_PHYSICS: [u8; 91] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x4D, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0x14, 0x74, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0xC8, 0x74, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x2D, 0x8B, 0xF0, 0x85, 0xF6,
    0x74, 0x27, 0x6A, 0x00, 0x6A, 0x14, 0xE8, 0x55, 0x0E, 0x03, 0x00, 0x8B, 0x86, 0x70, 0x05, 0x00,
    0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x07, 0x80, 0x88, 0x30, 0x01, 0x00, 0x00, 0x02, 0x81,
    0x8E, 0xCC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x5E, 0xC3,
];
const B_ACT_DISABLE_PHYSICS_CALLS: [usize; 3] = [0x17, 0x23, 0x36];
// act_enable_physics 0x6cb7e0..0x6cb84f size 0x6f
const B_ACT_ENABLE_PHYSICS: [u8; 111] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x61, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0xB4, 0x73, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0x68, 0x74, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x41, 0x8B, 0xF0, 0x85, 0xF6,
    0x74, 0x3B, 0x6A, 0x00, 0x6A, 0x0F, 0xE8, 0xF5, 0x0D, 0x03, 0x00, 0x8B, 0x86, 0x70, 0x05, 0x00,
    0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x07, 0x80, 0xA0, 0x30, 0x01, 0x00, 0x00, 0xFD, 0x6A,
    0x00, 0x6A, 0xFF, 0x8D, 0x4E, 0x14, 0x6A, 0x00, 0x51, 0x8B, 0xC6, 0xE8, 0x50, 0x38, 0x29, 0x00,
    0x83, 0xC4, 0x10, 0x81, 0xA6, 0xCC, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xBF, 0xFF, 0x5E, 0xC3,
];
const B_ACT_ENABLE_PHYSICS_CALLS: [usize; 4] = [0x17, 0x23, 0x36, 0x5b];
// act_hide_vehicle 0x6cb850..0x6cb88b size 0x3b
const B_ACT_HIDE_VEHICLE: [u8; 59] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x56, 0xE8, 0xD3, 0x47, 0x3A, 0x00, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x26, 0x8B, 0x16, 0x8B, 0x42, 0x2C, 0x8B, 0xCE, 0xFF, 0xD0, 0x84, 0xC0, 0x74, 0x19,
    0x8B, 0x16, 0x8B, 0x42, 0x34, 0x8B, 0xCE, 0xFF, 0xD0, 0x8B, 0x48, 0x44, 0x6A, 0x00, 0x6A, 0x01,
    0x51, 0xE8, 0xDA, 0x13, 0x3E, 0x00, 0x83, 0xC4, 0x0C, 0x5E, 0xC3,
];
const B_ACT_HIDE_VEHICLE_CALLS: [usize; 2] = [0x8, 0x31];
// act_show_vehicle 0x6cb890..0x6cb8d7 size 0x47
const B_ACT_SHOW_VEHICLE: [u8; 71] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x56, 0xE8, 0x93, 0x47, 0x3A, 0x00, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x32, 0x8B, 0x16, 0x8B, 0x42, 0x2C, 0x8B, 0xCE, 0xFF, 0xD0, 0x84, 0xC0, 0x74, 0x25,
    0x8B, 0x16, 0x8B, 0x42, 0x34, 0x8B, 0xCE, 0xFF, 0xD0, 0x8B, 0xF0, 0x8B, 0x4E, 0x44, 0x6A, 0x00,
    0x6A, 0x00, 0x51, 0xE8, 0x98, 0x13, 0x3E, 0x00, 0x8B, 0x16, 0x8B, 0x42, 0x20, 0x83, 0xC4, 0x0C,
    0x8B, 0xCE, 0x5E, 0xFF, 0xE0, 0x5E, 0xC3,
];
const B_ACT_SHOW_VEHICLE_CALLS: [usize; 2] = [0x8, 0x33];
// act_hide_group 0x6cb8e0..0x6cb900 size 0x20
const B_ACT_HIDE_GROUP: [u8; 32] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x14, 0xE8, 0x40, 0xBE, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x0B, 0x57, 0x6A, 0x01, 0x8B, 0xF8, 0xE8, 0x72, 0xC7, 0x38, 0x00, 0x5F, 0xC3,
];
const B_ACT_HIDE_GROUP_CALLS: [usize; 2] = [0xb, 0x19];
// act_show_group 0x6cb900..0x6cb920 size 0x20
const B_ACT_SHOW_GROUP: [u8; 32] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x14, 0xE8, 0x20, 0xBE, 0x38, 0x00,
    0x85, 0xC0, 0x74, 0x0B, 0x57, 0x6A, 0x00, 0x8B, 0xF8, 0xE8, 0x52, 0xC7, 0x38, 0x00, 0x5F, 0xC3,
];
const B_ACT_SHOW_GROUP_CALLS: [usize; 2] = [0xb, 0x19];
// act_look_at_nothing 0x6cb250..0x6cb299 size 0x49
const B_ACT_LOOK_AT_NOTHING: [u8; 73] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x3B, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0x44, 0x79, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0xF8, 0x79, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x1B, 0x85, 0xC0, 0x74, 0x17,
    0x83, 0x88, 0x58, 0x07, 0x00, 0x00, 0x01, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x07,
    0x80, 0xA0, 0x40, 0x01, 0x00, 0x00, 0xFB, 0x5E, 0xC3,
];
const B_ACT_LOOK_AT_NOTHING_CALLS: [usize; 2] = [0x17, 0x23];
// act_cam_follow_target 0x6cb6a0..0x6cb6da size 0x3a
const B_ACT_CAM_FOLLOW_TARGET: [u8; 58] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x10, 0x51, 0xE8, 0x48, 0xCE, 0x5C, 0x00, 0xD9, 0x5C, 0x24,
    0x08, 0xD9, 0xEE, 0x83, 0xC4, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xC6, 0x05, 0x90, 0x5D, 0x5F, 0x02,
    0x01, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0xD9, 0x1D, 0x94, 0x5D, 0x5F, 0x02,
    0x74, 0x07, 0xC6, 0x05, 0x90, 0x5D, 0x5F, 0x02, 0x00, 0xC3,
];
const B_ACT_CAM_FOLLOW_TARGET_CALLS: [usize; 1] = [0x8];
// act_cam_cut_to 0x6cb2a0..0x6cb2db size 0x3b
const B_ACT_CAM_CUT_TO: [u8; 59] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x8B, 0x43, 0x0C, 0x50, 0x33, 0xD2, 0x8D, 0x44, 0x24, 0x0C, 0xE8,
    0x7C, 0x17, 0x51, 0x00, 0xE8, 0x97, 0x38, 0x39, 0x00, 0x85, 0xC0, 0x74, 0x1C, 0x56, 0x57, 0x8D,
    0x78, 0x28, 0x83, 0xC0, 0x1C, 0x33, 0xC9, 0x33, 0xF6, 0xE8, 0x12, 0x77, 0xDD, 0xFF, 0x53, 0xE8,
    0x0C, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5B, 0xC3,
];
const B_ACT_CAM_CUT_TO_CALLS: [usize; 4] = [0xf, 0x14, 0x29, 0x2f];
// act_look_at 0x6cb110..0x6cb1c3 size 0xb3
const B_ACT_LOOK_AT: [u8; 179] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x56, 0x8B, 0x75, 0x08, 0x85, 0xF6, 0x0F, 0x84, 0x9F, 0x00, 0x00,
    0x00, 0x57, 0x8D, 0x44, 0x24, 0x10, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x10, 0x00, 0xE8, 0x7E, 0x7A,
    0x38, 0x00, 0x80, 0x7C, 0x24, 0x10, 0x00, 0x75, 0x09, 0xE8, 0x32, 0x7B, 0x38, 0x00, 0x85, 0xC0,
    0x74, 0x7D, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x77, 0x8B, 0x75, 0x0C, 0x85, 0xF6, 0x74, 0x70, 0x8D,
    0x44, 0x24, 0x10, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x10, 0x00, 0xE8, 0x51, 0x7A, 0x38, 0x00, 0x80,
    0x7C, 0x24, 0x10, 0x00, 0x75, 0x09, 0xE8, 0x05, 0x7B, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x50, 0x85,
    0xC0, 0x74, 0x4C, 0xBA, 0xFE, 0xFF, 0xFF, 0xFF, 0x21, 0x97, 0x58, 0x07, 0x00, 0x00, 0x8B, 0x48,
    0x44, 0x8D, 0x87, 0x30, 0x07, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x24, 0xD9, 0xEE, 0x21, 0x50, 0x30,
    0xD9, 0x10, 0x89, 0x48, 0x0C, 0xD9, 0x50, 0x04, 0xD9, 0x58, 0x08, 0xD9, 0x05, 0x54, 0x0C, 0xDD,
    0x00, 0x80, 0x48, 0x2C, 0x01, 0xD9, 0x58, 0x1C, 0xC7, 0x40, 0x18, 0xFF, 0xFF, 0xFF, 0xFF, 0x80,
    0xBF, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x07, 0x80, 0x8F, 0x40, 0x01, 0x00, 0x00, 0x04, 0x5F,
    0x5E, 0x5D, 0xC3,
];
const B_ACT_LOOK_AT_CALLS: [usize; 4] = [0x1d, 0x29, 0x4a, 0x56];
// act_look_at_nav 0x6cb1d0..0x6cb243 size 0x73
const B_ACT_LOOK_AT_NAV: [u8; 115] = [
    0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x77, 0x08, 0x85, 0xF6, 0x74, 0x63, 0x8D, 0x44, 0x24,
    0x0C, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x0C, 0x00, 0xE8, 0xC3, 0x79, 0x38, 0x00, 0x80, 0x7C, 0x24,
    0x0C, 0x00, 0x75, 0x09, 0xE8, 0x77, 0x7A, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x43, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x3D, 0x8B, 0x47, 0x0C, 0x85, 0xC0, 0x74, 0x36, 0x50, 0x33, 0xD2, 0x8D, 0x44, 0x24,
    0x10, 0xE8, 0x1A, 0x18, 0x51, 0x00, 0xE8, 0x35, 0x39, 0x39, 0x00, 0x85, 0xC0, 0x74, 0x21, 0xD9,
    0x05, 0x98, 0x79, 0xE8, 0x00, 0x83, 0xA6, 0x58, 0x07, 0x00, 0x00, 0xFE, 0x51, 0xD9, 0x1C, 0x24,
    0x8D, 0x50, 0x1C, 0x56, 0x32, 0xC0, 0x33, 0xF6, 0xE8, 0xC3, 0xA9, 0x2E, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3,
];
const B_ACT_LOOK_AT_NAV_CALLS: [usize; 5] = [0x18, 0x24, 0x41, 0x46, 0x68];
// act_teleports_to 0x6caa80..0x6caaff size 0x7f
const B_ACT_TELEPORTS_TO: [u8; 127] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x8B, 0x73, 0x08, 0x85, 0xF6, 0x74, 0x6F, 0x57, 0x8D, 0x44,
    0x24, 0x10, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x10, 0x00, 0xE8, 0x12, 0x81, 0x38, 0x00, 0x80, 0x7C,
    0x24, 0x10, 0x00, 0x75, 0x09, 0xE8, 0xC6, 0x81, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x4D, 0x8B, 0xF8,
    0x85, 0xFF, 0x74, 0x47, 0x8B, 0x43, 0x0C, 0x50, 0x33, 0xD2, 0x8D, 0x44, 0x24, 0x14, 0xE8, 0x6D,
    0x1F, 0x51, 0x00, 0xE8, 0x88, 0x40, 0x39, 0x00, 0x8B, 0xF0, 0x85, 0xF6, 0x74, 0x2D, 0x80, 0xBF,
    0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x0D, 0x6A, 0x00, 0x6A, 0x00, 0x57, 0xE8, 0xEF, 0xD1, 0xD7,
    0xFF, 0x83, 0xC4, 0x0C, 0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0x8D, 0x4E, 0x28, 0x8D,
    0x56, 0x1C, 0x57, 0xE8, 0x08, 0x60, 0x2D, 0x00, 0x83, 0xC4, 0x14, 0x5F, 0x5E, 0x5B, 0xC3,
];
const B_ACT_TELEPORTS_TO_CALLS: [usize; 6] = [0x19, 0x25, 0x3e, 0x43, 0x5c, 0x73];
// act_walk_to 0x6cab00..0x6cab63 size 0x63
const B_ACT_WALK_TO: [u8; 99] = [
    0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x77, 0x08, 0x85, 0xF6, 0x74, 0x53, 0x8D, 0x44, 0x24,
    0x0C, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x0C, 0x00, 0xE8, 0x93, 0x80, 0x38, 0x00, 0x80, 0x7C, 0x24,
    0x0C, 0x00, 0x75, 0x09, 0xE8, 0x47, 0x81, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x33, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x2D, 0x8B, 0x47, 0x0C, 0x50, 0x33, 0xD2, 0x8D, 0x44, 0x24, 0x10, 0xE8, 0xEE, 0x1E,
    0x51, 0x00, 0xE8, 0x09, 0x40, 0x39, 0x00, 0x85, 0xC0, 0x74, 0x15, 0x6A, 0x01, 0x50, 0xC7, 0x05,
    0x28, 0x7D, 0x52, 0x02, 0x03, 0x00, 0x00, 0x00, 0xE8, 0x03, 0xFA, 0xFF, 0xFF, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3,
];
const B_ACT_WALK_TO_CALLS: [usize; 5] = [0x18, 0x24, 0x3d, 0x42, 0x58];
// act_run_to 0x6cab70..0x6cabd3 size 0x63
const B_ACT_RUN_TO: [u8; 99] = [
    0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x77, 0x08, 0x85, 0xF6, 0x74, 0x53, 0x8D, 0x44, 0x24,
    0x0C, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x0C, 0x00, 0xE8, 0x23, 0x80, 0x38, 0x00, 0x80, 0x7C, 0x24,
    0x0C, 0x00, 0x75, 0x09, 0xE8, 0xD7, 0x80, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x33, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x2D, 0x8B, 0x47, 0x0C, 0x50, 0x33, 0xD2, 0x8D, 0x44, 0x24, 0x10, 0xE8, 0x7E, 0x1E,
    0x51, 0x00, 0xE8, 0x99, 0x3F, 0x39, 0x00, 0x85, 0xC0, 0x74, 0x15, 0x6A, 0x02, 0x50, 0xC7, 0x05,
    0x28, 0x7D, 0x52, 0x02, 0x07, 0x00, 0x00, 0x00, 0xE8, 0x93, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x08,
    0x5F, 0x5E, 0xC3,
];
const B_ACT_RUN_TO_CALLS: [usize; 5] = [0x18, 0x24, 0x3d, 0x42, 0x58];
// act_suicides 0x6cacd0..0x6cad1f size 0x4f
const B_ACT_SUICIDES: [u8; 79] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x41, 0x8D, 0x44, 0x24, 0x08,
    0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0xC4, 0x7E, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08,
    0x00, 0x75, 0x09, 0xE8, 0x78, 0x7F, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x21, 0x85, 0xC0, 0x74, 0x1D,
    0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x50, 0x33,
    0xD2, 0x33, 0xC9, 0xE8, 0x88, 0x9A, 0x2B, 0x00, 0xDD, 0xD8, 0x83, 0xC4, 0x10, 0x5E, 0xC3,
];
const B_ACT_SUICIDES_CALLS: [usize; 3] = [0x17, 0x23, 0x43];
// act_cam_stop_slide 0x6cb5e0..0x6cb674 size 0x94
const B_ACT_CAM_STOP_SLIDE: [u8; 148] = [
    0x8B, 0x15, 0x10, 0x7D, 0x52, 0x02, 0x85, 0xD2, 0x0F, 0x84, 0x85, 0x00, 0x00, 0x00, 0x8B, 0x82,
    0x40, 0x15, 0x00, 0x00, 0xD9, 0xEE, 0x83, 0xC0, 0x1C, 0xC6, 0x00, 0x00, 0x8B, 0x0D, 0x20, 0x5B,
    0x5F, 0x02, 0x89, 0x48, 0x0C, 0x8B, 0x0D, 0x24, 0x5B, 0x5F, 0x02, 0x89, 0x48, 0x10, 0x8B, 0x0D,
    0x28, 0x5B, 0x5F, 0x02, 0x89, 0x48, 0x14, 0x8B, 0x0D, 0x20, 0x5B, 0x5F, 0x02, 0x89, 0x48, 0x1C,
    0x8B, 0x0D, 0x24, 0x5B, 0x5F, 0x02, 0x89, 0x48, 0x20, 0x8B, 0x0D, 0x28, 0x5B, 0x5F, 0x02, 0xD9,
    0x50, 0x18, 0x56, 0xD9, 0x50, 0x08, 0x89, 0x48, 0x24, 0x57, 0xC6, 0x82, 0x80, 0x15, 0x00, 0x00,
    0x00, 0x8D, 0xBA, 0x8C, 0x15, 0x00, 0x00, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xBE, 0x5C, 0x5B, 0x5F,
    0x02, 0xF3, 0xA5, 0x8D, 0xBA, 0xB4, 0x15, 0x00, 0x00, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xBE, 0x5C,
    0x5B, 0x5F, 0x02, 0xF3, 0xA5, 0xD9, 0x92, 0xB0, 0x15, 0x00, 0x00, 0xD9, 0x9A, 0x88, 0x15, 0x00,
    0x00, 0x5F, 0x5E, 0xC3,
];
const B_ACT_CAM_STOP_SLIDE_CALLS: [usize; 0] = [];
// act_cam_slide_update 0x6cb590..0x6cb5d7 size 0x47
const B_ACT_CAM_SLIDE_UPDATE: [u8; 71] = [
    0xA1, 0x10, 0x7D, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x8B, 0x80, 0x40, 0x15,
    0x00, 0x00, 0x83, 0xC0, 0x1C, 0x80, 0x38, 0x00, 0x74, 0x1E, 0xD9, 0x05, 0x10, 0x67, 0x7E, 0x02,
    0xD9, 0x40, 0x18, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7B, 0x0C, 0x33, 0xC0, 0x33, 0xC9,
    0x3C, 0x01, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x33, 0xC9, 0x3A,
    0xC0, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3,
];
const B_ACT_CAM_SLIDE_UPDATE_CALLS: [usize; 0] = [];
pub const NPORTS: usize = 61;
struct Entry { name: &'static str, addr: usize, body: &'static [u8], calls: &'static [usize], thunk: extern "C" fn(), kind: u8 }
static ENTRIES: [Entry; NPORTS] = [
    Entry { name: "level_lights", addr: 0x6d7590, body: &B_LEVEL_LIGHTS, calls: &B_LEVEL_LIGHTS_CALLS, thunk: level_lights_t, kind: b'S' },
    Entry { name: "tree_shadows", addr: 0x6d75b0, body: &B_TREE_SHADOWS, calls: &B_TREE_SHADOWS_CALLS, thunk: tree_shadows_t, kind: b'S' },
    Entry { name: "tod_char_shadows", addr: 0x6d75d0, body: &B_TOD_CHAR_SHADOWS, calls: &B_TOD_CHAR_SHADOWS_CALLS, thunk: tod_char_shadows_t, kind: b'S' },
    Entry { name: "motion_blur", addr: 0x6d75e0, body: &B_MOTION_BLUR, calls: &B_MOTION_BLUR_CALLS, thunk: motion_blur_t, kind: b'S' },
    Entry { name: "motion_blur_override", addr: 0x6d7600, body: &B_MOTION_BLUR_OVERRIDE, calls: &B_MOTION_BLUR_OVERRIDE_CALLS, thunk: motion_blur_override_t, kind: b'S' },
    Entry { name: "camera_moves", addr: 0x6d7630, body: &B_CAMERA_MOVES, calls: &B_CAMERA_MOVES_CALLS, thunk: camera_moves_t, kind: b'S' },
    Entry { name: "disable_trees", addr: 0x6d7640, body: &B_DISABLE_TREES, calls: &B_DISABLE_TREES_CALLS, thunk: disable_trees_t, kind: b'S' },
    Entry { name: "city_no_render_al", addr: 0x6d7660, body: &B_CITY_NO_RENDER_AL, calls: &B_CITY_NO_RENDER_AL_CALLS, thunk: city_no_render_al_t, kind: b'S' },
    Entry { name: "shader_lod_force", addr: 0x6d7670, body: &B_SHADER_LOD_FORCE, calls: &B_SHADER_LOD_FORCE_CALLS, thunk: shader_lod_force_t, kind: b'S' },
    Entry { name: "city_shadow_dist", addr: 0x6d76b0, body: &B_CITY_SHADOW_DIST, calls: &B_CITY_SHADOW_DIST_CALLS, thunk: city_shadow_dist_t, kind: b'S' },
    Entry { name: "far_clip", addr: 0x6d7720, body: &B_FAR_CLIP, calls: &B_FAR_CLIP_CALLS, thunk: far_clip_t, kind: b'S' },
    Entry { name: "near_clip", addr: 0x6d7740, body: &B_NEAR_CLIP, calls: &B_NEAR_CLIP_CALLS, thunk: near_clip_t, kind: b'S' },
    Entry { name: "shot_fade_out", addr: 0x6d77a0, body: &B_SHOT_FADE_OUT, calls: &B_SHOT_FADE_OUT_CALLS, thunk: shot_fade_out_t, kind: b'S' },
    Entry { name: "shot_fade_out_midway", addr: 0x6d77e0, body: &B_SHOT_FADE_OUT_MIDWAY, calls: &B_SHOT_FADE_OUT_MIDWAY_CALLS, thunk: shot_fade_out_midway_t, kind: b'S' },
    Entry { name: "shot_cut_out_midway", addr: 0x6d7840, body: &B_SHOT_CUT_OUT_MIDWAY, calls: &B_SHOT_CUT_OUT_MIDWAY_CALLS, thunk: shot_cut_out_midway_t, kind: b'S' },
    Entry { name: "letterbox_fade_in", addr: 0x6d7890, body: &B_LETTERBOX_FADE_IN, calls: &B_LETTERBOX_FADE_IN_CALLS, thunk: letterbox_fade_in_t, kind: b'S' },
    Entry { name: "letterbox_fade_out", addr: 0x6d78d0, body: &B_LETTERBOX_FADE_OUT, calls: &B_LETTERBOX_FADE_OUT_CALLS, thunk: letterbox_fade_out_t, kind: b'S' },
    Entry { name: "framebuffer_tint", addr: 0x6d78f0, body: &B_FRAMEBUFFER_TINT, calls: &B_FRAMEBUFFER_TINT_CALLS, thunk: framebuffer_tint_t, kind: b'S' },
    Entry { name: "force_tod_lights", addr: 0x6d79e0, body: &B_FORCE_TOD_LIGHTS, calls: &B_FORCE_TOD_LIGHTS_CALLS, thunk: force_tod_lights_t, kind: b'S' },
    Entry { name: "lightning", addr: 0x6d7d70, body: &B_LIGHTNING, calls: &B_LIGHTNING_CALLS, thunk: lightning_t, kind: b'S' },
    Entry { name: "glares", addr: 0x6d8120, body: &B_GLARES, calls: &B_GLARES_CALLS, thunk: glares_t, kind: b'S' },
    Entry { name: "fog_distance", addr: 0x6d8170, body: &B_FOG_DISTANCE, calls: &B_FOG_DISTANCE_CALLS, thunk: fog_distance_t, kind: b'S' },
    Entry { name: "city_region_blend", addr: 0x6d81e0, body: &B_CITY_REGION_BLEND, calls: &B_CITY_REGION_BLEND_CALLS, thunk: city_region_blend_t, kind: b'S' },
    Entry { name: "city_render_fade", addr: 0x6d81f0, body: &B_CITY_RENDER_FADE, calls: &B_CITY_RENDER_FADE_CALLS, thunk: city_render_fade_t, kind: b'S' },
    Entry { name: "drug_effect", addr: 0x6d8200, body: &B_DRUG_EFFECT, calls: &B_DRUG_EFFECT_CALLS, thunk: drug_effect_t, kind: b'S' },
    Entry { name: "rain_override", addr: 0x6d8240, body: &B_RAIN_OVERRIDE, calls: &B_RAIN_OVERRIDE_CALLS, thunk: rain_override_t, kind: b'S' },
    Entry { name: "credits_roll", addr: 0x6d82d0, body: &B_CREDITS_ROLL, calls: &B_CREDITS_ROLL_CALLS, thunk: credits_roll_t, kind: b'S' },
    Entry { name: "spawning_peds", addr: 0x6d7510, body: &B_SPAWNING_PEDS, calls: &B_SPAWNING_PEDS_CALLS, thunk: spawning_peds_t, kind: b'C' },
    Entry { name: "spawning_cars", addr: 0x6d7550, body: &B_SPAWNING_CARS, calls: &B_SPAWNING_CARS_CALLS, thunk: spawning_cars_t, kind: b'C' },
    Entry { name: "spawning_remove_all", addr: 0x6d76d0, body: &B_SPAWNING_REMOVE_ALL, calls: &B_SPAWNING_REMOVE_ALL_CALLS, thunk: spawning_remove_all_t, kind: b'C' },
    Entry { name: "hide_player_compnt", addr: 0x6d7a60, body: &B_HIDE_PLAYER_COMPNT, calls: &B_HIDE_PLAYER_COMPNT_CALLS, thunk: hide_player_compnt_t, kind: b'C' },
    Entry { name: "time_of_day_set", addr: 0x6d8140, body: &B_TIME_OF_DAY_SET, calls: &B_TIME_OF_DAY_SET_CALLS, thunk: time_of_day_set_t, kind: b'C' },
    Entry { name: "bink_play", addr: 0x6d7aa0, body: &B_BINK_PLAY, calls: &B_BINK_PLAY_CALLS, thunk: bink_play_t, kind: b'C' },
    Entry { name: "mesh_mover_hide", addr: 0x6d7af0, body: &B_MESH_MOVER_HIDE, calls: &B_MESH_MOVER_HIDE_CALLS, thunk: mesh_mover_hide_t, kind: b'C' },
    Entry { name: "mesh_mover_show", addr: 0x6d7b10, body: &B_MESH_MOVER_SHOW, calls: &B_MESH_MOVER_SHOW_CALLS, thunk: mesh_mover_show_t, kind: b'C' },
    Entry { name: "chunk_hide", addr: 0x6d8280, body: &B_CHUNK_HIDE, calls: &B_CHUNK_HIDE_CALLS, thunk: chunk_hide_t, kind: b'C' },
    Entry { name: "chunk_show", addr: 0x6d82b0, body: &B_CHUNK_SHOW, calls: &B_CHUNK_SHOW_CALLS, thunk: chunk_show_t, kind: b'C' },
    Entry { name: "weather_stage", addr: 0x6d7db0, body: &B_WEATHER_STAGE, calls: &B_WEATHER_STAGE_CALLS, thunk: weather_stage_t, kind: b'C' },
    Entry { name: "shot_fade_in", addr: 0x6d7750, body: &B_SHOT_FADE_IN, calls: &B_SHOT_FADE_IN_CALLS, thunk: shot_fade_in_t, kind: b'C' },
    Entry { name: "act_cam_stop_track", addr: 0x6cb680, body: &B_ACT_CAM_STOP_TRACK, calls: &B_ACT_CAM_STOP_TRACK_CALLS, thunk: act_cam_stop_track_t, kind: b'C' },
    Entry { name: "act_cam_track_char", addr: 0x6cb2e0, body: &B_ACT_CAM_TRACK_CHAR, calls: &B_ACT_CAM_TRACK_CHAR_CALLS, thunk: act_cam_track_char_t, kind: b'C' },
    Entry { name: "act_cam_look_at_char", addr: 0x6cb330, body: &B_ACT_CAM_LOOK_AT_CHAR, calls: &B_ACT_CAM_LOOK_AT_CHAR_CALLS, thunk: act_cam_look_at_char_t, kind: b'C' },
    Entry { name: "act_hide_character", addr: 0x6cb6e0, body: &B_ACT_HIDE_CHARACTER, calls: &B_ACT_HIDE_CHARACTER_CALLS, thunk: act_hide_character_t, kind: b'C' },
    Entry { name: "act_show_character", addr: 0x6cb730, body: &B_ACT_SHOW_CHARACTER, calls: &B_ACT_SHOW_CHARACTER_CALLS, thunk: act_show_character_t, kind: b'C' },
    Entry { name: "act_disable_physics", addr: 0x6cb780, body: &B_ACT_DISABLE_PHYSICS, calls: &B_ACT_DISABLE_PHYSICS_CALLS, thunk: act_disable_physics_t, kind: b'C' },
    Entry { name: "act_enable_physics", addr: 0x6cb7e0, body: &B_ACT_ENABLE_PHYSICS, calls: &B_ACT_ENABLE_PHYSICS_CALLS, thunk: act_enable_physics_t, kind: b'C' },
    Entry { name: "act_hide_vehicle", addr: 0x6cb850, body: &B_ACT_HIDE_VEHICLE, calls: &B_ACT_HIDE_VEHICLE_CALLS, thunk: act_hide_vehicle_t, kind: b'C' },
    Entry { name: "act_show_vehicle", addr: 0x6cb890, body: &B_ACT_SHOW_VEHICLE, calls: &B_ACT_SHOW_VEHICLE_CALLS, thunk: act_show_vehicle_t, kind: b'C' },
    Entry { name: "act_hide_group", addr: 0x6cb8e0, body: &B_ACT_HIDE_GROUP, calls: &B_ACT_HIDE_GROUP_CALLS, thunk: act_hide_group_t, kind: b'C' },
    Entry { name: "act_show_group", addr: 0x6cb900, body: &B_ACT_SHOW_GROUP, calls: &B_ACT_SHOW_GROUP_CALLS, thunk: act_show_group_t, kind: b'C' },
    Entry { name: "act_look_at_nothing", addr: 0x6cb250, body: &B_ACT_LOOK_AT_NOTHING, calls: &B_ACT_LOOK_AT_NOTHING_CALLS, thunk: act_look_at_nothing_t, kind: b'C' },
    Entry { name: "act_cam_follow_target", addr: 0x6cb6a0, body: &B_ACT_CAM_FOLLOW_TARGET, calls: &B_ACT_CAM_FOLLOW_TARGET_CALLS, thunk: act_cam_follow_target_t, kind: b'C' },
    Entry { name: "act_cam_cut_to", addr: 0x6cb2a0, body: &B_ACT_CAM_CUT_TO, calls: &B_ACT_CAM_CUT_TO_CALLS, thunk: act_cam_cut_to_t, kind: b'C' },
    Entry { name: "act_look_at", addr: 0x6cb110, body: &B_ACT_LOOK_AT, calls: &B_ACT_LOOK_AT_CALLS, thunk: act_look_at_t, kind: b'C' },
    Entry { name: "act_look_at_nav", addr: 0x6cb1d0, body: &B_ACT_LOOK_AT_NAV, calls: &B_ACT_LOOK_AT_NAV_CALLS, thunk: act_look_at_nav_t, kind: b'C' },
    Entry { name: "act_teleports_to", addr: 0x6caa80, body: &B_ACT_TELEPORTS_TO, calls: &B_ACT_TELEPORTS_TO_CALLS, thunk: act_teleports_to_t, kind: b'C' },
    Entry { name: "act_walk_to", addr: 0x6cab00, body: &B_ACT_WALK_TO, calls: &B_ACT_WALK_TO_CALLS, thunk: act_walk_to_t, kind: b'C' },
    Entry { name: "act_run_to", addr: 0x6cab70, body: &B_ACT_RUN_TO, calls: &B_ACT_RUN_TO_CALLS, thunk: act_run_to_t, kind: b'C' },
    Entry { name: "act_suicides", addr: 0x6cacd0, body: &B_ACT_SUICIDES, calls: &B_ACT_SUICIDES_CALLS, thunk: act_suicides_t, kind: b'C' },
    Entry { name: "act_cam_stop_slide", addr: 0x6cb5e0, body: &B_ACT_CAM_STOP_SLIDE, calls: &B_ACT_CAM_STOP_SLIDE_CALLS, thunk: act_cam_stop_slide_t, kind: b'S' },
    Entry { name: "act_cam_slide_update", addr: 0x6cb590, body: &B_ACT_CAM_SLIDE_UPDATE, calls: &B_ACT_CAM_SLIDE_UPDATE_CALLS, thunk: act_cam_slide_update_t, kind: b'R' },
];
// total bytes 3554

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for (i, e) in ENTRIES.iter().enumerate() {
        unsafe {
            if e.kind == b'S' || e.kind == b'R' {
                match detour::relocate(e.addr, e.body, e.calls) {
                    Ok(c) => ST[i].orig.store(c, Ordering::Relaxed),
                    Err(err) => {
                        log(&format!("port_cutscene: relocating {} @ {:#x} failed: {err:?}", e.name, e.addr));
                        continue;
                    }
                }
            }
            if replace_pinned(e.name, e.addr, e.body, e.thunk as usize) {
                ok += 1;
            }
        }
    }
    log(&format!("port_cutscene: replaced {ok}/{NPORTS} with Rust"));
    let (bok, btotal) = (0u32, 0u32); // BISECT: big::install() held back while chasing the cutscene crash
    log(&format!("port_cutscene: replaced {bok}/{btotal} large ports with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut parts = Vec::new();
    for (i, e) in ENTRIES.iter().enumerate() {
        let n = ST[i].calls.load(Ordering::Relaxed);
        if n == 0 {
            continue;
        }
        let m = ST[i].mism.load(Ordering::Relaxed);
        if e.kind == b'C' {
            let b: Vec<u32> = (0..4).map(|k| BR[i * 4 + k].load(Ordering::Relaxed)).collect();
            parts.push(format!("{} {} [{},{},{},{}]", e.name, n, b[0], b[1], b[2], b[3]));
        } else {
            parts.push(format!("{} {} ({})", e.name, n, m));
        }
    }
    let big = big::summary();
    if !big.is_empty() {
        parts.push(big);
    }
    if parts.is_empty() { String::new() } else { format!("port_cutscene: {}", parts.join(", ")) }
}

// =================================================================================================
// Large cutscene functions, ported by hand and checked offline by a differential test against the
// original bytes (see `big::dt`): every callee is a recording stub; both versions run on the same
// random world and must make the same calls in the same order with the same registers, stack
// arguments and pointed-to data, write the same memory and preserve the same registers.
//
// Run: RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_cutscene -- --ignored --test-threads=1 --nocapture
// =================================================================================================
pub(crate) mod big {
    #![allow(clippy::missing_safety_doc)]
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::portlib::{keep_ecx_edx_arg1, replace_pinned};

    // --- memory access: in the offline test the game's globals sit DELTA higher -------------------
    /// The offline test keeps the game's globals (0xD00000..0x3000000) at a fixed offset.
    #[cfg(test)]
    pub(crate) const DELTA: usize = 0x5000_0000;
    #[cfg(test)]
    pub(crate) fn t(p: usize) -> usize {
        if (0xD0_0000..0x300_0000).contains(&p) { p + DELTA } else { p }
    }
    #[cfg(not(test))]
    #[inline(always)]
    pub(crate) fn t(p: usize) -> usize {
        p
    }
    /// The address of a game global as a value handed to a callee.
    pub(crate) fn gp(p: usize) -> u32 {
        t(p) as u32
    }
    pub(crate) unsafe fn rd(p: usize) -> u32 {
        unsafe { *(t(p) as *const u32) }
    }
    pub(crate) unsafe fn rb(p: usize) -> u8 {
        unsafe { *(t(p) as *const u8) }
    }
    pub(crate) unsafe fn rf(p: usize) -> f32 {
        unsafe { *(t(p) as *const f32) }
    }
    pub(crate) unsafe fn rf64(p: usize) -> f64 {
        unsafe { std::ptr::read_unaligned(t(p) as *const f64) }
    }
    pub(crate) unsafe fn wd(p: usize, v: u32) {
        unsafe { *(t(p) as *mut u32) = v }
    }
    pub(crate) unsafe fn wb(p: usize, v: u8) {
        unsafe { *(t(p) as *mut u8) = v }
    }
    pub(crate) unsafe fn wf(p: usize, v: f32) {
        unsafe { *(t(p) as *mut f32) = v }
    }
    #[cfg(test)]
    pub(crate) unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
        unsafe { crate::portlib::call_regs(dt::redirect(f), eax, ecx, edx, esi, edi, args) }
    }
    #[cfg(not(test))]
    pub(crate) use crate::portlib::call_regs;

    /// `call_regs` for a callee that returns on the x87 stack: the result is popped as a qword.
    pub(crate) unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
        #[cfg(test)]
        let f = dt::redirect(f);
        let mut out = 0f64;
        let block: [u32; 9] =
            [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
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

    /// `fld dword; fstp dword`: quiets a signalling NaN.
    fn q(x: f32) -> f32 {
        std::hint::black_box(x as f64) as f32
    }

    /// The x87 result of `r = a op b` when both operands are NaNs: the larger significand wins
    /// (SSE would return the first operand).
    fn x87(a: f64, b: f64, r: f64) -> f64 {
        if a.is_nan() && b.is_nan() {
            let m = |v: f64| (v.to_bits() | 1 << 51) & 0xF_FFFF_FFFF_FFFF;
            let pick = if m(b) > m(a) { b } else { a };
            f64::from_bits(pick.to_bits() | 1 << 51)
        } else {
            r
        }
    }

    /// Per-branch counters of the large ports.
    pub(crate) static HITS: [AtomicU32; 16] = [const { AtomicU32::new(0) }; 16];
    fn hit(i: usize) {
        HITS[i].fetch_add(1, Ordering::Relaxed);
    }

    // --- shared pieces ------------------------------------------------------------------------------
    const HANDLE_TABLE: usize = 0x0214_9C64;
    const PLAYER: usize = 0x0252_7D10;
    const LOCAL_PLAYER: usize = 0x0217_03D4;
    const FLAG_2529686: usize = 0x0252_9686;

    /// The object a handle names in the 4096-slot table (stride 16), 0 when the slot is empty or
    /// holds another handle.
    unsafe fn handle_obj(h: u32) -> u32 {
        unsafe {
            if h == 0 {
                return 0;
            }
            let o = rd(HANDLE_TABLE + (((h & 0xFFF) as usize) << 4));
            if o != 0 && rd(o as usize + 0x44) == h { o } else { 0 }
        }
    }

    /// The `imul 0xac769185` signed-division-by-190 expansion, then `* 0x5e8 + 0x340d240`: the
    /// 190-slot instance array. The original tests the sum for zero (never true).
    fn inst_of(ecx: i32) -> u32 {
        let p = (ecx as i64) * (0xac76_9185u32 as i32 as i64);
        let hi = (p >> 32) as i32;
        let edx = hi.wrapping_add(ecx) >> 7;
        let eax = ((edx as u32) >> 31) as i32 + edx;
        let rem = ecx.wrapping_sub(eax.wrapping_mul(0xbe));
        (rem.wrapping_mul(0x5e8) as u32).wrapping_add(0x340d240)
    }

    /// Virtual call as the originals make it: `ecx = obj`, `edx = vtable`, `eax = function`.
    unsafe fn vcall(obj: u32, slot: usize, args: &[u32]) -> u32 {
        unsafe {
            let vt = rd(obj as usize);
            let f = rd(vt as usize + slot * 4);
            call_regs(f as usize, f, obj, vt, 0, 0, args)
        }
    }

    /// The two inlined critical-section release sequences (`state` = 4, then 3).
    unsafe fn cs_release(state: u32) {
        const CS1: u32 = 0x33d6404;
        const CS2: u32 = 0x33d64d4;
        unsafe {
            let ecx = rd(0x33d6428);
            wd(0x33d6430, state);
            let enter = rd(0xdb80cc) as usize;
            let leave;
            if ecx == 2 {
                call_regs(enter, 0, 0, 0, 0, 0, &[CS1]);
                if rd(0x33d6428) == 2 {
                    call_regs(enter, 0, 0, 0, 0, 0, &[CS1]);
                    let ev = rd(0x33d6438);
                    call_regs(rd(0xdb81dc) as usize, 0, 0, 0, 0, 0, &[ev]);
                    leave = rd(0xdb80d0) as usize;
                    wd(0x33d6428, 3);
                    call_regs(leave, 0, 0, 0, 0, 0, &[CS1]);
                    wb(0x33d6420, 0);
                    call_regs(leave, 0, 0, 0, 0, 0, &[CS1]);
                } else {
                    leave = rd(0xdb80d0) as usize;
                    call_regs(leave, 0, 0, 0, 0, 0, &[CS1]);
                }
            } else {
                leave = rd(0xdb80d0) as usize;
            }
            let ecx = rd(0x33d64f8);
            wd(0x33d6500, state);
            if ecx == 2 {
                call_regs(enter, 0, 0, 0, 0, 0, &[CS2]);
                if rd(0x33d64f8) == 2 {
                    call_regs(enter, 0, 0, 0, 0, 0, &[CS2]);
                    let ev = rd(0x33d6508);
                    call_regs(rd(0xdb81dc) as usize, 0, 0, 0, 0, 0, &[ev]);
                    wd(0x33d64f8, 3);
                    call_regs(leave, 0, 0, 0, 0, 0, &[CS2]);
                    wb(0x33d64f0, 0);
                }
                call_regs(leave, 0, 0, 0, 0, 0, &[CS2]);
            }
        }
    }

    // === cutscene_shot_place_characters 0x6C24E0 (0xA03 bytes, cdecl, one stack argument) =========
    //
    // For each character of the shot record (`rec+0x100` array of 0x7C-byte entries, count at
    // `rec+0x108`): resolve the script object to a human, teleport it onto the entry's position
    // facing the shot's reference point, set weapon / look-at / packet state, then reset its
    // animation instance; a second pass re-resolves the humans and releases the audio sections.
    //
    // Deviation: the original reads `[4]` (crash) when the human's `[+0x5e0]->[+0x26c]` byte is not 1
    // on the "no controlled player" path; the port skips the two stores there.
    pub(crate) const PC_ADDR: usize = 0x6C24E0;

    /// The human a shot-record name resolves to (both passes inline the same lookup, with the
    /// crc-list walk replacing `script_object_find` in the second).
    unsafe fn resolve_a(name: u32) -> u32 {
        unsafe {
            let mut found = 0u8;
            let mut eax = call_regs(0xa52bb0, &mut found as *mut u8 as u32, name, 0, 0, 0, &[]);
            if std::ptr::read_volatile(&found) == 0 {
                let n = call_regs(0xa275f0, name, 0, 0, 0, 0, &[]) as i32;
                if n > 0 {
                    eax = call_regs(0xa27660, 0, 0, 0, 0, 0, &[n as u32]);
                } else {
                    let o = call_regs(0xa618a0, name, 0, 0, 0, 0, &[]);
                    if o == 0 {
                        return 0;
                    }
                    eax = vcall(o, 13, &[]);
                }
            }
            eax
        }
    }

    unsafe fn resolve_b(name: u32) -> u32 {
        unsafe {
            let mut found = 0u8;
            let mut eax = call_regs(0xa52bb0, &mut found as *mut u8 as u32, name, 0, 0, 0, &[]);
            if std::ptr::read_volatile(&found) == 0 {
                let n = call_regs(0xa275f0, name, 0, 0, 0, 0, &[]) as i32;
                if n > 0 {
                    eax = call_regs(0xa27660, 0, 0, 0, 0, 0, &[n as u32]);
                } else {
                    // inlined script_object_find: crc32 of the name, then the list at 0x2f25254
                    let mut crc = 0u32;
                    let r = call_regs(0xbdca30, &mut crc as *mut u32 as u32, 0, 0, 0, 0, &[name]);
                    let head = rd(0x2f25254);
                    if head == 0 {
                        return 0;
                    }
                    let want = rd(r as usize);
                    let mut c = head;
                    loop {
                        if rd(c as usize + 0xc) == want {
                            break;
                        }
                        c = rd(c as usize + 0x17c);
                        if c == head || c == 0 {
                            return 0;
                        }
                    }
                    if c == 0 {
                        return 0;
                    }
                    eax = vcall(c, 13, &[]);
                }
            }
            eax
        }
    }

    /// The 0.75 adjustment both weapon-state branches apply to the human's `[+0x3d0]` float.
    unsafe fn adjusted(edi: u32) -> f32 {
        unsafe {
            let f = q(rf(edi as usize + 0x3d0));
            let c75 = rf(0xdd1b58) as f64;
            // fcom: ST0 >= operand or unordered (jp) keeps it; otherwise subtract the double
            if (f as f64) < c75 { ((f as f64) - rf64(0xe5bfc8)) as f32 } else { f }
        }
    }

    /// `[human+0x44]` names a human (`+0x4c == 1`) whose `+0x3cc` byte equals `kind`.
    unsafe fn target_of(edi: u32, kind: u8) -> u32 {
        unsafe {
            let o = handle_obj(rd(edi as usize + 0x44));
            if o != 0 && rd(o as usize + 0x4c) == 1 && rb(o as usize + 0x3cc) == kind { o } else { 0 }
        }
    }

    unsafe fn place_one_a(rec: u32, i: i32, out_pos: &[f32; 3]) {
        unsafe {
            let ebp = rd(rec as usize + 0x100).wrapping_add((i as u32).wrapping_mul(0x7c));
            let name = rd(ebp as usize);
            if name == 0 {
                return;
            }
            let edi = resolve_a(name);
            if edi == 0 {
                return;
            }
            let e = ebp as usize;
            if rb(e + 0x37) == 0 {
                hit(1);
                let pos = [q(rf(e + 4)), q(rf(e + 8)), q(rf(e + 0xc))];
                let mut fwd = [0f32; 3];
                for k in 0..3 {
                    let (o, p) = (out_pos[k] as f64, pos[k] as f64);
                    fwd[k] = x87(o, p, o - p) as f32;
                }
                let (x, y, z) = (fwd[0] as f64, fwd[1] as f64, fwd[2] as f64);
                let (xx, yy, zz) = (x * x, y * y, z * z);
                let s1 = x87(xx, yy, xx + yy);
                let d2 = x87(s1, zz, s1 + zz) as f32;
                let len = call_regs_st0(0x40ea20, 0, 0, 0, 0, 0, &[d2.to_bits()]);
                if len > rf(0xdd0584) as f64 && rb(rd(PLAYER) as usize + 0xafd) == 1 {
                    hit(2);
                    call_regs(0xbddff0, 0, 0, 0, fwd.as_mut_ptr() as u32, 0, &[]);
                } else {
                    fwd = [0.0, 0.0, 1.0];
                }
                let mut mat = [0u32; 9];
                call_regs(0xbe4b20, fwd.as_ptr() as u32, mat.as_mut_ptr() as u32, 0, 0, 0, &[]);
                let angle = q(rf(e + 0x10));
                call_regs(0xbe51d0, 0, 0, 0, 0, 0, &[mat.as_ptr() as u32, angle.to_bits()]);
                let mut pos = pos;
                call_regs(0x6c78b0, 0, pos.as_mut_ptr() as u32, 0, 0, 0, &[pos.as_ptr() as u32, 0]);
                call_regs(0x6c7b90, 0, 0, 0, 0, 0, &[mat.as_ptr() as u32, mat.as_ptr() as u32]);
                wd(edi as usize + 0x3d8, rd(0x252a6d0));
                wd(edi as usize + 0x3dc, rd(0x252a6d4));
                wd(edi as usize + 0x3e0, rd(0x252a6d8));
                call_regs(0x9a0b00, 0, mat.as_ptr() as u32, pos.as_ptr() as u32, 0, 0, &[edi, 0, 1, 1, 0]);
            }
            let d = edi as usize;
            let inv = (edi + 0x10c4) as u32;
            let w = rd(e + 0x14);
            let slot = if w != 0 { rd(w as usize + 0x3c) } else { 0 };
            call_regs(0x55a930, 0, 0, 0, 0, 0, &[inv, slot, 1, 0]);
            wd(d + 0x10a4, rd(e + 0x18));
            let p = rd(e + 0x1c);
            wd(d + 0x10a8, if p != 0 && rd(p as usize + 0x14) != 0 { p } else { 0 });
            wd(d + 0x10ac, rd(e + 0x20));
            wd(d + 0x10b0, rd(e + 0x24));

            let mut lamp: Option<[f32; 3]> = None;
            if rb(e + 0x2c) == 1 {
                let f = q(rf(e + 0x30));
                lamp = Some([f, f, f]);
            } else if rb(e + 0x28) == 1 {
                wb(d + 0x1074, 1);
                if target_of(edi, 1) != 0 {
                    let f = adjusted(edi);
                    lamp = Some([f, f, f]);
                }
            } else if rb(e + 0x2a) == 0 && rb(e + 0x29) == 0 && rb(e + 0x2b) == 0 {
                wb(d + 0x1074, 0);
            } else {
                wb(d + 0x1074, 1);
                if target_of(edi, 1) != 0 {
                    let f = adjusted(edi) as f64;
                    let (b2a, b29, b2b) = (rb(e + 0x2a) as f64, rb(e + 0x29) as f64, rb(e + 0x2b) as f64);
                    lamp = Some([(b2b * f) as f32, (b29 * f) as f32, (b2a * f) as f32]);
                }
            }
            if let Some(a) = lamp {
                call_regs(0x95be10, 0, 0, 0, edi + 0x106c, 0, &[a[0].to_bits(), a[1].to_bits(), a[2].to_bits()]);
            }
            call_regs(0x926860, 0, edi, 0, 0, 0, &[rb(e + 0x34) as u32, 1, 0]);
            if rb(e + 0x35) == 1 {
                call_regs(0x55a930, 0, 0, 0, 0, 0, &[inv, 0xb, 1, 0]);
            }
            wb(d + 0x1089, rb(e + 0x38));
            if rb(e + 0x38) == 0 {
                if target_of(edi, 1) == 0 {
                    let pp = rd(d + 0xd4);
                    let ecx = rd(pp as usize + 8);
                    if rb(ecx as usize + 0x60) & 1 != 0 {
                        let a = rd(d + 0x5e0) as usize;
                        let fv = q(rf(rd(LOCAL_PLAYER) as usize + 0x108c));
                        if rb(a + 0x26c) == 1 {
                            let t = rd(a + 0x268) as usize;
                            wf(t + 0x1c, (0.0f64 * rf64(0xe5c2a0)) as f32);
                            wf(t + 0x20, fv);
                        }
                    }
                }
                wf(d + 0x108c, 0.0);
                call_regs(0x9a5ad0, 0, 0, 0, 0, 0, &[edi]);
            } else {
                wb(d + 0x1088, 0);
                wf(d + 0x1094, 0.0);
            }
            let tg = target_of(edi, 2);
            if tg != 0 {
                if rb(e + 0x39) == 1 {
                    let a = rd(tg as usize + 0x5e0) as usize;
                    if a != 0 && rb(a + 0x26c) == 1 {
                        call_regs(0x6e4de0, rd(a + 0x268), 0, 0, 0, 0, &[]);
                    }
                } else if rb(e + 0x3a) == 1 {
                    let a = rd(tg as usize + 0x5e0) as usize;
                    if a != 0 && rb(a + 0x26c) == 1 {
                        call_regs(0x6e4e10, rd(a + 0x268), 0, 0, 0, 0, &[]);
                    }
                }
            }
            wd(e + 0x3c, 0xFFFF_FFFF);
            let id = rd(rd(d + 0x5e0) as usize);
            if id != 0xFFFF_FFFF {
                call_regs(0xcebb60, 0, id, 0, 0, 0, &[]);
            }
            call_regs(0x6c2190, 0, 0, 0, 0, edi, &[ebp, 0]);
            call_regs(0x6c2050, 0, 0, 0, 0, 0, &[ebp, edi, 0]);
            call_regs(0x6c22c0, 0, 0, 0, ebp, 0, &[edi, 0]);
            let id = rd(rd(d + 0x5e0) as usize);
            if id == 0xFFFF_FFFF {
                return;
            }
            hit(3);
            let mut count = 0u32;
            let mut buf = [0u32; 64];
            call_regs(0xcec6c0, 0, id, 0, 0, 0, &[&mut count as *mut u32 as u32, buf.as_mut_ptr() as u32]);
            let id = rd(rd(d + 0x5e0) as usize) as i32;
            if id >= 0 {
                let inst = inst_of(id);
                if inst != 0 {
                    let n = std::ptr::read_volatile(&count) as i32;
                    let mut k = 0usize;
                    if n > 0 {
                        loop {
                            call_regs(0xce7f20, 0, buf[k], inst, 0, 0, &[1, 1]);
                            k += 1;
                            if k as i32 >= n {
                                break;
                            }
                        }
                    }
                }
            }
            let id = rd(rd(d + 0x5e0) as usize);
            wb(FLAG_2529686, 0);
            call_regs(0xcec840, 0, id, 0, 0, 0, &[]);
            call_regs(0x9a68d0, edi, 0, 0, 0, 0, &[]);
            call_regs(0x9a7150, 0, 0, 0, 0, 0, &[]);
            call_regs(0x9a9010, edi, 0, 0, 0, 0, &[]);
        }
    }

    unsafe fn place_one_b(rec: u32, i: i32) {
        unsafe {
            let ebp = rd(rec as usize + 0x100).wrapping_add((i as u32).wrapping_mul(0x7c));
            let name = rd(ebp as usize);
            if name == 0 {
                return;
            }
            let esi = resolve_b(name);
            if esi == 0 {
                return;
            }
            hit(4);
            call_regs(0x6c2340, 0, 0, 0, 0, esi, &[ebp, 0]);
            let s = esi as usize;
            let id = rd(rd(s + 0x5e0) as usize) as i32;
            if id != -1 && id >= 0 {
                let inst = inst_of(id) as usize;
                if inst != 0 {
                    wb(inst + 0x251, 0);
                    let m = rd(inst + 0xc);
                    if m != 0 {
                        call_regs(0xca0bc0, 0, 0, 0, 0, 0, &[m, 0, 0x300]);
                    }
                }
            }
            call_regs(0x9a68d0, esi, 0, 0, 0, 0, &[]);
            cs_release(4);
            let n = rd(0x252a634) as i32;
            if n > 0 {
                let mut p = gp(0x236fa00);
                let mut k = 0i32;
                loop {
                    if rb(p as usize + 0x18) & 0x10 != 0 {
                        call_regs(0xcedf60, 0, 0, 0, p, 0, &[]);
                    }
                    k += 1;
                    p = p.wrapping_add(0x1c);
                    if k >= rd(0x252a634) as i32 {
                        break;
                    }
                }
            }
            let obj = rd(0x236f9fc);
            if obj != 0 {
                let o = obj as usize;
                let f = rd(rd(o) as usize + 0x10);
                wd(o + 0x34, 0x33d6328);
                call_regs(f as usize, f, obj, 0, 0, 0, &[]);
                wd(o + 4, 0);
                wb(o + 8, 0);
                wd(o + 0x34, 0);
                wb(o + 9, 0);
                wd(o + 0x3c, 0);
                wd(o + 0x40, 0);
                wd(o + 0x44, 0);
                let g = rd(0xec5b98);
                let f = rd(g as usize + 4);
                call_regs(f as usize, f, gp(0xec5b98), 0, 0, 0, &[obj.wrapping_sub(8)]);
            }
            wd(0x252a634, 0);
            cs_release(3);
            call_regs(0x9a9010, esi, 0, 0, 0, 0, &[]);
        }
    }

    pub(crate) unsafe fn place_characters(rec: u32) {
        unsafe {
            hit(0);
            let mut out_pos = [0f32; 3];
            call_regs(0x6beed0, out_pos.as_mut_ptr() as u32, rec, 0, 0, 0, &[]);
            if rd(rec as usize + 0x108) as i32 > 0 {
                let mut i = 0i32;
                loop {
                    place_one_a(rec, i, &out_pos);
                    i += 1;
                    if i >= rd(rec as usize + 0x108) as i32 {
                        break;
                    }
                }
            }
            wb(FLAG_2529686, 0);
            if rd(rec as usize + 0x108) as i32 > 0 {
                let mut i = 0i32;
                loop {
                    place_one_b(rec, i);
                    i += 1;
                    if i >= rd(rec as usize + 0x108) as i32 {
                        break;
                    }
                }
            }
            wb(FLAG_2529686, 0);
        }
    }

    extern "C" fn place_characters_w(rec: u32) {
        unsafe { place_characters(rec) }
    }
    keep_ecx_edx_arg1!(place_characters_t, place_characters_w);

    pub(crate) const PC_BODY: [u8; 2563] = [
    0x81, 0xEC, 0x70, 0x02, 0x00, 0x00, 0x53, 0x55, 0x56, 0x8B, 0xB4, 0x24, 0x80, 0x02, 0x00, 0x00,
    0x57, 0x8B, 0xCE, 0x8D, 0x44, 0x24, 0x4C, 0xE8, 0xD4, 0xC9, 0xFF, 0xFF, 0x83, 0xBE, 0x08, 0x01,
    0x00, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x8E, 0x4B, 0x06, 0x00,
    0x00, 0x8B, 0x6C, 0x24, 0x20, 0x8B, 0x84, 0x24, 0x84, 0x02, 0x00, 0x00, 0x6B, 0xED, 0x7C, 0x03,
    0xA8, 0x00, 0x01, 0x00, 0x00, 0x8B, 0x75, 0x00, 0x85, 0xF6, 0x0F, 0x84, 0x0E, 0x06, 0x00, 0x00,
    0x8D, 0x44, 0x24, 0x1F, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x1F, 0x00, 0xE8, 0x70, 0x06, 0x39, 0x00,
    0x80, 0x7C, 0x24, 0x1F, 0x00, 0x75, 0x36, 0x8B, 0xC6, 0xE8, 0xA2, 0x50, 0x36, 0x00, 0x85, 0xC0,
    0x7E, 0x0B, 0x50, 0xE8, 0x08, 0x51, 0x36, 0x00, 0x83, 0xC4, 0x04, 0xEB, 0x18, 0x8B, 0xC6, 0xE8,
    0x3C, 0xF3, 0x39, 0x00, 0x85, 0xC0, 0x0F, 0x84, 0xD2, 0x05, 0x00, 0x00, 0x8B, 0x10, 0x8B, 0xC8,
    0x8B, 0x42, 0x34, 0xFF, 0xD0, 0x85, 0xC0, 0x0F, 0x84, 0xC1, 0x05, 0x00, 0x00, 0x8B, 0xF8, 0x85,
    0xFF, 0x0F, 0x84, 0xB7, 0x05, 0x00, 0x00, 0x80, 0x7D, 0x37, 0x00, 0x0F, 0x85, 0x30, 0x01, 0x00,
    0x00, 0xD9, 0x45, 0x04, 0x51, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x45, 0x08, 0xD9, 0x5C, 0x24, 0x38,
    0xD9, 0x45, 0x0C, 0xD9, 0x5C, 0x24, 0x3C, 0xD9, 0x44, 0x24, 0x50, 0xD8, 0x64, 0x24, 0x34, 0xD9,
    0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x54, 0xD8, 0x64, 0x24, 0x38, 0xD9, 0x5C, 0x24, 0x2C, 0xD9,
    0x44, 0x24, 0x58, 0xD8, 0x64, 0x24, 0x3C, 0xD9, 0x5C, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x28, 0xD9,
    0x44, 0x24, 0x2C, 0xD9, 0x44, 0x24, 0x30, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9,
    0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0xD9,
    0x1C, 0x24, 0xE8, 0x29, 0xC4, 0xD4, 0xFF, 0xD8, 0x1D, 0x84, 0x05, 0xDD, 0x00, 0x83, 0xC4, 0x04,
    0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x1A, 0x8B, 0x0D, 0x10, 0x7D, 0x52, 0x02, 0x80, 0xB9, 0xFD,
    0x0A, 0x00, 0x00, 0x01, 0x75, 0x0B, 0x8D, 0x74, 0x24, 0x24, 0xE8, 0xD1, 0xB9, 0x51, 0x00, 0xEB,
    0x28, 0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x40, 0x8B, 0x54, 0x24, 0x40, 0xD9, 0x5C, 0x24, 0x44, 0xD9,
    0xE8, 0x8B, 0x44, 0x24, 0x44, 0xD9, 0x5C, 0x24, 0x48, 0x89, 0x54, 0x24, 0x24, 0x89, 0x44, 0x24,
    0x28, 0x8B, 0x4C, 0x24, 0x48, 0x89, 0x4C, 0x24, 0x2C, 0x8D, 0x44, 0x24, 0x24, 0x8D, 0x4C, 0x24,
    0x5C, 0xE8, 0xCA, 0x24, 0x52, 0x00, 0xD9, 0x45, 0x10, 0x51, 0x8D, 0x54, 0x24, 0x60, 0xD9, 0x1C,
    0x24, 0x52, 0xE8, 0x69, 0x2B, 0x52, 0x00, 0x8D, 0x44, 0x24, 0x30, 0x6A, 0x00, 0x50, 0x8B, 0xC8,
    0xE8, 0x3B, 0x52, 0x00, 0x00, 0x8D, 0x4C, 0x24, 0x64, 0x51, 0x8B, 0xD1, 0x52, 0xE8, 0x0E, 0x55,
    0x00, 0x00, 0xA1, 0xD0, 0xA6, 0x52, 0x02, 0x89, 0x87, 0xD8, 0x03, 0x00, 0x00, 0x8B, 0x0D, 0xD4,
    0xA6, 0x52, 0x02, 0x6A, 0x00, 0x6A, 0x01, 0x89, 0x8F, 0xDC, 0x03, 0x00, 0x00, 0x8B, 0x15, 0xD8,
    0xA6, 0x52, 0x02, 0x6A, 0x01, 0x89, 0x97, 0xE0, 0x03, 0x00, 0x00, 0x6A, 0x00, 0x57, 0x8D, 0x8C,
    0x24, 0x80, 0x00, 0x00, 0x00, 0x8D, 0x54, 0x24, 0x54, 0xE8, 0x42, 0xE4, 0x2D, 0x00, 0x83, 0xC4,
    0x24, 0x8B, 0x45, 0x14, 0x85, 0xC0, 0x6A, 0x00, 0x8D, 0x9F, 0xC4, 0x10, 0x00, 0x00, 0x6A, 0x01,
    0x74, 0x06, 0x8B, 0x40, 0x3C, 0x50, 0xEB, 0x02, 0x6A, 0x00, 0x53, 0xE8, 0x50, 0x82, 0xE9, 0xFF,
    0x8B, 0x4D, 0x18, 0x89, 0x8F, 0xA4, 0x10, 0x00, 0x00, 0x8B, 0x45, 0x1C, 0x85, 0xC0, 0x74, 0x06,
    0x83, 0x78, 0x14, 0x00, 0x75, 0x02, 0x33, 0xC0, 0x89, 0x87, 0xA8, 0x10, 0x00, 0x00, 0x8B, 0x55,
    0x20, 0x89, 0x97, 0xAC, 0x10, 0x00, 0x00, 0x8B, 0x45, 0x24, 0x89, 0x87, 0xB0, 0x10, 0x00, 0x00,
    0x80, 0x7D, 0x2C, 0x01, 0x75, 0x1B, 0xD9, 0x45, 0x30, 0x83, 0xEC, 0x0C, 0xD9, 0x5C, 0x24, 0x20,
    0xD9, 0x44, 0x24, 0x20, 0xD9, 0x54, 0x24, 0x08, 0xD9, 0x54, 0x24, 0x04, 0xE9, 0x74, 0x01, 0x00,
    0x00, 0x80, 0x7D, 0x28, 0x01, 0x0F, 0x85, 0x87, 0x00, 0x00, 0x00, 0x8B, 0x4F, 0x44, 0x85, 0xC9,
    0xC6, 0x87, 0x74, 0x10, 0x00, 0x00, 0x01, 0x0F, 0x84, 0x66, 0x01, 0x00, 0x00, 0x8B, 0xC1, 0x25,
    0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F,
    0x84, 0x4E, 0x01, 0x00, 0x00, 0x39, 0x48, 0x44, 0x0F, 0x85, 0x45, 0x01, 0x00, 0x00, 0x83, 0x78,
    0x4C, 0x01, 0x0F, 0x85, 0x3B, 0x01, 0x00, 0x00, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x0F,
    0x85, 0x2E, 0x01, 0x00, 0x00, 0xD9, 0x87, 0xD0, 0x03, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x18, 0xD9,
    0x05, 0x58, 0x1B, 0xDD, 0x00, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6,
    0xC4, 0x05, 0x7A, 0x0E, 0xDC, 0x25, 0xC8, 0xBF, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44,
    0x24, 0x18, 0x83, 0xEC, 0x0C, 0xD9, 0x54, 0x24, 0x08, 0xD9, 0x54, 0x24, 0x04, 0xE9, 0xE3, 0x00,
    0x00, 0x00, 0x80, 0x7D, 0x2A, 0x00, 0x75, 0x18, 0x80, 0x7D, 0x29, 0x00, 0x75, 0x12, 0x80, 0x7D,
    0x2B, 0x00, 0x75, 0x0C, 0xC6, 0x87, 0x74, 0x10, 0x00, 0x00, 0x00, 0xE9, 0xD3, 0x00, 0x00, 0x00,
    0x8B, 0x4F, 0x44, 0x85, 0xC9, 0xC6, 0x87, 0x74, 0x10, 0x00, 0x00, 0x01, 0x0F, 0x84, 0xC1, 0x00,
    0x00, 0x00, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0xA8, 0x00, 0x00, 0x00, 0x39, 0x48, 0x44, 0x0F, 0x85,
    0x9F, 0x00, 0x00, 0x00, 0x83, 0x78, 0x4C, 0x01, 0x0F, 0x85, 0x95, 0x00, 0x00, 0x00, 0x80, 0xB8,
    0xCC, 0x03, 0x00, 0x00, 0x01, 0x0F, 0x85, 0x88, 0x00, 0x00, 0x00, 0xD9, 0x87, 0xD0, 0x03, 0x00,
    0x00, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x05, 0x58, 0x1B, 0xDD, 0x00, 0xD9, 0x44, 0x24, 0x18, 0xD8,
    0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x05, 0x7A, 0x0E, 0xDC, 0x25, 0xC8, 0xBF, 0xE5, 0x00,
    0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0x0F, 0xB6, 0x45, 0x2A, 0x0F, 0xB6, 0x4D, 0x29,
    0x0F, 0xB6, 0x55, 0x2B, 0x89, 0x44, 0x24, 0x14, 0x83, 0xEC, 0x0C, 0xDB, 0x44, 0x24, 0x20, 0xD8,
    0xC9, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x20, 0x89, 0x4C, 0x24, 0x20, 0xD9, 0x5C, 0x24,
    0x08, 0xDB, 0x44, 0x24, 0x20, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x20, 0x89,
    0x54, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x04, 0xDB, 0x44, 0x24, 0x20, 0xDE, 0xC9, 0xD9, 0x5C, 0x24,
    0x20, 0xD9, 0x44, 0x24, 0x20, 0x8D, 0xB7, 0x6C, 0x10, 0x00, 0x00, 0xD9, 0x1C, 0x24, 0xE8, 0x5D,
    0x95, 0x29, 0x00, 0x0F, 0xB6, 0x45, 0x34, 0x6A, 0x00, 0xBE, 0x01, 0x00, 0x00, 0x00, 0x56, 0x50,
    0x8B, 0xCF, 0xE8, 0x99, 0x3F, 0x26, 0x00, 0x83, 0xC4, 0x0C, 0x80, 0x7D, 0x35, 0x01, 0x75, 0x0B,
    0x6A, 0x00, 0x56, 0x6A, 0x0B, 0x53, 0xE8, 0x55, 0x80, 0xE9, 0xFF, 0x8A, 0x4D, 0x38, 0x88, 0x8F,
    0x89, 0x10, 0x00, 0x00, 0x80, 0x7D, 0x38, 0x00, 0x0F, 0x85, 0x92, 0x00, 0x00, 0x00, 0x8B, 0x4F,
    0x44, 0x85, 0xC9, 0x74, 0x28, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04,
    0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x13, 0x39, 0x48, 0x44, 0x75, 0x0E, 0x39,
    0x70, 0x4C, 0x75, 0x09, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x74, 0x50, 0x8B, 0x87, 0xD4,
    0x00, 0x00, 0x00, 0x8B, 0x48, 0x08, 0xF6, 0x41, 0x60, 0x01, 0x74, 0x41, 0x8B, 0x87, 0xE0, 0x05,
    0x00, 0x00, 0x8A, 0x90, 0x6C, 0x02, 0x00, 0x00, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0xD9, 0x81,
    0x8C, 0x10, 0x00, 0x00, 0x05, 0x64, 0x02, 0x00, 0x00, 0x80, 0xEA, 0x01, 0xD9, 0x5C, 0x24, 0x14,
    0xD9, 0xEE, 0xF6, 0xDA, 0xDC, 0x0D, 0xA0, 0xC2, 0xE5, 0x00, 0x1B, 0xD2, 0xF7, 0xD2, 0x23, 0xC2,
    0x8B, 0x40, 0x04, 0xD9, 0x58, 0x1C, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x58, 0x20, 0xD9, 0xEE, 0x57,
    0xD9, 0x9F, 0x8C, 0x10, 0x00, 0x00, 0xE8, 0x55, 0x31, 0x2E, 0x00, 0x83, 0xC4, 0x04, 0xEB, 0x0F,
    0xD9, 0xEE, 0xC6, 0x87, 0x88, 0x10, 0x00, 0x00, 0x00, 0xD9, 0x9F, 0x94, 0x10, 0x00, 0x00, 0x8B,
    0x4F, 0x44, 0x85, 0xC9, 0x0F, 0x84, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0xA7,
    0x00, 0x00, 0x00, 0x39, 0x48, 0x44, 0x0F, 0x85, 0x9E, 0x00, 0x00, 0x00, 0x39, 0x70, 0x4C, 0x0F,
    0x85, 0x95, 0x00, 0x00, 0x00, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x0F, 0x85, 0x88, 0x00,
    0x00, 0x00, 0x85, 0xC9, 0x74, 0x1E, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04,
    0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0A, 0x39, 0x48, 0x44, 0x75, 0x05, 0x39,
    0x70, 0x4C, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x10, 0x33, 0xC9, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00,
    0x02, 0x0F, 0x95, 0xC1, 0x2B, 0xCE, 0x23, 0xC1, 0x80, 0x7D, 0x39, 0x01, 0x75, 0x24, 0x85, 0xC0,
    0x74, 0x48, 0x8B, 0x80, 0xE0, 0x05, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x3E, 0x80, 0xB8, 0x6C, 0x02,
    0x00, 0x00, 0x01, 0x75, 0x35, 0x8B, 0x80, 0x68, 0x02, 0x00, 0x00, 0xE8, 0xB0, 0x23, 0x02, 0x00,
    0xEB, 0x28, 0x80, 0x7D, 0x3A, 0x01, 0x75, 0x22, 0x85, 0xC0, 0x74, 0x1E, 0x8B, 0x80, 0xE0, 0x05,
    0x00, 0x00, 0x85, 0xC0, 0x74, 0x14, 0x80, 0xB8, 0x6C, 0x02, 0x00, 0x00, 0x01, 0x75, 0x0B, 0x8B,
    0x80, 0x68, 0x02, 0x00, 0x00, 0xE8, 0xB6, 0x23, 0x02, 0x00, 0xC7, 0x45, 0x3C, 0xFF, 0xFF, 0xFF,
    0xFF, 0x8B, 0x97, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x0A, 0x83, 0xF9, 0xFF, 0x74, 0x05, 0xE8, 0xED,
    0x90, 0x62, 0x00, 0x6A, 0x00, 0x55, 0xE8, 0x15, 0xF7, 0xFF, 0xFF, 0x6A, 0x00, 0x57, 0x55, 0xE8,
    0xCC, 0xF5, 0xFF, 0xFF, 0x83, 0xC4, 0x14, 0x6A, 0x00, 0x57, 0x8B, 0xF5, 0xE8, 0x2F, 0xF8, 0xFF,
    0xFF, 0x8B, 0x87, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x08, 0x83, 0xC4, 0x08, 0x83, 0xF9, 0xFF, 0x0F,
    0x84, 0x99, 0x00, 0x00, 0x00, 0x8D, 0x94, 0x24, 0x80, 0x00, 0x00, 0x00, 0x52, 0x8D, 0x44, 0x24,
    0x40, 0x50, 0xE8, 0x09, 0x9C, 0x62, 0x00, 0x8B, 0x8F, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x09, 0x83,
    0xC4, 0x08, 0x85, 0xC9, 0x7C, 0x51, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1,
    0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B,
    0xC8, 0x69, 0xC9, 0xE8, 0x05, 0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x8B, 0xD1, 0x74,
    0x26, 0x8B, 0x6C, 0x24, 0x3C, 0x33, 0xF6, 0x85, 0xED, 0x7E, 0x1C, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x8C, 0xB4, 0x80, 0x00, 0x00, 0x00, 0x6A, 0x01, 0x6A, 0x01, 0xE8, 0x10, 0x54, 0x62, 0x00,
    0x83, 0xC6, 0x01, 0x3B, 0xF5, 0x7C, 0xE9, 0x8B, 0x8F, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x09, 0xC6,
    0x05, 0x86, 0x96, 0x52, 0x02, 0x00, 0xE8, 0x15, 0x9D, 0x62, 0x00, 0x8B, 0xC7, 0xE8, 0x9E, 0x3D,
    0x2E, 0x00, 0xE8, 0x19, 0x46, 0x2E, 0x00, 0x8B, 0xC7, 0xE8, 0xD2, 0x64, 0x2E, 0x00, 0x8B, 0x44,
    0x24, 0x20, 0x8B, 0x94, 0x24, 0x84, 0x02, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3B, 0x82, 0x08, 0x01,
    0x00, 0x00, 0x89, 0x44, 0x24, 0x20, 0x0F, 0x8C, 0xB5, 0xF9, 0xFF, 0xFF, 0x8B, 0xBC, 0x24, 0x84,
    0x02, 0x00, 0x00, 0x33, 0xC0, 0x39, 0x87, 0x08, 0x01, 0x00, 0x00, 0xC6, 0x05, 0x86, 0x96, 0x52,
    0x02, 0x00, 0x89, 0x44, 0x24, 0x20, 0x0F, 0x8E, 0x55, 0x03, 0x00, 0x00, 0x8B, 0x1D, 0xCC, 0x80,
    0xDB, 0x00, 0x89, 0x44, 0x24, 0x14, 0x8B, 0xAF, 0x00, 0x01, 0x00, 0x00, 0x03, 0x6C, 0x24, 0x14,
    0x8B, 0x75, 0x00, 0x85, 0xF6, 0x0F, 0x84, 0x1A, 0x03, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x1E, 0x8B,
    0xCE, 0xC6, 0x44, 0x24, 0x1E, 0x00, 0xE8, 0x05, 0x00, 0x39, 0x00, 0x80, 0x7C, 0x24, 0x1E, 0x00,
    0x74, 0x08, 0x8B, 0xF0, 0x89, 0x74, 0x24, 0x18, 0xEB, 0x6F, 0x8B, 0xC6, 0xE8, 0x2F, 0x4A, 0x36,
    0x00, 0x85, 0xC0, 0x7E, 0x0B, 0x50, 0xE8, 0x95, 0x4A, 0x36, 0x00, 0x83, 0xC4, 0x04, 0xEB, 0x4B,
    0x56, 0x33, 0xD2, 0x8D, 0x44, 0x24, 0x5C, 0xE8, 0x54, 0x9E, 0x51, 0x00, 0x8B, 0x15, 0x54, 0x52,
    0xF2, 0x02, 0x85, 0xD2, 0x8B, 0xCA, 0x0F, 0x84, 0xC9, 0x02, 0x00, 0x00, 0x8B, 0x00, 0x8B, 0xFF,
    0x39, 0x41, 0x0C, 0x74, 0x17, 0x8B, 0x89, 0x7C, 0x01, 0x00, 0x00, 0x3B, 0xCA, 0x0F, 0x84, 0xB2,
    0x02, 0x00, 0x00, 0x85, 0xC9, 0x75, 0xE9, 0xE9, 0xA9, 0x02, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0x84,
    0xA1, 0x02, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x0F, 0x84, 0x92,
    0x02, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0x8B, 0xF0, 0x85, 0xF6, 0x0F, 0x84, 0x84, 0x02, 0x00,
    0x00, 0x6A, 0x00, 0x55, 0x8B, 0xFE, 0xE8, 0x05, 0xF7, 0xFF, 0xFF, 0x8B, 0x86, 0xE0, 0x05, 0x00,
    0x00, 0x8B, 0x08, 0x83, 0xC4, 0x08, 0x83, 0xF9, 0xFF, 0x74, 0x4B, 0x85, 0xC9, 0x7C, 0x47, 0xB8,
    0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F,
    0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8, 0x05, 0x00, 0x00,
    0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x74, 0x1E, 0xC6, 0x81, 0x51, 0x02, 0x00, 0x00, 0x00, 0x8B,
    0x49, 0x0C, 0x85, 0xC9, 0x74, 0x10, 0x68, 0x00, 0x03, 0x00, 0x00, 0x6A, 0x00, 0x51, 0xE8, 0x2D,
    0xDF, 0x5D, 0x00, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0xE8, 0x33, 0x3C, 0x2E, 0x00, 0x8B, 0x0D, 0x28,
    0x64, 0x3D, 0x03, 0x83, 0xF9, 0x02, 0xBF, 0x04, 0x00, 0x00, 0x00, 0x89, 0x3D, 0x30, 0x64, 0x3D,
    0x03, 0x75, 0x5B, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0x8B, 0x15, 0x28, 0x64, 0x3D, 0x03,
    0x83, 0xFA, 0x02, 0x75, 0x3A, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0xA1, 0x38, 0x64, 0x3D,
    0x03, 0x50, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x8B, 0x35, 0xD0, 0x80, 0xDB, 0x00, 0xC7, 0x05,
    0x28, 0x64, 0x3D, 0x03, 0x03, 0x00, 0x00, 0x00, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0x68,
    0x04, 0x64, 0x3D, 0x03, 0xC6, 0x05, 0x20, 0x64, 0x3D, 0x03, 0x00, 0xFF, 0xD6, 0xEB, 0x15, 0x8B,
    0x35, 0xD0, 0x80, 0xDB, 0x00, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0xEB, 0x06, 0x8B, 0x35,
    0xD0, 0x80, 0xDB, 0x00, 0x8B, 0x0D, 0xF8, 0x64, 0x3D, 0x03, 0x83, 0xF9, 0x02, 0x89, 0x3D, 0x00,
    0x65, 0x3D, 0x03, 0x75, 0x44, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0x8B, 0x15, 0xF8, 0x64,
    0x3D, 0x03, 0x83, 0xFA, 0x02, 0x75, 0x2B, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0xA1, 0x08,
    0x65, 0x3D, 0x03, 0x50, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0xC7, 0x05, 0xF8, 0x64, 0x3D, 0x03,
    0x03, 0x00, 0x00, 0x00, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0xC6, 0x05, 0xF0, 0x64, 0x3D,
    0x03, 0x00, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0x33, 0xED, 0x39, 0x2D, 0x34, 0xA6, 0x52,
    0x02, 0x7E, 0x1E, 0xBE, 0x00, 0xFA, 0x36, 0x02, 0xF6, 0x46, 0x18, 0x10, 0x74, 0x05, 0xE8, 0xDD,
    0xB1, 0x62, 0x00, 0x83, 0xC5, 0x01, 0x83, 0xC6, 0x1C, 0x3B, 0x2D, 0x34, 0xA6, 0x52, 0x02, 0x7C,
    0xE7, 0x8B, 0x0D, 0xFC, 0xF9, 0x36, 0x02, 0x33, 0xFF, 0x3B, 0xCF, 0x8B, 0xF1, 0x74, 0x3C, 0x8B,
    0x11, 0x8B, 0x42, 0x10, 0x8D, 0x69, 0x34, 0xC7, 0x45, 0x00, 0x28, 0x63, 0x3D, 0x03, 0xFF, 0xD0,
    0x89, 0x7E, 0x04, 0xC6, 0x46, 0x08, 0x00, 0x89, 0x7D, 0x00, 0xC6, 0x46, 0x09, 0x00, 0x89, 0x7E,
    0x3C, 0x89, 0x7E, 0x40, 0x89, 0x7E, 0x44, 0x8B, 0x15, 0x98, 0x5B, 0xEC, 0x00, 0x8B, 0x42, 0x04,
    0x83, 0xC6, 0xF8, 0x56, 0xB9, 0x98, 0x5B, 0xEC, 0x00, 0xFF, 0xD0, 0x8B, 0x0D, 0x28, 0x64, 0x3D,
    0x03, 0x83, 0xF9, 0x02, 0x89, 0x3D, 0x34, 0xA6, 0x52, 0x02, 0xBF, 0x03, 0x00, 0x00, 0x00, 0x89,
    0x3D, 0x30, 0x64, 0x3D, 0x03, 0x75, 0x57, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0x8B, 0x15,
    0x28, 0x64, 0x3D, 0x03, 0x83, 0xFA, 0x02, 0x75, 0x36, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD3,
    0xA1, 0x38, 0x64, 0x3D, 0x03, 0x50, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x8B, 0x35, 0xD0, 0x80,
    0xDB, 0x00, 0x89, 0x3D, 0x28, 0x64, 0x3D, 0x03, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0x68,
    0x04, 0x64, 0x3D, 0x03, 0xC6, 0x05, 0x20, 0x64, 0x3D, 0x03, 0x00, 0xFF, 0xD6, 0xEB, 0x15, 0x8B,
    0x35, 0xD0, 0x80, 0xDB, 0x00, 0x68, 0x04, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0xEB, 0x06, 0x8B, 0x35,
    0xD0, 0x80, 0xDB, 0x00, 0x8B, 0x0D, 0xF8, 0x64, 0x3D, 0x03, 0x83, 0xF9, 0x02, 0x89, 0x3D, 0x00,
    0x65, 0x3D, 0x03, 0x75, 0x40, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0x8B, 0x15, 0xF8, 0x64,
    0x3D, 0x03, 0x83, 0xFA, 0x02, 0x75, 0x27, 0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD3, 0xA1, 0x08,
    0x65, 0x3D, 0x03, 0x50, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x89, 0x3D, 0xF8, 0x64, 0x3D, 0x03,
    0x68, 0xD4, 0x64, 0x3D, 0x03, 0xFF, 0xD6, 0xC6, 0x05, 0xF0, 0x64, 0x3D, 0x03, 0x00, 0x68, 0xD4,
    0x64, 0x3D, 0x03, 0xFF, 0xD6, 0x8B, 0x44, 0x24, 0x18, 0xE8, 0x62, 0x61, 0x2E, 0x00, 0x8B, 0xBC,
    0x24, 0x84, 0x02, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x20, 0x83, 0x44, 0x24, 0x14, 0x7C, 0x83, 0xC0,
    0x01, 0x3B, 0x87, 0x08, 0x01, 0x00, 0x00, 0x89, 0x44, 0x24, 0x20, 0x0F, 0x8C, 0xB5, 0xFC, 0xFF,
    0xFF, 0x5F, 0x5E, 0x5D, 0xC6, 0x05, 0x86, 0x96, 0x52, 0x02, 0x00, 0x5B, 0x81, 0xC4, 0x70, 0x02,
    0x00, 0x00, 0xC3,
];
    #[cfg(test)]
    pub(crate) const PC_CALLS_AT: [usize; 38] = [0x17, 0x5b, 0x69, 0x73, 0x7f, 0x112, 0x13a, 0x171, 0x182, 0x190, 0x19d, 0x1d9, 0x1fb, 0x3ce, 0x3e2, 0x3f6, 0x496, 0x54b, 0x575, 0x58e, 0x596, 0x59f, 0x5ac, 0x5d2, 0x62b, 0x646, 0x64d, 0x652, 0x659, 0x6c6, 0x6dc, 0x6e6, 0x6f7, 0x756, 0x7ae, 0x7b8, 0x89e, 0x9c9];
    #[cfg(test)]
    pub(crate) const PC_DISPS: [usize; 38] = [0x119, 0x129, 0x1a3, 0x1af, 0x1bf, 0x279, 0x2b1, 0x2c6, 0x31f, 0x357, 0x36c, 0x422, 0x45a, 0x476, 0x4c7, 0x502, 0x641, 0x68d, 0x69e, 0x6fe, 0x7f4, 0x7fa, 0x821, 0x830, 0x866, 0x88d, 0x894, 0x8ab, 0x8b3, 0x8e9, 0x8f5, 0x906, 0x938, 0x93e, 0x961, 0x970, 0x9a6, 0x9f6];


    pub fn install() -> (u32, u32) {
        let mut ok = 0;
        let ports: [(&str, usize, &[u8], usize); 1] =
            [("cutscene_shot_place_characters", PC_ADDR, &PC_BODY, place_characters_t as *const () as usize)];
        for (name, addr, body, thunk) in ports {
            if replace_pinned(name, addr, body, thunk) {
                ok += 1;
            }
        }
        (ok, ports.len() as u32)
    }

    pub fn summary() -> String {
        let n = HITS[0].load(Ordering::Relaxed);
        if n == 0 {
            return String::new();
        }
        format!(
            "place_characters {} [teleport {} norm {} anim {} pass2 {}]",
            n,
            HITS[1].load(Ordering::Relaxed),
            HITS[2].load(Ordering::Relaxed),
            HITS[3].load(Ordering::Relaxed),
            HITS[4].load(Ordering::Relaxed)
        )
    }

    #[cfg(test)]
    #[allow(dead_code, clippy::all)]
pub(crate) mod dt {
        use std::cell::{Cell, RefCell};
        use std::ffi::c_void;
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
    
        pub const EAX: u8 = 1;
        pub const ECX: u8 = 2;
        pub const EDX: u8 = 4;
        pub const ESI: u8 = 8;
        pub const EDI: u8 = 16;
    
        /// Where a callee's result comes from.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Ret {
            /// Nothing in EAX that matters (a random value comes back anyway).
            Any,
            /// A flag: 0, 1, 0x100 (AL clear but EAX not), or random.
            Bool,
            /// 0 or a member of pool `n`.
            Ptr(usize),
            /// Always 0.
            Zero,
            /// Always a member of pool `n`.
            PtrNz(usize),
            /// Returns a double on the x87 stack (and a random value in EAX).
            St0,
            /// A random value below `n`.
            Mod(u32),
        }
    
        /// What a callee does besides returning.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Fx {
            None,
            /// Writes a random u32 at [ESI] (the xtbl getters).
            WriteUint,
            /// Writes a random f32 at [ESI].
            WriteFloat,
            /// Logs the bytes of a buffer: `args[0]` pointer, `args[1]` length (byte 3 masked when the
            /// length is 9: the original leaves it unwritten).
            LogBuf,
            /// Logs `ECX * 3` dwords at [EDX] (a vector list).
            LogVec,
            /// Logs the three bytes at [EDX].
            LogEdx3,
            /// Logs `n` dwords at the pointer in `args[0]`.
            LogArg0(usize),
            /// Logs `n` bytes at the pointer in EAX.
            LogEaxBytes(usize),
        }
    
        /// Where a stub finds a pointer.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Src {
            Reg(u8),
            Arg(usize),
        }
    
        /// What a stub writes through a pointer.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Out {
            /// A byte, 0 or 1.
            Byte01,
            /// `n` random f32 bit patterns (small values over-represented).
            Floats(usize),
            /// `n` random dwords.
            Dwords(usize),
            /// A small count at the pointer and that many small ints at the buffer in `Arg(1)`.
            CountBuf,
        }
    
        #[derive(Clone)]
        pub struct Callee {
            pub target: usize,
            pub nargs: usize,
            pub pop: usize,
            /// Registers the callee reads on entry (the only ones compared).
            pub reads: u8,
            pub ret: Ret,
            pub fx: Fx,
            pub silent: bool,
            /// Stack arguments the callee reads as one byte (the caller leaves stale upper bits).
            pub byte_args: u32,
            /// Registers the callee reads as one byte (the caller leaves stale upper bits).
            pub byte_regs: u8,
            /// A native replacement the original's call is aimed at (0: the recording stub).
            pub native: usize,
            pub logs: Vec<(Src, usize)>,
            pub outs: Vec<(Src, Out)>,
        }
    
        pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
            Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, native: 0, logs: Vec::new(), outs: Vec::new() }
        }
    
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
            /// True once in `n`.
            pub fn one_in(&mut self, n: u32) -> bool {
                self.below(n) == 0
            }
            /// One of `v`, or (one time in four) a random value below `modulus` (any u32 when 0).
            pub fn mix(&mut self, v: &[u32], modulus: u32) -> u32 {
                if self.below(4) == 0 {
                    let r = self.u32();
                    if modulus == 0 { r } else { r % modulus }
                } else {
                    self.pick(v)
                }
            }
            pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
                v[self.below(v.len() as u32) as usize]
            }
            /// A float bit pattern with the awkward cases over-represented.
            pub fn float_bits(&mut self) -> u32 {
                match self.below(12) {
                    0 => 0,
                    1 => 0x8000_0000,
                    2 => 0x3F80_0000,
                    3 => 0xBF80_0000,
                    4 => 0x7FC0_0000,
                    5 => 0x7F80_0001, // signalling NaN
                    6 => 0x7F80_0000,
                    7 => 0x0000_0001, // denormal
                    8 => (self.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
                    9 => (self.below(20000) as f32 / 10.0).to_bits(),
                    _ => self.u32(),
                }
            }
        }
    
        pub fn fixed(addr: usize, len: usize) {
            let mut p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
            if p.is_null() {
                p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
            }
            assert_eq!(p as usize, addr, "cannot map {addr:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
        }
    
        /// The fixed regions the original touches: snapshotted, restored and compared as one.
        pub struct Mem {
            pub regions: Vec<(usize, usize)>,
        }
    
        impl Mem {
            /// Regions: every absolute address in `body` that falls in the game's data range (moved by
            /// `DELTA`), plus `extra` (game addresses), plus the arena; all rounded out to 64 KB.
            pub fn new(body: &[u8], extra: &[(usize, usize)]) -> Mem {
                let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::t(a), l)).collect();
                for i in 0..body.len().saturating_sub(3) {
                    let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                    if (0xD0_0000..0x300_0000).contains(&v) {
                        spans.push((super::t(v), 1));
                    }
                }
                spans.push((ARENA, ARENA_LEN));
                let mut pages: Vec<usize> = Vec::new();
                for (a, l) in spans {
                    let mut p = a & !0xFFFF;
                    while p < a + l {
                        pages.push(p);
                        p += 0x1_0000;
                    }
                }
                pages.sort_unstable();
                pages.dedup();
                let mut regions: Vec<(usize, usize)> = Vec::new();
                for p in pages {
                    match regions.last_mut() {
                        Some(r) if r.0 + r.1 == p => r.1 += 0x1_0000,
                        _ => regions.push((p, 0x1_0000)),
                    }
                }
                for &(a, l) in &regions {
                    fixed(a, l);
                }
                Mem { regions }
            }
    
            pub fn fill_random(&self, rng: &mut Rng) {
                for &(a, l) in &self.regions {
                    let s = unsafe { std::slice::from_raw_parts_mut(a as *mut u32, l / 4) };
                    for w in s.iter_mut() {
                        *w = rng.u32();
                    }
                }
            }
    
            /// Zero `l` bytes at the game address `a`.
            pub fn zero(&self, a: usize, l: usize) {
                unsafe { std::ptr::write_bytes(super::t(a) as *mut u8, 0, l) };
            }
    
            pub fn snapshot(&self) -> Vec<u8> {
                let mut v = Vec::new();
                for &(a, l) in &self.regions {
                    v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, l) });
                }
                v
            }
    
            pub fn restore(&self, snap: &[u8]) {
                let mut at = 0;
                for &(a, l) in &self.regions {
                    unsafe { std::ptr::copy_nonoverlapping(snap[at..at + l].as_ptr(), a as *mut u8, l) };
                    at += l;
                }
            }
    
            /// The first difference between two snapshots, as an address.
            pub fn first_diff(&self, a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
                let mut at = 0;
                for &(base, l) in &self.regions {
                    for i in 0..l {
                        if a[at + i] != b[at + i] {
                            return Some((base + i, a[at + i], b[at + i]));
                        }
                    }
                    at += l;
                }
                None
            }
    
            pub fn count_diffs(&self, a: &[u8], b: &[u8]) -> usize {
                a.iter().zip(b).filter(|(x, y)| x != y).count()
            }
        }
    
        #[repr(C)]
        pub struct Pushad {
            edi: u32,
            esi: u32,
            ebp: u32,
            esp: u32,
            ebx: u32,
            edx: u32,
            ecx: u32,
            eax: u32,
        }
    
        thread_local! {
            static CALLEES: RefCell<Vec<Callee>> = const { RefCell::new(Vec::new()) };
            static STUBS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
            static POOLS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
            static LOG: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
            static SRNG: Cell<u64> = const { Cell::new(1) };
        }
    
        pub fn set_pools(p: Vec<Vec<u32>>) {
            POOLS.with(|x| *x.borrow_mut() = p);
        }
    
        /// The address a call to `f` should really go to: its stub when it has one.
        pub fn redirect(f: usize) -> usize {
            CALLEES.with(|c| {
                let c = c.borrow();
                match c.iter().position(|x| x.target == f) {
                    Some(i) => STUBS.with(|s| s.borrow()[i]),
                    None => f,
                }
            })
        }
    
        fn srng() -> u32 {
            SRNG.with(|s| {
                let mut r = Rng(s.get());
                let v = r.u32();
                s.set(r.0);
                v
            })
        }
    
        fn get(r: &Pushad, args: *const u32, src: Src) -> u32 {
            match src {
                Src::Reg(b) => match b {
                    EAX => r.eax,
                    ECX => r.ecx,
                    EDX => r.edx,
                    ESI => r.esi,
                    _ => r.edi,
                },
                Src::Arg(k) => unsafe { *args.add(k) },
            }
        }
    
        extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
            let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
            let r = unsafe { &*regs };
            // Pointers into the stack differ between the two runs: log a marker for them.
            let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
            if !c.silent {
                let mut e = vec![c.target as u32];
                for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                    e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else { mask(v) });
                }
                for k in 0..c.nargs {
                    let v = unsafe { *args.add(k) };
                    e.push(if c.byte_args >> k & 1 != 0 { v & 0xFF } else { mask(v) });
                }
                match c.fx {
                    Fx::LogVec => {
                        for k in 0..(r.ecx.min(40) * 3) as usize {
                            e.push(mask(unsafe { *((r.edx as usize + k * 4) as *const u32) }));
                        }
                    }
                    Fx::LogEdx3 => {
                        for k in 0..3usize {
                            e.push(unsafe { *((r.edx as usize + k) as *const u8) } as u32);
                        }
                    }
                    Fx::LogEaxBytes(n) => {
                        for k in 0..n {
                            e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
                        }
                    }
                    Fx::LogArg0(n) => {
                        let p = unsafe { *args } as usize;
                        for k in 0..n {
                            e.push(mask(unsafe { *((p + k * 4) as *const u32) }));
                        }
                    }
                    _ => {}
                }
                for &(src, n) in &c.logs {
                    let p = get(r, args, src) as usize;
                    for k in 0..n {
                        e.push(if p == 0 { 0 } else { mask(unsafe { *((p + k * 4) as *const u32) }) });
                    }
                }
                if c.fx == Fx::LogBuf {
                    let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                    for k in 0..len {
                        let mut b = unsafe { *((p + k) as *const u8) };
                        if (len == 9 || len == 7) && k == 3 {
                            b = 0;
                        }
                        e.push(b as u32);
                    }
                }
                LOG.with(|l| l.borrow_mut().push(e));
            }
            match c.fx {
                Fx::WriteUint => {
                    let v = match srng() % 6 {
                        0 => 0,
                        1 => srng() % 10,
                        2 => srng() % 100,
                        3 => srng() % 5000,
                        4 => 0x7FFF_FFFF,
                        _ => srng(),
                    };
                    unsafe { *(super::t(r.esi as usize) as *mut u32) = v };
                }
                Fx::WriteFloat => {
                    let mut rng = Rng(srng() as u64 | 1 << 40);
                    unsafe { *(super::t(r.esi as usize) as *mut u32) = rng.float_bits() };
                }
                _ => {}
            }
            for &(src, out) in &c.outs {
                let p = get(r, args, src) as usize;
                if p == 0 {
                    continue;
                }
                unsafe {
                    match out {
                        Out::Byte01 => *(p as *mut u8) = (srng() & 1) as u8,
                        Out::Floats(n) => {
                            for k in 0..n {
                                let v = match srng() % 8 {
                                    0 => 0.0f32.to_bits(),
                                    1 => 1.0f32.to_bits(),
                                    2 => (-1.0f32).to_bits(),
                                    3..=5 => (((srng() % 2000) as f32) / 100.0 - 10.0).to_bits(),
                                    _ => Rng(srng() as u64 | 1 << 40).float_bits(),
                                };
                                *((p + k * 4) as *mut u32) = v;
                            }
                        }
                        Out::Dwords(n) => {
                            for k in 0..n {
                                *((p + k * 4) as *mut u32) = srng();
                            }
                        }
                        Out::CountBuf => {
                            let n = srng() % 9;
                            *(p as *mut u32) = n;
                            let buf = get(r, args, Src::Arg(1)) as usize;
                            for k in 0..n as usize {
                                *((buf + k * 4) as *mut u32) = (srng() % 40).wrapping_sub(3);
                            }
                        }
                    }
                }
            }
            match c.ret {
                Ret::Zero => 0,
                Ret::Mod(n) => srng() % n,
                Ret::Any => srng(),
                Ret::St0 => {
                    let v = match srng() % 8 {
                        0 => 0.0,
                        1 => f32::from_bits(srng()) as f64,
                        2 => srng() as f64 / 7.0,
                        3 => (srng() % 5000) as f64 / 10.0,
                        4 => f64::from_bits((srng() as u64) << 32 | srng() as u64),
                        5 => 1000.0,
                        _ => (srng() % 100) as f64,
                    };
                    unsafe { ST0V = v };
                    srng()
                }
                Ret::PtrNz(n) => POOLS.with(|p| {
                    let p = p.borrow();
                    let pool = &p[n];
                    pool[srng() as usize % pool.len()]
                }),
                Ret::Bool => match srng() % 6 {
                    0 | 1 => 0,
                    2 | 3 => 1,
                    4 => 0x100,
                    _ => srng(),
                },
                Ret::Ptr(n) => {
                    if srng() % 4 == 0 {
                        0
                    } else {
                        POOLS.with(|p| {
                            let p = p.borrow();
                            let pool = &p[n];
                            pool[srng() as usize % pool.len()]
                        })
                    }
                }
            }
        }
    
        static mut ST0V: f64 = 0.0;
    
        /// `stub_common` for the callees that return on ST0.
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
                "fld qword ptr [{v}]",
                "ret 4",
                f = sym stub_rs,
                v = sym ST0V,
            )
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
    
        #[repr(C)]
        #[derive(Clone, Copy, PartialEq, Debug)]
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
            args: [u32; 3],
            esp: u32,
            stack: u32,
        }
    
        /// Call `f` with all seven registers from `r` and three stack arguments, capture them after.
        pub unsafe fn call_orig(f: usize, r: &mut Regs, args: [u32; 3]) {
            let mut b = Block { f: f as u32, r: *r, args, esp: 0, stack: (STACK + STACK_LEN - 0x100) as u32 };
            unsafe {
                std::arch::asm!(
                    "push ebp", "push ebx", "push esi", "push edi",
                    "mov [eax + 44], esp",
                    "mov esp, [eax + 48]",
                    "push eax", "push eax", "push eax", "push eax",
                    "push dword ptr [eax]",
                    "push dword ptr [eax + 40]",
                    "push dword ptr [eax + 36]",
                    "push dword ptr [eax + 32]",
                    "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
                    "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
                    "mov eax, [eax + 4]",
                    "call dword ptr [esp + 12]",
                    "push ebx",
                    "mov ebx, [esp + 20]",
                    "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
                    "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
                    "pop dword ptr [ebx + 16]",
                    "mov esp, [ebx + 44]",
                    "pop edi", "pop esi", "pop ebx", "pop ebp",
                    inout("eax") &mut b as *mut Block => _,
                    lateout("ecx") _,
                    lateout("edx") _,
                    clobber_abi("C"),
                );
            }
            *r = b.r;
        }
    
        /// Reports a crash with the faulting address and where it happened (original copy offset).
        unsafe extern "system" fn veh(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
            unsafe {
                let rec = &*(*info).ExceptionRecord;
                let ctx = &*(*info).ContextRecord;
                let orig = ORIG_AT.with(|o| o.get());
                eprintln!(
                    "EXCEPTION {:#x} at eip {:#x} (original +{:#x}) access {:x?} eax {:#x} ecx {:#x} edx {:#x} ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}",
                    rec.ExceptionCode, ctx.Eip, (ctx.Eip as usize).wrapping_sub(orig), &rec.ExceptionInformation[..2], ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx, ctx.Esi, ctx.Edi, ctx.Ebp
                );
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
            0
        }
    
        thread_local! {
            static ORIG_AT: Cell<usize> = const { Cell::new(0) };
        }
    
        pub struct Harness {
            pub mem: Mem,
            /// The original body, relocated to a private copy with its calls aimed at the stubs.
            pub orig: usize,
            pub thunk: usize,
            /// Also compare EAX after the call.
            pub cmp_eax: bool,
        }
    
        /// Executable memory for the stubs and the patched copy.
        fn exec_alloc(len: usize) -> usize {
            let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
            assert!(p != 0);
            p
        }
    
        impl Harness {
            /// `base`: the original's address; `body` / `calls`: its bytes and the offsets of its `call
            /// rel32`s; `thunk`: the Rust replacement's entry.
            pub fn new(base: usize, body: &[u8], calls: &[usize], disps: &[usize], callees: Vec<Callee>, extra: &[(usize, usize)], thunk: usize) -> Harness {
                let mem = Mem::new(body, extra);
                fixed(STACK, STACK_LEN);
                let stubs = exec_alloc(callees.len() * 16 + 16);
                let mut addrs = Vec::new();
                for (i, c) in callees.iter().enumerate() {
                    let a = stubs + i * 16;
                    let mut code = [0x90u8; 16];
                    code[0] = 0x68;
                    code[1..5].copy_from_slice(&(i as u32).to_le_bytes());
                    code[5] = 0xE8;
                    let common = if c.ret == Ret::St0 { stub_common_st0 as usize } else { stub_common as usize };
                    let rel = (common as i32).wrapping_sub((a + 10) as i32);
                    code[6..10].copy_from_slice(&rel.to_le_bytes());
                    if c.pop == 0 {
                        code[10] = 0xC3;
                    } else {
                        code[10] = 0xC2;
                        code[11..13].copy_from_slice(&(c.pop as u16).to_le_bytes());
                    }
                    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), a as *mut u8, 16) };
                    addrs.push(a);
                }
                CALLEES.with(|c| *c.borrow_mut() = callees.clone());
                STUBS.with(|s| *s.borrow_mut() = addrs.clone());
                let orig = exec_alloc(body.len() + 16);
                unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), orig as *mut u8, body.len()) };
                for &off in disps {
                    let v = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()).wrapping_add(super::DELTA as u32);
                    unsafe { std::ptr::write_unaligned((orig + off) as *mut u32, v) };
                }
                for &off in calls {
                    let rel = i32::from_le_bytes(body[off + 1..off + 5].try_into().unwrap());
                    let target = (base + off + 5).wrapping_add(rel as usize);
                    let i = callees.iter().position(|c| c.target == target).unwrap_or_else(|| panic!("call at {off:#x} to {target:#x} has no callee entry"));
                    let dest = if callees[i].native != 0 { callees[i].native } else { addrs[i] };
                    let r = (dest as i32).wrapping_sub((orig + off + 5) as i32);
                    unsafe { std::ptr::write_unaligned((orig + off + 1) as *mut i32, r) };
                }
                ORIG_AT.with(|o| o.set(orig));
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| unsafe {
                    windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(veh));
                });
                Harness { mem, orig, thunk, cmp_eax: false }
            }
    
            fn run(&self, f: usize, ecx: u32, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
                SRNG.with(|s| s.set(seed | 1));
                LOG.with(|l| l.borrow_mut().clear());
                let before = Regs { eax: 0xAAAA_0001, ecx, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 };
                let mut r = before;
                unsafe { call_orig(f, &mut r, args) };
                (LOG.with(|l| std::mem::take(&mut *l.borrow_mut())), before, r)
            }
    
            /// One case on the current memory: original, then Rust, from the same snapshot.
            pub fn check(&self, ecx: u32, args: [u32; 3], seed: u64) -> Result<(), String> {
                let snap = self.mem.snapshot();
                let (la, ba, ra) = self.run(self.orig, ecx, args, seed);
                let after_a = self.mem.snapshot();
                self.mem.restore(&snap);
                let (lb, _bb, rb) = self.run(self.thunk, ecx, args, seed);
                let after_b = self.mem.snapshot();
                if self.cmp_eax && ra.eax != rb.eax {
                    return Err(format!("eax: original {:#x} rust {:#x}", ra.eax, rb.eax));
                }
                // callee-saved registers must come back unchanged
                for (name, x, y, z) in [("ebx", ba.ebx, ra.ebx, rb.ebx), ("esi", ba.esi, ra.esi, rb.esi), ("edi", ba.edi, ra.edi, rb.edi), ("ebp", ba.ebp, ra.ebp, rb.ebp)] {
                    if x != y || x != z {
                        return Err(format!("register {name}: in {x:#x} original {y:#x} rust {z:#x}"));
                    }
                }
                for (i, (a, b)) in la.iter().zip(&lb).enumerate() {
                    if a != b {
                        let prev: Vec<&Vec<u32>> = la[i.saturating_sub(3)..i].iter().collect();
                        return Err(format!("call #{i} differs:
      previous {:x?}
      original {:x?}
      rust     {:x?}", prev, a, b));
                    }
                }
                if la.len() != lb.len() {
                    let longer = if la.len() > lb.len() { ("original", &la) } else { ("rust", &lb) };
                    return Err(format!("original made {} calls, rust {}; the {} next calls {:x?}", la.len(), lb.len(), longer.0, longer.1[la.len().min(lb.len())]));
                }
                if let Some((a, x, y)) = self.mem.first_diff(&after_a, &after_b) {
                    return Err(format!("memory differs at {a:#x}: original {x:#x} rust {y:#x} ({} bytes differ)", self.mem.count_diffs(&after_a, &after_b)));
                }
                Ok(())
            }
        }
    
        pub fn lock() -> std::sync::MutexGuard<'static, ()> {
            static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
            L.lock().unwrap_or_else(|e| e.into_inner())
        }
    
        /// Where the test world's objects live.
        pub const ARENA: usize = 0x6000_0000;
        /// The tests run on a stack of their own, outside the game's address range.
        pub const STACK: usize = 0x6100_0000;
        pub const STACK_LEN: usize = 0x10_0000;
        pub const ARENA_LEN: usize = 0x18_0000;
    
        /// A bump allocator over the arena for one case.
        pub struct Arena(pub usize);
        impl Arena {
            pub fn new() -> Arena {
                Arena(ARENA)
            }
            /// `n` objects of `size` bytes (random contents are the caller's business).
            pub fn pool(&mut self, n: usize, size: usize) -> Vec<u32> {
                let v = (0..n).map(|i| (self.0 + i * size) as u32).collect();
                self.0 += n * size;
                assert!(self.0 <= ARENA + ARENA_LEN, "arena full");
                v
            }
        }
    
        pub unsafe fn put(a: u32, off: usize, v: u32) {
            unsafe { *((a as usize + off) as *mut u32) = v }
        }
        pub unsafe fn putb(a: u32, off: usize, v: u8) {
            unsafe { *((a as usize + off) as *mut u8) = v }
        }
    }
    

    #[cfg(test)]
    pub(crate) mod pc_test {
        use super::dt::*;
        use super::*;

        const VT34: usize = 0xCB00_0034;
        const VT10: usize = 0xCB00_0010;
        const VT04: usize = 0xCB00_0004;
        const ENTER: usize = 0xE100_0001;
        const LEAVE: usize = 0xE100_0002;
        const EVENT: usize = 0xE100_0003;

        fn c(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret) -> Callee {
            callee(target, nargs, pop, reads, ret, Fx::None)
        }

        pub(crate) fn callees() -> Vec<Callee> {
            let mut v = Vec::new();
            let mut x = c(0x6beed0, 0, 0, EAX | ECX, Ret::Any);
            x.outs.push((Src::Reg(EAX), Out::Floats(3)));
            v.push(x);
            let mut x = c(0xa52bb0, 0, 0, EAX | ECX, Ret::Ptr(0));
            x.outs.push((Src::Reg(EAX), Out::Byte01));
            v.push(x);
            v.push(c(0xa275f0, 0, 0, EAX, Ret::Bool));
            v.push(c(0xa27660, 1, 0, 0, Ret::Ptr(0)));
            v.push(c(0xa618a0, 0, 0, EAX, Ret::Ptr(1)));
            v.push(c(VT34, 0, 0, ECX, Ret::Ptr(0)));
            v.push(c(VT10, 0, 0, ECX, Ret::Any));
            v.push(c(VT04, 1, 4, ECX, Ret::Any));
            v.push(c(0x40ea20, 1, 0, 0, Ret::St0));
            let mut x = c(0xbddff0, 0, 0, ESI, Ret::Any);
            x.logs.push((Src::Reg(ESI), 3));
            x.outs.push((Src::Reg(ESI), Out::Floats(3)));
            v.push(x);
            let mut x = c(0xbe4b20, 0, 0, EAX | ECX, Ret::Any);
            x.logs.push((Src::Reg(EAX), 3));
            x.outs.push((Src::Reg(ECX), Out::Dwords(9)));
            v.push(x);
            let mut x = c(0xbe51d0, 2, 8, 0, Ret::Any);
            x.logs.push((Src::Arg(0), 9));
            v.push(x);
            let mut x = c(0x6c78b0, 2, 0, ECX, Ret::Any);
            x.logs.push((Src::Reg(ECX), 3));
            x.outs.push((Src::Reg(ECX), Out::Floats(3)));
            v.push(x);
            let mut x = c(0x6c7b90, 2, 0, 0, Ret::Any);
            x.logs.push((Src::Arg(0), 9));
            x.outs.push((Src::Arg(0), Out::Dwords(9)));
            v.push(x);
            let mut x = c(0x9a0b00, 5, 0, ECX | EDX, Ret::Any);
            x.logs.push((Src::Reg(ECX), 9));
            x.logs.push((Src::Reg(EDX), 3));
            v.push(x);
            v.push(c(0x55a930, 4, 16, 0, Ret::Any));
            v.push(c(0x95be10, 3, 12, ESI, Ret::Any));
            v.push(c(0x926860, 3, 0, ECX, Ret::Any));
            v.push(c(0x9a5ad0, 1, 0, 0, Ret::Any));
            v.push(c(0x6e4de0, 0, 0, EAX, Ret::Any));
            v.push(c(0x6e4e10, 0, 0, EAX, Ret::Any));
            v.push(c(0xcebb60, 0, 0, ECX, Ret::Any));
            v.push(c(0x6c2190, 2, 0, EDI, Ret::Any));
            v.push(c(0x6c2050, 3, 0, 0, Ret::Any));
            v.push(c(0x6c22c0, 2, 0, ESI, Ret::Any));
            v.push(c(0x6c2340, 2, 0, EDI, Ret::Any));
            let mut x = c(0xcec6c0, 2, 0, ECX, Ret::Any);
            x.outs.push((Src::Arg(0), Out::CountBuf));
            v.push(x);
            v.push(c(0xce7f20, 2, 8, ECX | EDX, Ret::Any));
            v.push(c(0xcec840, 0, 0, ECX, Ret::Any));
            v.push(c(0x9a68d0, 0, 0, EAX, Ret::Any));
            v.push(c(0x9a7150, 0, 0, 0, Ret::Any));
            v.push(c(0x9a9010, 0, 0, EAX, Ret::Any));
            v.push(c(0xca0bc0, 3, 0, 0, Ret::Any));
            v.push(c(0xbdca30, 1, 4, EAX | EDX, Ret::PtrNz(2)));
            v.push(c(0xcedf60, 0, 0, ESI, Ret::Any));
            v.push(c(ENTER, 1, 4, 0, Ret::Any));
            v.push(c(LEAVE, 1, 4, 0, Ret::Any));
            v.push(c(EVENT, 1, 4, 0, Ret::Any));
            v
        }

        fn fl(rng: &mut Rng) -> u32 {
            match rng.below(8) {
                0 => 0,
                1 => 1.0f32.to_bits(),
                2 => (-2.5f32).to_bits(),
                3 => (rng.below(2000) as f32 / 10.0 - 100.0).to_bits(),
                4 => 3.0e38f32.to_bits(),
                _ => (rng.below(2000) as f32 / 1000.0 - 1.0).to_bits(),
            }
        }

        fn flany(rng: &mut Rng) -> u32 {
            rng.float_bits()
        }

        fn f64_at(a: usize, v: f64) {
            let b = v.to_bits();
            unsafe {
                wd(a, b as u32);
                wd(a + 4, (b >> 32) as u32);
            }
        }

        /// Builds one random world; returns the shot record.
        unsafe fn world(h: &Harness, rng: &mut Rng) -> u32 {
            unsafe {
                h.mem.fill_random(rng);
                h.mem.zero(HANDLE_TABLE, 0x1_0000);
                wd(0xdd0584, 0.01f32.to_bits());
                wd(0xdd1b58, 0.75f32.to_bits());
                f64_at(0xe5bfc8, 0.25);
                f64_at(0xe5c2a0, 57.29582977294922);
                let mut a = Arena::new();
                let vt = a.pool(1, 0x80)[0];
                put(vt, 13 * 4, redirect(VT34) as u32);
                put(vt, 4 * 4, redirect(VT10) as u32);
                put(vt, 4, redirect(VT04) as u32);
                let humans = a.pool(5, 0x1400);
                let targets = a.pool(5, 0x1400);
                let p4s = a.pool(10, 0x300);
                let ts = a.pool(10, 0x40);
                let d4 = a.pool(5, 0x20);
                let d8 = a.pool(5, 0x80);
                let scripts = a.pool(6, 0x20);
                let nodes = a.pool(4, 0x200);
                let cells = a.pool(6, 8);
                let strs = a.pool(8, 0x10);
                let rec = a.pool(1, 0x200)[0];
                let ents = a.pool(1, 0x7c * 5 + 16)[0];
                let wptr = a.pool(4, 0x80);
                let w2 = a.pool(4, 0x40);
                let player = a.pool(1, 0x1000)[0];
                let local = a.pool(1, 0x1200)[0];
                let obj236 = a.pool(1, 0x80)[0];
                set_pools(vec![humans.clone(), scripts.clone(), cells.clone()]);

                for &s in &scripts {
                    put(s, 0, vt);
                }
                // crc list
                let keys: Vec<u32> = (0..4).map(|_| rng.u32()).collect();
                for (k, &n) in nodes.iter().enumerate() {
                    put(n, 0, vt);
                    put(n, 0xc, keys[k]);
                    put(n, 0x17c, if rng.one_in(8) { 0 } else { nodes[(k + 1) % 4] });
                }
                for &cl in &cells {
                    put(cl, 0, if rng.one_in(3) { rng.u32() } else { keys[rng.below(4) as usize] });
                }
                wd(0x2f25254, if rng.one_in(6) { 0 } else { nodes[rng.below(4) as usize] });

                for i in 0..5 {
                    let o = humans[i];
                    let h = if rng.one_in(8) { 0 } else { (1 + rng.below(0xFF)) << 12 | (10 + i as u32 * 5 + rng.below(5)) };
                    put(o, 0x44, h);
                    put(o, 0x4c, rng.pick(&[1, 1, 1, 5]));
                    putb(o, 0x3cc, rng.pick(&[1, 2, 1, 2, 0]));
                    let (fa, fb) = (rng.float_bits(), rng.float_bits());
                    put(o, 0x3d0, rng.pick(&[0, 0.5f32.to_bits(), 0.75f32.to_bits(), 0.8f32.to_bits(), 1.0f32.to_bits(), 0.749_999_9f32.to_bits(), fa, fb]));
                    put(o, 0xd4, d4[i]);
                    put(d4[i], 8, d8[i]);
                    putb(d8[i], 0x60, rng.u32() as u8);
                    put(o, 0x5e0, p4s[i]);
                    put(p4s[i], 0, rng.pick(&[0xFFFF_FFFF, 0xFFFF_FFFE, 0, 1, 189, 190, 191, 5000, 0x7FFF_FFFF, 0x8000_0000, 7, 33]));
                    putb(p4s[i], 0x26c, 1);
                    put(p4s[i], 0x268, ts[i]);
                    let b = targets[i];
                    put(b, 0x4c, rng.pick(&[1, 1, 1, 5]));
                    putb(b, 0x3cc, rng.pick(&[1, 2, 1, 2, 0]));
                    put(b, 0x5e0, p4s[5 + i]);
                    putb(p4s[5 + i], 0x26c, rng.pick(&[1, 1, 0, 2]));
                    put(p4s[5 + i], 0x268, ts[5 + i]);
                    if h != 0 {
                        let slot = (h & 0xFFF) as usize;
                        match rng.below(10) {
                            0..=4 => {
                                put(b, 0x44, h);
                                wd(HANDLE_TABLE + slot * 16, b);
                            }
                            5 => {
                                wd(HANDLE_TABLE + slot * 16, o);
                            }
                            6 => {
                                put(b, 0x44, h ^ 0x1000);
                                wd(HANDLE_TABLE + slot * 16, b);
                            }
                            7 => {
                                put(b, 0x44, h);
                                // table holds another handle's object
                                wd(HANDLE_TABLE + slot * 16, targets[(i + 1) % 5]);
                            }
                            _ => {}
                        }
                    }
                }
                // weapon-ish structs
                for &w in wptr.iter().chain(w2.iter()) {
                    put(w, 0x3c, rng.u32());
                    put(w, 0x14, rng.pick(&[0, 1, 5]));
                }
                // the record and its entries
                let n: i32 = rng.pick(&[-1, 0, 1, 2, 2, 3, 4, 1, 4]);
                put(rec, 0x108, n as u32);
                put(rec, 0x100, ents);
                for k in 0..5usize {
                    let e = ents + (k * 0x7c) as u32;
                    put(e, 0, if rng.one_in(8) { 0 } else { strs[rng.below(8) as usize] });
                    putb(e, 0x37, rng.pick(&[0, 0, 0, 1]));
                    put(e, 4, fl(rng));
                    put(e, 8, fl(rng));
                    put(e, 0xc, fl(rng));
                    put(e, 0x10, flany(rng));
                    put(e, 0x14, if rng.one_in(3) { 0 } else { wptr[rng.below(4) as usize] });
                    put(e, 0x18, rng.u32());
                    put(e, 0x1c, if rng.one_in(3) { 0 } else { w2[rng.below(4) as usize] });
                    put(e, 0x20, rng.u32());
                    put(e, 0x24, rng.u32());
                    putb(e, 0x28, rng.pick(&[0, 1, 1, 2]));
                    for off in [0x29usize, 0x2a, 0x2b] {
                        let rb8 = rng.u32() as u8;
                        putb(e, off, rng.pick(&[0, 0, 1, 2, 255, rb8]));
                    }
                    putb(e, 0x2c, rng.pick(&[0, 0, 1]));
                    put(e, 0x30, flany(rng));
                    putb(e, 0x34, rng.u32() as u8);
                    putb(e, 0x35, rng.pick(&[0, 1, 1, 3]));
                    putb(e, 0x38, rng.pick(&[0, 0, 1]));
                    putb(e, 0x39, rng.pick(&[0, 1, 1, 2]));
                    putb(e, 0x3a, rng.pick(&[0, 1, 1]));
                    put(e, 0x3c, rng.u32());
                }
                wd(PLAYER, player);
                putb(player, 0xafd, rng.pick(&[0, 1, 1]));
                wd(LOCAL_PLAYER, local);
                put(local, 0x108c, flany(rng));
                for off in [0usize, 4, 8] {
                    wd(0x252a6d0 + off, rng.u32());
                }
                wd(0x252a634, rng.pick(&[0, 1, 2, 3, 3, 0xFFFF_FFFF, 5]));
                wd(0x236f9fc, if rng.one_in(4) { 0 } else { obj236 });
                put(obj236, 0, vt);
                wd(0xec5b98, vt);
                wd(0x33d6428, rng.pick(&[2, 2, 2, 3, 0, 5]));
                wd(0x33d64f8, rng.pick(&[2, 2, 2, 3, 0, 5]));
                wd(0xdb80cc, redirect(ENTER) as u32);
                wd(0xdb80d0, redirect(LEAVE) as u32);
                wd(0xdb81dc, redirect(EVENT) as u32);
                rec
            }
        }

        #[test]
        #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
        fn place_characters_matches_original() {
            let _g = lock();
            let extras = [(HANDLE_TABLE, 0x1_0000), (0x33d6400, 0x200), (0x340d240, 190 * 0x5e8)];
            let h = Harness::new(PC_ADDR, &PC_BODY, &PC_CALLS_AT, &PC_DISPS, callees(), &extras, place_characters_t as *const () as usize);
            let mut rng = Rng(0x1234_5678_9ABC_DEF1);
            let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
            let mut bad = 0;
            for i in 0..cases {
                let rec = unsafe { world(&h, &mut rng) };
                if let Err(e) = h.check(0xBADC_0DE0, [rec, 0, 0], rng.u32() as u64) {
                    bad += 1;
                    if bad <= 5 {
                        eprintln!("case {i}: {e}");
                    }
                }
            }
            eprintln!("{}", summary());
            assert_eq!(bad, 0, "{bad} of {cases} cases differ");
        }
    }
}
