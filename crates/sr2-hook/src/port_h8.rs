//! Ported functions, round 9 list h8: small class methods and the scalar deleting destructor
//! shape (names D-F, plus a few get_start_message / pre_render / can_alloc bodies).
//!
//! Every port runs behind a `pushad` thunk (XMM0-7 saved, see `thunk!`). Callees stay original
//! and are called by address through `St::call` / `St::calli`, which run them with the registers
//! the original sets up and hand back what the callee left. ESP is restored by the call helper.
//!
//! Checks: per-function call counters only (every body calls out, except can_alloc, which is pure).

#![allow(unused_imports, dead_code, unused_macros)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 13;

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

    /// Call through a function pointer read from game memory (a vtable slot or an import).
    #[inline(always)]
    unsafe fn calli(&mut self, f: u32, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f as usize, self, args) };
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

/// Copy the callee-left registers the original leaves in place (EAX is set by the caller).
#[inline(always)]
fn out_regs(r: &mut Pushad, s: &St) {
    r.ecx = s.ecx;
    r.edx = s.edx;
    r.ebx = s.ebx;
    r.ebp = s.ebp;
}

// --- demo_derby_sp_level_instance::load_xtbl (15 bytes) ------------------------------------------
// push esi; esi = ecx; call 0x5F06C0; [esi + 0x1C] = eax; pop esi; ret 4.

extern "C" fn e_5f0f30(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(0);
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        s.call(0x005F_06C0, &[]);
        wd(this as usize + 0x1C, s.eax);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_5f0f30, e_5f0f30, "ret 4");

// --- script_mover::get_object (26 bytes) ---------------------------------------------------------
// esi = ecx; eax = [esi]; edx = [eax + 0x2C]; call edx (ECX = this); if al != 0: eax = [esi + 0x8C]
// else eax = 0.

extern "C" fn e_a5afa0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(1);
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        let vt = rd(this as usize);
        let f = rd(vt as usize + 0x2C);
        s.calli(f, &[]);
        if s.eax as u8 != 0 {
            r.eax = rd(this as usize + 0x8C);
        } else {
            r.eax = 0;
        }
        out_regs(r, &s);
    }
}
thunk!(t_a5afa0, e_a5afa0, "ret");

// --- vint_element_text::pre_render (21 bytes) ----------------------------------------------------
// esi = ecx; eax = [esi]; edx = [eax + 0x20]; call edx (ECX = this); cmp al, 1; jne out; call 0xBA2CC0.

extern "C" fn e_ba2c40(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(2);
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        let vt = rd(this as usize);
        let f = rd(vt as usize + 0x20);
        s.calli(f, &[]);
        if s.eax as u8 == 1 {
            s.call(0x00BA_2CC0, &[]);
        }
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_ba2c40, e_ba2c40, "ret");

// --- r_distant_ped_mesh_inst::pre_render (28 bytes) ----------------------------------------------
// eax = [esp + 4]; push esi; push eax; esi = ecx; call 0xCF8980 (stdcall, one argument);
// esi = [esi + 0x350]; call 0xCF7CD0; pop esi; ret 4.

extern "C" fn e_b7b4e0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(3);
        let arg1 = rd(a as usize);
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x00CF_8980, &[arg1]);
        let e = s.esi as usize;
        s.esi = rd(e + 0x350);
        s.call(0x00CF_7CD0, &[]);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_b7b4e0, e_b7b4e0, "ret 4");

// --- vint_element_sr2_map::init (30 bytes) -------------------------------------------------------
// call 0xB95A60 (ECX = this); if al != 0: fld1; eax = 1; fstp [ecx + 0x98]; [ecx + 0x9C] = eax.

extern "C" fn e_ba62e0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(4);
        let mut s = St::of(r);
        s.call(0x00B9_5A60, &[]);
        if s.eax as u8 != 0 {
            let c = s.ecx as usize;
            wf(c + 0x98, 1.0);
            s.eax = 1;
            wd(c + 0x9C, 1);
        }
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_ba62e0, e_ba62e0, "ret");

