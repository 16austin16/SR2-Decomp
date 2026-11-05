//! Render-thread hot paths ported by hand: the frustum / occlusion box test, the signed-float to
//! ubyte4 normal packer and the view-bounds visibility test (mission-log sample of the render
//! thread).
//!
//! All three are pure: they read game memory and (view_bounds_visible) call original functions,
//! and write nothing. The shadow runs a relocated copy of the original for the first
//! `SHADOW_CALLS` calls of each and keeps the original's result on a mismatch; after that the Rust
//! body runs alone (one relaxed atomic load of overhead). No Rust state is shared with other
//! threads except the atomic counters.
//!
//! Outputs reproduced:
//! - 0x530270 frustum_test_aabb: the whole of EAX (0 / 1 on the SSE path, the original's mixed
//!   value on the legacy path); ECX / EDX are preserved by the thunk (a superset).
//! - 0x9EE620 normal_pack_ubyte4 (stdcall, `ret 0x14`): the packed dword stored at `out`, EAX = out,
//!   ECX = packed, EDX = the last raw conversion (the x byte before masking), x87 stack empty.
//! - 0x530450 view_bounds_visible: AL (1 when visible), the rest of EAX is 0 / 1 on the
//!   paths callers see (they test AL only). ESI / EDI are inputs, preserved. ECX / EDX preserved.
//!
//! Deviations: frustum_test_aabb with a plane count of 0 loops 2^32 times in the original (reads
//! far past the table); the port treats it as no planes (passes). The Rust loop also stops at the
//! first plane that decides the answer, where the original always runs the whole set (it only
//! reads memory, so the result is the same).

use std::arch::x86::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const FRUSTUM_TEST_AABB: usize = 0x0053_0270;
const NORMAL_PACK_UBYTE4: usize = 0x009E_E620;
const VIEW_BOUNDS_VISIBLE: usize = 0x0053_0450;

const FRUSTUM_SSE_FLAG: usize = 0x00E8_E88A;
const VIEW_TABLE: usize = 0x0279_18A0;
const OCCLUDER_TABLE: usize = 0x0261_2D10;
const DIST_MASK: usize = 0x00E8_3E50;
const PLANES_SOFT: usize = 0x00A0_20C0;
const CELL_SOFT: usize = 0x00A0_23F0;
const VIEW_FRUSTUM_TEST: usize = 0x00BE_A660;

// --- bodies (python tools/body_bytes.py) --------------------------------------------------------

const FRUSTUM_BODY: [u8; 480] = [
    0x53, 0x8B, 0xDC, 0x83, 0xEC, 0x08, 0x83, 0xE4, 0xF0, 0x83, 0xC4, 0x04, 0x55, 0x8B, 0x6B, 0x04,
    0x89, 0x6C, 0x24, 0x04, 0x8B, 0xEC, 0x83, 0xEC, 0x48, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC5,
    0x89, 0x45, 0xFC, 0x80, 0x3D, 0x8A, 0xE8, 0xE8, 0x00, 0x00, 0x8B, 0x43, 0x08, 0x56, 0x8B, 0x73,
    0x0C, 0x57, 0x8B, 0xFA, 0x89, 0x4D, 0xC0, 0x89, 0x45, 0xC8, 0x89, 0x75, 0xC4, 0x0F, 0x84, 0x34,
    0x01, 0x00, 0x00, 0xF3, 0x0F, 0x10, 0x47, 0x08, 0xF3, 0x0F, 0x10, 0x0D, 0x8C, 0x51, 0xDD, 0x00,
    0xF3, 0x0F, 0x11, 0x45, 0xE8, 0xF3, 0x0F, 0x10, 0x05, 0xC0, 0x32, 0xDD, 0x00, 0xF3, 0x0F, 0x11,
    0x4D, 0xEC, 0xF3, 0x0F, 0x11, 0x45, 0xE4, 0xF3, 0x0F, 0x11, 0x45, 0xE0, 0x0F, 0x28, 0x55, 0xE0,
    0x0F, 0x12, 0x17, 0xF3, 0x0F, 0x11, 0x4D, 0xEC, 0xF3, 0x0F, 0x10, 0x49, 0x08, 0xF3, 0x0F, 0x11,
    0x45, 0xE4, 0xF3, 0x0F, 0x11, 0x45, 0xE0, 0xF3, 0x0F, 0x11, 0x4D, 0xE8, 0x0F, 0x28, 0x45, 0xE0,
    0x0F, 0x12, 0x01, 0x0F, 0x29, 0x55, 0xD0, 0x0F, 0x29, 0x45, 0xE0, 0x0F, 0x28, 0x45, 0xD0, 0x0F,
    0x28, 0x4D, 0xE0, 0x8B, 0x45, 0xC8, 0x8B, 0x08, 0x0F, 0x57, 0xF6, 0x0F, 0x28, 0xA0, 0xF0, 0x00,
    0x00, 0x00, 0x0F, 0x28, 0xDC, 0x0F, 0x55, 0xE0, 0x0F, 0x54, 0xD9, 0x0F, 0x56, 0xE3, 0x0F, 0x59,
    0x60, 0x10, 0x0F, 0x57, 0xDB, 0xF2, 0x0F, 0x7C, 0xE4, 0xF2, 0x0F, 0x7C, 0xE4, 0x83, 0xC0, 0x10,
    0xF3, 0x0F, 0xC2, 0xE3, 0x06, 0x83, 0xE9, 0x01, 0x0F, 0x56, 0xF4, 0x75, 0xCE, 0x0F, 0x50, 0xC6,
    0x83, 0xE0, 0x01, 0x83, 0xF0, 0x01, 0x89, 0x45, 0xCC, 0x83, 0x7D, 0xCC, 0x00, 0x75, 0x15, 0x32,
    0xC0, 0x5F, 0x5E, 0x8B, 0x4D, 0xFC, 0x33, 0xCD, 0xE8, 0xE2, 0x5C, 0x76, 0x00, 0x8B, 0xE5, 0x5D,
    0x8B, 0xE3, 0x5B, 0xC3, 0x8B, 0x7B, 0x10, 0x33, 0xD2, 0x85, 0xFF, 0x0F, 0x8E, 0xAF, 0x00, 0x00,
    0x00, 0x83, 0xC6, 0x10, 0x89, 0x75, 0xC4, 0x0F, 0x28, 0x45, 0xD0, 0x0F, 0x28, 0x4D, 0xE0, 0x8B,
    0x45, 0xC4, 0x8B, 0x08, 0x0F, 0x57, 0xF6, 0x0F, 0x28, 0xA0, 0x80, 0x01, 0x00, 0x00, 0x0F, 0x28,
    0xDC, 0x0F, 0x55, 0xE0, 0x0F, 0x54, 0xD9, 0x0F, 0x56, 0xE3, 0x0F, 0x59, 0x60, 0x10, 0x0F, 0x57,
    0xDB, 0xF2, 0x0F, 0x7C, 0xE4, 0xF2, 0x0F, 0x7C, 0xE4, 0x83, 0xC0, 0x10, 0xF3, 0x0F, 0xC2, 0xE3,
    0x06, 0x83, 0xE9, 0x01, 0x0F, 0x56, 0xF4, 0x75, 0xCE, 0x0F, 0x50, 0xC6, 0x83, 0xE0, 0x01, 0x89,
    0x45, 0xC8, 0x83, 0x7D, 0xC8, 0x00, 0x74, 0x87, 0x83, 0xC2, 0x01, 0x81, 0xC6, 0x20, 0x02, 0x00,
    0x00, 0x3B, 0xD7, 0x7C, 0x9F, 0xEB, 0x49, 0x50, 0xE8, 0xD3, 0x1C, 0x4D, 0x00, 0x83, 0xC4, 0x04,
    0x85, 0xC0, 0x0F, 0x84, 0x67, 0xFF, 0xFF, 0xFF, 0x33, 0xF6, 0x39, 0x73, 0x10, 0x7E, 0x31, 0x8B,
    0x45, 0xC4, 0x83, 0xC0, 0x10, 0x89, 0x45, 0xCC, 0x8B, 0x4D, 0xC0, 0x8B, 0x55, 0xCC, 0x51, 0x8B,
    0xCF, 0xE8, 0xDA, 0x1F, 0x4D, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x0F, 0x85, 0x3E, 0xFF, 0xFF,
    0xFF, 0x81, 0x45, 0xCC, 0x20, 0x02, 0x00, 0x00, 0x83, 0xC6, 0x01, 0x3B, 0x73, 0x10, 0x7C, 0xD8,
    0x8B, 0x4D, 0xFC, 0x5F, 0x33, 0xCD, 0xB0, 0x01, 0x5E, 0xE8, 0x11, 0x5C, 0x76, 0x00, 0x8B, 0xE5,
    0x5D, 0x8B, 0xE3, 0x5B, 0xC3, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC,
];
const FRUSTUM_CALLS: [usize; 4] = [0xf8, 0x178, 0x1a1, 0x1c9];

const PACK_BODY: [u8; 284] = [
    0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0xE8, 0xDC, 0xC1, 0xDD, 0x05, 0x70, 0xBF, 0xE5,
    0x00, 0xDC, 0xCA, 0xD9, 0xCA, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xDD, 0x05, 0x38,
    0xC0, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xD8,
    0xC3, 0xD9, 0x7C, 0x24, 0x1C, 0x0F, 0xB7, 0x44, 0x24, 0x1C, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89,
    0x04, 0x24, 0xD9, 0x2C, 0x24, 0xDF, 0x3C, 0x24, 0x8B, 0x0C, 0x24, 0xC1, 0xE1, 0x08, 0xD9, 0x6C,
    0x24, 0x1C, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0xC2, 0xD8, 0xCB, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44,
    0x24, 0x1C, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0xC3, 0xD9, 0x7C,
    0x24, 0x1C, 0x0F, 0xB7, 0x44, 0x24, 0x1C, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18,
    0xD9, 0x6C, 0x24, 0x18, 0xDF, 0x3C, 0x24, 0x8B, 0x14, 0x24, 0x0B, 0xCA, 0xC1, 0xE1, 0x08, 0xD9,
    0x6C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xC2, 0xD8, 0xCB, 0xD9, 0x5C, 0x24, 0x1C, 0xD9,
    0x44, 0x24, 0x1C, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0x7C, 0x24,
    0x1C, 0x0F, 0xB7, 0x44, 0x24, 0x1C, 0xD8, 0xC3, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89, 0x44, 0x24,
    0x18, 0xD9, 0x6C, 0x24, 0x18, 0xDF, 0x3C, 0x24, 0x8B, 0x04, 0x24, 0x0B, 0xC8, 0xC1, 0xE1, 0x08,
    0xD9, 0x6C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x10, 0xDE, 0xC2, 0xD9, 0xC9, 0xD8, 0xCA, 0xD9, 0x5C,
    0x24, 0x1C, 0xD8, 0x4C, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x1C, 0xD9, 0x7C,
    0x24, 0x1C, 0x0F, 0xB7, 0x44, 0x24, 0x1C, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18,
    0x8B, 0x44, 0x24, 0x0C, 0xD9, 0x6C, 0x24, 0x18, 0xDF, 0x3C, 0x24, 0x8B, 0x14, 0x24, 0x0B, 0xCA,
    0x89, 0x08, 0xD9, 0x6C, 0x24, 0x1C, 0x83, 0xC4, 0x08, 0xC2, 0x14, 0x00,
];
const PACK_CALLS: [usize; 0] = [];

const VBV_BODY: [u8; 158] = [
    0x51, 0x8D, 0x14, 0x76, 0xC1, 0xE2, 0x04, 0x81, 0xC2, 0xA0, 0x18, 0x79, 0x02, 0x80, 0x7A, 0x20,
    0x00, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x74, 0x32, 0xD9, 0x42, 0x0C, 0x53, 0xD9, 0x5C, 0x24, 0x08,
    0x8B, 0xCF, 0xD9, 0x44, 0x24, 0x08, 0xE8, 0x35, 0x5F, 0x6C, 0x00, 0xF3, 0x0F, 0x11, 0x44, 0x24,
    0x08, 0xD9, 0x44, 0x24, 0x08, 0x83, 0xC4, 0x04, 0xD9, 0xC1, 0xDE, 0xCA, 0xDE, 0xD9, 0xDF, 0xE0,
    0xF6, 0xC4, 0x41, 0x7B, 0x05, 0x32, 0xC0, 0x5B, 0x59, 0xC3, 0x80, 0x7A, 0x21, 0x00, 0x74, 0x18,
    0xD9, 0x42, 0x1C, 0x51, 0xD9, 0x1C, 0x24, 0x57, 0x8D, 0x42, 0x10, 0x53, 0xE8, 0xAF, 0xA1, 0x6B,
    0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x74, 0xDD, 0x8D, 0x04, 0xB6, 0xC1, 0xE0, 0x07, 0x05, 0x10,
    0x2D, 0x61, 0x02, 0x80, 0x78, 0x64, 0x00, 0x75, 0x1D, 0x6A, 0x00, 0x83, 0xC0, 0x70, 0x6A, 0x00,
    0x50, 0x8B, 0xCF, 0x8B, 0xD3, 0xE8, 0x96, 0xFD, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x75,
    0x05, 0x33, 0xC0, 0x5B, 0x59, 0xC3, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5B, 0x59, 0xC3,
];
const VBV_CALLS: [usize; 3] = [0x26, 0x5c, 0x85];

const ADD_BODY: [u8; 533] = [
    0x81, 0xEC, 0xCC, 0x02, 0x00, 0x00, 0x53, 0x8B, 0x9C, 0x24, 0xE0, 0x02, 0x00, 0x00, 0x83, 0xFB,
    0x01, 0x55, 0x56, 0x57, 0x74, 0x05, 0x83, 0xFB, 0x02, 0x75, 0x69, 0x8B, 0xB4, 0x24, 0xE4, 0x02,
    0x00, 0x00, 0x8B, 0x84, 0x24, 0xE0, 0x02, 0x00, 0x00, 0x8D, 0x34, 0xB6, 0xC1, 0xE6, 0x04, 0x03,
    0x70, 0x4C, 0x8B, 0x4E, 0x48, 0x85, 0xC9, 0x74, 0x0E, 0x8B, 0x46, 0x30, 0x25, 0x00, 0x00, 0x01,
    0x00, 0x33, 0xD2, 0x0B, 0xC2, 0x75, 0x03, 0x8B, 0x4E, 0x40, 0x8B, 0x46, 0x34, 0x8B, 0x7E, 0x30,
    0x89, 0x4C, 0x24, 0x10, 0x8B, 0xD0, 0x81, 0xE2, 0x00, 0x04, 0x00, 0x00, 0x33, 0xC9, 0x0B, 0xCA,
    0x74, 0x09, 0x80, 0x3D, 0x17, 0x7D, 0x52, 0x02, 0x00, 0x75, 0x19, 0x83, 0xFB, 0x01, 0x8B, 0xCF,
    0x8B, 0xD0, 0x75, 0x1D, 0x81, 0xE1, 0x20, 0x80, 0x38, 0xC0, 0x81, 0xE2, 0x00, 0x20, 0x02, 0x00,
    0x0B, 0xCA, 0x74, 0x2A, 0x32, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0xCC, 0x02, 0x00, 0x00,
    0xC3, 0x81, 0xE1, 0x20, 0x80, 0x38, 0xC1, 0x81, 0xE2, 0x00, 0x20, 0x00, 0x00, 0x0B, 0xCA, 0x74,
    0x0D, 0x32, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0xCC, 0x02, 0x00, 0x00, 0xC3, 0x25, 0x80,
    0x00, 0x00, 0x00, 0x33, 0xC9, 0x0B, 0xC8, 0x74, 0x09, 0x80, 0x3D, 0x04, 0x7E, 0x52, 0x02, 0x00,
    0x74, 0xC2, 0x8B, 0xC7, 0x25, 0x00, 0x00, 0x00, 0x02, 0x33, 0xC9, 0x0B, 0xC1, 0x74, 0x04, 0xB3,
    0x01, 0xEB, 0x02, 0x32, 0xDB, 0x8B, 0x2D, 0x00, 0x2D, 0x61, 0x02, 0x85, 0xED, 0x74, 0x24, 0x8B,
    0x4D, 0x20, 0x8B, 0xC1, 0xC1, 0xE8, 0x03, 0x24, 0x01, 0x74, 0x0D, 0x8B, 0xC7, 0x25, 0x00, 0x00,
    0x00, 0x08, 0x33, 0xD2, 0x0B, 0xC2, 0x75, 0x1C, 0x8B, 0xC1, 0xC1, 0xE8, 0x02, 0xF6, 0xD0, 0x24,
    0x01, 0x74, 0x81, 0x8B, 0xC7, 0x25, 0x00, 0x00, 0x00, 0x10, 0x33, 0xC9, 0x0B, 0xC1, 0x0F, 0x84,
    0x70, 0xFF, 0xFF, 0xFF, 0x85, 0xED, 0x74, 0x1A, 0x8B, 0x45, 0x20, 0xC1, 0xE8, 0x08, 0x24, 0x01,
    0x74, 0x10, 0x81, 0xE7, 0x00, 0x00, 0x80, 0x00, 0x33, 0xC0, 0x0B, 0xF8, 0x0F, 0x85, 0x52, 0xFF,
    0xFF, 0xFF, 0x8D, 0x44, 0x24, 0x14, 0xC7, 0x44, 0x24, 0x44, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x8D,
    0xF7, 0xFF, 0xFF, 0x8B, 0x56, 0x20, 0x8B, 0x4E, 0x30, 0x8B, 0x7E, 0x34, 0x89, 0x94, 0x24, 0xD0,
    0x02, 0x00, 0x00, 0x8B, 0xC1, 0x8B, 0xD7, 0x0F, 0xAC, 0xD0, 0x01, 0x66, 0xC1, 0xE0, 0x07, 0xD1,
    0xEA, 0x8B, 0x54, 0x24, 0x14, 0x81, 0xE1, 0x00, 0x08, 0x00, 0x00, 0x33, 0xC2, 0x0F, 0xB7, 0xD2,
    0x25, 0x80, 0x01, 0x00, 0x00, 0x33, 0xC2, 0x8B, 0x15, 0x2C, 0x57, 0x61, 0x02, 0x89, 0x54, 0x24,
    0x24, 0x8B, 0x15, 0x30, 0x57, 0x61, 0x02, 0x89, 0x54, 0x24, 0x28, 0x8B, 0x15, 0x34, 0x57, 0x61,
    0x02, 0x89, 0x54, 0x24, 0x2C, 0x8A, 0x94, 0x24, 0xE8, 0x02, 0x00, 0x00, 0x88, 0x94, 0x24, 0xBD,
    0x02, 0x00, 0x00, 0x0D, 0x01, 0x02, 0x00, 0x00, 0x33, 0xD2, 0x0B, 0xCA, 0x89, 0x44, 0x24, 0x14,
    0x74, 0x07, 0x83, 0xC8, 0x20, 0x89, 0x44, 0x24, 0x14, 0x84, 0xDB, 0x74, 0x1D, 0x83, 0xE7, 0x08,
    0x33, 0xC9, 0x0B, 0xCF, 0x74, 0x14, 0x0D, 0x00, 0x04, 0x00, 0x00, 0xC7, 0x84, 0x24, 0xC0, 0x02,
    0x00, 0x00, 0xE0, 0x7B, 0x2D, 0x02, 0x89, 0x44, 0x24, 0x14, 0x8B, 0x8C, 0x24, 0xE4, 0x02, 0x00,
    0x00, 0x8B, 0x94, 0x24, 0xE0, 0x02, 0x00, 0x00, 0x6A, 0x00, 0x8D, 0x46, 0x10, 0x50, 0x8B, 0x44,
    0x24, 0x18, 0x56, 0x51, 0x8B, 0x8C, 0x24, 0xFC, 0x02, 0x00, 0x00, 0x52, 0x50, 0x51, 0x8D, 0x44,
    0x24, 0x30, 0xE8, 0xE9, 0x9C, 0x07, 0x00, 0x83, 0xC4, 0x1C, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4,
    0xCC, 0x02, 0x00, 0x00, 0xC3,
];

