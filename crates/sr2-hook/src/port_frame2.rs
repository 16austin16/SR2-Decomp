//! Port module (frame-loop callees, round 2): eight sim_frame callees as whole-function
//! replacements, all on the game (sim) thread. Callees stay the original game functions, called by
//! address with the registers the LTCG build sets up; every comparison, list splice and global
//! write follows the disassembly.
//!
//! - zone_watchers_update   0xA93FF0 (329 bytes)
//! - persona_events_update  0x6ADB10 (541)
//! - nav_detours_update     0xA0AFF0 (568)
//! - vehicle_attached_effects_update 0x4D1D50 (484)
//! - ai_scheduled_tasks_tick 0x452820 (420)
//! - explosions_flush_queue 0x4C2BF0 (311)
//! - wiretap_triggers_update 0x54CCA0 (379)
//! - cellphone_do_frame     0xA66340 (585)
//!
//! Known, unreproducible detail: ai_scheduled_tasks_tick buckets nodes by `[node+0x10]` into a
//! 4-entry stack array without a bounds check; an out-of-range index would scribble on the
//! original's stack, here it is clamped to bucket 3 (counted as `tick_oob`).

#![allow(unsafe_op_in_unsafe_fn, unused_assignments, dead_code)]

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
    let f = difftest::redirect(f);
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

// The offline differential test (module `difftest`, bottom of the file) keeps the game's globals
// (0xD00000..0x350000) at a fixed offset, because their addresses are taken in a test process;
// the Rust ports read through these shadows there. Pointers to the test's objects lie outside
// that range. The production build uses the portlib versions.
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

// --- Counters -------------------------------------------------------------------------------

macro_rules! counters {
    ($($id:ident $name:expr,)*) => {
        #[allow(non_camel_case_types, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(usize)]
        enum C { $($id,)* N }
        static NAMES: [&str; C::N as usize] = [$($name,)*];
        static CNT: [AtomicU32; C::N as usize] = [const { AtomicU32::new(0) }; C::N as usize];
    };
}

counters! {
    ZwCalls "zone_calls",
    ZwMoved "zone_moved",
    ZwStay "zone_stay",
    ZwEnter "zone_enter_cb",
    ZwLeave "zone_leave_cb",
    ZwMember "zone_member_cb",
    PeCalls "persona_calls",
    PeTimers "persona_timers_fired",
    PeLookups "persona_hash_found",
    PeDeferred "persona_deferred_calls",
    NdCalls "nav_calls",
    NdInvalid "nav_invalid",
    NdActive "nav_active",
    NdRotate "nav_rotate",
    NdInactive "nav_inactive",
    VaCalls "vae_calls",
    VaCopy "vae_copy",
    VaMoved "vae_moved",
    TkCalls "tick_calls",
    TkFire "tick_fired",
    TkOob "tick_oob",
    ExCalls "expl_calls",
    ExRecords "expl_records",
    WtCalls "wiretap_calls",
    WtSkip "wiretap_skip",
    CpCalls "cell_calls",
    CpRemoved "cell_removed",
    HkCalls "hk_calls",
    HkNodes "hk_nodes",
    HkElems "hk_elems",
    HkNoEntries "hk_no_entries",
    HkNoType1 "hk_no_type1",
    HkType6 "hk_type6",
    HkLookupNull "hk_lookup_null",
    HkDirect "hk_direct",
    HkHashed "hk_hashed",
    HkSkipFlags "hk_skip_flags",
    HkRecPrimary "hk_rec_primary",
    HkChainSeen "hk_chain_seen",
    HkRecChain "hk_rec_chain",
    HkFree "hk_free_heap",
    HkNotify "hk_notify",
    HkBitOob "hk_bit_oob",
    ChCalls "cheats_calls",
    ChActive "cheats_active",
    ChStart "cheats_start",
    ChCancel "cheats_cancel",
    ChSame "cheats_same",
    ChBusy "cheats_busy",
    ChNoMatch "cheats_nomatch",
    ChSwitch "cheats_switch",
    ChCallback "cheats_callback",
    RpCalls "rich_calls",
    RpRun "rich_run",
    RpNoPlayer "rich_no_player",
    RpActivity "rich_activity",
    RpRankState "rich_rank",
    RpScoreState "rich_score",
    RpChanged "rich_changed",
    GaCalls "garage_calls",
    GaReset "garage_reset",
    GaStart "garage_start",
    GaBusy "garage_busy",
    GaAttach "garage_attach",
    GaState1 "garage_st1",
    GaState2 "garage_st2",
    GaState3 "garage_st3",
    GaLeave "garage_leave",
    GaText "garage_text",
    GaNullVehicle "garage_null_vehicle",
    GaPrompt "garage_prompt",
    GaClosePrompt "garage_close_prompt",
    TdCalls "trig_calls",
    TdFade "trig_fade",
    TdClear "trig_clear",
    TdEntries "trig_entries",
    TdCallback "trig_callback",
    ViCalls "vehint_calls",
    ViStale "vehint_stale",
    ViLive "vehint_live",
    ViExit "vehint_exit",
    ViDrop "vehint_drop",
    ViState1 "vehint_state1",
    ViState2 "vehint_state2",
    ViState3 "vehint_state3",
    TeCalls "tasks_expire_calls",
    TeTasks "tasks_expire_tasks",
    TeDue "tasks_expire_due",
    TeStale "tasks_expire_stale",
    TeHuman "tasks_expire_human",
    TeRange "tasks_expire_range",
    SwCalls "sched_events_calls",
    SwMoved "sched_events_moved",
    SwNodes "sched_events_nodes",
    SwFired "sched_events_fired",
}

fn bump(c: C) {
    CNT[c as usize].fetch_add(1, Ordering::Relaxed);
}

// --- Helpers --------------------------------------------------------------------------------

const HANDLE_TABLE: usize = 0x0214_9C64;

unsafe fn rw(p: usize) -> u16 {
    *(p as *const u16)
}

/// The handle-table slot pointer for handle `h` (no validation).
unsafe fn slot(h: u32) -> usize {
    rd(HANDLE_TABLE + (((h & 0xFFF) << 4) as usize)) as usize
}

/// `fld dword` + `fstp dword`: an f32 round trip through the x87.
unsafe fn f32rt(p: usize) -> f32 {
    (rf(p) as f64) as f32
}


// --- zone_watchers_update (0xA93FF0) ----------------------------------------------------------

kxs0!(zone_watchers_thunk, zone_watchers);

extern "C" fn zone_watchers() {
    unsafe {
        bump(C::ZwCalls);
        const G: usize = 0x0223_E95C; // list of watchers
        const MOVED: usize = 0x0223_E964; // list of members that left
        let mut edi = rd(G) as usize;
        if edi == 0 {
            return;
        }
        loop {
            let al = rb(edi + 0xC);
            let mut esi = rd(edi + 8) as usize;
            wb(edi + 0xD, al);
            wb(edi + 0xC, 1);
            if esi != 0 {
                while esi != 0 {
                    let eax = rd(esi);
                    wb(esi + 0x11, rb(esi + 0x10));
                    let ebp = if eax != rd(edi + 8) { eax as usize } else { 0 };
                    let r = call_regs(0x00A9_3BC0, 0, 0, 0, esi as u32, 0, &[]) as u8;
                    if r != 0 {
                        bump(C::ZwMoved);
                        let mut single = false;
                        if rd(edi + 8) as usize == esi {
                            let n = rd(esi);
                            if n as usize == esi {
                                wd(esi, 0);
                                wd(esi + 4, 0);
                                wd(edi + 8, 0);
                                single = true;
                            } else {
                                wd(edi + 8, n);
                            }
                        }
                        if !single {
                            let p = rd(esi + 4) as usize;
                            if p != 0 {
                                wd(p, rd(esi));
                            }
                            let n = rd(esi) as usize;
                            if n != 0 {
                                wd(n + 4, rd(esi + 4));
                            }
                            wd(esi, 0);
                            wd(esi + 4, 0);
                        }
                        let h = rd(MOVED) as usize;
                        if h == 0 {
                            wd(esi, esi as u32);
                            wd(esi + 4, esi as u32);
                            wd(MOVED, esi as u32);
                        } else {
                            let t = rd(h + 4);
                            wd(esi + 4, t);
                            wd(esi, h as u32);
                            let t2 = rd(h + 4) as usize;
                            wd(t2, esi as u32);
                            wd(h + 4, esi as u32);
                        }
                    } else {
                        bump(C::ZwStay);
                        let r2 = call_regs(0x00A9_3D30, esi as u32, 0, 0, 0, 0, &[]) as u8;
                        wb(esi + 0x10, r2);
                        wb(edi + 0xC, rb(edi + 0xC) & r2);
                    }
                    esi = ebp;
                }
                // a940bb: compare the new flag with the old one
                let cur = rb(edi + 0xC);
                let prev = rb(edi + 0xD);
                if cur != 0 {
                    if prev == 0 {
                        let f = rd(edi + 0x10) as usize;
                        if f != 0 {
                            bump(C::ZwEnter);
                            call_regs(f, 0, 0, 0, 0, 0, &[edi as u32]);
                        }
                    }
                } else if prev != 0 {
                    let f = rd(edi + 0x14) as usize;
                    if f != 0 {
                        bump(C::ZwLeave);
                        call_regs(f, 0, 0, 0, 0, 0, &[edi as u32]);
                    }
                } else {
                    let mut m = rd(edi + 8) as usize;
                    if m != 0 {
                        loop {
                            if rb(m + 0x10) != 0 && rb(m + 0x11) == 0 {
                                let f = rd(edi + 0x18) as usize;
                                if f != 0 {
                                    bump(C::ZwMember);
                                    call_regs(f, 0, 0, 0, 0, 0, &[m as u32]);
                                }
                            }
                            m = rd(m) as usize;
                            if m == rd(edi + 8) as usize || m == 0 {
                                break;
                            }
                        }
                    }
                }
            }
            edi = rd(edi) as usize;
            if edi == rd(G) as usize || edi == 0 {
                break;
            }
        }
    }
}

// --- persona_events_update (0x6ADB10) ---------------------------------------------------------

kxs0!(persona_thunk, persona_events);

extern "C" fn persona_events() {
    unsafe {
        bump(C::PeCalls);
        const HEAD: usize = 0x0145_0404; // pending timers
        const DONE: usize = 0x0145_0400; // fired timers
        const NOW: usize = 0x00E8_43AC;
        call(0x006A_C5A0);
        call(0x00A5_6710);
        if rb(0x0252_7CCE) != 0 {
            let al = call_regs(0x00A5_7160, 0, 0, 0, 0, 0, &[rd(0x0217_03D4)]) as u8;
            if al == 0 {
                wb(0x0252_7CCE, al);
            }
        }
        let mut now = rd(NOW) as i32;
        let mut esi = rd(HEAD) as usize;
        while esi != 0 {
            let head = rd(HEAD);
            let nx = rd(esi + 0x14);
            let edi = if nx != head { nx as usize } else { 0 };
            let t = rd(esi + 0x10) as i32;
            if t < 0 || !time_due(t, now) {
                esi = edi;
                continue;
            }
            bump(C::PeTimers);
            let cl = rb(esi + 0xE) as u32;
            let fl = f32rt(esi + 8);
            let w = rw(esi + 0xC) as u32;
            call_regs(0x006A_E180, rd(esi), cl, w, esi as u32, edi as u32, &[rd(esi + 4), fl.to_bits(), w, cl]);
            // unlink from the pending list ...
            let mut skip = false;
            if rd(HEAD) as usize == esi {
                let n = rd(esi + 0x14);
                if n as usize == esi {
                    wd(esi + 0x14, 0);
                    wd(esi + 0x18, 0);
                    wd(HEAD, 0);
                    skip = true;
                } else {
                    wd(HEAD, n);
                }
            }
            if !skip {
                let p = rd(esi + 0x18) as usize;
                if p != 0 {
                    wd(p + 0x14, rd(esi + 0x14));
                }
                let n = rd(esi + 0x14) as usize;
                if n != 0 {
                    wd(n + 0x18, rd(esi + 0x18));
                }
                wd(esi + 0x14, 0);
                wd(esi + 0x18, 0);
            }
            // ... and append to the fired list
            let h = rd(DONE) as usize;
            if h == 0 {
                wd(esi + 0x14, esi as u32);
                wd(esi + 0x18, esi as u32);
                wd(DONE, esi as u32);
            } else {
                let tail = rd(h + 0x18);
                wd(esi + 0x18, tail);
                wd(esi + 0x14, h as u32);
                let tail2 = rd(h + 0x18) as usize;
                wd(tail2 + 0x14, esi as u32);
                wd(h + 0x18, esi as u32);
            }
            now = rd(NOW) as i32;
            esi = edi;
        }

        // Deferred calls keyed by a 16-bit id through a hash table.
        let mut esi = rd(0x027D_8724) as usize;
        let mut ebx = rd(0x0212_E258) as usize;
        while esi != 0 {
            let eax = rd(esi + 0x10);
            let ebp = if eax != rd(0x027D_8724) { eax as usize } else { 0 };
            let ax = rw(esi);
            if ax != 0xFFFF {
                let edi = ax as u32;
                let mut e = edi << 15;
                e = !e;
                e = e.wrapping_add(edi);
                let mut c = e;
                c = ((c as i32) >> 10) as u32;
                c ^= e;
                c = c.wrapping_add(c.wrapping_mul(8)); // lea ecx,[ecx+ecx*8]
                let d = ((c as i32) >> 6) as u32;
                c ^= d;
                let mut e2 = c << 11;
                e2 = !e2;
                c = c.wrapping_add(e2);
                let mut e3 = ((c as i32) >> 16) as u32;
                e3 ^= c;
                let rem: i32;
                std::arch::asm!(
                    "cdq",
                    "idiv dword ptr [{d}]",
                    d = in(reg) ebx + 0x60,
                    inout("eax") e3 => _,
                    out("edx") rem,
                );
                let tbl = rd(ebx + 0x5C);
                let mut node = rd(tbl.wrapping_add((rem as u32).wrapping_mul(4)) as usize) as usize;
                let mut found = false;
                while node != 0 {
                    if edi == rd(node + 0xC) {
                        found = true;
                        break;
                    }
                    node = rd(node + 4) as usize;
                }
                if found {
                    let n = rd(node) as usize;
                    if n != 0 {
                        let handle = rd(n + 4);
                        if handle != 0 {
                            bump(C::PeLookups);
                            let s = slot(handle);
                            let obj = if s != 0 && rd(s + 0x44) == handle && rd(s + 0x4C) == 1 { s as u32 } else { 0 };
                            let w8 = rw(esi + 8) as u32;
                            bump(C::PeDeferred);
                            call_regs(
                                0x006A_F190,
                                obj,
                                rd(esi + 4),
                                w8,
                                esi as u32,
                                edi,
                                &[obj, rd(esi + 4), w8, rd(esi + 0xC)],
                            );
                            call_regs(0x006B_0000, 0, 0, 0, esi as u32, 0x027D_8718, &[]);
                            ebx = rd(0x0212_E258) as usize;
                        }
                    }
                }
            }
            esi = ebp;
        }
    }
}

// --- nav_detours_update (0xA0AFF0) ------------------------------------------------------------

kzf_thunk0!(nav_detours_thunk, nav_detours, 0xC);

extern "C" fn nav_detours(out: *mut [u32; 7], x: usize) -> u32 {
    unsafe {
        bump(C::NdCalls);
        const H: usize = 0x021F_2C08;
        let [e_c, e_d, _e_a, e_b, e_s, _e_i, e_p] = *out;
        let mut ecx = e_c;
        let mut edx;
        let mut esi = e_s;
        let mut ebp = e_p;
        let mut eax = rd(H);
        edx = 0u32;
        let mut edi = eax;
        let mut ebx = e_b;
        let outp = x + 4; // the original's local at [esp + 0x14]
        let _ = e_d;
        macro_rules! callx {
            ($f:expr, $a:expr, $c:expr, $args:expr) => {{
                let r = call_xr($f, $a, $c, edx, ebx, esi, edi, ebp, $args);
                eax = r.0;
                ecx = r.1;
                edx = r.2;
            }};
        }
        loop {
            if edi == 0 {
                break;
            }
            ecx = rd(edi as usize);
            ebx = if ecx != eax { ecx } else { 0 };
            ecx = edx;
            edx = edx.wrapping_add(1);
            let counter1 = edx;
            if (ecx as i32) > 0x96 {
                // Rotate the circular list so that this record becomes the head.
                bump(C::NdRotate);
                if eax == 0 {
                    edi = eax;
                    wd(H, edi);
                    break;
                }
                if eax == edi {
                    edi = eax;
                    eax = 0;
                } else {
                    edx = rd(edi as usize + 4);
                    ecx = rd(eax as usize + 4);
                    wd(edx as usize, eax);
                    edx = rd(edi as usize + 4);
                    wd(eax as usize + 4, edx);
                    wd(edi as usize + 4, ecx);
                    wd(ecx as usize, edi);
                }
                if edi == 0 {
                    edi = eax;
                    wd(H, edi);
                    break;
                }
                if eax == 0 {
                    wd(H, edi);
                    break;
                }
                ecx = rd(edi as usize + 4);
                wd(ecx as usize, eax);
                edx = rd(eax as usize + 4);
                wd(edx as usize, edi);
                edx = rd(edi as usize + 4);
                ecx = rd(eax as usize + 4);
                wd(eax as usize + 4, edx);
                wd(edi as usize + 4, ecx);
                wd(H, edi);
                break;
            }
            let ty = rd(edi as usize + 0x10) as i32;
            // validity of the target handle, with the registers the original leaves behind
            let mut valid = false;
            eax = ty as u32;
            match ty {
                0 => {
                    eax = rd(edi as usize + 0xC);
                    if eax != 0 {
                        edx = (eax & 0xFFF) << 4;
                        ecx = rd(edx as usize + 0x0214_9C64);
                        if ecx != 0 && rd(ecx as usize + 0x44) == eax {
                            valid = true;
                        }
                    }
                }
                1 => {
                    eax = 0;
                    ecx = rd(edi as usize + 0xC);
                    if ecx != 0 {
                        eax = (ecx & 0xFFFF).wrapping_mul(0x134).wrapping_add(0x02AE_50E0);
                        if rd(eax as usize + 8) == ecx {
                            valid = true;
                        }
                    }
                }
                2 => {
                    eax = 0;
                    ecx = rd(edi as usize + 0xC);
                    if ecx != 0 {
                        eax = (ecx & 0x1FF).wrapping_mul(0x208).wrapping_add(0x0267_4DE0);
                        if rd(eax as usize + 0x10) == ecx {
                            valid = true;
                        }
                    }
                }
                _ => {
                    eax = (ty as u32).wrapping_sub(2);
                }
            }
            if !valid {
                bump(C::NdInvalid);
                ecx = rd(edi as usize + 8);
                callx!(0x00A0_E0C0, eax, ecx, &[]);
                eax = rd(H);
                edx = counter1;
                edi = ebx;
                continue;
            }
            callx!(0x00A0_9B10, edi, ecx, &[outp as u32]);
            let o = rd(outp);
            if eax as u8 != 0 {
                bump(C::NdActive);
                if rd(edi as usize + 0x18) != 0 {
                    ebp = o;
                    esi = edi;
                    callx!(0x00A0_9FE0, eax, ecx, &[o]);
                    if eax as u8 == 0 {
                        eax = rd(H);
                        edx = counter1;
                        edi = ebx;
                        continue;
                    }
                    callx!(0x00A0_AED0, eax, ecx, &[]);
                    callx!(0x00A0_A760, eax, ecx, &[edi, ebp]);
                    eax = rd(H);
                    edx = counter1;
                    edi = ebx;
                    continue;
                }
                ecx = o;
                callx!(0x00A0_A760, eax, ecx, &[edi, o]);
                eax = rd(H);
                edx = counter1;
                edi = ebx;
                continue;
            }
            bump(C::NdInactive);
            if rd(edi as usize + 0x18) != 0 {
                ecx = rd(edi as usize + 0xC);
                let mut ok = false;
                if ecx != 0 {
                    edx = (ecx & 0xFFF) << 4;
                    eax = rd(edx as usize + 0x0214_9C64);
                    ok = eax != 0 && rd(eax as usize + 0x44) == ecx && rd(eax as usize + 0x4C) == 1;
                }
                if ok {
                    ecx = rd(edi as usize + 8);
                    callx!(0x00A0_E1D0, eax, ecx, &[]);
                } else {
                    callx!(0x00A0_AED0, eax, ecx, &[]);
                }
            }
            eax = rd(H);
            edx = counter1;
            edi = ebx;
        }
        (*out)[0] = ecx;
        (*out)[1] = edx;
        eax
    }
}

// --- vehicle_attached_effects_update (0x4D1D50) -----------------------------------------------

kxs0!(vae_thunk, vehicle_attached_effects);

extern "C" fn vehicle_attached_effects() {
    unsafe {
        bump(C::VaCalls);
        const HA: usize = 0x00F9_991C;
        const HB: usize = 0x00F9_9920;
        let mut ebx = rd(HA) as usize;
        while ebx != 0 {
            let nx = rd(ebx + 8);
            let ebp = if nx != rd(HA) { nx as usize } else { 0 };
            let h1 = rd(ebx);
            let mut copy_from = 0usize; // the human (ESI in the original)
            if h1 != 0 {
                let s = slot(h1);
                if s != 0 && rd(s + 0x44) == h1 && rd(s + 0x4C) == 5 && (rd(s + 0x8C28) as i32) > 0 {
                    let r = call_regs(0x00AA_B5E0, s as u32, 0, 0, 0, 0, &[]) as u8;
                    if r == 0 {
                        let r2 = call_regs(0x00AA_B6B0, 0, 0, 0, 0, s as u32, &[]) as u8;
                        if r2 == 0 {
                            copy_from = s;
                        }
                    }
                }
            }
            if copy_from != 0 {
                bump(C::VaCopy);
                let esi = copy_from;
                let mut buf = [0u32; 3];
                call_regs(0x004D_1A50, buf.as_mut_ptr() as u32, 0, 0, esi as u32, 0, &[]);
                let h2 = rd(ebx + 4);
                if h2 != 0 {
                    let q = slot(h2);
                    if q != 0 && rd(q + 0x44) == h2 && rd(q + 0x4C) == 6 {
                        let list = rd(q + 0xC4) as usize;
                        if list != 0 {
                            let mut e = list;
                            loop {
                                wd(e + 0x2C, buf[0]);
                                wd(e + 0x30, buf[1]);
                                wd(e + 0x34, buf[2]);
                                let nxt = rd(e) as usize;
                                copy_dwords(e + 0x38, esi + 0x20, 9);
                                e = nxt;
                                if e == rd(q + 0xC4) as usize || e == 0 {
                                    break;
                                }
                            }
                        }
                    }
                }
                ebx = ebp;
                continue;
            }
            let c = rd(ebx + 4);
            if c != 0 {
                let s = slot(c);
                let obj = if s != 0 && rd(s + 0x44) == c && rd(s + 0x4C) == 6 { s as u32 } else { 0 };
                call_regs(0x0050_AB60, 0, 0, 0, 0, obj, &[0, 0]);
            }
            // Move the node from list A to the tail of list B.
            bump(C::VaMoved);
            let mut single = false;
            if rd(HA) as usize == ebx {
                let n = rd(ebx + 8);
                if n as usize == ebx {
                    wd(HA, 0);
                    single = true;
                } else {
                    wd(HA, n);
                }
            }
            if !single {
                let p = rd(ebx + 0xC) as usize;
                if p != 0 {
                    wd(p + 8, rd(ebx + 8));
                }
                let n = rd(ebx + 8) as usize;
                if n != 0 {
                    wd(n + 0xC, rd(ebx + 0xC));
                }
            }
            wd(ebx + 0xC, 0);
            wd(ebx + 8, 0);
            let b = rd(HB) as usize;
            if b == 0 {
                wd(ebx + 8, ebx as u32);
                wd(ebx + 0xC, ebx as u32);
                wd(HB, ebx as u32);
            } else {
                let t = rd(b + 0xC);
                wd(ebx + 0xC, t);
                wd(ebx + 8, b as u32);
                let e = rd(b + 0xC) as usize;
                wd(e + 8, ebx as u32);
                wd(b + 0xC, ebx as u32);
            }
            ebx = ebp;
        }
    }
}

// --- ai_scheduled_tasks_tick (0x452820) -------------------------------------------------------

kz_thunk0!(tasks_tick_thunk, tasks_tick);

extern "C" fn tasks_tick(out: *mut [u32; 7]) -> u32 {
    unsafe {
        bump(C::TkCalls);
        const H: usize = 0x00EE_2C0C;
        let mut ecx = (*out)[0];
        let mut edx = (*out)[1];
        let mut esi = rd(H);
        let mut eax = esi;
        // Phase A: count down every due task.
        if esi != 0 {
            edx = rd(0x00E8_43AC);
            let edi = 0xFFFF_FFFFu32;
            loop {
                let t = rd(eax as usize + 0x14);
                if (t as i32) >= 0 {
                    ecx = t;
                    let fire = if (ecx as i32) > (edx as i32) {
                        ecx = ecx.wrapping_sub(edx);
                        (ecx as i32) >= 0x35A4_E900
                    } else {
                        (edx.wrapping_sub(ecx) as i32) < 0x35A4_E900
                    };
                    if fire {
                        bump(C::TkFire);
                        let cnt = rd(eax as usize + 0x10).wrapping_add(edi);
                        wd(eax as usize + 0x10, cnt);
                        if (cnt as i32) <= 0 {
                            wd(eax as usize + 0x14, edi);
                        } else {
                            ecx = edx.wrapping_add(0x29A);
                            wd(eax as usize + 0x14, ecx);
                            if (ecx as i32) > 0x6B49_D200 {
                                ecx = ecx.wrapping_add(0x94B6_2E00);
                                wd(eax as usize + 0x14, ecx);
                            }
                        }
                    }
                }
                eax = rd(eax as usize);
                if eax == esi || eax == 0 {
                    break;
                }
            }
        }
        // Phase B: re-bucket the list by [node+0x10] into 4 stack buckets (index unchecked, like the original).
        ecx = 0;
        edx = 0;
        let mut edi = 0u32;
        let mut ebx = 0u32;
        let mut bk = [0u32; 4];
        let bka = bk.as_mut_ptr() as usize;
        eax = esi;
        if esi != 0 {
            loop {
                edi = rd(esi as usize);
                if edi == esi {
                    wd(esi as usize, 0);
                    ecx = esi.wrapping_add(4);
                    esi = 0;
                } else {
                    edx = rd(eax as usize + 4);
                    ecx = eax.wrapping_add(4);
                    esi = edi;
                    if edx != 0 {
                        wd(edx as usize, edi);
                    }
                    edx = rd(eax as usize);
                    if edx != 0 {
                        edi = rd(ecx as usize);
                        wd(edx as usize + 4, edi);
                    }
                    wd(eax as usize, 0);
                }
                edi = rd(eax as usize + 0x10);
                let slot_a = bka.wrapping_add((edi as usize).wrapping_mul(4));
                edx = rd(slot_a);
                edi = slot_a as u32;
                wd(ecx as usize, 0);
                if edx != 0 {
                    edi = rd(edx as usize + 4);
                    wd(ecx as usize, edi);
                    wd(eax as usize, edx);
                    ecx = rd(edx as usize + 4);
                    wd(ecx as usize, eax);
                    edx = rd(eax as usize + 0x10);
                    ecx = rd(bka.wrapping_add((edx as usize).wrapping_mul(4)));
                    wd(ecx as usize + 4, eax);
                } else {
                    wd(eax as usize, eax);
                    wd(ecx as usize, eax);
                    wd(edi as usize, eax);
                }
                eax = esi;
                if esi == 0 {
                    break;
                }
            }
            ebx = rd(bka + 12);
            edi = rd(bka + 8);
            edx = rd(bka + 4);
            ecx = rd(bka);
        }
        // Phase C: concatenate b0..b3 and store the new head.
        eax = ecx;
        wd(H, eax);
        if ecx != 0 {
            if edx != 0 {
                esi = rd(ecx as usize + 4);
                wd(esi as usize, edx);
                esi = rd(edx as usize + 4);
                wd(esi as usize, ecx);
                esi = rd(edx as usize + 4);
                let ebp = rd(ecx as usize + 4);
                wd(edx as usize + 4, ebp);
                wd(ecx as usize + 4, esi);
            }
        } else {
            eax = edx;
            wd(H, eax);
        }
        if eax == 0 {
            eax = edi;
            wd(H, eax);
        } else if edi != 0 {
            edx = rd(eax as usize + 4);
            wd(edx as usize, edi);
            ecx = rd(edi as usize + 4);
            wd(ecx as usize, eax);
            edx = rd(eax as usize + 4);
            ecx = rd(edi as usize + 4);
            wd(edi as usize + 4, edx);
            wd(eax as usize + 4, ecx);
        }
        if eax == 0 {
            wd(H, ebx);
        } else if ebx != 0 {
            ecx = rd(eax as usize + 4);
            wd(ecx as usize, ebx);
            edx = rd(ebx as usize + 4);
            wd(edx as usize, eax);
            edx = rd(eax as usize + 4);
            ecx = rd(ebx as usize + 4);
            wd(ebx as usize + 4, edx);
            wd(eax as usize + 4, ecx);
        }
        (*out)[0] = ecx;
        (*out)[1] = edx;
        eax
    }
}

// --- explosions_flush_queue (0x4C2BF0) --------------------------------------------------------

kzfa_thunk0!(explosions_thunk, explosions_flush, 0x3B0C);

extern "C" fn explosions_flush(out: *mut [u32; 7], x: usize) -> u32 {
    unsafe {
        bump(C::ExCalls);
        const COUNT: usize = 0x0252_6D08;
        const QUEUE: usize = 0x0261_E418;
        const REC: usize = 0x1D8;
        // stack cookie, as in the original's prologue
        wd(x + 0x3B04, rd(0x00E8_0B50) ^ x as u32);
        let n = rd(COUNT) as usize;
        let mut edx = (*out)[1];
        if n != 0 {
            // the queue copy lives in the original's frame: buffer at x + 4, record i at x + 4 + i * 0x1D8
            let buf = x + 4;
            call_regs(0x00CA_0850, 0, 0, 0, 0, 0, &[buf as u32, ga(QUEUE), (n as u32).wrapping_mul(REC as u32)]);
            wd(COUNT, 0);
            let lookup = |h: u32| -> u32 {
                if h == 0 {
                    return 0;
                }
                let s = slot(h);
                if s != 0 && rd(s + 0x44) == h { s as u32 } else { 0 }
            };
            for i in 0..n {
                bump(C::ExRecords);
                let r = buf.wrapping_add(i.wrapping_mul(REC));
                let (edi, ebp) = if rb(r + 0x41) == 1 {
                    let o = lookup(rd(r + 0x48));
                    (r + 0x44, o)
                } else {
                    (0usize, 0u32)
                };
                let edx_l = lookup(rd(r + 8));
                let eax = lookup(rd(r + 4));
                edx = rd(r);
                call_regs(
                    0x004C_2600,
                    eax,
                    (r + 0xC) as u32,
                    edx,
                    0,
                    0,
                    &[
                        edx,
                        eax,
                        edx_l,
                        (r + 0xC) as u32,
                        (r + 0x30) as u32,
                        rd(r + 0x3C),
                        0,
                        rb(r + 0x40) as u32,
                        edi as u32,
                        ebp,
                        rd(r + 0x1D0),
                        0,
                        1,
                        rb(r + 0x1D5) as u32,
                    ],
                );
            }
        }
        let ecx = rd(x + 0x3B04) ^ x as u32;
        let eax = call_regs(0x00C9_604F, 0, ecx, 0, 0, 0, &[]);
        (*out)[0] = ecx;
        (*out)[1] = edx;
        eax
    }
}

// --- wiretap_triggers_update (0x54CCA0) -------------------------------------------------------

kz_thunk0!(wiretap_thunk, wiretap);

extern "C" fn wiretap(out: *mut [u32; 7]) -> u32 {
    unsafe {
        bump(C::WtCalls);
        let [e_c, e_d, _e_a, e_b, e_s, e_i, e_p] = *out;
        let f = rb(0x00FA_239B);
        let a = f & 1 != 0;
        let c = (f >> 2) & 1 != 0;
        let d = (f >> 3) & 1 != 0;
        let e = (f >> 4) & 1 != 0;
        let ebx = (e_b & !0xFF) | c as u32;
        let mut ecx = (e_c & !0xFF) | d as u32;
        let mut edx = e_d;
        let (mut eax, c1, d1) = call_xr(0x00A6_C870, 0x00DE_44A8, ecx, edx, ebx, e_s, e_i, e_p, &[]);
        let mut esi = eax;
        let r = call_xr(0x00A6_C870, 0x00DE_44D0, c1, d1, ebx, esi, e_i, e_p, &[]);
        let mut edi = r.0;
        let r = call_xr(0x00A6_C870, 0x00DE_44F8, r.1, r.2, ebx, esi, edi, e_p, &[]);
        let mut ebp = r.0;
        let r = call_xr(0x00A6_C870, 0x00DE_4520, r.1, r.2, ebx, esi, edi, ebp, &[]);
        let h4 = r.0;
        edx = h4;
        ecx = r.1;
        let v = rd(0x00E9_DA40);
        eax = v;
        let mut k = false;
        if (v & 0xFF) != 0xFF {
            ecx = v >> 8;
            if ecx != 0 {
                eax = (v & 0xFF).wrapping_mul(0x19C);
                k = ecx == rd(eax as usize + 0x00EE_B9AC) && rd(eax as usize + 0x00EE_B8F8) != 0xFFFF_FFFF;
            }
        }
        if esi == 0 || edi == 0 || ebp == 0 || edx == 0 {
            bump(C::WtSkip);
            (*out)[0] = ecx;
            (*out)[1] = edx;
            return eax;
        }
        macro_rules! cx {
            ($f:expr, $a:expr, $s:expr) => {{
                let r = call_xr($f, $a, ecx, edx, ebx, $s, edi, ebp, &[]);
                eax = r.0;
                ecx = r.1;
                edx = r.2;
            }};
        }
        if !a && !k {
            cx!(0x00A6_BCE0, eax, esi);
        } else {
            cx!(0x00A6_BD60, esi, esi);
        }
        if !c && !k {
            esi = edi;
            cx!(0x00A6_BCE0, eax, esi);
        } else {
            cx!(0x00A6_BD60, edi, esi);
        }
        if !d && !k {
            esi = ebp;
            cx!(0x00A6_BCE0, eax, esi);
        } else {
            cx!(0x00A6_BD60, ebp, esi);
        }
        let _ = &mut edi;
        let _ = &mut ebp;
        if e {
            cx!(0x00A6_BD60, h4, esi);
            eax = (eax & !0xFF) | 1;
        } else if a && c && d && !k {
            esi = h4;
            cx!(0x00A6_BCE0, eax, esi);
            eax &= !0xFF;
        } else {
            cx!(0x00A6_BD60, h4, esi);
            eax &= !0xFF;
        }
        // tail jump to 0x789570 with the entry callee-saved registers restored
        let r = call_xr(0x0078_9570, eax, ecx, edx, e_b, e_s, e_i, e_p, &[]);
        (*out)[0] = r.1;
        (*out)[1] = r.2;
        r.0
    }
}

// --- cellphone_do_frame (0xA66340) ------------------------------------------------------------

kxs0!(cellphone_thunk, cellphone_do_frame);

extern "C" fn cellphone_do_frame() {
    unsafe {
        bump(C::CpCalls);
        const CNT_: usize = 0x0222_3CB4;
        let mut flag = 0u8;
        wb(0x0213_B396, 0);
        let mut ebp: i32 = 0;
        let mut edi: usize = 0x02F3_5250;
        let mut ebx: usize = 0x02F3_5268;
        if (rd(CNT_) as i32) <= 0 {
            return;
        }
        // The cellphone-state gate shared by three places in the loop body.
        let gate = || -> bool {
            let e = rd(0x0252_8C14);
            e != 0 && (rb(0x0252_8B56) == 0 || rb(0x0252_8B2B) != 0) && rb(e as usize + 0x3F8) != 0
        };
        loop {
            let mut skip = false;
            if rb(edi + 0x3C) & 4 != 0 {
                if flag == 1 {
                    skip = true;
                } else {
                    let eax = call_regs(0x00A6_5810, 0, 0, 0, 0, 0, &[edi as u32]);
                    if eax == 1 || rb(edi + 0x3C) & 8 != 0 {
                        flag = 1;
                    }
                    wb(0x0213_B396, (rb(edi + 0x3C) >> 3) & 1);
                    if eax != 2 {
                        skip = true;
                    } else {
                        call(0x0078_2E10);
                        let e = rd(0x0252_8C14);
                        if e != 0
                            && call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[e]) as u8 != 0
                            && call_regs(0x007F_7AD0, 0, 0, 0, 0, 0, &[]) as u8 != 0
                            && rb(edi + 0x3C) & 0x80 == 0
                        {
                            skip = true;
                        }
                    }
                }
            } else {
                let v = rd(edi);
                let lo = (v & 0xFF) as usize;
                let direct = lo == 0xFF
                    || (v >> 8) == 0
                    || (v >> 8) != rd(0x00EE_B9AC + lo * 0x19C)
                    || rd(0x00EE_B8F8 + lo * 0x19C) == 0xFFFF_FFFF;
                if !direct {
                    let esi = rd(edi + 0xC);
                    if esi == 0 {
                        skip = true;
                    } else {
                        let a = call_regs(0x0098_5630, 0, 0, 0, 0, 0, &[esi]) as u8;
                        let ok = a != 0 || call_regs(0x0096_0C30, 0, 0, 0, esi, 0, &[]) as u8 != 0;
                        if !ok {
                            skip = true;
                        } else {
                            call_regs(0x0047_FEB0, 0, edi as u32, 0, 0, 0, &[]);
                        }
                    }
                }
            }
            if !skip {
                if rb(edi + 0x3C) & 4 != 0 {
                    let e = rd(0x0252_8C14);
                    if e != 0
                        && call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[e]) as u8 == 0
                        && call_regs(0x007F_7AD0, 0, 0, 0, 0, 0, &[]) as u8 != 0
                    {
                        call_regs(0x00A6_6230, 0, edi as u32, 0, 0, 0, &[]);
                    }
                }
                let cb = rd(edi + 0x34) as usize;
                if cb != 0 {
                    call_regs(cb, 0, 0, 0, 0, 0, &[edi as u32, rd(edi + 0x38)]);
                }
                if gate() {
                    let a = rd(edi + 0x14);
                    if a != 0xFFFF_FFFF {
                        let p = ((a >> 16) << 4).wrapping_add(0x02E7_A9C0);
                        if p != 0 {
                            let x = rd(p as usize + 0xC);
                            call_regs(0x00CE_01E0, rd(0x0252_983C), 0, 0, 0, 0, &[x, 0, 0, 0]);
                        }
                    }
                } else if rd(edi + 0x14) != 0xFFFF_FFFF {
                    call_regs(0x00A5_5360, 0, 0, 0, 0, 0, &[4, ebx as u32, 1]);
                }
                if gate() {
                    let a = rd(edi + 0x10);
                    if a != 0xFFFF_FFFF {
                        call_regs(0x00A2_6420, a, 0, 0, 0, 0, &[]);
                    }
                }
                wd(edi + 0x10, 0xFFFF_FFFF);
                if gate() {
                    let a = rd(edi + 0x14);
                    if a != 0xFFFF_FFFF {
                        call_regs(0x00A2_6420, a, 0, 0, 0, 0, &[]);
                    }
                }
                let rest = (rd(CNT_) as i32).wrapping_sub(ebp).wrapping_sub(1);
                wd(edi + 0x14, 0xFFFF_FFFF);
                if rest > 0 {
                    call_regs(0x00C9_6060, 0, 0, 0, 0, 0, &[edi as u32, (edi + 0x48) as u32, (rest as u32).wrapping_mul(0x48)]);
                    ebp -= 1;
                    edi -= 0x48;
                    ebx -= 0x48;
                }
                wd(CNT_, rd(CNT_).wrapping_sub(1));
                bump(C::CpRemoved);
            }
            ebp += 1;
            edi += 0x48;
            ebx += 0x48;
            if !(ebp < rd(CNT_) as i32) {
                break;
            }
        }
    }
}

// --- havok_triggers_update (0x71B910) ---------------------------------------------------------
//
// The original reserves a 0x39BC-byte frame: a stack container (vtable 0xE18EF0, 16 inline
// 16-byte entries), a 1024-bit set, a record array of 12-byte entries and a u16 work list. The
// frame here has the same size and relative layout, so the pointers handed to the original
// callees point at equivalent memory. Checked with per-branch counters only (the callees have
// side effects: trigger enter / leave notifications).

const HV_FRAME: usize = 0x39C0;
// Offsets inside the frame (original [esp+X] with the four saved registers pushed is X - 0x10).
const HV_BYTE_IDX: usize = 0x0C; // u16 out of 0x71AEA0
const HV_LOOKUP: usize = 0x10; // pair out of 0x7031C0
const HV_OBJ: usize = 0x18; // the container object
const HV_INLINE: usize = 0x2C; // its inline storage
const HV_RECS: usize = 0x130; // records, 12 bytes each
const HV_BITS: usize = 0x3130; // 1024-bit set, 0x80 bytes
const HV_LIST: usize = 0x31B0; // u16 work list
const HV_LIST_MAX: usize = 1000; // the original has room for ~1026 before the frame cookie

/// `bits[idx >> 3] |= 1 << (idx & 7)`. Ids from this table stay below 1024; a larger one would
/// write past the set in the original, so it is ignored here (counted).
unsafe fn hv_set_bit(b: usize, idx: u32) {
    if idx >= 1024 {
        bump(C::HkBitOob);
        return;
    }
    let p = b + HV_BITS + (idx >> 3) as usize;
    wb(p, rb(p) | (1u8 << (idx & 7)));
}

unsafe fn hv_test_bit(b: usize, idx: u32) -> bool {
    if idx >= 1024 {
        return false;
    }
    rb(b + HV_BITS + (idx >> 3) as usize) & (1u8 << (idx & 7)) != 0
}

kxs0!(havok_thunk, havok_triggers_update);

extern "C" fn havok_triggers_update() {
    unsafe {
        bump(C::HkCalls);
        let mut frame = std::mem::MaybeUninit::<[u32; HV_FRAME / 4]>::uninit();
        let b = frame.as_mut_ptr() as usize;
        let obj = b + HV_OBJ;
        const HEAD: usize = 0x0297_19BC;
        let mut n: u32 = 0; // records written so far
        let mut node = rd(HEAD) as usize;
        if node != 0 {
            loop {
                bump(C::HkNodes);
                // Construct the container on the frame.
                wd(obj, 0x00E1_8EF0);
                wb(obj + 4, 0);
                wd(obj + 8, (b + HV_INLINE) as u32);
                wd(obj + 0xC, 0);
                wd(obj + 0x10, 0x8000_0010);
                // Virtual method 15 of the node's object fills it: ECX = object, EDX = the
                // method itself, args (container, 0).
                let this = rd(node);
                let method = rd(rd(this as usize) as usize + 0x3C);
                call_regs(method as usize, obj as u32, this, method, n, node as u32, &[obj as u32, 0]);
                std::ptr::write_bytes((b + HV_BITS) as *mut u8, 0, 0x80);
                let count = rd(obj + 0xC);
                let mut off = 0usize;
                for _ in 0..count {
                    bump(C::HkElems);
                    let data = rd(obj + 8) as usize;
                    let elem = rd(data + off + 8) as usize;
                    off += 0x10;
                    let ecx = (elem as i32).wrapping_add(rb(elem + 0x10) as i8 as i32) as u32 as usize;
                    let ecount = rd(ecx + 0x78) as i32;
                    if ecount <= 0 {
                        bump(C::HkNoEntries);
                        continue;
                    }
                    let entries = rd(ecx + 0x74) as usize;
                    let has = |ty: u32| (0..ecount as usize).any(|k| rd(entries + k * 0x10) == ty);
                    if !has(1) {
                        bump(C::HkNoType1);
                        continue;
                    }
                    if has(6) {
                        bump(C::HkType6);
                        continue;
                    }
                    call_regs(0x0070_31C0, (b + HV_LOOKUP) as u32, ecx as u32, 0, 0, node as u32, &[1]);
                    let esi = rd(b + HV_LOOKUP);
                    if esi == 0 {
                        bump(C::HkLookupNull);
                        continue;
                    }
                    wd(b + HV_BYTE_IDX, 0xFFFF);
                    let hit = call_regs(0x0071_AEA0, 0, ecx as u32, 0, esi, node as u32, &[node as u32, esi, (b + HV_BYTE_IDX) as u32]) as u8;
                    if hit == 1 {
                        bump(C::HkDirect);
                        hv_set_bit(b, *((b + HV_BYTE_IDX) as *const u16) as u32);
                        continue;
                    }
                    let id = call_regs(0x0071_AFA0, 0, ecx as u32, node as u32, esi, node as u32, &[esi]) as u16 as u32;
                    bump(C::HkHashed);
                    hv_set_bit(b, id);
                    let flags = rb(node + 0xA);
                    if flags & 2 != 0 && flags & 1 != 0 {
                        bump(C::HkSkipFlags);
                        continue;
                    }
                    let rec = b + HV_RECS + n as usize * 12;
                    n += 1;
                    wd(rec, rd(node + 4));
                    wd(rec + 4, esi);
                    wb(rec + 8, 1);
                    wb(rec + 9, (flags & 4 == 4) as u8);
                    bump(C::HkRecPrimary);
                }
                // Walk the node's id chain (a tree of u16 ids in the table at 0x1F6D440).
                let first = *((node + 8) as *const u16);
                let list = b + HV_LIST;
                let mut len = 0usize;
                if first != 0xFFFF {
                    *(list as *mut u16) = first;
                    len = 1;
                    let mut i = 0usize;
                    loop {
                        let id = *((list + i * 2) as *const u16) as usize;
                        let entry = 0x01F6_D440 + id * 8;
                        let a = *((entry + 4) as *const u16);
                        if a != 0xFFFF && len < HV_LIST_MAX {
                            *((list + len * 2) as *mut u16) = a;
                            len += 1;
                        }
                        let c = *((entry + 6) as *const u16);
                        if c != 0xFFFF && len < HV_LIST_MAX {
                            *((list + len * 2) as *mut u16) = c;
                            len += 1;
                        }
                        i += 1;
                        if i >= len {
                            break;
                        }
                    }
                }
                for k in 0..len {
                    let id = *((list + k * 2) as *const u16) as u32;
                    if hv_test_bit(b, id) {
                        bump(C::HkChainSeen);
                        continue;
                    }
                    let key = rd(0x01F6_D440 + id as usize * 8);
                    call_regs(0x0071_B170, 0, 0, 0, key, node as u32, &[node as u32, key]);
                    let flags = rb(node + 0xA);
                    let rec = b + HV_RECS + n as usize * 12;
                    n += 1;
                    wd(rec, rd(node + 4));
                    wd(rec + 4, key);
                    wb(rec + 8, 0);
                    wb(rec + 9, (flags & 4 == 4) as u8);
                    bump(C::HkRecChain);
                }
                // Destroy the container: free its heap block if it owns one.
                let cap = rd(obj + 0x10) as i32;
                wb(node + 0xA, rb(node + 0xA) & 0xFD);
                wd(obj, 0x00E1_8EF0);
                if cap >= 0 {
                    bump(C::HkFree);
                    let size = ((cap as u32) & 0x3FFF_FFFF) << 4;
                    let heap = call_regs(rd(0x00DB_807C) as usize, 0, 0, 0, 0, 0, &[rd(0x031B_F2BC)]);
                    call_regs(0x00D2_B7D0, 0, heap, 0, size, 0, &[rd(obj + 8), size, 0x17]);
                }
                node = rd(node + 0xC) as usize;
                if node == rd(HEAD) as usize || node == 0 {
                    break;
                }
            }
        }
        for j in 0..n {
            let rec = b + HV_RECS + j as usize * 12;
            call_regs(0x0071_B300, 0, 0, rec as u32, rec as u32, n - j, &[]);
            bump(C::HkNotify);
        }
    }
}

// --- cheats_do_frame (0x687B40) ----------------------------------------------------------------
//
// The original pushes ECX (uninitialised) as a call argument at 0x687C3E, but overwrites that
// stack slot with [0x33D630C] two instructions later, so the value never reaches a callee.
// Checked with per-branch counters only.

/// The "bool" the original pushes for 0xA79240: `AL = (0x80FCF0(ptr) != 0)` with the rest of
/// EAX being what the callee left there (EAX is 0 when the pointer is null).
unsafe fn cheat_flag() -> u32 {
    let p = rd(0x0252_8C14);
    if p == 0 {
        return 0;
    }
    let r = call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[p]);
    if r as u8 != 0 { (r & !0xFF) | 1 } else { r & !0xFF }
}

kxs0!(cheats_thunk, cheats_do_frame);

extern "C" fn cheats_do_frame() {
    unsafe {
        bump(C::ChCalls);
        call(0x0068_75B0);
        call(0x0068_5FC0);
        if rb(0x00E8_E0C3) != 0 && (rd(0x0252_7C08) as i32) <= 0 {
            bump(C::ChActive);
            let players = [rd(0x0217_03D4), call_regs(0x009D_7BD0, 0, 0, 0, 0, 0, &[])];
            for i in 0..2usize {
                let slot_addr = 0x0252_7B64 + i * 4;
                let edi = rd(slot_addr);
                let esi = players[i];
                if edi == 0 {
                    // Nothing pending: start a pending-action if this human asks for it.
                    if esi == 0 || rb(esi as usize + 0x1161) & 0x40 == 0 {
                        continue;
                    }
                    if call_regs(0x0096_F3F0, 0, 0, 0, esi, 0, &[]) != 0xFFFF_FFFF {
                        continue;
                    }
                    bump(C::ChStart);
                    let edi2 = rd(rd(rd(esi as usize + 0xD4) as usize + 8) as usize + 0x68);
                    call_regs(0x009E_18F0, 0, 0, 0, esi, 0, &[0]);
                    let flag = cheat_flag();
                    call_regs(0x00A7_9240, 0, 0, edi2, 0, 0, &[4, 0, flag]);
                    continue;
                }
                if esi == 0 {
                    bump(C::ChCancel);
                    let flag = cheat_flag();
                    let target = rd(rd(edi as usize + 8) as usize + 0x68);
                    call_regs(0x00A7_9240, 0, 0, target, 0, 0, &[4, 0, flag]);
                    wd(slot_addr, 0);
                    continue;
                }
                let ebp = rd(esi as usize + 0xD4);
                if ebp == edi {
                    bump(C::ChSame);
                    continue;
                }
                if call_regs(0x0096_F3F0, 0, 0, 0, esi, 0, &[]) != 0xFFFF_FFFF {
                    bump(C::ChBusy);
                    continue;
                }
                let t = rd(rd(edi as usize + 8) as usize + 0x68);
                if call_regs(0x00A7_96D0, 0, 0, t, 0, 0, &[]) as u8 == 0 {
                    bump(C::ChNoMatch);
                    continue;
                }
                if rb(esi as usize + 0x1161) & 0x40 != 0 {
                    let flag = cheat_flag();
                    let target = rd(rd(ebp as usize + 8) as usize + 0x68);
                    call_regs(0x00A7_9240, 0, 0, target, 0, 0, &[4, 0, flag]);
                }
                bump(C::ChSwitch);
                let cur = rd(slot_addr);
                call_regs(0x009E_1600, 0, rd(cur as usize + 4), cur, 0, 0, &[esi, rd(cur as usize + 4), rd(0x033D_630C), 0]);
            }
        }
        let bl = call_regs(0x007F_7AD0, 0, 0, 0, 0, 0, &[]) as u8;
        if rb(0x0252_7B5B) == 0 && bl != 0 {
            let mut i = 0i32;
            while i < rd(0x0252_7B5C) as i32 {
                let esi = 0x027A_6AC0 + i as usize * 0x68;
                if rb(esi - 0x55) != 0 && rb(esi - 0x57) != 0 {
                    let f = rd(esi);
                    if f != 0 && (rb(esi - 0x58) == 0 || rb(esi - 0x57) != 0) {
                        bump(C::ChCallback);
                        call_regs(f as usize, f, (esi + 8) as u32, 0, 0, 0, &[(esi + 8) as u32, 1]);
                        wb(esi - 0x57, 0);
                    }
                }
                i += 1;
            }
        }
        wb(0x0252_7B5B, bl);
    }
}

// --- rich_presence_update (0x69A440) -----------------------------------------------------------
//
// Once a second, picks the presence state (0..10) for the current game mode and stores it in
// 0x347AE84 / 0x212E2C4, with extra counters for state 7 (rank) and states 8 (two scores).
// Jump tables decoded from the exe: 0x69A774 (14 entries, by mission-type 0..0xD) and 0x69A7AC
// (5 entries, by 0x6A7F30() + 1). Checked with per-branch counters only.

const RP_STATE: usize = 0x0347_AE84;
const RP_MODE_FLAG: usize = 0x0347_AE88;
const RP_RANK_ID: usize = 0x0347_AE8C;
const RP_RANK: usize = 0x0347_AE90;
const RP_SCORE_A: usize = 0x0347_AE94;
const RP_SCORE_B: usize = 0x0347_AE98;
const RP_LEAD: usize = 0x0347_AE9C;

/// The tail every path ends on: store the new state unless it equals the old one.
unsafe fn rp_commit(old: u32, new: u32) {
    if old != new {
        bump(C::RpChanged);
        wd(0x0212_E2C4, new);
        wd(RP_STATE, new);
    }
}

/// First node of the circular list at `head` whose `[node+4]` equals `value`, or 0.
unsafe fn rp_find(head: u32, value: u32) -> u32 {
    let mut n = head;
    while n != 0 {
        if rd(n as usize + 4) == value {
            return n;
        }
        n = rd(n as usize + 0xC);
        if n == head {
            break;
        }
    }
    0
}

kxs0!(rich_thunk, rich_presence_update);

extern "C" fn rich_presence_update() {
    unsafe {
        bump(C::RpCalls);
        let now = rd(0x00E8_43B4) as i32;
        if rb(0x0348_FFAC) & 1 == 0 {
            wd(0x0348_FFAC, rd(0x0348_FFAC) | 1);
        } else {
            let t = rd(0x0348_FFA8) as i32;
            if t >= 0 && !time_due(t, now) {
                return;
            }
        }
        wd(0x0348_FFA8, time_add(now, 1000) as u32);
        bump(C::RpRun);
        let id = rd(0x00E9_8DB0);
        wb((id as usize).wrapping_add(0x0347_AEA8), 0);
        if id == 0xFFFF_FFFF {
            bump(C::RpNoPlayer);
            for k in 0..6 {
                wd(RP_STATE + k * 4, 0);
            }
            return;
        }
        if rb(0x0252_8B48) != 1 || rb(0x0252_8B49) != 0 {
            if call_regs(0x009D_7BD0, 0, 0, 0, 0, 0, &[]) != 0 {
                if rd(RP_MODE_FLAG) != 1 {
                    wd(RP_MODE_FLAG, 1);
                }
            } else if rd(RP_MODE_FLAG) != 0 {
                wd(RP_MODE_FLAG, 0);
            }
        }
        let old = rd(RP_STATE);
        let activity = rd(0x00E9_6B7C) as i32;
        // Step 1: a running activity decides the state outright.
        if activity > -1 {
            let kind = rd(0x0144_E760 + activity as usize * 4);
            match kind {
                6 => {
                    if rd(0x00E9_6C00) != 6 {
                        return;
                    }
                    bump(C::RpActivity);
                    return rp_commit(old, 6);
                }
                2 => {
                    bump(C::RpActivity);
                    return rp_commit(old, 1);
                }
                8 if rb(0x0252_8CCE) != 0 => {
                    bump(C::RpActivity);
                    return rp_commit(old, 10);
                }
                9 => {
                    bump(C::RpActivity);
                    return rp_commit(old, 6);
                }
                _ => {}
            }
        }
        // Step 2: otherwise by the game mode (0x6976A0) and mission type.
        if call_regs(0x0069_76A0, 0, 0, 0, 0, 0, &[]) != 5 {
            return rp_commit(old, 0);
        }
        let mtype = rd(0x00E9_6C00);
        if mtype > 0xD {
            return rp_commit(old, 0);
        }
        match mtype {
            0 => rp_commit(old, 3),
            1 | 2 | 3 | 5 => rp_commit(old, 2),
            4 => {
                let r = call_regs(0x006A_7F30, 0, 0, 0, 0, 0, &[]).wrapping_add(1);
                if r > 4 {
                    return rp_commit(old, 0);
                }
                // Table 0x69A7AC: [0]=2 [1]=4 [2]=2 [3]=2 [4]=5.
                let state = match r {
                    1 => 4,
                    4 => 5,
                    _ => 2,
                };
                rp_commit(old, state)
            }
            6 => rp_commit(old, 6),
            7..=10 => rp_commit(old, 0),
            11 => {
                bump(C::RpRankState);
                let player = rd(0x0217_03D4);
                let rec = call_regs(0x0088_C900, 0, 0, player, 0, 0, &[]);
                if rec == 0 {
                    return rp_commit(old, 7);
                }
                let key = rd(rec as usize + 4);
                if !(rd(RP_RANK_ID) == key && old == 7) {
                    wd(RP_RANK_ID, key);
                }
                // Rank = 1 + number of list entries whose key is above ours.
                let head = rd(0x0213_6B78);
                let mut rank = 1u32;
                if head != 0 {
                    let mut n = head;
                    loop {
                        if (rd(n as usize + 4) as i32) > (key as i32) {
                            rank += 1;
                        }
                        n = rd(n as usize + 0x24);
                        if n == head || n == 0 {
                            break;
                        }
                    }
                }
                if rank == rd(RP_RANK) && old == 7 {
                    return;
                }
                wd(RP_RANK, rank);
                rp_commit(old, 7)
            }
            _ => {
                // 12, 13: two-sided score state (8).
                bump(C::RpScoreState);
                let player = rd(0x0217_03D4);
                let head = rd(0x0213_6B74);
                let (first, second) = if rb(player as usize + 0xFA8) == 0 {
                    (rp_find(head, 0), rp_find(head, 1))
                } else {
                    (rp_find(head, 1), rp_find(head, 0))
                };
                let a = if first != 0 { rd(first as usize + 8) as i32 } else { 0 };
                let b = if second != 0 { rd(second as usize + 8) as i32 } else { 0 };
                let cur = rd(RP_STATE);
                if !(a as u32 == rd(RP_SCORE_A) && cur == 8) {
                    wd(RP_SCORE_A, a as u32);
                }
                if !(b as u32 == rd(RP_SCORE_B) && cur == 8) {
                    wd(RP_SCORE_B, b as u32);
                }
                let lead = if a > b {
                    0
                } else if a < b {
                    1
                } else {
                    2
                };
                if rd(RP_LEAD) == lead && cur == 8 {
                    return;
                }
                wd(RP_LEAD, lead);
                rp_commit(old, 8)
            }
        }
    }
}

// --- garage_exit_tutorial (0x554E20) -----------------------------------------------------------
//
// A small state machine (0x14099AC: 0 idle, 1 waiting, 2 prompt, 3 leaving) driven by six
// flag bytes that 0x554BD0 writes into the frame. The original keeps those at [esp+0x15..0x1A],
// a 0x100-byte text buffer at [esp+0x24] and the player at [esp+0x1C]; the locals here sit at
// the same relative offsets. Checked with per-branch counters only.

const GA_STATE: usize = 0x0140_99AC;
const GA_CUR: usize = 0x0279_9304; // current tutorial record
const GA_HANDLE: usize = 0x0279_9308; // handle of the car being watched
const GA_TEXT_SHOWN: usize = 0x0252_74F5;
const GA_PROMPT: usize = 0x034A_5144;

/// Resolve a handle to its slot object when the stored type is 5 (a vehicle).
unsafe fn ga_vehicle(h: u32) -> usize {
    if h == 0 {
        return 0;
    }
    let p = slot(h);
    if p != 0 && rd(p + 0x44) == h && rd(p + 0x4C) == 5 { p } else { 0 }
}

kxs0!(garage_thunk, garage_exit_tutorial);

extern "C" fn garage_exit_tutorial() {
    unsafe {
        bump(C::GaCalls);
        // Locals indexed by the original's [esp+X] offsets.
        let mut loc = [0u8; 0x140];
        let lb = loc.as_mut_ptr() as usize;
        let (x15, x16, x17, x18, x19, x1a) = (lb + 0x15, lb + 0x16, lb + 0x17, lb + 0x18, lb + 0x19, lb + 0x1A);
        let player = rd(0x0217_03D4);
        call(0x0055_3B10);
        let mut i = 0u32;
        while (i as i32) < rd(0x0252_74E0) as i32 {
            let p = 0x0279_7BE0 + i * 0x94;
            call_regs(0x0055_4000, 0, 0, 0, i, p, &[p]);
            i += 1;
        }
        let ebp = ga_vehicle(rd(GA_HANDLE));
        if ebp == 0 {
            wd(GA_HANDLE, 0);
        }
        let watched = ga_vehicle(rd(0x0252_74F0));
        if watched != 0 {
            call_regs(0x005E_7160, watched as u32, rd(0x0252_74F0), 1, 0, 0, &[watched as u32, 2, 2, 2]);
        }
        call_regs(0x0055_39E0, GA_CUR as u32 - 4, 0, 0, 0, 0, &[]);
        let cur = rd(GA_CUR) as usize;
        if cur == 0 || rd(cur + 0x10) == 0 {
            bump(C::GaReset);
            wd(GA_STATE, 0);
            return;
        }
        if rd(GA_STATE) == 0 {
            // Idle: wait for the player-side check, then go to state 1.
            if call_regs(0x0055_2F80, rd(cur + 0x10), 0, 0, 0, 0, &[player.wrapping_add(0x14)]) as u8 != 0 {
                bump(C::GaStart);
                wd(GA_STATE, 1);
            }
            return;
        }
        call_regs(0x0055_4BD0, 0, 0, 0, 0, 0, &[x18 as u32, x16 as u32, x19 as u32, x17 as u32, x15 as u32, x1a as u32]);
        let esi;
        if ebp != 0 {
            // 554F5F: pass when (flag byte == 1 and count <= 0), else when 960D60 returns 0.
            let pass = if rb(ebp + 0x84D8) == 1 && (rd(ebp + 0x6CE8) as i32) <= 0 {
                true
            } else {
                call_regs(0x0096_0D60, rd(ebp + 0x8ABC), 0, 0, 0, 0, &[]) == 0
            };
            let skip = !pass;
            if skip || rb(x16) != 0 {
                bump(C::GaBusy);
                esi = rd(GA_CUR) as usize;
            } else {
                let mut e = rd(GA_CUR) as usize;
                if rb(e + 0x90) & 1 != 0 {
                    esi = e;
                } else {
                    if rb(x15) == 1 {
                        call_regs(0x0055_3D10, e as u32, 0, 0, 0, 0, &[]);
                        e = rd(GA_CUR) as usize;
                    }
                    wd(GA_HANDLE, 0);
                    bump(C::GaAttach);
                    let a = call_regs(0x0055_2F10, rd(e + 8), ebp as u32, 0, 0, 0, &[]) as u8;
                    let cur2 = rd(GA_CUR) as usize;
                    let c = call_regs(0x0055_2F10, rd(cur2 + 0xC), ebp as u32, 0, 0, 0, &[]) as u8;
                    wb(x17, c);
                    if a != 0 {
                        let rec = rd(GA_CUR) as usize;
                        let other = rd(rd(rec + 8) as usize + 0x44);
                        call_regs(0x00A6_CEB0, rd(rec + 8), player, rec as u32, rd(player as usize + 0x44), other, &[]);
                    }
                    if rb(x17) != 0 {
                        let rec = rd(GA_CUR) as usize;
                        let other = rd(rd(rec + 0xC) as usize + 0x44);
                        call_regs(0x00A6_CEB0, rd(rec + 0xC), player, rec as u32, rd(player as usize + 0x44), other, &[]);
                    }
                    esi = rd(GA_CUR) as usize;
                }
            }
        } else {
            esi = rd(GA_CUR) as usize;
        }
        match rd(GA_STATE) {
            1 => {
                bump(C::GaState1);
                if call_regs(0x00A7_BEB0, 0, 0, 0, 0, 0, &[]) as u8 == 0 {
                    wb(GA_TEXT_SHOWN, 0);
                    wd(GA_STATE, 2);
                    call_regs(0x0055_5E60, 0, 0, 0, esi as u32, 0, &[]);
                }
            }
            2 => {
                bump(C::GaState2);
                if rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0 {
                    return;
                }
                ga_state2(ebp, player, lb);
            }
            3 => {
                bump(C::GaState3);
                call_regs(0x0055_15E0, 0, 0, 0, esi as u32, 0, &[esi as u32]);
                let c = rd(GA_CUR) as usize;
                let limit = *(0x00E5_C368 as *const f64);
                let t = rf(c + 0x20) as f64;
                let owner = rd(c + 0x1C) as usize;
                if t < limit || owner == 0 || rd(owner + 0x94) == 0 {
                    bump(C::GaLeave);
                    call(0x0055_3CA0);
                    wd(GA_CUR, 0);
                    wd(GA_HANDLE, 0);
                } else if rb(x15) == 0 {
                    wd(GA_STATE, 1);
                }
            }
            _ => {}
        }
    }
}

/// State 2 of the garage tutorial (the prompt): body at 0x5550AD. Ends in the 0x5550D3 tail
/// (close the prompt when its handle is live) unless the prompt was just started.
unsafe fn ga_state2(ebp: usize, player: u32, lb: usize) {
    let (x15, x16, x18, x19, x1a, x1b) = (lb + 0x15, lb + 0x16, lb + 0x18, lb + 0x19, lb + 0x1A, lb + 0x1B);
    let cur = rd(GA_CUR) as usize;
    'tail: {
        if rb(x19) != 0 {
            break 'tail;
        }
        if rb(x18) == 0 {
            if rb(x16) == 0 && rb(x15) != 0 {
                call_regs(0x0055_3D10, cur as u32, 0, 0, 0, 0, &[]);
            }
            break 'tail;
        }
        // 55511B: x18 set.
        if rb(GA_TEXT_SHOWN) == 0 && ebp != 0 && call_regs(0x00AB_81C0, ebp as u32, 0, 0, 0, 0, &[]) as u8 != 0 {
            call_regs(0x00AA_AB60, 0, 0xFFFF_FFFF, ebp as u32, 0, 0, &[1.0f32.to_bits(), 1, 1]);
            wb(GA_TEXT_SHOWN, 1);
            bump(C::GaText);
        }
        let cur = rd(GA_CUR) as usize;
        if call_regs(0x0055_34C0, cur as u32, 0, 0, 0, 0, &[]) as u8 == 0 {
            break 'tail;
        }
        if ebp == 0 {
            // The original reads [0 + 0x8F48] here and faults; treat it as "no prompt".
            bump(C::GaNullVehicle);
            break 'tail;
        }
        let v = rd(ebp + 0x8F48) >> 0x14;
        if call_regs(0x0055_54C0, 0, 0, 0, 0, 0, &[v & 0xFFFF_FF01]) as u8 == 0 {
            break 'tail;
        }
        if rb(x1a) != 0 {
            break 'tail;
        }
        if call_regs(0x009B_8C50, 0, 0, 0, player, 0, &[]) as u8 == 0 && call_regs(0x009B_92E0, player, 0, 0, 0, 0, &[]) as u8 == 0 {
            call_regs(0x0055_3D10, cur as u32, 0, 0, 0, 0, &[]);
            break 'tail;
        }
        let mut buf = [0u8; 0x100];
        let r = call_regs(0x004F_ABE0, 0, 0, 0, 0, 0, &[buf.as_mut_ptr() as u32, 0x100, x1b as u32]) as u8;
        if r != 0 || rb(0x0213_B396) != r {
            break 'tail;
        }
        if call_regs(0x009B_92E0, player, 0, 0, 0, 0, &[]) as u8 != 0 {
            break 'tail;
        }
        // Start the prompt.
        let res = call_regs(0x007F_4AD0, 0, 0, 0, 0x00DE_6674, 0x00DE_6634, &[]);
        call_regs(0x0079_D700, rd(0x00EA_D20C), 0, 0, res, 0, &[GA_PROMPT as u32]);
        call_regs(0x006B_7260, 0, 0, 0, 0, 0, &[7, 0, 0, 0, 0]);
        bump(C::GaPrompt);
        return;
    }
    // 5550D3: close the prompt when its handle is live.
    let edx = rd(GA_PROMPT);
    if edx == 0 {
        return;
    }
    let eax = (edx & 0x1FFF) as i32;
    if eax >= 0x3C {
        return;
    }
    let rec = eax as usize * 15;
    if rd(0x029E_6C48 + rec * 4) != edx {
        return;
    }
    bump(C::GaClosePrompt);
    call_regs(0x0079_D1B0, 3, (0x029E_6C28 + rec * 4) as u32, edx, 0, 0, &[]);
}

// --- triggers_do_frame (0xA6C350, 310 bytes) --------------------------------------------------
//
// docs/analysis/40 and the old size estimate say ~9 KB; the real function ends at the tail
// `jmp 0xA6E810` (0xA6C481) and the code after it belongs to other functions. Checked with
// per-branch counters only (callees have side effects).

kxs0!(triggers_thunk, triggers_do_frame);

extern "C" fn triggers_do_frame() {
    unsafe {
        bump(C::TdCalls);
        // A pending camera-fade request: 1.0 == [0x2526DA0] (exact compare) runs the follow-up.
        if rd(0x0252_9E0C) != 0 {
            let cur = rd(0x0252_9E0C) as usize;
            if rf(0x0252_6DA0) == 1.0 {
                bump(C::TdFade);
                let eax = rd(cur + 0x140);
                let a1 = eax + 0x1C;
                let a2 = eax + 0x28;
                call_regs(0x009D_B480, 0, rd(0x0217_03D4), 0, 0, 0, &[a1, a2, 1, 1, 0, 0]);
                wd(0x0252_9E0C, 0);
                call_regs(0x004D_1270, 0, 0, 0, 0, 0, &[0x1F4, 0f32.to_bits()]);
            } else if rd(0x0252_6D9C) != 1 {
                bump(C::TdClear);
                wd(0x0252_9E0C, 0);
            }
        }
        // Every trigger-zone entry: a live type-7 object whose second handle is an enabled
        // type-1 object gets its callback.
        let head = 0x0252_9E08;
        let mut ebp = rd(head) as usize;
        while ebp != 0 {
            bump(C::TdEntries);
            let next = rd(ebp + 8);
            let next = if next != rd(head) { next as usize } else { 0 };
            let h1 = rd(ebp);
            if h1 != 0 {
                let edi = slot(h1);
                if edi != 0 && rd(edi + 0x44) == h1 && rd(edi + 0x4C) == 7 && rb(edi + 0x110) & 0x80 == 0 {
                    let h2 = rd(ebp + 4);
                    let mut obj2 = 0usize;
                    if h2 != 0 {
                        let s = slot(h2);
                        if s != 0 && rd(s + 0x44) == h2 && rd(s + 0x4C) == 1 {
                            obj2 = s;
                        }
                    }
                    if call_regs(0x00A6_B2B0, obj2 as u32, 0, 0, 0, 0, &[edi as u32, 1]) as u8 != 0 && rb(edi + 0x110) & 4 == 0 {
                        bump(C::TdCallback);
                        call_regs(0x00A6_CF10, 0, 0, 0, 0, rd(edi + 0x44), &[h2]);
                    }
                }
            }
            ebp = next;
        }
        call(0x00A6_E810);
    }
}

// --- vehicle_interactions_update (0xB08530, 353 bytes) ------------------------------------------
//
// Also not ~9 KB as docs/analysis/40 had it: the function is two passes over each of the
// vehicle-interaction records (0xC84 bytes, 8 rider slots of 0xA8 bytes each) and a tail call to
// 0xB0A7B0. Checked with per-branch counters only.

kxs0!(vehint_thunk, vehicle_interactions_update);

extern "C" fn vehicle_interactions_update() {
    unsafe {
        bump(C::ViCalls);
        let mut rec = 0x0314_92B8usize;
        while rec < 0x0316_2338 {
            let h = rd(rec);
            if h != 0 {
                let obj = slot(h);
                if obj == 0 || rd(obj + 0x44) != h || rd(obj + 0x4C) != 5 {
                    bump(C::ViStale);
                    call_regs(0x00B1_6780, 0, 0, 0, 0, rec as u32, &[]);
                } else {
                    bump(C::ViLive);
                    // Pass 1: drop rider slots whose handle is no longer a live type-1 object;
                    // start the exit for riders whose object has no animation state.
                    for i in 0..8usize {
                        let sl = rec + 0x34C + i * 0xA8;
                        let hb = rd(sl);
                        if hb == 0 {
                            continue;
                        }
                        let o = slot(hb);
                        if o != 0 && rd(o + 0x44) == hb && rd(o + 0x4C) == 1 {
                            if rd(o + 0xD74) == 0 {
                                bump(C::ViExit);
                                call_regs(0x00B1_68A0, i as u32, 0, 0, rec as u32, 0, &[]);
                            }
                        } else {
                            bump(C::ViDrop);
                            call_regs(0x00B0_4D60, sl as u32, (hb & 0xFFF) << 4, 0, 0, 0, &[]);
                            wd(rec + 0x348, rd(rec + 0x348).wrapping_sub(1));
                            let o = slot(hb);
                            if o != 0 && rd(o + 0x44) == hb && rd(o + 0x4C) == 1 {
                                wd(o + 0x10E0, 0);
                            }
                        }
                    }
                    call_regs(0x00B0_8E70, 0, 0, 0, 0, 0, &[rec as u32]);
                    call_regs(0x00B0_8C50, rec as u32, 0, 0, 0, 0, &[]);
                    // Pass 2: per-rider-state update.
                    for i in 0..8usize {
                        let sl = rec + 0x350 + i * 0xA8;
                        if rd(sl - 4) == 0 {
                            continue;
                        }
                        match rd(sl) {
                            1 => {
                                bump(C::ViState1);
                                call_regs(0x00B1_1650, rec as u32, i as u32, 0, 0, 0, &[]);
                            }
                            2 => {
                                bump(C::ViState2);
                                call_regs(0x00B1_51E0, i as u32, rec as u32, 0, 0, 0, &[]);
                            }
                            3 if rd(sl + 4) == 0x21 => {
                                bump(C::ViState3);
                                call_regs(0x00B1_7EC0, 0, 0, 0, 0, 0, &[rec as u32, i as u32]);
                            }
                            _ => {}
                        }
                    }
                }
            }
            rec += 0xC84;
        }
        call(0x00B0_A7B0);
    }
}

// --- x87 helper ---------------------------------------------------------------------------------

/// `call_regs_st0` that also returns the callee's EAX.
unsafe fn call_regs_st0e(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> (f64, u32) {
    #[cfg(test)]
    let f = difftest::redirect(f);
    let mut out = 0f64;
    let mut eaxo = 0u32;
    let block: [u32; 10] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32, &mut eaxo as *mut u32 as u32];
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
            "mov ecx, [ebx + 36]",
            "mov [ecx], eax",
            "mov ecx, [ebx + 32]",
            "fstp qword ptr [ecx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    (out, eaxo)
}

/// `call_regs` for a callee that returns on the x87 stack (popped as a qword).
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    #[cfg(test)]
    let f = difftest::redirect(f);
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

// --- ai_scheduled_tasks_expire (0x4525E0, 570 bytes) ----------------------------------------------
//
// Walks the scheduled-task list (head 0xEE2C00) and expires (0x452570) every task that is past
// its deadline, whose two handles are no longer live, whose live human changed state, or that is
// farther from either object than the object's range (25.0, or 9.0 for a type-1 object with the
// byte at +0x3CC set). Checked with per-branch counters only (the expiry callee has side effects).

/// Object for handle `h` when its slot still carries `h`, else 0.
unsafe fn live_obj(h: u32) -> usize {
    if h == 0 {
        return 0;
    }
    let p = slot(h);
    if p != 0 && rd(p + 0x44) == h { p } else { 0 }
}

/// One of the two "is this human still relevant" tests (`bit` is the flag in task byte +0x2C):
/// true means expire.
unsafe fn tasks_human_changed(h: u32, flags: u8, shift: u32) -> bool {
    if h == 0 {
        return false;
    }
    let s = slot(h);
    if s == 0 || rd(s + 0x44) != h || rd(s + 0x4C) != 1 {
        return false;
    }
    let state = rd(s + 0x80);
    if state & 0xB != 0 {
        return false;
    }
    if call_regs(0x0098_5630, 0, 0, 0, 0, 0, &[s as u32]) as u8 != 0 {
        return false;
    }
    if (state >> 4) & 1 != 0 || rd(s + 0xC8) & 0x0040_0000 == 0 {
        return false;
    }
    let cl = (rd(s + 0xC0) >> 7) as u8;
    let dl = flags >> shift;
    (cl ^ dl) & 1 != 0
}

/// Range test: the x87 status word (cond bits) and the callee's EAX; `expire` is "threshold < dist".
unsafe fn tasks_out_of_range(task_pt: usize, obj: usize) -> (bool, u32, u32) {
    let (d, ceax) = call_regs_st0e(0x00BD_EA30, task_pt as u32, (obj + 0x14) as u32, 0, 0, 0, &[]);
    let dist = d as f32;
    let near = rd(obj + 0x4C) == 1 && rb(obj + 0x3CC) == 1;
    let thresh = if near { rf(0x00DD_32B8) } else { rf(0x00ED_B934) };
    let (t, dd) = (thresh as f64, dist as f64);
    // fcompp (ST0 = thresh against ST1 = dist) then fnstsw ax: C0 = 0x100, C3 = 0x4000, C2 = 0x400
    let sw: u32 = if t.is_nan() || dd.is_nan() { 0x4500 } else if t < dd { 0x0100 } else if t == dd { 0x4000 } else { 0 };
    (t < dd, (ceax & 0xFFFF_0000) | sw, ceax)
}

kz_thunk0!(tasks_expire_thunk, ai_scheduled_tasks_expire);

extern "C" fn ai_scheduled_tasks_expire(out: *mut [u32; 7]) -> u32 {
    unsafe {
        bump(C::TeCalls);
        const HEAD: usize = 0x00EE_2C00;
        const SLOT: usize = 0x0214_9C64;
        let mut eax = (*out)[2];
        let mut ecx = (*out)[0];
        (*out)[1] = 0; // EDX: `xor edx, edx` at every loop top
        let mut edi = rd(HEAD) as usize;
        while edi != 0 {
            bump(C::TeTasks);
            eax = rd(edi);
            ecx = if eax != rd(HEAD) { eax } else { 0 };
            let next = ecx as usize;
            let deadline = rd(edi + 0x28);
            eax = deadline;
            let mut expire = false;
            if (deadline as i32) >= 0 {
                let now = rd(0x00E8_43AC);
                ecx = now;
                if (deadline as i32) > (now as i32) {
                    eax = deadline.wrapping_sub(now);
                    if (eax as i32) >= 0x35A4_E900 {
                        bump(C::TeDue);
                        expire = true;
                    }
                } else {
                    ecx = now.wrapping_sub(deadline);
                    if (ecx as i32) < 0x35A4_E900 {
                        bump(C::TeDue);
                        expire = true;
                    }
                }
            }
            'chk: {
                if expire {
                    break 'chk;
                }
                let ha = rd(edi + 8);
                ecx = ha;
                let mut obj1 = 0usize;
                if ha != 0 {
                    eax = (ha & 0xFFF) << 4;
                    eax = rd(SLOT + eax as usize);
                    if eax != 0 && rd(eax as usize + 0x44) == ha {
                        obj1 = eax as usize;
                    }
                }
                let hb = rd(edi + 0xC);
                let mut obj2 = 0usize;
                if hb != 0 {
                    eax = (hb & 0xFFF) << 4;
                    eax = rd(SLOT + eax as usize);
                    if eax != 0 && rd(eax as usize + 0x44) == hb {
                        obj2 = eax as usize;
                    }
                }
                if obj1 == 0 || obj2 == 0 {
                    bump(C::TeStale);
                    expire = true;
                    break 'chk;
                }
                let flags = rb(edi + 0x2C);
                // the two "human changed state" tests (shift 1 for handle a, 2 for handle b)
                for (h, shift) in [(ha, 1u32), (hb, 2u32)] {
                    if h == 0 {
                        continue;
                    }
                    eax = (h & 0xFFF) << 4;
                    let s = rd(SLOT + eax as usize) as usize;
                    if s == 0 || rd(s + 0x44) != h || rd(s + 0x4C) != 1 {
                        continue;
                    }
                    let ebx = rd(s + 0x80);
                    if ebx & 0xB != 0 {
                        continue;
                    }
                    eax = call_regs(0x0098_5630, 0, 0, 0, 0, 0, &[s as u32]);
                    if eax as u8 != 0 {
                        continue;
                    }
                    if (ebx >> 4) & 1 != 0 || rd(s + 0xC8) & 0x0040_0000 == 0 {
                        continue;
                    }
                    ecx = rd(s + 0xC0) >> 7;
                    let dl = flags >> shift;
                    let cl = (ecx as u8) ^ dl;
                    ecx = (ecx & !0xFF) | cl as u32;
                    if cl & 1 != 0 {
                        bump(C::TeHuman);
                        expire = true;
                        break 'chk;
                    }
                }
                let (e1, _, _) = tasks_out_of_range(edi + 0x10, obj1);
                if e1 {
                    bump(C::TeRange);
                    expire = true;
                    break 'chk;
                }
                let (e2, ax2, _) = tasks_out_of_range(edi + 0x1C, obj2);
                eax = ax2;
                if e2 {
                    bump(C::TeRange);
                    expire = true;
                }
            }
            if expire {
                eax = call_regs(0x0045_2570, edi as u32, 0, 0, 0, 0, &[]);
            }
            edi = next;
        }
        let _ = ecx;
        (*out)[0] = ecx;
        eax
    }
}

// --- scheduled_world_events_update (0x7B02E0, 607 bytes) --------------------------------------------
//
// Two passes over circular lists of scheduled events: (1) events on list A (0x25287FC) whose key
// (u16 +0x1C, u8 +0x1E, u8 +0x1F, u32 +0x28) is below the current clock key (0x140E728..0x140E734)
// move to list B (0x25287F8) and have their entry block started (0x48B550); (2) when the player
// is allowed to see them, entries on list B whose time (f32 at +0x10 of a 24-byte entry) has come
// (<= 0x4B1910()) move back to list A and are fired. Checked with per-branch counters only.

/// Unlink `n` (next pointer `next`) from the circular list whose head is at `head`, as the
/// original does: fix the head, then the neighbours, then clear both links.
unsafe fn sw_unlink(head: usize, n: usize, next: u32) {
    if rd(head) as usize == n {
        if next as usize == n {
            wd(n + 4, 0);
            wd(n, 0);
            wd(head, 0);
            return;
        }
        wd(head, next);
    }
    unsafe_unlink_links(n, next);
}

unsafe fn unsafe_unlink_links(n: usize, next: u32) {
    let prev = rd(n) as usize;
    if prev != 0 {
        wd(prev + 4, next);
    }
    let nx = rd(n + 4) as usize;
    if nx != 0 {
        wd(nx, rd(n));
    }
    wd(n + 4, 0);
    wd(n, 0);
}

/// Insert `n` at the tail of the circular list whose head is at `head`.
unsafe fn sw_insert(head: usize, n: usize) {
    let h = rd(head) as usize;
    if h == 0 {
        wd(n + 4, n as u32);
        wd(n, n as u32);
        wd(head, n as u32);
    } else {
        let tail = rd(h);
        wd(n, tail);
        wd(n + 4, h as u32);
        wd(tail as usize + 4, n as u32);
        wd(h, n as u32);
    }
}

kxs0!(sched_events_thunk, scheduled_world_events_update);

extern "C" fn scheduled_world_events_update() {
    unsafe {
        bump(C::SwCalls);
        const LIST_A: usize = 0x0252_87FC;
        const LIST_B: usize = 0x0252_87F8;
        // Pass 1: events whose key is below the clock key move from A to B.
        let mut n = rd(LIST_A) as usize;
        while n != 0 {
            let next = rd(n + 4);
            let following = if next != rd(LIST_A) { next as usize } else { 0 };
            let clock = (rw(0x0140_E728), rb(0x0140_E72A), rb(0x0140_E72B), rd(0x0140_E734));
            let key = (rw(n + 0x1C), rb(n + 0x1E), rb(n + 0x1F), rd(n + 0x28));
            if clock > key {
                bump(C::SwMoved);
                sw_unlink(LIST_A, n, next);
                sw_insert(LIST_B, n);
                let edi = rd(n + 0x14) + rb(n + 0x10) as u32 * 24 + 8;
                call_regs(0x0048_B550, n as u32, 0, 0, 0, edi, &[]);
            }
            n = following;
        }
        // Pass 2: due entries fire.
        if (rd(0x0252_6C90) as i32) <= 0 {
            return;
        }
        let p = rd(0x0252_8C14);
        if p != 0 {
            if rb(0x0252_8B56) != 0 && rb(0x0252_8B2B) == 0 {
                return;
            }
            if rb(p as usize + 0x3F8) == 0 {
                return;
            }
        }
        let now = call_f32(0x004B_1910);
        let mut edi = rd(LIST_B) as usize;
        while edi != 0 {
            bump(C::SwNodes);
            let edx = rd(edi + 4);
            let ebx = if edx != rd(LIST_B) { edx as usize } else { 0 };
            let count = rb(edi + 0x11);
            let mut hit = None;
            let mut cl = 0u8;
            while cl < count {
                let entry = rd(edi + 0x14) as usize + cl as usize * 24;
                if rb(entry + 0x14) & 1 == 0 && (rf(entry + 0x10) as f64) <= (now as f64) {
                    hit = Some(cl);
                    break;
                }
                cl += 1;
            }
            if let Some(cl) = hit {
                bump(C::SwFired);
                sw_unlink(LIST_B, edi, edx);
                sw_insert(LIST_A, edi);
                let off = cl as usize * 24;
                wb(edi + 0x10, cl);
                let base = rd(edi + 0x14) as usize;
                call_regs(0x0048_B1C0, 0, (base + off + 8) as u32, 0, 0, 0, &[0, 1]);
                let secs = rd(rd(edi + 0x14) as usize + off + 0xC).wrapping_mul(0x0036_EE80);
                call_regs(0x0068_B240, 0, 0, 0, (edi + 0x18) as u32, 0, &[secs]);
                let base = rd(edi + 0x14) as usize;
                wb(base + off + 0x14, rb(base + off + 0x14) | 1);
                call_regs(0x007A_FE60, (base + off + 0x14) as u32, 0, 0, 0, 0, &[(base + off) as u32]);
            }
            edi = ebx;
        }
    }
}

// --- Install / summary ------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let targets: [(&str, usize, &[u8], usize); 22] = [
        ("zone_watchers_update", 0x00A9_3FF0, &ZONE_WATCHERS_BODY, zone_watchers_thunk as extern "C" fn() as usize),
        ("persona_events_update", 0x006A_DB10, &PERSONA_BODY, persona_thunk as extern "C" fn() as usize),
        ("nav_detours_update", 0x00A0_AFF0, &NAV_DETOURS_BODY, nav_detours_thunk as extern "C" fn() as usize),
        ("vehicle_attached_effects_update", 0x004D_1D50, &VEH_ATTACHED_BODY, vae_thunk as extern "C" fn() as usize),
        ("ai_scheduled_tasks_tick", 0x0045_2820, &TASKS_TICK_BODY, tasks_tick_thunk as extern "C" fn() as usize),
        ("explosions_flush_queue", 0x004C_2BF0, &EXPLOSIONS_BODY, explosions_thunk as extern "C" fn() as usize),
        ("wiretap_triggers_update", 0x0054_CCA0, &WIRETAP_BODY, wiretap_thunk as extern "C" fn() as usize),
        ("cellphone_do_frame", 0x00A6_6340, &CELLPHONE_BODY, cellphone_thunk as extern "C" fn() as usize),
        ("havok_triggers_update", 0x0071_B910, &HAVOK_BODY, havok_thunk as extern "C" fn() as usize),
        ("cheats_do_frame", 0x0068_7B40, &CHEATS_BODY, cheats_thunk as extern "C" fn() as usize),
        ("rich_presence_update", 0x0069_A440, &RICH_BODY, rich_thunk as extern "C" fn() as usize),
        ("garage_exit_tutorial", 0x0055_4E20, &GARAGE_BODY, garage_thunk as extern "C" fn() as usize),
        ("triggers_do_frame", 0x00A6_C350, &TRIGGERS_BODY, triggers_thunk as extern "C" fn() as usize),
        ("vehicle_interactions_update", 0x00B0_8530, &VEHINT_BODY, vehint_thunk as extern "C" fn() as usize),
        ("ai_scheduled_tasks_expire", 0x0045_25E0, &TASKS_EXPIRE_BODY, tasks_expire_thunk as extern "C" fn() as usize),
        ("scheduled_world_events_update", 0x007B_02E0, &SCHED_EVENTS_BODY, sched_events_thunk as extern "C" fn() as usize),
        ("humans_post_update", 0x009A_7210, &HUMANS_POST_BODY, humans_post_thunk as extern "C" fn() as usize),
        ("object_items_update", 0x0093_05F0, &OBJECT_ITEMS_BODY, object_items_thunk as extern "C" fn() as usize),
        ("humans_movement_update", 0x009A_9890, &HUMANS_MOVE_BODY, humans_movement_thunk as extern "C" fn() as usize),
        ("object_props_update", 0x0093_5820, &OBJECT_PROPS_BODY, object_props_thunk as extern "C" fn() as usize),
        ("shops_do_frame", 0x009F_5420, &SHOPS_FRAME_BODY, shops_frame_thunk as extern "C" fn() as usize),
        ("tutorial_do_frame", 0x006B_7020, &TUTORIAL_FRAME_BODY, tutorial_frame_thunk as extern "C" fn() as usize),
    ];
    let mut ok = 0;
    for (name, addr, body, thunk) in targets {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    crate::log(&format!("port_frame2: replaced {ok}/{} with Rust", 22));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = (0..C::N as usize)
        .filter_map(|i| {
            let v = CNT[i].load(Ordering::Relaxed);
            (v != 0).then(|| format!("{} {v}", NAMES[i]))
        })
        .collect();
    let mut parts = parts;
    let hpu: Vec<String> = (0..HP_N).filter_map(|i| {
        let v = HP_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", HP_NAMES[i]))
    }).collect();
    if !hpu.is_empty() {
        parts.push(format!("humans_post_update ({})", hpu.join(" ")));
    }
    let oiu: Vec<String> = (0..OI_N).filter_map(|i| {
        let v = OI_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", OI_NAMES[i]))
    }).collect();
    if !oiu.is_empty() {
        parts.push(format!("object_items_update ({})", oiu.join(" ")));
    }
    let hmv: Vec<String> = (0..HM_N).filter_map(|i| {
        let v = HM_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", HM_NAMES[i]))
    }).collect();
    if !hmv.is_empty() {
        parts.push(format!("humans_movement_update ({})", hmv.join(" ")));
    }
    let opv: Vec<String> = (0..OP_N).filter_map(|i| {
        let v = OP_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", OP_NAMES[i]))
    }).collect();
    if !opv.is_empty() {
        parts.push(format!("object_props_update ({})", opv.join(" ")));
    }
    let sdv: Vec<String> = (0..SD_N).filter_map(|i| {
        let v = SD_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", SD_NAMES[i]))
    }).collect();
    if !sdv.is_empty() {
        parts.push(format!("shops_do_frame ({})", sdv.join(" ")));
    }
    let tdv: Vec<String> = (0..TD_N).filter_map(|i| {
        let v = TD_HITS[i].load(Ordering::Relaxed);
        (v != 0).then(|| format!("{}={v}", TD_NAMES[i]))
    }).collect();
    if !tdv.is_empty() {
        parts.push(format!("tutorial_do_frame ({})", tdv.join(" ")));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("port_frame2: {}", parts.join(", "))
}

// --- Original bodies (pins) -------------------------------------------------------------------

const ZONE_WATCHERS_BODY: [u8; 329] = [
    0x53, 0x57, 0x8B, 0x3D, 0x5C, 0xE9, 0x23, 0x02, 0x33, 0xDB, 0x3B, 0xFB, 0x0F, 0x84, 0x34, 0x01,
    0x00, 0x00, 0x55, 0x56, 0x8A, 0x47, 0x0C, 0x8B, 0x77, 0x08, 0x3B, 0xF3, 0x88, 0x47, 0x0D, 0xC6,
    0x47, 0x0C, 0x01, 0x0F, 0x84, 0x09, 0x01, 0x00, 0x00, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x3B, 0xF3, 0x0F, 0x84, 0x93, 0x00, 0x00, 0x00, 0x8B, 0x06, 0x8A, 0x4E, 0x10, 0x8B, 0xE8, 0x2B,
    0x6F, 0x08, 0x88, 0x4E, 0x11, 0xF7, 0xDD, 0x1B, 0xED, 0x23, 0xE8, 0xE8, 0x80, 0xFB, 0xFF, 0xFF,
    0x84, 0xC0, 0x74, 0x63, 0x39, 0x77, 0x08, 0x75, 0x13, 0x8B, 0x06, 0x3B, 0xC6, 0x75, 0x0A, 0x89,
    0x1E, 0x89, 0x5E, 0x04, 0x89, 0x5F, 0x08, 0xEB, 0x1F, 0x89, 0x47, 0x08, 0x8B, 0x46, 0x04, 0x3B,
    0xC3, 0x74, 0x04, 0x8B, 0x16, 0x89, 0x10, 0x8B, 0x06, 0x3B, 0xC3, 0x74, 0x06, 0x8B, 0x4E, 0x04,
    0x89, 0x48, 0x04, 0x89, 0x1E, 0x89, 0x5E, 0x04, 0xA1, 0x64, 0xE9, 0x23, 0x02, 0x3B, 0xC3, 0x75,
    0x0F, 0x89, 0x36, 0x89, 0x76, 0x04, 0x89, 0x35, 0x64, 0xE9, 0x23, 0x02, 0x8B, 0xF5, 0xEB, 0x90,
    0x8B, 0x50, 0x04, 0x89, 0x56, 0x04, 0x89, 0x06, 0x8B, 0x48, 0x04, 0x89, 0x31, 0x89, 0x70, 0x04,
    0x8B, 0xF5, 0xE9, 0x79, 0xFF, 0xFF, 0xFF, 0x8B, 0xC6, 0xE8, 0x82, 0xFC, 0xFF, 0xFF, 0x88, 0x46,
    0x10, 0x20, 0x47, 0x0C, 0x8B, 0xF5, 0xE9, 0x65, 0xFF, 0xFF, 0xFF, 0x8A, 0x47, 0x0C, 0x3A, 0xC3,
    0x74, 0x18, 0x38, 0x5F, 0x0D, 0x75, 0x0F, 0x8B, 0x47, 0x10, 0x3B, 0xC3, 0x74, 0x54, 0x57, 0xFF,
    0xD0, 0x83, 0xC4, 0x04, 0xEB, 0x4C, 0x3A, 0xC3, 0x75, 0x48, 0x38, 0x5F, 0x0D, 0x74, 0x0F, 0x8B,
    0x47, 0x14, 0x3B, 0xC3, 0x74, 0x3C, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xEB, 0x34, 0x3A, 0xC3,
    0x75, 0x30, 0x38, 0x5F, 0x0D, 0x75, 0x2B, 0x8B, 0x77, 0x08, 0x3B, 0xF3, 0x74, 0x24, 0x8B, 0xFF,
    0x38, 0x5E, 0x10, 0x74, 0x12, 0x38, 0x5E, 0x11, 0x75, 0x0D, 0x8B, 0x47, 0x18, 0x3B, 0xC3, 0x74,
    0x06, 0x56, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0x8B, 0x36, 0x3B, 0x77, 0x08, 0x74, 0x04, 0x3B, 0xF3,
    0x75, 0xDE, 0x8B, 0x3F, 0x3B, 0x3D, 0x5C, 0xE9, 0x23, 0x02, 0x74, 0x08, 0x3B, 0xFB, 0x0F, 0x85,
    0xD0, 0xFE, 0xFF, 0xFF, 0x5E, 0x5D, 0x5F, 0x5B, 0xC3,
];
#[allow(dead_code)]
const ZONE_WATCHERS_BODY_CALLS: [usize; 2] = [0x4b, 0xb9];
const PERSONA_BODY: [u8; 541] = [
    0x51, 0x53, 0x55, 0x56, 0x57, 0xE8, 0x86, 0xEA, 0xFF, 0xFF, 0xE8, 0xF1, 0x8B, 0x3A, 0x00, 0x80,
    0x3D, 0xCE, 0x7C, 0x52, 0x02, 0x00, 0x74, 0x17, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x50, 0xE8, 0x2D,
    0x96, 0x3A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x05, 0xA2, 0xCE, 0x7C, 0x52, 0x02, 0x8B,
    0x15, 0x04, 0x04, 0x45, 0x01, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x8B, 0xF2, 0x8D, 0x49, 0x00,
    0x85, 0xF6, 0x0F, 0x84, 0xE6, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x14, 0x8B, 0xF8, 0x2B, 0xFA, 0xF7,
    0xDF, 0x1B, 0xFF, 0x23, 0xF8, 0x8B, 0x46, 0x10, 0x85, 0xC0, 0x0F, 0x8C, 0xC7, 0x00, 0x00, 0x00,
    0x3B, 0xC1, 0x7E, 0x0F, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8C, 0xB6, 0x00, 0x00,
    0x00, 0xEB, 0x10, 0x8B, 0xD9, 0x2B, 0xD8, 0x81, 0xFB, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8D, 0xA4,
    0x00, 0x00, 0x00, 0x0F, 0xB6, 0x4E, 0x0E, 0xD9, 0x46, 0x08, 0x0F, 0xB7, 0x56, 0x0C, 0x8B, 0x46,
    0x04, 0x51, 0x52, 0x51, 0xD9, 0x1C, 0x24, 0x50, 0x8B, 0x06, 0xE8, 0xD1, 0x05, 0x00, 0x00, 0x8B,
    0x15, 0x04, 0x04, 0x45, 0x01, 0x83, 0xC4, 0x10, 0x3B, 0xD6, 0x75, 0x1F, 0x8B, 0x46, 0x14, 0x3B,
    0xC6, 0x75, 0x10, 0x33, 0xD2, 0x89, 0x56, 0x14, 0x89, 0x56, 0x18, 0x89, 0x15, 0x04, 0x04, 0x45,
    0x01, 0xEB, 0x2A, 0x8B, 0xD0, 0x89, 0x15, 0x04, 0x04, 0x45, 0x01, 0x8B, 0x46, 0x18, 0x33, 0xC9,
    0x3B, 0xC1, 0x74, 0x06, 0x8B, 0x5E, 0x14, 0x89, 0x58, 0x14, 0x8B, 0x46, 0x14, 0x3B, 0xC1, 0x74,
    0x06, 0x8B, 0x5E, 0x18, 0x89, 0x58, 0x18, 0x89, 0x4E, 0x14, 0x89, 0x4E, 0x18, 0xA1, 0x00, 0x04,
    0x45, 0x01, 0x85, 0xC0, 0x75, 0x19, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x89, 0x76, 0x14, 0x89,
    0x76, 0x18, 0x89, 0x35, 0x00, 0x04, 0x45, 0x01, 0x8B, 0xF7, 0xE9, 0x31, 0xFF, 0xFF, 0xFF, 0x8B,
    0x48, 0x18, 0x89, 0x4E, 0x18, 0x89, 0x46, 0x14, 0x8B, 0x48, 0x18, 0x89, 0x71, 0x14, 0x8B, 0x0D,
    0xAC, 0x43, 0xE8, 0x00, 0x89, 0x70, 0x18, 0x8B, 0xF7, 0xE9, 0x12, 0xFF, 0xFF, 0xFF, 0x8B, 0x35,
    0x24, 0x87, 0x7D, 0x02, 0x8B, 0x1D, 0x58, 0xE2, 0x12, 0x02, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x85, 0xF6, 0x0F, 0x84, 0xCF, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x10, 0x8B, 0xE8, 0x2B, 0x2D, 0x24,
    0x87, 0x7D, 0x02, 0xF7, 0xDD, 0x1B, 0xED, 0x23, 0xE8, 0x0F, 0xB7, 0x06, 0x66, 0x3D, 0xFF, 0xFF,
    0x0F, 0x84, 0xAA, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0xF8, 0x8B, 0xC7, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0,
    0x03, 0xC7, 0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xD1, 0xC1, 0xFA,
    0x06, 0x33, 0xCA, 0x8B, 0xC1, 0xC1, 0xE0, 0x0B, 0xF7, 0xD0, 0x03, 0xC8, 0x8B, 0xC1, 0xC1, 0xF8,
    0x10, 0x33, 0xC1, 0x99, 0xF7, 0x7B, 0x60, 0x8B, 0x4B, 0x5C, 0x8B, 0x14, 0x91, 0x85, 0xD2, 0x74,
    0x6F, 0x3B, 0x7A, 0x0C, 0x74, 0x0B, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75, 0xF4, 0x8B, 0xF5, 0xEB,
    0x8F, 0x8B, 0x12, 0x85, 0xD2, 0x74, 0x59, 0x8B, 0x42, 0x04, 0x85, 0xC0, 0x74, 0x52, 0x8B, 0xD0,
    0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x8A, 0x64, 0x9C, 0x14, 0x02, 0x85,
    0xC9, 0x74, 0x05, 0x39, 0x41, 0x44, 0x74, 0x04, 0x33, 0xC0, 0xEB, 0x0E, 0x8B, 0x41, 0x4C, 0x83,
    0xE8, 0x01, 0xF7, 0xD8, 0x1B, 0xC0, 0xF7, 0xD0, 0x23, 0xC1, 0x8B, 0x4E, 0x0C, 0x0F, 0xB7, 0x56,
    0x08, 0x51, 0x8B, 0x4E, 0x04, 0x52, 0x51, 0x50, 0xE8, 0x83, 0x14, 0x00, 0x00, 0x83, 0xC4, 0x10,
    0xBF, 0x18, 0x87, 0x7D, 0x02, 0xE8, 0xE6, 0x22, 0x00, 0x00, 0x8B, 0x1D, 0x58, 0xE2, 0x12, 0x02,
    0x8B, 0xF5, 0xE9, 0x29, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x59, 0xC3,
];
#[allow(dead_code)]
const PERSONA_BODY_CALLS: [usize; 6] = [0x5, 0xa, 0x1e, 0x9a, 0x1f8, 0x205];
const NAV_DETOURS_BODY: [u8; 568] = [
    0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x83, 0xEC, 0x0C, 0x53, 0x55, 0x56, 0x57, 0x33, 0xD2, 0x8B, 0xF8,
    0x85, 0xFF, 0x0F, 0x84, 0x18, 0x02, 0x00, 0x00, 0x8B, 0x0F, 0x8B, 0xD9, 0x2B, 0xD8, 0xF7, 0xDB,
    0x1B, 0xDB, 0x23, 0xD9, 0x8B, 0xCA, 0x83, 0xC2, 0x01, 0x81, 0xF9, 0x96, 0x00, 0x00, 0x00, 0x89,
    0x54, 0x24, 0x18, 0x0F, 0x8F, 0xA2, 0x01, 0x00, 0x00, 0x8B, 0x47, 0x10, 0x83, 0xE8, 0x00, 0x0F,
    0x84, 0x96, 0x00, 0x00, 0x00, 0x83, 0xE8, 0x01, 0x74, 0x5B, 0x83, 0xE8, 0x01, 0x74, 0x15, 0x8B,
    0x4F, 0x08, 0xE8, 0x79, 0x30, 0x00, 0x00, 0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x18,
    0x8B, 0xFB, 0xEB, 0xAC, 0x8B, 0x4F, 0x0C, 0x85, 0xC9, 0x74, 0x22, 0x8B, 0xC1, 0x25, 0xFF, 0x01,
    0x00, 0x00, 0x3D, 0xFF, 0x01, 0x00, 0x00, 0x77, 0x14, 0x69, 0xC0, 0x08, 0x02, 0x00, 0x00, 0x05,
    0xE0, 0x4D, 0x67, 0x02, 0x39, 0x48, 0x10, 0x0F, 0x84, 0x87, 0x00, 0x00, 0x00, 0x8B, 0x4F, 0x08,
    0xE8, 0x3B, 0x30, 0x00, 0x00, 0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x18, 0x8B, 0xFB,
    0xE9, 0x6B, 0xFF, 0xFF, 0xFF, 0x8B, 0x4F, 0x0C, 0x85, 0xC9, 0x74, 0x17, 0x8B, 0xC1, 0x25, 0xFF,
    0xFF, 0x00, 0x00, 0x69, 0xC0, 0x34, 0x01, 0x00, 0x00, 0x05, 0xE0, 0x50, 0xAE, 0x02, 0x39, 0x48,
    0x08, 0x74, 0x51, 0x8B, 0x4F, 0x08, 0xE8, 0x05, 0x30, 0x00, 0x00, 0xA1, 0x08, 0x2C, 0x1F, 0x02,
    0x8B, 0x54, 0x24, 0x18, 0x8B, 0xFB, 0xE9, 0x35, 0xFF, 0xFF, 0xFF, 0x8B, 0x47, 0x0C, 0x85, 0xC0,
    0x74, 0x1A, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x8A, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x05, 0x39, 0x41, 0x44, 0x74, 0x18, 0x8B, 0x4F, 0x08, 0xE8,
    0xCC, 0x2F, 0x00, 0x00, 0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x18, 0x8B, 0xFB, 0xE9,
    0xFC, 0xFE, 0xFF, 0xFF, 0x8D, 0x44, 0x24, 0x14, 0x50, 0x8B, 0xC7, 0xE8, 0x00, 0xEA, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0x84, 0xC0, 0x74, 0x5A, 0x83, 0x7F, 0x18, 0x00, 0x74, 0x36, 0x8B, 0x6C, 0x24,
    0x14, 0x55, 0x8B, 0xF7, 0xE8, 0xB7, 0xEE, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x0F, 0x84,
    0x87, 0x00, 0x00, 0x00, 0xE8, 0x97, 0xFD, 0xFF, 0xFF, 0x55, 0x57, 0xE8, 0x20, 0xF6, 0xFF, 0xFF,
    0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x20, 0x83, 0xC4, 0x08, 0x8B, 0xFB, 0xE9, 0xAD,
    0xFE, 0xFF, 0xFF, 0x8B, 0x4C, 0x24, 0x14, 0x51, 0x57, 0xE8, 0x02, 0xF6, 0xFF, 0xFF, 0xA1, 0x08,
    0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x20, 0x83, 0xC4, 0x08, 0x8B, 0xFB, 0xE9, 0x8F, 0xFE, 0xFF,
    0xFF, 0x83, 0x7F, 0x18, 0x00, 0x74, 0x44, 0x8B, 0x4F, 0x0C, 0x85, 0xC9, 0x74, 0x38, 0x8B, 0xD1,
    0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85,
    0xC0, 0x74, 0x23, 0x39, 0x48, 0x44, 0x75, 0x1E, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x18, 0x8B, 0x4F,
    0x08, 0xE8, 0x2A, 0x30, 0x00, 0x00, 0xA1, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x54, 0x24, 0x18, 0x8B,
    0xFB, 0xE9, 0x4A, 0xFE, 0xFF, 0xFF, 0xE8, 0x15, 0xFD, 0xFF, 0xFF, 0xA1, 0x08, 0x2C, 0x1F, 0x02,
    0x8B, 0x54, 0x24, 0x18, 0x8B, 0xFB, 0xE9, 0x35, 0xFE, 0xFF, 0xFF, 0x85, 0xC0, 0x74, 0x21, 0x3B,
    0xC7, 0x75, 0x06, 0x8B, 0xF8, 0x33, 0xC0, 0xEB, 0x13, 0x8B, 0x57, 0x04, 0x8B, 0x48, 0x04, 0x89,
    0x02, 0x8B, 0x57, 0x04, 0x89, 0x50, 0x04, 0x89, 0x4F, 0x04, 0x89, 0x39, 0x85, 0xFF, 0x75, 0x10,
    0x8B, 0xF8, 0x89, 0x3D, 0x08, 0x2C, 0x1F, 0x02, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
    0x85, 0xC0, 0x74, 0x16, 0x8B, 0x4F, 0x04, 0x89, 0x01, 0x8B, 0x50, 0x04, 0x89, 0x3A, 0x8B, 0x57,
    0x04, 0x8B, 0x48, 0x04, 0x89, 0x50, 0x04, 0x89, 0x4F, 0x04, 0x89, 0x3D, 0x08, 0x2C, 0x1F, 0x02,
    0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
];
#[allow(dead_code)]
const NAV_DETOURS_BODY_CALLS: [usize; 11] = [0x52, 0x90, 0xc6, 0xff, 0x11b, 0x134, 0x144, 0x14b, 0x169, 0x1b1, 0x1c6];
const VEH_ATTACHED_BODY: [u8; 484] = [
    0x83, 0xEC, 0x14, 0x53, 0x8B, 0x1D, 0x1C, 0x99, 0xF9, 0x00, 0x55, 0x56, 0x57, 0x8B, 0x3D, 0x20,
    0x99, 0xF9, 0x00, 0x33, 0xD2, 0x3B, 0xDA, 0x0F, 0x84, 0xBF, 0x01, 0x00, 0x00, 0x8B, 0x43, 0x08,
    0x8B, 0xE8, 0x2B, 0x2D, 0x1C, 0x99, 0xF9, 0x00, 0xF7, 0xDD, 0x1B, 0xED, 0x23, 0xE8, 0x8B, 0x03,
    0x3B, 0xC2, 0x89, 0x6C, 0x24, 0x14, 0x74, 0x6F, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00,
    0xC1, 0xE1, 0x04, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xF2, 0x74, 0x5A, 0x8B, 0x4E, 0x44,
    0x3B, 0xC8, 0x75, 0x53, 0x83, 0x7E, 0x4C, 0x05, 0x75, 0x4D, 0x3B, 0xCA, 0x74, 0x19, 0x8B, 0xC1,
    0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xC2,
    0x74, 0x05, 0x39, 0x48, 0x44, 0x74, 0x02, 0x33, 0xC0, 0x3B, 0xC6, 0x75, 0x2A, 0x39, 0x96, 0x28,
    0x8C, 0x00, 0x00, 0x7E, 0x22, 0x8B, 0xC6, 0xE8, 0x04, 0x98, 0x5D, 0x00, 0x84, 0xC0, 0x75, 0x17,
    0x8B, 0xFE, 0xE8, 0xC9, 0x98, 0x5D, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0xB2, 0x00, 0x00, 0x00, 0x8B,
    0x3D, 0x20, 0x99, 0xF9, 0x00, 0x33, 0xD2, 0x8B, 0x4B, 0x04, 0x3B, 0xCA, 0x74, 0x35, 0x8B, 0xC1,
    0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xC2,
    0x74, 0x0B, 0x39, 0x48, 0x44, 0x75, 0x06, 0x83, 0x78, 0x4C, 0x06, 0x74, 0x02, 0x33, 0xC0, 0x52,
    0x52, 0x8B, 0xF8, 0xE8, 0x38, 0x8D, 0x03, 0x00, 0x8B, 0x3D, 0x20, 0x99, 0xF9, 0x00, 0x83, 0xC4,
    0x08, 0x33, 0xD2, 0x39, 0x1D, 0x1C, 0x99, 0xF9, 0x00, 0x75, 0x14, 0x8B, 0x43, 0x08, 0x3B, 0xC3,
    0x75, 0x08, 0x89, 0x15, 0x1C, 0x99, 0xF9, 0x00, 0xEB, 0x1F, 0xA3, 0x1C, 0x99, 0xF9, 0x00, 0x8B,
    0x43, 0x0C, 0x3B, 0xC2, 0x74, 0x06, 0x8B, 0x4B, 0x08, 0x89, 0x48, 0x08, 0x8B, 0x43, 0x08, 0x3B,
    0xC2, 0x74, 0x06, 0x8B, 0x4B, 0x0C, 0x89, 0x48, 0x0C, 0x3B, 0xFA, 0x89, 0x53, 0x0C, 0x89, 0x53,
    0x08, 0x75, 0x15, 0x8B, 0xFB, 0x89, 0x5B, 0x08, 0x89, 0x5B, 0x0C, 0x89, 0x3D, 0x20, 0x99, 0xF9,
    0x00, 0x8B, 0xDD, 0xE9, 0xDB, 0xFE, 0xFF, 0xFF, 0x8B, 0x57, 0x0C, 0x89, 0x53, 0x0C, 0x89, 0x7B,
    0x08, 0x8B, 0x47, 0x0C, 0x89, 0x58, 0x08, 0x89, 0x5F, 0x0C, 0x8B, 0xDD, 0xE9, 0xC2, 0xFE, 0xFF,
    0xFF, 0x8D, 0x44, 0x24, 0x18, 0xE8, 0xA6, 0xFB, 0xFF, 0xFF, 0x8B, 0x5B, 0x04, 0x85, 0xDB, 0x74,
    0x6E, 0x8B, 0xCB, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x91, 0x64, 0x9C,
    0x14, 0x02, 0x85, 0xD2, 0x74, 0x0B, 0x39, 0x5A, 0x44, 0x75, 0x06, 0x83, 0x7A, 0x4C, 0x06, 0x74,
    0x02, 0x33, 0xD2, 0x85, 0xD2, 0x8D, 0x5E, 0x20, 0x74, 0x45, 0x8B, 0x82, 0xC4, 0x00, 0x00, 0x00,
    0x85, 0xC0, 0x74, 0x3B, 0x8B, 0x6C, 0x24, 0x20, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x4C, 0x24, 0x18, 0x89, 0x48, 0x2C, 0x8B, 0x4C, 0x24, 0x1C, 0x89, 0x48, 0x30, 0x89, 0x68,
    0x34, 0x8D, 0x78, 0x38, 0x8B, 0x00, 0xB9, 0x09, 0x00, 0x00, 0x00, 0x8B, 0xF3, 0xF3, 0xA5, 0x3B,
    0x82, 0xC4, 0x00, 0x00, 0x00, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xD5, 0x8B, 0x6C, 0x24, 0x14, 0x8B,
    0x3D, 0x20, 0x99, 0xF9, 0x00, 0x8B, 0xDD, 0xE9, 0x37, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B,
    0x83, 0xC4, 0x14, 0xC3,
];
#[allow(dead_code)]
const VEH_ATTACHED_BODY_CALLS: [usize; 4] = [0x87, 0x92, 0xd3, 0x155];
const TASKS_TICK_BODY: [u8; 420] = [
    0x83, 0xEC, 0x10, 0x53, 0x56, 0x8B, 0x35, 0x0C, 0x2C, 0xEE, 0x00, 0x85, 0xF6, 0x57, 0x8B, 0xC6,
    0x74, 0x69, 0x8B, 0x15, 0xAC, 0x43, 0xE8, 0x00, 0x83, 0xCF, 0xFF, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x83, 0x78, 0x14, 0x00, 0x7C, 0x4B, 0x8B, 0x48, 0x14, 0x85, 0xC9, 0x7C, 0x44, 0x3B, 0xCA, 0x7E,
    0x0C, 0x2B, 0xCA, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35, 0x7C, 0x36, 0xEB, 0x0C, 0x8B, 0xDA, 0x2B,
    0xD9, 0x81, 0xFB, 0x00, 0xE9, 0xA4, 0x35, 0x7D, 0x28, 0x01, 0x78, 0x10, 0x83, 0x78, 0x10, 0x00,
    0x7E, 0x1C, 0x8D, 0x8A, 0x9A, 0x02, 0x00, 0x00, 0x81, 0xF9, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x48,
    0x14, 0x7E, 0x0E, 0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x48, 0x14, 0xEB, 0x03, 0x89, 0x78,
    0x14, 0x8B, 0x00, 0x3B, 0xC6, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xA5, 0x33, 0xC9, 0x33, 0xD2, 0x33,
    0xFF, 0x33, 0xDB, 0x85, 0xF6, 0x89, 0x4C, 0x24, 0x0C, 0x89, 0x54, 0x24, 0x10, 0x89, 0x7C, 0x24,
    0x14, 0x89, 0x5C, 0x24, 0x18, 0x8B, 0xC6, 0x0F, 0x84, 0x7E, 0x00, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x8B, 0x3E, 0x3B, 0xFE, 0x75, 0x0D, 0xC7, 0x06, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x4E, 0x04, 0x33,
    0xF6, 0xEB, 0x1F, 0x8B, 0x50, 0x04, 0x85, 0xD2, 0x8D, 0x48, 0x04, 0x8B, 0xF7, 0x74, 0x02, 0x89,
    0x3A, 0x8B, 0x10, 0x85, 0xD2, 0x74, 0x05, 0x8B, 0x39, 0x89, 0x7A, 0x04, 0xC7, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x8B, 0x78, 0x10, 0x8B, 0x54, 0xBC, 0x0C, 0x85, 0xD2, 0x8D, 0x7C, 0xBC, 0x0C, 0xC7,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x75, 0x08, 0x89, 0x00, 0x89, 0x01, 0x89, 0x07, 0xEB, 0x16, 0x8B,
    0x7A, 0x04, 0x89, 0x39, 0x89, 0x10, 0x8B, 0x4A, 0x04, 0x89, 0x01, 0x8B, 0x50, 0x10, 0x8B, 0x4C,
    0x94, 0x0C, 0x89, 0x41, 0x04, 0x85, 0xF6, 0x8B, 0xC6, 0x75, 0x95, 0x8B, 0x5C, 0x24, 0x18, 0x8B,
    0x7C, 0x24, 0x14, 0x8B, 0x54, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x0C, 0x85, 0xC9, 0x8B, 0xC1, 0xA3,
    0x0C, 0x2C, 0xEE, 0x00, 0x75, 0x09, 0x8B, 0xC2, 0xA3, 0x0C, 0x2C, 0xEE, 0x00, 0xEB, 0x1C, 0x85,
    0xD2, 0x74, 0x18, 0x8B, 0x71, 0x04, 0x89, 0x16, 0x8B, 0x72, 0x04, 0x89, 0x0E, 0x8B, 0x72, 0x04,
    0x55, 0x8B, 0x69, 0x04, 0x89, 0x6A, 0x04, 0x89, 0x71, 0x04, 0x5D, 0x85, 0xC0, 0x75, 0x09, 0x8B,
    0xC7, 0xA3, 0x0C, 0x2C, 0xEE, 0x00, 0xEB, 0x1A, 0x85, 0xFF, 0x74, 0x16, 0x8B, 0x50, 0x04, 0x89,
    0x3A, 0x8B, 0x4F, 0x04, 0x89, 0x01, 0x8B, 0x50, 0x04, 0x8B, 0x4F, 0x04, 0x89, 0x57, 0x04, 0x89,
    0x48, 0x04, 0x85, 0xC0, 0x75, 0x0D, 0x5F, 0x5E, 0x89, 0x1D, 0x0C, 0x2C, 0xEE, 0x00, 0x5B, 0x83,
    0xC4, 0x10, 0xC3, 0x85, 0xDB, 0x74, 0x16, 0x8B, 0x48, 0x04, 0x89, 0x19, 0x8B, 0x53, 0x04, 0x89,
    0x02, 0x8B, 0x50, 0x04, 0x8B, 0x4B, 0x04, 0x89, 0x53, 0x04, 0x89, 0x48, 0x04, 0x5F, 0x5E, 0x5B,
    0x83, 0xC4, 0x10, 0xC3,
];
#[allow(dead_code)]
const TASKS_TICK_BODY_CALLS: [usize; 0] = [];
const EXPLOSIONS_BODY: [u8; 311] = [
    0xB8, 0x0C, 0x3B, 0x00, 0x00, 0xE8, 0x26, 0xDC, 0x7D, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33,
    0xC4, 0x89, 0x84, 0x24, 0x04, 0x3B, 0x00, 0x00, 0xA1, 0x08, 0x6D, 0x52, 0x02, 0x85, 0xC0, 0x53,
    0x55, 0x56, 0x57, 0x0F, 0x84, 0xF5, 0x00, 0x00, 0x00, 0x8B, 0xF8, 0x69, 0xC0, 0xD8, 0x01, 0x00,
    0x00, 0x50, 0x8D, 0x44, 0x24, 0x18, 0x68, 0x18, 0xE4, 0x61, 0x02, 0x50, 0xE8, 0x1F, 0xDC, 0x7D,
    0x00, 0x83, 0xC4, 0x0C, 0x85, 0xFF, 0xC7, 0x05, 0x08, 0x6D, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0x86, 0xC8, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x58, 0x8B, 0xDF, 0x80, 0x7E, 0xFD, 0x01,
    0x75, 0x2B, 0x85, 0xF6, 0x8B, 0xFE, 0x74, 0x27, 0x8B, 0x4E, 0x04, 0x85, 0xC9, 0x74, 0x20, 0x8B,
    0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02,
    0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48, 0x44, 0x75, 0x06, 0x8B, 0xE8, 0xEB, 0x04, 0x33, 0xFF, 0x33,
    0xED, 0x8B, 0x4E, 0xC4, 0x85, 0xC9, 0x74, 0x1D, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x09, 0x39, 0x48, 0x44, 0x75,
    0x04, 0x8B, 0xD0, 0xEB, 0x02, 0x33, 0xD2, 0x8B, 0x4E, 0xC0, 0x85, 0xC9, 0x74, 0x19, 0x8B, 0xC1,
    0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0,
    0x74, 0x05, 0x39, 0x48, 0x44, 0x74, 0x02, 0x33, 0xC0, 0x0F, 0xB6, 0x8E, 0x91, 0x01, 0x00, 0x00,
    0x51, 0x8B, 0x8E, 0x8C, 0x01, 0x00, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0x51, 0x0F, 0xB6, 0x4E, 0xFC,
    0x55, 0x57, 0x51, 0x8B, 0x4E, 0xF8, 0x6A, 0x00, 0x51, 0x8D, 0x4E, 0xEC, 0x51, 0x8D, 0x4E, 0xC8,
    0x51, 0x52, 0x8B, 0x56, 0xBC, 0x50, 0x52, 0xE8, 0x04, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x38, 0x81,
    0xC6, 0xD8, 0x01, 0x00, 0x00, 0x83, 0xEB, 0x01, 0x0F, 0x85, 0x3E, 0xFF, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x14, 0x3B, 0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x33, 0xCC, 0xE8, 0x2F, 0x33, 0x7D, 0x00,
    0x81, 0xC4, 0x0C, 0x3B, 0x00, 0x00, 0xC3,
];
#[allow(dead_code)]
const EXPLOSIONS_BODY_CALLS: [usize; 4] = [0x5, 0x3c, 0x107, 0x12b];
const WIRETAP_BODY: [u8; 379] = [
    0x83, 0xEC, 0x0C, 0xA0, 0x9B, 0x23, 0xFA, 0x00, 0x8A, 0xC8, 0x80, 0xE1, 0x01, 0x53, 0x88, 0x4C,
    0x24, 0x09, 0x8A, 0xD8, 0x8A, 0xC8, 0xC0, 0xE8, 0x04, 0x55, 0x24, 0x01, 0xC0, 0xE9, 0x03, 0x56,
    0x80, 0xE1, 0x01, 0x88, 0x44, 0x24, 0x13, 0xC0, 0xEB, 0x02, 0x57, 0xB8, 0xA8, 0x44, 0xDE, 0x00,
    0x80, 0xE3, 0x01, 0x88, 0x4C, 0x24, 0x16, 0xE8, 0x94, 0xFB, 0x51, 0x00, 0x8B, 0xF0, 0xB8, 0xD0,
    0x44, 0xDE, 0x00, 0xE8, 0x88, 0xFB, 0x51, 0x00, 0x8B, 0xF8, 0xB8, 0xF8, 0x44, 0xDE, 0x00, 0xE8,
    0x7C, 0xFB, 0x51, 0x00, 0x8B, 0xE8, 0xB8, 0x20, 0x45, 0xDE, 0x00, 0xE8, 0x70, 0xFB, 0x51, 0x00,
    0x8B, 0xD0, 0xA1, 0x40, 0xDA, 0xE9, 0x00, 0x3C, 0xFF, 0x89, 0x54, 0x24, 0x18, 0x74, 0x26, 0x8B,
    0xC8, 0xC1, 0xE9, 0x08, 0x74, 0x1F, 0x0F, 0xB6, 0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B,
    0x88, 0xAC, 0xB9, 0xEE, 0x00, 0x75, 0x0E, 0x83, 0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0xC6, 0x44,
    0x24, 0x14, 0x01, 0x75, 0x05, 0xC6, 0x44, 0x24, 0x14, 0x00, 0x85, 0xF6, 0x0F, 0x84, 0xD1, 0x00,
    0x00, 0x00, 0x85, 0xFF, 0x0F, 0x84, 0xC9, 0x00, 0x00, 0x00, 0x85, 0xED, 0x0F, 0x84, 0xC1, 0x00,
    0x00, 0x00, 0x85, 0xD2, 0x0F, 0x84, 0xB9, 0x00, 0x00, 0x00, 0x80, 0x7C, 0x24, 0x15, 0x00, 0x75,
    0x0E, 0x80, 0x7C, 0x24, 0x14, 0x00, 0x75, 0x07, 0xE8, 0x73, 0xEF, 0x51, 0x00, 0xEB, 0x07, 0x8B,
    0xC6, 0xE8, 0xEA, 0xEF, 0x51, 0x00, 0x84, 0xDB, 0x75, 0x0F, 0x38, 0x5C, 0x24, 0x14, 0x75, 0x09,
    0x8B, 0xF7, 0xE8, 0x59, 0xEF, 0x51, 0x00, 0xEB, 0x07, 0x8B, 0xC7, 0xE8, 0xD0, 0xEF, 0x51, 0x00,
    0x80, 0x7C, 0x24, 0x16, 0x00, 0x75, 0x10, 0x80, 0x7C, 0x24, 0x14, 0x00, 0x75, 0x09, 0x8B, 0xF5,
    0xE8, 0x3B, 0xEF, 0x51, 0x00, 0xEB, 0x07, 0x8B, 0xC5, 0xE8, 0xB2, 0xEF, 0x51, 0x00, 0x80, 0x7C,
    0x24, 0x17, 0x00, 0x74, 0x17, 0x8B, 0x44, 0x24, 0x18, 0xE8, 0xA2, 0xEF, 0x51, 0x00, 0xB0, 0x01,
    0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xE9, 0xA4, 0xC7, 0x23, 0x00, 0x80, 0x7C, 0x24, 0x15,
    0x00, 0x74, 0x29, 0x84, 0xDB, 0x74, 0x25, 0x80, 0x7C, 0x24, 0x16, 0x00, 0x74, 0x1E, 0x80, 0x7C,
    0x24, 0x14, 0x00, 0x75, 0x17, 0x8B, 0x74, 0x24, 0x18, 0xE8, 0xF2, 0xEE, 0x51, 0x00, 0x32, 0xC0,
    0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xE9, 0x74, 0xC7, 0x23, 0x00, 0x8B, 0x44, 0x24, 0x18,
    0xE8, 0x5B, 0xEF, 0x51, 0x00, 0x32, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xE9, 0x5D,
    0xC7, 0x23, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
];
#[allow(dead_code)]
const WIRETAP_BODY_CALLS: [usize; 13] = [0x37, 0x43, 0x4f, 0x5b, 0xc8, 0xd1, 0xe2, 0xeb, 0x100, 0x109, 0x119, 0x149, 0x160];
const CELLPHONE_BODY: [u8; 585] = [
    0x51, 0x53, 0x55, 0x56, 0x33, 0xED, 0x39, 0x2D, 0xB4, 0x3C, 0x22, 0x02, 0x57, 0xC6, 0x44, 0x24,
    0x13, 0x00, 0xC6, 0x05, 0x96, 0xB3, 0x13, 0x02, 0x00, 0xBF, 0x50, 0x52, 0xF3, 0x02, 0x0F, 0x8E,
    0x1F, 0x02, 0x00, 0x00, 0xBB, 0x68, 0x52, 0xF3, 0x02, 0xF6, 0x47, 0x3C, 0x04, 0x74, 0x68, 0x80,
    0x7C, 0x24, 0x13, 0x01, 0x0F, 0x84, 0xF4, 0x01, 0x00, 0x00, 0x57, 0xE8, 0x90, 0xF4, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0x83, 0xF8, 0x01, 0x74, 0x06, 0xF6, 0x47, 0x3C, 0x08, 0x74, 0x05, 0xC6, 0x44,
    0x24, 0x13, 0x01, 0x8A, 0x4F, 0x3C, 0xC0, 0xE9, 0x03, 0x80, 0xE1, 0x01, 0x83, 0xF8, 0x02, 0x88,
    0x0D, 0x96, 0xB3, 0x13, 0x02, 0x0F, 0x85, 0xC3, 0x01, 0x00, 0x00, 0xE8, 0x60, 0xCA, 0xD1, 0xFF,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x71, 0x50, 0xE8, 0x31, 0x99, 0xDA, 0xFF, 0x84,
    0xC0, 0x74, 0x67, 0xE8, 0x08, 0x17, 0xD9, 0xFF, 0x84, 0xC0, 0x74, 0x5E, 0xF6, 0x47, 0x3C, 0x80,
    0x75, 0x58, 0xE9, 0x97, 0x01, 0x00, 0x00, 0x8A, 0x07, 0x3C, 0xFF, 0x74, 0x4D, 0x8B, 0x0F, 0xC1,
    0xE9, 0x08, 0x74, 0x46, 0x0F, 0xB6, 0xC0, 0x69, 0xC0, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x88, 0xAC,
    0xB9, 0xEE, 0x00, 0x75, 0x35, 0x83, 0xB8, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x74, 0x2C, 0x8B, 0x77,
    0x0C, 0x85, 0xF6, 0x0F, 0x84, 0x65, 0x01, 0x00, 0x00, 0x56, 0xE8, 0x21, 0xF2, 0xF1, 0xFF, 0x83,
    0xC4, 0x04, 0x84, 0xC0, 0x75, 0x0D, 0xE8, 0x15, 0xA8, 0xEF, 0xFF, 0x84, 0xC0, 0x0F, 0x84, 0x4B,
    0x01, 0x00, 0x00, 0x8B, 0xCF, 0xE8, 0x86, 0x9A, 0xA1, 0xFF, 0xF6, 0x47, 0x3C, 0x04, 0x74, 0x23,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x1A, 0x50, 0xE8, 0xB1, 0x98, 0xDA, 0xFF, 0x84,
    0xC0, 0x75, 0x10, 0xE8, 0x88, 0x16, 0xD9, 0xFF, 0x84, 0xC0, 0x74, 0x07, 0x8B, 0xCF, 0xE8, 0xDD,
    0xFD, 0xFF, 0xFF, 0x8B, 0x47, 0x34, 0x85, 0xC0, 0x74, 0x0A, 0x8B, 0x57, 0x38, 0x52, 0x57, 0xFF,
    0xD0, 0x83, 0xC4, 0x08, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x49, 0x80, 0x3D, 0x56,
    0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x37, 0x80,
    0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x2E, 0x8B, 0x47, 0x14, 0x83, 0xF8, 0xFF, 0x74, 0x39,
    0xC1, 0xE8, 0x10, 0xC1, 0xE0, 0x04, 0x05, 0xC0, 0xA9, 0xE7, 0x02, 0x74, 0x2C, 0x8B, 0x40, 0x0C,
    0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x50, 0xA1, 0x3C, 0x98, 0x52, 0x02, 0xE8, 0x2F, 0x9D, 0x27,
    0x00, 0x83, 0xC4, 0x10, 0xEB, 0x13, 0x83, 0x7F, 0x14, 0xFF, 0x74, 0x0D, 0x6A, 0x01, 0x53, 0x6A,
    0x04, 0xE8, 0x9A, 0xEE, 0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0,
    0x74, 0x28, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52,
    0x02, 0x00, 0x74, 0x16, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x0D, 0x8B, 0x47, 0x10,
    0x83, 0xF8, 0xFF, 0x74, 0x05, 0xE8, 0x26, 0xFF, 0xFB, 0xFF, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85,
    0xC0, 0xC7, 0x47, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0x74, 0x28, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02,
    0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x16, 0x80, 0xB8, 0xF8, 0x03,
    0x00, 0x00, 0x00, 0x74, 0x0D, 0x8B, 0x47, 0x14, 0x83, 0xF8, 0xFF, 0x74, 0x05, 0xE8, 0xEE, 0xFE,
    0xFB, 0xFF, 0xA1, 0xB4, 0x3C, 0x22, 0x02, 0x2B, 0xC5, 0x83, 0xE8, 0x01, 0x85, 0xC0, 0xC7, 0x47,
    0x14, 0xFF, 0xFF, 0xFF, 0xFF, 0x7E, 0x20, 0x8D, 0x0C, 0xC0, 0x03, 0xC9, 0x03, 0xC9, 0x03, 0xC9,
    0x51, 0x8D, 0x57, 0x48, 0x52, 0x57, 0xE8, 0x05, 0xFB, 0x22, 0x00, 0x83, 0xC4, 0x0C, 0x83, 0xED,
    0x01, 0x83, 0xEF, 0x48, 0x83, 0xEB, 0x48, 0x83, 0x2D, 0xB4, 0x3C, 0x22, 0x02, 0x01, 0x83, 0xC5,
    0x01, 0x83, 0xC7, 0x48, 0x83, 0xC3, 0x48, 0x3B, 0x2D, 0xB4, 0x3C, 0x22, 0x02, 0x0F, 0x8C, 0xE6,
    0xFD, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x59, 0xC3,
];
#[allow(dead_code)]
const CELLPHONE_BODY_CALLS: [usize; 15] = [0x3b, 0x6b, 0x7a, 0x83, 0xca, 0xd6, 0xe5, 0xfa, 0x103, 0x10e, 0x16c, 0x181, 0x1b5, 0x1ed, 0x216];
const HAVOK_BODY: [u8; 824] = [
    0xB8, 0xBC, 0x39, 0x00, 0x00, 0xE8, 0x06, 0x4F, 0x58, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33,
    0xC4, 0x89, 0x84, 0x24, 0xB4, 0x39, 0x00, 0x00, 0x53, 0x55, 0x56, 0x57, 0x8B, 0x3D, 0xBC, 0x19,
    0x97, 0x02, 0x33, 0xF6, 0x85, 0xFF, 0x89, 0x74, 0x24, 0x10, 0x0F, 0x84, 0xCA, 0x02, 0x00, 0x00,
    0x8B, 0x0F, 0x33, 0xDB, 0x8D, 0x44, 0x24, 0x3C, 0x89, 0x44, 0x24, 0x30, 0xC7, 0x44, 0x24, 0x28,
    0xF0, 0x8E, 0xE1, 0x00, 0xC7, 0x44, 0x24, 0x38, 0x10, 0x00, 0x00, 0x80, 0x89, 0x5C, 0x24, 0x34,
    0x88, 0x5C, 0x24, 0x2C, 0x8B, 0x11, 0x8B, 0x52, 0x3C, 0x53, 0x8D, 0x44, 0x24, 0x2C, 0x50, 0xFF,
    0xD2, 0x68, 0x80, 0x00, 0x00, 0x00, 0x8D, 0x84, 0x24, 0x44, 0x31, 0x00, 0x00, 0x53, 0x50, 0xE8,
    0x3C, 0x52, 0x58, 0x00, 0x8B, 0x44, 0x24, 0x40, 0x83, 0xC4, 0x0C, 0x3B, 0xC3, 0x0F, 0x86, 0x33,
    0x01, 0x00, 0x00, 0x8D, 0x2C, 0x76, 0x89, 0x5C, 0x24, 0x18, 0x8D, 0xAC, 0xAC, 0x40, 0x01, 0x00,
    0x00, 0x89, 0x44, 0x24, 0x14, 0x8B, 0x4C, 0x24, 0x30, 0x8B, 0x54, 0x24, 0x18, 0x8B, 0x44, 0x11,
    0x08, 0x0F, 0xBE, 0x48, 0x10, 0x03, 0xC8, 0x8B, 0x41, 0x78, 0x33, 0xD2, 0x85, 0xC0, 0x0F, 0x8E,
    0xF2, 0x00, 0x00, 0x00, 0x8B, 0x59, 0x74, 0x8B, 0xF3, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x83, 0x3E, 0x01, 0x74, 0x0F, 0x83, 0xC2, 0x01, 0x83, 0xC6, 0x10, 0x3B, 0xD0, 0x7C, 0xF1, 0xE9,
    0xD2, 0x00, 0x00, 0x00, 0x33, 0xD2, 0x85, 0xC0, 0x7E, 0x19, 0x8B, 0xF3, 0x8D, 0x64, 0x24, 0x00,
    0x83, 0x3E, 0x06, 0x0F, 0x84, 0xBD, 0x00, 0x00, 0x00, 0x83, 0xC2, 0x01, 0x83, 0xC6, 0x10, 0x3B,
    0xD0, 0x7C, 0xED, 0x6A, 0x01, 0x8D, 0x44, 0x24, 0x24, 0xE8, 0xB2, 0x77, 0xFE, 0xFF, 0x8B, 0x74,
    0x24, 0x20, 0x85, 0xF6, 0x0F, 0x84, 0x9C, 0x00, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x1C, 0x50, 0x56,
    0x57, 0xC7, 0x44, 0x24, 0x28, 0xFF, 0xFF, 0x00, 0x00, 0xE8, 0x72, 0xF4, 0xFF, 0xFF, 0x83, 0xC4,
    0x0C, 0x3C, 0x01, 0x75, 0x24, 0x0F, 0xB7, 0x44, 0x24, 0x1C, 0x8B, 0xC8, 0xC1, 0xE9, 0x03, 0x83,
    0xE0, 0x07, 0x8A, 0x14, 0x85, 0x58, 0x85, 0xDC, 0x00, 0x08, 0x94, 0x0C, 0x40, 0x31, 0x00, 0x00,
    0x8D, 0x8C, 0x0C, 0x40, 0x31, 0x00, 0x00, 0xEB, 0x5D, 0x56, 0x8B, 0xD7, 0xE8, 0x3F, 0xF5, 0xFF,
    0xFF, 0x0F, 0xB7, 0xC0, 0x0F, 0xB7, 0xC0, 0x8B, 0xC8, 0xC1, 0xE9, 0x03, 0x83, 0xE0, 0x07, 0x8A,
    0x14, 0x85, 0x58, 0x85, 0xDC, 0x00, 0x08, 0x94, 0x0C, 0x44, 0x31, 0x00, 0x00, 0x8D, 0x8C, 0x0C,
    0x44, 0x31, 0x00, 0x00, 0x8A, 0x4F, 0x0A, 0x83, 0xC4, 0x04, 0xF6, 0xC1, 0x02, 0x74, 0x05, 0xF6,
    0xC1, 0x01, 0x75, 0x22, 0x83, 0x44, 0x24, 0x10, 0x01, 0x8B, 0x57, 0x04, 0x8B, 0xC5, 0x80, 0xE1,
    0x04, 0x83, 0xC5, 0x0C, 0x80, 0xF9, 0x04, 0x0F, 0x94, 0xC1, 0xC6, 0x40, 0x08, 0x01, 0x89, 0x10,
    0x89, 0x70, 0x04, 0x88, 0x48, 0x09, 0x83, 0x44, 0x24, 0x18, 0x10, 0x83, 0x6C, 0x24, 0x14, 0x01,
    0x0F, 0x85, 0xDF, 0xFE, 0xFF, 0xFF, 0x0F, 0xB7, 0x47, 0x08, 0x33, 0xDB, 0x66, 0x3D, 0xFF, 0xFF,
    0x74, 0x62, 0x66, 0x89, 0x84, 0x24, 0xC0, 0x31, 0x00, 0x00, 0xBB, 0x01, 0x00, 0x00, 0x00, 0x33,
    0xD2, 0xEB, 0x0D, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0xB7, 0x8C, 0x54, 0xC0, 0x31, 0x00, 0x00, 0x0F, 0xB7, 0x04, 0xCD, 0x44, 0xD4, 0xF6, 0x01,
    0x66, 0x3D, 0xFF, 0xFF, 0x8D, 0x0C, 0xCD, 0x40, 0xD4, 0xF6, 0x01, 0x74, 0x0B, 0x66, 0x89, 0x84,
    0x5C, 0xC0, 0x31, 0x00, 0x00, 0x83, 0xC3, 0x01, 0x0F, 0xB7, 0x41, 0x06, 0x66, 0x3D, 0xFF, 0xFF,
    0x74, 0x0B, 0x66, 0x89, 0x84, 0x5C, 0xC0, 0x31, 0x00, 0x00, 0x83, 0xC3, 0x01, 0x83, 0xC2, 0x01,
    0x3B, 0xD3, 0x72, 0xBC, 0x33, 0xF6, 0x85, 0xDB, 0x89, 0x74, 0x24, 0x14, 0x76, 0x7C, 0x8B, 0x44,
    0x24, 0x10, 0x8D, 0x2C, 0x40, 0x8D, 0xAC, 0xAC, 0x40, 0x01, 0x00, 0x00, 0x8D, 0x64, 0x24, 0x00,
    0x0F, 0xB7, 0x84, 0x74, 0xC0, 0x31, 0x00, 0x00, 0x0F, 0xB7, 0xC0, 0x8B, 0xD0, 0xC1, 0xEA, 0x03,
    0x8A, 0x8C, 0x14, 0x40, 0x31, 0x00, 0x00, 0x8B, 0xD0, 0x83, 0xE2, 0x07, 0x84, 0x0C, 0x95, 0x58,
    0x85, 0xDC, 0x00, 0x75, 0x3A, 0x8B, 0x34, 0xC5, 0x40, 0xD4, 0xF6, 0x01, 0x56, 0x57, 0xE8, 0xED,
    0xF5, 0xFF, 0xFF, 0x8A, 0x57, 0x0A, 0x8B, 0x4F, 0x04, 0x83, 0x44, 0x24, 0x18, 0x01, 0x8B, 0xC5,
    0x80, 0xE2, 0x04, 0x83, 0xC4, 0x08, 0x83, 0xC5, 0x0C, 0x80, 0xFA, 0x04, 0x89, 0x08, 0x0F, 0x94,
    0xC1, 0x89, 0x70, 0x04, 0x8B, 0x74, 0x24, 0x14, 0xC6, 0x40, 0x08, 0x00, 0x88, 0x48, 0x09, 0x83,
    0xC6, 0x01, 0x3B, 0xF3, 0x89, 0x74, 0x24, 0x14, 0x72, 0x96, 0x8B, 0x74, 0x24, 0x38, 0x80, 0x67,
    0x0A, 0xFD, 0x85, 0xF6, 0xC7, 0x44, 0x24, 0x28, 0xF0, 0x8E, 0xE1, 0x00, 0x78, 0x25, 0x8B, 0x15,
    0xBC, 0xF2, 0x1B, 0x03, 0x52, 0x81, 0xE6, 0xFF, 0xFF, 0xFF, 0x3F, 0xFF, 0x15, 0x7C, 0x80, 0xDB,
    0x00, 0x8B, 0x4C, 0x24, 0x30, 0x6A, 0x17, 0xC1, 0xE6, 0x04, 0x56, 0x51, 0x8B, 0xC8, 0xE8, 0xDD,
    0xFB, 0x60, 0x00, 0x8B, 0x7F, 0x0C, 0x3B, 0x3D, 0xBC, 0x19, 0x97, 0x02, 0x8B, 0x74, 0x24, 0x10,
    0x74, 0x08, 0x85, 0xFF, 0x0F, 0x85, 0x36, 0xFD, 0xFF, 0xFF, 0x85, 0xF6, 0x76, 0x21, 0x8B, 0x7C,
    0x24, 0x10, 0x8D, 0xB4, 0x24, 0x40, 0x01, 0x00, 0x00, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0xD6, 0xE8, 0xD9, 0xF6, 0xFF, 0xFF, 0x83, 0xC6, 0x0C, 0x83, 0xEF, 0x01, 0x75, 0xF1, 0x8B,
    0x8C, 0x24, 0xC4, 0x39, 0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x33, 0xCC, 0xE8, 0x0E, 0xA4, 0x57,
    0x00, 0x81, 0xC4, 0xBC, 0x39, 0x00, 0x00, 0xC3,
];
#[allow(dead_code)]
const HAVOK_BODY_CALLS: [usize; 9] = [0x5, 0x6f, 0xf9, 0x119, 0x14c, 0x26e, 0x2de, 0x312, 0x32c];

const CHEATS_BODY: [u8; 506] = [
    0x83, 0xEC, 0x0C, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x44, 0x24, 0x08, 0x53, 0x55,
    0x56, 0x57, 0xE8, 0x59, 0xFA, 0xFF, 0xFF, 0xE8, 0x64, 0xE4, 0xFF, 0xFF, 0x80, 0x3D, 0xC3, 0xE0,
    0xE8, 0x00, 0x00, 0x0F, 0x84, 0x5A, 0x01, 0x00, 0x00, 0x83, 0x3D, 0x08, 0x7C, 0x52, 0x02, 0x00,
    0x0F, 0x8F, 0x4D, 0x01, 0x00, 0x00, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x89, 0x44, 0x24, 0x10, 0xE8,
    0x4C, 0x00, 0x35, 0x00, 0x89, 0x44, 0x24, 0x14, 0x33, 0xDB, 0x8B, 0xBB, 0x64, 0x7B, 0x52, 0x02,
    0x85, 0xFF, 0x8B, 0x74, 0x1C, 0x10, 0x0F, 0x84, 0xC6, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x75, 0x38,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0E, 0x50, 0xE8, 0x41, 0x81, 0x18, 0x00, 0x84,
    0xC0, 0x74, 0x04, 0xB0, 0x01, 0xEB, 0x02, 0x32, 0xC0, 0x8B, 0x4F, 0x08, 0x8B, 0x51, 0x68, 0x50,
    0x6A, 0x00, 0x6A, 0x04, 0xE8, 0x77, 0x16, 0x3F, 0x00, 0xC7, 0x83, 0x64, 0x7B, 0x52, 0x02, 0x00,
    0x00, 0x00, 0x00, 0xE9, 0xDC, 0x00, 0x00, 0x00, 0x8B, 0xAE, 0xD4, 0x00, 0x00, 0x00, 0x3B, 0xEF,
    0x0F, 0x84, 0xD1, 0x00, 0x00, 0x00, 0xE8, 0x05, 0x78, 0x2E, 0x00, 0x83, 0xF8, 0xFF, 0x0F, 0x85,
    0xC3, 0x00, 0x00, 0x00, 0x8B, 0x57, 0x08, 0x8B, 0x52, 0x68, 0xE8, 0xD1, 0x1A, 0x3F, 0x00, 0x84,
    0xC0, 0x0F, 0x84, 0xB0, 0x00, 0x00, 0x00, 0xF6, 0x86, 0x61, 0x11, 0x00, 0x00, 0x40, 0x74, 0x2C,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0E, 0x50, 0xE8, 0xD1, 0x80, 0x18, 0x00, 0x84,
    0xC0, 0x74, 0x04, 0xB0, 0x01, 0xEB, 0x02, 0x32, 0xC0, 0x50, 0x8B, 0x45, 0x08, 0x8B, 0x50, 0x68,
    0x6A, 0x00, 0x6A, 0x04, 0xE8, 0x07, 0x16, 0x3F, 0x00, 0x83, 0xC4, 0x0C, 0x6A, 0x00, 0x51, 0x8B,
    0x0D, 0x0C, 0x63, 0x3D, 0x03, 0x8B, 0xC4, 0x89, 0x08, 0x8B, 0x93, 0x64, 0x7B, 0x52, 0x02, 0x51,
    0x8B, 0x4A, 0x04, 0x8B, 0xC4, 0x56, 0x89, 0x08, 0xE8, 0xA3, 0x99, 0x35, 0x00, 0x83, 0xC4, 0x10,
    0xEB, 0x55, 0x85, 0xF6, 0x74, 0x51, 0xF6, 0x86, 0x61, 0x11, 0x00, 0x00, 0x40, 0x74, 0x48, 0xE8,
    0x7C, 0x77, 0x2E, 0x00, 0x83, 0xF8, 0xFF, 0x75, 0x3E, 0x8B, 0x96, 0xD4, 0x00, 0x00, 0x00, 0x8B,
    0x42, 0x08, 0x8B, 0x78, 0x68, 0x6A, 0x00, 0xE8, 0x64, 0x9C, 0x35, 0x00, 0xA1, 0x14, 0x8C, 0x52,
    0x02, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x0E, 0x50, 0xE8, 0x52, 0x80, 0x18, 0x00, 0x84, 0xC0,
    0x74, 0x04, 0xB0, 0x01, 0xEB, 0x02, 0x32, 0xC0, 0x50, 0x6A, 0x00, 0x6A, 0x04, 0x8B, 0xD7, 0xE8,
    0x8C, 0x15, 0x3F, 0x00, 0x83, 0xC4, 0x0C, 0x83, 0xC3, 0x04, 0x83, 0xFB, 0x08, 0x0F, 0x8C, 0xC7,
    0xFE, 0xFF, 0xFF, 0xE8, 0x08, 0xFE, 0x16, 0x00, 0x80, 0x3D, 0x5B, 0x7B, 0x52, 0x02, 0x00, 0x8A,
    0xD8, 0x75, 0x4E, 0x84, 0xDB, 0x74, 0x4A, 0x33, 0xFF, 0x39, 0x3D, 0x5C, 0x7B, 0x52, 0x02, 0x7E,
    0x40, 0xBE, 0xC0, 0x6A, 0x7A, 0x02, 0x80, 0x7E, 0xAB, 0x00, 0x74, 0x27, 0x80, 0x7E, 0xA9, 0x00,
    0x74, 0x21, 0x8B, 0x06, 0x85, 0xC0, 0x74, 0x1B, 0x80, 0x7E, 0xA8, 0x00, 0x74, 0x06, 0x80, 0x7E,
    0xA9, 0x00, 0x74, 0x0F, 0x8D, 0x4E, 0x08, 0x6A, 0x01, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x08, 0xC6,
    0x46, 0xA9, 0x00, 0x83, 0xC7, 0x01, 0x83, 0xC6, 0x68, 0x3B, 0x3D, 0x5C, 0x7B, 0x52, 0x02, 0x7C,
    0xC5, 0x8B, 0x4C, 0x24, 0x18, 0x5F, 0x5E, 0x5D, 0x88, 0x1D, 0x5B, 0x7B, 0x52, 0x02, 0x5B, 0x33,
    0xCC, 0xE8, 0x19, 0xE3, 0x60, 0x00, 0x83, 0xC4, 0x0C, 0xC3,
];
#[allow(dead_code)]
const CHEATS_BODY_CALLS: [usize; 16] = [0x12, 0x17, 0x3f, 0x6a, 0x84, 0xa6, 0xba, 0xda, 0xf4, 0x118, 0x12f, 0x147, 0x159, 0x16f, 0x183, 0x1f1];

const RICH_BODY: [u8; 896] = [
    0x8B, 0x0D, 0xB4, 0x43, 0xE8, 0x00, 0x53, 0x33, 0xDB, 0xF6, 0x05, 0xAC, 0xFF, 0x48, 0x03, 0x01,
    0x56, 0x75, 0x09, 0x83, 0x0D, 0xAC, 0xFF, 0x48, 0x03, 0x01, 0xEB, 0x2C, 0xA1, 0xA8, 0xFF, 0x48,
    0x03, 0x3B, 0xC3, 0x7C, 0x23, 0x3B, 0xC1, 0x7E, 0x0F, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35,
    0x0F, 0x8C, 0xF9, 0x02, 0x00, 0x00, 0xEB, 0x10, 0x8B, 0xD1, 0x2B, 0xD0, 0x81, 0xFA, 0x00, 0xE9,
    0xA4, 0x35, 0x0F, 0x8D, 0xE7, 0x02, 0x00, 0x00, 0x8D, 0x81, 0xE8, 0x03, 0x00, 0x00, 0x3D, 0x00,
    0xD2, 0x49, 0x6B, 0xBE, 0x01, 0x00, 0x00, 0x00, 0xA3, 0xA8, 0xFF, 0x48, 0x03, 0x7E, 0x0A, 0x2D,
    0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xA8, 0xFF, 0x48, 0x03, 0xA1, 0xB0, 0x8D, 0xE9, 0x00, 0x55, 0x33,
    0xED, 0x83, 0xF8, 0xFF, 0x88, 0x98, 0xA8, 0xAE, 0x47, 0x03, 0x75, 0x28, 0x5D, 0x5E, 0x89, 0x1D,
    0x84, 0xAE, 0x47, 0x03, 0x89, 0x1D, 0x88, 0xAE, 0x47, 0x03, 0x89, 0x1D, 0x8C, 0xAE, 0x47, 0x03,
    0x89, 0x1D, 0x90, 0xAE, 0x47, 0x03, 0x89, 0x1D, 0x94, 0xAE, 0x47, 0x03, 0x89, 0x1D, 0x98, 0xAE,
    0x47, 0x03, 0x5B, 0xC3, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x0A, 0x8A, 0x0D, 0x49,
    0x8B, 0x52, 0x02, 0x84, 0xC9, 0x74, 0x27, 0xE8, 0xD4, 0xD6, 0x33, 0x00, 0x85, 0xC0, 0x74, 0x10,
    0x39, 0x35, 0x88, 0xAE, 0x47, 0x03, 0x74, 0x16, 0x89, 0x35, 0x88, 0xAE, 0x47, 0x03, 0xEB, 0x0E,
    0x39, 0x1D, 0x88, 0xAE, 0x47, 0x03, 0x74, 0x06, 0x89, 0x1D, 0x88, 0xAE, 0x47, 0x03, 0xA1, 0x7C,
    0x6B, 0xE9, 0x00, 0x83, 0xF8, 0xFF, 0x57, 0x8B, 0x3D, 0x84, 0xAE, 0x47, 0x03, 0x7E, 0x4F, 0x8B,
    0x04, 0x85, 0x60, 0xE7, 0x44, 0x01, 0x83, 0xF8, 0x06, 0x75, 0x13, 0x39, 0x05, 0x00, 0x6C, 0xE9,
    0x00, 0x0F, 0x85, 0x26, 0x02, 0x00, 0x00, 0x8B, 0xE8, 0xE9, 0x0F, 0x02, 0x00, 0x00, 0x83, 0xF8,
    0x02, 0x75, 0x07, 0x8B, 0xEE, 0xE9, 0x03, 0x02, 0x00, 0x00, 0x83, 0xF8, 0x08, 0x75, 0x10, 0x38,
    0x1D, 0xCE, 0x8C, 0x52, 0x02, 0x74, 0x08, 0x8D, 0x68, 0x02, 0xE9, 0xEE, 0x01, 0x00, 0x00, 0x83,
    0xF8, 0x09, 0x75, 0x0A, 0xBD, 0x06, 0x00, 0x00, 0x00, 0xE9, 0xDF, 0x01, 0x00, 0x00, 0xE8, 0x1D,
    0xD1, 0xFF, 0xFF, 0x83, 0xF8, 0x05, 0x0F, 0x85, 0xD1, 0x01, 0x00, 0x00, 0xA1, 0x00, 0x6C, 0xE9,
    0x00, 0x83, 0xF8, 0x0D, 0x0F, 0x87, 0xC3, 0x01, 0x00, 0x00, 0xFF, 0x24, 0x85, 0x74, 0xA7, 0x69,
    0x00, 0xBD, 0x03, 0x00, 0x00, 0x00, 0xE9, 0xB2, 0x01, 0x00, 0x00, 0xBD, 0x02, 0x00, 0x00, 0x00,
    0xE9, 0xA8, 0x01, 0x00, 0x00, 0xE8, 0x76, 0xD9, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x04,
    0x0F, 0x87, 0x97, 0x01, 0x00, 0x00, 0xFF, 0x24, 0x85, 0xAC, 0xA7, 0x69, 0x00, 0xBD, 0x04, 0x00,
    0x00, 0x00, 0xE9, 0x86, 0x01, 0x00, 0x00, 0xBD, 0x05, 0x00, 0x00, 0x00, 0xE9, 0x7C, 0x01, 0x00,
    0x00, 0x8B, 0x15, 0xD4, 0x03, 0x17, 0x02, 0xBD, 0x07, 0x00, 0x00, 0x00, 0xE8, 0x0F, 0x23, 0x1F,
    0x00, 0x3B, 0xC3, 0x0F, 0x84, 0x64, 0x01, 0x00, 0x00, 0x8B, 0x48, 0x04, 0x39, 0x0D, 0x8C, 0xAE,
    0x47, 0x03, 0x75, 0x04, 0x3B, 0xFD, 0x74, 0x06, 0x89, 0x0D, 0x8C, 0xAE, 0x47, 0x03, 0x8B, 0x15,
    0x78, 0x6B, 0x13, 0x02, 0x8B, 0xCA, 0x3B, 0xCB, 0xBE, 0x01, 0x00, 0x00, 0x00, 0x74, 0x16, 0x8B,
    0x40, 0x04, 0x39, 0x41, 0x04, 0x7E, 0x03, 0x83, 0xC6, 0x01, 0x8B, 0x49, 0x24, 0x3B, 0xCA, 0x74,
    0x04, 0x3B, 0xCB, 0x75, 0xED, 0x3B, 0x35, 0x90, 0xAE, 0x47, 0x03, 0x75, 0x08, 0x3B, 0xFD, 0x0F,
    0x84, 0x28, 0x01, 0x00, 0x00, 0x89, 0x35, 0x90, 0xAE, 0x47, 0x03, 0xE9, 0x0D, 0x01, 0x00, 0x00,
    0x8B, 0x15, 0xD4, 0x03, 0x17, 0x02, 0x38, 0x9A, 0xA8, 0x0F, 0x00, 0x00, 0x8B, 0x0D, 0x74, 0x6B,
    0x13, 0x02, 0xBD, 0x08, 0x00, 0x00, 0x00, 0x8B, 0xC1, 0x75, 0x3B, 0x3B, 0xC3, 0x74, 0x11, 0x90,
    0x39, 0x58, 0x04, 0x74, 0x2D, 0x8B, 0x40, 0x0C, 0x3B, 0xC1, 0x74, 0x04, 0x3B, 0xC3, 0x75, 0xF0,
    0x33, 0xF6, 0x8B, 0xC1, 0x3B, 0xC3, 0x74, 0x50, 0xBA, 0x01, 0x00, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x39, 0x50, 0x04, 0x74, 0x45, 0x8B, 0x40, 0x0C, 0x3B, 0xC1, 0x74, 0x3C, 0x3B, 0xC3, 0x75, 0xF0,
    0xEB, 0x36, 0x8B, 0xF0, 0xEB, 0xDC, 0x3B, 0xC3, 0x74, 0x16, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x70, 0x04, 0x74, 0x7E, 0x8B, 0x40, 0x0C, 0x3B, 0xC1, 0x74, 0x04, 0x3B, 0xC3, 0x75, 0xF0,
    0x33, 0xF6, 0x8B, 0xC1, 0x3B, 0xC3, 0x74, 0x10, 0x39, 0x58, 0x04, 0x74, 0x0D, 0x8B, 0x40, 0x0C,
    0x3B, 0xC1, 0x74, 0x04, 0x3B, 0xC3, 0x75, 0xF0, 0x33, 0xC0, 0x33, 0xD2, 0x3B, 0xF3, 0x74, 0x03,
    0x8B, 0x56, 0x08, 0x33, 0xC9, 0x3B, 0xC3, 0x74, 0x03, 0x8B, 0x48, 0x08, 0x3B, 0x15, 0x94, 0xAE,
    0x47, 0x03, 0xA1, 0x84, 0xAE, 0x47, 0x03, 0x75, 0x05, 0x83, 0xF8, 0x08, 0x74, 0x06, 0x89, 0x15,
    0x94, 0xAE, 0x47, 0x03, 0x3B, 0x0D, 0x98, 0xAE, 0x47, 0x03, 0x75, 0x05, 0x83, 0xF8, 0x08, 0x74,
    0x06, 0x89, 0x0D, 0x98, 0xAE, 0x47, 0x03, 0x3B, 0xD1, 0x7E, 0x1C, 0x39, 0x1D, 0x9C, 0xAE, 0x47,
    0x03, 0x75, 0x05, 0x83, 0xF8, 0x08, 0x74, 0x45, 0x89, 0x1D, 0x9C, 0xAE, 0x47, 0x03, 0x83, 0xF8,
    0x08, 0xEB, 0x2C, 0x8B, 0xF0, 0xEB, 0x8B, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x7C, 0x05, 0xB9, 0x02,
    0x00, 0x00, 0x00, 0x39, 0x0D, 0x9C, 0xAE, 0x47, 0x03, 0x75, 0x05, 0x83, 0xF8, 0x08, 0x74, 0x1D,
    0x89, 0x0D, 0x9C, 0xAE, 0x47, 0x03, 0x83, 0xF8, 0x08, 0xEB, 0x04, 0x33, 0xED, 0x3B, 0xFD, 0x74,
    0x0C, 0x89, 0x2D, 0xC4, 0xE2, 0x12, 0x02, 0x89, 0x2D, 0x84, 0xAE, 0x47, 0x03, 0x5F, 0x5D, 0x5E,
    0x5B, 0xC3, 0x8B, 0xFF, 0xA1, 0xA5, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00,
    0xAB, 0xA5, 0x69, 0x00, 0xB5, 0xA5, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00, 0x74, 0xA5, 0x69, 0x00,
    0x5B, 0xA7, 0x69, 0x00, 0x5B, 0xA7, 0x69, 0x00, 0x5B, 0xA7, 0x69, 0x00, 0x5B, 0xA7, 0x69, 0x00,
    0xE1, 0xA5, 0x69, 0x00, 0x50, 0xA6, 0x69, 0x00, 0x50, 0xA6, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00,
    0xCD, 0xA5, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00, 0xAB, 0xA5, 0x69, 0x00, 0xD7, 0xA5, 0x69, 0x00,
];
#[allow(dead_code)]
const RICH_BODY_CALLS: [usize; 4] = [0xb7, 0x13e, 0x175, 0x1ac];

const GARAGE_BODY: [u8; 1089] = [
    0x81, 0xEC, 0x1C, 0x02, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x14, 0x02, 0x00, 0x00, 0x53, 0x8B, 0x1D, 0xD4, 0x03, 0x17, 0x02, 0x55, 0x56, 0x57, 0x89, 0x5C,
    0x24, 0x1C, 0xE8, 0xC9, 0xEC, 0xFF, 0xFF, 0x33, 0xF6, 0x39, 0x35, 0xE0, 0x74, 0x52, 0x02, 0x7E,
    0x1F, 0xBF, 0xE0, 0x7B, 0x79, 0x02, 0x57, 0xE8, 0xA4, 0xF1, 0xFF, 0xFF, 0x83, 0xC6, 0x01, 0x83,
    0xC4, 0x04, 0x81, 0xC7, 0x94, 0x00, 0x00, 0x00, 0x3B, 0x35, 0xE0, 0x74, 0x52, 0x02, 0x7C, 0xE6,
    0x8B, 0x0D, 0x08, 0x93, 0x79, 0x02, 0x85, 0xC9, 0xBA, 0x05, 0x00, 0x00, 0x00, 0x74, 0x22, 0x8B,
    0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85,
    0xC0, 0x74, 0x0E, 0x39, 0x48, 0x44, 0x75, 0x09, 0x39, 0x50, 0x4C, 0x75, 0x04, 0x8B, 0xE8, 0xEB,
    0x08, 0x33, 0xED, 0x89, 0x2D, 0x08, 0x93, 0x79, 0x02, 0x8B, 0x0D, 0xF0, 0x74, 0x52, 0x02, 0x33,
    0xFF, 0x3B, 0xCF, 0x74, 0x2F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B,
    0x80, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xC7, 0x74, 0x1B, 0x39, 0x48, 0x44, 0x75, 0x16, 0x39, 0x50,
    0x4C, 0x75, 0x11, 0x6A, 0x02, 0x6A, 0x02, 0x6A, 0x02, 0x50, 0xB2, 0x01, 0xE8, 0x7F, 0x22, 0x09,
    0x00, 0x83, 0xC4, 0x10, 0xB8, 0x00, 0x93, 0x79, 0x02, 0xE8, 0xF2, 0xEA, 0xFF, 0xFF, 0xA1, 0x04,
    0x93, 0x79, 0x02, 0x3B, 0xC7, 0x0F, 0x84, 0x47, 0x03, 0x00, 0x00, 0x8B, 0x40, 0x10, 0x3B, 0xC7,
    0x0F, 0x84, 0x3C, 0x03, 0x00, 0x00, 0x39, 0x3D, 0xAC, 0x99, 0x40, 0x01, 0x75, 0x23, 0x83, 0xC3,
    0x14, 0x53, 0xE8, 0x69, 0xE0, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x0F, 0x84, 0x26, 0x03,
    0x00, 0x00, 0xC7, 0x05, 0xAC, 0x99, 0x40, 0x01, 0x01, 0x00, 0x00, 0x00, 0xE9, 0x17, 0x03, 0x00,
    0x00, 0x8D, 0x4C, 0x24, 0x1A, 0x51, 0x8D, 0x54, 0x24, 0x19, 0x52, 0x8D, 0x44, 0x24, 0x1F, 0x50,
    0x8D, 0x4C, 0x24, 0x25, 0x51, 0x8D, 0x54, 0x24, 0x26, 0x52, 0x8D, 0x44, 0x24, 0x2C, 0x50, 0xE8,
    0x7C, 0xFC, 0xFF, 0xFF, 0x83, 0xC4, 0x18, 0x3B, 0xEF, 0x0F, 0x84, 0xB5, 0x00, 0x00, 0x00, 0x80,
    0xBD, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x75, 0x0A, 0x8B, 0x85, 0xE8, 0x6C, 0x00, 0x00, 0x3B, 0xC7,
    0x7E, 0x13, 0x8B, 0x85, 0xBC, 0x8A, 0x00, 0x00, 0xE8, 0xE3, 0xBD, 0x40, 0x00, 0x3B, 0xC7, 0x0F,
    0x85, 0x8F, 0x00, 0x00, 0x00, 0x80, 0x7C, 0x24, 0x16, 0x00, 0x0F, 0x85, 0x84, 0x00, 0x00, 0x00,
    0x8B, 0x35, 0x04, 0x93, 0x79, 0x02, 0xF6, 0x86, 0x90, 0x00, 0x00, 0x00, 0x01, 0x75, 0x7B, 0x80,
    0x7C, 0x24, 0x15, 0x01, 0x75, 0x0D, 0x8B, 0xC6, 0xE8, 0x63, 0xED, 0xFF, 0xFF, 0x8B, 0x35, 0x04,
    0x93, 0x79, 0x02, 0x89, 0x3D, 0x08, 0x93, 0x79, 0x02, 0x8B, 0x46, 0x08, 0x8B, 0xCD, 0xE8, 0x4D,
    0xDF, 0xFF, 0xFF, 0x8B, 0x0D, 0x04, 0x93, 0x79, 0x02, 0x8A, 0xD8, 0x8B, 0x41, 0x0C, 0x8B, 0xCD,
    0xE8, 0x3B, 0xDF, 0xFF, 0xFF, 0x84, 0xDB, 0x88, 0x44, 0x24, 0x17, 0x74, 0x18, 0x8B, 0x15, 0x04,
    0x93, 0x79, 0x02, 0x8B, 0x42, 0x08, 0x8B, 0x4C, 0x24, 0x1C, 0x8B, 0x71, 0x44, 0x8B, 0x78, 0x44,
    0xE8, 0xBB, 0x7E, 0x51, 0x00, 0x80, 0x7C, 0x24, 0x17, 0x00, 0x74, 0x18, 0x8B, 0x15, 0x04, 0x93,
    0x79, 0x02, 0x8B, 0x42, 0x0C, 0x8B, 0x4C, 0x24, 0x1C, 0x8B, 0x71, 0x44, 0x8B, 0x78, 0x44, 0xE8,
    0x9C, 0x7E, 0x51, 0x00, 0x8B, 0x35, 0x04, 0x93, 0x79, 0x02, 0xA1, 0xAC, 0x99, 0x40, 0x01, 0x83,
    0xE8, 0x01, 0x0F, 0x84, 0xFB, 0x01, 0x00, 0x00, 0x83, 0xE8, 0x01, 0x74, 0x68, 0x83, 0xE8, 0x01,
    0x0F, 0x85, 0x12, 0x02, 0x00, 0x00, 0x56, 0xE8, 0xA4, 0xC5, 0xFF, 0xFF, 0x8B, 0x0D, 0x04, 0x93,
    0x79, 0x02, 0xD9, 0x41, 0x20, 0x83, 0xC4, 0x04, 0xDC, 0x1D, 0x68, 0xC3, 0xE5, 0x00, 0xDF, 0xE0,
    0xF6, 0xC4, 0x05, 0x7B, 0x2A, 0x8B, 0x41, 0x1C, 0x85, 0xC0, 0x74, 0x23, 0x83, 0xB8, 0x94, 0x00,
    0x00, 0x00, 0x00, 0x74, 0x1A, 0x80, 0x7C, 0x24, 0x15, 0x00, 0x0F, 0x85, 0xD8, 0x01, 0x00, 0x00,
    0xC7, 0x05, 0xAC, 0x99, 0x40, 0x01, 0x01, 0x00, 0x00, 0x00, 0xE9, 0xC9, 0x01, 0x00, 0x00, 0xE8,
    0x1C, 0xEC, 0xFF, 0xFF, 0x33, 0xC0, 0xA3, 0x08, 0x93, 0x79, 0x02, 0xA3, 0x04, 0x93, 0x79, 0x02,
    0xE9, 0xB3, 0x01, 0x00, 0x00, 0x8A, 0x15, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xFA, 0x01, 0x75, 0x0D,
    0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x0F, 0x84, 0x9B, 0x01, 0x00, 0x00, 0x80, 0x7C, 0x24,
    0x19, 0x00, 0x75, 0x1F, 0x80, 0x7C, 0x24, 0x18, 0x00, 0x75, 0x60, 0x80, 0x7C, 0x24, 0x16, 0x00,
    0x75, 0x11, 0x80, 0x7C, 0x24, 0x15, 0x00, 0x74, 0x0A, 0xA1, 0x04, 0x93, 0x79, 0x02, 0xE8, 0x3D,
    0xEC, 0xFF, 0xFF, 0x8B, 0x15, 0x44, 0x51, 0x4A, 0x03, 0x85, 0xD2, 0x0F, 0x84, 0x67, 0x01, 0x00,
    0x00, 0x8B, 0xC2, 0x25, 0xFF, 0x1F, 0x00, 0x00, 0x83, 0xF8, 0x3C, 0x0F, 0x8D, 0x57, 0x01, 0x00,
    0x00, 0x8B, 0xC8, 0xC1, 0xE1, 0x04, 0x2B, 0xC8, 0x39, 0x14, 0x8D, 0x48, 0x6C, 0x9E, 0x02, 0x8D,
    0x0C, 0x8D, 0x28, 0x6C, 0x9E, 0x02, 0x0F, 0x85, 0x3C, 0x01, 0x00, 0x00, 0xB8, 0x03, 0x00, 0x00,
    0x00, 0xE8, 0x9A, 0x80, 0x24, 0x00, 0xE9, 0x2D, 0x01, 0x00, 0x00, 0x80, 0x3D, 0xF5, 0x74, 0x52,
    0x02, 0x00, 0x75, 0x2D, 0x85, 0xED, 0x74, 0x29, 0x8B, 0xC5, 0xE8, 0x91, 0x30, 0x56, 0x00, 0x84,
    0xC0, 0x74, 0x1E, 0xD9, 0xE8, 0x6A, 0x01, 0x6A, 0x01, 0x51, 0x83, 0xC9, 0xFF, 0xD9, 0x1C, 0x24,
    0x8B, 0xD5, 0xE8, 0x19, 0x5A, 0x55, 0x00, 0x83, 0xC4, 0x0C, 0xC6, 0x05, 0xF5, 0x74, 0x52, 0x02,
    0x01, 0x8B, 0x1D, 0x04, 0x93, 0x79, 0x02, 0x8B, 0xC3, 0xE8, 0x62, 0xE3, 0xFF, 0xFF, 0x84, 0xC0,
    0x0F, 0x84, 0x6D, 0xFF, 0xFF, 0xFF, 0x8B, 0x8D, 0x48, 0x8F, 0x00, 0x00, 0xC1, 0xE9, 0x14, 0x81,
    0xE1, 0x01, 0xFF, 0xFF, 0xFF, 0x51, 0xE8, 0x45, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0,
    0x0F, 0x84, 0x4D, 0xFF, 0xFF, 0xFF, 0x80, 0x7C, 0x24, 0x1A, 0x00, 0x0F, 0x85, 0x42, 0xFF, 0xFF,
    0xFF, 0x8B, 0x74, 0x24, 0x1C, 0x33, 0xFF, 0xE8, 0xB4, 0x3A, 0x46, 0x00, 0x84, 0xC0, 0x75, 0x12,
    0x8B, 0xC6, 0xE8, 0x39, 0x41, 0x46, 0x00, 0x84, 0xC0, 0x75, 0x07, 0x8B, 0xC3, 0xE9, 0x1C, 0xFF,
    0xFF, 0xFF, 0x8D, 0x54, 0x24, 0x1B, 0x52, 0x8D, 0x44, 0x24, 0x24, 0x68, 0x00, 0x01, 0x00, 0x00,
    0x50, 0xE8, 0x1A, 0x5A, 0xFA, 0xFF, 0x83, 0xC4, 0x0C, 0x84, 0xC0, 0x0F, 0x85, 0x02, 0xFF, 0xFF,
    0xFF, 0x38, 0x05, 0x96, 0xB3, 0x13, 0x02, 0x0F, 0x85, 0xF6, 0xFE, 0xFF, 0xFF, 0x8B, 0xC6, 0xE8,
    0xFC, 0x40, 0x46, 0x00, 0x84, 0xC0, 0x0F, 0x85, 0xE7, 0xFE, 0xFF, 0xFF, 0xBF, 0x34, 0x66, 0xDE,
    0x00, 0xBE, 0x74, 0x66, 0xDE, 0x00, 0xE8, 0xD5, 0xF8, 0x29, 0x00, 0x8B, 0xF0, 0xA1, 0x0C, 0xD2,
    0xEA, 0x00, 0x68, 0x44, 0x51, 0x4A, 0x03, 0xE8, 0xF4, 0x84, 0x24, 0x00, 0x83, 0xC4, 0x04, 0x6A,
    0x00, 0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x07, 0xE8, 0x42, 0x20, 0x16, 0x00, 0x83, 0xC4,
    0x14, 0xEB, 0x25, 0xE8, 0x88, 0x6C, 0x52, 0x00, 0x84, 0xC0, 0x75, 0x1C, 0xA2, 0xF5, 0x74, 0x52,
    0x02, 0xC7, 0x05, 0xAC, 0x99, 0x40, 0x01, 0x02, 0x00, 0x00, 0x00, 0xE8, 0x20, 0x0C, 0x00, 0x00,
    0xEB, 0x06, 0x89, 0x3D, 0xAC, 0x99, 0x40, 0x01, 0x8B, 0x8C, 0x24, 0x24, 0x02, 0x00, 0x00, 0x5F,
    0x5E, 0x5D, 0x5B, 0x33, 0xCC, 0xE8, 0xF5, 0x0D, 0x74, 0x00, 0x81, 0xC4, 0x1C, 0x02, 0x00, 0x00,
    0xC3,
];
#[allow(dead_code)]
const GARAGE_BODY_CALLS: [usize; 30] = [0x22, 0x37, 0xbc, 0xc9, 0xf2, 0x12f, 0x158, 0x188, 0x19e, 0x1b0, 0x1d0, 0x1ef, 0x217, 0x25f, 0x2ae, 0x2f1, 0x30a, 0x322, 0x339, 0x356, 0x377, 0x382, 0x3a1, 0x3bf, 0x3d6, 0x3e7, 0x3f9, 0x403, 0x41b, 0x435];
const TRIGGERS_BODY: [u8; 310] = [
    0x8B, 0x0D, 0x0C, 0x9E, 0x52, 0x02, 0x85, 0xC9, 0x55, 0x56, 0x57, 0x74, 0x66, 0xD9, 0xE8, 0xD8,
    0x1D, 0xA0, 0x6D, 0x52, 0x02, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x7A, 0x44, 0x8B, 0x81, 0x40, 0x01,
    0x00, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x6A, 0x01, 0x8D, 0x48, 0x28, 0x6A, 0x01, 0x51, 0x8B, 0x0D,
    0xD4, 0x03, 0x17, 0x02, 0x83, 0xC0, 0x1C, 0x50, 0xE8, 0xF3, 0xF0, 0xF6, 0xFF, 0xD9, 0xEE, 0x83,
    0xC4, 0x14, 0xD9, 0x1C, 0x24, 0x68, 0xF4, 0x01, 0x00, 0x00, 0x32, 0xD2, 0xC7, 0x05, 0x0C, 0x9E,
    0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xC5, 0x4E, 0xA6, 0xFF, 0x83, 0xC4, 0x08, 0xEB, 0x13,
    0x83, 0x3D, 0x9C, 0x6D, 0x52, 0x02, 0x01, 0x74, 0x0A, 0xC7, 0x05, 0x0C, 0x9E, 0x52, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x8B, 0x2D, 0x08, 0x9E, 0x52, 0x02, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x85, 0xED, 0x0F, 0x84, 0xA6, 0x00, 0x00, 0x00, 0x8B, 0x45, 0x08, 0x8B, 0xF0, 0x2B, 0x35, 0x08,
    0x9E, 0x52, 0x02, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF0, 0x8B, 0x45, 0x00, 0x85, 0xC0, 0x0F, 0x84,
    0x83, 0x00, 0x00, 0x00, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B,
    0xBA, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x6E, 0x39, 0x47, 0x44, 0x75, 0x69, 0x83, 0x7F,
    0x4C, 0x07, 0x75, 0x63, 0xF6, 0x87, 0x10, 0x01, 0x00, 0x00, 0x80, 0x75, 0x5A, 0x8B, 0x45, 0x04,
    0x85, 0xC0, 0x74, 0x1A, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B,
    0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x05, 0x39, 0x41, 0x44, 0x74, 0x04, 0x33, 0xC0,
    0xEB, 0x0E, 0x8B, 0x41, 0x4C, 0x83, 0xE8, 0x01, 0xF7, 0xD8, 0x1B, 0xC0, 0xF7, 0xD0, 0x23, 0xC1,
    0x6A, 0x01, 0x57, 0xE8, 0x58, 0xEE, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x84, 0xC0, 0x74, 0x18, 0xF6,
    0x87, 0x10, 0x01, 0x00, 0x00, 0x04, 0x75, 0x0F, 0x8B, 0x55, 0x04, 0x8B, 0x7F, 0x44, 0x52, 0xE8,
    0x9C, 0x0A, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x8B, 0xEE, 0xE9, 0x52, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E,
    0x5D, 0xE9, 0x8A, 0x23, 0x00, 0x00,
];
#[allow(dead_code)]
const TRIGGERS_BODY_CALLS: [usize; 4] = [0x38, 0x56, 0x103, 0x11f];

const VEHINT_BODY: [u8; 353] = [
    0x51, 0x53, 0x55, 0x56, 0x57, 0xBF, 0xB8, 0x92, 0x14, 0x03, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0F, 0x85, 0xC9, 0x0F, 0x84, 0x29, 0x01, 0x00, 0x00, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00,
    0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48,
    0x44, 0x75, 0x06, 0x83, 0x78, 0x4C, 0x05, 0x74, 0x0A, 0xE8, 0x12, 0xE2, 0x00, 0x00, 0xE9, 0x00,
    0x01, 0x00, 0x00, 0x33, 0xF6, 0x89, 0x74, 0x24, 0x10, 0x8D, 0xAF, 0x4C, 0x03, 0x00, 0x00, 0x8B,
    0x5D, 0x00, 0x85, 0xDB, 0x74, 0x74, 0x8B, 0xCB, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1,
    0x04, 0x8B, 0x81, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x58, 0x44, 0x75, 0x06,
    0x83, 0x78, 0x4C, 0x01, 0x74, 0x3E, 0x8B, 0xC5, 0xE8, 0xB3, 0xC7, 0xFF, 0xFF, 0x83, 0x87, 0x48,
    0x03, 0x00, 0x00, 0xFF, 0x85, 0xDB, 0x74, 0x42, 0x8B, 0xD3, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00,
    0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x2D, 0x39, 0x58, 0x44,
    0x75, 0x28, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x22, 0xC7, 0x80, 0xE0, 0x10, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xEB, 0x16, 0x83, 0xB8, 0x74, 0x0D, 0x00, 0x00, 0x00, 0x75, 0x0D, 0x8B, 0xC6, 0x8B,
    0xF7, 0xE8, 0xAA, 0xE2, 0x00, 0x00, 0x8B, 0x74, 0x24, 0x10, 0x83, 0xC6, 0x01, 0x81, 0xC5, 0xA8,
    0x00, 0x00, 0x00, 0x83, 0xFE, 0x08, 0x89, 0x74, 0x24, 0x10, 0x0F, 0x8C, 0x6F, 0xFF, 0xFF, 0xFF,
    0x57, 0xE8, 0x5A, 0x08, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x8B, 0xC7, 0xE8, 0x30, 0x06, 0x00, 0x00,
    0x33, 0xDB, 0x8D, 0xB7, 0x50, 0x03, 0x00, 0x00, 0x83, 0x7E, 0xFC, 0x00, 0x74, 0x37, 0x8B, 0x06,
    0x83, 0xE8, 0x01, 0x74, 0x27, 0x83, 0xE8, 0x01, 0x74, 0x17, 0x83, 0xE8, 0x01, 0x75, 0x26, 0x83,
    0x7E, 0x04, 0x21, 0x75, 0x20, 0x53, 0x57, 0xE8, 0x74, 0xF8, 0x00, 0x00, 0x83, 0xC4, 0x08, 0xEB,
    0x14, 0x8B, 0xC3, 0x8B, 0xCF, 0xE8, 0x86, 0xCB, 0x00, 0x00, 0xEB, 0x09, 0x8B, 0xCB, 0x8B, 0xC7,
    0xE8, 0xEB, 0x8F, 0x00, 0x00, 0x83, 0xC3, 0x01, 0x81, 0xC6, 0xA8, 0x00, 0x00, 0x00, 0x83, 0xFB,
    0x08, 0x7C, 0xB5, 0x81, 0xC7, 0x84, 0x0C, 0x00, 0x00, 0x81, 0xFF, 0x38, 0x23, 0x16, 0x03, 0x0F,
    0x8C, 0xBB, 0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x04, 0xE9, 0x1F, 0x21, 0x00,
    0x00,
];
#[allow(dead_code)]
const VEHINT_BODY_CALLS: [usize; 8] = [0x39, 0x78, 0xc1, 0xe1, 0xeb, 0x117, 0x125, 0x130];
const TASKS_EXPIRE_BODY: [u8; 570] = [
    0x83, 0xEC, 0x10, 0x53, 0x55, 0x56, 0x57, 0x8B, 0x3D, 0x00, 0x2C, 0xEE, 0x00, 0x8D, 0x49, 0x00,
    0x33, 0xD2, 0x3B, 0xFA, 0x0F, 0x84, 0x18, 0x02, 0x00, 0x00, 0x8B, 0x07, 0x8B, 0xC8, 0x2B, 0x0D,
    0x00, 0x2C, 0xEE, 0x00, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xC8, 0x8B, 0x47, 0x28, 0x3B, 0xC2, 0x89,
    0x4C, 0x24, 0x1C, 0x7C, 0x2E, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC1, 0x7E, 0x16, 0x2B,
    0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x7C, 0x1B, 0x8B, 0xC7, 0xE8, 0x41, 0xFF, 0xFF, 0xFF, 0x8B,
    0x7C, 0x24, 0x1C, 0xEB, 0xBB, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8C, 0xBF,
    0x01, 0x00, 0x00, 0x8B, 0x4F, 0x08, 0x3B, 0xCA, 0x74, 0x1D, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00,
    0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xC2, 0x74, 0x09, 0x39, 0x48,
    0x44, 0x89, 0x44, 0x24, 0x10, 0x74, 0x04, 0x89, 0x54, 0x24, 0x10, 0x8B, 0x6F, 0x0C, 0x3B, 0xEA,
    0x74, 0x1D, 0x8B, 0xC5, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C,
    0x14, 0x02, 0x3B, 0xC2, 0x74, 0x09, 0x39, 0x68, 0x44, 0x89, 0x44, 0x24, 0x14, 0x74, 0x04, 0x89,
    0x54, 0x24, 0x14, 0x39, 0x54, 0x24, 0x10, 0x0F, 0x84, 0x65, 0x01, 0x00, 0x00, 0x39, 0x54, 0x24,
    0x14, 0x0F, 0x84, 0x5B, 0x01, 0x00, 0x00, 0x3B, 0xCA, 0x74, 0x64, 0x8B, 0xC1, 0x25, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0xB0, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xF2, 0x74, 0x50, 0x39,
    0x4E, 0x44, 0x75, 0x4B, 0x83, 0x7E, 0x4C, 0x01, 0x75, 0x45, 0x8B, 0x9E, 0x80, 0x00, 0x00, 0x00,
    0xF6, 0xC3, 0x0B, 0x75, 0x3A, 0x56, 0xE8, 0x55, 0x2F, 0x53, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0,
    0x75, 0x2D, 0xC1, 0xEB, 0x04, 0xF6, 0xC3, 0x01, 0x75, 0x25, 0xF7, 0x86, 0xC8, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x40, 0x00, 0x74, 0x19, 0x8B, 0x8E, 0xC0, 0x00, 0x00, 0x00, 0x8A, 0x57, 0x2C, 0xC1,
    0xE9, 0x07, 0xD0, 0xEA, 0x32, 0xCA, 0xF6, 0xC1, 0x01, 0x0F, 0x85, 0xF3, 0x00, 0x00, 0x00, 0x85,
    0xED, 0x74, 0x65, 0x8B, 0xC5, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0xB0, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x51, 0x39, 0x6E, 0x44, 0x75, 0x4C, 0x83, 0x7E, 0x4C, 0x01,
    0x75, 0x46, 0x8B, 0x9E, 0x80, 0x00, 0x00, 0x00, 0xF6, 0xC3, 0x0B, 0x75, 0x3B, 0x56, 0xE8, 0xED,
    0x2E, 0x53, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x2E, 0xC1, 0xEB, 0x04, 0xF6, 0xC3, 0x01,
    0x75, 0x26, 0xF7, 0x86, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x74, 0x1A, 0x8B, 0x8E,
    0xC0, 0x00, 0x00, 0x00, 0x8A, 0x57, 0x2C, 0xC1, 0xE9, 0x07, 0xC0, 0xEA, 0x02, 0x32, 0xCA, 0xF6,
    0xC1, 0x01, 0x0F, 0x85, 0x8A, 0x00, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x10, 0x8D, 0x47, 0x10, 0x8D,
    0x4A, 0x14, 0xE8, 0xA9, 0xC2, 0x78, 0x00, 0xD9, 0x5C, 0x24, 0x18, 0x83, 0x7A, 0x4C, 0x01, 0x75,
    0x11, 0x80, 0xBA, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x08, 0xD9, 0x05, 0xB8, 0x32, 0xDD, 0x00,
    0xEB, 0x06, 0xD9, 0x05, 0x34, 0xB9, 0xED, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x18,
    0xD9, 0x44, 0x24, 0x10, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x45, 0x8B, 0x54, 0x24,
    0x14, 0x8D, 0x47, 0x1C, 0x8D, 0x4A, 0x14, 0xE8, 0x64, 0xC2, 0x78, 0x00, 0xD9, 0x5C, 0x24, 0x18,
    0x83, 0x7A, 0x4C, 0x01, 0x75, 0x11, 0x80, 0xBA, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x08, 0xD9,
    0x05, 0xB8, 0x32, 0xDD, 0x00, 0xEB, 0x06, 0xD9, 0x05, 0x34, 0xB9, 0xED, 0x00, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x14, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05,
    0x7A, 0x07, 0x8B, 0xC7, 0xE8, 0x67, 0xFD, 0xFF, 0xFF, 0x8B, 0x7C, 0x24, 0x1C, 0xE9, 0xDE, 0xFD,
    0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x10, 0xC3,
];
#[allow(dead_code)]
const TASKS_EXPIRE_BODY_CALLS: [usize; 6] = [0x4a, 0xf6, 0x15e, 0x1a2, 0x1e7, 0x224];

const SCHED_EVENTS_BODY: [u8; 607] = [
    0x51, 0xA1, 0xFC, 0x87, 0x52, 0x02, 0x8A, 0x15, 0x2B, 0xE7, 0x40, 0x01, 0x53, 0x8A, 0x1D, 0x2A,
    0xE7, 0x40, 0x01, 0x55, 0x56, 0x66, 0x8B, 0x35, 0x28, 0xE7, 0x40, 0x01, 0x57, 0x8D, 0x49, 0x00,
    0x85, 0xC0, 0x0F, 0x84, 0xCF, 0x00, 0x00, 0x00, 0x8B, 0x78, 0x04, 0x0F, 0xB7, 0x48, 0x1C, 0x8B,
    0xEF, 0x2B, 0x2D, 0xFC, 0x87, 0x52, 0x02, 0xF7, 0xDD, 0x1B, 0xED, 0x23, 0xEF, 0x66, 0x3B, 0xF1,
    0x75, 0x17, 0x8A, 0x48, 0x1E, 0x3A, 0xD9, 0x75, 0x10, 0x8A, 0x48, 0x1F, 0x3A, 0xD1, 0x75, 0x09,
    0x8B, 0x0D, 0x34, 0xE7, 0x40, 0x01, 0x3B, 0x48, 0x28, 0x0F, 0x97, 0xC1, 0x84, 0xC9, 0x0F, 0x84,
    0x8C, 0x00, 0x00, 0x00, 0x39, 0x05, 0xFC, 0x87, 0x52, 0x02, 0x75, 0x19, 0x3B, 0xF8, 0x75, 0x0F,
    0x33, 0xC9, 0x89, 0x48, 0x04, 0x89, 0x08, 0x89, 0x0D, 0xFC, 0x87, 0x52, 0x02, 0xEB, 0x27, 0x89,
    0x3D, 0xFC, 0x87, 0x52, 0x02, 0x8B, 0x08, 0x85, 0xC9, 0x74, 0x03, 0x89, 0x79, 0x04, 0x8B, 0x48,
    0x04, 0x85, 0xC9, 0x74, 0x04, 0x8B, 0x10, 0x89, 0x11, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00,
    0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0xF8, 0x87, 0x52, 0x02, 0x85, 0xC9, 0x75, 0x0C,
    0x89, 0x40, 0x04, 0x89, 0x00, 0xA3, 0xF8, 0x87, 0x52, 0x02, 0xEB, 0x0E, 0x8B, 0x11, 0x89, 0x10,
    0x89, 0x48, 0x04, 0x8B, 0x11, 0x89, 0x42, 0x04, 0x89, 0x01, 0x0F, 0xB6, 0x48, 0x10, 0x8B, 0x50,
    0x14, 0x8D, 0x0C, 0x49, 0x8D, 0x7C, 0xCA, 0x08, 0xE8, 0x93, 0xB1, 0xCD, 0xFF, 0x8A, 0x15, 0x2B,
    0xE7, 0x40, 0x01, 0x8A, 0x1D, 0x2A, 0xE7, 0x40, 0x01, 0x66, 0x8B, 0x35, 0x28, 0xE7, 0x40, 0x01,
    0x8B, 0xC5, 0xE9, 0x29, 0xFF, 0xFF, 0xFF, 0x83, 0x3D, 0x90, 0x6C, 0x52, 0x02, 0x00, 0x0F, 0x8E,
    0x55, 0x01, 0x00, 0x00, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x23, 0x80, 0x3D, 0x56,
    0x8B, 0x52, 0x02, 0x00, 0x74, 0x0D, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x0F, 0x84, 0x36,
    0x01, 0x00, 0x00, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x29, 0x01, 0x00, 0x00,
    0xE8, 0xFB, 0x14, 0xD0, 0xFF, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x35, 0xF8, 0x87, 0x52, 0x02, 0xD9,
    0x44, 0x24, 0x10, 0x8B, 0xFE, 0x85, 0xFF, 0x0F, 0x84, 0x0A, 0x01, 0x00, 0x00, 0x8B, 0x57, 0x04,
    0x8B, 0xDA, 0x2B, 0xDE, 0xF7, 0xDB, 0x1B, 0xDB, 0x32, 0xC9, 0x23, 0xDA, 0x38, 0x4F, 0x11, 0x0F,
    0x86, 0xEB, 0x00, 0x00, 0x00, 0x8B, 0x6F, 0x14, 0x0F, 0xB6, 0xC1, 0x8D, 0x04, 0x40, 0xF6, 0x44,
    0xC5, 0x14, 0x01, 0x8D, 0x44, 0xC5, 0x00, 0x75, 0x0C, 0xD9, 0x40, 0x10, 0xD8, 0xD9, 0xDF, 0xE0,
    0xF6, 0xC4, 0x41, 0x7B, 0x0C, 0x80, 0xC1, 0x01, 0x3A, 0x4F, 0x11, 0x72, 0xDB, 0x8B, 0xFB, 0xEB,
    0xB4, 0x3B, 0xF7, 0xDD, 0xD8, 0x75, 0x18, 0x3B, 0xD7, 0x75, 0x0E, 0x33, 0xC0, 0x89, 0x47, 0x04,
    0x89, 0x07, 0xA3, 0xF8, 0x87, 0x52, 0x02, 0xEB, 0x27, 0x89, 0x15, 0xF8, 0x87, 0x52, 0x02, 0x8B,
    0x07, 0x85, 0xC0, 0x74, 0x03, 0x89, 0x50, 0x04, 0x8B, 0x47, 0x04, 0x85, 0xC0, 0x74, 0x04, 0x8B,
    0x17, 0x89, 0x10, 0xC7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x07, 0x00, 0x00, 0x00, 0x00,
    0xA1, 0xFC, 0x87, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x0D, 0x89, 0x7F, 0x04, 0x89, 0x3F, 0x89, 0x3D,
    0xFC, 0x87, 0x52, 0x02, 0xEB, 0x0E, 0x8B, 0x10, 0x89, 0x17, 0x89, 0x47, 0x04, 0x8B, 0x10, 0x89,
    0x7A, 0x04, 0x89, 0x38, 0x0F, 0xB6, 0xC1, 0x8D, 0x2C, 0x40, 0x8B, 0x47, 0x14, 0x03, 0xED, 0x03,
    0xED, 0x88, 0x4F, 0x10, 0x03, 0xED, 0x6A, 0x01, 0x6A, 0x00, 0x8D, 0x4C, 0x28, 0x08, 0xE8, 0xCD,
    0xAC, 0xCD, 0xFF, 0x8B, 0x4F, 0x14, 0x8B, 0x54, 0x29, 0x0C, 0x69, 0xD2, 0x80, 0xEE, 0x36, 0x00,
    0x83, 0xC4, 0x08, 0x52, 0x8D, 0x77, 0x18, 0xE8, 0x34, 0xAD, 0xED, 0xFF, 0x8B, 0x47, 0x14, 0x80,
    0x4C, 0x28, 0x14, 0x01, 0x8B, 0x4F, 0x14, 0x8D, 0x44, 0x28, 0x14, 0x03, 0xCD, 0x51, 0xE8, 0x3D,
    0xF9, 0xFF, 0xFF, 0xD9, 0x44, 0x24, 0x14, 0x8B, 0x35, 0xF8, 0x87, 0x52, 0x02, 0x83, 0xC4, 0x04,
    0x8B, 0xFB, 0xE9, 0xEE, 0xFE, 0xFF, 0xFF, 0xDD, 0xD8, 0x5F, 0x5E, 0x5D, 0x5B, 0x59, 0xC3,
];
#[allow(dead_code)]
const SCHED_EVENTS_BODY_CALLS: [usize; 5] = [0xd8, 0x130, 0x20e, 0x227, 0x23e];

// --- humans_post_update body (pin) ---
const HUMANS_POST_BODY: [u8; 3034] = [
    0x81, 0xEC, 0x7C, 0x01, 0x00, 0x00, 0x53, 0x55, 0x8B, 0x2D, 0x08, 0xE9, 0x15, 0x02, 0x56, 0x57,
    0x8B, 0xFD, 0x85, 0xFF, 0xBE, 0x00, 0x00, 0x10, 0x00, 0x0F, 0x84, 0xDA, 0x00, 0x00, 0x00, 0x90,
    0x85, 0xB7, 0xC8, 0x00, 0x00, 0x00, 0x0F, 0x84, 0xB5, 0x00, 0x00, 0x00, 0x80, 0xBF, 0x95, 0x05,
    0x00, 0x00, 0x00, 0x74, 0x0E, 0x80, 0xBF, 0x92, 0x05, 0x00, 0x00, 0x12, 0x74, 0x05, 0xE8, 0x2D,
    0xD8, 0xFF, 0xFF, 0x8B, 0x4F, 0x44, 0x85, 0xC9, 0x74, 0x19, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00,
    0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x48,
    0x44, 0x74, 0x02, 0x33, 0xC0, 0x3B, 0xC7, 0x75, 0x78, 0xD9, 0xEE, 0xD8, 0x9F, 0x24, 0x11, 0x00,
    0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x74, 0x69, 0x83, 0xBF, 0x8C, 0x0F, 0x00, 0x00, 0x00, 0x7D,
    0x60, 0x83, 0xBF, 0x70, 0x05, 0x00, 0x00, 0x00, 0x74, 0x47, 0x85, 0xC9, 0x74, 0x3C, 0xA1, 0x9C,
    0xCE, 0xEB, 0x00, 0x85, 0xC0, 0x74, 0x14, 0x3B, 0x48, 0x44, 0x75, 0x0F, 0xA1, 0xA0, 0xCE, 0xEB,
    0x00, 0x83, 0xF8, 0x07, 0x74, 0x05, 0x83, 0xF8, 0x08, 0x75, 0x1F, 0x8B, 0xC7, 0xE8, 0x1E, 0x20,
    0x01, 0x00, 0x84, 0xC0, 0x75, 0x14, 0x8B, 0x8F, 0x74, 0x0D, 0x00, 0x00, 0xE8, 0x1F, 0xB6, 0x13,
    0x00, 0x85, 0xC0, 0x0F, 0x95, 0xC0, 0x84, 0xC0, 0x75, 0x07, 0x8B, 0xC7, 0xE8, 0x8F, 0xC0, 0xFD,
    0xFF, 0x80, 0xBF, 0x95, 0x05, 0x00, 0x00, 0x00, 0x75, 0x07, 0x8B, 0xC7, 0xE8, 0x4F, 0xE5, 0xFF,
    0xFF, 0x8B, 0xBF, 0xB8, 0x00, 0x00, 0x00, 0x8B, 0x2D, 0x08, 0xE9, 0x15, 0x02, 0x3B, 0xFD, 0x74,
    0x08, 0x85, 0xFF, 0x0F, 0x85, 0x27, 0xFF, 0xFF, 0xFF, 0x85, 0xED, 0x0F, 0x84, 0xC5, 0x0A, 0x00,
    0x00, 0xEB, 0x05, 0xBE, 0x00, 0x00, 0x10, 0x00, 0x85, 0xB5, 0xC8, 0x00, 0x00, 0x00, 0x0F, 0x84,
    0x9C, 0x0A, 0x00, 0x00, 0x83, 0x7D, 0x4C, 0x01, 0x75, 0x45, 0x80, 0xBD, 0xCC, 0x03, 0x00, 0x00,
    0x01, 0x75, 0x3C, 0x83, 0xBD, 0x54, 0x15, 0x00, 0x00, 0x00, 0x7C, 0x33, 0x8B, 0x85, 0x54, 0x15,
    0x00, 0x00, 0x85, 0xC0, 0x7C, 0x29, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC1, 0x7E, 0x0B,
    0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x7C, 0x16, 0xEB, 0x0A, 0x2B, 0xC8, 0x81, 0xF9, 0x00,
    0xE9, 0xA4, 0x35, 0x7D, 0x0A, 0xC7, 0x85, 0x54, 0x15, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x55,
    0xE8, 0x5B, 0x83, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0xF7, 0x85, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x20, 0x75, 0x09, 0x80, 0xBD, 0x91, 0x05, 0x00, 0x00, 0x06, 0x75, 0x07, 0x8B, 0xF5, 0xE8,
    0x0C, 0x81, 0xFF, 0xFF, 0x8B, 0x4D, 0x44, 0x85, 0xC9, 0x74, 0x1A, 0x8B, 0xD1, 0x81, 0xE2, 0xFF,
    0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05,
    0x39, 0x48, 0x44, 0x74, 0x02, 0x33, 0xC0, 0x3B, 0xC5, 0x75, 0x18, 0xD9, 0xEE, 0xD8, 0x9D, 0x24,
    0x11, 0x00, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x74, 0x09, 0x83, 0xBD, 0x8C, 0x0F, 0x00, 0x00,
    0x00, 0x7C, 0x0E, 0x8B, 0xC5, 0xE8, 0x16, 0xDC, 0xFF, 0xFF, 0x8B, 0xFD, 0xE8, 0x9F, 0xDC, 0xFF,
    0xFF, 0x80, 0xBD, 0x92, 0x05, 0x00, 0x00, 0x12, 0x75, 0x07, 0x8B, 0xC5, 0xE8, 0x0F, 0xDD, 0xFF,
    0xFF, 0xE8, 0x9A, 0xF9, 0xD1, 0xFF, 0x84, 0xC0, 0x0F, 0x85, 0xC2, 0x00, 0x00, 0x00, 0xF6, 0x85,
    0xC8, 0x00, 0x00, 0x00, 0x02, 0x0F, 0x84, 0xB5, 0x00, 0x00, 0x00, 0xF6, 0x85, 0x14, 0x06, 0x00,
    0x00, 0x01, 0x0F, 0x84, 0xA1, 0x00, 0x00, 0x00, 0x8B, 0x85, 0xE0, 0x05, 0x00, 0x00, 0x80, 0xB8,
    0x6C, 0x02, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x8E, 0x00, 0x00, 0x00, 0x8B, 0x08, 0x85, 0xC9, 0x7C,
    0x30, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1,
    0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8, 0x05,
    0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x74, 0x07, 0x8B, 0x49, 0x04, 0x89, 0x4C, 0x24,
    0x30, 0x8D, 0x55, 0x20, 0x52, 0x8D, 0x45, 0x14, 0x50, 0x8B, 0x44, 0x24, 0x38, 0x50, 0x50, 0x8D,
    0xB5, 0xF4, 0x05, 0x00, 0x00, 0xE8, 0x46, 0x38, 0xD3, 0xFF, 0x8B, 0x8D, 0xE0, 0x05, 0x00, 0x00,
    0x8B, 0x09, 0x85, 0xC9, 0x7C, 0x33, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1,
    0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B,
    0xC8, 0x69, 0xC9, 0xE8, 0x05, 0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x74, 0x0A, 0x6A,
    0x00, 0x6A, 0x00, 0x51, 0xE8, 0x47, 0x08, 0x34, 0x00, 0x8B, 0xF5, 0xE8, 0xB0, 0xD4, 0xFF, 0xFF,
    0x55, 0xE8, 0x0A, 0xE6, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xF5, 0xE8, 0x00, 0xD2, 0xFF, 0xFF,
    0x8B, 0x4D, 0x44, 0x85, 0xC9, 0x74, 0x1A, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x48, 0x44, 0x74,
    0x02, 0x33, 0xC0, 0x3B, 0xC5, 0x75, 0x47, 0xD9, 0xEE, 0xD8, 0x9D, 0x24, 0x11, 0x00, 0x00, 0xDF,
    0xE0, 0xF6, 0xC4, 0x01, 0x74, 0x38, 0x83, 0xBD, 0x8C, 0x0F, 0x00, 0x00, 0x00, 0x7D, 0x2F, 0x80,
    0xBD, 0x91, 0x05, 0x00, 0x00, 0x07, 0x74, 0x26, 0xF7, 0x85, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x08,
    0x00, 0x00, 0x74, 0x32, 0x8B, 0x85, 0x70, 0x05, 0x00, 0x00, 0x8D, 0x95, 0xE8, 0x00, 0x00, 0x00,
    0x8D, 0x8D, 0xDC, 0x00, 0x00, 0x00, 0x50, 0xE8, 0xB4, 0xEB, 0xD8, 0xFF, 0xEB, 0x18, 0x8B, 0xB5,
    0x70, 0x05, 0x00, 0x00, 0x8D, 0x95, 0xDC, 0x00, 0x00, 0x00, 0x8D, 0x8D, 0xE8, 0x00, 0x00, 0x00,
    0x52, 0xE8, 0x8A, 0xEA, 0xD8, 0xFF, 0x55, 0xE8, 0x44, 0x58, 0x1C, 0x00, 0x8B, 0x85, 0xCC, 0x10,
    0x00, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x08, 0x8B, 0x98, 0xC4, 0x00, 0x00, 0x00, 0xEB,
    0x02, 0x33, 0xDB, 0x85, 0xC0, 0x74, 0x13, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74,
    0x09, 0x8B, 0x40, 0x08, 0x89, 0x44, 0x24, 0x14, 0xEB, 0x08, 0xC7, 0x44, 0x24, 0x14, 0x00, 0x00,
    0x00, 0x00, 0x85, 0xDB, 0x0F, 0x84, 0xA9, 0x04, 0x00, 0x00, 0x6A, 0x00, 0x8D, 0x4C, 0x24, 0x44,
    0x51, 0x8D, 0x54, 0x24, 0x3C, 0x52, 0x8B, 0xFD, 0xE8, 0x03, 0xBC, 0x01, 0x00, 0xD9, 0x44, 0x24,
    0x4C, 0xD9, 0x5C, 0x24, 0x70, 0x8B, 0x85, 0xCC, 0x10, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x50, 0x8D,
    0x78, 0x20, 0xD9, 0x5C, 0x24, 0x74, 0x8D, 0x74, 0x24, 0x70, 0xD9, 0x44, 0x24, 0x54, 0x83, 0xC4,
    0x0C, 0xD9, 0x5C, 0x24, 0x6C, 0xD9, 0x44, 0x24, 0x4C, 0xD9, 0x5C, 0x24, 0x70, 0xD9, 0x44, 0x24,
    0x50, 0xD9, 0x5C, 0x24, 0x74, 0xD9, 0x44, 0x24, 0x54, 0xD9, 0x5C, 0x24, 0x78, 0xD9, 0x44, 0x24,
    0x58, 0xD9, 0x5C, 0x24, 0x7C, 0xD9, 0x44, 0x24, 0x5C, 0xD9, 0x9C, 0x24, 0x80, 0x00, 0x00, 0x00,
    0xD9, 0x44, 0x24, 0x60, 0xD9, 0x9C, 0x24, 0x84, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x34, 0xD9,
    0x5C, 0x24, 0x1C, 0x8B, 0x4C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x38, 0x89, 0x48, 0x14, 0xD9, 0x5C,
    0x24, 0x20, 0x8B, 0x54, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x3C, 0x89, 0x50, 0x18, 0xD9, 0x5C, 0x24,
    0x24, 0x8B, 0x4C, 0x24, 0x24, 0x89, 0x48, 0x1C, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x8B,
    0x85, 0xCC, 0x10, 0x00, 0x00, 0x80, 0x88, 0xCC, 0x00, 0x00, 0x00, 0x01, 0x8B, 0x85, 0xE0, 0x05,
    0x00, 0x00, 0x8B, 0x08, 0x85, 0xC9, 0x8B, 0xB0, 0x68, 0x01, 0x00, 0x00, 0x0F, 0x8C, 0x81, 0x02,
    0x00, 0x00, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2,
    0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8,
    0x05, 0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x0F, 0x84, 0x54, 0x02, 0x00, 0x00, 0x51,
    0x8B, 0x0D, 0x10, 0x87, 0x5B, 0x01, 0x8B, 0xC6, 0xE8, 0xD3, 0xF3, 0x33, 0x00, 0x84, 0xC0, 0x0F,
    0x84, 0x3E, 0x02, 0x00, 0x00, 0x80, 0x4B, 0x0C, 0x02, 0x8B, 0x44, 0x24, 0x14, 0x0F, 0xB7, 0xB8,
    0x88, 0x00, 0x00, 0x00, 0x8B, 0x80, 0xCC, 0x00, 0x00, 0x00, 0x83, 0xF8, 0x01, 0xC6, 0x44, 0x24,
    0x13, 0x00, 0x74, 0x05, 0x83, 0xF8, 0x03, 0x75, 0x05, 0xC6, 0x44, 0x24, 0x13, 0x01, 0x3B, 0x2D,
    0xD4, 0x03, 0x17, 0x02, 0x0F, 0x85, 0x29, 0x01, 0x00, 0x00, 0xF6, 0x85, 0x63, 0x11, 0x00, 0x00,
    0x08, 0x74, 0x08, 0xD9, 0x05, 0xB4, 0x33, 0xEE, 0x00, 0xEB, 0x06, 0xD9, 0x05, 0xB0, 0x33, 0xEE,
    0x00, 0x8B, 0x75, 0x44, 0xD9, 0x5C, 0x24, 0x18, 0x85, 0xF6, 0x74, 0x57, 0xA1, 0x9C, 0xCE, 0xEB,
    0x00, 0x85, 0xC0, 0x74, 0x14, 0x3B, 0x70, 0x44, 0x75, 0x0F, 0xA1, 0xA0, 0xCE, 0xEB, 0x00, 0x83,
    0xF8, 0x07, 0x74, 0x05, 0x83, 0xF8, 0x08, 0x75, 0x3A, 0xE8, 0x12, 0x03, 0x16, 0x00, 0x85, 0xC0,
    0x74, 0x06, 0x83, 0x78, 0x04, 0x01, 0x74, 0x2B, 0xE8, 0x03, 0x03, 0x16, 0x00, 0x85, 0xC0, 0x74,
    0x06, 0x83, 0x78, 0x04, 0x02, 0x74, 0x1C, 0x8B, 0x8D, 0x74, 0x0D, 0x00, 0x00, 0xE8, 0xAE, 0xB1,
    0x13, 0x00, 0x85, 0xC0, 0x0F, 0x95, 0xC0, 0x84, 0xC0, 0x74, 0x08, 0xD9, 0x05, 0xD8, 0x33, 0xEE,
    0x00, 0xEB, 0x02, 0xD9, 0xE8, 0x6A, 0x5C, 0xD9, 0x5C, 0x24, 0x18, 0x33, 0xF6, 0x8D, 0x8C, 0x24,
    0xF0, 0x00, 0x00, 0x00, 0x56, 0x51, 0xE8, 0x55, 0x94, 0x2F, 0x00, 0xD9, 0xE8, 0x66, 0x83, 0x8C,
    0x24, 0x50, 0x01, 0x00, 0x00, 0x30, 0xD9, 0x9C, 0x24, 0x08, 0x01, 0x00, 0x00, 0xD9, 0x44, 0x24,
    0x20, 0x83, 0xC4, 0x0C, 0x80, 0x7C, 0x24, 0x13, 0x00, 0xD8, 0x4C, 0x24, 0x18, 0x89, 0xB4, 0x24,
    0x14, 0x01, 0x00, 0x00, 0x66, 0xC7, 0x84, 0x24, 0x1C, 0x01, 0x00, 0x00, 0xFF, 0xFF, 0x66, 0xC7,
    0x84, 0x24, 0x1E, 0x01, 0x00, 0x00, 0x01, 0x00, 0xD9, 0x9C, 0x24, 0xF8, 0x00, 0x00, 0x00, 0x89,
    0xB4, 0x24, 0x40, 0x01, 0x00, 0x00, 0x66, 0x89, 0xBC, 0x24, 0xEC, 0x00, 0x00, 0x00, 0x89, 0xB4,
    0x24, 0x04, 0x01, 0x00, 0x00, 0x89, 0xAC, 0x24, 0x38, 0x01, 0x00, 0x00, 0x8D, 0x84, 0x24, 0xEC,
    0x00, 0x00, 0x00, 0x75, 0x22, 0x8D, 0x8C, 0x24, 0x74, 0x01, 0x00, 0x00, 0xE8, 0xAF, 0xE3, 0xAB,
    0xFF, 0x8B, 0x10, 0x89, 0x53, 0x60, 0x8B, 0x48, 0x04, 0x89, 0x4B, 0x64, 0x8B, 0x50, 0x08, 0x89,
    0x53, 0x68, 0xE9, 0xEC, 0x00, 0x00, 0x00, 0x8D, 0x8C, 0x24, 0x50, 0x01, 0x00, 0x00, 0xE9, 0xCA,
    0x00, 0x00, 0x00, 0x8D, 0xB4, 0x24, 0x5C, 0x01, 0x00, 0x00, 0x8B, 0xCD, 0xE8, 0x7F, 0x2F, 0xFC,
    0xFF, 0x6A, 0x5C, 0x8D, 0x8C, 0x24, 0x90, 0x00, 0x00, 0x00, 0x6A, 0x00, 0x51, 0x8B, 0xF0, 0xE8,
    0x9C, 0x93, 0x2F, 0x00, 0xD9, 0xE8, 0x8B, 0x16, 0xD9, 0x94, 0x24, 0xA4, 0x00, 0x00, 0x00, 0x33,
    0xC0, 0xD9, 0x9C, 0x24, 0xA8, 0x00, 0x00, 0x00, 0x89, 0x84, 0x24, 0xC0, 0x00, 0x00, 0x00, 0xB9,
    0x01, 0x00, 0x00, 0x00, 0x66, 0x09, 0x8C, 0x24, 0xF0, 0x00, 0x00, 0x00, 0x89, 0x84, 0x24, 0xEC,
    0x00, 0x00, 0x00, 0x89, 0x84, 0x24, 0xB0, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x04, 0x89, 0x94, 0x24,
    0xB4, 0x00, 0x00, 0x00, 0x8B, 0x56, 0x08, 0x83, 0xC4, 0x0C, 0x80, 0x7C, 0x24, 0x13, 0x00, 0x89,
    0x84, 0x24, 0xAC, 0x00, 0x00, 0x00, 0x66, 0xC7, 0x84, 0x24, 0xBC, 0x00, 0x00, 0x00, 0xFF, 0xFF,
    0x66, 0x89, 0x8C, 0x24, 0xBE, 0x00, 0x00, 0x00, 0x66, 0x89, 0xBC, 0x24, 0x8C, 0x00, 0x00, 0x00,
    0x89, 0xAC, 0x24, 0xD8, 0x00, 0x00, 0x00, 0x89, 0x94, 0x24, 0xB0, 0x00, 0x00, 0x00, 0x8D, 0x84,
    0x24, 0x8C, 0x00, 0x00, 0x00, 0x75, 0x1F, 0x8D, 0x8C, 0x24, 0x68, 0x01, 0x00, 0x00, 0xE8, 0xDD,
    0xE2, 0xAB, 0xFF, 0x8B, 0x08, 0x89, 0x4B, 0x60, 0x8B, 0x50, 0x04, 0x89, 0x53, 0x64, 0x8B, 0x40,
    0x08, 0x89, 0x43, 0x68, 0xEB, 0x1D, 0x8D, 0x8C, 0x24, 0x80, 0x01, 0x00, 0x00, 0xE8, 0xBE, 0xE2,
    0xAB, 0xFF, 0x8B, 0x08, 0x89, 0x4B, 0x54, 0x8B, 0x50, 0x04, 0x89, 0x53, 0x58, 0x8B, 0x40, 0x08,
    0x89, 0x43, 0x5C, 0x8B, 0x85, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x08, 0x85, 0xC9, 0x8B, 0xB0, 0x68,
    0x01, 0x00, 0x00, 0x0F, 0x8C, 0xE1, 0x00, 0x00, 0x00, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9,
    0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00,
    0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8, 0x05, 0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03,
    0x0F, 0x84, 0xB4, 0x00, 0x00, 0x00, 0x51, 0x8B, 0x0D, 0x0C, 0x87, 0x5B, 0x01, 0x8B, 0xC6, 0xE8,
    0x3C, 0xF1, 0x33, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0x9E, 0x00, 0x00, 0x00, 0xBF, 0x01, 0x00, 0x00,
    0x00, 0x39, 0x7D, 0x4C, 0x0F, 0x85, 0x90, 0x00, 0x00, 0x00, 0x80, 0xBD, 0xCC, 0x03, 0x00, 0x00,
    0x01, 0x0F, 0x85, 0x83, 0x00, 0x00, 0x00, 0x80, 0x3D, 0x3A, 0x97, 0x52, 0x02, 0x00, 0x0F, 0x85,
    0x76, 0x00, 0x00, 0x00, 0xF6, 0x05, 0x40, 0x6A, 0x7A, 0x02, 0x40, 0x75, 0x6D, 0x8A, 0x0D, 0x48,
    0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01, 0x75, 0x12, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2,
    0x75, 0x08, 0x38, 0x15, 0xC5, 0xD2, 0xA4, 0x02, 0x75, 0x50, 0xF6, 0x43, 0x0C, 0x10, 0x75, 0x4A,
    0x39, 0x7D, 0x4C, 0x75, 0x24, 0x80, 0xBD, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x1B, 0x8B, 0x43,
    0x08, 0x83, 0x78, 0x34, 0x16, 0x74, 0x12, 0x8B, 0x8D, 0xDC, 0x11, 0x00, 0x00, 0x6A, 0x00, 0x51,
    0x8B, 0xF3, 0xE8, 0x69, 0x16, 0x1C, 0x00, 0xEB, 0x06, 0x8B, 0x43, 0x14, 0x03, 0x43, 0x10, 0x85,
    0xC0, 0x7F, 0x17, 0xD9, 0x05, 0xB4, 0x0B, 0xDD, 0x00, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x57,
    0x8B, 0xCD, 0xE8, 0xE9, 0x8D, 0xFC, 0xFF, 0x83, 0xC4, 0x0C, 0x8A, 0x43, 0x0C, 0xA8, 0x02, 0x74,
    0x38, 0xA8, 0x04, 0x74, 0x48, 0x80, 0x3D, 0x0B, 0x97, 0x52, 0x02, 0x00, 0x75, 0x19, 0x6A, 0x01,
    0x53, 0x55, 0xE8, 0xE9, 0x14, 0x02, 0x00, 0x83, 0xC4, 0x0C, 0x80, 0x63, 0x0C, 0xFB, 0xC6, 0x05,
    0x0B, 0x97, 0x52, 0x02, 0x01, 0xEB, 0x3C, 0x6A, 0x00, 0x53, 0x55, 0xE8, 0xD0, 0x14, 0x02, 0x00,
    0xC6, 0x05, 0x0B, 0x97, 0x52, 0x02, 0x00, 0xEB, 0x23, 0xA8, 0x04, 0x74, 0x10, 0x6A, 0x01, 0x53,
    0x55, 0xE8, 0xBA, 0x14, 0x02, 0x00, 0x83, 0xC4, 0x0C, 0x80, 0x63, 0x0C, 0xFB, 0xF6, 0x43, 0x0C,
    0x02, 0x74, 0x10, 0x6A, 0x00, 0x53, 0x55, 0xE8, 0xA4, 0x14, 0x02, 0x00, 0x83, 0xC4, 0x0C, 0x80,
    0x63, 0x0C, 0xFD, 0x8B, 0x85, 0xCC, 0x10, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x33, 0x8B, 0xB0, 0xC4,
    0x00, 0x00, 0x00, 0x85, 0xF6, 0x74, 0x29, 0x80, 0xBD, 0xC8, 0x10, 0x00, 0x00, 0x00, 0x75, 0x20,
    0x8B, 0x56, 0x08, 0x52, 0x56, 0x55, 0xE8, 0xF5, 0x58, 0x01, 0x00, 0x83, 0xC4, 0x0C, 0x55, 0x8B,
    0xC6, 0xE8, 0x0A, 0x60, 0x01, 0x00, 0x83, 0xC4, 0x04, 0x8B, 0xC5, 0xE8, 0xB0, 0x9F, 0x01, 0x00,
    0x66, 0x81, 0x7D, 0x48, 0xFF, 0xFF, 0x74, 0x53, 0xD9, 0x45, 0x18, 0xDC, 0x1D, 0x90, 0xC2, 0xE5,
    0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7B, 0x43, 0x8B, 0x85, 0xCC, 0x00, 0x00, 0x00, 0xA9, 0x00,
    0x00, 0x00, 0x08, 0x0F, 0x84, 0x03, 0x03, 0x00, 0x00, 0x25, 0xFF, 0xFF, 0xFF, 0xF7, 0x89, 0x85,
    0xCC, 0x00, 0x00, 0x00, 0xE8, 0xB7, 0xB0, 0xE9, 0xFF, 0x84, 0xC0, 0x0F, 0x85, 0xEB, 0x02, 0x00,
    0x00, 0xD9, 0xEE, 0x51, 0xD9, 0x1C, 0x24, 0x32, 0xD2, 0x68, 0xF4, 0x01, 0x00, 0x00, 0xE8, 0x9D,
    0x97, 0xB2, 0xFF, 0x83, 0xC4, 0x08, 0xE9, 0xD1, 0x02, 0x00, 0x00, 0x8B, 0x4D, 0x44, 0x85, 0xC9,
    0x0F, 0x84, 0x5E, 0x01, 0x00, 0x00, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04,
    0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0x46, 0x01, 0x00, 0x00, 0x39, 0x48,
    0x44, 0x0F, 0x85, 0x3D, 0x01, 0x00, 0x00, 0xBF, 0x01, 0x00, 0x00, 0x00, 0x39, 0x78, 0x4C, 0x0F,
    0x85, 0x34, 0x01, 0x00, 0x00, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x0F, 0x85, 0x27, 0x01,
    0x00, 0x00, 0x85, 0xC9, 0x74, 0x51, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04,
    0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x3D, 0x39, 0x48, 0x44, 0x75, 0x38, 0x8B,
    0xB0, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x74, 0x2E, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0,
    0x74, 0x25, 0x8B, 0x90, 0x08, 0x04, 0x00, 0x00, 0x85, 0xD2, 0x74, 0x1B, 0x8A, 0x06, 0x3A, 0x82,
    0xB0, 0x01, 0x00, 0x00, 0x0F, 0x85, 0x42, 0x02, 0x00, 0x00, 0xF6, 0x86, 0x7C, 0x06, 0x00, 0x00,
    0x01, 0x0F, 0x85, 0x35, 0x02, 0x00, 0x00, 0xBE, 0x00, 0x00, 0x00, 0x08, 0x85, 0xB5, 0xCC, 0x00,
    0x00, 0x00, 0x0F, 0x85, 0xE7, 0x01, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x1A, 0x8B, 0xD1, 0x81, 0xE2,
    0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74,
    0x05, 0x39, 0x48, 0x44, 0x74, 0x02, 0x33, 0xC0, 0x3B, 0xC5, 0x0F, 0x85, 0xFC, 0x01, 0x00, 0x00,
    0xD9, 0xEE, 0xD8, 0x9D, 0x24, 0x11, 0x00, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x0F, 0x84, 0xE9,
    0x01, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0x84, 0x2D, 0x01, 0x00, 0x00, 0x8B, 0xC1, 0x25, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0x15,
    0x01, 0x00, 0x00, 0x39, 0x48, 0x44, 0x0F, 0x85, 0x0C, 0x01, 0x00, 0x00, 0x39, 0x78, 0x4C, 0x0F,
    0x85, 0x03, 0x01, 0x00, 0x00, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x0F, 0x85, 0xF6, 0x00,
    0x00, 0x00, 0x55, 0xE8, 0x88, 0x66, 0xFB, 0xFF, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x0F, 0x85, 0xE5,
    0x00, 0x00, 0x00, 0x50, 0x55, 0xE8, 0x26, 0xE3, 0xFB, 0xFF, 0x6A, 0x00, 0x55, 0xE8, 0xFE, 0xEA,
    0xFB, 0xFF, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24,
    0x55, 0x33, 0xD2, 0x33, 0xC9, 0xE8, 0x66, 0xCB, 0xFD, 0xFF, 0xDD, 0xD8, 0x83, 0xC4, 0x10, 0xE9,
    0x68, 0x01, 0x00, 0x00, 0xBF, 0x01, 0x00, 0x00, 0x00, 0x80, 0xBD, 0xCC, 0x03, 0x00, 0x00, 0x02,
    0x75, 0x0D, 0x83, 0xBD, 0x78, 0x11, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xC3, 0xFE, 0xFF, 0xFF, 0x8A,
    0x0D, 0x48, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x74, 0x38, 0x8B, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x85,
    0xC9, 0x74, 0x2E, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0x80, 0x08, 0x04,
    0x00, 0x00, 0x85, 0xC0, 0x74, 0x1B, 0x8A, 0x11, 0x3A, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x0F, 0x85,
    0x18, 0x01, 0x00, 0x00, 0xF6, 0x81, 0x7C, 0x06, 0x00, 0x00, 0x01, 0x0F, 0x85, 0x0B, 0x01, 0x00,
    0x00, 0xD9, 0x45, 0x1C, 0x83, 0xEC, 0x18, 0xDD, 0x5C, 0x24, 0x10, 0xD9, 0x45, 0x18, 0xDD, 0x5C,
    0x24, 0x08, 0xD9, 0x45, 0x14, 0xDD, 0x1C, 0x24, 0x68, 0xE8, 0xBB, 0xE3, 0x00, 0x68, 0xE0, 0x60,
    0x34, 0x02, 0xE8, 0x2F, 0xEC, 0x2E, 0x00, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x83, 0xC4, 0x20,
    0x6A, 0x00, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x55, 0x33, 0xD2, 0x33, 0xC9, 0xE8, 0xBE, 0xCA,
    0xFD, 0xFF, 0xDD, 0xD8, 0x8B, 0x45, 0x00, 0x8B, 0x50, 0x10, 0x83, 0xC4, 0x10, 0x6A, 0x00, 0x8B,
    0xCD, 0xFF, 0xD2, 0xE9, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0x4D, 0x44, 0x09, 0xB5, 0xCC, 0x00, 0x00,
    0x00, 0x85, 0xC9, 0x74, 0x5F, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04,
    0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x4A, 0x39, 0x48, 0x44, 0x75, 0x45, 0x39,
    0x78, 0x4C, 0x75, 0x40, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x37, 0xE8, 0x3E, 0xAE,
    0xE9, 0xFF, 0x84, 0xC0, 0x75, 0x27, 0x68, 0xFF, 0x00, 0x00, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0x6A,
    0x00, 0xE8, 0xFA, 0xCA, 0x36, 0x00, 0xD9, 0xE8, 0x83, 0xC4, 0x0C, 0xD9, 0x1C, 0x24, 0x6A, 0x00,
    0x68, 0xF4, 0x01, 0x00, 0x00, 0xE8, 0x36, 0x93, 0xB2, 0xFF, 0x83, 0xC4, 0x0C, 0xE8, 0xCE, 0xD6,
    0xB5, 0xFF, 0xEB, 0x48, 0x66, 0xC7, 0x85, 0x5A, 0x06, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x3D, 0x85,
    0xC9, 0x74, 0x32, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x1E, 0x39, 0x48, 0x44, 0x75, 0x19, 0x39, 0x78, 0x4C, 0x75,
    0x14, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x0B, 0x55, 0xE8, 0xA0, 0xC4, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0xEB, 0x07, 0x8B, 0xF5, 0xE8, 0x74, 0xC0, 0xFF, 0xFF, 0xF7, 0x85, 0xC8, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x74, 0x08, 0xD9, 0xEE, 0xD9, 0x9D, 0xB0, 0x05, 0x00, 0x00,
    0x8B, 0xAD, 0xB8, 0x00, 0x00, 0x00, 0x3B, 0x2D, 0x08, 0xE9, 0x15, 0x02, 0x74, 0x08, 0x85, 0xED,
    0x0F, 0x85, 0x3D, 0xF5, 0xFF, 0xFF, 0xE8, 0xB5, 0x5B, 0xD3, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x81,
    0xC4, 0x7C, 0x01, 0x00, 0x00, 0xE9, 0x26, 0xFE, 0xD8, 0xFF,
];
#[allow(dead_code)]
const HUMANS_POST_BODY_CALLS: [usize; 56] = [0x3e, 0xad, 0xbc, 0xcc, 0xdc, 0x160, 0x17f, 0x1c5, 0x1cc, 0x1dc, 0x1e1, 0x265, 0x2a4, 0x2ab, 0x2b1, 0x2bb, 0x327, 0x341, 0x347, 0x398, 0x488, 0x509, 0x518, 0x52d, 0x556, 0x5cc, 0x5fc, 0x60f, 0x69e, 0x6bd, 0x71f, 0x7a2, 0x7c2, 0x7e2, 0x7fb, 0x811, 0x827, 0x856, 0x861, 0x86b, 0x8a4, 0x8be, 0x9f3, 0xa05, 0xa0d, 0xa25, 0xab2, 0xacd, 0xb1d, 0xb31, 0xb45, 0xb4d, 0xb8b, 0xb97, 0xbc6, 0xbd5];
#[allow(dead_code)]
const HUMANS_POST_BODY_DISPS: [usize; 44] = [0xa, 0x56, 0x8f, 0x9d, 0xe9, 0x138, 0x198, 0x244, 0x299, 0x2d4, 0x475, 0x482, 0x4c0, 0x4d5, 0x4dd, 0x4ed, 0x4fb, 0x53d, 0x70c, 0x719, 0x749, 0x756, 0x75f, 0x76a, 0x774, 0x7b5, 0x7d7, 0x7f0, 0x802, 0x87d, 0x8e2, 0x922, 0x93a, 0x989, 0x9c7, 0xa14, 0xa51, 0xa64, 0xaa9, 0xaae, 0xab9, 0xb02, 0xb6f, 0xbb8];

// =================================================================================================
// 0x9A7210  humans_post_update  (3034 bytes, no arguments, ends in a tail jump to 0x737C10)
// =================================================================================================
//
// Runs every frame for every human of the ring at [0x215E908] (links +0xB8; only humans with
// +0xC8 bit 0x100000 take part). Pass 1 checks the dead / knocked-down state (0x9A4A80, 0x983370,
// 0x9A5840); pass 2 does the per-human work: timers, pose for render (0x9A5AD0, 0x9A46D0,
// 0x7360F0 / 0x735FE0), the held weapon (transform 0x9C31B0 into the weapon object at
// [human+0x10CC], lock-on beep 0x9707C0, flags 0x9C8EE0, 0x9BD360 / 0x9BDA80 / 0x9C1A30) and the
// fell-through-the-world recovery. Then 0x6DD990 and a tail call of 0x737C10.
//
// Registers: the function takes none and keeps none; every callee gets exactly the registers the
// original sets for it (EDI / ESI / EAX holding the human where the callee reads them). The
// original's 0x17C-byte frame is kept as a local array (offsets below are from the original ESP
// after the prologue), because callees write into it.
//
// Quirks kept: the slot B+0x30 (a rig record field) survives from one human to the next and is
// passed to 0x6DACC0 stale when the record is missing (here it starts at 0 where the original has
// stack garbage); the compiler's divide-by-190 is done as `% 190` for ids >= 0.
//
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

const HP_N: usize = 40;
static HP_HITS: [AtomicU32; HP_N] = [const { AtomicU32::new(0) }; HP_N];
const HP_NAMES: [&str; HP_N] = [
    "calls", "p1_human", "p1_9a4a80", "p1_self", "p1_cond", "p1_983370", "p1_9a5840", "p2_human",
    "timer_clear", "c99f4a0", "c9a4ff0", "c9a5100", "rig_branch", "c6dacc0", "cce7d00", "c9a4970",
    "c7360f0", "c735fe0", "w_nonnull", "c9c31b0", "lock_path", "player_rec", "other_rec", "flag_b",
    "beep_9707c0", "c9c8ee0", "c9bd360", "fell_y", "chain_7adb", "b22", "c965f40", "path_7cf8",
    "cd14840", "w65a_clear", "c9a4240", "c9a3e20", "sprintf", "vcall", "c4d1270", "c505430",
];

fn hp(i: usize) {
    HP_HITS[i].fetch_add(1, Ordering::Relaxed);
}

/// `fld dword` + `fstp dword` on raw bits: a signalling NaN comes out quiet.
#[inline(always)]
fn q(u: u32) -> u32 {
    if u & 0x7F80_0000 == 0x7F80_0000 && u & 0x007F_FFFF != 0 { u | 0x0040_0000 } else { u }
}

/// A double from the exe (or the test's shifted copy).
#[inline(always)]
unsafe fn rq(p: usize) -> f64 {
    #[cfg(test)]
    let p = tr(p);
    *(p as *const f64)
}

/// The human of handle `h` (slot of the 4096-entry table 0x2149C64, validated), or 0.
#[inline(always)]
unsafe fn hv(h: u32) -> usize {
    if h == 0 {
        return 0;
    }
    let s = rd(HANDLE_TABLE + (((h & 0xFFF) << 4) as usize)) as usize;
    if s != 0 && rd(s + 0x44) == h { s } else { 0 }
}

/// `fldz; fcomp x; fnstsw; test ah, 1`: set when 0 < x or x is a NaN.
#[inline(always)]
fn c0_pos(x: f32) -> bool {
    !(x as f64 <= 0.0)
}

/// The 0x5E8-byte record of rig id `id` (>= 0) in the table at 0x340D240.
#[inline(always)]
fn rig_rec(id: i32) -> usize {
    (id % 190) as usize * 0x5E8 + 0x0340_D240
}

#[inline(always)]
unsafe fn fl(b: usize, off: usize) -> u32 {
    *((b + off) as *const u32)
}
#[inline(always)]
unsafe fn fs(b: usize, off: usize, v: u32) {
    *((b + off) as *mut u32) = v
}
#[inline(always)]
unsafe fn fsw(b: usize, off: usize, v: u16) {
    *((b + off) as *mut u16) = v
}

kxs0!(humans_post_thunk, humans_post_update);

extern "C" fn humans_post_update() -> u32 {
    unsafe {
        hp(0);
        const RING: usize = 0x0215_E908;
        const NOW: usize = 0x00E8_43AC;
        // The original's locals, addressed by their offset from ESP after the prologue.
        let mut frame = [0u32; 99];
        let b = frame.as_mut_ptr() as usize;

        // ---- pass 1 ----
        let mut ebp = rd(RING) as usize;
        let mut edi = ebp;
        if edi != 0 {
            loop {
                if rd(edi + 0xC8) & 0x10_0000 != 0 {
                    hp(1);
                    if rb(edi + 0x595) != 0 && rb(edi + 0x592) != 0x12 {
                        hp(2);
                        call_regs(0x009A_4A80, 0, 0, 0, 0, edi as u32, &[]);
                    }
                    let h = rd(edi + 0x44);
                    if hv(h) == edi && c0_pos(rf(edi + 0x1124)) && (rd(edi + 0xF8C) as i32) < 0 {
                        hp(3);
                        // 72da: stop the dead human's actions; skipped when it is in a vehicle
                        let do_983370 = if rd(edi + 0x570) == 0 {
                            false
                        } else if h == 0 {
                            true
                        } else {
                            let a = rd(0x00EB_CE9C) as usize;
                            let check_vehicle = if a == 0 || h != rd(a + 0x44) {
                                true
                            } else {
                                let m = rd(0x00EB_CEA0);
                                m == 7 || m == 8
                            };
                            if check_vehicle {
                                let al = call_regs(0x009B_92E0, edi as u32, 0, 0, 0, 0, &[]) as u8;
                                if al != 0 {
                                    true
                                } else {
                                    let r = call_regs(0x00AE_28F0, 0, rd(edi + 0xD74), 0, 0, 0, &[]);
                                    r == 0
                                }
                            } else {
                                true
                            }
                        };
                        if do_983370 {
                            hp(5);
                            call_regs(0x0098_3370, edi as u32, 0, 0, 0, 0, &[]);
                        }
                        if rb(edi + 0x595) == 0 {
                            hp(6);
                            call_regs(0x009A_5840, edi as u32, 0, 0, 0, 0, &[]);
                        }
                    }
                }
                edi = rd(edi + 0xB8) as usize;
                ebp = rd(RING) as usize;
                if edi == ebp || edi == 0 {
                    break;
                }
            }
        }

        // ---- pass 2 ----
        let mut l30: u32 = 0; // B+0x30, kept between humans
        if ebp != 0 {
            loop {
                if rd(ebp + 0xC8) & 0x10_0000 != 0 {
                    hp(7);
                    let hh = ebp as u32;
                    // timer at +0x1554 reached: clear it
                    if rd(ebp + 0x4C) == 1 && rb(ebp + 0x3CC) == 1 {
                        let t = rd(ebp + 0x1554) as i32;
                        if t >= 0 && time_due(t, rd(NOW) as i32) {
                            hp(8);
                            wd(ebp + 0x1554, 0xFFFF_FFFF);
                        }
                    }
                    call_regs(0x0099_F6D0, 0, 0, 0, 0, 0, &[hh]);
                    if rd(ebp + 0xC0) & 0x2000_0000 != 0 || rb(ebp + 0x591) == 6 {
                        hp(9);
                        call_regs(0x0099_F4A0, 0, 0, 0, hh, 0, &[]);
                    }
                    let alive = hv(rd(ebp + 0x44)) == ebp && c0_pos(rf(ebp + 0x1124)) && (rd(ebp + 0xF8C) as i32) < 0;
                    if !alive {
                        hp(10);
                        call_regs(0x009A_4FF0, hh, 0, 0, 0, 0, &[]);
                        call_regs(0x009A_5080, 0, 0, 0, 0, hh, &[]);
                    }
                    if rb(ebp + 0x592) == 0x12 {
                        hp(11);
                        call_regs(0x009A_5100, hh, 0, 0, 0, 0, &[]);
                    }
                    let al = call_regs(0x006C_6D90, 0, 0, 0, 0, 0, &[]) as u8;
                    if al == 0 && rb(ebp + 0xC8) & 2 != 0 {
                        hp(12);
                        if rb(ebp + 0x614) & 1 != 0 && rb(rd(ebp + 0x5E0) as usize + 0x26C) != 0 {
                            let id = rd(rd(ebp + 0x5E0) as usize) as i32;
                            if id >= 0 {
                                l30 = rd(rig_rec(id) + 4);
                            }
                            hp(13);
                            call_regs(0x006D_ACC0, 0, 0, 0, (ebp + 0x5F4) as u32, 0, &[l30, l30, (ebp + 0x14) as u32, (ebp + 0x20) as u32]);
                            let id = rd(rd(ebp + 0x5E0) as usize) as i32;
                            if id >= 0 {
                                hp(14);
                                call_regs(0x00CE_7D00, 0, 0, 0, 0, 0, &[ga(rig_rec(id)), 0, 0]);
                            }
                        }
                        hp(15);
                        call_regs(0x009A_4970, 0, 0, 0, hh, 0, &[]);
                    }
                    call_regs(0x009A_5AD0, 0, 0, 0, 0, 0, &[hh]);
                    call_regs(0x009A_46D0, 0, 0, 0, hh, 0, &[]);
                    let pose_dead = hv(rd(ebp + 0x44)) != ebp
                        || !c0_pos(rf(ebp + 0x1124))
                        || (rd(ebp + 0xF8C) as i32) >= 0
                        || rb(ebp + 0x591) == 7;
                    if !pose_dead {
                        if rd(ebp + 0xC0) & 0x800 != 0 {
                            hp(16);
                            call_regs(0x0073_60F0, 0, (ebp + 0xDC) as u32, (ebp + 0xE8) as u32, 0, 0, &[rd(ebp + 0x570)]);
                        }
                    } else {
                        hp(17);
                        call_regs(0x0073_5FE0, 0, (ebp + 0xE8) as u32, 0, rd(ebp + 0x570), 0, &[(ebp + 0xDC) as u32]);
                    }
                    call_regs(0x00B6_CDA0, 0, 0, 0, 0, 0, &[hh]);

                    // ---- the held weapon ----
                    let obj = rd(ebp + 0x10CC) as usize;
                    let ebx = if obj != 0 { rd(obj + 0xC4) as usize } else { 0 }; // weapon state W
                    let l14 = if obj != 0 && ebx != 0 { rd(ebx + 8) as usize } else { 0 };
                    if ebx != 0 {
                        hp(18);
                        hp(19);
                        call_regs(0x009C_31B0, 0, 0, 0, 0, hh, &[(b + 0x34) as u32, (b + 0x40) as u32, 0]);
                        // the 3x3 matrix and the position go into the weapon object
                        for i in 0..9 {
                            fs(b, 0x64 + 4 * i, q(fl(b, 0x40 + 4 * i)));
                        }
                        let o = rd(ebp + 0x10CC) as usize;
                        let x = q(fl(b, 0x34));
                        fs(b, 0x1C, x);
                        wd(o + 0x14, x);
                        let y = q(fl(b, 0x38));
                        fs(b, 0x20, y);
                        wd(o + 0x18, y);
                        let z = q(fl(b, 0x3C));
                        fs(b, 0x24, z);
                        wd(o + 0x1C, z);
                        for i in 0..9 {
                            wd(o + 0x20 + 4 * i, fl(b, 0x64 + 4 * i));
                        }
                        let o = rd(ebp + 0x10CC) as usize;
                        wb(o + 0xCC, rb(o + 0xCC) | 1);

                        let rig = rd(ebp + 0x5E0) as usize;
                        let id = rd(rig) as i32;
                        let rig168 = rd(rig + 0x168);
                        let mut lock = false;
                        if id >= 0 {
                            let r = call_regs(0x00CE_6A70, rig168, rd(0x015B_8710), 0, 0, 0, &[ga(rig_rec(id))]) as u8;
                            lock = r != 0;
                        }
                        if lock {
                            hp(20);
                            wb(ebx + 0xC, rb(ebx + 0xC) | 2);
                            let di = *((l14 + 0x88) as *const u16);
                            let kind = rd(l14 + 0xCC);
                            let flag_b = kind == 1 || kind == 3;
                            if flag_b {
                                hp(23);
                            }
                            *((b + 0x13) as *mut u8) = flag_b as u8;
                            let out: usize;
                            let rec: usize;
                            if hh == rd(0x0217_03D4) {
                                hp(21);
                                // player: the reticle size, scaled by what the player is doing
                                let s1 = if rb(ebp + 0x1163) & 8 != 0 { q(rd(0x00EE_33B4)) } else { q(rd(0x00EE_33B0)) };
                                fs(b, 0x18, s1);
                                let esi = rd(ebp + 0x44);
                                let one = 0x3F80_0000u32;
                                let s2 = if esi != 0 {
                                    let a = rd(0x00EB_CE9C) as usize;
                                    let mut go = true;
                                    if !(a == 0 || esi != rd(a + 0x44)) {
                                        let m = rd(0x00EB_CEA0);
                                        go = m == 7 || m == 8;
                                    }
                                    if go {
                                        let r1 = call_regs(0x00B0_7A30, 0, 0, 0, esi, 0, &[]) as usize;
                                        if r1 != 0 && rd(r1 + 4) == 1 {
                                            one
                                        } else {
                                            let r2 = call_regs(0x00B0_7A30, 0, 0, 0, esi, 0, &[]) as usize;
                                            if r2 != 0 && rd(r2 + 4) == 2 {
                                                one
                                            } else {
                                                let r3 = call_regs(0x00AE_28F0, 0, rd(ebp + 0xD74), 0, 0, 0, &[]);
                                                if r3 == 0 { one } else { q(rd(0x00EE_33D8)) }
                                            }
                                        }
                                    } else {
                                        one
                                    }
                                } else {
                                    one
                                };
                                fs(b, 0x14, s2);
                                rec = b + 0xEC;
                                std::ptr::write_bytes(rec as *mut u8, 0, 0x5C);
                                fs(b, 0xFC, one);
                                *((b + 0x144) as *mut u16) |= 0x30;
                                let m = ((f32::from_bits(s2) as f64) * (f32::from_bits(s1) as f64)) as f32;
                                fs(b, 0x114, 0);
                                fsw(b, 0x11C, 0xFFFF);
                                fsw(b, 0x11E, 1);
                                fs(b, 0xF8, m.to_bits());
                                fs(b, 0x140, 0);
                                fsw(b, 0xEC, di);
                                fs(b, 0x104, 0);
                                fs(b, 0x138, hh);
                                out = if flag_b { b + 0x150 } else { b + 0x174 };
                            } else {
                                hp(22);
                                let v = call_regs(0x0096_A790, 0, hh, 0, (b + 0x15C) as u32, 0, &[]) as usize;
                                rec = b + 0x8C;
                                std::ptr::write_bytes(rec as *mut u8, 0, 0x5C);
                                let vx = rd(v);
                                let one = 0x3F80_0000u32;
                                fs(b, 0x98, one);
                                fs(b, 0x9C, one);
                                fs(b, 0xB4, 0);
                                *((b + 0xE4) as *mut u16) |= 1;
                                fs(b, 0xE0, 0);
                                fs(b, 0xA4, 0);
                                let vy = rd(v + 4);
                                fs(b, 0xA8, vx);
                                let vz = rd(v + 8);
                                fs(b, 0xAC, vy);
                                fsw(b, 0xBC, 0xFFFF);
                                fsw(b, 0xBE, 1);
                                fsw(b, 0x8C, di);
                                fs(b, 0xD8, hh);
                                fs(b, 0xB0, vz);
                                out = if flag_b { b + 0x180 } else { b + 0x168 };
                            }
                            let v = call_regs(0x0046_5B90, rec as u32, out as u32, 0, 0, 0, &[]) as usize;
                            let dst = if flag_b { 0x54 } else { 0x60 };
                            let (v0, v1, v2) = (rd(v), rd(v + 4), rd(v + 8));
                            wd(ebx + dst, v0);
                            wd(ebx + dst + 4, v1);
                            wd(ebx + dst + 8, v2);
                        }

                        // lock-on beep
                        let rig = rd(ebp + 0x5E0) as usize;
                        let id = rd(rig) as i32;
                        let rig168 = rd(rig + 0x168);
                        if id >= 0 {
                            let r = call_regs(0x00CE_6A70, rig168, rd(0x015B_870C), 0, 0, 0, &[ga(rig_rec(id))]) as u8;
                            if r != 0
                                && rd(ebp + 0x4C) == 1
                                && rb(ebp + 0x3CC) == 1
                                && rb(0x0252_973A) == 0
                                && rb(0x027A_6A40) & 0x40 == 0
                            {
                                let cl = rb(0x0252_8B48);
                                let skip = cl == 1 && rb(0x0252_8B49) == 0 && rb(0x02A4_D2C5) != 0;
                                if !skip && rb(ebx + 0xC) & 0x10 == 0 {
                                    let eax = if rd(ebp + 0x4C) == 1 && rb(ebp + 0x3CC) == 1 && rd(rd(ebx + 8) as usize + 0x34) != 0x16 {
                                        call_regs(0x00B6_9020, 0, 0, 0, ebx as u32, 0, &[rd(ebp + 0x11DC), 0])
                                    } else {
                                        rd(ebx + 0x14).wrapping_add(rd(ebx + 0x10))
                                    };
                                    if (eax as i32) <= 0 {
                                        hp(24);
                                        call_regs(0x0097_07C0, 0, hh, 0, 0, 0, &[1, q(rd(0x00DD_0BB4)), 0]);
                                    }
                                }
                            }
                        }

                        // weapon flag bits 2 / 4 of W+0xC
                        let al = rb(ebx + 0xC);
                        if al & 2 != 0 {
                            if al & 4 != 0 {
                                hp(25);
                                if rb(0x0252_970B) == 0 {
                                    call_regs(0x009C_8EE0, 0, 0, 0, 0, 0, &[hh, ebx as u32, 1]);
                                    wb(ebx + 0xC, rb(ebx + 0xC) & 0xFB);
                                    wb(0x0252_970B, 1);
                                } else {
                                    call_regs(0x009C_8EE0, 0, 0, 0, 0, 0, &[hh, ebx as u32, 0]);
                                    wb(0x0252_970B, 0);
                                    wb(ebx + 0xC, rb(ebx + 0xC) & 0xFD);
                                }
                            } else {
                                // bit 2 only: 7a2d
                                hp(25);
                                call_regs(0x009C_8EE0, 0, 0, 0, 0, 0, &[hh, ebx as u32, 0]);
                                wb(ebx + 0xC, rb(ebx + 0xC) & 0xFD);
                            }
                        } else {
                            if al & 4 != 0 {
                                hp(25);
                                call_regs(0x009C_8EE0, 0, 0, 0, 0, 0, &[hh, ebx as u32, 1]);
                                wb(ebx + 0xC, rb(ebx + 0xC) & 0xFB);
                            }
                            if rb(ebx + 0xC) & 2 != 0 {
                                hp(25);
                                call_regs(0x009C_8EE0, 0, 0, 0, 0, 0, &[hh, ebx as u32, 0]);
                                wb(ebx + 0xC, rb(ebx + 0xC) & 0xFD);
                            }
                        }
                    }

                    // 7a43: refresh the weapon slots
                    let o = rd(ebp + 0x10CC) as usize;
                    if o != 0 {
                        let esi = rd(o + 0xC4) as usize;
                        if esi != 0 && rb(ebp + 0x10C8) == 0 {
                            hp(26);
                            call_regs(0x009B_D360, 0, 0, 0, 0, 0, &[hh, esi as u32, rd(esi + 8)]);
                            call_regs(0x009B_DA80, esi as u32, 0, 0, 0, 0, &[hh]);
                            call_regs(0x009C_1A30, hh, 0, 0, esi as u32, 0, &[]);
                        }
                    }

                    // 7a80: height check
                    if *((ebp + 0x48) as *const u16) != 0xFFFF && !(rf(ebp + 0x18) as f64 <= rq(0x00E5_C290)) {
                        hp(27);
                        // above the limit: only the "fell" flag handling
                        let v = rd(ebp + 0xCC);
                        if v & 0x0800_0000 != 0 {
                            wd(ebp + 0xCC, v & 0xF7FF_FFFF);
                            let al = call_regs(0x0084_2B70, 0, 0, 0, 0, 0, &[]) as u8;
                            if al == 0 {
                                hp(38);
                                call_regs(0x004D_1270, 0, 0, 0, 0, 0, &[0x1F4, 0]);
                            }
                        }
                    } else {
                        hp(28);
                        let ecx = rd(ebp + 0x44);
                        let mut goto_7b22 = false;
                        let first = hv(ecx);
                        if first != 0 && rd(first + 0x4C) == 1 && rb(first + 0x3CC) == 1 {
                            goto_7b22 = true;
                        } else if rb(ebp + 0x3CC) == 2 && rd(ebp + 0x1178) != 0 {
                            goto_7b22 = true;
                        }
                        if !goto_7b22 {
                            // 7c5f: a human with no valid owner: report it and kill it
                            let mut finish = false;
                            if rb(0x0252_8B48) != 0 {
                                let rec = rd(ebp + 0xB0) as usize;
                                if rec != 0 {
                                    let g = rd(0x0252_8C14) as usize;
                                    if g != 0 {
                                        let e = rd(g + 0x408) as usize;
                                        if e != 0 && (rb(rec) != rb(e + 0x1B0) || rb(rec + 0x67C) & 1 != 0) {
                                            finish = true;
                                        }
                                    }
                                }
                            }
                            if !finish {
                                hp(36);
                                hp(37);
                                let (x, y, zz) = (rd(ebp + 0x14), rd(ebp + 0x18), rd(ebp + 0x1C));
                                let dx = (f32::from_bits(q(x)) as f64).to_bits();
                                let dy = (f32::from_bits(q(y)) as f64).to_bits();
                                let dz = (f32::from_bits(q(zz)) as f64).to_bits();
                                call_regs(
                                    0x00C9_68F6,
                                    0,
                                    0,
                                    0,
                                    0,
                                    0,
                                    &[ga(0x0234_60E0), ga(0x00E3_BBE8), dx as u32, (dx >> 32) as u32, dy as u32, (dy >> 32) as u32, dz as u32, (dz >> 32) as u32],
                                );
                                call_regs_st0(0x0098_47A0, 0, 0, 0, 0, 0, &[hh, q(rd(0x00E8_7998)), 0, 0]);
                                let vt = rd(ebp) as usize;
                                let f = rd(vt + 0x10) as usize;
                                call_regs(f, 0, hh, 0, 0, 0, &[0]);
                            }
                        } else {
                            hp(29);
                            // 7b22: skip humans whose owner record belongs to another player
                            let mut finish = false;
                            let a = hv(ecx);
                            if a != 0 {
                                let esi = rd(a + 0xB0) as usize;
                                if esi != 0 {
                                    let g = rd(0x0252_8C14) as usize;
                                    if g != 0 {
                                        let e = rd(g + 0x408) as usize;
                                        if e != 0 && (rb(esi) != rb(e + 0x1B0) || rb(esi + 0x67C) & 1 != 0) {
                                            finish = true;
                                        }
                                    }
                                }
                            }
                            if !finish {
                                if rd(ebp + 0xCC) & 0x0800_0000 == 0 {
                                    // 7b88
                                    if hv(ecx) == ebp && c0_pos(rf(ebp + 0x1124)) {
                                        let a = hv(ecx);
                                        let mut done = false;
                                        if a != 0 && rd(a + 0x4C) == 1 && rb(a + 0x3CC) == 1 {
                                            let r = call_regs(0x0095_E290, 0, 0, 0, 0, 0, &[hh]);
                                            if r == 0 {
                                                hp(30);
                                                call_regs(0x0096_5F40, 0, 0, 0, 0, 0, &[hh, 0]);
                                                call_regs(0x0096_6720, 0, 0, 0, 0, 0, &[hh, 0]);
                                                call_regs_st0(0x0098_47A0, 0, 0, 0, 0, 0, &[hh, q(rd(0x00E8_7998)), 0, 0]);
                                                done = true;
                                            }
                                        }
                                        if !done {
                                            hp(31);
                                            // 7cf8
                                            wd(ebp + 0xCC, rd(ebp + 0xCC) | 0x0800_0000);
                                            let ecx2 = rd(ebp + 0x44);
                                            let a = hv(ecx2);
                                            if a != 0 && rd(a + 0x4C) == 1 && rb(a + 0x3CC) == 1 {
                                                let al = call_regs(0x0084_2B70, 0, 0, 0, 0, 0, &[]) as u8;
                                                if al == 0 {
                                                    hp(32);
                                                    call_regs(0x00D1_4840, 0, 0, 0, 0, 0, &[0, 0, 0, 0xFF]);
                                                    call_regs(0x004D_1090, 0, 0, 0, 0, 0, &[0x1F4, 0, 0x3F80_0000]);
                                                }
                                                hp(39);
                                                call_regs(0x0050_5430, 0, 0, 0, 0, 0, &[]);
                                            } else {
                                                hp(33);
                                                *((ebp + 0x65A) as *mut u16) = 0;
                                            }
                                        }
                                    }
                                } else {
                                    // 7d6f: the "fell" flag is up
                                    let a = hv(ecx);
                                    if a != 0 && rd(a + 0x4C) == 1 && rb(a + 0x3CC) == 1 {
                                        hp(34);
                                        call_regs(0x009A_4240, 0, 0, 0, 0, 0, &[hh]);
                                    } else {
                                        hp(35);
                                        call_regs(0x009A_3E20, 0, 0, 0, hh, 0, &[]);
                                    }
                                }
                            }
                        }
                    }
                    // 7dac
                    if rd(ebp + 0xC8) & 0x10_0000 != 0 {
                        wd(ebp + 0x5B0, 0);
                    }
                }
                ebp = rd(ebp + 0xB8) as usize;
                if ebp == rd(RING) as usize || ebp == 0 {
                    break;
                }
            }
        }
        call_regs(0x006D_D990, 0, 0, 0, 0, 0, &[]);
        // the original jumps to 0x737C10 here (a tail call)
        call_regs(0x0073_7C10, 0, 0, 0, 0, 0, &[])
    }
}

// =================================================================================================
// Offline differential test kit: the original machine code and the Rust port run on the same
// random world; every call into the rest of the game goes through a recording stub.
// (Copied from port_activities.rs, with a few extra stub effects.)
// =================================================================================================
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_frame2 -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
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
            Harness { mem, orig, thunk, cmp_eax: false }
        }

        fn run(&self, f: usize, ecx: u32, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
            SRNG.with(|s| s.set(seed | 1));
            unsafe { std::ptr::write_bytes((STACK + STACK_LEN - 0x1_0000) as *mut u8, 0, 0x1_0000) };
            // a clean x87 (53-bit precision, as the game runs)
            unsafe { std::arch::asm!("fninit", "fldcw word ptr [{cw}]", cw = in(reg) &0x027Fu16 as *const u16, options(nostack)) };
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
mod hpu_test {
    use super::difftest::*;
    use super::*;

    const CB: usize = 0xCB00_0001;
    const ADDR: usize = 0x009A_7210;
    const LOCAL_PLAYER: usize = 0x0217_03D4;
    const RING: usize = 0x0215_E908;

    extern "C" fn memset_native(p: *mut u8, v: i32, n: usize) -> *mut u8 {
        unsafe { std::ptr::write_bytes(p, v as u8, n) };
        p
    }

    fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0x9a4a80, 0, 0, EDI, Ret::Any, Fx::None),
            callee(0x9b92e0, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xae28f0, 0, 0, ECX, Ret::Ptr(0), Fx::None),
            callee(0x983370, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x9a5840, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x99f6d0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x99f4a0, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x9a4ff0, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x9a5080, 0, 0, EDI, Ret::Any, Fx::None),
            callee(0x9a5100, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x6c6d90, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x6dacc0, 4, 16, ESI, Ret::Any, Fx::None),
            callee(0xce7d00, 3, 12, 0, Ret::Any, Fx::None),
            callee(0x9a4970, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x9a5ad0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x9a46d0, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x7360f0, 1, 4, ECX | EDX, Ret::Any, Fx::None),
            callee(0x735fe0, 1, 4, ECX | ESI, Ret::Any, Fx::None),
            callee(0xb6cda0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x9c31b0, 3, 0, EDI, Ret::Any, Fx::FillOut2),
            callee(0x465b90, 0, 0, EAX | ECX, Ret::Ecx, Fx::Vec465),
            callee(0x96a790, 0, 0, ECX | ESI, Ret::Esi, Fx::Out3Esi),
            callee(0xca0bc0, 3, 0, 0, Ret::Any, Fx::None),
            callee(0x9707c0, 3, 0, ECX, Ret::Any, Fx::None),
            callee(0x9c8ee0, 3, 0, 0, Ret::Any, Fx::None),
            callee(0x9bd360, 3, 0, 0, Ret::Any, Fx::None),
            callee(0x9bda80, 1, 0, EAX, Ret::Any, Fx::None),
            callee(0x9c1a30, 0, 0, EAX | ESI, Ret::Any, Fx::None),
            callee(0x842b70, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x4d1270, 2, 0, EDX, Ret::Any, Fx::None),
            callee(0x95e290, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x965f40, 2, 8, 0, Ret::Any, Fx::None),
            callee(0x966720, 2, 8, 0, Ret::Any, Fx::None),
            callee(0x9847a0, 4, 0, ECX | EDX, Ret::St0, Fx::None),
            callee(0xd14840, 4, 0, 0, Ret::Any, Fx::None),
            callee(0x4d1090, 3, 0, 0, Ret::Any, Fx::None),
            callee(0x505430, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x9a4240, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x9a3e20, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0xc968f6, 8, 0, 0, Ret::Any, Fx::None),
            callee(0x6dd990, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x737c10, 0, 0, 0, Ret::Any, Fx::None),
            callee(0xb69020, 2, 8, ESI, Ret::Any, Fx::None),
            callee(0xce6a70, 1, 4, EAX | ECX, Ret::Bool, Fx::None),
            callee(0xb07a30, 0, 0, ESI, Ret::Ptr(1), Fx::None),
            callee(CB, 1, 4, ECX, Ret::Any, Fx::None),
        ];
        v[22].native = memset_native as *const () as usize;
        v[22].silent = true;
        v[29].byte_regs = EDX;
        v
    }

    fn sane_float(rng: &mut Rng) -> u32 {
        match rng.below(6) {
            0 => 0,
            1 => 1.0f32.to_bits(),
            2 => (rng.below(2000) as f32 / 10.0 - 50.0).to_bits(),
            3 => 0.5f32.to_bits(),
            _ => (rng.below(100) as f32).to_bits(),
        }
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            h.mem.zero(HANDLE_TABLE, 0x1_0000);
            let mut a = Arena::new();
            let humans = a.pool(6, 0x1600);
            let objs = a.pool(6, 0x100);
            let wst = a.pool(8, 0x100);
            let infos = a.pool(8, 0x100);
            let rigs = a.pool(6, 0x400);
            let recs = a.pool(4, 0x700);
            let g = a.pool(1, 0x500)[0];
            let g2 = a.pool(1, 0x300)[0];
            let vt = a.pool(1, 0x40)[0];
            let retp = a.pool(4, 0x10);
            let vecs = a.pool(2, 0x20);
            let _ = vecs;
            put(vt, 0x10, redirect(CB) as u32);
            for &r in &retp {
                put(r, 4, rng.pick(&[1, 2, 0, 3, 1]));
            }
            set_pools(vec![humans.clone(), retp.clone()]);
            for &i in &infos {
                putb(i, 0x88, rng.u32() as u8);
                putb(i, 0x89, rng.u32() as u8);
                put(i, 0xCC, rng.pick(&[1, 3, 0, 2, 5, 1, 3]));
                put(i, 0x34, rng.pick(&[0x16, 0x16, 3, 0]));
            }
            for &w in &wst {
                put(w, 8, infos[rng.below(8) as usize]);
                putb(w, 0xC, rng.u32() as u8);
                put(w, 0x10, rng.mix(&[0, 1, 5, 0xFFFF_FFFF], 0));
                put(w, 0x14, rng.mix(&[0, 1, 5, 0xFFFF_FFFF], 0));
            }
            for &o in &objs {
                put(o, 0xC4, if rng.one_in(6) { 0 } else { wst[rng.below(8) as usize] });
                putb(o, 0xCC, rng.u32() as u8);
            }
            for &r in &rigs {
                put(r, 0, rng.mix(&[0, 5, 189, 190, 380, 12345, 0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF], 400));
                putb(r, 0x26C, rng.pick(&[0, 1, 1]));
            }
            for &r in &recs {
                putb(r, 0, rng.below(2) as u8);
                putb(r, 0x67C, rng.u32() as u8);
            }
            put(g, 0x408, if rng.one_in(5) { 0 } else { g2 });
            putb(g2, 0x1B0, rng.below(2) as u8);
            // handles
            let mut hs = Vec::new();
            for &hum in &humans {
                let slot = 1 + rng.below(4000);
                let hnd = (1 + rng.below(0xFF)) << 12 | slot;
                hs.push(hnd);
                wd(HANDLE_TABLE + slot as usize * 16, hum);
                put(hum, 0x44, hnd);
            }
            let now = rng.mix(&[0, 100, 5000, 0x6B49_D200 - 3000, 0x6B49_D200 - 1, 0x1000_0000], 0x6B49_D200);
            wd(0x00E8_43AC, now);
            for (i, &hum) in humans.iter().enumerate() {
                put(hum, 0, vt);
                put(hum, 0x14, sane_float(rng));
                put(hum, 0x18, match rng.below(6) {
                    0 => 50.0f32.to_bits(),
                    1 => 100.0f32.to_bits(),
                    2 => 150.0f32.to_bits(),
                    3 => 0x7FC0_0000,
                    4 => rng.float_bits(),
                    _ => (-20.0f32).to_bits(),
                });
                put(hum, 0x1C, sane_float(rng));
                match rng.below(10) {
                    0 => put(hum, 0x44, 0),
                    1 => put(hum, 0x44, hs[i] ^ 0x1000),
                    2 => put(hum, 0x44, hs[rng.below(6) as usize]),
                    _ => {}
                }
                put(hum, 0x48, rng.pick(&[0xFFFF, 0xFFFF, 7, 0x1234, 0]) );
                put(hum, 0x4C, rng.pick(&[1, 1, 1, 1, 5, 0, 2]));
                put(hum, 0xB0, if rng.one_in(5) { 0 } else { recs[rng.below(4) as usize] });
                put(hum, 0xC0, rng.u32());
                put(hum, 0xC8, rng.pick(&[0x10_0000, 0x10_0000, 0x10_0002, 0x10_0000, 2, 0x10_0002, 0]) | (rng.u32() & 0xFFF0));
                put(hum, 0xCC, rng.mix(&[0, 0x0800_0000, 0x0800_0000, 1], 0));
                putb(hum, 0x3CC, rng.pick(&[1, 1, 1, 2, 0]));
                put(hum, 0x570, rng.mix(&[0, 0, 3], 0));
                putb(hum, 0x591, rng.pick(&[6, 7, 0, 3, 7, 0]));
                putb(hum, 0x592, rng.pick(&[0x12, 0, 5, 0x12]));
                putb(hum, 0x595, rng.pick(&[0, 1, 1]));
                put(hum, 0x5B0, rng.float_bits());
                put(hum, 0x5E0, rigs[rng.below(6) as usize]);
                putb(hum, 0x614, rng.u32() as u8);
                put(hum, 0x65A, rng.u32());
                put(hum, 0xD74, if rng.one_in(3) { 0 } else { hs[rng.below(6) as usize] });
                put(hum, 0xF8C, rng.mix(&[0xFFFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 0, 5], 0));
                putb(hum, 0x10C8, rng.pick(&[0, 0, 1]));
                put(hum, 0x10CC, if rng.one_in(8) { 0 } else { objs[rng.below(6) as usize] });
                put(hum, 0x1124, match rng.below(6) {
                    0 => 0,
                    1 => (-1.0f32).to_bits(),
                    2 => 0x7FC0_0000,
                    3 => 0x8000_0000,
                    _ => 5.0f32.to_bits(),
                });
                putb(hum, 0x1163, rng.u32() as u8);
                put(hum, 0x1178, rng.pick(&[0, 0, humans[0]]));
                put(hum, 0x11DC, rng.u32());
                put(hum, 0x1554, match rng.below(8) {
                    0 => 0xFFFF_FFFF,
                    1 => now.wrapping_sub(5),
                    2 => now.wrapping_add(5),
                    3 => now.wrapping_add(0x35A4_E900),
                    4 => now.wrapping_sub(0x35A4_E900),
                    5 => 0,
                    _ => rng.u32() % 0x6B49_D200,
                });
            }
            // the ring: a random subset in a random order, closed (or cut off)
            let n = 1 + rng.below(6) as usize;
            let mut order: Vec<usize> = (0..6).collect();
            for k in (1..6).rev() {
                let j = rng.below(k as u32 + 1) as usize;
                order.swap(k, j);
            }
            let ring: Vec<u32> = order[..n].iter().map(|&k| humans[k]).collect();
            for k in 0..n {
                let nxt = if k + 1 == n { if rng.one_in(8) { 0 } else { ring[0] } } else { ring[k + 1] };
                put(ring[k], 0xB8, nxt);
            }
            for &hum in &humans {
                if !ring.contains(&hum) {
                    put(hum, 0xB8, 0);
                }
            }
            wd(RING, ring[0]);
            wd(LOCAL_PLAYER, if rng.one_in(6) { 0 } else { humans[rng.below(6) as usize] });
            wd(0x00EB_CE9C, if rng.one_in(3) { 0 } else { humans[rng.below(6) as usize] });
            wd(0x00EB_CEA0, rng.pick(&[7, 8, 1, 0, 7]));
            wd(0x0252_8C14, if rng.one_in(5) { 0 } else { g });
            for p in [0x00EE_33B0, 0x00EE_33B4, 0x00EE_33D8, 0x00DD_0BB4, 0x00E8_7998] {
                wd(p, sane_float(rng));
            }
            let thr = 100.0f64.to_bits();
            wd(0x00E5_C290, thr as u32);
            wd(0x00E5_C294, (thr >> 32) as u32);
            wb(0x0252_973A, rng.pick(&[0, 0, 1]));
            wb(0x027A_6A40, rng.u32() as u8);
            wb(0x0252_8B48, rng.pick(&[0, 1, 1, 2]));
            wb(0x0252_8B49, rng.pick(&[0, 0, 1]));
            wb(0x02A4_D2C5, rng.pick(&[0, 1]));
            wb(0x0252_970B, rng.pick(&[0, 1]));
            wd(0x015B_8710, rng.u32());
            wd(0x015B_870C, rng.u32());
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn humans_post_update_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &HUMANS_POST_BODY, &HUMANS_POST_BODY_CALLS, &HUMANS_POST_BODY_DISPS, callees(), &[(HANDLE_TABLE, 0x1_0000), (0x0340_D240, 0x4_6000)], humans_post_thunk as *const () as usize);
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
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
        for (i, n) in HP_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, HP_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

// --- object_items_update body (pin) ---
const OBJECT_ITEMS_BODY: [u8; 2246] = [
    0x83, 0xEC, 0x34, 0x53, 0x55, 0x8B, 0x2D, 0x90, 0xD7, 0x15, 0x02, 0x56, 0x57, 0x33, 0xFF, 0x33,
    0xF6, 0x33, 0xC9, 0x89, 0x7C, 0x24, 0x18, 0x89, 0x74, 0x24, 0x2C, 0x89, 0x4C, 0x24, 0x30, 0x89,
    0x7C, 0x24, 0x28, 0x89, 0x7C, 0x24, 0x24, 0x89, 0x7C, 0x24, 0x20, 0x89, 0x7C, 0x24, 0x1C, 0x90,
    0x3B, 0xEF, 0x0F, 0x84, 0x5E, 0x08, 0x00, 0x00, 0x8B, 0x85, 0xB8, 0x00, 0x00, 0x00, 0x8B, 0xD8,
    0x2B, 0x1D, 0x90, 0xD7, 0x15, 0x02, 0xF7, 0xDB, 0x1B, 0xDB, 0x23, 0xD8, 0xF6, 0x85, 0x80, 0x00,
    0x00, 0x00, 0x0B, 0x89, 0x5C, 0x24, 0x34, 0x0F, 0x85, 0x28, 0x08, 0x00, 0x00, 0x8B, 0x45, 0x7C,
    0x83, 0xF8, 0xFF, 0x74, 0x38, 0x8D, 0x0C, 0x40, 0x8B, 0x0C, 0xCD, 0xC0, 0x25, 0x8F, 0x02, 0x8B,
    0x89, 0xB8, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x24, 0x8A, 0x51, 0x26, 0xC0, 0xEA, 0x02, 0x80,
    0xE2, 0x03, 0x74, 0x19, 0x8D, 0x7D, 0x20, 0x8D, 0x75, 0x14, 0x50, 0xE8, 0xE0, 0xC0, 0xDC, 0xFF,
    0x8B, 0x45, 0x00, 0x8B, 0x50, 0x20, 0x83, 0xC4, 0x04, 0x8B, 0xCD, 0xFF, 0xD2, 0xBF, 0x01, 0x00,
    0x00, 0x00, 0x39, 0x7C, 0x24, 0x18, 0x7D, 0x11, 0x55, 0xE8, 0xD2, 0xF7, 0xFF, 0xFF, 0x83, 0xC4,
    0x04, 0x84, 0xC0, 0x74, 0x04, 0x01, 0x7C, 0x24, 0x18, 0x83, 0xBD, 0xF4, 0x00, 0x00, 0x00, 0x00,
    0x74, 0x4B, 0x8B, 0x45, 0x00, 0x8B, 0x10, 0x6A, 0xFF, 0x8D, 0x4C, 0x24, 0x3C, 0x51, 0x8B, 0xCD,
    0xFF, 0xD2, 0xD9, 0x44, 0x24, 0x3C, 0xD9, 0x44, 0x24, 0x38, 0xD9, 0x44, 0x24, 0x40, 0xD9, 0xC1,
    0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xDC, 0x1D, 0xA0, 0xC1, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4,
    0x05, 0x7A, 0x0A, 0xC7, 0x85, 0xF4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x83, 0xBD, 0xCC,
    0x00, 0x00, 0x00, 0x00, 0x74, 0x59, 0x66, 0x8B, 0x45, 0x48, 0xE8, 0x51, 0xC0, 0x14, 0x00, 0x0F,
    0xB7, 0xC0, 0x3D, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x1C, 0x3D, 0x66, 0x08, 0x00, 0x00, 0x73, 0x15,
    0x8B, 0x04, 0x85, 0xE8, 0xAA, 0xF7, 0x00, 0x85, 0xC0, 0x74, 0x0A, 0xF6, 0x40, 0x1C, 0x01, 0x75,
    0x04, 0xB0, 0x01, 0xEB, 0x02, 0x32, 0xC0, 0x8B, 0x8D, 0x80, 0x00, 0x00, 0x00, 0xC1, 0xE9, 0x05,
    0xF6, 0xD1, 0xF6, 0xC1, 0x01, 0x0F, 0x84, 0xBD, 0x00, 0x00, 0x00, 0x3C, 0x01, 0x0F, 0x85, 0xB5,
    0x00, 0x00, 0x00, 0x8B, 0xC5, 0xE8, 0xE6, 0x2A, 0x00, 0x00, 0xE9, 0xA9, 0x00, 0x00, 0x00, 0x8B,
    0x85, 0xE4, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x7D, 0x07, 0xB8, 0x00, 0xD2, 0x49, 0x6B, 0xEB, 0x33,
    0x8B, 0x15, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC2, 0x7E, 0x10, 0x2B, 0xC2, 0x3D, 0x00, 0xE9, 0xA4,
    0x35, 0x7E, 0x20, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0xEB, 0x19, 0x8B, 0xCA, 0x2B, 0xC8, 0x81, 0xF9,
    0x00, 0xE9, 0xA4, 0x35, 0x7E, 0x09, 0x2B, 0xC2, 0x05, 0x00, 0xD2, 0x49, 0x6B, 0xEB, 0x04, 0xF7,
    0xD9, 0x8B, 0xC1, 0xF7, 0xD8, 0x8B, 0xF0, 0x8B, 0x85, 0xC0, 0x00, 0x00, 0x00, 0xF6, 0x40, 0x4C,
    0x40, 0x74, 0x1F, 0x01, 0x7C, 0x24, 0x1C, 0x3B, 0x74, 0x24, 0x24, 0x7E, 0x4B, 0x8B, 0xC5, 0xE8,
    0x5C, 0x5F, 0xED, 0xFF, 0x84, 0xC0, 0x74, 0x40, 0x89, 0x74, 0x24, 0x24, 0x89, 0x6C, 0x24, 0x30,
    0xEB, 0x36, 0x8A, 0x8D, 0xBC, 0x00, 0x00, 0x00, 0xF6, 0xC1, 0x20, 0x75, 0x2B, 0x81, 0x78, 0x48,
    0xF0, 0xFD, 0x92, 0x00, 0x75, 0x05, 0xF6, 0xC1, 0x08, 0x75, 0x1D, 0x01, 0x7C, 0x24, 0x20, 0x3B,
    0x74, 0x24, 0x28, 0x7E, 0x13, 0x8B, 0xC5, 0xE8, 0x24, 0x5F, 0xED, 0xFF, 0x84, 0xC0, 0x74, 0x08,
    0x89, 0x74, 0x24, 0x28, 0x89, 0x6C, 0x24, 0x2C, 0x8B, 0x8D, 0xC0, 0x00, 0x00, 0x00, 0xD9, 0x85,
    0x00, 0x01, 0x00, 0x00, 0xD8, 0x61, 0x40, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x05, 0x84, 0x59, 0xDD,
    0x00, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x54, 0xD9, 0xE0,
    0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x4B, 0xD9, 0xE8, 0xD9, 0x05, 0x7C, 0x43, 0xE8,
    0x00, 0x80, 0x79, 0x44, 0x00, 0x0F, 0x84, 0x39, 0x01, 0x00, 0x00, 0xD9, 0x85, 0x04, 0x01, 0x00,
    0x00, 0xD8, 0xE2, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xD3, 0xDF, 0xE0, 0xF6,
    0xC4, 0x05, 0x0F, 0x8A, 0xC7, 0x00, 0x00, 0x00, 0xD9, 0xE0, 0xD8, 0xDB, 0xDF, 0xE0, 0xDD, 0xDA,
    0xF6, 0xC4, 0x05, 0x0F, 0x8A, 0xBA, 0x00, 0x00, 0x00, 0xD9, 0xEE, 0xD9, 0xE8, 0xE9, 0xEE, 0x00,
    0x00, 0x00, 0xDD, 0xD8, 0xD9, 0x41, 0x40, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x41, 0x40, 0xD9, 0x85, 0x00, 0x01, 0x00, 0x00, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4,
    0x05, 0x7A, 0x44, 0xD9, 0x05, 0x7C, 0x43, 0xE8, 0x00, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x14, 0xD8,
    0x85, 0x00, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x95, 0x00,
    0x01, 0x00, 0x00, 0xD9, 0x41, 0x40, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x74, 0x05, 0xDD,
    0xD8, 0xD9, 0x41, 0x40, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x9D, 0x00, 0x01,
    0x00, 0x00, 0xE9, 0x5A, 0xFF, 0xFF, 0xFF, 0xD9, 0x85, 0x00, 0x01, 0x00, 0x00, 0xD9, 0x05, 0x7C,
    0x43, 0xE8, 0x00, 0xD9, 0xC0, 0xD8, 0x4C, 0x24, 0x14, 0xDE, 0xEA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x95, 0x00, 0x01, 0x00, 0x00, 0xD9, 0x41, 0x40, 0xD8, 0xD9,
    0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x05, 0xDD, 0xD8, 0xD9, 0x41, 0x40, 0xD9, 0x5C, 0x24, 0x14,
    0xD9, 0x44, 0x24, 0x14, 0xD9, 0x9D, 0x00, 0x01, 0x00, 0x00, 0xE9, 0x12, 0xFF, 0xFF, 0xFF, 0xDD,
    0xD8, 0xDD, 0xDA, 0xD9, 0x85, 0x04, 0x01, 0x00, 0x00, 0xD8, 0xC2, 0xD9, 0x5C, 0x24, 0x14, 0xD9,
    0x44, 0x24, 0x14, 0xD9, 0x95, 0x04, 0x01, 0x00, 0x00, 0xD9, 0xE8, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6,
    0xC4, 0x41, 0x75, 0x08, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x14, 0xEB, 0x06, 0xDD, 0xD9, 0xD9, 0x54,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x9D, 0x04, 0x01, 0x00, 0x00, 0xD9, 0xEE, 0xD9, 0xC9,
    0xD9, 0xCB, 0xDC, 0x35, 0xE0, 0xC9, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x05, 0x8C, 0x1C,
    0xDD, 0x00, 0xEB, 0x10, 0xDD, 0xDA, 0xD9, 0xEE, 0xD9, 0x95, 0x04, 0x01, 0x00, 0x00, 0xD9, 0xE8,
    0xEB, 0xDE, 0xD9, 0xC9, 0xD9, 0x44, 0x24, 0x14, 0xD8, 0xD2, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x0F,
    0x85, 0x85, 0x00, 0x00, 0x00, 0x80, 0xBD, 0x0C, 0x01, 0x00, 0x00, 0x00, 0x74, 0x36, 0xD8, 0x85,
    0x08, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x95, 0x08, 0x01,
    0x00, 0x00, 0xD8, 0xD4, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x52, 0xD8, 0xE3, 0xC6, 0x85, 0x0C,
    0x01, 0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0xCB, 0xD9, 0x95, 0x08, 0x01, 0x00, 0x00,
    0xD9, 0xCB, 0xEB, 0xB0, 0xD8, 0xAD, 0x08, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44,
    0x24, 0x14, 0xD9, 0x95, 0x08, 0x01, 0x00, 0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A,
    0x1C, 0xDC, 0x2D, 0x70, 0xBF, 0xE5, 0x00, 0xC6, 0x85, 0x0C, 0x01, 0x00, 0x00, 0x01, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x95, 0x08, 0x01, 0x00, 0x00, 0xE9, 0x77, 0xFF, 0xFF, 0xFF, 0xDD, 0xD8, 0xD9,
    0xC9, 0xD9, 0x54, 0x24, 0x14, 0xE9, 0x68, 0xFF, 0xFF, 0xFF, 0xF6, 0x41, 0x4C, 0x04, 0xDD, 0xD8,
    0xDD, 0xDB, 0xDD, 0xDA, 0xDD, 0xD9, 0xDD, 0xD8, 0x0F, 0x85, 0x37, 0x04, 0x00, 0x00, 0x8B, 0x85,
    0xCC, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0D, 0x8B, 0x80, 0x80, 0x00, 0x00, 0x00, 0x83, 0xF8,
    0xFF, 0x8B, 0xF8, 0x75, 0x03, 0x8B, 0x79, 0x3C, 0xF6, 0x85, 0xBC, 0x00, 0x00, 0x00, 0x20, 0x0F,
    0x84, 0x08, 0x02, 0x00, 0x00, 0x83, 0xFF, 0xFF, 0x0F, 0x84, 0xFF, 0x01, 0x00, 0x00, 0x8B, 0xB5,
    0xD4, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x8D, 0x95, 0xD4, 0x00, 0x00, 0x00, 0x0F, 0x8C, 0xC6, 0x01,
    0x00, 0x00, 0x8B, 0xC2, 0xE8, 0xA7, 0xC7, 0x2A, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0xB3, 0x01, 0x00,
    0x00, 0x8B, 0xC5, 0xC7, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0xE8, 0x82, 0x5C, 0xED, 0xFF, 0x84, 0xC0,
    0x0F, 0x84, 0xC7, 0x01, 0x00, 0x00, 0x8B, 0x55, 0x00, 0x8B, 0x42, 0x1C, 0x6A, 0x01, 0x6A, 0x01,
    0x8B, 0xCD, 0xFF, 0xD0, 0x8B, 0x4D, 0x7C, 0x8D, 0x7D, 0x20, 0x8D, 0x75, 0x14, 0x51, 0xE8, 0x9D,
    0xBC, 0xDC, 0xFF, 0x8A, 0x15, 0x48, 0x8B, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x32, 0xDB, 0x80, 0xFA,
    0x01, 0x0F, 0x85, 0xC0, 0x00, 0x00, 0x00, 0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x0F, 0x85,
    0xB3, 0x00, 0x00, 0x00, 0x8B, 0x85, 0xC0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x0F, 0x84, 0xA5, 0x00,
    0x00, 0x00, 0x8B, 0x0D, 0x00, 0x6C, 0xE9, 0x00, 0x83, 0xF9, 0x0B, 0x0F, 0x8C, 0x96, 0x00, 0x00,
    0x00, 0x83, 0xF9, 0x0D, 0x0F, 0x8F, 0x8D, 0x00, 0x00, 0x00, 0x8B, 0x00, 0xBE, 0x70, 0xD1, 0xA4,
    0x02, 0xE8, 0xEA, 0x8D, 0xF6, 0xFF, 0x83, 0xF8, 0x06, 0x75, 0x7C, 0x8B, 0x8D, 0xC0, 0x00, 0x00,
    0x00, 0x8B, 0x11, 0x52, 0xE8, 0xB7, 0x0B, 0x24, 0x00, 0x8B, 0xF8, 0x8B, 0x85, 0xCC, 0x00, 0x00,
    0x00, 0x83, 0xC4, 0x04, 0x85, 0xFF, 0x74, 0x5F, 0x0F, 0xB6, 0x40, 0x70, 0xC0, 0xE8, 0x03, 0x25,
    0x01, 0xFF, 0xFF, 0xFF, 0x50, 0xB8, 0x06, 0x00, 0x00, 0x00, 0xE8, 0x71, 0x91, 0xF6, 0xFF, 0x85,
    0xC0, 0x74, 0x44, 0x3B, 0xC7, 0x74, 0x40, 0x8B, 0x8D, 0xCC, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74,
    0x06, 0x8B, 0x50, 0x40, 0x89, 0x51, 0x74, 0x8B, 0x48, 0x40, 0x89, 0x8D, 0xC0, 0x00, 0x00, 0x00,
    0x8B, 0x50, 0x40, 0x8B, 0x42, 0x14, 0x89, 0x85, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0x85, 0xC8, 0x00,
    0x00, 0x00, 0x85, 0xC0, 0xB3, 0x02, 0x74, 0x0F, 0xE8, 0xF3, 0x10, 0x24, 0x00, 0xC7, 0x85, 0xC8,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8B, 0xB5, 0xC0, 0x00, 0x00, 0x00, 0xE8, 0xBE, 0x0B,
    0x24, 0x00, 0x85, 0xC0, 0x0F, 0x84, 0x8D, 0x00, 0x00, 0x00, 0x8B, 0x8D, 0xC8, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x75, 0x20, 0x8B, 0xF0, 0xE8, 0x05, 0x0F, 0x24, 0x00, 0x8B, 0x8D, 0xCC, 0x00, 0x00,
    0x00, 0x85, 0xC9, 0x89, 0x85, 0xC8, 0x00, 0x00, 0x00, 0x74, 0x5E, 0x8B, 0x49, 0x78, 0x85, 0xC9,
    0x7C, 0x57, 0xEB, 0x21, 0x0F, 0xB7, 0x90, 0xDC, 0x00, 0x00, 0x00, 0x89, 0x51, 0x10, 0x8B, 0x8D,
    0xCC, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x24, 0x8B, 0x49, 0x78, 0x85, 0xC9, 0x7C, 0x1D, 0x8B,
    0x85, 0xC8, 0x00, 0x00, 0x00, 0x89, 0x48, 0x14, 0x8B, 0x8D, 0xCC, 0x00, 0x00, 0x00, 0x8B, 0x95,
    0xC8, 0x00, 0x00, 0x00, 0x8B, 0x41, 0x7C, 0x89, 0x42, 0x20, 0xEB, 0x1D, 0x0F, 0xB7, 0x88, 0xDC,
    0x00, 0x00, 0x00, 0x8B, 0x95, 0xC8, 0x00, 0x00, 0x00, 0x89, 0x4A, 0x14, 0x8B, 0x85, 0xC8, 0x00,
    0x00, 0x00, 0xC7, 0x40, 0x20, 0x00, 0x00, 0x00, 0x00, 0x83, 0xCB, 0x01, 0x53, 0x8B, 0xD5, 0xE8,
    0x8C, 0x4D, 0xF1, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xC5, 0xE8, 0x92, 0x2D, 0x00, 0x00, 0x8B, 0x5C,
    0x24, 0x34, 0xEB, 0x29, 0x85, 0xF6, 0x7D, 0x25, 0xF6, 0x85, 0x80, 0x00, 0x00, 0x00, 0x04, 0x75,
    0x13, 0x8B, 0x85, 0xC8, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x12, 0xE8, 0x40, 0x20, 0x24, 0x00,
    0x84, 0xC0, 0x75, 0x09, 0x8B, 0xC7, 0x8B, 0xCA, 0xE8, 0x93, 0xC4, 0x2A, 0x00, 0xF6, 0x85, 0x80,
    0x00, 0x00, 0x00, 0x04, 0x0F, 0x85, 0xFB, 0x01, 0x00, 0x00, 0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02,
    0x80, 0xF9, 0x01, 0x75, 0x0E, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0x0F, 0x84, 0xC9,
    0x00, 0x00, 0x00, 0x83, 0xBD, 0xCC, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xC2, 0x01, 0x00, 0x00,
    0xF6, 0x85, 0xBC, 0x00, 0x00, 0x00, 0x08, 0x0F, 0x85, 0xB5, 0x01, 0x00, 0x00, 0x8B, 0x85, 0xC0,
    0x00, 0x00, 0x00, 0xF7, 0x40, 0x4C, 0x00, 0x01, 0x00, 0x00, 0x0F, 0x85, 0xA2, 0x01, 0x00, 0x00,
    0x8D, 0xB5, 0xD0, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0xE8, 0x63, 0xC5, 0x2A, 0x00, 0x84, 0xC0, 0x0F,
    0x84, 0x8D, 0x01, 0x00, 0x00, 0x8B, 0xC5, 0xE8, 0x44, 0x5A, 0xED, 0xFF, 0x84, 0xC0, 0x0F, 0x84,
    0x7E, 0x01, 0x00, 0x00, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0xF4, 0x01, 0x00, 0x00, 0x3D, 0x00,
    0xD2, 0x49, 0x6B, 0x89, 0x06, 0x7E, 0x07, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x06, 0xF6, 0x85,
    0xBC, 0x00, 0x00, 0x00, 0x10, 0x75, 0x38, 0x8B, 0x35, 0x3C, 0x04, 0x17, 0x02, 0x8B, 0xD6, 0x85,
    0xD2, 0x74, 0x2C, 0x8D, 0x7D, 0x14, 0x8D, 0x4A, 0x14, 0x8B, 0xC7, 0xE8, 0x10, 0xDD, 0x2A, 0x00,
    0xD8, 0x1D, 0x78, 0x07, 0xDD, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F, 0x8B, 0x31, 0x01, 0x00,
    0x00, 0x8B, 0x92, 0x94, 0x12, 0x00, 0x00, 0x3B, 0xD6, 0x74, 0x04, 0x85, 0xD2, 0x75, 0xD7, 0x8B,
    0x55, 0x00, 0x8B, 0x42, 0x10, 0x6A, 0x00, 0x8B, 0xCD, 0xFF, 0xD0, 0x8B, 0x74, 0x24, 0x2C, 0x8B,
    0x4C, 0x24, 0x30, 0x8B, 0xEB, 0x33, 0xFF, 0xE9, 0xC4, 0xF8, 0xFF, 0xFF, 0x8A, 0x0D, 0x48, 0x8B,
    0x52, 0x02, 0x84, 0xC9, 0x0F, 0x84, 0xF8, 0x00, 0x00, 0x00, 0x8B, 0xC5, 0xE8, 0xAF, 0x59, 0xED,
    0xFF, 0x84, 0xC0, 0x0F, 0x84, 0xE9, 0x00, 0x00, 0x00, 0x83, 0xBD, 0xCC, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0x85, 0xDC, 0x00, 0x00, 0x00, 0xF6, 0x85, 0xBC, 0x00, 0x00, 0x00, 0x28, 0x0F, 0x85, 0xCF,
    0x00, 0x00, 0x00, 0x8B, 0x95, 0xC0, 0x00, 0x00, 0x00, 0xF7, 0x42, 0x4C, 0x00, 0x01, 0x00, 0x00,
    0x0F, 0x85, 0xBC, 0x00, 0x00, 0x00, 0xE8, 0x05, 0x2A, 0xED, 0xFF, 0x84, 0xC0, 0x0F, 0x85, 0xAF,
    0x00, 0x00, 0x00, 0x8D, 0x95, 0xD0, 0x00, 0x00, 0x00, 0x8B, 0xC2, 0xE8, 0x70, 0xC4, 0x2A, 0x00,
    0x84, 0xC0, 0x0F, 0x84, 0x9A, 0x00, 0x00, 0x00, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0xF4, 0x01,
    0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x02, 0x7E, 0x07, 0x05, 0x00, 0x2E, 0xB6, 0x94,
    0x89, 0x02, 0xF6, 0x85, 0xBC, 0x00, 0x00, 0x00, 0x10, 0x75, 0x5A, 0x8B, 0x35, 0x3C, 0x04, 0x17,
    0x02, 0x8B, 0xD6, 0x85, 0xD2, 0x74, 0x4E, 0x8D, 0x7D, 0x14, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8D, 0x4A, 0x14, 0x8B, 0xC7, 0xE8, 0x26, 0xDC, 0x2A, 0x00, 0xD8, 0x1D, 0x78, 0x07, 0xDD, 0x00,
    0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x2A, 0x8B, 0x92, 0x94, 0x12, 0x00, 0x00, 0x3B, 0xD6, 0x74,
    0x24, 0x85, 0xD2, 0x75, 0xDB, 0x8B, 0x45, 0x00, 0x52, 0x8B, 0x50, 0x10, 0x8B, 0xCD, 0xFF, 0xD2,
    0x8B, 0x74, 0x24, 0x2C, 0x8B, 0x4C, 0x24, 0x30, 0x8B, 0xEB, 0x33, 0xFF, 0xE9, 0xDF, 0xF7, 0xFF,
    0xFF, 0x85, 0xD2, 0x75, 0x1D, 0x8B, 0x45, 0x00, 0x8B, 0x50, 0x10, 0x6A, 0x00, 0x8B, 0xCD, 0xFF,
    0xD2, 0x8B, 0x74, 0x24, 0x2C, 0x8B, 0x4C, 0x24, 0x30, 0x8B, 0xEB, 0x33, 0xFF, 0xE9, 0xBE, 0xF7,
    0xFF, 0xFF, 0x8B, 0x85, 0xC0, 0x00, 0x00, 0x00, 0xF6, 0x40, 0x4C, 0x01, 0x74, 0x07, 0x8B, 0xC5,
    0xE8, 0xCB, 0x2A, 0x00, 0x00, 0x8B, 0x74, 0x24, 0x2C, 0x8B, 0x4C, 0x24, 0x30, 0x8B, 0xEB, 0x33,
    0xFF, 0xE9, 0x9A, 0xF7, 0xFF, 0xFF, 0x3B, 0xCF, 0x74, 0x0F, 0x83, 0x7C, 0x24, 0x1C, 0x32, 0x7E,
    0x08, 0x8B, 0x11, 0x8B, 0x42, 0x10, 0x57, 0xFF, 0xD0, 0x3B, 0xF7, 0x74, 0x11, 0x83, 0x7C, 0x24,
    0x20, 0x0F, 0x7E, 0x0A, 0x8B, 0x16, 0x8B, 0x42, 0x10, 0x57, 0x8B, 0xCE, 0xFF, 0xD0, 0x5F, 0x5E,
    0x5D, 0x5B, 0x83, 0xC4, 0x34, 0xC3,
];
#[allow(dead_code)]
const OBJECT_ITEMS_BODY_CALLS: [usize; 27] = [0x8b, 0xa9, 0x11a, 0x165, 0x1cf, 0x207, 0x494, 0x4a9, 0x4ce, 0x521, 0x534, 0x55a, 0x598, 0x5ad, 0x5c6, 0x63f, 0x649, 0x66b, 0x678, 0x6d8, 0x6e7, 0x72b, 0x77c, 0x7b6, 0x7cb, 0x815, 0x880];
#[allow(dead_code)]
const OBJECT_ITEMS_BODY_DISPS: [usize; 26] = [0x7, 0x42, 0x6b, 0xf8, 0x133, 0x182, 0x22d, 0x24d, 0x2b5, 0x2ff, 0x384, 0x38e, 0x413, 0x4d5, 0x4e8, 0x504, 0x51d, 0x68c, 0x697, 0x6f5, 0x719, 0x732, 0x76e, 0x7d9, 0x7fd, 0x81c];

// =================================================================================================
// 0x9305F0  object_items_update  (2246 bytes, no arguments)
// =================================================================================================
//
// Walks the object_item list at [0x215D790] (links +0xB8; the next link is taken before the item
// is handled, because the item may destroy itself). Per item: physics-body sync (0x6FC760 and
// vtable slot 8), the velocity-squared test that clears +0xF4, then either the ownerless-item
// scoring (items beyond 0x32 / 0xF "ownerless" ones get the best-scoring one destroyed after the
// loop) or the attachment hook 0x933240, the blend of +0x100 / +0x104 towards the type's
// targets and the animation phase step (+0x108 with the direction flag +0x10C), the type swap
// of the item (0x899900 / 0x899CC0 / 0xB71760 / 0xB71AC0 / 0x8459C0) and the despawn rules
// (vtable slot 4 with 0, 0x933940).
//
// x87: the original's blend / phase code keeps four constants on the x87 stack; every f32 store
// and compare is reproduced with the polarities read from the disassembly (see comments).
// Registers: no arguments; every callee gets the registers the original sets for it (ESI / EDI
// for 0x6FC760 and 0x899CC0, ESI = 0x2A4D170 for 0x899900 / 0x899CC0, the pushed EBX whose low
// byte is the "swapped" flag for 0x8459C0).
//
// Check: per-branch counters plus the offline difftest.

const OI_N: usize = 40;
static OI_HITS: [AtomicU32; OI_N] = [const { AtomicU32::new(0) }; OI_N];
const OI_NAMES: [&str; OI_N] = [
    "calls", "items", "skip_flags", "body_sync", "c92fe70", "vel_check", "vel_clear", "owner",
    "owner_hook", "ownerless", "score_a", "score_b", "blend_adjust", "blend_lo", "blend_hi", "q_adjust",
    "t44_zero", "phase_loop", "phase_dir1", "phase_dir0", "skip_4c", "timer_due", "vehicle_swap", "swap_done",
    "type_lookup", "c8_new", "c8_update", "c9339d0", "esi_neg", "path_a", "path_b", "near_keep",
    "despawn", "c933940", "end_a", "end_b", "bdd230_zero", "c6dacc0", "c8459c0", "unused",
];

fn oi(i: usize) {
    OI_HITS[i].fetch_add(1, Ordering::Relaxed);
}

/// A virtual call: thiscall on `obj`, slot `slot`; EAX / EDX hold the vtable and the method in
/// the order the call site loaded them.
#[inline(always)]
unsafe fn vcall(obj: usize, slot: usize, eax_is_method: bool, args: &[u32]) -> u32 {
    let vt = rd(obj) as usize;
    let f = rd(vt + slot * 4) as usize;
    let (eax, edx) = if eax_is_method { (f, vt) } else { (vt, f) };
    call_regs(f, eax as u32, obj as u32, edx as u32, 0, 0, args)
}

/// The wrapped game timestamp `now + 500` the item timers are re-armed with.
#[inline(always)]
unsafe fn now_plus_500() -> u32 {
    let mut eax = rd(0x00E8_43AC).wrapping_add(0x1F4);
    if (eax as i32) > 0x6B49_D200 {
        eax = eax.wrapping_add(0x94B6_2E00);
    }
    eax
}

kxs0!(object_items_thunk, object_items_update);

extern "C" fn object_items_update() {
    unsafe {
        oi(0);
        const HEAD: usize = 0x0215_D790;
        let mut ebp = rd(HEAD) as usize;
        let mut cnt_a: i32 = 0; // B+0x18
        let mut cnt_c: i32 = 0; // B+0x1c
        let mut cnt_b: i32 = 0; // B+0x20
        let mut score_a: i32 = 0; // B+0x24
        let mut score_b: i32 = 0; // B+0x28
        let mut best_b: usize = 0; // B+0x2c
        let mut best_a: usize = 0; // B+0x30
        let mut outv = [0u32; 3]; // B+0x38 (the velocity the vtable call returns)
        while ebp != 0 {
            oi(1);
            let eax = rd(ebp + 0xB8);
            let next = if eax != rd(HEAD) { eax as usize } else { 0 }; // ebx
            'item: {
                if rb(ebp + 0x80) & 0xB != 0 {
                    oi(2);
                    break 'item;
                }
                // physics body sync
                let idx = rd(ebp + 0x7C);
                if idx != 0xFFFF_FFFF {
                    let e = rd(0x028F_25C0usize.wrapping_add((idx.wrapping_mul(3) as usize).wrapping_mul(8))) as usize;
                    let q = rd(e + 0xB8) as usize;
                    if q != 0 && (rb(q + 0x26) >> 2) & 3 != 0 {
                        oi(3);
                        call_regs(0x006F_C760, idx, 0, 0, (ebp + 0x14) as u32, (ebp + 0x20) as u32, &[idx]);
                        vcall(ebp, 8, false, &[]);
                    }
                }
                if cnt_a < 1 {
                    oi(4);
                    let al = call_regs(0x0092_FE70, 0, 0, 0, 0, 0, &[ebp as u32]) as u8;
                    if al != 0 {
                        cnt_a += 1;
                    }
                }
                if rd(ebp + 0xF4) != 0 {
                    oi(5);
                    vcall(ebp, 0, false, &[outv.as_mut_ptr() as u32, 0xFFFF_FFFF]);
                    let (x, y, z) = (f32::from_bits(outv[0]) as f64, f32::from_bits(outv[1]) as f64, f32::from_bits(outv[2]) as f64);
                    let len2 = (((y * y) + (x * x)) + z * z) as f32;
                    if (len2 as f64) < rq(0x00E5_C1A0) {
                        oi(6);
                        wd(ebp + 0xF4, 0);
                    }
                }
                if rd(ebp + 0xCC) != 0 {
                    oi(7);
                    // attached to an owner: the item-class hook
                    let ax = rw(ebp + 0x48) as u32;
                    let r = call_regs(0x00A7_C760, ax, 0, 0, 0, 0, &[]) & 0xFFFF;
                    let ok = if r != 0xFFFF && r < 0x866 {
                        let e = rd(0x00F7_AAE8 + r as usize * 4) as usize;
                        e != 0 && rb(e + 0x1C) & 1 == 0
                    } else {
                        false
                    };
                    if rd(ebp + 0x80) & 0x20 == 0 && ok {
                        oi(8);
                        call_regs(0x0093_3240, ebp as u32, 0, 0, 0, 0, &[]);
                    }
                } else {
                    oi(9);
                    let mut eax = rd(ebp + 0xE4) as i32;
                    if eax < 0 {
                        eax = 0x6B49_D200;
                    } else {
                        let now = rd(0x00E8_43AC) as i32;
                        if eax > now {
                            eax = eax.wrapping_sub(now);
                            if eax > 0x35A4_E900 {
                                eax = eax.wrapping_add(0x94B6_2E00u32 as i32);
                            }
                        } else {
                            let ecx = now.wrapping_sub(eax);
                            if ecx <= 0x35A4_E900 {
                                eax = ecx.wrapping_neg();
                            } else {
                                eax = eax.wrapping_sub(now).wrapping_add(0x6B49_D200);
                            }
                        }
                    }
                    let esi = eax.wrapping_neg();
                    let t = rd(ebp + 0xC0) as usize;
                    if rb(t + 0x4C) & 0x40 != 0 {
                        cnt_c += 1;
                        if esi > score_a {
                            let al = call_regs(0x0080_6720, ebp as u32, 0, 0, 0, 0, &[]) as u8;
                            if al != 0 {
                                oi(10);
                                score_a = esi;
                                best_a = ebp;
                            }
                        }
                    } else {
                        let cl = rb(ebp + 0xBC);
                        if cl & 0x20 == 0 && !(rd(t + 0x48) == 0x0092_FDF0 && cl & 8 != 0) {
                            cnt_b += 1;
                            if esi > score_b {
                                let al = call_regs(0x0080_6720, ebp as u32, 0, 0, 0, 0, &[]) as u8;
                                if al != 0 {
                                    oi(11);
                                    score_b = esi;
                                    best_b = ebp;
                                }
                            }
                        }
                    }
                }

                // ---- blend of +0x100 / +0x104 and the phase step ----
                let t = rd(ebp + 0xC0) as usize;
                let k1 = rf(0x00DD_5984) as f64;
                let dt = rf(0x00E8_437C) as f64;
                let t40 = rf(t + 0x40) as f64;
                let p = rf(ebp + 0x100) as f64;
                let d = (p - t40) as f32 as f64;
                if d >= k1 || -d >= k1 || d.is_nan() {
                    oi(12);
                    let e = (t40 - 1.0) as f32 as f64;
                    if p >= t40 || p.is_nan() {
                        oi(14);
                        let f = (p - dt * e) as f32;
                        let keep = t40 < f as f64;
                        if keep {
                            wd(ebp + 0x100, f.to_bits());
                        } else {
                            wd(ebp + 0x100, q(rd(t + 0x40)));
                        }
                    } else {
                        oi(13);
                        let f = (dt * e + p) as f32;
                        let keep = t40 > f as f64;
                        if keep {
                            wd(ebp + 0x100, f.to_bits());
                        } else {
                            wd(ebp + 0x100, q(rd(t + 0x40)));
                        }
                    }
                }
                if rb(t + 0x44) != 0 {
                    let q0 = rf(ebp + 0x104) as f64;
                    let g = (q0 - 1.0) as f32 as f64;
                    if g >= k1 || -g >= k1 || g.is_nan() {
                        oi(15);
                        let w = (q0 + dt) as f32;
                        let v = if 1.0 > w as f64 { w } else { 1.0 };
                        wd(ebp + 0x104, v.to_bits());
                    }
                } else {
                    oi(16);
                    wd(ebp + 0x104, 0);
                }
                let mut h = (dt / rq(0x00E5_C9E0)) as f32;
                let k2 = rf(0x00DD_1C8C);
                let dd = rq(0x00E5_BF70);
                loop {
                    if !(h as f64 > 0.0) {
                        break;
                    }
                    oi(17);
                    if rb(ebp + 0x10C) != 0 {
                        oi(18);
                        let t_ = (h as f64 + rf(ebp + 0x108) as f64) as f32;
                        wd(ebp + 0x108, t_.to_bits());
                        if t_ as f64 <= 1.0 || t_.is_nan() {
                            break;
                        }
                        wb(ebp + 0x10C, 0);
                        h = (t_ as f64 - 1.0) as f32;
                        wd(ebp + 0x108, 0x3F80_0000);
                    } else {
                        oi(19);
                        let t_ = (rf(ebp + 0x108) as f64 - h as f64) as f32;
                        wd(ebp + 0x108, t_.to_bits());
                        if t_ as f64 >= k2 as f64 || t_.is_nan() {
                            break;
                        }
                        wb(ebp + 0x10C, 1);
                        h = (dd - t_ as f64) as f32;
                        wd(ebp + 0x108, q(k2.to_bits()));
                    }
                }
                if rb(t + 0x4C) & 4 != 0 {
                    oi(20);
                    break 'item;
                }

                // ---- the despawn timer / type swap ----
                let o = rd(ebp + 0xCC) as usize;
                let edi = if o != 0 && rd(o + 0x80) != 0xFFFF_FFFF { rd(o + 0x80) } else { rd(t + 0x3C) };
                if rb(ebp + 0xBC) & 0x20 != 0 && edi != 0xFFFF_FFFF {
                    let tm = rd(ebp + 0xD4);
                    let edx = ebp + 0xD4;
                    if (tm as i32) >= 0 {
                        let al = call_regs(0x00BD_D230, edx as u32, 0, 0, 0, 0, &[]) as u8;
                        if al == 0 {
                            oi(36);
                        } else {
                            oi(21);
                            wd(edx, 0xFFFF_FFFF);
                            let al = call_regs(0x0080_6720, ebp as u32, 0, 0, 0, 0, &[]) as u8;
                            if al != 0 {
                                vcall(ebp, 7, true, &[1, 1]);
                                call_regs(0x006F_C760, 0, rd(ebp + 0x7C), 0, (ebp + 0x14) as u32, (ebp + 0x20) as u32, &[rd(ebp + 0x7C)]);
                                let esi_c = 0x02A4_D170u32;
                                let mut bl: u8 = 0;
                                if rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0 && rd(ebp + 0xC0) != 0 {
                                    let ecx = rd(0x00E9_6C00) as i32;
                                    if (0xB..=0xD).contains(&ecx) {
                                        let r = call_regs(0x0089_9900, rd(rd(ebp + 0xC0) as usize), 0, 0, ga(esi_c as usize), 0, &[]);
                                        if r == 6 {
                                            let edi_v = call_regs(0x00B7_16E0, 0, 0, 0, 0, 0, &[rd(rd(ebp + 0xC0) as usize)]);
                                            let o = rd(ebp + 0xCC) as usize;
                                            if edi_v != 0 {
                                                let a = ((rb(o + 0x70) >> 3) & 1) as u32;
                                                let r2 = call_regs(0x0089_9CC0, 6, 0, 0, ga(esi_c as usize), edi_v, &[a]);
                                                if r2 != 0 && r2 != edi_v {
                                                    oi(22);
                                                    let r2 = r2 as usize;
                                                    let ecx = rd(ebp + 0xCC) as usize;
                                                    if ecx != 0 {
                                                        wd(ecx + 0x74, rd(r2 + 0x40));
                                                    }
                                                    wd(ebp + 0xC0, rd(r2 + 0x40));
                                                    let edx2 = rd(r2 + 0x40) as usize;
                                                    wd(ebp + 0xC4, rd(edx2 + 0x14));
                                                    let c8 = rd(ebp + 0xC8);
                                                    bl = 2;
                                                    if c8 != 0 {
                                                        oi(23);
                                                        call_regs(0x00B7_1C80, c8, 0, 0, 0, 0, &[]);
                                                        wd(ebp + 0xC8, 0);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                // 930b97
                                let r = call_regs(0x00B7_1760, 0, 0, 0, rd(ebp + 0xC0), 0, &[]) as usize;
                                if r != 0 {
                                    oi(24);
                                    let c8 = rd(ebp + 0xC8) as usize;
                                    if c8 == 0 {
                                        oi(25);
                                        let n = call_regs(0x00B7_1AC0, 0, 0, 0, r as u32, 0, &[]);
                                        wd(ebp + 0xC8, n);
                                        let o = rd(ebp + 0xCC) as usize;
                                        if o != 0 {
                                            let ecx = rd(o + 0x78);
                                            if (ecx as i32) >= 0 {
                                                wd(n as usize + 0x14, ecx);
                                                let ecx = rd(ebp + 0xCC) as usize;
                                                let edx = rd(ebp + 0xC8) as usize;
                                                wd(edx + 0x20, rd(ecx + 0x7C));
                                            }
                                        }
                                    } else {
                                        oi(26);
                                        wd(c8 + 0x10, rw(r + 0xDC) as u32);
                                        let o = rd(ebp + 0xCC) as usize;
                                        let ecx = if o != 0 { rd(o + 0x78) } else { 0xFFFF_FFFF };
                                        if o != 0 && (ecx as i32) >= 0 {
                                            let c8 = rd(ebp + 0xC8) as usize;
                                            wd(c8 + 0x14, ecx);
                                            let ecx = rd(ebp + 0xCC) as usize;
                                            let edx = rd(ebp + 0xC8) as usize;
                                            wd(edx + 0x20, rd(ecx + 0x7C));
                                        } else {
                                            let c8 = rd(ebp + 0xC8) as usize;
                                            wd(c8 + 0x14, rw(r + 0xDC) as u32);
                                            let c8 = rd(ebp + 0xC8) as usize;
                                            wd(c8 + 0x20, 0);
                                        }
                                    }
                                    call_regs(0x0084_59C0, 0, 0, ebp as u32, 0, 0, &[(next as u32 & 0xFFFF_FF00) | bl as u32 | 1]);
                                    oi(38);
                                }
                                oi(27);
                                call_regs(0x0093_39D0, ebp as u32, 0, 0, 0, 0, &[]);
                            }
                        }
                    } else {
                        oi(28);
                        if rb(ebp + 0x80) & 4 != 0 || {
                            let c8 = rd(ebp + 0xC8);
                            c8 != 0 && call_regs(0x00B7_2CA0, c8, 0, 0, 0, 0, &[]) as u8 == 0
                        } {
                            call_regs(0x00BD_D100, edi, edx as u32, 0, 0, 0, &[]);
                        }
                    }
                }

                // ---- despawn rules ----
                if rb(ebp + 0x80) & 4 != 0 {
                    break 'item;
                }
                let cl = rb(0x0252_8B48);
                let despawned = if cl == 1 && rb(0x0252_8B49) == 0 {
                    oi(30);
                    // path B
                    if rb(0x0252_8B48) == 0
                        || call_regs(0x0080_6720, ebp as u32, 0, 0, 0, 0, &[]) as u8 == 0
                        || rd(ebp + 0xCC) != 0
                        || rb(ebp + 0xBC) & 0x28 != 0
                        || rd(rd(ebp + 0xC0) as usize + 0x4C) & 0x100 != 0
                        || call_regs(0x0080_37B0, 0, 0, 0, 0, 0, &[]) as u8 != 0
                        || call_regs(0x00BD_D230, (ebp + 0xD0) as u32, 0, 0, 0, 0, &[]) as u8 == 0
                    {
                        false
                    } else {
                        wd(ebp + 0xD0, now_plus_500());
                        if ring_has_player_near(ebp) {
                            false
                        } else {
                            oi(32);
                            vcall(ebp, 4, false, &[0]);
                            true
                        }
                    }
                } else {
                    oi(29);
                    // path A
                    if rd(ebp + 0xCC) != 0
                        || rb(ebp + 0xBC) & 8 != 0
                        || rd(rd(ebp + 0xC0) as usize + 0x4C) & 0x100 != 0
                        || call_regs(0x00BD_D230, (ebp + 0xD0) as u32, 0, 0, 0, 0, &[]) as u8 == 0
                        || call_regs(0x0080_6720, ebp as u32, 0, 0, 0, 0, &[]) as u8 == 0
                    {
                        false
                    } else {
                        wd(ebp + 0xD0, now_plus_500());
                        if ring_has_player_near(ebp) {
                            false
                        } else {
                            oi(32);
                            vcall(ebp, 4, true, &[0]);
                            true
                        }
                    }
                };
                if despawned {
                    break 'item;
                }
                // 930e62
                let t = rd(ebp + 0xC0) as usize;
                if rb(t + 0x4C) & 1 != 0 {
                    oi(33);
                    call_regs(0x0093_3940, ebp as u32, 0, 0, 0, 0, &[]);
                }
            }
            ebp = next;
        }
        if best_a != 0 && cnt_c > 0x32 {
            oi(34);
            vcall(best_a, 4, true, &[0]);
        }
        if best_b != 0 && cnt_b > 0xF {
            oi(35);
            vcall(best_b, 4, true, &[0]);
        }
    }
}

/// The player ring walk of the despawn rules: true when a player (ring at [0x217043C], links
/// +0x1294) is within 2500 units of the item; the walk is skipped when the item has the +0xBC
/// bit 0x10.
#[inline(always)]
unsafe fn ring_has_player_near(ebp: usize) -> bool {
    if rb(ebp + 0xBC) & 0x10 != 0 {
        return false;
    }
    let first = rd(0x0217_043C) as usize;
    let mut edx = first;
    if edx == 0 {
        return false;
    }
    loop {
        let d = call_regs_st0(0x00BD_EA30, (ebp + 0x14) as u32, (edx + 0x14) as u32, 0, 0, 0, &[]);
        if d < rf(0x00DD_0778) as f64 {
            return true;
        }
        edx = rd(edx + 0x1294) as usize;
        if edx == first || edx == 0 {
            return false;
        }
    }
}

#[cfg(test)]
mod oiu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0093_05F0;
    const CB0: usize = 0xCB00_0010;
    const CB4: usize = 0xCB00_0014;
    const CB7: usize = 0xCB00_0017;
    const CB8: usize = 0xCB00_0018;
    const HEAD: usize = 0x0215_D790;

    pub(super) fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0x6fc760, 1, 0, ESI | EDI, Ret::Any, Fx::None),
            callee(0x8037b0, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x806720, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x8459c0, 1, 0, EDX, Ret::Any, Fx::None),
            callee(0x899900, 0, 0, EAX | ESI, Ret::Ptr(0), Fx::None),
            callee(0x899cc0, 1, 4, EAX | ESI | EDI, Ret::Ptr(1), Fx::None),
            callee(0x92fe70, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x933240, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x933940, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x9339d0, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0xa7c760, 0, 0, EAX, Ret::Ptr(2), Fx::None),
            callee(0xb716e0, 1, 0, 0, Ret::Ptr(3), Fx::None),
            callee(0xb71760, 0, 0, ESI, Ret::Ptr(1), Fx::None),
            callee(0xb71ac0, 0, 0, ESI, Ret::PtrNz(1), Fx::None),
            callee(0xb71c80, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0xb72ca0, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xbdd100, 0, 0, EAX | ECX, Ret::Any, Fx::None),
            callee(0xbdd230, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xbdea30, 0, 0, EAX | ECX, Ret::St0, Fx::None),
            callee(CB0, 2, 8, ECX, Ret::Any, Fx::WriteVecArg0),
            callee(CB4, 1, 4, ECX, Ret::Any, Fx::None),
            callee(CB7, 2, 8, ECX, Ret::Any, Fx::None),
            callee(CB8, 0, 0, ECX, Ret::Any, Fx::None),
        ];
        v[10].word_regs = EAX;
        v
    }

    fn fl_small(rng: &mut Rng) -> u32 {
        match rng.below(8) {
            0 => 0,
            1 => 1.0f32.to_bits(),
            2 => 0.5f32.to_bits(),
            3 => (rng.below(1000) as f32 / 1000.0).to_bits(),
            4 => (rng.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
            5 => (rng.below(3000) as f32 / 1000.0).to_bits(),
            6 => 0.0005f32.to_bits(),
            _ => (rng.below(1000) as f32 / 100.0 - 5.0).to_bits(),
        }
    }

    pub(super) unsafe fn world(h: &Harness, rng: &mut Rng, big: bool) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let items = a.pool(70, 0x200);
            let types = a.pool(8, 0x100);
            let owners = a.pool(4, 0x100);
            let c8s = a.pool(4, 0x100);
            let misc = a.pool(8, 0x100);
            let bodies = a.pool(8, 0x100);
            let qs = a.pool(8, 0x40);
            let vt = a.pool(1, 0x40)[0];
            let players = a.pool(3, 0x1300);
            let tbl_objs = a.pool(4, 0x40);
            set_pools(vec![vec![6, 6, 6, 3, 5], misc.clone(), vec![0xFFFF, 5, 100, 0x865, 0x866, 0x900, 17, 18, 3], vec![1, 2, 3, 4, 6]]);
            put(vt, 0, redirect(CB0) as u32);
            put(vt, 0x10, redirect(CB4) as u32);
            put(vt, 0x1C, redirect(CB7) as u32);
            put(vt, 0x20, redirect(CB8) as u32);
            for &t in &types {
                put(t, 0, rng.below(10));
                put(t, 0x3C, rng.mix(&[0, 1, 0xFFFF_FFFF, 5], 100));
                put(t, 0x40, fl_small(rng));
                putb(t, 0x44, rng.pick(&[0, 1, 1]));
                put(t, 0x48, rng.pick(&[0x0092_FDF0, 0, 0x0092_FDF0, 5]));
                put(t, 0x4C, rng.pick(&[0x40, 4, 0x100, 1, 0, 0x41, 5, 0x140, 0]));
                if big { putb(t, 0x4C, rng.pick(&[0x40, 0x40, 0, 1])); put(t, 0x48, 0); }
            }
            for &m in &misc {
                put(m, 0x40, types[rng.below(8) as usize]);
                put(m, 0xDC, rng.u32());
            }
            for &t in &types {
                put(t, 0x14, rng.u32());
            }
            for &o in &owners {
                putb(o, 0x70, rng.u32() as u8);
                put(o, 0x78, rng.mix(&[0, 5, 0xFFFF_FFFF], 100));
                put(o, 0x7C, rng.u32());
                put(o, 0x80, rng.mix(&[0xFFFF_FFFF, 0xFFFF_FFFF, 3, 0], 50));
            }
            for k in 0..8 {
                put(bodies[k], 0xB8, qs[k]);
                putb(qs[k], 0x26, rng.u32() as u8);
                // the physics body table: 24 bytes per entry
                wd(0x028F_25C0 + k * 24, bodies[k]);
            }
            for &t in &tbl_objs {
                putb(t, 0x1C, rng.u32() as u8);
            }
            for k in 0..0x866usize {
                wd(0x00F7_AAE8 + k * 4, if rng.one_in(3) { 0 } else { tbl_objs[rng.below(4) as usize] });
            }
            // globals
            let b48 = if big { rng.pick(&[0u8, 2, 1]) } else { rng.pick(&[0u8, 1, 1, 1, 2]) };
            let b49 = if big { 1 } else { rng.pick(&[0u8, 0, 0, 1]) };
            wb(0x0252_8B48, b48);
            wb(0x0252_8B49, b49);
            let veh = b48 == 1 && b49 == 0;
            wd(0x00E9_6C00, rng.pick(&[0xA, 0xB, 0xC, 0xD, 0xE, 0xB]));
            let now = rng.mix(&[0, 100, 5000, 0x6B49_D200 - 3000, 0x6B49_D200 - 1, 0x1000_0000], 0x6B49_D200);
            wd(0x00E8_43AC, now);
            wd(0x00DD_5984, 0.001f32.to_bits());
            wd(0x00DD_1C8C, 0.5f32.to_bits());
            wd(0x00DD_0778, 2500.0f32.to_bits());
            let c = 2.5f64.to_bits();
            wd(0x00E5_C9E0, c as u32);
            wd(0x00E5_C9E4, (c >> 32) as u32);
            let c = 0.5f64.to_bits();
            wd(0x00E5_BF70, c as u32);
            wd(0x00E5_BF74, (c >> 32) as u32);
            let c = 0.0001f64.to_bits();
            wd(0x00E5_C1A0, c as u32);
            wd(0x00E5_C1A4, (c >> 32) as u32);
            wd(0x00E8_437C, rng.pick(&[0.016666668f32.to_bits(), 0.05f32.to_bits(), 0, 0.016666668f32.to_bits(), 0.0f32.to_bits(), 0.4f32.to_bits()]));
            // players
            for (i, &p) in players.iter().enumerate() {
                put(p, 0x14, fl_small(rng));
                put(p, 0x1294, if rng.one_in(6) { 0 } else { players[(i + 1) % 3] });
            }
            wd(0x0217_043C, if rng.one_in(5) { 0 } else { players[rng.below(3) as usize] });
            // items
            for &it in &items {
                put(it, 0, vt);
                put(it, 0x14, rng.float_bits());
                put(it, 0x7C, rng.pick(&[0xFFFF_FFFF, 0, 1, 2, 3, 7, 5, 0xFFFF_FFFF]));
                put(it, 0x80, if big { rng.pick(&[0, 0x20, 0x24, 0x10]) } else { rng.pick(&[0, 0, 0, 0xB, 4, 0x20, 0x24, 0x10, 0x8, 0x2, 0x1]) | if rng.one_in(2) { 0x20 } else { 0 } });
                put(it, 0x48, rng.u32());
                putb(it, 0xBC, rng.u32() as u8);
                put(it, 0xC0, if big { types[rng.below(3) as usize] } else { types[rng.below(8) as usize] });
                put(it, 0xC4, rng.u32());
                put(it, 0xC8, if rng.one_in(2) { 0 } else { c8s[rng.below(4) as usize] });
                put(it, 0xCC, if big { 0 } else if veh || (rng.one_in(3) && !rng.one_in(8)) { owners[rng.below(4) as usize] } else { 0 });
                put(it, 0xD0, rng.mix(&[0, 0xFFFF_FFFF, 5], 0x6B49_D200));
                put(it, 0xD4, rng.mix(&[0, 0xFFFF_FFFF, 5, 0x8000_0000, 0x7FFF_FFFF, 5000], 0x6B49_D200));
                put(it, 0xE4, rng.mix(&[0, 0xFFFF_FFFF, 5, now, now.wrapping_add(5), now.wrapping_sub(5), now.wrapping_add(0x35A4_E900), now.wrapping_sub(0x35A4_E900), 0x7FFF_FFFF], 0x6B49_D200));
                put(it, 0xF4, rng.pick(&[0, 1, 5]));
                put(it, 0x100, fl_small(rng));
                put(it, 0x104, fl_small(rng));
                put(it, 0x108, rng.pick(&[0.0f32, 0.25, 0.5, 0.75, 1.0, 0.3, 0.9]).to_bits());
                putb(it, 0x10C, rng.pick(&[0, 1, 1, 0]));
            }
            let n = if big { 52 + rng.below(18) as usize } else { 1 + rng.below(6) as usize };
            for k in 0..n {
                let nxt = if k + 1 == n { if rng.one_in(8) { 0 } else { items[0] } } else { items[k + 1] };
                put(items[k], 0xB8, nxt);
            }
            for k in n..70 {
                put(items[k], 0xB8, 0);
            }
            wd(HEAD, items[0]);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn object_items_update_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &OBJECT_ITEMS_BODY, &OBJECT_ITEMS_BODY_CALLS, &OBJECT_ITEMS_BODY_DISPS, callees(), &[(0x028F_25C0, 0x100), (0x00F7_AAE8, 0x2200)], object_items_thunk as *const () as usize);
        let mut rng = Rng(0x2468_ACE0_1357_9BDF);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng, false) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in OI_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, OI_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod oiu_big_test {
    use super::difftest::*;
    use super::*;

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn object_items_update_big_ring_matches_original() {
        let _g = lock();
        let h = Harness::new(0x0093_05F0, &OBJECT_ITEMS_BODY, &OBJECT_ITEMS_BODY_CALLS, &OBJECT_ITEMS_BODY_DISPS, oiu_test::callees(), &[(0x028F_25C0, 0x100), (0x00F7_AAE8, 0x2200)], object_items_thunk as *const () as usize);
        let mut rng = Rng(0x1357_9BDF_2468_ACE1);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { oiu_test::world(&h, &mut rng, true) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in OI_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, OI_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

// --- humans_movement_update body (pin) ---
const HUMANS_MOVE_BODY: [u8; 692] = [
    0xB8, 0x68, 0x20, 0x00, 0x00, 0xE8, 0x86, 0x6F, 0x2F, 0x00, 0xD9, 0xEE, 0xD8, 0x9C, 0x24, 0x6C,
    0x20, 0x00, 0x00, 0x53, 0x55, 0x56, 0x57, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x0F, 0x8B, 0x87, 0x02,
    0x00, 0x00, 0x8B, 0x1D, 0x08, 0xE9, 0x15, 0x02, 0x33, 0xFF, 0x85, 0xDB, 0x89, 0x7C, 0x24, 0x14,
    0x0F, 0x84, 0xF8, 0x00, 0x00, 0x00, 0x8D, 0xAC, 0x24, 0x84, 0x00, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0xF6, 0x83, 0x80, 0x00, 0x00, 0x00, 0x01, 0x0F, 0x85, 0xCB, 0x00, 0x00, 0x00, 0x53, 0xE8, 0x4D,
    0xBD, 0xFD, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x2C, 0x38, 0x83, 0x95, 0x05, 0x00, 0x00,
    0x75, 0x24, 0xD9, 0xEE, 0x89, 0x5D, 0xF4, 0xD9, 0x55, 0xF8, 0x83, 0xC7, 0x01, 0xD9, 0x55, 0xFC,
    0x89, 0x7C, 0x24, 0x14, 0xD9, 0x55, 0x00, 0xD9, 0x55, 0x04, 0xD9, 0x55, 0x08, 0xD9, 0x5D, 0x0C,
    0x88, 0x45, 0x10, 0x83, 0xC5, 0x20, 0x8B, 0xC3, 0xE8, 0x53, 0x7F, 0xFF, 0xFF, 0x84, 0xC0, 0x0F,
    0x84, 0x83, 0x00, 0x00, 0x00, 0x8B, 0x83, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x08, 0x85, 0xC9, 0x7C,
    0x29, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1,
    0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8, 0x05,
    0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x75, 0x08, 0xD9, 0xEE, 0xD9, 0x54, 0x24, 0x10,
    0xEB, 0x0C, 0x51, 0xE8, 0xB8, 0xF4, 0x33, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0xEE, 0xD9, 0x44,
    0x24, 0x10, 0xDD, 0x05, 0xA0, 0xBF, 0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x05,
    0x84, 0x05, 0xDD, 0x00, 0xD9, 0x44, 0x24, 0x10, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F,
    0x8A, 0xD2, 0x00, 0x00, 0x00, 0xD9, 0xE0, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F, 0x8A,
    0xC7, 0x00, 0x00, 0x00, 0xDD, 0xD8, 0xDD, 0xD8, 0x8B, 0x9B, 0xB8, 0x00, 0x00, 0x00, 0x3B, 0x1D,
    0x08, 0xE9, 0x15, 0x02, 0x74, 0x08, 0x85, 0xDB, 0x0F, 0x85, 0x12, 0xFF, 0xFF, 0xFF, 0x85, 0xFF,
    0x0F, 0x86, 0x73, 0x01, 0x00, 0x00, 0x8D, 0x9C, 0x24, 0x88, 0x00, 0x00, 0x00, 0x8B, 0xEF, 0xD9,
    0x84, 0x24, 0x7C, 0x20, 0x00, 0x00, 0x8B, 0x73, 0xF0, 0x51, 0xD9, 0x1C, 0x24, 0x8D, 0x43, 0x0C,
    0x50, 0x53, 0x8D, 0x4B, 0xF4, 0x51, 0x56, 0xE8, 0x84, 0x7F, 0xFF, 0xFF, 0x83, 0xC4, 0x14, 0x3C,
    0x01, 0x0F, 0x85, 0xBA, 0x00, 0x00, 0x00, 0xD9, 0x86, 0xC8, 0x05, 0x00, 0x00, 0xD9, 0x84, 0x24,
    0x7C, 0x20, 0x00, 0x00, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x86,
    0xCC, 0x05, 0x00, 0x00, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x28, 0xD8, 0x8E, 0xD0, 0x05, 0x00, 0x00,
    0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x46, 0x14, 0xD8, 0x44, 0x24, 0x24, 0xD9, 0x5C, 0x24, 0x18, 0x8B,
    0x54, 0x24, 0x18, 0xD9, 0x46, 0x18, 0xD8, 0x44, 0x24, 0x28, 0xD9, 0x5C, 0x24, 0x1C, 0x8B, 0x44,
    0x24, 0x1C, 0xD9, 0x46, 0x1C, 0x8B, 0xB6, 0xE0, 0x05, 0x00, 0x00, 0xD8, 0x44, 0x24, 0x2C, 0x81,
    0xC6, 0xCC, 0x01, 0x00, 0x00, 0x89, 0x16, 0x89, 0x46, 0x04, 0xD9, 0x5C, 0x24, 0x20, 0x8B, 0x4C,
    0x24, 0x20, 0x89, 0x4E, 0x08, 0xEB, 0x51, 0xDD, 0xD8, 0xDD, 0xD8, 0x83, 0xEC, 0x0C, 0xD9, 0x5C,
    0x24, 0x08, 0x8D, 0x4C, 0x24, 0x3C, 0xD9, 0x54, 0x24, 0x04, 0xD9, 0x1C, 0x24, 0x51, 0xE8, 0x0D,
    0x93, 0x23, 0x00, 0x8D, 0x54, 0x24, 0x30, 0x8D, 0x7B, 0x20, 0x52, 0x8B, 0xCF, 0x8D, 0x44, 0x24,
    0x58, 0xE8, 0xBA, 0x94, 0x23, 0x00, 0x8B, 0xF0, 0x8D, 0x43, 0x20, 0xB9, 0x09, 0x00, 0x00, 0x00,
    0x50, 0xF3, 0xA5, 0xE8, 0xC8, 0xA6, 0x23, 0x00, 0x8B, 0x7C, 0x24, 0x14, 0xE9, 0xF7, 0xFE, 0xFF,
    0xFF, 0xC7, 0x43, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x83, 0xC3, 0x20, 0x83, 0xED, 0x01, 0x0F, 0x85,
    0x0B, 0xFF, 0xFF, 0xFF, 0x85, 0xFF, 0x76, 0x71, 0x8D, 0x5C, 0x24, 0x78, 0x8B, 0xEF, 0x8B, 0x03,
    0x85, 0xC0, 0x74, 0x27, 0xD9, 0x84, 0x24, 0x7C, 0x20, 0x00, 0x00, 0x51, 0x8D, 0x50, 0x20, 0xD9,
    0x1C, 0x24, 0x8D, 0x48, 0x14, 0x52, 0x8D, 0xB0, 0xC8, 0x05, 0x00, 0x00, 0x8B, 0x80, 0xE0, 0x05,
    0x00, 0x00, 0x51, 0xE8, 0x28, 0x7B, 0xD4, 0xFF, 0x83, 0xC4, 0x0C, 0x83, 0xC3, 0x20, 0x83, 0xED,
    0x01, 0x75, 0xCB, 0x85, 0xFF, 0x76, 0x32, 0x8D, 0xB4, 0x24, 0x94, 0x00, 0x00, 0x00, 0x8B, 0x4E,
    0xE4, 0x85, 0xC9, 0x74, 0x1C, 0xD9, 0x84, 0x24, 0x7C, 0x20, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24,
    0x56, 0x8D, 0x56, 0xF4, 0x52, 0x8D, 0x46, 0xE8, 0x50, 0xE8, 0x42, 0x8B, 0xFF, 0xFF, 0x83, 0xC4,
    0x10, 0x83, 0xC6, 0x20, 0x83, 0xEF, 0x01, 0x75, 0xD5, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0x68,
    0x20, 0x00, 0x00, 0xC3,
];
#[allow(dead_code)]
const HUMANS_MOVE_BODY_CALLS: [usize; 10] = [0x5, 0x4e, 0x88, 0xd3, 0x157, 0x1ee, 0x201, 0x213, 0x263, 0x299];
#[allow(dead_code)]
const HUMANS_MOVE_BODY_DISPS: [usize; 5] = [0x24, 0xc4, 0xe4, 0xf0, 0x120];

// =================================================================================================
// 0x9A9890  humans_movement_update  (692 bytes, cdecl, one float argument `dt`)
// =================================================================================================
//
// sim_frame:4. Returns at once when dt == 0.0. Otherwise four passes over the human ring at
// [0x215E908] (links +0xB8):
//   1. every human without +0x80 bit 0: when it is neither a dead proxy (0x985630) nor flagged
//      (+0x595), a 32-byte entry {human, 6 zero floats, flag byte} is appended to a stack array;
//      the humans for which 0x9A1870 says yes get the yaw fix: the rig's ce8e20 value minus the
//      double at 0xE5BFA0; when it is outside +-[0xDD0584], a rotation about z (0xBE2D90) is
//      composed (0xBE2F50) into the matrix at human+0x20 and orthonormalised (0xBE4170);
//   2. per entry 0x9A1970(human, &e+4, &e+0x10, &e+0x1C, dt); on 1 the rig position at
//      rig+0x1CC becomes pos + velocity (+0x5C8..) * dt, otherwise the entry is cleared;
//   3. per live entry 0x6F1620 (EAX = rig, ESI = human+0x5C8);
//   4. per live entry 0x9A2670 (ECX = human).
// The original's 0x2068-byte frame holds ~255 entries; past that it would overwrite its own return
// address, here the extra humans are simply not appended (counted as `overflow`).
// Check: per-branch counters plus the offline difftest.

const HM_N: usize = 12;
static HM_HITS: [AtomicU32; HM_N] = [const { AtomicU32::new(0) }; HM_N];
const HM_NAMES: [&str; HM_N] = ["calls", "dt0", "humans", "skip_flag", "appended", "yaw_check", "yaw_fix", "p2_ok", "p2_cleared", "p3", "p4", "overflow"];

fn hm(i: usize) {
    HM_HITS[i].fetch_add(1, Ordering::Relaxed);
}

kxs1!(humans_movement_thunk, humans_movement_update);

extern "C" fn humans_movement_update(dt_bits: u32) {
    unsafe {
        hm(0);
        let dt = f32::from_bits(dt_bits);
        if dt == 0.0 {
            hm(1);
            return;
        }
        const RING: usize = 0x0215_E908;
        const MAX: usize = 255;
        // entry k at ent + k * 0x20: [+0] human, [+4..+0x1C) six floats, [+0x1C] flag byte
        let mut ent = std::mem::MaybeUninit::<[u32; MAX * 8]>::uninit();
        let eb = ent.as_mut_ptr() as usize;
        let mut count = 0usize;
        let mut m1 = [0u32; 9];
        let mut m2 = [0u32; 9];
        let dtq = q(dt_bits);

        let mut ebx = rd(RING) as usize;
        if ebx != 0 {
            loop {
                hm(2);
                if rb(ebx + 0x80) & 1 != 0 {
                    hm(3);
                } else {
                    let al = call_regs(0x0098_5630, 0, 0, 0, 0, 0, &[ebx as u32]) as u8;
                    if al == 0 && rb(ebx + 0x595) == 0 {
                        if count < MAX {
                            hm(4);
                            let a = eb + count * 0x20;
                            *(a as *mut u32) = ebx as u32;
                            for k in 1..8 {
                                *((a + k * 4) as *mut u32) = 0;
                            }
                            count += 1;
                        } else {
                            hm(11);
                        }
                    }
                    let al = call_regs(0x009A_1870, ebx as u32, 0, 0, 0, 0, &[]) as u8;
                    if al != 0 {
                        hm(5);
                        let rig = rd(ebx + 0x5E0) as usize;
                        let id = rd(rig) as i32;
                        let v = if id < 0 {
                            0.0f32
                        } else {
                            call_regs_st0(0x00CE_8E20, 0, 0, 0, 0, 0, &[ga(rig_rec(id))]) as f32
                        };
                        let u = (v as f64 - rq(0x00E5_BFA0)) as f32 as f64;
                        let k = rf(0x00DD_0584) as f64;
                        if u >= k || -u >= k || u.is_nan() {
                            hm(6);
                            call_regs(0x00BE_2D90, 0, 0, 0, 0, 0, &[m1.as_mut_ptr() as u32, 0, 0, q(v.to_bits())]);
                            let r = call_regs(0x00BE_2F50, m2.as_mut_ptr() as u32, (ebx + 0x20) as u32, 0, 0, 0, &[m1.as_mut_ptr() as u32]) as usize;
                            for i in 0..9 {
                                let w = *((r + i * 4) as *const u32);
                                wd(ebx + 0x20 + i * 4, w);
                            }
                            call_regs(0x00BE_4170, 0, 0, 0, 0, 0, &[(ebx + 0x20) as u32]);
                        }
                    }
                }
                ebx = rd(ebx + 0xB8) as usize;
                if ebx == rd(RING) as usize || ebx == 0 {
                    break;
                }
            }
        }
        if count == 0 {
            return;
        }
        // pass 2
        for k in 0..count {
            let a = eb + k * 0x20;
            let human = *(a as *const u32) as usize;
            let al = call_regs(0x009A_1970, 0, 0, 0, 0, 0, &[human as u32, (a + 4) as u32, (a + 0x10) as u32, (a + 0x1C) as u32, dtq]) as u8;
            if al == 1 {
                hm(7);
                let d = dtq;
                let dd = f32::from_bits(d) as f64;
                let v24 = (rf(human + 0x5C8) as f64 * dd) as f32 as f64;
                let v28 = (rf(human + 0x5CC) as f64 * dd) as f32 as f64;
                let v2c = (dd * rf(human + 0x5D0) as f64) as f32 as f64;
                let x = (rf(human + 0x14) as f64 + v24) as f32;
                let y = (rf(human + 0x18) as f64 + v28) as f32;
                let z = (rf(human + 0x1C) as f64 + v2c) as f32;
                let rig = rd(human + 0x5E0) as usize + 0x1CC;
                wd(rig, x.to_bits());
                wd(rig + 4, y.to_bits());
                wd(rig + 8, z.to_bits());
            } else {
                hm(8);
                *(a as *mut u32) = 0;
            }
        }
        // pass 3
        for k in 0..count {
            let a = eb + k * 0x20;
            let human = *(a as *const u32) as usize;
            if human != 0 {
                hm(9);
                let rig = rd(human + 0x5E0);
                call_regs(0x006F_1620, rig, 0, 0, (human + 0x5C8) as u32, 0, &[(human + 0x14) as u32, (human + 0x20) as u32, dtq]);
            }
        }
        // pass 4
        for k in 0..count {
            let a = eb + k * 0x20;
            let human = *(a as *const u32);
            if human != 0 {
                hm(10);
                call_regs(0x009A_2670, 0, human, 0, 0, 0, &[(a + 4) as u32, (a + 0x10) as u32, (a + 0x1C) as u32, dtq]);
            }
        }
    }
}

#[cfg(test)]
mod hmu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x009A_9890;
    const RING: usize = 0x0215_E908;

    /// `__alloca_probe`: EAX bytes of stack are taken from the caller.
    #[unsafe(naked)]
    extern "C" fn alloca_native() {
        std::arch::naked_asm!("pop ecx", "sub esp, eax", "push ecx", "ret")
    }

    fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0xca0820, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x985630, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x9a1870, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xce8e20, 1, 4, 0, Ret::St0, Fx::None),
            callee(0xbe2d90, 4, 16, 0, Ret::Any, Fx::None),
            callee(0xbe2f50, 1, 4, EAX | ECX, Ret::Keep, Fx::Fill9Eax),
            callee(0xbe4170, 1, 4, 0, Ret::Any, Fx::LogArg0(9)),
            callee(0x9a1970, 5, 0, 0, Ret::Bool, Fx::Fill1970),
            callee(0x6f1620, 3, 0, EAX | ESI, Ret::Any, Fx::None),
            callee(0x9a2670, 4, 0, ECX, Ret::Any, Fx::Log2670),
        ];
        v[0].native = alloca_native as *const () as usize;
        v[0].silent = true;
        v
    }

    fn fl_small(rng: &mut Rng) -> u32 {
        match rng.below(8) {
            0 => 0,
            1 => 1.0f32.to_bits(),
            2 => 0.5f32.to_bits(),
            3 => (rng.below(1000) as f32 / 10.0).to_bits(),
            4 => (rng.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
            5 => 0x0000_0001,
            6 => 0x8000_0001,
            _ => (rng.below(1000) as f32 / 100.0 - 5.0).to_bits(),
        }
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) -> u32 {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let humans = a.pool(12, 0x700);
            let rigs = a.pool(12, 0x400);
            set_pools(vec![]);
            for (i, &hum) in humans.iter().enumerate() {
                put(hum, 0x80, rng.pick(&[0, 0, 0, 1, 2]));
                putb(hum, 0x595, rng.pick(&[0, 0, 1]));
                for o in [0x14, 0x18, 0x1C] {
                    put(hum, o, fl_small(rng));
                }
                // (x87 picks the NaN with the larger significand when both operands are NaNs, SSE the
                // first one: velocities stay finite so a sum never has two NaN operands)
                for o in [0x5C8, 0x5CC, 0x5D0] {
                    let mut f = fl_small(rng);
                    if f32::from_bits(f).is_nan() {
                        f = 0x3F00_0000;
                    }
                    put(hum, o, f);
                }
                put(hum, 0x5E0, rigs[i]);
                put(rigs[i], 0, rng.mix(&[0, 5, 189, 190, 0xFFFF_FFFF, 12345], 400));
            }
            let n = 1 + rng.below(12) as usize;
            let mut order: Vec<usize> = (0..12).collect();
            for k in (1..12).rev() {
                let j = rng.below(k as u32 + 1) as usize;
                order.swap(k, j);
            }
            let ring: Vec<u32> = order[..n].iter().map(|&k| humans[k]).collect();
            for k in 0..n {
                let nxt = if k + 1 == n { if rng.one_in(8) { 0 } else { ring[0] } } else { ring[k + 1] };
                put(ring[k], 0xB8, nxt);
            }
            for &hum in &humans {
                if !ring.contains(&hum) {
                    put(hum, 0xB8, 0);
                }
            }
            wd(RING, if rng.one_in(10) { 0 } else { ring[0] });
            let c = rng.pick(&[0.0f64, 1.0, 0.5, 3.0]).to_bits();
            wd(0x00E5_BFA0, c as u32);
            wd(0x00E5_BFA4, (c >> 32) as u32);
            wd(0x00DD_0584, rng.pick(&[0.001f32, 0.5, 0.0, 1.0, 0.05]).to_bits());
            // dt
            match rng.below(8) {
                0 => 0,
                1 => 0x8000_0000,
                2 => 0x7FC0_0000,
                3 => 0x7F80_0001,
                _ => (rng.below(100) as f32 / 1000.0 + 0.001).to_bits(),
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn humans_movement_update_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &HUMANS_MOVE_BODY, &HUMANS_MOVE_BODY_CALLS, &HUMANS_MOVE_BODY_DISPS, callees(), &[], humans_movement_thunk as *const () as usize);
        let mut rng = Rng(0x0F0F_1234_5678_ABCD);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            let dt = unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [dt, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in HM_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, HM_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

// --- object_props_update body (pin) ---
const OBJECT_PROPS_BODY: [u8; 537] = [
    0x83, 0xEC, 0x14, 0x83, 0x3D, 0x1C, 0x96, 0x52, 0x02, 0x00, 0x56, 0x57, 0x7E, 0x15, 0xB8, 0x48,
    0x07, 0xBE, 0x02, 0xE8, 0x78, 0xEE, 0xFF, 0xFF, 0x85, 0xC0, 0x74, 0x07, 0x83, 0x2D, 0x1C, 0x96,
    0x52, 0x02, 0x01, 0x8B, 0x35, 0x98, 0xD7, 0x15, 0x02, 0x85, 0xF6, 0x0F, 0x84, 0xE2, 0x01, 0x00,
    0x00, 0xBF, 0x28, 0x00, 0x00, 0x00, 0xEB, 0x08, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x90,
    0xF6, 0x86, 0x20, 0x01, 0x00, 0x00, 0x20, 0x0F, 0x85, 0xB0, 0x01, 0x00, 0x00, 0xF6, 0x86, 0x80,
    0x00, 0x00, 0x00, 0x0B, 0x0F, 0x85, 0xA3, 0x01, 0x00, 0x00, 0x83, 0xBE, 0x10, 0x01, 0x00, 0x00,
    0x00, 0x74, 0x4A, 0x8B, 0x06, 0x8B, 0x10, 0x6A, 0xFF, 0x8D, 0x4C, 0x24, 0x14, 0x51, 0x8B, 0xCE,
    0xFF, 0xD2, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0xC1,
    0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1, 0xD9, 0x5C,
    0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xDC, 0x1D, 0xA0, 0xC1, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4,
    0x05, 0x7A, 0x0A, 0xC7, 0x86, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x83, 0xBE, 0x14,
    0x01, 0x00, 0x00, 0x00, 0x0F, 0x8C, 0xCD, 0x00, 0x00, 0x00, 0x8B, 0x86, 0x14, 0x01, 0x00, 0x00,
    0x85, 0xC0, 0x0F, 0x8C, 0xBF, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC1,
    0x7E, 0x0F, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8C, 0xA8, 0x00, 0x00, 0x00, 0xEB,
    0x0E, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8D, 0x98, 0x00, 0x00, 0x00, 0x8B,
    0x0D, 0x08, 0xE9, 0x15, 0x02, 0x85, 0xC9, 0x74, 0x4E, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0xF7, 0x81, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x74, 0x29, 0x8B, 0x81, 0x74, 0x03,
    0x00, 0x00, 0x85, 0xC0, 0x74, 0x1F, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2,
    0x04, 0x8B, 0x92, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xD2, 0x74, 0x0A, 0x39, 0x42, 0x44, 0x75, 0x05,
    0x3B, 0x46, 0x44, 0x74, 0x48, 0x8B, 0x89, 0xB8, 0x00, 0x00, 0x00, 0x3B, 0x0D, 0x08, 0xE9, 0x15,
    0x02, 0x74, 0x04, 0x85, 0xC9, 0x75, 0xB9, 0x8B, 0x46, 0x7C, 0x8D, 0x04, 0x40, 0x8B, 0x14, 0xC5,
    0xC0, 0x25, 0x8F, 0x02, 0x39, 0x7A, 0x2C, 0x74, 0x24, 0x89, 0x7A, 0x2C, 0x83, 0x3D, 0xAC, 0x7D,
    0x52, 0x02, 0x00, 0x76, 0x07, 0xE8, 0xB6, 0xFD, 0xDB, 0xFF, 0xEB, 0x11, 0x8B, 0x0D, 0x94, 0x7D,
    0x52, 0x02, 0x6A, 0x01, 0x52, 0x51, 0x33, 0xC0, 0xE8, 0xA3, 0x72, 0x42, 0x00, 0xC7, 0x86, 0x14,
    0x01, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xF6, 0x86, 0x20, 0x01, 0x00, 0x00, 0x30, 0x75, 0x6D,
    0x8B, 0x86, 0x0C, 0x01, 0x00, 0x00, 0x8B, 0xD0, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0xFF, 0x00, 0x00,
    0xC1, 0xEA, 0x10, 0x8B, 0x14, 0x95, 0x3C, 0x5B, 0x22, 0x02, 0x8D, 0x0C, 0x89, 0x8D, 0x0C, 0xCA,
    0x85, 0xC9, 0x74, 0x11, 0x8B, 0x49, 0x04, 0x83, 0xF9, 0x04, 0x74, 0x09, 0x83, 0xF9, 0x03, 0x74,
    0x04, 0x85, 0xC9, 0x75, 0x38, 0x8B, 0xC8, 0x25, 0xFF, 0xFF, 0x00, 0x00, 0xC1, 0xE9, 0x10, 0x8D,
    0x14, 0x80, 0x8B, 0x04, 0x8D, 0x3C, 0x5B, 0x22, 0x02, 0x8D, 0x04, 0xD0, 0x85, 0xC0, 0x74, 0x12,
    0x83, 0x78, 0x04, 0x04, 0x0F, 0x94, 0xC0, 0x84, 0xC0, 0x74, 0x07, 0xE8, 0x50, 0xF2, 0xFF, 0xFF,
    0xEB, 0x0B, 0x8B, 0x16, 0x8B, 0x42, 0x10, 0x6A, 0x00, 0x8B, 0xCE, 0xFF, 0xD0, 0x8B, 0xB6, 0xB4,
    0x00, 0x00, 0x00, 0x3B, 0x35, 0x98, 0xD7, 0x15, 0x02, 0x74, 0x08, 0x85, 0xF6, 0x0F, 0x85, 0x2D,
    0xFE, 0xFF, 0xFF, 0x5F, 0x5E, 0x83, 0xC4, 0x14, 0xC3,
];
#[allow(dead_code)]
const OBJECT_PROPS_BODY_CALLS: [usize; 4] = [0x13, 0x165, 0x178, 0x1eb];
#[allow(dead_code)]
const OBJECT_PROPS_BODY_DISPS: [usize; 15] = [0x5, 0xf, 0x1e, 0x25, 0x98, 0xca, 0xf1, 0x123, 0x13d, 0x150, 0x15e, 0x16e, 0x1a6, 0x1d5, 0x205];

// =================================================================================================
// 0x935820  object_props_update  (537 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c. First ticks down the counter at [0x252961C] when 0x9346B0 (EAX = 0x2BE0748)
// reports a hit. Then walks the object_prop list at [0x215D798] (links +0xB4; items with
// +0x120 bits 0x20 or +0x80 bits 0xB are skipped): the velocity-squared test clears +0x110; when
// the despawn time at +0x114 is due, the physics body's id (+0x2C) is set to 0x28 and queued
// (0x6F5740 or 0xD5CC40) unless a player's held handle (human +0x374, flag +0xC0 bit 0x40000)
// equals the prop's handle; then props whose record (handle +0x10C, 40-byte records of the table
// at [0x2225B3C + hi * 4]) has kind 0, 3, 4 (or no record) either get 0x934C60 (kind 4) or are
// destroyed through vtable slot 4.
// Check: per-branch counters plus the offline difftest.

const OP_N: usize = 14;
static OP_HITS: [AtomicU32; OP_N] = [const { AtomicU32::new(0) }; OP_N];
const OP_NAMES: [&str; OP_N] = [
    "calls", "counter_hit", "props", "skip", "vel_clear", "due", "held", "body_set",
    "queue_a", "queue_b", "rec_pass", "c934c60", "destroy", "unused",
];

fn op(i: usize) {
    OP_HITS[i].fetch_add(1, Ordering::Relaxed);
}

kxs0!(object_props_thunk, object_props_update);

extern "C" fn object_props_update() {
    unsafe {
        op(0);
        const HEAD: usize = 0x0215_D798;
        const CNT: usize = 0x0252_961C;
        if (rd(CNT) as i32) > 0 {
            let r = call_regs(0x0093_46B0, ga(0x02BE_0748), 0, 0, 0, 0, &[]);
            if r != 0 {
                op(1);
                wd(CNT, rd(CNT).wrapping_sub(1));
            }
        }
        let mut esi = rd(HEAD) as usize;
        if esi == 0 {
            return;
        }
        let mut out = [0u32; 3];
        loop {
            op(2);
            'prop: {
                if rb(esi + 0x120) & 0x20 != 0 || rb(esi + 0x80) & 0xB != 0 {
                    op(3);
                    break 'prop;
                }
                if rd(esi + 0x110) != 0 {
                    vcall(esi, 0, false, &[out.as_mut_ptr() as u32, 0xFFFF_FFFF]);
                    let (x, y, z) = (f32::from_bits(out[0]) as f64, f32::from_bits(out[1]) as f64, f32::from_bits(out[2]) as f64);
                    let len2 = (((y * y) + (x * x)) + z * z) as f32;
                    if (len2 as f64) < rq(0x00E5_C1A0) {
                        op(4);
                        wd(esi + 0x110, 0);
                    }
                }
                let t = rd(esi + 0x114) as i32;
                if t >= 0 && time_due(t, rd(0x00E8_43AC) as i32) {
                    op(5);
                    let mut held = false;
                    let mut ecx = rd(0x0215_E908) as usize;
                    if ecx != 0 {
                        loop {
                            if rd(ecx + 0xC0) & 0x4_0000 != 0 {
                                let h = rd(ecx + 0x374);
                                if h != 0 && hv(h) != 0 && h == rd(esi + 0x44) {
                                    held = true;
                                    break;
                                }
                            }
                            ecx = rd(ecx + 0xB8) as usize;
                            if ecx == rd(0x0215_E908) as usize || ecx == 0 {
                                break;
                            }
                        }
                    }
                    if held {
                        op(6);
                    } else {
                        let idx = rd(esi + 0x7C);
                        let edx = rd(0x028F_25C0usize.wrapping_add((idx.wrapping_mul(3) as usize).wrapping_mul(8))) as usize;
                        if rd(edx + 0x2C) != 0x28 {
                            op(7);
                            wd(edx + 0x2C, 0x28);
                            if rd(0x0252_7DAC) != 0 {
                                op(8);
                                call_regs(0x006F_5740, 0, 0, edx as u32, 0, 0, &[]);
                            } else {
                                op(9);
                                call_regs(0x00D5_CC40, 0, 0, 0, 0, 0, &[rd(0x0252_7D94), edx as u32, 1]);
                            }
                        }
                    }
                    wd(esi + 0x114, 0xFFFF_FFFF);
                }
                if rb(esi + 0x120) & 0x30 != 0 {
                    op(3);
                    break 'prop;
                }
                let h = rd(esi + 0x10C);
                let rec = (rd(0x0222_5B3C + (h >> 16) as usize * 4) as usize).wrapping_add((h & 0xFFFF) as usize * 40);
                if rec != 0 {
                    let kind = rd(rec + 4);
                    if kind != 4 && kind != 3 && kind != 0 {
                        break 'prop;
                    }
                }
                op(10);
                if rec != 0 && rd(rec + 4) == 4 {
                    op(11);
                    call_regs(0x0093_4C60, 0, 0, 0, esi as u32, 0, &[]);
                } else {
                    op(12);
                    vcall(esi, 4, true, &[0]);
                }
            }
            esi = rd(esi + 0xB4) as usize;
            if esi == rd(HEAD) as usize || esi == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod opu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0093_5820;
    const CB0: usize = 0xCB00_0020;
    const CB4: usize = 0xCB00_0024;
    const HEAD: usize = 0x0215_D798;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x9346b0, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x6f5740, 0, 0, EDX, Ret::Any, Fx::None),
            callee(0xd5cc40, 3, 12, EAX, Ret::Any, Fx::None),
            callee(0x934c60, 0, 0, ESI, Ret::Any, Fx::None),
            callee(CB0, 2, 8, ECX, Ret::Any, Fx::WriteVecArg0),
            callee(CB4, 1, 4, ECX, Ret::Any, Fx::None),
        ]
    }

    fn fl_small(rng: &mut Rng) -> u32 {
        match rng.below(6) {
            0 => 0,
            1 => 1.0f32.to_bits(),
            2 => (rng.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
            3 => 0.003f32.to_bits(),
            _ => (rng.below(1000) as f32 / 100.0 - 5.0).to_bits(),
        }
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            h.mem.zero(HANDLE_TABLE, 0x1_0000);
            let mut a = Arena::new();
            let props = a.pool(8, 0x200);
            let humans = a.pool(5, 0x400);
            let bodies = a.pool(8, 0x40);
            let tables = a.pool(4, 40 * 8);
            let vt = a.pool(1, 0x40)[0];
            set_pools(vec![]);
            put(vt, 0, redirect(CB0) as u32);
            put(vt, 0x10, redirect(CB4) as u32);
            for k in 0..8 {
                put(bodies[k], 0x2C, rng.pick(&[0x28, 0, 5, 0x28]));
                wd(0x028F_25C0 + k * 24, bodies[k]);
            }
            for t in 0..4 {
                wd(0x0222_5B3C + t * 4, tables[t]);
                for r in 0..8 {
                    put(tables[t] + 40 * r as u32, 4, rng.pick(&[0, 3, 4, 1, 5, 4, 0]));
                }
            }
            let mut hs = Vec::new();
            for &hum in &humans {
                let slot = 1 + rng.below(4000);
                let hnd = (1 + rng.below(0xFF)) << 12 | slot;
                hs.push(hnd);
                wd(HANDLE_TABLE + slot as usize * 16, hum);
                put(hum, 0x44, hnd);
                put(hum, 0xC0, rng.pick(&[0x4_0000, 0, 0x4_0000, 0x4_0001]));
                put(hum, 0x374, rng.pick(&[0, 1, 2]));
            }
            let mut prop_hs = Vec::new();
            for &p in &props {
                let slot = 4001 + rng.below(90);
                let hnd = (1 + rng.below(0xFF)) << 12 | slot;
                prop_hs.push(hnd);
                if !rng.one_in(5) {
                    wd(HANDLE_TABLE + slot as usize * 16, p);
                }
                put(p, 0x44, hnd);
            }
            for &hum in &humans {
                let v = match rng.below(4) {
                    0 => 0,
                    1 => hs[rng.below(5) as usize],
                    _ => prop_hs[rng.below(8) as usize],
                };
                put(hum, 0x374, v);
            }
            let now = rng.mix(&[0, 100, 5000, 0x6B49_D200 - 3000, 0x1000_0000], 0x6B49_D200);
            wd(0x00E8_43AC, now);
            wd(0x0252_961C, rng.pick(&[0, 1, 5, 0xFFFF_FFFF, 3]));
            wd(0x0252_7DAC, rng.pick(&[0, 3]));
            wd(0x0252_7D94, rng.u32());
            let c = 0.0001f64.to_bits();
            wd(0x00E5_C1A0, c as u32);
            wd(0x00E5_C1A4, (c >> 32) as u32);
            for (i, &p) in props.iter().enumerate() {
                put(p, 0, vt);
                put(p, 0x7C, rng.below(8));
                put(p, 0x80, rng.pick(&[0, 0, 0, 0xB, 2, 0, 1]));
                put(p, 0x110, rng.pick(&[0, 1, 5]));
                put(p, 0x114, rng.mix(&[0, 0xFFFF_FFFF, 5, now, now.wrapping_add(5), now.wrapping_sub(5), now.wrapping_add(0x35A4_E900), now.wrapping_sub(0x35A4_E900)], 0x6B49_D200));
                put(p, 0x10C, if rng.one_in(8) { 0 } else { rng.below(4) << 16 | rng.below(8) });
                putb(p, 0x120, rng.pick(&[0, 0, 0x20, 0x10, 0x30, 0, 0x40]));
                let _ = i;
            }
            let n = 1 + rng.below(8) as usize;
            for k in 0..n {
                let nxt = if k + 1 == n { if rng.one_in(8) { 0 } else { props[0] } } else { props[k + 1] };
                put(props[k], 0xB4, nxt);
            }
            for k in n..8 {
                put(props[k], 0xB4, 0);
            }
            wd(HEAD, props[0]);
            // the human ring
            for k in 0..5 {
                put(humans[k], 0xB8, if k == 4 { if rng.one_in(6) { 0 } else { humans[0] } } else { humans[k + 1] });
            }
            wd(0x0215_E908, if rng.one_in(6) { 0 } else { humans[0] });
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn object_props_update_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &OBJECT_PROPS_BODY, &OBJECT_PROPS_BODY_CALLS, &OBJECT_PROPS_BODY_DISPS, callees(), &[(HANDLE_TABLE, 0x1_0000), (0x028F_25C0, 0x100), (0x0222_5B3C, 0x20)], object_props_thunk as *const () as usize);
        let mut rng = Rng(0x7777_1234_5678_ABCD);
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
        for (i, n) in OP_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, OP_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

// --- shops_do_frame body (pin) ---
const SHOPS_FRAME_BODY: [u8; 288] = [
    0xA0, 0x48, 0x8B, 0x52, 0x02, 0x83, 0xEC, 0x08, 0x3C, 0x01, 0x55, 0x56, 0x57, 0x75, 0x0E, 0x8A,
    0x0D, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x0F, 0x84, 0xFC, 0x00, 0x00, 0x00, 0xA1, 0x60, 0x3C,
    0x7B, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x1F, 0x8B, 0x88, 0x90, 0x00,
    0x00, 0x00, 0x8B, 0x11, 0x8B, 0x42, 0x04, 0xFF, 0xD0, 0x83, 0xF8, 0x07, 0x0F, 0x84, 0xD7, 0x00,
    0x00, 0x00, 0x83, 0xF8, 0x0D, 0x0F, 0x84, 0xCE, 0x00, 0x00, 0x00, 0xE8, 0xE0, 0xEC, 0xFF, 0xFF,
    0x33, 0xED, 0x39, 0x2D, 0xA4, 0x2A, 0x1F, 0x02, 0x7E, 0x37, 0x33, 0xFF, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x0D, 0xA8, 0x2A, 0x1F, 0x02, 0xF6, 0x44, 0x0F, 0x24, 0x04, 0x8D, 0x34, 0x0F, 0x74, 0x09,
    0x8D, 0x56, 0x28, 0x52, 0xE8, 0xE7, 0x19, 0x00, 0x00, 0x8B, 0xC6, 0xE8, 0x30, 0xED, 0xFF, 0xFF,
    0x83, 0xC5, 0x01, 0x81, 0xC7, 0xA0, 0x00, 0x00, 0x00, 0x3B, 0x2D, 0xA4, 0x2A, 0x1F, 0x02, 0x7C,
    0xCF, 0xE8, 0xCA, 0xEF, 0xFF, 0xFF, 0x80, 0x3D, 0x2C, 0xE7, 0x40, 0x01, 0x13, 0x72, 0x6A, 0x80,
    0x3D, 0xE3, 0x97, 0x52, 0x02, 0x00, 0x75, 0x58, 0x33, 0xED, 0x39, 0x2D, 0xA4, 0x2A, 0x1F, 0x02,
    0xC6, 0x05, 0xE3, 0x97, 0x52, 0x02, 0x01, 0x7E, 0x47, 0xA1, 0xA8, 0x2A, 0x1F, 0x02, 0x33, 0xFF,
    0xF6, 0x44, 0x07, 0x26, 0x01, 0x74, 0x28, 0x8B, 0x44, 0x07, 0x10, 0x8B, 0x48, 0x18, 0x89, 0x4C,
    0x24, 0x10, 0xDB, 0x44, 0x24, 0x10, 0x51, 0x8B, 0xF4, 0x51, 0xD9, 0x1C, 0x24, 0xE8, 0x7E, 0xD1,
    0xAB, 0xFF, 0xE8, 0x69, 0x38, 0xB5, 0xFF, 0xA1, 0xA8, 0x2A, 0x1F, 0x02, 0x83, 0xC4, 0x04, 0x83,
    0xC5, 0x01, 0x81, 0xC7, 0xA0, 0x00, 0x00, 0x00, 0x3B, 0x2D, 0xA4, 0x2A, 0x1F, 0x02, 0x7C, 0xC0,
    0x80, 0x3D, 0x2C, 0xE7, 0x40, 0x01, 0x13, 0x73, 0x10, 0x80, 0x3D, 0xE3, 0x97, 0x52, 0x02, 0x00,
    0x74, 0x07, 0xC6, 0x05, 0xE3, 0x97, 0x52, 0x02, 0x00, 0x5F, 0x5E, 0x5D, 0x83, 0xC4, 0x08, 0xC3,
];
#[allow(dead_code)]
const SHOPS_FRAME_BODY_CALLS: [usize; 6] = [0x4b, 0x74, 0x7b, 0x91, 0xdd, 0xe2];
#[allow(dead_code)]
const SHOPS_FRAME_BODY_DISPS: [usize; 16] = [0x1, 0x11, 0x1e, 0x54, 0x62, 0x8b, 0x98, 0xa1, 0xac, 0xb2, 0xba, 0xe8, 0xfa, 0x102, 0x10b, 0x114];

// =================================================================================================
// 0x9F5420  shops_do_frame  (288 bytes, no arguments)
// =================================================================================================
//
// sim_frame:1c. Does nothing when [0x2528B48] == 1 with [0x2528B49] == 0, or while the activity
// at [0x27B3C60] (state +0x3C == 1) is of kind 7 or 13 (vtable slot 1 of its +0x90 object).
// Otherwise: 0x9F4150, then per shop (0xA0 bytes at
// [0x21F2AA8], count [0x21F2AA4]) 0x9F6E80 (shop+0x28, when +0x24 has bit 4) and 0x9F41D0, then
// 0x9F4480; when the byte at 0x140E72C is >= 0x13 the one-shot flag 0x25297E3 is raised and every
// shop with +0x26 bit 1 gets its owner's int (+0x18 of [shop+0x10]) pushed through 0x4B2680
// (which returns on the x87 stack) into 0x548D70 (ESI = the int's stack slot); below 0x13 the flag
// is lowered again. The 4B2680 / 548D70 pair stays in one asm block so ST0 travels between them.
// Check: per-branch counters plus the offline difftest.

const SD_N: usize = 10;
static SD_HITS: [AtomicU32; SD_N] = [const { AtomicU32::new(0) }; SD_N];
const SD_NAMES: [&str; SD_N] = ["calls", "idle", "activity_skip", "shops", "c9f6e80", "late", "flag_set", "chain", "flag_clear", "unused"];

fn sd(i: usize) {
    SD_HITS[i].fetch_add(1, Ordering::Relaxed);
}

/// `push ecx; mov esi, esp; push ecx; fstp [esp]; call 4B2680; call 548D70; add esp, 4` with `v`
/// the int and `f` its float bits: 4B2680 returns the clamped value on ST0 for 548D70.
#[inline(never)]
unsafe fn shop_chain(v: u32, f: u32) {
    #[cfg(test)]
    let (a, b) = (difftest::redirect(0x004B_2680), difftest::redirect(0x0054_8D70));
    #[cfg(not(test))]
    let (a, b) = (0x004B_2680usize, 0x0054_8D70usize);
    std::arch::asm!(
        "push esi",
        "push {b}",
        "push eax",
        "mov esi, esp",
        "push edx",
        "call ecx",
        "call dword ptr [esp + 4]",
        "add esp, 8",
        "pop esi",
        b = in(reg) b,
        in("eax") v,
        in("edx") f,
        in("ecx") a,
        clobber_abi("C"),
    );
}

kxs0!(shops_frame_thunk, shops_do_frame);

extern "C" fn shops_do_frame() {
    unsafe {
        sd(0);
        const COUNT: usize = 0x021F_2AA4;
        const SHOPS: usize = 0x021F_2AA8;
        const ONESHOT: usize = 0x0252_97E3;
        const LEVEL: usize = 0x0140_E72C;
        if rb(0x0252_8B48) == 1 && rb(0x0252_8B49) == 0 {
            sd(1);
            return;
        }
        let act = rd(0x027B_3C60) as usize;
        if act != 0 && rd(act + 0x3C) == 1 {
            let obj = rd(act + 0x90) as usize;
            let vt = rd(obj) as usize;
            let f = rd(vt + 4) as usize;
            let kind = call_regs(f, f as u32, obj as u32, vt as u32, 0, 0, &[]);
            if kind == 7 || kind == 0xD {
                sd(2);
                return;
            }
        }
        call_regs(0x009F_4150, 0, 0, 0, 0, 0, &[]);
        let mut i = 0i32;
        while i < rd(COUNT) as i32 {
            sd(3);
            let esi = rd(SHOPS) as usize + i as usize * 0xA0;
            if rb(esi + 0x24) & 4 != 0 {
                sd(4);
                call_regs(0x009F_6E80, 0, 0, 0, 0, 0, &[(esi + 0x28) as u32]);
            }
            call_regs(0x009F_41D0, esi as u32, 0, 0, 0, 0, &[]);
            i += 1;
        }
        call_regs(0x009F_4480, 0, 0, 0, 0, 0, &[]);
        if rb(LEVEL) >= 0x13 {
            sd(5);
            if rb(ONESHOT) == 0 {
                sd(6);
                wb(ONESHOT, 1);
                let mut eax = rd(SHOPS) as usize;
                let mut i = 0i32;
                while i < rd(COUNT) as i32 {
                    let edi = i as usize * 0xA0;
                    if rb(edi + eax + 0x26) & 1 != 0 {
                        sd(7);
                        let o = rd(edi + eax + 0x10) as usize;
                        let v = rd(o + 0x18);
                        shop_chain(v, (v as i32 as f32).to_bits());
                        eax = rd(SHOPS) as usize;
                    }
                    i += 1;
                }
            }
            if rb(LEVEL) >= 0x13 {
                return;
            }
        }
        if rb(ONESHOT) != 0 {
            sd(8);
            wb(ONESHOT, 0);
        }
    }
}

#[cfg(test)]
mod sdf_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x009F_5420;
    const CB1: usize = 0xCB00_0031;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x9f4150, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x9f6e80, 1, 4, 0, Ret::Any, Fx::None),
            callee(0x9f41d0, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x9f4480, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x4b2680, 1, 4, 0, Ret::St0, Fx::None),
            callee(0x548d70, 0, 0, ESI, Ret::Any, Fx::LogEsi1),
            callee(CB1, 0, 0, ECX, Ret::Ptr(0), Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let shops = a.pool(6, 0xA0);
            let owners = a.pool(6, 0x40);
            let act = a.pool(1, 0x100)[0];
            let obj = a.pool(1, 0x40)[0];
            let vt = a.pool(1, 0x40)[0];
            set_pools(vec![vec![7, 0xD, 1, 5, 7, 0xD]]);
            put(vt, 4, redirect(CB1) as u32);
            put(obj, 0, vt);
            put(act, 0x3C, rng.pick(&[1, 1, 0, 2]));
            put(act, 0x90, obj);
            wd(0x027B_3C60, if rng.one_in(4) { 0 } else { act });
            for (i, &s) in shops.iter().enumerate() {
                putb(s, 0x24, rng.u32() as u8);
                putb(s, 0x26, rng.u32() as u8);
                put(s, 0x10, owners[i]);
                put(owners[i], 0x18, rng.mix(&[0, 5, 0xFFFF_FFFF, 0x7FFF_FFFF, 0x8000_0000, 100], 0));
            }
            wd(0x021F_2AA8, shops[0]);
            wd(0x021F_2AA4, rng.pick(&[0, 1, 2, 3, 6, 0xFFFF_FFFF, 4]));
            wb(0x0252_8B48, rng.pick(&[0, 1, 1, 2]));
            wb(0x0252_8B49, rng.pick(&[0, 0, 1]));
            wb(0x0140_E72C, rng.pick(&[0, 0x12, 0x13, 0x14, 0xFF, 0x13]));
            wb(0x0252_97E3, rng.pick(&[0, 1]));
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn shops_do_frame_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &SHOPS_FRAME_BODY, &SHOPS_FRAME_BODY_CALLS, &SHOPS_FRAME_BODY_DISPS, callees(), &[], shops_frame_thunk as *const () as usize);
        let mut rng = Rng(0x3131_4242_5353_6464);
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

// --- tutorial_do_frame body (pin) ---
const TUTORIAL_FRAME_BODY: [u8; 272] = [
    0x53, 0x56, 0x57, 0xE8, 0xC8, 0xFA, 0xFF, 0xFF, 0x84, 0xC0, 0x0F, 0x84, 0xDC, 0x00, 0x00, 0x00,
    0x8B, 0x35, 0xDC, 0x1A, 0x45, 0x01, 0x8B, 0x46, 0x0C, 0x83, 0xE8, 0x02, 0x74, 0x16, 0x83, 0xE8,
    0x02, 0x0F, 0x85, 0xE5, 0x00, 0x00, 0x00, 0x56, 0xE8, 0xF3, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0x5F, 0x5E, 0x5B, 0xC3, 0x8B, 0xC6, 0xE8, 0xA5, 0xF8, 0xFF, 0xFF, 0x8B, 0x7E, 0x14, 0x85, 0xFF,
    0x8A, 0xD8, 0x0F, 0x8E, 0x96, 0x00, 0x00, 0x00, 0x84, 0xDB, 0x74, 0x16, 0xD9, 0x05, 0x80, 0x43,
    0xE8, 0x00, 0xDC, 0x0D, 0xD8, 0xC0, 0xE5, 0x00, 0xE8, 0xC3, 0x90, 0x6E, 0x00, 0x2B, 0xF8, 0x89,
    0x7E, 0x14, 0x83, 0x7E, 0x14, 0x00, 0x7E, 0x6F, 0x8B, 0x0E, 0x3B, 0xCE, 0x8D, 0x46, 0x04, 0x75,
    0x1A, 0xC7, 0x06, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x89, 0x36, 0x89,
    0x30, 0x89, 0x35, 0xDC, 0x1A, 0x45, 0x01, 0x5F, 0x5E, 0x5B, 0xC3, 0x8B, 0x10, 0x85, 0xD2, 0x89,
    0x0D, 0xDC, 0x1A, 0x45, 0x01, 0x74, 0x04, 0x8B, 0xF9, 0x89, 0x3A, 0x8B, 0x16, 0x85, 0xD2, 0x74,
    0x05, 0x8B, 0x38, 0x89, 0x7A, 0x04, 0x85, 0xC9, 0xC7, 0x06, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x75, 0x0E, 0x89, 0x36, 0x89, 0x30, 0x89, 0x35, 0xDC, 0x1A, 0x45, 0x01,
    0x5F, 0x5E, 0x5B, 0xC3, 0x8B, 0x51, 0x04, 0x89, 0x10, 0x89, 0x0E, 0x8B, 0x41, 0x04, 0x89, 0x30,
    0x89, 0x71, 0x04, 0x5F, 0x5E, 0x5B, 0xC3, 0xC7, 0x46, 0x14, 0x00, 0x00, 0x00, 0x00, 0x84, 0xDB,
    0x74, 0x2A, 0x8B, 0xC6, 0x5F, 0x5E, 0x5B, 0xE9, 0xA4, 0xF8, 0xFF, 0xFF, 0xA1, 0xDC, 0x1A, 0x45,
    0x01, 0x85, 0xC0, 0x74, 0x17, 0x83, 0x78, 0x0C, 0x03, 0x75, 0x11, 0xBE, 0x13, 0x00, 0x00, 0x00,
    0xC7, 0x40, 0x0C, 0x02, 0x00, 0x00, 0x00, 0xE8, 0xE4, 0x27, 0xFE, 0xFF, 0x5F, 0x5E, 0x5B, 0xC3,
];
#[allow(dead_code)]
const TUTORIAL_FRAME_BODY_CALLS: [usize; 6] = [0x3, 0x28, 0x36, 0x58, 0xe7, 0x107];
#[allow(dead_code)]
const TUTORIAL_FRAME_BODY_DISPS: [usize; 7] = [0x12, 0x4e, 0x54, 0x83, 0x91, 0xbc, 0xed];

// =================================================================================================
// 0x6B7020  tutorial_do_frame  (272 bytes, no arguments, ends in a tail jump to 0x6B69B0)
// =================================================================================================
//
// sim_frame:1,2,3. The active tutorial message is the node at [0x1451ADC] of a circular list
// (links +0, +4; state +0xC; remaining time +0x14). When 0x6B6AF0 says tutorials are off, a node in
// state 3 is moved to state 2 and 0x699910 (ESI = 0x13) runs. Otherwise state 4 closes the node
// (0x6B6A40) and state 2 counts the time down by [0xE84380] * [0xE5C0D8] (float to int through
// 0xDA0140, called with the product on ST0) while 0x6B6900 (EAX = node) says so; when time is left
// the node is rotated to the back of the list, otherwise it is closed through the tail call
// 0x6B69B0 (EAX = node).
// Check: per-branch counters plus the offline difftest.

const TD_N: usize = 10;
static TD_HITS: [AtomicU32; TD_N] = [const { AtomicU32::new(0) }; TD_N];
const TD_NAMES: [&str; TD_N] = ["calls", "off", "off_state3", "state4", "state2", "tick", "rotate_single", "rotate", "close", "unused"];

fn td(i: usize) {
    TD_HITS[i].fetch_add(1, Ordering::Relaxed);
}

/// `fld dword [0xE84380]; fmul qword [0xE5C0D8]; call 0xDA0140` (the float to int conversion
/// takes ST0): returns its EAX.
#[inline(never)]
unsafe fn tutorial_ftol(ecx: u32, edx: u32) -> (u32, u32, u32) {
    #[cfg(test)]
    let (f, a, b) = (difftest::redirect(0x00DA_0140), tr(0x00E8_4380), tr(0x00E5_C0D8));
    #[cfg(not(test))]
    let (f, a, b) = (0x00DA_0140usize, 0x00E8_4380usize, 0x00E5_C0D8usize);
    let (r, c, d): (u32, u32, u32);
    std::arch::asm!(
        "fld dword ptr [{a}]",
        "fmul qword ptr [{b}]",
        "call {f}",
        a = in(reg) a,
        b = in(reg) b,
        f = in(reg) f,
        inout("ecx") ecx => c,
        inout("edx") edx => d,
        out("eax") r,
        clobber_abi("C"),
    );
    (r, c, d)
}

kz_thunk0!(tutorial_frame_thunk, tutorial_do_frame);

extern "C" fn tutorial_do_frame(out: *mut [u32; 7]) -> u32 {
    unsafe {
        td(0);
        const CUR: usize = 0x0145_1ADC;
        let [e_c, e_d, e_a, e_b, e_s, e_i, e_p] = *out;
        let (mut eax, mut ecx, mut edx) = call_xr(0x006B_6AF0, e_a, e_c, e_d, e_b, e_s, e_i, e_p, &[]);
        macro_rules! finish {
            () => {{
                (*out)[0] = ecx;
                (*out)[1] = edx;
                return eax;
            }};
        }
        if eax as u8 == 0 {
            td(1);
            eax = rd(CUR);
            if eax == 0 || rd(eax as usize + 0xC) != 3 {
                finish!();
            }
            td(2);
            wd(eax as usize + 0xC, 2);
            (eax, ecx, edx) = call_xr(0x0069_9910, eax, ecx, edx, e_b, 0x13, e_i, e_p, &[]);
            finish!();
        }
        let esi = rd(CUR) as usize;
        eax = rd(esi + 0xC);
        if eax != 2 {
            eax = eax.wrapping_sub(4);
            if eax != 0 {
                finish!();
            }
            td(3);
            (eax, ecx, edx) = call_xr(0x006B_6A40, 0, ecx, edx, e_b, esi as u32, e_i, e_p, &[esi as u32]);
            finish!();
        }
        td(4);
        (eax, ecx, edx) = call_xr(0x006B_6900, esi as u32, ecx, edx, e_b, esi as u32, e_i, e_p, &[]);
        let bl = eax as u8;
        let mut edi = rd(esi + 0x14);
        if (edi as i32) > 0 {
            if bl != 0 {
                td(5);
                (eax, ecx, edx) = tutorial_ftol(ecx, edx);
                edi = edi.wrapping_sub(eax);
                wd(esi + 0x14, edi);
            }
            if (rd(esi + 0x14) as i32) > 0 {
                // time left: rotate the node to the back of the list
                ecx = rd(esi);
                eax = (esi + 4) as u32;
                if ecx as usize == esi {
                    td(6);
                    wd(esi, 0);
                    wd(eax as usize, 0);
                    wd(esi, esi as u32);
                    wd(eax as usize, esi as u32);
                    wd(CUR, esi as u32);
                    finish!();
                }
                td(7);
                edx = rd(eax as usize);
                wd(CUR, ecx);
                if edx != 0 {
                    wd(edx as usize, ecx);
                }
                edx = rd(esi);
                if edx != 0 {
                    wd(edx as usize + 4, rd(eax as usize));
                }
                wd(esi, 0);
                wd(eax as usize, 0);
                if ecx == 0 {
                    wd(esi, esi as u32);
                    wd(eax as usize, esi as u32);
                    wd(CUR, esi as u32);
                    finish!();
                }
                edx = rd(ecx as usize + 4);
                wd(eax as usize, edx);
                wd(esi, ecx);
                eax = rd(ecx as usize + 4);
                wd(eax as usize, esi as u32);
                wd(ecx as usize + 4, esi as u32);
                finish!();
            }
            wd(esi + 0x14, 0);
        }
        if bl == 0 {
            finish!();
        }
        td(8);
        (eax, ecx, edx) = call_xr(0x006B_69B0, esi as u32, ecx, edx, e_b, e_s, e_i, e_p, &[]);
        finish!();
    }
}


#[cfg(test)]
mod tdf_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x006B_7020;
    const CUR: usize = 0x0145_1ADC;

    /// `_ftol`: records the double on ST0 and answers with a random int.
    extern "C" fn ftol_log(v: f64) -> u32 {
        let b = v.to_bits();
        push_log(vec![0xDA0140, b as u32, (b >> 32) as u32]);
        match rand_u32() % 5 {
            0 => 0,
            1 => rand_u32() % 10,
            2 => rand_u32() % 100000,
            3 => 0x8000_0000,
            _ => rand_u32(),
        }
    }
    #[unsafe(naked)]
    extern "C" fn ftol_native() {
        std::arch::naked_asm!("sub esp, 8", "fstp qword ptr [esp]", "call {f}", "add esp, 8", "ret", f = sym ftol_log)
    }

    fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0x6b6af0, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x6b6a40, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x6b6900, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xda0140, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x699910, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x6b69b0, 0, 0, EAX, Ret::Any, Fx::None),
        ];
        v[3].native = ftol_native as *const () as usize;
        v[3].silent = true;
        v
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let nodes = a.pool(6, 0x40);
            set_pools(vec![]);
            // a circular list over a random subset (or a lone node)
            let n = 1 + rng.below(6) as usize;
            for k in 0..n {
                let nx = if n == 1 { if rng.one_in(2) { nodes[0] } else { 0 } } else { nodes[(k + 1) % n] };
                let pv = if n == 1 { if rng.one_in(2) { nodes[0] } else { 0 } } else { nodes[(k + n - 1) % n] };
                put(nodes[k], 0, nx);
                put(nodes[k], 4, pv);
            }
            for &nd in &nodes {
                put(nd, 0xC, rng.pick(&[2, 2, 2, 3, 4, 1, 0]));
                put(nd, 0x14, rng.mix(&[0xFFFF_FFFB, 0, 3, 100, 1000, 0x7FFF_FFFF, 0x8000_0000], 50));
            }
            wd(CUR, nodes[rng.below(n as u32) as usize]);
            wd(0x00E8_4380, rng.pick(&[0.0166f32, 0.05, 1.0, 1000.0, 0.0, 3.0e9]).to_bits());
            let c = rng.pick(&[1000.0f64, 0.5, 0.0, 1.0e6]).to_bits();
            wd(0x00E5_C0D8, c as u32);
            wd(0x00E5_C0DC, (c >> 32) as u32);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn tutorial_do_frame_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &TUTORIAL_FRAME_BODY, &TUTORIAL_FRAME_BODY_CALLS, &TUTORIAL_FRAME_BODY_DISPS, callees(), &[], tutorial_frame_thunk as *const () as usize);
        let mut rng = Rng(0x5151_6262_7373_8484);
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
        for (i, n) in TD_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, TD_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}
