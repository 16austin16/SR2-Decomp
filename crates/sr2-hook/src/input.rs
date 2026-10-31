//! Input, docs/analysis/14_input.md: the per-frame update (`input_do_frame` and its
//! tree: keyboard events, keyboard/mouse mapping, button and stick state, idle
//! time), the key queue the window procedure feeds, and the key/button queries.
//! Still original: the XInput / DirectInput pad polls, the rebind captures and
//! `mouse_update`.
//!
//! Verified in-process: in `Mode::Shadow` every hooked call runs the original on a
//! snapshot of the input state, then the Rust on the same snapshot, and compares
//! the two results byte for byte (the Rust result is kept). Both runs see the same
//! clock: `QueryPerformanceCounter` is wrapped, recorded during the original and
//! replayed to the Rust.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

use crate::{detour, log};

#[derive(PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)] // one mode is used per build
enum Mode {
    Original,
    /// Both, compared; the Rust result is kept.
    Shadow,
    Rust,
}

/// Shadow for test runs (tools/ingame/run_input.ps1); Rust otherwise, since the
/// snapshot restore can still race other threads' writes nearby.
const MODE: Mode = Mode::Rust;

// Keys: 256 x 12 bytes {u8 down, u8 pressed this frame, u8 eaten, -, u32 press time,
// u32 last event time}. Mouse buttons for keys 0x100..0x102.
const KEYS: usize = 0x0234_8B80;
const KEY_BLOCK: usize = 0x0252_A54F; // text entry: no key reads
const MOUSE_0X100: usize = 0x0234_F473;
const MOUSE_0X101: usize = 0x0234_F483;
const MOUSE_0X102: usize = 0x0234_F47B;
// Key event ring (32 x {u32 scan, u8 down}) and key-up ring (32 x {u32 scan, u8 delay}).
const EVENTS: usize = 0x0234_8A80;
const EVENTS_W: usize = 0x0252_A574;
const EVENTS_R: usize = 0x0252_A570;
const UPS: usize = 0x0234_8980;
const UPS_W: usize = 0x0252_A57C;
const UPS_R: usize = 0x0252_A578;
// Typed-key ring (32 x u32 scan | modifier bits).
const TEXT: usize = 0x0234_9780;
const TEXT_W: usize = 0x0234_897C;
const TEXT_R: usize = 0x0234_9800;
// Rebind capture.
const CAPTURE_KEYS: usize = 0x0252_A58F;
const CAPTURE_CB: usize = 0x0252_A5B3;
const CAPTURE_PAD: usize = 0x0252_A5B0;
const CAPTURE_FN: usize = 0x0252_A5B4;
const REBIND_KEYS: usize = 0x0234_9848;
// Pad record 0: 87 buttons (0x24 each) at +0x10, 8 sticks (0x40 each) at +0xC4C.
const PAD: usize = 0x0234_78A8;
const BUTTONS: usize = 0x0234_78B8;
const BUTTON_COUNT: usize = 87;
const STICKS: usize = 0x0234_84F4;
const BINDINGS: usize = 0x0234_E778; // 87 x {u32 key, u32 pad button}
const PAD_MODE: usize = 0x0234_86FC; // 3 = XInput
const ANY_INPUT: usize = 0x0234_86F8;
const IDLE_SECONDS: usize = 0x0234_86F4;
const IDLE_LAST: usize = 0x0252_A548;
const IDLE_PAUSED: usize = 0x0252_A54C;
const FRAME_FLAG: usize = 0x0252_A54D;
const WINDOW_ACTIVE: usize = 0x0252_A406;
// Mouse (written by mouse_update).
const MOUSE_DX: usize = 0x0234_F45C;
const MOUSE_DY: usize = 0x0234_F458;
const WHEEL: usize = 0x0234_EA34;
const SENS_X: usize = 0x025F_5C98;
const SENS_Y: usize = 0x025F_5C9C;
// Clock: ms = (QPC - base) / (frequency / 1000), 0 while 0x252A404 is clear.
const CLOCK_ON: usize = 0x0252_A404;
const CLOCK_BASE: usize = 0x0234_6508;
const CLOCK_DIV: usize = 0x0234_64F8;
const QPC_SLOT: usize = 0x00DB_8114;

const POLL_XINPUT: usize = 0x00C1_3C80; // cdecl (pad)
const POLL_DINPUT: usize = 0x00C1_41D0; // cdecl (pad)
const REBIND_CAPTURE_KEYS: usize = 0x00C1_2910;
const REBIND_CAPTURE_PAD: usize = 0x00C1_2B00;

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}
unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(p as *mut u8) = v }
}
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(p as *const f32) }
}
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(p as *mut f32) = v }
}

