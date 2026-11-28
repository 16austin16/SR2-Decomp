//! Port module: the large cutscene functions of doc 51, hand-ported and checked offline by a
//! differential test against the original bytes (see `dt`): every callee is a recording stub; the
//! original and the Rust version run on the same random world and must make the same calls in the
//! same order with the same registers, stack arguments and pointed-to data, write the same memory
//! and preserve the same registers.
//!
//! Check type at run time: per-branch counters only (the callees have side effects, so no shadow).
//!
//! Run the offline tests:
//!   RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//!   -p sr2-hook --target i686-pc-windows-msvc port_cutscene2 -- --ignored --test-threads=1 --nocapture


#![allow(clippy::missing_safety_doc)]

/// Thunks like portlib's keep_ecx_edx*, but also keeping XMM0-7 (the original leaves them alone).
#[allow(unused_macros)]
macro_rules! kx0 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
            "push ecx",
            "push edx",
            "sub esp, 128",
            "movups [esp + 0], xmm0",
            "movups [esp + 16], xmm1",
            "movups [esp + 32], xmm2",
            "movups [esp + 48], xmm3",
            "movups [esp + 64], xmm4",
            "movups [esp + 80], xmm5",
            "movups [esp + 96], xmm6",
            "movups [esp + 112], xmm7",
            "call {f}",
            "movups xmm0, [esp + 0]",
            "movups xmm1, [esp + 16]",
            "movups xmm2, [esp + 32]",
            "movups xmm3, [esp + 48]",
            "movups xmm4, [esp + 64]",
            "movups xmm5, [esp + 80]",
            "movups xmm6, [esp + 96]",
            "movups xmm7, [esp + 112]",
            "add esp, 128",
            "pop edx",
            "pop ecx",
            "ret",
            f = sym $f,
            )
        }
    };
}
#[allow(unused_macros)]
macro_rules! kx1 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
            "push ecx",
            "push edx",
            "sub esp, 128",
            "movups [esp + 0], xmm0",
            "movups [esp + 16], xmm1",
            "movups [esp + 32], xmm2",
            "movups [esp + 48], xmm3",
            "movups [esp + 64], xmm4",
            "movups [esp + 80], xmm5",
            "movups [esp + 96], xmm6",
            "movups [esp + 112], xmm7",
            "push dword ptr [esp + 140]",
            "call {f}",
            "add esp, 4",
            "movups xmm0, [esp + 0]",
            "movups xmm1, [esp + 16]",
            "movups xmm2, [esp + 32]",
            "movups xmm3, [esp + 48]",
            "movups xmm4, [esp + 64]",
            "movups xmm5, [esp + 80]",
            "movups xmm6, [esp + 96]",
            "movups xmm7, [esp + 112]",
            "add esp, 128",
            "pop edx",
            "pop ecx",
            "ret",
            f = sym $f,
            )
        }
    };
}
#[allow(unused_macros)]
macro_rules! kx2 {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
            "push ecx",
            "push edx",
            "sub esp, 128",
            "movups [esp + 0], xmm0",
            "movups [esp + 16], xmm1",
            "movups [esp + 32], xmm2",
            "movups [esp + 48], xmm3",
            "movups [esp + 64], xmm4",
            "movups [esp + 80], xmm5",
            "movups [esp + 96], xmm6",
            "movups [esp + 112], xmm7",
            "push dword ptr [esp + 144]",
            "push dword ptr [esp + 144]",
            "call {f}",
            "add esp, 8",
            "movups xmm0, [esp + 0]",
            "movups xmm1, [esp + 16]",
            "movups xmm2, [esp + 32]",
            "movups xmm3, [esp + 48]",
            "movups xmm4, [esp + 64]",
            "movups xmm5, [esp + 80]",
            "movups xmm6, [esp + 96]",
            "movups xmm7, [esp + 112]",
            "add esp, 128",
            "pop edx",
            "pop ecx",
            "ret",
            f = sym $f,
            )
        }
    };
}


use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::{replace_pinned};
use crate::log;

// --- memory access: in the offline test the game's globals sit DELTA higher -----------------------
/// The offline test keeps the game's globals (0xD00000..0x3000000) at a fixed offset.
#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn t(p: usize) -> usize {
    if (0xD0_0000..0x300_0000).contains(&p) { p + DELTA } else { p }
}
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn t(p: usize) -> usize {
    p
}
/// The address of a game global as a value handed to a callee.
fn gp(p: usize) -> u32 {
    t(p) as u32
}
unsafe fn rd(p: usize) -> u32 {
    unsafe { *(t(p) as *const u32) }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(t(p) as *const u8) }
}
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(t(p) as *const f32) }
}
unsafe fn rf64(p: usize) -> f64 {
    unsafe { std::ptr::read_unaligned(t(p) as *const f64) }
}
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(t(p) as *mut u32) = v }
}
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(t(p) as *mut u8) = v }
}
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(t(p) as *mut f32) = v }
}
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(dt::redirect(f), eax, ecx, edx, esi, edi, args) }
}
#[cfg(not(test))]
use crate::portlib::call_regs;

/// `call_regs` for a callee that returns on the x87 stack: the result is popped as a qword.
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    #[cfg(test)]
    let f = dt::redirect(f);
    let mut out = 0f64;
    let block: [u32; 9] =
        [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "mov ebx, eax",
            "mov ecx, [ebx + 24]",
            "mov esi, [ebx + 20]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]", "mov esi, [ebx + 12]", "mov edi, [ebx + 16]",
            "mov eax, [ebx + 28]",
            "call dword ptr [ebx]",
            "mov ecx, [ebx + 32]",
            "fstp qword ptr [ecx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    out
}

/// `fld dword; fstp dword`: quiets a signalling NaN.
fn q(x: f32) -> f32 {
    std::hint::black_box(x as f64) as f32
}

/// The x87 result of `r = a op b` when both operands are NaNs: the larger significand wins
/// (SSE would return the first operand).
fn x87(a: f64, b: f64, r: f64) -> f64 {
    if a.is_nan() && b.is_nan() {
        let m = |v: f64| (v.to_bits() | 1 << 51) & 0xF_FFFF_FFFF_FFFF;
        let pick = if m(b) > m(a) { b } else { a };
        f64::from_bits(pick.to_bits() | 1 << 51)
    } else {
        r
    }
}

/// Per-branch counters.
static HITS: [AtomicU32; 32] = [const { AtomicU32::new(0) }; 32];
fn hit(i: usize) {
    HITS[i].fetch_add(1, Ordering::Relaxed);
}

const PLAYER: usize = 0x0252_7D10; // the cutscene state object
const LOCAL_PLAYER: usize = 0x0217_03D4;

/// `script_find_object_by_name` (0xA52BB0, EAX = &found, ECX = name), then when it did not find
/// it `script_object_to_human` (0xA52C70, EAX = the first result, ESI = name). Returns
/// (object as the callers test it, whether the second call ran).
unsafe fn find_obj(name: u32) -> (u32, bool) {
    unsafe {
        let mut found = 0u8;
        let o = call_regs(0xA5_2BB0, &mut found as *mut u8 as u32, name, 0, 0, 0, &[]);
        if std::ptr::read_volatile(&found) != 0 {
            (o, false)
        } else {
            (call_regs(0xA5_2C70, o, 0, 0, name, 0, &[]), true)
        }
    }
}



/// A volatile read of a buffer a callee fills through a pointer.
fn rv(a: &[u32], i: usize) -> u32 {
    unsafe { std::ptr::read_volatile(&a[i]) }
}

/// The inner loop shared by the two slot-filling passes of `shot_start` (0x6C86F0 and 0x6C87E0):
/// `rec` is a 12-byte record {flags ptr, ?, count}; for every index with a zero flag byte, appends
/// `val` to the pointer list in the 0x90-byte slot of the same index (bounded by the slot's
/// capacity, as the original checks).
unsafe fn fill_slots(p: usize, rec: u32, val: u32) {
    unsafe {
        let rec = rec as usize;
        if rd(rec + 8) as i32 <= 0 {
            return;
        }
        let mut j = 0i32;
        let mut off = 0u32;
        loop {
            let slots = rd(p + 0xAF4) as usize;
            if j >= rd(slots + 0x10) as i32 {
                break;
            }
            let flags = rd(rec);
            if rb(flags.wrapping_add(j as u32) as usize) == 0 {
                let base = rd(slots + 0xC);
                let e = base.wrapping_add(off).wrapping_add(0x80) as usize;
                let n = rd(e + 8);
                let n1 = n.wrapping_add(1);
                if n1 as i32 <= rd(e + 4) as i32 {
                    wd(e + 8, n1);
                    let list = rd(e);
                    wd(list.wrapping_add(n << 2) as usize, val);
                }
            }
            j += 1;
            off = off.wrapping_add(0x90);
            if j >= rd(rec + 8) as i32 {
                break;
            }
        }
    }
}

// === cutscene_shot_start 0x6C8130 (0xA83 bytes, cdecl, one stack argument: the shot index) =========
//
// Starts shot `idx` of the running cutscene (state object [0x2527D10], shots of 0x170 bytes at
// +0x1534, count +0x153C, current +0x1540): readies the shot's characters, resets the vehicle
// objects, ends the previous shot, positions the camera, places characters and vehicles, fills the
// per-shot slot lists and finishes the 'shot started' bookkeeping. Returns nothing (EAX / ECX / EDX
// are left as scratch). The docs say "no args"; it takes the index on the stack.
//
// Deviation: the original reads memory at 0x7D68 when an object's +0x84D8 flag is not 1 (a crash);
// the port skips that object.
pub(crate) const SS_ADDR: usize = 0x6C8130;

