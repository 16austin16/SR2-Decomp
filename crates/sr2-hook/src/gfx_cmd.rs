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
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

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
const ENABLED: bool = true;
const MODE: Mode = Mode::Rust;

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
pub(crate) const DEFERRED_QUEUE: usize = 0x00CF_8EA0;

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
pub(crate) fn thread_context() -> u32 {
    let tls: u32;
    unsafe {
        std::arch::asm!("mov {}, dword ptr fs:[0x2C]", out(reg) tls, options(nostack, readonly));
        let t = rd(tls as usize) as usize;
        let ctx = rd(t + 0x60);
        if rb(t + 0x5C) == 0 && ctx == GAME_CONTEXT { RENDER_CONTEXT } else { ctx }
    }
}

// --- Memory: live, or predicting into an overlay ---------------------------------------

pub(crate) struct Mem {
    pub(crate) dry: bool,
    pub(crate) overlay: BTreeMap<usize, u32>,
    /// Dry: what was published or queued, as tape entries ([0, delta] / [1, type, args...]).
    pub(crate) tape: Vec<Vec<u32>>,
}

impl Mem {
    pub(crate) fn live() -> Self {
        Mem { dry: false, overlay: BTreeMap::new(), tape: Vec::new() }
    }

    pub(crate) fn rd(&self, p: usize) -> u32 {
        if self.dry
            && let Some(&v) = self.overlay.get(&p)
        {
            return v;
        }
        unsafe { rd(p) }
    }

    pub(crate) fn wr(&mut self, p: usize, v: u32) {
        if self.dry {
            self.overlay.insert(p, v);
        } else {
            unsafe { wr(p, v) }
        }
    }

    /// Append dwords at the ring's write pointer and advance it.
    pub(crate) fn put(&mut self, words: &[u32]) {
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
                // The source may itself hold predicted writes (the 2D vertices).
                let s = if (src as usize + i) % 4 == 0 { self.rd(src as usize + i).to_le_bytes() } else { [0; 4] };
                for b in 0..take {
                    v[b] = if (src as usize + i) % 4 == 0 {
                        s[b]
                    } else {
                        unsafe { *((src as usize + i + b) as *const u8) }
                    };
                }
                self.wr(w + i, u32::from_le_bytes(v));
                i += 4;
            }
        } else {
            unsafe { std::ptr::copy_nonoverlapping(src as *const u8, w as *mut u8, n as usize) };
        }
        self.wr(RING_WRITE, (w + n as usize) as u32);
    }

    /// `render_deferred_queue(type, args...)` (0xCF8EA0, cdecl varargs; floats as
    /// doubles): the API call is recorded for the render thread instead of run.
    pub(crate) fn queue(&mut self, kind: u32, args: &[u32]) {
        if self.dry {
            let mut e = vec![1, kind];
            e.extend_from_slice(args);
            self.tape.push(e);
            let buf: Vec<u32> = args.iter().copied().chain(std::iter::repeat(0).take(32)).collect();
            deferred_queue(self, kind, buf.as_ptr() as usize);
        } else {
            let mut a = vec![kind];
            a.extend_from_slice(args);
            unsafe { call_regs(DEFERRED_QUEUE, 0, 0, 0, 0, 0, &a) };
        }
    }

    /// Publish everything written since the last publish.
    fn publish(&mut self) {
        let delta = self.rd(RING_WRITE).wrapping_sub(self.rd(RING_LAST_PUBLISHED));
        if self.dry {
            self.tape.push(vec![0, delta]);
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

// --- The graphics API layer (18_scene.md) -------------------------------------------------
//
// Every API function first resolves the calling thread's context (the game thread's
// writes go to the render context) and, on the game thread and unless "force
// immediate" (0xE849DB) is set, records itself in the deferred list for the render
// thread instead of writing the ring.

const FORCE_IMMEDIATE: usize = 0x00E8_49DB;
const RENDER_STATE_CACHE: usize = 0x022F_57F0; // [D3DRENDERSTATETYPE]
const BITMAP_CACHE: usize = 0x00E8_4AA8; // [16] stage -> bitmap handle
const DEPTH_BIAS_CACHE: usize = 0x022F_5AFC; // RENDER_STATE_CACHE[0xC3]
const SLOPE_BIAS_CACHE: usize = 0x022F_5AAC; // RENDER_STATE_CACHE[0xAF]
const STRIP_COUNT: usize = 0x00E8_63C4; // vertices added since im2d_strip_begin (-1 = none)
const LIST_COUNT: usize = 0x00E8_63C8;
const VERTS_PENDING: usize = 0x0252_A2FC;
const VERTS_COUNT: usize = 0x0252_A2F8;
const VERTS: usize = 0x022F_6438; // 0x1C-byte 2D vertices
const COLOR_SCALE: usize = 0x00E5_C2B0; // double 1/255
const DEFAULT_BLOCK: u32 = 0x00EC_2748;

const GET_THREAD_STATE: usize = 0x00D2_2790;
const BITMAP_LOOKUP: usize = 0x00C0_94E0; // EAX = handle -> bitmap record or NULL

/// The calling thread's raw context and its "render thread" byte.
fn raw_context() -> (u32, u8) {
    let tls: u32;
    unsafe {
        std::arch::asm!("mov {}, dword ptr fs:[0x2C]", out(reg) tls, options(nostack, readonly));
        let t = rd(tls as usize) as usize;
        (rd(t + 0x60), rb(t + 0x5C))
    }
}

/// Whether an API call on this thread is deferred to the render thread.
fn defers() -> bool {
    let (raw, flag) = raw_context();
    (unsafe { rb(FORCE_IMMEDIATE) }) == 0 && (raw == RENDER_CONTEXT || (flag == 0 && raw == GAME_CONTEXT))
}

fn thread_state() -> usize {
    unsafe { std::mem::transmute::<usize, extern "C" fn() -> u32>(GET_THREAD_STATE)() as usize }
}

fn f64_args(v: f32) -> [u32; 2] {
    let d = (v as f64).to_bits();
    [d as u32, (d >> 32) as u32]
}

/// A render state through the cache (ring 15 when it changes).
fn render_state(m: &mut Mem, state: u32, v: u32) {
    let c = RENDER_STATE_CACHE + state as usize * 4;
    if m.rd(c) != v {
        m.wr(c, v);
        m.put(&[15, state, v]);
    }
}

/// A sampler 0 state through the cache (ring 19 when it changes).
fn sampler0_state(m: &mut Mem, kind: u32, v: u32) {
    let c = SAMPLER_CACHE + kind as usize * 4;
    if m.rd(c) != v {
        m.wr(c, v);
        m.put(&[19, 0, kind, v]);
    }
}

/// `gfx_set_texture_cached` (0x4D0D70; ESI = stage, EDI = bitmap handle): bind the
/// bitmap's texture unless the stage already shows that bitmap.
fn set_texture_cached(m: &mut Mem, stage: u32, handle: u32) {
    let bc = BITMAP_CACHE + stage as usize * 4;
    if m.rd(bc) == handle {
        return;
    }
    let bitmap = unsafe { call_regs(BITMAP_LOOKUP, handle, 0, 0, 0, 0, &[]) } as usize;
    let c = TEXTURE_CACHE + stage as usize * 4;
    let tex = if bitmap != 0 { m.rd(m.rd(bitmap + 4) as usize + 0x2C) } else { 0 };
    if m.rd(c) != tex {
        m.wr(c, tex);
        m.put(&[14, stage, tex]);
    }
    m.wr(bc, handle);
}

/// `gfx_apply_render_state_block` (0xD1CDB0; ECX = &{a, b}): a's nibbles select the
/// blend mode, the stage 0 address mode, the depth mode and the alpha-test / stencil
/// mode; b's low nibble the cull mode. Each sets render states through the cache.
fn apply_render_state_block(m: &mut Mem, a: u32, b: u32) {
    const ALPHABLENDENABLE: u32 = 0x1B;
    const SRCBLEND: u32 = 0x13;
    const DESTBLEND: u32 = 0x14;
    const BLENDOP: u32 = 0xAB;
    let blend: &[(u32, u32)] = match a & 0xF {
        0 => &[(ALPHABLENDENABLE, 0)],
        1 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 2), (DESTBLEND, 2), (BLENDOP, 1)],
        2 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 5), (DESTBLEND, 2), (BLENDOP, 1)],
        4 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 7), (DESTBLEND, 2), (BLENDOP, 1)],
        5 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 5), (DESTBLEND, 6)],
        6 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 7), (DESTBLEND, 8)],
        7 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 8), (DESTBLEND, 7)],
        9 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 2), (DESTBLEND, 2), (BLENDOP, 3)],
        10 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 5), (DESTBLEND, 2), (BLENDOP, 3)],
        12 => &[(ALPHABLENDENABLE, 1), (SRCBLEND, 9), (DESTBLEND, 1), (BLENDOP, 1)],
        _ => &[],
    };
    for &(s, v) in blend {
        render_state(m, s, v);
    }
    match a >> 4 & 0xF {
        0 => {
            sampler0_state(m, 1, 1); // ADDRESSU / V = WRAP
            sampler0_state(m, 2, 1);
        }
        1 => {
            sampler0_state(m, 1, 3); // CLAMP
            sampler0_state(m, 2, 3);
        }
        _ => {}
    }
    const ZENABLE: u32 = 7;
    const ZFUNC: u32 = 0x17;
    const ZWRITEENABLE: u32 = 0xE;
    let depth: &[(u32, u32)] = match a >> 8 & 0xF {
        0 => &[(ZENABLE, 0)],
        1 => &[(ZENABLE, 1), (ZFUNC, 4), (ZWRITEENABLE, 0)],
        2 => &[(ZENABLE, 1), (ZFUNC, 3), (ZWRITEENABLE, 0)],
        4 => &[(ZENABLE, 1), (ZFUNC, 4), (ZWRITEENABLE, 1)],
        _ => &[],
    };
    for &(s, v) in depth {
        render_state(m, s, v);
    }
    let stencil: &[(u32, u32)] = match a >> 12 & 0xF {
        0 => &[(0x34, 0)],
        1 => &[(0xF, 1), (0x19, 5), (0x18, 0), (0x34, 1), (0x38, 8), (0x37, 3), (0x39, 1)],
        2 => &[(0x34, 1), (0x38, 3), (0x39, 1), (0x37, 1), (0x35, 1), (0x36, 1)],
        _ => &[],
    };
    for &(s, v) in stencil {
        render_state(m, s, v);
    }
    match b & 0xF {
        0 => render_state(m, 0x16, 1), // CULLMODE = NONE
        1 => render_state(m, 0x16, 3), // CCW
        _ => {}
    }
}