/// x87 precision control (bits 8-9 of the control word): 0 = 24-bit, which D3D9
/// sets unless asked not to. The original's unstored x87 intermediates round to it.
fn x87_precision() -> u16 {
    let mut cw: u16 = 0;
    unsafe { std::arch::asm!("fnstcw word ptr [{}]", in(reg) &mut cw, options(nostack)) };
    (cw >> 8) & 3
}

/// Round an x87 intermediate the way the current precision control does.
fn rnd(x: f64, pc: u16) -> f64 {
    if pc == 0 { x as f32 as f64 } else { x }
}

/// Call a game function with no register arguments and cdecl stack arguments.
unsafe fn cdecl(f: usize, args: &[u32]) {
    unsafe {
        match *args {
            [] => {
                let f: extern "C" fn() = std::mem::transmute(f);
                f()
            }
            [a] => {
                let f: extern "C" fn(u32) = std::mem::transmute(f);
                f(a)
            }
            _ => unreachable!(),
        }
    }
}

// --- Clock ------------------------------------------------------------------------------

static QPC_REAL: AtomicUsize = AtomicUsize::new(0);
static CLOCK_MODE: AtomicU8 = AtomicU8::new(0); // 0 pass through, 1 record, 2 replay
static CLOCK_THREAD: AtomicU32 = AtomicU32::new(0);
static CLOCK: Mutex<VecDeque<i64>> = Mutex::new(VecDeque::new());
static CLOCK_UNDERFLOW: AtomicU32 = AtomicU32::new(0);
static CLOCK_LEFTOVER: AtomicU32 = AtomicU32::new(0);

extern "system" fn qpc_hook(out: *mut i64) -> i32 {
    let real: extern "system" fn(*mut i64) -> i32 = unsafe { std::mem::transmute(QPC_REAL.load(Ordering::Acquire)) };
    let mode = CLOCK_MODE.load(Ordering::Acquire);
    if mode != 0 && unsafe { GetCurrentThreadId() } == CLOCK_THREAD.load(Ordering::Acquire) {
        let mut q = CLOCK.lock().unwrap_or_else(|p| p.into_inner());
        if mode == 1 {
            let r = real(out);
            q.push_back(unsafe { *out });
            return r;
        }
        if let Some(v) = q.pop_front() {
            unsafe { *out = v };
            return 1;
        }
        CLOCK_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
    }
    real(out)
}

/// The game's millisecond clock, read the way the input code reads it (through the
/// QPC slot, so the shadow replay sees it).
unsafe fn now_ms() -> u32 {
    unsafe {
        if rb(CLOCK_ON) == 0 {
            return 0;
        }
        let qpc: extern "system" fn(*mut i64) -> i32 = std::mem::transmute(rd(QPC_SLOT) as usize);
        let mut c = 0i64;
        qpc(&mut c);
        let base = *(CLOCK_BASE as *const i64);
        let div = *(CLOCK_DIV as *const i64);
        (c.wrapping_sub(base) / div) as u32
    }
}

// --- Keys ---------------------------------------------------------------------------------

/// `input_key_down` (0xC111D0; EAX = key -> EAX, the result in AL): keys 0-255 are
/// down and not eaten (and reads not blocked); 0x100..0x102 are mouse buttons.
unsafe fn key_down(key: u32) -> u32 {
    unsafe {
        if key < 0x100 {
            let k = KEYS + key as usize * 12;
            return (rb(k) != 0 && rb(k + 2) == 0 && rb(KEY_BLOCK) == 0) as u32;
        }
        let b = match key {
            0x100 => rb(MOUSE_0X100),
            0x101 => rb(MOUSE_0X101),
            0x102 => rb(MOUSE_0X102),
            _ => 0,
        };
        (key & 0xFFFF_FF00) | b as u32
    }
}

fn down(key: u32) -> bool {
    unsafe { key_down(key) as u8 != 0 }
}

/// `input_key_queue_push` (0xC10B10; cdecl (scan), DL = down): key-downs go to the
/// event ring, key-ups to the key-up ring with a one-frame delay.
unsafe fn key_queue_push(scan: u32, is_down: u8) {
    unsafe {
        let scan = scan & 0xFF;
        let (ring, w, flag) = if is_down != 0 { (EVENTS, EVENTS_W, is_down) } else { (UPS, UPS_W, 1) };
        let i = rd(w);
        wr(ring + i as usize * 8, scan);
        wb(ring + i as usize * 8 + 4, flag);
        wr(w, if i == 0x1F { 0 } else { i + 1 });
    }
}

