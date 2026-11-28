//! Ported functions, round 8 trial list h1: small get_title / get_start_message shapes, a vint
//! draw-clip end, vint_element_point::init, vint_object_tween::get_start_value_type, a stunt
//! diversion update, and a fuzz_crime predicate with a float argument.
//!
//! Every port runs behind a `pushad` thunk (XMM0-7 saved, see `thunk!`). Callees stay original
//! and are called by address through `St::call`, which runs them with the registers the original
//! sets up and hands back what the callee left in EAX / ECX / EDX / EBX / ESI / EDI / EBP.
//! ESP is restored by the call helper, so a callee that pops its own stack arguments or not both
//! match the original.
//!
//! Checks: per-function call counters only (every body calls out).

#![allow(unused_imports, dead_code, unused_macros)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 8;

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

/// The seven general registers as a straight-line asm function would hold them. `call` runs a
/// game function with exactly these registers (EBX / EBP and the callee's own stack arguments
/// included) and leaves in the struct what the callee left. ESP is restored from a TEB scratch
/// slot, so the callee may pop its arguments or not. The callee sees `args` (at most 8).
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

    /// Call a game function by address; returns its EAX.
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

// --- get_title / get_start_message shape ---------------------------------------------------------
// push esi; push edi; edi = a constant; esi = a constant; call 0x7F49E0; pop edi; pop esi; ret.

macro_rules! gt {
    ($e:ident, $t:ident, $i:expr, $ret:literal, $f:expr, $edi:expr, $esi:expr) => {
        extern "C" fn $e(r: &mut Pushad, _a: *const u32) {
            unsafe {
                count($i);
                // push esi / edi: ESI / EDI come back as they were; EBX / EBP flow out
                let mut s = St::of(r);
                s.esi = $esi;
                s.edi = $edi;
                s.call($f, &[]);
                r.eax = s.eax;
                r.ecx = s.ecx;
                r.edx = s.edx;
                r.ebx = s.ebx;
                r.ebp = s.ebp;
            }
        }
        thunk!($t, $e, $ret);
    };
}

gt!(e_601e20, t_601e20, 0, "ret", 0x007F_49E0, 0x00DF_3118, 0x00DF_3134);
gt!(e_609940, t_609940, 1, "ret", 0x007F_49E0, 0x00DF_4140, 0x00DF_4160);
gt!(e_5750d0, t_5750d0, 2, "ret", 0x007F_49E0, 0x00DE_8984, 0x00DE_89B4);

// --- vint_draw_clip_end --------------------------------------------------------------------------
// eax = [0x22FDC20]; push esi; esi = [0x22FDC1C]; push edi; push eax; push 0; edi = 0;
// call 0xD14D20; add esp, 8; pop edi; pop esi; ret.

extern "C" fn e_7f0a90(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(3);
        let v = rd(0x022F_DC20);
        let mut s = St::of(r);
        s.eax = v;
        s.esi = rd(0x022F_DC1C);
        s.edi = 0;
        s.call(0x00D1_4D20, &[0, v]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}
thunk!(t_7f0a90, e_7f0a90, "ret");

// --- vint_element_point::init --------------------------------------------------------------------
// call 0xB95A60 (ECX = this); if al != 0: fld1; mov al, 1; fst [ecx+0x70]; fstp [ecx+0x74]; ret.
// ECX is the callee's leftover in the stores (the original stores through it).

extern "C" fn e_ba47d0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(4);
        let mut s = St::of(r);
        s.call(0x00B9_5A60, &[]);
        if s.eax as u8 != 0 {
            s.eax = (s.eax & !0xFF) | 1;
            let c = s.ecx as usize;
            wf(c + 0x70, 1.0);
            wf(c + 0x74, 1.0);
        }
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
        r.esi = s.esi;
        r.edi = s.edi;
    }
}
thunk!(t_ba47d0, e_ba47d0, "ret");

// --- vint_object_tween::get_start_value_type -----------------------------------------------------
// eax = [ecx+0xDC]; push esi; esi = arg1; push eax; call 0xB93590 (stdcall, one argument);
// mov al, 1; pop esi; ret 8.

extern "C" fn e_b9fd80(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(5);
        let arg1 = rd(a as usize);
        let mut s = St::of(r);
        s.eax = rd(r.ecx as usize + 0xDC);
        s.esi = arg1;
        let v = s.eax;
        s.call(0x00B9_3590, &[v]);
        s.eax = (s.eax & !0xFF) | 1;
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
        r.edi = s.edi;
    }
}
thunk!(t_b9fd80, e_b9fd80, "ret 8");

// --- stunt_peel_out_diversion_type::update -------------------------------------------------------
// if byte [ecx+0x18] != 0: push ecx; call 0x61DBC0 (stdcall, one argument); ret.
// else: push esi; esi = ecx; call 0x61DCD0; pop esi; ret.

