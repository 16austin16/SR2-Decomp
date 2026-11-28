//! Port module: the vint property-accessor trampolines (`vint_prop_get_* / vint_prop_set_*`, 156
//! of them), checked against the disassembly.
//!
//! Every one is the same 29- or 32-byte function, differing only in the virtual-method slot:
//!
//! ```text
//!   mov ecx,[esp+4]; test ecx,ecx; jne L; xor al,al; ret          ; obj == NULL -> AL = 0
//! L: mov edx,[esp+0xc]; mov eax,[ecx]; mov eax,[eax+SLOT]         ; SLOT as disp8 (29 bytes) or disp32 (32)
//!   push edx; mov edx,[esp+0xc]; push edx; call eax; ret
//! ```
//!
//! i.e. `fn(obj, a, b)`: a NULL object returns with only AL cleared (the rest of EAX is the
//! caller's); otherwise `obj->vtbl[SLOT/4]` is called with ECX = obj, EDX = a, EAX = the method,
//! stack (a, b) (the method pops them with `ret 8`), and its EAX is returned. The caller pops the
//! three arguments. The callee gets the caller's EBX / ESI / EDI / EBP unchanged.
//!
//! The thunk is `pushad` + XMM0-7 saved: every register and XMM register is back as the caller
//! left it except EAX, a superset of what the original leaves (it leaves the method's ECX / EDX;
//! no caller can depend on those, the method being unknown to the compiler).
//!
//! Check: these call game methods with side effects (the setters write), so no shadow copy of
//! the original runs. The summary counts the two branches (NULL object / method call). The
//! offline test (see the bottom) runs the original bytes next to the port on random registers.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::log;

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
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

static NULL_OBJ: AtomicU32 = AtomicU32::new(0);
static METHOD_CALLS: AtomicU32 = AtomicU32::new(0);

/// The seven general registers as the original's straight-line asm holds them. `call` runs the
/// method with exactly these registers and leaves in the struct what the method left. The method
/// may pop any number of its own arguments (or be garbage that returns anyway): the frame is
/// found again by a marker word above the arguments. The callee sees `args` (at most 12 words)
/// and zeros above them; the caller passes the original's own stack words after (a, b), so a
/// method that reads past its arguments sees what it would see in the original.
#[derive(Clone, Copy)]
struct St {
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

#[inline(never)]
unsafe fn call_regs_st(f: u32, st: &mut St, args: &[u32]) {
    let mut a8 = [0u32; 12];
    a8[..args.len()].copy_from_slice(args);
    let block: [u32; 3] = [f, a8.as_ptr() as u32, st as *mut St as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push dword ptr [eax + 8]",
            "mov ebx, eax",
            "push dword ptr [ebx]",
            "push 0x5352324D",
            "mov esi, [ebx + 4]",
            "mov ecx, 12",
            "2:",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jnz 2b",
            "mov ebx, [ebx + 8]",
            "mov eax, [ebx]", "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]",
            "mov esi, [ebx + 16]", "mov edi, [ebx + 20]", "mov ebp, [ebx + 24]", "mov ebx, [ebx + 12]",
            "call dword ptr [esp + 52]",
            // keep the method's registers, then find the marker the method's `ret N` left above
            "push eax", "push ecx", "push edx", "push ebx", "push esi", "push edi", "push ebp",
            "lea eax, [esp + 28]",
            "mov ecx, 16",
            "3:",
            "cmp dword ptr [eax], 0x5352324D",
            "je 4f",
            "add eax, 4",
            "dec ecx",
            "jnz 3b",
            "xor eax, eax", "mov eax, [eax]",
            "4:",
            "mov edi, [eax + 8]",
            "mov ecx, [esp + 24]", "mov [edi], ecx",
            "mov ecx, [esp + 20]", "mov [edi + 4], ecx",
            "mov ecx, [esp + 16]", "mov [edi + 8], ecx",
            "mov ecx, [esp + 12]", "mov [edi + 12], ecx",
            "mov ecx, [esp + 8]", "mov [edi + 16], ecx",
            "mov ecx, [esp + 4]", "mov [edi + 20], ecx",
            "mov ecx, [esp]", "mov [edi + 24], ecx",
            "lea esp, [eax + 12]",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
}

extern "C" fn wrap_entry(p: &mut Pushad, args: *const u32, slot: u32) {
    unsafe {
        let obj = *args;
        if obj == 0 {
            NULL_OBJ.fetch_add(1, Ordering::Relaxed);
            // the original leaves ECX = 0 (it loaded the object into ECX to test it) and clears AL
            p.eax &= !0xFF;
            p.ecx = 0;
            return;
        }
        METHOD_CALLS.fetch_add(1, Ordering::Relaxed);
        let a = *args.add(1);
        let b = *args.add(2);
        let f = rd(rd(obj as usize) as usize + slot as usize);
        // eax = the method, ecx = obj, edx = a, stack (a, b); EBX / ESI / EDI / EBP as the caller's
        let mut s = St { eax: f, ecx: obj, edx: a, ebx: p.ebx, esi: p.esi, edi: p.edi, ebp: p.ebp };
        // the words above (a, b): the original's own return address, object, a, b and what follows
        let w = |i: isize| *args.offset(i);
        call_regs_st(f, &mut s, &[a, b, w(-1), w(0), w(1), w(2), w(3), w(4), w(5), w(6), w(7), w(8)]);
        p.eax = s.eax;
        p.ecx = s.ecx;
        p.edx = s.edx;
        p.ebx = s.ebx;
        p.esi = s.esi;
        p.edi = s.edi;
        p.ebp = s.ebp;
    }
}

macro_rules! wrap_thunk {
    ($name:ident, $slot:expr) => {
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
                "push {slot}",
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
                "ret",
                f = sym wrap_entry,
                slot = const $slot,
            )
        }
    };
}