/// `input_key_event` (0xC10B80; cdecl (scan, down)): update the key, and for a down
/// event queue the typed key with its modifiers (0x1000 Shift, 0x2000 Alt, 0x4000
/// Ctrl, 0x8000 grave held).
unsafe fn key_event(scan: u32, is_down: u32) {
    unsafe {
        let scan = scan & 0xFF;
        let d = is_down as u8;
        if rb(CAPTURE_KEYS) == 1 || rb(CAPTURE_CB) == 1 || rb(CAPTURE_PAD) == 1 {
            wb(REBIND_KEYS + scan as usize, d);
            if d == 1 {
                return;
            }
        }
        let k = KEYS + scan as usize * 12;
        if rb(k) == 0 {
            if d == 0 {
                wb(k, d);
                return;
            }
            wb(k + 1, 1);
            wr(k + 4, now_ms());
        }
        if d == 0 {
            wb(k, d);
            return;
        }
        let held = |key: usize| rb(KEYS + key * 12) != 0;
        let mut code = scan;
        if held(0x2A) || held(0x36) {
            code |= 0x1000;
        }
        if held(0x38) || held(0xB8) {
            code |= 0x2000;
        }
        if held(0x1D) || held(0x9D) {
            code |= 0x4000;
        }
        if held(0x29) {
            code |= 0x8000;
        }
        wr(k + 8, now_ms());
        let w = rd(TEXT_W);
        let next = (w + 1) & 0x1F;
        if next != rd(TEXT_R) {
            wr(TEXT + w as usize * 4, code);
            wr(TEXT_W, next);
        }
        wb(k, d);
    }
}

/// `input_keyboard_reset` (0xC10EB0): all keys up, the event and text rings empty.
unsafe fn keyboard_reset() {
    unsafe {
        for i in 0..256 {
            let k = KEYS + i * 12;
            wb(k + 1, 0);
            wb(k, 0);
            wr(k + 8, 0);
            wr(k + 4, 0);
        }
        wr(EVENTS_W, 0);
        wr(EVENTS_R, 0);
        wr(TEXT_W, 0);
        wr(TEXT_R, 0);
    }
}

/// `input_keyboard_events` (0xC11710): replay the queued key-downs, then release the
/// keys whose key-up delay has run out (counting the others down).
unsafe fn keyboard_events() {
    unsafe {
        wb(KEY_BLOCK, 0);
        for i in 0..256 {
            wb(KEYS + i * 12 + 1, 0);
            wb(KEYS + i * 12 + 2, 0);
        }
        let next = |i: u32| if i == 0x1F { 0 } else { i + 1 };
        if rd(EVENTS_R) != rd(EVENTS_W) {
            loop {
                let r = rd(EVENTS_R) as usize;
                key_event(rd(EVENTS + r * 8), rb(EVENTS + r * 8 + 4) as u32);
                let r = next(rd(EVENTS_R));
                wr(EVENTS_R, r);
                if r == rd(EVENTS_W) {
                    break;
                }
            }
        }
        let mut r = rd(UPS_R);
        let w = rd(UPS_W);
        if r == w {
            return;
        }
        let capture_keys = rb(CAPTURE_KEYS);
        loop {
            if rb(UPS + r as usize * 8 + 4) != 0 {
                wr(UPS_R, r);
                while r != w {
                    let p = UPS + r as usize * 8 + 4;
                    if rb(p) > 0 {
                        wb(p, rb(p) - 1);
                    }
                    r = next(r);
                }
                return;
            }
            let scan = rb(UPS + r as usize * 8) as usize;
            if capture_keys == 1 || rb(CAPTURE_CB) == 1 || rb(CAPTURE_PAD) == 1 {
                wb(REBIND_KEYS + scan, 0);
            }
            wb(KEYS + scan * 12, 0);
            r = next(r);
            if r == w {
                wr(UPS_R, r);
                return;
            }
        }
    }
}

/// `input_wndproc_keys` (0xC10CD0; cdecl (msg, wParam, lParam)), the window
/// procedure's keyboard callback (only while 0x252A54E is set): key-downs through
/// the key queue (a left Ctrl while AltGr is held is dropped), key-ups straight into
/// the key-up ring, focus changes reset the keyboard.
unsafe fn wndproc_keys(msg: u32, lparam: u32) {
    unsafe {
        if rb(0x0252_A54E) == 0 {
            return;
        }
        let hi = lparam >> 16;
        let mut scan = hi & 0xFF;
        if hi & 0x100 != 0 {
            scan += 0x80;
        }
        match msg {
            7 | 8 => keyboard_reset(),
            0x100 | 0x104 => {
                if scan == 0x1D {
                    let get_async_key_state: extern "system" fn(i32) -> i16 =
                        std::mem::transmute(rd(0x00DB_82D0) as usize);
                    if get_async_key_state(0xA5) < 0 {
                        return;
                    }
                }
                key_queue_push(scan, 1);
            }
            0x101 | 0x105 => key_queue_push(scan, 0),
            _ => {}
        }
    }
}

// --- Mapping, pads, frame -----------------------------------------------------------------

