//! Port module: eight hot leaf functions from the 2026-11-04c tss02 game-thread log (doc 57,
//! section 6, candidates 1 to 8). All of them are leaves (no calls except the one tail call of
//! `segment_closest_t_dist_sq`) and are written by hand; none is machine-lifted.
//!
//! The recipes in doc 57 were checked against the disassembly. Differences:
//! - `segment_closest_t_dist_sq` (0xBE0400): the doc says t = dot(P-A, d) / dot(d, d) with d = B-A.
//!   The code projects the stack point B onto the segment from EDX to EAX: q = EAX - EDX,
//!   t = dot(B-EDX, ...) -- see `seg_core`. The degenerate test is `|EAX-EDX|^2 < 1e-10`, and the
//!   tail call is vec3_distance_sq(ECX = stack point, EAX = EDX point) with t = the float 0.5
//!   at 0xDD1C8C (not 0).
//! - `memcmp_regs` (0x4F4E60): same as the doc (first differing byte decides, +1 / -1).
//! - `bsp_point_in_volume`, `aabb_transform_by_mat33_extents`: as the doc, plus the exact select
//!   rules (see the comments at the cores).
//!
//! x87 code (53-bit precision): products of two floats are exact in f64, sums are f64 sums, and
//! every `fstp dword` is an `as f32`. The SSE parts are f32 moves / selects only.
//!
//! Checks:
//! - `vec3_mul_mat33_to`, `aabb_transform_by_mat33_extents`, `rig_bone_matrices_transpose_to_palette`,
//!   `quicksort_pairs_u32_by_key`, `quicksort_u64_pairs`, `segment_closest_t_dist_sq`: snapshot
//!   shadow for the first `SHADOW_CALLS` calls (run the relocated original on the written range,
//!   save, restore, run Rust, compare; the original's writes win on a mismatch). The two sorts and
//!   the palette copy skip the shadow when the written range exceeds 64 KB.
//! - `bsp_point_in_volume`, `memcmp_regs`: pure, relocated-original shadow (EAX compared, the
//!   original's result wins).
//! - Every function is also checked offline against the original bytes (tests at the end).
//!
//! Known deviations (none matter to a caller that follows the calling convention):
//! - NaN inputs: x87 picks the NaN with the larger significand, SSE the first operand, so a NaN
//!   result can carry a different payload.
//! - The two quicksorts write their `lo` stack argument (the original does, the caller's slot is
//!   dead after the call); the port does not.
//! - ECX / EDX on exit of the functions that clobber them differ from the original's leftovers
//!   (memcmp_regs, bsp_point_in_volume, the palette copy, the sorts).

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;
const SHADOW_BYTES: usize = 64 * 1024;

// --- original bodies (tools/body_bytes.py, explicit ends) -----------------------------------------

