//! Port module: the small camera / animation recipes of doc 51 (camera mode stack, smooth blend,
//! angle wrap, follow-camera commit, context classification, shake presets, polar params,
//! anim state lookup), plus the larger maps that could be done exactly.
//!
//! Every port goes through a `pushad` thunk, so all registers come back as the caller left them
//! except what the original defines as its output (EAX, or the registers a tail jump / call into
//! a camera mode's enter function needs). Where an original returns AL only, the upper bytes of
//! EAX are reproduced as the original leaves them.
//!
//! Checks (first `SHADOW_CALLS` calls of each port; the original's answer wins on a mismatch):
//! - pure results (anim lookup, classify, context for handle, request compare): a relocated copy
//!   of the original runs on the same registers and EAX is compared;
//! - state writers (mode stack, blend, follow commit, polar params): snapshot shadow: the copy
//!   runs, its writes are saved and undone, the Rust version runs, and the writes are compared;
//!   the tail `jmp eax` / `call eax` into a mode enter function is patched out of the copy and
//!   the target it would have used is compared instead;
//! - the shake ports: the copy's call to `camera_shake_start` is aimed at a recording stub and
//!   the registers / stack arguments it would have been called with are compared.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- addresses ------------------------------------------------------------------------------------

const HANDLES: usize = 0x0214_9C64;
const LOCAL_PLAYER: usize = 0x0217_03D4;
const MODE_CUR: usize = 0x025F_5AE8;
const MODE_STACK: usize = 0x025F_5AEC;
const MODE_COUNT: usize = 0x025F_5B0C;
const MODE_BASE: usize = 0x025F_5B10;
const CAM_FLAGS: usize = 0x025F_5BE8;
const MODE_TABLE: usize = 0x00DD_5250; // 0x14-byte records, enter fn first
const BLEND: usize = 0x025F_6990;
const SHAKE_START: usize = 0x004A_0A10;
const SQRTF_WRAPPER: usize = 0x0040_EA20;
const FTOL2: usize = 0x00DA_0140;
const ATAN_ST0: usize = 0x00DA_0DC0;

// --- registers ------------------------------------------------------------------------------------

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)] // esp is never read
struct Pushad {
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// The seven general registers in and out of `call_full`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Regs {
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

impl Regs {
    fn of(p: &Pushad) -> Regs {
        Regs { eax: p.eax, ecx: p.ecx, edx: p.edx, ebx: p.ebx, esi: p.esi, edi: p.edi, ebp: p.ebp }
    }
}

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    args: [u32; 4],
    esp: u32,
    want_st0: u32,
    st0: [u32; 2],
}

/// Call `f` with all seven registers from `r` and four stack arguments (harmless for callees
/// that take fewer; a `ret N` callee pops its own), then capture all seven registers into `r`
/// and, when `want_st0`, pop ST0 (returned).
unsafe fn call_full(f: usize, r: &mut Regs, args: [u32; 4], want_st0: bool) -> f64 {
    let mut b = Block { f: f as u32, r: *r, args, esp: 0, want_st0: want_st0 as u32, st0: [0; 2] };
    unsafe {
        // Above the arguments: the target at [esp + 16], then five copies of the block pointer,
        // so that after the callee popped 0..16 bytes of arguments one is at [esp + 20].
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 48], esp",
            "push eax", "push eax", "push eax", "push eax", "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 44]", "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]", "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 16]",
            "push ebx",
            "mov ebx, [esp + 24]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "cmp dword ptr [ebx + 52], 0",
            "je 2f",
            "fstp qword ptr [ebx + 56]",
            "2:",
            "mov esp, [ebx + 48]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
    f64::from_bits(b.st0[0] as u64 | (b.st0[1] as u64) << 32)
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX.
macro_rules! regs_thunk {
    ($name:ident, $f:ident) => {
        regs_thunk!($name, $f, "ret");
    };
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

/// Like `regs_thunk`, but after `popad` EAX is a function to tail-jump to (0 = just return).
macro_rules! tail_thunk {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                "test eax, eax",
                "jz 2f",
                "jmp eax",
                "2:",
                "ret",
                f = sym $f,
            )
        }
    };
}

/// Like `regs_thunk`, but after `popad` EAX is a function to call (0 = none), then `ret`.
macro_rules! call_thunk {
    ($name:ident, $f:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                "test eax, eax",
                "jz 2f",
                "call eax",
                "2:",
                "ret",
                f = sym $f,
            )
        }
    };
}

// --- the port table -------------------------------------------------------------------------------

struct Port {
    name: &'static str,
    addr: usize,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize) -> Port {
    Port { name, addr, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const MODE_PUSH: usize = 0;
const MODE_POP: usize = 1;
const SET_BASE: usize = 2;
const SET_BASE_DEF: usize = 3;
const SMOOTH: usize = 4;
const ANGLE: usize = 5;
const COMMIT: usize = 6;
const SET_TARGET: usize = 7;
const EQUALS: usize = 8;
const CTX_HANDLE: usize = 9;
const CLASSIFY: usize = 10;
const PRESET: usize = 11;
const FROM_POINT: usize = 12;
const POLAR: usize = 13;
const FIND: usize = 14;
const COUNT: usize = 15;

static PORTS: [Port; COUNT] = [
    port("camera_mode_push", 0x0049_3C80),
    port("camera_mode_pop", 0x0049_3E30),
    port("camera_mode_set_base", 0x0049_3EE0),
    port("camera_mode_set_base_default", 0x0049_EEC0),
    port("camera_smooth_blend_begin", 0x004A_33F0),
    port("angle_wrap_in_place", 0x004A_1A50),
    port("camera_follow_commit_context", 0x0049_A670),
    port("camera_follow_set_target_context", 0x0049_A6E0),
    port("camera_follow_request_equals_current", 0x0049_9AF0),
    port("camera_context_for_handle", 0x0049_AAB0),
    port("camera_human_context_classify", 0x0049_AB10),
    port("camera_shake_start_preset", 0x004A_0B60),
    port("camera_shake_from_world_point", 0x004A_0BC0),
    port("camera_params_set_polar", 0x0049_9940),
    port("anim_state_find_by_name", 0x006E_F510),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_camanim: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_camanim: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

/// Bytes of the given ranges, concatenated.
unsafe fn snap(ranges: &[(usize, usize)]) -> Vec<u8> {
    let mut v = Vec::new();
    for &(a, n) in ranges {
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, n) });
    }
    v
}

unsafe fn restore(ranges: &[(usize, usize)], data: &[u8]) {
    let mut o = 0;
    for &(a, n) in ranges {
        unsafe { std::ptr::copy_nonoverlapping(data[o..o + n].as_ptr(), a as *mut u8, n) };
        o += n;
    }
}

/// Snapshot shadow: run `orig`, save its writes and undo them, run `ours`, compare the writes
/// and the returned value. On a mismatch the original's writes are re-applied and its value
/// returned.
unsafe fn shadow_state<T: PartialEq + std::fmt::Debug>(
    id: usize,
    n: u32,
    ranges: &[(usize, usize)],
    orig: impl FnOnce() -> T,
    ours: impl FnOnce() -> T,
) -> T {
    unsafe {
        let before = snap(ranges);
        let theirs_v = orig();
        let theirs = snap(ranges);
        restore(ranges, &before);
        let ours_v = ours();
        let mine = snap(ranges);
        if mine != theirs || ours_v != theirs_v {
            let at = (0..mine.len()).find(|&i| mine[i] != theirs[i]);
            mismatch(id, n, format!("original {theirs_v:?}, Rust {ours_v:?}, first differing byte {at:?}"));
            restore(ranges, &theirs);
            return theirs_v;
        }
        ours_v
    }
}

// --- shared helpers -------------------------------------------------------------------------------

unsafe fn rd64(p: usize) -> f64 {
    unsafe { std::ptr::read_unaligned(p as *const f64) }
}

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// EAX with AL replaced by a bool.
fn lo(eax: u32, al: bool) -> u32 {
    (eax & !0xFF) | al as u32
}

/// The object of a handle (slot table 0x2149C64, 16-byte entries), or 0 when 0 / null.
unsafe fn slot_object(h: u32) -> u32 {
    unsafe { rd(HANDLES + ((h as usize & 0xFFF) << 4)) }
}

