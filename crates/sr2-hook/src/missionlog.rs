//! Mission logger: which code runs during each part of a mission, for finding and
//! naming functions (tools/mission_report.py turns the log into a report).
//!
//! A sampling thread looks at where the game and render threads are (their EIP)
//! about once a millisecond and counts the addresses. The counts are split into
//! sections: a new section starts when a point is marked (menu or F8), when the
//! picked mission changes, or when the game's active mission changes
//! (`0x27B3C60` -> record, name at +0x14). Each finished section is appended to
//! `sr2-mission.log` in the game folder:
//!
//! ```text
//! == section 4 | mission 5 | game tss02 | point 2 | frames 1200-5400 | 4100 samples
//! g 00a3b950 12
//! r 00d0a380 40
//! ```
//!
//! The thread is suspended only for `GetThreadContext`; nothing allocates while it
//! is (the game thread might hold the heap lock).

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Diagnostics::Debug::{CONTEXT, GetThreadContext, ReadProcessMemory};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenThread, ResumeThread, Sleep, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
};

const ACTIVE_MISSION: usize = 0x027B_3C60;
const LOG_FILE: &str = "sr2-mission.log";
const CONTEXT_CONTROL: u32 = 0x0001_0001;
const TEXT_LO: u32 = 0x0040_1000;
const TEXT_HI: u32 = 0x00DB_0000;

static ON: AtomicBool = AtomicBool::new(false);
static GAME_THREAD: AtomicU32 = AtomicU32::new(0);
static RENDER_THREAD: AtomicU32 = AtomicU32::new(0);
static FRAME: AtomicU32 = AtomicU32::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

struct Section {
    number: u32,
    mission: u32,
    game_mission: String,
    point: u32,
    first_frame: u32,
    samples: u32,
    game: BTreeMap<u32, u32>,
    render: BTreeMap<u32, u32>,
}

impl Section {
    const fn new() -> Self {
        Section {
            number: 0,
            mission: 1,
            game_mission: String::new(),
            point: 0,
            first_frame: 0,
            samples: 0,
            game: BTreeMap::new(),
            render: BTreeMap::new(),
        }
    }
}

static SECTION: Mutex<Section> = Mutex::new(Section::new());

fn lock() -> std::sync::MutexGuard<'static, Section> {
    SECTION.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct Status {
    pub on: bool,
    pub point: u32,
    pub mission_label: String,
    pub samples_text: String,
}

pub fn status() -> Status {
    let s = lock();
    let game = if s.game_mission.is_empty() { "none".to_string() } else { s.game_mission.clone() };
    Status {
        on: ON.load(Ordering::Acquire),
        point: s.point,
        mission_label: format!("{} (game: {game})", s.mission),
        samples_text: format!("section {}, {} samples", s.number, s.samples),
    }
}

/// End the current section (written if logging was on) and start the next one.
fn next_section(s: &mut Section) {
    if ON.load(Ordering::Acquire) && s.samples > 0 {
        write_section(s);
    }
    s.number += 1;
    s.first_frame = FRAME.load(Ordering::Relaxed);
    s.samples = 0;
    s.game.clear();
    s.render.clear();
}

fn write_section(s: &Section) {
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOG_FILE) else {
        return;
    };
    let game = if s.game_mission.is_empty() { "none" } else { &s.game_mission };
    let mut out = format!(
        "== section {} | mission {} | game {game} | point {} | frames {}-{} | {} samples\n",
        s.number,
        s.mission,
        s.point,
        s.first_frame,
        FRAME.load(Ordering::Relaxed),
        s.samples
    );
    for (tag, map) in [("g", &s.game), ("r", &s.render)] {
        for (a, n) in map {
            out += &format!("{tag} {a:08x} {n}\n");
        }
    }
    let _ = f.write_all(out.as_bytes());
}