/// `gfx_set_render_state_block` (0xD148D0; ECX = &{a, b}): applied when it differs
/// from the context's (+0x20 / +0x24).
fn set_render_state_block(m: &mut Mem, p: u32) {
    let ctx = thread_context() as usize;
    let (a, b) = (m.rd(p as usize), m.rd(p as usize + 4));
    if m.rd(ctx + 0x20) == a && m.rd(ctx + 0x24) == b {
        return;
    }
    m.wr(ctx + 0x20, a);
    m.wr(ctx + 0x24, b);
    if defers() {
        m.queue(3, &[p]);
    } else {
        apply_render_state_block(m, a, b);
    }
}

/// `gfx_invalidate_render_state_block` (0xD14950).
fn invalidate_render_state_block(m: &mut Mem) {
    let ctx = thread_context() as usize;
    m.wr(ctx + 0x20, u32::MAX);
    m.wr(ctx + 0x24, u32::MAX);
    if defers() {
        m.queue(2, &[]);
    }
}

/// `gfx_set_viewport_rect` (0xD149B0; EDI = y, cdecl (x, w, h, no_clamp)): the
/// context's +8..+0x1C = x0, x1, y0, y1 (clamped to the target unless no_clamp), w, h.
fn set_viewport_rect(m: &mut Mem, y: i32, x: i32, w: i32, h: i32, no_clamp: u32) {
    let ctx = thread_context() as usize;
    let (x1, y1) = (x.wrapping_add(w).wrapping_sub(1), y.wrapping_add(h).wrapping_sub(1));
    m.wr(ctx + 8, x as u32);
    m.wr(ctx + 0xC, x1 as u32);
    m.wr(ctx + 0x10, y as u32);
    m.wr(ctx + 0x14, y1 as u32);
    if no_clamp == 0 {
        let size = m.rd(ctx) as usize;
        let (wm, hm) = ((m.rd(size) as i32).wrapping_sub(1), (m.rd(size + 4) as i32).wrapping_sub(1));
        let clamp = |v: i32, max: i32| {
            if v < 0 {
                0
            } else if v > max {
                max
            } else {
                v
            }
        };
        m.wr(ctx + 8, clamp(x, wm) as u32);
        m.wr(ctx + 0xC, clamp(x1, wm) as u32);
        m.wr(ctx + 0x10, clamp(y, hm) as u32);
        m.wr(ctx + 0x14, clamp(y1, hm) as u32);
    }
    let width = m.rd(ctx + 0xC).wrapping_sub(m.rd(ctx + 8)).wrapping_add(1);
    let height = m.rd(ctx + 0x14).wrapping_sub(m.rd(ctx + 0x10)).wrapping_add(1);
    m.wr(ctx + 0x18, width);
    m.wr(ctx + 0x1C, height);
    if defers() {
        m.queue(0, &[x as u32, y as u32, w as u32, h as u32, no_clamp]);
    }
}

/// `gfx_reset_viewport` (0xD14D90): the viewport rect to the whole target.
fn reset_viewport(m: &mut Mem) {
    let size = m.rd(thread_context() as usize) as usize;
    let (w, h) = (m.rd(size), m.rd(size + 4));
    set_viewport_rect(m, 0, 0, w as i32, h as i32, 0);
}

/// `gfx_set_scissor_rect` (0xD14D20; EDI = left, ESI = right, cdecl (top, bottom)):
/// ignored when inverted; stored in the context (+0x44..+0x50), then the scissor.
fn set_scissor_rect(m: &mut Mem, left: i32, right: i32, top: i32, bottom: i32) {
    if right < left || bottom < top {
        return;
    }
    let ctx = thread_context() as usize;
    m.wr(ctx + 0x44, left as u32);
    m.wr(ctx + 0x48, right as u32);
    m.wr(ctx + 0x4C, top as u32);
    m.wr(ctx + 0x50, bottom as u32);
    if defers() {
        m.queue(1, &[left as u32, top as u32, right as u32, bottom as u32]);
    } else {
        set_scissor(m, left, top, right, bottom);
    }
}

/// `gfx_set_color` (0xD14840; cdecl (r, g, b, a) 0..255) -> the context's +0x28..+0x34.
fn set_color(m: &mut Mem, c: [u32; 4]) {
    let ctx = thread_context() as usize;
    let k = unsafe { *(COLOR_SCALE as *const f64) };
    for (i, &v) in c.iter().enumerate() {
        m.wr(ctx + 0x28 + i * 4, ((v as i32 as f64 * k) as f32).to_bits());
    }
    if defers() {
        m.queue(4, &c);
    }
}

/// `gfx_set_alpha` (0xD147E0; cdecl (a)) -> +0x34.
fn set_alpha(m: &mut Mem, a: u32) {
    let ctx = thread_context() as usize;
    let k = unsafe { *(COLOR_SCALE as *const f64) };
    m.wr(ctx + 0x34, ((a as i32 as f64 * k) as f32).to_bits());
    if defers() {
        m.queue(5, &[a]);
    }
}

/// `gfx_set_bitmap` (0x56C2B0; ESI = bitmap handle) -> +0x38.
fn set_bitmap(m: &mut Mem, h: u32) {
    let ctx = thread_context() as usize;
    m.wr(ctx + 0x38, h);
    if defers() {
        m.queue(6, &[h]);
    }
}

/// The depth bias stacks (16 deep, in the thread state): push / pop, then the render
/// state = the sum of the stack (unless it is the cache's "unset" 0xBAADF00D).
fn bias(m: &mut Mem, slope: bool, push: Option<f32>) {
    let (kind, count_off, stack_off, cache, state) = if slope {
        (if push.is_some() { 0xA } else { 0xB }, 0x170C, 0x16CC, SLOPE_BIAS_CACHE, 0xAF)
    } else {
        (if push.is_some() { 8 } else { 9 }, 0x16C8, 0x1688, DEPTH_BIAS_CACHE, 0xC3)
    };
    if defers() {
        match push {
            Some(v) => m.queue(kind, &f64_args(v)),
            None => m.queue(kind, &[]),
        }
        return;
    }
    let st = thread_state();
    let n = m.rd(st + count_off);
    match push {
        Some(v) => {
            if n != 16 {
                m.wr(st + stack_off + n as usize * 4, v.to_bits());
                m.wr(st + count_off, n + 1);
            }
        }
        None => {
            if n != 0 {
                m.wr(st + count_off, n - 1);
            }
        }
    }
    let n = m.rd(st + count_off) as i32;
    let mut sum = 0.0f32;
    for i in 0..n.max(0) as usize {
        sum += f32::from_bits(m.rd(st + stack_off + i * 4));
    }
    let bits = sum.to_bits();
    if m.rd(cache) != bits && bits != 0xBAAD_F00D {
        m.wr(cache, bits);
        m.put(&[15, state, bits]);
    }
}

/// `gfx_clear` (0xD14AD0; ECX = value): clear the stencil to it.
fn api_clear(m: &mut Mem, value: u32) {
    if defers() {
        m.queue(0x11, &[value]);
    } else {
        clear(m, 4, 0, 1.0f32.to_bits(), value);
    }
}

/// `gfx_clear_region` (0xD14B20; CL, DL, cdecl (a, b) bytes): clear target, depth and
/// stencil to color cl << 24 | a << 16 | b << 8 | dl, then re-apply the default state
/// block.
fn clear_region(m: &mut Mem, cl: u32, dl: u32, a: u32, b: u32) {
    let (cl, dl, a, b) = (cl & 0xFF, dl & 0xFF, a & 0xFF, b & 0xFF);
    if defers() {
        m.queue(0x10, &[a, b, dl, cl]);
        return;
    }
    clear(m, 7, cl << 24 | a << 16 | b << 8 | dl, 1.0f32.to_bits(), 0);
    invalidate_render_state_block(m);
    set_render_state_block(m, DEFAULT_BLOCK);
}

/// `gfx_restore_default_targets` (0xD1EF40): the main render target, depth surface,
/// a full viewport, and the viewport rect reset.
fn restore_default_targets(m: &mut Mem) {
    let (target, depth) = (m.rd(0x022F_DCB4), m.rd(0x022F_DCB0));
    set_render_target(m, m.rd(0x022F_DCAC), target);
    set_depth_stencil(m, depth);
    set_viewport(m, m.rd(0x022F_63F8), m.rd(0x022F_63FC));
    reset_viewport(m);
}