/// `call_regs` for a callee that returns on the x87 stack: the result is popped as a qword.
unsafe fn call_regs_st0(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> f64 {
    let mut out = 0f64;
    let block: [u32; 9] =
        [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax, &mut out as *mut f64 as u32];
    unsafe {
        std::arch::asm!(
            "push ebx", "push esi", "push edi", "push ebp",
            "mov ebp, esp",
            "mov ebx, eax",
            "mov ecx, [ebx + 24]",
            "mov esi, [ebx + 20]",
            "2:",
            "test ecx, ecx",
            "jz 3f",
            "push dword ptr [esi + ecx*4 - 4]",
            "dec ecx",
            "jmp 2b",
            "3:",
            "mov ecx, [ebx + 4]", "mov edx, [ebx + 8]", "mov esi, [ebx + 12]", "mov edi, [ebx + 16]",
            "mov eax, [ebx + 28]",
            "call dword ptr [ebx]",
            "mov ecx, [ebx + 32]",
            "fstp qword ptr [ecx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    out
}

/// `sqrtf_wrapper(x)`: cdecl, f32 argument, result on ST0 (an f32 value).
unsafe fn sqrtf(x: u32) -> f32 {
    unsafe { call_regs_st0(SQRTF_WRAPPER, 0, 0, 0, 0, 0, &[x]) as f32 }
}

/// A callee taking its argument on ST0 and returning on ST0 (`f` is called as is).
unsafe fn call_st0_in_out(f: usize, x: f64) -> f64 {
    let mut out = 0f64;
    unsafe {
        std::arch::asm!(
            "fld qword ptr [{x}]",
            "call {f}",
            "fstp qword ptr [edi]",
            x = in(reg) &x as *const f64,
            in("edi") &mut out as *mut f64,
            f = in(reg) f,
            clobber_abi("C"),
        );
    }
    out
}

/// `_ftol2_sse(x)`: argument on ST0, result in EAX.
unsafe fn ftol(x: f64) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!(
            "fld qword ptr [{x}]",
            "call {f}",
            x = in(reg) &x as *const f64,
            f = in(reg) FTOL2,
            out("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

// --- camera mode stack ----------------------------------------------------------------------------

/// What the original's tail jump / call into a mode's enter function sees.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Plan {
    call: u32,
    ecx: u32,
    edx: u32,
}

unsafe fn mode_enter_fn(mode: u32) -> u32 {
    unsafe { rd(MODE_TABLE + (mode as usize).wrapping_mul(0x14)) }
}

const MODE_RANGES: [(usize, usize); 4] = [(MODE_CUR, 0x2C), (CAM_FLAGS, 4), (BLEND, 0x2C), (0x00F5_3AE0, 4)];

// --- 0x493C80 camera_mode_push (ECX = mode, tail-jumps to the enter fn) ---------------------------

/// Remove `mode` from the stack if present (shifting the rest down), push it, clear flag 2 and
/// switch to it unless it is already current. The tail jump sees EAX = enter fn, ECX = mode,
/// EDX = the stack count minus 1 when it was found, else the count.
unsafe fn mode_push(mode: u32) -> Plan {
    unsafe {
        let n = rd(MODE_COUNT) as i32;
        let mut e = n.wrapping_sub(1);
        let mut found = false;
        while e >= 0 {
            if rd(MODE_STACK + (e as usize) * 4) == mode {
                found = true;
                break;
            }
            e -= 1;
        }
        let mut cnt = n;
        if found {
            if e < n.wrapping_sub(1) {
                loop {
                    wd(MODE_STACK + (e as usize) * 4, rd(MODE_STACK + (e as usize) * 4 + 4));
                    cnt = rd(MODE_COUNT) as i32;
                    e += 1;
                    if e >= cnt.wrapping_sub(1) {
                        break;
                    }
                }
            }
            cnt = cnt.wrapping_sub(1);
            wd(MODE_COUNT, cnt as u32);
        }
        wd(MODE_STACK + (cnt as usize).wrapping_mul(4), mode);
        wd(MODE_COUNT, rd(MODE_COUNT).wrapping_add(1));
        if rd(MODE_CUR) == mode {
            return Plan::default();
        }
        wd(CAM_FLAGS, rd(CAM_FLAGS) & !2);
        let f = mode_enter_fn(mode);
        wd(MODE_CUR, mode);
        if f == 0 { Plan::default() } else { Plan { call: f, ecx: mode, edx: cnt as u32 } }
    }
}

/// The plan a patched original copy (jump / call replaced by a return / nothing) would have
/// followed, from its registers and the mode change.
unsafe fn derived_plan(r: &Regs, old_cur: u32) -> Plan {
    unsafe {
        let new = rd(MODE_CUR);
        let f = if new != old_cur { mode_enter_fn(new) } else { 0 };
        if f == 0 { Plan::default() } else { Plan { call: r.eax, ecx: r.ecx, edx: r.edx } }
    }
}

extern "C" fn mode_push_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let plan = match shadowing(MODE_PUSH) {
        None => unsafe { mode_push(p.ecx) },
        Some((n, orig)) => unsafe {
            let old = rd(MODE_CUR);
            shadow_state(
                MODE_PUSH,
                n,
                &MODE_RANGES_PUSH,
                || {
                    let mut rr = Regs::of(&p);
                    call_full(orig, &mut rr, [0; 4], false);
                    derived_plan(&rr, old)
                },
                || mode_push(p.ecx),
            )
        },
    };
    r.eax = plan.call;
    if plan.call != 0 {
        r.ecx = plan.ecx;
        r.edx = plan.edx;
    }
}
tail_thunk!(mode_push_thunk, mode_push_entry);

const MODE_RANGES_PUSH: [(usize, usize); 2] = [(MODE_CUR, 0x2C), (CAM_FLAGS, 4)];

// --- 0x493E30 camera_mode_pop (EDX = mode, tail-jumps to the enter fn) ----------------------------

/// Remove `mode` from the stack if present; the new current mode is the new top, or the base
/// mode when empty. Does nothing for an empty stack. The tail jump sees EAX = enter fn,
/// ECX = the new mode, EDX = the input mode, or (when removed from below the top) the count
/// read last minus 1.
unsafe fn mode_pop(mode: u32) -> Plan {
    unsafe {
        let n = rd(MODE_COUNT) as i32;
        if n <= 0 {
            return Plan::default();
        }
        let old = rd(MODE_CUR);
        let mut cnt = n;
        let mut edx = mode;
        let mut remove = false;
        if rd(MODE_STACK + (n as usize - 1) * 4) == mode {
            remove = true;
        } else {
            let mut e = n - 2;
            let mut found = false;
            while e >= 0 {
                if rd(MODE_STACK + (e as usize) * 4) == mode {
                    found = true;
                    break;
                }
                e -= 1;
            }
            if found {
                edx = (n - 1) as u32;
                if e < n - 1 {
                    loop {
                        wd(MODE_STACK + (e as usize) * 4, rd(MODE_STACK + (e as usize) * 4 + 4));
                        cnt = rd(MODE_COUNT) as i32;
                        e += 1;
                        edx = cnt.wrapping_sub(1) as u32;
                        if e >= cnt.wrapping_sub(1) {
                            break;
                        }
                    }
                }
                remove = true;
            }
        }
        if remove {
            cnt = cnt.wrapping_sub(1);
            wd(MODE_COUNT, cnt as u32);
        }
        let new = if cnt == 0 { rd(MODE_BASE) } else { rd(MODE_STACK + (cnt as usize).wrapping_sub(1).wrapping_mul(4)) };
        wd(CAM_FLAGS, rd(CAM_FLAGS) & !2);
        wd(MODE_CUR, new);
        if new == old {
            return Plan::default();
        }
        let f = mode_enter_fn(new);
        if f == 0 { Plan::default() } else { Plan { call: f, ecx: new, edx } }
    }
}

extern "C" fn mode_pop_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let plan = match shadowing(MODE_POP) {
        None => unsafe { mode_pop(p.edx) },
        Some((n, orig)) => unsafe {
            let old = rd(MODE_CUR);
            shadow_state(
                MODE_POP,
                n,
                &MODE_RANGES_PUSH,
                || {
                    let mut rr = Regs::of(&p);
                    call_full(orig, &mut rr, [0; 4], false);
                    derived_plan(&rr, old)
                },
                || mode_pop(p.edx),
            )
        },
    };
    r.eax = plan.call;
    if plan.call != 0 {
        r.ecx = plan.ecx;
        r.edx = plan.edx;
    }
}
tail_thunk!(mode_pop_thunk, mode_pop_entry);

// --- 0x4A33F0 camera_smooth_blend_begin (cdecl f32 duration) --------------------------------------

/// Orientation -> quaternion into the blend record (0xC053E0, EAX = dest, EDI = the matrix),
/// position and fov copied, elapsed = 0, duration, type 7, flag 2 set, 0xF53AE0 cleared.
unsafe fn smooth_begin(dur: u32) {
    unsafe {
        call_regs(0x00C0_53E0, BLEND as u32, 0, 0, 0, 0x025F_5B38, &[]);
        wd(BLEND + 0x1C, q32(rd(0x025F_5BA4)));
        wd(BLEND + 0x20, 0);
        wd(CAM_FLAGS, rd(CAM_FLAGS) | 2);
        wd(BLEND + 0x24, q32(dur));
        copy_dwords(BLEND + 0x10, 0x025F_5B14, 3);
        wd(BLEND + 0x28, 7);
        wd(0x00F5_3AE0, 0);
    }
}

extern "C" fn smooth_entry(_r: &mut Pushad, args: *const u32) {
    let p = *_r;
    let dur = unsafe { *args };
    let Some((n, orig)) = shadowing(SMOOTH) else {
        unsafe { smooth_begin(dur) };
        return;
    };
    unsafe {
        shadow_state(
            SMOOTH,
            n,
            &MODE_RANGES,
            || {
                let mut rr = Regs::of(&p);
                call_full(orig, &mut rr, [dur, 0, 0, 0], false);
            },
            || smooth_begin(dur),
        );
    }
}
regs_thunk!(smooth_thunk, smooth_entry);

// --- 0x493EE0 camera_mode_set_base (EAX = mode, cdecl (u8 smooth, f32 duration)) ------------------

/// Record the base mode; with an empty stack and a different current mode, switch (clear flag 2,
/// optionally begin the smooth blend, which leaves EDX = 0x25F5B1C) and call the enter fn.
unsafe fn set_base(mode: u32, smooth: u8, dur: u32, ecx: u32, edx: u32) -> Plan {
    unsafe {
        wd(MODE_BASE, mode);
        if rd(MODE_COUNT) != 0 || rd(MODE_CUR) == mode {
            return Plan::default();
        }
        wd(CAM_FLAGS, rd(CAM_FLAGS) & !2);
        wd(MODE_CUR, mode);
        let mut edx = edx;
        let mut m = mode;
        if smooth != 0 {
            smooth_begin(dur);
            m = rd(MODE_CUR);
            edx = rd(0x025F_5B1C);
        }
        let f = mode_enter_fn(m);
        if f == 0 { Plan::default() } else { Plan { call: f, ecx, edx } }
    }
}

extern "C" fn set_base_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (smooth, dur) = unsafe { (*args as u8, *args.add(1)) };
    let plan = match shadowing(SET_BASE) {
        None => unsafe { set_base(p.eax, smooth, dur, p.ecx, p.edx) },
        Some((n, orig)) => unsafe {
            let old = rd(MODE_CUR);
            shadow_state(
                SET_BASE,
                n,
                &MODE_RANGES_BASE,
                || {
                    let mut rr = Regs::of(&p);
                    call_full(orig, &mut rr, [smooth as u32 | (*args & !0xFF), dur, 0, 0], false);
                    derived_plan(&rr, old)
                },
                || set_base(p.eax, smooth, dur, p.ecx, p.edx),
            )
        },
    };
    r.eax = plan.call;
    if plan.call != 0 {
        r.ecx = plan.ecx;
        r.edx = plan.edx;
    }
}
call_thunk!(set_base_thunk, set_base_entry);

const MODE_RANGES_BASE: [(usize, usize); 4] = MODE_RANGES;

// --- 0x49EEC0 camera_mode_set_base_default (no args) ----------------------------------------------

/// `set_base` with the mode 0x2526C20 and always a 0.4f blend ([0xDD1C50]).
unsafe fn set_base_default(ecx: u32) -> Plan {
    unsafe {
        let mode = rd(0x0252_6C20);
        wd(MODE_BASE, mode);
        if rd(MODE_COUNT) != 0 || rd(MODE_CUR) == mode {
            return Plan::default();
        }
        wd(CAM_FLAGS, rd(CAM_FLAGS) & !2);
        let dur = q32(rd(0x00DD_1C50));
        wd(MODE_CUR, mode);
        smooth_begin(dur);
        let m = rd(MODE_CUR);
        let f = mode_enter_fn(m);
        // The original's EDX here is 0x25F5B1C (the blend start's last load); ECX is the caller's.
        if f == 0 { Plan::default() } else { Plan { call: f, ecx, edx: rd(0x025F_5B1C) } }
    }
}

extern "C" fn set_base_default_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let plan = match shadowing(SET_BASE_DEF) {
        None => unsafe { set_base_default(p.ecx) },
        Some((n, orig)) => unsafe {
            let old = rd(MODE_CUR);
            shadow_state(
                SET_BASE_DEF,
                n,
                &MODE_RANGES_BASE,
                || {
                    let mut rr = Regs::of(&p);
                    call_full(orig, &mut rr, [0; 4], false);
                    derived_plan(&rr, old)
                },
                || set_base_default(p.ecx),
            )
        },
    };
    r.eax = plan.call;
    if plan.call != 0 {
        r.ecx = plan.ecx;
        r.edx = plan.edx;
    }
}
call_thunk!(set_base_default_thunk, set_base_default_entry);

