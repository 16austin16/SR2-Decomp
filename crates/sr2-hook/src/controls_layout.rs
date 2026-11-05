//! GTA V (PC) style default keyboard layout (docs/analysis/14_input.md, "Bindings").
//!
//! The game binds keys per *button record* (87 records, `{key, pad_button}` at 0x234E778); the
//! actions reach the records through the action table, so changing a record's key changes every
//! action on that record. The defaults come from the table at 0xE829A0 (87 DIK scan codes, in
//! writable `.data`), which `input_default_key_bindings` (0xC12090) copies in. That function is
//! called at start-up and by the options menu's "restore defaults" (0x774170), so patching the
//! table makes the new layout the default everywhere and the player can still rebind in the menu.
//!
//! Existing installs: `input_dinput8_create` loads `input.bin` over the defaults, so a saved file
//! would keep the old keys. The hook on `input_pads_init_force_feedback` (0xC13500, the last call of
//! `input_dinput8_create`) applies the new layout once: each changed record whose key is still the
//! old default is switched to the new one (a key the player rebound is left alone), and
//! `input_save_bindings` (0xC12200) writes `input.bin`. `sr2-controls-applied.txt` in the game
//! folder records that this ran. Creating `sr2-controls-classic.txt` turns the whole module off.
//!
//! Undo: create `sr2-controls-classic.txt`, delete `input.bin` (save folder) and
//! `sr2-controls-applied.txt`; the next start has the original SR2 keys.
//!
//! Touches only key data; none of the input ports (input.rs, actions.rs, controls.rs) change, so
//! their shadow checks see the same code paths with different key numbers.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

use crate::{detour, log};

const DEFAULT_KEYS: usize = 0x00E8_29A0; // 87 x u32 DIK scan codes
const BINDINGS: usize = 0x0234_E778; // 87 x {u32 key, u32 pad button}
const RECORDS: usize = 87;
const PADS_INIT_FF: usize = 0x00C1_3500;
const SAVE_BINDINGS: usize = 0x00C1_2200;
const MARKER: &str = "sr2-controls-applied.txt";
const OPT_OUT: &str = "sr2-controls-classic.txt";
const MARKER_TEXT: &str = "gta5-layout v1\n";

const UNBOUND: u32 = 0xFFFF_FFFF;

// DirectInput scan codes.
const TAB: u32 = 0x0F;
const Q: u32 = 0x10;
const E: u32 = 0x12;
const LCTRL: u32 = 0x1D;
const K: u32 = 0x25;
const L: u32 = 0x26;
const X: u32 = 0x2D;
const C: u32 = 0x2E;
const V: u32 = 0x2F;
const B: u32 = 0x30;
const M: u32 = 0x32;
const COMMA: u32 = 0x33;
const PERIOD: u32 = 0x34;
const F: u32 = 0x21;
const H: u32 = 0x23;
const R: u32 = 0x13;
const LEFT: u32 = 0xCB;
const RIGHT: u32 = 0xCD;

/// (record, original SR2 key, GTA V style key).
const CHANGES: [(usize, u32, u32); 14] = [
    (4, E, F),                // action / use / enter and exit vehicle
    (6, Q, TAB),              // weapon menu (GTA weapon wheel)
    (15, C, LCTRL),           // crouch
    (17, LCTRL, Q),           // pick up Havok object / multi pratfall (moved off Ctrl)
    (20, TAB, M),             // pause map
    (26, H, E),               // horn (and hydraulics max)
    (27, C, B),               // cruise control a (moved off C)
    (28, UNBOUND, V),         // vehicle camera
    (29, LEFT, COMMA),        // radio previous
    (30, RIGHT, PERIOD),      // radio next
    (48, C, X),               // heli strafe (moved off C)
    (49, X, C),               // look behind
    (64, F, K),               // cruise control b (moved off F)
    (65, R, L),               // cruise control c (moved off R)
];

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

fn opted_out() -> bool {
    std::path::Path::new(OPT_OUT).exists()
}

/// Patch the default-key table so new installs and "restore defaults" get the new layout.
unsafe fn patch_defaults() -> bool {
    let table = DEFAULT_KEYS as *mut u32;
    for &(rec, old, _) in &CHANGES {
        let cur = unsafe { *table.add(rec) };
        if cur != old {
            log(&format!("controls_layout: default key table differs at record {rec} ({cur:#x}), not patching"));
            return false;
        }
    }
    let mut prev = 0u32;
    if unsafe { VirtualProtect(table as *const c_void, RECORDS * 4, PAGE_READWRITE, &mut prev) } == 0 {
        log("controls_layout: cannot unprotect the default key table");
        return false;
    }
    for &(rec, _, new) in &CHANGES {
        unsafe { *table.add(rec) = new };
    }
    unsafe { VirtualProtect(table as *const c_void, RECORDS * 4, prev, &mut prev) };
    true
}

/// Runs after the game has loaded (or written) `input.bin`: move old-default keys to the new ones once.
extern "C" fn pads_init_ff_hook() {
    migrate();
    let orig = ORIGINAL.load(Ordering::Relaxed);
    if orig != 0 {
        let f: extern "C" fn() = unsafe { std::mem::transmute(orig) };
        f();
    }
}

fn migrate() {
    if opted_out() || std::fs::read_to_string(MARKER).is_ok_and(|s| s == MARKER_TEXT) {
        return;
    }
    let mut changed = 0;
    for &(rec, old, new) in &CHANGES {
        let slot = (BINDINGS + rec * 8) as *mut u32;
        if unsafe { *slot } == old {
            unsafe { *slot = new };
            changed += 1;
        }
    }
    if changed > 0 {
        let save: extern "C" fn() = unsafe { std::mem::transmute(SAVE_BINDINGS) };
        save();
    }
    match std::fs::write(MARKER, MARKER_TEXT) {
        Ok(()) => log(&format!("controls_layout: applied the GTA V layout to input.bin ({changed} keys changed)")),
        Err(e) => log(&format!("controls_layout: applied {changed} keys but cannot write {MARKER}: {e}")),
    }
}

pub fn install() {
    if opted_out() {
        log("controls_layout: sr2-controls-classic.txt present, keeping the original SR2 keys");
        return;
    }
    if !unsafe { patch_defaults() } {
        return;
    }
    log("controls_layout: default key table now uses the GTA V style layout");
    const PROLOGUE: [u8; 11] = [0x81, 0xEC, 0x4C, 0x02, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00];
    match unsafe { detour::install(PADS_INIT_FF, &PROLOGUE, pads_init_ff_hook as extern "C" fn() as usize) } {
        Ok(tramp) => ORIGINAL.store(tramp, Ordering::Relaxed),
        Err(e) => log(&format!("controls_layout: hook input_pads_init_force_feedback failed: {e:?}")),
    }
}
