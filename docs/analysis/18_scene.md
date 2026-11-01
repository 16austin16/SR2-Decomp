# 18: The scene side of rendering

What fills the D3D command ring each frame. The executor side is in 07_render.md and the frame hand-off in 11_frame.md. Names are in `docs/functions.csv` (subsystem `render`).

## The render thread loop

`game_render_thread_frame` (0x522640) runs on the supervisor thread forever:

1. `pump_window_messages`. When the window is inactive, also `Sleep(50)` and lower the process priority.
2. **"wait-4-sim"**: wait on the hand-off job `0x2774198` that `frame_end_handoff` signals (11_frame.md), then create the next one ("game_render_thread.cpp" line 0xD0).
3. Set `0x277419C` = 1, telling the game thread that the frame was picked up.
4. **"main-render"**:
   1. `gfx_cmd_clear(7, 0xFF000000, 1.0, 0)`.
   2. Run the render context's deferred commands (0xCF8EA0(2)) and 0xD148D0.
5. **"build_render_batch"**, only when the world is visible (`0xE9D68C` and `0xE9D68D`):
   1. 0x52A3B0 (viewport).
   2. **`city_build_render_batches`** (0x4B7CC0, city_render.cpp): cull and batch the world chunks.
   3. 0x9FA9E0, 0x533910, then 0x533A70 (wakes the job workers), 0x533CD0.
   4. Two batch passes: 0x4B68D0(0x4B6550) + 0x4B6E10, then 0x4B68D0(0x4B6580) + 0x4B7070, each followed by 0x4B66A0 when `0xF83AF4` is set.
   5. 0x4B6760.
6. Set `0x27741AD` = 1. Then **`render_frame`** (0x52A710) when the world is visible, otherwise `gfx_command_buffer_wait`.
7. 0xD14700 (2D flush), then **`render_deferred_execute`** (0xCF8FA0), with `0x252A2A2` set while it runs.
8. **Publish** the ring (`InterlockedExchangeAdd(0x33D62F4, …)`, 07_render.md), then set `0x27741AC` = 1 (frame done) and run 0x522D00.

## The deferred command lists, step by step, for porting

The game has two command layers. Don't confuse them:

| Layer | Who writes | Who executes | Contents |
|---|---|---|---|
| **Deferred lists** (this section) | game thread (and the main menu / loading loops) | render thread, in `render_deferred_execute` | calls to the *graphics API* (`gfx_set_*`, `im2d_*`, …) recorded with their arguments |
| **D3D ring** (07_render.md) | render thread (and the game thread while `0xE849DB` is set) | device thread, in `gfx_execute_commands` | D3D9 calls (32 ids) |

A deferred command is a recorded call to a graphics API function. When replayed on the render thread, that function runs immediately and writes ring commands. So **no deferred handler calls D3D itself**. Every D3D call goes through the ring, and its device vtable slot is in the ring table in 07_render.md.

### How an API function decides to defer

Every function in the table below starts the same way:
1. Read the thread's graphics context: `ctx = *(*(fs:[0x2C]) + 0x60)`. TLS slot 0 → +0x60. The byte at +0x5C marks the render thread.
2. If it's the game thread's context `0xEC26A0`, the fields it writes go to the render context `0xEC2610` instead.
3. If `0xE849DB` (force immediate) is clear and the caller is the game thread (or `ctx` is `0xEC2610`), call `render_deferred_queue(type, args…)` and return.
4. Otherwise run immediately and write ring commands.

`0xE849DB` is set by `frame_end_handoff` (and the gamewide / loading code at 0x69AB80, 0x69AED0, 0x73CCF0) while the render thread is parked. It is cleared by `startup_loading_render_loop` and 0xD14BF0. While it is set, the game thread writes the ring directly.

There is a second per-thread structure: `gfx_get_thread_state` (0xD22790) returns `*(*(fs:[0x2C]) + 0x74)`, or `0x31BF4B0` on the device-owner thread (`0x252A3EC`). It holds the depth-bias stacks (+0x1688 / count +0x16C8, and the slope-scale stack with count +0x170C) and the float at +0x110.

### The list

There are two lists of 0x1C5C8 bytes, `0x2305980` and `0x2321F48`. The pointers are `0x230597C` (game side, written) and `0x2305978` (render side, executed). `frame_end_handoff` swaps them. `rl_mesh_inst::render` and 0xBDB2A0 temporarily repoint `0x230597C` to record into another list.

| Off | Field |
|---|---|
| +0x0000 | 3000 records of 8 bytes: {u32 type, u32 offset into the data area} |
| +0x5DC0 | record count |
| +0x5DC4 | data area (0x16800 bytes) |
| +0x1C5C4 | data used (0x16801 marks overflow) |

**`render_deferred_queue`** (0xCF8EA0): cdecl varargs `bool (int type, …)`, caller pops.
1. Refuse (return 0) when data used > 0x16800 or the count is 3000. Mark overflow by setting data used to 0x16801.
2. `ctx = {data + used, 0, 0x16800 - used}`.
3. Call `handler[type](0, &ctx, &va)`. The handler copies its arguments into `ctx.data` and adds their size to `ctx.used`. It returns 0 when they don't fit in `ctx.space`.
4. On success, append the record `{type, used}`, then `used += ctx.used` and `count += 1`.
5. The type is not range-checked.

**`render_deferred_execute`** (0xCF8FA0), on the render thread after `render_frame` (`0x252A2A2` is set around it):
1. For each record `i < count`: `ctx = {data + record.offset, 0, …}`, then `handler[record.type](1, &ctx, NULL)`.
2. Fill `0xE84AA8[0..15]` with -1. This is the per-stage bitmap handle cache of `gfx_set_texture_cached`.
3. It does **not** reset the count. The list is reset when it is swapped back in.

**Handler convention.** Every handler is `bool __cdecl h(int mode, ctx *c, va_list *args)`, and the caller pops 12.
- In mode 0 it reads from `*args` and writes to `c->data + c->used`.
- In mode 1 it reads the same fields back, advancing `c->used`, and calls the API function with the same registers and stack the original caller used.

Float arguments arrive in the varargs as doubles and are stored as 4-byte floats.

### Handler table (0xE847E0, 43 entries, types 0x00-0x2A)

Handlers are named `render_deferred_h<type>_<api>`. "Record" is what the handler stores, in order.

| Type | Handler | Replays | Convention of the API function (= record) | What it does |
|---|---|---|---|---|
| 00 | 0xCF9710 | `gfx_set_viewport_rect` 0xD149B0 | EDI = y; cdecl (x, w, h, no_clamp), pops 16 | ctx+8..+0x1C = x0, x1, y0, y1, w, h, clamped to the target size |
| 01 | 0xCFB170 | `gfx_set_scissor_rect` 0xD14D20 | ESI, EDI; cdecl (a, b), pops 8 | ctx+0x44..+0x50; `gfx_cmd_set_scissor` |
| 02 | 0xCF9700 | `gfx_invalidate_render_state_block` 0xD14950 | - | ctx+0x20 / +0x24 = -1 |
| 03 | 0xCF9670 | `gfx_set_render_state_block` 0xD148D0 | ECX = &{u32 a, u32 b} | if different from ctx+0x20 / +0x24: store, then `gfx_apply_render_state_block` |
| 04 | 0xCF95A0 | `gfx_set_color` 0xD14840 | cdecl (r, g, b, a) ints, pops 16 | ctx+0x28..+0x34 = v / 255 |
| 05 | 0xCF9540 | `gfx_set_alpha` 0xD147E0 | cdecl (a), pops 4 | ctx+0x34 = a / 255 |
| 06 | 0xCF9140 | `gfx_set_bitmap` 0x56C2B0 | ESI = bitmap handle | ctx+0x38 |
| 07 | 0xCF94A0 | `gfx_set_tint_color` 0xD14700 | ESI = &{f32 r, g, b} | ctx+0x70..+0x7C = (r, g, b, 1.0), then tail-jump `gfx_cmd_set_ps_c4` (pixel shader constant c4). Not the gamma ramp: that is `gfx_apply_gamma` 0xD14770 |
| 08 / 09 | 0xCFB100 / 0xCFB0F0 | `gfx_push_depth_bias` 0xD232C0 / `gfx_pop_depth_bias` 0xD23200 | cdecl (float), pops 4 / - | 16-deep stack; `D3DRS_DEPTHBIAS` (0xC3) = sum |
| 0A / 0B | 0xCFB080 / 0xCFB070 | `gfx_push_slope_depth_bias` 0xD23120 / `gfx_pop_slope_depth_bias` 0xD23060 | cdecl (float) / - | `D3DRS_SLOPESCALEDEPTHBIAS` (0xAF) |
| 0C | 0xCF9350 | `gfx_set_clip_plane` 0xD22090 | index + cdecl 4 floats | `gfx_set_user_clip_plane` 0xD18BD0: `CLIPPLANEENABLE` (0x98) through the cache, and **SetClipPlane called directly on the device** (not the ring) |
| 0D / 0E | 0xCFB000 / 0xCFAF90 | `gfx_set_state_float_110` 0xD23000 / `gfx_set_state_float_0e` 0xD22F90 | cdecl (float) | thread state floats (role open) |
| 0F | 0xCF92B0 | `gfx_set_shader_state3` 0xD09E20 | EAX = &{3 dwords} | 0xD18E50 (ring 7) and 0xD1CCF0 (render states + vertex declaration) |
| 10 | 0xCF91E0 | `gfx_clear_region` 0xD14B20 | registers + cdecl 2, pops 8 | `gfx_cmd_clear`, re-applies the state block |
| 11 | 0xCF9190 | `gfx_clear` 0xD14AD0 | ECX = stencil value | `gfx_cmd_clear` of the **stencil** only (flags 4) |
| 12 | 0xCFB740 | `gfx_draw_bitmap` 0xD11240 | EAX + cdecl 10, pops 0x2C | bitmap, state block, 0xD1F0D0 (2D quad) |
| 13 / 15 | 0xCFAD50 / 0xCFA8C0 | `gfx_draw_text` 0xD15C60 / `gfx_draw_text_wide` 0xCF5D90 | ESI, EDI + cdecl (3 floats, 1), pops 16; the record also holds the string | font layer 0xBD68E0 / 0xBDB5C0 |
| 14 / 16 | 0xCFAB00 / 0xCFA670 | `gfx_draw_text_3d` 0xD15BD0 | ESI, EAX + cdecl 4, pops 16 | font layer 0xBD6E60 |
| 17 | 0xCFA4E0 | `im2d_draw_17` 0xD0B020 | ECX + cdecl 4 floats, pops 16 | 2D shape (role open) |
| 18 / 19 / 1A | 0xCFA450 / 0xCFA340 / 0xCFA330 | `im2d_c_begin` 0xD0A580 / `_add` 0xD0A4F0 / `_end` 0xD0A4B0 | ESI = &{state pair} / ESI = count + cdecl (data) / - | batch at 0x22F6430 with its own vertex declaration |
| 1B / 1C / 1D | 0xCFA2A0 / 0xCFA190 / 0xCFA180 | `im2d_strip_begin` 0xD0A440 / `_add` 0xD0A380 / `_end` 0xD0A330 | ECX = &{state pair} / ESI = count ≤ 64 + cdecl (vertices) / - | strip; end = `gfx_2d_flush(TRIANGLESTRIP, n - 2)` |
| 1E / 1F / 20 | 0xCFA0F0 / 0xCF9FE0 / 0xCF9FD0 | `im2d_list_begin` 0xD0A2C0 / `_add` 0xD0A1F0 / `_end` 0xD0A190 | same | list; end = `gfx_2d_flush(TRIANGLELIST, n / 3)` |
| 21 | 0xCF9E60 | `im2d_draw_line` 0xD0A130 | ESI, EDI + cdecl (ptr), pops 4 | state block, 0xD1E330 |
| 22 / 23 / 24 | 0xCF9D00 / 0xCF9AD0 / 0xCF9910 | `im2d_fill_rect` 0xD0B980 / `im2d_draw_textured_rect` 0xD0B7D0 / `im2d_draw_rotated_bitmap` 0xD0A5E0 | cdecl 5 / EDI + 6 / 4 floats + 1 | 2D shapes over the strip / list batches (role open) |
| 25 | 0xCFBA10 | `gfx_draw_world_sprite` 0xD13A60 | EAX + cdecl (float, ptr), pops 8 | role open |
| 26 / 27 | 0xCF9810 / 0xCF9800 | `gfx_set_render_target_level` 0xD1EF80 / `gfx_restore_default_targets` 0xD1EF40 | ESI = target index, EDX = level + cdecl (3 bools) / - | 26: render target `0x22FDC28[index]` level `level` (GetSurfaceLevel), then optional clears; 27: the default render target, depth target and viewport (0x22F63F8 / 0x22F63FC) |
| 28 | 0xCFB240 | queued by `gfx_queue_28` 0xD09D90 (16 dwords from 0x25282F4…) | - | calls 0x7715E0 with 8 floats (the pause map) |
| 29 | 0xCFB450 | `gfx_draw_text_vint` 0xD15E80 | EDI, EAX + cdecl 6, pops 0x18 | role open |
| 2A | 0xCF9060 | `render_deferred_h_call_fn` | record {fn, arg} | `fn(arg)` on the render thread. `gfx_queue_call_79edf0` (0xCF90E0) uses it |

