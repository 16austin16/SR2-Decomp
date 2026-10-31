//! The D3D ring writers (`gfx_cmd_*`, docs/analysis/07_render.md and 18_scene.md):
//! small functions that append one command to the ring, most behind a cache so a
//! redundant state change writes nothing, some publishing the ring to the device
//! thread (`InterlockedExchangeAdd` on the published end).
//!
//! Verified in `Mode::Shadow`: publishing cannot be undone, so the Rust runs first
//! "predicting" (its writes go to an overlay, its publishes to a list), then the
//! original runs live with its publishes taped, and the two are compared: every
//! predicted dword against memory, the publishes, and the caches the prediction did
//! not touch (they must not have changed).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

use crate::{detour, log};

#[derive(PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)] // one mode is used per build
enum Mode {
    Original,
    /// Rust predicts, the original runs; compared.
    Shadow,
    Rust,
}

/// Off until tested in-game (the game is in use elsewhere); then Shadow, then Rust.
const ENABLED: bool = false;
const MODE: Mode = Mode::Shadow;

const RING_BASE: usize = 0x033D_62EC;
const RING_PUBLISHED: usize = 0x033D_62F4;
const RING_WRITE: usize = 0x033D_62F8;
const RING_LAST_PUBLISHED: usize = 0x033D_62FC;

// Caches.
const VS_CACHE: usize = 0x022F_2AEC;
const PS_CACHE: usize = 0x022F_2AE8;
const DEFAULT_VS: usize = 0x00E8_4B98;
const DEFAULT_PS: usize = 0x00E8_4B9C;
const DECL_CACHE: usize = 0x0252_A30C;
const IB_CACHE: usize = 0x0252_A310; // the gr_index_buffer wrapper, not the D3D object
const DEPTH_CACHE: usize = 0x0252_A308;
const TARGET_CACHE: usize = 0x022F_DCB8;
const TEXTURE_CACHE: usize = 0x022F_2AF0; // [16] stage -> D3D texture
const SAMPLER_CACHE: usize = 0x022F_52F0; // [sampler * 16 + type]
const STREAM_VB: usize = 0x0230_4D8C; // [stream] gr_vertex_buffer wrapper
const STREAM_OFFSET: usize = 0x0230_4D6C;
const STREAM_STRIDE: usize = 0x0230_4D7C;
const EXECUTING_DEFERRED: usize = 0x0252_A2A2;
const SCISSOR_SCALE_X: usize = 0x022F_DCC0;
const SCISSOR_SCALE_Y: usize = 0x022F_DCBC;
const GAME_CONTEXT: u32 = 0x00EC_26A0;
const RENDER_CONTEXT: u32 = 0x00EC_2610;

const SLOT_INTERLOCKED_ADD: usize = 0x00DB_809C;

unsafe fn rd(p: usize) -> u32 {
    unsafe { *(p as *const u32) }
}
unsafe fn wr(p: usize, v: u32) {
    unsafe { *(p as *mut u32) = v }
}
unsafe fn rb(p: usize) -> u8 {
    unsafe { *(p as *const u8) }
}

/// The calling thread's graphics context (TLS slot 0 + 0x60); the game thread's
/// context means the render context. Also the "render thread" byte (+0x5C).
fn thread_context() -> u32 {
    let tls: u32;
    unsafe {
        std::arch::asm!("mov {}, dword ptr fs:[0x2C]", out(reg) tls, options(nostack, readonly));
        let t = rd(tls as usize) as usize;
        let ctx = rd(t + 0x60);
        if rb(t + 0x5C) == 0 && ctx == GAME_CONTEXT { RENDER_CONTEXT } else { ctx }
    }
}

// --- Memory: live, or predicting into an overlay ---------------------------------------

struct Mem {
    dry: bool,
    overlay: BTreeMap<usize, u32>,
    publishes: Vec<u32>,
}

impl Mem {
    fn live() -> Self {
        Mem { dry: false, overlay: BTreeMap::new(), publishes: Vec::new() }
    }

    fn rd(&self, p: usize) -> u32 {
        if self.dry
            && let Some(&v) = self.overlay.get(&p)
        {
            return v;
        }
        unsafe { rd(p) }
    }

    fn wr(&mut self, p: usize, v: u32) {
        if self.dry {
            self.overlay.insert(p, v);
        } else {
            unsafe { wr(p, v) }
        }
    }

    /// Append dwords at the ring's write pointer and advance it.
    fn put(&mut self, words: &[u32]) {
        let w = self.rd(RING_WRITE) as usize;
        for (i, &v) in words.iter().enumerate() {
            self.wr(w + i * 4, v);
        }
        self.wr(RING_WRITE, (w + words.len() * 4) as u32);
    }

