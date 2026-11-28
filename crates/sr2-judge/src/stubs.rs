//! Recording stubs for the functions the function under test calls. Each callee's entry in the mapped
//! image gets a jump to its stub; the stub records the call, returns seeded values and cleans the stack
//! the way the callee really does. The original bytes and the port both reach the same stubs.

use std::sync::atomic::Ordering;

use crate::analysis::Callee;
use crate::image::Image;
use crate::sys;

pub const STUB_BASE: usize = 0x3A00_0000;
pub const STUB_SLOT: usize = 32;
pub const MAX_STUBS: usize = 2048;
pub const LOG_CAP: usize = 1024;
pub const NSTACK: usize = 16;
/// A page every dword of which points at the indirect-call stub. The world hands out pointers into it
/// as object vtables, so `call [vtbl+N]` lands in a recording stub instead of faulting.
pub const VT_BASE: usize = 0x3B00_0000;
pub const VT_SIZE: usize = 0x1_0000;
/// The exe's import address table (data directory 12).
pub const IAT_BASE: usize = 0x00DB_8000;
pub const IAT_SIZE: usize = 0x440;
/// Offset of the `ret imm16` operand in an indirect stub: push id (5), call (5), lea (4), C2.
const IND_RET_IMM: usize = 15;

#[derive(Clone, Copy, Debug)]
pub struct CallRec {
    pub id: u32,
    pub regs: [u32; 8],
    pub depth: u32,
    pub stack: [u32; NSTACK],
}

const EMPTY_REC: CallRec = CallRec { id: 0, regs: [0; 8], depth: 0, stack: [0; NSTACK] };

#[derive(Clone, Copy, Default)]
pub struct StubInfo {
    #[allow(dead_code)]
    pub target: u32,
    pub clobber: u8,
    pub float_st0: bool,
    pub xmm0: bool,
    pub indirect: bool,
    /// (stack argument index, byte count) of an output structure the stub fills; count 0 = none.
    pub out: (u8, u16),
}

pub static mut TABLE: [StubInfo; MAX_STUBS] =
    [StubInfo { target: 0, clobber: 0, float_st0: false, xmm0: false, indirect: false, out: (0, 0) }; MAX_STUBS];
/// Indirect call sites of the function under test: (return address, bytes the callee pops).
pub static mut IND_SITES: [(u32, u16); 256] = [(0, 0); 256];
pub static mut IND_N: usize = 0;
/// The indirect stub's address (0 when the function has no indirect calls).
pub static mut IND_STUB: u32 = 0;
/// The import stub's address (0 when the function makes no import calls).
pub static mut IMP_STUB: u32 = 0;
pub static mut NSTUBS: usize = 0;
pub static mut LOG: [CallRec; LOG_CAP] = [EMPTY_REC; LOG_CAP];
pub static mut LOG_N: usize = 0;
pub static mut POISON: [u32; 4096] = [0; 4096];
pub static mut POISON_N: usize = 0;
pub static mut FRET: [f64; MAX_STUBS] = [0.0; MAX_STUBS];
pub static mut FRET32: [f32; MAX_STUBS] = [0.0; MAX_STUBS];
pub static mut SEED: u64 = 0;
pub static mut SEQ: u32 = 0;
pub static mut ARENA_LO: u32 = 0;
pub static mut ARENA_LEN: u32 = 0;

pub fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A seeded value of the kinds the callers see back: 0, 1, small, pointers, -1, anything.
fn mixed(r: u64) -> u32 {
    let v = (r >> 8) as u32;
    unsafe {
        match r % 8 {
            0 => 0,
            1 => 1,
            2 => v % 17,
            3 | 4 => ARENA_LO + (v % (ARENA_LEN / 4)) * 4,
            5 => 0xFFFF_FFFF,
            6 => v & 0xFF,
            // Top bit set: never a mapped address, so it cannot alias the judge's own memory.
            _ => v | 0x8000_0000,
        }
    }
}

