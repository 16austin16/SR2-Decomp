//! Streaming test harness: teleports the local player around a loop of points
//! so the streamer loads and unloads chunks at the same places every run.
//! Enabled by `TELEPORT_TEST` in hooks.rs; called from the frame hook.
//!
//! It does what `luafn_teleport` (0xA47310) does: flag = `0x683F40(EAX = obj, 4, 0)`
//! (a u16); for a player human (`0x963540`: object type +0x4C == 1 and +0x3CC == 1)
//! `0x9DB480(ECX = obj; position, orientation, 1, 1, false, flag)`, caller-clean;
//! for anything else `0x447CD0(obj, 0, 0)` then
//! `0x9A0B00(ECX = orientation, EDX = position; obj, 0, 1, flag != 0, 0)`.
//!
//! Object layout used: +0x14 position (3 floats, Y up), +0x20 orientation
//! (3x3 floats), +0x48 grid index (`object::update_grid_index`, 0x925A00).

use crate::log;

const LOCAL_PLAYER: usize = 0x0217_03D4;
const IS_PLAYER_HUMAN: usize = 0x0096_3540; // EAX = object -> AL
const TELEPORT_FLAG: usize = 0x0068_3F40; // EAX = object, cdecl (4, 0) -> AX
const TELEPORT_PLAYER: usize = 0x009D_B480; // ECX = object, (position, orientation, 1, 1, bool, flag) caller-clean
const PREPARE_TELEPORT: usize = 0x0044_7CD0; // cdecl (object, 0, 0)
const TELEPORT_OBJECT: usize = 0x009A_0B00; // ECX = orientation, EDX = position, cdecl (object, 0, 1, flag, 0)
/// A move this large in one frame that we didn't make (the save loading) restarts the test.
const JUMP_RESET: f32 = 50.0;

/// Frames the player must have existed before the first jump, and between jumps.
const SETTLE_FRAMES: u32 = 1800;
const INTERVAL: u32 = 1200;
/// Offsets from the starting point (metres, X/Z), visited in order, then home.
/// Y is lifted so the player drops onto whatever is below.
const LOOP: [(f32, f32); 8] = [
    (600.0, 0.0),
    (600.0, 600.0),
    (0.0, 600.0),
    (-600.0, 600.0),
    (-600.0, 0.0),
    (-600.0, -600.0),
    (0.0, -600.0),
    (0.0, 0.0),
];
const LIFT: f32 = 40.0;

struct State {
    last: [f32; 3],
    seen_at: u32,
    start: [f32; 3],
    next: usize,
}

static STATE: std::sync::Mutex<Option<State>> = std::sync::Mutex::new(None);

unsafe fn player() -> usize {
    unsafe { *(LOCAL_PLAYER as *const usize) }
}

unsafe fn position(obj: usize) -> [f32; 3] {
    unsafe { *((obj + 0x14) as *const [f32; 3]) }
}

/// Call `f` with EAX = `obj` and cdecl stack arguments; returns EAX.
unsafe fn call_eax(f: usize, obj: usize, a: u32, b: u32) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "push {b}", "push {a}", "call {f}", "add esp, 8",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            f = in(reg) f, a = in(reg) a, b = in(reg) b,
            inout("eax") obj => r, clobber_abi("C"),
        );
    }
    r
}

/// Returns which path was taken.
unsafe fn teleport(obj: usize, pos: &[f32; 3]) -> &'static str {
    unsafe {
        let orient: [f32; 9] = *((obj + 0x20) as *const [f32; 9]);
        let flag = call_eax(TELEPORT_FLAG, obj, 4, 0) & 0xFFFF;
        if call_eax(IS_PLAYER_HUMAN, obj, 0, 0) as u8 != 0 {
            let args: [u32; 6] = [pos.as_ptr() as u32, orient.as_ptr() as u32, 1, 1, 0, flag];
            std::arch::asm!(
                "push ebx", "push esi", "push edi", "push ebp",
                "push dword ptr [{a} + 20]", "push dword ptr [{a} + 16]", "push dword ptr [{a} + 12]",
                "push dword ptr [{a} + 8]", "push dword ptr [{a} + 4]", "push dword ptr [{a}]",
                "call {f}",
                "add esp, 24",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                a = in(reg) args.as_ptr(), f = in(reg) TELEPORT_PLAYER, in("ecx") obj, clobber_abi("C"),
            );
            "player path"
        } else {
            std::arch::asm!(
                "push ebx", "push esi", "push edi", "push ebp",
                "push 0", "push 0", "push {o}", "call {f}", "add esp, 12",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                o = in(reg) obj, f = in(reg) PREPARE_TELEPORT, clobber_abi("C"),
            );
            let args: [u32; 5] = [obj as u32, 0, 1, (flag != 0) as u32, 0];
            std::arch::asm!(
                "push ebx", "push esi", "push edi", "push ebp",
                "push dword ptr [{a} + 16]", "push dword ptr [{a} + 12]", "push dword ptr [{a} + 8]",
                "push dword ptr [{a} + 4]", "push dword ptr [{a}]",
                "call {f}",
                "add esp, 20",
                "pop ebp", "pop edi", "pop esi", "pop ebx",
                a = in(reg) args.as_ptr(), f = in(reg) TELEPORT_OBJECT,
                in("ecx") orient.as_ptr(), in("edx") pos.as_ptr(), clobber_abi("C"),
            );
            "object path"
        }
    }
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Called once per frame (before the game's frame) with the frame number.
pub fn tick(frame: u32) {
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let obj = unsafe { player() };
    if obj == 0 {
        if st.is_some() {
            log(&format!("teleport test: frame {frame}: local player gone"));
        }
        *st = None;
        return;
    }
    let pos = unsafe { position(obj) };
    let fresh = |frame| State { last: pos, seen_at: frame, start: pos, next: 0 };
    let s = st.get_or_insert_with(|| {
        log(&format!("teleport test: frame {frame}: local player {obj:#x} at {pos:?}"));
        fresh(frame)
    });
    if dist(&pos, &s.last) > JUMP_RESET {
        log(&format!("teleport test: frame {frame}: player moved {:?} -> {pos:?} by itself, restarting", s.last));
        *s = fresh(frame);
    }
    s.last = pos;
    if s.next >= LOOP.len() || frame != s.seen_at + SETTLE_FRAMES + s.next as u32 * INTERVAL {
        return;
    }
    let (dx, dz) = LOOP[s.next];
    let target = [s.start[0] + dx, s.start[1] + LIFT, s.start[2] + dz];
    let path = unsafe { teleport(obj, &target) };
    let after = unsafe { position(obj) };
    s.last = after;
    log(&format!(
        "teleport test: frame {frame}: jump {} of {} to {target:?} ({path}): from {pos:?}, now {after:?}",
        s.next + 1,
        LOOP.len(),
    ));
    s.next += 1;
    if s.next == LOOP.len() {
        log(&format!("teleport test: done at frame {frame}"));
    }
}
