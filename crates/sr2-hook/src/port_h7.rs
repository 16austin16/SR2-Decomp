//! Port module h7 (round 9): small functions from scratchpad/r9/list_h7.txt.
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each body keeps the
//! original's register flow: `St` carries the seven general registers into and out of every callee
//! call, so ECX / EDX / EBX / EDI / ESI / EBP leftovers match what the original leaves behind.
//!
//! Families:
//! - `vint_callback_find_by_hash` (pure hash-bucket walk, no calls);
//! - `vint_loc_to_lower` (an indirect call through a global object, else a byte-table lookup);
//! - `wsprintf_1024` (one cdecl call with four stack arguments);
//! - `scheduling_trigger::scalar_deleting_dtor` (vtable store, import calls, CRT free);
//! - `vint_object_tween::get_target_property` (two stdcall-style calls);
//! - `demo_derby_sp_info::load_from_table` (one thiscall, then a byte store);
//! - `rl_container_inst::is_ready` (a loop of virtual calls through each child);
//! - `act_packet_sub_0f` (writes its own stack argument's low byte, a call, a virtual call).
//! - `util_sprintf_scratch` and `get_start_message` (cdecl / thiscall helpers with stack buffers).
//!
//! Every thunk saves XMM0-7 and `pushad`s; the Rust body edits the saved registers, `popad` restores.

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 10;

const CAP: u32 = 1 << 24;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

// --- game addresses ------------------------------------------------------------------------------

const HASH_TABLE: usize = 0x022F_2570; // vint_callback_find_by_hash: 64 bucket heads
const LOC_OBJ: usize = 0x0252_A1E0; // vint_loc_to_lower: global object pointer
const LOC_TABLE: usize = 0x00DC_81D8; // vint_loc_to_lower: byte table
const WSPRINTF_CALL: usize = 0x00C9_9626; // wsprintf_1024's callee
const SDD_VT: u32 = 0x00E5_8C44; // scheduling_trigger vtable
const SDD_IMP_DTOR: usize = 0x00DB_80EC; // import slot: the member destructor (stdcall, 1 argument)
const NO_FREE_FLAG: usize = 0x0252_7BF7; // byte: 1 = the CRT free is skipped
const IMP_ENTER: usize = 0x00DB_80CC; // import slot: lock
const IMP_LEAVE: usize = 0x00DB_80D0; // import slot: unlock
const LOCK_ARG: u32 = 0x0347_ADE4;
const FREE: usize = 0x00CD_E870; // EAX = pointer; clobbers ECX / EDX
const TWEEN_GET: usize = 0x00B9_E930; // vint_object_tween: the member lookup (ECX = this)
const TWEEN_CALL: usize = 0x00B9_3800; // vint_object_tween: callee with two stdcall arguments
const DEMO_CALL: usize = 0x0053_D7C0; // demo_derby_sp_info: the thiscall callee (two stdcall arguments)
const RL_OBJ_VT_SLOT: usize = 0x18; // rl_container_inst::is_ready: slot 6 of each child
const ACT_CALL: usize = 0x0060_3F80; // act_packet_sub_0f: cdecl callee (one argument)
const ACT_GLOBAL: usize = 0x0140_B170; // act_packet_sub_0f: global object
const ACT_VT_SLOT: usize = 0x4C; // act_packet_sub_0f: slot 19
const UTIL_SPRINTF_CALL: usize = 0x00C9_6E3E; // util_sprintf_scratch's callee
const UTIL_SPRINTF_BUF: u32 = 0x0234_60E0; // util_sprintf_scratch: the pushed buffer
const START_MSG_CALL: usize = 0x00C1_4C40; // get_start_message's second callee (one argument)

/// The seven general registers as a straight-line asm function would hold them. `call` runs a game
/// function with exactly these registers (EBX / EBP and the callee's own stack arguments included)
/// and leaves in the struct what the callee left, so a port threads the original's register flow.
/// ESP is restored from a TEB scratch slot (`fs:[0x14]`, saved and restored around the call), so the
/// callee may pop its own arguments or not.
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

