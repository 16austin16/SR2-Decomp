//! Port module h4: small functions from scratchpad/r8/list_h4.txt, each pinned to its whole original body.
//!
//! Every port runs behind one `pushad` thunk that also saves XMM0-7. Callees stay original and are called
//! by address through `St::call`, which hands them the registers and stack words the original sets up and
//! returns the registers they leave (ECX / EDX included). Tail jumps pass the caller's own stack words.

#![allow(unused_imports, dead_code, unused_variables, unused_macros)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::log;
use crate::portlib::*;

const COUNT: usize = 12;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < (1 << 24) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// What a thunk hands to its handler: XMM0-7, then the `pushad` block (lowest address first).
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Pushad {
    xmm: [[u32; 4]; 8],
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// `pushad`, XMM0-7 saved, `f(&mut block, &stack args)`, XMM0-7 restored, `popad`.
macro_rules! hot_thunk {
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
                "mov eax, esp",
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

    /// Call through a function pointer read from game memory.
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

/// The stack words a callee sees when the original calls it with `own` pushed above its own return
/// address: `own` (top first), the return address, then the caller's own stack arguments.
unsafe fn frame(a: *const u32, own: &[u32]) -> [u32; 8] {
    let mut w = [0u32; 8];
    let mut k = 0;
    for &v in own {
        w[k] = v;
        k += 1;
    }
    w[k] = unsafe { *a.sub(1) };
    k += 1;
    let mut i = 0;
    while k < 8 {
        w[k] = unsafe { *a.add(i) };
        k += 1;
        i += 1;
    }
    w
}

/// The stack words a callee sees after a tail `jmp` from the port: the caller's own stack arguments.
unsafe fn tail(a: *const u32) -> [u32; 8] {
    let mut w = [0u32; 8];
    for (i, v) in w.iter_mut().enumerate() {
        *v = unsafe { *a.add(i) };
    }
    w
}

/// Copy the callee's leftover registers back into the pushad block (ESI / EDI are restored by the original's pops).
#[inline(always)]
fn back(r: &mut Pushad, s: &St) {
    r.eax = s.eax;
    r.ecx = s.ecx;
    r.edx = s.edx;
    r.ebx = s.ebx;
    r.ebp = s.ebp;
}

// --- original bodies (pinned; the DLL refuses a port whose bytes differ from the exe) -----------

// fuzz_crime_car_bomb::get_bar_title @ 0x005D03B0
const BODY_CAR_BOMB_TITLE: [u8; 20] = [
    0x56, 0x57, 0xBF, 0x8C, 0xFB, 0xDE, 0x00, 0xBE, 0x9C, 0xFB, 0xDE, 0x00, 0xE8, 0x1F, 0x46, 0x22,
    0x00, 0x5F, 0x5E, 0xC3,
];
// barnstorming_diversion_type::get_title @ 0x005F7F90
const BODY_BARN_TITLE: [u8; 20] = [
    0x56, 0x57, 0xBF, 0x50, 0x1E, 0xDF, 0x00, 0xBE, 0x6C, 0x1E, 0xDF, 0x00, 0xE8, 0x3F, 0xCA, 0x1F,
    0x00, 0x5F, 0x5E, 0xC3,
];
// vint_draw_set_blend @ 0x007F0A50
const BODY_VINT_SET_BLEND: [u8; 23] = [
    0x8B, 0x44, 0x24, 0x04, 0x3B, 0x05, 0x48, 0xC5, 0xE8, 0x00, 0x74, 0x0A, 0xA3, 0x48, 0xC5, 0xE8,
    0x00, 0xE9, 0xEA, 0xFE, 0xFF, 0xFF, 0xC3,
];
// net_recv_migrated @ 0x00810A00
const BODY_NET_RECV_MIGRATED: [u8; 25] = [
    0x8B, 0x44, 0x24, 0x08, 0x83, 0x78, 0x04, 0x00, 0x74, 0x0E, 0x8B, 0x48, 0x04, 0x8B, 0x81, 0xAC,
    0x01, 0x00, 0x00, 0xE9, 0xD8, 0xFA, 0xFF, 0xFF, 0xC3,
];
// effect_object::activate @ 0x0050B3A0
const BODY_EFFECT_ACTIVATE: [u8; 31] = [
    0x85, 0xC9, 0x74, 0x18, 0x8B, 0x81, 0x80, 0x00, 0x00, 0x00, 0xA8, 0x0B, 0x75, 0x0E, 0x83, 0xC8,
    0x02, 0x89, 0x81, 0x80, 0x00, 0x00, 0x00, 0xE9, 0xC4, 0xA3, 0x41, 0x00, 0xC2, 0x04, 0x00,
];
// detour_hull_object::activate @ 0x00919620
const BODY_DETOUR_ACTIVATE: [u8; 40] = [
    0x56, 0x8B, 0xF1, 0x8B, 0x8E, 0xD4, 0x00, 0x00, 0x00, 0xE8, 0x92, 0x4A, 0x0F, 0x00, 0x8B, 0x44,
    0x24, 0x08, 0x50, 0x8B, 0xCE, 0xC7, 0x86, 0xD4, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xE8,
    0x3C, 0xC1, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00,
];
// fuzz_crime_street_racing::any_participant_near_player @ 0x005D2D20
const BODY_ANY_PARTICIPANT: [u8; 26] = [
    0xD9, 0x44, 0x24, 0x04, 0x51, 0xDC, 0xC0, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9,
    0x1C, 0x24, 0xE8, 0x29, 0x0D, 0xFF, 0xFF, 0xC2, 0x04, 0x00,
];
// chop_shop_diversion_type::init @ 0x005FFEE0
const BODY_CHOP_SHOP_INIT: [u8; 42] = [
    0x56, 0x8B, 0xF1, 0x56, 0xE8, 0xC7, 0xFE, 0xFF, 0xFF, 0x56, 0xE8, 0x01, 0xF5, 0xFF, 0xFF, 0x6A,
    0x01, 0x56, 0xC6, 0x86, 0x84, 0x13, 0x00, 0x00, 0x00, 0xE8, 0x62, 0x0B, 0x00, 0x00, 0xC7, 0x86,
    0x44, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
// act_packet_sub_4e @ 0x006494D0
const BODY_ACT_PACKET_4E: [u8; 47] = [
    0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50,
    0x08, 0xFF, 0xD2, 0x83, 0xF8, 0x18, 0x75, 0x16, 0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74,
    0x0D, 0x83, 0x78, 0x14, 0x07, 0x74, 0x07, 0xC6, 0x80, 0xA6, 0x00, 0x00, 0x00, 0x01, 0xC3,
];
// act_packet_sub_56 @ 0x00645620
const BODY_ACT_PACKET_56: [u8; 47] = [
    0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50,
    0x08, 0xFF, 0xD2, 0x83, 0xF8, 0x19, 0x75, 0x16, 0x8B, 0x0D, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC9,
    0x74, 0x0C, 0xC6, 0x81, 0xE0, 0x00, 0x00, 0x00, 0x01, 0xE9, 0xC2, 0xDF, 0xFF, 0xFF, 0xC3,
];
// stunt_oncoming_traffic_diversion_type::on_game_event_15 @ 0x0061D420
const BODY_STUNT_ONCOMING_EVENT15: [u8; 42] = [
    0x80, 0x79, 0x1C, 0x00, 0x74, 0x21, 0x83, 0x7C, 0x24, 0x0C, 0x64, 0x7C, 0x1A, 0x8B, 0x44, 0x24,
    0x04, 0x8B, 0x50, 0x44, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x3B, 0x90, 0x74, 0x0D, 0x00, 0x00, 0x75,
    0x06, 0x51, 0xE8, 0x29, 0x03, 0x00, 0x00, 0xC2, 0x14, 0x00,
];
// stunt_back_seat_driver_diversion_type::update @ 0x00617060
const BODY_STUNT_BACK_SEAT_UPDATE: [u8; 43] = [
    0x56, 0x8B, 0xF1, 0x56, 0xE8, 0xD7, 0x00, 0x00, 0x00, 0x80, 0x7E, 0x39, 0x00, 0x74, 0x08, 0x56,
    0xE8, 0x9B, 0x02, 0x00, 0x00, 0x5E, 0xC3, 0x80, 0x7E, 0x38, 0x00, 0x74, 0x0C, 0xD9, 0xEE, 0xC6,
    0x46, 0x39, 0x01, 0xD9, 0x56, 0x48, 0xD9, 0x5E, 0x3C, 0x5E, 0xC3,
];
// --- helpers for the globals and vtable shapes ------------------------------------------------------

const G_ACT_OBJ: usize = 0x0252_7888; // the act-packet object pointer read by act_packet_sub_4e / _56

// --- ports ----------------------------------------------------------------------------------------

/// The `get_title` shape: push EDI, push ESI, load EDI and ESI with constants, call 0x7F49E0, pop, pop, ret.
unsafe fn title_shape(r: &mut Pushad, a: *const u32, k_edi: u32, k_esi: u32) {
    unsafe {
        let w = frame(a, &[r.edi, r.esi]);
        let mut s = St::of(r);
        s.esi = k_esi;
        s.edi = k_edi;
        s.call(0x007F_49E0, &w);
        back(r, &s);
    }
}

hot_thunk!(thunk_car_bomb_title, port_car_bomb_title, "ret");
unsafe extern "C" fn port_car_bomb_title(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(0);
        title_shape(r, a, 0x00DE_FB8C, 0x00DE_FB9C);
    }
}

hot_thunk!(thunk_barn_title, port_barn_title, "ret");
unsafe extern "C" fn port_barn_title(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(1);
        title_shape(r, a, 0x00DF_1E50, 0x00DF_1E6C);
    }
}

// vint_draw_set_blend: EAX = arg; if EAX differs from the global at 0xE8C548, store it and tail-jump to 0x7F0950.
hot_thunk!(thunk_vint_set_blend, port_vint_set_blend, "ret");
unsafe extern "C" fn port_vint_set_blend(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(2);
        let v = rd(a as usize);
        r.eax = v;
        if v == rd(0x00E8_C548) {
            return;
        }
        wd(0x00E8_C548, v);
        let w = tail(a);
        let mut s = St::of(r);
        s.eax = v;
        s.call(0x007F_0950, &w);
        back(r, &s);
    }
}

