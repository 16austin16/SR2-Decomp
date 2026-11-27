//! Registry mode, for the offline judge (crates/sr2-judge). Agents must not edit this file.
//!
//! While the mode is on, `portlib::replace_pinned` records (name, address, original body, thunk) and
//! returns true WITHOUT patching anything, logging or touching the trace ring. `collect()` runs every
//! port module's install() in that mode and returns what they registered. The DLL never switches the
//! mode on, so its behaviour in the game is unchanged.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// One registered port.
#[derive(Clone)]
pub struct RegEntry {
    pub name: String,
    pub addr: usize,
    pub body: Vec<u8>,
    pub thunk: usize,
    /// Set when the address is in portlib's EXCLUDED list (known-bad ports, not installed in the game).
    pub excluded: Option<&'static str>,
    /// Installed by a module the game does not install (held back).
    pub held: bool,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: AtomicBool = AtomicBool::new(false);
static ENTRIES: Mutex<Vec<RegEntry>> = Mutex::new(Vec::new());

pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub(crate) fn record(name: &str, addr: usize, body: &[u8], thunk: usize) {
    let excluded = crate::portlib::EXCLUDED.iter().find(|&&(x, _)| x == addr).map(|&(_, why)| why);
    if let Ok(mut v) = ENTRIES.lock() {
        v.push(RegEntry {
            name: name.to_string(),
            addr,
            body: body.to_vec(),
            thunk,
            excluded,
            held: HELD.load(Ordering::Relaxed),
        });
    }
}

fn guarded(what: &str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        eprintln!("registry: {what} panicked while installing (its ports after the panic are missing)");
    }
}

/// Run every port module's install() in registry mode. The first registration of an address wins, as
/// in the game (a second detour of the same address fails its body check there).
pub fn collect() -> Vec<RegEntry> {
    if let Ok(mut v) = ENTRIES.lock() {
        v.clear();
    }
    if let Ok(mut m) = crate::portlib::PINNED.lock() {
        m.clear();
    }
    ACTIVE.store(true, Ordering::SeqCst);
    HELD.store(false, Ordering::SeqCst);
    guarded("the port modules", crate::hooks::install_port_modules);
    guarded("twins", crate::twins::install);
    HELD.store(true, Ordering::SeqCst);
    guarded("port_nav2", crate::port_nav2::install);
    ACTIVE.store(false, Ordering::SeqCst);
    let all = ENTRIES.lock().map(|v| v.clone()).unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    all.into_iter().filter(|e| seen.insert(e.addr)).collect()
}
