//! Port module h6: small functions from the round 9 list (16-200 bytes).
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each port runs behind a
//! `pushad` thunk that also saves XMM0-7; the Rust body edits the saved registers and `popad`
//! restores them. Callees stay original: they are called by address (`St::call` / `St::calli`) with
//! the registers the original sets up, and the registers each callee leaves are copied back, so ECX
//! / EDX / EBX / EBP / ESI / EDI flow out exactly as the original leaves them.
//!
//! Checks: per-function call counters (`summary`); acceptance is the judge (`sr2-judge`).

#![allow(dead_code, unused_imports, unused_macros)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 12;

const CAP: u32 = 1 << 24;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

// --- registers and thunks ------------------------------------------------------------------------

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

/// The seven general registers as the original holds them at a call site.
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

    /// Call a game function by address; returns its EAX (the registers it leaves are kept in self).
    #[inline(always)]
    unsafe fn call(&mut self, f: usize, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f, self, args) };
        self.eax
    }

    /// Call through a function pointer read from game memory (virtual or import calls).
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

// --- ports ---

// render_lists_reset (0x0052E7C0, 128 bytes): clears the render-list counters and the frame
// allocator stats, calls virtual slot 3 of the global object at 0x2791848 (ECX = that object).
// EBX is zeroed for the call and restored after it.
const RL_OBJ: usize = 0x0279_1848;

extern "C" fn e_52e7c0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(0);
        let mut s = St::of(r);
        let ebx_in = s.ebx;
        for g in [0x0278_4990usize, 0x0278_9998, 0x0278_C1A0, 0x0278_D5A8, 0x0278_E9B0, 0x0278_EC38, 0x0278_ED80, 0x0279_0188, 0x0279_0D48, 0x0279_1840] {
            wd(g, 0);
        }
        let vt = rd(RL_OBJ);
        let fp = rd(vt as usize + 0xC);
        s.eax = vt;
        s.edx = fp;
        s.ecx = RL_OBJ as u32;
        s.ebx = 0;
        let res = s.calli(fp, &[]);
        s.ebx = ebx_in;
        let v = rd(0x0279_1884).wrapping_sub(res);
        s.ecx = v;
        wd(0x00FA_6C20, v);
        wd(0x0279_188C, 0);
        wd(0x00FA_6C24, 0);
        wd(0x00FA_6C28, 0xFFFF_FFFF);
        wd(0x0252_73EC, 0);
        wb(0x0252_73E6, 0);
        s.eax = res;
        s.put(r);
    }
}

thunk!(t_52e7c0, e_52e7c0, "ret");

// camera_ctx_pred_human_shield (0x0049A9B0, 80 bytes): a predicate on the human object at EDX.
extern "C" fn e_49a9b0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(1);
        let mut s = St::of(r);
        let mut done = false;
        if rd(s.edx as usize + 0xEA8) != 0 {
            s.call(0x009B_3680, &[]);
            if s.eax & 0xFF == 0 {
                s.eax = (s.eax & !0xFF) | 1;
                done = true;
            }
        }
        if !done {
            if rb(s.edx as usize + 0x149) == 0x22 && rd(s.edx as usize + 0x158) == 0 {
                let obj = rd(s.edx as usize + 0x5E0);
                s.edx = obj;
                s.eax = rd(obj as usize + 0x168);
                let a0 = rd(0x015B_873C);
                let a1 = s.eax;
                s.ecx = rd(obj as usize);
                let res = s.call(0x00CE_CBC0, &[a0, a1]);
                s.eax = (res & !0xFF) | u32::from(res & 0xFF != 0);
            } else {
                s.eax &= !0xFF;
            }
        }
        s.put(r);
    }
}

thunk!(t_49a9b0, e_49a9b0, "ret");

// vint_object_tween::get_target_handle (0x00B9F3C0, 40 bytes; ret 8).
extern "C" fn e_b9f3c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(2);
        let mut s = St::of(r);
        let edi_in = s.edi;
        s.edi = s.ecx;
        s.call(0x00B9_E930, &[]);
        let h = rd(s.edi as usize + 0x88);
        let arg0 = rd(a as usize);
        s.eax = h;
        s.ecx = arg0;
        s.call(0x00B9_3490, &[arg0, h]);
        let flag = rd(s.edi as usize + 0x88) != 0;
        s.edi = edi_in;
        s.eax = u32::from(flag);
        s.put(r);
    }
}