const SUBMIT_BODY: [u8; 1286] = [
    0x81, 0xEC, 0xF4, 0x02, 0x00, 0x00, 0x83, 0xBC, 0x24, 0x04, 0x03, 0x00, 0x00, 0x02, 0x53, 0x55,
    0x56, 0x57, 0x75, 0x0D, 0x32, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0xF4, 0x02, 0x00, 0x00,
    0xC3, 0x8B, 0xBC, 0x24, 0x0C, 0x03, 0x00, 0x00, 0x8B, 0x9C, 0x24, 0x08, 0x03, 0x00, 0x00, 0x8D,
    0x2C, 0xBF, 0xC1, 0xE5, 0x04, 0x03, 0x6B, 0x4C, 0x8B, 0x4D, 0x48, 0x85, 0xC9, 0x74, 0x14, 0x8B,
    0x45, 0x30, 0x25, 0x00, 0x00, 0x01, 0x00, 0x33, 0xD2, 0x0B, 0xC2, 0x74, 0x06, 0x89, 0x4C, 0x24,
    0x14, 0xEB, 0x07, 0x8B, 0x45, 0x40, 0x89, 0x44, 0x24, 0x14, 0x80, 0xBC, 0x24, 0x10, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x40, 0x8B, 0x45, 0x0C, 0xF6, 0x80, 0xCC, 0x00, 0x00, 0x00, 0x10, 0x74, 0x34,
    0x83, 0xB8, 0xD0, 0x00, 0x00, 0x00, 0x03, 0x75, 0x2B, 0x8B, 0x80, 0xA0, 0x01, 0x00, 0x00, 0xE8,
    0xCC, 0xB7, 0x4A, 0x00, 0x85, 0xC0, 0x74, 0x1C, 0x8B, 0x80, 0x80, 0x00, 0x00, 0x00, 0x8B, 0xC8,
    0xC1, 0xE9, 0x04, 0xF6, 0xC1, 0x01, 0x0F, 0x85, 0x78, 0xFF, 0xFF, 0xFF, 0xA8, 0x04, 0x0F, 0x85,
    0x70, 0xFF, 0xFF, 0xFF, 0x8B, 0x45, 0x34, 0x8B, 0x75, 0x30, 0x8B, 0xD0, 0x81, 0xE2, 0x00, 0x04,
    0x00, 0x00, 0x33, 0xC9, 0x0B, 0xCA, 0x74, 0x1C, 0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74,
    0x13, 0x8B, 0x15, 0x10, 0x7D, 0x52, 0x02, 0x80, 0xBA, 0xFF, 0x0A, 0x00, 0x00, 0x00, 0x0F, 0x84,
    0x40, 0xFF, 0xFF, 0xFF, 0x8B, 0xCE, 0x8B, 0xD0, 0x81, 0xE1, 0x20, 0x80, 0x38, 0xC1, 0x81, 0xE2,
    0x00, 0x20, 0x02, 0x00, 0x0B, 0xCA, 0x0F, 0x85, 0x28, 0xFF, 0xFF, 0xFF, 0x8B, 0xD0, 0x83, 0xE2,
    0x10, 0x0B, 0xCA, 0x74, 0x07, 0xB9, 0x01, 0x00, 0x00, 0x00, 0xEB, 0x02, 0x33, 0xC9, 0x33, 0xD2,
    0x83, 0xBC, 0x24, 0x14, 0x03, 0x00, 0x00, 0x04, 0x0F, 0x94, 0xC2, 0x3B, 0xD1, 0x0F, 0x85, 0x01,
    0xFF, 0xFF, 0xFF, 0x25, 0x80, 0x00, 0x00, 0x00, 0x33, 0xC9, 0x0B, 0xC8, 0x74, 0x0D, 0x80, 0x3D,
    0x04, 0x7E, 0x52, 0x02, 0x00, 0x0F, 0x84, 0xE9, 0xFE, 0xFF, 0xFF, 0x8B, 0xC6, 0x25, 0x00, 0x00,
    0x00, 0x02, 0x33, 0xC9, 0x0B, 0xC1, 0xC6, 0x44, 0x24, 0x13, 0x01, 0x75, 0x04, 0x88, 0x4C, 0x24,
    0x13, 0x8B, 0x44, 0x24, 0x14, 0xF7, 0x40, 0x5C, 0x00, 0x00, 0x40, 0x00, 0x0F, 0x85, 0xC2, 0xFE,
    0xFF, 0xFF, 0x8B, 0x4B, 0x58, 0x66, 0x8B, 0x15, 0xD0, 0x6C, 0x52, 0x02, 0x0F, 0xB7, 0x5D, 0x38,
    0x8D, 0x3C, 0xF9, 0x89, 0x7C, 0x24, 0x34, 0x66, 0x89, 0x55, 0x38, 0xE8, 0x70, 0xEF, 0xFF, 0xFF,
    0x84, 0xC0, 0x74, 0x0D, 0x8B, 0xC6, 0x25, 0x00, 0x00, 0x00, 0x08, 0x33, 0xC9, 0x0B, 0xC1, 0x75,
    0x1E, 0xE8, 0x7A, 0xEF, 0xFF, 0xFF, 0x84, 0xC0, 0x0F, 0x84, 0x86, 0xFE, 0xFF, 0xFF, 0x8B, 0xC6,
    0x25, 0x00, 0x00, 0x00, 0x10, 0x33, 0xC9, 0x0B, 0xC1, 0x0F, 0x84, 0x75, 0xFE, 0xFF, 0xFF, 0xE8,
    0xBC, 0xED, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x10, 0x81, 0xE6, 0x00, 0x00, 0x80, 0x00, 0x33, 0xC0,
    0x0B, 0xF0, 0x0F, 0x85, 0x5C, 0xFE, 0xFF, 0xFF, 0x8B, 0xB4, 0x24, 0x08, 0x03, 0x00, 0x00, 0x8B,
    0xC6, 0xE8, 0xFA, 0x2C, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x13, 0x53, 0x55, 0x56, 0xE8, 0x2E, 0x3C,
    0x00, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x0F, 0x84, 0x37, 0xFE, 0xFF, 0xFF, 0xD9, 0xEE, 0x8B,
    0x44, 0x24, 0x14, 0xD8, 0x58, 0x4C, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x0F, 0x8B, 0x23, 0xFE, 0xFF,
    0xFF, 0x8D, 0x4C, 0x24, 0x38, 0xE8, 0xC6, 0xFB, 0xFF, 0xFF, 0x8B, 0x4D, 0x20, 0x8B, 0x75, 0x34,
    0x89, 0x8C, 0x24, 0xF4, 0x02, 0x00, 0x00, 0x8B, 0x4D, 0x30, 0x8B, 0xD1, 0x8B, 0xC6, 0x0F, 0xAC,
    0xC2, 0x01, 0x0F, 0xB7, 0xDA, 0x8B, 0x54, 0x24, 0x38, 0x66, 0xC1, 0xE3, 0x07, 0xD1, 0xE8, 0x0F,
    0xB7, 0xC2, 0x33, 0xDA, 0x81, 0xE3, 0x80, 0x01, 0x00, 0x00, 0x33, 0xD8, 0x8B, 0xC1, 0x25, 0x00,
    0x08, 0x00, 0x00, 0x33, 0xD2, 0x0B, 0xC2, 0x89, 0x5C, 0x24, 0x38, 0x74, 0x07, 0x83, 0xCB, 0x20,
    0x89, 0x5C, 0x24, 0x38, 0x38, 0x15, 0xBD, 0x6C, 0x52, 0x02, 0x74, 0x0A, 0x81, 0xCB, 0x00, 0x02,
    0x00, 0x00, 0x89, 0x5C, 0x24, 0x38, 0x83, 0xBC, 0x24, 0x14, 0x03, 0x00, 0x00, 0x04, 0x75, 0x0A,
    0x81, 0xCB, 0x00, 0x02, 0x00, 0x00, 0x89, 0x5C, 0x24, 0x38, 0x8B, 0x94, 0x24, 0x08, 0x03, 0x00,
    0x00, 0x8A, 0x42, 0x1C, 0x8A, 0xD0, 0x80, 0xE2, 0x01, 0x74, 0x0A, 0x81, 0xCB, 0x00, 0x00, 0x08,
    0x00, 0x89, 0x5C, 0x24, 0x38, 0x80, 0x3D, 0x16, 0x7D, 0x52, 0x02, 0x01, 0x75, 0x05, 0xC6, 0x44,
    0x24, 0x44, 0x00, 0x83, 0xBC, 0x24, 0x14, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xDB, 0x00, 0x00,
    0x00, 0x81, 0xE1, 0x00, 0x00, 0x00, 0x02, 0x83, 0xE6, 0x08, 0x0B, 0xCE, 0x0F, 0x84, 0xCA, 0x00,
    0x00, 0x00, 0x80, 0x7F, 0x06, 0x00, 0x0F, 0x84, 0xC0, 0x00, 0x00, 0x00, 0xA8, 0x10, 0xD9, 0x45,
    0x10, 0xD8, 0x65, 0x00, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x45, 0x14, 0xD8, 0x65, 0x04, 0xD9, 0x5C,
    0x24, 0x2C, 0xD9, 0x45, 0x18, 0xD8, 0x65, 0x08, 0xD9, 0x5C, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x28,
    0xDD, 0x05, 0x70, 0xBF, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44,
    0x24, 0x2C, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0xD8, 0x4C, 0x24, 0x30, 0xD9, 0x5C, 0x24, 0x20,
    0x75, 0x04, 0x84, 0xD2, 0x74, 0x2D, 0xD9, 0x05, 0x88, 0x1C, 0xDD, 0x00, 0x83, 0xEC, 0x0C, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x05, 0x30, 0xCE, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x45, 0x04,
    0xD9, 0x1C, 0x24, 0xE8, 0xC8, 0xB5, 0x73, 0x00, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x74, 0x04, 0xB0,
    0x01, 0xEB, 0x02, 0x32, 0xC0, 0xD9, 0x45, 0x00, 0xF6, 0xD8, 0xD8, 0x44, 0x24, 0x18, 0x8D, 0x7C,
    0x24, 0x28, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x45, 0x04, 0xD8, 0x44, 0x24, 0x1C, 0x1B, 0xC0, 0x83,
    0xC0, 0x02, 0xD9, 0x5C, 0x24, 0x2C, 0x50, 0xD9, 0x44, 0x24, 0x24, 0xD8, 0x45, 0x08, 0xD9, 0x5C,
    0x24, 0x34, 0xE8, 0x09, 0xE0, 0x07, 0x00, 0x8B, 0x7C, 0x24, 0x38, 0x83, 0xC4, 0x04, 0x81, 0xCB,
    0x00, 0x04, 0x00, 0x00, 0x89, 0x44, 0x24, 0x40, 0x89, 0x5C, 0x24, 0x38, 0x80, 0xBC, 0x24, 0x10,
    0x03, 0x00, 0x00, 0x00, 0x75, 0x53, 0x80, 0x7F, 0x06, 0x00, 0x74, 0x4D, 0x8B, 0x4D, 0x34, 0x81,
    0xE1, 0x00, 0x01, 0x00, 0x00, 0x33, 0xC0, 0x0B, 0xC1, 0x75, 0x3E, 0x8B, 0x44, 0x24, 0x14, 0x8B,
    0x48, 0x58, 0x8B, 0x11, 0x8B, 0x42, 0x20, 0x89, 0x44, 0x24, 0x18, 0x8B, 0x44, 0x24, 0x6C, 0x85,
    0xC0, 0x89, 0x44, 0x24, 0x1C, 0x74, 0x0C, 0x6A, 0x00, 0x50, 0x8D, 0x74, 0x24, 0x20, 0xE8, 0xCD,
    0x2A, 0x00, 0x00, 0x8D, 0x4C, 0x24, 0x18, 0xE8, 0x44, 0xAC, 0x07, 0x00, 0x66, 0x89, 0x47, 0x04,
    0x8D, 0x44, 0x24, 0x18, 0xE8, 0x87, 0x2A, 0x00, 0x00, 0x8B, 0xBC, 0x24, 0x14, 0x03, 0x00, 0x00,
    0xD9, 0xE8, 0x83, 0xFF, 0x04, 0x75, 0x1A, 0xD9, 0x54, 0x24, 0x18, 0x8B, 0x4C, 0x24, 0x18, 0xD9,
    0x54, 0x24, 0x1C, 0x8B, 0x54, 0x24, 0x1C, 0xD9, 0x54, 0x24, 0x20, 0x8B, 0x44, 0x24, 0x20, 0xEB,
    0x2B, 0x80, 0x7C, 0x24, 0x13, 0x00, 0x74, 0x13, 0x8B, 0x0D, 0x04, 0x2D, 0x61, 0x02, 0x8B, 0x15,
    0x08, 0x2D, 0x61, 0x02, 0xA1, 0x0C, 0x2D, 0x61, 0x02, 0xEB, 0x11, 0x8B, 0x0D, 0x20, 0x57, 0x61,
    0x02, 0x8B, 0x15, 0x24, 0x57, 0x61, 0x02, 0xA1, 0x28, 0x57, 0x61, 0x02, 0x8B, 0x75, 0x30, 0x89,
    0x54, 0x24, 0x4C, 0x8B, 0x55, 0x34, 0x89, 0x4C, 0x24, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0xCA,
    0x81, 0xE1, 0x00, 0x02, 0x00, 0x00, 0x33, 0xC0, 0x0B, 0xC1, 0x74, 0x07, 0x83, 0xCB, 0x01, 0x89,
    0x5C, 0x24, 0x38, 0x80, 0x7C, 0x24, 0x13, 0x00, 0x74, 0x1F, 0x83, 0xE2, 0x08, 0x33, 0xC0, 0x0B,
    0xC2, 0x74, 0x16, 0x8B, 0x4C, 0x24, 0x14, 0xDD, 0xD8, 0xD9, 0x41, 0x4C, 0xC7, 0x84, 0x24, 0xE4,
    0x02, 0x00, 0x00, 0xE0, 0x7B, 0x2D, 0x02, 0xEB, 0x23, 0x8B, 0x54, 0x24, 0x14, 0xD8, 0x5A, 0x4C,
    0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x74, 0x0E, 0x81, 0xE6, 0x00, 0x04, 0x00, 0x00, 0x8B, 0xC6, 0x33,
    0xC9, 0x0B, 0xC1, 0x74, 0x12, 0x8B, 0x44, 0x24, 0x14, 0xD9, 0x40, 0x4C, 0x83, 0xCB, 0x40, 0xD9,
    0x5C, 0x24, 0x3C, 0x89, 0x5C, 0x24, 0x38, 0x80, 0xBC, 0x24, 0x10, 0x03, 0x00, 0x00, 0x00, 0x74,
    0x21, 0x8B, 0x54, 0x24, 0x14, 0x55, 0x52, 0x8D, 0x4D, 0x10, 0x57, 0x8D, 0x44, 0x24, 0x44, 0xE8,
    0x1C, 0xA1, 0x07, 0x00, 0x83, 0xC4, 0x0C, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0xF4, 0x02, 0x00,
    0x00, 0xC3, 0x8B, 0x8C, 0x24, 0x0C, 0x03, 0x00, 0x00, 0x8B, 0x94, 0x24, 0x08, 0x03, 0x00, 0x00,
    0x6A, 0x00, 0x8D, 0x45, 0x10, 0x50, 0x8B, 0x44, 0x24, 0x1C, 0x55, 0x51, 0x52, 0x50, 0x57, 0x8D,
    0x44, 0x24, 0x54, 0xE8, 0x08, 0x9F, 0x07, 0x00, 0x83, 0xC4, 0x1C, 0x5F, 0x5E, 0x5D, 0x5B, 0x81,
    0xC4, 0xF4, 0x02, 0x00, 0x00, 0xC3,
];
#[cfg(test)]
const SUBMIT_CALLS: [usize; 14] = [0x7f, 0x16b, 0x181, 0x19f, 0x1c1, 0x1cd, 0x1f5, 0x323, 0x362, 0x3be, 0x3c7, 0x3d4, 0x4bf, 0x4f3];

