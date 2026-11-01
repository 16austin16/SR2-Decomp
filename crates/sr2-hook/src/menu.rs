//! In-game menu (F7): player options and the mission logger's controls.
//!
//! - Keys come from the input layer (input.rs): after `input_do_frame` runs, `input`
//!   reads the keys pressed this frame, and while the menu is open it clears the
//!   pad buttons and sticks so the game does not act on them.
//! - The menu draws from the game thread right after the input update, so its calls
//!   go into the frame's deferred list like the HUD's: `gfx_set_color`,
//!   `im2d_fill_rect` and `gfx_draw_text` with the HUD's state pair, in a 640 x 480
//!   space the game stretches over the screen.
//! - Player actions run on the game thread from `game_frame_tick` (`tick`).
//!
//! F8 marks a point in the mission log without opening the menu.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::gfx_cmd::call_regs;
use crate::missionlog;

// Keys: 256 x 12 bytes {u8 down, u8 pressed this frame, ...} by scan code (extended = +0x80).
const KEYS: usize = 0x0234_8B80;
const KEY_MENU: usize = 0x41; // F7
const KEY_MARK: usize = 0x42; // F8
const KEY_UP: usize = 0xC8;
const KEY_DOWN: usize = 0xD0;
const KEY_LEFT: usize = 0xCB;
const KEY_RIGHT: usize = 0xCD;
const KEY_ENTER: usize = 0x1C;
const KEY_BACK: usize = 0x0E;
const BUTTONS: usize = 0x0234_78B8; // 87 x 0x24
const STICKS: usize = 0x0234_84F4; // 8 x 0x40

// Drawing.
const SET_COLOR: usize = 0x00D1_4840; // cdecl (r, g, b, a) 0..255
const FILL_RECT: usize = 0x00D0_B980; // cdecl (x, y, w, h, &pair)
const DRAW_TEXT: usize = 0x00D1_5C60; // ESI = text, EDI = &pair, cdecl (f32 x, f32 y, f32 scale, font)
const SET_TINT: usize = 0x00D1_4700; // ESI = &{f32 r, g, b}: pixel constant c4, which the 2D shaders multiply by
const FONT_LOOKUP: usize = 0x00C1_5CF0; // EAX = font id -> record (+0x14 s16 line height)
const HUD_PAIR: u32 = 0x00EC_2740; // the state pair the HUD widgets draw with
// Font ids the cash widget draws with: 0xE98A80 is its outline (black glyphs, drawn
// first), 0xE98A7C the fill, which takes the colour.
const HUD_FONT: usize = 0x00E9_8A7C;

// Player.
const LOCAL_PLAYER: usize = 0x0217_03D4;
const SET_INVULNERABLE: usize = 0x0096_5F40; // cdecl (human, bool)
const SET_HIT_POINTS: usize = 0x0096_9C90; // ECX = human, (&f32)
const ADJUST_CASH: usize = 0x0096_A430; // ECX = human, (&cents delta)
/// `notoriety_set` as the `notoriety_set` script call uses it: cdecl (team 0-3, f32 level).
const NOTORIETY_SET: usize = 0x0055_F870;
/// Advances the clock (what `set_time_of_day` calls): cdecl (f32 seconds, 1, 0, 1).
const ADVANCE_TIME: usize = 0x0068_A1E0;
const CLOCK_HOUR: usize = 0x0140_E72C; // u8, then u8 minute
const INVENTORY: usize = 0x10C4; // -> circular list: item +0xB4 next, +0xC4 weapon instance
const WEAPON_INFO: usize = 0x08; // in the weapon instance
const WEAPON_CLIP: usize = 0x10; // i32 rounds in the magazine
const FORCE_FLAGS: usize = 0x110C; // bit 0x40 = invulnerable
const MAX_HP: usize = 0x1120; // i32
const HP: usize = 0x1124; // f32
const RESPECT: usize = 0x1200; // i32
/// The GPS route's destination (vec3): the waypoint set on the pause map. Written by
/// the route function 0x765E40 from its target argument.
const GPS_TARGET: usize = 0x029B_A748;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Player,
    Logging,
}