// --- split_page_mempool::can_alloc (70 bytes, pure) ----------------------------------------------
// edx = [ecx + 0x40]; eax = arg2; if eax < edx: eax = edx; edx = arg1; eax = (eax - 1) & [ecx + 0x38];
// esi = [ecx + 0x4C]; ecx = [ecx + 0x58]; eax = eax + esi + edx - 1; edx = 0; eax /= esi (edx = rem);
// walk the list at ecx: if [ecx + 4] >= eax (unsigned): al = 1; else next = [ecx + 0xC] until 0.

extern "C" fn e_c16bd0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(5);
        let this = r.ecx as usize;
        let arg1 = rd(a as usize);
        let arg2 = rd(a as usize + 4);
        let mut eax = arg2;
        let edx0 = rd(this + 0x40);
        if eax < edx0 {
            eax = edx0;
        }
        eax = (eax.wrapping_sub(1)) & rd(this + 0x38);
        let esi = rd(this + 0x4C);
        let mut ecx = rd(this + 0x58);
        eax = eax.wrapping_add(esi).wrapping_add(arg1).wrapping_sub(1);
        // unsigned 32-bit divide of EDX:EAX (EDX = 0) by ESI
        let mut rem: u32 = 0;
        std::arch::asm!("div {d}", d = in(reg) esi, inout("eax") eax, inout("edx") rem, options(nostack));
        let q = eax;
        let mut found = false;
        if ecx != 0 {
            loop {
                if rd(ecx as usize + 4) >= q {
                    found = true;
                    break;
                }
                ecx = rd(ecx as usize + 0xC);
                if ecx == 0 {
                    break;
                }
            }
        }
        r.eax = (q & !0xFF) | found as u32;
        r.ecx = ecx;
        r.edx = rem;
    }
}
thunk!(t_c16bd0, e_c16bd0, "ret 8");

// --- get_start_message shapes (taxi_diversion_type_info, vehicle_surfing_style_two_..._info) -----
// push esi; push edi; edi = C1; esi = C2; call 0x7F49E0; esi = arg2; edi = arg1; push eax;
// call 0xC14C40 (cdecl, one argument); add esp, 4; pop edi; pop esi; ret 8.

extern "C" fn e_628fb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(6);
        let arg1 = rd(a as usize);
        let arg2 = rd(a as usize + 4);
        let mut s = St::of(r);
        s.edi = 0x00DF_8020;
        s.esi = 0x00DF_8098;
        let e = s.call(0x007F_49E0, &[]);
        s.esi = arg2;
        s.edi = arg1;
        s.call(0x00C1_4C40, &[e]);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_628fb0, e_628fb0, "ret 8");

extern "C" fn e_633220(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let arg1 = rd(a as usize);
        let arg2 = rd(a as usize + 4);
        let mut s = St::of(r);
        s.edi = 0;
        s.esi = 0x00DF_9344;
        let e = s.call(0x007F_49E0, &[]);
        s.esi = arg2;
        s.edi = arg1;
        s.call(0x00C1_4C40, &[e]);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_633220, e_633220, "ret 8");

// --- fuzz_crime_noise_violation::enter_state3 (45 bytes) -----------------------------------------
// push esi; esi = ecx; call 0x5C3020; ecx = [esi + 0xFC]; eax = [ecx]; edx = [eax + 0x34];
// call edx; test eax, eax; pop esi; je out; eax += 0x8554; push 0; push eax; ecx = 0;
// call 0xB5FB30 (cdecl, two arguments); add esp, 8; out: ret.

