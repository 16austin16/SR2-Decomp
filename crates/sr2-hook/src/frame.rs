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
//! states. Each state's enter / exit / frame functions stay the game's. So is
//! `frame_end_handoff` (0x521A00, 11_frame.md "frame_end_handoff at instruction level"):
//! the render snapshots, the render-list swap and the sync with the render thread; its
//! callees stay the game's.

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
        frame_end_handoff();
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

// --- frame_end_handoff (0x521A00) ------------------------------------------------------------

// The handoff to the render thread (11_frame.md, "frame_end_handoff at instruction level").
const RENDER_THREAD_WAIT_FRAME: usize = 0x0052_2B80; // cdecl (u8 first)
const FIRST_HANDOFF: usize = 0x00ED_4710; // u8
const DISPLAY_CHANGE_REQUEST: usize = 0x0252_7383; // u8, compared with 0x2527382 at the end
const RENDER_SYNC_PREV: usize = 0x0252_737E; // u8: last frame's 0x252737F
const RENDER_SYNC: usize = 0x0252_737F; // u8: set = don't wait for the render thread's frame-done
const RENDER_FRAME_DONE: usize = 0x0277_41AC; // u8, set by the render thread
const RENDER_ACK: usize = 0x0277_419C; // u8, set by the render thread when it wakes
const RENDER_BATCHES_BUILT: usize = 0x0277_41AD; // u8
const RENDER_JOB: usize = 0x0277_4198; // -> job; vtable +0xC signals the render thread
const HANDOFF_STARTED: usize = 0x0252_7363; // u8
const HANDOFF_ACTIVE: usize = 0x0252_737D; // u8
const GFX_DEVICE_ACQUIRE: usize = 0x00CF_BC00;
const GFX_IN_HANDOFF: usize = 0x00E8_49DB; // u8
const GFX_INVALIDATE_STATE: usize = 0x00D1_4950;
const GFX_APPLY_GAMMA: usize = 0x00D1_4770;
const GFX_SET_TINT: usize = 0x00D1_4700; // ESI = &rgb
const STATE_STREAM_FRAME: usize = 0x0069_7740; // cdecl (level, flag)
const GAMEPLAY_STREAMING_FRAME: usize = 0x0068_E410;
const FRAME_PRESENT: usize = 0x0052_17E0;
const AUDIO_UPDATE_VOICES: usize = 0x0046_55B0; // -> audio job or 0
const AUDIO_JOB_TAG: u32 = 0x033D_6328;
const AUDIO_JOB_POOL: usize = 0x00EC_5B98; // object; vtable +4 frees (job - 8)
const TEMP_BATCHING_RELEASE: usize = 0x0052_1970;
const TEMP_BATCHING_CREATE: usize = 0x0052_18D0;
const SNAPSHOT_COPY_VIEW: usize = 0x0052_20B0; // EAX = view
const SNAPSHOT_RESET_SCALES: usize = 0x0052_1870; // EAX = block
const VIEW: usize = 0x00E9_D620; // the render snapshot of the camera
const SCALES: usize = 0x00E9_D6D0;
const WORLD_VISIBLE: usize = 0x00E9_4D3E; // u8

const TOD_RANGE_ON: usize = 0x0252_6DC4; // u8
const TOD_IN_RANGE: usize = 0x004D_9900; // cdecl (from, to) -> al
const TOD_RANGE_FROM: usize = 0x0279_7784;
const TOD_RANGE_TO: usize = 0x0279_7788;
const TOD_RANGE_RESULT: usize = 0x00E8_49D0; // -1 / 0
const TOD_UPDATES: [usize; 7] =
    [0x004D_9DD0, 0x004D_4820, 0x004D_4F90, 0x004D_5470, 0x004D_5300, 0x004D_5670, 0x004E_69D0];