/// The reset every run starts with, so both runs see the same sequence of returned values.
pub fn reset_run(seed: u64) {
    unsafe {
        LOG_N = 0;
        POISON_N = 0;
        SEQ = 0;
        SEED = seed;
    }
}

/// Called from `exec::stub_common` with a pointer to the saved registers.
/// Frame: [0..8) pushad block (edi, esi, ebp, esp, ebx, edx, ecx, eax), [8] eflags, [9] return into the
/// stub, [10] stub id, [11] return address of the call, [12..] the callee's stack arguments.
pub extern "C" fn record(frame: *mut u32) -> u32 {
    unsafe {
        let id = *frame.add(10) as usize;
        if id >= NSTUBS {
            // Not one of our stubs: the code under test jumped into the stub area with a bad target.
            sys::FAULT_CODE.store(sys::STUB_FLOOD, Ordering::SeqCst);
            return 1;
        }
        if LOG_N >= LOG_CAP {
            sys::FAULT_CODE.store(sys::STUB_FLOOD, Ordering::SeqCst);
            return 1;
        }
        let info = TABLE[id];
        if info.indirect {
            // Pop what this call site pushed. Unknown sites (the port's own calls) pop nothing; the port's
            // call helpers restore ESP themselves.
            let ret = *frame.add(11);
            let pop = IND_SITES[..IND_N].iter().find(|s| s.0 == ret).map_or(0, |s| s.1);
            let at = (STUB_BASE + id * STUB_SLOT + IND_RET_IMM) as *mut u16;
            at.write_unaligned(pop);
        }
        let rec = &mut LOG[LOG_N];
        LOG_N += 1;
        rec.id = id as u32;
        for k in 0..8 {
            rec.regs[k] = *frame.add(7 - k);
        }
        for k in 0..NSTACK {
            rec.stack[k] = *frame.add(12 + k);
        }
        let mut env = [0u32; 7];
        std::arch::asm!("fnstenv [{0}]", "fldenv [{0}]", in(reg) env.as_mut_ptr(), options(nostack));
        let tags = env[2] & 0xFFFF;
        rec.depth = (0..8).filter(|i| (tags >> (i * 2)) & 3 != 3).count() as u32;

        let seq = SEQ;
        SEQ += 1;
        let r = |salt: u64| splitmix(SEED ^ ((seq as u64) << 24) ^ salt);
        if info.clobber & 1 != 0 {
            *frame.add(7) = mixed(r(1));
        }
        if info.clobber & 2 != 0 {
            let v = (r(2) >> 16) as u32 | 0x8100_0000;
            *frame.add(6) = v;
            if POISON_N < POISON.len() {
                POISON[POISON_N] = v;
                POISON_N += 1;
            }
        }
        if info.clobber & 4 != 0 {
            let v = (r(3) >> 16) as u32 | 0x8100_0000;
            *frame.add(5) = v;
            if POISON_N < POISON.len() {
                POISON[POISON_N] = v;
                POISON_N += 1;
            }
        }
        if info.out.1 > 0 {
            // Fill the output structure the real callee would write, with values both runs share.
            let p = *frame.add(12 + info.out.0 as usize) as usize;
            let in_arena = p >= ARENA_LO as usize && p + info.out.1 as usize <= (ARENA_LO + ARENA_LEN) as usize;
            let in_stack = p >= crate::exec::STACK_BASE && p + info.out.1 as usize <= crate::exec::STACK_BASE + crate::exec::STACK_SIZE;
            if in_arena || in_stack {
                for k in 0..(info.out.1 as usize / 4) {
                    *((p + k * 4) as *mut u32) = mixed(r(100 + k as u64));
                }
            }
        }
        if info.float_st0 || info.xmm0 {
            let f = (((r(4) >> 20) % 40001) as i32 - 20000) as f32 / 32.0;
            FRET[id] = f as f64;
            FRET32[id] = f;
        }
    }
    0
}

