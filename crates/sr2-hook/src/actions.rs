//! The game-action accessors in Rust (docs/analysis/14_input.md, "The accessors at instruction
//! level"). Gameplay and menus read input through these: an action maps to a primary and a
//! secondary button record (table 0xF9EA38, 16 bytes: +0 primary, +4 secondary, +8 stick
//! direction code, +0xC name), and the answers come from the control block that
//! `player_controls_update` builds (controls.rs).
//!
//! Every accessor is entered through one generic thunk: it saves all registers (`pushad`),
//! hands the register block and the caller's stack arguments to `dispatch`, and restores
//! everything except EAX, which carries the result. Results on the x87 stack are loaded from the
//! block's ESP slot (ignored by `popad`). So ECX / EDX come back unchanged, which is at least as
//! much as any original preserves.
//!
//! **Shadow check.** For the first `SHADOW_CALLS` calls of each accessor the original runs first
//! through its trampoline, on the same control block; its result and the control block it leaves
//! are compared with the Rust version's, and on a mismatch the original's are kept.
//!
//! Quirks kept from the originals:
//! - the "first" button is the primary, or the secondary when the primary is -1, and the second
//!   test is always the secondary (so an unbound primary tests the secondary twice);
//! - value reads test eaten as `== 1`, and round through f32;
//! - `take_repeat`: a button held without a repeat pulse this frame calls the stick fallback,
//!   ignores it, and returns 0;
//! - `take_pressed` always calls its stick fallback;
//! - `press_time` with an unbound primary reads 0x2659BFC[-1], and with primary 0 returns 0;
//! - `consume` clears only the first button.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::gfx_cmd::call_regs;
use crate::{detour, log};

const ACTIONS: usize = 0x00F9_EA38;
const PADS: usize = 0x0234_78B8;
const PAD_SIZE: usize = 0x104C;
const PAD_INDEX: usize = 0x00E9_8DB0;
const BUTTON_SIZE: usize = 0x24;
const BLOCK: usize = 0x0265_9908;
const BLOCK_END: usize = 0x0265_9D58;
const DOWN: usize = 0x0265_9A48;
const REPEAT: usize = 0x0265_9A9F;
const AVAIL: usize = 0x0265_9AF6;
const PRESSED: usize = 0x0265_9B4D;
const AVAIL_PRESSED: usize = 0x0265_9BA4;
const PRESS_TIME: usize = 0x0265_9BFC;
const STICK_DIR_DOWN: usize = 0x004F_E380;
const STICK_DIR_JUST_PRESSED: usize = 0x004F_E460;
const STICK_DIR_TAKE_PRESSED: usize = 0x004F_E520;
const STICK_DIR_REPEAT: usize = 0x004F_E5E0;
const STICK_DIR_TAKE_REPEAT: usize = 0x004F_E6A0;
const NONE: u32 = u32::MAX;

const SHADOW_CALLS: u32 = 3000;

#[derive(Clone, Copy, PartialEq)]
enum Ret {
    /// AL only is meaningful.
    Al,
    /// The whole of EAX.
    Eax,
    /// ST0 (an f32 value).
    St0,
    /// Nothing (EAX is whatever the original left).
    Void,
}

struct Accessor {
    name: &'static str,
    addr: usize,
    prologue: &'static [u8],
    ret: Ret,
}

