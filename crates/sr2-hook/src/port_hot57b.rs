//! Port module: hot game-thread / render-thread math leaves (docs 57 and 58).
//!
//! `transform_compose_to_esi` (0x719FF0), `nav_tri_bounds_xz` (0x501850),
//! `world_mesh_record_set_lod_list` (0xD098B0), `occluder_encloses_shape_points` (0xA024C0),
//! `vec3_normalize_in_place_ret_len` (0xBDE060), `segment_point_distance_sq` (0xBE0250),
//! `aabb_contains_point` (0xBE9830), `mat33_mul_vec3_in_place` (0xBDE500),
//! `lower_bound_key64_rec16` (0xBD5C30) and `view_distance_sq_scaled` (0x4B7650).
//!
//! The originals are x87: every `fld dword` / `fstp dword` pair is an f64 operation with an `as f32`
//! at the store (the game runs the x87 at 53-bit precision, so f64 arithmetic is bit-exact); the
//! comparisons keep the original's polarity (see the notes at each function).
//!
//! Every port runs behind one `pushad` thunk that also saves XMM1-7 (an LTCG caller may keep a
//! value there across a call to a leaf that never touches them) and puts the result in EAX, ST0
//! (a Rust `-> f32` return) or XMM0 (`vec3_normalize_in_place_ret_len`, which carries the length
//! in the unused `esp` slot of the pushad block).
//!
//! Checks:
//! - `occluder_encloses_shape_points`, `aabb_contains_point`, `lower_bound_key64_rec16`,
//!   `view_distance_sq_scaled`: pure, relocated-original shadow for the first `SHADOW_CALLS` calls
//!   (result compared, the original's wins on a mismatch).
//! - `transform_compose_to_esi`, `nav_tri_bounds_xz`, `world_mesh_record_set_lod_list`,
//!   `vec3_normalize_in_place_ret_len`, `segment_point_distance_sq`, `mat33_mul_vec3_in_place`:
//!   snapshot shadow (the written bytes saved, the original run, restored, the Rust version run,
//!   compared; the original's writes win on a mismatch).
//! - After the window every port is the bare Rust body: no logging, no allocation, no atomics.
//! - Offline: `mod tests` runs the original bytes (relocated, constants and callees redirected to
//!   test copies) against the real thunk on random worlds (see the comment above `mod tests`).

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- game addresses and constants ----------------------------------------------------------------

const MUL_VEC_MAT: usize = 0x0040_6810; // ECX = mat33, EDX = vec3, stack (out) -> EAX = out
const MAT_MUL: usize = 0x00BE_2F50; // EAX = out mat33, ECX = A, stack (B), ret 4 -> EAX = out
const SQRTF: usize = 0x0040_EA20; // cdecl (float) -> ST0
const VEC3_DIST_SQ: usize = 0x00BD_EA30; // EAX, ECX = vec3* -> ST0
const THREAD_STATE: usize = 0x00D2_2790; // no inputs, result unused
const SHAPE_TABLE: usize = 0x00E8_2680; // 0x80 bytes per entry, point count at +4
const VIEW_FLAG: usize = 0x0252_73B6; // byte

/// The double at 0xE5C460: the degenerate-segment threshold.
fn seg_threshold() -> f64 {
    f64::from_bits(0x3DDB_7CDF_E000_0000)
}

/// The double at 0xE5CF28 (0.5925...).
fn view_scale() -> f64 {
    f64::from_bits(0x3FE2_F5E6_E000_0000)
}

#[cfg(test)]
pub(crate) static mut T_TBL: [u32; 16 * 32] = [0; 16 * 32];
#[cfg(test)]
pub(crate) static mut T_FLAG: u32 = 0;
#[cfg(test)]
pub(crate) static mut T_THR: u64 = 0x3DDB_7CDF_E000_0000;
#[cfg(test)]
pub(crate) static mut T_ONE: u32 = 0x3F80_0000;
#[cfg(test)]
pub(crate) static mut T_K: u64 = 0x3FE2_F5E6_E000_0000;
#[cfg(test)]
pub(crate) static T_REDIR: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

fn shape_table() -> usize {
    #[cfg(test)]
    {
        std::ptr::addr_of!(T_TBL) as usize
    }
    #[cfg(not(test))]
    {
        SHAPE_TABLE
    }
}

fn view_flag() -> usize {
    #[cfg(test)]
    {
        std::ptr::addr_of!(T_FLAG) as usize
    }
    #[cfg(not(test))]
    {
        VIEW_FLAG
    }
}

/// The callee's address (the offline test aims the game's functions at test copies).
#[inline(always)]
fn tgt(a: usize) -> usize {
    #[cfg(test)]
    {
        T_REDIR.lock().unwrap().iter().find(|e| e.0 == a).map(|e| e.1).unwrap_or(a)
    }
    #[cfg(not(test))]
    {
        a
    }
}

unsafe fn cr(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { call_regs(tgt(f), eax, ecx, edx, esi, edi, args) }
}

/// Call a game function that returns a double on ST0 (popped).
unsafe fn call_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    let f = tgt(f);
    let mut block: [u32; 10] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, 0, 0];
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
            "fstp qword ptr [ebx + 32]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_mut_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    f64::from_bits(block[8] as u64 | (block[9] as u64) << 32)
}

/// What `fld dword` / `fstp dword` does to a signalling NaN.
#[inline(always)]
fn q(b: u32) -> u32 {
    if b & 0x7F80_0000 == 0x7F80_0000 && b & 0x007F_FFFF != 0 { b | 0x0040_0000 } else { b }
}

#[inline(always)]
unsafe fn fx(p: usize) -> f64 {
    unsafe { f32::from_bits(rd(p)) as f64 }
}

