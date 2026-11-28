//! Round 9, list s5: small functions (AI orders, net packets, diversions, vint, script objects ...).
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each thunk saves XMM0-7
//! and `pushad`s; the Rust body edits the saved registers (`Pushad`), `popad` restores them. Callees
//! stay original and are called by address with the exact registers (EBX / EBP included) through
//! `St::call`. The judge (crates/sr2-judge) is the acceptance test; no per-function shadow checks.

#![allow(unused_imports)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const CAP: u32 = 1 << 24;

#[inline(always)]
fn count_at(c: &AtomicU32) {
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// The general registers as a straight-line asm function would hold them.
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

    /// Write back EAX / ECX / EDX / EBP (EBX / ESI / EDI are saved and restored by most bodies;
    /// use `put_all` when the original leaves them as the callee did).
    #[inline(always)]
    fn put(&self, r: &mut Pushad) {
        r.eax = self.eax;
        r.ecx = self.ecx;
        r.edx = self.edx;
        r.ebp = self.ebp;
    }

    #[inline(always)]
    fn put_all(&self, r: &mut Pushad) {
        r.eax = self.eax;
        r.ecx = self.ecx;
        r.edx = self.edx;
        r.ebx = self.ebx;
        r.esi = self.esi;
        r.edi = self.edi;
        r.ebp = self.ebp;
    }

    /// Call a game function by address with exactly these registers; returns its EAX and leaves
    /// in `self` what the callee left.
    #[inline(always)]
    unsafe fn call(&mut self, f: usize, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f, self, args) };
        self.eax
    }
}

#[inline(never)]
unsafe fn call_regs_st(f: usize, st: &mut St, args: &[u32]) {
    let mut a8 = [0u32; 16];
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
            "mov ecx, 16",
            "2:",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jnz 2b",
            "mov ebx, [ebx + 8]",
            "mov eax, [ebx]", "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]",
            "mov esi, [ebx + 16]", "mov edi, [ebx + 20]", "mov ebp, [ebx + 24]", "mov ebx, [ebx + 12]",
            "call dword ptr [esp + 64]",
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

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
    cnt: &'static AtomicU32,
}

// --- helpers ---------------------------------------------------------------------------------------

/// The caller's first eight stack slots, for a tail jump (the callee sees the caller's arguments).
#[inline(always)]
unsafe fn fwd(a: *const u32) -> [u32; 8] {
    unsafe { [rd(a as usize), rd(a as usize + 4), rd(a as usize + 8), rd(a as usize + 12), rd(a as usize + 16), rd(a as usize + 20), rd(a as usize + 24), rd(a as usize + 28)] }
}

/// The stack as a callee sees it: `top` (arguments, then bytes the body pushed, most recent first),
/// then the return address of the original, then the caller's own arguments.
#[inline(always)]
unsafe fn im(r: &Pushad, a: *const u32, top: &[u32]) -> [u32; 16] {
    unsafe {
        let mut o = [0u32; 16];
        let mut n = 0;
        for &v in top.iter().take(16) {
            o[n] = v;
            n += 1;
        }
        if n < 16 {
            o[n] = rd(r.esp as usize);
            n += 1;
        }
        let mut k = 0;
        while n < 16 {
            o[n] = rd(a as usize + 4 * k);
            n += 1;
            k += 1;
        }
        o
    }
}

#[inline(always)]
unsafe fn arg(a: *const u32, i: usize) -> u32 {
    unsafe { rd(a as usize + 4 * i) }
}

// --- 0x004AEAE0 chunk_section_1c_object_attachments (EAX = ctx, EDX = offset) ---------------------

unsafe fn e_4aeae0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let ctx = r.eax as usize;
        let mut ecx = rd(ctx).wrapping_add(r.edx);
        let edx = rd(ecx as usize);
        wd(ctx + 0x1DC, edx);
        wd(ctx + 0x1E0, 0);
        if edx != 0 {
            ecx = ecx.wrapping_add(4);
            wd(ctx + 0x1E0, ecx);
        }
        r.ecx = ecx;
        r.edx = edx;
    }
}

// --- 0x005C2650 act_packet_fuzz_objective ------------------------------------------------------------

unsafe fn e_5c2650(r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = rd(0x027B_3C60);
        if p == 0 {
            r.eax = 0;
            return;
        }
        r.eax = p;
        if rd(p as usize + 0x3C) != 1 {
            return;
        }
        let mut s = St::of(r);
        s.ecx = rd(p as usize + 0x90);
        s.eax = rd(s.ecx as usize);
        s.edx = rd(s.eax as usize + 4);
        let f = s.edx as usize;
        s.call(f, &[]);
        if s.eax != 5 {
            s.put_all(r);
            return;
        }
        s.call(0x005B_C410, &fwd(a));
        s.put_all(r);
    }
}

// --- 0x007F0B00 vint_draw_set_mask_mode ----------------------------------------------------------------

unsafe fn e_7f0b00(r: &mut Pushad, a: *const u32) {
    unsafe {
        let v = arg(a, 0);
        r.eax = v;
        if v == rd(0x0252_8B14) {
            return;
        }
        wd(0x0252_8B14, v);
        let mut s = St::of(r);
        s.call(0x007F_0950, &fwd(a));
        s.put_all(r);
    }
}

// --- 0x0093C660 ambient_spawn_timer_update ------------------------------------------------------------

unsafe fn e_93c660(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut i = 0u32;
        loop {
            if rb(0x0215_DA1C + i as usize) == 0 {
                wd(0x00EB_7A80, 0xFFFF_FFFF);
                r.eax = i;
                return;
            }
            i += 1;
            if i >= 6 {
                break;
            }
        }
        r.eax = 6;
        if (rd(0x00EB_7A80) as i32) >= 0 {
            return;
        }
        let mut s = St::of(r);
        s.call(0x0093_C610, &fwd(a));
        s.put_all(r);
    }
}

// --- 0x00AABC80 vehicle_flag_8f50_stamp_set (EAX = vehicle, EDX = value) --------------------------------

unsafe fn e_aabc80(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let v = r.eax as usize;
        let mut ecx = rd(v + 0x8F50);
        ecx = (ecx & 0xFFFF_FEBF) | 0x20;
        wd(v + 0x8F50, ecx);
        ecx = rd(0x00E8_43AC);
        wd(v + 0x8F78, ecx);
        if (ecx as i32) > 0x6B49_D200 {
            ecx = ecx.wrapping_add(0x94B6_2E00);
            wd(v + 0x8F78, ecx);
        }
        wd(v + 0x8F80, r.edx);
        r.ecx = ecx;
    }
}

// --- 0x004616F0 ai_order_k1e_enter ---------------------------------------------------------------------

unsafe fn e_4616f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi = arg(a, 0);
        let mut s = St::of(r);
        if esi != 0 {
            let ecx = rd(esi as usize + 0x44);
            if ecx != 0 {
                let eax = rd((((ecx & 0xFFF) << 4) as usize) + 0x0214_9C64);
                if eax != 0 && rd(eax as usize + 0x44) == ecx {
                    if rd(eax as usize + 0x4C) == 1 && rb(eax as usize + 0x3CC) == 1 {
                        r.eax = eax;
                        r.ecx = 1;
                        return;
                    }
                }
            }
        }
        s.esi = esi;
        s.eax = 0;
        s.ecx = esi;
        let f = r.esi;
        s.call(0x0043_9FA0, &im(r, a, &[0, f]));
        s.call(0x0043_75B0, &im(r, a, &[esi, 0x1E, 0xB8, 0, f]));
        let esi0 = r.esi;
        s.put_all(r);
        r.esi = esi0;
    }
}

// --- 0x005D2060 fuzz_crime_arsonist::enter_state3 (ECX = this) -----------------------------------------

unsafe fn e_5d2060(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        let this = r.ecx;
        let f0 = r.esi;
        s.call(0x005C_3020, &im(r, a, &[f0]));
        s.ecx = rd(this as usize + 0xF8);
        s.eax = rd(s.ecx as usize);
        s.edx = rd(s.eax as usize + 0x34);
        let f = s.edx as usize;
        s.call(f, &im(r, a, &[f0]));
        if s.eax != 0 {
            let p = s.eax.wrapping_add(0x8554);
            s.esi = p;
            s.ecx = 0;
            s.call(0x00B5_FB30, &im(r, a, &[p, 0, f0]));
            s.call(0x00B2_D020, &im(r, a, &[p, 1, f0]));
        }
        let esi0 = r.esi;
        s.put_all(r);
        r.esi = esi0;
    }
}

// --- 0x004DEDD0 weather_find ---------------------------------------------------------------------------

unsafe fn e_4dedd0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let name = arg(a, 0);
        let mut s = St::of(r);
        let mut i: u32 = 0;
        s.ebx = name;
        s.esi = 0;
        if (rd(0x0252_6DCC) as i32) > 0 {
            let mut off: u32 = 0;
            s.edi = 0;
            loop {
                s.esi = i;
                s.edi = off;
                s.eax = rd(0x0252_6DD0);
                s.ecx = rd((off.wrapping_add(s.eax)) as usize);
                let e = s.call(0x00C9_9030, &im(r, a, &[name, s.ecx, r.edi, r.esi, r.ebx]));
                if e == 0 {
                    s.eax = i.wrapping_mul(0xD8).wrapping_add(rd(0x0252_6DD0));
                    s.put(r);
                    return;
                }
                i = i.wrapping_add(1);
                off = off.wrapping_add(0xD8);
                if !((i as i32) < (rd(0x0252_6DCC) as i32)) {
                    break;
                }
            }
        }
        s.eax = 0;
        s.put(r);
    }
}

// --- 0x00404290 ai_order_k02_enter ---------------------------------------------------------------------

unsafe fn e_404290(r: &mut Pushad, a: *const u32) {
    unsafe {
        let ebx = arg(a, 0);
        let mut s = St::of(r);
        let fr = [r.edi, r.esi, r.ebx];
        s.ebx = ebx;
        s.edi = ebx;
        s.call(0x0040_3630, &im(r, a, &fr));
        s.eax = rd(ebx as usize + 0x44);
        let v = s.eax;
        s.esi = 0x19;
        s.call(0x0040_2160, &im(r, a, &[v, v, fr[0], fr[1], fr[2]]));
        let idx = rd(0x00EC_3ABC);
        s.eax = idx;
        s.ecx = rd((idx.wrapping_mul(4).wrapping_add(0x00EC_3AC0)) as usize);
        s.esi = rd(ebx as usize + 0x44);
        let n = idx.wrapping_add(1);
        wd(0x00EC_3ABC, n);
        if (n as i32) >= 0x800 {
            wd(0x00EC_3ABC, 0);
        }
        let hi = ((0x0603_538Bu64 * s.ecx as u64) >> 32) as u32;
        let mut e = s.ecx.wrapping_sub(hi);
        e >>= 1;
        e = e.wrapping_add(hi);
        e >>= 10;
        s.edx = e.wrapping_mul(0x7D1);
        s.eax = s.ecx.wrapping_sub(s.edx).wrapping_add(0x3E8);
        s.edi = 0;
        s.ecx = 0x19;
        let esi = s.esi;
        s.call(0x0040_1420, &im(r, a, &[esi, esi, 2, 0, fr[0], fr[1], fr[2]]));
        s.ecx = rd(ebx as usize + 0xAF0);
        s.call(0x0080_66C0, &im(r, a, &fr));
        if s.eax & 0xFF == 0 {
            s.call(0x0080_6910, &im(r, a, &[ebx, 0, 0, fr[0], fr[1], fr[2]]));
        }
        s.put(r);
    }
}

// --- 0x00428950 ai_order_k0c_enter ----------------------------------------------------------------------

unsafe fn e_428950(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi = arg(a, 0);
        let ebp0 = r.ebp;
        let mut s = St::of(r);
        s.ebp = r.esp.wrapping_sub(4);
        s.esi = esi;
        if esi != 0 {
            let h = rd(esi as usize + 0x44);
            s.ecx = h;
            if h != 0 {
                s.eax = rd((((h & 0xFFF) << 4) as usize) + 0x0214_9C64);
                if s.eax != 0 && rd(s.eax as usize + 0x44) == h && rd(s.eax as usize + 0x4C) == 1 && rb(s.eax as usize + 0x3CC) == 1 {
                    s.put(r);
                    r.ebp = ebp0;
                    return;
                }
            }
        }
        if rb(esi as usize + 0x470) & 0x10 == 0 {
            s.call(0x0099_3BD0, &[esi, 0]);
        }
        if rb(esi as usize + 0x140) & 4 == 0 {
            s.ecx = rd(esi as usize + 0x154);
            wd(esi as usize + 0x730, s.ecx);
            s.edx = rd(esi as usize + 0x158);
            wd(esi as usize + 0x734, s.edx);
            s.eax = rd(esi as usize + 0x15C);
            wd(esi as usize + 0x760, rd(esi as usize + 0x760) & 0xFFFF_FFFE);
            wb(esi as usize + 0x75C, rb(esi as usize + 0x75C) & 0xFE);
            wd(esi as usize + 0x738, s.eax);
            wd(esi as usize + 0x748, 0xFFFF_FFFF);
        }
        s.eax = 0;
        s.ecx = esi;
        s.call(0x0043_9FA0, &[1]);
        s.call(0x0043_75B0, &[esi, 0x0C, 0x75, 1]);
        s.put(r);
        r.ebp = ebp0;
    }
}

// --- 0x00603830 game_mode_diversion_leave ---------------------------------------------------------------

unsafe fn e_603830(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        let esi0 = r.esi;
        let p = rd(0x0252_7888);
        s.eax = p;
        if p != 0 {
            s.ecx = rd(p as usize + 8);
            s.eax = rd(s.ecx as usize);
            s.edx = rd(s.eax as usize + 8);
            let f = s.edx as usize;
            s.call(f, &im(r, a, &[esi0]));
            s.ecx = rd(0x0252_7888);
            s.esi = s.eax;
            s.eax = rd(s.ecx as usize);
            s.edx = rd(s.eax as usize + 4);
            let f = s.edx as usize;
            s.call(f, &im(r, a, &[esi0]));
            wd(0x0252_7888, 0);
            let e = s.esi;
            if e != 3 && e != 0x0C && e != 0x12 {
                s.eax = rd(0x0252_8C14);
                let mut go = true;
                if s.eax != 0 {
                    let p2 = s.eax;
                    s.call(0x0080_FCF0, &im(r, a, &[p2, esi0]));
                    go = s.eax & 0xFF != 0;
                }
                if go {
                    s.call(0x0055_F330, &im(r, a, &[0, 0, esi0]));
                }
            }
        }
        s.esi = esi0;
        s.call(0x0053_B050, &fwd(a));
        s.put_all(r);
    }
}

// --- 0x0060BFA0 firetruck_diversion_type_info::update (ECX = this) -----------------------------------------

unsafe fn e_60bfa0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        let esi = rd(0x0217_03D4);
        s.esi = esi;
        s.edx = r.ecx;
        let this = r.ecx as usize;
        let h = rd(esi as usize + 0xD74);
        s.ecx = h;
        let mut skip = false; // true: leave without touching the timer (60c01a)
        if h != 0 {
            s.eax = rd((((h & 0xFFF) << 4) as usize) + 0x0214_9C64);
            if s.eax != 0 && rd(s.eax as usize + 0x44) == h && rd(s.eax as usize + 0x4C) == 5 {
                let mut call = true;
                if rb(s.eax as usize + 0x84D8) == 1 {
                    s.ecx = rd(s.eax as usize + 0x6CE8);
                    if s.ecx == 0 || (s.ecx as i32) < 0 {
                        s.eax = 0;
                        call = false;
                    }
                }
                if call {
                    s.eax = rd(s.eax as usize + 0x8ABC);
                    s.call(0x0096_0D60, &[]);
                }
                if s.eax == esi {
                    skip = true;
                }
            }
        }
        if !skip {
            s.eax = rd(0x00E8_43AC).wrapping_add(0x1388);
            wd(this + 0x18, s.eax);
            if (s.eax as i32) > 0x6B49_D200 {
                s.eax = s.eax.wrapping_add(0x94B6_2E00);
                wd(this + 0x18, s.eax);
            }
        }
        s.put(r);
    }
}

// --- 0x006186A0 stunt_jumping_diversion_type::on_game_event_15 (ECX = this, ret 0x14) ----------------------

unsafe fn e_6186a0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        let this = r.ecx as usize;
        s.edx = r.ecx;
        if rd(this + 0x18) == 0xFFFF_FFFF {
            s.put(r);
            return;
        }
        s.eax = arg(a, 0);
        if s.eax == 0 {
            s.put(r);
            return;
        }
        if rb(s.eax as usize + 0x84D8) == 1 {
            s.ecx = rd(s.eax as usize + 0x6CE8);
            if (s.ecx as i32) <= 0 {
                s.put(r);
                return;
            }
        }
        s.eax = rd(s.eax as usize + 0x8ABC);
        s.call(0x0096_0D60, &[]);
        if s.eax == 0 || s.eax != rd(0x0217_03D4) {
            s.put(r);
            return;
        }
        s.eax = arg(a, 2);
        wd(this + 0x64, rd(this + 0x64).wrapping_add(s.eax));
        let cmp = rd(this + 0x1C) as i32;
        s.eax = rd(this + 0x64);
        if cmp <= 1 && (s.eax as i32) > 0x3C {
            s.eax = s.edx;
            s.call(0x0061_8770, &[]);
        }
        s.put(r);
    }
}

// --- virtual-call helpers -------------------------------------------------------------------------------
// `mov eax, [ecx]; mov edx, [eax + off]; call edx`: EAX = vtable, EDX = target.
#[inline(always)]
unsafe fn vc_edx(s: &mut St, obj: u32, off: u32, args: &[u32]) -> u32 {
    unsafe {
        s.ecx = obj;
        s.eax = rd(obj as usize);
        s.edx = rd(s.eax.wrapping_add(off) as usize);
        let f = s.edx as usize;
        s.call(f, args)
    }
}

// `mov edx, [ecx]; mov eax, [edx + off]; call eax`: EDX = vtable, EAX = target.
#[inline(always)]
unsafe fn vc_eax(s: &mut St, obj: u32, off: u32, args: &[u32]) -> u32 {
    unsafe {
        s.ecx = obj;
        s.edx = rd(obj as usize);
        s.eax = rd(s.edx.wrapping_add(off) as usize);
        let f = s.eax as usize;
        s.call(f, args)
    }
}

// --- 0x005AC070 security_level_instance::update (ECX = this) ----------------------------------------------

unsafe fn e_5ac070(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let (esi0, edi0) = (r.esi, r.edi);
        let mut s = St::of(r);
        s.esi = this;
        s.edi = 3;
        let fr = [edi0, esi0];
        s.call(0x0053_F850, &im(r, a, &fr));
        let state = rd(this as usize + 0x1C);
        s.eax = state;
        let t = this as usize;
        if state <= 4 {
            match state {
                0 => {
                    s.call(0x005A_B560, &im(r, a, &fr));
                    s.edi = 1;
                    s.eax = 1;
                }
                1 => {
                    s.call(0x005A_B3F0, &im(r, a, &fr));
                    s.edi = 1;
                    s.eax = 1;
                }
                2 => {
                    s.eax = this;
                    s.call(0x005A_B6A0, &im(r, a, &fr));
                    s.eax = s.edi;
                }
                3 => {
                    s.call(0x005A_B980, &im(r, a, &[this, edi0, esi0]));
                }
                _ => {
                    let p = this.wrapping_add(0x2644);
                    s.eax = p;
                    if (rd(t + 0x2644) as i32) < 0 {
                        s.edi = 0;
                    } else {
                        s.call(0x00BD_D230, &im(r, a, &fr));
                        if s.eax & 0xFF != 0 {
                            s.edi = 0;
                        }
                    }
                    s.eax = s.edi;
                }
            }
        } else {
            s.eax = s.edi;
        }
        wd(t + 0x2BF8, 0);
        s.put(r);
        r.esi = esi0;
        r.edi = edi0;
    }
}

// --- 0x005C7A10 fuzz_crime_purse_snatch::enter_state3 (ECX = this) ---------------------------------------

unsafe fn e_5c7a10(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let t = this as usize;
        let (esi0, ebx0, edi0) = (r.esi, r.ebx, r.edi);
        let mut s = St::of(r);
        s.esi = this;
        s.call(0x005C_3020, &im(r, a, &[esi0]));
        'body: {
            let o = rd(t + 0x104);
            s.ecx = o;
            if o == 0 {
                break 'body;
            }
            s.eax = rd(o as usize);
            s.edx = rd(s.eax as usize + 0x2C);
            let f = s.edx as usize;
            s.call(f, &im(r, a, &[esi0]));
            if s.eax & 0xFF == 0 {
                break 'body;
            }
            let o = rd(t + 0x104);
            vc_edx(&mut s, o, 0x34, &im(r, a, &[edi0, ebx0, esi0]));
            s.ebx = s.eax;
            let p = s.ebx.wrapping_add(0xF8);
            s.edi = p;
            s.call(0x0044_0A60, &im(r, a, &[p, 1, edi0, ebx0, esi0]));
            s.call(0x0044_0920, &im(r, a, &[p, 1, edi0, ebx0, esi0]));
            s.call(0x0044_06A0, &im(r, a, &[p, 1, edi0, ebx0, esi0]));
            let b = s.ebx as usize;
            s.ecx = rd(b + 0x14);
            s.eax = s.ebx.wrapping_add(0x14);
            wd(t + 0xF8, s.ecx);
            s.edx = rd(s.eax as usize + 4);
            let o2 = rd(t + 0x108);
            s.ecx = o2;
            wd(t + 0xFC, s.edx);
            s.eax = rd(s.eax as usize + 8);
            wd(t + 0x100, s.eax);
            s.ebx = ebx0;
            s.edi = edi0;
            if o2 == 0 {
                break 'body;
            }
            s.edx = rd(o2 as usize);
            s.eax = rd(s.edx as usize + 0x2C);
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[esi0]));
            if s.eax & 0xFF == 0 {
                break 'body;
            }
            let o = rd(t + 0x104);
            s.ecx = o;
            s.edx = rd(o as usize);
            s.eax = rd(s.edx as usize + 0x34);
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[esi0]));
            s.eax = rd(s.eax as usize + 0x44);
            let h = s.eax;
            let o = rd(t + 0x108);
            s.ecx = o;
            s.edx = rd(o as usize);
            s.eax = rd(s.edx as usize + 0x34);
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[h, 0, 0, 0, 0, esi0]));
            s.ecx = s.eax;
            s.call(0x0041_2750, &im(r, a, &[h, 0, 0, 0, 0, esi0]));
        }
        s.put(r);
        r.esi = esi0;
        r.ebx = ebx0;
        r.edi = edi0;
    }
}