thunk!(t_b9f3c0, e_b9f3c0, "ret 8");

// rl_decal_inst::pre_render (0x00D75500, 26 bytes; ret 4): ESI = this + 0x70, a test, then a call.
extern "C" fn e_d75500(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(3);
        let mut s = St::of(r);
        let esi_in = s.esi;
        s.esi = s.ecx.wrapping_add(0x70);
        s.eax = s.esi;
        let r1 = s.call(0x00D0_78E0, &[]);
        if r1 & 0xFF == 0 {
            s.eax = s.esi;
            s.call(0x00D0_7940, &[]);
        }
        s.esi = esi_in;
        s.put(r);
    }
}

thunk!(t_d75500, e_d75500, "ret 4");

// net_recv_strm_req (0x00A79210, 40 bytes; ret). Reads the request fields from the pointer
// argument, two cdecl calls with (ESI, 0) and (ESI, EBX) as they stand after the first call.
extern "C" fn e_a79210(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(4);
        let mut s = St::of(r);
        let (ebx_in, esi_in, edi_in) = (s.ebx, s.esi, s.edi);
        let p = rd(a as usize) as usize;
        let ebx_v = rd(p + 5);
        let esi_v = rb(p + 4) as i8 as i32 as u32;
        let edi_v = rd(p);
        s.eax = p as u32;
        s.ebx = ebx_v;
        s.esi = esi_v;
        s.edi = edi_v;
        s.ecx = ebx_v;
        s.call(0x00A7_9120, &[esi_v, 0]);
        // The first call's pushed arguments stay on the stack (the caller cleans up after call 2).
        let (a0, a1) = (s.esi, s.ebx);
        s.call(0x00A7_8D40, &[a0, a1, esi_v, 0]);
        s.ebx = ebx_in;
        s.esi = esi_in;
        s.edi = edi_in;
        s.put(r);
    }
}

thunk!(t_a79210, e_a79210, "ret");

// async_virtual_mempool::reset (0x00C17CF0, 37 bytes): lock at this + 0x7C, a virtual call
// (slot 13, ECX = this, one zero argument), unlock. Returns AL = 1.
extern "C" fn e_c17cf0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(5);
        let mut s = St::of(r);
        let (esi_in, edi_in) = (s.esi, s.edi);
        s.esi = s.ecx;
        s.edi = s.esi.wrapping_add(0x7C);
        let lock = rd(0x00DB_80CC);
        let edi = s.edi;
        s.calli(lock, &[edi]);
        s.eax = rd(s.esi as usize);
        let fp = rd(s.eax as usize + 0x34);
        s.edx = fp;
        s.ecx = s.esi;
        s.calli(fp, &[0]);
        let unlock = rd(0x00DB_80D0);
        let edi = s.edi;
        s.calli(unlock, &[edi]);
        s.edi = edi_in;
        s.esi = esi_in;
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
    }
}

thunk!(t_c17cf0, e_c17cf0, "ret");

// anim_track_mask_clear_every_third_bit (0x00CE5780, 38 bytes): clears bit (i & 7) of the byte at
// ESI + 0x2C + i / 8 for i = 0, 3, 6, ... below 0xD5. No calls.
extern "C" fn e_ce5780(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(11);
        let mut s = St::of(r);
        let esi = s.esi;
        let mut eax: u32 = 0;
        let mut ecx: u32;
        let mut edx: u32;
        loop {
            let t = rb(0x00DC_8558 + (eax as usize & 7) * 4);
            let m = !t;
            edx = u32::from(m);
            ecx = (eax >> 3).wrapping_add(esi).wrapping_add(0x2C);
            wb(ecx as usize, rb(ecx as usize) & m);
            eax = eax.wrapping_add(3);
            if eax >= 0xD5 {
                break;
            }
        }
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.put(r);
    }
}

thunk!(t_ce5780, e_ce5780, "ret");

