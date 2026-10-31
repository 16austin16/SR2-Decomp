# 05 — World streamer (`streamman`)

Status: core mapped 2026-10-30 (see the sections below). Nothing ported yet.

The streamer sits on top of the file API (all Rust now). It decides which world chunks,
pegs (textures), meshes, characters, vehicles and animations are resident, and loads them
through the packfile layer.

## Where the code is

| Area | Address range | Evidence |
|---|---|---|
| streamman2 core + loading-info classes | ~`0xA84000`–`0xA89000` | vtables below; strings `streamman2 reading resource %s` (`0xE44504`), `… chunk extras reading resource %s` (`0xE444B4`), `… vehicles reading resource %s` (`0xE44540`) |
| Chunk file loader (`.chunk_pc`, BBCHUNK header + CHUNK_VERSION check) | ~`0x4AF2E0`–`0x4AF630` | "does not appear to be a BBCHUNK file", "CHUNK_VERSION is too low/high" |
| Chunk / superchunk pool set-up (cpu/gpu pools: chunk0..2, always loaded, anim, mp) | ~`0x51E300`–`0x51E930` | pool name strings `chunk0 cpu` … `mp_gb_chunk gpu` |
| `g_chunk_pc` / `_ml.chunk` / `_al.chunk` names | ~`0x6992EF`, `0x69C5BE`–`0x69C720` | |
| Chunk swap sequences (`chunk_swap_sequences.xtbl`) | ~`0x542005` | |
| Component streaming + "Temp component stream pool" | ~`0x9E4364`–`0x9E82BB` | |
| Chunk limits lookup (`sr2_chunk%s`, "Unable to find chunk limit") | ~`0x9D8E20`–`0x9D8F66` | |

## Loading-info classes (2 virtuals each)

| Class | vtable | slot 0 | slot 1 |
|---|---|---|---|
| `streamman_resource_loading_info_base` | `0xE44714` | `0xA887C0` | `0xA887D0` |
| `…chunk_geom_loading_info` | `0xE44720` | `0xA85330` | `0xA887D0` |
| `…chunk_peg_loading_info` | `0xE4472C` | `0xA85B10` | `0xA85AB0` |
| `…chunk_extras_loading_info` | `0xE44738` | `0xA84900` | `0xA84860` |
| `…peg`, `…static_mesh`, `…resource_character`, `…resource_logo`, `…resource_vehicle`, `…animation` | `0xE44744`…`0xE44780` | `0xA887C0` | `0xA887D0` |

Only the three chunk loaders override the base. Slot 1 of `chunk_extras` (`0xA84860`) strips
the extension from the name at `this+0x6F0` (64-char buffer), calls vtable slot 0, then
`0xA84510(buf, *(u16*)(this+0x772), name, 0xFFFF)`.

Ghidra hasn't made functions at most of these slot targets yet (vtable-only references), so
they need disassembling by hand or defining first.

## Side note: D3D wrapper

`IDirect3DDevice9Lost` (vtable `0xE584A4`, 119 slots) is a small null-object device: all its
methods together are about 1.3 KB of stubs. It is not the renderer. `gr_vertex_buffer` /
`gr_index_buffer` are about 270 bytes each.

## Correction: where the core is

The streamman core is `0xA72C70`–`0xA88B50`, not `0xA88A40`–`0xA8E000`. Most of that later
range is other systems: traffic lanes (`traffic_lanes.xtbl`, `0xA88BC0`), glare textures
(`0xA8D640`) and SpeedTree loading (`0xA8DAFB`–`0xA8DFAF`). Only `0xA88A40`/`0xA88B50` (slot
setup) are streamman. Names below are in `docs/functions.csv` (subsystem `streamman`).

## Types, slots and handles

There are 10 resource types. `0xE87BB0[t]` is the slot count per type, and `0x2225B3C[t]` points
to that type's array of 0x28-byte slot records, allocated by `streamman_init_slots` (`0xA88A40`).
`0xE87B54[t]` (that is `0xE87B50[t + 1]`; code indexes it as `[t*4 + 0xE87B4C]` with t from 1) is
the loading info of single-slot types:

