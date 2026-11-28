//! Port module s4 (Sonnet trial round): small functions from the s4 list.
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each port is a Rust
//! handler `h_XXXXXX(&mut Pushad, *const u32)` behind a `pushad` + XMM0-7 save thunk; the handler
//! edits the saved registers (`popad` restores them) and reads the stack arguments through `a`.
//! Callees are called by address through `St::call` with the registers the original sets up, so
//! the leftover EAX / ECX / EDX and the callee-clobbered registers flow out as in the original.
//!
//! Check: the shared sr2-judge (see docs/PORTING_RULES.md); per-port call counters at run time.

#![allow(unused_imports)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const CAP: u32 = 1 << 24;

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

    /// Write every register back (use when the original keeps none of them).
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

    /// Write back all but ESI / EDI (the original pushes and pops them).
    #[inline(always)]
    fn put_ne(&self, r: &mut Pushad) {
        r.eax = self.eax;
        r.ecx = self.ecx;
        r.edx = self.edx;
        r.ebx = self.ebx;
        r.ebp = self.ebp;
    }

    /// Call a game function by address; returns its EAX.
    #[inline(always)]
    unsafe fn call(&mut self, f: usize, args: &[u32]) -> u32 {
        unsafe { call_regs_st(f, self, args, std::ptr::null_mut()) };
        self.eax
    }

    /// Call a game function that returns on the x87 stack (popped as f64).
    #[inline(always)]
    unsafe fn callf(&mut self, f: usize, args: &[u32]) -> f64 {
        let mut out = 0f64;
        unsafe { call_regs_st(f, self, args, &mut out) };
        out
    }
}

/// Registers in and out, the callee's stack arguments (at most 12; zero padding above), and the
/// x87 result popped into `fout` when it is not null. ESP is restored from a TEB scratch slot.
#[inline(never)]
unsafe fn call_regs_st(f: usize, st: &mut St, args: &[u32], fout: *mut f64) {
    let mut a12 = [0u32; 12];
    a12[..args.len()].copy_from_slice(args);
    let block: [u32; 4] = [f as u32, a12.as_ptr() as u32, st as *mut St as u32, fout as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push dword ptr [eax + 12]",
            "push dword ptr [eax + 8]",
            "push dword ptr fs:[0x14]",
            "mov ebx, eax",
            "mov fs:[0x14], esp",
            "push dword ptr [ebx]",
            "mov esi, [ebx + 4]",
            "mov ecx, 12",
            "2:",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jnz 2b",
            "mov ebx, [ebx + 8]",
            "mov eax, [ebx]", "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]",
            "mov esi, [ebx + 16]", "mov edi, [ebx + 20]", "mov ebp, [ebx + 24]", "mov ebx, [ebx + 12]",
            "call dword ptr [esp + 48]",
            "push edi",
            "mov edi, fs:[0x14]",
            "mov edi, [edi + 4]",
            "mov [edi], eax", "mov [edi + 4], ecx", "mov [edi + 8], edx", "mov [edi + 12], ebx",
            "mov [edi + 16], esi", "mov [edi + 24], ebp",
            "pop dword ptr [edi + 20]",
            "mov esp, fs:[0x14]",
            "mov edi, [esp + 8]",
            "test edi, edi",
            "jz 3f",
            "fstp qword ptr [edi]",
            "3:",
            "pop dword ptr fs:[0x14]",
            "add esp, 8",
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

// --- handlers ------------------------------------------------------------------------------------

#[inline(always)]
fn lo(full: u32, b: u8) -> u32 {
    (full & !0xFF) | b as u32
}

// camera_mode8_update: copy the external camera block
#[inline(always)]
unsafe fn hb_495950(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let ecx = rd(0x25f5d60);
        let eax = rd(0x25f5d5c);
        let edx = rd(0x25f5d64);
        wd(0x25f5b24, ecx);
        wd(0x25f5b20, eax);
        wd(0x25f5b28, edx);
        std::ptr::copy(0x25f5d68usize as *const u32, 0x25f5b5cusize as *mut u32, 9);
        wf(0x25f5ba8, f32q(rf(0x25f5d8c)));
        r.eax = eax;
        r.ecx = 0;
        r.edx = edx;
    }
}

// camera_external_mode_enter
#[inline(always)]
unsafe fn hb_4a0140(_r: &mut Pushad, _a: *const u32) {
    unsafe { wf(0x25f5ba8, f32q(rf(0xdd3448))) }
}

// chunk_section_08_collision_mopp: EAX = this, ECX = offset
#[inline(always)]
unsafe fn hb_4ae280(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let this = r.eax as usize;
        let edx = rd(this);
        let mut ecx = r.ecx;
        let mut esi = rd(edx as usize + ecx as usize);
        ecx = ecx.wrapping_add(4);
        wd(this + 0x7c, esi);
        esi = edx.wrapping_add(ecx);
        ecx = ecx.wrapping_add(rd(this + 0x7c));
        let flag = rb(this + 0x1c) & 0x13 != 0;
        wd(this + 0x78, esi);
        esi = rd(edx.wrapping_add(ecx) as usize);
        ecx = edx.wrapping_add(ecx).wrapping_add(4);
        wd(this + 0x84, esi);
        wd(this + 0x80, ecx);
        if flag {
            ecx = 0;
            wd(this + 0x7c, 0);
            wd(this + 0x78, 0);
            wd(this + 0x84, 0);
            wd(this + 0x80, 0);
        }
        r.edx = edx;
        r.ecx = ecx;
    }
}

// chunk_section_24_object_runtime_data: ESI = size, ECX = this
#[inline(always)]
unsafe fn hb_4aef60(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut e = r.esi & 0x8000_0003;
        if (e as i32) < 0 {
            e = (e.wrapping_sub(1) | 0xFFFF_FFFC).wrapping_add(1);
        }
        let edx = if e == 0 { 0 } else { 4u32.wrapping_sub(e) };
        let this = r.ecx as usize;
        wd(this + 0x230, rd(this + 0x48));
        let eax = rd(this).wrapping_add(edx).wrapping_add(r.esi);
        wd(this + 0x234, eax);
        r.eax = eax;
        r.edx = edx;
    }
}

// script_object::dtor_3 (constructor-shaped): ECX = this
#[inline(always)]
unsafe fn hb_a58ab0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let o = r.ecx as usize;
        const G: usize = 0x33d6308;
        wd(o, 0xe3e3bc);
        wd(o + 0xc, rd(G));
        wd(o + 0xc, rd(G));
        wd(o + 4, 0xa62940);
        wd(o + 0x10, 0);
        wd(o + 8, 0);
        wb(o + 0x14, 0);
        wb(o + 0x15, 0xff);
        wd(o, 0xe430e0);
        wd(o + 0x40, rd(G));
        wd(o + 0x38, 0xFFFF_FFFF);
        wd(o + 0x3c, 0xFFFF_FFFF);
        wd(o + 0x40, rd(G));
        wd(o + 4, 0xa62960);
        wd(o + 0x20, 0xFFFF_FFFF);
        wd(o + 0x28, 0xffff);
        wd(o + 0x24, 0);
        wd(o + 0x2c, 0);
        r.eax = o as u32;
        r.ecx = 0xFFFF_FFFF;
        r.edx = 0;
    }
}

// road_lane_zone_traffic_info: EAX = zone pointer, result in EAX
#[inline(always)]
unsafe fn hb_b800c0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let al = rb(r.eax as usize + 0x29);
        let mut go_12a = false;
        let mut ten = false;
        if al >= 0x40 {
            let cl = rb(0x2528b48);
            r.ecx = lo(r.ecx, cl);
            if cl != 1 {
                go_12a = true;
            } else {
                let dl = rb(0x2528b49);
                r.edx = lo(r.edx, dl);
                if dl != 0 {
                    go_12a = true;
                }
            }
        } else {
            ten = true;
            let mut eax = al as u32;
            if (eax as i32) < (rd(0x2526cb0) as i32) {
                eax = rd(eax as usize * 4 + 0xf839e0);
                if (eax as i32) >= 0 && (eax as i32) < (rd(0x2526c90) as i32) {
                    eax = eax.wrapping_mul(0xa8).wrapping_add(rd(0x2526c94));
                    if eax != 0 {
                        r.eax = rd(eax as usize + 0x74);
                        return;
                    }
                }
            }
        }
        if ten {
            let a = rb(0x2528b48);
            if a != 1 {
                go_12a = true;
            } else {
                let cl = rb(0x2528b49);
                r.ecx = lo(r.ecx, cl);
                if cl != 0 {
                    go_12a = true;
                }
            }
        }
        if !go_12a {
            let eax = rd(0x2526c84);
            if eax != 0 {
                r.eax = eax;
                return;
            }
        }
        r.eax = rd(0x2526c7c);
    }
}

// vint_element_sr2_map::post_render: tail jump when [this + 0x9c] == 1
#[inline(always)]
unsafe fn hb_ba6320(r: &mut Pushad, _a: *const u32) {
    unsafe {
        if rd(r.ecx as usize + 0x9c) == 1 {
            let mut s = St::of(r);
            s.call(0x7a37e0, &[]);
            s.put(r);
        }
    }
}

// vint_element_text::get_horz_align: ECX = this, ret 8
#[inline(always)]
unsafe fn hb_ba3820(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.eax = rd(r.ecx as usize + 0xb8);
        s.esi = rd(a as usize);
        let e = s.eax;
        s.call(0xb93590, &[e]);
        let esi = r.esi;
        s.eax = lo(s.eax, 1);
        s.put(r);
        r.esi = esi;
    }
}

// vint_element_video::get_vid_id_handle: ECX = this, ret 8
#[inline(always)]
unsafe fn hb_ba6bb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.eax = rd(r.ecx as usize + 0x98);
        s.esi = rd(a as usize);
        let e = s.eax;
        s.call(0xb93590, &[e]);
        let esi = r.esi;
        s.eax = lo(s.eax, 1);
        s.put(r);
        r.esi = esi;
    }
}

// vint_draw_clip_begin: arg0 = rect pointer
#[inline(always)]
unsafe fn hb_7f0a70(r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = rd(a as usize) as usize;
        let mut s = St::of(r);
        s.eax = p as u32;
        s.ecx = rd(p + 0xc);
        s.edx = rd(p + 4);
        s.esi = rd(p + 8);
        s.edi = rd(p);
        let (c, d) = (s.ecx, s.edx);
        s.call(0xd14d20, &[d, c]);
        let (esi, edi) = (r.esi, r.edi);
        s.put(r);
        r.esi = esi;
        r.edi = edi;
    }
}

// ui_button_image_tag: arg0 = hash (its address is handed to the lookup)
#[inline(always)]
unsafe fn hb_7f36d0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.call(0xc11c00, &[a as u32, 1]);
        if s.eax != 0 {
            s.put(r);
            return;
        }
        let edx = rd(a as usize);
        let mut ecx = 0xe8c980u32;
        let mut eax = 0u32;
        loop {
            if rd(ecx as usize) == edx {
                s.put(r);
                r.eax = rd(((eax << 4).wrapping_add(0xe8c98c)) as usize);
                r.ecx = ecx;
                r.edx = edx;
                return;
            }
            eax += 1;
            ecx += 0x10;
            if eax >= 0x4e {
                break;
            }
        }
        s.put(r);
        r.eax = 0xe25da8;
        r.ecx = ecx;
        r.edx = edx;
    }
}

