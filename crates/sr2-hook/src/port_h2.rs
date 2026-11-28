//! Port module h2: small functions (list_h2), ports pinned with replace_pinned.
//!
//! Every thunk saves XMM0-7 and pushad's; the Rust body edits the saved registers and the thunk
//! restores them. Callees are called by address through `St::call`, which hands them exactly the
//! registers and stack words the original sets up.

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 9;

const CAP: u32 = 1 << 24;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// The seven general registers as a straight-line asm function would hold them. `call` runs a game
/// function with exactly these registers (EBX / EBP and the callee's own stack arguments included)
/// and leaves in the struct what the callee left, so a port threads the original's register flow:
/// the original passes the caller's EBX / EBP to its callees, reads the ECX / EDX a callee leaves,
/// and sees a callee's changes to registers it never saved. ESP is restored from a TEB scratch slot
/// (`fs:[0x14]`, saved and restored around the call), so the callee may pop its own arguments or
/// not. The callee sees `args` (at most 8) and zero padding above them.
#[derive(Clone, Copy)]
pub(crate) struct St {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
}

impl St {
    #[inline(always)]
    fn of(r: &Pushad) -> St {
        St { eax: r.eax, ecx: r.ecx, edx: r.edx, ebx: r.ebx, esi: r.esi, edi: r.edi, ebp: r.ebp }
    }

    /// Call a game function by address (the offline test may redirect it); returns its EAX.
    #[inline(always)]
    unsafe fn call(&mut self, f: usize, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f, self, args) };
        self.eax
    }

}

