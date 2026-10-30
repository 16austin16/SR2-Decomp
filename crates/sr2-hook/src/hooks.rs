//! Detours into the original game, installed once `.text` is decrypted.
//!
//! Addresses are for SR2_pc.exe sha256 e20156a0…cfe8 (Steam, AppID 9480);
//! see docs/functions.csv.

use std::sync::Once;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::{detour, log};

/// 0x520450 `game_frame_tick`: cdecl, no args, returns nonzero to quit.
/// Called in a loop by `game_main_thread_proc` (0x520790).
const GAME_FRAME_TICK: usize = 0x0052_0450;
const GAME_FRAME_TICK_PROLOGUE: [u8; 9] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x10];

static FRAME_TICK_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);

extern "C" fn game_frame_tick_hook() -> i32 {
    let original: extern "C" fn() -> i32 =
        unsafe { std::mem::transmute(FRAME_TICK_ORIGINAL.load(Ordering::Relaxed)) };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n % 3600 == 0 {
        log(&format!("game_frame_tick: frame {n}"));
    }
    original()
}

/// Install all hooks. Safe to call repeatedly; runs once.
pub fn install_all() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        match detour::install(GAME_FRAME_TICK, &GAME_FRAME_TICK_PROLOGUE, game_frame_tick_hook as extern "C" fn() -> i32 as usize) {
            Ok(tramp) => {
                FRAME_TICK_ORIGINAL.store(tramp, Ordering::Relaxed);
                log("hooked game_frame_tick @ 0x520450");
            }
            Err(e) => log(&format!("hook game_frame_tick failed: {e:?}")),
        }
    });
}
