//! Port module h5: small functions from the round 9 list (small game functions, 16-200 bytes).
//!
//! Every address is pinned to its whole original body by `replace_pinned`. Each port runs behind a
//! `pushad` thunk that also saves XMM0-7; the Rust body edits the saved registers and `popad`
//! restores them. Callees stay original: they are called by address (`St::call` / `St::calli`)
//! with the registers the original sets up, and the registers each callee leaves behind are copied
//! back, so ECX / EDX / EBX / EBP / EDI flow out exactly as the original leaves them.
//!
//! Shapes:
//! - `gt` (`get_title`): ESI / EDI constants, one call, EAX = its result;
//! - `objects_deactivate_all`: a loop over a linked list of objects, one virtual call per object;
//! - `profile_clear_and_init`: a memset, two stores, then a tail jump (here: a call and return);
//! - `cutscene_action_exit`: x87 only, with the scratch float stored over the pushed ECX;
//! - `smesh_cache_record_add_chunk`: a 6-slot word table search, pure;
//! - `audio_category_from_name`: a stricmp loop over a 4-entry table.
//!
//! Checks: per-function call counters (`summary`); acceptance is the judge (`sr2-judge`).

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

/// The seven general registers as a straight-line asm function would hold them. `call` runs a game
/// function with exactly these registers and leaves in the struct what the callee left.
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

    /// Call through a function pointer read from game memory (a vtable slot); returns its EAX.
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

// --- get_title shape -----------------------------------------------------------------------------
// push esi / edi; edi = X; esi = Y; call the string helper (ESI / EDI in); pop; EAX = its result.

macro_rules! gt {
    ($e:ident, $t:ident, $i:expr, $ret:literal, $f:expr, $edi:expr, $esi:expr) => {
        extern "C" fn $e(r: &mut Pushad, _a: *const u32) {
            unsafe {
                count($i);
                // push esi / edi: EBX / EBP flow out
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

gt!(e_60aaf0, t_60aaf0, 0, "ret", 0x007F_4AD0, 0x00DF_42A0, 0x00DF_4344);

// --- objects_deactivate_all ----------------------------------------------------------------------
// The global list head at 0x25295EC; each node holds the object at +0 and the node's value at +0xC.
// Walk: eax = node.value; esi = (eax != head) ? eax : 0; call vtable slot 5 (+0x14) with ECX = object;
// then ECX = ESI (whatever the callee left there).

const OBJ_HEAD: usize = 0x0252_95EC;

extern "C" fn e_924fe0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(1);
        let mut s = St::of(r);
        let mut ecx = rd(OBJ_HEAD);
        while ecx != 0 {
            let eax = rd(ecx as usize + 0xC);
            let head = rd(OBJ_HEAD);
            let next = if eax.wrapping_sub(head) != 0 { eax } else { 0 };
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 0x14);
            s.eax = vt;
            s.ecx = ecx;
            s.edx = f;
            s.esi = next;
            s.calli(f, &[]);
            ecx = s.esi;
        }
        r.eax = s.eax;
        r.ecx = ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_924fe0, e_924fe0, "ret");

// --- profile_clear_and_init ----------------------------------------------------------------------
// memset(ESI, 0, 0xBD8); zero +0xBC8 and +0xBCC; EAX = ESI; tail jump to 0x690D00 (called here).

extern "C" fn e_690cd0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(2);
        let obj = r.esi;
        let mut s = St::of(r);
        s.call(0x00CA_0BC0, &[obj, 0, 0xBD8]);
        wd(obj as usize + 0xBC8, 0);
        wd(obj as usize + 0xBCC, 0);
        s.eax = obj;
        s.call(0x0069_0D00, &[]);
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.esi = s.esi;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_690cd0, e_690cd0, "ret");

/// One x87 arithmetic step on f32 operands, stored as f32. When both operands are NaN the x87 returns
/// the one with the larger significand (SSE would return the first); otherwise the operation is done in
/// f64, which matches the 53-bit precision control.
fn x87_op(x: f32, y: f32, op: impl Fn(f64, f64) -> f64) -> f32 {
    if x.is_nan() && y.is_nan() {
        let q = |v: f32| f32::from_bits(v.to_bits() | 0x40_0000);
        let (qx, qy) = (q(x), q(y));
        return if qx.to_bits() & 0x7F_FFFF >= qy.to_bits() & 0x7F_FFFF { qx } else { qy };
    }
    op(x as f64, y as f64) as f32
}

// --- cutscene_action_exit (x87) ------------------------------------------------------------------
// d = f32(a - b), where a = [0x27E6714] and b = [0x27E6710]; [0x27E6710] = f32(b + d);
// [0x27E66FC] = f32(d + c). The original pushes ECX and stores d over that slot, so ECX leaves as
// the bits of d.

extern "C" fn e_6ccf20(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(3);
        let a = rf(0x027E_6714);
        let b = rf(0x027E_6710);
        let d = x87_op(a, b, |p, q| p - q);
        wf(0x027E_6710, x87_op(b, d, |p, q| p + q));
        let c = rf(0x027E_66FC);
        wf(0x027E_66FC, x87_op(d, c, |p, q| p + q));
        r.ecx = d.to_bits();
    }
}