/// `input_map_keyboard_mouse` (0xC13660): every button from its bound key; the
/// mouse delta to the look stick (1) and stick 6, the bound keys to the move stick
/// (0), and a few fixed buttons and sticks from the mouse, wheel and H / J.
unsafe fn map_keyboard_mouse() {
    unsafe {
        let pc = x87_precision();
        for i in 0..BUTTON_COUNT {
            let b = BUTTONS + i * 0x24;
            wf(b + 4, rf(b));
            wf(b, if down(rd(BINDINGS + i * 8)) { 1.0 } else { 0.0 });
        }
        let (dx, dy) = (rd(MOUSE_DX) as i32, rd(MOUSE_DY) as i32);
        let (sx, sy) = (rf(SENS_X) as f64, rf(SENS_Y) as f64);
        let look_x = if dx != 0 { rnd(dx as f64 / 30.0, pc) } else { 0.0 };
        wf(STICKS + 0x40, rnd(look_x * sx, pc) as f32);
        let ndy = dy.wrapping_neg() as f64;
        let look_y = if dy != 0 { rnd(ndy / 30.0, pc) } else { 0.0 };
        wf(STICKS + 0x44, rnd(look_y * sy, pc) as f32);
        wf(STICKS + 0x180, rnd(dx as f64 / 10.0, pc) as f32);
        wf(STICKS + 0x184, rnd(ndy / 30.0, pc) as f32);
        let axis = |plus: usize, minus: usize| {
            if down(rd(BINDINGS + plus * 8)) {
                1.0
            } else if down(rd(BINDINGS + minus * 8)) {
                -1.0
            } else {
                0.0
            }
        };
        wf(STICKS + 4, axis(0, 1));
        wf(STICKS, axis(3, 2));
        wr(STICKS + 0x1C0, rd(STICKS + 0x40));
        wr(STICKS + 0x1C4, rd(STICKS + 0x44));
        let both = rb(MOUSE_0X100) != 0 && rb(MOUSE_0X101) != 0;
        wf(BUTTONS + 19 * 0x24, if both { 1.0 } else { 0.0 });
        let wheel = rd(WHEEL) as i32;
        wf(BUTTONS + 33 * 0x24, if wheel > 0 { 1.0 } else { 0.0 });
        wf(BUTTONS + 34 * 0x24, if wheel < 0 { 1.0 } else { 0.0 });
        let block = rb(KEY_BLOCK) != 0;
        let key = |k: usize| rb(KEYS + k * 12) != 0 && rb(KEYS + k * 12 + 2) == 0 && !block;
        wf(
            STICKS + 0x140,
            if key(0x23) {
                1.0
            } else if key(0x24) {
                -1.0
            } else {
                0.0
            },
        );
        wf(
            STICKS + 0x80,
            if dx > 5 {
                1.0
            } else if dx < -5 {
                -1.0
            } else {
                0.0
            },
        );
        wf(
            STICKS + 0x84,
            if dy > 5 {
                -1.0
            } else if dy < -5 {
                1.0
            } else {
                0.0
            },
        );
    }
}

/// `input_pad_reset_buttons` (0x75C700; EAX = pad): clear the 87 buttons' values
/// and flags and the 8 sticks (axes, eaten flags, repeat timers).
unsafe fn pad_reset_buttons(pad: usize) {
    unsafe {
        for i in 0..BUTTON_COUNT {
            let b = pad + 0x10 + i * 0x24;
            wf(b, 0.0);
            wr(b + 0x18, 0);
            wf(b + 4, 0.0);
            wr(b + 0x14, 0);
            for o in 0x1C..=0x20 {
                wb(b + o, 0);
            }
        }
        for i in 0..8 {
            let s = pad + 0xC4C + i * 0x40;
            wf(s + 4, 0.0);
            wb(s + 0xF, 0);
            wf(s, 0.0);
            wb(s + 0xE, 0);
            for t in 0..4 {
                let t = s + 0x10 + t * 0xC;
                wr(t, u32::MAX);
                wr(t + 4, u32::MAX);
                wb(t + 8, 0);
            }
        }
    }
}

/// `input_poll_devices` (0xC14780): while the window is active, keyboard events,
/// then keyboard/mouse mapping (or, while a rebind captures, buttons to zero), the
/// pad poll, and the rebind captures.
unsafe fn poll_devices() {
    unsafe {
        if rb(WINDOW_ACTIVE) == 0 {
            return;
        }
        keyboard_events();
        if rb(CAPTURE_CB) == 0 && rb(CAPTURE_KEYS) == 0 {
            map_keyboard_mouse();
        } else {
            for i in 0..BUTTON_COUNT {
                let b = BUTTONS + i * 0x24;
                wf(b + 4, rf(b));
                wf(b, 0.0);
            }
        }
        cdecl(if rd(PAD_MODE) == 3 { POLL_XINPUT } else { POLL_DINPUT }, &[PAD as u32]);
        if rb(CAPTURE_KEYS) == 1 {
            cdecl(REBIND_CAPTURE_KEYS, &[]);
        } else if rb(CAPTURE_CB) == 1 {
            cdecl(rd(CAPTURE_FN) as usize, &[]);
        } else if rb(CAPTURE_PAD) != 0 {
            cdecl(REBIND_CAPTURE_PAD, &[]);
        } else {
            return;
        }
        pad_reset_buttons(PAD);
    }
}

