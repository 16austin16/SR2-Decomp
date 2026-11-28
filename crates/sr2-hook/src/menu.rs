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
//! F8 opens the label box: gameplay is frozen (the game's pause counter is raised, no pause
//! menu), the player types a label, Enter marks a point in the mission log with it and
//! Esc cancels. The F7 menu's "Mark point" item opens the same box.

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
const FORCE_FLAGS: usize = 0x110C; // bit 0x40 = invulnerable
const MAX_HP: usize = 0x1120; // i32
const HP: usize = 0x1124; // f32
const RESPECT: usize = 0x1200; // i32
/// The GPS route's destination (vec3): the waypoint set on the pause map. Written by
/// the route function 0x765E40 from its target argument.
const GPS_TARGET: usize = 0x029B_A748;

// Game flags and cheats (docs/analysis/38_cheats_and_menu_hooks.md).
const CHEAT_FLAGS: usize = 0x027A_6A40; // u32: 0x40 infinite ammo, 0x1000 unlimited clip
// Only Infinite_Ammo: with Unlimited Clip too, dual-wielded pistols fire slowly (no reload
// rhythm), so the clip still empties and reloads from the endless reserve.
const AMMO_FLAGS: u32 = 0x40;
// 0x68-byte records: +0 stays on (else one-shot), +1 active, +0xC u16 *display name
// (localized), +0x10 char *phone number.
const CHEATS: usize = 0x027A_6A68;
const CHEAT_COUNT: usize = 0x0252_7B5C;
const CHEAT_ON: usize = 0x0068_62F0; // ESI = record, (1), callee pops
const CHEAT_OFF: usize = 0x0068_63C0; // ESI = record, (1, 0), callee pops
const WEATHERS: usize = 0x0252_6DD0; // -> 0xD8-byte records, +0 char *name
const WEATHER_COUNT: usize = 0x0252_6DCC;
const WEATHER_SET: usize = 0x004D_EC00; // EAX = index, CL = 1
const WEATHER_FORCED: usize = 0x0252_6E14; // 0 = the normal weather cycle
// The label box.
/// The game's pause counter (> 0: `sim_frame` runs only its always-on parts and the player
/// controls are skipped). The pause menu raises it through 0x699840 (which also pauses
/// audio and the game clock); the box raises just the counter.
const PAUSE_COUNT: usize = 0x0252_7C08;
const KEY_ESC: usize = 0x01;
const KEY_NUM_ENTER: usize = 0x9C;
const KEY_LSHIFT: usize = 0x2A;
const KEY_RSHIFT: usize = 0x36;
const LABEL_MAX: usize = 60;
/// Characters by scan code (US layout), 0 = prints nothing.
const CHARS: &[u8; 58] = b"\0\01234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0\0\0 ";
const CHARS_SHIFT: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0\0\0 ";

struct LabelBox {
    open: bool,
    text: String,
}

static LABEL: Mutex<LabelBox> = Mutex::new(LabelBox { open: false, text: String::new() });
/// We raised the pause counter and have not lowered it yet.
static PAUSED_BY_BOX: AtomicBool = AtomicBool::new(false);
/// Set when the label box closes; cleared once Esc and Enter are both up.
static SWALLOW_KEYS: AtomicBool = AtomicBool::new(false);

fn label_lock() -> std::sync::MutexGuard<'static, LabelBox> {
    LABEL.lock().unwrap_or_else(|p| p.into_inner())
}

fn down(scan: usize) -> bool {
    unsafe { *((KEYS + scan * 12) as *const u8) != 0 }
}

fn open_label_box() {
    let mut l = label_lock();
    if l.open {
        return;
    }
    l.open = true;
    l.text.clear();
    unsafe { *(PAUSE_COUNT as *mut i32) += 1 };
    PAUSED_BY_BOX.store(true, Ordering::Relaxed);
}

/// Lower the counter we raised (only if it is still above zero: a level load may have reset it).
fn release_pause() {
    if PAUSED_BY_BOX.swap(false, Ordering::Relaxed) {
        let c = PAUSE_COUNT as *mut i32;
        unsafe {
            if *c > 0 {
                *c -= 1;
            }
        }
    }
}

/// Closes the box (saving the typed text as a mission-log point when `save`) and swallows
/// Esc / Enter until they are released.
fn close_label_box(save: bool) {
    let mut l = label_lock();
    let text = std::mem::take(&mut l.text);
    l.open = false;
    drop(l);
    release_pause();
    SWALLOW_KEYS.store(true, Ordering::Relaxed);
    if save {
        missionlog::mark_labeled(text.trim());
        let n = missionlog::status().point;
        lock().status = format!("marked point {n}");
    }
}

