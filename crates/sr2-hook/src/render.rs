//! The D3D command executor, docs/analysis/07_render.md: `gfx_execute_commands`
//! (0xD20360) drains the command ring on the device thread and calls Direct3D, and
//! `gfx_command_buffer_wait` (0xD1C780) rewinds the ring for the writers.
//!
//! Verified in `Mode::Shadow`: the original runs live with the device pointer aimed
//! at a recording proxy, recording every Direct3D call it makes (method, device, arguments,
//! and the contents of pointer arguments); its Windows calls (QueryPerformanceCounter
//! / Frequency, Sleep) and `gfx_check_device`'s outcome go on a tape. Then the Rust
//! executor runs "dry" over the same stretch of the ring: it makes no calls, records
//! the ones it would make, takes the Windows results from the tape and keeps its
//! global writes in an overlay. Both call lists and the globals are compared. The
//! ring stretch cannot be rewound meanwhile: the rewind takes the same lock.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

use crate::{detour, log};

#[derive(PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)] // one mode is used per build
enum Mode {
    Original,
    /// The original runs live, the Rust dry; compared.
    Shadow,
    Rust,
}

/// Shadow for test runs; Rust otherwise.
const MODE: Mode = Mode::Rust;

// The ring.
const RING_BASE: usize = 0x033D_62EC;
const RING_PUBLISHED: usize = 0x033D_62F4;
const RING_WRITE: usize = 0x033D_62F8;
const RING_LAST_PUBLISHED: usize = 0x033D_62FC;
const RING_READ: usize = 0x033D_6300;

// Devices.
const DEVICE: usize = 0x0252_A2D0; // the one commands use: real, or the null device while lost
const REAL_DEVICE: usize = 0x0252_A2D4;

// Executor state.
const VB_CACHE: usize = 0x022F_63F4;
const IB_CACHE: usize = 0x022F_63F0;
const TEXTURE_CACHE: usize = 0x022F_5BF0; // [stage]
const FLOAT_TEXTURES: usize = 0x022F_FCC8; // [slot], 512x1 A32B32G32R32F
const T_PRESENT_PREV: usize = 0x034A_50E8; // i64 QPC at the last Present
const T_PRESENT_NOW: usize = 0x034A_50F0;
const T_OVERLAY_PREV: usize = 0x034A_50F8;
const T_OVERLAY_NOW: usize = 0x034A_5100;
const T_IDLE: usize = 0x034A_5108; // i64 QPC ticks between executor runs
const T_END: usize = 0x034A_5110;
const T_START: usize = 0x034A_5118;
const T_FREQ: usize = 0x034A_5120;
const FPS_OVERLAY: usize = 0x0252_734B;
const FPS_FONT: usize = 0x0252_A2C8;
const RELEASE_CS: u32 = 0x0230_1CCC;
const RELEASE_LIST: usize = 0x0252_A2BC; // std::vector of objects to Release at EndScene
const RELEASE_LIST_2: usize = 0x0230_1CC8;

// Import slots.
const SLOT_QPC: usize = 0x00DB_8114;
const SLOT_QPF: usize = 0x00DB_8110;
const SLOT_SLEEP: usize = 0x00DB_80B8;
const SLOT_ENTER_CS: usize = 0x00DB_80CC;
const SLOT_LEAVE_CS: usize = 0x00DB_80D0;
const SLOT_SET_RECT: usize = 0x00DB_82D8;
const SLOT_OFFSET_RECT: usize = 0x00DB_82D4;

// Game functions.
const CHECK_DEVICE: usize = 0x00D2_0230;
const CREATE_FLOAT_TEXTURE: usize = 0x00D1_95E0; // EDX = &slot, cdecl (w, h, levels, usage, format)
const READBACK: usize = 0x00D1_C530; // cdecl (arg, 0, 0)
const RELEASE_LIST_CLEAR: usize = 0x00BD_4A80;
const RELEASE_LIST_2_CLEAR: usize = 0x00BD_4AD0;
const D3DX_CREATE_FONT: usize = 0x00CC_D522;
const SPRINTF: usize = 0x00C9_68F6; // cdecl (buf, fmt, ...)

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}
unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}
unsafe fn slot<T: Copy>(s: usize) -> T {
    unsafe {
        let p = rd(s) as usize;
        std::mem::transmute_copy(&p)
    }
}

/// Call a COM method (stdcall, `this` first).
unsafe fn com_call(f: usize, this: u32, a: &[u32]) -> u32 {
    unsafe {
        match *a {
            [] => std::mem::transmute::<usize, extern "system" fn(u32) -> u32>(f)(this),
            [a0] => std::mem::transmute::<usize, extern "system" fn(u32, u32) -> u32>(f)(this, a0),
            [a0, a1] => std::mem::transmute::<usize, extern "system" fn(u32, u32, u32) -> u32>(f)(this, a0, a1),
            [a0, a1, a2] => {
                std::mem::transmute::<usize, extern "system" fn(u32, u32, u32, u32) -> u32>(f)(this, a0, a1, a2)
            }
            [a0, a1, a2, a3] => std::mem::transmute::<usize, extern "system" fn(u32, u32, u32, u32, u32) -> u32>(f)(
                this, a0, a1, a2, a3,
            ),
            [a0, a1, a2, a3, a4] => {
                std::mem::transmute::<usize, extern "system" fn(u32, u32, u32, u32, u32, u32) -> u32>(f)(
                    this, a0, a1, a2, a3, a4,
                )
            }
            [a0, a1, a2, a3, a4, a5] => std::mem::transmute::<
                usize,
                extern "system" fn(u32, u32, u32, u32, u32, u32, u32) -> u32,
            >(f)(this, a0, a1, a2, a3, a4, a5),
            _ => unreachable!(),
        }
    }
}

// --- Direct3D methods the executor calls (IDirect3DDevice9 vtable slots) -------------------