thunk!(t_6ccf20, e_6ccf20, "ret");

// --- smesh_cache_record_add_chunk ----------------------------------------------------------------
// key = low word of the stack argument; search the six words at EDI + 0x116 for the key (hit: AL = 1);
// else store the key into the first free (0xFFFF) word (AL = 1); else AL = 0.
// ECX / EDX are the loop state the original leaves behind.

unsafe fn rw(p: usize) -> u16 {
    unsafe { *(p as *const u16) }
}

unsafe fn ww(p: usize, v: u16) {
    unsafe { *(p as *mut u16) = v }
}

extern "C" fn e_6e6b30(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(4);
        let key = rd(a as usize) as u16;
        let edi = r.edi as usize;
        let base = edi + 0x116;
        let mut eax = r.eax & !0xFF;
        let mut hit = None;
        for k in 0..6usize {
            if rw(base + 2 * k) == key {
                hit = Some(k);
                break;
            }
        }
        let (ecx, edx) = match hit {
            Some(k) => {
                eax |= 1;
                (k, base + 2 * k)
            }
            None => {
                let mut free = None;
                for k in 0..6usize {
                    if rw(base + 2 * k) == 0xFFFF {
                        free = Some(k);
                        break;
                    }
                }
                match free {
                    Some(k) => {
                        ww(edi + 0x116 + 2 * k, key);
                        eax |= 1;
                        (k, base + 2 * k)
                    }
                    None => (6, base + 12),
                }
            }
        };
        r.eax = eax;
        r.ecx = ecx as u32;
        r.edx = edx as u32;
    }
}

thunk!(t_6e6b30, e_6e6b30, "ret");

// --- audio_category_from_name --------------------------------------------------------------------
// For ESI = 0..4: stricmp(table[ESI], EDI) (table at 0xDD5428); a zero result returns ESI; no match
// returns -1. ESI is the callee-preserved index the original keeps across each call.

extern "C" fn e_7ecca0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(5);
        let name = r.edi;
        let mut s = St::of(r);
        s.esi = 0;
        loop {
            let p = rd(0x00DD_5428 + 4 * s.esi as usize);
            s.eax = p;
            let e = s.call(0x00C9_9030, &[p, name]);
            if e == 0 {
                r.eax = s.esi;
                break;
            }
            s.esi = s.esi.wrapping_add(1);
            if s.esi >= 4 {
                r.eax = 0xFFFF_FFFF;
                break;
            }
        }
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_7ecca0, e_7ecca0, "ret");