// --- ports ---------------------------------------------------------------------------------------

/// vint_callback_find_by_hash (0x00BA0950): walk the bucket `(key & 0x3F)` of a chained table,
/// ECX = the bucket head, EDX = the key, EAX = the node whose key (+0x14) matches, or 0.
extern "C" fn e_ba0950(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(0);
        let key = r.edx;
        let bucket = rd(HASH_TABLE + ((key & 0x3F) as usize) * 4);
        let mut n = bucket;
        let mut found = 0u32;
        if n != 0 {
            loop {
                if rd(n as usize + 0x14) == key {
                    found = n;
                    break;
                }
                let nx = rd(n as usize + 0x10);
                if nx == bucket || nx == 0 {
                    break;
                }
                n = nx;
            }
        }
        r.eax = found;
        r.ecx = bucket;
    }
}

/// vint_loc_to_lower (0x00B92240): when the global object and its slot +0x10 hold a function,
/// call it with ECX as one stack argument and return what it leaves. Otherwise EAX = a byte-table
/// lookup of CX (CX above 0xFF returns CX itself). ECX and EDX are left as they were.
extern "C" fn e_b92240(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(1);
        let g = rd(LOC_OBJ);
        if g != 0 {
            let f = rd(g as usize + 0x10);
            if f != 0 {
                let mut s = St::of(r);
                s.eax = f;
                s.calli(f, &[r.ecx]);
                r.eax = s.eax;
                r.ecx = s.ecx;
                r.edx = s.edx;
                r.ebx = s.ebx;
                r.esi = s.esi;
                r.edi = s.edi;
                r.ebp = s.ebp;
                return;
            }
        }
        let cx = r.ecx & 0xFFFF;
        r.eax = if cx > 0xFF { cx } else { rb(LOC_TABLE + cx as usize) as u32 };
    }
}