const PRESENT: u32 = 17;
const UPDATE_SURFACE: u32 = 30;
const GET_RENDER_TARGET_DATA: u32 = 32;
const STRETCH_RECT: u32 = 34;
const SET_RENDER_TARGET: u32 = 37;
const SET_DEPTH_STENCIL_SURFACE: u32 = 39;
const BEGIN_SCENE: u32 = 41;
const END_SCENE: u32 = 42;
const CLEAR: u32 = 43;
const SET_VIEWPORT: u32 = 47;
const SET_RENDER_STATE: u32 = 57;
const SET_TEXTURE: u32 = 65;
const SET_SAMPLER_STATE: u32 = 69;
const SET_SCISSOR_RECT: u32 = 75;
const DRAW_PRIMITIVE: u32 = 81;
const DRAW_INDEXED_PRIMITIVE: u32 = 82;
const DRAW_PRIMITIVE_UP: u32 = 83;
const SET_VERTEX_DECLARATION: u32 = 87;
const SET_VERTEX_SHADER: u32 = 92;
const SET_VERTEX_SHADER_CONSTANT_F: u32 = 94;
const SET_STREAM_SOURCE: u32 = 100;
const SET_INDICES: u32 = 104;
const SET_PIXEL_SHADER: u32 = 107;
const SET_PIXEL_SHADER_CONSTANT_F: u32 = 109;

/// `gfx_prim_vertex_count` (0xCF5900; ECX = type, EAX = count): points n, lines 2n,
/// line strip n+1, triangles 3n, strip n+2, and fan n+1 (sic); else -1.
fn prim_vertex_count(kind: u32, n: u32) -> u32 {
    match kind {
        1 => n,
        2 => n.wrapping_add(n),
        3 | 6 => n.wrapping_add(1),
        4 => n.wrapping_mul(3),
        5 => n.wrapping_add(2),
        _ => u32::MAX,
    }
}

