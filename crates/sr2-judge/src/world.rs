//! Random worlds: a seeded arena of pointers, integers and floats, random registers, stack arguments,
//! XMM registers and the globals the function reads.

use crate::analysis::FnInfo;
use crate::exec::STACK_ARGS;
use crate::stubs::{self, splitmix};

pub const ARENA_BASE: usize = 0x3000_0000;
pub const ARENA_SIZE: usize = 0x20_0000;

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = splitmix(self.0);
        self.0
    }
    pub fn u32(&mut self) -> u32 {
        (self.next() >> 16) as u32
    }
    pub fn below(&mut self, n: u32) -> u32 {
        (self.u32() as u64 * n as u64 >> 32) as u32
    }
}

const SPECIAL_F32: [f32; 12] = [0.0, -0.0, 1.0, -1.0, 0.5, 100.0, -100.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-30, 3.0e9];

pub struct World {
    pub regs: [u32; 8],
    pub stack: [u32; STACK_ARGS],
    pub xmm: [[u32; 4]; 8],
    /// (address, bytes) written into the image before the case.
    pub globals: Vec<(usize, Vec<u8>)>,
}

fn arena_ptr(r: &mut Rng) -> u32 {
    // Half of the pointers land in the first 64 KB, which keeps chains of dereferences short.
    let span = if r.below(2) == 0 { 0x1_0000 } else { ARENA_SIZE as u32 - 0x100 };
    ARENA_BASE as u32 + (r.below(span / 4)) * 4
}

/// One value of the mix the arena is made of.
pub fn value(r: &mut Rng) -> u32 {
    match r.below(16) {
        0..=3 => arena_ptr(r),
        4 => arena_ptr(r) + 1 + r.below(3),
        5 | 6 => r.below(33),
        7 => (-(r.below(16) as i32) - 1) as u32,
        8 => {
            let m = r.below(2000) as f32 - 1000.0;
            (m * (r.below(1000) as f32 / 128.0 + 0.01)).to_bits()
        }
        9 => SPECIAL_F32[r.below(SPECIAL_F32.len() as u32) as usize].to_bits(),
        10..=12 => 0,
        13 => r.below(0x1_0000),
        _ => r.u32(),
    }
}

fn reg_value(r: &mut Rng) -> u32 {
    if r.below(100) < 55 { arena_ptr(r) } else { value(r) }
}

/// Fill the arena. Deterministic in `seed`.
pub fn fill_arena(seed: u64) {
    let mut r = Rng(seed ^ 0xA5A5_5A5A_1234_5678);
    let p = ARENA_BASE as *mut u32;
    unsafe {
        for i in 0..ARENA_SIZE / 4 {
            *p.add(i) = value(&mut r);
        }
    }
}

pub fn make(seed: u64, info: &FnInfo) -> World {
    let mut r = Rng(seed);
    let mut regs = [0u32; 8];
    for (i, v) in regs.iter_mut().enumerate() {
        if i == crate::analysis::ESP {
            continue;
        }
        *v = reg_value(&mut r);
    }
    let mut stack = [0u32; STACK_ARGS];
    for v in stack.iter_mut() {
        *v = reg_value(&mut r);
    }
    let mut xmm = [[0u32; 4]; 8];
    for x in xmm.iter_mut() {
        for l in x.iter_mut() {
            *l = match r.below(8) {
                0 => 0,
                1 => SPECIAL_F32[r.below(SPECIAL_F32.len() as u32) as usize].to_bits(),
                _ => ((r.below(20001) as f32 - 10000.0) / 8.0).to_bits(),
            };
        }
    }
    let mut globals = Vec::new();
    for &(addr, size) in &info.globals {
        let mut bytes = Vec::new();
        let n = size.div_ceil(4).max(1);
        for _ in 0..n {
            bytes.extend(reg_value(&mut r).to_le_bytes());
        }
        bytes.truncate(size.max(1));
        globals.push((addr, bytes));
    }
    unsafe {
        stubs::ARENA_LO = ARENA_BASE as u32;
        stubs::ARENA_LEN = ARENA_SIZE as u32;
    }
    World { regs, stack, xmm, globals }
}