unsafe fn shot_start(idx: i32) {
    unsafe {
        hit(0);
        let mut p = rd(PLAYER) as usize; // ebx
        if p == 0 || idx < 0 || idx >= rd(p + 0x153C) as i32 {
            hit(1);
            return;
        }
        // the previous shot's index: (current - base) / 0x170 as the original's signed multiply
        let cur = rd(p + 0x1540);
        let prev: i32 = if cur != 0 {
            let x = cur.wrapping_sub(rd(p + 0x1534)) as i32;
            let m = (x as i64) * (0xB216_42C9u32 as i32 as i64);
            let edx = ((m >> 32) as i32).wrapping_add(x) >> 8;
            edx.wrapping_add(((edx as u32) >> 31) as i32)
        } else {
            -1
        };
        let shot_first = (idx as u32).wrapping_mul(0x170).wrapping_add(rd(p + 0x1534)) as usize;
        // pass 1: ready the human of every character entry that is used
        if rd(shot_first + 0x108) as i32 > 0 {
            let mut i = 0i32;
            let mut off = 0u32;
            loop {
                let e = rd(shot_first + 0x100).wrapping_add(off) as usize;
                if rb(p + 0xAFD) != 0 || rd(e + 0x60) != 0 || rd(e + 0x6C) != 0 {
                    let name = rd(e);
                    if name != 0 {
                        let (obj, second) = find_obj(name);
                        if second {
                            p = rd(PLAYER) as usize;
                        }
                        if obj != 0 {
                            hit(2);
                            let obj = obj as usize;
                            if rb(e + 0x36) == 1 {
                                call_regs(0x55_A930, 0, 0, 0, 0, obj as u32, &[(obj + 0x10C4) as u32, 0, 1, 0]);
                                p = rd(PLAYER) as usize;
                            }
                            let esi = rd(obj + 0x5E0);
                            call_regs(0x6C_32A0, 0, 0, 0, esi, 0, &[]);
                            let edx = rd(obj + 0x5E0) as usize;
                            wd(obj + 0x758, rd(obj + 0x758) | 1);
                            wd(edx + 0x14, 1);
                            let h = rd(obj + 0x5E0);
                            let a = rd(h as usize + 0x14);
                            let r = call_regs(0x6E_F620, 0, 0, 0, 0, 0, &[h, a, 0]);
                            wd(h as usize + 0x18, r);
                        }
                    }
                }
                i += 1;
                off = off.wrapping_add(0x7C);
                if i >= rd(shot_first + 0x108) as i32 {
                    break;
                }
            }
        }
        // pass 2: reset the voice / sound state of every cutscene object
        if rd(p + 0xAE8) as i32 > 0 {
            let mut i = 0u32;
            loop {
                let arr = rd(p + 0xAE0) as usize;
                let o = rd(arr + (i as usize) * 4);
                let vt = rd(o as usize);
                let f = rd(vt as usize + 0x34);
                let eax = call_regs(f as usize, vt, o, f, 0, 0, &[]);
                if eax != 0 {
                    hit(3);
                    let ecx = if rb(eax as usize + 0x84D8) == 1 { eax.wrapping_add(0x538) } else { 0 };
                    let esi = ecx.wrapping_add(0x7D68);
                    if ecx != 0 && esi != 0 && rd(esi as usize) != 0xFFFF_FFFF {
                        call_regs(0x6F_0660, 0, 0, 0, 0, 0, &[esi, 0, 0xFFFF_FFFF]);
                        let c = rd(esi as usize);
                        call_regs(0xCE_DC10, 0, c, 0, 0, 0, &[]);
                        let c = rd(esi as usize);
                        call_regs(0xCE_DC50, 0, c, 0, 0, 0, &[0]);
                        wf(esi as usize + 0x24, 0.0);
                        wd(esi as usize + 0x14, 0xFFFF_FFFF);
                        wd(esi as usize + 0x18, 0xFFFF);
                    }
                }
                p = rd(PLAYER) as usize;
                i += 1;
                if i as i32 >= rd(p + 0xAE8) as i32 {
                    break;
                }
            }
        }
        if prev >= 0 {
            hit(4);
            let base = rd(p + 0x1534) as usize;
            let v = rd(base.wrapping_add((prev as u32).wrapping_mul(0x170) as usize).wrapping_add(0x15C)).wrapping_add(0x64);
            call_regs(0x6C_1770, prev as u32, 0, 0, 0, 0, &[v]);
            p = rd(PLAYER) as usize;
        }
        // the new current shot
        let shot_off = (idx as u32).wrapping_mul(0x170);
        let shot = rd(p + 0x1534).wrapping_add(shot_off) as usize; // ebp, and ebx from 0x6C8572 on
        wd(p + 0x1540, shot as u32);
        if rd(shot + 0xFC) != 0 {
            call_regs(0x52_2B80, 0, 0, 0, 0, 0, &[0]);
            let v = rd(shot + 0xFC);
            call_regs(0x6B_F6C0, 0, 0, 0, 0, 0, &[v, 1]);
        }
        let next_idx = idx.wrapping_add(1);
        // "shot%u" into a 0x40-byte buffer (the original's frame has 0x40 bytes there)
        let mut name_buf = [0u8; 0x40];
        call_regs(0xBF_2C10, 0, 0, 0, 0x40, name_buf.as_mut_ptr() as u32, &[gp(0xE0_C71C), next_idx as u32]);
        let _ = std::hint::black_box(&name_buf);
        let sn = rd(shot + 4);
        call_regs(0x6C_7D30, 0, sn, 0, 0, 0, &[sn, 1]);
        // the 12 bytes the original reuses as the output buffer of 0x6C8F50, 0x6C34C0 and 0x6C78B0
        let mut ub = [0u32; 3];
        p = rd(PLAYER) as usize; // eax
        if rb(p + 0xB02) == 0 {
            hit(5);
            if rb(p + 0xAFD) == 0 {
                call_regs(0x49_5240, 0, 0, 0, 0, 0, &[]);
            } else {
                call_regs(0x6C_8F50, ub.as_mut_ptr() as u32, 0, 0, 0, 0, &[]);
                let lp = rd(LOCAL_PLAYER);
                call_regs(0x4A_2840, lp, 0, 0, 0, 0, &[ub.as_ptr() as u32, 0, 0, 0, 0]);
            }
            let p2 = rd(PLAYER) as usize; // ecx
            let o = rd(p2 + 0x15F8);
            if o != 0 {
                wb(o as usize + 0x460, 0);
            }
            if rd(p2 + 0x1600) != 0 {
                let a = rd(0x140_E670);
                call_regs(rd(0xDB_83D0) as usize, 0, 0, 0, 0, 0, &[a, 0]);
            }
        }
        call_regs(0x6C_24E0, 0, 0, 0, 0, 0, &[shot as u32]);
        call_regs(0x6C_2EF0, 0, 0, 0, 0, 0, &[shot as u32]);
        call_regs(0x6C_1770, idx as u32, 0, 0, 0, 0, &[0]);
        let mut edi_p = rd(PLAYER) as usize; // edi
        if rb(edi_p + 0xB02) == 0 {
            if (rb(0x252_7D14) != 0 && rb(0x252_7CF7) != 0) || rb(0x252_7D15) != 0 {
                if rb(0x252_7D25) == 0 {
                    call_regs(0x47_B660, 0, 0, 0, gp(0xEA_B83C), 0, &[]);
                    wb(0x252_7D25, 1);
                }
            }
            wd(0x252_6A74, 0);
        }
        if rb(edi_p + 0xAFD) == 1 {
            hit(6);
            if idx > 0 {
                let prevshot = rd(edi_p + 0x1534).wrapping_add(shot_off).wrapping_sub(0x170);
                call_regs(0x6C_1EF0, 0, 0, 0, 0, prevshot, &[0]);
            }
            call_regs(0x6C_1D30, 0, 0, 0, 0, 0, &[shot as u32]);
            call_regs(0x6C_D6D0, 0, shot as u32, 0, 0, 0, &[]);
            call_regs(0x6C_1B50, 0, 0, 0, 0, 0, &[shot as u32]);
            edi_p = rd(PLAYER) as usize;
            if next_idx < rd(edi_p + 0x153C) as i32 {
                let esi = rd(edi_p + 0x1534).wrapping_add(shot_off).wrapping_add(0x170);
                if rd(esi as usize + 0x12C) as i32 > 0 {
                    let mut n = 0i32;
                    let mut off = 0u32;
                    loop {
                        let e = rd(esi as usize + 0x124).wrapping_add(off);
                        call_regs(0x6C_1CB0, 0, 0, shot as u32, esi, e, &[]);
                        n += 1;
                        off = off.wrapping_add(0x2C);
                        if n >= rd(esi as usize + 0x12C) as i32 {
                            break;
                        }
                    }
                }
                call_regs(0x6C_D6D0, 0, esi, 0, 0, 0, &[]);
                edi_p = rd(PLAYER) as usize;
            }
        }
        // common part from 0x6C856C
        wf(0xE9_97C8, q(rf(0xDD_0550)));
        wb(0x25F_5D90, 0);
        wf(0xE9_97CC, q(rf(0xDD_0794)));
        wd(edi_p + 0x155C, 0xFFFF_FFFF);
        wf(0xE9_97D0, q(rf(0xDD_1944)));
        wf(0xE9_97D4, q(rf(0xDD_1C50)));
        wf(0xE9_898C, q(rf(edi_p + 0x1660)));
        wd(edi_p + 0x15E4, 0);
        wd(edi_p + 0x15E8, 0);
        wd(edi_p + 0x15EC, 0);
        call_regs(0x6D_8900, 0, 0, 0, 0, shot as u32, &[]);
        let p = rd(PLAYER) as usize; // ebp
        let af0 = rd(p + 0xAF0);
        if af0 != 0 && rd(p + 0xAC0) != 0 {
            hit(7);
            call_regs(0x6C_1660, rd(p + 0xAC4), (shot + 0x14C) as u32, 0, 0, 0, &[af0]);
            let mut eax = rd(p + 0xAF4) as usize;
            if rd(eax + 0x10) as i32 > 0 {
                let mut k = 0i32;
                let mut off = 0u32;
                loop {
                    let a = rd(eax + 0xC).wrapping_add(off) as usize;
                    if (rd(a + 0x84) as i32) >= 0 {
                        wd(a + 0x88, 0);
                    }
                    eax = rd(p + 0xAF4) as usize;
                    k += 1;
                    off = off.wrapping_add(0x90);
                    if k >= rd(eax + 0x10) as i32 {
                        break;
                    }
                }
            }
            // slot lists of the characters
            if rd(shot + 0x108) as i32 > 0 {
                let mut k = 0u32;
                loop {
                    let off = k.wrapping_mul(0x7C);
                    let ent = rd(shot + 0x100).wrapping_add(off) as usize;
                    if rb(ent + 0x34) != 1 {
                        let obj: u32;
                        if rb(p + 0x1608) == 1 {
                            if k == 0 {
                                obj = rd(LOCAL_PLAYER);
                            } else {
                                let c = rd((rd(p + 0xAC8) as usize).wrapping_add((k as usize) * 4).wrapping_sub(4));
                                let vt = rd(c as usize);
                                let f = rd(vt as usize + 0x34);
                                obj = call_regs(f as usize, f, c, vt, 0, 0, &[]);
                            }
                        } else {
                            let c = rd((rd(p + 0xAC8) as usize).wrapping_add((k as usize) * 4));
                            let vt = rd(c as usize);
                            let f = rd(vt as usize + 0x34);
                            obj = call_regs(f as usize, f, c, vt, 0, 0, &[]);
                        }
                        let ent = rd(shot + 0x100).wrapping_add(off) as usize;
                        if rd(ent + 0x54) != 0 {
                            let rec = rd(ent + 0x4C).wrapping_add(rd(p + 0xAC4).wrapping_mul(12));
                            fill_slots(p, rec, obj);
                        }
                    }
                    k += 1;
                    if k as i32 >= rd(shot + 0x108) as i32 {
                        break;
                    }
                }
            }
            // slot lists of the second array
            if rd(shot + 0x114) as i32 > 0 {
                let mut k = 0u32;
                let mut off = 0u32;
                loop {
                    let ent = rd(shot + 0x10C).wrapping_add(off) as usize;
                    if rd(ent + 0x54) as i32 > rd(p + 0xAC4) as i32 && rb(ent + 0x34) != 1 {
                        let arr = rd(p + 0xAE0) as usize;
                        let c = rd(arr.wrapping_add((k as usize) * 4));
                        if c != 0 {
                            let vt = rd(c as usize);
                            let f = rd(vt as usize + 0x34);
                            let obj = call_regs(f as usize, f, c, vt, 0, 0, &[]);
                            let ent = rd(shot + 0x10C).wrapping_add(off) as usize;
                            if rd(ent + 0x54) != 0 {
                                let rec = rd(ent + 0x4C).wrapping_add(rd(p + 0xAC4).wrapping_mul(12));
                                fill_slots(p, rec, obj);
                            }
                        }
                    }
                    k += 1;
                    off = off.wrapping_add(0x7C);
                    if k as i32 >= rd(shot + 0x114) as i32 {
                        break;
                    }
                }
            }
        }
        if rb(p + 0xAFD) == 1 {
            hit(8);
            // the shot's camera point (passed by value) converted by 0x6C34C0
            let mut tvec = [0u32; 3];
            call_regs(0x6C_92A0, 0, 0, 0, (shot + 0x1C) as u32, tvec.as_mut_ptr() as u32, &[0]);
            let r = call_regs(0x6C_34C0, shot as u32, ub.as_mut_ptr() as u32, 0, 0, 0, &[rv(&tvec, 0), rv(&tvec, 1), rv(&tvec, 2)]) as usize;
            let (x, y, z) = (rd(r), rd(r + 4), rd(r + 8));
            // V: the reference point (the original's stack slots 0x28..0x30)
            let mut vv = [x, y, z];
            let nm = rd(shot + 0xB0);
            if nm != 0 {
                hit(9);
                let mut esi: u32;
                let mut found = 0u8;
                let o = call_regs(0xA5_2BB0, &mut found as *mut u8 as u32, nm, 0, 0, 0, &[]);
                if std::ptr::read_volatile(&found) != 0 {
                    esi = o;
                } else {
                    esi = 0;
                    let n = call_regs(0xA2_75F0, nm, 0, 0, 0, 0, &[]) as i32;
                    if n > 0 {
                        esi = call_regs(0xA2_7660, 0, 0, 0, 0, 0, &[n as u32]);
                    } else {
                        let mut h = 0u32;
                        let hp = call_regs(0xBD_CA30, &mut h as *mut u32 as u32, 0, 0, 0, 0, &[nm]);
                        let hash = rd(hp as usize);
                        let head = rd(0x2F2_5254);
                        let mut c = head;
                        if head != 0 {
                            loop {
                                if rd(c as usize + 0xC) == hash {
                                    let vt = rd(c as usize);
                                    let f = rd(vt as usize + 0x34);
                                    esi = call_regs(f as usize, vt, c, f, 0, 0, &[]);
                                    break;
                                }
                                c = rd(c as usize + 0x17C);
                                if c == head || c == 0 {
                                    break;
                                }
                            }
                        }
                    }
                }
                let bv = rd(shot + 0xB4);
                call_regs(0x4A_29E0, vv.as_mut_ptr() as u32, bv, 0, esi, 0, &[]);
                if rb(0x25F_5D90) == 1 {
                    // distance on the ground plane from the object to the point
                    let fv = |b: u32| f32::from_bits(b) as f64;
                    let ax = rf(esi as usize + 0x14) as f64;
                    let az = rf(esi as usize + 0x1C) as f64;
                    let (vx, vz) = (fv(rv(&vv, 0)), fv(rv(&vv, 2)));
                    let t0 = x87(ax, vx, ax - vx) as f32;
                    let t1 = x87(az, vz, az - vz) as f32;
                    let (a, b) = (t0 as f64 * t0 as f64, t1 as f64 * t1 as f64);
                    let s0 = x87(a, 0.0, a + 0.0);
                    let sum = x87(s0, b, s0 + b) as f32;
                    let d = call_regs_st0(0x40_EA20, 0, 0, 0, 0, 0, &[sum.to_bits()]) as f32;
                    wf(0x25F_5D94, d);
                }
            } else {
                hit(10);
                let mut w = [0u32; 3];
                call_regs(0x6B_EED0, w.as_mut_ptr() as u32, shot as u32, 0, 0, 0, &[]);
                call_regs(0x6C_78B0, 0, w.as_ptr() as u32, 0, 0, 0, &[ub.as_mut_ptr() as u32, 0]);
                let fv = |b: u32| f32::from_bits(b) as f64;
                let (ux, uy, uz) = (fv(rv(&ub, 0)), fv(rv(&ub, 1)), fv(rv(&ub, 2)));
                let (vx, vy, vz) = (fv(vv[0]), fv(vv[1]), fv(vv[2]));
                let wx = x87(ux, vx, ux - vx) as f32;
                let wy = x87(uy, vy, uy - vy) as f32;
                let wz = x87(uz, vz, uz - vz) as f32;
                // (the original stores through the x87: W.x + 0.0, W.y + a constant, 0.0 + W.z)
                let k = rf64(0xE5_BFE8);
                ub[0] = (((wx as f64) + 0.0) as f32).to_bits();
                ub[1] = (((wy as f64) + k) as f32).to_bits();
                ub[2] = ((0.0 + (wz as f64)) as f32).to_bits();
                call_regs(0xBD_E150, 0, 0, 0, ub.as_mut_ptr() as u32, 0, &[]);
                let mut xm = [0u32; 9];
                call_regs(0xBE_4B20, ub.as_ptr() as u32, xm.as_mut_ptr() as u32, 0, 0, 0, &[]);
                call_regs(0x4A_29E0, vv.as_mut_ptr() as u32, 0, 0, 0, xm.as_mut_ptr() as u32, &[]);
                wb(0x25F_5D90, 0);
                let _ = std::hint::black_box(&xm);
            }
            if rb(shot + 0x9C) == 1 {
                wf(0xE9_97C8, q(rf(shot + 0xA0)));
                wf(0xE9_97CC, q(rf(shot + 0xA4)));
                wf(0xE9_97D0, q(rf(shot + 0xA8)));
                wf(0xE9_97D4, q(rf(shot + 0xAC)));
            }
        }
        call_regs(0x49_32F0, 0, 0, 0, 0, 0, &[]);
        call_regs(0x4B_3FF0, gp(0x25F_5B20), 0, 0, 0, 0, &[0]);
        call_regs(0x6B_EFC0, 0, 0, 0, 0, 0, &[shot as u32, 0, 0]);
        call_regs(0x4C_8770, 0, 0, 0, 0, 0, &[]);
        call_regs(0x4C_F840, 0, 0, 0, 0, 0, &[]);
        let ecx = rd(PLAYER) as usize;
        let edi = rd(ecx + 0x1534).wrapping_add(shot_off);
        call_regs(0x6C_D9A0, 0, 0, 0, 0, edi, &[0]);
        let p = rd(PLAYER) as usize;
        if rb(p + 0xB02) == 0 {
            hit(11);
            if rb(p + 0xB03) == 0 {
                call_regs(0x4D_1270, 0, 0, 0, 0, 0, &[0x1F4, 0]);
            }
            wf(0x1F7_6904, 1.0);
            wb(0x140_C85A, 1);
            wd(0xEA_C468, 0xFFFF_FFFF);
            let eax = rd(0x252_853C);
            if eax != 0 && (eax & 0xFFFF) < 0xC {
                let edi = (eax & 0xFFFF).wrapping_mul(0x57C).wrapping_add(0x319_F6B0);
                if rd(edi as usize + 0x4C) == eax {
                    let hv = call_regs(0xBD_C9B0, 0, gp(0xE0_C638), 0, 0, 0, &[]);
                    if hv != 0 {
                        let head = rd(0x22E_F1B8 + ((hv & 0x3F) as usize) * 4);
                        let mut esi = head;
                        if esi != 0 {
                            loop {
                                if rd(esi as usize + 0x30) == edi && rd(esi as usize + 0x2C) == hv {
                                    break;
                                }
                                esi = rd(esi as usize + 4);
                                if esi == head {
                                    esi = 0;
                                    break;
                                }
                                if esi == 0 {
                                    break;
                                }
                            }
                        }
                        if esi != 0 {
                            let w = rd(0xEB_DF6C);
                            let a = rd(w as usize);
                            let vt = rd(esi as usize);
                            let f = rd(vt as usize + 0x18);
                            let r = call_regs(f as usize, w, esi, f, 0, 0, &[a]);
                            if r & 0xFF == 1 {
                                hit(12);
                                // a 0x10-byte variant {tag 5, byte 1}: the original builds only these
                                let mut ev = [0u32; 4];
                                ev[0] = 5;
                                ev[1] = 1;
                                let f = rd(vt as usize + 0x3C);
                                call_regs(f as usize, vt, esi, f, 0, 0, &[ev.as_ptr() as u32, 0]);
                                call_regs(0xB9_3790, ev.as_ptr() as u32, 0, 0, 0, 0, &[]);
                            }
                        }
                    }
                }
            }
            let p = rd(PLAYER) as usize;
            wb(p + 0xB02, 1);
            wb(p + 0xB0C, 1);
        } else {
            wb(p + 0xB0C, 1);
        }
    }
}