// --- act_packet_sub_40 --------------------------------------------------------------------------
// The record at the stack argument: ECX = word at +1; if byte 0 is 0, call 0x465CD0(ECX, 0), else
// call 0x465D70(ECX). ESI is a frame pointer (E - 12, E = entry ESP) that the callees see.

extern "C" fn e_5c2610(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(6);
        let rec = rd(a as usize) as usize;
        let ecx = rw(rec + 1) as u32;
        let mut s = St::of(r);
        s.eax = rec as u32;
        s.ecx = ecx;
        s.esi = r.esp.wrapping_sub(12);
        if rb(rec) == 0 {
            s.call(0x0046_5CD0, &[ecx, 0]);
        } else {
            s.call(0x0046_5D70, &[ecx]);
        }
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_5c2610, e_5c2610, "ret");

// --- act_packet_sub_55 ---------------------------------------------------------------------------
// g = [0x2527888]; if g: call the vtable slot 2 of g.+8 (ECX = that object); if it returns 0x19 and
// [0x2527888] is set and its +0x14 is not 6, set its +0xE2 byte to 1.

const PKT_G: usize = 0x0252_7888;

extern "C" fn e_6455f0(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(7);
        let mut s = St::of(r);
        let g = rd(PKT_G);
        let mut out = g;
        if g != 0 {
            let ecx = rd(g as usize + 8);
            let vt = rd(ecx as usize);
            let f = rd(vt as usize + 8);
            s.eax = vt;
            s.ecx = ecx;
            s.edx = f;
            s.calli(f, &[]);
            if s.eax == 0x19 {
                let g2 = rd(PKT_G);
                out = g2;
                if g2 != 0 && rd(g2 as usize + 0x14) != 6 {
                    wb(g2 as usize + 0xE2, 1);
                }
            } else {
                out = s.eax;
            }
        }
        r.eax = out;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_6455f0, e_6455f0, "ret");

// --- chunk_extras_list_release -------------------------------------------------------------------
// DI = 16-bit key (0xFFFF returns at once), DL = flag. Scan the 0xD0-byte records from 0x22295F6 up to
// 0x2229C76 for one whose word at -2 is DI and whose byte is DL. On a hit (ESI = the record): with DL = 0
// clear +0xCF, else clear +0xCE and call 0xA82500(EDI, 0); then, when +0xCE and +0xCF are both zero,
// set +0xCC = 0xFFFF, +0xC8 = 0 and call memset(ESI, -1, 0xC8).