const EXTRAS_BODY: [u8; 286] = [
    0x83, 0xEC, 0x24, 0xA1, 0x40, 0x59, 0x97, 0x02, 0xD9, 0x05, 0xCC, 0xA8, 0x73, 0x01, 0x83, 0xC0,
    0x01, 0xDD, 0x05, 0x70, 0xBF, 0xE5, 0x00, 0x33, 0xD2, 0xDC, 0xC9, 0xB9, 0x03, 0x00, 0x00, 0x00,
    0xF7, 0xF1, 0x53, 0x55, 0x33, 0xED, 0x39, 0x2D, 0xC4, 0xFB, 0xF6, 0x00, 0x56, 0x57, 0x8D, 0x04,
    0x52, 0x8B, 0x14, 0x85, 0x80, 0x58, 0x97, 0x02, 0x8D, 0x04, 0x85, 0x80, 0x58, 0x97, 0x02, 0x89,
    0x54, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0x8B, 0x50, 0x08, 0xD9, 0xC0, 0xD8, 0xE3, 0x8B, 0x48,
    0x04, 0x89, 0x54, 0x24, 0x18, 0x89, 0x4C, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x05, 0xD0,
    0xA8, 0x73, 0x01, 0xDE, 0xCA, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0xC0, 0xD8, 0xE3, 0xD9, 0x5C, 0x24,
    0x24, 0xD9, 0xC9, 0xDE, 0xC3, 0xD9, 0xCA, 0xD9, 0x5C, 0x24, 0x28, 0xDE, 0xC1, 0xD9, 0x5C, 0x24,
    0x2C, 0x0F, 0x8E, 0x8F, 0x00, 0x00, 0x00, 0x8B, 0x34, 0xAD, 0xD8, 0xFB, 0xF6, 0x00, 0x0F, 0xB7,
    0x46, 0x08, 0x80, 0xB8, 0x08, 0x3B, 0xF8, 0x00, 0x00, 0x74, 0x6C, 0x33, 0xDB, 0x39, 0x9E, 0x0C,
    0x02, 0x00, 0x00, 0x7E, 0x62, 0xEB, 0x09, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8B, 0xFF,
    0x8B, 0x8E, 0x10, 0x02, 0x00, 0x00, 0x8B, 0x3C, 0x99, 0x8D, 0x04, 0xBF, 0xC1, 0xE0, 0x04, 0x03,
    0x46, 0x4C, 0x8D, 0x4C, 0x24, 0x20, 0xD9, 0x00, 0x8D, 0x54, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x10,
    0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x40, 0x10, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x40,
    0x18, 0xD9, 0x5C, 0x24, 0x1C, 0xE8, 0xC6, 0x0C, 0x00, 0x00, 0x3C, 0x01, 0x75, 0x0E, 0x6A, 0x04,
    0x6A, 0x00, 0x57, 0x56, 0xE8, 0x47, 0xDF, 0xFF, 0xFF, 0x83, 0xC4, 0x10, 0x83, 0xC3, 0x01, 0x3B,
    0x9E, 0x0C, 0x02, 0x00, 0x00, 0x7C, 0xA9, 0x83, 0xC5, 0x01, 0x3B, 0x2D, 0xC4, 0xFB, 0xF6, 0x00,
    0x0F, 0x8C, 0x71, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x24, 0xC3,
];
#[cfg(test)]
#[allow(dead_code)]
const EXTRAS_CALLS: [usize; 2] = [0xe5, 0xf4];

const BEGIN_BODY: [u8; 117] = [
    0x83, 0x3D, 0xC8, 0x6C, 0x52, 0x02, 0x00, 0x75, 0x2C, 0xA1, 0xB8, 0x5B, 0xEC, 0x00, 0x8B, 0x50,
    0x08, 0xB9, 0xB8, 0x5B, 0xEC, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x03, 0x83, 0xC0, 0x08, 0xC7,
    0x80, 0xAC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x40, 0x28, 0x00, 0x00, 0x00, 0x00,
    0xA3, 0xC8, 0x6C, 0x52, 0x02, 0x33, 0xC9, 0xB8, 0x14, 0x00, 0x00, 0x00, 0xBA, 0x24, 0x00, 0x00,
    0x00, 0xF7, 0xE2, 0x0F, 0x90, 0xC1, 0x6A, 0x00, 0xF7, 0xD9, 0x0B, 0xC8, 0xA1, 0x48, 0x41, 0x77,
    0x02, 0x8B, 0x50, 0x1C, 0x51, 0xB9, 0x48, 0x41, 0x77, 0x02, 0xFF, 0xD2, 0xA3, 0xF8, 0x3A, 0xF8,
    0x00, 0x8B, 0x44, 0x24, 0x04, 0xC7, 0x05, 0xF4, 0x3A, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA3,
    0xF0, 0x3A, 0xF8, 0x00, 0xC3,
];
#[cfg(test)]
#[allow(dead_code)]
const BEGIN_CALLS: [usize; 0] = [];
#[cfg(test)]
const BIND_BODY: [u8; 275] = [
    0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x56, 0x8B, 0xF0, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00, 0x57,
    0x8B, 0xF9, 0x8B, 0x08, 0x8B, 0x41, 0x58, 0x8B, 0x88, 0x84, 0x04, 0x00, 0x00, 0x8D, 0x14, 0x7F,
    0x8B, 0x4C, 0x91, 0x08, 0x0F, 0xB6, 0x51, 0x09, 0x83, 0xC2, 0xFF, 0x3B, 0xD6, 0x7D, 0x02, 0x8B,
    0xF2, 0x8B, 0x51, 0x2C, 0x8B, 0x54, 0xEA, 0x04, 0x8B, 0x5C, 0x24, 0x18, 0x0F, 0xBF, 0x14, 0x9A,
    0x89, 0xB8, 0x6C, 0x04, 0x00, 0x00, 0x8B, 0x7C, 0x24, 0x1C, 0x89, 0xB0, 0x70, 0x04, 0x00, 0x00,
    0x89, 0x90, 0x74, 0x04, 0x00, 0x00, 0x89, 0xB8, 0x78, 0x04, 0x00, 0x00, 0x8B, 0x7C, 0x24, 0x20,
    0x89, 0xB8, 0x7C, 0x04, 0x00, 0x00, 0x8B, 0x49, 0x28, 0x8B, 0x0C, 0xB1, 0x8D, 0x14, 0xD1, 0x8B,
    0x0A, 0x33, 0xDB, 0x3B, 0xCB, 0x75, 0x06, 0x8B, 0x0D, 0x98, 0x4B, 0xE8, 0x00, 0x39, 0x0D, 0xEC,
    0x2A, 0x2F, 0x02, 0xBF, 0x08, 0x00, 0x00, 0x00, 0x74, 0x1B, 0x8B, 0x35, 0xF8, 0x62, 0x3D, 0x03,
    0xC7, 0x06, 0x04, 0x00, 0x00, 0x00, 0x89, 0x4E, 0x04, 0x01, 0x3D, 0xF8, 0x62, 0x3D, 0x03, 0x89,
    0x0D, 0xEC, 0x2A, 0x2F, 0x02, 0x83, 0xFD, 0x07, 0x75, 0x2B, 0x39, 0x1D, 0x90, 0x5A, 0x2F, 0x02,
    0x74, 0x23, 0x8B, 0x0D, 0xF8, 0x62, 0x3D, 0x03, 0x89, 0x1D, 0x90, 0x5A, 0x2F, 0x02, 0xC7, 0x01,
    0x0F, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x04, 0xA8, 0x00, 0x00, 0x00, 0x89, 0x59, 0x08, 0x83, 0x05,
    0xF8, 0x62, 0x3D, 0x03, 0x0C, 0x38, 0x98, 0x80, 0x04, 0x00, 0x00, 0x74, 0x07, 0x8B, 0x52, 0x04,
    0x3B, 0xD3, 0x75, 0x06, 0x8B, 0x15, 0x9C, 0x4B, 0xE8, 0x00, 0x39, 0x15, 0xE8, 0x2A, 0x2F, 0x02,
    0x74, 0x1A, 0xA1, 0xF8, 0x62, 0x3D, 0x03, 0xC7, 0x00, 0x03, 0x00, 0x00, 0x00, 0x89, 0x50, 0x04,
    0x01, 0x3D, 0xF8, 0x62, 0x3D, 0x03, 0x89, 0x15, 0xE8, 0x2A, 0x2F, 0x02, 0x5F, 0x5E, 0x5D, 0xB0,
    0x01, 0x5B, 0xC3,
];
#[cfg(test)]
#[allow(dead_code)]
const BIND_CALLS: [usize; 0] = [];

// --- shadow bookkeeping -----------------------------------------------------------------------------

struct Shadow {
    name: &'static str,
    orig: AtomicUsize,
    calls: AtomicU32,
    bad: AtomicU32,
}

impl Shadow {
    const fn new(name: &'static str) -> Shadow {
        Shadow { name, orig: AtomicUsize::new(0), calls: AtomicU32::new(0), bad: AtomicU32::new(0) }
    }
    /// True while the shadow window is open and a relocated original exists.
    #[inline(always)]
    fn active(&self) -> bool {
        self.calls.load(Ordering::Relaxed) < SHADOW_CALLS && self.orig.load(Ordering::Relaxed) != 0
    }
}

static S_FRUSTUM: Shadow = Shadow::new("frustum_test_aabb");
static S_PACK: Shadow = Shadow::new("normal_pack_ubyte4");
static S_VBV: Shadow = Shadow::new("view_bounds_visible");

// --- 0x530270 frustum_test_aabb -----------------------------------------------------------------

/// The box corner vector the original builds on its stack: (x, y, z, 1.0).
#[inline(always)]
unsafe fn corner(p: usize) -> __m128 {
    unsafe { _mm_set_ps(1.0, rf(p + 8), rf(p + 4), rf(p)) }
}

/// One plane set: count at +0, planes (16 bytes each) from +0x10, corner select masks from
/// `moff`. True when some plane has `!(dot <= 0)` (positive or NaN), the original's `cmpnless`.
/// `a` is the vector taken where the mask is set (the ECX box), `b` where it is clear.
#[target_feature(enable = "sse3")]
unsafe fn any_out(set: usize, moff: usize, a: __m128, b: __m128) -> bool {
    unsafe {
        let n = rd(set);
        let mut plane = set + 0x10;
        let mut mask = set + moff;
        let mut i = 0;
        while i < n {
            let m = _mm_loadu_ps(mask as *const f32);
            let sel = _mm_or_ps(_mm_andnot_ps(m, b), _mm_and_ps(m, a));
            let prod = _mm_mul_ps(sel, _mm_loadu_ps(plane as *const f32));
            let h = _mm_hadd_ps(prod, prod);
            let h = _mm_hadd_ps(h, h);
            if !(_mm_cvtss_f32(h) <= 0.0) {
                return true;
            }
            plane += 0x10;
            mask += 0x10;
            i += 1;
        }
        false
    }
}

#[target_feature(enable = "sse3")]
unsafe fn frustum_sse(min: usize, max: usize, planes: usize, cells: usize, count: i32) -> u32 {
    unsafe {
        let a = corner(min);
        let b = corner(max);
        if any_out(planes, 0xF0, a, b) {
            return 0;
        }
        let mut cell = cells + 0x10;
        let mut i = 0;
        while i < count {
            if !any_out(cell, 0x180, a, b) {
                return 0;
            }
            cell += 0x220;
            i += 1;
        }
        1
    }
}

/// The legacy path (flag 0xE8E88A clear): the original's helper functions, EAX as it leaves it.
unsafe fn frustum_soft(min: u32, max: u32, planes: u32, cells: u32, count: i32) -> u32 {
    unsafe {
        let r = call_regs(PLANES_SOFT, 0, min, max, 0, 0, &[planes]);
        if r == 0 {
            return 0;
        }
        let mut last = r;
        let mut cell = cells.wrapping_add(0x10);
        let mut i = 0;
        while i < count {
            last = call_regs(CELL_SOFT, 0, max, cell, 0, 0, &[min]);
            if last != 0 {
                return last & !0xFF;
            }
            cell = cell.wrapping_add(0x220);
            i += 1;
        }
        (last & !0xFF) | 1
    }
}

pub(crate) unsafe fn frustum_impl(min: u32, max: u32, planes: u32, cells: u32, count: i32) -> u32 {
    unsafe {
        if rb(FRUSTUM_SSE_FLAG) != 0 {
            frustum_sse(min as usize, max as usize, planes as usize, cells as usize, count)
        } else {
            frustum_soft(min, max, planes, cells, count)
        }
    }
}

extern "C" fn frustum_rs(min: u32, max: u32, planes: u32, cells: u32, count: u32) -> u32 {
    unsafe {
        if !S_FRUSTUM.active() {
            return frustum_impl(min, max, planes, cells, count as i32);
        }
        S_FRUSTUM.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_FRUSTUM.orig.load(Ordering::Relaxed), 0, min, max, 0, 0, &[planes, cells, count]);
        let mine = frustum_impl(min, max, planes, cells, count as i32);
        if mine != orig {
            S_FRUSTUM.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

/// ECX = &min, EDX = &max, stack (planes, cells, count) cdecl. ECX / EDX are kept.
#[unsafe(naked)]
extern "C" fn frustum_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "push edx",
        "push ecx",
        "call {f}",
        "add esp, 20",
        "pop edx",
        "pop ecx",
        "ret",
        f = sym frustum_rs,
    )
}

// --- 0x9EE620 normal_pack_ubyte4 ----------------------------------------------------------------

/// The low dword of `fistp qword` with truncation (the original forces round-toward-zero): the
/// "integer indefinite" 0x8000000000000000 has a low dword of 0.
#[inline(always)]
fn trunc_lo(v: f64) -> u32 {
    if v > -2147483649.0 && v < 2147483648.0 {
        (v as i32) as u32
    } else if v > -9.223372036854775808e18 && v < 9.223372036854775808e18 {
        (v as i64) as u32
    } else {
        0
    }
}

/// One channel: ((v + 1) * 0.5) stored as f32, times 255 stored as f32, plus 0.5, truncated. The
/// x87 runs at 53-bit precision, so the f64 steps match.
#[inline(always)]
fn channel(v: f32) -> u32 {
    let t = ((v as f64 + 1.0) * 0.5) as f32;
    let u = (t as f64 * 255.0) as f32;
    trunc_lo(u as f64 + 0.5)
}

/// Returns EDX:EAX = (raw x conversion : packed dword) and stores the packed dword at `out`.
extern "C" fn pack_rs(out: u32, x: u32, y: u32, z: u32, w: u32) -> u64 {
    unsafe {
        if !S_PACK.active() {
            return pack_impl(out, x, y, z, w);
        }
        S_PACK.calls.fetch_add(1, Ordering::Relaxed);
        let mut o_orig = 0xA5A5_A5A5u32;
        let mut o_mine = 0x5A5A_5A5Au32;
        let r = call_regs(S_PACK.orig.load(Ordering::Relaxed), 0, 0, 0, 0, 0, &[&mut o_orig as *mut u32 as u32, x, y, z, w]);
        let m = pack_impl(&mut o_mine as *mut u32 as u32, x, y, z, w);
        if r != &mut o_orig as *mut u32 as u32 || o_orig != o_mine || (m as u32) != o_orig {
            S_PACK.bad.fetch_add(1, Ordering::Relaxed);
        }
        *(out as usize as *mut u32) = o_orig;
        // EDX is not observable through call_regs; the raw x byte is `m >> 32` (Rust-computed).
        (m & 0xFFFF_FFFF_0000_0000) | o_orig as u64
    }
}

#[inline(always)]
unsafe fn pack_impl(out: u32, x: u32, y: u32, z: u32, w: u32) -> u64 {
    let cw = channel(f32::from_bits(w));
    let cz = channel(f32::from_bits(z));
    let cy = channel(f32::from_bits(y));
    let cx = channel(f32::from_bits(x));
    let packed = ((((cw << 8) | cz) << 8 | cy) << 8) | cx;
    unsafe { *(out as usize as *mut u32) = packed };
    ((cx as u64) << 32) | packed as u64
}

/// stdcall (out, x, y, z, w): EAX = out, ECX = packed, EDX = last raw conversion, `ret 0x14`.
#[unsafe(naked)]
extern "C" fn pack_thunk() {
    std::arch::naked_asm!(
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "push dword ptr [esp + 20]",
        "call {f}",
        "add esp, 20",
        "mov ecx, eax",
        "mov eax, [esp + 4]",
        "ret 20",
        f = sym pack_rs,
    )
}

// --- 0x530450 view_bounds_visible ---------------------------------------------------------------

/// Squared distance from the view point to the box, as `0xBF63B0` computes it: lanes 0-2 only
/// (mask at 0xE83E50). `view` is the 16-byte view record head (x, y, z, radius).
#[inline(always)]
unsafe fn box_dist2(view: usize, min: usize, max: usize) -> f32 {
    unsafe {
        let pos = _mm_loadu_ps(view as *const f32);
        let lo = _mm_loadu_ps(min as *const f32);
        let hi = _mm_loadu_ps(max as *const f32);
        let zero = _mm_setzero_ps();
        let below = _mm_max_ps(_mm_sub_ps(pos, lo), zero);
        let above = _mm_max_ps(_mm_sub_ps(hi, pos), zero);
        let d = _mm_add_ps(below, above);
        let sq = _mm_and_ps(_mm_mul_ps(d, d), _mm_load_ps(DIST_MASK as *const f32));
        let hl = _mm_movehl_ps(sq, sq);
        let s = _mm_add_ps(hl, sq);
        let t = _mm_shuffle_ps::<0xE5>(s, s);
        _mm_cvtss_f32(_mm_add_ps(t, s))
    }
}