/// `gfx_2d_flush` (0xD1DF50; EAX = primitive type, cdecl (count)): finish the pending
/// 2D vertices (scaled while deferred commands execute; z and rhw 1.0), draw them
/// (unless the type is 0x20), and point sampler 0 back at linear filtering.
fn flush_2d(m: &mut Mem, kind: u32, prims: u32) {
    if m.rd(VERTS_PENDING) == 0 {
        return;
    }
    m.wr(VERTS_PENDING, 0);
    let n = m.rd(VERTS_COUNT) as i32;
    if n > 0 {
        let (sx, sy) = unsafe { (*(SCISSOR_SCALE_X as *const f32), *(SCISSOR_SCALE_Y as *const f32)) };
        let scale = unsafe { rb(EXECUTING_DEFERRED) } == 1;
        for i in 0..n as usize {
            let v = VERTS + i * 0x1C;
            if scale {
                m.wr(v, (f32::from_bits(m.rd(v)) * sx).to_bits());
                m.wr(v + 4, (f32::from_bits(m.rd(v + 4)) * sy).to_bits());
            }
            m.wr(v + 8, 1.0f32.to_bits());
            m.wr(v + 0xC, 1.0f32.to_bits());
        }
    }
    if kind != 0x20 {
        draw_up(m, kind, prims, 0x1C, VERTS as u32);
    }
    sampler0_state(m, 7, 2); // MIPFILTER = LINEAR
    sampler0_state(m, 5, 2); // MAGFILTER = LINEAR
    for i in 0..4 {
        m.wr(STREAM_VB + i * 4, 0);
        m.wr(STREAM_OFFSET + i * 4, 0);
    }
}

/// `im2d_strip_begin` / `im2d_list_begin` (0xD0A440 / 0xD0A2C0; ECX = &state pair):
/// the state block, the context's bitmap on stage 0, and an empty batch.
fn batch_begin(m: &mut Mem, list: bool, block: u32) {
    if defers() {
        m.queue(if list { 0x1E } else { 0x1B }, &[block]);
        return;
    }
    set_render_state_block(m, block);
    let h = m.rd(thread_context() as usize + 0x38);
    set_texture_cached(m, 0, h);
    m.wr(if list { LIST_COUNT } else { STRIP_COUNT }, u32::MAX);
}

/// `im2d_strip_end` / `im2d_list_end` (0xD0A330 / 0xD0A190): flush the batch as a
/// triangle strip (n - 2) or list (n / 3).
fn batch_end(m: &mut Mem, list: bool) {
    if defers() {
        m.queue(if list { 0x20 } else { 0x1D }, &[]);
        return;
    }
    if list {
        let n = m.rd(LIST_COUNT) as i32;
        if n >= 3 {
            flush_2d(m, 4, (n / 3) as u32);
        }
    } else {
        let n = m.rd(STRIP_COUNT) as i32;
        if n >= 0 {
            flush_2d(m, 5, n.wrapping_sub(2) as u32);
        }
    }
}

// --- The deferred command lists (18_scene.md "The deferred command lists") -----------------
//
// Two lists of 3000 records {type, offset} with a 0x16800-byte data area. API calls
// on the game thread are recorded into the game-side list by `render_deferred_queue`,
// whose per-type handler copies the arguments (mode 0); `render_deferred_execute`
// replays the render-side list on the render thread (handler mode 1 calls the API).

const LIST_GAME: usize = 0x0230_597C; // the list being recorded
const LIST_RENDER: usize = 0x0230_5978; // the list being executed
const HANDLERS: usize = 0x00E8_47E0; // [43] bool cdecl (mode, ctx *{data, used, space}, args)
const L_COUNT: usize = 0x5DC0;
const L_DATA: usize = 0x5DC4;
const L_USED: usize = 0x1_C5C4;
const L_SPACE: u32 = 0x1_6800;
const L_MAX: u32 = 3000;

/// How a handler records its arguments.
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)] // Clip waits for an in-game check
enum Rec {
    /// Nothing.
    None,
    /// n dwords from the arguments, each written only if it still fits.
    Dwords(usize),
    /// A double argument stored as a float.
    Float,
    /// n dwords copied from the pointer argument, written only if all fit.
    Block(usize),
    /// A dword (the plane), then four doubles stored as floats (the clip plane).
    Clip,
    /// Vertex space: count, byte size, then the space (padded to 4), whose address
    /// goes back into the caller's {dst, bytes}; the caller fills it.
    Vtx,
}

/// The ported handler types: how they record, and the API function they replay.
fn spec(kind: u32) -> Option<(Rec, usize)> {
    Some(match kind {
        0x00 => (Rec::Dwords(5), 0x00D1_49B0),
        0x01 => (Rec::Dwords(4), 0x00D1_4D20),
        0x02 => (Rec::None, 0x00D1_4950),
        0x03 => (Rec::Block(2), 0x00D1_48D0),
        0x04 => (Rec::Dwords(4), 0x00D1_4840),
        0x05 => (Rec::Dwords(1), 0x00D1_47E0),
        0x06 => (Rec::Dwords(1), 0x0056_C2B0),
        0x07 => (Rec::Block(3), 0x00D1_4700),
        0x08 => (Rec::Float, 0x00D2_32C0),
        0x09 => (Rec::None, 0x00D2_3200),
        0x0A => (Rec::Float, 0x00D2_3120),
        0x0B => (Rec::None, 0x00D2_3060),
        // 0x0C (clip plane, Rec::Clip) is written but stays original: never seen in play.
        0x0D => (Rec::Float, 0x00D2_3000),
        0x0E => (Rec::Float, 0x00D2_2F90),
        0x0F => (Rec::Block(3), 0x00D0_9E20),
        0x10 => (Rec::Dwords(4), 0x00D1_4B20),
        0x11 => (Rec::Dwords(1), 0x00D1_4AD0),
        0x18 => (Rec::Block(2), 0x00D0_A580),
        0x19 => (Rec::Vtx, 0x00D0_A4F0),
        0x1A => (Rec::None, 0x00D0_A4B0),
        0x1B => (Rec::Block(2), 0x00D0_A440),
        0x1C => (Rec::Vtx, 0x00D0_A380),
        0x1D => (Rec::None, 0x00D0_A330),
        0x1E => (Rec::Block(2), 0x00D0_A2C0),
        0x1F => (Rec::Vtx, 0x00D0_A1F0),
        0x20 => (Rec::None, 0x00D0_A190),
        0x27 => (Rec::None, 0x00D1_EF40),
        0x2A => (Rec::Dwords(2), 0),
        _ => return None,
    })
}

/// Replay a recorded call: each type's registers and stack, as its original handler.
fn replay(kind: u32, target: usize, v: &[u32]) {
    let mut local = [0u32; 3];
    local[..v.len().min(3)].copy_from_slice(&v[..v.len().min(3)]);
    let p = local.as_ptr() as u32;
    unsafe {
        match kind {
            0x00 => call_regs(target, 0, 0, 0, 0, v[1], &[v[0], v[2], v[3], v[4]]),
            0x01 => call_regs(target, 0, 0, 0, v[2], v[0], &[v[1], v[3]]),
            0x03 | 0x1B | 0x1E => call_regs(target, 0, p, 0, 0, 0, &[]),
            0x07 | 0x18 => call_regs(target, 0, 0, 0, p, 0, &[]),
            0x0F => call_regs(target, p, 0, 0, 0, 0, &[]),
            0x04 => call_regs(target, 0, 0, 0, 0, 0, &[v[0], v[1], v[2], v[3]]),
            0x05 | 0x08 | 0x0A | 0x0D | 0x0E => call_regs(target, 0, 0, 0, 0, 0, &[v[0]]),
            0x06 => call_regs(target, 0, 0, 0, v[0], 0, &[]),
            0x10 => call_regs(target, 0, v[3], v[2], 0, 0, &[v[0], v[1]]),
            0x11 => call_regs(target, 0, v[0], 0, 0, 0, &[]),
            0x2A => call_regs(v[0] as usize, 0, 0, 0, 0, 0, &[v[1]]),
            0x0C => call_regs(target, 0, v[0], 0, 0, 0, &[v[1], v[2], v[3], v[4]]),
            0x19 | 0x1C | 0x1F => call_regs(target, 0, 0, 0, v[0], 0, &[v[2]]),
            _ => call_regs(target, 0, 0, 0, 0, 0, &[]),
        };
    }
}

fn rec_size(r: Rec) -> u32 {
    match r {
        Rec::None => 0,
        Rec::Dwords(n) | Rec::Block(n) => n as u32 * 4,
        Rec::Float => 4,
        Rec::Clip => 20,
        Rec::Vtx => 0, // variable: handled in `handler`
    }
}