wrap_thunk!(t_b96430, 0x30);
wrap_thunk!(t_b96470, 0x34);
wrap_thunk!(t_b96500, 0x38);
wrap_thunk!(t_b96550, 0x3c);
wrap_thunk!(t_b96590, 0x40);
wrap_thunk!(t_b965e0, 0x44);
wrap_thunk!(t_b96620, 0x48);
wrap_thunk!(t_b96670, 0x4c);
wrap_thunk!(t_b966d0, 0x50);
wrap_thunk!(t_b96720, 0x54);
wrap_thunk!(t_b96780, 0x58);
wrap_thunk!(t_b967d0, 0x5c);
wrap_thunk!(t_b96840, 0x60);
wrap_thunk!(t_b968a0, 0x64);
wrap_thunk!(t_b96940, 0x68);
wrap_thunk!(t_b96980, 0x6c);
wrap_thunk!(t_b96a10, 0x78);
wrap_thunk!(t_b96a70, 0x7c);
wrap_thunk!(t_b96aa0, 0x80);
wrap_thunk!(t_b96b00, 0x84);
wrap_thunk!(t_b96b30, 0x70);
wrap_thunk!(t_b96bc0, 0x74);
wrap_thunk!(t_b96da0, 0x88);
wrap_thunk!(t_b96e00, 0x8c);
wrap_thunk!(t_b96e90, 0x90);
wrap_thunk!(t_b96ef0, 0x94);
wrap_thunk!(t_b96f50, 0x98);
wrap_thunk!(t_b96f90, 0x9c);
wrap_thunk!(t_b97010, 0xa0);
wrap_thunk!(t_b97070, 0xa4);
wrap_thunk!(t_b9c850, 0x70);
wrap_thunk!(t_b9c880, 0x74);
wrap_thunk!(t_b9c8b0, 0x80);
wrap_thunk!(t_b9c910, 0x84);
wrap_thunk!(t_b9c940, 0x78);
wrap_thunk!(t_b9c9a0, 0x7c);
wrap_thunk!(t_b9cef0, 0x30);
wrap_thunk!(t_b9cf90, 0x2c);
wrap_thunk!(t_b9cff0, 0x38);
wrap_thunk!(t_b9d140, 0x34);
wrap_thunk!(t_b9d190, 0x40);
wrap_thunk!(t_b9d2b0, 0x3c);
wrap_thunk!(t_b9f3a0, 0x2c);
wrap_thunk!(t_b9f3f0, 0x30);
wrap_thunk!(t_b9f440, 0x34);
wrap_thunk!(t_b9f490, 0x38);
wrap_thunk!(t_b9f4e0, 0x3c);
wrap_thunk!(t_b9f530, 0x40);
wrap_thunk!(t_b9f580, 0x44);
wrap_thunk!(t_b9f5c0, 0x48);
wrap_thunk!(t_b9f650, 0x54);
wrap_thunk!(t_b9f6b0, 0x58);
wrap_thunk!(t_b9f740, 0x4c);
wrap_thunk!(t_b9f7a0, 0x50);
wrap_thunk!(t_b9f820, 0x5c);
wrap_thunk!(t_b9f860, 0x60);
wrap_thunk!(t_b9f8b0, 0x64);
wrap_thunk!(t_b9f8f0, 0x68);
wrap_thunk!(t_b9f940, 0x6c);
wrap_thunk!(t_b9f980, 0x70);
wrap_thunk!(t_b9fa10, 0x74);
wrap_thunk!(t_b9fa50, 0x78);
wrap_thunk!(t_b9fae0, 0x7c);
wrap_thunk!(t_b9fb40, 0x80);
wrap_thunk!(t_b9fbe0, 0x88);
wrap_thunk!(t_b9fc20, 0x84);
wrap_thunk!(t_b9fc60, 0x90);
wrap_thunk!(t_b9fca0, 0x8c);
wrap_thunk!(t_b9fce0, 0x98);
wrap_thunk!(t_b9fd20, 0x94);
wrap_thunk!(t_b9fd60, 0x9c);
wrap_thunk!(t_b9fda0, 0xa0);
wrap_thunk!(t_b9fe30, 0xa4);
wrap_thunk!(t_b9fe70, 0xa8);
wrap_thunk!(t_ba0e40, 0xb8);
wrap_thunk!(t_ba0eb0, 0xbc);
wrap_thunk!(t_ba0fd0, 0xc0);
wrap_thunk!(t_ba1000, 0xc4);
wrap_thunk!(t_ba10e0, 0xc8);
wrap_thunk!(t_ba1140, 0xcc);
wrap_thunk!(t_ba1240, 0xd0);
wrap_thunk!(t_ba12a0, 0xd4);
wrap_thunk!(t_ba13a0, 0xdc);
wrap_thunk!(t_ba13e0, 0xd8);
wrap_thunk!(t_ba1c10, 0xe0);
wrap_thunk!(t_ba1c70, 0xe4);
wrap_thunk!(t_ba1ce0, 0xe8);
wrap_thunk!(t_ba1d40, 0xec);
wrap_thunk!(t_ba1db0, 0xf0);
wrap_thunk!(t_ba1e10, 0xf4);
wrap_thunk!(t_ba1e80, 0xf8);
wrap_thunk!(t_ba1ee0, 0xfc);
wrap_thunk!(t_ba1f50, 0x100);
wrap_thunk!(t_ba1fb0, 0x104);
wrap_thunk!(t_ba2050, 0x108);
wrap_thunk!(t_ba20b0, 0x10c);
wrap_thunk!(t_ba2150, 0x110);
wrap_thunk!(t_ba21b0, 0x114);
wrap_thunk!(t_ba2250, 0x118);
wrap_thunk!(t_ba22b0, 0x11c);
wrap_thunk!(t_ba2fd0, 0xc0);
wrap_thunk!(t_ba3010, 0xc4);
wrap_thunk!(t_ba3130, 0xc8);
wrap_thunk!(t_ba3170, 0xcc);
wrap_thunk!(t_ba31e0, 0xb8);
wrap_thunk!(t_ba3250, 0xbc);
wrap_thunk!(t_ba3350, 0xd0);
wrap_thunk!(t_ba33b0, 0xd4);
wrap_thunk!(t_ba3420, 0x70);
wrap_thunk!(t_ba3470, 0x74);
wrap_thunk!(t_ba3580, 0xd8);
wrap_thunk!(t_ba35d0, 0xdc);
wrap_thunk!(t_ba3620, 0xe0);
wrap_thunk!(t_ba3680, 0xe4);
wrap_thunk!(t_ba3710, 0xf0);
wrap_thunk!(t_ba3750, 0xf4);
wrap_thunk!(t_ba3800, 0xe8);
wrap_thunk!(t_ba3840, 0xec);
wrap_thunk!(t_ba38f0, 0xf8);
wrap_thunk!(t_ba3930, 0xfc);
wrap_thunk!(t_ba39e0, 0x128);
wrap_thunk!(t_ba3a40, 0x12c);
wrap_thunk!(t_ba3c80, 0x10c);
wrap_thunk!(t_ba3cd0, 0x108);
wrap_thunk!(t_ba3d20, 0x114);
wrap_thunk!(t_ba3e10, 0x110);
wrap_thunk!(t_ba3eb0, 0x11c);
wrap_thunk!(t_ba3fa0, 0x118);
wrap_thunk!(t_ba4040, 0x124);
wrap_thunk!(t_ba4130, 0x120);
wrap_thunk!(t_ba41d0, 0x104);
wrap_thunk!(t_ba4290, 0x100);
wrap_thunk!(t_ba4820, 0x70);
wrap_thunk!(t_ba4870, 0x74);
wrap_thunk!(t_ba4f50, 0xb8);
wrap_thunk!(t_ba4fb0, 0xbc);
wrap_thunk!(t_ba5010, 0xc0);
wrap_thunk!(t_ba5060, 0xc4);
wrap_thunk!(t_ba56f0, 0xb8);
wrap_thunk!(t_ba5760, 0xbc);
wrap_thunk!(t_ba5870, 0xc0);
wrap_thunk!(t_ba58d0, 0xc4);
wrap_thunk!(t_ba59d0, 0xc8);
wrap_thunk!(t_ba5a30, 0xcc);
wrap_thunk!(t_ba5b30, 0xd0);
wrap_thunk!(t_ba5b90, 0xd4);
wrap_thunk!(t_ba5c30, 0xd8);
wrap_thunk!(t_ba5c90, 0xdc);
wrap_thunk!(t_ba5d30, 0xe0);
wrap_thunk!(t_ba5d70, 0xe4);
wrap_thunk!(t_ba63e0, 0xb8);
wrap_thunk!(t_ba6440, 0xbc);
wrap_thunk!(t_ba64c0, 0xc0);
wrap_thunk!(t_ba6500, 0xc4);
wrap_thunk!(t_ba6b90, 0xb8);
wrap_thunk!(t_ba6bd0, 0xbc);