fn fnv(p: u32, len: usize) -> u32 {
    let mut h = 0x811C_9DC5u32;
    for &b in unsafe { std::slice::from_raw_parts(p as *const u8, len) } {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

/// A call as compared: method, device, then the arguments with every pointer
/// argument replaced by a hash of what it points at (0 for NULL).
fn record_of(method: u32, this: u32, a: &[u32]) -> Vec<u32> {
    let mut r = vec![method, this];
    r.extend_from_slice(a);
    let mut hash = |i: usize, len: usize| {
        let p = a[i];
        r[2 + i] = if p == 0 { 0 } else { fnv(p, len) | 1 };
    };
    match method {
        CLEAR => hash(1, a[0] as usize * 16),
        SET_VERTEX_SHADER_CONSTANT_F | SET_PIXEL_SHADER_CONSTANT_F => hash(1, a[2] as usize * 16),
        DRAW_PRIMITIVE_UP => {
            let n = prim_vertex_count(a[0], a[1]);
            if n != u32::MAX {
                hash(2, n as usize * a[3] as usize);
            }
        }
        STRETCH_RECT => {
            hash(1, 16);
            hash(3, 16);
        }
        SET_SCISSOR_RECT => hash(0, 16),
        SET_VIEWPORT => hash(0, 24),
        PRESENT => (0..4).for_each(|i| hash(i, 0)),
        UPDATE_SURFACE => {
            hash(1, 0);
            hash(3, 0);
        }
        _ => {}
    }
    r
}

// --- Recording the original ---------------------------------------------------------------

static RECORDING: AtomicBool = AtomicBool::new(false);
static RECORD_THREAD: AtomicU32 = AtomicU32::new(0);
static RECORDED: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());

fn recording() -> bool {
    RECORDING.load(Ordering::Acquire) && unsafe { GetCurrentThreadId() } == RECORD_THREAD.load(Ordering::Acquire)
}

/// While the original runs, the device pointer (0x252A2D0) points at this proxy: its
/// vtable records the 24 executor methods and forwards every method to the device
/// that was current (`PROXY_TARGET`). The device object itself is never touched (a
/// D3D wrapper such as DFEngine may own its vtable).
static PROXY_TARGET: AtomicU32 = AtomicU32::new(0);
static PROXY: Mutex<Option<usize>> = Mutex::new(None);

fn forward(method: u32, _proxy: u32, a: &[u32]) -> u32 {
    let this = PROXY_TARGET.load(Ordering::Acquire);
    if recording() {
        let r = record_of(method, this, a);
        RECORDED.lock().unwrap_or_else(|p| p.into_inner()).push(r);
    }
    unsafe { com_call(rd(rd(this as usize) as usize + method as usize * 4) as usize, this, a) }
}

/// The proxy object {vtable, 0}, built on first use.
fn proxy() -> usize {
    let mut p = PROXY.lock().unwrap_or_else(|p| p.into_inner());
    *p.get_or_insert_with(|| {
        let mut vt: Vec<u32> = FORWARDERS.iter().map(|&f| f as u32).collect();
        for (m, f) in thunks() {
            vt[m as usize] = f as u32;
        }
        let vt = Box::leak(vt.into_boxed_slice());
        Box::leak(Box::new([vt.as_ptr() as u32, 0u32])).as_ptr() as usize
    })
}

macro_rules! thunk {
    ($name:ident, $method:expr $(, $a:ident)*) => {
        extern "system" fn $name(this: u32 $(, $a: u32)*) -> u32 {
            forward($method, this, &[$($a),*])
        }
    };
}
thunk!(t_present, PRESENT, a, b, c, d);
thunk!(t_update_surface, UPDATE_SURFACE, a, b, c, d);
thunk!(t_get_render_target_data, GET_RENDER_TARGET_DATA, a, b);
thunk!(t_stretch_rect, STRETCH_RECT, a, b, c, d, e);
thunk!(t_set_render_target, SET_RENDER_TARGET, a, b);
thunk!(t_set_depth_stencil, SET_DEPTH_STENCIL_SURFACE, a);
thunk!(t_begin_scene, BEGIN_SCENE);
thunk!(t_end_scene, END_SCENE);
thunk!(t_clear, CLEAR, a, b, c, d, e, f);
thunk!(t_set_viewport, SET_VIEWPORT, a);
thunk!(t_set_render_state, SET_RENDER_STATE, a, b);
thunk!(t_set_texture, SET_TEXTURE, a, b);
thunk!(t_set_sampler_state, SET_SAMPLER_STATE, a, b, c);
thunk!(t_set_scissor_rect, SET_SCISSOR_RECT, a);
thunk!(t_draw_primitive, DRAW_PRIMITIVE, a, b, c);
thunk!(t_draw_indexed_primitive, DRAW_INDEXED_PRIMITIVE, a, b, c, d, e, f);
thunk!(t_draw_primitive_up, DRAW_PRIMITIVE_UP, a, b, c, d);
thunk!(t_set_vertex_declaration, SET_VERTEX_DECLARATION, a);
thunk!(t_set_vertex_shader, SET_VERTEX_SHADER, a);
thunk!(t_set_vs_constant_f, SET_VERTEX_SHADER_CONSTANT_F, a, b, c);
thunk!(t_set_stream_source, SET_STREAM_SOURCE, a, b, c, d);
thunk!(t_set_indices, SET_INDICES, a);
thunk!(t_set_pixel_shader, SET_PIXEL_SHADER, a);
thunk!(t_set_ps_constant_f, SET_PIXEL_SHADER_CONSTANT_F, a, b, c);

fn thunks() -> [(u32, usize); 24] {
    [
        (PRESENT, t_present as extern "system" fn(u32, u32, u32, u32, u32) -> u32 as usize),
        (UPDATE_SURFACE, t_update_surface as extern "system" fn(u32, u32, u32, u32, u32) -> u32 as usize),
        (GET_RENDER_TARGET_DATA, t_get_render_target_data as extern "system" fn(u32, u32, u32) -> u32 as usize),
        (STRETCH_RECT, t_stretch_rect as extern "system" fn(u32, u32, u32, u32, u32, u32) -> u32 as usize),
        (SET_RENDER_TARGET, t_set_render_target as extern "system" fn(u32, u32, u32) -> u32 as usize),
        (SET_DEPTH_STENCIL_SURFACE, t_set_depth_stencil as extern "system" fn(u32, u32) -> u32 as usize),
        (BEGIN_SCENE, t_begin_scene as extern "system" fn(u32) -> u32 as usize),
        (END_SCENE, t_end_scene as extern "system" fn(u32) -> u32 as usize),
        (CLEAR, t_clear as extern "system" fn(u32, u32, u32, u32, u32, u32, u32) -> u32 as usize),
        (SET_VIEWPORT, t_set_viewport as extern "system" fn(u32, u32) -> u32 as usize),
        (SET_RENDER_STATE, t_set_render_state as extern "system" fn(u32, u32, u32) -> u32 as usize),
        (SET_TEXTURE, t_set_texture as extern "system" fn(u32, u32, u32) -> u32 as usize),
        (SET_SAMPLER_STATE, t_set_sampler_state as extern "system" fn(u32, u32, u32, u32) -> u32 as usize),
        (SET_SCISSOR_RECT, t_set_scissor_rect as extern "system" fn(u32, u32) -> u32 as usize),
        (DRAW_PRIMITIVE, t_draw_primitive as extern "system" fn(u32, u32, u32, u32) -> u32 as usize),
        (
            DRAW_INDEXED_PRIMITIVE,
            t_draw_indexed_primitive as extern "system" fn(u32, u32, u32, u32, u32, u32, u32) -> u32 as usize,
        ),
        (DRAW_PRIMITIVE_UP, t_draw_primitive_up as extern "system" fn(u32, u32, u32, u32, u32) -> u32 as usize),
        (SET_VERTEX_DECLARATION, t_set_vertex_declaration as extern "system" fn(u32, u32) -> u32 as usize),
        (SET_VERTEX_SHADER, t_set_vertex_shader as extern "system" fn(u32, u32) -> u32 as usize),
        (SET_VERTEX_SHADER_CONSTANT_F, t_set_vs_constant_f as extern "system" fn(u32, u32, u32, u32) -> u32 as usize),
        (SET_STREAM_SOURCE, t_set_stream_source as extern "system" fn(u32, u32, u32, u32, u32) -> u32 as usize),
        (SET_INDICES, t_set_indices as extern "system" fn(u32, u32) -> u32 as usize),
        (SET_PIXEL_SHADER, t_set_pixel_shader as extern "system" fn(u32, u32) -> u32 as usize),
        (SET_PIXEL_SHADER_CONSTANT_F, t_set_ps_constant_f as extern "system" fn(u32, u32, u32, u32) -> u32 as usize),
    ]
}

/// Forward method `n` of the proxy to the proxied device, arguments untouched.
macro_rules! fwd {
    ($name:ident, $n:expr) => {
        #[unsafe(naked)]
        extern "system" fn $name() {
            std::arch::naked_asm!(
                "mov eax, dword ptr [{t}]",
                "mov dword ptr [esp + 4], eax",
                "mov ecx, dword ptr [eax]",
                "jmp dword ptr [ecx + {off}]",
                t = sym PROXY_TARGET,
                off = const $n * 4,
            )
        }
    };
}
fwd!(f0, 0);
fwd!(f1, 1);
fwd!(f2, 2);
fwd!(f3, 3);
fwd!(f4, 4);
fwd!(f5, 5);
fwd!(f6, 6);
fwd!(f7, 7);
fwd!(f8, 8);
fwd!(f9, 9);
fwd!(f10, 10);
fwd!(f11, 11);
fwd!(f12, 12);
fwd!(f13, 13);
fwd!(f14, 14);
fwd!(f15, 15);
fwd!(f16, 16);
fwd!(f17, 17);
fwd!(f18, 18);
fwd!(f19, 19);
fwd!(f20, 20);
fwd!(f21, 21);
fwd!(f22, 22);
fwd!(f23, 23);
fwd!(f24, 24);
fwd!(f25, 25);
fwd!(f26, 26);
fwd!(f27, 27);
fwd!(f28, 28);
fwd!(f29, 29);
fwd!(f30, 30);
fwd!(f31, 31);
fwd!(f32, 32);
fwd!(f33, 33);
fwd!(f34, 34);
fwd!(f35, 35);
fwd!(f36, 36);
fwd!(f37, 37);
fwd!(f38, 38);
fwd!(f39, 39);
fwd!(f40, 40);
fwd!(f41, 41);
fwd!(f42, 42);
fwd!(f43, 43);
fwd!(f44, 44);
fwd!(f45, 45);
fwd!(f46, 46);
fwd!(f47, 47);
fwd!(f48, 48);
fwd!(f49, 49);
fwd!(f50, 50);
fwd!(f51, 51);
fwd!(f52, 52);
fwd!(f53, 53);
fwd!(f54, 54);
fwd!(f55, 55);
fwd!(f56, 56);
fwd!(f57, 57);
fwd!(f58, 58);
fwd!(f59, 59);
fwd!(f60, 60);
fwd!(f61, 61);
fwd!(f62, 62);
fwd!(f63, 63);
fwd!(f64, 64);
fwd!(f65, 65);
fwd!(f66, 66);
fwd!(f67, 67);
fwd!(f68, 68);
fwd!(f69, 69);
fwd!(f70, 70);
fwd!(f71, 71);
fwd!(f72, 72);
fwd!(f73, 73);
fwd!(f74, 74);
fwd!(f75, 75);
fwd!(f76, 76);
fwd!(f77, 77);
fwd!(f78, 78);
fwd!(f79, 79);
fwd!(f80, 80);
fwd!(f81, 81);
fwd!(f82, 82);
fwd!(f83, 83);
fwd!(f84, 84);
fwd!(f85, 85);
fwd!(f86, 86);
fwd!(f87, 87);
fwd!(f88, 88);
fwd!(f89, 89);
fwd!(f90, 90);
fwd!(f91, 91);
fwd!(f92, 92);
fwd!(f93, 93);
fwd!(f94, 94);
fwd!(f95, 95);
fwd!(f96, 96);
fwd!(f97, 97);
fwd!(f98, 98);
fwd!(f99, 99);
fwd!(f100, 100);
fwd!(f101, 101);
fwd!(f102, 102);
fwd!(f103, 103);
fwd!(f104, 104);
fwd!(f105, 105);
fwd!(f106, 106);
fwd!(f107, 107);
fwd!(f108, 108);
fwd!(f109, 109);
fwd!(f110, 110);
fwd!(f111, 111);
fwd!(f112, 112);
fwd!(f113, 113);
fwd!(f114, 114);
fwd!(f115, 115);
fwd!(f116, 116);
fwd!(f117, 117);
fwd!(f118, 118);

const FORWARDERS: [extern "system" fn(); 119] = [
    f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15, f16, f17, f18, f19, f20, f21, f22, f23, f24,
    f25, f26, f27, f28, f29, f30, f31, f32, f33, f34, f35, f36, f37, f38, f39, f40, f41, f42, f43, f44, f45, f46, f47,
    f48, f49, f50, f51, f52, f53, f54, f55, f56, f57, f58, f59, f60, f61, f62, f63, f64, f65, f66, f67, f68, f69, f70,
    f71, f72, f73, f74, f75, f76, f77, f78, f79, f80, f81, f82, f83, f84, f85, f86, f87, f88, f89, f90, f91, f92, f93,
    f94, f95, f96, f97, f98, f99, f100, f101, f102, f103, f104, f105, f106, f107, f108, f109, f110, f111, f112, f113,
    f114, f115, f116, f117, f118,
];

// The Windows calls and gfx_check_device outcomes of the recorded run.
const K_QPC: u32 = 0;
const K_QPF: u32 = 1;
const K_SLEEP: u32 = 2;
const K_CHECK_DEVICE: u32 = 3;
const K_READBACK: u32 = 4;
static TAPE: Mutex<VecDeque<[u32; 3]>> = Mutex::new(VecDeque::new());
static REAL_QPC: AtomicUsize = AtomicUsize::new(0);
static REAL_QPF: AtomicUsize = AtomicUsize::new(0);
static REAL_SLEEP: AtomicUsize = AtomicUsize::new(0);

fn tape_push(e: [u32; 3]) {
    TAPE.lock().unwrap_or_else(|p| p.into_inner()).push_back(e);
}

extern "system" fn qpc_w(out: *mut i64) -> i32 {
    let f: extern "system" fn(*mut i64) -> i32 = unsafe { std::mem::transmute(REAL_QPC.load(Ordering::Acquire)) };
    let r = f(out);
    if recording() {
        let v = unsafe { *out };
        tape_push([K_QPC, v as u32, (v >> 32) as u32]);
    }
    r
}

extern "system" fn qpf_w(out: *mut i64) -> i32 {
    let f: extern "system" fn(*mut i64) -> i32 = unsafe { std::mem::transmute(REAL_QPF.load(Ordering::Acquire)) };
    let r = f(out);
    if recording() {
        let v = unsafe { *out };
        tape_push([K_QPF, v as u32, (v >> 32) as u32]);
    }
    r
}

extern "system" fn sleep_w(ms: u32) {
    if recording() {
        tape_push([K_SLEEP, ms, 0]);
    }
    let f: extern "system" fn(u32) = unsafe { std::mem::transmute(REAL_SLEEP.load(Ordering::Acquire)) };
    f(ms)
}

/// `gfx_check_device` runs live in both modes (it is not ported); while recording,
/// its own device calls are left out and the device it leaves current is taped.
extern "C" fn check_device_hook() {
    let proxied = recording() && unsafe { rd(DEVICE) } as usize == proxy();
    if proxied {
        unsafe { wr(DEVICE, PROXY_TARGET.load(Ordering::Acquire)) };
    }
    let rec = recording();
    let was = rec && RECORDING.swap(false, Ordering::AcqRel);
    unsafe { std::mem::transmute::<usize, extern "C" fn()>(orig(CHECK_DEVICE_HOOK))() };
    if rec {
        RECORDING.store(was, Ordering::Release);
    }
    if recording() {
        tape_push([K_CHECK_DEVICE, unsafe { rd(DEVICE) }, 0]);
    }
    if proxied {
        PROXY_TARGET.store(unsafe { rd(DEVICE) }, Ordering::Release);
        unsafe { wr(DEVICE, proxy() as u32) };
    }
}

/// The readback helper (0xD1C530, cdecl (index, u32* out, clear) -> data pointer;
/// command 31 and the render thread call it) runs live in both modes and calls
/// Direct3D itself: those calls are left out, and the call is taped with its index.
extern "C" fn readback_hook(arg: u32, b: u32, c: u32) -> u32 {
    // Only the recording thread pauses (the render thread calls this too).
    let rec = recording();
    let was = rec && RECORDING.swap(false, Ordering::AcqRel);
    let r =
        unsafe { std::mem::transmute::<usize, extern "C" fn(u32, u32, u32) -> u32>(orig(READBACK_HOOK))(arg, b, c) };
    if rec {
        RECORDING.store(was, Ordering::Release);
    }
    if rec {
        tape_push([K_READBACK, arg, 0]);
    }
    r
}

// --- The executor ---------------------------------------------------------------------

/// Live: reads and writes the game's globals and calls Direct3D. Dry: globals go to
/// an overlay over a snapshot taken before the original ran, calls and Windows
/// results go to / come from the tape.
struct Exec {
    dry: bool,
    overlay: BTreeMap<usize, u32>,
    end: u32,
    calls: Vec<Vec<u32>>,
    tape: VecDeque<[u32; 3]>,
    tape_mismatch: u32,
    unverified: u32,
}

impl Exec {
    fn live() -> Self {
        Exec {
            dry: false,
            overlay: BTreeMap::new(),
            end: 0,
            calls: Vec::new(),
            tape: VecDeque::new(),
            tape_mismatch: 0,
            unverified: 0,
        }
    }

    fn rd(&self, p: usize) -> u32 {
        match self.overlay.get(&p) {
            Some(&v) if self.dry => v,
            _ => unsafe { rd(p) },
        }
    }

    fn wr(&mut self, p: usize, v: u32) {
        if self.dry {
            self.overlay.insert(p, v);
        } else {
            unsafe { wr(p, v) }
        }
    }

    fn rd64(&self, p: usize) -> i64 {
        (self.rd(p) as u64 | (self.rd(p + 4) as u64) << 32) as i64
    }

    fn wr64(&mut self, p: usize, v: i64) {
        self.wr(p, v as u32);
        self.wr(p + 4, (v >> 32) as u32);
    }

    fn has_data(&self) -> bool {
        self.rd(RING_READ) < if self.dry { self.end } else { unsafe { rd(RING_PUBLISHED) } }
    }

    /// The next ring dword (ring contents are read as they are; only the pointer moves).
    fn pop(&mut self) -> u32 {
        let r = self.rd(RING_READ);
        self.wr(RING_READ, r + 4);
        unsafe { rd(r as usize) }
    }

    /// The read pointer, then advance it by `n` bytes.
    fn take(&mut self, n: u32) -> u32 {
        let r = self.rd(RING_READ);
        self.wr(RING_READ, r.wrapping_add(n));
        r
    }

    fn device(&mut self, method: u32, a: &[u32]) -> u32 {
        let this = self.rd(DEVICE);
        if self.dry {
            self.calls.push(record_of(method, this, a));
            0
        } else {
            unsafe { com_call(rd(rd(this as usize) as usize + method as usize * 4) as usize, this, a) }
        }
    }

    fn tape_next(&mut self, kind: u32) -> Option<[u32; 3]> {
        match self.tape.pop_front() {
            Some(e) if e[0] == kind => Some(e),
            _ => {
                self.tape_mismatch += 1;
                None
            }
        }
    }

    /// QueryPerformanceCounter / Frequency into a global.
    fn qp(&mut self, kind: u32, p: usize) {
        if self.dry {
            if let Some(e) = self.tape_next(kind) {
                self.wr64(p, (e[1] as u64 | (e[2] as u64) << 32) as i64);
            }
        } else {
            let f: extern "system" fn(*mut i64) -> i32 =
                unsafe { slot(if kind == K_QPC { SLOT_QPC } else { SLOT_QPF }) };
            f(p as *mut i64);
        }
    }

    fn sleep(&mut self, ms: u32) {
        if self.dry {
            if let Some(e) = self.tape_next(K_SLEEP)
                && e[1] != ms
            {
                self.tape_mismatch += 1;
            }
        } else {
            let f: extern "system" fn(u32) = unsafe { slot(SLOT_SLEEP) };
            f(ms)
        }
    }

    fn run(&mut self) {
        self.qp(K_QPF, T_FREQ);
        self.qp(K_QPC, T_START);
        let idle = self.rd64(T_IDLE).wrapping_add(self.rd64(T_START).wrapping_sub(self.rd64(T_END)));
        self.wr64(T_IDLE, idle);
        while self.has_data() {
            let id = self.pop();
            if !self.command(id) {
                log(&format!("render: command {id} at {:#x} (the original breaks here)", self.rd(RING_READ) - 4));
                break;
            }
        }
        self.qp(K_QPC, T_END);
    }

    fn command(&mut self, id: u32) -> bool {
        match id {
            0 => {
                self.device(BEGIN_SCENE, &[]);
            }
            1 => self.end_scene(),
            2 => {
                let count = self.pop();
                let rects: Vec<u32> = (0..count * 4).map(|_| self.pop()).collect();
                let (flags, color, z, stencil) = (self.pop(), self.pop(), self.pop(), self.pop());
                let p = if count == 0 { 0 } else { rects.as_ptr() as u32 };
                self.device(CLEAR, &[count, p, flags, color, z, stencil]);
            }
            3 | 4 | 5 => {
                let v = self.pop();
                let m = [SET_PIXEL_SHADER, SET_VERTEX_SHADER, SET_VERTEX_DECLARATION][id as usize - 3];
                self.device(m, &[v]);
            }
            6 | 7 => {
                let p = self.rd(RING_READ) as usize;
                let (reg, count) = unsafe { (rd(p), rd(p + 4)) };
                let m = if id == 6 { SET_VERTEX_SHADER_CONSTANT_F } else { SET_PIXEL_SHADER_CONSTANT_F };
                self.device(m, &[reg, p as u32 + 8, count]);
                self.take(count.wrapping_mul(16) + 8);
            }
            8 => {
                let p = self.take(16) as usize;
                let a = unsafe { [rd(p), rd(p + 4), rd(p + 8), rd(p + 12)] };
                self.device(SET_STREAM_SOURCE, &a);
                self.wr(VB_CACHE, a[1]);
            }
            9 => {
                self.pop();
                self.pop();
            }
            10 => {
                let ib = self.pop();
                self.device(SET_INDICES, &[ib]);
                self.wr(IB_CACHE, ib);
            }
            11 => {
                let a = [self.pop(), self.pop(), self.pop()];
                self.device(DRAW_PRIMITIVE, &a);
            }
            12 => {
                let p = self.take(0x18) as usize;
                let a: Vec<u32> = (0..6).map(|i| unsafe { rd(p + i * 4) }).collect();
                self.device(DRAW_INDEXED_PRIMITIVE, &a);
            }
            13 => {
                let p = self.rd(RING_READ) as usize;
                let (kind, n, stride) = unsafe { (rd(p), rd(p + 4), rd(p + 8)) };
                let verts = prim_vertex_count(kind, n);
                self.device(DRAW_PRIMITIVE_UP, &[kind, n, p as u32 + 12, stride]);
                self.take(verts.wrapping_mul(stride).wrapping_add(12));
            }
            14 => {
                let p = self.take(8) as usize;
                let (stage, tex) = unsafe { (rd(p), rd(p + 4)) };
                self.device(SET_TEXTURE, &[stage, tex]);
                self.wr(TEXTURE_CACHE + stage as usize * 4, tex);
            }
            15 => {
                let p = self.take(8) as usize;
                let a = unsafe { [rd(p), rd(p + 4)] };
                self.device(SET_RENDER_STATE, &a);
            }
            16 => {
                let a = [self.pop(), self.pop()];
                self.device(SET_RENDER_TARGET, &a);
            }
            17 => {
                let s = self.pop();
                self.device(SET_DEPTH_STENCIL_SURFACE, &[s]);
            }
            18 => {
                let src = self.pop();
                let src_rect = self.take(16);
                let dst = self.pop();
                let dst_rect = self.take(16);
                self.device(STRETCH_RECT, &[src, src_rect, dst, dst_rect, 0]);
            }
            19 => {
                let p = self.take(12) as usize;
                let a = unsafe { [rd(p), rd(p + 4), rd(p + 8)] };
                self.device(SET_SAMPLER_STATE, &a);
            }
            20 => self.present(),
            21 => {
                let rect = self.take(16);
                self.device(SET_RENDER_STATE, &[0xAE, 1]); // D3DRS_SCISSORTESTENABLE
                self.device(SET_SCISSOR_RECT, &[rect]);
            }
            22 => {
                self.pop();
                let size = self.pop();
                let index = self.pop();
                let data = self.take(size);
                self.float_texture_upload(index, data, size);
            }
            23 => {
                let stage = self.pop();
                let index = self.pop();
                let tex = self.rd(FLOAT_TEXTURES + index as usize * 4);
                self.device(SET_TEXTURE, &[stage.wrapping_add(0x101), tex]); // D3DVERTEXTEXTURESAMPLER0 + stage
            }
            24 => {
                let vp = self.take(24);
                self.device(SET_VIEWPORT, &[vp]);
            }
            25 | 26 => {
                let query = self.pop();
                if self.dry {
                    self.unverified += 1;
                } else {
                    // IDirect3DQuery9::Issue: BEGIN (2) or END (1).
                    unsafe { com_call(rd(rd(query as usize) as usize + 0x18) as usize, query, &[27 - id]) };
                }
            }
            29 => {
                let a = [self.pop(), self.pop()];
                self.device(GET_RENDER_TARGET_DATA, &a);
            }
            30 => {
                let (src, dst) = (self.pop(), self.pop());
                self.device(UPDATE_SURFACE, &[src, 0, dst, 0]);
            }
            31 => {
                let arg = self.pop();
                if self.dry {
                    if let Some(e) = self.tape_next(K_READBACK)
                        && e[1] != arg
                    {
                        self.tape_mismatch += 1;
                    }
                } else {
                    unsafe { std::mem::transmute::<usize, extern "C" fn(u32, u32, u32)>(READBACK)(arg, 0, 0) };
                }
            }
            _ => return false,
        }
        true
    }

    /// Command 1: the FPS overlay when on, EndScene, then Release the objects queued
    /// for release (under their critical section) and clear the second list.
    fn end_scene(&mut self) {
        if unsafe { rb(FPS_OVERLAY) } == 1 {
            self.fps_overlay();
        }
        self.device(END_SCENE, &[]);
        if self.dry {
            self.wr(0x0252_A2C0, 0);
            self.wr(0x0252_A2C4, 0);
            return;
        }
        unsafe {
            slot::<extern "system" fn(u32)>(SLOT_ENTER_CS)(RELEASE_CS);
            let list = rd(RELEASE_LIST) as usize;
            if list != 0 {
                let (begin, end) = (rd(list + 4), rd(list + 8));
                let count = if begin == 0 { 0 } else { (end - begin) / 4 };
                for i in 0..count {
                    let obj = rd(begin as usize + i as usize * 4);
                    com_call(rd(rd(obj as usize) as usize + 8) as usize, obj, &[]); // Release
                }
                std::mem::transmute::<usize, extern "C" fn()>(RELEASE_LIST_CLEAR)();
            }
            slot::<extern "system" fn(u32)>(SLOT_LEAVE_CS)(RELEASE_CS);
            if rd(RELEASE_LIST_2) != 0 {
                std::mem::transmute::<usize, extern "C" fn()>(RELEASE_LIST_2_CLEAR)();
            }
        }
        self.wr(0x0252_A2C0, 0);
        self.wr(0x0252_A2C4, 0);
    }

    /// The debug FPS / idle overlay (flag 0x252734B), drawn just before EndScene.
    fn fps_overlay(&mut self) {
        self.qp(K_QPC, T_OVERLAY_NOW);
        let freq = self.rd64(T_FREQ) as f64;
        let dt = (self.rd64(T_OVERLAY_NOW) as f64 - self.rd64(T_OVERLAY_PREV) as f64) / freq;
        let now = self.rd64(T_OVERLAY_NOW);
        self.wr64(T_OVERLAY_PREV, now);
        let idle = self.rd64(T_IDLE) as f64 / freq;
        if self.dry {
            self.unverified += 1;
        } else {
            unsafe {
                if rd(FPS_FONT) == 0 {
                    let create: extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> u32 =
                        std::mem::transmute(D3DX_CREATE_FONT);
                    create(rd(REAL_DEVICE), 16, 0, 400, 0, 0, 1, 4, 1, 0, 0x00DC_902C, FPS_FONT as u32);
                }
                let mut rect = [0i32; 4];
                slot::<extern "system" fn(*mut [i32; 4], i32, i32, i32, i32) -> i32>(SLOT_SET_RECT)(
                    &mut rect, 0, 0, 300, 40,
                );
                let mut buf = [0u8; 0x70];
                let sprintf: extern "C" fn(*mut u8, u32, f64, f64) -> i32 = std::mem::transmute(SPRINTF);
                let draw = |buf: &[u8], rect: &mut [i32; 4]| {
                    let font = rd(FPS_FONT);
                    if font != 0 {
                        // ID3DXFont::DrawTextA(sprite, text, -1, rect, 0, yellow)
                        let f: extern "system" fn(u32, u32, *const u8, i32, *mut [i32; 4], u32, u32) -> i32 =
                            std::mem::transmute(rd(rd(font as usize) as usize + 0x38) as usize);
                        f(font, 0, buf.as_ptr(), -1, rect, 0, 0xFF00_FFFF);
                    }
                };
                sprintf(buf.as_mut_ptr(), 0x00DC_9034, 1.0 / dt, dt * 1000.0);
                draw(&buf, &mut rect);
                slot::<extern "system" fn(*mut [i32; 4], i32, i32) -> i32>(SLOT_OFFSET_RECT)(&mut rect, 0, 15);
                sprintf(buf.as_mut_ptr(), 0x00DC_904C, 1.0 / idle, idle * 1000.0);
                draw(&buf, &mut rect);
            }
        }
        self.wr64(T_IDLE, 0);
    }

    /// Command 20: Present, the device check, then the frame limiter: above 35 fps
    /// sleep half the time left to a 35 fps frame (at most 100 ms), below 10 fps
    /// sleep 30 ms, else Sleep(0).
    fn present(&mut self) {
        self.device(PRESENT, &[0, 0, 0, 0]);
        if self.dry {
            if let Some(e) = self.tape_next(K_CHECK_DEVICE) {
                self.wr(DEVICE, e[1]);
            }
        } else {
            unsafe { std::mem::transmute::<usize, extern "C" fn()>(CHECK_DEVICE)() };
        }
        self.qp(K_QPC, T_PRESENT_NOW);
        let dt = (self.rd64(T_PRESENT_NOW) as f64 - self.rd64(T_PRESENT_PREV) as f64) / self.rd64(T_FREQ) as f64;
        let now = self.rd64(T_PRESENT_NOW);
        self.wr64(T_PRESENT_PREV, now);
        let fps = 1.0 / dt;
        if fps > 35.0 {
            let mut t = (0.028_571_428_731_083_87 - 1.0 / fps) as f32;
            t = (t as f64 * 1000.0) as f32;
            t = (t as f64 / 2.0) as f32;
            if t as f64 > 100.0 {
                t = 100.0;
            }
            if 0.0 > t as f64 {
                t = 0.0;
            }
            self.sleep(t as i32 as u32);
        } else if 10.0 > fps {
            self.sleep(30);
        } else {
            self.sleep(0);
        }
    }

    /// Command 22: copy `size` bytes into float texture `index` (created as 512x1
    /// A32B32G32R32F on first use).
    fn float_texture_upload(&mut self, index: u32, data: u32, size: u32) {
        if self.dry {
            self.unverified += 1;
            return;
        }
        unsafe {
            let entry = FLOAT_TEXTURES + index as usize * 4;
            if rd(entry) == 0 {
                std::arch::asm!(
                    "push 0x74", "push 0", "push 1", "push 1", "push 0x200",
                    "call {f}",
                    "add esp, 0x14",
                    f = in(reg) CREATE_FLOAT_TEXTURE, inout("edx") entry => _,
                    clobber_abi("C"),
                );
            }
            let tex = rd(entry);
            let mut locked = [0u32; 2]; // D3DLOCKED_RECT {pitch, bits}
            com_call(rd(rd(tex as usize) as usize + 0x4C) as usize, tex, &[0, locked.as_mut_ptr() as u32, 0, 0]);
            std::ptr::copy_nonoverlapping(data as *const u8, locked[1] as *mut u8, size as usize);
            com_call(rd(rd(tex as usize) as usize + 0x50) as usize, tex, &[0]);
        }
    }
}

// --- Ring rewind ----------------------------------------------------------------------

/// Guards the ring rewind against a shadow check reading the stretch the original
/// just executed. Reentrant for its owner.
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

fn atomic(p: usize) -> &'static std::sync::atomic::AtomicU32 {
    unsafe { &*(p as *const std::sync::atomic::AtomicU32) }
}

