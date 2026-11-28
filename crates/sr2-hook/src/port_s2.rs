//! Port module s2: small functions (Sonnet trial round 8).
//!
//! Every address is pinned to its whole original body by `replace_pinned`. The thunks save XMM0-7
//! and `pushad`; the Rust body edits the saved registers. Callees stay original and are called by
//! address through `cx*` (all registers, XMM0-7 and the stack words the original has at the call).
//! Checks: the judge (`sr2-judge`) only.

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 48;
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

/// Same, with ST0 = the f32 in the block's `esp` slot (an x87 float result).
macro_rules! hot_thunk_st0 {
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
                "fld dword ptr [esp + 140]",
                "add esp, 128",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// Stack words handed to every callee: its own arguments, then what the original's frame holds above them.
const FW: usize = 12;

/// The words the original has on the stack at a call: `own` (top first), then its return address and the
/// caller's own stack arguments.
unsafe fn fr(a: *const u32, own: &[u32]) -> [u32; FW] {
    let mut w = [0u32; FW];
    let mut n = 0;
    for &v in own {
        w[n] = v;
        n += 1;
    }
    w[n] = unsafe { *a.sub(1) };
    n += 1;
    let mut i = 0;
    while n < FW {
        w[n] = unsafe { *a.add(i) };
        n += 1;
        i += 1;
    }
    w
}

/// The words a callee entered by a tail `jmp` sees: the original's own stack arguments.
unsafe fn fr_tail(a: *const u32) -> [u32; FW] {
    let mut w = [0u32; FW];
    for (i, v) in w.iter_mut().enumerate() {
        *v = unsafe { *a.add(i) };
    }
    w
}

/// Call `f` with every register and XMM0-7 of `c`, the stack words `w` pushed, and load what the callee
/// leaves back into `c`. ESP is restored from a TEB scratch slot (`fs:[0x14]`), so the callee may pop
/// any number of its words or none.
#[inline(never)]
unsafe fn cx(f: usize, c: &mut Pushad, w: &[u32; FW]) {
    let blk: [u32; 3] = [f as u32, w.as_ptr() as u32, c as *mut Pushad as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebx, eax",
            "push dword ptr [ebx + 8]",
            "push dword ptr [ebx]",
            "push dword ptr fs:[0x14]",
            "mov fs:[0x14], esp",
            "mov esi, [ebx + 4]",
            "mov ecx, 12",
            "2:",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jnz 2b",
            "mov eax, [ebx + 8]",
            "movups xmm0, [eax]", "movups xmm1, [eax + 16]", "movups xmm2, [eax + 32]", "movups xmm3, [eax + 48]",
            "movups xmm4, [eax + 64]", "movups xmm5, [eax + 80]", "movups xmm6, [eax + 96]", "movups xmm7, [eax + 112]",
            "mov edi, [eax + 128]", "mov esi, [eax + 132]", "mov ebp, [eax + 136]", "mov ebx, [eax + 144]",
            "mov edx, [eax + 148]", "mov ecx, [eax + 152]", "mov eax, [eax + 156]",
            "call dword ptr [esp + 52]",
            "mov esp, fs:[0x14]",
            "push eax",
            "mov eax, [esp + 12]",
            "movups [eax], xmm0", "movups [eax + 16], xmm1", "movups [eax + 32], xmm2", "movups [eax + 48], xmm3",
            "movups [eax + 64], xmm4", "movups [eax + 80], xmm5", "movups [eax + 96], xmm6", "movups [eax + 112], xmm7",
            "mov [eax + 128], edi", "mov [eax + 132], esi", "mov [eax + 136], ebp", "mov [eax + 144], ebx",
            "mov [eax + 148], edx", "mov [eax + 152], ecx",
            "pop ecx",
            "mov [eax + 156], ecx",
            "pop dword ptr fs:[0x14]",
            "add esp, 8",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") blk.as_ptr() => _,
            out("ecx") _, out("edx") _,
            clobber_abi("C"),
        );
    }
}

struct Port {
    id: usize,
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

// ---- get_title / get_start_message shapes: push esi / edi; edi = X; esi = Y; call; pop; ret ----

macro_rules! gt {
    ($t:ident, $p:ident, $id:expr, $f:expr, $edi:expr, $esi:expr) => {
        unsafe extern "C" fn $p(r: &mut Pushad, a: *const u32) {
            unsafe {
                count($id);
                let (s0, d0) = (r.esi, r.edi);
                r.edi = $edi;
                r.esi = $esi;
                cx($f, r, &fr(a, &[]));
                r.esi = s0;
                r.edi = d0;
            }
        }
        hot_thunk!($t, $p, "ret");
    };
}

gt!(t_5751d0, p_5751d0, 0, 0x007F_4AD0, 0x00DE_8A64, 0x00DE_8A90);
gt!(t_5ce520, p_5ce520, 1, 0x007F_49E0, 0x00DE_FA6C, 0x00DE_FA80);
gt!(t_6457e0, p_6457e0, 2, 0x007F_49E0, 0x00DF_ABE0, 0x00DF_ABF4);

// ---- exploration_diversion_type::get_start_message @ 0x60aac0 ----
unsafe extern "C" fn p_60aac0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(3);
        let (s0, d0) = (r.esi, r.edi);
        r.esi = *a;
        r.edi = *a.add(1);
        cx(0x00C1_4B50, r, &fr(a, &[0x00DC_5797]));
        r.esi = s0;
        r.edi = d0;
    }
}
hot_thunk!(t_60aac0, p_60aac0, "ret 8");

// ---- create_instance shapes: ecx = a[0]; alloc through vtable slot 7 (ret 8) ----
unsafe fn vt_alloc(r: &mut Pushad, a: *const u32, size: u32) {
    unsafe {
        let o = *a;
        r.ecx = o;
        let vt = rd(o as usize);
        r.eax = vt;
        let f = rd(vt as usize + 0x1c);
        r.edx = f;
        cx(f as usize, r, &fr(a, &[size, 0]));
    }
}

unsafe extern "C" fn p_6164e0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(5);
        vt_alloc(r, a, 0x18);
        if r.eax != 0 {
            wd(r.eax as usize, 0x00DF_5D4C);
        }
    }
}
hot_thunk!(t_6164e0, p_6164e0, "ret 4");

unsafe extern "C" fn p_61f120(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(6);
        vt_alloc(r, a, 0x150);
        if r.eax != 0 {
            wd(r.eax as usize, 0x00DF_7154);
            wd(r.eax as usize + 0xf8, 0xFFFF_FFFF);
        }
    }
}
hot_thunk!(t_61f120, p_61f120, "ret 4");

unsafe extern "C" fn p_641930(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(7);
        vt_alloc(r, a, 0x90c);
        if r.eax == 0 {
            return;
        }
        cx(0x0064_17E0, r, &fr(a, &[]));
    }
}
hot_thunk!(t_641930, p_641930, "ret 4");