// --- 0x005E49F0 button_mashing_hud_open ------------------------------------------------------------------

unsafe fn e_5e49f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0, ecx0) = (r.esi, r.edi, r.ecx);
        let fr = [edi0, esi0, ecx0];
        let mut s = St::of(r);
        s.ecx = 0x00DF_0724;
        s.call(0x00BD_C9B0, &im(r, a, &fr));
        let head = rd(0x0252_A1AC);
        s.edx = head;
        s.ecx = head;
        let mut found = false;
        if head != 0 {
            let mut c = head;
            loop {
                if rd(c as usize + 0x48) == s.eax {
                    found = true;
                    s.ecx = c;
                    break;
                }
                c = rd(c as usize);
                s.ecx = c;
                if c == head || c == 0 {
                    break;
                }
            }
        }
        let mut go = found;
        if !found {
            s.call(0x007F_3720, &im(r, a, &[0x00DF_0724, edi0, esi0, ecx0]));
            go = s.eax != 0;
        }
        if go {
            s.ecx = 0x00DF_0730;
            s.call(0x00B9_51E0, &im(r, a, &fr));
            if s.eax != 0 {
                s.call(0x005E_4C10, &im(r, a, &fr));
            } else {
                s.eax = 0x00DF_0730;
                s.call(0x00B9_4FE0, &im(r, a, &fr));
                let e = s.eax;
                s.edi = e;
                if e != 0 {
                    s.esi = 0x00E9_7D64;
                    s.eax = e.wrapping_add(0x14);
                    loop {
                        if rd(s.eax as usize) == 0 {
                            s.ecx = rd(s.esi as usize);
                            s.call(0x00B9_3A10, &im(r, a, &fr));
                        }
                        s.esi = s.esi.wrapping_add(4);
                        s.eax = s.eax.wrapping_add(0x10);
                        if !((s.esi as i32) < 0x00E9_7D74) {
                            break;
                        }
                    }
                    wd(e as usize + 0x214, 4);
                    s.call(0x005E_4C10, &im(r, a, &fr));
                }
            }
        }
        s.put(r);
        r.esi = esi0;
        r.edi = edi0;
        r.ecx = ecx0;
    }
}


/// 64 / 32 unsigned divide (a hardware `div`, so a zero divisor faults as the original does).
#[inline(always)]
unsafe fn div32(hi: u32, lo: u32, d: u32) -> (u32, u32) {
    let (q, rem): (u32, u32);
    unsafe {
        std::arch::asm!("div {d}", d = in(reg) d, inout("eax") lo => q, inout("edx") hi => rem, options(nomem, nostack));
    }
    (q, rem)
}

// --- 0x006852E0 cheat_repair_vehicle -------------------------------------------------------------------

unsafe fn e_6852e0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let fr = [esi0];
        let mut s = St::of(r);
        let esi = rd(0x0217_03D4);
        s.esi = esi;
        'body: {
            if esi == 0 {
                break 'body;
            }
            s.eax = rd(esi as usize + 0x44);
            if s.eax == 0 {
                break 'body;
            }
            s.ecx = rd(0x00EB_CE9C);
            if s.ecx != 0 && s.eax == rd(s.ecx as usize + 0x44) {
                s.eax = rd(0x00EB_CEA0);
                if s.eax != 7 && s.eax != 8 {
                    break 'body;
                }
            }
            s.eax = esi;
            s.call(0x009B_92E0, &im(r, a, &fr));
            if s.eax & 0xFF != 0 {
                break 'body;
            }
            let h = rd(esi as usize + 0xD74);
            s.esi = h;
            s.ecx = h;
            s.call(0x00AE_28F0, &im(r, a, &fr));
            s.eax = (s.eax & !0xFF) | (s.eax != 0) as u32;
            if s.eax & 0xFF == 0 || h == 0 {
                break 'body;
            }
            s.eax = (h & 0xFFF) << 4;
            s.edx = rd(s.eax as usize + 0x0214_9C64);
            if s.edx == 0 || rd(s.edx as usize + 0x44) != h || rd(s.edx as usize + 0x4C) != 5 {
                break 'body;
            }
            s.ecx = 0xFFFF_FFFF;
            s.call(0x00AA_AB60, &im(r, a, &[0x3F80_0000, 1, 1, esi0]));
        }
        s.put(r);
    }
}

// --- 0x0070F100 havok_game_filter::ctor (ESI = this) -------------------------------------------------------

unsafe fn e_70f100(r: &mut Pushad, a: *const u32) {
    unsafe {
        let t = r.esi as usize;
        *((t + 6) as *mut u16) = 1;
        wd(t + 8, 0x00E1_8D20);
        wd(t + 0xC, 0x00E1_8CA4);
        wd(t + 0x10, 0x00E1_8D14);
        wd(t + 0x14, 0x00E1_8D2C);
        wd(t + 0x20, 0);
        wd(t, 0x00E1_8CB8);
        wd(t + 8, 0x00E1_8CDC);
        wd(t + 0xC, 0x00E1_8CE8);
        wd(t + 0x10, 0x00E1_8CFC);
        wd(t + 0x14, 0x00E1_8D08);
        wd(t + 0x30, 0xD);
        wd(t + 0x34, 0x0168_D680);
        wd(t + 0x20, 4);
        wd(t + 0x38, 0xD);
        let mut s = St::of(r);
        s.eax = 0xD;
        s.call(0x00CA_0BC0, &im(r, a, &[0x0168_D680, 0xFF, 0x800]));
        s.eax = r.esi;
        s.put(r);
    }
}

// --- 0x008AFEE0 mp_bh_activity_hitman_info::ctor (ESI = this) ---------------------------------------------

unsafe fn e_8afee0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let t = r.esi as usize;
        let (ebx0, edi0) = (r.ebx, r.edi);
        wd(t + 0xC, 0x3F80_0000);
        wd(t + 4, 0);
        wd(t + 0x14, 0xFFFF_FFFF);
        wd(t + 0x18, 0x1D4C0);
        wd(t + 0x1C, 0x7530);
        wb(t + 0x20, 0);
        wd(t, 0x00E2_F514);
        let p = (t as u32).wrapping_add(0x48C);
        wd(t + 0x2C, 0);
        wd(t + 0x28, 0x10);
        wd(t + 0x24, (t as u32).wrapping_add(0x30));
        let mut s = St::of(r);
        s.ebx = 0;
        s.edi = p;
        s.eax = (t as u32).wrapping_add(0x30);
        s.call(0x0040_2450, &im(r, a, &[p, 4, 0x10, 0x0040_4A70, edi0, ebx0]));
        wd(t + 0x480, p);
        wd(t + 0x488, 0);
        wd(t + 0x484, 0x10);
        wd(t + 0x2C, 0);
        wd(t + 0x14, 0x22);
        s.eax = t as u32;
        s.put(r);
    }
}

// --- 0x00925500 object_register (EDI = object, one stack argument) ---------------------------------------------

unsafe fn e_925500(r: &mut Pushad, a: *const u32) {
    unsafe {
        let o = r.edi as usize;
        let (ebx0, esi0) = (r.ebx, r.esi);
        let mut s = St::of(r);
        s.ebx = 0;
        let head = rd(0x0252_95E8);
        s.eax = head;
        if head == 0 {
            wd(o + 4, o as u32);
            wd(o + 8, o as u32);
            wd(0x0252_95E8, o as u32);
        } else {
            s.ecx = rd(head as usize + 8);
            wd(o + 8, s.ecx);
            wd(o + 4, head);
            s.edx = rd(head as usize + 8);
            wd(s.edx as usize + 4, o as u32);
            wd(head as usize + 8, o as u32);
        }
        s.call(0x0092_4DB0, &im(r, a, &[o as u32, esi0, ebx0]));
        wd(o + 0x44, s.eax);
        let v = arg(a, 0);
        wd(o + 0x4C, v);
        wd(o + 0x80, 0);
        wb(o + 0x84, 0);
        wd(o + 0x80, rd(o + 0x80) | 0x20);
        let p = o + 0x5C;
        s.esi = p as u32;
        wd(o + 0x7C, 0xFFFF_FFFF);
        wd(p, 0);
        wd(p + 4, 0x3F80_0000);
        wd(p + 0xC, 4);
        let f = rf(0x00DD_0550);
        wd(p + 0x10, 4);
        wf(p + 0x18, f);
        s.eax = o as u32;
        wd(p + 8, 0);
        wd(p + 0x14, 0);
        wd(p + 0x1C, 0);
        s.call(0x0092_5310, &im(r, a, &[esi0, ebx0]));
        wd(o + 0x88, 0);
        wd(o + 0xAC, 0);
        wd(o + 0xB0, 0);
        wd(o + 0x98, 0xFFFF_FFFF);
        wd(o + 0x9C, 0xFFFF_FFFF);
        s.put(r);
    }
}

// --- 0x0070BCE0 vhashtable<>::insert_3 (ECX = this, ret 0xC) -----------------------------------------------

unsafe fn e_70bce0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let t = this as usize;
        let (ebx0, edi0, esi0) = (r.ebx, r.edi, r.esi);
        let ebx = rd(t + 4);
        if rd(t + 0xC) == ebx {
            r.eax &= !0xFF;
            return;
        }
        let mut s = St::of(r);
        s.edi = this;
        s.ebx = ebx;
        s.esi = arg(a, 0);
        s.call(0x0070_9B40, &im(r, a, &[esi0, edi0, ebx0]));
        let (_q, mut edx) = div32(0, s.eax, ebx);
        let mut eax = 0u32;
        let mut ecx = 0u32;
        let tab = rd(t + 8);
        loop {
            if ebx == 0 {
                break;
            }
            if *((tab.wrapping_add(edx.wrapping_mul(2))) as usize as *const u16) == 0xFFFF {
                break;
            }
            edx = edx.wrapping_add(1);
            eax = eax.wrapping_add(1);
            if edx == ebx {
                edx = 0;
            }
            ecx = ecx.wrapping_add(1);
            if ecx >= ebx {
                break;
            }
        }
        if eax >= rd(t + 0x10) {
            eax = eax.wrapping_add(1);
            wd(t + 0x10, eax);
        }
        eax = rd(t + 8);
        ecx = arg(a, 1);
        let w = *(ecx as usize as *const u16);
        ecx = (ecx & 0xFFFF_0000) | w as u32;
        *((eax.wrapping_add(edx.wrapping_mul(2))) as usize as *mut u16) = w;
        wd(t + 0xC, rd(t + 0xC).wrapping_add(1));
        r.eax = (eax & !0xFF) | 1;
        r.ecx = ecx;
        r.edx = edx;
        r.ebp = s.ebp;
    }
}

// --- 0x00887060 parked_vehicles_parse_abandon_despawn_info -------------------------------------------------------

unsafe fn e_887060(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, ebp0, esi0, edi0) = (r.ebx, r.ebp, r.esi, r.edi);
        let mut s = St::of(r);
        let ebx = arg(a, 1);
        let ebp = arg(a, 0);
        s.ebx = ebx;
        s.ebp = ebp;
        wd(ebx as usize + 0x50, 0);
        // the first child whose name compares equal, starting at the list head
        let mut esi: u32 = 0;
        if ebp != 0 {
            let mut n = rd(ebp as usize + 8);
            if n != 0 {
                loop {
                    s.esi = n;
                    s.eax = rd(n as usize);
                    let nm = s.eax;
                    s.call(0x00C9_9030, &im(r, a, &[nm, 0x00E2_C700, edi0, esi0, ebp0, ebx0]));
                    if s.eax == 0 {
                        esi = n;
                        break;
                    }
                    n = rd(n as usize + 4);
                    s.esi = n;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
        s.esi = esi;
        s.edi = esi;
        while esi != 0 {
            let cnt = rd(ebx as usize + 0x50);
            let slot = ebx.wrapping_add(cnt.wrapping_mul(8));
            s.esi = slot;
            s.eax = cnt.wrapping_add(1);
            wd(ebx as usize + 0x50, s.eax);
            let cur = s.edi;
            s.ecx = 0x00E2_C6F4;
            s.call(0x00B7_47D0, &im(r, a, &[cur, edi0, esi0, ebp0, ebx0]));
            s.esi = s.esi.wrapping_add(4);
            s.ecx = 0x00E0_9858;
            s.call(0x00B7_47D0, &im(r, a, &[cur, cur, edi0, esi0, ebp0, ebx0]));
            s.ecx = rd(s.esi as usize);
            s.ecx = s.ecx.wrapping_mul(0x3E8);
            wd(s.esi as usize, s.ecx);
            let mut n = cur;
            s.esi = n;
            let mut found = 0u32;
            if ebp != 0 && cur != 0 {
                loop {
                    n = rd(n as usize + 4);
                    s.esi = n;
                    if n == 0 {
                        break;
                    }
                    s.edx = rd(n as usize);
                    let nm = s.edx;
                    s.call(0x00C9_9030, &im(r, a, &[nm, 0x00E2_C700, edi0, esi0, ebp0, ebx0]));
                    if s.eax == 0 {
                        found = n;
                        break;
                    }
                }
            }
            esi = found;
            s.esi = found;
            s.edi = found;
        }
        s.put(r);
        r.ebp = ebp0;
    }
}


// --- 0x00A7DD50 streamman_read_nodes_free_list (EAX = list head) -------------------------------------------

unsafe fn e_a7dd50(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut eax = r.eax;
        let mut ecx = r.ecx;
        let mut edx = r.edx;
        let mut ebx = rd(0x0252_A020);
        let mut edi = eax;
        loop {
            if eax == 0 {
                break;
            }
            let n = eax as usize;
            edx = rd(n + 4);
            ecx = if edx != edi { edx } else { 0 };
            let mut unlink = true;
            if edi == eax {
                if edx == eax {
                    edi = 0;
                    unlink = false;
                } else {
                    edi = edx;
                }
            }
            if unlink {
                let esi = rd(n);
                if esi != 0 {
                    wd(esi as usize + 4, edx);
                }
                edx = rd(n + 4);
                if edx != 0 {
                    let esi = rd(n);
                    wd(edx as usize, esi);
                }
            }
            wd(n, 0);
            wd(n + 4, 0);
            wd(n + 0x10, 0);
            wd(n + 0x14, 0);
            if ebx == 0 {
                wd(0x0252_A024, rd(0x0252_A024).wrapping_add(1));
                wd(n + 4, eax);
                wd(n, eax);
                ebx = eax;
                eax = ecx;
            } else {
                edx = rd(ebx as usize);
                wd(0x0252_A024, rd(0x0252_A024).wrapping_add(1));
                wd(n, edx);
                wd(n + 4, ebx);
                edx = rd(ebx as usize);
                wd(edx as usize + 4, eax);
                wd(ebx as usize, eax);
                eax = ecx;
            }
        }
        wd(0x0252_A020, ebx);
        r.eax = eax;
        r.ecx = ecx;
        r.edx = edx;
    }
}

// --- 0x00A82AD0 chunk_extras_list_get (EAX = out, DX = id, one stack argument) ----------------------------------

unsafe fn e_a82ad0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let edx = r.edx;
        let dx = (edx & 0xFFFF) as u16;
        let out = r.eax;
        let dst = arg(a, 0);
        if dx == 0xFFFF {
            r.eax &= !0xFF;
            return;
        }
        let mut eax = 0u32;
        let mut ecx = 0x0222_95F4u32;
        loop {
            if *(ecx as usize as *const u16) == dx {
                break;
            }
            ecx = ecx.wrapping_add(0xD0);
            eax = eax.wrapping_add(1);
            if !((ecx as i32) < 0x0222_9C74) {
                let mut s = St::of(r);
                s.eax = eax;
                s.ecx = ecx;
                s.esi = out;
                s.edi = dst;
                s.call(0x00A8_2760, &im(r, a, &[dst, out, edx, edi0, esi0]));
                s.put(r);
                return;
            }
        }
        eax = eax.wrapping_mul(0xD0).wrapping_add(0x0222_9528);
        ecx = rd(eax as usize + 0xC8);
        wd(out as usize, ecx);
        if (ecx as i32) > 0 {
            let mut n = 0usize;
            while (n as u32) < ecx {
                wd(dst as usize + 4 * n, rd(eax as usize + 4 * n));
                n += 1;
            }
            ecx = 0;
        }
        r.eax = (eax & !0xFF) | 1;
        r.ecx = ecx;
    }
}

// --- 0x00A73850 async_io_wait_event (EDI = this, one stack argument, ret 4) ----------------------------------------

unsafe fn e_a73850(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let mut s = St::of(r);
        let t = r.edi as usize;
        let h = rd(t + 0x34);
        s.eax = h;
        if h == 0 {
            s.ecx = rd(0x0252_9E24);
            s.edx = rd(s.ecx as usize);
            wd(0x0252_9E28, s.edx);
        } else {
            s.eax = rd(h as usize);
            let f = rd(0x00DB_8100) as usize;
            let w = s.eax;
            s.call(f, &im(r, a, &[w, 0xFFFF_FFFF]));
        }
        let p = arg(a, 0).wrapping_add(8);
        s.esi = p;
        let f = rd(0x00DB_80CC) as usize;
        s.call(f, &im(r, a, &[p, esi0]));
        s.ecx = rd(s.edi as usize + 0x34);
        s.edx = rd(0x00EB_80C0);
        s.eax = rd(s.edx as usize + 4);
        let c = s.ecx.wrapping_sub(8);
        s.ecx = 0x00EB_80C0;
        let f = s.eax as usize;
        s.call(f, &im(r, a, &[c, esi0]));
        wd(s.edi as usize + 0x34, 0);
        let f = rd(0x00DB_80D0) as usize;
        s.call(f, &im(r, a, &[p, esi0]));
        s.put_all(r);
        r.esi = esi0;
    }
}


// --- 0x00B69B60 weapon_effect_alloc ------------------------------------------------------------------------

unsafe fn e_b69b60(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let mut s = St::of(r);
        let e = rd(0x022D_7B5C);
        s.esi = e;
        s.edi = 0;
        if e == 0 {
            s.eax = 0;
            s.put(r);
            return;
        }
        let n = e as usize;
        s.ecx = rd(n + 0x70);
        if s.ecx == e {
            wd(0x022D_7B5C, 0);
        } else {
            s.eax = rd(n + 0x74);
            wd(0x022D_7B5C, s.ecx);
            if s.eax != 0 {
                wd(s.eax as usize + 0x70, s.ecx);
            }
            s.eax = rd(n + 0x70);
            if s.eax != 0 {
                s.ecx = rd(n + 0x74);
                wd(s.eax as usize + 0x74, s.ecx);
            }
        }
        wd(n + 0x74, 0);
        wd(n + 0x70, 0);
        s.call(0x00CA_0BC0, &im(r, a, &[e, 0, 0x78, edi0, esi0]));
        s.eax = rd(0x022D_7B58);
        if s.eax == 0 {
            wd(n + 0x70, e);
            wd(n + 0x74, e);
            wd(0x022D_7B58, e);
        } else {
            let h = s.eax as usize;
            s.edx = rd(h + 0x74);
            wd(n + 0x74, s.edx);
            wd(n + 0x70, s.eax);
            s.ecx = rd(h + 0x74);
            wd(s.ecx as usize + 0x70, e);
            wd(h + 0x74, e);
        }
        s.eax = e;
        s.put(r);
    }
}

// --- 0x00B94290 vint_datagroup_add_item (EAX = this) -------------------------------------------------------

unsafe fn e_b94290(r: &mut Pushad, a: *const u32) {
    unsafe {
        let edi0 = r.edi;
        let this = r.eax;
        let t = this as usize;
        let mut s = St::of(r);
        s.edi = this;
        if rb(t + 0x1C) != 0 {
            s.eax = 0;
            s.put(r);
            return;
        }
        s.call(0x00B9_5120, &im(r, a, &[edi0]));
        if s.eax == 0 {
            s.put(r);
            return;
        }
        let n = s.eax as usize;
        s.ecx = rd(t + 0xC);
        if s.ecx == 0 {
            wd(n + 4, s.eax);
            wd(n, s.eax);
            wd(t + 0xC, s.eax);
        } else {
            s.ecx = rd(s.ecx as usize);
            wd(n, s.ecx);
            s.edx = rd(t + 0xC);
            wd(n + 4, s.edx);
            s.ecx = rd(t + 0xC);
            s.edx = rd(s.ecx as usize);
            wd(s.edx as usize + 4, s.eax);
            s.ecx = rd(t + 0xC);
            wd(s.ecx as usize, s.eax);
        }
        s.put(r);
    }
}

