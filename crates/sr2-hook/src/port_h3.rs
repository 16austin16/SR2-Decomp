//! Port module h3 (Haiku trial round): small functions from the h3 list.
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each port is a Rust
//! handler `h_XXXXXX(&mut Pushad, *const u32)` behind a `pushad` + XMM0-7 save thunk. The handler
//! edits the saved registers (`popad` restores them) and reads stack arguments through `a`.
//! Callees are called by address through `St::call`, so the leftover EAX / ECX / EDX of the callee
//! flow out as in the original.
//!
//! Ported (7): vint_element_group::init, vint_element_clip::init, vint_object_tween::get_loop_mode,
//! vint_element_text::get_leading, gfx_set_color_bytes, and two copies of the list
//! "take head, append to tail" body (vint_object_factory_imp<>::alloc_2 and alloc_9).

#![allow(unused_imports)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 7;
const CAP: u32 = 1 << 24;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
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

/// The seven general registers as straight-line asm would hold them. `call` runs a game function
/// with exactly these registers and leaves in the struct what the callee left (EBX / EBP / ESI /
/// EDI included), so a port threads the original's register flow.
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

impl St {
    #[inline(always)]
    fn of(r: &Pushad) -> St {
        St { eax: r.eax, ecx: r.ecx, edx: r.edx, ebx: r.ebx, esi: r.esi, edi: r.edi, ebp: r.ebp }
    }

    /// Write every register back.
    #[inline(always)]
    fn put(&self, r: &mut Pushad) {
        r.eax = self.eax;
        r.ecx = self.ecx;
        r.edx = self.edx;
        r.ebx = self.ebx;
        r.esi = self.esi;
        r.edi = self.edi;
        r.ebp = self.ebp;
    }

    /// Call a game function by address; returns its EAX.
    #[inline(always)]
    unsafe fn call(&mut self, f: usize, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f, self, args) };
        self.eax
    }
}

/// Registers in and out, and the callee's stack arguments (at most 8; zero padding above). ESP is
/// restored from a TEB scratch slot, so the callee may or may not pop its own arguments.
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

/// `fld dword` + `fstp dword` round trip (quiets a signalling NaN, as the x87 load does).
#[inline(always)]
fn f32q(v: f32) -> f32 {
    (v as f64) as f32
}

#[inline(always)]
fn lo(full: u32, b: u8) -> u32 {
    (full & !0xFF) | b as u32
}

// --- handlers ------------------------------------------------------------------------------------

// vint_element_group::init: call 0xB95A60; AL = (AL != 0); ret
#[inline(always)]
unsafe fn hb_b9c500(r: &mut Pushad) {
    unsafe {
        let mut s = St::of(r);
        s.call(0xB95A60, &[]);
        let nz = (s.eax as u8) != 0;
        s.eax = lo(s.eax, nz as u8);
        s.put(r);
    }
}

// vint_element_clip::init: call 0xB95A60; if AL != 0 store [0xDD071C] at [this+0x98] and
// [this+0x9C] and 1 at [this+0xA0]. The original reads ECX after the call, so the callee's leftover
// ECX is the base of those stores.
#[inline(always)]
unsafe fn hb_ba4d80(r: &mut Pushad) {
    unsafe {
        let mut s = St::of(r);
        s.call(0xB95A60, &[]);
        if (s.eax as u8) != 0 {
            let p = s.ecx as usize;
            let v = f32q(rf(0xDD071C));
            s.eax = lo(s.eax, 1);
            wf(p + 0x98, v);
            wf(p + 0x9C, v);
            wb(p + 0xA0, 1);
        }
        s.put(r);
    }
}

// Shared shape of vint_object_tween::get_loop_mode and vint_element_text::get_leading: a stdcall
// call 0xB93590(value) with ESI = the first stack argument; AL = 1 on return; ESI restored.
#[inline(always)]
unsafe fn hb_set_value(r: &mut Pushad, a: *const u32, value: u32) {
    unsafe {
        let esi = r.esi;
        let mut s = St::of(r);
        s.eax = value;
        s.esi = rd(a as usize);
        s.call(0xB93590, &[value]);
        s.eax = lo(s.eax, 1);
        s.esi = esi;
        s.put(r);
    }
}

// vint_object_tween::get_loop_mode: value = [this+0x98]
#[inline(always)]
unsafe fn hb_b9f960(r: &mut Pushad, a: *const u32) {
    unsafe {
        let v = rd(r.ecx as usize + 0x98);
        hb_set_value(r, a, v);
    }
}

// vint_element_text::get_leading: value = sign-extended byte [this+0xC5]
#[inline(always)]
unsafe fn hb_ba42b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let v = rb(r.ecx as usize + 0xC5) as i8 as i32 as u32;
        hb_set_value(r, a, v);
    }
}

