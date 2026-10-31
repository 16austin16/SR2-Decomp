# 07: Renderer threads, the D3D command buffer and the device wrapper

The seam for a Rust renderer. Names are in `docs/functions.csv` (subsystem `render`).

## Threads

The game never calls Direct3D to draw from its game or render threads. It writes **D3D commands** into a buffer (the "ring"), and the thread that owns the window and the device executes them.

| Thread | Entry | What it does |
|---|---|---|
| WinMain (device thread) | `WinMain` 0x520BA0 | Stores its thread id in `0xFA2394`, then starts the supervisor thread. Its loop (0x521000) does: on the init request flag `0x2527323`, create the window, `gfx_init_request` (0xD151A0) → `gfx_create_device`. Unless `0x252734A` is set, run `gfx_execute_commands`. Then `pump_window_messages` and `Sleep(0)`. |
| supervisor thread | `app_supervisor_thread_proc` 0x520A30 | First `startup_loading_render_loop` (0x520630), then the `game_render_thread_frame` loop (0x522640): waits for the sim, builds render batches, runs `render_frame` (0x52A710) |
| game thread | `game_main_thread_proc` 0x520790 | `game_frame_tick` (0x520450): sim, UI (vint draws through the 2D layer), and the end of the frame |

## The ring

A struct at `0x33D62EC`. The buffer is 16 MB and linear: it is rewound, not wrapped.

| Address | Off | Field |
|---|---|---|
| `0x33D62EC` | +0x00 | base |
| `0x33D62F4` | +0x08 | **published end**: the reader stops here. Writers advance it with `InterlockedExchangeAdd` |
| `0x33D62F8` | +0x0C | write pointer. Writers store dwords here with no bounds check |
| `0x33D62FC` | +0x10 | write position at the last publish |
| `0x33D6300` | +0x14 | read pointer |

**Writing.** A command is a u32 id followed by its arguments, packed as dwords (or inline blobs). The `gfx_cmd_*` writers (0xD1xxxx / 0xD21xxx) store at the write pointer and advance it. Some writers skip redundant state: `gfx_cmd_set_texture` keeps `0x22F2AF0[stage]`, and `gfx_cmd_reset_texture_cache` (0xD21940) clears it each frame.

**Publishing** (`gfx_end_frame` 0xD211F0, at the end of every frame):
1. Write command 1 (EndScene) and command 20 (Present).
2. `InterlockedExchangeAdd(0x33D62F4, write - last_published)`, then `last_published = write`.
3. If `write - base > 16 MB`, run `gfx_command_buffer_wait` (0xD1C780):
   1. `Sleep(1)` until `read >= published end`.
   2. `InterlockedExchange` published end to base and the read pointer to base, and set write and last-published to base.
4. If the current thread is the device thread (`GetCurrentThreadId() == 0xFA2394`), run the executor inline. This happens during startup.
5. Open the next frame: write command 0 (BeginScene) and reset the texture cache. Then write 16 × command 14 (SetTexture(stage, NULL)) for stages 0-15 and publish again.

`gfx_command_buffer_wait` is also called directly by the loading loops, `render_frame` and 0xD1E140.

**Reading** (`gfx_execute_commands` 0xD20360, device thread only):
1. Add the time since the last run to the idle counter (`QueryPerformanceCounter`; frequency in `0x34A5120`, idle in `0x34A5108`).
2. While `gfx_ring_has_data` (0xCF5940: read < published end):
   1. Pop the u32 id.
   2. Dispatch through the jump table `0xD21164`.
3. Stamp the end time (`0x34A5110`).

Readers: `gfx_ring_read_u32` (several identical template copies 0xBD5550-0xBD56C0, one per argument type; 0xBD55B0 reads a float), `gfx_ring_read_16` (0xBD5590 / 0xBD5650, a RECT), `gfx_ring_read_24` (0xBD5670, a D3DVIEWPORT9) and `gfx_ring_skip` (0xCF59A0, returns the old read pointer). Several cases read straight from `[0x33D6300]` and then advance it.

The executor stores each HRESULT in a local and ignores it.

## The 32 commands

The arguments are in ring order, after the id. "dev->X" is a call on `0x252A2D0`.