// ---- base_jumping_diversion_type_info::init @ 0x5f9e00 ----
unsafe extern "C" fn p_5f9e00(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(8);
        let s0 = r.esi;
        r.esi = r.ecx;
        cx(0x0060_2A50, r, &fr(a, &[]));
        wb(r.esi as usize + 0x18, 0);
        cx(0x005F_9AA0, r, &fr(a, &[r.esi]));
        wb(r.esi as usize + 0xd4, 0);
        wd(r.esi as usize + 0x13c, 0);
        r.esi = s0;
    }
}
hot_thunk!(t_5f9e00, p_5f9e00, "ret");

// ---- whoring_diversion_type_info::init @ 0x638190 ----
unsafe extern "C" fn p_638190(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(9);
        let s0 = r.esi;
        r.esi = r.ecx;
        cx(0x0060_2A50, r, &fr(a, &[]));
        cx(0x0063_7CE0, r, &fr(a, &[]));
        cx(0x0063_8850, r, &fr(a, &[r.esi]));
        r.eax = 0;
        wd(r.esi as usize + 0x73c, 0);
        wb(r.esi as usize + 0x20, 1);
        wb(r.esi as usize + 0x21, 0);
        r.esi = s0;
    }
}
hot_thunk!(t_638190, p_638190, "ret");

// ---- escort_enemy_spawn_group::reset @ 0x6794d0 ----
unsafe extern "C" fn p_6794d0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(10);
        let s0 = r.esi;
        r.esi = r.ecx;
        cx(0x0095_6080, r, &fr(a, &[]));
        r.eax = 0;
        wd(r.esi as usize + 0x1e4, 0);
        wd(r.esi as usize + 0x1fc, 0);
        r.esi = s0;
    }
}
hot_thunk!(t_6794d0, p_6794d0, "ret");

// ---- special_vehicle_spawn_group::reset @ 0x956080 ----
unsafe extern "C" fn p_956080(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(11);
        let c = r.ecx as usize;
        r.eax = 0;
        wb(c + 0x55, 0);
        wb(c + 0x56, 0);
        wb(c + 0x54, 0);
        for off in [0x2c, 0x30, 0x34, 0x38, 0x3c, 0x40, 0x44, 0x48, 0x4c, 0x50, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x1c, 0x20, 0x24, 0x28] {
            wd(c + off, 0);
        }
        r.ecx = (c + 0x5c) as u32;
        cx(0x00CA_0BC0, r, &fr(a, &[r.ecx, 0, 0x180]));
    }
}
hot_thunk!(t_956080, p_956080, "ret");

// ---- crash_complication::load_from_table @ 0x66e070 ----
unsafe extern "C" fn p_66e070(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(12);
        let (s0, d0) = (r.esi, r.edi);
        r.edi = *a;
        r.esi = r.ecx;
        cx(0x0066_DB90, r, &fr(a, &[r.edi]));
        r.esi = r.esi.wrapping_add(0x24);
        r.ecx = 0x00DD_8BDC;
        cx(0x00B7_47D0, r, &fr(a, &[r.edi]));
        r.esi = s0;
        r.edi = d0;
    }
}
hot_thunk!(t_66e070, p_66e070, "ret 4");

// ---- brass::deactivate @ 0x4bd530 (tail jump to 0x9257f0) ----
unsafe extern "C" fn p_4bd530(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(13);
        let s = r.ecx as usize;
        let s0 = r.esi;
        r.eax = rd(s + 0x7c);
        wd(s + 0x80, rd(s + 0x80) | 8);
        if (r.eax as i32) >= 0 {
            let e = r.eax;
            r.esi = s as u32;
            cx(0x006F_BE90, r, &fr(a, &[e]));
            wd(s + 0x7c, 0xFFFF_FFFF);
        }
        wd(s + 0xc0, 0xFFFF_FFFF);
        r.ecx = s as u32;
        r.esi = s0;
        cx(0x0092_57F0, r, &fr_tail(a));
    }
}
hot_thunk!(t_4bd530, p_4bd530, "ret");

// ---- city_view_instances_job @ 0x4b6520: cdecl(a0, ptr, count): 0x4b5ff0(ptr) per 16-byte element ----
unsafe extern "C" fn p_4b6520(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(14);
        let (s0, d0) = (r.esi, r.edi);
        r.edi = *a.add(2);
        if (r.edi as i32) > 0 {
            r.esi = *a.add(1);
            loop {
                cx(0x004B_5FF0, r, &fr(a, &[r.esi]));
                r.esi = r.esi.wrapping_add(0x10);
                r.edi = r.edi.wrapping_sub(1);
                if r.edi == 0 {
                    break;
                }
            }
        }
        r.esi = s0;
        r.edi = d0;
    }
}
hot_thunk!(t_4b6520, p_4b6520, "ret");

// ---- zone_chunk_object_active @ 0x4ab370 ----
unsafe extern "C" fn p_4ab370(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(15);
        cx(0x00A7_C760, r, &fr(a, &[]));
        let i = r.eax & 0xFFFF;
        r.eax = 0;
        if i == 0xFFFF || i >= 0x866 {
            return;
        }
        let p = rd(0x00F7_AAE8 + i as usize * 4);
        r.eax = p;
        if p == 0 || rb(p as usize + 0x1c) & 1 != 0 {
            r.eax = 0;
            return;
        }
        r.eax = 1;
    }
}
hot_thunk!(t_4ab370, p_4ab370, "ret");

// ---- cutscene_action_mesh_mover_play @ 0x6cc770 ----
unsafe extern "C" fn p_6cc770(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(16);
        let s0 = r.esi;
        r.esi = *a;
        r.eax = rd(r.esi as usize + 8);
        'end: {
            if r.eax == 0 || rd(r.esi as usize + 0x10) == 0 {
                break 'end;
            }
            cx(0x00A5_B6E0, r, &fr(a, &[]));
            if r.eax == 0 {
                break 'end;
            }
            let c = rd(r.esi as usize + 0x10);
            r.ecx = r.eax;
            cx(0x00A5_BD40, r, &fr(a, &[c]));
        }
        r.esi = s0;
    }
}
hot_thunk!(t_6cc770, p_6cc770, "ret");

// ---- cash_widget::get_height @ 0x74bbb0 and widget_info::get_height @ 0x77efc0 ----
unsafe fn get_height(r: &mut Pushad, a: *const u32) {
    unsafe {
        r.ecx = rd(0x00E9_8A7C);
        cx(0x00C1_5D60, r, &fr(a, &[]));
        let k = *(0x00E5_C228usize as *const f64);
        let v = (r.eax as i32 as f64 * k) as f32;
        r.esp = v.to_bits();
        // push ecx / pop ecx: the slot held the stored result when it was popped
        r.ecx = v.to_bits();
    }
}
unsafe extern "C" fn p_74bbb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(17);
        get_height(r, a);
    }
}
hot_thunk_st0!(t_74bbb0, p_74bbb0, "ret 4");
unsafe extern "C" fn p_77efc0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(18);
        get_height(r, a);
    }
}
hot_thunk_st0!(t_77efc0, p_77efc0, "ret 4");