    /// Append `n` bytes from `src`.
    fn put_bytes(&mut self, src: u32, n: u32) {
        let w = self.rd(RING_WRITE) as usize;
        if self.dry {
            let mut i = 0;
            while i < n as usize {
                let take = (n as usize - i).min(4);
                let mut v = self.rd(w + i).to_le_bytes();
                for b in 0..take {
                    v[b] = unsafe { *((src as usize + i + b) as *const u8) };
                }
                self.wr(w + i, u32::from_le_bytes(v));
                i += 4;
            }
        } else {
            unsafe { std::ptr::copy_nonoverlapping(src as *const u8, w as *mut u8, n as usize) };
        }
        self.wr(RING_WRITE, (w + n as usize) as u32);
    }

    /// Publish everything written since the last publish.
    fn publish(&mut self) {
        let delta = self.rd(RING_WRITE).wrapping_sub(self.rd(RING_LAST_PUBLISHED));
        if self.dry {
            self.publishes.push(delta);
        } else {
            unsafe { (*(RING_PUBLISHED as *const AtomicU32)).fetch_add(delta, Ordering::SeqCst) };
        }
        let w = self.rd(RING_WRITE);
        self.wr(RING_LAST_PUBLISHED, w);
    }
}

// --- The writers ------------------------------------------------------------------------

/// `gfx_cmd_set_stream_source` (0xD21310; ECX = stream, EDI = gr_vertex_buffer,
/// EDX = offset, ESI = stride): ring 8 unless all three match the stream's cache.
fn set_stream_source(m: &mut Mem, stream: u32, vb: u32, offset: u32, stride: u32) {
    let s = stream as usize * 4;
    if vb == m.rd(STREAM_VB + s) && offset == m.rd(STREAM_OFFSET + s) && stride == m.rd(STREAM_STRIDE + s) {
        return;
    }
    let d3d = if vb == 0 { 0 } else { m.rd(vb as usize + 4) };
    m.put(&[8, stream, d3d, offset, stride]);
    m.wr(STREAM_VB + s, vb);
    m.wr(STREAM_STRIDE + s, stride);
    m.wr(STREAM_OFFSET + s, offset);
}

/// `gfx_cmd_set_indices` (0xD213B0; ECX = gr_index_buffer): ring 10 with its D3D buffer.
fn set_indices(m: &mut Mem, ib: u32) {
    if m.rd(IB_CACHE) == ib {
        return;
    }
    let d3d = if ib == 0 { 0 } else { m.rd(ib as usize + 4) };
    m.put(&[10, d3d]);
    m.wr(IB_CACHE, ib);
}

/// `gfx_cmd_set_vertex_declaration` (0xD21450; ECX = declaration): ring 5.
fn set_vertex_declaration(m: &mut Mem, decl: u32) {
    if decl == m.rd(DECL_CACHE) {
        return;
    }
    m.put(&[5, decl]);
    m.wr(DECL_CACHE, decl);
}

/// `gfx_cmd_set_pixel_shader` / `_vertex_shader` (0xD21480 / 0xD214B0; ECX = shader,
/// NULL = the default one): ring 3 / 4.
fn set_shader(m: &mut Mem, vertex: bool, shader: u32) {
    let (default, cache, id) = if vertex { (DEFAULT_VS, VS_CACHE, 4) } else { (DEFAULT_PS, PS_CACHE, 3) };
    let s = if shader == 0 { m.rd(default) } else { shader };
    if m.rd(cache) == s {
        return;
    }
    m.put(&[id, s]);
    m.wr(cache, s);
}

/// `gfx_cmd_9` (0xD214E0; cdecl (a), DX = b): ring 9 (a no-op on PC).
fn cmd_9(m: &mut Mem, a: u32, b: u32) {
    m.put(&[9, a, b & 0xFFFF]);
}

/// The shader constant writers (0xD21520 PS unchecked, 0xD21560 PS skipping a negative
/// register, 0xD215E0 VS; ECX = count of vec4s, EDX = register, cdecl (data)).
fn set_constants(m: &mut Mem, id: u32, checked: bool, count: u32, reg: u32, data: u32) {
    if checked && (reg as i32) < 0 {
        return;
    }
    m.put(&[id, reg, count]);
    m.put_bytes(data, count << 4);
}

/// `gfx_cmd_set_texture` (0xD21620; EDX = stage, ECX = texture): ring 14.
fn set_texture(m: &mut Mem, stage: u32, tex: u32) {
    let c = TEXTURE_CACHE + stage as usize * 4;
    if m.rd(c) == tex {
        return;
    }
    m.wr(c, tex);
    m.put(&[14, stage, tex]);
}