// --- 0x00B8DBB0 vint_callback_create (EDX = argument) -------------------------------------------------------

unsafe fn e_b8dbb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let edx0 = r.edx;
        let mut s = St::of(r);
        let head = rd(0x0252_A1C0);
        s.ecx = head;
        s.esi = head;
        s.edi = 0;
        if head == 0 {
            s.eax = 0;
            s.put(r);
            return;
        }
        let n = head as usize;
        s.eax = rd(rd(n as usize) as usize + 8);
        let f = s.eax as usize;
        s.call(f, &im(r, a, &[edx0, edi0, esi0]));
        if s.eax & 0xFF == 0 {
            s.eax = 0;
            s.put(r);
            return;
        }
        let mut relink = true;
        if rd(0x0252_A1C0) == head {
            s.eax = rd(n + 0x20);
            if s.eax == head {
                wd(0x0252_A1C0, 0);
                relink = false;
            } else {
                wd(0x0252_A1C0, s.eax);
            }
        }
        if relink {
            s.ecx = rd(n + 0x24);
            s.edx = rd(n + 0x20);
            wd(s.ecx as usize + 0x20, s.edx);
            s.eax = rd(n + 0x20);
            s.ecx = rd(n + 0x24);
            wd(s.eax as usize + 0x24, s.ecx);
        }
        s.eax = rd(0x0252_A1BC);
        wd(n + 0x24, 0);
        wd(n + 0x20, 0);
        if s.eax == 0 {
            wd(n + 0x20, head);
            wd(n + 0x24, head);
            wd(0x0252_A1BC, head);
        } else {
            let h = s.eax as usize;
            s.edx = rd(h + 0x24);
            wd(n + 0x20, s.eax);
            wd(n + 0x24, s.edx);
            s.ecx = rd(h + 0x24);
            wd(s.ecx as usize + 0x20, head);
            wd(h + 0x24, head);
        }
        s.eax = head;
        s.put(r);
    }
}

// --- 0x00B8AE10 vint_document_parse_object_list (ECX = this, ret 4) --------------------------------------------

unsafe fn e_b8ae10(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, esi0, edi0, ecx0) = (r.ebx, r.esi, r.edi, r.ecx);
        let fr = [edi0, esi0, ebx0, ecx0];
        let mut s = St::of(r);
        let ebx = arg(a, 0);
        s.ebx = ebx;
        s.edi = ecx0;
        'body: {
            if ebx == 0 || ecx0 == 0 {
                break 'body;
            }
            s.call(0x00BF_BEE0, &im(r, a, &fr));
            if s.eax == 0 {
                break 'body;
            }
            s.edi = 0x00E3_9A74;
            s.call(0x00BF_BEE0, &im(r, a, &fr));
            s.esi = s.eax;
            if s.esi == 0 {
                break 'body;
            }
            loop {
                let n = s.esi;
                s.call(0x00B9_84E0, &im(r, a, &[n, ebx, edi0, esi0, ebx0, ecx0]));
                if s.esi == 0 {
                    break 'body;
                }
                loop {
                    s.esi = rd(s.esi as usize + 4);
                    if s.esi == 0 {
                        break 'body;
                    }
                    s.eax = rd(s.esi as usize);
                    let nm = s.eax;
                    s.call(0x00C9_9030, &im(r, a, &[nm, 0x00E3_9A74, edi0, esi0, ebx0, ecx0]));
                    if s.eax == 0 {
                        break;
                    }
                }
            }
        }
        s.put(r);
        r.ecx = ecx0;
    }
}

// --- 0x00B952F0 vint_dataitem_notify (ESI = this) ---------------------------------------------------------------

unsafe fn e_b952f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let edi0 = r.edi;
        let mut s = St::of(r);
        let t = r.esi as usize;
        if rb(t + 0x229) == 1 {
            s.eax = rd(t + 0x21C);
            if s.eax != 0 {
                s.ecx = rd(t + 0x224);
                let e = s.eax;
                s.call(0x00B9_44C0, &im(r, a, &[e]));
            }
        }
        if rb(t + 0x228) == 1 {
            s.edi = rd(t + 0x220);
            let mut cur = s.edi;
            'walk: {
                if cur == 0 {
                    break 'walk;
                }
                loop {
                    let c = cur as usize;
                    s.edx = rd(c + 0xC);
                    if s.edx != 0 {
                        s.eax = rd(((s.edx & 0x3F) as usize) * 4 + 0x022F_2670);
                        s.ecx = s.eax;
                        if s.ecx != 0 {
                            loop {
                                if rd(s.ecx as usize + 0x18) == s.edx {
                                    s.eax = rd(s.ecx as usize);
                                    s.edx = rd(c + 8);
                                    s.eax = rd(s.eax as usize + 4);
                                    let (d, f) = (s.edx, s.eax as usize);
                                    s.call(f, &im(r, a, &[d, 2, t as u32, 0x22C, edi0]));
                                    break;
                                }
                                s.ecx = rd(s.ecx as usize + 8);
                                if s.ecx == s.eax || s.ecx == 0 {
                                    break;
                                }
                            }
                        }
                    }
                    s.eax = rd(c + 0x14);
                    if s.eax == rd(t + 0x220) {
                        break 'walk;
                    }
                    cur = s.eax;
                    s.edi = cur;
                    if cur == 0 {
                        break 'walk;
                    }
                }
            }
            s.eax = rd(t + 0x21C);
            if s.eax != 0 {
                s.ecx = rd(t + 0x224);
                let e = s.eax;
                s.edi = edi0;
                s.call(0x00B9_4420, &im(r, a, &[e]));
            }
        }
        wb(t + 0x228, 0);
        wb(t + 0x229, 0);
        s.put(r);
    }
}

// --- 0x00BA3190 vint_element_text::set_text_tag_crc (ECX = this, ret 8) ------------------------------------------

unsafe fn e_ba3190(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let this = r.ecx as usize;
        let mut s = St::of(r);
        s.esi = this as u32;
        s.ecx = arg(a, 0);
        s.call(0x0063_4BA0, &im(r, a, &[esi0]));
        wd(this + 0x98, s.eax);
        s.eax = rd(0x0252_A1E4);
        if s.eax != 0 {
            s.eax = rd(s.eax as usize + 4);
            if s.eax != 0 {
                s.ecx = rd(this + 0x9C);
                let (c, f) = (s.ecx, s.eax as usize);
                s.call(f, &im(r, a, &[c, esi0]));
            }
        }
        wb(this + 0xF0, rb(this + 0xF0) | 2);
        wd(this + 0x9C, 0);
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
    }
}

// --- 0x00BA4CD0 vint_element_clip_register_type ------------------------------------------------------------------

unsafe fn e_ba4cd0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let mut s = St::of(r);
        s.eax = rd(0x00EB_DF6C);
        s.ecx = rd(s.eax as usize);
        s.esi = 0x031B_F160;
        wd(0x031B_F168, s.ecx);
        wd(0x031B_F160, 0x00EB_E808);
        wd(0x031B_F164, 2);
        s.call(0x00B9_82E0, &im(r, a, &[esi0]));
        s.ecx = rd(0x0252_A270);
        s.edx = rd(0x00EB_DF6C);
        s.esi = rd(s.edx as usize);
        if s.ecx >= 0x20 {
            s.edx = 0xFFFF_FFFF;
        } else {
            s.eax = s.ecx << 4;
            s.edx = s.ecx;
            s.ecx = s.ecx.wrapping_add(1);
            let o = s.eax as usize;
            wd(o + 0x022F_2370, 0x00E1_7D60);
            wd(o + 0x022F_2374, s.esi);
            wd(o + 0x022F_2378, 0x031B_F160);
            wd(0x0252_A270, s.ecx);
        }
        wd(0x00E8_6C74, s.edx);
        s.call(0x00BA_5120, &im(r, a, &[esi0]));
        s.eax = rd(0x00E8_6C74);
        if s.eax < rd(0x0252_A270) {
            s.eax <<= 4;
            wd(s.eax as usize + 0x022F_237C, 0x00EB_E854);
        }
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
    }
}

// --- 0x00C0A350 packfile_hash_insert (one stack argument) ------------------------------------------------------------

unsafe fn e_c0a350(r: &mut Pushad, a: *const u32) {
    unsafe {
        let ebx = arg(a, 0);
        let mut ecx = rd(0x0234_894C);
        let eax0 = rd(ebx as usize);
        let esi = ecx.wrapping_sub(0x67);
        let (_q, edx) = div32(0, eax0, esi);
        let mut eax = 0u32;
        ecx = ecx.wrapping_sub(edx);
        if (ecx as i32) > 0 {
            let ebp = rd(0x0234_8948);
            let mut edi = rd(0x0252_A568);
            let mut p = ebp.wrapping_add(edx.wrapping_mul(4));
            loop {
                if rd(p as usize) == 0 {
                    eax = eax.wrapping_add(edx);
                    wd(0x0252_A568, edi);
                    wd(ebp.wrapping_add(eax.wrapping_mul(4)) as usize, ebx);
                    break;
                }
                if (eax as i32) > (edi as i32) {
                    edi = eax;
                }
                eax = eax.wrapping_add(1);
                p = p.wrapping_add(4);
                if !((eax as i32) < (ecx as i32)) {
                    wd(0x0252_A568, edi);
                    break;
                }
            }
        }
        r.eax = eax;
        r.ecx = ecx;
        r.edx = edx;
    }
}

// --- 0x00CE0E00 rig_find_bone_by_name (ret 8) ------------------------------------------------------------------------

unsafe fn e_ce0e00(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, ebp0, esi0, edi0) = (r.ebx, r.ebp, r.esi, r.edi);
        let mut s = St::of(r);
        let ebp = arg(a, 0);
        s.ebp = ebp;
        s.eax = rd(ebp as usize + 0x18);
        s.ecx = rd(s.eax as usize + 0x24);
        s.edi = rd(s.eax as usize + 0x30);
        s.esi = 0;
        wd(a as usize, s.ecx);
        if (s.edi as i32) > 0 {
            s.ebx = 0;
            loop {
                s.edx = rd(ebp as usize + 0x18);
                s.eax = rd(s.edx as usize + 0x3C);
                s.ecx = rd(s.eax.wrapping_add(s.ebx) as usize);
                s.edx = arg(a, 1);
                let (nm, c) = (s.edx, s.ecx);
                s.call(0x00C9_9030, &im(r, a, &[nm, c, edi0, esi0, ebp0, ebx0]));
                if s.eax == 0 {
                    s.eax = rd(a as usize).wrapping_add(s.esi);
                    s.put(r);
                    r.ebp = ebp0;
                    return;
                }
                s.esi = s.esi.wrapping_add(1);
                s.ebx = s.ebx.wrapping_add(0x3C);
                if !((s.esi as i32) < (s.edi as i32)) {
                    break;
                }
            }
        }
        s.eax = 0xFFFF_FFFF;
        s.put(r);
        r.ebp = ebp0;
    }
}


// --- 0x00A66EF0 script_object::dtor_5 (EAX = this) ------------------------------------------------------------------

unsafe fn e_a66ef0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let t = r.eax as usize;
        wd(t, 0x00E3_E3BC);
        let g = rd(0x033D_6308);
        wd(t + 0xC, g);
        let g2 = rd(0x033D_6308);
        wd(t + 0xC, g2);
        wd(t + 0x10, 0);
        wd(t + 8, 0);
        wb(t + 0x14, 0);
        wb(t + 0x15, 0xFF);
        wd(t + 4, 0x00A6_2940);
        wd(t, 0x00E4_380C);
        wd(t + 0x3C, 0);
        wd(t + 0x40, 0);
        wd(t + 0x34, 0);
        wd(t + 0x38, 0);
        wd(t + 0x30, 0);
        wd(t + 0x24, 0);
        wd(t + 0x28, 0);
        wd(t + 0x2C, 0);
        wd(t + 0x18, 0);
        wd(t + 0x1C, 0);
        wd(t + 0x20, 0);
        r.ecx = 0;
        r.edx = g2;
    }
}

// --- 0x00A22740 script_object::dtor (ECX = this) ---------------------------------------------------------------------

unsafe fn e_a22740(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let t = r.ecx as usize;
        wd(t, 0x00E3_E3BC);
        let g = rd(0x033D_6308);
        wd(t + 0xC, g);
        let g2 = rd(0x033D_6308);
        wd(t + 0xC, g2);
        wd(t + 0x48, 0);
        wd(t + 0x4C, 0);
        wb(t + 0x15, 0xFF);
        wd(t + 0x58, 0);
        wd(t + 4, 0x00A6_2940);
        wd(t + 0x5C, 0);
        wd(t, 0x00E3_E3A8);
        wd(t + 0x50, 0);
        wd(t + 0x54, 0);
        wd(t + 0x10, 0);
        wd(t + 8, 0);
        wb(t + 0x14, 0);
        wd(t + 0x60, 0);
        wd(t + 0x64, 0);
        wd(t + 0x68, 0);
        wd(t + 0x6C, 0);
        wd(t + 0x70, 0);
        wd(t + 0x74, 0);
        *((t + 0x78) as *mut u16) = 0;
        wd(t + 0x18, 0);
        wd(t + 0x1C, 0);
        wd(t + 0x20, 0);
        wd(t + 0x24, 0x3F80_0000);
        wd(t + 0x34, 0x3F80_0000);
        wd(t + 0x44, 0x3F80_0000);
        wd(t + 0x28, 0);
        wd(t + 0x2C, 0);
        wd(t + 0x30, 0);
        wd(t + 0x38, 0);
        wd(t + 0x3C, 0);
        wd(t + 0x40, 0);
        r.eax = t as u32;
        r.ecx = 0;
        r.edx = g2;
    }
}

// --- 0x00957000 current_spawn_group_alloc_member (one stack argument) -----------------------------------------------------

unsafe fn e_957000(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi = rd(0x0215_E5BC);
        if esi == 0 {
            r.eax = 0;
            return;
        }
        let mut edx = 0u32;
        let mut eax = esi.wrapping_add(0x5C);
        loop {
            if rb(eax as usize) == 0 {
                break;
            }
            edx = edx.wrapping_add(1);
            eax = eax.wrapping_add(0x30);
            if !((edx as i32) < 8) {
                r.eax = 0;
                r.edx = edx;
                return;
            }
        }
        let ecx = edx.wrapping_mul(3).wrapping_shl(4).wrapping_add(esi);
        let c = ecx as usize;
        wb(c + 0x5C, 1);
        let v = arg(a, 0);
        let edi = rd(v as usize + 0x44);
        let e2 = ecx.wrapping_add(0x5C);
        let slot = edx.wrapping_mul(3).wrapping_add(6).wrapping_shl(4);
        wd(c + 0x88, 0);
        wd(slot.wrapping_add(esi) as usize, edi);
        wd(c + 0x68, 0);
        wd(c + 0x6C, 0);
        wd(c + 0x70, 0);
        wd(c + 0x74, 0);
        wd(c + 0x78, 0);
        wd(c + 0x7C, 0);
        wd(c + 0x80, 0);
        wd(c + 0x84, 0);
        wd(c + 0x64, 0xFFFF_FFFF);
        r.eax = e2;
        r.ecx = ecx;
        r.edx = 0;
    }
}

// --- 0x0079A580 hud_diversion_respect_message (ECX = value, one stack argument, ret 4) -------------------------------------

unsafe fn e_79a580(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, esi0, edi0) = (r.ebx, r.esi, r.edi);
        if (r.ecx as i32) <= -1 {
            r.eax &= !0xFF;
            return;
        }
        let ebx = arg(a, 0);
        let t = ebx as usize;
        let v = rd(t + 0x7C);
        if v != 0 {
            if v != 1 {
                r.eax = v.wrapping_sub(1) & !0xFF;
                return;
            }
            wd(t + 0x80, 0);
        }
        let mut s = St::of(r);
        s.ebx = ebx;
        s.eax = 0;
        s.esi = ebx.wrapping_add(0x3C);
        s.edi = 0x19;
        let ecx = r.ecx;
        s.call(0x00C1_4B50, &im(r, a, &[0x00E1_E5CC, ecx, edi0, esi0, ebx0]));
        s.edi = 0x00E1_E5D0;
        s.esi = 0x00E1_E5F8;
        s.call(0x007F_49E0, &im(r, a, &[edi0, esi0, ebx0]));
        wd(t + 0xA0, 0);
        wd(t + 0x70, s.eax);
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
    }
}

// --- 0x00845790 item_multi_registration::on_ownership_taken (ret 0xC) -----------------------------------------------------------

unsafe fn e_845790(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let h = arg(a, 0);
        r.eax = h;
        if h == 0 {
            return;
        }
        let ecx = (h & 0xFFF) << 4;
        r.ecx = ecx;
        let esi = rd(ecx as usize + 0x0214_9C64);
        if esi == 0 || rd(esi as usize + 0x44) != h || rd(esi as usize + 0x4C) != 2 {
            return;
        }
        let n = esi as usize;
        let eax = rd(n + 0x110);
        let edi = arg(a, 1);
        wd(eax as usize + 4, 0);
        wb(eax as usize, 0);
        r.eax = eax;
        if edi == 0 {
            return;
        }
        let mut s = St::of(r);
        s.esi = esi;
        s.edi = edi;
        s.eax = eax;
        s.ecx = rd(n + 0x7C);
        s.call(0x006F_C9E0, &im(r, a, &[0x0252_A6D0, edi0, esi0]));
        s.ecx = arg(a, 2);
        s.eax = rd(s.ecx as usize) & 0xFFFF;
        s.edx = s.eax;
        let nz = (rb(s.edx.wrapping_add(edi) as usize) != 0) as u32;
        s.edx = (s.edx & !0xFF) | nz;
        s.eax = s.eax.wrapping_add(1);
        *(s.ecx as usize as *mut u16) = s.eax as u16;
        s.eax = rd(n + 0x80) >> 4;
        s.eax = (s.eax & !0xFF) | (s.eax & 1);
        let dl = (s.edx & 0xFF) as u8;
        wb(a as usize, dl);
        if dl != (s.eax & 0xFF) as u8 {
            s.ecx = rd(a as usize);
            let c = s.ecx;
            s.ecx = esi;
            s.call(0x0092_6860, &im(r, a, &[c, 1, 0, edi0, esi0]));
        }
        s.put(r);
    }
}


// --- 0x00BF3E10 event_trigger::begin (ECX = this) -----------------------------------------------------------------

unsafe fn e_bf3e10(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let mut s = St::of(r);
        let t = r.ecx as usize;
        s.esi = t as u32;
        wd(t + 0x28, rd(t + 0x28).wrapping_add(1));
        let p = (t as u32).wrapping_add(0xC);
        s.edi = p;
        let f = rd(0x00DB_80CC) as usize;
        s.call(f, &im(r, a, &[p, edi0, esi0]));
        wd(t + 4, rd(t + 4).wrapping_add(1));
        let f = rd(0x00DB_80D0) as usize;
        s.call(f, &im(r, a, &[p, edi0, esi0]));
        s.put(r);
    }
}

// --- 0x00B97100 vint_writer_write_string (ESI = this, EDX = string) ----------------------------------------------

unsafe fn e_b97100(r: &mut Pushad, a: *const u32) {
    unsafe {
        let edi0 = r.edi;
        let t = r.esi as usize;
        let str_p = r.edx;
        let mut p = str_p;
        loop {
            let c = rb(p as usize);
            p = p.wrapping_add(1);
            if c == 0 {
                break;
            }
        }
        let len = p.wrapping_sub(str_p.wrapping_add(1));
        let buf = rd(t);
        let off = rd(t + 0xC);
        wd(buf.wrapping_add(off) as usize, len);
        wd(t + 0xC, off.wrapping_add(4));
        let ecx = rd(t + 0xC);
        let edi = len.wrapping_add(1);
        let dst = rd(t).wrapping_add(ecx);
        let mut s = St::of(r);
        s.eax = len;
        s.ecx = ecx;
        s.edx = dst;
        s.edi = edi;
        s.call(0x00CA_0850, &im(r, a, &[dst, str_p, edi, edi0]));
        wd(t + 0xC, rd(t + 0xC).wrapping_add(s.edi));
        s.eax = t as u32;
        s.put(r);
    }
}

// --- 0x00B979F0 vint_object_attach_child (ECX = child, ESI = parent) -----------------------------------------------

unsafe fn e_b979f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let c = r.ecx as usize;
        if c == 0 {
            r.eax &= !0xFF;
            return;
        }
        let mut s = St::of(r);
        if rd(c + 0x24) != 0 {
            s.call(0x00B9_7A20, &im(r, a, &[]));
        }
        let (c, p) = (s.ecx as usize, s.esi as usize);
        s.eax = rd(p + 0x1C);
        wd(c + 0x20, s.eax);
        wd(c + 0x24, p as u32);
        s.edx = rd(p + 0x30);
        wd(p + 0x1C, c as u32);
        let d = s.edx;
        s.call(0x00B9_79B0, &im(r, a, &[d]));
        s.eax = (s.eax & !0xFF) | 1;
        s.put_all(r);
    }
}

// --- 0x00D097A0 rl_lod_mesh_inst::select_shadow_lod_entry (ECX = this) -----------------------------------------------