const B_VEC3_MUL: [u8; 191] = [
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
const B_AABB_XFORM: [u8; 927] = [
    0x83, 0xEC, 0x14, 0xD9, 0x07, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8,
    0x0E, 0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x0A, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x04, 0x24, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xD9, 0xEE, 0xD9, 0xC9,
    0xDB, 0xF1, 0x72, 0x05, 0x0F, 0x28, 0xCA, 0xEB, 0x03, 0x0F, 0x28, 0xC8, 0xDF, 0xF1, 0xF3, 0x0F,
    0x11, 0x4C, 0x24, 0x08, 0xF3, 0x0F, 0x11, 0x09, 0x73, 0x03, 0x0F, 0x28, 0xC2, 0xF3, 0x0F, 0x11,
    0x41, 0x0C, 0xD9, 0x47, 0x0C, 0xD9, 0x5C, 0x24, 0x10, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x0C, 0xD9,
    0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x4E, 0x04, 0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24,
    0xD8, 0x4A, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04, 0x24, 0xF3, 0x0F, 0x10, 0x4C, 0x24, 0x04,
    0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF1, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x04, 0xEB, 0x06,
    0xF3, 0x0F, 0x11, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x11, 0xD9, 0xC9, 0xDF, 0xF2, 0x72, 0x08, 0xF3, 0x0F,
    0x11, 0x44, 0x24, 0x08, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08,
    0xD8, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x51, 0x0C, 0xD9,
    0x47, 0x18, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x4E, 0x08, 0xD9,
    0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x4A, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04,
    0x24, 0xF3, 0x0F, 0x10, 0x4C, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF3, 0x72, 0x08, 0xF3,
    0x0F, 0x11, 0x4C, 0x24, 0x0C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24,
    0x0C, 0xDE, 0xC3, 0xD9, 0xCA, 0xD9, 0x19, 0xD9, 0xC9, 0xDF, 0xF2, 0x72, 0x08, 0xF3, 0x0F, 0x11,
    0x44, 0x24, 0x0C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x0C, 0xD8, 0x44, 0x24, 0x0C, 0xD9,
    0x59, 0x0C, 0xD9, 0x47, 0x04, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8,
    0x0E, 0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x0A, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x04, 0x24, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF1, 0x72, 0x05,
    0x0F, 0x28, 0xCA, 0xEB, 0x03, 0x0F, 0x28, 0xC8, 0xDF, 0xF1, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x08,
    0xF3, 0x0F, 0x11, 0x49, 0x04, 0x73, 0x03, 0x0F, 0x28, 0xC2, 0xF3, 0x0F, 0x11, 0x41, 0x10, 0xD9,
    0x47, 0x10, 0xD9, 0x5C, 0x24, 0x0C, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x0C,
    0xD9, 0xC0, 0xD8, 0x4E, 0x04, 0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x4A, 0x04,
    0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04, 0x24, 0xF3, 0x0F, 0x10, 0x4C, 0x24, 0x04, 0xD8, 0x64, 0x24,
    0x04, 0xDB, 0xF1, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x0C, 0xEB, 0x06, 0xF3, 0x0F, 0x11,
    0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0xD9,
    0x44, 0x24, 0x0C, 0xD9, 0x51, 0x04, 0xD9, 0xC9, 0xDF, 0xF2, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x44,
    0x24, 0x0C, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD8, 0x44,
    0x24, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x51, 0x10, 0xD9, 0x47, 0x1C,
    0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x4E, 0x08, 0xD9, 0x1C, 0x24,
    0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x4A, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04, 0x24, 0xF3,
    0x0F, 0x10, 0x4C, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF3, 0x72, 0x08, 0xF3, 0x0F, 0x11,
    0x4C, 0x24, 0x10, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDE,
    0xC3, 0xD9, 0xCA, 0xD9, 0x59, 0x04, 0xD9, 0xC9, 0xDF, 0xF2, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x44,
    0x24, 0x10, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x10, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x59,
    0x10, 0xD9, 0x47, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x0E,
    0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x0A, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04,
    0x24, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF1, 0x72, 0x05, 0x0F,
    0x28, 0xCA, 0xEB, 0x03, 0x0F, 0x28, 0xC8, 0xDF, 0xF1, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x0C, 0xF3,
    0x0F, 0x11, 0x49, 0x08, 0x73, 0x03, 0x0F, 0x28, 0xC2, 0xF3, 0x0F, 0x11, 0x41, 0x14, 0xD9, 0x47,
    0x14, 0xD9, 0x5C, 0x24, 0x10, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x10, 0xD9,
    0xC0, 0xD8, 0x4E, 0x04, 0xD9, 0x1C, 0x24, 0xF3, 0x0F, 0x10, 0x04, 0x24, 0xD8, 0x4A, 0x04, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x04, 0x24, 0xF3, 0x0F, 0x10, 0x4C, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04,
    0xDB, 0xF1, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x10, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0x51, 0x08, 0xD9, 0xC9, 0xDF, 0xF2, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x44, 0x24,
    0x10, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x4C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x44, 0x24,
    0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x51, 0x14, 0xD9, 0x47, 0x20, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x4E, 0x08, 0xD9, 0x1C, 0x24, 0xF3,
    0x0F, 0x10, 0x04, 0x24, 0xD8, 0x4A, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x04, 0x24, 0xF3, 0x0F,
    0x10, 0x4C, 0x24, 0x04, 0xD8, 0x64, 0x24, 0x04, 0xDB, 0xF3, 0x72, 0x08, 0xF3, 0x0F, 0x11, 0x4C,
    0x24, 0x10, 0xEB, 0x06, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0xDE, 0xC3,
    0xD9, 0xCA, 0xD9, 0x59, 0x08, 0xD9, 0xC9, 0xDF, 0xF2, 0xDD, 0xD9, 0x72, 0x11, 0xF3, 0x0F, 0x11,
    0x44, 0x24, 0x10, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x59, 0x14, 0x83, 0xC4, 0x14, 0xC3, 0xF3, 0x0F,
    0x11, 0x4C, 0x24, 0x10, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x59, 0x14, 0x83, 0xC4, 0x14, 0xC3,
];
const B_QSORT_U32: [u8; 168] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x83, 0xEC, 0x24, 0x53, 0x8B, 0x5D, 0x0C, 0x56, 0x8B, 0x75,
    0x08, 0x57, 0x8B, 0x45, 0x10, 0x8D, 0x14, 0x03, 0xD1, 0xFA, 0x8B, 0x0C, 0xD6, 0x8B, 0x54, 0xD6,
    0x04, 0x8B, 0xFB, 0x89, 0x54, 0x24, 0x24, 0x39, 0x0C, 0xFE, 0x73, 0x0C, 0x8D, 0x64, 0x24, 0x00,
    0x83, 0xC7, 0x01, 0x39, 0x0C, 0xFE, 0x72, 0xF8, 0x3B, 0x0C, 0xC6, 0x73, 0x0B, 0x8D, 0x49, 0x00,
    0x83, 0xE8, 0x01, 0x3B, 0x0C, 0xC6, 0x72, 0xF8, 0x3B, 0xC7, 0x7C, 0x33, 0x74, 0x27, 0x8B, 0x5C,
    0xC6, 0x04, 0x8B, 0x14, 0xC6, 0x89, 0x5C, 0x24, 0x14, 0x8B, 0x1C, 0xFE, 0x89, 0x1C, 0xC6, 0x8B,
    0x5C, 0xFE, 0x04, 0x89, 0x5C, 0xC6, 0x04, 0x8B, 0x5D, 0x0C, 0x89, 0x14, 0xFE, 0x8B, 0x54, 0x24,
    0x14, 0x89, 0x54, 0xFE, 0x04, 0x83, 0xE8, 0x01, 0x83, 0xC7, 0x01, 0x3B, 0xF8, 0x7E, 0xA8, 0x3B,
    0xD8, 0x7D, 0x0F, 0x8B, 0x4D, 0x14, 0x51, 0x50, 0x53, 0x56, 0xE8, 0x71, 0xFF, 0xFF, 0xFF, 0x83,
    0xC4, 0x10, 0x3B, 0x7D, 0x10, 0x7D, 0x0A, 0x8B, 0xDF, 0x89, 0x5D, 0x0C, 0xE9, 0x71, 0xFF, 0xFF,
    0xFF, 0x5F, 0x5E, 0x5B, 0x8B, 0xE5, 0x5D, 0xC3,
];
const B_RIG_PALETTE: [u8; 130] = [
    0x8B, 0x01, 0x56, 0x8B, 0xB1, 0x28, 0x02, 0x00, 0x00, 0x33, 0xD2, 0x39, 0x56, 0x04, 0x7E, 0x6E,
    0x8B, 0x4C, 0x24, 0x08, 0x57, 0x83, 0xC1, 0x08, 0x8D, 0xBE, 0x08, 0x01, 0x00, 0x00, 0x8B, 0xFF,
    0x66, 0x83, 0x3F, 0x00, 0x7C, 0x46, 0xD9, 0x00, 0xD9, 0x59, 0xF8, 0xD9, 0x40, 0x0C, 0xD9, 0x59,
    0xFC, 0xD9, 0x40, 0x18, 0xD9, 0x19, 0xD9, 0x40, 0x24, 0xD9, 0x59, 0x04, 0xD9, 0x40, 0x04, 0xD9,
    0x59, 0x08, 0xD9, 0x40, 0x10, 0xD9, 0x59, 0x0C, 0xD9, 0x40, 0x1C, 0xD9, 0x59, 0x10, 0xD9, 0x40,
    0x28, 0xD9, 0x59, 0x14, 0xD9, 0x40, 0x08, 0xD9, 0x59, 0x18, 0xD9, 0x40, 0x14, 0xD9, 0x59, 0x1C,
    0xD9, 0x40, 0x20, 0xD9, 0x59, 0x20, 0xD9, 0x40, 0x2C, 0xD9, 0x59, 0x24, 0x83, 0xC2, 0x01, 0x83,
    0xC0, 0x30, 0x83, 0xC7, 0x02, 0x83, 0xC1, 0x30, 0x3B, 0x56, 0x04, 0x7C, 0xA3, 0x5F, 0x5E, 0xC2,
    0x04, 0x00,
];
const B_BSP_POINT: [u8; 203] = [
    0x83, 0xEC, 0x0C, 0xF3, 0x0F, 0x10, 0x05, 0xB0, 0xBF, 0xE5, 0x00, 0xF3, 0x0F, 0x5C, 0x44, 0x24,
    0x10, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x10, 0xF3, 0x0F, 0x10, 0x40, 0x04, 0xD9, 0x44, 0x24, 0x10,
    0xF3, 0x0F, 0x11, 0x04, 0x24, 0xF3, 0x0F, 0x10, 0x00, 0xD9, 0x04, 0x24, 0x56, 0x8B, 0xF1, 0x8B,
    0x56, 0x0C, 0x8B, 0x76, 0x08, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x08, 0xF3, 0x0F, 0x10, 0x40, 0x08,
    0xD9, 0x44, 0x24, 0x08, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0x8B, 0xCA,
    0x0F, 0xB7, 0x01, 0xC1, 0xE0, 0x04, 0x03, 0xC6, 0xD9, 0x40, 0x04, 0xD8, 0xCB, 0xD9, 0x00, 0xD8,
    0xCB, 0xDE, 0xC1, 0xD9, 0x40, 0x08, 0xD8, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44,
    0x24, 0x14, 0xD8, 0x40, 0x0C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xCC, 0xDB,
    0xF4, 0xDD, 0xDC, 0x76, 0x14, 0x0F, 0xB7, 0x41, 0x02, 0x66, 0x85, 0xC0, 0x74, 0x1F, 0x0F, 0xB7,
    0xC0, 0x8D, 0x04, 0x40, 0x8D, 0x0C, 0x42, 0xEB, 0xB7, 0x0F, 0xB7, 0x41, 0x04, 0x66, 0x85, 0xC0,
    0x74, 0x1A, 0x0F, 0xB7, 0xC0, 0x8D, 0x0C, 0x40, 0x8D, 0x0C, 0x4A, 0xEB, 0xA3, 0xDD, 0xDB, 0x32,
    0xC0, 0xDD, 0xD9, 0x5E, 0xDD, 0xD8, 0xDD, 0xD8, 0x83, 0xC4, 0x0C, 0xC3, 0xDD, 0xDB, 0xB0, 0x01,
    0xDD, 0xD9, 0x5E, 0xDD, 0xD8, 0xDD, 0xD8, 0x83, 0xC4, 0x0C, 0xC3,
];
const B_QSORT_U64: [u8; 179] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x83, 0xEC, 0x24, 0x53, 0x56, 0x8B, 0x75, 0x08, 0x57, 0x90,
    0x8B, 0x4D, 0x0C, 0x8B, 0x45, 0x10, 0x8B, 0xF9, 0x03, 0xC8, 0xD1, 0xF9, 0x8B, 0x5C, 0xCE, 0x04,
    0x8B, 0x14, 0xCE, 0x89, 0x5C, 0x24, 0x24, 0x8B, 0x0C, 0xFE, 0x3B, 0xCA, 0x72, 0x08, 0x75, 0x10,
    0x39, 0x5C, 0xFE, 0x04, 0x73, 0x0A, 0x83, 0xC7, 0x01, 0xEB, 0xEC, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x0C, 0xC6, 0x3B, 0xD1, 0x72, 0x08, 0x75, 0x0B, 0x3B, 0x5C, 0xC6, 0x04, 0x73, 0x05, 0x83,
    0xE8, 0x01, 0xEB, 0xEC, 0x3B, 0xC7, 0x7C, 0x31, 0x74, 0x25, 0x8B, 0x5C, 0xC6, 0x04, 0x89, 0x5C,
    0x24, 0x14, 0x8B, 0x1C, 0xFE, 0x89, 0x1C, 0xC6, 0x8B, 0x5C, 0xFE, 0x04, 0x89, 0x5C, 0xC6, 0x04,
    0x8B, 0x5C, 0x24, 0x24, 0x89, 0x0C, 0xFE, 0x8B, 0x4C, 0x24, 0x14, 0x89, 0x4C, 0xFE, 0x04, 0x83,
    0xE8, 0x01, 0x83, 0xC7, 0x01, 0x3B, 0xF8, 0x7E, 0x9E, 0x8B, 0x4D, 0x0C, 0x3B, 0xC8, 0x7D, 0x0F,
    0x8B, 0x55, 0x14, 0x52, 0x50, 0x51, 0x56, 0xE8, 0x64, 0xFF, 0xFF, 0xFF, 0x83, 0xC4, 0x10, 0x3B,
    0x7D, 0x10, 0x7D, 0x08, 0x89, 0x7D, 0x0C, 0xE9, 0x64, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5B, 0x8B,
    0xE5, 0x5D, 0xC3,
];
const B_MEMCMP_REGS: [u8; 139] = [
    0x83, 0xF8, 0x04, 0x8B, 0x54, 0x24, 0x04, 0x56, 0x57, 0x72, 0x19, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x32, 0x3B, 0x31, 0x75, 0x12, 0x83, 0xE8, 0x04, 0x83, 0xC1, 0x04, 0x83, 0xC2, 0x04, 0x83,
    0xF8, 0x04, 0x73, 0xEC, 0x85, 0xC0, 0x74, 0x5E, 0x0F, 0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7,
    0x75, 0x45, 0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x01, 0x85, 0xC0, 0x74, 0x47, 0x0F,
    0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7, 0x75, 0x2E, 0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83,
    0xC2, 0x01, 0x85, 0xC0, 0x74, 0x30, 0x0F, 0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7, 0x75, 0x17,
    0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x01, 0x85, 0xC0, 0x74, 0x19, 0x0F, 0xB6, 0x32,
    0x0F, 0xB6, 0x01, 0x2B, 0xF0, 0x74, 0x0F, 0x85, 0xF6, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x7F, 0x08,
    0x5F, 0x83, 0xC8, 0xFF, 0x5E, 0xC3, 0x33, 0xC0, 0x5F, 0x5E, 0xC3,
];
const B_SEG_CLOSEST: [u8; 439] = [
    0x83, 0xEC, 0x24, 0x8B, 0x4C, 0x24, 0x28, 0xD9, 0x01, 0xD9, 0x1C, 0x24, 0xD9, 0x02, 0xD9, 0x5C,
    0x24, 0x28, 0xD9, 0x04, 0x24, 0xD9, 0x44, 0x24, 0x28, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x18, 0xD9, 0x41, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x42, 0x04, 0xD9, 0x5C, 0x24,
    0x28, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x28, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x1C, 0xD9, 0x41, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x42, 0x08, 0xD9, 0x5C, 0x24,
    0x28, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x28, 0xD9, 0xC0, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x00, 0xD8, 0xE3, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x40, 0x04, 0xDE, 0xE2,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD8, 0x68, 0x08, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x0C, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xD9,
    0xC2, 0xDE, 0xCB, 0xD9, 0xC4, 0xDE, 0xCD, 0xD9, 0xCA, 0xDE, 0xC4, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9,
    0xCB, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x28, 0xDD, 0x05, 0x60, 0xC4, 0xE5,
    0x00, 0xDF, 0xF1, 0x76, 0x20, 0xF3, 0x0F, 0x10, 0x05, 0x8C, 0x1C, 0xDD, 0x00, 0xDD, 0xD8, 0xDD,
    0xDB, 0xF3, 0x0F, 0x11, 0x06, 0xDD, 0xD9, 0x8B, 0xC2, 0xDD, 0xD9, 0xDD, 0xD8, 0x83, 0xC4, 0x24,
    0xE9, 0x5B, 0xE5, 0xFF, 0xFF, 0xD9, 0xC3, 0xD8, 0x4C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x1C, 0xD8,
    0xCB, 0xDE, 0xC1, 0xD9, 0xC3, 0xD8, 0x4C, 0x24, 0x20, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x28, 0xD8,
    0x7C, 0x24, 0x28, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x28, 0xD9, 0x16, 0xD9, 0xEE, 0xD9,
    0xC9, 0xDF, 0xF1, 0xDD, 0xD8, 0x76, 0x18, 0xF3, 0x0F, 0x10, 0x05, 0x8C, 0x51, 0xDD, 0x00, 0xF3,
    0x0F, 0x10, 0x4C, 0x24, 0x28, 0x0F, 0x2F, 0xC1, 0x76, 0x08, 0x0F, 0x28, 0xC1, 0xEB, 0x03, 0x0F,
    0x57, 0xC0, 0xF3, 0x0F, 0x11, 0x44, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x28, 0xF3, 0x0F, 0x11, 0x06,
    0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x28, 0xD9, 0xC0, 0xDE, 0xCC, 0xD9, 0xCB, 0xD9, 0x5C,
    0x24, 0x18, 0xD8, 0xCA, 0xD9, 0x5C, 0x24, 0x1C, 0xDE, 0xC9, 0xD9, 0x5C, 0x24, 0x20, 0xD8, 0x44,
    0x24, 0x18, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x42, 0x04, 0xD9, 0x5C, 0x24,
    0x10, 0xD9, 0x42, 0x08, 0xD8, 0x44, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x0C,
    0xD8, 0x24, 0x24, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0x64, 0x24, 0x04, 0xD9,
    0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0x64, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x20, 0xD9,
    0x44, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x20, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9,
    0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x28, 0xD9,
    0x44, 0x24, 0x28, 0x83, 0xC4, 0x24, 0xC3,
];