// vandalism_news_spawn_group::is_active (0x00669830, 71 bytes): a virtual call (slot 1) on the
// object behind two globals, then a compare of its result and a field of the global at 0x1408E88.
extern "C" fn e_669830(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(6);
        let mut s = St::of(r);
        let esi_in = s.esi;
        s.eax = rd(0x0140_8E88);
        'x: {
            if s.eax == 0 {
                s.eax &= !0xFF;
                break 'x;
            }
            s.esi = rd(s.eax as usize + 0x10);
            if s.esi == 0 {
                s.eax &= !0xFF;
                break 'x;
            }
            s.eax = rd(0x027B_3C60);
            if s.eax == 0 || rd(s.eax as usize + 0x3C) != 1 {
                s.eax = 0;
                break 'x;
            }
            s.ecx = rd(s.eax as usize + 0x90);
            s.eax = rd(s.ecx as usize);
            let fp = rd(s.eax as usize + 4);
            s.edx = fp;
            s.calli(fp, &[]);
            s.eax = if s.eax != 0x10 || rd(s.esi as usize + 0x78) < 0x14 { 0 } else { 1 };
        }
        s.esi = esi_in;
        s.put(r);
    }
}

thunk!(t_669830, e_669830, "ret");

// racing_diversion_type_info::create_instance (0x00615050, 58 bytes; ret 4): a call with (ESI, 0),
// then, when the global at 0x2527B14 is set, a virtual call (slot 7) with (0x10, 0) and a store.
extern "C" fn e_615050(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        let mut s = St::of(r);
        let esi_in = s.esi;
        s.esi = rd(a as usize);
        let arg = s.esi;
        s.call(0x0068_0E40, &[arg, 0]);
        if rd(0x0252_7B14) != 0 {
            s.eax = rd(s.esi as usize);
            let fp = rd(s.eax as usize + 0x1C);
            s.edx = fp;
            s.ecx = s.esi;
            let res = s.calli(fp, &[0x10, 0]);
            if res != 0 {
                wd(res as usize, 0x00DF_58A4);
                s.eax = res;
            } else {
                s.eax = 0;
            }
        } else {
            s.eax = 0;
        }
        s.esi = esi_in;
        s.put(r);
    }
}

thunk!(t_615050, e_615050, "ret 4");

// vint_element_text::get_font (0x00BA3200, 66 bytes; ret 8): a 10-entry table lookup keyed by the
// 16-bit value at this + 0xA0; on a match, one call (cdecl-free, one stack argument).
extern "C" fn e_ba3200(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(8);
        let mut s = St::of(r);
        let esi_in = s.esi;
        s.eax = rd(s.ecx as usize + 0xA0);
        let mut ok = false;
        if s.eax != 0 && (s.eax & 0xFFFF) < 0xA {
            let c = s.eax & 0xFFFF;
            let edx = c.wrapping_mul(8).wrapping_sub(c);
            s.edx = edx;
            let ecx = edx.wrapping_mul(4).wrapping_add(0x031B_C400);
            s.ecx = ecx;
            if rd(0x031B_C408 + edx as usize * 4) == s.eax {
                s.esi = rd(a as usize);
                s.call(0x00B9_3340, &[ecx]);
                ok = true;
            }
        }
        s.eax = if ok { (s.eax & !0xFF) | 1 } else { s.eax & !0xFF };
        s.esi = esi_in;
        s.put(r);
    }
}

thunk!(t_ba3200, e_ba3200, "ret 8");

// air_complication::load_from_table (0x0066E200, 52 bytes; ret 4): two calls, then a store of
// 1000 * the word at this + 0x24 back to it.
extern "C" fn e_66e200(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(9);
        let mut s = St::of(r);
        let (ebx_in, esi_in, edi_in) = (s.ebx, s.esi, s.edi);
        s.ebx = rd(a as usize);
        s.edi = s.ecx;
        let arg = s.ebx;
        s.call(0x0066_DB90, &[arg]);
        s.esi = s.edi.wrapping_add(0x24);
        s.ecx = 0x00DF_DC6C;
        let b = s.ebx;
        s.call(0x00B7_47D0, &[b]);
        s.eax = rd(s.esi as usize).wrapping_mul(1000);
        wb(s.edi as usize + 0x2C, 0);
        s.edi = edi_in;
        wd(s.esi as usize, s.eax);
        s.esi = esi_in;
        s.ebx = ebx_in;
        s.put(r);
    }
}

thunk!(t_66e200, e_66e200, "ret 4");

