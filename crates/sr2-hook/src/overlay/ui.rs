//! The ImGui side of the overlay: context, theme, input from the OS, and the F7 menu's pages.
//!
//! Runs on the render thread, inside the Present hook (`mod.rs`). The pages only call the
//! `menu::ui_*` functions, which touch the same state as the old menu's keys, so the actions
//! (run by `menu::tick` on the game thread) behave identically.

use std::sync::Mutex;
use std::time::Instant;

use imgui::{Condition, Context, FontConfig, FontSource, Io, Key, MouseButton, StyleColor, TabItemFlags, TextureId, Ui, WindowFlags};
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, MapVirtualKeyW, ToUnicode};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos, GetForegroundWindow};

use super::d3d9::Backend;
use crate::menu::{self, Action, Light, Snapshot};

/// The game's window handle (0x252A410, as input.rs's mouse update reads it).
const GAME_HWND: usize = 0x0252_A410;
const LABEL_MAX: usize = 60;
/// The game's mouse-wheel accumulator.
const WHEEL_ACC: usize = 0x0234_F460;

const ACCENT: [f32; 4] = [112.0 / 255.0, 40.0 / 255.0, 176.0 / 255.0, 1.0];
const BRIGHT: [f32; 4] = [170.0 / 255.0, 110.0 / 255.0, 235.0 / 255.0, 1.0];

fn rgb(r: u32, g: u32, b: u32, a: f32) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a]
}

/// Keys fed to ImGui for navigation, editing and shortcuts.
const KEYMAP: &[(i32, Key)] = &[
    (0x26, Key::UpArrow),
    (0x28, Key::DownArrow),
    (0x25, Key::LeftArrow),
    (0x27, Key::RightArrow),
    (0x0D, Key::Enter),
    (0x1B, Key::Escape),
    (0x08, Key::Backspace),
    (0x09, Key::Tab),
    (0x2E, Key::Delete),
    (0x24, Key::Home),
    (0x23, Key::End),
    (0x21, Key::PageUp),
    (0x22, Key::PageDown),
    (0x20, Key::Space),
    (0x41, Key::A),
    (0x43, Key::C),
    (0x56, Key::V),
    (0x58, Key::X),
    (0x59, Key::Y),
    (0x5A, Key::Z),
    (0x11, Key::ModCtrl),
    (0x10, Key::ModShift),
];

/// Virtual keys that type characters: space, 0-9, A-Z, the numpad and the OEM punctuation keys.
fn types_text(vk: i32) -> bool {
    matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2)
}

fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

struct Input {
    keys: [bool; KEYMAP.len()],
    mouse: [bool; 2],
    /// Seconds until a held text key repeats; negative = not held.
    hold: [f32; 256],
    esc: bool,
    wheel_last: Option<i32>,
    arrow_raw: [bool; 2],
    /// Pages to move by (Left / Right arrows when no slider is under the mouse or highlighted).
    tab_step: i32,
    /// Enter is also fed as Space, ImGui's activate key, outside text boxes.
    enter_space: bool,
}

/// Set by the page code when a slider is hovered or highlighted: the arrows then go to the slider.
static SLIDER_HOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Left / Right presses for the hot slider, applied by `mark_slider` on the next frame.
static SLIDER_STEP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Call right after a slider: marks it hot when hovered / highlighted, and returns the arrow
/// steps to apply to it (0 when it is not hot).
fn mark_slider(ui: &Ui) -> i32 {
    use std::sync::atomic::Ordering::Relaxed;
    if ui.is_item_hovered() || ui.is_item_focused() || ui.is_item_active() {
        SLIDER_HOT.store(true, Relaxed);
        SLIDER_STEP.swap(0, Relaxed)
    } else {
        0
    }
}

impl Input {
    fn new() -> Input {
        Input { keys: [false; KEYMAP.len()], mouse: [false; 2], hold: [-1.0; 256], esc: false, wheel_last: None, tab_step: 0, enter_space: false, arrow_raw: [false; 2] }
    }

