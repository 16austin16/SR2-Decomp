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
//! callees stay the game's. And `sim_frame` (0x68CA70), the gameplay simulation step, as
//! a call sequence under its profiler sections.

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
        game_state_dispatch(s);
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

// --- sim_frame (0x68CA70) ---------------------------------------------------------------------

// The gameplay simulation step, called by `gameplay_do_frame` (docs/analysis/40, "sim_frame").
// A call sequence under the game's profiler sections; the callees stay the game's. Names for
// the calls are in docs/functions.csv and doc 40.
const SIM_FRAME: usize = 0x0068_CA70;
const SIM_FRAME_PROLOGUE: [u8; 9] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x4C];
/// Run sim_frame's body in Rust instead of the original.
const RUST_SIM: bool = true;

const LABEL_SIM_A: u32 = 0x00DF_FA88;
const LABEL_SIM_B: u32 = 0x00DF_FA98;
const LABEL_SIM_C: u32 = 0x00DF_FAA8;
const LABEL_SIM_D: u32 = 0x00DF_FAB8;
const LABEL_SIM_E: u32 = 0x00DF_FAC4;
const LABEL_SIM_F: u32 = 0x00DF_FAD0;
const SIM_PAUSED: usize = 0x0252_7C08; // i32 > 0: only the always-on parts run
const CUTSCENE_A: usize = 0x0252_7D14; // u8 (also read by the handoff)
const CUTSCENE_B: usize = 0x0252_7CF7; // u8
const CUTSCENE_C: usize = 0x0252_7D15; // u8
const EXTRA_HOOK: usize = 0x0212_A634; // optional function pointer
const PHYSICS_FRAME_TIME: usize = 0x006F_F860; // -> st0
const IMP_QPC: usize = 0x00DB_8114;
const QPC_ON: usize = 0x0252_A404; // u8
const MP_ACTIVE: usize = 0x0252_8B48; // u8
const MP_CLIENT: usize = 0x0252_8B49; // u8
const WORKER_A: u32 = 0x033D_63F8; // job worker control blocks (+0x38 work kind)
const WORKER_B: u32 = 0x033D_64C8;
const JOB_WORKER_WAKE: usize = 0x00BF_3760; // ESI = worker

/// Call a function that returns on the x87 stack, rounded to f32 as the callers store it.
unsafe fn call_f32(f: usize) -> f32 {
    let mut out = 0f32;
    unsafe {
        std::arch::asm!(
            "push edi",
            "call {f}",
            "pop edi",
            "fstp dword ptr [edi]",
            f = in(reg) f,
            in("edi") &mut out as *mut f32,
            clobber_abi("C"),
        );
    }
    out
}

unsafe fn calls(fs: &[usize]) {
    for &f in fs {
        unsafe { call(f) };
    }
}

unsafe fn call_f(f: usize, v: f32) {
    unsafe { call_regs(f, 0, 0, 0, 0, 0, &[v.to_bits()]) };
}

unsafe fn extra_hook() {
    unsafe {
        let f = rd(EXTRA_HOOK) as usize;
        if f != 0 {
            call(f);
        }
    }
}

unsafe fn qpc() {
    unsafe {
        if rb(QPC_ON) != 0 {
            let mut t = 0u64;
            let f: extern "system" fn(*mut u64) -> i32 = std::mem::transmute(import(IMP_QPC));
            f(&mut t);
        }
    }
}

unsafe fn wake_workers(kind: u32) {
    unsafe {
        wd(WORKER_A as usize + 0x38, kind);
        call_regs(JOB_WORKER_WAKE, 0, 0, 0, WORKER_A, 0, &[]);
        wd(WORKER_B as usize + 0x38, kind);
        call_regs(JOB_WORKER_WAKE, 0, 0, 0, WORKER_B, 0, &[]);
    }
}

fn cutscene() -> bool {
    unsafe { (rb(CUTSCENE_A) != 0 && rb(CUTSCENE_B) != 0) || rb(CUTSCENE_C) != 0 }
}

/// The calls run once early in the frame, and again late if the early run was skipped.
unsafe fn cutscene_updates() {
    unsafe {
        calls(&[0x0049_32F0, 0x00A7_0B40, 0x00A5_5310, 0x00A6_6340]);
        extra_hook();
    }
}

extern "C" fn sim_frame_hook() {
    if RUST_SIM {
        sim_frame();
    } else {
        let f: extern "C" fn() = unsafe { std::mem::transmute(SIM_ORIGINAL.load(Ordering::Relaxed)) };
        f();
    }
}

static SIM_ORIGINAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn sim_frame() {
    unsafe {
        let paused = rd(SIM_PAUSED) as i32 > 0;
        let mut early_cutscene = false;
        let (mut whole, mut section) = (0u32, 0u32);
        profile_start(LABEL_SIM_A, &mut whole);
        if !paused {
            call(0x006C_6870);
            if cutscene() {
                early_cutscene = true;
                call(0x0049_3260);
                cutscene_updates();
            }
            calls(&[0x0054_3920, 0x0068_F4B0, 0x0093_D150]);
        } else if rb(0x01F7_68DA) != 0 {
            call(0x0068_F4B0);
        }
        call(0x00CD_DE70);
        call_regs(0x006D_A350, 0, 0, 0, 0, 0, &[paused as u32]);

        // Drop a pending item reference whose slot no longer matches.
        let r = rd(0x00E9_DA40);
        if r as u8 != 0xFF && r >> 8 != 0 {
            let i = (r & 0xFF) as usize * 0x19C;
            if rd(0x00EE_B9AC + i) != r >> 8 || rd(0x00EE_B8F8 + i) == u32::MAX {
                wd(0x00E9_DA40, 0xFF);
                call(0x0054_CCA0);
            }
        }
        qpc();
        calls(&[0x006B_7020, 0x0069_05E0, 0x0075_A120, 0x004F_5040, 0x006A_B6B0]);
        match rd(0x00EA_AF84) {
            1 => call(0x006A_B240),
            2 => call(0x006A_B2D0),
            _ => {}
        }
        qpc();
        call(0x0076_5BB0);

        if !paused {
            profile_start(LABEL_SIM_B, &mut section);
            call_regs(0x0068_A1E0, 0, 0, 0, 0, 0, &[rd(0x00E8_437C), 0, 1, 0]);
            calls(&[
                0x0068_A320, 0x0068_B860, 0x00A1_7690, 0x00A1_CCC0, 0x00A0_AFF0, 0x00A0_BF10, 0x00A6_C350,
                0x00A5_8EE0, 0x0050_1FB0, 0x006A_5BB0, 0x007B_02E0, 0x0054_8260, 0x00A9_3FF0,
            ]);
            profile_stop(&mut section);
            profile_start(LABEL_SIM_C, &mut section);
            calls(&[0x0080_45F0, 0x0080_4850]);
            if rb(0x0252_8B56) != 0 {
                let a = rd(0x0252_A5F4);
                wb(0x0252_8B2B, 0);
                if a != 0 {
                    wd(0x0252_A5EC, a);
                }
                calls(&[0x0080_45F0, 0x0080_4850]);
                let a = rd(0x0252_A5F0);
                wb(0x0252_8B2B, 1);
                if a != 0 {
                    wd(0x0252_A5EC, a);
                }
            }
            let dt = call_f32(PHYSICS_FRAME_TIME);
            calls(&[0x00B0_8530, 0x006A_DB10]);
            call_f(0x009D_7F90, dt);
            call_f(0x009A_7080, dt);
            call(0x009A_7150);
            wb(0x0252_9686, 0);
            calls(&[0x009A_71B0, 0x0093_05F0, 0x0093_5820]);
            wd(0x0252_970C, 5);
            let host_only = rb(MP_ACTIVE) == 1 && rb(MP_CLIENT) == 0;
            if !(host_only && rb(0x0252_9262) != 0) {
                call(0x0043_7160);
            }
            if !host_only {
                calls(&[0x0055_BA40, 0x0056_BD60, 0x004B_11F0, 0x0055_07C0, 0x009F_5420, 0x0065_5750]);
                let o = rd(0x0140_928C);
                if o != 0 && call_regs(vslot(o, 3), 0, o, 0, 0, 0, &[]) as u8 == 0 {
                    let o = rd(0x0140_928C);
                    call_regs(vslot(o, 2), 0, o, 0, 0, 0, &[]);
                    wd(0x0140_928C, 0);
                }
                call(0x0054_2B00);
            }
            calls(&[0x0056_5DA0, 0x0060_3140, 0x00A9_5A30, 0x00AD_D120, 0x00AE_F510]);
            // The original stops this marker twice.
            profile_stop(&mut section);
            profile_stop(&mut section);
            profile_start(LABEL_SIM_D, &mut section);
            calls(&[0x00AE_3A80, 0x0092_B3B0, 0x009F_A620, 0x0091_C6C0, 0x00B8_0070, 0x00B7_CA80, 0x009D_F820]);
            wake_workers(4);
            call(0x0055_4E20);
            let dt = call_f32(PHYSICS_FRAME_TIME);
            call_f(0x009A_9890, dt);
            call(0x006F_A0E0);
            call_regs(0x009A_9B60, 0, 0, 0, 0, 0, &[dt.to_bits(), 0]);
            call(0x004C_2BF0);
            wake_workers(3);
            call_f(0x009A_9CE0, dt);
            call(0x0071_B910);
            profile_stop(&mut section);
            profile_start(LABEL_SIM_E, &mut section);
            calls(&[0x00A8_B400, 0x0095_A3D0, 0x00AE_3C60, 0x00B0_95D0]);
            if rd(0x00EA_1020) == 2 {
                call(0x0063_4350);
            }
            calls(&[
                0x009A_7210, 0x0094_C0A0, 0x0068_8FE0, 0x00B7_2FD0, 0x004B_D790, 0x00B6_A4A0, 0x004D_1D50,
                0x0093_0EC0, 0x009F_A670, 0x009F_F5A0, 0x00A5_E3E0, 0x0093_5B80, 0x0049_0940, 0x0048_9CC0,
                0x004A_AC40, 0x004A_B140, 0x0091_93C0, 0x0055_85E0,
            ]);
            if !early_cutscene && !(rb(CUTSCENE_A) != 0 && rb(CUTSCENE_B) != 0) && rb(CUTSCENE_C) == 0 {
                cutscene_updates();
            }
            profile_stop(&mut section);
            profile_start(LABEL_SIM_F, &mut section);
            calls(&[0x00B6_AD40, 0x004D_E770, 0x004D_6850, 0x004E_66A0, 0x004C_F840, 0x004C_A750, 0x0050_3AE0, 0x004C_BFC0]);

            // Every ~2 s of game time (wrapping), clamp the local player's weapon ammo
            // counters at 0 for the 9 slots.
            let t = rd(0x00EA_ABD4) as i32;
            let now = rd(0x00E8_43AC) as i32;
            let recent = t >= 0
                && if t > now { t.wrapping_sub(now) < 0x35A4_E900 } else { now.wrapping_sub(t) >= 0x35A4_E900 };
            if !recent {
                let mut v = now.wrapping_add(2000);
                wd(0x00EA_ABD4, v as u32);
                if v > 0x6B49_D200 {
                    v -= 0x6B49_D200;
                    wd(0x00EA_ABD4, v as u32);
                }
                let p = rd(LOCAL_PLAYER);
                if p != 0 {
                    for slot in 0..9u32 {
                        let w = call_regs(0x0055_ADF0, p + 0x10C4, 0, 0, slot, 0, &[]) as usize;
                        if w != 0 && rd(w + 0xC4) != 0 {
                            for o in [0x10, 0x14, 0x18, 0x1C] {
                                let a = rd(w + 0xC4) as usize + o;
                                if (rd(a) as i32) < 0 {
                                    wd(a, 0);
                                }
                            }
                        }
                    }
                }
            }
            let dt = call_f32(PHYSICS_FRAME_TIME);
            call_f(0x0072_7610, dt);
            calls(&[0x0095_5470, 0x0050_53B0, 0x0051_42A0, 0x0091_A840]);
            if rb(CUTSCENE_A) == 0 || rb(rd(0x0252_7D10) as usize + 0xAFF) != 0 {
                calls(&[0x0045_25E0, 0x0045_2820, 0x0045_3120]);
            }
            let mut focus = [0u32; 3];
            let mut focus2 = [0u32; 3];
            call_regs(0x009D_F4C0, focus.as_mut_ptr() as u32, 0, 0, 0, 0, &[focus2.as_mut_ptr() as u32]);
            let free_camera = rd(CAMERA_MODE) == 0 && !(rb(CUTSCENE_A) != 0 && rb(CUTSCENE_B) != 0) && rb(CUTSCENE_C) == 0;
            copy_dwords(0x02F9_B7E4, focus.as_ptr() as usize, 3);
            copy_dwords(0x02F9_B7F0, focus2.as_ptr() as usize, 3);
            wb(0x0213_B397, free_camera as u8);
            calls(&[0x0092_6170, 0x004C_6CD0, 0x004C_35C0, 0x00D0_7B40, 0x004D_A880, 0x0092_D620, 0x009F_CF90, 0x00B8_2180]);
            let a = call_regs(0x00D2_2790, 0, 0, 0, 0, 0, &[]) as usize;
            let b = call_regs(0x00D2_2790, 0, 0, 0, 0, 0, &[]) as usize;
            // Two 4x4 matrices by value: b + 0x44, then a + 0x84.
            let mut m = [0u32; 32];
            copy_dwords(m.as_mut_ptr() as usize, b + 0x44, 16);
            copy_dwords(m.as_mut_ptr() as usize + 64, a + 0x84, 16);
            call_regs(0x00B8_21E0, 0, 0, 0, 0, 0, &m);
            profile_stop(&mut section);
        }

        if !(rb(MP_ACTIVE) == 1 && rb(MP_CLIENT) == 0) {
            calls(&[0x006B_CC60, 0x0054_C220, 0x0068_7B40]);
        }
        call(0x0055_F1D0);
        if rb(0x0252_863F) == 0 && rb(0x0252_8775) != 0 {
            if call_regs(0x007F_7AD0, 0, 0, 0, 0, 0, &[]) as u8 == 0 {
                wb(0x0252_8775, 0);
            } else {
                let a = rd(0x0252_8778);
                if a == 0 {
                    wb(0x0252_8775, 0);
                } else {
                    call_regs(0x007A_8F70, 0, 0, 0, 0, 0, &[a, rd(LOCAL_PLAYER)]);
                }
            }
        }
        if rd(0x0252_6CF8) != 0 {
            call(0x004B_C420);
        }
        call(0x004C_4210);
        call_regs(0x0091_E9F0, 0, 0, 0, 0, 0, &[CAMERA_POS as u32]);
        calls(&[0x004D_9F20, 0x004C_2DD0, 0x0068_3E00]);
        if rb(0x0252_88AE) != 0 {
            let p = rd(LOCAL_PLAYER);
            if call_regs(0x009E_99F0, 0, 0, 0, p, rd(p as usize + 0x13B0), &[]) as u8 != 0 {
                call_regs(0x0092_6860, 0, p, 0, 0, 0, &[0, 1, 0]);
                wb(0x0252_88AE, 0);
            }
        }
        if rb(MP_ACTIVE) != 0 {
            call(0x0083_6BB0);
            let m = rd(0x0252_8C14) as usize;
            if m != 0 {
                let direct = rb(0x0252_8B56) != 0 && rb(0x0252_8B2B) == 0;
                if direct || rb(m + 0x3F8) == 0 {
                    let n = rd(m + 0x404) as usize;
                    if n != 0 && rb(n + 0x1B8) == 0 && rb(0x01F7_68DA) != 0 {
                        wb(0x01F7_68DA, 0);
                    }
                }
            }
        }
        if rb(0x0252_73AC) != 0 {
            call_regs(0x00D2_4B70, CAMERA_POS as u32, 0, 0, 0, 0, &[CAMERA_MATRIX as u32, rd(0x00DD_32B4)]);
        } else {
            let fov = call_f32(CAMERA_GET_FOV);
            call_regs(0x00D2_4D80, CAMERA_POS as u32, 0, 0, 0, 0, &[CAMERA_MATRIX as u32, fov.to_bits(), 1, 0]);
        }
        calls(&[0x0079_3A40, 0x007F_3500]);
        let _ = whole;
    }
}