extern "C" fn e_a82600(r: &mut Pushad, _a: *const u32) {
    unsafe {
        count(8);
        let di = (r.edi & 0xFFFF) as u16;
        let dl = (r.edx & 0xFF) as u8;
        if di == 0xFFFF {
            return;
        }
        let mut eax: u32 = 0x0222_95F6;
        let mut idx: u32 = 0;
        let hit = loop {
            if rw(eax as usize - 2) == di && rb(eax as usize) == dl {
                break true;
            }
            eax = eax.wrapping_add(0xD0);
            idx = idx.wrapping_add(1);
            if eax >= 0x0222_9C76 {
                break false;
            }
        };
        if !hit {
            r.eax = eax;
            r.ecx = idx;
            return;
        }
        let slot = idx.wrapping_mul(0xD0).wrapping_add(0x0222_9528);
        let mut s = St::of(r);
        s.eax = eax;
        s.ecx = slot;
        s.esi = slot;
        if dl == 0 {
            wb(slot as usize + 0xCF, 0);
        } else {
            wb(slot as usize + 0xCE, 0);
            s.call(0x00A8_2500, &[r.edi, 0]);
        }
        let esi = s.esi as usize;
        if rb(esi + 0xCE) == 0 && rb(esi + 0xCF) == 0 {
            ww(esi + 0xCC, 0xFFFF);
            wd(esi + 0xC8, 0);
            s.call(0x00CA_0BC0, &[esi as u32, 0xFFFF_FFFF, 0xC8]);
        }
        r.eax = s.eax;
        r.ecx = s.ecx;
        r.edx = s.edx;
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_a82600, e_a82600, "ret");

// --- projectile::get_local_bbox -----------------------------------------------------------------
// The object's vtable slot 1 (ECX = this) returns a pointer obj; if it is non-null, the two out
// structures (stack arguments 1 and 2) get the 3-dword blocks at P = [[obj+0x44]+0x30]: +0 .. +8 into
// arg 1, and +0xC .. +0x14 into arg 2 (ECX = arg 2, EDX = P+0x10, EAX = P+0x14 on return). Otherwise both
// structures are zeroed (EAX = arg 2).

extern "C" fn e_94bb60(r: &mut Pushad, a: *const u32) {
    unsafe {
        count(9);
        let out1 = rd(a as usize) as usize;
        let out2 = rd(a as usize + 4) as usize;
        let this = r.ecx;
        let mut s = St::of(r);
        let vt = rd(this as usize);
        let f = rd(vt as usize + 4);
        s.eax = vt;
        s.edx = f;
        s.calli(f, &[]);
        let obj = s.eax as usize;
        if obj != 0 {
            let p = rd(rd(obj + 0x44) as usize + 0x30) as usize;
            wd(out1, rd(p));
            wd(out1 + 4, rd(p + 4));
            wd(out1 + 8, rd(p + 8));
            wd(out2, rd(p + 0xC));
            wd(out2 + 4, rd(p + 0x10));
            wd(out2 + 8, rd(p + 0x14));
            r.eax = rd(p + 0x14);
            r.ecx = out2 as u32;
            r.edx = rd(p + 0x10);
        } else {
            for q in [out1, out2] {
                wd(q, 0);
                wd(q + 4, 0);
                wd(q + 8, 0);
            }
            r.eax = out2 as u32;
            r.ecx = s.ecx;
            r.edx = s.edx;
        }
        r.ebx = s.ebx;
        r.edi = s.edi;
        r.ebp = s.ebp;
    }
}

thunk!(t_94bb60, e_94bb60, "ret 8");

// --- the originals' bytes (true ends) ------------------------------------------------------------

const B_60AAF0: [u8; 20] = [
    0x56, 0x57, 0xBF, 0xA0, 0x42, 0xDF, 0x00, 0xBE, 0x44, 0x43, 0xDF, 0x00, 0xE8, 0xCF, 0x9F, 0x1E,
    0x00, 0x5F, 0x5E, 0xC3,
];
const B_924FE0: [u8; 41] = [
    0x8B, 0x0D, 0xEC, 0x95, 0x52, 0x02, 0x56, 0x85, 0xC9, 0x74, 0x1C, 0x8B, 0x41, 0x0C, 0x8B, 0xF0,
    0x2B, 0x35, 0xEC, 0x95, 0x52, 0x02, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF0, 0x8B, 0x01, 0x8B, 0x50,
    0x14, 0xFF, 0xD2, 0x8B, 0xCE, 0xEB, 0xE0, 0x5E, 0xC3,
];
const B_690CD0: [u8; 43] = [
    0x68, 0xD8, 0x0B, 0x00, 0x00, 0x6A, 0x00, 0x56, 0xE8, 0xE3, 0xFE, 0x60, 0x00, 0x83, 0xC4, 0x0C,
    0xC7, 0x86, 0xC8, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x86, 0xCC, 0x0B, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0xE9, 0x05, 0x00, 0x00, 0x00,
];
const B_6CCF20: [u8; 51] = [
    0x51, 0xD9, 0x05, 0x14, 0x67, 0x7E, 0x02, 0xD9, 0x05, 0x10, 0x67, 0x7E, 0x02, 0xD9, 0xC0, 0xDE,
    0xEA, 0xD9, 0xC9, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0xC0, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9,
    0x1D, 0x10, 0x67, 0x7E, 0x02, 0xD8, 0x05, 0xFC, 0x66, 0x7E, 0x02, 0xD9, 0x1D, 0xFC, 0x66, 0x7E,
    0x02, 0x59, 0xC3,
];
const B_6E6B30: [u8; 73] = [
    0x53, 0x66, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x8D, 0xB7, 0x16, 0x01, 0x00, 0x00, 0x32, 0xC0, 0x33,
    0xC9, 0x8B, 0xD6, 0x66, 0x39, 0x1A, 0x74, 0x2C, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x02, 0x83, 0xF9,
    0x06, 0x7C, 0xF0, 0x33, 0xC9, 0x8B, 0xD6, 0x66, 0x81, 0x3A, 0xFF, 0xFF, 0x74, 0x0E, 0x83, 0xC1,
    0x01, 0x83, 0xC2, 0x02, 0x83, 0xF9, 0x06, 0x7C, 0xEE, 0x5E, 0x5B, 0xC3, 0x66, 0x89, 0x9C, 0x4F,
    0x16, 0x01, 0x00, 0x00, 0x5E, 0xB0, 0x01, 0x5B, 0xC3,
];
const B_5C2610: [u8; 51] = [
    0x8B, 0x44, 0x24, 0x04, 0x0F, 0xB7, 0x48, 0x01, 0x83, 0xEC, 0x0C, 0x80, 0x38, 0x00, 0x56, 0x8D,
    0x74, 0x24, 0x04, 0x74, 0x0E, 0x51, 0xE8, 0x45, 0x37, 0xEA, 0xFF, 0x83, 0xC4, 0x04, 0x5E, 0x83,
    0xC4, 0x0C, 0xC3, 0x6A, 0x00, 0x51, 0xE8, 0x95, 0x36, 0xEA, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0x83,
    0xC4, 0x0C, 0xC3,
];
const B_6455F0: [u8; 47] = [
    0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x25, 0x8B, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x50,
    0x08, 0xFF, 0xD2, 0x83, 0xF8, 0x19, 0x75, 0x16, 0xA1, 0x88, 0x78, 0x52, 0x02, 0x85, 0xC0, 0x74,
    0x0D, 0x83, 0x78, 0x14, 0x06, 0x74, 0x07, 0xC6, 0x80, 0xE2, 0x00, 0x00, 0x00, 0x01, 0xC3,
];
const B_A82600: [u8; 143] = [
    0x66, 0x81, 0xFF, 0xFF, 0xFF, 0x74, 0x7E, 0x33, 0xC9, 0xB8, 0xF6, 0x95, 0x22, 0x02, 0x8B, 0xFF,
    0x66, 0x39, 0x78, 0xFE, 0x75, 0x04, 0x38, 0x10, 0x74, 0x10, 0x05, 0xD0, 0x00, 0x00, 0x00, 0x83,
    0xC1, 0x01, 0x3D, 0x76, 0x9C, 0x22, 0x02, 0x7C, 0xE7, 0xC3, 0x69, 0xC9, 0xD0, 0x00, 0x00, 0x00,
    0x81, 0xC1, 0x28, 0x95, 0x22, 0x02, 0x84, 0xD2, 0x56, 0x8B, 0xF1, 0x74, 0x49, 0x6A, 0x00, 0x57,
    0xC6, 0x86, 0xCE, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xB4, 0xFE, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x80,
    0xBE, 0xCE, 0x00, 0x00, 0x00, 0x00, 0x75, 0x2C, 0x80, 0xBE, 0xCF, 0x00, 0x00, 0x00, 0x00, 0x75,
    0x23, 0x68, 0xC8, 0x00, 0x00, 0x00, 0x6A, 0xFF, 0x56, 0x66, 0xC7, 0x86, 0xCC, 0x00, 0x00, 0x00,
    0xFF, 0xFF, 0xC7, 0x86, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x3F, 0xE5, 0x21,
    0x00, 0x83, 0xC4, 0x0C, 0x5E, 0xC3, 0xC6, 0x86, 0xCF, 0x00, 0x00, 0x00, 0x00, 0xEB, 0xC0,
];
const B_94BB60: [u8; 101] = [
    0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x3D, 0x8B, 0x48, 0x44, 0x8B, 0x49,
    0x30, 0x8B, 0x54, 0x24, 0x04, 0x56, 0x8B, 0x31, 0x89, 0x32, 0x8B, 0x71, 0x04, 0x89, 0x72, 0x04,
    0x8B, 0x49, 0x08, 0x89, 0x4A, 0x08, 0x8B, 0x50, 0x44, 0x8B, 0x42, 0x30, 0x8B, 0x50, 0x0C, 0x8B,
    0x4C, 0x24, 0x0C, 0x83, 0xC0, 0x0C, 0x89, 0x11, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x40,
    0x08, 0x89, 0x41, 0x08, 0x5E, 0xC2, 0x08, 0x00, 0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x04, 0xD9, 0x10,
    0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0x8B, 0x44, 0x24, 0x08, 0xD9, 0x10, 0xD9, 0x50, 0x04, 0xD9,
    0x58, 0x08, 0xC2, 0x08, 0x00,
];
const B_7ECCA0: [u8; 41] = [
    0x56, 0x33, 0xF6, 0x8B, 0x04, 0xB5, 0x28, 0x54, 0xDD, 0x00, 0x57, 0x50, 0xE8, 0x7F, 0xC3, 0x4A,
    0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x0D, 0x83, 0xC6, 0x01, 0x83, 0xFE, 0x04, 0x7C, 0xE3,
    0x83, 0xC8, 0xFF, 0x5E, 0xC3, 0x8B, 0xC6, 0x5E, 0xC3,
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let ports: [(&str, usize, &[u8], usize); 10] = [
        ("exploration_diversion_type::get_title", 0x0060_AAF0, &B_60AAF0[..], t_60aaf0 as *const () as usize),
        ("objects_deactivate_all", 0x0092_4FE0, &B_924FE0[..], t_924fe0 as *const () as usize),
        ("profile_clear_and_init", 0x0069_0CD0, &B_690CD0[..], t_690cd0 as *const () as usize),
        ("cutscene_action_exit", 0x006C_CF20, &B_6CCF20[..], t_6ccf20 as *const () as usize),
        ("smesh_cache_record_add_chunk", 0x006E_6B30, &B_6E6B30[..], t_6e6b30 as *const () as usize),
        ("audio_category_from_name", 0x007E_CCA0, &B_7ECCA0[..], t_7ecca0 as *const () as usize),
        ("act_packet_sub_40", 0x005C_2610, &B_5C2610[..], t_5c2610 as *const () as usize),
        ("act_packet_sub_55", 0x0064_55F0, &B_6455F0[..], t_6455f0 as *const () as usize),
        ("chunk_extras_list_release", 0x00A8_2600, &B_A82600[..], t_a82600 as *const () as usize),
        ("projectile::get_local_bbox", 0x0094_BB60, &B_94BB60[..], t_94bb60 as *const () as usize),
    ];
    let mut ok = 0;
    for (name, addr, body, thunk) in ports {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_h5: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let names = [
        "get_title",
        "objects_deactivate_all",
        "profile_clear_and_init",
        "cutscene_action_exit",
        "smesh_cache_record_add_chunk",
        "audio_category_from_name",
        "act_packet_sub_40",
        "act_packet_sub_55",
        "chunk_extras_list_release",
        "projectile::get_local_bbox",
    ];
    let parts: Vec<String> = names
        .iter()
        .zip(CNT.iter())
        .filter_map(|(n, c)| {
            let v = c.load(Ordering::Relaxed);
            (v > 0).then(|| format!("{n} {v}"))
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_h5: {}", parts.join(", "))
}