extern "C" fn e_5c66e0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(8);
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x005C_3020, &[]);
        let this = s.esi;
        s.ecx = rd(this as usize + 0xFC);
        let vt = rd(s.ecx as usize);
        let f = rd(vt as usize + 0x34);
        s.calli(f, &[]);
        if s.eax != 0 {
            let v = s.eax.wrapping_add(0x8554);
            s.ecx = 0;
            s.call(0x00B5_FB30, &[v, 0]);
        }
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_5c66e0, e_5c66e0, "ret");

// --- event_trigger::end (56 bytes) ---------------------------------------------------------------
// push esi; push edi; esi = ecx; edi = esi + 0xC; push edi; call [0xDB80CC] (stdcall);
// eax = [esi + 4] - 1; [esi + 4] = eax; if eax != 0 goto unlock; if byte [esi + 8] != 0:
// push [esi + 0x2C]; byte [esi + 0x38] = 1; call [0xDB81DC] (stdcall); unlock: push edi;
// call [0xDB80D0] (stdcall); pop edi; pop esi; ret.

extern "C" fn e_bf3d30(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(9);
        let this = r.ecx;
        let edi = this + 0x0C;
        let mut s = St::of(r);
        s.esi = this;
        s.edi = edi;
        s.calli(rd(0x00DB_80CC), &[edi]);
        let v = rd(this as usize + 4).wrapping_sub(1);
        wd(this as usize + 4, v);
        if v == 0 && rb(this as usize + 8) != 0 {
            let ecx = rd(this as usize + 0x2C);
            wb(this as usize + 0x38, 1);
            s.ecx = ecx;
            s.calli(rd(0x00DB_81DC), &[ecx]);
        }
        s.calli(rd(0x00DB_80D0), &[edi]);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_bf3d30, e_bf3d30, "ret");

// --- fober_lock_free_queue::scalar_deleting_dtor (60 bytes) --------------------------------------
// [ecx] = 0xE44180; if bit 0 of the flag argument: if byte [0x2527BF7] != 1: lock (import 0xDB80CC),
// free (0xCDE870, EAX = this), unlock (import 0xDB80D0). eax = this; ret 4.

extern "C" fn e_a74cc0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(10);
        let this = r.ecx;
        wd(this as usize, 0x00E4_4180);
        let mut s = St::of(r);
        if rb(a as usize) & 1 != 0 && rb(0x0252_7BF7) != 1 {
            s.calli(rd(0x00DB_80CC), &[0x0347_ADE4]);
            s.eax = this;
            s.call(0x00CD_E870, &[]);
            s.calli(rd(0x00DB_80D0), &[0x0347_ADE4]);
        }
        r.eax = this;
        out_regs(r, &s);
    }
}
thunk!(t_a74cc0, e_a74cc0, "ret 4");

// --- skirmish_spawn_group::reset (95 bytes) ------------------------------------------------------
// push esi; push edi; esi = ecx; call 0x956080; [esi+0x1FC] = [esi+0x200] = [esi+0x1F8] = -1;
// push 0x36EE80; edi = esi + 0x1E4; eax = 0x5265C0; [esi+0x1F0] = 0; [esi+0x1F5] = 0; [esi+0x1F4] = 1;
// call 0xBDD1B0 (stdcall, one argument); [esi+0x1F5] = 0; [esi+0x1F0] = 1; pop edi; pop esi; ret.

extern "C" fn e_567270(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(11);
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x0095_6080, &[]);
        let t = s.esi as usize;
        wd(t + 0x1FC, 0xFFFF_FFFF);
        wd(t + 0x200, 0xFFFF_FFFF);
        wd(t + 0x1F8, 0xFFFF_FFFF);
        s.edi = (t + 0x1E4) as u32;
        s.eax = 0x0052_65C0;
        wd(t + 0x1F0, 0);
        wb(t + 0x1F5, 0);
        wb(t + 0x1F4, 1);
        s.call(0x00BD_D1B0, &[0x0036_EE80]);
        let t = s.esi as usize;
        wb(t + 0x1F5, 0);
        wd(t + 0x1F0, 1);
        r.eax = s.eax;
        out_regs(r, &s);
    }
}
thunk!(t_567270, e_567270, "ret");