/// `input_repeat_timer` (0xC061C0; ESI = timer {start, last, u8 pulse}, XMM0 =
/// value): past 0.5 pulse at once, then after 500 ms every 150 ms.
unsafe fn repeat_timer(t: usize, v: f32) {
    unsafe {
        if v > 0.5 {
            wb(t + 8, 0);
            let now = now_ms();
            if rd(t) == u32::MAX {
                wr(t, now);
                wr(t + 4, now);
                wb(t + 8, 1);
            } else if now.wrapping_sub(rd(t)) as i32 > 500 && now.wrapping_sub(rd(t + 4)) as i32 > 150 {
                wb(t + 8, 1);
                wr(t + 4, now);
            }
        } else {
            wr(t, u32::MAX);
            wr(t + 4, u32::MAX);
            wb(t + 8, 0);
        }
    }
}

/// `input_do_frame` (0xC063D0): clear last frame's eaten flags, poll, then derive
/// the sticks' direction repeats and the buttons' press / repeat / release state,
/// "any input" and the idle time.
unsafe fn do_frame() {
    unsafe {
        let pc = x87_precision();
        wb(FRAME_FLAG, 0);
        for i in 0..BUTTON_COUNT {
            wb(BUTTONS + i * 0x24 + 0x20, 0);
        }
        for i in 0..8 {
            wb(STICKS + i * 0x40 + 0xE, 0);
            wb(STICKS + i * 0x40 + 0xF, 0);
        }
        poll_devices();
        wb(ANY_INPUT, 0);
        for i in 0..8 {
            let s = STICKS + i * 0x40;
            let (mut x, mut y) = (rf(s), rf(s + 4));
            if rb(s + 0xE) != 0 {
                x = 0.0;
            } else if rb(s + 0xC) != 0 {
                x = -0.0 - x;
            }
            if rb(s + 0xF) != 0 {
                y = 0.0;
            } else if rb(s + 0xD) != 0 {
                y = -0.0 - y;
            }
            repeat_timer(s + 0x10, y);
            repeat_timer(s + 0x1C, -0.0 - y);
            repeat_timer(s + 0x28, -0.0 - x);
            repeat_timer(s + 0x34, x);
            if x.abs() > 0.5 || y.abs() > 0.5 {
                wb(ANY_INPUT, 1);
            }
        }
        let now = now_ms();
        for i in 0..BUTTON_COUNT {
            let b = BUTTONS + i * 0x24;
            let on = rf(b) > rf(b + 8) && rb(b + 0x20) == 0;
            let mut fresh = false;
            if rb(b + 0x1F) != 0 {
                if !on {
                    wb(b + 0x1E, 1);
                }
            } else if on {
                wb(b + 0x1C, 1);
                wr(b + 0x14, now);
                wr(b + 0x18, now);
                wb(b + 0x1D, 1);
                fresh = true;
            }
            if !fresh {
                wb(b + 0x1C, 0);
                wb(b + 0x1D, 0);
                if on
                    && now.wrapping_sub(rd(b + 0x14)) as i32 > rd(b + 0x10) as i32
                    && now.wrapping_sub(rd(b + 0x18)) as i32 > rd(b + 0x0C) as i32
                {
                    wr(b + 0x18, now);
                    wb(b + 0x1D, 1);
                }
            }
            wb(b + 0x1F, on as u8);
            if on {
                wb(ANY_INPUT, 1);
            }
        }
        let now = now_ms();
        let last = rd(IDLE_LAST);
        if last != 0 {
            if rb(ANY_INPUT) == 1 {
                wf(IDLE_SECONDS, 0.0);
            } else if rb(IDLE_PAUSED) == 0 {
                let mut d = now.wrapping_sub(last);
                if now < last {
                    d = d.wrapping_sub(1);
                }
                let mut v = d as i32 as f64;
                if (d as i32) < 0 {
                    v = rnd(v + 4294967296.0, pc);
                }
                let v = rnd(rnd(v * 0.0010000000474974513, pc) + rf(IDLE_SECONDS) as f64, pc);
                wf(IDLE_SECONDS, v as f32);
            }
        }
        wr(IDLE_LAST, now);
    }
}

/// `input_button_analog` (0xC06870; EAX = button -> ST0): the value past the press
/// threshold, rescaled to 0..1 with the value's sign.
unsafe fn button_analog(b: usize) -> f32 {
    unsafe {
        let pc = x87_precision();
        let (v, th) = (rf(b), rf(b + 8));
        let sign = if 0.0 > v { -1.0f64 } else { 1.0 };
        let mut d = rnd(v.abs() as f64 - th as f64, pc);
        if 0.0 > d {
            d = 0.0;
        }
        let q = rnd(sign / rnd(1.0 - th as f64, pc), pc);
        rnd(q * (d as f32) as f64, pc) as f32
    }
}