// --- 0x4A1A50 angle_wrap_in_place (ECX = &f32) ------------------------------------------------------

/// `while v > pi { v = (f32)(v - 2pi) }` then `while v < -pi { v = (f32)(v + 2pi) }`, with the
/// double constants at 0xE5C160 (pi), 0xE5BF78 (2pi) and 0xE5C170 (-pi). A NaN skips both loops.
unsafe fn angle_wrap(p: usize) {
    unsafe {
        let pi = rd64(0x00E5_C160);
        let two_pi = rd64(0x00E5_BF78);
        let neg_pi = rd64(0x00E5_C170);
        let mut v = rf(p);
        if v as f64 > pi {
            loop {
                v = (v as f64 - two_pi) as f32;
                wf(p, v);
                if v as f64 <= pi || v.is_nan() {
                    break;
                }
            }
        }
        let v0 = rf(p);
        if (v0 as f64) < neg_pi {
            let mut v = v0;
            loop {
                v = (v as f64 + two_pi) as f32;
                wf(p, v);
                if (v as f64) >= neg_pi || v.is_nan() {
                    break;
                }
            }
        }
    }
}

extern "C" fn angle_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(ANGLE) else {
        unsafe { angle_wrap(p.ecx as usize) };
        return;
    };
    unsafe {
        let mut a = rd(p.ecx as usize);
        let mut b = a;
        let mut rr = Regs::of(&p);
        rr.ecx = &mut a as *mut u32 as u32;
        call_full(orig, &mut rr, [0; 4], false);
        angle_wrap(&mut b as *mut u32 as usize);
        if a != b {
            mismatch(ANGLE, n, format!("input {:#x}: original {a:#x}, Rust {b:#x}", rd(p.ecx as usize)));
        }
        wd(p.ecx as usize, a);
    }
}
regs_thunk!(angle_thunk, angle_entry);

// --- 0x49A670 camera_follow_commit_context (no args) ----------------------------------------------

const COMMIT_RANGES: [(usize, usize); 5] =
    [(CAM_FLAGS, 4), (0x00E9_A5C8, 4), (0x00E9_A5CC, 0x14), (0x00E9_A638, 0x2C), (0x0252_6B5A, 1)];

/// Flag 2 cleared, blend 1.0, the active params record (0xE9A638) loaded from the table row of
/// the context, and the previous-request fields saved.
unsafe fn commit_ctx() {
    unsafe {
        wd(CAM_FLAGS, rd(CAM_FLAGS) & !2);
        wd(0x00E9_A5C8, 0x3F80_0000);
        copy_dwords(0x00E9_A638, 0x025F_6318usize.wrapping_add((rd(0x00E9_A5BC) as usize).wrapping_mul(0x2C)), 11);
        wd(0x00E9_A5CC, rd(0x00E9_A5B8));
        wd(0x00E9_A5D0, rd(0x00E9_A5BC));
        wd(0x00E9_A5D4, rd(0x00E9_A5C0));
        wd(0x00E9_A5D8, rd(0x00E9_A5C4));
        wd(0x00E9_A5DC, rd(0x00E9_A5C8));
        wb(0x0252_6B5A, 1);
    }
}

extern "C" fn commit_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(COMMIT) else {
        unsafe { commit_ctx() };
        return;
    };
    unsafe {
        shadow_state(
            COMMIT,
            n,
            &COMMIT_RANGES,
            || {
                let mut rr = Regs::of(&p);
                call_full(orig, &mut rr, [0; 4], false);
            },
            || commit_ctx(),
        );
    }
}
regs_thunk!(commit_thunk, commit_entry);

// --- 0x49A6E0 camera_follow_set_target_context (EAX = handle, EDX = mode, cdecl-ish ret 8) ---------

const SET_TARGET_RANGES: [(usize, usize); 6] = [
    (0x00E9_A5B8, 0x28),
    (0x025F_5BB0, 0x3C),
    (0x00E9_A638, 0x2C),
    (CAM_FLAGS, 4),
    (0x0252_6B5A, 1),
    (0x00E9_A5C8, 4),
];

/// Store the target handle (also at 0x25F5BB0), snapshot the object's position / orientation (or
/// the origin / identity at 0x252A6D0 / 0xEC5AE4), store the context unless 0x19, then either
/// store the transition (mode EDX, duration > 0 or NaN) or commit.
unsafe fn set_target(h: u32, mode: u32, ctx: u32, dur: u32) {
    unsafe {
        wd(0x00E9_A5B8, h);
        wd(0x025F_5BB0, h);
        let mut obj = 0;
        if h != 0 {
            let o = slot_object(h);
            if o != 0 && rd(o as usize + 0x44) == h {
                obj = o as usize;
            }
        }
        let src;
        if obj != 0 {
            wd(0x025F_5BB4, rd(obj + 0x14));
            wd(0x025F_5BB8, rd(obj + 0x18));
            wd(0x025F_5BBC, rd(obj + 0x1C));
            src = obj + 0x20;
        } else {
            let x = rd(0x0252_A6D0);
            let y = rd(0x0252_A6D4);
            wd(0x025F_5BB4, x);
            let z = rd(0x0252_A6D8);
            wd(0x025F_5BB8, y);
            wd(0x025F_5BBC, z);
            src = 0x00EC_5AE4;
        }
        copy_dwords(0x025F_5BC0, src, 9);
        if ctx != 0x19 {
            wd(0x00E9_A5BC, ctx);
        }
        if mode != 0 {
            let d = f32::from_bits(dur);
            if d > 0.0 || d.is_nan() {
                wd(0x00E9_A5C4, q32(dur));
                wd(0x00E9_A5C0, mode);
                return;
            }
        }
        commit_ctx();
    }
}

