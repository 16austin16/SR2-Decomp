//! Port module: hot math / Havok-vector / vehicle leaves (doc 60 section 4).
//!
//! `vec3_scale_in_place_eax` (0x40B790), `hkvector4_sub_store` (0x485C30), `hkvector4_equal_store`
//! (0xBB8A10), `action_node_from_handle` (0x9149A0), `world_mesh_record_set_instance_data` (0xCF7760),
//! `vec3_dist_sq_xz_regs` (0xBDEA80), `vehicle_transform_store` (0xA96A70),
//! `view_clip_planes_sphere_outside` (0xD22930), `vehicle_is_helicopter` (0xAB5190),
//! `vec3_length_sq_eax` (0x40EA60), `mat33_copy_regs` (0xBE2C60), `vec3_dist_xz_regs` (0xBDEAB0),
//! `rng_chance_f32` (0x40B8D0), `f32_round_to_int` (0xBF0EA0), `hkvector4_dot4_store` (0x729A60),
//! `hkvector4_less_store` (0x72A100), `hkvector4_madd_splat_x` (0x71A0A0),
//! `hkvector4_add_in_place` (0x485D30), `hkvector4_and_mask_table` (0xBB8970) and
//! `hkrotation_from_quaternion_store` (0xD93F20).
//!
//! x87 originals are f64 arithmetic with an `as f32` at every `fstp dword` (the game runs the x87 at
//! 53-bit precision); the SSE originals are `core::arch` intrinsics in the original's operand order.
//!
//! Registers: every thunk is `pushad` + the XMM registers the original leaves alone, `f(&mut block,
//! &stack args)`, restore, `popad`. The block's EAX slot (or the `esp` slot, which carries XMM0 for
//! `f32_round_to_int`) holds the result. Four integer-only ports (`action_node_from_handle`,
//! `vehicle_is_helicopter`, `vehicle_transform_store`, `world_mesh_record_set_instance_data`) skip the
//! XMM save once the shadow window is over; `hkrotation_from_quaternion_store` clobbers XMM0-7 like the
//! original and saves none.
//!
//! Checks:
//! - Pure functions (`vec3_dist_sq_xz_regs`, `vec3_dist_xz_regs`, `vec3_length_sq_eax`,
//!   `action_node_from_handle`, `vehicle_is_helicopter`, `view_clip_planes_sphere_outside` (AL only),
//!   `f32_round_to_int`): relocated-original shadow for the first `SHADOW_CALLS` calls.
//! - Small known writers (`vec3_scale_in_place_eax`, the `hkvector4_*` stores, `mat33_copy_regs`,
//!   `vehicle_transform_store`, `world_mesh_record_set_instance_data`,
//!   `hkrotation_from_quaternion_store`): snapshot shadow (run original, save, restore, run Rust,
//!   compare, keep the original's writes on a mismatch).
//! - `rng_chance_f32`: its callee advances the generator, so no shadow; per-branch counters only
//!   (true / false results over the first `SHADOW_CALLS` calls).
//! - After the window every port is the bare Rust body: no logging, no allocation, no atomic writes.
//!
//! Deviations / notes:
//! - `hkvector4_*` and `hkrotation_from_quaternion_store` use unaligned loads and stores (the originals
//!   fault on a misaligned pointer; callers always pass aligned ones).
//! - Two NaN operands in one x87 multiply pick the larger payload, SSE picks the first operand; the
//!   game never feeds two NaNs into these leaves.
//! - `view_clip_planes_sphere_outside` returns the original's whole EAX assuming an empty x87 stack on
//!   entry (the upper bits are the x87 status the original leaves behind); callers read AL only. It
//!   also stores the last distance into its first stack argument slot like the original.
//! - `f32_round_to_int` keeps the original's `_ftol2_sse` flag check (0x34A536C): zero takes the
//!   original x87 slow path through the CRT routine itself.

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use core::arch::x86::*;

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- game addresses and constants ----------------------------------------------------------------

const THREAD_STATE: usize = 0x00D2_2790; // no inputs, EAX = per-thread state
const SQRTF: usize = 0x0040_EA20; // cdecl (float) -> ST0
const RNG_FLOAT01: usize = 0x00BD_CE30; // EAX = generator -> ST0 in [0, 1)
const RNG_GEN: usize = 0x00EC_30F8; // the global generator
const FTOL2: usize = 0x00DA_0140; // _ftol2_sse: ST0 -> EAX
const FTOL_FLAG: usize = 0x034A_536C; // nonzero: SSE2 path
const ACTION_TABLE: usize = 0x02A4_E508; // 0x960 records of 0x6C bytes
const SLOT_TABLE: usize = 0x0324_F7C4; // 0x4C bytes per slot, ids 1..=0x80
const ROT_ONE: usize = 0x00DC_9C20; // 1.0 in lane 0
const ROT_S0: usize = 0x00E5_7460;
const ROT_S1: usize = 0x00E5_7470;
const MASK_TABLE: usize = 0x00E5_38F0; // 16-byte rows

#[cfg(test)]
#[repr(C, align(16))]
pub(crate) struct A16(pub [u32; 4]);
#[cfg(test)]
#[repr(C, align(16))]
pub(crate) struct A16x(pub [u32; 64]);
#[cfg(test)]
pub(crate) static mut T_ACTION: [u8; 0x960 * 0x6C + 16] = [0; 0x960 * 0x6C + 16];
#[cfg(test)]
pub(crate) static mut T_SLOTS: [u8; 0x81 * 0x4C + 0x40] = [0; 0x81 * 0x4C + 0x40];
#[cfg(test)]
pub(crate) static mut T_GEN: [u32; (0x9C8 + 0x2000) / 4] = [0; (0x9C8 + 0x2000) / 4];
#[cfg(test)]
pub(crate) static mut T_FLAG: u32 = 1;
#[cfg(test)]
pub(crate) static mut T_C1: A16 = A16([0x3F80_0000, 0, 0, 0]);
#[cfg(test)]
pub(crate) static mut T_S0: A16 = A16([0, 0x8000_0000, 0x8000_0000, 0x8000_0000]);
#[cfg(test)]
pub(crate) static mut T_S1: A16 = A16([0x8000_0000, 0, 0x8000_0000, 0x8000_0000]);
#[cfg(test)]
pub(crate) static mut T_MASKS: A16x = A16x([0; 64]);
#[cfg(test)]
pub(crate) static T_REDIR: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

