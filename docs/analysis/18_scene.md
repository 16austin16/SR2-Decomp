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
| 07 | 0xCF94A0 | `gfx_set_ps_params3` 0xD14700 | ESI = &{3 dwords} | pixel shader constants (ring 7) via 0xD18DE0, then 0xD1D840 |
| 08 / 09 | 0xCFB100 / 0xCFB0F0 | `gfx_push_depth_bias` 0xD232C0 / `gfx_pop_depth_bias` 0xD23200 | cdecl (float), pops 4 / - | 16-deep stack; `D3DRS_DEPTHBIAS` (0xC3) = sum |
| 0A / 0B | 0xCFB080 / 0xCFB070 | `gfx_push_slope_depth_bias` 0xD23120 / `gfx_pop_slope_depth_bias` 0xD23060 | cdecl (float) / - | `D3DRS_SLOPESCALEDEPTHBIAS` (0xAF) |
| 0C | 0xCF9350 | `gfx_set_clip_plane` 0xD22090 | cdecl 4 floats, pops 16 | 0xD18BD0: ring 7 plus `CLIPPLANEENABLE` (0x98) |
| 0D / 0E | 0xCFB000 / 0xCFAF90 | `gfx_set_state_float_110` 0xD23000 / `gfx_set_state_float_0e` 0xD22F90 | cdecl (float) | thread state floats (role open) |
| 0F | 0xCF92B0 | `gfx_set_shader_state3` 0xD09E20 | EAX = &{3 dwords} | 0xD18E50 (ring 7) and 0xD1CCF0 (render states + vertex declaration) |
| 10 | 0xCF91E0 | `gfx_clear_region` 0xD14B20 | registers + cdecl 2, pops 8 | `gfx_cmd_clear`, re-applies the state block |
| 11 | 0xCF9190 | `gfx_clear` 0xD14AD0 | ECX = value | `gfx_cmd_clear` |
| 12 | 0xCFB740 | `gfx_draw_bitmap` 0xD11240 | EAX + cdecl 10, pops 0x2C | bitmap, state block, 0xD1F0D0 (2D quad) |
| 13 / 15 | 0xCFAD50 / 0xCFA8C0 | `gfx_draw_text_13` 0xD15C60 / `gfx_draw_text_15` 0xCF5D90 | ESI, EDI + cdecl (3 floats, 1), pops 16; the record also holds the string | font layer 0xBD68E0 / 0xBDB5C0 |
| 14 / 16 | 0xCFAB00 / 0xCFA670 | `gfx_draw_text_14` 0xD15BD0 | ESI, EAX + cdecl 4, pops 16 | font layer 0xBD6E60 |
| 17 | 0xCFA4E0 | `im2d_draw_17` 0xD0B020 | ECX + cdecl 4 floats, pops 16 | 2D shape (role open) |
| 18 / 19 / 1A | 0xCFA450 / 0xCFA340 / 0xCFA330 | `im2d_c_begin` 0xD0A580 / `_add` 0xD0A4F0 / `_end` 0xD0A4B0 | ESI = &{state pair} / ESI = count + cdecl (data) / - | batch at 0x22F6430 with its own vertex declaration |
| 1B / 1C / 1D | 0xCFA2A0 / 0xCFA190 / 0xCFA180 | `im2d_strip_begin` 0xD0A440 / `_add` 0xD0A380 / `_end` 0xD0A330 | ECX = &{state pair} / ESI = count ≤ 64 + cdecl (vertices) / - | strip; end = `gfx_2d_flush(TRIANGLESTRIP, n - 2)` |
| 1E / 1F / 20 | 0xCFA0F0 / 0xCF9FE0 / 0xCF9FD0 | `im2d_list_begin` 0xD0A2C0 / `_add` 0xD0A1F0 / `_end` 0xD0A190 | same | list; end = `gfx_2d_flush(TRIANGLELIST, n / 3)` |
| 21 | 0xCF9E60 | `im2d_draw_quad` 0xD0A130 | ESI, EDI + cdecl (ptr), pops 4 | state block, 0xD1E330 |
| 22 / 23 / 24 | 0xCF9D00 / 0xCF9AD0 / 0xCF9910 | `im2d_draw_22` 0xD0B980 / `_23` 0xD0B7D0 / `_24` 0xD0A5E0 | cdecl 5 / EDI + 6 / 4 floats + 1 | 2D shapes over the strip / list batches (role open) |
| 25 | 0xCFBA10 | `gfx_draw_25` 0xD13A60 | EAX + cdecl (float, ptr), pops 8 | role open |
| 26 / 27 | 0xCF9810 / 0xCF9800 | `gfx_restore_default_targets_clear` 0xD1EF80 / `gfx_restore_default_targets` 0xD1EF40 | cdecl (3 bools) / - | default render target, depth target and viewport (0x22F63F8 / 0x22F63FC), optional clear |
| 28 | 0xCFB240 | queued by `gfx_queue_28` 0xD09D90 (16 dwords from 0x25282F4…) | - | calls 0x7715E0 with 8 floats (the pause map) |
| 29 | 0xCFB450 | `gfx_draw_29` 0xD15E80 | EDI, EAX + cdecl 6, pops 0x18 | role open |
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
0xD19070), `gfx_set_ps_params3`, the clip plane, text and the queue / handlers / execute.

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

## Open

- The deferred types marked "role open" in the handler table.
- What render passes 7 and the generic pass are (forward lighting variants).
- The shadow technique: "lispsm" / "uniform" strings in 0x5380A0, and shadow-map sizes.
