# 00 — Initial overview: SR2_pc.unpacked.exe

- Target: `bin/SR2_pc.unpacked.exe`, sha256 `f76b3cccd10775e231e6ae4df2be9a76c4346179d4fbe01f9d5d12bef223870e`
- Tooling: REA, Ghidra 12.1.4, default auto-analysis. The session's ephemeral DB stays open.
- PDB reference in the PE: `SR2_pc_final.pdb`, GUID `0a2dda9a-3be0-4e55-a9b7-2fe103e22b89`, age `0xaf`. The PDB itself is not available.
- Confidence labels:
  - **Observed** means read directly from decompiler, xref or caller output.
  - **Inferred** means my interpretation of that evidence.
- Evidence IDs (`ev_…`) refer to REA session evidence.

## 1. Inventory (Observed, `ev_484817d6…4d0a`)

| Item | Value |
|---|---|
| Procedures | 25,763 |
| Defined strings | 23,587 |
| Image base | 0x400000 |

| Segment | Start | End (excl.) | Size |
|---|---|---|---|
| Headers | 0x400000 | 0x401000 | 4 KiB |
| .text | 0x401000 | 0xdb8000 | 9.7 MiB |
| .rdata | 0xdb8000 | 0xe80000 | 800 KiB |
| .data | 0xe80000 | 0x34a639c | ~38 MiB (mostly BSS) |
| .tls | 0x34a7000 | 0x34a9000 | 8 KiB |
| .rsrc | 0x34a9000 | 0x34b5000 | 48 KiB |
| .reloc | 0x34b5000 | 0x3559000 | 656 KiB |
| .bind | 0x3559000 | 0x35ad000 | 336 KiB. SteamStub leftover, ignore. |

## 2. Startup chain

| Address | Proposed name | Role | Basis | Evidence |
|---|---|---|---|---|
| 0x00c9e22e | `entry` | Calls `___security_init_cookie`, then `___tmainCRTStartup` | Observed | ev_a691ba76… |
| 0x00c9e04e | `___tmainCRTStartup` | VS2005 CRT startup (Ghidra library match). Calls `WinMain(0x400000, 0, cmdline, nShow)` and then `_exit` or `__cexit`. | Observed | ev_0a096ed1… |
| 0x00520ba0 | `WinMain` | See below | Observed | ev_465f78b1… |

### WinMain (0x520ba0): summary (Observed)

1. Calls `CoInitializeEx`. Saves the system STICKYKEYS, TOGGLEKEYS and FILTERKEYS settings (SPI 0x3a, 0x34, 0x32) and SPI 0x101c, then calls `FUN_00520910(0)`.
2. Reads CPUID leaf 1 and stores `edx & 1` (FPU flag) in `DAT_00e8e88a`.
3. Sets the working directory to the exe's directory (via `_splitpath`-like `FUN_00c98c24`, then `_makepath_s`, then `SetCurrentDirectoryA`).
4. Parses command-line arguments into flags:

   | Argument | Effect |
   |---|---|
   | `2ndmon` | `DAT_02527348` |
   | `nologos` | `DAT_02527349` |
   | `nocb` | `DAT_0252734a` |
   | `showfps` | `DAT_0252734b` |
   | `defaults` | `DAT_0252734c` |
   | `forceeng` / `forcelang_xx` | Sets a language id in `DAT_00e98af8` (en=9, de=7, fr=0xc, es=10, it=0x10, nl=0x13, se=0x1d, dk=6, pl=0x15, cz=5, ru=0x19, ch=4, jp=0x11) |

5. Creates `%CSIDL_COMMON_APPDATA%\<str@0xde183c>\Saints Row 2` (`SHGetFolderPathA` 0x1c). The resulting path is kept in `DAT_0144e650`.
6. Records the main thread id in `DAT_00fa2394`, then starts `CreateThread(FUN_00520a30)`.
7. Runs an infinite main-thread loop:
   - When `DAT_02527323 == 1` (a request from another thread), it calls `FUN_00bfa620` (window), `FUN_00d151a0` (graphics device), `FUN_0051cd80` and `FUN_00c19350`. On first run it also does a one-time table reset.
   - Unless `nocb` was passed, it calls `FUN_00d20360`.
   - It then calls `pump_window_messages()` and `Sleep(0)`.

### Threading model and top-level functions