// ---- nav_frame_update @ 0xa132c0 ----
unsafe extern "C" fn p_a132c0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(19);
        let ecx0 = r.ecx;
        cx(0x00A1_7690, r, &fr(a, &[]));
        cx(0x00A1_CCC0, r, &fr(a, &[]));
        cx(0x00A0_AFF0, r, &fr(a, &[]));
        cx(0x00A0_BF10, r, &fr(a, &[]));
        r.ecx = ecx0;
    }
}
hot_thunk!(t_a132c0, p_a132c0, "ret");

// ---- vehicle_class_is_1 @ 0xab51b0 ----
unsafe extern "C" fn p_ab51b0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(20);
        if r.eax == 0 {
            r.eax &= !0xff;
            return;
        }
        let p = rd(r.eax as usize + 0x84e4);
        let c = (rd(p as usize + 0x9c) == 1) as u32;
        r.ecx = c;
        r.eax = (p & !0xff) | c;
    }
}
hot_thunk!(t_ab51b0, p_ab51b0, "ret");

// ---- profile_options_copy @ 0x690c50: eax = source, edx = destination ----
#[inline(always)]
unsafe fn cd(d: usize, s: usize, n: usize) {
    for i in 0..n {
        unsafe { std::ptr::write_volatile((d + i * 4) as *mut u32, std::ptr::read_volatile((s + i * 4) as *const u32)) };
    }
}
#[inline(always)]
unsafe fn cbyte(d: usize, s: usize) {
    unsafe { std::ptr::write_volatile(d as *mut u8, std::ptr::read_volatile(s as *const u8)) };
}
unsafe extern "C" fn p_690c50(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(21);
        let (e, d) = (r.eax as usize, r.edx as usize);
        cd(d, e, 1);
        cd(d + 4, e + 4, 0x15);
        cd(d + 0x58, e + 0x58, 3);
        std::ptr::write_volatile((d + 0x64) as *mut u16, std::ptr::read_volatile((e + 0x64) as *const u16));
        cbyte(d + 0x66, e + 0x66);
        cd(d + 0x68, e + 0x68, 1);
        cd(d + 0x6c, e + 0x6c, 0xc);
        cbyte(d + 0x9c, e + 0x9c);
        cbyte(d + 0x9d, e + 0x9d);
        cd(d + 0xa0, e + 0xa0, 1);
        cd(d + 0xa4, e + 0xa4, 0xc);
        cbyte(d + 0xd4, e + 0xd4);
        r.ecx = 0;
    }
}
hot_thunk!(t_690c50, p_690c50, "ret");

// ---- gamesave_parse_digits @ 0x6953c0: eax = length, ecx = text ----
unsafe extern "C" fn p_6953c0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(22);
        let ptr = r.ecx;
        if ptr == 0 {
            r.eax = 0;
            return;
        }
        let mut ecx = r.ecx;
        let mut edx = r.edx;
        let mut esi = r.eax as i32;
        let mut edi = ptr as usize;
        let mut ebp = 0u32;
        let mut eax = 1u32;
        loop {
            let bl = rb(edi);
            esi = esi.wrapping_sub(1);
            if bl == 0x2d {
                eax = 0xFFFF_FFFF;
            } else {
                let b = bl as i8;
                if b < 0x30 || b > 0x39 {
                    break;
                }
                edx = esi as u32;
                ecx = 1;
                if esi > 0 {
                    loop {
                        ecx = ecx.wrapping_mul(5);
                        edx = edx.wrapping_sub(1);
                        ecx = ecx.wrapping_add(ecx);
                        if (edx as i32) <= 0 {
                            break;
                        }
                    }
                }
                edx = (b as i32).wrapping_sub(0x30) as u32;
                edx = edx.wrapping_mul(ecx);
                ebp = ebp.wrapping_add(edx);
            }
            edi += 1;
            if esi <= 0 {
                break;
            }
        }
        r.eax = eax.wrapping_mul(ebp);
        r.ecx = ecx;
        r.edx = edx;
    }
}
hot_thunk!(t_6953c0, p_6953c0, "ret");

// ---- vint_property_call_setter @ 0xb8a2a0: eax = object ----
unsafe extern "C" fn p_b8a2a0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(24);
        let f = rd(r.eax as usize + 8);
        r.eax = f;
        if f == 0 {
            r.eax &= !0xff;
            return;
        }
        let ecx0 = r.ecx;
        r.edx = *a.add(1);
        r.ecx = *a;
        cx(f as usize, r, &fr(a, &[*a, *a.add(1), ecx0]));
    }
}
hot_thunk!(t_b8a2a0, p_b8a2a0, "ret 8");

// ---- vint_document_parse_sections_v2 @ 0xb8cc60: eax = esi arg, edi = document ----
unsafe extern "C" fn p_b8cc60(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(25);
        let s0 = r.esi;
        r.esi = r.eax;
        r.eax = r.edi;
        cx(0x00B8_AF20, r, &fr(a, &[]));
        r.eax = r.edi;
        cx(0x00B8_AE80, r, &fr(a, &[r.esi]));
        r.eax = r.edi;
        cx(0x00B8_CA90, r, &fr(a, &[r.esi]));
        r.eax = r.edi;
        cx(0x00B8_CB90, r, &fr(a, &[r.esi]));
        r.eax = r.edi;
        cx(0x00B8_C3A0, r, &fr(a, &[r.esi, 0]));
        r.eax = r.edi;
        cx(0x00B8_C3A0, r, &fr(a, &[r.esi, 1]));
        r.eax = (r.eax & !0xff) | 1;
        r.esi = s0;
    }
}
hot_thunk!(t_b8cc60, p_b8cc60, "ret");

// ---- net_recv_create_ack @ 0x805c70 ----
unsafe extern "C" fn p_805c70(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(26);
        let s0 = r.esi;
        let w = *(*a as usize as *const u16) as u32;
        r.eax = w;
        r.esi = rd(0x0212_E258);
        r.ecx = w;
        cx(0x0080_8E30, r, &fr(a, &[w, 0]));
        r.esi = s0;
        if r.eax == 0 {
            return;
        }
        r.eax = rd(r.eax as usize);
        if r.eax == 0 {
            return;
        }
        let o = *a.add(1);
        r.edx = o;
        let c = rd(o as usize + 4);
        let cl = rb(c as usize + 0x1b0);
        r.ecx = (c & !0xff) | cl as u32;
        r.edx = 1u32 << (cl & 31);
        let p = r.eax as usize + 0x67e;
        *(p as *mut u16) |= r.edx as u16;
    }
}
hot_thunk!(t_805c70, p_805c70, "ret");