// gfx_set_color_bytes(EAX = pointer to r, g, b, a bytes): gfx_set_color(cdecl r, g, b, a). When the
// call is made ECX holds the second byte and EDX the first (the original loads them last).
#[inline(always)]
unsafe fn hb_d14d00(r: &mut Pushad) {
    unsafe {
        let p = r.eax as usize;
        let (b0, b1, b2, b3) = (rb(p), rb(p + 1), rb(p + 2), rb(p + 3));
        let mut s = St::of(r);
        s.ecx = b1 as u32;
        s.edx = b0 as u32;
        s.call(0xD14840, &[b0 as u32, b1 as u32, b2 as u32, b3 as u32]);
        s.put(r);
    }
}

// Circular-list "take the head node and append it to the tail" (vint_object_factory_imp alloc_2 and
// alloc_9 share this body). ECX = list: +4 head, +8 tail, +0xC count. Node: +0x14 next, +0x18 prev.
// Empty list: EAX = 0 and nothing else changes. Otherwise EAX = the moved node.
#[inline(always)]
unsafe fn hb_list_head_to_tail(r: &mut Pushad) {
    unsafe {
        let this = r.ecx as usize;
        let node = rd(this + 4);
        if node == 0 {
            r.eax = 0;
            return;
        }
        let n = node as usize;
        let mut edx = rd(n + 0x14);
        if edx != node {
            wd(this + 4, edx);
            edx = rd(n + 0x18);
            let edi = rd(n + 0x14);
            wd(edx as usize + 0x14, edi);
            let edi = rd(n + 0x18);
            edx = rd(n + 0x14);
            wd(edx as usize + 0x18, edi);
            wd(n + 0x14, 0);
            wd(n + 0x18, 0);
        } else {
            wd(n + 0x14, 0);
            wd(n + 0x18, 0);
            wd(this + 4, 0);
        }
        let tail = rd(this + 8);
        if tail != 0 {
            edx = rd(tail as usize + 0x18);
            wd(n + 0x18, edx);
            edx = rd(this + 8);
            wd(n + 0x14, edx);
            edx = rd(this + 8);
            edx = rd(edx as usize + 0x18);
            wd(edx as usize + 0x14, node);
            edx = rd(this + 8);
            wd(edx as usize + 0x18, node);
            wd(this + 0xC, rd(this + 0xC).wrapping_add(1));
        } else {
            wd(n + 0x14, node);
            wd(n + 0x18, node);
            wd(this + 0xC, rd(this + 0xC).wrapping_add(1));
            wd(this + 8, node);
            edx = 0;
        }
        r.eax = node;
        r.edx = edx;
    }
}

extern "C" fn h_b9c500(r: &mut Pushad, _a: *const u32) {
    count(0);
    unsafe { hb_b9c500(r) }
}

extern "C" fn h_ba4d80(r: &mut Pushad, _a: *const u32) {
    count(1);
    unsafe { hb_ba4d80(r) }
}

extern "C" fn h_b9f960(r: &mut Pushad, a: *const u32) {
    count(2);
    unsafe { hb_b9f960(r, a) }
}

extern "C" fn h_ba42b0(r: &mut Pushad, a: *const u32) {
    count(3);
    unsafe { hb_ba42b0(r, a) }
}

extern "C" fn h_d14d00(r: &mut Pushad, _a: *const u32) {
    count(4);
    unsafe { hb_d14d00(r) }
}

extern "C" fn h_b9d490(r: &mut Pushad, _a: *const u32) {
    count(5);
    unsafe { hb_list_head_to_tail(r) }
}

extern "C" fn h_ba5f60(r: &mut Pushad, _a: *const u32) {
    count(6);
    unsafe { hb_list_head_to_tail(r) }
}

thunk!(t_b9c500, h_b9c500, "ret");
thunk!(t_ba4d80, h_ba4d80, "ret");
thunk!(t_b9f960, h_b9f960, "ret 8");
thunk!(t_ba42b0, h_ba42b0, "ret 8");
thunk!(t_d14d00, h_d14d00, "ret");
thunk!(t_b9d490, h_b9d490, "ret");
thunk!(t_ba5f60, h_ba5f60, "ret");

// --- pinned original bodies ----------------------------------------------------------------------

