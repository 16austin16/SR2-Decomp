//! Port module (frame-loop callees, round 4): frame-loop callees as whole-function replacements.
//! Callees stay the original game functions, called by address with the registers the LTCG build
//! sets up; every comparison and global write follows the disassembly.
//!
//! - activity_hud_markers_apply 0x5E6DF0 (443 bytes)
//! - pushback_phone_call_hud_text 0x565B30 (616)
//! - net_remote_objects_do_frame 0x804850 (112)
//! - net_registered_objects_update 0x8045F0 (285)
//! - septic_decals_update 0x56BD60 (211)
//! - restricted_zones_update 0xA58EE0 (225)
//! - humans_update_neighborhoods 0x9A7080 (207)
//! - hud_do_frame 0x793A40 (354)
//! - humans_character_step 0x9A9B60 (370)
//! - population_update 0x95A3D0 (247)
//!
//! The eight multi_registration::remote_do_frame bodies (item, mover, npc, pipebomb, player,
//! prop, rocket, vehicle) are not ported here: 0.9 - 8 KB of SSE quaternion code working on
//! overlapping stack buffers.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
include!("fix_thunks.inc");

// Thunks that keep XMM0-7, give the Rust body `out: *mut [u32; 7]` = entry [ecx, edx, eax, ebx, esi, edi, ebp] as FIRST parameter
// (the original's ECX/EDX at exit are whatever the body leaves in out[0]/out[1]; EAX at exit is the body's u32 return value).
// kzN: N stack arguments follow `out`. kzf/kzfa: the thunk first reserves the original's local frame (`sub esp, FRAME`, or the
// original's `mov eax, FRAME; call 0xca0820` probe for kzfa), the body gets (out, x, args...) with x = esp right after that
// reservation, and the thunk releases the frame before `ret`.

#[allow(unused_macros)]
macro_rules! kz_thunk0 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "lea eax, [esp + 128 + 0]",
                "push eax",
                "call {f}",
                "add esp, 4",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kz_thunk1 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 160]",
                "lea eax, [esp + 128 + 4]",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kz_thunk2 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 164]",
                "push dword ptr [esp + 164]",
                "lea eax, [esp + 128 + 8]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kz_thunk3 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + 168]",
                "push dword ptr [esp + 168]",
                "push dword ptr [esp + 168]",
                "lea eax, [esp + 128 + 12]",
                "push eax",
                "call {f}",
                "add esp, 16",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "ret",
                f = sym $f,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzf_thunk0 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, {fr}",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "lea eax, [esp + 156 + 0]",
                "push eax",
                "lea eax, [esp + 128 + 4]",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzf_thunk1 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, {fr}",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 160]",
                "lea eax, [esp + 156 + 4]",
                "push eax",
                "lea eax, [esp + 128 + 8]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzf_thunk2 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, {fr}",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 164]",
                "push dword ptr [esp + {fr} + 164]",
                "lea eax, [esp + 156 + 8]",
                "push eax",
                "lea eax, [esp + 128 + 12]",
                "push eax",
                "call {f}",
                "add esp, 16",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzf_thunk3 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "sub esp, {fr}",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 168]",
                "push dword ptr [esp + {fr} + 168]",
                "push dword ptr [esp + {fr} + 168]",
                "lea eax, [esp + 156 + 12]",
                "push eax",
                "lea eax, [esp + 128 + 16]",
                "push eax",
                "call {f}",
                "add esp, 20",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzfa_thunk0 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "mov eax, {fr}",
                "call dword ptr [{alloca}]",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "lea eax, [esp + 156 + 0]",
                "push eax",
                "lea eax, [esp + 128 + 4]",
                "push eax",
                "call {f}",
                "add esp, 8",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
                alloca = sym ALLOCA_PROBE,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzfa_thunk1 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "mov eax, {fr}",
                "call dword ptr [{alloca}]",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 160]",
                "lea eax, [esp + 156 + 4]",
                "push eax",
                "lea eax, [esp + 128 + 8]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
                alloca = sym ALLOCA_PROBE,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzfa_thunk2 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "mov eax, {fr}",
                "call dword ptr [{alloca}]",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 164]",
                "push dword ptr [esp + {fr} + 164]",
                "lea eax, [esp + 156 + 8]",
                "push eax",
                "lea eax, [esp + 128 + 12]",
                "push eax",
                "call {f}",
                "add esp, 16",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
                alloca = sym ALLOCA_PROBE,
            )
        }
    };
}

#[allow(unused_macros)]
macro_rules! kzfa_thunk3 {
    ($name:ident, $f:ident, $fr:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "mov eax, {fr}",
                "call dword ptr [{alloca}]",
                "sub esp, 28",
                "mov [esp], ecx",
                "mov [esp + 4], edx",
                "mov [esp + 8], eax",
                "mov [esp + 12], ebx",
                "mov [esp + 16], esi",
                "mov [esp + 20], edi",
                "mov [esp + 24], ebp",
                "sub esp, 128",
                "movups [esp + 0], xmm0",
                "movups [esp + 16], xmm1",
                "movups [esp + 32], xmm2",
                "movups [esp + 48], xmm3",
                "movups [esp + 64], xmm4",
                "movups [esp + 80], xmm5",
                "movups [esp + 96], xmm6",
                "movups [esp + 112], xmm7",
                "push dword ptr [esp + {fr} + 168]",
                "push dword ptr [esp + {fr} + 168]",
                "push dword ptr [esp + {fr} + 168]",
                "lea eax, [esp + 156 + 12]",
                "push eax",
                "lea eax, [esp + 128 + 16]",
                "push eax",
                "call {f}",
                "add esp, 20",
                "movups xmm0, [esp + 0]",
                "movups xmm1, [esp + 16]",
                "movups xmm2, [esp + 32]",
                "movups xmm3, [esp + 48]",
                "movups xmm4, [esp + 64]",
                "movups xmm5, [esp + 80]",
                "movups xmm6, [esp + 96]",
                "movups xmm7, [esp + 112]",
                "add esp, 128",
                "mov ecx, [esp]",
                "mov edx, [esp + 4]",
                "add esp, 28",
                "add esp, {fr}",
                "ret",
                f = sym $f,
                fr = const $fr,
                alloca = sym ALLOCA_PROBE,
            )
        }
    };
}

#[allow(dead_code)]
static ALLOCA_PROBE: usize = 0x00CA_0820;

/// `call_regs` with EBX and EBP set too (any callee-saved register can be an LTCG callee input); returns the
/// callee's (EAX, ECX, EDX). ESP is restored by scanning for a marker pushed above the arguments (cdecl and ret N).
#[inline(never)]
unsafe fn call_xr(f: usize, eax: u32, ecx: u32, edx: u32, ebx: u32, esi: u32, edi: u32, ebp: u32, args: &[u32]) -> (u32, u32, u32) {
    #[cfg(test)]
    let f = fa(f);
    let mut block: [u32; 12] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, ebx, ebp, 0, 0];
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push eax",
            "push 0x5EC0DE01",
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
            "mov ebp, [ebx + 36]", "mov eax, [ebx + 28]",
            "call 6f",
            "mov esi, esp",
            "4:",
            "cmp dword ptr [esi], 0x5EC0DE01",
            "je 5f",
            "add esi, 4",
            "jmp 4b",
            "5:",
            "lea esp, [esi + 4]",
            "pop ebx",
            "mov [ebx + 40], ecx",
            "mov [ebx + 44], edx",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            "jmp 9f",
            "6:",
            "push dword ptr [ebx]",
            "mov ebx, [ebx + 32]",
            "ret",
            "9:",
            inout("eax") block.as_mut_ptr() => r, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    (r, block[10], block[11])
}

// The offline differential test (module `difftest`) keeps the game's globals (0xD00000..0x350000)
// at a fixed offset, because their addresses are taken in a test process; the Rust ports read
// through these shadows there. Pointers to the test's objects lie outside that range. The
// production build uses the portlib versions.
#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn tr(p: usize) -> usize {
    if (0xD0_0000..0x350_0000).contains(&p) { p + DELTA } else { p }
}
#[cfg(test)]
unsafe fn rd(p: usize) -> u32 {
    unsafe { *(tr(p) as *const u32) }
}
#[cfg(test)]
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(tr(p) as *const u8) }
}
#[cfg(test)]
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(tr(p) as *const f32) }
}
#[cfg(test)]
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(tr(p) as *mut u32) = v }
}
#[cfg(test)]
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(tr(p) as *mut u8) = v }
}
#[cfg(test)]
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(tr(p) as *mut f32) = v }
}
/// Every call into the rest of the game goes through a recording stub in the test.
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(difftest::redirect(f), eax, ecx, edx, esi, edi, args) }
}
/// An absolute game address passed to a callee as a value (the test runs the original on a
/// shifted copy of the game's globals).
#[cfg(test)]
fn ga(p: usize) -> u32 {
    tr(p) as u32
}
#[cfg(not(test))]
fn ga(p: usize) -> u32 {
    p as u32
}
/// The address a call to the game function `f` really goes to.
#[cfg(test)]
fn fa(f: usize) -> usize {
    difftest::redirect(f)
}
#[cfg(not(test))]
fn fa(f: usize) -> usize {
    f
}

/// A double stored in the exe's data (two dwords).
unsafe fn rd64(p: usize) -> f64 {
    f64::from_bits(rd(p) as u64 | (rd(p + 4) as u64) << 32)
}

/// A callee that takes one f32 on the stack (cdecl) and returns on ST0: its result as f64.
#[inline(never)]
unsafe fn call_st0_arg(f: usize, arg: f32) -> f64 {
    let mut out = 0f64;
    std::arch::asm!(
        "push edi",
        "push {a}",
        "call {f}",
        "add esp, 4",
        "pop edi",
        "fstp qword ptr [edi]",
        a = in(reg) arg.to_bits(),
        f = in(reg) fa(f),
        in("edi") &mut out as *mut f64,
        clobber_abi("C"),
    );
    out
}

/// `_ftol2_sse` (0xDA0140): the double goes in on ST0, the truncated int comes back in EAX.
#[inline(never)]
unsafe fn call_ftol(v: f64) -> u32 {
    let r: u32;
    std::arch::asm!(
        "fld qword ptr [{p}]",
        "call {f}",
        p = in(reg) &v as *const f64,
        f = in(reg) fa(0x00DA_0140),
        out("eax") r,
        clobber_abi("C"),
    );
    r
}

/// Timestamp wrap used inline by the game: `t = now (+ 0x94B62E00 when above 1,800,000,000)`.
fn wrap_now(now: u32) -> u32 {
    if (now as i32) > 0x6B49_D200 { now.wrapping_add(0x94B6_2E00) } else { now }
}

macro_rules! hits {
    ($hits:ident, $names:ident, $f:ident, [$($n:expr),* $(,)?]) => {
        static $hits: [AtomicU32; { [$($n),*].len() }] = [const { AtomicU32::new(0) }; { [$($n),*].len() }];
        const $names: [&str; { [$($n),*].len() }] = [$($n),*];
        #[inline(always)]
        fn $f(i: usize) {
            $hits[i].fetch_add(1, Ordering::Relaxed);
        }
    };
}

fn fmt_hits(name: &str, hits: &[AtomicU32], names: &[&str]) -> Option<String> {
    let v: Vec<String> = (0..hits.len())
        .filter_map(|i| {
            let n = hits[i].load(Ordering::Relaxed);
            (n != 0).then(|| format!("{}={n}", names[i]))
        })
        .collect();
    (!v.is_empty()).then(|| format!("{name} ({})", v.join(" ")))
}