| Address | Proposed name | Role | Basis | Evidence |
|---|---|---|---|---|
| 0x00520910 | `accessibility_keys_disable_or_restore` | With arg 0: turns off the Sticky/Toggle/Filter-key hotkeys. With arg ≠0: restores the saved values (SPI_SET 0x3b, 0x35, 0x33, 0x101d). | Observed (SPI ids) | ev_a9479ccc… |
| 0x00520a30 | `app_supervisor_thread_proc` | Thread started by WinMain; it creates the game thread. While the game thread initialises, it pumps messages and, on request, runs the loading loop. After that it waits for shutdown, calls `FUN_00522640`, restores accessibility keys and calls `ExitProcess(0)`. | Observed | ev_4d078821… |
| 0x00520790 | `game_main_thread_proc` | The game thread. Runs init calls (`FUN_0051d800`, `FUN_00bf3290`, `FUN_0051f600`, `FUN_00696c70`, `FUN_00522450`, `FUN_0051f700`, `FUN_00cfbc00`, `FUN_0069a7c0`, `FUN_00d14950`). Then loops `while(!quit) if (game_frame_tick()) break;`. Finishes with `FUN_00699bc0` (shutdown). | Observed structure; per-call roles not yet known | ev_e5d274d9… |
| 0x00520450 | `game_frame_tick` | Per-frame update. Returns 1 to quit. Calls ~20 subsystems (`FUN_00699c90`, `FUN_00c063d0`, `FUN_00b99550`, `FUN_004fe760`, `FUN_00500ca0`, `FUN_00c1f320`, `FUN_00753aa0`, `FUN_0069b160`, `FUN_00bf0710(paused?, dt, …)`, `FUN_008217f0`, `FUN_00809640`, `FUN_00691430`, `FUN_0069a440`, `FUN_00697780`, `FUN_00521a00`), with timer start/stop around some of them (`FUN_00c0be10` / `FUN_00c0c990`). It also handles the `screen_change_failed_pc` UI message. | Observed calls; "frame tick" is Inferred | ev_a4a44054… |
| 0x00520630 | `startup_loading_render_loop` | Loops until `DAT_02527e94 == 3`: pumps messages, then `FUN_00d18b30(7, 0xff000000, 1.0f, 0)` (looks like a D3D `Clear` of target+z+stencil to black) and present/render calls. | Inferred | ev_20d4c136… |
| 0x00bfa570 | `pump_window_messages` | `PeekMessageW`/`TranslateMessage`/`DispatchMessageW` loop, plus IME composition reset | Observed | ev_469e70c2… |
| 0x00bfa620 | `single_instance_check_and_create_window` | `CreateMutexA`. On `ERROR_ALREADY_EXISTS` it calls `FindWindowA("…","Saints Row 2")`, `SetForegroundWindow` and `_exit(1)`. Otherwise it calls `create_game_window`. | Observed | ev_f4ec35e5… |
| 0x00bfa220 | `create_game_window` | Sole caller of `RegisterClassExA` and `CreateWindowExA` | Observed (callers) | ev_f977e3ae…, ev_24568986…, ev_aa24b297… |
| 0x00d151a0 | (unnamed) graphics init | Called from the WinMain request block; leads to `FUN_00d1f3f0`, then `gfx_d3d9_create` | Observed call chain; role Inferred | ev_6ecbf7b8…, ev_5a00c947… |
| 0x00c29d70 | `gfx_d3d9_create` | Only caller of the `Direct3DCreate9` thunk (0xccd540) | Observed | ev_6b9687a9… |
| 0x00c13910 | `input_dinput8_create` | Only caller of the `DirectInput8Create` thunk (0xccd546) | Observed | ev_dbd852bb… |

**Inferred architecture:**
- The main (WinMain) thread owns the Win32 window and D3D device creation, triggered by flag `DAT_02527323`.
- A supervisor thread drives loading-screen rendering.
- A dedicated game thread runs `game_frame_tick`.
- Code at `DAT_033d62d0` (via `DAT_0252a370`/`DAT_0252a374`) looks like a recursive critical section guarding the render device.

For the Rust rewrite this suggests a window/event-loop thread plus a game thread.

Imported input and graphics APIs (Observed, `ev_bb56dcbb…`):
- `XInputEnable`, `XInputGetState` and `XInputSetState` (XINPUT1_3)
- `Direct3DCreate9` (D3D9)
- `DirectInput8Create` (DINPUT8)
- `CreateWindowExA` and `RegisterClassExA` (USER32)

## 3. String anchors

Searches: `ev_c8e51c92…` (parent `ev_5c7092e0…`, 413 rows) and `ev_21e8215e…`.