// --- registers, thunks, shadow machinery ---------------------------------------------------------

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)] // not every register is read
struct Pushad {
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// `pushad`, XMM1-7 saved, `f(&mut block, &stack args)`, XMM1-7 restored, `popad`.
macro_rules! hot_thunk {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 112",
                "movups xmmword ptr [esp], xmm1",
                "movups xmmword ptr [esp + 16], xmm2",
                "movups xmmword ptr [esp + 32], xmm3",
                "movups xmmword ptr [esp + 48], xmm4",
                "movups xmmword ptr [esp + 64], xmm5",
                "movups xmmword ptr [esp + 80], xmm6",
                "movups xmmword ptr [esp + 96], xmm7",
                "lea eax, [esp + 112]",
                "lea ecx, [esp + 148]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm1, xmmword ptr [esp]",
                "movups xmm2, xmmword ptr [esp + 16]",
                "movups xmm3, xmmword ptr [esp + 32]",
                "movups xmm4, xmmword ptr [esp + 48]",
                "movups xmm5, xmmword ptr [esp + 64]",
                "movups xmm6, xmmword ptr [esp + 80]",
                "movups xmm7, xmmword ptr [esp + 96]",
                "add esp, 112",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// Same, and XMM0 = the dword in the block's `esp` slot (the function's float result).
macro_rules! hot_thunk_x0 {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 112",
                "movups xmmword ptr [esp], xmm1",
                "movups xmmword ptr [esp + 16], xmm2",
                "movups xmmword ptr [esp + 32], xmm3",
                "movups xmmword ptr [esp + 48], xmm4",
                "movups xmmword ptr [esp + 64], xmm5",
                "movups xmmword ptr [esp + 80], xmm6",
                "movups xmmword ptr [esp + 96], xmm7",
                "lea eax, [esp + 112]",
                "lea ecx, [esp + 148]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm1, xmmword ptr [esp]",
                "movups xmm2, xmmword ptr [esp + 16]",
                "movups xmm3, xmmword ptr [esp + 32]",
                "movups xmm4, xmmword ptr [esp + 48]",
                "movups xmm5, xmmword ptr [esp + 64]",
                "movups xmm6, xmmword ptr [esp + 80]",
                "movups xmm7, xmmword ptr [esp + 96]",
                "movd xmm0, dword ptr [esp + 124]",
                "add esp, 112",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// One function's outputs, as far as callers read them.
#[derive(Clone, Copy, Default)]
struct Res {
    eax: u32,
    xmm0: u32,
    st0: u32, // f32 bits
}

const M_EAX: u8 = 1;
const M_XMM0: u8 = 2;
const M_ST0: u8 = 4;

#[repr(C)]
#[allow(dead_code)] // the asm reads and writes the fields through the pointer
struct OBlk {
    f: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    esi: u32,
    edi: u32,
    args: u32,
    nargs: u32,
    o_eax: u32,
    o_ecx: u32,
    o_edx: u32,
    top0: u32,
    top1: u32,
    xmm0: u32,
    st0: u64,
}

/// Run the relocated original with the caller's registers and stack arguments; captures EAX, XMM0
/// and a result left on ST0 (popped again, so the x87 depth is what it was).
unsafe fn call_orig(f: usize, r: &Pushad, args: &[u32]) -> (Res, u32) {
    let mut b = OBlk {
        f: f as u32, eax: r.eax, ecx: r.ecx, edx: r.edx, esi: r.esi, edi: r.edi,
        args: args.as_ptr() as u32, nargs: args.len() as u32,
        o_eax: 0, o_ecx: 0, o_edx: 0, top0: 0, top1: 0, xmm0: 0, st0: 0,
    };
    unsafe {
        std::arch::asm!(
            "push ebx", "push ebp", "push esi", "push edi",
            "mov ebx, eax",
            "mov ebp, esp",
            "mov ecx, [ebx + 28]",
            "mov esi, [ebx + 24]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "fnstsw word ptr [ebx + 44]",
            "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 16]", "mov edi, [ebx + 20]",
            "mov eax, [ebx + 4]",
            "call dword ptr [ebx]",
            "mov [ebx + 32], eax", "mov [ebx + 36], ecx", "mov [ebx + 40], edx",
            "movd dword ptr [ebx + 52], xmm0",
            "fnstsw word ptr [ebx + 48]",
            "movzx eax, word ptr [ebx + 48]",
            "and eax, 0x3800",
            "movzx ecx, word ptr [ebx + 44]",
            "and ecx, 0x3800",
            "cmp eax, ecx",
            "je 5f",
            "fstp qword ptr [ebx + 56]",
            "5:",
            "mov esp, ebp",
            "pop edi", "pop esi", "pop ebp", "pop ebx",
            inout("eax") &mut b as *mut OBlk => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    let depth = ((b.top0 & 0x3800).wrapping_sub(b.top1 & 0x3800) & 0x3800) >> 11;
    (Res { eax: b.o_eax, xmm0: b.xmm0, st0: (f64::from_bits(b.st0) as f32).to_bits() }, depth)
}

struct Port {
    name: &'static str,
    addr: usize,
    calls: AtomicU32,
    mismatches: AtomicU32,
    /// A relocated copy of the original (every port here is shadowed).
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize) -> Port {
    Port { name, addr, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const XFORM: usize = 0;
const TRI: usize = 1;
const LOD: usize = 2;
const OCCL: usize = 3;
const NORM: usize = 4;
const SEGD: usize = 5;
const AABB: usize = 6;
const MATV: usize = 7;
const LOWB: usize = 8;
const VDIST: usize = 9;
const COUNT: usize = 10;

static PORTS: [Port; COUNT] = [
    port("transform_compose_to_esi", 0x0071_9FF0),
    port("nav_tri_bounds_xz", 0x0050_1850),
    port("world_mesh_record_set_lod_list", 0x00D0_98B0),
    port("occluder_encloses_shape_points", 0x00A0_24C0),
    port("vec3_normalize_in_place_ret_len", 0x00BD_E060),
    port("segment_point_distance_sq", 0x00BE_0250),
    port("aabb_contains_point", 0x00BE_9830),
    port("mat33_mul_vec3_in_place", 0x00BD_E500),
    port("lower_bound_key64_rec16", 0x00BD_5C30),
    port("view_distance_sq_scaled", 0x004B_7650),
];

const SNAP_MAX: usize = 320;

unsafe fn snap_save(ranges: &[(usize, usize)], buf: &mut [u8; SNAP_MAX]) {
    let mut o = 0;
    for &(a, l) in ranges {
        unsafe { std::ptr::copy_nonoverlapping(a as *const u8, buf.as_mut_ptr().add(o), l) };
        o += l;
    }
}

unsafe fn snap_restore(ranges: &[(usize, usize)], buf: &[u8; SNAP_MAX]) {
    let mut o = 0;
    for &(a, l) in ranges {
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr().add(o), a as *mut u8, l) };
        o += l;
    }
}

#[cold]
#[inline(never)]
unsafe fn shadow_slow(id: usize, r: &Pushad, args: &[u32], ranges: &[(usize, usize)], mask: u8, rust: &mut dyn FnMut() -> Res) -> Res {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n + 1 == SHADOW_CALLS {
        log(&format!("port_hot57b: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    let total: usize = ranges.iter().map(|r| r.1).sum();
    if orig == 0 || total > SNAP_MAX {
        return rust();
    }
    let (mut s0, mut s1, mut s2) = ([0u8; SNAP_MAX], [0u8; SNAP_MAX], [0u8; SNAP_MAX]);
    unsafe {
        snap_save(ranges, &mut s0);
        let (o, depth) = call_orig(orig, r, args);
        snap_save(ranges, &mut s1);
        snap_restore(ranges, &s0);
        let ours = rust();
        snap_save(ranges, &mut s2);
        let mut bad = String::new();
        if mask & M_EAX != 0 && o.eax != ours.eax {
            bad += &format!(" eax original {:#x} Rust {:#x};", o.eax, ours.eax);
        }
        if mask & M_XMM0 != 0 && o.xmm0 != ours.xmm0 {
            bad += &format!(" xmm0 original {:#x} Rust {:#x};", o.xmm0, ours.xmm0);
        }
        if mask & M_ST0 != 0 && o.st0 != ours.st0 {
            bad += &format!(" st0 original {:#x} Rust {:#x};", o.st0, ours.st0);
        }
        if depth != (mask & M_ST0 != 0) as u32 {
            bad += &format!(" x87 depth {depth};");
        }
        if s1[..total] != s2[..total] {
            bad += " memory differs;";
        }
        if bad.is_empty() {
            return ours;
        }
        snap_restore(ranges, &s1);
        let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
        if m <= 10 {
            log(&format!("port_hot57b: {} mismatch (call {n}): regs eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} args {:x?}:{bad}", p.name, r.eax, r.ecx, r.edx, r.esi, r.edi, args));
        }
        o
    }
}

/// The shadow window, then the bare Rust body.
#[inline(always)]
unsafe fn shadowed<F: FnMut() -> Res>(id: usize, r: &Pushad, args: &[u32], ranges: &[(usize, usize)], mask: u8, mut rust: F) -> Res {
    if PORTS[id].calls.load(Ordering::Relaxed) >= SHADOW_CALLS {
        return rust();
    }
    unsafe { shadow_slow(id, r, args, ranges, mask, &mut rust) }
}

// --- the originals' bytes (python tools/body_bytes.py, explicit ends) -------------------------------

const XFORM_BODY: [u8; 148] = [
    0x83, 0xEC, 0x3C, 0x53, 0x8B, 0x5C, 0x24, 0x44, 0x8D, 0x44, 0x24, 0x10, 0x8D, 0x53, 0x24, 0x50,
    0x8B, 0xCF, 0xE8, 0x09, 0xC8, 0xCE, 0xFF, 0xD9, 0x00, 0xD8, 0x47, 0x24, 0x83, 0xC4, 0x04, 0x53,
    0x8B, 0xCF, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x47, 0x28, 0xD8, 0x40, 0x04, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x47, 0x2C, 0xD8, 0x40, 0x08, 0x8D, 0x44, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x10, 0xE8, 0x1D,
    0x8F, 0x4C, 0x00, 0xD9, 0x00, 0x5B, 0xD9, 0x1E, 0xD9, 0x40, 0x04, 0xD9, 0x5E, 0x04, 0xD9, 0x40,
    0x08, 0xD9, 0x5E, 0x08, 0xD9, 0x40, 0x0C, 0xD9, 0x5E, 0x0C, 0xD9, 0x40, 0x10, 0xD9, 0x5E, 0x10,
    0xD9, 0x40, 0x14, 0xD9, 0x5E, 0x14, 0xD9, 0x40, 0x18, 0xD9, 0x5E, 0x18, 0xD9, 0x40, 0x1C, 0xD9,
    0x5E, 0x1C, 0xD9, 0x40, 0x20, 0x8B, 0xC6, 0xD9, 0x5E, 0x20, 0xD9, 0x04, 0x24, 0xD9, 0x5E, 0x24,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0x5E, 0x28, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x5E, 0x2C, 0x83, 0xC4,
    0x3C, 0xC2, 0x04, 0x00,
];
const XFORM_CALLS: [usize; 2] = [0x12, 0x3e];
const TRI_BODY: [u8; 279] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x8B, 0x73, 0x04, 0x57, 0x8B, 0x3B, 0xD9, 0x07, 0xD9, 0x19,
    0xD9, 0x47, 0x08, 0xD9, 0x59, 0x04, 0xD9, 0x01, 0xD9, 0x06, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4,
    0x41, 0x75, 0x04, 0xD9, 0x01, 0xEB, 0x02, 0xD9, 0x06, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24,
    0x10, 0xD9, 0x11, 0xD9, 0x41, 0x04, 0xD9, 0x46, 0x08, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41,
    0x75, 0x05, 0xD9, 0x41, 0x04, 0xEB, 0x03, 0xD9, 0x46, 0x08, 0x8B, 0x5B, 0x08, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x51, 0x04, 0xD9, 0x03, 0xD8, 0xDA, 0xDF, 0xE0, 0xF6, 0xC4,
    0x41, 0x75, 0x04, 0xD9, 0xC9, 0xEB, 0x04, 0xDD, 0xD9, 0xD9, 0x03, 0xD9, 0x5C, 0x24, 0x10, 0xD9,
    0x44, 0x24, 0x10, 0xD9, 0x19, 0xD9, 0x43, 0x08, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x74,
    0x05, 0xDD, 0xD8, 0xD9, 0x43, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x59,
    0x04, 0xD9, 0x07, 0xD9, 0x1A, 0xD9, 0x47, 0x08, 0xD9, 0x5A, 0x04, 0xD9, 0x02, 0xD9, 0x06, 0xDE,
    0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x04, 0xD9, 0x02, 0xEB, 0x02, 0xD9, 0x06, 0xD9, 0x5C,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x12, 0xD9, 0x42, 0x04, 0xD9, 0x46, 0x08, 0xDE, 0xD9,
    0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x05, 0xD9, 0x42, 0x04, 0xEB, 0x03, 0xD9, 0x46, 0x08, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x52, 0x04, 0xD9, 0x03, 0xD8, 0xDA, 0xDF, 0xE0,
    0xF6, 0xC4, 0x05, 0x7A, 0x04, 0xD9, 0xC9, 0xEB, 0x04, 0xDD, 0xD9, 0xD9, 0x03, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x1A, 0xD9, 0x43, 0x08, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4,
    0x05, 0x7B, 0x05, 0xDD, 0xD8, 0xD9, 0x43, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0x5F, 0xD9, 0x44, 0x24,
    0x0C, 0x5E, 0xD9, 0x5A, 0x04, 0x5B, 0xC3,
];
const TRI_CALLS: [usize; 0] = [];
const LOD_BODY: [u8; 69] = [
    0x33, 0xC0, 0x85, 0xC9, 0x89, 0x8A, 0x34, 0x03, 0x00, 0x00, 0x7E, 0x1B, 0x8D, 0x8A, 0x38, 0x03,
    0x00, 0x00, 0x57, 0x8B, 0x3C, 0x86, 0x89, 0x39, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x04, 0x3B, 0x82,
    0x34, 0x03, 0x00, 0x00, 0x7C, 0xED, 0x5F, 0x8A, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x08, 0xD9,
    0x9A, 0x30, 0x03, 0x00, 0x00, 0x88, 0x82, 0x49, 0x03, 0x00, 0x00, 0xC6, 0x82, 0x48, 0x03, 0x00,
    0x00, 0xFF, 0xC2, 0x08, 0x00,
];
const LOD_CALLS: [usize; 0] = [];
const OCCL_BODY: [u8; 139] = [
    0x51, 0x8B, 0x44, 0x24, 0x08, 0x8B, 0x00, 0x53, 0x55, 0x8B, 0x29, 0xC1, 0xE0, 0x07, 0x56, 0x05,
    0x80, 0x26, 0xE8, 0x00, 0x33, 0xDB, 0x85, 0xED, 0x57, 0x7E, 0x5B, 0x8B, 0x78, 0x04, 0xD9, 0xEE,
    0x8D, 0x71, 0x18, 0x33, 0xD2, 0x85, 0xFF, 0x7E, 0x41, 0x8B, 0x4C, 0x24, 0x18, 0x83, 0xC1, 0x0C,
    0xD9, 0x41, 0xFC, 0xD8, 0x4E, 0xFC, 0xD9, 0x41, 0xF8, 0xD8, 0x4E, 0xF8, 0xDE, 0xC1, 0xD9, 0x06,
    0xD8, 0x09, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x46, 0x04, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x74, 0x21,
    0x83, 0xC2, 0x01, 0x83, 0xC1, 0x0C, 0x3B, 0xD7, 0x7C, 0xC6, 0x83, 0xC3, 0x01, 0x83, 0xC6, 0x10,
    0x3B, 0xDD, 0x7C, 0xAF, 0xDD, 0xD8, 0x5F, 0x5E, 0x5D, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5B, 0x59,
    0xC3, 0x5F, 0xDD, 0xD8, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0x59, 0xC3,
];
const OCCL_CALLS: [usize; 0] = [];
const NORM_BODY: [u8; 126] = [
    0x83, 0xEC, 0x14, 0xD9, 0x46, 0x04, 0x51, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9,
    0x06, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x46, 0x08, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2,
    0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x1C, 0x24, 0xE8,
    0x7C, 0x09, 0x83, 0xFF, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xF3, 0x0F, 0x10, 0x44,
    0x24, 0x10, 0xD9, 0xE8, 0xDE, 0xF1, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x14, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x1E, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0xC9,
    0xD9, 0x5E, 0x04, 0xD8, 0x4C, 0x24, 0x0C, 0xD9, 0x5E, 0x08, 0x83, 0xC4, 0x18, 0xC3,
];
const NORM_CALLS: [usize; 1] = [0x3f];
const SEGD_BODY: [u8; 420] = [
    0x83, 0xEC, 0x18, 0x8B, 0x4C, 0x24, 0x1C, 0xD9, 0x02, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x01, 0xD9,
    0x44, 0x24, 0x1C, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x42, 0x04,
    0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x41, 0x04, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x42, 0x08, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x41, 0x08, 0xD9,
    0x44, 0x24, 0x1C, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x00, 0xD8,
    0xE3, 0xD9, 0x1C, 0x24, 0xD9, 0x40, 0x04, 0xDE, 0xE2, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x04, 0xD8,
    0x68, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x04, 0xD9,
    0xC0, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC4, 0xDE, 0xCD, 0xD9,
    0xCA, 0xDE, 0xC4, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x1C, 0xD9,
    0x44, 0x24, 0x1C, 0xDD, 0x05, 0x60, 0xC4, 0xE5, 0x00, 0xDF, 0xF1, 0x76, 0x28, 0x85, 0xF6, 0xDD,
    0xD8, 0xDD, 0xDB, 0xDD, 0xD9, 0xDD, 0xD9, 0xDD, 0xD8, 0x74, 0x10, 0x8B, 0x02, 0x89, 0x06, 0x8B,
    0x42, 0x04, 0x89, 0x46, 0x04, 0x8B, 0x42, 0x08, 0x89, 0x46, 0x08, 0x8B, 0xC2, 0x83, 0xC4, 0x18,
    0xE9, 0x1B, 0xE7, 0xFF, 0xFF, 0xD9, 0xC3, 0xD8, 0x4C, 0x24, 0x0C, 0xD9, 0xC2, 0xD8, 0x4C, 0x24,
    0x10, 0xDE, 0xC1, 0xD9, 0xC3, 0xD8, 0x4C, 0x24, 0x14, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x1C, 0xD8,
    0x7C, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x1C, 0xDB, 0xF1, 0xDD,
    0xD9, 0x76, 0x11, 0xF3, 0x0F, 0x10, 0x05, 0x8C, 0x51, 0xDD, 0x00, 0x0F, 0x2F, 0x44, 0x24, 0x1C,
    0x76, 0x05, 0xEB, 0x0F, 0x0F, 0x57, 0xC0, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x1C, 0xDD, 0xD8, 0xD9,
    0x44, 0x24, 0x1C, 0x85, 0xF6, 0xD9, 0xC0, 0xDE, 0xCC, 0xD9, 0xCB, 0xD9, 0x5C, 0x24, 0x0C, 0xD8,
    0xCA, 0xD9, 0x5C, 0x24, 0x10, 0xDE, 0xC9, 0xD9, 0x5C, 0x24, 0x14, 0xD8, 0x44, 0x24, 0x0C, 0xD9,
    0x1C, 0x24, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x42, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x42, 0x08,
    0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x08, 0x74, 0x13, 0x8B, 0x14, 0x24, 0x8B, 0x44, 0x24,
    0x04, 0x89, 0x16, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x46, 0x04, 0x89, 0x56, 0x08, 0xD9, 0x04, 0x24,
    0xD8, 0x21, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x10,
    0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB,
    0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C,
    0x83, 0xC4, 0x18, 0xC3,
];
const SEGD_CALLS: [usize; 0] = []; // plus the tail `jmp` at +0xC0, patched by hand
const AABB_BODY: [u8; 90] = [
    0x51, 0xD9, 0x02, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x01, 0xDF, 0xF1, 0x77, 0x45, 0xD9,
    0x06, 0xD9, 0xC9, 0xDF, 0xF1, 0xDD, 0xD8, 0x77, 0x3D, 0xD9, 0x42, 0x04, 0xD9, 0x1C, 0x24, 0xD9,
    0x04, 0x24, 0xD9, 0x41, 0x04, 0xDF, 0xF1, 0x77, 0x2B, 0xD9, 0x46, 0x04, 0xD9, 0xC9, 0xDF, 0xF1,
    0xDD, 0xD8, 0x77, 0x22, 0xD9, 0x42, 0x08, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x41, 0x08,
    0xDF, 0xF1, 0x77, 0x10, 0xD9, 0x46, 0x08, 0xDF, 0xF1, 0xDD, 0xD8, 0x72, 0x09, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x59, 0xC3, 0xDD, 0xD8, 0x33, 0xC0, 0x59, 0xC3,
];
const AABB_CALLS: [usize; 0] = [];
const MATV_BODY: [u8; 111] = [
    0x83, 0xEC, 0x0C, 0xD9, 0x01, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04, 0xD9, 0x1C, 0x24, 0xD9,
    0x41, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x40, 0x04, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCA,
    0xD9, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9, 0x40,
    0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9, 0x19, 0xD9,
    0x40, 0x10, 0xD8, 0xCA, 0xD9, 0x40, 0x0C, 0xD8, 0xCC, 0xDE, 0xC1, 0xD9, 0x40, 0x14, 0xD8, 0xCA,
    0xDE, 0xC1, 0xD9, 0x59, 0x04, 0xD9, 0x40, 0x1C, 0xDE, 0xCA, 0xD9, 0x40, 0x18, 0xDE, 0xCB, 0xD9,
    0xC9, 0xDE, 0xC2, 0xD8, 0x48, 0x20, 0xDE, 0xC1, 0xD9, 0x59, 0x08, 0x83, 0xC4, 0x0C, 0xC3,
];
const MATV_CALLS: [usize; 0] = [];
const LOWB_BODY: [u8; 83] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0xF0, 0x8B, 0xF9, 0x2B, 0xF7, 0xC1, 0xFE, 0x04,
    0x85, 0xF6, 0x7E, 0x39, 0x53, 0x8B, 0x5D, 0x04, 0x8B, 0xC6, 0x99, 0x2B, 0xC2, 0xD1, 0xF8, 0x8B,
    0xC8, 0xC1, 0xE1, 0x04, 0x8B, 0x54, 0x39, 0x04, 0x03, 0xCF, 0x3B, 0xD3, 0x75, 0x05, 0x8B, 0x11,
    0x3B, 0x55, 0x00, 0x0F, 0x92, 0xC2, 0x84, 0xD2, 0x74, 0x0C, 0x8D, 0x79, 0x10, 0x83, 0xC9, 0xFF,
    0x2B, 0xC8, 0x03, 0xF1, 0xEB, 0x02, 0x8B, 0xF0, 0x85, 0xF6, 0x7F, 0xCC, 0x5B, 0x8B, 0xC7, 0x5F,
    0x5E, 0x5D, 0xC3,
];
const LOWB_CALLS: [usize; 0] = [];
const VDIST_BODY: [u8; 123] = [
    0x83, 0xEC, 0x14, 0x80, 0x3D, 0xB6, 0x73, 0x52, 0x02, 0x00, 0xD9, 0x00, 0xD9, 0x5C, 0x24, 0x08,
    0xD9, 0x40, 0x04, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0x74, 0x05,
    0xE8, 0x1B, 0xB1, 0x86, 0x00, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0x26, 0xD9, 0x1C, 0x24, 0xD9, 0x04,
    0x24, 0xDC, 0xC8, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0x66, 0x04, 0xD9, 0x1C,
    0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x10, 0xD8, 0x66, 0x08, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24,
    0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDC, 0x0D, 0x28, 0xCF, 0xE5, 0x00, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x83, 0xC4, 0x14, 0xC3,
];
const VDIST_CALLS: [usize; 1] = [0x20];
#[cfg(test)]
const C_406810_BODY: [u8; 191] = [
    0x83, 0xEC, 0x30, 0x8B, 0x44, 0x24, 0x34, 0xD9, 0x42, 0x08, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x44,
    0x24, 0x34, 0xD9, 0xC0, 0xD8, 0x49, 0x18, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x41, 0x1C, 0xD8, 0xC9,
    0xD9, 0x5C, 0x24, 0x28, 0xD8, 0x49, 0x20, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x42, 0x04, 0xD9, 0x5C,
    0x24, 0x34, 0xD9, 0x41, 0x0C, 0xD9, 0x44, 0x24, 0x34, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x0C, 0xD9, 0x41, 0x10, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD8, 0x49, 0x14, 0xD9,
    0x5C, 0x24, 0x14, 0xD9, 0x02, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x01, 0xD9, 0x44, 0x24, 0x34, 0xD9,
    0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x1C, 0x24, 0xD9, 0x41, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24,
    0x04, 0xD8, 0x49, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x04, 0x24, 0xD8, 0x44, 0x24, 0x0C, 0xD9,
    0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x1C, 0xD9,
    0x44, 0x24, 0x08, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xD8,
    0x44, 0x24, 0x24, 0xD9, 0x18, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x28, 0xD9, 0x58, 0x04,
    0xD9, 0x44, 0x24, 0x20, 0xD8, 0x44, 0x24, 0x2C, 0xD9, 0x58, 0x08, 0x83, 0xC4, 0x30, 0xC3,
];
#[cfg(test)]
const C_406810_CALLS: [usize; 0] = [];
#[cfg(test)]
const C_BE2F50_BODY: [u8; 501] = [
    0x83, 0xEC, 0x3C, 0x8B, 0x54, 0x24, 0x40, 0xD9, 0x42, 0x1C, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x41,
    0x0C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x42, 0x18, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x01, 0xD9, 0x5C,
    0x24, 0x40, 0xD9, 0x41, 0x18, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x42, 0x20, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x44, 0x24, 0x40, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCC, 0xDE, 0xC1,
    0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x08, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x5C,
    0x24, 0x30, 0xD9, 0x41, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x41, 0x04, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x41, 0x1C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0xCB, 0xD9, 0x44, 0x24,
    0x10, 0xD8, 0xCD, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x34, 0xD9, 0x41, 0x14, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD9, 0x5C, 0x24, 0x40, 0xD9,
    0x41, 0x20, 0xD9, 0x1C, 0x24, 0xD9, 0x44, 0x24, 0x40, 0xD9, 0xC0, 0xDE, 0xCC, 0xD9, 0x44, 0x24,
    0x04, 0xD9, 0xC0, 0xDE, 0xCE, 0xD9, 0xCC, 0xDE, 0xC5, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xCB,
    0xD9, 0xCD, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x42, 0x10, 0xD9, 0x1C, 0x24,
    0xD9, 0x42, 0x0C, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x42, 0x14, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x40, 0xD8, 0xCD, 0xD9, 0x04, 0x24, 0xD8, 0xCB, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8,
    0x4C, 0x24, 0x08, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24,
    0x0C, 0xD9, 0x04, 0x24, 0xD8, 0x4C, 0x24, 0x10, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x4C,
    0x24, 0x14, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0xC9, 0xD9, 0x04,
    0x24, 0xD8, 0xCC, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xCD, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x2C, 0xD9, 0x42, 0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x02, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x42, 0x08,
    0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCE, 0xD9, 0x04, 0x24, 0xD9,
    0xC0, 0xDE, 0xCC, 0xD9, 0xCE, 0xDE, 0xC3, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24, 0x08, 0xDE,
    0xC3, 0xD9, 0xCA, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0xC1, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x18, 0xD8,
    0x4C, 0x24, 0x0C, 0xF3, 0x0F, 0x11, 0x00, 0xD9, 0xC5, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x24, 0xD8,
    0x4C, 0x24, 0x10, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x40, 0xD8, 0x4C, 0x24, 0x14, 0xDE, 0xC1, 0xD9,
    0x5C, 0x24, 0x1C, 0xDE, 0xC9, 0xD9, 0xCB, 0xDE, 0xC9, 0xDE, 0xC2, 0xD8, 0x4C, 0x24, 0x40, 0xDE,
    0xC1, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x20,
    0xD9, 0x58, 0x08, 0xF3, 0x0F, 0x11, 0x40, 0x0C, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x28, 0xF3, 0x0F,
    0x11, 0x40, 0x10, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x2C, 0xF3, 0x0F, 0x11, 0x40, 0x14, 0xF3, 0x0F,
    0x10, 0x44, 0x24, 0x30, 0xF3, 0x0F, 0x11, 0x40, 0x18, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x34, 0xF3,
    0x0F, 0x11, 0x40, 0x1C, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x38, 0xF3, 0x0F, 0x11, 0x40, 0x20, 0x83,
    0xC4, 0x3C, 0xC2, 0x04, 0x00,
];
#[cfg(test)]
const C_BE2F50_CALLS: [usize; 0] = [];
#[cfg(test)]
const C_40EA20_BODY: [u8; 29] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08, 0xE8, 0x9F, 0x1A, 0x89,
    0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D, 0xC3,
];
#[cfg(test)]
const C_40EA20_CALLS: [usize; 1] = [0xc];
#[cfg(test)]
const C_BDEA30_BODY: [u8; 71] = [
    0x83, 0xEC, 0x08, 0xD9, 0x01, 0xD8, 0x20, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04, 0xD8, 0x60, 0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC,
    0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD8, 0x60, 0x08, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x04, 0x83, 0xC4, 0x08, 0xC3,
];
#[cfg(test)]
const C_BDEA30_CALLS: [usize; 0] = [];


// --- 0x719FF0 transform_compose_to_esi --------------------------------------------------------------
// EDI = &A (mat33 at +0, pos at +0x24), ESI = &out (12 floats), stack (&B), ret 4. EAX = ESI.
// out.rot = A.rot x B.rot (0xBE2F50), out.pos = (B.pos * A.rot) (0x406810) + A.pos. Everything is
// read before the first write, so `out` may alias A or B.

unsafe fn xform(a: u32, b: u32, out: u32) {
    unsafe {
        let mut tmp = [0u32; 3];
        cr(MUL_VEC_MAT, 0, a, b + 0x24, 0, 0, &[tmp.as_mut_ptr() as u32]);
        let px = (f32::from_bits(tmp[0]) as f64 + fx(a as usize + 0x24)) as f32;
        let py = (fx(a as usize + 0x28) + f32::from_bits(tmp[1]) as f64) as f32;
        let pz = (fx(a as usize + 0x2C) + f32::from_bits(tmp[2]) as f64) as f32;
        let mut rot = [0u32; 12];
        cr(MAT_MUL, rot.as_mut_ptr() as u32, a, 0, 0, 0, &[b]);
        let o = out as usize;
        for (i, v) in rot[..9].iter().enumerate() {
            wd(o + i * 4, q(*v));
        }
        wf(o + 0x24, px);
        wf(o + 0x28, py);
        wf(o + 0x2C, pz);
    }
}

extern "C" fn xform_entry(r: &mut Pushad, args: *const u32) {
    let b = unsafe { *args };
    let (a, out) = (r.edi, r.esi);
    let res = unsafe {
        shadowed(XFORM, r, &[b], &[(out as usize, 48)], M_EAX, || {
            xform(a, b, out);
            Res { eax: out, ..Res::default() }
        })
    };
    r.eax = res.eax;
}
hot_thunk!(xform_thunk, xform_entry, "ret 4");

// --- 0x501850 nav_tri_bounds_xz ---------------------------------------------------------------------
// ECX = &out_min (x, z), EDX = &out_max (x, z), stack (&tri) -> three vertex pointers (x at +0, z at
// +8); plain ret. min: start at v0, then v1, v2 each replace the current value unless `cur < new`
// (the original's `test ah, 0x41; jne` takes the new vertex when new <= cur or unordered).
// max: replace unless `new < cur` (`test ah, 5; jp` takes new when new >= cur or unordered).

#[inline(always)]
fn lt(a: u32, b: u32) -> bool {
    f32::from_bits(a) < f32::from_bits(b)
}

unsafe fn tri_bounds(min: usize, max: usize, tri: usize) {
    unsafe {
        let v0 = rd(tri) as usize;
        let v1 = rd(tri + 4) as usize;
        let v2 = rd(tri + 8) as usize;
        wd(min, q(rd(v0)));
        wd(min + 4, q(rd(v0 + 8)));
        for v in [v1, v2] {
            let cur = q(rd(min));
            let new = q(rd(v));
            wd(min, if lt(cur, new) { cur } else { new });
            let cur = q(rd(min + 4));
            let new = q(rd(v + 8));
            wd(min + 4, if lt(cur, new) { cur } else { new });
        }
        wd(max, q(rd(v0)));
        wd(max + 4, q(rd(v0 + 8)));
        for v in [v1, v2] {
            let cur = q(rd(max));
            let new = q(rd(v));
            wd(max, if lt(new, cur) { cur } else { new });
            let cur = q(rd(max + 4));
            let new = q(rd(v + 8));
            wd(max + 4, if lt(new, cur) { cur } else { new });
        }
    }
}

extern "C" fn tri_entry(r: &mut Pushad, args: *const u32) {
    let tri = unsafe { *args } as usize;
    let (min, max) = (r.ecx as usize, r.edx as usize);
    unsafe {
        shadowed(TRI, r, &[tri as u32], &[(min, 8), (max, 8)], 0, || {
            tri_bounds(min, max, tri);
            Res::default()
        });
    }
}
hot_thunk!(tri_thunk, tri_entry, "ret");

// --- 0xD098B0 world_mesh_record_set_lod_list --------------------------------------------------------
// ECX = count, EDX = &record, ESI = &list (u32s), stack (byte flag, f32 value), ret 8.
// Copies list[0..count) to record+0x338.., count at +0x334, f32 at +0x330, byte at +0x349, 0xFF at
// +0x348. EAX on exit = the loop counter with AL replaced by the flag.

unsafe fn lod_list(count: i32, rec: usize, list: usize, flag: u32, val: u32) -> u32 {
    unsafe {
        wd(rec + 0x334, count as u32);
        let mut i = 0usize;
        if count > 0 {
            loop {
                let v = std::ptr::read_volatile((list + i * 4) as *const u32);
                std::ptr::write_volatile((rec + 0x338 + i * 4) as *mut u32, v);
                i += 1;
                if (i as i32) >= (std::ptr::read_volatile((rec + 0x334) as *const u32) as i32) {
                    break;
                }
            }
        }
        wd(rec + 0x330, q(val));
        wb(rec + 0x349, flag as u8);
        wb(rec + 0x348, 0xFF);
        ((i as u32) & !0xFF) | (flag & 0xFF)
    }
}

extern "C" fn lod_entry(r: &mut Pushad, args: *const u32) {
    let (flag, val) = unsafe { (*args, *args.add(1)) };
    let (count, rec, list) = (r.ecx as i32, r.edx as usize, r.esi as usize);
    let len = if count > 0 { (8 + 4 * count as usize).max(0x1A) } else { 0x1A };
    let len = len.min(4096);
    let res = unsafe {
        shadowed(LOD, r, &[flag, val], &[(rec + 0x330, len)], M_EAX, || Res { eax: lod_list(count, rec, list, flag, val), ..Res::default() })
    };
    r.eax = res.eax;
}
hot_thunk!(lod_thunk, lod_entry, "ret 8");

// --- 0xA024C0 occluder_encloses_shape_points --------------------------------------------------------
// ECX = &occluder (plane count at [0]; planes at +0x10, stride 0x10: nx, ny, nz, d), stack (&shape):
// [shape] = table index (0x80 bytes per entry at 0xE82680, point count at +4), points at shape+4,
// 12 bytes each. AL = 1 unless some dist >= 0 (ordered: `fcomp; test ah, 1; je`, so a NaN dist counts
// as behind); EAX is 0 / 1.
// dist = f32(f32((y*ny + x*nx) + z*nz) + d), the sums in 53-bit.

unsafe fn occl(occ: usize, shape: usize) -> u32 {
    unsafe {
        let entry = shape_table().wrapping_add(rd(shape).wrapping_shl(7) as usize);
        let planes = rd(occ) as i32;
        if planes <= 0 {
            return 1;
        }
        let pts = rd(entry.wrapping_add(4)) as i32;
        if pts <= 0 {
            return 1;
        }
        for p in 0..planes as usize {
            let pl = occ + 0x10 + 0x10 * p;
            let (nx, ny, nz, d) = (fx(pl), fx(pl + 4), fx(pl + 8), fx(pl + 12));
            for i in 0..pts as usize {
                let pt = shape + 4 + 12 * i;
                let s = ((fx(pt + 4) * ny + fx(pt) * nx) + nz * fx(pt + 8)) as f32;
                let dist = (s as f64 + d) as f32;
                if dist >= 0.0 {
                    return 0;
                }
            }
        }
        1
    }
}

extern "C" fn occl_entry(r: &mut Pushad, args: *const u32) {
    let shape = unsafe { *args } as usize;
    let occ = r.ecx as usize;
    let res = unsafe { shadowed(OCCL, r, &[shape as u32], &[], M_EAX, || Res { eax: occl(occ, shape), ..Res::default() }) };
    r.eax = res.eax;
}
hot_thunk!(occl_thunk, occl_entry, "ret");

// --- 0xBDE060 vec3_normalize_in_place_ret_len -------------------------------------------------------
// ESI = &v. v *= 1/len (len = f32 sqrt via 0x40EA20 of f32((x*x + y*y) + z*z)), XMM0 = len, no zero
// guard. x, y, z are read before any write.

unsafe fn normalize(v: usize) -> u32 {
    unsafe {
        let (x, y, z) = (fx(v), fx(v + 4), fx(v + 8));
        let sumsq = ((x * x + y * y) + z * z) as f32;
        let len = call_st0(SQRTF, 0, 0, 0, 0, 0, &[sumsq.to_bits()]) as f32;
        let inv = (1.0f64 / len as f64) as f32 as f64;
        wf(v, (x * inv) as f32);
        wf(v + 4, (y * inv) as f32);
        wf(v + 8, (z * inv) as f32);
        len.to_bits()
    }
}

extern "C" fn norm_entry(r: &mut Pushad, _args: *const u32) {
    let v = r.esi as usize;
    let res = unsafe { shadowed(NORM, r, &[], &[(v, 12)], M_XMM0, || Res { xmm0: normalize(v), ..Res::default() }) };
    r.esp = res.xmm0; // the thunk loads XMM0 from here
}
hot_thunk_x0!(norm_thunk, norm_entry, "ret");

// --- 0xBE0250 segment_point_distance_sq -------------------------------------------------------------
// EAX = &S1, EDX = &S0, stack (&P), ESI = &out (closest point) or 0; ST0 = |P - closest|^2.
// d = S1 - S0, dp = P - S0 (each stored f32), dd = f32((dx^2 + dy^2) + dz^2). dd below the double at
// 0xE5C460 (ordered): out = S0 (bit copy), tail call 0xBDEA30(EAX = S0, ECX = P). Otherwise
// t = f32(f32(dx*dpx + dy*dpy + dz*dpz) / dd) clamped (t <= 0 or NaN -> +0, t >= 1 -> 1.0), closest =
// f32(S0 + f32(d*t)), then the squared distance from the stored closest to P (re-read after the
// write, as the original does).

unsafe fn seg_dist(s1: usize, s0: usize, p: usize, out: usize) -> u32 {
    unsafe {
        let (s0x, s0y, s0z) = (fx(s0), fx(s0 + 4), fx(s0 + 8));
        let dpx = (fx(p) - s0x) as f32 as f64;
        let dpy = (fx(p + 4) - s0y) as f32 as f64;
        let dpz = (fx(p + 8) - s0z) as f32 as f64;
        let dx = (fx(s1) - s0x) as f32 as f64;
        let dy = (fx(s1 + 4) - s0y) as f32 as f64;
        let dz = (fx(s1 + 8) - s0z) as f32 as f64;
        let dd = ((dx * dx + dy * dy) + dz * dz) as f32 as f64;
        if seg_threshold() > dd {
            if out != 0 {
                wd(out, rd(s0));
                wd(out + 4, rd(s0 + 4));
                wd(out + 8, rd(s0 + 8));
            }
            return (call_st0(VEC3_DIST_SQ, s0 as u32, p as u32, 0, 0, 0, &[]) as f32).to_bits();
        }
        let num = ((dx * dpx + dy * dpy) + dz * dpz) as f32 as f64;
        let t = (num / dd) as f32;
        let t = if !(t > 0.0) {
            0.0f32
        } else if 1.0f32 <= t {
            1.0f32
        } else {
            t
        } as f64;
        let cx = (s0x + (dx * t) as f32 as f64) as f32;
        let cy = ((dy * t) as f32 as f64 + fx(s0 + 4)) as f32;
        let cz = (fx(s0 + 8) + (dz * t) as f32 as f64) as f32;
        if out != 0 {
            wf(out, cx);
            wf(out + 4, cy);
            wf(out + 8, cz);
        }
        let ex = (cx as f64 - fx(p)) as f32 as f64;
        let ey = (cy as f64 - fx(p + 4)) as f32 as f64;
        let ez = (cz as f64 - fx(p + 8)) as f32 as f64;
        (((ex * ex + ey * ey) + ez * ez) as f32).to_bits()
    }
}

extern "C" fn segd_entry(r: &mut Pushad, args: *const u32) -> f32 {
    let p = unsafe { *args } as usize;
    let (s1, s0, out) = (r.eax as usize, r.edx as usize, r.esi as usize);
    let rg = [(out, 12usize)];
    let res = unsafe { shadowed(SEGD, r, &[p as u32], &rg[..(out != 0) as usize], M_ST0, || Res { st0: seg_dist(s1, s0, p, out), ..Res::default() }) };
    f32::from_bits(res.st0)
}
hot_thunk!(segd_thunk, segd_entry, "ret");

// --- 0xBE9830 aabb_contains_point -------------------------------------------------------------------
// ECX = &min, ESI = &max, EDX = &point, plain ret; EAX = 1 inside, else 0. x and y: outside when
// min > p or p > max (both ordered, `ja`); z: outside when min > p (`ja`) or NOT (max >= p) (`jb`
// after the swapped compare, so an unordered z also fails).

unsafe fn aabb(min: usize, max: usize, p: usize) -> u32 {
    unsafe {
        let pt = |o: usize| f32::from_bits(q(rd(p + o)));
        let f = |a: usize| f32::from_bits(rd(a));
        let (px, py, pz) = (pt(0), pt(4), pt(8));
        if f(min) > px || px > f(max) {
            return 0;
        }
        if f(min + 4) > py || py > f(max + 4) {
            return 0;
        }
        if f(min + 8) > pz || !(f(max + 8) >= pz) {
            return 0;
        }
        1
    }
}

extern "C" fn aabb_entry(r: &mut Pushad, _args: *const u32) {
    let (min, max, p) = (r.ecx as usize, r.esi as usize, r.edx as usize);
    let res = unsafe { shadowed(AABB, r, &[], &[], M_EAX, || Res { eax: aabb(min, max, p), ..Res::default() }) };
    r.eax = res.eax;
}
hot_thunk!(aabb_thunk, aabb_entry, "ret");

// --- 0xBDE500 mat33_mul_vec3_in_place ---------------------------------------------------------------
// EAX = &m (3x3 row-major), ECX = &v (in / out), plain ret: v = M * v, each row (m0*x + m1*y) + m2*z
// in 53-bit then stored f32 (row 1 is written before row 2's matrix entries are read, like the
// original).

unsafe fn mat_vec(m: usize, v: usize) {
    unsafe {
        let (x, y, z) = (fx(v), fx(v + 4), fx(v + 8));
        let r1 = ((fx(m) * x + fx(m + 4) * y) + fx(m + 8) * z) as f32;
        wf(v, r1);
        let r2 = ((fx(m + 12) * x + fx(m + 16) * y) + fx(m + 20) * z) as f32;
        wf(v + 4, r2);
        let r3 = ((fx(m + 24) * x + fx(m + 28) * y) + fx(m + 32) * z) as f32;
        wf(v + 8, r3);
    }
}

extern "C" fn matv_entry(r: &mut Pushad, _args: *const u32) {
    let (m, v) = (r.eax as usize, r.ecx as usize);
    unsafe {
        shadowed(MATV, r, &[], &[(v, 12)], 0, || {
            mat_vec(m, v);
            Res::default()
        });
    }
}
hot_thunk!(matv_thunk, matv_entry, "ret");

// --- 0xBD5C30 lower_bound_key64_rec16 ---------------------------------------------------------------
// ECX = first, EAX = last (16-byte records), stack (&key {lo, hi}), plain ret; EAX = the first record
// not below the key (high dword compared first, then low, both unsigned).

unsafe fn lower_bound(first: u32, last: u32, key: usize) -> u32 {
    unsafe {
        let mut first = first;
        let mut n = (last.wrapping_sub(first) as i32) >> 4;
        if n <= 0 {
            return first;
        }
        let hi = rd(key + 4);
        while n > 0 {
            let half = n >> 1;
            let mid = first.wrapping_add((half as u32) << 4);
            let mh = rd(mid as usize + 4);
            let less = if mh != hi { mh < hi } else { rd(mid as usize) < rd(key) };
            if less {
                first = mid.wrapping_add(16);
                n -= half + 1;
            } else {
                n = half;
            }
        }
        first
    }
}

extern "C" fn lowb_entry(r: &mut Pushad, args: *const u32) {
    let key = unsafe { *args };
    let (first, last) = (r.ecx, r.eax);
    let res = unsafe { shadowed(LOWB, r, &[key], &[], M_EAX, || Res { eax: lower_bound(first, last, key as usize), ..Res::default() }) };
    r.eax = res.eax;
}
hot_thunk!(lowb_thunk, lowb_entry, "ret");

// --- 0x4B7650 view_distance_sq_scaled ---------------------------------------------------------------
// EAX = &p, ESI = &q, plain ret; ST0 = f32(sum * 0.5925...), sum = f32 of the squared f32 differences
// accumulated x, y, z (each step stored f32). When the byte at 0x25273B6 is set the original first
// calls 0xD22790 (thread state, result unused) after reading p.

unsafe fn view_dist(p: usize, qv: usize) -> u32 {
    unsafe {
        let (px, py, pz) = (fx(p), fx(p + 4), fx(p + 8));
        if rb(view_flag()) != 0 {
            cr(THREAD_STATE, 0, 0, 0, 0, 0, &[]);
        }
        let dx = (px - fx(qv)) as f32 as f64;
        let s = (dx * dx) as f32;
        let dy = (py - fx(qv + 4)) as f32 as f64;
        let s = (dy * dy + s as f64) as f32;
        let dz = (pz - fx(qv + 8)) as f32 as f64;
        let s = (dz * dz + s as f64) as f32;
        ((s as f64 * view_scale()) as f32).to_bits()
    }
}

extern "C" fn vdist_entry(r: &mut Pushad, _args: *const u32) -> f32 {
    let (p, qv) = (r.eax as usize, r.esi as usize);
    let res = unsafe { shadowed(VDIST, r, &[], &[], M_ST0, || Res { st0: view_dist(p, qv), ..Res::default() }) };
    f32::from_bits(res.st0)
}
hot_thunk!(vdist_thunk, vdist_entry, "ret");

// --- install ----------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    // (port, original body, its call offsets, thunk)
    let list: [(usize, &[u8], &[usize], usize); COUNT] = [
        (XFORM, &XFORM_BODY, &XFORM_CALLS, xform_thunk as *const () as usize),
        (TRI, &TRI_BODY, &TRI_CALLS, tri_thunk as *const () as usize),
        (LOD, &LOD_BODY, &LOD_CALLS, lod_thunk as *const () as usize),
        (OCCL, &OCCL_BODY, &OCCL_CALLS, occl_thunk as *const () as usize),
        (NORM, &NORM_BODY, &NORM_CALLS, norm_thunk as *const () as usize),
        (SEGD, &SEGD_BODY, &SEGD_CALLS, segd_thunk as *const () as usize),
        (AABB, &AABB_BODY, &AABB_CALLS, aabb_thunk as *const () as usize),
        (MATV, &MATV_BODY, &MATV_CALLS, matv_thunk as *const () as usize),
        (LOWB, &LOWB_BODY, &LOWB_CALLS, lowb_thunk as *const () as usize),
        (VDIST, &VDIST_BODY, &VDIST_CALLS, vdist_thunk as *const () as usize),
    ];
    let mut ok = 0;
    for (id, body, calls, thunk) in list {
        let port = &PORTS[id];
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, body, calls) } {
            Ok(copy) => {
                if id == SEGD {
                    // the tail `jmp rel32` to 0xBDEA30 at +0xC0
                    let rel = (VEC3_DIST_SQ as i32).wrapping_sub((copy + 0xC0 + 5) as i32);
                    unsafe { std::ptr::write_unaligned((copy + 0xC1) as *mut i32, rel) };
                }
                port.original.store(copy, Ordering::Relaxed)
            }
            Err(e) => log(&format!("port_hot57b: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_hot57b: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log (the first `SHADOW_CALLS` calls are counted).
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({} shadow mismatches)", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_hot57b: {}", parts.join(", "))
}


// =================================================================================================
// Offline differential test
// =================================================================================================
//
// Each test runs the ORIGINAL bytes (copied to executable memory; their calls aimed at test copies
// of the game's callee functions, their absolute data references at test statics) and the Rust
// port through its real thunk on the same random world, and compares: EAX / ST0 / XMM0 where they
// are results, the x87 depth, every register the original preserves, XMM1-7, and every word of the
// arena the pointers point into.
//
// Run:
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_hot57b -- --ignored --test-threads=1 --nocapture
// (CASES=n changes the number of random cases per function, default 4000.)

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    // --- the call harness -------------------------------------------------------------------------

    #[repr(C)]
    #[derive(Clone, Copy)]
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
        top0: u32,
        top1: u32,
        esp: u32,
        xmm: [u32; 32],
        st0: u64,
    }

    static mut SLOTS: [u32; 2] = [0; 2];

    #[derive(Clone, Copy)]
    struct In {
        eax: u32,
        ecx: u32,
        edx: u32,
        esi: u32,
        edi: u32,
    }

    const EBX_IN: u32 = 0xB1B1_B1B1;
    const EBP_IN: u32 = 0xE9E9_E9E9;

    fn xmm_pattern() -> [u32; 32] {
        let mut x = [0u32; 32];
        for (i, w) in x.iter_mut().enumerate() {
            *w = 0x3F00_0000 + (i as u32) * 0x0101_0101;
        }
        x
    }

    /// Call `f` with these registers and stack arguments (53-bit x87, clean stack); capture everything.
    unsafe fn call_t(f: usize, r: In, args: &[u32]) -> TB {
        let mut b = TB {
            f: f as u32, eax: r.eax, ecx: r.ecx, edx: r.edx, ebx: EBX_IN, esi: r.esi, edi: r.edi, ebp: EBP_IN,
            args: args.as_ptr() as u32, nargs: args.len() as u32,
            o_eax: 0, o_ecx: 0, o_edx: 0, o_ebx: 0, o_esi: 0, o_edi: 0, o_ebp: 0, top0: 0, top1: 0, esp: 0,
            xmm: xmm_pattern(), st0: 0,
        };
        unsafe {
            let slots = std::ptr::addr_of_mut!(SLOTS);
            (*slots)[0] = &mut b as *mut TB as u32;
            (*slots)[1] = f as u32;
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 76], esp",
                "mov [{s}], eax",
                "mov ebx, eax",
                "fninit",
                "push 0x27F",
                "fldcw word ptr [esp]",
                "add esp, 4",
                "movups xmm0, xmmword ptr [ebx + 80]",
                "movups xmm1, xmmword ptr [ebx + 96]",
                "movups xmm2, xmmword ptr [ebx + 112]",
                "movups xmm3, xmmword ptr [ebx + 128]",
                "movups xmm4, xmmword ptr [ebx + 144]",
                "movups xmm5, xmmword ptr [ebx + 160]",
                "movups xmm6, xmmword ptr [ebx + 176]",
                "movups xmm7, xmmword ptr [ebx + 192]",
                "mov ecx, [ebx + 36]",
                "mov esi, [ebx + 32]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "fnstsw word ptr [ebx + 68]",
                "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 20]", "mov edi, [ebx + 24]",
                "mov ebp, [ebx + 28]", "mov eax, [ebx + 4]", "mov ebx, [ebx + 16]",
                "call dword ptr [{s} + 4]",
                "push eax",
                "mov eax, dword ptr [{s}]",
                "mov [eax + 44], ecx", "mov [eax + 48], edx", "mov [eax + 52], ebx", "mov [eax + 56], esi",
                "mov [eax + 60], edi", "mov [eax + 64], ebp",
                "pop dword ptr [eax + 40]",
                "movups xmmword ptr [eax + 80], xmm0",
                "movups xmmword ptr [eax + 96], xmm1",
                "movups xmmword ptr [eax + 112], xmm2",
                "movups xmmword ptr [eax + 128], xmm3",
                "movups xmmword ptr [eax + 144], xmm4",
                "movups xmmword ptr [eax + 160], xmm5",
                "movups xmmword ptr [eax + 176], xmm6",
                "movups xmmword ptr [eax + 192], xmm7",
                "fnstsw word ptr [eax + 72]",
                "movzx ecx, word ptr [eax + 72]",
                "and ecx, 0x3800",
                "movzx edx, word ptr [eax + 68]",
                "and edx, 0x3800",
                "cmp ecx, edx",
                "je 5f",
                "fstp qword ptr [eax + 208]",
                "5:",
                "mov esp, [eax + 76]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                s = sym SLOTS,
                inout("eax") &mut b as *mut TB => _,
                clobber_abi("C"),
            );
        }
        b
    }

    fn exec_alloc(len: usize) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0);
        p
    }

    /// The original's bytes in executable memory: `abs` = (game address, test address) operands
    /// rewritten wherever they appear, `calls` = (offset of an E8 / E9, destination).
    fn build(body: &[u8], calls: &[(usize, usize)], abs: &[(u32, u32)]) -> usize {
        let mut b = body.to_vec();
        for &(old, new) in abs {
            let ob = old.to_le_bytes();
            let mut found = 0;
            for i in 0..b.len() - 3 {
                if b[i..i + 4] == ob {
                    b[i..i + 4].copy_from_slice(&new.to_le_bytes());
                    found += 1;
                }
            }
            assert!(found > 0, "operand {old:#x} not in the body");
        }
        let p = exec_alloc(b.len() + 16);
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len()) };
        for &(off, dest) in calls {
            let rel = (dest as i32).wrapping_sub((p + off + 5) as i32);
            unsafe { std::ptr::write_unaligned((p + off + 1) as *mut i32, rel) };
        }
        p
    }

    /// Test copies of the callee functions, registered as the redirect targets.
    fn setup() {
        let mut r = T_REDIR.lock().unwrap();
        if !r.is_empty() {
            return;
        }
        let c406810 = build(&C_406810_BODY, &[], &[]);
        let cbe2f50 = build(&C_BE2F50_BODY, &[], &[]);
        let cbdea30 = build(&C_BDEA30_BODY, &[], &[]);
        let sqrt_stub = build(&[0xD9, 0xFA, 0xC3], &[], &[]); // fsqrt; ret (the CRT's checked sqrt)
        let c40ea20 = build(&C_40EA20_BODY, &[(0xC, sqrt_stub)], &[]);
        let ts = build(&[0xC3], &[], &[]);
        r.extend([(MUL_VEC_MAT, c406810), (MAT_MUL, cbe2f50), (VEC3_DIST_SQ, cbdea30), (SQRTF, c40ea20), (THREAD_STATE, ts)]);
    }

    fn redirect(a: usize) -> usize {
        setup();
        tgt(a)
    }

    // --- random numbers ---------------------------------------------------------------------------

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
        fn f01(&mut self) -> f64 {
            self.u32() as f64 / 4294967296.0
        }
        /// A finite float: mostly full-mantissa values, some round ones, zeros, huge and tiny.
        fn fin(&mut self, scale: f64) -> u32 {
            match self.below(40) {
                0 => 0,
                1 => 0x8000_0000,
                2 | 3 => (((self.below(21) as f64 - 10.0) / 10.0 * scale) as f32).to_bits(),
                4 => ((self.f01() * 2.0 - 1.0) as f32 * 1e30).to_bits(),
                5 => ((self.f01() * 2.0 - 1.0) as f32 * 1e-30).to_bits(),
                6 => ((self.f01() * 2.0 - 1.0) as f32 * 1e-40).to_bits(),
                _ => (((self.f01() * 2.0 - 1.0) * scale) as f32).to_bits(),
            }
        }
        /// Any float: NaNs (quiet and signalling) and infinities included.
        fn any(&mut self) -> u32 {
            match self.below(10) {
                0 => 0x7F80_0000,
                1 => 0xFF80_0000,
                2 => 0x7FC0_0000 | self.u32() & 0x3F_FFFF,
                3 => 0x7F80_0000 | (self.u32() & 0x7F_FFFF) | 1,
                _ => self.fin(10.0),
            }
        }
    }

    const ARENA_WORDS: usize = 2048;

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    fn same_f(a: u32, b: u32) -> bool {
        a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
    }

    fn st0_f32(t: &TB) -> u32 {
        (f64::from_bits(t.st0) as f32).to_bits()
    }

    /// Run original and thunk on the same arena; returns both captures and arenas.
    unsafe fn both(orig: usize, thunk: usize, arena: &mut Vec<u32>, r: In, arg_words: &[usize]) -> (TB, Vec<u32>, TB, Vec<u32>) {
        let args: Vec<u32> = arg_words.iter().map(|&w| ptr(arena, w)).collect();
        unsafe { both_raw(orig, thunk, arena, r, &args) }
    }

    unsafe fn both_raw(orig: usize, thunk: usize, arena: &mut Vec<u32>, r: In, args: &[u32]) -> (TB, Vec<u32>, TB, Vec<u32>) {
        unsafe {
            let snap = arena.clone();
            let o = call_t(orig, r, args);
            let a1 = arena.clone();
            arena.copy_from_slice(&snap);
            let t = call_t(thunk, r, args);
            let a2 = arena.clone();
            arena.copy_from_slice(&snap);
            (o, a1, t, a2)
        }
    }

    /// What every thunk must satisfy besides its own result checks.
    fn common(o: &TB, t: &TB, a1: &[u32], a2: &[u32], r: In, float_mem: bool) -> Result<(), String> {
        if t.o_ebx != EBX_IN || t.o_esi != r.esi || t.o_edi != r.edi || t.o_ebp != EBP_IN {
            return Err(format!("registers changed: ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}", t.o_ebx, t.o_esi, t.o_edi, t.o_ebp));
        }
        if t.o_ecx != r.ecx || t.o_edx != r.edx {
            return Err(format!("ecx {:#x} edx {:#x} changed", t.o_ecx, t.o_edx));
        }
        if t.xmm[4..] != xmm_pattern()[4..] {
            return Err("xmm1-7 changed".into());
        }
        if o.o_ebx != EBX_IN || o.o_esi != r.esi || o.o_edi != r.edi || o.o_ebp != EBP_IN {
            return Err("the ORIGINAL changed a register the thunk is assumed to preserve".into());
        }
        let od = ((o.top0 & 0x3800).wrapping_sub(o.top1 & 0x3800) & 0x3800) >> 11;
        let td = ((t.top0 & 0x3800).wrapping_sub(t.top1 & 0x3800) & 0x3800) >> 11;
        if od != td {
            return Err(format!("x87 depth: original {od} Rust {td}"));
        }
        for (i, (x, y)) in a1.iter().zip(a2).enumerate() {
            let ok = if float_mem { same_f(*x, *y) } else { x == y };
            if !ok {
                return Err(format!("arena word {i}: original {x:#x} Rust {y:#x}"));
            }
        }
        Ok(())
    }

    fn ptr(arena: &Vec<u32>, word: usize) -> u32 {
        arena.as_ptr() as u32 + (word as u32) * 4
    }

    fn fill(arena: &mut Vec<u32>, rng: &mut Rng, f: impl Fn(&mut Rng) -> u32) {
        for w in arena.iter_mut() {
            *w = f(rng);
        }
    }

    fn report(name: &str, n: u32, bad: u32) {
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }

    macro_rules! check {
        ($name:expr, $i:expr, $bad:ident, $e:expr) => {
            if let Err(m) = $e {
                $bad += 1;
                if $bad <= 5 {
                    eprintln!("{} case {}: {}", $name, $i, m);
                }
            }
        };
    }

    // --- 0x719FF0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn xform_matches_original() {
        let orig = build(&XFORM_BODY, &[(0x12, redirect(MUL_VEC_MAT)), (0x3E, redirect(MAT_MUL))], &[]);
        let thunk = xform_thunk as *const () as usize;
        PORTS[XFORM].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[XFORM].calls.store(0, Ordering::Relaxed);
        PORTS[XFORM].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let sc = [1.0, 100.0, 1000.0][rng.below(3) as usize];
            fill(&mut arena, &mut rng, |r| r.fin(sc));
            if rng.below(30) == 0 {
                // one NaN or infinity (two NaNs in one product / sum pick a different payload on SSE)
                arena[64 + rng.below(24) as usize] = rng.any();
                arena[320 + rng.below(24) as usize] = rng.any();
            }
            let a = 64;
            let b = if rng.below(10) == 0 { a } else { 320 };
            let out = match rng.below(10) {
                0 => a,
                1 => b,
                2 => a + 3,
                _ => 640,
            };
            let r = In { eax: 0x1111_1111, ecx: 0x2222_2222, edx: 0x3333_3333, esi: ptr(&arena, out), edi: ptr(&arena, a) };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[b]) };
            check!("xform", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, true)?;
                if o.o_eax != t.o_eax || t.o_eax != r.esi {
                    return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
                }
                Ok(())
            })());
        }
        eprintln!("xform: shadow ran {} calls, {} mismatches", PORTS[XFORM].calls.load(Ordering::Relaxed), PORTS[XFORM].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[XFORM].mismatches.load(Ordering::Relaxed), 0);
        report("xform", n, bad);
    }

    // --- 0x501850 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn tri_matches_original() {
        let orig = build(&TRI_BODY, &[], &[]);
        let thunk = tri_thunk as *const () as usize;
        PORTS[TRI].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[TRI].calls.store(0, Ordering::Relaxed);
        PORTS[TRI].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x0BAD_CAFE_1234_5678);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            fill(&mut arena, &mut rng, |r| r.any());
            // three vertices of 4 words, the tri (three pointers), min and max (2 words)
            let v = [64usize, 68, 72];
            let vs: [usize; 3] = [v[0], if rng.below(6) == 0 { v[0] } else { v[1] }, if rng.below(6) == 0 { v[1] } else { v[2] }];
            // equal values now and then (the compare polarity on ties)
            if rng.below(3) == 0 {
                for k in 0..4 {
                    arena[v[1] + k] = arena[v[0] + k];
                }
            }
            if rng.below(3) == 0 {
                arena[v[2]] = arena[v[0]];
                arena[v[2] + 2] = arena[v[1] + 2];
            }
            if rng.below(8) == 0 {
                // signed zeros
                arena[v[1]] = 0x8000_0000;
                arena[v[0]] = 0;
                arena[v[2]] = 0x8000_0000;
                arena[v[2] + 2] = 0;
            }
            for k in 0..3 {
                arena[100 + k] = ptr(&arena, vs[k]);
            }
            let r = In { eax: 0, ecx: ptr(&arena, 200), edx: ptr(&arena, 204), esi: 0x5151_5151, edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[100]) };
            check!("tri", i, bad, common(&o, &t, &a1, &a2, r, false));
        }
        eprintln!("tri: shadow ran {} calls, {} mismatches", PORTS[TRI].calls.load(Ordering::Relaxed), PORTS[TRI].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[TRI].mismatches.load(Ordering::Relaxed), 0);
        report("tri", n, bad);
    }

    // --- 0xD098B0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn lod_matches_original() {
        let orig = build(&LOD_BODY, &[], &[]);
        let thunk = lod_thunk as *const () as usize;
        PORTS[LOD].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[LOD].calls.store(0, Ordering::Relaxed);
        PORTS[LOD].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x7777_1234_ABCD_0001);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            fill(&mut arena, &mut rng, |r| r.u32());
            let count = rng.below(30) as i32 - 3;
            let r = In { eax: rng.u32(), ecx: count as u32, edx: ptr(&arena, 300), esi: ptr(&arena, 1000), edi: 0xD1D1_D1D1 };
            let flag = rng.u32();
            let val = rng.any();
            let (o, a1, t, a2) = unsafe { both_raw(orig, thunk, &mut arena, r, &[flag, val]) };
            check!("lod", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if o.o_eax != t.o_eax {
                    return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
                }
                Ok(())
            })());
        }
        eprintln!("lod: shadow ran {} calls, {} mismatches", PORTS[LOD].calls.load(Ordering::Relaxed), PORTS[LOD].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[LOD].mismatches.load(Ordering::Relaxed), 0);
        report("lod", n, bad);
    }

    // --- 0xA024C0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn occl_matches_original() {
        let tbl = std::ptr::addr_of!(T_TBL) as u32;
        let orig = build(&OCCL_BODY, &[], &[(SHAPE_TABLE as u32, tbl)]);
        let thunk = occl_thunk as *const () as usize;
        PORTS[OCCL].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[OCCL].calls.store(0, Ordering::Relaxed);
        PORTS[OCCL].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x5EED_5EED_0011_2233);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        let mut ones = 0;
        for i in 0..n {
            let mode = rng.below(4);
            fill(&mut arena, &mut rng, |r| r.fin(3.0));
            // occluder at word 64: count, planes at +0x10 (word 4), 4 words each
            let planes = rng.below(7) as i32 - 1;
            arena[64] = planes as u32;
            for p in 0..6usize {
                let d = match mode {
                    0 => -1000.0f32,
                    1 => -6.0,
                    2 => -1.0,
                    _ => 1.0,
                };
                arena[64 + 4 + p * 4 + 3] = (d + (rng.f01() as f32 - 0.5) * d.abs()).to_bits();
                if rng.below(40) == 0 {
                    arena[64 + 4 + p * 4 + rng.below(4) as usize] = rng.any();
                }
            }
            // shape at word 300: idx, points
            let idx = rng.below(16) as usize;
            let pts = rng.below(9) as i32 - 1;
            unsafe {
                let t = std::ptr::addr_of_mut!(T_TBL);
                for e in 0..16 {
                    (*t)[e * 32 + 1] = (rng.below(9)) as u32;
                }
                (*t)[idx * 32 + 1] = pts as u32;
            }
            arena[300] = idx as u32;
            let r = In { eax: rng.u32(), ecx: ptr(&arena, 64), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[300]) };
            ones += (o.o_eax == 1) as u32;
            check!("occl", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if o.o_eax != t.o_eax {
                    return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
                }
                Ok(())
            })());
        }
        eprintln!("occl: {ones} of {n} cases returned 1");
        eprintln!("occl: shadow ran {} calls, {} mismatches", PORTS[OCCL].calls.load(Ordering::Relaxed), PORTS[OCCL].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[OCCL].mismatches.load(Ordering::Relaxed), 0);
        report("occl", n, bad);
    }

    // --- 0xBDE060 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn norm_matches_original() {
        let orig = build(&NORM_BODY, &[(0x3F, redirect(SQRTF))], &[]);
        let thunk = norm_thunk as *const () as usize;
        PORTS[NORM].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[NORM].calls.store(0, Ordering::Relaxed);
        PORTS[NORM].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x00DE_0600_ABCD_EF01);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let sc = [1.0, 50.0, 1e6][rng.below(3) as usize];
            fill(&mut arena, &mut rng, |r| r.fin(sc));
            if rng.below(30) == 0 {
                arena[64] = 0;
                arena[65] = 0;
                arena[66] = 0;
            }
            if rng.below(40) == 0 {
                arena[64 + rng.below(3) as usize] = rng.any();
            }
            let r = In { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), esi: ptr(&arena, 64), edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[]) };
            check!("norm", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, true)?;
                if !same_f(o.xmm[0], t.xmm[0]) {
                    return Err(format!("xmm0 original {:#x} Rust {:#x}", o.xmm[0], t.xmm[0]));
                }
                if o.xmm[1..4] != t.xmm[1..4] {
                    return Err("xmm0 upper lanes differ".to_string());
                }
                Ok(())
            })());
        }
        eprintln!("norm: shadow ran {} calls, {} mismatches", PORTS[NORM].calls.load(Ordering::Relaxed), PORTS[NORM].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[NORM].mismatches.load(Ordering::Relaxed), 0);
        report("norm", n, bad);
    }

    // --- 0xBE0250 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn segd_matches_original() {
        let thr = std::ptr::addr_of!(T_THR) as u32;
        let one = std::ptr::addr_of!(T_ONE) as u32;
        let orig = build(&SEGD_BODY, &[(0xC0, redirect(VEC3_DIST_SQ))], &[(0x00E5_C460, thr), (0x00DD_518C, one)]);
        let thunk = segd_thunk as *const () as usize;
        PORTS[SEGD].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[SEGD].calls.store(0, Ordering::Relaxed);
        PORTS[SEGD].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x0BE0_2500_1357_9BDF);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let sc = [1.0, 100.0, 5000.0][rng.below(3) as usize];
            fill(&mut arena, &mut rng, |r| r.fin(sc));
            // S0 at 64, S1 at 70, P at 76, out at 90
            let kind = rng.below(12);
            for k in 0..3 {
                let s0 = f32::from_bits(arena[64 + k]);
                match kind {
                    0 => arena[70 + k] = arena[64 + k],
                    1 => arena[70 + k] = (s0 + (rng.f01() as f32 - 0.5) * 2e-5).to_bits(),
                    2 => arena[76 + k] = (s0 + (rng.f01() as f32 - 0.5) * sc as f32 * 4.0).to_bits(),
                    _ => {}
                }
            }
            if kind >= 8 {
                // P near the segment (t in range): P = S0 + u * (S1 - S0) + noise
                let u = rng.f01() as f32;
                for k in 0..3 {
                    let (s0, s1) = (f32::from_bits(arena[64 + k]), f32::from_bits(arena[70 + k]));
                    arena[76 + k] = (s0 + u * (s1 - s0) + (rng.f01() as f32 - 0.5) * 0.01).to_bits();
                }
            }
            let out = if rng.below(4) == 0 { 0 } else if rng.below(12) == 0 { 76 } else { 90 };
            let r = In { eax: ptr(&arena, 70), ecx: rng.u32(), edx: ptr(&arena, 64), esi: if out == 0 { 0 } else { ptr(&arena, out) }, edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[76]) };
            check!("segd", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if st0_f32(&o) != st0_f32(&t) {
                    return Err(format!("st0 original {:#x} Rust {:#x}", st0_f32(&o), st0_f32(&t)));
                }
                if ((o.top0 & 0x3800).wrapping_sub(o.top1 & 0x3800) & 0x3800) >> 11 != 1 {
                    return Err("original left no ST0".to_string());
                }
                Ok(())
            })());
        }
        eprintln!("segd: shadow ran {} calls, {} mismatches", PORTS[SEGD].calls.load(Ordering::Relaxed), PORTS[SEGD].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[SEGD].mismatches.load(Ordering::Relaxed), 0);
        report("segd", n, bad);
    }

    // --- 0xBE9830 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn aabb_matches_original() {
        let orig = build(&AABB_BODY, &[], &[]);
        let thunk = aabb_thunk as *const () as usize;
        PORTS[AABB].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[AABB].calls.store(0, Ordering::Relaxed);
        PORTS[AABB].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x0BE9_8300_2468_ACE0);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        let mut ones = 0;
        for i in 0..n {
            for k in 0..9 {
                // min below, max above on most axes; p in between with some luck
                let lo = rng.f01() as f32 * -1.5;
                let hi = rng.f01() as f32 * 1.5;
                match k {
                    0..=2 => arena[64 + k] = lo.to_bits(),
                    3..=5 => arena[70 + k - 3] = hi.to_bits(),
                    _ => arena[76 + k - 6] = ((rng.f01() as f32 - 0.5) * 2.2).to_bits(),
                }
            }
            // ties and specials
            for _ in 0..rng.below(3) {
                let which = rng.below(9) as usize;
                let pos = [64usize, 65, 66, 70, 71, 72, 76, 77, 78][which];
                match rng.below(6) {
                    0 => arena[pos] = rng.any(),
                    1 => arena[pos] = arena[[64usize, 65, 66, 70, 71, 72, 76, 77, 78][rng.below(9) as usize]],
                    2 => arena[pos] = arena[76 + (pos % 3)],
                    _ => {}
                }
            }
            let r = In { eax: rng.u32(), ecx: ptr(&arena, 64), edx: ptr(&arena, 76), esi: ptr(&arena, 70), edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[]) };
            ones += (o.o_eax == 1) as u32;
            check!("aabb", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if o.o_eax != t.o_eax {
                    return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
                }
                Ok(())
            })());
        }
        eprintln!("aabb: {ones} of {n} cases returned 1");
        eprintln!("aabb: shadow ran {} calls, {} mismatches", PORTS[AABB].calls.load(Ordering::Relaxed), PORTS[AABB].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[AABB].mismatches.load(Ordering::Relaxed), 0);
        report("aabb", n, bad);
    }

    // --- 0xBDE500 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn matv_matches_original() {
        let orig = build(&MATV_BODY, &[], &[]);
        let thunk = matv_thunk as *const () as usize;
        PORTS[MATV].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[MATV].calls.store(0, Ordering::Relaxed);
        PORTS[MATV].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x00DE_5000_FEDC_BA98);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let sc = [1.0, 100.0, 1e5][rng.below(3) as usize];
            fill(&mut arena, &mut rng, |r| r.fin(sc));
            if rng.below(40) == 0 {
                arena[64 + rng.below(9) as usize] = rng.any();
            }
            let v = match rng.below(12) {
                0 => 64,
                1 => 64 + 3,
                2 => 64 + 6,
                3 => 64 + 8,
                _ => 200,
            };
            let r = In { eax: ptr(&arena, 64), ecx: ptr(&arena, v), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[]) };
            check!("matv", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, true)?;
                if t.o_eax != r.eax {
                    return Err("eax changed".to_string());
                }
                Ok(())
            })());
        }
        eprintln!("matv: shadow ran {} calls, {} mismatches", PORTS[MATV].calls.load(Ordering::Relaxed), PORTS[MATV].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[MATV].mismatches.load(Ordering::Relaxed), 0);
        report("matv", n, bad);
    }

    // --- 0xBD5C30 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn lowb_matches_original() {
        let orig = build(&LOWB_BODY, &[], &[]);
        let thunk = lowb_thunk as *const () as usize;
        PORTS[LOWB].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[LOWB].calls.store(0, Ordering::Relaxed);
        PORTS[LOWB].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x0BD5_C300_1122_3344);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let small = rng.below(2) == 0;
            fill(&mut arena, &mut rng, |r| if small { r.below(4) } else { r.u32() });
            if rng.below(3) == 0 {
                // sort the records by key so the search is meaningful
                let cnt = 60usize;
                let mut recs: Vec<[u32; 4]> = (0..cnt).map(|k| [arena[64 + k * 4], arena[65 + k * 4], arena[66 + k * 4], arena[67 + k * 4]]).collect();
                recs.sort_by_key(|r| (r[1], r[0]));
                for (k, rc) in recs.iter().enumerate() {
                    arena[64 + k * 4..68 + k * 4].copy_from_slice(rc);
                }
            }
            let cnt = match rng.below(10) {
                0 => 0,
                1 => 1,
                _ => rng.below(60) as i32,
            };
            let first = ptr(&arena, 64);
            let last = if rng.below(20) == 0 { first.wrapping_sub(32) } else { first + 16 * cnt as u32 };
            // the key: sometimes a record's own key
            let (lo, hi) = if rng.below(2) == 0 {
                let k = rng.below(60) as usize;
                (arena[64 + k * 4], arena[65 + k * 4])
            } else {
                (if small { rng.below(4) } else { rng.u32() }, if small { rng.below(4) } else { rng.u32() })
            };
            arena[400] = lo;
            arena[401] = hi;
            let r = In { eax: last, ecx: first, edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[400]) };
            check!("lowb", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if o.o_eax != t.o_eax {
                    return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
                }
                Ok(())
            })());
        }
        eprintln!("lowb: shadow ran {} calls, {} mismatches", PORTS[LOWB].calls.load(Ordering::Relaxed), PORTS[LOWB].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[LOWB].mismatches.load(Ordering::Relaxed), 0);
        report("lowb", n, bad);
    }

    // --- 0x4B7650 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn vdist_matches_original() {
        let flag = std::ptr::addr_of!(T_FLAG) as u32;
        let k = std::ptr::addr_of!(T_K) as u32;
        let orig = build(&VDIST_BODY, &[(0x20, redirect(THREAD_STATE))], &[(VIEW_FLAG as u32, flag), (0x00E5_CF28, k)]);
        let thunk = vdist_thunk as *const () as usize;
        PORTS[VDIST].original.store(orig, Ordering::Relaxed); // the in-game shadow runs inside the thunk
        PORTS[VDIST].calls.store(0, Ordering::Relaxed);
        PORTS[VDIST].mismatches.store(0, Ordering::Relaxed);
        let mut rng = Rng(0x04B7_6500_9988_7766);
        let mut arena = vec![0u32; ARENA_WORDS];
        let (mut bad, n) = (0, cases());
        for i in 0..n {
            let sc = [1.0, 100.0, 1e4, 1e15][rng.below(4) as usize];
            fill(&mut arena, &mut rng, |r| r.fin(sc));
            unsafe { *std::ptr::addr_of_mut!(T_FLAG) = (rng.below(3) == 0) as u32 * (1 + rng.below(200)) };
            let r = In { eax: ptr(&arena, 64), ecx: rng.u32(), edx: rng.u32(), esi: ptr(&arena, 70), edi: 0xD1D1_D1D1 };
            let (o, a1, t, a2) = unsafe { both(orig, thunk, &mut arena, r, &[]) };
            check!("vdist", i, bad, (|| {
                common(&o, &t, &a1, &a2, r, false)?;
                if st0_f32(&o) != st0_f32(&t) {
                    return Err(format!("st0 original {:#x} Rust {:#x}", st0_f32(&o), st0_f32(&t)));
                }
                Ok(())
            })());
        }
        eprintln!("vdist: shadow ran {} calls, {} mismatches", PORTS[VDIST].calls.load(Ordering::Relaxed), PORTS[VDIST].mismatches.load(Ordering::Relaxed));
        assert_eq!(PORTS[VDIST].mismatches.load(Ordering::Relaxed), 0);
        report("vdist", n, bad);
    }
    /// Control: the shadow must notice a difference (the original's scale constant is changed under it).
    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn shadow_detects_a_difference() {
        let flag = std::ptr::addr_of!(T_FLAG) as u32;
        let k = std::ptr::addr_of!(T_K) as u32;
        let orig = build(&VDIST_BODY, &[(0x20, redirect(THREAD_STATE))], &[(VIEW_FLAG as u32, flag), (0x00E5_CF28, k)]);
        let thunk = vdist_thunk as *const () as usize;
        PORTS[VDIST].original.store(orig, Ordering::Relaxed);
        PORTS[VDIST].calls.store(0, Ordering::Relaxed);
        PORTS[VDIST].mismatches.store(0, Ordering::Relaxed);
        unsafe { *std::ptr::addr_of_mut!(T_K) = 0x3FF0_0000_0000_0000 }; // 1.0 instead of 0.5925
        let mut arena = vec![0u32; ARENA_WORDS];
        for (i, v) in [1.0f32, 2.0, 3.0, 4.0, 6.0, 9.0].iter().enumerate() {
            arena[64 + i % 3] = v.to_bits();
            arena[70 + i % 3] = (v * 0.5).to_bits();
        }
        let r = In { eax: ptr(&arena, 64), ecx: 0, edx: 0, esi: ptr(&arena, 70), edi: 0xD1D1_D1D1 };
        let want = unsafe { call_t(orig, r, &[]) };
        let got = unsafe { call_t(thunk, r, &[]) };
        unsafe { *std::ptr::addr_of_mut!(T_K) = 0x3FE2_F5E6_E000_0000 };
        assert!(PORTS[VDIST].mismatches.load(Ordering::Relaxed) >= 1, "the shadow missed a changed constant");
        assert_eq!(st0_f32(&want), st0_f32(&got), "on a mismatch the original's result must win");
        PORTS[VDIST].original.store(0, Ordering::Relaxed);
    }
}