#[derive(Clone, Copy)]
enum Action {
    Heal,
    Cash,
    Respect,
    ClearWanted,
    God(bool),
    TeleportWaypoint,
    /// Hours forward (1..23).
    Time(u32),
}

struct State {
    open: bool,
    page: Page,
    cursor: usize,
    god: bool,
    ammo: bool,
    pending: Vec<Action>,
    /// The last thing done, shown at the bottom.
    status: String,
}

static STATE: Mutex<State> = Mutex::new(State {
    open: false,
    page: Page::Main,
    cursor: 0,
    god: false,
    ammo: false,
    pending: Vec::new(),
    status: String::new(),
});
static OPEN: AtomicBool = AtomicBool::new(false);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}

/// Pressed this frame (and down, so a stale flag cannot open the menu).
fn pressed(scan: usize) -> bool {
    let k = KEYS + scan * 12;
    unsafe { *((k + 1) as *const u8) != 0 && *(k as *const u8) != 0 }
}

fn items(s: &State) -> Vec<String> {
    match s.page {
        Page::Main => vec!["Player options".into(), "Mission logging".into(), "Close".into()],
        Page::Player => vec![
            format!("God mode: {}", if s.god { "ON" } else { "off" }),
            format!("Infinite ammo: {}", if s.ammo { "ON" } else { "off" }),
            "Refill health".into(),
            "Add $10,000".into(),
            "Add 5,000 respect".into(),
            "Clear wanted level".into(),
            "Teleport to waypoint".into(),
            format!("Time: < {} >", clock()),
            "Back".into(),
        ],
        Page::Logging => {
            let l = missionlog::status();
            vec![
                format!("Logging: {}", if l.on { "ON" } else { "off" }),
                format!("Mission: < {} >", l.mission_label),
                format!("Mark point (next: {})   [F8]", l.point + 1),
                "Back".into(),
            ]
        }
    }
}

/// Called on the game thread right after `input_do_frame`: keys, then the menu's
/// draw calls, which go into the frame's deferred list like the HUD's.
pub fn input() {
    keys();
    draw();
}

fn keys() {
    if pressed(KEY_MARK) {
        missionlog::mark();
        lock().status = format!("marked point {}", missionlog::status().point);
    }
    let mut s = lock();
    if pressed(KEY_MENU) {
        s.open = !s.open;
        s.page = Page::Main;
        s.cursor = 0;
    }
    OPEN.store(s.open, Ordering::Release);
    if !s.open {
        return;
    }
    block_game_input();
    let n = items(&s).len();
    if pressed(KEY_UP) {
        s.cursor = (s.cursor + n - 1) % n;
    }
    if pressed(KEY_DOWN) {
        s.cursor = (s.cursor + 1) % n;
    }
    if pressed(KEY_BACK) {
        if s.page == Page::Main {
            s.open = false;
        } else {
            s.page = Page::Main;
            s.cursor = 0;
        }
    }
    let (left, right, enter) = (pressed(KEY_LEFT), pressed(KEY_RIGHT), pressed(KEY_ENTER));
    match s.page {
        Page::Main if enter => match s.cursor {
            0 => (s.page, s.cursor) = (Page::Player, 0),
            1 => (s.page, s.cursor) = (Page::Logging, 0),
            _ => s.open = false,
        },
        Page::Player if s.cursor == 7 && (left || right || enter) => {
            s.pending.push(Action::Time(if left { 23 } else { 1 }));
        }
        Page::Player if enter => {
            let a = match s.cursor {
                0 => {
                    s.god = !s.god;
                    Some(Action::God(s.god))
                }
                1 => {
                    s.ammo = !s.ammo;
                    s.status = format!("infinite ammo {}", if s.ammo { "on" } else { "off" });
                    None
                }
                2 => Some(Action::Heal),
                3 => Some(Action::Cash),
                4 => Some(Action::Respect),
                5 => Some(Action::ClearWanted),
                6 => Some(Action::TeleportWaypoint),
                _ => {
                    (s.page, s.cursor) = (Page::Main, 0);
                    None
                }
            };
            if let Some(a) = a {
                s.pending.push(a);
            }
        }
        Page::Logging => match s.cursor {
            0 if enter => missionlog::set_on(!missionlog::status().on),
            1 if left => missionlog::pick_mission(-1),
            1 if right || enter => missionlog::pick_mission(1),
            2 if enter => {
                missionlog::mark();
                s.status = format!("marked point {}", missionlog::status().point);
            }
            3 if enter => (s.page, s.cursor) = (Page::Main, 0),
            _ => {}
        },
        _ => {}
    }
    OPEN.store(s.open, Ordering::Release);
}

