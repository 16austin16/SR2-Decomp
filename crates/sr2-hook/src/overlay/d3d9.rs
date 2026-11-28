//! A small Direct3D 9 backend for Dear ImGui, written against the raw COM vtables (no extra crates).
//!
//! Everything is drawn with `DrawIndexedPrimitiveUP` (no default-pool vertex or index buffers), so
//! there is nothing to release before a device `Reset`; the font texture lives in the managed pool,
//! which also survives `Reset`. The game's own device state is saved with an `IDirect3DStateBlock9`
//! (D3DSBT_ALL, created per frame) before drawing and re-applied afterwards.
//!
//! Slot numbers are the `IDirect3DDevice9` vtable indices from d3d9.h.

use imgui::{DrawCmd, DrawData};

// IDirect3DDevice9 slots.
const DEV_GET_BACK_BUFFER: usize = 18;
const DEV_CREATE_TEXTURE: usize = 23;
const DEV_SET_RENDER_TARGET: usize = 37;
const DEV_GET_RENDER_TARGET: usize = 38;
const DEV_SET_DEPTH_STENCIL: usize = 39;
const DEV_GET_DEPTH_STENCIL: usize = 40;
const DEV_BEGIN_SCENE: usize = 41;
const DEV_END_SCENE: usize = 42;
const DEV_SET_VIEWPORT: usize = 47;
const DEV_SET_RENDER_STATE: usize = 57;
const DEV_CREATE_STATE_BLOCK: usize = 59;
const DEV_SET_TEXTURE: usize = 65;
const DEV_SET_TEXTURE_STAGE_STATE: usize = 67;
const DEV_SET_SAMPLER_STATE: usize = 69;
const DEV_SET_SCISSOR_RECT: usize = 75;
const DEV_DRAW_INDEXED_PRIMITIVE_UP: usize = 84;
const DEV_SET_FVF: usize = 89;
const DEV_SET_VERTEX_SHADER: usize = 92;
const DEV_SET_STREAM_SOURCE_FREQ: usize = 102;
const DEV_SET_PIXEL_SHADER: usize = 107;
// IUnknown / IDirect3DSurface9 / IDirect3DTexture9 / IDirect3DStateBlock9 slots.
const RELEASE: usize = 2;
const SURF_GET_DESC: usize = 12;
const TEX_LOCK_RECT: usize = 19;
const TEX_UNLOCK_RECT: usize = 20;
const SB_APPLY: usize = 5;

/// D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1.
const FVF: usize = 0x4 | 0x40 | 0x100;
const D3DFMT_A8R8G8B8: usize = 21;
const D3DFMT_INDEX16: usize = 101;
const D3DPOOL_MANAGED: usize = 1;
const D3DPT_TRIANGLELIST: usize = 4;
const D3DSBT_ALL: usize = 1;

/// One vertex as the fixed-function pipeline reads it: pixel position, rhw, ARGB colour, uv.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
    u: f32,
    v: f32,
}

// --- raw COM calls: stdcall, `this` first; floats travel as their bits -------------------------

unsafe fn slot(obj: usize, n: usize) -> usize {
    unsafe { *((*(obj as *const usize)) as *const usize).add(n) }
}

macro_rules! com {
    ($name:ident ( $($a:ident),* )) => {
        #[allow(clippy::too_many_arguments)]
        unsafe fn $name(obj: usize, n: usize $(, $a: usize)*) -> i32 {
            unsafe {
                let f: extern "system" fn(usize $(, $a: usize)*) -> i32 = std::mem::transmute(slot(obj, n));
                f(obj $(, $a)*)
            }
        }
    };
}
com!(c0());
com!(c1(a));
com!(c2(a, b));
com!(c3(a, b, c));
com!(c4(a, b, c, d));
com!(c8(a, b, c, d, e, f, g, h));

unsafe fn release(obj: usize) {
    if obj != 0 {
        unsafe { c0(obj, RELEASE) };
    }
}

fn ok(hr: i32) -> bool {
    hr >= 0
}

pub struct Backend {
    font_tex: usize,
    verts: Vec<Vertex>,
}

// The backend is only used from the render thread.
unsafe impl Send for Backend {}