const SS_BODY: [u8; 2691] = [
    0x81, 0xEC, 0xB0, 0x00, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0xAC, 0x00, 0x00, 0x00, 0x53, 0x8B, 0x1D, 0x10, 0x7D, 0x52, 0x02, 0x85, 0xDB, 0x55, 0x56, 0x8B,
    0xB4, 0x24, 0xC0, 0x00, 0x00, 0x00, 0x57, 0x0F, 0x84, 0x3D, 0x0A, 0x00, 0x00, 0x85, 0xF6, 0x0F,
    0x8C, 0x35, 0x0A, 0x00, 0x00, 0x3B, 0xB3, 0x3C, 0x15, 0x00, 0x00, 0x0F, 0x8D, 0x29, 0x0A, 0x00,
    0x00, 0x8B, 0x83, 0x40, 0x15, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x21, 0x2B, 0x83, 0x34, 0x15, 0x00,
    0x00, 0x8B, 0xC8, 0xB8, 0xC9, 0x42, 0x16, 0xB2, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x08, 0x8B,
    0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x89, 0x44, 0x24, 0x18, 0xEB, 0x08, 0xC7, 0x44, 0x24, 0x18,
    0xFF, 0xFF, 0xFF, 0xFF, 0x85, 0xF6, 0x0F, 0x8C, 0xC2, 0x01, 0x00, 0x00, 0x8B, 0xC6, 0x69, 0xC0,
    0x70, 0x01, 0x00, 0x00, 0x03, 0x83, 0x34, 0x15, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x24, 0x00, 0x00,
    0x00, 0x00, 0x83, 0xB8, 0x08, 0x01, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x1C, 0x0F, 0x8E, 0xE0,
    0x00, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x44, 0x24, 0x1C, 0x8B, 0xA8, 0x00, 0x01, 0x00, 0x00, 0x03, 0x6C, 0x24, 0x20, 0x80, 0xBB,
    0xFD, 0x0A, 0x00, 0x00, 0x00, 0x75, 0x10, 0x83, 0x7D, 0x60, 0x00, 0x75, 0x0A, 0x83, 0x7D, 0x6C,
    0x00, 0x0F, 0x84, 0x8C, 0x00, 0x00, 0x00, 0x8B, 0x75, 0x00, 0x85, 0xF6, 0x0F, 0x84, 0x81, 0x00,
    0x00, 0x00, 0x8D, 0x44, 0x24, 0x17, 0x8B, 0xCE, 0xC6, 0x44, 0x24, 0x17, 0x00, 0xE8, 0x8E, 0xA9,
    0x38, 0x00, 0x80, 0x7C, 0x24, 0x17, 0x00, 0x75, 0x0F, 0xE8, 0x42, 0xAA, 0x38, 0x00, 0x85, 0xC0,
    0x8B, 0x1D, 0x10, 0x7D, 0x52, 0x02, 0x74, 0x5B, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x55, 0x80, 0x7D,
    0x36, 0x01, 0x75, 0x18, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x00, 0x8D, 0x8F, 0xC4, 0x10, 0x00, 0x00,
    0x51, 0xE8, 0xDA, 0x26, 0xE9, 0xFF, 0x8B, 0x1D, 0x10, 0x7D, 0x52, 0x02, 0x8B, 0xB7, 0xE0, 0x05,
    0x00, 0x00, 0xE8, 0x39, 0xB0, 0xFF, 0xFF, 0x8B, 0x97, 0xE0, 0x05, 0x00, 0x00, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0x09, 0x87, 0x58, 0x07, 0x00, 0x00, 0x89, 0x42, 0x14, 0x8B, 0xBF, 0xE0, 0x05, 0x00,
    0x00, 0x8B, 0x47, 0x14, 0x6A, 0x00, 0x50, 0x57, 0xE8, 0x93, 0x73, 0x02, 0x00, 0x83, 0xC4, 0x0C,
    0x89, 0x47, 0x18, 0x8B, 0x44, 0x24, 0x24, 0x8B, 0x4C, 0x24, 0x1C, 0x83, 0x44, 0x24, 0x20, 0x7C,
    0x83, 0xC0, 0x01, 0x3B, 0x81, 0x08, 0x01, 0x00, 0x00, 0x89, 0x44, 0x24, 0x24, 0x0F, 0x8C, 0x2D,
    0xFF, 0xFF, 0xFF, 0x33, 0xFF, 0x39, 0xBB, 0xE8, 0x0A, 0x00, 0x00, 0x0F, 0x8E, 0x7E, 0x00, 0x00,
    0x00, 0x83, 0xCD, 0xFF, 0x8B, 0x93, 0xE0, 0x0A, 0x00, 0x00, 0x8B, 0x0C, 0xBA, 0x8B, 0x01, 0x8B,
    0x50, 0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x56, 0x8A, 0x88, 0xD8, 0x84, 0x00, 0x00, 0x05, 0x38,
    0x05, 0x00, 0x00, 0x80, 0xE9, 0x01, 0xF6, 0xD9, 0x1B, 0xC9, 0xF7, 0xD1, 0x23, 0xC8, 0x8B, 0xF1,
    0x81, 0xC6, 0x68, 0x7D, 0x00, 0x00, 0x74, 0x36, 0x39, 0x2E, 0x74, 0x32, 0xD9, 0xEE, 0x55, 0x51,
    0xD9, 0x1C, 0x24, 0x56, 0xE8, 0x57, 0x83, 0x02, 0x00, 0x8B, 0x0E, 0x83, 0xC4, 0x0C, 0xE8, 0xFD,
    0x58, 0x62, 0x00, 0x8B, 0x0E, 0x6A, 0x00, 0xE8, 0x34, 0x59, 0x62, 0x00, 0xD9, 0xEE, 0xD9, 0x5E,
    0x24, 0x83, 0xC4, 0x04, 0x89, 0x6E, 0x14, 0xC7, 0x46, 0x18, 0xFF, 0xFF, 0x00, 0x00, 0x8B, 0x1D,
    0x10, 0x7D, 0x52, 0x02, 0x83, 0xC7, 0x01, 0x3B, 0xBB, 0xE8, 0x0A, 0x00, 0x00, 0x7C, 0x85, 0x8B,
    0x44, 0x24, 0x18, 0x85, 0xC0, 0x7C, 0x27, 0x8B, 0x8B, 0x34, 0x15, 0x00, 0x00, 0x8B, 0xD0, 0x69,
    0xD2, 0x70, 0x01, 0x00, 0x00, 0x8B, 0x94, 0x11, 0x5C, 0x01, 0x00, 0x00, 0x83, 0xC2, 0x64, 0x52,
    0xE8, 0x0B, 0x94, 0xFF, 0xFF, 0x8B, 0x1D, 0x10, 0x7D, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x8B, 0x84,
    0x24, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0xAB, 0x34, 0x15, 0x00, 0x00, 0x69, 0xC0, 0x70, 0x01, 0x00,
    0x00, 0x03, 0xE8, 0x89, 0xAB, 0x40, 0x15, 0x00, 0x00, 0x83, 0xBD, 0xFC, 0x00, 0x00, 0x00, 0x00,
    0x89, 0x44, 0x24, 0x24, 0x89, 0x6C, 0x24, 0x20, 0x74, 0x1B, 0x6A, 0x00, 0xE8, 0xDF, 0xA7, 0xE5,
    0xFF, 0x8B, 0x85, 0xFC, 0x00, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x6A, 0x01, 0x50, 0xE8, 0x0E, 0x73,
    0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x8B, 0x9C, 0x24, 0xC4, 0x00, 0x00, 0x00, 0x8D, 0x43, 0x01, 0x50,
    0x68, 0x1C, 0xC7, 0xE0, 0x00, 0xBE, 0x40, 0x00, 0x00, 0x00, 0x8D, 0xBC, 0x24, 0x80, 0x00, 0x00,
    0x00, 0x89, 0x44, 0x24, 0x20, 0xE8, 0x36, 0xA8, 0x52, 0x00, 0x83, 0xC4, 0x08, 0x6A, 0x01, 0x51,
    0x8B, 0x4D, 0x04, 0x8B, 0xC4, 0x89, 0x08, 0xE8, 0x44, 0xF9, 0xFF, 0xFF, 0xA1, 0x10, 0x7D, 0x52,
    0x02, 0x83, 0xC4, 0x08, 0x80, 0xB8, 0x02, 0x0B, 0x00, 0x00, 0x00, 0x75, 0x5E, 0x80, 0xB8, 0xFD,
    0x0A, 0x00, 0x00, 0x00, 0x75, 0x07, 0xE8, 0x35, 0xCE, 0xDC, 0xFF, 0xEB, 0x1F, 0x6A, 0x00, 0x6A,
    0x00, 0x6A, 0x00, 0x6A, 0x00, 0x8D, 0x44, 0x24, 0x48, 0xE8, 0x32, 0x0B, 0x00, 0x00, 0x50, 0xA1,
    0xD4, 0x03, 0x17, 0x02, 0xE8, 0x17, 0xA4, 0xDD, 0xFF, 0x83, 0xC4, 0x14, 0x8B, 0x0D, 0x10, 0x7D,
    0x52, 0x02, 0x8B, 0x81, 0xF8, 0x15, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x07, 0xC6, 0x80, 0x60, 0x04,
    0x00, 0x00, 0x00, 0x83, 0xB9, 0x00, 0x16, 0x00, 0x00, 0x00, 0x74, 0x0F, 0x8B, 0x15, 0x70, 0xE6,
    0x40, 0x01, 0x6A, 0x00, 0x52, 0xFF, 0x15, 0xD0, 0x83, 0xDB, 0x00, 0x55, 0xE8, 0x7F, 0xA0, 0xFF,
    0xFF, 0x83, 0xC4, 0x04, 0x55, 0xE8, 0x86, 0xAA, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x6A, 0x00, 0x8B,
    0xC3, 0xE8, 0xFA, 0x92, 0xFF, 0xFF, 0x8B, 0x3D, 0x10, 0x7D, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x80,
    0xBF, 0x02, 0x0B, 0x00, 0x00, 0x00, 0x75, 0x3F, 0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74,
    0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00, 0x75, 0x09, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02,
    0x00, 0x74, 0x1A, 0x80, 0x3D, 0x25, 0x7D, 0x52, 0x02, 0x00, 0x75, 0x11, 0xBE, 0x3C, 0xB8, 0xEA,
    0x00, 0xE8, 0xAA, 0x31, 0xDB, 0xFF, 0xC6, 0x05, 0x25, 0x7D, 0x52, 0x02, 0x01, 0xC7, 0x05, 0x74,
    0x6A, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0x80, 0xBF, 0xFD, 0x0A, 0x00, 0x00, 0x01, 0x0F, 0x85,
    0x98, 0x00, 0x00, 0x00, 0x85, 0xDB, 0x7E, 0x1B, 0x8B, 0x87, 0x34, 0x15, 0x00, 0x00, 0x8B, 0x4C,
    0x24, 0x24, 0x6A, 0x00, 0x8D, 0xBC, 0x08, 0x90, 0xFE, 0xFF, 0xFF, 0xE8, 0x00, 0x9A, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0x55, 0xE8, 0x37, 0x98, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xCD, 0xE8, 0xCD,
    0x51, 0x00, 0x00, 0x55, 0xE8, 0x47, 0x96, 0xFF, 0xFF, 0x8B, 0x3D, 0x10, 0x7D, 0x52, 0x02, 0x8B,
    0x54, 0x24, 0x1C, 0x83, 0xC4, 0x04, 0x3B, 0x97, 0x3C, 0x15, 0x00, 0x00, 0x7D, 0x4E, 0x8B, 0x87,
    0x34, 0x15, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x24, 0x8D, 0xB4, 0x08, 0x70, 0x01, 0x00, 0x00, 0x33,
    0xDB, 0x39, 0x9E, 0x2C, 0x01, 0x00, 0x00, 0x7E, 0x26, 0x33, 0xED, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0xBE, 0x24, 0x01, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x20, 0x03, 0xFD, 0xE8, 0x5F, 0x97, 0xFF,
    0xFF, 0x83, 0xC3, 0x01, 0x83, 0xC5, 0x2C, 0x3B, 0x9E, 0x2C, 0x01, 0x00, 0x00, 0x7C, 0xE1, 0x8B,
    0xCE, 0xE8, 0x6A, 0x51, 0x00, 0x00, 0x8B, 0x3D, 0x10, 0x7D, 0x52, 0x02, 0xD9, 0x05, 0x50, 0x05,
    0xDD, 0x00, 0x8B, 0x5C, 0x24, 0x20, 0xD9, 0x1D, 0xC8, 0x97, 0xE9, 0x00, 0xC6, 0x05, 0x90, 0x5D,
    0x5F, 0x02, 0x00, 0xD9, 0x05, 0x94, 0x07, 0xDD, 0x00, 0xC7, 0x87, 0x5C, 0x15, 0x00, 0x00, 0xFF,
    0xFF, 0xFF, 0xFF, 0xD9, 0x1D, 0xCC, 0x97, 0xE9, 0x00, 0xD9, 0x05, 0x44, 0x19, 0xDD, 0x00, 0xD9,
    0x1D, 0xD0, 0x97, 0xE9, 0x00, 0xD9, 0x05, 0x50, 0x1C, 0xDD, 0x00, 0xD9, 0x1D, 0xD4, 0x97, 0xE9,
    0x00, 0xD9, 0x87, 0x60, 0x16, 0x00, 0x00, 0xD9, 0x1D, 0x8C, 0x89, 0xE9, 0x00, 0xD9, 0xEE, 0xD9,
    0x54, 0x24, 0x28, 0x8B, 0x54, 0x24, 0x28, 0xD9, 0x54, 0x24, 0x2C, 0x8B, 0x44, 0x24, 0x2C, 0xD9,
    0x5C, 0x24, 0x30, 0x8B, 0x4C, 0x24, 0x30, 0x89, 0x97, 0xE4, 0x15, 0x00, 0x00, 0x89, 0x87, 0xE8,
    0x15, 0x00, 0x00, 0x89, 0x8F, 0xEC, 0x15, 0x00, 0x00, 0x8B, 0xFB, 0xE8, 0x10, 0x03, 0x01, 0x00,
    0x8B, 0x2D, 0x10, 0x7D, 0x52, 0x02, 0x8B, 0x85, 0xF0, 0x0A, 0x00, 0x00, 0x33, 0xF6, 0x3B, 0xC6,
    0x0F, 0x84, 0x45, 0x02, 0x00, 0x00, 0x39, 0xB5, 0xC0, 0x0A, 0x00, 0x00, 0x0F, 0x84, 0x39, 0x02,
    0x00, 0x00, 0x50, 0x8B, 0x85, 0xC4, 0x0A, 0x00, 0x00, 0x8D, 0x8B, 0x4C, 0x01, 0x00, 0x00, 0xE8,
    0x3C, 0x90, 0xFF, 0xFF, 0x8B, 0x85, 0xF4, 0x0A, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x33, 0xC9, 0x39,
    0x70, 0x10, 0x7E, 0x2C, 0x33, 0xD2, 0x8B, 0x40, 0x0C, 0x39, 0xB4, 0x10, 0x84, 0x00, 0x00, 0x00,
    0x8D, 0x84, 0x10, 0x80, 0x00, 0x00, 0x00, 0x7C, 0x03, 0x89, 0x70, 0x08, 0x8B, 0x85, 0xF4, 0x0A,
    0x00, 0x00, 0x83, 0xC1, 0x01, 0x81, 0xC2, 0x90, 0x00, 0x00, 0x00, 0x3B, 0x48, 0x10, 0x7C, 0xD6,
    0x33, 0xC0, 0x39, 0xB3, 0x08, 0x01, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0x0F, 0x8E, 0xDE, 0x00,
    0x00, 0x00, 0x8B, 0x8B, 0x00, 0x01, 0x00, 0x00, 0x8B, 0xF0, 0x6B, 0xF6, 0x7C, 0x80, 0x7C, 0x31,
    0x34, 0x01, 0x0F, 0x84, 0xAD, 0x00, 0x00, 0x00, 0x80, 0xBD, 0x08, 0x16, 0x00, 0x00, 0x01, 0x75,
    0x1C, 0x85, 0xC0, 0x75, 0x0C, 0x8B, 0x15, 0xD4, 0x03, 0x17, 0x02, 0x89, 0x54, 0x24, 0x1C, 0xEB,
    0x26, 0x8B, 0x8D, 0xC8, 0x0A, 0x00, 0x00, 0x8B, 0x4C, 0x81, 0xFC, 0xEB, 0x09, 0x8B, 0x8D, 0xC8,
    0x0A, 0x00, 0x00, 0x8B, 0x0C, 0x81, 0x8B, 0x11, 0x8B, 0x42, 0x34, 0xFF, 0xD0, 0x8B, 0x2D, 0x10,
    0x7D, 0x52, 0x02, 0x89, 0x44, 0x24, 0x1C, 0x8B, 0x8B, 0x00, 0x01, 0x00, 0x00, 0x83, 0x7C, 0x31,
    0x54, 0x00, 0x8D, 0x14, 0x31, 0x74, 0x5E, 0x8B, 0x8D, 0xC4, 0x0A, 0x00, 0x00, 0x8D, 0x04, 0x49,
    0x8B, 0x4A, 0x4C, 0x8D, 0x1C, 0x81, 0x33, 0xD2, 0x39, 0x53, 0x08, 0x7E, 0x48, 0x33, 0xC0, 0x90,
    0x8B, 0x8D, 0xF4, 0x0A, 0x00, 0x00, 0x3B, 0x51, 0x10, 0x7D, 0x3A, 0x8B, 0x33, 0x80, 0x3C, 0x32,
    0x00, 0x75, 0x25, 0x8B, 0x49, 0x0C, 0x8B, 0xB4, 0x01, 0x88, 0x00, 0x00, 0x00, 0x8D, 0x8C, 0x01,
    0x80, 0x00, 0x00, 0x00, 0x8D, 0x7E, 0x01, 0x3B, 0x79, 0x04, 0x7F, 0x0C, 0x89, 0x79, 0x08, 0x8B,
    0x09, 0x8B, 0x7C, 0x24, 0x1C, 0x89, 0x3C, 0xB1, 0x83, 0xC2, 0x01, 0x05, 0x90, 0x00, 0x00, 0x00,
    0x3B, 0x53, 0x08, 0x7C, 0xBB, 0x8B, 0x44, 0x24, 0x18, 0x8B, 0x5C, 0x24, 0x20, 0x83, 0xC0, 0x01,
    0x3B, 0x83, 0x08, 0x01, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0x0F, 0x8C, 0x22, 0xFF, 0xFF, 0xFF,
    0x33, 0xC9, 0x39, 0x8B, 0x14, 0x01, 0x00, 0x00, 0x89, 0x4C, 0x24, 0x18, 0x0F, 0x8E, 0xE9, 0x00,
    0x00, 0x00, 0x33, 0xF6, 0x89, 0x74, 0x24, 0x1C, 0x8B, 0x93, 0x0C, 0x01, 0x00, 0x00, 0x8D, 0x04,
    0x16, 0x8B, 0x50, 0x54, 0x3B, 0x95, 0xC4, 0x0A, 0x00, 0x00, 0x0F, 0x8E, 0xA5, 0x00, 0x00, 0x00,
    0x80, 0x78, 0x34, 0x01, 0x0F, 0x84, 0x9B, 0x00, 0x00, 0x00, 0x8B, 0x85, 0xE0, 0x0A, 0x00, 0x00,
    0x83, 0x3C, 0x88, 0x00, 0x8D, 0x04, 0x88, 0x0F, 0x84, 0x88, 0x00, 0x00, 0x00, 0x8B, 0x08, 0x8B,
    0x11, 0x8B, 0x42, 0x34, 0xFF, 0xD0, 0x8B, 0x8B, 0x0C, 0x01, 0x00, 0x00, 0x8B, 0x2D, 0x10, 0x7D,
    0x52, 0x02, 0x03, 0xCE, 0x83, 0x79, 0x54, 0x00, 0x89, 0x44, 0x24, 0x34, 0x74, 0x67, 0x8B, 0x95,
    0xC4, 0x0A, 0x00, 0x00, 0x8B, 0x41, 0x4C, 0x8D, 0x14, 0x52, 0x8D, 0x1C, 0x90, 0x33, 0xD2, 0x39,
    0x53, 0x08, 0x7E, 0x51, 0x33, 0xC0, 0xEB, 0x08, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x90,
    0x8B, 0x8D, 0xF4, 0x0A, 0x00, 0x00, 0x3B, 0x51, 0x10, 0x7D, 0x3A, 0x8B, 0x33, 0x80, 0x3C, 0x32,
    0x00, 0x75, 0x25, 0x8B, 0x49, 0x0C, 0x8B, 0xB4, 0x01, 0x88, 0x00, 0x00, 0x00, 0x8D, 0x8C, 0x01,
    0x80, 0x00, 0x00, 0x00, 0x8D, 0x7E, 0x01, 0x3B, 0x79, 0x04, 0x7F, 0x0C, 0x89, 0x79, 0x08, 0x8B,
    0x09, 0x8B, 0x7C, 0x24, 0x34, 0x89, 0x3C, 0xB1, 0x83, 0xC2, 0x01, 0x05, 0x90, 0x00, 0x00, 0x00,
    0x3B, 0x53, 0x08, 0x7C, 0xBB, 0x8B, 0x4C, 0x24, 0x18, 0x8B, 0x74, 0x24, 0x1C, 0x8B, 0x5C, 0x24,
    0x20, 0x83, 0xC1, 0x01, 0x83, 0xC6, 0x7C, 0x3B, 0x8B, 0x14, 0x01, 0x00, 0x00, 0x89, 0x4C, 0x24,
    0x18, 0x89, 0x74, 0x24, 0x1C, 0x0F, 0x8C, 0x1D, 0xFF, 0xFF, 0xFF, 0x80, 0xBD, 0xFD, 0x0A, 0x00,
    0x00, 0x01, 0x0F, 0x85, 0xF4, 0x01, 0x00, 0x00, 0xD9, 0xEE, 0x83, 0xEC, 0x0C, 0x8B, 0xFC, 0x51,
    0x8D, 0x73, 0x1C, 0xD9, 0x1C, 0x24, 0xE8, 0x35, 0x0A, 0x00, 0x00, 0x8B, 0xC3, 0x8D, 0x4C, 0x24,
    0x44, 0xE8, 0x4A, 0xAC, 0xFF, 0xFF, 0x8B, 0x10, 0x8B, 0xB3, 0xB0, 0x00, 0x00, 0x00, 0x8B, 0x48,
    0x04, 0x89, 0x54, 0x24, 0x34, 0x8B, 0x50, 0x08, 0x83, 0xC4, 0x0C, 0x85, 0xF6, 0x89, 0x4C, 0x24,
    0x2C, 0x89, 0x54, 0x24, 0x30, 0x0F, 0x84, 0xE6, 0x00, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x17, 0x8B,
    0xCE, 0xC6, 0x44, 0x24, 0x17, 0x00, 0xE8, 0x05, 0xA3, 0x38, 0x00, 0x80, 0x7C, 0x24, 0x17, 0x00,
    0x74, 0x04, 0x8B, 0xF0, 0xEB, 0x4F, 0x8B, 0xC6, 0xE8, 0x33, 0xED, 0x35, 0x00, 0x85, 0xC0, 0x7E,
    0x0E, 0x50, 0xE8, 0x99, 0xED, 0x35, 0x00, 0x83, 0xC4, 0x04, 0xE9, 0xAA, 0x00, 0x00, 0x00, 0x56,
    0x33, 0xD2, 0x8D, 0x44, 0x24, 0x38, 0xE8, 0x55, 0x41, 0x51, 0x00, 0x8B, 0x15, 0x54, 0x52, 0xF2,
    0x02, 0x85, 0xD2, 0x8B, 0xCA, 0x74, 0x1C, 0x8B, 0x00, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x41, 0x0C, 0x74, 0x79, 0x8B, 0x89, 0x7C, 0x01, 0x00, 0x00, 0x3B, 0xCA, 0x74, 0x04, 0x85,
    0xC9, 0x75, 0xED, 0x33, 0xF6, 0x8B, 0x8B, 0xB4, 0x00, 0x00, 0x00, 0x33, 0xFF, 0x8D, 0x44, 0x24,
    0x28, 0xE8, 0xCA, 0xA0, 0xDD, 0xFF, 0x80, 0x3D, 0x90, 0x5D, 0x5F, 0x02, 0x01, 0x0F, 0x85, 0xF0,
    0x00, 0x00, 0x00, 0xD9, 0x46, 0x14, 0x51, 0xD8, 0x64, 0x24, 0x2C, 0xD9, 0x5C, 0x24, 0x3C, 0xD9,
    0x46, 0x1C, 0xD8, 0x64, 0x24, 0x34, 0xD9, 0x5C, 0x24, 0x44, 0xD9, 0x44, 0x24, 0x3C, 0xD9, 0x44,
    0x24, 0x44, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xEE, 0xDC, 0xC8, 0xDE, 0xC2, 0xDC, 0xC8, 0xDE, 0xC1,
    0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x44, 0x24, 0x38, 0xD9, 0x1C, 0x24, 0xE8, 0xC0, 0x60, 0xD4, 0xFF,
    0xD9, 0x1D, 0x94, 0x5D, 0x5F, 0x02, 0x83, 0xC4, 0x04, 0xE9, 0xA5, 0x00, 0x00, 0x00, 0x85, 0xC9,
    0x74, 0x91, 0x8B, 0x01, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x85, 0xC0, 0x74, 0x86, 0x8B, 0xF0, 0xEB,
    0x84, 0x8B, 0xCB, 0x8D, 0x44, 0x24, 0x48, 0xE8, 0x44, 0x65, 0xFF, 0xFF, 0x8D, 0x44, 0x24, 0x38,
    0x6A, 0x00, 0x50, 0x8D, 0x4C, 0x24, 0x50, 0xE8, 0x14, 0xEF, 0xFF, 0xFF, 0xD9, 0x44, 0x24, 0x40,
    0xD8, 0x64, 0x24, 0x30, 0x83, 0xC4, 0x08, 0x8D, 0x74, 0x24, 0x38, 0xD9, 0x5C, 0x24, 0x48, 0xD9,
    0x44, 0x24, 0x3C, 0xD8, 0x64, 0x24, 0x2C, 0xD9, 0x5C, 0x24, 0x4C, 0xD9, 0x44, 0x24, 0x40, 0xD8,
    0x64, 0x24, 0x30, 0xD9, 0x5C, 0x24, 0x50, 0xD9, 0x44, 0x24, 0x48, 0xD9, 0xEE, 0xDC, 0xC1, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x44, 0x24, 0x4C, 0xDC, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xD9,
    0x5C, 0x24, 0x3C, 0xD8, 0x44, 0x24, 0x50, 0xD9, 0x5C, 0x24, 0x40, 0xE8, 0x60, 0x57, 0x51, 0x00,
    0x8B, 0xC6, 0x8D, 0x4C, 0x24, 0x54, 0xE8, 0x25, 0xC1, 0x51, 0x00, 0x8D, 0x7C, 0x24, 0x54, 0x8D,
    0x44, 0x24, 0x28, 0x33, 0xC9, 0x33, 0xF6, 0xE8, 0xD4, 0x9F, 0xDD, 0xFF, 0xC6, 0x05, 0x90, 0x5D,
    0x5F, 0x02, 0x00, 0x80, 0xBB, 0x9C, 0x00, 0x00, 0x00, 0x01, 0x75, 0x30, 0xD9, 0x83, 0xA0, 0x00,
    0x00, 0x00, 0xD9, 0x1D, 0xC8, 0x97, 0xE9, 0x00, 0xD9, 0x83, 0xA4, 0x00, 0x00, 0x00, 0xD9, 0x1D,
    0xCC, 0x97, 0xE9, 0x00, 0xD9, 0x83, 0xA8, 0x00, 0x00, 0x00, 0xD9, 0x1D, 0xD0, 0x97, 0xE9, 0x00,
    0xD9, 0x83, 0xAC, 0x00, 0x00, 0x00, 0xD9, 0x1D, 0xD4, 0x97, 0xE9, 0x00, 0xE8, 0x9F, 0xA8, 0xDC,
    0xFF, 0x6A, 0x00, 0xB8, 0x20, 0x5B, 0x5F, 0x02, 0xE8, 0x93, 0xB5, 0xDE, 0xFF, 0xD9, 0xEE, 0x83,
    0xC4, 0x04, 0x6A, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x53, 0xE8, 0x52, 0x65, 0xFF, 0xFF, 0x83, 0xC4,
    0x0C, 0xE8, 0xFA, 0xFC, 0xDF, 0xFF, 0xE8, 0xC5, 0x6D, 0xE0, 0xFF, 0x8B, 0x0D, 0x10, 0x7D, 0x52,
    0x02, 0x8B, 0xB9, 0x34, 0x15, 0x00, 0x00, 0x03, 0x7C, 0x24, 0x24, 0x6A, 0x00, 0xE8, 0x0E, 0x4F,
    0x00, 0x00, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x80, 0xB8, 0x02, 0x0B, 0x00, 0x00,
    0x00, 0x0F, 0x85, 0xEC, 0x00, 0x00, 0x00, 0x80, 0xB8, 0x03, 0x0B, 0x00, 0x00, 0x00, 0x75, 0x15,
    0xD9, 0xEE, 0x51, 0xD9, 0x1C, 0x24, 0x32, 0xD2, 0x68, 0xF4, 0x01, 0x00, 0x00, 0xE8, 0xAE, 0x87,
    0xE0, 0xFF, 0x83, 0xC4, 0x08, 0xD9, 0xE8, 0xA1, 0x3C, 0x85, 0x52, 0x02, 0x85, 0xC0, 0xD9, 0x1D,
    0x04, 0x69, 0xF7, 0x01, 0xC6, 0x05, 0x5A, 0xC8, 0x40, 0x01, 0x01, 0xC7, 0x05, 0x68, 0xC4, 0xEA,
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x84, 0x9C, 0x00, 0x00, 0x00, 0x66, 0x3D, 0x0C, 0x00, 0x0F,
    0x83, 0x92, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0xF8, 0x69, 0xFF, 0x7C, 0x05, 0x00, 0x00, 0x81, 0xC7,
    0xB0, 0xF6, 0x19, 0x03, 0x39, 0x47, 0x4C, 0x75, 0x7E, 0xB9, 0x38, 0xC6, 0xE0, 0x00, 0xE8, 0x9D,
    0x3E, 0x51, 0x00, 0x85, 0xC0, 0x74, 0x70, 0x8B, 0xC8, 0x83, 0xE1, 0x3F, 0x8B, 0x0C, 0x8D, 0xB8,
    0xF1, 0x2E, 0x02, 0x8B, 0xF1, 0x85, 0xF6, 0x74, 0x5E, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x7E, 0x30, 0x75, 0x05, 0x39, 0x46, 0x2C, 0x74, 0x0F, 0x8B, 0x76, 0x04, 0x3B, 0xF1, 0x74,
    0x06, 0x85, 0xF6, 0x75, 0xEB, 0xEB, 0x02, 0x33, 0xF6, 0x85, 0xF6, 0x74, 0x3A, 0xA1, 0x6C, 0xDF,
    0xEB, 0x00, 0x8B, 0x08, 0x8B, 0x16, 0x8B, 0x52, 0x18, 0x51, 0x8B, 0xCE, 0xFF, 0xD2, 0x3C, 0x01,
    0x75, 0x25, 0x88, 0x44, 0x24, 0x3C, 0x8B, 0x06, 0x8B, 0x50, 0x3C, 0x6A, 0x00, 0x8D, 0x4C, 0x24,
    0x3C, 0x51, 0x8B, 0xCE, 0xC7, 0x44, 0x24, 0x40, 0x05, 0x00, 0x00, 0x00, 0xFF, 0xD2, 0x8D, 0x44,
    0x24, 0x38, 0xE8, 0x09, 0xAC, 0x4C, 0x00, 0xA1, 0x10, 0x7D, 0x52, 0x02, 0xC6, 0x80, 0x02, 0x0B,
    0x00, 0x00, 0x01, 0xC6, 0x80, 0x0C, 0x0B, 0x00, 0x00, 0x01, 0x8B, 0x8C, 0x24, 0xBC, 0x00, 0x00,
    0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x33, 0xCC, 0xE8, 0xA3, 0xD4, 0x5C, 0x00, 0x81, 0xC4, 0xB0, 0x00,
    0x00, 0x00, 0xC3,
];
#[cfg(test)]
const SS_CALLS_AT: [usize; 51] = [0xed, 0xf9, 0x121, 0x132, 0x158, 0x1d4, 0x1de, 0x1e7, 0x230, 0x26c, 0x27d, 0x2a5, 0x2b7, 0x2d6, 0x2e9, 0x2f4, 0x32c, 0x335, 0x341, 0x381, 0x3bb, 0x3c4, 0x3ce, 0x3d4, 0x41c, 0x431, 0x4bb, 0x4ef, 0x736, 0x741, 0x776, 0x788, 0x792, 0x7a6, 0x7e1, 0x82b, 0x857, 0x867, 0x8bb, 0x8c6, 0x8d7, 0x91c, 0x928, 0x939, 0x941, 0x946, 0x95d, 0x98d, 0x9de, 0xa52, 0xa77];
#[cfg(test)]
const SS_DISPS: [usize; 56] = [0x7, 0x17, 0x102, 0x128, 0x200, 0x237, 0x291, 0x2bd, 0x2f0, 0x2fe, 0x31e, 0x327, 0x348, 0x35a, 0x363, 0x36c, 0x375, 0x37d, 0x388, 0x38f, 0x3db, 0x438, 0x43e, 0x448, 0x44e, 0x455, 0x465, 0x46b, 0x471, 0x477, 0x47d, 0x489, 0x4c2, 0x567, 0x58f, 0x67e, 0x7ad, 0x7e8, 0x832, 0x8ab, 0x8de, 0x8f4, 0x900, 0x90c, 0x918, 0x924, 0x94d, 0x963, 0x998, 0x9a0, 0x9a6, 0x9ad, 0x9da, 0x9ef, 0xa1e, 0xa58];


