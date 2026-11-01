//! `game_frame_tick` (0x520450) in Rust: the game thread's whole frame, called in a
//! loop by `game_main_thread_proc`; returns 1 to quit (docs/analysis/40, "game_frame_tick").
//!
//! The body is a call sequence. The callees stay the game's (called by address; none
//! takes register arguments) except the ones already ported, `input_do_frame` and
//! `mouse_update`, which reach their Rust versions through their hooks:
//!
//! 1. Profiler markers "sim_frame" and "simfr:1,2,3,4,5,6".
//! 2. Quit when the quit flag is set and the frame counter is past 200 (the
//!    markers are left open, as in the original).
//! 3. A pending "screen change failed" (PC): run `screen_change_failed_pc` in the
//!    `pause_menu` document, as a vint script thread.
//! 4. Input, vint input (unless a text box owns it), player controls, mouse, dialog
//!    boxes, pause.
//! 5. `frame_timer_update(fixed, step, extra)` (11_frame.md, "Timing"): fixed-step
//!    mode with step = physics step / slow-motion factor when 0x25272DE is set and
//!    0x25273B4 clear. `extra` is 0xE98AF0 when 0x2527321 is set, else 0; the doc
//!    lists the arguments as (fixed, ?, step), but the code passes step second.
//! 6. Network, saves and per-frame services, then `game_state_frame`, then
//!    `frame_end_handoff` (the handoff to the render thread) under "sim-hand".
//!
//! `game_state_frame` (0x697780) and `game_state_stack_dispatch` (0x697700) are Rust
//! too (11_frame.md, "Game states"): the state stack, its transitions, and running the
//! states. Each state's enter / exit / frame functions stay the game's.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::gfx_cmd::call_regs;
use crate::{detour, log};

const PROFILE_START: usize = 0x00C0_BE10; // ECX = label, (&id), ret 4
const PROFILE_STOP: usize = 0x00C0_C990; // cdecl (&id)
const LABEL_SIM_FRAME: u32 = 0x00DE_16E4;
const LABEL_SIMFR_SECTIONS: u32 = 0x00DE_16F0;
const LABEL_SIM_HAND: u32 = 0x00DE_1728;

const QUIT_REQUESTED: usize = 0x00FA_23A0;
const FRAME_COUNTER: usize = 0x0252_A3C0;

const SCREEN_CHANGE_FAILED: usize = 0x0252_A26F; // u8, set when a display change failed
const STR_PAUSE_MENU: u32 = 0x00DE_1704; // "pause_menu"
const STR_SCREEN_CHANGE_FAILED_PC: u32 = 0x00DE_1710; // its Lua function
const STRING_CRC32_LOWER: usize = 0x00BD_C9B0; // ECX = string
const DOCUMENT_LIST: usize = 0x0252_A1AC; // circular, link at +0, name hash +0x48, handle +0x4C
const VINT_LUA_SLOT: usize = 0x0252_A1B8; // script slot of the vint state
const SCRIPT_THREAD_CREATE: usize = 0x00CE_01E0; // EAX = slot, cdecl (name, 0, 0, 0) -> entry
const SCRIPT_THREAD_RESUME: usize = 0x00CE_0570; // EDI = &entry, cdecl (0)
const THREAD_DEAD: u8 = 0x40; // entry +0x18

const SERVICES_PRE_INPUT: usize = 0x0069_9C90;
const INPUT_DO_FRAME: usize = 0x00C0_63D0;
const VINT_INPUT_DO_FRAME: usize = 0x00B9_9550;
// vint input is blocked while 0x2528B24 > 0, unless 0x1F768DC or 0x2527FEB is set.
const VINT_INPUT_BLOCK: usize = 0x0252_8B24;
const VINT_INPUT_UNBLOCK: usize = 0x01F7_68DC;
const VINT_INPUT_UNBLOCK2: usize = 0x0252_7FEB; // u8
const PLAYER_CONTROLS_UPDATE: usize = 0x004F_E760;
const CONTROLS_POST: usize = 0x0050_0CA0;
const MOUSE_UPDATE: usize = 0x00C1_F320;
const DIALOG_BOX_DO_FRAME: usize = 0x0075_3AA0;
const GAME_PAUSE_DO_FRAME: usize = 0x0069_B160;

const FRAME_TIMER_UPDATE: usize = 0x00BF_0710; // cdecl (u8 fixed, f32 step, f32 extra)
const FIXED_STEP_ON: usize = 0x0252_72DE; // u8
const FIXED_STEP_OFF: usize = 0x0252_73B4; // u8
const EXTRA_ON: usize = 0x0252_7321; // u8
const EXTRA_VALUE: usize = 0x00E9_8AF0; // f32
const PHYSICS_STEP: usize = 0x0252_7DA4; // f32
const SLOW_MOTION: usize = 0x00E8_ECAC; // f32, 1.0 normally