| Anchor | Result |
|---|---|
| `.vpp_pc` | Only `hmaps.vpp_pc` @0xddb804 exists. Archive names are stored as bare `*.vpp`: `patch.vpp` @0xde0dc8 through `misc_tables_startup.vpp` @0xde0eb8 (a contiguous list: textures, pegs, music1-4, meshes, cutscenes, common, chunks1-4, audio, anims, vint_docs, startup, shaders). Also `misc_tables.vpp` @0xe00b78, `city_load.vpp` @0xe00ba0, `preload.vpp` @0xe06cb8. The `_pc` suffix is probably appended at runtime (Inferred). |
| `.str2_pc` | **No match**. There is no `str2` string at all; the name may be built at runtime or this SR2 build may not use str2 containers. |
| `%s.tbl` | **No match**. Related strings: `preload.tbl` @0xe06cc4, `preload_anim.tbl` @0xe06cd0, `obj_glow.tbl` @0xddcbb4, `effects.tbl` messages @0xde03c0. |
| `xtbl` | About 330 `*.xtbl` table names (0xdc5938 … 0xe4b384). Format strings: `%s_%s.xtbl` @0xdda814, `%s%s%s.xtbl` @0xde5fdc, `%s%s.xtbl` @0xdf5900, `%s_veh.xtbl` @0xe480f4, `hitman_%s.xtbl` @0xdfa338. `*.xtbl` @0xe47e44 is referenced from 0xaed20e in `FUN_00aeb850` (6.7 KB function; likely an xtbl enumerate/load, Inferred). Evidence: ev_f69be0bd…, ev_fe868b63…. |
| lua | Lua 5.x runtime strings: `LUA_PATH` @0xdc64f8, `?;?.lua` @0xdc6510, `lua_getinfo` error @0xdc6af4. Script names: `system_lib.lua` @0xdc70cc (xref 0xcde44f in `FUN_00cde400`, ev_59c573c8… / ev_6cafc8e7…), `ug_lib.lua` @0xe3e808, `vint_lib.lua` @0xe4cea8, `lua_script_file` @0xe4ce8c, `vint_force_lua_gc` @0xe4cb50. |
| d3d | `d3d9.dll` @0xe7f4e4 has no xrefs (ev_c9bb0b07…). D3D9 is entered through the import `Direct3DCreate9`, whose caller is 0xc29d70. |
| `Direct3DCreate9` callers | 0xc29d70, which is called by 0xd1f3f0, which is called by 0xd151a0, which is called from WinMain (Observed). |
| `CreateWindowExA` callers | 0xbfa220, called by 0xbfa620, called from WinMain (Observed). |
| SR2 | `Saints Row 2` (window title / appdata folder). Many `sr2_*` UI data-responder names (`sr2_local_player_*`, `sr2_multi_lobby_*`), chunk names `sr2_chunk%s` @0xe3cac8, `sr2_city` @0xdffb60, `sr2.vint_proj` @0xe25d98 (Vint UI project), `SR2_Main_Rig.rig` @0xe4b094. |
| Packfile / memory | `packfile_pool` @0xe59698 is referenced from `FUN_00c0aa50` (0xc0abfb), probably packfile memory-pool setup (Inferred). `temp packfile mempool` @0xe00c64. Havok `hkPackfileHeader` @0xe4b7e4 shows Havok physics is linked in. Evidence: ev_8517c55b…, ev_06b2841f…. |

Archive-list strings such as `startup.vpp` (0xde0ea0) and `misc_tables.vpp` have no code xrefs. `misc_tables.vpp` is referenced only from data at 0xe96acc, which itself has no xrefs. These are probably pointer tables indexed by computed offsets, so they need data-type work in Ghidra.

## 4. Naming status / what went wrong

- `set_address_name` returned `capability_unavailable`: "Windows Ghidra P0 does not admit database mutation". `annotate_native_function` is documented as unsupported on Windows P0 as well.
- **No names were written to the Ghidra DB.** The "Proposed name" column above is the naming to apply once a mutable provider or session is available.
- On the first attempt the target was not open, because my REA session was separate from the coordinator's. I reopened it with `open_binary` (provider `ghidra`) after setting `startup_timeout_ms = 3600000` and `decompile_timeout_seconds = 120`. Analysis completed without timeout.

## 5. Suggested next steps

- Decompile the game-thread init calls to identify subsystems: `FUN_0051d800`, `FUN_00bf3290`, `FUN_0051f600`, `FUN_00696c70`, `FUN_00522450`, `FUN_00cfbc00`, `FUN_0069a7c0`.
- Decompile the `game_frame_tick` callees to identify subsystems.
- Trace the packfile name table around 0xde0dc8–0xde0eb8 to its consumer, i.e. the VPP mount order.
- Decompile `FUN_00aeb850` to work out the xtbl loader.