/// `fld` / `fstp dword` turns a signalling NaN quiet.
#[inline(always)]
fn through_x87(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

pub(crate) unsafe fn vbv_impl(view_idx: u32, min: u32, bounds: u32) -> u32 {
    unsafe {
        let e = VIEW_TABLE + view_idx as usize * 0x30;
        if rb(e + 0x20) != 0 {
            let f = rf(e + 0xC) as f64;
            let r = box_dist2(e, min as usize, bounds as usize) as f64;
            // fcompp; test ah, 0x41; jnp: pass only when r <= f * f (ordered)
            if !(r <= f * f) {
                return 0;
            }
        }
        if rb(e + 0x21) != 0 {
            let radius = through_x87(rd(e + 0x1C));
            let r = call_regs(VIEW_FRUSTUM_TEST, (e + 0x10) as u32, 0, e as u32, 0, 0, &[bounds, min, radius]);
            if r & 0xFF == 0 {
                return 0;
            }
        }
        let o = OCCLUDER_TABLE + view_idx as usize * 0x280;
        if rb(o + 0x64) == 0 && frustum_impl(min, bounds, (o + 0x70) as u32, 0, 0) & 0xFF == 0 {
            return 0;
        }
        1
    }
}

extern "C" fn vbv_rs(esi: u32, edi: u32, bounds: u32) -> u32 {
    unsafe {
        if !S_VBV.active() {
            return vbv_impl(esi, edi, bounds);
        }
        S_VBV.calls.fetch_add(1, Ordering::Relaxed);
        let orig = call_regs(S_VBV.orig.load(Ordering::Relaxed), 0, 0, 0, esi, edi, &[bounds]);
        let mine = vbv_impl(esi, edi, bounds);
        if mine & 0xFF != orig & 0xFF {
            S_VBV.bad.fetch_add(1, Ordering::Relaxed);
        }
        orig
    }
}

/// ESI = view index, EDI = &min, stack (&max) cdecl; AL = visible. ECX / EDX are kept.
#[unsafe(naked)]
extern "C" fn vbv_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push dword ptr [esp + 12]",
        "push edi",
        "push esi",
        "call {f}",
        "add esp, 12",
        "pop edx",
        "pop ecx",
        "ret",
        f = sym vbv_rs,
    )
}


// --- 0x4B5A20 city_instance_add -----------------------------------------------------------------

const CITY_INSTANCE_ADD: usize = 0x004B_5A20;
const RECORD_INIT: usize = 0x004B_52F0;
const WORLD_MESH_SUBMIT: usize = 0x0052_F910;

/// Branch counters for the first `SHADOW_CALLS` calls (the callees have side effects, so there is
/// no result shadow): 0 entered, 1 bad pass, 2 / 3 / 4 / 5 / 6 / 7 rejected, 8 submitted.
static ADD_BRANCH: [AtomicU32; 9] = [const { AtomicU32::new(0) }; 9];
static ADD_SEEN: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
fn add_bump(i: usize) {
    if ADD_SEEN.load(Ordering::Relaxed) < SHADOW_CALLS {
        ADD_BRANCH[i].fetch_add(1, Ordering::Relaxed);
    }
}

/// A scratch record the size of the original's stack frame; its unwritten bytes stay undefined,
/// as they are on the original's stack.
#[repr(C, align(16))]
struct Frame([std::mem::MaybeUninit<u8>; 0x2E0]);

/// A game function taking EAX only (no stack arguments).
#[inline(always)]
unsafe fn call_eax0(f: usize, eax: u32) {
    unsafe {
        std::arch::asm!("call edx", in("edx") f, inout("eax") eax => _, clobber_abi("C"));
    }
}

/// A game function taking ECX only (no stack arguments).
#[inline(always)]
unsafe fn call_ecx0(f: usize, ecx: u32) {
    unsafe {
        std::arch::asm!("call eax", in("eax") f, inout("ecx") ecx => _, clobber_abi("C"));
    }
}

/// A game function taking ECX only and returning EAX.
#[inline(always)]
unsafe fn call_ecx_ret(f: usize, ecx: u32) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!("call eax", inout("eax") f => r, in("ecx") ecx, clobber_abi("C"));
    }
    r
}

/// EAX and ECX plus three cdecl stack arguments (`args[0]` on top); returns EAX.
#[inline(always)]
unsafe fn call_eax_ecx3(f: usize, eax: u32, ecx: u32, args: &[u32; 3]) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push dword ptr [edx + 8]",
            "push dword ptr [edx + 4]",
            "push dword ptr [edx]",
            "call edi",
            "add esp, 12",
            in("edx") args.as_ptr(),
            in("edi") f,
            inout("eax") eax => r,
            in("ecx") ecx,
            clobber_abi("C"),
        );
    }
    r
}

/// `eax` plus seven cdecl stack arguments (`args[0]` on top).
#[inline(always)]
unsafe fn call_eax7(f: usize, eax: u32, args: &[u32; 7]) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push dword ptr [ecx + 24]",
            "push dword ptr [ecx + 20]",
            "push dword ptr [ecx + 16]",
            "push dword ptr [ecx + 12]",
            "push dword ptr [ecx + 8]",
            "push dword ptr [ecx + 4]",
            "push dword ptr [ecx]",
            "call edx",
            "add esp, 28",
            in("ecx") args.as_ptr(),
            in("edx") f,
            inout("eax") eax => r,
            clobber_abi("C"),
        );
    }
    r
}