/// A ported handler: mode 0 records (false when it does not fit), any other mode
/// reads the record back (mode 1 also replays it). `ctx` is {data, used, space}.
fn handler(m: &mut Mem, kind: u32, mode: u32, ctx: usize, args: usize) -> bool {
    let (rec, target) = spec(kind).unwrap();
    let data = m.rd(ctx) as usize;
    if mode == 0 {
        let put = |m: &mut Mem, v: u32| -> bool {
            let used = m.rd(ctx + 4);
            if used.wrapping_add(4) > m.rd(ctx + 8) {
                return false;
            }
            m.wr(data + used as usize, v);
            m.wr(ctx + 4, used + 4);
            true
        };
        return match rec {
            Rec::None => true,
            Rec::Dwords(n) => {
                for i in 0..n {
                    let v = m.rd(args + i * 4);
                    if !put(m, v) {
                        return false;
                    }
                }
                true
            }
            Rec::Float => {
                let d = f64::from_bits(m.rd(args) as u64 | (m.rd(args + 4) as u64) << 32);
                let v = (d as f32).to_bits();
                put(m, v)
            }
            Rec::Clip => {
                let v = m.rd(args);
                if !put(m, v) {
                    return false;
                }
                for i in 0..4 {
                    let a = args + 4 + i * 8;
                    let d = f64::from_bits(m.rd(a) as u64 | (m.rd(a + 4) as u64) << 32);
                    let f = (d as f32).to_bits();
                    if !put(m, f) {
                        return false;
                    }
                }
                true
            }
            Rec::Vtx => {
                let count = m.rd(args);
                let out = m.rd(args + 4) as usize;
                if !put(m, count) {
                    m.wr(out, 0);
                    return false;
                }
                let bytes = m.rd(out + 4);
                let rem = (bytes as i32) % 4;
                let pad = if rem == 0 { 0 } else { (4 - rem) as u32 };
                let used = m.rd(ctx + 4);
                if used.wrapping_add(bytes).wrapping_add(pad).wrapping_add(4) > m.rd(ctx + 8) {
                    m.wr(out, 0);
                    return false;
                }
                m.wr(data + used as usize, bytes);
                let used = used + 4;
                m.wr(out, (data + used as usize) as u32);
                m.wr(ctx + 4, used.wrapping_add(bytes).wrapping_add(pad));
                true
            }
            Rec::Block(n) => {
                let p = m.rd(args) as usize;
                let v: Vec<u32> = (0..n).map(|i| m.rd(p + i * 4)).collect();
                let used = m.rd(ctx + 4);
                if used.wrapping_add(n as u32 * 4) > m.rd(ctx + 8) {
                    return false;
                }
                for (i, &x) in v.iter().enumerate() {
                    m.wr(data + used as usize + i * 4, x);
                }
                m.wr(ctx + 4, used + n as u32 * 4);
                true
            }
        };
    }
    let used = m.rd(ctx + 4) as usize;
    if rec == Rec::Vtx {
        let (count, bytes) = (m.rd(data + used), m.rd(data + used + 4));
        let rem = (bytes as i32) % 4;
        let pad = if rem == 0 { 0 } else { (4 - rem) as u32 };
        let ptr = (data + used + 8) as u32;
        m.wr(ctx + 4, (used as u32 + 8).wrapping_add(bytes).wrapping_add(pad));
        if mode == 1 && count as i32 <= 0x40 {
            let v = [count, bytes, ptr];
            if m.dry {
                m.tape.push(expected_call(kind, &v));
            } else {
                replay(kind, target, &v);
            }
        }
        return true;
    }
    let n = rec_size(rec) as usize / 4;
    let v: Vec<u32> = (0..n).map(|i| m.rd(data + used + i * 4)).collect();
    m.wr(ctx + 4, (used + n * 4) as u32);
    if mode == 1 {
        if m.dry {
            m.tape.push(expected_call(kind, &v));
        } else {
            replay(kind, target, &v);
        }
    }
    true
}

/// `render_deferred_queue` (0xCF8EA0; cdecl varargs (type, ...)): record an API call
/// in the game-side list; false (and the list marked overflowed) when it is full.
fn deferred_queue(m: &mut Mem, kind: u32, args: usize) -> bool {
    let list = m.rd(LIST_GAME) as usize;
    if m.rd(list + L_USED) as i32 > L_SPACE as i32 {
        return false;
    }
    if m.rd(list + L_COUNT) == L_MAX {
        m.wr(list + L_USED, L_SPACE + 1);
        return false;
    }
    let used = m.rd(list + L_USED);
    let mut ctx = [0u32; 3];
    let c = ctx.as_mut_ptr() as usize;
    m.wr(c, list as u32 + L_DATA as u32 + used);
    m.wr(c + 4, 0);
    m.wr(c + 8, L_SPACE.wrapping_sub(used));
    let ok = if spec(kind).is_some() {
        handler(m, kind, 0, c, args)
    } else if m.dry {
        // Not ported: the original handler cannot run in a prediction.
        for k in 0..3 {
            m.overlay.remove(&(c + k * 4));
        }
        return true;
    } else {
        unsafe {
            call_regs(rd(HANDLERS + kind as usize * 4) as usize, 0, 0, 0, 0, 0, &[0, c as u32, args as u32]) as u8 != 0
        }
    };
    if !ok {
        for k in 0..3 {
            m.overlay.remove(&(c + k * 4));
        }
        m.wr(list + L_USED, L_SPACE + 1);
        return false;
    }
    let added = m.rd(c + 4);
    // The context is scratch on our stack (the original's is on its own): not compared.
    for k in 0..3 {
        m.overlay.remove(&(c + k * 4));
    }
    let count = m.rd(list + L_COUNT) as usize;
    m.wr(list + count * 8, kind);
    m.wr(list + count * 8 + 4, m.rd(list + L_USED));
    let used = m.rd(list + L_USED);
    m.wr(list + L_USED, used.wrapping_add(added));
    m.wr(list + L_COUNT, count as u32 + 1);
    true
}

/// `render_deferred_execute` (0xCF8FA0): replay the render-side list, then forget
/// the bitmaps bound per stage.
fn deferred_execute(m: &mut Mem) {
    let mut i = 0usize;
    loop {
        let list = m.rd(LIST_RENDER) as usize;
        if i as i32 >= m.rd(list + L_COUNT) as i32 {
            break;
        }
        let (kind, off) = (m.rd(list + i * 8), m.rd(list + i * 8 + 4));
        let mut ctx = [0u32; 3];
        let c = ctx.as_mut_ptr() as usize;
        // Through the memory interface, so a prediction sees fresh values at a reused address.
        m.wr(c, list as u32 + L_DATA as u32 + off);
        m.wr(c + 4, 0);
        m.wr(c + 8, 0);
        if spec(kind).is_some() {
            handler(m, kind, 1, c, 0);
        } else if m.dry {
            m.tape.push(vec![3, kind]);
        } else {
            unsafe { call_regs(rd(HANDLERS + kind as usize * 4) as usize, 0, 0, 0, 0, 0, &[1, c as u32, 0]) };
        }
        i += 1;
    }
    for s in 0..16 {
        m.wr(BITMAP_CACHE + s * 4, u32::MAX);
    }
}

// Replay check. While the original executes, the API functions a handler calls
// directly are taped from their hooks (registers and stack, ECX's block for the
// block types); types whose API is not hooked tape only their type.
const T_ECX: usize = 3;
const T_EDX: usize = 4;
const T_ESI: usize = 5;
const T_EDI: usize = 6;
const T_ARG: usize = 7; // 6 stack arguments
const T_BLOCK: usize = 13; // 2 dwords at ECX

/// What the hook tape should show for a replayed call: [2, port, then (field, value)
/// pairs], or [3, type] when the API function is not hooked.
fn expected_call(kind: u32, v: &[u32]) -> Vec<u32> {
    let (port, fields): (usize, Vec<(usize, u32)>) = match kind {
        0x00 => {
            (VIEWPORT_RECT, vec![(T_EDI, v[1]), (T_ARG, v[0]), (T_ARG + 1, v[2]), (T_ARG + 2, v[3]), (T_ARG + 3, v[4])])
        }
        0x01 => (SCISSOR_RECT, vec![(T_EDI, v[0]), (T_ESI, v[2]), (T_ARG, v[1]), (T_ARG + 1, v[3])]),
        0x02 => (INVALIDATE_BLOCK, vec![]),
        0x03 => (SET_BLOCK, vec![(T_BLOCK, v[0]), (T_BLOCK + 1, v[1])]),
        0x04 => (SET_COLOR, (0..4).map(|i| (T_ARG + i, v[i])).collect()),
        0x05 => (SET_ALPHA, vec![(T_ARG, v[0])]),
        0x06 => (SET_BITMAP, vec![(T_ESI, v[0])]),
        0x08 => (PUSH_DEPTH_BIAS, vec![(T_ARG, v[0])]),
        0x09 => (POP_DEPTH_BIAS, vec![]),
        0x0A => (PUSH_SLOPE_BIAS, vec![(T_ARG, v[0])]),
        0x0B => (POP_SLOPE_BIAS, vec![]),
        0x10 => (CLEAR_REGION, vec![(T_ECX, v[3]), (T_EDX, v[2]), (T_ARG, v[0]), (T_ARG + 1, v[1])]),
        0x11 => (API_CLEAR, vec![(T_ECX, v[0])]),
        0x1B => (STRIP_BEGIN, vec![(T_BLOCK, v[0]), (T_BLOCK + 1, v[1])]),
        0x1D => (STRIP_END, vec![]),
        0x1E => (LIST_BEGIN, vec![(T_BLOCK, v[0]), (T_BLOCK + 1, v[1])]),
        0x20 => (LIST_END, vec![]),
        0x27 => (RESTORE_TARGETS, vec![]),
        0x0C => {
            (CLIP_PLANE, vec![(T_ECX, v[0]), (T_ARG, v[1]), (T_ARG + 1, v[2]), (T_ARG + 2, v[3]), (T_ARG + 3, v[4])])
        }
        0x1C => (STRIP_ADD, vec![(T_ESI, v[0]), (T_ARG, v[2])]),
        0x1F => (LIST_ADD, vec![(T_ESI, v[0]), (T_ARG, v[2])]),
        _ => return vec![3, kind],
    };
    let mut e = vec![2, port as u32];
    for (f, x) in fields {
        e.push(f as u32);
        e.push(x);
    }
    e
}