// --- Shadow checking ----------------------------------------------------------------------

/// Everything the hooked functions write, and nothing else (a restore must not undo
/// another thread's writes): pad record 0, the text ring's write index, the key-up
/// and event rings, the key table, the text ring and its read index, the pad's
/// rebind bytes and the rebind key array, their indices and flags, and the pad's
/// raw bytes.
fn regions() -> Vec<(usize, usize)> {
    let mut r = vec![
        (PAD, 0x104C),
        (TEXT_W, 4),
        (UPS, TEXT + 0x84 - UPS),
        (0x0234_9808, 0x14),
        (REBIND_KEYS, 0x100),
        (IDLE_LAST, 8),
        (EVENTS_R, 0x20),
        (0x00E8_DFBF, 1),
    ];
    let raw = unsafe { rd(PAD + 0xF68) } as usize;
    if raw != 0 {
        r.push((raw, 0x20));
    }
    r
}

fn snapshot(regions: &[(usize, usize)]) -> Vec<Vec<u8>> {
    regions.iter().map(|&(p, n)| unsafe { std::slice::from_raw_parts(p as *const u8, n) }.to_vec()).collect()
}

fn restore(regions: &[(usize, usize)], snap: &[Vec<u8>]) {
    for (&(p, n), s) in regions.iter().zip(snap) {
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), p as *mut u8, n) };
    }
}

/// Key events arrive on the window thread. They take this lock, and a shadow check
/// holds it, so no event lands between the original's run and the restore (and is
/// lost). Reentrant for the owning thread.
static LOCK_OWNER: AtomicU32 = AtomicU32::new(0);
static LOCK_DEPTH: AtomicU32 = AtomicU32::new(0);

fn with_lock<R>(f: impl FnOnce() -> R) -> R {
    let me = unsafe { GetCurrentThreadId() };
    if LOCK_OWNER.load(Ordering::Acquire) == me {
        LOCK_DEPTH.fetch_add(1, Ordering::Relaxed);
    } else {
        while LOCK_OWNER.compare_exchange_weak(0, me, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::thread::yield_now();
        }
        LOCK_DEPTH.store(1, Ordering::Relaxed);
    }
    let r = f();
    if LOCK_DEPTH.fetch_sub(1, Ordering::Relaxed) == 1 {
        LOCK_OWNER.store(0, Ordering::Release);
    }
    r
}

static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// Run the original and the Rust on the same input state and clock; keep the Rust.
fn shadow(i: usize, original: impl FnOnce(), rust: impl FnOnce()) {
    with_lock(|| shadow_locked(i, original, rust))
}

fn shadow_locked(i: usize, original: impl FnOnce(), rust: impl FnOnce()) {
    let regs = regions();
    let before = snapshot(&regs);
    CLOCK.lock().unwrap_or_else(|p| p.into_inner()).clear();
    CLOCK_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    CLOCK_MODE.store(1, Ordering::Release);
    original();
    CLOCK_MODE.store(2, Ordering::Release);
    let theirs = snapshot(&regs);
    restore(&regs, &before);
    rust();
    CLOCK_MODE.store(0, Ordering::Release);
    let left = CLOCK.lock().unwrap_or_else(|p| p.into_inner()).len();
    if left > 0 {
        CLOCK_LEFTOVER.fetch_add(left as u32, Ordering::Relaxed);
    }
    let ours = snapshot(&regs);
    let mut diffs = Vec::new();
    for (k, (a, b)) in theirs.iter().zip(&ours).enumerate() {
        for (o, (x, y)) in a.iter().zip(b).enumerate() {
            if x != y && diffs.len() < 8 {
                diffs.push(format!("{:#x}: {x:#04x} vs {y:#04x}", regs[k].0 + o));
            }
        }
    }
    if !diffs.is_empty() {
        MISMATCHES[i].fetch_add(1, Ordering::Relaxed);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 20 {
            log(&format!("input: {} mismatch (original vs Rust): {}", PORTS[i].0, diffs.join(", ")));
        }
    }
}

// --- Hooks ------------------------------------------------------------------------------

const DO_FRAME: usize = 0;
const KEY_QUEUE_PUSH: usize = 1;
const KEYBOARD_RESET: usize = 2;
const BUTTON_ANALOG: usize = 3;
const KEY_DOWN: usize = 4;
const WNDPROC_KEYS: usize = 5;
const COUNT: usize = 6;

