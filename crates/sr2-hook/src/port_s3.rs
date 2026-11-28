//! Port module s3: small functions (list s3): tail-call wrappers, small class methods, string-helper
//! shapes. Every address is pinned to its whole original body by `replace_pinned`.
//!
//! Every body threads the original's register flow through `St` (the callee sees the registers
//! the original has at the call and leaves its own in them), so the ECX / EDX the original leaves
//! behind are reproduced. Every thunk saves XMM0-7 and `pushad`s; the Rust body edits the saved
//! registers, `popad` restores them. Acceptance: the shared judge (sr2-judge).

#![allow(unused_imports)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const COUNT: usize = 22;

const CAP: u32 = 1 << 24;
static CNT: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

#[inline(always)]
fn count(i: usize) {
    let c = &CNT[i];
    if c.load(Ordering::Relaxed) < CAP {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// The seven general registers as a straight-line asm function would hold them.
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

    /// Call a game function by address with these registers; its leftovers stay in the struct.
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

    /// Write every register back to the saved block.
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

// --- tail-call wrappers: the callee sees the caller's registers and returns for us ---------------

macro_rules! tail0 {
    ($e:ident, $t:ident, $i:expr, $f:expr) => {
        extern "C" fn $e(r: &mut Pushad, _a: *const u32) {
            unsafe {
                count($i);
                let mut s = St::of(r);
                s.call($f, &[]);
                s.put(r);
            }
        }
        thunk!($t, $e, "ret");
    };
}

tail0!(e_5d2a70, t_5d2a70, 0, 0x005C_3BB0);
tail0!(e_80b610, t_80b610, 1, 0x0080_AB90);

// call 602ed0, then tail jump to 681370.
extern "C" fn e_6150f0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(2);
        let mut s = St::of(r);
        s.call(0x0060_2ED0, &[]);
        s.call(0x0068_1370, &[]);
        s.put(r);
    }
}
thunk!(t_6150f0, e_6150f0, "ret");

// init: esi = this; call 602a50; clear two bytes, -1 into two dwords.
extern "C" fn e_616f80(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(3);
        let esi0 = r.esi;
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.call(0x0060_2A50, &[]);
        let this = s.esi as usize;
        wb(this + 0x39, 0);
        wb(this + 0x38, 0);
        s.eax = 0xFFFF_FFFF;
        wd(this + 0x40, 0xFFFF_FFFF);
        wd(0x00EA_03A4, 0xFFFF_FFFF);
        s.put(r);
        r.esi = esi0;
    }
}
thunk!(t_616f80, e_616f80, "ret");

// update: a flag byte picks between `f2` with ESI = this and `f1(this)` (stack argument).
macro_rules! upd {
    ($e:ident, $t:ident, $i:expr, $off:expr, $f1:expr, $f2:expr) => {
        extern "C" fn $e(r: &mut Pushad, _a: *const u32) {
            unsafe {
                count($i);
                let this = r.ecx;
                let esi0 = r.esi;
                let mut s = St::of(r);
                if rb(this as usize + $off) == 0 {
                    s.esi = this;
                    s.call($f2, &[]);
                    s.put(r);
                    r.esi = esi0;
                } else {
                    s.call($f1, &[this]);
                    s.put(r);
                }
            }
        }
        thunk!($t, $e, "ret");
    };
}

upd!(e_61a7f0, t_61a7f0, 4, 0xC8, 0x0061_A810, 0x0061_AC00);
upd!(e_61d400, t_61d400, 5, 0x1C, 0x0061_D450, 0x0061_D690);

// get_screen_size(a0, a1): a1 = 0, then a tail jump (ret 8).
extern "C" fn e_b9c870(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(6);
        wd(a as usize + 4, 0);
        let mut s = St::of(r);
        s.call(0x00B9_6B50, &[rd(a as usize), 0]);
        s.put(r);
    }
}
thunk!(t_b9c870, e_b9c870, "ret 8");

// begin: ++[this+0x28]; ecx = this + 4; a stdcall import with ecx.
extern "C" fn e_bf3e30(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(7);
        let this = r.ecx as usize;
        wd(this + 0x28, rd(this + 0x28).wrapping_add(1));
        let mut s = St::of(r);
        s.ecx = (this + 4) as u32;
        s.calli(rd(0x00DB_80A4), &[s.ecx]);
        s.put(r);
    }
}
thunk!(t_bf3e30, e_bf3e30, "ret");