/// `gfx_cmd_draw_indexed` (0xD21650; cdecl (type, start index, prim count)): ring 12
/// with the vertex count from stream 0 (buffer size - offset) / stride, then publish.
/// Nothing without a stream 0 buffer and stride. The buffer's size is read once
/// (GetDesc) into the wrapper's +8.
fn draw_indexed(m: &mut Mem, kind: u32, start: u32, prims: u32) {
    let vb = m.rd(STREAM_VB) as usize;
    if vb == 0 || m.rd(vb + 4) == 0 || m.rd(STREAM_STRIDE) == 0 {
        return;
    }
    if m.rd(vb + 8) == u32::MAX {
        let d3d = m.rd(vb + 4);
        let mut desc = [0u32; 6];
        unsafe {
            let get_desc: extern "system" fn(u32, *mut [u32; 6]) -> i32 =
                std::mem::transmute(rd(rd(d3d as usize) as usize + 0x34) as usize);
            get_desc(d3d, &mut desc);
        }
        m.wr(vb + 8, desc[4]);
    }
    let verts = (m.rd(vb + 8).wrapping_sub(m.rd(STREAM_OFFSET)) as i32) / m.rd(STREAM_STRIDE) as i32;
    m.put(&[12, kind, 0, 0, verts as u32, start, prims]);
    m.publish();
}

/// `gfx_cmd_draw` (0xD21740; ECX = type, EDX = count): ring 11, then publish. Type
/// 0x20 means `count` quads, each a 2-triangle fan of 4 vertices.
fn draw(m: &mut Mem, kind: u32, count: u32) {
    if kind == 0x20 {
        if (count as i32) > 0 {
            for i in 0..count {
                m.put(&[11, 6, i * 4, 2]);
            }
        } else {
            // The write pointer still moves by count * 16 (backwards when negative).
            let w = m.rd(RING_WRITE);
            m.wr(RING_WRITE, w.wrapping_add(count << 4));
        }
    } else {
        m.put(&[11, kind, 0, count]);
    }
    m.publish();
}

/// `gfx_cmd_draw_up` (0xD217F0; EDI = type, ECX = prim count, EDX = stride, cdecl
/// (vertices)): ring 13 with the vertices inline.
fn draw_up(m: &mut Mem, kind: u32, prims: u32, stride: u32, data: u32) {
    let verts = match kind {
        1 => prims,
        2 => prims.wrapping_add(prims),
        3 | 6 => prims.wrapping_add(1),
        4 => prims.wrapping_mul(3),
        5 => prims.wrapping_add(2),
        _ => u32::MAX,
    };
    let size = verts.wrapping_mul(stride);
    m.put(&[13, kind, prims, stride]);
    m.put_bytes(data, size);
}

/// `gfx_cmd_set_sampler_state` (0xD21890; ESI = sampler < 20, EDX = type < 16, ECX =
/// value): ring 19 when the cached value differs and the value is not 0xBAADF00D.
fn set_sampler_state(m: &mut Mem, sampler: u32, kind: u32, value: u32) {
    if sampler >= 0x14 || kind as i32 >= 0x10 {
        return;
    }
    let c = SAMPLER_CACHE + (sampler as usize * 16 + kind as usize) * 4;
    if m.rd(c) == value || value == 0xBAAD_F00D {
        return;
    }
    m.wr(c, value);
    m.put(&[19, sampler, kind, value]);
}

/// `gfx_cmd_reset_texture_cache` (0xD21940).
fn reset_texture_cache(m: &mut Mem) {
    for i in 0..16 {
        m.wr(TEXTURE_CACHE + i * 4, 0);
    }
}

/// `gfx_cmd_clear` (0xD18B30; cdecl (flags, color, z, stencil)): ring 2 with no
/// rectangles, then publish.
fn clear(m: &mut Mem, flags: u32, color: u32, z: u32, stencil: u32) {
    m.put(&[2, 0, flags, color, z, stencil]);
    m.publish();
}

/// `gfx_cmd_set_depth_stencil` (0xD1CBE0; cdecl (surface)): ring 17, then publish.
fn set_depth_stencil(m: &mut Mem, surface: u32) {
    if m.rd(DEPTH_CACHE) == surface {
        return;
    }
    m.put(&[17, surface]);
    m.wr(DEPTH_CACHE, surface);
    m.publish();
}