/// Does a taped hook call match an expected one?
fn call_matches(tape: &[u32], expected: &[u32]) -> bool {
    if tape[0] != expected[0] || tape[1] != expected[1] {
        return false;
    }
    expected[2..].chunks(2).all(|p| tape[p[0] as usize] == p[1])
}

thread_local! {
    static EXEC_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn exec_taping() -> bool {
    EXEC_TAPING.with(|t| t.get())
}

/// Called from `run` for every hooked API call: tape it if a handler made it.
fn tape_exec_call(i: usize, r: &Regs) -> bool {
    if !exec_taping() || EXEC_DEPTH.with(|d| d.get()) != 0 {
        return false;
    }
    let mut e = vec![2, i as u32, r.0, r.1, r.2, r.3, r.4];
    e.extend((0..6).map(|k| arg(r.5, k)));
    let block = matches!(i, SET_BLOCK | STRIP_BEGIN | LIST_BEGIN);
    e.extend((0..2).map(|k| if block { unsafe { rd(r.1 as usize + k * 4) } } else { 0 }));
    tape_push(e);
    EXEC_DEPTH.with(|d| d.set(1));
    true
}

fn end_exec_call(taped: bool) {
    if taped {
        EXEC_DEPTH.with(|d| d.set(0));
    }
}

/// Shadow-mode handler shims (all 43 types): while the original executes, a type
/// whose API is not hooked tapes its type and hides its inner calls.
fn shim(kind: u32, mode: u32, ctx: u32, args: u32) -> u32 {
    let orig = HANDLER_ORIG[kind as usize].load(Ordering::Acquire);
    let hooked = !matches!(expected_call(kind, &[0; 5])[0], 3);
    if mode == 1 && exec_taping() && EXEC_DEPTH.with(|d| d.get()) == 0 && !hooked {
        tape_push(vec![3, kind]);
        EXEC_DEPTH.with(|d| d.set(1));
        let r = unsafe { call_regs(orig, 0, 0, 0, 0, 0, &[mode, ctx, args]) };
        EXEC_DEPTH.with(|d| d.set(0));
        return r;
    }
    unsafe { call_regs(orig, 0, 0, 0, 0, 0, &[mode, ctx, args]) }
}

/// Rust-mode handler entries for the ported types.
fn live_handler(kind: u32, mode: u32, ctx: u32, args: u32) -> u32 {
    handler(&mut Mem::live(), kind, mode, ctx as usize, args as usize) as u32
}

static HANDLER_ORIG: [AtomicUsize; 43] = [const { AtomicUsize::new(0) }; 43];

macro_rules! handler_entries {
    ($($name:ident = $k:expr),* $(,)?) => {
        $(
            extern "C" fn $name(mode: u32, ctx: u32, args: u32) -> u32 {
                if MODE == Mode::Shadow { shim($k, mode, ctx, args) } else { live_handler($k, mode, ctx, args) }
            }
        )*
        const HANDLER_ENTRIES: [extern "C" fn(u32, u32, u32) -> u32; 43] = [$($name),*];
    };
}
handler_entries!(
    hd00 = 0x00,
    hd01 = 0x01,
    hd02 = 0x02,
    hd03 = 0x03,
    hd04 = 0x04,
    hd05 = 0x05,
    hd06 = 0x06,
    hd07 = 0x07,
    hd08 = 0x08,
    hd09 = 0x09,
    hd0a = 0x0A,
    hd0b = 0x0B,
    hd0c = 0x0C,
    hd0d = 0x0D,
    hd0e = 0x0E,
    hd0f = 0x0F,
    hd10 = 0x10,
    hd11 = 0x11,
    hd12 = 0x12,
    hd13 = 0x13,
    hd14 = 0x14,
    hd15 = 0x15,
    hd16 = 0x16,
    hd17 = 0x17,
    hd18 = 0x18,
    hd19 = 0x19,
    hd1a = 0x1A,
    hd1b = 0x1B,
    hd1c = 0x1C,
    hd1d = 0x1D,
    hd1e = 0x1E,
    hd1f = 0x1F,
    hd20 = 0x20,
    hd21 = 0x21,
    hd22 = 0x22,
    hd23 = 0x23,
    hd24 = 0x24,
    hd25 = 0x25,
    hd26 = 0x26,
    hd27 = 0x27,
    hd28 = 0x28,
    hd29 = 0x29,
    hd2a = 0x2A,
);

/// Point the handler table at our entries: all of them for the shadow check, the
/// ported ones in the Rust build.
fn install_handlers() {
    unsafe {
        let mut old = 0u32;
        if VirtualProtect(HANDLERS as *const _, 43 * 4, PAGE_READWRITE, &mut old) == 0 {
            log("gfx: could not write the deferred handler table");
            return;
        }
        for k in 0..43u32 {
            let p = HANDLERS + k as usize * 4;
            HANDLER_ORIG[k as usize].store(rd(p) as usize, Ordering::Release);
            if MODE == Mode::Shadow || spec(k).is_some() {
                wr(p, HANDLER_ENTRIES[k as usize] as usize as u32);
            }
        }
        VirtualProtect(HANDLERS as *const _, 43 * 4, old, &mut old);
    }
}

#[unsafe(naked)]
extern "C" fn queue_entry() {
    std::arch::naked_asm!(
        "lea eax, [esp + 8]",
        "push eax",
        "push dword ptr [esp + 8]",
        "call {h}",
        "add esp, 8",
        "ret",
        h = sym queue_hook,
    )
}

/// `render_deferred_queue`: tape it while an original runs (type and ten argument
/// dwords), then original / shadow / Rust.
extern "C" fn queue_hook(kind: u32, args: u32) -> u32 {
    if taping() {
        let mut e = vec![1, kind];
        e.extend((0..10).map(|k| unsafe { rd(args as usize + k * 4) }));
        tape_push(e);
    }
    CALLS[QUEUE].fetch_add(1, Ordering::Relaxed);
    // Varargs: pass on the caller's argument block (32 dwords covers every type).
    let original = move || unsafe {
        let mut a = vec![kind];
        a.extend((0..32).map(|k| rd(args as usize + k * 4)));
        call_regs(ORIG[QUEUE].load(Ordering::Acquire), 0, 0, 0, 0, 0, &a) & 0xFF
    };
    match MODE {
        Mode::Original => original(),
        Mode::Shadow => {
            if IN_SHADOW.with(|f| f.replace(true)) {
                return original();
            }
            let r = shadow(QUEUE, |m| deferred_queue(m, kind, args as usize) as u32, original);
            IN_SHADOW.with(|f| f.set(false));
            r
        }
        Mode::Rust => deferred_queue(&mut Mem::live(), kind, args as usize) as u32,
    }
}

/// `render_deferred_execute`: in the shadow check, the Rust decodes the list into the
/// calls the handlers should make, then the original executes with those calls taped.
extern "C" fn execute_hook() {
    CALLS[EXECUTE_LIST].fetch_add(1, Ordering::Relaxed);
    let original =
        || unsafe { std::mem::transmute::<usize, extern "C" fn()>(ORIG[EXECUTE_LIST].load(Ordering::Acquire))() };
    match MODE {
        Mode::Original => original(),
        Mode::Shadow => {
            let mut m = Mem { dry: true, ..Mem::live() };
            deferred_execute(&mut m);
            tape_take();
            EXEC_TAPING.with(|t| t.set(true));
            original();
            EXEC_TAPING.with(|t| t.set(false));
            let theirs = tape_take();
            let theirs: Vec<Vec<u32>> = theirs.into_iter().filter(|e| e[0] >= 2).collect();
            let ok = theirs.len() == m.tape.len()
                && theirs.iter().zip(&m.tape).all(|(t, x)| if x[0] == 3 { t == x } else { call_matches(t, x) });
            REPLAYS.fetch_add(theirs.len() as u32, Ordering::Relaxed);
            if !ok {
                MISMATCHES[EXECUTE_LIST].fetch_add(1, Ordering::Relaxed);
                let at =
                    theirs.iter().zip(&m.tape).position(|(t, x)| if x[0] == 3 { t != x } else { !call_matches(t, x) });
                if LOGGED.fetch_add(1, Ordering::Relaxed) < 40 {
                    log(&format!(
                        "gfx: render_deferred_execute mismatch: {} calls taped, {} expected; first difference at {at:?}: {:x?} vs {:x?}",
                        theirs.len(),
                        m.tape.len(),
                        at.and_then(|i| theirs.get(i)),
                        at.and_then(|i| m.tape.get(i))
                    ));
                }
            }
        }
        Mode::Rust => deferred_execute(&mut Mem::live()),
    }
}

static REPLAYS: AtomicU32 = AtomicU32::new(0);

// --- 2D vertex space, tint color, clip planes -------------------------------------------------

const SHADER_PAIRS: usize = 0x00E8_4AF0; // 0x1C-byte entries {+0 VS, +4 PS, +0xC declaration}
const PS_C4_COPY: usize = 0x0230_4D9C; // the last tint constant, kept for a device reset
const CLIP_PLANE_MASK: usize = 0x0252_A314;
const CLIP_PLANE_CACHE: usize = 0x022F_5A50; // RENDER_STATE_CACHE[0x98]

/// `gfx_bind_shader_pair` (0xD19070; EAX = pair index): its vertex and pixel shader
/// (NULL = the defaults) through the shader caches.
fn bind_shader_pair(m: &mut Mem, idx: u32) {
    let e = SHADER_PAIRS + idx as usize * 0x1C;
    let vs = match m.rd(e) {
        0 => m.rd(DEFAULT_VS),
        v => v,
    };
    if m.rd(VS_CACHE) != vs {
        m.put(&[4, vs]);
        m.wr(VS_CACHE, vs);
    }
    let ps = match m.rd(e + 4) {
        0 => m.rd(DEFAULT_PS),
        v => v,
    };
    if m.rd(PS_CACHE) != ps {
        m.put(&[3, ps]);
        m.wr(PS_CACHE, ps);
    }
}

/// `gfx_2d_alloc` (0xD1E070; cdecl (vertex count) -> the 2D vertex buffer, or NULL
/// while a batch is pending): linear filtering on sampler 0, and the 2D shader pair
/// (5 without a bitmap on stage 0, else 4) with its declaration.
fn alloc_2d(m: &mut Mem, count: u32) -> u32 {
    if m.rd(VERTS_PENDING) != 0 {
        return 0;
    }
    m.wr(VERTS_PENDING, 1);
    m.wr(VERTS_COUNT, count);
    sampler0_state(m, 7, 2);
    sampler0_state(m, 5, 2);
    let idx = 4 + ((m.rd(BITMAP_CACHE) as i32) < 0) as u32;
    let decl = m.rd(SHADER_PAIRS + idx as usize * 0x1C + 0xC);
    if decl != m.rd(DECL_CACHE) {
        m.put(&[5, decl]);
        m.wr(DECL_CACHE, decl);
    }
    bind_shader_pair(m, idx);
    VERTS as u32
}

/// `im2d_strip_add` / `im2d_list_add` (0xD0A380 / 0xD0A1F0; ESI = vertex count, cdecl
/// (vertices or NULL) -> where the vertices go): flush what the batch holds, take the
/// vertex space and copy the vertices in. Deferred, the space is in the list record
/// (type 0x1C / 0x1F) and the caller fills it.
fn batch_add(m: &mut Mem, list: bool, count: u32, src: u32) -> u32 {
    if defers() {
        let mut out = [0u32, count.wrapping_mul(0x1C)];
        let p = out.as_mut_ptr() as usize;
        m.wr(p, 0);
        m.wr(p + 4, count.wrapping_mul(0x1C));
        m.queue(if list { 0x1F } else { 0x1C }, &[count, p as u32]);
        if m.dry
            && let Some(e) = m.tape.last_mut()
        {
            e.truncate(3); // the {dst, bytes} pointer is each caller's own stack: not compared
        }
        let dst = m.rd(p);
        // Our own scratch: the original's is on its own stack.
        m.overlay.remove(&p);
        m.overlay.remove(&(p + 4));
        return dst;
    }
    let var = if list { LIST_COUNT } else { STRIP_COUNT };
    let n = m.rd(var) as i32;
    if list {
        if n >= 3 {
            flush_2d(m, 4, (n / 3) as u32);
        }
    } else if n >= 0 {
        flush_2d(m, 5, n.wrapping_sub(2) as u32);
    }
    m.wr(var, count);
    let dst = alloc_2d(m, count);
    if dst != 0 && src != 0 {
        let n = count.wrapping_mul(0x1C) as usize;
        if m.dry {
            for i in (0..n).step_by(4) {
                let v = m.rd(src as usize + i);
                m.wr(dst as usize + i, v);
            }
        } else {
            unsafe { std::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, n) };
        }
    }
    dst
}