// --- loading_state_frame (0x73B7F0, game state 10) --------------------------------------------

// The loading screen's frame (docs/analysis/40, "Other game states"): fade, the loading UI,
// the co-op join wait, the fade to black, then it replaces itself with gameplay (5) or the
// front end (2). Callees stay the game's.
const LOADING_FRAME: usize = 0x0073_B7F0;
const SCREEN_FADE_APPLY: usize = 0x004D_1790;
const SCREEN_FADE_START: usize = 0x004D_1090; // cdecl (ms, 0, f32 target)
const SCREEN_FADE: usize = 0x0252_6DA0; // f32, 0 = clear, 1 = black
const GFX_SET_COLOR: usize = 0x00D1_4840; // cdecl (r, g, b, a)
const LOAD_RESULT: usize = 0x00E8_EB88; // 1 = loaded, -1 = failed, 0 = still loading
const LOAD_TO_FRONTEND: usize = 0x0252_7E80; // u8
const LOAD_FADED: usize = 0x0252_7E81; // u8
const COOP_WAIT_STARTED: usize = 0x0252_7E82; // u8
const COOP_WAIT_DONE: usize = 0x0252_7E83; // u8
const COOP_TIMER: usize = 0x034A_5218; // timestamp (ms)
const COOP_TIMER_INIT: usize = 0x034A_521C; // bit 0
const TIMESTAMP_EXPIRED: usize = 0x00BD_D6A0; // EAX = &timestamp -> al
const COOP_PARTNER: usize = 0x009D_7BD0; // -> partner human or 0
const SESSION: usize = 0x0252_8C14;
const STREAMMAN_PROCESS: usize = 0x00A7_D6D0;
const NET_TIMED_CALLBACKS: usize = 0x0083_6BB0;
const GFX_DEVICE_RELEASE: usize = 0x00CF_BBC0;

unsafe fn timer_expired() -> bool {
    unsafe { call_regs(TIMESTAMP_EXPIRED, COOP_TIMER as u32, 0, 0, 0, 0, &[]) as u8 != 0 }
}

unsafe fn net_pump_tail() {
    unsafe { calls(&[0x00B0_A7B0, 0x0091_14B0]) };
}

/// The co-op join wait. Returns true when this frame ends here (the original returns
/// from inside the wait).
unsafe fn coop_wait() -> bool {
    unsafe {
        let s = rd(SESSION);
        let wait = rb(COOP_WAIT_DONE) == 0
            && rd(LOAD_RESULT) == 1
            && rb(MP_ACTIVE) == 1
            && rb(MP_CLIENT) == 1
            && s != 0
            && call_regs(0x0080_FCF0, 0, 0, 0, 0, 0, &[s]) as u8 == 0
            && rb(0x0252_7BA0) == 0;
        if !wait {
            return false;
        }
        if rb(COOP_TIMER_INIT) & 1 == 0 {
            wd(COOP_TIMER_INIT, rd(COOP_TIMER_INIT) | 1);
            wd(COOP_TIMER, u32::MAX);
        }
        if rb(COOP_WAIT_STARTED) == 0 {
            let esi = rd(s as usize + 0x408);
            wb(COOP_WAIT_STARTED, 1);
            call_regs(0x007F_8BA0, 0, 0xB, 0, esi, 0, &[]);
            let mut v = rd(0x00E8_43B4).wrapping_add(30000) as i32;
            wd(COOP_TIMER, v as u32);
            if v > 0x6B49_D200 {
                v -= 0x6B49_D200;
                wd(COOP_TIMER, v as u32);
            }
            wb(0x0252_7E8C, 1);
        }
        if call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]) == 0 && !timer_expired() {
            return true;
        }
        // Partner not here yet: pump the network and streaming only.
        if !timer_expired() && call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]) != 0 {
            let p = call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]) as usize;
            if rb(p + 0x129D) == 0 {
                call(NET_TIMED_CALLBACKS);
                net_pump_tail();
                call_regs(0x0094_7570, 0, 0, 0, 0, 0, &[0]);
                call(STREAMMAN_PROCESS);
                return true;
            }
        }
        // Partner still loading: also pump its loader inside a device acquire / release.
        if call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]) != 0 {
            let p = call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]);
            if call_regs(0x009E_99F0, 0, 0, 0, 0, rd(p as usize + 0x13B0), &[]) as u8 == 0 {
                calls(&[STREAMMAN_PROCESS, 0x009E_3AF0, GFX_DEVICE_ACQUIRE, 0x009E_3B10, GFX_DEVICE_RELEASE]);
                call(NET_TIMED_CALLBACKS);
                net_pump_tail();
                return true;
            }
        }
        // Streaming requests still queued.
        if rd(0x02F9_B88C) != 0 && !timer_expired() {
            call(NET_TIMED_CALLBACKS);
            call_regs(0x0094_7570, 0, 0, 0, 0, 0, &[0]);
            calls(&[STREAMMAN_PROCESS, 0x0081_7BA0]);
            net_pump_tail();
            return true;
        }
        wb(COOP_WAIT_STARTED, 0);
        wb(COOP_WAIT_DONE, 1);
        false
    }
}

/// Replace the loading state with `state` when no state change is pending.
unsafe fn loading_leave(state: u32) {
    unsafe {
        if rd(STATE_PENDING) == 0 && rb(STATE_PUSH) == 0 && rb(STATE_POP) == 0 {
            let f = (STATE_FLAGS + state as usize) as *mut u8;
            *f &= 0xFC;
            wd(STATE_PENDING, state);
            wb(STATE_POP, 0);
            wb(STATE_PUSH, 0);
        }
    }
}

extern "C" fn loading_state_frame() {
    unsafe {
        call(SCREEN_FADE_APPLY);
        let depth = rd(STATE_DEPTH) as i32;
        if depth > -1 && stack(depth) == 10 {
            calls(&[0x007F_3500, 0x007F_33B0, 0x0073_EDC0]); // interface_do_frame / _render, loading audio
        }
        if coop_wait() {
            return;
        }
        if rb(LOAD_FADED) == 0 {
            let fade = *(SCREEN_FADE as *const f32);
            if rd(LOAD_RESULT) != 0 && fade == 0.0 {
                let ms = if rd(0x0212_FEC4) == 6 { 0 } else { 250 };
                call_regs(GFX_SET_COLOR, 0, 0, 0, 0, 0, &[0, 0, 0, 0xFF]);
                call_regs(SCREEN_FADE_START, 0, 0, 0, 0, 0, &[ms, 0, 1f32.to_bits()]);
                return;
            }
            if rd(LOAD_RESULT) == 0 || fade != 1.0 {
                return;
            }
            wb(LOAD_FADED, 1);
        }
        let r = rd(LOAD_RESULT);
        if r == 0 {
            return;
        }
        if r == u32::MAX || rb(LOAD_TO_FRONTEND) != 0 {
            loading_leave(2);
        } else if r == 1 {
            loading_leave(5);
        }
    }
}

// --- gameplay_do_frame (0x68E1B0, game state 5) ------------------------------------------------

// The gameplay state's frame (11_frame.md, "gameplay_do_frame and gameplay_streaming_frame at
// instruction level"): death callbacks for humans whose death time just passed, the local
// player's respawn, sim_frame, then the gameplay UI.
const GAMEPLAY_FRAME: usize = 0x0068_E1B0;
const LABEL_GAMEPLAY: u32 = 0x00DF_FB6C;
const LABEL_GAMEPLAY_UI: u32 = 0x00DF_FB80;
const HUMANS: usize = 0x0217_043C; // circular list, next at +0x1294
const HUMAN_CALLBACK_THREAD_BEGIN: usize = 0x0042_7410; // ECX = callback block, EDX = &out -> thread entry
const OBJECT_SCRIPT_NAME: usize = 0x00A6_1C60; // EDX = object -> name or 0
const LUA_PUSHSTRING: usize = 0x00CD_CAF0; // ESI = L, cdecl (str)
const SCRIPT_THREAD_PUSH_STRING: usize = 0x00CE_01B0; // EDI = entry, EAX = string
const SCRIPT_THREAD_FIND: usize = 0x00CD_FEB0; // cdecl (id)
const PLAYER_DEATH_RESPAWN: usize = 0x0068_D570; // cdecl (human)
const OBJECT_HANDLES: usize = 0x0214_9C64; // 16-byte slots by handle & 0xFFF

/// Push nil as the next argument of a script thread entry.
unsafe fn thread_push_nil(e: usize) {
    unsafe {
        let l = rd(e + 4) as usize;
        let t = rd(l + 8) as usize;
        wd(t, 0);
        wd(l + 8, t as u32 + 16);
        wd(e + 0x1C, rd(e + 0x1C) + 1);
    }
}