/// `gfx_cmd_set_render_target` (0xD1CC40; EDX = stored in the thread context's +0,
/// cdecl (surface, NULL = the current default target)): ring 16 (index 0), publish.
fn set_render_target(m: &mut Mem, edx: u32, surface: u32) {
    let s = if surface != 0 {
        surface
    } else if unsafe { rb(0x0252_A2A3) } != 0 {
        m.rd(0x0252_A2E4)
    } else {
        m.rd(0x022F_DCA8)
    };
    let ctx = thread_context() as usize;
    m.wr(ctx, edx);
    if m.rd(TARGET_CACHE) == s {
        return;
    }
    m.put(&[16, 0, s]);
    m.wr(TARGET_CACHE, s);
    m.publish();
}

/// `gfx_cmd_stretch_rect` (0xD1DA50; EDX = &src rect, ECX = &dst rect, cdecl (src,
/// dst)): ring 18, then publish.
fn stretch_rect(m: &mut Mem, src_rect: u32, dst_rect: u32, src: u32, dst: u32) {
    let r = |p: u32| -> [u32; 4] { unsafe { std::array::from_fn(|i| rd(p as usize + i * 4)) } };
    let (a, b) = (r(src_rect), r(dst_rect));
    m.put(&[18, src, a[0], a[1], a[2], a[3], dst, b[0], b[1], b[2], b[3]]);
    m.publish();
}

/// `gfx_cmd_set_viewport` (0xD1DB50; cdecl (width, height)): ring 24 {0, 0, w, h,
/// 0.0, 1.0}. No publish.
fn set_viewport(m: &mut Mem, w: u32, h: u32) {
    m.put(&[24, 0, 0, w, h, 0, 1.0f32.to_bits()]);
}

/// `gfx_cmd_set_scissor` (0xD1E7D0; cdecl (left, top, right, bottom)): while deferred
/// commands execute (0x252A2A2) the rectangle is scaled (0x22FDCC0 / 0x22FDCBC,
/// truncated); then clamped to 0 and to the thread context's target size; ring 21,
/// then publish.
fn set_scissor(m: &mut Mem, mut left: i32, mut top: i32, mut right: i32, mut bottom: i32) {
    if unsafe { rb(EXECUTING_DEFERRED) } == 1 {
        let (sx, sy) = unsafe { (*(SCISSOR_SCALE_X as *const f32) as f64, *(SCISSOR_SCALE_Y as *const f32) as f64) };
        left = (left as f64 * sx) as i32;
        right = (right as f64 * sx) as i32;
        top = (top as f64 * sy) as i32;
        bottom = (bottom as f64 * sy) as i32;
    }
    let size = m.rd(thread_context() as usize) as usize;
    let (w, h) = (m.rd(size) as i32, m.rd(size + 4) as i32);
    let clamp = |v: i32, max: i32| {
        if v < 0 {
            0
        } else if v > max {
            max
        } else {
            v
        }
    };
    let (left, right, top, bottom) = (clamp(left, w), clamp(right, w), clamp(top, h), clamp(bottom, h));
    m.put(&[21, left as u32, top as u32, right as u32, bottom as u32]);
    m.publish();
}

// --- Calling the originals ------------------------------------------------------------------

/// Call a game function with ECX/EDX/ESI/EDI set and cdecl stack arguments.
unsafe fn call_regs(f: usize, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) {
    let block: [u32; 7] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32];
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
            "call dword ptr [ebx]",
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
}

// --- Shadow check -----------------------------------------------------------------------

static TAPE_ON: AtomicBool = AtomicBool::new(false);
static TAPE_THREAD: AtomicU32 = AtomicU32::new(0);
static TAPE: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static REAL_INTERLOCKED_ADD: AtomicUsize = AtomicUsize::new(0);

extern "system" fn interlocked_add_w(p: *mut i32, v: i32) -> i32 {
    if TAPE_ON.load(Ordering::Acquire)
        && unsafe { GetCurrentThreadId() } == TAPE_THREAD.load(Ordering::Acquire)
        && p as usize == RING_PUBLISHED
    {
        TAPE.lock().unwrap_or_else(|p| p.into_inner()).push(v as u32);
    }
    let f: extern "system" fn(*mut i32, i32) -> i32 =
        unsafe { std::mem::transmute(REAL_INTERLOCKED_ADD.load(Ordering::Acquire)) };
    f(p, v)
}

/// Caches a writer may touch, watched for writes the prediction did not make.
fn watched() -> Vec<usize> {
    let mut v =
        vec![RING_WRITE, RING_LAST_PUBLISHED, VS_CACHE, PS_CACHE, DECL_CACHE, IB_CACHE, DEPTH_CACHE, TARGET_CACHE];
    v.extend((0..16).map(|i| TEXTURE_CACHE + i * 4));
    v.extend((0..4).flat_map(|i| [STREAM_VB + i * 4, STREAM_OFFSET + i * 4, STREAM_STRIDE + i * 4]));
    v.extend((0..20 * 16).map(|i| SAMPLER_CACHE + i * 4));
    v
}