// control_scheme_data_free
#[inline(always)]
unsafe fn hb_773830(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let esi = rd(0x2528364);
        if esi == 0 {
            return;
        }
        let mut s = St::of(r);
        s.esi = esi;
        s.edi = 0;
        s.call(0xc15720, &[]);
        let z = s.edi;
        wd(0x2528364, z);
        wd(0x2528360, z);
        for k in 0..6 {
            wd(0x1f7a948 + k * 4, z);
        }
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

// widget_activities::get_height / garage_cash_widget::get_height: ST0 = height
#[inline(always)]
unsafe fn height_x87(r: &mut Pushad, global: usize) -> f32 {
    unsafe {
        let mut s = St::of(r);
        s.ecx = rd(global);
        s.call(0xc15d60, &[]);
        let v = (s.eax as i32 as f64) * f64::from_bits(((rd(0xe5c228 + 4) as u64) << 32) | rd(0xe5c228) as u64);
        let h = v as f32;
        s.put(r);
        r.ecx = h.to_bits(); // the slot the original pops into ECX still holds the float
        h
    }
}

#[inline(always)]
unsafe fn hb_77b310(r: &mut Pushad, _a: *const u32) -> f32 {
    unsafe { height_x87(r, 0xe98a80) }
}

#[inline(always)]
unsafe fn hb_552b30(r: &mut Pushad, _a: *const u32) -> f32 {
    unsafe { height_x87(r, 0xe98a7c) }
}

// trigger_multi_registration::free_snapshot: arg0 = object
#[inline(always)]
unsafe fn hb_872140(r: &mut Pushad, a: *const u32) {
    unsafe {
        let o = rd(a as usize);
        let mut s = St::of(r);
        s.edi = o;
        s.esi = rd(o as usize + 0x10);
        s.call(0x872900, &[]);
        let edi = s.edi as usize;
        wd(edi + 0x10, 0);
        wd(edi + 0xc, 0);
        let (oesi, oedi) = (r.esi, r.edi);
        s.eax = 0;
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

// parked_vehicles_quick_update
#[inline(always)]
unsafe fn hb_948d80(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut i = 0u32;
        loop {
            if rb(0x215da14 + i as usize) == 0 {
                r.eax = i;
                return;
            }
            i += 1;
            if i >= 6 {
                break;
            }
        }
        let mut s = St::of(r);
        s.eax = 6;
        s.call(0x9480a0, &[]);
        s.call(0x947f40, &[]);
        s.call(0x948770, &[1]);
        let ecx = r.ecx;
        s.put(r);
        r.ecx = ecx;
    }
}

// vint_loc_lookup_id: calls the localisation hook with the entry ECX as its stack argument
#[inline(always)]
unsafe fn hb_b92200(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let g = rd(0x252a1e0);
        if g == 0 {
            r.eax = 0;
            return;
        }
        let f = rd(g as usize + 4);
        if f == 0 {
            r.eax = 0;
            return;
        }
        let mut s = St::of(r);
        s.eax = f;
        let c = r.ecx;
        s.call(f as usize, &[c]);
        s.put(r);
    }
}

// vint_object_tween::set_target_name: ECX = this, arg0 = name value, ret 8
#[inline(always)]
unsafe fn hb_b9f4b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = rd(a as usize) as usize;
        let this = r.ecx as usize;
        wd(this + 0x84, 0);
        wd(this + 0x88, 0);
        let mut s = St::of(r);
        s.eax = p as u32;
        s.esi = this as u32;
        s.ecx = if rd(p) == 4 { rd(p + 4) } else { 0 };
        s.call(0xbdc9b0, &[]);
        wd(this + 0x8c, s.eax);
        s.eax = lo(s.eax, 1);
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// static_mempool_base::ctor: ESI = this
#[inline(always)]
unsafe fn hb_c00ae0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let this = r.esi as usize;
        wd(this, 0xe58c8c);
        let mut s = St::of(r);
        s.eax = (this + 0xc) as u32;
        s.call(0xc990f0, &[(this + 0xc) as u32, 0xe592b8, 0x20]);
        let t = s.esi as usize;
        wb(t + 0x2b, 0);
        wd(t + 0x30, 0);
        wb(t + 0x2c, 0);
        wd(t + 8, 0);
        wb(t + 4, 0);
        wd(t + 0x38, 0);
        wd(t + 0x40, 0);
        wd(t + 0x3c, 0);
        wd(t, 0xe59484);
        wd(t + 0x48, 0);
        wd(t + 0x44, 0);
        s.eax = t as u32;
        s.put(r);
    }
}

// async_virtual_mempool::alloc: ECX = this, two stack arguments, ret 8
#[inline(always)]
unsafe fn hb_c17cc0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let lock = this.wrapping_add(0x7c);
        let mut s = St::of(r);
        s.esi = this;
        s.edi = lock;
        s.call(rd(0xdb80cc) as usize, &[lock]);
        let (a0, a1) = (rd(a as usize), rd(a as usize + 4));
        s.eax = a1;
        s.ecx = this;
        s.call(0xbfca60, &[a0, a1]);
        let res = s.eax;
        s.esi = res;
        let edi = s.edi;
        s.call(rd(0xdb80d0) as usize, &[edi]);
        s.eax = res;
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

/// The caller's stack as a tail-jumped callee sees it (its own arguments are whatever the caller pushed).
#[inline(always)]
unsafe fn tail_args(a: *const u32) -> [u32; 12] {
    let mut t = [0u32; 12];
    unsafe {
        for (k, v) in t.iter_mut().enumerate() {
            *v = *a.add(k);
        }
    }
    t
}

// barnstorming_diversion_type::init: ECX = this
#[inline(always)]
unsafe fn hb_5f9340(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x602a50, &[]);
        s.edi = rd(0x21703d4);
        let e = s.esi;
        s.call(0x5f8ea0, &[e, 0]);
        wd(s.edi as usize + 0x14cc, s.eax);
        let e = s.esi;
        s.call(0x5f8f90, &[e]);
        let t = s.esi as usize;
        wd(t + 0xc0, 0);
        wd(t + 0xa8, 0);
        wd(t + 0xac, 0);
        wd(t + 0xbc, 0xFFFF_FFFF);
        wd(t + 0xb4, 0xFFFF_FFFF);
        s.eax = 0xFFFF_FFFF;
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

// driver_bailout_diversion_type_info::init: ECX = this
#[inline(always)]
unsafe fn hb_6098d0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x602a50, &[]);
        let t = s.esi as usize;
        wd(t + 0x20, 0xFFFF_FFFF);
        wd(t + 0x18, 0);
        wd(t + 0x4c, 0xFFFF_FFFF);
        s.eax = 0xFFFF_FFFF;
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// stunt_jumping_diversion_type::init: ECX = this
#[inline(always)]
unsafe fn hb_6185d0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x602a50, &[]);
        let t = s.esi as usize;
        wd(t + 0x18, 0xFFFF_FFFF);
        wd(t + 0x24, 0xFFFF_FFFF);
        wd(t + 0x50, 0xFFFF_FFFF);
        wd(t + 0x5c, 0);
        wb(t + 0x68, 0);
        s.eax = 0;
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// taxi_diversion_type_info::init: ECX = this, then a tail jump
#[inline(always)]
unsafe fn hb_628e50(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x602a50, &[]);
        let t = s.esi as usize;
        wb(t + 0xc, 1);
        wd(t + 4, 4);
        s.esi = r.esi;
        s.call(0x627e60, &tail_args(a));
        s.put(r);
    }
}

// windshield_cannon_diversion_type_info::init: ECX = this
#[inline(always)]
unsafe fn hb_63a760(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x602a50, &[]);
        let t = s.esi as usize;
        wd(t + 0x30, 0xFFFF_FFFF);
        wd(t + 0x28, 0);
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// act_packet_sub_49: arg0 = packet
#[inline(always)]
unsafe fn hb_612430(r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = rd(a as usize);
        r.eax = p;
        if rb(p as usize) == 1 {
            wb(0x2527895, 1);
            return;
        }
        if rb(0x2527895) != 0 {
            let mut s = St::of(r);
            s.call(0x610d10, &[]);
            s.call(0x610920, &[]);
            wb(0x2527895, 0);
            s.put(r);
        }
    }
}

// act_packet_sub_4f: asks the object's virtual method 2, then a tail jump
#[inline(always)]
unsafe fn hb_649500(r: &mut Pushad, a: *const u32) {
    unsafe {
        let g = rd(0x2527888);
        if g == 0 {
            r.eax = g;
            return;
        }
        let ecx = rd(g as usize + 8);
        let eax = rd(ecx as usize);
        let edx = rd(eax as usize + 8);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.call(edx as usize, &[]);
        if s.eax == 0x18 {
            let g2 = rd(0x2527888);
            s.ecx = g2;
            if g2 == 0 {
                s.put(r);
                return;
            }
            wb(g2 as usize + 0xa4, 1);
            s.call(0x647ac0, &tail_args(a));
        }
        s.put(r);
    }
}

// act_packet_sub_1f
#[inline(always)]
unsafe fn hb_677bf0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        if rd(0x1408e88) == 0 {
            return;
        }
        let g = rd(0x27b3c60);
        r.eax = g;
        if g == 0 {
            return;
        }
        if rd(g as usize + 0x3c) != 1 {
            return;
        }
        let ecx = rd(g as usize + 0x90);
        let eax = rd(ecx as usize);
        let edx = rd(eax as usize + 4);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.call(edx as usize, &[]);
        if s.eax != 2 {
            s.put(r);
            return;
        }
        let o = rd(0x1408e88);
        s.eax = o;
        if o == 0 {
            // The original stores AL to address 0xFD here and faults; keep the fault.
            std::ptr::write_volatile(0xFDusize as *mut u8, 0);
        }
        let p = rd(o as usize + 0x10);
        s.eax = p;
        wb(p as usize + 0xfd, 0);
        s.put(r);
    }
}

// hud_pkt_sub_10: arg0 = packet (key, flag byte)
#[inline(always)]
unsafe fn hb_79c100(r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = rd(a as usize) as usize;
        let key = rd(p);
        let dl = rb(p + 4);
        r.edx = lo(r.edx, dl);
        r.ecx = key;
        let mut i = 0u32;
        loop {
            if rd(0x29e6bb0 + i as usize * 8) == key {
                break;
            }
            i += 1;
            if (i as i32) >= 0xa {
                r.eax = i;
                return;
            }
        }
        let ent = 0x29e6bb0 + i * 8;
        let flag = (dl != 0) as u32;
        let ecx = lo(key, flag as u8);
        let mut s = St::of(r);
        s.eax = ent;
        s.edx = rd(ent as usize + 4);
        s.ecx = ecx;
        s.call(0x79d770, &[ecx]);
        s.put(r);
        r.ecx = ecx;
    }
}

// firetruck_diversion_type_info::create_instance: arg0 = factory, ret 4
#[inline(always)]
unsafe fn hb_60c020(r: &mut Pushad, a: *const u32) {
    unsafe {
        let ecx = rd(a as usize);
        let eax = rd(ecx as usize);
        let edx = rd(eax as usize + 0x1c);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.call(edx as usize, &[0x1528, 0]);
        if s.eax == 0 {
            s.put(r);
            return;
        }
        let oesi = r.esi;
        s.esi = s.eax;
        s.call(0x60c050, &[]);
        s.put(r);
        r.esi = oesi;
    }
}

// vehicle_protect_objective::get_start_message
#[inline(always)]
unsafe fn hb_575080(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.edi = 0xde88f0;
        s.esi = 0xde8958;
        s.call(0x7f49e0, &[]);
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

// chop_shop_diversion_type::get_start_message / safecracking_diversion_type_info::get_start_message:
// two stack arguments (ESI, EDI for the helper), ret 8
#[inline(always)]
unsafe fn start_message(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = rd(a as usize);
        s.edi = rd(a as usize + 4);
        s.call(0xc14b50, &[0xdc5797]);
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

#[inline(always)]
unsafe fn hb_600f60(r: &mut Pushad, a: *const u32) {
    unsafe { start_message(r, a) }
}

#[inline(always)]
unsafe fn hb_615dd0(r: &mut Pushad, a: *const u32) {
    unsafe { start_message(r, a) }
}

// rig_handle_bone_local_transform: ECX = handle, args (a, b, c), c travels in EAX
#[inline(always)]
unsafe fn hb_cedec0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let ecx = r.ecx as i32;
        if ecx < 0 {
            return;
        }
        let prod = (0xac76_9185u32 as i32 as i64) * (ecx as i64);
        let mut edx = ((prod >> 32) as i32).wrapping_add(ecx);
        edx >>= 7;
        let mut eax = ((edx as u32) >> 31).wrapping_add(edx as u32);
        eax = eax.wrapping_mul(0xbe);
        let ecx2 = (ecx as u32).wrapping_sub(eax).wrapping_mul(0x5e8).wrapping_add(0x340d240);
        if ecx2 == 0 {
            r.eax = eax;
            r.ecx = 0;
            r.edx = edx as u32;
            return;
        }
        let (a0, a1, a2) = (rd(a as usize), rd(a as usize + 4), rd(a as usize + 8));
        let mut s = St::of(r);
        s.eax = a2;
        s.ecx = ecx2;
        s.edx = a1;
        s.call(0xce10f0, &[a0, a1]);
        s.put(r);
    }
}

// rl_container_inst::casts_shadow: ECX = this; asks each element's virtual method 0
#[inline(always)]
unsafe fn hb_cf6160(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let this = r.ecx;
        let mut s = St::of(r);
        s.ebx = this;
        s.esi = 0;
        if rd(this as usize + 8) != 0 {
            s.edi = this + 0xc;
            loop {
                s.ecx = rd(s.edi as usize);
                s.eax = rd(s.ecx as usize);
                s.edx = rd(s.eax as usize);
                s.call(s.edx as usize, &[]);
                if s.eax & 0xFF != 0 {
                    s.eax = lo(s.eax, 1);
                    r.eax = s.eax;
                    r.ecx = s.ecx;
                    r.edx = s.edx;
                    return;
                }
                s.esi += 1;
                s.edi += 4;
                if s.esi >= rd(s.ebx as usize + 8) {
                    break;
                }
            }
        }
        r.eax = lo(s.eax, 0);
        r.ecx = s.ecx;
        r.edx = s.edx;
    }
}

// activity_instance_phase_run_level: ESI = this; jump table keyed on a virtual call's result + 1
#[inline(always)]
unsafe fn hb_53ed30(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let this = r.esi as usize;
        let mut s = St::of(r);
        s.ecx = rd(this + 0x10);
        s.eax = rd(s.ecx as usize);
        s.edx = rd(s.eax as usize + 0xc);
        s.call(s.edx as usize, &[]);
        let idx = s.eax.wrapping_add(1);
        let t = s.esi as usize;
        match idx {
            0 => {
                wb(0x2526d96, 0);
                s.eax = lo(idx, 0);
            }
            1 => {
                let obj = rd(t + 8);
                s.ecx = obj;
                wd(t + 0xc, 6);
                s.eax = idx;
                let mut set1 = true;
                if obj != 0 {
                    s.eax = rd(obj as usize);
                    s.edx = rd(s.eax as usize + 4);
                    s.call(s.edx as usize, &[]);
                    if s.eax == 1 {
                        wd(t + 0xc, 9);
                        wb(t + 0x24, 0);
                        set1 = false;
                    }
                }
                if set1 {
                    wb(t + 0x24, 1);
                }
                s.eax = lo(s.eax, 1);
            }
            2 => {
                wb(t + 0x24, 1);
                s.eax = lo(idx, 1);
            }
            3 => {
                wd(t + 0xc, 7);
                s.eax = lo(idx, 1);
            }
            4 => {
                wb(t + 0x24, 0);
                s.eax = lo(idx, 1);
            }
            _ => {
                s.eax = lo(idx, 0);
            }
        }
        s.put(r);
    }
}

