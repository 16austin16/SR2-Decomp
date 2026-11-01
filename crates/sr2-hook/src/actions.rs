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
//! - `consume` clears only the first button;
//! - the stick-direction leaves compare an f32 with the f64 thresholds +-0.4 (NaN is "not past"),
//!   and a negative y step repeats +y.

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
    /// `player_stick_get(_raw)`: x through the argument pointer, y through ECX.
    StickGet,
}

struct Accessor {
    name: &'static str,
    addr: usize,
    prologue: &'static [u8],
    ret: Ret,
}

const ACCESSORS: [Accessor; 49] = [
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
    Accessor { name: "input_action_stick_dir_down", addr: 0x004FE380, prologue: &[0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0E], ret: Ret::Al },
    Accessor { name: "input_action_stick_dir_just_pressed", addr: 0x004FE460, prologue: &[0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0C], ret: Ret::Al },
    Accessor { name: "input_action_stick_dir_take_pressed", addr: 0x004FE520, prologue: &[0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0C], ret: Ret::Al },
    Accessor { name: "input_action_stick_dir_repeat", addr: 0x004FE5E0, prologue: &[0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0C], ret: Ret::Al },
    Accessor { name: "input_action_stick_dir_take_repeat", addr: 0x004FE6A0, prologue: &[0x83, 0xC0, 0x01, 0x83, 0xF8, 0x0C], ret: Ret::Al },
    Accessor { name: "player_stick_held_neg_x", addr: 0x004FD6D0, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x28, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_held_pos_x", addr: 0x004FD700, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x29, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_held_neg_y", addr: 0x004FD730, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2B, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_held_pos_y", addr: 0x004FD760, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2A, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_just_neg_x", addr: 0x004FD790, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x28, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_just_pos_x", addr: 0x004FD7D0, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x29, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_just_pos_y", addr: 0x004FD810, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x2A, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_just_neg_y", addr: 0x004FD850, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x2B, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_take_just_neg_x", addr: 0x004FD890, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x28, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_just_pos_x", addr: 0x004FD8E0, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x29, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_just_pos_y", addr: 0x004FD930, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x2A, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_just_neg_y", addr: 0x004FD980, prologue: &[0x8D, 0x0C, 0x80, 0x80, 0x3C, 0xCD, 0x2B, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_repeat_neg_x", addr: 0x004FD9D0, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x28, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_repeat_pos_x", addr: 0x004FD9F0, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x29, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_repeat_pos_y", addr: 0x004FDA10, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2A, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_repeat_neg_y", addr: 0x004FDA30, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2B, 0x99, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_take_repeat_neg_x", addr: 0x004FDA50, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x28, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_repeat_pos_x", addr: 0x004FDA90, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x29, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_repeat_pos_y", addr: 0x004FDAD0, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2A, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_take_repeat_neg_y", addr: 0x004FDB10, prologue: &[0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2B, 0x99, 0x65, 0x02, 0x00], ret: Ret::Al },
    Accessor { name: "player_stick_x", addr: 0x004FD510, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2C, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "player_stick_y", addr: 0x004FD540, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2D, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "player_stick_raw_x", addr: 0x004FD570, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2C, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "player_stick_raw_y", addr: 0x004FD5A0, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2D, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "stick_response_curve", addr: 0x004FD5D0, prologue: &[0xD9, 0x44, 0x24, 0x04, 0xD9, 0xC0], ret: Ret::St0 },
    Accessor { name: "player_stick_x_curved", addr: 0x004FD630, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2C, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "player_stick_y_curved", addr: 0x004FD660, prologue: &[0x51, 0x8D, 0x04, 0x80, 0x80, 0x3C, 0xC5, 0x2D, 0x99, 0x65, 0x02, 0x00], ret: Ret::St0 },
    Accessor { name: "input_just_pressed_count", addr: 0x004FD690, prologue: &[0x33, 0xC0, 0x33, 0xC9, 0x80, 0xB9, 0x4D, 0x9B, 0x65, 0x02, 0x00], ret: Ret::Eax },
    Accessor { name: "input_action_table", addr: 0x004FF590, prologue: &[0xB8, 0x38, 0xEA, 0xF9, 0x00], ret: Ret::Eax },
    Accessor { name: "player_stick_get_raw", addr: 0x004FF5A0, prologue: &[0x51, 0xD9, 0xEE, 0x8D, 0x04, 0x80], ret: Ret::StickGet },
    Accessor { name: "player_stick_get", addr: 0x004FF5F0, prologue: &[0x51, 0xD9, 0xEE, 0x8D, 0x04, 0x80], ret: Ret::StickGet },
    Accessor { name: "input_stick_action_consume", addr: 0x00500230, prologue: &[0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x78, 0xF2, 0xF9, 0x00], ret: Ret::Void },
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

// --- Stick directions -----------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    Held,
    Just,
    TakeJust,
    Repeat,
    TakeRepeat,
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    NegX,
    PosX,
    PosY,
    NegY,
}

const STICK_NEG: usize = 0x00E5_C350; // f64 -0.4
const STICK_POS: usize = 0x00E5_C358; // f64 +0.4

/// A stick-direction leaf on control-block entry `e`. Taken bytes: -x +0x20, +x +0x21,
/// +y +0x22, -y +0x23. Held: past +-0.4 (NaN is not past). Just: past now and not last frame
/// (+8 / +0xC). Repeat: the step value (+0x18 x, +0x1C y; +y repeats on a negative step).
/// The take forms set the taken byte (and zero the step) on true.
unsafe fn stick_leaf(kind: Kind, e: u32, dir: Dir) -> u32 {
    unsafe {
        let b = BLOCK + e as usize * 0x28;
        let (axis, taken) = match dir {
            Dir::NegX => (0, 0x20),
            Dir::PosX => (0, 0x21),
            Dir::PosY => (4, 0x22),
            Dir::NegY => (4, 0x23),
        };
        if rb(b + taken) != 0 {
            return 0;
        }
        let neg = matches!(dir, Dir::NegX | Dir::NegY);
        let past = |v: f32| {
            if neg { (v as f64) < *(STICK_NEG as *const f64) } else { (v as f64) > *(STICK_POS as *const f64) }
        };
        let f = |o: usize| *((b + o) as *const f32);
        let hit = match kind {
            Kind::Held => past(f(axis)),
            Kind::Just | Kind::TakeJust => past(f(axis)) && !past(f(axis + 8)),
            Kind::Repeat | Kind::TakeRepeat => {
                let step = rd(b + 0x18 + axis) as i32;
                if matches!(dir, Dir::NegX | Dir::PosY) { step < 0 } else { step > 0 }
            }
        };
        if hit {
            match kind {
                Kind::TakeJust => wb(b + taken, 1),
                Kind::TakeRepeat => {
                    *((b + 0x18 + axis) as *mut u32) = 0;
                    wb(b + taken, 1);
                }
                _ => {}
            }
        }
        hit as u32
    }
}

/// A stick-direction dispatcher: action code -> control-block entry and direction. Codes 0-11
/// cover entries 0, 1 and 5 (+y, -y, +x, -x each); the held test also has 12 / 13 (entry 6, +-x).
/// Anything else is 0.
unsafe fn stick_dir(kind: Kind, code: u32) -> u32 {
    let held = matches!(kind, Kind::Held);
    let c = code as i32;
    let (e, d) = match c {
        0..=11 => ([0, 1, 5][c as usize / 4], c % 4),
        12 | 13 if held => (6, c - 10),
        _ => return 0,
    };
    let dir = [Dir::PosY, Dir::NegY, Dir::PosX, Dir::NegX][d as usize];
    unsafe { stick_leaf(kind, e, dir) }
}

// --- Stick readers ---------------------------------------------------------------------------

const CURVE_K3: usize = 0x00E5_C210; // f64 0.9
const CURVE_K: usize = 0x00DD_50E8; // f64 0.1
const STICK_ACTIONS: usize = 0x00F9_F278; // 16-byte entries, +0 kind

/// Arguments each accessor takes on the stack (the rest take registers only).
fn nargs(id: usize) -> usize {
    if matches!(id, 41 | 46 | 47) { 1 } else { 0 }
}

/// An axis of control-block entry `e`: 0 while its taken byte is set.
unsafe fn axis(e: u32, taken: usize, off: usize) -> f32 {
    unsafe {
        let b = BLOCK + e as usize * 0x28;
        if rb(b + taken) != 0 { 0.0 } else { *((b + off) as *const f32) }
    }
}

/// `stick_response_curve`: f32(0.9 v^3), then +-0.1 away from 0 (x87 at 53 bits: f64 steps).
unsafe fn curve(v: f32) -> f32 {
    unsafe {
        let v = v as f64;
        let mut c = (v * v * *(CURVE_K3 as *const f64) * v) as f32;
        let k = *(CURVE_K as *const f64);
        if c > 0.0 {
            c = (c as f64 + k) as f32;
        }
        if c < 0.0 {
            c = (c as f64 - k) as f32;
        }
        c
    }
}

unsafe fn curved(e: u32, taken: usize, off: usize) -> f32 {
    unsafe {
        let b = BLOCK + e as usize * 0x28;
        if rb(b + taken) != 0 { 0.0 } else { curve(*((b + off) as *const f32)) }
    }
}

/// `player_stick_get(_raw)`: x through the argument pointer, then y over the argument slot itself
/// and through ECX.
unsafe fn stick_get(e: u32, py: u32, args: *mut u32, raw: bool) {
    unsafe {
        let o = if raw { 0x10 } else { 0 };
        let x = axis(e, 0x24, o);
        let y = axis(e, 0x25, o + 4);
        *(*args as *mut f32) = x;
        *args = y.to_bits();
        *(py as *mut f32) = y;
    }
}

unsafe fn stick_action_consume(i: u32) {
    unsafe {
        let a = match rd(STICK_ACTIONS + i as usize * 16) {
            0 => BLOCK + 0x28 + 0x24,
            1 => BLOCK + 0x28 + 0x25,
            2 => BLOCK + 0x24,
            3 => BLOCK + 0x25,
            _ => return,
        };
        wb(a, 1);
    }
}

unsafe fn just_pressed_count() -> u32 {
    unsafe { (0..87).filter(|&b| rb(PRESSED + b) != 0).count() as u32 }
}

/// The Rust accessor `id`: (EAX result, f32 result).
unsafe fn rust(id: usize, r: &Regs, args: *mut u32) -> (u32, f32) {
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
            11 => {
                consume_all();
                (0, 0.0)
            }
            12 => (stick_dir(Kind::Held, r.eax), 0.0),
            13 => (stick_dir(Kind::Just, r.eax), 0.0),
            14 => (stick_dir(Kind::TakeJust, r.eax), 0.0),
            15 => (stick_dir(Kind::Repeat, r.eax), 0.0),
            16 => (stick_dir(Kind::TakeRepeat, r.eax), 0.0),
            17..=36 => {
                let k = (id - 17) / 4;
                let kind = [Kind::Held, Kind::Just, Kind::TakeJust, Kind::Repeat, Kind::TakeRepeat][k];
                let dirs = if k == 0 {
                    [Dir::NegX, Dir::PosX, Dir::NegY, Dir::PosY]
                } else {
                    [Dir::NegX, Dir::PosX, Dir::PosY, Dir::NegY]
                };
                (stick_leaf(kind, r.eax, dirs[(id - 17) % 4]), 0.0)
            }
            37 => (0, axis(r.eax, 0x24, 0)),
            38 => (0, axis(r.eax, 0x25, 4)),
            39 => (0, axis(r.eax, 0x24, 0x10)),
            40 => (0, axis(r.eax, 0x25, 0x14)),
            41 => (0, curve(f32::from_bits(*args))),
            42 => (0, curved(r.eax, 0x24, 0)),
            43 => (0, curved(r.eax, 0x25, 4)),
            44 => (just_pressed_count(), 0.0),
            45 => (ACTIONS as u32, 0.0),
            46 | 47 => {
                stick_get(r.eax, r.ecx, args, id == 46);
                (0, 0.0)
            }
            48 => {
                stick_action_consume(r.eax);
                (0, 0.0)
            }
            _ => (0, 0.0),
        }
    }
}