// ---- net_send_packet_routed @ 0x8145b0: eax = this, edx = packet ----
unsafe extern "C" fn p_8145b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(27);
        let s0 = r.esi;
        r.esi = r.eax;
        r.eax = rd(r.esi as usize + 0x1ac);
        r.ecx = rd(r.eax as usize + 0x408);
        let k = rb(r.edx as usize) as u32;
        r.eax = k;
        let row = if (k as i32) < 0x32 {
            k.wrapping_mul(24).wrapping_add(0x00E8_BF08)
        } else {
            k.wrapping_mul(3).wrapping_sub(0x96).wrapping_mul(8).wrapping_add(0x00E8_B6A8)
        };
        r.eax = rd(row as usize + 0xc);
        let mut call = false;
        if r.eax == 0 {
            if r.ecx != 0 {
                r.eax = rd(r.ecx as usize + 0x1ac);
                if r.ecx == rd(r.eax as usize + 0x404) {
                    call = true;
                }
            }
        } else if r.eax != 1 || r.ecx == 0 {
            call = true;
        } else {
            r.eax = rd(r.ecx as usize + 0x1ac);
            if r.ecx != rd(r.eax as usize + 0x404) {
                call = true;
            }
        }
        if call {
            cx(0x007F_A390, r, &fr(a, &[r.edx]));
        }
        r.esi = s0;
    }
}
hot_thunk!(t_8145b0, p_8145b0, "ret");

// ---- hud_pkt_sub_03 @ 0x92d110 ----
unsafe extern "C" fn p_92d110(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(28);
        let s0 = r.esi;
        let p = *a;
        r.ecx = p;
        let al = rb(p as usize);
        r.eax = (r.eax & !0xff) | al as u32;
        r.esi = rd(p as usize + 1);
        wb(a as usize, al);
        cx(0x0092_C2E0, r, &fr(a, &[]));
        r.esi = s0;
        if r.eax == 0 {
            return;
        }
        let res = r.eax;
        r.eax = *a;
        cx(0x0092_D4B0, r, &fr(a, &[*a, res, 1, 0]));
    }
}
hot_thunk!(t_92d110, p_92d110, "ret");

// ---- net_recv_req_team @ 0x82b940 ----
unsafe extern "C" fn p_82b940(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(29);
        let p = rd(*a.add(1) as usize + 4);
        if p == 0 {
            r.eax = 0;
            return;
        }
        let w = std::ptr::read_volatile((p as usize + 0x1a8) as *const u16) as u32;
        // (black_box: rustc drops the mask when the 16-bit load address is derived from `p`)
        let up = std::hint::black_box(p) & 0xFFFF_0000;
        std::ptr::write_volatile(&mut r.eax, up | w);
        cx(0x007F_8020, r, &fr(a, &[]));
        if r.eax == 0 {
            return;
        }
        let c = *a;
        r.ecx = c;
        let d = rb(c as usize) as i8 as i32 as u32;
        r.edx = d;
        if rd(0x00E9_6C00) == 6 {
            r.ecx = rd(0x00E9_6B7C);
            if (r.ecx as i32) > -1 && rd(0x0144_E760 + r.ecx as usize * 4) == 5 && rb(0x0252_91B5) != 0 {
                return;
            }
        }
        cx(0x0087_F890, r, &fr(a, &[d]));
        r.ecx = d;
    }
}
hot_thunk!(t_82b940, p_82b940, "ret");

// ---- vector3_uncompressed::compress @ 0x7fe5b0 ----
unsafe extern "C" fn p_7fe5b0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(30);
        r.ecx = *a;
        if r.ecx == 0 {
            return;
        }
        let (s0, d0) = (r.esi, r.edi);
        r.esi = *a.add(1);
        if r.esi != 0 {
            r.edi = *a.add(3);
            if r.edi != 0 {
                cx(0x007F_D870, r, &fr(a, &[]));
                let (s, d) = (r.esi as usize, r.edi as usize);
                r.eax = rd(s);
                wd(d, r.eax);
                r.ecx = rd(s + 4);
                wd(d + 4, r.ecx);
                r.edx = rd(s + 8);
                wd(d + 8, r.edx);
            }
        }
        r.esi = s0;
        r.edi = d0;
    }
}
hot_thunk!(t_7fe5b0, p_7fe5b0, "ret 16");

// ---- diversion_try_start_offered @ 0x603250 ----
unsafe extern "C" fn p_603250(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(31);
        let idx = rd(0x00E9_7C0C);
        r.eax = idx;
        r.ecx = rd(0x0140_B168usize.wrapping_add(idx as usize * 4));
        r.edx = rd(r.ecx as usize);
        let f = rd(r.edx as usize + 0x40);
        r.eax = f;
        cx(f as usize, r, &fr(a, &[]));
        if r.eax != *a {
            return;
        }
        r.ecx = rd(0x00E9_7C0C);
        r.eax = 2;
        wd(0x00E9_7C10, r.ecx);
        if rd(0x00E9_6C00) == 2 {
            return;
        }
        wd(0x00E9_6C04, 2);
        wd(0x0252_7BB4, 0);
    }
}
hot_thunk!(t_603250, p_603250, "ret");

// ---- stunt_near_miss_diversion_type::on_game_event_15 @ 0x61cf60 ----
unsafe extern "C" fn p_61cf60(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(32);
        let edx = rd(0x0217_03D4);
        r.edx = edx;
        if rd(edx as usize + 0xd74) == 0 || *a.add(3) == 0 {
            return;
        }
        r.eax = *a.add(1);
        if r.eax != 0 {
            r.eax = rd(r.eax as usize + 0x44);
            if r.eax != rd(edx as usize + 0x44) {
                return;
            }
        }
        let mut t = rd(0x00E8_43AC).wrapping_add(0x3e8);
        r.eax = t;
        wd(r.ecx as usize + 0x44, t);
        if (t as i32) > 0x6B49_D200 {
            t = t.wrapping_add(0x94B6_2E00);
            r.eax = t;
            wd(r.ecx as usize + 0x44, t);
        }
        if (rd(r.ecx as usize + 0x3c) as i32) <= 0 {
            return;
        }
        let c = r.ecx;
        cx(0x0061_CEE0, r, &fr(a, &[c]));
    }
}
hot_thunk!(t_61cf60, p_61cf60, "ret 20");

// ---- weapon_clip_size @ 0xb73cc0: eax = owner, esi = weapon ----
unsafe extern "C" fn p_b73cc0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(35);
        let d0 = r.edi;
        r.edi = r.eax;
        let mut use_call = false;
        if r.edi != 0 {
            let o = rd(r.edi as usize + 0x10cc);
            r.eax = if o != 0 { rd(o as usize + 0xc4) } else { 0 };
            if r.eax == r.esi {
                use_call = true;
            }
        }
        if use_call {
            cx(0x009C_9130, r, &fr(a, &[]));
        } else {
            let b = rb(r.esi as usize + 0xd) & 1;
            r.eax = (r.eax & !0xff) | b as u32;
        }
        r.edi = d0;
        if r.eax & 0xff != 0 {
            let w = *((rd(r.esi as usize + 8) as usize + 0xdc) as *const u16) as u32;
            r.eax = w.wrapping_add(w);
        } else {
            r.ecx = rd(r.esi as usize + 8);
            r.eax = *((r.ecx as usize + 0xdc) as *const u16) as u32;
        }
    }
}
hot_thunk!(t_b73cc0, p_b73cc0, "ret");