| Id | Ring arguments (dwords unless noted) | Executor |
|---|---|---|
| 0 | - | `dev->BeginScene()` |
| 1 | - | If `0x252734B` (FPS overlay): `D3DXCreateFontA(16, 0, 400, 0, 1, …, "verdana")` once into `0x252A2C8`, then draws "FPS : %+5.2f (%+5.1fms)" and "Idle: …" in yellow (0xFF00FFFF). Then `dev->EndScene()`. Under the critical section `0x2301CCC`, Release (vtable +8) every object in the deferred-release list `0x252A2BC` and clear it; then 0xBD4AD0 if `0x2301CC8`; zero `0x252A2C0/C4`. |
| 2 | count, count × RECT (16 bytes), flags, color, z (float), stencil | `dev->Clear(count, count ? rects : NULL, flags, color, z, stencil)` |
| 3 | shader | `dev->SetPixelShader(shader)` |
| 4 | shader | `dev->SetVertexShader(shader)` |
| 5 | decl | `dev->SetVertexDeclaration(decl)` |
| 6 | reg, count, count × 16 bytes | `dev->SetVertexShaderConstantF(reg, data, count)` |
| 7 | reg, count, count × 16 bytes | `dev->SetPixelShaderConstantF(reg, data, count)` |
| 8 | stream, vb, offset, stride | `dev->SetStreamSource(stream, vb, offset, stride)`; caches vb in `0x22F63F4` |
| 9 | value, u16 (as a dword) | **no-op on PC**: both are read and dropped. Writer 0xD214E0 |
| 10 | ib | `dev->SetIndices(ib)`; caches it in `0x22F63F0` |
| 11 | type, start, count | `dev->DrawPrimitive(type, start, count)` |
| 12 | type, base vertex, min index, num vertices, start index, prim count | `dev->DrawIndexedPrimitive(…)` (6 dwords) |
| 13 | type, prim count, stride, vertex data | `dev->DrawPrimitiveUP(type, count, data, stride)`. The data is `gfx_prim_vertex_count(type, count) × stride` bytes. 0xCF5900 gives points n, lines 2n, line strip n+1, triangles 3n, strip / fan n+2. |
| 14 | stage, texture | `dev->SetTexture(stage, texture)`; caches it in `0x22F5BF0[stage]` |
| 15 | state, value | `dev->SetRenderState(state, value)` |
| 16 | index, surface | `dev->SetRenderTarget(index, surface)` |
| 17 | surface | `dev->SetDepthStencilSurface(surface)` |
| 18 | src, src RECT (16 bytes), dst, dst RECT (16 bytes) | `dev->StretchRect(src, &r, dst, &r, D3DTEXF_NONE)` |
| 19 | sampler, type, value | `dev->SetSamplerState(sampler, type, value)` |
| 20 | - | `dev->Present(NULL, NULL, NULL, NULL)`, then `gfx_check_device` (0xD20230), then the **frame limiter** (below) |
| 21 | RECT (16 bytes) | `dev->SetRenderState(D3DRS_SCISSORTESTENABLE, 1)`, then `dev->SetScissorRect(&r)` |
| 22 | (dword, ignored), size, slot, size bytes | **Float data texture upload.** If `0x22FFCC8[slot]` is empty, create a 512×1 `D3DFMT_A32B32G32R32F` texture with 0xD195E0. Then LockRect(0), memcpy size bytes, UnlockRect(0). |
| 23 | stage, slot | `dev->SetTexture(D3DVERTEXTEXTURESAMPLER0 + stage, 0x22FFCC8[slot])`, vertex texture fetch |
| 24 | D3DVIEWPORT9 (24 bytes) | `dev->SetViewport(&vp)` |
| 25 | query | `query->Issue(D3DISSUE_BEGIN)` |
| 26 | query | `query->Issue(D3DISSUE_END)` |
| 27, 28 | - | unused (the case is an int3) |
| 29 | render target, dst surface | `dev->GetRenderTargetData(rt, dst)` |
| 30 | src, dst | `dev->UpdateSurface(src, NULL, dst, NULL)` |
| 31 | arg | 0xD1C530(arg, 0, 0): screenshot / readback |

**Device vtable slots, as called by each case.** Each case loads the method from `[[0x252A2D0]] + off` and calls it stdcall with the device first. The arguments are in the order the table above lists them, so the callee pops. The slots below were read from the executor and match `d3d9.h`.