// eax = [this + off]; esi = arg0; call b93590(eax); AL = 1 (ret 8).
macro_rules! getv {
    ($e:ident, $t:ident, $i:expr, $off:expr) => {
        extern "C" fn $e(r: &mut Pushad, a: *const u32) {
            unsafe {
                count($i);
                let esi0 = r.esi;
                let mut s = St::of(r);
                s.eax = rd(r.ecx as usize + $off);
                s.esi = rd(a as usize);
                s.call(0x00B9_3590, &[s.eax]);
                s.eax = (s.eax & !0xFF) | 1;
                s.put(r);
                r.esi = esi0;
            }
        }
        thunk!($t, $e, "ret 8");
    };
}

getv!(e_b96960, t_b96960, 8, 0x54);
getv!(e_ba3730, t_ba3730, 9, 0xB4);

// get_title / string-helper shapes: push esi / edi; edi = X; esi = Y; call; pop; EAX = result.
macro_rules! gt {
    ($e:ident, $t:ident, $i:expr, $f:expr, $edi:expr, $esi:expr) => {
        extern "C" fn $e(r: &mut Pushad, _a: *const u32) {
            unsafe {
                count($i);
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
        thunk!($t, $e, "ret");
    };
}

gt!(e_5fbc80, t_5fbc80, 10, 0x007F_4AD0, 0x00DF_26D4, 0x00DF_26E0);
gt!(e_60b8d0, t_60b8d0, 11, 0x007F_49E0, 0x00DF_4520, 0x00DF_4534);
gt!(e_611270, t_611270, 12, 0x007F_49E0, 0x00DF_50C0, 0x00DF_50D4);

// get_start_message(a0, a1): the title helper, then `c14c40(eax)` with EDI = a0, ESI = a1 (ret 8).
extern "C" fn e_62e960(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(13);
        let mut s = St::of(r);
        s.esi = 0x00DF_8E78;
        s.edi = 0;
        s.call(0x007F_49E0, &[]);
        s.esi = rd(a as usize + 4);
        s.edi = rd(a as usize);
        s.call(0x00C1_4C40, &[s.eax]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.ebp = s.ebp;
    }
}
thunk!(t_62e960, e_62e960, "ret 8");

// chunk_section_22: edi = [esi]; ecx = the address of the first stack argument; call 4ac260.
extern "C" fn e_4aedb0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(14);
        let edi0 = r.edi;
        let mut s = St::of(r);
        s.edi = rd(r.esi as usize);
        s.ecx = a as u32;
        s.call(0x004A_C260, &[]);
        wd(s.esi as usize + 0x224, s.eax);
        s.put(r);
        r.edi = edi0;
    }
}
thunk!(t_4aedb0, e_4aedb0, "ret");

// hud_pkt_sub_13(p): c(p[0], float(p[1]), p[2] != 0) through the x87 (a signalling NaN is quieted).
extern "C" fn e_4c3be0(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(15);
        let p = rd(a as usize);
        let pp = p as usize;
        let ecx = rd(pp);
        let edx = rd(pp + 4);
        let al = (rb(pp + 8) != 0) as u32;
        let eax = (p & !0xFF) | al;
        wd(a as usize, edx);
        let fbits = ((f32::from_bits(edx) as f64) as f32).to_bits();
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.call(0x004C_3810, &[ecx, fbits, eax]);
        s.put(r);
    }
}
thunk!(t_4c3be0, e_4c3be0, "ret");

// input_action_button_name(EAX = index): the table entry, or a default string.
extern "C" fn e_4ffbb0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(16);
        let v = rd((r.eax << 4).wrapping_add(0x00F9_EA38) as usize);
        if v == 0xFFFF_FFFF {
            r.eax = 0x00DD_FE9C;
        } else {
            let mut s = St::of(r);
            s.eax = v;
            s.call(0x007F_36D0, &[v]);
            s.put(r);
        }
    }
}
thunk!(t_4ffbb0, e_4ffbb0, "ret");

// load_xtbl(a0): 680e40(a0, 1); then a0 := [0x2527b14] and a tail jump to 67ac60 (ret 4).
extern "C" fn e_598030(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(17);
        let a0 = rd(a as usize);
        let mut s = St::of(r);
        s.eax = a0;
        s.call(0x0068_0E40, &[a0, 1]);
        let ecx = rd(0x0252_7B14);
        wd(a as usize, ecx);
        s.ecx = ecx;
        s.call(0x0067_AC60, &[ecx]);
        s.put(r);
    }
}
thunk!(t_598030, e_598030, "ret 4");