// --- per-function state ----------------------------------------------------------------------------

struct Stat {
    orig: AtomicUsize,
    calls: AtomicU32,
    bad: AtomicU32,
}

const fn stat() -> Stat {
    Stat { orig: AtomicUsize::new(0), calls: AtomicU32::new(0), bad: AtomicU32::new(0) }
}

static S_VEC3: Stat = stat();
static S_AABB: Stat = stat();
static S_Q32: Stat = stat();
static S_RIG: Stat = stat();
static S_BSP: Stat = stat();
static S_Q64: Stat = stat();
static S_MEMCMP: Stat = stat();
static S_SEG: Stat = stat();

impl Stat {
    /// Count a call (plain load / store: a lost count on a race does not matter) and say whether
    /// this one is shadowed.
    #[inline(always)]
    fn tick(&self) -> bool {
        let n = self.calls.load(Ordering::Relaxed);
        if n < u32::MAX {
            self.calls.store(n + 1, Ordering::Relaxed);
        }
        n < SHADOW_CALLS && self.orig.load(Ordering::Relaxed) != 0
    }
    #[cold]
    fn mismatch(&self) {
        self.bad.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(always)]
unsafe fn rf64(p: usize) -> f64 {
    unsafe { *(p as *const f32) as f64 }
}

#[inline(always)]
unsafe fn rw(p: usize) -> u32 {
    unsafe { std::ptr::read_unaligned(p as *const u16) as u32 }
}

/// Call a game function that returns a double on ST0 (copy of the helper in port_shops).
unsafe fn call_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
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

// =================================================================================================
// 1. vec3_mul_mat33_to (0x406810): ECX = &mat33, EDX = &in, stack &out (plain ret); out = in * mat.
// =================================================================================================
//
// Per axis the original stores x*row0, y*row1, z*row2 (each product rounded to f32), then
// (xt + yt) rounded to f32, then + zt rounded to f32. All of `in` is read before `out` is written.
// EAX = &out on exit; ECX / EDX / ESI / EDI / EBX preserved; x87 empty.

#[inline(always)]
unsafe fn vec3_mul_core(m: usize, v: usize, out: usize) {
    unsafe {
        let (z, y, x) = (rf64(v + 8), rf64(v + 4), rf64(v));
        let zt0 = (z * rf64(m + 0x18)) as f32;
        let zt1 = (z * rf64(m + 0x1C)) as f32;
        let zt2 = (z * rf64(m + 0x20)) as f32;
        let yt0 = (y * rf64(m + 0x0C)) as f32;
        let yt1 = (y * rf64(m + 0x10)) as f32;
        let yt2 = (y * rf64(m + 0x14)) as f32;
        let xt0 = (x * rf64(m)) as f32;
        let xt1 = (x * rf64(m + 4)) as f32;
        let xt2 = (x * rf64(m + 8)) as f32;
        let s0 = (xt0 as f64 + yt0 as f64) as f32;
        let s1 = (xt1 as f64 + yt1 as f64) as f32;
        let s2 = (xt2 as f64 + yt2 as f64) as f32;
        *(out as *mut f32) = (s0 as f64 + zt0 as f64) as f32;
        *((out + 4) as *mut f32) = (s1 as f64 + zt1 as f64) as f32;
        *((out + 8) as *mut f32) = (s2 as f64 + zt2 as f64) as f32;
    }
}

extern "C" fn vec3_mul_rs(m: u32, v: u32, out: u32) -> u32 {
    unsafe {
        if S_VEC3.tick() {
            let o = out as usize;
            let saved = [rd(o), rd(o + 4), rd(o + 8)];
            let ra = call_regs(S_VEC3.orig.load(Ordering::Relaxed), 0, m, v, 0, 0, &[out]);
            let orig = [rd(o), rd(o + 4), rd(o + 8)];
            wd(o, saved[0]);
            wd(o + 4, saved[1]);
            wd(o + 8, saved[2]);
            vec3_mul_core(m as usize, v as usize, o);
            let mine = [rd(o), rd(o + 4), rd(o + 8)];
            if mine != orig || ra != out {
                S_VEC3.mismatch();
                wd(o, orig[0]);
                wd(o + 4, orig[1]);
                wd(o + 8, orig[2]);
            }
        } else {
            vec3_mul_core(m as usize, v as usize, out as usize);
        }
    }
    out
}

#[unsafe(naked)]
extern "C" fn vec3_mul_thunk() {
    std::arch::naked_asm!(
        "push ecx", "push edx",
        "push dword ptr [esp + 12]", "push edx", "push ecx",
        "call {f}",
        "add esp, 12",
        "pop edx", "pop ecx",
        "ret",
        f = sym vec3_mul_rs,
    )
}

// =================================================================================================
// 2. aabb_transform_by_mat33_extents (0xBF4C30): EDI = &mat33 (M[i][j] = edi[3i+j]), ESI = &lo,
//    EDX = &hi, ECX = &out[6] (min at +0, max at +0xC). Plain ret, x87 empty, EAX / ECX / EDX /
//    ESI / EDI all preserved.
// =================================================================================================
//
// For each column j and row i: A = f32(M[i][j]*lo[i]), B = f32(M[i][j]*hi[i]), d = A - B (f64).
// `lt` = d < 0 or unordered (the `jb` after fcomi against 0); then min term = lt ? A : B and
// max term = lt ? B : A (the first row's max uses `jae` on the same flags: same rule).
// Row 0 stores the terms; rows 1 and 2 store f32(term + running) (x87 add rounded at the fstp).
// The stores go to out[j] and out[3+j] after every row, in the original's order (min, then max),
// and rows read M / lo / hi only after the previous row's stores, so overlapping buffers behave
// like the original.

#[inline(always)]
unsafe fn aabb_core(m: usize, lo: usize, hi: usize, out: usize) {
    unsafe {
        let o = out as *mut f32;
        for j in 0..3usize {
            let mut mn = 0f32;
            let mut mx = 0f32;
            for i in 0..3usize {
                let mm = rf64(m + (3 * i + j) * 4);
                let l = rf64(lo + i * 4);
                let h = rf64(hi + i * 4);
                let a = (mm * l) as f32;
                let b = (mm * h) as f32;
                let d = a as f64 - b as f64;
                let (tmin, tmax) = if d >= 0.0 { (b, a) } else { (a, b) };
                if i == 0 {
                    mn = tmin;
                    mx = tmax;
                } else {
                    mn = (tmin as f64 + mn as f64) as f32;
                    mx = (tmax as f64 + mx as f64) as f32;
                }
                *o.add(j) = mn;
                *o.add(3 + j) = mx;
            }
        }
    }
}

extern "C" fn aabb_rs(m: u32, lo: u32, hi: u32, out: u32) {
    unsafe {
        if S_AABB.tick() {
            let o = out as usize;
            let mut saved = [0u32; 6];
            for (k, s) in saved.iter_mut().enumerate() {
                *s = rd(o + k * 4);
            }
            let ra = call_regs(S_AABB.orig.load(Ordering::Relaxed), 0x1357_9BDF, out, hi, lo, m, &[]);
            let mut orig = [0u32; 6];
            for (k, s) in orig.iter_mut().enumerate() {
                *s = rd(o + k * 4);
            }
            for (k, s) in saved.iter().enumerate() {
                wd(o + k * 4, *s);
            }
            aabb_core(m as usize, lo as usize, hi as usize, o);
            let mut bad = ra != 0x1357_9BDF;
            for (k, s) in orig.iter().enumerate() {
                if rd(o + k * 4) != *s {
                    bad = true;
                }
            }
            if bad {
                S_AABB.mismatch();
                for (k, s) in orig.iter().enumerate() {
                    wd(o + k * 4, *s);
                }
            }
        } else {
            aabb_core(m as usize, lo as usize, hi as usize, out as usize);
        }
    }
}

#[unsafe(naked)]
extern "C" fn aabb_thunk() {
    std::arch::naked_asm!(
        "push eax", "push ecx", "push edx",
        "push ecx", "push edx", "push esi", "push edi",
        "call {f}",
        "add esp, 16",
        "pop edx", "pop ecx", "pop eax",
        "ret",
        f = sym aabb_rs,
    )
}

// =================================================================================================
// 3. quicksort_pairs_u32_by_key (0xBBC4E0) and 6. quicksort_u64_pairs (0xC5D160)
// =================================================================================================
//
// cdecl (array, lo, hi, unused), 8-byte records, no return value. Hoare partition around the
// record at (lo+hi)>>1 (arithmetic shift), keys compared UNSIGNED: i advances while arr[i] < pivot,
// j retreats while pivot < arr[j], swap when i < j, then i++ / j-- while i <= j; recurse on
// [lo, j] when lo < j and loop on [i, hi] when i < hi. The u32 version compares the dword at +0
// only; the u64 version compares (dword0, dword1) lexicographically, both unsigned, i.e. as a
// u64 with dword0 in the high half. The pivot is read once per pass (before the i / j scan).
// The original saves and restores EBX / ESI / EDI / EBP; the Rust ABI keeps them too, so the
// Rust function is hooked directly. Indices wrap like the 32-bit address arithmetic.

#[inline(always)]
fn at(arr: usize, i: i32) -> usize {
    arr.wrapping_add((i as u32 as usize).wrapping_mul(8))
}

unsafe fn qs32(arr: usize, mut lo: i32, hi: i32) {
    unsafe {
        loop {
            let pivot = rd(at(arr, lo.wrapping_add(hi) >> 1));
            let mut i = lo;
            let mut j = hi;
            loop {
                while rd(at(arr, i)) < pivot {
                    i = i.wrapping_add(1);
                }
                while pivot < rd(at(arr, j)) {
                    j = j.wrapping_sub(1);
                }
                if j < i {
                    break;
                }
                if i != j {
                    let pi = at(arr, i);
                    let pj = at(arr, j);
                    let (ki, vi) = (rd(pi), rd(pi + 4));
                    let (kj, vj) = (rd(pj), rd(pj + 4));
                    wd(pj, ki);
                    wd(pj + 4, vi);
                    wd(pi, kj);
                    wd(pi + 4, vj);
                }
                j = j.wrapping_sub(1);
                i = i.wrapping_add(1);
                if i > j {
                    break;
                }
            }
            if lo < j {
                qs32(arr, lo, j);
            }
            if i < hi {
                lo = i;
            } else {
                break;
            }
        }
    }
}

#[inline(always)]
unsafe fn rd64(p: usize) -> u64 {
    // dword0 is the high half of the comparison key
    unsafe { ((rd(p) as u64) << 32) | rd(p + 4) as u64 }
}

unsafe fn qs64(arr: usize, mut lo: i32, hi: i32) {
    unsafe {
        loop {
            let pivot = rd64(at(arr, lo.wrapping_add(hi) >> 1));
            let mut i = lo;
            let mut j = hi;
            loop {
                while rd64(at(arr, i)) < pivot {
                    i = i.wrapping_add(1);
                }
                while pivot < rd64(at(arr, j)) {
                    j = j.wrapping_sub(1);
                }
                if j < i {
                    break;
                }
                if i != j {
                    let pi = at(arr, i);
                    let pj = at(arr, j);
                    let (ki, vi) = (rd(pi), rd(pi + 4));
                    let (kj, vj) = (rd(pj), rd(pj + 4));
                    wd(pj, ki);
                    wd(pj + 4, vi);
                    wd(pi, kj);
                    wd(pi + 4, vj);
                }
                j = j.wrapping_sub(1);
                i = i.wrapping_add(1);
                if i > j {
                    break;
                }
            }
            if lo < j {
                qs64(arr, lo, j);
            }
            if i < hi {
                lo = i;
            } else {
                break;
            }
        }
    }
}

/// Snapshot shadow shared by the two sorts: the range [lo, hi] (only when it is a sane size).
unsafe fn sort_shadow(st: &Stat, run: unsafe fn(usize, i32, i32), arr: u32, lo: i32, hi: i32, extra: u32) {
    unsafe {
        let len = (hi as i64 - lo as i64 + 1).max(0) as usize;
        let ok = len > 0 && len * 8 <= SHADOW_BYTES && lo >= 0 && (lo as i64 + len as i64) < 0x1000_0000;
        if !ok {
            run(arr as usize, lo, hi);
            return;
        }
        let base = at(arr as usize, lo);
        let saved: Vec<u8> = std::slice::from_raw_parts(base as *const u8, len * 8).to_vec();
        call_regs(st.orig.load(Ordering::Relaxed), 0, 0, 0, 0, 0, &[arr, lo as u32, hi as u32, extra]);
        let orig: Vec<u8> = std::slice::from_raw_parts(base as *const u8, len * 8).to_vec();
        std::ptr::copy_nonoverlapping(saved.as_ptr(), base as *mut u8, len * 8);
        run(arr as usize, lo, hi);
        let mine = std::slice::from_raw_parts(base as *const u8, len * 8);
        if mine != &orig[..] {
            st.mismatch();
            std::ptr::copy_nonoverlapping(orig.as_ptr(), base as *mut u8, len * 8);
        }
    }
}

extern "C" fn qsort32_rs(arr: u32, lo: i32, hi: i32, extra: u32) {
    unsafe {
        if S_Q32.tick() {
            sort_shadow(&S_Q32, qs32, arr, lo, hi, extra);
        } else {
            qs32(arr as usize, lo, hi);
        }
    }
}

extern "C" fn qsort64_rs(arr: u32, lo: i32, hi: i32, extra: u32) {
    unsafe {
        if S_Q64.tick() {
            sort_shadow(&S_Q64, qs64, arr, lo, hi, extra);
        } else {
            qs64(arr as usize, lo, hi);
        }
    }
}

// =================================================================================================
// 4. rig_bone_matrices_transpose_to_palette (0xCE4A40): ECX = rig, stack &dst (callee pops, ret 4).
// =================================================================================================
//
// src = [rig] (12 floats = 0x30 bytes per bone), n = [[rig+0x228]+4] (signed), flags are the signed
// words at [[rig+0x228]+0x108+2i]. For each bone i < n with a non-negative word: dst[c*4+r] =
// src[r*3+c] (r < 4, c < 3), through fld / fstp (a signalling NaN comes out quiet). Both pointers
// advance 0x30 bytes per bone whether it was written or not. n is re-read every iteration.

#[inline(always)]
fn quiet(v: u32) -> u32 {
    if (v & 0x7FFF_FFFF) > 0x7F80_0000 { v | 0x0040_0000 } else { v }
}

#[inline(always)]
unsafe fn rig_core(rig: usize, dst: usize) {
    unsafe {
        let src0 = rd(rig) as usize;
        let ctx = rd(rig + 0x228) as usize;
        if (rd(ctx + 4) as i32) <= 0 {
            return;
        }
        let mut s = src0;
        let mut w = ctx + 0x108;
        let mut d = dst;
        let mut i: i32 = 0;
        loop {
            if (rw(w) as u16 as i16) >= 0 {
                wd(d, quiet(rd(s)));
                wd(d + 4, quiet(rd(s + 0x0C)));
                wd(d + 8, quiet(rd(s + 0x18)));
                wd(d + 0x0C, quiet(rd(s + 0x24)));
                wd(d + 0x10, quiet(rd(s + 4)));
                wd(d + 0x14, quiet(rd(s + 0x10)));
                wd(d + 0x18, quiet(rd(s + 0x1C)));
                wd(d + 0x1C, quiet(rd(s + 0x28)));
                wd(d + 0x20, quiet(rd(s + 8)));
                wd(d + 0x24, quiet(rd(s + 0x14)));
                wd(d + 0x28, quiet(rd(s + 0x20)));
                wd(d + 0x2C, quiet(rd(s + 0x2C)));
            }
            i = i.wrapping_add(1);
            s = s.wrapping_add(0x30);
            w = w.wrapping_add(2);
            d = d.wrapping_add(0x30);
            if i >= rd(ctx + 4) as i32 {
                break;
            }
        }
    }
}

extern "C" fn rig_rs(rig: u32, dst: u32) {
    unsafe {
        if S_RIG.tick() {
            let ctx = rd(rig as usize + 0x228) as usize;
            let n = rd(ctx + 4) as i32;
            let len = if n > 0 { n as usize * 0x30 } else { 0 };
            if len == 0 || len > SHADOW_BYTES {
                rig_core(rig as usize, dst as usize);
                return;
            }
            let saved: Vec<u8> = std::slice::from_raw_parts(dst as *const u8, len).to_vec();
            call_regs(S_RIG.orig.load(Ordering::Relaxed), 0, rig, 0, 0, 0, &[dst]);
            let orig: Vec<u8> = std::slice::from_raw_parts(dst as *const u8, len).to_vec();
            std::ptr::copy_nonoverlapping(saved.as_ptr(), dst as *mut u8, len);
            rig_core(rig as usize, dst as usize);
            let mine = std::slice::from_raw_parts(dst as *const u8, len);
            if mine != &orig[..] {
                S_RIG.mismatch();
                std::ptr::copy_nonoverlapping(orig.as_ptr(), dst as *mut u8, len);
            }
        } else {
            rig_core(rig as usize, dst as usize);
        }
    }
}

#[unsafe(naked)]
extern "C" fn rig_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 4]", "push ecx",
        "call {f}",
        "add esp, 8",
        "ret 4",
        f = sym rig_rs,
    )
}