| t | Loading info | Slots | Object(s) |
|---|---|---|---|
| 0 | `streamman_chunk_geom_loading_info` | 1 | `0xEB81B0` |
| 1 | `streamman_chunk_peg_loading_info` | 4 | `0x2FA4098` + i*0x544 (built by a static initialiser at `0xDB1890`) |
| 2 | `streamman_resource_vehicle_loading_info` | 1 | `0x2FA7398` (vtable set at `0xA889D9`) |
| 3 | `streamman_resource_character_loading_info` | 1 | `0xEB86F8` |
| 4 | `streamman_chunk_extras_loading_info` | 4 | `0x2FA55A8` + i*0x77C (static initialiser `0xDB1940`) |
| 5 | `streamman_resource_character_loading_info` | 1 | `0xEBA360` |
| 6 | `streamman_animation_loading_info` | 1 | `0xEBBAD8` |
| 7 | `streamman_peg_loading_info` | 1 | `0xEBBD40` |
| 8 | `streamman_static_mesh_loading_info` | 1 | `0xEBC280` |
| 9 | `streamman_resource_logo_loading_info` | 1 | `0xEB9E70` |

`0xE87B50[0]` is `0x223E410`, which is not a loading info (it is used by `0xA88D10`/`0xA8B6F0`).
`0xE87B84` lists the 3 chunk types (0, 1, 4) for the chunk-wide helpers.

A **handle** is `type << 16 | index`; `0xFFFF0000` means none. The slot record (0x28 bytes):

| Off | Field |
|---|---|
| +0x00 | type |
| +0x04 | state: 0 idle, 1 requested (queued), 2 reading, 3 aborted/finished, 4 loaded. "Idle" for the API means 0, 3 or 4 |
| +0x08, +0x0C | cleared on request |
| +0x10 | loading info |
| +0x14 | request-queue node index, -1 when not queued |
| +0x18, +0x1C | outstanding async read ids (cancelled by abort) |
| +0x20 | active batch (set by start_load) |

## Request, process, complete