/// `fld dword; fstp dword`: a signalling NaN comes out quiet.
fn ldst(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

// =================================================================================================
// 0x5E6DF0  activity_hud_markers_apply  (443 bytes, no arguments)
// =================================================================================================
//
// Called from hud_do_frame. Eight marker records at 0x140A77C (0x1C bytes each; dword at -4 is the
// marker's handle index, +0 / +4 floats, +8 int, +0xC / +0x10 floats, bytes +0x14 / +0x15, byte
// +0x16 "dirty"). A dirty record whose handle (table 0x25277D8, slot = low word < 0x80, objects of
// 0x22C bytes at 0x31A96F0 with the full handle at +0x224) is live pushes its eight values into the
// item with vint_dataitem_set_value (0xB94B60, EAX = &value, stack item / index) and frees each
// temporary value with vint_value_free (0xB93790). Then every dirty flag is cleared.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(AH_HITS, AH_NAMES, ah, ["calls", "dirty", "live", "sets"]);

const AH_RECS: usize = 0x0140_A77C;

/// One value pushed into the item: (type 1 int / 3 float / 5 byte, payload).
unsafe fn ah_set(item: u32, index: u32, ty: u32, val: u32) {
    let mut v = [ty, val];
    call_regs(0x00B9_4B60, v.as_mut_ptr() as u32, 0, 0, 0, 0, &[item, index]);
    call_regs(0x00B9_3790, v.as_mut_ptr() as u32, 0, 0, 0, 0, &[]);
    ah(3);
}

kxs0!(activity_hud_markers_thunk, activity_hud_markers_apply);

extern "C" fn activity_hud_markers_apply() {
    unsafe {
        ah(0);
        let mut m = AH_RECS;
        loop {
            if rb(m + 0x16) != 0 {
                ah(1);
                let id = rd(m - 4);
                let h = rd((id.wrapping_mul(4) as usize).wrapping_add(0x0252_77D8));
                let slot = h & 0xFFFF;
                if h != 0 && slot < 0x80 {
                    let item = 0x031A_96F0 + slot as usize * 0x22C;
                    if rd(item + 0x224) == h {
                        ah(2);
                        let item = ga(item);
                        ah_set(item, 1, 1, id);
                        ah_set(item, 2, 3, ldst(rd(m)));
                        ah_set(item, 3, 3, ldst(rd(m + 4)));
                        ah_set(item, 4, 1, rd(m + 8));
                        ah_set(item, 5, 3, ldst(rd(m + 0xC)));
                        ah_set(item, 6, 3, ldst(rd(m + 0x10)));
                        ah_set(item, 7, 5, rb(m + 0x14) as u32);
                        ah_set(item, 8, 5, rb(m + 0x15) as u32);
                    }
                }
            }
            m += 0x1C;
            if m >= AH_RECS + 0xE0 {
                break;
            }
        }
        let mut f = AH_RECS + 0x16;
        while f < AH_RECS + 0xF6 {
            wb(f, 0);
            f += 0x1C;
        }
    }
}

// =================================================================================================
// 0x804850  net_remote_objects_do_frame  (112 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c. Unless 0x2528BB4 > 0 (signed), and with a local player [0x2528C14] present: for
// each of the ten registration types (0x212E284 + 4*i = head of a circular list of objects linked
// at +0x674; the type's registration object is 0x212E25C + 4*i) and each object whose owner byte
// ([obj+0]) differs from the local player's ([[0x2528C14]+0x408]+0x1B0): calls virtual slot 8
// (remote_do_frame, stdcall, one argument: the handle at +4) of the registration object named by
// the object's own type index (+8). The remote_do_frame bodies stay original.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(NR_HITS, NR_NAMES, nr, ["calls", "types", "objs", "remote", "own"]);

kxs0!(net_remote_objects_thunk, net_remote_objects_do_frame);

extern "C" fn net_remote_objects_do_frame() {
    unsafe {
        nr(0);
        if (rd(0x0252_8BB4) as i32) > 0 {
            return;
        }
        let player = rd(0x0252_8C14) as usize;
        if player == 0 {
            return;
        }
        let ebx = rd(player + 0x408) as usize;
        let mut i = 0usize;
        while i < 0x28 {
            if rd(i + 0x0212_E25C) != 0 {
                let mut esi = rd(i + 0x0212_E284) as usize;
                if esi != 0 {
                    nr(1);
                    loop {
                        nr(2);
                        if rb(esi) != rb(ebx + 0x1B0) {
                            nr(3);
                            let reg = rd((rd(esi + 8) as usize).wrapping_mul(4).wrapping_add(0x0212_E25C)) as usize;
                            let h = rd(esi + 4);
                            let f = rd(rd(reg) as usize + 0x20);
                            call_regs(f as usize, h, reg as u32, f, 0, 0, &[h]);
                        } else {
                            nr(4);
                        }
                        esi = rd(esi + 0x674) as usize;
                        if esi == rd(i + 0x0212_E284) as usize || esi == 0 {
                            break;
                        }
                    }
                }
            }
            i += 4;
        }
    }
}
// =================================================================================================
// 0x8045F0  net_registered_objects_update  (285 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c. With a local player [0x2528C14], walks the pending-registration table at 0x212BA38
// ([0x2528BB0] entries of 8 bytes: u16 key, dword value). Each key is hashed (a shift / xor mixer)
// and looked up in the hash table object [0x212E258] (buckets at +0x5C, count at +0x60, chains
// linked at +4 with the key at +0xC, the registered object pointer at +0). A found object whose
// registration object (0x212E25C + 4 * [obj+8]) answers virtual slot 0x44 (stdcall, handle) with
// AL == 1 is resolved from its handle (0x2149C64 table) and passed to 0x806910 (object, entry
// value, 0); then the entry is removed (the last entry moves into its place) and the same index
// is revisited. Keys with no (or a null) match are removed without a call; an answer other than
// 1 keeps the entry.
// Deviation: a bucket count of 0 (a division fault in the original) is treated as "not found".
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(NU_HITS, NU_NAMES, nu, ["calls", "entries", "found", "ready", "resolved", "removed", "kept"]);

/// The key mixer at 0x804617..0x804645 (all 32-bit wrapping integer arithmetic).
fn nu_hash(key: u32) -> u32 {
    let mut eax = !(key << 15);
    eax = eax.wrapping_add(key);
    let mut ecx = (((eax as i32) >> 10) as u32) ^ eax;
    ecx = ecx.wrapping_add(ecx.wrapping_mul(8));
    ecx ^= ((ecx as i32) >> 6) as u32;
    let edx = !(ecx << 11);
    ecx = ecx.wrapping_add(edx);
    (((ecx as i32) >> 16) as u32) ^ ecx
}

kxs0!(net_registered_objects_thunk, net_registered_objects_update);

extern "C" fn net_registered_objects_update() {
    unsafe {
        nu(0);
        if rd(0x0252_8C14) == 0 {
            return;
        }
        let mut count = rd(0x0252_8BB0) as i32;
        if count <= 0 {
            return;
        }
        let mut i: i32 = 0;
        loop {
            nu(1);
            let ent = (0x0212_BA38isize + (i as isize) * 8) as usize;
            let key = rd(ent) & 0xFFFF;
            let table = rd(0x0212_E258) as usize;
            let div = rd(table + 0x60) as i32;
            let mut keep = false;
            if div != 0 {
                let r = (nu_hash(key) as i32).wrapping_rem(div);
                let mut node = rd((rd(table + 0x5C) as usize).wrapping_add((r as u32 as usize).wrapping_mul(4))) as usize;
                while node != 0 && rd(node + 0xC) != key {
                    node = rd(node + 4) as usize;
                }
                if node != 0 {
                    let obj = rd(node) as usize;
                    if obj != 0 {
                        nu(2);
                        let reg = rd((rd(obj + 8) as usize).wrapping_mul(4).wrapping_add(0x0212_E25C)) as usize;
                        let h = rd(obj + 4);
                        let f = rd(rd(reg) as usize + 0x44);
                        let al = call_regs(f as usize, h, reg as u32, f, 0, 0, &[h]) & 0xFF;
                        if al == 1 {
                            nu(3);
                            let h = rd(obj + 4) as usize;
                            let mut o = 0usize;
                            if h != 0 {
                                o = rd(((h & 0xFFF) << 4) + 0x0214_9C64) as usize;
                                if o != 0 && rd(o + 0x44) as usize != h {
                                    o = 0;
                                }
                            }
                            if o != 0 {
                                nu(4);
                            }
                            let v = rd((i as u32).wrapping_mul(8) as usize + 0x0212_BA3C);
                            call_regs(0x0080_6910, o as u32, v, 0, 0, 0, &[o as u32, v, 0]);
                            count = rd(0x0252_8BB0) as i32;
                        } else {
                            keep = true;
                        }
                    }
                }
            }
            if keep {
                nu(6);
                count = rd(0x0252_8BB0) as i32;
            } else {
                nu(5);
                let last = count.wrapping_sub(1);
                if i != last {
                    let src = (0x0212_BA30isize + (count as isize) * 8) as usize;
                    let (a, b) = (rd(src), rd(src + 4));
                    wd(ent, a);
                    wd(ent + 4, b);
                }
                count = last;
                i = i.wrapping_sub(1);
                wd(0x0252_8BB0, count as u32);
            }
            i = i.wrapping_add(1);
            if !(i < count) {
                break;
            }
        }
    }
}

// =================================================================================================
// 0x56BD60  septic_decals_update  (211 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c. (1) The active decal list: pointer array at 0x140A1F8, count [0x2527538]. Each
// entry is passed to 0x56B310 (cdecl, one pointer); when it answers non-zero the entry's flag
// byte (+0x15) is cleared and the last pointer is moved into its place (count - 1, index
// revisited). (2) The circular list [0x252753C] (next at +4): a node whose time (+0x34, game ms)
// is due, with +0x24 non-zero and bit 2 of +0x18 set, gets the next due time now + 100 (wrapped)
// and is handed to 0x56B7A0 (EAX = node + 0x38, stdcall pointer node + 0x44). (3) 0x569FF0.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(SD_HITS, SD_NAMES, sd, ["calls", "entries", "freed", "nodes", "due", "spawn"]);

kxs0!(septic_decals_thunk, septic_decals_update);

extern "C" fn septic_decals_update() {
    unsafe {
        sd(0);
        let mut i: i32 = 0;
        if (rd(0x0252_7538) as i32) > 0 {
            loop {
                sd(1);
                let p = rd(0x0140_A1F8 + (i as u32 as usize) * 4);
                if call_regs(0x0056_B310, 0, 0, 0, 0, 0, &[p]) & 0xFF != 0 {
                    sd(2);
                    let e = rd(0x0140_A1F8 + (i as u32 as usize) * 4) as usize;
                    wb(e + 0x15, 0);
                    let n = rd(0x0252_7538);
                    let last = rd((n as usize).wrapping_mul(4).wrapping_add(0x0140_A1F4));
                    wd(0x0140_A1F8 + (i as u32 as usize) * 4, last);
                    wd(0x0252_7538, n.wrapping_sub(1));
                    i = i.wrapping_sub(1);
                }
                i = i.wrapping_add(1);
                if !(i < rd(0x0252_7538) as i32) {
                    break;
                }
            }
        }
        let head = rd(0x0252_753C) as usize;
        if head != 0 {
            let now = rd(0x00E8_43AC) as i32;
            let mut n = head;
            loop {
                sd(3);
                let t = rd(n + 0x34) as i32;
                if time_due(t, now) && rb(n + 0x24) != 0 && rb(n + 0x18) & 4 != 0 {
                    sd(4);
                    wd(n + 0x34, time_add(now, 0x64) as u32);
                    call_regs(0x0056_B7A0, (n + 0x38) as u32, 0, 0, 0, 0, &[(n + 0x44) as u32]);
                    sd(5);
                }
                n = rd(n + 4) as usize;
                if n == head || n == 0 {
                    break;
                }
            }
        }
        call_regs(0x0056_9FF0, 0, 0, 0, 0, 0, &[]);
    }
}

// =================================================================================================
// 0xA58EE0  restricted_zones_update  (225 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1b. Skipped when [0x2527D14] and [0x2527CF7] are both set, or [0x2527D15] is set.
// Walks the circular zone list [0x2529DEC] (next at +0x1C) and stops at the first zone for which
// 0xA59240 (EAX = [0x21703D4], EDI = zone +0x24) answers true: that zone's +0x24 becomes the
// "current" value; if [0xE87BE3] == 1, the zone's +0x2C is positive, 0x610580 (ESI = +0x28) answers
// false and the local player's [[0x2528C14]+0x408]+0x3C is 0xD (or there is no player), the
// "setting" is the zone's +0x2C (else -1). A change of the setting calls 0x55FA40 (3, setting, 1)
// and is stored in [0xE87C84]; a change of the current value calls 0xA58900 (value) and is stored
// in [0x2529DF8].
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(RZ_HITS, RZ_NAMES, rz, ["calls", "run", "zone", "setting", "set_changed", "cur_changed"]);

kxs0!(restricted_zones_thunk, restricted_zones_update);

extern "C" fn restricted_zones_update() {
    unsafe {
        rz(0);
        if rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) != 0 {
            return;
        }
        if rb(0x0252_7D15) != 0 {
            return;
        }
        rz(1);
        let player = rd(0x0252_8C14) as usize;
        let local = if player != 0 { rd(player + 0x408) as usize } else { 0 };
        let head = rd(0x0252_9DEC) as usize;
        let mut ebp = 0xFFFF_FFFFu32;
        let mut cur = 0u32;
        let mut ebx = head;
        if head != 0 {
            loop {
                let edi = rd(ebx + 0x24);
                if call_regs(0x00A5_9240, rd(0x0217_03D4), 0, 0, head as u32, edi, &[]) & 0xFF == 0 {
                    ebx = rd(ebx + 0x1C) as usize;
                    if ebx == head || ebx == 0 {
                        break;
                    }
                    continue;
                }
                rz(2);
                cur = edi;
                if rb(0x00E8_7BE3) == 1 && (rd(ebx + 0x2C) as i32) > 0 {
                    let esi = rd(ebx + 0x28);
                    if call_regs(0x0061_0580, 0, 0, 0, esi, edi, &[]) & 0xFF == 0 && (local == 0 || rd(local + 0x3C) == 0xD) {
                        rz(3);
                        ebp = rd(ebx + 0x2C);
                    }
                }
                break;
            }
        }
        if rd(0x00E8_7C84) != ebp {
            rz(4);
            call_regs(0x0055_FA40, 0, 0, 0, 0, 0, &[3, ebp, 1]);
            wd(0x00E8_7C84, ebp);
        }
        if cur != rd(0x0252_9DF8) {
            rz(5);
            call_regs(0x00A5_8900, 0, 0, 0, 0, 0, &[cur]);
            wd(0x0252_9DF8, cur);
        }
    }
}

// =================================================================================================
// 0x9A7080  humans_update_neighborhoods  (207 bytes, one float argument on the stack, cdecl)
// =================================================================================================
//
// sim_frame:1c. Walks the circular human list [0x215E908] (next at +0xB8). For each human the
// neighbourhood index comes from 0x4B19F0 (cdecl: human + 0x14, [human + 0xF00]); -1 maps to
// null, else base [0x2526C94] + index * 0xA8. A human whose +0x4C is 1 and byte +0x3CC is 1 is
// passed to 0x9E1190 (ESI = human, stack: the pointer), which stores it itself; every other
// human gets it stored directly at +0xF00. Afterwards 0x9CFB30 (the dt argument, as the x87
// round trip leaves it), 0x98F680, 0x98DCC0 and 0x986060 run.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(HN_HITS, HN_NAMES, hn, ["calls", "humans", "active", "none", "store"]);

kxs1!(humans_update_neighborhoods_thunk, humans_update_neighborhoods);

extern "C" fn humans_update_neighborhoods(dt: u32) {
    unsafe {
        hn(0);
        let mut esi = rd(0x0215_E908) as usize;
        if esi != 0 {
            let mut base = rd(0x0252_6C94);
            loop {
                hn(1);
                let idx = call_regs(0x004B_19F0, 0, 0, 0, 0, 0, &[(esi + 0x14) as u32, rd(esi + 0xF00)]);
                if rd(esi + 0x4C) == 1 && rb(esi + 0x3CC) == 1 {
                    hn(2);
                    let ptr = if idx == 0xFFFF_FFFF { 0 } else { idx.wrapping_mul(0xA8).wrapping_add(base) };
                    if idx == 0xFFFF_FFFF {
                        hn(3);
                    }
                    call_regs(0x009E_1190, ptr, 0, 0, esi as u32, base, &[ptr]);
                    base = rd(0x0252_6C94);
                } else {
                    let ptr = if idx == 0xFFFF_FFFF { 0 } else { idx.wrapping_mul(0xA8).wrapping_add(base) };
                    hn(4);
                    wd(esi + 0xF00, ptr);
                }
                esi = rd(esi + 0xB8) as usize;
                if esi == rd(0x0215_E908) as usize || esi == 0 {
                    break;
                }
            }
        }
        call_regs(0x009C_FB30, 0, 0, 0, 0, 0, &[ldst(dt)]);
        call_regs(0x0098_F680, 0, 0, 0, 0, 0, &[]);
        call_regs(0x0098_DCC0, 0, 0, 0, 0, 0, &[]);
        call_regs(0x0098_6060, 0, 0, 0, 0, 0, &[]);
    }
}

// =================================================================================================
// 0x793A40  hud_do_frame  (354 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c HUD update. 0x79DA00, 0x79D640, 0x79B9F0, 0x7DF470 first; [0x2528528] takes the
// local player's [+0x10CC]. A pending HUD item handle [0x252858C] (slot = low word < 0x20, 32-byte
// records at 0x31BACF0 with the full handle at +0x14 and a byte at +0x1C that must be clear) is
// released with 0xB94380 (EDX = record). With [0x2527C08] <= 0 the float [0xE84380] goes to
// [0x25285E4] and 0x79AAE0 runs. Then 0x5E5460 and activity_hud_markers_apply (0x5E6DF0). In
// mode [0x2528B48] == 1 (with [0x2528B49] clear, state [0xE96C00] == 0xD, [0x213C444] == 1) the
// widget chain [0x213B4F8] -> +4 -> +4 -> +4 whose first dword is 5 is handed to 0x8C2250
// (stdcall); and with the same mode a live item ([0x25285C4], 0x22C-byte records at 0x31A96F0)
// gets float value 3 = [0x2A4D148] (word) pushed as item value 0 via vint_dataitem_set_value
// (0xB94B60) and freed (0xB93790). Finally a request [0x2528525] == 1 clears the byte +0x460 of
// the 0x57C-byte record (0x319F6B0, index < 12) named by [0x25285CC] (when its +0x4C matches),
// and the request byte is cleared.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(HD_HITS, HD_NAMES, hd, ["calls", "release", "float", "chain", "item", "request", "req_clear"]);

kxs0!(hud_do_frame_thunk, hud_do_frame);

extern "C" fn hud_do_frame() {
    unsafe {
        hd(0);
        call_regs(0x0079_DA00, 0, 0, 0, 0, 0, &[]);
        call_regs(0x0079_D640, 0, 0, 0, 0, 0, &[]);
        call_regs(0x0079_B9F0, 0, 0, 0, 0, 0, &[]);
        call_regs(0x007D_F470, 0, 0, 0, 0, 0, &[]);
        let player = rd(0x0217_03D4) as usize;
        wd(0x0252_8528, rd(player + 0x10CC));
        let h = rd(0x0252_858C);
        if h != 0 {
            let slot = (h & 0xFFFF) as usize;
            if slot < 0x20 {
                let rec = (slot << 5) + 0x031B_ACF0;
                if rd(rec + 0x14) == h && rb(rec + 0x1C) == 0 {
                    hd(1);
                    call_regs(0x00B9_4380, ga(rec), h, ga(rec), 0, 0, &[]);
                }
            }
        }
        if (rd(0x0252_7C08) as i32) <= 0 {
            hd(2);
            wd(0x0252_85E4, ldst(rd(0x00E8_4380)));
            call_regs(0x0079_AAE0, 0, 0, 0, 0, 0, &[]);
        }
        call_regs(0x005E_5460, 0, 0, 0, 0, 0, &[]);
        call_regs(0x005E_6DF0, 0, 0, 0, 0, 0, &[]);
        if rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0 && rd(0x00E9_6C00) == 0xD && rd(0x0213_C444) == 1 {
            let c = rd(0x0213_B4F8) as usize;
            if c != 0 {
                let a = rd(c + 4) as usize;
                if a != 0 {
                    let b = rd(a + 4) as usize;
                    if b != 0 {
                        let d = rd(b + 4) as usize;
                        if rd(d) == 5 {
                            hd(3);
                            call_regs(0x008C_2250, b as u32, c as u32, d as u32, 0, 0, &[c as u32]);
                        }
                    }
                }
            }
        }
        if rb(0x0252_8B48) == 1 {
            let h = rd(0x0252_85C4);
            if h != 0 {
                let slot = (h & 0xFFFF) as usize;
                if slot < 0x80 {
                    let item = slot * 0x22C + 0x031A_96F0;
                    if rd(item + 0x224) == h {
                        hd(4);
                        let v = (rd(0x02A4_D148) & 0xFFFF) as i32 as f32;
                        let mut val = [3u32, v.to_bits()];
                        call_regs(0x00B9_4B60, val.as_mut_ptr() as u32, 0, 0, 0, 0, &[ga(item), 0]);
                        call_regs(0x00B9_3790, val.as_mut_ptr() as u32, 0, 0, 0, 0, &[]);
                    }
                }
            }
        }
        if rb(0x0252_8525) == 1 {
            hd(5);
            let h = rd(0x0252_85CC);
            if h != 0 && (h & 0xFFFF) < 0xC {
                let rec = (h & 0xFFFF) as usize * 0x57C + 0x0319_F6B0;
                if rd(rec + 0x4C) == h {
                    hd(6);
                    wb(rec + 0x460, 0);
                }
            }
            wb(0x0252_8525, 0);
        }
    }
}