/// The game sees no buttons or sticks while the menu is open.
fn block_game_input() {
    unsafe {
        for i in 0..87 {
            let b = BUTTONS + i * 0x24;
            *(b as *mut f32) = 0.0;
            for k in 0x1C..0x20 {
                *((b + k) as *mut u8) = 0;
            }
        }
        for i in 0..8 {
            let s = STICKS + i * 0x40;
            *(s as *mut f32) = 0.0;
            *((s + 4) as *mut f32) = 0.0;
        }
    }
}

/// Called on the game thread before each `game_frame_tick`.
pub fn tick() {
    let (actions, god, ammo) = {
        let mut s = lock();
        (std::mem::take(&mut s.pending), s.god, s.ammo)
    };
    log_memory();
    let p = unsafe { rd(LOCAL_PLAYER) } as usize;
    if p == 0 {
        return;
    }
    let mut msg = None;
    for a in actions {
        unsafe {
            match a {
                Action::God(on) => {
                    call_regs(SET_INVULNERABLE, 0, 0, 0, 0, 0, &[p as u32, on as u32]);
                    msg = Some(format!("god mode {}", if on { "on" } else { "off" }));
                }
                Action::Heal => {
                    heal(p);
                    msg = Some("health refilled".into());
                }
                Action::Cash => {
                    let cents: i32 = 10_000 * 100;
                    call_regs(ADJUST_CASH, 0, p as u32, 0, 0, 0, &[&cents as *const i32 as u32]);
                    msg = Some("added $10,000".into());
                }
                Action::Respect => {
                    let r = (p + RESPECT) as *mut i32;
                    *r = (*r).saturating_add(5000);
                    msg = Some(format!("respect now {}", *r));
                }
                Action::ClearWanted => {
                    for team in 0..4u32 {
                        call_regs(NOTORIETY_SET, 0, 0, 0, 0, 0, &[team, 0.0f32.to_bits()]);
                    }
                    msg = Some("wanted level cleared".into());
                }
                Action::TeleportWaypoint => {
                    let t = *(GPS_TARGET as *const [f32; 3]);
                    msg = Some(if t == [0.0; 3] {
                        "no waypoint set".into()
                    } else {
                        // A little above it: the teleport puts the player down on the ground.
                        let to = [t[0], t[1] + 2.0, t[2]];
                        let how = crate::teleport::teleport(p, &to);
                        format!("teleported to {:.0}, {:.0}, {:.0} ({how})", t[0], t[1], t[2])
                    });
                }
                Action::Time(hours) => {
                    let secs = hours as f32 * 3600.0;
                    call_regs(ADVANCE_TIME, 0, 0, 0, 0, 0, &[secs.to_bits(), 1, 0, 1]);
                    msg = Some(format!("time now {}", clock()));
                }
            }
        }
    }
    if god {
        unsafe {
            // Scripts can clear the flag (cutscenes, mission starts): set it again.
            if rd(p + FORCE_FLAGS) & 0x40 == 0 {
                call_regs(SET_INVULNERABLE, 0, 0, 0, 0, 0, &[p as u32, 1]);
            }
            if (*((p + HP) as *const f32)) < (rd(p + MAX_HP) as i32 as f32) {
                heal(p);
            }
        }
    }
    full_magazines(p, ammo);
    if let Some(m) = msg {
        crate::log(&format!("menu: {m}"));
        lock().status = m;
    }
}

fn clock() -> String {
    let (h, m) = unsafe { (*(CLOCK_HOUR as *const u8), *((CLOCK_HOUR + 1) as *const u8)) };
    format!("{h:02}:{m:02}")
}

/// The most each weapon's magazine has held, by (weapon instance, weapon info).
static MAGAZINES: Mutex<BTreeMap<(usize, usize), i32>> = Mutex::new(BTreeMap::new());