pub fn set_on(on: bool) {
    let mut s = lock();
    if on {
        ON.store(true, Ordering::Release);
        let _ = OpenOptions::new().create(true).append(true).open(LOG_FILE).and_then(|mut f| {
            writeln!(f, "== logging on | mission {} | frame {}", s.mission, FRAME.load(Ordering::Relaxed))
        });
        next_section(&mut s);
        start_sampler();
    } else {
        next_section(&mut s);
        ON.store(false, Ordering::Release);
    }
    crate::log(&format!("missionlog: logging {}", if on { "on" } else { "off" }));
}

pub fn mark() {
    let mut s = lock();
    next_section(&mut s);
    s.point += 1;
}

pub fn pick_mission(d: i32) {
    let mut s = lock();
    next_section(&mut s);
    s.mission = (s.mission as i32 + d).clamp(1, 99) as u32;
    s.point = 0;
}

fn read<T: Copy + Default>(addr: usize) -> Option<T> {
    let mut v = T::default();
    let mut n = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            addr as *const _,
            &mut v as *mut T as *mut _,
            std::mem::size_of::<T>(),
            &mut n,
        )
    };
    (ok != 0 && n == std::mem::size_of::<T>()).then_some(v)
}

fn read_name(addr: usize) -> Option<String> {
    let b: [u8; 32] = read(addr)?;
    let end = b.iter().position(|&c| c == 0)?;
    (end >= 2 && b[..end].iter().all(|c| c.is_ascii_graphic())).then(|| String::from_utf8_lossy(&b[..end]).into())
}

/// The game's active mission name, if any (inline at +0x14, or a pointer there).
fn active_mission() -> String {
    let Some(rec) = read::<u32>(ACTIVE_MISSION).filter(|&r| r != 0) else {
        return String::new();
    };
    let at = rec as usize + 0x14;
    read_name(at).or_else(|| read::<u32>(at).and_then(|p| read_name(p as usize))).unwrap_or_default()
}

/// Called on the game thread every frame.
pub fn tick() {
    GAME_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    let f = FRAME.fetch_add(1, Ordering::Relaxed);
    if f % 30 != 0 {
        return;
    }
    let name = active_mission();
    let mut s = lock();
    if name != s.game_mission {
        next_section(&mut s);
        if !name.is_empty() {
            s.point = 0;
        }
        crate::log(&format!("missionlog: game mission now '{name}'"));
        s.game_mission = name;
    }
}

/// Called on the render thread every frame.
pub fn render_tick() {
    RENDER_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
}

fn start_sampler() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let mut handles: [(u32, HANDLE); 2] = [(0, std::ptr::null_mut()); 2];
        let mut batch: Vec<(u8, u32)> = Vec::with_capacity(64);
        loop {
            unsafe { Sleep(1) };
            if !ON.load(Ordering::Acquire) {
                unsafe { Sleep(50) };
                continue;
            }
            for (k, id) in [GAME_THREAD.load(Ordering::Relaxed), RENDER_THREAD.load(Ordering::Relaxed)]
                .into_iter()
                .enumerate()
            {
                if id == 0 {
                    continue;
                }
                if handles[k].0 != id {
                    if !handles[k].1.is_null() {
                        unsafe { CloseHandle(handles[k].1) };
                    }
                    let h = unsafe {
                        OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, id)
                    };
                    handles[k] = (id, h);
                }
                let h = handles[k].1;
                if h.is_null() {
                    continue;
                }
                let mut ctx: CONTEXT = unsafe { std::mem::zeroed() };
                ctx.ContextFlags = CONTEXT_CONTROL;
                let eip = unsafe {
                    if SuspendThread(h) == u32::MAX {
                        continue;
                    }
                    let ok = GetThreadContext(h, &mut ctx);
                    ResumeThread(h);
                    if ok == 0 {
                        continue;
                    }
                    ctx.Eip
                };
                if (TEXT_LO..TEXT_HI).contains(&eip) {
                    batch.push((k as u8, eip));
                }
            }
            if batch.len() >= 32 {
                let mut s = lock();
                for &(k, eip) in &batch {
                    *if k == 0 { &mut s.game } else { &mut s.render }.entry(eip).or_insert(0) += 1;
                    s.samples += 1;
                }
                batch.clear();
            }
        }
    });
}