impl Backend {
    /// Creates the font texture from the atlas pixels (RGBA8).
    pub unsafe fn new(dev: usize, w: u32, h: u32, rgba: &[u8]) -> Option<Backend> {
        unsafe {
            let mut tex = 0usize;
            let hr = c8(
                dev,
                DEV_CREATE_TEXTURE,
                w as usize,
                h as usize,
                1,
                0,
                D3DFMT_A8R8G8B8,
                D3DPOOL_MANAGED,
                &mut tex as *mut usize as usize,
                0,
            );
            if !ok(hr) || tex == 0 {
                crate::log(&format!("overlay: CreateTexture failed {hr:#x}"));
                return None;
            }
            let mut lr = [0usize; 2]; // D3DLOCKED_RECT { pitch, bits }
            let hr = c4(tex, TEX_LOCK_RECT, 0, lr.as_mut_ptr() as usize, 0, 0);
            if !ok(hr) || lr[1] == 0 {
                crate::log(&format!("overlay: font LockRect failed {hr:#x}"));
                release(tex);
                return None;
            }
            let pitch = lr[0];
            for y in 0..h as usize {
                let dst = (lr[1] + y * pitch) as *mut u8;
                for x in 0..w as usize {
                    let s = (y * w as usize + x) * 4;
                    // RGBA -> BGRA (A8R8G8B8 in memory).
                    *dst.add(x * 4) = rgba[s + 2];
                    *dst.add(x * 4 + 1) = rgba[s + 1];
                    *dst.add(x * 4 + 2) = rgba[s];
                    *dst.add(x * 4 + 3) = rgba[s + 3];
                }
            }
            c1(tex, TEX_UNLOCK_RECT, 0);
            Some(Backend { font_tex: tex, verts: Vec::new() })
        }
    }

    /// The back buffer's size, or None while the device cannot give it.
    pub unsafe fn back_buffer_size(dev: usize) -> Option<(u32, u32)> {
        unsafe {
            let mut bb = 0usize;
            if !ok(c4(dev, DEV_GET_BACK_BUFFER, 0, 0, 0, &mut bb as *mut usize as usize)) || bb == 0 {
                return None;
            }
            // D3DSURFACE_DESC: format, type, usage, pool, msaa type, msaa quality, width, height.
            let mut desc = [0u32; 8];
            let r = c1(bb, SURF_GET_DESC, desc.as_mut_ptr() as usize);
            release(bb);
            if ok(r) && desc[6] > 0 && desc[7] > 0 { Some((desc[6], desc[7])) } else { None }
        }
    }

    /// Draws `data` onto the back buffer, then puts the device back as it was.
    pub unsafe fn render(&mut self, dev: usize, data: &DrawData) {
        if data.total_vtx_count == 0 || data.display_size[0] <= 0.0 || data.display_size[1] <= 0.0 {
            return;
        }
        unsafe {
            let mut bb = 0usize;
            if !ok(c4(dev, DEV_GET_BACK_BUFFER, 0, 0, 0, &mut bb as *mut usize as usize)) || bb == 0 {
                return;
            }
            let mut sb = 0usize;
            if !ok(c2(dev, DEV_CREATE_STATE_BLOCK, D3DSBT_ALL, &mut sb as *mut usize as usize)) || sb == 0 {
                release(bb);
                return;
            }
            // Render target 0 and the depth surface are not part of a state block.
            let mut rt = 0usize;
            c2(dev, DEV_GET_RENDER_TARGET, 0, &mut rt as *mut usize as usize);
            let switch = rt != bb;
            let mut ds = 0usize;
            if switch {
                c1(dev, DEV_GET_DEPTH_STENCIL, &mut ds as *mut usize as usize);
                c2(dev, DEV_SET_RENDER_TARGET, 0, bb);
                c1(dev, DEV_SET_DEPTH_STENCIL, 0);
            }
            let began = ok(c0(dev, DEV_BEGIN_SCENE));
            self.setup(dev, data);
            self.draw(dev, data);
            if began {
                c0(dev, DEV_END_SCENE);
            }
            // Target first (it resets the viewport), then the block puts everything else back.
            if switch {
                c2(dev, DEV_SET_RENDER_TARGET, 0, rt);
                c1(dev, DEV_SET_DEPTH_STENCIL, ds);
            }
            c0(sb, SB_APPLY);
            release(sb);
            release(ds);
            release(rt);
            release(bb);
        }
    }