static B_B9C500: [u8; 11] = [
    0xE8, 0x5B, 0x95, 0xFF, 0xFF, 0x84, 0xC0, 0x0F, 0x95, 0xC0, 0xC3,
];
static B_BA4D80: [u8; 37] = [
    0xE8, 0xDB, 0x0C, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x01, 0xC3, 0xD9, 0x05, 0x1C, 0x07, 0xDD, 0x00,
    0xB0, 0x01, 0xD9, 0x91, 0x98, 0x00, 0x00, 0x00, 0xD9, 0x99, 0x9C, 0x00, 0x00, 0x00, 0x88, 0x81,
    0xA0, 0x00, 0x00, 0x00, 0xC3,
];
static B_B9F960: [u8; 23] = [
    0x8B, 0x81, 0x98, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x1F, 0x3C, 0xFF,
    0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,
];
static B_BA42B0: [u8; 24] = [
    0x0F, 0xBE, 0x81, 0xC5, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0xCE, 0xF2,
    0xFE, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,
];
static B_D14D00: [u8; 28] = [
    0x0F, 0xB6, 0x48, 0x03, 0x0F, 0xB6, 0x50, 0x02, 0x51, 0x0F, 0xB6, 0x48, 0x01, 0x52, 0x0F, 0xB6,
    0x10, 0x51, 0x52, 0xE8, 0x28, 0xFB, 0xFF, 0xFF, 0x83, 0xC4, 0x10, 0xC3,
];
static B_B9D490: [u8; 112] = [
    0x8B, 0x41, 0x04, 0x56, 0x33, 0xF6, 0x3B, 0xC6, 0x74, 0x64, 0x8B, 0x50, 0x14, 0x3B, 0xD0, 0x75,
    0x0B, 0x89, 0x70, 0x14, 0x89, 0x70, 0x18, 0x89, 0x71, 0x04, 0xEB, 0x1D, 0x89, 0x51, 0x04, 0x8B,
    0x50, 0x18, 0x57, 0x8B, 0x78, 0x14, 0x89, 0x7A, 0x14, 0x8B, 0x78, 0x18, 0x8B, 0x50, 0x14, 0x89,
    0x7A, 0x18, 0x89, 0x70, 0x14, 0x89, 0x70, 0x18, 0x5F, 0x8B, 0x51, 0x08, 0x3B, 0xD6, 0x75, 0x0F,
    0x89, 0x40, 0x14, 0x89, 0x40, 0x18, 0x83, 0x41, 0x0C, 0x01, 0x89, 0x41, 0x08, 0x5E, 0xC3, 0x8B,
    0x52, 0x18, 0x89, 0x50, 0x18, 0x8B, 0x51, 0x08, 0x89, 0x50, 0x14, 0x8B, 0x51, 0x08, 0x8B, 0x52,
    0x18, 0x89, 0x42, 0x14, 0x8B, 0x51, 0x08, 0x89, 0x42, 0x18, 0x83, 0x41, 0x0C, 0x01, 0x5E, 0xC3,
];
static B_BA5F60: [u8; 112] = [
    0x8B, 0x41, 0x04, 0x56, 0x33, 0xF6, 0x3B, 0xC6, 0x74, 0x64, 0x8B, 0x50, 0x14, 0x3B, 0xD0, 0x75,
    0x0B, 0x89, 0x70, 0x14, 0x89, 0x70, 0x18, 0x89, 0x71, 0x04, 0xEB, 0x1D, 0x89, 0x51, 0x04, 0x8B,
    0x50, 0x18, 0x57, 0x8B, 0x78, 0x14, 0x89, 0x7A, 0x14, 0x8B, 0x78, 0x18, 0x8B, 0x50, 0x14, 0x89,
    0x7A, 0x18, 0x89, 0x70, 0x14, 0x89, 0x70, 0x18, 0x5F, 0x8B, 0x51, 0x08, 0x3B, 0xD6, 0x75, 0x0F,
    0x89, 0x40, 0x14, 0x89, 0x40, 0x18, 0x83, 0x41, 0x0C, 0x01, 0x89, 0x41, 0x08, 0x5E, 0xC3, 0x8B,
    0x52, 0x18, 0x89, 0x50, 0x18, 0x8B, 0x51, 0x08, 0x89, 0x50, 0x14, 0x8B, 0x51, 0x08, 0x8B, 0x52,
    0x18, 0x89, 0x42, 0x14, 0x8B, 0x51, 0x08, 0x89, 0x42, 0x18, 0x83, 0x41, 0x0C, 0x01, 0x5E, 0xC3,
];

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "vint_element_group::init", addr: 0x00B9_C500, body: &B_B9C500, thunk: t_b9c500 },
    Port { name: "vint_element_clip::init", addr: 0x00BA_4D80, body: &B_BA4D80, thunk: t_ba4d80 },
    Port { name: "vint_object_tween::get_loop_mode", addr: 0x00B9_F960, body: &B_B9F960, thunk: t_b9f960 },
    Port { name: "vint_element_text::get_leading", addr: 0x00BA_42B0, body: &B_BA42B0, thunk: t_ba42b0 },
    Port { name: "gfx_set_color_bytes", addr: 0x00D1_4D00, body: &B_D14D00, thunk: t_d14d00 },
    Port { name: "vint_object_factory_imp<>::alloc_2", addr: 0x00B9_D490, body: &B_B9D490, thunk: t_b9d490 },
    Port { name: "vint_object_factory_imp<>::alloc_9", addr: 0x00BA_5F60, body: &B_BA5F60, thunk: t_ba5f60 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h3: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .zip(CNT.iter())
        .filter_map(|(p, c)| {
            let n = c.load(Ordering::Relaxed);
            (n > 0).then(|| format!("{} {n}", p.name))
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("port_h3: {}", parts.join(", "))
    }
}