// --- find_vehicle_respawn_functor::ctor (83 bytes, this in ESI, ret 0x10) -------------------------
// [esi] = 0xE2C898; eax = [0x2770FE0]; edx = [eax + 0x1C]; push 0; push 0x180; ecx = 0x2770FE0;
// call edx; fldz; [esi + 0x10] = 0.0; [esi + 4] = eax; [esi + 0x14] = arg1 (f32); dl = arg4 (byte);
// eax = arg2; [esi + 0x18] = eax; [esi + 8] = 0; [esi + 0xC] = 0; [esi + 0x1C] = arg3 (ecx);
// [esi + 0x20] = dl; eax = esi; ret 0x10.

extern "C" fn e_884bc0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(12);
        let arg1 = rf(a as usize);
        let arg2 = rd(a as usize + 4);
        let arg3 = rd(a as usize + 8);
        let arg4 = rb(a as usize + 12);
        let mut s = St::of(r);
        wd(s.esi as usize, 0x00E2_C898);
        let obj = rd(0x0277_0FE0);
        s.eax = obj;
        s.edx = rd(obj as usize + 0x1C);
        s.ecx = 0x0277_0FE0;
        let f = s.edx;
        let ret = s.calli(f, &[0x0180, 0]);
        let t = s.esi as usize;
        wf(t + 0x10, 0.0);
        wd(t + 4, ret);
        wf(t + 0x14, arg1);
        s.edx = (s.edx & !0xFF) | arg4 as u32;
        wd(t + 0x18, arg2);
        wd(t + 8, 0);
        wd(t + 0xC, 0);
        s.ecx = arg3;
        wd(t + 0x1C, arg3);
        wb(t + 0x20, arg4);
        r.eax = t as u32;
        out_regs(r, &s);
        r.edx = s.edx;
        r.esi = t as u32;
        r.edi = s.edi;
    }
}
thunk!(t_884bc0, e_884bc0, "ret 16");

// --- table -----------------------------------------------------------------------------------------