// ---- unlockable_parse_team @ 0x6bbcb0: eax = list owner, a0 = the unlockable ----
unsafe extern "C" fn p_6bbcb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(33);
        if r.eax == 0 {
            r.eax &= !0xff;
            return;
        }
        let (s0, d0) = (r.esi, r.edi);
        r.esi = rd(r.eax as usize + 8);
        let mut res = 0u32;
        while r.esi != 0 {
            r.eax = rd(r.esi as usize);
            let name = r.eax;
            cx(0x00C9_9030, r, &fr(a, &[name, 0x00DD_AB10]));
            if r.eax == 0 {
                res = r.esi;
                break;
            }
            r.esi = rd(r.esi as usize + 4);
        }
        r.eax = res;
        r.edi = 8;
        cx(0x00B7_5580, r, &fr(a, &[0x00DD_0BD0]));
        r.ecx = *a;
        wd(r.ecx as usize + 8, r.eax);
        r.eax = (r.eax & !0xff) | 1;
        r.esi = s0;
        r.edi = d0;
    }
}
hot_thunk!(t_6bbcb0, p_6bbcb0, "ret 4");

// ---- driveby_diversion_type_info::on_human_killed @ 0x606a30 (thiscall, ret 0x14) ----
unsafe extern "C" fn p_606a30(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(34);
        let s0 = r.esi;
        r.eax = *a.add(1);
        r.esi = r.ecx;
        'end: {
            if r.eax != rd(0x0217_03D4) {
                break 'end;
            }
            r.ecx = *a.add(1);
            r.ecx = rb(r.ecx as usize + 0x111c) as i8 as i32 as u32;
            if r.ecx > 4 {
                break 'end;
            }
            r.eax = rd(r.eax as usize + 0xd74);
            if r.eax == 0 {
                break 'end;
            }
            r.edx = (r.eax & 0xfff) << 4;
            r.ecx = rd(r.edx as usize + 0x0214_9C64);
            if r.ecx == 0 || rd(r.ecx as usize + 0x44) != r.eax || rd(r.ecx as usize + 0x4c) != 5 {
                break 'end;
            }
            if *a.add(3) == 0 {
                break 'end;
            }
            r.edx = r.esi.wrapping_add(0x40);
            let this = r.esi as usize;
            let mut to_l1 = (rd(this + 0x40) as i32) < 0;
            if !to_l1 {
                r.eax = r.edx;
                cx(0x00BD_D230, r, &fr(a, &[]));
                if r.eax & 0xff != 0 {
                    to_l1 = true;
                }
            }
            if to_l1 {
                let mut t = rd(0x00E8_43AC).wrapping_add(0xea60);
                r.eax = t;
                wd(r.edx as usize, t);
                if (t as i32) > 0x6B49_D200 {
                    t = t.wrapping_add(0x94B6_2E00);
                    r.eax = t;
                    wd(r.edx as usize, t);
                }
            } else {
                let mut t = rd(0x00E8_43AC).wrapping_add(0x1388);
                r.eax = t;
                wd(this + 0x3c, t);
                if (t as i32) > 0x6B49_D200 {
                    t = t.wrapping_add(0x94B6_2E00);
                    r.eax = t;
                    wd(this + 0x3c, t);
                }
            }
        }
        r.esi = s0;
    }
}
hot_thunk!(t_606a30, p_606a30, "ret 20");

macro_rules! port {
    ($id:expr, $n:expr, $a:expr, $b:ident, $t:ident) => {
        Port { id: $id, name: $n, addr: $a, body: &$b, thunk: $t }
    };
}

static PORTS: [Port; 34] = [
    port!(0, "choplifter_objective::get_start_message", 0x005751D0, B_5751D0, t_5751d0),
    port!(1, "fuzz_crime_riot::get_bar_title", 0x005CE520, B_5CE520, t_5ce520),
    port!(2, "death_tag_diversion_type_info::get_title", 0x006457E0, B_6457E0, t_6457e0),
    port!(3, "exploration_diversion_type::get_start_message", 0x0060AAC0, B_60AAC0, t_60aac0),
    port!(5, "streaking_diversion_type::create_instance", 0x006164E0, B_6164E0, t_6164e0),
    port!(6, "survival_diversion_type_info::create_instance", 0x0061F120, B_61F120, t_61f120),
    port!(7, "cat_and_mouse_diversion_type_info::create_instance", 0x00641930, B_641930, t_641930),
    port!(8, "base_jumping_diversion_type_info::init", 0x005F9E00, B_5F9E00, t_5f9e00),
    port!(9, "whoring_diversion_type_info::init", 0x00638190, B_638190, t_638190),
    port!(10, "escort_enemy_spawn_group::reset", 0x006794D0, B_6794D0, t_6794d0),
    port!(11, "special_vehicle_spawn_group::reset", 0x00956080, B_956080, t_956080),
    port!(12, "crash_complication::load_from_table", 0x0066E070, B_66E070, t_66e070),
    port!(13, "brass::deactivate", 0x004BD530, B_4BD530, t_4bd530),
    port!(14, "city_view_instances_job", 0x004B6520, B_4B6520, t_4b6520),
    port!(15, "zone_chunk_object_active", 0x004AB370, B_4AB370, t_4ab370),
    port!(16, "cutscene_action_mesh_mover_play", 0x006CC770, B_6CC770, t_6cc770),
    port!(17, "cash_widget::get_height", 0x0074BBB0, B_74BBB0, t_74bbb0),
    port!(18, "widget_info::get_height", 0x0077EFC0, B_77EFC0, t_77efc0),
    port!(19, "nav_frame_update", 0x00A132C0, B_A132C0, t_a132c0),
    port!(20, "vehicle_class_is_1", 0x00AB51B0, B_AB51B0, t_ab51b0),
    port!(21, "profile_options_copy", 0x00690C50, B_690C50, t_690c50),
    port!(22, "gamesave_parse_digits", 0x006953C0, B_6953C0, t_6953c0),
    port!(24, "vint_property_call_setter", 0x00B8A2A0, B_B8A2A0, t_b8a2a0),
    port!(25, "vint_document_parse_sections_v2", 0x00B8CC60, B_B8CC60, t_b8cc60),
    port!(26, "net_recv_create_ack", 0x00805C70, B_805C70, t_805c70),
    port!(27, "net_send_packet_routed", 0x008145B0, B_8145B0, t_8145b0),
    port!(28, "hud_pkt_sub_03", 0x0092D110, B_92D110, t_92d110),
    port!(29, "net_recv_req_team", 0x0082B940, B_82B940, t_82b940),
    port!(30, "vector3_uncompressed::compress", 0x007FE5B0, B_7FE5B0, t_7fe5b0),
    port!(31, "diversion_try_start_offered", 0x00603250, B_603250, t_603250),
    port!(32, "stunt_near_miss_diversion_type::on_game_event_15", 0x0061CF60, B_61CF60, t_61cf60),
    port!(35, "weapon_clip_size", 0x00B73CC0, B_B73CC0, t_b73cc0),
    port!(33, "unlockable_parse_team", 0x006BBCB0, B_6BBCB0, t_6bbcb0),
    port!(34, "driveby_diversion_type_info::on_human_killed", 0x00606A30, B_606A30, t_606a30),
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_s2: replaced {ok}/{} with Rust", PORTS.len()));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let n = CNT[p.id].load(Ordering::Relaxed);
            (n > 0).then(|| format!("{} {n}", p.name))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_s2: {}", parts.join(", "))
}