/// Write the stub for every callee and aim each callee's entry at it. Returns the stub count.
pub fn install(img: &mut Image, callees: &[Callee]) -> Result<(), String> {
    if callees.len() > MAX_STUBS {
        return Err(format!("{} callees, more than {MAX_STUBS} stubs", callees.len()));
    }
    let common = crate::exec::stub_common as *const () as usize;
    unsafe {
        NSTUBS = callees.len();
        IND_STUB = 0;
        IMP_STUB = 0;
        for (id, c) in callees.iter().enumerate() {
            TABLE[id] =
                StubInfo {
                    target: c.target as u32,
                    clobber: c.clobber,
                    float_st0: c.float_st0,
                    xmm0: c.xmm0,
                    indirect: c.indirect,
                    out: c.out_arg.unwrap_or((0, 0)),
                };
            let at = STUB_BASE + id * STUB_SLOT;
            let mut b: Vec<u8> = Vec::with_capacity(STUB_SLOT);
            b.push(0x68); // push id
            b.extend((id as u32).to_le_bytes());
            let call_at = at + b.len();
            b.push(0xE8); // call common
            b.extend((common.wrapping_sub(call_at + 5) as u32).to_le_bytes());
            b.extend([0x8D, 0x64, 0x24, 0x04]); // lea esp, [esp+4]
            if c.float_st0 {
                b.extend([0xDD, 0x05]); // fld qword ptr [FRET + 8*id]
                b.extend(((&raw const FRET) as usize as u32 + id as u32 * 8).to_le_bytes());
            }
            if c.xmm0 {
                b.extend([0xF3, 0x0F, 0x10, 0x05]); // movss xmm0, [FRET32 + 4*id]
                b.extend(((&raw const FRET32) as usize as u32 + id as u32 * 4).to_le_bytes());
            }
            if c.pop_st0 {
                b.extend([0xDD, 0xD8]); // fstp st(0)
            }
            if c.indirect {
                assert_eq!(b.len(), IND_RET_IMM - 1);
                b.extend([0xC2, 0, 0]); // ret imm16, set per call in record()
                if c.import {
                    IMP_STUB = at as u32;
                } else {
                    IND_STUB = at as u32;
                }
            } else if c.ret_n == 0 {
                b.push(0xC3);
            } else {
                b.push(0xC2);
                b.extend((c.ret_n as u16).to_le_bytes());
            }
            assert!(b.len() <= STUB_SLOT);
            std::ptr::copy_nonoverlapping(b.as_ptr(), at as *mut u8, b.len());
            if c.indirect {
                continue; // reached through the vtable page, not patched into the image
            }
            // callee entry: jmp stub
            let mut j = vec![0xE9u8];
            j.extend((at.wrapping_sub(c.target + 5) as u32).to_le_bytes());
            img.patch(c.target, &j);
        }
    }
    Ok(())
}

/// Put the callees' original bytes back.
pub fn remove(img: &mut Image, orig: &[(usize, [u8; 5])]) {
    for (a, b) in orig {
        img.patch(*a, b);
    }
}

/// Fill the vtable page with the indirect stub's address (0 when there is none) and load the call sites.
/// Done before every run, since a function may write into the page.
pub fn reset_vtables(sites: &[(u32, u16)]) {
    unsafe {
        let n = sites.len().min(IND_SITES.len());
        IND_SITES[..n].copy_from_slice(&sites[..n]);
        IND_N = n;
        let p = VT_BASE as *mut u32;
        for i in 0..VT_SIZE / 4 {
            *p.add(i) = IND_STUB;
        }
        // The exe's import address table is not resolved in the judge: a `call [import]` would jump to a
        // name hint. Every slot leads to the indirect stub instead.
        let iat = IAT_BASE as *mut u32;
        let imp = if IMP_STUB != 0 { IMP_STUB } else { IND_STUB };
        for i in 0..IAT_SIZE / 4 {
            *iat.add(i) = imp;
        }
    }
}
