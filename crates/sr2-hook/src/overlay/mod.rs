//! Opt-in Dear ImGui overlay, drawn on the game's Direct3D 9 device.
//!
//! Nothing here runs unless a file `sr2-imgui.txt` exists in the game folder (the working
//! directory). If it also contains the word `showcase`, the menu opens by itself once the game is
//! running (for unattended screenshots). With the file, `menu::input` hands the F7 menu to this
//! overlay (see `ui.rs`), and `poll` hooks `IDirect3DDevice9::Present` (vtable slot 17) on the
//! game's real device (global 0x252A2D4, docs/analysis/07_render.md). The hook draws ImGui just
//! before the original Present, from the render thread, through `d3d9::Backend`.
//!
//! Device loss and `Reset` need no hooks: the backend keeps no default-pool resources (it draws
//! with `DrawIndexedPrimitiveUP`, the font texture is managed), and the game's own
//! `gfx_check_device` swaps the device pointer to its null device while lost, which the real
//! device's vtable never sees.

mod d3d9;
mod ui;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

const CONFIG_FILE: &str = "sr2-imgui.txt";
/// The real IDirect3DDevice9 (the one every command uses, unless it is lost).
const REAL_DEVICE: usize = 0x0252_A2D4;
const PRESENT_SLOT: usize = 17;

struct Config {
    showcase: bool,
}

fn config() -> &'static Option<Config> {
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    CONFIG.get_or_init(|| match std::fs::read_to_string(CONFIG_FILE) {
        Ok(text) => {
            let showcase = text.to_ascii_lowercase().contains("showcase");
            crate::log(&format!("overlay: {CONFIG_FILE} found, ImGui overlay on{}", if showcase { " (showcase)" } else { "" }));
            Some(Config { showcase })
        }
        Err(_) => None,
    })
}

/// The opt-in file exists.
pub fn active() -> bool {
    config().is_some()
}

/// The opt-in file asks for the showcase mode.
pub fn showcase() -> bool {
    config().as_ref().is_some_and(|c| c.showcase)
}

// --- the Present hook --------------------------------------------------------------------------

/// Hooked vtables: (vtable address, original Present).
static HOOKS: [(AtomicUsize, AtomicUsize); 4] = [const { (AtomicUsize::new(0), AtomicUsize::new(0)) }; 4];
static DISABLED: AtomicBool = AtomicBool::new(false);
static FAILURES: AtomicU32 = AtomicU32::new(0);
static POLLS: AtomicU32 = AtomicU32::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);

fn readable(p: usize) -> bool {
    (0x10000..0x8000_0000).contains(&p)
}

/// Game thread, once a frame: hooks the real device's Present once it exists (and again, cheaply
/// checked every 120 frames, if the game ever makes a device with another vtable).
pub fn poll() {
    if DISABLED.load(Ordering::Relaxed) {
        return;
    }
    let n = POLLS.fetch_add(1, Ordering::Relaxed);
    if n > 0 && n % 120 != 0 {
        return;
    }
    let dev = unsafe { *(REAL_DEVICE as *const usize) };
    if !readable(dev) {
        return;
    }
    let vtbl = unsafe { *(dev as *const usize) };
    if !readable(vtbl) || HOOKS.iter().any(|h| h.0.load(Ordering::Acquire) == vtbl) {
        return;
    }
    let Some(free) = HOOKS.iter().find(|h| h.0.load(Ordering::Acquire) == 0) else {
        return;
    };
    let entry = (vtbl + PRESENT_SLOT * 4) as *mut usize;
    let original = unsafe { *entry };
    if !readable(original) {
        crate::log("overlay: Present slot looks wrong, not hooking");
        DISABLED.store(true, Ordering::Relaxed);
        return;
    }
    let mut old = 0u32;
    if unsafe { VirtualProtect(entry.cast(), 4, PAGE_READWRITE, &mut old) } == 0 {
        crate::log("overlay: VirtualProtect on the device vtable failed");
        DISABLED.store(true, Ordering::Relaxed);
        return;
    }
    // Publish the original before the hook can run.
    free.1.store(original, Ordering::Release);
    free.0.store(vtbl, Ordering::Release);
    unsafe {
        entry.write_volatile(present_hook as extern "system" fn(usize, usize, usize, usize, usize) -> i32 as usize);
        VirtualProtect(entry.cast(), 4, old, &mut old);
    }
    crate::log(&format!("overlay: hooked IDirect3DDevice9::Present, device {dev:#x}, vtable {vtbl:#x}, original {original:#x}"));
}

fn original_for(this: usize) -> usize {
    let vtbl = unsafe { *(this as *const usize) };
    HOOKS.iter().find(|h| h.0.load(Ordering::Acquire) == vtbl).map_or(0, |h| h.1.load(Ordering::Acquire))
}

/// `HRESULT IDirect3DDevice9::Present(this, src, dst, window, dirty)`, stdcall.
extern "system" fn present_hook(this: usize, a: usize, b: usize, c: usize, d: usize) -> i32 {
    let original = original_for(this);
    // Only the game's real device is drawn on; never re-enter, never after repeated failures.
    if this == unsafe { *(REAL_DEVICE as *const usize) }
        && !DISABLED.load(Ordering::Relaxed)
        && !BUSY.swap(true, Ordering::Acquire)
    {
        let r = std::panic::catch_unwind(|| unsafe { ui::frame(this) });
        BUSY.store(false, Ordering::Release);
        if r.is_err() && FAILURES.fetch_add(1, Ordering::Relaxed) >= 2 {
            DISABLED.store(true, Ordering::Relaxed);
            crate::log("overlay: panicked three times, drawing disabled");
        }
    }
    let f: extern "system" fn(usize, usize, usize, usize, usize) -> i32 = unsafe { std::mem::transmute(original) };
    f(this, a, b, c, d)
}

pub(crate) fn disable(why: &str) {
    crate::log(&format!("overlay: disabled: {why}"));
    DISABLED.store(true, Ordering::Relaxed);
}