/// Infinite ammo: no weapon's magazine goes below the most it has held, so it
/// never runs out or needs reloading.
fn full_magazines(p: usize, on: bool) {
    let mut seen = MAGAZINES.lock().unwrap_or_else(|e| e.into_inner());
    if !on {
        seen.clear();
        return;
    }
    let mut current = Vec::new();
    unsafe {
        let head = rd(p + INVENTORY) as usize;
        let mut item = head;
        for _ in 0..64 {
            if item == 0 {
                break;
            }
            let w = rd(item + 0xC4) as usize;
            if w != 0 {
                let key = (w, rd(w + WEAPON_INFO) as usize);
                current.push(key);
                let clip = (w + WEAPON_CLIP) as *mut i32;
                let most = seen.entry(key).or_insert(*clip);
                if *clip > *most {
                    *most = *clip;
                } else if *clip < *most {
                    *clip = *most;
                }
            }
            item = rd(item + 0xB4) as usize;
            if item == head {
                break;
            }
        }
    }
    // Forget weapons no longer carried (their instances are freed and reused).
    seen.retain(|k, _| current.contains(k));
}

static MEMORY_FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Address space (in use, total) in MB: 2 GB total unless the exe is large-address-aware.
fn memory() -> (u64, u64) {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return (0, 0);
    }
    ((m.ullTotalVirtual - m.ullAvailVirtual) >> 20, m.ullTotalVirtual >> 20)
}

/// The largest free run of address space, MB: low means fragmented (a big texture
/// create can fail with plenty free in total).
fn largest_free_mb() -> usize {
    use windows_sys::Win32::System::Memory::{MEM_FREE, MEMORY_BASIC_INFORMATION, VirtualQuery};
    let (mut addr, mut best) = (0usize, 0usize);
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
    while unsafe { VirtualQuery(addr as *const _, &mut mbi, size) } == size {
        if mbi.State == MEM_FREE {
            best = best.max(mbi.RegionSize);
        }
        let next = mbi.BaseAddress as usize + mbi.RegionSize;
        if next <= addr {
            break;
        }
        addr = next;
    }
    best >> 20
}

fn log_memory() {
    if MEMORY_FRAMES.fetch_add(1, Ordering::Relaxed) % 3600 == 0 {
        let (used, total) = memory();
        crate::log(&format!(
            "memory: {used} of {total} MB of address space in use, largest free block {} MB",
            largest_free_mb()
        ));
    }
}

unsafe fn heal(p: usize) {
    unsafe {
        let max = rd(p + MAX_HP) as i32 as f32;
        call_regs(SET_HIT_POINTS, 0, p as u32, 0, 0, 0, &[&max as *const f32 as u32]);
    }
}

// --- Drawing (game thread, deferred) --------------------------------------------------------------

fn color(r: u32, g: u32, b: u32, a: u32) {
    unsafe { call_regs(SET_COLOR, 0, 0, 0, 0, 0, &[r, g, b, a]) };
}

fn rect(x: i32, y: i32, w: i32, h: i32) {
    unsafe { call_regs(FILL_RECT, 0, 0, 0, 0, 0, &[x as u32, y as u32, w as u32, h as u32, HUD_PAIR]) };
}

fn text(x: f32, y: f32, scale: f32, s: &str) {
    let mut z: Vec<u8> = s.bytes().filter(|&c| c >= 0x20 && c < 0x7F).collect();
    z.push(0);
    let font = unsafe { rd(HUD_FONT) };
    unsafe {
        call_regs(DRAW_TEXT, 0, 0, 0, z.as_ptr() as u32, HUD_PAIR, &[x.to_bits(), y.to_bits(), scale.to_bits(), font])
    };
}

const SCREEN_W: usize = 0x022F_DC1C; // i32
const SCREEN_H: usize = 0x022F_DC20; // i32
const RESET_VIEWPORT: usize = 0x00D1_4D90; // the viewport to the whole target
const SET_VIEWPORT: usize = 0x00D1_49B0; // EDI = y, cdecl (x, w, h, no_clamp)
const SET_SCISSOR: usize = 0x00D1_4D20; // EDI = left, ESI = right, cdecl (top, bottom)
/// The 2D calls draw in the HUD's design space stretched over the screen: 640 x 480
/// at 4:3, 1280 x 720 in widescreen (the same aspect 1.45 cut the game uses to pick
/// its UI layouts). The menu is laid out in 640 x 480 units and scaled to that space,
/// so it keeps its size on screen at any resolution.
fn virtual_size(sw: i32, sh: i32) -> (f32, f32) {
    if sw as f32 / sh as f32 > 1.45 { (1280.0, 720.0) } else { (640.0, 480.0) }
}
const TEXT_SCALE: f32 = 0.3;