/// A human's death callback: the thread gets the human's name and the killer's (or nil).
unsafe fn death_callback(h: usize) {
    unsafe {
        let mut out = 0u32;
        let e = call_regs(HUMAN_CALLBACK_THREAD_BEGIN, 0, h as u32 + 0x1234, &mut out as *mut u32 as u32, 0, 0, &[])
            as usize;
        if e == 0 {
            return;
        }
        let name = call_regs(OBJECT_SCRIPT_NAME, 0, 0, h as u32, 0, 0, &[]);
        if name == 0 {
            thread_push_nil(e);
        } else {
            call_regs(LUA_PUSHSTRING, 0, 0, 0, rd(e + 4), 0, &[name]);
            wd(e + 0x1C, rd(e + 0x1C) + 1);
        }
        let k = rd(h + 0xC0C);
        let rec = if k != 0 { rd(OBJECT_HANDLES + (k as usize & 0xFFF) * 16) } else { 0 };
        if rec != 0 && rd(rec as usize + 0x44) == k && rd(rec as usize + 0x4C) == 1 {
            let n = call_regs(OBJECT_SCRIPT_NAME, 0, 0, rec, 0, 0, &[]);
            call_regs(SCRIPT_THREAD_PUSH_STRING, n, 0, 0, 0, e as u32, &[]);
        } else {
            thread_push_nil(e);
        }
    }
}

extern "C" fn gameplay_do_frame() {
    unsafe {
        let (mut marker, mut ui_marker) = (0u32, 0u32);
        profile_start(LABEL_GAMEPLAY, &mut marker);
        wb(0x0252_7BA2, 1);
        call(0x007F_4E90);
        // The original loads 0x2527CF7 here but never tests it.
        if rb(CUTSCENE_A) == 0 && rb(CUTSCENE_C) == 0 {
            call(0x006B_5810);
        }
        let mut h = rd(HUMANS) as usize;
        while h != 0 {
            let t = rd(h + 0x122C) as i32;
            let now = rd(0x00E8_43AC) as i32;
            let due = t >= 0
                && if t > now { t.wrapping_sub(now) >= 0x35A4_E900 } else { now.wrapping_sub(t) < 0x35A4_E900 };
            if due {
                let local = h as u32 == rd(LOCAL_PLAYER);
                let flags = rd(h + 0xC0);
                if flags & 1 == 0 {
                    wd(h + 0xC0, flags | 1);
                    death_callback(h);
                }
                let id = *((h + 0x1290) as *const u16) as u32;
                if call_regs(SCRIPT_THREAD_FIND, 0, 0, 0, 0, 0, &[id]) == 0 {
                    let respawn = local && rb(h + 0xFA4) & 0x10 != 0;
                    *((h + 0x1290) as *mut u16) = 0xFFFF;
                    if respawn {
                        call_regs(RENDER_THREAD_WAIT_FRAME, 0, 0, 0, 0, 0, &[0]);
                        call_regs(PLAYER_DEATH_RESPAWN, 0, 0, 0, 0, 0, &[h as u32]);
                    }
                }
            }
            h = rd(h + 0x1294) as usize;
            if h as u32 == rd(HUMANS) {
                break;
            }
        }
        if rb(0x02A1_890F) != 0 {
            call(0x0081_EDF0);
            std::ptr::write_bytes(0x02A1_87D0 as *mut u8, 0, 0x144);
        }
        call(SIM_FRAME);
        profile_stop(&mut marker);
        profile_start(LABEL_GAMEPLAY_UI, &mut ui_marker);
        if rb(0x0279_C0C0) != 0 {
            call(0x0068_C770);
        }
        call(0x0068_D2F0); // gameplay_ui_frame
        if rb(0x0252_976C) != 0 {
            call_regs(GFX_SET_COLOR, 0, 0, 0, 0, 0, &[0, 0, 0xFF, 0xFF]);
            call_regs(0x00D1_3A60, 0x02CE_30D0, 0, 0, 0, 0, &[rd(0x00DD_1C8C), 0x00EC_2748]);
        }
        wb(0x00E9_D73D, 1);
        profile_stop(&mut ui_marker);
    }
}

// --- vint_update_documents (0xB8BC10) -----------------------------------------------------------

// Per-frame vint document update (06_vint.md, "interface_do_frame and vint_update_documents at
// instruction level"): the dt rules, each unpaused document's clock, the (empty) tree updates,
// then the unload pass.
const VINT_UPDATE_DOCUMENTS: usize = 0x00B8_BC10; // cdecl (f32 dt); writes the dt it used back
const VINT_ASPECT_RELOAD: usize = 0x0252_A17D; // u8
const VINT_FRAME_DT: usize = 0x00E8_4380; // f32
const VINT_DT_MAX: usize = 0x00DD_1C8C; // f32 0.5
const VINT_DT_MIN: usize = 0x00DD_5984; // f32 0.001
const DOC_LIST: usize = 0x0252_A1AC;
const DOC_UNLOAD_HOOK: usize = 0x0252_A1EC;

extern "C" fn vint_update_documents(arg: *mut f32) {
    unsafe {
        if rb(VINT_ASPECT_RELOAD) != 0 {
            call(0x00B8_A6F0);
        }
        wd(0x0252_A1B0, 0);
        let mut dt = *arg;
        if dt < 0.0 {
            dt = *(VINT_FRAME_DT as *const f32);
            *arg = dt;
        }
        let max = *(VINT_DT_MAX as *const f32);
        let step = if !(max > dt) {
            // At or over the cap (or NaN): clamp, and skip the low test.
            dt = max;
            *arg = dt;
            true
        } else {
            *arg = dt;
            dt > *(VINT_DT_MIN as *const f32)
        };
        if step {
            let head = rd(DOC_LIST) as usize;
            let mut d = head;
            while d != 0 {
                if rb(d + 0x460) == 0 {
                    let t = (d + 0x45C) as *mut f32;
                    *t += if 0.0 < dt { dt } else { *(VINT_FRAME_DT as *const f32) };
                }
                call_regs(0x00B8_AC40, rd(d + 0x454), 0, 0, 0, 0, &[]);
                call_regs(0x00B8_AC40, rd(d + 0x458), 0, 0, 0, 0, &[]);
                d = rd(d) as usize;
                if d == rd(DOC_LIST) as usize {
                    break;
                }
            }
        }
        // Unload the documents whose reference count dropped (+0x572 == 1).
        let mut head = rd(DOC_LIST);
        let mut d = head as usize;
        while d != 0 {
            let next = rd(d);
            let after = if next != head { next } else { 0 };
            if rb(d + 0x572) == 1 {
                call_regs(0x00B9_1400, 0, 0, 0, 0, 0, &[d as u32]);
                let hook = rd(DOC_UNLOAD_HOOK) as usize;
                if hook != 0 {
                    call_regs(hook, 0, 0, 0, 0, 0, &[d as u32]);
                }
                call_regs(0x00B8_A9F0, 0, 0, 0, d as u32, 0, &[]);
                call_regs(0x00B8_ABD0, d as u32, 0, 0, 0, 0, &[]);
                head = rd(DOC_LIST);
            }
            d = after as usize;
        }
    }
}

#[unsafe(naked)]
extern "C" fn vint_update_documents_thunk() {
    std::arch::naked_asm!(
        "lea eax, [esp + 4]",
        "push eax",
        "call {f}",
        "add esp, 4",
        "ret",
        f = sym vint_update_documents,
    )
}

// --- interface_do_frame (0x7F3500) --------------------------------------------------------------

// The vint UI's frame (06_vint.md, "interface_do_frame and vint_update_documents at instruction
// level"): the in-vehicle control hints, screen state, virtual keyboard, data items / groups,
// tweens, then the documents.
const INTERFACE_DO_FRAME: usize = 0x007F_3500;
const INTERFACE_READY: usize = 0x0252_8AFF; // u8
const IN_VEHICLE_HINTS: usize = 0x0347_AF34; // u8, last state

extern "C" fn interface_do_frame() {
    unsafe {
        if rb(INTERFACE_READY) == 0 {
            return;
        }
        let p = rd(LOCAL_PLAYER);
        if p != 0 {
            let entering = call_regs(0x009B_9030, 0, 0, 0, 0, 0, &[p]) as u8 != 0;
            let state = entering || call_regs(0x009B_9070, 0, 0, 0, 0, 0, &[p]) as u8 != 0;
            if state as u8 != rb(IN_VEHICLE_HINTS) {
                wb(IN_VEHICLE_HINTS, state as u8);
                call(0x007F_2040); // hud_control_hints_rebuild
            }
        }
        calls(&[0x007E_C170, 0x007F_3450, 0x00B9_5290, 0x00B9_4560, 0x00B9_EA70]);
        // -1.0: use the frame dt.
        call_regs(VINT_UPDATE_DOCUMENTS, 0, 0, 0, 0, 0, &[rd(0x00E8_7998)]);
    }
}

// --- frame_present (0x5217E0) -------------------------------------------------------------------

// The one-shot clear, present, the screenshot-mode flag and the screenshot save (11_frame.md,
// "frame_present and the render thread at instruction level").
const FRAME_PRESENT_FN: usize = 0x0052_17E0;
const CLEAR_ONCE: usize = 0x00E9_4D3F; // u8
const SCREENSHOT_MODE: usize = 0x00E9_8978; // u32 0 / 1

extern "C" fn frame_present() {
    unsafe {
        if rb(CLEAR_ONCE) != 0 {
            call_regs(0x00D1_8B30, 0, 0, 0, 0, 0, &[7, 0xFF00_0000, 1f32.to_bits(), 0]); // gfx_cmd_clear
            call(GFX_INVALIDATE_STATE);
            call_regs(0x00D1_48D0, 0, 0x00EC_2748, 0, 0, 0, &[]);
            wb(CLEAR_ONCE, 0);
        }
        call(0x00D1_5110); // gfx_present
        let on = if rb(0x0252_7392) != 0 {
            rb(0x0252_7391) != 0
        } else {
            rb(0x00E9_8970 + rd(0x00E9_8974) as usize) != 0
        };
        if rd(SCREENSHOT_MODE) != on as u32 {
            wd(SCREENSHOT_MODE, on as u32);
        }
        call(0x0069_9480); // screenshot_save, returns early unless one was asked for
    }
}

// --- gameplay_streaming_frame (0x68E410) --------------------------------------------------------

// Gameplay streaming, run inside frame_end_handoff (11_frame.md, "gameplay_do_frame and
// gameplay_streaming_frame at instruction level"): chunk requests, the streaming view, chunk
// activation, world streaming and the character asset streams.
const GAMEPLAY_STREAMING_FRAME_FN: usize = 0x0068_E410;
const THREAD_VIEW: usize = 0x00D2_2790; // -> the current thread's view (position +0, matrix +0xC)
const CHUNK_COUNT: usize = 0x00F6_FBD4; // i32
const CHUNKS: usize = 0x00F7_0B98; // chunk pointers, state at +0x18

extern "C" fn gameplay_streaming_frame() {
    unsafe {
        call(0x00A7_B880); // streaming_chunk_requests_update
        let v = call_regs(THREAD_VIEW, 0, 0, 0, 0, 0, &[]) as usize;
        let mut matrix = [0u32; 9];
        copy_dwords(matrix.as_mut_ptr() as usize, v + 0xC, 9);
        let v = call_regs(THREAD_VIEW, 0, 0, 0, 0, 0, &[]) as usize;
        let mut pos = [0u32; 3];
        copy_dwords(pos.as_mut_ptr() as usize, v, 3);
        call_regs(0x004B_39A0, 0, 0, 0, 0, 0, &[pos.as_ptr() as u32, matrix.as_ptr() as u32]); // city_streaming_set_view
        let mut i = 0;
        while i < rd(CHUNK_COUNT) as i32 {
            let c = rd(CHUNKS + i as usize * 4);
            match rd(c as usize + 0x18) {
                1 => {
                    call_regs(0x004A_FC70, 0, 0, 0, 0, 0, &[c]); // chunk_activate
                }
                2 => {
                    wd(c as usize + 0x18, 3);
                    call_regs(0x004A_A840, c, 0, 0, 0, 0, &[]); // chunk_ready_notify
                    call(0x0056_2100);
                }
                _ => {}
            }
            i += 1;
        }
        calls(&[0x0068_ED30, 0x0068_C950, 0x007D_6760, 0x009E_8400, 0x009E_CEA0, 0x009E_F5C0]);
        let head = rd(0x0217_068C);
        let mut o = head;
        while o != 0 {
            wb(o as usize + 0x15, 0);
            o = rd(o as usize + 0x1C);
            if o == head {
                break;
            }
        }
        calls(&[0x00AC_5FB0, 0x006C_6C60, 0x006E_79D0]);
        if rd(0x0252_9E20) != 0 && (0..6).all(|k| rb(0x0215_DA04 + k) != 0) {
            call(0x00B7_C210);
        }
        calls(&[0x007B_FC90, 0x00A5_4870]);
    }
}