"Role open" rows are named from their shape. Port them by copying the record layout, and name them once traced in-game.

### State caches checked before writing the ring

| Cache | Indexed by | Written by | Dirty test |
|---|---|---|---|
| `0x22F57F0` u32[] | `D3DRENDERSTATETYPE` (e.g. +0x6C = `ALPHABLENDENABLE`, +0x2AC = `BLENDOP`, +0x30C = `DEPTHBIAS`) | `gfx_apply_render_state_block`, the depth-bias pushes, `gfx_set_clip_plane` | `cache[state] != value` → write ring 15 and update. `0xBAADF00D` means "never set" for depth bias |
| `0x22F52F0` u32[] | sampler state for stage 0 (+0x14 MAGFILTER, +0x1C MIPFILTER) | `gfx_2d_flush`, 0xD1E070 | same; writes ring 19 |
| `0x22F2AF0` [16] | stage → D3D texture | `gfx_set_texture_cached`, `gfx_cmd_set_texture` | texture differs → ring 14 |
| `0xE84AA8` [16] | stage → bitmap handle | `gfx_set_texture_cached` | handle differs. Reset to -1 by `render_deferred_execute` and each frame |
| ctx+0x20 / +0x24 | the last state block | `gfx_set_render_state_block` | pair differs. `gfx_invalidate_render_state_block` forces a re-apply |
| `0x22F5BF0` [16], `0x22F63F4`, `0x22F63F0` | executor side: texture, vb, ib | `gfx_execute_commands` | no test; kept for device reset |

A shadow check must compare the ring bytes *after* these caches. The same call can emit nothing when the cache already holds the value.

### Shared globals (locking for a shadow check)

| Global | Writers | Readers | Synchronisation |
|---|---|---|---|
| `0x230597C` / `0x2305978` list pointers | game thread (`frame_end_handoff` swaps them) | render thread | swapped only while the render thread waits on the hand-off job `0x2774198`. The flags `0x277419C` (picked up) and `0x27741AC` (frame done) are in 11_frame.md |
| `0xE849DB` force immediate | game thread, around the hand-off and loading | every API function | plain byte, no lock. Only changed while the render thread is parked |
| ring write `0x33D62F8`, last published `0x33D62FC` | the thread that owns writing (the render thread; the game thread while `0xE849DB` is set) | - | single writer by protocol |
| ring published end `0x33D62F4` | `InterlockedExchangeAdd` in `gfx_end_frame` | device thread | interlocked |
| ring read `0x33D6300` | device thread | `gfx_command_buffer_wait` (spins with `Sleep(1)`) | interlocked rewind |
| deferred-release list `0x252A2BC` | any thread | device thread at EndScene | critical section `0x2301CCC` |
| `0x252A2A2` "executing deferred" | render thread | `gfx_2d_flush`, `gfx_cmd_set_scissor` (coordinate scaling) | plain byte |
| `0x252A3EC` device-owner thread id | device creation | `gfx_get_thread_state` and about 30 others | written once |

### Porting order (leaves first)

| Step | Functions | Depends on |
|---|---|---|
| 1 | the ring writers `gfx_cmd_*` (07_render.md) | the ring |
| 2 | the caches above, `gfx_set_texture_cached` 0x4D0D70, `gfx_apply_render_state_block` 0xD1CDB0 | 1, bitmap lookup 0xC094E0 |
| 3 | `gfx_get_thread_state` 0xD22790 and the TLS context accessor (inline in each function) | - |
| 4 | the plain setters: viewport, scissor, state block, color, alpha, bitmap, depth bias push / pop, clear | 1-3 |
| 5 | `gfx_2d_flush` 0xD1DF50 and the 2D batches (`im2d_*_begin / _add / _end`, `im2d_draw_quad`, `gfx_draw_bitmap`) | 1-4 |
| 6 | `render_deferred_queue`, the 43 handlers, `render_deferred_execute` | 4-5. The handlers only copy arguments, so port them per type alongside their API function |
| 7 | the font (`gfx_draw_text_*` → 0xBD6xxx) and "role open" types | 5 |

Each API function's deferred path and immediate path can be ported together: the deferred path is only the `render_deferred_queue` call.

### Remaining leaves, step by step

**`gfx_bind_shader_pair`** (0xD19070; `EAX` = pair index; no stack arguments; called by 0xD1CCF0, 0xD1DEB0 and 0xD1E070):
1. `entry = 0xE84AF0 + index × 0x1C`: +0 vertex shader, +4 pixel shader, +0xC vertex declaration (used by the 2D allocator). `vs = entry[+0]`, or the default `0xE84B98` when 0.
2. If `vs != 0x22F2AEC` (vertex shader cache): write ring `{4, vs}` (SetVertexShader, 8 bytes) and update the cache.
3. `ps = entry[+4]`, or the default `0xE84B9C` when 0.
4. If `ps != 0x22F2AE8` (pixel shader cache): write ring `{3, ps}` (SetPixelShader) and update the cache.

The 0x1C-byte entries are filled at shader load. The default pair is entry 6 (0xE84B98 = 0xE84AF0 + 6 × 0x1C). Corrected from 0x38 by the main session's port.

**`gfx_set_tint_color`** (0xD14700; `ESI` = &{f32 r, g, b}):
1. `ctx+0x70..+0x7C` = (r, g, b, 1.0). This is done on every call, before the defer test.
2. Defer as type 0x07 (record = the 3 floats), or tail-jump `gfx_cmd_set_ps_c4(EAX = ctx+0x70)`.

**`gfx_cmd_set_ps_c4`** (0xD18DE0; `EAX` = &float4):
1. Copy the 4 floats to `0x2304D9C..0x2304DA8` (a copy kept for the device-reset restore).
2. Write ring `{7, 4, 1, f0, f1, f2, f3}` (SetPixelShaderConstantF(4, data, 1), 0x1C bytes). There is no cache test.