// create_instance(a0): a virtual call (0xa4c, 0) through [[a0] + 0x1c]; on a result call 606b40.
extern "C" fn e_606b10(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(18);
        let ecx = rd(a as usize);
        let eax = rd(ecx as usize);
        let edx = rd(eax as usize + 0x1C);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = ecx;
        s.edx = edx;
        s.calli(edx, &[0xA4C, 0]);
        if s.eax != 0 {
            s.call(0x0060_6B40, &[]);
        } else {
            s.eax = 0;
        }
        s.put(r);
    }
}
thunk!(t_606b10, e_606b10, "ret 4");

// chunk_request_history_find(DX = id): index of the 16-byte entry (of 10) whose word equals DX, or -1.
extern "C" fn e_a80de0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(19);
        let dx = r.edx as u16;
        let mut eax = 0u32;
        let mut ecx = 0x02F9_B8A8u32;
        loop {
            if *(ecx as usize as *const u16) == dx {
                break;
            }
            ecx = ecx.wrapping_add(0x10);
            eax = eax.wrapping_add(1);
            if ecx >= 0x02F9_B948 {
                eax = 0xFFFF_FFFF;
                break;
            }
        }
        r.eax = eax;
        r.ecx = ecx;
    }
}
thunk!(t_a80de0, e_a80de0, "ret");

// set_target_handle(a0): esi = this; ecx = a0; call 634ba0; [esi+0x88] = result; zero two fields; AL = 1.
extern "C" fn e_b9f410(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(20);
        let esi0 = r.esi;
        let mut s = St::of(r);
        s.esi = r.ecx;
        s.ecx = rd(a as usize);
        s.call(0x0063_4BA0, &[]);
        let this = s.esi as usize;
        wd(this + 0x88, s.eax);
        wd(this + 0x84, 0);
        wd(this + 0x8C, 0);
        s.eax = 1;
        s.put(r);
        r.esi = esi0;
    }
}
thunk!(t_b9f410, e_b9f410, "ret 8");

// post_render: when the byte at [this+0xa0] is 1, EDX = [this+0x30] and a tail jump to b8b990.
extern "C" fn e_ba4f30(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(21);
        let this = r.ecx as usize;
        if rb(this + 0xA0) == 1 {
            let mut s = St::of(r);
            s.edx = rd(this + 0x30);
            s.call(0x00B8_B990, &[]);
            s.put(r);
        }
    }
}
thunk!(t_ba4f30, e_ba4f30, "ret");