const LIGHTING_SRC: usize = 0x0265_2C80; // 0x260 dwords copied for the render thread
const LIGHTING_DST: usize = 0x0265_3600;
const SNAPSHOT_6255A0_ON: usize = 0x027A_3F18; // u8
const SNAPSHOTS_A: [usize; 3] = [0x0046_CCD0, 0x00D0_8310, 0x004C_7190]; // audio, decals, glares
const GLARE_FLAG_SRC: usize = 0x0252_73F2; // u8
const GLARE_FLAG_DST: usize = 0x0252_73F3;
// water, render params publish / reset, 0x9FAA20
const SNAPSHOTS_B: [usize; 4] = [0x0072_8AB0, 0x0051_A880, 0x0051_A4A0, 0x009F_AA20];
const CAMERA_POS: usize = 0x025F_5B14; // vec3
const CAMERA_MATRIX: usize = 0x025F_5B38; // 9 floats
const CAMERA_GET_FOV: usize = 0x0049_4080; // -> st0
const MATRIX_ORTHONORMALIZE: usize = 0x00BE_4170; // stdcall (&m)
const SKY_LIGHT_SNAPSHOT: usize = 0x004D_9A50; // ESI / EDI / ECX / EDX = out
const SNAPSHOT_B77CA0: usize = 0x00B7_7CA0;
const CAMERA_MODE: usize = 0x025F_5AE8;
const LOCAL_PLAYER: usize = 0x0217_03D4;
const LISTENER_POS: usize = 0x0277_C9D0; // vec3
const LISTENER_SCALE: usize = 0x00E9_896C; // f32
const HUMAN_9B92E0: usize = 0x009B_92E0; // EAX = human -> al
const OBJECT_AE28F0: usize = 0x00AE_28F0; // ECX = object -> eax
// 0xAE1020, render_lists_reset, game_render_func, game_build_render_batches
const AFTER_SNAPSHOTS: [usize; 4] = [0x00AE_1020, 0x0052_E7C0, 0x0052_50E0, 0x0052_2FC0];
const MENU_CAMERA_POS: usize = 0x0252_A6D0;
const MENU_CAMERA_MATRIX: usize = 0x00EC_5AE4;
const MENU_COLOURS: usize = 0x0252_A6F0; // 4 floats
const MENU_SKY: usize = 0x00EC_5ACC; // vec3

const RENDER_LISTS: usize = 0x0230_5978; // two list pointers, swapped each frame
const DEVICE_CS: u32 = 0x033D_62D0;
const DEVICE_THREAD: usize = 0x0252_A370;
const DEVICE_LOCKS: usize = 0x0252_A368;
const DEVICE_OWNER: usize = 0x0252_A374;
const IMP_GET_CURRENT_THREAD_ID: usize = 0x00DB_8080;
const IMP_LEAVE_CRITICAL_SECTION: usize = 0x00DB_80D0;
const IMP_SLEEP: usize = 0x00DB_80B8;

unsafe fn vb(p: usize) -> u8 {
    unsafe { std::ptr::read_volatile(p as *const u8) }
}

unsafe fn wb(p: usize, v: u8) {
    unsafe { std::ptr::write_volatile(p as *mut u8, v) }
}

unsafe fn wd(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}

unsafe fn copy_dwords(dst: usize, src: usize, n: usize) {
    unsafe { std::ptr::copy(src as *const u32, dst as *mut u32, n) }
}

unsafe fn import(slot: usize) -> usize {
    unsafe { rd(slot) as usize }
}

unsafe fn vslot(obj: u32, slot: usize) -> usize {
    unsafe { rd(rd(obj as usize) as usize + slot * 4) as usize }
}

/// `camera_get_fov`, which returns on the x87 stack.
unsafe fn camera_fov() -> f32 {
    let mut out = 0f32;
    unsafe {
        std::arch::asm!(
            "push edi",
            "call {f}",
            "pop edi",
            "fstp dword ptr [edi]",
            f = in(reg) CAMERA_GET_FOV,
            in("edi") &mut out as *mut f32,
            clobber_abi("C"),
        );
    }
    out
}

/// The render-snapshot fields both branches fill from the same places.
unsafe fn snapshot_common() {
    unsafe {
        wd(0xE9D67C, rd(0x0252_73A0));
        wd(0xE9D680, rd(0x00E9_8964));
        wd(0xE9D684, rd(0x00E9_8968));
        for (dst, src) in [
            (0xE9D664, 0xE9D7C8),
            (0xE9D668, 0xE9D7CC),
            (0xE9D66C, 0xE9D7D0),
            (0xE9D670, 0xE9D7D4),
            (0xE9D674, 0xE9D7D8),
            (0xE9D678, 0xE9D7DC),
        ] {
            wd(dst, rd(src));
        }
        wb(0xE9D68E, rb(0x00E9_96E5));
        wd(0xE9D688, rd(0x0252_A3C8));
        wb(0xE9D68C, 1);
    }
}