// event_trigger::wait (0x00BF3E70, 86 bytes; ret): a critical section at this + 0xC around one flag
// store, then (when the two fields are set) a call and one virtual-table style import call.
extern "C" fn e_bf3e70(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(10);
        let mut s = St::of(r);
        let (ebx_in, esi_in, edi_in) = (s.ebx, s.esi, s.edi);
        s.esi = s.ecx;
        s.edi = s.esi.wrapping_add(0x0C);
        let enter = rd(0x00DB_80CC);
        let a = s.edi;
        s.calli(enter, &[a]);
        wb(s.esi as usize + 8, 1);
        s.ebx = rd(s.esi as usize + 4);
        let leave = rd(0x00DB_80D0);
        let a = s.edi;
        s.calli(leave, &[a]);
        s.eax = rd(s.esi as usize + 0x28);
        if s.eax != 0 && s.ebx != 0 {
            if rd(s.esi as usize + 0x34) != 0 {
                let b = rb(0x0252_A3B2);
                s.ecx = (s.ecx & !0xFF) | u32::from(b);
                if b == 0 {
                    s.call(0x00BF_3AD0, &[]);
                }
            }
            let edx = rd(s.esi as usize + 0x2C);
            s.edx = edx;
            let f = rd(0x00DB_8100);
            s.calli(f, &[edx, 0xFFFF_FFFF]);
            s.eax = (s.eax & !0xFF) | u32::from(rb(s.esi as usize + 0x38));
            s.ecx = rd(s.esi as usize + 4);
            s.edx = (s.edx & !0xFF) | u32::from(rb(s.esi as usize + 0x38));
        }
        s.edi = edi_in;
        s.esi = esi_in;
        s.ebx = ebx_in;
        s.put(r);
    }
}

thunk!(t_bf3e70, e_bf3e70, "ret");

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

const B_52E7C0: [u8; 128] = [
    0xA1, 0x48, 0x18, 0x79, 0x02, 0x8B, 0x50, 0x0C, 0x53, 0x33, 0xDB, 0xB9, 0x48, 0x18, 0x79, 0x02,
    0x89, 0x1D, 0x90, 0x49, 0x78, 0x02, 0x89, 0x1D, 0x98, 0x99, 0x78, 0x02, 0x89, 0x1D, 0xA0, 0xC1,
    0x78, 0x02, 0x89, 0x1D, 0xA8, 0xD5, 0x78, 0x02, 0x89, 0x1D, 0xB0, 0xE9, 0x78, 0x02, 0x89, 0x1D,
    0x38, 0xEC, 0x78, 0x02, 0x89, 0x1D, 0x80, 0xED, 0x78, 0x02, 0x89, 0x1D, 0x88, 0x01, 0x79, 0x02,
    0x89, 0x1D, 0x48, 0x0D, 0x79, 0x02, 0x89, 0x1D, 0x40, 0x18, 0x79, 0x02, 0xFF, 0xD2, 0x8B, 0x0D,
    0x84, 0x18, 0x79, 0x02, 0x2B, 0xC8, 0x89, 0x0D, 0x20, 0x6C, 0xFA, 0x00, 0x89, 0x1D, 0x8C, 0x18,
    0x79, 0x02, 0x89, 0x1D, 0x24, 0x6C, 0xFA, 0x00, 0xC7, 0x05, 0x28, 0x6C, 0xFA, 0x00, 0xFF, 0xFF,
    0xFF, 0xFF, 0x89, 0x1D, 0xEC, 0x73, 0x52, 0x02, 0x88, 0x1D, 0xE6, 0x73, 0x52, 0x02, 0x5B, 0xC3,
];
const B_49A9B0: [u8; 78] = [
    0x83, 0xBA, 0xA8, 0x0E, 0x00, 0x00, 0x00, 0x74, 0x0C, 0xE8, 0xC2, 0x8C, 0x51, 0x00, 0x84, 0xC0,
    0x75, 0x03, 0xB0, 0x01, 0xC3, 0x80, 0xBA, 0x49, 0x01, 0x00, 0x00, 0x22, 0x75, 0x2D, 0x83, 0xBA,
    0x58, 0x01, 0x00, 0x00, 0x00, 0x75, 0x24, 0x8B, 0x92, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0x82, 0x68,
    0x01, 0x00, 0x00, 0x8B, 0x0D, 0x3C, 0x87, 0x5B, 0x01, 0x50, 0x51, 0x8B, 0x0A, 0xE8, 0xCE, 0x21,
    0x85, 0x00, 0x83, 0xC4, 0x08, 0x84, 0xC0, 0x0F, 0x95, 0xC0, 0xC3, 0x32, 0xC0, 0xC3,
];
const B_B9F3C0: [u8; 40] = [
    0x57, 0x8B, 0xF9, 0xE8, 0x68, 0xF5, 0xFF, 0xFF, 0x8B, 0x87, 0x88, 0x00, 0x00, 0x00, 0x8B, 0x4C,
    0x24, 0x08, 0x50, 0x51, 0xE8, 0xB7, 0x40, 0xFF, 0xFF, 0x33, 0xC0, 0x39, 0x87, 0x88, 0x00, 0x00,
    0x00, 0x5F, 0x0F, 0x95, 0xC0, 0xC2, 0x08, 0x00,
];
const B_D75500: [u8; 26] = [
    0x56, 0x8D, 0x71, 0x70, 0x8B, 0xC6, 0xE8, 0xD5, 0x23, 0xF9, 0xFF, 0x84, 0xC0, 0x75, 0x07, 0x8B,
    0xC6, 0xE8, 0x2A, 0x24, 0xF9, 0xFF, 0x5E, 0xC2, 0x04, 0x00,
];