// =================================================================================================
// 5. bsp_point_in_volume (0xBFD510): ECX = tree, EAX = &point, stack f32 margin (plain ret).
//    Returns EAX = 0 or 1 (exactly), x87 empty, ESI preserved, ECX / EDX clobbered.
// =================================================================================================
//
// nm = -0.0 - margin (SSE single). Planes at [tree+8] (16 bytes: nx ny nz d), nodes at [tree+0xC]
// (6 bytes: u16 plane, u16 child_a, u16 child_b). dist = f32(f32((ny*py + nx*px) + nz*pz) + d)
// (x87 sums); if nm <= dist or either is NaN (`jbe` after fcomi) the next node is child_b, else
// child_a. A child number of 0 ends the walk: 1 from child_b, 0 from child_a; otherwise the next
// node is nodes + child*6.

#[inline(always)]
unsafe fn bsp_core(tree: usize, pt: usize, margin: u32) -> u32 {
    unsafe {
        let nm = f32::from_bits(0x8000_0000) - f32::from_bits(margin);
        let (py, px, pz) = (rf64(pt + 4), rf64(pt), rf64(pt + 8));
        let planes = rd(tree + 8) as usize;
        let nodes = rd(tree + 0x0C) as usize;
        let mut node = nodes;
        loop {
            let pl = planes.wrapping_add((rw(node) as usize) << 4);
            let s = ((rf64(pl + 4) * py + rf64(pl) * px) + rf64(pl + 8) * pz) as f32;
            let dist = (s as f64 + rf64(pl + 0x0C)) as f32;
            let (c, ret) = if !(nm > dist) { (rw(node + 4), 1) } else { (rw(node + 2), 0) };
            if c == 0 {
                return ret;
            }
            node = nodes.wrapping_add(c as usize * 6);
        }
    }
}

extern "C" fn bsp_rs(tree: u32, pt: u32, margin: u32) -> u32 {
    unsafe {
        let r = bsp_core(tree as usize, pt as usize, margin);
        if S_BSP.tick() {
            let o = call_regs(S_BSP.orig.load(Ordering::Relaxed), pt, tree, 0, 0, 0, &[margin]);
            if o != r {
                S_BSP.mismatch();
                return o;
            }
        }
        r
    }
}

#[unsafe(naked)]
extern "C" fn bsp_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 4]", "push eax", "push ecx",
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym bsp_rs,
    )
}

// =================================================================================================
// 7. memcmp_regs (0x4F4E60): EAX = length, ECX = a, stack b (plain ret). EAX = 0 when equal, else
//    +1 when the first differing byte of b is above the byte of a, -1 (0xFFFFFFFF) otherwise.
//    ESI / EDI preserved; ECX / EDX end advanced (not reproduced).
// =================================================================================================

