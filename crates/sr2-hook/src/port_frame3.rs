//! Port module (frame-loop callees, round 3): sim_frame callees as whole-function replacements
//! on the game (sim) thread. Callees stay the original game functions, called by address with
//! the registers the LTCG build sets up; every comparison, list splice and global write follows
//! the disassembly.
//!
//! - weapon_target_lock_update 0xB6AD40 (1412 bytes)
//! - vehicle_siren_update      0x489CC0 (1333)
//! - chunk_effects_update      0x4AAC40 (1205)

#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
include!("fix_thunks.inc");

// The offline differential test (module `difftest`) keeps the game's globals (0xD00000..0x350000)
// at a fixed offset, because their addresses are taken in a test process; the Rust ports read
// through these shadows there. Pointers to the test's objects lie outside that range. The
// production build uses the portlib versions.
#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn tr(p: usize) -> usize {
    if (0xD0_0000..0x350_0000).contains(&p) { p + DELTA } else { p }
}
#[cfg(test)]
unsafe fn rd(p: usize) -> u32 {
    unsafe { *(tr(p) as *const u32) }
}
#[cfg(test)]
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(tr(p) as *const u8) }
}
#[cfg(test)]
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(tr(p) as *const f32) }
}
#[cfg(test)]
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(tr(p) as *mut u32) = v }
}
#[cfg(test)]
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(tr(p) as *mut u8) = v }
}
#[cfg(test)]
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(tr(p) as *mut f32) = v }
}
/// Every call into the rest of the game goes through a recording stub in the test.
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(difftest::redirect(f), eax, ecx, edx, esi, edi, args) }
}
/// An absolute game address passed to a callee as a value (the test runs the original on a
/// shifted copy of the game's globals).
#[cfg(test)]
fn ga(p: usize) -> u32 {
    tr(p) as u32
}
#[cfg(not(test))]
fn ga(p: usize) -> u32 {
    p as u32
}
/// The address a call to the game function `f` really goes to.
#[cfg(test)]
fn fa(f: usize) -> usize {
    difftest::redirect(f)
}
#[cfg(not(test))]
fn fa(f: usize) -> usize {
    f
}

/// A double stored in the exe's data (two dwords).
unsafe fn rd64(p: usize) -> f64 {
    f64::from_bits(rd(p) as u64 | (rd(p + 4) as u64) << 32)
}

/// A callee that takes one f32 on the stack (cdecl) and returns on ST0: its result as f64.
#[inline(never)]
unsafe fn call_st0_arg(f: usize, arg: f32) -> f64 {
    let mut out = 0f64;
    std::arch::asm!(
        "push edi",
        "push {a}",
        "call {f}",
        "add esp, 4",
        "pop edi",
        "fstp qword ptr [edi]",
        a = in(reg) arg.to_bits(),
        f = in(reg) fa(f),
        in("edi") &mut out as *mut f64,
        clobber_abi("C"),
    );
    out
}

/// `_ftol2_sse` (0xDA0140): the double goes in on ST0, the truncated int comes back in EAX.
#[inline(never)]
unsafe fn call_ftol(v: f64) -> u32 {
    let r: u32;
    std::arch::asm!(
        "fld qword ptr [{p}]",
        "call {f}",
        p = in(reg) &v as *const f64,
        f = in(reg) fa(0x00DA_0140),
        out("eax") r,
        clobber_abi("C"),
    );
    r
}

/// Timestamp wrap used inline by the game: `t = now (+ 0x94B62E00 when above 1,800,000,000)`.
fn wrap_now(now: u32) -> u32 {
    if (now as i32) > 0x6B49_D200 { now.wrapping_add(0x94B6_2E00) } else { now }
}

macro_rules! hits {
    ($hits:ident, $names:ident, $f:ident, [$($n:expr),* $(,)?]) => {
        static $hits: [AtomicU32; { [$($n),*].len() }] = [const { AtomicU32::new(0) }; { [$($n),*].len() }];
        const $names: [&str; { [$($n),*].len() }] = [$($n),*];
        #[inline(always)]
        fn $f(i: usize) {
            $hits[i].fetch_add(1, Ordering::Relaxed);
        }
    };
}

fn fmt_hits(name: &str, hits: &[AtomicU32], names: &[&str]) -> Option<String> {
    let v: Vec<String> = (0..hits.len())
        .filter_map(|i| {
            let n = hits[i].load(Ordering::Relaxed);
            (n != 0).then(|| format!("{}={n}", names[i]))
        })
        .collect();
    (!v.is_empty()).then(|| format!("{name} ({})", v.join(" ")))
}

// =================================================================================================
// 0x4AAC40  chunk_effects_update  (1205 bytes, no arguments)
// =================================================================================================
//
// sim_frame:5. For each loaded chunk ([0xF71B58] entries at 0xF70BA8) and each of its placed
// effect records (0x50 bytes at [chunk+0x184], count [chunk+0x180]): the record's own position
// (+0x10) and matrix (+0x1C) are replaced by the attached definition's (def pointer at
// [chunk+0x4C] + index [rec+0x48] * 0x50 + 0xC: 0xBE2F50 matrix product, 0xBDDE20 point
// transform, and the live effect object [rec+0] - type 6 - is moved by 0x50AA60 and its own
// position / matrix words). Kind [rec+8] == -1 skips the record. A type-1 record (+0xC) is
// wanted when the definition allows it (flags +0x30 & 0x400000E0 clear, [[def+0x130]+0x64]),
// the time of day [0x140E734] lies in the window [rec+0x40, rec+0x44] (a window narrower than
// 1e-4 is always open) and the camera distance ratio (kind table 0x26B5DE0, 0x30 bytes each)
// is below 1.44; it is created through 0x4171C0 + 0x50E330 unless its object already lists
// the record. Otherwise a started effect (+0x4C bit 0) is stopped through 0x50AB60. The 0x6C
// byte spawn record lives in the original's stack frame and keeps its contents from one record
// to the next; here it is one buffer for the whole call. 0x4171C0 and the rest of the record's
// fields are the same either way.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(CE_HITS, CE_NAMES, ce, [
    "calls", "chunks", "records", "def", "obj_update", "kind_none", "other_type", "type1", "window",
    "win_out", "no_def_ok", "dist", "dist_far", "spawn", "spawn_dup", "spawn_failed", "stop", "stop_none",
]);

/// The x87 `fcom` status high byte (C0 = 1, C2 = 4, C3 = 0x40) of `a` against `b`.
fn fcom(a: f64, b: f64) -> u8 {
    if a.is_nan() || b.is_nan() {
        0x45
    } else if a > b {
        0
    } else if a < b {
        1
    } else {
        0x40
    }
}

/// Parity flag after a `test`: set when the result byte has an even number of one bits.
fn pf(v: u8) -> bool {
    v.count_ones() % 2 == 0
}

/// The effect object a handle names when it is live and of type 6, else 0.
unsafe fn effect_object(h: u32) -> usize {
    if h == 0 {
        return 0;
    }
    let o = rd(0x0214_9C64 + (((h & 0xFFF) as usize) << 4)) as usize;
    if o != 0 && rd(o + 0x44) == h && rd(o + 0x4C) == 6 { o } else { 0 }
}

/// 0x4AB05C / 0x4AB07C: stops the started effect of a record that is no longer wanted.
unsafe fn effect_release(rec: usize) {
    let h = rd(rec);
    if h == 0 || rb(rec + 0x4C) & 1 == 0 {
        ce(17);
        return;
    }
    let o = effect_object(h);
    if o != 0 {
        ce(16);
        call_regs(0x0050_AB60, 0, 0, 0, 0, o as u32, &[0, rec as u32]);
    }
    wb(rec + 0x4C, rb(rec + 0x4C) & 0xFE);
}

kxs0!(chunk_effects_thunk, chunk_effects_update);

extern "C" fn chunk_effects_update() {
    unsafe {
        ce(0);
        let mut spawn = [0u32; 27];
        if (rd(0x00F7_1B58) as i32) <= 0 {
            return;
        }
        let mut i = 0usize;
        loop {
            let chunk = rd(0x00F7_0BA8 + i * 4) as usize;
            ce(1);
            let mut j = 0u32;
            if (rd(chunk + 0x180) as i32) > 0 {
                loop {
                    let rec = j.wrapping_mul(0x50).wrapping_add(rd(chunk + 0x184)) as usize;
                    chunk_record(chunk, rec, &mut spawn);
                    j = j.wrapping_add(1);
                    if !((j as i32) < rd(chunk + 0x180) as i32) {
                        break;
                    }
                }
            }
            i += 1;
            if !((i as i32) < rd(0x00F7_1B58) as i32) {
                break;
            }
        }
    }
}

unsafe fn chunk_record(chunk: usize, ebp: usize, sp: &mut [u32; 27]) {
    ce(2);
    let mut pos = [ldst(rd(ebp + 0x10)), ldst(rd(ebp + 0x14)), ldst(rd(ebp + 0x18))];
    let mut mat = [0u32; 9];
    for (k, m) in mat.iter_mut().enumerate() {
        *m = ldst(rd(ebp + 0x1C + k * 4));
    }
    let slot = rd(ebp + 0x48) as i32;
    if slot >= 0 {
        let edi = rd((slot as u32).wrapping_mul(0x50).wrapping_add(rd(chunk + 0x4C)) as usize + 0xC) as usize;
        let esi = effect_object(rd(ebp));
        if edi != 0 {
            ce(3);
            let mut m2 = [0u32; 9];
            let mut tp = pos;
            call_regs(0x00BE_2F50, m2.as_mut_ptr() as u32, (edi + 0x20) as u32, 0, 0, 0, &[(ebp + 0x1C) as u32]);
            let mut vo = [0u32; 3];
            let e = call_regs(0x00BD_DE20, vo.as_mut_ptr() as u32, (edi + 0x20) as u32, 0, 0, 0, &[tp.as_mut_ptr() as u32]) as usize;
            let x = ((rf(edi + 0x14) as f64) + rf(e) as f64) as f32;
            let y = ((rf(e + 4) as f64) + rf(edi + 0x18) as f64) as f32;
            let z = ((rf(e + 8) as f64) + rf(edi + 0x1C) as f64) as f32;
            tp = [x.to_bits(), y.to_bits(), z.to_bits()];
            if esi != 0 {
                ce(4);
                call_regs(0x0050_AA60, 0, ebp as u32, tp.as_ptr() as u32, 0, 0, &[esi as u32, m2.as_ptr() as u32]);
                wd(esi + 0x14, tp[0]);
                wd(esi + 0x18, tp[1]);
                wd(esi + 0x1C, tp[2]);
                for (k, &m) in m2.iter().enumerate() {
                    wd(esi + 0x20 + k * 4, m);
                }
            }
            pos = tp;
            mat = m2;
        }
    }
    let kind = rd(ebp + 8) as i32;
    if kind == -1 {
        ce(5);
        return;
    }
    if rb(ebp + 0xC) != 1 {
        ce(6);
        return effect_release(ebp);
    }
    ce(7);
    // time of day as a fraction of 2^32
    let t = rd(0x0140_E734) as i32;
    let mut v = t as f64;
    if t < 0 {
        v += f32::from_bits(rd(0x00E5_BFD8)) as f64;
    }
    let tod = (v * rd64(0x00E5_C030)) as f32;
    let mut wanted = true;
    if slot >= 0 {
        let e = (slot as u32).wrapping_mul(0x50).wrapping_add(rd(chunk + 0x4C)) as usize;
        let edi = rd(e + 0xC) as usize;
        if rd(e + 0x30) & 0x4000_00E0 != 0 {
            wanted = false;
        }
        if edi == 0 {
            wanted = false;
        } else {
            let w = rd(rd(edi + 0x130) as usize + 0x64);
            if w & 0x2000_0000 != 0 && w & 1 == 0 {
                wanted = false;
            }
        }
    } else {
        ce(10);
    }
    // the time-of-day window; a window narrower than 1e-4 is always open
    let a = rf(ebp + 0x40) as f64;
    let b = rf(ebp + 0x44) as f64;
    let w = (a - b) as f32 as f64;
    let c = rd64(0x00E5_C1A0);
    let mut check = true;
    if fcom(c, w) & 0x41 == 0 && !pf(fcom(-w, c) & 5) {
        check = false;
    }
    if check {
        ce(8);
        let f1 = fcom(a, b);
        let f2 = fcom(a, tod as f64);
        let f3 = fcom(b, tod as f64);
        let outside = if pf(f1 & 5) {
            f2 & 0x41 == 0 && !pf(f3 & 0x41)
        } else {
            f2 & 0x41 == 0 || !pf(f3 & 0x41)
        };
        if outside {
            ce(9);
            wanted = false;
        }
    }
    // the camera distance check (only when the record has no slot or already has an effect)
    if slot == -1 || rd(ebp) != 0 {
        ce(11);
        let sq = |d: f32| (d as f64 * d as f64) as f32;
        let d0 = (rf(0x025F_5B14) as f64 - f32::from_bits(pos[0]) as f64) as f32;
        let mut acc = sq(d0);
        let d1 = (rf(0x025F_5B18) as f64 - f32::from_bits(pos[1]) as f64) as f32;
        acc = ((d1 as f64 * d1 as f64) + acc as f64) as f32;
        let d2 = (rf(0x025F_5B1C) as f64 - f32::from_bits(pos[2]) as f64) as f32;
        let dist2 = ((d2 as f64 * d2 as f64) + acc as f64) as f32;
        let r = if kind < 0 || kind >= rd(0x0252_7288) as i32 {
            0.0f32
        } else {
            let k = 0x026B_5DE0 + kind as usize * 0x30;
            if (rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) != 0) || rb(0x0252_7D15) != 0 { rf(k + 0x14) } else { rf(k + 0x10) }
        };
        if fcom(dist2 as f64 / r as f64, rd64(0x00E5_D868)) & 1 == 0 {
            ce(12);
            return effect_release(ebp);
        }
    }
    if !wanted {
        return effect_release(ebp);
    }
    // already listed by its object?
    let h = rd(ebp);
    let o = effect_object(h);
    if o != 0 {
        let head = rd(o + 0xC4) as usize;
        if head != 0 {
            let mut n = head;
            loop {
                if rd(n + 0xC) as usize == ebp {
                    ce(14);
                    return;
                }
                n = rd(n) as usize;
                if n == head || n == 0 {
                    break;
                }
            }
        }
    }
    ce(13);
    let sp0 = sp.as_mut_ptr() as usize;
    call_regs(0x0041_71C0, 0, mat.as_ptr() as u32, pos.as_ptr() as u32, kind as u32, 0, &[sp0 as u32]);
    wb(sp0 + 0x34, 1);
    let flag = if kind < 0 || kind >= rd(0x0252_7288) as i32 {
        0
    } else {
        (rb(0x026B_5DE0 + kind as usize * 0x30 + 0x2E) >> 4) & 1
    };
    wb(sp0 + 0x40, flag);
    let id = rd(chunk + 8) as u16;
    wb(sp0 + 0x48, id as u8);
    wb(sp0 + 0x49, (id >> 8) as u8);
    wd(sp0 + 0x64, ebp as u32);
    let nh = call_regs(0x0050_E330, 0, 0, 0, 0, 0, &[sp0 as u32]);
    wd(ebp, nh);
    if nh == 0 {
        ce(15);
        return;
    }
    wb(ebp + 0x4C, rb(ebp + 0x4C) | 1);
}

// =================================================================================================
// 0x4FB160  player_controls_do_frame  (527 bytes, no arguments, ends in a tail jump)
// =================================================================================================
//
// sim_frame:1,2,3 (input context 2 update). Clears [0x2527267]. While [0x2527C08] > 0 or
// [0x2528989] is set: a cutscene block (0x2527D14 and 0x2527CF7, or 0x2527D15) lets action 0x59
// (input_action_take_pressed) call 0x6C43F0, and with [0x2528989] the camera pair 0x25F5CB0 /
// 0x25F5CB8 is set to ([0xE87998], 1.0). Otherwise the local player's control block (+0x139C ..
// +0x13AC) is cleared, the context's callback (table 0xDD4CB4, 16 bytes per context [0xE8E780],
// +0) runs with the player, a dead player (0x985630) gets the vehicle handling (0x9DA290,
// 0xAE28F0, 0xAB4960, 0xAB4720), and a flag pair (stack byte [esp+0x13], bl) is cleared in the
// co-op / cutscene cases (action 0x59 -> 0x6C43F0; action 0x2F into byte 0xF8480F bit 3) or
// left to 0x4FAA90 (it may clear the stack byte); then 0x4FAE90, the callback at +8 when bl,
// else the camera pair, 0x493260 when [0x2527C08] <= 0, and finally the callback at +4 is
// tail-called when the stack byte is still set.
// Quirks: a null player makes the original write to low memory; here it returns. The tail jump is
// a call followed by the return (the registers left over are not read by the callbacks).
// Check: per-branch counters plus the offline difftest.

hits!(PC_HITS, PC_NAMES, pc, [
    "calls", "blocked", "blocked_skip", "dead", "dead_zoom", "dead_vehicle", "ctx_none", "coop", "coop_skip",
    "co_flag", "dispatch", "cb8", "camera_pair", "pre_update", "tail", "no_tail",
]);

kxs0!(player_controls_thunk, player_controls_do_frame);