extern "C" fn shot_start_w(idx: u32) {
    unsafe { shot_start(idx as i32) }
}
kx1!(shot_start_t, shot_start_w);

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let ports: [(&str, usize, &[u8], usize); 1] =
        [("cutscene_shot_start", SS_ADDR, &SS_BODY, shot_start_t as *const () as usize)];
    let mut ok = 0;
    for (name, addr, body, thunk) in ports {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    log(&format!("port_cutscene2: replaced {ok}/{} with Rust", ports.len()));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let n = HITS[0].load(Ordering::Relaxed);
    if n == 0 {
        return String::new();
    }
    let h = |i: usize| HITS[i].load(Ordering::Relaxed);
    format!(
        "port_cutscene2: shot_start {} [early {} objs {} cobjs {} prev {} nonfirst {} afd1 {} slots {} cam {} named {} unnamed {} tail {} evt {}]",
        n, h(1), h(2), h(3), h(4), h(5), h(6), h(7), h(8), h(9), h(10), h(11), h(12)
    )
}

#[cfg(test)]
#[allow(dead_code, clippy::all)]
pub(crate) mod dt {
        use std::cell::{Cell, RefCell};
        use std::ffi::c_void;
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
    
        pub const EAX: u8 = 1;
        pub const ECX: u8 = 2;
        pub const EDX: u8 = 4;
        pub const ESI: u8 = 8;
        pub const EDI: u8 = 16;
    