/// `gfx_cmd_set_ps_c4` (0xD18DE0; EAX = &float4): pixel shader constant c4, also
/// kept at 0x2304D9C for a device reset. No cache.
fn set_ps_c4(m: &mut Mem, p: u32) {
    let v: Vec<u32> = (0..4).map(|i| m.rd(p as usize + i * 4)).collect();
    for (i, &x) in v.iter().enumerate() {
        m.wr(PS_C4_COPY + i * 4, x);
    }
    m.put(&[7, 4, 1, v[0], v[1], v[2], v[3]]);
}

/// `gfx_set_tint_color` (0xD14700; ESI = &{r, g, b}): the context's +0x70 = (r, g, b,
/// 1.0), then pixel constant c4 from it.
fn set_tint_color(m: &mut Mem, p: u32) {
    let ctx = thread_context() as usize;
    for i in 0..3 {
        let v = m.rd(p as usize + i * 4);
        m.wr(ctx + 0x70 + i * 4, v);
    }
    m.wr(ctx + 0x7C, 1.0f32.to_bits());
    if defers() {
        m.queue(7, &[p]);
    } else {
        set_ps_c4(m, ctx as u32 + 0x70);
    }
}

/// `gfx_set_user_clip_plane` (0xD18BD0; ECX = plane 0-5, cdecl (a, b, c, d)): a plane
/// with a = b = c = 0 is switched off, otherwise on and set on the device directly
/// (SetClipPlane, not through the ring); CLIPPLANEENABLE through the cache.
fn set_user_clip_plane(m: &mut Mem, idx: u32, plane: [u32; 4]) {
    if idx > 5 {
        return;
    }
    let off = (0..3).all(|i| f32::from_bits(plane[i]) == 0.0);
    let mask = if off { m.rd(CLIP_PLANE_MASK) & !(1 << idx) } else { m.rd(CLIP_PLANE_MASK) | 1 << idx };
    m.wr(CLIP_PLANE_MASK, mask);
    if m.rd(CLIP_PLANE_CACHE) != mask && mask != 0xBAAD_F00D {
        m.wr(CLIP_PLANE_CACHE, mask);
        m.put(&[15, 0x98, mask]);
    }
    if off || m.dry {
        return; // (the device call cannot be predicted: not compared)
    }
    unsafe {
        let dev = rd(0x0252_A2D0);
        let f: extern "system" fn(u32, u32, *const [u32; 4]) -> i32 =
            std::mem::transmute(rd(rd(dev as usize) as usize + 0xDC) as usize);
        f(dev, idx, &plane);
    }
}

/// `gfx_set_clip_plane` (0xD22090; ECX = plane, cdecl (a, b, c, d)).
fn set_clip_plane(m: &mut Mem, idx: u32, plane: [u32; 4]) {
    if defers() {
        let mut a = vec![idx];
        for v in plane {
            a.extend_from_slice(&f64_args(f32::from_bits(v)));
        }
        m.queue(0xC, &a);
    } else {
        set_user_clip_plane(m, idx, plane);
    }
}

/// What a hook's Rust body returns to the caller in EAX.
trait Ret {
    fn ret(self) -> u32;
}
impl Ret for () {
    fn ret(self) -> u32 {
        0
    }
}
impl Ret for u32 {
    fn ret(self) -> u32 {
        self
    }
}

// --- Calling the originals ------------------------------------------------------------------

/// Call a game function with ECX/EDX/ESI/EDI set and cdecl stack arguments.
pub(crate) unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    let block: [u32; 8] = [f as u32, ecx, edx, esi, edi, args.as_ptr() as u32, args.len() as u32, eax];
    let r: u32;
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
            "mov esp, ebp",
            "pop ebp", "pop edi", "pop esi", "pop ebx",
            inout("eax") block.as_ptr() => r, lateout("ecx") _, lateout("edx") _,
            clobber_abi("C"),
        );
    }
    r
}

// --- Shadow check -----------------------------------------------------------------------