    unsafe fn setup(&mut self, dev: usize, data: &DrawData) {
        unsafe {
            let vp = [0u32, 0, data.display_size[0] as u32, data.display_size[1] as u32, 0f32.to_bits(), 1f32.to_bits()];
            c1(dev, DEV_SET_VIEWPORT, vp.as_ptr() as usize);
            c1(dev, DEV_SET_PIXEL_SHADER, 0);
            c1(dev, DEV_SET_VERTEX_SHADER, 0);
            c1(dev, DEV_SET_FVF, FVF);
            c2(dev, DEV_SET_STREAM_SOURCE_FREQ, 0, 1);
            let rs = |s: usize, v: usize| {
                c2(dev, DEV_SET_RENDER_STATE, s, v);
            };
            rs(7, 0); // ZENABLE
            rs(8, 3); // FILLMODE solid
            rs(9, 2); // SHADEMODE gouraud
            rs(14, 0); // ZWRITEENABLE
            rs(15, 0); // ALPHATESTENABLE
            rs(19, 5); // SRCBLEND = SRCALPHA
            rs(20, 6); // DESTBLEND = INVSRCALPHA
            rs(22, 1); // CULLMODE none
            rs(27, 1); // ALPHABLENDENABLE
            rs(28, 0); // FOGENABLE
            rs(29, 0); // SPECULARENABLE
            rs(52, 0); // STENCILENABLE
            rs(136, 1); // CLIPPING
            rs(137, 0); // LIGHTING
            rs(168, 0xF); // COLORWRITEENABLE
            rs(171, 1); // BLENDOP = ADD
            rs(174, 1); // SCISSORTESTENABLE
            rs(194, 0); // SRGBWRITEENABLE
            rs(206, 0); // SEPARATEALPHABLENDENABLE
            let ts = |s: usize, t: usize, v: usize| {
                c3(dev, DEV_SET_TEXTURE_STAGE_STATE, s, t, v);
            };
            ts(0, 1, 4); // COLOROP = MODULATE
            ts(0, 2, 2); // COLORARG1 = TEXTURE
            ts(0, 3, 0); // COLORARG2 = DIFFUSE
            ts(0, 4, 4); // ALPHAOP = MODULATE
            ts(0, 5, 2); // ALPHAARG1 = TEXTURE
            ts(0, 6, 0); // ALPHAARG2 = DIFFUSE
            ts(0, 11, 0); // TEXCOORDINDEX
            ts(0, 24, 0); // TEXTURETRANSFORMFLAGS = disable
            ts(1, 1, 1); // stage 1 COLOROP = DISABLE
            ts(1, 4, 1); // stage 1 ALPHAOP = DISABLE
            let ss = |t: usize, v: usize| {
                c3(dev, DEV_SET_SAMPLER_STATE, 0, t, v);
            };
            ss(1, 3); // ADDRESSU clamp
            ss(2, 3); // ADDRESSV clamp
            ss(5, 2); // MAGFILTER linear
            ss(6, 2); // MINFILTER linear
            ss(7, 0); // MIPFILTER none
            c2(dev, DEV_SET_TEXTURE, 0, self.font_tex);
        }
    }

    unsafe fn draw(&mut self, dev: usize, data: &DrawData) {
        let (fw, fh) = (data.display_size[0], data.display_size[1]);
        for list in data.draw_lists() {
            self.verts.clear();
            self.verts.extend(list.vtx_buffer().iter().map(|v| Vertex {
                x: v.pos[0] - 0.5,
                y: v.pos[1] - 0.5,
                z: 0.0,
                rhw: 1.0,
                // ImGui stores RGBA bytes; D3D wants ARGB in a DWORD.
                color: (v.col[3] as u32) << 24 | (v.col[0] as u32) << 16 | (v.col[1] as u32) << 8 | v.col[2] as u32,
                u: v.uv[0],
                v: v.uv[1],
            }));
            let idx = list.idx_buffer();
            for cmd in list.commands() {
                if let DrawCmd::Elements { count, cmd_params } = cmd {
                    let c = cmd_params.clip_rect;
                    let rect = [
                        c[0].max(0.0).min(fw) as i32,
                        c[1].max(0.0).min(fh) as i32,
                        c[2].max(0.0).min(fw) as i32,
                        c[3].max(0.0).min(fh) as i32,
                    ];
                    if rect[2] <= rect[0] || rect[3] <= rect[1] {
                        continue;
                    }
                    let first = cmd_params.idx_offset;
                    let vo = cmd_params.vtx_offset;
                    if count < 3 || first + count > idx.len() || vo >= self.verts.len() {
                        continue;
                    }
                    unsafe {
                        c1(dev, DEV_SET_SCISSOR_RECT, rect.as_ptr() as usize);
                        c8(
                            dev,
                            DEV_DRAW_INDEXED_PRIMITIVE_UP,
                            D3DPT_TRIANGLELIST,
                            0,
                            self.verts.len() - vo,
                            count / 3,
                            idx.as_ptr().add(first) as usize,
                            D3DFMT_INDEX16,
                            self.verts.as_ptr().add(vo) as usize,
                            std::mem::size_of::<Vertex>(),
                        );
                    }
                }
            }
        }
    }
}