const B_5D2A70: [u8; 5] = [0xE9, 0x3B, 0x11, 0xFF, 0xFF,];
const B_80B610: [u8; 5] = [0xE9, 0x7B, 0xF5, 0xFF, 0xFF,];
const B_6150F0: [u8; 10] = [0xE8, 0xDB, 0xDD, 0xFE, 0xFF, 0xE9, 0x76, 0xC2, 0x06, 0x00,];
const B_616F80: [u8; 29] = [0x56, 0x8B, 0xF1, 0xE8, 0xC8, 0xBA, 0xFE, 0xFF, 0x32, 0xC0, 0x88, 0x46, 0x39, 0x88, 0x46, 0x38, 0x83, 0xC8, 0xFF, 0x89, 0x46, 0x40, 0xA3, 0xA4, 0x03, 0xEA, 0x00, 0x5E, 0xC3,];
const B_61A7F0: [u8; 26] = [0x80, 0xB9, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x74, 0x07, 0x51, 0xE8, 0x11, 0x00, 0x00, 0x00, 0xC3, 0x56, 0x8B, 0xF1, 0xE8, 0xF8, 0x03, 0x00, 0x00, 0x5E, 0xC3,];
const B_61D400: [u8; 23] = [0x80, 0x79, 0x1C, 0x00, 0x74, 0x07, 0x51, 0xE8, 0x44, 0x00, 0x00, 0x00, 0xC3, 0x56, 0x8B, 0xF1, 0xE8, 0x7B, 0x02, 0x00, 0x00, 0x5E, 0xC3,];
const B_B9C870: [u8; 13] = [0xC7, 0x44, 0x24, 0x08, 0x00, 0x00, 0x00, 0x00, 0xE9, 0xD3, 0xA2, 0xFF, 0xFF,];
const B_BF3E30: [u8; 15] = [0x83, 0x41, 0x28, 0x01, 0x83, 0xC1, 0x04, 0x51, 0xFF, 0x15, 0xA4, 0x80, 0xDB, 0x00, 0xC3,];
const B_B96960: [u8; 20] = [0x8B, 0x41, 0x54, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x22, 0xCC, 0xFF, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,];
const B_BA3730: [u8; 23] = [0x8B, 0x81, 0xB4, 0x00, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x50, 0xE8, 0x4F, 0xFE, 0xFE, 0xFF, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,];
const B_5FBC80: [u8; 20] = [0x56, 0x57, 0xBF, 0xD4, 0x26, 0xDF, 0x00, 0xBE, 0xE0, 0x26, 0xDF, 0x00, 0xE8, 0x3F, 0x8E, 0x1F, 0x00, 0x5F, 0x5E, 0xC3,];
const B_60B8D0: [u8; 20] = [0x56, 0x57, 0xBF, 0x20, 0x45, 0xDF, 0x00, 0xBE, 0x34, 0x45, 0xDF, 0x00, 0xE8, 0xFF, 0x90, 0x1E, 0x00, 0x5F, 0x5E, 0xC3,];
const B_611270: [u8; 20] = [0x56, 0x57, 0xBF, 0xC0, 0x50, 0xDF, 0x00, 0xBE, 0xD4, 0x50, 0xDF, 0x00, 0xE8, 0x5F, 0x37, 0x1E, 0x00, 0x5F, 0x5E, 0xC3,];
const B_62E960: [u8; 36] = [0x56, 0x57, 0x33, 0xFF, 0xBE, 0x78, 0x8E, 0xDF, 0x00, 0xE8, 0x72, 0x60, 0x1C, 0x00, 0x8B, 0x74, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x0C, 0x50, 0xE8, 0xC4, 0x62, 0x5E, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0xC2, 0x08, 0x00,];
const B_4AEDB0: [u8; 20] = [0x57, 0x8B, 0x3E, 0x8D, 0x4C, 0x24, 0x08, 0xE8, 0xA4, 0xD4, 0xFF, 0xFF, 0x89, 0x86, 0x24, 0x02, 0x00, 0x00, 0x5F, 0xC3,];
const B_4C3BE0: [u8; 39] = [0x8B, 0x44, 0x24, 0x04, 0x8B, 0x08, 0x80, 0x78, 0x08, 0x00, 0x8B, 0x50, 0x04, 0x0F, 0x95, 0xC0, 0x89, 0x54, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0x50, 0x51, 0xD9, 0x1C, 0x24, 0x51, 0xE8, 0x0D, 0xFC, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0xC3,];
const B_4FFBB0: [u8; 30] = [0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x38, 0xEA, 0xF9, 0x00, 0x83, 0xF8, 0xFF, 0x75, 0x06, 0xB8, 0x9C, 0xFE, 0xDD, 0x00, 0xC3, 0x50, 0xE8, 0x06, 0x3B, 0x2F, 0x00, 0x83, 0xC4, 0x04, 0xC3,];
const B_598030: [u8; 30] = [0x8B, 0x44, 0x24, 0x04, 0x6A, 0x01, 0x50, 0xE8, 0x04, 0x8E, 0x0E, 0x00, 0x8B, 0x0D, 0x14, 0x7B, 0x52, 0x02, 0x83, 0xC4, 0x08, 0x89, 0x4C, 0x24, 0x04, 0xE9, 0x12, 0x2C, 0x0E, 0x00,];
const B_606B10: [u8; 35] = [0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x01, 0x8B, 0x50, 0x1C, 0x6A, 0x00, 0x68, 0x4C, 0x0A, 0x00, 0x00, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x08, 0xE8, 0x15, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00, 0x33, 0xC0, 0xC2, 0x04, 0x00,];
const B_A80DE0: [u8; 30] = [0x33, 0xC0, 0xB9, 0xA8, 0xB8, 0xF9, 0x02, 0x66, 0x39, 0x11, 0x74, 0x11, 0x83, 0xC1, 0x10, 0x83, 0xC0, 0x01, 0x81, 0xF9, 0x48, 0xB9, 0xF9, 0x02, 0x7C, 0xED, 0x83, 0xC8, 0xFF, 0xC3,];
const B_B9F410: [u8; 38] = [0x56, 0x8B, 0xF1, 0x8B, 0x4C, 0x24, 0x08, 0xE8, 0x84, 0x57, 0xA9, 0xFF, 0x89, 0x86, 0x88, 0x00, 0x00, 0x00, 0x33, 0xC0, 0x89, 0x86, 0x84, 0x00, 0x00, 0x00, 0x89, 0x86, 0x8C, 0x00, 0x00, 0x00, 0xB0, 0x01, 0x5E, 0xC2, 0x08, 0x00,];
const B_BA4F30: [u8; 18] = [0x80, 0xB9, 0xA0, 0x00, 0x00, 0x00, 0x01, 0x75, 0x08, 0x8B, 0x51, 0x30, 0xE9, 0x4F, 0x6A, 0xFE, 0xFF, 0xC3,];