/// wsprintf_1024 (0x00C14C20): one cdecl call `f(edx, 0x400, arg1, &arg2)`, with ECX = arg1 and
/// EAX = &arg2 at the call. Returns the callee's EAX; ECX / EDX are the callee's leftovers.
extern "C" fn e_c14c20(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(2);
        let arg1 = rd(a as usize);
        let va = a as u32 + 4;
        let mut s = St::of(r);
        s.ecx = arg1;
        s.eax = va;
        s.call(WSPRINTF_CALL, &[r.edx, 0x400, arg1, va]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.esi = s.esi;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

/// scheduling_trigger::scalar_deleting_dtor (0x00BF43B0), `ret 4`: store the vtable, call the
/// member destructor (import, one stdcall argument this + 0xC), then, when bit 0 of the flag is
/// set and the CRT free is not skipped, lock / free / unlock. EAX = this.
extern "C" fn e_bf43b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(3);
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        s.eax = this.wrapping_add(0xC);
        wd(this as usize, SDD_VT);
        s.calli(rd(SDD_IMP_DTOR), &[this.wrapping_add(0xC)]);
        if rb(a as usize) & 1 != 0 {
            let flag = rb(NO_FREE_FLAG);
            s.ecx = (s.ecx & !0xFF) | flag as u32;
            if flag != 1 {
                s.calli(rd(IMP_ENTER), &[LOCK_ARG]);
                s.eax = s.esi;
                s.call(FREE, &[]);
                s.calli(rd(IMP_LEAVE), &[LOCK_ARG]);
            }
        }
        r.eax = s.esi;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

/// vint_object_tween::get_target_property (0x00B9F500), `ret 8`: call the member lookup (ECX =
/// this), then, if the slot at +0x94 is set, call the helper with (arg1, *slot); the result is
/// (*(this + 0x94) != 0).
extern "C" fn e_b9f500(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(4);
        let this = r.ecx;
        let mut s = St::of(r);
        s.edi = this;
        s.call(TWEEN_GET, &[]);
        let t = s.edi as usize;
        let p = rd(t + 0x94);
        if p != 0 {
            let v = rd(p as usize);
            let arg1 = rd(a as usize);
            s.ecx = arg1;
            s.eax = v;
            s.call(TWEEN_CALL, &[arg1, v]);
        }
        let t2 = s.edi as usize;
        r.eax = (rd(t2 + 0x94) != 0) as u32;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}

/// demo_derby_sp_info::load_from_table (0x005F0DD0), `ret 8`: call the thiscall helper with
/// (arg1, arg2) and ECX = this, then clear the byte at this + 0x194.
extern "C" fn e_5f0dd0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(5);
        let this = r.ecx;
        let arg1 = rd(a as usize);
        let arg2 = rd(a as usize + 4);
        let mut s = St::of(r);
        s.eax = arg2;
        s.esi = this;
        s.ecx = this;
        s.call(DEMO_CALL, &[arg1, arg2]);
        wb(s.esi as usize + 0x194, 0);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

/// rl_container_inst::is_ready (0x00CF6120), plain `ret`: for each child in the array at +0xC
/// (count at +8), call its slot 6; stop at the first that returns 0. AL = 1 when all did.
extern "C" fn e_cf6120(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(6);
        let mut s = St::of(r);
        s.edi = r.ecx;
        s.esi = 0;
        let mut ok = true;
        if rd(s.edi as usize + 8) > s.esi {
            s.ebx = s.edi.wrapping_add(0xC);
            loop {
                s.ecx = rd(s.ebx as usize);
                s.eax = rd(s.ecx as usize);
                let f = rd(s.eax as usize + RL_OBJ_VT_SLOT);
                s.edx = f;
                s.calli(f, &[]);
                if (s.eax as u8) == 0 {
                    ok = false;
                    break;
                }
                s.esi = s.esi.wrapping_add(1);
                s.ebx = s.ebx.wrapping_add(4);
                if !(s.esi < rd(s.edi as usize + 8)) {
                    break;
                }
            }
        }
        r.eax = (s.eax & !0xFF) | ok as u32;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebp = s.ebp;
    }
}

/// act_packet_sub_0f (0x00A479C0), plain `ret`: AL = (*arg1 != 0) is stored into the low byte of
/// the argument slot. When AL is set, call 0x603F80 with the slot; if it returns nonzero, return.
/// Otherwise call slot 19 of the global object's vtable with the slot's dword.
extern "C" fn e_a479c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let p = rd(a as usize);
        let al = (rb(p as usize) != 0) as u8;
        let mut s = St::of(r);
        s.eax = (p & !0xFF) | al as u32;
        wb(a as usize, al);
        if al != 0 {
            let slot = rd(a as usize);
            s.call(ACT_CALL, &[slot]);
            if (s.eax as u8) != 0 {
                r.eax = s.eax;
                r.ecx = s.ecx;
                r.edx = s.edx;
                r.ebx = s.ebx;
                r.esi = s.esi;
                r.edi = s.edi;
                r.ebp = s.ebp;
                return;
            }
        }
        let obj = rd(ACT_GLOBAL);
        s.ecx = obj;
        let vt = rd(obj as usize);
        let f = rd(vt as usize + ACT_VT_SLOT);
        let slot = rd(a as usize);
        s.edx = f;
        s.eax = slot;
        s.calli(f, &[slot]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.esi = s.esi;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

/// util_sprintf_scratch (0x00BE1B10): one cdecl call `f(0x23460E0, arg1, &arg2)` with ECX = arg1
/// and EAX = &arg2 at the call. EDX is left as it was going in, then as the callee leaves it.
extern "C" fn e_be1b10(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(8);
        let arg1 = rd(a as usize);
        let va = a as u32 + 4;
        let mut s = St::of(r);
        s.ecx = arg1;
        s.eax = va;
        s.call(UTIL_SPRINTF_CALL, &[UTIL_SPRINTF_BUF, arg1, va]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.esi = s.esi;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

/// base_jumping_diversion_type_info::get_start_message (0x005F95C0), `ret 8`: call the string
/// helper with EDI / ESI constants, then call 0x00C14C40 with its result, with ESI = arg2 and
/// EDI = arg1 at that call. Returns the second callee's EAX.
extern "C" fn e_5f95c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(9);
        let arg1 = rd(a as usize);
        let arg2 = rd(a as usize + 4);
        let mut s = St::of(r);
        s.edi = 0x00DF_21C8;
        s.esi = 0x00DF_2220;
        s.call(0x007F_49E0, &[]);
        s.esi = arg2;
        s.edi = arg1;
        let res = s.eax;
        s.call(START_MSG_CALL, &[res]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}

thunk!(t_ba0950, e_ba0950, "ret");
thunk!(t_b92240, e_b92240, "ret");
thunk!(t_c14c20, e_c14c20, "ret");
thunk!(t_bf43b0, e_bf43b0, "ret 4");
thunk!(t_b9f500, e_b9f500, "ret 8");
thunk!(t_5f0dd0, e_5f0dd0, "ret 8");
thunk!(t_cf6120, e_cf6120, "ret");
thunk!(t_a479c0, e_a479c0, "ret");
thunk!(t_be1b10, e_be1b10, "ret");
thunk!(t_5f95c0, e_5f95c0, "ret 8");

// --- the originals' bytes (true ends, from tools/body_bytes.py) ------------------------------------

const B_BA0950: [u8; 37] = [
    0x8B, 0xC2, 0x83, 0xE0, 0x3F, 0x8B, 0x0C, 0x85, 0x70, 0x25, 0x2F, 0x02, 0x8B, 0xC1, 0x85, 0xC0,
    0x74, 0x10, 0x39, 0x50, 0x14, 0x74, 0x0D, 0x8B, 0x40, 0x10, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0,
    0x75, 0xF0, 0x33, 0xC0, 0xC3,
];
const B_B92240: [u8; 41] = [
    0xA1, 0xE0, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0E, 0x8B, 0x40, 0x10, 0x85, 0xC0, 0x74, 0x07,
    0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xC3, 0x66, 0x81, 0xF9, 0xFF, 0x00, 0x0F, 0xB7, 0xC1, 0x77,
    0x07, 0x0F, 0xB6, 0x80, 0xD8, 0x81, 0xDC, 0x00, 0xC3,
];
const B_C14C20: [u8; 25] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8D, 0x44, 0x24, 0x08, 0x50, 0x51, 0x68, 0x00, 0x04, 0x00, 0x00, 0x52,
    0xE8, 0xF1, 0x49, 0x08, 0x00, 0x83, 0xC4, 0x10, 0xC3,
];
const B_BF43B0: [u8; 72] = [
    0x56, 0x8B, 0xF1, 0x8D, 0x46, 0x0C, 0x50, 0xC7, 0x06, 0x44, 0x8C, 0xE5, 0x00, 0xFF, 0x15, 0xEC,
    0x80, 0xDB, 0x00, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x28, 0x8A, 0x0D, 0xF7, 0x7B, 0x52, 0x02,
    0x80, 0xF9, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00,
    0x8B, 0xC6, 0xE8, 0x89, 0xA4, 0x0E, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80,
    0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00,
];
const B_B9F500: [u8; 46] = [
    0x57, 0x8B, 0xF9, 0xE8, 0x28, 0xF4, 0xFF, 0xFF, 0x8B, 0x87, 0x94, 0x00, 0x00, 0x00, 0x85, 0xC0,
    0x74, 0x0D, 0x8B, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x50, 0x51, 0xE8, 0xE1, 0x42, 0xFF, 0xFF, 0x33,
    0xC0, 0x39, 0x87, 0x94, 0x00, 0x00, 0x00, 0x5F, 0x0F, 0x95, 0xC0, 0xC2, 0x08, 0x00,
];
const B_5F0DD0: [u8; 31] = [
    0x8B, 0x44, 0x24, 0x08, 0x56, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x08, 0x50, 0x51, 0x8B, 0xCE, 0xE8,
    0xDC, 0xC9, 0xF4, 0xFF, 0xC6, 0x86, 0x94, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC2, 0x08, 0x00,
];
const B_CF6120: [u8; 52] = [
    0x53, 0x56, 0x57, 0x8B, 0xF9, 0x33, 0xF6, 0x39, 0x77, 0x08, 0x76, 0x1C, 0x8D, 0x5F, 0x0C, 0x90,
    0x8B, 0x0B, 0x8B, 0x01, 0x8B, 0x50, 0x18, 0xFF, 0xD2, 0x84, 0xC0, 0x74, 0x11, 0x83, 0xC6, 0x01,
    0x83, 0xC3, 0x04, 0x3B, 0x77, 0x08, 0x72, 0xE8, 0x5F, 0x5E, 0xB0, 0x01, 0x5B, 0xC3, 0x5F, 0x5E,
    0x32, 0xC0, 0x5B, 0xC3,
];
const B_A479C0: [u8; 46] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0x38, 0x00, 0x0F, 0x95, 0xC0, 0x84, 0xC0, 0x88, 0x44, 0x24, 0x04,
    0x74, 0x09, 0xE8, 0xA9, 0xC5, 0xBB, 0xFF, 0x84, 0xC0, 0x75, 0x12, 0x8B, 0x0D, 0x70, 0xB1, 0x40,
    0x01, 0x8B, 0x11, 0x8B, 0x44, 0x24, 0x04, 0x8B, 0x52, 0x4C, 0x50, 0xFF, 0xD2, 0xC3,
];

const B_BE1B10: [u8; 24] = [
    0x8B, 0x4C, 0x24, 0x04, 0x8D, 0x44, 0x24, 0x08, 0x50, 0x51, 0x68, 0xE0, 0x60, 0x34, 0x02, 0xE8,
    0x1A, 0x53, 0x0B, 0x00, 0x83, 0xC4, 0x0C, 0xC3,
];
const B_5F95C0: [u8; 39] = [
    0x56, 0x57, 0xBF, 0xC8, 0x21, 0xDF, 0x00, 0xBE, 0x20, 0x22, 0xDF, 0x00, 0xE8, 0x0F, 0xB4, 0x1F,
    0x00, 0x8B, 0x74, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x0C, 0x50, 0xE8, 0x61, 0xB6, 0x61, 0x00, 0x83,
    0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00,
];

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "vint_callback_find_by_hash", addr: 0x00BA0950, body: &B_BA0950, thunk: t_ba0950 },
    Port { name: "vint_loc_to_lower", addr: 0x00B92240, body: &B_B92240, thunk: t_b92240 },
    Port { name: "wsprintf_1024", addr: 0x00C14C20, body: &B_C14C20, thunk: t_c14c20 },
    Port { name: "scheduling_trigger::scalar_deleting_dtor", addr: 0x00BF43B0, body: &B_BF43B0, thunk: t_bf43b0 },
    Port { name: "vint_object_tween::get_target_property", addr: 0x00B9F500, body: &B_B9F500, thunk: t_b9f500 },
    Port { name: "demo_derby_sp_info::load_from_table", addr: 0x005F0DD0, body: &B_5F0DD0, thunk: t_5f0dd0 },
    Port { name: "rl_container_inst::is_ready", addr: 0x00CF6120, body: &B_CF6120, thunk: t_cf6120 },
    Port { name: "act_packet_sub_0f", addr: 0x00A479C0, body: &B_A479C0, thunk: t_a479c0 },
    Port { name: "util_sprintf_scratch", addr: 0x00BE1B10, body: &B_BE1B10, thunk: t_be1b10 },
    Port { name: "base_jumping_diversion_type_info::get_start_message", addr: 0x005F95C0, body: &B_5F95C0, thunk: t_5f95c0 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h7: replaced {ok}/{COUNT} with Rust"));
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
    format!("port_h7: {}", parts.join(", "))
}
