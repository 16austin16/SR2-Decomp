//! Ported small class methods H-Q (porting round "small_b"): the `IDirect3DDevice9Lost` stub
//! methods, the multiplayer registration handlers, scalar deleting destructors, memory pools and
//! assorted overlay / diversion / object methods.
//!
//! Every port is a naked `pushad` thunk (`port!`) around a Rust `unsafe fn(&mut Pushad, *const u32)
//! -> u32`: the register block gives the entry registers (EAX / ECX / EDX / ESI / EDI as the
//! LTCG original expects them), `a[i]` the stack arguments; the return value is EAX, and the
//! block's fields are what `popad` restores (so every register the original preserves comes back
//! unchanged, and the ones it clobbers deterministically are written explicitly). XMM0-7 are
//! saved around the Rust body. Callees stay the original game functions, called by address
//! through `call_regs` with the exact registers and stack arguments the original sets up. The
//! `IDirect3DDevice9Lost` stubs are naked asm identical to the original instructions.
//!
//! Checks: offline native differential tests (`mod tests`) run the original bytes and the real
//! thunk on the same random world and compare every register the original defines, the stack
//! pointer, every arena / global word and the ordered log of callee calls (callees are recording
//! stubs). In game each port counts its calls (first-calls counter only, a plain increment) and the
//! counts appear in `summary()`; there is no in-process shadow.

#![allow(unused_imports, non_upper_case_globals, dead_code, clippy::missing_safety_doc)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Pushad {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
}

/// Game address -> the address the code uses (identity in the game; a test page offline).
#[inline(always)]
fn ga(a: usize) -> usize {
    #[cfg(test)]
    {
        tmap::ga(a)
    }
    #[cfg(not(test))]
    {
        a
    }
}

/// The callee's address (the offline test aims the game's functions at recording stubs).
#[inline(always)]
fn tgt(a: usize) -> usize {
    #[cfg(test)]
    {
        tmap::tgt(a)
    }
    #[cfg(not(test))]
    {
        a
    }
}

#[inline(always)]
unsafe fn cr(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { call_regs(tgt(f), eax, ecx, edx, esi, edi, args) }
}

/// Call through a function pointer read from game memory (vtable slot / import slot).
#[inline(always)]
unsafe fn crp(f: u32, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { call_regs(f as usize, eax, ecx, edx, esi, edi, args) }
}

#[inline(always)]
unsafe fn arg(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

/// What `fld dword` / `fstp dword` does to a signalling NaN.
#[inline(always)]
fn q(b: u32) -> u32 {
    if b & 0x7F80_0000 == 0x7F80_0000 && b & 0x007F_FFFF != 0 { b | 0x0040_0000 } else { b }
}

#[inline(always)]
fn lo8(old: u32, v: u32) -> u32 {
    (old & !0xFF) | (v & 0xFF)
}

/// A naked `pushad` thunk (XMM0-7 saved) around `f(&mut Pushad, args) -> eax`, counting calls.
macro_rules! port {
    ($th:ident, $f:ident, $cnt:ident, $ret:literal) => {
        static $cnt: AtomicU32 = AtomicU32::new(0);
        #[unsafe(naked)]
        extern "C" fn $th() {
            std::arch::naked_asm!(
                "inc dword ptr [{c}]",
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
                "mov dword ptr [esp + 156], eax",
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
                c = sym $cnt,
                f = sym $f,
            )
        }
    };
}

/// One replaced function.
struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
    cnt: &'static AtomicU32,
}

fn install_ports(ports: &[Port], ok: &mut u32, tot: &mut u32) {
    for p in ports {
        *tot += 1;
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            *ok += 1;
        }
    }
}

fn sum_ports(ports: &[Port], out: &mut Vec<String>) {
    for p in ports {
        let c = p.cnt.load(Ordering::Relaxed);
        if c > 0 {
            out.push(format!("{} {c}", p.name));
        }
    }
}

// ---- IDirect3DDevice9Lost stubs (naked asm identical to the originals) ----

#[unsafe(naked)]
extern "C" fn d3d_z12() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 12", c = sym c_d3d_z12)
}
static c_d3d_z12: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z8() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 8", c = sym c_d3d_z8)
}
static c_d3d_z8: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z16() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 16", c = sym c_d3d_z16)
}
static c_d3d_z16: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_424() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 424]",
        "jmp eax",
        c = sym c_d3d_feax_424,
    )
}
static c_d3d_feax_424: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z20() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 20", c = sym c_d3d_z20)
}
static c_d3d_z20: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_364() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 364]",
        "jmp eax",
        c = sym c_d3d_feax_364,
    )
}
static c_d3d_feax_364: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_344() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 344]",
        "jmp eax",
        c = sym c_d3d_feax_344,
    )
}
static c_d3d_feax_344: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z28() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 28", c = sym c_d3d_z28)
}
static c_d3d_z28: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z36() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 36", c = sym c_d3d_z36)
}
static c_d3d_z36: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z4() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 4", c = sym c_d3d_z4)
}
static c_d3d_z4: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_144() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 144]",
        "jmp eax",
        c = sym c_d3d_feax_144,
    )
}
static c_d3d_feax_144: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z24() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 24", c = sym c_d3d_z24)
}
static c_d3d_z24: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_116() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 116]",
        "jmp eax",
        c = sym c_d3d_feax_116,
    )
}
static c_d3d_feax_116: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_112() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 112]",
        "jmp eax",
        c = sym c_d3d_feax_112,
    )
}
static c_d3d_feax_112: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_108() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 108]",
        "jmp eax",
        c = sym c_d3d_feax_108,
    )
}
static c_d3d_feax_108: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_104() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 104]",
        "jmp eax",
        c = sym c_d3d_feax_104,
    )
}
static c_d3d_feax_104: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_100() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 100]",
        "jmp eax",
        c = sym c_d3d_feax_100,
    )
}
static c_d3d_feax_100: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_z40() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "xor eax, eax", "ret 40", c = sym c_d3d_z40)
}
static c_d3d_z40: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_feax_92() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov eax, dword ptr [ecx + 92]",
        "jmp eax",
        c = sym c_d3d_feax_92,
    )
}
static c_d3d_feax_92: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_one8() {
    std::arch::naked_asm!("inc dword ptr [{c}]", "mov eax, 1", "ret 8", c = sym c_d3d_one8)
}
static c_d3d_one8: AtomicU32 = AtomicU32::new(0);
#[unsafe(naked)]
extern "C" fn d3d_fedx_12() {
    std::arch::naked_asm!(
        "inc dword ptr [{c}]",
        "mov eax, dword ptr [esp + 4]",
        "mov eax, dword ptr [eax + 4]",
        "mov ecx, dword ptr [eax]",
        "mov dword ptr [esp + 4], eax",
        "mov edx, dword ptr [ecx + 12]",
        "jmp edx",
        c = sym c_d3d_fedx_12,
    )
}
static c_d3d_fedx_12: AtomicU32 = AtomicU32::new(0);

static D3D_PORTS: [Port; 115] = [
    Port { name: "D3DLost::CreateQuery", addr: 0xcf50c0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetStreamSourceFreq", addr: 0xcf51c0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetStreamSourceFreq", addr: 0xcf51d0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetPaletteEntries", addr: 0xcf53d0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetPaletteEntries", addr: 0xcf53e0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetTexture", addr: 0xcf5440, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetTexture", addr: 0xcf5450, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::CreateStateBlock", addr: 0xcf54a0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetRenderState", addr: 0xcf54b0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetRenderState", addr: 0xcf54c0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetClipPlane", addr: 0xcf54d0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetClipPlane", addr: 0xcf54e0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetLightEnable", addr: 0xcf54f0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::LightEnable", addr: 0xcf5500, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetLight", addr: 0xcf5510, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetLight", addr: 0xcf5520, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::MultiplyTransform", addr: 0xcf5570, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetTransform", addr: 0xcf5580, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetTransform", addr: 0xcf5590, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetRenderTarget", addr: 0xcf55f0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::SetRenderTarget", addr: 0xcf5600, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetFrontBufferData", addr: 0xcf5650, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetRenderTargetData", addr: 0xcf5660, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::UpdateTexture", addr: 0xcf5670, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetRasterStatus", addr: 0xcf5790, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetSwapChain", addr: 0xcf57e0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::CreateAdditionalSwapChain", addr: 0xcf57f0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::GetDisplayMode", addr: 0xcf5840, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::QueryInterface", addr: 0xcf58d0, body: &[0x33, 0xC0, 0xC2, 0x0C, 0x00], thunk: d3d_z12, cnt: &c_d3d_z12 },
    Port { name: "D3DLost::DeletePatch", addr: 0xcf50d0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetPixelShader", addr: 0xcf5160, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetPixelShader", addr: 0xcf5170, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetIndices", addr: 0xcf51a0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetIndices", addr: 0xcf51b0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetVertexShader", addr: 0xcf5260, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetVertexShader", addr: 0xcf5270, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetFVF", addr: 0xcf52a0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetFVF", addr: 0xcf52b0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetVertexDeclaration", addr: 0xcf52c0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetVertexDeclaration", addr: 0xcf52d0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetNPatchMode", addr: 0xcf5360, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetSoftwareVertexProcessing", addr: 0xcf5380, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetScissorRect", addr: 0xcf5390, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetScissorRect", addr: 0xcf53a0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetCurrentTexturePalette", addr: 0xcf53b0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetCurrentTexturePalette", addr: 0xcf53c0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::ValidateDevice", addr: 0xcf53f0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetClipStatus", addr: 0xcf5460, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetClipStatus", addr: 0xcf5470, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::EndStateBlock", addr: 0xcf5480, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetMaterial", addr: 0xcf5530, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetMaterial", addr: 0xcf5540, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetViewport", addr: 0xcf5550, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetViewport", addr: 0xcf5560, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetDepthStencilSurface", addr: 0xcf55d0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetDepthStencilSurface", addr: 0xcf55e0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::SetDialogBoxMode", addr: 0xcf5780, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::Reset", addr: 0xcf57c0, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetCreationParameters", addr: 0xcf5830, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetDeviceCaps", addr: 0xcf5850, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::GetDirect3D", addr: 0xcf5860, body: &[0x33, 0xC0, 0xC2, 0x08, 0x00], thunk: d3d_z8, cnt: &c_d3d_z8 },
    Port { name: "D3DLost::DrawTriPatch", addr: 0xcf50e0, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::DrawRectPatch", addr: 0xcf50f0, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetPixelShaderConstantB", addr: 0xcf5100, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetPixelShaderConstantB", addr: 0xcf5110, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetPixelShaderConstantI", addr: 0xcf5120, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetPixelShaderConstantI", addr: 0xcf5130, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetPixelShaderConstantF", addr: 0xcf5140, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetPixelShaderConstantF", addr: 0xcf5150, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetVertexShaderConstantB", addr: 0xcf5200, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetVertexShaderConstantB", addr: 0xcf5210, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetVertexShaderConstantI", addr: 0xcf5220, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetVertexShaderConstantI", addr: 0xcf5230, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetVertexShaderConstantF", addr: 0xcf5240, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetVertexShaderConstantF", addr: 0xcf5250, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::DrawPrimitive", addr: 0xcf5340, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetSamplerState", addr: 0xcf5400, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetSamplerState", addr: 0xcf5410, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetTextureStageState", addr: 0xcf5420, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::GetTextureStageState", addr: 0xcf5430, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::ColorFill", addr: 0xcf5630, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::SetCursorProperties", addr: 0xcf5820, body: &[0x33, 0xC0, 0xC2, 0x10, 0x00], thunk: d3d_z16, cnt: &c_d3d_z16 },
    Port { name: "D3DLost::CreatePixelShader", addr: 0xcf5180, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0xA8, 0x01, 0x00, 0x00, 0xFF, 0xE0], thunk: d3d_feax_424, cnt: &c_d3d_feax_424 },
    Port { name: "D3DLost::GetStreamSource", addr: 0xcf51e0, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::SetStreamSource", addr: 0xcf51f0, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::DrawPrimitiveUP", addr: 0xcf5320, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::UpdateSurface", addr: 0xcf5680, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::GetBackBuffer", addr: 0xcf57a0, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::Present", addr: 0xcf57b0, body: &[0x33, 0xC0, 0xC2, 0x14, 0x00], thunk: d3d_z20, cnt: &c_d3d_z20 },
    Port { name: "D3DLost::CreateVertexShader", addr: 0xcf5280, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x6C, 0x01, 0x00, 0x00, 0xFF, 0xE0], thunk: d3d_feax_364, cnt: &c_d3d_feax_364 },
    Port { name: "D3DLost::CreateVertexDeclaration", addr: 0xcf52e0, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x58, 0x01, 0x00, 0x00, 0xFF, 0xE0], thunk: d3d_feax_344, cnt: &c_d3d_feax_344 },
    Port { name: "D3DLost::ProcessVertices", addr: 0xcf5300, body: &[0x33, 0xC0, 0xC2, 0x1C, 0x00], thunk: d3d_z28, cnt: &c_d3d_z28 },
    Port { name: "D3DLost::DrawIndexedPrimitive", addr: 0xcf5330, body: &[0x33, 0xC0, 0xC2, 0x1C, 0x00], thunk: d3d_z28, cnt: &c_d3d_z28 },
    Port { name: "D3DLost::Clear", addr: 0xcf55a0, body: &[0x33, 0xC0, 0xC2, 0x1C, 0x00], thunk: d3d_z28, cnt: &c_d3d_z28 },
    Port { name: "D3DLost::DrawIndexedPrimitiveUP", addr: 0xcf5310, body: &[0x33, 0xC0, 0xC2, 0x24, 0x00], thunk: d3d_z36, cnt: &c_d3d_z36 },
    Port { name: "D3DLost::GetSoftwareVertexProcessing", addr: 0xcf5370, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::BeginStateBlock", addr: 0xcf5490, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::EndScene", addr: 0xcf55b0, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::BeginScene", addr: 0xcf55c0, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::GetNumberOfSwapChains", addr: 0xcf57d0, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::EvictManagedResources", addr: 0xcf5870, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::GetAvailableTextureMem", addr: 0xcf5880, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::Release", addr: 0xcf58b0, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::AddRef", addr: 0xcf58c0, body: &[0x33, 0xC0, 0xC2, 0x04, 0x00], thunk: d3d_z4, cnt: &c_d3d_z4 },
    Port { name: "D3DLost::CreateOffscreenPlainSurface", addr: 0xcf5610, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x90, 0x00, 0x00, 0x00, 0xFF, 0xE0], thunk: d3d_feax_144, cnt: &c_d3d_feax_144 },
    Port { name: "D3DLost::StretchRect", addr: 0xcf5640, body: &[0x33, 0xC0, 0xC2, 0x18, 0x00], thunk: d3d_z24, cnt: &c_d3d_z24 },
    Port { name: "D3DLost::CreateDepthStencilSurface", addr: 0xcf5690, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x74, 0xFF, 0xE0], thunk: d3d_feax_116, cnt: &c_d3d_feax_116 },
    Port { name: "D3DLost::CreateRenderTarget", addr: 0xcf56b0, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x70, 0xFF, 0xE0], thunk: d3d_feax_112, cnt: &c_d3d_feax_112 },
    Port { name: "D3DLost::CreateIndexBuffer", addr: 0xcf56d0, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x6C, 0xFF, 0xE0], thunk: d3d_feax_108, cnt: &c_d3d_feax_108 },
    Port { name: "D3DLost::CreateVertexBuffer", addr: 0xcf56f0, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x68, 0xFF, 0xE0], thunk: d3d_feax_104, cnt: &c_d3d_feax_104 },
    Port { name: "D3DLost::CreateCubeTexture", addr: 0xcf5710, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x64, 0xFF, 0xE0], thunk: d3d_feax_100, cnt: &c_d3d_feax_100 },
    Port { name: "D3DLost::CreateVolumeTexture", addr: 0xcf5730, body: &[0x33, 0xC0, 0xC2, 0x28, 0x00], thunk: d3d_z40, cnt: &c_d3d_z40 },
    Port { name: "D3DLost::CreateTexture", addr: 0xcf5740, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x5C, 0xFF, 0xE0], thunk: d3d_feax_92, cnt: &c_d3d_feax_92 },
    Port { name: "D3DLost::ShowCursor", addr: 0xcf5800, body: &[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x08, 0x00], thunk: d3d_one8, cnt: &c_d3d_one8 },
    Port { name: "D3DLost::TestCooperativeLevel", addr: 0xcf5890, body: &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x51, 0x0C, 0xFF, 0xE2], thunk: d3d_fedx_12, cnt: &c_d3d_fedx_12 },
];

// ---- ports ----

// 0x004edbc0 .. 0x004edbfc (60 bytes)
const B_num_put_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x20, 0xF2, 0xDB, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x85, 0x0C, 0x7F, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_num_put_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xdbf220);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_num_put_sdd, f_num_put_sdd, c_num_put_sdd, "ret 4");
// 0x0068f020 .. 0x0068f05c (60 bytes)
const B_pc_multi_session_info_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x54, 0xFD, 0xDF, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x25, 0xF8, 0x64, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pc_multi_session_info_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xdffd54);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_pc_multi_session_info_sdd, f_pc_multi_session_info_sdd, c_pc_multi_session_info_sdd, "ret 4");
// 0x0068f070 .. 0x0068f0ac (60 bytes)
const B_multi_session_info_base_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x54, 0xFD, 0xDF, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xD5, 0xF7, 0x64, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_multi_session_info_base_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xdffd54);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_multi_session_info_base_sdd, f_multi_session_info_base_sdd, c_multi_session_info_base_sdd, "ret 4");
// 0x006f4d70 .. 0x006f4dac (60 bytes)
const B_havok_closest_ray_hit_collector_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xDC, 0x87, 0xE1, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xD5, 0x9A, 0x5E, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_closest_ray_hit_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe187dc);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_havok_closest_ray_hit_collector_sdd, f_havok_closest_ray_hit_collector_sdd, c_havok_closest_ray_hit_collector_sdd, "ret 4");
// 0x008277a0 .. 0x008277dc (60 bytes)
const B_multi_core_object_registration_sdd2: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x7C, 0x7E, 0xE2, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xA5, 0x70, 0x4B, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_multi_core_object_registration_sdd2(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe27e7c);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_multi_core_object_registration_sdd2, f_multi_core_object_registration_sdd2, c_multi_core_object_registration_sdd2, "ret 4");
// 0x00843170 .. 0x008431ac (60 bytes)
const B_item_multi_registration_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x7C, 0x7E, 0xE2, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xD5, 0xB6, 0x49, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_item_multi_registration_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe27e7c);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_item_multi_registration_sdd, f_item_multi_registration_sdd, c_item_multi_registration_sdd, "ret 4");
// 0x0085c650 .. 0x0085c68c (60 bytes)
const B_pipebomb_multi_registration_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x7C, 0x7E, 0xE2, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xF5, 0x21, 0x48, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pipebomb_multi_registration_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe27e7c);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_pipebomb_multi_registration_sdd, f_pipebomb_multi_registration_sdd, c_pipebomb_multi_registration_sdd, "ret 4");
// 0x0086ba20 .. 0x0086ba5c (60 bytes)
const B_prop_multi_registration_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x7C, 0x7E, 0xE2, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x25, 0x2E, 0x47, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_prop_multi_registration_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe27e7c);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_prop_multi_registration_sdd, f_prop_multi_registration_sdd, c_prop_multi_registration_sdd, "ret 4");
// 0x00bc45a0 .. 0x00bc45dc (60 bytes)
const B_nearest_hit_collector_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xDC, 0x87, 0xE1, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xA5, 0xA2, 0x11, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_nearest_hit_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe187dc);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_nearest_hit_collector_sdd, f_nearest_hit_collector_sdd, c_nearest_hit_collector_sdd, "ret 4");
// 0x00bf4950 .. 0x00bf498c (60 bytes)
const B_mempool_sdd: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x8C, 0x8C, 0xE5, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0xF5, 0x9E, 0x0E, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mempool_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // test byte [esp+4],1 ; push esi ; mov esi,ecx ; mov [esi],VT ; je .. ; mov al,[0x2527bf7] ; cmp al,1 ; je ..
    let this = r.ecx;
    wd(this as usize, 0xe58c8c);
    if arg(a, 0) & 1 != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(r.eax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_mempool_sdd, f_mempool_sdd, c_mempool_sdd, "ret 4");