// net_recv_migrated: obj = arg2; if obj.+4 is null return; else ECX = obj.+4, EAX = ECX.+0x1AC, tail-jump 0x8104F0.
hot_thunk!(thunk_net_recv_migrated, port_net_recv_migrated, "ret");
unsafe extern "C" fn port_net_recv_migrated(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(3);
        let obj = rd(a as usize + 4);
        r.eax = obj;
        if rd(obj as usize + 4) == 0 {
            return;
        }
        let ecx = rd(obj as usize + 4);
        r.ecx = ecx;
        r.eax = rd(ecx as usize + 0x1AC);
        let w = tail(a);
        let mut s = St::of(r);
        s.call(0x0081_04F0, &w);
        back(r, &s);
    }
}

// effect_object::activate: this = ECX; null -> ret 4; if (flags & 0xB) == 0, flags |= 2 and tail-jump 0x925780.
hot_thunk!(thunk_effect_activate, port_effect_activate, "ret 4");
unsafe extern "C" fn port_effect_activate(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(4);
        let this = r.ecx;
        if this == 0 {
            return;
        }
        let v = rd(this as usize + 0x80);
        r.eax = v;
        if v & 0x0B != 0 {
            return;
        }
        let nv = v | 2;
        wd(this as usize + 0x80, nv);
        r.eax = nv;
        let w = tail(a);
        let mut s = St::of(r);
        s.call(0x0092_5780, &w);
        back(r, &s);
    }
}