// --- game_state_dispatch (0x697490) and hud_render (0x75A230) ---------------------------------

// 11_frame.md, "game_state_dispatch, overlays and the gameplay UI at instruction level".
const GAME_STATE_DISPATCH: usize = 0x0069_7490; // ESI = state
const HUD_RENDER: usize = 0x0075_A230;
const OVERLAYS_UPDATE: usize = 0x0075_B220;
const OVERLAYS_RENDER: usize = 0x0075_B270;

/// Each state's frame function (index = state - 1; 0 = none).
const STATE_FRAMES: [usize; 11] = [
    0,
    0x0073_DCB0, // 2 front end
    0,
    0,
    GAMEPLAY_FRAME, // 5
    0,
    0x0083_0AB0, // 7 multiplayer lobby
    0x0083_4D40, // 8 multiplayer setup
    0x0082_F570, // 9 multiplayer results
    LOADING_FRAME, // 10
    0x0068_8FE0, // 11 full-screen video
];

/// Overlays update (not in state 4), the state's frame, overlays render (not in 2 or 6).
fn game_state_dispatch(state: u32) {
    unsafe {
        if state != 4 {
            call(OVERLAYS_UPDATE);
        }
        if valid(state) {
            let f = STATE_FRAMES[state as usize - 1];
            if f != 0 {
                call_regs(f, 0, 0, 0, state, 0, &[]);
            }
        }
        if state != 2 && state != 6 {
            call(OVERLAYS_RENDER);
        }
    }
}

extern "C" fn game_state_dispatch_esi(state: u32) {
    game_state_dispatch(state);
}

#[unsafe(naked)]
extern "C" fn game_state_dispatch_thunk() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push esi",
        "call {f}",
        "add esp, 4",
        "pop edx",
        "pop ecx",
        "ret",
        f = sym game_state_dispatch_esi,
    )
}

/// The HUD element callbacks in their order, letterbox, player extras, dialog boxes, then the
/// vint documents.
extern "C" fn hud_render() {
    unsafe {
        let mut i = 0;
        while i < rd(0x0252_7FF4) as i32 {
            let idx = rd(0x01F7_68E4 + i as usize * 4) as usize;
            let cb = rd(0x00E8_E730 + idx * 0x14 + 0xC) as usize;
            if cb != 0 {
                call(cb);
            }
            i += 1;
        }
        calls(&[0x0075_5B80, 0x006C_6BE0]); // letterbox_render, player_hud_extras_render
        call_regs(0x0075_52D0, 0, 0, 0, 0, 0, &[0x00EA_C208]); // dialog_boxes_render
        call(0x007F_33B0); // interface_render
    }
}

// --- sim_frame list-walkers -------------------------------------------------------------------

// Ten per-frame list walkers called from sim_frame (doc 40, "Ten list-walkers at instruction
// level"). Shape A computes the next node before the body (the body may free the node); shape
// B reads it after (the body must not free it). The per-item functions stay the game's.
// Each is entered through a thunk that keeps ECX / EDX, which some originals preserve.

/// Shape A over a circular list: `next = link != head ? link : 0`, read before the body.
unsafe fn walk_a(head: usize, link: usize, mut body: impl FnMut(u32)) {
    unsafe {
        let mut n = rd(head);
        while n != 0 {
            let l = rd(n as usize + link);
            let next = if l != rd(head) { l } else { 0 };
            body(n);
            n = next;
        }
    }
}

/// Shape B: next read after the body, stopping at the live head.
unsafe fn walk_b(head: usize, link: usize, mut body: impl FnMut(u32)) {
    unsafe {
        let mut n = rd(head);
        while n != 0 {
            body(n);
            n = rd(n as usize + link);
            if n == rd(head) {
                break;
            }
        }
    }
}

/// The pooled vehicles (ambulances, meter maids, taxis): update, and free the ones that end.
unsafe fn pooled_update(pool: u32, link: usize, update: usize, free: usize) {
    unsafe {
        walk_a(pool as usize + 0xC, link, |n| {
            if call_regs(update, 0, 0, 0, 0, 0, &[n]) as u8 == 0 {
                call_regs(free, 0, 0, 0, n, pool, &[]);
            }
        });
    }
}

extern "C" fn ambulances_update() {
    unsafe { pooled_update(0x02FA_8D30, 0x14, 0x00A9_5470, 0x00A9_5C30) };
}

extern "C" fn meter_maids_update() {
    unsafe { pooled_update(0x02FA_CF78, 0x14, 0x00AD_CBB0, 0x00AD_D330) };
}

extern "C" fn taxis_update() {
    unsafe { pooled_update(0x0313_7BF8, 0x18, 0x00AE_EE80, 0x00AE_F8B0) };
}

extern "C" fn weapon_effects_update() {
    unsafe {
        walk_a(0x022D_7B58, 0x70, |n| {
            if call_regs(0x00B6_A030, 0, 0, 0, 0, 0, &[n]) as u8 == 0 {
                call_regs(0x00B6_9BE0, n, 0, 0, 0, 0, &[]); // weapon_effect_free
            }
        });
    }
}

extern "C" fn searchlights_update() {
    unsafe { walk_a(0x0215_E0C0, 0xB4, |n| _ = call_regs(0x0095_42F0, n, 0, 0, 0, 0, &[])) };
}

extern "C" fn debris_do_frame() {
    unsafe {
        call(0x0092_B720); // debris_pre_update
        walk_a(0x0252_9604, 0xB4, |n| {
            if rb(n as usize + 0xC0) & 2 != 0 {
                call_regs(vslot(n, 4), 0, n, 0, 0, 0, &[0]);
            }
        });
    }
}

extern "C" fn trigger_objects_reap() {
    unsafe {
        walk_a(0x02F9_B18C, 0xBC, |n| {
            if call_regs(vslot(n, 13), 0, n, 0, 0, 0, &[]) == 0 {
                call_regs(0x00A6_ED70, 0, 0, 0, 0, 0, &[n]); // trigger_object_remove
            }
        });
    }
}

extern "C" fn object_props_post_update() {
    unsafe {
        walk_b(0x0215_D798, 0xB4, |n| {
            if rb(n as usize + 0x120) & 0x20 == 0 {
                call_regs(0x0093_5010, 0, 0, 0, 0, 0, &[n]); // stdcall
            }
        });
    }
}

extern "C" fn detour_hull_objects_update() {
    unsafe { walk_b(0x0214_3ED4, 4, |n| _ = call_regs(0x0091_8B20, 0, 0, 0, 0, 0, &[n])) };
}

extern "C" fn vehicle_interaction_riders_update() {
    unsafe {
        for rec in (0x0314_92B8usize..0x0316_2338).step_by(0xC84) {
            if rd(rec) == 0 {
                continue;
            }
            for i in 0..8u32 {
                let h = rd(rec + 0x34C + i as usize * 0xA8);
                if h == 0 {
                    continue;
                }
                let o = rd(OBJECT_HANDLES + (h as usize & 0xFFF) * 16) as usize;
                if o != 0 && rd(o + 0x44) == h && rd(o + 0x4C) == 1 {
                    call_regs(0x00B0_5230, 0, rec as u32, 0, 0, 0, &[i]);
                }
            }
        }
    }
}

/// A thunk that keeps ECX / EDX around a Rust `extern "C" fn()`.
macro_rules! keep_ecx_edx {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!("push ecx", "push edx", "call {f}", "pop edx", "pop ecx", "ret", f = sym $f)
        }
    };
}
keep_ecx_edx!(ambulances_update_thunk, ambulances_update);
keep_ecx_edx!(meter_maids_update_thunk, meter_maids_update);
keep_ecx_edx!(taxis_update_thunk, taxis_update);
keep_ecx_edx!(weapon_effects_update_thunk, weapon_effects_update);
keep_ecx_edx!(searchlights_update_thunk, searchlights_update);
keep_ecx_edx!(debris_do_frame_thunk, debris_do_frame);
keep_ecx_edx!(trigger_objects_reap_thunk, trigger_objects_reap);
keep_ecx_edx!(object_props_post_update_thunk, object_props_post_update);
keep_ecx_edx!(detour_hull_objects_update_thunk, detour_hull_objects_update);
keep_ecx_edx!(vehicle_interaction_riders_update_thunk, vehicle_interaction_riders_update);

const WALKERS: [(&str, usize, usize); 10] = [
    ("ambulances_update", 0x00A9_5A30, 0x00A9_5A74),
    ("meter_maids_update", 0x00AD_D120, 0x00AD_D164),
    ("taxis_update", 0x00AE_F510, 0x00AE_F554),
    ("weapon_effects_update", 0x00B6_A4A0, 0x00B6_A4E1),
    ("searchlights_update", 0x0095_5470, 0x0095_5499),
    ("debris_do_frame", 0x0092_B3B0, 0x0092_B3F0),
    ("trigger_objects_reap", 0x00A7_0B40, 0x00A7_0B7D),
    ("object_props_post_update", 0x0093_5B80, 0x0093_5BB3),
    ("detour_hull_objects_update", 0x0091_93C0, 0x0091_93EA),
    ("vehicle_interaction_riders_update", 0x00B0_95D0, 0x00B0_963D),
];

fn walker_thunk(i: usize) -> usize {
    let f: extern "C" fn() = match i {
        0 => ambulances_update_thunk,
        1 => meter_maids_update_thunk,
        2 => taxis_update_thunk,
        3 => weapon_effects_update_thunk,
        4 => searchlights_update_thunk,
        5 => debris_do_frame_thunk,
        6 => trigger_objects_reap_thunk,
        7 => object_props_post_update_thunk,
        8 => detour_hull_objects_update_thunk,
        _ => vehicle_interaction_riders_update_thunk,
    };
    f as usize
}

/// Each walker is pinned to its whole original body (WALKER_BODIES).
fn install_walkers() {
    let mut ok = 0;
    for (i, &(name, start, end)) in WALKERS.iter().enumerate() {
        let body = unsafe { std::slice::from_raw_parts(start as *const u8, end - start) };
        let expect = &WALKER_BODIES[WALKER_OFFSETS[i]..WALKER_OFFSETS[i + 1]];
        if body != expect {
            log(&format!("frame: {name} bytes differ, not replaced"));
            continue;
        }
        match unsafe { detour::replace(start, expect, walker_thunk(i)) } {
            Ok(()) => ok += 1,
            Err(e) => log(&format!("frame: replacing {name} failed: {e:?}")),
        }
    }
    log(&format!("frame: replaced {ok}/10 sim_frame list-walkers with Rust"));
}

// --- overlays_update / overlays_render (0x75B220 / 0x75B270), interface_render (0x7F33B0) ----

// 11_frame.md, "game_state_dispatch, overlays and the gameplay UI" and "frontend_render and
// interface_render at instruction level".
const OVERLAYS: usize = 0x00EA_C4E0; // -> overlay pointers, bottom first
const OVERLAY_COUNT: usize = 0x00EA_C4E8; // i32
const OVERLAY_HIDE: usize = 0x0075_B1B0; // ESI = overlay
const INTERFACE_RENDER: usize = 0x007F_33B0;

unsafe fn overlay(i: i32) -> u32 {
    unsafe { rd(rd(OVERLAYS) as usize + i as usize * 4) }
}

/// From the top down: update each (vtable +8); hide the ones that return false; stop below the
/// first one that doesn't let the overlays under it run (+4 == 0).
extern "C" fn overlays_update() {
    unsafe {
        let mut i = rd(OVERLAY_COUNT) as i32 - 1;
        while i >= 0 {
            let o = overlay(i);
            if call_regs(vslot(o, 2), 0, o, 0, 0, 0, &[]) as u8 == 0 {
                call_regs(OVERLAY_HIDE, 0, 0, 0, overlay(i), 0, &[]);
            }
            if rb(overlay(i) as usize + 4) == 0 {
                break;
            }
            i -= 1;
        }
    }
}

/// The render target tint the overlays are drawn under (thread-local render target).
unsafe fn render_target() -> usize {
    let tls: u32;
    unsafe {
        std::arch::asm!("mov {}, fs:[0x2c]", out(reg) tls, options(nostack, readonly));
        let t = rd(rd(tls as usize) as usize) as usize;
        if rb(t + 0x5C) == 0 && rd(t + 0x60) == 0x00EC_26A0 { 0x00EC_2610 } else { rd(t + 0x60) as usize }
    }
}