// 0x00843320 .. 0x00843370 (80 bytes)
const B_item_on_register: [u8; 80] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x45, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x2E, 0x39, 0x46, 0x44, 0x75, 0x29, 0x83, 0x7E, 0x4C, 0x02, 0x75, 0x23, 0xE8, 0xA2, 0x3D, 0x00, 0x00, 0x85, 0xC0, 0x89, 0x86, 0x10, 0x01, 0x00, 0x00, 0x74, 0x0A, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0x81, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_item_on_register(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 2 {
        return id;
    }
    let p = cr(0x8470f0, id, r.ecx, r.edx, e, r.edi, &[]);
    wd(e as usize + 0x110, p);
    if p != 0 {
        wd(p as usize + 4, 0);
        wb(p as usize, 0);
    }
    wd(e as usize + 0x80, rd(e as usize + 0x80) | 0x180);
    p
    }
}
port!(th_item_on_register, f_item_on_register, c_item_on_register, "ret 4");
// 0x00843b50 .. 0x00843b94 (68 bytes)
const B_item_on_unregister: [u8; 68] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x39, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x57, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x22, 0x39, 0x47, 0x44, 0x75, 0x1D, 0x83, 0x7F, 0x4C, 0x02, 0x75, 0x17, 0x56, 0x8B, 0xB7, 0x10, 0x01, 0x00, 0x00, 0xE8, 0x6B, 0x36, 0x00, 0x00, 0xC7, 0x87, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_item_on_unregister(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 2 {
        return id;
    }
    let ret = cr(0x8471f0, id, r.ecx, r.edx, rd(e as usize + 0x110), e, &[]);
    wd(e as usize + 0x110, 0);
    ret
    }
}
port!(th_item_on_unregister, f_item_on_unregister, c_item_on_unregister, "ret 4");
// 0x00843bb0 .. 0x00843c05 (85 bytes)
const B_item_destroy_from_data: [u8; 85] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x4A, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x33, 0x39, 0x46, 0x44, 0x75, 0x2E, 0x83, 0x7E, 0x4C, 0x02, 0x75, 0x28, 0x8B, 0x8E, 0xCC, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x09, 0x8B, 0x11, 0x8B, 0x42, 0x20, 0x6A, 0x01, 0xFF, 0xD0, 0x8B, 0x16, 0x8B, 0x42, 0x10, 0x6A, 0x01, 0x8B, 0xCE, 0xFF, 0xD0, 0x81, 0xA6, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_item_destroy_from_data(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 2 {
        return id;
    }
    let c = rd(e as usize + 0xcc);
    if c != 0 {
        let vt = rd(c as usize);
        crp(rd(vt as usize + 0x20), rd(vt as usize + 0x20), c, vt, e, r.edi, &[1]);
        r.ecx = c;
        r.edx = vt;
    }
    let vt = rd(e as usize);
    let ax = crp(rd(vt as usize + 0x10), rd(vt as usize + 0x10), e, vt, e, r.edi, &[1]);
    r.ecx = e;
    r.edx = vt;
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    ax
    }
}
port!(th_item_destroy_from_data, f_item_destroy_from_data, c_item_destroy_from_data, "ret 12");
// 0x0084b8b0 .. 0x0084b907 (87 bytes)
const B_mover_on_register: [u8; 87] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x4C, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x35, 0x39, 0x46, 0x44, 0x75, 0x30, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x39, 0x46, 0x4C, 0x75, 0x26, 0x39, 0x86, 0xD0, 0x00, 0x00, 0x00, 0x75, 0x1E, 0xE8, 0x46, 0x4D, 0x00, 0x00, 0x85, 0xC0, 0x89, 0x86, 0x88, 0x01, 0x00, 0x00, 0x74, 0x05, 0xE8, 0x67, 0xD4, 0xFE, 0xFF, 0x81, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mover_on_register(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id {
        return id;
    }
    if rd(e as usize + 0x4c) != 3 || rd(e as usize + 0xd0) != 3 {
        return 3;
    }
    let p = cr(0x850630, 3, r.ecx, r.edx, e, r.edi, &[]);
    wd(e as usize + 0x188, p);
    let mut ret = p;
    if p != 0 {
        ret = cr(0x838d60, p, r.ecx, r.edx, e, r.edi, &[]);
    }
    wd(e as usize + 0x80, rd(e as usize + 0x80) | 0x180);
    ret
    }
}
port!(th_mover_on_register, f_mover_on_register, c_mover_on_register, "ret 4");
// 0x0084bf10 .. 0x0084bf6a (90 bytes)
const B_mover_on_unregister: [u8; 90] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x4F, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x57, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x38, 0x39, 0x47, 0x44, 0x75, 0x33, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x39, 0x47, 0x4C, 0x75, 0x29, 0x39, 0x87, 0xD0, 0x00, 0x00, 0x00, 0x75, 0x21, 0x56, 0x8B, 0xB7, 0x88, 0x01, 0x00, 0x00, 0xE8, 0xDF, 0x47, 0x00, 0x00, 0x81, 0xA7, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xC7, 0x87, 0x88, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mover_on_unregister(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id {
        return id;
    }
    if rd(e as usize + 0x4c) != 3 || rd(e as usize + 0xd0) != 3 {
        return 3;
    }
    let ret = cr(0x850730, 3, r.ecx, r.edx, rd(e as usize + 0x188), e, &[]);
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    wd(e as usize + 0x188, 0);
    ret
    }
}
port!(th_mover_on_unregister, f_mover_on_unregister, c_mover_on_unregister, "ret 4");
// 0x00851be0 .. 0x00851c31 (81 bytes)
const B_npc_on_register: [u8; 81] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x46, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x2F, 0x39, 0x46, 0x44, 0x75, 0x2A, 0x83, 0x7E, 0x4C, 0x01, 0x75, 0x24, 0x80, 0xBE, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x1B, 0xE8, 0xC9, 0xA3, 0x00, 0x00, 0x89, 0x86, 0xB4, 0x11, 0x00, 0x00, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_on_register(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 1 || rb(e as usize + 0x3cc) != 2 {
        return id;
    }
    let p = cr(0x85bfe0, id, r.ecx, r.edx, e, r.edi, &[]);
    wd(e as usize + 0x11b4, p);
    if p != 0 {
        // the original stores through EAX unchecked (a null result would fault)
        wd(p as usize, 0);
    }
    wd(e as usize + 0x80, rd(e as usize + 0x80) | 0x180);
    p
    }
}
port!(th_npc_on_register, f_npc_on_register, c_npc_on_register, "ret 4");
// 0x00852450 .. 0x0085249d (77 bytes)
const B_npc_on_unregister: [u8; 77] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x42, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x57, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x2B, 0x39, 0x47, 0x44, 0x75, 0x26, 0x83, 0x7F, 0x4C, 0x01, 0x75, 0x20, 0x80, 0xBF, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x17, 0x56, 0x8B, 0xB7, 0xB4, 0x11, 0x00, 0x00, 0xE8, 0x52, 0x9C, 0x00, 0x00, 0xC7, 0x87, 0xB4, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_on_unregister(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 1 || rb(e as usize + 0x3cc) != 2 {
        return id;
    }
    let ret = cr(0x85c0e0, id, r.ecx, r.edx, rd(e as usize + 0x11b4), e, &[]);
    wd(e as usize + 0x11b4, 0);
    ret
    }
}
port!(th_npc_on_unregister, f_npc_on_unregister, c_npc_on_unregister, "ret 4");
// 0x008524b0 .. 0x00852516 (102 bytes)
const B_npc_destroy_from_data: [u8; 102] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x5B, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x57, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x44, 0x39, 0x47, 0x44, 0x75, 0x3F, 0x83, 0x7F, 0x4C, 0x01, 0x75, 0x39, 0x80, 0xBF, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x30, 0x8B, 0x17, 0x8B, 0x42, 0x10, 0x56, 0x6A, 0x01, 0x8B, 0xCF, 0xFF, 0xD0, 0x8B, 0xB7, 0xBC, 0x11, 0x00, 0x00, 0x81, 0xA7, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x85, 0xF6, 0x74, 0x05, 0xE8, 0x99, 0x84, 0x00, 0x00, 0xC7, 0x87, 0xBC, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_destroy_from_data(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 1 || rb(e as usize + 0x3cc) != 2 {
        return id;
    }
    let vt = rd(e as usize);
    let fnp = rd(vt as usize + 0x10);
    let mut ax = crp(fnp, fnp, e, vt, r.esi, e, &[1]);
    r.ecx = e;
    r.edx = vt;
    let s = rd(e as usize + 0x11bc);
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    if s != 0 {
        ax = cr(0x85a9a0, ax, e, vt, s, e, &[]);
    }
    wd(e as usize + 0x11bc, 0);
    ax
    }
}
port!(th_npc_destroy_from_data, f_npc_destroy_from_data, c_npc_destroy_from_data, "ret 12");
// 0x0085c810 .. 0x0085c863 (83 bytes)
const B_pipebomb_on_register: [u8; 83] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x48, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x31, 0x39, 0x46, 0x44, 0x75, 0x2C, 0x83, 0x7E, 0x4C, 0x04, 0x75, 0x26, 0x57, 0xBF, 0xA8, 0x36, 0xEB, 0x00, 0xE8, 0xEC, 0x20, 0x00, 0x00, 0x89, 0x86, 0x1C, 0x01, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0x81, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pipebomb_on_register(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 4 {
        return id;
    }
    let p = cr(0x85e930, id, r.ecx, r.edx, e, 0xeb36a8, &[]);
    wd(e as usize + 0x11c, p);
    if p != 0 {
        // unchecked store in the original
        wd(p as usize + 4, 0);
        wb(p as usize, 0);
    }
    wd(e as usize + 0x80, rd(e as usize + 0x80) | 0x180);
    p
    }
}
port!(th_pipebomb_on_register, f_pipebomb_on_register, c_pipebomb_on_register, "ret 4");
// 0x0085cbf0 .. 0x0085cc3b (75 bytes)
const B_pipebomb_on_unregister: [u8; 75] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x40, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x53, 0x8B, 0x99, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xDB, 0x74, 0x29, 0x39, 0x43, 0x44, 0x75, 0x24, 0x83, 0x7B, 0x4C, 0x04, 0x75, 0x1E, 0x56, 0x8B, 0xB3, 0x1C, 0x01, 0x00, 0x00, 0x57, 0xBF, 0xA8, 0x36, 0xEB, 0x00, 0xE8, 0xF5, 0x1D, 0x00, 0x00, 0x5F, 0xC7, 0x83, 0x1C, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5B, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pipebomb_on_unregister(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 4 {
        return id;
    }
    let ret = cr(0x85ea20, id, r.ecx, r.edx, rd(e as usize + 0x11c), 0xeb36a8, &[]);
    wd(e as usize + 0x11c, 0);
    ret
    }
}
port!(th_pipebomb_on_unregister, f_pipebomb_on_unregister, c_pipebomb_on_unregister, "ret 4");
// 0x0085cc50 .. 0x0085cc8c (60 bytes)
const B_pipebomb_destroy_from_data: [u8; 60] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x31, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x1A, 0x39, 0x46, 0x44, 0x75, 0x15, 0x8B, 0x16, 0x8B, 0x42, 0x10, 0x6A, 0x01, 0x8B, 0xCE, 0xFF, 0xD0, 0x81, 0xA6, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pipebomb_destroy_from_data(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id {
        return id;
    }
    let vt = rd(e as usize);
    let fnp = rd(vt as usize + 0x10);
    let ret = crp(fnp, fnp, e, vt, e, r.edi, &[1]);
    r.ecx = e;
    r.edx = vt;
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    ret
    }
}
port!(th_pipebomb_destroy_from_data, f_pipebomb_destroy_from_data, c_pipebomb_destroy_from_data, "ret 12");
// 0x0086bbd0 .. 0x0086bc27 (87 bytes)
const B_prop_on_register: [u8; 87] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x4C, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x35, 0x39, 0x46, 0x44, 0x75, 0x30, 0x83, 0x7E, 0x4C, 0x0C, 0x75, 0x2A, 0x57, 0xBF, 0x18, 0x34, 0xEB, 0x00, 0xE8, 0x2C, 0x2D, 0xFF, 0xFF, 0x85, 0xC0, 0x89, 0x86, 0x18, 0x01, 0x00, 0x00, 0x5F, 0x74, 0x0A, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0x81, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_prop_on_register(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 0xc {
        return id;
    }
    let p = cr(0x85e930, id, r.ecx, r.edx, e, 0xeb3418, &[]);
    wd(e as usize + 0x118, p);
    if p != 0 {
        wd(p as usize + 4, 0);
        wb(p as usize, 0);
    }
    wd(e as usize + 0x80, rd(e as usize + 0x80) | 0x180);
    p
    }
}
port!(th_prop_on_register, f_prop_on_register, c_prop_on_register, "ret 4");
// 0x0086c040 .. 0x0086c08b (75 bytes)
const B_prop_on_unregister: [u8; 75] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x40, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x53, 0x8B, 0x99, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xDB, 0x74, 0x29, 0x39, 0x43, 0x44, 0x75, 0x24, 0x83, 0x7B, 0x4C, 0x0C, 0x75, 0x1E, 0x56, 0x8B, 0xB3, 0x18, 0x01, 0x00, 0x00, 0x57, 0xBF, 0x18, 0x34, 0xEB, 0x00, 0xE8, 0xA5, 0x29, 0xFF, 0xFF, 0x5F, 0xC7, 0x83, 0x18, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5B, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_prop_on_unregister(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 0xc {
        return id;
    }
    let ret = cr(0x85ea20, id, r.ecx, r.edx, rd(e as usize + 0x118), 0xeb3418, &[]);
    wd(e as usize + 0x118, 0);
    ret
    }
}
port!(th_prop_on_unregister, f_prop_on_unregister, c_prop_on_unregister, "ret 4");
// 0x0086c0a0 .. 0x0086c0de (62 bytes)
const B_prop_destroy_from_data: [u8; 62] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x33, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x1E, 0x39, 0x41, 0x44, 0x75, 0x19, 0x83, 0x79, 0x4C, 0x0C, 0x75, 0x13, 0x8B, 0x11, 0x8B, 0x42, 0x10, 0x81, 0xA1, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x6A, 0x01, 0xFF, 0xD0, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_prop_destroy_from_data(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    let e = rd(ga(0x2149C64 + (((id & 0xFFF) as usize) << 4)));
    r.ecx = e;
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 0xc {
        return id;
    }
    let vt = rd(e as usize);
    let fnp = rd(vt as usize + 0x10);
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    r.edx = vt;
    crp(fnp, fnp, e, vt, r.esi, r.edi, &[1])
    }
}
port!(th_prop_destroy_from_data, f_prop_destroy_from_data, c_prop_destroy_from_data, "ret 12");
// 0x0086c670 .. 0x0086c6c1 (81 bytes)
const B_prop_free_snapshot: [u8; 81] = [0xA0, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x77, 0x10, 0x74, 0x12, 0xE8, 0x59, 0x1E, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x47, 0x10, 0x89, 0x47, 0x0C, 0x5F, 0x5E, 0xC2, 0x04, 0x00, 0x83, 0x3D, 0x38, 0xC4, 0xE8, 0x00, 0x08, 0x75, 0x12, 0xE8, 0xBE, 0x12, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x47, 0x10, 0x89, 0x47, 0x0C, 0x5F, 0x5E, 0xC2, 0x04, 0x00, 0xE8, 0x7C, 0x18, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x47, 0x10, 0x89, 0x47, 0x0C, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_prop_free_snapshot(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // eax low byte = [0x2528b49]; esi = [arg+0x10]
    let obj = arg(a, 0);
    let b = rb(ga(0x2528b49)) as u32;
    let s = rd(obj as usize + 0x10);
    let ax = lo8(r.eax, b);
    if b != 0 {
        cr(0x86e4e0, ax, r.ecx, r.edx, s, obj, &[]);
    } else if rd(ga(0xe8c438)) == 8 {
        cr(0x86d960, ax, r.ecx, r.edx, s, obj, &[]);
    } else {
        cr(0x86df30, ax, r.ecx, r.edx, s, obj, &[]);
    }
    wd(obj as usize + 0x10, 0);
    wd(obj as usize + 0xc, 0);
    0
    }
}
port!(th_prop_free_snapshot, f_prop_free_snapshot, c_prop_free_snapshot, "ret 4");
// 0x00845530 .. 0x0084558f (95 bytes)
const B_item_on_owner_left: [u8; 95] = [0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x2A, 0x8A, 0x0D, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x75, 0x20, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x50, 0xE8, 0x9E, 0xA7, 0xFC, 0xFF, 0x84, 0xC0, 0x74, 0x05, 0x33, 0xC0, 0xC2, 0x04, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x1B, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x75, 0xCF, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_item_on_owner_left(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // al = [0x2528b48]; cl = [0x2528b49] (only when al == 1)
    let b48 = rb(ga(0x2528b48)) as u32;
    let host = rd(ga(0x2528c14));
    if b48 == 1 {
        let b49 = rb(ga(0x2528b49)) as u32;
        r.ecx = lo8(r.ecx, b49);
        if b49 == 0 {
            if host != 0 {
                let ok = cr(0x80fcf0, host, r.ecx, r.edx, r.esi, r.edi, &[host]);
                if ok & 0xFF != 0 {
                    return 0;
                }
            }
            return 2;
        }
    }
    if host == 0 {
        return 1;
    }
    if rb(ga(0x2528b56)) != 0 && rb(ga(0x2528b2b)) == 0 {
        return 1;
    }
    if rb(host as usize + 0x3f8) != 0 {
        return 0;
    }
    1
    }
}
port!(th_item_on_owner_left, f_item_on_owner_left, c_item_on_owner_left, "ret 4");
// 0x0084dfa0 .. 0x0084dfff (95 bytes)
const B_mover_on_owner_left: [u8; 95] = [0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x2A, 0x8A, 0x0D, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x75, 0x20, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x50, 0xE8, 0x2E, 0x1D, 0xFC, 0xFF, 0x84, 0xC0, 0x74, 0x05, 0x33, 0xC0, 0xC2, 0x04, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x1B, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x75, 0xCF, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mover_on_owner_left(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // al = [0x2528b48]; cl = [0x2528b49] (only when al == 1)
    let b48 = rb(ga(0x2528b48)) as u32;
    let host = rd(ga(0x2528c14));
    if b48 == 1 {
        let b49 = rb(ga(0x2528b49)) as u32;
        r.ecx = lo8(r.ecx, b49);
        if b49 == 0 {
            if host != 0 {
                let ok = cr(0x80fcf0, host, r.ecx, r.edx, r.esi, r.edi, &[host]);
                if ok & 0xFF != 0 {
                    return 0;
                }
            }
            return 2;
        }
    }
    if host == 0 {
        return 1;
    }
    if rb(ga(0x2528b56)) != 0 && rb(ga(0x2528b2b)) == 0 {
        return 1;
    }
    if rb(host as usize + 0x3f8) != 0 {
        return 0;
    }
    1
    }
}
port!(th_mover_on_owner_left, f_mover_on_owner_left, c_mover_on_owner_left, "ret 4");
// 0x00575190 .. 0x005751a4 (20 bytes)
const B_large_convoy_objective_get_start_message: [u8; 20] = [0x56, 0x57, 0xBF, 0x98, 0x88, 0xDE, 0x00, 0xBE, 0xC8, 0x88, 0xDE, 0x00, 0xE8, 0x3F, 0xF8, 0x27, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_large_convoy_objective_get_start_message(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f49e0, r.eax, r.ecx, r.edx, 0xde88c8, 0xde8898, &[])
    }
}
port!(th_large_convoy_objective_get_start_message, f_large_convoy_objective_get_start_message, c_large_convoy_objective_get_start_message, "ret");
// 0x00575210 .. 0x00575224 (20 bytes)
const B_liberator_objective_get_start_message: [u8; 20] = [0x56, 0x57, 0xBF, 0xB8, 0x8A, 0xDE, 0x00, 0xBE, 0x20, 0x8B, 0xDE, 0x00, 0xE8, 0xAF, 0xF8, 0x27, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_liberator_objective_get_start_message(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f4ad0, r.eax, r.ecx, r.edx, 0xde8b20, 0xde8ab8, &[])
    }
}
port!(th_liberator_objective_get_start_message, f_liberator_objective_get_start_message, c_liberator_objective_get_start_message, "ret");
// 0x006124f0 .. 0x00612504 (20 bytes)
const B_hostage_diversion_get_title: [u8; 20] = [0x56, 0x57, 0xBF, 0x48, 0x54, 0xDF, 0x00, 0xBE, 0x58, 0x54, 0xDF, 0x00, 0xE8, 0xDF, 0x24, 0x1E, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_hostage_diversion_get_title(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f49e0, r.eax, r.ecx, r.edx, 0xdf5458, 0xdf5448, &[])
    }
}
port!(th_hostage_diversion_get_title, f_hostage_diversion_get_title, c_hostage_diversion_get_title, "ret");
// 0x006142c0 .. 0x006142d4 (20 bytes)
const B_mugging_diversion_get_title: [u8; 20] = [0x56, 0x57, 0xBF, 0x60, 0x57, 0xDF, 0x00, 0xBE, 0x68, 0x57, 0xDF, 0x00, 0xE8, 0xFF, 0x07, 0x1E, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mugging_diversion_get_title(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f4ad0, r.eax, r.ecx, r.edx, 0xdf5768, 0xdf5760, &[])
    }
}
port!(th_mugging_diversion_get_title, f_mugging_diversion_get_title, c_mugging_diversion_get_title, "ret");
// 0x00635890 .. 0x006358a4 (20 bytes)
const B_poker_diversion_get_title: [u8; 20] = [0x56, 0x57, 0xBF, 0x0C, 0x98, 0xDF, 0x00, 0xBE, 0x18, 0x98, 0xDF, 0x00, 0xE8, 0x2F, 0xF2, 0x1B, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_poker_diversion_get_title(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f4ad0, r.eax, r.ecx, r.edx, 0xdf9818, 0xdf980c, &[])
    }
}
port!(th_poker_diversion_get_title, f_poker_diversion_get_title, c_poker_diversion_get_title, "ret");
// 0x0063ab70 .. 0x0063ab84 (20 bytes)
const B_hitman_diversion_get_title: [u8; 20] = [0x56, 0x57, 0xBF, 0x10, 0x9F, 0xDF, 0x00, 0xBE, 0x34, 0x9F, 0xDF, 0x00, 0xE8, 0x5F, 0x9E, 0x1B, 0x00, 0x5F, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_hitman_diversion_get_title(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi/edi; edi = fallback, esi = id; call lstr_get*; ESI/EDI restored by popad
    cr(0x7f49e0, r.eax, r.ecx, r.edx, 0xdf9f34, 0xdf9f10, &[])
    }
}
port!(th_hitman_diversion_get_title, f_hitman_diversion_get_title, c_hitman_diversion_get_title, "ret");
// 0x006124c0 .. 0x006124e7 (39 bytes)
const B_hostage_diversion_get_start_message: [u8; 39] = [0x56, 0x57, 0xBF, 0xC8, 0x53, 0xDF, 0x00, 0xBE, 0x28, 0x54, 0xDF, 0x00, 0xE8, 0x0F, 0x25, 0x1E, 0x00, 0x8B, 0x74, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x0C, 0x50, 0xE8, 0x61, 0x27, 0x60, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_hostage_diversion_get_start_message(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = lstr_get(esi=0xdf5428, edi=0xdf53c8); then esi = arg1, edi = arg0, snwprintf(eax = the string)
    let s = cr(0x7f49e0, r.eax, r.ecx, r.edx, 0xdf5428, 0xdf53c8, &[]);
    cr(0xc14c40, s, r.ecx, r.edx, arg(a, 1), arg(a, 0), &[s])
    }
}
port!(th_hostage_diversion_get_start_message, f_hostage_diversion_get_start_message, c_hostage_diversion_get_start_message, "ret 8");
// 0x00614270 .. 0x0061428c (28 bytes)
const B_mugging_diversion_get_start_message: [u8; 28] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x68, 0x97, 0x57, 0xDC, 0x00, 0xE8, 0xCC, 0x08, 0x60, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mugging_diversion_get_start_message(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = arg0, edi = arg1, util_sprintf-like(0xc14b50)(0xdc5797)
    cr(0xc14b50, r.eax, r.ecx, r.edx, arg(a, 0), arg(a, 1), &[0xdc5797])
    }
}
port!(th_mugging_diversion_get_start_message, f_mugging_diversion_get_start_message, c_mugging_diversion_get_start_message, "ret 8");
// 0x00612c50 .. 0x00612c7d (45 bytes)
const B_hostage_diversion_create_instance: [u8; 45] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x6A, 0x5C, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x15, 0x83, 0xC9, 0xFF, 0xC7, 0x00, 0x14, 0x57, 0xDF, 0x00, 0x89, 0x48, 0x24, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_hostage_diversion_create_instance(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // ecx = allocator; call [allocator.vtbl + 0x1c](size, 0); on success init vtbl + three -1 fields
    let alloc = arg(a, 0);
    let vt = rd(alloc as usize);
    let f = rd(vt as usize + 0x1c);
    r.ecx = alloc;
    r.edx = f;
    let p = crp(f, vt, alloc, f, r.esi, r.edi, &[0x5c, 0]);
    if p == 0 {
        return 0;
    }
    r.ecx = 0xFFFF_FFFF;
    wd(p as usize, 0xdf5714);
    wd(p as usize + 0x24, 0xFFFF_FFFF);
    wd(p as usize + 0x28, 0xFFFF_FFFF);
    wd(p as usize + 0x2c, 0xFFFF_FFFF);
    p
    }
}
port!(th_hostage_diversion_create_instance, f_hostage_diversion_create_instance, c_hostage_diversion_create_instance, "ret 4");
// 0x006357d0 .. 0x00635809 (57 bytes)
const B_poker_diversion_create_instance: [u8; 57] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x68, 0x68, 0x01, 0x00, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x1E, 0x83, 0xC9, 0xFF, 0xC7, 0x00, 0xB4, 0x99, 0xDF, 0x00, 0x89, 0x88, 0x0C, 0x01, 0x00, 0x00, 0x89, 0x88, 0x10, 0x01, 0x00, 0x00, 0x89, 0x88, 0x14, 0x01, 0x00, 0x00, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_poker_diversion_create_instance(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // ecx = allocator; call [allocator.vtbl + 0x1c](size, 0); on success init vtbl + three -1 fields
    let alloc = arg(a, 0);
    let vt = rd(alloc as usize);
    let f = rd(vt as usize + 0x1c);
    r.ecx = alloc;
    r.edx = f;
    let p = crp(f, vt, alloc, f, r.esi, r.edi, &[0x168, 0]);
    if p == 0 {
        return 0;
    }
    r.ecx = 0xFFFF_FFFF;
    wd(p as usize, 0xdf99b4);
    wd(p as usize + 0x10c, 0xFFFF_FFFF);
    wd(p as usize + 0x110, 0xFFFF_FFFF);
    wd(p as usize + 0x114, 0xFFFF_FFFF);
    p
    }
}
port!(th_poker_diversion_create_instance, f_poker_diversion_create_instance, c_poker_diversion_create_instance, "ret 4");
// 0x00575290 .. 0x00575295 (5 bytes)
const B_heli_info_load_from_table: [u8; 5] = [0xE9, 0x2B, 0x85, 0xFC, 0xFF];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_heli_info_load_from_table(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // jmp 0x53d7c0: the callee returns to our caller with its own ret 8
    cr(0x53d7c0, r.eax, r.ecx, r.edx, r.esi, r.edi, &[arg(a, 0), arg(a, 1)])
    }
}
port!(th_heli_info_load_from_table, f_heli_info_load_from_table, c_heli_info_load_from_table, "ret 8");
// 0x00591a70 .. 0x00591a8e (30 bytes)
const B_piracy_info_load_from_table: [u8; 30] = [0x8B, 0x44, 0x24, 0x08, 0x56, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x08, 0x50, 0x51, 0x8B, 0xCE, 0xE8, 0x3C, 0xBD, 0xFA, 0xFF, 0x89, 0x35, 0x80, 0x75, 0x52, 0x02, 0x5E, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_piracy_info_load_from_table(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // eax = arg1; this -> activity_info::load_from_table(arg0, arg1); [0x2527580] = this
    let this = r.ecx;
    let ret = cr(0x53d7c0, arg(a, 1), this, r.edx, this, r.edi, &[arg(a, 0), arg(a, 1)]);
    wd(ga(0x2527580), this);
    ret
    }
}
port!(th_piracy_info_load_from_table, f_piracy_info_load_from_table, c_piracy_info_load_from_table, "ret 8");
// 0x0066e710 .. 0x0066e738 (40 bytes)
const B_near_miss_complication_load_from_table: [u8; 40] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x56, 0x8B, 0xF9, 0xE8, 0x72, 0xF4, 0xFF, 0xFF, 0x56, 0x8D, 0x77, 0x24, 0xB9, 0xE8, 0xD1, 0xDD, 0x00, 0xE8, 0xA4, 0x60, 0x50, 0x00, 0x83, 0xC4, 0x04, 0xC6, 0x47, 0x28, 0x00, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_near_miss_complication_load_from_table(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // drive_complication::load_from_table(arg0); xtbl_get_int(ecx = 0xddd1e8, esi = this+0x24, arg0); this[0x28] = 0
    let this = r.ecx;
    let x = arg(a, 0);
    let ret = cr(0x66db90, r.eax, this, r.edx, x, this, &[x]);
    let ret = cr(0xb747d0, ret, 0xddd1e8, r.edx, this + 0x24, this, &[x]);
    wb(this as usize + 0x28, 0);
    r.ecx = 0xddd1e8;
    ret
    }
}
port!(th_near_miss_complication_load_from_table, f_near_miss_complication_load_from_table, c_near_miss_complication_load_from_table, "ret 4");
// 0x00c00c40 .. 0x00c00c47 (7 bytes)
const B_mempool_base_dtor: [u8; 7] = [0xC7, 0x01, 0x8C, 0x8C, 0xE5, 0x00, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mempool_base_dtor(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    wd(r.ecx as usize, 0xe58c8c);
    r.eax
    }
}
port!(th_mempool_base_dtor, f_mempool_base_dtor, c_mempool_base_dtor, "ret");
// 0x00c1d160 .. 0x00c1d168 (8 bytes)
const B_page_mempool_size: [u8; 8] = [0x8B, 0x41, 0x50, 0x0F, 0xAF, 0x41, 0x4C, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_size(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    rd(r.ecx as usize + 0x50).wrapping_mul(rd(r.ecx as usize + 0x4c))
    }
}
port!(th_page_mempool_size, f_page_mempool_size, c_page_mempool_size, "ret");
// 0x00c1d170 .. 0x00c1d178 (8 bytes)
const B_page_mempool_used_bytes: [u8; 8] = [0x8B, 0x41, 0x44, 0x0F, 0xAF, 0x41, 0x4C, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_used_bytes(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    rd(r.ecx as usize + 0x44).wrapping_mul(rd(r.ecx as usize + 0x4c))
    }
}
port!(th_page_mempool_used_bytes, f_page_mempool_used_bytes, c_page_mempool_used_bytes, "ret");
// 0x00c1d110 .. 0x00c1d115 (5 bytes)
const B_page_mempool_align_to: [u8; 5] = [0x32, 0xC0, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_align_to(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    lo8(r.eax, 0)
    }
}
port!(th_page_mempool_align_to, f_page_mempool_align_to, c_page_mempool_align_to, "ret 4");
// 0x00c1d120 .. 0x00c1d125 (5 bytes)
const B_page_mempool_release: [u8; 5] = [0x32, 0xC0, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_release(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    lo8(r.eax, 0)
    }
}
port!(th_page_mempool_release, f_page_mempool_release, c_page_mempool_release, "ret 4");
// 0x00c1d130 .. 0x00c1d135 (5 bytes)
const B_page_mempool_release_to: [u8; 5] = [0x32, 0xC0, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_release_to(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    lo8(r.eax, 0)
    }
}
port!(th_page_mempool_release_to, f_page_mempool_release_to, c_page_mempool_release_to, "ret 4");
// 0x007617e0 .. 0x007617ec (12 bytes)
const B_ovl_text_entry_hide: [u8; 12] = [0x8B, 0x41, 0x08, 0x68, 0x50, 0x80, 0x52, 0x02, 0xFF, 0xD0, 0x59, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_text_entry_hide(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // eax = [this+8]; push 0x2528050; call eax; pop ecx (the cdecl argument slot comes back)
    let f = rd(r.ecx as usize + 8);
    let ret = crp(f, f, r.ecx, r.edx, r.esi, r.edi, &[0x2528050]);
    r.ecx = 0x2528050;
    ret
    }
}
port!(th_ovl_text_entry_hide, f_ovl_text_entry_hide, c_ovl_text_entry_hide, "ret");
// 0x007617f0 .. 0x00761830 (64 bytes)
const B_ovl_text_entry_update: [u8; 64] = [0x56, 0x8B, 0xF1, 0xE8, 0xB8, 0xEA, 0xD9, 0xFF, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x38, 0x46, 0x0C, 0x75, 0x18, 0x80, 0x3D, 0xCD, 0x8B, 0x52, 0x02, 0x00, 0x75, 0x09, 0x80, 0x3D, 0x38, 0x80, 0x52, 0x02, 0x00, 0x74, 0x1A, 0xC6, 0x46, 0x0C, 0x00, 0x5E, 0xC3, 0x80, 0x3D, 0xCD, 0x8B, 0x52, 0x02, 0x00, 0x75, 0x0B, 0x80, 0x3D, 0x38, 0x80, 0x52, 0x02, 0x00, 0x75, 0x02, 0x33, 0xC0, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_text_entry_update(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let ax = cr(0x5002b0, r.eax, this, r.edx, this, r.edi, &[]);
    let _ = ax;
    r.eax = 1;
    let b = rb(ga(0x2528bcd)) != 0 || rb(ga(0x2528038)) != 0;
    if rb(this as usize + 0xc) == 1 {
        if b {
            wb(this as usize + 0xc, 0);
        }
        return 1;
    }
    if b {
        return 1;
    }
    0
    }
}
port!(th_ovl_text_entry_update, f_ovl_text_entry_update, c_ovl_text_entry_update, "ret");
// 0x00503090 .. 0x005030c1 (49 bytes)
const B_ovl_framerate_hide: [u8; 49] = [0x56, 0x8B, 0x35, 0x54, 0xF5, 0xF9, 0x00, 0xE8, 0x84, 0x26, 0x71, 0x00, 0x6A, 0x75, 0xB9, 0x90, 0x01, 0xDE, 0x00, 0xE8, 0x88, 0x68, 0x23, 0x00, 0x83, 0xC4, 0x04, 0xBE, 0x07, 0x00, 0x00, 0x00, 0xC7, 0x05, 0x4C, 0xF5, 0xF9, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x51, 0x68, 0x19, 0x00, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_framerate_hide(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let esi = rd(ga(0xf9f554));
    let e1 = cr(0xc15720, r.eax, r.ecx, r.edx, esi, r.edi, &[]);
    let e2 = cr(0x739930, e1, 0xde0190, r.edx, esi, r.edi, &[0x75]);
    let _ = e2;
    wd(ga(0xf9f54c), 0);
    r.ecx = 0xde0190;
    cr(0x699910, e2, 0xde0190, r.edx, 7, r.edi, &[])
    }
}
port!(th_ovl_framerate_hide, f_ovl_framerate_hide, c_ovl_framerate_hide, "ret");
// 0x007612f0 .. 0x00761309 (25 bytes)
const B_ovl_selection_menu_hide: [u8; 25] = [0xE8, 0xEB, 0x70, 0xFE, 0xFF, 0xE8, 0x36, 0x66, 0xFE, 0xFF, 0x6A, 0x00, 0x6A, 0x01, 0x33, 0xC0, 0xE8, 0x2B, 0xF4, 0xFD, 0xFF, 0x83, 0xC4, 0x08, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_selection_menu_hide(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let e1 = cr(0x7483e0, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
    let e2 = cr(0x747930, e1, r.ecx, r.edx, r.esi, r.edi, &[]);
    let _ = e2;
    cr(0x740730, 0, r.ecx, r.edx, r.esi, r.edi, &[1, 0])
    }
}
port!(th_ovl_selection_menu_hide, f_ovl_selection_menu_hide, c_ovl_selection_menu_hide, "ret");
// 0x006f4ee0 .. 0x006f4f1f (63 bytes)
const B_havok_all_ray_hit_collector_sdd: [u8; 63] = [0x56, 0x8B, 0xF1, 0xE8, 0x38, 0x00, 0x00, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x2A, 0x85, 0xF6, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x62, 0x99, 0x5E, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_all_ray_hit_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let ax = cr(0x6f4f20, r.eax, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 && this != 0 {
        let flag = rb(ga(0x2527bf7)) as u32;
        if flag != 1 {
            crp(rd(ga(0xdb80cc)), lo8(ax, flag), r.ecx, r.edx, this, r.edi, &[0x347ade4]);
            let e = cr(0xcde870, this, r.ecx, r.edx, this, r.edi, &[]);
            crp(rd(ga(0xdb80d0)), e, r.ecx, r.edx, this, r.edi, &[0x347ade4]);
        }
    }
    this
    }
}
port!(th_havok_all_ray_hit_collector_sdd, f_havok_all_ray_hit_collector_sdd, c_havok_all_ray_hit_collector_sdd, "ret 4");
// 0x006f5050 .. 0x006f5098 (72 bytes)
const B_closest_linear_cast_collector_sdd: [u8; 72] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xEC, 0x8A, 0xE1, 0x00, 0x74, 0x32, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x8B, 0x48, 0x64, 0x3B, 0x48, 0x34, 0x7C, 0x12, 0x6A, 0x1F, 0x56, 0x6A, 0x05, 0x8B, 0xC8, 0xE8, 0x10, 0x63, 0x63, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00, 0x8B, 0x50, 0x60, 0x83, 0x40, 0x64, 0x01, 0x89, 0x16, 0x89, 0x70, 0x60, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_closest_linear_cast_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    wd(this as usize, 0xe18aec);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let cnt = rd(tm as usize + 0x64);
        r.ecx = cnt;
        if (cnt as i32) >= (rd(tm as usize + 0x34) as i32) {
            r.ecx = tm;
            cr(0xd2b390, tm, tm, r.edx, this, r.edi, &[0x5, this, 0x1f]);
        } else {
            let head = rd(tm as usize + 0x60);
            r.edx = head;
            wd(tm as usize + 0x64, cnt.wrapping_add(1));
            wd(this as usize, head);
            wd(tm as usize + 0x60, this);
        }
    }
    this
    }
}
port!(th_closest_linear_cast_collector_sdd, f_closest_linear_cast_collector_sdd, c_closest_linear_cast_collector_sdd, "ret 4");
// 0x006fdf20 .. 0x006fdf68 (72 bytes)
const B_object_collector_sdd: [u8; 72] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xF8, 0x8A, 0xE1, 0x00, 0x74, 0x32, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x8B, 0x48, 0x44, 0x3B, 0x48, 0x34, 0x7C, 0x12, 0x6A, 0x1F, 0x56, 0x6A, 0x01, 0x8B, 0xC8, 0xE8, 0x40, 0xD4, 0x62, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00, 0x8B, 0x50, 0x40, 0x83, 0x40, 0x44, 0x01, 0x89, 0x16, 0x89, 0x70, 0x40, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_object_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    wd(this as usize, 0xe18af8);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let cnt = rd(tm as usize + 0x44);
        r.ecx = cnt;
        if (cnt as i32) >= (rd(tm as usize + 0x34) as i32) {
            r.ecx = tm;
            cr(0xd2b390, tm, tm, r.edx, this, r.edi, &[0x1, this, 0x1f]);
        } else {
            let head = rd(tm as usize + 0x40);
            r.edx = head;
            wd(tm as usize + 0x44, cnt.wrapping_add(1));
            wd(this as usize, head);
            wd(tm as usize + 0x40, this);
        }
    }
    this
    }
}
port!(th_object_collector_sdd, f_object_collector_sdd, c_object_collector_sdd, "ret 4");
// 0x0072b150 .. 0x0072b1a7 (87 bytes)
const B_custom_character_collector_sdd: [u8; 87] = [0x56, 0x8B, 0xF1, 0xE8, 0x58, 0x00, 0x00, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x42, 0x85, 0xF6, 0x74, 0x3E, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x8B, 0x88, 0xAC, 0x00, 0x00, 0x00, 0x3B, 0x48, 0x34, 0x7C, 0x12, 0x6A, 0x1F, 0x56, 0x6A, 0x0E, 0x8B, 0xC8, 0xE8, 0x0A, 0x02, 0x60, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00, 0x8B, 0x90, 0xA8, 0x00, 0x00, 0x00, 0x83, 0x80, 0xAC, 0x00, 0x00, 0x00, 0x01, 0x89, 0x16, 0x89, 0xB0, 0xA8, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_custom_character_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let _ = cr(0x72b1b0, r.eax, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 && this != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let cnt = rd(tm as usize + 0xac);
        r.ecx = cnt;
        if (cnt as i32) >= (rd(tm as usize + 0x34) as i32) {
            r.ecx = tm;
            cr(0xd2b390, tm, tm, r.edx, this, r.edi, &[0xe, this, 0x1f]);
        } else {
            let head = rd(tm as usize + 0xa8);
            r.edx = head;
            wd(tm as usize + 0xac, cnt.wrapping_add(1));
            wd(this as usize, head);
            wd(tm as usize + 0xa8, this);
        }
    }
    this
    }
}
port!(th_custom_character_collector_sdd, f_custom_character_collector_sdd, c_custom_character_collector_sdd, "ret 4");
// 0x006f51c0 .. 0x006f5217 (87 bytes)
const B_all_linear_cast_collector_sdd: [u8; 87] = [0x56, 0x8B, 0xF1, 0xE8, 0x58, 0x00, 0x00, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x42, 0x85, 0xF6, 0x74, 0x3E, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x8B, 0x88, 0x9C, 0x00, 0x00, 0x00, 0x3B, 0x48, 0x34, 0x7C, 0x12, 0x6A, 0x1F, 0x56, 0x6A, 0x0C, 0x8B, 0xC8, 0xE8, 0x9A, 0x61, 0x63, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00, 0x8B, 0x90, 0x98, 0x00, 0x00, 0x00, 0x83, 0x80, 0x9C, 0x00, 0x00, 0x00, 0x01, 0x89, 0x16, 0x89, 0xB0, 0x98, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_all_linear_cast_collector_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let _ = cr(0x6f5220, r.eax, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 && this != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let cnt = rd(tm as usize + 0x9c);
        r.ecx = cnt;
        if (cnt as i32) >= (rd(tm as usize + 0x34) as i32) {
            r.ecx = tm;
            cr(0xd2b390, tm, tm, r.edx, this, r.edi, &[0xc, this, 0x1f]);
        } else {
            let head = rd(tm as usize + 0x98);
            r.edx = head;
            wd(tm as usize + 0x9c, cnt.wrapping_add(1));
            wd(this as usize, head);
            wd(tm as usize + 0x98, this);
        }
    }
    this
    }
}
port!(th_all_linear_cast_collector_sdd, f_all_linear_cast_collector_sdd, c_all_linear_cast_collector_sdd, "ret 4");
// 0x0070efe0 .. 0x0070f012 (50 bytes)
const B_havok_game_filter_sdd: [u8; 50] = [0x56, 0x8B, 0xF1, 0x8B, 0xC6, 0xE8, 0x36, 0x00, 0x00, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x1B, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x0F, 0xB7, 0x4E, 0x04, 0x6A, 0x27, 0x51, 0x56, 0x8B, 0xC8, 0xE8, 0xC4, 0xC7, 0x61, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_game_filter_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let _ = cr(0x70f020, this, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let size = rd(this as usize + 4) & 0xFFFF;
        r.ecx = tm;
        cr(0xd2b7d0, tm, tm, r.edx, this, r.edi, &[this, size, 0x27]);
    }
    this
    }
}
port!(th_havok_game_filter_sdd, f_havok_game_filter_sdd, c_havok_game_filter_sdd, "ret 4");
// 0x00ba9de0 .. 0x00ba9e12 (50 bytes)
const B_never_collide_filter_sdd: [u8; 50] = [0x56, 0x8B, 0xF1, 0x8B, 0xC6, 0xE8, 0x36, 0x52, 0xB6, 0xFF, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x1B, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x0F, 0xB7, 0x4E, 0x04, 0x6A, 0x27, 0x51, 0x56, 0x8B, 0xC8, 0xE8, 0xC4, 0x19, 0x18, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_never_collide_filter_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let _ = cr(0x70f020, this, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let size = rd(this as usize + 4) & 0xFFFF;
        r.ecx = tm;
        cr(0xd2b7d0, tm, tm, r.edx, this, r.edi, &[this, size, 0x27]);
    }
    this
    }
}
port!(th_never_collide_filter_sdd, f_never_collide_filter_sdd, c_never_collide_filter_sdd, "ret 4");
// 0x0070e900 .. 0x0070e930 (48 bytes)
const B_havok_filter_shape_collection_sdd: [u8; 48] = [0x56, 0x8B, 0xF1, 0xE8, 0x28, 0x25, 0x63, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x1B, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x0F, 0xB7, 0x4E, 0x04, 0x6A, 0x27, 0x51, 0x56, 0x8B, 0xC8, 0xE8, 0xA6, 0xCE, 0x61, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_filter_shape_collection_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let _ = cr(0xd40e30, r.eax, this, r.edx, this, r.edi, &[]);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let size = rd(this as usize + 4) & 0xFFFF;
        r.ecx = tm;
        cr(0xd2b7d0, tm, tm, r.edx, this, r.edi, &[this, size, 0x27]);
    }
    this
    }
}
port!(th_havok_filter_shape_collection_sdd, f_havok_filter_shape_collection_sdd, c_havok_filter_shape_collection_sdd, "ret 4");
// 0x00710430 .. 0x00710472 (66 bytes)
const B_havok_navmesh_shape_sdd: [u8; 66] = [0x56, 0x8B, 0xF1, 0x85, 0xF6, 0x74, 0x05, 0x8D, 0x46, 0x10, 0xEB, 0x02, 0x33, 0xC0, 0xF6, 0x44, 0x24, 0x08, 0x01, 0xC7, 0x00, 0x38, 0x8D, 0xE1, 0x00, 0xC7, 0x06, 0x4C, 0xE4, 0xDB, 0x00, 0x74, 0x1B, 0xA1, 0xBC, 0xF2, 0x1B, 0x03, 0x50, 0xFF, 0x15, 0x7C, 0x80, 0xDB, 0x00, 0x0F, 0xB7, 0x4E, 0x04, 0x6A, 0x27, 0x51, 0x56, 0x8B, 0xC8, 0xE8, 0x64, 0xB3, 0x61, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_navmesh_shape_sdd(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    // eax = this ? this + 0x10 : 0; [eax] = 0xe18d38; [this] = 0xdbe44c (a null this would fault)
    if this == 0 {
        return 0;
    }
    wd(this as usize + 0x10, 0xe18d38);
    wd(this as usize, 0xdbe44c);
    if arg(a, 0) & 1 != 0 {
        let tls = rd(ga(0x31bf2bc));
        let tm = crp(rd(ga(0xdb807c)), tls, r.ecx, r.edx, this, r.edi, &[tls]);
        let size = rd(this as usize + 4) & 0xFFFF;
        r.ecx = tm;
        cr(0xd2b7d0, tm, tm, r.edx, this, r.edi, &[this, size, 0x27]);
    }
    this
    }
}
port!(th_havok_navmesh_shape_sdd, f_havok_navmesh_shape_sdd, c_havok_navmesh_shape_sdd, "ret 4");
// 0x008592b0 .. 0x008592f4 (68 bytes)
const B_npc_orientation_heading_compress: [u8; 68] = [0x57, 0x8B, 0x7C, 0x24, 0x08, 0x85, 0xFF, 0x74, 0x37, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x85, 0xF6, 0x74, 0x2D, 0x8B, 0x54, 0x24, 0x18, 0x85, 0xD2, 0x74, 0x25, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xC0, 0x14, 0xEB, 0x02, 0x33, 0xC0, 0x53, 0x8B, 0x59, 0x04, 0x83, 0xC2, 0x14, 0x52, 0x50, 0x8B, 0x03, 0x83, 0xC1, 0x04, 0x83, 0xC6, 0x14, 0x56, 0x57, 0xFF, 0xD0, 0x5B, 0x5E, 0x5F, 0xC2, 0x10, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_orientation_heading_compress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // virtual call [[ecx+4]+0](a0, a1+0x14, a2 ? a2+0x14 : 0, a3+0x14) with ecx = this+4
    let (a0, a1, a2, a3) = (arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3));
    if a0 == 0 || a1 == 0 {
        return r.eax;
    }
    r.edx = a3;
    if a3 == 0 {
        return r.eax;
    }
    let p2 = if a2 != 0 { a2.wrapping_add(0x14) } else { 0 };
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize + 0);
    let c4 = r.ecx.wrapping_add(4);
    let d = a3.wrapping_add(0x14);
    r.ecx = c4;
    r.edx = d;
    crp(f, f, c4, d, a1.wrapping_add(0x14), a0, &[a0, a1.wrapping_add(0x14), p2, d])
    }
}
port!(th_npc_orientation_heading_compress, f_npc_orientation_heading_compress, c_npc_orientation_heading_compress, "ret 16");
// 0x008594e0 .. 0x00859524 (68 bytes)
const B_npc_orientation_quaternion_compress: [u8; 68] = [0x57, 0x8B, 0x7C, 0x24, 0x08, 0x85, 0xFF, 0x74, 0x37, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x85, 0xF6, 0x74, 0x2D, 0x8B, 0x54, 0x24, 0x18, 0x85, 0xD2, 0x74, 0x25, 0x8B, 0x44, 0x24, 0x14, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xC0, 0x14, 0xEB, 0x02, 0x33, 0xC0, 0x53, 0x8B, 0x59, 0x04, 0x83, 0xC2, 0x14, 0x52, 0x50, 0x8B, 0x03, 0x83, 0xC1, 0x04, 0x83, 0xC6, 0x14, 0x56, 0x57, 0xFF, 0xD0, 0x5B, 0x5E, 0x5F, 0xC2, 0x10, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_orientation_quaternion_compress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // virtual call [[ecx+4]+0](a0, a1+0x14, a2 ? a2+0x14 : 0, a3+0x14) with ecx = this+4
    let (a0, a1, a2, a3) = (arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3));
    if a0 == 0 || a1 == 0 {
        return r.eax;
    }
    r.edx = a3;
    if a3 == 0 {
        return r.eax;
    }
    let p2 = if a2 != 0 { a2.wrapping_add(0x14) } else { 0 };
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize + 0);
    let c4 = r.ecx.wrapping_add(4);
    let d = a3.wrapping_add(0x14);
    r.ecx = c4;
    r.edx = d;
    crp(f, f, c4, d, a1.wrapping_add(0x14), a0, &[a0, a1.wrapping_add(0x14), p2, d])
    }
}
port!(th_npc_orientation_quaternion_compress, f_npc_orientation_quaternion_compress, c_npc_orientation_quaternion_compress, "ret 16");
// 0x00859300 .. 0x00859337 (55 bytes)
const B_npc_orientation_heading_decompress: [u8; 55] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x85, 0xF6, 0x74, 0x2A, 0x8B, 0x54, 0x24, 0x0C, 0x85, 0xD2, 0x74, 0x22, 0x8B, 0x44, 0x24, 0x10, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xC0, 0x14, 0xEB, 0x02, 0x33, 0xC0, 0x57, 0x8B, 0x79, 0x04, 0x50, 0x8B, 0x47, 0x04, 0x83, 0xC1, 0x04, 0x83, 0xC2, 0x14, 0x52, 0x56, 0xFF, 0xD0, 0x5F, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_orientation_heading_decompress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // virtual call [[ecx+4]+4](a0, a1+0x14, a2 ? a2+0x14 : 0) with ecx = this+4
    let (a0, a1, a2) = (arg(a, 0), arg(a, 1), arg(a, 2));
    if a0 == 0 {
        return r.eax;
    }
    r.edx = a1;
    if a1 == 0 {
        return r.eax;
    }
    let p2 = if a2 != 0 { a2.wrapping_add(0x14) } else { 0 };
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize + 4);
    let c4 = r.ecx.wrapping_add(4);
    let d = a1.wrapping_add(0x14);
    r.ecx = c4;
    r.edx = d;
    r.eax = f;
    crp(f, f, c4, d, a0, vt, &[a0, d, p2])
    }
}
port!(th_npc_orientation_heading_decompress, f_npc_orientation_heading_decompress, c_npc_orientation_heading_decompress, "ret 12");
// 0x00859530 .. 0x00859567 (55 bytes)
const B_npc_orientation_quaternion_decompress: [u8; 55] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x85, 0xF6, 0x74, 0x2A, 0x8B, 0x54, 0x24, 0x0C, 0x85, 0xD2, 0x74, 0x22, 0x8B, 0x44, 0x24, 0x10, 0x85, 0xC0, 0x74, 0x05, 0x83, 0xC0, 0x14, 0xEB, 0x02, 0x33, 0xC0, 0x57, 0x8B, 0x79, 0x04, 0x50, 0x8B, 0x47, 0x04, 0x83, 0xC1, 0x04, 0x83, 0xC2, 0x14, 0x52, 0x56, 0xFF, 0xD0, 0x5F, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_orientation_quaternion_decompress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // virtual call [[ecx+4]+4](a0, a1+0x14, a2 ? a2+0x14 : 0) with ecx = this+4
    let (a0, a1, a2) = (arg(a, 0), arg(a, 1), arg(a, 2));
    if a0 == 0 {
        return r.eax;
    }
    r.edx = a1;
    if a1 == 0 {
        return r.eax;
    }
    let p2 = if a2 != 0 { a2.wrapping_add(0x14) } else { 0 };
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize + 4);
    let c4 = r.ecx.wrapping_add(4);
    let d = a1.wrapping_add(0x14);
    r.ecx = c4;
    r.edx = d;
    r.eax = f;
    crp(f, f, c4, d, a0, vt, &[a0, d, p2])
    }
}
port!(th_npc_orientation_quaternion_decompress, f_npc_orientation_quaternion_decompress, c_npc_orientation_quaternion_decompress, "ret 12");
// 0x008595e0 .. 0x00859611 (49 bytes)
const B_npc_spline_pathfind_uncompressed_compress: [u8; 49] = [0x8B, 0x41, 0x04, 0x8B, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x14, 0x8D, 0x56, 0x40, 0x52, 0x8B, 0x54, 0x24, 0x10, 0x6A, 0x00, 0x83, 0xC2, 0x40, 0x83, 0xC1, 0x04, 0x52, 0x8B, 0x54, 0x24, 0x14, 0x52, 0xFF, 0xD0, 0x33, 0xC0, 0xE8, 0x97, 0x81, 0xFF, 0xFF, 0xC6, 0x46, 0x3C, 0x01, 0x5E, 0xC2, 0x10, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_spline_pathfind_uncompressed_compress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // [[this+4]](a0, a1+0x40, 0, a3+0x40) with ecx = this+4, edx = a0, esi = a3; then (eax = 0) 0x8517a0; a3[0x3c] = 1
    let (a0, a1, a3) = (arg(a, 0), arg(a, 1), arg(a, 3));
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize);
    let c4 = r.ecx.wrapping_add(4);
    r.ecx = c4;
    r.edx = a0;
    crp(f, f, c4, a0, a3, r.edi, &[a0, a1.wrapping_add(0x40), 0, a3.wrapping_add(0x40)]);
    let ret = cr(0x8517a0, 0, c4, a0, a3, r.edi, &[]);
    wb(a3 as usize + 0x3c, 1);
    ret
    }
}
port!(th_npc_spline_pathfind_uncompressed_compress, f_npc_spline_pathfind_uncompressed_compress, c_npc_spline_pathfind_uncompressed_compress, "ret 16");
// 0x00859620 .. 0x00859646 (38 bytes)
const B_npc_spline_pathfind_uncompressed_decompress: [u8; 38] = [0x8B, 0x41, 0x04, 0x8B, 0x40, 0x04, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x6A, 0x00, 0x8D, 0x56, 0x40, 0x83, 0xC1, 0x04, 0x52, 0x8B, 0x54, 0x24, 0x10, 0x52, 0xFF, 0xD0, 0x33, 0xC0, 0xE8, 0x5E, 0x81, 0xFF, 0xFF, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_spline_pathfind_uncompressed_decompress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // [[this+4]+4](a0, a1+0x40, 0) with ecx = this+4, edx = a0, esi = a1; then (eax = 0) 0x8517a0
    let (a0, a1) = (arg(a, 0), arg(a, 1));
    let vt = rd(r.ecx as usize + 4);
    let f = rd(vt as usize + 4);
    let c4 = r.ecx.wrapping_add(4);
    r.ecx = c4;
    r.edx = a0;
    crp(f, f, c4, a0, a1, r.edi, &[a0, a1.wrapping_add(0x40), 0]);
    cr(0x8517a0, 0, c4, a0, a1, r.edi, &[])
    }
}
port!(th_npc_spline_pathfind_uncompressed_decompress, f_npc_spline_pathfind_uncompressed_decompress, c_npc_spline_pathfind_uncompressed_decompress, "ret 12");
// 0x00bf48f0 .. 0x00bf4942 (82 bytes)
const B_mempool_ctor: [u8; 82] = [0x56, 0x8B, 0xF1, 0x6A, 0x20, 0x8D, 0x46, 0x0C, 0x68, 0xB8, 0x92, 0xE5, 0x00, 0x50, 0xC7, 0x06, 0x8C, 0x8C, 0xE5, 0x00, 0xE8, 0xE7, 0x47, 0x0A, 0x00, 0x33, 0xC0, 0x88, 0x46, 0x2B, 0x89, 0x46, 0x30, 0x88, 0x46, 0x2C, 0x89, 0x46, 0x08, 0x88, 0x46, 0x04, 0xC7, 0x06, 0x84, 0x94, 0xE5, 0x00, 0x89, 0x46, 0x38, 0x89, 0x46, 0x40, 0x89, 0x46, 0x3C, 0x89, 0x46, 0x48, 0x89, 0x46, 0x44, 0xC7, 0x06, 0xD4, 0x8C, 0xE5, 0x00, 0x89, 0x46, 0x4C, 0x89, 0x46, 0x50, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mempool_ctor(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // push esi; esi = this; memset-like init 0xc990f0(this+0xc, 0xe592b8, 0x20)
    let this = r.ecx;
    wd(this as usize, 0xe58c8c);
    cr(0xc990f0, this + 0xc, this, r.edx, this, r.edi, &[this + 0xc, 0xe592b8, 0x20]);
    let t = this as usize;
    wb(t + 0x2b, 0);
    wd(t + 0x30, 0);
    wb(t + 0x2c, 0);
    wd(t + 8, 0);
    wb(t + 4, 0);
    wd(t, 0xe59484);
    wd(t + 0x38, 0);
    wd(t + 0x40, 0);
    wd(t + 0x3c, 0);
    wd(t + 0x48, 0);
    wd(t + 0x44, 0);
    wd(t, 0xe58cd4);
    wd(t + 0x4c, 0);
    wd(t + 0x50, 0);
    this
    }
}
port!(th_mempool_ctor, f_mempool_ctor, c_mempool_ctor, "ret");
// 0x00bf4990 .. 0x00bf49da (74 bytes)
const B_mempool_ctor_2: [u8; 74] = [0x6A, 0x20, 0x50, 0x8D, 0x4E, 0x0C, 0x51, 0xC7, 0x06, 0x8C, 0x8C, 0xE5, 0x00, 0xE8, 0x4E, 0x47, 0x0A, 0x00, 0x33, 0xC0, 0x88, 0x46, 0x2B, 0x89, 0x46, 0x30, 0x88, 0x46, 0x2C, 0x89, 0x46, 0x08, 0x88, 0x46, 0x04, 0xC7, 0x06, 0x84, 0x94, 0xE5, 0x00, 0x89, 0x46, 0x38, 0x89, 0x46, 0x40, 0x89, 0x46, 0x3C, 0x89, 0x46, 0x48, 0x89, 0x46, 0x44, 0xC7, 0x06, 0xD4, 0x8C, 0xE5, 0x00, 0x89, 0x46, 0x4C, 0x89, 0x46, 0x50, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mempool_ctor_2(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = this, eax = the second argument of 0xc990f0(this+0xc, eax, 0x20)
    let this = r.esi;
    wd(this as usize, 0xe58c8c);
    cr(0xc990f0, r.eax, this + 0xc, r.edx, this, r.edi, &[this + 0xc, r.eax, 0x20]);
    let t = this as usize;
    wb(t + 0x2b, 0);
    wd(t + 0x30, 0);
    wb(t + 0x2c, 0);
    wd(t + 8, 0);
    wb(t + 4, 0);
    wd(t, 0xe59484);
    wd(t + 0x38, 0);
    wd(t + 0x40, 0);
    wd(t + 0x3c, 0);
    wd(t + 0x48, 0);
    wd(t + 0x44, 0);
    wd(t, 0xe58cd4);
    wd(t + 0x4c, 0);
    wd(t + 0x50, 0);
    r.ecx = this + 0xc;
    this
    }
}
port!(th_mempool_ctor_2, f_mempool_ctor_2, c_mempool_ctor_2, "ret");
// 0x00bf49e0 .. 0x00bf4a00 (32 bytes)
const B_mempool_ctor_3: [u8; 32] = [0x8B, 0x54, 0x24, 0x04, 0x50, 0x51, 0x57, 0x52, 0x56, 0xE8, 0xC2, 0xC1, 0x00, 0x00, 0xC7, 0x06, 0xD4, 0x8C, 0xE5, 0x00, 0x89, 0x7E, 0x4C, 0x89, 0x7E, 0x50, 0x8B, 0xC6, 0xC2, 0x04, 0x00, 0xCC];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mempool_ctor_3(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = this, edi, eax, ecx live; stack arg0 -> 0xc00bb0(this, arg0, edi, ecx, eax)
    let this = r.esi;
    let x = arg(a, 0);
    r.edx = x;
    cr(0xc00bb0, r.eax, r.ecx, x, this, r.edi, &[this, x, r.edi, r.ecx, r.eax]);
    wd(this as usize, 0xe58cd4);
    wd(this as usize + 0x4c, r.edi);
    wd(this as usize + 0x50, r.edi);
    this
    }
}
port!(th_mempool_ctor_3, f_mempool_ctor_3, c_mempool_ctor_3, "ret 4");
// 0x00c1cc00 .. 0x00c1cc4c (76 bytes)
const B_page_mempool_can_alloc: [u8; 76] = [0x56, 0x57, 0x8D, 0x44, 0x24, 0x10, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x10, 0x50, 0x8B, 0x44, 0x24, 0x10, 0xE8, 0x8A, 0xFF, 0xFF, 0xFF, 0x8B, 0x56, 0x54, 0x83, 0xFA, 0xFF, 0x74, 0x20, 0x8B, 0x7E, 0x50, 0x3B, 0xD7, 0x73, 0x0B, 0x8B, 0x4E, 0x4C, 0x0F, 0xAF, 0xCA, 0x03, 0x4E, 0x38, 0xEB, 0x02, 0x33, 0xC9, 0x39, 0x01, 0x73, 0x0F, 0x8B, 0x51, 0x04, 0x83, 0xFA, 0xFF, 0x75, 0xE3, 0x5F, 0x32, 0xC0, 0x5E, 0xC2, 0x08, 0x00, 0x5F, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_can_alloc(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // size = 0xc1cba0(eax = arg0, ecx = arg1, esi = this, &arg1); walk the free list for a block >= size
    let this = r.ecx;
    let size = cr(0xc1cba0, arg(a, 0), arg(a, 1), r.edx, this, r.edi, &[a.add(1) as u32]);
    let t = this as usize;
    let mut edx = rd(t + 0x54);
    r.ecx = arg(a, 1);
    r.edx = edx;
    if edx == 0xFFFF_FFFF {
        return lo8(size, 0);
    }
    let count = rd(t + 0x50);
    loop {
        let ecx = if edx < count { rd(t + 0x4c).wrapping_mul(edx).wrapping_add(rd(t + 0x38)) } else { 0 };
        r.ecx = ecx;
        if ecx == 0 {
            // the original reads [0] here (a crash); report "no block" instead
            return lo8(size, 0);
        }
        if rd(ecx as usize) >= size {
            return lo8(size, 1);
        }
        edx = rd(ecx as usize + 4);
        r.edx = edx;
        if edx == 0xFFFF_FFFF {
            return lo8(size, 0);
        }
    }
    }
}
port!(th_page_mempool_can_alloc, f_page_mempool_can_alloc, c_page_mempool_can_alloc, "ret 8");
// 0x00c1d000 .. 0x00c1d049 (73 bytes)
const B_page_mempool_reset: [u8; 73] = [0x56, 0x8B, 0xF1, 0x8B, 0x46, 0x3C, 0x8B, 0x4E, 0x38, 0x50, 0x6A, 0x00, 0x51, 0xE8, 0xAE, 0x3B, 0x08, 0x00, 0x8B, 0x46, 0x50, 0x83, 0xC4, 0x0C, 0x85, 0xC0, 0xC7, 0x46, 0x44, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x54, 0x00, 0x00, 0x00, 0x00, 0x76, 0x10, 0x8B, 0x76, 0x38, 0x89, 0x06, 0xC7, 0x46, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xB0, 0x01, 0x5E, 0xC3, 0x33, 0xF6, 0x89, 0x06, 0xC7, 0x46, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xB0, 0x01, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_page_mempool_reset(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // memset(base, 0, size); count/first-free reset; the first block's header = (count, -1)
    let this = r.ecx;
    let t = this as usize;
    let size = rd(t + 0x3c);
    let base = rd(t + 0x38);
    cr(0xca0bc0, size, base, r.edx, this, r.edi, &[base, 0, size]);
    let count = rd(t + 0x50);
    wd(t + 0x44, 0);
    wd(t + 0x54, 0);
    r.ecx = base;
    if count == 0 {
        // the original stores through a null pointer here (a crash)
        return lo8(count, 1);
    }
    wd(base as usize, count);
    wd(base as usize + 4, 0xFFFF_FFFF);
    lo8(count, 1)
    }
}
port!(th_page_mempool_reset, f_page_mempool_reset, c_page_mempool_reset, "ret");
// 0x00925750 .. 0x00925776 (38 bytes)
const B_object_get_attach_point: [u8; 38] = [0x83, 0x7C, 0x24, 0x08, 0xFF, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x04, 0xD9, 0x10, 0xD9, 0x50, 0x04, 0xD9, 0x58, 0x08, 0x75, 0x0E, 0x8B, 0x49, 0x7C, 0x85, 0xC9, 0x7C, 0x07, 0x50, 0xE8, 0x1E, 0x72, 0xDD, 0xFF, 0x59, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_object_get_attach_point(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let a0 = arg(a, 0);
    wd(a0 as usize, 0);
    wd(a0 as usize + 4, 0);
    wd(a0 as usize + 8, 0);
    if arg(a, 1) != 0xFFFF_FFFF {
        return a0;
    }
    let c = rd(r.ecx as usize + 0x7c);
    r.ecx = c;
    if (c as i32) < 0 {
        return a0;
    }
    let ret = cr(0x6fc990, a0, c, r.edx, r.esi, r.edi, &[a0]);
    r.ecx = a0;
    ret
    }
}
port!(th_object_get_attach_point, f_object_get_attach_point, c_object_get_attach_point, "ret 8");
// 0x00933610 .. 0x0093363c (44 bytes)
const B_object_item_activate: [u8; 44] = [0x56, 0x8B, 0xF1, 0x83, 0xBE, 0xCC, 0x00, 0x00, 0x00, 0x00, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x74, 0x0E, 0x8B, 0x8E, 0xCC, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x20, 0x57, 0xFF, 0xD2, 0x57, 0x8B, 0xCE, 0xE8, 0x49, 0x21, 0xFF, 0xFF, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_object_item_activate(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let x = arg(a, 0);
    let sub = rd(this as usize + 0xcc);
    let mut ax = r.eax;
    if sub != 0 {
        let vt = rd(sub as usize);
        let f = rd(vt as usize + 0x20);
        ax = crp(f, vt, sub, f, this, x, &[x]);
        r.edx = f;
    }
    r.ecx = this;
    cr(0x925780, ax, this, r.edx, this, x, &[x])
    }
}
port!(th_object_item_activate, f_object_item_activate, c_object_item_activate, "ret 4");
// 0x0075a7c0 .. 0x0075a805 (69 bytes)
const B_ovl_ingame_main_menu_show: [u8; 69] = [0x56, 0x8B, 0xF1, 0x6A, 0x01, 0xC7, 0x46, 0x08, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x8F, 0x95, 0x03, 0x00, 0xD9, 0x05, 0x50, 0x1C, 0xDD, 0x00, 0xD9, 0x1C, 0x24, 0x6A, 0x00, 0xD9, 0x46, 0x3C, 0x51, 0x8D, 0x46, 0x18, 0xD9, 0x1C, 0x24, 0x50, 0x8D, 0x46, 0x0C, 0xE8, 0xB1, 0xB1, 0xD3, 0xFF, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x01, 0xE8, 0x60, 0xC0, 0x1C, 0x00, 0x83, 0xC4, 0x1C, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_ingame_main_menu_show(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    wd(this as usize + 8, 0);
    let e1 = cr(0x793d60, r.eax, this, r.edx, this, r.edi, &[1]);
    let f1 = q(rd(ga(0xdd1c50)));
    let f2 = q(rd(this as usize + 0x3c));
    let e2 = cr(0x4959a0, this + 0xc, this, r.edx, this, r.edi, &[this + 0x18, f2, 0, f1]);
    let _ = e1;
    let pl = rd(ga(0x21703d4));
    r.ecx = pl;
    cr(0x926860, e2, pl, r.edx, this, r.edi, &[1, 1, 0])
    }
}
port!(th_ovl_ingame_main_menu_show, f_ovl_ingame_main_menu_show, c_ovl_ingame_main_menu_show, "ret");
// 0x0075a810 .. 0x0075a883 (115 bytes)
const B_ovl_ingame_main_menu_hide: [u8; 115] = [0x6A, 0x00, 0xE8, 0x49, 0x95, 0x03, 0x00, 0x83, 0xC4, 0x04, 0x83, 0x3D, 0x0C, 0x5B, 0x5F, 0x02, 0x00, 0xC7, 0x05, 0x10, 0x5B, 0x5F, 0x02, 0x00, 0x00, 0x00, 0x00, 0x75, 0x41, 0x83, 0x3D, 0xE8, 0x5A, 0x5F, 0x02, 0x00, 0x74, 0x38, 0xD9, 0x05, 0x50, 0x1C, 0xDD, 0x00, 0x83, 0x25, 0xE8, 0x5B, 0x5F, 0x02, 0xFD, 0x51, 0xD9, 0x1C, 0x24, 0xC7, 0x05, 0xE8, 0x5A, 0x5F, 0x02, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x9A, 0x8B, 0xD4, 0xFF, 0xA1, 0xE8, 0x5A, 0x5F, 0x02, 0x8D, 0x04, 0x80, 0x8B, 0x04, 0x85, 0x50, 0x52, 0xDD, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xD0, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0xE8, 0xE1, 0xBF, 0x1C, 0x00, 0x83, 0xC4, 0x0C, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_ingame_main_menu_hide(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let mut ax = cr(0x793d60, r.eax, r.ecx, r.edx, r.esi, r.edi, &[0]);
    let skip = rd(ga(0x25f5b0c)) != 0;
    wd(ga(0x25f5b10), 0);
    if !skip && rd(ga(0x25f5ae8)) != 0 {
        let f1 = q(rd(ga(0xdd1c50)));
        wd(ga(0x25f5be8), rd(ga(0x25f5be8)) & 0xFFFF_FFFD);
        wd(ga(0x25f5ae8), 0);
        ax = cr(0x4a33f0, ax, r.ecx, r.edx, r.esi, r.edi, &[f1]);
        let idx = rd(ga(0x25f5ae8));
        let f = rd(ga(0xdd5250 + (idx.wrapping_mul(5) as usize).wrapping_mul(4)));
        ax = f;
        if f != 0 {
            ax = crp(f, f, r.ecx, r.edx, r.esi, r.edi, &[]);
        }
    }
    let pl = rd(ga(0x21703d4));
    r.ecx = pl;
    cr(0x926860, ax, pl, r.edx, r.esi, r.edi, &[0, 1, 0])
    }
}
port!(th_ovl_ingame_main_menu_hide, f_ovl_ingame_main_menu_hide, c_ovl_ingame_main_menu_hide, "ret");
// 0x0075a890 .. 0x0075a90c (124 bytes)
const B_ovl_ingame_main_menu_update: [u8; 124] = [0x53, 0xBB, 0x01, 0x00, 0x00, 0x00, 0x56, 0x8D, 0x43, 0x35, 0x8B, 0xF1, 0xE8, 0x8F, 0x51, 0xDA, 0xFF, 0x84, 0xC0, 0x75, 0x50, 0x8D, 0x43, 0x36, 0xE8, 0x83, 0x51, 0xDA, 0xFF, 0x84, 0xC0, 0x75, 0x2A, 0xE8, 0xAA, 0x5F, 0xDA, 0xFF, 0x84, 0xC0, 0x74, 0x48, 0x8B, 0x76, 0x08, 0x2B, 0xF3, 0x75, 0x0E, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3A, 0xC3, 0x75, 0x05, 0xE8, 0x61, 0xCF, 0x09, 0x00, 0x32, 0xDB, 0xE8, 0xDA, 0x59, 0xDA, 0xFF, 0x5E, 0x8A, 0xC3, 0x5B, 0xC3, 0x01, 0x5E, 0x08, 0x83, 0x7E, 0x08, 0x05, 0x7C, 0x1E, 0xC7, 0x46, 0x08, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xC0, 0x59, 0xDA, 0xFF, 0x5E, 0x8A, 0xC3, 0x5B, 0xC3, 0x83, 0x46, 0x08, 0xFF, 0x79, 0x07, 0xC7, 0x46, 0x08, 0x04, 0x00, 0x00, 0x00, 0xE8, 0xA9, 0x59, 0xDA, 0xFF, 0x5E, 0x8A, 0xC3, 0x5B, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_ovl_ingame_main_menu_update(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let a1 = cr(0x4ffa30, 0x36, this, r.edx, this, r.edi, &[]);
    if a1 & 0xFF != 0 {
        let v = rd(this as usize + 8).wrapping_sub(1);
        wd(this as usize + 8, if (v as i32) < 0 { 4 } else { v });
        let e = cr(0x5002b0, a1, this, r.edx, this, r.edi, &[]);
        return lo8(e, 1);
    }
    let a2 = cr(0x4ffa30, 0x37, this, r.edx, this, r.edi, &[]);
    if a2 & 0xFF != 0 {
        let v = rd(this as usize + 8).wrapping_add(1);
        wd(this as usize + 8, if (v as i32) < 5 { v } else { 0 });
        let e = cr(0x5002b0, a2, this, r.edx, this, r.edi, &[]);
        return lo8(e, 1);
    }
    let a3 = cr(0x500860, a2, this, r.edx, this, r.edi, &[]);
    if a3 & 0xFF == 0 {
        let e = cr(0x5002b0, a3, this, r.edx, this, r.edi, &[]);
        return lo8(e, 1);
    }
    let s = rd(this as usize + 8).wrapping_sub(1);
    let mut ax = a3;
    if s == 0 {
        let b = rb(ga(0x2528b48)) as u32;
        ax = lo8(a3, b);
        if b == 1 {
            ax = cr(0x7f7830, ax, this, r.edx, 0, r.edi, &[]);
        }
    }
    let e = cr(0x5002b0, ax, this, r.edx, s, r.edi, &[]);
    lo8(e, 0)
    }
}
port!(th_ovl_ingame_main_menu_update, f_ovl_ingame_main_menu_update, c_ovl_ingame_main_menu_update, "ret");
// 0x00863350 .. 0x008633aa (90 bytes)
const B_player_destroy_from_data: [u8; 90] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x4F, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x38, 0x39, 0x46, 0x44, 0x75, 0x33, 0x83, 0x7E, 0x4C, 0x01, 0x75, 0x2D, 0x80, 0xBE, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x75, 0x24, 0x3B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x74, 0x1C, 0x8B, 0xC6, 0xE8, 0x1F, 0xC3, 0xE2, 0xFF, 0x8B, 0x16, 0x8B, 0x42, 0x10, 0x6A, 0x01, 0x8B, 0xCE, 0xFF, 0xD0, 0x81, 0xA6, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x5E, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_player_destroy_from_data(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let id = arg(a, 0);
    if id == 0 {
        return 0;
    }
    r.ecx = (id & 0xFFF) << 4;
    let e = rd(ga(0x2149C64 + r.ecx as usize));
    if e == 0 || rd(e as usize + 0x44) != id || rd(e as usize + 0x4c) != 1 || rb(e as usize + 0x3cc) != 1 || e == rd(ga(0x21703d4)) {
        return id;
    }
    cr(0x68f6b0, e, r.ecx, r.edx, e, r.edi, &[]);
    let vt = rd(e as usize);
    let f = rd(vt as usize + 0x10);
    r.ecx = e;
    r.edx = vt;
    let ret = crp(f, f, e, vt, e, r.edi, &[1]);
    wd(e as usize + 0x80, rd(e as usize + 0x80) & 0xFFFF_FF7F);
    ret
    }
}
port!(th_player_destroy_from_data, f_player_destroy_from_data, c_player_destroy_from_data, "ret 12");
// 0x008633b0 .. 0x008633c2 (18 bytes)
const B_player_on_net_tick: [u8; 18] = [0x56, 0x8B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x85, 0xF6, 0x74, 0x05, 0xE8, 0x10, 0x9B, 0x15, 0x00, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_player_on_net_tick(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let p = rd(ga(0x21703d4));
    if p != 0 {
        return cr(0x9bced0, r.eax, r.ecx, r.edx, p, r.edi, &[]);
    }
    r.eax
    }
}
port!(th_player_on_net_tick, f_player_on_net_tick, c_player_on_net_tick, "ret");
// 0x00852520 .. 0x00852580 (96 bytes)
const B_npc_on_net_tick: [u8; 96] = [0x55, 0x8B, 0x2D, 0xC0, 0xFF, 0x15, 0x02, 0x56, 0x8B, 0xF5, 0x85, 0xF6, 0x74, 0x4F, 0x57, 0x8B, 0x3D, 0x14, 0x8C, 0x52, 0x02, 0xF6, 0x86, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x75, 0x30, 0x8B, 0x8E, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x21, 0x85, 0xFF, 0x74, 0x1D, 0x8B, 0x87, 0x08, 0x04, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x13, 0x8A, 0x11, 0x3A, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x75, 0x0E, 0xF6, 0x81, 0x7C, 0x06, 0x00, 0x00, 0x01, 0x75, 0x05, 0xE8, 0x62, 0xA9, 0x16, 0x00, 0x8B, 0xB6, 0x60, 0x11, 0x00, 0x00, 0x3B, 0xF5, 0x74, 0x04, 0x85, 0xF6, 0x75, 0xB9, 0x5F, 0x5E, 0x5D, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_npc_on_net_tick(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let head = rd(ga(0x215ffc0));
    if head == 0 {
        return r.eax;
    }
    let edi = rd(ga(0x2528c14));
    let (mut ax, mut cx, mut dx) = (r.eax, r.ecx, r.edx);
    let mut esi = head;
    loop {
        if rd(esi as usize + 0x80) & 0xb == 0 {
            cx = rd(esi as usize + 0xb0);
            let mut go = false;
            if cx == 0 || edi == 0 {
                go = true;
            } else {
                ax = rd(edi as usize + 0x408);
                if ax == 0 {
                    go = true;
                } else {
                    let b = rb(cx as usize);
                    dx = lo8(dx, b as u32);
                    if b == rb(ax as usize + 0x1b0) && rb(cx as usize + 0x67c) & 1 == 0 {
                        go = true;
                    }
                }
            }
            if go {
                ax = cr(0x9bced0, ax, cx, dx, esi, edi, &[]);
            }
        }
        esi = rd(esi as usize + 0x1160);
        if esi == head || esi == 0 {
            break;
        }
    }
    r.ecx = cx;
    r.edx = dx;
    ax
    }
}
port!(th_npc_on_net_tick, f_npc_on_net_tick, c_npc_on_net_tick, "ret");
// 0x0084e210 .. 0x0084e26b (91 bytes)
const B_mover_data_compressor_compress: [u8; 91] = [0x83, 0x7C, 0x24, 0x04, 0x00, 0x55, 0x8B, 0xE9, 0x74, 0x4D, 0x53, 0x8B, 0x5C, 0x24, 0x10, 0x85, 0xDB, 0x74, 0x43, 0x57, 0x8B, 0x7C, 0x24, 0x1C, 0x85, 0xFF, 0x74, 0x39, 0x8B, 0x45, 0x04, 0x8B, 0x00, 0x56, 0x8D, 0x57, 0x34, 0x52, 0x8B, 0x54, 0x24, 0x18, 0x6A, 0x00, 0x8D, 0x4D, 0x04, 0x8D, 0x73, 0x34, 0x56, 0x52, 0xFF, 0xD0, 0xF6, 0x06, 0x08, 0x5E, 0x74, 0x19, 0x8B, 0x55, 0x0C, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x12, 0x83, 0xC7, 0x38, 0x57, 0x6A, 0x00, 0x8D, 0x4D, 0x0C, 0x83, 0xC3, 0x38, 0x53, 0x50, 0xFF, 0xD2, 0x5F, 0x5B, 0x5D, 0xC2, 0x10, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mover_data_compressor_compress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let a0 = arg(a, 0);
    if a0 == 0 {
        return r.eax;
    }
    let (a1, a3) = (arg(a, 1), arg(a, 3));
    if a1 == 0 || a3 == 0 {
        return r.eax;
    }
    let this = r.ecx;
    let vt = rd(this as usize + 4);
    let f = rd(vt as usize);
    r.ecx = this + 4;
    r.edx = a0;
    let ret = crp(f, f, this + 4, a0, a1 + 0x34, a3, &[a0, a1 + 0x34, 0, a3 + 0x34]);
    if rb(a1 as usize + 0x34) & 8 == 0 {
        return ret;
    }
    let vt2 = rd(this as usize + 0xc);
    let f2 = rd(vt2 as usize);
    r.ecx = this + 0xc;
    r.edx = f2;
    crp(f2, a0, this + 0xc, f2, r.esi, a3 + 0x38, &[a0, a1 + 0x38, 0, a3 + 0x38])
    }
}
port!(th_mover_data_compressor_compress, f_mover_data_compressor_compress, c_mover_data_compressor_compress, "ret 16");
// 0x0084e270 .. 0x0084e2b6 (70 bytes)
const B_mover_data_compressor_decompress: [u8; 70] = [0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x85, 0xED, 0x8B, 0xD9, 0x74, 0x35, 0x57, 0x8B, 0x7C, 0x24, 0x14, 0x85, 0xFF, 0x74, 0x2B, 0x8B, 0x43, 0x04, 0x8B, 0x50, 0x04, 0x56, 0x6A, 0x00, 0x8D, 0x4B, 0x04, 0x8D, 0x77, 0x34, 0x56, 0x55, 0xFF, 0xD2, 0xF6, 0x06, 0x08, 0x5E, 0x74, 0x12, 0x8B, 0x43, 0x0C, 0x8B, 0x50, 0x04, 0x6A, 0x00, 0x8D, 0x4B, 0x0C, 0x83, 0xC7, 0x38, 0x57, 0x55, 0xFF, 0xD2, 0x5F, 0x5D, 0x5B, 0xC2, 0x0C, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_mover_data_compressor_decompress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let a0 = arg(a, 0);
    if a0 == 0 {
        return r.eax;
    }
    let a1 = arg(a, 1);
    if a1 == 0 {
        return r.eax;
    }
    let this = r.ecx;
    let vt = rd(this as usize + 4);
    let f = rd(vt as usize + 4);
    r.ecx = this + 4;
    r.edx = f;
    let ret = crp(f, vt, this + 4, f, a1 + 0x34, a1, &[a0, a1 + 0x34, 0]);
    if rb(a1 as usize + 0x34) & 8 == 0 {
        return ret;
    }
    let vt2 = rd(this as usize + 0xc);
    let f2 = rd(vt2 as usize + 4);
    r.ecx = this + 0xc;
    r.edx = f2;
    crp(f2, vt2, this + 0xc, f2, r.esi, a1 + 0x38, &[a0, a1 + 0x38, 0])
    }
}
port!(th_mover_data_compressor_decompress, f_mover_data_compressor_decompress, c_mover_data_compressor_decompress, "ret 12");
// 0x008584a0 .. 0x00858519 (121 bytes)
const B_killed_data_compressor_compress: [u8; 121] = [0x53, 0x8B, 0x5C, 0x24, 0x08, 0x85, 0xDB, 0x74, 0x6C, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x85, 0xF6, 0x74, 0x62, 0x57, 0x8B, 0x7C, 0x24, 0x1C, 0x85, 0xFF, 0x74, 0x58, 0x8A, 0x16, 0x8B, 0xC3, 0xE8, 0x3C, 0x4C, 0xFA, 0xFF, 0x8B, 0x4E, 0x04, 0xE8, 0xA4, 0xCA, 0xFA, 0xFF, 0x8B, 0xCB, 0x8B, 0xD0, 0xE8, 0xEB, 0x4E, 0xFA, 0xFF, 0x8B, 0x4E, 0x08, 0xE8, 0x93, 0xCA, 0xFA, 0xFF, 0x8B, 0xCB, 0x8B, 0xD0, 0xE8, 0xDA, 0x4E, 0xFA, 0xFF, 0x8B, 0x46, 0x0C, 0xE8, 0x82, 0xB6, 0x31, 0x00, 0x8A, 0xD0, 0x8B, 0xC3, 0xE8, 0x09, 0x4C, 0xFA, 0xFF, 0x8B, 0x06, 0x89, 0x07, 0x8B, 0x4E, 0x04, 0x89, 0x4F, 0x04, 0x8B, 0x56, 0x08, 0x89, 0x57, 0x08, 0x8B, 0x46, 0x0C, 0x89, 0x47, 0x0C, 0x8B, 0x4E, 0x10, 0x89, 0x4F, 0x10, 0x5F, 0x5E, 0x5B, 0xC2, 0x10, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_killed_data_compressor_compress(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let (a0, a1, a3) = (arg(a, 0), arg(a, 1), arg(a, 3));
    if a0 == 0 || a1 == 0 || a3 == 0 {
        return r.eax;
    }
    let e = a1 as usize;
    let ecx = r.ecx;
    let mut dx = lo8(r.edx, rb(e) as u32);
    let mut ax = cr(0x7fd100, a0, ecx, dx, a1, a3, &[]);
    let c1 = rd(e + 4);
    ax = cr(0x804f70, ax, c1, dx, a1, a3, &[]);
    dx = ax;
    ax = cr(0x7fd3c0, ax, a0, dx, a1, a3, &[]);
    let c2 = rd(e + 8);
    ax = cr(0x804f70, ax, c2, dx, a1, a3, &[]);
    dx = ax;
    ax = cr(0x7fd3c0, ax, a0, dx, a1, a3, &[]);
    let v = rd(e + 0xc);
    ax = cr(0xb73b70, v, a0, dx, a1, a3, &[]);
    dx = lo8(dx, ax);
    cr(0x7fd100, a0, a0, dx, a1, a3, &[]);
    wd(a3 as usize, rd(e));
    wd(a3 as usize + 4, rd(e + 4));
    wd(a3 as usize + 8, rd(e + 8));
    wd(a3 as usize + 0xc, rd(e + 0xc));
    wd(a3 as usize + 0x10, rd(e + 0x10));
    r.ecx = rd(e + 0x10);
    r.edx = rd(e + 8);
    rd(e + 0xc)
    }
}
port!(th_killed_data_compressor_compress, f_killed_data_compressor_compress, c_killed_data_compressor_compress, "ret 16");
// 0x0058cda0 .. 0x0058ce59 (185 bytes)
const B_human_torch_instance_load_from_table: [u8; 185] = [0x53, 0x55, 0x56, 0x57, 0x8B, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0xBB, 0x01, 0x00, 0x00, 0x00, 0x88, 0x59, 0x04, 0xE8, 0x19, 0xAE, 0x44, 0x00, 0x6A, 0x00, 0x57, 0x8B, 0xF0, 0xE8, 0x3F, 0x1F, 0x45, 0x00, 0x83, 0xC4, 0x08, 0x6A, 0x00, 0x53, 0x6A, 0x00, 0x8D, 0xAF, 0xC4, 0x10, 0x00, 0x00, 0x55, 0xE8, 0x5B, 0xDB, 0xFC, 0xFF, 0x85, 0xF6, 0x74, 0x1C, 0x6A, 0x00, 0x56, 0xE8, 0x1F, 0x1F, 0x45, 0x00, 0x83, 0xC4, 0x08, 0x6A, 0x00, 0x53, 0x6A, 0x00, 0x8D, 0x86, 0xC4, 0x10, 0x00, 0x00, 0x50, 0xE8, 0x3B, 0xDB, 0xFC, 0xFF, 0x8B, 0xD7, 0xE8, 0x94, 0x6C, 0x43, 0x00, 0x84, 0xC0, 0x74, 0x0B, 0x57, 0xE8, 0xFA, 0x6C, 0x43, 0x00, 0x83, 0xC4, 0x04, 0xEB, 0x0B, 0x6A, 0x00, 0x53, 0x6A, 0x00, 0x55, 0xE8, 0x1A, 0xDB, 0xFC, 0xFF, 0x85, 0xF6, 0x74, 0x32, 0x8B, 0xD6, 0xE8, 0x6F, 0x6C, 0x43, 0x00, 0x84, 0xC0, 0x74, 0x16, 0x56, 0xE8, 0xD5, 0x6C, 0x43, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x88, 0x1D, 0x23, 0x96, 0x52, 0x02, 0x5B, 0xC2, 0x08, 0x00, 0x6A, 0x00, 0x53, 0x6A, 0x00, 0x81, 0xC6, 0xC4, 0x10, 0x00, 0x00, 0x56, 0xE8, 0xE4, 0xDA, 0xFC, 0xFF, 0x5F, 0x5E, 0x5D, 0x88, 0x1D, 0x23, 0x96, 0x52, 0x02, 0x5B, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_human_torch_instance_load_from_table(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // [this+4] = 1; equip slot 0 on the player and (when there is one) the co-op partner
    let this = r.ecx;
    let pl = rd(ga(0x21703d4));
    wb(this as usize + 4, 1);
    let partner = cr(0x9d7bd0, r.eax, this, r.edx, r.esi, pl, &[]);
    let mut ax = cr(0x9ded00, partner, this, r.edx, partner, pl, &[pl, 0]);
    let inv = pl + 0x10c4;
    ax = cr(0x55a930, ax, this, r.edx, partner, pl, &[inv, 0, 1, 0]);
    if partner != 0 {
        ax = cr(0x9ded00, ax, this, r.edx, partner, pl, &[partner, 0]);
        ax = cr(0x55a930, partner + 0x10c4, this, r.edx, partner, pl, &[partner + 0x10c4, 0, 1, 0]);
    }
    r.edx = pl;
    ax = cr(0x9c3a90, ax, this, pl, partner, pl, &[]);
    if ax & 0xFF != 0 {
        ax = cr(0x9c3b00, ax, this, pl, partner, pl, &[pl]);
    } else {
        ax = cr(0x55a930, ax, this, pl, partner, pl, &[inv, 0, 1, 0]);
    }
    if partner != 0 {
        r.edx = partner;
        ax = cr(0x9c3a90, ax, this, partner, partner, pl, &[]);
        if ax & 0xFF != 0 {
            ax = cr(0x9c3b00, ax, this, partner, partner, pl, &[partner]);
        } else {
            ax = cr(0x55a930, ax, this, partner, partner + 0x10c4, pl, &[partner + 0x10c4, 0, 1, 0]);
        }
    }
    wb(ga(0x2529623), 1);
    ax
    }
}
port!(th_human_torch_instance_load_from_table, f_human_torch_instance_load_from_table, c_human_torch_instance_load_from_table, "ret 8");
// 0x00656be0 .. 0x00656c6f (143 bytes)
const B_pimping_instance_load_from_table: [u8; 143] = [0x8B, 0x44, 0x24, 0x04, 0x53, 0x56, 0x57, 0x8B, 0xF1, 0x50, 0x56, 0xC6, 0x46, 0x04, 0x01, 0xE8, 0x9C, 0xF9, 0xFF, 0xFF, 0x8B, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0xE8, 0xD1, 0x0F, 0x38, 0x00, 0x6A, 0x03, 0x8B, 0xCF, 0x8B, 0xD8, 0xE8, 0x06, 0x77, 0x38, 0x00, 0x83, 0xC4, 0x04, 0x6A, 0x01, 0x57, 0xE8, 0x3B, 0x97, 0x38, 0x00, 0x6A, 0x00, 0x57, 0xE8, 0xE3, 0x80, 0x38, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xDB, 0x74, 0x1F, 0x6A, 0x03, 0x8B, 0xCB, 0xE8, 0xE3, 0x76, 0x38, 0x00, 0x83, 0xC4, 0x04, 0x6A, 0x01, 0x53, 0xE8, 0x18, 0x97, 0x38, 0x00, 0x6A, 0x00, 0x53, 0xE8, 0xC0, 0x80, 0x38, 0x00, 0x83, 0xC4, 0x08, 0x33, 0xC0, 0x89, 0x86, 0x10, 0x02, 0x00, 0x00, 0x89, 0x86, 0x14, 0x02, 0x00, 0x00, 0x89, 0x86, 0x18, 0x02, 0x00, 0x00, 0x89, 0x86, 0x1C, 0x02, 0x00, 0x00, 0x89, 0x86, 0x20, 0x02, 0x00, 0x00, 0x5F, 0x89, 0x86, 0x24, 0x02, 0x00, 0x00, 0x5E, 0x5B, 0xC2, 0x08, 0x00];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_pimping_instance_load_from_table(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    let this = r.ecx;
    let a0 = arg(a, 0);
    wb(this as usize + 4, 1);
    let e0 = cr(0x656590, a0, this, r.edx, this, r.edi, &[this, a0]);
    let pl = rd(ga(0x21703d4));
    let partner = cr(0x9d7bd0, e0, this, r.edx, this, pl, &[]);
    let mut ax = cr(0x9de310, partner, pl, r.edx, this, pl, &[3]);
    ax = cr(0x9e0350, ax, pl, r.edx, this, pl, &[pl, 1]);
    ax = cr(0x9ded00, ax, pl, r.edx, this, pl, &[pl, 0]);
    r.ecx = pl;
    if partner != 0 {
        ax = cr(0x9de310, ax, partner, r.edx, this, pl, &[3]);
        ax = cr(0x9e0350, ax, partner, r.edx, this, pl, &[partner, 1]);
        ax = cr(0x9ded00, ax, partner, r.edx, this, pl, &[partner, 0]);
        r.ecx = partner;
    }
    let _ = ax;
    let t = this as usize;
    wd(t + 0x210, 0);
    wd(t + 0x214, 0);
    wd(t + 0x218, 0);
    wd(t + 0x21c, 0);
    wd(t + 0x220, 0);
    wd(t + 0x224, 0);
    0
    }
}
port!(th_pimping_instance_load_from_table, f_pimping_instance_load_from_table, c_pimping_instance_load_from_table, "ret 8");
// 0x006f4dc0 .. 0x006f4e34 (116 bytes)
const B_havok_all_ray_hit_collector_ctor: [u8; 116] = [0xD9, 0xE8, 0x56, 0x57, 0x8B, 0xF1, 0x68, 0x00, 0x42, 0x6F, 0x00, 0xD9, 0x5E, 0x04, 0x6A, 0x08, 0x8D, 0x46, 0x20, 0xC7, 0x06, 0xC8, 0x89, 0xE1, 0x00, 0x6A, 0x60, 0x33, 0xFF, 0x50, 0x89, 0x46, 0x10, 0x89, 0x7E, 0x14, 0xC7, 0x46, 0x18, 0x08, 0x00, 0x00, 0x80, 0xE8, 0x60, 0xD6, 0xD0, 0xFF, 0xD9, 0xE8, 0x68, 0x90, 0x31, 0x70, 0x00, 0xD9, 0x5E, 0x04, 0x6A, 0x08, 0x8D, 0x86, 0x2C, 0x03, 0x00, 0x00, 0x89, 0x7E, 0x14, 0xC7, 0x06, 0xF8, 0x89, 0xE1, 0x00, 0x6A, 0x0C, 0x50, 0x89, 0x86, 0x20, 0x03, 0x00, 0x00, 0x89, 0xBE, 0x24, 0x03, 0x00, 0x00, 0xC7, 0x86, 0x28, 0x03, 0x00, 0x00, 0x08, 0x00, 0x00, 0x80, 0xE8, 0x27, 0xD6, 0xD0, 0xFF, 0x89, 0xBE, 0x8C, 0x03, 0x00, 0x00, 0x5F, 0x8B, 0xC6, 0x5E, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_all_ray_hit_collector_ctor(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = ecx = this; two vector-ctor passes over the hit arrays
    let this = r.ecx;
    let t = this as usize;
    wd(t + 4, 0x3F80_0000);
    wd(t, 0xe189c8);
    wd(t + 0x10, this + 0x20);
    wd(t + 0x14, 0);
    wd(t + 0x18, 0x8000_0008);
    cr(0x402450, this + 0x20, this, r.edx, this, 0, &[this + 0x20, 0x60, 8, 0x6f4200]);
    wd(t + 4, 0x3F80_0000);
    wd(t + 0x14, 0);
    wd(t, 0xe189f8);
    wd(t + 0x320, this + 0x32c);
    wd(t + 0x324, 0);
    wd(t + 0x328, 0x8000_0008);
    cr(0x402450, this + 0x32c, this, r.edx, this, 0, &[this + 0x32c, 0xc, 8, 0x703190]);
    wd(t + 0x38c, 0);
    this
    }
}
port!(th_havok_all_ray_hit_collector_ctor, f_havok_all_ray_hit_collector_ctor, c_havok_all_ray_hit_collector_ctor, "ret");
// 0x006f50b0 .. 0x006f510a (90 bytes)
const B_havok_all_linear_cast_collector_ctor: [u8; 90] = [0xD9, 0x05, 0x88, 0xC2, 0xE5, 0x00, 0x57, 0x68, 0x90, 0x31, 0x70, 0x00, 0xC7, 0x46, 0x18, 0x08, 0x00, 0x00, 0x80, 0x8D, 0x46, 0x20, 0x89, 0x46, 0x10, 0xD9, 0x5E, 0x04, 0x68, 0x00, 0x01, 0x00, 0x00, 0x8D, 0x86, 0xAC, 0x01, 0x00, 0x00, 0x33, 0xFF, 0x89, 0x7E, 0x14, 0xC7, 0x06, 0x10, 0x8A, 0xE1, 0x00, 0x6A, 0x0C, 0x50, 0x89, 0x86, 0xA0, 0x01, 0x00, 0x00, 0x89, 0xBE, 0xA4, 0x01, 0x00, 0x00, 0xC7, 0x86, 0xA8, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x80, 0xE8, 0x50, 0xD3, 0xD0, 0xFF, 0x89, 0xBE, 0xAC, 0x0D, 0x00, 0x00, 0x8B, 0xC6, 0x5F, 0xC3];
#[allow(unused_unsafe, unused_assignments)]
unsafe fn f_havok_all_linear_cast_collector_ctor(r: &mut Pushad, a: *const u32) -> u32 {
    let _ = a;
    unsafe {
    // esi = this (register argument); edi is used as a zero and restored
    let this = r.esi;
    let t = this as usize;
    wd(t + 0x18, 0x8000_0008);
    wd(t + 0x10, this + 0x20);
    wd(t + 4, q(rd(ga(0xe5c288))));
    wd(t + 0x14, 0);
    wd(t, 0xe18a10);
    wd(t + 0x1a0, this + 0x1ac);
    wd(t + 0x1a4, 0);
    wd(t + 0x1a8, 0x8000_0100);
    cr(0x402450, this + 0x1ac, r.ecx, r.edx, this, 0, &[this + 0x1ac, 0xc, 0x100, 0x703190]);
    wd(t + 0xdac, 0);
    this
    }
}
port!(th_havok_all_linear_cast_collector_ctor, f_havok_all_linear_cast_collector_ctor, c_havok_all_linear_cast_collector_ctor, "ret");