/// `gfx_command_buffer_wait` (0xD1C780): wait (Sleep(1)) until the device thread has
/// read everything published, then rewind the ring to its base.
unsafe fn command_buffer_wait() {
    unsafe {
        while rd(RING_READ) < rd(RING_PUBLISHED) {
            slot::<extern "system" fn(u32)>(SLOT_SLEEP)(1);
        }
        with_lock(|| {
            let base = rd(RING_BASE);
            atomic(RING_PUBLISHED).swap(base, Ordering::SeqCst);
            let base = rd(RING_BASE);
            wr(RING_WRITE, base);
            wr(RING_LAST_PUBLISHED, base);
            atomic(RING_READ).swap(base, Ordering::SeqCst);
        });
    }
}

// --- Shadow check ---------------------------------------------------------------------

static RUNS: AtomicU32 = AtomicU32::new(0);
static CALLS_COMPARED: AtomicU32 = AtomicU32::new(0);
static MISMATCHED_RUNS: AtomicU32 = AtomicU32::new(0);
static TAPE_MISMATCHES: AtomicU32 = AtomicU32::new(0);
static UNVERIFIED: AtomicU32 = AtomicU32::new(0);
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// The globals the executor reads and writes, snapshotted for the dry run.
fn snapshot_globals() -> BTreeMap<usize, u32> {
    let mut m = BTreeMap::new();
    let mut add = |p: usize, n: usize| {
        for i in 0..n {
            m.insert(p + i * 4, unsafe { rd(p + i * 4) });
        }
    };
    add(RING_READ, 1);
    add(DEVICE, 1);
    add(VB_CACHE, 1);
    add(IB_CACHE, 1);
    add(TEXTURE_CACHE, 0x110); // stages 0-15 and the vertex samplers 0x100-0x103
    add(T_PRESENT_PREV, 16);
    add(0x0252_A2C0, 2);
    m
}