/// Pass 1 from the lowest visible overlay: the ones not drawn on top (vtable +0x10 false) render
/// (+0xC). Pass 2, from index 0 (a quirk kept from the original), the on-top ones under a white
/// tint; then the tint is restored.
extern "C" fn overlays_render() {
    unsafe {
        let n = rd(OVERLAY_COUNT) as i32;
        if n <= 0 {
            return;
        }
        let mut start = n - 1;
        while start > 0 && rb(overlay(start) as usize + 4) != 0 {
            start -= 1;
        }
        let mut i = start;
        while i < rd(OVERLAY_COUNT) as i32 {
            let o = overlay(i);
            if call_regs(vslot(o, 4), 0, o, 0, 0, 0, &[]) as u8 == 0 {
                let o = overlay(i);
                call_regs(vslot(o, 3), 0, o, 0, 0, 0, &[]);
            }
            i += 1;
        }
        let t = render_target();
        let saved = [rd(t + 0x70), rd(t + 0x74), rd(t + 0x78)];
        let white = [1f32; 3];
        call_regs(GFX_SET_TINT, 0, 0, 0, white.as_ptr() as u32, 0, &[]);
        let mut i = 0;
        while i < rd(OVERLAY_COUNT) as i32 {
            let o = overlay(i);
            if call_regs(vslot(o, 4), 0, o, 0, 0, 0, &[]) as u8 == 1 {
                let o = overlay(i);
                call_regs(vslot(o, 3), 0, o, 0, 0, 0, &[]);
            }
            i += 1;
        }
        call_regs(GFX_SET_TINT, 0, 0, 0, saved.as_ptr() as u32, 0, &[]);
    }
}

/// The screen-state callback, the 2D render state and white colour, then the vint documents.
extern "C" fn interface_render() {
    unsafe {
        let st = rd(0x00E8_CF80);
        if st <= 0x14 {
            let cb = rd(0x00E8_CF94 + st as usize * 16) as usize;
            if cb != 0 {
                call(cb);
            }
        }
        copy_dwords(0x02A1_4B28, 0x00EC_2740, 2);
        wd(0x00E8_C548, 1);
        wd(0x0252_8B14, 0);
        for k in 0..4 {
            wb(0x00EB_15BC + k, 0xFF);
        }
        call_regs(0x00D1_48D0, 0, 0x00EC_2740, 0, 0, 0, &[]);
        let c = |k: usize| rb(0x00EB_15BC + k) as u32;
        call_regs(GFX_SET_COLOR, 0, 0, 0, 0, 0, &[c(0), c(1), c(2), c(3)]);
        call(0x00B8_BD40); // vint_render_documents
    }
}

keep_ecx_edx!(overlays_update_thunk, overlays_update);
keep_ecx_edx!(overlays_render_thunk, overlays_render);
keep_ecx_edx!(interface_render_thunk, interface_render);