        /// Where a callee's result comes from.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Ret {
            /// Nothing in EAX that matters (a random value comes back anyway).
            Any,
            /// A flag: 0, 1, 0x100 (AL clear but EAX not), or random.
            Bool,
            /// 0 or a member of pool `n`.
            Ptr(usize),
            /// Always 0.
            Zero,
            /// Always a member of pool `n`.
            PtrNz(usize),
            /// Returns a double on the x87 stack (and a random value in EAX).
            St0,
            /// A random value below `n`.
            Mod(u32),
            /// The value of a register on entry (the callee returns its pointer argument).
            Reg(u8),
        }
    
        /// What a callee does besides returning.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Fx {
            None,
            /// Writes a random u32 at [ESI] (the xtbl getters).
            WriteUint,
            /// Writes a random f32 at [ESI].
            WriteFloat,
            /// Logs the bytes of a buffer: `args[0]` pointer, `args[1]` length (byte 3 masked when the
            /// length is 9: the original leaves it unwritten).
            LogBuf,
            /// Logs `ECX * 3` dwords at [EDX] (a vector list).
            LogVec,
            /// Logs the three bytes at [EDX].
            LogEdx3,
            /// Logs `n` dwords at the pointer in `args[0]`.
            LogArg0(usize),
            /// Logs `n` bytes at the pointer in EAX.
            LogEaxBytes(usize),
            /// Sets the byte at 0x25F5D90 to 0 / 1 (0 when ESI is 0): what 0x4A29E0 does.
            Flag25f,
        }
    
        /// Where a stub finds a pointer.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Src {
            Reg(u8),
            Arg(usize),
        }
    
        /// What a stub writes through a pointer.
        #[derive(Clone, Copy, PartialEq)]
        pub enum Out {
            /// A byte, 0 or 1.
            Byte01,
            /// `n` random f32 bit patterns (small values over-represented).
            Floats(usize),
            /// `n` random dwords.
            Dwords(usize),
            /// A small count at the pointer and that many small ints at the buffer in `Arg(1)`.
            CountBuf,
        }
    
        #[derive(Clone)]
        pub struct Callee {
            pub target: usize,
            pub nargs: usize,
            pub pop: usize,
            /// Registers the callee reads on entry (the only ones compared).
            pub reads: u8,
            pub ret: Ret,
            pub fx: Fx,
            pub silent: bool,
            /// Stack arguments the callee reads as one byte (the caller leaves stale upper bits).
            pub byte_args: u32,
            /// Registers the callee reads as one byte (the caller leaves stale upper bits).
            pub byte_regs: u8,
            /// A native replacement the original's call is aimed at (0: the recording stub).
            pub native: usize,
            pub logs: Vec<(Src, usize)>,
            /// (pointer source, byte offset, byte count) logged as bytes.
            pub blogs: Vec<(Src, usize, usize)>,
            pub outs: Vec<(Src, Out)>,
        }
    
        pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
            Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, native: 0, logs: Vec::new(), blogs: Vec::new(), outs: Vec::new() }
        }
    
        pub struct Rng(pub u64);
        impl Rng {
            pub fn u32(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 16) as u32
            }
            pub fn below(&mut self, n: u32) -> u32 {
                self.u32() % n
            }
            /// True once in `n`.
            pub fn one_in(&mut self, n: u32) -> bool {
                self.below(n) == 0
            }
            /// One of `v`, or (one time in four) a random value below `modulus` (any u32 when 0).
            pub fn mix(&mut self, v: &[u32], modulus: u32) -> u32 {
                if self.below(4) == 0 {
                    let r = self.u32();
                    if modulus == 0 { r } else { r % modulus }
                } else {
                    self.pick(v)
                }
            }
            pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
                v[self.below(v.len() as u32) as usize]
            }
            /// A float bit pattern with the awkward cases over-represented.
            pub fn float_bits(&mut self) -> u32 {
                match self.below(12) {
                    0 => 0,
                    1 => 0x8000_0000,
                    2 => 0x3F80_0000,
                    3 => 0xBF80_0000,
                    4 => 0x7FC0_0000,
                    5 => 0x7F80_0001, // signalling NaN
                    6 => 0x7F80_0000,
                    7 => 0x0000_0001, // denormal
                    8 => (self.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
                    9 => (self.below(20000) as f32 / 10.0).to_bits(),
                    _ => self.u32(),
                }
            }
        }
    
        pub fn fixed(addr: usize, len: usize) {
            let mut p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
            if p.is_null() {
                p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
            }
            assert_eq!(p as usize, addr, "cannot map {addr:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
        }
    
        /// The fixed regions the original touches: snapshotted, restored and compared as one.
        pub struct Mem {
            pub regions: Vec<(usize, usize)>,
        }
    
        impl Mem {
            /// Regions: every absolute address in `body` that falls in the game's data range (moved by
            /// `DELTA`), plus `extra` (game addresses), plus the arena; all rounded out to 64 KB.
            pub fn new(body: &[u8], extra: &[(usize, usize)]) -> Mem {
                let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::t(a), l)).collect();
                for i in 0..body.len().saturating_sub(3) {
                    let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                    if (0xD0_0000..0x300_0000).contains(&v) {
                        spans.push((super::t(v), 1));
                    }
                }
                spans.push((ARENA, ARENA_LEN));
                let mut pages: Vec<usize> = Vec::new();
                for (a, l) in spans {
                    let mut p = a & !0xFFFF;
                    while p < a + l {
                        pages.push(p);
                        p += 0x1_0000;
                    }
                }
                pages.sort_unstable();
                pages.dedup();
                let mut regions: Vec<(usize, usize)> = Vec::new();
                for p in pages {
                    match regions.last_mut() {
                        Some(r) if r.0 + r.1 == p => r.1 += 0x1_0000,
                        _ => regions.push((p, 0x1_0000)),
                    }
                }
                for &(a, l) in &regions {
                    fixed(a, l);
                }
                Mem { regions }
            }
    
            pub fn fill_random(&self, rng: &mut Rng) {
                for &(a, l) in &self.regions {
                    let s = unsafe { std::slice::from_raw_parts_mut(a as *mut u32, l / 4) };
                    for w in s.iter_mut() {
                        *w = rng.u32();
                    }
                }
            }
    
            /// Zero `l` bytes at the game address `a`.
            pub fn zero(&self, a: usize, l: usize) {
                unsafe { std::ptr::write_bytes(super::t(a) as *mut u8, 0, l) };
            }
    
            pub fn snapshot(&self) -> Vec<u8> {
                let mut v = Vec::new();
                for &(a, l) in &self.regions {
                    v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, l) });
                }
                v
            }
    
            pub fn restore(&self, snap: &[u8]) {
                let mut at = 0;
                for &(a, l) in &self.regions {
                    unsafe { std::ptr::copy_nonoverlapping(snap[at..at + l].as_ptr(), a as *mut u8, l) };
                    at += l;
                }
            }
    
            /// The first difference between two snapshots, as an address.
            pub fn first_diff(&self, a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
                let mut at = 0;
                for &(base, l) in &self.regions {
                    for i in 0..l {
                        if a[at + i] != b[at + i] {
                            return Some((base + i, a[at + i], b[at + i]));
                        }
                    }
                    at += l;
                }
                None
            }
    
            pub fn count_diffs(&self, a: &[u8], b: &[u8]) -> usize {
                a.iter().zip(b).filter(|(x, y)| x != y).count()
            }
        }
    
        #[repr(C)]
        pub struct Pushad {
            edi: u32,
            esi: u32,
            ebp: u32,
            esp: u32,
            ebx: u32,
            edx: u32,
            ecx: u32,
            eax: u32,
        }
    
        thread_local! {
            static CALLEES: RefCell<Vec<Callee>> = const { RefCell::new(Vec::new()) };
            static STUBS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
            static POOLS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
            static LOG: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
            static SRNG: Cell<u64> = const { Cell::new(1) };
        }
    
        pub fn set_pools(p: Vec<Vec<u32>>) {
            POOLS.with(|x| *x.borrow_mut() = p);
        }
    
        /// The address a call to `f` should really go to: its stub when it has one.
        pub fn redirect(f: usize) -> usize {
            CALLEES.with(|c| {
                let c = c.borrow();
                match c.iter().position(|x| x.target == f) {
                    Some(i) => STUBS.with(|s| s.borrow()[i]),
                    None => f,
                }
            })
        }
    
        fn srng() -> u32 {
            SRNG.with(|s| {
                let mut r = Rng(s.get());
                let v = r.u32();
                s.set(r.0);
                v
            })
        }
    
        fn get(r: &Pushad, args: *const u32, src: Src) -> u32 {
            match src {
                Src::Reg(b) => match b {
                    EAX => r.eax,
                    ECX => r.ecx,
                    EDX => r.edx,
                    ESI => r.esi,
                    _ => r.edi,
                },
                Src::Arg(k) => unsafe { *args.add(k) },
            }
        }
    
        extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
            let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
            unsafe { LAST_STUB = c.target as u32 };
            let r = unsafe { &*regs };
            // Pointers into the stack differ between the two runs: log a marker for them.
            let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
            if !c.silent {
                let mut e = vec![c.target as u32];
                for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                    e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else { mask(v) });
                }
                for k in 0..c.nargs {
                    let v = unsafe { *args.add(k) };
                    e.push(if c.byte_args >> k & 1 != 0 { v & 0xFF } else { mask(v) });
                }
                match c.fx {
                    Fx::LogVec => {
                        for k in 0..(r.ecx.min(40) * 3) as usize {
                            e.push(mask(unsafe { *((r.edx as usize + k * 4) as *const u32) }));
                        }
                    }
                    Fx::LogEdx3 => {
                        for k in 0..3usize {
                            e.push(unsafe { *((r.edx as usize + k) as *const u8) } as u32);
                        }
                    }
                    Fx::LogEaxBytes(n) => {
                        for k in 0..n {
                            e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
                        }
                    }
                    Fx::LogArg0(n) => {
                        let p = unsafe { *args } as usize;
                        for k in 0..n {
                            e.push(mask(unsafe { *((p + k * 4) as *const u32) }));
                        }
                    }
                    _ => {}
                }
                for &(src, n) in &c.logs {
                    let p = get(r, args, src) as usize;
                    for k in 0..n {
                        e.push(if p == 0 { 0 } else { mask(unsafe { *((p + k * 4) as *const u32) }) });
                    }
                }
                for &(src, off, n) in &c.blogs {
                    let p = get(r, args, src) as usize;
                    for k in 0..n {
                        e.push(if p == 0 { 0 } else { (unsafe { *((p + off + k) as *const u8) }) as u32 });
                    }
                }
                if c.fx == Fx::LogBuf {
                    let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                    for k in 0..len {
                        let mut b = unsafe { *((p + k) as *const u8) };
                        if (len == 9 || len == 7) && k == 3 {
                            b = 0;
                        }
                        e.push(b as u32);
                    }
                }
                LOG.with(|l| l.borrow_mut().push(e));
            }
            match c.fx {
                Fx::WriteUint => {
                    let v = match srng() % 6 {
                        0 => 0,
                        1 => srng() % 10,
                        2 => srng() % 100,
                        3 => srng() % 5000,
                        4 => 0x7FFF_FFFF,
                        _ => srng(),
                    };
                    unsafe { *(super::t(r.esi as usize) as *mut u32) = v };
                }
                Fx::Flag25f => {
                    let v = if r.esi == 0 { 0 } else { (srng() & 1) as u8 };
                    unsafe { *(super::t(0x25F_5D90) as *mut u8) = v };
                }
                Fx::WriteFloat => {
                    let mut rng = Rng(srng() as u64 | 1 << 40);
                    unsafe { *(super::t(r.esi as usize) as *mut u32) = rng.float_bits() };
                }
                _ => {}
            }
            for &(src, out) in &c.outs {
                let p = get(r, args, src) as usize;
                if p == 0 {
                    continue;
                }
                unsafe {
                    match out {
                        Out::Byte01 => *(p as *mut u8) = (srng() & 1) as u8,
                        Out::Floats(n) => {
                            for k in 0..n {
                                let v = match srng() % 8 {
                                    0 => 0.0f32.to_bits(),
                                    1 => 1.0f32.to_bits(),
                                    2 => (-1.0f32).to_bits(),
                                    3..=5 => (((srng() % 2000) as f32) / 100.0 - 10.0).to_bits(),
                                    _ => Rng(srng() as u64 | 1 << 40).float_bits(),
                                };
                                *((p + k * 4) as *mut u32) = v;
                            }
                        }
                        Out::Dwords(n) => {
                            for k in 0..n {
                                *((p + k * 4) as *mut u32) = srng();
                            }
                        }
                        Out::CountBuf => {
                            // A character's skeleton lists up to 0x80 nodes (the original's buffer
                            // holds 124): mostly small counts, often a whole skeleton.
                            let n = match srng() % 4 {
                                0 => srng() % 9,
                                1 => 60 + srng() % 65,
                                _ => srng() % 125,
                            };
                            *(p as *mut u32) = n;
                            let buf = get(r, args, Src::Arg(1)) as usize;
                            for k in 0..n as usize {
                                *((buf + k * 4) as *mut u32) = (srng() % 40).wrapping_sub(3);
                            }
                        }
                    }
                }
            }
            match c.ret {
                Ret::Zero => 0,
                Ret::Reg(b) => get(r, args, Src::Reg(b)),
                Ret::Mod(n) => srng() % n,
                Ret::Any => srng(),
                Ret::St0 => {
                    let v = match srng() % 8 {
                        0 => 0.0,
                        1 => f32::from_bits(srng()) as f64,
                        2 => srng() as f64 / 7.0,
                        3 => (srng() % 5000) as f64 / 10.0,
                        4 => f64::from_bits((srng() as u64) << 32 | srng() as u64),
                        5 => 1000.0,
                        _ => (srng() % 100) as f64,
                    };
                    unsafe { ST0V = v };
                    srng()
                }
                Ret::PtrNz(n) => POOLS.with(|p| {
                    let p = p.borrow();
                    let pool = &p[n];
                    pool[srng() as usize % pool.len()]
                }),
                Ret::Bool => match srng() % 6 {
                    0 | 1 => 0,
                    2 | 3 => 1,
                    4 => 0x100,
                    _ => srng(),
                },
                Ret::Ptr(n) => {
                    if srng() % 4 == 0 {
                        0
                    } else {
                        POOLS.with(|p| {
                            let p = p.borrow();
                            let pool = &p[n];
                            pool[srng() as usize % pool.len()]
                        })
                    }
                }
            }
        }
    
        static mut ST0V: f64 = 0.0;
        pub static mut LAST_STUB: u32 = 0;
    
        /// `stub_common` for the callees that return on ST0.
        #[unsafe(naked)]
        extern "C" fn stub_common_st0() {
            std::arch::naked_asm!(
                "pushad",
                "lea eax, [esp + 44]",
                "push eax",
                "push dword ptr [esp + 40]",
                "lea eax, [esp + 8]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "mov [esp + 28], eax",
                "popad",
                "fld qword ptr [{v}]",
                "ret 4",
                f = sym stub_rs,
                v = sym ST0V,
            )
        }
    
        #[unsafe(naked)]
        extern "C" fn stub_common() {
            std::arch::naked_asm!(
                "pushad",
                "lea eax, [esp + 44]",
                "push eax",
                "push dword ptr [esp + 40]",
                "lea eax, [esp + 8]",
                "push eax",
                "call {f}",
                "add esp, 12",
                "mov [esp + 28], eax",
                "popad",
                "ret 4",
                f = sym stub_rs,
            )
        }
    
        #[repr(C)]
        #[derive(Clone, Copy, PartialEq, Debug)]
        pub struct Regs {
            pub eax: u32,
            pub ecx: u32,
            pub edx: u32,
            pub ebx: u32,
            pub esi: u32,
            pub edi: u32,
            pub ebp: u32,
        }
    
        #[repr(C)]
        struct Block {
            f: u32,
            r: Regs,
            args: [u32; 3],
            esp: u32,
            stack: u32,
        }
    
        /// Call `f` with all seven registers from `r` and three stack arguments, capture them after.
        pub unsafe fn call_orig(f: usize, r: &mut Regs, args: [u32; 3]) {
            let mut b = Block { f: f as u32, r: *r, args, esp: 0, stack: (STACK + STACK_LEN - 0x100) as u32 };
            unsafe {
                std::arch::asm!(
                    "push ebp", "push ebx", "push esi", "push edi",
                    "mov [eax + 44], esp",
                    "mov esp, [eax + 48]",
                    "push eax", "push eax", "push eax", "push eax",
                    "push dword ptr [eax]",
                    "push dword ptr [eax + 40]",
                    "push dword ptr [eax + 36]",
                    "push dword ptr [eax + 32]",
                    "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
                    "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
                    "mov eax, [eax + 4]",
                    "call dword ptr [esp + 12]",
                    "push ebx",
                    "mov ebx, [esp + 20]",
                    "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
                    "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
                    "pop dword ptr [ebx + 16]",
                    "mov esp, [ebx + 44]",
                    "pop edi", "pop esi", "pop ebx", "pop ebp",
                    inout("eax") &mut b as *mut Block => _,
                    lateout("ecx") _,
                    lateout("edx") _,
                    clobber_abi("C"),
                );
            }
            *r = b.r;
        }
    
        /// Reports a crash with the faulting address and where it happened (original copy offset).
        unsafe extern "system" fn veh(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
            unsafe {
                let rec = &*(*info).ExceptionRecord;
                let ctx = &*(*info).ContextRecord;
                let orig = ORIG_AT.with(|o| o.get());
                eprintln!("last stub {:#x}", unsafe { LAST_STUB });
                eprintln!(
                    "EXCEPTION {:#x} at eip {:#x} (original +{:#x}) access {:x?} eax {:#x} ecx {:#x} edx {:#x} ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}",
                    rec.ExceptionCode, ctx.Eip, (ctx.Eip as usize).wrapping_sub(orig), &rec.ExceptionInformation[..2], ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx, ctx.Esi, ctx.Edi, ctx.Ebp
                );
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
            0
        }
    
        thread_local! {
            static ORIG_AT: Cell<usize> = const { Cell::new(0) };
        }
    
        pub struct Harness {
            pub mem: Mem,
            /// The original body, relocated to a private copy with its calls aimed at the stubs.
            pub orig: usize,
            pub thunk: usize,
            /// Also compare EAX after the call.
            pub cmp_eax: bool,
        }
    
        /// Executable memory for the stubs and the patched copy.
        fn exec_alloc(len: usize) -> usize {
            let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
            assert!(p != 0);
            p
        }
    
        impl Harness {
            /// `base`: the original's address; `body` / `calls`: its bytes and the offsets of its `call
            /// rel32`s; `thunk`: the Rust replacement's entry.
            pub fn new(base: usize, body: &[u8], calls: &[usize], disps: &[usize], callees: Vec<Callee>, extra: &[(usize, usize)], thunk: usize) -> Harness {
                let mem = Mem::new(body, extra);
                fixed(STACK, STACK_LEN);
                let stubs = exec_alloc(callees.len() * 16 + 16);
                let mut addrs = Vec::new();
                for (i, c) in callees.iter().enumerate() {
                    let a = stubs + i * 16;
                    let mut code = [0x90u8; 16];
                    code[0] = 0x68;
                    code[1..5].copy_from_slice(&(i as u32).to_le_bytes());
                    code[5] = 0xE8;
                    let common = if c.ret == Ret::St0 { stub_common_st0 as usize } else { stub_common as usize };
                    let rel = (common as i32).wrapping_sub((a + 10) as i32);
                    code[6..10].copy_from_slice(&rel.to_le_bytes());
                    if c.pop == 0 {
                        code[10] = 0xC3;
                    } else {
                        code[10] = 0xC2;
                        code[11..13].copy_from_slice(&(c.pop as u16).to_le_bytes());
                    }
                    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), a as *mut u8, 16) };
                    addrs.push(a);
                }
                CALLEES.with(|c| *c.borrow_mut() = callees.clone());
                STUBS.with(|s| *s.borrow_mut() = addrs.clone());
                let orig = exec_alloc(body.len() + 16);
                unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), orig as *mut u8, body.len()) };
                for &off in disps {
                    let v = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()).wrapping_add(super::DELTA as u32);
                    unsafe { std::ptr::write_unaligned((orig + off) as *mut u32, v) };
                }
                for &off in calls {
                    let rel = i32::from_le_bytes(body[off + 1..off + 5].try_into().unwrap());
                    let target = (base + off + 5).wrapping_add(rel as usize);
                    let i = callees.iter().position(|c| c.target == target).unwrap_or_else(|| panic!("call at {off:#x} to {target:#x} has no callee entry"));
                    let dest = if callees[i].native != 0 { callees[i].native } else { addrs[i] };
                    let r = (dest as i32).wrapping_sub((orig + off + 5) as i32);
                    unsafe { std::ptr::write_unaligned((orig + off + 1) as *mut i32, r) };
                }
                ORIG_AT.with(|o| o.set(orig));
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| unsafe {
                    windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(veh));
                });
                Harness { mem, orig, thunk, cmp_eax: false }
            }
    
            fn run(&self, f: usize, ecx: u32, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
                SRNG.with(|s| s.set(seed | 1));
                LOG.with(|l| l.borrow_mut().clear());
                let before = Regs { eax: 0xAAAA_0001, ecx, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 };
                let mut r = before;
                unsafe { call_orig(f, &mut r, args) };
                (LOG.with(|l| std::mem::take(&mut *l.borrow_mut())), before, r)
            }
    
            /// One case on the current memory: original, then Rust, from the same snapshot.
            pub fn check(&self, ecx: u32, args: [u32; 3], seed: u64) -> Result<(), String> {
                let snap = self.mem.snapshot();
                let (la, ba, ra) = self.run(self.orig, ecx, args, seed);
                let after_a = self.mem.snapshot();
                self.mem.restore(&snap);
                let (lb, _bb, rb) = self.run(self.thunk, ecx, args, seed);
                let after_b = self.mem.snapshot();
                if self.cmp_eax && ra.eax != rb.eax {
                    return Err(format!("eax: original {:#x} rust {:#x}", ra.eax, rb.eax));
                }
                // callee-saved registers must come back unchanged
                for (name, x, y, z) in [("ebx", ba.ebx, ra.ebx, rb.ebx), ("esi", ba.esi, ra.esi, rb.esi), ("edi", ba.edi, ra.edi, rb.edi), ("ebp", ba.ebp, ra.ebp, rb.ebp)] {
                    if x != y || x != z {
                        return Err(format!("register {name}: in {x:#x} original {y:#x} rust {z:#x}"));
                    }
                }
                for (i, (a, b)) in la.iter().zip(&lb).enumerate() {
                    if a != b {
                        let prev: Vec<&Vec<u32>> = la[i.saturating_sub(3)..i].iter().collect();
                        return Err(format!("call #{i} differs:
      previous {:x?}
      original {:x?}
      rust     {:x?}", prev, a, b));
                    }
                }
                if la.len() != lb.len() {
                    let longer = if la.len() > lb.len() { ("original", &la) } else { ("rust", &lb) };
                    return Err(format!("original made {} calls, rust {}; the {} next calls {:x?}", la.len(), lb.len(), longer.0, longer.1[la.len().min(lb.len())]));
                }
                if let Some((a, x, y)) = self.mem.first_diff(&after_a, &after_b) {
                    return Err(format!("memory differs at {a:#x}: original {x:#x} rust {y:#x} ({} bytes differ)", self.mem.count_diffs(&after_a, &after_b)));
                }
                Ok(())
            }
        }
    
        pub fn lock() -> std::sync::MutexGuard<'static, ()> {
            static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
            L.lock().unwrap_or_else(|e| e.into_inner())
        }
    
        /// Where the test world's objects live.
        pub const ARENA: usize = 0x6000_0000;
        /// The tests run on a stack of their own, outside the game's address range.
        pub const STACK: usize = 0x6100_0000;
        pub const STACK_LEN: usize = 0x10_0000;
        pub const ARENA_LEN: usize = 0x18_0000;
    
        /// A bump allocator over the arena for one case.
        pub struct Arena(pub usize);
        impl Arena {
            pub fn new() -> Arena {
                Arena(ARENA)
            }
            /// `n` objects of `size` bytes (random contents are the caller's business).
            pub fn pool(&mut self, n: usize, size: usize) -> Vec<u32> {
                let v = (0..n).map(|i| (self.0 + i * size) as u32).collect();
                self.0 += n * size;
                assert!(self.0 <= ARENA + ARENA_LEN, "arena full");
                v
            }
        }
    
        pub unsafe fn put(a: u32, off: usize, v: u32) {
            unsafe { *((a as usize + off) as *mut u32) = v }
        }
        pub unsafe fn putb(a: u32, off: usize, v: u8) {
            unsafe { *((a as usize + off) as *mut u8) = v }
        }
    }


