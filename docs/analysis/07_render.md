# 07: Renderer threads, the D3D command buffer and the device wrapper

Groundwork for the D3D wrapper port. Names are in `docs/functions.csv` (subsystem `render`).

## Threads

The game never calls Direct3D from its game or render threads. It writes **D3D commands** into a ring buffer, and the thread that owns the window and the device executes them.

| Thread | Entry | What it does |
|---|---|---|
| WinMain (main thread) | `WinMain` 0x520BA0 | window, device creation (`gfx_init_request` 0xD151A0 → `gfx_create_device` 0xD1F3F0), then its loop runs `gfx_execute_commands` (0xD20360) |
| supervisor thread | `app_supervisor_thread_proc` 0x520A30 | first `startup_loading_render_loop` (0x520630), then the `game_render_thread_frame` loop (0x522640): waits for the sim, builds render batches, runs `render_frame` (0x52A710) |
| game thread | `game_main_thread_proc` 0x520790 | `game_frame_tick` (0x520450): sim, UI (vint draws through the 2D layer), and the end of the frame |

**End of frame from the game thread:**
1. `game_frame_tick` → 0x521A00 → 0x5217E0 → `gfx_present` (0xD15110).
2. `gfx_present` uses the calling thread's render context (TLS: `fs:[0x2C]` → +0x5C, +0x60; defaults 0xEC2610 / 0xEC26A0), then calls `gfx_end_frame` (0xD211F0).
3. `gfx_end_frame`:
   1. Writes commands 1 (EndScene) and 0x14 (Present).
   2. Publishes the written bytes (InterlockedExchangeAdd on `0x33D62F4`).
   3. Once more than 16 MB are in flight, calls `gfx_command_buffer_wait` (0xD1C780). It sleeps until the executor has caught up, then rewinds the buffer.
   4. Runs the executor once itself (0xD20360).

`gfx_present` has other callers too: startup, `game_init_allocators_and_packfiles`, the supervisor 0x68BE50, 0x697B30 and 0x758B90 (loading screens).

## The command buffer

| Address | Field |
|---|---|
| `0x33D62EC` | buffer base |
| `0x33D62F8` | write pointer (writers append dwords: command id, then arguments) |
| `0x33D62FC` | last published position |
| `0x33D62F4` | published byte count (atomic) |
| `0x33D6300` | read side |
| 16 MB | flush threshold |

Readers 0xBD5550-0xBD56C0 pop a u32 / u8 / float / rect / blob from the buffer (EAX = buffer). `0xCF5940` reports whether there is data. `0xCF5900` turns (primitive type, count) into a vertex count for DrawPrimitiveUP.

**gfx_execute_commands** (0xD20360):
1. Stamps the frame time (QueryPerformanceCounter → `0x34A5108`).
2. While there is data, pops a command id and switches over it (jump table `0xD21164`, 32 entries):

| Id | D3D call | Writer (examples) |
|---|---|---|
| 0 | BeginScene | |
| 1 | EndScene (plus the PerfHUD FPS overlay: D3DXCreateFontA "verdana", "FPS : …") | `gfx_end_frame` |
| 2 | Clear (with flag/colour/z/stencil arguments) | `gfx_cmd_clear` 0xD18B30 |
| 3 | SetPixelShader | 0xD21480 |
| 4 | SetVertexShader | 0xD214B0 |
| 5 | SetVertexDeclaration | 0xD21450 |
| 6 | SetVertexShaderConstantF | 0xD215E0 |
| 7 | SetPixelShaderConstantF | 0xD21560 |
| 8 | SetStreamSource | 0xD21310 |
| 9 | (two values) | 0xD214E0 |
| 10 | SetIndices | 0xD213B0 |
| 11 | DrawPrimitive | 0xD21740 |
| 12 | DrawIndexedPrimitive | 0xD21650 |
| 13 | DrawPrimitiveUP (vertex data inline in the buffer) | 0xD217F0 |
| 14 | SetTexture | 0xD21620 |
| 15 | SetRenderState | written inline by many render functions |
| 16 | SetRenderTarget | 0xD1CC40 |
| 17 | SetDepthStencilSurface | 0xD1CBE0 |
| 18 | StretchRect | 0xD1DA50 |
| 19 | SetSamplerState | 0xD21890 |
| 20 | Present, then device-lost handling (0xD20230: TestCooperativeLevel / Reset) and frame pacing (Sleep) | `gfx_end_frame` |
| 21 | SetRenderState + SetScissorRect | 0xD1E7D0 |
| 22 | texture upload (0xD195E0 and a lock/copy) | 0xD1DBB0 |
| 23 | SetTexture (stage, resource) | |
| 24 | SetViewport | 0xD1DB50 |
| 25, 26 | resource methods (vtable +0x18) | 0xD1CAB0, 0xD1CA60 |
| 27, 28 | unused | |
| 29 | GetRenderTargetData | 0xD1D9F0 |
| 30 | UpdateSurface | 0xD1D990 |
| 31 | screenshot / readback (0xD1C530) | 0xD1CB00 |

The device calls outside the buffer are all creation and reset. Resource creation happens directly on the calling thread:
- `CreateTexture` / `CreateVertexBuffer` / `CreateIndexBuffer` / `CreateVertexDeclaration` (many callers);
- shaders: 0xCF4C10, 0xD19120, 0xD1B5B0;
- render targets: 0xD1E140, 0xD1F7D0;
- queries: 0xD19470;
- device reset: 0xD1FE70 / 0xD1FF60.

## Device and wrapper objects

| Global | Holds |
|---|---|
| `0x252A2CC` | IDirect3D9, from `gfx_d3d9_create` (0xC29D70, which prefers an already-loaded d3d9.dll) |
| `0x22F63F8` | D3DPRESENT_PARAMETERS (format 0x15 A8R8G8B8, window `0x22F6414`, windowed `0x22F6418`) |
| `0x252A2D0` | **the IDirect3DDevice9**. Every executor case reads it; 223 references |
| `0x252A2D4` | a copy of the device pointer |
| `0xEC24CC` | `IDirect3DDevice9Lost` |

`IDirect3DDevice9Lost` (vtable `0xE584A4`, 119 slots) is a static holder {vtable, device}. Its static initialiser 0xCF58E0 sets it up, and `gfx_create_device` stores the real device in its +4 (`0xEC24D0`). Its methods are stubs (QueryInterface returns 0, …). It is a null-object device, not a proxy for the real one.

`gr_vertex_buffer` (vtable `0xE583D4`) and `gr_index_buffer` (`0xE58410`), 14 slots each, are the game's own implementations of IDirect3DVertexBuffer9 / IDirect3DIndexBuffer9 (Lock / Unlock / GetDesc). They are built at 0xBD8200 / 0xBD88E0 and wrap game-owned memory. They are used for dynamic geometry such as distant vehicle headlights.

For a port, the seam is clear:
- intercept `gfx_execute_commands`, or replace the 32 command handlers;
- intercept the creation calls listed above;
- the game side only ever writes the command stream.