// detour_hull_object::activate: call 0xA0E0C0 on [this+0xD4], set [this+0xD4] = -1, then call 0x925780(arg1).
hot_thunk!(thunk_detour_activate, port_detour_activate, "ret 4");
unsafe extern "C" fn port_detour_activate(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(5);
        let this = r.ecx;
        let saved_esi = r.esi;
        let mut s = St::of(r);
        s.esi = this;
        s.ecx = rd(this as usize + 0xD4);
        let w1 = frame(a, &[saved_esi]);
        s.call(0x00A0_E0C0, &w1);
        let arg = rd(a as usize);
        wd(this as usize + 0xD4, 0xFFFF_FFFF);
        s.eax = arg;
        s.ecx = this;
        let w2 = frame(a, &[arg, saved_esi]);
        s.call(0x0092_5780, &w2);
        back(r, &s);
    }
}

// any_participant_near_player: x = arg1 + arg1 (written back to the arg slot), then call 0x5C3A60(x), ret 4.
hot_thunk!(thunk_any_participant, port_any_participant, "ret 4");
unsafe extern "C" fn port_any_participant(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(6);
        let x = rf(a as usize);
        let d = x + x;
        wf(a as usize, d);
        let arg = d.to_bits();
        let w = frame(a, &[arg]);
        let mut s = St::of(r);
        s.call(0x005C_3A60, &w);
        back(r, &s);
    }
}