/// A game data address (the offline test aims them at test statics).
#[inline(always)]
fn k(a: usize) -> usize {
    #[cfg(test)]
    {
        match a {
            ACTION_TABLE => std::ptr::addr_of!(T_ACTION) as usize,
            SLOT_TABLE => std::ptr::addr_of!(T_SLOTS) as usize,
            RNG_GEN => std::ptr::addr_of!(T_GEN) as usize,
            FTOL_FLAG => std::ptr::addr_of!(T_FLAG) as usize,
            ROT_ONE => std::ptr::addr_of!(T_C1) as usize,
            ROT_S0 => std::ptr::addr_of!(T_S0) as usize,
            ROT_S1 => std::ptr::addr_of!(T_S1) as usize,
            MASK_TABLE => std::ptr::addr_of!(T_MASKS) as usize,
            _ => a,
        }
    }
    #[cfg(not(test))]
    {
        a
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
    unsafe { call_st0e(f, eax, ecx, edx, esi, edi, args).0 }
}

/// As `call_st0`, and also returns the callee's EAX.
unsafe fn call_st0e(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> (f64, u32) {
    let f = tgt(f);
    let mut block: [u32; 11] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, 0, 0, 0];
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
            "mov [ebx + 40], eax",
            "fstp qword ptr [ebx + 32]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_mut_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    (f64::from_bits(block[8] as u64 | (block[9] as u64) << 32), block[10])
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

/// `rep movsd` (forward, so overlapping source and destination behave like the original).
#[inline(always)]
unsafe fn movsd(dst: usize, src: usize, n: usize) {
    unsafe {
        std::arch::asm!(
            "push esi",
            "mov esi, {src}",
            "rep movsd dword ptr es:[edi], dword ptr [esi]",
            "pop esi",
            src = in(reg) src,
            inout("ecx") n => _,
            inout("edi") dst => _,
            options(preserves_flags),
        );
    }
}

/// Aligned load / store (`movaps`): the originals fault on a misaligned operand and so do we. The
/// compiler emits `movups` for the store, so the store is preceded by an aligned (volatile) load of the
/// same 16 bytes, which takes the fault the original's `movaps` store takes.
#[inline(always)]
unsafe fn lda(p: usize) -> __m128 {
    unsafe {
        let v: __m128;
        std::arch::asm!("movaps {v}, xmmword ptr [{p}]", v = out(xmm_reg) v, p = in(reg) p, options(nostack, readonly, preserves_flags));
        v
    }
}

#[inline(always)]
unsafe fn sta(p: usize, v: __m128) {
    unsafe {
        std::arch::asm!("movaps xmmword ptr [{p}], {v}", v = in(xmm_reg) v, p = in(reg) p, options(nostack, preserves_flags));
    }
}

/// Set XMM register `i` of the thunk's output array (the stack is only 4-byte aligned).
#[inline(always)]
unsafe fn xo_set(xo: *mut f32, i: usize, v: __m128) {
    unsafe { _mm_storeu_ps(xo.add(i * 4), v) }
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

/// Same, and XMM0 = the dword in the block's `esp` slot (the function's float result).
macro_rules! thunk_x1_out0 {
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

/// `pushad`, XMM0-7 saved (the original touches no XMM register at all).
macro_rules! thunk_all {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 128",
                "movups xmmword ptr [esp], xmm0",
                "movups xmmword ptr [esp + 16], xmm1",
                "movups xmmword ptr [esp + 32], xmm2",
                "movups xmmword ptr [esp + 48], xmm3",
                "movups xmmword ptr [esp + 64], xmm4",
                "movups xmmword ptr [esp + 80], xmm5",
                "movups xmmword ptr [esp + 96], xmm6",
                "movups xmmword ptr [esp + 112], xmm7",
                "lea eax, [esp + 128]",
                "lea ecx, [esp + 164]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, xmmword ptr [esp]",
                "movups xmm1, xmmword ptr [esp + 16]",
                "movups xmm2, xmmword ptr [esp + 32]",
                "movups xmm3, xmmword ptr [esp + 48]",
                "movups xmm4, xmmword ptr [esp + 64]",
                "movups xmm5, xmmword ptr [esp + 80]",
                "movups xmm6, xmmword ptr [esp + 96]",
                "movups xmm7, xmmword ptr [esp + 112]",
                "add esp, 128",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// `pushad`, XMM0-7 saved, `f(&mut block, &stack args, &mut [__m128; 8])` where the array holds the
/// entry XMM0-7 and is what the thunk loads back: the body overwrites the registers the original
/// leaves changed.
macro_rules! thunk_xo {
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "sub esp, 128",
                "movups xmmword ptr [esp], xmm0",
                "movups xmmword ptr [esp + 16], xmm1",
                "movups xmmword ptr [esp + 32], xmm2",
                "movups xmmword ptr [esp + 48], xmm3",
                "movups xmmword ptr [esp + 64], xmm4",
                "movups xmmword ptr [esp + 80], xmm5",
                "movups xmmword ptr [esp + 96], xmm6",
                "movups xmmword ptr [esp + 112], xmm7",
                "lea eax, [esp + 128]",
                "lea ecx, [esp + 164]",
                "push esp",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 12",
                "movups xmm0, xmmword ptr [esp]",
                "movups xmm1, xmmword ptr [esp + 16]",
                "movups xmm2, xmmword ptr [esp + 32]",
                "movups xmm3, xmmword ptr [esp + 48]",
                "movups xmm4, xmmword ptr [esp + 64]",
                "movups xmm5, xmmword ptr [esp + 80]",
                "movups xmm6, xmmword ptr [esp + 96]",
                "movups xmm7, xmmword ptr [esp + 112]",
                "add esp, 128",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// Integer-only body: XMM0-7 are saved only while the port's shadow window is open (the shadow
/// machinery uses SSE); afterwards just `pushad` / `popad`.
macro_rules! thunk_win {
    ($name:ident, $f:ident, $id:expr, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "cmp dword ptr [{p} + {off}], {n}",
                "jb 2f",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                $ret,
                "2:",
                "sub esp, 128",
                "movups xmmword ptr [esp], xmm0",
                "movups xmmword ptr [esp + 16], xmm1",
                "movups xmmword ptr [esp + 32], xmm2",
                "movups xmmword ptr [esp + 48], xmm3",
                "movups xmmword ptr [esp + 64], xmm4",
                "movups xmmword ptr [esp + 80], xmm5",
                "movups xmmword ptr [esp + 96], xmm6",
                "movups xmmword ptr [esp + 112], xmm7",
                "lea eax, [esp + 128]",
                "lea ecx, [esp + 164]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, xmmword ptr [esp]",
                "movups xmm1, xmmword ptr [esp + 16]",
                "movups xmm2, xmmword ptr [esp + 32]",
                "movups xmm3, xmmword ptr [esp + 48]",
                "movups xmm4, xmmword ptr [esp + 64]",
                "movups xmm5, xmmword ptr [esp + 80]",
                "movups xmm6, xmmword ptr [esp + 96]",
                "movups xmm7, xmmword ptr [esp + 112]",
                "add esp, 128",
                "popad",
                $ret,
                f = sym $f,
                p = sym PORTS,
                off = const ($id * std::mem::size_of::<Port>() + std::mem::offset_of!(Port, calls)),
                n = const SHADOW_CALLS,
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
const M_AL: u8 = 8;
/// The written memory is f32 lanes: two differing NaN payloads count as equal.
const M_FMEM: u8 = 16;

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
    /// A relocated copy of the original.
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize) -> Port {
    Port { name, addr, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const SCALE: usize = 0;
const SUB: usize = 1;
const EQ: usize = 2;
const ACTN: usize = 3;
const INST: usize = 4;
const DSQ: usize = 5;
const XFS: usize = 6;
const SPH: usize = 7;
const HELI: usize = 8;
const LSQ: usize = 9;
const M33: usize = 10;
const DXZ: usize = 11;
const RNG: usize = 12;
const ROUND: usize = 13;
const DOT4: usize = 14;
const LESS: usize = 15;
const MADD: usize = 16;
const ADD: usize = 17;
const ANDM: usize = 18;
const ROT: usize = 19;
const COUNT: usize = 20;

static PORTS: [Port; COUNT] = [
    port("vec3_scale_in_place_eax", 0x0040_B790),
    port("hkvector4_sub_store", 0x0048_5C30),
    port("hkvector4_equal_store", 0x00BB_8A10),
    port("action_node_from_handle", 0x0091_49A0),
    port("world_mesh_record_set_instance_data", 0x00CF_7760),
    port("vec3_dist_sq_xz_regs", 0x00BD_EA80),
    port("vehicle_transform_store", 0x00A9_6A70),
    port("view_clip_planes_sphere_outside", 0x00D2_2930),
    port("vehicle_is_helicopter", 0x00AB_5190),
    port("vec3_length_sq_eax", 0x0040_EA60),
    port("mat33_copy_regs", 0x00BE_2C60),
    port("vec3_dist_xz_regs", 0x00BD_EAB0),
    port("rng_chance_f32", 0x0040_B8D0),
    port("f32_round_to_int", 0x00BF_0EA0),
    port("hkvector4_dot4_store", 0x0072_9A60),
    port("hkvector4_less_store", 0x0072_A100),
    port("hkvector4_madd_splat_x", 0x0071_A0A0),
    port("hkvector4_add_in_place", 0x0048_5D30),
    port("hkvector4_and_mask_table", 0x00BB_8970),
    port("hkrotation_from_quaternion_store", 0x00D9_3F20),
];

const SNAP_MAX: usize = 640;

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
        log(&format!("port_hot60: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
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
        if mask & M_AL != 0 && o.eax & 0xFF != ours.eax & 0xFF {
            bad += &format!(" al original {:#x} Rust {:#x};", o.eax & 0xFF, ours.eax & 0xFF);
        }
        if mask & M_XMM0 != 0 && o.xmm0 != ours.xmm0 {
            bad += &format!(" xmm0 original {:#x} Rust {:#x};", o.xmm0, ours.xmm0);
        }
        if mask & M_ST0 != 0 && o.st0 != ours.st0 && !(f32::from_bits(o.st0).is_nan() && f32::from_bits(ours.st0).is_nan()) {
            bad += &format!(" st0 original {:#x} Rust {:#x};", o.st0, ours.st0);
        }
        if depth != (mask & M_ST0 != 0) as u32 {
            bad += &format!(" x87 depth {depth};");
        }
        if s1[..total] != s2[..total] && !(mask & M_FMEM != 0 && same_floats(&s1[..total], &s2[..total])) {
            bad += " memory differs;";
        }
        if bad.is_empty() {
            return ours;
        }
        snap_restore(ranges, &s1);
        let m = p.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
        if m <= 10 {
            log(&format!("port_hot60: {} mismatch (call {n}): regs eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} args {:x?}:{bad}", p.name, r.eax, r.ecx, r.edx, r.esi, r.edi, args));
        }
        o
    }
}

/// Memory that differs only in NaN payloads (two NaN operands in one SSE op) counts as equal for
/// the vector stores (every differing dword is a NaN in both).
fn same_floats(a: &[u8], b: &[u8]) -> bool {
    if a.len() % 4 != 0 {
        return false;
    }
    a.chunks_exact(4).zip(b.chunks_exact(4)).all(|(x, y)| {
        let (x, y) = (u32::from_le_bytes(x.try_into().unwrap()), u32::from_le_bytes(y.try_into().unwrap()));
        x == y || (f32::from_bits(x).is_nan() && f32::from_bits(y).is_nan())
    })
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

const SCALE_BODY: [u8; 31] = [
    0xD9, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x18, 0xD9, 0x40,
    0x04, 0xD8, 0xC9, 0xD9, 0x58, 0x04, 0xD8, 0x48, 0x08, 0xD9, 0x58, 0x08, 0xC2, 0x04, 0x00,
];
const SCALE_BODY_CALLS: [usize; 0] = [];
const SUB_BODY: [u8; 30] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x45, 0x08, 0x8B, 0x55, 0x0C, 0x0F, 0x28, 0x00, 0x0F,
    0x28, 0x0A, 0x0F, 0x5C, 0xC1, 0x0F, 0x29, 0x01, 0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00,
];
const SUB_BODY_CALLS: [usize; 0] = [];
const EQ_BODY: [u8; 31] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x45, 0x0C, 0x0F, 0x28, 0x00, 0x0F, 0x28, 0x09, 0x8B,
    0x45, 0x08, 0x0F, 0xC2, 0xC1, 0x00, 0x0F, 0x29, 0x00, 0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00,
];
const EQ_BODY_CALLS: [usize; 0] = [];
const ACTN_BODY: [u8; 58] = [
    0x85, 0xD2, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0xC2, 0x25, 0xFF, 0xFF, 0x00, 0x00, 0x3D, 0x60,
    0x09, 0x00, 0x00, 0x7D, 0xEF, 0x6B, 0xC0, 0x6C, 0x05, 0x08, 0xE5, 0xA4, 0x02, 0x74, 0xE5, 0x8B,
    0x48, 0x30, 0x85, 0xC9, 0x74, 0xDE, 0x83, 0x79, 0x04, 0x00, 0x74, 0xD8, 0x33, 0xC9, 0x39, 0x50,
    0x1C, 0x0F, 0x95, 0xC1, 0x83, 0xE9, 0x01, 0x23, 0xC1, 0xC3,
];
const ACTN_BODY_CALLS: [usize; 0] = [];
const INST_BODY: [u8; 97] = [
    0x8B, 0x54, 0x24, 0x0C, 0x89, 0x88, 0x60, 0x03, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x10, 0x56, 0x8B,
    0x74, 0x24, 0x0C, 0x57, 0x89, 0x88, 0x88, 0x05, 0x00, 0x00, 0x89, 0x90, 0x84, 0x05, 0x00, 0x00,
    0x66, 0x8B, 0x54, 0x24, 0x0C, 0x8D, 0xB8, 0x68, 0x03, 0x00, 0x00, 0xB9, 0x80, 0x00, 0x00, 0x00,
    0x66, 0x89, 0x90, 0x64, 0x03, 0x00, 0x00, 0xF3, 0xA5, 0x83, 0xC9, 0xFF, 0x89, 0x88, 0x68, 0x05,
    0x00, 0x00, 0x89, 0x88, 0x6C, 0x05, 0x00, 0x00, 0x33, 0xC9, 0x5F, 0x89, 0x88, 0x74, 0x05, 0x00,
    0x00, 0x89, 0x88, 0x78, 0x05, 0x00, 0x00, 0x89, 0x88, 0x7C, 0x05, 0x00, 0x00, 0x5E, 0xC2, 0x10,
    0x00,
];
const INST_BODY_CALLS: [usize; 0] = [];
const DSQ_BODY: [u8; 47] = [
    0x83, 0xEC, 0x08, 0xD9, 0x01, 0xD8, 0x20, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD8, 0x60,
    0x08, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x44, 0x24, 0x04, 0xDC, 0xC8, 0xD9, 0xC1, 0xDE,
    0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x83, 0xC4, 0x08, 0xC3,
];
const DSQ_BODY_CALLS: [usize; 0] = [];
const XFS_BODY: [u8; 113] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8D, 0xB8, 0xEC, 0x00, 0x00, 0x00, 0xB9, 0x09, 0x00,
    0x00, 0x00, 0x8B, 0xF3, 0xF3, 0xA5, 0x8B, 0x0A, 0x89, 0x88, 0x10, 0x01, 0x00, 0x00, 0x8B, 0x4A,
    0x04, 0x89, 0x88, 0x14, 0x01, 0x00, 0x00, 0x8B, 0x4A, 0x08, 0x89, 0x88, 0x18, 0x01, 0x00, 0x00,
    0x8B, 0x80, 0xA4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x31, 0x3D, 0x80, 0x00, 0x00, 0x00, 0x77,
    0x2A, 0x6B, 0xC0, 0x4C, 0x8D, 0x80, 0xC4, 0xF7, 0x24, 0x03, 0x85, 0xC0, 0x74, 0x1D, 0x8B, 0x0A,
    0x89, 0x48, 0x04, 0x8B, 0x4A, 0x04, 0x89, 0x48, 0x08, 0x8B, 0x52, 0x08, 0x8D, 0x78, 0x10, 0xB9,
    0x09, 0x00, 0x00, 0x00, 0x8B, 0xF3, 0x89, 0x50, 0x0C, 0xF3, 0xA5, 0x5F, 0x5E, 0x5B, 0xC2, 0x04,
    0x00,
];
const XFS_BODY_CALLS: [usize; 0] = [];
const SPH_BODY: [u8; 115] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0xF8, 0x8B, 0xF1, 0xE8, 0x50, 0xFE, 0xFF, 0xFF,
    0x85, 0xF6, 0x75, 0x0C, 0x8B, 0xB8, 0x84, 0x16, 0x00, 0x00, 0x8D, 0xB0, 0xDC, 0x15, 0x00, 0x00,
    0xD9, 0x44, 0x24, 0x14, 0x33, 0xD2, 0x8D, 0x4E, 0x0C, 0xD9, 0x41, 0xF8, 0xD8, 0x0B, 0xD9, 0x41,
    0xFC, 0xD8, 0x4B, 0x04, 0xDE, 0xC1, 0xD9, 0x43, 0x08, 0xD8, 0x09, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x41, 0x04, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10,
    0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x74, 0x12, 0x83, 0xC2, 0x01, 0x83, 0xC1, 0x1C, 0x3B,
    0xD7, 0x7C, 0xC6, 0x5F, 0xDD, 0xD8, 0x5E, 0x32, 0xC0, 0x5B, 0xC3, 0x5F, 0xDD, 0xD8, 0x5E, 0xB0,
    0x01, 0x5B, 0xC3,
];
const SPH_BODY_CALLS: [usize; 1] = [0xb];
const HELI_BODY: [u8; 28] = [
    0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x80, 0xE4, 0x84, 0x00, 0x00, 0x33, 0xC9, 0x83,
    0xB8, 0x9C, 0x00, 0x00, 0x00, 0x03, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3,
];
const HELI_BODY_CALLS: [usize; 0] = [];
const LSQ_BODY: [u8; 33] = [
    0x51, 0xD9, 0x40, 0x04, 0xD9, 0x00, 0xD9, 0x40, 0x08, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE,
    0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59,
    0xC3,
];
const LSQ_BODY_CALLS: [usize; 0] = [];
const M33_BODY: [u8; 53] = [
    0xD9, 0x01, 0xD9, 0x18, 0xD9, 0x41, 0x04, 0xD9, 0x58, 0x04, 0xD9, 0x41, 0x08, 0xD9, 0x58, 0x08,
    0xD9, 0x41, 0x0C, 0xD9, 0x58, 0x0C, 0xD9, 0x41, 0x10, 0xD9, 0x58, 0x10, 0xD9, 0x41, 0x14, 0xD9,
    0x58, 0x14, 0xD9, 0x41, 0x18, 0xD9, 0x58, 0x18, 0xD9, 0x41, 0x1C, 0xD9, 0x58, 0x1C, 0xD9, 0x41,
    0x20, 0xD9, 0x58, 0x20, 0xC3,
];
const M33_BODY_CALLS: [usize; 0] = [];
const DXZ_BODY: [u8; 58] = [
    0x83, 0xEC, 0x0C, 0xD9, 0x01, 0x51, 0xD8, 0x20, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x41, 0x08, 0xD8,
    0x60, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0xC8,
    0xD9, 0xC1, 0xDE, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x1C,
    0x24, 0xE8, 0x3A, 0xFF, 0x82, 0xFF, 0x83, 0xC4, 0x10, 0xC3,
];
const DXZ_BODY_CALLS: [usize; 1] = [0x31];
const RNG_BODY: [u8; 32] = [
    0xB8, 0xF8, 0x30, 0xEC, 0x00, 0xE8, 0x56, 0x15, 0x7D, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xDE, 0xD9,
    0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const RNG_BODY_CALLS: [usize; 1] = [0x5];
const ROUND_BODY: [u8; 41] = [
    0xF3, 0x0F, 0x10, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x0F, 0x2F, 0x05, 0xC0, 0x32, 0xDD,
    0x00, 0x72, 0x0B, 0xDC, 0x05, 0x70, 0xBF, 0xE5, 0x00, 0xE9, 0x82, 0xF2, 0x1A, 0x00, 0xDC, 0x25,
    0x70, 0xBF, 0xE5, 0x00, 0xE9, 0x77, 0xF2, 0x1A, 0x00,
];
const ROUND_BODY_CALLS: [usize; 0] = [];
const DOT4_BODY: [u8; 50] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x55, 0x0C, 0x0F, 0x28, 0x01, 0x0F, 0x28, 0x0A, 0x8B,
    0x45, 0x08, 0x0F, 0x59, 0xC8, 0x0F, 0x28, 0xC1, 0x0F, 0xC6, 0xC1, 0x4E, 0x0F, 0x58, 0xC1, 0x0F,
    0x28, 0xC8, 0x0F, 0xC6, 0xC8, 0xB1, 0x0F, 0x58, 0xC8, 0x0F, 0x29, 0x08, 0x8B, 0xE5, 0x5D, 0xC2,
    0x08, 0x00,
];
const DOT4_BODY_CALLS: [usize; 0] = [];
const LESS_BODY: [u8; 31] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x55, 0x0C, 0x0F, 0x28, 0x02, 0x0F, 0x28, 0x09, 0x8B,
    0x45, 0x08, 0x0F, 0xC2, 0xC8, 0x01, 0x0F, 0x29, 0x08, 0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00,
];
const LESS_BODY_CALLS: [usize; 0] = [];
const MADD_BODY: [u8; 45] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x55, 0x0C, 0x8B, 0xC1, 0x8B, 0x4D, 0x08, 0x0F, 0x28,
    0x01, 0x0F, 0x28, 0xC8, 0x0F, 0xC6, 0xC8, 0x00, 0x0F, 0x28, 0x02, 0x0F, 0x59, 0xC8, 0x0F, 0x28,
    0x00, 0x0F, 0x58, 0xC8, 0x0F, 0x29, 0x08, 0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00,
];
const MADD_BODY_CALLS: [usize; 0] = [];
const ADD_BODY: [u8; 29] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0xC1, 0x8B, 0x4D, 0x08, 0x0F, 0x28, 0x00, 0x0F, 0x28,
    0x09, 0x0F, 0x58, 0xC1, 0x0F, 0x29, 0x00, 0x8B, 0xE5, 0x5D, 0xC2, 0x04, 0x00,
];
const ADD_BODY_CALLS: [usize; 0] = [];
const ANDM_BODY: [u8; 35] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x8B, 0x45, 0x08, 0x0F, 0x28, 0x09, 0xC1, 0xE0, 0x04, 0x05,
    0xF0, 0x38, 0xE5, 0x00, 0x0F, 0x28, 0x00, 0x0F, 0x54, 0xC1, 0x0F, 0x29, 0x01, 0x8B, 0xE5, 0x5D,
    0xC2, 0x04, 0x00,
];
const ANDM_BODY_CALLS: [usize; 0] = [];
const ROT_BODY: [u8; 154] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x83, 0xEC, 0x10, 0x8B, 0x45, 0x08, 0x0F, 0x28, 0x00, 0x0F,
    0x28, 0x35, 0x20, 0x9C, 0xDC, 0x00, 0x0F, 0x28, 0x3D, 0x60, 0x74, 0xE5, 0x00, 0x0F, 0x28, 0xC8,
    0x0F, 0x58, 0xC8, 0x0F, 0x28, 0xD8, 0x0F, 0xC6, 0xD8, 0x41, 0x0F, 0x28, 0xE0, 0x0F, 0x28, 0xD1,
    0x0F, 0xC6, 0xD1, 0xA5, 0xF3, 0x0F, 0x59, 0xE1, 0x0F, 0xC6, 0xC0, 0xFE, 0x0F, 0xC6, 0xC9, 0x1A,
    0x0F, 0x59, 0xC1, 0x0F, 0x28, 0x0D, 0x70, 0x74, 0xE5, 0x00, 0x0F, 0x57, 0xC1, 0x0F, 0x59, 0xD3,
    0xF3, 0x0F, 0x58, 0xC6, 0x0F, 0x28, 0xEA, 0x0F, 0x57, 0x15, 0x60, 0x74, 0xE5, 0x00, 0x0F, 0x28,
    0xC8, 0x0F, 0x5C, 0xCA, 0x0F, 0x29, 0x3C, 0x24, 0x0F, 0x57, 0x04, 0x24, 0xF3, 0x0F, 0x10, 0xD4,
    0x0F, 0x28, 0xDE, 0x0F, 0x29, 0x09, 0x0F, 0x5C, 0xC2, 0x0F, 0xC6, 0xC0, 0xB1, 0xF3, 0x0F, 0x5C,
    0xDC, 0x0F, 0x12, 0xC8, 0xF3, 0x0F, 0x5C, 0xDD, 0x0F, 0xC6, 0xCB, 0x8D, 0x0F, 0x29, 0x41, 0x10,
    0x0F, 0x29, 0x49, 0x20, 0x8B, 0xE5, 0x5D, 0xC2, 0x04, 0x00,
];
const ROT_BODY_CALLS: [usize; 0] = [];


#[cfg(test)]
const T_BDCE30_BODY: [u8; 71] = [
    0x51, 0x8B, 0x88, 0xC4, 0x09, 0x00, 0x00, 0x8B, 0x94, 0x88, 0xC8, 0x09, 0x00, 0x00, 0x83, 0xC1,
    0x01, 0x81, 0xF9, 0x00, 0x08, 0x00, 0x00, 0x89, 0x88, 0xC4, 0x09, 0x00, 0x00, 0x75, 0x0A, 0xC7,
    0x80, 0xC4, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x85, 0xD2, 0x89, 0x14, 0x24, 0xDB, 0x04,
    0x24, 0x7D, 0x06, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xDC, 0x0D, 0x30, 0xC0, 0xE5, 0x00, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
#[cfg(test)]
const T_DA0140_BODY: [u8; 171] = [
    0x83, 0x3D, 0x6C, 0x53, 0x4A, 0x03, 0x00, 0x74, 0x2D, 0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x08, 0x83,
    0xE4, 0xF8, 0xDD, 0x1C, 0x24, 0xF2, 0x0F, 0x2C, 0x04, 0x24, 0xC9, 0xC3, 0x83, 0x3D, 0x6C, 0x53,
    0x4A, 0x03, 0x00, 0x74, 0x11, 0x83, 0xEC, 0x04, 0xD9, 0x3C, 0x24, 0x58, 0x66, 0x83, 0xE0, 0x7F,
    0x66, 0x83, 0xF8, 0x7F, 0x74, 0xD3, 0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x20, 0x83, 0xE4, 0xF0, 0xD9,
    0xC0, 0xD9, 0x54, 0x24, 0x18, 0xDF, 0x7C, 0x24, 0x10, 0xDF, 0x6C, 0x24, 0x10, 0x8B, 0x54, 0x24,
    0x18, 0x8B, 0x44, 0x24, 0x10, 0x85, 0xC0, 0x74, 0x3C, 0xDE, 0xE9, 0x85, 0xD2, 0x79, 0x1E, 0xD9,
    0x1C, 0x24, 0x8B, 0x0C, 0x24, 0x81, 0xF1, 0x00, 0x00, 0x00, 0x80, 0x81, 0xC1, 0xFF, 0xFF, 0xFF,
    0x7F, 0x83, 0xD0, 0x00, 0x8B, 0x54, 0x24, 0x14, 0x83, 0xD2, 0x00, 0xEB, 0x2C, 0xD9, 0x1C, 0x24,
    0x8B, 0x0C, 0x24, 0x81, 0xC1, 0xFF, 0xFF, 0xFF, 0x7F, 0x83, 0xD8, 0x00, 0x8B, 0x54, 0x24, 0x14,
    0x83, 0xDA, 0x00, 0xEB, 0x14, 0x8B, 0x54, 0x24, 0x14, 0xF7, 0xC2, 0xFF, 0xFF, 0xFF, 0x7F, 0x75,
    0xB8, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x18, 0xC9, 0xC3,
];
#[cfg(test)]
const T_40EA20_BODY: [u8; 29] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08, 0xE8, 0x9F, 0x1A, 0x89,
    0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0xE5, 0x5D, 0xC3,
];


// --- 0x40B790 vec3_scale_in_place_eax ---------------------------------------------------------------
// EAX = &v, stack (f32 s), ret 4: v.x = x*s, v.y = y*s, v.z = s*z (each a 24x24-bit product, exact in
// the x87 mantissa, so one f32 multiply). EAX preserved, x87 empty.

unsafe fn scale(v: usize, sbits: u32) {
    unsafe {
        let s = f32::from_bits(sbits);
        wf(v, rf(v) * s);
        wf(v + 4, rf(v + 4) * s);
        wf(v + 8, s * rf(v + 8));
    }
}

extern "C" fn scale_entry(r: &mut Pushad, args: *const u32) {
    let (v, s) = (r.eax as usize, unsafe { *args });
    unsafe {
        shadowed(SCALE, r, &[s], &[(v, 12)], 0, || {
            scale(v, s);
            Res::default()
        });
    }
}
thunk_all!(scale_thunk, scale_entry, "ret 4");

// --- 0x485C30 hkvector4_sub_store -------------------------------------------------------------------
// ECX = &out, stack (&a, &b), ret 8: out = a - b (subps, aligned loads and store like the original).
// Exit: EAX = &a, EDX = &b, ECX = &out, XMM0 = result, XMM1 = b.

unsafe fn sub_op(out: usize, a: usize, b: usize) -> (__m128, __m128) {
    unsafe {
        let va = lda(a);
        let vb = lda(b);
        let res = _mm_sub_ps(va, vb);
        sta(out, res);
        (res, vb)
    }
}

extern "C" fn sub_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (out, a, b) = (r.ecx as usize, unsafe { *args }, unsafe { *args.add(1) });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(SUB, r, &[a, b], &[(out, 16)], M_FMEM, || {
            o = sub_op(out, a as usize, b as usize);
            Res::default()
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = a;
    r.edx = b;
}
thunk_xo!(sub_thunk, sub_entry, "ret 8");

// --- 0xBB8A10 hkvector4_equal_store -----------------------------------------------------------------
// ECX = &a, stack (&out, &b), ret 8: out = cmpeqps(b, a) per lane (NaN lanes are 0).
// Exit: EAX = &out, XMM0 = result, XMM1 = a.

unsafe fn eq_op(a: usize, out: usize, b: usize) -> (__m128, __m128) {
    unsafe {
        let vb = lda(b);
        let va = lda(a);
        let res = _mm_cmpeq_ps(vb, va);
        sta(out, res);
        (res, va)
    }
}

extern "C" fn eq_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (a, out, b) = (r.ecx as usize, unsafe { *args }, unsafe { *args.add(1) });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(EQ, r, &[out, b], &[(out as usize, 16)], 0, || {
            o = eq_op(a, out as usize, b as usize);
            Res::default()
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = out;
}
thunk_xo!(eq_thunk, eq_entry, "ret 8");

// --- 0x72A100 hkvector4_less_store ------------------------------------------------------------------
// ECX = &a, stack (&out, &b), ret 8: out = cmpltps(a, b) (all ones where a < b).
// Exit: EAX = &out, EDX = &b, XMM0 = b, XMM1 = result.

unsafe fn less_op(a: usize, out: usize, b: usize) -> (__m128, __m128) {
    unsafe {
        let vb = lda(b);
        let va = lda(a);
        let res = _mm_cmplt_ps(va, vb);
        sta(out, res);
        (vb, res)
    }
}

extern "C" fn less_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (a, out, b) = (r.ecx as usize, unsafe { *args }, unsafe { *args.add(1) });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(LESS, r, &[out, b], &[(out as usize, 16)], 0, || {
            o = less_op(a, out as usize, b as usize);
            Res::default()
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = out;
    r.edx = b;
}
thunk_xo!(less_thunk, less_entry, "ret 8");

// --- 0x729A60 hkvector4_dot4_store ------------------------------------------------------------------
// ECX = &a, stack (&out, &b), ret 8: t = b*a, u = t + shuf(t, 0x4E), out = u + shuf(u, 0xB1).
// Exit: EAX = &out, EDX = &b, XMM0 = u, XMM1 = result.

unsafe fn dot4_op(a: usize, out: usize, b: usize) -> (__m128, __m128) {
    unsafe {
        let va = lda(a);
        let vb = lda(b);
        let t = _mm_mul_ps(vb, va);
        let u = _mm_add_ps(_mm_shuffle_ps::<0x4E>(t, t), t);
        let w = _mm_add_ps(_mm_shuffle_ps::<0xB1>(u, u), u);
        sta(out, w);
        (u, w)
    }
}

extern "C" fn dot4_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (a, out, b) = (r.ecx as usize, unsafe { *args }, unsafe { *args.add(1) });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(DOT4, r, &[out, b], &[(out as usize, 16)], M_FMEM, || {
            o = dot4_op(a, out as usize, b as usize);
            Res::default()
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = out;
    r.edx = b;
}
thunk_xo!(dot4_thunk, dot4_entry, "ret 8");

// --- 0x71A0A0 hkvector4_madd_splat_x ----------------------------------------------------------------
// ECX = &out, stack (&s, &v), ret 8: out = splat(s.x) * v + out.
// Exit: EAX = &out, ECX = &s, EDX = &v, XMM0 = the old out, XMM1 = result.

unsafe fn madd_op(out: usize, s: usize, v: usize) -> (__m128, __m128) {
    unsafe {
        let vs = lda(s);
        let sp = _mm_shuffle_ps::<0>(vs, vs);
        let vv = lda(v);
        let prod = _mm_mul_ps(sp, vv);
        let old = lda(out);
        let res = _mm_add_ps(prod, old);
        sta(out, res);
        (old, res)
    }
}

extern "C" fn madd_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (out, s, v) = (r.ecx as usize, unsafe { *args }, unsafe { *args.add(1) });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(MADD, r, &[s, v], &[(out, 16)], M_EAX | M_FMEM, || {
            o = madd_op(out, s as usize, v as usize);
            Res { eax: out as u32, ..Res::default() }
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = out as u32;
    r.ecx = s;
    r.edx = v;
}
thunk_xo!(madd_thunk, madd_entry, "ret 8");

// --- 0x485D30 hkvector4_add_in_place ----------------------------------------------------------------
// ECX = &a, stack (&b), ret 4: a = a + b.
// Exit: EAX = &a, ECX = &b, XMM0 = result, XMM1 = b.

unsafe fn add_op(a: usize, b: usize) -> (__m128, __m128) {
    unsafe {
        let va = lda(a);
        let vb = lda(b);
        let res = _mm_add_ps(va, vb);
        sta(a, res);
        (res, vb)
    }
}

extern "C" fn add_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (a, b) = (r.ecx as usize, unsafe { *args });
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(ADD, r, &[b], &[(a, 16)], M_EAX | M_FMEM, || {
            o = add_op(a, b as usize);
            Res { eax: a as u32, ..Res::default() }
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = a as u32;
    r.ecx = b;
}
thunk_xo!(add_thunk, add_entry, "ret 4");

// --- 0xBB8970 hkvector4_and_mask_table --------------------------------------------------------------
// ECX = &v, stack (index), ret 4: v = row(0xE538F0 + index*16) & v.
// Exit: EAX = the row address, XMM0 = result, XMM1 = the old v.

unsafe fn andm_op(v: usize, row: usize) -> (__m128, __m128) {
    unsafe {
        let old = lda(v);
        let m = lda(row);
        let res = _mm_and_ps(m, old);
        sta(v, res);
        (res, old)
    }
}

extern "C" fn andm_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (v, idx) = (r.ecx as usize, unsafe { *args });
    let row = k(MASK_TABLE).wrapping_add((idx << 4) as usize);
    let mut o = (unsafe { _mm_setzero_ps() }, unsafe { _mm_setzero_ps() });
    unsafe {
        shadowed(ANDM, r, &[idx], &[(v, 16)], 0, || {
            o = andm_op(v, row);
            Res::default()
        });
        xo_set(xo, 0, o.0);
        xo_set(xo, 1, o.1);
    }
    r.eax = row as u32;
}
thunk_xo!(andm_thunk, andm_entry, "ret 4");

// --- 0xD93F20 hkrotation_from_quaternion_store ------------------------------------------------------
// ECX = &out (three 16-byte rows), stack (&quat), ret 4. Intrinsic-for-intrinsic translation of the
// SSE body (constants: 1.0 row at 0xDC9C20, sign rows at 0xE57460 / 0xE57470). The original clobbers
// XMM0-7 and leaves EAX = &quat; the final value of every XMM register is handed back through `xo`.
// (The original also spills the sign row to a dead 16-byte stack slot below its frame; not repeated.)

unsafe fn rot(out: usize, quat: usize) -> [__m128; 8] {
    unsafe {
        let q = lda(quat);
        let c1 = lda(k(ROT_ONE));
        let s0 = lda(k(ROT_S0));
        let d = _mm_add_ps(q, q);
        let x3 = _mm_shuffle_ps::<0x41>(q, q);
        let x2 = _mm_shuffle_ps::<0xA5>(d, d);
        let x4 = _mm_mul_ss(q, d);
        let x0 = _mm_shuffle_ps::<0xFE>(q, q);
        let x1 = _mm_shuffle_ps::<0x1A>(d, d);
        let x0 = _mm_mul_ps(x0, x1);
        let s1 = lda(k(ROT_S1));
        let x0 = _mm_xor_ps(x0, s1);
        let x2 = _mm_mul_ps(x2, x3);
        let x0 = _mm_add_ss(x0, c1);
        let x5 = x2;
        let x2 = _mm_xor_ps(x2, lda(k(ROT_S0)));
        let row0 = _mm_sub_ps(x0, x2);
        let x0 = _mm_xor_ps(x0, s0);
        let x2 = _mm_move_ss(x2, x4);
        sta(out, row0);
        let x0 = _mm_sub_ps(x0, x2);
        let x0 = _mm_shuffle_ps::<0xB1>(x0, x0);
        let x3 = _mm_sub_ss(c1, x4);
        let x1 = _mm_movehl_ps(row0, x0);
        let x3 = _mm_sub_ss(x3, x5);
        let x1 = _mm_shuffle_ps::<0x8D>(x1, x3);
        sta(out + 0x10, x0);
        sta(out + 0x20, x1);
        [x0, x1, x2, x3, x4, x5, c1, s0]
    }
}

extern "C" fn rot_entry(r: &mut Pushad, args: *const u32, xo: *mut f32) {
    let (out, quat) = (r.ecx as usize, unsafe { *args });
    let mut o = [unsafe { _mm_setzero_ps() }; 8];
    for (i, v) in o.iter_mut().enumerate() {
        *v = unsafe { _mm_loadu_ps(xo.add(i * 4)) };
    }
    unsafe {
        shadowed(ROT, r, &[quat], &[(out, 48)], M_FMEM, || {
            o = rot(out, quat as usize);
            Res::default()
        });
        for (i, v) in o.iter().enumerate() {
            xo_set(xo, i, *v);
        }
    }
    r.eax = quat;
}
thunk_xo!(rot_thunk, rot_entry, "ret 4");

// --- 0x9149A0 action_node_from_handle ---------------------------------------------------------------
// EDX = handle, plain ret, EAX = record (0x6C bytes at 0x2A4E508 + (handle & 0xFFFF) * 0x6C) or 0:
// handle 0, index >= 0x960 (signed), [rec+0x30] == 0, [[rec+0x30]+4] == 0, or [rec+0x1C] != handle
// all give 0. ECX is clobbered (the thunk preserves it).

// Exit ECX: untouched on the h == 0 / index-too-big paths, the loaded [rec+0x30] pointer on the two
// early-out paths after it, and (mismatch ? 0 : -1) on the last path.

unsafe fn action_node(h: u32, ecx: &mut u32) -> u32 {
    unsafe {
        if h == 0 {
            return 0;
        }
        let idx = h & 0xFFFF;
        if idx >= 0x960 {
            return 0;
        }
        let rec = k(ACTION_TABLE) + idx as usize * 0x6C;
        let n = rd(rec + 0x30) as usize;
        *ecx = n as u32;
        if n == 0 || rd(n + 4) == 0 {
            return 0;
        }
        let same = rd(rec + 0x1C) == h;
        *ecx = if same { 0xFFFF_FFFF } else { 0 };
        if same { rec as u32 } else { 0 }
    }
}

extern "C" fn actn_entry(r: &mut Pushad, _args: *const u32) {
    let h = r.edx;
    let mut ecx = r.ecx;
    let res = unsafe { shadowed(ACTN, r, &[], &[], M_EAX, || Res { eax: action_node(h, &mut ecx), ..Res::default() }) };
    r.eax = res.eax;
    r.ecx = ecx;
}
thunk_win!(actn_thunk, actn_entry, ACTN, "ret");

// --- 0xCF7760 world_mesh_record_set_instance_data ---------------------------------------------------
// EAX = record, ECX = value, stack (a0, &src, a2, a3), ret 0x10: [rec+0x360] = ECX, [rec+0x588] = a3,
// [rec+0x584] = a2, word [rec+0x364] = a0, 0x80 dwords from src to rec+0x368 (rep movsd), then
// [rec+0x568] = [rec+0x56C] = -1 and [rec+0x574..0x580] = 0. EAX preserved.

unsafe fn inst(rec: usize, val: u32, a0: u32, src: usize, a2: u32, a3: u32) {
    unsafe {
        wd(rec + 0x360, val);
        wd(rec + 0x588, a3);
        wd(rec + 0x584, a2);
        *((rec + 0x364) as *mut u16) = a0 as u16;
        movsd(rec + 0x368, src, 0x80);
        wd(rec + 0x568, 0xFFFF_FFFF);
        wd(rec + 0x56C, 0xFFFF_FFFF);
        wd(rec + 0x574, 0);
        wd(rec + 0x578, 0);
        wd(rec + 0x57C, 0);
    }
}

extern "C" fn inst_entry(r: &mut Pushad, args: *const u32) {
    let (rec, val) = (r.eax as usize, r.ecx);
    let (a0, src, a2, a3) = unsafe { (*args, *args.add(1), *args.add(2), *args.add(3)) };
    unsafe {
        shadowed(INST, r, &[a0, src, a2, a3], &[(rec + 0x360, 0x22C)], 0, || {
            inst(rec, val, a0, src as usize, a2, a3);
            Res::default()
        });
    }
    // Exit: ECX = 0 (`xor ecx, ecx`), EDX = a2 with its low word replaced by a0's (`mov dx, ...`).
    r.ecx = 0;
    r.edx = (a2 & 0xFFFF_0000) | (a0 & 0xFFFF);
}
thunk_win!(inst_thunk, inst_entry, INST, "ret 0x10");

// --- 0xBDEA80 vec3_dist_sq_xz_regs ------------------------------------------------------------------
// ECX = &a, EAX = &b, plain ret, ST0 = f32(dx*dx + dz*dz) with dx = f32(a.x - b.x), dz = f32(a.z - b.z)
// (each stored through the stack as f32, so the differences are rounded to f32 first). y is ignored.

unsafe fn dist_sq_xz(a: usize, b: usize) -> f32 {
    unsafe { dist_sq_dz(a, b).0 }
}

/// The squared distance and the f32 `dz` the original keeps in a stack slot.
unsafe fn dist_sq_dz(a: usize, b: usize) -> (f32, f32) {
    unsafe {
        let dx = (fx(a) - fx(b)) as f32 as f64;
        let dzf = (fx(a + 8) - fx(b + 8)) as f32;
        let dz = dzf as f64;
        ((dx * dx + dz * dz) as f32, dzf)
    }
}

extern "C" fn dsq_entry(r: &mut Pushad, _args: *const u32) -> f32 {
    let (a, b) = (r.ecx as usize, r.eax as usize);
    let res = unsafe { shadowed(DSQ, r, &[], &[], M_ST0, || Res { st0: dist_sq_xz(a, b).to_bits(), ..Res::default() }) };
    f32::from_bits(res.st0)
}
thunk_all!(dsq_thunk, dsq_entry, "ret");

// --- 0xBDEAB0 vec3_dist_xz_regs ---------------------------------------------------------------------
// Same inputs; ST0 = sqrtf_wrapper(f32(dx*dx + dz*dz)) (0x40EA20, cdecl f32 on the stack, ST0).

// Exit EAX is whatever the callee left in it; ECX / EDX come from the callee too (not compared).
unsafe fn dist_xz(a: usize, b: usize, r: &Pushad) -> (f32, u32) {
    unsafe {
        let (sum, dz) = dist_sq_dz(a, b);
        // The original calls with its whole 16-byte frame on the stack: sum, an unwritten slot, dz, sum.
        let (v, eax) = call_st0e(SQRTF, b as u32, a as u32, r.edx, r.esi, r.edi, &[sum.to_bits(), 0, dz.to_bits(), sum.to_bits()]);
        (v as f32, eax)
    }
}

extern "C" fn dxz_entry(r: &mut Pushad, _args: *const u32) -> f32 {
    let (a, b) = (r.ecx as usize, r.eax as usize);
    let mut eax = r.eax;
    let res = unsafe {
        shadowed(DXZ, r, &[], &[], M_ST0, || {
            let (v, e) = dist_xz(a, b, r);
            eax = e;
            Res { st0: v.to_bits(), ..Res::default() }
        })
    };
    r.eax = eax;
    f32::from_bits(res.st0)
}
thunk_all!(dxz_thunk, dxz_entry, "ret");

// --- 0x40EA60 vec3_length_sq_eax --------------------------------------------------------------------
// EAX = &v, plain ret, ST0 = f32((x*x + y*y) + z*z) (x87, one f32 store). ECX preserved.

unsafe fn length_sq(v: usize) -> f32 {
    unsafe {
        let (x, y, z) = (fx(v), fx(v + 4), fx(v + 8));
        ((x * x + y * y) + z * z) as f32
    }
}

extern "C" fn lsq_entry(r: &mut Pushad, _args: *const u32) -> f32 {
    let v = r.eax as usize;
    let res = unsafe { shadowed(LSQ, r, &[], &[], M_ST0, || Res { st0: length_sq(v).to_bits(), ..Res::default() }) };
    // `fstp dword [esp]` overwrites the pushed ECX; the final `pop ecx` leaves the result bits there.
    r.ecx = res.st0;
    f32::from_bits(res.st0)
}
thunk_all!(lsq_thunk, lsq_entry, "ret");

// --- 0xBE2C60 mat33_copy_regs -----------------------------------------------------------------------
// ECX = &src, EAX = &dst, plain ret: nine `fld dword` / `fstp dword` pairs in order (a signalling NaN
// comes out quiet). Nothing clobbered.

unsafe fn m33_copy(src: usize, dst: usize) {
    unsafe {
        for i in 0..9 {
            let v = std::ptr::read_volatile((src + i * 4) as *const u32);
            std::ptr::write_volatile((dst + i * 4) as *mut u32, q(v));
        }
    }
}

extern "C" fn m33_entry(r: &mut Pushad, _args: *const u32) {
    let (src, dst) = (r.ecx as usize, r.eax as usize);
    unsafe {
        shadowed(M33, r, &[], &[(dst, 36)], 0, || {
            m33_copy(src, dst);
            Res::default()
        });
    }
}
thunk_all!(m33_thunk, m33_entry, "ret");

// --- 0xAB5190 vehicle_is_helicopter -----------------------------------------------------------------
// EAX = vehicle or 0, plain ret, AL = ([[veh+0x84E4]+0x9C] == 3); EAX's upper bits are the class-info
// pointer's (the original does `mov al, cl` into the loaded pointer). ECX clobbered (preserved here).

unsafe fn is_heli(veh: u32) -> u32 {
    unsafe {
        if veh == 0 {
            return 0;
        }
        let info = rd(veh as usize + 0x84E4);
        (info & 0xFFFF_FF00) | (rd(info as usize + 0x9C) == 3) as u32
    }
}

extern "C" fn heli_entry(r: &mut Pushad, _args: *const u32) {
    let veh = r.eax;
    let res = unsafe { shadowed(HELI, r, &[], &[], M_EAX, || Res { eax: is_heli(veh), ..Res::default() }) };
    r.eax = res.eax;
    // The original leaves the answer in ECX too (`xor ecx, ecx; sete cl`) and the helicopter AI's
    // caller reads it: with ECX left as the caller's, police helicopters stopped tracking the player
    // (play test 2026-11-26). The null path leaves ECX alone, like the original.
    if veh != 0 {
        r.ecx = res.eax & 1;
    }
}
thunk_win!(heli_thunk, heli_entry, HELI, "ret");

// --- 0xA96A70 vehicle_transform_store ---------------------------------------------------------------
// EAX = vehicle, EDX = &pos (3 dwords), stack (&mat33), ret 4: matrix to veh+0xEC (9 dwords), pos to
// veh+0x110, then id = [veh+0xA4]: 0 or > 0x80 (unsigned) stops, else slot = 0x324F7C4 + id*0x4C gets
// pos at +4 and the matrix at +0x10. Dword copies, no float conversion. ECX / EDX end up clobbered in
// the original (EDX = pos.z on the slot path), the thunk preserves them.

// Returns the exit (EAX, ECX, EDX): early path (id 0 or > 0x80) EAX = id, ECX = pos.z, EDX = pos; slot
// path EAX = slot, ECX = 0 (rep movsd), EDX = pos.z.
unsafe fn xform_store(veh: usize, pos: usize, mat: usize) -> (u32, u32, u32) {
    unsafe {
        movsd(veh + 0xEC, mat, 9);
        wd(veh + 0x110, rd(pos));
        wd(veh + 0x114, rd(pos + 4));
        wd(veh + 0x118, rd(pos + 8));
        let id = rd(veh + 0xA4);
        if id == 0 || id > 0x80 {
            return (id, rd(pos + 8), pos as u32);
        }
        let slot = k(SLOT_TABLE) + id as usize * 0x4C;
        wd(slot + 4, rd(pos));
        wd(slot + 8, rd(pos + 4));
        let z = rd(pos + 8);
        wd(slot + 0xC, z);
        movsd(slot + 0x10, mat, 9);
        (slot as u32, 0, z)
    }
}

extern "C" fn xfs_entry(r: &mut Pushad, args: *const u32) {
    let (veh, pos, mat) = (r.eax as usize, r.edx as usize, unsafe { *args });
    unsafe {
        let id = rd(veh + 0xA4);
        let rg = [(veh + 0xEC, 48usize), (k(SLOT_TABLE) + id as usize * 0x4C + 4, 0x30usize)];
        let n = if id == 0 || id > 0x80 { 1 } else { 2 };
        let mut ex = (r.eax, r.ecx, r.edx);
        shadowed(XFS, r, &[mat], &rg[..n], 0, || {
            ex = xform_store(veh, pos, mat as usize);
            Res::default()
        });
        r.eax = ex.0;
        r.ecx = ex.1;
        r.edx = ex.2;
    }
}
thunk_win!(xfs_thunk, xfs_entry, XFS, "ret 4");

// --- 0xD22930 view_clip_planes_sphere_outside -------------------------------------------------------
// EAX = plane count, ECX = planes (0x1C bytes: normal at +4/+8/+0xC, d at +0x10), stack (&center,
// f32 radius), plain ret. gfx_get_thread_state() (0xD22790, EAX = state) is called first; ECX == 0
// takes the state's list (state+0x15DC, count state+0x1684, signed). do-while over the planes: dist =
// f32(f32((nx*cx + ny*cy) + cz*nz) + d); `fcomp radius; test ah, 0x41; je` -> AL = 1 when dist > radius
// (ordered; NaN does not exit), all planes passing -> AL = 0. The loop body runs once even for a count
// of 0 or less (do-while, signed `jl`). The original leaves the last f32 distance in its first stack
// argument slot and EAX = (state & 0xFFFF0000) | (x87 status high byte << 8) | AL, the status being
// the compare's C0/C2/C3 plus TOP = 7 (an empty stack on entry).

// Exit ECX = start + 0xC + 0x1C * EDX and EDX = the number of planes stepped past (the loop counter).
unsafe fn sphere(count: i32, planes: usize, edx0: u32, center: usize, radius: u32, slot: *mut u32) -> (u32, u32, u32) {
    unsafe {
        let state = cr(THREAD_STATE, count as u32, planes as u32, edx0, planes as u32, count as u32, &[]);
        let (mut n, mut p) = (count, planes);
        if planes == 0 {
            n = rd(state as usize + 0x1684) as i32;
            p = state as usize + 0x15DC;
        }
        let start = p;
        let rad = f32::from_bits(radius) as f64;
        let (cx, cy, cz) = (fx(center), fx(center + 4), fx(center + 8));
        let mut i = 0i32;
        loop {
            let dot = ((fx(p + 4) * cx + fx(p + 8) * cy) + cz * fx(p + 0xC)) as f32;
            let dist = (dot as f64 + fx(p + 0x10)) as f32;
            *slot = dist.to_bits();
            let dd = dist as f64;
            let cc: u32 = if dd > rad {
                0x00
            } else if dd < rad {
                0x01
            } else if dd == rad {
                0x40
            } else {
                0x45
            };
            let hi = (state & 0xFFFF_0000) | ((cc | 0x38) << 8);
            if cc & 0x41 == 0 {
                return (hi | 1, (start + 0xC + i as usize * 0x1C) as u32, i as u32);
            }
            i += 1;
            p += 0x1C;
            if i >= n {
                return (hi, (start + 0xC + i as usize * 0x1C) as u32, i as u32);
            }
        }
    }
}

extern "C" fn sph_entry(r: &mut Pushad, args: *const u32) {
    let (count, planes) = (r.eax as i32, r.ecx as usize);
    let (center, radius) = unsafe { (*args as usize, *args.add(1)) };
    let slot = args as *mut u32;
    let edx0 = r.edx;
    let mut ex = (0u32, 0u32, 0u32);
    let res = unsafe {
        shadowed(SPH, r, &[center as u32, radius], &[], M_AL, || {
            ex = sphere(count, planes, edx0, center, radius, slot);
            Res { eax: ex.0, ..Res::default() }
        })
    };
    r.eax = res.eax;
    r.ecx = ex.1;
    r.edx = ex.2;
}
thunk_all!(sph_thunk, sph_entry, "ret");

// --- 0x40B8D0 rng_chance_f32 ------------------------------------------------------------------------
// Stack (f32 p), plain ret, EAX = 1 or 0: r = rng_float01(0xEC30F8) (0xBDCE30, EAX = generator, ST0),
// `fld p; fcompp; test ah, 1; jne zero`: C0 is set when p < r and when unordered, so the result is
// (p >= r) with NaN giving 0. The callee advances the generator, so there is no shadow; the two
// result branches are counted over the first SHADOW_CALLS calls.

static RNG_YES: AtomicU32 = AtomicU32::new(0);
static RNG_NO: AtomicU32 = AtomicU32::new(0);

unsafe fn rng_chance(pbits: u32) -> u32 {
    unsafe {
        let r = call_st0(RNG_FLOAT01, k(RNG_GEN) as u32, 0, 0, 0, 0, &[]);
        let p = f32::from_bits(pbits) as f64;
        (p >= r) as u32
    }
}

extern "C" fn rng_entry(r: &mut Pushad, args: *const u32) {
    let res = unsafe { rng_chance(*args) };
    let c = &PORTS[RNG].calls;
    if c.load(Ordering::Relaxed) < SHADOW_CALLS {
        c.fetch_add(1, Ordering::Relaxed);
        (if res != 0 { &RNG_YES } else { &RNG_NO }).fetch_add(1, Ordering::Relaxed);
    }
    r.eax = res;
}
thunk_all!(rng_thunk, rng_entry, "ret");

// --- 0xBF0EA0 f32_round_to_int ----------------------------------------------------------------------
// Stack (f32 x), plain ret, EAX = trunc(x >= 0 ? x + 0.5 : x - 0.5) through `_ftol2_sse` (0xDA0140:
// cvttsd2si when the flag at 0x34A536C is set, so overflow and NaN give 0x80000000), XMM0 = x (movss).
// `comiss xmm0, 0.0; jb` takes the subtract branch for x < 0 and for NaN. The 0.5 add is exact for
// every |x| < 2^31, so the x87 precision / rounding control cannot change the result.

unsafe fn round_int(xbits: u32) -> u32 {
    unsafe {
        let x = f32::from_bits(xbits);
        let v = if x >= 0.0 { x as f64 + 0.5 } else { x as f64 - 0.5 };
        if rd(k(FTOL_FLAG)) != 0 {
            return _mm_cvttsd_si32(_mm_set_sd(v)) as u32;
        }
        // the CRT's x87 fallback: ST0 = v, the routine pops it
        let r: u32;
        std::arch::asm!(
            "fld qword ptr [{p}]",
            "call {f}",
            p = in(reg) &v as *const f64,
            f = in(reg) tgt(FTOL2),
            lateout("eax") r,
            clobber_abi("C"),
        );
        r
    }
}

extern "C" fn round_entry(r: &mut Pushad, args: *const u32) {
    let x = unsafe { *args };
    let res = unsafe { shadowed(ROUND, r, &[x], &[], M_EAX | M_XMM0, || Res { eax: round_int(x), xmm0: x, st0: 0 }) };
    r.eax = res.eax;
    r.esp = x;
}
thunk_x1_out0!(round_thunk, round_entry, "ret");

// --- install ----------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    // (port, original body, its call offsets, thunk)
    let list: [(usize, &[u8], &[usize], usize); COUNT] = [
        (SCALE, &SCALE_BODY, &SCALE_BODY_CALLS, scale_thunk as *const () as usize),
        (SUB, &SUB_BODY, &SUB_BODY_CALLS, sub_thunk as *const () as usize),
        (EQ, &EQ_BODY, &EQ_BODY_CALLS, eq_thunk as *const () as usize),
        (ACTN, &ACTN_BODY, &ACTN_BODY_CALLS, actn_thunk as *const () as usize),
        (INST, &INST_BODY, &INST_BODY_CALLS, inst_thunk as *const () as usize),
        (DSQ, &DSQ_BODY, &DSQ_BODY_CALLS, dsq_thunk as *const () as usize),
        (XFS, &XFS_BODY, &XFS_BODY_CALLS, xfs_thunk as *const () as usize),
        (SPH, &SPH_BODY, &SPH_BODY_CALLS, sph_thunk as *const () as usize),
        (HELI, &HELI_BODY, &HELI_BODY_CALLS, heli_thunk as *const () as usize),
        (LSQ, &LSQ_BODY, &LSQ_BODY_CALLS, lsq_thunk as *const () as usize),
        (M33, &M33_BODY, &M33_BODY_CALLS, m33_thunk as *const () as usize),
        (DXZ, &DXZ_BODY, &DXZ_BODY_CALLS, dxz_thunk as *const () as usize),
        (RNG, &RNG_BODY, &RNG_BODY_CALLS, rng_thunk as *const () as usize),
        (ROUND, &ROUND_BODY, &ROUND_BODY_CALLS, round_thunk as *const () as usize),
        (DOT4, &DOT4_BODY, &DOT4_BODY_CALLS, dot4_thunk as *const () as usize),
        (LESS, &LESS_BODY, &LESS_BODY_CALLS, less_thunk as *const () as usize),
        (MADD, &MADD_BODY, &MADD_BODY_CALLS, madd_thunk as *const () as usize),
        (ADD, &ADD_BODY, &ADD_BODY_CALLS, add_thunk as *const () as usize),
        (ANDM, &ANDM_BODY, &ANDM_BODY_CALLS, andm_thunk as *const () as usize),
        (ROT, &ROT_BODY, &ROT_BODY_CALLS, rot_thunk as *const () as usize),
    ];
    let mut ok = 0;
    for (id, body, calls, thunk) in list {
        let port = &PORTS[id];
        if id != RNG {
            // The shadow copy first: replacing patches the entry.
            match unsafe { detour::relocate(port.addr, body, calls) } {
                Ok(copy) => {
                    if id == ROUND {
                        // the two tail `jmp rel32` to _ftol2_sse at +0x19 and +0x24
                        for off in [0x19usize, 0x24] {
                            let rel = (FTOL2 as i32).wrapping_sub((copy + off + 5) as i32);
                            unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
                        }
                    }
                    port.original.store(copy, Ordering::Relaxed)
                }
                Err(e) => log(&format!("port_hot60: {}: no shadow copy: {e:?}", port.name)),
            }
        }
        if replace_pinned(port.name, port.addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_hot60: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log (the first `SHADOW_CALLS` calls are counted).
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            if c == 0 {
                return None;
            }
            if p.addr == 0x0040_B8D0 {
                return Some(format!(
                    "{} {c} (true {}, false {})",
                    p.name,
                    RNG_YES.load(Ordering::Relaxed),
                    RNG_NO.load(Ordering::Relaxed)
                ));
            }
            Some(format!("{} {c} ({} shadow mismatches)", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_hot60: {}", parts.join(", "))
}


// =================================================================================================
// Offline differential test
// =================================================================================================
//
// Each test runs the ORIGINAL bytes (copied to executable memory; their calls aimed at test copies
// of the game's callee functions, their absolute data references at test statics) and the Rust
// port through its real thunk on the same random world, twice: once with the shadow window open
// (the first 3000 calls run the relocated original inside the thunk too, and must not mismatch) and
// once with the window closed (the bare Rust body). It compares EAX / ST0 / XMM0 where they are
// results, the x87 depth, every register the original preserves, the XMM registers the thunk must
// keep, and every word of the arena (and of the test statics the function writes).
//
// Run (one test per process):
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc <name> -- --ignored --test-threads=1 --nocapture
// (CASES=n changes the number of random cases per phase, default 4000.)

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

    fn redirect_set(a: usize, to: usize) {
        let mut r = T_REDIR.lock().unwrap();
        r.retain(|e| e.0 != a);
        r.push((a, to));
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

    // --- arena ------------------------------------------------------------------------------------

    const AW: usize = 16384;

    #[repr(C, align(16))]
    struct Arena([u32; AW]);

    fn new_arena() -> Box<Arena> {
        Box::new(Arena([0; AW]))
    }

    fn ptr(a: &Arena, word: usize) -> u32 {
        a.0.as_ptr() as u32 + (word as u32) * 4
    }

    fn fill(a: &mut Arena, rng: &mut Rng, f: impl Fn(&mut Rng) -> u32) {
        for w in a.0.iter_mut() {
            *w = f(rng);
        }
    }

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    fn same_f(a: u32, b: u32) -> bool {
        a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
    }

    fn st0_f32(t: &TB) -> u32 {
        (f64::from_bits(t.st0) as f32).to_bits()
    }

    fn grab(a: &Arena, extra: &[(usize, usize)]) -> Vec<u32> {
        let mut v = a.0.to_vec();
        for &(p, l) in extra {
            for i in 0..l / 4 {
                v.push(unsafe { *((p + i * 4) as *const u32) });
            }
        }
        v
    }

    fn put(a: &mut Arena, extra: &[(usize, usize)], v: &[u32]) {
        a.0.copy_from_slice(&v[..AW]);
        let mut o = AW;
        for &(p, l) in extra {
            for i in 0..l / 4 {
                unsafe { *((p + i * 4) as *mut u32) = v[o] };
                o += 1;
            }
        }
    }

    unsafe fn both_raw(orig: usize, thunk: usize, arena: &mut Arena, extra: &[(usize, usize)], r: In, args: &[u32]) -> (TB, Vec<u32>, TB, Vec<u32>) {
        unsafe {
            let snap = grab(arena, extra);
            let o = call_t(orig, r, args);
            let a1 = grab(arena, extra);
            put(arena, extra, &snap);
            let t = call_t(thunk, r, args);
            let a2 = grab(arena, extra);
            put(arena, extra, &snap);
            (o, a1, t, a2)
        }
    }

    /// What the thunk must satisfy besides the memory comparison.
    #[derive(Clone, Copy, Default)]
    struct Spec {
        ecx: bool,
        edx: bool,
        eax_keep: bool,
        /// XMM registers (bit i = xmm i) the thunk must leave alone.
        xk: u8,
        /// Bits of EAX that are a result.
        eax_mask: u32,
        st0: bool,
        xmm0: bool,
        fmem: bool,
    }

    fn check(o: &TB, t: &TB, a1: &[u32], a2: &[u32], r: In, sp: &Spec) -> Result<(), String> {
        if t.o_ebx != EBX_IN || t.o_esi != r.esi || t.o_edi != r.edi || t.o_ebp != EBP_IN {
            return Err(format!("registers changed: ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}", t.o_ebx, t.o_esi, t.o_edi, t.o_ebp));
        }
        if o.o_ebx != EBX_IN || o.o_esi != r.esi || o.o_edi != r.edi || o.o_ebp != EBP_IN {
            return Err("the ORIGINAL changed a register the thunk is assumed to preserve".into());
        }
        if sp.ecx && t.o_ecx != r.ecx {
            return Err(format!("ecx {:#x} changed", t.o_ecx));
        }
        if sp.edx && t.o_edx != r.edx {
            return Err(format!("edx {:#x} changed", t.o_edx));
        }
        if sp.eax_keep && t.o_eax != r.eax {
            return Err(format!("eax {:#x} changed", t.o_eax));
        }
        let xp = xmm_pattern();
        for i in 0..8 {
            if sp.xk >> i & 1 != 0 && t.xmm[i * 4..i * 4 + 4] != xp[i * 4..i * 4 + 4] {
                return Err(format!("xmm{i} changed"));
            }
        }
        let od = ((o.top0 & 0x3800).wrapping_sub(o.top1 & 0x3800) & 0x3800) >> 11;
        let td = ((t.top0 & 0x3800).wrapping_sub(t.top1 & 0x3800) & 0x3800) >> 11;
        if od != td || td != sp.st0 as u32 {
            return Err(format!("x87 depth: original {od} Rust {td}"));
        }
        if sp.eax_mask != 0 && (o.o_eax ^ t.o_eax) & sp.eax_mask != 0 {
            return Err(format!("eax original {:#x} Rust {:#x}", o.o_eax, t.o_eax));
        }
        if sp.st0 && !same_f(st0_f32(o), st0_f32(t)) {
            return Err(format!("st0 original {:#x} Rust {:#x}", st0_f32(o), st0_f32(t)));
        }
        if sp.xmm0 && o.xmm[0..4] != t.xmm[0..4] {
            return Err(format!("xmm0 original {:x?} Rust {:x?}", &o.xmm[0..4], &t.xmm[0..4]));
        }
        for (i, (x, y)) in a1.iter().zip(a2).enumerate() {
            let ok = if sp.fmem { same_f(*x, *y) } else { x == y };
            if !ok {
                return Err(format!("memory word {i}: original {x:#x} Rust {y:#x}"));
            }
        }
        Ok(())
    }

    macro_rules! check_case {
        ($name:expr, $i:expr, $bad:ident, $e:expr) => {
            if let Err(m) = $e {
                $bad += 1;
                if $bad <= 5 {
                    eprintln!("{} case {}: {}", $name, $i, m);
                }
            }
        };
    }

    fn inn(rng: &mut Rng, eax: u32) -> In {
        In { eax, ecx: rng.u32(), edx: rng.u32(), esi: 0x5151_5151, edi: 0xD1D1_D1D1 }
    }

    /// Both phases (shadow window open, then closed) over `cases()` random worlds.
    unsafe fn drive(
        name: &str,
        id: usize,
        orig: usize,
        thunk: usize,
        sp: Spec,
        extra: &[(usize, usize)],
        arena: &mut Arena,
        seed: u64,
        gen_case: &mut dyn FnMut(&mut Rng, &mut Arena, u32) -> (In, Vec<u32>),
    ) {
        let n = cases();
        let mut bad = 0u32;
        let mut nonzero = 0u32;
        for phase in 0..2u32 {
            PORTS[id].original.store(orig, Ordering::Relaxed);
            PORTS[id].calls.store(if phase == 0 { 0 } else { SHADOW_CALLS }, Ordering::Relaxed);
            PORTS[id].mismatches.store(0, Ordering::Relaxed);
            let mut rng = Rng(seed.wrapping_add(phase as u64 * 0x9E37_79B9_7F4A_7C15) | 1);
            for i in 0..n {
                let (r, args) = gen_case(&mut rng, arena, i);
                let (o, a1, t, a2) = unsafe { both_raw(orig, thunk, arena, extra, r, &args) };
                check_case!(name, i, bad, check(&o, &t, &a1, &a2, r, &sp));
                nonzero += (o.o_eax & sp.eax_mask & 0xFF != 0) as u32;
            }
            if phase == 0 && id != RNG {
                let m = PORTS[id].mismatches.load(Ordering::Relaxed);
                eprintln!("{name}: shadow ran {} calls, {m} mismatches", PORTS[id].calls.load(Ordering::Relaxed));
                assert_eq!(m, 0, "the in-game shadow found a mismatch");
            }
        }
        eprintln!("{name}: {} cases, {bad} differ ({nonzero} with a nonzero AL result)", 2 * n);
        assert_eq!(bad, 0, "{bad} of {} cases differ", 2 * n);
    }

    fn sc_pick(rng: &mut Rng) -> f64 {
        [1.0, 10.0, 100.0, 1e4, 1e8][rng.below(5) as usize]
    }

    // --- 0x40B790 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn scale_matches_original() {
        let orig = build(&SCALE_BODY, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk: 0xFF, fmem: true, ..Spec::default() };
        let base = ptr(&arena, 64);
        unsafe {
            drive("scale", SCALE, orig, scale_thunk as *const () as usize, sp, &[], &mut arena, 0x40B7_9001, &mut |rng, a, _| {
                let sc = sc_pick(rng);
                fill(a, rng, |r| r.fin(sc));
                let mut s = rng.fin(sc);
                if rng.below(25) == 0 {
                    let w = rng.below(4);
                    let v = rng.any();
                    if w == 3 { s = v } else { a.0[64 + w as usize] = v }
                }
                (inn(rng, base), vec![s])
            });
        }
    }

    // --- the Havok vector family ------------------------------------------------------------------

    /// kind 0: ECX = out, (a, b); 1: ECX = a, (out, b); 2: ECX = out, (s, v); 3: ECX = a, (b).
    fn vec_test(name: &'static str, id: usize, body: &[u8], thunk: usize, kind: u32, eax_result: bool) {
        let orig = build(body, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, xk: 0xFE, fmem: true, eax_mask: if eax_result { !0 } else { 0 }, ..Spec::default() };
        let (p1, p2, p3) = (ptr(&arena, 64), ptr(&arena, 96), ptr(&arena, 128));
        unsafe {
            drive(name, id, orig, thunk, sp, &[], &mut arena, 0x7A00_0000 + id as u64, &mut |rng, a, _| {
                let sc = sc_pick(rng);
                fill(a, rng, |r| r.fin(sc));
                // equal / ordered lanes for the compares
                for l in 0..4 {
                    match rng.below(5) {
                        0 => a.0[96 + l] = a.0[64 + l],
                        1 => a.0[96 + l] = a.0[64 + l] ^ 0x8000_0000,
                        _ => {}
                    }
                    if rng.below(30) == 0 {
                        a.0[64 + l] = rng.any();
                    }
                    if rng.below(30) == 0 {
                        a.0[96 + l] = rng.any();
                    }
                    if rng.below(30) == 0 {
                        a.0[128 + l] = rng.any();
                    }
                }
                let (pa, mut pb, mut pc) = (p1, p2, p3);
                match rng.below(8) {
                    0 => pc = pa,
                    1 => pc = pb,
                    2 => pb = pa,
                    _ => {}
                }
                match kind {
                    0 => (inn(rng, 0), vec![pa, pb]).with_ecx(pc),
                    1 => (inn(rng, 0), vec![pc, pb]).with_ecx(pa),
                    2 => (inn(rng, 0), vec![pa, pb]).with_ecx(pc),
                    _ => (inn(rng, 0), vec![pb]).with_ecx(pa),
                }
            });
        }
    }

    trait WithEcx {
        fn with_ecx(self, ecx: u32) -> Self;
    }
    impl WithEcx for (In, Vec<u32>) {
        fn with_ecx(mut self, ecx: u32) -> Self {
            self.0.ecx = ecx;
            self
        }
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn sub_matches_original() {
        vec_test("sub", SUB, &SUB_BODY, sub_thunk as *const () as usize, 0, false);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn eq_matches_original() {
        vec_test("eq", EQ, &EQ_BODY, eq_thunk as *const () as usize, 1, false);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn less_matches_original() {
        vec_test("less", LESS, &LESS_BODY, less_thunk as *const () as usize, 1, false);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn dot4_matches_original() {
        vec_test("dot4", DOT4, &DOT4_BODY, dot4_thunk as *const () as usize, 1, false);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn madd_matches_original() {
        vec_test("madd", MADD, &MADD_BODY, madd_thunk as *const () as usize, 2, true);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn add_matches_original() {
        vec_test("add", ADD, &ADD_BODY, add_thunk as *const () as usize, 3, true);
    }

    // --- 0xBB8970 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn andm_matches_original() {
        let masks = std::ptr::addr_of_mut!(T_MASKS) as *mut u32;
        // the image's first four rows: all ones except lane i
        for r in 0..16usize {
            for l in 0..4usize {
                let v = if r < 4 { if l == r { 0 } else { 0xFFFF_FFFF } } else { 0x9E37_79B9u32.wrapping_mul((r * 4 + l) as u32 + 1) };
                unsafe { *masks.add(r * 4 + l) = v };
            }
        }
        let orig = build(&ANDM_BODY, &[], &[(0x00E5_38F0, masks as u32)]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, xk: 0xFE, ..Spec::default() };
        let p1 = ptr(&arena, 64);
        unsafe {
            drive("andm", ANDM, orig, andm_thunk as *const () as usize, sp, &[], &mut arena, 0xBB89_7001, &mut |rng, a, _| {
                fill(a, rng, |r| r.u32());
                let idx = if rng.below(4) == 0 { rng.below(16) } else { rng.below(4) };
                (inn(rng, 0).with_ecx_in(p1), vec![idx])
            });
        }
    }

    trait WithEcxIn {
        fn with_ecx_in(self, ecx: u32) -> In;
    }
    impl WithEcxIn for In {
        fn with_ecx_in(mut self, ecx: u32) -> In {
            self.ecx = ecx;
            self
        }
    }

    // --- 0xD93F20 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn rot_matches_original() {
        let c1 = std::ptr::addr_of!(T_C1) as u32;
        let s0 = std::ptr::addr_of!(T_S0) as u32;
        let s1 = std::ptr::addr_of!(T_S1) as u32;
        let orig = build(&ROT_BODY, &[], &[(0x00DC_9C20, c1), (0x00E5_7460, s0), (0x00E5_7470, s1)]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, xk: 0, fmem: true, ..Spec::default() };
        let (pq, po) = (ptr(&arena, 64), ptr(&arena, 128));
        unsafe {
            drive("rot", ROT, orig, rot_thunk as *const () as usize, sp, &[], &mut arena, 0xD93F_2001, &mut |rng, a, _| {
                let sc = [1.0, 2.0, 100.0][rng.below(3) as usize];
                fill(a, rng, |r| r.fin(sc));
                if rng.below(4) == 0 {
                    // a unit-ish quaternion
                    let v: Vec<f64> = (0..4).map(|_| rng.f01() * 2.0 - 1.0).collect();
                    let n = v.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-9);
                    for l in 0..4 {
                        a.0[64 + l] = ((v[l] / n) as f32).to_bits();
                    }
                }
                if rng.below(30) == 0 {
                    a.0[64 + rng.below(4) as usize] = rng.any();
                }
                let out = if rng.below(8) == 0 { pq } else { po };
                (inn(rng, 0).with_ecx_in(out), vec![pq])
            });
        }
    }

    // --- 0x9149A0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn actn_matches_original() {
        let table = std::ptr::addr_of_mut!(T_ACTION) as usize;
        let orig = build(&ACTN_BODY, &[], &[(0x02A4_E508, table as u32)]);
        let mut arena = new_arena();
        let mut rng0 = Rng(0x9149_A0A0_1234_5671);
        for i in 0..0x960usize {
            let rec = table + i * 0x6C;
            unsafe {
                for w in 0..27 {
                    *((rec + w * 4) as *mut u32) = rng0.u32();
                }
                let h = if rng0.below(10) < 6 { (rng0.u32() & 0xFFFF_0000) | i as u32 } else { rng0.u32() };
                *((rec + 0x1C) as *mut u32) = h;
                let n = match rng0.below(6) {
                    0 => 0,
                    _ => ptr(&arena, 1000 + rng0.below(2000) as usize),
                };
                *((rec + 0x30) as *mut u32) = n;
            }
        }
        let sp = Spec { ecx: true, edx: true, eax_mask: !0, xk: 0xFF, ..Spec::default() };
        unsafe {
            drive("actn", ACTN, orig, actn_thunk as *const () as usize, sp, &[], &mut arena, 0x9149_A001, &mut |rng, a, _| {
                fill(a, rng, |r| if r.below(3) == 0 { 0 } else { r.u32() });
                let i = rng.below(0x960) as usize;
                let stored = *((table + i * 0x6C + 0x1C) as *const u32);
                let h = match rng.below(10) {
                    0 => 0,
                    1 => {
                        let hi = rng.u32() & 0xFFFF_0000;
                        hi | (0x960 + rng.below(0x6000))
                    }
                    2 | 3 => (rng.u32() & 0xFFFF_0000) | i as u32,
                    _ => stored,
                };
                let e = rng.u32();
                let mut r = inn(rng, e);
                r.edx = h;
                (r, vec![])
            });
        }
    }

    // --- 0xAB5190 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn heli_matches_original() {
        let orig = build(&HELI_BODY, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_mask: !0, xk: 0xFF, ..Spec::default() };
        let veh = ptr(&arena, 100);
        unsafe {
            drive("heli", HELI, orig, heli_thunk as *const () as usize, sp, &[], &mut arena, 0xAB51_9001, &mut |rng, a, _| {
                fill(a, rng, |r| r.u32());
                let iw = 9000 + rng.below(1000) as usize;
                a.0[100 + 0x84E4 / 4] = ptr(a, iw);
                a.0[iw + 0x9C / 4] = if rng.below(2) == 0 { 3 } else { rng.below(6) };
                let e = if rng.below(8) == 0 { 0 } else { veh };
                let r = inn(rng, e);
                (r, vec![])
            });
        }
    }

    // --- 0xBE2C60 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn m33_matches_original() {
        let orig = build(&M33_BODY, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk: 0xFF, ..Spec::default() };
        unsafe {
            drive("m33", M33, orig, m33_thunk as *const () as usize, sp, &[], &mut arena, 0xBE2C_6001, &mut |rng, a, _| {
                fill(a, rng, |r| if r.below(6) == 0 { r.any() } else { r.u32() });
                let src = ptr(a, 64);
                let dst = match rng.below(8) {
                    0 => src + 4 * (1 + rng.below(8)),
                    1 => src - 4 * (1 + rng.below(8)),
                    _ => ptr(a, 96),
                };
                let mut r = inn(rng, dst);
                r.ecx = src;
                (r, vec![])
            });
        }
    }

    // --- 0x40EA60 / 0xBDEA80 / 0xBDEAB0 -----------------------------------------------------------

    fn float_in(rng: &mut Rng, a: &mut Arena, words: &[usize]) {
        let sc = sc_pick(rng);
        fill(a, rng, |r| r.fin(sc));
        if rng.below(25) == 0 {
            let w = words[rng.below(words.len() as u32) as usize];
            a.0[w] = rng.any();
        }
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn lsq_matches_original() {
        let orig = build(&LSQ_BODY, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk: 0xFF, st0: true, ..Spec::default() };
        let base = ptr(&arena, 64);
        unsafe {
            drive("lsq", LSQ, orig, lsq_thunk as *const () as usize, sp, &[], &mut arena, 0x40EA_6001, &mut |rng, a, _| {
                float_in(rng, a, &[64, 65, 66]);
                (inn(rng, base), vec![])
            });
        }
    }

    fn dist_test(name: &'static str, id: usize, body: &[u8], thunk: usize, xk: u8, calls: &[(usize, usize)]) {
        let sqrt_stub = build(&[0xD9, 0xFA, 0xC3], &[], &[]); // fsqrt; ret
        let c40ea20 = build(&T_40EA20_BODY, &[(0xC, sqrt_stub)], &[]);
        redirect_set(SQRTF, c40ea20);
        let calls: Vec<(usize, usize)> = calls.iter().map(|&(o, _)| (o, c40ea20)).collect();
        let orig = build(body, &calls, &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk, st0: true, ..Spec::default() };
        let (pa, pb) = (ptr(&arena, 64), ptr(&arena, 80));
        unsafe {
            drive(name, id, orig, thunk, sp, &[], &mut arena, 0xBDEA_8000 + id as u64, &mut |rng, a, _| {
                float_in(rng, a, &[64, 66, 80, 82]);
                if rng.below(10) == 0 {
                    for w in 0..3 {
                        a.0[80 + w] = a.0[64 + w];
                    }
                }
                let b = if rng.below(10) == 0 { pa } else { pb };
                let mut r = inn(rng, b);
                r.ecx = pa;
                (r, vec![])
            });
        }
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn dsq_matches_original() {
        dist_test("dsq", DSQ, &DSQ_BODY, dsq_thunk as *const () as usize, 0xFF, &[]);
    }

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn dxz_matches_original() {
        dist_test("dxz", DXZ, &DXZ_BODY, dxz_thunk as *const () as usize, 0xFE, &[(0x31, 0)]);
    }

    // --- 0xCF7760 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn inst_matches_original() {
        let orig = build(&INST_BODY, &[], &[]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk: 0xFF, ..Spec::default() };
        let rec = ptr(&arena, 100);
        unsafe {
            drive("inst", INST, orig, inst_thunk as *const () as usize, sp, &[], &mut arena, 0xCF77_6001, &mut |rng, a, _| {
                fill(a, rng, |r| r.u32());
                let src = if rng.below(10) == 0 { ptr(a, 300 + rng.below(150) as usize) } else { ptr(a, 1000) };
                let mut r = inn(rng, rec);
                r.ecx = rng.u32();
                (r, vec![rng.u32(), src, rng.u32(), rng.u32()])
            });
        }
    }

    // --- 0xA96A70 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn xfs_matches_original() {
        let slots = std::ptr::addr_of_mut!(T_SLOTS) as usize;
        let orig = build(&XFS_BODY, &[], &[(0x0324_F7C4, slots as u32)]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_keep: true, xk: 0xFF, ..Spec::default() };
        let (veh, mat, pos) = (ptr(&arena, 100), ptr(&arena, 400), ptr(&arena, 440));
        let extra = [(slots, 0x81 * 0x4C + 0x40)];
        unsafe {
            drive("xfs", XFS, orig, xfs_thunk as *const () as usize, sp, &extra, &mut arena, 0xA96A_7001, &mut |rng, a, _| {
                fill(a, rng, |r| r.u32());
                for i in 0..(0x81 * 0x4C + 0x40) / 4 {
                    *((slots + i * 4) as *mut u32) = rng.u32();
                }
                a.0[100 + 0xA4 / 4] = match rng.below(8) {
                    0 => 0,
                    1 => 0x81,
                    2 => 0x80,
                    3 => 1,
                    4 => rng.u32(),
                    _ => 1 + rng.below(0x80),
                };
                let mut r = inn(rng, veh);
                r.edx = pos;
                (r, vec![mat])
            });
        }
    }

    // --- 0xD22930 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn sph_matches_original() {
        let mut arena = new_arena();
        let state = ptr(&arena, 4096);
        let mut stub = vec![0xB8u8];
        stub.extend_from_slice(&state.to_le_bytes());
        stub.push(0xC3);
        let ts = build(&stub, &[], &[]);
        redirect_set(THREAD_STATE, ts);
        let orig = build(&SPH_BODY, &[(0xB, ts)], &[]);
        let sp = Spec { ecx: true, edx: true, eax_mask: !0, xk: 0xFF, ..Spec::default() };
        let (center, planes) = (ptr(&arena, 64), ptr(&arena, 200));
        unsafe {
            drive("sph", SPH, orig, sph_thunk as *const () as usize, sp, &[], &mut arena, 0xD229_3001, &mut |rng, a, _| {
                let sc = sc_pick(rng);
                if rng.below(6) == 0 {
                    // small integers: many exact ties
                    fill(a, rng, |r| ((r.below(7) as f32) - 3.0).to_bits());
                } else {
                    fill(a, rng, |r| r.fin(sc));
                }
                if rng.below(40) == 0 {
                    a.0[200 + rng.below(80) as usize] = rng.any();
                }
                if rng.below(40) == 0 {
                    a.0[64 + rng.below(3) as usize] = rng.any();
                }
                let radius = match rng.below(8) {
                    0 => rng.any(),
                    1 => 0,
                    _ => rng.fin(sc),
                };
                let explicit = rng.below(3) != 0;
                let count: i32 = if explicit { rng.below(14) as i32 - 2 } else { 0 };
                a.0[4096 + 0x1684 / 4] = rng.below(8).wrapping_sub(1);
                let mut r = inn(rng, count as u32);
                r.ecx = if explicit { planes } else { 0 };
                (r, vec![center, radius])
            });
        }
    }

    // --- 0x40B8D0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn rng_matches_original() {
        static mut K32: u32 = 0x4F80_0000;
        static mut K64: u64 = 0x3DF0_0000_0000_0000;
        let gen_ptr = std::ptr::addr_of_mut!(T_GEN) as usize;
        let callee = build(
            &T_BDCE30_BODY,
            &[],
            &[(0x00E5_BFD8, std::ptr::addr_of!(K32) as u32), (0x00E5_C030, std::ptr::addr_of!(K64) as u32)],
        );
        redirect_set(RNG_FLOAT01, callee);
        let orig = build(&RNG_BODY, &[(0x5, callee)], &[(0x00EC_30F8, gen_ptr as u32)]);
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_mask: !0, xk: 0xFF, ..Spec::default() };
        let extra = [(gen_ptr, 0x9C8 + 0x2000)];
        unsafe {
            drive("rng", RNG, orig, rng_thunk as *const () as usize, sp, &extra, &mut arena, 0x40B8_D001, &mut |rng, _a, _| {
                let idx = if rng.below(8) == 0 { 0x7FF } else { rng.below(0x800) };
                *((gen_ptr + 0x9C4) as *mut u32) = idx;
                for i in 0..0x800 {
                    *((gen_ptr + 0x9C8 + i * 4) as *mut u32) = rng.u32();
                }
                let v = *((gen_ptr + 0x9C8 + idx as usize * 4) as *const u32);
                let rr = ((v as i32) as f64 + if (v as i32) < 0 { 4294967296.0 } else { 0.0 }) * 2f64.powi(-32);
                let p = match rng.below(10) {
                    0 => (rr as f32).to_bits(),
                    1 => 0,
                    2 => 0x3F80_0000,
                    3 => rng.any(),
                    4 => ((rng.f01() * -0.5) as f32).to_bits(),
                    _ => (rng.f01() as f32).to_bits(),
                };
                { let e = rng.u32(); (inn(rng, e), vec![p]) }
            });
        }
    }

    // --- 0xBF0EA0 ---------------------------------------------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn round_matches_original() {
        static mut ZERO: u32 = 0;
        static mut HALF: u64 = 0x3FE0_0000_0000_0000;
        let flag = std::ptr::addr_of_mut!(T_FLAG);
        let ftol = build(&T_DA0140_BODY, &[], &[(0x034A_536C, flag as u32)]);
        redirect_set(FTOL2, ftol);
        let orig = build(
            &ROUND_BODY,
            &[(0x19, ftol), (0x24, ftol)],
            &[(0x00DD_32C0, std::ptr::addr_of!(ZERO) as u32), (0x00E5_BF70, std::ptr::addr_of!(HALF) as u32)],
        );
        let mut arena = new_arena();
        let sp = Spec { ecx: true, edx: true, eax_mask: !0, xk: 0xFE, xmm0: true, ..Spec::default() };
        unsafe {
            drive("round", ROUND, orig, round_thunk as *const () as usize, sp, &[], &mut arena, 0xBF0E_A001, &mut |rng, _a, _| {
                *flag = if rng.below(6) == 0 { 0 } else { 1 + rng.below(100) };
                let sc = [1.0, 10.0, 1000.0, 1e6, 3e9][rng.below(5) as usize];
                let x = match rng.below(12) {
                    0 => rng.any(),
                    1 => ((rng.below(2001) as f32 - 1000.0) + 0.5).to_bits(),
                    2 => (rng.below(2001) as f32 - 1000.0).to_bits(),
                    3 => 0x8000_0000,
                    4 => (-(rng.f01() * 0.9) as f32).to_bits(),
                    5 => (((rng.f01() * 2.0 - 1.0) * 4e9) as f32).to_bits(),
                    6 => 0x4F00_0000 | rng.below(0x100),
                    7 => 0xCF00_0000 | rng.below(0x100),
                    _ => rng.fin(sc),
                };
                { let e = rng.u32(); (inn(rng, e), vec![x]) }
            });
            *flag = 1;
        }
    }

    // --- control: the shadow must notice a difference ---------------------------------------------

    #[test]
    #[ignore = "offline differential test (see the comment above)"]
    fn shadow_detects_a_difference() {
        // vec3_dist_sq_xz's shadow runs vec3_length_sq's bytes: the results differ
        let orig = build(&LSQ_BODY, &[], &[]);
        let thunk = dsq_thunk as *const () as usize;
        PORTS[DSQ].original.store(orig, Ordering::Relaxed);
        PORTS[DSQ].calls.store(0, Ordering::Relaxed);
        PORTS[DSQ].mismatches.store(0, Ordering::Relaxed);
        let mut arena = new_arena();
        for i in 0..24 {
            arena.0[64 + i] = ((i + 1) as f32 * 1.5).to_bits();
        }
        let r = In { eax: ptr(&arena, 70), ecx: ptr(&arena, 64), edx: 0, esi: 0, edi: 0 };
        let want = unsafe { call_t(orig, r, &[]) };
        let got = unsafe { call_t(thunk, r, &[]) };
        assert!(PORTS[DSQ].mismatches.load(Ordering::Relaxed) >= 1, "the shadow missed a different result");
        assert_eq!(st0_f32(&want), st0_f32(&got), "on a mismatch the original's result must win");
        PORTS[DSQ].original.store(0, Ordering::Relaxed);
    }
}