extern "C" fn set_target_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (ctx, dur) = unsafe { (*args, *args.add(1)) };
    let Some((n, orig)) = shadowing(SET_TARGET) else {
        unsafe { set_target(p.eax, p.edx, ctx, dur) };
        return;
    };
    unsafe {
        shadow_state(
            SET_TARGET,
            n,
            &SET_TARGET_RANGES,
            || {
                let mut rr = Regs::of(&p);
                call_full(orig, &mut rr, [ctx, dur, 0, 0], false);
            },
            || set_target(p.eax, p.edx, ctx, dur),
        );
    }
}
regs_thunk!(set_target_thunk, set_target_entry, "ret 8");

// --- 0x499AF0 camera_follow_request_equals_current (ECX = &{handle, ctx, mode, dur, blend}) -------

/// `fld cur; fld req; fucompp; fnstsw ax`, as the original (the status word's TOP bits end up in
/// EAX).
unsafe fn fucompp_sw(cur: usize, req: usize) -> u32 {
    let sw: u32;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fld dword ptr [{b}]",
            "fucompp",
            "fnstsw ax",
            a = in(reg) cur,
            b = in(reg) req,
            out("eax") sw,
            clobber_abi("C"),
        );
    }
    sw & 0xFFFF
}


/// AL = all five fields equal (floats by `fucompp`; any NaN is unequal). EAX otherwise holds
/// what the original leaves: the last field loaded (handle / mode) with AL cleared, and after a
/// float compare the status word in AX with AL cleared (1 on equality).
unsafe fn request_equals(req: usize) -> u32 {
    unsafe {
        let h = rd(0x00E9_A5B8);
        if h != rd(req) {
            return h & !0xFF;
        }
        if rd(0x00E9_A5BC) != rd(req + 4) {
            return h & !0xFF;
        }
        let m = rd(0x00E9_A5C0);
        if m != rd(req + 8) {
            return m & !0xFF;
        }
        // `test ah, 0x44; jp`: the compare passes when exactly one of C3 / C2 is set.
        let sw = fucompp_sw(0x00E9_A5C4, req + 0xC);
        let e = (m & 0xFFFF_0000) | sw;
        if ((sw >> 8) & 0x44).count_ones() != 1 {
            return e & !0xFF;
        }
        let sw2 = fucompp_sw(0x00E9_A5C8, req + 0x10);
        let e2 = (m & 0xFFFF_0000) | sw2;
        if ((sw2 >> 8) & 0x44).count_ones() != 1 {
            return e2 & !0xFF;
        }
        lo(e2, true)
    }
}

extern "C" fn equals_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { request_equals(p.ecx as usize) };
    r.eax = ours;
    if let Some((n, orig)) = shadowing(EQUALS) {
        let mut rr = Regs::of(&p);
        unsafe { call_full(orig, &mut rr, [0; 4], false) };
        if rr.eax != ours {
            mismatch(EQUALS, n, format!("request {:#x}: original {:#x}, Rust {ours:#x}", p.ecx, rr.eax));
            r.eax = rr.eax;
        }
    }
}
regs_thunk!(equals_thunk, equals_entry);

// --- 0x49AB10 camera_human_context_classify (ESI = human) -----------------------------------------

/// One of the 0x49A8xx-0x49AAxx predicates; they read the human from ECX, EDX or EAX (all are
/// set), the answer is AL.
unsafe fn pred(f: usize, h: u32) -> bool {
    unsafe { call_regs(f, h, h, h, h, 0, &[]) as u8 != 0 }
}

/// The context 0-0x17 of a human, tests in the original's order (first hit wins).
unsafe fn classify(h: u32) -> u32 {
    unsafe {
        let hp = h as usize;
        if h != 0 && rd(hp + 0xE88) != 0 {
            return 0xA;
        }
        let s = rb(hp + 0x595);
        if s != 0 && s != 3 {
            return 0xD;
        }
        if rb(hp + 0x591) == 5 {
            return 0xC;
        }
        if pred(0x0049_A8C0, h) {
            return 0xE;
        }
        if pred(0x0049_A870, h) {
            return 9;
        }
        if rd(rd(hp + 0x5E0) as usize + 0x164) == 0x441 {
            return 0xF;
        }
        if pred(0x0049_A950, h) {
            return 0x11;
        }
        if pred(0x0049_A920, h) {
            return 0x10;
        }
        if pred(0x0049_A980, h) {
            return 0x12;
        }
        if pred(0x0049_A9B0, h) {
            return 0x16;
        }
        if pred(0x0049_AA00, h) {
            return 0x17;
        }
        if pred(0x0049_AA40, h) {
            return 2;
        }
        if pred(0x0049_AA80, h) {
            return 0x14;
        }
        if (rd(hp + 0xC0) >> 0x1E) & 1 != 0 {
            return 0x15;
        }
        call_regs(0x0049_EDB0, 0, 0, 0, 0, 0, &[])
    }
}

extern "C" fn classify_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { classify(p.esi) };
    r.eax = ours;
    if let Some((n, orig)) = shadowing(CLASSIFY) {
        let mut rr = Regs::of(&p);
        unsafe { call_full(orig, &mut rr, [0; 4], false) };
        if rr.eax != ours {
            mismatch(CLASSIFY, n, format!("human {:#x}: original {:#x}, Rust {ours:#x}", p.esi, rr.eax));
            r.eax = rr.eax;
        }
    }
}
regs_thunk!(classify_thunk, classify_entry);

// --- 0x49AAB0 camera_context_for_handle (ECX = handle) --------------------------------------------

/// The seat logic of a vehicle (0x49AC10): the player's seat entry compared with the sentinel
/// 0x33D630C, then by vehicle class 2 / 3 / 4 -> 8 / 6 / 5 (else [0xE99854]); a real occupant -> 7.
unsafe fn vehicle_context(obj: u32) -> u32 {
    unsafe {
        let player = rd(LOCAL_PLAYER) as usize;
        let info = rd(obj as usize + 0x84E4) as usize;
        let seat = rd(player + 0xD78) as usize;
        let class = rd(info + 0x9C);
        let entry = rd(info.wrapping_add(seat.wrapping_mul(4)).wrapping_add(0x6F4));
        if entry != rd(0x033D_630C) {
            return 7;
        }
        match class {
            2 => 8,
            3 => 6,
            4 => 5,
            _ => rd(0x00E9_9854),
        }
    }
}

/// 0x19 for a null handle or stale object; a live human -> classify; a live vehicle -> seat
/// logic; any other live object -> 0xB.
unsafe fn context_for_handle(h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return 0x19;
        }
        let o = slot_object(h);
        if o == 0 {
            return 0x19;
        }
        if rd(o as usize + 0x44) != h {
            return 0x19;
        }
        match rd(o as usize + 0x4C) {
            1 => classify(o),
            5 => vehicle_context(o),
            _ => 0xB,
        }
    }
}

extern "C" fn ctx_handle_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { context_for_handle(p.ecx) };
    r.eax = ours;
    if let Some((n, orig)) = shadowing(CTX_HANDLE) {
        let mut rr = Regs::of(&p);
        unsafe { call_full(orig, &mut rr, [0; 4], false) };
        if rr.eax != ours {
            mismatch(CTX_HANDLE, n, format!("handle {:#x}: original {:#x}, Rust {ours:#x}", p.ecx, rr.eax));
            r.eax = rr.eax;
        }
    }
}
regs_thunk!(ctx_handle_thunk, ctx_handle_entry);

// --- the shake ports ------------------------------------------------------------------------------

/// What `camera_shake_start` is called with: ECX and five stack dwords.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Shake {
    ecx: u32,
    a: [u32; 5],
}

static REC: [AtomicU32; 7] = [const { AtomicU32::new(0) }; 7];

extern "C" fn shake_record(ecx: u32, args: *const u32) {
    unsafe {
        REC[0].store(ecx, Ordering::Relaxed);
        for i in 0..5 {
            REC[1 + i].store(*args.add(i), Ordering::Relaxed);
        }
    }
    REC[6].store(1, Ordering::Relaxed);
}

/// Stands in for `camera_shake_start` in the shadow copies: records ECX and the stack.
#[unsafe(naked)]
extern "C" fn shake_stub() {
    std::arch::naked_asm!(
        "push eax", "push ecx", "push edx",
        "lea eax, [esp + 16]",
        "push eax", "push ecx",
        "call {f}",
        "add esp, 8",
        "pop edx", "pop ecx", "pop eax",
        "ret",
        f = sym shake_record,
    )
}

fn rec_reset() {
    REC[6].store(0, Ordering::Relaxed);
}

fn rec_take() -> Option<Shake> {
    if REC[6].load(Ordering::Relaxed) == 0 {
        return None;
    }
    let g = |i: usize| REC[i].load(Ordering::Relaxed);
    Some(Shake { ecx: g(0), a: [g(1), g(2), g(3), g(4), g(5)] })
}

unsafe fn shake_start(s: &Shake) {
    unsafe { call_regs(SHAKE_START, 0, s.ecx, 0, 0, 0, &s.a) };
}

// --- 0x4A0B60 camera_shake_start_preset (EAX = preset, cdecl f32 scale) ---------------------------