// fuzz_crime::tick_state2: ECX = this
#[inline(always)]
unsafe fn hb_5c2e80(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let this = r.ecx as usize;
        let flags = rd(this + 0xf4);
        r.eax = flags;
        if flags & 0x10 != 0 || flags & 8 == 0 {
            return;
        }
        let mut s = St::of(r);
        s.edi = this as u32;
        s.eax = flags;
        s.ebp = 0;
        if rd(this + 0x98) != 0 {
            s.ebx = (this + 0x3c) as u32;
            loop {
                let e = s.edi as usize;
                let go = if rb(e + s.ebp as usize + 0x78) != 0 {
                    true
                } else {
                    s.eax = rd(e + 0x20);
                    rb(s.eax as usize + 0x80) & 4 != 0
                };
                if go {
                    s.esi = rd(s.ebx as usize);
                    s.call(0x5c54a0, &[]);
                }
                s.ebp = s.ebp.wrapping_add(1);
                s.ebx = s.ebx.wrapping_add(4);
                if s.ebp >= rd(s.edi as usize + 0x98) {
                    break;
                }
            }
        }
        let e = s.edi as usize;
        wd(e + 0xf4, rd(e + 0xf4) | 0x10);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
    }
}

// fuzz_crime_littering_from_car::enter_state3: ECX = this
#[inline(always)]
unsafe fn hb_5c7400(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x5c3020, &[]);
        s.ecx = rd(s.esi as usize + 0xf8);
        s.eax = rd(s.ecx as usize);
        s.edx = rd(s.eax as usize + 0x34);
        s.call(s.edx as usize, &[]);
        if s.eax != 0 {
            let p = s.eax.wrapping_add(0x8554);
            s.ecx = 0;
            s.call(0xb5fb30, &[p, 0]);
        }
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// fuzz_crime_street_racing::enter_state3: ECX = this
#[inline(always)]
unsafe fn hb_5d2d40(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x5c3020, &[]);
        let fl = f32q(rf(0xe87998));
        let this = s.esi as usize;
        let ecx = rd(this + 0xfc);
        let eax = rd(this + 0xf8);
        let mut buf = [eax, ecx];
        s.ecx = ecx;
        s.eax = eax;
        s.edx = buf.as_mut_ptr() as u32;
        let (e, p) = (s.esi, s.edx);
        s.call(0x5c53c0, &[e, p, fl.to_bits()]);
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// fuzz_crime_stolen_heli::update: ECX = this, then a tail jump
#[inline(always)]
unsafe fn hb_5cdd30(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let mut s = St::of(r);
        s.edi = this;
        let oesi = r.esi;
        let oedi = r.edi;
        if rd(this as usize + 0x24) == 3 {
            let obj = rd(this as usize + 0xf8);
            s.ecx = obj;
            let esi: u32;
            if obj != 0 {
                s.eax = rd(obj as usize);
                s.edx = rd(s.eax as usize + 0x34);
                s.call(s.edx as usize, &[]);
                esi = s.eax;
            } else {
                esi = 0;
            }
            s.esi = esi;
            let h = rd(this as usize + 0x100);
            s.eax = h;
            let mut clear = true;
            if h != 0 {
                s.ecx = (h & 0xfff) << 4;
                s.edx = rd(s.ecx as usize + 0x2149c64);
                if s.edx != 0 && rd(s.edx as usize + 0x44) == h && rd(s.edx as usize + 0x4c) == 1 {
                    s.call(0x9855f0, &[]);
                    if s.eax & 0xFF == 0 {
                        clear = false;
                    }
                }
            }
            if clear {
                wd(this as usize + 0x100, 0);
            }
            s.esi = oesi;
            if esi != 0 && rd(this as usize + 0x100) == 0 {
                s.call(0x5cdbf0, &[]);
            }
        }
        s.ecx = this;
        s.edi = oedi;
        s.call(0x5c3bb0, &tail_args(a));
        s.put(r);
    }
}

// cheat_vehicle_drop_update
#[inline(always)]
unsafe fn hb_6875b0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let ecx0 = r.ecx;
        let mut s = St::of(r);
        let mut eax = rd(0xeaa8ac).wrapping_sub(1);
        if eax == 0 {
            s.eax = eax;
            eax = s.call(0x687630, &[]);
        } else {
            eax = eax.wrapping_sub(1);
            if eax != 0 {
                r.eax = eax;
                return;
            }
        }
        s.eax = eax;
        let h = rd(0xeaa8b8);
        s.ecx = h;
        if h != 0 {
            s.eax = rd(((h & 0xfff) << 4) as usize + 0x2149c64);
            let o = s.eax as usize;
            if o != 0 && rd(o + 0x44) == h && rd(o + 0x4c) == 5 && rb(o + 0x84d8) != 0 {
                s.call(0x687750, &[]);
            }
        }
        s.put(r);
        r.ecx = ecx0;
    }
}

// vehicle::get_attach_point: ECX = this, args (out vector, index), ret 8
#[inline(always)]
unsafe fn hb_aa5150(r: &mut Pushad, a: *const u32) {
    unsafe {
        let out = rd(a as usize) as usize;
        let idx = rd(a as usize + 4);
        wf(out, 0.0);
        wf(out + 4, 0.0);
        wf(out + 8, 0.0);
        r.eax = out as u32;
        r.edx = idx;
        if idx == 0xFFFF_FFFF {
            let mut s = St::of(r);
            s.eax = out as u32;
            s.edx = idx;
            s.call(0xaa68e0, &[out as u32]);
            s.put(r);
            return;
        }
        let this = r.ecx as usize;
        if rb(this + 0x84d8) != 1 {
            return;
        }
        let edx = idx.wrapping_mul(9) << 5;
        r.edx = edx;
        let ecx = rd(edx as usize + this + 0x560 + 4) as usize;
        r.ecx = ecx as u32;
        if ecx == 0 {
            return;
        }
        wf(out, f32q(rf(ecx + 0x1a0)));
        wf(out + 4, f32q(rf(ecx + 0x1a4)));
        wf(out + 8, f32q(rf(ecx + 0x1a8)));
    }
}

// mp_switch_modes_impl: AL = mode
#[inline(always)]
unsafe fn hb_841900(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let eax = (r.eax & 0xFF).wrapping_add(0xb);
        r.eax = eax;
        if rd(0xe8b210) == eax {
            return;
        }
        wd(0xe8b210, eax);
        if (eax as i32) > 0xe {
            wd(0xe8b210, 0xFFFF_FFFF);
            *(0xe8b214usize as *mut u16) = 0xFFFF;
            wd(0x2528d94, 0xFFFF_FFFF);
            r.eax = 0xFFFF_FFFF;
            return;
        }
        let mut s = St::of(r);
        s.eax = eax;
        s.call(0x83f460, &[]);
        s.eax = 0xFFFF_FFFF;
        *(0xe8b214usize as *mut u16) = 0xFFFF;
        wd(0x2528d94, 0xFFFF_FFFF);
        s.eax = rd(0xe8b210);
        s.ecx = 0;
        s.call(0x89bb30, &[]);
        s.edi = s.eax;
        s.esi = 0x2a4d120;
        s.call(0x89b500, &[]);
        let (oesi, oedi) = (r.esi, r.edi);
        s.put(r);
        r.esi = oesi;
        r.edi = oedi;
    }
}

// pc_multi_session_info::ctor: ESI = this
#[inline(always)]
unsafe fn hb_810da0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let t = r.esi as usize;
        let edi = (t + 0x59) as u32;
        wd(t, 0xdffd88);
        wd(t + 0x30, 0);
        wb(t + 0x34, 0);
        wd(t + 0x38, 0);
        wd(t + 0x3c, 1);
        for o in [0x41usize, 0x42, 0x43, 0x44, 0x46, 0x47, 0x48, 0x49] {
            wb(t + o, 0);
        }
        let mut s = St::of(r);
        s.ebx = 0;
        s.edi = edi;
        s.call(0xca0bc0, &[edi, 0, 0x29]);
        wb(s.edi as usize, 0);
        s.eax = s.esi;
        let (oebx, oedi) = (r.ebx, r.edi);
        s.put(r);
        r.ebx = oebx;
        r.edi = oedi;
    }
}

// havok_filter_shape_collection::ctor: ESI = this, EDI = count, EAX passes through; arg0 = source list, ret 4
#[inline(always)]
unsafe fn hb_70e9b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let src = rd(a as usize);
        let mut s = St::of(r);
        let (e_in, edi) = (r.eax, r.edi);
        s.eax = s.esi;
        s.call(0xd41d20, &[e_in, edi]);
        let this = s.esi as usize;
        let cnt = s.edi as i32;
        s.eax = 0;
        wd(this, 0xe18c30);
        wd(this + 0x10, 0xe18c58);
        if cnt > 0 {
            let mut ecx = 0u32;
            let mut i = 0u32;
            loop {
                let t = rd(src as usize);
                s.edx = rd(t as usize + i as usize * 4);
                let ebp = rd(this + 0x18);
                wd(ecx.wrapping_add(ebp).wrapping_add(4) as usize, s.edx);
                i += 1;
                ecx = ecx.wrapping_add(0x10);
                if (i as i32) >= cnt {
                    break;
                }
            }
            s.ecx = ecx;
        }
        s.eax = this as u32;
        let (oebx, oebp) = (r.ebx, r.ebp);
        s.put(r);
        r.ebx = oebx;
        r.ebp = oebp;
    }
}

// vint_element_base_init_defaults: ECX = this
#[inline(always)]
unsafe fn hb_b95a60(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        s.call(0xb97220, &[]);
        if s.eax & 0xFF == 0 {
            s.put(r);
            return;
        }
        let t = s.ecx as usize;
        for o in [0x5cusize, 0x60, 0x64, 0x68, 0x78] {
            wf(t + o, 0.0);
        }
        for o in [0x70usize, 0x74, 0x7c, 0x80, 0x84, 0x88, 0x90, 0x94] {
            wf(t + o, 1.0);
        }
        wd(t + 0x6c, 0);
        wb(t + 0x58, 0);
        wd(t + 0x54, 0);
        wb(t + 0x59, 1);
        wd(t + 0x8c, 0);
        s.eax = 1;
        s.put(r);
    }
}

// vint_type_find_by_name: arg0 = name; walks the type table (16-byte entries)
#[inline(always)]
unsafe fn hb_b9d780(r: &mut Pushad, a: *const u32) {
    unsafe {
        let name = rd(a as usize);
        r.eax = 0;
        if rd(0x252a270) == 0 {
            return;
        }
        let mut s = St::of(r);
        s.ebx = 0;
        s.ebp = name;
        let mut i = 0u32;
        let mut edi = 0x22f2370u32;
        loop {
            let p = rd(edi as usize);
            s.eax = p;
            s.esi = i;
            s.edi = edi;
            if p != 0 {
                s.call(0xc99030, &[name, p]);
                if s.eax == 0 {
                    s.eax = (i << 4).wrapping_add(0x22f2370);
                    r.eax = s.eax;
                    r.ecx = s.ecx;
                    r.edx = s.edx;
                    return;
                }
            }
            i += 1;
            edi = edi.wrapping_add(0x10);
            if i >= rd(0x252a270) {
                break;
            }
        }
        r.eax = 0;
        r.ecx = s.ecx;
        r.edx = s.edx;
    }
}

// pcu_worn_remove_by_slot: args (owner, slot, extra); returns how many were removed
#[inline(always)]
unsafe fn hb_7c20b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let (a0, a1, a2) = (rd(a as usize), rd(a as usize + 4), rd(a as usize + 8));
        let idx = rd(a0 as usize + 0x10);
        let mut s = St::of(r);
        let mut removed = 0u32;
        let mut i = 0u32;
        if (rd(idx as usize * 4 + 0x1fac828) as i32) > 0 {
            let mut edi = idx.wrapping_mul(0x1720).wrapping_add(0x1fac868);
            loop {
                s.ecx = rd(edi as usize);
                s.edx = rb(s.ecx as usize + 0x15) as i8 as i32 as u32;
                if s.edx == a1 {
                    s.eax = a2;
                    s.ecx = a0;
                    s.call(0x7c2000, &[a0, i, a2]);
                    removed += 1;
                } else {
                    i += 1;
                    edi = edi.wrapping_add(0x50);
                }
                if (i as i32) >= (rd(idx as usize * 4 + 0x1fac828) as i32) {
                    break;
                }
            }
        }
        s.eax = removed;
        let (oebx, oesi, oedi, oebp) = (r.ebx, r.esi, r.edi, r.ebp);
        s.put(r);
        r.ebx = oebx;
        r.esi = oesi;
        r.edi = oedi;
        r.ebp = oebp;
    }
}

const IMP_ENTER: usize = 0x00DB_80CC;
const IMP_LEAVE: usize = 0x00DB_80D0;
const LOCK_ARG: u32 = 0x0347_ADE4;
const NO_FREE_FLAG: usize = 0x0252_7BF7;