| Id | Method | Offset | Slot |
|---|---|---|---|
| 0 | BeginScene | +0xA4 | 41 |
| 1 | EndScene (after ID3DXFont::DrawTextA, slot 14 of the font, for the FPS overlay) | +0xA8 | 42 |
| 2 | Clear | +0xAC | 43 |
| 3 | SetPixelShader | +0x1AC | 107 |
| 4 | SetVertexShader | +0x170 | 92 |
| 5 | SetVertexDeclaration | +0x15C | 87 |
| 6 | SetVertexShaderConstantF | +0x178 | 94 |
| 7 | SetPixelShaderConstantF | +0x1B4 | 109 |
| 8 | SetStreamSource | +0x190 | 100 |
| 10 | SetIndices | +0x1A0 | 104 |
| 11 | DrawPrimitive | +0x144 | 81 |
| 12 | DrawIndexedPrimitive | +0x148 | 82 |
| 13 | DrawPrimitiveUP | +0x14C | 83 |
| 14, 23 | SetTexture | +0x104 | 65 |
| 15 | SetRenderState | +0xE4 | 57 |
| 16 | SetRenderTarget | +0x94 | 37 |
| 17 | SetDepthStencilSurface | +0x9C | 39 |
| 18 | StretchRect | +0x88 | 34 |
| 19 | SetSamplerState | +0x114 | 69 |
| 20 | Present | +0x44 | 17 |
| 21 | SetRenderState, then SetScissorRect | +0xE4, +0x12C | 57, 75 |
| 22 | IDirect3DTexture9::LockRect / UnlockRect | +0x4C / +0x50 | 19 / 20 |
| 24 | SetViewport | +0xBC | 47 |
| 25, 26 | IDirect3DQuery9::Issue | +0x18 | 6 |
| 29 | GetRenderTargetData | +0x80 | 32 |
| 30 | UpdateSurface | +0x78 | 30 |

The ring readers take `EAX = &ring (0x33D62EC)` and push the output pointer. The render-side layer that writes these commands (the deferred lists, state caches and 2D batches) is in 18_scene.md, "The deferred command lists, step by step, for porting".

The executor keeps no state of its own besides the vb, ib and texture caches above, the FPS font and the timers.

### Frame limiter (inside command 20)

The time between Presents gives fps.
- fps > 35: `Sleep(clamp((1/35 - 1/fps) × 1000 / 2, 0, 100) ms)`. Fast frames sleep for half the time left to a 35 fps frame.
- 10 ≤ fps ≤ 35: `Sleep(0)`.
- fps < 10: `Sleep(30)`.

This is the PC build's known frame pacing behaviour. A port can drop it or replace it.

## Device creation, loss and reset

**`gfx_create_device`** (0xD1F3F0, on the device thread):
1. `InitializeCriticalSection(0x2301CCC)`.
2. `gfx_d3d9_create` (0xC29D70) → `0x252A2CC`.
3. Fill D3DPRESENT_PARAMETERS `0x22F63F8`:

   | Field | Value |
   |---|---|
   | width, height | from the settings |
   | BackBufferFormat | `D3DFMT_A8R8G8B8` (0x15) |
   | BackBufferCount | 1 |
   | MultiSampleType | 2 or 4 by the setting |
   | SwapEffect | DISCARD |
   | hDeviceWindow | `0x252A410` |
   | Windowed | `0x22F6418` |
   | EnableAutoDepthStencil | 0 |
   | PresentationInterval (`0x22F642C`) | 1 with vsync, else IMMEDIATE |

4. Check the caps (at least 512 in the two caps fields it reads).
5. Pick an adapter named "PerfHUD" with device type REF, if there is one.
6. `CreateDevice(adapter, type, hwnd, MULTITHREADED | PUREDEVICE | HARDWARE_VERTEXPROCESSING (0x54), &pp, &0x252A2D0)`. On failure it retries at 1024×768 windowed.
7. Copy the device to `0x252A2D4` (the real device) and `0xEC24D0` (inside the null device).
8. 0xD19230 and 0xD1E9A0 (resource setup; 0xD1E9A0 also calls 0xD219A0).
9. `CheckDeviceFormat(…, D3DUSAGE_QUERY_VERTEXTEXTURE, A32B32G32R32F)`.
10. Clear the slot tables `0x22FFCC8` / `0x22FDCC8` (0x800 entries each).
11. Write SetRenderTarget / SetDepthStencil commands for the main surfaces, then command 0, and publish.