pub fn install() {
    match unsafe { detour::replace(INTERFACE_RENDER, &INTERFACE_RENDER_BODY, interface_render_thunk as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced interface_render @ 0x7f33b0 with Rust"),
        Err(e) => log(&format!("frame: replacing interface_render failed: {e:?}")),
    }
    match unsafe { detour::replace(OVERLAYS_RENDER, &OVERLAYS_RENDER_BODY, overlays_render_thunk as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced overlays_render @ 0x75b270 with Rust"),
        Err(e) => log(&format!("frame: replacing overlays_render failed: {e:?}")),
    }
    match unsafe { detour::replace(OVERLAYS_UPDATE, &OVERLAYS_UPDATE_BODY, overlays_update_thunk as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced overlays_update @ 0x75b220 with Rust"),
        Err(e) => log(&format!("frame: replacing overlays_update failed: {e:?}")),
    }
    install_walkers();
    match unsafe { detour::replace(HUD_RENDER, &HUD_RENDER_BODY, hud_render as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced hud_render @ 0x75a230 with Rust"),
        Err(e) => log(&format!("frame: replacing hud_render failed: {e:?}")),
    }
    match unsafe { detour::replace(GAME_STATE_DISPATCH, &GAME_STATE_DISPATCH_BODY, game_state_dispatch_thunk as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced game_state_dispatch @ 0x697490 with Rust"),
        Err(e) => log(&format!("frame: replacing game_state_dispatch failed: {e:?}")),
    }
    match unsafe { detour::replace(GAMEPLAY_STREAMING_FRAME_FN, &GAMEPLAY_STREAMING_FRAME_BODY, gameplay_streaming_frame as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced gameplay_streaming_frame @ 0x68e410 with Rust"),
        Err(e) => log(&format!("frame: replacing gameplay_streaming_frame failed: {e:?}")),
    }
    match unsafe { detour::replace(FRAME_PRESENT_FN, &FRAME_PRESENT_BODY, frame_present as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced frame_present @ 0x5217e0 with Rust"),
        Err(e) => log(&format!("frame: replacing frame_present failed: {e:?}")),
    }
    match unsafe { detour::replace(INTERFACE_DO_FRAME, &INTERFACE_DO_FRAME_BODY, interface_do_frame as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced interface_do_frame @ 0x7f3500 with Rust"),
        Err(e) => log(&format!("frame: replacing interface_do_frame failed: {e:?}")),
    }
    match unsafe {
        detour::replace(VINT_UPDATE_DOCUMENTS, &VINT_UPDATE_DOCUMENTS_BODY, vint_update_documents_thunk as extern "C" fn() as usize)
    } {
        Ok(()) => log("frame: replaced vint_update_documents @ 0xb8bc10 with Rust"),
        Err(e) => log(&format!("frame: replacing vint_update_documents failed: {e:?}")),
    }
    match unsafe { detour::replace(GAMEPLAY_FRAME, &GAMEPLAY_FRAME_BODY, gameplay_do_frame as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced gameplay_do_frame @ 0x68e1b0 with Rust"),
        Err(e) => log(&format!("frame: replacing gameplay_do_frame failed: {e:?}")),
    }
    match unsafe { detour::replace(LOADING_FRAME, &LOADING_FRAME_BODY, loading_state_frame as extern "C" fn() as usize) } {
        Ok(()) => log("frame: replaced loading_state_frame @ 0x73b7f0 with Rust"),
        Err(e) => log(&format!("frame: replacing loading_state_frame failed: {e:?}")),
    }
    match unsafe { detour::install(SIM_FRAME, &SIM_FRAME_PROLOGUE, sim_frame_hook as extern "C" fn() as usize) } {
        Ok(t) => {
            SIM_ORIGINAL.store(t, Ordering::Relaxed);
            log(&format!("frame: sim_frame @ 0x68ca70 hooked ({})", if RUST_SIM { "Rust body" } else { "original" }));
        }
        Err(e) => log(&format!("frame: hooking sim_frame failed: {e:?}")),
    }
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

const LOADING_FRAME_BODY: [u8; 828] = [
    0x53, 0x56, 0x57, 0xE8, 0x98, 0x5F, 0xD9, 0xFF, 0xA1, 0x7C, 0x6B, 0xE9, 0x00, 0x83, 0xF8, 0xFF,
    0x7E, 0x19, 0x83, 0x3C, 0x85, 0x60, 0xE7, 0x44, 0x01, 0x0A, 0x75, 0x0F, 0xE8, 0xEF, 0x7C, 0x0B,
    0x00, 0xE8, 0x9A, 0x7B, 0x0B, 0x00, 0xE8, 0xA5, 0x35, 0x00, 0x00, 0x80, 0x3D, 0x83, 0x7E, 0x52,
    0x02, 0x00, 0xBB, 0x01, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xA1, 0x01, 0x00, 0x00, 0xA1, 0x88, 0xEB,
    0xE8, 0x00, 0x3B, 0xC3, 0x0F, 0x85, 0x94, 0x01, 0x00, 0x00, 0x8A, 0x0D, 0x48, 0x8B, 0x52, 0x02,
    0x3A, 0xCB, 0x0F, 0x85, 0x86, 0x01, 0x00, 0x00, 0x8A, 0x15, 0x49, 0x8B, 0x52, 0x02, 0x3A, 0xD3,
    0x0F, 0x85, 0x78, 0x01, 0x00, 0x00, 0x8B, 0x15, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xD2, 0x0F, 0x84,
    0x6A, 0x01, 0x00, 0x00, 0x52, 0xE8, 0x86, 0x44, 0x0D, 0x00, 0x84, 0xC0, 0x0F, 0x85, 0x5C, 0x01,
    0x00, 0x00, 0x38, 0x05, 0xA0, 0x7B, 0x52, 0x02, 0x0F, 0x85, 0x50, 0x01, 0x00, 0x00, 0x84, 0x1D,
    0x1C, 0x52, 0x4A, 0x03, 0x75, 0x10, 0x09, 0x1D, 0x1C, 0x52, 0x4A, 0x03, 0xC7, 0x05, 0x18, 0x52,
    0x4A, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0x80, 0x3D, 0x82, 0x7E, 0x52, 0x02, 0x00, 0x75, 0x3C, 0x8B,
    0xB2, 0x08, 0x04, 0x00, 0x00, 0xB9, 0x0B, 0x00, 0x00, 0x00, 0x88, 0x1D, 0x82, 0x7E, 0x52, 0x02,
    0xE8, 0xEB, 0xD2, 0x0B, 0x00, 0xA1, 0xB4, 0x43, 0xE8, 0x00, 0x05, 0x30, 0x75, 0x00, 0x00, 0x3D,
    0x00, 0xD2, 0x49, 0x6B, 0xA3, 0x18, 0x52, 0x4A, 0x03, 0x7E, 0x0A, 0x2D, 0x00, 0xD2, 0x49, 0x6B,
    0xA3, 0x18, 0x52, 0x4A, 0x03, 0x88, 0x1D, 0x8C, 0x7E, 0x52, 0x02, 0xE8, 0xF0, 0xC2, 0x29, 0x00,
    0x85, 0xC0, 0x75, 0x12, 0xB8, 0x18, 0x52, 0x4A, 0x03, 0xE8, 0xB2, 0x1D, 0x4A, 0x00, 0x84, 0xC0,
    0x0F, 0x84, 0x32, 0x02, 0x00, 0x00, 0xB8, 0x18, 0x52, 0x4A, 0x03, 0xE8, 0xA0, 0x1D, 0x4A, 0x00,
    0x84, 0xC0, 0x75, 0x38, 0xE8, 0xC7, 0xC2, 0x29, 0x00, 0x85, 0xC0, 0x74, 0x2F, 0xE8, 0xBE, 0xC2,
    0x29, 0x00, 0x80, 0xB8, 0x9D, 0x12, 0x00, 0x00, 0x00, 0x75, 0x21, 0xE8, 0x90, 0xB2, 0x0F, 0x00,
    0xE8, 0x8B, 0xEE, 0x3C, 0x00, 0xE8, 0x86, 0x5B, 0x1D, 0x00, 0x6A, 0x00, 0xE8, 0x3F, 0xBC, 0x20,
    0x00, 0x83, 0xC4, 0x04, 0x5F, 0x5E, 0x5B, 0xE9, 0x94, 0x1D, 0x34, 0x00, 0xE8, 0x8F, 0xC2, 0x29,
    0x00, 0x85, 0xC0, 0x74, 0x3F, 0xE8, 0x86, 0xC2, 0x29, 0x00, 0x8B, 0xB8, 0xB0, 0x13, 0x00, 0x00,
    0xE8, 0x9B, 0xE0, 0x2A, 0x00, 0x84, 0xC0, 0x75, 0x2B, 0xE8, 0x72, 0x1D, 0x34, 0x00, 0xE8, 0x8D,
    0x81, 0x2A, 0x00, 0xE8, 0x98, 0x02, 0x5C, 0x00, 0xE8, 0xA3, 0x81, 0x2A, 0x00, 0xE8, 0x4E, 0x02,
    0x5C, 0x00, 0xE8, 0x39, 0xB2, 0x0F, 0x00, 0xE8, 0x34, 0xEE, 0x3C, 0x00, 0x5F, 0x5E, 0x5B, 0xE9,
    0x2C, 0x5B, 0x1D, 0x00, 0x83, 0x3D, 0x8C, 0xB8, 0xF9, 0x02, 0x00, 0x74, 0x34, 0xB8, 0x18, 0x52,
    0x4A, 0x03, 0xE8, 0x09, 0x1D, 0x4A, 0x00, 0x84, 0xC0, 0x75, 0x26, 0xE8, 0x10, 0xB2, 0x0F, 0x00,
    0x6A, 0x00, 0xE8, 0xC9, 0xBB, 0x20, 0x00, 0x83, 0xC4, 0x04, 0xE8, 0x21, 0x1D, 0x34, 0x00, 0xE8,
    0xEC, 0xC1, 0x0D, 0x00, 0xE8, 0xF7, 0xED, 0x3C, 0x00, 0x5F, 0x5E, 0x5B, 0xE9, 0xEF, 0x5A, 0x1D,
    0x00, 0xC6, 0x05, 0x82, 0x7E, 0x52, 0x02, 0x00, 0x88, 0x1D, 0x83, 0x7E, 0x52, 0x02, 0x80, 0x3D,
    0x81, 0x7E, 0x52, 0x02, 0x00, 0x75, 0x60, 0xA1, 0x88, 0xEB, 0xE8, 0x00, 0x85, 0xC0, 0x0F, 0x84,
    0xD1, 0x00, 0x00, 0x00, 0xD9, 0xEE, 0xD8, 0x1D, 0xA0, 0x6D, 0x52, 0x02, 0xDF, 0xE0, 0xF6, 0xC4,
    0x44, 0x0F, 0x8A, 0xBE, 0x00, 0x00, 0x00, 0x83, 0x3D, 0xC4, 0xFE, 0x12, 0x02, 0x06, 0xBE, 0xFA,
    0x00, 0x00, 0x00, 0x75, 0x02, 0x33, 0xF6, 0x68, 0xFF, 0x00, 0x00, 0x00, 0x6A, 0x00, 0x6A, 0x00,
    0x6A, 0x00, 0xE8, 0x29, 0x8E, 0x5D, 0x00, 0xD9, 0xE8, 0x83, 0xC4, 0x0C, 0xD9, 0x1C, 0x24, 0x6A,
    0x00, 0x56, 0xE8, 0x69, 0x56, 0xD9, 0xFF, 0x83, 0xC4, 0x0C, 0x80, 0x3D, 0x81, 0x7E, 0x52, 0x02,
    0x00, 0x0F, 0x84, 0xF1, 0x00, 0x00, 0x00, 0x8B, 0x15, 0x88, 0xEB, 0xE8, 0x00, 0x85, 0xD2, 0x0F,
    0x84, 0xE3, 0x00, 0x00, 0x00, 0xA1, 0x88, 0xEB, 0xE8, 0x00, 0x83, 0xF8, 0xFF, 0x0F, 0x84, 0x9B,
    0x00, 0x00, 0x00, 0x8A, 0x0D, 0x80, 0x7E, 0x52, 0x02, 0x84, 0xC9, 0x0F, 0x85, 0x8D, 0x00, 0x00,
    0x00, 0x8B, 0x15, 0x88, 0xEB, 0xE8, 0x00, 0x3B, 0xD3, 0x0F, 0x85, 0xB9, 0x00, 0x00, 0x00, 0x83,
    0x3D, 0x5C, 0xE7, 0x44, 0x01, 0x00, 0x0F, 0x85, 0xAC, 0x00, 0x00, 0x00, 0x38, 0x0D, 0xCA, 0xBB,
    0x40, 0x01, 0x0F, 0x85, 0xA0, 0x00, 0x00, 0x00, 0x38, 0x0D, 0xC9, 0xBB, 0x40, 0x01, 0x0F, 0x85,
    0x94, 0x00, 0x00, 0x00, 0x80, 0x25, 0x55, 0xE7, 0x44, 0x01, 0xFC, 0xC7, 0x05, 0x5C, 0xE7, 0x44,
    0x01, 0x05, 0x00, 0x00, 0x00, 0x88, 0x0D, 0xC9, 0xBB, 0x40, 0x01, 0x88, 0x0D, 0xCA, 0xBB, 0x40,
    0x01, 0x5F, 0x5E, 0x5B, 0xC3, 0x80, 0x3D, 0x81, 0x7E, 0x52, 0x02, 0x00, 0x0F, 0x85, 0x75, 0xFF,
    0xFF, 0xFF, 0x8B, 0x0D, 0x88, 0xEB, 0xE8, 0x00, 0x85, 0xC9, 0x0F, 0x84, 0x5A, 0xFF, 0xFF, 0xFF,
    0xD9, 0xE8, 0xD8, 0x1D, 0xA0, 0x6D, 0x52, 0x02, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x0F, 0x8A, 0x47,
    0xFF, 0xFF, 0xFF, 0x88, 0x1D, 0x81, 0x7E, 0x52, 0x02, 0xE9, 0x49, 0xFF, 0xFF, 0xFF, 0x83, 0x3D,
    0x5C, 0xE7, 0x44, 0x01, 0x00, 0x75, 0x31, 0x80, 0x3D, 0xCA, 0xBB, 0x40, 0x01, 0x00, 0x75, 0x28,
    0x80, 0x3D, 0xC9, 0xBB, 0x40, 0x01, 0x00, 0x75, 0x1F, 0x80, 0x25, 0x52, 0xE7, 0x44, 0x01, 0xFC,
    0xC7, 0x05, 0x5C, 0xE7, 0x44, 0x01, 0x02, 0x00, 0x00, 0x00, 0xC6, 0x05, 0xC9, 0xBB, 0x40, 0x01,
    0x00, 0xC6, 0x05, 0xCA, 0xBB, 0x40, 0x01, 0x00, 0x5F, 0x5E, 0x5B, 0xC3,
];

const GAMEPLAY_FRAME_BODY: [u8; 602] = [
    0x83, 0xEC, 0x0C, 0x53, 0x55, 0x56, 0x57, 0x8D, 0x44, 0x24, 0x18, 0x50, 0xB9, 0x6C, 0xFB, 0xDF,
    0x00, 0xE8, 0x4A, 0xDC, 0x57, 0x00, 0xC6, 0x05, 0xA2, 0x7B, 0x52, 0x02, 0x01, 0xE8, 0xBE, 0x6C,
    0x16, 0x00, 0x80, 0x3D, 0x14, 0x7D, 0x52, 0x02, 0x00, 0x74, 0x07, 0xA0, 0xF7, 0x7C, 0x52, 0x02,
    0xEB, 0x0E, 0x80, 0x3D, 0x15, 0x7D, 0x52, 0x02, 0x00, 0x75, 0x05, 0xE8, 0x20, 0x76, 0x02, 0x00,
    0x8B, 0x2D, 0x3C, 0x04, 0x17, 0x02, 0x85, 0xED, 0x0F, 0x84, 0x65, 0x01, 0x00, 0x00, 0x8B, 0xFF,
    0x83, 0xBD, 0x2C, 0x12, 0x00, 0x00, 0x00, 0x0F, 0x8C, 0x40, 0x01, 0x00, 0x00, 0x8B, 0x85, 0x2C,
    0x12, 0x00, 0x00, 0x85, 0xC0, 0x0F, 0x8C, 0x32, 0x01, 0x00, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8,
    0x00, 0x3B, 0xC1, 0x7E, 0x0F, 0x2B, 0xC1, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8C, 0x1B, 0x01,
    0x00, 0x00, 0xEB, 0x0E, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35, 0x0F, 0x8D, 0x0B, 0x01,
    0x00, 0x00, 0x8B, 0x85, 0xC0, 0x00, 0x00, 0x00, 0x32, 0xDB, 0x3B, 0x2D, 0xD4, 0x03, 0x17, 0x02,
    0x0F, 0x94, 0x44, 0x24, 0x13, 0xA8, 0x01, 0x0F, 0x85, 0xAB, 0x00, 0x00, 0x00, 0xBE, 0x01, 0x00,
    0x00, 0x00, 0x0B, 0xC6, 0x8D, 0x8D, 0x34, 0x12, 0x00, 0x00, 0x8D, 0x54, 0x24, 0x14, 0x89, 0x85,
    0xC0, 0x00, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x14, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x8F, 0x91, 0xD9,
    0xFF, 0x8B, 0xF8, 0x85, 0xFF, 0x0F, 0x84, 0x7D, 0x00, 0x00, 0x00, 0x8B, 0xD5, 0xE8, 0xCE, 0x39,
    0x3D, 0x00, 0x85, 0xC0, 0x75, 0x15, 0x8B, 0x47, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x83, 0x40, 0x08, 0x10, 0x01, 0x77, 0x1C, 0xEB, 0x15, 0x8B, 0x77, 0x04, 0x50, 0xE8,
    0x3C, 0xE8, 0x64, 0x00, 0x83, 0xC4, 0x04, 0x83, 0x47, 0x1C, 0x01, 0xBE, 0x01, 0x00, 0x00, 0x00,
    0x8B, 0x85, 0x0C, 0x0C, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x2B, 0x8B, 0xD0, 0x81, 0xE2, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x92, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xD2, 0x74, 0x16, 0x39,
    0x42, 0x44, 0x75, 0x11, 0x39, 0x72, 0x4C, 0x75, 0x0C, 0xE8, 0x72, 0x39, 0x3D, 0x00, 0xE8, 0xBD,
    0x1E, 0x65, 0x00, 0xEB, 0x13, 0x8B, 0x47, 0x04, 0x8B, 0x48, 0x08, 0xC7, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x83, 0x40, 0x08, 0x10, 0x01, 0x77, 0x1C, 0x0F, 0xB7, 0x95, 0x90, 0x12, 0x00, 0x00, 0x52,
    0xE8, 0x9B, 0x1B, 0x65, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x75, 0x31, 0x38, 0x44, 0x24, 0x13,
    0x74, 0x0B, 0xF6, 0x85, 0xA4, 0x0F, 0x00, 0x00, 0x10, 0x74, 0x02, 0xB3, 0x01, 0x84, 0xDB, 0x66,
    0xC7, 0x85, 0x90, 0x12, 0x00, 0x00, 0xFF, 0xFF, 0x74, 0x13, 0x6A, 0x00, 0xE8, 0x3F, 0x48, 0xE9,
    0xFF, 0x83, 0xC4, 0x04, 0x55, 0xE8, 0x26, 0xF2, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xAD, 0x94,
    0x12, 0x00, 0x00, 0x3B, 0x2D, 0x3C, 0x04, 0x17, 0x02, 0x74, 0x08, 0x85, 0xED, 0x0F, 0x85, 0x9D,
    0xFE, 0xFF, 0xFF, 0x80, 0x3D, 0x0F, 0x89, 0xA1, 0x02, 0x00, 0x74, 0x19, 0xE8, 0x7F, 0x0A, 0x19,
    0x00, 0x68, 0x44, 0x01, 0x00, 0x00, 0x6A, 0x00, 0x68, 0xD0, 0x87, 0xA1, 0x02, 0xE8, 0x3E, 0x28,
    0x61, 0x00, 0x83, 0xC4, 0x0C, 0xE8, 0xE6, 0xE6, 0xFF, 0xFF, 0x8D, 0x44, 0x24, 0x18, 0x50, 0xE8,
    0xFC, 0xE5, 0x57, 0x00, 0x8D, 0x4C, 0x24, 0x18, 0x51, 0xB9, 0x80, 0xFB, 0xDF, 0x00, 0xE8, 0x6D,
    0xDA, 0x57, 0x00, 0x80, 0x3D, 0xC0, 0xC0, 0x79, 0x02, 0x00, 0x74, 0x05, 0xE8, 0xBF, 0xE3, 0xFF,
    0xFF, 0xE8, 0x3A, 0xEF, 0xFF, 0xFF, 0x80, 0x3D, 0x6C, 0x97, 0x52, 0x02, 0x00, 0x74, 0x32, 0x68,
    0xFF, 0x00, 0x00, 0x00, 0x68, 0xFF, 0x00, 0x00, 0x00, 0x6A, 0x00, 0x6A, 0x00, 0xE8, 0x6E, 0x64,
    0x68, 0x00, 0xD9, 0x05, 0x8C, 0x1C, 0xDD, 0x00, 0x83, 0xC4, 0x10, 0x68, 0x48, 0x27, 0xEC, 0x00,
    0x51, 0xB8, 0xD0, 0x30, 0xCE, 0x02, 0xD9, 0x1C, 0x24, 0xE8, 0x72, 0x56, 0x68, 0x00, 0x83, 0xC4,
    0x08, 0x8D, 0x54, 0x24, 0x18, 0x52, 0xC6, 0x05, 0x3D, 0xD7, 0xE9, 0x00, 0x01, 0xE8, 0x8E, 0xE5,
    0x57, 0x00, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
];

const VINT_UPDATE_DOCUMENTS_BODY: [u8; 304] = [
    0x80, 0x3D, 0x7D, 0xA1, 0x52, 0x02, 0x00, 0x56, 0x57, 0x74, 0x05, 0xE8, 0xD0, 0xEA, 0xFF, 0xFF,
    0xD9, 0xEE, 0xC7, 0x05, 0xB0, 0xA1, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x0C,
    0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x05, 0x7A, 0x10, 0xDD, 0xD8, 0xD9, 0x05, 0x80,
    0x43, 0xE8, 0x00, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x05, 0x8C, 0x1C, 0xDD,
    0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x54, 0xDD, 0xD8, 0xD9, 0x54, 0x24, 0x0C,
    0xD8, 0x1D, 0x84, 0x59, 0xDD, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x0F, 0x85, 0x7D, 0x00, 0x00,
    0x00, 0x8B, 0x0D, 0xAC, 0xA1, 0x52, 0x02, 0x85, 0xC9, 0x8B, 0xF1, 0x74, 0x77, 0x8D, 0x49, 0x00,
    0x80, 0xBE, 0x60, 0x04, 0x00, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x0C, 0x75, 0x37, 0xD9, 0xEE, 0xD8,
    0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x75, 0x1E, 0xDD, 0xD8, 0xD9, 0x86, 0x5C, 0x04, 0x00, 0x00,
    0xD8, 0x05, 0x80, 0x43, 0xE8, 0x00, 0xD9, 0x9E, 0x5C, 0x04, 0x00, 0x00, 0xEB, 0x18, 0xDD, 0xD9,
    0xD9, 0x5C, 0x24, 0x0C, 0xEB, 0xBB, 0xD8, 0x86, 0x5C, 0x04, 0x00, 0x00, 0xD9, 0x9E, 0x5C, 0x04,
    0x00, 0x00, 0xEB, 0x02, 0xDD, 0xD8, 0x8B, 0x86, 0x54, 0x04, 0x00, 0x00, 0xE8, 0x6F, 0xEF, 0xFF,
    0xFF, 0x8B, 0x86, 0x58, 0x04, 0x00, 0x00, 0xE8, 0x64, 0xEF, 0xFF, 0xFF, 0x8B, 0x36, 0x8B, 0x0D,
    0xAC, 0xA1, 0x52, 0x02, 0x3B, 0xF1, 0x74, 0x0C, 0x85, 0xF6, 0x75, 0x94, 0xEB, 0x06, 0x8B, 0x0D,
    0xAC, 0xA1, 0x52, 0x02, 0x8B, 0xF1, 0x85, 0xF6, 0x74, 0x43, 0x8B, 0x06, 0x8B, 0xF8, 0x2B, 0xF9,
    0xF7, 0xDF, 0x1B, 0xFF, 0x23, 0xF8, 0x80, 0xBE, 0x72, 0x05, 0x00, 0x00, 0x01, 0x75, 0x2A, 0x56,
    0xE8, 0xEB, 0x56, 0x00, 0x00, 0xA1, 0xEC, 0xA1, 0x52, 0x02, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74,
    0x06, 0x56, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0xE8, 0xC4, 0xEC, 0xFF, 0xFF, 0x8B, 0xC6, 0xE8, 0x9D,
    0xEE, 0xFF, 0xFF, 0x8B, 0x0D, 0xAC, 0xA1, 0x52, 0x02, 0x8B, 0xF7, 0xEB, 0xB9, 0x5F, 0x5E, 0xC3,
];

const INTERFACE_DO_FRAME_BODY: [u8; 111] = [
    0x80, 0x3D, 0xFF, 0x8A, 0x52, 0x02, 0x00, 0x56, 0x74, 0x63, 0x8B, 0x35, 0xD4, 0x03, 0x17, 0x02,
    0x85, 0xF6, 0x74, 0x2E, 0x56, 0xE8, 0x16, 0x5B, 0x1C, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75,
    0x0D, 0x56, 0xE8, 0x49, 0x5B, 0x1C, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x74, 0x02, 0xB0, 0x01,
    0x3A, 0x05, 0x34, 0xAF, 0x47, 0x03, 0x74, 0x0A, 0xA2, 0x34, 0xAF, 0x47, 0x03, 0xE8, 0xFE, 0xEA,
    0xFF, 0xFF, 0xE8, 0x29, 0x8C, 0xFF, 0xFF, 0xE8, 0x04, 0xFF, 0xFF, 0xFF, 0xE8, 0x3F, 0x1D, 0x3A,
    0x00, 0xE8, 0x0A, 0x10, 0x3A, 0x00, 0xE8, 0x15, 0xB5, 0x3A, 0x00, 0xD9, 0x05, 0x98, 0x79, 0xE8,
    0x00, 0x51, 0xD9, 0x1C, 0x24, 0xE8, 0xA6, 0x86, 0x39, 0x00, 0x83, 0xC4, 0x04, 0x5E, 0xC3,
];

const FRAME_PRESENT_BODY: [u8; 143] = [
    0x51, 0x80, 0x3D, 0x3F, 0x4D, 0xE9, 0x00, 0x00, 0x74, 0x2D, 0xD9, 0xE8, 0x6A, 0x00, 0x51, 0xD9,
    0x1C, 0x24, 0x68, 0x00, 0x00, 0x00, 0xFF, 0x6A, 0x07, 0xE8, 0x32, 0x73, 0x7F, 0x00, 0x83, 0xC4,
    0x10, 0xE8, 0x4A, 0x31, 0x7F, 0x00, 0xB9, 0x48, 0x27, 0xEC, 0x00, 0xE8, 0xC0, 0x30, 0x7F, 0x00,
    0xC6, 0x05, 0x3F, 0x4D, 0xE9, 0x00, 0x00, 0xE8, 0xF4, 0x38, 0x7F, 0x00, 0x80, 0x3D, 0x92, 0x73,
    0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x91, 0x73, 0x52, 0x02, 0x00, 0xEB, 0x0C, 0xA1, 0x74,
    0x89, 0xE9, 0x00, 0x80, 0xB8, 0x70, 0x89, 0xE9, 0x00, 0x00, 0x74, 0x19, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0x39, 0x05, 0x78, 0x89, 0xE9, 0x00, 0x74, 0x1F, 0xA3, 0x78, 0x89, 0xE9, 0x00, 0xE8, 0x2D,
    0x7C, 0x17, 0x00, 0x59, 0xC3, 0x83, 0x3D, 0x78, 0x89, 0xE9, 0x00, 0x00, 0x74, 0x0A, 0xC7, 0x05,
    0x78, 0x89, 0xE9, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x13, 0x7C, 0x17, 0x00, 0x59, 0xC3,
];

const GAMEPLAY_STREAMING_FRAME_BODY: [u8; 325] = [
    0x83, 0xEC, 0x34, 0x56, 0x57, 0xE8, 0x66, 0xD4, 0x3E, 0x00, 0xE8, 0x71, 0x43, 0x69, 0x00, 0xD9,
    0x40, 0x0C, 0xD9, 0x5C, 0x24, 0x18, 0x83, 0xC0, 0x0C, 0xD9, 0x40, 0x04, 0xD9, 0x5C, 0x24, 0x1C,
    0xD9, 0x40, 0x08, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x40, 0x0C, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x40,
    0x10, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x40, 0x14, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x40, 0x18, 0xD9,
    0x5C, 0x24, 0x30, 0xD9, 0x40, 0x1C, 0xD9, 0x5C, 0x24, 0x34, 0xD9, 0x40, 0x20, 0xD9, 0x5C, 0x24,
    0x38, 0xE8, 0x2A, 0x43, 0x69, 0x00, 0xD9, 0x00, 0x8D, 0x4C, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x40, 0x04, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x40, 0x08, 0x8D, 0x44, 0x24, 0x18, 0x50, 0xD9,
    0x5C, 0x24, 0x18, 0x51, 0xE8, 0x17, 0x55, 0xE2, 0xFF, 0x33, 0xF6, 0x83, 0xC4, 0x08, 0x39, 0x35,
    0xD4, 0xFB, 0xF6, 0x00, 0x7E, 0x47, 0xEB, 0x08, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x90,
    0x8B, 0x3C, 0xB5, 0x98, 0x0B, 0xF7, 0x00, 0x8B, 0x47, 0x18, 0x83, 0xF8, 0x01, 0x75, 0x0B, 0x57,
    0xE8, 0xBB, 0x17, 0xE2, 0xFF, 0x83, 0xC4, 0x04, 0xEB, 0x18, 0x83, 0xF8, 0x02, 0x75, 0x13, 0x8B,
    0xC7, 0xC7, 0x47, 0x18, 0x03, 0x00, 0x00, 0x00, 0xE8, 0x73, 0xC3, 0xE1, 0xFF, 0xE8, 0x2E, 0x3C,
    0xED, 0xFF, 0x83, 0xC6, 0x01, 0x3B, 0x35, 0xD4, 0xFB, 0xF6, 0x00, 0x7C, 0xC3, 0xE8, 0x4E, 0x08,
    0x00, 0x00, 0xE8, 0x69, 0xE4, 0xFF, 0xFF, 0xE8, 0x74, 0x82, 0x14, 0x00, 0xE8, 0x0F, 0x9F, 0x35,
    0x00, 0xE8, 0xAA, 0xE9, 0x35, 0x00, 0xE8, 0xC5, 0x10, 0x36, 0x00, 0x8B, 0x0D, 0x8C, 0x06, 0x17,
    0x02, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x0F, 0xC6, 0x40, 0x15, 0x00, 0x8B, 0x40, 0x1C, 0x3B, 0xC1,
    0x74, 0x04, 0x85, 0xC0, 0x75, 0xF1, 0xE8, 0x95, 0x7A, 0x43, 0x00, 0xE8, 0x40, 0x87, 0x03, 0x00,
    0xE8, 0xAB, 0x94, 0x05, 0x00, 0x83, 0x3D, 0x20, 0x9E, 0x52, 0x02, 0x00, 0x74, 0x18, 0x33, 0xC0,
    0x80, 0xB8, 0x04, 0xDA, 0x15, 0x02, 0x00, 0x74, 0x0D, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x06, 0x72,
    0xEF, 0xE8, 0xCA, 0xDC, 0x4E, 0x00, 0xE8, 0x45, 0x17, 0x13, 0x00, 0x5F, 0x5E, 0x83, 0xC4, 0x34,
    0xE9, 0x1B, 0x63, 0x3C, 0x00,
];

const GAME_STATE_DISPATCH_BODY: [u8; 90] = [
    0x51, 0x83, 0xFE, 0x04, 0x74, 0x05, 0xE8, 0x85, 0x3D, 0x0C, 0x00, 0x8D, 0x46, 0xFF, 0x83, 0xF8,
    0x0A, 0x77, 0x36, 0xFF, 0x24, 0x85, 0xEC, 0x74, 0x69, 0x00, 0xE8, 0x01, 0x68, 0x0A, 0x00, 0xEB,
    0x28, 0xE8, 0xFA, 0x6C, 0xFF, 0xFF, 0xEB, 0x21, 0xE8, 0xF3, 0x95, 0x19, 0x00, 0xEB, 0x1A, 0xE8,
    0x7C, 0xD8, 0x19, 0x00, 0xEB, 0x13, 0xE8, 0xA5, 0x80, 0x19, 0x00, 0xEB, 0x0C, 0xE8, 0x1E, 0x43,
    0x0A, 0x00, 0xEB, 0x05, 0xE8, 0x07, 0x1B, 0xFF, 0xFF, 0x83, 0xFE, 0x02, 0x74, 0x0A, 0x83, 0xFE,
    0x06, 0x74, 0x05, 0xE8, 0x88, 0x3D, 0x0C, 0x00, 0x59, 0xC3,
];

const HUD_RENDER_BODY: [u8; 82] = [
    0x56, 0x33, 0xF6, 0x39, 0x35, 0xF4, 0x7F, 0x52, 0x02, 0x7E, 0x2A, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x04, 0xB5, 0xE4, 0x68, 0xF7, 0x01, 0x8D, 0x04, 0x80, 0x8D, 0x04, 0x85, 0x30, 0xE7, 0xE8,
    0x00, 0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xD0, 0x83, 0xC6, 0x01, 0x3B, 0x35, 0xF4,
    0x7F, 0x52, 0x02, 0x7C, 0xDB, 0xE8, 0x16, 0xB9, 0xFF, 0xFF, 0xE8, 0x71, 0xC9, 0xF6, 0xFF, 0x68,
    0x08, 0xC2, 0xEA, 0x00, 0xE8, 0x57, 0xB0, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x5E, 0xE9, 0x2E, 0x91,
    0x09, 0x00,
];

const WALKER_BODIES: [u8; 637] = [
    0x53, 0x56, 0x8B, 0x35, 0x3C, 0x8D, 0xFA, 0x02, 0x57, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x85, 0xF6, 0x74, 0x2C, 0x8B, 0x46, 0x14, 0x8B, 0xD8, 0x2B, 0x1D, 0x3C, 0x8D, 0xFA, 0x02, 0x56,
    0xF7, 0xDB, 0x1B, 0xDB, 0x23, 0xD8, 0xE8, 0x15, 0xFA, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0,
    0x75, 0x0A, 0xBF, 0x30, 0x8D, 0xFA, 0x02, 0xE8, 0xC4, 0x01, 0x00, 0x00, 0x8B, 0xF3, 0xEB, 0xD0,
    0x5F, 0x5E, 0x5B, 0xC3, 0x53, 0x56, 0x8B, 0x35, 0x84, 0xCF, 0xFA, 0x02, 0x57, 0x8D, 0xA4, 0x24,
    0x00, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x74, 0x2C, 0x8B, 0x46, 0x14, 0x8B, 0xD8, 0x2B, 0x1D, 0x84,
    0xCF, 0xFA, 0x02, 0x56, 0xF7, 0xDB, 0x1B, 0xDB, 0x23, 0xD8, 0xE8, 0x65, 0xFA, 0xFF, 0xFF, 0x83,
    0xC4, 0x04, 0x84, 0xC0, 0x75, 0x0A, 0xBF, 0x78, 0xCF, 0xFA, 0x02, 0xE8, 0xD4, 0x01, 0x00, 0x00,
    0x8B, 0xF3, 0xEB, 0xD0, 0x5F, 0x5E, 0x5B, 0xC3, 0x53, 0x56, 0x8B, 0x35, 0x04, 0x7C, 0x13, 0x03,
    0x57, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x74, 0x2C, 0x8B, 0x46, 0x18, 0x8B,
    0xD8, 0x2B, 0x1D, 0x04, 0x7C, 0x13, 0x03, 0x56, 0xF7, 0xDB, 0x1B, 0xDB, 0x23, 0xD8, 0xE8, 0x45,
    0xF9, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x0A, 0xBF, 0xF8, 0x7B, 0x13, 0x03, 0xE8,
    0x64, 0x03, 0x00, 0x00, 0x8B, 0xF3, 0xEB, 0xD0, 0x5F, 0x5E, 0x5B, 0xC3, 0x51, 0x56, 0x57, 0x8B,
    0x3D, 0x58, 0x7B, 0x2D, 0x02, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x85, 0xFF, 0x74, 0x29,
    0x8B, 0x47, 0x70, 0x8B, 0xF0, 0x2B, 0x35, 0x58, 0x7B, 0x2D, 0x02, 0x57, 0xF7, 0xDE, 0x1B, 0xF6,
    0x23, 0xF0, 0xE8, 0x65, 0xFB, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x07, 0x8B, 0xC7,
    0xE8, 0x07, 0xF7, 0xFF, 0xFF, 0x8B, 0xFE, 0xEB, 0xD3, 0x5F, 0x5E, 0x59, 0xC3, 0xA1, 0xC0, 0xE0,
    0x15, 0x02, 0x56, 0x85, 0xC0, 0x74, 0x1D, 0x8B, 0x88, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0xF1, 0x2B,
    0x35, 0xC0, 0xE0, 0x15, 0x02, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF1, 0xE8, 0x5D, 0xEE, 0xFF, 0xFF,
    0x8B, 0xC6, 0xEB, 0xDF, 0x5E, 0xC3, 0x56, 0xE8, 0x6A, 0x03, 0x00, 0x00, 0x8B, 0x0D, 0x04, 0x96,
    0x52, 0x02, 0x8D, 0x64, 0x24, 0x00, 0x85, 0xC9, 0x74, 0x2A, 0x8B, 0x81, 0xB4, 0x00, 0x00, 0x00,
    0x8B, 0xF0, 0x2B, 0x35, 0x04, 0x96, 0x52, 0x02, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF0, 0xF6, 0x81,
    0xC0, 0x00, 0x00, 0x00, 0x02, 0x74, 0x09, 0x8B, 0x01, 0x8B, 0x50, 0x10, 0x6A, 0x00, 0xFF, 0xD2,
    0x8B, 0xCE, 0xEB, 0xD2, 0x5E, 0xC3, 0x56, 0x57, 0x8B, 0x3D, 0x8C, 0xB1, 0xF9, 0x02, 0x85, 0xFF,
    0x74, 0x2E, 0x8B, 0x87, 0xBC, 0x00, 0x00, 0x00, 0x8B, 0xF0, 0x2B, 0x35, 0x8C, 0xB1, 0xF9, 0x02,
    0x8B, 0xCF, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF0, 0x8B, 0x07, 0x8B, 0x50, 0x34, 0xFF, 0xD2, 0x85,
    0xC0, 0x75, 0x09, 0x57, 0xE8, 0xFD, 0xE1, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x8B, 0xFE, 0xEB, 0xCE,
    0x5F, 0x5E, 0xC3, 0x56, 0x8B, 0x35, 0x98, 0xD7, 0x15, 0x02, 0x85, 0xF6, 0x74, 0x26, 0xEB, 0x03,
    0x8D, 0x49, 0x00, 0xF6, 0x86, 0x20, 0x01, 0x00, 0x00, 0x20, 0x75, 0x06, 0x56, 0xE8, 0x71, 0xF4,
    0xFF, 0xFF, 0x8B, 0xB6, 0xB4, 0x00, 0x00, 0x00, 0x3B, 0x35, 0x98, 0xD7, 0x15, 0x02, 0x74, 0x04,
    0x85, 0xF6, 0x75, 0xDF, 0x5E, 0xC3, 0x56, 0x8B, 0x35, 0xD4, 0x3E, 0x14, 0x02, 0x85, 0xF6, 0x74,
    0x1D, 0xEB, 0x03, 0x8D, 0x49, 0x00, 0x56, 0xE8, 0x4A, 0xF7, 0xFF, 0xFF, 0x8B, 0x76, 0x04, 0x83,
    0xC4, 0x04, 0x3B, 0x35, 0xD4, 0x3E, 0x14, 0x02, 0x74, 0x04, 0x85, 0xF6, 0x75, 0xE8, 0x5E, 0xC3,
    0x51, 0x53, 0x55, 0x56, 0x57, 0xBB, 0xB8, 0x92, 0x14, 0x03, 0xBD, 0x01, 0x00, 0x00, 0x00, 0x90,
    0x83, 0x3B, 0x00, 0x74, 0x44, 0x33, 0xF6, 0x8D, 0xBB, 0x4C, 0x03, 0x00, 0x00, 0x8B, 0x0F, 0x85,
    0xC9, 0x74, 0x29, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64,
    0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x15, 0x39, 0x48, 0x44, 0x75, 0x10, 0x39, 0x68, 0x4C, 0x75,
    0x0B, 0x56, 0x8B, 0xCB, 0xE8, 0x17, 0xBC, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x03, 0xF5, 0x81, 0xC7,
    0xA8, 0x00, 0x00, 0x00, 0x83, 0xFE, 0x08, 0x7C, 0xC4, 0x81, 0xC3, 0x84, 0x0C, 0x00, 0x00, 0x81,
    0xFB, 0x38, 0x23, 0x16, 0x03, 0x7C, 0xA9, 0x5F, 0x5E, 0x5D, 0x5B, 0x59, 0xC3,
];
const WALKER_OFFSETS: [usize; 11] = [0, 68, 136, 204, 269, 310, 374, 435, 486, 528, 637];

const OVERLAYS_UPDATE_BODY: [u8; 70] = [
    0x51, 0xA1, 0xE8, 0xC4, 0xEA, 0x00, 0x85, 0xC0, 0x56, 0x57, 0x7E, 0x36, 0x8D, 0x78, 0xFF, 0xA1,
    0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB8, 0x8B, 0x01, 0x8B, 0x50, 0x08, 0xFF, 0xD2, 0x84, 0xC0,
    0x75, 0x0D, 0xA1, 0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x34, 0xB8, 0xE8, 0x61, 0xFF, 0xFF, 0xFF, 0xA1,
    0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB8, 0x80, 0x79, 0x04, 0x00, 0x74, 0x05, 0x83, 0xEF, 0x01,
    0x79, 0xD2, 0x5F, 0x5E, 0x59, 0xC3,
];

const OVERLAYS_RENDER_BODY: [u8; 284] = [
    0x8B, 0x15, 0xE8, 0xC4, 0xEA, 0x00, 0x83, 0xEC, 0x20, 0x85, 0xD2, 0x0F, 0x8E, 0x07, 0x01, 0x00,
    0x00, 0x8B, 0x0D, 0xE0, 0xC4, 0xEA, 0x00, 0x8D, 0x42, 0xFF, 0x85, 0xC0, 0x56, 0x7E, 0x11, 0x90,
    0x8B, 0x34, 0x81, 0x80, 0x7E, 0x04, 0x00, 0x74, 0x07, 0x83, 0xE8, 0x01, 0x85, 0xC0, 0x7F, 0xF0,
    0x3B, 0xC2, 0x8B, 0xF0, 0x7D, 0x38, 0xEB, 0x0E, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x0D, 0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB1, 0x8B, 0x01, 0x8B, 0x50, 0x10, 0xFF, 0xD2,
    0x84, 0xC0, 0x75, 0x0F, 0xA1, 0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB0, 0x8B, 0x11, 0x8B, 0x42,
    0x0C, 0xFF, 0xD0, 0x83, 0xC6, 0x01, 0x3B, 0x35, 0xE8, 0xC4, 0xEA, 0x00, 0x7C, 0xD2, 0x64, 0x8B,
    0x0D, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x80, 0x78, 0x5C, 0x00, 0x75, 0x10, 0x81, 0x78, 0x60,
    0xA0, 0x26, 0xEC, 0x00, 0x75, 0x07, 0xB8, 0x10, 0x26, 0xEC, 0x00, 0xEB, 0x03, 0x8B, 0x40, 0x60,
    0xD9, 0x40, 0x74, 0x8D, 0x74, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x40, 0x78, 0xD9, 0x5C,
    0x24, 0x08, 0xD9, 0x40, 0x70, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24,
    0x1C, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0xE8, 0xD9, 0x54, 0x24, 0x0C, 0xD9,
    0x54, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x14, 0xE8, 0xC4, 0x93, 0x5B, 0x00, 0x33, 0xF6, 0x39, 0x35,
    0xE8, 0xC4, 0xEA, 0x00, 0x7E, 0x38, 0xEB, 0x08, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x90,
    0x8B, 0x15, 0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB2, 0x8B, 0x01, 0x8B, 0x50, 0x10, 0xFF, 0xD2,
    0x3C, 0x01, 0x75, 0x0F, 0xA1, 0xE0, 0xC4, 0xEA, 0x00, 0x8B, 0x0C, 0xB0, 0x8B, 0x11, 0x8B, 0x42,
    0x0C, 0xFF, 0xD0, 0x83, 0xC6, 0x01, 0x3B, 0x35, 0xE8, 0xC4, 0xEA, 0x00, 0x7C, 0xD2, 0x8D, 0x74,
    0x24, 0x18, 0xE8, 0x79, 0x93, 0x5B, 0x00, 0x5E, 0x83, 0xC4, 0x20, 0xC3,
];

const INTERFACE_RENDER_BODY: [u8; 147] = [
    0x51, 0xA1, 0x80, 0xCF, 0xE8, 0x00, 0x83, 0xF8, 0x14, 0x77, 0x0F, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x94, 0xCF, 0xE8, 0x00, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xD0, 0xA1, 0x40, 0x27, 0xEC, 0x00, 0x8B,
    0x0D, 0x44, 0x27, 0xEC, 0x00, 0xA3, 0x28, 0x4B, 0xA1, 0x02, 0xB0, 0xFF, 0x89, 0x0D, 0x2C, 0x4B,
    0xA1, 0x02, 0xB9, 0x40, 0x27, 0xEC, 0x00, 0xC7, 0x05, 0x48, 0xC5, 0xE8, 0x00, 0x01, 0x00, 0x00,
    0x00, 0xC7, 0x05, 0x14, 0x8B, 0x52, 0x02, 0x00, 0x00, 0x00, 0x00, 0xA2, 0xBC, 0x15, 0xEB, 0x00,
    0xA2, 0xBD, 0x15, 0xEB, 0x00, 0xA2, 0xBE, 0x15, 0xEB, 0x00, 0xA2, 0xBF, 0x15, 0xEB, 0x00, 0xE8,
    0xBC, 0x14, 0x52, 0x00, 0x0F, 0xB6, 0x15, 0xBF, 0x15, 0xEB, 0x00, 0x0F, 0xB6, 0x05, 0xBE, 0x15,
    0xEB, 0x00, 0x0F, 0xB6, 0x0D, 0xBD, 0x15, 0xEB, 0x00, 0x52, 0x0F, 0xB6, 0x15, 0xBC, 0x15, 0xEB,
    0x00, 0x50, 0x51, 0x52, 0xE8, 0x07, 0x14, 0x52, 0x00, 0x83, 0xC4, 0x10, 0xE8, 0xFF, 0x88, 0x39,
    0x00, 0x59, 0xC3,
];