const SERVICES_8217F0: usize = 0x0082_17F0;
const NETWORK_PUMP: usize = 0x0080_9640;
const PROFILE_FRAME: usize = 0x0069_1430; // profile.dat / .bbsave saves
const SERVICES_69A440: usize = 0x0069_A440;
const FRAME_END_HANDOFF: usize = 0x0052_1A00;

pub static FRAMES: AtomicU32 = AtomicU32::new(0);

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

unsafe fn rf(p: usize) -> f32 {
    unsafe { *(p as *const f32) }
}

unsafe fn call(f: usize) {
    unsafe { call_regs(f, 0, 0, 0, 0, 0, &[]) };
}

unsafe fn profile_start(label: u32, id: &mut u32) {
    unsafe { call_regs(PROFILE_START, 0, label, 0, 0, 0, &[id as *mut u32 as u32]) };
}

unsafe fn profile_stop(id: &mut u32) {
    unsafe { call_regs(PROFILE_STOP, 0, 0, 0, 0, 0, &[id as *mut u32 as u32]) };
}

/// Run `screen_change_failed_pc` in the document it belongs to.
unsafe fn screen_change_failed() {
    unsafe {
        *(SCREEN_CHANGE_FAILED as *mut u8) = 0;
        let h = call_regs(STRING_CRC32_LOWER, 0, STR_PAUSE_MENU, 0, 0, 0, &[]);
        let head = rd(DOCUMENT_LIST);
        let mut d = head;
        while d != 0 && rd(d as usize + 0x48) != h {
            d = rd(d as usize);
            if d == head {
                return;
            }
        }
        if d == 0 {
            return;
        }
        let mut t = call_regs(SCRIPT_THREAD_CREATE, rd(VINT_LUA_SLOT), 0, 0, 0, 0, &[STR_SCREEN_CHANGE_FAILED_PC, 0, 0, 0]);
        *((t as usize + 0x14) as *mut u32) = rd(d as usize + 0x4C);
        if rb(t as usize + 0x18) & THREAD_DEAD == 0 {
            call_regs(SCRIPT_THREAD_RESUME, 0, 0, 0, 0, &mut t as *mut u32 as u32, &[0]);
        }
    }
}

/// The frame. Returns 1 to quit.
pub fn game_frame_tick() -> i32 {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    let (mut simfr, mut sections, mut hand) = (0u32, 0u32, 0u32);
    unsafe {
        profile_start(LABEL_SIM_FRAME, &mut simfr);
        profile_start(LABEL_SIMFR_SECTIONS, &mut sections);
        if rd(QUIT_REQUESTED) != 0 && rd(FRAME_COUNTER) as i32 > 200 {
            return 1;
        }
        if rb(SCREEN_CHANGE_FAILED) == 1 {
            screen_change_failed();
        }
        call(SERVICES_PRE_INPUT);
        call(INPUT_DO_FRAME);
        if rd(VINT_INPUT_BLOCK) == 0 || rd(VINT_INPUT_UNBLOCK) != 0 || rb(VINT_INPUT_UNBLOCK2) != 0 {
            call(VINT_INPUT_DO_FRAME);
        }
        call(PLAYER_CONTROLS_UPDATE);
        call(CONTROLS_POST);
        call(MOUSE_UPDATE);
        call(DIALOG_BOX_DO_FRAME);
        call(GAME_PAUSE_DO_FRAME);

        let extra = if rb(EXTRA_ON) != 0 { rf(EXTRA_VALUE) } else { 0.0 };
        let (fixed, step) = if rb(FIXED_STEP_ON) != 0 && rb(FIXED_STEP_OFF) == 0 {
            // f32 / f32 rounds the same at the game's 53-bit x87 precision.
            (1u32, rf(PHYSICS_STEP) / rf(SLOW_MOTION))
        } else {
            (0, 0.0)
        };
        call_regs(FRAME_TIMER_UPDATE, 0, 0, 0, 0, 0, &[fixed, step.to_bits(), extra.to_bits()]);

        call(SERVICES_8217F0);
        call(NETWORK_PUMP);
        call(PROFILE_FRAME);
        call(SERVICES_69A440);
        profile_stop(&mut sections);
        game_state_frame();
        // The original stops the same marker twice ("sim_frame" is never stopped).
        profile_stop(&mut sections);
        profile_start(LABEL_SIM_HAND, &mut hand);
        call(FRAME_END_HANDOFF);
        profile_stop(&mut hand);
    }
    let _ = simfr;
    0
}

// --- game_state_frame (0x697780) -----------------------------------------------------------