#[inline(always)]
unsafe fn memcmp_core(mut len: u32, mut a: usize, mut b: usize) -> u32 {
    unsafe {
        while len >= 4 {
            let x = std::ptr::read_unaligned(b as *const u32);
            let y = std::ptr::read_unaligned(a as *const u32);
            if x != y {
                let sh = (x ^ y).trailing_zeros() & !7;
                return if ((x >> sh) & 0xFF) > ((y >> sh) & 0xFF) { 1 } else { 0xFFFF_FFFF };
            }
            len -= 4;
            a += 4;
            b += 4;
        }
        while len != 0 {
            let x = *(b as *const u8);
            let y = *(a as *const u8);
            if x != y {
                return if x > y { 1 } else { 0xFFFF_FFFF };
            }
            len -= 1;
            a += 1;
            b += 1;
        }
        0
    }
}

extern "C" fn memcmp_rs(len: u32, a: u32, b: u32) -> u32 {
    unsafe {
        let r = memcmp_core(len, a as usize, b as usize);
        if S_MEMCMP.tick() {
            let o = call_regs(S_MEMCMP.orig.load(Ordering::Relaxed), len, a, 0, 0, 0, &[b]);
            if o != r {
                S_MEMCMP.mismatch();
                return o;
            }
        }
        r
    }
}

#[unsafe(naked)]
extern "C" fn memcmp_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 4]", "push ecx", "push eax",
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym memcmp_rs,
    )
}

// =================================================================================================
// 8. segment_closest_t_dist_sq (0xBE0400): EAX = segment end E, EDX = segment start S, stack = &point
//    B (plain ret), ESI = &out_t. Projects B onto the segment S->E.
// =================================================================================================
//
// q = E - S (f32 each), dot = f32(f32-rounded squares summed as (qx^2+qy^2)+qz^2). If dot < the
// double 1e-10 (0xE5C460): [esi] = the float at 0xDD1C8C (0.5), then a tail call to vec3_distance_sq
// (0xBDEA30, ECX = B, EAX = S) whose ST0 comes back. Otherwise tn = f32(qx*dx + qy*dy + qz*dz) with
// d = B - S (each f32), t = f32(tn / dot), [esi] = t, then the clamp (t <= 0 or NaN -> 0.0,
// 1.0 <= t -> 1.0), [esi] = clamp, closest = S + q*clamp (S.y and S.z re-read from memory after the
// stores), e = closest - B (f32), ST0 = f32((ex^2+ey^2)+ez^2).
// Quirks reproduced: the caller's stack argument slot ends up holding dot (degenerate) or the
// result (normal) as a float; ECX = &B on exit; EAX = E (normal) or S (degenerate).

const VEC3_DIST_SQ: usize = 0x00BD_EA30;

// The offline test cannot map the exe at its own addresses, so it points these at private copies.
#[cfg(test)]
static TEST_DIST_SQ: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static TEST_DEGEN_T: AtomicU32 = AtomicU32::new(0);

#[cfg(not(test))]
#[inline(always)]
fn dist_sq_fn() -> usize {
    VEC3_DIST_SQ
}
#[cfg(test)]
fn dist_sq_fn() -> usize {
    TEST_DIST_SQ.load(Ordering::Relaxed)
}
#[cfg(not(test))]
#[inline(always)]
unsafe fn degen_t() -> u32 {
    unsafe { rd(SEG_DEGEN_T) }
}
#[cfg(test)]
unsafe fn degen_t() -> u32 {
    TEST_DEGEN_T.load(Ordering::Relaxed)
}
const SEG_EPS: f64 = f64::from_bits(0x3DDB_7CDF_E000_0000); // the double at 0xE5C460
const SEG_DEGEN_T: usize = 0x00DD_1C8C; // float 0.5

#[repr(C, packed(4))]
struct SegOut {
    st0: f64,
    eax: u32,
    ecx: u32,
}

unsafe fn seg_core(e: usize, s: usize, outt: usize, b: usize, slot: *mut u32) -> SegOut {
    unsafe {
        let bz = rf64(b + 8) as f32 as f64;
        let by = rf64(b + 4) as f32 as f64;
        let bx = rf64(b);
        let ax = rf64(s);
        let ay = rf64(s + 4);
        let az = rf64(s + 8);
        let dx = (bx - ax) as f32;
        let dy = (by - ay) as f32;
        let dz = (bz - az) as f32;
        let qx = (rf64(e) - ax) as f32;
        let qy = (rf64(e + 4) - ay) as f32;
        let qz = (rf64(e + 8) - az) as f32;
        let (qx, qy, qz) = (qx as f64, qy as f64, qz as f64);
        let dot = ((qx * qx + qy * qy) + qz * qz) as f32;
        if (dot as f64) < SEG_EPS {
            *slot = dot.to_bits();
            wd(outt, degen_t());
            let mut r = 0f64;
            // ECX = &B, EAX = &S, callee returns the f32 on ST0
            let f = dist_sq_fn();
            std::arch::asm!(
                "push edi", "call {f}", "pop edi", "fstp qword ptr [edi]",
                f = in(reg) f, in("edi") &mut r as *mut f64, in("eax") s, in("ecx") b,
                clobber_abi("C"),
            );
            return SegOut { st0: r, eax: s as u32, ecx: b as u32 };
        }
        let tn = ((qx * dx as f64 + qy * dy as f64) + qz * dz as f64) as f32;
        let t = (tn as f64 / dot as f64) as f32;
        wf(outt, t);
        let tc = if !(t > 0.0) {
            0.0f32
        } else if 1.0f32 <= t {
            1.0f32
        } else {
            t
        };
        wf(outt, tc);
        let tcd = tc as f64;
        let vx = (qx * tcd) as f32;
        let vy = (qy * tcd) as f32;
        let vz = (qz * tcd) as f32;
        let cx = (ax + vx as f64) as f32;
        let cy = (vy as f64 + rf64(s + 4)) as f32;
        let cz = (rf64(s + 8) + vz as f64) as f32;
        let ex = (cx as f64 - bx) as f32 as f64;
        let ey = (cy as f64 - by) as f32 as f64;
        let ez = (cz as f64 - bz) as f32 as f64;
        let res = ((ex * ex + ey * ey) + ez * ez) as f32;
        *slot = res.to_bits();
        SegOut { st0: res as f64, eax: e as u32, ecx: b as u32 }
    }
}

extern "C" fn seg_rs(e: u32, s: u32, outt: u32, slot: *mut u32, out: *mut SegOut) {
    unsafe {
        let b = *slot as usize;
        if S_SEG.tick() {
            let o = outt as usize;
            let saved = rd(o);
            let st0 = call_st0(S_SEG.orig.load(Ordering::Relaxed), e, 0, s, outt, 0, &[b as u32]);
            let ot = rd(o);
            wd(o, saved);
            let mut scratch = 0u32;
            let r = seg_core(e as usize, s as usize, o, b, &mut scratch);
            if r.st0.to_bits() != st0.to_bits() || rd(o) != ot {
                S_SEG.mismatch();
                wd(o, ot);
                *out = SegOut { st0, eax: r.eax, ecx: r.ecx };
            } else {
                *out = r;
            }
            *slot = scratch;
        } else {
            *out = seg_core(e as usize, s as usize, outt as usize, b, slot);
        }
    }
}

// Stack at entry: [esp] = return, [esp+4] = &B. SegOut lives in 16 bytes below it.
#[unsafe(naked)]
extern "C" fn seg_thunk() {
    std::arch::naked_asm!(
        "sub esp, 16",
        "push edx",
        "lea ecx, [esp + 4]",
        "push ecx",
        "lea ecx, [esp + 28]",
        "push ecx",
        "push esi", "push edx", "push eax",
        "call {f}",
        "add esp, 20",
        "pop edx",
        "mov eax, [esp + 8]",
        "mov ecx, [esp + 12]",
        "fld qword ptr [esp]",
        "add esp, 16",
        "ret",
        f = sym seg_rs,
    )
}

// =================================================================================================
// install / summary
// =================================================================================================