// =================================================================================================
// 0x565B30  pushback_phone_call_hud_text  (616 bytes; ECX = pushback object, one stack argument
//           whose low byte is a flag, stdcall ret 4; returns AL)
// =================================================================================================
//
// Starts the pushback phone call / text message for the object in ECX. Returns 0 (nothing done)
// when the object is null, [0x25272CE] is set, or - unless the local player's 0x80FCF0 flag is
// false - the game state is not 3, the failure reason [0x1450380] is 5 / 6, 0x6A74E0 is not -1,
// 0x6045E0 is true, [0xE9E3E4] is neither 0 nor 1, the object's flags (+0x58) have bit 1 or bit 0
// set, +0x4C is non-zero, or its +0x50 is not 1 / 2 / 3. Otherwise: for a networked local player
// (0x80FCF0 true) a 9 / 10 byte packet {0x6C, u16 length - 3, [0x13 at +3, or at +4 when
// [0xE8BC2C]], dword [obj+0x40], byte flag} goes out through 0x814740 (EDI = player); +0x58 bit 0
// is set; with the argument flag, or 0x6045E0 true while [0x2527888] is 0, the text of message id
// [obj+0x44] (0x7F49E0 with ESI / EDI table pointers) is formatted into a 0x80 wide-char stack
// buffer (0xC14C40, ESI = 0x80, EDI = buffer) and shown through 0x79CD40 (EAX = buffer,
// EDI = [0xEAD20C]); else a phone call entry is queued with 0xA66740 (11 arguments: the call id
// found by hashing the message name 0xDE785C / 0xDE7870 through 0xBDCB30 + 0x479A50, the
// object's word +0x84 or 0xFFFF, the float 1.0, [0xDD1B58], the callback 0x562F50, the object)
// and bit 1 of the new entry's byte +0x3D is set. Returns 1 in both paths.
// Deviations: the /GS cookie check (0xC9604F) is not reproduced; the unwritten byte 3 of the
// packet in the [0xE8BC2C] case is 0 instead of stack garbage; a null result of 0xA66740 (the
// original would fault writing +0x3D) is ignored.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(PB_HITS, PB_NAMES, pb, ["calls", "null", "refused", "packet", "text", "phone", "phone_default", "phone_null"]);

/// ECX = object, one stack argument; the original returns with `ret 4`.
#[unsafe(naked)]
extern "C" fn pushback_phone_call_thunk() {
    std::arch::naked_asm!(
        "push edx",
        "push dword ptr [esp + 8]",
        "push ecx",
        "call {f}",
        "add esp, 8",
        "pop edx",
        "ret 4",
        f = sym pushback_phone_call_hud_text,
    )
}

extern "C" fn pushback_phone_call_hud_text(this: u32, arg: u32) -> u32 {
    unsafe {
        pb(0);
        let esi = this as usize;
        let bl = arg as u8;
        if esi == 0 || rb(0x0252_72CE) != 0 {
            pb(1);
            return 0;
        }
        let edi = rd(0x0252_8C14);
        let mut skip = false;
        if edi != 0 && call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[edi]) & 0xFF == 0 {
            skip = true;
        }
        if !skip {
            let refused = rd(0x00E9_6C00) != 3
                || matches!(rd(0x0145_0380), 5 | 6)
                || call_regs(0x006A_74E0, 0, 0, 0, 0, 0, &[]) != 0xFFFF_FFFF
                || call_regs(0x0060_45E0, 0, 0, 0, 0, 0, &[]) & 0xFF != 0
                || !matches!(rd(0x00E9_E3E4), 0 | 1)
                || rb(esi + 0x58) & 2 != 0
                || rd(esi + 0x4C) != 0
                || !matches!(rd(esi + 0x50), 1..=3)
                || rb(esi + 0x58) & 1 != 0;
            if refused {
                pb(2);
                return 0;
            }
        }
        if edi != 0 && call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[edi]) & 0xFF != 0 {
            pb(3);
            let mut buf = [0u8; 16];
            buf[0] = 0x6C;
            let mut p: usize = if rb(0x00E8_BC2C) != 0 { 4 } else { 3 };
            buf[p] = 0x13;
            p += 1;
            buf[p..p + 4].copy_from_slice(&rd(esi + 0x40).to_le_bytes());
            p += 4;
            buf[p] = (bl != 0) as u8;
            p += 1;
            buf[1..3].copy_from_slice(&((p - 3) as u16).to_le_bytes());
            call_regs(0x0081_4740, 0, 0, 0, 0, edi, &[buf.as_ptr() as u32, p as u32]);
        }
        wb(esi + 0x58, rb(esi + 0x58) | 1);
        let c = (call_regs(0x0060_45E0, 0, 0, 0, 0, 0, &[]) & 0xFF != 0 && rd(0x0252_7888) == 0) as u8 | bl;
        if c != 0 {
            pb(4);
            let id = rd(esi + 0x44);
            let s = call_regs(0x007F_49E0, 0, 0, id, ga(0x00DE_75F4), ga(0x00DE_75C8), &[id]);
            let mut w = [0u16; 0x80];
            call_regs(0x00C1_4C40, 0, 0, 0, 0x80, w.as_mut_ptr() as u32, &[s, id]);
            call_regs(0x0079_CD40, w.as_ptr() as u32, 0, 0, 0, rd(0x00EA_D20C), &[]);
            return 1;
        }
        pb(5);
        let word = rd(esi + 0x84) & 0xFFFF;
        let mut tmp = 0u32;
        let name = if word == 0xFFFF { ga(0x00DE_785C) } else { ga(0x00DE_7870) };
        if word == 0xFFFF {
            pb(6);
        }
        let out = call_regs(0x00BD_CB30, 0, name, 0, 0, 0, &[&mut tmp as *mut u32 as u32]);
        let ax = call_regs(0x0047_9A50, 0, 0, out, 0, 0, &[]) & 0xFFFF;
        let f = ldst(rd(0x00DD_1B58));
        let e = call_regs(0x00A6_6740, 0, 0, 0, 0, 0, &[ax, word, 0, 5, 1.0f32.to_bits(), f, 0, 0, 0x0056_2F50, esi as u32, 0]) as usize;
        if e != 0 {
            wb(e + 0x3D, rb(e + 0x3D) | 2);
        } else {
            pb(7);
        }
        1
    }
}

// =================================================================================================
// 0x9A9B60  humans_character_step  (370 bytes; cdecl: float dt, byte flag)
// =================================================================================================
//
// sim_frame:4. Does nothing when dt == 0.0. Otherwise it collects (up to 0x100) the humans of the
// circular list [0x215E908] (next at +0xB8) whose "active" state (+0x4C == 1 and byte +0x3CC == 1)
// equals the flag argument and whose +0xCC has bit 0x200: through the answers of 0x6C6D90,
// 0x6C6D20, 0x9632E0 (EAX = human) and 0x6C6E80 (EDI = human), see the branch layout below. Each
// collected human then goes through 0x736A10 (ECX = [human+0x570], stdcall dt); the whole array
// goes to 0x9A6110 (cdecl array, count, dt) when [0x252A3E8] is non-zero, else every human goes
// through 0x736BC0 (ECX = human, stdcall [human+0x570], dt); finally 0x736CC0 (stdcall
// [human+0x570], human) for each.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(HC_HITS, HC_NAMES, hc, ["calls", "zero_dt", "visited", "collected", "cap", "parallel", "serial"]);

keep_ecx_edx_arg2!(humans_character_step_thunk, humans_character_step);

extern "C" fn humans_character_step(dt: u32, flag: u32) {
    unsafe {
        hc(0);
        if f32::from_bits(dt) == 0.0 {
            hc(1);
            return;
        }
        let want = flag as u8;
        let mut arr = [0u32; 0x100];
        let mut n = 0usize;
        let head = rd(0x0215_E908) as usize;
        if head != 0 {
            let mut edi = head;
            loop {
                hc(2);
                let ecx = rd(edi + 0x4C);
                let active = ecx == 1 && rb(edi + 0x3CC) == 1;
                if active as u8 == want && rd(edi + 0xCC) & 0x200 != 0 {
                    let add = if call_regs(0x006C_6D90, 0, 0, 0, 0, 0, &[]) & 0xFF == 1 {
                        !active && call_regs(0x006C_6E80, 0, 0, 0, 0, edi as u32, &[]) & 0xFF == 0
                    } else if call_regs(0x006C_6D20, 0, 0, 0, 0, 0, &[]) & 0xFF == 0 {
                        true
                    } else if active && call_regs(0x0096_32E0, edi as u32, 0, 0, 0, 0, &[]) & 0xFF == 0 {
                        false
                    } else {
                        call_regs(0x006C_6E80, 0, 0, 0, 0, edi as u32, &[]) & 0xFF == 0
                    };
                    if add {
                        if n < 0x100 {
                            arr[n] = edi as u32;
                            n += 1;
                            hc(3);
                        } else {
                            hc(4);
                            n = 0x100;
                            break;
                        }
                    }
                }
                edi = rd(edi + 0xB8) as usize;
                if edi == rd(0x0215_E908) as usize || edi == 0 {
                    break;
                }
            }
        }
        let fdt = ldst(dt);
        for k in 0..n {
            let h = arr[k] as usize;
            let obj = rd(h + 0x570);
            call_regs(0x0073_6A10, h as u32, obj, 0, 0, 0, &[fdt]);
        }
        if rd(0x0252_A3E8) != 0 {
            hc(5);
            call_regs(0x009A_6110, 0, 0, 0, 0, 0, &[arr.as_ptr() as u32, n as u32, fdt]);
        } else {
            hc(6);
            for k in 0..n {
                let h = arr[k];
                let obj = rd(h as usize + 0x570);
                call_regs(0x0073_6BC0, 0, h, obj, 0, 0, &[obj, fdt]);
            }
        }
        for k in 0..n {
            let h = arr[k];
            let obj = rd(h as usize + 0x570);
            call_regs(0x0073_6CC0, obj, 0, 0, 0, 0, &[obj, h]);
        }
    }
}

// =================================================================================================
// 0x95A3D0  population_update  (247 bytes, no arguments; tail-jumps to 0x959950)
// =================================================================================================
//
// sim_frame:5. Skipped when the local player's [+0x30] is above 1 (the original reads [[0x2528C14] +
// 0x30] without a null check, as here). Walks the circular list [0x215E5C0] (next at +0, the next
// pointer is read before the node's own work) of population groups: each group's slot array
// (+8, count +0x1C) is processed: a slot with a non-zero +0x1C goes to 0x958DC0 (EDI = slot);
// a slot with a zero +0x1C is unlinked from its two neighbours (+0x14 / +0x18), cleared in its
// owner's slot table (owner +0x10, index +8, owner count +0x20 decremented, owner reset by
// 0x9580C0 when it reaches 0) and freed with 0x957FB0 (EAX = slot). When 0x6C6D20 is false the
// group goes to 0x959B50. Finally the function tail-jumps to 0x959950 (its result is returned).
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(PO_HITS, PO_NAMES, po, ["calls", "groups", "slots", "busy", "freed", "owner_empty", "update"]);

kxs0!(population_update_thunk, population_update);

extern "C" fn population_update() -> u32 {
    unsafe {
        po(0);
        let player = rd(0x0252_8C14);
        if (rd(player as usize + 0x30) as i32) > 1 {
            return player;
        }
        let mut ebx = rd(0x0215_E5C0) as usize;
        while ebx != 0 {
            po(1);
            let eax = rd(ebx);
            let next = if eax != rd(0x0215_E5C0) { eax as usize } else { 0 };
            if (rd(ebx + 0x1C) as i32) > 0 {
                let mut slotp = ebx + 8;
                let mut i: i32 = 0;
                loop {
                    po(2);
                    let edi = rd(slotp) as usize;
                    if edi != 0 {
                        if rd(edi + 0x1C) != 0 {
                            po(3);
                            call_regs(0x0095_8DC0, 0, 0, 0, 0, edi as u32, &[]);
                        } else {
                            po(4);
                            let a = rd(edi + 0x14) as usize;
                            if a != 0 {
                                wd(a + 0x18, 0);
                                wd(edi + 0x14, 0);
                            }
                            let a = rd(edi + 0x18) as usize;
                            if a != 0 {
                                wd(a + 0x14, 0);
                                wd(edi + 0x18, 0);
                            }
                            let owner = rd(edi + 0x10) as usize;
                            if owner != 0 {
                                if rb(edi + 0xC) & 1 != 0 {
                                    wd(owner + 0x18, 0);
                                }
                                let idx = rd(edi + 8) as usize;
                                let o = rd(edi + 0x10) as usize;
                                wd(o.wrapping_add(idx.wrapping_mul(4)).wrapping_add(8), 0);
                                let o = rd(edi + 0x10) as usize;
                                wd(o + 0x20, rd(o + 0x20).wrapping_sub(1));
                                let o = rd(edi + 0x10) as usize;
                                if rd(o + 0x20) == 0 {
                                    po(5);
                                    call_regs(0x0095_80C0, o as u32, 0, 0, 0, edi as u32, &[]);
                                }
                            }
                            call_regs(0x0095_7FB0, edi as u32, 0, 0, 0, edi as u32, &[]);
                        }
                    }
                    slotp += 4;
                    i = i.wrapping_add(1);
                    if !(i < rd(ebx + 0x1C) as i32) {
                        break;
                    }
                }
            }
            if call_regs(0x006C_6D20, 0, 0, 0, 0, 0, &[]) & 0xFF == 0 {
                po(6);
                call_regs(0x0095_9B50, 0, 0, 0, 0, 0, &[ebx as u32]);
            }
            ebx = next;
        }
        call_regs(0x0095_9950, 0, 0, 0, 0, 0, &[])
    }
}

//@PORTS@

// --- Install / summary ------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let targets: [(&str, usize, &[u8], usize); 10] = [
        ("activity_hud_markers_apply", 0x005E_6DF0, &AHM_BODY, activity_hud_markers_thunk as extern "C" fn() as usize),
        ("net_remote_objects_do_frame", 0x0080_4850, &NRO_BODY, net_remote_objects_thunk as extern "C" fn() as usize),
        ("net_registered_objects_update", 0x0080_45F0, &NRU_BODY, net_registered_objects_thunk as extern "C" fn() as usize),
        ("septic_decals_update", 0x0056_BD60, &SDU_BODY, septic_decals_thunk as extern "C" fn() as usize),
        ("restricted_zones_update", 0x00A5_8EE0, &RZU_BODY, restricted_zones_thunk as extern "C" fn() as usize),
        ("humans_update_neighborhoods", 0x009A_7080, &HNU_BODY, humans_update_neighborhoods_thunk as extern "C" fn() as usize),
        ("hud_do_frame", 0x0079_3A40, &HDF_BODY, hud_do_frame_thunk as extern "C" fn() as usize),
        ("pushback_phone_call_hud_text", 0x0056_5B30, &PBP_BODY, pushback_phone_call_thunk as extern "C" fn() as usize),
        ("humans_character_step", 0x009A_9B60, &HCS_BODY, humans_character_step_thunk as extern "C" fn() as usize),
        ("population_update", 0x0095_A3D0, &POP_BODY, population_update_thunk as extern "C" fn() as usize),