/// Preset record (24 bytes at 0xF539E8): the two floats scaled by `scale` (as f32), the rest
/// passed through. Returns the call and the value the original leaves in the caller's argument
/// slot (the scaled +0 float).
unsafe fn preset_args(preset: u32, scale: u32) -> (Shake, u32) {
    unsafe {
        let rec = 0x00F5_39E8usize.wrapping_add((preset as usize).wrapping_mul(0x18));
        let sc = f32::from_bits(scale) as f64;
        let t0 = (rf(rec + 4) as f64 * sc) as f32;
        let t1 = (sc * rf(rec) as f64) as f32;
        (Shake { ecx: rd(rec + 0x10), a: [t1.to_bits(), rd(rec + 8), rd(rec + 0xC), t0.to_bits(), rd(rec + 0x14)] }, t1.to_bits())
    }
}

extern "C" fn preset_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let scale = unsafe { *args };
    unsafe {
        let (mut s, t1) = preset_args(p.eax, scale);
        if let Some((n, orig)) = shadowing(PRESET) {
            rec_reset();
            let mut rr = Regs::of(&p);
            call_full(orig, &mut rr, [scale, 0, 0, 0], false);
            match rec_take() {
                Some(t) if t == s => {}
                other => {
                    mismatch(PRESET, n, format!("preset {}: original {other:x?}, Rust {s:x?}", p.eax));
                    if let Some(t) = other {
                        s = t;
                    }
                }
            }
        }
        shake_start(&s);
        // The original leaves the scaled float in the caller's argument slot.
        *(args as *mut u32) = t1;
    }
}
regs_thunk!(preset_thunk, preset_entry);

// --- 0x4A0BC0 camera_shake_from_world_point (EAX = &vec3, cdecl f32, f32) ------------------------

/// Distance from the player (0x21703D4) to the point (sum of squares stored as f32, through
/// sqrtf_wrapper); only under 30.0: x = (f32)(1 - d/30), ms = 200 - ftol(c * x), amplitude =
/// (f32)(x * (a1 * K / a2) + 0); shake_start(ECX = 0, amp, ms, 0x1F4, 0.075, 0).
unsafe fn from_point_args(pt: u32, a1: u32, a2: u32) -> Option<Shake> {
    unsafe {
        let pl = rd(LOCAL_PLAYER) as usize;
        let p = pt as usize;
        let dx = (rf(p) as f64 - rf(pl + 0x14) as f64) as f32;
        let dy = (rf(p + 4) as f64 - rf(pl + 0x18) as f64) as f32;
        let dz = (rf(p + 8) as f64 - rf(pl + 0x1C) as f64) as f32;
        let (dx, dy, dz) = (dx as f64, dy as f64, dz as f64);
        let sum = ((dx * dx + dy * dy) + dz * dz) as f32;
        let d = sqrtf(sum.to_bits());
        let k30 = rd64(0x00E5_BF68);
        if !(k30 > d as f64) {
            return None;
        }
        let x = (1.0 - d as f64 / k30) as f32;
        let t = ftol(rd64(0x00E5_CCA0) * x as f64);
        let ms = 0xC8u32.wrapping_sub(t);
        let q = f32::from_bits(a1) as f64 * rd64(0x00E5_C300) / f32::from_bits(a2) as f64;
        let amp = (x as f64 * (q - 0.0) + 0.0) as f32;
        Some(Shake { ecx: 0, a: [amp.to_bits(), ms, 0x1F4, q32(rd(0x00E9_9384)), 0] })
    }
}

extern "C" fn from_point_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (a1, a2) = unsafe { (*args, *args.add(1)) };
    unsafe {
        let mut s = from_point_args(p.eax, a1, a2);
        if let Some((n, orig)) = shadowing(FROM_POINT) {
            rec_reset();
            let mut rr = Regs::of(&p);
            call_full(orig, &mut rr, [a1, a2, 0, 0], false);
            let theirs = rec_take();
            if theirs != s {
                mismatch(FROM_POINT, n, format!("point {:#x}: original {theirs:x?}, Rust {s:x?}", p.eax));
                s = theirs;
            }
        }
        if let Some(s) = s {
            shake_start(&s);
        }
    }
}
regs_thunk!(from_point_thunk, from_point_entry);

// --- 0x499940 camera_params_set_polar (ESI = &params, cdecl-ish ret 8: f32 a, f32 b) --------------

/// d = (f32)(a - K[0xE5BFA0]); with d < 0.001 and -d < 0.001 (the constant at 0xDD5984): angle
/// and radius 0. Otherwise angle = (f32)atan-helper((f32)(b / a)), radius = sqrtf((f32)(b*b + a*a)).
unsafe fn polar(rec: usize, a: u32, b: u32) {
    unsafe {
        let (a, b) = (f32::from_bits(a) as f64, f32::from_bits(b) as f64);
        let d = (a - rd64(0x00E5_BFA0)) as f32 as f64;
        let thr = rf(0x00DD_5984) as f64;
        if d < thr && -d < thr {
            wf(rec + 0x18, 0.0);
            wf(rec + 0x1C, 0.0);
            return;
        }
        let q = (b / a) as f32;
        let ang = call_st0_in_out(ATAN_ST0, q as f64) as f32;
        wf(rec + 0x18, ang);
        let s = (b * b + a * a) as f32;
        wf(rec + 0x1C, sqrtf(s.to_bits()));
    }
}

extern "C" fn polar_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (a, b) = unsafe { (*args, *args.add(1)) };
    let Some((n, orig)) = shadowing(POLAR) else {
        unsafe { polar(p.esi as usize, a, b) };
        return;
    };
    let ranges = [(p.esi as usize + 0x18, 8)];
    unsafe {
        shadow_state(
            POLAR,
            n,
            &ranges,
            || {
                let mut rr = Regs::of(&p);
                call_full(orig, &mut rr, [a, b, 0, 0], false);
            },
            || polar(p.esi as usize, a, b),
        );
    }
}
regs_thunk!(polar_thunk, polar_entry, "ret 8");

// --- 0x6EF510 anim_state_find_by_name (cdecl name) ------------------------------------------------

/// -1 for no name; else the first of the 0x2A4 states (12-byte records at 0xE8EE90, hashes at
/// 0x28F05E8) whose hash matches `string_hash_lower(name)` and whose name is `_stricmp`-equal.
unsafe fn anim_find(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return u32::MAX;
        }
        let mut h = 0u32;
        call_regs(0x00BD_CB30, 0, name, 0, 0, 0, &[&mut h as *mut u32 as u32]);
        for i in 0..0x2A4usize {
            if rd(0x028F_05E8 + i * 4) == h && call_regs(0x00C9_9030, 0, 0, 0, 0, 0, &[name, rd(0x00E8_EE90 + i * 12)]) == 0 {
                return i as u32;
            }
        }
        u32::MAX
    }
}

extern "C" fn find_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let name = unsafe { *args };
    let ours = unsafe { anim_find(name) };
    r.eax = ours;
    if let Some((n, orig)) = shadowing(FIND) {
        let mut rr = Regs::of(&p);
        unsafe { call_full(orig, &mut rr, [name, 0, 0, 0], false) };
        if rr.eax != ours {
            mismatch(FIND, n, format!("original {:#x}, Rust {ours:#x}", rr.eax));
            r.eax = rr.eax;
        }
    }
}
regs_thunk!(find_thunk, find_entry);

// --- install --------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy.
unsafe fn set_rel32(copy: usize, off: usize, dest: usize) {
    let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
    unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
}

/// Overwrite two bytes of a copy (checking what was there).
unsafe fn patch2(copy: usize, off: usize, old: [u8; 2], new: [u8; 2]) -> bool {
    unsafe {
        let p = (copy + off) as *mut [u8; 2];
        if *p != old {
            return false;
        }
        *p = new;
        true
    }
}