struct W {
    name: &'static str,
    addr: usize,
    slot: u32,
    size: usize,
    thunk: extern "C" fn(),
}

const COUNT: usize = 156;

static TABLE: [W; COUNT] = [
    W { name: "vint_prop_get_element_render_mode", addr: 0xb96430, slot: 0x30, size: 29, thunk: t_b96430 },
    W { name: "vint_prop_set_element_render_mode", addr: 0xb96470, slot: 0x34, size: 29, thunk: t_b96470 },
    W { name: "vint_prop_get_element_visible", addr: 0xb96500, slot: 0x38, size: 29, thunk: t_b96500 },
    W { name: "vint_prop_set_element_visible", addr: 0xb96550, slot: 0x3c, size: 29, thunk: t_b96550 },
    W { name: "vint_prop_get_element_mask", addr: 0xb96590, slot: 0x40, size: 29, thunk: t_b96590 },
    W { name: "vint_prop_set_element_mask", addr: 0xb965e0, slot: 0x44, size: 29, thunk: t_b965e0 },
    W { name: "vint_prop_get_element_anchor", addr: 0xb96620, slot: 0x48, size: 29, thunk: t_b96620 },
    W { name: "vint_prop_set_element_anchor", addr: 0xb96670, slot: 0x4c, size: 29, thunk: t_b96670 },
    W { name: "vint_prop_get_element_offset", addr: 0xb966d0, slot: 0x50, size: 29, thunk: t_b966d0 },
    W { name: "vint_prop_set_element_offset", addr: 0xb96720, slot: 0x54, size: 29, thunk: t_b96720 },
    W { name: "vint_prop_get_element_tint", addr: 0xb96780, slot: 0x58, size: 29, thunk: t_b96780 },
    W { name: "vint_prop_set_element_tint", addr: 0xb967d0, slot: 0x5c, size: 29, thunk: t_b967d0 },
    W { name: "vint_prop_get_element_alpha", addr: 0xb96840, slot: 0x60, size: 29, thunk: t_b96840 },
    W { name: "vint_prop_set_element_alpha", addr: 0xb968a0, slot: 0x64, size: 29, thunk: t_b968a0 },
    W { name: "vint_prop_get_element_depth", addr: 0xb96940, slot: 0x68, size: 29, thunk: t_b96940 },
    W { name: "vint_prop_set_element_depth", addr: 0xb96980, slot: 0x6c, size: 29, thunk: t_b96980 },
    W { name: "vint_prop_get_element_screen_nw", addr: 0xb96a10, slot: 0x78, size: 29, thunk: t_b96a10 },
    W { name: "vint_prop_set_element_screen_nw", addr: 0xb96a70, slot: 0x7c, size: 29, thunk: t_b96a70 },
    W { name: "vint_prop_get_element_screen_se", addr: 0xb96aa0, slot: 0x80, size: 32, thunk: t_b96aa0 },
    W { name: "vint_prop_set_element_screen_se", addr: 0xb96b00, slot: 0x84, size: 32, thunk: t_b96b00 },
    W { name: "vint_prop_get_element_screen_size", addr: 0xb96b30, slot: 0x70, size: 29, thunk: t_b96b30 },
    W { name: "vint_prop_set_element_screen_size", addr: 0xb96bc0, slot: 0x74, size: 29, thunk: t_b96bc0 },
    W { name: "vint_prop_get_element_rotation", addr: 0xb96da0, slot: 0x88, size: 32, thunk: t_b96da0 },
    W { name: "vint_prop_set_element_rotation", addr: 0xb96e00, slot: 0x8c, size: 32, thunk: t_b96e00 },
    W { name: "vint_prop_get_element_scale", addr: 0xb96e90, slot: 0x90, size: 32, thunk: t_b96e90 },
    W { name: "vint_prop_set_element_scale", addr: 0xb96ef0, slot: 0x94, size: 32, thunk: t_b96ef0 },
    W { name: "vint_prop_get_element_auto_offset", addr: 0xb96f50, slot: 0x98, size: 32, thunk: t_b96f50 },
    W { name: "vint_prop_set_element_auto_offset", addr: 0xb96f90, slot: 0x9c, size: 32, thunk: t_b96f90 },
    W { name: "vint_prop_get_element_unscaled_size", addr: 0xb97010, slot: 0xa0, size: 32, thunk: t_b97010 },
    W { name: "vint_prop_set_element_unscaled_size", addr: 0xb97070, slot: 0xa4, size: 32, thunk: t_b97070 },
    W { name: "vint_prop_get_group_screen_size", addr: 0xb9c850, slot: 0x70, size: 29, thunk: t_b9c850 },
    W { name: "vint_prop_set_group_screen_size", addr: 0xb9c880, slot: 0x74, size: 29, thunk: t_b9c880 },
    W { name: "vint_prop_get_group_screen_se", addr: 0xb9c8b0, slot: 0x80, size: 32, thunk: t_b9c8b0 },
    W { name: "vint_prop_set_group_screen_se", addr: 0xb9c910, slot: 0x84, size: 32, thunk: t_b9c910 },
    W { name: "vint_prop_get_group_screen_nw", addr: 0xb9c940, slot: 0x78, size: 29, thunk: t_b9c940 },
    W { name: "vint_prop_set_group_screen_nw", addr: 0xb9c9a0, slot: 0x7c, size: 29, thunk: t_b9c9a0 },
    W { name: "vint_prop_set_animation_start_time", addr: 0xb9cef0, slot: 0x30, size: 29, thunk: t_b9cef0 },
    W { name: "vint_prop_get_animation_start_time", addr: 0xb9cf90, slot: 0x2c, size: 29, thunk: t_b9cf90 },
    W { name: "vint_prop_set_animation_is_paused", addr: 0xb9cff0, slot: 0x38, size: 29, thunk: t_b9cff0 },
    W { name: "vint_prop_get_animation_is_paused", addr: 0xb9d140, slot: 0x34, size: 29, thunk: t_b9d140 },
    W { name: "vint_prop_set_animation_target_handle", addr: 0xb9d190, slot: 0x40, size: 29, thunk: t_b9d190 },
    W { name: "vint_prop_get_animation_target_handle", addr: 0xb9d2b0, slot: 0x3c, size: 29, thunk: t_b9d2b0 },
    W { name: "vint_prop_get_tween_target_handle", addr: 0xb9f3a0, slot: 0x2c, size: 29, thunk: t_b9f3a0 },
    W { name: "vint_prop_set_tween_target_handle", addr: 0xb9f3f0, slot: 0x30, size: 29, thunk: t_b9f3f0 },
    W { name: "vint_prop_get_tween_target_name", addr: 0xb9f440, slot: 0x34, size: 29, thunk: t_b9f440 },
    W { name: "vint_prop_set_tween_target_name", addr: 0xb9f490, slot: 0x38, size: 29, thunk: t_b9f490 },
    W { name: "vint_prop_get_tween_target_property", addr: 0xb9f4e0, slot: 0x3c, size: 29, thunk: t_b9f4e0 },
    W { name: "vint_prop_set_tween_target_property", addr: 0xb9f530, slot: 0x40, size: 29, thunk: t_b9f530 },
    W { name: "vint_prop_get_tween_state", addr: 0xb9f580, slot: 0x44, size: 29, thunk: t_b9f580 },
    W { name: "vint_prop_set_tween_state", addr: 0xb9f5c0, slot: 0x48, size: 29, thunk: t_b9f5c0 },
    W { name: "vint_prop_get_tween_duration", addr: 0xb9f650, slot: 0x54, size: 29, thunk: t_b9f650 },
    W { name: "vint_prop_set_tween_duration", addr: 0xb9f6b0, slot: 0x58, size: 29, thunk: t_b9f6b0 },
    W { name: "vint_prop_get_tween_start_time", addr: 0xb9f740, slot: 0x4c, size: 29, thunk: t_b9f740 },
    W { name: "vint_prop_set_tween_start_time", addr: 0xb9f7a0, slot: 0x50, size: 29, thunk: t_b9f7a0 },
    W { name: "vint_prop_get_tween_start_value", addr: 0xb9f820, slot: 0x5c, size: 29, thunk: t_b9f820 },
    W { name: "vint_prop_set_tween_start_value", addr: 0xb9f860, slot: 0x60, size: 29, thunk: t_b9f860 },
    W { name: "vint_prop_get_tween_end_value", addr: 0xb9f8b0, slot: 0x64, size: 29, thunk: t_b9f8b0 },
    W { name: "vint_prop_set_tween_end_value", addr: 0xb9f8f0, slot: 0x68, size: 29, thunk: t_b9f8f0 },
    W { name: "vint_prop_get_tween_loop_mode", addr: 0xb9f940, slot: 0x6c, size: 29, thunk: t_b9f940 },
    W { name: "vint_prop_set_tween_loop_mode", addr: 0xb9f980, slot: 0x70, size: 29, thunk: t_b9f980 },
    W { name: "vint_prop_get_tween_algorithm", addr: 0xb9fa10, slot: 0x74, size: 29, thunk: t_b9fa10 },
    W { name: "vint_prop_set_tween_algorithm", addr: 0xb9fa50, slot: 0x78, size: 29, thunk: t_b9fa50 },
    W { name: "vint_prop_get_tween_max_loops", addr: 0xb9fae0, slot: 0x7c, size: 29, thunk: t_b9fae0 },
    W { name: "vint_prop_set_tween_max_loops", addr: 0xb9fb40, slot: 0x80, size: 32, thunk: t_b9fb40 },
    W { name: "vint_prop_set_tween_start_event", addr: 0xb9fbe0, slot: 0x88, size: 32, thunk: t_b9fbe0 },
    W { name: "vint_prop_get_tween_start_event", addr: 0xb9fc20, slot: 0x84, size: 32, thunk: t_b9fc20 },
    W { name: "vint_prop_set_tween_end_event", addr: 0xb9fc60, slot: 0x90, size: 32, thunk: t_b9fc60 },
    W { name: "vint_prop_get_tween_end_event", addr: 0xb9fca0, slot: 0x8c, size: 32, thunk: t_b9fca0 },
    W { name: "vint_prop_set_tween_per_frame_event", addr: 0xb9fce0, slot: 0x98, size: 32, thunk: t_b9fce0 },
    W { name: "vint_prop_get_tween_per_frame_event", addr: 0xb9fd20, slot: 0x94, size: 32, thunk: t_b9fd20 },
    W { name: "vint_prop_get_tween_start_value_type", addr: 0xb9fd60, slot: 0x9c, size: 32, thunk: t_b9fd60 },
    W { name: "vint_prop_set_tween_start_value_type", addr: 0xb9fda0, slot: 0xa0, size: 32, thunk: t_b9fda0 },
    W { name: "vint_prop_get_tween_end_value_type", addr: 0xb9fe30, slot: 0xa4, size: 32, thunk: t_b9fe30 },
    W { name: "vint_prop_set_tween_end_value_type", addr: 0xb9fe70, slot: 0xa8, size: 32, thunk: t_b9fe70 },
    W { name: "vint_prop_get_bitmap_image", addr: 0xba0e40, slot: 0xb8, size: 32, thunk: t_ba0e40 },
    W { name: "vint_prop_set_bitmap_image", addr: 0xba0eb0, slot: 0xbc, size: 32, thunk: t_ba0eb0 },
    W { name: "vint_prop_get_bitmap_image_crc", addr: 0xba0fd0, slot: 0xc0, size: 32, thunk: t_ba0fd0 },
    W { name: "vint_prop_set_bitmap_image_crc", addr: 0xba1000, slot: 0xc4, size: 32, thunk: t_ba1000 },
    W { name: "vint_prop_get_bitmap_source_nw", addr: 0xba10e0, slot: 0xc8, size: 32, thunk: t_ba10e0 },
    W { name: "vint_prop_set_bitmap_source_nw", addr: 0xba1140, slot: 0xcc, size: 32, thunk: t_ba1140 },
    W { name: "vint_prop_get_bitmap_source_se", addr: 0xba1240, slot: 0xd0, size: 32, thunk: t_ba1240 },
    W { name: "vint_prop_set_bitmap_source_se", addr: 0xba12a0, slot: 0xd4, size: 32, thunk: t_ba12a0 },
    W { name: "vint_prop_set_bitmap_custom_source_coords", addr: 0xba13a0, slot: 0xdc, size: 32, thunk: t_ba13a0 },
    W { name: "vint_prop_get_bitmap_custom_source_coords", addr: 0xba13e0, slot: 0xd8, size: 32, thunk: t_ba13e0 },
    W { name: "vint_prop_get_gradient_gradient_nw", addr: 0xba1c10, slot: 0xe0, size: 32, thunk: t_ba1c10 },
    W { name: "vint_prop_set_gradient_gradient_nw", addr: 0xba1c70, slot: 0xe4, size: 32, thunk: t_ba1c70 },
    W { name: "vint_prop_get_gradient_gradient_ne", addr: 0xba1ce0, slot: 0xe8, size: 32, thunk: t_ba1ce0 },
    W { name: "vint_prop_set_gradient_gradient_ne", addr: 0xba1d40, slot: 0xec, size: 32, thunk: t_ba1d40 },
    W { name: "vint_prop_get_gradient_gradient_sw", addr: 0xba1db0, slot: 0xf0, size: 32, thunk: t_ba1db0 },
    W { name: "vint_prop_set_gradient_gradient_sw", addr: 0xba1e10, slot: 0xf4, size: 32, thunk: t_ba1e10 },
    W { name: "vint_prop_get_gradient_gradient_se", addr: 0xba1e80, slot: 0xf8, size: 32, thunk: t_ba1e80 },
    W { name: "vint_prop_set_gradient_gradient_se", addr: 0xba1ee0, slot: 0xfc, size: 32, thunk: t_ba1ee0 },
    W { name: "vint_prop_get_gradient_alpha_nw", addr: 0xba1f50, slot: 0x100, size: 32, thunk: t_ba1f50 },
    W { name: "vint_prop_set_gradient_alpha_nw", addr: 0xba1fb0, slot: 0x104, size: 32, thunk: t_ba1fb0 },
    W { name: "vint_prop_get_gradient_alpha_ne", addr: 0xba2050, slot: 0x108, size: 32, thunk: t_ba2050 },
    W { name: "vint_prop_set_gradient_alpha_ne", addr: 0xba20b0, slot: 0x10c, size: 32, thunk: t_ba20b0 },
    W { name: "vint_prop_get_gradient_alpha_sw", addr: 0xba2150, slot: 0x110, size: 32, thunk: t_ba2150 },
    W { name: "vint_prop_set_gradient_alpha_sw", addr: 0xba21b0, slot: 0x114, size: 32, thunk: t_ba21b0 },
    W { name: "vint_prop_get_gradient_alpha_se", addr: 0xba2250, slot: 0x118, size: 32, thunk: t_ba2250 },
    W { name: "vint_prop_set_gradient_alpha_se", addr: 0xba22b0, slot: 0x11c, size: 32, thunk: t_ba22b0 },
    W { name: "vint_prop_get_text_text_tag", addr: 0xba2fd0, slot: 0xc0, size: 32, thunk: t_ba2fd0 },
    W { name: "vint_prop_set_text_text_tag", addr: 0xba3010, slot: 0xc4, size: 32, thunk: t_ba3010 },
    W { name: "vint_prop_get_text_text_tag_crc", addr: 0xba3130, slot: 0xc8, size: 32, thunk: t_ba3130 },
    W { name: "vint_prop_set_text_text_tag_crc", addr: 0xba3170, slot: 0xcc, size: 32, thunk: t_ba3170 },
    W { name: "vint_prop_get_text_font", addr: 0xba31e0, slot: 0xb8, size: 32, thunk: t_ba31e0 },
    W { name: "vint_prop_set_text_font", addr: 0xba3250, slot: 0xbc, size: 32, thunk: t_ba3250 },
    W { name: "vint_prop_get_text_text_scale", addr: 0xba3350, slot: 0xd0, size: 32, thunk: t_ba3350 },
    W { name: "vint_prop_set_text_text_scale", addr: 0xba33b0, slot: 0xd4, size: 32, thunk: t_ba33b0 },
    W { name: "vint_prop_get_text_screen_size", addr: 0xba3420, slot: 0x70, size: 29, thunk: t_ba3420 },
    W { name: "vint_prop_set_text_screen_size", addr: 0xba3470, slot: 0x74, size: 29, thunk: t_ba3470 },
    W { name: "vint_prop_get_text_word_wrap", addr: 0xba3580, slot: 0xd8, size: 32, thunk: t_ba3580 },
    W { name: "vint_prop_set_text_word_wrap", addr: 0xba35d0, slot: 0xdc, size: 32, thunk: t_ba35d0 },
    W { name: "vint_prop_get_text_wrap_width", addr: 0xba3620, slot: 0xe0, size: 32, thunk: t_ba3620 },
    W { name: "vint_prop_set_text_wrap_width", addr: 0xba3680, slot: 0xe4, size: 32, thunk: t_ba3680 },
    W { name: "vint_prop_get_text_vert_align", addr: 0xba3710, slot: 0xf0, size: 32, thunk: t_ba3710 },
    W { name: "vint_prop_set_text_vert_align", addr: 0xba3750, slot: 0xf4, size: 32, thunk: t_ba3750 },
    W { name: "vint_prop_get_text_horz_align", addr: 0xba3800, slot: 0xe8, size: 32, thunk: t_ba3800 },
    W { name: "vint_prop_set_text_horz_align", addr: 0xba3840, slot: 0xec, size: 32, thunk: t_ba3840 },
    W { name: "vint_prop_get_text_force_case", addr: 0xba38f0, slot: 0xf8, size: 32, thunk: t_ba38f0 },
    W { name: "vint_prop_set_text_force_case", addr: 0xba3930, slot: 0xfc, size: 32, thunk: t_ba3930 },
    W { name: "vint_prop_get_text_insert_values", addr: 0xba39e0, slot: 0x128, size: 32, thunk: t_ba39e0 },
    W { name: "vint_prop_set_text_insert_values", addr: 0xba3a40, slot: 0x12c, size: 32, thunk: t_ba3a40 },
    W { name: "vint_prop_set_text_line_frame_enable", addr: 0xba3c80, slot: 0x10c, size: 32, thunk: t_ba3c80 },
    W { name: "vint_prop_get_text_line_frame_enable", addr: 0xba3cd0, slot: 0x108, size: 32, thunk: t_ba3cd0 },
    W { name: "vint_prop_set_text_line_frame_w", addr: 0xba3d20, slot: 0x114, size: 32, thunk: t_ba3d20 },
    W { name: "vint_prop_get_text_line_frame_w", addr: 0xba3e10, slot: 0x110, size: 32, thunk: t_ba3e10 },
    W { name: "vint_prop_set_text_line_frame_m", addr: 0xba3eb0, slot: 0x11c, size: 32, thunk: t_ba3eb0 },
    W { name: "vint_prop_get_text_line_frame_m", addr: 0xba3fa0, slot: 0x118, size: 32, thunk: t_ba3fa0 },
    W { name: "vint_prop_set_text_line_frame_e", addr: 0xba4040, slot: 0x124, size: 32, thunk: t_ba4040 },
    W { name: "vint_prop_get_text_line_frame_e", addr: 0xba4130, slot: 0x120, size: 32, thunk: t_ba4130 },
    W { name: "vint_prop_set_text_leading", addr: 0xba41d0, slot: 0x104, size: 32, thunk: t_ba41d0 },
    W { name: "vint_prop_get_text_leading", addr: 0xba4290, slot: 0x100, size: 32, thunk: t_ba4290 },
    W { name: "vint_prop_get_point_screen_size", addr: 0xba4820, slot: 0x70, size: 29, thunk: t_ba4820 },
    W { name: "vint_prop_set_point_screen_size", addr: 0xba4870, slot: 0x74, size: 29, thunk: t_ba4870 },
    W { name: "vint_prop_get_clip_clip_size", addr: 0xba4f50, slot: 0xb8, size: 32, thunk: t_ba4f50 },
    W { name: "vint_prop_set_clip_clip_size", addr: 0xba4fb0, slot: 0xbc, size: 32, thunk: t_ba4fb0 },
    W { name: "vint_prop_get_clip_clip_enabled", addr: 0xba5010, slot: 0xc0, size: 32, thunk: t_ba5010 },
    W { name: "vint_prop_set_clip_clip_enabled", addr: 0xba5060, slot: 0xc4, size: 32, thunk: t_ba5060 },
    W { name: "vint_prop_get_bitmap_circle_image", addr: 0xba56f0, slot: 0xb8, size: 32, thunk: t_ba56f0 },
    W { name: "vint_prop_set_bitmap_circle_image", addr: 0xba5760, slot: 0xbc, size: 32, thunk: t_ba5760 },
    W { name: "vint_prop_get_bitmap_circle_source_nw", addr: 0xba5870, slot: 0xc0, size: 32, thunk: t_ba5870 },
    W { name: "vint_prop_set_bitmap_circle_source_nw", addr: 0xba58d0, slot: 0xc4, size: 32, thunk: t_ba58d0 },
    W { name: "vint_prop_get_bitmap_circle_source_se", addr: 0xba59d0, slot: 0xc8, size: 32, thunk: t_ba59d0 },
    W { name: "vint_prop_set_bitmap_circle_source_se", addr: 0xba5a30, slot: 0xcc, size: 32, thunk: t_ba5a30 },
    W { name: "vint_prop_get_bitmap_circle_start_angle", addr: 0xba5b30, slot: 0xd0, size: 32, thunk: t_ba5b30 },
    W { name: "vint_prop_set_bitmap_circle_start_angle", addr: 0xba5b90, slot: 0xd4, size: 32, thunk: t_ba5b90 },
    W { name: "vint_prop_get_bitmap_circle_end_angle", addr: 0xba5c30, slot: 0xd8, size: 32, thunk: t_ba5c30 },
    W { name: "vint_prop_set_bitmap_circle_end_angle", addr: 0xba5c90, slot: 0xdc, size: 32, thunk: t_ba5c90 },
    W { name: "vint_prop_get_bitmap_circle_num_wedges", addr: 0xba5d30, slot: 0xe0, size: 32, thunk: t_ba5d30 },
    W { name: "vint_prop_set_bitmap_circle_num_wedges", addr: 0xba5d70, slot: 0xe4, size: 32, thunk: t_ba5d70 },
    W { name: "vint_prop_get_sr2_map_zoom", addr: 0xba63e0, slot: 0xb8, size: 32, thunk: t_ba63e0 },
    W { name: "vint_prop_set_sr2_map_zoom", addr: 0xba6440, slot: 0xbc, size: 32, thunk: t_ba6440 },
    W { name: "vint_prop_get_sr2_map_map_mode", addr: 0xba64c0, slot: 0xc0, size: 32, thunk: t_ba64c0 },
    W { name: "vint_prop_set_sr2_map_map_mode", addr: 0xba6500, slot: 0xc4, size: 32, thunk: t_ba6500 },
    W { name: "vint_prop_get_video_vid_id_handle", addr: 0xba6b90, slot: 0xb8, size: 32, thunk: t_ba6b90 },
    W { name: "vint_prop_set_video_vid_id_handle", addr: 0xba6bd0, slot: 0xbc, size: 32, thunk: t_ba6bd0 },
];