extern "C" fn player_controls_do_frame() {
    unsafe {
        pc(0);
        wb(0x0252_7267, 0);
        let camera_pair = || {
            wd(0x025F_5CB0, ldst(rd(0x00E8_7998)));
            wd(0x025F_5CB8, 1.0f32.to_bits());
        };
        if (rd(0x0252_7C08) as i32) > 0 || rb(0x0252_8989) != 0 {
            pc(1);
            if (rb(0x0252_7D14) != 0 && rb(0x0252_7CF7) != 0) || rb(0x0252_7D15) != 0 {
                pc(2);
                if call_regs(0x004F_FAF0, 0x59, 0, 0, 0, 0, &[]) as u8 != 0 {
                    call_regs(0x006C_43F0, 0, 0, 0, 0, 0, &[]);
                }
            }
            if rb(0x0252_8989) != 0 {
                camera_pair();
            }
            return;
        }
        let player = rd(0x0217_03D4) as usize;
        if player == 0 {
            return;
        }
        let ebp = player + 0x139C;
        let entry = |ctx: u32, off: usize| rd(((ctx as usize) << 4).wrapping_add(0x00DD_4CB4 + off) as usize);
        let ctx0 = rd(0x00E8_E780);
        wd(ebp, 0);
        for o in [0x13A0usize, 0x13A4, 0x13A8, 0x13AC] {
            wd(player + o, 0);
        }
        let cb = entry(ctx0, 0) as usize;
        let mut flag_a = 1u8;
        let mut bl = true;
        if cb != 0 {
            pc(6);
            call_regs(cb, 0, 0, 0, 0, 0, &[player as u32]);
        }
        if call_regs(0x0098_5630, 0, 0, 0, 0, 0, &[player as u32]) as u8 != 0 {
            pc(3);
            if rb(player + 0x1161) & 1 != 0 {
                pc(4);
                call_regs(0x009D_A290, 0, player as u32, 0, 0, 0, &[0]);
            }
            let h = rd(player + 0xD74);
            if h != 0 {
                let v = call_regs(0x00AE_28F0, 0, h, 0, 0, 0, &[]);
                if v != 0 && call_regs(0x00AB_4960, 0, v, 0, 0, 0, &[]) as u8 != 0 {
                    pc(5);
                    call_regs(0x00AB_4720, v, 0, 0, 0, 0, &[0, 0]);
                }
            }
        }
        if rd(0x025F_5AE8) == 2 {
            pc(7);
            flag_a = 0;
            bl = false;
        } else if call_regs(0x006C_6D20, 0, 0, 0, 0, 0, &[]) as u8 != 0 {
            pc(8);
            if call_regs(0x004F_FAF0, 0x59, 0, 0, 0, 0, &[]) as u8 != 0 {
                call_regs(0x006C_43F0, 0, 0, 0, 0, 0, &[]);
            }
            flag_a = 0;
            bl = false;
        } else if rb(player + 0x18E8) & 2 != 0 {
            if rb(ebp) & 1 != 0 {
                pc(9);
                let mut al = call_regs(0x004F_FAF0, 0x2F, 0, 0, 0, 0, &[]) as u8;
                let cl = rb(0x00F8_480F);
                al = al.wrapping_add(al);
                al = al.wrapping_add(al);
                al = al.wrapping_add(al);
                al = (al ^ cl) & 8;
                wb(0x00F8_480F, cl ^ al);
            }
            flag_a = 0;
            bl = false;
        } else if rb(player + 0x1163) & 0x20 == 0 {
            pc(10);
            call_regs(0x004F_AA90, 0, 0, 0, 0, 0, &[&mut flag_a as *mut u8 as u32]);
        }
        call_regs(0x004F_AE90, 0, 0, 0, 0, 0, &[]);
        let mut ctx = rd(0x00E8_E780);
        if ctx == 0xFFFF_FFFF {
            return;
        }
        let cb8 = if bl { entry(ctx, 8) as usize } else { 0 };
        if cb8 != 0 {
            pc(11);
            call_regs(cb8, 0, 0, 0, 0, 0, &[]);
            ctx = rd(0x00E8_E780);
        } else {
            pc(12);
            camera_pair();
        }
        if (rd(0x0252_7C08) as i32) <= 0 {
            pc(13);
            call_regs(0x0049_3260, 0, 0, 0, 0, 0, &[]);
            ctx = rd(0x00E8_E780);
        }
        if flag_a == 0 {
            pc(15);
            return;
        }
        let tail = entry(ctx, 4) as usize;
        if tail == 0 {
            pc(15);
            return;
        }
        pc(14);
        call_regs(tail, tail as u32, 0, 0, 0, 0, &[]);
    }
}

// =================================================================================================
// 0x489CC0  vehicle_siren_update  (1333 bytes, no arguments)
// =================================================================================================
//
// sim_frame:5. The siren of the emergency vehicle whose handle is [0x2526B74]: the sound handle
// [0xE9A4A0] (low byte slot of the 0x19C-byte voices at 0xEEB8F8, upper 24 bits the generation
// at +0xB4) must be live, else nothing happens. A vehicle that is gone (handle stale or not type
// 5) fades the voice out over 150 ms (0x47C740), clears the handle and draws the restart delay
// 0xBDCF50(min [0xEE3478], max [0xEE347A]) seconds into [0xE9A4B0]. For a live vehicle the
// 5 s timer [0xE9A4AC] (or bit 0 of [veh+0x8F50]) ends the siren the same way (also lights
// off, voice fades, then the cue [0xE99B94] restarted at volume [0xE99B98] through the 0x464970 /
// 0x464CA0 params builders and 0x465B90). Otherwise the voice follows the vehicle (0x47FFC0,
// 0x465FD0, 0x4660B0 / 0x4802D0) and its volume ramps by dt * 0.5 towards a target through
// 0x466010; the lights (five objects at +0x6DA4 / +0x6DD0 / +0x6E58 / +0x6DF8 / +0x6E28, vtable
// slot 6, thiscall (flag, 1.0f), callee pops) flash every second of the timer.
// The flag dword of the final light loop is the byte of `flash` merged into whatever the stack
// slot [esp+0x14] held: when the volume target was stored there (as a float) its upper three
// bytes are passed on; otherwise they are stale stack garbage and zero here.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(VS_HITS, VS_NAMES, vs, [
    "calls", "no_voice", "vanished", "elapsed", "flag_stop", "follow", "fol_bl_2d", "fol_bl_3d", "fol_nobl_far",
    "fol_nobl_near", "vel_lost", "pos_lost", "t1_extend", "flash_on", "flash_off", "lights_loop", "stop_lights",
    "stop_fade2", "stop_3d", "stop_2d", "stop_flagged", "ramp_clamped",
]);

/// `fld dword; fstp dword`: a signalling NaN comes out quiet.
fn ldst(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

unsafe fn rw(p: usize) -> u32 {
    rd(p) & 0xFFFF
}

unsafe fn ww(p: usize, v: u32) {
    wb(p, v as u8);
    wb(p + 1, (v >> 8) as u8);
}

/// 0xE9A4B0 = now + `ms` seconds, wrapped (the signed sign tests of the original).
unsafe fn siren_deadline(ms: i32) {
    const TS: usize = 0x00E9_A4B0;
    let t = (rd(0x00E8_43AC) as i32).wrapping_add(ms);
    wd(TS, t as u32);
    if ms < 0 {
        if t < 0 {
            wd(TS, t.wrapping_add(0x6B49_D200) as u32);
        }
    } else if t > 0x6B49_D200 {
        wd(TS, t.wrapping_sub(0x6B49_D200) as u32);
    }
}

/// The five vehicle light objects, called through vtable slot 6 with `(flag, 1.0f)`.
unsafe fn siren_lights(veh: usize, flag: u32) {
    for off in [0x6DA4usize, 0x6DD0, 0x6E58, 0x6DF8, 0x6E28] {
        let obj = veh + off;
        let f = rd(rd(obj) as usize + 0x18) as usize;
        call_regs(f, 0, obj as u32, 0, 0, 0, &[flag, 1.0f32.to_bits()]);
    }
}

/// The restart delay: random 0xEE3478..0xEE347A seconds.
unsafe fn siren_restart_delay() {
    let max = rw(0x00EE_347A);
    let min = rw(0x00EE_3478);
    wd(0x00E9_A4A0, 0xFF);
    let r = call_regs(0x00BD_CF50, 0, max, 0, min, 0, &[]) as i32;
    siren_deadline(r.wrapping_mul(1000));
}

kxs0!(vehicle_siren_thunk, vehicle_siren_update);

extern "C" fn vehicle_siren_update() {
    unsafe {
        vs(0);
        const H: usize = 0x00E9_A4A0;
        const T1: usize = 0x00E9_A4AC;
        const NOW: usize = 0x00E8_43AC;
        const VOL: usize = 0x00E9_9B98;
        let h = rd(H);
        let vgen = h >> 8;
        if h as u8 == 0xFF || vgen == 0 {
            vs(1);
            return;
        }
        let slot = (h & 0xFF) as usize;
        let voice = slot * 0x19C;
        if vgen != rd(0x00EE_B9AC + voice) || rd(0x00EE_B8F8 + voice) == 0xFFFF_FFFF {
            vs(1);
            return;
        }
        let v = rd(0x0252_6B74);
        let mut ebp = 0usize;
        if v != 0 {
            let e = rd(0x0214_9C64 + (((v & 0xFFF) as usize) << 4)) as usize;
            if e != 0 && rd(e + 0x44) == v && rd(e + 0x4C) == 5 {
                ebp = e;
            }
        }
        if ebp == 0 {
            vs(2);
            call_regs(0x0047_C740, slot as u32, 0, 0x96, 0, 0, &[]);
            siren_restart_delay();
            return;
        }
        let bl = call_regs(0x009B_8E50, 0, 0, 0, 0, rd(0x0217_03D4), &[ebp as u32]) as u8;
        // the timer test: stop when [0xE9A4AC] has been reached (with the wrap rule) or bit 0 of
        // the vehicle flags is set
        let t1 = rd(T1) as i32;
        let now = rd(NOW) as i32;
        let mut stop = false;
        if t1 >= 0 {
            if t1 > now {
                if t1.wrapping_sub(now) >= 0x35A4_E900 {
                    stop = true;
                    vs(3);
                }
            } else if now.wrapping_sub(t1) < 0x35A4_E900 {
                stop = true;
                vs(3);
            }
        }
        if !stop && rb(ebp + 0x8F50) & 1 != 0 {
            stop = true;
            vs(4);
        }
        if stop {
            siren_stop(ebp, bl);
            return;
        }
        vs(5);
        let edi = ebp + 0x14;
        if call_regs(0x0047_FFC0, 0, edi as u32, ga(H), 0, 0, &[]) as u8 == 0 {
            vs(11);
            wd(H, 0xFF);
        }
        call_regs(0x0046_5FD0, 0, ga(H), 0, 0, 0, &[(ebp + 0x20) as u32]);
        let al = call_regs(0x0046_60B0, 0, ga(H), 0, 0, 0, &[]) as u8;
        // the volume target (stored in [esp+0x14] as a float) or none
        let mut slot14: Option<u32> = None;
        let mut ramp: Option<u32> = None;
        if bl != 0 {
            if al != 0 {
                vs(6);
                ramp = Some(ldst(rd(0x00EE_3480)));
            } else {
                vs(7);
                let rem = call_regs(0x00BD_D260, ga(T1), 0, 0, 0, 0, &[]) as i32;
                if rem > 0x1388 {
                    vs(12);
                    let t = now.wrapping_add(0x1388);
                    wd(T1, t as u32);
                    if t > 0x6B49_D200 {
                        wd(T1, t.wrapping_sub(0x6B49_D200) as u32);
                    }
                }
                let b = ldst(rd(0x00EE_347C));
                wd(VOL, b);
                call_regs(0x0046_60F0, 0, 0, 0, ga(H), 0, &[ldst(rd(VOL))]);
            }
        } else if al == 0 {
            vs(8);
            ramp = Some(1.0f32.to_bits());
        } else {
            vs(9);
            let mut vec = [0f32; 3];
            for (i, x) in vec.iter_mut().enumerate() {
                *x = (rf(edi + i * 4) as f64 - rf(ebp + 0x50 + i * 4) as f64) as f32;
            }
            let ok = call_regs(0x0048_02D0, 0, ga(H), 0, 0, vec.as_mut_ptr() as u32, &[edi as u32, (ebp + 0x20) as u32]) as u8;
            if ok == 0 {
                vs(10);
                wd(H, 0xFF);
            }
        }
        if let Some(target) = ramp {
            slot14 = Some(target);
            let nv = (rf(0x00E8_437C) as f64 * rd64(0x00E5_BF70) + rf(VOL) as f64) as f32;
            wf(VOL, nv);
            if !(f32::from_bits(target) as f64 > nv as f64) {
                vs(21);
                wd(VOL, target);
            }
            call_regs(0x0046_6010, 0, 0, ga(H), 0, 0, &[ldst(rd(VOL))]);
        }
        // the flash flag: bit 0 of (remaining ms / 1000)
        let rem = call_regs(0x00BD_D260, ga(T1), 0, 0, 0, 0, &[]) as i32;
        let hi = ((rem as i64 * 0x1062_4DD3) >> 32) as i32;
        let q = hi >> 6;
        let q = q.wrapping_add(((q as u32) >> 31) as i32);
        let flash = (q & 1 == 1) as u32;
        if rw(ebp + 0x8D88) & 0x800 != 0 {
            if flash != 0 {
                vs(13);
                call_regs(0x0048_9190, rd(ebp + 0x44), 0, 0, 0, 0, &[]);
            } else {
                vs(14);
                call_regs(0x0048_92E0, 0, rd(ebp + 0x44), 0, 0, 0, &[]);
                call_regs(0x0046_5F70, 0, (ebp + 0x8D14) as u32, 0, 0, 0, &[0x7D]);
            }
        }
        if rb(ebp + 0x84D8) == 1 {
            vs(15);
            let flag = match slot14 {
                Some(f) => (f & 0xFFFF_FF00) | flash,
                None => flash,
            };
            siren_lights(ebp, flag);
        }
    }
}

/// 0x489DEE: the siren of a live vehicle ends (timer reached or vehicle flag): lights off, voices
/// faded, restart delay drawn and the cue restarted.
unsafe fn siren_stop(ebp: usize, bl: u8) {
    const H: usize = 0x00E9_A4A0;
    if rb(ebp + 0x84D8) == 1 {
        vs(16);
        siren_lights(ebp, 0);
    }
    let w = rw(ebp + 0x8D88);
    if w & 0x800 != 0 {
        vs(17);
        ww(ebp + 0x8D88, w & 0xF7FF);
        call_regs(0x0046_5F70, 0, (ebp + 0x8D14) as u32, 0, 0, 0, &[0x7D]);
    }
    call_regs(0x0046_5F70, 0, ga(H), 0, 0, 0, &[0x96]);
    siren_restart_delay();
    let flags = rd(ebp + 0x8F50);
    if flags & 1 != 0 {
        vs(20);
        return;
    }
    let vol = ldst(rd(0x00E9_9B98));
    let cue = rw(0x00E9_9B94);
    let mut params = [0u32; 23];
    let mut handle = [0u32; 3];
    let pw = params.as_mut_ptr() as usize;
    if bl != 0 {
        vs(19);
        call_regs(0x0046_4970, 0, 0, 0, pw as u32, 0, &[cue, 0, ebp as u32, vol]);
        let edx = rd(pw + 0x58);
        let mut cx = (((flags >> 13) & 1) as u16) << 5;
        cx |= (edx & 0xFFDF) as u16;
        cx |= 0x10;
        ww(pw + 0x58, cx as u32);
    } else {
        vs(18);
        call_regs(0x0046_4CA0, 0, 0, 0, pw as u32, (ebp + 0x14) as u32, &[cue, 0, ebp as u32, vol]);
    }
    call_regs(0x0046_5B90, pw as u32, handle.as_mut_ptr() as u32, 0, 0, 0, &[]);
}

// =================================================================================================
// 0xB6AD40  weapon_target_lock_update  (1412 bytes, no arguments)
// =================================================================================================
//
// sim_frame:6,7. The local player's lock-on to a vehicle. Lock state [0x252A130]: 0 idle, 1
// locking (beeping), 2 locked; the target is the handle [0x252A12C]. The weapon info (EBX) is
// [[[player+0x10CC]+0xC4]+8]; it must have bit 0x400 of +0xC. The angle between the player's view
// vector (0x9C5FD0) and the normalised direction to the target (0xBDE150) is arccos(dot)
// (0x42EF30; 0 outside the open interval (C, 1) with C = [0xE87998]). State 2 drops the lock when
// the angle exceeds [weapon+0xC4] * K1 / K2 (the lock then goes back to state 1 unless the
// current candidate 0xB6A740 is still the target), state 1 compares against [weapon+0xC0] and
// beeps (cue hashes of the strings at 0xE497EC / 0xE49800 through 0xBDCB30 / 0x479A50 / 0x465CD0)
// by the timers at 0xEBD054 / 0xEBD058, and moves to state 2 when the 0xEBD054 timer has run out.
// 0xB6A8B0 drives the reticle, 0xB6B460 clears the lock.
// Check: per-branch counters (the callees have side effects) plus the offline difftest.

hits!(WT_HITS, WT_NAMES, wt, [
    "calls", "idle", "s0_new", "s0_none", "s1", "s1_reset", "s1_clear", "s1_beep", "s1_not_inside",
    "s1_cand_none", "s1_cand_other", "s1_expired", "s1_locked", "s1_wait", "s2", "s2_lost", "s2_clear",
    "s2_drop", "s2_stay",
]);

/// `[[[player+0x10CC]+0xC4]+8]` (zero when any link is missing), re-read from `p = player+0x10CC`.
unsafe fn target_weapon(p: usize) -> u32 {
    let o = rd(p) as usize;
    if o == 0 {
        return 0;
    }
    let w = rd(o + 0xC4) as usize;
    if w == 0 { 0 } else { rd(w + 8) }
}

/// `0xB6A8B0(ECX = vehicle, EDX = weapon info, f32)`.
unsafe fn reticle(veh: u32, w: u32, f: f32) {
    call_regs(0x00B6_A8B0, 0, veh, w, 0, 0, &[f.to_bits()]);
}

/// The angle (radians, f32) between the view vector `view` and the direction from the camera
/// position 0x25F5B20 to the vehicle at `veh + 0x14`.
unsafe fn target_angle(veh: usize, view: &[f32; 3]) -> f32 {
    let mut d = [0f32; 3];
    for i in 0..3 {
        d[i] = (rf(veh + 0x14 + i * 4) as f64 - rf(0x025F_5B20 + i * 4) as f64) as f32;
    }
    call_regs(0x00BD_E150, 0, 0, 0, d.as_mut_ptr() as u32, 0, &[]);
    let dot = (d[1] as f64 * view[1] as f64 + d[0] as f64 * view[0] as f64) + d[2] as f64 * view[2] as f64;
    let dot = dot as f32;
    let c = rf(0x00E8_7998) as f64;
    let dv = dot as f64;
    if !(dv > c) || 1.0 <= dv {
        0.0
    } else {
        call_st0_arg(0x0042_EF30, dot) as f32
    }
}

/// Plays the cue named by the string at `s` into `buf` (hash, foley lookup, 2D play).
unsafe fn play_cue(s: usize, buf: &mut [u32; 3]) {
    let mut hash = 0u32;
    let out = call_regs(0x00BD_CB30, 0, ga(s), 0, 0, 0, &[&mut hash as *mut u32 as u32]);
    let cue = call_regs(0x0047_9A50, 0, 0, out, 0, 0, &[]) & 0xFFFF;
    call_regs(0x0046_5CD0, 0, 0, 0, buf.as_mut_ptr() as u32, 0, &[cue, 0]);
}

/// 0xB6B29C: nothing to lock; a running lock is reset.
unsafe fn lock_idle() {
    if rd(0x0252_A130) != 0 {
        wt(1);
        lock_reset();
    }
}

/// 0xB6B2A4: reticle off, state 0.
unsafe fn lock_reset() {
    reticle(0, 0, 0.0);
    wd(0x0252_A130, 0);
}

kxs0!(weapon_target_lock_thunk, weapon_target_lock_update);

extern "C" fn weapon_target_lock_update() {
    unsafe {
        wt(0);
        const PLAYER: usize = 0x0217_03D4;
        const STATE: usize = 0x0252_A130;
        const HANDLE: usize = 0x0252_A12C;
        const TS: usize = 0x00EB_D054;
        const TS2: usize = 0x00EB_D058;
        const VOICE: usize = 0x00EB_D05C;
        const NOW: usize = 0x00E8_43AC;
        let edi = rd(PLAYER);
        if rb(0x00E8_6E8B) == 0 || edi == 0 {
            return lock_idle();
        }
        let p = edi as usize + 0x10CC;
        let ebx = target_weapon(p) as usize;
        let mut view = [0f32; 3];
        call_regs(0x009C_5FD0, edi, 0, 0, 0, 0, &[view.as_mut_ptr() as u32]);
        if ebx == 0 || rd(ebx + 0xC) & 0x400 == 0 {
            return lock_idle();
        }
        let k1 = rd64(0x00E5_C160);
        let k2 = rd64(0x00E5_C348);
        let k3 = rd64(0x00E5_C0D8);
        match rd(STATE) {
            0 => {
                let cur = call_regs(0x00B6_A740, 0, 0, 0, 0, 0, &[]) as usize;
                if cur == 0 {
                    wt(3);
                    return;
                }
                wt(2);
                wd(HANDLE, rd(cur + 0x44));
                call_regs(0x00BD_D100, rd(ebx + 0xAC), ga(TS), 0, 0, 0, &[]);
                wd(STATE, 1);
                wd(TS2, 0xFFFF_FFFF);
            }
            1 => {
                wt(4);
                let ebp = call_regs(0x00AE_28F0, 0, rd(HANDLE), 0, 0, 0, &[]);
                if ebp == 0 || call_regs(0x00AA_B370, 0, 0, ebp, 0, 0, &[]) as u8 != 0 {
                    wt(5);
                    return lock_reset();
                }
                if call_regs(0x009B_8E50, 0, 0, 0, 0, edi, &[ebp]) as u8 != 0 {
                    wt(6);
                    call_regs(0x00B6_B460, 0, 0, 0, 0, 0, &[]);
                    return;
                }
                let angle = target_angle(ebp as usize, &view);
                let lim = (rf(ebx + 0xC0) as f64 * k1 / k2) as f32;
                let inside = lim as f64 > angle as f64;
                let ac = rd(ebx + 0xAC) as i32;
                let frac = if ac != 0 {
                    let rem = call_regs(0x00BD_D260, ga(TS), 0, 0, 0, 0, &[]) as i32;
                    (rem as f64 / ac as f64) as f32
                } else {
                    0.0
                };
                let v24 = ((rf(ebx + 0xB8) as f64 - rf(ebx + 0xBC) as f64) * frac as f64 + rf(ebx + 0xBC) as f64) as f32;
                let beep = if (rd(TS2) as i32) < 0 {
                    true
                } else {
                    let el = call_regs(0x00BD_D2A0, ga(TS2), 0, 0, 0, 0, &[]) as i32;
                    (v24 as f64 * k3) < el as f64
                };
                let mut buf = [0u32; 3];
                if beep {
                    wt(7);
                    wd(TS2, wrap_now(rd(NOW)));
                    play_cue(0x00E4_97EC, &mut buf);
                }
                if !inside {
                    wt(8);
                    let cur = call_regs(0x00B6_A740, 0, 0, 0, 0, 0, &[]);
                    if cur != ebp {
                        if cur != 0 {
                            wt(10);
                            return lock_reset();
                        }
                        wt(9);
                        let v = rf(0x00E8_437C) as f64 * k3;
                        let ms = call_ftol(v + v);
                        call_regs(0x00BD_D130, 0, 0, 0, ga(TS), 0, &[ms]);
                        let rem = call_regs(0x00BD_D260, ga(TS), 0, 0, 0, 0, &[]) as i32;
                        if rem > rd(ebx + 0xAC) as i32 {
                            wt(11);
                            return lock_reset();
                        }
                        wt(13);
                        let w = target_weapon(p);
                        reticle(ebp, w, frac);
                        return;
                    }
                }
                if call_regs(0x00BD_D230, ga(TS), 0, 0, 0, 0, &[]) as u8 == 0 {
                    wt(13);
                    let w = target_weapon(p);
                    reticle(ebp, w, frac);
                    return;
                }
                wt(12);
                play_cue(0x00E4_9800, &mut buf);
                wd(VOICE, buf[0]);
                wd(VOICE + 4, buf[1]);
                wd(VOICE + 8, buf[2]);
                let w = target_weapon(p);
                reticle(ebp, w, 0.0);
                wd(STATE, 2);
            }
            2 => {
                wt(14);
                let ebp = call_regs(0x00AE_28F0, 0, rd(HANDLE), 0, 0, 0, &[]);
                if ebp == 0 || call_regs(0x00AA_B370, 0, 0, ebp, 0, 0, &[]) as u8 != 0 {
                    return lock_lost();
                }
                if call_regs(0x009B_8E50, 0, 0, 0, 0, edi, &[ebp]) as u8 != 0 {
                    wt(16);
                    call_regs(0x00B6_B460, 0, 0, 0, 0, 0, &[]);
                    return;
                }
                let cur = call_regs(0x00B6_A740, 0, 0, 0, 0, 0, &[]);
                if cur != 0 && cur != ebp {
                    return lock_lost();
                }
                let angle = target_angle(ebp as usize, &view);
                let lim = (rf(ebx + 0xC4) as f64 * k1 / k2) as f32;
                if (lim as f64) < angle as f64 {
                    let cur = call_regs(0x00B6_A740, 0, 0, 0, 0, 0, &[]);
                    if cur != ebp {
                        wt(17);
                        wd(STATE, 1);
                        call_regs(0x0047_FEB0, 0, ga(VOICE), 0, 0, 0, &[]);
                        wd(TS, wrap_now(rd(NOW)));
                        return;
                    }
                }
                wt(18);
                let w = target_weapon(p);
                reticle(ebp, w, 0.0);
            }
            _ => {}
        }
    }
}

/// 0xB6AF4C: the target is gone, invalid or no longer the candidate (state 2).
unsafe fn lock_lost() {
    wt(15);
    lock_reset();
    call_regs(0x0047_FEB0, 0, ga(0x00EB_D05C), 0, 0, 0, &[]);
}

// =================================================================================================
// 0x7F7910  net_frame_update  (116 bytes, no arguments)
// =================================================================================================
//
// sim_frame (net_do_frame). With the network mode byte [0x2528B48] set: the connection and
// session ticks 0x7FC930, 0x7FA280, 0x817BA0, 0x8312A0, 0x80AF30, 0x82E210, 0x7F8390; with a
// session [0x2528C14] in state 1, 6 or 8 (+0x1C): 0x80FCF0(session) (stdcall) and, when it
// answers, 0x801880, then 0x4017C0; then 0x814BB0 and the import in slot 0xDB80B8 with
// [0x2528B5C] when that is set. No register inputs reach any callee.
// Check: per-branch counters plus the offline difftest.

hits!(NF_HITS, NF_NAMES, nf, ["calls", "net_off", "session_tick", "session_ok", "import"]);

kxs0!(net_frame_update_thunk, net_frame_update);

extern "C" fn net_frame_update() {
    unsafe {
        nf(0);
        if rb(0x0252_8B48) == 0 {
            nf(1);
            return;
        }
        for f in [0x007F_C930usize, 0x007F_A280, 0x0081_7BA0, 0x0083_12A0, 0x0080_AF30, 0x0082_E210, 0x007F_8390] {
            call_regs(f, 0, 0, 0, 0, 0, &[]);
        }
        let s = rd(0x0252_8C14);
        if s != 0 {
            let st = rd(s as usize + 0x1C);
            if st == 1 || st == 6 || st == 8 {
                nf(2);
                if call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[s]) as u8 != 0 {
                    nf(3);
                    call_regs(0x0080_1880, 0, 0, 0, 0, 0, &[]);
                }
                call_regs(0x0040_17C0, 0, 0, 0, 0, 0, &[]);
            }
        }
        call_regs(0x0081_4BB0, 0, 0, 0, 0, 0, &[]);
        let e = rd(0x0252_8B5C);
        if e != 0 {
            nf(4);
            call_regs(rd(0x00DB_80B8) as usize, 0, 0, 0, 0, 0, &[e]);
        }
    }
}