/// Handles the open box for one frame; true while it is open (nothing else reads keys).
fn label_keys() -> bool {
    // After the box closes, keep the game's input blocked until Esc / Enter are released: the game
    // would otherwise see the still-held key as a fresh press (Esc opened its pause menu).
    if SWALLOW_KEYS.load(Ordering::Relaxed) {
        if down(KEY_ESC) || down(KEY_ENTER) || down(KEY_NUM_ENTER) {
            block_game_input();
            return true;
        }
        SWALLOW_KEYS.store(false, Ordering::Relaxed);
    }
    if !label_lock().open {
        return false;
    }
    block_game_input();
    let enter = pressed(KEY_ENTER) || pressed(KEY_NUM_ENTER);
    let esc = pressed(KEY_ESC);
    let mut l = label_lock();
    if enter || esc {
        drop(l);
        close_label_box(enter);
        return true;
    }
    if pressed(KEY_BACK) {
        l.text.pop();
    }
    let table = if down(KEY_LSHIFT) || down(KEY_RSHIFT) { CHARS_SHIFT } else { CHARS };
    for scan in 2..table.len() {
        let c = table[scan];
        if c != 0 && pressed(scan) && l.text.len() < LABEL_MAX {
            l.text.push(c as char);
        }
    }
    true
}

/// Rows shown at once on long pages.
const WINDOW: usize = 11;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Player,
    World,
    Cheats,
    Logging,
    /// Shop chains.
    Shops,
    /// The shops of the chain `State::shop_chain`.
    ShopList,
}

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Heal,
    Cash,
    Respect,
    ClearWanted,
    /// Police notoriety, 1-5 stars.
    Wanted(u32),
    God(bool),
    TeleportWaypoint,
    /// Teleport to the door of shop i (index into the game's shop array).
    TeleportShop(usize),
    /// Hours forward (1..23).
    Time(u32),
    /// Weather index, or -1 for the normal cycle.
    Weather(i32),
    /// Cheat record index, on.
    Cheat(usize, bool),
}

struct State {
    open: bool,
    page: Page,
    cursor: usize,
    god: bool,
    ammo: bool,
    /// Stars for "Set wanted level".
    wanted: u32,
    /// Chain index on the Shops page, for ShopList.
    shop_chain: usize,
    weather: i32,
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
    wanted: 3,
    shop_chain: 0,
    weather: -1,
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
        Page::Main => {
            vec![
                "Player options".into(),
                "World and cheats".into(),
                "Shops".into(),
                "Mission logging".into(),
                "Close".into(),
            ]
        }
        Page::World => vec![
            format!("Weather: < {} >", if s.weather < 0 { "normal".into() } else { weather_name(s.weather as usize) }),
            "Game cheats...".into(),
            "Back".into(),
        ],
        Page::Cheats => {
            let mut v: Vec<String> = (0..cheat_count())
                .map(|i| {
                    let tag = if !cheat_toggles(i) {
                        "   (once)"
                    } else if cheat_active(i) {
                        "   [ON]"
                    } else {
                        ""
                    };
                    format!("{}{tag}", cheat_name(i))
                })
                .collect();
            v.push("Back".into());
            v
        }
        Page::Player => vec![
            format!("God mode: {}", if s.god { "ON" } else { "off" }),
            format!("Infinite ammo: {}", if s.ammo { "ON" } else { "off" }),
            "Refill health".into(),
            "Add $10,000".into(),
            "Add 5,000 respect".into(),
            "Clear wanted level".into(),
            format!("Set wanted level: < {} star{} >", s.wanted, if s.wanted == 1 { "" } else { "s" }),
            "Teleport to waypoint".into(),
            format!("Time: < {} >", clock()),
            "Back".into(),
        ],
        Page::Shops => {
            let g = crate::shops_menu::chains();
            let mut v: Vec<String> = match g.as_ref() {
                Some(c) => c.iter().map(|c| format!("{}  ({})", c.name, c.shops.len())).collect(),
                None => vec!["(shops not loaded yet)".into()],
            };
            v.push("Back".into());
            v
        }
        Page::ShopList => {
            let g = crate::shops_menu::chains();
            let mut v: Vec<String> = match g.as_ref().and_then(|c| c.get(s.shop_chain)) {
                Some(c) => c
                    .shops
                    .iter()
                    .map(|(i, n)| {
                        let d = crate::shops_menu::distance_text(*i);
                        if d.is_empty() { n.clone() } else { format!("{n}   ({d})") }
                    })
                    .collect(),
                None => Vec::new(),
            };
            v.push("Back".into());
            v
        }
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
    if crate::overlay::active() {
        overlay_input();
        return;
    }
    keys();
    draw();
}