static PORTS: [Port; 85] = [
    Port { name: "num_put_sdd", addr: 0x4edbc0, body: &B_num_put_sdd, thunk: th_num_put_sdd, cnt: &c_num_put_sdd },
    Port { name: "pc_multi_session_info_sdd", addr: 0x68f020, body: &B_pc_multi_session_info_sdd, thunk: th_pc_multi_session_info_sdd, cnt: &c_pc_multi_session_info_sdd },
    Port { name: "multi_session_info_base_sdd", addr: 0x68f070, body: &B_multi_session_info_base_sdd, thunk: th_multi_session_info_base_sdd, cnt: &c_multi_session_info_base_sdd },
    Port { name: "havok_closest_ray_hit_collector_sdd", addr: 0x6f4d70, body: &B_havok_closest_ray_hit_collector_sdd, thunk: th_havok_closest_ray_hit_collector_sdd, cnt: &c_havok_closest_ray_hit_collector_sdd },
    Port { name: "multi_core_object_registration_sdd2", addr: 0x8277a0, body: &B_multi_core_object_registration_sdd2, thunk: th_multi_core_object_registration_sdd2, cnt: &c_multi_core_object_registration_sdd2 },
    Port { name: "item_multi_registration_sdd", addr: 0x843170, body: &B_item_multi_registration_sdd, thunk: th_item_multi_registration_sdd, cnt: &c_item_multi_registration_sdd },
    Port { name: "pipebomb_multi_registration_sdd", addr: 0x85c650, body: &B_pipebomb_multi_registration_sdd, thunk: th_pipebomb_multi_registration_sdd, cnt: &c_pipebomb_multi_registration_sdd },
    Port { name: "prop_multi_registration_sdd", addr: 0x86ba20, body: &B_prop_multi_registration_sdd, thunk: th_prop_multi_registration_sdd, cnt: &c_prop_multi_registration_sdd },
    Port { name: "nearest_hit_collector_sdd", addr: 0xbc45a0, body: &B_nearest_hit_collector_sdd, thunk: th_nearest_hit_collector_sdd, cnt: &c_nearest_hit_collector_sdd },
    Port { name: "mempool_sdd", addr: 0xbf4950, body: &B_mempool_sdd, thunk: th_mempool_sdd, cnt: &c_mempool_sdd },
    Port { name: "item_on_register", addr: 0x843320, body: &B_item_on_register, thunk: th_item_on_register, cnt: &c_item_on_register },
    Port { name: "item_on_unregister", addr: 0x843b50, body: &B_item_on_unregister, thunk: th_item_on_unregister, cnt: &c_item_on_unregister },
    Port { name: "item_destroy_from_data", addr: 0x843bb0, body: &B_item_destroy_from_data, thunk: th_item_destroy_from_data, cnt: &c_item_destroy_from_data },
    Port { name: "mover_on_register", addr: 0x84b8b0, body: &B_mover_on_register, thunk: th_mover_on_register, cnt: &c_mover_on_register },
    Port { name: "mover_on_unregister", addr: 0x84bf10, body: &B_mover_on_unregister, thunk: th_mover_on_unregister, cnt: &c_mover_on_unregister },
    Port { name: "npc_on_register", addr: 0x851be0, body: &B_npc_on_register, thunk: th_npc_on_register, cnt: &c_npc_on_register },
    Port { name: "npc_on_unregister", addr: 0x852450, body: &B_npc_on_unregister, thunk: th_npc_on_unregister, cnt: &c_npc_on_unregister },
    Port { name: "npc_destroy_from_data", addr: 0x8524b0, body: &B_npc_destroy_from_data, thunk: th_npc_destroy_from_data, cnt: &c_npc_destroy_from_data },
    Port { name: "pipebomb_on_register", addr: 0x85c810, body: &B_pipebomb_on_register, thunk: th_pipebomb_on_register, cnt: &c_pipebomb_on_register },
    Port { name: "pipebomb_on_unregister", addr: 0x85cbf0, body: &B_pipebomb_on_unregister, thunk: th_pipebomb_on_unregister, cnt: &c_pipebomb_on_unregister },
    Port { name: "pipebomb_destroy_from_data", addr: 0x85cc50, body: &B_pipebomb_destroy_from_data, thunk: th_pipebomb_destroy_from_data, cnt: &c_pipebomb_destroy_from_data },
    Port { name: "prop_on_register", addr: 0x86bbd0, body: &B_prop_on_register, thunk: th_prop_on_register, cnt: &c_prop_on_register },
    Port { name: "prop_on_unregister", addr: 0x86c040, body: &B_prop_on_unregister, thunk: th_prop_on_unregister, cnt: &c_prop_on_unregister },
    Port { name: "prop_destroy_from_data", addr: 0x86c0a0, body: &B_prop_destroy_from_data, thunk: th_prop_destroy_from_data, cnt: &c_prop_destroy_from_data },
    Port { name: "prop_free_snapshot", addr: 0x86c670, body: &B_prop_free_snapshot, thunk: th_prop_free_snapshot, cnt: &c_prop_free_snapshot },
    Port { name: "item_on_owner_left", addr: 0x845530, body: &B_item_on_owner_left, thunk: th_item_on_owner_left, cnt: &c_item_on_owner_left },
    Port { name: "mover_on_owner_left", addr: 0x84dfa0, body: &B_mover_on_owner_left, thunk: th_mover_on_owner_left, cnt: &c_mover_on_owner_left },
    Port { name: "large_convoy_objective_get_start_message", addr: 0x575190, body: &B_large_convoy_objective_get_start_message, thunk: th_large_convoy_objective_get_start_message, cnt: &c_large_convoy_objective_get_start_message },
    Port { name: "liberator_objective_get_start_message", addr: 0x575210, body: &B_liberator_objective_get_start_message, thunk: th_liberator_objective_get_start_message, cnt: &c_liberator_objective_get_start_message },
    Port { name: "hostage_diversion_get_title", addr: 0x6124f0, body: &B_hostage_diversion_get_title, thunk: th_hostage_diversion_get_title, cnt: &c_hostage_diversion_get_title },
    Port { name: "mugging_diversion_get_title", addr: 0x6142c0, body: &B_mugging_diversion_get_title, thunk: th_mugging_diversion_get_title, cnt: &c_mugging_diversion_get_title },
    Port { name: "poker_diversion_get_title", addr: 0x635890, body: &B_poker_diversion_get_title, thunk: th_poker_diversion_get_title, cnt: &c_poker_diversion_get_title },
    Port { name: "hitman_diversion_get_title", addr: 0x63ab70, body: &B_hitman_diversion_get_title, thunk: th_hitman_diversion_get_title, cnt: &c_hitman_diversion_get_title },
    Port { name: "hostage_diversion_get_start_message", addr: 0x6124c0, body: &B_hostage_diversion_get_start_message, thunk: th_hostage_diversion_get_start_message, cnt: &c_hostage_diversion_get_start_message },
    Port { name: "mugging_diversion_get_start_message", addr: 0x614270, body: &B_mugging_diversion_get_start_message, thunk: th_mugging_diversion_get_start_message, cnt: &c_mugging_diversion_get_start_message },
    Port { name: "hostage_diversion_create_instance", addr: 0x612c50, body: &B_hostage_diversion_create_instance, thunk: th_hostage_diversion_create_instance, cnt: &c_hostage_diversion_create_instance },
    Port { name: "poker_diversion_create_instance", addr: 0x6357d0, body: &B_poker_diversion_create_instance, thunk: th_poker_diversion_create_instance, cnt: &c_poker_diversion_create_instance },
    Port { name: "heli_info_load_from_table", addr: 0x575290, body: &B_heli_info_load_from_table, thunk: th_heli_info_load_from_table, cnt: &c_heli_info_load_from_table },
    Port { name: "piracy_info_load_from_table", addr: 0x591a70, body: &B_piracy_info_load_from_table, thunk: th_piracy_info_load_from_table, cnt: &c_piracy_info_load_from_table },
    Port { name: "near_miss_complication_load_from_table", addr: 0x66e710, body: &B_near_miss_complication_load_from_table, thunk: th_near_miss_complication_load_from_table, cnt: &c_near_miss_complication_load_from_table },
    Port { name: "mempool_base_dtor", addr: 0xc00c40, body: &B_mempool_base_dtor, thunk: th_mempool_base_dtor, cnt: &c_mempool_base_dtor },
    Port { name: "page_mempool_size", addr: 0xc1d160, body: &B_page_mempool_size, thunk: th_page_mempool_size, cnt: &c_page_mempool_size },
    Port { name: "page_mempool_used_bytes", addr: 0xc1d170, body: &B_page_mempool_used_bytes, thunk: th_page_mempool_used_bytes, cnt: &c_page_mempool_used_bytes },
    Port { name: "page_mempool_align_to", addr: 0xc1d110, body: &B_page_mempool_align_to, thunk: th_page_mempool_align_to, cnt: &c_page_mempool_align_to },
    Port { name: "page_mempool_release", addr: 0xc1d120, body: &B_page_mempool_release, thunk: th_page_mempool_release, cnt: &c_page_mempool_release },
    Port { name: "page_mempool_release_to", addr: 0xc1d130, body: &B_page_mempool_release_to, thunk: th_page_mempool_release_to, cnt: &c_page_mempool_release_to },
    Port { name: "ovl_text_entry_hide", addr: 0x7617e0, body: &B_ovl_text_entry_hide, thunk: th_ovl_text_entry_hide, cnt: &c_ovl_text_entry_hide },
    Port { name: "ovl_text_entry_update", addr: 0x7617f0, body: &B_ovl_text_entry_update, thunk: th_ovl_text_entry_update, cnt: &c_ovl_text_entry_update },
    Port { name: "ovl_framerate_hide", addr: 0x503090, body: &B_ovl_framerate_hide, thunk: th_ovl_framerate_hide, cnt: &c_ovl_framerate_hide },
    Port { name: "ovl_selection_menu_hide", addr: 0x7612f0, body: &B_ovl_selection_menu_hide, thunk: th_ovl_selection_menu_hide, cnt: &c_ovl_selection_menu_hide },
    Port { name: "havok_all_ray_hit_collector_sdd", addr: 0x6f4ee0, body: &B_havok_all_ray_hit_collector_sdd, thunk: th_havok_all_ray_hit_collector_sdd, cnt: &c_havok_all_ray_hit_collector_sdd },
    Port { name: "closest_linear_cast_collector_sdd", addr: 0x6f5050, body: &B_closest_linear_cast_collector_sdd, thunk: th_closest_linear_cast_collector_sdd, cnt: &c_closest_linear_cast_collector_sdd },
    Port { name: "object_collector_sdd", addr: 0x6fdf20, body: &B_object_collector_sdd, thunk: th_object_collector_sdd, cnt: &c_object_collector_sdd },
    Port { name: "custom_character_collector_sdd", addr: 0x72b150, body: &B_custom_character_collector_sdd, thunk: th_custom_character_collector_sdd, cnt: &c_custom_character_collector_sdd },
    Port { name: "all_linear_cast_collector_sdd", addr: 0x6f51c0, body: &B_all_linear_cast_collector_sdd, thunk: th_all_linear_cast_collector_sdd, cnt: &c_all_linear_cast_collector_sdd },
    Port { name: "havok_game_filter_sdd", addr: 0x70efe0, body: &B_havok_game_filter_sdd, thunk: th_havok_game_filter_sdd, cnt: &c_havok_game_filter_sdd },
    Port { name: "never_collide_filter_sdd", addr: 0xba9de0, body: &B_never_collide_filter_sdd, thunk: th_never_collide_filter_sdd, cnt: &c_never_collide_filter_sdd },
    Port { name: "havok_filter_shape_collection_sdd", addr: 0x70e900, body: &B_havok_filter_shape_collection_sdd, thunk: th_havok_filter_shape_collection_sdd, cnt: &c_havok_filter_shape_collection_sdd },
    Port { name: "havok_navmesh_shape_sdd", addr: 0x710430, body: &B_havok_navmesh_shape_sdd, thunk: th_havok_navmesh_shape_sdd, cnt: &c_havok_navmesh_shape_sdd },
    Port { name: "npc_orientation_heading_compress", addr: 0x8592b0, body: &B_npc_orientation_heading_compress, thunk: th_npc_orientation_heading_compress, cnt: &c_npc_orientation_heading_compress },
    Port { name: "npc_orientation_quaternion_compress", addr: 0x8594e0, body: &B_npc_orientation_quaternion_compress, thunk: th_npc_orientation_quaternion_compress, cnt: &c_npc_orientation_quaternion_compress },
    Port { name: "npc_orientation_heading_decompress", addr: 0x859300, body: &B_npc_orientation_heading_decompress, thunk: th_npc_orientation_heading_decompress, cnt: &c_npc_orientation_heading_decompress },
    Port { name: "npc_orientation_quaternion_decompress", addr: 0x859530, body: &B_npc_orientation_quaternion_decompress, thunk: th_npc_orientation_quaternion_decompress, cnt: &c_npc_orientation_quaternion_decompress },
    Port { name: "npc_spline_pathfind_uncompressed_compress", addr: 0x8595e0, body: &B_npc_spline_pathfind_uncompressed_compress, thunk: th_npc_spline_pathfind_uncompressed_compress, cnt: &c_npc_spline_pathfind_uncompressed_compress },
    Port { name: "npc_spline_pathfind_uncompressed_decompress", addr: 0x859620, body: &B_npc_spline_pathfind_uncompressed_decompress, thunk: th_npc_spline_pathfind_uncompressed_decompress, cnt: &c_npc_spline_pathfind_uncompressed_decompress },
    Port { name: "mempool_ctor", addr: 0xbf48f0, body: &B_mempool_ctor, thunk: th_mempool_ctor, cnt: &c_mempool_ctor },
    Port { name: "mempool_ctor_2", addr: 0xbf4990, body: &B_mempool_ctor_2, thunk: th_mempool_ctor_2, cnt: &c_mempool_ctor_2 },
    Port { name: "mempool_ctor_3", addr: 0xbf49e0, body: &B_mempool_ctor_3, thunk: th_mempool_ctor_3, cnt: &c_mempool_ctor_3 },
    Port { name: "page_mempool_can_alloc", addr: 0xc1cc00, body: &B_page_mempool_can_alloc, thunk: th_page_mempool_can_alloc, cnt: &c_page_mempool_can_alloc },
    Port { name: "page_mempool_reset", addr: 0xc1d000, body: &B_page_mempool_reset, thunk: th_page_mempool_reset, cnt: &c_page_mempool_reset },
    Port { name: "object_get_attach_point", addr: 0x925750, body: &B_object_get_attach_point, thunk: th_object_get_attach_point, cnt: &c_object_get_attach_point },
    Port { name: "object_item_activate", addr: 0x933610, body: &B_object_item_activate, thunk: th_object_item_activate, cnt: &c_object_item_activate },
    Port { name: "ovl_ingame_main_menu_show", addr: 0x75a7c0, body: &B_ovl_ingame_main_menu_show, thunk: th_ovl_ingame_main_menu_show, cnt: &c_ovl_ingame_main_menu_show },
    Port { name: "ovl_ingame_main_menu_hide", addr: 0x75a810, body: &B_ovl_ingame_main_menu_hide, thunk: th_ovl_ingame_main_menu_hide, cnt: &c_ovl_ingame_main_menu_hide },
    Port { name: "ovl_ingame_main_menu_update", addr: 0x75a890, body: &B_ovl_ingame_main_menu_update, thunk: th_ovl_ingame_main_menu_update, cnt: &c_ovl_ingame_main_menu_update },
    Port { name: "player_destroy_from_data", addr: 0x863350, body: &B_player_destroy_from_data, thunk: th_player_destroy_from_data, cnt: &c_player_destroy_from_data },
    Port { name: "player_on_net_tick", addr: 0x8633b0, body: &B_player_on_net_tick, thunk: th_player_on_net_tick, cnt: &c_player_on_net_tick },
    Port { name: "npc_on_net_tick", addr: 0x852520, body: &B_npc_on_net_tick, thunk: th_npc_on_net_tick, cnt: &c_npc_on_net_tick },
    Port { name: "mover_data_compressor_compress", addr: 0x84e210, body: &B_mover_data_compressor_compress, thunk: th_mover_data_compressor_compress, cnt: &c_mover_data_compressor_compress },
    Port { name: "mover_data_compressor_decompress", addr: 0x84e270, body: &B_mover_data_compressor_decompress, thunk: th_mover_data_compressor_decompress, cnt: &c_mover_data_compressor_decompress },
    Port { name: "killed_data_compressor_compress", addr: 0x8584a0, body: &B_killed_data_compressor_compress, thunk: th_killed_data_compressor_compress, cnt: &c_killed_data_compressor_compress },
    Port { name: "human_torch_instance_load_from_table", addr: 0x58cda0, body: &B_human_torch_instance_load_from_table, thunk: th_human_torch_instance_load_from_table, cnt: &c_human_torch_instance_load_from_table },
    Port { name: "pimping_instance_load_from_table", addr: 0x656be0, body: &B_pimping_instance_load_from_table, thunk: th_pimping_instance_load_from_table, cnt: &c_pimping_instance_load_from_table },
    Port { name: "havok_all_ray_hit_collector_ctor", addr: 0x6f4dc0, body: &B_havok_all_ray_hit_collector_ctor, thunk: th_havok_all_ray_hit_collector_ctor, cnt: &c_havok_all_ray_hit_collector_ctor },
    Port { name: "havok_all_linear_cast_collector_ctor", addr: 0x6f50b0, body: &B_havok_all_linear_cast_collector_ctor, thunk: th_havok_all_linear_cast_collector_ctor, cnt: &c_havok_all_linear_cast_collector_ctor },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let (mut ok, mut tot) = (0u32, 0u32);
    // D3D_PORTS (115 IDirect3DDevice9Lost stubs) are not installed: they are naked asm copied from the
    // original bytes, which is not a Rust port. To rewrite as real Rust functions (2026-11-23).
    let _ = &D3D_PORTS;
    install_ports(&PORTS, &mut ok, &mut tot);
    log(&format!("port_small_b: replaced {ok}/{tot} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut v = Vec::new();
    sum_ports(&D3D_PORTS, &mut v);
    sum_ports(&PORTS, &mut v);
    if v.is_empty() {
        return String::new();
    }
    format!("port_small_b: {}", v.join(", "))
}

// =================================================================================================
// Offline differential test
// =================================================================================================
//
// RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_small_b -- --ignored --test-threads=1 --nocapture
// (CASES=n changes the number of random cases per function, default 2000.)

#[cfg(test)]
pub(crate) mod tmap {
    use std::sync::Mutex;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};

    /// Committed game pages (page base addresses), in commit order.
    pub static PAGES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    pub static REDIR: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
    static BASE: Mutex<usize> = Mutex::new(0);
    const LO: usize = 0x40_0000;
    const SPAN: usize = 0x360_0000;

    fn base() -> usize {
        let mut b = BASE.lock().unwrap();
        if *b == 0 {
            let p = unsafe { VirtualAlloc(std::ptr::null(), SPAN, MEM_RESERVE, PAGE_READWRITE) } as usize;
            assert!(p != 0);
            *b = p;
        }
        *b
    }

    /// Make the game range `[a, a + len)` readable / writable test memory.
    pub fn commit(a: usize, len: usize) {
        let h = base();
        let mut pages = PAGES.lock().unwrap();
        let mut p = a & !0xFFF;
        while p < a + len.max(1) {
            if !pages.contains(&p) {
                let host = h + (p - LO);
                let r = unsafe { VirtualAlloc(host as *const _, 4096, MEM_COMMIT, PAGE_READWRITE) };
                assert!(!r.is_null());
                pages.push(p);
            }
            p += 0x1000;
        }
    }

    /// A game address -> the same offset in the test region (committing its page).
    pub fn ga(a: usize) -> usize {
        commit(a, 4);
        base() + (a - LO)
    }

    pub fn tgt(a: usize) -> usize {
        REDIR.lock().unwrap().iter().find(|e| e.0 == a).map(|e| e.1).unwrap_or(a)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    pub const IGN_EAX: u8 = 1;
    pub const IGN_ECX: u8 = 2;
    pub const IGN_EDX: u8 = 4;
    pub const IGN_ESI: u8 = 8;
    pub const IGN_EDI: u8 = 16;

    #[derive(Clone, Copy)]
    pub enum Ret {
        Rnd,
        Ptr,
        PtrN,
        Zero,
        One,
        Bool,
        Byte,
    }

    #[derive(Clone, Copy)]
    pub struct Callee {
        pub addr: usize,
        pub nargs: usize,
        pub pop: bool,
        pub ret: Ret,
        pub ign_regs: u8,
        pub ign_args: u32,
    }

    impl Callee {
        pub const fn ign(mut self, m: u8) -> Self {
            self.ign_regs = m;
            self
        }
        pub const fn ign_args(mut self, m: u32) -> Self {
            self.ign_args = m;
            self
        }
    }

    pub const fn cal(addr: usize, nargs: usize, pop: bool, ret: Ret) -> Callee {
        Callee { addr, nargs, pop, ret, ign_regs: 0, ign_args: 0 }
    }

    pub const CAL_T: Callee = cal(0xFFFF_0001, 1, false, Ret::Rnd);

    pub struct Spec<'a> {
        pub name: &'static str,
        pub body: &'a [u8],
        pub thunk: extern "C" fn(),
        pub nargs: usize,
        pub calls: &'a [(usize, usize)],
        pub abs: &'a [usize],
        pub callees: &'a [Callee],
        pub ign: u8,
    }

    // --- random numbers -----------------------------------------------------------------------------

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
        pub fn chance(&mut self, pct: u32) -> bool {
            self.below(100) < pct
        }
        /// A float bit pattern: finite, huge, tiny, zero, infinite or NaN (signalling too).
        pub fn any_float(&mut self) -> u32 {
            match self.below(10) {
                0 => 0,
                1 => 0x7F80_0000,
                2 => 0x7F80_0000 | (self.u32() & 0x7F_FFFF) | 1,
                3 => 0x7FC0_0000 | (self.u32() & 0x3F_FFFF),
                _ => ((self.u32() as f64 / 4294967296.0 * 200.0 - 100.0) as f32).to_bits(),
            }
        }
        /// A mostly-random dword, with common special values mixed in.
        pub fn word(&mut self) -> u32 {
            match self.below(8) {
                0 => 0,
                1 => 1,
                2 => 0xFFFF_FFFF,
                3 => self.below(16),
                _ => self.u32(),
            }
        }
    }

    // --- the world: arena + game pages + callee log ---------------------------------------------------

    pub const ARENA_WORDS: usize = 32768;
    static mut ARENA: usize = 0;

    pub fn arena() -> usize {
        unsafe {
            if ARENA == 0 {
                ARENA = Box::leak(vec![0u32; ARENA_WORDS].into_boxed_slice()).as_mut_ptr() as usize;
            }
            ARENA
        }
    }

    #[derive(Clone, PartialEq, Debug)]
    pub struct Rec {
        id: usize,
        regs: [u32; 5], // eax ecx edx esi edi (after masking)
        args: Vec<u32>,
    }

    static mut LOG: Vec<Rec> = Vec::new();
    static mut CALLEES: Vec<Callee> = Vec::new();
    static mut RRNG: Rng = Rng(1);

    pub struct World {
        pub rng: Rng,
    }

    impl World {
        /// A random pointer (16-aligned) into the arena, with `room` bytes after it.
        pub fn ptr(&mut self, room: usize) -> u32 {
            let words = ARENA_WORDS - room / 4 - 8;
            (arena() + (self.rng.below(words as u32) as usize & !3) * 4) as u32
        }
        /// A random pointer in arena quarter `zone` (0..4), so objects of different roles never overlap.
        pub fn ptr_z(&mut self, zone: usize, room: usize) -> u32 {
            let zw = ARENA_WORDS / 4;
            let words = zw - room / 4 - 8;
            (arena() + (zone * zw + (self.rng.below(words as u32) as usize & !3)) * 4) as u32
        }
        pub fn fill_arena(&mut self) {
            let a = arena() as *mut u32;
            for i in 0..ARENA_WORDS {
                unsafe { *a.add(i) = self.rng.word() };
            }
        }
        /// Random words over a game range.
        pub fn fill_g(&mut self, addr: usize, len: usize) {
            tmap::commit(addr, len);
            for i in 0..len / 4 {
                unsafe { *(tmap::ga(addr + i * 4) as *mut u32) = self.rng.word() };
            }
        }
        pub fn g(&mut self, addr: usize, v: u32) {
            unsafe { *(tmap::ga(addr) as *mut u32) = v };
        }
        pub fn gb(&mut self, addr: usize, v: u8) {
            unsafe { *(tmap::ga(addr) as *mut u8) = v };
        }
        pub fn rd_g(&self, addr: usize) -> u32 {
            unsafe { *(tmap::ga(addr) as *const u32) }
        }
        pub fn w(&mut self, host: u32, v: u32) {
            unsafe { *(host as *mut u32) = v };
        }
        pub fn wb(&mut self, host: u32, v: u8) {
            unsafe { *(host as *mut u8) = v };
        }
        pub fn r(&self, host: u32) -> u32 {
            unsafe { *(host as *const u32) }
        }
        /// The stub address of the callee with game address `addr`.
        pub fn stub(&self, addr: usize) -> u32 {
            tmap::tgt(addr) as u32
        }
    }

    pub fn wr(host: u32, v: u32) {
        unsafe { *(host as *mut u32) = v };
    }
    pub fn wrb(host: u32, v: u8) {
        unsafe { *(host as *mut u8) = v };
    }
    pub fn rdh(host: u32) -> u32 {
        unsafe { *(host as *const u32) }
    }
    pub fn wg(addr: usize, v: u32) {
        unsafe { *(tmap::ga(addr) as *mut u32) = v };
    }
    pub fn wgb(addr: usize, v: u8) {
        unsafe { *(tmap::ga(addr) as *mut u8) = v };
    }
    pub fn rdg(addr: usize) -> u32 {
        unsafe { *(tmap::ga(addr) as *const u32) }
    }
    /// The stub address of the callee with game address `addr`.
    pub fn stub(addr: usize) -> u32 {
        tmap::tgt(addr) as u32
    }

    #[derive(Clone, Default)]
    pub struct Inp {
        pub eax: u32,
        pub ecx: u32,
        pub edx: u32,
        pub esi: u32,
        pub edi: u32,
        pub args: Vec<u32>,
    }

    extern "C" fn record(id: u32, regs: *const Pushad, args: *const u32) -> u32 {
        unsafe {
            let c = (&*std::ptr::addr_of!(CALLEES))[id as usize];
            let r = &*regs;
            let mut rg = [r.eax, r.ecx, r.edx, r.esi, r.edi];
            for (i, v) in rg.iter_mut().enumerate() {
                if c.ign_regs & (1 << i) != 0 {
                    *v = 0;
                }
            }
            let mut av = Vec::new();
            for i in 0..c.nargs {
                av.push(if c.ign_args & (1 << i) != 0 { 0 } else { *args.add(i) });
            }
            (&mut *std::ptr::addr_of_mut!(LOG)).push(Rec { id: id as usize, regs: rg, args: av });
            let rng = &mut *std::ptr::addr_of_mut!(RRNG);
            match c.ret {
                Ret::Rnd => rng.u32(),
                Ret::Ptr => (arena() + (rng.below(ARENA_WORDS as u32 - 600) as usize & !3) * 4) as u32,
                Ret::PtrN => {
                    if rng.below(2) == 0 {
                        0
                    } else {
                        (arena() + (rng.below(ARENA_WORDS as u32 - 600) as usize & !3) * 4) as u32
                    }
                }
                Ret::Zero => 0,
                Ret::One => 1,
                Ret::Bool => rng.below(2),
                Ret::Byte => rng.below(256),
            }
        }
    }

    pub fn exec_alloc(len: usize) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0);
        p
    }

    /// Machine code for a recording stub of callee `id`.
    fn make_stub(id: usize, pop_bytes: usize) -> usize {
        let mut c: Vec<u8> = vec![
            0x60, // pushad
            0x8D, 0x44, 0x24, 0x24, // lea eax,[esp+0x24]
            0x50, // push eax
            0x8D, 0x44, 0x24, 0x04, // lea eax,[esp+4]
            0x50, // push eax
            0x68,
        ];
        c.extend_from_slice(&(id as u32).to_le_bytes());
        c.push(0xE8);
        let call_at = c.len();
        c.extend_from_slice(&[0; 4]);
        c.extend_from_slice(&[0x83, 0xC4, 0x0C]); // add esp,12
        c.extend_from_slice(&[0x89, 0x44, 0x24, 0x1C]); // mov [esp+0x1c],eax
        c.push(0x61); // popad
        if pop_bytes == 0 {
            c.push(0xC3);
        } else {
            c.extend_from_slice(&[0xC2, pop_bytes as u8, (pop_bytes >> 8) as u8]);
        }
        let p = exec_alloc(c.len() + 16);
        let rel = (record as usize as i32).wrapping_sub((p + call_at + 4) as i32);
        c[call_at..call_at + 4].copy_from_slice(&rel.to_le_bytes());
        unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), p as *mut u8, c.len()) };
        p
    }

    /// The original's bytes in executable memory with the callees and absolute data aimed at the
    /// test stubs / pages.
    fn build(spec: &Spec<'_>) -> usize {
        let mut b = spec.body.to_vec();
        for &a in spec.abs {
            let ob = (a as u32).to_le_bytes();
            let nb = (tmap::ga(a) as u32).to_le_bytes();
            let mut found = 0;
            for i in 0..b.len().saturating_sub(3) {
                if b[i..i + 4] == ob {
                    b[i..i + 4].copy_from_slice(&nb);
                    found += 1;
                }
            }
            assert!(found > 0, "{}: operand {a:#x} not in the body", spec.name);
        }
        let p = exec_alloc(b.len() + 16);
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p as *mut u8, b.len()) };
        for &(off, dest) in spec.calls {
            let d = tmap::tgt(dest);
            let rel = (d as i32).wrapping_sub((p + off + 5) as i32);
            unsafe { std::ptr::write_unaligned((p + off + 1) as *mut i32, rel) };
        }
        p
    }

    // --- the call harness ---------------------------------------------------------------------------

    #[repr(C)]
    #[derive(Clone, Copy, Default, Debug)]
    pub struct TB {
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
        pub o_eax: u32,
        pub o_ecx: u32,
        pub o_edx: u32,
        pub o_ebx: u32,
        pub o_esi: u32,
        pub o_edi: u32,
        pub o_ebp: u32,
        pub esp_d: u32,
        saved: u32,
    }

    static mut SLOTS: [u32; 3] = [0; 3];
    const EBX_IN: u32 = 0xB1B1_B1B1;
    const EBP_IN: u32 = 0xE9E9_E9E9;

    unsafe fn call_t(f: usize, r: &Inp) -> TB {
        let mut b = TB {
            f: f as u32,
            eax: r.eax,
            ecx: r.ecx,
            edx: r.edx,
            ebx: EBX_IN,
            esi: r.esi,
            edi: r.edi,
            ebp: EBP_IN,
            args: r.args.as_ptr() as u32,
            nargs: r.args.len() as u32,
            ..Default::default()
        };
        unsafe {
            let slots = std::ptr::addr_of_mut!(SLOTS);
            (*slots)[0] = &mut b as *mut TB as u32;
            (*slots)[1] = f as u32;
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 72], esp",
                "mov ebx, eax",
                "mov ecx, [ebx + 36]",
                "mov esi, [ebx + 32]",
                "2:",
                "test ecx, ecx",
                "jz 3f",
                "push dword ptr [esi + ecx*4 - 4]",
                "dec ecx",
                "jmp 2b",
                "3:",
                "mov ecx, [ebx + 8]", "mov edx, [ebx + 12]", "mov esi, [ebx + 20]", "mov edi, [ebx + 24]",
                "mov ebp, [ebx + 28]", "mov eax, [ebx + 4]", "mov ebx, [ebx + 16]",
                "call dword ptr [{s} + 4]",
                "mov [{s} + 8], esp",
                "push eax",
                "mov eax, dword ptr [{s}]",
                "mov [eax + 44], ecx", "mov [eax + 48], edx", "mov [eax + 52], ebx", "mov [eax + 56], esi",
                "mov [eax + 60], edi", "mov [eax + 64], ebp",
                "pop dword ptr [eax + 40]",
                "mov ecx, [eax + 72]",
                "sub ecx, [{s} + 8]",
                "mov [eax + 68], ecx",
                "mov esp, [eax + 72]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                s = sym SLOTS,
                inout("eax") &mut b as *mut TB => _,
                clobber_abi("C"),
            );
        }
        b
    }

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000)
    }

    type Snap = (Vec<u32>, Vec<(usize, Vec<u8>)>);

    fn page_host(g: usize) -> usize {
        tmap::ga(g)
    }

    /// Everything the functions can write: the arena and every game page.
    fn snapshot() -> Snap {
        let a = unsafe { std::slice::from_raw_parts(arena() as *const u32, ARENA_WORDS) }.to_vec();
        let list = tmap::PAGES.lock().unwrap().clone();
        let pages = list.iter().map(|&g| (g, unsafe { std::slice::from_raw_parts(page_host(g) as *const u8, 4096) }.to_vec())).collect();
        (a, pages)
    }

    fn restore(s: &Snap) {
        unsafe { std::ptr::copy_nonoverlapping(s.0.as_ptr(), arena() as *mut u32, ARENA_WORDS) };
        for (g, bytes) in &s.1 {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), page_host(*g) as *mut u8, 4096) };
        }
    }

    fn setup_spec(spec: &Spec<'_>) {
        unsafe {
            let cs = &mut *std::ptr::addr_of_mut!(CALLEES);
            cs.clear();
            let mut r = tmap::REDIR.lock().unwrap();
            r.clear();
            for (i, c) in spec.callees.iter().enumerate() {
                cs.push(*c);
                let s = make_stub(i, if c.pop { c.nargs * 4 } else { 0 });
                r.push((c.addr, s));
            }
        }
    }

    /// Run `n` random cases of `spec`; `gen` builds the world and the inputs.
    pub fn run(spec: &Spec<'_>, genf: impl FnMut(&mut World) -> Inp) {
        run_n(spec, cases(), genf)
    }

    pub fn run_n(spec: &Spec<'_>, n: u32, mut genf: impl FnMut(&mut World) -> Inp) {
        setup_spec(spec);
        let orig = build(spec);
        let mut bad = 0;
        for case in 0..n {
            let mut w = World { rng: Rng(0x9E37_79B9_7F4A_7C15 ^ ((case as u64 + 1) * 0x1000_0000_01B3)) };
            let inp = genf(&mut w);
            assert_eq!(inp.args.len(), spec.nargs, "{}: arg count", spec.name);
            let snap = snapshot();
            let seed = w.rng.u32() as u64 | 1;
            let mut out = Vec::new();
            for which in 0..2 {
                restore(&snap);
                unsafe {
                    (&mut *std::ptr::addr_of_mut!(LOG)).clear();
                    (*std::ptr::addr_of_mut!(RRNG)).0 = seed;
                }
                let f = if which == 0 { orig } else { spec.thunk as usize };
                let tb = unsafe { call_t(f, &inp) };
                let log = unsafe { (&*std::ptr::addr_of!(LOG)).clone() };
                out.push((tb, log, snapshot()));
            }
            let (a, b) = (&out[0], &out[1]);
            let mut msg = String::new();
            let chk = |nm: &str, x: u32, y: u32, bit: u8, msg: &mut String| {
                if x != y && spec.ign & bit == 0 {
                    msg.push_str(&format!(" {nm}: orig {x:#x} rust {y:#x};"));
                }
            };
            chk("eax", a.0.o_eax, b.0.o_eax, IGN_EAX, &mut msg);
            chk("ecx", a.0.o_ecx, b.0.o_ecx, IGN_ECX, &mut msg);
            chk("edx", a.0.o_edx, b.0.o_edx, IGN_EDX, &mut msg);
            chk("esi", a.0.o_esi, b.0.o_esi, IGN_ESI, &mut msg);
            chk("edi", a.0.o_edi, b.0.o_edi, IGN_EDI, &mut msg);
            chk("ebx", a.0.o_ebx, b.0.o_ebx, 0, &mut msg);
            chk("ebp", a.0.o_ebp, b.0.o_ebp, 0, &mut msg);
            chk("esp_d", a.0.esp_d, b.0.esp_d, 0, &mut msg);
            if a.1 != b.1 {
                msg.push_str(&format!(" call log differs: orig {:?} rust {:?};", a.1, b.1));
            }
            if a.2.0 != b.2.0 {
                let i = (0..ARENA_WORDS).find(|&i| a.2.0[i] != b.2.0[i]).unwrap();
                msg.push_str(&format!(" arena word {i} (+{:#x}): orig {:#x} rust {:#x};", i * 4, a.2.0[i], b.2.0[i]));
            }
            for (pa, pb) in a.2.1.iter().zip(b.2.1.iter()) {
                if pa.1 != pb.1 {
                    let i = (0..4096).find(|&i| pa.1[i] != pb.1[i]).unwrap();
                    msg.push_str(&format!(" game page {:#x}+{i:#x}: orig {:#x} rust {:#x};", pa.0, pa.1[i], pb.1[i]));
                }
            }
            if !msg.is_empty() {
                bad += 1;
                if bad <= 5 {
                    println!("{} case {case} MISMATCH:{msg}  inp eax={:#x} ecx={:#x} edx={:#x} esi={:#x} edi={:#x} args={:x?}", spec.name, inp.eax, inp.ecx, inp.edx, inp.esi, inp.edi, inp.args);
                }
            }
        }
        println!("{}: {n} cases, {bad} mismatches", spec.name);
        assert_eq!(bad, 0, "{} mismatched", spec.name);
    }

    /// Random registers / stack arguments, and a random arena.
    pub fn plain(w: &mut World, nargs: usize) -> Inp {
        w.fill_arena();
        Inp {
            eax: w.rng.u32(),
            ecx: w.rng.u32(),
            edx: w.rng.u32(),
            esi: w.rng.u32(),
            edi: w.rng.u32(),
            args: (0..nargs).map(|_| w.rng.u32()).collect(),
        }
    }

    #[test]
    #[ignore]
    fn port_small_b_d3d_lost() {
        let all: &[(usize, &'static str, &'static [u8], extern "C" fn(), Option<usize>)] = &[
            (0xcf50c0, "CreateQuery", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf51c0, "GetStreamSourceFreq", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf51d0, "SetStreamSourceFreq", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf53d0, "GetPaletteEntries", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf53e0, "SetPaletteEntries", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5440, "SetTexture", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5450, "GetTexture", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54a0, "CreateStateBlock", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54b0, "GetRenderState", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54c0, "SetRenderState", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54d0, "GetClipPlane", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54e0, "SetClipPlane", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf54f0, "GetLightEnable", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5500, "LightEnable", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5510, "GetLight", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5520, "SetLight", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5570, "MultiplyTransform", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5580, "GetTransform", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5590, "SetTransform", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf55f0, "GetRenderTarget", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5600, "SetRenderTarget", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5650, "GetFrontBufferData", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5660, "GetRenderTargetData", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5670, "UpdateTexture", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5790, "GetRasterStatus", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf57e0, "GetSwapChain", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf57f0, "CreateAdditionalSwapChain", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf5840, "GetDisplayMode", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf58d0, "QueryInterface", &[0x33, 0xC0, 0xC2, 0x0C, 0x00], d3d_z12, None),
            (0xcf50d0, "DeletePatch", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5160, "GetPixelShader", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5170, "SetPixelShader", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf51a0, "GetIndices", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf51b0, "SetIndices", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5260, "GetVertexShader", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5270, "SetVertexShader", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf52a0, "GetFVF", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf52b0, "SetFVF", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf52c0, "GetVertexDeclaration", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf52d0, "SetVertexDeclaration", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5360, "SetNPatchMode", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5380, "SetSoftwareVertexProcessing", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5390, "GetScissorRect", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf53a0, "SetScissorRect", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf53b0, "GetCurrentTexturePalette", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf53c0, "SetCurrentTexturePalette", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf53f0, "ValidateDevice", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5460, "GetClipStatus", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5470, "SetClipStatus", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5480, "EndStateBlock", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5530, "GetMaterial", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5540, "SetMaterial", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5550, "GetViewport", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5560, "SetViewport", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf55d0, "GetDepthStencilSurface", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf55e0, "SetDepthStencilSurface", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5780, "SetDialogBoxMode", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf57c0, "Reset", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5830, "GetCreationParameters", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5850, "GetDeviceCaps", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf5860, "GetDirect3D", &[0x33, 0xC0, 0xC2, 0x08, 0x00], d3d_z8, None),
            (0xcf50e0, "DrawTriPatch", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf50f0, "DrawRectPatch", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5100, "GetPixelShaderConstantB", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5110, "SetPixelShaderConstantB", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5120, "GetPixelShaderConstantI", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5130, "SetPixelShaderConstantI", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5140, "GetPixelShaderConstantF", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5150, "SetPixelShaderConstantF", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5200, "GetVertexShaderConstantB", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5210, "SetVertexShaderConstantB", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5220, "GetVertexShaderConstantI", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5230, "SetVertexShaderConstantI", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5240, "GetVertexShaderConstantF", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5250, "SetVertexShaderConstantF", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5340, "DrawPrimitive", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5400, "SetSamplerState", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5410, "GetSamplerState", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5420, "SetTextureStageState", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5430, "GetTextureStageState", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5630, "ColorFill", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5820, "SetCursorProperties", &[0x33, 0xC0, 0xC2, 0x10, 0x00], d3d_z16, None),
            (0xcf5180, "CreatePixelShader", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0xA8, 0x01, 0x00, 0x00, 0xFF, 0xE0], d3d_feax_424, Some(424)),
            (0xcf51e0, "GetStreamSource", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf51f0, "SetStreamSource", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf5320, "DrawPrimitiveUP", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf5680, "UpdateSurface", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf57a0, "GetBackBuffer", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf57b0, "Present", &[0x33, 0xC0, 0xC2, 0x14, 0x00], d3d_z20, None),
            (0xcf5280, "CreateVertexShader", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x6C, 0x01, 0x00, 0x00, 0xFF, 0xE0], d3d_feax_364, Some(364)),
            (0xcf52e0, "CreateVertexDeclaration", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x58, 0x01, 0x00, 0x00, 0xFF, 0xE0], d3d_feax_344, Some(344)),
            (0xcf5300, "ProcessVertices", &[0x33, 0xC0, 0xC2, 0x1C, 0x00], d3d_z28, None),
            (0xcf5330, "DrawIndexedPrimitive", &[0x33, 0xC0, 0xC2, 0x1C, 0x00], d3d_z28, None),
            (0xcf55a0, "Clear", &[0x33, 0xC0, 0xC2, 0x1C, 0x00], d3d_z28, None),
            (0xcf5310, "DrawIndexedPrimitiveUP", &[0x33, 0xC0, 0xC2, 0x24, 0x00], d3d_z36, None),
            (0xcf5370, "GetSoftwareVertexProcessing", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf5490, "BeginStateBlock", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf55b0, "EndScene", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf55c0, "BeginScene", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf57d0, "GetNumberOfSwapChains", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf5870, "EvictManagedResources", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf5880, "GetAvailableTextureMem", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf58b0, "Release", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf58c0, "AddRef", &[0x33, 0xC0, 0xC2, 0x04, 0x00], d3d_z4, None),
            (0xcf5610, "CreateOffscreenPlainSurface", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x81, 0x90, 0x00, 0x00, 0x00, 0xFF, 0xE0], d3d_feax_144, Some(144)),
            (0xcf5640, "StretchRect", &[0x33, 0xC0, 0xC2, 0x18, 0x00], d3d_z24, None),
            (0xcf5690, "CreateDepthStencilSurface", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x74, 0xFF, 0xE0], d3d_feax_116, Some(116)),
            (0xcf56b0, "CreateRenderTarget", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x70, 0xFF, 0xE0], d3d_feax_112, Some(112)),
            (0xcf56d0, "CreateIndexBuffer", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x6C, 0xFF, 0xE0], d3d_feax_108, Some(108)),
            (0xcf56f0, "CreateVertexBuffer", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x68, 0xFF, 0xE0], d3d_feax_104, Some(104)),
            (0xcf5710, "CreateCubeTexture", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x64, 0xFF, 0xE0], d3d_feax_100, Some(100)),
            (0xcf5730, "CreateVolumeTexture", &[0x33, 0xC0, 0xC2, 0x28, 0x00], d3d_z40, None),
            (0xcf5740, "CreateTexture", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x41, 0x5C, 0xFF, 0xE0], d3d_feax_92, Some(92)),
            (0xcf5800, "ShowCursor", &[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x08, 0x00], d3d_one8, None),
            (0xcf5890, "TestCooperativeLevel", &[0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x04, 0x8B, 0x08, 0x89, 0x44, 0x24, 0x04, 0x8B, 0x51, 0x0C, 0xFF, 0xE2], d3d_fedx_12, Some(12)),
        ];
        for &(addr, name, body, thunk, fwd) in all {
            let spec = Spec {
                name,
                body,
                thunk,
                nargs: 10,
                calls: &[],
                abs: &[],
                callees: &[CAL_T],
                ign: 0,
            };
            let _ = addr;
            run_n(&spec, 1000, |w| {
                let mut i = plain(w, 10);
                if let Some(off) = fwd {
                    // args[0] -> obj; obj[1] -> inner; inner[0] -> vtbl; vtbl[off/4] = the recording stub
                    let obj = w.ptr(64);
                    let inner = w.ptr(64);
                    let vtbl = w.ptr(1024);
                    wr(obj + 4, inner);
                    wr(inner, vtbl);
                    let st = stub(0xFFFF_0001);
                    wr(vtbl + off as u32, st);
                    i.args[0] = obj;
                }
                i
            });
        }
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_num_put_sdd() {
        let spec = Spec {
            name: "num_put_sdd",
            body: &B_num_put_sdd,
            thunk: th_num_put_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pc_multi_session_info_sdd() {
        let spec = Spec {
            name: "pc_multi_session_info_sdd",
            body: &B_pc_multi_session_info_sdd,
            thunk: th_pc_multi_session_info_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_multi_session_info_base_sdd() {
        let spec = Spec {
            name: "multi_session_info_base_sdd",
            body: &B_multi_session_info_base_sdd,
            thunk: th_multi_session_info_base_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_closest_ray_hit_collector_sdd() {
        let spec = Spec {
            name: "havok_closest_ray_hit_collector_sdd",
            body: &B_havok_closest_ray_hit_collector_sdd,
            thunk: th_havok_closest_ray_hit_collector_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_multi_core_object_registration_sdd2() {
        let spec = Spec {
            name: "multi_core_object_registration_sdd2",
            body: &B_multi_core_object_registration_sdd2,
            thunk: th_multi_core_object_registration_sdd2,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_item_multi_registration_sdd() {
        let spec = Spec {
            name: "item_multi_registration_sdd",
            body: &B_item_multi_registration_sdd,
            thunk: th_item_multi_registration_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pipebomb_multi_registration_sdd() {
        let spec = Spec {
            name: "pipebomb_multi_registration_sdd",
            body: &B_pipebomb_multi_registration_sdd,
            thunk: th_pipebomb_multi_registration_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_prop_multi_registration_sdd() {
        let spec = Spec {
            name: "prop_multi_registration_sdd",
            body: &B_prop_multi_registration_sdd,
            thunk: th_prop_multi_registration_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_nearest_hit_collector_sdd() {
        let spec = Spec {
            name: "nearest_hit_collector_sdd",
            body: &B_nearest_hit_collector_sdd,
            thunk: th_nearest_hit_collector_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mempool_sdd() {
        let spec = Spec {
            name: "mempool_sdd",
            body: &B_mempool_sdd,
            thunk: th_mempool_sdd,
            nargs: 1,
            calls: &[(0x26, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            let f = if w.rng.chance(40) { 1 } else { w.rng.below(256) as u8 };
            wgb(0x2527bf7, f);
            i.ecx = w.ptr_z(0, 512);
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_item_on_register() {
        let spec = Spec {
            name: "item_on_register",
            body: &B_item_on_register,
            thunk: th_item_on_register,
            nargs: 1,
            calls: &[(0x29, 0x8470f0)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x8470f0, 0, false, Ret::PtrN)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 2 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_item_on_unregister() {
        let spec = Spec {
            name: "item_on_unregister",
            body: &B_item_on_unregister,
            thunk: th_item_on_unregister,
            nargs: 1,
            calls: &[(0x30, 0x8471f0)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x8471f0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 2 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_item_destroy_from_data() {
        let spec = Spec {
            name: "item_destroy_from_data",
            body: &B_item_destroy_from_data,
            thunk: th_item_destroy_from_data,
            nargs: 3,
            calls: &[],
            abs: &[0x2149c64],
            callees: &[
            cal(0xFFFF_0020, 1, true, Ret::Rnd),
            cal(0xFFFF_0021, 1, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 2 } else { w.rng.below(16) });
            // ent->vtbl and the optional sub-object at +0xcc, each with a vtbl whose slots are recording stubs
            let vt = w.ptr_z(2, 0x100);
            wr(ent, vt);
            wr(vt + 0x10, stub(0xFFFF_0020));
            if w.rng.chance(70) {
                let sub = w.ptr_z(1, 0x100);
                let svt = w.ptr_z(3, 0x100);
                wr(sub, svt);
                wr(svt + 0x20, stub(0xFFFF_0021));
                wr(ent + 0xcc, sub);
            } else {
                wr(ent + 0xcc, 0);
            }
            i.args[0] = id;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mover_on_register() {
        let spec = Spec {
            name: "mover_on_register",
            body: &B_mover_on_register,
            thunk: th_mover_on_register,
            nargs: 1,
            calls: &[(0x35, 0x850630), (0x44, 0x838d60)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x850630, 0, false, Ret::PtrN),
            cal(0x838d60, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 3 } else { w.rng.below(16) });
            wr(ent + 0xd0, if w.rng.chance(90) { 3 } else { w.rng.below(8) });
            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mover_on_unregister() {
        let spec = Spec {
            name: "mover_on_unregister",
            body: &B_mover_on_unregister,
            thunk: th_mover_on_unregister,
            nargs: 1,
            calls: &[(0x3c, 0x850730)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x850730, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 3 } else { w.rng.below(16) });
            wr(ent + 0xd0, if w.rng.chance(90) { 3 } else { w.rng.below(8) });
            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_on_register() {
        let spec = Spec {
            name: "npc_on_register",
            body: &B_npc_on_register,
            thunk: th_npc_on_register,
            nargs: 1,
            calls: &[(0x32, 0x85bfe0)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85bfe0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 1 } else { w.rng.below(16) });
            wrb(ent + 0x3cc, if w.rng.chance(90) { 2 } else { w.rng.below(4) as u8 });
            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_on_unregister() {
        let spec = Spec {
            name: "npc_on_unregister",
            body: &B_npc_on_unregister,
            thunk: th_npc_on_unregister,
            nargs: 1,
            calls: &[(0x39, 0x85c0e0)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85c0e0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 1 } else { w.rng.below(16) });
            wrb(ent + 0x3cc, if w.rng.chance(90) { 2 } else { w.rng.below(4) as u8 });
            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_destroy_from_data() {
        let spec = Spec {
            name: "npc_destroy_from_data",
            body: &B_npc_destroy_from_data,
            thunk: th_npc_destroy_from_data,
            nargs: 3,
            calls: &[(0x52, 0x85a9a0)],
            abs: &[0x2149c64],
            callees: &[
            cal(0xFFFF_0020, 1, true, Ret::Rnd),
            cal(0x85a9a0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 1 } else { w.rng.below(16) });
            wrb(ent + 0x3cc, if w.rng.chance(90) { 2 } else { w.rng.below(4) as u8 });
            let vt = w.ptr_z(2, 0x100);
            wr(ent, vt);
            wr(vt + 0x10, stub(0xFFFF_0020));
            wr(ent + 0x11bc, if w.rng.chance(50) { 0 } else { w.ptr(64) });
            i.args[0] = id;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pipebomb_on_register() {
        let spec = Spec {
            name: "pipebomb_on_register",
            body: &B_pipebomb_on_register,
            thunk: th_pipebomb_on_register,
            nargs: 1,
            calls: &[(0x2f, 0x85e930)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85e930, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 4 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pipebomb_on_unregister() {
        let spec = Spec {
            name: "pipebomb_on_unregister",
            body: &B_pipebomb_on_unregister,
            thunk: th_pipebomb_on_unregister,
            nargs: 1,
            calls: &[(0x36, 0x85ea20)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85ea20, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 4 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pipebomb_destroy_from_data() {
        let spec = Spec {
            name: "pipebomb_destroy_from_data",
            body: &B_pipebomb_destroy_from_data,
            thunk: th_pipebomb_destroy_from_data,
            nargs: 3,
            calls: &[],
            abs: &[0x2149c64],
            callees: &[
            cal(0xFFFF_0020, 1, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 4 } else { w.rng.below(16) });
            let vt = w.ptr_z(2, 0x100);
            wr(ent, vt);
            wr(vt + 0x10, stub(0xFFFF_0020));
            i.args[0] = id;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_prop_on_register() {
        let spec = Spec {
            name: "prop_on_register",
            body: &B_prop_on_register,
            thunk: th_prop_on_register,
            nargs: 1,
            calls: &[(0x2f, 0x85e930)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85e930, 0, false, Ret::PtrN)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 12 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_prop_on_unregister() {
        let spec = Spec {
            name: "prop_on_unregister",
            body: &B_prop_on_unregister,
            thunk: th_prop_on_unregister,
            nargs: 1,
            calls: &[(0x36, 0x85ea20)],
            abs: &[0x2149c64],
            callees: &[
            cal(0x85ea20, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 12 } else { w.rng.below(16) });

            i.args[0] = id;
            wg(0x2527bf7, 0);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_prop_destroy_from_data() {
        let spec = Spec {
            name: "prop_destroy_from_data",
            body: &B_prop_destroy_from_data,
            thunk: th_prop_destroy_from_data,
            nargs: 3,
            calls: &[],
            abs: &[0x2149c64],
            callees: &[
            cal(0xFFFF_0020, 1, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 0xc } else { w.rng.below(16) });
            let vt = w.ptr_z(2, 0x100);
            wr(ent, vt);
            wr(vt + 0x10, stub(0xFFFF_0020));
            i.args[0] = id;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_prop_free_snapshot() {
        let spec = Spec {
            name: "prop_free_snapshot",
            body: &B_prop_free_snapshot,
            thunk: th_prop_free_snapshot,
            nargs: 1,
            calls: &[(0x12, 0x86e4e0), (0x2d, 0x86d960), (0x3f, 0x86df30)],
            abs: &[0x2528b49, 0xe8c438],
            callees: &[
            cal(0x86e4e0, 0, false, Ret::Rnd),
            cal(0x86d960, 0, false, Ret::Rnd),
            cal(0x86df30, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            i.args[0] = w.ptr(64);
            wgb(0x2528b49, if w.rng.chance(50) { 0 } else { w.rng.below(255) as u8 + 1 });
            wg(0xe8c438, if w.rng.chance(50) { 8 } else { w.rng.below(16) });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_item_on_owner_left() {
        let spec = Spec {
            name: "item_on_owner_left",
            body: &B_item_on_owner_left,
            thunk: th_item_on_owner_left,
            nargs: 1,
            calls: &[(0x1d, 0x80fcf0)],
            abs: &[0x2528b48, 0x2528b49, 0x2528c14, 0x2528b56, 0x2528b2b],
            callees: &[
            cal(0x80fcf0, 1, true, Ret::Byte)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wgb(0x2528b48, if w.rng.chance(60) { 1 } else { w.rng.below(3) as u8 });
            wgb(0x2528b49, if w.rng.chance(50) { 0 } else { w.rng.below(3) as u8 });
            let host = if w.rng.chance(85) { w.ptr(0x500) } else { 0 };
            wg(0x2528c14, host);
            wgb(0x2528b56, if w.rng.chance(50) { 0 } else { 1 });
            wgb(0x2528b2b, if w.rng.chance(50) { 0 } else { 1 });
            if host != 0 {
                wrb(host + 0x3f8, if w.rng.chance(50) { 0 } else { 1 });
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mover_on_owner_left() {
        let spec = Spec {
            name: "mover_on_owner_left",
            body: &B_mover_on_owner_left,
            thunk: th_mover_on_owner_left,
            nargs: 1,
            calls: &[(0x1d, 0x80fcf0)],
            abs: &[0x2528b48, 0x2528b49, 0x2528c14, 0x2528b56, 0x2528b2b],
            callees: &[
            cal(0x80fcf0, 1, true, Ret::Byte)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wgb(0x2528b48, if w.rng.chance(60) { 1 } else { w.rng.below(3) as u8 });
            wgb(0x2528b49, if w.rng.chance(50) { 0 } else { w.rng.below(3) as u8 });
            let host = if w.rng.chance(85) { w.ptr(0x500) } else { 0 };
            wg(0x2528c14, host);
            wgb(0x2528b56, if w.rng.chance(50) { 0 } else { 1 });
            wgb(0x2528b2b, if w.rng.chance(50) { 0 } else { 1 });
            if host != 0 {
                wrb(host + 0x3f8, if w.rng.chance(50) { 0 } else { 1 });
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_large_convoy_objective_get_start_message() {
        let spec = Spec {
            name: "large_convoy_objective_get_start_message",
            body: &B_large_convoy_objective_get_start_message,
            thunk: th_large_convoy_objective_get_start_message,
            nargs: 0,
            calls: &[(0xc, 0x7f49e0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_liberator_objective_get_start_message() {
        let spec = Spec {
            name: "liberator_objective_get_start_message",
            body: &B_liberator_objective_get_start_message,
            thunk: th_liberator_objective_get_start_message,
            nargs: 0,
            calls: &[(0xc, 0x7f4ad0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_hostage_diversion_get_title() {
        let spec = Spec {
            name: "hostage_diversion_get_title",
            body: &B_hostage_diversion_get_title,
            thunk: th_hostage_diversion_get_title,
            nargs: 0,
            calls: &[(0xc, 0x7f49e0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mugging_diversion_get_title() {
        let spec = Spec {
            name: "mugging_diversion_get_title",
            body: &B_mugging_diversion_get_title,
            thunk: th_mugging_diversion_get_title,
            nargs: 0,
            calls: &[(0xc, 0x7f4ad0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_poker_diversion_get_title() {
        let spec = Spec {
            name: "poker_diversion_get_title",
            body: &B_poker_diversion_get_title,
            thunk: th_poker_diversion_get_title,
            nargs: 0,
            calls: &[(0xc, 0x7f4ad0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_hitman_diversion_get_title() {
        let spec = Spec {
            name: "hitman_diversion_get_title",
            body: &B_hitman_diversion_get_title,
            thunk: th_hitman_diversion_get_title,
            nargs: 0,
            calls: &[(0xc, 0x7f49e0)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0x7f4ad0, 0, false, Ret::Ptr)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_hostage_diversion_get_start_message() {
        let spec = Spec {
            name: "hostage_diversion_get_start_message",
            body: &B_hostage_diversion_get_start_message,
            thunk: th_hostage_diversion_get_start_message,
            nargs: 2,
            calls: &[(0xc, 0x7f49e0), (0x1a, 0xc14c40)],
            abs: &[],
            callees: &[
            cal(0x7f49e0, 0, false, Ret::Ptr),
            cal(0xc14c40, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 2)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mugging_diversion_get_start_message() {
        let spec = Spec {
            name: "mugging_diversion_get_start_message",
            body: &B_mugging_diversion_get_start_message,
            thunk: th_mugging_diversion_get_start_message,
            nargs: 2,
            calls: &[(0xf, 0xc14b50)],
            abs: &[],
            callees: &[
            cal(0xc14b50, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 2)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_hostage_diversion_create_instance() {
        let spec = Spec {
            name: "hostage_diversion_create_instance",
            body: &B_hostage_diversion_create_instance,
            thunk: th_hostage_diversion_create_instance,
            nargs: 1,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0030, 2, true, Ret::PtrN)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let alloc = w.ptr_z(1, 64);
            let vt = w.ptr_z(2, 256);
            wr(alloc, vt);
            wr(vt + 0x1c, stub(0xFFFF_0030));
            i.args[0] = alloc;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_poker_diversion_create_instance() {
        let spec = Spec {
            name: "poker_diversion_create_instance",
            body: &B_poker_diversion_create_instance,
            thunk: th_poker_diversion_create_instance,
            nargs: 1,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0030, 2, true, Ret::PtrN)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let alloc = w.ptr_z(1, 64);
            let vt = w.ptr_z(2, 256);
            wr(alloc, vt);
            wr(vt + 0x1c, stub(0xFFFF_0030));
            i.args[0] = alloc;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_heli_info_load_from_table() {
        let spec = Spec {
            name: "heli_info_load_from_table",
            body: &B_heli_info_load_from_table,
            thunk: th_heli_info_load_from_table,
            nargs: 2,
            calls: &[(0x0, 0x53d7c0)],
            abs: &[],
            callees: &[
            cal(0x53d7c0, 2, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 2)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_piracy_info_load_from_table() {
        let spec = Spec {
            name: "piracy_info_load_from_table",
            body: &B_piracy_info_load_from_table,
            thunk: th_piracy_info_load_from_table,
            nargs: 2,
            calls: &[(0xf, 0x53d7c0)],
            abs: &[0x2527580],
            callees: &[
            cal(0x53d7c0, 2, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 2);
            i.ecx = w.ptr_z(0, 64);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_near_miss_complication_load_from_table() {
        let spec = Spec {
            name: "near_miss_complication_load_from_table",
            body: &B_near_miss_complication_load_from_table,
            thunk: th_near_miss_complication_load_from_table,
            nargs: 1,
            calls: &[(0x9, 0x66db90), (0x17, 0xb747d0)],
            abs: &[],
            callees: &[
            cal(0x66db90, 1, true, Ret::Rnd),
            cal(0xb747d0, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            i.ecx = w.ptr_z(0, 128);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mempool_base_dtor() {
        let spec = Spec {
            name: "mempool_base_dtor",
            body: &B_mempool_base_dtor,
            thunk: th_mempool_base_dtor,
            nargs: 0,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 64);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_size() {
        let spec = Spec {
            name: "page_mempool_size",
            body: &B_page_mempool_size,
            thunk: th_page_mempool_size,
            nargs: 0,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 128);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_used_bytes() {
        let spec = Spec {
            name: "page_mempool_used_bytes",
            body: &B_page_mempool_used_bytes,
            thunk: th_page_mempool_used_bytes,
            nargs: 0,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 128);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_align_to() {
        let spec = Spec {
            name: "page_mempool_align_to",
            body: &B_page_mempool_align_to,
            thunk: th_page_mempool_align_to,
            nargs: 1,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 1)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_release() {
        let spec = Spec {
            name: "page_mempool_release",
            body: &B_page_mempool_release,
            thunk: th_page_mempool_release,
            nargs: 1,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 1)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_release_to() {
        let spec = Spec {
            name: "page_mempool_release_to",
            body: &B_page_mempool_release_to,
            thunk: th_page_mempool_release_to,
            nargs: 1,
            calls: &[],
            abs: &[],
            callees: &[
            
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 1)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_text_entry_hide() {
        let spec = Spec {
            name: "ovl_text_entry_hide",
            body: &B_ovl_text_entry_hide,
            thunk: th_ovl_text_entry_hide,
            nargs: 0,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0030, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 64);
            wr(i.ecx + 8, stub(0xFFFF_0030));
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_text_entry_update() {
        let spec = Spec {
            name: "ovl_text_entry_update",
            body: &B_ovl_text_entry_update,
            thunk: th_ovl_text_entry_update,
            nargs: 0,
            calls: &[(0x3, 0x5002b0)],
            abs: &[0x2528bcd, 0x2528038],
            callees: &[
            cal(0x5002b0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 64);
            wrb(i.ecx + 0xc, if w.rng.chance(50) { 1 } else { w.rng.below(3) as u8 });
            wgb(0x2528bcd, if w.rng.chance(50) { 0 } else { 1 });
            wgb(0x2528038, if w.rng.chance(50) { 0 } else { 1 });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_framerate_hide() {
        let spec = Spec {
            name: "ovl_framerate_hide",
            body: &B_ovl_framerate_hide,
            thunk: th_ovl_framerate_hide,
            nargs: 0,
            calls: &[(0x7, 0xc15720), (0x13, 0x739930), (0x2a, 0x699910)],
            abs: &[0xf9f554, 0xf9f54c],
            callees: &[
            cal(0xc15720, 0, false, Ret::Rnd),
            cal(0x739930, 1, false, Ret::Rnd),
            cal(0x699910, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_selection_menu_hide() {
        let spec = Spec {
            name: "ovl_selection_menu_hide",
            body: &B_ovl_selection_menu_hide,
            thunk: th_ovl_selection_menu_hide,
            nargs: 0,
            calls: &[(0x0, 0x7483e0), (0x5, 0x747930), (0x10, 0x740730)],
            abs: &[],
            callees: &[
            cal(0x7483e0, 0, false, Ret::Rnd),
            cal(0x747930, 0, false, Ret::Rnd),
            cal(0x740730, 2, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            plain(w, 0)
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_all_ray_hit_collector_sdd() {
        let spec = Spec {
            name: "havok_all_ray_hit_collector_sdd",
            body: &B_havok_all_ray_hit_collector_sdd,
            thunk: th_havok_all_ray_hit_collector_sdd,
            nargs: 1,
            calls: &[(0x3, 0x6f4f20), (0x29, 0xcde870)],
            abs: &[0x2527bf7, 0xdb80cc, 0xdb80d0],
            callees: &[
            cal(0x6f4f20, 0, false, Ret::Rnd),
            cal(0xFFFF_0010, 1, true, Ret::Rnd).ign(31),
            cal(0xcde870, 0, false, Ret::Rnd),
            cal(0xFFFF_0011, 1, true, Ret::Rnd).ign(31)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_closest_linear_cast_collector_sdd() {
        let spec = Spec {
            name: "closest_linear_cast_collector_sdd",
            body: &B_closest_linear_cast_collector_sdd,
            thunk: th_closest_linear_cast_collector_sdd,
            nargs: 1,
            calls: &[(0x2b, 0xd2b390)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b390, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_object_collector_sdd() {
        let spec = Spec {
            name: "object_collector_sdd",
            body: &B_object_collector_sdd,
            thunk: th_object_collector_sdd,
            nargs: 1,
            calls: &[(0x2b, 0xd2b390)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b390, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_custom_character_collector_sdd() {
        let spec = Spec {
            name: "custom_character_collector_sdd",
            body: &B_custom_character_collector_sdd,
            thunk: th_custom_character_collector_sdd,
            nargs: 1,
            calls: &[(0x3, 0x72b1b0), (0x31, 0xd2b390)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0x72b1b0, 0, false, Ret::Rnd),
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b390, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_all_linear_cast_collector_sdd() {
        let spec = Spec {
            name: "all_linear_cast_collector_sdd",
            body: &B_all_linear_cast_collector_sdd,
            thunk: th_all_linear_cast_collector_sdd,
            nargs: 1,
            calls: &[(0x3, 0x6f5220), (0x31, 0xd2b390)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0x6f5220, 0, false, Ret::Rnd),
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b390, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_game_filter_sdd() {
        let spec = Spec {
            name: "havok_game_filter_sdd",
            body: &B_havok_game_filter_sdd,
            thunk: th_havok_game_filter_sdd,
            nargs: 1,
            calls: &[(0x5, 0x70f020), (0x27, 0xd2b7d0)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0x70f020, 0, false, Ret::Rnd),
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b7d0, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_never_collide_filter_sdd() {
        let spec = Spec {
            name: "never_collide_filter_sdd",
            body: &B_never_collide_filter_sdd,
            thunk: th_never_collide_filter_sdd,
            nargs: 1,
            calls: &[(0x5, 0x70f020), (0x27, 0xd2b7d0)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0x70f020, 0, false, Ret::Rnd),
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b7d0, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_filter_shape_collection_sdd() {
        let spec = Spec {
            name: "havok_filter_shape_collection_sdd",
            body: &B_havok_filter_shape_collection_sdd,
            thunk: th_havok_filter_shape_collection_sdd,
            nargs: 1,
            calls: &[(0x3, 0xd40e30), (0x25, 0xd2b7d0)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0xd40e30, 0, false, Ret::Rnd),
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b7d0, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_navmesh_shape_sdd() {
        let spec = Spec {
            name: "havok_navmesh_shape_sdd",
            body: &B_havok_navmesh_shape_sdd,
            thunk: th_havok_navmesh_shape_sdd,
            nargs: 1,
            calls: &[(0x37, 0xd2b7d0)],
            abs: &[0x31bf2bc, 0xdb807c],
            callees: &[
            cal(0xFFFF_0012, 1, true, Ret::Ptr).ign(31),
            cal(0xd2b7d0, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            wg(0xdb807c, stub(0xFFFF_0012));
            wg(0xdb80cc, stub(0xFFFF_0010));
            wg(0xdb80d0, stub(0xFFFF_0011));
            wg(0x31bf2bc, w.rng.u32());
            wgb(0x2527bf7, if w.rng.chance(30) { 1 } else { w.rng.below(256) as u8 });
            i.ecx = if w.rng.chance(0) { 0 } else { w.ptr(512) };
            i.args[0] = if w.rng.chance(60) { w.rng.u32() | 1 } else { w.rng.u32() & !1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_orientation_heading_compress() {
        let spec = Spec {
            name: "npc_orientation_heading_compress",
            body: &B_npc_orientation_heading_compress,
            thunk: th_npc_orientation_heading_compress,
            nargs: 4,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0xFFFF_0041, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 4);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            for k in 0..4 {
                i.args[k] = if w.rng.chance(15) { 0 } else { w.ptr(128) };
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_orientation_quaternion_compress() {
        let spec = Spec {
            name: "npc_orientation_quaternion_compress",
            body: &B_npc_orientation_quaternion_compress,
            thunk: th_npc_orientation_quaternion_compress,
            nargs: 4,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0xFFFF_0041, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 4);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            for k in 0..4 {
                i.args[k] = if w.rng.chance(15) { 0 } else { w.ptr(128) };
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_orientation_heading_decompress() {
        let spec = Spec {
            name: "npc_orientation_heading_decompress",
            body: &B_npc_orientation_heading_decompress,
            thunk: th_npc_orientation_heading_decompress,
            nargs: 3,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0xFFFF_0041, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            for k in 0..3 {
                i.args[k] = if w.rng.chance(15) { 0 } else { w.ptr(128) };
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_orientation_quaternion_decompress() {
        let spec = Spec {
            name: "npc_orientation_quaternion_decompress",
            body: &B_npc_orientation_quaternion_decompress,
            thunk: th_npc_orientation_quaternion_decompress,
            nargs: 3,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0xFFFF_0041, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            for k in 0..3 {
                i.args[k] = if w.rng.chance(15) { 0 } else { w.ptr(128) };
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_spline_pathfind_uncompressed_compress() {
        let spec = Spec {
            name: "npc_spline_pathfind_uncompressed_compress",
            body: &B_npc_spline_pathfind_uncompressed_compress,
            thunk: th_npc_spline_pathfind_uncompressed_compress,
            nargs: 4,
            calls: &[(0x24, 0x8517a0)],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0x8517a0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 4);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            i.args[3] = w.ptr_z(1, 256);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_spline_pathfind_uncompressed_decompress() {
        let spec = Spec {
            name: "npc_spline_pathfind_uncompressed_decompress",
            body: &B_npc_spline_pathfind_uncompressed_decompress,
            thunk: th_npc_spline_pathfind_uncompressed_decompress,
            nargs: 3,
            calls: &[(0x1d, 0x8517a0)],
            abs: &[],
            callees: &[
            cal(0xFFFF_0041, 3, true, Ret::Rnd),
            cal(0x8517a0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            i.ecx = w.ptr_z(0, 64);
            let vt = w.ptr_z(2, 64);
            wr(i.ecx + 4, vt);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mempool_ctor() {
        let spec = Spec {
            name: "mempool_ctor",
            body: &B_mempool_ctor,
            thunk: th_mempool_ctor,
            nargs: 0,
            calls: &[(0x14, 0xc990f0)],
            abs: &[],
            callees: &[
            cal(0xc990f0, 3, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 256);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mempool_ctor_2() {
        let spec = Spec {
            name: "mempool_ctor_2",
            body: &B_mempool_ctor_2,
            thunk: th_mempool_ctor_2,
            nargs: 0,
            calls: &[(0xd, 0xc990f0)],
            abs: &[],
            callees: &[
            cal(0xc990f0, 3, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.esi = w.ptr_z(0, 256);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mempool_ctor_3() {
        let spec = Spec {
            name: "mempool_ctor_3",
            body: &B_mempool_ctor_3,
            thunk: th_mempool_ctor_3,
            nargs: 1,
            calls: &[(0x9, 0xc00bb0)],
            abs: &[],
            callees: &[
            cal(0xc00bb0, 5, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            i.esi = w.ptr_z(0, 256);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_can_alloc() {
        let spec = Spec {
            name: "page_mempool_can_alloc",
            body: &B_page_mempool_can_alloc,
            thunk: th_page_mempool_can_alloc,
            nargs: 2,
            calls: &[(0x11, 0xc1cba0)],
            abs: &[],
            callees: &[
            cal(0xc1cba0, 1, true, Ret::Byte)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 2);
            let this = w.ptr_z(0, 512);
            let count = 1 + w.rng.below(6);
            let esz = 16 + 4 * w.rng.below(8);
            let base = w.ptr_z(1, 8 * 64);
            wr(this + 0x4c, esz);
            wr(this + 0x50, count);
            wr(this + 0x38, base);
            wr(this + 0x54, if w.rng.chance(10) { 0xFFFF_FFFF } else { w.rng.below(count) });
            for k in 0..count {
                let e = base + k * esz;
                wr(e, w.rng.below(200));
                let next = if k + 1 < count && w.rng.chance(70) { k + 1 + w.rng.below(count - k - 1) } else { 0xFFFF_FFFF };
                wr(e + 4, next);
            }
            i.ecx = this;
            i.args[0] = w.rng.below(220);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_page_mempool_reset() {
        let spec = Spec {
            name: "page_mempool_reset",
            body: &B_page_mempool_reset,
            thunk: th_page_mempool_reset,
            nargs: 0,
            calls: &[(0xd, 0xca0bc0)],
            abs: &[],
            callees: &[
            cal(0xca0bc0, 3, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            let this = w.ptr_z(0, 512);
            let base = w.ptr_z(1, 1024);
            wr(this + 0x38, base);
            wr(this + 0x3c, w.rng.u32());
            wr(this + 0x50, if w.rng.chance(95) { w.rng.below(50) + 1 } else { w.rng.u32() | 1 });
            i.ecx = this;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_object_get_attach_point() {
        let spec = Spec {
            name: "object_get_attach_point",
            body: &B_object_get_attach_point,
            thunk: th_object_get_attach_point,
            nargs: 2,
            calls: &[(0x1d, 0x6fc990)],
            abs: &[],
            callees: &[
            cal(0x6fc990, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 2);
            i.ecx = w.ptr_z(0, 256);
            wr(i.ecx + 0x7c, if w.rng.chance(30) { 0xFFFF_FFFF } else { w.rng.below(50) });
            i.args[0] = w.ptr(64);
            i.args[1] = if w.rng.chance(60) { 0xFFFF_FFFF } else { w.rng.u32() };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_object_item_activate() {
        let spec = Spec {
            name: "object_item_activate",
            body: &B_object_item_activate,
            thunk: th_object_item_activate,
            nargs: 1,
            calls: &[(0x22, 0x925780)],
            abs: &[],
            callees: &[
            cal(0xFFFF_0050, 1, true, Ret::Rnd),
            cal(0x925780, 1, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 1);
            let this = w.ptr_z(0, 512);
            i.ecx = this;
            if w.rng.chance(70) {
                let sub = w.ptr_z(1, 128);
                let vt = w.ptr_z(2, 128);
                wr(sub, vt);
                wr(vt + 0x20, stub(0xFFFF_0050));
                wr(this + 0xcc, sub);
            } else {
                wr(this + 0xcc, 0);
            }
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_ingame_main_menu_show() {
        let spec = Spec {
            name: "ovl_ingame_main_menu_show",
            body: &B_ovl_ingame_main_menu_show,
            thunk: th_ovl_ingame_main_menu_show,
            nargs: 0,
            calls: &[(0xc, 0x793d60), (0x2a, 0x4959a0), (0x3b, 0x926860)],
            abs: &[0xdd1c50, 0x21703d4],
            callees: &[
            cal(0x793d60, 1, false, Ret::Rnd),
            cal(0x4959a0, 4, false, Ret::Rnd),
            cal(0x926860, 3, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 256);
            wr(i.ecx + 8, w.rng.below(7));
            wr(i.ecx + 0x3c, w.rng.any_float());
            wg(0x21703d4, w.rng.u32());
            wgb(0x2528b48, if w.rng.chance(50) { 1 } else { 0 });
            wg(0x25f5b0c, if w.rng.chance(50) { 0 } else { 1 });
            wg(0x25f5ae8, if w.rng.chance(60) { 0 } else { w.rng.below(3) });
            wg(0x25f5be8, w.rng.u32());
            wg(0xdd1c50, w.rng.any_float());
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_ingame_main_menu_hide() {
        let spec = Spec {
            name: "ovl_ingame_main_menu_hide",
            body: &B_ovl_ingame_main_menu_hide,
            thunk: th_ovl_ingame_main_menu_hide,
            nargs: 0,
            calls: &[(0x2, 0x793d60), (0x41, 0x4a33f0), (0x6a, 0x926860)],
            abs: &[0x25f5b0c, 0x25f5b10, 0x25f5ae8, 0xdd1c50, 0x25f5be8, 0xdd5250, 0x21703d4],
            callees: &[
            cal(0x793d60, 1, false, Ret::Rnd),
            cal(0x4a33f0, 1, false, Ret::Rnd),
            cal(0xFFFF_0051, 0, false, Ret::Rnd),
            cal(0x926860, 3, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            wg(0x21703d4, w.rng.u32());
            wg(0x25f5b0c, if w.rng.chance(30) { 1 } else { 0 });
            let idx = w.rng.below(3);
            wg(0x25f5ae8, if w.rng.chance(70) { 0 } else { w.rng.below(3) });
            wg(0x25f5be8, w.rng.u32());
            wg(0xdd1c50, w.rng.any_float());
            // the callee 0x4a33f0 stub changes nothing, so the table index is whatever [0x25f5ae8] is after the clear: 0
            let _ = idx;
            wg(0xdd5250, if w.rng.chance(60) { stub(0xFFFF_0051) } else { 0 });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_ovl_ingame_main_menu_update() {
        let spec = Spec {
            name: "ovl_ingame_main_menu_update",
            body: &B_ovl_ingame_main_menu_update,
            thunk: th_ovl_ingame_main_menu_update,
            nargs: 0,
            calls: &[(0xc, 0x4ffa30), (0x18, 0x4ffa30), (0x21, 0x500860), (0x3a, 0x7f7830), (0x41, 0x5002b0), (0x5b, 0x5002b0), (0x72, 0x5002b0)],
            abs: &[0x2528b48],
            callees: &[
            cal(0x4ffa30, 0, false, Ret::Bool),
            cal(0x500860, 0, false, Ret::Bool),
            cal(0x7f7830, 0, false, Ret::Rnd),
            cal(0x5002b0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            i.ecx = w.ptr_z(0, 256);
            wr(i.ecx + 8, w.rng.below(7));
            wgb(0x2528b48, if w.rng.chance(50) { 1 } else { 0 });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_player_destroy_from_data() {
        let spec = Spec {
            name: "player_destroy_from_data",
            body: &B_player_destroy_from_data,
            thunk: th_player_destroy_from_data,
            nargs: 3,
            calls: &[(0x3c, 0x68f6b0)],
            abs: &[0x2149c64, 0x21703d4],
            callees: &[
            cal(0x68f6b0, 0, false, Ret::Rnd),
            cal(0xFFFF_0020, 1, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let ent = w.ptr_z(0, 0x1400);
            let id = if w.rng.chance(6) { 0 } else { w.rng.u32() | 1 };
            let idx = (id & 0xFFF) as usize;
            let slot = if w.rng.chance(6) { 0 } else { ent };
            wg(0x2149c64 + idx * 16, slot);
            wr(ent + 0x44, if w.rng.chance(90) { id } else { w.rng.u32() });
            wr(ent + 0x4c, if w.rng.chance(90) { 1 } else { w.rng.below(16) });
            wrb(ent + 0x3cc, if w.rng.chance(90) { 1 } else { w.rng.below(4) as u8 });
            wg(0x21703d4, if w.rng.chance(90) { w.rng.u32() } else { ent });
            let vt = w.ptr_z(2, 0x100);
            wr(ent, vt);
            wr(vt + 0x10, stub(0xFFFF_0020));
            i.args[0] = id;
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_player_on_net_tick() {
        let spec = Spec {
            name: "player_on_net_tick",
            body: &B_player_on_net_tick,
            thunk: th_player_on_net_tick,
            nargs: 0,
            calls: &[(0xb, 0x9bced0)],
            abs: &[0x21703d4],
            callees: &[
            cal(0x9bced0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            wg(0x21703d4, if w.rng.chance(70) { w.rng.u32() } else { 0 });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_npc_on_net_tick() {
        let spec = Spec {
            name: "npc_on_net_tick",
            body: &B_npc_on_net_tick,
            thunk: th_npc_on_net_tick,
            nargs: 0,
            calls: &[(0x49, 0x9bced0)],
            abs: &[0x215ffc0, 0x2528c14],
            callees: &[
            cal(0x9bced0, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0);
            let n = 1 + w.rng.below(5) as usize;
            let objs = w.ptr_z(1, 0x800);
            let host = w.ptr_z(2, 0x500);
            let cur = w.ptr_z(3, 0x300);
            wg(0x2528c14, if w.rng.chance(85) { host } else { 0 });
            wr(host + 0x408, if w.rng.chance(85) { cur } else { 0 });
            wrb(cur + 0x1b0, w.rng.below(3) as u8);
            let zone0 = arena() as u32;
            let mut prev = 0u32;
            let mut first = 0u32;
            for k in 0..n {
                let nd = zone0 + (k as u32) * 0x1200 + 0x20;
                wr(nd + 0x80, if w.rng.chance(50) { 0 } else { w.rng.below(16) });
                let o = if w.rng.chance(15) { 0 } else { objs + (w.rng.below(4) * 0x100) };
                wr(nd + 0xb0, o);
                if o != 0 {
                    wrb(o, w.rng.below(3) as u8);
                    wr(o + 0x67c, w.rng.below(4));
                }
                wr(nd + 0x1160, 0);
                if k == 0 {
                    first = nd;
                } else {
                    wr(prev + 0x1160, nd);
                }
                prev = nd;
            }
            if n > 1 && w.rng.chance(50) {
                wr(prev + 0x1160, first);
            }
            wg(0x215ffc0, if w.rng.chance(95) { first } else { 0 });
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mover_data_compressor_compress() {
        let spec = Spec {
            name: "mover_data_compressor_compress",
            body: &B_mover_data_compressor_compress,
            thunk: th_mover_data_compressor_compress,
            nargs: 4,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0040, 4, true, Ret::Rnd),
            cal(0xFFFF_0042, 4, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 4);
            let this = w.ptr_z(0, 128);
            let vt = w.ptr_z(2, 64);
            let vt2 = w.ptr_z(3, 64);
            wr(this + 4, vt);
            wr(this + 0xc, vt2);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            wr(vt2, stub(0xFFFF_0042));
            wr(vt2 + 4, stub(0xFFFF_0043));
            i.ecx = this;
            let o = w.ptr_z(1, 256);
            wrb(o + 0x34, w.rng.below(256) as u8);
            i.args[0] = if w.rng.chance(8) { 0 } else { w.rng.u32() | 1 };
            i.args[1] = if w.rng.chance(8) { 0 } else { o };
            i.args[3] = if w.rng.chance(8) { 0 } else { w.rng.u32() | 1 };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_mover_data_compressor_decompress() {
        let spec = Spec {
            name: "mover_data_compressor_decompress",
            body: &B_mover_data_compressor_decompress,
            thunk: th_mover_data_compressor_decompress,
            nargs: 3,
            calls: &[],
            abs: &[],
            callees: &[
            cal(0xFFFF_0041, 3, true, Ret::Rnd),
            cal(0xFFFF_0043, 3, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 3);
            let this = w.ptr_z(0, 128);
            let vt = w.ptr_z(2, 64);
            let vt2 = w.ptr_z(3, 64);
            wr(this + 4, vt);
            wr(this + 0xc, vt2);
            wr(vt, stub(0xFFFF_0040));
            wr(vt + 4, stub(0xFFFF_0041));
            wr(vt2, stub(0xFFFF_0042));
            wr(vt2 + 4, stub(0xFFFF_0043));
            i.ecx = this;
            let o = w.ptr_z(1, 256);
            wrb(o + 0x34, w.rng.below(256) as u8);
            i.args[0] = if w.rng.chance(8) { 0 } else { w.rng.u32() | 1 };
            i.args[1] = if w.rng.chance(8) { 0 } else { o };

            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_killed_data_compressor_compress() {
        let spec = Spec {
            name: "killed_data_compressor_compress",
            body: &B_killed_data_compressor_compress,
            thunk: th_killed_data_compressor_compress,
            nargs: 4,
            calls: &[(0x1f, 0x7fd100), (0x27, 0x804f70), (0x30, 0x7fd3c0), (0x38, 0x804f70), (0x41, 0x7fd3c0), (0x49, 0xb73b70), (0x52, 0x7fd100)],
            abs: &[],
            callees: &[
            cal(0x7fd100, 0, false, Ret::Rnd),
            cal(0x804f70, 0, false, Ret::Rnd),
            cal(0x7fd3c0, 0, false, Ret::Rnd),
            cal(0xb73b70, 0, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 4);
            let o = w.ptr_z(1, 128);
            i.args[1] = if w.rng.chance(8) { 0 } else { o };
            i.args[0] = if w.rng.chance(8) { 0 } else { w.rng.u32() | 1 };
            i.args[3] = if w.rng.chance(8) { 0 } else { w.ptr_z(3, 128) };
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_human_torch_instance_load_from_table() {
        let spec = Spec {
            name: "human_torch_instance_load_from_table",
            body: &B_human_torch_instance_load_from_table,
            thunk: th_human_torch_instance_load_from_table,
            nargs: 2,
            calls: &[(0x12, 0x9d7bd0), (0x1c, 0x9ded00), (0x30, 0x55a930), (0x3c, 0x9ded00), (0x50, 0x55a930), (0x57, 0x9c3a90), (0x61, 0x9c3b00), (0x71, 0x55a930), (0x7c, 0x9c3a90), (0x86, 0x9c3b00), (0xa7, 0x55a930)],
            abs: &[0x21703d4, 0x2529623],
            callees: &[
            cal(0x9d7bd0, 0, false, Ret::PtrN),
            cal(0x9ded00, 2, false, Ret::Rnd),
            cal(0x55a930, 4, true, Ret::Rnd),
            cal(0x9c3a90, 0, false, Ret::Bool),
            cal(0x9c3b00, 1, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 2);
            i.ecx = w.ptr_z(0, 64);
            wg(0x21703d4, w.rng.u32() | 0x10000);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_pimping_instance_load_from_table() {
        let spec = Spec {
            name: "pimping_instance_load_from_table",
            body: &B_pimping_instance_load_from_table,
            thunk: th_pimping_instance_load_from_table,
            nargs: 2,
            calls: &[(0xf, 0x656590), (0x1a, 0x9d7bd0), (0x25, 0x9de310), (0x30, 0x9e0350), (0x38, 0x9ded00), (0x48, 0x9de310), (0x53, 0x9e0350), (0x5b, 0x9ded00)],
            abs: &[0x21703d4],
            callees: &[
            cal(0x656590, 2, true, Ret::Rnd),
            cal(0x9d7bd0, 0, false, Ret::PtrN),
            cal(0x9de310, 1, false, Ret::Rnd),
            cal(0x9e0350, 2, true, Ret::Rnd),
            cal(0x9ded00, 2, false, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 2);
            i.ecx = w.ptr_z(0, 0x300);
            wg(0x21703d4, w.rng.u32() | 0x10000);
            i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_all_ray_hit_collector_ctor() {
        let spec = Spec {
            name: "havok_all_ray_hit_collector_ctor",
            body: &B_havok_all_ray_hit_collector_ctor,
            thunk: th_havok_all_ray_hit_collector_ctor,
            nargs: 0,
            calls: &[(0x2b, 0x402450), (0x64, 0x402450)],
            abs: &[],
            callees: &[
            cal(0x402450, 4, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0); i.ecx = w.ptr_z(0, 0x400); i
        });
    }

    #[test]
    #[ignore]
    #[allow(unused_mut)]
    fn port_small_b_havok_all_linear_cast_collector_ctor() {
        let spec = Spec {
            name: "havok_all_linear_cast_collector_ctor",
            body: &B_havok_all_linear_cast_collector_ctor,
            thunk: th_havok_all_linear_cast_collector_ctor,
            nargs: 0,
            calls: &[(0x4b, 0x402450)],
            abs: &[0xe5c288],
            callees: &[
            cal(0x402450, 4, true, Ret::Rnd)
            ],
            ign: 0,
        };
        run(&spec, |w| {
            let mut i = plain(w, 0); i.esi = w.ptr_z(0, 0xe00); wg(0xe5c288, w.rng.any_float()); i
        });
    }
}