/// cdecl (chunk, index, u8 lod, pass 1 or 2): the per-view instance submit with the flag filters.
/// Returns what `world_mesh_submit` returns, or AL = 0 when filtered.
pub(crate) unsafe extern "C" fn city_instance_add_rs(chunk: u32, index: u32, lod: u32, pass: u32) -> u32 {
    unsafe {
        add_bump(0);
        if pass != 1 && pass != 2 {
            add_bump(1);
            return 0;
        }
        let rec = rd(chunk as usize + 0x4C).wrapping_add(index.wrapping_mul(0x50)) as usize;
        let a = rd(rec + 0x30);
        let b = rd(rec + 0x34);
        let m48 = rd(rec + 0x48);
        let mesh = if m48 != 0 && a & 0x10000 != 0 { m48 } else { rd(rec + 0x40) };
        if b & 0x400 != 0 && rb(0x0252_7D17) != 0 {
            add_bump(2);
            return 0;
        }
        let rejected = if pass == 1 { (a & 0xC038_8020) | (b & 0x22000) } else { (a & 0xC138_8020) | (b & 0x2000) };
        if rejected != 0 {
            add_bump(3);
            return 0;
        }
        if b & 0x80 != 0 && rb(0x0252_7E04) == 0 {
            add_bump(4);
            return 0;
        }
        let bl = a & 0x0200_0000 != 0;
        let ebp = rd(0x0261_2D00) as usize;
        if ebp != 0 {
            let flags = rd(ebp + 0x20);
            if !(flags & 8 != 0 && a & 0x0800_0000 != 0) {
                if flags & 4 != 0 || a & 0x1000_0000 == 0 {
                    add_bump(5);
                    return 0;
                }
            }
            if flags & 0x100 != 0 && a & 0x0080_0000 != 0 {
                add_bump(6);
                return 0;
            }
        } else if a & 0x1000_0000 == 0 {
            add_bump(5);
            return 0;
        }

        let mut frame = Frame([std::mem::MaybeUninit::uninit(); 0x2E0]);
        let r = frame.0.as_mut_ptr() as usize;
        wd(r + 0x30, 0);
        call_eax0(RECORD_INIT, r as u32);
        wd(r + 0x2BC, rd(rec + 0x20));
        let t = (a >> 1) | (b << 31);
        let eax = (t & 0xFFFF_0000) | (((t & 0xFFFF) as u16) << 7) as u32;
        let r0 = rd(r);
        let mut v = (((eax ^ r0) & 0x180) ^ (r0 & 0xFFFF)) | 0x201;
        wd(r + 0x10, rd(0x0261_572C));
        wd(r + 0x14, rd(0x0261_5730));
        wd(r + 0x18, rd(0x0261_5734));
        wb(r + 0x2A9, lod as u8);
        if a & 0x800 != 0 {
            v |= 0x20;
        }
        if bl && b & 8 != 0 {
            v |= 0x400;
            wd(r + 0x2AC, 0x022D_7BE0);
        }
        wd(r, v);
        add_bump(8);
        let res = call_eax7(WORLD_MESH_SUBMIT, r as u32, &[pass, mesh, chunk, index, rec as u32, (rec + 0x10) as u32, 0]);
        if ADD_SEEN.load(Ordering::Relaxed) < SHADOW_CALLS {
            ADD_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        res
    }
}


// --- 0x4B5510 city_instance_submit --------------------------------------------------------------

const CITY_INSTANCE_SUBMIT: usize = 0x004B_5510;
const INSTANCE_VISIBLE_FADE: usize = 0x0096_0D60;
const INSTANCE_STREAM_OK: usize = 0x004B_9310;
const RECORD_INIT_ECX: usize = 0x004B_52D0;
const FLOAT_BAND: usize = 0x00BF_0E00;
const PLACE_EFFECT: usize = 0x0053_3880;
const SCOPE_BEGIN: usize = 0x004B_83A0;
const SCOPE_SLOT: usize = 0x0053_0520;
const SCOPE_END: usize = 0x004B_8370;
const WORLD_MESH_SUBMIT_ALT: usize = 0x0052_FAF0;

/// Branch counters for the first `SHADOW_CALLS` entered calls (the callees write, so no result
/// shadow): 0 entered, 1 pass 2, 2 filtered before the stamp, 3 filtered after it, 4 radius 0,
/// 5 submitted (main), 6 submitted (alt), 7 effect placed, 8 slot lookup.
static SUB_BRANCH: [AtomicU32; 9] = [const { AtomicU32::new(0) }; 9];

#[inline(always)]
fn sub_bump(i: usize) {
    if SUB_BRANCH[0].load(Ordering::Relaxed) < SHADOW_CALLS {
        SUB_BRANCH[i].fetch_add(1, Ordering::Relaxed);
    }
}

/// 0x4B45F0: the current view object (0x2612D00) has bit 3 set.
#[inline(always)]
unsafe fn view_bit(bit: u32) -> bool {
    unsafe {
        let obj = rd(0x0261_2D00) as usize;
        obj != 0 && (rd(obj + 0x20) >> bit) & 1 != 0
    }
}

/// 0x4B4610: the view object is absent or has bit 2 clear.
#[inline(always)]
unsafe fn view_bit2_clear() -> bool {
    unsafe {
        let obj = rd(0x0261_2D00) as usize;
        obj == 0 || (rd(obj + 0x20) >> 2) & 1 == 0
    }
}

/// cdecl (chunk, index, u8 alt, pass): the main instance submit. Returns what the world-mesh
/// submit returns, or AL = 0 when the instance is filtered out.
pub(crate) unsafe extern "C" fn city_instance_submit_rs(chunk: u32, index: u32, alt: u32, pass: u32) -> u32 {
    unsafe {
        sub_bump(0);
        let alt = alt as u8;
        if pass == 2 {
            sub_bump(1);
            return 0;
        }
        let rec = rd(chunk as usize + 0x4C).wrapping_add(index.wrapping_mul(0x50)) as usize;
        let a = rd(rec + 0x30);
        let b = rd(rec + 0x34);
        let m48 = rd(rec + 0x48);
        let mesh = (if m48 != 0 && a & 0x10000 != 0 { m48 } else { rd(rec + 0x40) }) as usize;
        if alt != 0 {
            let obj = rd(rec + 0xC) as usize;
            if rd(obj + 0xCC) & 0x10 != 0 && rd(obj + 0xD0) == 3 {
                let x = call_regs(INSTANCE_VISIBLE_FADE, rd(obj + 0x1A0), 0, 0, 0, 0, &[]) as usize;
                if x != 0 {
                    let f = rd(x + 0x80);
                    if f & 0x10 != 0 || f & 4 != 0 {
                        sub_bump(2);
                        return 0;
                    }
                }
            }
        }
        if b & 0x400 != 0 && rb(0x0252_7D14) != 0 && rb(rd(0x0252_7D10) as usize + 0xAFF) == 0 {
            sub_bump(2);
            return 0;
        }
        if (a & 0xC138_8020) | (b & 0x22000) != 0 {
            sub_bump(2);
            return 0;
        }
        if (pass == 4) != (b & 0x10 != 0) {
            sub_bump(2);
            return 0;
        }
        if b & 0x80 != 0 && rb(0x0252_7E04) == 0 {
            sub_bump(2);
            return 0;
        }
        let big = a & 0x0200_0000 != 0;
        if rd(mesh + 0x5C) & 0x40_0000 != 0 {
            sub_bump(2);
            return 0;
        }
        let entry = (rd(chunk as usize + 0x58) as usize).wrapping_add(index as usize * 8);
        let old_stamp = *((rec + 0x38) as *const u16) as u32;
        *((rec + 0x38) as *mut u16) = *(0x0252_6CD0 as *const u16);

        // 4B45F0 / 4B4610 / 4B4470 inlined (pure reads of the view object)
        if !(view_bit(3) && a & 0x0800_0000 != 0) && (!view_bit2_clear() || a & 0x1000_0000 == 0) {
            sub_bump(3);
            return 0;
        }
        if view_bit(8) && a & 0x0080_0000 != 0 {
            sub_bump(3);
            return 0;
        }
        // 4B83D0: chunk kind 7 or 8
        let kind = rd(chunk as usize + 0x20) as i32;
        if (7..=8).contains(&kind) && call_regs(INSTANCE_STREAM_OK, 0, 0, 0, 0, 0, &[chunk, rec as u32, old_stamp]) & 0xFF == 0 {
            sub_bump(3);
            return 0;
        }
        // fldz; fcomp [mesh + 0x4C]; ... jnp: equal (ordered) means no radius
        if rf(mesh + 0x4C) == 0.0 {
            sub_bump(4);
            return 0;
        }

        let mut frame = Frame([std::mem::MaybeUninit::uninit(); 0x2E0]);
        let r = frame.0.as_mut_ptr() as usize;
        call_ecx0(RECORD_INIT_ECX, r as u32);
        wd(r + 0x2BC, rd(rec + 0x20));
        let t = (a >> 1) | (b << 31);
        let bx = (((t & 0xFFFF) as u16) << 7) as u32;
        let r0 = rd(r);
        let mut v = ((bx ^ r0) & 0x180) ^ (r0 & 0xFFFF);
        if a & 0x800 != 0 {
            v |= 0x20;
        }
        if rb(0x0252_6CBD) != 0 || pass == 4 {
            v |= 0x200;
        }
        let cb = rb(chunk as usize + 0x1C);
        if cb & 1 != 0 {
            v |= 0x8_0000;
        }
        if rb(0x0252_7D16) == 1 {
            wb(r + 0xC, 0);
        }

        if pass == 0 && (a & 0x0200_0000) | (b & 8) != 0 && rb(entry + 6) != 0 {
            sub_bump(7);
            // half extents and the box centre, each rounded to f32 as the x87 stores do
            let d0 = (rf(rec + 0x10) as f64 - rf(rec) as f64) as f32;
            let d1 = (rf(rec + 0x14) as f64 - rf(rec + 4) as f64) as f32;
            let d2 = (rf(rec + 0x18) as f64 - rf(rec + 8) as f64) as f32;
            let h0 = (d0 as f64 * 0.5) as f32;
            let h1 = (d1 as f64 * 0.5) as f32;
            let h2 = (0.5 * d2 as f64) as f32;
            let near = (cb & 0x10 != 0 || cb & 1 != 0)
                && call_regs(
                    FLOAT_BAND,
                    0,
                    0,
                    0,
                    0,
                    0,
                    &[through_x87(rd(rec + 4)), through_x87(rd(0x00E5_CE30)), through_x87(rd(0x00DD_1C88))],
                ) & 0xFF
                    != 0;
            let p = [
                (rf(rec) as f64 + h0 as f64) as f32,
                (rf(rec + 4) as f64 + h1 as f64) as f32,
                (h2 as f64 + rf(rec + 8) as f64) as f32,
            ];
            let res = call_regs(PLACE_EFFECT, 0, 0, 0, 0, p.as_ptr() as u32, &[if near { 1 } else { 2 }]);
            v |= 0x400;
            wd(r + 8, res);
        }

        if alt == 0 && rb(entry + 6) != 0 && b & 0x100 == 0 {
            sub_bump(8);
            let mut loc = [0u32; 8];
            loc[0] = rd(rd(rd(mesh + 0x58) as usize) as usize + 0x20);
            let obj = rd(r + 0x34);
            loc[1] = obj;
            if obj != 0 {
                call_regs(SCOPE_BEGIN, 0, 0, 0, loc.as_mut_ptr() as u32, 0, &[obj, 0]);
            }
            let slot = call_ecx_ret(SCOPE_SLOT, loc.as_mut_ptr() as u32);
            *((entry + 4) as *mut u16) = slot as u16;
            call_eax0(SCOPE_END, loc.as_mut_ptr() as u32);
        }

        let (c0, c1, c2) = if pass == 4 {
            (0x3F80_0000, 0x3F80_0000, 0x3F80_0000)
        } else if big {
            (rd(0x0261_2D04), rd(0x0261_2D08), rd(0x0261_2D0C))
        } else {
            (rd(0x0261_5720), rd(0x0261_5724), rd(0x0261_5728))
        };
        wd(r + 0x10, c0);
        wd(r + 0x14, c1);
        wd(r + 0x18, c2);
        if b & 0x200 != 0 {
            v |= 1;
        }
        let m = rf(mesh + 0x4C);
        if big && b & 8 != 0 {
            wd(r + 0x2AC, 0x022D_7BE0);
            v |= 0x40;
            wd(r + 4, through_x87(rd(mesh + 0x4C)));
        } else if 1.0 > m as f64 || a & 0x400 != 0 {
            v |= 0x40;
            wd(r + 4, through_x87(rd(mesh + 0x4C)));
        }
        wd(r, v);

        if alt != 0 {
            sub_bump(6);
            call_eax_ecx3(WORLD_MESH_SUBMIT_ALT, r as u32, (rec + 0x10) as u32, &[pass, mesh as u32, rec as u32])
        } else {
            sub_bump(5);
            call_eax7(WORLD_MESH_SUBMIT, r as u32, &[pass, mesh as u32, chunk, index, rec as u32, (rec + 0x10) as u32, 0])
        }
    }
}


// --- 0x4B74D0 city_submit_chunk_extras ----------------------------------------------------------

const CITY_SUBMIT_CHUNK_EXTRAS: usize = 0x004B_74D0;

static EXTRAS_SEEN: AtomicU32 = AtomicU32::new(0);
static EXTRAS_TESTS: AtomicU32 = AtomicU32::new(0);
static EXTRAS_SUBMITS: AtomicU32 = AtomicU32::new(0);

/// A game function with four cdecl stack arguments (`args[0]` on top).
#[inline(always)]
unsafe fn call_stack4(f: usize, args: &[u32; 4]) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push dword ptr [ecx + 12]",
            "push dword ptr [ecx + 8]",
            "push dword ptr [ecx + 4]",
            "push dword ptr [ecx]",
            "call edx",
            "add esp, 16",
            in("ecx") args.as_ptr(),
            in("edx") f,
            out("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

/// No arguments, no result: the extra-instance pass. Each frame it takes one of three fixed
/// boxes (rotating with the frame counter at 0x2975940), and for every streamed chunk whose
/// kind flag is set it submits the extra instances whose x / z bounds overlap that box (the
/// overlap test 0x4B8280 is inlined: it is pure, a four-way `>` compare on x87 doubles).
pub(crate) unsafe extern "C" fn city_submit_chunk_extras_rs() {
    unsafe {
        let n = rd(0x00F6_FBC4) as i32;
        let idx = rd(0x0297_5940).wrapping_add(1) % 3;
        let e = 0x0297_5880 + idx as usize * 12;
        let h = rf(0x0173_A8CC) as f64 * 0.5;
        let p = 0.5 * rf(0x0173_A8D0) as f64;
        let (a0, a2) = (rf(e) as f64, rf(e + 8) as f64);
        let lo0 = (a0 - h) as f32 as f64;
        let lo2 = (a2 - p) as f32 as f64;
        let hi0 = (h + a0) as f32 as f64;
        let hi2 = (a2 + p) as f32 as f64;
        if n <= 0 {
            return;
        }
        if EXTRAS_SEEN.load(Ordering::Relaxed) < SHADOW_CALLS {
            EXTRAS_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        let count_mode = EXTRAS_SEEN.load(Ordering::Relaxed) < SHADOW_CALLS;
        let mut i = 0usize;
        while (i as i32) < (rd(0x00F6_FBC4) as i32) {
            let chunk = rd(0x00F6_FBD8 + i * 4) as usize;
            let kind = *((chunk + 8) as *const u16) as usize;
            if rb(0x00F8_3B08 + kind) != 0 {
                let mut k = 0usize;
                while (rd(chunk + 0x20C) as i32) > 0 && (k as i32) < (rd(chunk + 0x20C) as i32) {
                    let inst = rd(rd(chunk + 0x210) as usize + k * 4);
                    let rec = (rd(chunk + 0x4C) as usize).wrapping_add(inst as usize * 0x50);
                    let q0 = rf(rec) as f64;
                    let q1 = rf(rec + 8) as f64;
                    let q2 = rf(rec + 0x10) as f64;
                    let q3 = rf(rec + 0x18) as f64;
                    if count_mode {
                        EXTRAS_TESTS.fetch_add(1, Ordering::Relaxed);
                    }
                    // overlap = none of the four separations holds (NaN compares false)
                    if !(lo0 > q2 || lo2 > q3 || q0 > hi0 || q1 > hi2) {
                        if count_mode {
                            EXTRAS_SUBMITS.fetch_add(1, Ordering::Relaxed);
                        }
                        call_stack4(CITY_INSTANCE_SUBMIT, &[chunk as u32, inst, 0, 4]);
                    }
                    k += 1;
                }
            }
            i += 1;
        }
    }
}


// --- 0x4B68D0 city_instance_batch_begin ---------------------------------------------------------

const CITY_INSTANCE_BATCH_BEGIN: usize = 0x004B_68D0;
static BEGIN_SEEN: AtomicU32 = AtomicU32::new(0);

/// cdecl (cb): makes sure the shared scope object (0x2526CC8) exists, then starts accumulator B:
/// a fresh buffer of 20 entries x 0x24 bytes from the pool at 0x2774148, count 0, callback `cb`.
/// The two virtual calls stay the originals. Once per frame, so not worth more than parity.
pub(crate) unsafe extern "C" fn city_instance_batch_begin_rs(cb: u32) {
    unsafe {
        if BEGIN_SEEN.load(Ordering::Relaxed) < SHADOW_CALLS {
            BEGIN_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        if rd(0x0252_6CC8) == 0 {
            let mut e = call_regs(vslot(0x00EC_5BB8, 2), 0, 0x00EC_5BB8, 0, 0, 0, &[]);
            if e != 0 {
                e = e.wrapping_add(8);
            }
            wd(e as usize + 0xAC, 0);
            wd(e as usize + 0x28, 0);
            wd(0x0252_6CC8, e);
        }
        // 20 * 0x24 with the original's overflow-to-all-ones sizing (never overflows here)
        let size = 20u32 * 0x24;
        let buf = call_regs(vslot(0x0277_4148, 7), 0, 0x0277_4148, 0, 0, 0, &[size, 0]);
        wd(0x00F8_3AF8, buf);
        wd(0x00F8_3AF4, 0);
        wd(0x00F8_3AF0, cb);
    }
}

// --- 0xD1A860 gfx_bind_shaders ------------------------------------------------------------------

// Ported and tested against the original, but NOT installed: the original is a ~40-instruction
// leaf (7-9 ns) and every Rust version measured 1.2x slower (the entry shim and prologue).
#[allow(dead_code)]
const GFX_BIND_SHADERS: usize = 0x00D1_A860;

/// ECX = shader id, EAX = LOD, cdecl (pass, column, decl a, decl b): picks the technique, stores
/// the binding in the thread's context (+0x46C..+0x47C) and appends vertex-shader (command 4) and
/// pixel-shader (command 3) changes to the command ring (0x33D62F8) when the cached values differ.
/// AL = 1.
#[allow(dead_code)]
unsafe extern "C" fn bind_shaders_rs(id: u32, lod: u32, args: *const u32) -> u32 {
    unsafe {
        let (pass, column, decl_a, decl_b) = (*args, *args.add(1), *args.add(2), *args.add(3));
        let tls: usize;
        std::arch::asm!("mov {0}, fs:[0x2C]", out(reg) tls, options(nostack, preserves_flags));
        let ctx = rd(rd(tls) as usize + 0x58) as usize;
        let shader = rd((rd(ctx + 0x484) as usize).wrapping_add((id as usize).wrapping_mul(12)).wrapping_add(8)) as usize;
        let max_lod = rb(shader + 9) as i32 - 1;
        let lod = if max_lod >= lod as i32 { lod as i32 } else { max_lod };
        let cols = rd((rd(shader + 0x2C) as usize).wrapping_add((pass as usize).wrapping_mul(8)).wrapping_add(4)) as usize;
        let tech = *(cols.wrapping_add((column as usize).wrapping_mul(4)) as *const i16) as i32;
        wd(ctx + 0x46C, id);
        wd(ctx + 0x470, lod as u32);
        wd(ctx + 0x474, tech as u32);
        wd(ctx + 0x478, decl_a);
        wd(ctx + 0x47C, decl_b);
        let lods = rd(shader + 0x28) as usize;
        let pairs = rd(lods.wrapping_add((lod as usize).wrapping_mul(4))) as usize;
        let pair = pairs.wrapping_add((tech as usize).wrapping_mul(8));
        let mut vs = rd(pair);
        if vs == 0 {
            vs = rd(0x00E8_4B98);
        }
        if rd(0x022F_2AEC) != vs {
            let ring = rd(0x033D_62F8) as usize;
            wd(ring, 4);
            wd(ring + 4, vs);
            wd(0x033D_62F8, (ring + 8) as u32);
            wd(0x022F_2AEC, vs);
        }
        if pass == 7 && rd(0x022F_5A90) != 0 {
            let ring = rd(0x033D_62F8) as usize;
            wd(0x022F_5A90, 0);
            wd(ring, 0xF);
            wd(ring + 4, 0xA8);
            wd(ring + 8, 0);
            wd(0x033D_62F8, (ring + 0xC) as u32);
        }
        let ps = if rb(ctx + 0x480) != 0 { rd(pair + 4) } else { 0 };
        let ps = if ps == 0 { rd(0x00E8_4B9C) } else { ps };
        if rd(0x022F_2AE8) != ps {
            let ring = rd(0x033D_62F8) as usize;
            wd(ring, 3);
            wd(ring + 4, ps);
            wd(0x033D_62F8, (ring + 8) as u32);
            wd(0x022F_2AE8, ps);
        }
        1
    }
}

/// Entry shim: EAX = LOD, ECX = shader id and four cdecl stack arguments become a plain call.
#[allow(dead_code)]
#[unsafe(naked)]
extern "C" fn bind_shaders_thunk() {
    std::arch::naked_asm!(
        "lea edx, [esp + 4]",
        "push edx",
        "push eax",
        "push ecx",
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym bind_shaders_rs,
    )
}

// --- install / summary --------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(&Shadow, usize, &[u8], &[usize], usize); 3] = [
        (&S_FRUSTUM, FRUSTUM_TEST_AABB, &FRUSTUM_BODY, &FRUSTUM_CALLS, frustum_thunk as *const () as usize),
        (&S_PACK, NORMAL_PACK_UBYTE4, &PACK_BODY, &PACK_CALLS, pack_thunk as *const () as usize),
        (&S_VBV, VIEW_BOUNDS_VISIBLE, &VBV_BODY, &VBV_CALLS, vbv_thunk as *const () as usize),
    ];
    let mut ok = 0;
    for (s, addr, body, calls, thunk) in list {
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(addr, body, calls) } {
            Ok(copy) => s.orig.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_render2: {}: no shadow copy: {e:?}", s.name)),
        }
        if replace_pinned(s.name, addr, body, thunk) {
            ok += 1;
        }
    }
    let mut total = list.len();
    for (name, addr, body, thunk) in [
        ("city_instance_add", CITY_INSTANCE_ADD, &ADD_BODY[..], city_instance_add_rs as *const () as usize),
        ("city_instance_submit", CITY_INSTANCE_SUBMIT, &SUBMIT_BODY[..], city_instance_submit_rs as *const () as usize),
        ("city_submit_chunk_extras", CITY_SUBMIT_CHUNK_EXTRAS, &EXTRAS_BODY[..], city_submit_chunk_extras_rs as *const () as usize),
        ("city_instance_batch_begin", CITY_INSTANCE_BATCH_BEGIN, &BEGIN_BODY[..], city_instance_batch_begin_rs as *const () as usize),
    ] {
        total += 1;
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_render2: replaced {ok}/{total} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut parts = Vec::new();
    for s in [&S_FRUSTUM, &S_PACK, &S_VBV] {
        let c = s.calls.load(Ordering::Relaxed);
        if c > 0 {
            parts.push(format!("{} {} ({})", s.name, c, s.bad.load(Ordering::Relaxed)));
        }
    }
    let br: Vec<u32> = ADD_BRANCH.iter().map(|c| c.load(Ordering::Relaxed)).collect();
    if br[0] > 0 {
        parts.push(format!("city_instance_add {} (branches {:?})", br[0], &br[1..]));
    }
    let sb: Vec<u32> = SUB_BRANCH.iter().map(|c| c.load(Ordering::Relaxed)).collect();
    if sb[0] > 0 {
        parts.push(format!("city_instance_submit {} (branches {:?})", sb[0], &sb[1..]));
    }
    if EXTRAS_SEEN.load(Ordering::Relaxed) > 0 {
        parts.push(format!(
            "city_submit_chunk_extras {} ({} box tests, {} submits)",
            EXTRAS_SEEN.load(Ordering::Relaxed),
            EXTRAS_TESTS.load(Ordering::Relaxed),
            EXTRAS_SUBMITS.load(Ordering::Relaxed)
        ));
    }
    if BEGIN_SEEN.load(Ordering::Relaxed) > 0 {
        parts.push(format!("city_instance_batch_begin {}", BEGIN_SEEN.load(Ordering::Relaxed)));
    }
    if parts.is_empty() { String::new() } else { format!("port_render2: {}", parts.join(", ")) }
}

// --- offline differential tests -----------------------------------------------------------------
//
// The test process maps the unpacked exe's sections at their real addresses (so the original
// bytes and their absolute data references run as they are), which needs a test exe that does not
// sit on them:
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" \
//   cargo test --release -p sr2-hook --target i686-pc-windows-msvc port_render2 -- --ignored --test-threads=1 --nocapture

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
    }

    /// Floats that exercise the comparisons: NaN (quiet / signalling), infinities, zeros, edges.
    fn special(rng: &mut Rng) -> f32 {
        match rng.below(16) {
            0 => f32::NAN,
            1 => f32::from_bits(0x7FA0_0001),
            2 => f32::INFINITY,
            3 => f32::NEG_INFINITY,
            4 => 0.0,
            5 => -0.0,
            6 => 1.0,
            7 => -1.0,
            8 => f32::MAX,
            9 => f32::MIN_POSITIVE / 4.0,
            10 => f32::from_bits(rng.u32()),
            11 => (rng.below(9) as f32) - 4.0,
            12 => 0.5,
            13 => -0.5,
            _ => (rng.u32() as i32 as f32) / 2147483648.0 * 4.0,
        }
    }

    /// Mostly small integers (so dot products hit exact zero) with some specials when `wild`.
    fn val(rng: &mut Rng, wild: bool) -> f32 {
        if wild && rng.below(6) == 0 { special(rng) } else { (rng.below(7) as f32) - 3.0 }
    }

    #[repr(C, align(16))]
    struct Buf<const N: usize>([u8; N]);

    static EXE: std::sync::OnceLock<(Vec<u8>, Vec<[usize; 4]>)> = std::sync::OnceLock::new();

    /// The exe file and its sections as (va, virtual size, raw offset, raw size).
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

    /// The exe's bytes for a virtual address range (zeros beyond a section's raw data).
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

    /// Maps a window of the exe's address space at its real addresses (the pages the originals
    /// read): the process may have other mappings in the low 4 MB, so only the windows are taken.
    fn map_window(lo: usize, hi: usize) {
        let (lo, hi) = (lo & !0xFFF, hi.div_ceil(0x1000) * 0x1000);
        let mut blk = lo & !0xFFFF;
        while blk < hi {
            let p = unsafe { VirtualAlloc(blk as *const c_void, 0x1_0000, MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
            let _ = p; // already reserved by an earlier window of the same block is fine
            blk += 0x1_0000;
        }
        let c = unsafe { VirtualAlloc(lo as *const c_void, hi - lo, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
        assert_eq!(c as usize, lo, "cannot map {lo:#x}..{hi:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
        let bytes = exe_bytes(lo, hi - lo);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), lo as *mut u8, bytes.len()) };
    }

    struct Origs {
        frustum: usize,
        pack: usize,
        vbv: usize,
        add: usize,
    }

    /// Maps the data windows and copies the original bodies to executable memory, their calls
    /// aimed at stubs (the paths the tests do not take) or at the copies.
    fn origs() -> &'static Origs {
        static O: std::sync::OnceLock<Origs> = std::sync::OnceLock::new();
        O.get_or_init(|| {
            map_window(0xDD_3000, 0xDD_6000);
            map_window(0xE5_B000, 0xE5_D000);
            map_window(0xE8_0000, 0xE9_0000);
            map_window(0x261_2000, 0x261_6000);
            map_window(0xDD_0000, 0xDD_2000);
            map_window(0x4B_4000, 0x4B_6000);
            map_window(0x4B_8000, 0x4B_A000);
            map_window(0x96_0000, 0x96_1000);
            map_window(0xBF_0000, 0xBF_1000);
            map_window(0x53_0000, 0x53_4000);
            map_window(0x252_6000, 0x252_8000);
                        map_window(0x52_F000, 0x53_0000);
            hook_wms();
            map_window(0x279_1000, 0x279_2000);
            let ret_stub = stub(&[0xC3]);
            let true_stub = stub(&[0xB0, 0x01, 0xC3]);
            let copy = |body: &[u8], calls: &[(usize, usize)]| -> usize {
                let p = stub(body);
                for &(off, target) in calls {
                    assert_eq!(body[off], 0xE8);
                    let rel = (target as i32).wrapping_sub((p + off + 5) as i32);
                    unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (p + off + 1) as *mut u8, 4) };
                }
                p
            };
            let frustum = copy(&FRUSTUM_BODY, &[(0xf8, ret_stub), (0x178, true_stub), (0x1a1, true_stub), (0x1c9, ret_stub)]);
            let pack = copy(&PACK_BODY, &[]);
            let dist = copy(&exe_bytes(0xBF_63B0, 0xBF_640E - 0xBF_63B0 + 1), &[]);
            let vbv = copy(&VBV_BODY, &[(0x26, dist), (0x5c, true_stub), (0x85, frustum)]);
            let add = copy(&ADD_BODY, &[(0x13e, RECORD_INIT), (0x202, WORLD_MESH_SUBMIT)]);
            Origs { frustum, pack, vbv, add }
        })
    }

    fn stub(code: &[u8]) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), code.len().max(64), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, code.len()) };
        p
    }

    /// Calls `f` with the given registers and stack args; returns (eax, ecx, edx).
    unsafe fn call_full(f: usize, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> (u32, u32, u32) {
        let mut out = [0u32; 3];
        let block: [u32; 8] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, out.as_mut_ptr() as u32];
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
                "call dword ptr [ebx]",
                "mov ebx, [ebx + 28]",
                "mov [ebx], eax", "mov [ebx + 4], ecx", "mov [ebx + 8], edx",
                "mov esp, ebp",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
                clobber_abi("C"),
            );
        }
        (out[0], out[1], out[2])
    }

    fn fill_set(b: &mut [u8], moff: usize, maxn: u32, rng: &mut Rng, wild: bool) {
        let n = 1 + rng.below(maxn);
        b[0..4].copy_from_slice(&n.to_le_bytes());
        for i in 0..n as usize {
            for k in 0..4 {
                let v = if k == 3 && !wild { (rng.below(5) as f32) - 2.0 } else { val(rng, wild) };
                b[0x10 + i * 16 + k * 4..][..4].copy_from_slice(&v.to_le_bytes());
                let m = if rng.below(2) == 0 { 0u32 } else { u32::MAX };
                b[moff + i * 16 + k * 4..][..4].copy_from_slice(&m.to_le_bytes());
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn frustum_matches_original() {
        let og = origs();
        unsafe { *(FRUSTUM_SSE_FLAG as *mut u8) = 1 };
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let (mut pass, mut fail) = (0u32, 0u32);
        for it in 0..400_000u32 {
            let wild = it % 3 == 0;
            let mut planes = Box::new(Buf::<0x200>([0; 0x200]));
            fill_set(&mut planes.0, 0xF0, 14, &mut rng, wild);
            let ncell = rng.below(4);
            let mut cells = Box::new(Buf::<{ 0x110 + 4 * 0x220 }>([0; 0x110 + 4 * 0x220]));
            for c in 0..ncell as usize {
                fill_set(&mut cells.0[0x10 + c * 0x220..], 0x180, 10, &mut rng, wild);
            }
            let mut mn = Buf::<32>([0; 32]);
            let mut mx = Buf::<32>([0; 32]);
            for k in 0..3 {
                mn.0[k * 4..][..4].copy_from_slice(&val(&mut rng, wild).to_le_bytes());
                mx.0[k * 4..][..4].copy_from_slice(&val(&mut rng, wild).to_le_bytes());
            }
            let args = [planes.0.as_ptr() as u32, cells.0.as_ptr() as u32, ncell];
            let (mp, mxp) = (mn.0.as_ptr() as u32, mx.0.as_ptr() as u32);
            let o = unsafe { call_full(og.frustum, mp, mxp, 0x11, 0x22, &args) };
            let r = unsafe { call_full(frustum_thunk as *const () as usize, mp, mxp, 0x11, 0x22, &args) };
            assert_eq!(o.0, r.0, "iter {it}: eax");
            assert_eq!((r.1, r.2), (mp, mxp), "thunk must keep ecx / edx");
            if o.0 & 0xFF != 0 { pass += 1 } else { fail += 1 }
        }
        println!("frustum: {pass} visible, {fail} culled, all equal");
        assert!(pass > 1000 && fail > 1000);
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn normal_pack_matches_original() {
        let og = origs();
        let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
        for it in 0..2_000_000u32 {
            let mut v = [0f32; 4];
            for x in &mut v {
                *x = match rng.below(8) {
                    0 => special(&mut rng),
                    1 => ((rng.below(511) as f32) - 255.0) / 255.0,
                    2 => ((rng.below(256) as f32) * 2.0 - 255.0 + 0.5 * (rng.below(3) as f32 - 1.0)) / 255.0,
                    3 => (rng.u32() as i32 as f32) / 2147483648.0 * 3.0,
                    _ => (rng.u32() as i32 as f32) / 2147483648.0,
                };
            }
            let (mut oa, mut ob) = (0xA5A5A5A5u32, 0x5A5A5A5Au32);
            let ra = [&mut oa as *mut u32 as u32, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()];
            let rb2 = [&mut ob as *mut u32 as u32, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()];
            let o = unsafe { call_full(og.pack, 7, 8, 9, 10, &ra) };
            let r = unsafe { call_full(pack_thunk as *const () as usize, 7, 8, 9, 10, &rb2) };
            assert_eq!(oa, ob, "iter {it}: stored dword for {v:?}");
            assert_eq!(o.0, ra[0], "orig eax = out");
            assert_eq!(r.0, rb2[0], "port eax = out");
            assert_eq!(o.1, r.1, "ecx {v:?}");
            assert_eq!(o.2, r.2, "edx {v:?}");
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn view_bounds_visible_matches_original() {
        let og = origs();
        unsafe { *(FRUSTUM_SSE_FLAG as *mut u8) = 1 };
        let mut rng = Rng(0x0F0F_1234_ABCD_9876);
        let (mut pass, mut fail) = (0u32, 0u32);
        for it in 0..300_000u32 {
            let wild = it % 3 == 0;
            let idx = rng.below(4);
            let e = VIEW_TABLE + idx as usize * 0x30;
            let o = OCCLUDER_TABLE + idx as usize * 0x280;
            unsafe {
                for k in 0..3 {
                    wf(e + k * 4, val(&mut rng, wild) * 2.0);
                }
                wf(e + 12, if rng.below(4) == 0 { special(&mut rng) } else { (rng.below(40) as f32) * 0.5 });
                wb(e + 0x20, (rng.below(4) != 0) as u8);
                wb(e + 0x21, 0);
                wb(o + 0x64, (rng.below(3) == 0) as u8);
                let set = std::slice::from_raw_parts_mut((o + 0x70) as *mut u8, 0x1E0);
                fill_set(set, 0xF0, 14, &mut rng, wild);
            }
            let mut mn = Buf::<32>([0; 32]);
            let mut mx = Buf::<32>([0; 32]);
            for k in 0..4 {
                mn.0[k * 4..][..4].copy_from_slice(&val(&mut rng, wild).to_le_bytes());
                mx.0[k * 4..][..4].copy_from_slice(&val(&mut rng, wild).to_le_bytes());
            }
            let (mp, mxp) = (mn.0.as_ptr() as u32, mx.0.as_ptr() as u32);
            let a = unsafe { call_full(og.vbv, 0xC1, 0xD2, idx, mp, &[mxp]) };
            let b = unsafe { call_full(vbv_thunk as *const () as usize, 0xC1, 0xD2, idx, mp, &[mxp]) };
            assert_eq!(a.0 & 0xFF, b.0 & 0xFF, "iter {it}: al");
            assert_eq!((b.1, b.2), (0xC1, 0xD2));
            if a.0 & 0xFF != 0 { pass += 1 } else { fail += 1 }
        }
        println!("view_bounds_visible: {pass} visible, {fail} culled, all equal");
        assert!(pass > 1000 && fail > 1000);
    }


    /// world_mesh_submit is replaced by a logging stub (both the copies and the port call it here).
    fn hook_wms() {
        let jmp = [&[0x68u8][..], &(wms_stub as *const () as usize as u32).to_le_bytes(), &[0xC3]].concat();
        unsafe { std::ptr::copy_nonoverlapping(jmp.as_ptr(), WORLD_MESH_SUBMIT as *mut u8, jmp.len()) };
    }

    /// What the logging stub of `world_mesh_submit` saw on its last call.
    static mut WMS_REC: [u32; 0xB8] = [0; 0xB8];
    static mut WMS_ARGS: [u32; 7] = [0; 7];
    static mut WMS_COUNT: u32 = 0;
    static mut WMS_RET: u32 = 0;

    #[unsafe(naked)]
    extern "C" fn wms_stub() {
        std::arch::naked_asm!(
            "push esi",
            "push edi",
            "push ecx",
            "mov esi, eax",
            "lea edi, [{rec}]",
            "mov ecx, 0xB8",
            "rep movsd",
            "lea esi, [esp + 16]",
            "lea edi, [{args}]",
            "mov ecx, 7",
            "rep movsd",
            "inc dword ptr [{count}]",
            "pop ecx",
            "pop edi",
            "pop esi",
            "mov eax, [{ret}]",
            "ret",
            rec = sym WMS_REC,
            args = sym WMS_ARGS,
            count = sym WMS_COUNT,
            ret = sym WMS_RET,
        )
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn city_instance_add_matches_original() {
        let og = origs();
        hook_wms();
        let mut rng = Rng(0x5EED_1234_ABCD_0042);
        let mut submitted = 0u32;
        let mut shapes = [0u32; 4];
        for it in 0..300_000u32 {
            // the chunk: +0x4C -> records of 0x50 bytes
            let mut recs = Box::new(Buf::<0x200>([0; 0x200]));
            for i in 0..4 {
                let rec = &mut recs.0[i * 0x50..(i + 1) * 0x50];
                for b in rec.iter_mut() {
                    *b = rng.u32() as u8;
                }
                // flags A (+0x30) and B (+0x34): clear the rejecting bits most of the time
                let mut a = rng.u32() & rng.u32();
                let mut bb = rng.u32() & rng.u32();
                if rng.below(4) != 0 {
                    a &= !0xC138_8020;
                    bb &= !0x22000;
                }
                if rng.below(2) == 0 {
                    bb &= !0x400;
                }
                if rng.below(3) != 0 {
                    a |= 0x1000_0000;
                }
                rec[0x30..0x34].copy_from_slice(&a.to_le_bytes());
                rec[0x34..0x38].copy_from_slice(&bb.to_le_bytes());
            }
            let mut chunk = Box::new(Buf::<0x100>([0; 0x100]));
            chunk.0[0x4C..0x50].copy_from_slice(&(recs.0.as_ptr() as u32).to_le_bytes());
            let obj = Box::new(Buf::<0x40>([0; 0x40]));
            unsafe {
                wb(0x0252_7D17, (rng.below(3) == 0) as u8);
                wb(0x0252_7E04, (rng.below(4) != 0) as u8);
                let objp = if rng.below(4) == 0 { 0 } else { obj.0.as_ptr() as u32 };
                wd(0x0261_2D00, objp);
                if objp != 0 {
                    wd(objp as usize + 0x20, rng.u32() & rng.u32() | if rng.below(2) == 0 { 0 } else { 8 });
                }
                for k in 0..3 {
                    wd(0x0261_572C + k * 4, rng.u32());
                }
                WMS_RET = rng.u32();
            }
            let index = rng.below(4);
            let lod = rng.u32();
            let pass = match rng.below(10) {
                0 => rng.u32(),
                1..=5 => 1,
                _ => 2,
            };
            let args = [chunk.0.as_ptr() as u32, index, lod, pass];
            let mut saved: Option<(u32, Vec<u32>, [u32; 7], u32)> = None;
            for which in 0..2 {
                unsafe {
                    WMS_COUNT = 0;
                    WMS_REC = [0xDEAD_BEEF; 0xB8];
                    WMS_ARGS = [0; 7];
                }
                let f = if which == 0 { og.add } else { city_instance_add_rs as *const () as usize };
                let r = unsafe { call_full(f, 0x1111, 0x2222, 0x3333, 0x4444, &args) };
                // the registers the original preserves
                let (count, rec, wargs) = unsafe { (std::ptr::addr_of!(WMS_COUNT).read(), std::ptr::addr_of!(WMS_REC).read().to_vec(), std::ptr::addr_of!(WMS_ARGS).read()) };
                // initialised record dwords: +0..0x3C, +0x2A8, +0x2AC, +0x2BC
                let sel = |rec: &Vec<u32>| -> Vec<u32> {
                    let mut v: Vec<u32> = rec[0..15].to_vec();
                    v[3] &= 0xFFFF; // +0xE / +0xF are never written
                    v.push(rec[0x2A8 / 4] & 0xFF00);
                    v.push(rec[0x2AC / 4]);
                    v.push(rec[0x2BC / 4]);
                    v
                };
                let ret = if count > 0 { r.0 } else { r.0 & 0xFF };
                let cur = (count, if count > 0 { sel(&rec) } else { vec![] }, if count > 0 { wargs } else { [0; 7] }, ret);
                match which {
                    0 => saved = Some(cur),
                    _ => {
                        let o = saved.take().unwrap();
                        // args[0] of the stub is the pass; record pointers are chunk-relative (equal)
                        assert_eq!(o.0, cur.0, "iter {it}: submitted?");
                        assert_eq!(o.1, cur.1, "iter {it}: record");
                        assert_eq!(o.2, cur.2, "iter {it}: stub args");
                        assert_eq!(o.3, cur.3, "iter {it}: result");
                        if o.0 > 0 {
                            submitted += 1;
                            shapes[(pass == 1) as usize] += 1;
                        }
                    }
                }
            }
        }
        println!("city_instance_add: {submitted} submitted of 300000, pass1/pass2 submitted {shapes:?}, all equal");
        assert!(submitted > 5000);
    }


    // ---- city_instance_submit: callee stubs that log what they see ---------------------------------

    static mut LOG: [u32; 4096] = [0; 4096];
    static mut LOGN: usize = 0;
    static mut LAST_ID: u32 = u32::MAX;
    static mut SEEN: [u32; 9] = [0; 9];
    /// Values the stubs return / write, set per iteration (index = stub id).
    static mut STUB_RET: [u32; 9] = [0; 9];
    static mut STUB_X: [u32; 2] = [0; 2];

    unsafe fn log_push(v: u32) {
        unsafe {
            let n = LOGN;
            if n < 4096 {
                LOG[n] = v;
                LOGN = n + 1;
            }
        }
    }

    unsafe fn log_record(rec: usize) {
        unsafe {
            for i in 0..15 {
                let mut v = rd(rec + i * 4);
                if i == 3 {
                    v &= 0xFFFF; // +0xE / +0xF are never written
                }
                log_push(v);
            }
            log_push(rd(rec + 0x2AC));
            log_push(rd(rec + 0x2BC));
        }
    }

    /// `blk`: pushad block (edi, esi, ebp, esp, ebx, edx, ecx, eax), then the stub id, the return
    /// address and the callee's stack arguments.
    unsafe extern "C" fn stub_handler(blk: *mut u32) {
        unsafe {
            let (edi, esi, ecx, eax) = (*blk, *blk.add(1), *blk.add(6), *blk.add(7));
            let id = *blk.add(8);
            let arg = |i: usize| *blk.add(10 + i);
            LAST_ID = id;
            SEEN[id as usize] += 1;
            log_push(id);
            match id {
                0 => log_push(eax),
                1 => {
                    for i in 0..3 {
                        log_push(arg(i));
                    }
                }
                2 => {
                    for i in 0..3 {
                        log_push(rd(edi as usize + i * 4));
                    }
                    log_push(arg(0));
                }
                3 => {
                    log_push(rd(esi as usize));
                    log_push(rd(esi as usize + 4));
                    log_push(arg(0));
                    log_push(arg(1));
                    wd(esi as usize + 8, STUB_X[0]);
                    wd(esi as usize + 12, STUB_X[1]);
                    *blk.add(7) = esi;
                    return;
                }
                4 => {
                    let obj = rd(ecx as usize + 4);
                    log_push(rd(ecx as usize));
                    log_push(obj);
                    if obj != 0 {
                        log_push(rd(ecx as usize + 8));
                        log_push(rd(ecx as usize + 12));
                    }
                }
                5 => {
                    log_push(rd(eax as usize));
                    log_push(rd(eax as usize + 4));
                }
                6 => {
                    log_record(eax as usize);
                    log_push(ecx);
                    for i in 0..3 {
                        log_push(arg(i));
                    }
                }
                8 => {
                    for i in 0..4 {
                        log_push(arg(i));
                    }
                }
                _ => {
                    log_record(eax as usize);
                    for i in 0..7 {
                        log_push(arg(i));
                    }
                }
            }
            *blk.add(7) = STUB_RET[id as usize];
        }
    }

    macro_rules! stub_common {
        ($name:ident, $ret:literal) => {
            #[unsafe(naked)]
            extern "C" fn $name() {
                std::arch::naked_asm!(
                    "pushad",
                    "mov eax, esp",
                    "push eax",
                    "call {h}",
                    "add esp, 4",
                    "popad",
                    "add esp, 4",
                    $ret,
                    h = sym stub_handler,
                )
            }
        };
    }
    stub_common!(stub_common0, "ret");
    stub_common!(stub_common8, "ret 8");

    /// `push id; push common; ret` at the callee's real address.
    fn hook_stub(addr: usize, id: u32, common: usize) {
        let code = [&[0x68u8][..], &id.to_le_bytes(), &[0x68], &(common as u32).to_le_bytes(), &[0xC3]].concat();
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), addr as *mut u8, code.len()) };
    }

    fn poison_stack() {
        #[inline(never)]
        fn go() {
            let mut buf = [0xA5A5_A5A5u32; 4096];
            std::hint::black_box(&mut buf);
        }
        go();
    }

    fn x87_top() -> u32 {
        let w: u32;
        unsafe { std::arch::asm!("fnstsw ax", out("eax") w, options(nomem, nostack)) };
        (w >> 11) & 7
    }

    #[repr(C, align(16))]
    struct Arena([u8; 0x2000]);

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn city_instance_submit_matches_original() {
        origs();
        for (addr, id, common) in [
            (0x0096_0D60usize, 0u32, stub_common0 as *const () as usize),
            (0x004B_9310, 1, stub_common0 as *const () as usize),
            (0x0053_3880, 2, stub_common0 as *const () as usize),
            (0x004B_83A0, 3, stub_common8 as *const () as usize),
            (0x0053_0520, 4, stub_common0 as *const () as usize),
            (0x004B_8370, 5, stub_common0 as *const () as usize),
            (0x0052_FAF0, 6, stub_common0 as *const () as usize),
            (0x0052_F910, 7, stub_common0 as *const () as usize),
        ] {
            hook_stub(addr, id, common);
        }
        let targets: [(usize, usize); 14] = [
            (0x7f, 0x960D60),
            (0x16b, 0x4B45F0),
            (0x181, 0x4B4610),
            (0x19f, 0x4B4470),
            (0x1c1, 0x4B83D0),
            (0x1cd, 0x4B9310),
            (0x1f5, 0x4B52D0),
            (0x323, 0xBF0E00),
            (0x362, 0x533880),
            (0x3be, 0x4B83A0),
            (0x3c7, 0x530520),
            (0x3d4, 0x4B8370),
            (0x4bf, 0x52FAF0),
            (0x4f3, 0x52F910),
        ];
        assert_eq!(targets.map(|t| t.0), SUBMIT_CALLS);
        let orig = {
            let p = stub(&SUBMIT_BODY);
            for &(off, target) in &targets {
                let rel = (target as i32).wrapping_sub((p + off + 5) as i32);
                unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (p + off + 1) as *mut u8, 4) };
            }
            p
        };
        let rust = city_instance_submit_rs as *const () as usize;

        let mut rng = Rng(0x0C17_1157_A9CE_0001);
        let mut arena = Box::new(Arena([0; 0x2000]));
        let base = arena.0.as_ptr() as usize;
        let mut seen_total = [0u32; 9];
        let (mut submitted, mut alt_sub) = (0u32, 0u32);
        for it in 0..200_000u32 {
            let a = &mut arena.0;
            for x in a.iter_mut() {
                *x = rng.u32() as u8;
            }
            let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
            let fl = |rng: &mut Rng| match rng.below(10) {
                0 => special(rng),
                _ => (rng.below(9) as f32) - 4.0,
            };
            const CHUNK: usize = 0;
            const RECS: usize = 0x100;
            const ENTRIES: usize = 0x240;
            const OBJ: usize = 0x300;
            const MESH0: usize = 0x600;
            const MESH1: usize = 0x800;
            const PQ: usize = 0xA00;
            const FADE: usize = 0xB00;
            const COND: usize = 0xC00;
            const VIEW: usize = 0x1800;
            put(a, CHUNK + 0x4C, (base + RECS) as u32);
            put(a, CHUNK + 0x58, (base + ENTRIES) as u32);
            put(a, CHUNK + 0x20, [7, 8, 7, 8, 0, 1, 5][rng.below(7) as usize]);
            a[CHUNK + 0x1C] = [0, 1, 0x10, 0x11, 0x20, 0xFF][rng.below(6) as usize];
            for i in 0..4 {
                let r = RECS + i * 0x50;
                for k in [0usize, 4, 8, 0x10, 0x14, 0x18] {
                    let v = fl(&mut rng);
                    put(a, r + k, v.to_bits());
                }
                put(a, r + 0xC, (base + OBJ) as u32);
                let mut fa = rng.u32() & rng.u32();
                let mut fb = rng.u32() & rng.u32();
                if rng.below(4) != 0 {
                    fa &= !0xC138_8020;
                    fb &= !0x22000;
                }
                if rng.below(2) == 0 {
                    fb &= !0x400;
                }
                if rng.below(3) != 0 {
                    fa |= 0x1000_0000;
                }
                if rng.below(3) == 0 {
                    fb |= 0x8;
                }
                if rng.below(2) == 0 {
                    fb &= !0x100;
                }
                put(a, r + 0x30, fa);
                put(a, r + 0x34, fb);
                put(a, r + 0x40, (base + MESH0) as u32);
                put(a, r + 0x48, if rng.below(2) == 0 { 0 } else { (base + MESH1) as u32 });
                a[ENTRIES + i * 8 + 6] = (rng.below(3) & 1) as u8;
            }
            for m in [MESH0, MESH1] {
                let radius = match rng.below(8) {
                    0 => 0.0,
                    1 => f32::NAN,
                    2 => 1.0,
                    3 => 0.5,
                    4 => f32::from_bits(0x7FA0_0000),
                    _ => (rng.below(5) as f32) * 0.75,
                };
                put(a, m + 0x4C, radius.to_bits());
                put(a, m + 0x58, (base + PQ) as u32);
                let flags = if rng.below(8) == 0 { 0x40_0000 } else { rng.u32() & !0x40_0000 };
                put(a, m + 0x5C, flags);
            }
            put(a, PQ, (base + PQ + 0x10) as u32);
            put(a, OBJ + 0xCC, if rng.below(3) != 0 { 0x10 } else { 0 });
            put(a, OBJ + 0xD0, if rng.below(4) != 0 { 3 } else { 1 });
            put(a, OBJ + 0x1A0, rng.u32());
            put(a, FADE + 0x80, [0u32, 0x10, 4, 0x14, 1][rng.below(5) as usize]);
            a[COND + 0xAFF] = rng.below(2) as u8;
            let view_flags = (rng.u32() & rng.u32()) | if rng.below(2) == 0 { 0 } else { 0x108 };
            put(a, VIEW + 0x20, view_flags);
            unsafe {
                wb(0x0252_7D14, rng.below(2) as u8);
                wd(0x0252_7D10, (base + COND) as u32);
                wb(0x0252_7E04, (rng.below(5) != 0) as u8);
                wb(0x0252_6CBD, rng.below(2) as u8);
                *(0x0252_6CD0 as *mut u16) = rng.u32() as u16;
                wb(0x0252_7D16, rng.below(2) as u8);
                wd(0x0261_2D00, if rng.below(4) == 0 { 0 } else { (base + VIEW) as u32 });
                for k in 0..3 {
                    wd(0x0261_2D04 + k * 4, rng.u32());
                    wd(0x0261_5720 + k * 4, rng.u32());
                }
                STUB_RET = [if rng.below(2) == 0 { 0 } else { (base + FADE) as u32 }, rng.u32(), rng.u32(), 0, rng.u32() & 0xFFFF, 0, rng.u32(), rng.u32(), 0];
                STUB_X = [rng.u32(), rng.u32()];
            }
            let index = rng.below(4);
            let alt = if rng.below(2) == 0 { 0 } else { rng.u32() | 1 };
            let pass = [0u32, 0, 0, 1, 2, 3, 4, 4][rng.below(8) as usize];
            let args = [base as u32 + CHUNK as u32, index, alt, pass];
            let snapshot: Vec<u8> = arena.0.to_vec();
            let mut results: Vec<(u32, Vec<u32>, Vec<u8>, u32)> = Vec::new();
            for f in [orig, rust] {
                arena.0.copy_from_slice(&snapshot);
                unsafe {
                    LOGN = 0;
                    LAST_ID = u32::MAX;
                    SEEN = [0; 9];
                }
                poison_stack();
                let top0 = x87_top();
                let r = unsafe { call_full(f, 0x1111, 0x2222, 0x3333, 0x4444, &args) };
                assert_eq!(x87_top(), top0, "iter {it}: x87 depth");
                let (log, last) = unsafe { (LOG[..LOGN].to_vec(), LAST_ID) };
                if f == orig {
                    unsafe {
                        for k in 0..9 {
                            seen_total[k] += SEEN[k];
                        }
                    }
                }
                results.push((r.0, log, arena.0.to_vec(), last));
            }
            let (o, r) = (&results[0], &results[1]);
            assert_eq!(o.1, r.1, "iter {it}: callee log (args {args:x?})");
            assert_eq!(o.3, r.3);
            assert_eq!(o.2, r.2, "iter {it}: memory");
            if o.3 == 6 || o.3 == 7 {
                assert_eq!(o.0, r.0, "iter {it}: result");
                submitted += 1;
                if alt & 0xFF != 0 {
                    alt_sub += 1;
                }
            } else {
                assert_eq!(o.0 & 0xFF, r.0 & 0xFF, "iter {it}: al");
            }
        }
        println!("city_instance_submit: {submitted} reached a submit ({alt_sub} alt); callee stubs hit by id: {seen_total:?}; all equal");
        assert!(submitted > 2000);
        // stub 3 (4B83A0) is dead in practice: it needs record+0x34 non-zero after the init sets it to 0
        assert!(seen_total[2] > 100 && seen_total[4] > 100 && seen_total[1] > 100 && seen_total[0] > 100);
    }


    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn city_submit_chunk_extras_matches_original() {
        origs();
        map_window(0x173_A000, 0x173_B000);
        map_window(0xF6_F000, 0xF7_0000);
        map_window(0xF8_3000, 0xF8_4000);
        map_window(0x297_5000, 0x297_6000);
        hook_stub(0x004B_5510, 8, stub_common0 as *const () as usize);
        let orig = {
            let p = stub(&EXTRAS_BODY);
            for (off, target) in [(0xe5usize, 0x4B8280usize), (0xf4, 0x4B5510)] {
                let rel = (target as i32).wrapping_sub((p + off + 5) as i32);
                unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (p + off + 1) as *mut u8, 4) };
            }
            p
        };
        let rust = city_submit_chunk_extras_rs as *const () as usize;
        let mut rng = Rng(0x00E5_7EA5_1234_5678);
        let mut arena = Box::new(Arena([0; 0x2000]));
        let base = arena.0.as_ptr() as usize;
        let (mut calls, mut with_submits) = (0u32, 0u32);
        for it in 0..100_000u32 {
            let a = &mut arena.0;
            for x in a.iter_mut() {
                *x = rng.u32() as u8;
            }
            let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
            let fl = |rng: &mut Rng| match rng.below(12) {
                0 => special(rng),
                _ => (rng.below(41) as f32) - 20.0,
            };
            // up to 3 chunks of 0x400 bytes at base + i * 0x400; instance list at +0x300; records at +0x40.. (8 of 0x50 -> 0x280)
            let nchunks = rng.below(4);
            for c in 0..3usize {
                let ch = c * 0x500;
                put(a, ch + 0x4C, (base + ch + 0x40 + 0x0) as u32);
                // records: 0x50 each, 5 of them would need 0x1F0; the chunk header fields live at +0x20C / +0x210 -> keep records below 0x200
                a[ch + 8] = (rng.below(3)) as u8;
                a[ch + 9] = 0;
                put(a, ch + 0x20C, if rng.below(6) == 0 { 0 } else { rng.below(5) });
                put(a, ch + 0x210, (base + ch + 0x300) as u32);
                for k in 0..5 {
                    put(a, ch + 0x300 + k * 4, rng.below(3));
                }
                for r in 0..3usize {
                    let rec = ch + 0x40 + r * 0x50;
                    for off in [0usize, 8, 0x10, 0x18] {
                        let v = fl(&mut rng);
                        put(a, rec + off, v.to_bits());
                    }
                }
            }
            unsafe {
                wd(0x00F6_FBC4, nchunks);
                for c in 0..3usize {
                    wd(0x00F6_FBD8 + c * 4, (base + c * 0x500) as u32);
                }
                for kind in 0..3usize {
                    wb(0x00F8_3B08 + kind, (rng.below(3) != 0) as u8);
                }
                wd(0x0297_5940, rng.below(8));
                for k in 0..9usize {
                    wd(0x0297_5880 + k * 4, fl(&mut rng).to_bits());
                }
                wd(0x0173_A8CC, (rng.below(30) as f32).to_bits());
                wd(0x0173_A8D0, (rng.below(30) as f32).to_bits());
            }
            let mut logs: Vec<Vec<u32>> = Vec::new();
            for f in [orig, rust] {
                unsafe {
                    LOGN = 0;
                    SEEN = [0; 9];
                }
                poison_stack();
                let top0 = x87_top();
                unsafe { call_full(f, 0x1111, 0x2222, 0x3333, 0x4444, &[]) };
                assert_eq!(x87_top(), top0, "iter {it}: x87 depth");
                logs.push(unsafe { LOG[..LOGN].to_vec() });
            }
            assert_eq!(logs[0], logs[1], "iter {it}: submit calls");
            calls += 1;
            if !logs[0].is_empty() {
                with_submits += 1;
            }
        }
        println!("city_submit_chunk_extras: {with_submits} of {calls} runs submitted at least one instance, all equal");
        assert!(with_submits > 5000);
    }


    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn timing_extras() {
        origs();
        map_window(0x173_A000, 0x173_B000);
        map_window(0xF6_F000, 0xF7_0000);
        map_window(0xF8_3000, 0xF8_4000);
        map_window(0x297_5000, 0x297_6000);
        unsafe { std::ptr::copy_nonoverlapping([0x31u8, 0xC0, 0xC3].as_ptr(), 0x004B_5510 as *mut u8, 3) };
        let orig = {
            let p = stub(&EXTRAS_BODY);
            for (off, target) in [(0xe5usize, 0x4B8280usize), (0xf4, 0x4B5510)] {
                let rel = (target as i32).wrapping_sub((p + off + 5) as i32);
                unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (p + off + 1) as *mut u8, 4) };
            }
            p
        };
        let rust = city_submit_chunk_extras_rs as *const () as usize;
        let mut arena = Box::new(Arena([0; 0x2000]));
        let base = arena.0.as_ptr() as usize;
        let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
        // 2 chunks with 40 extras each, half overlapping the box
        for c in 0..2usize {
            let ch = c * 0x800;
            put(&mut arena.0, ch + 0x4C, (base + ch + 0x40) as u32);
            put(&mut arena.0, ch + 0x20C, 40);
            put(&mut arena.0, ch + 0x210, (base + ch + 0x400) as u32);
            for k in 0..40usize {
                put(&mut arena.0, ch + 0x400 + k * 4, (k % 8) as u32);
            }
            for r in 0..8usize {
                let rec = ch + 0x40 + r * 0x50;
                let x = if r % 2 == 0 { 0.0f32 } else { 500.0 };
                for (off, v) in [(0usize, x), (8, 0.0), (0x10, x + 10.0), (0x18, 10.0)] {
                    put(&mut arena.0, rec + off, v.to_bits());
                }
            }
        }
        unsafe {
            wd(0x00F6_FBC4, 2);
            wd(0x00F6_FBD8, base as u32);
            wd(0x00F6_FBDC, (base + 0x800) as u32);
            wb(0x00F8_3B08, 1);
            wd(0x0297_5940, 0);
            for k in 0..9usize {
                wd(0x0297_5880 + k * 4, 0.0f32.to_bits());
            }
            wd(0x0173_A8CC, 20.0f32.to_bits());
            wd(0x0173_A8D0, 20.0f32.to_bits());
            *((base + 8) as *mut u16) = 0;
            *((base + 0x808) as *mut u16) = 0;
        }
        const N: u32 = 300_000;
        let a = ns(|| unsafe { std::hint::black_box(call_full(orig, 0, 0, 0, 0, &[])); }, N);
        let b = ns(|| unsafe { std::hint::black_box(call_full(rust, 0, 0, 0, 0, &[])); }, N);
        println!("city_submit_chunk_extras (80 extras, 40 tests passing): orig {a:.0} ns, rust {b:.0} ns, ratio {:.2}", b / a);
    }


    // ---- batch_begin: the two virtual calls are stubs that log ------------------------------------

    static mut VLOG: [u32; 16] = [0; 16];
    static mut VLOGN: usize = 0;
    static mut V_RET: [u32; 2] = [0; 2];

    #[unsafe(naked)]
    extern "C" fn vcall_scope() {
        std::arch::naked_asm!(
            "mov edx, dword ptr [{n}]",
            "mov dword ptr [{log} + edx*4], ecx",
            "inc dword ptr [{n}]",
            "mov eax, dword ptr [{ret}]",
            "ret",
            n = sym VLOGN,
            log = sym VLOG,
            ret = sym V_RET,
        )
    }

    #[unsafe(naked)]
    extern "C" fn vcall_alloc() {
        std::arch::naked_asm!(
            "mov edx, dword ptr [{n}]",
            "mov dword ptr [{log} + edx*4], ecx",
            "mov eax, dword ptr [esp + 4]",
            "mov dword ptr [{log} + edx*4 + 4], eax",
            "mov eax, dword ptr [esp + 8]",
            "mov dword ptr [{log} + edx*4 + 8], eax",
            "add dword ptr [{n}], 3",
            "mov eax, dword ptr [{ret} + 4]",
            "ret 8",
            n = sym VLOGN,
            log = sym VLOG,
            ret = sym V_RET,
        )
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn batch_begin_matches_original() {
        origs();
        map_window(0xEC_5000, 0xEC_6000);
        map_window(0x277_4000, 0x277_5000);
        map_window(0xF8_3000, 0xF8_4000);
        let orig = stub(&BEGIN_BODY);
        let rust = city_instance_batch_begin_rs as *const () as usize;
        let vt_scope: Box<[u32; 8]> = Box::new([0, 0, vcall_scope as *const () as usize as u32, 0, 0, 0, 0, 0]);
        let vt_alloc: Box<[u32; 8]> = Box::new([0, 0, 0, 0, 0, 0, 0, vcall_alloc as *const () as usize as u32]);
        unsafe {
            wd(0x00EC_5BB8, vt_scope.as_ptr() as u32);
            wd(0x0277_4148, vt_alloc.as_ptr() as u32);
        }
        let mut rng = Rng(0xBA7C_4B36_0011_2233);
        for it in 0..2000u32 {
            let mut res: Vec<(Vec<u32>, [u32; 4], Vec<u8>)> = Vec::new();
            let cb = rng.u32();
            let scope = rng.below(2) == 0;
            let seed = rng.u32();
            for f in [orig, rust] {
                let mut obj = Box::new([0u8; 0x200]);
                for (i, b) in obj.iter_mut().enumerate() {
                    *b = (seed.wrapping_add(i as u32 * 7)) as u8;
                }
                let objp = obj.as_ptr() as u32;
                unsafe {
                    V_RET = [objp, 0xA10C_0000 + it];
                    VLOGN = 0;
                    wd(0x0252_6CC8, if scope { 0x1234_5678 } else { 0 });
                    wd(0x00F8_3AF0, 1);
                    wd(0x00F8_3AF4, 2);
                    wd(0x00F8_3AF8, 3);
                    call_full(f, 0x1111, 0x2222, 0x3333, 0x4444, &[cb]);
                }
                let log = unsafe { VLOG[..VLOGN].to_vec() };
                let g = unsafe { [rd(0x0252_6CC8), rd(0x00F8_3AF0), rd(0x00F8_3AF4), rd(0x00F8_3AF8)] };
                // pointers into the scratch object differ by run; normalise them
                let norm = |v: u32| if v.wrapping_sub(objp) < 0x200 { v.wrapping_sub(objp) | 0xFFFF_0000 } else { v };
                res.push((log.iter().map(|&v| norm(v)).collect(), g.map(norm), obj.to_vec()));
            }
            assert_eq!(res[0].0, res[1].0, "iter {it}: virtual calls");
            assert_eq!(res[0].1, res[1].1, "iter {it}: globals");
            assert_eq!(res[0].2, res[1].2, "iter {it}: scope object");
        }
    }

    // ---- gfx_bind_shaders ---------------------------------------------------------------------

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn gfx_bind_shaders_matches_original() {
        origs();
        map_window(0x22F_2000, 0x22F_6000);
        map_window(0x33D_6000, 0x33D_7000);
        map_window(0xE8_4000, 0xE8_5000);
        let orig = stub(&BIND_BODY);
        let rust = bind_shaders_thunk as *const () as usize;
        let mut rng = Rng(0x5AD3_0000_7777_1234);
        let (mut vs_w, mut ps_w, mut p7) = (0u32, 0u32, 0u32);
        for it in 0..300_000u32 {
            // arena layout (offsets): tls array 0, tls block 0x10, ctx 0x100 (0x500 bytes), shader table 0x700
            // (4 x 12), shaders 0x800.. (0x80 each, 2), technique tables, lod tables, pair arrays, ring
            let mut arena = Box::new(Arena([0; 0x2000]));
            let base = arena.0.as_ptr() as usize;
            for x in arena.0.iter_mut() {
                *x = rng.u32() as u8;
            }
            let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
            put(&mut arena.0, 0, (base + 0x10) as u32);
            put(&mut arena.0, 0x10 + 0x58, (base + 0x100) as u32);
            put(&mut arena.0, 0x100 + 0x484, (base + 0x700) as u32);
            arena.0[0x100 + 0x480] = (rng.below(2)) as u8;
            for s in 0..4usize {
                put(&mut arena.0, 0x700 + s * 12 + 8, (base + 0x800 + (s % 2) * 0x100) as u32);
            }
            for s in 0..2usize {
                let sh = 0x800 + s * 0x100;
                arena.0[sh + 9] = 1 + rng.below(3) as u8; // LOD count 1..3 (0 would index before the table)
                put(&mut arena.0, sh + 0x2C, (base + sh + 0x40) as u32); // technique tables: 8 passes x 8 bytes at +0x40
                put(&mut arena.0, sh + 0x28, (base + sh + 0xA0) as u32); // lod table at +0xA0 (4 entries)
                for pass in 0..8usize {
                    put(&mut arena.0, sh + 0x40 + pass * 8 + 4, (base + 0x1000 + (pass % 4) * 0x40) as u32);
                }
                for l in 0..4usize {
                    put(&mut arena.0, sh + 0xA0 + l * 4, (base + 0x1400 + (s * 4 + l) * 0x40) as u32);
                }
            }
            // column tables: i16 per column x 4 bytes stride; techniques 0..3
            for t in 0..4usize {
                for c in 0..4usize {
                    let v = rng.below(4) as u16;
                    arena.0[0x1000 + t * 0x40 + c * 4..][..2].copy_from_slice(&v.to_le_bytes());
                }
            }
            // pair arrays (8 bytes each: vs, ps) with zeros mixed in
            for l in 0..8usize {
                for t in 0..4usize {
                    for k in 0..2usize {
                        let v = if rng.below(3) == 0 { 0 } else { rng.below(4) + 1 };
                        put(&mut arena.0, 0x1400 + l * 0x40 + t * 8 + k * 4, v);
                    }
                }
            }
            let ring = base + 0x1800;
            let cache = |rng: &mut Rng| if rng.below(3) == 0 { 0 } else { rng.below(5) };
            let globals = [cache(&mut rng), cache(&mut rng), rng.below(2), cache(&mut rng), cache(&mut rng)];
            let id = rng.below(4);
            let lod = rng.below(6);
            let pass = rng.below(8);
            let column = rng.below(4);
            let (da, db) = (rng.u32(), rng.u32());
            let snapshot = arena.0.to_vec();
            let mut res: Vec<(Vec<u8>, [u32; 5], u32)> = Vec::new();
            for f in [orig, rust] {
                arena.0.copy_from_slice(&snapshot);
                unsafe {
                    wd(0x022F_2AEC, globals[0]);
                    wd(0x022F_2AE8, globals[1]);
                    wd(0x022F_5A90, globals[2]);
                    wd(0x00E8_4B98, globals[3]);
                    wd(0x00E8_4B9C, globals[4]);
                    wd(0x033D_62F8, ring as u32);
                }
                // fs:[0x2C] -> an array whose slot 0 is the TLS block of the test arena
                let saved: u32;
                unsafe {
                    std::arch::asm!("mov {0}, fs:[0x2C]", "mov fs:[0x2C], {1}", out(reg) saved, in(reg) base as u32, options(nostack));
                }
                let r = unsafe { call_with_eax(f, lod, id, &[pass, column, da, db]) };
                unsafe { std::arch::asm!("mov fs:[0x2C], {0}", in(reg) saved, options(nostack)) };
                let g = unsafe { [rd(0x022F_2AEC), rd(0x022F_2AE8), rd(0x022F_5A90), rd(0x033D_62F8), 0] };
                res.push((arena.0.to_vec(), g, r & 0xFF));
            }
            assert_eq!(res[0].0, res[1].0, "iter {it}: arena (ctx / ring)");
            assert_eq!(res[0].1, res[1].1, "iter {it}: globals");
            assert_eq!(res[0].2, 1);
            assert_eq!(res[1].2, 1);
            let ring_len = (res[0].1[3] as usize).wrapping_sub(ring);
            if ring_len > 0 {
                vs_w += (res[0].0[0x1800..0x1800 + ring_len].chunks(4).next() == Some(&4u32.to_le_bytes()[..])) as u32;
                ps_w += 1;
            }
            p7 += (pass == 7) as u32;
        }
        println!("gfx_bind_shaders: {vs_w} runs began with a VS write, {ps_w} wrote the ring, {p7} pass-7 runs, all equal");
        assert!(ps_w > 10_000);
    }

    /// EAX and ECX plus cdecl stack args (the original's gfx_bind_shaders takes EAX = LOD, ECX = id).
    unsafe fn call_with_eax(f: usize, eax: u32, ecx: u32, args: &[u32]) -> u32 {
        let mut out = 0u32;
        let block: [u32; 8] = [f as u32, ecx, eax, 0, 0, args.as_ptr() as u32, args.len() as u32, &mut out as *mut u32 as u32];
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
                "mov ecx, [ebx + 4]", "mov eax, [ebx + 8]",
                "call dword ptr [ebx]",
                "mov ebx, [ebx + 28]",
                "mov [ebx], eax",
                "mov esp, ebp",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
                clobber_abi("C"),
            );
        }
        out
    }


    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn timing_bind() {
        origs();
        map_window(0x22F_2000, 0x22F_6000);
        map_window(0x33D_6000, 0x33D_7000);
        map_window(0xE8_4000, 0xE8_5000);
        let orig = stub(&BIND_BODY);
        let rust = bind_shaders_thunk as *const () as usize;
        let mut arena = Box::new(Arena([0; 0x2000]));
        let base = arena.0.as_ptr() as usize;
        let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut arena.0, 0, (base + 0x10) as u32);
        put(&mut arena.0, 0x10 + 0x58, (base + 0x100) as u32);
        put(&mut arena.0, 0x100 + 0x484, (base + 0x700) as u32);
        arena.0[0x100 + 0x480] = 1;
        put(&mut arena.0, 0x700 + 8, (base + 0x800) as u32);
        arena.0[0x800 + 9] = 2;
        put(&mut arena.0, 0x800 + 0x2C, (base + 0x840) as u32);
        put(&mut arena.0, 0x800 + 0x28, (base + 0x8A0) as u32);
        put(&mut arena.0, 0x840 + 4, (base + 0x1000) as u32);
        put(&mut arena.0, 0x8A0, (base + 0x1400) as u32);
        put(&mut arena.0, 0x8A4, (base + 0x1400) as u32);
        put(&mut arena.0, 0x1400, 5);
        put(&mut arena.0, 0x1404, 6);
        let ring = base + 0x1800;
        let args = [0u32, 0, 7, 8];
        for (name, reset) in [("both caches miss", true), ("caches hit", false)] {
            let run = |f: usize| -> f64 {
                let saved: u32;
                unsafe { std::arch::asm!("mov {0}, fs:[0x2C]", "mov fs:[0x2C], {1}", out(reg) saved, in(reg) base as u32, options(nostack)) };
                unsafe {
                    wd(0x022F_5A90, 0);
                    wd(0x033D_62F8, ring as u32);
                }
                let r = ns(
                    || unsafe {
                        if reset {
                            wd(0x022F_2AEC, 0);
                            wd(0x022F_2AE8, 0);
                        }
                        wd(0x033D_62F8, ring as u32);
                        std::hint::black_box(call_with_eax(f, 0, 0, &args));
                    },
                    2_000_000,
                );
                unsafe { std::arch::asm!("mov fs:[0x2C], {0}", in(reg) saved, options(nostack)) };
                r
            };
            let a = run(orig);
            let b = run(rust);
            println!("gfx_bind_shaders {name}: orig {a:.1} ns, rust {b:.1} ns, ratio {:.2}", b / a);
        }
    }

    fn ns<F: FnMut()>(mut f: F, n: u32) -> f64 {
        for _ in 0..1000 {
            f();
        }
        let t = std::time::Instant::now();
        for _ in 0..n {
            f();
        }
        t.elapsed().as_nanos() as f64 / n as f64
    }


    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn timing_city() {
        let og = origs();
        // every callee that costs something is a bare `xor eax, eax; ret`
        for (addr, ret_n) in [
            (0x0096_0D60usize, 0u8),
            (0x004B_9310, 0),
            (0x0053_3880, 0),
            (0x004B_83A0, 8),
            (0x0053_0520, 0),
            (0x004B_8370, 0),
            (0x0052_FAF0, 0),
            (0x0052_F910, 0),
        ] {
            let code: Vec<u8> = if ret_n == 0 { vec![0x31, 0xC0, 0xC3] } else { vec![0x31, 0xC0, 0xC2, ret_n, 0] };
            unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), addr as *mut u8, code.len()) };
        }
        let targets: [(usize, usize); 14] = [
            (0x7f, 0x960D60),
            (0x16b, 0x4B45F0),
            (0x181, 0x4B4610),
            (0x19f, 0x4B4470),
            (0x1c1, 0x4B83D0),
            (0x1cd, 0x4B9310),
            (0x1f5, 0x4B52D0),
            (0x323, 0xBF0E00),
            (0x362, 0x533880),
            (0x3be, 0x4B83A0),
            (0x3c7, 0x530520),
            (0x3d4, 0x4B8370),
            (0x4bf, 0x52FAF0),
            (0x4f3, 0x52F910),
        ];
        let sub_orig = {
            let p = stub(&SUBMIT_BODY);
            for &(off, target) in &targets {
                let rel = (target as i32).wrapping_sub((p + off + 5) as i32);
                unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (p + off + 1) as *mut u8, 4) };
            }
            p
        };
        let mut arena = Box::new(Arena([0; 0x2000]));
        let base = arena.0.as_ptr() as usize;
        let put = |a: &mut [u8; 0x2000], off: usize, v: u32| a[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut arena.0, 0x4C, (base + 0x100) as u32);
        put(&mut arena.0, 0x58, (base + 0x240) as u32);
        put(&mut arena.0, 0x20, 0);
        put(&mut arena.0, 0x100 + 0x40, (base + 0x600) as u32);
        put(&mut arena.0, 0x100 + 0xC, (base + 0x300) as u32);
        put(&mut arena.0, 0x600 + 0x4C, 1.0f32.to_bits());
        put(&mut arena.0, 0x600 + 0x58, (base + 0xA00) as u32);
        put(&mut arena.0, 0xA00, (base + 0xA10) as u32);
        arena.0[0x240 + 6] = 1;
        unsafe {
            wb(0x0252_7D14, 0);
            wb(0x0252_7E04, 1);
            wb(0x0252_6CBD, 0);
            wd(0x0261_2D00, 0);
            wb(0x0252_7D16, 0);
        }
        const N: u32 = 3_000_000;
        let ret_stub = stub(&[0xC3]);
        for (name, fa, fb, alt, pass) in [
            ("submit main", 0x1000_0000u32, 0u32, 0u32, 0u32),
            ("submit main + effect", 0x1000_0000, 8, 0, 0),
            ("submit alt", 0x1000_0000, 0, 1, 0),
            ("submit filtered", 0xC000_0000, 0, 0, 0),
        ] {
            put(&mut arena.0, 0x100 + 0x30, fa);
            put(&mut arena.0, 0x100 + 0x34, fb);
            let args = [base as u32, 0, alt, pass];
            let rust = city_instance_submit_rs as *const () as usize;
            let a = ns(|| unsafe { std::hint::black_box(call_full(sub_orig, 0, 0, 0, 0, &args)); }, N);
            let b = ns(|| unsafe { std::hint::black_box(call_full(rust, 0, 0, 0, 0, &args)); }, N);
            let w = ns(|| unsafe { std::hint::black_box(call_full(ret_stub, 0, 0, 0, 0, &args)); }, N);
            println!("{name}: orig {a:.1} ns, rust {b:.1} ns, wrapper {w:.1} ns, ratio {:.2} (net {:.2})", b / a, (b - w) / (a - w));
        }
        for (name, fa, pass) in [("add pass 1", 0x1000_0000u32, 1u32), ("add filtered", 0xC000_0000, 1)] {
            put(&mut arena.0, 0x100 + 0x30, fa);
            put(&mut arena.0, 0x100 + 0x34, 0);
            let args = [base as u32, 0, 0, pass];
            let rust = city_instance_add_rs as *const () as usize;
            let a = ns(|| unsafe { std::hint::black_box(call_full(og.add, 0, 0, 0, 0, &args)); }, N);
            let b = ns(|| unsafe { std::hint::black_box(call_full(rust, 0, 0, 0, 0, &args)); }, N);
            let w = ns(|| unsafe { std::hint::black_box(call_full(ret_stub, 0, 0, 0, 0, &args)); }, N);
            println!("{name}: orig {a:.1} ns, rust {b:.1} ns, wrapper {w:.1} ns, ratio {:.2} (net {:.2})", b / a, (b - w) / (a - w));
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn timing() {
        let og = origs();
        unsafe { *(FRUSTUM_SSE_FLAG as *mut u8) = 1 };
        let ret_stub = stub(&[0xC3]);
        let ret20_stub = stub(&[0xC2, 0x14, 0x00]);
        // A typical frustum: 6 planes of the box [-100, 100] on each axis.
        let mut planes = Box::new(Buf::<0x200>([0; 0x200]));
        planes.0[0..4].copy_from_slice(&6u32.to_le_bytes());
        for i in 0..6usize {
            let axis = i / 2;
            let sign = if i % 2 == 0 { 1.0f32 } else { -1.0 };
            for k in 0..4 {
                let v = if k == axis { sign } else if k == 3 { -100.0 } else { 0.0 };
                planes.0[0x10 + i * 16 + k * 4..][..4].copy_from_slice(&v.to_le_bytes());
                // corner select: the far corner along the plane normal
                let m: u32 = if k == axis && sign > 0.0 { u32::MAX } else { 0 };
                planes.0[0xF0 + i * 16 + k * 4..][..4].copy_from_slice(&m.to_le_bytes());
            }
        }
        let cells = Box::new(Buf::<0x240>([0; 0x240]));
        let mk = |a: [f32; 3]| {
            let mut b = Box::new(Buf::<32>([0; 32]));
            for k in 0..3 {
                b.0[k * 4..][..4].copy_from_slice(&a[k].to_le_bytes());
            }
            b
        };
        let (mn_in, mx_in) = (mk([-1.0, -1.0, -1.0]), mk([1.0, 1.0, 1.0]));
        let (mn_out, mx_out) = (mk([500.0, 0.0, 0.0]), mk([501.0, 1.0, 1.0]));
        let args = [planes.0.as_ptr() as u32, cells.0.as_ptr() as u32, 0u32];
        let th = frustum_thunk as *const () as usize;
        const N: u32 = 3_000_000;
        for (name, mn, mx) in [("inside", &mn_in, &mx_in), ("outside", &mn_out, &mx_out)] {
            let (mp, mxp) = (mn.0.as_ptr() as u32, mx.0.as_ptr() as u32);
            let r1 = unsafe { call_full(og.frustum, mp, mxp, 0, 0, &args).0 };
            let r2 = unsafe { call_full(th, mp, mxp, 0, 0, &args).0 };
            assert_eq!(r1, r2);
            let a = ns(|| unsafe { std::hint::black_box(call_full(og.frustum, mp, mxp, 0, 0, &args)); }, N);
            let b = ns(|| unsafe { std::hint::black_box(call_full(th, mp, mxp, 0, 0, &args)); }, N);
            let base = ns(|| unsafe { std::hint::black_box(call_full(ret_stub, mp, mxp, 0, 0, &args)); }, N);
            println!("frustum {name} (result {r1}): orig {a:.1} ns, rust {b:.1} ns, wrapper {base:.1} ns, ratio {:.2} (net {:.2})", b / a, (b - base) / (a - base));
        }
        // normal_pack
        let vals = [0.25f32.to_bits(), (-0.5f32).to_bits(), 0.9f32.to_bits(), 1.0f32.to_bits()];
        let mut out = 0u32;
        let args = [&mut out as *mut u32 as u32, vals[0], vals[1], vals[2], vals[3]];
        let a = ns(|| unsafe { std::hint::black_box(call_full(og.pack, 0, 0, 0, 0, &args)); }, N);
        let b = ns(|| unsafe { std::hint::black_box(call_full(pack_thunk as *const () as usize, 0, 0, 0, 0, &args)); }, N);
        let base = ns(|| unsafe { std::hint::black_box(call_full(ret20_stub, 0, 0, 0, 0, &args)); }, N);
        println!("normal_pack: orig {a:.1} ns, rust {b:.1} ns, wrapper {base:.1} ns, ratio {:.2} (net {:.2})", b / a, (b - base) / (a - base));
        // view_bounds_visible: view 0 with radius, occluder planes set
        let e = VIEW_TABLE;
        let o = OCCLUDER_TABLE;
        unsafe {
            wf(e, 0.0);
            wf(e + 4, 0.0);
            wf(e + 8, 0.0);
            wf(e + 12, 100.0);
            wb(e + 0x20, 1);
            wb(e + 0x21, 0);
            wb(o + 0x64, 0);
            std::ptr::copy_nonoverlapping(planes.0.as_ptr(), (o + 0x70) as *mut u8, 0x1F0);
        }
        for (name, mn, mx) in [("inside", &mn_in, &mx_in), ("outside", &mn_out, &mx_out)] {
            let (mp, mxp) = (mn.0.as_ptr() as u32, mx.0.as_ptr() as u32);
            let th = vbv_thunk as *const () as usize;
            let r1 = unsafe { call_full(og.vbv, 0, 0, 0, mp, &[mxp]).0 & 0xFF };
            let r2 = unsafe { call_full(th, 0, 0, 0, mp, &[mxp]).0 & 0xFF };
            assert_eq!(r1, r2);
            let a = ns(|| unsafe { std::hint::black_box(call_full(og.vbv, 0, 0, 0, mp, &[mxp])); }, N);
            let b = ns(|| unsafe { std::hint::black_box(call_full(th, 0, 0, 0, mp, &[mxp])); }, N);
            let base = ns(|| unsafe { std::hint::black_box(call_full(ret_stub, 0, 0, 0, mp, &[mxp])); }, N);
            println!("view_bounds_visible {name} (result {r1}): orig {a:.1} ns, rust {b:.1} ns, wrapper {base:.1} ns, ratio {:.2} (net {:.2})", b / a, (b - base) / (a - base));
        }
    }
}