#[cfg(test)]
mod ss_test {
    use super::dt::*;
    use super::*;

    const VT34: usize = 0xCB00_0034;
    const VT18: usize = 0xCB00_0018;
    const VT3C: usize = 0xCB00_003C;
    const IMP: usize = 0xCB00_0100;

    fn c(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret) -> Callee {
        callee(target, nargs, pop, reads, ret, Fx::None)
    }

    pub(crate) fn callees() -> Vec<Callee> {
        let mut v = Vec::new();
        let mut x = c(0xa52bb0, 0, 0, EAX | ECX, Ret::Ptr(0));
        x.outs.push((Src::Reg(EAX), Out::Byte01));
        v.push(x);
        v.push(c(0xa52c70, 0, 0, EAX | ESI, Ret::Ptr(0)));
        v.push(c(0x55a930, 4, 16, 0, Ret::Any));
        v.push(c(0x6c32a0, 0, 0, ESI, Ret::Any));
        v.push(c(0x6ef620, 3, 0, 0, Ret::Any));
        v.push(c(0x6f0660, 3, 0, 0, Ret::Any));
        v.push(c(0xcedc10, 0, 0, ECX, Ret::Any));
        v.push(c(0xcedc50, 1, 0, ECX, Ret::Any));
        v.push(c(0x6c1770, 1, 0, EAX, Ret::Any));
        v.push(c(0x522b80, 1, 0, 0, Ret::Any));
        v.push(c(0x6bf6c0, 2, 0, 0, Ret::Any));
        v.push(c(0xbf2c10, 2, 0, EDI | ESI, Ret::Any));
        v.push(c(0x6c7d30, 2, 0, 0, Ret::Any));
        v.push(c(0x495240, 0, 0, 0, Ret::Any));
        let mut x = c(0x6c8f50, 0, 0, EAX, Ret::Reg(EAX));
        x.outs.push((Src::Reg(EAX), Out::Floats(3)));
        v.push(x);
        let mut x = c(0x4a2840, 5, 0, EAX, Ret::Any);
        x.logs.push((Src::Arg(0), 3));
        v.push(x);
        v.push(c(0x6c24e0, 1, 0, 0, Ret::Any));
        v.push(c(0x6c2ef0, 1, 0, 0, Ret::Any));
        v.push(c(0x47b660, 0, 0, ESI, Ret::Any));
        v.push(c(0x6c1ef0, 1, 0, EDI, Ret::Any));
        v.push(c(0x6c1d30, 1, 0, 0, Ret::Any));
        v.push(c(0x6cd6d0, 0, 0, ECX, Ret::Any));
        v.push(c(0x6c1b50, 1, 0, 0, Ret::Any));
        v.push(c(0x6c1cb0, 0, 0, EDI | EDX | ESI, Ret::Any));
        v.push(c(0x6d8900, 0, 0, EDI, Ret::Any));
        v.push(c(0x6c1660, 1, 0, EAX | ECX, Ret::Any));
        let mut x = c(0x6c92a0, 1, 4, EDI | ESI, Ret::Any);
        x.logs.push((Src::Reg(ESI), 3));
        x.outs.push((Src::Reg(EDI), Out::Floats(3)));
        v.push(x);
        let mut x = c(0x6c34c0, 3, 0, EAX | ECX, Ret::Reg(ECX));
        x.outs.push((Src::Reg(ECX), Out::Floats(3)));
        v.push(x);
        let mut x = c(0x4a29e0, 0, 0, EAX | ECX | ESI | EDI, Ret::Any);
        x.fx = Fx::Flag25f;
        x.logs.push((Src::Reg(EAX), 3));
        x.logs.push((Src::Reg(EDI), 9));
        v.push(x);
        let mut x = c(0x6beed0, 0, 0, EAX | ECX, Ret::Any);
        x.outs.push((Src::Reg(EAX), Out::Floats(3)));
        v.push(x);
        let mut x = c(0x6c78b0, 2, 0, ECX, Ret::Any);
        x.logs.push((Src::Reg(ECX), 3));
        x.outs.push((Src::Arg(0), Out::Floats(3)));
        v.push(x);
        let mut x = c(0xbde150, 0, 0, ESI, Ret::Any);
        x.logs.push((Src::Reg(ESI), 3));
        x.outs.push((Src::Reg(ESI), Out::Floats(3)));
        v.push(x);
        let mut x = c(0xbe4b20, 0, 0, EAX | ECX, Ret::Any);
        x.logs.push((Src::Reg(EAX), 3));
        x.outs.push((Src::Reg(ECX), Out::Dwords(9)));
        v.push(x);
        v.push(c(0x40ea20, 1, 0, 0, Ret::St0));
        v.push(c(0x4932f0, 0, 0, 0, Ret::Any));
        v.push(c(0x4b3ff0, 1, 0, EAX, Ret::Any));
        v.push(c(0x6befc0, 3, 0, 0, Ret::Any));
        v.push(c(0x4c8770, 0, 0, 0, Ret::Any));
        v.push(c(0x4cf840, 0, 0, 0, Ret::Any));
        v.push(c(0x6cd9a0, 1, 0, EDI, Ret::Any));
        let mut x = c(0x4d1270, 2, 0, EDX, Ret::Any);
        x.byte_regs = EDX;
        v.push(x);
        v.push(c(0xbdc9b0, 0, 0, ECX, Ret::Mod(4)));
        v.push(c(0xbdca30, 1, 4, EAX | EDX, Ret::PtrNz(2)));
        v.push(c(0xa275f0, 0, 0, EAX, Ret::Any));
        v.push(c(0xa27660, 1, 0, 0, Ret::Ptr(0)));
        v.push(c(VT34, 0, 0, ECX, Ret::Ptr(0)));
        v.push(c(VT18, 1, 4, ECX, Ret::Bool));
        let mut x = c(VT3C, 2, 8, ECX, Ret::Any);
        x.logs.push((Src::Arg(0), 1));
        x.blogs.push((Src::Arg(0), 4, 1));
        v.push(x);
        v.push(c(0xb93790, 0, 0, EAX, Ret::Any));
        v.push(c(IMP, 2, 8, 0, Ret::Any));
        let mut x = c(0xc9604f, 0, 0, 0, Ret::Any);
        x.silent = true;
        v.push(x);
        v
    }

