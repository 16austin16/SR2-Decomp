//! Out-of-memory guard for `peg_texture_create` (0xD19720).
//!
//! The game never checks the HRESULT of `CreateTexture` / `CreateCubeTexture`
//! (device slots 0x5C / 0x64). When one fails (address space exhausted or too
//! fragmented for a big texture), the texture pointer at +0x2C stays null and the
//! upload's LockRect at 0xD19936 crashes on it (docs/analysis/04_mempool.md, "Address
//! space: what uses it, and what can grow").
//!
//! Both creates continue at 0xD19807 (`mov eax, [esp+0x20]; cdq`, exactly 5 bytes),
//! with EBX = &texture (+0x2C). That spot jumps here: with a texture, the two
//! instructions run and execution goes back to 0xD1980C; without one, the failure is
//! logged and the function leaves through its own "nothing to upload" exit 0xD19996
//! (the same stack depth: the COM call popped its arguments). The texture then draws
//! black instead of crashing the game.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::{detour, log};

const AFTER_CREATE: usize = 0x00D1_9807;
const AFTER_CREATE_BYTES: [u8; 5] = [0x8B, 0x44, 0x24, 0x20, 0x99];
const RESUME: usize = 0x00D1_980C;
const EXIT: usize = 0x00D1_9996;

static FAILURES: AtomicU32 = AtomicU32::new(0);
static CREATES: AtomicU32 = AtomicU32::new(0);

/// Test switch: fake a failure for creates number FORCE_FROM.. FORCE_FROM + FORCE_COUNT
/// (the texture is released and the pointer cleared), to prove the exit works. None
/// in normal builds.
const FORCE: Option<(u32, u32)> = None;

/// EBX = &texture, ESI = peg record. True = take the failure exit.
extern "C" fn failed(tex: *mut u32, peg: u32) -> bool {
    let n = CREATES.fetch_add(1, Ordering::Relaxed) + 1;
    unsafe {
        if let Some((from, count)) = FORCE
            && n >= from
            && n < from + count
            && *tex != 0
        {
            let t = *tex as usize;
            let release: extern "system" fn(usize) -> u32 = std::mem::transmute(*((*(t as *const usize)) as *const usize).add(2));
            release(t);
            *tex = 0;
            log(&format!("texguard: test: faking a failed create (number {n})"));
        }
        if *tex != 0 {
            return false;
        }
    }
    let f = FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
    if f <= 20 || f.is_multiple_of(100) {
        log(&format!("texguard: CreateTexture failed (peg record {peg:#x}); skipped the upload, failure {f}"));
    }
    true
}

#[unsafe(naked)]
extern "C" fn guard() {
    std::arch::naked_asm!(
        "pushad",
        "push esi",
        "push ebx",
        "call {failed}",
        "add esp, 8",
        "test al, al",
        "popad", // leaves the flags alone
        "jnz 2f",
        "mov eax, dword ptr [esp + 0x20]",
        "cdq",
        "push {resume}",
        "ret",
        "2:",
        "xor eax, eax",
        "push {exit}",
        "ret",
        resume = const RESUME,
        exit = const EXIT,
        failed = sym failed,
    )
}

pub fn install() {
    match unsafe { detour::replace(AFTER_CREATE, &AFTER_CREATE_BYTES, guard as extern "C" fn() as usize) } {
        Ok(()) => log("texguard: guarding peg_texture_create's CreateTexture at 0xd19807"),
        Err(e) => log(&format!("texguard: not installed: {e:?}")),
    }
}

pub fn summary() -> String {
    format!(
        "texguard: {} texture creates, {} failed",
        CREATES.load(Ordering::Relaxed),
        FAILURES.load(Ordering::Relaxed)
    )
}