fn log_limited(s: String) {
    if LOGGED.fetch_add(1, Ordering::Relaxed) < 30 {
        log(&s);
    }
}

fn shadow() {
    with_lock(|| {
        let before = snapshot_globals();
        let r0 = unsafe { rd(RING_READ) };
        RECORDED.lock().unwrap_or_else(|p| p.into_inner()).clear();
        TAPE.lock().unwrap_or_else(|p| p.into_inner()).clear();
        RECORD_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        PROXY_TARGET.store(unsafe { rd(DEVICE) }, Ordering::Release);
        unsafe { wr(DEVICE, proxy() as u32) };
        RECORDING.store(true, Ordering::Release);
        unsafe { std::mem::transmute::<usize, extern "C" fn()>(orig(EXECUTE))() };
        RECORDING.store(false, Ordering::Release);
        if unsafe { rd(DEVICE) } as usize == proxy() {
            unsafe { wr(DEVICE, PROXY_TARGET.load(Ordering::Acquire)) };
        }
        let r1 = unsafe { rd(RING_READ) };
        let theirs = std::mem::take(&mut *RECORDED.lock().unwrap_or_else(|p| p.into_inner()));
        let tape = std::mem::take(&mut *TAPE.lock().unwrap_or_else(|p| p.into_inner()));

        let mut ex = Exec { dry: true, overlay: before.clone(), end: r1, ..Exec::live() };
        ex.tape = tape;
        ex.wr(RING_READ, r0);
        ex.run();
        if RUNS.fetch_add(1, Ordering::Relaxed) < 3 {
            log(&format!(
                "render: shadow run: ring {r0:#x}..{r1:#x}, {} calls recorded, {} dry",
                theirs.len(),
                ex.calls.len()
            ));
        }
        CALLS_COMPARED.fetch_add(theirs.len() as u32, Ordering::Relaxed);
        UNVERIFIED.fetch_add(ex.unverified, Ordering::Relaxed);
        let left = ex.tape.len() as u32;
        if ex.tape_mismatch + left > 0 {
            TAPE_MISMATCHES.fetch_add(ex.tape_mismatch + left, Ordering::Relaxed);
            log_limited(format!(
                "render: tape: {} mismatched, {left} left over (ring {r0:#x}..{r1:#x})",
                ex.tape_mismatch
            ));
        }
        let mut bad = false;
        if theirs.len() != ex.calls.len() {
            bad = true;
            let tail =
                |v: &Vec<Vec<u32>>| v.iter().rev().take(3).map(|c| format!("{c:x?}")).collect::<Vec<_>>().join(" ");
            log_limited(format!(
                "render: original made {} calls, Rust {} (ring {r0:#x}..{r1:#x}, Rust stopped at {:#x}); last original: {} | last Rust: {}",
                theirs.len(),
                ex.calls.len(),
                ex.rd(RING_READ),
                tail(&theirs),
                tail(&ex.calls)
            ));
        }
        if let Some(i) = (0..theirs.len().min(ex.calls.len())).find(|&i| theirs[i] != ex.calls[i]) {
            bad = true;
            log_limited(format!("render: call {i} differs: original {:x?}, Rust {:x?}", theirs[i], ex.calls[i]));
        }
        for (&p, _) in before.iter() {
            let (o, r) = (unsafe { rd(p) }, ex.rd(p));
            if o != r {
                bad = true;
                log_limited(format!("render: global {p:#x}: original {o:#x}, Rust {r:#x}"));
                break;
            }
        }
        if bad {
            MISMATCHED_RUNS.fetch_add(1, Ordering::Relaxed);
        }
    });
}