const STATE_DEPTH: usize = 0x00E9_6B7C; // i32, index of the top
const STATE_STACK: usize = 0x0144_E760; // u32 states
const STATE_PENDING: usize = 0x0144_E75C; // also reads as STATE_STACK[-1]
const STATE_FLAGS: usize = 0x0144_E750; // u8 per state; bit 0 = the state below keeps running
const STATE_PUSH: usize = 0x0140_BBCA; // u8
const STATE_POP: usize = 0x0140_BBC9; // u8
const STATE_EXIT_SKIP: usize = 0x0252_7BF6; // u8, cleared each transition
const STATE_EXIT: usize = 0x0069_73E0; // EAX = state - 1, cdecl (next state)
const STATE_APPLY: usize = 0x0069_7550; // -> the new top state
const STATE_ENTER: usize = 0x0069_7330; // EAX = state - 1, EDI = state
const STATE_DISPATCH: usize = 0x0069_7490; // ESI = state
const STATE_COUNT: u32 = 11;
const ASPECT_CHANGED: usize = 0x0252_A26E; // u8
const BACKBUFFER_W: usize = 0x022F_DC1C; // i32
const BACKBUFFER_H: usize = 0x022F_DC20; // i32
const WIDESCREEN_ASPECT: usize = 0x00E5_C080; // f64, 1.45
const VINT_SET_WIDESCREEN: usize = 0x00B8_A760; // cdecl (u8 widescreen)
const STACK_DISPATCH: usize = 0x0069_7700;

fn valid(state: u32) -> bool {
    state.wrapping_sub(1) < STATE_COUNT
}

unsafe fn stack(level: i32) -> u32 {
    unsafe { rd((STATE_STACK as isize + level as isize * 4) as usize) }
}

/// Run the state transition, if one is pending, then the states. Returns the top state.
fn game_state_frame() -> u32 {
    unsafe {
        let depth = rd(STATE_DEPTH) as i32;
        let mut state = if depth >= 0 { stack(depth) } else { 0 };
        let pending = rd(STATE_PENDING);
        let pop = rb(STATE_POP) != 0;
        if pending != 0 || pop {
            if rb(STATE_PUSH) == 0 && rb(STATE_EXIT_SKIP) == 0 && valid(state) {
                // On a pop, the original passes the entry one below the top.
                let next = if pop { stack(depth - 1) } else { pending };
                call_regs(STATE_EXIT, state - 1, 0, 0, 0, 0, &[next]);
            }
            *(STATE_EXIT_SKIP as *mut u8) = 0;
            state = call_regs(STATE_APPLY, 0, 0, 0, 0, 0, &[]);
            if rb(ASPECT_CHANGED) == 1 {
                *(ASPECT_CHANGED as *mut u8) = 0;
                // fild / fidiv / fcomp at the game's 53-bit precision: an f64 divide.
                let aspect = rd(BACKBUFFER_W) as i32 as f64 / rd(BACKBUFFER_H) as i32 as f64;
                let wide = aspect > *(WIDESCREEN_ASPECT as *const f64);
                call_regs(VINT_SET_WIDESCREEN, 0, 0, 0, 0, 0, &[wide as u32]);
            }
            if !pop && valid(state) {
                call_regs(STATE_ENTER, state - 1, 0, 0, 0, state, &[]);
            }
        }
        stack_dispatch(rd(STATE_DEPTH) as i32, 0);
        state
    }
}

/// `game_state_stack_dispatch` (0x697700, cdecl (level, flag)): run the states from
/// the lowest one still running up to `level`. The flag is passed down and unused.
extern "C" fn stack_dispatch(level: i32, _flag: u32) {
    unsafe {
        let s = stack(level);
        let flags = rb(STATE_FLAGS + s as usize);
        if flags & 1 != 0 && level > 0 {
            stack_dispatch(level - 1, ((flags >> 1) & 1) as u32);
        }
        call_regs(STATE_DISPATCH, 0, 0, 0, s, 0, &[]);
    }
}

pub fn install() {
    match unsafe { detour::replace(STACK_DISPATCH, &STACK_DISPATCH_BODY, stack_dispatch as extern "C" fn(i32, u32) as usize) } {
        Ok(()) => log("frame: replaced game_state_stack_dispatch @ 0x697700 with Rust"),
        Err(e) => log(&format!("frame: replacing game_state_stack_dispatch failed: {e:?}")),
    }
}

const STACK_DISPATCH_BODY: [u8; 54] = [
    0x8B, 0x4C, 0x24, 0x04, 0x56, 0x8B, 0x34, 0x8D, 0x60, 0xE7, 0x44, 0x01, 0x0F, 0xB6, 0x86, 0x50,
    0xE7, 0x44, 0x01, 0xA8, 0x01, 0x74, 0x18, 0x85, 0xC9, 0x7E, 0x14, 0xD0, 0xE8, 0x83, 0xC1, 0xFF,
    0x25, 0x01, 0xFF, 0xFF, 0xFF, 0x50, 0x51, 0xE8, 0xD4, 0xFF, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0xE8,
    0x5C, 0xFD, 0xFF, 0xFF, 0x5E, 0xC3,
];
