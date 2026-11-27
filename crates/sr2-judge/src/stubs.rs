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
}

pub static mut TABLE: [StubInfo; MAX_STUBS] = [StubInfo { target: 0, clobber: 0, float_st0: false, xmm0: false }; MAX_STUBS];
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
            _ => v,
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
        if LOG_N >= LOG_CAP {
            sys::FAULT_CODE.store(sys::STUB_FLOOD, Ordering::SeqCst);
            return 1;
        }
        let info = TABLE[id];
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
            let v = (r(2) >> 16) as u32 | 0x0100_0000;
            *frame.add(6) = v;
            if POISON_N < POISON.len() {
                POISON[POISON_N] = v;
                POISON_N += 1;
            }
        }
        if info.clobber & 4 != 0 {
            let v = (r(3) >> 16) as u32 | 0x0100_0000;
            *frame.add(5) = v;
            if POISON_N < POISON.len() {
                POISON[POISON_N] = v;
                POISON_N += 1;
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
        for (id, c) in callees.iter().enumerate() {
            TABLE[id] = StubInfo { target: c.target as u32, clobber: c.clobber, float_st0: c.float_st0, xmm0: c.xmm0 };
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
            if c.ret_n == 0 {
                b.push(0xC3);
            } else {
                b.push(0xC2);
                b.extend((c.ret_n as u16).to_le_bytes());
            }
            assert!(b.len() <= STUB_SLOT);
            std::ptr::copy_nonoverlapping(b.as_ptr(), at as *mut u8, b.len());
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