// =================================================================================================
// 0x53BCA0  game_mode_activity_frame  (389 bytes, no arguments, /GS cookie)
// =================================================================================================
//
// Game mode 0 callback 4 (frame), called by game_mode_do_frame through [mode*0x6C+0xE96C20].
// The activity [0x27B3C60] (+0x3C state, +0x90 a controller object when the state is 1): its
// vtable slot 0x40 runs first, 0x78FEF0(0) when [0x1F8ACB4] is set. Without a held-up handler
// [0x1408E88] a session [0x2528C14] that 0x80FCF0 does not refuse gets the vtable slot 0xF of
// the singleton 0x6A8CA0; with one, its slot 7 decides (non-zero returns); the controller of
// state 1 answering 0xE from slot 1 (while [0x140BF8E] is set) ends in 0x680130. Otherwise, when
// 0x6A33F0 agrees: the player's flag bit 3 at +0x1163 is cleared (0x9D9FD0), a running
// conversation [0x2527C90] stopped (0x754020, ESI), a state other than 0 / 3 ends the mission
// (0x6A2B40, ECX = activity, 1) and states 0 / 3 arm the timer [0xEAAF48] = now + 2000 and copy
// [0x33D6308] to 0x27B3C74 / 0x27B3C78. The /GS cookie check is not reproduced. A null controller
// (state != 1) makes the original read address 0; here that returns instead.
// Check: per-branch counters plus the offline difftest.

hits!(GM_HITS, GM_NAMES, gm, [
    "calls", "no_activity", "no_handler", "handler", "handler_busy", "diversion", "not_ready", "flag_clear", "conversation",
    "mission_end", "timer", "null_controller",
]);

/// Calls virtual method at byte offset `off` of the object `this` (thiscall, no stack args).
unsafe fn vcall(this: usize, off: usize) -> u32 {
    let vt = rd(this) as usize;
    let f = rd(vt + off) as usize;
    call_regs(f, vt as u32, this as u32, f as u32, 0, 0, &[])
}

kxs0!(game_mode_activity_thunk, game_mode_activity_frame);

extern "C" fn game_mode_activity_frame() {
    unsafe {
        gm(0);
        const ACT: usize = 0x027B_3C60;
        let act = rd(ACT) as usize;
        if act == 0 {
            gm(1);
            return;
        }
        let ctl = if rd(act + 0x3C) == 1 { rd(act + 0x90) as usize } else { 0 };
        if ctl == 0 {
            gm(11);
            return;
        }
        vcall(ctl, 0x40);
        if rb(0x01F8_ACB4) != 0 {
            call_regs(0x0078_FEF0, 0, 0, 0, 0, 0, &[0]);
        }
        let handler = rd(0x0140_8E88) as usize;
        if handler == 0 {
            gm(2);
            let s = rd(0x0252_8C14);
            if s == 0 {
                return;
            }
            if call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[s]) as u8 != 0 {
                return;
            }
            let o = call_regs(0x006A_8CA0, 0, 0, 0, 0, 0, &[]) as usize;
            if o != 0 {
                vcall(o, 0x3C);
            }
            return;
        }
        gm(3);
        if vcall(handler, 0x1C) as u8 != 0 {
            gm(4);
            return;
        }
        let mode_flag = rb(0x0140_BF8E) != 0;
        let keep = rd(0x033D_6308);
        let act = rd(ACT) as usize;
        if mode_flag && act != 0 && rd(act + 0x3C) == 1 {
            let c = rd(act + 0x90) as usize;
            if c != 0 && vcall(c, 4) == 0xE {
                gm(5);
                call_regs(0x0068_0130, 0, 0, 0, 0, 0, &[]);
                return;
            }
        }
        if call_regs(0x006A_33F0, 0, 0, 0, 0, 0, &[]) as u8 == 0 {
            gm(6);
            return;
        }
        let player = rd(0x0217_03D4) as usize;
        let fl = rb(player + 0x1163);
        if fl & 8 != 0 {
            gm(7);
            wb(player + 0x1163, fl & 0xFB);
            call_regs(0x009D_9FD0, 0, 0, 0, 0, 0, &[player as u32]);
        }
        if rd(0x0252_7C90) != 0 {
            gm(8);
            call_regs(0x0075_4020, 0, 0, 0, ga(0x0252_7C90), 0, &[]);
        }
        let act = rd(ACT) as usize;
        if act == 0 {
            gm(11);
            return;
        }
        let st = rd(act + 0x3C);
        if st != 0 && st != 3 {
            gm(9);
            call_regs(0x006A_2B40, 0, act as u32, 0, 0, 0, &[1]);
            return;
        }
        gm(10);
        wd(0x00EA_AF48, wrap_now(rd(0x00E8_43AC).wrapping_add(0x7D0)));
        wd(0x027B_3C74, keep);
        wd(0x027B_3C78, keep);
    }
}

// --- Install / summary ------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let targets: [(&str, usize, &[u8], usize); 6] = [
        ("weapon_target_lock_update", 0x00B6_AD40, &WTL_BODY, weapon_target_lock_thunk as extern "C" fn() as usize),
        ("vehicle_siren_update", 0x0048_9CC0, &VSU_BODY, vehicle_siren_thunk as extern "C" fn() as usize),
        ("chunk_effects_update", 0x004A_AC40, &CFU_BODY, chunk_effects_thunk as extern "C" fn() as usize),
        ("player_controls_do_frame", 0x004F_B160, &PCF_BODY, player_controls_thunk as extern "C" fn() as usize),
        ("game_mode_activity_frame", 0x0053_BCA0, &GMA_BODY, game_mode_activity_thunk as extern "C" fn() as usize),
        ("net_frame_update", 0x007F_7910, &NFU_BODY, net_frame_update_thunk as extern "C" fn() as usize),
    ];
    let mut ok = 0;
    for (name, addr, body, thunk) in targets {
        if replace_pinned(name, addr, body, thunk) {
            ok += 1;
        }
    }
    crate::log(&format!("port_frame3: replaced {ok}/{} with Rust", targets.len()));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = [
        fmt_hits("weapon_target_lock_update", &WT_HITS, &WT_NAMES),
        fmt_hits("vehicle_siren_update", &VS_HITS, &VS_NAMES),
        fmt_hits("chunk_effects_update", &CE_HITS, &CE_NAMES),
        fmt_hits("player_controls_do_frame", &PC_HITS, &PC_NAMES),
        fmt_hits("game_mode_activity_frame", &GM_HITS, &GM_NAMES),
        fmt_hits("net_frame_update", &NF_HITS, &NF_NAMES),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("port_frame3: {}", parts.join(", "))
}

// --- Original bodies (pins) -------------------------------------------------------------------