fn copy_of(id: usize) -> usize {
    PORTS[id].original.load(Ordering::Relaxed)
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (MODE_PUSH, &PUSH_BODY, &PUSH_BODY_CALLS, mode_push_thunk),
        (MODE_POP, &POP_BODY, &POP_BODY_CALLS, mode_pop_thunk),
        (SET_BASE, &SET_BASE_BODY, &SET_BASE_BODY_CALLS, set_base_thunk),
        (SET_BASE_DEF, &SET_BASE_DEF_BODY, &SET_BASE_DEF_BODY_CALLS, set_base_default_thunk),
        (SMOOTH, &SMOOTH_BODY, &SMOOTH_BODY_CALLS, smooth_thunk),
        (ANGLE, &ANGLE_BODY, &ANGLE_BODY_CALLS, angle_thunk),
        (COMMIT, &COMMIT_BODY, &COMMIT_BODY_CALLS, commit_thunk),
        (SET_TARGET, &SET_TARGET_BODY, &SET_TARGET_BODY_CALLS, set_target_thunk),
        (EQUALS, &EQUALS_BODY, &EQUALS_BODY_CALLS, equals_thunk),
        (CTX_HANDLE, &CTX_HANDLE_BODY, &CTX_HANDLE_BODY_CALLS, ctx_handle_thunk),
        (CLASSIFY, &CLASSIFY_BODY, &CLASSIFY_BODY_CALLS, classify_thunk),
        (PRESET, &PRESET_BODY, &PRESET_BODY_CALLS, preset_thunk),
        (FROM_POINT, &FROM_POINT_BODY, &FROM_POINT_BODY_CALLS, from_point_thunk),
        (POLAR, &POLAR_BODY, &POLAR_BODY_CALLS, polar_thunk),
        (FIND, &FIND_BODY, &FIND_BODY_CALLS, find_thunk),
    ];
    // Shadow copies first (replacing patches the entries), then their fix-ups.
    for &(id, body, calls, _) in &list {
        match unsafe { detour::relocate(PORTS[id].addr, body, calls) } {
            Ok(copy) => PORTS[id].original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_camanim: {}: no shadow copy: {e:?}", PORTS[id].name)),
        }
    }
    let veh_tail = unsafe { detour::relocate(0x0049_AC10, &VEH_TAIL_BODY, &VEH_TAIL_BODY_CALLS) }.unwrap_or(0);
    unsafe {
        let fix = |id: usize, f: &dyn Fn(usize) -> bool| {
            let c = copy_of(id);
            if c != 0 && !f(c) {
                log(&format!("port_camanim: {}: shadow copy fix-up failed, shadow off", PORTS[id].name));
                PORTS[id].original.store(0, Ordering::Relaxed);
            }
        };
        // The tail `jmp eax` becomes `ret` (EAX = the enter fn it would jump to).
        fix(MODE_PUSH, &|c| patch2(c, 0x89, [0xFF, 0xE0], [0xC3, 0x90]));
        fix(MODE_POP, &|c| patch2(c, 0x9A, [0xFF, 0xE0], [0xC3, 0x90]));
        // `call eax` becomes two nops; the blend begin is the copy of the original.
        fix(SET_BASE, &|c| {
            set_rel32(c, 0x32, copy_of(SMOOTH));
            copy_of(SMOOTH) != 0 && patch2(c, 0x4D, [0xFF, 0xD0], [0x90, 0x90])
        });
        fix(SET_BASE_DEF, &|c| {
            set_rel32(c, 0x32, copy_of(SMOOTH));
            copy_of(SMOOTH) != 0 && patch2(c, 0x4D, [0xFF, 0xD0], [0x90, 0x90])
        });
        fix(SET_TARGET, &|c| {
            set_rel32(c, 0xB3, copy_of(COMMIT));
            copy_of(COMMIT) != 0
        });
        fix(CLASSIFY, &|c| {
            set_rel32(c, 0xF5, 0x0049_EDB0);
            true
        });
        fix(CTX_HANDLE, &|c| {
            set_rel32(c, 0x26, copy_of(CLASSIFY));
            set_rel32(c, 0x42, veh_tail);
            copy_of(CLASSIFY) != 0 && veh_tail != 0
        });
        fix(PRESET, &|c| {
            set_rel32(c, 0x4C, shake_stub as *const () as usize);
            true
        });
        fix(FROM_POINT, &|c| {
            set_rel32(c, 0xC7, shake_stub as *const () as usize);
            true
        });
    }
    let mut ok = 0;
    for &(id, body, _, thunk) in &list {
        if replace_pinned(PORTS[id].name, PORTS[id].addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_camanim: replaced {ok}/{COUNT} with Rust"));
}

/// One line for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_camanim: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const PUSH_BODY: [u8; 140] = [
    0x8B, 0x15, 0x0C, 0x5B, 0x5F, 0x02, 0x56, 0x8D, 0x72, 0xFF, 0x8B, 0xC6, 0x85, 0xC0, 0x7C, 0x47,
    0x39, 0x0C, 0x85, 0xEC, 0x5A, 0x5F, 0x02, 0x74, 0x07, 0x83, 0xE8, 0x01, 0x79, 0xF2, 0xEB, 0x37,
    0x3B, 0xC6, 0x7D, 0x2A, 0xEB, 0x0A, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x8B, 0x14, 0x85, 0xF0, 0x5A, 0x5F, 0x02, 0x89, 0x14, 0x85, 0xEC, 0x5A, 0x5F, 0x02, 0x8B, 0x15,
    0x0C, 0x5B, 0x5F, 0x02, 0x83, 0xC0, 0x01, 0x8D, 0x72, 0xFF, 0x3B, 0xC6, 0x7C, 0xE2, 0x83, 0xEA,
    0x01, 0x89, 0x15, 0x0C, 0x5B, 0x5F, 0x02, 0x89, 0x0C, 0x95, 0xEC, 0x5A, 0x5F, 0x02, 0x83, 0x05,
    0x0C, 0x5B, 0x5F, 0x02, 0x01, 0x39, 0x0D, 0xE8, 0x5A, 0x5F, 0x02, 0x5E, 0x74, 0x1D, 0x83, 0x25,
    0xE8, 0x5B, 0x5F, 0x02, 0xFD, 0x8D, 0x04, 0x89, 0x8B, 0x04, 0x85, 0x50, 0x52, 0xDD, 0x00, 0x85,
    0xC0, 0x89, 0x0D, 0xE8, 0x5A, 0x5F, 0x02, 0x74, 0x02, 0xFF, 0xE0, 0xC3,
];
const PUSH_BODY_CALLS: [usize; 0] = [];
const POP_BODY: [u8; 157] = [
    0x8B, 0x0D, 0x0C, 0x5B, 0x5F, 0x02, 0x85, 0xC9, 0x0F, 0x8E, 0x8E, 0x00, 0x00, 0x00, 0x39, 0x14,
    0x8D, 0xE8, 0x5A, 0x5F, 0x02, 0x56, 0x8B, 0x35, 0xE8, 0x5A, 0x5F, 0x02, 0x74, 0x40, 0x8D, 0x41,
    0xFE, 0x85, 0xC0, 0x7C, 0x42, 0x39, 0x14, 0x85, 0xEC, 0x5A, 0x5F, 0x02, 0x74, 0x07, 0x83, 0xE8,
    0x01, 0x79, 0xF2, 0xEB, 0x32, 0x8D, 0x51, 0xFF, 0x3B, 0xC2, 0x7D, 0x22, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x0C, 0x85, 0xF0, 0x5A, 0x5F, 0x02, 0x89, 0x0C, 0x85, 0xEC, 0x5A, 0x5F, 0x02, 0x8B, 0x0D,
    0x0C, 0x5B, 0x5F, 0x02, 0x83, 0xC0, 0x01, 0x8D, 0x51, 0xFF, 0x3B, 0xC2, 0x7C, 0xE2, 0x83, 0xE9,
    0x01, 0x89, 0x0D, 0x0C, 0x5B, 0x5F, 0x02, 0x85, 0xC9, 0x75, 0x08, 0x8B, 0x0D, 0x10, 0x5B, 0x5F,
    0x02, 0xEB, 0x07, 0x8B, 0x0C, 0x8D, 0xE8, 0x5A, 0x5F, 0x02, 0x83, 0x25, 0xE8, 0x5B, 0x5F, 0x02,
    0xFD, 0x3B, 0xCE, 0x89, 0x0D, 0xE8, 0x5A, 0x5F, 0x02, 0x5E, 0x74, 0x10, 0x8D, 0x04, 0x89, 0x8B,
    0x04, 0x85, 0x50, 0x52, 0xDD, 0x00, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xE0, 0xC3,
];
const POP_BODY_CALLS: [usize; 0] = [];
const SET_BASE_BODY: [u8; 81] = [
    0x51, 0x83, 0x3D, 0x0C, 0x5B, 0x5F, 0x02, 0x00, 0xA3, 0x10, 0x5B, 0x5F, 0x02, 0x75, 0x40, 0x39,
    0x05, 0xE8, 0x5A, 0x5F, 0x02, 0x74, 0x38, 0x83, 0x25, 0xE8, 0x5B, 0x5F, 0x02, 0xFD, 0x80, 0x7C,
    0x24, 0x08, 0x00, 0xA3, 0xE8, 0x5A, 0x5F, 0x02, 0x74, 0x15, 0xD9, 0x44, 0x24, 0x0C, 0x51, 0xD9,
    0x1C, 0x24, 0xE8, 0xD9, 0xF4, 0x00, 0x00, 0xA1, 0xE8, 0x5A, 0x5F, 0x02, 0x83, 0xC4, 0x04, 0x8D,
    0x04, 0x80, 0x8B, 0x04, 0x85, 0x50, 0x52, 0xDD, 0x00, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xD0, 0x59,
    0xC3,
];
const SET_BASE_BODY_CALLS: [usize; 1] = [0x32];
const SET_BASE_DEF_BODY: [u8; 81] = [
    0x51, 0x83, 0x3D, 0x0C, 0x5B, 0x5F, 0x02, 0x00, 0xA1, 0x20, 0x6C, 0x52, 0x02, 0xA3, 0x10, 0x5B,
    0x5F, 0x02, 0x75, 0x3B, 0x39, 0x05, 0xE8, 0x5A, 0x5F, 0x02, 0x74, 0x33, 0xD9, 0x05, 0x50, 0x1C,
    0xDD, 0x00, 0x83, 0x25, 0xE8, 0x5B, 0x5F, 0x02, 0xFD, 0x51, 0xD9, 0x1C, 0x24, 0xA3, 0xE8, 0x5A,
    0x5F, 0x02, 0xE8, 0xF9, 0x44, 0x00, 0x00, 0xA1, 0xE8, 0x5A, 0x5F, 0x02, 0x8D, 0x04, 0x80, 0x8B,
    0x04, 0x85, 0x50, 0x52, 0xDD, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x02, 0xFF, 0xD0, 0x59,
    0xC3,
];
const SET_BASE_DEF_BODY_CALLS: [usize; 1] = [0x32];
const SMOOTH_BODY: [u8; 111] = [
    0x51, 0x57, 0xBF, 0x38, 0x5B, 0x5F, 0x02, 0xB8, 0x90, 0x69, 0x5F, 0x02, 0xE8, 0xDF, 0x1F, 0x76,
    0x00, 0xD9, 0x05, 0xA4, 0x5B, 0x5F, 0x02, 0xA1, 0x14, 0x5B, 0x5F, 0x02, 0xD9, 0x1D, 0xAC, 0x69,
    0x5F, 0x02, 0xD9, 0xEE, 0x8B, 0x0D, 0x18, 0x5B, 0x5F, 0x02, 0x8B, 0x15, 0x1C, 0x5B, 0x5F, 0x02,
    0xD9, 0x1D, 0xB0, 0x69, 0x5F, 0x02, 0xD9, 0x44, 0x24, 0x0C, 0x83, 0x0D, 0xE8, 0x5B, 0x5F, 0x02,
    0x02, 0xD9, 0x1D, 0xB4, 0x69, 0x5F, 0x02, 0xA3, 0xA0, 0x69, 0x5F, 0x02, 0x89, 0x0D, 0xA4, 0x69,
    0x5F, 0x02, 0x89, 0x15, 0xA8, 0x69, 0x5F, 0x02, 0xC7, 0x05, 0xB8, 0x69, 0x5F, 0x02, 0x07, 0x00,
    0x00, 0x00, 0xC7, 0x05, 0xE0, 0x3A, 0xF5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5F, 0x59, 0xC3,
];
const SMOOTH_BODY_CALLS: [usize; 1] = [0xc];
const ANGLE_BODY: [u8; 85] = [
    0xD9, 0x01, 0xDD, 0x05, 0x60, 0xC1, 0xE5, 0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4,
    0x05, 0xDD, 0x05, 0x78, 0xBF, 0xE5, 0x00, 0x7A, 0x11, 0xD9, 0x01, 0xD8, 0xE1, 0xD9, 0x19, 0xD9,
    0x01, 0xD8, 0xDA, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x74, 0xEF, 0xDD, 0xD9, 0xD9, 0x01, 0xDD, 0x05,
    0x70, 0xC1, 0xE5, 0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0x75, 0x11, 0xD9,
    0x01, 0xD8, 0xC2, 0xD9, 0x19, 0xD9, 0x01, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0xEF,
    0xDD, 0xD9, 0xDD, 0xD8, 0xC3,
];
const ANGLE_BODY_CALLS: [usize; 0] = [];
const COMMIT_BODY: [u8; 110] = [
    0xD9, 0xE8, 0x83, 0x25, 0xE8, 0x5B, 0x5F, 0x02, 0xFD, 0x56, 0xD9, 0x1D, 0xC8, 0xA5, 0xE9, 0x00,
    0x8B, 0x35, 0xBC, 0xA5, 0xE9, 0x00, 0x6B, 0xF6, 0x2C, 0x57, 0x81, 0xC6, 0x18, 0x63, 0x5F, 0x02,
    0xB9, 0x0B, 0x00, 0x00, 0x00, 0xBF, 0x38, 0xA6, 0xE9, 0x00, 0xF3, 0xA5, 0xA1, 0xB8, 0xA5, 0xE9,
    0x00, 0x8B, 0x0D, 0xBC, 0xA5, 0xE9, 0x00, 0x8B, 0x15, 0xC0, 0xA5, 0xE9, 0x00, 0xA3, 0xCC, 0xA5,
    0xE9, 0x00, 0xA1, 0xC4, 0xA5, 0xE9, 0x00, 0x89, 0x0D, 0xD0, 0xA5, 0xE9, 0x00, 0x8B, 0x0D, 0xC8,
    0xA5, 0xE9, 0x00, 0x5F, 0x89, 0x15, 0xD4, 0xA5, 0xE9, 0x00, 0xA3, 0xD8, 0xA5, 0xE9, 0x00, 0x89,
    0x0D, 0xDC, 0xA5, 0xE9, 0x00, 0xC6, 0x05, 0x5A, 0x6B, 0x52, 0x02, 0x01, 0x5E, 0xC3,
];
const COMMIT_BODY_CALLS: [usize; 0] = [];
const SET_TARGET_BODY: [u8; 187] = [
    0x85, 0xC0, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x57, 0xA3, 0xB8, 0xA5, 0xE9, 0x00, 0xA3, 0xB0,
    0x5B, 0x5F, 0x02, 0x74, 0x37, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04,
    0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x22, 0x39, 0x41, 0x44, 0x75, 0x1D, 0x8B,
    0x41, 0x14, 0xA3, 0xB4, 0x5B, 0x5F, 0x02, 0x8B, 0x41, 0x18, 0xA3, 0xB8, 0x5B, 0x5F, 0x02, 0x8B,
    0x41, 0x1C, 0xA3, 0xBC, 0x5B, 0x5F, 0x02, 0x8D, 0x71, 0x20, 0xEB, 0x27, 0x8B, 0x0D, 0xD0, 0xA6,
    0x52, 0x02, 0xA1, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x0D, 0xB4, 0x5B, 0x5F, 0x02, 0x8B, 0x0D, 0xD8,
    0xA6, 0x52, 0x02, 0xA3, 0xB8, 0x5B, 0x5F, 0x02, 0x89, 0x0D, 0xBC, 0x5B, 0x5F, 0x02, 0xBE, 0xE4,
    0x5A, 0xEC, 0x00, 0x83, 0xFB, 0x19, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xBF, 0xC0, 0x5B, 0x5F, 0x02,
    0xF3, 0xA5, 0x74, 0x06, 0x89, 0x1D, 0xBC, 0xA5, 0xE9, 0x00, 0x85, 0xD2, 0x5F, 0x5E, 0x5B, 0x74,
    0x22, 0xD9, 0xEE, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41,
    0x7B, 0x0F, 0xD9, 0x1D, 0xC4, 0xA5, 0xE9, 0x00, 0x89, 0x15, 0xC0, 0xA5, 0xE9, 0x00, 0xC2, 0x08,
    0x00, 0xDD, 0xD8, 0xE8, 0xD8, 0xFE, 0xFF, 0xFF, 0xC2, 0x08, 0x00,
];
const SET_TARGET_BODY_CALLS: [usize; 1] = [0xb3];
const EQUALS_BODY: [u8; 72] = [
    0xA1, 0xB8, 0xA5, 0xE9, 0x00, 0x3B, 0x01, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x15, 0xBC, 0xA5,
    0xE9, 0x00, 0x3B, 0x51, 0x04, 0x75, 0xF2, 0xA1, 0xC0, 0xA5, 0xE9, 0x00, 0x3B, 0x41, 0x08, 0x75,
    0xE8, 0xD9, 0x05, 0xC4, 0xA5, 0xE9, 0x00, 0xD9, 0x41, 0x0C, 0xDA, 0xE9, 0xDF, 0xE0, 0xF6, 0xC4,
    0x44, 0x7A, 0xD6, 0xD9, 0x05, 0xC8, 0xA5, 0xE9, 0x00, 0xD9, 0x41, 0x10, 0xDA, 0xE9, 0xDF, 0xE0,
    0xF6, 0xC4, 0x44, 0x7A, 0xC4, 0xB0, 0x01, 0xC3,
];
const EQUALS_BODY_CALLS: [usize; 0] = [];
const CTX_HANDLE_BODY: [u8; 91] = [
    0x85, 0xC9, 0x74, 0x51, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x3D, 0x39, 0x48, 0x44, 0x75, 0x10, 0x83, 0x78, 0x4C,
    0x01, 0x75, 0x0A, 0x56, 0x8B, 0xF0, 0xE8, 0x35, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x85, 0xC0, 0x74,
    0x24, 0x39, 0x48, 0x44, 0x75, 0x11, 0x83, 0x78, 0x4C, 0x05, 0x75, 0x0B, 0x8B, 0x15, 0xD4, 0x03,
    0x17, 0x02, 0xE9, 0x19, 0x01, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x0A, 0x39, 0x48, 0x44, 0xB8, 0x0B,
    0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x19, 0x00, 0x00, 0x00, 0xC3,
];
const CTX_HANDLE_BODY_CALLS: [usize; 1] = [0x26];
const CLASSIFY_BODY: [u8; 250] = [
    0x85, 0xF6, 0x74, 0x0F, 0x83, 0xBE, 0x88, 0x0E, 0x00, 0x00, 0x00, 0x74, 0x06, 0xB8, 0x0A, 0x00,
    0x00, 0x00, 0xC3, 0x8A, 0x86, 0x95, 0x05, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x0A, 0x3C, 0x03, 0x74,
    0x06, 0xB8, 0x0D, 0x00, 0x00, 0x00, 0xC3, 0x80, 0xBE, 0x91, 0x05, 0x00, 0x00, 0x05, 0x75, 0x06,
    0xB8, 0x0C, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0xD6, 0xE8, 0x73, 0xFD, 0xFF, 0xFF, 0x84, 0xC0, 0x74,
    0x06, 0xB8, 0x0E, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0xCE, 0xE8, 0x12, 0xFD, 0xFF, 0xFF, 0x84, 0xC0,
    0x74, 0x06, 0xB8, 0x09, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x86, 0xE0, 0x05, 0x00, 0x00, 0x81, 0xB8,
    0x64, 0x01, 0x00, 0x00, 0x41, 0x04, 0x00, 0x00, 0x75, 0x06, 0xB8, 0x0F, 0x00, 0x00, 0x00, 0xC3,
    0xE8, 0xCB, 0xFD, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x11, 0x00, 0x00, 0x00, 0xC3, 0x8B,
    0xCE, 0xE8, 0x8A, 0xFD, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x10, 0x00, 0x00, 0x00, 0xC3,
    0xE8, 0xDB, 0xFD, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x12, 0x00, 0x00, 0x00, 0xC3, 0xE8,
    0xFC, 0xFD, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x16, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0xC6,
    0xE8, 0x3B, 0xFE, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x17, 0x00, 0x00, 0x00, 0xC3, 0x8B,
    0xC6, 0xE8, 0x6A, 0xFE, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3,
    0x8B, 0xC6, 0xE8, 0x99, 0xFE, 0xFF, 0xFF, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x14, 0x00, 0x00, 0x00,
    0xC3, 0x8B, 0x8E, 0xC0, 0x00, 0x00, 0x00, 0xC1, 0xE9, 0x1E, 0xF6, 0xC1, 0x01, 0x74, 0x06, 0xB8,
    0x15, 0x00, 0x00, 0x00, 0xC3, 0xE9, 0xA6, 0x41, 0x00, 0x00,
];
const CLASSIFY_BODY_CALLS: [usize; 9] = [0x38, 0x49, 0x70, 0x81, 0x90, 0x9f, 0xb0, 0xc1, 0xd2];
const PRESET_BODY: [u8; 85] = [
    0x8D, 0x04, 0x40, 0xD9, 0x04, 0xC5, 0xEC, 0x39, 0xF5, 0x00, 0x8B, 0x0C, 0xC5, 0xFC, 0x39, 0xF5,
    0x00, 0xD9, 0x44, 0x24, 0x04, 0x8B, 0x14, 0xC5, 0xF4, 0x39, 0xF5, 0x00, 0x8D, 0x04, 0xC5, 0xE8,
    0x39, 0xF5, 0x00, 0xD9, 0xC0, 0xDE, 0xCA, 0x51, 0x51, 0xD9, 0xC9, 0x8B, 0x48, 0x08, 0xD9, 0x5C,
    0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x1C, 0x24, 0x52, 0x51, 0xD8, 0x08, 0x51, 0x8B, 0x48,
    0x10, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0x1C, 0x24, 0xE8, 0x5F, 0xFE, 0xFF,
    0xFF, 0x83, 0xC4, 0x14, 0xC3,
];
const PRESET_BODY_CALLS: [usize; 1] = [0x4c];
const FROM_POINT_BODY: [u8; 219] = [
    0x83, 0xEC, 0x14, 0x8B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0xD9, 0x00, 0xD8, 0x61, 0x14, 0x51, 0xD9,
    0x5C, 0x24, 0x0C, 0xD9, 0x40, 0x04, 0xD8, 0x61, 0x18, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x40, 0x08,
    0xD8, 0x61, 0x1C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x10, 0xD9,
    0x44, 0x24, 0x14, 0xD9, 0xC1, 0xDE, 0xCA, 0xD9, 0xC2, 0xDE, 0xCB, 0xD9, 0xC9, 0xDE, 0xC2, 0xDC,
    0xC8, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8, 0x0D,
    0xDE, 0xF6, 0xFF, 0xD9, 0x5C, 0x24, 0x08, 0x83, 0xC4, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xDD, 0x05,
    0x68, 0xBF, 0xE5, 0x00, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x66, 0xDE, 0xF9, 0x6A,
    0x00, 0x51, 0xD9, 0xE8, 0xDE, 0xE1, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x05, 0x84, 0x93, 0xE9, 0x00,
    0xD9, 0x1C, 0x24, 0x68, 0xF4, 0x01, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x10, 0xDD, 0x05, 0xA0, 0xCC,
    0xE5, 0x00, 0xD8, 0xC9, 0xE8, 0xE7, 0xF4, 0x8F, 0x00, 0xD9, 0x44, 0x24, 0x24, 0xDC, 0x0D, 0x00,
    0xC3, 0xE5, 0x00, 0xB9, 0xC8, 0x00, 0x00, 0x00, 0x2B, 0xC8, 0x51, 0xD8, 0x74, 0x24, 0x2C, 0x51,
    0x33, 0xC9, 0xD9, 0xEE, 0xDC, 0xE9, 0xD9, 0xC9, 0xDE, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x18,
    0xD9, 0x44, 0x24, 0x18, 0xD9, 0x1C, 0x24, 0xE8, 0x84, 0xFD, 0xFF, 0xFF, 0x83, 0xC4, 0x14, 0x83,
    0xC4, 0x14, 0xC3, 0xDD, 0xD9, 0xDD, 0xD8, 0x83, 0xC4, 0x14, 0xC3,
];
const FROM_POINT_BODY_CALLS: [usize; 3] = [0x4e, 0x94, 0xc7];
const POLAR_BODY: [u8; 144] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x08, 0xD9, 0x45, 0x08, 0xDD, 0x05, 0xA0, 0xBF,
    0xE5, 0x00, 0xD8, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x05, 0x84, 0x59, 0xDD, 0x00, 0xD9, 0x44,
    0x24, 0x04, 0xD8, 0xD1, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x1B, 0xD9, 0xE0, 0xDE, 0xD9, 0xDF,
    0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x14, 0xDD, 0xD8, 0xD9, 0xEE, 0xD9, 0x56, 0x18, 0xD9, 0x5E, 0x1C,
    0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00, 0xDD, 0xD9, 0xDD, 0xD8, 0xD8, 0x7D, 0x0C, 0xD9, 0x5C, 0x24,
    0x04, 0xD9, 0x44, 0x24, 0x04, 0xE8, 0x26, 0x74, 0x90, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0x51, 0xD9, 0x5E, 0x18, 0xD9, 0x45, 0x08, 0xD9, 0x45, 0x0C, 0xDC, 0xC8, 0xD9, 0xC1,
    0xDE, 0xCA, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x1C, 0x24, 0xE8,
    0x5C, 0x50, 0xF7, 0xFF, 0xD9, 0x5E, 0x1C, 0x83, 0xC4, 0x04, 0x8B, 0xE5, 0x5D, 0xC2, 0x08, 0x00,
];
const POLAR_BODY_CALLS: [usize; 2] = [0x55, 0x7f];
const FIND_BODY: [u8; 102] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x85, 0xDB, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0x5B, 0xC3, 0x55, 0x56,
    0x57, 0x8D, 0x44, 0x24, 0x14, 0x50, 0x33, 0xC0, 0x8B, 0xCB, 0xE8, 0x01, 0xD6, 0x4E, 0x00, 0x8B,
    0x6C, 0x24, 0x14, 0x33, 0xF6, 0xBF, 0x90, 0xEE, 0xE8, 0x00, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x3B, 0x2C, 0xB5, 0xE8, 0x05, 0x8F, 0x02, 0x75, 0x10, 0x8B, 0x0F, 0x51, 0x53, 0xE8, 0xDE, 0x9A,
    0x5A, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x16, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x0C, 0x81,
    0xFE, 0xA4, 0x02, 0x00, 0x00, 0x7C, 0xD9, 0x5F, 0x5E, 0x5D, 0x83, 0xC8, 0xFF, 0x5B, 0xC3, 0x5F,
    0x8B, 0xC6, 0x5E, 0x5D, 0x5B, 0xC3,
];
const FIND_BODY_CALLS: [usize; 2] = [0x1a, 0x3d];
const VEH_TAIL_BODY: [u8; 87] = [
    0x85, 0xC0, 0x75, 0x1A, 0x83, 0xC9, 0xFF, 0x8B, 0xC1, 0x83, 0xE8, 0x02, 0x74, 0x43, 0x83, 0xE8,
    0x01, 0x74, 0x38, 0x83, 0xE8, 0x01, 0x74, 0x2D, 0xA1, 0x54, 0x98, 0xE9, 0x00, 0xC3, 0x8B, 0x80,
    0xE4, 0x84, 0x00, 0x00, 0x8B, 0x92, 0x78, 0x0D, 0x00, 0x00, 0x8B, 0x88, 0x9C, 0x00, 0x00, 0x00,
    0x8B, 0x84, 0x90, 0xF4, 0x06, 0x00, 0x00, 0x3B, 0x05, 0x0C, 0x63, 0x3D, 0x03, 0x74, 0xC8, 0xB8,
    0x07, 0x00, 0x00, 0x00, 0xC3, 0xB8, 0x05, 0x00, 0x00, 0x00, 0xC3, 0xB8, 0x06, 0x00, 0x00, 0x00,
    0xC3, 0xB8, 0x08, 0x00, 0x00, 0x00, 0xC3,
];
const VEH_TAIL_BODY_CALLS: [usize; 0] = [];