const B_A79210: [u8; 40] = [
    0x8B, 0x44, 0x24, 0x04, 0x53, 0x8B, 0x58, 0x05, 0x56, 0x0F, 0xBE, 0x70, 0x04, 0x57, 0x8B, 0x38,
    0x6A, 0x00, 0x56, 0x8B, 0xCB, 0xE8, 0xF6, 0xFE, 0xFF, 0xFF, 0x53, 0x56, 0xE8, 0x0F, 0xFB, 0xFF,
    0xFF, 0x83, 0xC4, 0x10, 0x5F, 0x5E, 0x5B, 0xC3,
];
const B_CE5780: [u8; 38] = [
    0x33, 0xC0, 0x8B, 0xD0, 0x83, 0xE2, 0x07, 0x8A, 0x14, 0x95, 0x58, 0x85, 0xDC, 0x00, 0x8B, 0xC8,
    0xC1, 0xE9, 0x03, 0x8D, 0x4C, 0x31, 0x2C, 0xF6, 0xD2, 0x20, 0x11, 0x83, 0xC0, 0x03, 0x3D, 0xD5,
    0x00, 0x00, 0x00, 0x72, 0xDD, 0xC3,
];

const B_C17CF0: [u8; 37] = [
    0x56, 0x57, 0x8B, 0xF1, 0x8D, 0x7E, 0x7C, 0x57, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0x06,
    0x8B, 0x50, 0x34, 0x6A, 0x00, 0x8B, 0xCE, 0xFF, 0xD2, 0x57, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00,
    0x5F, 0xB0, 0x01, 0x5E, 0xC3,
];