static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static LOGGED: AtomicU32 = AtomicU32::new(0);

fn shadow(i: usize, rust: impl FnOnce(&mut Mem), original: impl FnOnce()) {
    let watch = watched();
    let before: Vec<u32> = watch.iter().map(|&p| unsafe { rd(p) }).collect();
    let mut m = Mem { dry: true, ..Mem::live() };
    rust(&mut m);
    TAPE.lock().unwrap_or_else(|p| p.into_inner()).clear();
    TAPE_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    TAPE_ON.store(true, Ordering::Release);
    original();
    TAPE_ON.store(false, Ordering::Release);
    let theirs = std::mem::take(&mut *TAPE.lock().unwrap_or_else(|p| p.into_inner()));
    let mut diffs = Vec::new();
    for (&p, &v) in &m.overlay {
        let o = unsafe { rd(p) };
        if o != v && diffs.len() < 6 {
            diffs.push(format!("{p:#x}: original {o:#x}, Rust {v:#x}"));
        }
    }
    for (k, &p) in watch.iter().enumerate() {
        let o = unsafe { rd(p) };
        if !m.overlay.contains_key(&p) && o != before[k] && diffs.len() < 6 {
            diffs.push(format!("{p:#x} changed to {o:#x}, Rust left it {:#x}", before[k]));
        }
    }
    if theirs != m.publishes {
        diffs.push(format!("publishes: original {theirs:x?}, Rust {:x?}", m.publishes));
    }
    if !diffs.is_empty() {
        MISMATCHES[i].fetch_add(1, Ordering::Relaxed);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 30 {
            log(&format!("gfx_cmd: {} mismatch: {}", PORTS[i].0, diffs.join("; ")));
        }
    }
}

/// One hook body: count, then original / shadow / Rust.
fn run(i: usize, rust: impl Fn(&mut Mem), original: impl FnOnce()) {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    match MODE {
        Mode::Original => original(),
        Mode::Shadow => shadow(i, rust, original),
        Mode::Rust => rust(&mut Mem::live()),
    }
}

// --- Hooks ------------------------------------------------------------------------------

/// Every hook gets (ecx, edx, esi, edi, &first stack argument); ECX, EDX, ESI and EDI
/// are preserved for the caller (the originals leave some of them alone and LTCG
/// callers may rely on that). All the writers are caller-pop.
macro_rules! entry {
    ($name:ident, $hook:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx", "push edx", "push esi", "push edi",
                "lea eax, [esp + 20]",
                "push eax", "push edi", "push esi", "push edx", "push ecx",
                "call {h}",
                "add esp, 20",
                "pop edi", "pop esi", "pop edx", "pop ecx",
                "ret",
                h = sym $hook,
            )
        }
    };
}

type Regs = (u32, u32, u32, u32, *const u32);