1. **Request.** For example `streamman_request_chunk` (`0xA80360`) builds the `chunk_pc`/`g_chunk_pc`/`.pvs_pc`/`.peg_pc` names and calls `streamman_request_chunk_extras` (`0xA84510`), `_peg` (`0xA85890`) and `_geom` (`0xA85140`). Each fills the loading info (names, chunk id, completion callback at +0x1C), sets up its read steps with `streamman_step_init` (`0xA7F770`), sets slot state 1 and pushes the slot onto the request queue `0x2F9B88C` (`streamman_queue_push`, `0xA72C70`, ECX = queue). `0xA7FC30` is the same for characters.
2. **Process.** `streamman_process` (`0xA7D6D0`, cdecl, no args) is the pump. If nothing is current (`0x2529FF0`), it pops the best request (`0xA72DA0`, using the queue's compare callback at +0x14). Type 6 (animation) goes to `0xA7D0C0`, all others to `streamman_process_generic` (`0xA7F100`). If the state is 1, `streamman_start_load` (`0xA7DDE0`) issues the reads: per step, a begin callback, size/alignment bookkeeping on info +8/+0xC, read nodes from `0xA7D930` submitted by `0xA73290`. The batch goes to the active list `0x252A01C`, state 2. Chunk extras (type 4) use `0xA7E100`/`0xA7E900` instead.
3. **Complete.** `streamman_poll_active` (`0xA7F0C0`) checks every active batch (`0xA7ED40`; `0xA7E900` for type 4). On completion the info's callback at +0x1C runs (chunk geom: `0xA84FF0`, which reaches `chunk_load_from_memory` from `0xA8504E`; peg: `0xA857A0`; character: `0xA7FA40`).
4. **Unload/abort.** `streamman_unload` (`0xA7D4F0`) calls loading-info **slot 0** (`unload`), or `streamman_abort_current(false)` if the slot is the current one. `streamman_abort_current(true)` (`0xA7CE70`) calls **slot 1** (`abort`). Both cancel the outstanding reads and the queue node, and set the state (0, or 3 for abort).

**Per frame:** `game_frame_tick` (0x520450) → `0x521A00` → `0x68E410` → `world_streaming_do_frame` (`0x68ED30`), which runs the other streamers and then calls `streamman_process` twice. Blocking waits (`streamman_wait` 0xA7D600, `loading_screen_pump` 0x481080, `component_streaming_wait` 0x9DF160) call the pump in a loop with Sleep. In total 19 call sites pump it.

**Calling conventions** are mostly cdecl, with these exceptions: the queue functions (ECX = queue for push; EAX = index, EDX = queue for remove), `streamman_step_init` (ESI = step, EAX = file, CX = alignment), `streamman_request_chunk_geom` (EAX = request), `streamman_request_chunk_extras` (ECX = chunk pool), `streamman_request_character` (EAX/ECX/EDX = buffers and size), `streamman_unload_flagged_geom` (AL = flag). Loading-info slots 0 and 1 are thiscall with no arguments, and both are an empty `ret` in the base.

## Loading-info layout

| Off | Field |
|---|---|
| +0x00 | vtable |
| +0x04 | request argument (peg: request[3]) |
| +0x08 | destination offset/cursor, aligned per step |
| +0x0C | bytes left in the destination |
| +0x10 | optional per-step filter `bool(*)(file)`; when NULL a step runs if its file field is non-zero |
| +0x14 | pointer to the read steps (normally the inline array at +0x24) |
| +0x18 | request argument |
| +0x1C | completion callback |
| +0x20 | flags: bits 0-9 current step, bits 10-19 step count, bit 20 (0x100000) no destination accounting (the step manages its own memory), bit 21 (0x200000) destination too small, bits 22-23 also cleared on request |
| +0x24 | read steps, 0x244 bytes each |

Read step (0x244 bytes, set up by `streamman_step_init`): +0x000 file, +0x004 u8, +0x008 0x200-byte buffer, +0x208, +0x20C, +0x210 read size, +0x214 u16 alignment, +0x218 file offset, +0x21C flags (bit 0, bit 1 set by the chunk requests), +0x22C begin callback `bool(info)`, +0x230 callback, +0x234 issue callback `void(info)`, +0x238 after callback `bool(info)`, +0x23C, +0x240 (peg sets it to `0xA856E0`).

Per type, after the steps:
- **chunk geom / chunk peg** (2 steps, end 0x4AC; record size 0x544): +0x4AC name[0x41], +0x4EC 0, +0x4ED second name[0x41], +0x52D 0, +0x52E u16 chunk id, +0x530, +0x534 object, +0x538, +0x53C mempool, +0x540..+0x543 flag bytes (from the request; +0x542 marks geom that `0xA80CC0` unloads).
- **chunk extras** (3 steps, end 0x6F0; size 0x77C): +0x6F0 name (64), +0x772 u16 chunk id, +0x778 chunk pool (one of the 3 at `0x2771894`).
- **character (type 3)**: +0x1750/+0x1754 two destination buffers, +0x1758 size, +0x175C request argument, +0x1760/+0x1764 cursors (start equal to +0x1750/+0x1754), +0x1768 mode byte (from 0x2527D14/0x2527CF7/0x2527D15 and `0x6C9090`), +0x176C/+0x1770 written at `0xA801A1`. Steps start at +0x24 with alignment 0x40; callbacks `0xA7F840`/`0xA7F9C0`. These fields are on the character info, not the static-mesh one.

## Chunk loading

`chunk_load_from_memory` (`0x4AF280`; ECX = cpu mempool, EDI = second pool, called from the chunk-geom completion at `0xA8504E`):
1. Allocates a 0x250-byte chunk record through the pool's alloc slot (size 0x250, align 0x10). On failure: "Failed %d byte alloc loading chunk %s".
2. Resolves the chunk id (`0x4A9840`), records it in `0xF7AAE8[id]` (ids below 0x866), and links the neighbouring ids at record +0xA/+0xC.
3. Header: magic `0xBBCACA12` (else "does not appear to be a BBCHUNK file"); version `== 0x79`, with "too low" below and "too high" above. It copies header words [0x25], [0x26], [0x28], [0x35..0x3A] into the record and flags from header[3].
4. Walks header[4] section entries of 12 bytes `{type, ?, offset}` and dispatches types 0-0x26 to `0x4ACDF0`, `0x4ADCD0`, … (some just store count/pointer pairs into the record).
5. Adds the record to the loaded-chunk lists (`0xF6FBD8`, count `0xF6FBC4`, and per-feature lists from `0xF70B88` to `0xF77994`), then calls `0xA8F7C0`, `0x4AB7D0` and `0xA132E0` and finishes with `0x4AFC70` or marks it pending.

The chunk pools are set up inside `game_init_allocators_and_packfiles` (`0x51D800`, called from `game_main_thread_proc` at `0x52079F`). The `chunk0/1/2 cpu`/`gpu`, `chunk always loaded`, `chunk_anim0..2`, `interiors` and `mp_*_chunk` pools are created there. The three chunk pools are 0x58-byte objects at `0x2771894` (`0x27718EC`, `0x2771944`); `streamman_request_chunk_extras` gets one in ECX.