**`gfx_apply_gamma`** (0xD14770; no arguments; reached by fall-through after `gfx_set_tint_color`'s body, with its own entry at 0xD14770):
1. Pick the context as usual (0xEC26A0 → 0xEC2610).
2. If `ctx+0x5C` (byte, gamma dirty) is 1, call `gfx_set_gamma_ramp(lo = ctx+0x64, hi = ctx+0x68, gamma = ctx+0x60)`.
3. Keep copies: `0xE84A28` = gamma, `0x252A320` = lo, `0xE84A2C` = hi.
4. Clear `ctx+0x5C`.

**`gfx_set_gamma_ramp`** (0xD1D840; cdecl `(f32 lo, f32 hi, f32 gamma)`, caller pops 12):
1. `dev->GetDeviceCaps(&caps)` (slot 7). Unless `caps.Caps2 & D3DCAPS2_FULLSCREENGAMMA (0x20000)`, return.
2. For i = 0..255:
   1. `v = pow(i / 255.0, gamma) × (hi − lo) + lo` (pow is the CRT x87 helper 0xCA05D0).
   2. Clamp `v` to [0, 1].
   3. `w = (int)(v × 1024) << 6`, clamped to 0xFFFF.
   4. Store `w` into red[i], green[i] and blue[i] of a `D3DGAMMARAMP` on the stack.
3. `dev->SetGammaRamp(0, 0, &ramp)` (slot 21).

**Device thread.** This calls the device **directly**, not through the ring. It runs on whichever thread applies the context: the render thread, or the game thread while `0xE849DB` is set. Keep that in mind for the device-thread-only rule of the ring executor.

**`gfx_set_user_clip_plane`** (0xD18BD0; `ECX` = plane index, cdecl `(f32 a, b, c, d)`; returns with `ret`, caller pops 16; called only by `gfx_set_clip_plane`):
1. If index > 5, return.
2. If a, b and c are all 0.0 (the `fucom` tests), the plane is off:
   1. `mask = 0x252A314 & ~(1 << index)`.
   2. If `mask` differs from the render-state cache `0x22F5A50` (`D3DRS_CLIPPLANEENABLE`, 0x98) and isn't `0xBAADF00D`, write ring `{15, 0x98, mask}` and update the cache.
   3. Return.
3. Otherwise:
   1. `mask = 0x252A314 | (1 << index)`, with the same cached SetRenderState(0x98).
   2. Then `dev->SetClipPlane(index, {a, b, c, d})` (slot 55) **directly on the device**, not through the ring.

`0x252A314` is the enabled-plane mask, and it is kept even when the cache already matches.

### 2D batches and shapes, step by step

**Shader-pair entries** (0xE84AF0 + index × 0x1C):

| Off | Field |
|---|---|
| +0 | vertex shader |
| +4 | pixel shader |
| +0xC | vertex declaration |
| +0x10 | vertex stride |

The 2D code picks between them:
- entries 0-3 for the declared-vertex batch: 0 / 1 textured / untextured, 2 / 3 the same when `pair.b & 0xF00`;
- entries 4 / 5 for the strip / list batch: textured / untextured.

The vertex declaration goes through its own cache `0x252A30C` (ring 5, SetVertexDeclaration).

**`gfx_2d_alloc`** (0xD1E070; cdecl `(count)`; returns the vertex buffer `0x22F6438` (0x1C stride), or 0 when a batch is already pending):
1. If `0x252A2FC` (pending) is set, return 0. Callers flush first.
2. `0x252A2FC = 1`, `0x252A2F8 = count`.
3. Sampler 0 MIPFILTER (7) and MAGFILTER (5) = 2 (linear), through the sampler cache `0x22F530C` / `0x22F5304` (ring 19).
4. `index = 4 + (0xE84AA8[0] < 0)`. The 0xE84AA8 bitmap cache tells whether a texture is bound.
5. Set the declaration from that entry through the cache.
6. `gfx_bind_shader_pair(EAX = index)`.
7. Return `0x22F6438`.

The vertices are flushed later by `gfx_2d_flush` (strip / list end, or the next shape).

**Strip / list vertex** (0x1C bytes):

| Off | Field |
|---|---|
| +0 / +4 | f32 x, y (screen) |
| +8 / +0xC | not written by the 2D code |
| +0x10 | u8 r, g, b, a |
| +0x14 / +0x18 | f32 u, v |

**Declared-vertex batch (`im2d_c_*`).** Its vertices are 0x24 bytes and its stride comes from the entry.
- **`im2d_c_begin`** (0xD0A580; `ESI` = &state pair):
  1. Defer as type 0x18. Otherwise:
  2. `gfx_set_render_state_block(ECX = pair)`.
  3. `0x252A300 = gfx_2d_bind_material(pair)` (0xD1DEB0). That binds stage 0 to the context bitmap (`gfx_set_texture_cached(0, ctx+0x38)`), picks entry `(pair.b & 0xF00 ? 2 : 0) + (bitmap < 0)`, sets its declaration and shaders, stores the stride in `0x22F6430` and returns it.
  4. `0xE863C0 = −1`.
- **`im2d_c_add`** (0xD0A4F0; `ESI` = count; cdecl `(src)`; returns the vertex pointer):
  1. **Deferred**: type 0x19 with a VTX of `count × 0x24` bytes; return the record's space for the caller to fill.
  2. **Immediate**: `gfx_2d_c_alloc(count)` (0xD1E600). If a batch is open (`0xE863C0 >= 0`), it first draws it: `gfx_cmd_draw_up(ECX = n − 2, EDI = 5 (strip), EDX = stride, push 0x22FD438)`, then clears the eight dwords `0x2304D6C..0x2304D98`. Then `0xE863C0 = count`, and it returns `0x22FD438`.
  3. If `src` is given, memcpy `count × 0x24` bytes into the buffer.
- **`im2d_c_end`** (0xD0A4B0): defer as type 0x1A, or `gfx_2d_c_flush` (0xD1E590): the same draw as above, then `0xE863C0 = −1`.

**`gfx_cmd_draw_up`** (0xD217F0): `ECX` = primitive count, `EDI` = D3DPRIMITIVETYPE, `EDX` = stride, then a push of the data pointer; the caller pops 4.

**`gfx_clip_rect_to_viewport`** (0xD0A050; cdecl `(int *x, int *y, int *w, int *h)` → bool):
1. Offset x / y by the viewport origin (ctx+8 / +0x10).
2. Clip against the viewport (origin + size at ctx+0x18 / +0x1C). It adjusts x / y / w / h in place.
3. Return 0 when nothing is left.

**Shapes:**

| Type | API (renamed) | Convention | Steps |
|---|---|---|---|
| 21 | `im2d_draw_line` 0xD0A130 (was im2d_draw_quad) | `ESI` = &pair, `EDI` = &p0 (vec3), cdecl `(&p1)` | Defer, or `gfx_set_render_state_block`, then `gfx_draw_line_immediate` (0xD1E330; `EAX` = &p0, `ECX` = &p1, cdecl `(&pair)`). That builds 2 vertices with the colour × 255 (u, v = 0, 0 and 1, 1), binds no bitmap (`gfx_set_bitmap(−1)`) and `gfx_2d_bind_material(pair)`, then `gfx_cmd_draw_up(1, D3DPT_LINELIST, stride, verts)` and clears 0x2304D6C..D98 |
| 22 | `im2d_fill_rect` 0xD0B980 | cdecl `(int x, y, w, h, &pair)` | `gfx_clip_rect_to_viewport` (return if empty), defer, or: no bitmap, colour × 255, `im2d_strip_begin(pair)`, `im2d_strip_add` of 4 corners, `im2d_strip_end` |
| 23 | `im2d_draw_textured_rect` 0xD0B7D0 | `EDI` = bitmap; cdecl `(int x, y, w, h, 4 UV values, &pair)` | Clip, defer, or: bitmap, colour, one strip quad with the given UV rectangle |
| 12 | `gfx_draw_bitmap` 0xD11240 | `EAX` + cdecl 10, pops 0x2C | Defer, or: bitmap and state block; 0xD09E70 computes the clipped screen and UV rectangle; 0xD1F0D0 binds the texture and emits one strip quad (`im2d_strip_begin`, `gfx_2d_flush` + `gfx_2d_alloc(4)`, or deferred type 0x1C with the write-back) |
| 24 | `im2d_draw_rotated_bitmap` 0xD0A5E0 | `u32` + cdecl 4 floats + &pair | Uses sin / cos (0x428CD0 / 0x40B820) of an angle and the bitmap's size (0xC094E0) to rotate the quad's corners, then one strip quad. *Name from structure* |
| 17 | `im2d_draw_17` 0xD0B020 | `ECX` + cdecl 4 floats | 400 FPU instructions, texture via `gfx_set_texture_cached`, `gfx_clip_rect_to_viewport`, 0xD1E690. Several quads (probably a stretched or framed bitmap). *Role still open* |
| 25 | `gfx_draw_world_sprite` 0xD13A60 | `EAX` + cdecl `(f32, ptr)` | Projects a world position through the thread state's camera (0xD22CE0 reads `gfx_get_thread_state()+0x1554 / +0x1720`), sin / cos for rotation, then a screen quad. *Name from structure* |
| 29 | `gfx_draw_text_vint` 0xD15E80 | `EDI`, `EAX` + cdecl 6, pops 0x18 | vint's text draw (draw slot +0x28, 06_vint.md): UTF-16 text through 0xD156D0 (`font_lookup`, `font_glyph_metrics_wide`, `im2d_strip_begin`) inside a rect, with vint's state pair 0x2A14B28; sets sampler states first |

### The text path, step by step

Four deferred types carry text. The API functions are thin wrappers; the drawing happens in the font layer (0xBD6xxx-0xBDCxxx), which builds 2D quads through the strip batch (2D text) or the declared-vertex batch (3D text).

| Type | API (renamed) | Font layer | Text |
|---|---|---|---|
| 13 | `gfx_draw_text` 0xD15C60 | `font_draw_string_2d` 0xBD68E0 | `char *`, 2D |
| 15 | `gfx_draw_text_wide` 0xCF5D90 | `font_draw_string_2d_wide` 0xBDB5C0 | `u16 *` (UTF-16), 2D |
| 14 | `gfx_draw_text_3d` 0xD15BD0 | `font_draw_string_3d` 0xBD6E60 | `char *`, 3D (position + orientation) |
| 16 | none: no queue site in the binary | its handler also replays `gfx_draw_text_3d` | unused |

**`gfx_draw_text` / `gfx_draw_text_wide`** (`ESI` = string, `EDI` = &state pair; cdecl `(f32 x, f32 y, f32 scale, i32 font)`, caller pops 16):
1. Return if the string is NULL or empty.
2. If `gfx_should_defer()` (0x56C310: `0xE849DB` clear and a game-thread context), queue type 13 / 15. The record is x, y, the string (with its length including the terminator; ×2 bytes for wide), scale, font and the state pair. Return.
3. Otherwise call the font layer: `font_draw_string_2d(x, y, str, scale, font, &pair)` (cdecl 0x18 bytes).

**`gfx_draw_text_3d`** (`ESI` = string, `EAX` = font, cdecl `(vec3 *pos, mat3 *orient, f32 scale, &pair)`):
- It defers as type 14 (the record copies the vec3, the 3×3 matrix, the scale, the string and the pair).
- Otherwise it calls `font_draw_string_3d(EAX = pos, EDI = orient; scale, str, font, &pair)`.

**`font_lookup`** (0xC15CF0; `EAX` = font id; returns the font record or 0):

| Font id | Result |
|---|---|
| ≥ 0 | `0x234F268[id]`, while id < `0x252A5C0` |
| −1 | the default font `0x252A5C8` |
| below −1 | an alias: entry `(id + 0x7FFFFFFF)` of the 8-byte table `0x234F238` = `{f32 scale, i32 font id}` (count `0x252A5CC`), looked up again |

**Font record** (fields used here):

| Off | Field |
|---|---|
| +0x08 | glyph count |
| +0x0C | first character code |
| +0x10 | default advance (characters outside the font) |
| +0x14 | s16 line height |
| +0x16 | s16 used for the inverse scale |
| +0x1C | letter spacing added to every advance |
| +0x20 | kerning pair count |
| +0x24 / +0x26 | s16 baseline y / x offsets |
| +0xA8 | kerning pairs, 4 bytes `{u8 left, u8 right, s8 adjust, pad}`, grouped by left glyph |
| +0xAC | glyph table, 16 bytes per glyph: +0 advance, +4 width, +0xC s16 index of the glyph's first kerning pair (−1 = none) |
| +0xB0 | the font's bitmap handle |
| +0xB4 / +0xB8 | per-glyph u / v pixel tables (i32) |

**`font_draw_string_2d`** (0xBD68E0; cdecl `(f32 x, f32 y, char *str, f32 scale, i32 font, pair *state)`; plain `ret`):
1. Return if the string is NULL or empty.
2. `f = font_lookup(font)`. `k` = the alias scale (1.0 unless the id is an alias). `sx = k × scale`.
3. If `x == 9999.123` (`0xE5CCE8`), the line is centred: `x = text_center_x(str, scale, font)` (0xD15520; `EAX` = str).
4. `pen_x = x + f.base_x(+0x26) × sx`, `pen_y = y + f.base_y(+0x24) × sx`.
5. `gfx_set_bitmap(f+0xB0)`.
6. Vertex colour from the context: R, G, B = `ctx+0x28 / +0x2C / +0x30 × 128` (bytes, so 1.0 maps to 128 for 2× modulate). A = `ctx+0x34 × 255`.
7. The bitmap's size (0xC094E0 lookup → +4 / +6) gives the half-texel offsets `0.5 / w` and `0.5 / h`.
8. `im2d_strip_begin(ECX = state)`.
9. For each character:
   - **`\n`**: `pen_y += round(f.line_height(+0x14) × sx)`, then reset `pen_x` (centring again on the rest of the string when the sentinel was given).
   - **Other characters below 0x20**: skipped.
   - **Printable characters**:
     1. `font_glyph_metrics` (0xBD7D70) gives the glyph index, width and advance, with kerning against the *next* character (described below).
     2. `u0 = f.u[g] / w + 0.5 / w`, `v0 = f.v[g] / h + 0.5 / h`, and u1 / v1 from the glyph size.
     3. Get space for 4 vertices of 0x1C bytes:
        - **deferring**: `render_deferred_queue(0x1C, 4, &{dst = 0, bytes = 0x70})`, then write the vertices to the `dst` it returns (the VTX write-back);
        - **immediate**: when a strip is open (`0xE863C4 >= 0`), flush it (`gfx_2d_flush(5, count − 2)`), then `0xE863C4 = 4` and `0xD1E070(4)` returns the vertex pointer.
     4. Fill the quad: four vertices `{f32 x, f32 y, (+8 / +0xC left as is), u8 r, g, b, a at +0x10, f32 u, v at +0x14 / +0x18}` for the glyph's corners.
     5. `pen_x += advance × scale`.
10. Tail-jump `im2d_strip_end`.

So **each glyph is its own 4-vertex strip and its own draw** in immediate mode.

**`font_glyph_metrics`** (0xBD7D70; `ECX` = font id, `ESI` = &advance; cdecl `(u8 c, u8 next, int *width)`; returns the glyph index or −1):
1. `f = font_lookup(id)`. `k` = the alias scale (1.0 unless the id is an alias).
2. `g = c − f.first_char`. If `g` is outside `[0, f.count)`:
   1. `*width = 0`.
   2. `*advance = trunc((f.default_advance + f.spacing) × k)`.
   3. Return −1.
3. Otherwise, with `e = f.glyphs[g]` (16-byte entries): `*width = e.width` and `adv = e.advance + f.spacing`.
4. **Kerning.** Only when `e.kern_start > −1` and `next` isn't 0 or `
`, and `next` is inside the font:
   1. Scan the pairs from `kern_start` while the left glyph is `g` and the right glyph is below `next − first`.
   2. On an exact `(g, next)` match, add the pair's signed adjust to `adv`.
5. `*width = trunc(*width × k)`, `*advance = trunc(adv × k)`. Return `g`.

There's no cache: every glyph is looked up again on every draw.

**`font_draw_string_2d_wide`** (0xBDB5C0) is the same with `u16` characters, `text_center_x_wide` (0xCF5C20) and `font_glyph_metrics_wide` (0xBDC760).

**`font_draw_string_3d`** (0xBD6E60; `EAX` = &pos, `EDI` = &orient; cdecl `(f32 scale, char *str, i32 font, pair *state)`):
- It follows the same per-glyph steps (font lookup, bitmap, glyph metrics).
- It builds the quads in 3D: each corner is `pos + orient × (pen offset × scale)`.
- It uses the declared-vertex batch: `im2d_c_begin`, then type 0x19 / `0xD1E600` for the space (VTX write-back again when deferring).

### Deferred record layouts

What each handler stores in mode 0. Fields are 4 bytes in order; `f32` arguments arrive in the varargs as doubles and are stored as floats. Mode 1 reads the same fields back and calls the API with the convention in the handler table.

**Shared pieces:**

| Piece | Meaning |
|---|---|
| `pair8` | the `{u32, u32}` render-state pair (`gfx_set_render_state_block`'s argument), passed by pointer and copied by value; it always comes last |
| `STR` | `u32 len`, then `len` bytes padded to 4 |
| `VTX` | `u32 count`, `u32 bytes`, then `bytes` of vertices padded to 4. In mode 0 the handler writes the address of that space back into the caller's `{dst*, bytes}` struct, so **the caller fills the vertices after `render_deferred_queue` returns**. Execute skips the call when count > 64 |

**Records by type:**

| Type | API | Record |
|---|---|---|
| 00 | `gfx_set_viewport_rect` | y, x, w, h, no_clamp |
| 01 | `gfx_set_scissor_rect` | 4 × u32 (2 registers + 2 stack) |
| 02, 09, 0B, 1A, 1D, 20, 27 | (no arguments) | empty |
| 03 | `gfx_set_render_state_block` | pair8 |
| 04 | `gfx_set_color` | r, g, b, a |
| 05 | `gfx_set_alpha` | a |
| 06 | `gfx_set_bitmap` | handle |
| 07 | `gfx_set_tint_color` | f32 r, g, b |
| 08, 0A, 0D, 0E | the float setters | f32 |
| 0C | `gfx_set_clip_plane` | u32 index, f32 a, b, c, d |
| 0F | `gfx_set_shader_state3` | 3 × u32 (12 bytes copied) |
| 10 | `gfx_clear_region` | 4 × u32 |
| 11 | `gfx_clear` | u32 stencil |
| 12 | `gfx_draw_bitmap` | 11 × u32 (EAX + 10 stack), pair8 |
| 13, 15 | `gfx_draw_text` / `_wide` | f32 x, f32 y, STR text, u32, f32 scale, u32, pair8 |
| 14, 16 | `gfx_draw_text_3d` (16 unused) | 12 bytes (vec3 position), 36 bytes (3 × 3 orientation), f32, STR text, u32, u32, pair8 (3D text) |
| 17 | `im2d_draw_17` | f32 × 4, pair8 |
| 18, 1B, 1E | `im2d_*_begin` | pair8 |
| 19, 1C, 1F | `im2d_*_add` | VTX |
| 21 | `im2d_draw_line` | 12 bytes (from ESI), 12 bytes (from EDI), pair8 |
| 22 | `im2d_fill_rect` | 4 × u32, pair8 |
| 23 | `im2d_draw_textured_rect` | 8 × u32, pair8 |
| 24 | `im2d_draw_rotated_bitmap` | u32, f32 × 4, pair8 |
| 25 | `gfx_draw_world_sprite` | 12 bytes, f32, pair8 |
| 26 | `gfx_set_render_target_level` | index, level, 3 × bool |
| 28 | (pause map draw 0x7715E0) | f32 × 8 |
| 29 | `gfx_draw_text_vint` | f32, f32, STR, u32, 12 bytes, 12 bytes, u32, u32, pair8 |
| 2A | callback | fn, arg |

Each handler checks `used + size <= space` before every field. When a field doesn't fit, it returns 0 and `render_deferred_queue` marks the list overflowed (0x16801). The VTX check happens before the space is handed out.

### In Rust (gfx_cmd.rs)

Ported and verified in one batch: 16 ring writers (`gfx_cmd_*`, steps 1-2) and 22 API
functions (steps 2-5): `gfx_set_texture_cached`, `gfx_apply_render_state_block`, set / invalidate
state block, viewport rect and its reset (0xD14D90), scissor rect, color, alpha, bitmap, the
depth and slope bias stacks, the two clears, `gfx_restore_default_targets`, `gfx_2d_flush`, and
the strip / list begin and end. Their deferred path calls the original `render_deferred_queue`.

**Check.** Several of these publish the ring, which cannot be undone, so the Rust runs first as
a prediction: writes to an overlay, publishes and `render_deferred_queue` calls to a list. Then
the original runs live with `InterlockedExchangeAdd` (import 0xDB809C) and the queue taped, and
every predicted dword, about 1,500 watched cache dwords, the publishes and the queued calls
(type and arguments) are compared. A hooked function reached inside a check runs as the original.
Result over a scripted play run: about 30 million calls, 0 differences; then the Rust drew the
game normally.

Not ported yet: `gfx_cmd_set_vertex_declaration`, the three shader-constant writers and
`gfx_cmd_draw`. They never ran in the play run (those commands are written inline elsewhere),
so they stay original. Also not yet: the 2D `_add` functions (they allocate through 0xD1E070 /
0xD19070), `gfx_set_tint_color` (0xD14700), the clip plane and text.


**Deferred lists (second batch).** `render_deferred_queue`, `render_deferred_execute` and the
handlers of 25 types are Rust: 00-0B, 0D-11, 18, 1A, 1B, 1D, 1E, 20, 27 and 2A. Their records are
four shapes: dwords (each written only if it still fits), a double stored as a float, a block
copied from a pointer (all or nothing), or nothing. The other 18 types (2D add, text, shapes,
the bitmap draw, 0x0C, 0x26, 0x28, 0x29) keep their original handlers, called from the Rust
queue and replay loop.

Check: the API checks' predictions now include the list bytes a deferred call writes, and the
queue itself is predicted when called from unported code; replay is checked by decoding each
executed list in Rust into the API calls its handlers should make, then executing the original
with every handler-table entry routed through a shim and the handlers' direct API calls taped
from their hooks (registers, stack, and the pair for the block types; types whose API is not
hooked tape only their type). Over a play run: 1,264,367 queued calls and 3,599 executed lists,
0 differences; then the Rust lists drew the game. Tapes are per thread (the game and render
threads check at the same time).


**2D vertex space, tint (third batch).** Also Rust: `gfx_bind_shader_pair`, `gfx_2d_alloc`,
`im2d_strip_add` / `im2d_list_add`, `gfx_cmd_set_ps_c4`, `gfx_set_tint_color`, and the handlers of
types 19, 1C and 1F (vertex space: the record's space address goes back into the caller's
{dst, bytes}; replay skips counts over 64). Checked with 1,285,885 queued calls and every replay,
0 differences; live, the HUD text and minimap draw through it. The clip plane
(`gfx_set_clip_plane`, `gfx_set_user_clip_plane`, type 0C) is written but is dead code on PC (no caller,
nothing queues type 0C; Function Analysis), so it
stays original. In the check, the {dst, bytes} pointer an add passes to the queue is each caller's
own stack and is not compared (its contents are).

Details:
- `gfx_apply_render_state_block`: blend modes 0-12 (0 off; 1 one/one add; 2 srcalpha/one add;
  4 destalpha/one add; 5 srcalpha/invsrcalpha; 6 destalpha/invdestalpha; 7 invdestalpha/
  destalpha; 9 one/one revsubtract; 10 srcalpha/one revsubtract; 12 destcolor/zero add; 3, 8,
  11 change nothing), address mode (0 wrap, 1 clamp, sampler 0 U and V), depth (0 off; 1
  lessequal, no write; 2 equal, no write; 4 lessequal + write), alpha test / stencil (0 stencil
  off; 1 alpha > 0 and stencil replace 1; 2 stencil equal 1, keep), cull (0 none, 1 CCW).
- `gfx_clear` (ECX) clears the **stencil** to ECX (flags 4, color 0). `gfx_clear_region` (CL,
  DL, cdecl 2 bytes) clears all three to color cl<<24 | a<<16 | b<<8 | dl, then re-applies the
  state block at 0xEC2748.
- The deferral test is: force-immediate clear and (raw context is the render context, or the
  render-thread byte is clear and the raw context is the game context).
- `gfx_get_thread_state` returns the TLS state; 0x31BF4B0 only when that state is 0x31C0C20
  and the thread is not the device owner (the 07 note had it the other way round).

## World batch passes, for porting

This covers what `game_render_thread_frame`'s "build_render_batch" block does before `render_frame`. Everything here runs on the render thread. The job workers only help through the job object `0x2526CC8`.

**Batch accumulators.** Two global "callback + buffer + count" accumulators collect work items and hand them to a callback in groups:

| Accumulator | Callback | Buffer | Count | Group | Start | Flush |
|---|---|---|---|---|---|---|
| A (chunks) | `0xF83AFC` | `0xF83B04` | `0xF83B00` | 5 × 16-byte chunk entries | `city_batch_begin(cb)` 0x4B6830 | `city_batch_flush` 0x4B65F0 |
| B (instances) | `0xF83AF0` | `0xF83AF8` | `0xF83AF4` | 20 × 0x24-byte entries | `city_instance_batch_begin(cb)` 0x4B68D0 | `city_instance_batch_flush` 0x4B66A0 |

- **Start** (cdecl `(cb)`):
  1. Create the job object `0x2526CC8` once, from the pool `0xEC5BB8` (vtable +8), and clear its +0x28 / +0xAC.
  2. Allocate a group buffer from the allocator `0x2774148` (vtable +0x1C, `(bytes, 0)`).
  3. Count = 0, store the callback.
- **Flush:**
  1. Call `cb(&{…, buffer, count, 6}, buffer, count)` on this thread.
  2. Run the job object's vtable +4 and +0xC (kick, then sync).
  3. Allocate a fresh buffer and reset the count.

  The per-entry functions append and flush when a group is full (0x4B6970 → `city_batch_flush` / `city_instance_batch_flush`).

**`frustum_test_aabb`** (0x530270; `ECX` = &min (vec3), `EDX` = &max; cdecl `(planes *, …)`; returns bool):
- Uses SSE. `planes` = {u32 count, then 16-byte planes from +0x10}, with a per-plane sign mask at +0xF0 that picks the AABB corner.
- An AABB outside any plane fails. With `0xE8E88A` clear, the test is skipped (always visible).
- Every per-instance step calls it.

**`city_build_render_batches`** (0x4B7CC0, city_render.cpp), in order:
1. `0xFA6C28` = the pass 0 list count `0x2784990` (for the stat at step 15).
2. Camera vectors from 0x4D9120 / 0x4D91E0 → `0x2615720..0x2615734`. This is done twice, the second for the current view. The camera position `0x319BEB4..BC` → `0x2612D04..0C`.
3. Clear each streamed chunk's visibility bitmap: for each of the `0xF6FBC4` entries of `0xF6FBD8[]`, memset(+0x58, 0, +0x54).
4. Profile "citycul:1".
5. **Cull the chunks.** If there are views (`0x2526CCC`) and view 0 has flag 0x2000 at `0x2612D18`:
   - begin A with 0x4B64F0 (per entry 0x4B5CE0 → 0x4B5FF0 → 0x4B6160 / 0x4B6310);
   - then `0x4B6B60`;
   - the main view index starts at 1.

   Otherwise:
   - begin A with 0x4B64C0 (per entry 0x4B5C40, the AABB test);
   - then `0x4B6A20`;
   - start at 0.
6. Flush A if anything is pending (`0xF83B00`).
7. Profile "citybrb:3".
8. **Per-view batches.** Begin A with 0x4B6520 (per entry 0x4B5FF0: `frustum_test_aabb` against view N, then 0x4B6160 / 0x4B6310, which read the instance and its LOD and add it). For each remaining view N (0x280-byte records at `0x2612D18`, count `0x2526CCC`) with flag 0x100, call `0x4B6CC0(N)`. Flush A.
9. Dynamic water: `0xB779C0` ("Dynamic water", picks the water instances).
10. `0x4B7870`: time-of-day lights.
11. `city_batch_job_release` (0x4B6760): waits for the batch job object `0x2526CC8`, then releases it.
12. Profile "citybrb:5".
13. `city_submit_chunk_extras` (0x4B74D0): each chunk's extra instances go to pass 4.
14. `city_submit_visible_instances` (0x4B72E0): every visible instance goes to the render lists, in jobs of 75 (next section).
15. `0xFA6C24` = pass 0 count − `0xFA6C28`: how many pass 0 entries the city added.

After this, `game_render_thread_frame` runs:
- 0x9FA9E0, 0x533910 and 0x533A70 (wake the job workers), then 0x533CD0;
- the two instance passes: `city_instance_batch_begin(0x4B6550)` + 0x4B6E10, then `city_instance_batch_begin(0x4B6580)` + 0x4B7070, each followed by `city_instance_batch_flush` when `0xF83AF4` is non-zero;
- then `city_batch_job_release` again.

0x4B6550 / 0x4B6580 call 0x4B6160 / 0x4B6310 per entry: the same instance-add path as above, for the shadow and main instance lists.

**What each stage writes:**

| Stage | Writes |
|---|---|
| culling (5, 8) | the chunks' visibility bitmaps (+0x58), and the per-view instance lists through 0x4B6160 / 0x4B6310 (LOD pick, `rl_*` pre-render) |
| water (9) | the water instance list |
| lights (10) | `+0x2E6` on the light records, through `cmd_r_force_time_of_day_lights` / 0x52F910 |
| lists (11-14) | the global render lists (next section), which `render_frame`'s stages consume |

**Porting order (leaves first):**
1. `frustum_test_aabb` (pure maths).
2. The two accumulators (begin / flush) and the job object (can start synchronous: call the callback inline).
3. The per-entry functions 0x4B5C40, 0x4B5FF0, 0x4B6160, 0x4B6310.
4. `city_build_render_batches` steps 1-8.
5. Water (0xB779C0), lights (0x4B7870) and the list build (0x4B6760, 0x4B74D0, 0x4B72E0; next section).
6. `render_frame`'s stages (table above), which consume the lists through `rl_*::render` (`rl_mesh_inst::render` 0xCF8C90: `gfx_set_shader_state3`, `mesh_render_pass7`, `mesh_render_depth`, `mesh_render_generic`).

### Render lists and mesh drawing, for porting

The batch lists are fixed global arrays, not allocated per frame. Each holds 0x14-byte entries:

| Off | Field |
|---|---|
| +0 | instance (`rl_*_inst *`) |
| +4 | render flags (the flags record's first dword) |
| +8 | per-list argument (a bool, a byte mask or a u16) |
| +0xC | visibility dword, or 0 |

Each list keeps its count right after its last slot. An adder does `n = InterlockedIncrement(&count)`, writes slot `n − 1`, and clamps the count at the capacity, so the lists can be filled from the job workers.

| List | Adder | First entry | Count | Capacity | Consumers (render call) |
|---|---|---|---|---|---|
| pass 0 | `render_list0_add` 0x52FDA0 | 0x277D190 | 0x2784990 | 1536 | 0x5258B0 (`render(7, 0)`), 0x526C50 per view (`render(4, …)`), 0x524290, 0x526A90, 0x528820, `render_shadows` |
| pass 0, flag 0x40 | `render_list0b_add` 0x52FE10 | 0x2784998 | 0x2789998 | 1024 | 0x524290, 0x527020 |
| visibility u16 set | `render_list_vis_add` 0x52FE80 | 0x27899A0 | 0x278C1A0 | 512 | 0x524750 (`render(5, 0)`), 0x529AC0 |
| pass 1 | `render_list_small_add` 0x52FEF0 (`EDI` = list) | 0x278C1A8 | 0x278D5A8 | 256 | `render_stage2_a4(2, bit)`: entries whose +8 mask has the bit, `render(8, view)` |
| pass 2 | same | 0x278D5B0 | 0x278E9B0 | 256 | `render_stage2_a4(1)`: all entries |
| pass 3 | `render_list3_add` 0x52FF60 | 0x278E9B8 | 0x278EC38 | 32 | `render_env_cubemap` 0x532810 (`render(0, 0)` per cube face), `render_stage2_a4(0, bit)`, 0x525BB0 (`render(0, 0)`) |
| pass 4 | `render_list4_add` 0x52FFD0 | 0x278EC40 | 0x278ED80 | 16 | 0x529EB0 in render1 (`render(0xA, 0)`) |
| pass 5 | `render_list_small_add` | 0x278ED88 | 0x2790188 | 256 | 0x531840 (character poses) |
| pass 6 | `render_list6_add` 0x530030 | 0x2790190 | 0x2790D48 | 150 | 0x527A60 in render4 (`render(8, view 0x2612D10)`) |
| pass 7 | `render_list7_add` 0x5300A0 | 0x2790D50 | 0x2791840 | 140 | 0x5293D0 in render4 |
| all instances | `render_list_add` | 0x1406C30 (pointers) | 0x25273EC | 2048 | `game_render_do_pre_render_jobs` |

`render_lists_reset` (0x52E7C0) zeroes every count at the start of the frame, along with the frame allocator stats (`0xFA6C20`), `0xFA6C24 = 0` and `0xFA6C28 = −1`.

`render_stage2_a4` handles two flags:
- Entries with flag 0x10 are drawn with render states 7 and 14 off, then back on.
- Entries with flag 0x20 set state 24 to 0xA0 or 3 (stencil).

It picks the shader set from the flags with `(f >> 4 & 0x200 | f & 0x180) >> 7` (0x5230B0).

**From a world instance to a list entry:**
1. `city_submit_visible_instances` (0x4B72E0) walks each streamed chunk's instances:
   - It collects `{chunk, index}` for each instance whose visibility entry is set (chunk+0x58, 8 bytes per instance).
   - It groups them in 75s and runs `city_submit_job` (0x4B72A0) on a job for each group.
2. `city_instance_submit` (0x4B5510; cdecl `(chunk, index, bool alt, pass)`):
   - The instance record is at chunk+0x4C + index × 0x50:

     | Off | Field |
     |---|---|
     | +0 / +0x10 | AABB min / max |
     | +0xC | the object |
     | +0x1C | LOD distance |
     | +0x30 / +0x34 | flags |
     | +0x38 | frame stamp (set to `0x2526CD0`) |
     | +0x40 | mesh |
     | +0x48 | alternate mesh, used when flag 0x10000 is set |
   - It filters on:
     - the flag words (0xC1388020 / 0x22000 exclude);
     - the cutscene and LOD state;
     - the fade (+0x4C of the mesh).
   - Then it calls `world_mesh_submit`, or `world_mesh_submit_alt` when `alt` is set.
   - Two other paths use it:
     - `city_submit_chunk_extras` (0x4B74D0) submits each chunk's extra list (+0x20C count, +0x210 indices) as pass 4.
     - The per-view culls (`city_instances_cull_view` 0x4B6160 / `_2` 0x4B6310) submit passes 1 and 2 through `city_instance_add` (0x4B5A20).
3. `world_mesh_submit` (0x52F910; `EAX` = the flags record; cdecl `(pass, mesh, chunk, index, scale *)`):
   1. Allocate 0x350 bytes from the frame allocator `0x2791848` (vtable +0x1C). Failure sets `0x25273E6` (out of memory).
   2. Set the vtable to `rl_lod_mesh_inst` (0xDE2574) and +4 = 1 (type).
   3. `rl_renderable_init_transform` (0xCF62A0) fills:

      | Off | Field |
      |---|---|
      | +8 | position |
      | +0x14 | rotation (3×3) |
      | +0x38 | scale |
      | +0x44 | flags (bit 0 = hidden) |
      | +0x50 / +0x60 | scaled AABB |
      | +0x5C | a byte |
   4. Material and lighting setup: 0x530150 (the flags record's +4 / +8 / +0x248…), 0xD098B0.
   5. `render_list_add` (0x52E840; `ESI` = the flags record; cdecl `(pass, inst, vis *)`) checks the visibility entry and appends to a list:
      - the pass's own list; for pass 0, the 0b list when flag 0x40 is set;
      - also the visibility list when the entry's `vis+4` u16 is set;
      - with a group at flags+0x2B0, the first instance goes into the group.
   6. `world_decal_submit` (0x52E550): a 0x90-byte `rl_decal_inst` (type 5) through the same path, pass 0.

**`rl_mesh_inst::render`** (0xCF8C90; `ECX` = instance, cdecl `(pass, arg)` → bool):
1. Return 1 if hidden (+0x44 bit 0).
2. `mesh = vtable[+0x28]()`.
3. If the fade (+0x74) is below 1, `gfx_set_instance_fade(fade)` (0xD1A620).
4. `gfx_set_shader_state3`.
5. Build a 0x40-byte draw descriptor on the stack:

   | Off | From |
   |---|---|
   | +0 | &inst+8 (position) |
   | +4 | &inst+0x14 (rotation) |
   | +8 / +0xC | &inst+0x50 / +0x60 (AABB) |
   | +0x10 | mesh |
   | +0x14 | &inst+0x98 (cached transform-stack entry, 0x144 bytes, valid flag +0x140) |
   | +0x18 | &inst+0x1DC (cached camera distance²) |
   | +0x1C | byte flags from inst+0x95 (1 → 2, 2 → 4, 4 → 8) |
   | +0x1D | byte inst+0x5C |
   | +0x20 | *(inst+0x328) or 0 |
   | +0x24 | inst+0x78 |
   | +0x28..+0x30 | scale (inst+0x38..+0x40) |
   | +0x38 | byte inst+0x94 |
   | +0x3A | 1 when +0x3C is the fallback |
   | +0x3C | material set: inst+0x1E4, or *(inst+0x1E0), or mesh+0xC → +8 |
6. Pass 7 → `mesh_render_pass7` (0xD06940). Pass 3 → `mesh_render_depth` (0xD05130). Anything else → `mesh_render_generic(desc, pass, arg)` (0xD06430).
7. Restore the fade to 1.0.

**`mesh_render_generic`** (0xD06430), step by step:
1. **Transform.** If the cached entry is valid and `0x252A338` is set, `gfx_transform_push_cached` it and reuse the cached distance. Otherwise:
   - compute the squared distance from the camera (`gfx_get_thread_state()`) to the AABB (SSE);
   - `gfx_transform_push` from the descriptor;
   - copy the new top entry into the cache and mark it valid.
2. `mesh_vertex_cache_select(mesh)` (0xD04FC0).
3. **Index buffer:** mesh+8 → +0x10. If it differs from the cache `0x252A310`, write command 10 (SetIndices).
4. `gfx_set_material_constants(desc+4, arg, desc+0x28)` (0xD1B100): the world transform constants.
5. **For each submesh.** The count is the u16 at mesh+2, and each submesh is a 16-byte record from mesh+4:

   | Off | Field |
   |---|---|
   | +0 | vertex-buffer index |
   | +4 | start index |
   | +8 | base vertex |
   | +0xC | index count |
   | +0xE | material |

   For each one:
   1. The vertex buffer record is (mesh+8)+8 + vb × 0x10: +0 / +1 declaration indices, +2 u16 stride, +0xC buffer.
   2. `gfx_bind_vertex_stream`. If it returns 0, look up the declaration in `ctx+0x308[a × 5 + b]` and write command 5 if it changed (cache `0x252A30C`).
   3. Stream source: command 8 `{0, buffer, base × stride, stride}`, unless it matches the cache `0x2304D8C / 0x2304D6C / 0x2304D7C`.
   4. Material (`(mesh+0xC)+4 + material × 0x18`):
      - its shader id (optionally remapped by `0x2302D0C` when `0x252A339` is set) goes to `gfx_material_vs_constants` / `gfx_material_ps_constants`;
      - its sampler states (count +0xC, 6-byte `{stage, type, value}` at +0x14) go out as command 0x13, cached in `0x22F52F0[stage × 16 + type]` (stages below 20, types below 16).
   5. `gfx_bind_material_textures` (textures from the material set at desc+0x3C).
   6. `LOD = gfx_shader_pick_lod(distance)`.
   7. `gfx_bind_shaders(pass, 0, decl a, decl b)` with that LOD.
   8. If there are any primitives: `gfx_cmd_draw_indexed(5 = D3DPT_TRIANGLESTRIP, start index, index count − 2)` (0xD21650).
   9. `gfx_unbind_special_texture` for each special slot, then `gfx_sampler_states_pop`.
6. Pop the transform (`ts+0x1554 −= 1`).

**Porting order:**
1. The list adders and `render_lists_reset`: plain arrays with an atomic counter.
2. `render_list_add`.
3. `rl_renderable_init_transform`.
4. `world_mesh_submit` and `world_decal_submit`, on a Rust frame allocator.
5. `city_instance_submit` and its callers.
6. The descriptor build in `rl_mesh_inst::render`.
7. `mesh_render_generic`. It sits on functions that are already ported: `gfx_cmd_*`, `gfx_bind_shaders` and `gfx_set_material_constants`.
8. `mesh_render_depth` and `mesh_render_pass7`, which follow the same shape.

**Open:** passes 5 and 0xA (see the next section).

### Mesh-path leaves, for porting

Ring command numbers are from the table in 07_render.md:
- 3 / 4: pixel / vertex shader;
- 5: vertex declaration;
- 6 / 7: VS / PS constants `{reg, count, vec4s}`;
- 8: stream source;
- 10: indices;
- 14: texture `{stage, texture}`;
- 15: render state `{state, value}`;
- 0x13: sampler state `{stage, type, value}`.

`ctx` is the render context (`[fs:0x2C][0]+0x58`). `ts` is `gfx_get_thread_state()`.

**Render passes.** The `pass` argument of `render(pass, arg)` is not a list index. It is the technique column that `gfx_bind_shaders` looks up in each shader. Values and where they are used:

| Pass | Used by | What it is |
|---|---|---|
| 0 | `render_env_cubemap` (each face), 0x525BB0 at the end of render3 | plain lit draw (environment map and late world) |
| 3 | `rl_mesh_inst::render` → `mesh_render_depth` | shadow map depth |
| 4 | 0x526C50, once per extra view in render3 (0x528A10) | extra views (views 1.. at `0x2612D18`) |
| 5 | 0x524750 (visibility list) in render4b (0x527020) | *open* |
| 7 | 0x5258B0 in render2:b (0x528820) → `mesh_render_pass7` | depth only: `gfx_bind_shaders` sets `D3DRS_COLORWRITEENABLE` (0xA8) to 0. It fills the depth that render3 resolves |
| 8 | `render_stage2_a4` in render2:a3, 0x527A60 in render4 | main view: opaque world, then the pass 6 list (transparent) |
| 0xA | 0x529EB0 in render1 (pass 4 list) | *open*: drawn before the main passes |

So "opaque / transparent" is decided by which list an instance is in (2, 1 / 6), and "depth / shadow / main" by the pass.

**The transform stack** (what I called the light set before). Each thread state has a stack of 0x144-byte entries at `ts + 0x114 + level × 0x144`, with the depth at `ts+0x1554`:

| Off | Field |
|---|---|
| +0 | matrix 0 (4×4, from 0xBE62B0; probably world × view × projection) |
| +0x40 | matrix 1 (4×4; probably world × view) |
| +0x80 | world matrix: the 3×3 rotation rows with w = 0, then the position with w = 1 |
| +0xC0..+0x143 | the rest of the entry, copied as a whole. +0x140 is the cache-valid byte in an instance's copy |

- **`gfx_transform_push`** (0xD233B0; `EAX` = &rotation 3×3, `ECX` = &position, cdecl `(bool absolute)`):
  1. Depth += 1.
  2. The world matrix is the rotation and position, or, with a parent level and `absolute` = 0, combined with the parent (0xBE62B0).
  3. Matrices 1 and 0 are rebuilt from it.
  4. `gfx_transform_apply`.
- **`gfx_transform_push_cached`** (0xD23660; cdecl `(entry *)`): depth += 1, copy the 0x144-byte entry in, then `gfx_transform_apply`.
- **`gfx_transform_apply`** (0xD229B0): when deferring (`0xE849DB`) or on a context other than the main one, 0xD18EE0 uploads the matrices.
- The pop is inline: `ts+0x1554 −= 1` at the end of each `mesh_render_*`.

`rl_mesh_inst` keeps a copy of its entry at +0x98 and its camera distance² at +0x1DC. `mesh_render_generic` pushes that copy when it is valid and `0x252A338` allows the cache. Otherwise it pushes from the descriptor (`EAX` = desc+4, `ECX` = desc+0, absolute 0) and copies the new top (`ts + depth × 0x144 − 0x30`) into the instance.

**`gfx_set_material_constants`** (0xD1B100; `EAX` = &position; cdecl `(&rotation, light *, &scale)`). 0xBDE500 transforms each vector after it is built:

| Constant | Command | Value | Context copy |
|---|---|---|---|
| VS c10 | 6 | camera (`ts`+0) − position | ctx+0x650 |
| VS c19 | 6 | scale, w = 1 | ctx+0x6B0 |
| PS c18 | 7 | lighting camera (ctx+0x5C0) − position; w = the LOD distance ctx+0x6E4 | ctx+0x6D0 |
| VS c2 | 6 | when a light is given: light+0x10 − position, w = 1 | ctx+0x5D0 |
| VS c18 | 6 | when the light's type (+0) is 2: its direction (+0x1C), w = 0 | ctx+0x630 |

**`gfx_set_instance_fade`** (0xD1A620; cdecl `(f32 fade)`): if fade ≠ ctx+0x6E0, store it and write command 7 (PS c4) `{ctx+0x6E8 ? 0 : 1.0, ctx+0x6EC, 0, fade}`.

**`gfx_shader_pick_lod`** (0xD19D00; `EAX` = shader id, cdecl `(f32 distance²)` → LOD in `EAX`):
1. The shader record is `ctx+0x484 → [id × 0xC + 8]`.
2. With fewer than 2 LODs (+9 byte), return 0.
3. Otherwise add the squares of the thresholds at +0x30 one at a time. Return the first index where the sum exceeds the distance, or the last LOD.

**`gfx_bind_shaders`** (0xD1A860; `ECX` = shader id, `EAX` = LOD; cdecl `(pass, column, decl a, decl b)`):
1. Clamp the LOD to the shader's count − 1.
2. `technique = (s16) shader+0x2C → [pass × 8 + 4] → [column]`.
3. Store id, LOD, technique, a, b in ctx+0x46C..+0x47C.
4. The pair is `shader+0x28 → [LOD] + technique × 8`: {vertex shader, pixel shader}.
5. Vertex shader (0 → default `0xE84B98`): command 4 if it differs from the cache `0x22F2AEC`.
6. Pass 7: render state 0xA8 (color write) = 0 through the render-state cache `0x22F57F0[state]`.
7. Pixel shader (the default `0xE84B9C` when ctx+0x480 is clear or the pair has none): command 3 if it differs from `0x22F2AE8`.

**Material constants** (`EDX` = the material set desc+0x3C, `ECX` = material row; cdecl `(shader id)`):
- `gfx_material_vs_constants` (0xD01DA0) reads entry `set+0 → [row × 8]`: {size, float offset}.
- If the size is non-zero, it writes command 6 to register `shader+0x17`. The data is `size >> 2` vec4s from `set+0xC` + offset × 4.
- `gfx_material_ps_constants` (0xD01D20) does the same from `set+4` with command 7.

**`gfx_bind_material_textures`** (0xD049E0; `EAX` = row; cdecl `(&saved count, set, slot count)`). For each slot `v = set+0x10 → [row × 0x40 + i × 4]`:
- If `v ≥ −1`, it is a texture: `gfx_set_texture_cached`.
- Otherwise it is a special texture `id = v & 0x7FFFFFFF`:
  - For ids 1 and 0x1E, first build a 3×3 product (0xD01C30, as 3 vec4s) and send it as VS c15 and PS c15 (3 registers each).
  - Then `gfx_bind_special_texture` (0xD1ADC0; `EAX` = id, `ESI` = stage). Ids 1-4 read ctx+0x304 + id × 0x10, and ids 0x10-0x1E read ctx+0x254 + id × 0x10. The record is {texture, three sampler settings}.
  - It writes command 14 if the texture differs from the cache `0x22F2AF0[stage]`, and pushes the settings that aren't −1 (0xD17E20). Each push adds to the saved count.

After the draw, `gfx_unbind_special_texture` (0xD19FA0; `EAX` = id, `EDX` = stage) handles ids 1-4: it clears the bitmap cache `0xE84AA8[stage]` and resets sampler types 1 and 2 (address U / V) to 1 (wrap).

`gfx_sampler_states_pop` (0xD17D40; `EAX` = count) pops that many saved `{stage, type, old value}` entries:
- They live in ctx+4 (12 bytes each, count at ctx+0x304, at most 0x40).
- Each restore is a command 0x13 through the sampler cache `0x22F52F0[stage × 16 + type]`.

`mesh_render_generic` pushes one entry per material sampler state it changes and pops them all after each submesh.

**`mesh_vertex_cache_select`** (0xD04FC0; cdecl `(mesh)`) only runs when `0x252A378` is set and `0xE863BC` is clear:
1. Initialise once (0xBD9AC0).
2. Look up the mesh's vertex buffer (+8 → +8 → +0xC) in a cache (0xBD9C80), and create the entry if it is missing (0xBD99B0, 0xD00C30, 0xD04CD0).
3. If the entry is active (+0x108), call 0xD04CD0 and set `0x252A334` = entry. Otherwise set `0x252A334 = 0` and call 0xD01480.

*Its role (per-mesh vertex lighting or instancing data) is open.* With `0x252A378` = 0 it does nothing.

**Porting order:**
1. `gfx_shader_pick_lod`, `gfx_set_instance_fade`, `gfx_sampler_states_pop`, `gfx_unbind_special_texture` (no dependencies).
2. `gfx_bind_shaders`.
3. The two material-constant writers.
4. `gfx_bind_special_texture` and `gfx_bind_material_textures`.
5. `gfx_transform_push`, `_push_cached` and `_apply` (they need 0xBE62B0 / 0xD18EE0, the matrix maths and the upload).
6. `gfx_set_material_constants` (needs 0xBDE500).
7. `mesh_vertex_cache_select` can stay original: it is off unless `0x252A378` is set.

## render_frame (0x52A710), by profiler stage

| Stage | Calls | What |
|---|---|---|
| render1 | 0x529EB0, 0xD1C530 (screenshot), 0x728C40, **`render_stage1`** (0x52A4F0: `game_render_do_pre_render_jobs`, 0xA8FAD0, 0x4C7050, 0xA8D890, …) | set the render targets, viewport, scissor and clear; pre-render jobs |
| render-shadow-jobs | **`render_shadows`** (0x5285F0, profiler "shadows:1-6"), 0xD1CB00 (readback) | shadow map passes |
| render-flush | `gfx_command_buffer_wait` | wait for the device thread to drain the ring before the main passes |
| render2:a | 0xD00F20, 0x5380A0 (lispsm / uniform shadow setup), 0xD1A070, 0x9F0BA0 | |
| render2:a3 | **`render_stage2_a2_a3`** (0x533C00: 0x532810, 0x533110 → `render_stage2_a4`, …), 0x531840 (character poses), 0x9EF600, 0xB782B0, 0xD1AFA0, 0xD1A280, 0xB78150 / 0xB78190 | opaque world and characters |
| render2:b | 0x528820 | |
| render3 | 0xD14D20 (viewport), 0xD1A400 (screen size), 0x528A10 | depth resolve (`shaders\resolve_depth*.vso_pc / .pso_pc`) |
| render4 / 4a / 4b | 0xAE1090; 0x527A60; 0x518810, 0x527020, 0x625620, 0x738E80, 0x5293D0 | transparent, effects, post (stretch-rect copies) |
| render4d | 0xD14700, 0xA8B440, 0x4C7290, 0x4C32F0, 0x4CFC60, 0x8AB9E0 (draw-UP), 0xD1A4D0(0) | late effects, 2D |
| render5 | 0x51CF40, 0xD1E140, 0x517BB0, 0x518CA0, 0x517D40 | final composite and post-processing to the back buffer |
| | 0x5211F0 | end of frame bookkeeping |

The camera position used for lighting comes from the snapshot `0x277C9D0` (written by `frame_end_handoff`) into the render context +0x5C0, together with the LOD distance `0xE9896C` → +0x6E4.

## render_frame (0x52A710) porting map, with its shared-state reads

Render thread, no arguments; called from `game_render_thread_frame` when the world is visible (`[0xE9D68D]`). The frame is `sub esp, 0x24` with a security cookie; it saves `EBX` / `EBP` / `ESI` / `EDI`. Each stage is wrapped in `profile_start` / `profile_stop` with the names from the table above. `render_deferred_execute` (0xCF8FA0) is already in Rust; it runs after this function and only reads the deferred list (see "The deferred command lists").

### Steps

1. **Render targets.** Save `[0x22FDC9C]` / `[0x22FDCA0]` into `0x22FDCB0` / `0x22FDCB4`; `[0x22FDCAC]` = 0x22FD8E4. Then 0x529EB0.
2. **Shadow flag.** `BL` = (`[0x2526CCC]` ≠ 0) and bit 0x2000 of view 0's flags `[0x2612D18]`. It is used in steps 6 and 8.
3. **Screenshot.** If `[0x2975A5C]` & 2: `[0x2975914]` = 0xD1C530(cdecl ([0x2975940] + 1) % 3 + 0xC, 0x297591C, 0).
4. **render1:** 0x728C40; `render_stage1` (0x52A4F0).
5. **render-shadow-jobs:** `render_shadows` (0x5285F0), then 0xD1CB00(cdecl ([0x2975940] + 1) % 3 + 0xC).
6. **render-flush:** `gfx_command_buffer_wait`.
7. **render2:a:** 0xD00F20; `[0x25295F0]` += 1; 0x5380A0 **only if `BL`**; 0xD1A070(`ECX` = 0x25273C4); 0x9F0BA0.
8. **render2:a3:**
   1. `render_stage2_a2_a3`.
   2. If `[0x25273F3]`: 0x531840, then `[0x25273F2]` = 0. **The render thread writes a game-side byte here.** The game thread copies 0x25273F2 → 0x25273F3 in `frame_end_handoff`.
   3. 0x9EF600, 0xB782B0, 0xD1AFA0, 0xD1A280.
   4. **Lighting centre:** render context (`fs:[0x2C]` → [0] = `EBP`) +0x58 → ctx; ctx +0x5C0..+0x5C8 = `0x277C9D0..D8`; +0x5CC = 0.0; +0x6E4 = `[0xE9896C]`.
   5. If `[0x27948A8]`: 0xB78150(cdecl f32 `[0x27948B0]`). Then `tod_light_shader_constants` (0xB78190):
      - with `BL`: (`EAX` = 0xE9D690, `EDI` = 0xE9D6C0, cdecl 0xE9D69C, 0xE9D6B0), the snapshot sun;
      - otherwise: the defaults (`EAX` = 0xEC5ACC, `EDI` = 0x252A6F0, cdecl 0xEC5ACC, 0x252A6F0).
9. **render2:b:** `[0x252A338]` = 1; 0x528820; `[0x252A338]` = 0.
10. **render3:**
    1. The screen size comes from the context (`EBP` +0x5C byte, +0x60 pointer; when the byte is 0 and the pointer is 0xEC26A0, use 0xEC2610). Then `gfx_set_scissor_rect(cdecl 0, size[1])` and 0xD1A400(cdecl f32 size[0], f32 size[1]), both converted with `fild`; size = the two dwords at the chosen pointer.
    2. `[0x252A338]` = 1; 0x528A10 (depth resolve, extra views); `[0x252A338]` = 0.
11. **render4:** 0xAE1090. **render4a:** 0x527A60.
12. **render4b:** `[0xE98D74]` = `[0xE5C340]`. If `[0x25273E4]` and `[0xE94D37]`: 0x518810. Then `[0xE94D37]` = 1; 0x527020.
13. If `[0x27A3F18]` (tagging): 0x625620. Then 0x738E80, 0x5293D0.
14. **render4d:** `gfx_set_tint_color(ESI = 0xE9D670)`; 0xA8B440, 0x4C7290, 0x4C32F0, 0x4CFC60, 0x8AB9E0, 0xD1A4D0(cdecl 0).
15. **render5:**
    - If `[0x25273B4]` is set: 0x51CF40(cdecl 0), 0xD1E140; restore `0x22FDCB0 / B4` from `0x22FDCA4 / A8`.
    - Otherwise: build {`[0xFA2228]`, `[0xFA222C]`, 0} on the stack; 0x51CF40(cdecl 0), `0x517BB0(ESI = &it)`, 0xD1E140, 0x518CA0(cdecl &it); then the same restore.
    - Then `[0x22FDCAC]` = 0x22FDC1C; 0x517D40.
16. 0x5211F0(cdecl &profile block); check the cookie; return.

### What reads the snapshot (0xE9D620..0xE9D6D0)

Found by scanning render_frame and 197 callees, three calls deep.

| Snapshot field | Written by (game thread, doc 11) | Read in render_frame by |
|---|---|---|
| `0xE9D620..28` camera position | `render_snapshot_copy_view` | `render_stage1` (view setup through `gfx_view_set_world`), 0x536CB0 (under 0x5380A0, shadows), `render_stage2_a2_a3` / `render_env_cubemap`, `render_viewport_setup` (0x52A3B0, through 0x531840), 0x526070 / 0x526410 (under 0x528A10) |
| `0xE9D62C..4C` camera matrix (9 dwords) | the same | `render_stage1` (`gfx_view_set_world(&0xE9D62C, …)`), 0x536CB0, `render_viewport_setup`, 0x526070 / 0x526410 |
| `0xE9D650` FOV | the same (`camera_get_fov`) | `render_stage1`, `render_viewport_setup`, 0x526070 / 0x526410 |
| `0xE9D664..6C`, `0xE9D670..78` (tint), `0xE9D67C..84` | `frame_end_handoff` step 9 | 0x518CA0 (render5 post, all nine dwords); `gfx_set_tint_color` (render4d, 0xE9D670) |
| `0xE9D690` / `0xE9D69C` / `0xE9D6B0` / `0xE9D6C0` sun direction / colour | `sky_light_snapshot` | `tod_light_shader_constants`, only with the shadow flag `BL` |
| `0x277C9D0..D8` LOD / lighting centre | `frame_end_handoff` step 12 | render_frame itself → ctx +0x5C0 |

**Not snapshotted but read on the render thread.** Every one of these is written by the game thread. Today they are safe only because the game thread waits for the acknowledgement (doc 11, "Snapshot window"):
- `[0xE9896C]` (LOD distance);
- the view count `[0x2526CCC]` and view flags `[0x2612D18]` (written by `game_render_func`);
- `[0x25273F3]`, `[0x27948A8]` / `[0x27948B0]`, `[0x25273E4]`, `[0x27A3F18]`, `[0x25273B4]`;
- the screenshot ring `[0x2975A5C]` / `[0x2975940]`.

**Render-thread-only state** (no sharing):
- the scratch allocator `0x2774180` (base) / `0x2774184` (end) / `0x2774188` (size) / `0x277418C` (cursor, `InterlockedExchangeAdd` in 0x528210, reset in `render_stage1`);
- the profiler blocks `0x277C9DC` / `0x277D178` (`render_shadows`);
- `0x27741D0` (0xA8EAB0);
- the flags `0x252A338`, `0xE94D37`, `0xE98D74`;
- the render-target save slots `0x22FDCA4..0x22FDCB4`.

**Port order.** Keep render_frame a call skeleton with the 16 steps above. Port the leaves stage by stage, render5 and render4d first: they are short and read only the tint fields. When the stage functions are ported, give them the snapshot as a struct pointer, not the raw globals. That is the one place where a later double-buffered snapshot would plug in.

## Mesh submission

Renderable instances are `rl_*` / `r_*` classes (for example `rl_mesh_inst`, `r_dynamic_water_detailed_inst`, `r_dynamic_license_plate_inst`). Their `render(pass, …)` virtual writes the commands.

**`rl_mesh_inst::render`** (0xCF8C90):
1. Skip it when hidden.
2. Get the mesh (vtable +0x28) and set the alpha / fade (0xD1A620) when below 1.0.
3. Fill a draw description: transform +0x14, bounds, the material +0x328, flags +0x95 → 2/4/8, and lighting.
4. By pass:

   | Pass | Function |
   |---|---|
   | 7 | `mesh_render_pass7` (0xD06940) |
   | 3 (shadow / depth) | `mesh_render_depth` (0xD05130) |
   | others | `mesh_render_generic` (0xD06430) |

Those functions bind the material and draw. The binding helpers each write specific commands:

| Helper | Commands written | Role |
|---|---|---|
| 0xD1A860 | 3, 4, 15 | **bind shaders** (pixel / vertex) and their render states |
| 0xD1B100, 0xD049E0 | 6, 7 | shader constants (material and lighting) |
| 0xD01D20 / 0xD01DA0 / 0xD1A620 / 0xD1A6D0 | 7 / 6 / 7 / 7 | per-object constants (transform, fade) |
| 0xD17D40, 0xD17E70, 0xD19FA0 | 15, 19 | render states and sampler states |
| 0xD01E60 | 5, 8 | vertex declaration and stream source |
| 0xD04FC0, 0xD19D00, 0xD1C2F0 | | texture binding (cache) |

Then `gfx_cmd_set_stream_source` and `gfx_cmd_draw_indexed`. `gfx_cmd_draw_indexed` is written only by those three mesh functions, 0xCF32D0, 0xCFD340 / 0xCFD7B0, 0xD04AE0, the water, license plate and 0x532D10 / 0xA8FE00 / 0xA8FF20 paths. `gfx_cmd_draw` is only used by simple water. `gfx_cmd_draw_up` is used by the 2D / debug primitives (0xD1CCF0 … 0xD1E600).

**Shaders**:
- Precompiled `.vso_pc` / `.pso_pc` files, created on the calling thread (`CreateVertexShader` / `CreatePixelShader`: 0xCF4C10, 0xD19120, 0xD1B5B0; 07_render.md).
- Only one debug path compiles HLSL at run time (0x517D40, `D3DXCompileShaderFromFileA`, vs_3_0 / ps_3_0).
- The full-screen post passes bind their shaders through 0x51CA60 (commands 3, 4, 14, 16, 17).

## game_build_render_batches and game_render_func (porting maps)

Both run on the game thread inside `frame_end_handoff`, at the end of its world-visible branch. Their order there: `render_lists_reset`, then `game_render_func`, then `game_build_render_batches` (doc 11).

### `game_build_render_batches` (0x522FC0)

No arguments. It saves `EDI`, and the visible path ends with a **tail jump**.

1. view = TLS render context (`fs:[0x2C]` → [0]) +0x58 → +0x740; then 0x4B7700(cdecl view).
2. 0x530860, 0x533630, `game_build_render_batch_func` (0x522E00).
3. **Only when the snapshot is not yet valid** (`[0xE9D68C]` == 0) and the world is visible (`[0xE9D68D]`): `render_viewport_setup`, `city_build_render_batches`, `level_objects_render_collect(EDI = 0xE9D620)`.
   - In the normal frame, `frame_end_handoff` has already set `0xE9D68C` = 1 before this call. So this path is skipped and the render thread does the same work (doc 11, render loop step 15).
   - The two threads share these calls; never run them on both.
4. 0x522F10.
5. Under the same condition as step 3: 0x533910, 0x533A70, 0x533CD0, then `pop edi` / `jmp 0x4B7EF0`. Otherwise `pop edi` / `ret`.

**Port:** a call sequence with two snapshot checks. With the snapshot always valid in gameplay, only steps 1, 2 and 4 run.

### `game_render_func` (0x5250E0)

No arguments. Frame: `sub esp, 0x7C`; saves `EBX` / `EBP` / `ESI` / `EDI`. `EBX` = 0 throughout; `EBP` = the `GetCurrentThreadId` import.

1. **Job.** If `[0x252A3E8]` (worker jobs on): `[0xFA6BD8]` = `job_alloc(cdecl 0x7AE, 0xDE2374)`.
2. **First job.**
   1. Build a job descriptor on the stack: `[esp+0x28]` = 0, `[esp+0x40]` = 0, then (after the 3 pushes) the function 0x5250C0, kind 6, the thread id.
   2. Call 0x5250C0 directly with (&descriptor, two stack words). It is `jmp 0x511050`.
   3. If the job exists: `job->vtbl[+4]()` then `job->vtbl[+0xC]()` (arm and signal).
3. **Sky, dynamic lights, constants**, in order: 0x4D7160 (the sky), `[0x252A364]` = 0 (the light count), then the **dynamic light** collectors 0xAE0FD0 (vehicle headlights), 0x505350, 0x94DFA0, 0x4C85D0, 0x9556A0, 0x963750, each adding lights through the `render_light_add_*` helpers; then 0x524060 (sets the shader constants). (Corrected: these are not render-instance fills.)
4. `render_viewport_setup`; copy 0x84 dwords of the thread view (context +0x58 → +0x740) to `0x277CBF0`.
5. **Co-op second view.** When `0x2528B48` and `0x2528B49` are both 1, the split mode `0x21603CC` == 2, and a partner exists:
   1. 0x9DFA00(`ECX` = partner, `EAX` = &local, cdecl &local2).
   2. Two floats: the partner's +0x12D0 / +0x12D4, or, when the partner is the local player, the thread view's +0x1710 / +0x1714.
   3. 0x523EB0(cdecl &a, &b, f32, f32). The floats are written after `sub esp, 8`.
6. `[0xFA6BDC]` = `[0xE98980]` if `0x2527393`, else 1.0; `[0x2526CCC]` = 0.
7. 0x523BD0(cdecl 0): the main pass; it can raise `0x2526CCC`.
8. 0x4B3FF0(`EAX` = 0x25F5B14 camera position, cdecl 1).
9. **Occlusion flag** `[0xE99382]`:
   - 0 if `0xE996E4` is 0 or `0x25273B4` is set;
   - else `0x2526CC4` when `0x2526CC3` == 1;
   - else 0x4B4A60().
10. If not `0x25273B4` and render mode `0xE98948` is 1 or 2: 0x4BA270(`ECX` = 0x25F5B38 camera matrix, cdecl &0x25F5B14).
11. **Second job.**
    - If `[0xE99382]`: a descriptor for 0x5250D0 with 0x2612D10. 0x5250D0 is cdecl (descriptor, arg) and calls 0x523BD0(arg), so this pass runs with the occlusion data. Call it directly, then arm and signal the job as in step 2.
    - Else: `[0x252739C]` = 0.
12. `0x2526CD0` += 1 (word); `[0x25273A4]` = 1.0; copy 0x81 dwords `0x26128F8` → `0x2612B00` (the time-of-day snapshot).
13. If `[0x2526CCC]` > 0 and `[0x2612D18]` & 0x2000: 0xA02550(`EAX` = 0x277C9E0, `ECX` = 0x25F5B14, cdecl (0x2615510, 0x2612D20, f32 `[0x2612D58]`)).
14. **Job wait.** If jobs are on and `[0xFA6BD8]`:
    1. Set +0x34 = 0x33D6328 and call vtbl +0x10 (wait).
    2. Clear +4, +8, +0x34, +9, +0x3C, +0x40, +0x44.
    3. `pool 0xEC5B98 ->vtbl[+4](job − 8)`.

    This is the same pattern as the audio job in `frame_end_handoff`.

**Quirks:**
- Steps 2 and 11 call the job function directly on the game thread, then arm and signal the job object. The job object is a completion fence, not the place the work runs.
- 0x5250C0 receives a descriptor whose own function field is 0x5250C0.

**The job descriptor (queue item 32).** It is a 32-byte stack struct, built twice. `E` is `ESP` after the prologue (`sub esp, 0x7C` and 4 pushes).

| Off | First job (at E+0x24) | Second job (at E+0x44) | Meaning |
|---|---|---|---|
| +0x00 | 0x5250C0 | 0x5250D0 | the function |
| +0x04 | 0, then the job object `[0xFA6BD8]` after the call (only when one exists) | same | the job |
| +0x08 | **not written** (stale stack) | 0x2612D10 | argument 1 |
| +0x0C | not written | not written | argument 2 |
| +0x10 | 6 | 6 | the kind |
| +0x14 | not written | not written | — |
| +0x18 | `GetCurrentThreadId()` (`call [0xDB8080]`, kept in `EBP`) | same | the creating thread |
| +0x1C | 0 | 0 | — |

The order of the writes:
1. +0x04 and +0x1C are zeroed **before** `GetCurrentThreadId`;
2. then +0x18 is written;
3. then the call's three cdecl arguments are pushed: (&descriptor, +0x08, +0x0C), with +0x08 / +0x0C pushed as they are;
4. +0x00 and +0x10 (and +0x08 for the second job) are stored **after** the pushes, through `ESP`-relative addresses. The second job's pushed argument 2 is the constant 0x2612D10, so it is right even though the stored +0x08 comes later.

The descriptor is **dead data**:
- 0x5250C0 is `jmp 0x511050`, and 0x511050 reads none of its stack arguments: it works on globals (`[0xF9F694]`, `[0xFA20A0]`).
- 0x5250D0 reads only its second argument and calls `0x523BD0(cdecl 0x2612D10)`.
- The job object is then driven with `ECX` = job only: vtbl +4 (kick), then vtbl +0xC (sync), as `job_alloc` documents. The descriptor is not passed.
- Nothing reads +0x04 after it is written.

The struct is what is left of an inlined "submit job" helper that the compiler reduced to a direct call.

**Port:**
- No struct is needed. Call 0x511050 (or the port of it), then 0x523BD0(0x2612D10) when `[0xE99382]` is set.
- Keep the two kick / sync pairs on `[0xFA6BD8]` and the step 14 wait. The render thread is synchronised through them.
- If you call the originals, any 32 bytes of stack satisfy them.

**Port:** keep the call order and the job fence. 0x523BD0 is the heavy part (the main and occlusion passes), so port this function as a skeleton calling originals.

The roles of 0x523BD0, 0x4B3FF0, 0x4BA270 and 0xA02550 are not identified yet; which light source each collector in step 3 handles (other than the headlights) is not checked.

## Open

- The deferred types marked "role open" in the handler table.
- What render passes 7 and the generic pass are (forward lighting variants).
- The shadow technique: "lispsm" / "uniform" strings in 0x5380A0, and shadow-map sizes.