unsafe fn e_d097a0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let t = r.ecx as usize;
        let mut s = St::of(r);
        s.esi = t as u32;
        let o = rd(t + 0x70);
        s.eax = o;
        if rd(o as usize + 0x24) == 0 {
            s.eax = 0;
            s.put(r);
            return;
        }
        s.edx = rd(t);
        s.eax = rd(s.edx as usize + 0x34);
        let f = s.eax as usize;
        s.call(f, &im(r, a, &[esi0]));
        s.ecx = rd(t + 0x70);
        s.eax = (s.eax << 4).wrapping_add(rd(s.ecx as usize + 0x24));
        s.put(r);
    }
}

// --- 0x00CF6040 rl_container_inst::render (ECX = this, ret 8) ----------------------------------------------------------

unsafe fn e_cf6040(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, esi0, ebp0, edi0) = (r.ebx, r.esi, r.ebp, r.edi);
        let t = r.ecx;
        let mut s = St::of(r);
        s.ebx = t;
        s.esi = 0;
        if rd(t as usize + 8) != 0 {
            s.ebp = arg(a, 1);
            s.edi = t.wrapping_add(0xC);
            loop {
                s.ecx = rd(s.edi as usize);
                s.eax = rd(s.ecx as usize);
                s.edx = arg(a, 0);
                s.eax = rd(s.eax as usize + 8);
                let (d, bp, f) = (s.edx, s.ebp, s.eax as usize);
                s.call(f, &im(r, a, &[d, bp, edi0, ebp0, esi0, ebx0]));
                s.esi = s.esi.wrapping_add(1);
                s.edi = s.edi.wrapping_add(4);
                if s.esi >= rd(t as usize + 8) {
                    break;
                }
            }
        }
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
        r.ebp = ebp0;
    }
}

// --- 0x009A17B0 human_player_move_input_active (EDX = player) -------------------------------------------------------------

unsafe fn e_9a17b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let p = r.edx as usize;
        let mut s = St::of(r);
        'no: {
            if rb(0x00E8_843F) == 0 || rb(0x0252_9715) != 0 {
                break 'no;
            }
            if rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) != 0 {
                break 'no;
            }
            if rb(0x0252_7D15) != 0 {
                break 'no;
            }
            s.ecx = rd(p + 0x44);
            s.call(0x0092_65C0, &im(r, a, &[]));
            if s.eax & 0xFF == 0 {
                break 'no;
            }
            if rb(p + 0xC0) & 0x40 != 0 {
                break 'no;
            }
            let f = rf(p + 0x1024);
            let fr = [esi0];
            // `fnstsw ax` after the compare: the condition codes C0 / C2 / C3 over the live TOP field
            let sw: u32;
            std::arch::asm!("fnstsw ax", "movzx eax, ax", out("eax") sw, options(nomem, nostack));
            let cc: u32 = if f.is_nan() {
                0x4500
            } else if 0.0f64 < f as f64 {
                0x0100
            } else if 0.0f64 == f as f64 {
                0x4000
            } else {
                0
            };
            s.eax = (s.eax & 0xFFFF_0000) | ((sw & !0x4700) | cc);
            if !(0.0f64 == f as f64) {
                break 'no;
            }
            if rd(p + 0x1020) != 0xFFFF_FFFF {
                break 'no;
            }
            let e = rd(p + 0x5E0);
            s.eax = e;
            let esi = rd(e as usize + 0x164);
            s.esi = esi;
            s.ecx = 0x800;
            s.eax = esi;
            s.call(0x006E_F750, &im(r, a, &fr));
            if s.eax & 0xFF != 0 {
                s.esi = esi0;
                break 'no;
            }
            s.eax = p as u32;
            s.call(0x0092_6650, &im(r, a, &fr));
            if s.eax & 0xFF != 0 {
                s.ecx = 0x0020_0000;
                s.eax = esi;
                s.call(0x006E_F750, &im(r, a, &fr));
                if s.eax & 0xFF != 0 {
                    break 'no;
                }
            }
            let m = rb(p + 0x591);
            s.eax = (s.eax & !0xFF) | m as u32;
            if (m == 2 || m == 3) && rb(p + 0x474) != 2 {
                break 'no;
            }
            s.eax = (s.eax & !0xFF) | 1;
            s.put(r);
            return;
        }
        s.eax &= !0xFF;
        s.put(r);
    }
}

// --- 0x006F6310 GeneralCollisionListener::scalar_deleting_dtor (ECX = this, ret 4) ---------------------------------------------

unsafe fn e_6f6310(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let flag = arg(a, 0) & 1;
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        wd(this as usize, 0x00E1_8AD4);
        if flag != 0 {
            let b = rb(0x0252_7BF7);
            s.eax = (s.eax & !0xFF) | b as u32;
            if b != 1 {
                let f = rd(0x00DB_80CC) as usize;
                s.call(f, &im(r, a, &[0x0347_ADE4, esi0]));
                s.eax = this;
                s.call(0x00CD_E870, &im(r, a, &[esi0]));
                let f = rd(0x00DB_80D0) as usize;
                s.call(f, &im(r, a, &[0x0347_ADE4, esi0]));
            }
        }
        s.eax = this;
        s.put(r);
    }
}

// --- 0x005F16D0 ambulance_diversion_type_info::get_title -------------------------------------------------------------------

unsafe fn e_5f16d0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let mut s = St::of(r);
        s.edi = 0x00DF_12B0;
        s.esi = 0x00DF_12C4;
        s.call(0x007F_49E0, &im(r, a, &[edi0, esi0]));
        s.put_all(r);
        r.esi = esi0;
        r.edi = edi0;
    }
}

// --- 0x00645650 act_packet_sub_59 (one stack argument) ---------------------------------------------------------------------

unsafe fn e_645650(r: &mut Pushad, a: *const u32) {
    unsafe {
        let edi0 = r.edi;
        let mut s = St::of(r);
        s.eax = rd(0x0252_7888);
        if s.eax == 0 {
            s.put(r);
            return;
        }
        s.ecx = rd(s.eax as usize + 8);
        s.eax = rd(s.ecx as usize);
        s.edx = rd(s.eax as usize + 8);
        let f = s.edx as usize;
        s.call(f, &im(r, a, &[]));
        if s.eax != 0x19 {
            s.put(r);
            return;
        }
        s.eax = rd(0x0252_7888);
        if s.eax == 0 {
            s.put(r);
            return;
        }
        let o = s.eax as usize;
        let x = rd(o + 0x2C);
        let zf;
        if rd(o + 0x10) == 1 {
            s.ecx = if (x as i32) < 0 { (x & 1).wrapping_neg() } else { x & 1 };
            zf = x & 1 == 0;
        } else {
            let rem = if (x as i32) < 0 { (x & 1).wrapping_neg() } else { x & 1 };
            s.edx = rem;
            zf = rem == 1;
        }
        s.ecx = (s.ecx & !0xFF) | zf as u32;
        if zf {
            s.put(r);
            return;
        }
        s.ecx = arg(a, 0);
        s.edx = rd(s.ecx as usize);
        let nv = rd(o + 0x78).wrapping_add(s.edx);
        wd(o + 0x78, nv);
        s.ecx = rd(o + 0x78);
        s.edi = rd(o + 0x908);
        let c = s.ecx;
        s.call(0x0078_D440, &im(r, a, &[c, edi0]));
        s.put(r);
    }
}

// --- 0x00667950 act_packet_sub_43 (one stack argument) ----------------------------------------------------------------------

unsafe fn e_667950(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, ebx0) = (r.esi, r.ebx);
        let p = arg(a, 0) as usize;
        let mut s = St::of(r);
        s.ecx = (r.ecx & !0xFF) | rb(p + 1) as u32;
        s.ebx = (r.ebx & !0xFF) | rb(p) as u32;
        s.esi = rd(p + 2);
        s.eax = rd(0x027B_3C60);
        wb(a as usize, (s.ecx & 0xFF) as u8);
        'body: {
            if s.eax == 0 {
                break 'body;
            }
            if rd(s.eax as usize + 0x3C) != 1 {
                break 'body;
            }
            s.eax = rd(s.eax as usize + 0x90);
            s.edx = rd(s.eax as usize);
            s.ecx = s.eax;
            s.eax = rd(s.edx as usize + 4);
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[esi0, ebx0]));
            if s.eax != 0xA {
                break 'body;
            }
            let nzc = (rb(a as usize) != 0) as u32;
            s.ecx = (s.ecx & !0xFF) | nzc;
            let nzd = ((s.ebx & 0xFF) != 0) as u32;
            s.edx = (s.edx & !0xFF) | nzd;
            let (c, d, e) = (s.ecx, s.edx, s.esi);
            s.call(0x0065_E910, &im(r, a, &[d, c, e, esi0, ebx0]));
        }
        s.put(r);
    }
}

// --- 0x0066DEE0 stop_complication::load_from_table (ECX = this, ret 4) ----------------------------------------------------------

unsafe fn e_66dee0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (ebx0, esi0, edi0) = (r.ebx, r.esi, r.edi);
        let ebx = arg(a, 0);
        let mut s = St::of(r);
        s.ebx = ebx;
        s.edi = r.ecx;
        s.call(0x0066_DB90, &im(r, a, &[ebx, edi0, esi0, ebx0]));
        s.esi = s.edi.wrapping_add(0x24);
        s.ecx = 0x00DF_DC6C;
        s.call(0x00B7_47D0, &im(r, a, &[ebx, edi0, esi0, ebx0]));
        let v = rd(s.esi as usize).wrapping_mul(0x3E8);
        s.eax = v;
        wb(s.edi as usize + 0xC, 0);
        wd(s.esi as usize, v);
        s.put(r);
    }
}


/// A model of the stack for bodies that push several argument words without cleaning up between
/// calls: `img` gives the first eight words a callee sees.
struct Stk {
    w: [u32; 40],
    n: usize,
}

impl Stk {
    fn new() -> Stk {
        Stk { w: [0; 40], n: 0 }
    }

    fn push(&mut self, v: u32) {
        self.w[self.n] = v;
        self.n += 1;
    }

    fn pop(&mut self, k: usize) {
        self.n -= k;
    }

    unsafe fn img(&self, r: &Pushad, a: *const u32) -> [u32; 16] {
        unsafe {
            let mut o = [0u32; 16];
            for (i, slot) in o.iter_mut().enumerate() {
                *slot = if i < self.n {
                    self.w[self.n - 1 - i]
                } else if i == self.n {
                    rd(r.esp as usize)
                } else {
                    rd(a as usize + 4 * (i - self.n - 1))
                };
            }
            o
        }
    }
}

// --- 0x004A0FB0 camera_blend_state_reset ------------------------------------------------------------------------

unsafe fn e_4a0fb0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let ecx = rd(0x0252_A6D4);
        wd(0x0252_6C48, 0);
        let eax = rd(0x0252_A6D0);
        let edx = rd(0x0252_A6D8);
        wd(0x025F_68D8, ecx);
        wd(0x025F_68F0, ecx);
        wd(0x025F_689C, ecx);
        for i in 0..9usize {
            wd(0x025F_68A4 + 4 * i, rd(0x00EC_5AE4 + 4 * i));
        }
        wd(0x025F_68D4, eax);
        wd(0x025F_68DC, edx);
        wd(0x025F_68EC, eax);
        wd(0x025F_68F4, edx);
        wd(0x025F_6898, eax);
        wd(0x025F_68A0, edx);
        r.eax = eax;
        r.ecx = 0;
        r.edx = edx;
    }
}

// --- 0x0061D360 stunt_oncoming_traffic_diversion_type::init (ECX = this) -------------------------------------------

unsafe fn e_61d360(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let t = r.ecx as usize;
        let mut s = St::of(r);
        s.esi = t as u32;
        s.call(0x0060_2A50, &im(r, a, &[esi0]));
        s.eax &= !0xFF;
        wb(t + 0x1C, 0);
        wd(t + 0xEC, 0xFFFF_FFFF);
        wb(t + 0xF8, 0);
        s.put(r);
    }
}

// --- 0x00495AC0 camera_free_xtbl_load ----------------------------------------------------------------------------

unsafe fn e_495ac0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi0 = r.esi;
        let mut s = St::of(r);
        s.ecx = 0;
        s.eax = 0x00DD_9E3C;
        s.call(0x00B7_43F0, &im(r, a, &[esi0]));
        let mut esi = 0u32;
        if s.eax != 0 {
            let mut n = rd(s.eax as usize + 8);
            s.esi = n;
            if n != 0 {
                loop {
                    s.eax = rd(n as usize);
                    let nm = s.eax;
                    s.call(0x00C9_9030, &im(r, a, &[nm, 0x00DD_980C, esi0]));
                    if s.eax == 0 {
                        esi = n;
                        break;
                    }
                    n = rd(n as usize + 4);
                    s.esi = n;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
        s.esi = esi;
        s.call(0x0049_5B20, &im(r, a, &[esi, esi0]));
        s.ecx = rd(0x00E8_7138);
        s.edx = rd(s.ecx as usize);
        s.eax = rd(0x00E8_7134);
        s.edx = rd(s.edx as usize + 0x34);
        let (e, f) = (s.eax, s.edx as usize);
        s.call(f, &im(r, a, &[e, esi0]));
        wd(0x00E8_7134, 0xFFFF_FFFF);
        s.put(r);
    }
}

// --- 0x004B6580 city_instances_job_b (frame pointer, three stack arguments) ------------------------------------------

unsafe fn e_4b6580(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0, ebp0) = (r.esi, r.edi, r.ebp);
        let mut s = St::of(r);
        s.ebp = r.esp.wrapping_sub(4);
        let mut edi = arg(a, 2);
        s.edi = edi;
        if (edi as i32) > 0 {
            let mut esi = arg(a, 1);
            loop {
                s.esi = esi;
                s.call(0x004B_6310, &im(r, a, &[esi, edi0, esi0]));
                esi = esi.wrapping_add(0x24);
                edi = edi.wrapping_sub(1);
                s.edi = edi;
                s.esi = esi;
                if edi == 0 {
                    break;
                }
            }
        }
        s.put(r);
        r.ebp = ebp0;
    }
}

// --- 0x004AE510 chunk_section_0f_render_materials (ESI, EDX implicit, four stack arguments) -----------------------------

unsafe fn e_4ae510(r: &mut Pushad, a: *const u32) {
    unsafe {
        let esi = r.esi as usize;
        let edx = r.edx as usize;
        let x = rd(esi);
        let mut m = x & 0x8000_000F;
        if (m as i32) < 0 {
            m = (m.wrapping_sub(1) | 0xFFFF_FFF0).wrapping_add(1);
        }
        let ecx = if m == 0 { 0 } else { 0x10u32.wrapping_sub(m) };
        let eax = rd(esi).wrapping_add(ecx);
        let mut c = rd(edx);
        c = c.wrapping_add(arg(a, 0));
        let edi = rd(c as usize);
        wd(edx + 0x174, edi);
        c = c.wrapping_add(4);
        wd(edx + 0x178, c);
        let v = eax.wrapping_add(arg(a, 1));
        let p3 = arg(a, 3);
        wd(p3 as usize, v);
        let c2 = arg(a, 2);
        let e2 = rd(c2 as usize + 0x90).wrapping_add(eax);
        wd(esi, e2);
        r.eax = eax;
        r.ecx = c2;
        r.edx = e2;
    }
}

// --- 0x006A8E40 act_packet_mission_set_checkpoint (one stack argument + one that is written) ----------------------------

unsafe fn e_6a8e40(r: &mut Pushad, a: *const u32) {
    unsafe {
        let str_p = arg(a, 0);
        let mut p = str_p;
        loop {
            let c = rb(p as usize);
            p = p.wrapping_add(1);
            if c == 0 {
                break;
            }
        }
        let len = p.wrapping_sub(str_p.wrapping_add(1));
        let eax = (len.wrapping_add(1)) & 0xFFFF;
        let ne = (rb(eax.wrapping_add(str_p) as usize) != 0) as u32;
        let ecx = (r.ecx & !0xFF) | ne;
        wd(a as usize + 4, ecx);
        wd(a as usize, str_p);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = str_p;
        s.call(0x006A_7D90, &fwd(a));
        s.put_all(r);
    }
}

// --- 0x00691020 profile_write_blocks (EAX = this, ESI = stream) --------------------------------------------------------

unsafe fn e_691020(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.eax;
        let mut st = Stk::new();
        st.push(r.ebx);
        st.push(r.edi);
        st.push(r.esi);
        let esi = r.esi;
        let mut s = St::of(r);
        s.edi = this;
        // 0xBB8
        st.push(1);
        let b1 = this.wrapping_add(0xBB8);
        s.ebx = b1;
        st.push(4);
        st.push(b1);
        s.call(0x00C9_817E, &st.img(r, a));
        s.eax = rd(b1 as usize);
        st.push(esi);
        st.push(s.eax);
        st.push(1);
        st.push(this);
        s.call(0x00C9_817E, &st.img(r, a));
        st.push(esi);
        st.push(1);
        let b2 = this.wrapping_add(0xBBC);
        s.ebx = b2;
        st.push(4);
        st.push(b2);
        s.call(0x00C9_817E, &st.img(r, a));
        s.ecx = rd(b2 as usize);
        st.push(esi);
        st.push(s.ecx);
        s.edx = this.wrapping_add(0x3E8);
        st.push(1);
        st.push(s.edx);
        s.call(0x00C9_817E, &st.img(r, a));
        st.pop(16);
        st.push(esi);
        st.push(1);
        let b3 = this.wrapping_add(0xBC0);
        s.ebx = b3;
        st.push(4);
        st.push(b3);
        s.call(0x00C9_817E, &st.img(r, a));
        s.eax = rd(b3 as usize);
        st.push(esi);
        st.push(s.eax);
        st.push(1);
        s.edi = this.wrapping_add(0x7D0);
        st.push(s.edi);
        s.call(0x00C9_817E, &st.img(r, a));
        st.push(esi);
        s.call(0x00C9_78DE, &st.img(r, a));
        s.eax = (s.eax & !0xFF) | 1;
        s.put(r);
    }
}


/// 64 / 32 signed divide (a hardware `idiv`, so a zero divisor or an overflow faults as the original does).
#[inline(always)]
unsafe fn idiv32(hi: u32, lo: u32, d: u32) -> (u32, u32) {
    let (q, rem): (u32, u32);
    unsafe {
        std::arch::asm!("idiv {d}", d = in(reg) d, inout("eax") lo => q, inout("edx") hi => rem, options(nomem, nostack));
    }
    (q, rem)
}

/// What `fld dword; fstp dword` makes of a 32-bit pattern: a signalling NaN comes out quiet.
#[inline(always)]
fn x87_quiet(v: u32) -> u32 {
    if v & 0x7F80_0000 == 0x7F80_0000 && v & 0x007F_FFFF != 0 { v | 0x0040_0000 } else { v }
}

// --- 0x0055A250 human_pkt_sub_40 (one stack argument) -----------------------------------------------------------------

unsafe fn e_55a250(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let edi = arg(a, 0);
        let ax = rd(edi as usize) & 0xFFFF;
        r.eax = ax;
        if ax == 0xFFFF {
            return;
        }
        let esi = ax;
        let mut eax = (esi << 15 ^ 0xFFFF_FFFF).wrapping_add(esi);
        let mut ecx = ((eax as i32) >> 10) as u32 ^ eax;
        ecx = ecx.wrapping_mul(9);
        eax = ((ecx as i32) >> 6) as u32;
        ecx ^= eax;
        let e2 = !(ecx << 11);
        ecx = ecx.wrapping_add(e2);
        eax = ((ecx as i32) >> 16) as u32 ^ ecx;
        ecx = rd(0x0212_E258);
        let mut edx: u32;
        let hi = ((eax as i32) >> 31) as u32;
        let (_q, rem) = idiv32(hi, eax, rd(ecx as usize + 0x60));
        edx = rem;
        eax = rd(ecx as usize + 0x5C);
        edx = rd(eax.wrapping_add(edx.wrapping_mul(4)) as usize);
        r.eax = eax;
        r.ecx = ecx;
        r.edx = edx;
        if edx == 0 {
            return;
        }
        loop {
            if esi == rd(edx as usize + 0xC) {
                break;
            }
            edx = rd(edx as usize + 4);
            r.edx = edx;
            if edx == 0 {
                return;
            }
        }
        edx = rd(edx as usize);
        r.edx = edx;
        if edx == 0 {
            return;
        }
        ecx = rd(edx as usize + 4);
        r.ecx = ecx;
        if ecx == 0 {
            return;
        }
        edx = (ecx & 0xFFF) << 4;
        r.edx = edx;
        eax = rd(edx as usize + 0x0214_9C64);
        r.eax = eax;
        if eax == 0 || rd(eax as usize + 0x44) != ecx || rd(eax as usize + 0x4C) != 1 || rb(eax as usize + 0x80) & 0x0B != 0 {
            return;
        }
        let ne = (rb(edi as usize + 3) != 0) as u32;
        edx = rb(edi as usize + 2) as i8 as i32 as u32;
        ecx = (ecx & !0xFF) | ne;
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.esi = esi;
        s.edi = eax.wrapping_add(0x10C4);
        s.call(0x0055_A310, &im(r, a, &[edx, ecx, esi0, edi0]));
        s.put(r);
    }
}

// --- 0x006B5910 act_packet_sub_11 (one stack argument) -------------------------------------------------------------------