/// The `delete` tail of a scalar deleting destructor, with the registers the original has (AL = the
/// flag byte read from the no-free flag, EAX = the object for the free call).
#[inline(always)]
unsafe fn sdd_tail(s: &mut St, this: u32) {
    unsafe {
        let flag = rb(NO_FREE_FLAG);
        if flag == 1 {
            return;
        }
        s.eax = lo(s.eax, flag);
        s.call(rd(IMP_ENTER) as usize, &[LOCK_ARG]);
        s.eax = this;
        s.call(0xcde870, &[]);
        s.call(rd(IMP_LEAVE) as usize, &[LOCK_ARG]);
    }
}

// ctype<>::scalar_deleting_dtor: ECX = this, arg0 = flags, ret 4
#[inline(always)]
unsafe fn hb_4e41c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let mut s = St::of(r);
        s.esi = this;
        wd(this as usize, 0xdde230);
        s.call(0x4e3fd0, &[]);
        let del = rb(a as usize) & 1 != 0;
        wd(this as usize, 0xdbf220);
        if del {
            sdd_tail(&mut s, this);
        }
        s.eax = this;
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// basic_ifstream<>::scalar_deleting_dtor: ECX = this + 0x58, arg0 = flags, ret 4
#[inline(always)]
unsafe fn hb_4ec3d0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx.wrapping_sub(0x58);
        let mut s = St::of(r);
        s.esi = this;
        s.eax = this;
        s.call(0x4ea370, &[]);
        let del = rb(a as usize) & 1 != 0;
        if del && this != 0 {
            sdd_tail(&mut s, this);
        }
        s.eax = this;
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// vint_callback_native::scalar_deleting_dtor: ECX = this, arg0 = flags, ret 4
#[inline(always)]
unsafe fn hb_ba0740(r: &mut Pushad, a: *const u32) {
    unsafe {
        let this = r.ecx;
        let del = rb(a as usize) & 1 != 0;
        wd(this as usize, 0xe4cebc);
        let mut s = St::of(r);
        if del {
            sdd_tail(&mut s, this);
        }
        s.eax = this;
        s.put(r);
    }
}

#[inline(always)]
unsafe fn reg_type(r: &mut Pushad, g: usize, obj: u32, vt0: u32, id: u32, vt1: u32, idx_g: usize, f: usize, last: u32) {
    unsafe {
        let o = obj as usize;
        let ecx = rd(rd(g) as usize);
        wd(o + 8, ecx);
        wd(o, vt0);
        wd(o + 4, id);
        let mut s = St::of(r);
        s.eax = rd(g);
        s.ecx = ecx;
        s.esi = obj;
        s.call(0xb982e0, &[]);
        s.ecx = rd(0x252a270);
        s.edx = rd(g);
        s.esi = rd(s.edx as usize);
        if s.ecx < 0x20 {
            let e = (s.ecx << 4) as usize;
            s.edx = s.ecx;
            s.ecx += 1;
            s.eax = e as u32;
            wd(e + 0x22f2370, vt1);
            wd(e + 0x22f2374, s.esi);
            wd(e + 0x22f2378, obj);
            wd(0x252a270, s.ecx);
        } else {
            s.edx = 0xFFFF_FFFF;
        }
        wd(idx_g, s.edx);
        s.call(f, &[]);
        s.eax = rd(idx_g);
        if s.eax < rd(0x252a270) {
            s.eax <<= 4;
            wd(s.eax as usize + 0x22f237c, last);
        }
        s.eax = lo(s.eax, 1);
        let oesi = r.esi;
        s.put(r);
        r.esi = oesi;
    }
}

// vint_element_gradient_register_type
#[inline(always)]
unsafe fn hb_ba1920(r: &mut Pushad, _a: *const u32) {
    unsafe { reg_type(r, 0xebe3f4, 0x31bf13c, 0xebe410, 8, 0xe4d9b8, 0xe86cc8, 0xba23d0, 0xebe534) }
}

// vint_element_point_register_type
#[inline(always)]
unsafe fn hb_ba4720(r: &mut Pushad, _a: *const u32) {
    unsafe { reg_type(r, 0xebdf6c, 0x31bf154, 0xebe7c4, 1, 0xde01d0, 0xe86c78, 0xba4920, 0xebe7ec) }
}

// cutscene_action_team_despawn: arg0 = action
#[inline(always)]
unsafe fn hb_6cc4f0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let arg = rd(a as usize);
        let esi = rd(arg as usize + 8);
        r.eax = arg;
        if esi == 0 {
            return;
        }
        let oesi = r.esi;
        // The original's one-byte local is the low byte of its own argument slot.
        wb(a as usize, 0);
        let loc = a as *const u8;
        let mut s = St::of(r);
        s.esi = esi;
        s.eax = a as u32;
        s.ecx = esi;
        s.call(0xa52bb0, &[]);
        if std::ptr::read_volatile(loc) == 0 {
            s.call(0xa52c70, &[]);
        }
        if s.eax != 0 {
            let e = rb(s.eax as usize + 0x111c) as i8 as i32 as u32;
            s.esi = e;
            s.call(0xae4a00, &[e, 1]);
            s.call(0x964660, &[e, 1, e, 1]);
        }
        s.put(r);
        r.esi = oesi;
    }
}

/// The thread's TLS pointer array (`fs:[0x2c]`).
#[inline(always)]
unsafe fn tls_array() -> u32 {
    let v: u32;
    unsafe {
        std::arch::asm!("mov {}, fs:[0x2c]", out(reg) v, options(nostack, readonly, preserves_flags));
    }
    v
}

// spawn_system_init
#[inline(always)]
unsafe fn hb_93ce20(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let ecx0 = r.ecx;
        let mut s = St::of(r);
        s.call(0x943230, &[]);
        *(0x215da28usize as *mut u16) = 0x0101;
        wd(0x215da24, 0x0101_0101);
        wd(0x215da1c, 0x0101_0101);
        *(0x215da20usize as *mut u16) = 0x0101;
        wd(0xeb7a80, 0xFFFF_FFFF);
        let mut i = 0u32;
        loop {
            if rb(0x215da1c + i as usize) == 0 {
                s.eax = i;
                wd(0xeb7a80, 0xFFFF_FFFF);
                break;
            }
            i += 1;
            if i >= 6 {
                s.eax = i;
                s.call(0x93c610, &[]);
                break;
            }
        }
        s.eax = 0x0101_0101;
        wd(0x215da14, s.eax);
        *(0x215da18usize as *mut u16) = 0x0101;
        wd(0x215da0c, s.eax);
        *(0x215da10usize as *mut u16) = 0x0101;
        wd(0x215da04, s.eax);
        *(0x215da08usize as *mut u16) = 0x0101;
        s.call(0xb7da30, &[]);
        s.eax = 0x0101_0101;
        wd(0x215d9fc, s.eax);
        *(0x215da00usize as *mut u16) = 0x0101;
        s.call(0xb81130, &[]);
        wb(0xe8857f, 1);
        s.call(0x946610, &[]);
        wd(0xeb7a98, 0xFFFF_FFFF);
        s.put(r);
        r.ecx = ecx0;
    }
}

// vint_document_destroy: EAX = document, then a tail jump
#[inline(always)]
unsafe fn hb_b8a980(r: &mut Pushad, a: *const u32) {
    unsafe {
        let doc = r.eax;
        let mut s = St::of(r);
        s.esi = doc;
        s.call(0xb91400, &[doc]);
        let f = rd(0x252a1ec);
        s.eax = f;
        if f != 0 {
            let e = s.esi;
            s.call(f as usize, &[e]);
        }
        s.call(0xb8a9f0, &[]);
        s.eax = s.esi;
        s.esi = r.esi;
        s.call(0xb8abd0, &tail_args(a));
        s.put(r);
    }
}

// im2d_c_end
#[inline(always)]
unsafe fn hb_d0a4b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        let mut s = St::of(r);
        let mut tail = true;
        if rb(0xe849db) == 0 {
            s.eax = rd(tls_array() as usize);
            let o = s.eax as usize;
            let v = rd(o + 0x60);
            if (rb(o + 0x5c) == 0 && v == 0xec26a0) || v == 0xec2610 {
                tail = false;
            }
        }
        if tail {
            s.call(0xd1e590, &tail_args(a));
        } else {
            s.call(0xcf8ea0, &[0x1a]);
        }
        s.put(r);
    }
}

// gfx_transform_apply
#[inline(always)]
unsafe fn hb_d229b0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        let oesi = r.esi;
        let mut s = St::of(r);
        s.call(0xd22790, &[]);
        s.ecx = rd(s.eax as usize + 0x1554).wrapping_mul(0x144);
        s.esi = s.ecx.wrapping_add(s.eax).wrapping_sub(0x30);
        let mut call = true;
        if rb(0xe849db) == 0 {
            s.edx = tls_array();
            s.eax = rd(s.edx as usize);
            let o = s.eax as usize;
            let v = rd(o + 0x60);
            if (rb(o + 0x5c) == 0 && v == 0xec26a0) || v == 0xec2610 {
                call = false;
            }
        }
        if call {
            s.call(0xd18ee0, &[]);
        }
        s.put(r);
        r.esi = oesi;
    }
}

