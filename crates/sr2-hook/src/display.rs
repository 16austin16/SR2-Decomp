//! Display mode from `sr2-hook.ini` in the game folder: any resolution (the game's own
//! list stops short of 1440p) and a borderless window.
//!
//! ```text
//! display = borderless      # borderless | fullscreen | windowed | game (leave it to the game)
//! resolution = desktop      # desktop, or WIDTHxHEIGHT such as 2560x1440
//! ```
//!
//! How (docs/analysis/07_render.md):
//! - The process is made DPI-aware at load, so Windows reports the real desktop size
//!   and does not stretch the picture.
//! - The game picks 4:3 or widescreen UI layouts at boot only (crossing aspect 1.45,
//!   `gfx_set_display_mode` shows "display change needs a restart" instead), so at load
//!   the configured width, height and fullscreen flag are written into `settings.dat`
//!   (%LOCALAPPDATA%\THQ\Saints Row 2: width +0x3C, height +0x40, fullscreen byte
//!   +0x48; the loader 0x774190 has no checksum) before the game reads it. A backup
//!   is kept as settings.dat.bak.
//! - The game applies a pending display mode after a Present (`gfx_check_device`
//!   0xD20230: `0x22FD848` set -> `gfx_set_display_mode(w 0x22FD84C, h 0x22FD850,
//!   fullscreen 0x22FD854, vsync bit 4 of 0x22FD85C)`). When the device's mode
//!   (present parameters 0x22F63F8) differs from the ini, this sets those fields, and
//!   the options menu's working copy (0x1F7A998) to match.
//! - Borderless is windowed mode with the window restyled to a bare popup covering
//!   the screen. The game never restyles its window (the style it passes to
//!   AdjustWindowRect only sizes it), so this stays.
//!
//! All of it runs on the render thread, which owns the window and applies the mode.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetSystemMetrics, WS_CAPTION, WS_THICKFRAME, GetWindowLongA, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN, SWP_FRAMECHANGED,
    SWP_SHOWWINDOW, SetProcessDPIAware, SetWindowLongA, SetWindowPos, WS_POPUP, WS_VISIBLE,
};

use crate::log;

const INI: &str = "sr2-hook.ini";
const DEFAULT_INI: &str = "\
# Saints Row 2 hook DLL settings.
# display: borderless | fullscreen | windowed | game (leave it to the game's options)
display = borderless
# resolution: desktop, or WIDTHxHEIGHT such as 2560x1440
resolution = desktop
";

const HWND: usize = 0x0252_A410;
const PP_WIDTH: usize = 0x022F_63F8; // D3DPRESENT_PARAMETERS
const PP_HEIGHT: usize = 0x022F_63FC;
const PP_WINDOWED: usize = 0x022F_6418;
const PENDING: usize = 0x022F_D848; // u8
const PENDING_W: usize = 0x022F_D84C;
const PENDING_H: usize = 0x022F_D850;
const PENDING_FULLSCREEN: usize = 0x022F_D854; // u8
const OPTIONS_W: usize = 0x01F7_A9A0; // the options menu's working copy
const OPTIONS_H: usize = 0x01F7_A9A4;
const OPTIONS_FULLSCREEN: usize = 0x01F7_A9AC; // u8
const DEVICE: usize = 0x0252_A2D0; // the null device while lost
const REAL_DEVICE: usize = 0x0252_A2D4;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Game,
    Fullscreen,
    Windowed,
    Borderless,
}

#[derive(Clone, Copy, Debug)]
struct Config {
    mode: Mode,
    /// None = the desktop size.
    size: Option<(u32, u32)>,
}

fn config() -> Config {
    static C: OnceLock<Config> = OnceLock::new();
    *C.get_or_init(|| {
        let text = std::fs::read_to_string(INI).unwrap_or_else(|_| {
            let _ = std::fs::write(INI, DEFAULT_INI);
            DEFAULT_INI.to_string()
        });
        let mut c = Config { mode: Mode::Game, size: None };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().to_ascii_lowercase();
            match k.trim().to_ascii_lowercase().as_str() {
                "display" => {
                    c.mode = match v.as_str() {
                        "borderless" => Mode::Borderless,
                        "fullscreen" => Mode::Fullscreen,
                        "windowed" => Mode::Windowed,
                        _ => Mode::Game,
                    }
                }
                "resolution" => {
                    c.size = v.split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                }
                _ => {}
            }
        }
        log(&format!("display: {INI}: {c:?}"));
        c
    })
}

static TARGET: OnceLock<(u32, u32)> = OnceLock::new();