unsafe fn e_6b5910(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let fr = [edi0, esi0];
        let edi = arg(a, 0);
        let ax = rd(edi as usize) & 0xFFFF;
        let esi = ax as u16 as i16 as i32 as u32;
        let mut s = St::of(r);
        s.eax = ax;
        s.edi = edi;
        s.esi = esi;
        'body: {
            if esi > 0x18D || (esi as i32) <= -1 || (esi as i32) >= (rd(0x0252_7CE8) as i32) {
                break 'body;
            }
            let o = ((esi << 4).wrapping_add(esi)).wrapping_mul(4).wrapping_add(0x0145_2BE0);
            s.eax = o;
            if rb(o as usize + 0xC) & 2 == 0 {
                break 'body;
            }
            s.eax = esi;
            s.call(0x006B_08C0, &im(r, a, &fr));
            if s.eax & 0xFF != 0 {
                s.eax = rd(edi as usize + 2);
                s.call(0x006B_4E30, &im(r, a, &fr));
                break 'body;
            }
            s.eax = esi;
            s.call(0x006B_0940, &im(r, a, &fr));
            if s.eax & 0xFF == 0 {
                break 'body;
            }
            s.ecx = rd(edi as usize + 2);
            wd(a as usize, s.ecx);
            let v = x87_quiet(s.ecx);
            s.call(0x006B_4EA0, &im(r, a, &[v, edi0, esi0]));
        }
        s.put(r);
    }
}

// --- 0x00AD6580 vehicle_runninglight_manager::update (ECX = this, ret 4) --------------------------------------------------

unsafe fn e_ad6580(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, ebx0, ecx0) = (r.esi, r.ebx, r.ecx);
        let fr = [ebx0, esi0, ecx0];
        let this = ecx0;
        let t = this as usize;
        let mut s = St::of(r);
        s.esi = this;
        s.eax = rd(t + 0x10);
        if s.eax == 0 {
            s.put(r);
            r.ecx = ecx0;
            return;
        }
        let nz = rd(s.eax as usize + 0x9060) != 0;
        s.eax = rd(t);
        s.edx = rd(s.eax as usize + 0x50);
        s.ebx = (s.ebx & !0xFF) | nz as u32;
        let f = s.edx as usize;
        s.call(f, &im(r, a, &fr));
        let obj = s.eax;
        let mut direct = false; // true: take the 0xAD65D6 path
        if obj != 0 {
            s.ecx = rd(obj as usize + 0x8F48);
            s.ecx >>= 1;
            s.ecx = (s.ecx & !0xFF) | (s.ecx & 1);
            if s.ecx & 0xFF != 0 {
                direct = true;
            }
        }
        if !direct {
            if rb(obj as usize + 0x9539) & 2 != 0 || s.ebx & 0xFF != 0 {
                direct = true;
            }
        }
        if !direct {
            s.edx = rd(t);
            s.eax = rd(s.edx as usize + 0x18);
            s.ecx = this;
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[0, 0x3F80_0000, ebx0, esi0, ecx0]));
        } else {
            s.ecx = (s.ecx & !0xFF) | rb(obj as usize + 0x9539) as u32;
            let cl = (s.ecx & 0xFF) as u8;
            let mut ret1 = false;
            if cl & 4 != 0 {
                s.eax &= !0xFF;
            } else if cl & 2 != 0 {
                ret1 = true;
            } else {
                s.ecx = rd(obj as usize + 0x44);
                let c = s.ecx;
                s.call(0x004D_9740, &im(r, a, &[c, 0xFFFF, 1, ebx0, esi0, ecx0]));
                if s.eax & 0xFF != 0 {
                    ret1 = true;
                } else {
                    s.eax &= !0xFF;
                }
            }
            if ret1 {
                s.eax = 1;
            }
            s.edx = rd(t);
            let e = s.eax;
            s.eax = rd(s.edx as usize + 0x18);
            s.ecx = this;
            let f = s.eax as usize;
            s.call(f, &im(r, a, &[e, 0x3F80_0000, ebx0, esi0, ecx0]));
            s.edx = rd(t);
            s.eax = rd(s.edx as usize + 0x48);
            s.ecx = this;
            let f = s.eax as usize;
            s.call(f, &im(r, a, &fr));
        }
        s.put(r);
        r.ecx = ecx0;
    }
}


// --- 0x00B9F370 vint_object_tween::on_destroy (ECX = this) ----------------------------------------------------------

unsafe fn e_b9f370(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (esi0, edi0) = (r.esi, r.edi);
        let this = r.ecx;
        let mut s = St::of(r);
        s.edi = this;
        for off in [0xACu32, 0xBC, 0xCC] {
            s.esi = this.wrapping_add(off);
            s.call(0x00B9_3590, &im(r, a, &[0, edi0, esi0]));
        }
        s.put(r);
    }
}

// --- 0x007759F0 options_init_from_settings ------------------------------------------------------------------------------

unsafe fn e_7759f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let ecx0 = r.ecx;
        let mut s = St::of(r);
        s.eax = rd(0x00E9_86F4);
        wd(0x01F7_A99C, s.eax);
        s.call(0x0077_3BE0, &im(r, a, &[ecx0]));
        s.call(0x0077_3EB0, &im(r, a, &[0, ecx0]));
        wd(0x01F7_A98C, x87_quiet(rd(0x00EE_34D8)));
        let c = rd(0x0252_6B70);
        wd(0x01F7_A990, x87_quiet(rd(0x00EE_34E0)));
        wd(0x01F7_A998, c);
        wd(0x01F7_A994, x87_quiet(rd(0x00EE_34DC)));
        s.put(r);
        r.ecx = ecx0;
    }
}

static CNT: [AtomicU32; 58] = [const { AtomicU32::new(0) }; 58];

extern "C" fn w_4aeae0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[0]);
    unsafe { e_4aeae0(r, a) }
}
thunk!(t_4aeae0, w_4aeae0, "ret");
extern "C" fn w_5c2650(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[1]);
    unsafe { e_5c2650(r, a) }
}
thunk!(t_5c2650, w_5c2650, "ret");
extern "C" fn w_7f0b00(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[2]);
    unsafe { e_7f0b00(r, a) }
}
thunk!(t_7f0b00, w_7f0b00, "ret");
extern "C" fn w_93c660(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[3]);
    unsafe { e_93c660(r, a) }
}
thunk!(t_93c660, w_93c660, "ret");
extern "C" fn w_aabc80(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[4]);
    unsafe { e_aabc80(r, a) }
}
thunk!(t_aabc80, w_aabc80, "ret");
extern "C" fn w_4616f0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[5]);
    unsafe { e_4616f0(r, a) }
}
thunk!(t_4616f0, w_4616f0, "ret");
extern "C" fn w_5d2060(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[6]);
    unsafe { e_5d2060(r, a) }
}
thunk!(t_5d2060, w_5d2060, "ret");
extern "C" fn w_4dedd0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[7]);
    unsafe { e_4dedd0(r, a) }
}
thunk!(t_4dedd0, w_4dedd0, "ret");
extern "C" fn w_404290(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[8]);
    unsafe { e_404290(r, a) }
}
thunk!(t_404290, w_404290, "ret");
extern "C" fn w_428950(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[9]);
    unsafe { e_428950(r, a) }
}
thunk!(t_428950, w_428950, "ret");
extern "C" fn w_603830(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[10]);
    unsafe { e_603830(r, a) }
}
thunk!(t_603830, w_603830, "ret");
extern "C" fn w_60bfa0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[11]);
    unsafe { e_60bfa0(r, a) }
}
thunk!(t_60bfa0, w_60bfa0, "ret");
extern "C" fn w_6186a0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[12]);
    unsafe { e_6186a0(r, a) }
}
thunk!(t_6186a0, w_6186a0, "ret 20");
extern "C" fn w_5ac070(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[13]);
    unsafe { e_5ac070(r, a) }
}
thunk!(t_5ac070, w_5ac070, "ret");
extern "C" fn w_5e49f0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[14]);
    unsafe { e_5e49f0(r, a) }
}
thunk!(t_5e49f0, w_5e49f0, "ret");
extern "C" fn w_6852e0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[15]);
    unsafe { e_6852e0(r, a) }
}
thunk!(t_6852e0, w_6852e0, "ret");
extern "C" fn w_70f100(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[16]);
    unsafe { e_70f100(r, a) }
}
thunk!(t_70f100, w_70f100, "ret");
extern "C" fn w_8afee0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[17]);
    unsafe { e_8afee0(r, a) }
}
thunk!(t_8afee0, w_8afee0, "ret");
extern "C" fn w_925500(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[18]);
    unsafe { e_925500(r, a) }
}
thunk!(t_925500, w_925500, "ret 4");
extern "C" fn w_887060(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[19]);
    unsafe { e_887060(r, a) }
}
thunk!(t_887060, w_887060, "ret");
extern "C" fn w_a7dd50(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[20]);
    unsafe { e_a7dd50(r, a) }
}
thunk!(t_a7dd50, w_a7dd50, "ret");
extern "C" fn w_a82ad0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[21]);
    unsafe { e_a82ad0(r, a) }
}
thunk!(t_a82ad0, w_a82ad0, "ret");
extern "C" fn w_a73850(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[22]);
    unsafe { e_a73850(r, a) }
}
thunk!(t_a73850, w_a73850, "ret 4");
extern "C" fn w_b69b60(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[23]);
    unsafe { e_b69b60(r, a) }
}
thunk!(t_b69b60, w_b69b60, "ret");
extern "C" fn w_b94290(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[24]);
    unsafe { e_b94290(r, a) }
}
thunk!(t_b94290, w_b94290, "ret");
extern "C" fn w_b8dbb0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[25]);
    unsafe { e_b8dbb0(r, a) }
}
thunk!(t_b8dbb0, w_b8dbb0, "ret");
extern "C" fn w_b8ae10(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[26]);
    unsafe { e_b8ae10(r, a) }
}
thunk!(t_b8ae10, w_b8ae10, "ret 4");
extern "C" fn w_b952f0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[27]);
    unsafe { e_b952f0(r, a) }
}
thunk!(t_b952f0, w_b952f0, "ret");
extern "C" fn w_ba3190(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[28]);
    unsafe { e_ba3190(r, a) }
}
thunk!(t_ba3190, w_ba3190, "ret 8");
extern "C" fn w_ba4cd0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[29]);
    unsafe { e_ba4cd0(r, a) }
}
thunk!(t_ba4cd0, w_ba4cd0, "ret");
extern "C" fn w_ce0e00(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[30]);
    unsafe { e_ce0e00(r, a) }
}
thunk!(t_ce0e00, w_ce0e00, "ret 8");
extern "C" fn w_a66ef0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[31]);
    unsafe { e_a66ef0(r, a) }
}
thunk!(t_a66ef0, w_a66ef0, "ret");
extern "C" fn w_a22740(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[32]);
    unsafe { e_a22740(r, a) }
}
thunk!(t_a22740, w_a22740, "ret");
extern "C" fn w_957000(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[33]);
    unsafe { e_957000(r, a) }
}
thunk!(t_957000, w_957000, "ret");
extern "C" fn w_79a580(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[34]);
    unsafe { e_79a580(r, a) }
}
thunk!(t_79a580, w_79a580, "ret 4");
extern "C" fn w_845790(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[35]);
    unsafe { e_845790(r, a) }
}
thunk!(t_845790, w_845790, "ret 12");
extern "C" fn w_bf3e10(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[36]);
    unsafe { e_bf3e10(r, a) }
}
thunk!(t_bf3e10, w_bf3e10, "ret");
extern "C" fn w_b97100(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[37]);
    unsafe { e_b97100(r, a) }
}
thunk!(t_b97100, w_b97100, "ret");
extern "C" fn w_b979f0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[38]);
    unsafe { e_b979f0(r, a) }
}
thunk!(t_b979f0, w_b979f0, "ret");
extern "C" fn w_d097a0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[39]);
    unsafe { e_d097a0(r, a) }
}
thunk!(t_d097a0, w_d097a0, "ret");
extern "C" fn w_cf6040(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[40]);
    unsafe { e_cf6040(r, a) }
}
thunk!(t_cf6040, w_cf6040, "ret 8");
extern "C" fn w_9a17b0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[41]);
    unsafe { e_9a17b0(r, a) }
}
thunk!(t_9a17b0, w_9a17b0, "ret");
extern "C" fn w_6f6310(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[42]);
    unsafe { e_6f6310(r, a) }
}
thunk!(t_6f6310, w_6f6310, "ret 4");
extern "C" fn w_5f16d0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[43]);
    unsafe { e_5f16d0(r, a) }
}
thunk!(t_5f16d0, w_5f16d0, "ret");
extern "C" fn w_645650(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[44]);
    unsafe { e_645650(r, a) }
}
thunk!(t_645650, w_645650, "ret");
extern "C" fn w_667950(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[45]);
    unsafe { e_667950(r, a) }
}
thunk!(t_667950, w_667950, "ret");
extern "C" fn w_66dee0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[46]);
    unsafe { e_66dee0(r, a) }
}
thunk!(t_66dee0, w_66dee0, "ret 4");
extern "C" fn w_4a0fb0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[47]);
    unsafe { e_4a0fb0(r, a) }
}
thunk!(t_4a0fb0, w_4a0fb0, "ret");
extern "C" fn w_61d360(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[48]);
    unsafe { e_61d360(r, a) }
}
thunk!(t_61d360, w_61d360, "ret");
extern "C" fn w_495ac0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[49]);
    unsafe { e_495ac0(r, a) }
}
thunk!(t_495ac0, w_495ac0, "ret");
extern "C" fn w_4b6580(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[50]);
    unsafe { e_4b6580(r, a) }
}
thunk!(t_4b6580, w_4b6580, "ret");
extern "C" fn w_4ae510(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[51]);
    unsafe { e_4ae510(r, a) }
}
thunk!(t_4ae510, w_4ae510, "ret");
extern "C" fn w_6a8e40(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[52]);
    unsafe { e_6a8e40(r, a) }
}
thunk!(t_6a8e40, w_6a8e40, "ret");
extern "C" fn w_691020(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[53]);
    unsafe { e_691020(r, a) }
}
thunk!(t_691020, w_691020, "ret");
extern "C" fn w_55a250(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[54]);
    unsafe { e_55a250(r, a) }
}
thunk!(t_55a250, w_55a250, "ret");
extern "C" fn w_6b5910(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[55]);
    unsafe { e_6b5910(r, a) }
}
thunk!(t_6b5910, w_6b5910, "ret");
extern "C" fn w_b9f370(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[56]);
    unsafe { e_b9f370(r, a) }
}
thunk!(t_b9f370, w_b9f370, "ret");
extern "C" fn w_7759f0(r: &mut Pushad, a: *const u32) {
    count_at(&CNT[57]);
    unsafe { e_7759f0(r, a) }
}
thunk!(t_7759f0, w_7759f0, "ret");

