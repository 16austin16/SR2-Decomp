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

## The deferred command lists (0x2305978 / 0x230597C)

These are **not mesh lists**. They are a double-buffered queue of *render-state commands* that the game thread records and the render thread executes. `frame_end_handoff` swaps the two pointers.

| Off | Field |
|---|---|
| +0x0000 | 3000 records of 8 bytes: {u32 type, u32 offset into the data area} |
| +0x5DC0 | record count |
| +0x5DC4 | data area (0x16800 bytes) |
| +0x1C5C4 | data used (0x16801 marks overflow) |

- **`render_deferred_queue`** (0xCF8EA0, (type, args…)) writes into the *game-side* list `0x230597C`. It calls `handler[type](0 = record, &dst, &args)`, which serialises the arguments into the data area.
- **`render_deferred_execute`** (0xCF8FA0) runs on the render thread after `render_frame`. It walks the *render-side* list `0x2305978` and calls `handler[type](1 = execute, &data, 0)`, then invalidates the 16 cached texture stages (`0xE84AA8…`).
- The handler table is `0xE847E0`: 24 types, functions 0xCF9xxx / 0xCFAxxx / 0xCFBxxx.

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

- The meaning of each deferred handler type (0xE847E0).
- What render passes 7 and the generic pass are (forward lighting variants).
- The shadow technique: "lispsm" / "uniform" strings in 0x5380A0, and shadow-map sizes.