/// The original's bytes, rebuilt from the template (the install checks them against the exe, and
/// the offline test against the exe file).
fn body(slot: u32, size: usize) -> Vec<u8> {
    let mut v: Vec<u8> = vec![
        0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x54, 0x24, 0x0C, 0x8B, 0x01,
    ];
    if size == 29 {
        v.extend_from_slice(&[0x8B, 0x40, slot as u8]);
    } else {
        v.extend_from_slice(&[0x8B, 0x80]);
        v.extend_from_slice(&slot.to_le_bytes());
    }
    v.extend_from_slice(&[0x52, 0x8B, 0x54, 0x24, 0x0C, 0x52, 0xFF, 0xD0, 0xC3]);
    v
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut n = 0;
    for w in TABLE.iter() {
        let b = body(w.slot, w.size);
        if b.len() == w.size && replace_pinned(w.name, w.addr, &b, w.thunk as usize) {
            n += 1;
        }
    }
    log(&format!("port_small_vint: replaced {n}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let (n, c) = (NULL_OBJ.load(Ordering::Relaxed), METHOD_CALLS.load(Ordering::Relaxed));
    if n + c == 0 {
        return String::new();
    }
    format!("port_small_vint: prop accessors {} calls ({n} null object, {c} method calls)", n + c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    /// [0..6] the registers on entry (eax, ecx, edx, ebx, esi, edi, ebp), [7..8] stack args,
    /// [9] call count, [10] the value to return.
    static mut REC: [u32; 12] = [0; 12];

    #[unsafe(naked)]
    extern "C" fn rec_stub() {
        std::arch::naked_asm!(
            "mov [{r}], eax",
            "mov [{r} + 4], ecx",
            "mov [{r} + 8], edx",
            "mov [{r} + 12], ebx",
            "mov [{r} + 16], esi",
            "mov [{r} + 20], edi",
            "mov [{r} + 24], ebp",
            "mov ecx, [esp + 4]",
            "mov [{r} + 28], ecx",
            "mov ecx, [esp + 8]",
            "mov [{r} + 32], ecx",
            "add dword ptr [{r} + 36], 1",
            "mov eax, [{r} + 40]",
            "mov ecx, 0x11111111",
            "mov edx, 0x22222222",
            "ret 8",
            r = sym REC,
        )
    }

    #[repr(C)]
    struct TB {
        f: u32,
        ecx: u32,
        edx: u32,
        ebx: u32,
        esi: u32,
        edi: u32,
        ebp: u32,
        eax: u32,
        a1: u32,
        a2: u32,
        a3: u32,
        esp: u32,
        o_eax: u32,
        o_ebx: u32,
        o_esi: u32,
        o_edi: u32,
        o_ebp: u32,
    }

    /// Call `f` (a plain-`ret` function) with three stack arguments and the seven registers.
    unsafe fn call3(f: usize, r: [u32; 7], args: [u32; 3]) -> TB {
        let [eax, ecx, edx, ebx, esi, edi, ebp] = r;
        let mut b = TB { f: f as u32, ecx, edx, ebx, esi, edi, ebp, eax, a1: args[0], a2: args[1], a3: args[2], esp: 0, o_eax: 0, o_ebx: 0, o_esi: 0, o_edi: 0, o_ebp: 0 };
        unsafe {
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 44], esp",
                "push dword ptr [eax]",
                "push eax",
                "push dword ptr [eax + 40]",
                "push dword ptr [eax + 36]",
                "push dword ptr [eax + 32]",
                "mov ecx, [eax + 4]", "mov edx, [eax + 8]", "mov ebx, [eax + 12]",
                "mov esi, [eax + 16]", "mov edi, [eax + 20]", "mov ebp, [eax + 24]",
                "mov eax, [eax + 28]",
                "call dword ptr [esp + 16]",
                "mov ecx, [esp + 12]",
                "mov [ecx + 48], eax",
                "mov [ecx + 52], ebx",
                "mov [ecx + 56], esi",
                "mov [ecx + 60], edi",
                "mov [ecx + 64], ebp",
                "mov esp, [ecx + 44]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                inout("eax") &mut b as *mut TB => _,
                lateout("ecx") _,
                lateout("edx") _,
                clobber_abi("C"),
            );
        }
        b
    }

    struct Rng(u64);
    impl Rng {
        fn u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
    }

    fn exe() -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/SR2_pc.unpacked.exe");
        std::fs::read(p).expect("bin/SR2_pc.unpacked.exe")
    }

    fn file_off(exe: &[u8], va: usize) -> usize {
        let u32_at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
        let u16_at = |o: usize| u16::from_le_bytes(exe[o..o + 2].try_into().unwrap()) as usize;
        let pe = u32_at(0x3C);
        let nsec = u16_at(pe + 6);
        let opt = u16_at(pe + 20);
        for i in 0..nsec {
            let s = pe + 24 + opt + i * 40;
            let (vsz, va0, rsz, raw) = (u32_at(s + 8), u32_at(s + 12), u32_at(s + 16), u32_at(s + 20));
            if va >= va0 + 0x400000 && va < va0 + 0x400000 + vsz.max(rsz) {
                return raw + va - va0 - 0x400000;
            }
        }
        panic!("{va:#x} not in a section");
    }

    fn exec(code: &[u8]) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), code.len() + 16, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0);
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, code.len()) };
        p
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 and bin/SR2_pc.unpacked.exe"]
    fn prop_accessors_match_original() {
        let exe = exe();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut total = 0u32;
        for w in TABLE.iter() {
            let o = file_off(&exe, w.addr);
            let b = body(w.slot, w.size);
            assert_eq!(&exe[o..o + w.size], &b[..], "{}: template differs from the exe bytes", w.name);
            let orig = exec(&exe[o..o + w.size]);
            let ours = w.thunk as usize;
            // a fake object whose vtable has the stub at `slot` and poison everywhere else
            let mut vtbl = vec![0xDEAD_0000u32; 128];
            vtbl[w.slot as usize / 4] = rec_stub as *const () as usize as u32;
            let obj_mem = vec![vtbl.as_ptr() as u32, 0, 0, 0];
            for it in 0..1500u32 {
                let null = it % 6 == 0;
                let obj = if null { 0 } else { obj_mem.as_ptr() as u32 };
                let r: [u32; 7] = std::array::from_fn(|_| rng.u32());
                let args = [obj, rng.u32(), rng.u32()];
                let ret = rng.u32();
                let mut got = Vec::new();
                for f in [orig, ours] {
                    unsafe {
                        REC = [0; 12];
                        REC[10] = ret;
                        let t = call3(f, r, args);
                        got.push((t.o_eax, t.o_ebx, t.o_esi, t.o_edi, t.o_ebp, REC));
                    }
                }
                assert_eq!(got[0], got[1], "{} iter {it}: regs in {r:x?} args {args:x?}", w.name);
                if null {
                    assert_eq!(got[0].0, r[0] & !0xFF, "{}: null path eax", w.name);
                    assert_eq!(got[0].5[9], 0);
                } else {
                    assert_eq!(got[0].0, ret);
                    assert_eq!(got[0].5[9], 1);
                    assert_eq!(got[0].5[1], obj);
                    assert_eq!(got[0].5[2], args[1]);
                }
                total += 1;
            }
        }
        eprintln!("prop_accessors_match_original: {} functions, {total} cases, all matched", TABLE.len());
    }
}