const B_4AEAE0: [u8; 36] = [0x8B, 0x08, 0x03, 0xCA, 0x8B, 0x11, 0x85, 0xD2, 0x89, 0x90, 0xDC, 0x01, 0x00, 0x00, 0xC7, 0x80, 0xE0, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x76, 0x09, 0x83, 0xC1, 0x04, 0x89, 0x88, 0xE0, 0x01, 0x00, 0x00, 0xC3];
const B_5C2650: [u8; 39] = [0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74, 0x1D, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x17, 0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0x83, 0xF8, 0x05, 0x75, 0x05, 0xE9, 0x9A, 0x9D, 0xFF, 0xFF, 0xC3];
const B_7F0B00: [u8; 23] = [0x8B, 0x44, 0x24, 0x04, 0x3B, 0x05, 0x14, 0x8B, 0x52, 0x02, 0x74, 0x0A, 0xA3, 0x14, 0x8B, 0x52, 0x02, 0xE9, 0x3A, 0xFE, 0xFF, 0xFF, 0xC3];
const B_93C660: [u8; 44] = [0x33, 0xC0, 0x80, 0xB8, 0x1C, 0xDA, 0x15, 0x02, 0x00, 0x74, 0x16, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x06, 0x72, 0xEF, 0x83, 0x3D, 0x80, 0x7A, 0xEB, 0x00, 0x00, 0x7D, 0x0F, 0xE9, 0x8F, 0xFF, 0xFF, 0xFF, 0xC7, 0x05, 0x80, 0x7A, 0xEB, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3];
const B_AABC80: [u8; 60] = [0x8B, 0x88, 0x50, 0x8F, 0x00, 0x00, 0x81, 0xE1, 0xBF, 0xFE, 0xFF, 0xFF, 0x83, 0xC9, 0x20, 0x89, 0x88, 0x50, 0x8F, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x81, 0xF9, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x88, 0x78, 0x8F, 0x00, 0x00, 0x7E, 0x0C, 0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x88, 0x78, 0x8F, 0x00, 0x00, 0x89, 0x90, 0x80, 0x8F, 0x00, 0x00, 0xC3];
const B_4616F0: [u8; 88] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x85, 0xF6, 0x74, 0x32, 0x8B, 0x4E, 0x44, 0x85, 0xC9, 0x74, 0x2B, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x17, 0x39, 0x48, 0x44, 0x75, 0x12, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x39, 0x48, 0x4C, 0x75, 0x08, 0x38, 0x88, 0xCC, 0x03, 0x00, 0x00, 0x74, 0x1B, 0x6A, 0x00, 0x33, 0xC0, 0x8B, 0xCE, 0xE8, 0x6A, 0x88, 0xFD, 0xFF, 0x68, 0xB8, 0x00, 0x00, 0x00, 0x6A, 0x1E, 0x56, 0xE8, 0x6D, 0x5E, 0xFD, 0xFF, 0x83, 0xC4, 0x10, 0x5E, 0xC3];
const B_5D2060: [u8; 54] = [0x56, 0x8B, 0xF1, 0xE8, 0xB8, 0x0F, 0xFF, 0xFF, 0x8B, 0x8E, 0xF8, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x1B, 0x8D, 0xB0, 0x54, 0x85, 0x00, 0x00, 0x6A, 0x00, 0x56, 0x33, 0xC9, 0xE8, 0xA7, 0xDA, 0x58, 0x00, 0x83, 0xC4, 0x08, 0x6A, 0x01, 0x56, 0xE8, 0x8C, 0xAF, 0x55, 0x00, 0x5E, 0xC3];
const B_4DEDD0: [u8; 82] = [0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x33, 0xF6, 0x39, 0x35, 0xCC, 0x6D, 0x52, 0x02, 0x57, 0x7E, 0x29, 0x33, 0xFF, 0xA1, 0xD0, 0x6D, 0x52, 0x02, 0x8B, 0x0C, 0x07, 0x51, 0x53, 0xE8, 0x3E, 0xA2, 0x7B, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x17, 0x83, 0xC6, 0x01, 0x81, 0xC7, 0xD8, 0x00, 0x00, 0x00, 0x3B, 0x35, 0xCC, 0x6D, 0x52, 0x02, 0x7C, 0xD9, 0x5F, 0x5E, 0x33, 0xC0, 0x5B, 0xC3, 0x8B, 0xC6, 0x69, 0xC0, 0xD8, 0x00, 0x00, 0x00, 0x03, 0x05, 0xD0, 0x6D, 0x52, 0x02, 0x5F, 0x5E, 0x5B, 0xC3];
const B_404290: [u8; 158] = [0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x8B, 0xFB, 0xE8, 0x92, 0xF3, 0xFF, 0xFF, 0x8B, 0x43, 0x44, 0x50, 0x50, 0xBE, 0x19, 0x00, 0x00, 0x00, 0xE8, 0xB3, 0xDE, 0xFF, 0xFF, 0xA1, 0xBC, 0x3A, 0xEC, 0x00, 0x8B, 0x0C, 0x85, 0xC0, 0x3A, 0xEC, 0x00, 0x8B, 0x73, 0x44, 0x83, 0xC0, 0x01, 0x83, 0xC4, 0x08, 0x3D, 0x00, 0x08, 0x00, 0x00, 0xA3, 0xBC, 0x3A, 0xEC, 0x00, 0x7C, 0x0A, 0xC7, 0x05, 0xBC, 0x3A, 0xEC, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x8B, 0x53, 0x03, 0x06, 0xF7, 0xE1, 0x8B, 0xC1, 0x2B, 0xC2, 0xD1, 0xE8, 0x03, 0xC2, 0xC1, 0xE8, 0x0A, 0x8B, 0xD0, 0x69, 0xD2, 0xD1, 0x07, 0x00, 0x00, 0x6A, 0x00, 0x8B, 0xC1, 0x6A, 0x02, 0x2B, 0xC2, 0x33, 0xFF, 0x56, 0x05, 0xE8, 0x03, 0x00, 0x00, 0x56, 0x8D, 0x4F, 0x19, 0xE8, 0x15, 0xD1, 0xFF, 0xFF, 0x8B, 0x8B, 0xF0, 0x0A, 0x00, 0x00, 0x83, 0xC4, 0x10, 0xE8, 0xA7, 0x23, 0x40, 0x00, 0x84, 0xC0, 0x75, 0x0D, 0x6A, 0x00, 0x6A, 0x00, 0x53, 0xE8, 0xE9, 0x25, 0x40, 0x00, 0x83, 0xC4, 0x0C, 0x5F, 0x5E, 0x5B, 0xC3];
const B_428950: [u8; 180] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x51, 0x56, 0x8B, 0x75, 0x08, 0x85, 0xF6, 0x74, 0x2F, 0x8B, 0x4E, 0x44, 0x85, 0xC9, 0x74, 0x28, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x14, 0x39, 0x48, 0x44, 0x75, 0x0F, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x09, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x01, 0x74, 0x71, 0xF6, 0x86, 0x70, 0x04, 0x00, 0x00, 0x10, 0x75, 0x0B, 0x6A, 0x00, 0x56, 0xE8, 0x31, 0xB2, 0x56, 0x00, 0x83, 0xC4, 0x08, 0xF6, 0x86, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x3C, 0x8B, 0x8E, 0x54, 0x01, 0x00, 0x00, 0x89, 0x8E, 0x30, 0x07, 0x00, 0x00, 0x8B, 0x96, 0x58, 0x01, 0x00, 0x00, 0x89, 0x96, 0x34, 0x07, 0x00, 0x00, 0x8B, 0x86, 0x5C, 0x01, 0x00, 0x00, 0x83, 0xA6, 0x60, 0x07, 0x00, 0x00, 0xFE, 0x80, 0xA6, 0x5C, 0x07, 0x00, 0x00, 0xFE, 0x89, 0x86, 0x38, 0x07, 0x00, 0x00, 0xC7, 0x86, 0x48, 0x07, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x6A, 0x01, 0x33, 0xC0, 0x8B, 0xCE, 0xE8, 0xAE, 0x15, 0x01, 0x00, 0x6A, 0x75, 0x6A, 0x0C, 0x56, 0xE8, 0xB4, 0xEB, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0x8B, 0xE5, 0x5D, 0xC3];
const B_603830: [u8; 97] = [0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x56, 0x74, 0x51, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x08, 0xFF, 0xD2, 0x8B, 0x0D, 0x88, 0x78, 0x52, 0x02, 0x8B, 0xF0, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0x83, 0xFE, 0x03, 0xC7, 0x05, 0x88, 0x78, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0x74, 0x29, 0x83, 0xFE, 0x0C, 0x74, 0x24, 0x83, 0xFE, 0x12, 0x74, 0x1F, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0A, 0x50, 0xE8, 0x75, 0xC4, 0x20, 0x00, 0x84, 0xC0, 0x74, 0x0C, 0x6A, 0x00, 0x6A, 0x00, 0xE8, 0xA8, 0xBA, 0xF5, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xE9, 0xBF, 0x77, 0xF3, 0xFF];
const B_60BFA0: [u8; 124] = [0x56, 0x8B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0xD1, 0x8B, 0x8E, 0x74, 0x0D, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x4B, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x37, 0x39, 0x48, 0x44, 0x75, 0x32, 0x83, 0x78, 0x4C, 0x05, 0x75, 0x2C, 0x80, 0xB8, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x75, 0x14, 0x8B, 0x88, 0xE8, 0x6C, 0x00, 0x00, 0x85, 0xC9, 0x75, 0x04, 0x33, 0xC0, 0xEB, 0x11, 0x7F, 0x04, 0x33, 0xC0, 0xEB, 0x0B, 0x8B, 0x80, 0xBC, 0x8A, 0x00, 0x00, 0xE8, 0x66, 0x4D, 0x35, 0x00, 0x3B, 0xC6, 0x74, 0x1C, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0x88, 0x13, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x42, 0x18, 0x7E, 0x08, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x42, 0x18, 0x5E, 0xC3];
const B_6186A0: [u8; 89] = [0x8B, 0xD1, 0x83, 0x7A, 0x18, 0xFF, 0x74, 0x4E, 0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x74, 0x46, 0x80, 0xB8, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x75, 0x0A, 0x8B, 0x88, 0xE8, 0x6C, 0x00, 0x00, 0x85, 0xC9, 0x7E, 0x33, 0x8B, 0x80, 0xBC, 0x8A, 0x00, 0x00, 0xE8, 0x92, 0x86, 0x34, 0x00, 0x85, 0xC0, 0x74, 0x24, 0x3B, 0x05, 0xD4, 0x03, 0x17, 0x02, 0x75, 0x1C, 0x8B, 0x44, 0x24, 0x0C, 0x01, 0x42, 0x64, 0x83, 0x7A, 0x1C, 0x01, 0x8B, 0x42, 0x64, 0x7F, 0x0C, 0x83, 0xF8, 0x3C, 0x7E, 0x07, 0x8B, 0xC2, 0xE8, 0x7A, 0x00, 0x00, 0x00, 0xC2, 0x14, 0x00];
const B_5AC070: [u8; 163] = [0x56, 0x57, 0x8B, 0xF1, 0xBF, 0x03, 0x00, 0x00, 0x00, 0xE8, 0xD2, 0x37, 0xF9, 0xFF, 0x8B, 0x46, 0x1C, 0x83, 0xF8, 0x04, 0x77, 0x7E, 0xFF, 0x24, 0x85, 0x14, 0xC1, 0x5A, 0x00, 0xE8, 0xCE, 0xF4, 0xFF, 0xFF, 0xBF, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xC7, 0x5F, 0xC7, 0x86, 0xF8, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0xE8, 0x45, 0xF3, 0xFF, 0xFF, 0xBF, 0x01, 0x00, 0x00, 0x00, 0x8B, 0xC7, 0x5F, 0xC7, 0x86, 0xF8, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x8B, 0xC6, 0xE8, 0xDA, 0xF5, 0xFF, 0xFF, 0x8B, 0xC7, 0x5F, 0xC7, 0x86, 0xF8, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x56, 0xE8, 0xA5, 0xF8, 0xFF, 0xFF, 0x8B, 0xF8, 0x5F, 0xC7, 0x86, 0xF8, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x83, 0xBE, 0x44, 0x26, 0x00, 0x00, 0x00, 0x8D, 0x86, 0x44, 0x26, 0x00, 0x00, 0x7C, 0x09, 0xE8, 0x32, 0x11, 0x63, 0x00, 0x84, 0xC0, 0x74, 0x02, 0x33, 0xFF, 0x8B, 0xC7, 0x5F, 0xC7, 0x86, 0xF8, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3];
const B_5E49F0: [u8; 147] = [0x51, 0x56, 0x57, 0xB9, 0x24, 0x07, 0xDF, 0x00, 0xE8, 0xB3, 0x7F, 0x5F, 0x00, 0x8B, 0x15, 0xAC, 0xA1, 0x52, 0x02, 0x85, 0xD2, 0x8B, 0xCA, 0x74, 0x16, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x39, 0x41, 0x48, 0x74, 0x1B, 0x8B, 0x09, 0x3B, 0xCA, 0x74, 0x04, 0x85, 0xC9, 0x75, 0xF1, 0x68, 0x24, 0x07, 0xDF, 0x00, 0xE8, 0xF7, 0xEC, 0x20, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x4F, 0xB9, 0x30, 0x07, 0xDF, 0x00, 0xE8, 0xA6, 0x07, 0x5B, 0x00, 0x85, 0xC0, 0x75, 0x3C, 0xB8, 0x30, 0x07, 0xDF, 0x00, 0xE8, 0x98, 0x05, 0x5B, 0x00, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x31, 0xBE, 0x64, 0x7D, 0xE9, 0x00, 0x8D, 0x47, 0x14, 0x83, 0x38, 0x00, 0x75, 0x07, 0x8B, 0x0E, 0xE8, 0xAE, 0xEF, 0x5A, 0x00, 0x83, 0xC6, 0x04, 0x83, 0xC0, 0x10, 0x81, 0xFE, 0x74, 0x7D, 0xE9, 0x00, 0x7C, 0xE6, 0xC7, 0x87, 0x14, 0x02, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0xE8, 0x91, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0x59, 0xC3];
const B_6852E0: [u8; 139] = [0x56, 0x8B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x85, 0xF6, 0x74, 0x7E, 0x8B, 0x46, 0x44, 0x85, 0xC0, 0x74, 0x77, 0x8B, 0x0D, 0x9C, 0xCE, 0xEB, 0x00, 0x85, 0xC9, 0x74, 0x14, 0x3B, 0x41, 0x44, 0x75, 0x0F, 0xA1, 0xA0, 0xCE, 0xEB, 0x00, 0x83, 0xF8, 0x07, 0x74, 0x05, 0x83, 0xF8, 0x08, 0x75, 0x59, 0x8B, 0xC6, 0xE8, 0xC9, 0x3F, 0x33, 0x00, 0x84, 0xC0, 0x75, 0x4E, 0x8B, 0xB6, 0x74, 0x0D, 0x00, 0x00, 0x8B, 0xCE, 0xE8, 0xC8, 0xD5, 0x45, 0x00, 0x85, 0xC0, 0x0F, 0x95, 0xC0, 0x84, 0xC0, 0x74, 0x38, 0x85, 0xF6, 0x74, 0x34, 0x8B, 0xC6, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x90, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xD2, 0x74, 0x20, 0x39, 0x72, 0x44, 0x75, 0x1B, 0x83, 0x7A, 0x4C, 0x05, 0x75, 0x15, 0xD9, 0xE8, 0x6A, 0x01, 0x6A, 0x01, 0x51, 0x83, 0xC9, 0xFF, 0xD9, 0x1C, 0x24, 0xE8, 0xFA, 0x57, 0x42, 0x00, 0x83, 0xC4, 0x0C, 0x5E, 0xC3];
const B_70F100: [u8; 126] = [0x66, 0xC7, 0x46, 0x06, 0x01, 0x00, 0xC7, 0x46, 0x08, 0x20, 0x8D, 0xE1, 0x00, 0xC7, 0x46, 0x0C, 0xA4, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x10, 0x14, 0x8D, 0xE1, 0x00, 0xC7, 0x46, 0x14, 0x2C, 0x8D, 0xE1, 0x00, 0xC7, 0x46, 0x20, 0x00, 0x00, 0x00, 0x00, 0x68, 0x00, 0x08, 0x00, 0x00, 0xB8, 0x0D, 0x00, 0x00, 0x00, 0x68, 0xFF, 0x00, 0x00, 0x00, 0x68, 0x80, 0xD6, 0x68, 0x01, 0xC7, 0x06, 0xB8, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x08, 0xDC, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x0C, 0xE8, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x10, 0xFC, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x14, 0x08, 0x8D, 0xE1, 0x00, 0x89, 0x46, 0x30, 0xC7, 0x46, 0x34, 0x80, 0xD6, 0x68, 0x01, 0xC7, 0x46, 0x20, 0x04, 0x00, 0x00, 0x00, 0x89, 0x46, 0x38, 0xE8, 0x48, 0x1A, 0x59, 0x00, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0xC3];
const B_8AFEE0: [u8; 116] = [0xD9, 0xE8, 0x53, 0x57, 0xD9, 0x5E, 0x0C, 0x68, 0x70, 0x4A, 0x40, 0x00, 0x6A, 0x10, 0x33, 0xDB, 0x89, 0x5E, 0x04, 0xC7, 0x46, 0x14, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x46, 0x18, 0xC0, 0xD4, 0x01, 0x00, 0xC7, 0x46, 0x1C, 0x30, 0x75, 0x00, 0x00, 0x88, 0x5E, 0x20, 0xC7, 0x06, 0x14, 0xF5, 0xE2, 0x00, 0x6A, 0x04, 0x8D, 0x46, 0x30, 0x8D, 0xBE, 0x8C, 0x04, 0x00, 0x00, 0x57, 0x89, 0x5E, 0x2C, 0xC7, 0x46, 0x28, 0x10, 0x00, 0x00, 0x00, 0x89, 0x46, 0x24, 0xE8, 0x21, 0x25, 0xB5, 0xFF, 0x89, 0xBE, 0x80, 0x04, 0x00, 0x00, 0x89, 0x9E, 0x88, 0x04, 0x00, 0x00, 0xC7, 0x86, 0x84, 0x04, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x5F, 0x89, 0x5E, 0x2C, 0xC7, 0x46, 0x14, 0x22, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0x5B, 0xC3];
const B_925500: [u8; 181] = [0xA1, 0xE8, 0x95, 0x52, 0x02, 0x53, 0x33, 0xDB, 0x3B, 0xC3, 0x56, 0x75, 0x0E, 0x89, 0x7F, 0x04, 0x89, 0x7F, 0x08, 0x89, 0x3D, 0xE8, 0x95, 0x52, 0x02, 0xEB, 0x12, 0x8B, 0x48, 0x08, 0x89, 0x4F, 0x08, 0x89, 0x47, 0x04, 0x8B, 0x50, 0x08, 0x89, 0x7A, 0x04, 0x89, 0x78, 0x08, 0x57, 0xE8, 0x7D, 0xF8, 0xFF, 0xFF, 0xD9, 0xEE, 0x89, 0x47, 0x44, 0x8B, 0x44, 0x24, 0x10, 0x89, 0x47, 0x4C, 0x89, 0x9F, 0x80, 0x00, 0x00, 0x00, 0x88, 0x9F, 0x84, 0x00, 0x00, 0x00, 0x83, 0x8F, 0x80, 0x00, 0x00, 0x00, 0x20, 0x8D, 0x77, 0x5C, 0xC7, 0x47, 0x7C, 0xFF, 0xFF, 0xFF, 0xFF, 0xD9, 0x1E, 0xD9, 0xE8, 0xB8, 0x04, 0x00, 0x00, 0x00, 0xD9, 0x5E, 0x04, 0x89, 0x46, 0x0C, 0xD9, 0x05, 0x50, 0x05, 0xDD, 0x00, 0x89, 0x46, 0x10, 0x83, 0xC4, 0x04, 0xD9, 0x5E, 0x18, 0x8B, 0xC7, 0x89, 0x5E, 0x08, 0x89, 0x5E, 0x14, 0x89, 0x5E, 0x1C, 0xE8, 0x86, 0xFD, 0xFF, 0xFF, 0x5E, 0x89, 0x9F, 0x88, 0x00, 0x00, 0x00, 0x89, 0x9F, 0xAC, 0x00, 0x00, 0x00, 0x89, 0x9F, 0xB0, 0x00, 0x00, 0x00, 0xC7, 0x87, 0x98, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x87, 0x9C, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x5B, 0xC2, 0x04, 0x00];
const B_887060: [u8; 170] = [0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x85, 0xED, 0x56, 0x57, 0xC7, 0x43, 0x50, 0x00, 0x00, 0x00, 0x00, 0x74, 0x24, 0x8B, 0x75, 0x08, 0x85, 0xF6, 0x74, 0x1D, 0x8B, 0xFF, 0x8B, 0x06, 0x68, 0x00, 0xC7, 0xE2, 0x00, 0x50, 0xE8, 0xA3, 0x1F, 0x41, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xF6, 0x85, 0xF6, 0x8B, 0xFE, 0x74, 0x62, 0x8B, 0x43, 0x50, 0x8D, 0x34, 0xC3, 0x83, 0xC0, 0x01, 0x57, 0xB9, 0xF4, 0xC6, 0xE2, 0x00, 0x89, 0x43, 0x50, 0xE8, 0x16, 0xD7, 0x2E, 0x00, 0x83, 0xC6, 0x04, 0x57, 0xB9, 0x58, 0x98, 0xE0, 0x00, 0xE8, 0x08, 0xD7, 0x2E, 0x00, 0x8B, 0x0E, 0x69, 0xC9, 0xE8, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xED, 0x89, 0x0E, 0x8B, 0xF7, 0x74, 0x22, 0x85, 0xFF, 0x74, 0x1E, 0x90, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x74, 0x16, 0x8B, 0x16, 0x68, 0x00, 0xC7, 0xE2, 0x00, 0x52, 0xE8, 0x3C, 0x1F, 0x41, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0xE5, 0xEB, 0x02, 0x33, 0xF6, 0x85, 0xF6, 0x8B, 0xFE, 0x75, 0x9E, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3];
const B_A7DD50: [u8; 136] = [0x53, 0x8B, 0x1D, 0x20, 0xA0, 0x52, 0x02, 0x55, 0x56, 0x57, 0x8B, 0xF8, 0x33, 0xED, 0x8B, 0xFF, 0x3B, 0xC5, 0x74, 0x69, 0x8B, 0x50, 0x04, 0x8B, 0xCA, 0x2B, 0xCF, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xCA, 0x3B, 0xF8, 0x75, 0x0A, 0x3B, 0xD0, 0x75, 0x04, 0x33, 0xFF, 0xEB, 0x16, 0x8B, 0xFA, 0x8B, 0x30, 0x3B, 0xF5, 0x74, 0x03, 0x89, 0x56, 0x04, 0x8B, 0x50, 0x04, 0x3B, 0xD5, 0x74, 0x04, 0x8B, 0x30, 0x89, 0x32, 0x3B, 0xDD, 0x89, 0x28, 0x89, 0x68, 0x04, 0x89, 0x68, 0x10, 0x89, 0x68, 0x14, 0x75, 0x12, 0x83, 0x05, 0x24, 0xA0, 0x52, 0x02, 0x01, 0x89, 0x40, 0x04, 0x89, 0x00, 0x8B, 0xD8, 0x8B, 0xC1, 0xEB, 0xAC, 0x8B, 0x13, 0x83, 0x05, 0x24, 0xA0, 0x52, 0x02, 0x01, 0x89, 0x10, 0x89, 0x58, 0x04, 0x8B, 0x13, 0x89, 0x42, 0x04, 0x89, 0x03, 0x8B, 0xC1, 0xEB, 0x93, 0x5F, 0x5E, 0x5D, 0x89, 0x1D, 0x20, 0xA0, 0x52, 0x02, 0x5B, 0xC3];
const B_A82AD0: [u8; 100] = [0x66, 0x81, 0xFA, 0xFF, 0xFF, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0xF0, 0x75, 0x05, 0x32, 0xC0, 0x5F, 0x5E, 0xC3, 0x33, 0xC0, 0xB9, 0xF4, 0x95, 0x22, 0x02, 0xEB, 0x03, 0x8D, 0x49, 0x00, 0x66, 0x39, 0x11, 0x74, 0x1F, 0x81, 0xC1, 0xD0, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x81, 0xF9, 0x74, 0x9C, 0x22, 0x02, 0x7C, 0xEA, 0x52, 0x56, 0x57, 0xE8, 0x52, 0xFC, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x5F, 0x5E, 0xC3, 0x69, 0xC0, 0xD0, 0x00, 0x00, 0x00, 0x05, 0x28, 0x95, 0x22, 0x02, 0x8B, 0x88, 0xC8, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x89, 0x0E, 0x7E, 0x04, 0x8B, 0xF0, 0xF3, 0xA5, 0x5F, 0xB0, 0x01, 0x5E, 0xC3];
const B_A73850: [u8; 90] = [0x8B, 0x47, 0x34, 0x85, 0xC0, 0x75, 0x10, 0x8B, 0x0D, 0x24, 0x9E, 0x52, 0x02, 0x8B, 0x11, 0x89, 0x15, 0x28, 0x9E, 0x52, 0x02, 0xEB, 0x0B, 0x8B, 0x00, 0x6A, 0xFF, 0x50, 0xFF, 0x15, 0x00, 0x81, 0xDB, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x83, 0xC6, 0x08, 0x56, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0x4F, 0x34, 0x8B, 0x15, 0xC0, 0x80, 0xEB, 0x00, 0x8B, 0x42, 0x04, 0x83, 0xE9, 0x08, 0x51, 0xB9, 0xC0, 0x80, 0xEB, 0x00, 0xFF, 0xD0, 0x56, 0xC7, 0x47, 0x34, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5E, 0xC2, 0x04, 0x00];
const B_B69B60: [u8; 125] = [0x56, 0x8B, 0x35, 0x5C, 0x7B, 0x2D, 0x02, 0x57, 0x33, 0xFF, 0x3B, 0xF7, 0x74, 0x6A, 0x8B, 0x4E, 0x70, 0x3B, 0xCE, 0x75, 0x08, 0x89, 0x3D, 0x5C, 0x7B, 0x2D, 0x02, 0xEB, 0x1D, 0x8B, 0x46, 0x74, 0x3B, 0xC7, 0x89, 0x0D, 0x5C, 0x7B, 0x2D, 0x02, 0x74, 0x03, 0x89, 0x48, 0x70, 0x8B, 0x46, 0x70, 0x3B, 0xC7, 0x74, 0x06, 0x8B, 0x4E, 0x74, 0x89, 0x48, 0x74, 0x6A, 0x78, 0x57, 0x56, 0x89, 0x7E, 0x74, 0x89, 0x7E, 0x70, 0xE8, 0x17, 0x70, 0x13, 0x00, 0xA1, 0x58, 0x7B, 0x2D, 0x02, 0x83, 0xC4, 0x0C, 0x3B, 0xC7, 0x75, 0x11, 0x5F, 0x89, 0x76, 0x70, 0x89, 0x76, 0x74, 0x89, 0x35, 0x58, 0x7B, 0x2D, 0x02, 0x8B, 0xC6, 0x5E, 0xC3, 0x8B, 0x50, 0x74, 0x89, 0x56, 0x74, 0x89, 0x46, 0x70, 0x8B, 0x48, 0x74, 0x89, 0x71, 0x70, 0x89, 0x70, 0x74, 0x5F, 0x8B, 0xC6, 0x5E, 0xC3];
const B_B94290: [u8; 64] = [0x57, 0x8B, 0xF8, 0x80, 0x7F, 0x1C, 0x00, 0x74, 0x04, 0x33, 0xC0, 0x5F, 0xC3, 0xE8, 0x7E, 0x0E, 0x00, 0x00, 0x85, 0xC0, 0x74, 0xF3, 0x8B, 0x4F, 0x0C, 0x85, 0xC9, 0x75, 0x0A, 0x89, 0x40, 0x04, 0x89, 0x00, 0x89, 0x47, 0x0C, 0x5F, 0xC3, 0x8B, 0x09, 0x89, 0x08, 0x8B, 0x57, 0x0C, 0x89, 0x50, 0x04, 0x8B, 0x4F, 0x0C, 0x8B, 0x11, 0x89, 0x42, 0x04, 0x8B, 0x4F, 0x0C, 0x89, 0x01, 0x5F, 0xC3];
const B_B8DBB0: [u8; 134] = [0x8B, 0x0D, 0xC0, 0xA1, 0x52, 0x02, 0x56, 0x57, 0x33, 0xFF, 0x3B, 0xCF, 0x8B, 0xF1, 0x74, 0x55, 0x8B, 0x01, 0x8B, 0x40, 0x08, 0x52, 0xFF, 0xD0, 0x84, 0xC0, 0x74, 0x65, 0x39, 0x35, 0xC0, 0xA1, 0x52, 0x02, 0x75, 0x14, 0x8B, 0x46, 0x20, 0x3B, 0xC6, 0x75, 0x08, 0x89, 0x3D, 0xC0, 0xA1, 0x52, 0x02, 0xEB, 0x17, 0xA3, 0xC0, 0xA1, 0x52, 0x02, 0x8B, 0x4E, 0x24, 0x8B, 0x56, 0x20, 0x89, 0x51, 0x20, 0x8B, 0x46, 0x20, 0x8B, 0x4E, 0x24, 0x89, 0x48, 0x24, 0xA1, 0xBC, 0xA1, 0x52, 0x02, 0x3B, 0xC7, 0x89, 0x7E, 0x24, 0x89, 0x7E, 0x20, 0x75, 0x11, 0x89, 0x76, 0x20, 0x89, 0x76, 0x24, 0x89, 0x35, 0xBC, 0xA1, 0x52, 0x02, 0x5F, 0x8B, 0xC6, 0x5E, 0xC3, 0x8B, 0x50, 0x24, 0x89, 0x46, 0x20, 0x89, 0x56, 0x24, 0x8B, 0x48, 0x24, 0x89, 0x71, 0x20, 0x89, 0x70, 0x24, 0x5F, 0x8B, 0xC6, 0x5E, 0xC3, 0x5F, 0x33, 0xC0, 0x5E, 0xC3];
const B_B8AE10: [u8; 100] = [0x51, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x85, 0xDB, 0x56, 0x57, 0x8B, 0xF9, 0x74, 0x4F, 0x85, 0xFF, 0x74, 0x4B, 0xE8, 0xB9, 0x10, 0x07, 0x00, 0x85, 0xC0, 0x74, 0x42, 0xBF, 0x74, 0x9A, 0xE3, 0x00, 0xE8, 0xAB, 0x10, 0x07, 0x00, 0x8B, 0xF0, 0x85, 0xF6, 0x74, 0x32, 0xEB, 0x03, 0x8D, 0x49, 0x00, 0x53, 0x56, 0xE8, 0x99, 0xD6, 0x00, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xF6, 0x74, 0x1F, 0x8B, 0xFF, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x74, 0x16, 0x8B, 0x06, 0x68, 0x74, 0x9A, 0xE3, 0x00, 0x50, 0xE8, 0xCC, 0xE1, 0x10, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0xE5, 0xEB, 0xD3, 0x5F, 0x5E, 0x5B, 0x59, 0xC2, 0x04, 0x00];
const B_B952F0: [u8; 172] = [0x80, 0xBE, 0x29, 0x02, 0x00, 0x00, 0x01, 0x75, 0x16, 0x8B, 0x86, 0x1C, 0x02, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0C, 0x8B, 0x8E, 0x24, 0x02, 0x00, 0x00, 0x50, 0xE8, 0xB1, 0xF1, 0xFF, 0xFF, 0x80, 0xBE, 0x28, 0x02, 0x00, 0x00, 0x01, 0x75, 0x75, 0x57, 0x8B, 0xBE, 0x20, 0x02, 0x00, 0x00, 0x85, 0xFF, 0x74, 0x53, 0x8B, 0x57, 0x0C, 0x85, 0xD2, 0x74, 0x3B, 0x8B, 0xC2, 0x83, 0xE0, 0x3F, 0x8B, 0x04, 0x85, 0x70, 0x26, 0x2F, 0x02, 0x8B, 0xC8, 0x85, 0xC9, 0x74, 0x29, 0x8D, 0x64, 0x24, 0x00, 0x39, 0x51, 0x18, 0x74, 0x0D, 0x8B, 0x49, 0x08, 0x3B, 0xC8, 0x74, 0x19, 0x85, 0xC9, 0x75, 0xF0, 0xEB, 0x13, 0x8B, 0x01, 0x8B, 0x57, 0x08, 0x8B, 0x40, 0x04, 0x68, 0x2C, 0x02, 0x00, 0x00, 0x56, 0x6A, 0x02, 0x52, 0xFF, 0xD0, 0x8B, 0x47, 0x14, 0x3B, 0x86, 0x20, 0x02, 0x00, 0x00, 0x74, 0x06, 0x8B, 0xF8, 0x85, 0xFF, 0x75, 0xAD, 0x8B, 0x86, 0x1C, 0x02, 0x00, 0x00, 0x85, 0xC0, 0x5F, 0x74, 0x0C, 0x8B, 0x8E, 0x24, 0x02, 0x00, 0x00, 0x50, 0xE8, 0x93, 0xF0, 0xFF, 0xFF, 0xC6, 0x86, 0x28, 0x02, 0x00, 0x00, 0x00, 0xC6, 0x86, 0x29, 0x02, 0x00, 0x00, 0x00, 0xC3];
const B_BA3190: [u8; 69] = [0x56, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x08, 0xE8, 0x04, 0x1A, 0xA9, 0xFF, 0x89, 0x86, 0x98, 0x00, 0x00, 0x00, 0xA1, 0xE4, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x13, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0x0C, 0x8B, 0x8E, 0x9C, 0x00, 0x00, 0x00, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0x80, 0x8E, 0xF0, 0x00, 0x00, 0x00, 0x02, 0xC7, 0x86, 0x9C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00];
const B_BA4CD0: [u8; 151] = [0xA1, 0x6C, 0xDF, 0xEB, 0x00, 0x8B, 0x08, 0x56, 0xBE, 0x60, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x68, 0xF1, 0x1B, 0x03, 0xC7, 0x05, 0x60, 0xF1, 0x1B, 0x03, 0x08, 0xE8, 0xEB, 0x00, 0xC7, 0x05, 0x64, 0xF1, 0x1B, 0x03, 0x02, 0x00, 0x00, 0x00, 0xE8, 0xE4, 0x35, 0xFF, 0xFF, 0x8B, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0x83, 0xF9, 0x20, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x8B, 0x32, 0x73, 0x2C, 0x8B, 0xC1, 0xC1, 0xE0, 0x04, 0x8B, 0xD1, 0x83, 0xC1, 0x01, 0xC7, 0x80, 0x70, 0x23, 0x2F, 0x02, 0x60, 0x7D, 0xE1, 0x00, 0x89, 0xB0, 0x74, 0x23, 0x2F, 0x02, 0xC7, 0x80, 0x78, 0x23, 0x2F, 0x02, 0x60, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0xEB, 0x03, 0x83, 0xCA, 0xFF, 0x89, 0x15, 0x74, 0x6C, 0xE8, 0x00, 0xE8, 0xD7, 0x03, 0x00, 0x00, 0xA1, 0x74, 0x6C, 0xE8, 0x00, 0x3B, 0x05, 0x70, 0xA2, 0x52, 0x02, 0x5E, 0x73, 0x0D, 0xC1, 0xE0, 0x04, 0xC7, 0x80, 0x7C, 0x23, 0x2F, 0x02, 0x54, 0xE8, 0xEB, 0x00, 0xB0, 0x01, 0xC3];
const B_CE0E00: [u8; 92] = [0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x8B, 0x45, 0x18, 0x8B, 0x48, 0x24, 0x56, 0x57, 0x8B, 0x78, 0x30, 0x33, 0xF6, 0x85, 0xFF, 0x89, 0x4C, 0x24, 0x14, 0x7E, 0x2A, 0x33, 0xDB, 0x8D, 0x49, 0x00, 0x8B, 0x55, 0x18, 0x8B, 0x42, 0x3C, 0x8B, 0x0C, 0x18, 0x8B, 0x54, 0x24, 0x18, 0x51, 0x52, 0xE8, 0xFC, 0x81, 0xFB, 0xFF, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x14, 0x83, 0xC6, 0x01, 0x83, 0xC3, 0x3C, 0x3B, 0xF7, 0x7C, 0xDB, 0x5F, 0x5E, 0x5D, 0x83, 0xC8, 0xFF, 0x5B, 0xC2, 0x08, 0x00, 0x8B, 0x44, 0x24, 0x14, 0x5F, 0x03, 0xC6, 0x5E, 0x5D, 0x5B, 0xC2, 0x08, 0x00];
const B_A66EF0: [u8; 88] = [0xD9, 0xEE, 0xC7, 0x00, 0xBC, 0xE3, 0xE3, 0x00, 0x8B, 0x0D, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x48, 0x0C, 0x8B, 0x15, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x50, 0x0C, 0x33, 0xC9, 0x89, 0x48, 0x10, 0x89, 0x48, 0x08, 0x88, 0x48, 0x14, 0xC6, 0x40, 0x15, 0xFF, 0xC7, 0x40, 0x04, 0x40, 0x29, 0xA6, 0x00, 0xC7, 0x00, 0x0C, 0x38, 0xE4, 0x00, 0x89, 0x48, 0x3C, 0x89, 0x48, 0x40, 0x89, 0x48, 0x34, 0x89, 0x48, 0x38, 0x89, 0x48, 0x30, 0xD9, 0x50, 0x24, 0xD9, 0x50, 0x28, 0xD9, 0x50, 0x2C, 0xD9, 0x50, 0x18, 0xD9, 0x50, 0x1C, 0xD9, 0x58, 0x20, 0xC3];
const B_A22740: [u8; 135] = [0xD9, 0xEE, 0x8B, 0xC1, 0xC7, 0x00, 0xBC, 0xE3, 0xE3, 0x00, 0x8B, 0x0D, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x48, 0x0C, 0x8B, 0x15, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x50, 0x0C, 0xD9, 0x50, 0x48, 0xD9, 0x50, 0x4C, 0xC6, 0x40, 0x15, 0xFF, 0xD9, 0x50, 0x58, 0xC7, 0x40, 0x04, 0x40, 0x29, 0xA6, 0x00, 0xD9, 0x50, 0x5C, 0xC7, 0x00, 0xA8, 0xE3, 0xE3, 0x00, 0xD9, 0x50, 0x50, 0x33, 0xC9, 0xD9, 0x50, 0x54, 0x89, 0x48, 0x10, 0x89, 0x48, 0x08, 0x88, 0x48, 0x14, 0xD9, 0x50, 0x60, 0xD9, 0x50, 0x64, 0xD9, 0x50, 0x68, 0xD9, 0x50, 0x6C, 0xD9, 0x50, 0x70, 0xD9, 0x50, 0x74, 0x66, 0x89, 0x48, 0x78, 0xD9, 0x50, 0x18, 0xD9, 0x50, 0x1C, 0xD9, 0x50, 0x20, 0xD9, 0xE8, 0xD9, 0x50, 0x24, 0xD9, 0x50, 0x34, 0xD9, 0x58, 0x44, 0xD9, 0x50, 0x28, 0xD9, 0x50, 0x2C, 0xD9, 0x50, 0x30, 0xD9, 0x50, 0x38, 0xD9, 0x50, 0x3C, 0xD9, 0x58, 0x40, 0xC3];
const B_957000: [u8; 122] = [0x56, 0x8B, 0x35, 0xBC, 0xE5, 0x15, 0x02, 0x85, 0xF6, 0x74, 0x15, 0x33, 0xD2, 0x8D, 0x46, 0x5C, 0x80, 0x38, 0x00, 0x74, 0x0F, 0x83, 0xC2, 0x01, 0x83, 0xC0, 0x30, 0x83, 0xFA, 0x08, 0x7C, 0xF0, 0x33, 0xC0, 0x5E, 0xC3, 0x8D, 0x04, 0x52, 0xC1, 0xE0, 0x04, 0x8D, 0x0C, 0x30, 0xC6, 0x41, 0x5C, 0x01, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x7F, 0x44, 0x8D, 0x41, 0x5C, 0x8D, 0x54, 0x52, 0x06, 0xC7, 0x81, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x89, 0x3C, 0x32, 0x33, 0xD2, 0x89, 0x51, 0x68, 0x89, 0x51, 0x6C, 0x89, 0x51, 0x70, 0x89, 0x51, 0x74, 0x89, 0x51, 0x78, 0x89, 0x51, 0x7C, 0x89, 0x91, 0x80, 0x00, 0x00, 0x00, 0x5F, 0x89, 0x91, 0x84, 0x00, 0x00, 0x00, 0xC7, 0x41, 0x64, 0xFF, 0xFF, 0xFF, 0xFF, 0x5E, 0xC3];
const B_79A580: [u8; 95] = [0x83, 0xF9, 0xFF, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x7F, 0x06, 0x32, 0xC0, 0x5B, 0xC2, 0x04, 0x00, 0x8B, 0x43, 0x7C, 0x83, 0xE8, 0x00, 0x74, 0x0D, 0x83, 0xE8, 0x01, 0x75, 0xED, 0xD9, 0xEE, 0xD9, 0x9B, 0x80, 0x00, 0x00, 0x00, 0x56, 0x57, 0x51, 0x68, 0xCC, 0xE5, 0xE1, 0x00, 0x8D, 0x73, 0x3C, 0xBF, 0x19, 0x00, 0x00, 0x00, 0xE8, 0x96, 0xA5, 0x47, 0x00, 0x83, 0xC4, 0x08, 0xBF, 0xD0, 0xE5, 0xE1, 0x00, 0xBE, 0xF8, 0xE5, 0xE1, 0x00, 0xE8, 0x14, 0xA4, 0x05, 0x00, 0xD9, 0xEE, 0x5F, 0xD9, 0x9B, 0xA0, 0x00, 0x00, 0x00, 0x89, 0x43, 0x70, 0x5E, 0xB0, 0x01, 0x5B, 0xC2, 0x04, 0x00];
const B_845790: [u8; 152] = [0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0, 0x0F, 0x84, 0x89, 0x00, 0x00, 0x00, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x56, 0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x72, 0x39, 0x46, 0x44, 0x75, 0x6D, 0x83, 0x7E, 0x4C, 0x02, 0x75, 0x67, 0x8B, 0x86, 0x10, 0x01, 0x00, 0x00, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x85, 0xFF, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0x74, 0x4D, 0x8B, 0x4E, 0x7C, 0x68, 0xD0, 0xA6, 0x52, 0x02, 0xE8, 0xFD, 0x71, 0xEB, 0xFF, 0x8B, 0x4C, 0x24, 0x18, 0x0F, 0xB7, 0x01, 0x0F, 0xB7, 0xD0, 0x83, 0xC4, 0x04, 0x80, 0x3C, 0x3A, 0x00, 0x0F, 0x95, 0xC2, 0x83, 0xC0, 0x01, 0x66, 0x89, 0x01, 0x8B, 0x86, 0x80, 0x00, 0x00, 0x00, 0xC1, 0xE8, 0x04, 0x24, 0x01, 0x3A, 0xD0, 0x88, 0x54, 0x24, 0x0C, 0x74, 0x13, 0x8B, 0x4C, 0x24, 0x0C, 0x6A, 0x00, 0x6A, 0x01, 0x51, 0x8B, 0xCE, 0xE8, 0x40, 0x10, 0x0E, 0x00, 0x83, 0xC4, 0x0C, 0x5F, 0x5E, 0xC2, 0x0C, 0x00];
const B_BF3E10: [u8; 32] = [0x56, 0x8B, 0xF1, 0x83, 0x46, 0x28, 0x01, 0x57, 0x8D, 0x7E, 0x0C, 0x57, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x83, 0x46, 0x04, 0x01, 0x57, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5F, 0x5E, 0xC3];
const B_B97100: [u8; 57] = [0x8B, 0xC2, 0x57, 0x8D, 0x78, 0x01, 0x8A, 0x08, 0x83, 0xC0, 0x01, 0x84, 0xC9, 0x75, 0xF7, 0x8B, 0x0E, 0x2B, 0xC7, 0x8B, 0x7E, 0x0C, 0x89, 0x04, 0x39, 0x83, 0x46, 0x0C, 0x04, 0x8B, 0x4E, 0x0C, 0x8D, 0x78, 0x01, 0x57, 0x52, 0x8B, 0x16, 0x03, 0xD1, 0x52, 0xE8, 0x21, 0x97, 0x10, 0x00, 0x01, 0x7E, 0x0C, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0x5F, 0xC3];
const B_B979F0: [u8; 42] = [0x85, 0xC9, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x83, 0x79, 0x24, 0x00, 0x74, 0x05, 0xE8, 0x1E, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x1C, 0x89, 0x41, 0x20, 0x89, 0x71, 0x24, 0x8B, 0x56, 0x30, 0x52, 0x89, 0x4E, 0x1C, 0xE8, 0x99, 0xFF, 0xFF, 0xFF, 0xB0, 0x01, 0xC3];
const B_D097A0: [u8; 34] = [0x56, 0x8B, 0xF1, 0x8B, 0x46, 0x70, 0x83, 0x78, 0x24, 0x00, 0x74, 0x12, 0x8B, 0x16, 0x8B, 0x42, 0x34, 0xFF, 0xD0, 0x8B, 0x4E, 0x70, 0xC1, 0xE0, 0x04, 0x03, 0x41, 0x24, 0x5E, 0xC3, 0x33, 0xC0, 0x5E, 0xC3];
const B_CF6040: [u8; 55] = [0x53, 0x56, 0x8B, 0xD9, 0x33, 0xF6, 0x39, 0x73, 0x08, 0x76, 0x25, 0x55, 0x8B, 0x6C, 0x24, 0x14, 0x57, 0x8D, 0x7B, 0x0C, 0x8B, 0x0F, 0x8B, 0x01, 0x8B, 0x54, 0x24, 0x14, 0x8B, 0x40, 0x08, 0x55, 0x52, 0xFF, 0xD0, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x04, 0x3B, 0x73, 0x08, 0x72, 0xE6, 0x5F, 0x5D, 0x5E, 0xB0, 0x01, 0x5B, 0xC2, 0x08, 0x00];
const B_9A17B0: [u8; 180] = [0x80, 0x3D, 0x3F, 0x84, 0xE8, 0x00, 0x00, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x80, 0x3D, 0x15, 0x97, 0x52, 0x02, 0x00, 0x75, 0xF4, 0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00, 0x75, 0xE2, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02, 0x00, 0x75, 0xD9, 0x8B, 0x4A, 0x44, 0xE8, 0xD8, 0x4D, 0xF8, 0xFF, 0x84, 0xC0, 0x74, 0xCD, 0xF6, 0x82, 0xC0, 0x00, 0x00, 0x00, 0x40, 0x75, 0xC4, 0xD9, 0xEE, 0x56, 0xD8, 0x9A, 0x24, 0x10, 0x00, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x7A, 0x5B, 0x83, 0xBA, 0x20, 0x10, 0x00, 0x00, 0xFF, 0x75, 0x52, 0x8B, 0x82, 0xE0, 0x05, 0x00, 0x00, 0x8B, 0xB0, 0x64, 0x01, 0x00, 0x00, 0xB9, 0x00, 0x08, 0x00, 0x00, 0x8B, 0xC6, 0xE8, 0x2A, 0xDF, 0xD4, 0xFF, 0x84, 0xC0, 0x75, 0x36, 0x8B, 0xC2, 0xE8, 0x1F, 0x4E, 0xF8, 0xFF, 0x84, 0xC0, 0x74, 0x10, 0xB9, 0x00, 0x00, 0x20, 0x00, 0x8B, 0xC6, 0xE8, 0x0F, 0xDF, 0xD4, 0xFF, 0x84, 0xC0, 0x75, 0x1B, 0x8A, 0x82, 0x91, 0x05, 0x00, 0x00, 0x3C, 0x02, 0x74, 0x04, 0x3C, 0x03, 0x75, 0x09, 0x80, 0xBA, 0x74, 0x04, 0x00, 0x00, 0x02, 0x75, 0x04, 0xB0, 0x01, 0x5E, 0xC3, 0x32, 0xC0, 0x5E, 0xC3];
const B_6F6310: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xD4, 0x8A, 0xE1, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x35, 0x85, 0x5E, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
const B_5F16D0: [u8; 20] = [0x56, 0x57, 0xBF, 0xB0, 0x12, 0xDF, 0x00, 0xBE, 0xC4, 0x12, 0xDF, 0x00, 0xE8, 0xFF, 0x32, 0x20, 0x00, 0x5F, 0x5E, 0xC3];
const B_645650: [u8; 110] = [0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x64, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x08, 0xFF, 0xD2, 0x83, 0xF8, 0x19, 0x75, 0x55, 0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x4C, 0x83, 0x78, 0x10, 0x01, 0x75, 0x12, 0x8B, 0x48, 0x2C, 0x81, 0xE1, 0x01, 0x00, 0x00, 0x80, 0x79, 0x1A, 0x49, 0x83, 0xC9, 0xFE, 0x41, 0xEB, 0x13, 0x8B, 0x50, 0x2C, 0x81, 0xE2, 0x01, 0x00, 0x00, 0x80, 0x79, 0x05, 0x4A, 0x83, 0xCA, 0xFE, 0x42, 0x83, 0xFA, 0x01, 0x0F, 0x94, 0xC1, 0x84, 0xC9, 0x75, 0x1A, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x11, 0x01, 0x50, 0x78, 0x8B, 0x48, 0x78, 0x57, 0x8B, 0xB8, 0x08, 0x09, 0x00, 0x00, 0x51, 0xE8, 0x84, 0x7D, 0x14, 0x00, 0x5F, 0xC3];
const B_667950: [u8; 80] = [0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x01, 0x53, 0x8A, 0x18, 0x56, 0x8B, 0x70, 0x02, 0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x88, 0x4C, 0x24, 0x0C, 0x74, 0x32, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x2C, 0x8B, 0x80, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x10, 0x8B, 0xC8, 0x8B, 0x42, 0x04, 0xFF, 0xD0, 0x83, 0xF8, 0x0A, 0x75, 0x18, 0x80, 0x7C, 0x24, 0x0C, 0x00, 0x56, 0x0F, 0x95, 0xC1, 0x84, 0xDB, 0x0F, 0x95, 0xC2, 0x51, 0x52, 0xE8, 0x76, 0x6F, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x5E, 0x5B, 0xC3];
const B_66DEE0: [u8; 52] = [0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0x53, 0x8B, 0xF9, 0xE8, 0xA1, 0xFC, 0xFF, 0xFF, 0x8D, 0x77, 0x24, 0x53, 0xB9, 0x6C, 0xDC, 0xDF, 0x00, 0xE8, 0xD3, 0x68, 0x50, 0x00, 0x8B, 0x06, 0x69, 0xC0, 0xE8, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x04, 0xC6, 0x47, 0x0C, 0x00, 0x5F, 0x89, 0x06, 0x5E, 0x5B, 0xC2, 0x04, 0x00];
const B_4A0FB0: [u8; 98] = [0x8B, 0x0D, 0xD4, 0xA6, 0x52, 0x02, 0xD9, 0xEE, 0xA1, 0xD0, 0xA6, 0x52, 0x02, 0xD9, 0x1D, 0x48, 0x6C, 0x52, 0x02, 0x8B, 0x15, 0xD8, 0xA6, 0x52, 0x02, 0x56, 0x57, 0x89, 0x0D, 0xD8, 0x68, 0x5F, 0x02, 0x89, 0x0D, 0xF0, 0x68, 0x5F, 0x02, 0x89, 0x0D, 0x9C, 0x68, 0x5F, 0x02, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xBE, 0xE4, 0x5A, 0xEC, 0x00, 0xBF, 0xA4, 0x68, 0x5F, 0x02, 0xF3, 0xA5, 0x5F, 0xA3, 0xD4, 0x68, 0x5F, 0x02, 0x89, 0x15, 0xDC, 0x68, 0x5F, 0x02, 0xA3, 0xEC, 0x68, 0x5F, 0x02, 0x89, 0x15, 0xF4, 0x68, 0x5F, 0x02, 0xA3, 0x98, 0x68, 0x5F, 0x02, 0x89, 0x15, 0xA0, 0x68, 0x5F, 0x02, 0x5E, 0xC3];
const B_61D360: [u8; 31] = [0x56, 0x8B, 0xF1, 0xE8, 0xE8, 0x56, 0xFE, 0xFF, 0x32, 0xC0, 0x88, 0x46, 0x1C, 0xC7, 0x86, 0xEC, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x88, 0x86, 0xF8, 0x00, 0x00, 0x00, 0x5E, 0xC3];
const B_495AC0: [u8; 90] = [0x56, 0x33, 0xC9, 0xB8, 0x3C, 0x9E, 0xDD, 0x00, 0xE8, 0x23, 0xE9, 0x6D, 0x00, 0x85, 0xC0, 0x74, 0x22, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x1B, 0x8B, 0x06, 0x68, 0x0C, 0x98, 0xDD, 0x00, 0x50, 0xE8, 0x4B, 0x35, 0x80, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xF6, 0x56, 0xE8, 0x25, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0x38, 0x71, 0xE8, 0x00, 0x8B, 0x11, 0xA1, 0x34, 0x71, 0xE8, 0x00, 0x8B, 0x52, 0x34, 0x50, 0xFF, 0xD2, 0xC7, 0x05, 0x34, 0x71, 0xE8, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x5E, 0xC3];
const B_4B6580: [u8; 41] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x56, 0x57, 0x8B, 0x7D, 0x10, 0x85, 0xFF, 0x7E, 0x14, 0x8B, 0x75, 0x0C, 0x56, 0xE8, 0x78, 0xFD, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x83, 0xC6, 0x24, 0x83, 0xEF, 0x01, 0x75, 0xEF, 0x5F, 0x5E, 0x8B, 0xE5, 0x5D, 0xC3];
const B_4AE510: [u8; 86] = [0x8B, 0x06, 0x25, 0x0F, 0x00, 0x00, 0x80, 0x79, 0x05, 0x48, 0x83, 0xC8, 0xF0, 0x40, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x07, 0xB9, 0x10, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x8B, 0x06, 0x03, 0xC1, 0x8B, 0x0A, 0x57, 0x8B, 0x7C, 0x24, 0x08, 0x03, 0xCF, 0x8B, 0x39, 0x89, 0xBA, 0x74, 0x01, 0x00, 0x00, 0x83, 0xC1, 0x04, 0x89, 0x8A, 0x78, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x0C, 0x8D, 0x0C, 0x10, 0x8B, 0x54, 0x24, 0x14, 0x89, 0x0A, 0x8B, 0x4C, 0x24, 0x10, 0x8B, 0x91, 0x90, 0x00, 0x00, 0x00, 0x03, 0xD0, 0x89, 0x16, 0x5F, 0xC3];
const B_6A8E40: [u8; 53] = [0x8B, 0x54, 0x24, 0x04, 0x8B, 0xC2, 0x56, 0x8D, 0x70, 0x01, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00, 0x8A, 0x08, 0x83, 0xC0, 0x01, 0x84, 0xC9, 0x75, 0xF7, 0x2B, 0xC6, 0x83, 0xC0, 0x01, 0x0F, 0xB7, 0xC0, 0x38, 0x0C, 0x10, 0x5E, 0x0F, 0x95, 0xC1, 0x89, 0x4C, 0x24, 0x08, 0x89, 0x54, 0x24, 0x04, 0xE9, 0x1B, 0xEF, 0xFF, 0xFF];
const B_691020: [u8; 120] = [0x53, 0x57, 0x56, 0x8B, 0xF8, 0x6A, 0x01, 0x8D, 0x9F, 0xB8, 0x0B, 0x00, 0x00, 0x6A, 0x04, 0x53, 0xE8, 0x49, 0x71, 0x60, 0x00, 0x8B, 0x03, 0x56, 0x50, 0x6A, 0x01, 0x57, 0xE8, 0x3D, 0x71, 0x60, 0x00, 0x56, 0x6A, 0x01, 0x8D, 0x9F, 0xBC, 0x0B, 0x00, 0x00, 0x6A, 0x04, 0x53, 0xE8, 0x2C, 0x71, 0x60, 0x00, 0x8B, 0x0B, 0x56, 0x51, 0x8D, 0x97, 0xE8, 0x03, 0x00, 0x00, 0x6A, 0x01, 0x52, 0xE8, 0x1A, 0x71, 0x60, 0x00, 0x83, 0xC4, 0x40, 0x56, 0x6A, 0x01, 0x8D, 0x9F, 0xC0, 0x0B, 0x00, 0x00, 0x6A, 0x04, 0x53, 0xE8, 0x06, 0x71, 0x60, 0x00, 0x8B, 0x03, 0x56, 0x50, 0x6A, 0x01, 0x81, 0xC7, 0xD0, 0x07, 0x00, 0x00, 0x57, 0xE8, 0xF4, 0x70, 0x60, 0x00, 0x56, 0xE8, 0x4E, 0x68, 0x60, 0x00, 0x83, 0xC4, 0x24, 0x5F, 0xB0, 0x01, 0x5B, 0xC3];
const B_55A250: [u8; 180] = [0x57, 0x8B, 0x7C, 0x24, 0x08, 0x0F, 0xB7, 0x07, 0x66, 0x3D, 0xFF, 0xFF, 0x0F, 0x84, 0xA0, 0x00, 0x00, 0x00, 0x56, 0x0F, 0xB7, 0xF0, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6, 0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1, 0xC1, 0xF8, 0x06, 0x33, 0xC8, 0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33, 0xC1, 0x8B, 0x0D, 0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x41, 0x5C, 0x8B, 0x14, 0x90, 0x85, 0xD2, 0x74, 0x5D, 0x3B, 0x72, 0x0C, 0x74, 0x0A, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75, 0xF4, 0x5E, 0x5F, 0xC3, 0x8B, 0x12, 0x85, 0xD2, 0x74, 0x48, 0x8B, 0x4A, 0x04, 0x85, 0xC9, 0x74, 0x41, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x2C, 0x39, 0x48, 0x44, 0x75, 0x27, 0x83, 0x78, 0x4C, 0x01, 0x75, 0x21, 0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x75, 0x18, 0x80, 0x7F, 0x03, 0x00, 0x0F, 0xBE, 0x57, 0x02, 0x0F, 0x95, 0xC1, 0x8D, 0xB8, 0xC4, 0x10, 0x00, 0x00, 0x51, 0x52, 0xE8, 0x0F, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC3];
const B_6B5910: [u8; 116] = [0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x0F, 0xB7, 0x07, 0x0F, 0xBF, 0xF0, 0x81, 0xFE, 0x8D, 0x01, 0x00, 0x00, 0x77, 0x5D, 0x83, 0xFE, 0xFF, 0x7E, 0x58, 0x3B, 0x35, 0xE8, 0x7C, 0x52, 0x02, 0x7D, 0x50, 0x8B, 0xC6, 0xC1, 0xE0, 0x04, 0x03, 0xC6, 0x8D, 0x04, 0x85, 0xE0, 0x2B, 0x45, 0x01, 0x85, 0xC0, 0x74, 0x3E, 0xF6, 0x40, 0x0C, 0x02, 0x74, 0x38, 0x8B, 0xC6, 0xE8, 0x70, 0xAF, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x0B, 0x8B, 0x47, 0x02, 0xE8, 0xD4, 0xF4, 0xFF, 0xFF, 0x5F, 0x5E, 0xC3, 0x8B, 0xC6, 0xE8, 0xDA, 0xAF, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x17, 0x8B, 0x4F, 0x02, 0x89, 0x4C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0x51, 0xD9, 0x1C, 0x24, 0xE8, 0x22, 0xF5, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC3];
const B_B9F370: [u8; 46] = [0x56, 0x57, 0x8B, 0xF9, 0x6A, 0x00, 0x8D, 0xB7, 0xAC, 0x00, 0x00, 0x00, 0xE8, 0x0F, 0x42, 0xFF, 0xFF, 0x6A, 0x00, 0x8D, 0xB7, 0xBC, 0x00, 0x00, 0x00, 0xE8, 0x02, 0x42, 0xFF, 0xFF, 0x6A, 0x00, 0x8D, 0xB7, 0xCC, 0x00, 0x00, 0x00, 0xE8, 0xF5, 0x41, 0xFF, 0xFF, 0x5F, 0x5E, 0xC3];
const B_7759F0: [u8; 76] = [0x51, 0xA1, 0xF4, 0x86, 0xE9, 0x00, 0xA3, 0x9C, 0xA9, 0xF7, 0x01, 0xE8, 0xE0, 0xE1, 0xFF, 0xFF, 0x6A, 0x00, 0xE8, 0xA9, 0xE4, 0xFF, 0xFF, 0xD9, 0x05, 0xD8, 0x34, 0xEE, 0x00, 0x8B, 0x0D, 0x70, 0x6B, 0x52, 0x02, 0xD9, 0x1D, 0x8C, 0xA9, 0xF7, 0x01, 0xD9, 0x05, 0xE0, 0x34, 0xEE, 0x00, 0x83, 0xC4, 0x04, 0xD9, 0x1D, 0x90, 0xA9, 0xF7, 0x01, 0x89, 0x0D, 0x98, 0xA9, 0xF7, 0x01, 0xD9, 0x05, 0xDC, 0x34, 0xEE, 0x00, 0xD9, 0x1D, 0x94, 0xA9, 0xF7, 0x01, 0x59, 0xC3];