//@TARGETS@
    ];
    let mut ok = 0;
    for (name, addr, body, thunk) in targets {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    crate::log(&format!("port_frame4: replaced {ok}/{} with Rust", targets.len()));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = [
        fmt_hits("activity_hud_markers_apply", &AH_HITS, &AH_NAMES),
        fmt_hits("net_remote_objects_do_frame", &NR_HITS, &NR_NAMES),
        fmt_hits("net_registered_objects_update", &NU_HITS, &NU_NAMES),
        fmt_hits("septic_decals_update", &SD_HITS, &SD_NAMES),
        fmt_hits("restricted_zones_update", &RZ_HITS, &RZ_NAMES),
        fmt_hits("humans_update_neighborhoods", &HN_HITS, &HN_NAMES),
        fmt_hits("hud_do_frame", &HD_HITS, &HD_NAMES),
        fmt_hits("pushback_phone_call_hud_text", &PB_HITS, &PB_NAMES),
        fmt_hits("humans_character_step", &HC_HITS, &HC_NAMES),
        fmt_hits("population_update", &PO_HITS, &PO_NAMES),
//@SUMMARY@
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_frame4: {}", parts.join(", "))
}

//@INSTALL@

#[allow(dead_code)]
const AHM_BODY: [u8; 443] = [
    0x81, 0xEC, 0x84, 0x00, 0x00, 0x00, 0x53, 0x55, 0x56, 0x33, 0xDB, 0x57, 0xBF, 0x7C, 0xA7, 0x40,
    0x01, 0x8D, 0x6B, 0x03, 0x38, 0x5F, 0x16, 0x0F, 0x84, 0x73, 0x01, 0x00, 0x00, 0x8B, 0x4F, 0xFC,
    0x8B, 0x04, 0x8D, 0xD8, 0x77, 0x52, 0x02, 0x3B, 0xC3, 0x0F, 0x86, 0x61, 0x01, 0x00, 0x00, 0x0F,
    0xB7, 0xF0, 0x81, 0xFE, 0x80, 0x00, 0x00, 0x00, 0x0F, 0x83, 0x52, 0x01, 0x00, 0x00, 0x69, 0xF6,
    0x2C, 0x02, 0x00, 0x00, 0x81, 0xC6, 0xF0, 0x96, 0x1A, 0x03, 0x39, 0x86, 0x24, 0x02, 0x00, 0x00,
    0x0F, 0x85, 0x3A, 0x01, 0x00, 0x00, 0x6A, 0x01, 0x56, 0x8D, 0x44, 0x24, 0x18, 0xC7, 0x44, 0x24,
    0x18, 0x01, 0x00, 0x00, 0x00, 0x89, 0x4C, 0x24, 0x1C, 0xE8, 0x02, 0xDD, 0x5A, 0x00, 0x8D, 0x44,
    0x24, 0x10, 0xE8, 0x29, 0xC9, 0x5A, 0x00, 0xD9, 0x07, 0x6A, 0x02, 0xD9, 0x5C, 0x24, 0x28, 0x56,
    0x8D, 0x44, 0x24, 0x28, 0x89, 0x5C, 0x24, 0x18, 0x89, 0x6C, 0x24, 0x28, 0xE8, 0xDF, 0xDC, 0x5A,
    0x00, 0x8D, 0x44, 0x24, 0x20, 0xE8, 0x06, 0xC9, 0x5A, 0x00, 0xD9, 0x47, 0x04, 0x55, 0xD9, 0x5C,
    0x24, 0x38, 0x56, 0x8D, 0x44, 0x24, 0x38, 0x89, 0x5C, 0x24, 0x28, 0x89, 0x6C, 0x24, 0x38, 0xE8,
    0xBC, 0xDC, 0x5A, 0x00, 0x8D, 0x44, 0x24, 0x30, 0xE8, 0xE3, 0xC8, 0x5A, 0x00, 0x8B, 0x47, 0x08,
    0x6A, 0x04, 0x89, 0x44, 0x24, 0x48, 0x56, 0x8D, 0x44, 0x24, 0x48, 0x89, 0x5C, 0x24, 0x38, 0xC7,
    0x44, 0x24, 0x48, 0x01, 0x00, 0x00, 0x00, 0xE8, 0x94, 0xDC, 0x5A, 0x00, 0x8D, 0x44, 0x24, 0x40,
    0xE8, 0xBB, 0xC8, 0x5A, 0x00, 0xD9, 0x47, 0x0C, 0x6A, 0x05, 0xD9, 0x5C, 0x24, 0x58, 0x56, 0x8D,
    0x44, 0x24, 0x58, 0x89, 0x5C, 0x24, 0x48, 0x89, 0x6C, 0x24, 0x58, 0xE8, 0x70, 0xDC, 0x5A, 0x00,
    0x8D, 0x44, 0x24, 0x50, 0xE8, 0x97, 0xC8, 0x5A, 0x00, 0xD9, 0x47, 0x10, 0x6A, 0x06, 0xD9, 0x5C,
    0x24, 0x68, 0x56, 0x8D, 0x44, 0x24, 0x68, 0x89, 0x5C, 0x24, 0x58, 0x89, 0x6C, 0x24, 0x68, 0xE8,
    0x4C, 0xDC, 0x5A, 0x00, 0x8D, 0x44, 0x24, 0x60, 0xE8, 0x73, 0xC8, 0x5A, 0x00, 0x8A, 0x4F, 0x14,
    0x6A, 0x07, 0x56, 0x8D, 0x44, 0x24, 0x78, 0x89, 0x5C, 0x24, 0x68, 0xC7, 0x44, 0x24, 0x78, 0x05,
    0x00, 0x00, 0x00, 0x88, 0x4C, 0x24, 0x7C, 0xE8, 0x24, 0xDC, 0x5A, 0x00, 0x8D, 0x44, 0x24, 0x70,
    0xE8, 0x4B, 0xC8, 0x5A, 0x00, 0x8A, 0x57, 0x15, 0x6A, 0x08, 0x56, 0x8D, 0x84, 0x24, 0x88, 0x00,
    0x00, 0x00, 0x89, 0x5C, 0x24, 0x78, 0xC7, 0x84, 0x24, 0x88, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00,
    0x00, 0x88, 0x94, 0x24, 0x8C, 0x00, 0x00, 0x00, 0xE8, 0xF3, 0xDB, 0x5A, 0x00, 0x8D, 0x84, 0x24,
    0x80, 0x00, 0x00, 0x00, 0xE8, 0x17, 0xC8, 0x5A, 0x00, 0x89, 0x9C, 0x24, 0x80, 0x00, 0x00, 0x00,
    0x83, 0xC7, 0x1C, 0x81, 0xFF, 0x5C, 0xA8, 0x40, 0x01, 0x0F, 0x8C, 0x75, 0xFE, 0xFF, 0xFF, 0xB8,
    0x92, 0xA7, 0x40, 0x01, 0x88, 0x18, 0x83, 0xC0, 0x1C, 0x3D, 0x72, 0xA8, 0x40, 0x01, 0x7C, 0xF4,
    0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0x84, 0x00, 0x00, 0x00, 0xC3,
];
#[allow(dead_code)]
const AHM_BODY_CALLS: [usize; 16] = [0x69, 0x72, 0x8c, 0x95, 0xaf, 0xb8, 0xd7, 0xe0, 0xfb, 0x104, 0x11f, 0x128, 0x147, 0x150, 0x178, 0x184];
#[allow(dead_code)]
const AHM_BODY_DISPS: [usize; 6] = [0xd, 0x23, 0x46, 0x195, 0x1a0, 0x1aa];

#[allow(dead_code)]
const NRO_BODY: [u8; 112] = [
    0x83, 0x3D, 0xB4, 0x8B, 0x52, 0x02, 0x00, 0x7F, 0x66, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0,
    0x74, 0x5D, 0x53, 0x8B, 0x98, 0x08, 0x04, 0x00, 0x00, 0x56, 0x57, 0x33, 0xFF, 0x8D, 0x49, 0x00,
    0x83, 0xBF, 0x5C, 0xE2, 0x12, 0x02, 0x00, 0x74, 0x3B, 0x8B, 0xB7, 0x84, 0xE2, 0x12, 0x02, 0x85,
    0xF6, 0x74, 0x31, 0x8A, 0x06, 0x3A, 0x83, 0xB0, 0x01, 0x00, 0x00, 0x74, 0x15, 0x8B, 0x4E, 0x08,
    0x8B, 0x0C, 0x8D, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11, 0x8B, 0x46, 0x04, 0x8B, 0x52, 0x20, 0x50,
    0xFF, 0xD2, 0x8B, 0xB6, 0x74, 0x06, 0x00, 0x00, 0x3B, 0xB7, 0x84, 0xE2, 0x12, 0x02, 0x74, 0x04,
    0x85, 0xF6, 0x75, 0xCF, 0x83, 0xC7, 0x04, 0x83, 0xFF, 0x28, 0x72, 0xB4, 0x5F, 0x5E, 0x5B, 0xC3,
];
#[allow(dead_code)]
const NRO_BODY_CALLS: [usize; 0] = [];
#[allow(dead_code)]
const NRO_BODY_DISPS: [usize; 6] = [0x2, 0xa, 0x22, 0x2b, 0x43, 0x5a];
#[allow(dead_code)]
const NRU_BODY: [u8; 285] = [
    0x51, 0x83, 0x3D, 0x14, 0x8C, 0x52, 0x02, 0x00, 0x53, 0x55, 0x56, 0x57, 0x0F, 0x84, 0x05, 0x01,
    0x00, 0x00, 0x8B, 0x3D, 0xB0, 0x8B, 0x52, 0x02, 0x33, 0xDB, 0x85, 0xFF, 0x0F, 0x8E, 0xF5, 0x00,
    0x00, 0x00, 0xBD, 0x38, 0xBA, 0x12, 0x02, 0x0F, 0xB7, 0x75, 0x00, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F,
    0xF7, 0xD0, 0x03, 0xC6, 0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1,
    0xC1, 0xF8, 0x06, 0x33, 0xC8, 0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1,
    0xC1, 0xF8, 0x10, 0x33, 0xC1, 0x8B, 0x0D, 0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B,
    0x41, 0x5C, 0x8B, 0x14, 0x90, 0x85, 0xD2, 0x74, 0x6F, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x3B, 0x72, 0x0C, 0x74, 0x09, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75, 0xF4, 0xEB, 0x5A, 0x8B, 0x32,
    0x85, 0xF6, 0x74, 0x54, 0x8B, 0x4E, 0x08, 0x8B, 0x0C, 0x8D, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11,
    0x8B, 0x46, 0x04, 0x8B, 0x52, 0x44, 0x50, 0xFF, 0xD2, 0x3C, 0x01, 0x75, 0x66, 0x8B, 0x76, 0x04,
    0x85, 0xF6, 0x74, 0x19, 0x8B, 0xC6, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x70, 0x44, 0x74, 0x02, 0x33, 0xC0, 0x8B,
    0x0C, 0xDD, 0x3C, 0xBA, 0x12, 0x02, 0x6A, 0x00, 0x51, 0x50, 0xE8, 0x51, 0x22, 0x00, 0x00, 0x8B,
    0x3D, 0xB0, 0x8B, 0x52, 0x02, 0x83, 0xC4, 0x0C, 0x8D, 0x47, 0xFF, 0x3B, 0xD8, 0x74, 0x14, 0x8B,
    0x14, 0xFD, 0x30, 0xBA, 0x12, 0x02, 0x89, 0x55, 0x00, 0x8B, 0x0C, 0xFD, 0x34, 0xBA, 0x12, 0x02,
    0x89, 0x4D, 0x04, 0x8B, 0xF8, 0x83, 0xEB, 0x01, 0x89, 0x3D, 0xB0, 0x8B, 0x52, 0x02, 0x83, 0xED,
    0x08, 0xEB, 0x06, 0x8B, 0x3D, 0xB0, 0x8B, 0x52, 0x02, 0x83, 0xC3, 0x01, 0x83, 0xC5, 0x08, 0x3B,
    0xDF, 0x0F, 0x8C, 0x10, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x59, 0xC3,
];
#[allow(dead_code)]
const NRU_BODY_CALLS: [usize; 1] = [0xca];
#[allow(dead_code)]
const NRU_BODY_DISPS: [usize; 12] = [0x3, 0x14, 0x23, 0x57, 0x8a, 0xb0, 0xc2, 0xd1, 0xe2, 0xec, 0xfa, 0x105];

#[allow(dead_code)]
const SDU_BODY: [u8; 211] = [
    0x51, 0x56, 0x33, 0xF6, 0x39, 0x35, 0x38, 0x75, 0x52, 0x02, 0x57, 0x7E, 0x4B, 0x8D, 0x49, 0x00,
    0x8B, 0x04, 0xB5, 0xF8, 0xA1, 0x40, 0x01, 0x50, 0xE8, 0x93, 0xF5, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0x84, 0xC0, 0x74, 0x29, 0x8B, 0x04, 0xB5, 0xF8, 0xA1, 0x40, 0x01, 0xC6, 0x40, 0x15, 0x00, 0xA1,
    0x38, 0x75, 0x52, 0x02, 0x8B, 0x0C, 0x85, 0xF4, 0xA1, 0x40, 0x01, 0x83, 0xE8, 0x01, 0x89, 0x0C,
    0xB5, 0xF8, 0xA1, 0x40, 0x01, 0xA3, 0x38, 0x75, 0x52, 0x02, 0x83, 0xEE, 0x01, 0x83, 0xC6, 0x01,
    0x3B, 0x35, 0x38, 0x75, 0x52, 0x02, 0x7C, 0xB8, 0x8B, 0x3D, 0x3C, 0x75, 0x52, 0x02, 0x85, 0xFF,
    0x8B, 0xD7, 0x74, 0x66, 0x8B, 0x35, 0xAC, 0x43, 0xE8, 0x00, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x42, 0x34, 0x85, 0xC0, 0x7C, 0x48, 0x3B, 0xC6, 0x7E, 0x0B, 0x2B, 0xC6, 0x3D, 0x00, 0xE9,
    0xA4, 0x35, 0x7C, 0x3B, 0xEB, 0x0C, 0x8B, 0xCE, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35,
    0x7D, 0x2D, 0x80, 0x7A, 0x24, 0x00, 0x74, 0x27, 0xF6, 0x42, 0x18, 0x04, 0x74, 0x21, 0x8D, 0x46,
    0x64, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x42, 0x34, 0x7E, 0x08, 0x05, 0x00, 0x2E, 0xB6, 0x94,
    0x89, 0x42, 0x34, 0x8D, 0x42, 0x44, 0x50, 0x8D, 0x42, 0x38, 0xE8, 0x81, 0xF9, 0xFF, 0xFF, 0x8B,
    0x52, 0x04, 0x3B, 0xD7, 0x74, 0x04, 0x85, 0xD2, 0x75, 0xA6, 0xE8, 0xC1, 0xE1, 0xFF, 0xFF, 0x5F,
    0x5E, 0x59, 0xC3,
];
#[allow(dead_code)]
const SDU_BODY_CALLS: [usize; 3] = [0x18, 0xba, 0xca];
#[allow(dead_code)]
const SDU_BODY_DISPS: [usize; 10] = [0x6, 0x13, 0x27, 0x30, 0x37, 0x41, 0x46, 0x52, 0x5a, 0x66];

#[allow(dead_code)]
const RZU_BODY: [u8; 225] = [
    0x83, 0xEC, 0x0C, 0x53, 0x55, 0x33, 0xC9, 0x38, 0x0D, 0x14, 0x7D, 0x52, 0x02, 0x56, 0x57, 0x89,
    0x4C, 0x24, 0x18, 0x74, 0x0C, 0x38, 0x0D, 0xF7, 0x7C, 0x52, 0x02, 0x0F, 0x85, 0xB8, 0x00, 0x00,
    0x00, 0x38, 0x0D, 0x15, 0x7D, 0x52, 0x02, 0x0F, 0x85, 0xAC, 0x00, 0x00, 0x00, 0xA1, 0x14, 0x8C,
    0x52, 0x02, 0x3B, 0xC1, 0x89, 0x4C, 0x24, 0x14, 0x74, 0x0A, 0x8B, 0x80, 0x08, 0x04, 0x00, 0x00,
    0x89, 0x44, 0x24, 0x14, 0x8B, 0x35, 0xEC, 0x9D, 0x52, 0x02, 0x83, 0xCD, 0xFF, 0x3B, 0xF1, 0x8B,
    0xDE, 0x74, 0x50, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x7B, 0x24, 0xE8, 0x00, 0x03, 0x00, 0x00,
    0x84, 0xC0, 0x75, 0x0D, 0x8B, 0x5B, 0x1C, 0x3B, 0xDE, 0x74, 0x38, 0x85, 0xDB, 0x75, 0xE4, 0xEB,
    0x32, 0x80, 0x3D, 0xE3, 0x7B, 0xE8, 0x00, 0x01, 0x8B, 0xCF, 0x89, 0x4C, 0x24, 0x18, 0x75, 0x23,
    0x83, 0x7B, 0x2C, 0x00, 0x7E, 0x1D, 0x8B, 0x73, 0x28, 0xE8, 0x12, 0x76, 0xBB, 0xFF, 0x84, 0xC0,
    0x75, 0x11, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0, 0x74, 0x06, 0x83, 0x78, 0x3C, 0x0D, 0x75, 0x03,
    0x8B, 0x6B, 0x2C, 0x39, 0x2D, 0x84, 0x7C, 0xE8, 0x00, 0x74, 0x13, 0x6A, 0x01, 0x55, 0x6A, 0x03,
    0xE8, 0xAB, 0x6A, 0xB0, 0xFF, 0x83, 0xC4, 0x0C, 0x89, 0x2D, 0x84, 0x7C, 0xE8, 0x00, 0x8B, 0x74,
    0x24, 0x18, 0x3B, 0x35, 0xF8, 0x9D, 0x52, 0x02, 0x74, 0x0F, 0x56, 0xE8, 0x50, 0xF9, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0x89, 0x35, 0xF8, 0x9D, 0x52, 0x02, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C,
    0xC3,
];
#[allow(dead_code)]
const RZU_BODY_CALLS: [usize; 4] = [0x5b, 0x89, 0xb0, 0xcb];
#[allow(dead_code)]
const RZU_BODY_DISPS: [usize; 11] = [0x9, 0x17, 0x23, 0x2e, 0x46, 0x54, 0x73, 0xa5, 0xba, 0xc4, 0xd5];

#[allow(dead_code)]
const HNU_BODY: [u8; 207] = [
    0x51, 0x53, 0x56, 0x8B, 0x35, 0x08, 0xE9, 0x15, 0x02, 0x85, 0xF6, 0x57, 0x0F, 0x84, 0x99, 0x00,
    0x00, 0x00, 0x8B, 0x3D, 0x94, 0x6C, 0x52, 0x02, 0xBB, 0x01, 0x00, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x39, 0x5E, 0x4C, 0x75, 0x46, 0x38, 0x9E, 0xCC, 0x03, 0x00, 0x00, 0x75, 0x3E, 0x8B, 0x86, 0x00,
    0x0F, 0x00, 0x00, 0x50, 0x8D, 0x4E, 0x14, 0x51, 0xE8, 0x33, 0xA9, 0xB0, 0xFF, 0x83, 0xC4, 0x08,
    0x83, 0xF8, 0xFF, 0x75, 0x10, 0x33, 0xC0, 0x50, 0xE8, 0xC3, 0xA0, 0x03, 0x00, 0x8B, 0x3D, 0x94,
    0x6C, 0x52, 0x02, 0xEB, 0x40, 0x69, 0xC0, 0xA8, 0x00, 0x00, 0x00, 0x03, 0xC7, 0x50, 0xE8, 0xAD,
    0xA0, 0x03, 0x00, 0x8B, 0x3D, 0x94, 0x6C, 0x52, 0x02, 0xEB, 0x2A, 0x8B, 0x96, 0x00, 0x0F, 0x00,
    0x00, 0x52, 0x8D, 0x46, 0x14, 0x50, 0xE8, 0xF5, 0xA8, 0xB0, 0xFF, 0x83, 0xC4, 0x08, 0x83, 0xF8,
    0xFF, 0x75, 0x04, 0x33, 0xC0, 0xEB, 0x08, 0x69, 0xC0, 0xA8, 0x00, 0x00, 0x00, 0x03, 0xC7, 0x89,
    0x86, 0x00, 0x0F, 0x00, 0x00, 0x8B, 0xB6, 0xB8, 0x00, 0x00, 0x00, 0x3B, 0x35, 0x08, 0xE9, 0x15,
    0x02, 0x74, 0x08, 0x85, 0xF6, 0x0F, 0x85, 0x75, 0xFF, 0xFF, 0xFF, 0xD9, 0x44, 0x24, 0x14, 0x51,
    0xD9, 0x1C, 0x24, 0xE8, 0xF8, 0x89, 0x02, 0x00, 0x83, 0xC4, 0x04, 0xE8, 0x40, 0x85, 0xFE, 0xFF,
    0xE8, 0x7B, 0x6B, 0xFE, 0xFF, 0xE8, 0x16, 0xEF, 0xFD, 0xFF, 0x5F, 0x5E, 0x5B, 0x59, 0xC3,
];
#[allow(dead_code)]
const HNU_BODY_CALLS: [usize; 8] = [0x38, 0x48, 0x5e, 0x76, 0xb3, 0xbb, 0xc0, 0xc5];
#[allow(dead_code)]
const HNU_BODY_DISPS: [usize; 5] = [0x5, 0x14, 0x4f, 0x65, 0x9d];

#[allow(dead_code)]
const HDF_BODY: [u8; 354] = [
    0x83, 0xEC, 0x1C, 0xE8, 0xB8, 0x9F, 0x00, 0x00, 0xE8, 0xF3, 0x9B, 0x00, 0x00, 0xE8, 0x9E, 0x7F,
    0x00, 0x00, 0xE8, 0x19, 0xBA, 0x04, 0x00, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x88, 0xCC, 0x10,
    0x00, 0x00, 0x89, 0x0D, 0x28, 0x85, 0x52, 0x02, 0x8B, 0x0D, 0x8C, 0x85, 0x52, 0x02, 0x85, 0xC9,
    0x74, 0x22, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x20, 0x73, 0x1A, 0xC1, 0xE0, 0x05, 0x05, 0xF0, 0xAC,
    0x1B, 0x03, 0x39, 0x48, 0x14, 0x75, 0x0D, 0x80, 0x78, 0x1C, 0x00, 0x75, 0x07, 0x8B, 0xD0, 0xE8,
    0xEC, 0x08, 0x40, 0x00, 0x83, 0x3D, 0x08, 0x7C, 0x52, 0x02, 0x00, 0x7F, 0x11, 0xD9, 0x05, 0x80,
    0x43, 0xE8, 0x00, 0xD9, 0x1D, 0xE4, 0x85, 0x52, 0x02, 0xE8, 0x32, 0x70, 0x00, 0x00, 0xE8, 0xAD,
    0x19, 0xE5, 0xFF, 0xE8, 0x38, 0x33, 0xE5, 0xFF, 0x8A, 0x15, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xFA,
    0x01, 0x75, 0x41, 0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x75, 0x38, 0x83, 0x3D, 0x00, 0x6C,
    0xE9, 0x00, 0x0D, 0x75, 0x2F, 0x83, 0x3D, 0x44, 0xC4, 0x13, 0x02, 0x01, 0x75, 0x26, 0x8B, 0x0D,
    0xF8, 0xB4, 0x13, 0x02, 0x85, 0xC9, 0x74, 0x1C, 0x8B, 0x41, 0x04, 0x85, 0xC0, 0x74, 0x15, 0x8B,
    0x40, 0x04, 0x85, 0xC0, 0x74, 0x0E, 0x8B, 0x50, 0x04, 0x83, 0x3A, 0x05, 0x75, 0x06, 0x51, 0xE8,
    0x4C, 0xE7, 0x12, 0x00, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x57, 0x8B, 0x0D, 0xC4,
    0x85, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x4D, 0x0F, 0xB7, 0xC1, 0x3D, 0x80, 0x00, 0x00, 0x00, 0x73,
    0x43, 0x69, 0xC0, 0x2C, 0x02, 0x00, 0x00, 0x05, 0xF0, 0x96, 0x1A, 0x03, 0x39, 0x88, 0x24, 0x02,
    0x00, 0x00, 0x75, 0x30, 0x0F, 0xB7, 0x0D, 0x48, 0xD1, 0xA4, 0x02, 0x89, 0x4C, 0x24, 0x04, 0x6A,
    0x00, 0x50, 0xDB, 0x44, 0x24, 0x0C, 0x8D, 0x44, 0x24, 0x10, 0xC7, 0x44, 0x24, 0x10, 0x03, 0x00,
    0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xE8, 0x05, 0x10, 0x40, 0x00, 0x8D, 0x44, 0x24, 0x08, 0xE8,
    0x2C, 0xFC, 0x3F, 0x00, 0x80, 0x3D, 0x25, 0x85, 0x52, 0x02, 0x01, 0x75, 0x31, 0x8B, 0x0D, 0xCC,
    0x85, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x20, 0x66, 0x83, 0xF9, 0x0C, 0x73, 0x1A, 0x0F, 0xB7, 0xC1,
    0x69, 0xC0, 0x7C, 0x05, 0x00, 0x00, 0x05, 0xB0, 0xF6, 0x19, 0x03, 0x39, 0x48, 0x4C, 0x75, 0x07,
    0xC6, 0x80, 0x60, 0x04, 0x00, 0x00, 0x00, 0xC6, 0x05, 0x25, 0x85, 0x52, 0x02, 0x00, 0x83, 0xC4,
    0x1C, 0xC3,
];
#[allow(dead_code)]
const HDF_BODY_CALLS: [usize; 11] = [0x3, 0x8, 0xd, 0x12, 0x4f, 0x69, 0x6e, 0x73, 0xbf, 0x116, 0x11f];
#[allow(dead_code)]
const HDF_BODY_DISPS: [usize; 20] = [0x18, 0x24, 0x2a, 0x3e, 0x56, 0x5f, 0x65, 0x7a, 0x84, 0x8e, 0x97, 0xa0, 0xc5, 0xcf, 0xe8, 0xf7, 0x126, 0x12f, 0x147, 0x159];

#[allow(dead_code)]
const PBP_BODY: [u8; 616] = [
    0x81, 0xEC, 0x2C, 0x06, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x28, 0x06, 0x00, 0x00, 0x53, 0x8A, 0x9C, 0x24, 0x34, 0x06, 0x00, 0x00, 0x56, 0x8B, 0xF1, 0x85,
    0xF6, 0x57, 0x0F, 0x84, 0x24, 0x02, 0x00, 0x00, 0x80, 0x3D, 0xCE, 0x72, 0x52, 0x02, 0x00, 0x0F,
    0x85, 0x17, 0x02, 0x00, 0x00, 0x8B, 0x3D, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xFF, 0x74, 0x0E, 0x57,
    0xE8, 0x7B, 0xA1, 0x2A, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0x86, 0x00, 0x00, 0x00, 0x83, 0x3D, 0x00,
    0x6C, 0xE9, 0x00, 0x03, 0x0F, 0x85, 0xF2, 0x01, 0x00, 0x00, 0xA1, 0x80, 0x03, 0x45, 0x01, 0x83,
    0xF8, 0x05, 0x0F, 0x84, 0xE4, 0x01, 0x00, 0x00, 0x83, 0xF8, 0x06, 0x0F, 0x84, 0xDB, 0x01, 0x00,
    0x00, 0xE8, 0x3A, 0x19, 0x14, 0x00, 0x83, 0xF8, 0xFF, 0x0F, 0x85, 0xCD, 0x01, 0x00, 0x00, 0xE8,
    0x2C, 0xEA, 0x09, 0x00, 0x84, 0xC0, 0x0F, 0x85, 0xC0, 0x01, 0x00, 0x00, 0xA1, 0xE4, 0xE3, 0xE9,
    0x00, 0x85, 0xC0, 0x74, 0x09, 0x83, 0xF8, 0x01, 0x0F, 0x85, 0xAE, 0x01, 0x00, 0x00, 0x8A, 0x4E,
    0x58, 0xF6, 0xC1, 0x02, 0x0F, 0x85, 0xA2, 0x01, 0x00, 0x00, 0x83, 0x7E, 0x4C, 0x00, 0x0F, 0x85,
    0x98, 0x01, 0x00, 0x00, 0x8B, 0x46, 0x50, 0x83, 0xF8, 0x01, 0x74, 0x0E, 0x83, 0xF8, 0x02, 0x74,
    0x09, 0x83, 0xF8, 0x03, 0x0F, 0x85, 0x82, 0x01, 0x00, 0x00, 0xF6, 0xC1, 0x01, 0x0F, 0x85, 0x79,
    0x01, 0x00, 0x00, 0x85, 0xFF, 0x74, 0x74, 0x57, 0xE8, 0xE3, 0xA0, 0x2A, 0x00, 0x84, 0xC0, 0x74,
    0x6A, 0x80, 0x3D, 0x2C, 0xBC, 0xE8, 0x00, 0x00, 0xC6, 0x84, 0x24, 0x18, 0x01, 0x00, 0x00, 0x6C,
    0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x04, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0xC8, 0x83,
    0xC0, 0x01, 0x0F, 0xB7, 0xD0, 0xC6, 0x84, 0x0C, 0x18, 0x01, 0x00, 0x00, 0x13, 0x8B, 0x4E, 0x40,
    0x89, 0x8C, 0x14, 0x18, 0x01, 0x00, 0x00, 0x83, 0xC0, 0x04, 0x0F, 0xB7, 0xC8, 0x84, 0xDB, 0x0F,
    0x95, 0xC2, 0x88, 0x94, 0x0C, 0x18, 0x01, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x8D, 0x50, 0xFD, 0x0F,
    0xB7, 0xC0, 0x50, 0x8D, 0x8C, 0x24, 0x1C, 0x01, 0x00, 0x00, 0x51, 0x66, 0x89, 0x94, 0x24, 0x21,
    0x01, 0x00, 0x00, 0xE8, 0xC8, 0xEA, 0x2A, 0x00, 0x83, 0xC4, 0x08, 0x80, 0x4E, 0x58, 0x01, 0xE8,
    0x5C, 0xE9, 0x09, 0x00, 0x84, 0xC0, 0x74, 0x10, 0x83, 0x3D, 0x88, 0x78, 0x52, 0x02, 0x00, 0x75,
    0x07, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xEB, 0x02, 0x33, 0xC0, 0x0A, 0xC3, 0x0F, 0x85, 0xA2, 0x00,
    0x00, 0x00, 0x66, 0x81, 0xBE, 0x84, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x75, 0x41, 0x8D, 0x54, 0x24,
    0x14, 0x52, 0x33, 0xC0, 0xB9, 0x5C, 0x78, 0xDE, 0x00, 0xE8, 0x72, 0x6E, 0x67, 0x00, 0x8B, 0xD0,
    0xE8, 0x8B, 0x3D, 0xF1, 0xFF, 0xD9, 0x05, 0x58, 0x1B, 0xDD, 0x00, 0x6A, 0x00, 0x56, 0x68, 0x50,
    0x2F, 0x56, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x83, 0xEC, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0xE8,
    0xD9, 0x1C, 0x24, 0x6A, 0x05, 0x6A, 0x00, 0x68, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x42, 0x8D, 0x44,
    0x24, 0x10, 0x50, 0x33, 0xC0, 0xB9, 0x70, 0x78, 0xDE, 0x00, 0xE8, 0x31, 0x6E, 0x67, 0x00, 0x8B,
    0xD0, 0xE8, 0x4A, 0x3D, 0xF1, 0xFF, 0xD9, 0x05, 0x58, 0x1B, 0xDD, 0x00, 0x0F, 0xB7, 0x8E, 0x84,
    0x00, 0x00, 0x00, 0x6A, 0x00, 0x56, 0x68, 0x50, 0x2F, 0x56, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x83,
    0xEC, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0xE8, 0xD9, 0x1C, 0x24, 0x6A, 0x05, 0x6A, 0x00, 0x51,
    0x0F, 0xB7, 0xC0, 0x50, 0xE8, 0x07, 0x0A, 0x50, 0x00, 0x83, 0xC4, 0x2C, 0x80, 0x48, 0x3D, 0x02,
    0xB0, 0x01, 0xEB, 0x3A, 0x8B, 0x56, 0x44, 0x52, 0xBF, 0xC8, 0x75, 0xDE, 0x00, 0xBE, 0xF4, 0x75,
    0xDE, 0x00, 0xE8, 0x89, 0xEC, 0x28, 0x00, 0x50, 0xBE, 0x80, 0x00, 0x00, 0x00, 0x8D, 0x7C, 0x24,
    0x20, 0xE8, 0xDA, 0xEE, 0x6A, 0x00, 0x8B, 0x3D, 0x0C, 0xD2, 0xEA, 0x00, 0x83, 0xC4, 0x08, 0x8D,
    0x44, 0x24, 0x18, 0xE8, 0xC8, 0x6F, 0x23, 0x00, 0xB0, 0x01, 0xEB, 0x02, 0x32, 0xC0, 0x8B, 0x8C,
    0x24, 0x34, 0x06, 0x00, 0x00, 0x5F, 0x5E, 0x5B, 0x33, 0xCC, 0xE8, 0xC0, 0x02, 0x73, 0x00, 0x81,
    0xC4, 0x2C, 0x06, 0x00, 0x00, 0xC2, 0x04, 0x00,
];
#[allow(dead_code)]
const PBP_BODY_CALLS: [usize; 15] = [0x40, 0x71, 0x7f, 0xd8, 0x143, 0x14f, 0x189, 0x190, 0x1ca, 0x1d1, 0x204, 0x222, 0x231, 0x243, 0x25a];
#[allow(dead_code)]
const PBP_BODY_DISPS: [usize; 15] = [0x7, 0x2a, 0x37, 0x4f, 0x5b, 0x8d, 0xe3, 0x15a, 0x185, 0x197, 0x1c6, 0x1d8, 0x219, 0x21e, 0x238];

#[allow(dead_code)]
const HCS_BODY: [u8; 370] = [
    0xD9, 0xEE, 0x81, 0xEC, 0x04, 0x04, 0x00, 0x00, 0xD8, 0x9C, 0x24, 0x08, 0x04, 0x00, 0x00, 0x53,
    0x55, 0x56, 0x57, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x0F, 0x8B, 0x49, 0x01, 0x00, 0x00, 0x8B, 0x3D,
    0x08, 0xE9, 0x15, 0x02, 0x33, 0xED, 0x85, 0xFF, 0x8D, 0x5D, 0x01, 0x0F, 0x84, 0x9F, 0x00, 0x00,
    0x00, 0xBE, 0x00, 0x02, 0x00, 0x00, 0x8B, 0x4F, 0x4C, 0x3B, 0xCB, 0x75, 0x0C, 0x38, 0x9F, 0xCC,
    0x03, 0x00, 0x00, 0x75, 0x04, 0x8A, 0xC3, 0xEB, 0x02, 0x32, 0xC0, 0x3A, 0x84, 0x24, 0x1C, 0x04,
    0x00, 0x00, 0x75, 0x5F, 0x85, 0xB7, 0xCC, 0x00, 0x00, 0x00, 0x74, 0x57, 0xE8, 0xCF, 0xD1, 0xD1,
    0xFF, 0x3A, 0xC3, 0x75, 0x17, 0x3B, 0xCB, 0x75, 0x08, 0x38, 0x9F, 0xCC, 0x03, 0x00, 0x00, 0x74,
    0x42, 0xE8, 0xAA, 0xD2, 0xD1, 0xFF, 0x84, 0xC0, 0x74, 0x2B, 0xEB, 0x37, 0xE8, 0x3F, 0xD1, 0xD1,
    0xFF, 0x84, 0xC0, 0x74, 0x20, 0x3B, 0xCB, 0x75, 0x13, 0x38, 0x9F, 0xCC, 0x03, 0x00, 0x00, 0x75,
    0x0B, 0x8B, 0xC7, 0xE8, 0xE8, 0x96, 0xFB, 0xFF, 0x84, 0xC0, 0x74, 0x17, 0xE8, 0x7F, 0xD2, 0xD1,
    0xFF, 0x84, 0xC0, 0x75, 0x0E, 0x81, 0xFD, 0x00, 0x01, 0x00, 0x00, 0x73, 0x1E, 0x89, 0x7C, 0xAC,
    0x10, 0x03, 0xEB, 0x8B, 0xBF, 0xB8, 0x00, 0x00, 0x00, 0x3B, 0x3D, 0x08, 0xE9, 0x15, 0x02, 0x74,
    0x0F, 0x85, 0xFF, 0x0F, 0x85, 0x6D, 0xFF, 0xFF, 0xFF, 0xEB, 0x05, 0xBD, 0x00, 0x01, 0x00, 0x00,
    0x33, 0xF6, 0x85, 0xED, 0x76, 0x20, 0x8B, 0x44, 0xB4, 0x10, 0xD9, 0x84, 0x24, 0x18, 0x04, 0x00,
    0x00, 0x51, 0x8B, 0x88, 0x70, 0x05, 0x00, 0x00, 0xD9, 0x1C, 0x24, 0xE8, 0xC0, 0xCD, 0xD8, 0xFF,
    0x03, 0xF3, 0x3B, 0xF5, 0x72, 0xE0, 0x83, 0x3D, 0xE8, 0xA3, 0x52, 0x02, 0x00, 0x74, 0x1B, 0xD9,
    0x84, 0x24, 0x18, 0x04, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x8D, 0x4C, 0x24, 0x14, 0x55, 0x51,
    0xE8, 0x9B, 0xC4, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0xEB, 0x27, 0x33, 0xF6, 0x85, 0xED, 0x76, 0x21,
    0x8B, 0x4C, 0xB4, 0x10, 0xD9, 0x84, 0x24, 0x18, 0x04, 0x00, 0x00, 0x8B, 0x91, 0x70, 0x05, 0x00,
    0x00, 0x51, 0xD9, 0x1C, 0x24, 0x52, 0xE8, 0x25, 0xCF, 0xD8, 0xFF, 0x03, 0xF3, 0x3B, 0xF5, 0x72,
    0xDF, 0x33, 0xF6, 0x85, 0xED, 0x76, 0x20, 0xEB, 0x07, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x44, 0xB4, 0x10, 0x50, 0x8B, 0x80, 0x70, 0x05, 0x00, 0x00, 0x50, 0xE8, 0xFF, 0xCF, 0xD8,
    0xFF, 0x03, 0xF3, 0x3B, 0xF5, 0x72, 0xE9, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0x04, 0x04, 0x00,
    0x00, 0xC3,
];
#[allow(dead_code)]
const HCS_BODY_CALLS: [usize; 9] = [0x5c, 0x71, 0x7c, 0x93, 0x9c, 0xeb, 0x110, 0x136, 0x15c];
#[allow(dead_code)]
const HCS_BODY_DISPS: [usize; 3] = [0x20, 0xbb, 0xf8];

#[allow(dead_code)]
const POP_BODY: [u8; 247] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x83, 0xEC, 0x0C, 0x83, 0x78, 0x30, 0x01, 0x53, 0x55, 0x56, 0x57,
    0x0F, 0x8F, 0xD9, 0x00, 0x00, 0x00, 0x8B, 0x1D, 0xC0, 0xE5, 0x15, 0x02, 0x33, 0xF6, 0x8B, 0xFF,
    0x3B, 0xDE, 0x0F, 0x84, 0xBB, 0x00, 0x00, 0x00, 0x8B, 0x03, 0x8B, 0xE8, 0x2B, 0x2D, 0xC0, 0xE5,
    0x15, 0x02, 0x89, 0x74, 0x24, 0x18, 0xF7, 0xDD, 0x1B, 0xED, 0x23, 0xE8, 0x39, 0x73, 0x1C, 0x0F,
    0x8E, 0x85, 0x00, 0x00, 0x00, 0x8D, 0x4B, 0x08, 0x89, 0x4C, 0x24, 0x14, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x54, 0x24, 0x14, 0x8B, 0x3A, 0x3B, 0xFE, 0x74, 0x5B, 0x39, 0x77, 0x1C, 0x75, 0x51, 0x8B,
    0x47, 0x14, 0x3B, 0xC6, 0x74, 0x06, 0x89, 0x70, 0x18, 0x89, 0x77, 0x14, 0x8B, 0x47, 0x18, 0x3B,
    0xC6, 0x74, 0x06, 0x89, 0x70, 0x14, 0x89, 0x77, 0x18, 0x8B, 0x47, 0x10, 0x3B, 0xC6, 0x74, 0x27,
    0xF6, 0x47, 0x0C, 0x01, 0x74, 0x03, 0x89, 0x70, 0x18, 0x8B, 0x47, 0x08, 0x8B, 0x4F, 0x10, 0x89,
    0x74, 0x81, 0x08, 0x8B, 0x47, 0x10, 0x83, 0x40, 0x20, 0xFF, 0x8B, 0x47, 0x10, 0x39, 0x70, 0x20,
    0x75, 0x05, 0xE8, 0x49, 0xDC, 0xFF, 0xFF, 0x8B, 0xC7, 0xE8, 0x32, 0xDB, 0xFF, 0xFF, 0xEB, 0x05,
    0xE8, 0x3B, 0xE9, 0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x18, 0x83, 0x44, 0x24, 0x14, 0x04, 0x83, 0xC0,
    0x01, 0x3B, 0x43, 0x1C, 0x89, 0x44, 0x24, 0x18, 0x7C, 0x86, 0xE8, 0x81, 0xC8, 0xD6, 0xFF, 0x84,
    0xC0, 0x75, 0x09, 0x53, 0xE8, 0xA7, 0xF6, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xDD, 0xE9, 0x3D,
    0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xE9, 0x91, 0xF4, 0xFF, 0xFF, 0x5F,
    0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
];
#[allow(dead_code)]
const POP_BODY_CALLS: [usize; 5] = [0xa2, 0xa9, 0xb0, 0xca, 0xd4];
#[allow(dead_code)]
const POP_BODY_DISPS: [usize; 3] = [0x1, 0x18, 0x2e];