// chop_shop_diversion_type::init: push esi; esi = this; three stdcall callees, then [esi+0x1344] = 0.
hot_thunk!(thunk_chop_shop_init, port_chop_shop_init, "ret");
unsafe extern "C" fn port_chop_shop_init(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let saved = r.esi;
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x005F_FDB0, &frame(a, &[s.esi, saved]));
        let t = s.esi;
        s.call(0x005F_F3F0, &frame(a, &[t, saved]));
        let t = s.esi;
        wb(t as usize + 0x1384, 0);
        s.call(0x0060_0A60, &frame(a, &[t, 1, saved]));
        wd(s.esi as usize + 0x1344, 0);
        back(r, &s);
    }
}

// act_packet_sub_4e: g = [G]; if g: vt-slot-2 call on g.+8; if the result is 0x18 and [G] is set and its
// +0x14 is not 7, set byte +0xA6 = 1.
hot_thunk!(thunk_act_packet_4e, port_act_packet_4e, "ret");
unsafe extern "C" fn port_act_packet_4e(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(8);
        let g = rd(G_ACT_OBJ);
        r.eax = g;
        if g == 0 {
            return;
        }
        let ecx = rd(g as usize + 8);
        let vt = rd(ecx as usize);
        let fnp = rd(vt as usize + 8);
        let mut s = St::of(r);
        s.ecx = ecx;
        s.eax = vt;
        s.edx = fnp;
        s.calli(fnp, &frame(a, &[]));
        if s.eax != 0x18 {
            back(r, &s);
            return;
        }
        let g2 = rd(G_ACT_OBJ);
        s.eax = g2;
        if g2 != 0 && rd(g2 as usize + 0x14) != 7 {
            wb(g2 as usize + 0xA6, 1);
        }
        back(r, &s);
    }
}

// act_packet_sub_56: as _4e, but result 0x19 tail-jumps to 0x643610 after setting byte +0xE0 = 1.
hot_thunk!(thunk_act_packet_56, port_act_packet_56, "ret");
unsafe extern "C" fn port_act_packet_56(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(9);
        let g = rd(G_ACT_OBJ);
        r.eax = g;
        if g == 0 {
            return;
        }
        let ecx = rd(g as usize + 8);
        let vt = rd(ecx as usize);
        let fnp = rd(vt as usize + 8);
        let mut s = St::of(r);
        s.ecx = ecx;
        s.eax = vt;
        s.edx = fnp;
        s.calli(fnp, &frame(a, &[]));
        if s.eax != 0x19 {
            back(r, &s);
            return;
        }
        let g2 = rd(G_ACT_OBJ);
        s.ecx = g2;
        if g2 == 0 {
            back(r, &s);
            return;
        }
        wb(g2 as usize + 0xE0, 1);
        s.call(0x0064_3610, &tail(a));
        back(r, &s);
    }
}

// stunt_oncoming_traffic_diversion_type::on_game_event_15 (ret 0x14): when this+0x1C is set and arg3 >= 100
// and arg1.+0x44 equals the global object's +0xD74, call 0x61D770(this).
hot_thunk!(thunk_stunt_oncoming_event15, port_stunt_oncoming_event15, "ret 20");
unsafe extern "C" fn port_stunt_oncoming_event15(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(10);
        let this = r.ecx;
        if rb(this as usize + 0x1C) == 0 {
            return;
        }
        if (rd(a as usize + 8) as i32) < 0x64 {
            return;
        }
        let a0 = rd(a as usize);
        r.eax = a0;
        let edx = rd(a0 as usize + 0x44);
        r.edx = edx;
        let g1 = rd(0x0217_03D4);
        r.eax = g1;
        if edx != rd(g1 as usize + 0xD74) {
            return;
        }
        let mut s = St::of(r);
        s.call(0x0061_D770, &frame(a, &[this]));
        back(r, &s);
    }
}