const COUNT: usize = 57;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
const B_495950: [u8; 68] = [0x8B, 0x0D, 0x60, 0x5D, 0x5F, 0x02, 0xA1, 0x5C, 0x5D, 0x5F, 0x02, 0x8B, 0x15, 0x64, 0x5D, 0x5F, 0x02, 0x56, 0x57, 0x89, 0x0D, 0x24, 0x5B, 0x5F, 0x02, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xBE, 0x68, 0x5D, 0x5F, 0x02, 0xBF, 0x5C, 0x5B, 0x5F, 0x02, 0xA3, 0x20, 0x5B, 0x5F, 0x02, 0x89, 0x15, 0x28, 0x5B, 0x5F, 0x02, 0xF3, 0xA5, 0xD9, 0x05, 0x8C, 0x5D, 0x5F, 0x02, 0xD9, 0x1D, 0xA8, 0x5B, 0x5F, 0x02, 0x5F, 0x5E, 0xC3];
extern "C" fn h_495950(r: &mut Pushad, a: *const u32) {
    count(0);
    unsafe { hb_495950(r, a) }
}
thunk!(t_495950, h_495950, "ret");
const B_4A0140: [u8; 13] = [0xD9, 0x05, 0x48, 0x34, 0xDD, 0x00, 0xD9, 0x1D, 0xA8, 0x5B, 0x5F, 0x02, 0xC3];
extern "C" fn h_4a0140(r: &mut Pushad, a: *const u32) {
    count(1);
    unsafe { hb_4a0140(r, a) }
}
thunk!(t_4a0140, h_4a0140, "ret");
const B_4AE280: [u8; 68] = [0x8B, 0x10, 0x56, 0x8B, 0x34, 0x0A, 0x83, 0xC1, 0x04, 0x89, 0x70, 0x7C, 0x8D, 0x34, 0x0A, 0x03, 0x48, 0x7C, 0xF6, 0x40, 0x1C, 0x13, 0x89, 0x70, 0x78, 0x8B, 0x34, 0x0A, 0x8D, 0x4C, 0x0A, 0x04, 0x89, 0xB0, 0x84, 0x00, 0x00, 0x00, 0x89, 0x88, 0x80, 0x00, 0x00, 0x00, 0x5E, 0x74, 0x14, 0x33, 0xC9, 0x89, 0x48, 0x7C, 0x89, 0x48, 0x78, 0x89, 0x88, 0x84, 0x00, 0x00, 0x00, 0x89, 0x88, 0x80, 0x00, 0x00, 0x00, 0xC3];
extern "C" fn h_4ae280(r: &mut Pushad, a: *const u32) {
    count(2);
    unsafe { hb_4ae280(r, a) }
}
thunk!(t_4ae280, h_4ae280, "ret");
const B_4AEF60: [u8; 49] = [0x8B, 0xC6, 0x25, 0x03, 0x00, 0x00, 0x80, 0x79, 0x05, 0x48, 0x83, 0xC8, 0xFC, 0x40, 0x75, 0x04, 0x33, 0xD2, 0xEB, 0x07, 0xBA, 0x04, 0x00, 0x00, 0x00, 0x2B, 0xD0, 0x8B, 0x41, 0x48, 0x89, 0x81, 0x30, 0x02, 0x00, 0x00, 0x8B, 0x01, 0x03, 0xC2, 0x03, 0xC6, 0x89, 0x81, 0x34, 0x02, 0x00, 0x00, 0xC3];
extern "C" fn h_4aef60(r: &mut Pushad, a: *const u32) {
    count(3);
    unsafe { hb_4aef60(r, a) }
}
thunk!(t_4aef60, h_4aef60, "ret");
const B_A58AB0: [u8; 107] = [0x8B, 0xC1, 0xC7, 0x00, 0xBC, 0xE3, 0xE3, 0x00, 0x8B, 0x0D, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x48, 0x0C, 0x8B, 0x15, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x50, 0x0C, 0x33, 0xD2, 0xC7, 0x40, 0x04, 0x40, 0x29, 0xA6, 0x00, 0x89, 0x50, 0x10, 0x89, 0x50, 0x08, 0x88, 0x50, 0x14, 0xC6, 0x40, 0x15, 0xFF, 0xC7, 0x00, 0xE0, 0x30, 0xE4, 0x00, 0x8B, 0x0D, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x48, 0x40, 0x83, 0xC9, 0xFF, 0x89, 0x48, 0x38, 0x89, 0x48, 0x3C, 0x56, 0x8B, 0x35, 0x08, 0x63, 0x3D, 0x03, 0x89, 0x70, 0x40, 0xC7, 0x40, 0x04, 0x60, 0x29, 0xA6, 0x00, 0x89, 0x48, 0x20, 0xC7, 0x40, 0x28, 0xFF, 0xFF, 0x00, 0x00, 0x89, 0x50, 0x24, 0x89, 0x50, 0x2C, 0x5E, 0xC3];
extern "C" fn h_a58ab0(r: &mut Pushad, a: *const u32) {
    count(4);
    unsafe { hb_a58ab0(r, a) }
}
thunk!(t_a58ab0, h_a58ab0, "ret");
const B_B800C0: [u8; 112] = [0x8A, 0x40, 0x29, 0x3C, 0x40, 0x72, 0x15, 0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02, 0x80, 0xF9, 0x01, 0x75, 0x58, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xD2, 0xEB, 0x43, 0x0F, 0xB6, 0xC0, 0x3B, 0x05, 0xB0, 0x6C, 0x52, 0x02, 0x7D, 0x27, 0x8B, 0x04, 0x85, 0xE0, 0x39, 0xF8, 0x00, 0x85, 0xC0, 0x7C, 0x1C, 0x3B, 0x05, 0x90, 0x6C, 0x52, 0x02, 0x7D, 0x14, 0x69, 0xC0, 0xA8, 0x00, 0x00, 0x00, 0x03, 0x05, 0x94, 0x6C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x04, 0x8B, 0x40, 0x74, 0xC3, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x75, 0x13, 0x8A, 0x0D, 0x49, 0x8B, 0x52, 0x02, 0x84, 0xC9, 0x75, 0x09, 0xA1, 0x84, 0x6C, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x05, 0xA1, 0x7C, 0x6C, 0x52, 0x02, 0xC3];
extern "C" fn h_b800c0(r: &mut Pushad, a: *const u32) {
    count(5);
    unsafe { hb_b800c0(r, a) }
}
thunk!(t_b800c0, h_b800c0, "ret");
const B_BA6320: [u8; 15] = [0x83, 0xB9, 0x9C, 0x00, 0x00, 0x00, 0x01, 0x75, 0x05, 0xE9, 0xB2, 0xD4, 0xBF, 0xFF, 0xC3];
extern "C" fn h_ba6320(r: &mut Pushad, a: *const u32) {
    count(6);
    unsafe { hb_ba6320(r, a) }
}
thunk!(t_ba6320, h_ba6320, "ret");
const B_BA3820: [u8; 23] = [0x8B, 0x81, 0xB8, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x5F, 0xFD, 0xFE, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_ba3820(r: &mut Pushad, a: *const u32) {
    count(7);
    unsafe { hb_ba3820(r, a) }
}
thunk!(t_ba3820, h_ba3820, "ret 8");
const B_BA6BB0: [u8; 23] = [0x8B, 0x81, 0x98, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0xCF, 0xC9, 0xFE, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_ba6bb0(r: &mut Pushad, a: *const u32) {
    count(8);
    unsafe { hb_ba6bb0(r, a) }
}
thunk!(t_ba6bb0, h_ba6bb0, "ret 8");
const B_7F0A70: [u8; 30] = [0x8B, 0x44, 0x24, 0x04, 0x8B, 0x48, 0x0C, 0x8B, 0x50, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x57, 0x8B, 0x38, 0x51, 0x52, 0xE8, 0x98, 0x42, 0x52, 0x00, 0x83, 0xC4, 0x08, 0x5F, 0x5E, 0xC3];
extern "C" fn h_7f0a70(r: &mut Pushad, a: *const u32) {
    count(9);
    unsafe { hb_7f0a70(r, a) }
}
thunk!(t_7f0a70, h_7f0a70, "ret");
const B_7F36D0: [u8; 63] = [0x8D, 0x44, 0x24, 0x04, 0x6A, 0x01, 0x50, 0xE8, 0x24, 0xE5, 0x41, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0x21, 0x8B, 0x54, 0x24, 0x04, 0xB9, 0x80, 0xC9, 0xE8, 0x00, 0x8D, 0x64, 0x24, 0x00, 0x39, 0x11, 0x74, 0x11, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x10, 0x83, 0xF8, 0x4E, 0x72, 0xF1, 0xB8, 0xA8, 0x5D, 0xE2, 0x00, 0xC3, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x8C, 0xC9, 0xE8, 0x00, 0xC3];
extern "C" fn h_7f36d0(r: &mut Pushad, a: *const u32) {
    count(10);
    unsafe { hb_7f36d0(r, a) }
}
thunk!(t_7f36d0, h_7f36d0, "ret");
const B_773830: [u8; 70] = [0x56, 0x8B, 0x35, 0x64, 0x83, 0x52, 0x02, 0x57, 0x33, 0xFF, 0x3B, 0xF7, 0x74, 0x35, 0xE8, 0xDD, 0x1E, 0x4A, 0x00, 0x89, 0x3D, 0x64, 0x83, 0x52, 0x02, 0x89, 0x3D, 0x60, 0x83, 0x52, 0x02, 0x89, 0x3D, 0x48, 0xA9, 0xF7, 0x01, 0x89, 0x3D, 0x4C, 0xA9, 0xF7, 0x01, 0x89, 0x3D, 0x50, 0xA9, 0xF7, 0x01, 0x89, 0x3D, 0x54, 0xA9, 0xF7, 0x01, 0x89, 0x3D, 0x58, 0xA9, 0xF7, 0x01, 0x89, 0x3D, 0x5C, 0xA9, 0xF7, 0x01, 0x5F, 0x5E, 0xC3];
extern "C" fn h_773830(r: &mut Pushad, a: *const u32) {
    count(11);
    unsafe { hb_773830(r, a) }
}
thunk!(t_773830, h_773830, "ret");
const B_77B310: [u8; 34] = [0x51, 0x8B, 0x0D, 0x80, 0x8A, 0xE9, 0x00, 0xE8, 0x44, 0xAA, 0x49, 0x00, 0x89, 0x04, 0x24, 0xDB, 0x04, 0x24, 0xDC, 0x0D, 0x28, 0xC2, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00];
extern "C" fn h_77b310(r: &mut Pushad, a: *const u32) -> f32 {
    count(12);
    unsafe { hb_77b310(r, a) }
}
thunk!(t_77b310, h_77b310, "ret 4");
const B_552B30: [u8; 34] = [0x51, 0x8B, 0x0D, 0x7C, 0x8A, 0xE9, 0x00, 0xE8, 0x24, 0x32, 0x6C, 0x00, 0x89, 0x04, 0x24, 0xDB, 0x04, 0x24, 0xDC, 0x0D, 0x28, 0xC2, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00];
extern "C" fn h_552b30(r: &mut Pushad, a: *const u32) -> f32 {
    count(13);
    unsafe { hb_552b30(r, a) }
}
thunk!(t_552b30, h_552b30, "ret 4");
const B_872140: [u8; 27] = [0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x77, 0x10, 0xE8, 0xB2, 0x07, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x47, 0x10, 0x89, 0x47, 0x0C, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
extern "C" fn h_872140(r: &mut Pushad, a: *const u32) {
    count(14);
    unsafe { hb_872140(r, a) }
}
thunk!(t_872140, h_872140, "ret 4");
const B_948D80: [u8; 42] = [0x51, 0x33, 0xC0, 0x80, 0xB8, 0x14, 0xDA, 0x15, 0x02, 0x00, 0x74, 0x1C, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x06, 0x72, 0xEF, 0xE8, 0x07, 0xF3, 0xFF, 0xFF, 0xE8, 0xA2, 0xF1, 0xFF, 0xFF, 0x6A, 0x01, 0xE8, 0xCB, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x59, 0xC3];
extern "C" fn h_948d80(r: &mut Pushad, a: *const u32) {
    count(15);
    unsafe { hb_948d80(r, a) }
}
thunk!(t_948d80, h_948d80, "ret");
const B_B92200: [u8; 26] = [0xA1, 0xE0, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x0E, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0x07, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xC3, 0x33, 0xC0, 0xC3];
extern "C" fn h_b92200(r: &mut Pushad, a: *const u32) {
    count(16);
    unsafe { hb_b92200(r, a) }
}
thunk!(t_b92200, h_b92200, "ret");
const B_B9F4B0: [u8; 46] = [0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0xF1, 0x33, 0xC9, 0x89, 0x8E, 0x84, 0x00, 0x00, 0x00, 0x89, 0x8E, 0x88, 0x00, 0x00, 0x00, 0x83, 0x38, 0x04, 0x75, 0x03, 0x8B, 0x48, 0x04, 0xE8, 0xDE, 0xD4, 0x03, 0x00, 0x89, 0x86, 0x8C, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_b9f4b0(r: &mut Pushad, a: *const u32) {
    count(17);
    unsafe { hb_b9f4b0(r, a) }
}
thunk!(t_b9f4b0, h_b9f4b0, "ret 8");
const B_C00AE0: [u8; 66] = [0x6A, 0x20, 0x8D, 0x46, 0x0C, 0x68, 0xB8, 0x92, 0xE5, 0x00, 0x50, 0xC7, 0x06, 0x8C, 0x8C, 0xE5, 0x00, 0xE8, 0xFA, 0x85, 0x09, 0x00, 0x33, 0xC0, 0x88, 0x46, 0x2B, 0x89, 0x46, 0x30, 0x88, 0x46, 0x2C, 0x89, 0x46, 0x08, 0x88, 0x46, 0x04, 0x89, 0x46, 0x38, 0x89, 0x46, 0x40, 0x89, 0x46, 0x3C, 0xC7, 0x06, 0x84, 0x94, 0xE5, 0x00, 0x89, 0x46, 0x48, 0x89, 0x46, 0x44, 0x83, 0xC4, 0x0C, 0x8B, 0xC6, 0xC3];
extern "C" fn h_c00ae0(r: &mut Pushad, a: *const u32) {
    count(18);
    unsafe { hb_c00ae0(r, a) }
}
thunk!(t_c00ae0, h_c00ae0, "ret");
const B_C17CC0: [u8; 47] = [0x56, 0x57, 0x8B, 0xF1, 0x8D, 0x7E, 0x7C, 0x57, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x0C, 0x50, 0x51, 0x8B, 0xCE, 0xE8, 0x81, 0x4D, 0xFE, 0xFF, 0x57, 0x8B, 0xF0, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x5F, 0x8B, 0xC6, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_c17cc0(r: &mut Pushad, a: *const u32) {
    count(19);
    unsafe { hb_c17cc0(r, a) }
}
thunk!(t_c17cc0, h_c17cc0, "ret 8");
const B_5F9340: [u8; 83] = [0x56, 0x57, 0x8B, 0xF1, 0xE8, 0x07, 0x97, 0x00, 0x00, 0x8B, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x6A, 0x00, 0x56, 0xE8, 0x49, 0xFB, 0xFF, 0xFF, 0x56, 0x89, 0x87, 0xCC, 0x14, 0x00, 0x00, 0xE8, 0x2D, 0xFC, 0xFF, 0xFF, 0x83, 0xC8, 0xFF, 0x5F, 0xC7, 0x86, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x86, 0xA8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x86, 0xAC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x89, 0x86, 0xBC, 0x00, 0x00, 0x00, 0x89, 0x86, 0xB4, 0x00, 0x00, 0x00, 0x5E, 0xC3];
extern "C" fn h_5f9340(r: &mut Pushad, a: *const u32) {
    count(20);
    unsafe { hb_5f9340(r, a) }
}
thunk!(t_5f9340, h_5f9340, "ret");
const B_6098D0: [u8; 26] = [0x56, 0x8B, 0xF1, 0xE8, 0x78, 0x91, 0xFF, 0xFF, 0x83, 0xC8, 0xFF, 0x89, 0x46, 0x20, 0xC7, 0x46, 0x18, 0x00, 0x00, 0x00, 0x00, 0x89, 0x46, 0x4C, 0x5E, 0xC3];
extern "C" fn h_6098d0(r: &mut Pushad, a: *const u32) {
    count(21);
    unsafe { hb_6098d0(r, a) }
}
thunk!(t_6098d0, h_6098d0, "ret");
const B_6185D0: [u8; 30] = [0x56, 0x8B, 0xF1, 0xE8, 0x78, 0xA4, 0xFE, 0xFF, 0x83, 0xC8, 0xFF, 0x89, 0x46, 0x18, 0x89, 0x46, 0x24, 0x89, 0x46, 0x50, 0x33, 0xC0, 0x89, 0x46, 0x5C, 0x88, 0x46, 0x68, 0x5E, 0xC3];
extern "C" fn h_6185d0(r: &mut Pushad, a: *const u32) {
    count(22);
    unsafe { hb_6185d0(r, a) }
}
thunk!(t_6185d0, h_6185d0, "ret");
const B_628E50: [u8; 25] = [0x56, 0x8B, 0xF1, 0xE8, 0xF8, 0x9B, 0xFD, 0xFF, 0xC6, 0x46, 0x0C, 0x01, 0xC7, 0x46, 0x04, 0x04, 0x00, 0x00, 0x00, 0x5E, 0xE9, 0xF7, 0xEF, 0xFF, 0xFF];
extern "C" fn h_628e50(r: &mut Pushad, a: *const u32) {
    count(23);
    unsafe { hb_628e50(r, a) }
}
thunk!(t_628e50, h_628e50, "ret");
const B_63A760: [u8; 24] = [0x56, 0x8B, 0xF1, 0xE8, 0xE8, 0x82, 0xFC, 0xFF, 0xC7, 0x46, 0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3];
extern "C" fn h_63a760(r: &mut Pushad, a: *const u32) {
    count(24);
    unsafe { hb_63a760(r, a) }
}
thunk!(t_63a760, h_63a760, "ret");
const B_612430: [u8; 44] = [0x8B, 0x44, 0x24, 0x04, 0x80, 0x38, 0x01, 0x75, 0x08, 0xC6, 0x05, 0x95, 0x78, 0x52, 0x02, 0x01, 0xC3, 0x80, 0x3D, 0x95, 0x78, 0x52, 0x02, 0x00, 0x74, 0x11, 0xE8, 0xC1, 0xE8, 0xFF, 0xFF, 0xE8, 0xCC, 0xE4, 0xFF, 0xFF, 0xC6, 0x05, 0x95, 0x78, 0x52, 0x02, 0x00, 0xC3];
extern "C" fn h_612430(r: &mut Pushad, a: *const u32) {
    count(25);
    unsafe { hb_612430(r, a) }
}
thunk!(t_612430, h_612430, "ret");
const B_649500: [u8; 47] = [0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50, 0x08, 0xFF, 0xD2, 0x83, 0xF8, 0x18, 0x75, 0x16, 0x8B, 0x0D, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x0C, 0xC6, 0x81, 0xA4, 0x00, 0x00, 0x00, 0x01, 0xE9, 0x92, 0xE5, 0xFF, 0xFF, 0xC3];
extern "C" fn h_649500(r: &mut Pushad, a: *const u32) {
    count(26);
    unsafe { hb_649500(r, a) }
}
thunk!(t_649500, h_649500, "ret");
const B_677BF0: [u8; 69] = [0x83, 0x3D, 0x88, 0x8E, 0x40, 0x01, 0x00, 0x74, 0x3B, 0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74, 0x32, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x2C, 0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0x83, 0xF8, 0x02, 0x75, 0x1A, 0xA1, 0x88, 0x8E, 0x40, 0x01, 0x85, 0xC0, 0x75, 0x07, 0x88, 0x80, 0xFD, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x40, 0x10, 0xC6, 0x80, 0xFD, 0x00, 0x00, 0x00, 0x00, 0xC3];
extern "C" fn h_677bf0(r: &mut Pushad, a: *const u32) {
    count(27);
    unsafe { hb_677bf0(r, a) }
}
thunk!(t_677bf0, h_677bf0, "ret");
const B_79C100: [u8; 61] = [0x8B, 0x44, 0x24, 0x04, 0x8B, 0x08, 0x8A, 0x50, 0x04, 0x33, 0xC0, 0xEB, 0x03, 0x8D, 0x49, 0x00, 0x39, 0x0C, 0xC5, 0xB0, 0x6B, 0x9E, 0x02, 0x74, 0x09, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0A, 0x7C, 0xEF, 0xC3, 0x8D, 0x04, 0xC5, 0xB0, 0x6B, 0x9E, 0x02, 0x85, 0xC0, 0x74, 0x0F, 0x84, 0xD2, 0x8B, 0x50, 0x04, 0x0F, 0x95, 0xC1, 0x51, 0xE8, 0x35, 0x16, 0x00, 0x00, 0x59, 0xC3];
extern "C" fn h_79c100(r: &mut Pushad, a: *const u32) {
    count(28);
    unsafe { hb_79c100(r, a) }
}
thunk!(t_79c100, h_79c100, "ret");
const B_60C020: [u8; 39] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x68, 0x28, 0x15, 0x00, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x0C, 0x56, 0x8B, 0xF0, 0xE8, 0x12, 0x00, 0x00, 0x00, 0x5E, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
extern "C" fn h_60c020(r: &mut Pushad, a: *const u32) {
    count(29);
    unsafe { hb_60c020(r, a) }
}
thunk!(t_60c020, h_60c020, "ret 4");
const B_575080: [u8; 20] = [0x56, 0x57, 0xBF, 0xF0, 0x88, 0xDE, 0x00, 0xBE, 0x58, 0x89, 0xDE, 0x00, 0xE8, 0x4F, 0xF9, 0x27, 0x00, 0x5F, 0x5E, 0xC3];
extern "C" fn h_575080(r: &mut Pushad, a: *const u32) {
    count(30);
    unsafe { hb_575080(r, a) }
}
thunk!(t_575080, h_575080, "ret");
const B_600F60: [u8; 28] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x68, 0x97, 0x57, 0xDC, 0x00, 0xE8, 0xDC, 0x3B, 0x61, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_600f60(r: &mut Pushad, a: *const u32) {
    count(31);
    unsafe { hb_600f60(r, a) }
}
thunk!(t_600f60, h_600f60, "ret 8");
const B_615DD0: [u8; 28] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x68, 0x97, 0x57, 0xDC, 0x00, 0xE8, 0x6C, 0xED, 0x5F, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00];
extern "C" fn h_615dd0(r: &mut Pushad, a: *const u32) {
    count(32);
    unsafe { hb_615dd0(r, a) }
}
thunk!(t_615dd0, h_615dd0, "ret 8");
const B_CEDEC0: [u8; 65] = [0x85, 0xC9, 0x7C, 0x3C, 0xB8, 0x85, 0x91, 0x76, 0xAC, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x07, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x69, 0xC0, 0xBE, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x69, 0xC9, 0xE8, 0x05, 0x00, 0x00, 0x81, 0xC1, 0x40, 0xD2, 0x40, 0x03, 0x74, 0x13, 0x8B, 0x54, 0x24, 0x08, 0x8B, 0x44, 0x24, 0x04, 0x52, 0x50, 0x8B, 0x44, 0x24, 0x14, 0xE8, 0xF0, 0x31, 0xFF, 0xFF, 0xC3];
extern "C" fn h_cedec0(r: &mut Pushad, a: *const u32) {
    count(33);
    unsafe { hb_cedec0(r, a) }
}
thunk!(t_cedec0, h_cedec0, "ret");
const B_CF6160: [u8; 51] = [0x53, 0x56, 0x8B, 0xD9, 0x33, 0xF6, 0x39, 0x73, 0x08, 0x57, 0x76, 0x1B, 0x8D, 0x7B, 0x0C, 0x90, 0x8B, 0x0F, 0x8B, 0x01, 0x8B, 0x10, 0xFF, 0xD2, 0x84, 0xC0, 0x75, 0x11, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x04, 0x3B, 0x73, 0x08, 0x72, 0xE9, 0x5F, 0x5E, 0x32, 0xC0, 0x5B, 0xC3, 0x5F, 0x5E, 0xB0, 0x01, 0x5B, 0xC3];
extern "C" fn h_cf6160(r: &mut Pushad, a: *const u32) {
    count(34);
    unsafe { hb_cf6160(r, a) }
}
thunk!(t_cf6160, h_cf6160, "ret");
const B_5C2E80: [u8; 85] = [0x57, 0x8B, 0xF9, 0x8B, 0x87, 0xF4, 0x00, 0x00, 0x00, 0xA8, 0x10, 0x75, 0x46, 0xA8, 0x08, 0x74, 0x42, 0x55, 0x33, 0xED, 0x39, 0xAF, 0x98, 0x00, 0x00, 0x00, 0x76, 0x2F, 0x53, 0x56, 0x8D, 0x5F, 0x3C, 0x80, 0x7C, 0x2F, 0x78, 0x00, 0x75, 0x0C, 0x8B, 0x47, 0x20, 0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x04, 0x74, 0x07, 0x8B, 0x33, 0xE8, 0xE5, 0x25, 0x00, 0x00, 0x83, 0xC5, 0x01, 0x83, 0xC3, 0x04, 0x3B, 0xAF, 0x98, 0x00, 0x00, 0x00, 0x72, 0xD8, 0x5E, 0x5B, 0x83, 0x8F, 0xF4, 0x00, 0x00, 0x00, 0x10, 0x5D, 0x5F, 0xC3];
extern "C" fn h_5c2e80(r: &mut Pushad, a: *const u32) {
    count(35);
    unsafe { hb_5c2e80(r, a) }
}
thunk!(t_5c2e80, h_5c2e80, "ret");
const B_5C7400: [u8; 45] = [0x56, 0x8B, 0xF1, 0xE8, 0x18, 0xBC, 0xFF, 0xFF, 0x8B, 0x8E, 0xF8, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x5E, 0x74, 0x12, 0x05, 0x54, 0x85, 0x00, 0x00, 0x6A, 0x00, 0x50, 0x33, 0xC9, 0xE8, 0x07, 0x87, 0x59, 0x00, 0x83, 0xC4, 0x08, 0xC3];
extern "C" fn h_5c7400(r: &mut Pushad, a: *const u32) {
    count(36);
    unsafe { hb_5c7400(r, a) }
}
thunk!(t_5c7400, h_5c7400, "ret");
const B_5D2D40: [u8; 57] = [0x83, 0xEC, 0x08, 0x56, 0x8B, 0xF1, 0xE8, 0xD5, 0x02, 0xFF, 0xFF, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x8B, 0x8E, 0xFC, 0x00, 0x00, 0x00, 0x8B, 0x86, 0xF8, 0x00, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x8D, 0x54, 0x24, 0x08, 0x52, 0x56, 0x89, 0x44, 0x24, 0x10, 0x89, 0x4C, 0x24, 0x14, 0xE8, 0x4C, 0x26, 0xFF, 0xFF, 0x5E, 0x83, 0xC4, 0x08, 0xC3];
extern "C" fn h_5d2d40(r: &mut Pushad, a: *const u32) {
    count(37);
    unsafe { hb_5d2d40(r, a) }
}
thunk!(t_5d2d40, h_5d2d40, "ret");
const B_5CDD30: [u8; 121] = [0x57, 0x8B, 0xF9, 0x83, 0x7F, 0x24, 0x03, 0x75, 0x68, 0x8B, 0x8F, 0xF8, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x56, 0x74, 0x0B, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x8B, 0xF0, 0xEB, 0x02, 0x33, 0xF6, 0x8B, 0x87, 0x00, 0x01, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x29, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x91, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xD2, 0x74, 0x14, 0x39, 0x42, 0x44, 0x75, 0x0F, 0x83, 0x7A, 0x4C, 0x01, 0x75, 0x09, 0xE8, 0x70, 0x78, 0x3B, 0x00, 0x84, 0xC0, 0x74, 0x0A, 0xC7, 0x87, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x5E, 0x74, 0x0E, 0x83, 0xBF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x75, 0x05, 0xE8, 0x4F, 0xFE, 0xFF, 0xFF, 0x8B, 0xCF, 0x5F, 0xE9, 0x07, 0x5E, 0xFF, 0xFF];
extern "C" fn h_5cdd30(r: &mut Pushad, a: *const u32) {
    count(38);
    unsafe { hb_5cdd30(r, a) }
}
thunk!(t_5cdd30, h_5cdd30, "ret");
const B_6875B0: [u8; 80] = [0x51, 0xA1, 0xAC, 0xA8, 0xEA, 0x00, 0x83, 0xE8, 0x01, 0x74, 0x07, 0x83, 0xE8, 0x01, 0x74, 0x07, 0x59, 0xC3, 0xE8, 0x69, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0xB8, 0xA8, 0xEA, 0x00, 0x85, 0xC9, 0x74, 0x2D, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x19, 0x39, 0x48, 0x44, 0x75, 0x14, 0x83, 0x78, 0x4C, 0x05, 0x75, 0x0E, 0x80, 0xB8, 0xD8, 0x84, 0x00, 0x00, 0x00, 0x74, 0x05, 0xE8, 0x52, 0x01, 0x00, 0x00, 0x59, 0xC3];
extern "C" fn h_6875b0(r: &mut Pushad, a: *const u32) {
    count(39);
    unsafe { hb_6875b0(r, a) }
}
thunk!(t_6875b0, h_6875b0, "ret");
const B_AA5150: [u8; 93] = [0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0xD9, 0x10, 0x83, 0xFA, 0xFF, 0xD9, 0x50, 0x04, 0xD9, 0x58, 0x08, 0x75, 0x0C, 0x50, 0xE8, 0x73, 0x17, 0x00, 0x00, 0x83, 0xC4, 0x04, 0xC2, 0x08, 0x00, 0x80, 0xB9, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x75, 0x2E, 0x8D, 0x14, 0xD2, 0xC1, 0xE2, 0x05, 0x8D, 0x8C, 0x0A, 0x60, 0x05, 0x00, 0x00, 0x8B, 0x49, 0x04, 0x85, 0xC9, 0x74, 0x1A, 0xD9, 0x81, 0xA0, 0x01, 0x00, 0x00, 0xD9, 0x18, 0xD9, 0x81, 0xA4, 0x01, 0x00, 0x00, 0xD9, 0x58, 0x04, 0xD9, 0x81, 0xA8, 0x01, 0x00, 0x00, 0xD9, 0x58, 0x08, 0xC2, 0x08, 0x00];
extern "C" fn h_aa5150(r: &mut Pushad, a: *const u32) {
    count(40);
    unsafe { hb_aa5150(r, a) }
}
thunk!(t_aa5150, h_aa5150, "ret 8");
const B_841900: [u8; 92] = [0x0F, 0xB6, 0xC0, 0x83, 0xC0, 0x0B, 0x39, 0x05, 0x10, 0xB2, 0xE8, 0x00, 0x74, 0x4D, 0x83, 0xF8, 0x0E, 0xA3, 0x10, 0xB2, 0xE8, 0x00, 0x7E, 0x14, 0x83, 0xC8, 0xFF, 0xA3, 0x10, 0xB2, 0xE8, 0x00, 0x66, 0xA3, 0x14, 0xB2, 0xE8, 0x00, 0xA3, 0x94, 0x8D, 0x52, 0x02, 0xC3, 0x56, 0x57, 0xE8, 0x2D, 0xDB, 0xFF, 0xFF, 0x83, 0xC8, 0xFF, 0x66, 0xA3, 0x14, 0xB2, 0xE8, 0x00, 0xA3, 0x94, 0x8D, 0x52, 0x02, 0xA1, 0x10, 0xB2, 0xE8, 0x00, 0x33, 0xC9, 0xE8, 0xE3, 0xA1, 0x05, 0x00, 0x8B, 0xF8, 0xBE, 0x20, 0xD1, 0xA4, 0x02, 0xE8, 0xA7, 0x9B, 0x05, 0x00, 0x5F, 0x5E, 0xC3];
extern "C" fn h_841900(r: &mut Pushad, a: *const u32) {
    count(41);
    unsafe { hb_841900(r, a) }
}
thunk!(t_841900, h_841900, "ret");
const B_810DA0: [u8; 72] = [0x53, 0x33, 0xDB, 0x57, 0x6A, 0x29, 0x8D, 0x7E, 0x59, 0x53, 0x57, 0xC7, 0x06, 0x88, 0xFD, 0xDF, 0x00, 0x89, 0x5E, 0x30, 0x88, 0x5E, 0x34, 0x89, 0x5E, 0x38, 0xC7, 0x46, 0x3C, 0x01, 0x00, 0x00, 0x00, 0x88, 0x5E, 0x41, 0x88, 0x5E, 0x42, 0x88, 0x5E, 0x43, 0x88, 0x5E, 0x44, 0x88, 0x5E, 0x46, 0x88, 0x5E, 0x47, 0x88, 0x5E, 0x48, 0x88, 0x5E, 0x49, 0xE8, 0xE2, 0xFD, 0x48, 0x00, 0x83, 0xC4, 0x0C, 0x88, 0x1F, 0x5F, 0x8B, 0xC6, 0x5B, 0xC3];
extern "C" fn h_810da0(r: &mut Pushad, a: *const u32) {
    count(42);
    unsafe { hb_810da0(r, a) }
}
thunk!(t_810da0, h_810da0, "ret");
const B_70E9B0: [u8; 65] = [0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x57, 0x50, 0x8B, 0xC6, 0xE8, 0x61, 0x33, 0x63, 0x00, 0x33, 0xC0, 0x85, 0xFF, 0xC7, 0x06, 0x30, 0x8C, 0xE1, 0x00, 0xC7, 0x46, 0x10, 0x58, 0x8C, 0xE1, 0x00, 0x7E, 0x18, 0x33, 0xC9, 0x8B, 0x13, 0x8B, 0x14, 0x82, 0x8B, 0x6E, 0x18, 0x89, 0x54, 0x29, 0x04, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x10, 0x3B, 0xC7, 0x7C, 0xEA, 0x5D, 0x8B, 0xC6, 0x5B, 0xC2, 0x04, 0x00];
extern "C" fn h_70e9b0(r: &mut Pushad, a: *const u32) {
    count(43);
    unsafe { hb_70e9b0(r, a) }
}
thunk!(t_70e9b0, h_70e9b0, "ret 4");
const B_B95A60: [u8; 94] = [0xE8, 0xBB, 0x17, 0x00, 0x00, 0x84, 0xC0, 0x75, 0x01, 0xC3, 0xD9, 0xEE, 0x33, 0xC0, 0xD9, 0x51, 0x5C, 0xD9, 0x51, 0x60, 0xD9, 0x51, 0x64, 0xD9, 0x51, 0x68, 0xD9, 0xE8, 0xD9, 0x51, 0x70, 0xD9, 0x51, 0x74, 0xD9, 0xC9, 0xD9, 0x59, 0x78, 0x89, 0x41, 0x6C, 0x88, 0x41, 0x58, 0x89, 0x41, 0x54, 0xC6, 0x41, 0x59, 0x01, 0xD9, 0x51, 0x7C, 0xD9, 0x91, 0x80, 0x00, 0x00, 0x00, 0xD9, 0x91, 0x84, 0x00, 0x00, 0x00, 0x89, 0x81, 0x8C, 0x00, 0x00, 0x00, 0xD9, 0x91, 0x88, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xD9, 0x91, 0x90, 0x00, 0x00, 0x00, 0xD9, 0x99, 0x94, 0x00, 0x00, 0x00, 0xC3];
extern "C" fn h_b95a60(r: &mut Pushad, a: *const u32) {
    count(44);
    unsafe { hb_b95a60(r, a) }
}
thunk!(t_b95a60, h_b95a60, "ret");
const B_B9D780: [u8; 95] = [0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x56, 0x33, 0xDB, 0x33, 0xF6, 0x39, 0x1D, 0x70, 0xA2, 0x52, 0x02, 0x76, 0x46, 0x57, 0xBF, 0x70, 0x23, 0x2F, 0x02, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x07, 0x85, 0xC0, 0x74, 0x0E, 0x50, 0x55, 0xE8, 0x83, 0xB8, 0x0F, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x15, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x10, 0x3B, 0x35, 0x70, 0xA2, 0x52, 0x02, 0x72, 0xDE, 0x5F, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC3, 0xC1, 0xE6, 0x04, 0x81, 0xC6, 0x70, 0x23, 0x2F, 0x02, 0x5F, 0x8B, 0xC6, 0x5E, 0x5D, 0x5B, 0xC3, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC3];
extern "C" fn h_b9d780(r: &mut Pushad, a: *const u32) {
    count(45);
    unsafe { hb_b9d780(r, a) }
}
thunk!(t_b9d780, h_b9d780, "ret");
const B_7C20B0: [u8; 108] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x51, 0x8B, 0x45, 0x08, 0x8B, 0x40, 0x10, 0x53, 0x56, 0x33, 0xDB, 0x33, 0xF6, 0x39, 0x1C, 0x85, 0x28, 0xC8, 0xFA, 0x01, 0x57, 0x89, 0x44, 0x24, 0x0C, 0x7E, 0x42, 0x8B, 0xF8, 0x69, 0xFF, 0x20, 0x17, 0x00, 0x00, 0x81, 0xC7, 0x68, 0xC8, 0xFA, 0x01, 0x8B, 0x0F, 0x0F, 0xBE, 0x51, 0x15, 0x3B, 0x55, 0x0C, 0x75, 0x1A, 0x8B, 0x45, 0x10, 0x8B, 0x4D, 0x08, 0x50, 0x56, 0x51, 0xE8, 0x08, 0xFF, 0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x18, 0x83, 0xC4, 0x0C, 0x83, 0xC3, 0x01, 0xEB, 0x06, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x50, 0x3B, 0x34, 0x85, 0x28, 0xC8, 0xFA, 0x01, 0x7C, 0xCC, 0x5F, 0x5E, 0x8B, 0xC3, 0x5B, 0x8B, 0xE5, 0x5D, 0xC3];
extern "C" fn h_7c20b0(r: &mut Pushad, a: *const u32) {
    count(46);
    unsafe { hb_7c20b0(r, a) }
}
thunk!(t_7c20b0, h_7c20b0, "ret");
const B_4E41C0: [u8; 71] = [0x56, 0x8B, 0xF1, 0xC7, 0x06, 0x30, 0xE2, 0xDD, 0x00, 0xE8, 0x02, 0xFE, 0xFF, 0xFF, 0xF6, 0x44, 0x24, 0x08, 0x01, 0xC7, 0x06, 0x20, 0xF2, 0xDB, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x7A, 0xA6, 0x7F, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
extern "C" fn h_4e41c0(r: &mut Pushad, a: *const u32) {
    count(47);
    unsafe { hb_4e41c0(r, a) }
}
thunk!(t_4e41c0, h_4e41c0, "ret 4");
const B_4EC3D0: [u8; 66] = [0x56, 0x8D, 0x71, 0xA8, 0x8B, 0xC6, 0xE8, 0x95, 0xDF, 0xFF, 0xFF, 0xF6, 0x44, 0x24, 0x08, 0x01, 0x74, 0x2A, 0x85, 0xF6, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x6F, 0x24, 0x7F, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
extern "C" fn h_4ec3d0(r: &mut Pushad, a: *const u32) {
    count(48);
    unsafe { hb_4ec3d0(r, a) }
}
thunk!(t_4ec3d0, h_4ec3d0, "ret 4");
const B_BA0740: [u8; 60] = [0xF6, 0x44, 0x24, 0x04, 0x01, 0x56, 0x8B, 0xF1, 0xC7, 0x06, 0xBC, 0xCE, 0xE4, 0x00, 0x74, 0x26, 0xA0, 0xF7, 0x7B, 0x52, 0x02, 0x3C, 0x01, 0x74, 0x1D, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xCC, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0xE8, 0x05, 0xE1, 0x13, 0x00, 0x68, 0xE4, 0xAD, 0x47, 0x03, 0xFF, 0x15, 0xD0, 0x80, 0xDB, 0x00, 0x8B, 0xC6, 0x5E, 0xC2, 0x04, 0x00];
extern "C" fn h_ba0740(r: &mut Pushad, a: *const u32) {
    count(49);
    unsafe { hb_ba0740(r, a) }
}
thunk!(t_ba0740, h_ba0740, "ret 4");
const B_BA1920: [u8; 151] = [0xA1, 0xF4, 0xE3, 0xEB, 0x00, 0x8B, 0x08, 0x56, 0xBE, 0x3C, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x44, 0xF1, 0x1B, 0x03, 0xC7, 0x05, 0x3C, 0xF1, 0x1B, 0x03, 0x10, 0xE4, 0xEB, 0x00, 0xC7, 0x05, 0x40, 0xF1, 0x1B, 0x03, 0x08, 0x00, 0x00, 0x00, 0xE8, 0x94, 0x69, 0xFF, 0xFF, 0x8B, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0x83, 0xF9, 0x20, 0x8B, 0x15, 0xF4, 0xE3, 0xEB, 0x00, 0x8B, 0x32, 0x73, 0x2C, 0x8B, 0xC1, 0xC1, 0xE0, 0x04, 0x8B, 0xD1, 0x83, 0xC1, 0x01, 0xC7, 0x80, 0x70, 0x23, 0x2F, 0x02, 0xB8, 0xD9, 0xE4, 0x00, 0x89, 0xB0, 0x74, 0x23, 0x2F, 0x02, 0xC7, 0x80, 0x78, 0x23, 0x2F, 0x02, 0x3C, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0xEB, 0x03, 0x83, 0xCA, 0xFF, 0x89, 0x15, 0xC8, 0x6C, 0xE8, 0x00, 0xE8, 0x37, 0x0A, 0x00, 0x00, 0xA1, 0xC8, 0x6C, 0xE8, 0x00, 0x3B, 0x05, 0x70, 0xA2, 0x52, 0x02, 0x5E, 0x73, 0x0D, 0xC1, 0xE0, 0x04, 0xC7, 0x80, 0x7C, 0x23, 0x2F, 0x02, 0x34, 0xE5, 0xEB, 0x00, 0xB0, 0x01, 0xC3];
extern "C" fn h_ba1920(r: &mut Pushad, a: *const u32) {
    count(50);
    unsafe { hb_ba1920(r, a) }
}
thunk!(t_ba1920, h_ba1920, "ret");
const B_BA4720: [u8; 151] = [0xA1, 0x6C, 0xDF, 0xEB, 0x00, 0x8B, 0x08, 0x56, 0xBE, 0x54, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x5C, 0xF1, 0x1B, 0x03, 0xC7, 0x05, 0x54, 0xF1, 0x1B, 0x03, 0xC4, 0xE7, 0xEB, 0x00, 0xC7, 0x05, 0x58, 0xF1, 0x1B, 0x03, 0x01, 0x00, 0x00, 0x00, 0xE8, 0x94, 0x3B, 0xFF, 0xFF, 0x8B, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0x83, 0xF9, 0x20, 0x8B, 0x15, 0x6C, 0xDF, 0xEB, 0x00, 0x8B, 0x32, 0x73, 0x2C, 0x8B, 0xC1, 0xC1, 0xE0, 0x04, 0x8B, 0xD1, 0x83, 0xC1, 0x01, 0xC7, 0x80, 0x70, 0x23, 0x2F, 0x02, 0xD0, 0x01, 0xDE, 0x00, 0x89, 0xB0, 0x74, 0x23, 0x2F, 0x02, 0xC7, 0x80, 0x78, 0x23, 0x2F, 0x02, 0x54, 0xF1, 0x1B, 0x03, 0x89, 0x0D, 0x70, 0xA2, 0x52, 0x02, 0xEB, 0x03, 0x83, 0xCA, 0xFF, 0x89, 0x15, 0x78, 0x6C, 0xE8, 0x00, 0xE8, 0x87, 0x01, 0x00, 0x00, 0xA1, 0x78, 0x6C, 0xE8, 0x00, 0x3B, 0x05, 0x70, 0xA2, 0x52, 0x02, 0x5E, 0x73, 0x0D, 0xC1, 0xE0, 0x04, 0xC7, 0x80, 0x7C, 0x23, 0x2F, 0x02, 0xEC, 0xE7, 0xEB, 0x00, 0xB0, 0x01, 0xC3];
extern "C" fn h_ba4720(r: &mut Pushad, a: *const u32) {
    count(51);
    unsafe { hb_ba4720(r, a) }
}
thunk!(t_ba4720, h_ba4720, "ret");
const B_6CC4F0: [u8; 76] = [0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x3E, 0x8D, 0x44, 0x24, 0x08, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x08, 0x00, 0xE8, 0xA4, 0x66, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x08, 0x00, 0x75, 0x09, 0xE8, 0x58, 0x67, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x1E, 0x85, 0xC0, 0x74, 0x1A, 0x0F, 0xBE, 0xB0, 0x1C, 0x11, 0x00, 0x00, 0x6A, 0x01, 0x56, 0xE8, 0xD1, 0x84, 0x41, 0x00, 0x6A, 0x01, 0x56, 0xE8, 0x29, 0x81, 0x29, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0xC3];
extern "C" fn h_6cc4f0(r: &mut Pushad, a: *const u32) {
    count(52);
    unsafe { hb_6cc4f0(r, a) }
}
thunk!(t_6cc4f0, h_6cc4f0, "ret");
const B_93CE20: [u8; 170] = [0x51, 0xE8, 0x0A, 0x64, 0x00, 0x00, 0xB8, 0x01, 0x01, 0x01, 0x01, 0xA3, 0x24, 0xDA, 0x15, 0x02, 0x66, 0xA3, 0x28, 0xDA, 0x15, 0x02, 0xA3, 0x1C, 0xDA, 0x15, 0x02, 0x66, 0xA3, 0x20, 0xDA, 0x15, 0x02, 0xC7, 0x05, 0x80, 0x7A, 0xEB, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x33, 0xC0, 0x8D, 0x49, 0x00, 0x80, 0xB8, 0x1C, 0xDA, 0x15, 0x02, 0x00, 0x74, 0x0F, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x06, 0x72, 0xEF, 0xE8, 0xAA, 0xF7, 0xFF, 0xFF, 0xEB, 0x0A, 0xC7, 0x05, 0x80, 0x7A, 0xEB, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xB8, 0x01, 0x01, 0x01, 0x01, 0xA3, 0x14, 0xDA, 0x15, 0x02, 0x66, 0xA3, 0x18, 0xDA, 0x15, 0x02, 0xA3, 0x0C, 0xDA, 0x15, 0x02, 0x66, 0xA3, 0x10, 0xDA, 0x15, 0x02, 0xA3, 0x04, 0xDA, 0x15, 0x02, 0x66, 0xA3, 0x08, 0xDA, 0x15, 0x02, 0xE8, 0x93, 0x0B, 0x24, 0x00, 0xB8, 0x01, 0x01, 0x01, 0x01, 0xA3, 0xFC, 0xD9, 0x15, 0x02, 0x66, 0xA3, 0x00, 0xDA, 0x15, 0x02, 0xE8, 0x7E, 0x42, 0x24, 0x00, 0xC6, 0x05, 0x7F, 0x85, 0xE8, 0x00, 0x01, 0xE8, 0x52, 0x97, 0x00, 0x00, 0xC7, 0x05, 0x98, 0x7A, 0xEB, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x59, 0xC3];
extern "C" fn h_93ce20(r: &mut Pushad, a: *const u32) {
    count(53);
    unsafe { hb_93ce20(r, a) }
}
thunk!(t_93ce20, h_93ce20, "ret");
const B_B8A980: [u8; 40] = [0x56, 0x8B, 0xF0, 0x56, 0xE8, 0x77, 0x6A, 0x00, 0x00, 0xA1, 0xEC, 0xA1, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x06, 0x56, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0x50, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0x5E, 0xE9, 0x28, 0x02, 0x00, 0x00];
extern "C" fn h_b8a980(r: &mut Pushad, a: *const u32) {
    count(54);
    unsafe { hb_b8a980(r, a) }
}
thunk!(t_b8a980, h_b8a980, "ret");
const B_D0A4B0: [u8; 57] = [0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00, 0x75, 0x2B, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x00, 0x80, 0x78, 0x5C, 0x00, 0x75, 0x09, 0x81, 0x78, 0x60, 0xA0, 0x26, 0xEC, 0x00, 0x74, 0x09, 0x81, 0x78, 0x60, 0x10, 0x26, 0xEC, 0x00, 0x75, 0x0B, 0x6A, 0x1A, 0xE8, 0xC0, 0xE9, 0xFE, 0xFF, 0x83, 0xC4, 0x04, 0xC3, 0xE9, 0xA7, 0x40, 0x01, 0x00];
extern "C" fn h_d0a4b0(r: &mut Pushad, a: *const u32) {
    count(55);
    unsafe { hb_d0a4b0(r, a) }
}
thunk!(t_d0a4b0, h_d0a4b0, "ret");
const B_D229B0: [u8; 71] = [0x56, 0xE8, 0xDA, 0xFD, 0xFF, 0xFF, 0x8B, 0x88, 0x54, 0x15, 0x00, 0x00, 0x69, 0xC9, 0x44, 0x01, 0x00, 0x00, 0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00, 0x8D, 0x74, 0x01, 0xD0, 0x75, 0x21, 0x64, 0x8B, 0x15, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x02, 0x80, 0x78, 0x5C, 0x00, 0x75, 0x09, 0x81, 0x78, 0x60, 0xA0, 0x26, 0xEC, 0x00, 0x74, 0x0E, 0x81, 0x78, 0x60, 0x10, 0x26, 0xEC, 0x00, 0x74, 0x05, 0xE8, 0xEB, 0x64, 0xFF, 0xFF, 0x5E, 0xC3];
extern "C" fn h_d229b0(r: &mut Pushad, a: *const u32) {
    count(56);
    unsafe { hb_d229b0(r, a) }
}
thunk!(t_d229b0, h_d229b0, "ret");

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "camera_mode8_update", addr: 0x00495950, body: &B_495950, thunk: t_495950 },
    Port { name: "camera_external_mode_enter", addr: 0x004A0140, body: &B_4A0140, thunk: t_4a0140 },
    Port { name: "chunk_section_08_collision_mopp", addr: 0x004AE280, body: &B_4AE280, thunk: t_4ae280 },
    Port { name: "chunk_section_24_object_runtime_data", addr: 0x004AEF60, body: &B_4AEF60, thunk: t_4aef60 },
    Port { name: "script_object::dtor_3", addr: 0x00A58AB0, body: &B_A58AB0, thunk: t_a58ab0 },
    Port { name: "road_lane_zone_traffic_info", addr: 0x00B800C0, body: &B_B800C0, thunk: t_b800c0 },
    Port { name: "vint_element_sr2_map::post_render", addr: 0x00BA6320, body: &B_BA6320, thunk: t_ba6320 },
    Port { name: "vint_element_text::get_horz_align", addr: 0x00BA3820, body: &B_BA3820, thunk: t_ba3820 },
    Port { name: "vint_element_video::get_vid_id_handle", addr: 0x00BA6BB0, body: &B_BA6BB0, thunk: t_ba6bb0 },
    Port { name: "vint_draw_clip_begin", addr: 0x007F0A70, body: &B_7F0A70, thunk: t_7f0a70 },
    Port { name: "ui_button_image_tag", addr: 0x007F36D0, body: &B_7F36D0, thunk: t_7f36d0 },
    Port { name: "control_scheme_data_free", addr: 0x00773830, body: &B_773830, thunk: t_773830 },
    Port { name: "widget_activities::get_height", addr: 0x0077B310, body: &B_77B310, thunk: t_77b310 },
    Port { name: "garage_cash_widget::get_height", addr: 0x00552B30, body: &B_552B30, thunk: t_552b30 },
    Port { name: "trigger_multi_registration::free_snapshot", addr: 0x00872140, body: &B_872140, thunk: t_872140 },
    Port { name: "parked_vehicles_quick_update", addr: 0x00948D80, body: &B_948D80, thunk: t_948d80 },
    Port { name: "vint_loc_lookup_id", addr: 0x00B92200, body: &B_B92200, thunk: t_b92200 },
    Port { name: "vint_object_tween::set_target_name", addr: 0x00B9F4B0, body: &B_B9F4B0, thunk: t_b9f4b0 },
    Port { name: "static_mempool_base::ctor", addr: 0x00C00AE0, body: &B_C00AE0, thunk: t_c00ae0 },
    Port { name: "async_virtual_mempool::alloc", addr: 0x00C17CC0, body: &B_C17CC0, thunk: t_c17cc0 },
    Port { name: "barnstorming_diversion_type::init", addr: 0x005F9340, body: &B_5F9340, thunk: t_5f9340 },
    Port { name: "driver_bailout_diversion_type_info::init", addr: 0x006098D0, body: &B_6098D0, thunk: t_6098d0 },
    Port { name: "stunt_jumping_diversion_type::init", addr: 0x006185D0, body: &B_6185D0, thunk: t_6185d0 },
    Port { name: "taxi_diversion_type_info::init", addr: 0x00628E50, body: &B_628E50, thunk: t_628e50 },
    Port { name: "windshield_cannon_diversion_type_info::init", addr: 0x0063A760, body: &B_63A760, thunk: t_63a760 },
    Port { name: "act_packet_sub_49", addr: 0x00612430, body: &B_612430, thunk: t_612430 },
    Port { name: "act_packet_sub_4f", addr: 0x00649500, body: &B_649500, thunk: t_649500 },
    Port { name: "act_packet_sub_1f", addr: 0x00677BF0, body: &B_677BF0, thunk: t_677bf0 },
    Port { name: "hud_pkt_sub_10", addr: 0x0079C100, body: &B_79C100, thunk: t_79c100 },
    Port { name: "firetruck_diversion_type_info::create_instance", addr: 0x0060C020, body: &B_60C020, thunk: t_60c020 },
    Port { name: "vehicle_protect_objective::get_start_message", addr: 0x00575080, body: &B_575080, thunk: t_575080 },
    Port { name: "chop_shop_diversion_type::get_start_message", addr: 0x00600F60, body: &B_600F60, thunk: t_600f60 },
    Port { name: "safecracking_diversion_type_info::get_start_message", addr: 0x00615DD0, body: &B_615DD0, thunk: t_615dd0 },
    Port { name: "rig_handle_bone_local_transform", addr: 0x00CEDEC0, body: &B_CEDEC0, thunk: t_cedec0 },
    Port { name: "rl_container_inst::casts_shadow", addr: 0x00CF6160, body: &B_CF6160, thunk: t_cf6160 },
    Port { name: "fuzz_crime::tick_state2", addr: 0x005C2E80, body: &B_5C2E80, thunk: t_5c2e80 },
    Port { name: "fuzz_crime_littering_from_car::enter_state3", addr: 0x005C7400, body: &B_5C7400, thunk: t_5c7400 },
    Port { name: "fuzz_crime_street_racing::enter_state3", addr: 0x005D2D40, body: &B_5D2D40, thunk: t_5d2d40 },
    Port { name: "fuzz_crime_stolen_heli::update", addr: 0x005CDD30, body: &B_5CDD30, thunk: t_5cdd30 },
    Port { name: "cheat_vehicle_drop_update", addr: 0x006875B0, body: &B_6875B0, thunk: t_6875b0 },
    Port { name: "vehicle::get_attach_point", addr: 0x00AA5150, body: &B_AA5150, thunk: t_aa5150 },
    Port { name: "mp_switch_modes_impl", addr: 0x00841900, body: &B_841900, thunk: t_841900 },
    Port { name: "pc_multi_session_info::ctor", addr: 0x00810DA0, body: &B_810DA0, thunk: t_810da0 },
    Port { name: "havok_filter_shape_collection::ctor", addr: 0x0070E9B0, body: &B_70E9B0, thunk: t_70e9b0 },
    Port { name: "vint_element_base_init_defaults", addr: 0x00B95A60, body: &B_B95A60, thunk: t_b95a60 },
    Port { name: "vint_type_find_by_name", addr: 0x00B9D780, body: &B_B9D780, thunk: t_b9d780 },
    Port { name: "pcu_worn_remove_by_slot", addr: 0x007C20B0, body: &B_7C20B0, thunk: t_7c20b0 },
    Port { name: "ctype<>::scalar_deleting_dtor", addr: 0x004E41C0, body: &B_4E41C0, thunk: t_4e41c0 },
    Port { name: "basic_ifstream<>::scalar_deleting_dtor", addr: 0x004EC3D0, body: &B_4EC3D0, thunk: t_4ec3d0 },
    Port { name: "vint_callback_native::scalar_deleting_dtor", addr: 0x00BA0740, body: &B_BA0740, thunk: t_ba0740 },
    Port { name: "vint_element_gradient_register_type", addr: 0x00BA1920, body: &B_BA1920, thunk: t_ba1920 },
    Port { name: "vint_element_point_register_type", addr: 0x00BA4720, body: &B_BA4720, thunk: t_ba4720 },
    Port { name: "cutscene_action_team_despawn", addr: 0x006CC4F0, body: &B_6CC4F0, thunk: t_6cc4f0 },
    Port { name: "spawn_system_init", addr: 0x0093CE20, body: &B_93CE20, thunk: t_93ce20 },
    Port { name: "vint_document_destroy", addr: 0x00B8A980, body: &B_B8A980, thunk: t_b8a980 },
    Port { name: "im2d_c_end", addr: 0x00D0A4B0, body: &B_D0A4B0, thunk: t_d0a4b0 },
    Port { name: "gfx_transform_apply", addr: 0x00D229B0, body: &B_D229B0, thunk: t_d229b0 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_s4: replaced {ok}/{COUNT} with Rust"));
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
        format!("port_s4: {}", parts.join(", "))
    }
}