// Tapes are per thread: the game and render threads both run checks.
thread_local! {
    static TAPE: std::cell::RefCell<Vec<Vec<u32>>> = const { std::cell::RefCell::new(Vec::new()) };
    static TAPING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EXEC_TAPING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn tape_push(e: Vec<u32>) {
    TAPE.with(|t| t.borrow_mut().push(e));
}

fn tape_take() -> Vec<Vec<u32>> {
    TAPE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}
static REAL_INTERLOCKED_ADD: AtomicUsize = AtomicUsize::new(0);

fn taping() -> bool {
    TAPING.with(|t| t.get())
}

extern "system" fn interlocked_add_w(p: *mut i32, v: i32) -> i32 {
    if p as usize == RING_PUBLISHED && taping() {
        tape_push(vec![0, v as u32]);
    }
    let f: extern "system" fn(*mut i32, i32) -> i32 =
        unsafe { std::mem::transmute(REAL_INTERLOCKED_ADD.load(Ordering::Acquire)) };
    f(p, v)
}

/// Caches the code may touch, watched for writes the prediction did not make.
fn watched() -> Vec<usize> {
    let mut v = vec![
        RING_WRITE,
        RING_LAST_PUBLISHED,
        VS_CACHE,
        PS_CACHE,
        DECL_CACHE,
        IB_CACHE,
        DEPTH_CACHE,
        TARGET_CACHE,
        STRIP_COUNT,
        LIST_COUNT,
        VERTS_PENDING,
        VERTS_COUNT,
    ];
    v.extend((0..16).map(|i| TEXTURE_CACHE + i * 4));
    v.extend((0..16).map(|i| BITMAP_CACHE + i * 4));
    v.extend((0..4).flat_map(|i| [STREAM_VB + i * 4, STREAM_OFFSET + i * 4, STREAM_STRIDE + i * 4]));
    v.extend((0..20 * 16).map(|i| SAMPLER_CACHE + i * 4));
    v.extend((0..0x100).map(|i| RENDER_STATE_CACHE + i * 4));
    v
}

thread_local! {
    static IN_SHADOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static CALLS: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static MISMATCHES: [AtomicU32; COUNT] = [const { AtomicU32::new(0) }; COUNT];
static LOGGED: AtomicU32 = AtomicU32::new(0);

fn shadow(i: usize, rust: impl FnOnce(&mut Mem) -> u32, original: impl FnOnce() -> u32) -> u32 {
    let watch = watched();
    let before: Vec<u32> = watch.iter().map(|&p| unsafe { rd(p) }).collect();
    let mut m = Mem { dry: true, ..Mem::live() };
    rust(&mut m);
    tape_take();
    TAPING.with(|t| t.set(true));
    let r = original();
    TAPING.with(|t| t.set(false));
    let theirs = tape_take();
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
    let same = theirs.len() == m.tape.len()
        && theirs
            .iter()
            .zip(&m.tape)
            .all(|(t, r)| if t[0] == 0 { t == r } else { t.len() >= r.len() && t[..r.len()] == r[..] });
    if !same {
        diffs.push(format!("publishes / queued calls: original {theirs:x?}, Rust {:x?}", m.tape));
    }
    if !diffs.is_empty() {
        MISMATCHES[i].fetch_add(1, Ordering::Relaxed);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 40 {
            log(&format!("gfx: {} mismatch: {}", PORTS[i].0, diffs.join("; ")));
        }
    }
    r
}

/// One hook body: count, then original / shadow / Rust. A hook reached while a check
/// runs on this thread (one ported function calling another) runs the original: the
/// outer check predicted it already.
fn run(i: usize, rust: impl FnOnce(&mut Mem) -> u32, original: impl FnOnce() -> u32) -> u32 {
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    if exec_taping() {
        return original(); // a handler's call during the replay check: taped by the hook
    }
    match MODE {
        Mode::Original => original(),
        Mode::Shadow => {
            if IN_SHADOW.with(|f| f.replace(true)) {
                return original();
            }
            let r = shadow(i, rust, original);
            IN_SHADOW.with(|f| f.set(false));
            r
        }
        Mode::Rust => rust(&mut Mem::live()),
    }
}

// --- Hooks ------------------------------------------------------------------------------

/// Every hook gets (eax, ecx, edx, esi, edi, &first stack argument); ECX, EDX, ESI and
/// EDI are preserved for the caller (the originals leave some of them alone and LTCG
/// callers may rely on that). All the hooked functions are caller-pop.
macro_rules! entry {
    ($name:ident, $hook:ident) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push ecx", "push edx", "push esi", "push edi", "push eax",
                "lea ecx, [esp + 24]",
                "push ecx", "push edi", "push esi", "push edx",
                "push dword ptr [esp + 32]",
                "push dword ptr [esp + 20]",
                "call {h}",
                "add esp, 28",
                "pop edi", "pop esi", "pop edx", "pop ecx",
                "ret",
                h = sym $hook,
            )
        }
    };
}

type Regs = (u32, u32, u32, u32, u32, *const u32);

fn arg(a: *const u32, i: usize) -> u32 {
    unsafe { *a.add(i) }
}

fn original(i: usize, r: Regs, n: usize) -> impl FnOnce() -> u32 {
    move || unsafe {
        let args: Vec<u32> = (0..n).map(|k| arg(r.5, k)).collect();
        call_regs(ORIG[i].load(Ordering::Acquire), r.0, r.1, r.2, r.3, r.4, &args)
    }
}

macro_rules! hook {
    ($entry:ident, $hook:ident, $idx:expr, $nargs:expr, |$m:ident, $eax:ident, $ecx:ident, $edx:ident, $esi:ident, $edi:ident, $a:ident| $body:expr) => {
        extern "C" fn $hook($eax: u32, $ecx: u32, $edx: u32, $esi: u32, $edi: u32, $a: *const u32) -> u32 {
            let r = ($eax, $ecx, $edx, $esi, $edi, $a);
            let t = tape_exec_call($idx, &r);
            let res = run($idx, |$m: &mut Mem| Ret::ret($body), original($idx, r, $nargs));
            end_exec_call(t);
            res
        }
        entry!($entry, $hook);
    };
}

hook!(e_stream, h_stream, STREAM, 0, |m, _eax, ecx, edx, esi, edi, _a| set_stream_source(m, ecx, edi, edx, esi));
hook!(e_indices, h_indices, INDICES, 0, |m, _eax, ecx, _edx, _esi, _edi, _a| set_indices(m, ecx));
hook!(e_decl, h_decl, DECL, 0, |m, _eax, ecx, _edx, _esi, _edi, _a| set_vertex_declaration(m, ecx));
hook!(e_ps, h_ps, PIXEL_SHADER, 0, |m, _eax, ecx, _edx, _esi, _edi, _a| set_shader(m, false, ecx));
hook!(e_vs, h_vs, VERTEX_SHADER, 0, |m, _eax, ecx, _edx, _esi, _edi, _a| set_shader(m, true, ecx));
hook!(e_cmd9, h_cmd9, CMD_9, 1, |m, _eax, _ecx, edx, _esi, _edi, a| cmd_9(m, arg(a, 0), edx));
hook!(e_psc_u, h_psc_u, PS_CONSTANTS_UNCHECKED, 1, |m, _eax, ecx, edx, _esi, _edi, a| set_constants(
    m,
    7,
    false,
    ecx,
    edx,
    arg(a, 0)
));
hook!(e_psc, h_psc, PS_CONSTANTS, 1, |m, _eax, ecx, edx, _esi, _edi, a| set_constants(m, 7, true, ecx, edx, arg(a, 0)));
hook!(e_vsc, h_vsc, VS_CONSTANTS, 1, |m, _eax, ecx, edx, _esi, _edi, a| set_constants(
    m,
    6,
    false,
    ecx,
    edx,
    arg(a, 0)
));
hook!(e_texture, h_texture, TEXTURE, 0, |m, _eax, ecx, edx, _esi, _edi, _a| set_texture(m, edx, ecx));
hook!(e_draw_indexed, h_draw_indexed, DRAW_INDEXED, 3, |m, _eax, _ecx, _edx, _esi, _edi, a| draw_indexed(
    m,
    arg(a, 0),
    arg(a, 1),
    arg(a, 2)
));
hook!(e_draw, h_draw, DRAW, 0, |m, _eax, ecx, edx, _esi, _edi, _a| draw(m, ecx, edx));
hook!(e_draw_up, h_draw_up, DRAW_UP, 1, |m, _eax, ecx, edx, _esi, edi, a| draw_up(m, edi, ecx, edx, arg(a, 0)));
hook!(e_sampler, h_sampler, SAMPLER, 0, |m, _eax, ecx, edx, esi, _edi, _a| set_sampler_state(m, esi, edx, ecx));
hook!(e_reset_tex, h_reset_tex, RESET_TEXTURES, 0, |m, _eax, _ecx, _edx, _esi, _edi, _a| reset_texture_cache(m));
hook!(e_clear, h_clear, CLEAR, 4, |m, _eax, _ecx, _edx, _esi, _edi, a| clear(
    m,
    arg(a, 0),
    arg(a, 1),
    arg(a, 2),
    arg(a, 3)
));
hook!(e_depth, h_depth, DEPTH_STENCIL, 1, |m, _eax, _ecx, _edx, _esi, _edi, a| set_depth_stencil(m, arg(a, 0)));
hook!(e_target, h_target, RENDER_TARGET, 1, |m, _eax, _ecx, edx, _esi, _edi, a| set_render_target(m, edx, arg(a, 0)));
hook!(e_stretch, h_stretch, STRETCH, 2, |m, _eax, ecx, edx, _esi, _edi, a| stretch_rect(
    m,
    edx,
    ecx,
    arg(a, 0),
    arg(a, 1)
));
hook!(e_viewport, h_viewport, VIEWPORT, 2, |m, _eax, _ecx, _edx, _esi, _edi, a| set_viewport(m, arg(a, 0), arg(a, 1)));
hook!(e_scissor, h_scissor, SCISSOR, 4, |m, _eax, _ecx, _edx, _esi, _edi, a| set_scissor(
    m,
    arg(a, 0) as i32,
    arg(a, 1) as i32,
    arg(a, 2) as i32,
    arg(a, 3) as i32
));

hook!(e_tex_cached, h_tex_cached, SET_TEXTURE_CACHED, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_texture_cached(m, esi, edi)
});
hook!(e_apply_block, h_apply_block, APPLY_BLOCK, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    {
        let (x, y) = (m.rd(ecx as usize), m.rd(ecx as usize + 4));
        apply_render_state_block(m, x, y)
    }
});
hook!(e_set_block, h_set_block, SET_BLOCK, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_render_state_block(m, ecx)
});
hook!(e_inv_block, h_inv_block, INVALIDATE_BLOCK, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    invalidate_render_state_block(m)
});
hook!(e_vp_rect, h_vp_rect, VIEWPORT_RECT, 4, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_viewport_rect(m, edi as i32, arg(a, 0) as i32, arg(a, 1) as i32, arg(a, 2) as i32, arg(a, 3))
});
hook!(e_reset_vp, h_reset_vp, RESET_VIEWPORT, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    reset_viewport(m)
});
hook!(e_sc_rect, h_sc_rect, SCISSOR_RECT, 2, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_scissor_rect(m, edi as i32, esi as i32, arg(a, 0) as i32, arg(a, 1) as i32)
});
hook!(e_color, h_color, SET_COLOR, 4, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_color(m, [arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3)])
});
hook!(e_alpha, h_alpha, SET_ALPHA, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_alpha(m, arg(a, 0))
});
hook!(e_bitmap, h_bitmap, SET_BITMAP, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_bitmap(m, esi)
});
hook!(e_push_db, h_push_db, PUSH_DEPTH_BIAS, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    bias(m, false, Some(f32::from_bits(arg(a, 0))))
});
hook!(e_pop_db, h_pop_db, POP_DEPTH_BIAS, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    bias(m, false, None)
});
hook!(e_push_sb, h_push_sb, PUSH_SLOPE_BIAS, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    bias(m, true, Some(f32::from_bits(arg(a, 0))))
});
hook!(e_pop_sb, h_pop_sb, POP_SLOPE_BIAS, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    bias(m, true, None)
});
hook!(e_api_clear, h_api_clear, API_CLEAR, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    api_clear(m, ecx)
});
hook!(e_clear_region, h_clear_region, CLEAR_REGION, 2, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    clear_region(m, ecx, edx, arg(a, 0), arg(a, 1))
});
hook!(e_restore, h_restore, RESTORE_TARGETS, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    restore_default_targets(m)
});
hook!(e_flush, h_flush, FLUSH_2D, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    flush_2d(m, eax, arg(a, 0))
});
hook!(e_strip_begin, h_strip_begin, STRIP_BEGIN, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_begin(m, false, ecx)
});
hook!(e_strip_end, h_strip_end, STRIP_END, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_end(m, false)
});
hook!(e_list_begin, h_list_begin, LIST_BEGIN, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_begin(m, true, ecx)
});
hook!(e_list_end, h_list_end, LIST_END, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_end(m, true)
});