// stunt_back_seat_driver_diversion_type::update: callee 0x617140(this); if byte +0x39 set, call 0x617310(this);
// otherwise if byte +0x38 set, set +0x39 = 1 and zero the two floats at +0x48 and +0x3C.
hot_thunk!(thunk_stunt_back_seat_update, port_stunt_back_seat_update, "ret");
unsafe extern "C" fn port_stunt_back_seat_update(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(11);
        let saved = r.esi;
        let mut s = St::of(r);
        s.esi = r.ecx;
        let t = s.esi;
        s.call(0x0061_7140, &frame(a, &[t, saved]));
        let this = s.esi;
        if rb(this as usize + 0x39) != 0 {
            s.call(0x0061_7310, &frame(a, &[this, saved]));
            back(r, &s);
            return;
        }
        if rb(this as usize + 0x38) != 0 {
            wb(this as usize + 0x39, 1);
            wf(this as usize + 0x48, 0.0);
            wf(this as usize + 0x3C, 0.0);
        }
        back(r, &s);
    }
}

// --- install / summary -----------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(&str, usize, &[u8], usize); COUNT] = [
        ("fuzz_crime_car_bomb::get_bar_title", 0x005D_03B0, &BODY_CAR_BOMB_TITLE[..], thunk_car_bomb_title as *const () as usize),
        ("barnstorming_diversion_type::get_title", 0x005F_7F90, &BODY_BARN_TITLE[..], thunk_barn_title as *const () as usize),
        ("vint_draw_set_blend", 0x007F_0A50, &BODY_VINT_SET_BLEND[..], thunk_vint_set_blend as *const () as usize),
        ("net_recv_migrated", 0x0081_0A00, &BODY_NET_RECV_MIGRATED[..], thunk_net_recv_migrated as *const () as usize),
        ("effect_object::activate", 0x0050_B3A0, &BODY_EFFECT_ACTIVATE[..], thunk_effect_activate as *const () as usize),
        ("detour_hull_object::activate", 0x0091_9620, &BODY_DETOUR_ACTIVATE[..], thunk_detour_activate as *const () as usize),
        ("fuzz_crime_street_racing::any_participant_near_player", 0x005D_2D20, &BODY_ANY_PARTICIPANT[..], thunk_any_participant as *const () as usize),
        ("chop_shop_diversion_type::init", 0x005F_FEE0, &BODY_CHOP_SHOP_INIT[..], thunk_chop_shop_init as *const () as usize),
        ("act_packet_sub_4e", 0x0064_94D0, &BODY_ACT_PACKET_4E[..], thunk_act_packet_4e as *const () as usize),
        ("act_packet_sub_56", 0x0064_5620, &BODY_ACT_PACKET_56[..], thunk_act_packet_56 as *const () as usize),
        ("stunt_oncoming_traffic_diversion_type::on_game_event_15", 0x0061_D420, &BODY_STUNT_ONCOMING_EVENT15[..], thunk_stunt_oncoming_event15 as *const () as usize),
        ("stunt_back_seat_driver_diversion_type::update", 0x0061_7060, &BODY_STUNT_BACK_SEAT_UPDATE[..], thunk_stunt_back_seat_update as *const () as usize),
    ];
    let mut ok = 0;
    for (name, addr, body, thunk) in list {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_h4: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    const NAMES: [&str; COUNT] = [
        "car_bomb_title",
        "barn_title",
        "vint_set_blend",
        "net_recv_migrated",
        "effect_activate",
        "detour_activate",
        "any_participant",
        "chop_shop_init",
        "act_packet_4e",
        "act_packet_56",
        "stunt_oncoming_e15",
        "stunt_back_seat_update",
    ];
    let mut parts: Vec<String> = Vec::new();
    for i in 0..COUNT {
        let c = CNT[i].load(Ordering::Relaxed);
        if c > 0 {
            parts.push(format!("{} {c}", NAMES[i]));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("port_h4: {}", parts.join(", "))
}