/// The world is drawn: time of day, the per-system render snapshots, the camera and
/// listener, then the render lists are built.
unsafe fn handoff_world() {
    unsafe {
        if rb(TOD_RANGE_ON) != 0 {
            let r = call_regs(TOD_IN_RANGE, 0, 0, 0, 0, 0, &[rd(TOD_RANGE_FROM), rd(TOD_RANGE_TO)]) as u8;
            wd(TOD_RANGE_RESULT, if r != 0 { u32::MAX } else { 0 });
        }
        for f in TOD_UPDATES {
            call(f);
        }
        copy_dwords(LIGHTING_DST, LIGHTING_SRC, 0x260);
        if rb(SNAPSHOT_6255A0_ON) != 0 {
            call(0x0062_55A0);
        }
        for f in SNAPSHOTS_A {
            call(f);
        }
        wb(GLARE_FLAG_DST, rb(GLARE_FLAG_SRC));
        for f in SNAPSHOTS_B {
            call(f);
        }
        copy_dwords(VIEW, CAMERA_POS, 3);
        copy_dwords(VIEW + 0xC, CAMERA_MATRIX, 9);
        let fov = camera_fov();
        // The original pushes the matrix before this store and calls after it.
        *((VIEW + 0x30) as *mut f32) = fov;
        call_regs(MATRIX_ORTHONORMALIZE, 0, 0, 0, 0, 0, &[(VIEW + 0xC) as u32]);
        snapshot_common();
        call_regs(SKY_LIGHT_SNAPSHOT, 0, 0xE9D6B0, 0xE9D6C0, 0xE9D690, 0xE9D69C, &[]);
        call(SNAPSHOT_B77CA0);

        let scale_bits;
        if rd(CAMERA_MODE) == 1 {
            copy_dwords(LISTENER_POS, CAMERA_POS, 3);
            scale_bits = rd(0x00DD_0550);
        } else {
            let h = rd(LOCAL_PLAYER) as usize;
            if h == 0 {
                copy_dwords(LISTENER_POS, MENU_CAMERA_POS, 3);
                scale_bits = 1f32.to_bits();
            } else {
                copy_dwords(LISTENER_POS, h + 0x14, 3);
                let vehicle = rd(h + 0x44);
                let mut near = false;
                if vehicle != 0 {
                    let c = rd(0x00EB_CE9C) as usize;
                    let st = rd(0x00EB_CEA0);
                    let check = c == 0 || vehicle != rd(c + 0x44) || st == 7 || st == 8;
                    near = check
                        && call_regs(HUMAN_9B92E0, h as u32, 0, 0, 0, 0, &[]) as u8 == 0
                        && call_regs(OBJECT_AE28F0, 0, rd(h + 0xD74), 0, 0, 0, &[]) != 0;
                }
                scale_bits = rd(if near { 0x00DD_0678 } else { 0x00DD_0794 });
            }
        }
        wd(LISTENER_SCALE, scale_bits);
        for f in AFTER_SNAPSHOTS {
            call(f);
        }
    }
}

/// No world (menus, loading): a fixed camera and colours.
unsafe fn handoff_menu() {
    unsafe {
        copy_dwords(VIEW, MENU_CAMERA_POS, 3);
        copy_dwords(VIEW + 0xC, MENU_CAMERA_MATRIX, 9);
        wd(VIEW + 0x30, rd(0x00DD_3448));
        snapshot_common();
        copy_dwords(0xE9D6B0, MENU_COLOURS, 4);
        copy_dwords(0xE9D6C0, MENU_COLOURS, 3);
        wd(0xE9D6CC, rd(MENU_COLOURS + 0xC));
        copy_dwords(0xE9D690, MENU_SKY, 3);
        copy_dwords(0xE9D69C, MENU_SKY, 3);
    }
}