#[allow(dead_code)]
const CFU_BODY: [u8; 1205] = [
    0x81, 0xEC, 0xFC, 0x00, 0x00, 0x00, 0x53, 0x55, 0x33, 0xC0, 0x39, 0x05, 0x58, 0x1B, 0xF7, 0x00,
    0x56, 0x57, 0x89, 0x44, 0x24, 0x2C, 0x0F, 0x8E, 0x8E, 0x04, 0x00, 0x00, 0x8B, 0x14, 0x85, 0xA8,
    0x0B, 0xF7, 0x00, 0x33, 0xED, 0x39, 0xAA, 0x80, 0x01, 0x00, 0x00, 0x89, 0x54, 0x24, 0x18, 0x89,
    0x6C, 0x24, 0x28, 0x0F, 0x8E, 0x5E, 0x04, 0x00, 0x00, 0x8D, 0x6C, 0xAD, 0x00, 0xC1, 0xE5, 0x04,
    0x03, 0xAA, 0x84, 0x01, 0x00, 0x00, 0xD9, 0x45, 0x10, 0x8D, 0x45, 0x1C, 0xD9, 0x5C, 0x24, 0x1C,
    0x8B, 0x4D, 0x48, 0x85, 0xC9, 0xD9, 0x45, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x45, 0x18, 0xD9,
    0x5C, 0x24, 0x24, 0xD9, 0x00, 0xD9, 0x5C, 0x24, 0x48, 0xD9, 0x40, 0x04, 0xD9, 0x5C, 0x24, 0x4C,
    0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x50, 0xD9, 0x40, 0x0C, 0xD9, 0x5C, 0x24, 0x54, 0xD9, 0x40,
    0x10, 0xD9, 0x5C, 0x24, 0x58, 0xD9, 0x40, 0x14, 0xD9, 0x5C, 0x24, 0x5C, 0xD9, 0x40, 0x18, 0xD9,
    0x5C, 0x24, 0x60, 0xD9, 0x40, 0x1C, 0xD9, 0x5C, 0x24, 0x64, 0xD9, 0x40, 0x20, 0xD9, 0x5C, 0x24,
    0x68, 0x0F, 0x8C, 0xFE, 0x00, 0x00, 0x00, 0x8B, 0x52, 0x4C, 0x8D, 0x0C, 0x89, 0xC1, 0xE1, 0x04,
    0x8B, 0x7C, 0x11, 0x0C, 0x8B, 0x4D, 0x00, 0x33, 0xF6, 0x85, 0xC9, 0x74, 0x22, 0x8B, 0xD1, 0x81,
    0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0xB2, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6,
    0x74, 0x0B, 0x39, 0x4E, 0x44, 0x75, 0x06, 0x83, 0x7E, 0x4C, 0x06, 0x74, 0x02, 0x33, 0xF6, 0x85,
    0xFF, 0x0F, 0x84, 0xBE, 0x00, 0x00, 0x00, 0xD9, 0x45, 0x10, 0x50, 0xD9, 0x5C, 0x24, 0x40, 0x8D,
    0x4F, 0x20, 0xD9, 0x45, 0x14, 0x8D, 0x44, 0x24, 0x70, 0xD9, 0x5C, 0x24, 0x44, 0xD9, 0x45, 0x18,
    0xD9, 0x5C, 0x24, 0x48, 0xE8, 0x07, 0x82, 0x73, 0x00, 0x8D, 0x44, 0x24, 0x3C, 0x50, 0x8D, 0x84,
    0x24, 0x94, 0x00, 0x00, 0x00, 0xE8, 0xC6, 0x30, 0x73, 0x00, 0xD9, 0x47, 0x14, 0x85, 0xF6, 0xD8,
    0x00, 0xD9, 0x5C, 0x24, 0x30, 0x8B, 0x5C, 0x24, 0x30, 0xD9, 0x40, 0x04, 0x89, 0x5C, 0x24, 0x3C,
    0xD8, 0x47, 0x18, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x40, 0x08, 0xD8, 0x47, 0x1C, 0x8B, 0x7C, 0x24,
    0x34, 0x89, 0x7C, 0x24, 0x40, 0xD9, 0x5C, 0x24, 0x38, 0x8B, 0x4C, 0x24, 0x38, 0x89, 0x4C, 0x24,
    0x44, 0x74, 0x33, 0x8D, 0x54, 0x24, 0x6C, 0x52, 0x56, 0x8B, 0xCD, 0x8D, 0x54, 0x24, 0x44, 0xE8,
    0xBC, 0xFC, 0x05, 0x00, 0x8B, 0x44, 0x24, 0x40, 0x89, 0x5E, 0x14, 0x89, 0x7E, 0x18, 0x89, 0x46,
    0x1C, 0x8D, 0x7E, 0x20, 0xB9, 0x09, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x74, 0xF3, 0xA5, 0x8B,
    0x7C, 0x24, 0x3C, 0x83, 0xC4, 0x08, 0x8B, 0x4C, 0x24, 0x38, 0x89, 0x7C, 0x24, 0x20, 0x89, 0x4C,
    0x24, 0x24, 0xB9, 0x09, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x6C, 0x8D, 0x7C, 0x24, 0x48, 0x89,
    0x5C, 0x24, 0x1C, 0xF3, 0xA5, 0x8B, 0x75, 0x08, 0x83, 0xFE, 0xFF, 0x0F, 0x84, 0xC7, 0x02, 0x00,
    0x00, 0x80, 0x7D, 0x0C, 0x01, 0x0F, 0x85, 0x81, 0x02, 0x00, 0x00, 0x8B, 0x15, 0x34, 0xE7, 0x40,
    0x01, 0xDB, 0x05, 0x34, 0xE7, 0x40, 0x01, 0x85, 0xD2, 0xB3, 0x01, 0x7D, 0x06, 0xD8, 0x05, 0xD8,
    0xBF, 0xE5, 0x00, 0xDC, 0x0D, 0x30, 0xC0, 0xE5, 0x00, 0x8B, 0x4D, 0x48, 0x85, 0xC9, 0xD9, 0x5C,
    0x24, 0x10, 0x7C, 0x3A, 0x8B, 0x54, 0x24, 0x18, 0x8D, 0x04, 0x89, 0xC1, 0xE0, 0x04, 0x03, 0x42,
    0x4C, 0x33, 0xD2, 0x8B, 0x78, 0x0C, 0x8B, 0x40, 0x30, 0x25, 0xE0, 0x00, 0x00, 0x40, 0x0B, 0xC2,
    0x74, 0x02, 0x32, 0xDB, 0x85, 0xFF, 0x74, 0x14, 0x8B, 0x87, 0x30, 0x01, 0x00, 0x00, 0x8B, 0x40,
    0x64, 0xA9, 0x00, 0x00, 0x00, 0x20, 0x74, 0x06, 0xA8, 0x01, 0x75, 0x02, 0x32, 0xDB, 0xD9, 0x45,
    0x40, 0xD8, 0x65, 0x44, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xDD, 0x05, 0xA0, 0xC1,
    0xE5, 0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x0F, 0xD9, 0xC9, 0xD9, 0xE0, 0xDE,
    0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x52, 0xEB, 0x04, 0xDD, 0xD9, 0xDD, 0xD8, 0xD9, 0x45,
    0x44, 0xD9, 0x45, 0x40, 0xDE, 0xD9, 0xDF, 0xE0, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0x45, 0x40, 0xF6,
    0xC4, 0x05, 0xD8, 0xD9, 0xDF, 0xE0, 0x7A, 0x1B, 0xF6, 0xC4, 0x41, 0x74, 0x10, 0xD9, 0x45, 0x44,
    0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7A, 0x21, 0x32, 0xDB, 0xEB, 0x1D, 0xDD, 0xD8, 0x32,
    0xDB, 0xEB, 0x17, 0xF6, 0xC4, 0x41, 0x75, 0x10, 0xD9, 0x45, 0x44, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6,
    0xC4, 0x41, 0x7A, 0x06, 0x32, 0xDB, 0xEB, 0x02, 0xDD, 0xD8, 0x83, 0xF9, 0xFF, 0x74, 0x0A, 0x83,
    0x7D, 0x00, 0x00, 0x0F, 0x84, 0xAB, 0x00, 0x00, 0x00, 0x85, 0xF6, 0xD9, 0x05, 0x14, 0x5B, 0x5F,
    0x02, 0xD8, 0x64, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xDC, 0xC8, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x05, 0x18, 0x5B, 0x5F, 0x02, 0xD8, 0x64, 0x24, 0x20, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x44, 0x24, 0x14, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x10, 0xD9,
    0x05, 0x1C, 0x5B, 0x5F, 0x02, 0xD8, 0x64, 0x24, 0x24, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x14, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x14, 0x7C, 0x38, 0x3B, 0x35, 0x88,
    0x72, 0x52, 0x02, 0x7D, 0x30, 0x8D, 0x04, 0x76, 0xC1, 0xE0, 0x04, 0x05, 0xE0, 0x5D, 0x6B, 0x02,
    0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00,
    0x75, 0x09, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x05, 0xD9, 0x40, 0x14, 0xEB, 0x07,
    0xD9, 0x40, 0x10, 0xEB, 0x02, 0xD9, 0xEE, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44, 0x24, 0x14, 0xD8,
    0x74, 0x24, 0x10, 0xDC, 0x1D, 0x68, 0xD8, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x0F, 0x84,
    0xC8, 0x00, 0x00, 0x00, 0x84, 0xDB, 0x0F, 0x84, 0xC0, 0x00, 0x00, 0x00, 0x8B, 0x4D, 0x00, 0x85,
    0xC9, 0x74, 0x40, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x2B, 0x39, 0x48, 0x44, 0x75, 0x26, 0x83, 0x78, 0x4C,
    0x06, 0x75, 0x20, 0x8B, 0x88, 0xC4, 0x00, 0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x14, 0x90,
    0x39, 0x68, 0x0C, 0x0F, 0x84, 0xDF, 0x00, 0x00, 0x00, 0x8B, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85,
    0xC0, 0x75, 0xED, 0x8D, 0x84, 0x24, 0x9C, 0x00, 0x00, 0x00, 0x50, 0x8D, 0x4C, 0x24, 0x4C, 0x8D,
    0x54, 0x24, 0x20, 0xE8, 0xC8, 0xC1, 0xF6, 0xFF, 0x85, 0xF6, 0xC6, 0x84, 0x24, 0xD0, 0x00, 0x00,
    0x00, 0x01, 0x7C, 0x1B, 0x3B, 0x35, 0x88, 0x72, 0x52, 0x02, 0x7D, 0x13, 0x8D, 0x0C, 0x76, 0xC1,
    0xE1, 0x04, 0x8A, 0x81, 0x0E, 0x5E, 0x6B, 0x02, 0xC0, 0xE8, 0x04, 0x24, 0x01, 0xEB, 0x02, 0x32,
    0xC0, 0x8B, 0x54, 0x24, 0x18, 0x88, 0x84, 0x24, 0xDC, 0x00, 0x00, 0x00, 0x66, 0x8B, 0x42, 0x08,
    0x8D, 0x8C, 0x24, 0x9C, 0x00, 0x00, 0x00, 0x51, 0x66, 0x89, 0x84, 0x24, 0xE8, 0x00, 0x00, 0x00,
    0x89, 0xAC, 0x24, 0x04, 0x01, 0x00, 0x00, 0xE8, 0xE4, 0x32, 0x06, 0x00, 0x83, 0xC4, 0x04, 0x85,
    0xC0, 0x89, 0x45, 0x00, 0x74, 0x62, 0x80, 0x4D, 0x4C, 0x01, 0xEB, 0x5C, 0x8B, 0x45, 0x00, 0x85,
    0xC0, 0x74, 0x55, 0xF6, 0x45, 0x4C, 0x01, 0x74, 0x4F, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F, 0x00,
    0x00, 0xC1, 0xE2, 0x04, 0x8B, 0xBA, 0x64, 0x9C, 0x14, 0x02, 0xEB, 0x1E, 0x8B, 0x45, 0x00, 0x85,
    0xC0, 0x74, 0x35, 0xF6, 0x45, 0x4C, 0x01, 0x74, 0x2F, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00,
    0x00, 0xC1, 0xE1, 0x04, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x16, 0x39, 0x47,
    0x44, 0x75, 0x11, 0x83, 0x7F, 0x4C, 0x06, 0x75, 0x0B, 0x55, 0x6A, 0x00, 0xE8, 0xAF, 0xFA, 0x05,
    0x00, 0x83, 0xC4, 0x08, 0x80, 0x65, 0x4C, 0xFE, 0x8B, 0x6C, 0x24, 0x28, 0x8B, 0x54, 0x24, 0x18,
    0x83, 0xC5, 0x01, 0x3B, 0xAA, 0x80, 0x01, 0x00, 0x00, 0x89, 0x6C, 0x24, 0x28, 0x0F, 0x8C, 0xA6,
    0xFB, 0xFF, 0xFF, 0x8B, 0x44, 0x24, 0x2C, 0x83, 0xC0, 0x01, 0x3B, 0x05, 0x58, 0x1B, 0xF7, 0x00,
    0x89, 0x44, 0x24, 0x2C, 0x0F, 0x8C, 0x72, 0xFB, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4,
    0xFC, 0x00, 0x00, 0x00, 0xC3,
];
#[allow(dead_code)]
const CFU_BODY_CALLS: [usize; 6] = [0x104, 0x115, 0x15f, 0x3b3, 0x407, 0x46c];
#[allow(dead_code)]
const CFU_BODY_DISPS: [usize; 23] = [0xc, 0x1f, 0xca, 0x1bd, 0x1c3, 0x1cf, 0x1d5, 0x22e, 0x2ad, 0x2c5, 0x2e1, 0x2ff, 0x30c, 0x312, 0x31b, 0x324, 0x345, 0x370, 0x3c6, 0x3d4, 0x436, 0x456, 0x49c];
#[allow(dead_code)]
const GMA_BODY: [u8; 389] = [
    0x51, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x04, 0x24, 0xA1, 0x60, 0x3C, 0x7B, 0x02,
    0x85, 0xC0, 0x53, 0x56, 0x57, 0x0F, 0x84, 0x5A, 0x01, 0x00, 0x00, 0x83, 0x78, 0x3C, 0x01, 0x75,
    0x08, 0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0xEB, 0x02, 0x33, 0xC9, 0x8B, 0x01, 0x8B, 0x50, 0x40,
    0xFF, 0xD2, 0x80, 0x3D, 0xB4, 0xAC, 0xF8, 0x01, 0x00, 0x74, 0x0A, 0x6A, 0x00, 0xE8, 0x0E, 0x42,
    0x25, 0x00, 0x83, 0xC4, 0x04, 0x8B, 0x0D, 0x88, 0x8E, 0x40, 0x01, 0x85, 0xC9, 0x75, 0x38, 0xA1,
    0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0x19, 0x01, 0x00, 0x00, 0x50, 0xE8, 0xEE, 0x3F,
    0x2D, 0x00, 0x84, 0xC0, 0x0F, 0x85, 0x0B, 0x01, 0x00, 0x00, 0xE8, 0x91, 0xCF, 0x16, 0x00, 0x8B,
    0x10, 0x8B, 0xC8, 0x8B, 0x42, 0x3C, 0xFF, 0xD0, 0x5F, 0x5E, 0x5B, 0x8B, 0x0C, 0x24, 0x33, 0xCC,
    0xE8, 0x2A, 0xA3, 0x75, 0x00, 0x59, 0xC3, 0x8B, 0x11, 0x8B, 0x42, 0x1C, 0xFF, 0xD0, 0x84, 0xC0,
    0x0F, 0x85, 0xDF, 0x00, 0x00, 0x00, 0x80, 0x3D, 0x8E, 0xBF, 0x40, 0x01, 0x00, 0xA1, 0x08, 0x63,
    0x3D, 0x03, 0x8B, 0xF8, 0x8B, 0xD8, 0x74, 0x35, 0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74,
    0x2C, 0x83, 0x78, 0x3C, 0x01, 0x75, 0x26, 0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x11, 0x8B,
    0x42, 0x04, 0xFF, 0xD0, 0x83, 0xF8, 0x0E, 0x75, 0x14, 0xE8, 0xC2, 0x43, 0x14, 0x00, 0x5F, 0x5E,
    0x5B, 0x8B, 0x0C, 0x24, 0x33, 0xCC, 0xE8, 0xD4, 0xA2, 0x75, 0x00, 0x59, 0xC3, 0xE8, 0x6E, 0x76,
    0x16, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0x8B, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02,
    0x8A, 0x81, 0x63, 0x11, 0x00, 0x00, 0xA8, 0x08, 0x74, 0x11, 0x24, 0xFB, 0x51, 0x88, 0x81, 0x63,
    0x11, 0x00, 0x00, 0xE8, 0x28, 0xE2, 0x49, 0x00, 0x83, 0xC4, 0x04, 0x83, 0x3D, 0x90, 0x7C, 0x52,
    0x02, 0x00, 0x74, 0x0A, 0xBE, 0x90, 0x7C, 0x52, 0x02, 0xE8, 0x62, 0x82, 0x21, 0x00, 0x8B, 0x0D,
    0x60, 0x3C, 0x7B, 0x02, 0x8B, 0x41, 0x3C, 0x85, 0xC0, 0x74, 0x1E, 0x83, 0xF8, 0x03, 0x74, 0x19,
    0x6A, 0x01, 0xE8, 0x69, 0x6D, 0x16, 0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5B, 0x8B, 0x0C, 0x24,
    0x33, 0xCC, 0xE8, 0x68, 0xA2, 0x75, 0x00, 0x59, 0xC3, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0xD0,
    0x07, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x48, 0xAF, 0xEA, 0x00, 0x7E, 0x0A, 0x2D,
    0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x48, 0xAF, 0xEA, 0x00, 0x89, 0x1D, 0x74, 0x3C, 0x7B, 0x02, 0x89,
    0x3D, 0x78, 0x3C, 0x7B, 0x02, 0x8B, 0x4C, 0x24, 0x0C, 0x5F, 0x5E, 0x5B, 0x33, 0xCC, 0xE8, 0x2C,
    0xA2, 0x75, 0x00, 0x59, 0xC3,
];
#[allow(dead_code)]
const GMA_BODY_CALLS: [usize; 12] = [0x3d, 0x5d, 0x6a, 0x80, 0xc9, 0xd6, 0xdd, 0x103, 0x119, 0x132, 0x142, 0x17e];
#[allow(dead_code)]
const GMA_BODY_DISPS: [usize; 17] = [0x2, 0xc, 0x34, 0x47, 0x50, 0x98, 0x9e, 0xa9, 0xec, 0x10d, 0x115, 0x120, 0x14a, 0x159, 0x165, 0x16b, 0x171];
#[allow(dead_code)]
const NFU_BODY: [u8; 116] = [
    0x51, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x84, 0xC0, 0x74, 0x68, 0xE8, 0x11, 0x50, 0x00, 0x00, 0xE8,
    0x5C, 0x29, 0x00, 0x00, 0xE8, 0x77, 0x02, 0x02, 0x00, 0xE8, 0x72, 0x99, 0x03, 0x00, 0xE8, 0xFD,
    0x35, 0x01, 0x00, 0xE8, 0xD8, 0x68, 0x03, 0x00, 0xE8, 0x53, 0x0A, 0x00, 0x00, 0x8B, 0x0D, 0x14,
    0x8C, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x26, 0x8B, 0x41, 0x1C, 0x83, 0xE8, 0x01, 0x74, 0x0A, 0x83,
    0xE8, 0x05, 0x74, 0x05, 0x83, 0xE8, 0x02, 0x75, 0x14, 0x51, 0xE8, 0x91, 0x83, 0x01, 0x00, 0x84,
    0xC0, 0x74, 0x05, 0xE8, 0x18, 0x9F, 0x00, 0x00, 0xE8, 0x53, 0x9E, 0xC0, 0xFF, 0xE8, 0x3E, 0xD2,
    0x01, 0x00, 0xA1, 0x5C, 0x8B, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x07, 0x50, 0xFF, 0x15, 0xB8, 0x80,
    0xDB, 0x00, 0x59, 0xC3,
];
#[allow(dead_code)]
const NFU_BODY_CALLS: [usize; 11] = [0xa, 0xf, 0x14, 0x19, 0x1e, 0x23, 0x28, 0x4a, 0x53, 0x58, 0x5d];
#[allow(dead_code)]
const NFU_BODY_DISPS: [usize; 4] = [0x2, 0x2f, 0x63, 0x6e];
#[allow(dead_code)]
const PCF_BODY: [u8; 527] = [
    0x51, 0x57, 0x33, 0xFF, 0x39, 0x3D, 0x08, 0x7C, 0x52, 0x02, 0xC6, 0x05, 0x67, 0x72, 0x52, 0x02,
    0x00, 0x0F, 0x8F, 0xA4, 0x01, 0x00, 0x00, 0x80, 0x3D, 0x89, 0x89, 0x52, 0x02, 0x00, 0x0F, 0x85,
    0x97, 0x01, 0x00, 0x00, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0x8B, 0x0D, 0x80, 0xE7, 0xE8, 0x00, 0x53,
    0x55, 0x8D, 0xA8, 0x9C, 0x13, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x89, 0x7D, 0x00, 0x89, 0xB8, 0xA0,
    0x13, 0x00, 0x00, 0x89, 0xB8, 0xA4, 0x13, 0x00, 0x00, 0x89, 0xB8, 0xA8, 0x13, 0x00, 0x00, 0x89,
    0xB8, 0xAC, 0x13, 0x00, 0x00, 0x8B, 0x89, 0xB4, 0x4C, 0xDD, 0x00, 0x3B, 0xCF, 0x56, 0x8B, 0xF0,
    0xC6, 0x44, 0x24, 0x13, 0x01, 0xB3, 0x01, 0x74, 0x06, 0x50, 0xFF, 0xD1, 0x83, 0xC4, 0x04, 0x56,
    0xE8, 0x5B, 0xA4, 0x48, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x74, 0x42, 0xF6, 0x86, 0x61, 0x11,
    0x00, 0x00, 0x01, 0x74, 0x0B, 0x57, 0x8B, 0xCE, 0xE8, 0xA3, 0xF0, 0x4D, 0x00, 0x83, 0xC4, 0x04,
    0x8B, 0x8E, 0x74, 0x0D, 0x00, 0x00, 0x3B, 0xCF, 0x74, 0x24, 0xE8, 0xF1, 0x76, 0x5E, 0x00, 0x8B,
    0xF8, 0x85, 0xFF, 0x74, 0x19, 0x8B, 0xCF, 0xE8, 0x54, 0x97, 0x5B, 0x00, 0x84, 0xC0, 0x74, 0x0E,
    0x6A, 0x00, 0x6A, 0x00, 0x8B, 0xC7, 0xE8, 0x05, 0x95, 0x5B, 0x00, 0x83, 0xC4, 0x08, 0xB9, 0x02,
    0x00, 0x00, 0x00, 0x39, 0x0D, 0xE8, 0x5A, 0x5F, 0x02, 0x75, 0x09, 0xC6, 0x44, 0x24, 0x13, 0x00,
    0x32, 0xDB, 0xEB, 0x74, 0xE8, 0xE7, 0xBA, 0x1C, 0x00, 0x84, 0xC0, 0x74, 0x1C, 0xB8, 0x59, 0x00,
    0x00, 0x00, 0xE8, 0xA9, 0x48, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x05, 0xE8, 0xA0, 0x91, 0x1C, 0x00,
    0xC6, 0x44, 0x24, 0x13, 0x00, 0x32, 0xDB, 0xEB, 0x4F, 0x84, 0x8E, 0xE8, 0x18, 0x00, 0x00, 0x74,
    0x31, 0xF6, 0x45, 0x00, 0x01, 0x74, 0x22, 0xB8, 0x2F, 0x00, 0x00, 0x00, 0xE8, 0x7F, 0x48, 0x00,
    0x00, 0x8A, 0x0D, 0x0F, 0x48, 0xF8, 0x00, 0x02, 0xC0, 0x02, 0xC0, 0x02, 0xC0, 0x32, 0xC1, 0x24,
    0x08, 0x32, 0xC8, 0x88, 0x0D, 0x0F, 0x48, 0xF8, 0x00, 0xC6, 0x44, 0x24, 0x13, 0x00, 0x32, 0xDB,
    0xEB, 0x16, 0xF6, 0x86, 0x63, 0x11, 0x00, 0x00, 0x20, 0x75, 0x0D, 0x8D, 0x54, 0x24, 0x13, 0x52,
    0xE8, 0xEB, 0xF7, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0xE8, 0xE3, 0xFB, 0xFF, 0xFF, 0xA1, 0x80, 0xE7,
    0xE8, 0x00, 0x83, 0xF8, 0xFF, 0x0F, 0x84, 0xAE, 0x00, 0x00, 0x00, 0x84, 0xDB, 0x74, 0x18, 0x8B,
    0xC8, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0xBC, 0x4C, 0xDD, 0x00, 0x85, 0xC9, 0x74, 0x09, 0xFF, 0xD1,
    0xA1, 0x80, 0xE7, 0xE8, 0x00, 0xEB, 0x14, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9, 0x1D, 0xB0,
    0x5C, 0x5F, 0x02, 0xD9, 0xE8, 0xD9, 0x1D, 0xB8, 0x5C, 0x5F, 0x02, 0x83, 0x3D, 0x08, 0x7C, 0x52,
    0x02, 0x00, 0x7F, 0x0A, 0xE8, 0x67, 0x7F, 0xF9, 0xFF, 0xA1, 0x80, 0xE7, 0xE8, 0x00, 0x80, 0x7C,
    0x24, 0x13, 0x00, 0x74, 0x64, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0xB8, 0x4C, 0xDD, 0x00, 0x85, 0xC0,
    0x74, 0x57, 0x5E, 0x5D, 0x5B, 0x5F, 0x83, 0xC4, 0x04, 0xFF, 0xE0, 0x80, 0x3D, 0x14, 0x7D, 0x52,
    0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0xF7, 0x7C, 0x52, 0x02, 0x00, 0x75, 0x09, 0x80, 0x3D, 0x15,
    0x7D, 0x52, 0x02, 0x00, 0x74, 0x13, 0xB8, 0x59, 0x00, 0x00, 0x00, 0xE8, 0xB0, 0x47, 0x00, 0x00,
    0x84, 0xC0, 0x74, 0x05, 0xE8, 0xA7, 0x90, 0x1C, 0x00, 0x80, 0x3D, 0x89, 0x89, 0x52, 0x02, 0x00,
    0x74, 0x1A, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x5F, 0xD9, 0x1D, 0xB0, 0x5C, 0x5F, 0x02, 0xD9,
    0xE8, 0xD9, 0x1D, 0xB8, 0x5C, 0x5F, 0x02, 0x59, 0xC3, 0x5E, 0x5D, 0x5B, 0x5F, 0x59, 0xC3,
];
#[allow(dead_code)]
const PCF_BODY_CALLS: [usize; 14] = [0x70, 0x88, 0x9a, 0xa7, 0xb6, 0xd4, 0xe2, 0xeb, 0x10c, 0x140, 0x148, 0x194, 0x1db, 0x1e4];
#[allow(dead_code)]
const PCF_BODY_DISPS: [usize; 25] = [0x6, 0xc, 0x19, 0x25, 0x2b, 0x57, 0xc5, 0x113, 0x125, 0x14e, 0x166, 0x171, 0x179, 0x17f, 0x187, 0x18d, 0x19a, 0x1aa, 0x1bd, 0x1c6, 0x1cf, 0x1eb, 0x1f4, 0x1fb, 0x203];
#[allow(dead_code)]
const VSU_BODY: [u8; 1333] = [
    0xA1, 0xA0, 0xA4, 0xE9, 0x00, 0x83, 0xEC, 0x74, 0x3C, 0xFF, 0x53, 0x55, 0x56, 0x57, 0x0F, 0x84,
    0x19, 0x05, 0x00, 0x00, 0x8B, 0xD0, 0xC1, 0xEA, 0x08, 0x0F, 0x84, 0x0E, 0x05, 0x00, 0x00, 0x0F,
    0xB6, 0xC0, 0x8B, 0xC8, 0x69, 0xC9, 0x9C, 0x01, 0x00, 0x00, 0x3B, 0x91, 0xAC, 0xB9, 0xEE, 0x00,
    0x0F, 0x85, 0xF7, 0x04, 0x00, 0x00, 0x83, 0xB9, 0xF8, 0xB8, 0xEE, 0x00, 0xFF, 0x0F, 0x84, 0xEA,
    0x04, 0x00, 0x00, 0x8B, 0x0D, 0x74, 0x6B, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x24, 0x8B, 0xD1, 0x81,
    0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0xAA, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xED,
    0x74, 0x0F, 0x39, 0x4D, 0x44, 0x75, 0x0A, 0x83, 0x7D, 0x4C, 0x05, 0x0F, 0x84, 0x80, 0x00, 0x00,
    0x00, 0xBA, 0x96, 0x00, 0x00, 0x00, 0xE8, 0x05, 0x2A, 0xFF, 0xFF, 0x0F, 0xB7, 0x0D, 0x7A, 0x34,
    0xEE, 0x00, 0x0F, 0xB7, 0x35, 0x78, 0x34, 0xEE, 0x00, 0xC7, 0x05, 0xA0, 0xA4, 0xE9, 0x00, 0xFF,
    0x00, 0x00, 0x00, 0xE8, 0xF8, 0x31, 0x75, 0x00, 0x69, 0xC0, 0xE8, 0x03, 0x00, 0x00, 0x85, 0xC0,
    0x7C, 0x2A, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x03, 0xC1, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3,
    0xB0, 0xA4, 0xE9, 0x00, 0x0F, 0x8E, 0x73, 0x04, 0x00, 0x00, 0x2D, 0x00, 0xD2, 0x49, 0x6B, 0xA3,
    0xB0, 0xA4, 0xE9, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x74, 0xC3, 0x8B, 0x15, 0xAC, 0x43,
    0xE8, 0x00, 0x03, 0xC2, 0xA3, 0xB0, 0xA4, 0xE9, 0x00, 0x0F, 0x89, 0x4E, 0x04, 0x00, 0x00, 0x05,
    0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xB0, 0xA4, 0xE9, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x74,
    0xC3, 0x8B, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x55, 0xE8, 0x93, 0xF0, 0x52, 0x00, 0x8A, 0xD8, 0xA1,
    0xAC, 0xA4, 0xE9, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x88, 0x5C, 0x24, 0x13, 0x0F, 0x8C, 0x45,
    0x01, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC1, 0x0F, 0x8E, 0x29, 0x01, 0x00,
    0x00, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8C, 0x2A, 0x01, 0x00, 0x00, 0x80, 0xBD,
    0xD8, 0x84, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x86, 0x00, 0x00, 0x00, 0x80, 0xBD, 0xD8, 0x84, 0x00,
    0x00, 0x01, 0x75, 0x7D, 0xD9, 0xE8, 0x8B, 0x95, 0xA4, 0x6D, 0x00, 0x00, 0x8B, 0x42, 0x18, 0x8D,
    0x8D, 0xA4, 0x6D, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x6A, 0x00, 0xFF, 0xD0, 0xD9, 0xE8, 0x8B,
    0x95, 0xD0, 0x6D, 0x00, 0x00, 0x8B, 0x42, 0x18, 0x8D, 0x8D, 0xD0, 0x6D, 0x00, 0x00, 0x51, 0xD9,
    0x1C, 0x24, 0x6A, 0x00, 0xFF, 0xD0, 0xD9, 0xE8, 0x8B, 0x95, 0x58, 0x6E, 0x00, 0x00, 0x8B, 0x42,
    0x18, 0x8D, 0x8D, 0x58, 0x6E, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x6A, 0x00, 0xFF, 0xD0, 0xD9,
    0xE8, 0x8B, 0x95, 0xF8, 0x6D, 0x00, 0x00, 0x8B, 0x42, 0x18, 0x8D, 0x8D, 0xF8, 0x6D, 0x00, 0x00,
    0x51, 0xD9, 0x1C, 0x24, 0x6A, 0x00, 0xFF, 0xD0, 0xD9, 0xE8, 0x8B, 0x95, 0x28, 0x6E, 0x00, 0x00,
    0x8B, 0x42, 0x18, 0x8D, 0x8D, 0x28, 0x6E, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x6A, 0x00, 0xFF,
    0xD0, 0x0F, 0xB7, 0x85, 0x88, 0x8D, 0x00, 0x00, 0xA9, 0x00, 0x08, 0x00, 0x00, 0x74, 0x1C, 0x25,
    0xFF, 0xF7, 0x00, 0x00, 0x6A, 0x7D, 0x8D, 0x8D, 0x14, 0x8D, 0x00, 0x00, 0x66, 0x89, 0x85, 0x88,
    0x8D, 0x00, 0x00, 0xE8, 0xC8, 0xC0, 0xFD, 0xFF, 0x83, 0xC4, 0x04, 0x68, 0x96, 0x00, 0x00, 0x00,
    0xB9, 0xA0, 0xA4, 0xE9, 0x00, 0xE8, 0xB6, 0xC0, 0xFD, 0xFF, 0x0F, 0xB7, 0x0D, 0x7A, 0x34, 0xEE,
    0x00, 0x0F, 0xB7, 0x35, 0x78, 0x34, 0xEE, 0x00, 0x83, 0xC4, 0x04, 0xC7, 0x05, 0xA0, 0xA4, 0xE9,
    0x00, 0xFF, 0x00, 0x00, 0x00, 0xE8, 0x76, 0x30, 0x75, 0x00, 0x69, 0xC0, 0xE8, 0x03, 0x00, 0x00,
    0x85, 0xC0, 0x0F, 0x8C, 0x6E, 0x02, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x03, 0xC1,
    0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xB0, 0xA4, 0xE9, 0x00, 0x0F, 0x8E, 0x6F, 0x02, 0x00, 0x00,
    0x2D, 0x00, 0xD2, 0x49, 0x6B, 0xE9, 0x60, 0x02, 0x00, 0x00, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9,
    0xA4, 0x35, 0x0F, 0x8C, 0xD6, 0xFE, 0xFF, 0xFF, 0xF6, 0x85, 0x50, 0x8F, 0x00, 0x00, 0x01, 0x0F,
    0x85, 0xC9, 0xFE, 0xFF, 0xFF, 0x8D, 0x7D, 0x14, 0x8B, 0xCF, 0xBA, 0xA0, 0xA4, 0xE9, 0x00, 0xE8,
    0x8C, 0x60, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x0A, 0xC7, 0x05, 0xA0, 0xA4, 0xE9, 0x00, 0xFF, 0x00,
    0x00, 0x00, 0x8D, 0x75, 0x20, 0x56, 0xB9, 0xA0, 0xA4, 0xE9, 0x00, 0xE8, 0x80, 0xC0, 0xFD, 0xFF,
    0x83, 0xC4, 0x04, 0xB9, 0xA0, 0xA4, 0xE9, 0x00, 0xE8, 0x53, 0xC1, 0xFD, 0xFF, 0x84, 0xDB, 0x74,
    0x5E, 0x84, 0xC0, 0x0F, 0x85, 0x9A, 0x00, 0x00, 0x00, 0xB8, 0xAC, 0xA4, 0xE9, 0x00, 0xE8, 0xED,
    0x32, 0x75, 0x00, 0x3D, 0x88, 0x13, 0x00, 0x00, 0x7E, 0x20, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05,
    0x88, 0x13, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xAC, 0xA4, 0xE9, 0x00, 0x7E, 0x0A,
    0x2D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xAC, 0xA4, 0xE9, 0x00, 0xD9, 0x05, 0x7C, 0x34, 0xEE, 0x00,
    0x51, 0xD9, 0x1D, 0x98, 0x9B, 0xE9, 0x00, 0xBE, 0xA0, 0xA4, 0xE9, 0x00, 0xD9, 0x05, 0x98, 0x9B,
    0xE9, 0x00, 0xD9, 0x1C, 0x24, 0xE8, 0x36, 0xC1, 0xFD, 0xFF, 0xE9, 0x9D, 0x00, 0x00, 0x00, 0x84,
    0xC0, 0x74, 0x48, 0xD9, 0x07, 0x56, 0xD8, 0x65, 0x50, 0x57, 0xB9, 0xA0, 0xA4, 0xE9, 0x00, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x47, 0x04, 0xD8, 0x65, 0x54, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x47, 0x08,
    0x8D, 0x7C, 0x24, 0x20, 0xD8, 0x65, 0x58, 0xD9, 0x5C, 0x24, 0x28, 0xE8, 0xE0, 0x62, 0xFF, 0xFF,
    0x83, 0xC4, 0x08, 0x84, 0xC0, 0x75, 0x68, 0xC7, 0x05, 0xA0, 0xA4, 0xE9, 0x00, 0xFF, 0x00, 0x00,
    0x00, 0xEB, 0x5C, 0xD9, 0x05, 0x80, 0x34, 0xEE, 0x00, 0xEB, 0x02, 0xD9, 0xE8, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x05, 0x7C, 0x43, 0xE8, 0x00, 0xDC, 0x0D, 0x70, 0xBF, 0xE5, 0x00, 0xD8, 0x05, 0x98,
    0x9B, 0xE9, 0x00, 0xD9, 0x1D, 0x98, 0x9B, 0xE9, 0x00, 0xD9, 0x05, 0x98, 0x9B, 0xE9, 0x00, 0xD9,
    0x44, 0x24, 0x14, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0x74, 0x08, 0xD9, 0x1D,
    0x98, 0x9B, 0xE9, 0x00, 0xEB, 0x02, 0xDD, 0xD8, 0xD9, 0x05, 0x98, 0x9B, 0xE9, 0x00, 0x51, 0xBA,
    0xA0, 0xA4, 0xE9, 0x00, 0xD9, 0x1C, 0x24, 0xE8, 0xB4, 0xBF, 0xFD, 0xFF, 0x83, 0xC4, 0x04, 0xB8,
    0xAC, 0xA4, 0xE9, 0x00, 0xE8, 0xF7, 0x31, 0x75, 0x00, 0x8B, 0xC8, 0xB8, 0xD3, 0x4D, 0x62, 0x10,
    0xF7, 0xE9, 0xC1, 0xFA, 0x06, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x24, 0x01, 0x3C, 0x01,
    0x0F, 0x94, 0xC0, 0x66, 0xF7, 0x85, 0x88, 0x8D, 0x00, 0x00, 0x00, 0x08, 0x88, 0x44, 0x24, 0x14,
    0x74, 0x26, 0x84, 0xC0, 0x74, 0x0A, 0x8B, 0x45, 0x44, 0xE8, 0xF2, 0xF0, 0xFF, 0xFF, 0xEB, 0x18,
    0x8B, 0x4D, 0x44, 0xE8, 0x38, 0xF2, 0xFF, 0xFF, 0x6A, 0x7D, 0x8D, 0x8D, 0x14, 0x8D, 0x00, 0x00,
    0xE8, 0xBB, 0xBE, 0xFD, 0xFF, 0x83, 0xC4, 0x04, 0x80, 0xBD, 0xD8, 0x84, 0x00, 0x00, 0x00, 0x0F,
    0x84, 0x28, 0x01, 0x00, 0x00, 0x80, 0xBD, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x0F, 0x85, 0x1B, 0x01,
    0x00, 0x00, 0xD9, 0xE8, 0x8B, 0x95, 0xA4, 0x6D, 0x00, 0x00, 0x8B, 0x74, 0x24, 0x14, 0x8B, 0x42,
    0x18, 0x8D, 0x8D, 0xA4, 0x6D, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x56, 0xFF, 0xD0, 0xD9, 0xE8,
    0x8B, 0x95, 0xD0, 0x6D, 0x00, 0x00, 0x8B, 0x42, 0x18, 0x8D, 0x8D, 0xD0, 0x6D, 0x00, 0x00, 0x51,
    0xD9, 0x1C, 0x24, 0x56, 0xFF, 0xD0, 0xD9, 0xE8, 0x8B, 0x95, 0x58, 0x6E, 0x00, 0x00, 0x8B, 0x42,
    0x18, 0x8D, 0x8D, 0x58, 0x6E, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x56, 0xFF, 0xD0, 0xD9, 0xE8,
    0x8B, 0x95, 0xF8, 0x6D, 0x00, 0x00, 0x8B, 0x42, 0x18, 0x8D, 0x8D, 0xF8, 0x6D, 0x00, 0x00, 0x51,
    0xD9, 0x1C, 0x24, 0x56, 0xFF, 0xD0, 0xD9, 0xE8, 0x8B, 0x95, 0x28, 0x6E, 0x00, 0x00, 0x8B, 0x42,
    0x18, 0x8D, 0x8D, 0x28, 0x6E, 0x00, 0x00, 0x51, 0xD9, 0x1C, 0x24, 0x56, 0xFF, 0xD0, 0x5F, 0x5E,
    0x5D, 0x5B, 0x83, 0xC4, 0x74, 0xC3, 0x8B, 0x15, 0xAC, 0x43, 0xE8, 0x00, 0x03, 0xC2, 0xA3, 0xB0,
    0xA4, 0xE9, 0x00, 0x79, 0x0A, 0x05, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0xB0, 0xA4, 0xE9, 0x00, 0x8B,
    0x9D, 0x50, 0x8F, 0x00, 0x00, 0xF6, 0xC3, 0x01, 0x75, 0x73, 0x80, 0x7C, 0x24, 0x13, 0x00, 0xD9,
    0x05, 0x98, 0x9B, 0xE9, 0x00, 0x0F, 0xB7, 0x05, 0x94, 0x9B, 0xE9, 0x00, 0x51, 0x8D, 0x74, 0x24,
    0x28, 0xD9, 0x1C, 0x24, 0x55, 0x6A, 0x00, 0x50, 0x74, 0x3E, 0xE8, 0xD1, 0xA7, 0xFD, 0xFF, 0x8B,
    0x54, 0x24, 0x7C, 0xC1, 0xEB, 0x0D, 0x80, 0xE3, 0x01, 0x66, 0x0F, 0xB6, 0xCB, 0x66, 0xC1, 0xE1,
    0x05, 0x81, 0xE2, 0xDF, 0xFF, 0x00, 0x00, 0x66, 0x0B, 0xCA, 0x66, 0x83, 0xC9, 0x10, 0x66, 0x89,
    0x4C, 0x24, 0x7C, 0x8D, 0x44, 0x24, 0x24, 0x8D, 0x4C, 0x24, 0x18, 0xE8, 0xC0, 0xB9, 0xFD, 0xFF,
    0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x74, 0xC3, 0x8D, 0x7D, 0x14, 0xE8, 0xC0, 0xAA, 0xFD, 0xFF,
    0x8D, 0x44, 0x24, 0x24, 0x8D, 0x4C, 0x24, 0x18, 0xE8, 0xA3, 0xB9, 0xFD, 0xFF, 0x5F, 0x5E, 0x5D,
    0x5B, 0x83, 0xC4, 0x74, 0xC3,
];
#[allow(dead_code)]
const VSU_BODY_CALLS: [usize; 21] = [0x76, 0x93, 0xf8, 0x1e3, 0x1f5, 0x215, 0x26f, 0x28b, 0x298, 0x2ae, 0x2f5, 0x32b, 0x397, 0x3a4, 0x3d9, 0x3e3, 0x3f0, 0x4da, 0x50b, 0x51b, 0x528];
#[allow(dead_code)]
const VSU_BODY_DISPS: [usize; 52] = [0x1, 0x2c, 0x38, 0x45, 0x5a, 0x7e, 0x85, 0x8b, 0xa4, 0xb0, 0xc0, 0xce, 0xd5, 0xe5, 0xf3, 0x100, 0x115, 0x1f1, 0x1fd, 0x204, 0x20d, 0x22a, 0x236, 0x26b, 0x27a, 0x287, 0x294, 0x2aa, 0x2bb, 0x2ca, 0x2d6, 0x2dc, 0x2e3, 0x2e8, 0x2ee, 0x30b, 0x339, 0x345, 0x353, 0x359, 0x35f, 0x365, 0x36b, 0x380, 0x38a, 0x390, 0x3a0, 0x498, 0x49f, 0x4ab, 0x4c1, 0x4c8];
#[allow(dead_code)]
const WTL_BODY: [u8; 1412] = [
    0x83, 0xEC, 0x3C, 0x53, 0x55, 0x56, 0x33, 0xF6, 0x80, 0x3D, 0x8B, 0x6E, 0xE8, 0x00, 0x00, 0x57,
    0x0F, 0x84, 0x46, 0x05, 0x00, 0x00, 0x8B, 0x3D, 0xD4, 0x03, 0x17, 0x02, 0x3B, 0xFE, 0x0F, 0x84,
    0x38, 0x05, 0x00, 0x00, 0x8D, 0x87, 0xCC, 0x10, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0x8B, 0x00,
    0x3B, 0xC6, 0x74, 0x0F, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x3B, 0xC6, 0x74, 0x05, 0x8B, 0x58,
    0x08, 0xEB, 0x02, 0x33, 0xDB, 0x8D, 0x44, 0x24, 0x34, 0x50, 0x8B, 0xC7, 0xE8, 0x3F, 0xB2, 0xE5,
    0xFF, 0x83, 0xC4, 0x04, 0x3B, 0xDE, 0x0F, 0x84, 0x00, 0x05, 0x00, 0x00, 0xF7, 0x43, 0x0C, 0x00,
    0x04, 0x00, 0x00, 0x0F, 0x84, 0xF3, 0x04, 0x00, 0x00, 0xA1, 0x30, 0xA1, 0x52, 0x02, 0x2B, 0xC6,
    0x0F, 0x84, 0xA8, 0x04, 0x00, 0x00, 0x83, 0xE8, 0x01, 0x0F, 0x84, 0xB7, 0x01, 0x00, 0x00, 0x83,
    0xE8, 0x01, 0x0F, 0x85, 0xF4, 0x04, 0x00, 0x00, 0x8B, 0x0D, 0x2C, 0xA1, 0x52, 0x02, 0xE8, 0x1D,
    0x7B, 0xF7, 0xFF, 0x8B, 0xE8, 0x3B, 0xEE, 0x0F, 0x84, 0x6F, 0x01, 0x00, 0x00, 0x8B, 0xD5, 0xE8,
    0x8C, 0x05, 0xF4, 0xFF, 0x84, 0xC0, 0x0F, 0x85, 0x60, 0x01, 0x00, 0x00, 0x55, 0xE8, 0x5E, 0xE0,
    0xE4, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x0F, 0x85, 0xAA, 0x01, 0x00, 0x00, 0xE8, 0x3E, 0xF9,
    0xFF, 0xFF, 0x3B, 0xC6, 0x74, 0x08, 0x3B, 0xC5, 0x0F, 0x85, 0x3E, 0x01, 0x00, 0x00, 0xD9, 0x45,
    0x14, 0x8D, 0x74, 0x24, 0x28, 0xD8, 0x25, 0x20, 0x5B, 0x5F, 0x02, 0xD9, 0x5C, 0x24, 0x28, 0xD9,
    0x45, 0x18, 0xD8, 0x25, 0x24, 0x5B, 0x5F, 0x02, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x45, 0x1C, 0xD8,
    0x25, 0x28, 0x5B, 0x5F, 0x02, 0xD9, 0x5C, 0x24, 0x30, 0xE8, 0x12, 0x33, 0x07, 0x00, 0xD9, 0x44,
    0x24, 0x2C, 0xD8, 0x4C, 0x24, 0x38, 0xD9, 0x44, 0x24, 0x28, 0xD8, 0x4C, 0x24, 0x34, 0xDE, 0xC1,
    0xD9, 0x44, 0x24, 0x30, 0xD8, 0x4C, 0x24, 0x3C, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x05,
    0x98, 0x79, 0xE8, 0x00, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4,
    0x41, 0x75, 0x19, 0xD9, 0xE8, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x0E, 0x51, 0xD9,
    0x1C, 0x24, 0xE8, 0xA9, 0x40, 0x8C, 0xFF, 0x83, 0xC4, 0x04, 0xEB, 0x04, 0xDD, 0xD8, 0xD9, 0xEE,
    0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0x83, 0xC4, 0x00, 0x00, 0x00, 0xDC, 0x0D,
    0x60, 0xC1, 0xE5, 0x00, 0xDC, 0x35, 0x48, 0xC3, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44,
    0x24, 0x1C, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x44, 0xE8, 0x80, 0xF8, 0xFF, 0xFF,
    0x3B, 0xC5, 0x74, 0x3B, 0xB9, 0x5C, 0xD0, 0xEB, 0x00, 0xC7, 0x05, 0x30, 0xA1, 0x52, 0x02, 0x01,
    0x00, 0x00, 0x00, 0xE8, 0xD8, 0x4F, 0x91, 0xFF, 0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x3D, 0x00, 0xD2,
    0x49, 0x6B, 0xA3, 0x54, 0xD0, 0xEB, 0x00, 0x0F, 0x8E, 0xCF, 0x03, 0x00, 0x00, 0x05, 0x00, 0x2E,
    0xB6, 0x94, 0xA3, 0x54, 0xD0, 0xEB, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0x8B,
    0x4C, 0x24, 0x18, 0x8B, 0x01, 0x85, 0xC0, 0x74, 0x27, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x85,
    0xC0, 0x74, 0x1D, 0x8B, 0x40, 0x08, 0xD9, 0xEE, 0x51, 0xD9, 0x1C, 0x24, 0x8B, 0xD0, 0x8B, 0xCD,
    0xE8, 0x8B, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3,
    0xD9, 0xEE, 0x33, 0xC0, 0x51, 0xD9, 0x1C, 0x24, 0x8B, 0xD0, 0x8B, 0xCD, 0xE8, 0x6F, 0xF9, 0xFF,
    0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0xD9, 0xEE, 0x51, 0x33,
    0xD2, 0xD9, 0x1C, 0x24, 0x33, 0xC9, 0xE8, 0x55, 0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0xB9, 0x5C,
    0xD0, 0xEB, 0x00, 0x89, 0x35, 0x30, 0xA1, 0x52, 0x02, 0xE8, 0x42, 0x4F, 0x91, 0xFF, 0x5F, 0x5E,
    0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0x8B, 0x0D, 0x2C, 0xA1, 0x52, 0x02, 0xE8, 0x6F, 0x79, 0xF7,
    0xFF, 0x8B, 0xE8, 0x3B, 0xEE, 0x0F, 0x84, 0x19, 0x03, 0x00, 0x00, 0x8B, 0xD5, 0xE8, 0xDE, 0x03,
    0xF4, 0xFF, 0x84, 0xC0, 0x0F, 0x85, 0x0A, 0x03, 0x00, 0x00, 0x55, 0xE8, 0xB0, 0xDE, 0xE4, 0xFF,
    0x83, 0xC4, 0x04, 0x84, 0xC0, 0x74, 0x0D, 0xE8, 0xB4, 0x04, 0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x5B,
    0x83, 0xC4, 0x3C, 0xC3, 0xD9, 0x45, 0x14, 0x8D, 0x74, 0x24, 0x28, 0xD8, 0x25, 0x20, 0x5B, 0x5F,
    0x02, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x45, 0x18, 0xD8, 0x25, 0x24, 0x5B, 0x5F, 0x02, 0xD9, 0x5C,
    0x24, 0x2C, 0xD9, 0x45, 0x1C, 0xD8, 0x25, 0x28, 0x5B, 0x5F, 0x02, 0xD9, 0x5C, 0x24, 0x30, 0xE8,
    0x6C, 0x31, 0x07, 0x00, 0xD9, 0x44, 0x24, 0x38, 0xD8, 0x4C, 0x24, 0x2C, 0xD9, 0x44, 0x24, 0x28,
    0xD8, 0x4C, 0x24, 0x34, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x3C, 0xD8, 0x4C, 0x24, 0x30, 0xDE, 0xC1,
    0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0xD1,
    0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0x75, 0x19, 0xD9, 0xE8, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6,
    0xC4, 0x41, 0x75, 0x0E, 0x51, 0xD9, 0x1C, 0x24, 0xE8, 0x03, 0x3F, 0x8C, 0xFF, 0x83, 0xC4, 0x04,
    0xEB, 0x04, 0xDD, 0xD8, 0xD9, 0xEE, 0xD9, 0x5C, 0x24, 0x1C, 0xC6, 0x44, 0x24, 0x17, 0x01, 0xD9,
    0x44, 0x24, 0x1C, 0xD9, 0x83, 0xC0, 0x00, 0x00, 0x00, 0xDC, 0x0D, 0x60, 0xC1, 0xE5, 0x00, 0xDC,
    0x35, 0x48, 0xC3, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x1C, 0xDE, 0xD9, 0xDF,
    0xE0, 0xF6, 0xC4, 0x41, 0x74, 0x05, 0xC6, 0x44, 0x24, 0x17, 0x00, 0x8B, 0x83, 0xAC, 0x00, 0x00,
    0x00, 0x33, 0xFF, 0x3B, 0xC7, 0x89, 0x44, 0x24, 0x20, 0x74, 0x18, 0xB8, 0x54, 0xD0, 0xEB, 0x00,
    0xE8, 0xDB, 0x21, 0x07, 0x00, 0x89, 0x44, 0x24, 0x1C, 0xDB, 0x44, 0x24, 0x1C, 0xDA, 0x74, 0x24,
    0x20, 0xEB, 0x02, 0xD9, 0xEE, 0x39, 0x3D, 0x58, 0xD0, 0xEB, 0x00, 0xD9, 0x5C, 0x24, 0x1C, 0xD9,
    0x83, 0xB8, 0x00, 0x00, 0x00, 0xD8, 0xA3, 0xBC, 0x00, 0x00, 0x00, 0xD8, 0x4C, 0x24, 0x1C, 0xD8,
    0x83, 0xBC, 0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x24, 0x7C, 0x25, 0xB8, 0x58, 0xD0, 0xEB, 0x00,
    0xE8, 0xDB, 0x21, 0x07, 0x00, 0x89, 0x44, 0x24, 0x20, 0xDB, 0x44, 0x24, 0x20, 0xD9, 0x44, 0x24,
    0x24, 0xDC, 0x0D, 0xD8, 0xC0, 0xE5, 0x00, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x44,
    0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x58, 0xD0, 0xEB, 0x00, 0x7E,
    0x0A, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0xA3, 0x58, 0xD0, 0xEB, 0x00, 0x8D, 0x54, 0x24, 0x24, 0x52,
    0x33, 0xC0, 0xB9, 0xEC, 0x97, 0xE4, 0x00, 0xE8, 0x24, 0x1A, 0x07, 0x00, 0x8B, 0xD0, 0xE8, 0x3D,
    0xE9, 0x90, 0xFF, 0x0F, 0xB7, 0xC0, 0x57, 0x50, 0x8D, 0x74, 0x24, 0x3C, 0xE8, 0xAF, 0xAB, 0x8F,
    0xFF, 0x83, 0xC4, 0x08, 0x80, 0x7C, 0x24, 0x17, 0x00, 0x75, 0x0D, 0xE8, 0x10, 0xF6, 0xFF, 0xFF,
    0x3B, 0xC5, 0x0F, 0x85, 0xCA, 0x00, 0x00, 0x00, 0xB8, 0x54, 0xD0, 0xEB, 0x00, 0xE8, 0xEE, 0x20,
    0x07, 0x00, 0x84, 0xC0, 0x0F, 0x84, 0x81, 0x00, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x24, 0x50, 0x33,
    0xC0, 0xB9, 0x00, 0x98, 0xE4, 0x00, 0xE8, 0xD5, 0x19, 0x07, 0x00, 0x8B, 0xD0, 0xE8, 0xEE, 0xE8,
    0x90, 0xFF, 0x0F, 0xB7, 0xC0, 0x57, 0x50, 0x8D, 0x74, 0x24, 0x48, 0xE8, 0x60, 0xAB, 0x8F, 0xFF,
    0x8B, 0x08, 0x89, 0x0D, 0x5C, 0xD0, 0xEB, 0x00, 0x8B, 0x50, 0x04, 0x8B, 0x4C, 0x24, 0x20, 0x89,
    0x15, 0x60, 0xD0, 0xEB, 0x00, 0x8B, 0x40, 0x08, 0xA3, 0x64, 0xD0, 0xEB, 0x00, 0x8B, 0x01, 0x83,
    0xC4, 0x08, 0x3B, 0xC7, 0x74, 0x0F, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x3B, 0xC7, 0x74, 0x05,
    0x8B, 0x40, 0x08, 0xEB, 0x02, 0x33, 0xC0, 0xD9, 0xEE, 0x51, 0x8B, 0xD0, 0xD9, 0x1C, 0x24, 0x8B,
    0xCD, 0xE8, 0xFA, 0xF6, 0xFF, 0xFF, 0xC7, 0x05, 0x30, 0xA1, 0x52, 0x02, 0x02, 0x00, 0x00, 0x00,
    0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0x8B, 0x54, 0x24, 0x18, 0x8B,
    0x02, 0x3B, 0xC7, 0x74, 0x0F, 0x8B, 0x80, 0xC4, 0x00, 0x00, 0x00, 0x3B, 0xC7, 0x74, 0x05, 0x8B,
    0x40, 0x08, 0xEB, 0x02, 0x33, 0xC0, 0xD9, 0x44, 0x24, 0x1C, 0x51, 0x8B, 0xD0, 0xD9, 0x1C, 0x24,
    0x8B, 0xCD, 0xE8, 0xB9, 0xF6, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4,
    0x3C, 0xC3, 0x3B, 0xC7, 0x74, 0x20, 0xD9, 0xEE, 0x51, 0x33, 0xD2, 0xD9, 0x1C, 0x24, 0x33, 0xC9,
    0xE8, 0x9B, 0xF6, 0xFF, 0xFF, 0x89, 0x3D, 0x30, 0xA1, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x5F, 0x5E,
    0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0xD9, 0x05, 0x7C, 0x43, 0xE8, 0x00, 0xDC, 0x0D, 0xD8, 0xC0,
    0xE5, 0x00, 0xDC, 0xC0, 0xE8, 0x07, 0x4F, 0x23, 0x00, 0x50, 0xBE, 0x54, 0xD0, 0xEB, 0x00, 0xE8,
    0xEC, 0x1E, 0x07, 0x00, 0x8B, 0xC6, 0xE8, 0x15, 0x20, 0x07, 0x00, 0x3B, 0x83, 0xAC, 0x00, 0x00,
    0x00, 0x7F, 0xB3, 0x8B, 0x44, 0x24, 0x18, 0x8B, 0x00, 0xE9, 0x73, 0xFF, 0xFF, 0xFF, 0xE8, 0xDD,
    0xF4, 0xFF, 0xFF, 0x3B, 0xC6, 0x74, 0x55, 0x8B, 0x48, 0x44, 0x8B, 0x83, 0xAC, 0x00, 0x00, 0x00,
    0x89, 0x0D, 0x2C, 0xA1, 0x52, 0x02, 0xB9, 0x54, 0xD0, 0xEB, 0x00, 0xE8, 0x80, 0x1E, 0x07, 0x00,
    0xC7, 0x05, 0x30, 0xA1, 0x52, 0x02, 0x01, 0x00, 0x00, 0x00, 0xC7, 0x05, 0x58, 0xD0, 0xEB, 0x00,
    0xFF, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x3C, 0xC3, 0x39, 0x35, 0x30, 0xA1,
    0x52, 0x02, 0x74, 0x18, 0xD9, 0xEE, 0x51, 0x33, 0xD2, 0xD9, 0x1C, 0x24, 0x33, 0xC9, 0xE8, 0xFD,
    0xF5, 0xFF, 0xFF, 0x89, 0x35, 0x30, 0xA1, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5D, 0x5B,
    0x83, 0xC4, 0x3C, 0xC3,
];
#[allow(dead_code)]
const WTL_BODY_CALLS: [usize; 38] = [0x4c, 0x8e, 0x9f, 0xad, 0xbd, 0xf9, 0x142, 0x17b, 0x193, 0x1e0, 0x1fc, 0x216, 0x229, 0x23c, 0x24d, 0x25b, 0x267, 0x29f, 0x2e8, 0x340, 0x380, 0x3c7, 0x3ce, 0x3dc, 0x3eb, 0x3fd, 0x416, 0x41d, 0x42b, 0x471, 0x4b2, 0x4d0, 0x4f4, 0x4ff, 0x506, 0x51e, 0x53b, 0x56e];
#[allow(dead_code)]
const WTL_BODY_DISPS: [usize; 48] = [0xa, 0x18, 0x6a, 0x8a, 0xd7, 0xe4, 0xf1, 0x120, 0x160, 0x166, 0x185, 0x18b, 0x199, 0x1a3, 0x1b3, 0x21f, 0x225, 0x238, 0x27d, 0x28a, 0x297, 0x2c6, 0x30b, 0x311, 0x33c, 0x357, 0x37c, 0x393, 0x3a1, 0x3ab, 0x3b7, 0x3c3, 0x3f9, 0x412, 0x434, 0x441, 0x449, 0x478, 0x4d7, 0x4e8, 0x4ee, 0x4fb, 0x532, 0x537, 0x542, 0x54c, 0x55e, 0x575];