static PORTS: [Port; 58] = [
    Port { name: "chunk_section_1c_object_attachments", addr: 0x004AEAE0, body: &B_4AEAE0, thunk: t_4aeae0, cnt: &CNT[0] },
    Port { name: "act_packet_fuzz_objective", addr: 0x005C2650, body: &B_5C2650, thunk: t_5c2650, cnt: &CNT[1] },
    Port { name: "vint_draw_set_mask_mode", addr: 0x007F0B00, body: &B_7F0B00, thunk: t_7f0b00, cnt: &CNT[2] },
    Port { name: "ambient_spawn_timer_update", addr: 0x0093C660, body: &B_93C660, thunk: t_93c660, cnt: &CNT[3] },
    Port { name: "vehicle_flag_8f50_stamp_set", addr: 0x00AABC80, body: &B_AABC80, thunk: t_aabc80, cnt: &CNT[4] },
    Port { name: "ai_order_k1e_enter", addr: 0x004616F0, body: &B_4616F0, thunk: t_4616f0, cnt: &CNT[5] },
    Port { name: "fuzz_crime_arsonist::enter_state3", addr: 0x005D2060, body: &B_5D2060, thunk: t_5d2060, cnt: &CNT[6] },
    Port { name: "weather_find", addr: 0x004DEDD0, body: &B_4DEDD0, thunk: t_4dedd0, cnt: &CNT[7] },
    Port { name: "ai_order_k02_enter", addr: 0x00404290, body: &B_404290, thunk: t_404290, cnt: &CNT[8] },
    Port { name: "ai_order_k0c_enter", addr: 0x00428950, body: &B_428950, thunk: t_428950, cnt: &CNT[9] },
    Port { name: "game_mode_diversion_leave", addr: 0x00603830, body: &B_603830, thunk: t_603830, cnt: &CNT[10] },
    Port { name: "firetruck_diversion_type_info::update", addr: 0x0060BFA0, body: &B_60BFA0, thunk: t_60bfa0, cnt: &CNT[11] },
    Port { name: "stunt_jumping_diversion_type::on_game_event_15", addr: 0x006186A0, body: &B_6186A0, thunk: t_6186a0, cnt: &CNT[12] },
    Port { name: "security_level_instance::update", addr: 0x005AC070, body: &B_5AC070, thunk: t_5ac070, cnt: &CNT[13] },
    Port { name: "button_mashing_hud_open", addr: 0x005E49F0, body: &B_5E49F0, thunk: t_5e49f0, cnt: &CNT[14] },
    Port { name: "cheat_repair_vehicle", addr: 0x006852E0, body: &B_6852E0, thunk: t_6852e0, cnt: &CNT[15] },
    Port { name: "havok_game_filter::ctor", addr: 0x0070F100, body: &B_70F100, thunk: t_70f100, cnt: &CNT[16] },
    Port { name: "mp_bh_activity_hitman_info::ctor", addr: 0x008AFEE0, body: &B_8AFEE0, thunk: t_8afee0, cnt: &CNT[17] },
    Port { name: "object_register", addr: 0x00925500, body: &B_925500, thunk: t_925500, cnt: &CNT[18] },
    Port { name: "parked_vehicles_parse_abandon_despawn_info", addr: 0x00887060, body: &B_887060, thunk: t_887060, cnt: &CNT[19] },
    Port { name: "streamman_read_nodes_free_list", addr: 0x00A7DD50, body: &B_A7DD50, thunk: t_a7dd50, cnt: &CNT[20] },
    Port { name: "chunk_extras_list_get", addr: 0x00A82AD0, body: &B_A82AD0, thunk: t_a82ad0, cnt: &CNT[21] },
    Port { name: "async_io_wait_event", addr: 0x00A73850, body: &B_A73850, thunk: t_a73850, cnt: &CNT[22] },
    Port { name: "weapon_effect_alloc", addr: 0x00B69B60, body: &B_B69B60, thunk: t_b69b60, cnt: &CNT[23] },
    Port { name: "vint_datagroup_add_item", addr: 0x00B94290, body: &B_B94290, thunk: t_b94290, cnt: &CNT[24] },
    Port { name: "vint_callback_create", addr: 0x00B8DBB0, body: &B_B8DBB0, thunk: t_b8dbb0, cnt: &CNT[25] },
    Port { name: "vint_document_parse_object_list", addr: 0x00B8AE10, body: &B_B8AE10, thunk: t_b8ae10, cnt: &CNT[26] },
    Port { name: "vint_dataitem_notify", addr: 0x00B952F0, body: &B_B952F0, thunk: t_b952f0, cnt: &CNT[27] },
    Port { name: "vint_element_text::set_text_tag_crc", addr: 0x00BA3190, body: &B_BA3190, thunk: t_ba3190, cnt: &CNT[28] },
    Port { name: "vint_element_clip_register_type", addr: 0x00BA4CD0, body: &B_BA4CD0, thunk: t_ba4cd0, cnt: &CNT[29] },
    Port { name: "rig_find_bone_by_name", addr: 0x00CE0E00, body: &B_CE0E00, thunk: t_ce0e00, cnt: &CNT[30] },
    Port { name: "script_object::dtor_5", addr: 0x00A66EF0, body: &B_A66EF0, thunk: t_a66ef0, cnt: &CNT[31] },
    Port { name: "script_object::dtor", addr: 0x00A22740, body: &B_A22740, thunk: t_a22740, cnt: &CNT[32] },
    Port { name: "current_spawn_group_alloc_member", addr: 0x00957000, body: &B_957000, thunk: t_957000, cnt: &CNT[33] },
    Port { name: "hud_diversion_respect_message", addr: 0x0079A580, body: &B_79A580, thunk: t_79a580, cnt: &CNT[34] },
    Port { name: "item_multi_registration::on_ownership_taken", addr: 0x00845790, body: &B_845790, thunk: t_845790, cnt: &CNT[35] },
    Port { name: "event_trigger::begin", addr: 0x00BF3E10, body: &B_BF3E10, thunk: t_bf3e10, cnt: &CNT[36] },
    Port { name: "vint_writer_write_string", addr: 0x00B97100, body: &B_B97100, thunk: t_b97100, cnt: &CNT[37] },
    Port { name: "vint_object_attach_child", addr: 0x00B979F0, body: &B_B979F0, thunk: t_b979f0, cnt: &CNT[38] },
    Port { name: "rl_lod_mesh_inst::select_shadow_lod_entry", addr: 0x00D097A0, body: &B_D097A0, thunk: t_d097a0, cnt: &CNT[39] },
    Port { name: "rl_container_inst::render", addr: 0x00CF6040, body: &B_CF6040, thunk: t_cf6040, cnt: &CNT[40] },
    Port { name: "human_player_move_input_active", addr: 0x009A17B0, body: &B_9A17B0, thunk: t_9a17b0, cnt: &CNT[41] },
    Port { name: "GeneralCollisionListener::scalar_deleting_dtor", addr: 0x006F6310, body: &B_6F6310, thunk: t_6f6310, cnt: &CNT[42] },
    Port { name: "ambulance_diversion_type_info::get_title", addr: 0x005F16D0, body: &B_5F16D0, thunk: t_5f16d0, cnt: &CNT[43] },
    Port { name: "act_packet_sub_59", addr: 0x00645650, body: &B_645650, thunk: t_645650, cnt: &CNT[44] },
    Port { name: "act_packet_sub_43", addr: 0x00667950, body: &B_667950, thunk: t_667950, cnt: &CNT[45] },
    Port { name: "stop_complication::load_from_table", addr: 0x0066DEE0, body: &B_66DEE0, thunk: t_66dee0, cnt: &CNT[46] },
    Port { name: "camera_blend_state_reset", addr: 0x004A0FB0, body: &B_4A0FB0, thunk: t_4a0fb0, cnt: &CNT[47] },
    Port { name: "stunt_oncoming_traffic_diversion_type::init", addr: 0x0061D360, body: &B_61D360, thunk: t_61d360, cnt: &CNT[48] },
    Port { name: "camera_free_xtbl_load", addr: 0x00495AC0, body: &B_495AC0, thunk: t_495ac0, cnt: &CNT[49] },
    Port { name: "city_instances_job_b", addr: 0x004B6580, body: &B_4B6580, thunk: t_4b6580, cnt: &CNT[50] },
    Port { name: "chunk_section_0f_render_materials", addr: 0x004AE510, body: &B_4AE510, thunk: t_4ae510, cnt: &CNT[51] },
    Port { name: "act_packet_mission_set_checkpoint", addr: 0x006A8E40, body: &B_6A8E40, thunk: t_6a8e40, cnt: &CNT[52] },
    Port { name: "profile_write_blocks", addr: 0x00691020, body: &B_691020, thunk: t_691020, cnt: &CNT[53] },
    Port { name: "human_pkt_sub_40", addr: 0x0055A250, body: &B_55A250, thunk: t_55a250, cnt: &CNT[54] },
    Port { name: "act_packet_sub_11", addr: 0x006B5910, body: &B_6B5910, thunk: t_6b5910, cnt: &CNT[55] },
    Port { name: "vint_object_tween::on_destroy", addr: 0x00B9F370, body: &B_B9F370, thunk: t_b9f370, cnt: &CNT[56] },
    Port { name: "options_init_from_settings", addr: 0x007759F0, body: &B_7759F0, thunk: t_7759f0, cnt: &CNT[57] },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_s5: replaced {ok}/{} with Rust", PORTS.len()));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let n = p.cnt.load(Ordering::Relaxed);
            (n > 0).then(|| format!("{} {n}", p.name))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_s5: {}", parts.join(", "))
}