extern "C" fn e_61dba0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(6);
        let this = r.ecx;
        let mut s = St::of(r);
        if rb(this as usize + 0x18) != 0 {
            s.call(0x0061_DBC0, &[this]);
            r.esi = s.esi;
        } else {
            s.esi = this;
            s.call(0x0061_DCD0, &[]);
        }
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
        r.edi = s.edi;
    }
}
thunk!(t_61dba0, e_61dba0, "ret");

// --- fuzz_crime_to_catch_a_predator::any_participant_near_player ---------------------------------
// x = arg (f32); push ecx; arg slot = (f32)(x * C) with C a double in the exe; ECX slot = arg;
// call 0x5C3A60 (stdcall, one argument = the product); ret 4.

extern "C" fn e_5d4350(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let x = rf(a as usize);
        let c = *(0x00E5_C358 as *const f64);
        let v = ((x as f64) * c) as f32;
        // the original stores the product into its argument slot and into the slot of the pushed ECX
        wd(a as usize, v.to_bits());
        wd(a as usize - 8, v.to_bits());
        let mut s = St::of(r);
        s.call(0x005C_3A60, &[v.to_bits()]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
        r.esi = s.esi;
        r.edi = s.edi;
    }
}
thunk!(t_5d4350, e_5d4350, "ret 4");

// --- table -----------------------------------------------------------------------------------------

const B_601E20: [u8; 20] = [
    0x56, 0x57, 0xBF, 0x18, 0x31, 0xDF, 0x00, 0xBE, 0x34, 0x31, 0xDF, 0x00, 0xE8, 0xAF, 0x2B, 0x1F,
    0x00, 0x5F, 0x5E, 0xC3,
];
const B_609940: [u8; 20] = [
    0x56, 0x57, 0xBF, 0x40, 0x41, 0xDF, 0x00, 0xBE, 0x60, 0x41, 0xDF, 0x00, 0xE8, 0x8F, 0xB0, 0x1E,
    0x00, 0x5F, 0x5E, 0xC3,
];
const B_5750D0: [u8; 20] = [
    0x56, 0x57, 0xBF, 0x84, 0x89, 0xDE, 0x00, 0xBE, 0xB4, 0x89, 0xDE, 0x00, 0xE8, 0xFF, 0xF8, 0x27,
    0x00, 0x5F, 0x5E, 0xC3,
];
const B_7F0A90: [u8; 29] = [
    0xA1, 0x20, 0xDC, 0x2F, 0x02, 0x56, 0x8B, 0x35, 0x1C, 0xDC, 0x2F, 0x02, 0x57, 0x50, 0x6A, 0x00,
    0x33, 0xFF, 0xE8, 0x79, 0x42, 0x52, 0x00, 0x83, 0xC4, 0x08, 0x5F, 0x5E, 0xC3,
];
const B_BA47D0: [u8; 21] = [
    0xE8, 0x8B, 0x12, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x01, 0xC3, 0xD9, 0xE8, 0xB0, 0x01, 0xD9, 0x51,
    0x70, 0xD9, 0x59, 0x74, 0xC3,
];
const B_B9FD80: [u8; 23] = [
    0x8B, 0x81, 0xDC, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0xFF, 0x37, 0xFF,
    0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,
];
const B_61DBA0: [u8; 23] = [
    0x80, 0x79, 0x18, 0x00, 0x74, 0x07, 0x51, 0xE8, 0x14, 0x00, 0x00, 0x00, 0xC3, 0x56, 0x8B, 0xF1,
    0xE8, 0x1B, 0x01, 0x00, 0x00, 0x5E, 0xC3,
];
const B_5D4350: [u8; 30] = [
    0xD9, 0x44, 0x24, 0x04, 0x51, 0xDC, 0x0D, 0x58, 0xC3, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x08, 0xD9,
    0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8, 0xF5, 0xF6, 0xFE, 0xFF, 0xC2, 0x04, 0x00,
];

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "combat_tricks_diversion_type::get_title", addr: 0x00601E20, body: &B_601E20, thunk: t_601e20 },
    Port { name: "driver_bailout_diversion_type_info::get_title", addr: 0x00609940, body: &B_609940, thunk: t_609940 },
    Port { name: "aircraft_kill_objective::get_start_message", addr: 0x005750D0, body: &B_5750D0, thunk: t_5750d0 },
    Port { name: "vint_draw_clip_end", addr: 0x007F0A90, body: &B_7F0A90, thunk: t_7f0a90 },
    Port { name: "vint_element_point::init", addr: 0x00BA47D0, body: &B_BA47D0, thunk: t_ba47d0 },
    Port { name: "vint_object_tween::get_start_value_type", addr: 0x00B9FD80, body: &B_B9FD80, thunk: t_b9fd80 },
    Port { name: "stunt_peel_out_diversion_type::update", addr: 0x0061DBA0, body: &B_61DBA0, thunk: t_61dba0 },
    Port { name: "fuzz_crime_to_catch_a_predator::any_participant_near_player", addr: 0x005D4350, body: &B_5D4350, thunk: t_5d4350 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h1: replaced {ok}/{COUNT} with Rust"));
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
        return String::new();
    }
    format!("port_h1: {}", parts.join(", "))
}