    fn fl(rng: &mut Rng) -> u32 {
        rng.float_bits()
    }

    /// Builds one random world; returns the shot index to start.
    unsafe fn world(h: &Harness, rng: &mut Rng) -> u32 {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let vt = a.pool(1, 0x100)[0];
            let objs = a.pool(6, 0x8500);
            let hs = a.pool(6, 0x40);
            let cells = a.pool(6, 8);
            let cobjs = a.pool(12, 0x40);
            let nodes = a.pool(4, 0x200);
            let knodes = a.pool(64 * 3, 0x40);
            let strs = a.pool(8, 0x10);
            let p = a.pool(1, 0x1700)[0];
            let shots = a.pool(1, 6 * 0x170)[0];
            let arr_a = a.pool(6, 6 * 0x7c);
            let arr_b = a.pool(6, 6 * 0x7c);
            let arr_c = a.pool(6, 6 * 0x2c);
            let recs = a.pool(64, 4 * 12);
            let flags = a.pool(256, 16);
            let slots_obj = a.pool(1, 0x20)[0];
            let slots_arr = a.pool(1, 8 * 0x90)[0];
            let lists = a.pool(8, 16 * 4);
            let ac8 = a.pool(1, 16 * 4)[0];
            let ae0 = a.pool(1, 8 * 4)[0];
            let o15f8 = a.pool(1, 0x480)[0];
            let local = a.pool(1, 0x40)[0];
            let evcell = a.pool(2, 8);
            set_pools(vec![objs.clone(), hs.clone(), cells.clone()]);

            put(vt, 0x34, redirect(VT34) as u32);
            put(vt, 0x18, redirect(VT18) as u32);
            put(vt, 0x3c, redirect(VT3C) as u32);
            wd(0xdb83d0, redirect(IMP) as u32);
            for &o in &cobjs {
                put(o, 0, vt);
            }
            for &o in &objs {
                put(o, 0x5e0, hs[rng.below(6) as usize]);
                putb(o, 0x84d8, 1);
                put(o, 0x82a0, rng.mix(&[0xFFFF_FFFF, 0xFFFF_FFFF, 0, 1], 0));
                put(o, 0x758, rng.u32());
            }
            // crc list and cells
            let keys: Vec<u32> = (0..4).map(|_| rng.u32()).collect();
            for (k, &n) in nodes.iter().enumerate() {
                put(n, 0, vt);
                put(n, 0xc, keys[k]);
                put(n, 0x17c, if rng.one_in(8) { 0 } else { nodes[(k + 1) % 4] });
            }
            for &cl in &cells {
                put(cl, 0, if rng.one_in(3) { rng.u32() } else { keys[rng.below(4) as usize] });
            }
            wd(0x2f25254, if rng.one_in(6) { 0 } else { nodes[rng.below(4) as usize] });
            // the hash table of the event objects
            let ax = rng.mix(&[0, 1, 5, 11, 12, 0x1_0003, 7, 3], 0x20);
            wd(0x252853c, ax);
            let lo = (ax & 0xFFFF) as usize;
            for k in 0..12usize {
                let e = 0x319f6b0 + k * 0x57c;
                wd(e + 0x4c, if rng.one_in(3) { rng.u32() } else if lo == k { ax } else { ax ^ 1 });
            }
            let target_edi = if lo < 12 { (0x319f6b0 + lo * 0x57c) as u32 } else { 0x319f6b0 };
            for s in 0..64usize {
                let mut head = 0u32;
                if s < 6 || rng.one_in(8) {
                    let n = 1 + rng.below(3) as usize;
                    let ks: Vec<u32> = (0..n).map(|i| knodes[s * 3 + i]).collect();
                    for (i, &kn) in ks.iter().enumerate() {
                        put(kn, 0, vt);
                        put(kn, 4, if i + 1 < n { ks[i + 1] } else if rng.one_in(6) { 0 } else { ks[0] });
                        put(kn, 0x2c, rng.pick(&[(s & 3) as u32, 1, 2, 3, 0]));
                        let rnd = rng.u32();
                        put(kn, 0x30, rng.pick(&[target_edi, target_edi, rnd]));
                    }
                    head = ks[0];
                }
                wd(0x22ef1b8 + s * 4, head);
            }
            wd(0xebdf6c, evcell[rng.below(2) as usize]);
            put(evcell[0], 0, rng.u32());
            put(evcell[1], 0, rng.u32());
            for (k, &s) in strs.iter().enumerate() {
                put(s, 0, 0x6968_6700 + k as u32);
            }
            // global values
            for g in [0x2527d14usize, 0x2527d15, 0x2527cf7, 0x2527d25, 0x25f5d90] {
                wb(g, rng.pick(&[0, 1, 1, 0, 2]));
            }
            for g in [0xdd0550usize, 0xdd0794, 0xdd1944, 0xdd1c50] {
                wd(g, fl(rng));
            }
            let (kr1, kr2) = (rng.u32() as u64, rng.u32() as u64);
            let kbits = rng.pick(&[0.25f64.to_bits(), 1.5f64.to_bits(), kr1 | kr2 << 32, 0x7FF8_0000_0000_0001]);
            wd(0xe5bfe8, kbits as u32);
            wd(0xe5bfe8 + 4, (kbits >> 32) as u32);
            wd(0x21703d4, local);
            wd(0x140e670, rng.u32());

            // the cutscene state
            let nshots = 1 + rng.below(6);
            let max_shots = rng.one_in(5);
            let ns = if max_shots { 6 } else { nshots };
            put(p, 0x153c, ns);
            put(p, 0x1534, shots);
            put(p, 0x1540, if rng.one_in(3) { 0 } else { shots + 0x170 * rng.below(6) });
            for off in [0xafdusize, 0xb02, 0xb03, 0x1608] {
                putb(p, off, rng.pick(&[0, 1, 1, 0, 2]));
            }
            let nobj = if rng.one_in(6) { 4 } else { rng.below(4) };
            put(p, 0xae8, nobj);
            put(p, 0xae0, ae0);
            for k in 0..8usize {
                put(ae0, k * 4, cobjs[k]);
                put(ac8, k * 4, cobjs[4 + k]);
            }
            put(p, 0xac8, ac8);
            put(p, 0xac0, if rng.one_in(5) { 0 } else { rng.u32() | 1 });
            let ac4 = rng.below(4);
            put(p, 0xac4, ac4);
            put(p, 0xaf0, if rng.one_in(5) { 0 } else { rng.u32() | 1 });
            put(p, 0xaf4, slots_obj);
            put(p, 0x15f8, if rng.one_in(3) { 0 } else { o15f8 });
            put(p, 0x1600, if rng.one_in(2) { 0 } else { rng.u32() });
            put(p, 0x1660, fl(rng));
            let nslots = rng.pick(&[0u32, 1, 3, 6, 8, 8]);
            put(slots_obj, 0xc, slots_arr);
            put(slots_obj, 0x10, nslots);
            for s in 0..8usize {
                let sl = slots_arr + (s * 0x90) as u32;
                put(sl, 0x80, lists[s]);
                put(sl, 0x84, rng.pick(&[0xFFFF_FFFFu32, 0, 1, 2, 4, 7, 7]));
                put(sl, 0x88, rng.pick(&[0u32, 0, 1, 2, 3, 7]));
            }
            // records and flags
            for (ri, &r) in recs.iter().enumerate() {
                for k in 0..4usize {
                    let rec = r + (k * 12) as u32;
                    put(rec, 0, flags[(ri * 4 + k) % 256]);
                    put(rec, 4, rng.u32());
                    put(rec, 8, rng.pick(&[0xFFFF_FFFFu32, 0, 1, 2, 3, 6, 8, 8]));
                }
            }
            for &f in &flags {
                for k in 0..16u32 {
                    putb(f, k as usize, rng.pick(&[0, 0, 1, 2]));
                }
            }
            // the shots
            let max_a = rng.one_in(4);
            for s in 0..6usize {
                let sh = shots + (s as u32) * 0x170;
                put(sh, 4, rng.u32());
                putb(sh, 0x9c, rng.pick(&[0, 1, 1, 2]));
                for off in [0xa0usize, 0xa4, 0xa8, 0xac] {
                    put(sh, off, fl(rng));
                }
                put(sh, 0xb0, if rng.one_in(3) { 0 } else { strs[rng.below(8) as usize] });
                put(sh, 0xb4, rng.u32());
                put(sh, 0xfc, if rng.one_in(2) { 0 } else { rng.u32() });
                put(sh, 0x100, arr_a[s]);
                put(sh, 0x108, if max_a { 6 } else { rng.pick(&[0xFFFF_FFFFu32, 0, 1, 2, 3, 6]) });
                put(sh, 0x10c, arr_b[s]);
                put(sh, 0x114, rng.pick(&[0u32, 1, 2, 6, 6]));
                put(sh, 0x124, arr_c[s]);
                put(sh, 0x12c, rng.pick(&[0u32, 1, 3, 6, 6]));
                put(sh, 0x15c, rng.u32());
                for e in 0..6usize {
                    for (arr, is_a) in [(arr_a[s], true), (arr_b[s], false)] {
                        let en = arr + (e * 0x7c) as u32;
                        put(en, 0, if rng.one_in(4) { 0 } else { strs[rng.below(8) as usize] });
                        putb(en, 0x34, rng.pick(&[0, 0, 1, 2]));
                        putb(en, 0x36, rng.pick(&[0, 1, 1]));
                        put(en, 0x4c, recs[rng.below(64) as usize]);
                        put(en, 0x54, rng.pick(&[0xFFFF_FFFFu32, 0, 1, 2, 3, 4, 5]));
                        if is_a {
                            put(en, 0x60, if rng.one_in(2) { 0 } else { rng.u32() });
                            put(en, 0x6c, if rng.one_in(2) { 0 } else { rng.u32() });
                        }
                    }
                }
            }
            let idx: u32 = match rng.below(24) {
                0 => 0xFFFF_FFFF,
                1 => ns,
                _ => rng.below(ns),
            };
            wd(0x2527d10, if rng.one_in(30) { 0 } else { p });
            idx
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the module comment)"]
    fn shot_start_matches_original() {
        let _g = lock();
        let extras = [(0x319f6b0usize, 12 * 0x57c), (0x22ef1b8usize, 64 * 4)];
        let h = Harness::new(SS_ADDR, &SS_BODY, &SS_CALLS_AT, &SS_DISPS, callees(), &extras, shot_start_t as *const () as usize);
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            let idx = unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0xBADC_0DE0, [idx, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        eprintln!("{}", summary());
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}