hook!(e_bind_pair, h_bind_pair, BIND_SHADER_PAIR, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    bind_shader_pair(m, eax)
});
hook!(e_alloc_2d, h_alloc_2d, ALLOC_2D, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    alloc_2d(m, arg(a, 0))
});
hook!(e_strip_add, h_strip_add, STRIP_ADD, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_add(m, false, esi, arg(a, 0))
});
hook!(e_list_add, h_list_add, LIST_ADD, 1, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    batch_add(m, true, esi, arg(a, 0))
});
hook!(e_ps_c4, h_ps_c4, PS_C4, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_ps_c4(m, eax)
});
hook!(e_tint, h_tint, TINT, 0, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_tint_color(m, esi)
});
hook!(e_user_clip, h_user_clip, USER_CLIP_PLANE, 4, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_user_clip_plane(m, ecx, [arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3)])
});
hook!(e_clip, h_clip, CLIP_PLANE, 4, |m, eax, ecx, edx, esi, edi, a| {
    let _ = (eax, ecx, edx, esi, edi, a);
    set_clip_plane(m, ecx, [arg(a, 0), arg(a, 1), arg(a, 2), arg(a, 3)])
});

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
const SET_TEXTURE_CACHED: usize = 21;
const APPLY_BLOCK: usize = 22;
const SET_BLOCK: usize = 23;
const INVALIDATE_BLOCK: usize = 24;
const VIEWPORT_RECT: usize = 25;
const RESET_VIEWPORT: usize = 26;
const SCISSOR_RECT: usize = 27;
const SET_COLOR: usize = 28;
const SET_ALPHA: usize = 29;
const SET_BITMAP: usize = 30;
const PUSH_DEPTH_BIAS: usize = 31;
const POP_DEPTH_BIAS: usize = 32;
const PUSH_SLOPE_BIAS: usize = 33;
const POP_SLOPE_BIAS: usize = 34;
const API_CLEAR: usize = 35;
const CLEAR_REGION: usize = 36;
const RESTORE_TARGETS: usize = 37;
const FLUSH_2D: usize = 38;
const STRIP_BEGIN: usize = 39;
const STRIP_END: usize = 40;
const LIST_BEGIN: usize = 41;
const LIST_END: usize = 42;
const QUEUE: usize = 43;
const EXECUTE_LIST: usize = 44;
const BIND_SHADER_PAIR: usize = 45;
const ALLOC_2D: usize = 46;
const STRIP_ADD: usize = 47;
const LIST_ADD: usize = 48;
const PS_C4: usize = 49;
const TINT: usize = 50;
const USER_CLIP_PLANE: usize = 51;
const CLIP_PLANE: usize = 52;
const COUNT: usize = 53;

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
    ("gfx_set_texture_cached", 0x004D0D70, &[0x39, 0x3C, 0xB5, 0xA8, 0x4A, 0xE8, 0x00]),
    ("gfx_apply_render_state_block", 0x00D1CDB0, &[0x83, 0xEC, 0x08, 0x8B, 0x01]),
    ("gfx_set_render_state_block", 0x00D148D0, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_invalidate_render_state_block", 0x00D14950, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_set_viewport_rect", 0x00D149B0, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_reset_viewport", 0x00D14D90, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_set_scissor_rect", 0x00D14D20, &[0x51, 0x3B, 0xF7, 0x53, 0x8B, 0x5C, 0x24, 0x10]),
    ("gfx_set_color", 0x00D14840, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_set_alpha", 0x00D147E0, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_set_bitmap", 0x0056C2B0, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_push_depth_bias", 0x00D232C0, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xC0]),
    ("gfx_pop_depth_bias", 0x00D23200, &[0x51, 0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("gfx_push_slope_depth_bias", 0x00D23120, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xC0]),
    ("gfx_pop_slope_depth_bias", 0x00D23060, &[0x51, 0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("gfx_clear", 0x00D14AD0, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("gfx_clear_region", 0x00D14B20, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("gfx_restore_default_targets", 0x00D1EF40, &[0x51, 0xA1, 0xB4, 0xDC, 0x2F, 0x02]),
    ("gfx_2d_flush", 0x00D1DF50, &[0x56, 0x33, 0xF6, 0x39, 0x35, 0xFC, 0xA2, 0x52, 0x02]),
    ("im2d_strip_begin", 0x00D0A440, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("im2d_strip_end", 0x00D0A330, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("im2d_list_begin", 0x00D0A2C0, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("im2d_list_end", 0x00D0A190, &[0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("render_deferred_queue", 0x00CF_8EA0, &[0xA1, 0x7C, 0x59, 0x30, 0x02]),
    ("render_deferred_execute", 0x00CF_8FA0, &[0xA1, 0x78, 0x59, 0x30, 0x02]),
    ("gfx_bind_shader_pair", 0x00D19070, &[0x8D, 0x0C, 0xC5, 0x00, 0x00, 0x00, 0x00]),
    ("gfx_2d_alloc", 0x00D1E070, &[0x33, 0xD2, 0x39, 0x15, 0xFC, 0xA2, 0x52, 0x02]),
    ("im2d_strip_add", 0x00D0A380, &[0x83, 0xEC, 0x08, 0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("im2d_list_add", 0x00D0A1F0, &[0x83, 0xEC, 0x08, 0x80, 0x3D, 0xDB, 0x49, 0xE8, 0x00, 0x00]),
    ("gfx_cmd_set_ps_c4", 0x00D18DE0, &[0xD9, 0x00, 0xD9, 0x1D, 0x9C, 0x4D, 0x30, 0x02]),
    ("gfx_set_tint_color", 0x00D14700, &[0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00]),
    ("gfx_set_user_clip_plane", 0x00D18BD0, &[0x83, 0xEC, 0x10, 0x83, 0xF9, 0x05]),
    ("gfx_set_clip_plane", 0x00D22090, &[0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xC0]),
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

/// Never called in the shadow-checked play run (the game writes these commands
/// inline elsewhere), so not verified: left original in the Rust build.
const NOT_SEEN_IN_GAME: [usize; 7] =
    [DECL, PS_CONSTANTS_UNCHECKED, PS_CONSTANTS, VS_CONSTANTS, DRAW, USER_CLIP_PLANE, CLIP_PLANE];

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
        SET_TEXTURE_CACHED => e_tex_cached,
        APPLY_BLOCK => e_apply_block,
        SET_BLOCK => e_set_block,
        INVALIDATE_BLOCK => e_inv_block,
        VIEWPORT_RECT => e_vp_rect,
        RESET_VIEWPORT => e_reset_vp,
        SCISSOR_RECT => e_sc_rect,
        SET_COLOR => e_color,
        SET_ALPHA => e_alpha,
        SET_BITMAP => e_bitmap,
        PUSH_DEPTH_BIAS => e_push_db,
        POP_DEPTH_BIAS => e_pop_db,
        PUSH_SLOPE_BIAS => e_push_sb,
        POP_SLOPE_BIAS => e_pop_sb,
        API_CLEAR => e_api_clear,
        CLEAR_REGION => e_clear_region,
        RESTORE_TARGETS => e_restore,
        FLUSH_2D => e_flush,
        STRIP_BEGIN => e_strip_begin,
        STRIP_END => e_strip_end,
        LIST_BEGIN => e_list_begin,
        LIST_END => e_list_end,
        QUEUE => queue_entry,
        BIND_SHADER_PAIR => e_bind_pair,
        ALLOC_2D => e_alloc_2d,
        STRIP_ADD => e_strip_add,
        LIST_ADD => e_list_add,
        PS_C4 => e_ps_c4,
        TINT => e_tint,
        USER_CLIP_PLANE => e_user_clip,
        CLIP_PLANE => e_clip,
        EXECUTE_LIST => execute_hook,
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
        if MODE == Mode::Rust && NOT_SEEN_IN_GAME.contains(&i) {
            continue;
        }
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
    install_handlers();
    log(&format!(
        "replaced {ok}/{COUNT} graphics functions (ring writers, API, deferred lists) and the ported deferred handlers"
    ));
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
    format!("gfx live: {} | deferred replays compared {}", parts.join(", "), REPLAYS.load(Ordering::Relaxed))
}