#[inline(never)]
unsafe fn call_regs_st(f: usize, st: &mut St, args: &[u32]) {
    let mut a8 = [0u32; 8];
    a8[..args.len()].copy_from_slice(args);
    let block: [u32; 3] = [f as u32, a8.as_ptr() as u32, st as *mut St as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push dword ptr [eax + 8]",
            "push dword ptr fs:[0x14]",
            "mov ebx, eax",
            "mov fs:[0x14], esp",
            "push dword ptr [ebx]",
            "mov esi, [ebx + 4]",
            "mov ecx, 8",
            "2:",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jnz 2b",
            "mov ebx, [ebx + 8]",
            "mov eax, [ebx]", "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]",
            "mov esi, [ebx + 16]", "mov edi, [ebx + 20]", "mov ebp, [ebx + 24]", "mov ebx, [ebx + 12]",
            "call dword ptr [esp + 32]",
            "push edi",
            "mov edi, fs:[0x14]",
            "mov edi, [edi + 4]",
            "mov [edi], eax", "mov [edi + 4], ecx", "mov [edi + 8], edx", "mov [edi + 12], ebx",
            "mov [edi + 16], esi", "mov [edi + 24], ebp",
            "pop dword ptr [edi + 20]",
            "mov esp, fs:[0x14]",
            "pop dword ptr fs:[0x14]",
            "add esp, 4",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
}

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

/// `pushad`, XMM0-7 saved, `f(&mut block, &stack args)`, XMM0-7 restored, `popad`, `ret N`.
macro_rules! thunk {
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


const B_606920: [u8; 29] = [
    0x83, 0x3D, 0x68, 0x7B, 0xE9, 0x00, 0xFF, 0x74, 0x13, 0xE8, 0xA2, 0x12, 0x3D, 0x00, 0x85, 0xC0,
    0x75, 0x0A, 0xC7, 0x05, 0x68, 0x7B, 0xE9, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const B_619B50: [u8; 32] = [
    0x56, 0x8B, 0xF1, 0xE8, 0xF8, 0x8E, 0xFE, 0xFF, 0xD9, 0xEE, 0x33, 0xC9, 0xD9, 0x5E, 0x20, 0x83,
    0xC8, 0xFF, 0x88, 0x4E, 0x18, 0x89, 0x46, 0x1C, 0x89, 0x4E, 0x28, 0x89, 0x46, 0x2C, 0x5E, 0xC3,
];
const B_61E990: [u8; 33] = [
    0x56, 0x8B, 0xF1, 0xE8, 0xB8, 0x40, 0xFE, 0xFF, 0x56, 0xC7, 0x46, 0x04, 0x06, 0x00, 0x00, 0x00,
    0xE8, 0x1B, 0x00, 0x00, 0x00, 0xC7, 0x86, 0x1C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E,
    0xC3,
];
const B_784560: [u8; 48] = [
    0x53, 0x56, 0x57, 0x8B, 0xD9, 0xE8, 0x16, 0x1B, 0x1D, 0x00, 0x8D, 0xB3, 0x08, 0x02, 0x00, 0x00,
    0x8D, 0x7B, 0x2C, 0xB9, 0x0A, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x5F, 0x33, 0xC0, 0x5E, 0x88, 0x83,
    0x04, 0x02, 0x00, 0x00, 0xC6, 0x43, 0x54, 0x01, 0x89, 0x83, 0x00, 0x02, 0x00, 0x00, 0x5B, 0xC3,
];
const B_9545C0: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x54, 0x24, 0x04, 0x6A, 0x01, 0x50, 0x52, 0x51, 0xE8, 0xEE, 0xE6,
    0xFF, 0xFF, 0xC2, 0x08, 0x00,
];
const B_B96F70: [u8; 20] = [
    0x8B, 0x41, 0x6C, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x12, 0xC6, 0xFF, 0xFF, 0xB0, 0x01,
    0x5E, 0xC2, 0x08, 0x00,
];
const B_B9F5A0: [u8; 23] = [
    0x8B, 0x81, 0xA4, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0xDF, 0x3F, 0xFF,
    0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,
];
const B_B9FE50: [u8; 23] = [
    0x8B, 0x81, 0xE0, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x2F, 0x37, 0xFF,
    0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,
];
const B_C166C0: [u8; 13] = [
    0x8B, 0x51, 0x44, 0x8B, 0x41, 0x50, 0x2B, 0xC2, 0x0F, 0xAF, 0x41, 0x4C, 0xC3,
];

// --- ports ---------------------------------------------------------------------------------------

// class stunt_drifting_diversion_type::init (0x619B50): base init at 0x602A50 (ECX = this), then the
// fields are reset. The base call's EDX / EBX / EBP leftovers flow out as the original leaves them.
extern "C" fn p_619b50(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(0);
        let this = r.ecx;
        let ent_esi = r.esi;
        let mut s = St::of(r);
        s.esi = this;
        s.ecx = this;
        s.call(0x0060_2A50, &[]);
        let obj = s.esi as usize;
        wf(obj + 0x20, 0.0);
        wb(obj + 0x18, 0);
        wd(obj + 0x1c, 0xFFFF_FFFF);
        wd(obj + 0x28, 0);
        wd(obj + 0x2c, 0xFFFF_FFFF);
        r.eax = 0xFFFF_FFFF;
        r.ecx = 0;
        r.edx = s.edx;
        r.esi = ent_esi;
        r.edi = s.edi;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}
thunk!(t_619b50, p_619b50, "ret");

// class cell_spawn_group::reset (0x784560): base reset at 0x956080 (ECX = EBX = this), then 40 bytes
// copied from this+0x208 to this+0x2C and three fields cleared.
extern "C" fn p_784560(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(1);
        let this = r.ecx;
        let (ent_ebx, ent_esi, ent_edi) = (r.ebx, r.esi, r.edi);
        let mut s = St::of(r);
        s.ebx = this;
        s.ecx = this;
        s.call(0x0095_6080, &[]);
        let obj = s.ebx as usize;
        copy_dwords(obj + 0x2C, obj + 0x208, 10);
        wb(obj + 0x204, 0);
        wb(obj + 0x54, 1);
        wd(obj + 0x200, 0);
        r.eax = 0;
        r.ecx = 0;
        r.edx = s.edx;
        r.ebx = ent_ebx;
        r.esi = ent_esi;
        r.edi = ent_edi;
        r.ebp = s.ebp;
    }
}
thunk!(t_784560, p_784560, "ret");

// class survival_diversion_type_info::init (0x61E990): base init at 0x602A50, then the object's
// type field is set to 6, 0x61E9C0 is called with the object as its one stack argument, and the
// word at +0x31C is cleared.
extern "C" fn p_61e990(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(2);
        let this = r.ecx;
        let ent_esi = r.esi;
        let mut s = St::of(r);
        s.esi = this;
        s.ecx = this;
        s.call(0x0060_2A50, &[]);
        let obj = s.esi;
        wd(obj as usize + 4, 6);
        s.call(0x0061_E9C0, &[obj]);
        wd(s.esi as usize + 0x31C, 0);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.esi = ent_esi;
        r.edi = s.edi;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}
thunk!(t_61e990, p_61e990, "ret");

// class driveby_diversion_type_info::update (0x606920): once the global at 0xE97B68 is not -1, call
// 0x9D7BD0; when it returns 0 the global is set to -1.
extern "C" fn p_606920(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(3);
        if rd(0x00E9_7B68) != 0xFFFF_FFFF {
            let mut s = St::of(r);
            s.call(0x009D_7BD0, &[]);
            if s.eax == 0 {
                wd(0x00E9_7B68, 0xFFFF_FFFF);
            }
            r.eax = s.eax;
            r.ecx = s.ecx;
            r.edx = s.edx;
            r.esi = s.esi;
            r.edi = s.edi;
            r.ebx = s.ebx;
            r.ebp = s.ebp;
        }
    }
}
thunk!(t_606920, p_606920, "ret");