// --- Hooks ----------------------------------------------------------------------------

const EXECUTE: usize = 0;
const BUFFER_WAIT: usize = 1;
const CHECK_DEVICE_HOOK: usize = 2;
const READBACK_HOOK: usize = 3;
const COUNT: usize = 4;

const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("gfx_execute_commands", 0x00D2_0360, &[0x55, 0x8B, 0xEC, 0x81, 0xEC, 0x0C, 0x08, 0x00, 0x00]),
    ("gfx_command_buffer_wait", 0x00D1_C780, &[0xA1, 0x00, 0x63, 0x3D, 0x03]),
    ("gfx_check_device", 0x00D2_0230, &[0xA1, 0xD0, 0xA2, 0x52, 0x02]),
    ("gfx_readback", 0x00D1_C530, &[0x83, 0xEC, 0x10, 0xA1, 0xD0, 0xA2, 0x52, 0x02]),
];

static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];
static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

extern "C" fn execute_hook() {
    CALLS[EXECUTE].fetch_add(1, Ordering::Relaxed);
    match MODE {
        Mode::Original => unsafe { std::mem::transmute::<usize, extern "C" fn()>(orig(EXECUTE))() },
        Mode::Shadow => shadow(),
        Mode::Rust => Exec::live().run(),
    }
}