const ACCESSORS: [Accessor; 12] = [
    Accessor { name: "input_action_down", addr: 0x004F_F640, prologue: &[0xC1, 0xE1, 0x04, 0x8B, 0x91, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Al },
    Accessor { name: "input_action_repeat", addr: 0x004F_F6A0, prologue: &[0xC1, 0xE1, 0x04, 0x8B, 0x91, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Al },
    Accessor { name: "input_action_just_pressed", addr: 0x004F_F700, prologue: &[0xC1, 0xE1, 0x04, 0x8B, 0x91, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Al },
    Accessor { name: "input_action_value", addr: 0x004F_F770, prologue: &[0x51, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::St0 },
    Accessor { name: "input_action_prev_value", addr: 0x004F_F7C0, prologue: &[0x51, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::St0 },
    Accessor { name: "input_action_alt_value", addr: 0x004F_F810, prologue: &[0x51, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x3C, 0xEA, 0xF9, 0x00], ret: Ret::St0 },
    Accessor { name: "input_action_alt_prev_value", addr: 0x004F_F860, prologue: &[0x51, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x3C, 0xEA, 0xF9, 0x00], ret: Ret::St0 },
    Accessor { name: "input_action_take_repeat", addr: 0x004F_FA30, prologue: &[0x53, 0xC1, 0xE0, 0x04, 0x56], ret: Ret::Al },
    Accessor { name: "input_action_take_pressed", addr: 0x004F_FAF0, prologue: &[0xC1, 0xE0, 0x04, 0x8B, 0x88, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Al },
    Accessor { name: "input_action_press_time", addr: 0x004F_FB80, prologue: &[0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Eax },
    Accessor { name: "input_action_consume", addr: 0x0050_01D0, prologue: &[0xC1, 0xE0, 0x04, 0x8B, 0x88, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Void },
    Accessor { name: "input_consume_all", addr: 0x0050_02B0, prologue: &[0x6A, 0x57, 0x6A, 0x00, 0x68, 0xF6, 0x9A, 0x65, 0x02], ret: Ret::Void },
];
const COUNT: usize = ACCESSORS.len();

static ORIGINAL: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

thread_local! {
    static IN_SHADOW: Cell<bool> = const { Cell::new(false) };
}

/// The `pushad` block, lowest address first. The ESP slot carries an f32 result out.
#[repr(C)]
struct Regs {
    edi: u32,
    esi: u32,
    ebp: u32,
    st0: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}

unsafe fn entry(action: u32) -> usize {
    ACTIONS + (action as usize) * 16
}

unsafe fn primary(action: u32) -> u32 {
    unsafe { rd(entry(action)) }
}

unsafe fn secondary(action: u32) -> u32 {
    unsafe { rd(entry(action) + 4) }
}

unsafe fn stick_code(action: u32) -> u32 {
    unsafe { rd(entry(action) + 8) }
}

/// The primary, or the secondary when the primary is unbound.
unsafe fn first(action: u32) -> u32 {
    unsafe {
        let p = primary(action);
        if p != NONE { p } else { secondary(action) }
    }
}

unsafe fn test(state: usize, mask: usize, b: u32) -> bool {
    unsafe { rb(mask + b as usize) & rb(state + b as usize) != 0 }
}

unsafe fn record(b: u32) -> usize {
    let i = unsafe { *(PAD_INDEX as *const i32) };
    PADS + if i == -1 { 0 } else { i as usize } * PAD_SIZE + b as usize * BUTTON_SIZE
}

unsafe fn take(b: u32) {
    unsafe {
        wb(AVAIL + b as usize, 0);
        wb(AVAIL_PRESSED + b as usize, 0);
    }
}

/// down / repeat / just pressed: either button, else the stick fallback.
unsafe fn query(action: u32, state: usize, mask: usize, fallback: usize) -> u32 {
    unsafe {
        let f = first(action);
        let s = secondary(action);
        let r = (f != NONE && test(state, mask, f)) | (s != NONE && test(state, mask, s));
        if r { 1 } else { call_regs(fallback, stick_code(action), 0, 0, 0, 0, &[]) }
    }
}

/// value reads: 0 when unbound (or eaten, for the current value).
unsafe fn value(b: u32, off: usize, eaten_test: bool) -> f32 {
    unsafe {
        if b == NONE {
            return 0.0;
        }
        let r = record(b);
        if eaten_test && rb(r + 0x20) == 1 { 0.0 } else { *((r + off) as *const f32) }
    }
}

unsafe fn take_repeat(action: u32) -> u32 {
    unsafe {
        let code = stick_code(action);
        let fallback = || call_regs(STICK_DIR_TAKE_REPEAT, code, 0, 0, 0, 0, &[]) as u8;
        let mut taken = false;
        let f = first(action);
        if f != NONE {
            let held = test(DOWN, AVAIL, f);
            if test(REPEAT, AVAIL, f) {
                take(f);
                taken = true;
            } else if held {
                fallback();
                return 0;
            }
        }
        let s = secondary(action);
        if s != NONE {
            let held = test(DOWN, AVAIL, s);
            let t = test(REPEAT, AVAIL, s);
            if t {
                take(s);
            }
            taken |= t;
            if !taken && held {
                fallback();
                return 0;
            }
        }
        (fallback() != 0 || taken) as u32
    }
}

unsafe fn take_pressed(action: u32) -> u32 {
    unsafe {
        let mut taken = false;
        let f = first(action);
        if f != NONE && test(PRESSED, AVAIL_PRESSED, f) {
            take(f);
            taken = true;
        }
        let s = secondary(action);
        if s != NONE && test(PRESSED, AVAIL_PRESSED, s) {
            take(s);
            taken = true;
        }
        let r = call_regs(STICK_DIR_TAKE_PRESSED, stick_code(action), 0, 0, 0, 0, &[]);
        if r as u8 != 0 || taken { 1 } else { r }
    }
}

unsafe fn press_time(action: u32) -> u32 {
    unsafe {
        let p = primary(action);
        if p == 0 {
            return 0;
        }
        // An unbound primary reads the slot before the array, as the original does.
        rd((PRESS_TIME as isize + (p as i32 as isize) * 4) as usize)
    }
}

unsafe fn consume(action: u32) {
    unsafe {
        let f = first(action);
        if f != NONE {
            take(f);
        }
    }
}

unsafe fn consume_all() {
    unsafe {
        std::ptr::write_bytes(AVAIL as *mut u8, 0, 87);
        std::ptr::write_bytes(AVAIL_PRESSED as *mut u8, 0, 87);
        for e in [BLOCK, BLOCK + 0x28] {
            std::ptr::write_bytes((e + 0x20) as *mut u8, 1, 6);
        }
    }
}

/// The Rust accessor `id`: (EAX result, f32 result).
unsafe fn rust(id: usize, r: &Regs) -> (u32, f32) {
    unsafe {
        match id {
            0 => (query(r.ecx, DOWN, AVAIL, STICK_DIR_DOWN), 0.0),
            1 => (query(r.ecx, REPEAT, AVAIL, STICK_DIR_REPEAT), 0.0),
            2 => (query(r.ecx, PRESSED, AVAIL_PRESSED, STICK_DIR_JUST_PRESSED), 0.0),
            3 => (0, value(primary(r.eax), 0, true)),
            4 => (0, value(primary(r.eax), 4, false)),
            5 => (0, value(secondary(r.eax), 0, true)),
            6 => (0, value(secondary(r.eax), 4, false)),
            7 => (take_repeat(r.eax), 0.0),
            8 => (take_pressed(r.eax), 0.0),
            9 => (press_time(r.eax), 0.0),
            10 => {
                consume(r.eax);
                (0, 0.0)
            }
            _ => {
                consume_all();
                (0, 0.0)
            }
        }
    }
}

/// Call the original through its trampoline with the same registers.
unsafe fn original(id: usize, r: &Regs) -> (u32, f32) {
    let f = ORIGINAL[id].load(Ordering::Relaxed);
    unsafe {
        let eax = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, &[]);
        let mut v = 0f32;
        if ACCESSORS[id].ret == Ret::St0 {
            std::arch::asm!("fstp dword ptr [{p}]", p = in(reg) &mut v as *mut f32, options(nostack));
        }
        (eax, v)
    }
}

fn same(ret: Ret, a: (u32, f32), b: (u32, f32)) -> bool {
    match ret {
        Ret::Al => a.0 as u8 == b.0 as u8,
        Ret::Eax => a.0 == b.0,
        Ret::St0 => a.1.to_bits() == b.1.to_bits(),
        Ret::Void => true,
    }
}

extern "C" fn dispatch(id: usize, r: &mut Regs) -> u32 {
    let n = CALLS[id].fetch_add(1, Ordering::Relaxed);
    let shadow = n < SHADOW_CALLS && !IN_SHADOW.with(|s| s.get());
    let out = unsafe {
        if shadow {
            IN_SHADOW.with(|s| s.set(true));
            let block = || std::slice::from_raw_parts(BLOCK as *const u8, BLOCK_END - BLOCK).to_vec();
            let load = |v: &[u8]| std::ptr::copy_nonoverlapping(v.as_ptr(), BLOCK as *mut u8, v.len());
            let before = block();
            let theirs = original(id, r);
            let theirs_block = block();
            load(&before);
            let ours = rust(id, r);
            let ours_block = block();
            IN_SHADOW.with(|s| s.set(false));
            if !same(ACCESSORS[id].ret, theirs, ours) || theirs_block != ours_block {
                let m = MISMATCHES[id].fetch_add(1, Ordering::Relaxed) + 1;
                if m <= 10 {
                    log(&format!(
                        "actions: {} mismatch (call {n}, eax {:#x} ecx {:#x}): original {:#x} / {}, Rust {:#x} / {}, block {}",
                        ACCESSORS[id].name,
                        r.eax,
                        r.ecx,
                        theirs.0,
                        theirs.1,
                        ours.0,
                        ours.1,
                        if theirs_block == ours_block { "same" } else { "differs" }
                    ));
                }
                load(&theirs_block);
                theirs
            } else {
                ours
            }
        } else {
            rust(id, r)
        }
    };
    r.eax = out.0;
    if ACCESSORS[id].ret == Ret::St0 {
        r.st0 = out.1.to_bits();
        1
    } else {
        0
    }
}

macro_rules! thunk {
    ($name:ident, $id:expr) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "push eax",
                "push {id}",
                "call {d}",
                "add esp, 8",
                "test eax, eax",
                "jz 2f",
                "fld dword ptr [esp + 12]",
                "2:",
                "popad",
                "ret",
                id = const $id,
                d = sym dispatch,
            )
        }
    };
}
thunk!(t0, 0);
thunk!(t1, 1);
thunk!(t2, 2);
thunk!(t3, 3);
thunk!(t4, 4);
thunk!(t5, 5);
thunk!(t6, 6);
thunk!(t7, 7);
thunk!(t8, 8);
thunk!(t9, 9);
thunk!(t10, 10);
thunk!(t11, 11);
const THUNKS: [extern "C" fn(); COUNT] = [t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11];

pub fn install() {
    let mut ok = 0;
    for (i, a) in ACCESSORS.iter().enumerate() {
        match unsafe { detour::install(a.addr, a.prologue, THUNKS[i] as usize) } {
            Ok(t) => {
                ORIGINAL[i].store(t, Ordering::Relaxed);
                ok += 1;
            }
            Err(e) => log(&format!("actions: hooking {} failed: {e:?}", a.name)),
        }
    }
    log(&format!("actions: replaced {ok}/{COUNT} game-action accessors with Rust (shadow-checked)"));
}

pub fn summary() -> String {
    let parts: Vec<String> = ACCESSORS
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let c = CALLS[i].load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({} mismatched)", a.name.trim_start_matches("input_"), MISMATCHES[i].load(Ordering::Relaxed)))
        })
        .collect();
    format!("actions: {}", parts.join(", "))
}