/// `frame_end_handoff`: finish the game thread's frame and hand it to the render thread.
fn frame_end_handoff() {
    unsafe {
        let first = rb(FIRST_HANDOFF);
        wb(DISPLAY_CHANGE_REQUEST, 0);
        call_regs(RENDER_THREAD_WAIT_FRAME, 0, 0, 0, 0, 0, &[first as u32]);
        wb(FIRST_HANDOFF, 0);
        if rb(RENDER_SYNC_PREV) != 0 && rb(RENDER_SYNC) == 0 {
            // A busy spin, as in the original.
            while vb(RENDER_FRAME_DONE) == 0 {}
            wb(RENDER_FRAME_DONE, 0);
        }
        wb(HANDOFF_STARTED, 1);
        wb(RENDER_SYNC_PREV, rb(RENDER_SYNC));
        call(GFX_DEVICE_ACQUIRE);
        wb(GFX_IN_HANDOFF, 1);
        call(GFX_INVALIDATE_STATE);
        call(GFX_APPLY_GAMMA);
        let white = [1f32; 3];
        call_regs(GFX_SET_TINT, 0, 0, 0, white.as_ptr() as u32, 0, &[]);

        let depth = rd(STATE_DEPTH) as i32;
        let top = stack(depth);
        let flags = rb(STATE_FLAGS + top as usize);
        if flags & 1 != 0 && depth > 0 {
            call_regs(STATE_STREAM_FRAME, 0, 0, 0, 0, 0, &[(depth - 1) as u32, ((flags >> 1) & 1) as u32]);
        }
        if top == 5 {
            call(GAMEPLAY_STREAMING_FRAME);
        }
        call(FRAME_PRESENT);

        let depth = rd(STATE_DEPTH) as i32;
        let gameplay = (if depth > -1 { stack(depth) } else { 0 }) == 5;
        let job = if gameplay { call_regs(AUDIO_UPDATE_VOICES, 0, 0, 0, 0, 0, &[]) as usize } else { 0 };
        wb(HANDOFF_ACTIVE, 1);
        wb(RENDER_BATCHES_BUILT, 0);
        call(TEMP_BATCHING_RELEASE);
        call(TEMP_BATCHING_CREATE);
        call_regs(SNAPSHOT_COPY_VIEW, VIEW as u32, 0, 0, 0, 0, &[]);
        call_regs(SNAPSHOT_RESET_SCALES, SCALES as u32, 0, 0, 0, 0, &[]);
        if rb(WORLD_VISIBLE) != 0 {
            handoff_world();
        } else {
            handoff_menu();
        }

        // Swap the render lists; the render thread executes the one now in [0x2305978].
        let skip_present = rb(0x0252_7D14);
        let (a, b) = (rd(RENDER_LISTS), rd(RENDER_LISTS + 4));
        wd(RENDER_LISTS + 4, a);
        wd(RENDER_LISTS, b);
        wd(a as usize + 0x5DC0, 0);
        wd(a as usize + 0x1_C5C4, 0);
        wb(HANDOFF_ACTIVE, 0);
        let busy = (skip_present != 0 && rb(0x0252_7CF7) != 0) || rb(0x0252_7D15) != 0;
        wb(0x00E9_4D37, (!busy) as u8);

        if gameplay && job != 0 {
            wd(job + 0x34, AUDIO_JOB_TAG);
            call_regs(vslot(job as u32, 4), 0, job as u32, 0, 0, 0, &[]);
            wd(job + 4, 0);
            wb(job + 8, 0);
            wd(job + 0x34, 0);
            wb(job + 9, 0);
            wd(job + 0x3C, 0);
            wd(job + 0x40, 0);
            wd(job + 0x44, 0);
            call_regs(vslot(AUDIO_JOB_POOL as u32, 1), 0, AUDIO_JOB_POOL as u32, 0, 0, 0, &[(job - 8) as u32]);
        }
        wb(GFX_IN_HANDOFF, 0);
        call(GFX_INVALIDATE_STATE);

        // Release the device lock before waking the render thread, or it blocks on it.
        let tid: extern "system" fn() -> u32 = std::mem::transmute(import(IMP_GET_CURRENT_THREAD_ID));
        if tid() != rd(DEVICE_THREAD) {
            let n = rd(DEVICE_LOCKS).wrapping_sub(1);
            wd(DEVICE_LOCKS, n);
            if n == 0 {
                wd(DEVICE_OWNER, 0);
                let leave: extern "system" fn(u32) = std::mem::transmute(import(IMP_LEAVE_CRITICAL_SECTION));
                leave(DEVICE_CS);
            }
        }
        let job = rd(RENDER_JOB);
        call_regs(vslot(job, 3), 0, job, 0, 0, 0, &[]);
        let sleep: extern "system" fn(u32) = std::mem::transmute(import(IMP_SLEEP));
        while vb(RENDER_ACK) == 0 {
            sleep(1);
        }
        wb(RENDER_ACK, 0);
        if rb(RENDER_SYNC_PREV) == 0 {
            while vb(RENDER_FRAME_DONE) == 0 {
                sleep(1);
            }
            wb(RENDER_FRAME_DONE, 0);
        }

        // A display change requested this frame (0xE98948 / 0xE98994).
        let req = vb(DISPLAY_CHANGE_REQUEST);
        if rb(0x0252_7382) != req {
            if req != 0 {
                if rd(0x00E9_8948) == 2 {
                    wb(0x0252_7381, 1);
                    wd(0x00E9_89D8, 2);
                    wd(0x00E9_8948, 1);
                }
                let e = rd(0x00E9_8994);
                if e == 2 || e == 1 {
                    wb(0x0252_7380, 1);
                }
            } else {
                if rb(0x0252_7381) != 0 {
                    let v = rd(0x00E9_89D8);
                    wb(0x0252_7381, 0);
                    wd(0x00E9_8948, v);
                }
                if rb(0x0252_7380) != 0 {
                    wb(0x0252_7380, 0);
                }
            }
            wb(0x0252_7382, req);
        }
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