const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("input_do_frame", 0x00C0_63D0, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8]),
    ("input_key_queue_push", 0x00C1_0B10, &[0x84, 0xD2, 0x0F, 0xB6, 0x4C, 0x24, 0x04]),
    ("input_keyboard_reset", 0x00C1_0EB0, &[0xB8, 0x80, 0x8B, 0x34, 0x02]),
    ("input_button_analog", 0x00C0_6870, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8]),
    ("input_key_down", 0x00C1_11D0, &KEY_DOWN_BODY),
    ("input_wndproc_keys", 0x00C1_0CD0, &[0x80, 0x3D, 0x4E, 0xA5, 0x52, 0x02, 0x00]),
];

/// `input_key_down`'s whole body: its prologue holds a short branch, so the original
/// is relocated rather than trampolined.
const KEY_DOWN_BODY: [u8; 98] = [
    0x83, 0xF8, 0xFF, 0x74, 0x32, 0x3D, 0x00, 0x01, 0x00, 0x00, 0x73, 0x30, 0x8D, 0x04, 0x40, 0x03, 0xC0, 0x03, 0xC0,
    0x80, 0xB8, 0x80, 0x8B, 0x34, 0x02, 0x00, 0x74, 0x18, 0x80, 0xB8, 0x82, 0x8B, 0x34, 0x02, 0x00, 0x75, 0x0F, 0x80,
    0x3D, 0x4F, 0xA5, 0x52, 0x02, 0x00, 0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3, 0x3D, 0x00,
    0x01, 0x00, 0x00, 0x75, 0x06, 0xA0, 0x73, 0xF4, 0x34, 0x02, 0xC3, 0x3D, 0x01, 0x01, 0x00, 0x00, 0x75, 0x06, 0xA0,
    0x83, 0xF4, 0x34, 0x02, 0xC3, 0x3D, 0x02, 0x01, 0x00, 0x00, 0x0F, 0x95, 0xC0, 0x2C, 0x01, 0x22, 0x05, 0x7B, 0xF4,
    0x34, 0x02, 0xC3,
];

static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

extern "C" fn do_frame_hook() {
    CALLS[DO_FRAME].fetch_add(1, Ordering::Relaxed);
    let original = || unsafe { cdecl(orig(DO_FRAME), &[]) };
    with_lock(|| match MODE {
        Mode::Original => original(),
        Mode::Shadow => shadow(DO_FRAME, original, || unsafe { do_frame() }),
        Mode::Rust => unsafe { do_frame() },
    })
}

extern "C" fn wndproc_keys_hook(msg: u32, wparam: u32, lparam: u32) {
    CALLS[WNDPROC_KEYS].fetch_add(1, Ordering::Relaxed);
    let original = || unsafe {
        let f: extern "C" fn(u32, u32, u32) = std::mem::transmute(orig(WNDPROC_KEYS));
        f(msg, wparam, lparam)
    };
    with_lock(|| match MODE {
        Mode::Original => original(),
        Mode::Shadow => shadow(WNDPROC_KEYS, original, || unsafe { wndproc_keys(msg, lparam) }),
        Mode::Rust => unsafe { wndproc_keys(msg, lparam) },
    })
}

#[unsafe(naked)]
extern "C" fn key_queue_push_entry() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push edx",
        "push dword ptr [esp + 16]",
        "call {h}",
        "add esp, 8",
        "pop edx",
        "pop ecx",
        "ret",
        h = sym key_queue_push_hook,
    )
}

extern "C" fn key_queue_push_hook(scan: u32, edx: u32) {
    CALLS[KEY_QUEUE_PUSH].fetch_add(1, Ordering::Relaxed);
    let is_down = edx as u8;
    let original = || unsafe {
        let f = orig(KEY_QUEUE_PUSH);
        std::arch::asm!(
            "push {scan}",
            "call {f}",
            "add esp, 4",
            f = in(reg) f, scan = in(reg) scan,
            in("edx") edx, lateout("eax") _, lateout("ecx") _,
        );
    };
    with_lock(|| match MODE {
        Mode::Original => original(),
        Mode::Shadow => shadow(KEY_QUEUE_PUSH, original, || unsafe { key_queue_push(scan, is_down) }),
        Mode::Rust => unsafe { key_queue_push(scan, is_down) },
    })
}

extern "C" fn keyboard_reset_hook() {
    CALLS[KEYBOARD_RESET].fetch_add(1, Ordering::Relaxed);
    let original = || unsafe { cdecl(orig(KEYBOARD_RESET), &[]) };
    with_lock(|| match MODE {
        Mode::Original => original(),
        Mode::Shadow => shadow(KEYBOARD_RESET, original, || unsafe { keyboard_reset() }),
        Mode::Rust => unsafe { keyboard_reset() },
    })
}

#[unsafe(naked)]
extern "C" fn button_analog_entry() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push eax",
        "call {h}",
        "add esp, 4",
        "pop edx",
        "pop ecx",
        "ret",
        h = sym button_analog_hook,
    )
}