//@BODIES@

//@KIT@
// (Copied from port_activities.rs, with a few extra stub effects.)
// =================================================================================================
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_frame2 -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod difftest {
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
        /// EAX comes back unchanged (the /GS cookie check).
        Keep,
        /// EAX comes back as the ECX the callee got.
        Ecx,
        /// EAX comes back as the ESI the callee got.
        Esi,
        /// EAX comes back as the first stack argument.
        Arg0,
        /// A millisecond count: mostly 0..5000, sometimes negative or huge.
        Ms,
        /// An index: -1 about a third of the time, else a small number.
        Idx,
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
        /// Removes one handle record from the player's pending list (args: player, object).
        RemoveListEntry,
        /// Writes random floats through the two pointer arguments (3 and 9 dwords).
        FillOut2,
        /// Logs 0x5C bytes at [EAX] and writes three random dwords at [ECX].
        Vec465,
        /// Writes three random dwords at [ESI].
        Out3Esi,
        /// Writes three random floats through the first stack argument.
        WriteVecArg0,
        /// Writes nine random floats at [EAX].
        Fill9Eax,
        /// 0x9A1970's outputs: three floats at each of the two pointers, a flag byte at the third.
        Fill1970,
        /// Logs what 0x9A2670 reads: 3 dwords at each of the first two pointers, the byte at the third.
        Log2670,
        /// Logs the dword at [ESI].
        LogEsi1,
        /// Logs three dwords at [ESI], then rewrites them as a (mostly unit-length) vector.
        LogNormEsi,
        /// Writes a (mostly unit-length) vector through the first stack argument.
        UnitArg0,
        /// Writes a random dword through the first stack argument (a hash).
        HashArg0,
        /// Writes three random dwords at [ESI] (an audio handle).
        Fill3Esi,
        /// Zeroes the 0x5C bytes at [ESI] (the audio params builders).
        ZeroParamsEsi,
        /// Logs three dwords at [EDI].
        LogEdi3,
        /// 0xBE2F50: logs 9 dwords at [ECX] and at the first stack argument, writes 9 floats at [EAX].
        MatMul,
        /// 0xBDDE20: logs 3 dwords at the first stack argument and 9 at [ECX], writes 3 floats at [EAX].
        VecMat,
        /// 0x4171C0: logs 3 dwords at [EDX] and 9 at [ECX], builds the 0x6C-byte record at the first
        /// stack argument as the original does.
        DamageRec,
        /// 0x50AA60: logs 3 dwords at [EDX] and 9 at the second stack argument.
        ObjUpd,
        /// Writes a random 0 / 1 byte at the pointer in the first stack argument.
        FlagArg0,
        /// Writes a random 0 / 1 byte at +0x139C of the player passed as the first stack argument.
        PlayerFlag,
        /// Sets the state dword (+0x3C) of the activity [0x27B3C60] to a random value of 0, 1, 3, 5.
        SetActState,
        /// Logs the vint value at [EAX]: its type, then the byte (type 5) or the dword at +4.
        VintVal,
        /// Like LogBuf, but byte 3 is masked only when the length is 10 (the original leaves it unset).
        LogBufP,
        /// Logs the dword at [EDX].
        LogEdxDword,
        /// Logs `args[1]` dwords (at most 0x100) at the pointer in `args[0]`.
        LogPtrArr,
        /// Logs 0x80 wide chars at [EAX].
        LogWbuf,
        /// A wide-char formatter: fills [EDI] (ESI chars) with random u16 and ends it with a zero.
        FmtBuf,
        /// Writes a random dword at the game address 0x2526C94.
        Rand2526c94,
        /// Logs three dwords at [EAX] and three at the pointer in the first stack argument.
        LogEax3Arg3,
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
        /// Registers the callee reads as 16 bits.
        pub word_regs: u8,
        /// A native replacement the original's call is aimed at (0: the recording stub).
        pub native: usize,
    }

    pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
        Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, word_regs: 0, native: 0 }
    }

    /// Prints what already occupies the address space the game's globals need.
    pub fn dump_occupied() {
        use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, MEM_FREE, VirtualQuery};
        let mut a = 0x40_0000usize;
        while a < 0x400_0000 {
            let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { VirtualQuery(a as *const c_void, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                break;
            }
            if m.State != MEM_FREE {
                eprintln!("occupied {:#x}..{:#x} state {:#x} type {:#x}", m.BaseAddress as usize, m.BaseAddress as usize + m.RegionSize, m.State, m.Type);
            }
            a = m.BaseAddress as usize + m.RegionSize;
        }
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
            let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::tr(a), l)).collect();
            for i in 0..body.len().saturating_sub(3) {
                let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                if (0xD0_0000..0x350_0000).contains(&v) {
                    spans.push((super::tr(v), 1));
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
            unsafe { std::ptr::write_bytes(super::tr(a) as *mut u8, 0, l) };
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
                Some(i) if c[i].native != 0 => c[i].native,
                Some(i) => STUBS.with(|s| s.borrow()[i]),
                None => f,
            }
        })
    }

    /// Adds a line to the recorded call log (for natives that stand in for a callee).
    pub fn push_log(e: Vec<u32>) {
        LOG.with(|l| l.borrow_mut().push(e));
    }

    /// The next number of the stubs' random stream.
    pub fn rand_u32() -> u32 {
        srng()
    }

    fn srng() -> u32 {
        SRNG.with(|s| {
            let mut r = Rng(s.get());
            let v = r.u32();
            s.set(r.0);
            v
        })
    }

    extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
        let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
        let r = unsafe { &*regs };
        if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("stub {:#x} eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} a0 {:#x} a1 {:#x}", c.target, r.eax, r.ecx, r.edx, r.esi, r.edi, unsafe { *args }, unsafe { *args.add(1) });
        }
        // Pointers into the stack differ between the two runs: log a marker for them.
        let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
        if !c.silent {
            let mut e = vec![c.target as u32];
            for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else if c.word_regs & bit != 0 { v & 0xFFFF } else { mask(v) });
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
                Fx::LogEsi1 => e.push(unsafe { *(r.esi as usize as *const u32) }),
                Fx::MatMul | Fx::VecMat | Fx::DamageRec | Fx::ObjUpd => {
                    let rd = |p: usize, n: usize, e: &mut Vec<u32>| {
                        for k in 0..n {
                            e.push(unsafe { *((p + k * 4) as *const u32) });
                        }
                    };
                    let a0 = unsafe { *args } as usize;
                    let a1 = unsafe { *args.add(1) } as usize;
                    match c.fx {
                        Fx::MatMul => {
                            rd(r.ecx as usize, 9, &mut e);
                            rd(a0, 9, &mut e);
                        }
                        Fx::VecMat => {
                            rd(a0, 3, &mut e);
                            rd(r.ecx as usize, 9, &mut e);
                        }
                        Fx::DamageRec => {
                            rd(r.edx as usize, 3, &mut e);
                            rd(r.ecx as usize, 9, &mut e);
                        }
                        _ => {
                            rd(r.edx as usize, 3, &mut e);
                            rd(a1, 9, &mut e);
                        }
                    }
                }
                Fx::LogEdi3 => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.edi as usize + k * 4) as *const u32) });
                    }
                }
                Fx::LogPtrArr => unsafe {
                    let (p0, n) = (*args as usize, (*args.add(1) as usize).min(0x100));
                    for k in 0..n {
                        e.push(*((p0 + k * 4) as *const u32));
                    }
                },
                Fx::LogEdxDword => unsafe {
                    e.push(*(r.edx as usize as *const u32));
                },
                Fx::LogWbuf => unsafe {
                    for k in 0..0x80usize {
                        e.push(*((r.eax as usize + k * 2) as *const u16) as u32);
                    }
                },
                Fx::LogEax3Arg3 => unsafe {
                    let p0 = *args as usize;
                    for k in 0..3usize {
                        e.push(*((r.eax as usize + k * 4) as *const u32));
                    }
                    for k in 0..3usize {
                        e.push(*((p0 + k * 4) as *const u32));
                    }
                },
                Fx::VintVal => unsafe {
                    let t = *(r.eax as usize as *const u32);
                    e.push(t);
                    e.push(if t == 5 { *((r.eax as usize + 4) as *const u8) as u32 } else { *((r.eax as usize + 4) as *const u32) });
                },
                Fx::LogNormEsi => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.esi as usize + k * 4) as *const u32) });
                    }
                }
                Fx::Log2670 => {
                    let (p0, p1, p2) = unsafe { (*args as usize, *args.add(1) as usize, *args.add(2) as usize) };
                    for k in 0..3usize {
                        e.push(unsafe { *((p0 + k * 4) as *const u32) });
                        e.push(unsafe { *((p1 + k * 4) as *const u32) });
                    }
                    e.push(unsafe { *(p2 as *const u8) } as u32);
                }
                Fx::Vec465 => {
                    for k in 0..0x5Cusize {
                        e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
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
            if c.fx == Fx::LogBufP {
                let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                assert!(len < 64, "LogBufP: callee {:#x} len {len:#x} ptr {p:#x}", c.target);
                for k in 0..len {
                    let mut b = unsafe { *((p + k) as *const u8) };
                    if len == 10 && k == 3 {
                        b = 0;
                    }
                    e.push(b as u32);
                }
            }
            if c.fx == Fx::LogBuf {
                let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                assert!(len < 64, "LogBuf: callee {:#x} len {len:#x} ptr {p:#x}", c.target);
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
        if c.fx == Fx::RemoveListEntry {
            unsafe {
                let p = *args as usize;
                let o = *args.add(1);
                let cnt = *((p + 0xC3C) as *const u32) as usize;
                if cnt > 0 && cnt < 64 {
                    let obj_of = |h: u32| -> u32 {
                        if h == 0 {
                            return 0;
                        }
                        let e = *(super::tr(0x0214_9C64 + (((h & 0xFFF) as usize) << 4)) as *const u32);
                        if e != 0 && *((e as usize + 0x44) as *const u32) == h { e } else { 0 }
                    };
                    let hk = |k: usize| *((p + 0xC40 + k * 0x18) as *const u32);
                    let mut sel = None;
                    if o != 0 {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == o);
                    }
                    if sel.is_none() {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == 0);
                    }
                    let j = sel.unwrap_or(0);
                    for k in j..cnt - 1 {
                        std::ptr::copy((p + 0xC40 + (k + 1) * 0x18) as *const u8, (p + 0xC40 + k * 0x18) as *mut u8, 0x18);
                    }
                    *((p + 0xC3C) as *mut u32) = (cnt - 1) as u32;
                }
            }
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
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::FillOut2 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let (p0, p1) = (*args as usize, *args.add(1) as usize);
                for k in 0..3 {
                    *((p0 + k * 4) as *mut u32) = rng.float_bits();
                }
                for k in 0..9 {
                    *((p1 + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Vec465 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.ecx as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Fill9Eax => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..9 {
                    *((r.eax as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Fill1970 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let (p1, p2, p3) = (*args.add(1) as usize, *args.add(2) as usize, *args.add(3) as usize);
                for k in 0..3 {
                    *((p1 + k * 4) as *mut u32) = rng.float_bits();
                    *((p2 + k * 4) as *mut u32) = rng.float_bits();
                }
                *(p3 as *mut u8) = rng.pick(&[0u8, 1, 1, 2]);
            },
            Fx::WriteVecArg0 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let p0 = *args as usize;
                for k in 0..3 {
                    *((p0 + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Out3Esi => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.esi as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::LogNormEsi | Fx::UnitArg0 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let p = if c.fx == Fx::UnitArg0 { *args as usize } else { r.esi as usize };
                let unit = rng.below(4) != 0;
                for k in 0..3 {
                    *((p + k * 4) as *mut u32) = if unit { (rng.below(2001) as f32 / 1000.0 - 1.0).to_bits() } else { rng.float_bits() };
                }
            },
            Fx::HashArg0 => unsafe {
                *(*args as usize as *mut u32) = srng();
            },
            Fx::MatMul | Fx::VecMat => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let n = if c.fx == Fx::MatMul { 9 } else { 3 };
                for k in 0..n {
                    *((r.eax as usize + k * 4) as *mut u32) =
                        if rng.below(4) != 0 { (rng.below(4001) as f32 / 1000.0 - 2.0).to_bits() } else { rng.float_bits() };
                }
            },
            Fx::DamageRec => unsafe {
                let p = *args as usize;
                let w = |o: usize, v: u32| *((p + o) as *mut u32) = v;
                let wb8 = |o: usize, v: u8| *((p + o) as *mut u8) = v;
                w(0, r.esi);
                for k in 0..3 {
                    w(4 + k * 4, *((r.edx as usize + k * 4) as *const u32));
                }
                for k in 0..9 {
                    w(0x10 + k * 4, *((r.ecx as usize + k * 4) as *const u32));
                }
                wb8(0x34, 0);
                w(0x38, 0);
                w(0x50, 1.0f32.to_bits());
                w(0x3C, 0xFFFF_FFFF);
                w(0x68, 1.0f32.to_bits());
                for o in [0x40usize, 0x41, 0x4A, 0x4B, 0x4C, 0x4D, 0x54, 0x55] {
                    wb8(o, 0);
                }
                w(0x44, 0);
                *((p + 0x48) as *mut u16) = 0xFFFF;
                w(0x58, 1);
                w(0x5C, 0);
                w(0x60, 0);
                w(0x64, 0);
            },
            Fx::FmtBuf => unsafe {
                let n = (r.esi as usize).min(0x100);
                for k in 0..n {
                    *((r.edi as usize + k * 2) as *mut u16) = if k + 1 == n { 0 } else { (srng() % 0xFFFF + 1) as u16 };
                }
            },
            Fx::Rand2526c94 => unsafe {
                *(super::tr(0x0252_6C94) as *mut u32) = if srng() % 3 == 0 { 0x6000_0000 + (srng() % 0x1000) } else { srng() };
            },
            Fx::PlayerFlag => unsafe {
                *((*args as usize + 0x139C) as *mut u8) = (srng() % 2) as u8;
            },
            Fx::FlagArg0 => unsafe {
                *(*args as usize as *mut u8) = (srng() % 2) as u8;
            },
            Fx::SetActState => unsafe {
                let act = *(super::tr(0x027B_3C60) as *const u32) as usize;
                if act != 0 {
                    *((act + 0x3C) as *mut u32) = [0u32, 1, 3, 5, 1][srng() as usize % 5];
                }
            },
            Fx::ZeroParamsEsi => unsafe {
                std::ptr::write_bytes(r.esi as usize as *mut u8, 0, 0x5C);
            },
            Fx::Fill3Esi => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.esi as usize + k * 4) as *mut u32) = rng.u32();
                }
            },
            Fx::WriteFloat => {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = rng.float_bits() };
            }
            _ => {}
        }
        match c.ret {
            Ret::Zero => 0,
            Ret::Mod(n) => srng() % n,
            Ret::Keep => r.eax,
            Ret::Ecx => r.ecx,
            Ret::Esi => r.esi,
            Ret::Arg0 => unsafe { *args },
            Ret::Ms => match srng() % 8 {
                0 => 0,
                1 => srng(),
                2 => (srng() % 100) as u32 | 0xFFFF_FF00,
                _ => srng() % 5000,
            },
            Ret::Any => srng(),
            Ret::Idx => match srng() % 6 {
                0 | 1 => 0xFFFF_FFFF,
                2 => 0,
                3 => 1,
                4 => srng() % 40,
                _ => 5,
            },
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
    pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
        /// Also compare AL after the call.
        pub cmp_al: bool,
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
                let common = if c.ret == Ret::St0 { stub_common_st0 as *const () as usize } else { stub_common as *const () as usize };
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
            Harness { mem, orig, thunk, cmp_eax: false, cmp_al: false }
        }

        fn run(&self, f: usize, before: Regs, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
            SRNG.with(|s| s.set(seed | 1));
            unsafe { std::ptr::write_bytes((STACK + STACK_LEN - 0x1_0000) as *mut u8, 0, 0x1_0000) };
            // a clean x87 (53-bit precision, as the game runs)
            unsafe { std::arch::asm!("fninit", "fldcw word ptr [{cw}]", cw = in(reg) &0x027Fu16 as *const u16, options(nostack)) };
            LOG.with(|l| l.borrow_mut().clear());
            let mut r = before;
            unsafe { call_orig(f, &mut r, args) };
            (LOG.with(|l| std::mem::take(&mut *l.borrow_mut())), before, r)
        }

        /// One case on the current memory: original, then Rust, from the same snapshot.
        pub fn check(&self, ecx: u32, args: [u32; 3], seed: u64) -> Result<(), String> {
            let before = Regs { eax: 0xAAAA_0001, ecx, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 };
            self.check_regs(before, args, seed)
        }

        /// Like `check`, with every entry register given (ebx / ebp are checked as callee-saved).
        pub fn check_regs(&self, before: Regs, args: [u32; 3], seed: u64) -> Result<(), String> {
            let snap = self.mem.snapshot();
            let (la, ba, ra) = self.run(self.orig, before, args, seed);
            let after_a = self.mem.snapshot();
            self.mem.restore(&snap);
            let (lb, _bb, rb) = self.run(self.thunk, before, args, seed);
            let after_b = self.mem.snapshot();
            if self.cmp_eax && ra.eax != rb.eax {
                return Err(format!("eax: original {:#x} rust {:#x}", ra.eax, rb.eax));
            }
            if self.cmp_al && (ra.eax & 0xFF) != (rb.eax & 0xFF) {
                return Err(format!("al: original {:#x} rust {:#x}", ra.eax & 0xFF, rb.eax & 0xFF));
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
mod ahm_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x005E_6DF0;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0xb94b60, 2, 8, EAX, Ret::Any, Fx::VintVal),
            callee(0xb93790, 0, 0, EAX, Ret::Any, Fx::VintVal),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut tabs = vec![];
            for k in 0..8usize {
                let m = AH_RECS + k * 0x1C;
                wb(m + 0x16, rng.pick(&[0, 1, 1, 1, 2]));
                let id = rng.below(64);
                wd(m - 4, id);
                tabs.push(id);
                for q in [0usize, 4, 0xC, 0x10] {
                    wd(m + q, rng.float_bits());
                }
                wb(m + 0x14, rng.u32() as u8);
                wb(m + 0x15, rng.u32() as u8);
                let slot = if rng.one_in(8) { 0x80 + rng.below(0x100) } else { rng.below(0x80) };
                let gn = rng.pick(&[0u32, 1, 2, 0x7FFF, 0xFFFF]);
                let hnd = if rng.one_in(10) { 0 } else { gn << 16 | slot };
                wd(0x0252_77D8 + id as usize * 4, hnd);
                if slot < 0x80 {
                    wd(0x031A_96F0 + slot as usize * 0x22C + 0x224, if rng.one_in(5) { rng.u32() } else { hnd });
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn activity_hud_markers_apply_matches_original() {
        let _g = lock();
        let extra = [(0x0140_A700usize, 0x200), (0x0252_77D8usize, 0x400), (0x031A_96F0usize, 0x80 * 0x22C)];
        let h = Harness::new(ADDR, &AHM_BODY, &AHM_BODY_CALLS, &AHM_BODY_DISPS, callees(), &extra, activity_hud_markers_thunk as *const () as usize);
        let mut rng = Rng(0x1357_9BDF_2468_ACE1);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in AH_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, AH_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

/// Registration objects shared by the net tests: ten of them (one in eight slots left null) whose
/// vtable holds the recording stubs at slot 8 (0x20) and slot 0x11 (0x44).
#[cfg(test)]
unsafe fn net_regs(a: &mut difftest::Arena, rng: &mut difftest::Rng) -> Vec<u32> {
    use difftest::*;
    let vt = a.pool(1, 0x80)[0];
    let s8 = redirect(0xBEEF_0001) as u32;
    let s44 = redirect(0xBEEF_0002) as u32;
    for k in 0..0x20usize {
        put(vt, k * 4, 0xDEAD_0000 + k as u32);
    }
    put(vt, 0x20, s8);
    put(vt, 0x44, s44);
    let regs = a.pool(10, 0x10);
    let mut live = vec![];
    for k in 0..10usize {
        put(regs[k], 0, vt);
        let r = if rng.one_in(8) { 0 } else { regs[k] };
        wd(0x0212_E25C + k * 4, r);
        if r != 0 {
            live.push(k as u32);
        }
    }
    if live.is_empty() {
        wd(0x0212_E25C, regs[0]);
        live.push(0);
    }
    live
}

#[cfg(test)]
mod nro_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0080_4850;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0xBEEF_0001, 1, 4, EAX | ECX | EDX, Ret::Any, Fx::None),
            callee(0xBEEF_0002, 1, 4, EAX | ECX | EDX, Ret::Bool, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let live = net_regs(&mut a, rng);
            wd(0x0252_8BB4, rng.pick(&[0, 0, 0, 0xFFFF_FFFF, 1, 5]));
            let player = a.pool(1, 0x500)[0];
            let pl = a.pool(1, 0x200)[0];
            wd(0x0252_8C14, if rng.one_in(8) { 0 } else { player });
            put(player, 0x408, pl);
            let owner = rng.below(3) as u8;
            putb(pl, 0x1B0, owner);
            let nodes = a.pool(40, 0x680);
            let mut n = 0usize;
            for t in 0..10usize {
                let k = rng.pick(&[0usize, 0, 1, 2, 3, 4]);
                if k == 0 {
                    wd(0x0212_E284 + t * 4, 0);
                    continue;
                }
                let chain: Vec<u32> = (0..k).map(|q| nodes[n + q]).collect();
                n += k;
                let circular = rng.below(3) != 0;
                for q in 0..k {
                    let nd = chain[q];
                    putb(nd, 0, if rng.one_in(2) { owner } else { rng.below(3) as u8 });
                    put(nd, 4, rng.u32());
                    put(nd, 8, rng.pick(&live));
                    put(nd, 0x674, if q + 1 < k { chain[q + 1] } else if circular { chain[0] } else { 0 });
                }
                wd(0x0212_E284 + t * 4, chain[0]);
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn net_remote_objects_do_frame_matches_original() {
        let _g = lock();
        let extra = [(0x0212_E250usize, 0x80)];
        let h = Harness::new(ADDR, &NRO_BODY, &NRO_BODY_CALLS, &NRO_BODY_DISPS, callees(), &extra, net_remote_objects_thunk as *const () as usize);
        let mut rng = Rng(0x0123_4567_89AB_CDEF);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in NR_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, NR_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod nru_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0080_45F0;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0xBEEF_0001, 1, 4, EAX | ECX | EDX, Ret::Any, Fx::None),
            callee(0xBEEF_0002, 1, 4, EAX | ECX | EDX, Ret::Bool, Fx::None),
            callee(0x806910, 3, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let live = net_regs(&mut a, rng);
            wd(0x0252_8C14, if rng.one_in(8) { 0 } else { 0x1111_1000 });
            let count = rng.pick(&[0u32, 1, 2, 3, 4, 6, 8, 8, 0xFFFF_FFFF, 5]);
            wd(0x0252_8BB0, count);
            // handles and their objects
            let hs: Vec<u32> = (0..6u32).map(|k| 0x1000 * (k + 1) + 0x30 + k).collect();
            let hobjs = a.pool(6, 0x80);
            for k in 0..6usize {
                put(hobjs[k], 0x44, if rng.one_in(6) { rng.u32() } else { hs[k] });
                wd(0x0214_9C64 + (((hs[k] & 0xFFF) as usize) << 4), if rng.one_in(8) { 0 } else { hobjs[k] });
            }
            // registered objects
            let robjs = a.pool(8, 0x20);
            for k in 0..8usize {
                put(robjs[k], 4, if rng.one_in(8) { 0 } else { rng.pick(&hs) });
                put(robjs[k], 8, rng.pick(&live));
            }
            // the hash table: keys 1..=12
            let nb = rng.pick(&[1u32, 2, 3, 7, 7, 16, 31]);
            let buckets_mem = a.pool(1, 0x400)[0];
            let buckets = buckets_mem + 0x100;
            for q in 0..nb as i32 + 40 {
                put(buckets, (q as usize) * 4, 0);
            }
            for q in 1..=0x40i32 {
                put(buckets, ((-q) as usize).wrapping_mul(4), 0);
            }
            let tbl = a.pool(1, 0x80)[0];
            put(tbl, 0x5C, buckets);
            put(tbl, 0x60, nb);
            let nodes = a.pool(16, 0x20);
            for k in 1..=12u32 {
                if rng.one_in(3) {
                    continue;
                }
                let nd = nodes[(k - 1) as usize];
                put(nd, 0, if rng.one_in(6) { 0 } else { rng.pick(&robjs) });
                put(nd, 0xC, k);
                let r = (super::nu_hash(k) as i32).wrapping_rem(nb as i32);
                let slot = (buckets as usize).wrapping_add((r as u32 as usize).wrapping_mul(4));
                // chain in front of whatever is there
                put(nd, 4, *(slot as *const u32));
                *(slot as *mut u32) = nd;
            }
            wd(0x0212_E258, tbl);
            // the pending table
            for k in 0..9usize {
                let e = 0x0212_BA38 + k * 8;
                wd(e, rng.mix(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0x10001], 0x20));
                wd(e + 4, rng.u32());
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn net_registered_objects_update_matches_original() {
        let _g = lock();
        let extra = [(0x0212_BA20usize, 0x100), (0x0212_E250usize, 0x80), (0x0214_9C64usize, 0x10000)];
        let h = Harness::new(ADDR, &NRU_BODY, &NRU_BODY_CALLS, &NRU_BODY_DISPS, callees(), &extra, net_registered_objects_thunk as *const () as usize);
        let mut rng = Rng(0x0FED_CBA9_8765_4321);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in NU_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, NU_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod sdu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0056_BD60;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x56b310, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x56b7a0, 1, 4, EAX, Ret::Any, Fx::LogEax3Arg3),
            callee(0x569ff0, 0, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let ents = a.pool(24, 0x18);
            let n = rng.pick(&[0u32, 1, 2, 3, 5, 8, 20, 0xFFFF_FFFF, 4]);
            wd(0x0252_7538, n);
            for k in 0..22usize {
                wd(0x0140_A1F4 + k * 4, ents[(rng.below(24)) as usize]);
            }
            let now = rng.mix(&[0, 100, 1000, 0x6B49_D200, 0x6B49_D1F0, 0x6B49_D19C, 0x4000_0000, 0x7000_0000], 0x7000_0000);
            wd(0x00E8_43AC, now);
            let nodes = a.pool(8, 0x50);
            let k = rng.pick(&[0usize, 1, 2, 3, 4, 6]);
            if k == 0 {
                wd(0x0252_753C, 0);
            } else {
                wd(0x0252_753C, nodes[0]);
                let circ = rng.below(3) != 0;
                for q in 0..k {
                    let nd = nodes[q];
                    put(nd, 4, if q + 1 < k { nodes[q + 1] } else if circ { nodes[0] } else { 0 });
                    let t = match rng.below(8) {
                        0 => rng.u32(),
                        1 => now,
                        2 => now.wrapping_add(rng.below(300)),
                        3 => now.wrapping_sub(rng.below(300)),
                        4 => now.wrapping_add(0x35A4_E900),
                        5 => now.wrapping_sub(0x35A4_E900),
                        6 => 0xFFFF_FFFF,
                        _ => rng.below(0x7000_0000),
                    };
                    put(nd, 0x34, t);
                    putb(nd, 0x24, rng.pick(&[0, 1, 1, 7]));
                    putb(nd, 0x18, rng.pick(&[0, 4, 4, 5, 3, 0xFF]));
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn septic_decals_update_matches_original() {
        let _g = lock();
        let extra = [(0x0140_A1F0usize, 0x60)];
        let h = Harness::new(ADDR, &SDU_BODY, &SDU_BODY_CALLS, &SDU_BODY_DISPS, callees(), &extra, septic_decals_thunk as *const () as usize);
        let mut rng = Rng(0x2468_ACE1_3579_BDF1);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in SD_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, SD_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod rzu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x00A5_8EE0;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0xa59240, 0, 0, EAX | EDI, Ret::Bool, Fx::None),
            callee(0x610580, 0, 0, ESI, Ret::Bool, Fx::None),
            callee(0x55fa40, 3, 0, 0, Ret::Any, Fx::None),
            callee(0xa58900, 1, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            wb(0x0252_7D14, rng.pick(&[0, 1, 0]));
            wb(0x0252_7CF7, rng.pick(&[0, 1, 0]));
            wb(0x0252_7D15, rng.pick(&[0, 0, 0, 1]));
            wb(0x00E8_7BE3, rng.pick(&[1, 1, 1, 0, 2]));
            let player = a.pool(1, 0x500)[0];
            let local = a.pool(1, 0x80)[0];
            wd(0x0252_8C14, if rng.one_in(5) { 0 } else { player });
            put(player, 0x408, if rng.one_in(6) { 0 } else { local });
            put(local, 0x3C, rng.pick(&[0xD, 0xD, 0xD, 1, 0xE]));
            wd(0x0217_03D4, rng.pick(&[0x1234_5678, a.pool(1, 0x40)[0]]));
            let zones = a.pool(6, 0x40);
            let ptrs = a.pool(6, 0x40);
            let k = rng.pick(&[0usize, 1, 2, 3, 4, 6]);
            if k == 0 {
                wd(0x0252_9DEC, 0);
            } else {
                wd(0x0252_9DEC, zones[0]);
                let circ = rng.below(3) != 0;
                for q in 0..k {
                    let z = zones[q];
                    put(z, 0x1C, if q + 1 < k { zones[q + 1] } else if circ { zones[0] } else { 0 });
                    put(z, 0x24, if rng.one_in(6) { 0 } else { ptrs[q] });
                    put(z, 0x28, rng.u32());
                    put(z, 0x2C, rng.pick(&[0, 1, 5, 0xFFFF_FFFF, 0x7FFF_FFFF, 3, 9]));
                }
            }
            wd(0x00E8_7C84, rng.mix(&[0xFFFF_FFFF, 1, 5, 3, 9, 0], 0));
            let rv = rng.u32();
            wd(0x0252_9DF8, rng.pick(&[0, ptrs[0], ptrs[1], ptrs[2], ptrs[3], rv]));
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn restricted_zones_update_matches_original() {
        let _g = lock();
        let extra = [(0x0252_9DE0usize, 0x20)];
        let h = Harness::new(ADDR, &RZU_BODY, &RZU_BODY_CALLS, &RZU_BODY_DISPS, callees(), &extra, restricted_zones_thunk as *const () as usize);
        let mut rng = Rng(0x7531_ECA8_6420_9BDF);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in RZ_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, RZ_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod hnu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x009A_7080;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x4b19f0, 2, 0, 0, Ret::Idx, Fx::None),
            callee(0x9e1190, 1, 4, ESI, Ret::Any, Fx::Rand2526c94),
            callee(0x9cfb30, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x98f680, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x98dcc0, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x986060, 0, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let hum = a.pool(6, 0x1000);
            wd(0x0252_6C94, if rng.one_in(3) { 0x6000_0000 + rng.below(0x1000) } else { rng.u32() });
            let k = rng.pick(&[0usize, 1, 2, 3, 4, 6]);
            if k == 0 {
                wd(0x0215_E908, 0);
            } else {
                wd(0x0215_E908, hum[0]);
                let circ = rng.below(4) != 0;
                for q in 0..k {
                    let x = hum[q];
                    put(x, 0xB8, if q + 1 < k { hum[q + 1] } else if circ { hum[0] } else { 0 });
                    put(x, 0x4C, rng.pick(&[1, 1, 1, 0, 2, 0x101]));
                    putb(x, 0x3CC, rng.pick(&[1, 1, 1, 0, 2]));
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn humans_update_neighborhoods_matches_original() {
        let _g = lock();
        let extra = [(0x0215_E900usize, 0x20)];
        let h = Harness::new(ADDR, &HNU_BODY, &HNU_BODY_CALLS, &HNU_BODY_DISPS, callees(), &extra, humans_update_neighborhoods_thunk as *const () as usize);
        let mut rng = Rng(0x5A5A_1234_C3C3_9876);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            let dt = rng.float_bits();
            if let Err(e) = h.check(0, [dt, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in HN_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, HN_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod hdf_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0079_3A40;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x79da00, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x79d640, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x79b9f0, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x7df470, 0, 0, 0, Ret::Any, Fx::None),
            callee(0xb94380, 0, 0, EDX, Ret::Any, Fx::None),
            callee(0x79aae0, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x5e5460, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x5e6df0, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x8c2250, 1, 4, 0, Ret::Any, Fx::None),
            callee(0xb94b60, 2, 8, EAX, Ret::Any, Fx::VintVal),
            callee(0xb93790, 0, 0, EAX, Ret::Any, Fx::VintVal),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let pl = a.pool(1, 0x1200)[0];
            wd(0x0217_03D4, pl);
            wd(0x0252_7C08, rng.pick(&[0, 0, 1, 0xFFFF_FFFF]));
            // pending HUD item
            let slot = rng.below(0x20);
            let hh = rng.pick(&[0u32, 3, 5]) << 16 | slot;
            let hh = if rng.one_in(6) { 0 } else if rng.one_in(8) { slot + 0x20 } else { hh };
            wd(0x0252_858C, hh);
            if (hh & 0xFFFF) < 0x20 {
                let rec = 0x031B_ACF0 + (hh & 0xFFFF) as usize * 32;
                wd(rec + 0x14, if rng.one_in(5) { rng.u32() } else { hh });
                wb(rec + 0x1C, rng.pick(&[0, 0, 0, 1]));
            }
            // widget chain
            let ch = a.pool(5, 0x20);
            wd(0x0213_B4F8, if rng.one_in(8) { 0 } else { ch[0] });
            put(ch[0], 4, if rng.one_in(8) { 0 } else { ch[1] });
            put(ch[1], 4, if rng.one_in(8) { 0 } else { ch[2] });
            put(ch[2], 4, ch[3]);
            put(ch[3], 0, rng.pick(&[5, 5, 5, 4]));
            wb(0x0252_8B48, rng.pick(&[1, 1, 1, 0]));
            wb(0x0252_8B49, rng.pick(&[0, 0, 0, 1]));
            wd(0x00E9_6C00, rng.pick(&[0xD, 0xD, 0xD, 0xC]));
            wd(0x0213_C444, rng.pick(&[1, 1, 1, 0]));
            // human item
            let s2 = rng.below(0x80);
            let h2 = rng.pick(&[0u32, 1, 7]) << 16 | s2;
            let h2 = if rng.one_in(6) { 0 } else if rng.one_in(8) { s2 + 0x80 } else { h2 };
            wd(0x0252_85C4, h2);
            if (h2 & 0xFFFF) < 0x80 {
                let item = 0x031A_96F0 + (h2 & 0xFFFF) as usize * 0x22C;
                wd(item + 0x224, if rng.one_in(5) { rng.u32() } else { h2 });
            }
            wd(0x02A4_D148, rng.mix(&[0, 1, 100, 0xFFFF, 0x1_0005], 0));
            // pending request
            wb(0x0252_8525, rng.pick(&[1, 1, 0, 2]));
            let s3 = rng.below(0xC);
            let h3 = rng.pick(&[0u32, 2, 9]) << 16 | s3;
            let h3 = if rng.one_in(6) { 0 } else if rng.one_in(8) { s3 + 0xC } else { h3 };
            wd(0x0252_85CC, h3);
            if (h3 & 0xFFFF) < 0xC {
                let rec = 0x0319_F6B0 + (h3 & 0xFFFF) as usize * 0x57C;
                wd(rec + 0x4C, if rng.one_in(5) { rng.u32() } else { h3 });
                wb(rec + 0x460, rng.u32() as u8);
            }
            wd(0x00E8_4380, rng.float_bits());
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn hud_do_frame_matches_original() {
        let _g = lock();
        let extra = [(0x031B_ACF0usize, 0x20 * 32), (0x031A_96F0usize, 0x80 * 0x22C), (0x0319_F6B0usize, 0xC * 0x57C), (0x02A4_D140usize, 0x10)];
        let h = Harness::new(ADDR, &HDF_BODY, &HDF_BODY_CALLS, &HDF_BODY_DISPS, callees(), &extra, hud_do_frame_thunk as *const () as usize);
        let mut rng = Rng(0x6B6B_2D2D_9A9A_0707);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in HD_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, HD_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod pbp_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0056_5B30;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x80fcf0, 1, 4, 0, Ret::Bool, Fx::None),
            callee(0x6a74e0, 0, 0, 0, Ret::Idx, Fx::None),
            callee(0x6045e0, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x814740, 2, 0, EDI, Ret::Any, Fx::LogBufP),
            callee(0xbdcb30, 1, 4, EAX | ECX, Ret::Arg0, Fx::HashArg0),
            callee(0x479a50, 0, 0, EDX, Ret::Any, Fx::LogEdxDword),
            callee(0xa66740, 11, 0, 0, Ret::PtrNz(0), Fx::None),
            callee(0x7f49e0, 1, 0, ESI | EDI | EDX, Ret::Any, Fx::None),
            callee(0xc14c40, 2, 0, ESI | EDI, Ret::Any, Fx::FmtBuf),
            callee(0x79cd40, 0, 0, EAX | EDI, Ret::Any, Fx::LogWbuf),
            Callee { silent: true, ..callee(0xc9604f, 0, 0, 0, Ret::Keep, Fx::None) },
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) -> u32 {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            set_pools(vec![a.pool(4, 0x80)]);
            let obj = a.pool(1, 0x100)[0];
            let player = a.pool(1, 0x500)[0];
            wb(0x0252_72CE, rng.pick(&[0, 0, 0, 0, 0, 0, 1]));
            wd(0x0252_8C14, if rng.one_in(3) { 0 } else { player });
            wd(0x00E9_6C00, rng.pick(&[3, 3, 3, 3, 2]));
            wd(0x0145_0380, rng.pick(&[0, 0, 0, 2, 5, 6]));
            wd(0x00E9_E3E4, rng.pick(&[0, 0, 1, 1, 2]));
            wb(0x00E8_BC2C, rng.pick(&[0, 1]));
            wd(0x0252_7888, rng.pick(&[0, 0, 1]));
            wd(0x00EA_D20C, rng.u32());
            wd(0x00DD_1B58, rng.float_bits());
            put(obj, 0x40, rng.u32());
            put(obj, 0x44, rng.u32());
            put(obj, 0x4C, rng.pick(&[0, 0, 0, 0, 1]));
            put(obj, 0x50, rng.pick(&[1, 2, 3, 1, 2, 3, 0, 4]));
            putb(obj, 0x58, rng.pick(&[0, 0, 0, 1, 2, 3, 4, 0xFC]));
            put(obj, 0x84, rng.pick(&[0xFFFF, 0xFFFF, 0x1234, 0x0001_FFFF, 0, 7]));
            if rng.one_in(8) { 0 } else { obj }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn pushback_phone_call_hud_text_matches_original() {
        let _g = lock();
        let extra = [(0x0252_72C0usize, 0x20), (0x0252_8C00usize, 0x20), (0x00E9_E3E0usize, 0x10), (0x0145_0380usize, 0x10), (0x00DD_1B58usize, 8)];
        let mut h = Harness::new(ADDR, &PBP_BODY, &PBP_BODY_CALLS, &PBP_BODY_DISPS, callees(), &extra, pushback_phone_call_thunk as *const () as usize);
        h.cmp_al = true;
        let mut rng = Rng(0x3C3C_8181_E7E7_1A1A);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            let obj = unsafe { world(&h, &mut rng) };
            let flag = rng.pick(&[0u32, 1, 1, 0x100, 0x2, 0xFFFF_FF00, 0xAB]);
            if let Err(e) = h.check(obj, [flag, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in PB_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, PB_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod hcs_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x009A_9B60;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x6c6d90, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x6c6e80, 0, 0, EDI, Ret::Bool, Fx::None),
            callee(0x6c6d20, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x9632e0, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x736a10, 1, 4, ECX | EAX, Ret::Any, Fx::None),
            callee(0x9a6110, 3, 0, 0, Ret::Any, Fx::LogPtrArr),
            callee(0x736bc0, 2, 8, ECX | EDX, Ret::Any, Fx::None),
            callee(0x736cc0, 2, 8, EAX, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) -> Option<u32> {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let big = rng.one_in(25);
            let k = if big { 600usize } else { rng.pick(&[0usize, 1, 2, 3, 5, 10]) };
            let hum = a.pool(k.max(1), 0x580);
            wd(0x0252_A3E8, rng.pick(&[0, 1, 0, 2]));
            if k == 0 {
                wd(0x0215_E908, 0);
                return None;
            }
            wd(0x0215_E908, hum[0]);
            let circ = rng.below(4) != 0;
            for q in 0..k {
                let x = hum[q];
                put(x, 0xB8, if q + 1 < k { hum[q + 1] } else if circ { hum[0] } else { 0 });
                put(x, 0x4C, if big { 0 } else { rng.pick(&[1, 1, 1, 0, 2]) });
                putb(x, 0x3CC, rng.pick(&[1, 1, 1, 0, 2]));
                put(x, 0xCC, if rng.one_in(6) && !big { rng.u32() & !0x200 } else { rng.u32() | 0x200 });
            }
            if big { Some(0) } else { None }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn humans_character_step_matches_original() {
        let _g = lock();
        let extra = [(0x0215_E900usize, 0x20), (0x0252_A3E0usize, 0x20)];
        let h = Harness::new(ADDR, &HCS_BODY, &HCS_BODY_CALLS, &HCS_BODY_DISPS, callees(), &extra, humans_character_step_thunk as *const () as usize);
        let mut rng = Rng(0x4D4D_A1A1_7373_C5C5);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            let forced = unsafe { world(&h, &mut rng) };
            let dt = if rng.one_in(10) && forced.is_none() { rng.pick(&[0u32, 0x8000_0000]) } else if forced.is_some() { 0x3F80_0000 } else { rng.float_bits() };
            let flag = forced.unwrap_or_else(|| rng.pick(&[0u32, 1, 1, 0x100, 0xFF01, 2]));
            if let Err(e) = h.check(0, [dt, flag, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in HC_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, HC_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod pop_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0095_A3D0;
    /// The original's calls plus the tail `jmp 0x959950` at +0xEA (patched the same way).
    const CALLS_AND_TAIL: [usize; 6] = [0xa2, 0xa9, 0xb0, 0xca, 0xd4, 0xea];

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x958dc0, 0, 0, EDI, Ret::Any, Fx::None),
            callee(0x9580c0, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x957fb0, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x6c6d20, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x959b50, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x959950, 0, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let player = a.pool(1, 0x40)[0];
            wd(0x0252_8C14, player);
            put(player, 0x30, rng.pick(&[0, 1, 1, 2, 0xFFFF_FFFF, 0]));
            let nodes = a.pool(4, 0x60);
            let slots = a.pool(16, 0x40);
            let k = rng.pick(&[0usize, 1, 2, 3, 4]);
            if k == 0 {
                wd(0x0215_E5C0, 0);
            } else {
                wd(0x0215_E5C0, nodes[0]);
            }
            let circ = rng.below(3) != 0;
            for q in 0..k {
                let nd = nodes[q];
                put(nd, 0, if q + 1 < k { nodes[q + 1] } else if circ { nodes[0] } else { 0 });
                let cnt = rng.pick(&[0u32, 1, 2, 3, 4, 0xFFFF_FFFF, 3]);
                put(nd, 0x1C, cnt);
                put(nd, 0x20, rng.pick(&[0, 1, 2, 5, 0xFFFF_FFFF]));
                put(nd, 0x18, rng.u32());
                for s in 0..4usize {
                    let sl = slots[q * 4 + s];
                    put(nd, 8 + s * 4, if rng.one_in(5) { 0 } else { sl });
                    // slot fields
                    put(sl, 0x1C, rng.pick(&[0, 0, 0, 1, 7]));
                    put(sl, 0x14, if rng.one_in(2) { 0 } else { slots[rng.below(16) as usize] });
                    put(sl, 0x18, if rng.one_in(2) { 0 } else { slots[rng.below(16) as usize] });
                    put(sl, 0x10, if rng.one_in(4) { 0 } else { nodes[rng.below(4) as usize] });
                    put(sl, 8, rng.below(4));
                    putb(sl, 0xC, rng.u32() as u8);
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn population_update_matches_original() {
        let _g = lock();
        let extra = [(0x0215_E5C0usize, 0x20), (0x0252_8C10usize, 0x10)];
        let mut h = Harness::new(ADDR, &POP_BODY, &CALLS_AND_TAIL, &POP_BODY_DISPS, callees(), &extra, population_update_thunk as *const () as usize);
        h.cmp_eax = true;
        let mut rng = Rng(0x1F1F_4B4B_90C0_AD07);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in PO_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, PO_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

//@TESTS@