extern "C" fn buffer_wait_hook() {
    CALLS[BUFFER_WAIT].fetch_add(1, Ordering::Relaxed);
    match MODE {
        Mode::Original => unsafe { std::mem::transmute::<usize, extern "C" fn()>(orig(BUFFER_WAIT))() },
        _ => unsafe { command_buffer_wait() },
    }
}

fn hook_address(i: usize) -> usize {
    match i {
        EXECUTE => execute_hook as extern "C" fn() as usize,
        BUFFER_WAIT => buffer_wait_hook as extern "C" fn() as usize,
        CHECK_DEVICE_HOOK => check_device_hook as extern "C" fn() as usize,
        READBACK_HOOK => readback_hook as extern "C" fn(u32, u32, u32) -> u32 as usize,
        _ => unreachable!(),
    }
}

static WRAPPED: AtomicU8 = AtomicU8::new(0);

pub fn install() {
    if MODE == Mode::Shadow {
        for (s, real, w) in [
            (SLOT_QPC, &REAL_QPC, qpc_w as extern "system" fn(*mut i64) -> i32 as usize),
            (SLOT_QPF, &REAL_QPF, qpf_w as extern "system" fn(*mut i64) -> i32 as usize),
            (SLOT_SLEEP, &REAL_SLEEP, sleep_w as extern "system" fn(u32) as usize),
        ] {
            unsafe {
                let mut old = 0u32;
                if VirtualProtect(s as *const _, 4, PAGE_READWRITE, &mut old) == 0 {
                    log("render: could not wrap an import; not hooking the executor");
                    return;
                }
                real.store(rd(s) as usize, Ordering::Release);
                wr(s, w as u32);
                VirtualProtect(s as *const _, 4, old, &mut old);
            }
        }
        WRAPPED.store(1, Ordering::Release);
    }
    let mut ok = 0;
    for (i, &(name, addr, bytes)) in PORTS.iter().enumerate() {
        if (i == CHECK_DEVICE_HOOK || i == READBACK_HOOK) && MODE != Mode::Shadow {
            continue;
        }
        match unsafe { detour::install(addr, bytes, hook_address(i)) } {
            Ok(t) => {
                ORIG[i].store(t, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("render: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    let mode = match MODE {
        Mode::Original => "original",
        Mode::Shadow => "original live, Rust dry-run compared",
        Mode::Rust => "Rust",
    };
    log(&format!("replaced {ok} render functions ({mode})"));
}

pub fn summary() -> String {
    format!(
        "render live: execute {}, buffer_wait {} | shadow runs {}, {} D3D calls compared, {} runs mismatched, tape mismatches {}, unverified ops {}",
        CALLS[EXECUTE].load(Ordering::Relaxed),
        CALLS[BUFFER_WAIT].load(Ordering::Relaxed),
        RUNS.load(Ordering::Relaxed),
        CALLS_COMPARED.load(Ordering::Relaxed),
        MISMATCHED_RUNS.load(Ordering::Relaxed),
        TAPE_MISMATCHES.load(Ordering::Relaxed),
        UNVERIFIED.load(Ordering::Relaxed),
    )
}