/// Copy the original into executable memory for the shadows. The two sorts call themselves
/// through a relative call, which stays correct inside the copy.
fn orig_copy(addr: usize, body: &[u8]) -> usize {
    unsafe { detour::relocate(addr, body, &[]) }.unwrap_or(0)
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    let total = 8;
    unsafe {
        S_VEC3.orig.store(orig_copy(0x406810, &B_VEC3_MUL), Ordering::Relaxed);
        S_AABB.orig.store(orig_copy(0xBF4C30, &B_AABB_XFORM), Ordering::Relaxed);
        S_Q32.orig.store(orig_copy(0xBBC4E0, &B_QSORT_U32), Ordering::Relaxed);
        S_RIG.orig.store(orig_copy(0xCE4A40, &B_RIG_PALETTE), Ordering::Relaxed);
        S_BSP.orig.store(orig_copy(0xBFD510, &B_BSP_POINT), Ordering::Relaxed);
        S_Q64.orig.store(orig_copy(0xC5D160, &B_QSORT_U64), Ordering::Relaxed);
        S_MEMCMP.orig.store(orig_copy(0x4F4E60, &B_MEMCMP_REGS), Ordering::Relaxed);
        let seg = orig_copy(0xBE0400, &B_SEG_CLOSEST);
        if seg != 0 {
            // the copy's tail `jmp rel32` (at +0xD0) to vec3_distance_sq must be re-aimed
            let rel = (VEC3_DIST_SQ as i32).wrapping_sub((seg + 0xD0 + 5) as i32);
            std::ptr::write_unaligned((seg + 0xD1) as *mut i32, rel);
        }
        S_SEG.orig.store(seg, Ordering::Relaxed);
    }
    if replace_pinned("vec3_mul_mat33_to", 0x406810, &B_VEC3_MUL, vec3_mul_thunk as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("aabb_transform_by_mat33_extents", 0xBF4C30, &B_AABB_XFORM, aabb_thunk as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("quicksort_pairs_u32_by_key", 0xBBC4E0, &B_QSORT_U32, qsort32_rs as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("rig_bone_matrices_transpose_to_palette", 0xCE4A40, &B_RIG_PALETTE, rig_thunk as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("bsp_point_in_volume", 0xBFD510, &B_BSP_POINT, bsp_thunk as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("quicksort_u64_pairs", 0xC5D160, &B_QSORT_U64, qsort64_rs as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("memcmp_regs", 0x4F4E60, &B_MEMCMP_REGS, memcmp_thunk as *const () as usize) {
        ok += 1;
    }
    if replace_pinned("segment_closest_t_dist_sq", 0xBE0400, &B_SEG_CLOSEST, seg_thunk as *const () as usize) {
        ok += 1;
    }
    log(&format!("port_hot57a: replaced {ok}/{total} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let all: [(&str, &Stat); 8] = [
        ("vec3_mul_mat33_to", &S_VEC3),
        ("aabb_transform_by_mat33_extents", &S_AABB),
        ("quicksort_pairs_u32_by_key", &S_Q32),
        ("rig_bone_matrices_transpose_to_palette", &S_RIG),
        ("bsp_point_in_volume", &S_BSP),
        ("quicksort_u64_pairs", &S_Q64),
        ("memcmp_regs", &S_MEMCMP),
        ("segment_closest_t_dist_sq", &S_SEG),
    ];
    let parts: Vec<String> = all
        .iter()
        .filter(|(_, s)| s.calls.load(Ordering::Relaxed) != 0)
        .map(|(n, s)| format!("{n} {} ({})", s.calls.load(Ordering::Relaxed), s.bad.load(Ordering::Relaxed)))
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_hot57a: {}", parts.join(", ")) }
}

// =================================================================================================
// Offline differential test
// =================================================================================================
//
// The original exe image is mapped at its own addresses inside the test process, so the original
// bytes (and the constants and the callee 0xBDEA30) run in place; the Rust port runs through its
// real thunk on an identical copy of the same random memory. Compared: every word of the memory
// both touch (NaN against NaN counts as equal), EAX / ECX / EDX / EBX / ESI / EDI after the call
// (as far as the original defines them), the x87 depth, ST0, and the stack argument slot.
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_hot57a -- --ignored --test-threads=1 --nocapture
// CASES=n changes the number of random cases per function (default 4000).

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use std::sync::Mutex;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    static LOCK: Mutex<()> = Mutex::new(());

    /// Private executable copies of the original functions (see `setup`).
    struct Origs {
        vec3: usize,
        aabb: usize,
        q32: usize,
        rig: usize,
        bsp: usize,
        q64: usize,
        memcmp: usize,
        seg: usize,
    }

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    fn alloc_exec(bytes: &[u8]) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), bytes.len().max(16), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0, "VirtualAlloc failed");
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len()) };
        p
    }

    /// Replace every occurrence of the 32-bit little-endian value `from` in `code` by `to`.
    fn patch_abs(code: &mut [u8], from: u32, to: u32) -> usize {
        let f = from.to_le_bytes();
        let mut n = 0;
        for i in 0..code.len().saturating_sub(3) {
            if code[i..i + 4] == f {
                code[i..i + 4].copy_from_slice(&to.to_le_bytes());
                n += 1;
            }
        }
        n
    }

    /// Build the private copies of the originals (once) and set the x87 to 53-bit precision.
    /// The bodies come from the embedded arrays; the constants and the callee 0xBDEA30 are read from
    /// the unpacked exe and the copies' absolute references are re-aimed at private copies of them.
    fn setup() -> (std::sync::MutexGuard<'static, ()>, &'static Origs) {
        static ORIGS: std::sync::OnceLock<Origs> = std::sync::OnceLock::new();
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let o = ORIGS.get_or_init(|| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/SR2_pc.unpacked.exe");
            let exe = std::fs::read(path).expect("read the unpacked exe");
            let u32at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
            let u16at = |o: usize| u16::from_le_bytes(exe[o..o + 2].try_into().unwrap()) as usize;
            let pe = u32at(0x3C);
            let nsec = u16at(pe + 6);
            let opt = u16at(pe + 20);
            let bytes_at = |va: usize, len: usize| -> Vec<u8> {
                for i in 0..nsec {
                    let s = pe + 24 + opt + i * 40;
                    let (vsz, sva, rsz, raw) = (u32at(s + 8), u32at(s + 12), u32at(s + 16), u32at(s + 20));
                    if va >= 0x40_0000 + sva && va + len <= 0x40_0000 + sva + vsz.max(rsz) {
                        let o = raw + va - 0x40_0000 - sva;
                        return exe[o..o + len].to_vec();
                    }
                }
                panic!("{va:#x} is not in the exe");
            };
            // the callee 0xBDEA30 (ends with ret at 0xBDEA76) and the constants
            let dist = alloc_exec(&bytes_at(0xBD_EA30, 0x47));
            let mut pool = vec![0u8; 64];
            pool[0..4].copy_from_slice(&bytes_at(0xDD_1C8C, 4)); // 0.5f
            pool[8..16].copy_from_slice(&bytes_at(0xE5_C460, 8)); // 1e-10
            pool[16..20].copy_from_slice(&bytes_at(0xDD_518C, 4)); // 1.0f
            pool[20..24].copy_from_slice(&bytes_at(0xE5_BFB0, 4)); // -0.0f
            assert_eq!(&pool[8..16], &SEG_EPS.to_bits().to_le_bytes(), "SEG_EPS differs from the exe");
            let poolp = alloc_exec(&pool);
            TEST_DIST_SQ.store(dist, Ordering::Relaxed);
            TEST_DEGEN_T.store(u32::from_le_bytes(pool[0..4].try_into().unwrap()), Ordering::Relaxed);
            let copy = |body: &[u8], pat: &[(u32, usize)]| -> usize {
                let mut c = body.to_vec();
                for &(from, to) in pat {
                    assert!(patch_abs(&mut c, from, to as u32) >= 1, "no reference to {from:#x}");
                }
                alloc_exec(&c)
            };
            let seg = {
                let mut c = B_SEG_CLOSEST.to_vec();
                assert_eq!(c[0xD0], 0xE9);
                let rel = (dist as i32).wrapping_sub(0); // fixed up below once the copy's address is known
                let _ = rel;
                for &(from, to) in &[(0xE5_C460u32, poolp + 8), (0xDD_1C8C, poolp), (0xDD_518C, poolp + 16)] {
                    assert!(patch_abs(&mut c, from, to as u32) >= 1, "no reference to {from:#x}");
                }
                let p = alloc_exec(&c);
                let rel = (dist as i32).wrapping_sub((p + 0xD0 + 5) as i32);
                unsafe { std::ptr::write_unaligned((p + 0xD1) as *mut i32, rel) };
                p
            };
            unsafe {
                let cw: u16 = 0x027F;
                std::arch::asm!("fldcw word ptr [{p}]", p = in(reg) &cw as *const u16);
            }
            Origs {
                vec3: copy(&B_VEC3_MUL, &[]),
                aabb: copy(&B_AABB_XFORM, &[]),
                q32: copy(&B_QSORT_U32, &[]),
                rig: copy(&B_RIG_PALETTE, &[]),
                bsp: copy(&B_BSP_POINT, &[(0xE5_BFB0, poolp + 20)]),
                q64: copy(&B_QSORT_U64, &[]),
                memcmp: copy(&B_MEMCMP_REGS, &[]),
                seg,
            }
        });
        (g, o)
    }

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
        /// A float bit pattern: mostly ordinary values, some zeros, huge, tiny, denormal, inf, rarely NaN.
        fn fl(&mut self) -> u32 {
            match self.below(40) {
                0 => 0,
                1 => 0x8000_0000,
                2 => (self.below(21) as f32 - 10.0).to_bits(),
                3 => self.u32() & 0x007F_FFFF,
                4 => (self.below(1000) as f32 / 7.0 * 1e20).to_bits(),
                5 => (self.below(1000) as f32 / 7.0 * 1e-20).to_bits(),
                6 if self.one_in(8) => {
                    if self.one_in(2) { 0x7F80_0000 } else { 0xFF80_0000 }
                }
                7 if self.one_in(30) => 0x7F80_0000 | (self.u32() & 0x807F_FFFF) | 1,
                _ => {
                    let m = (self.u32() >> 8) as f32 / 16_777_216.0 * 2.0 - 1.0;
                    (m * 10f32.powi(self.below(7) as i32 - 3)).to_bits()
                }
            }
        }
        /// A moderate float in +-scale (no specials).
        fn small(&mut self, scale: f32) -> u32 {
            (((self.u32() >> 8) as f32 / 16_777_216.0 * 2.0 - 1.0) * scale).to_bits()
        }
    }

    fn same(a: u32, b: u32) -> bool {
        a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
    }

    #[repr(C, packed)]
    #[derive(Clone, Copy)]
    struct Blk {
        r_in: [u32; 6], // eax ecx edx ebx esi edi
        f: u32,
        args: u32,
        nargs: u32,
        r_out: [u32; 6],
        slot: u32,
        status: u32,
        st0: u64,
    }

    impl Blk {
        fn eax(&self) -> u32 {
            let r = self.r_out;
            r[0]
        }
        fn ro(&self, i: usize) -> u32 {
            let r = self.r_out;
            r[i]
        }
        fn depth(&self) -> u32 {
            let s = self.status;
            (s & 0x3800) >> 11
        }
        fn st0(&self) -> u64 {
            self.st0
        }
        fn slot(&self) -> u32 {
            self.slot
        }
    }

    /// Call `f` with EAX ECX EDX EBX ESI EDI set and the cdecl stack args pushed; record the
    /// registers afterwards, the dword on top of the stack right after the call (the first argument
    /// for a plain-ret callee), the x87 TOP field and ST0 when something was pushed.
    unsafe fn call_x(f: usize, r_in: [u32; 6], args: &[u32]) -> Blk {
        let mut b = Blk {
            r_in,
            f: f as u32,
            args: args.as_ptr() as u32,
            nargs: args.len() as u32,
            r_out: [0; 6],
            slot: 0,
            status: 0,
            st0: 0,
        };
        unsafe {
            std::arch::asm!(
                "push ebx", "push esi", "push edi", "push ebp",
                "mov ebp, esp",
                "push eax",
                "push dword ptr [eax + 24]",
                "mov ebx, eax",
                "mov ecx, [ebx + 32]",
                "mov esi, [ebx + 28]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "mov eax, [ebx]", "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]",
                "mov esi, [ebx + 16]", "mov edi, [ebx + 20]", "mov ebx, [ebx + 12]",
                "call dword ptr [ebp - 8]",
                "xchg eax, [ebp - 4]",
                "mov [eax + 40], ecx", "mov [eax + 44], edx", "mov [eax + 48], ebx",
                "mov [eax + 52], esi", "mov [eax + 56], edi",
                "mov ecx, [ebp - 4]", "mov [eax + 36], ecx",
                "mov ecx, [esp]", "mov [eax + 60], ecx",
                "fnstsw word ptr [eax + 64]",
                "movzx ecx, word ptr [eax + 64]",
                "and ecx, 0x3800",
                "jz 4f",
                "fstp qword ptr [eax + 68]",
                "4:",
                "mov esp, ebp",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                inout("eax") &mut b as *mut Blk => _, lateout("ecx") _, lateout("edx") _,
                clobber_abi("C"),
            );
        }
        b
    }

    const SENT: [u32; 3] = [0xB1B1_B1B1, 0xC2C2_C2C2, 0xD3D3_D3D3]; // EBX, ESI, EDI sentinels

    /// A pair of identical word buffers (the original runs on `a`, the port on `b`).
    struct Pair {
        a: Vec<u32>,
        b: Vec<u32>,
    }
    impl Pair {
        fn new(words: usize) -> Pair {
            Pair { a: vec![0; words], b: vec![0; words] }
        }
        fn sync(&mut self) {
            self.b.copy_from_slice(&self.a);
        }
        fn pa(&self, byte_off: usize) -> u32 {
            self.a.as_ptr() as u32 + byte_off as u32
        }
        fn pb(&self, byte_off: usize) -> u32 {
            self.b.as_ptr() as u32 + byte_off as u32
        }
        fn put(&mut self, byte_off: usize, v: u32) {
            self.a[byte_off / 4] = v;
        }
        fn diff(&self, lo: usize, hi: usize) -> Option<(usize, u32, u32)> {
            (lo / 4..hi / 4).find(|&i| !same(self.a[i], self.b[i])).map(|i| (i * 4, self.a[i], self.b[i]))
        }
        fn diff_exact(&self, lo: usize, hi: usize) -> Option<(usize, u32, u32)> {
            (lo / 4..hi / 4).find(|&i| self.a[i] != self.b[i]).map(|i| (i * 4, self.a[i], self.b[i]))
        }
    }

    fn report(name: &str, n: u32, bad: u32) {
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{name}: {bad} of {n} cases differ");
    }

    // --- 1. vec3_mul_mat33_to ----------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn vec3_mul_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let (mut bad, n) = (0, cases());
        for case in 0..n {
            let mut p = Pair::new(64);
            for i in 0..64 {
                p.a[i] = rng.fl();
            }
            p.sync();
            let alias = rng.one_in(4);
            let (m, v, o) = (0usize, 64usize, if alias { 64 } else { 128 });
            let ra = [0x1111_1111, p.pa(m), p.pa(v), SENT[0], SENT[1], SENT[2]];
            let rb = [0x1111_1111, p.pb(m), p.pb(v), SENT[0], SENT[1], SENT[2]];
            let oa = unsafe { call_x(og.vec3, ra, &[p.pa(o)]) };
            let ob = unsafe { call_x(vec3_mul_thunk as *const () as usize, rb, &[p.pb(o)]) };
            let mut why = String::new();
            if let Some(d) = p.diff(0, 256) {
                why = format!("memory @{:#x}: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            if oa.eax() != p.pa(o) || ob.eax() != p.pb(o) {
                why = format!("eax orig {:#x} port {:#x}", oa.eax(), ob.eax());
            }
            for k in 1..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != rb[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if oa.depth() != 0 || ob.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("vec3_mul case {case}: {why}");
                }
            }
        }
        report("vec3_mul_mat33_to", n, bad);
    }

    // --- 2. aabb_transform_by_mat33_extents ---------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn aabb_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0x2468_ACE0_1357_9BDF);
        let (mut bad, n) = (0, cases());
        for case in 0..n {
            let mut p = Pair::new(96);
            for i in 0..96 {
                p.a[i] = rng.fl();
            }
            // make the "box" usually ordered, as the callers' boxes are
            if !rng.one_in(3) {
                for k in 0..3 {
                    let (a, b) = (f32::from_bits(p.a[16 + k]), f32::from_bits(p.a[24 + k]));
                    if a > b {
                        p.a[16 + k] = b.to_bits();
                        p.a[24 + k] = a.to_bits();
                    }
                }
            }
            p.sync();
            let (m, lo, hi, out) = (0usize, 64usize, 96usize, 160usize);
            // alias modes: 0 none, 1 lo = out, 2 hi = out + 12, 3 m = out, 4 lo overlaps out+4
            let mode = if rng.one_in(3) { 1 + rng.below(4) } else { 0 };
            let (m, lo, hi) = match mode {
                1 => (m, out, hi),
                2 => (m, lo, out + 12),
                3 => (out, lo, hi),
                4 => (m, out + 4, hi),
                _ => (m, lo, hi),
            };
            let ra = [0x7777_7777, p.pa(out), p.pa(hi), SENT[0], p.pa(lo), p.pa(m)];
            let rb = [0x7777_7777, p.pb(out), p.pb(hi), SENT[0], p.pb(lo), p.pb(m)];
            let oa = unsafe { call_x(og.aabb, ra, &[]) };
            let ob = unsafe { call_x(aabb_thunk as *const () as usize, rb, &[]) };
            let mut why = String::new();
            if let Some(d) = p.diff(0, 384) {
                why = format!("memory @{:#x}: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            for k in 0..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != rb[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if oa.depth() != 0 || ob.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("aabb case {case} mode {mode}: {why}");
                }
            }
        }
        report("aabb_transform_by_mat33_extents", n, bad);
    }

    // --- 3. / 6. the two quicksorts -----------------------------------------------------------

    fn sort_test(name: &str, which: fn(&Origs) -> usize, port: usize, wide: bool) {
        let (_g, og) = setup();
        let orig = which(og);
        let mut rng = Rng(0x0F0F_1234_5555_AAAA ^ port as u64);
        let (mut bad, n) = (0, cases());
        for case in 0..n {
            let len = match rng.below(6) {
                0 => rng.below(4) + 1,
                1 => rng.below(40) + 1,
                _ => rng.below(300) + 1,
            } as usize;
            let mut p = Pair::new(2 * (len + 8));
            let key_mode = rng.below(5);
            for i in 0..len + 8 {
                let k = match key_mode {
                    0 => 5,
                    1 => rng.below(6),
                    2 => rng.below(1000),
                    3 => rng.u32(),
                    _ => rng.mix_high(),
                };
                if wide {
                    // (dword0, dword1) both unsigned: duplicates in dword0, varied dword1
                    p.a[2 * i] = k;
                    p.a[2 * i + 1] = match rng.below(3) {
                        0 => rng.below(3),
                        1 => rng.u32(),
                        _ => 0xFFFF_FFFF - rng.below(3),
                    };
                } else {
                    p.a[2 * i] = k;
                    p.a[2 * i + 1] = i as u32 * 7 + 1; // payload shows the order of equal keys
                }
            }
            p.sync();
            let (lo, hi) = match rng.below(5) {
                0 => (rng.below(len as u32) as i32, len as i32 - 1),
                1 => {
                    let a = rng.below(len as u32) as i32;
                    (a, a + rng.below((len as u32 - a as u32).max(1)) as i32)
                }
                2 => (1.min(len as i32 - 1), len as i32 - 1),
                _ => (0, len as i32 - 1),
            };
            let ra = [0x1111_1111, 0x2222_2222, 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let oa = unsafe { call_x(orig, ra, &[p.pa(0), lo as u32, hi as u32, 0xDEAD_0001]) };
            let ob = unsafe { call_x(port, ra, &[p.pb(0), lo as u32, hi as u32, 0xDEAD_0001]) };
            let mut why = String::new();
            if let Some(d) = p.diff_exact(0, p.a.len() * 4) {
                why = format!("memory @{:#x}: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            for k in 3..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != ra[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if oa.depth() != 0 || ob.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name} case {case} len {len} lo {lo} hi {hi} keys {key_mode}: {why}");
                }
            }
        }
        report(name, n, bad);
    }

    impl Rng {
        /// Keys around the sign bit, to test the unsigned compares.
        fn mix_high(&mut self) -> u32 {
            [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFE, 0xFFFF_FFFF][self.below(7) as usize]
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn qsort32_matches_original() {
        sort_test("quicksort_pairs_u32_by_key", |o| o.q32, qsort32_rs as *const () as usize, false);
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn qsort64_matches_original() {
        sort_test("quicksort_u64_pairs", |o| o.q64, qsort64_rs as *const () as usize, true);
    }

    // --- 4. rig_bone_matrices_transpose_to_palette --------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn rig_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0x7E57_0000_1234_4321);
        let (mut bad, n) = (0, cases());
        for case in 0..n {
            let nb = match rng.below(8) {
                0 => 0i32,
                1 => -(rng.below(3) as i32),
                _ => rng.below(40) as i32 + 1,
            };
            let nbp = nb.max(0) as usize;
            // layout (bytes): rig 0x240 | ctx 0x108 + 2n + pad | src n*0x30 | dst n*0x30 (+ guard)
            let rig = 0usize;
            let ctx = 0x240usize;
            let src = (ctx + 0x108 + 2 * nbp + 0x20 + 15) & !15;
            let dst = src + nbp * 0x30 + 0x40;
            let total = dst + nbp * 0x30 + 0x40;
            let mut p = Pair::new(total / 4 + 4);
            for i in 0..p.a.len() {
                p.a[i] = if rng.one_in(50) { 0x7F80_0000 | (rng.u32() & 0x807F_FFFF) | 1 } else { rng.fl() };
            }
            // words: mostly non-negative, some negative
            for i in 0..nbp {
                let w: u16 = if rng.one_in(3) { 0x8000 | (rng.u32() as u16 & 0x7FFF) } else { rng.u32() as u16 & 0x7FFF };
                let o = ctx + 0x108 + 2 * i;
                let cur = p.a[o / 4];
                p.a[o / 4] = if o % 4 == 0 { (cur & 0xFFFF_0000) | w as u32 } else { (cur & 0xFFFF) | ((w as u32) << 16) };
            }
            p.put(ctx + 4, nb as u32);
            p.sync();
            // pointers differ between the copies
            p.a[rig / 4] = p.pa(src);
            p.a[(rig + 0x228) / 4] = p.pa(ctx);
            p.b[rig / 4] = p.pb(src);
            p.b[(rig + 0x228) / 4] = p.pb(ctx);
            let ra = [0x1111_1111, p.pa(rig), 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let rb = [0x1111_1111, p.pb(rig), 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let oa = unsafe { call_x(og.rig, ra, &[p.pa(dst)]) };
            let ob = unsafe { call_x(rig_thunk as *const () as usize, rb, &[p.pb(dst)]) };
            let mut why = String::new();
            if let Some(d) = p.diff_exact(dst, total) {
                why = format!("dst @{:#x}: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            if let Some(d) = p.diff_exact(src, dst) {
                why = format!("src @{:#x} changed: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            for k in 3..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != ra[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if oa.depth() != 0 || ob.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("rig case {case} n {nb}: {why}");
                }
            }
        }
        report("rig_bone_matrices_transpose_to_palette", n, bad);
    }

    // --- 5. bsp_point_in_volume ----------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn bsp_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0xB59B_0000_7777_1357);
        let (mut bad, n) = (0, cases());
        let (mut ins, mut outs) = (0, 0);
        for case in 0..n {
            let nplanes = rng.below(6) as usize + 1;
            let nnodes = rng.below(14) as usize + 1;
            // layout: tree 0x20 | planes | nodes (6 bytes each) | point
            let tree = 0usize;
            let planes = 0x40usize;
            let nodes = planes + nplanes * 16;
            let pt = (nodes + nnodes * 6 + 3 + 15) & !15;
            let total = pt + 16;
            let mut p = Pair::new(total / 4 + 2);
            let scale = [1.0f32, 10.0, 100.0][rng.below(3) as usize];
            for i in 0..nplanes {
                for k in 0..4 {
                    p.a[(planes + i * 16) / 4 + k] = if rng.one_in(20) { rng.fl() } else { rng.small(if k == 3 { scale } else { 1.0 }) };
                }
            }
            // nodes: node 0 is the root; children point to later nodes (acyclic) or 0
            for i in 0..nnodes {
                let plane = rng.below(nplanes as u32) as u16;
                let child = |r: &mut Rng| -> u16 {
                    if i + 1 >= nnodes || r.one_in(3) { 0 } else { (i + 1 + r.below((nnodes - i - 1) as u32) as usize) as u16 }
                };
                let (ca, cb) = (child(&mut rng), child(&mut rng));
                for (k, w) in [plane, ca, cb].iter().enumerate() {
                    let o = nodes + i * 6 + k * 2;
                    let cur = p.a[o / 4];
                    p.a[o / 4] = if o % 4 == 0 { (cur & 0xFFFF_0000) | *w as u32 } else { (cur & 0xFFFF) | ((*w as u32) << 16) };
                }
            }
            for k in 0..3 {
                p.a[pt / 4 + k] = rng.small(scale);
            }
            let margin = match rng.below(6) {
                0 => 0,
                1 => 0x8000_0000,
                2 => rng.fl(),
                _ => rng.small(2.0),
            };
            p.sync();
            p.a[tree / 4 + 2] = p.pa(planes);
            p.a[tree / 4 + 3] = p.pa(nodes);
            p.b[tree / 4 + 2] = p.pb(planes);
            p.b[tree / 4 + 3] = p.pb(nodes);
            let ra = [p.pa(pt), p.pa(tree), 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let rb = [p.pb(pt), p.pb(tree), 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let oa = unsafe { call_x(og.bsp, ra, &[margin]) };
            let ob = unsafe { call_x(bsp_thunk as *const () as usize, rb, &[margin]) };
            if oa.eax() == 1 {
                ins += 1;
            } else {
                outs += 1;
            }
            let mut why = String::new();
            if oa.eax() != ob.eax() {
                why = format!("eax orig {:#x} port {:#x}", oa.eax(), ob.eax());
            }
            for k in 3..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != ra[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if oa.depth() != 0 || ob.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("bsp case {case}: {why}");
                }
            }
        }
        eprintln!("bsp: {ins} inside, {outs} outside");
        report("bsp_point_in_volume", n, bad);
    }

    // --- 7. memcmp_regs ------------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn memcmp_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0x3E3C_0000_9999_4242);
        let (mut bad, n) = (0, cases());
        let mut counts = [0u32; 3];
        for case in 0..n {
            let mut ba = vec![0u8; 128];
            let mut bb = vec![0u8; 128];
            for i in 0..128 {
                ba[i] = if rng.one_in(3) { 0x80 | rng.u32() as u8 } else { rng.u32() as u8 };
                bb[i] = rng.u32() as u8;
            }
            let (oa, ob) = (rng.below(8) as usize, rng.below(8) as usize);
            let len = match rng.below(4) {
                0 => rng.below(5),
                1 => rng.below(12),
                _ => rng.below(70),
            } as usize;
            // b equals a over [0, len) (at another alignment), then usually differs at one position
            for i in 0..len {
                bb[ob + i] = ba[oa + i];
            }
            if len > 0 && !rng.one_in(4) {
                let pos = rng.below(len as u32) as usize;
                bb[ob + pos] = ba[oa + pos].wrapping_add(1 + rng.below(250) as u8);
                if rng.one_in(2) {
                    bb[ob + (pos + 1) % len] ^= 0x55;
                }
            }
            let a_ptr = ba.as_ptr() as usize + oa;
            let b_ptr = bb.as_ptr() as usize + ob;
            let ra = [len as u32, a_ptr as u32, 0x3333_3333, SENT[0], SENT[1], SENT[2]];
            let r0 = unsafe { call_x(og.memcmp, ra, &[b_ptr as u32]) };
            let p = unsafe { call_x(memcmp_thunk as *const () as usize, ra, &[b_ptr as u32]) };
            counts[match r0.eax() {
                0 => 0,
                1 => 1,
                _ => 2,
            }] += 1;
            let mut why = String::new();
            if r0.eax() != p.eax() {
                why = format!("eax orig {:#x} port {:#x}", r0.eax(), p.eax());
            }
            for k in 3..6 {
                if r0.ro(k) != ra[k] || p.ro(k) != ra[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if r0.depth() != 0 || p.depth() != 0 {
                why = "x87 not empty".into();
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("memcmp case {case} len {len}: {why}");
                }
            }
        }
        eprintln!("memcmp: equal {} / b>a {} / b<a {}", counts[0], counts[1], counts[2]);
        report("memcmp_regs", n, bad);
    }

    // --- 8. segment_closest_t_dist_sq -----------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn segment_matches_original() {
        let (_g, og) = setup();
        let mut rng = Rng(0x5E65_0000_ABCD_0123);
        let (mut bad, n) = (0, cases());
        let mut kinds = [0u32; 4]; // degenerate, t<=0, 0<t<1, t>=1
        for case in 0..n {
            let mut p = Pair::new(32);
            // layout (bytes): E 0 | S 16 | B 32 | out_t 48
            let (e, s, b, ot) = (0usize, 16usize, 32usize, 48usize);
            let scale = [1.0f32, 50.0, 1000.0][rng.below(3) as usize];
            let spec = rng.below(14) == 0;
            for k in 0..3 {
                p.a[s / 4 + k] = if spec { rng.fl() } else { rng.small(scale) };
                p.a[b / 4 + k] = if spec { rng.fl() } else { rng.small(scale) };
                p.a[e / 4 + k] = if spec { rng.fl() } else { rng.small(scale) };
            }
            match rng.below(8) {
                0 => {
                    // E == S
                    for k in 0..3 {
                        p.a[e / 4 + k] = p.a[s / 4 + k];
                    }
                }
                1 | 2 => {
                    // E a hair from S (dot around 1e-10)
                    let tiny = [1e-4f32, 5e-6, 1e-5, 2e-5, 1e-7][rng.below(5) as usize];
                    for k in 0..3 {
                        let sv = f32::from_bits(p.a[s / 4 + k]);
                        p.a[e / 4 + k] = (sv + f32::from_bits(rng.small(tiny))).to_bits();
                    }
                }
                3 => {
                    // B on the segment's line, beyond the ends or inside
                    let tt = [-0.5f32, 0.0, 0.3, 1.0, 1.5, 0.999999][rng.below(6) as usize];
                    for k in 0..3 {
                        let (sv, ev) = (f32::from_bits(p.a[s / 4 + k]), f32::from_bits(p.a[e / 4 + k]));
                        p.a[b / 4 + k] = (sv + (ev - sv) * tt).to_bits();
                    }
                }
                _ => {}
            }
            p.a[ot / 4] = rng.u32();
            p.sync();
            let ra = [p.pa(e), 0x2222_2222, p.pa(s), SENT[0], p.pa(ot), SENT[2]];
            let rb = [p.pb(e), 0x2222_2222, p.pb(s), SENT[0], p.pb(ot), SENT[2]];
            let oa = unsafe { call_x(og.seg, ra, &[p.pa(b)]) };
            let ob = unsafe { call_x(seg_thunk as *const () as usize, rb, &[p.pb(b)]) };
            let t = f32::from_bits(p.a[ot / 4]);
            kinds[if oa.depth() == 7 && p.a[ot / 4] == 0x3F00_0000 { 0 } else if t <= 0.0 { 1 } else if t < 1.0 { 2 } else { 3 }] += 1;
            let mut why = String::new();
            if let Some(d) = p.diff(0, 128) {
                why = format!("memory @{:#x}: orig {:#x} port {:#x}", d.0, d.1, d.2);
            }
            let (so, sp) = (f64::from_bits(oa.st0()), f64::from_bits(ob.st0()));
            if !(oa.st0() == ob.st0() || (so.is_nan() && sp.is_nan())) {
                why = format!("ST0 orig {so:e} ({:#x}) port {sp:e} ({:#x})", oa.st0(), ob.st0());
            }
            if oa.depth() != 7 || ob.depth() != 7 {
                why = format!("x87 depth orig {} port {}", oa.depth(), ob.depth());
            }
            if !same(oa.slot(), ob.slot()) {
                why = format!("stack slot orig {:#x} port {:#x}", oa.slot(), ob.slot());
            }
            // EAX = E (normal) or S (degenerate); ECX = B; EDX, ESI, EBX, EDI unchanged
            if oa.eax().wrapping_sub(p.pa(0)) != ob.eax().wrapping_sub(p.pb(0)) {
                why = format!("eax orig {:#x} port {:#x}", oa.eax().wrapping_sub(p.pa(0)), ob.eax().wrapping_sub(p.pb(0)));
            }
            if oa.ro(1).wrapping_sub(p.pa(0)) != ob.ro(1).wrapping_sub(p.pb(0)) {
                why = "ecx differs".into();
            }
            for k in 2..6 {
                if oa.ro(k) != ra[k] || ob.ro(k) != rb[k] {
                    why = format!("register {k} not preserved");
                }
            }
            if !why.is_empty() {
                bad += 1;
                if bad <= 5 {
                    eprintln!("segment case {case}: {why}");
                }
            }
        }
        eprintln!("segment: degenerate {} / t<=0 {} / 0<t<1 {} / t>=1 {}", kinds[0], kinds[1], kinds[2], kinds[3]);
        report("segment_closest_t_dist_sq", n, bad);
    }
}