    /// Everything released (the overlay was hidden, or the game window lost focus).
    fn release_all(&mut self, io: &mut Io) {
        self.wheel_last = None;
        for (i, (_, k)) in KEYMAP.iter().enumerate() {
            if self.keys[i] {
                io.add_key_event(*k, false);
                self.keys[i] = false;
            }
        }
        for (i, b) in [MouseButton::Left, MouseButton::Right].into_iter().enumerate() {
            if self.mouse[i] {
                io.add_mouse_button_event(b, false);
                self.mouse[i] = false;
            }
        }
        self.hold = [-1.0; 256];
    }

    /// Mouse, keys and typed characters from the OS (the game reads DirectInput, so polling is
    /// independent of it). Returns true when Esc went down this frame.
    fn feed(&mut self, io: &mut Io, size: (u32, u32), dt: f32) -> bool {
        let hwnd = unsafe { *(GAME_HWND as *const usize) } as *mut std::ffi::c_void;
        let focused = !hwnd.is_null() && unsafe { GetForegroundWindow() } == hwnd;
        if !focused {
            self.release_all(io);
            self.esc = false;
            return false;
        }
        let mut pt = POINT { x: 0, y: 0 };
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            if GetCursorPos(&mut pt) != 0 && ScreenToClient(hwnd, &mut pt) != 0 && GetClientRect(hwnd, &mut rc) != 0 {
                let (cw, ch) = ((rc.right - rc.left).max(1) as f32, (rc.bottom - rc.top).max(1) as f32);
                io.add_mouse_pos_event([pt.x as f32 * size.0 as f32 / cw, pt.y as f32 * size.1 as f32 / ch]);
            }
        }
        // The wheel: the game's window procedure sums WM_MOUSEWHEEL deltas at 0x234F460 (see
        // input.rs `mouse_update`); the change since last frame, in notches of 120.
        let acc = unsafe { *(WHEEL_ACC as *const i32) };
        if let Some(last) = self.wheel_last {
            let d = acc.wrapping_sub(last);
            if d != 0 && d.abs() < 120 * 50 {
                io.add_mouse_wheel_event([0.0, d as f32 / 120.0]);
            }
        }
        self.wheel_last = Some(acc);
        for (i, (vk, b)) in [(0x01, MouseButton::Left), (0x02, MouseButton::Right)].into_iter().enumerate() {
            let d = key_down(vk);
            if d != self.mouse[i] {
                io.add_mouse_button_event(b, d);
                self.mouse[i] = d;
            }
        }
        let mut esc_edge = false;
        let typing = io.want_text_input;
        let arrows_to_tabs = !typing && !SLIDER_HOT.load(std::sync::atomic::Ordering::Relaxed);
        for (i, (vk, k)) in KEYMAP.iter().enumerate() {
            let mut d = key_down(*vk);
            if *vk == 0x25 || *vk == 0x27 {
                let j = (*vk == 0x27) as usize;
                if d && !self.arrow_raw[j] && !typing {
                    let s = if j == 1 { 1 } else { -1 };
                    if arrows_to_tabs {
                        self.tab_step += s;
                    } else {
                        SLIDER_STEP.fetch_add(s, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                self.arrow_raw[j] = d;
                d = d && typing;
            }
            if d != self.keys[i] {
                io.add_key_event(*k, d);
                self.keys[i] = d;
                if *vk == 0x1B && d {
                    esc_edge = true;
                }
            }
        }
        // Enter presses the highlighted button (ImGui only activates with Space).
        let es = key_down(0x0D) && !typing;
        if es != self.enter_space {
            io.add_key_event(Key::Space, es);
            self.enter_space = es;
        }
        // Characters: the first press, then repeats after a delay. Ctrl combinations type nothing.
        let ctrl = key_down(0x11) && !key_down(0x12);
        let mut state = [0u8; 256];
        if key_down(0x10) {
            state[0x10] = 0x80;
        }
        if unsafe { GetKeyState(0x14) } & 1 != 0 {
            state[0x14] = 1;
        }
        for vk in 0..256i32 {
            if !types_text(vk) {
                continue;
            }
            let h = &mut self.hold[vk as usize];
            if !key_down(vk) {
                *h = -1.0;
                continue;
            }
            let emit = if *h < 0.0 {
                *h = 0.45;
                true
            } else {
                *h -= dt;
                if *h <= 0.0 {
                    *h = 0.04;
                    true
                } else {
                    false
                }
            };
            if emit && !ctrl {
                let mut buf = [0u16; 4];
                let n = unsafe { ToUnicode(vk as u32, MapVirtualKeyW(vk as u32, 0), state.as_ptr(), buf.as_mut_ptr(), 4, 4) };
                if n > 0 {
                    for c in char::decode_utf16(buf[..n as usize].iter().copied()).flatten() {
                        if !c.is_control() {
                            io.add_input_character(c);
                        }
                    }
                }
            }
        }
        esc_edge
    }
}

/// Which pages' widgets keep state between frames.
struct Panel {
    label_buf: String,
    label_was_open: bool,
    weather_pick: usize,
    /// The page shown, and pages to move by on the next frame.
    tab: usize,
    tab_step: i32,
}

struct UiState {
    ctx: Context,
    backend: Backend,
    input: Input,
    panel: Panel,
    init_h: u32,
    theme_k: f32,
    last: Instant,
}

// Only the render thread touches the context.
unsafe impl Send for UiState {}

static UI: Mutex<Option<UiState>> = Mutex::new(None);

/// The theme: Saints purple, dark translucent panels, rounded everything. `k` scales sizes with the
/// screen height (1.0 at 1080p).
fn apply_theme(ctx: &mut Context, k: f32) {
    let s = ctx.style_mut();
    s.window_padding = [16.0 * k, 14.0 * k];
    s.window_rounding = 10.0 * k;
    s.window_border_size = 1.0;
    s.window_title_align = [0.5, 0.5];
    s.child_rounding = 8.0 * k;
    s.popup_rounding = 8.0 * k;
    s.frame_padding = [10.0 * k, 6.0 * k];
    s.frame_rounding = 8.0 * k;
    s.frame_border_size = 0.0;
    s.item_spacing = [10.0 * k, 8.0 * k];
    s.item_inner_spacing = [8.0 * k, 6.0 * k];
    s.scrollbar_size = 14.0 * k;
    s.scrollbar_rounding = 10.0 * k;
    s.grab_min_size = 14.0 * k;
    s.grab_rounding = 8.0 * k;
    s.tab_rounding = 8.0 * k;
    let dim = |c: [f32; 4], a: f32| [c[0], c[1], c[2], a];
    s[StyleColor::Text] = rgb(236, 233, 246, 1.0);
    s[StyleColor::TextDisabled] = rgb(140, 135, 160, 1.0);
    s[StyleColor::WindowBg] = rgb(14, 12, 22, 0.90);
    s[StyleColor::ChildBg] = rgb(0, 0, 0, 0.0);
    s[StyleColor::PopupBg] = rgb(20, 17, 32, 0.96);
    s[StyleColor::Border] = dim(ACCENT, 0.60);
    s[StyleColor::BorderShadow] = rgb(0, 0, 0, 0.0);
    s[StyleColor::FrameBg] = rgb(38, 31, 60, 0.90);
    s[StyleColor::FrameBgHovered] = rgb(62, 45, 98, 0.95);
    s[StyleColor::FrameBgActive] = rgb(86, 56, 136, 1.0);
    s[StyleColor::TitleBg] = rgb(78, 28, 124, 0.95);
    s[StyleColor::TitleBgActive] = ACCENT;
    s[StyleColor::TitleBgCollapsed] = rgb(60, 22, 98, 0.90);
    s[StyleColor::ScrollbarBg] = rgb(0, 0, 0, 0.20);
    s[StyleColor::ScrollbarGrab] = dim(ACCENT, 0.75);
    s[StyleColor::ScrollbarGrabHovered] = dim(BRIGHT, 0.85);
    s[StyleColor::ScrollbarGrabActive] = BRIGHT;
    s[StyleColor::CheckMark] = BRIGHT;
    s[StyleColor::SliderGrab] = BRIGHT;
    s[StyleColor::SliderGrabActive] = rgb(205, 155, 255, 1.0);
    s[StyleColor::Button] = dim(ACCENT, 0.78);
    s[StyleColor::ButtonHovered] = rgb(142, 72, 208, 1.0);
    s[StyleColor::ButtonActive] = BRIGHT;
    s[StyleColor::Header] = dim(ACCENT, 0.55);
    s[StyleColor::HeaderHovered] = rgb(142, 72, 208, 0.85);
    s[StyleColor::HeaderActive] = dim(BRIGHT, 0.90);
    s[StyleColor::Separator] = dim(ACCENT, 0.55);
    s[StyleColor::SeparatorHovered] = dim(BRIGHT, 0.80);
    s[StyleColor::SeparatorActive] = BRIGHT;
    s[StyleColor::ResizeGrip] = dim(ACCENT, 0.40);
    s[StyleColor::ResizeGripHovered] = dim(BRIGHT, 0.70);
    s[StyleColor::ResizeGripActive] = BRIGHT;
    s[StyleColor::Tab] = rgb(66, 27, 104, 0.85);
    s[StyleColor::TabHovered] = rgb(142, 72, 208, 1.0);
    s[StyleColor::TabActive] = ACCENT;
    s[StyleColor::TabUnfocused] = rgb(48, 22, 78, 0.85);
    s[StyleColor::TabUnfocusedActive] = rgb(90, 34, 142, 1.0);
    s[StyleColor::TextSelectedBg] = dim(ACCENT, 0.55);
    s[StyleColor::NavHighlight] = BRIGHT;
    s[StyleColor::NavWindowingHighlight] = BRIGHT;
    s[StyleColor::NavWindowingDimBg] = rgb(0, 0, 0, 0.45);
    s[StyleColor::ModalWindowDimBg] = rgb(0, 0, 0, 0.45);
}

fn font_data() -> Option<Vec<u8>> {
    let dir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    ["segoeui.ttf", "arial.ttf"].iter().find_map(|f| std::fs::read(format!("{dir}\\Fonts\\{f}")).ok())
}

fn init(dev: usize, size: (u32, u32)) -> Option<UiState> {
    let mut ctx = Context::create();
    ctx.set_ini_filename(None);
    ctx.set_log_filename(None);
    // The font size is fixed when the atlas is built (20 px at 1080p); later resolution changes
    // only scale it.
    let px = (size.1 as f32 / 1080.0 * 20.0).clamp(16.0, 40.0).round();
    let ttf = font_data();
    match &ttf {
        Some(data) => {
            ctx.fonts().add_font(&[FontSource::TtfData {
                data,
                size_pixels: px,
                config: Some(FontConfig { oversample_h: 3, oversample_v: 2, ..FontConfig::default() }),
            }]);
        }
        None => {
            crate::log("overlay: no system font found, using ImGui's built-in one");
            ctx.fonts().add_font(&[FontSource::DefaultFontData {
                config: Some(FontConfig { size_pixels: px, ..FontConfig::default() }),
            }]);
        }
    }
    let (w, h, pixels) = {
        let t = ctx.fonts().build_rgba32_texture();
        (t.width, t.height, t.data.to_vec())
    };
    let backend = unsafe { Backend::new(dev, w, h, &pixels) }?;
    ctx.fonts().tex_id = TextureId::new(1);
    let k = (size.1 as f32 / 1080.0).clamp(0.75, 2.0);
    apply_theme(&mut ctx, k);
    {
        let io = ctx.io_mut();
        io.config_flags |= imgui::ConfigFlags::NAV_ENABLE_KEYBOARD;
        io.mouse_draw_cursor = true;
    }
    crate::log(&format!("overlay: ImGui ready, back buffer {}x{}, font {px} px, atlas {w}x{h}", size.0, size.1));
    Some(UiState {
        ctx,
        backend,
        input: Input::new(),
        panel: Panel { label_buf: String::new(), label_was_open: false, weather_pick: 0, tab: 0, tab_step: 0 },
        init_h: size.1,
        theme_k: k,
        last: Instant::now(),
    })
}

/// Called from the Present hook on the render thread, right before the original Present.
pub(super) unsafe fn frame(dev: usize) {
    let mut g = UI.lock().unwrap_or_else(|p| p.into_inner());
    if !menu::overlay_visible() {
        // Hidden: nothing is drawn or polled. Let go of anything still held.
        if let Some(st) = g.as_mut() {
            let UiState { ctx, input, .. } = st;
            input.release_all(ctx.io_mut());
            input.esc = false;
            st.panel.label_was_open = false;
            st.last = Instant::now();
        }
        return;
    }
    let Some(size) = (unsafe { Backend::back_buffer_size(dev) }) else {
        return;
    };
    if g.is_none() {
        match init(dev, size) {
            Some(s) => *g = Some(s),
            None => {
                super::disable("could not create the font texture");
                return;
            }
        }
    }
    let Some(st) = g.as_mut() else { return };
    let UiState { ctx, backend, input, panel, init_h, theme_k, last } = st;
    let now = Instant::now();
    let dt = now.duration_since(*last).as_secs_f32().clamp(1.0 / 240.0, 0.1);
    *last = now;
    let k = (size.1 as f32 / 1080.0).clamp(0.75, 2.0);
    if (k - *theme_k).abs() > 0.01 {
        apply_theme(ctx, k);
        *theme_k = k;
    }
    let esc = {
        let io = ctx.io_mut();
        io.display_size = [size.0 as f32, size.1 as f32];
        io.font_global_scale = size.1 as f32 / *init_h as f32;
        io.delta_time = dt;
        input.feed(io, size, dt)
    };
    let step = std::mem::take(&mut input.tab_step);
    if menu::overlay_menu_open() && !menu::overlay_label_open() {
        panel.tab_step += step;
    }
    // A step nobody took (the slider stopped being hot) is dropped next frame.
    if !SLIDER_HOT.swap(false, std::sync::atomic::Ordering::Relaxed) {
        SLIDER_STEP.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    // Esc cancels the label box, or else closes the menu.
    if esc {
        if menu::overlay_label_open() {
            menu::ui_close_label_box(None);
        } else if menu::overlay_menu_open() {
            menu::ui_close_menu();
        }
    }
    let snap = menu::overlay_snapshot();
    let light = menu::ui_light();
    let ui = ctx.new_frame();
    build(ui, panel, &snap, &light, size, k);
    let data = ctx.render();
    unsafe { backend.render(dev, data) };
}

fn wide(ui: &Ui, label: &str) -> bool {
    ui.button_with_size(label, [ui.content_region_avail()[0], 0.0])
}

fn build(ui: &Ui, p: &mut Panel, snap: &Snapshot, light: &Light, size: (u32, u32), k: f32) {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let label_open = menu::overlay_label_open();
    if label_open {
        ui.get_background_draw_list().add_rect([0.0, 0.0], [w, h], [0.0, 0.0, 0.0, 0.43]).filled(true).build();
    }
    if menu::overlay_menu_open() {
        main_window(ui, p, snap, light, w, h, k, !label_open);
    }
    if label_open {
        label_window(ui, p, light, w, h, k);
    }
    p.label_was_open = label_open;
}

#[allow(clippy::too_many_arguments)]
fn main_window(ui: &Ui, p: &mut Panel, snap: &Snapshot, light: &Light, w: f32, h: f32, k: f32, focus: bool) {
    let mut open = true;
    let mut win = ui
        .window("SR2 Decomp  -  test menu###sr2main")
        .opened(&mut open)
        .position([w - 24.0 * k, h - 24.0 * k], Condition::FirstUseEver)
        .position_pivot([1.0, 1.0])
        .size([500.0 * k, (h - 48.0 * k).min(700.0 * k)], Condition::FirstUseEver)
        .collapsible(false);
    if focus {
        win = win.focus_on_appearing(true);
    }
    win.build(|| {
        ui.text_colored(BRIGHT, "Saints Row 2 Decomp");
        ui.same_line();
        ui.text_disabled("F7 closes, F8 marks a log point");
        ui.separator();
        // Left / Right arrows (outside sliders and text boxes) move between the pages.
        const PAGES: usize = 5;
        let target = (p.tab_step != 0).then(|| (p.tab as i32 + p.tab_step).rem_euclid(PAGES as i32) as usize);
        p.tab_step = 0;
        if let Some(_tabs) = ui.tab_bar("pages") {
            let fl = |i: usize| if target == Some(i) { TabItemFlags::SET_SELECTED } else { TabItemFlags::empty() };
            if let Some(_t) = ui.tab_item_with_flags("Player", None, fl(0)) {
                p.tab = 0;
                page_player(ui, light, snap);
            }
            if let Some(_t) = ui.tab_item_with_flags("World", None, fl(1)) {
                p.tab = 1;
                page_world(ui, p, light, snap);
            }
            if let Some(_t) = ui.tab_item_with_flags("Cheats", None, fl(2)) {
                p.tab = 2;
                page_cheats(ui, snap);
            }
            if let Some(_t) = ui.tab_item_with_flags("Shops", None, fl(3)) {
                p.tab = 3;
                page_shops(ui, snap);
            }
            if let Some(_t) = ui.tab_item_with_flags("Logging", None, fl(4)) {
                p.tab = 4;
                page_logging(ui, snap, light);
            }
        }
        ui.separator();
        if !light.status.is_empty() {
            ui.text_colored([200.0 / 255.0, 160.0 / 255.0, 1.0, 1.0], &light.status);
        }
        ui.text_disabled(format!("log: {} | point {} | {} | mem {} / {} MB", snap.log_mission, snap.log_point, snap.log_samples, snap.mem.0, snap.mem.1));
    });
    if !open {
        menu::ui_close_menu();
    }
}

fn page_player(ui: &Ui, light: &Light, snap: &Snapshot) {
    let mut god = light.god;
    if ui.checkbox("God mode", &mut god) {
        menu::ui_set_god(god);
    }
    let mut ammo = light.ammo;
    if ui.checkbox("Infinite ammo", &mut ammo) {
        menu::ui_set_ammo(ammo);
    }
    ui.spacing();
    if wide(ui, "Refill health") {
        menu::ui_action(Action::Heal);
    }
    if wide(ui, "Add $10,000") {
        menu::ui_action(Action::Cash);
    }
    if wide(ui, "Add 5,000 respect") {
        menu::ui_action(Action::Respect);
    }
    if wide(ui, "Clear wanted level") {
        menu::ui_action(Action::ClearWanted);
    }
    if wide(ui, "Teleport to waypoint") {
        menu::ui_action(Action::TeleportWaypoint);
    }
    ui.spacing();
    let mut stars = light.wanted as i32;
    let mut changed = ui.slider("Wanted stars", 1, 5, &mut stars);
    let step = mark_slider(ui);
    if step != 0 {
        stars = (stars + step).clamp(1, 5);
        changed = true;
    }
    if changed {
        menu::ui_set_wanted(stars as u32);
    }
    if wide(ui, &format!("Set wanted level to {stars}")) {
        menu::ui_action(Action::Wanted(stars.clamp(1, 5) as u32));
    }
    ui.spacing();
    ui.text(format!("Time of day: {}", if snap.clock.is_empty() { "--:--" } else { &snap.clock }));
    if ui.button("-1 hour") {
        menu::ui_action(Action::Time(23));
    }
    ui.same_line();
    if ui.button("+1 hour") {
        menu::ui_action(Action::Time(1));
    }
}

fn page_world(ui: &Ui, p: &mut Panel, light: &Light, snap: &Snapshot) {
    ui.text("Weather");
    let mut names: Vec<String> = vec!["normal cycle".into()];
    names.extend(snap.weathers.iter().cloned());
    p.weather_pick = (light.weather + 1).clamp(0, names.len() as i32 - 1) as usize;
    ui.set_next_item_width(-1.0);
    if ui.combo_simple_string("##weather", &mut p.weather_pick, &names) {
        menu::ui_set_weather(p.weather_pick as i32 - 1);
    }
    ui.text_disabled("Picking a weather applies it at once.");
}

fn page_cheats(ui: &Ui, snap: &Snapshot) {
    ui.text_disabled("Game cheats (may affect achievements)");
    if snap.cheats.is_empty() {
        ui.text("(no cheats loaded yet)");
    }
    for (i, (name, toggles, active)) in snap.cheats.iter().enumerate() {
        let tag = if !toggles {
            "   (once)"
        } else if *active {
            "   [ON]"
        } else {
            ""
        };
        if ui.selectable(format!("{name}{tag}##cheat{i}")) {
            menu::ui_action(Action::Cheat(i, !active));
        }
    }
}

fn page_shops(ui: &Ui, snap: &Snapshot) {
    if !snap.shops_loaded {
        ui.text("(shops not loaded yet)");
        return;
    }
    ui.text_disabled("Pick a shop to teleport to its door");
    for (ci, (name, shops)) in snap.chains.iter().enumerate() {
        if let Some(_n) = ui.tree_node(format!("{name}  ({})###chain{ci}", shops.len())) {
            for (i, label) in shops {
                if ui.selectable(format!("{label}##shop{i}")) {
                    menu::ui_action(Action::TeleportShop(*i));
                }
            }
        }
    }
}

fn page_logging(ui: &Ui, snap: &Snapshot, light: &Light) {
    let mut on = snap.log_on;
    if ui.checkbox("Mission logging", &mut on) {
        crate::missionlog::set_on(on);
    }
    ui.text(format!("Mission: {}", snap.log_mission));
    if ui.button("<  previous") {
        crate::missionlog::pick_mission(-1);
    }
    ui.same_line();
    if ui.button("next  >") {
        crate::missionlog::pick_mission(1);
    }
    ui.spacing();
    if wide(ui, &format!("Mark point (next: {})   [F8]", light.label_point)) {
        menu::ui_open_label_box();
    }
    ui.text_disabled(&snap.log_samples);
}

fn label_window(ui: &Ui, p: &mut Panel, light: &Light, w: f32, h: f32, k: f32) {
    let first = !p.label_was_open;
    if first {
        p.label_buf.clear();
    }
    let mut save = false;
    let mut cancel = false;
    ui.window(format!("Mission log point {}###sr2label", light.label_point))
        .position([w * 0.5, h * 0.5], Condition::Always)
        .position_pivot([0.5, 0.5])
        .size([460.0 * k, 0.0], Condition::Always)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_RESIZE | WindowFlags::NO_MOVE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::ALWAYS_AUTO_RESIZE)
        .focused(true)
        .build(|| {
            ui.text("Label for this point:");
            if first {
                ui.set_keyboard_focus_here();
            }
            ui.set_next_item_width(-1.0);
            if ui.input_text("##label", &mut p.label_buf).enter_returns_true(true).build() {
                save = true;
            }
            if p.label_buf.chars().count() > LABEL_MAX {
                p.label_buf = p.label_buf.chars().take(LABEL_MAX).collect();
            }
            if ui.button("Save") {
                save = true;
            }
            ui.same_line();
            if ui.button("Cancel") {
                cancel = true;
            }
            ui.text_disabled("Enter = save, Esc = cancel");
        });
    if save {
        menu::ui_close_label_box(Some(p.label_buf.trim().to_string()));
    } else if cancel {
        menu::ui_close_label_box(None);
    }
}