/// Call the original through its trampoline with the same registers.
unsafe fn original(id: usize, r: &Regs, args: *const u32) -> (u32, f32) {
    let f = ORIGINAL[id].load(Ordering::Relaxed);
    unsafe {
        let a = if nargs(id) > 0 { std::slice::from_raw_parts(args, nargs(id)) } else { &[] };
        let eax = call_regs(f, r.eax, r.ecx, r.edx, r.esi, r.edi, a);
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
        Ret::Void | Ret::StickGet => true,
    }
}

/// What `player_stick_get(_raw)` wrote through its pointers (x, y), for the shadow compare.
/// `px` is the x pointer read before the call (the Rust version overwrites the argument slot).
unsafe fn stick_outputs(id: usize, r: &Regs, px: u32) -> (u32, u32) {
    unsafe {
        if ACCESSORS[id].ret == Ret::StickGet { (rd(px as usize), rd(r.ecx as usize)) } else { (0, 0) }
    }
}

extern "C" fn dispatch(id: usize, r: &mut Regs, args: *mut u32) -> u32 {
    let n = CALLS[id].fetch_add(1, Ordering::Relaxed);
    let shadow = n < SHADOW_CALLS && !IN_SHADOW.with(|s| s.get());
    let out = unsafe {
        if shadow {
            IN_SHADOW.with(|s| s.set(true));
            let block = || std::slice::from_raw_parts(BLOCK as *const u8, BLOCK_END - BLOCK).to_vec();
            let load = |v: &[u8]| std::ptr::copy_nonoverlapping(v.as_ptr(), BLOCK as *mut u8, v.len());
            let before = block();
            let px = if ACCESSORS[id].ret == Ret::StickGet { *args } else { 0 };
            let theirs = original(id, r, args);
            let theirs_out = stick_outputs(id, r, px);
            let theirs_block = block();
            load(&before);
            let ours = rust(id, r, args);
            let ours_out = stick_outputs(id, r, px);
            let ours_block = block();
            IN_SHADOW.with(|s| s.set(false));
            if !same(ACCESSORS[id].ret, theirs, ours) || theirs_block != ours_block || theirs_out != ours_out {
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
            rust(id, r, args)
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

/// Synthetic check of the stick-direction leaves and dispatchers against the originals: every
/// leaf / code on entries 0, 1, 5 and 6 over a grid of axis values around the thresholds (with
/// NaN), previous values, taken flags and steps. The control block is restored afterwards. Run
/// once on the game thread.
pub fn stick_self_test() {
    let vals = [-1.0f32, -0.5, -0.400_000_1, -0.4, -0.399_999_9, 0.0, 0.399_999_9, 0.4, 0.400_000_1, 0.5, f32::NAN];
    let mut checked = 0u32;
    let mut bad = 0u32;
    unsafe {
        let len = BLOCK_END - BLOCK;
        let saved = std::slice::from_raw_parts(BLOCK as *const u8, len).to_vec();
        let block = || std::slice::from_raw_parts(BLOCK as *const u8, len).to_vec();
        let load = |v: &[u8]| std::ptr::copy_nonoverlapping(v.as_ptr(), BLOCK as *mut u8, len);
        IN_SHADOW.with(|x| x.set(true));
        for &v in &vals {
            for &pv in &vals {
                for taken in [0u8, 1] {
                    for step in [-1i32, 0, 1] {
                        for e in [0usize, 1, 5, 6] {
                            let b = BLOCK + e * 0x28;
                            for o in [0, 4, 8, 0xC] {
                                *((b + o) as *mut f32) = if o < 8 { v } else { pv };
                            }
                            *((b + 0x18) as *mut i32) = step;
                            *((b + 0x1C) as *mut i32) = -step;
                            std::ptr::write_bytes((b + 0x20) as *mut u8, taken, 4);
                        }
                        let state = block();
                        let mut run = |id: usize, eax: u32| {
                            load(&state);
                            let r = Regs { edi: 0, esi: 0, ebp: 0, st0: 0, ebx: 0, edx: 0, ecx: 0, eax };
                            let theirs = original(id, &r, std::ptr::null());
                            let tb = block();
                            load(&state);
                            let ours = rust(id, &r, std::ptr::null_mut());
                            let ob = block();
                            checked += 1;
                            if !same(ACCESSORS[id].ret, theirs, ours) || tb != ob {
                                bad += 1;
                                if bad <= 10 {
                                    log(&format!(
                                        "actions: self-test {} eax {eax:#x} v {v} prev {pv} taken {taken} step {step}: original {:#x}, Rust {:#x}{}",
                                        ACCESSORS[id].name,
                                        theirs.0,
                                        ours.0,
                                        if tb == ob { "" } else { " (block differs)" }
                                    ));
                                }
                            }
                        };
                        for id in 17..37 {
                            for e in [0u32, 1, 5, 6] {
                                run(id, e);
                            }
                        }
                        for id in 12..17 {
                            for code in -2i32..16 {
                                run(id, code as u32);
                            }
                        }
                        for id in [37, 38, 39, 40, 42, 43] {
                            for e in [0u32, 1, 5, 6] {
                                run(id, e);
                            }
                        }
                    }
                }
            }
        }
        // The response curve over a sweep of inputs.
        for k in -2000i32..=2000 {
            for v in [k as f32 / 1000.0, k as f32 / 1000.0 + 1e-4, f32::from_bits(0x3E80_0000u32.wrapping_add(k as u32))] {
                let mut a = [v.to_bits()];
                let r = Regs { edi: 0, esi: 0, ebp: 0, st0: 0, ebx: 0, edx: 0, ecx: 0, eax: 0 };
                let theirs = original(41, &r, a.as_ptr());
                let ours = rust(41, &r, a.as_mut_ptr());
                checked += 1;
                if theirs.1.to_bits() != ours.1.to_bits() {
                    bad += 1;
                    if bad <= 10 {
                        log(&format!("actions: self-test stick_response_curve({v}): original {}, Rust {}", theirs.1, ours.1));
                    }
                }
            }
        }
        IN_SHADOW.with(|x| x.set(false));
        load(&saved);
    }
    log(&format!("actions: stick self-test: {checked} cases, {bad} mismatched"));
}

macro_rules! thunk {
    ($name:ident, $id:expr) => {
        thunk!($name, $id, "ret");
    };
    ($name:ident, $id:expr, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "push {id}",
                "call {d}",
                "add esp, 12",
                "test eax, eax",
                "jz 2f",
                "fld dword ptr [esp + 12]",
                "2:",
                "popad",
                $ret,
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
thunk!(t12, 12);
thunk!(t13, 13);
thunk!(t14, 14);
thunk!(t15, 15);
thunk!(t16, 16);
thunk!(t17, 17);
thunk!(t18, 18);
thunk!(t19, 19);
thunk!(t20, 20);
thunk!(t21, 21);
thunk!(t22, 22);
thunk!(t23, 23);
thunk!(t24, 24);
thunk!(t25, 25);
thunk!(t26, 26);
thunk!(t27, 27);
thunk!(t28, 28);
thunk!(t29, 29);
thunk!(t30, 30);
thunk!(t31, 31);
thunk!(t32, 32);
thunk!(t33, 33);
thunk!(t34, 34);
thunk!(t35, 35);
thunk!(t36, 36);
thunk!(t37, 37);
thunk!(t38, 38);
thunk!(t39, 39);
thunk!(t40, 40);
thunk!(t41, 41, "ret 4");
thunk!(t42, 42);
thunk!(t43, 43);
thunk!(t44, 44);
thunk!(t45, 45);
thunk!(t46, 46);
thunk!(t47, 47);
thunk!(t48, 48);
const THUNKS: [extern "C" fn(); COUNT] = [
    t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15, t16, t17, t18, t19, t20, t21, t22, t23, t24, t25, t26, t27, t28, t29, t30, t31, t32, t33, t34, t35, t36, t37, t38, t39, t40, t41, t42, t43, t44, t45, t46, t47, t48,
];

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