const B_5F0F30: [u8; 15] = [0x56, 0x8B, 0xF1, 0xE8, 0x88, 0xF7, 0xFF, 0xFF, 0x89, 0x46, 0x1C, 0x5E, 0xC2, 0x04, 0x00];
const B_A5AFA0: [u8; 26] = [
    0x56, 0x8B, 0xF1, 0x8B, 0x06, 0x8B, 0x50, 0x2C, 0xFF, 0xD2, 0x84, 0xC0, 0x74, 0x08, 0x8B, 0x86, 0x8C, 0x00,
    0x00, 0x00, 0x5E, 0xC3, 0x33, 0xC0, 0x5E, 0xC3,
];
const B_BA2C40: [u8; 21] = [
    0x56, 0x8B, 0xF1, 0x8B, 0x06, 0x8B, 0x50, 0x20, 0xFF, 0xD2, 0x3C, 0x01, 0x75, 0x05, 0xE8, 0x6D, 0x00, 0x00,
    0x00, 0x5E, 0xC3,
];
const B_B7B4E0: [u8; 28] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x50, 0x8B, 0xF1, 0xE8, 0x93, 0xD4, 0x17, 0x00, 0x8B, 0xB6, 0x50, 0x03, 0x00,
    0x00, 0xE8, 0xD8, 0xC7, 0x17, 0x00, 0x5E, 0xC2, 0x04, 0x00,
];
const B_BA62E0: [u8; 30] = [
    0xE8, 0x7B, 0xF7, 0xFE, 0xFF, 0x84, 0xC0, 0x75, 0x01, 0xC3, 0xD9, 0xE8, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xD9,
    0x99, 0x98, 0x00, 0x00, 0x00, 0x89, 0x81, 0x9C, 0x00, 0x00, 0x00, 0xC3,
];
const B_C16BD0: [u8; 70] = [
    0x8B, 0x51, 0x40, 0x8B, 0x44, 0x24, 0x08, 0x3B, 0xC2, 0x0F, 0x42, 0xC2, 0x8B, 0x54, 0x24, 0x04, 0x83, 0xC0,
    0xFF, 0x23, 0x41, 0x38, 0x56, 0x8B, 0x71, 0x4C, 0x8B, 0x49, 0x58, 0x03, 0xC6, 0x8D, 0x44, 0x10, 0xFF, 0x33,
    0xD2, 0xF7, 0xF6, 0x85, 0xC9, 0x5E, 0x74, 0x10, 0x8D, 0x64, 0x24, 0x00, 0x39, 0x41, 0x04, 0x73, 0x0C, 0x8B,
    0x49, 0x0C, 0x85, 0xC9, 0x75, 0xF4, 0x32, 0xC0, 0xC2, 0x08, 0x00, 0xB0, 0x01, 0xC2, 0x08, 0x00,
];
const B_628FB0: [u8; 39] = [
    0x56, 0x57, 0xBF, 0x20, 0x80, 0xDF, 0x00, 0xBE, 0x98, 0x80, 0xDF, 0x00, 0xE8, 0x1F, 0xBA, 0x1C, 0x00, 0x8B,
    0x74, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x0C, 0x50, 0xE8, 0x71, 0xBC, 0x5E, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E,
    0xC2, 0x08, 0x00,
];
const B_633220: [u8; 36] = [
    0x56, 0x57, 0x33, 0xFF, 0xBE, 0x44, 0x93, 0xDF, 0x00, 0xE8, 0xB2, 0x17, 0x1C, 0x00, 0x8B, 0x74, 0x24, 0x10,
    0x8B, 0x7C, 0x24, 0x0C, 0x50, 0xE8, 0x04, 0x1A, 0x5E, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00,
];
const B_5C66E0: [u8; 45] = [
    0x56, 0x8B, 0xF1, 0xE8, 0x38, 0xC9, 0xFF, 0xFF, 0x8B, 0x8E, 0xFC, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50,
    0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x5E, 0x74, 0x12, 0x05, 0x54, 0x85, 0x00, 0x00, 0x6A, 0x00, 0x50, 0x33, 0xC9,
    0xE8, 0x27, 0x94, 0x59, 0x00, 0x83, 0xC4, 0x08, 0xC3,
];
const B_BF3D30: [u8; 56] = [
    0x56, 0x57, 0x8B, 0xF1, 0x8D, 0x7E, 0x0C, 0x57, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0x46, 0x04, 0x83,
    0xC0, 0xFF, 0x89, 0x46, 0x04, 0x75, 0x15, 0x8A, 0x46, 0x08, 0x84, 0xC0, 0x74, 0x0E, 0x8B, 0x4E, 0x2C, 0x51,
    0xC6, 0x46, 0x38, 0x01, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x57, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5F,
    0x5E, 0xC3,
];
const B_A74CC0: [u8; 60] = [
    0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x80, 0x41, 0xE4, 0x00, 0x74, 0x26, 0xA0, 0xF7,
    0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00,
    0x8B, 0xC6, 0xE8, 0x85, 0x9B, 0x26, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00,
    0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00,
];