// vint_object_tween::get_state / get_end_value_type and vint_element_base::get_auto_offset: push the
// field at ECX + off, call 0x0B93590 with it, and return AL = 1 (the upper EAX bits are the callee's).
// ESI is the first stack argument while the callee runs.
#[inline(always)]
unsafe fn field_query(r: &mut Pushad, a: *const u32, off: usize) {
    unsafe {
        let v = rd(r.ecx as usize + off);
        let ent_esi = r.esi;
        let mut s = St::of(r);
        s.esi = rd(a as usize);
        s.eax = v;
        s.call(0x00B9_3590, &[v]);
        r.eax = (s.eax & !0xFF) | 1;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.esi = ent_esi;
        r.edi = s.edi;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}

extern "C" fn p_b9f5a0(r: &mut Pushad, a: *const u32) {
    count(4);
    unsafe { field_query(r, a, 0xA4) }
}
thunk!(t_b9f5a0, p_b9f5a0, "ret 8");

extern "C" fn p_b9fe50(r: &mut Pushad, a: *const u32) {
    count(5);
    unsafe { field_query(r, a, 0xE0) }
}
thunk!(t_b9fe50, p_b9fe50, "ret 8");

extern "C" fn p_b96f70(r: &mut Pushad, a: *const u32) {
    count(6);
    unsafe { field_query(r, a, 0x6C) }
}
thunk!(t_b96f70, p_b96f70, "ret 8");

// class searchlight::get_world_bbox (0x9545C0): forwards its two stack arguments to 0x952CC0 as
// (ECX, arg0, arg1, 1).
extern "C" fn p_9545c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let this = r.ecx;
        let mut s = St::of(r);
        let (a0, a1) = (rd(a as usize), rd(a as usize + 4));
        s.call(0x0095_2CC0, &[this, a0, a1, 1]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.esi = s.esi;
        r.edi = s.edi;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}
thunk!(t_9545c0, p_9545c0, "ret 8");

// split_page_mempool::bytes_free (0xC166C0): (field 0x50 - field 0x44) * field 0x4C; EDX = field 0x44.
extern "C" fn p_c166c0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(8);
        let this = r.ecx as usize;
        let edx = rd(this + 0x44);
        let eax = rd(this + 0x50).wrapping_sub(edx).wrapping_mul(rd(this + 0x4C));
        r.eax = eax;
        r.edx = edx;
    }
}
thunk!(t_c166c0, p_c166c0, "ret");


static PORTS: [(&str, usize, &[u8], extern "C" fn()); COUNT] = [
    ("stunt_drifting_diversion_type::init", 0x0061_9B50, &B_619B50, t_619b50),
    ("cell_spawn_group::reset", 0x0078_4560, &B_784560, t_784560),
    ("survival_diversion_type_info::init", 0x0061_E990, &B_61E990, t_61e990),
    ("driveby_diversion_type_info::update", 0x0060_6920, &B_606920, t_606920),
    ("vint_object_tween::get_state", 0x00B9_F5A0, &B_B9F5A0, t_b9f5a0),
    ("vint_object_tween::get_end_value_type", 0x00B9_FE50, &B_B9FE50, t_b9fe50),
    ("vint_element_base::get_auto_offset", 0x00B9_6F70, &B_B96F70, t_b96f70),
    ("searchlight::get_world_bbox", 0x0095_45C0, &B_9545C0, t_9545c0),
    ("split_page_mempool::bytes_free", 0x00C1_66C0, &B_C166C0, t_c166c0),
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for &(name, addr, body, thunk) in PORTS.iter() {
        if replace_pinned(name, addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h2: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .zip(CNT.iter())
        .filter_map(|(p, c)| {
            let n = c.load(Ordering::Relaxed);
            (n > 0).then(|| format!("{} {n}", p.0))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_h2: {}", parts.join(", "))
}