/// Pure: compare the two results (bits), return the Rust one unless in Original mode.
extern "C" fn button_analog_hook(b: u32) -> f32 {
    CALLS[BUTTON_ANALOG].fetch_add(1, Ordering::Relaxed);
    let o = if MODE == Mode::Rust {
        None
    } else {
        let r: f32;
        unsafe {
            let f = orig(BUTTON_ANALOG);
            let mut out = 0f32;
            std::arch::asm!(
                "call {f}",
                "fstp dword ptr [{out}]",
                f = in(reg) f, out = in(reg) &mut out,
                inout("eax") b => _, clobber_abi("C"),
            );
            r = out;
        }
        Some(r)
    };
    if MODE == Mode::Original {
        return o.unwrap();
    }
    let r = unsafe { button_analog(b as usize) };
    if let Some(o) = o
        && o.to_bits() != r.to_bits()
    {
        MISMATCHES[BUTTON_ANALOG].fetch_add(1, Ordering::Relaxed);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 20 {
            log(&format!("input: input_button_analog mismatch ({b:#x}): original {o}, Rust {r}"));
        }
    }
    r
}

#[unsafe(naked)]
extern "C" fn key_down_entry() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push eax",
        "call {h}",
        "add esp, 4",
        "pop edx",
        "pop ecx",
        "ret",
        h = sym key_down_hook,
    )
}

/// Pure: compare EAX, return the Rust one unless in Original mode.
extern "C" fn key_down_hook(key: u32) -> u32 {
    CALLS[KEY_DOWN].fetch_add(1, Ordering::Relaxed);
    let o = (MODE != Mode::Rust).then(|| {
        let r: u32;
        unsafe {
            std::arch::asm!(
                "call {f}",
                f = in(reg) orig(KEY_DOWN),
                inout("eax") key => r, lateout("ecx") _, lateout("edx") _,
            );
        }
        r
    });
    if MODE == Mode::Original {
        return o.unwrap();
    }
    let r = unsafe { key_down(key) };
    if let Some(o) = o
        && o != r
    {
        MISMATCHES[KEY_DOWN].fetch_add(1, Ordering::Relaxed);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 20 {
            log(&format!("input: input_key_down mismatch ({key:#x}): original {o:#x}, Rust {r:#x}"));
        }
    }
    r
}

fn hook_address(i: usize) -> usize {
    match i {
        DO_FRAME => do_frame_hook as extern "C" fn() as usize,
        KEY_QUEUE_PUSH => key_queue_push_entry as extern "C" fn() as usize,
        KEYBOARD_RESET => keyboard_reset_hook as extern "C" fn() as usize,
        BUTTON_ANALOG => button_analog_entry as extern "C" fn() as usize,
        KEY_DOWN => key_down_entry as extern "C" fn() as usize,
        WNDPROC_KEYS => wndproc_keys_hook as extern "C" fn(u32, u32, u32) as usize,
        _ => unreachable!(),
    }
}

pub fn install() {
    // The clock wrapper first: the shadow replay depends on it.
    unsafe {
        let mut old = 0u32;
        if VirtualProtect(QPC_SLOT as *const _, 4, PAGE_READWRITE, &mut old) != 0 {
            QPC_REAL.store(rd(QPC_SLOT) as usize, Ordering::Release);
            wr(QPC_SLOT, qpc_hook as extern "system" fn(*mut i64) -> i32 as usize as u32);
            VirtualProtect(QPC_SLOT as *const _, 4, old, &mut old);
        } else {
            log("input: could not wrap QueryPerformanceCounter; not hooking input");
            return;
        }
    }
    let mut ok = 0;
    for (i, &(name, addr, bytes)) in PORTS.iter().enumerate() {
        let r = unsafe {
            if i == KEY_DOWN {
                detour::relocate(addr, bytes, &[]).and_then(|copy| {
                    detour::replace(addr, bytes, hook_address(i))?;
                    Ok(copy)
                })
            } else {
                detour::install(addr, bytes, hook_address(i))
            }
        };
        match r {
            Ok(original) => {
                ORIG[i].store(original, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("input: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    let mode = match MODE {
        Mode::Original => "original",
        Mode::Shadow => "Rust, shadow-checked against the original",
        Mode::Rust => "Rust",
    };
    log(&format!("replaced {ok}/{COUNT} input functions ({mode})"));
}

pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .enumerate()
        .filter_map(|(i, &(name, ..))| {
            let c = CALLS[i].load(Ordering::Relaxed);
            (c > 0).then(|| format!("{name} {c} ({} mismatched)", MISMATCHES[i].load(Ordering::Relaxed)))
        })
        .collect();
    format!(
        "input live: {} | clock underflow {}, leftover {} | x87 precision {}",
        parts.join(", "),
        CLOCK_UNDERFLOW.load(Ordering::Relaxed),
        CLOCK_LEFTOVER.load(Ordering::Relaxed),
        x87_precision()
    )
}
