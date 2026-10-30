//! Detours into the original game, installed once `.text` is decrypted.
//!
//! Addresses are for SR2_pc.exe sha256 e20156a0…cfe8 (Steam, AppID 9480);
//! see docs/functions.csv.

use std::arch::{asm, naked_asm};
use std::sync::Once;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use sr2_formats::hash::filename_hash_ptr;

use crate::{detour, log};

// --- 0x520450 game_frame_tick (observe only) -------------------------------

/// cdecl, no args, returns nonzero to quit. Called in a loop by
/// `game_main_thread_proc` (0x520790).
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

// --- 0xBF2BD0 hash_filename_ci (replaced) ----------------------------------

/// Custom LTCG convention: EDX = `const char*`, result in EAX.
/// On exit the original leaves ECX = 0 and EDX one past the NUL (or, for a
/// NULL string, EDX = 0 and ECX untouched). The thunk reproduces that, in case
/// a caller relies on it, and preserves every other register.
const HASH_FILENAME_CI: usize = 0x00BF_2BD0;
const HASH_FILENAME_CI_BODY: [u8; 48] = [
    0x85, 0xD2, 0x75, 0x04, 0x83, 0xC8, 0xFF, 0xC3, 0x0F, 0xBE, 0x0A, 0x33, 0xC0, 0x83, 0xC2, 0x01,
    0x85, 0xC9, 0x74, 0x1C, 0x56, 0x8D, 0x71, 0xBF, 0x83, 0xFE, 0x19, 0x77, 0x03, 0x83, 0xC1, 0x20,
    0xC1, 0xC0, 0x06, 0x33, 0xC1, 0x0F, 0xBE, 0x0A, 0x83, 0xC2, 0x01, 0x85, 0xC9, 0x75, 0xE6, 0x5E,
];

extern "C" fn hash_filename_ci_impl(name: *const u8) -> u32 {
    unsafe { filename_hash_ptr(name) }
}

#[unsafe(naked)]
extern "C" fn hash_filename_ci_thunk() {
    naked_asm!(
        "push ecx",
        "push edx",
        "push edx",
        "call {imp}",
        "add esp, 4",
        "pop edx",
        "pop ecx",
        "test edx, edx",
        "jz 3f",
        "2:",
        "cmp byte ptr [edx], 0",
        "lea edx, [edx + 1]",
        "jne 2b",
        "xor ecx, ecx",
        "3:",
        "ret",
        imp = sym hash_filename_ci_impl,
    )
}

/// Call a function with the original's convention (EDX = str).
unsafe fn call_edx_str(f: usize, s: *const u8) -> u32 {
    let r: u32;
    unsafe {
        asm!("call {f}", f = in(reg) f, inout("edx") s => _, out("eax") r, out("ecx") _);
    }
    r
}

/// Run the original and the Rust thunk on the same inputs. Only replace if they agree.
fn hash_self_test() -> bool {
    let cases: [&[u8]; 6] =
        [b"\0", b"patch.vpp_pc\0", b"CUSTOMIZATION_ITEMS.XTBL\0", b"Sr2_City.Chunk_PC\0", b"\xe9t\xffZ[@`{\0", b"a\0"];
    let thunk = hash_filename_ci_thunk as extern "C" fn() as usize;
    for c in cases {
        let (a, b) = unsafe { (call_edx_str(HASH_FILENAME_CI, c.as_ptr()), call_edx_str(thunk, c.as_ptr())) };
        if a != b {
            log(&format!("hash self-test mismatch for {c:?}: original {a:#x}, rust {b:#x}"));
            return false;
        }
    }
    let (a, b) = unsafe { (call_edx_str(HASH_FILENAME_CI, std::ptr::null()), call_edx_str(thunk, std::ptr::null())) };
    a == b
}

/// Install all hooks. Safe to call repeatedly; runs once.
pub fn install_all() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        match detour::install(
            GAME_FRAME_TICK,
            &GAME_FRAME_TICK_PROLOGUE,
            game_frame_tick_hook as extern "C" fn() -> i32 as usize,
        ) {
            Ok(tramp) => {
                FRAME_TICK_ORIGINAL.store(tramp, Ordering::Relaxed);
                log("hooked game_frame_tick @ 0x520450");
            }
            Err(e) => log(&format!("hook game_frame_tick failed: {e:?}")),
        }

        let current = std::slice::from_raw_parts(HASH_FILENAME_CI as *const u8, HASH_FILENAME_CI_BODY.len());
        if current != HASH_FILENAME_CI_BODY {
            log("hash_filename_ci: unexpected bytes, not replacing");
        } else if !hash_self_test() {
            log("hash_filename_ci: self-test failed, not replacing");
        } else {
            match detour::replace(
                HASH_FILENAME_CI,
                &HASH_FILENAME_CI_BODY,
                hash_filename_ci_thunk as extern "C" fn() as usize,
            ) {
                Ok(()) => log("replaced hash_filename_ci @ 0xbf2bd0 with Rust (self-test passed)"),
                Err(e) => log(&format!("replace hash_filename_ci failed: {e:?}")),
            }
        }
    });
}