const B_567270: [u8; 95] = [
    0x56, 0x57, 0x8B, 0xF1, 0xE8, 0x07, 0xEE, 0x3E, 0x00, 0x83, 0xC8, 0xFF, 0x89, 0x86, 0xFC, 0x01,
    0x00, 0x00, 0x89, 0x86, 0x00, 0x02, 0x00, 0x00, 0x89, 0x86, 0xF8, 0x01, 0x00, 0x00, 0x68, 0x80,
    0xEE, 0x36, 0x00, 0x8D, 0xBE, 0xE4, 0x01, 0x00, 0x00, 0xB8, 0xC0, 0x65, 0x52, 0x00, 0xC7, 0x86,
    0xF0, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x86, 0xF5, 0x01, 0x00, 0x00, 0x00, 0xC6,
    0x86, 0xF4, 0x01, 0x00, 0x00, 0x01, 0xE8, 0xF5, 0x5E, 0x67, 0x00, 0x5F, 0xC6, 0x86, 0xF5, 0x01,
    0x00, 0x00, 0x00, 0xC7, 0x86, 0xF0, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const B_884BC0: [u8; 83] = [
    0xC7, 0x06, 0x98, 0xC8, 0xE2, 0x00, 0xA1, 0xE0, 0x0F, 0x77, 0x02, 0x8B, 0x50, 0x1C, 0x6A, 0x00,
    0x68, 0x80, 0x01, 0x00, 0x00, 0xB9, 0xE0, 0x0F, 0x77, 0x02, 0xFF, 0xD2, 0xD9, 0xEE, 0x8B, 0x4C,
    0x24, 0x0C, 0xD9, 0x5E, 0x10, 0xD9, 0x44, 0x24, 0x04, 0x8A, 0x54, 0x24, 0x10, 0x89, 0x46, 0x04,
    0xD9, 0x5E, 0x14, 0x8B, 0x44, 0x24, 0x08, 0x89, 0x46, 0x18, 0xC7, 0x46, 0x08, 0x00, 0x00, 0x00,
    0x00, 0xC7, 0x46, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x89, 0x4E, 0x1C, 0x88, 0x56, 0x20, 0x8B, 0xC6,
    0xC2, 0x10, 0x00,
];
struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "demo_derby_sp_level_instance::load_xtbl", addr: 0x005F_0F30, body: &B_5F0F30, thunk: t_5f0f30 },
    Port { name: "script_mover::get_object", addr: 0x00A5_AFA0, body: &B_A5AFA0, thunk: t_a5afa0 },
    Port { name: "vint_element_text::pre_render", addr: 0x00BA_2C40, body: &B_BA2C40, thunk: t_ba2c40 },
    Port { name: "r_distant_ped_mesh_inst::pre_render", addr: 0x00B7_B4E0, body: &B_B7B4E0, thunk: t_b7b4e0 },
    Port { name: "vint_element_sr2_map::init", addr: 0x00BA_62E0, body: &B_BA62E0, thunk: t_ba62e0 },
    Port { name: "split_page_mempool::can_alloc", addr: 0x00C1_6BD0, body: &B_C16BD0, thunk: t_c16bd0 },
    Port { name: "taxi_diversion_type_info::get_start_message", addr: 0x0062_8FB0, body: &B_628FB0, thunk: t_628fb0 },
    Port { name: "vehicle_surfing_style_two_diversion_type_info::get_start_message", addr: 0x0063_3220, body: &B_633220, thunk: t_633220 },
    Port { name: "fuzz_crime_noise_violation::enter_state3", addr: 0x005C_66E0, body: &B_5C66E0, thunk: t_5c66e0 },
    Port { name: "event_trigger::end", addr: 0x00BF_3D30, body: &B_BF3D30, thunk: t_bf3d30 },
    Port { name: "fober_lock_free_queue::scalar_deleting_dtor", addr: 0x00A7_4CC0, body: &B_A74CC0, thunk: t_a74cc0 },
    Port { name: "skirmish_spawn_group::reset", addr: 0x0056_7270, body: &B_567270, thunk: t_567270 },
    Port { name: "find_vehicle_respawn_functor::ctor", addr: 0x0088_4BC0, body: &B_884BC0, thunk: t_884bc0 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h8: replaced {ok}/{COUNT} with Rust"));
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
    format!("port_h8: {}", parts.join(", "))
}