/// At load (DllMain), before the game reads its settings and makes its window.
pub fn early() {
    let c = config();
    if c.mode == Mode::Game {
        return;
    }
    unsafe { SetProcessDPIAware() };
    let (w, h) = c.size.unwrap_or_else(|| unsafe {
        (GetSystemMetrics(SM_CXSCREEN) as u32, GetSystemMetrics(SM_CYSCREEN) as u32)
    });
    let _ = TARGET.set((w, h));
    write_settings(w, h, c.mode == Mode::Fullscreen);
}

fn write_settings(w: u32, h: u32, fullscreen: bool) {
    let Ok(dir) = std::env::var("LOCALAPPDATA") else { return };
    let path = format!(r"{dir}\THQ\Saints Row 2\settings.dat");
    let Ok(mut d) = std::fs::read(&path) else {
        log(&format!("display: no {path}; the game's defaults stay for this boot"));
        return;
    };
    let field = |d: &[u8], o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    if d.len() != 340 || !(320..=16384).contains(&field(&d, 0x3C)) || !(200..=16384).contains(&field(&d, 0x40)) {
        log("display: settings.dat has an unexpected layout; not changed");
        return;
    }
    let old = (field(&d, 0x3C), field(&d, 0x40), d[0x48]);
    if old == (w, h, fullscreen as u8) {
        return;
    }
    let bak = format!("{path}.bak");
    if std::fs::metadata(&bak).is_err() {
        let _ = std::fs::write(&bak, &d);
    }
    d[0x3C..0x40].copy_from_slice(&w.to_le_bytes());
    d[0x40..0x44].copy_from_slice(&h.to_le_bytes());
    d[0x48] = fullscreen as u8;
    match std::fs::write(&path, &d) {
        Ok(()) => log(&format!("display: settings.dat {}x{} fullscreen {} -> {w}x{h} fullscreen {}", old.0, old.1, old.2, fullscreen as u8)),
        Err(e) => log(&format!("display: could not write settings.dat: {e}")),
    }
}

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

static FRAMES: AtomicU32 = AtomicU32::new(0);
static TRIES: AtomicU32 = AtomicU32::new(0);
const MAX_TRIES: u32 = 3;

/// Called on the render thread once a frame.
pub fn render_tick() {
    let c = config();
    if c.mode == Mode::Game {
        return;
    }
    let n = FRAMES.fetch_add(1, Ordering::Relaxed);
    if n < 120 || n % 30 != 0 {
        return;
    }
    unsafe {
        let hwnd = rd(HWND) as usize;
        if hwnd == 0 || rd(DEVICE) != rd(REAL_DEVICE) || *(PENDING as *const u8) != 0 {
            return;
        }
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN) as u32, GetSystemMetrics(SM_CYSCREEN) as u32);
        let Some(&(w, h)) = TARGET.get() else { return };
        let fullscreen = c.mode == Mode::Fullscreen;
        let (cw, ch, windowed) = (rd(PP_WIDTH), rd(PP_HEIGHT), rd(PP_WINDOWED) != 0);
        if (cw, ch, !windowed) != (w, h, fullscreen) {
            let t = TRIES.fetch_add(1, Ordering::Relaxed);
            if t < MAX_TRIES {
                log(&format!(
                    "display: frame {n}: {cw}x{ch} {} -> {w}x{h} {} (try {})",
                    if windowed { "windowed" } else { "fullscreen" },
                    if fullscreen { "fullscreen" } else { "windowed" },
                    t + 1
                ));
                for (a, v) in [(PENDING_W, w), (PENDING_H, h), (OPTIONS_W, w), (OPTIONS_H, h)] {
                    *(a as *mut u32) = v;
                }
                *(PENDING_FULLSCREEN as *mut u8) = fullscreen as u8;
                *(OPTIONS_FULLSCREEN as *mut u8) = fullscreen as u8;
                *(PENDING as *mut u8) = 1;
            } else if t == MAX_TRIES {
                log(&format!("display: gave up; the device stays at {cw}x{ch}"));
            }
            return;
        }
        if c.mode == Mode::Borderless {
            let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
            let style = (WS_POPUP | WS_VISIBLE) as i32;
            let mut r = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let want = (0, 0, sw as i32, sh as i32);
            // Windows adds bits of its own (WS_CLIPSIBLINGS): only a frame counts as wrong.
            let framed = GetWindowLongA(hwnd, GWL_STYLE) as u32 & (WS_CAPTION | WS_THICKFRAME) != 0;
            if framed || (r.left, r.top, r.right, r.bottom) != want {
                SetWindowLongA(hwnd, GWL_STYLE, style);
                SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, sw as i32, sh as i32, SWP_FRAMECHANGED | SWP_SHOWWINDOW);
                log(&format!("display: frame {n}: borderless window {sw}x{sh}, back buffer {w}x{h}"));
            }
        }
    }
}