const B_669830: [u8; 71] = [
    0xA1, 0x88, 0x8E, 0x40, 0x01, 0x85, 0xC0, 0x56, 0x74, 0x39, 0x8B, 0x70, 0x10, 0x85, 0xF6, 0x74,
    0x32, 0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x1F,
    0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0x83, 0xF8, 0x10,
    0x75, 0x0D, 0x83, 0x7E, 0x78, 0x14, 0x72, 0x07, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x33,
    0xC0, 0x5E, 0xC3, 0x32, 0xC0, 0x5E, 0xC3,
];
const B_615050: [u8; 58] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x6A, 0x00, 0x56, 0xE8, 0xE3, 0xBD, 0x06, 0x00, 0x83, 0xC4, 0x08,
    0x83, 0x3D, 0x14, 0x7B, 0x52, 0x02, 0x00, 0x74, 0x1B, 0x8B, 0x06, 0x8B, 0x50, 0x1C, 0x6A, 0x00,
    0x6A, 0x10, 0x8B, 0xCE, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x0A, 0xC7, 0x00, 0xA4, 0x58, 0xDF, 0x00,
    0x5E, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0x5E, 0xC2, 0x04, 0x00,
];
const B_BA3200: [u8; 66] = [
    0x8B, 0x81, 0xA0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x33, 0x66, 0x3D, 0x0A, 0x00, 0x73, 0x2D,
    0x0F, 0xB7, 0xC8, 0x8D, 0x14, 0xCD, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xD1, 0x39, 0x04, 0x95, 0x08,
    0xC4, 0x1B, 0x03, 0x8D, 0x0C, 0x95, 0x00, 0xC4, 0x1B, 0x03, 0x75, 0x11, 0x56, 0x8B, 0x74, 0x24,
    0x08, 0x51, 0xE8, 0x09, 0x01, 0xFF, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00, 0x32, 0xC0, 0xC2,
    0x08, 0x00,
];
const B_66E200: [u8; 52] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x53, 0x8B, 0xF9, 0xE8, 0x81, 0xF9, 0xFF, 0xFF, 0x8D,
    0x77, 0x24, 0x53, 0xB9, 0x6C, 0xDC, 0xDF, 0x00, 0xE8, 0xB3, 0x65, 0x50, 0x00, 0x8B, 0x06, 0x69,
    0xC0, 0xE8, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x04, 0xC6, 0x47, 0x2C, 0x00, 0x5F, 0x89, 0x06, 0x5E,
    0x5B, 0xC2, 0x04, 0x00,
];

const B_BF3E70: [u8; 86] = [
    0x53, 0x56, 0x57, 0x8B, 0xF1, 0x8D, 0x7E, 0x0C, 0x57, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0xC6,
    0x46, 0x08, 0x01, 0x8B, 0x5E, 0x04, 0x57, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0x46, 0x28,
    0x85, 0xC0, 0x74, 0x2E, 0x85, 0xDB, 0x74, 0x2A, 0x83, 0x7E, 0x34, 0x00, 0x74, 0x0F, 0x8A, 0x0D,
    0xB2, 0xA3, 0x52, 0x02, 0x84, 0xC9, 0x75, 0x05, 0xE8, 0x23, 0xFC, 0xFF, 0xFF, 0x8B, 0x56, 0x2C,
    0x6A, 0xFF, 0x52, 0xFF, 0x15, 0x00, 0x81, 0xDB, 0x00, 0x8A, 0x46, 0x38, 0x8B, 0x4E, 0x04, 0x8A,
    0x56, 0x38, 0x5F, 0x5E, 0x5B, 0xC3,
];

static PORTS: [Port; COUNT] = [
    Port { name: "render_lists_reset", addr: 0x0052_E7C0, body: &B_52E7C0, thunk: t_52e7c0 },
    Port { name: "camera_ctx_pred_human_shield", addr: 0x0049_A9B0, body: &B_49A9B0, thunk: t_49a9b0 },
    Port { name: "vint_object_tween::get_target_handle", addr: 0x00B9_F3C0, body: &B_B9F3C0, thunk: t_b9f3c0 },
    Port { name: "rl_decal_inst::pre_render", addr: 0x00D7_5500, body: &B_D75500, thunk: t_d75500 },
    Port { name: "net_recv_strm_req", addr: 0x00A7_9210, body: &B_A79210, thunk: t_a79210 },
    Port { name: "async_virtual_mempool::reset", addr: 0x00C1_7CF0, body: &B_C17CF0, thunk: t_c17cf0 },
    Port { name: "vandalism_news_spawn_group::is_active", addr: 0x0066_9830, body: &B_669830, thunk: t_669830 },
    Port { name: "racing_diversion_type_info::create_instance", addr: 0x0061_5050, body: &B_615050, thunk: t_615050 },
    Port { name: "vint_element_text::get_font", addr: 0x00BA_3200, body: &B_BA3200, thunk: t_ba3200 },
    Port { name: "air_complication::load_from_table", addr: 0x0066_E200, body: &B_66E200, thunk: t_66e200 },
    Port { name: "event_trigger::wait", addr: 0x00BF_3E70, body: &B_BF3E70, thunk: t_bf3e70 },
    Port { name: "anim_track_mask_clear_every_third_bit", addr: 0x00CE_5780, body: &B_CE5780, thunk: t_ce5780 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_h6: replaced {ok}/{COUNT} with Rust"));
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
    format!("port_h6: {}", parts.join(", "))
}