fn arg(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

fn original(i: usize, r: Regs, n: usize) -> impl FnOnce() {
    move || unsafe {
        let args: Vec<u32> = (0..n).map(|k| arg(r.4, k)).collect();
        call_regs(ORIG[i].load(Ordering::Acquire), r.0, r.1, r.2, r.3, &args)
    }
}

macro_rules! hook {
    ($entry:ident, $hook:ident, $idx:expr, $nargs:expr, |$m:ident, $ecx:ident, $edx:ident, $esi:ident, $edi:ident, $a:ident| $body:expr) => {
        extern "C" fn $hook($ecx: u32, $edx: u32, $esi: u32, $edi: u32, $a: *const u32) {
            let r = ($ecx, $edx, $esi, $edi, $a);
            run($idx, |$m: &mut Mem| $body, original($idx, r, $nargs));
        }
        entry!($entry, $hook);
    };
}

hook!(e_stream, h_stream, STREAM, 0, |m, ecx, edx, esi, edi, _a| set_stream_source(m, ecx, edi, edx, esi));
hook!(e_indices, h_indices, INDICES, 0, |m, ecx, _edx, _esi, _edi, _a| set_indices(m, ecx));
hook!(e_decl, h_decl, DECL, 0, |m, ecx, _edx, _esi, _edi, _a| set_vertex_declaration(m, ecx));
hook!(e_ps, h_ps, PIXEL_SHADER, 0, |m, ecx, _edx, _esi, _edi, _a| set_shader(m, false, ecx));
hook!(e_vs, h_vs, VERTEX_SHADER, 0, |m, ecx, _edx, _esi, _edi, _a| set_shader(m, true, ecx));
hook!(e_cmd9, h_cmd9, CMD_9, 1, |m, _ecx, edx, _esi, _edi, a| cmd_9(m, arg(a, 0), edx));
hook!(e_psc_u, h_psc_u, PS_CONSTANTS_UNCHECKED, 1, |m, ecx, edx, _esi, _edi, a| set_constants(
    m,
    7,
    false,
    ecx,
    edx,
    arg(a, 0)
));
hook!(e_psc, h_psc, PS_CONSTANTS, 1, |m, ecx, edx, _esi, _edi, a| set_constants(m, 7, true, ecx, edx, arg(a, 0)));
hook!(e_vsc, h_vsc, VS_CONSTANTS, 1, |m, ecx, edx, _esi, _edi, a| set_constants(m, 6, false, ecx, edx, arg(a, 0)));
hook!(e_texture, h_texture, TEXTURE, 0, |m, ecx, edx, _esi, _edi, _a| set_texture(m, edx, ecx));
hook!(e_draw_indexed, h_draw_indexed, DRAW_INDEXED, 3, |m, _ecx, _edx, _esi, _edi, a| draw_indexed(
    m,
    arg(a, 0),
    arg(a, 1),
    arg(a, 2)
));
hook!(e_draw, h_draw, DRAW, 0, |m, ecx, edx, _esi, _edi, _a| draw(m, ecx, edx));
hook!(e_draw_up, h_draw_up, DRAW_UP, 1, |m, ecx, edx, _esi, edi, a| draw_up(m, edi, ecx, edx, arg(a, 0)));
hook!(e_sampler, h_sampler, SAMPLER, 0, |m, ecx, edx, esi, _edi, _a| set_sampler_state(m, esi, edx, ecx));
hook!(e_reset_tex, h_reset_tex, RESET_TEXTURES, 0, |m, _ecx, _edx, _esi, _edi, _a| reset_texture_cache(m));
hook!(e_clear, h_clear, CLEAR, 4, |m, _ecx, _edx, _esi, _edi, a| clear(m, arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3)));
hook!(e_depth, h_depth, DEPTH_STENCIL, 1, |m, _ecx, _edx, _esi, _edi, a| set_depth_stencil(m, arg(a, 0)));
hook!(e_target, h_target, RENDER_TARGET, 1, |m, _ecx, edx, _esi, _edi, a| set_render_target(m, edx, arg(a, 0)));
hook!(e_stretch, h_stretch, STRETCH, 2, |m, ecx, edx, _esi, _edi, a| stretch_rect(m, edx, ecx, arg(a, 0), arg(a, 1)));
hook!(e_viewport, h_viewport, VIEWPORT, 2, |m, _ecx, _edx, _esi, _edi, a| set_viewport(m, arg(a, 0), arg(a, 1)));
hook!(e_scissor, h_scissor, SCISSOR, 4, |m, _ecx, _edx, _esi, _edi, a| set_scissor(
    m,
    arg(a, 0) as i32,
    arg(a, 1) as i32,
    arg(a, 2) as i32,
    arg(a, 3) as i32
));

const STREAM: usize = 0;
const INDICES: usize = 1;
const DECL: usize = 2;
const PIXEL_SHADER: usize = 3;
const VERTEX_SHADER: usize = 4;
const CMD_9: usize = 5;
const PS_CONSTANTS_UNCHECKED: usize = 6;
const PS_CONSTANTS: usize = 7;
const VS_CONSTANTS: usize = 8;
const TEXTURE: usize = 9;
const DRAW_INDEXED: usize = 10;
const DRAW: usize = 11;
const DRAW_UP: usize = 12;
const SAMPLER: usize = 13;
const RESET_TEXTURES: usize = 14;
const CLEAR: usize = 15;
const DEPTH_STENCIL: usize = 16;
const RENDER_TARGET: usize = 17;
const STRETCH: usize = 18;
const VIEWPORT: usize = 19;
const SCISSOR: usize = 20;
const COUNT: usize = 21;