// ---- the originals' bytes (true ends) ----
const B_5751D0: [u8; 20] = [0x56, 0x57, 0xBF, 0x64, 0x8A, 0xDE, 0x00, 0xBE, 0x90, 0x8A, 0xDE, 0x00, 0xE8, 0xEF, 0xF8, 0x27, 0x00, 0x5F, 0x5E, 0xC3];
const B_5CE520: [u8; 20] = [0x56, 0x57, 0xBF, 0x6C, 0xFA, 0xDE, 0x00, 0xBE, 0x80, 0xFA, 0xDE, 0x00, 0xE8, 0xAF, 0x64, 0x22, 0x00, 0x5F, 0x5E, 0xC3];
const B_6457E0: [u8; 20] = [0x56, 0x57, 0xBF, 0xE0, 0xAB, 0xDF, 0x00, 0xBE, 0xF4, 0xAB, 0xDF, 0x00, 0xE8, 0xEF, 0xF1, 0x1A, 0x00, 0x5F, 0x5E, 0xC3];
const B_60AAC0: [u8; 28] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x68, 0x97, 0x57, 0xDC, 0x00, 0xE8, 0x7C, 0xA0, 0x60, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00];
const B_6164E0: [u8; 33] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x6A, 0x18, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x09, 0xC7, 0x00, 0x4C, 0x5D, 0xDF, 0x00, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
const B_61F120: [u8; 46] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x68, 0x50, 0x01, 0x00, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x13, 0xC7, 0x00, 0x54, 0x71, 0xDF, 0x00, 0xC7, 0x80, 0xF8, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
const B_641930: [u8; 35] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x68, 0x0C, 0x09, 0x00, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x08, 0xE8, 0x95, 0xFE, 0xFF, 0xFF, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00];
const B_5F9E00: [u8; 37] = [0x56, 0x8B, 0xF1, 0xE8, 0x48, 0x8C, 0x00, 0x00, 0x56, 0xC6, 0x46, 0x18, 0x00, 0xE8, 0x8E, 0xFC, 0xFF, 0xFF, 0xC6, 0x86, 0xD4, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x86, 0x3C, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3];
const B_638190: [u8; 36] = [0x56, 0x8B, 0xF1, 0xE8, 0xB8, 0xA8, 0xFC, 0xFF, 0xE8, 0x43, 0xFB, 0xFF, 0xFF, 0x56, 0xE8, 0xAD, 0x06, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x86, 0x3C, 0x07, 0x00, 0x00, 0xC6, 0x46, 0x20, 0x01, 0x88, 0x46, 0x21, 0x5E, 0xC3];
const B_6794D0: [u8; 24] = [0x56, 0x8B, 0xF1, 0xE8, 0xA8, 0xCB, 0x2D, 0x00, 0x33, 0xC0, 0x89, 0x86, 0xE4, 0x01, 0x00, 0x00, 0x89, 0x86, 0xFC, 0x01, 0x00, 0x00, 0x5E, 0xC3];
const B_956080: [u8; 90] = [0x33, 0xC0, 0x88, 0x41, 0x55, 0x88, 0x41, 0x56, 0x88, 0x41, 0x54, 0x89, 0x41, 0x2C, 0x89, 0x41, 0x30, 0x89, 0x41, 0x34, 0x89, 0x41, 0x38, 0x89, 0x41, 0x3C, 0x89, 0x41, 0x40, 0x89, 0x41, 0x44, 0x89, 0x41, 0x48, 0x89, 0x41, 0x4C, 0x89, 0x41, 0x50, 0x89, 0x41, 0x04, 0x89, 0x41, 0x08, 0x89, 0x41, 0x0C, 0x89, 0x41, 0x10, 0x89, 0x41, 0x14, 0x89, 0x41, 0x18, 0x89, 0x41, 0x1C, 0x89, 0x41, 0x20, 0x89, 0x41, 0x24, 0x68, 0x80, 0x01, 0x00, 0x00, 0x89, 0x41, 0x28, 0x50, 0x83, 0xC1, 0x5C, 0x51, 0xE8, 0xEA, 0xAA, 0x34, 0x00, 0x83, 0xC4, 0x0C, 0xC3];
const B_66E070: [u8; 36] = [0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x57, 0x8B, 0xF1, 0xE8, 0x12, 0xFB, 0xFF, 0xFF, 0x57, 0x83, 0xC6, 0x24, 0xB9, 0xDC, 0x8B, 0xDD, 0x00, 0xE8, 0x44, 0x67, 0x50, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x04, 0x00];
const B_4BD530: [u8; 51] = [0x56, 0x8B, 0xF1, 0x8B, 0x46, 0x7C, 0x83, 0x8E, 0x80, 0x00, 0x00, 0x00, 0x08, 0x85, 0xC0, 0x7C, 0x10, 0x50, 0xE8, 0x49, 0xE9, 0x23, 0x00, 0x83, 0xC4, 0x04, 0xC7, 0x46, 0x7C, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x86, 0xC0, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0xCE, 0x5E, 0xE9, 0x8D, 0x82, 0x46, 0x00];
const B_4B6520: [u8; 36] = [0x56, 0x57, 0x8B, 0x7C, 0x24, 0x14, 0x85, 0xFF, 0x7E, 0x17, 0x8B, 0x74, 0x24, 0x10, 0x8B, 0xFF, 0x56, 0xE8, 0xBA, 0xFA, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x83, 0xC6, 0x10, 0x83, 0xEF, 0x01, 0x75, 0xEF, 0x5F, 0x5E, 0xC3];
const B_4AB370: [u8; 48] = [0xE8, 0xEB, 0x13, 0x5D, 0x00, 0x0F, 0xB7, 0xC0, 0x3D, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x1E, 0x3D, 0x66, 0x08, 0x00, 0x00, 0x73, 0x17, 0x8B, 0x04, 0x85, 0xE8, 0xAA, 0xF7, 0x00, 0x85, 0xC0, 0x74, 0x0C, 0xF6, 0x40, 0x1C, 0x01, 0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3];
const B_6CC770: [u8; 40] = [0x56, 0x8B, 0x74, 0x24, 0x08, 0x8B, 0x46, 0x08, 0x85, 0xC0, 0x74, 0x1A, 0x83, 0x7E, 0x10, 0x00, 0x74, 0x14, 0xE8, 0x59, 0xEF, 0x38, 0x00, 0x85, 0xC0, 0x74, 0x0B, 0x8B, 0x4E, 0x10, 0x51, 0x8B, 0xC8, 0xE8, 0xAA, 0xF5, 0x38, 0x00, 0x5E, 0xC3];
const B_74BBB0: [u8; 34] = [0x51, 0x8B, 0x0D, 0x7C, 0x8A, 0xE9, 0x00, 0xE8, 0xA4, 0xA1, 0x4C, 0x00, 0x89, 0x04, 0x24, 0xDB, 0x04, 0x24, 0xDC, 0x0D, 0x28, 0xC2, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00];
const B_77EFC0: [u8; 34] = [0x51, 0x8B, 0x0D, 0x7C, 0x8A, 0xE9, 0x00, 0xE8, 0x94, 0x6D, 0x49, 0x00, 0x89, 0x04, 0x24, 0xDB, 0x04, 0x24, 0xDC, 0x0D, 0x28, 0xC2, 0xE5, 0x00, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC2, 0x04, 0x00];
const B_A132C0: [u8; 23] = [0x51, 0xE8, 0xCA, 0x43, 0x00, 0x00, 0xE8, 0xF5, 0x99, 0x00, 0x00, 0xE8, 0x20, 0x7D, 0xFF, 0xFF, 0xE8, 0x3B, 0x8C, 0xFF, 0xFF, 0x59, 0xC3];
const B_AB51B0: [u8; 28] = [0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x80, 0xE4, 0x84, 0x00, 0x00, 0x33, 0xC9, 0x83, 0xB8, 0x9C, 0x00, 0x00, 0x00, 0x01, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3];
const B_690C50: [u8; 120] = [0x8B, 0x08, 0x89, 0x0A, 0x56, 0x57, 0x8D, 0x70, 0x04, 0x8D, 0x7A, 0x04, 0xB9, 0x15, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x8B, 0x48, 0x58, 0x89, 0x4A, 0x58, 0x8B, 0x48, 0x5C, 0x89, 0x4A, 0x5C, 0x8B, 0x48, 0x60, 0x89, 0x4A, 0x60, 0x66, 0x8B, 0x48, 0x64, 0x66, 0x89, 0x4A, 0x64, 0x0F, 0xB6, 0x48, 0x66, 0x88, 0x4A, 0x66, 0x8B, 0x48, 0x68, 0x89, 0x4A, 0x68, 0x8D, 0x70, 0x6C, 0x8D, 0x7A, 0x6C, 0xB9, 0x0C, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0xA4, 0x0F, 0xB6, 0x88, 0x9D, 0x00, 0x00, 0x00, 0x88, 0x8A, 0x9D, 0x00, 0x00, 0x00, 0x8B, 0x88, 0xA0, 0x00, 0x00, 0x00, 0x89, 0x8A, 0xA0, 0x00, 0x00, 0x00, 0x8D, 0xB0, 0xA4, 0x00, 0x00, 0x00, 0x8D, 0xBA, 0xA4, 0x00, 0x00, 0x00, 0xB9, 0x0C, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0xA4, 0x5F, 0x5E, 0xC3];
const B_6953C0: [u8; 106] = [0x55, 0x56, 0x57, 0x8B, 0xF9, 0x33, 0xED, 0x85, 0xFF, 0x8B, 0xF0, 0x75, 0x06, 0x5F, 0x5E, 0x33, 0xC0, 0x5D, 0xC3, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x53, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8A, 0x1F, 0x83, 0xEE, 0x01, 0x80, 0xFB, 0x2D, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0xEB, 0x2C, 0x80, 0xFB, 0x30, 0x7C, 0x2E, 0x80, 0xFB, 0x39, 0x7F, 0x29, 0x85, 0xF6, 0x8B, 0xD6, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x7E, 0x0C, 0x8D, 0x0C, 0x89, 0x83, 0xEA, 0x01, 0x03, 0xC9, 0x85, 0xD2, 0x7F, 0xF4, 0x0F, 0xBE, 0xD3, 0x83, 0xEA, 0x30, 0x0F, 0xAF, 0xD1, 0x03, 0xEA, 0x83, 0xC7, 0x01, 0x85, 0xF6, 0x7F, 0xBE, 0x5B, 0x0F, 0xAF, 0xC5, 0x5F, 0x5E, 0x5D, 0xC3];
const B_B8A2A0: [u8; 31] = [0x8B, 0x40, 0x08, 0x85, 0xC0, 0x74, 0x13, 0x8B, 0x54, 0x24, 0x08, 0x51, 0x8B, 0x4C, 0x24, 0x08, 0x52, 0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x0C, 0xC2, 0x08, 0x00, 0x32, 0xC0, 0xC2, 0x08, 0x00];
const B_B8CC60: [u8; 58] = [0x56, 0x8B, 0xF0, 0x8B, 0xC7, 0xE8, 0xB6, 0xE2, 0xFF, 0xFF, 0x56, 0x8B, 0xC7, 0xE8, 0x0E, 0xE2, 0xFF, 0xFF, 0x56, 0x8B, 0xC7, 0xE8, 0x16, 0xFE, 0xFF, 0xFF, 0x56, 0x8B, 0xC7, 0xE8, 0x0E, 0xFF, 0xFF, 0xFF, 0x6A, 0x00, 0x56, 0x8B, 0xC7, 0xE8, 0x14, 0xF7, 0xFF, 0xFF, 0x6A, 0x01, 0x56, 0x8B, 0xC7, 0xE8, 0x0A, 0xF7, 0xFF, 0xFF, 0xB0, 0x01, 0x5E, 0xC3];
const B_805C70: [u8; 64] = [0x8B, 0x44, 0x24, 0x04, 0x0F, 0xB7, 0x00, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02, 0x0F, 0xB7, 0xC8, 0x6A, 0x00, 0x51, 0xE8, 0xA7, 0x31, 0x00, 0x00, 0x85, 0xC0, 0x5E, 0x74, 0x21, 0x8B, 0x00, 0x85, 0xC0, 0x74, 0x1B, 0x8B, 0x54, 0x24, 0x08, 0x8B, 0x4A, 0x04, 0x8A, 0x89, 0xB0, 0x01, 0x00, 0x00, 0xBA, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE2, 0x66, 0x09, 0x90, 0x7E, 0x06, 0x00, 0x00, 0xC3];
const B_8145B0: [u8; 110] = [0x56, 0x8B, 0xF0, 0x8B, 0x86, 0xAC, 0x01, 0x00, 0x00, 0x8B, 0x88, 0x08, 0x04, 0x00, 0x00, 0x0F, 0xB6, 0x02, 0x83, 0xF8, 0x32, 0x7D, 0x0C, 0x8D, 0x04, 0x40, 0x8D, 0x04, 0xC5, 0x08, 0xBF, 0xE8, 0x00, 0xEB, 0x0E, 0x8D, 0x84, 0x40, 0x6A, 0xFF, 0xFF, 0xFF, 0x8D, 0x04, 0xC5, 0xA8, 0xB6, 0xE8, 0x00, 0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x75, 0x14, 0x85, 0xC9, 0x74, 0x30, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0x88, 0x04, 0x04, 0x00, 0x00, 0x74, 0x19, 0x5E, 0xC3, 0x83, 0xF8, 0x01, 0x75, 0x12, 0x85, 0xC9, 0x74, 0x0E, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0x88, 0x04, 0x04, 0x00, 0x00, 0x74, 0x09, 0x52, 0xE8, 0x77, 0x5D, 0xFE, 0xFF, 0x83, 0xC4, 0x04, 0x5E, 0xC3];
const B_92D110: [u8; 43] = [0x8B, 0x4C, 0x24, 0x04, 0x8A, 0x01, 0x56, 0x8B, 0x71, 0x01, 0x88, 0x44, 0x24, 0x08, 0xE8, 0xBD, 0xF1, 0xFF, 0xFF, 0x85, 0xC0, 0x5E, 0x74, 0x12, 0x6A, 0x00, 0x6A, 0x01, 0x50, 0x8B, 0x44, 0x24, 0x10, 0x50, 0xE8, 0x79, 0x03, 0x00, 0x00, 0x83, 0xC4, 0x10, 0xC3];
const B_82B940: [u8; 81] = [0x8B, 0x44, 0x24, 0x08, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0x45, 0x66, 0x8B, 0x80, 0xA8, 0x01, 0x00, 0x00, 0xE8, 0xC9, 0xC6, 0xFC, 0xFF, 0x85, 0xC0, 0x74, 0x35, 0x83, 0x3D, 0x00, 0x6C, 0xE9, 0x00, 0x06, 0x8B, 0x4C, 0x24, 0x04, 0x0F, 0xBE, 0x11, 0x75, 0x1E, 0x8B, 0x0D, 0x7C, 0x6B, 0xE9, 0x00, 0x83, 0xF9, 0xFF, 0x7E, 0x13, 0x83, 0x3C, 0x8D, 0x60, 0xE7, 0x44, 0x01, 0x05, 0x75, 0x09, 0x80, 0x3D, 0xB5, 0x91, 0x52, 0x02, 0x00, 0x75, 0x07, 0x52, 0xE8, 0x01, 0x3F, 0x05, 0x00, 0x59, 0xC3];
const B_7FE5B0: [u8; 52] = [0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x29, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x85, 0xF6, 0x74, 0x1F, 0x57, 0x8B, 0x7C, 0x24, 0x18, 0x85, 0xFF, 0x74, 0x15, 0xE8, 0xA1, 0xF2, 0xFF, 0xFF, 0x8B, 0x06, 0x89, 0x07, 0x8B, 0x4E, 0x04, 0x89, 0x4F, 0x04, 0x8B, 0x56, 0x08, 0x89, 0x57, 0x08, 0x5F, 0x5E, 0xC2, 0x10, 0x00];
const B_603250: [u8; 66] = [0xA1, 0x0C, 0x7C, 0xE9, 0x00, 0x8B, 0x0C, 0x85, 0x68, 0xB1, 0x40, 0x01, 0x8B, 0x11, 0x8B, 0x42, 0x40, 0xFF, 0xD0, 0x3B, 0x44, 0x24, 0x04, 0x75, 0x28, 0x8B, 0x0D, 0x0C, 0x7C, 0xE9, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0x39, 0x05, 0x00, 0x6C, 0xE9, 0x00, 0x89, 0x0D, 0x10, 0x7C, 0xE9, 0x00, 0x74, 0x0F, 0xA3, 0x04, 0x6C, 0xE9, 0x00, 0xC7, 0x05, 0xB4, 0x7B, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0xC3];
const B_61CF60: [u8; 81] = [0x8B, 0x15, 0xD4, 0x03, 0x17, 0x02, 0x83, 0xBA, 0x74, 0x0D, 0x00, 0x00, 0x00, 0x74, 0x3F, 0x83, 0x7C, 0x24, 0x10, 0x00, 0x74, 0x38, 0x8B, 0x44, 0x24, 0x08, 0x85, 0xC0, 0x74, 0x08, 0x8B, 0x40, 0x44, 0x3B, 0x42, 0x44, 0x75, 0x28, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0xE8, 0x03, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x41, 0x44, 0x7E, 0x08, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x41, 0x44, 0x83, 0x79, 0x3C, 0x00, 0x7E, 0x06, 0x51, 0xE8, 0x32, 0xFF, 0xFF, 0xFF, 0xC2, 0x14, 0x00];
const B_B73CC0: [u8; 72] = [0x57, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x1F, 0x8B, 0x87, 0xCC, 0x10, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x08, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0xEB, 0x02, 0x33, 0xC0, 0x3B, 0xC6, 0x75, 0x07, 0xE8, 0x4C, 0x54, 0xE5, 0xFF, 0xEB, 0x05, 0x8A, 0x46, 0x0D, 0x24, 0x01, 0x84, 0xC0, 0x5F, 0x74, 0x0D, 0x8B, 0x46, 0x08, 0x0F, 0xB7, 0x80, 0xDC, 0x00, 0x00, 0x00, 0x03, 0xC0, 0xC3, 0x8B, 0x4E, 0x08, 0x0F, 0xB7, 0x81, 0xDC, 0x00, 0x00, 0x00, 0xC3];
const B_6BBCB0: [u8; 83] = [0x85, 0xC0, 0x75, 0x05, 0x32, 0xC0, 0xC2, 0x04, 0x00, 0x56, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x57, 0x74, 0x1B, 0x8B, 0x06, 0x68, 0x10, 0xAB, 0xDD, 0x00, 0x50, 0xE8, 0x61, 0xD3, 0x5D, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x29, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xC0, 0x68, 0xD0, 0x0B, 0xDD, 0x00, 0xBF, 0x08, 0x00, 0x00, 0x00, 0xE8, 0x92, 0x98, 0x4B, 0x00, 0x8B, 0x4C, 0x24, 0x10, 0x83, 0xC4, 0x04, 0x5F, 0x89, 0x41, 0x08, 0xB0, 0x01, 0x5E, 0xC2, 0x04, 0x00, 0x8B, 0xC6, 0xEB, 0xDC];
const B_606A30: [u8; 166] = [0x8B, 0x44, 0x24, 0x08, 0x3B, 0x05, 0xD4, 0x03, 0x17, 0x02, 0x56, 0x8B, 0xF1, 0x0F, 0x85, 0x8F, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x08, 0x0F, 0xBE, 0x89, 0x1C, 0x11, 0x00, 0x00, 0x83, 0xF9, 0x04, 0x77, 0x7F, 0x8B, 0x80, 0x74, 0x0D, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x75, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x8A, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x60, 0x39, 0x41, 0x44, 0x75, 0x5B, 0x83, 0x79, 0x4C, 0x05, 0x75, 0x55, 0x83, 0x7C, 0x24, 0x14, 0x00, 0x74, 0x4E, 0x83, 0x7E, 0x40, 0x00, 0x8D, 0x56, 0x40, 0x7C, 0x2B, 0x8B, 0xC2, 0xE8, 0x9C, 0x67, 0x5D, 0x00, 0x84, 0xC0, 0x75, 0x20, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0x88, 0x13, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x46, 0x3C, 0x7E, 0x26, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x46, 0x3C, 0x5E, 0xC2, 0x14, 0x00, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0x60, 0xEA, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x02, 0x7E, 0x07, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x02, 0x5E, 0xC2, 0x14, 0x00];