fn font_height() -> f32 {
    let f = (unsafe { call_regs(FONT_LOOKUP, rd(HUD_FONT), 0, 0, 0, 0, &[]) }) as usize;
    if f == 0 { 32.0 } else { (unsafe { *((f + 0x14) as *const i16) }) as f32 }
}

/// Queues the menu into the frame's deferred list (game thread). Drawn from the render
/// thread after the list instead, its text came out black. The viewport and scissor
/// go to the whole screen while the menu draws, then back.
fn draw() {
    if !OPEN.load(Ordering::Acquire) {
        return;
    }
    let (title, list, cursor, status) = {
        let s = lock();
        let title = match s.page {
            Page::Main => "SR2 Decomp menu",
            Page::Player => "Player options",
            Page::Logging => "Mission logging",
        };
        (title, items(&s), s.cursor, s.status.clone())
    };
    let l = missionlog::status();
    let (sw, sh) = unsafe { (rd(SCREEN_W) as i32, rd(SCREEN_H) as i32) };
    if sw <= 0 || sh <= 0 {
        return;
    }
    let (vw, vh) = virtual_size(sw, sh);
    let u = vh / 480.0;
    let ts = TEXT_SCALE * u;
    let lh = font_height() * ts + 2.0 * u;
    let pad = 6.0 * u;
    let w = 220.0 * u;
    let h = lh * (list.len() as f32 + 3.4) + pad * 2.0;
    let x = vw - w - 12.0 * u;
    let y = vh - h - 12.0 * u;
    let r = |x: f32, y: f32, w: f32, h: f32| rect(x as i32, y as i32, w as i32, h as i32);
    let ctx = crate::gfx_cmd::thread_context() as usize;
    let saved: [i32; 10] = std::array::from_fn(|i| unsafe { rd(ctx + if i < 6 { 8 + i * 4 } else { 0x44 + (i - 6) * 4 }) } as i32);
    unsafe {
        call_regs(RESET_VIEWPORT, 0, 0, 0, 0, 0, &[]);
        call_regs(SET_SCISSOR, 0, 0, 0, sw as u32, 0, &[0, sh as u32]);
        // The list can leave a dark tint behind (fades): draw with none.
        let white = [1.0f32; 3];
        call_regs(SET_TINT, 0, 0, 0, white.as_ptr() as u32, 0, &[]);
        color(0, 0, 0, 200);
        r(x, y, w, h);
        color(60, 140, 255, 255);
        r(x, y, w, 2.0 * u);
        let tx = x + pad;
        let mut ty = y + pad;
        color(255, 255, 255, 255);
        text(tx, ty, ts, title);
        ty += lh * 1.3;
        for (i, item) in list.iter().enumerate() {
            if i == cursor {
                color(60, 140, 255, 140);
                r(x + pad * 0.5, ty - u, w - pad, lh);
                color(255, 255, 255, 255);
            } else {
                color(190, 190, 190, 255);
            }
            text(tx, ty, ts, item);
            ty += lh;
        }
        ty += lh * 0.3;
        color(150, 150, 150, 255);
        text(tx, ty, ts * 0.8, &format!("log: {} | point {} | {}", l.mission_label, l.point, l.samples_text));
        ty += lh * 0.8;
        text(tx, ty, ts * 0.8, &format!("{status}   [mem {} / {} MB]", memory().0, memory().1));
        color(255, 255, 255, 255);
        let [x0, _x1, y0, _y1, vw, vh, l, rr, t, b] = saved;
        call_regs(SET_VIEWPORT, 0, 0, 0, 0, y0 as u32, &[x0 as u32, vw as u32, vh as u32, 1]);
        call_regs(SET_SCISSOR, 0, 0, 0, rr as u32, l as u32, &[t as u32, b as u32]);
    }
}