**Device pointer swap.** `0x252A2D0` is the device every command uses. When the device is lost it points at **`IDirect3DDevice9Lost`** (`0xEC24CC`), a null object whose methods do nothing. The commands then run harmlessly; the real device is in `0x252A2D4`.

**`gfx_check_device`** (0xD20230, after every Present):
1. Call `TestCooperativeLevel`.
   - **DEVICELOST** (0x88760868): set `0x252A17F` and point `0x252A2D0` at the null device.
   - **DEVICENOTRESET** (0x88760869): tail-call `gfx_reset_device`.
   - **OK**: if `0x252A17F` is set, clear it and restore the real device.
2. After a reset (`0x252A262` set), count frames in `0x252A2B8`. Past 3:
   1. Restore the real device.
   2. `Sleep(200)`.
   3. `gfx_restore_after_reset` (0xD219A0).
3. If a display mode change is pending (`0x22FD848`), call `gfx_set_display_mode` (0xD1FF60) with (width, height, fullscreen, vsync) from `0x22FD84C..`. On failure, put the pending values back to the current ones and call 0x773DB0.

**`gfx_reset_device`** (0xD1FE70):
1. Restore the real device.
2. `gfx_release_default_pool` (0xD1F960).
3. `Reset(&pp)`. Until `TestCooperativeLevel` succeeds, retry `Reset`, `pump_window_messages` and `Sleep(100)`. After 100 tries it calls `TerminateProcess`.
4. `gfx_recreate_default_pool` (0xD1F7D0).
5. Point `0x252A2D0` at the null device again and set `0x252A262`. The next 3 frames are dropped before the real device is restored (above).

**`gfx_set_display_mode`** (0xD1FF60):
- If the aspect ratio crosses 1.45 (4:3 ↔ widescreen), it does not reset. It calls 0x773EB0(0) and 0x760B80 instead (a UI reload).
- Otherwise, if anything changed:
  1. `Sleep(500)` and release the default pool.
  2. `AdjustWindowRect`, `SetWindowPos`, and update the resolution globals (`0x22FDCAC`, `0xEC26A0`, `0x22FD8E4`, `0x22FDC1C`, aspect `0xEC26A4`).
  3. `Reset`. On failure it restores the old mode; after 2 failures it falls back to `gfx_reset_device`.
  4. Recreate the default pool.

Resources are created directly on the calling thread (`CreateTexture` / `CreateVertexBuffer` / `CreateIndexBuffer` / `CreateVertexDeclaration` from many places; shaders 0xCF4C10, 0xD19120, 0xD1B5B0; render targets 0xD1E140, 0xD1F7D0; queries 0xD19470). The device is created MULTITHREADED for this reason.

## Device and wrapper objects

| Global | Holds |
|---|---|
| `0x252A2CC` | IDirect3D9, from `gfx_d3d9_create` (0xC29D70, which prefers an already-loaded d3d9.dll) |
| `0x22F63F8` | D3DPRESENT_PARAMETERS |
| `0x252A2D0` | the device every command uses: the real one, or the null device while lost |
| `0x252A2D4` | the real IDirect3DDevice9 |
| `0xEC24CC` | `IDirect3DDevice9Lost`: vtable `0xE584A4`, 119 slots, all stubs; +4 = the real device. Its static initialiser is 0xCF58E0. |

`gr_vertex_buffer` (vtable `0xE583D4`) and `gr_index_buffer` (`0xE58410`), 14 slots each:
- They are the game's own implementations of IDirect3DVertexBuffer9 / IDirect3DIndexBuffer9 (Lock / Unlock / GetDesc) over game-owned memory.
- They are built at 0xBD8200 / 0xBD88E0 and used for dynamic geometry such as distant vehicle headlights.

## Port seam

A Rust renderer can:
- replace `gfx_execute_commands` (keep the ring and the writers);
- or consume the ring itself on its own thread, honouring the published-end pointer and the 16 MB rewind handshake in `gfx_command_buffer_wait`.

Only the creation calls above and commands 22/23 (the float texture slots) touch resources outside the stream.