struct Port {
    name: &'static str,
    addr: usize,
    body: &'static [u8],
    thunk: extern "C" fn(),
}

static PORTS: [Port; COUNT] = [
    Port { name: "fuzz_crime_stolen_car::update", addr: 0x5D2A70, body: &B_5D2A70, thunk: t_5d2a70 },
    Port { name: "net_recv_snd_lmt", addr: 0x80B610, body: &B_80B610, thunk: t_80b610 },
    Port { name: "racing_diversion_instance::stop", addr: 0x6150F0, body: &B_6150F0, thunk: t_6150f0 },
    Port { name: "stunt_back_seat_driver_diversion_type::init", addr: 0x616F80, body: &B_616F80, thunk: t_616f80 },
    Port { name: "stunt_hydraulics_diversion_type::update", addr: 0x61A7F0, body: &B_61A7F0, thunk: t_61a7f0 },
    Port { name: "stunt_oncoming_traffic_diversion_type::update", addr: 0x61D400, body: &B_61D400, thunk: t_61d400 },
    Port { name: "vint_element_group::get_screen_size", addr: 0xB9C870, body: &B_B9C870, thunk: t_b9c870 },
    Port { name: "scheduling_trigger::begin", addr: 0xBF3E30, body: &B_BF3E30, thunk: t_bf3e30 },
    Port { name: "vint_element_base::get_depth", addr: 0xB96960, body: &B_B96960, thunk: t_b96960 },
    Port { name: "vint_element_text::get_vert_align", addr: 0xBA3730, body: &B_BA3730, thunk: t_ba3730 },
    Port { name: "blackjack_diversion_type_info::get_title", addr: 0x5FBC80, body: &B_5FBC80, thunk: t_5fbc80 },
    Port { name: "firetruck_diversion_type_info::get_title", addr: 0x60B8D0, body: &B_60B8D0, thunk: t_60b8d0 },
    Port { name: "flashing_diversion_type::get_title", addr: 0x611270, body: &B_611270, thunk: t_611270 },
    Port { name: "tow_truck_diversion_type_info::get_start_message", addr: 0x62E960, body: &B_62E960, thunk: t_62e960 },
    Port { name: "chunk_section_22", addr: 0x4AEDB0, body: &B_4AEDB0, thunk: t_4aedb0 },
    Port { name: "hud_pkt_sub_13", addr: 0x4C3BE0, body: &B_4C3BE0, thunk: t_4c3be0 },
    Port { name: "input_action_button_name", addr: 0x4FFBB0, body: &B_4FFBB0, thunk: t_4ffbb0 },
    Port { name: "racing_activity_level_instance::load_xtbl", addr: 0x598030, body: &B_598030, thunk: t_598030 },
    Port { name: "driveby_diversion_type_info::create_instance", addr: 0x606B10, body: &B_606B10, thunk: t_606b10 },
    Port { name: "chunk_request_history_find", addr: 0xA80DE0, body: &B_A80DE0, thunk: t_a80de0 },
    Port { name: "vint_object_tween::set_target_handle", addr: 0xB9F410, body: &B_B9F410, thunk: t_b9f410 },
    Port { name: "vint_element_clip::post_render", addr: 0xBA4F30, body: &B_BA4F30, thunk: t_ba4f30 },
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    for p in PORTS.iter() {
        if replace_pinned(p.name, p.addr, p.body, p.thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_s3: replaced {ok}/{COUNT} with Rust"));
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
    format!("port_s3: {}", parts.join(", "))
}