// ===== Offline differential test kit (copied from port_frame2.rs, with extra stub effects) =====
// Run: RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release -p sr2-hook --target i686-pc-windows-msvc port_frame3 -- --ignored --test-threads=1
// Offline differential test kit: the original machine code and the Rust port run on the same
// random world; every call into the rest of the game goes through a recording stub.
// (Copied from port_activities.rs, with a few extra stub effects.)
// =================================================================================================
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc port_frame2 -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod difftest {
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
        /// EAX comes back unchanged (the /GS cookie check).
        Keep,
        /// EAX comes back as the ECX the callee got.
        Ecx,
        /// EAX comes back as the ESI the callee got.
        Esi,
        /// EAX comes back as the first stack argument.
        Arg0,
        /// A millisecond count: mostly 0..5000, sometimes negative or huge.
        Ms,
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
        /// Removes one handle record from the player's pending list (args: player, object).
        RemoveListEntry,
        /// Writes random floats through the two pointer arguments (3 and 9 dwords).
        FillOut2,
        /// Logs 0x5C bytes at [EAX] and writes three random dwords at [ECX].
        Vec465,
        /// Writes three random dwords at [ESI].
        Out3Esi,
        /// Writes three random floats through the first stack argument.
        WriteVecArg0,
        /// Writes nine random floats at [EAX].
        Fill9Eax,
        /// 0x9A1970's outputs: three floats at each of the two pointers, a flag byte at the third.
        Fill1970,
        /// Logs what 0x9A2670 reads: 3 dwords at each of the first two pointers, the byte at the third.
        Log2670,
        /// Logs the dword at [ESI].
        LogEsi1,
        /// Logs three dwords at [ESI], then rewrites them as a (mostly unit-length) vector.
        LogNormEsi,
        /// Writes a (mostly unit-length) vector through the first stack argument.
        UnitArg0,
        /// Writes a random dword through the first stack argument (a hash).
        HashArg0,
        /// Writes three random dwords at [ESI] (an audio handle).
        Fill3Esi,
        /// Zeroes the 0x5C bytes at [ESI] (the audio params builders).
        ZeroParamsEsi,
        /// Logs three dwords at [EDI].
        LogEdi3,
        /// 0xBE2F50: logs 9 dwords at [ECX] and at the first stack argument, writes 9 floats at [EAX].
        MatMul,
        /// 0xBDDE20: logs 3 dwords at the first stack argument and 9 at [ECX], writes 3 floats at [EAX].
        VecMat,
        /// 0x4171C0: logs 3 dwords at [EDX] and 9 at [ECX], builds the 0x6C-byte record at the first
        /// stack argument as the original does.
        DamageRec,
        /// 0x50AA60: logs 3 dwords at [EDX] and 9 at the second stack argument.
        ObjUpd,
        /// Writes a random 0 / 1 byte at the pointer in the first stack argument.
        FlagArg0,
        /// Writes a random 0 / 1 byte at +0x139C of the player passed as the first stack argument.
        PlayerFlag,
        /// Sets the state dword (+0x3C) of the activity [0x27B3C60] to a random value of 0, 1, 3, 5.
        SetActState,
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
        /// Registers the callee reads as 16 bits.
        pub word_regs: u8,
        /// A native replacement the original's call is aimed at (0: the recording stub).
        pub native: usize,
    }

    pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
        Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, word_regs: 0, native: 0 }
    }

    /// Prints what already occupies the address space the game's globals need.
    pub fn dump_occupied() {
        use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, MEM_FREE, VirtualQuery};
        let mut a = 0x40_0000usize;
        while a < 0x400_0000 {
            let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { VirtualQuery(a as *const c_void, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                break;
            }
            if m.State != MEM_FREE {
                eprintln!("occupied {:#x}..{:#x} state {:#x} type {:#x}", m.BaseAddress as usize, m.BaseAddress as usize + m.RegionSize, m.State, m.Type);
            }
            a = m.BaseAddress as usize + m.RegionSize;
        }
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
            let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::tr(a), l)).collect();
            for i in 0..body.len().saturating_sub(3) {
                let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                if (0xD0_0000..0x350_0000).contains(&v) {
                    spans.push((super::tr(v), 1));
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
            unsafe { std::ptr::write_bytes(super::tr(a) as *mut u8, 0, l) };
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
                Some(i) if c[i].native != 0 => c[i].native,
                Some(i) => STUBS.with(|s| s.borrow()[i]),
                None => f,
            }
        })
    }

    /// Adds a line to the recorded call log (for natives that stand in for a callee).
    pub fn push_log(e: Vec<u32>) {
        LOG.with(|l| l.borrow_mut().push(e));
    }

    /// The next number of the stubs' random stream.
    pub fn rand_u32() -> u32 {
        srng()
    }

    fn srng() -> u32 {
        SRNG.with(|s| {
            let mut r = Rng(s.get());
            let v = r.u32();
            s.set(r.0);
            v
        })
    }

    extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
        let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
        let r = unsafe { &*regs };
        if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("stub {:#x} eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} a0 {:#x} a1 {:#x}", c.target, r.eax, r.ecx, r.edx, r.esi, r.edi, unsafe { *args }, unsafe { *args.add(1) });
        }
        // Pointers into the stack differ between the two runs: log a marker for them.
        let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
        if !c.silent {
            let mut e = vec![c.target as u32];
            for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else if c.word_regs & bit != 0 { v & 0xFFFF } else { mask(v) });
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
                Fx::LogEsi1 => e.push(unsafe { *(r.esi as usize as *const u32) }),
                Fx::MatMul | Fx::VecMat | Fx::DamageRec | Fx::ObjUpd => {
                    let rd = |p: usize, n: usize, e: &mut Vec<u32>| {
                        for k in 0..n {
                            e.push(unsafe { *((p + k * 4) as *const u32) });
                        }
                    };
                    let a0 = unsafe { *args } as usize;
                    let a1 = unsafe { *args.add(1) } as usize;
                    match c.fx {
                        Fx::MatMul => {
                            rd(r.ecx as usize, 9, &mut e);
                            rd(a0, 9, &mut e);
                        }
                        Fx::VecMat => {
                            rd(a0, 3, &mut e);
                            rd(r.ecx as usize, 9, &mut e);
                        }
                        Fx::DamageRec => {
                            rd(r.edx as usize, 3, &mut e);
                            rd(r.ecx as usize, 9, &mut e);
                        }
                        _ => {
                            rd(r.edx as usize, 3, &mut e);
                            rd(a1, 9, &mut e);
                        }
                    }
                }
                Fx::LogEdi3 => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.edi as usize + k * 4) as *const u32) });
                    }
                }
                Fx::LogNormEsi => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.esi as usize + k * 4) as *const u32) });
                    }
                }
                Fx::Log2670 => {
                    let (p0, p1, p2) = unsafe { (*args as usize, *args.add(1) as usize, *args.add(2) as usize) };
                    for k in 0..3usize {
                        e.push(unsafe { *((p0 + k * 4) as *const u32) });
                        e.push(unsafe { *((p1 + k * 4) as *const u32) });
                    }
                    e.push(unsafe { *(p2 as *const u8) } as u32);
                }
                Fx::Vec465 => {
                    for k in 0..0x5Cusize {
                        e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
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
            if c.fx == Fx::LogBuf {
                let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                assert!(len < 64, "LogBuf: callee {:#x} len {len:#x} ptr {p:#x}", c.target);
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
        if c.fx == Fx::RemoveListEntry {
            unsafe {
                let p = *args as usize;
                let o = *args.add(1);
                let cnt = *((p + 0xC3C) as *const u32) as usize;
                if cnt > 0 && cnt < 64 {
                    let obj_of = |h: u32| -> u32 {
                        if h == 0 {
                            return 0;
                        }
                        let e = *(super::tr(0x0214_9C64 + (((h & 0xFFF) as usize) << 4)) as *const u32);
                        if e != 0 && *((e as usize + 0x44) as *const u32) == h { e } else { 0 }
                    };
                    let hk = |k: usize| *((p + 0xC40 + k * 0x18) as *const u32);
                    let mut sel = None;
                    if o != 0 {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == o);
                    }
                    if sel.is_none() {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == 0);
                    }
                    let j = sel.unwrap_or(0);
                    for k in j..cnt - 1 {
                        std::ptr::copy((p + 0xC40 + (k + 1) * 0x18) as *const u8, (p + 0xC40 + k * 0x18) as *mut u8, 0x18);
                    }
                    *((p + 0xC3C) as *mut u32) = (cnt - 1) as u32;
                }
            }
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
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::FillOut2 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let (p0, p1) = (*args as usize, *args.add(1) as usize);
                for k in 0..3 {
                    *((p0 + k * 4) as *mut u32) = rng.float_bits();
                }
                for k in 0..9 {
                    *((p1 + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Vec465 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.ecx as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Fill9Eax => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..9 {
                    *((r.eax as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Fill1970 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let (p1, p2, p3) = (*args.add(1) as usize, *args.add(2) as usize, *args.add(3) as usize);
                for k in 0..3 {
                    *((p1 + k * 4) as *mut u32) = rng.float_bits();
                    *((p2 + k * 4) as *mut u32) = rng.float_bits();
                }
                *(p3 as *mut u8) = rng.pick(&[0u8, 1, 1, 2]);
            },
            Fx::WriteVecArg0 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let p0 = *args as usize;
                for k in 0..3 {
                    *((p0 + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::Out3Esi => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.esi as usize + k * 4) as *mut u32) = rng.float_bits();
                }
            },
            Fx::LogNormEsi | Fx::UnitArg0 => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let p = if c.fx == Fx::UnitArg0 { *args as usize } else { r.esi as usize };
                let unit = rng.below(4) != 0;
                for k in 0..3 {
                    *((p + k * 4) as *mut u32) = if unit { (rng.below(2001) as f32 / 1000.0 - 1.0).to_bits() } else { rng.float_bits() };
                }
            },
            Fx::HashArg0 => unsafe {
                *(*args as usize as *mut u32) = srng();
            },
            Fx::MatMul | Fx::VecMat => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                let n = if c.fx == Fx::MatMul { 9 } else { 3 };
                for k in 0..n {
                    *((r.eax as usize + k * 4) as *mut u32) =
                        if rng.below(4) != 0 { (rng.below(4001) as f32 / 1000.0 - 2.0).to_bits() } else { rng.float_bits() };
                }
            },
            Fx::DamageRec => unsafe {
                let p = *args as usize;
                let w = |o: usize, v: u32| *((p + o) as *mut u32) = v;
                let wb8 = |o: usize, v: u8| *((p + o) as *mut u8) = v;
                w(0, r.esi);
                for k in 0..3 {
                    w(4 + k * 4, *((r.edx as usize + k * 4) as *const u32));
                }
                for k in 0..9 {
                    w(0x10 + k * 4, *((r.ecx as usize + k * 4) as *const u32));
                }
                wb8(0x34, 0);
                w(0x38, 0);
                w(0x50, 1.0f32.to_bits());
                w(0x3C, 0xFFFF_FFFF);
                w(0x68, 1.0f32.to_bits());
                for o in [0x40usize, 0x41, 0x4A, 0x4B, 0x4C, 0x4D, 0x54, 0x55] {
                    wb8(o, 0);
                }
                w(0x44, 0);
                *((p + 0x48) as *mut u16) = 0xFFFF;
                w(0x58, 1);
                w(0x5C, 0);
                w(0x60, 0);
                w(0x64, 0);
            },
            Fx::PlayerFlag => unsafe {
                *((*args as usize + 0x139C) as *mut u8) = (srng() % 2) as u8;
            },
            Fx::FlagArg0 => unsafe {
                *(*args as usize as *mut u8) = (srng() % 2) as u8;
            },
            Fx::SetActState => unsafe {
                let act = *(super::tr(0x027B_3C60) as *const u32) as usize;
                if act != 0 {
                    *((act + 0x3C) as *mut u32) = [0u32, 1, 3, 5, 1][srng() as usize % 5];
                }
            },
            Fx::ZeroParamsEsi => unsafe {
                std::ptr::write_bytes(r.esi as usize as *mut u8, 0, 0x5C);
            },
            Fx::Fill3Esi => unsafe {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                for k in 0..3 {
                    *((r.esi as usize + k * 4) as *mut u32) = rng.u32();
                }
            },
            Fx::WriteFloat => {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = rng.float_bits() };
            }
            _ => {}
        }
        match c.ret {
            Ret::Zero => 0,
            Ret::Mod(n) => srng() % n,
            Ret::Keep => r.eax,
            Ret::Ecx => r.ecx,
            Ret::Esi => r.esi,
            Ret::Arg0 => unsafe { *args },
            Ret::Ms => match srng() % 8 {
                0 => 0,
                1 => srng(),
                2 => (srng() % 100) as u32 | 0xFFFF_FF00,
                _ => srng() % 5000,
            },
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
    pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
                let common = if c.ret == Ret::St0 { stub_common_st0 as *const () as usize } else { stub_common as *const () as usize };
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
            unsafe { std::ptr::write_bytes((STACK + STACK_LEN - 0x1_0000) as *mut u8, 0, 0x1_0000) };
            // a clean x87 (53-bit precision, as the game runs)
            unsafe { std::arch::asm!("fninit", "fldcw word ptr [{cw}]", cw = in(reg) &0x027Fu16 as *const u16, options(nostack)) };
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
mod cfu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x004A_AC40;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0xbe2f50, 1, 4, EAX | ECX, Ret::Keep, Fx::MatMul),
            callee(0xbdde20, 1, 4, EAX | ECX, Ret::Keep, Fx::VecMat),
            callee(0x50aa60, 2, 0, ECX | EDX, Ret::Any, Fx::ObjUpd),
            callee(0x4171c0, 1, 4, ESI | ECX | EDX, Ret::Any, Fx::DamageRec),
            callee(0x50e330, 1, 0, 0, Ret::Ptr(0), Fx::LogArg0(27)),
            callee(0x50ab60, 2, 0, EDI, Ret::Any, Fx::None),
        ]
    }

    fn nice(rng: &mut Rng, lo: f32, hi: f32) -> u32 {
        if rng.one_in(12) {
            return rng.float_bits();
        }
        (lo + (hi - lo) * (rng.below(10001) as f32 / 10000.0)).to_bits()
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let chunks = a.pool(3, 0x200);
            let recs = a.pool(3, 0x50 * 4);
            let defs = a.pool(3, 0x50 * 4);
            let defobjs = a.pool(6, 0x200);
            let classes = a.pool(6, 0x80);
            let objs = a.pool(8, 0xE0);
            let nodes = a.pool(16, 0x20);
            let handles: Vec<u32> = (0..8).map(|k| (0x100 + k * 7) | (k + 1) << 12).collect();
            set_pools(vec![vec![0x1234_5001, 0x2000_0002, 0x3000_0003, 0x5000_0007]]);
            // kinds, camera, time of day, constants
            let nkinds = 1 + rng.below(5);
            wd(0x0252_7288, nkinds);
            for k in 0..6usize {
                let b = 0x026B_5DE0 + k * 0x30;
                wd(b + 0x10, nice(rng, 5.0, 2000.0));
                wd(b + 0x14, nice(rng, 5.0, 2000.0));
                wd(b + 0x2C, rng.u32());
                wd(b + 0x28, rng.u32());
            }
            for k in 0..3 {
                wd(0x025F_5B14 + k * 4, nice(rng, -20.0, 20.0));
            }
            wb(0x0252_7D14, rng.pick(&[0, 1]));
            wb(0x0252_7CF7, rng.pick(&[0, 1]));
            wb(0x0252_7D15, rng.pick(&[0, 1]));
            let t = rng.u32();
            wd(0x0140_E734, t);
            wd(0x00E5_BFD8, 4294967296.0f32.to_bits());
            let sets: [(usize, f64); 3] = [(0x00E5_C030, 2.3283064365386963e-10), (0x00E5_C1A0, 9.999999747378752e-05), (0x00E5_D868, 1.44)];
            for (addr, v) in sets {
                let b = v.to_bits();
                wd(addr, b as u32);
                wd(addr + 4, (b >> 32) as u32);
            }
            // objects
            for k in 0..8 {
                let o = objs[k];
                put(o, 0x44, handles[k]);
                put(o, 0x4C, if rng.one_in(8) { 5 } else { 6 });
                put(o, 0xC4, if rng.one_in(4) { 0 } else { nodes[2 * k] });
                // a short circular list: node -> node (or two nodes)
                let n = nodes[2 * k];
                let m = nodes[2 * k + 1];
                if rng.one_in(2) {
                    put(n, 0, n);
                } else {
                    put(n, 0, m);
                    put(m, 0, n);
                }
                wd(0x0214_9C64 + (((handles[k] & 0xFFF) as usize) << 4), if rng.one_in(10) { 0 } else { o });
            }
            // definitions
            for k in 0..6 {
                let d = defobjs[k];
                for q in 0..3 {
                    put(d, 0x14 + q * 4, nice(rng, -20.0, 20.0));
                }
                for q in 0..9 {
                    put(d, 0x20 + q * 4, nice(rng, -1.0, 1.0));
                }
                put(d, 0x130, classes[k]);
                put(classes[k], 0x64, rng.pick(&[0, 0x2000_0000, 0x2000_0001, 1, 0x2000_0000]));
            }
            let nch = rng.pick(&[0u32, 1, 2, 3, 3, 0xFFFF_FFFF]);
            wd(0x00F7_1B58, nch);
            for c in 0..3 {
                wd(0x00F7_0BA8 + c * 4, chunks[c]);
                let ch = chunks[c];
                put(ch, 8, rng.u32());
                put(ch, 0x180, rng.pick(&[0, 1, 2, 3, 4, 4, 0xFFFF_FFFF]));
                put(ch, 0x184, recs[c]);
                put(ch, 0x4C, defs[c]);
                for q in 0..4usize {
                    let d = defs[c] as usize + q * 0x50;
                    let pick = rng.below(7) as usize;
                    put(d as u32, 0xC, if rng.one_in(8) { 0 } else { defobjs[pick % 6] });
                    put(d as u32, 0x30, if rng.one_in(5) { rng.u32() } else { rng.u32() & !0x4000_00E0 });
                }
                for q in 0..4usize {
                    let r = recs[c] + (q * 0x50) as u32;
                    let hh = match rng.below(5) {
                        0 | 1 => 0,
                        2 => {
                            let hv = rng.u32();
                            let e = if rng.one_in(2) { 0 } else { objs[rng.below(8) as usize] };
                            wd(0x0214_9C64 + (((hv & 0xFFF) as usize) << 4), e);
                            hv
                        }
                        _ => handles[rng.below(8) as usize],
                    };
                    put(r, 0, hh);
                    put(r, 8, rng.pick(&[0xFFFF_FFFF, 0, 1, 2, nkinds - 1, nkinds, 0xFFFF_FFFB, 1, 0]));
                    putb(r, 0xC, rng.pick(&[1, 1, 1, 1, 0, 2]));
                    for qq in 0..3 {
                        put(r, 0x10 + qq * 4, nice(rng, -20.0, 20.0));
                    }
                    for qq in 0..9 {
                        put(r, 0x1C + qq * 4, nice(rng, -1.0, 1.0));
                    }
                    let (wa, wb2) = match rng.below(5) {
                        0 => (0.0f32, 0.0f32),
                        1 => (0.2, 0.7),
                        2 => (0.8, 0.3),
                        3 => (0.5, 0.50001),
                        _ => (f32::from_bits(nice(rng, 0.0, 1.0)), f32::from_bits(nice(rng, 0.0, 1.0))),
                    };
                    put(r, 0x40, wa.to_bits());
                    put(r, 0x44, wb2.to_bits());
                    put(r, 0x48, rng.pick(&[0xFFFF_FFFF, 0xFFFF_FFFF, 0, 1, 2, 3, 0xFFFF_FFFE]));
                    putb(r, 0x4C, rng.u32() as u8);
                    // some records are already listed by their object
                    if rng.one_in(3) {
                        let k = rng.below(8) as usize;
                        put(nodes[2 * k + rng.below(2) as usize], 0xC, r);
                        put(r, 0, handles[k]);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn chunk_effects_update_matches_original() {
        let _g = lock();
        let extra = [(0x0214_9C64usize, 0x10000), (0x026B_5DE0usize, 0x400)];
        let h = Harness::new(ADDR, &CFU_BODY, &CFU_BODY_CALLS, &CFU_BODY_DISPS, callees(), &extra, chunk_effects_thunk as *const () as usize);
        let mut rng = Rng(0x2828_3939_4A4A_5B5B);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in CE_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, CE_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod vsu_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x0048_9CC0;
    const CB: usize = 0xCB00_0001;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x47c740, 0, 0, EAX | EDX, Ret::Any, Fx::None),
            callee(0xbdcf50, 0, 0, ESI | ECX, Ret::Ms, Fx::None),
            callee(0x9b8e50, 1, 0, EDI, Ret::Bool, Fx::None),
            callee(0x465f70, 1, 0, ECX, Ret::Any, Fx::None),
            callee(0x47ffc0, 0, 0, ECX | EDX, Ret::Bool, Fx::None),
            callee(0x465fd0, 1, 0, ECX, Ret::Any, Fx::None),
            callee(0x4660b0, 0, 0, ECX, Ret::Bool, Fx::None),
            callee(0xbdd260, 0, 0, EAX, Ret::Ms, Fx::None),
            callee(0x4660f0, 1, 0, ESI, Ret::Any, Fx::None),
            callee(0x4802d0, 2, 0, ECX | EDI, Ret::Bool, Fx::LogEdi3),
            callee(0x466010, 1, 0, EDX, Ret::Any, Fx::None),
            callee(0x489190, 0, 0, EAX, Ret::Any, Fx::None),
            callee(0x4892e0, 0, 0, ECX, Ret::Any, Fx::None),
            callee(0x464970, 4, 0x10, ESI, Ret::Esi, Fx::ZeroParamsEsi),
            callee(0x464ca0, 4, 0x10, ESI | EDI, Ret::Esi, Fx::ZeroParamsEsi),
            callee(0x465b90, 0, 0, EAX | ECX, Ret::Ecx, Fx::LogEaxBytes(0x5C)),
            callee(CB, 2, 8, ECX, Ret::Any, Fx::None),
        ]
    }

    fn flt(rng: &mut Rng, lo: f32, hi: f32) -> u32 {
        if rng.one_in(10) {
            return rng.float_bits();
        }
        (lo + (hi - lo) * (rng.below(10001) as f32 / 10000.0)).to_bits()
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let veh = a.pool(1, 0x9000)[0];
            let vt = a.pool(1, 0x40)[0];
            set_pools(vec![]);
            put(vt, 0x18, redirect(CB) as u32);
            for off in [0x6DA4usize, 0x6DD0, 0x6E58, 0x6DF8, 0x6E28] {
                put(veh, off, vt);
            }
            let idx = rng.below(0x300);
            let v = idx | rng.below(8) << 12;
            put(veh, 0x44, v);
            put(veh, 0x4C, if rng.one_in(8) { 6 } else { 5 });
            for off in [0x14usize, 0x18, 0x1C, 0x50, 0x54, 0x58] {
                put(veh, off, flt(rng, -30.0, 30.0));
            }
            putb(veh, 0x84D8, rng.pick(&[0, 1, 1, 1, 2]));
            put(veh, 0x8D88, rng.u32() & !0x800 | rng.pick(&[0, 0x800, 0x800]));
            put(veh, 0x8F50, rng.u32() & !1 | rng.pick(&[0, 0, 0, 0, 1]));
            wd(0x0252_6B74, if rng.one_in(10) { 0 } else { v });
            wd(0x0214_9C64 + ((idx as usize) << 4), if rng.one_in(10) { 0 } else { veh });
            if rng.one_in(10) {
                put(veh, 0x44, rng.u32());
            }
            // the voice
            let slot = rng.below(40);
            let vgen = 1 + rng.below(0xFFFF);
            let hd = match rng.below(12) {
                0 => slot | 0xFF,
                1 => slot,
                2 => 0xFF,
                3 => slot | (vgen + 1) << 8,
                _ => slot | vgen << 8,
            };
            wd(0x00E9_A4A0, hd);
            wd(0x00EE_B9AC + slot as usize * 0x19C, vgen);
            wd(0x00EE_B8F8 + slot as usize * 0x19C, if rng.one_in(8) { 0xFFFF_FFFF } else { 5 });
            // time
            let r = rng.below(0x6B49_D200);
            let now = rng.pick(&[5000, 0x3000_0000, 0x6B00_0000, 0x6B49_D200, r]) as i32;
            wd(0x00E8_43AC, now as u32);
            let r = rng.u32();
            let t1 = rng.pick(&[0xFFFF_FFFF, now.wrapping_add(1000) as u32, now.wrapping_sub(1000) as u32, now.wrapping_add(0x4000_0000) as u32, now.wrapping_sub(0x4000_0000) as u32, 0, now.wrapping_add(6000) as u32, now.wrapping_add(0x35A4_E900) as u32, r]);
            wd(0x00E9_A4AC, t1);
            wd(0x00E9_A4B0, rng.u32());
            wd(0x00EE_3478, rng.below(30));
            wd(0x00EE_347A, rng.below(30) << 16 | rng.below(0x10000));
            wd(0x00E9_9B94, rng.u32());
            wd(0x00EE_347C, flt(rng, 0.0, 1.5));
            wd(0x00EE_3480, flt(rng, 0.0, 1.5));
            wd(0x00E9_9B98, flt(rng, 0.0, 1.5));
            wd(0x00E8_437C, flt(rng, 0.0, 0.1));
            wd(0x00E5_BF70, 0);
            wd(0x00E5_BF74, 0x3FE0_0000);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn vehicle_siren_update_matches_original() {
        let _g = lock();
        let extra = [(0x00EE_B8F8usize, 0x19C * 64 + 0x100), (0x0214_9C64usize, 0x10000)];
        let h = Harness::new(ADDR, &VSU_BODY, &VSU_BODY_CALLS, &VSU_BODY_DISPS, callees(), &extra, vehicle_siren_thunk as *const () as usize);
        let mut rng = Rng(0x1717_2828_3939_4A4A);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in VS_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, VS_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod wtl_test {
    use super::difftest::*;
    use super::*;

    const ADDR: usize = 0x00B6_AD40;

    /// `_ftol`: records the double on ST0 and answers with a random int.
    extern "C" fn ftol_log(v: f64) -> u32 {
        let b = v.to_bits();
        push_log(vec![0xDA0140, b as u32, (b >> 32) as u32]);
        match rand_u32() % 5 {
            0 => 0,
            1 => rand_u32() % 10,
            2 => rand_u32() % 3000,
            3 => 0x8000_0000,
            _ => rand_u32(),
        }
    }
    #[unsafe(naked)]
    extern "C" fn ftol_native() {
        std::arch::naked_asm!("sub esp, 8", "fstp qword ptr [esp]", "call {f}", "add esp, 8", "ret", f = sym ftol_log)
    }

    fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0x9c5fd0, 1, 0, EAX, Ret::Any, Fx::UnitArg0),
            callee(0xae28f0, 0, 0, ECX, Ret::Ptr(0), Fx::None),
            callee(0xaab370, 0, 0, EDX, Ret::Bool, Fx::None),
            callee(0x9b8e50, 1, 0, EDI, Ret::Bool, Fx::None),
            callee(0xb6a740, 0, 0, 0, Ret::Ptr(0), Fx::None),
            callee(0xbde150, 0, 0, ESI, Ret::Any, Fx::LogNormEsi),
            callee(0x42ef30, 1, 0, 0, Ret::St0, Fx::None),
            callee(0xb6a8b0, 1, 0, ECX | EDX, Ret::Any, Fx::None),
            callee(0x47feb0, 0, 0, ECX, Ret::Any, Fx::None),
            callee(0xbdd260, 0, 0, EAX, Ret::Ms, Fx::None),
            callee(0xbdd2a0, 0, 0, EAX, Ret::Ms, Fx::None),
            callee(0xbdd230, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0xbdd100, 0, 0, EAX | ECX, Ret::Any, Fx::None),
            callee(0xbdd130, 1, 4, ESI, Ret::Any, Fx::None),
            callee(0xbdcb30, 1, 4, EAX | ECX, Ret::Arg0, Fx::HashArg0),
            callee(0x479a50, 0, 0, EDX, Ret::Any, Fx::None),
            callee(0x465cd0, 2, 0, ESI, Ret::Esi, Fx::Fill3Esi),
            callee(0xb6b460, 0, 0, 0, Ret::Any, Fx::None),
            callee(0xda0140, 0, 0, 0, Ret::Any, Fx::None),
        ];
        let n = v.len();
        v[n - 1].native = ftol_native as *const () as usize;
        v[n - 1].silent = true;
        v
    }

    fn fl(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (rng.below(10001) as f32 / 10000.0)
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let player = a.pool(1, 0x1200)[0];
            let obj = a.pool(1, 0x100)[0];
            let w = a.pool(1, 0x20)[0];
            let wi = a.pool(1, 0x100)[0];
            let vehs = a.pool(4, 0x80);
            set_pools(vec![vehs.clone()]);
            for &v in &vehs {
                for k in 0..3 {
                    put(v, 0x14 + k * 4, fl(rng, -50.0, 50.0).to_bits());
                }
            }
            wb(0x00E8_6E8B, if rng.one_in(10) { 0 } else { 1 });
            wd(0x0217_03D4, if rng.one_in(12) { 0 } else { player });
            put(player, 0x10CC, if rng.one_in(12) { 0 } else { obj });
            put(obj, 0xC4, if rng.one_in(12) { 0 } else { w });
            put(w, 8, if rng.one_in(12) { 0 } else { wi });
            put(wi, 0xC, if rng.one_in(8) { rng.u32() } else { 0x400 | rng.u32() & 0xFF });
            put(wi, 0xC0, fl(rng, 5.0, 90.0).to_bits());
            put(wi, 0xC4, fl(rng, 5.0, 90.0).to_bits());
            put(wi, 0xAC, rng.pick(&[0, 0, 500, 1500, 3000, 100, 0xFFFF_FF00]));
            put(wi, 0xB8, fl(rng, 0.0, 800.0).to_bits());
            put(wi, 0xBC, fl(rng, 0.0, 800.0).to_bits());
            wd(0x0252_A130, rng.pick(&[0, 1, 1, 1, 2, 2, 2, 3]));
            wd(0x0252_A12C, rng.u32());
            let r = rng.u32();
            wd(0x00EB_D054, rng.pick(&[0xFFFF_FFFF, 0, 1000, 0x6B49_D200, r]));
            let r = rng.u32();
            wd(0x00EB_D058, rng.pick(&[0xFFFF_FFFF, 0, 1000, 0x7FFF_FFFF, r, 5000]));
            let r = rng.u32();
            wd(0x00E8_43AC, rng.pick(&[1000, 0x6B49_D200, 0x6B49_D201, 0x6000_0000, 0x7000_0000, r]));
            wf(0x00E8_437C, rng.pick(&[0.016f32, 0.033, 0.0, 1.0, 5.0, -0.5]));
            for k in 0..3 {
                wf(0x025F_5B20 + k * 4, fl(rng, -50.0, 50.0));
            }
            if !rng.one_in(8) {
                wf(0x00E8_7998, -1.0);
            }
            let (k1, k2) = (std::f64::consts::PI as f32 as f64, 180.0f64);
            if !rng.one_in(8) {
                let b = k1.to_bits();
                wd(0x00E5_C160, b as u32);
                wd(0x00E5_C164, (b >> 32) as u32);
                let b = k2.to_bits();
                wd(0x00E5_C348, b as u32);
                wd(0x00E5_C34C, (b >> 32) as u32);
            }
            let b = 1000.0f64.to_bits();
            wd(0x00E5_C0D8, b as u32);
            wd(0x00E5_C0DC, (b >> 32) as u32);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn weapon_target_lock_update_matches_original() {
        let _g = lock();
        let h = Harness::new(ADDR, &WTL_BODY, &WTL_BODY_CALLS, &WTL_BODY_DISPS, callees(), &[], weapon_target_lock_thunk as *const () as usize);
        let mut rng = Rng(0x7171_8282_9393_A4A4);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in WT_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, WT_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod nfu_test {
    use super::difftest::*;
    use super::*;

    const CBI: usize = 0xCB00_0100;

    fn callees() -> Vec<Callee> {
        let mut v: Vec<Callee> = [0x7fc930usize, 0x7fa280, 0x817ba0, 0x8312a0, 0x80af30, 0x82e210, 0x7f8390, 0x801880, 0x4017c0, 0x814bb0]
            .iter()
            .map(|&a| callee(a, 0, 0, 0, Ret::Any, Fx::None))
            .collect();
        v.push(callee(0x80fcf0, 1, 4, 0, Ret::Bool, Fx::None));
        v.push(callee(CBI, 1, 4, 0, Ret::Any, Fx::None));
        v
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let sess = a.pool(1, 0x40)[0];
            set_pools(vec![]);
            wb(0x0252_8B48, rng.pick(&[0, 1, 1, 1]));
            wd(0x0252_8C14, if rng.one_in(5) { 0 } else { sess });
            put(sess, 0x1C, rng.pick(&[1, 6, 8, 2, 0, 5, 7, 1, 6, 8]));
            wd(0x0252_8B5C, if rng.one_in(2) { 0 } else { rng.u32() });
            wd(0x00DB_80B8, redirect(CBI) as u32);
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn net_frame_update_matches_original() {
        let _g = lock();
        let h = Harness::new(0x007F_7910, &NFU_BODY, &NFU_BODY_CALLS, &NFU_BODY_DISPS, callees(), &[], net_frame_update_thunk as *const () as usize);
        let mut rng = Rng(0x3939_4A4A_5B5B_6C6C);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in NF_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, NF_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod gma_test {
    use super::difftest::*;
    use super::*;

    const CBA: usize = 0xCB00_0201;
    const CBB: usize = 0xCB00_0202;
    const CBC: usize = 0xCB00_0203;
    const CBD: usize = 0xCB00_0204;

    fn callees() -> Vec<Callee> {
        let mut v = vec![
            callee(0x78fef0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x80fcf0, 1, 4, 0, Ret::Bool, Fx::None),
            callee(0x6a8ca0, 0, 0, 0, Ret::PtrNz(0), Fx::None),
            callee(0x680130, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x6a33f0, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x9d9fd0, 1, 0, 0, Ret::Any, Fx::None),
            callee(0x754020, 0, 0, ESI, Ret::Any, Fx::None),
            callee(0x6a2b40, 1, 0, ECX, Ret::Any, Fx::None),
            callee(CBA, 0, 0, ECX, Ret::Any, Fx::SetActState),
            callee(CBB, 0, 0, ECX, Ret::Bool, Fx::None),
            callee(CBC, 0, 0, ECX, Ret::PtrNz(1), Fx::None),
            callee(CBD, 0, 0, ECX, Ret::Any, Fx::None),
            callee(0xc9604f, 0, 0, ECX, Ret::Any, Fx::None),
        ];
        v[12].silent = true;
        v
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let act = a.pool(1, 0x100)[0];
            let ctl = a.pool(1, 0x40)[0];
            let cvt = a.pool(1, 0x100)[0];
            let hnd = a.pool(1, 0x40)[0];
            let hvt = a.pool(1, 0x100)[0];
            let sing = a.pool(1, 0x40)[0];
            let svt = a.pool(1, 0x100)[0];
            let sess = a.pool(1, 0x40)[0];
            let player = a.pool(1, 0x1200)[0];
            set_pools(vec![vec![sing, sing], vec![0xE, 0xE, 3, 0, 7, 0xE]]);
            put(cvt, 0x40, redirect(CBA) as u32);
            put(cvt, 4, redirect(CBC) as u32);
            put(ctl, 0, cvt);
            put(hvt, 0x1C, redirect(CBB) as u32);
            put(hnd, 0, hvt);
            put(svt, 0x3C, redirect(CBD) as u32);
            put(sing, 0, svt);
            put(act, 0x3C, rng.pick(&[1, 1, 1, 1, 1, 1]));
            put(act, 0x90, ctl);
            wd(0x027B_3C60, if rng.one_in(10) { 0 } else { act });
            wd(0x0140_8E88, if rng.one_in(3) { 0 } else { hnd });
            wd(0x0252_8C14, if rng.one_in(4) { 0 } else { sess });
            wb(0x0140_BF8E, rng.pick(&[0, 1, 1]));
            wb(0x01F8_ACB4, rng.pick(&[0, 1]));
            wd(0x0217_03D4, player);
            putb(player, 0x1163, rng.u32() as u8);
            wd(0x0252_7C90, rng.pick(&[0, 0, 0x1234]));
            let r = rng.u32();
            wd(0x00E8_43AC, rng.pick(&[1000, 0x6B49_D200 - 0x7D0, 0x6B49_D200 - 0x100, 0x6B00_0000, r]));
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn game_mode_activity_frame_matches_original() {
        let _g = lock();
        let h = Harness::new(0x0053_BCA0, &GMA_BODY, &GMA_BODY_CALLS, &GMA_BODY_DISPS, callees(), &[], game_mode_activity_thunk as *const () as usize);
        let mut rng = Rng(0x4A4A_5B5B_6C6C_7D7D);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in GM_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, GM_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}

#[cfg(test)]
mod pcf_test {
    use super::difftest::*;
    use super::*;

    const CB0: usize = 0xCB00_0301;
    const CB4: usize = 0xCB00_0302;
    const CB8: usize = 0xCB00_0303;

    fn callees() -> Vec<Callee> {
        vec![
            callee(0x4ffaf0, 0, 0, EAX, Ret::Bool, Fx::None),
            callee(0x6c43f0, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x985630, 1, 0, 0, Ret::Bool, Fx::None),
            callee(0x9da290, 1, 0, ECX, Ret::Any, Fx::None),
            callee(0xae28f0, 0, 0, ECX, Ret::Ptr(0), Fx::None),
            callee(0xab4960, 0, 0, ECX, Ret::Bool, Fx::None),
            callee(0xab4720, 2, 0, EAX, Ret::Any, Fx::None),
            callee(0x6c6d20, 0, 0, 0, Ret::Bool, Fx::None),
            callee(0x4faa90, 1, 0, 0, Ret::Any, Fx::FlagArg0),
            callee(0x4fae90, 0, 0, 0, Ret::Any, Fx::None),
            callee(0x493260, 0, 0, 0, Ret::Any, Fx::None),
            callee(CB0, 1, 0, 0, Ret::Any, Fx::PlayerFlag),
            callee(CB4, 0, 0, 0, Ret::Any, Fx::None),
            callee(CB8, 0, 0, 0, Ret::Any, Fx::None),
        ]
    }

    unsafe fn world(h: &Harness, rng: &mut Rng) {
        unsafe {
            h.mem.fill_random(rng);
            let mut a = Arena::new();
            let player = a.pool(1, 0x1400)[0];
            let vehs = a.pool(2, 0x40);
            set_pools(vec![vehs.clone()]);
            wd(0x0217_03D4, player);
            put(player, 0x139C, rng.u32());
            put(player, 0xD74, rng.pick(&[0, 0x1234, 0x77]));
            putb(player, 0x1161, rng.u32() as u8);
            putb(player, 0x1163, rng.u32() as u8);
            putb(player, 0x18E8, rng.u32() as u8);
            wd(0x0252_7C08, rng.pick(&[0, 0, 0, 0, 1, 0xFFFF_FFFF, 5]));
            wb(0x0252_8989, rng.pick(&[0, 0, 0, 1]));
            wb(0x0252_7D14, rng.pick(&[0, 1]));
            wb(0x0252_7CF7, rng.pick(&[0, 1]));
            wb(0x0252_7D15, rng.pick(&[0, 0, 1]));
            wd(0x025F_5AE8, rng.pick(&[2, 1, 0, 2, 3]));
            wb(0x00F8_480F, rng.u32() as u8);
            let ctx = rng.pick(&[0u32, 1, 2, 3, 4, 5, 2, 2, 0xFFFF_FFFF]);
            wd(0x00E8_E780, ctx);
            for c in 0..9usize {
                let b = 0x00DD_4CA4 + c * 16;
                wd(b, if rng.one_in(4) { 0 } else { redirect(CB0) as u32 });
                wd(b + 4, if rng.one_in(4) { 0 } else { redirect(CB4) as u32 });
                wd(b + 8, if rng.one_in(4) { 0 } else { redirect(CB8) as u32 });
            }
            wd(0x00E8_7998, rng.pick(&[0xBF80_0000, 0x7F80_0001, 0, 0x3F80_0000]));
        }
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the kit comment)"]
    fn player_controls_do_frame_matches_original() {
        let _g = lock();
        let h = Harness::new(0x004F_B160, &PCF_BODY, &PCF_BODY_CALLS, &PCF_BODY_DISPS, callees(), &[(0x00DD_4CB4usize, 0x100)], player_controls_thunk as *const () as usize);
        let mut rng = Rng(0x5B5B_6C6C_7D7D_8E8E);
        let cases: u32 = std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let mut bad = 0;
        for i in 0..cases {
            unsafe { world(&h, &mut rng) };
            if let Err(e) = h.check(0, [0, 0, 0], rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("case {i}: {e}");
                }
            }
        }
        let mut s = String::new();
        for (i, n) in PC_NAMES.iter().enumerate() {
            s.push_str(&format!(" {}={}", n, PC_HITS[i].load(Ordering::Relaxed)));
        }
        eprintln!("hits:{s}");
        assert_eq!(bad, 0, "{bad} of {cases} cases differ");
    }
}