fn keys() {
    if label_keys() {
        return;
    }
    if pressed(KEY_MARK) {
        open_label_box();
        return;
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
        match s.page {
            Page::Main => s.open = false,
            Page::Cheats => (s.page, s.cursor) = (Page::World, 1),
            Page::ShopList => (s.page, s.cursor) = (Page::Shops, s.shop_chain),
            _ => (s.page, s.cursor) = (Page::Main, 0),
        }
    }
    let (left, right, enter) = (pressed(KEY_LEFT), pressed(KEY_RIGHT), pressed(KEY_ENTER));
    match s.page {
        Page::Main if enter => match s.cursor {
            0 => (s.page, s.cursor) = (Page::Player, 0),
            1 => (s.page, s.cursor) = (Page::World, 0),
            2 => (s.page, s.cursor) = (Page::Shops, 0),
            3 => (s.page, s.cursor) = (Page::Logging, 0),
            _ => s.open = false,
        },
        Page::World => match s.cursor {
            0 if left || right => {
                let n = weather_count() as i32;
                let d = if left { -1 } else { 1 };
                s.weather = (s.weather + 1 + d).rem_euclid(n + 1) - 1;
            }
            0 if enter => {
                let w = s.weather;
                s.pending.push(Action::Weather(w));
            }
            1 if enter => (s.page, s.cursor) = (Page::Cheats, 0),
            2 if enter => (s.page, s.cursor) = (Page::Main, 0),
            _ => {}
        },
        Page::Cheats if enter => {
            let i = s.cursor;
            if i >= cheat_count() {
                (s.page, s.cursor) = (Page::World, 1);
            } else {
                s.pending.push(Action::Cheat(i, !cheat_active(i)));
            }
        }
        Page::Player if s.cursor == 6 && (left || right) => {
            s.wanted = if left { if s.wanted <= 1 { 5 } else { s.wanted - 1 } } else if s.wanted >= 5 { 1 } else { s.wanted + 1 };
        }
        Page::Player if s.cursor == 8 && (left || right || enter) => {
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
                6 => Some(Action::Wanted(s.wanted)),
                7 => Some(Action::TeleportWaypoint),
                _ => {
                    (s.page, s.cursor) = (Page::Main, 0);
                    None
                }
            };
            if let Some(a) = a {
                s.pending.push(a);
            }
        }
        Page::Shops if enter => {
            let n = crate::shops_menu::chains().as_ref().map_or(0, |c| c.len());
            if s.cursor >= n {
                (s.page, s.cursor) = (Page::Main, 2);
            } else {
                s.shop_chain = s.cursor;
                (s.page, s.cursor) = (Page::ShopList, 0);
            }
        }
        Page::ShopList if enter => {
            let pick = crate::shops_menu::chains()
                .as_ref()
                .and_then(|c| c.get(s.shop_chain))
                .and_then(|c| c.shops.get(s.cursor).map(|x| x.0));
            match pick {
                Some(i) => s.pending.push(Action::TeleportShop(i)),
                None => (s.page, s.cursor) = (Page::Shops, s.shop_chain),
            }
        }
        Page::Logging => match s.cursor {
            0 if enter => missionlog::set_on(!missionlog::status().on),
            1 if left => missionlog::pick_mission(-1),
            1 if right || enter => missionlog::pick_mission(1),
            2 if enter => open_label_box(),
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
                Action::Wanted(stars) => {
                    // Team 3 is the police (doc 43: notoriety_set's team ids).
                    call_regs(NOTORIETY_SET, 0, 0, 0, 0, 0, &[3, (stars as f32).to_bits()]);
                    msg = Some(format!("police wanted level set to {stars}"));
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
                Action::TeleportShop(i) => {
                    msg = Some(match crate::shops_menu::door(i) {
                        Some(d) => {
                            let to = [d[0], d[1] + 1.0, d[2]];
                            let how = crate::teleport::teleport(p, &to);
                            format!("teleported to the shop door ({how})")
                        }
                        None => "that shop's door is not loaded".into(),
                    });
                }
                Action::Time(hours) => {
                    let secs = hours as f32 * 3600.0;
                    call_regs(ADVANCE_TIME, 0, 0, 0, 0, 0, &[secs.to_bits(), 1, 0, 1]);
                    msg = Some(format!("time now {}", clock()));
                }
                Action::Weather(w) => {
                    if w < 0 {
                        *(WEATHER_FORCED as *mut u32) = 0;
                        msg = Some("weather back to normal".into());
                    } else if (w as usize) < weather_count() {
                        call_regs(WEATHER_SET, w as u32, 1, 0, 0, 0, &[]);
                        msg = Some(format!("weather: {}", weather_name(w as usize)));
                    }
                }
                Action::Cheat(i, on) => {
                    if i < cheat_count() {
                        let rec = (CHEATS + i * 0x68) as u32;
                        if on {
                            call_regs(CHEAT_ON, 0, 0, 0, rec, 0, &[1]);
                        } else {
                            call_regs(CHEAT_OFF, 0, 0, 0, rec, 0, &[1, 0]);
                        }
                        let state = if !cheat_toggles(i) { "done" } else if cheat_active(i) { "on" } else { "off" };
                        msg = Some(format!("cheat {}: {state}", cheat_name(i)));
                    }
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
    ammo_flags(ammo);
    if let Some(m) = msg {
        crate::log(&format!("menu: {m}"));
        lock().status = m;
    }
}

fn clock() -> String {
    let (h, m) = unsafe { (*(CLOCK_HOUR as *const u8), *((CLOCK_HOUR + 1) as *const u8)) };
    format!("{h:02}:{m:02}")
}

static AMMO_SET: AtomicBool = AtomicBool::new(false);

/// Infinite ammo: the game's own Infinite_Ammo (reserve and grenades) and Unlimited
/// Clip flags, set directly so no cheat is recorded. Cleared again only if we set them.
fn ammo_flags(on: bool) {
    let f = CHEAT_FLAGS as *mut u32;
    unsafe {
        if on {
            if *f & AMMO_FLAGS != AMMO_FLAGS {
                *f |= AMMO_FLAGS;
            }
            AMMO_SET.store(true, Ordering::Relaxed);
        } else if AMMO_SET.swap(false, Ordering::Relaxed) {
            *f &= !AMMO_FLAGS;
        }
    }
}

/// A NUL-terminated string at `p` (read safely), or "?".
fn cstr(p: usize) -> String {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut b = [0u8; 48];
    let mut n = 0usize;
    let ok = p != 0
        && unsafe { ReadProcessMemory(GetCurrentProcess(), p as *const _, b.as_mut_ptr().cast(), b.len(), &mut n) }
            != 0;
    if !ok {
        return "?".into();
    }
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into()
}

fn cheat_count() -> usize {
    (unsafe { rd(CHEAT_COUNT) } as usize).min(256)
}

fn cheat_name(i: usize) -> String {
    let r = CHEATS + i * 0x68;
    let name = wstr(unsafe { rd(r + 0xC) } as usize);
    if name.is_empty() || name == "?" { cstr(unsafe { rd(r + 0x10) } as usize) } else { name }
}

fn cheat_toggles(i: usize) -> bool {
    unsafe { *((CHEATS + i * 0x68) as *const u8) != 0 }
}

/// A NUL-terminated UTF-16 string at `p` (read safely; the menu font is ASCII), or "?".
fn wstr(p: usize) -> String {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut b = [0u16; 40];
    let mut n = 0usize;
    let ok = p != 0
        && unsafe { ReadProcessMemory(GetCurrentProcess(), p as *const _, b.as_mut_ptr().cast(), 80, &mut n) } != 0;
    if !ok {
        return "?".into();
    }
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..end])
}

fn cheat_active(i: usize) -> bool {
    unsafe { *((CHEATS + i * 0x68 + 1) as *const u8) != 0 }
}

fn weather_count() -> usize {
    (unsafe { rd(WEATHER_COUNT) } as usize).min(64)
}

fn weather_name(i: usize) -> String {
    let base = unsafe { rd(WEATHERS) } as usize;
    if base == 0 { "?".into() } else { cstr(unsafe { rd(base + i * 0xD8) } as usize) }
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

/// A filled rectangle with rounded corners of radius `rad`, built from one-unit slices at the top and
/// bottom that follow a circle (the 2D layer only fills plain rectangles). `top_only` leaves the bottom
/// corners square.
fn rrect(x: f32, y: f32, w: f32, h: f32, rad: f32, top_only: bool) {
    // Snap to whole units first so the corner slices and the body meet with no seam.
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    let (wi, hi) = (w.round().max(1.0) as i32, h.round().max(1.0) as i32);
    let n = (rad.round() as i32).min(wi / 2).min(hi / 2).max(0);
    let r = n as f32;
    for k in 0..n {
        let dy = r - k as f32 - 0.5;
        let inset = (r - (r * r - dy * dy).max(0.0).sqrt()).round() as i32;
        rect(xi + inset, yi + k, wi - 2 * inset, 1);
        if !top_only {
            rect(xi + inset, yi + hi - 1 - k, wi - 2 * inset, 1);
        }
    }
    let body = if top_only { hi - n } else { hi - 2 * n };
    if body > 0 {
        rect(xi, yi + n, wi, body);
    }
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
    draw_menu();
    draw_label_box();
}

/// The label box: centred, over a dimmed screen, in the same 2D state as the menu.
fn draw_label_box() {
    let (typed, point) = {
        let l = label_lock();
        if !l.open {
            return;
        }
        (l.text.clone(), missionlog::status().point + 1)
    };
    let (sw, sh) = unsafe { (rd(SCREEN_W) as i32, rd(SCREEN_H) as i32) };
    if sw <= 0 || sh <= 0 {
        return;
    }
    let (vw, vh) = virtual_size(sw, sh);
    let u = vh / 480.0;
    let ts = TEXT_SCALE * u;
    let lh = font_height() * ts + 2.0 * u;
    let pad = 8.0 * u;
    let w = 400.0 * u;
    let h = lh * 4.2 + pad * 2.0;
    let x = (vw - w) * 0.5;
    let y = (vh - h) * 0.5;
    let r = |x: f32, y: f32, w: f32, h: f32| rect(x as i32, y as i32, w as i32, h as i32);
    let ctx = crate::gfx_cmd::thread_context() as usize;
    let saved: [i32; 10] = std::array::from_fn(|i| unsafe { rd(ctx + if i < 6 { 8 + i * 4 } else { 0x44 + (i - 6) * 4 }) } as i32);
    let blink = (missionlog::frame() / 30) % 2 == 0;
    unsafe {
        call_regs(RESET_VIEWPORT, 0, 0, 0, 0, 0, &[]);
        call_regs(SET_SCISSOR, 0, 0, 0, sw as u32, 0, &[0, sh as u32]);
        let white = [1.0f32; 3];
        call_regs(SET_TINT, 0, 0, 0, white.as_ptr() as u32, 0, &[]);
        color(0, 0, 0, 110);
        r(0.0, 0.0, vw, vh);
        color(0, 0, 0, 225);
        r(x, y, w, h);
        color(60, 140, 255, 255);
        r(x, y, w, 2.0 * u);
        let tx = x + pad;
        let mut ty = y + pad;
        color(255, 255, 255, 255);
        text(tx, ty, ts, &format!("Mission log point {point}"));
        ty += lh * 1.4;
        color(40, 40, 40, 255);
        r(tx, ty - u, w - pad * 2.0, lh + 2.0 * u);
        color(255, 255, 255, 255);
        text(tx + 3.0 * u, ty, ts, &format!("{typed}{}", if blink { "_" } else { " " }));
        ty += lh * 1.7;
        color(150, 150, 150, 255);
        text(tx, ty, ts * 0.8, "Enter = save, Esc = cancel");
        color(255, 255, 255, 255);
        let [x0, _x1, y0, _y1, vw, vh, l, rr, t, b] = saved;
        call_regs(SET_VIEWPORT, 0, 0, 0, 0, y0 as u32, &[x0 as u32, vw as u32, vh as u32, 1]);
        call_regs(SET_SCISSOR, 0, 0, 0, rr as u32, l as u32, &[t as u32, b as u32]);
    }
}

fn draw_menu() {
    if !OPEN.load(Ordering::Acquire) {
        return;
    }
    let (page, list, cursor, status, chain) = {
        let s = lock();
        (s.page, items(&s), s.cursor, s.status.clone(), s.shop_chain)
    };
    let chain_name = if page == Page::ShopList {
        crate::shops_menu::chains().as_ref().and_then(|c| c.get(chain).map(|c| c.name.clone())).unwrap_or_default()
    } else {
        String::new()
    };
    let (title, crumb): (String, String) = match page {
        Page::Main => ("SR2 Decomp".into(), "test menu".into()),
        Page::Player => ("Player options".into(), "Main > Player".into()),
        Page::World => ("World".into(), "Main > World".into()),
        Page::Cheats => ("Game cheats".into(), "Main > World > Cheats (may affect achievements)".into()),
        Page::Logging => ("Mission logging".into(), "Main > Logging".into()),
        Page::Shops => ("Shops".into(), "Main > Shops: pick a chain".into()),
        Page::ShopList => (chain_name.clone(), format!("Main > Shops > {chain_name}: Enter teleports")),
    };
    let l = missionlog::status();
    let (sw, sh) = unsafe { (rd(SCREEN_W) as i32, rd(SCREEN_H) as i32) };
    if sw <= 0 || sh <= 0 {
        return;
    }
    let (vw, vh) = virtual_size(sw, sh);
    let u = vh / 480.0;
    let ts = TEXT_SCALE * u;
    let small = ts * 0.78;
    let lh = font_height() * ts + 3.0 * u;
    let slh = font_height() * small + 2.0 * u;
    let pad = 8.0 * u;
    let w = 270.0 * u;
    // Long pages show a window of rows around the cursor.
    let start = cursor.saturating_sub(WINDOW / 2).min(list.len().saturating_sub(WINDOW));
    let shown = list.len().min(WINDOW);
    let head = lh * 1.7;
    let rows_h = lh * shown as f32;
    let h = head + slh * 1.4 + rows_h + pad * 1.5 + slh * 3.4;
    let x = vw - w - 16.0 * u;
    let y = (vh - h - 16.0 * u).max(8.0 * u);
    let r = |x: f32, y: f32, w: f32, h: f32| rect(x as i32, y as i32, w.max(1.0) as i32, h.max(1.0) as i32);
    let rad = 10.0 * u;
    let ctx = crate::gfx_cmd::thread_context() as usize;
    let saved: [i32; 10] = std::array::from_fn(|i| unsafe { rd(ctx + if i < 6 { 8 + i * 4 } else { 0x44 + (i - 6) * 4 }) } as i32);
    unsafe {
        call_regs(RESET_VIEWPORT, 0, 0, 0, 0, 0, &[]);
        call_regs(SET_SCISSOR, 0, 0, 0, sw as u32, 0, &[0, sh as u32]);
        // The list can leave a dark tint behind (fades): draw with none.
        let white = [1.0f32; 3];
        call_regs(SET_TINT, 0, 0, 0, white.as_ptr() as u32, 0, &[]);
        // Shadow, body, header band.
        color(0, 0, 0, 80);
        rrect(x + 4.0 * u, y + 5.0 * u, w, h, rad, false);
        color(14, 12, 22, 228);
        rrect(x, y, w, h, rad, false);
        color(112, 40, 176, 245);
        rrect(x, y, w, head, rad, true);
        color(170, 110, 235, 200);
        r(x, y + head - 1.5 * u, w, 1.5 * u);
        color(255, 255, 255, 255);
        text(x + pad, y + (head - lh) * 0.5 + u, ts * 1.12, &title);
        // Breadcrumb.
        let mut ty = y + head + slh * 0.3;
        color(165, 160, 185, 255);
        text(x + pad, ty, small, &crumb);
        ty += slh * 1.1;
        // Rows.
        let rows_top = ty;
        for (i, item) in list.iter().enumerate().skip(start).take(shown) {
            if i == cursor {
                color(112, 40, 176, 170);
                rrect(x + pad * 0.5, ty - u, w - pad * 1.5, lh, 5.0 * u, false);
                color(225, 195, 255, 255);
                rrect(x + pad * 0.5 + 2.0 * u, ty + 2.0 * u, 3.0 * u, lh - 6.0 * u, 1.5 * u, false);
                color(255, 255, 255, 255);
            } else if item == "Back" || item == "Close" {
                color(150, 145, 170, 255);
            } else {
                color(205, 205, 215, 255);
            }
            text(x + pad + 3.0 * u, ty, ts, item);
            ty += lh;
        }
        // Scrollbar for long pages.
        if list.len() > shown && shown > 0 {
            let tx = x + w - pad * 0.6;
            color(255, 255, 255, 28);
            r(tx, rows_top - u, 3.0 * u, rows_h);
            let th = (rows_h * shown as f32 / list.len() as f32).max(6.0 * u);
            let off = (rows_h - th) * start as f32 / (list.len() - shown) as f32;
            color(170, 110, 235, 230);
            r(tx, rows_top - u + off, 3.0 * u, th);
        }
        // Footer: hints, status, log / memory.
        ty += pad * 0.5;
        color(255, 255, 255, 30);
        r(x + pad, ty, w - pad * 2.0, u.max(1.0));
        ty += pad * 0.6;
        color(140, 135, 160, 255);
        text(x + pad, ty, small, "Up/Down move  Left/Right change  Enter select  Bksp back");
        ty += slh;
        if !status.is_empty() {
            color(200, 160, 255, 255);
            text(x + pad, ty, small, &status);
        }
        ty += slh;
        color(110, 105, 130, 255);
        text(x + pad, ty, small, &format!("log: {} | point {} | {} | mem {} / {} MB", l.mission_label, l.point, l.samples_text, memory().0, memory().1));
        color(255, 255, 255, 255);
        let [x0, _x1, y0, _y1, vw, vh, l, rr, t, b] = saved;
        call_regs(SET_VIEWPORT, 0, 0, 0, 0, y0 as u32, &[x0 as u32, vw as u32, vh as u32, 1]);
        call_regs(SET_SCISSOR, 0, 0, 0, rr as u32, l as u32, &[t as u32, b as u32]);
    }
}

// --- ImGui overlay support (src/overlay, opt-in through sr2-imgui.txt) ----------------------------
//
// With the opt-in file present, `input` calls `overlay_input` instead of drawing the HUD menu. The
// game thread keeps what the game thread must do (the F7 / F8 keys, blocking the game's input,
// reading game memory into a snapshot, running the queued actions in `tick`); the overlay's UI,
// drawn on the render thread, reads the snapshot and calls the `ui_*` functions below, which only
// touch the same state the old menu's keys touch.

/// The ImGui menu window is open.
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
/// The showcase mode already opened the window.
static SHOWCASE_DONE: AtomicBool = AtomicBool::new(false);
static OVERLAY_FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static OVERLAY_START: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Game memory read on the game thread for the overlay's pages.
#[derive(Clone, Default)]
pub(crate) struct Snapshot {
    /// (name, toggles, active)
    pub cheats: Vec<(String, bool, bool)>,
    pub weathers: Vec<String>,
    /// (chain name, [(shop index, label with distance)])
    pub chains: Vec<(String, Vec<(usize, String)>)>,
    pub shops_loaded: bool,
    pub clock: String,
    pub log_on: bool,
    pub log_mission: String,
    pub log_point: u32,
    pub log_samples: String,
    pub mem: (u64, u64),
}

static SNAP: Mutex<Option<Snapshot>> = Mutex::new(None);

pub(crate) fn overlay_snapshot() -> Snapshot {
    SNAP.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_default()
}

fn take_snapshot() {
    let l = missionlog::status();
    let (chains, shops_loaded) = match crate::shops_menu::chains().as_ref() {
        Some(c) => (
            c.iter()
                .map(|c| {
                    let shops = c
                        .shops
                        .iter()
                        .map(|(i, n)| {
                            let d = crate::shops_menu::distance_text(*i);
                            (*i, if d.is_empty() { n.clone() } else { format!("{n}   ({d})") })
                        })
                        .collect();
                    (c.name.clone(), shops)
                })
                .collect(),
            true,
        ),
        None => (Vec::new(), false),
    };
    let snap = Snapshot {
        cheats: (0..cheat_count()).map(|i| (cheat_name(i), cheat_toggles(i), cheat_active(i))).collect(),
        weathers: (0..weather_count()).map(weather_name).collect(),
        chains,
        shops_loaded,
        clock: clock(),
        log_on: l.on,
        log_mission: l.mission_label,
        log_point: l.point,
        log_samples: l.samples_text,
        mem: memory(),
    };
    *SNAP.lock().unwrap_or_else(|p| p.into_inner()) = Some(snap);
}

/// The overlay has something to show (the render thread asks every frame).
pub(crate) fn overlay_visible() -> bool {
    OVERLAY_OPEN.load(Ordering::Acquire) || label_lock().open
}

pub(crate) fn overlay_menu_open() -> bool {
    OVERLAY_OPEN.load(Ordering::Acquire)
}

pub(crate) fn overlay_label_open() -> bool {
    label_lock().open
}

/// Frees the Windows cursor: the game warps it back to the window centre every frame while mouse
/// look is on (flag at 0x252A5D5, see input.rs `mouse_update`).
fn free_cursor() {
    unsafe {
        if SAVED_WARP.load(Ordering::Relaxed) < 0 {
            SAVED_WARP.store(*(0x0252_A5D5 as *const u8) as i32, Ordering::Relaxed);
        }
        *(0x0252_A5D5 as *mut u8) = 0;
    }
}

/// The warp flag as it was before the menu freed the cursor (-1: not freed).
static SAVED_WARP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Puts mouse look back once the menu and the label box are closed. The capture flag
/// (0x252A5D6) is cleared too, so the game's first frame back has a zero mouse delta and
/// warps the cursor to the centre instead of turning the camera by the cursor's offset.
fn restore_cursor() {
    let w = SAVED_WARP.swap(-1, Ordering::Relaxed);
    if w >= 0 {
        unsafe {
            // Centre the cursor first (the game's centre: the middle of the window rectangle), so the
            // first mouse delta the game reads is zero.
            use windows_sys::Win32::Foundation::RECT;
            use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetCursorPos};
            let hwnd = *(0x0252_A410 as *const usize) as *mut std::ffi::c_void;
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if !hwnd.is_null() && GetWindowRect(hwnd, &mut r) != 0 {
                SetCursorPos(r.left + (r.right - r.left) / 2, r.top + (r.bottom - r.top) / 2);
            }
            *(0x0252_A5D6 as *mut u8) = 0;
            *(0x0252_A5D5 as *mut u8) = w as u8;
            // The look stick is filled early in the frame from the delta `mouse_update` measured
            // late in the previous one (cursor minus centre, while the menu had the cursor free):
            // clear it, or the camera turns by where the cursor was.
            for a in [0x0234_F458usize, 0x0234_F45C, 0x0234_F464, 0x0234_F468] {
                *(a as *mut u32) = 0;
            }
        }
        // This runs after `input_do_frame`, which already turned that delta into this frame's
        // look stick: block this one frame too.
        block_game_input();
    }
}

/// Game-thread half of the overlay menu: keys, input blocking, the snapshot.
fn overlay_input() {
    crate::overlay::poll();
    OVERLAY_FRAMES.fetch_add(1, Ordering::Relaxed);
    // Showcase mode (unattended screenshot): open once the game has run a while or has a player.
    if crate::overlay::showcase() && !SHOWCASE_DONE.load(Ordering::Relaxed) {
        let start = *OVERLAY_START.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(std::time::Instant::now);
        let in_game = unsafe { rd(LOCAL_PLAYER) } != 0 && OVERLAY_FRAMES.load(Ordering::Relaxed) > 300;
        if in_game || start.elapsed().as_secs() >= 40 {
            SHOWCASE_DONE.store(true, Ordering::Relaxed);
            OVERLAY_OPEN.store(true, Ordering::Release);
            crate::log("overlay: showcase mode opened the menu");
        }
    }
    // Same as the old label box: keep the game's input blocked until Esc / Enter are released.
    if SWALLOW_KEYS.load(Ordering::Relaxed) {
        if down(KEY_ESC) || down(KEY_ENTER) || down(KEY_NUM_ENTER) {
            block_game_input();
            return;
        }
        SWALLOW_KEYS.store(false, Ordering::Relaxed);
    }
    if label_lock().open {
        block_game_input();
        free_cursor();
        take_snapshot();
        return;
    }
    if pressed(KEY_MARK) {
        open_label_box();
        return;
    }
    if pressed(KEY_MENU) {
        SHOWCASE_DONE.store(true, Ordering::Relaxed);
        let open = !OVERLAY_OPEN.load(Ordering::Acquire);
        OVERLAY_OPEN.store(open, Ordering::Release);
        if open {
            OVERLAY_FRAMES.store(0, Ordering::Relaxed);
        }
    }
    if OVERLAY_OPEN.load(Ordering::Acquire) {
        block_game_input();
        free_cursor();
        if OVERLAY_FRAMES.load(Ordering::Relaxed) % 10 == 0 {
            take_snapshot();
        }
    } else if !label_lock().open {
        restore_cursor();
    }
}

/// What the overlay shows without waiting for the next snapshot.
pub(crate) struct Light {
    pub god: bool,
    pub ammo: bool,
    pub wanted: u32,
    pub weather: i32,
    pub status: String,
    pub label_point: u32,
}

pub(crate) fn ui_light() -> Light {
    let s = lock();
    Light {
        god: s.god,
        ammo: s.ammo,
        wanted: s.wanted,
        weather: s.weather,
        status: s.status.clone(),
        label_point: missionlog::status().point + 1,
    }
}

pub(crate) fn ui_action(a: Action) {
    lock().pending.push(a);
}

pub(crate) fn ui_set_god(on: bool) {
    let mut s = lock();
    s.god = on;
    s.pending.push(Action::God(on));
}

pub(crate) fn ui_set_ammo(on: bool) {
    let mut s = lock();
    s.ammo = on;
    s.status = format!("infinite ammo {}", if on { "on" } else { "off" });
}

pub(crate) fn ui_set_wanted(stars: u32) {
    lock().wanted = stars.clamp(1, 5);
}

pub(crate) fn ui_set_weather(w: i32) {
    let mut s = lock();
    s.weather = w;
    s.pending.push(Action::Weather(w));
}

pub(crate) fn ui_open_label_box() {
    open_label_box();
}

/// Closes the label box, saving `text` as the point's label when it is Some.
pub(crate) fn ui_close_label_box(text: Option<String>) {
    let save = text.is_some();
    if let Some(t) = text {
        label_lock().text = t.chars().take(LABEL_MAX).collect();
    }
    close_label_box(save);
}

/// Closes the menu window (Esc / the close button) and swallows Esc until released.
pub(crate) fn ui_close_menu() {
    OVERLAY_OPEN.store(false, Ordering::Release);
    SWALLOW_KEYS.store(true, Ordering::Relaxed);
}