/// (name, address, the bytes the detour steals (whole instructions, no relative branches)).
const PORTS: [(&str, usize, &[u8]); COUNT] = [
    ("gfx_cmd_set_stream_source", 0x00D2_1310, &[0x3B, 0x3C, 0x8D, 0x8C, 0x4D, 0x30, 0x02]),
    ("gfx_cmd_set_indices", 0x00D2_13B0, &[0x39, 0x0D, 0x10, 0xA3, 0x52, 0x02]),
    ("gfx_cmd_set_vertex_declaration", 0x00D2_1450, &[0x3B, 0x0D, 0x0C, 0xA3, 0x52, 0x02]),
    ("gfx_cmd_set_pixel_shader", 0x00D2_1480, &BODY_SET_PIXEL_SHADER),
    ("gfx_cmd_set_vertex_shader", 0x00D2_14B0, &BODY_SET_VERTEX_SHADER),
    ("gfx_cmd_9", 0x00D2_14E0, &[0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_ps_constants_unchecked", 0x00D2_1520, &[0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_ps_constants", 0x00D2_1560, &BODY_SET_PS_CONSTANTS),
    ("gfx_cmd_set_vs_constants", 0x00D2_15E0, &[0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_texture", 0x00D2_1620, &[0x39, 0x0C, 0x95, 0xF0, 0x2A, 0x2F, 0x02]),
    ("gfx_cmd_draw_indexed", 0x00D2_1650, &[0x8B, 0x0D, 0x8C, 0x4D, 0x30, 0x02]),
    ("gfx_cmd_draw", 0x00D2_1740, &[0x83, 0xF9, 0x20, 0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_draw_up", 0x00D2_17F0, &[0x8D, 0x47, 0xFF, 0x83, 0xF8, 0x05]),
    ("gfx_cmd_set_sampler_state", 0x00D2_1890, &BODY_SET_SAMPLER_STATE),
    ("gfx_cmd_reset_texture_cache", 0x00D2_1940, &[0x33, 0xC0, 0xA3, 0xF0, 0x2A, 0x2F, 0x02]),
    ("gfx_cmd_clear", 0x00D1_8B30, &[0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_depth_stencil", 0x00D1_CBE0, &[0x8B, 0x44, 0x24, 0x04, 0x39, 0x05, 0x08, 0xA3, 0x52, 0x02]),
    ("gfx_cmd_set_render_target", 0x00D1_CC40, &[0x8B, 0x44, 0x24, 0x04, 0x85, 0xC0]),
    ("gfx_cmd_stretch_rect", 0x00D1_DA50, &[0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_viewport", 0x00D1_DB50, &[0x83, 0xEC, 0x18, 0xA1, 0xF8, 0x62, 0x3D, 0x03]),
    ("gfx_cmd_set_scissor", 0x00D1_E7D0, &[0x80, 0x3D, 0xA2, 0xA2, 0x52, 0x02, 0x01]),
];

/// Whole bodies of the writers whose first five bytes hold a short branch: these are
/// relocated (kept callable) and replaced, like `input_key_down`.
const BODY_SET_PIXEL_SHADER: [u8; 46] = [
    0x85, 0xC9, 0x75, 0x06, 0x8B, 0x0D, 0x9C, 0x4B, 0xE8, 0x00, 0x39, 0x0D, 0xE8, 0x2A, 0x2F, 0x02, 0x74, 0x1B, 0xA1,
    0xF8, 0x62, 0x3D, 0x03, 0xC7, 0x00, 0x03, 0x00, 0x00, 0x00, 0x89, 0x48, 0x04, 0x83, 0x05, 0xF8, 0x62, 0x3D, 0x03,
    0x08, 0x89, 0x0D, 0xE8, 0x2A, 0x2F, 0x02, 0xC3,
];
const BODY_SET_VERTEX_SHADER: [u8; 46] = [
    0x85, 0xC9, 0x75, 0x06, 0x8B, 0x0D, 0x98, 0x4B, 0xE8, 0x00, 0x39, 0x0D, 0xEC, 0x2A, 0x2F, 0x02, 0x74, 0x1B, 0xA1,
    0xF8, 0x62, 0x3D, 0x03, 0xC7, 0x00, 0x04, 0x00, 0x00, 0x00, 0x89, 0x48, 0x04, 0x83, 0x05, 0xF8, 0x62, 0x3D, 0x03,
    0x08, 0x89, 0x0D, 0xEC, 0x2A, 0x2F, 0x02, 0xC3,
];
const BODY_SET_PS_CONSTANTS: [u8; 62] = [
    0x85, 0xD2, 0x7C, 0x39, 0xA1, 0xF8, 0x62, 0x3D, 0x03, 0x89, 0x48, 0x08, 0x56, 0xC1, 0xE1, 0x04, 0x8B, 0xF1, 0x8B,
    0x4C, 0x24, 0x08, 0x56, 0xC7, 0x00, 0x07, 0x00, 0x00, 0x00, 0x89, 0x50, 0x04, 0x51, 0x83, 0xC0, 0x0C, 0x50, 0xE8,
    0xC6, 0xF2, 0xF7, 0xFF, 0x8B, 0x15, 0xF8, 0x62, 0x3D, 0x03, 0x8D, 0x44, 0x32, 0x0C, 0x83, 0xC4, 0x0C, 0xA3, 0xF8,
    0x62, 0x3D, 0x03, 0x5E, 0xC3,
];
const BODY_SET_SAMPLER_STATE: [u8; 71] = [
    0x83, 0xFE, 0x14, 0x73, 0x41, 0x83, 0xFA, 0x10, 0x7D, 0x3C, 0x8B, 0xC6, 0xC1, 0xE0, 0x04, 0x03, 0xC2, 0x39, 0x0C,
    0x85, 0xF0, 0x52, 0x2F, 0x02, 0x8D, 0x04, 0x85, 0xF0, 0x52, 0x2F, 0x02, 0x74, 0x25, 0x81, 0xF9, 0x0D, 0xF0, 0xAD,
    0xBA, 0x74, 0x1D, 0x89, 0x08, 0xA1, 0xF8, 0x62, 0x3D, 0x03, 0xC7, 0x00, 0x13, 0x00, 0x00, 0x00, 0x89, 0x70, 0x04,
    0x89, 0x50, 0x08, 0x89, 0x48, 0x0C, 0x83, 0x05, 0xF8, 0x62, 0x3D, 0x03, 0x10, 0xC3,
];
/// The relocated bodies' `call rel32` offsets (memcpy in gfx_cmd_set_ps_constants).
const RELOCATED_CALLS: [(usize, &[usize]); 4] =
    [(PIXEL_SHADER, &[]), (VERTEX_SHADER, &[]), (PS_CONSTANTS, &[37]), (SAMPLER, &[])];

static ORIG: [AtomicUsize; COUNT] = [const { AtomicUsize::new(0) }; COUNT];

fn entry_address(i: usize) -> usize {
    let f: extern "C" fn() = match i {
        STREAM => e_stream,
        INDICES => e_indices,
        DECL => e_decl,
        PIXEL_SHADER => e_ps,
        VERTEX_SHADER => e_vs,
        CMD_9 => e_cmd9,
        PS_CONSTANTS_UNCHECKED => e_psc_u,
        PS_CONSTANTS => e_psc,
        VS_CONSTANTS => e_vsc,
        TEXTURE => e_texture,
        DRAW_INDEXED => e_draw_indexed,
        DRAW => e_draw,
        DRAW_UP => e_draw_up,
        SAMPLER => e_sampler,
        RESET_TEXTURES => e_reset_tex,
        CLEAR => e_clear,
        DEPTH_STENCIL => e_depth,
        RENDER_TARGET => e_target,
        STRETCH => e_stretch,
        VIEWPORT => e_viewport,
        SCISSOR => e_scissor,
        _ => unreachable!(),
    };
    f as usize
}

pub fn install() {
    if !ENABLED {
        return;
    }
    if MODE == Mode::Shadow {
        unsafe {
            let mut old = 0u32;
            if VirtualProtect(SLOT_INTERLOCKED_ADD as *const _, 4, PAGE_READWRITE, &mut old) == 0 {
                log("gfx_cmd: could not wrap InterlockedExchangeAdd; not hooking");
                return;
            }
            REAL_INTERLOCKED_ADD.store(rd(SLOT_INTERLOCKED_ADD) as usize, Ordering::Release);
            wr(SLOT_INTERLOCKED_ADD, interlocked_add_w as extern "system" fn(*mut i32, i32) -> i32 as usize as u32);
            VirtualProtect(SLOT_INTERLOCKED_ADD as *const _, 4, old, &mut old);
        }
    }
    let mut ok = 0;
    for (i, &(name, addr, bytes)) in PORTS.iter().enumerate() {
        let relocated = RELOCATED_CALLS.iter().find(|r| r.0 == i).map(|r| r.1);
        let r = unsafe {
            match relocated {
                Some(calls) => detour::relocate(addr, bytes, calls).and_then(|copy| {
                    detour::replace(addr, bytes, entry_address(i))?;
                    Ok(copy)
                }),
                None => detour::install(addr, bytes, entry_address(i)),
            }
        };
        match r {
            Ok(t) => {
                ORIG[i].store(t, Ordering::Release);
                ok += 1;
            }
            Err(e) => log(&format!("gfx_cmd: hooking {name} @ {addr:#x} failed: {e:?}")),
        }
    }
    log(&format!("replaced {ok}/{COUNT} ring writers"));
    let _ = RING_BASE;
}

pub fn summary() -> String {
    if !ENABLED {
        return "gfx_cmd: off".into();
    }
    let parts: Vec<String> = PORTS
        .iter()
        .enumerate()
        .filter_map(|(i, &(name, ..))| {
            let c = CALLS[i].load(Ordering::Relaxed);
            (c > 0).then(|| format!("{name} {c} ({} mismatched)", MISMATCHES[i].load(Ordering::Relaxed)))
        })
        .collect();
    format!("gfx_cmd live: {}", parts.join(", "))
}
