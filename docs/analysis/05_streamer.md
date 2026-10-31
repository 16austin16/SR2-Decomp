# 05 — World streamer (`streamman`)

Status: core mapped 2026-10-30 (see the sections below). The request queue and the slot
queries are in Rust (see "Ported" at the end).

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

## Ported

`crates/sr2-hook/src/streamman.rs`, inline detours, shadow-checked on every call (the queue
methods snapshot the whole queue, run the original, rewind, run Rust and compare):

| Function | Address | Calls in a boot plus save load |
|---|---|---|
| `streamman_queue_init` (ECX = queue, kept) | `0xA72E20` | 86 |
| `streamman_queue_push` (thiscall-like, `ret 4`) | `0xA72C70` | 10,864 |
| `streamman_queue_remove` (EAX = index, EDX = queue kept, leaves ECX = node) | `0xA72D10` | 10,783 |
| `streamman_queue_pop_best` (stdcall) | `0xA72DA0` | 33,882 |
| `streamman_queue_node_index` (EAX = node, ECX = queue) | `0xA72E80` | 0 |
| `streamman_is_idle` / `_is_loaded` / `_is_unused` | `0xA7D560` / `0xA7D5A0` / `0xA7D5D0` | 21,070 / 0 / 0 |
| `streamman_get_slot_info` / `_clear_slot` | `0xA7D670` / `0xA7D6A0` | 0 / 2 |

76,687 calls, 0 mismatches. The queue is generic: besides the request queue `0x2F9B88C` it is
used by about 20 other call sites in `0xA75900`–`0xA77E90` (the read scheduler), and there are
59 queues at boot.

`pop_best` details: the compare callback gets `(best, candidate)` values. A result of 1 takes the
candidate unless `best < candidate`, so ties go to the later node. 2 always takes the
candidate, and anything else keeps the current best.

## Batches, read nodes and completion

`streamman_start_load` / `streamman_start_chunk_extras` group the reads of one slot into a **batch**:

| Off | Batch field |
|---|---|
| +0x00, +0x04 | links (free list `0x252A018`, active list `0x252A01C`) |
| +0x08..+0x14 | cleared on take |
| +0x0C | pending read nodes (circular list) |
| +0x10 | finished read nodes |
| +0x14 | slot |
| +0x18 | u16 chunk id (type 0 only, else 0xFFFF) |

Read nodes come from a pool at `0x252A020` (free count `0x252A024`, `streamman_read_node_alloc` 0xA7D930):

| Off | Read node field |
|---|---|
| +0x00, +0x04 | links |
| +0x08 | file |
| +0x0C | name suffix for error messages (e.g. ".anim_pc"), or 0 |
| +0x10, +0x14 | io request and sequence (from `async_io_submit`) |
| +0x20 | destination |
| +0x24 | size |
| +0x28 | u8 priority: 0x70 types 0/1, 0x40 other types, 0x30 chunk extras |
| +0x2C | step index (extras: part kind 0 mesh/peg, 1 anim, 2 file) |
| +0x30 | extras: object id |

Each pump, `streamman_poll_active` runs `streamman_check_batch` (0xA7ED40) on every active batch (`streamman_poll_chunk_extras` 0xA7E900 for type 4). The status codes come from `async_io_status` (0xA741A0): 1 pending, 2 error, 3 done, 5 failed. When nothing is pending, `streamman_finish_current` (0xA7CFC0) replays the step sizes into slot +8/+0xC and calls the loading info's completion callback. It returns true for state 4, or false to abort. The read requests are then released (`async_io_release`), and the nodes and the batch go back to their pools.

Completion callbacks: chunk geom `chunk_geom_on_loaded` (0xA84FF0, calls `chunk_load_from_memory` with step 0's buffer/offset/size), chunk peg `chunk_peg_on_loaded` (0xA857A0), character `character_on_loaded` (0xA7FA40), chunk extras 0xA81620.

Type 6 (animation) does not use batches: `streamman_process_sequential` (0xA7D0C0) issues one read at a time and keeps the request in slot +0x18/+0x1C (which is why `streamman_abort_current` cancels those).

Read failures are reported with names from the table at `0xE87BEC` ("UNKNOWN", …), indexed by `async_io_error_code` (0xA74340; 10 = "not enough memory", 0xB = no request).

## Read scheduler ("hdd thread")

Below the streamer, every read goes through one scheduler thread. Its manager is `*(*0xEB80E0)` (the first entry of a manager list: count `0xEB80E8`, capacity `0xEB80E4`). It is created once by `hdd_scheduler_create` (`0xA73F90`), which allocates the 0x44-byte manager from the default mempool, initialises it (`hdd_scheduler_init`, `0xA73030`) and starts the thread with `0xBFA930(hdd_thread_proc, mgr, 1)` (handle at +0x24).

**Correction to the batch section above:** `async_io_status` code 2 means *being read*, not error. The step callback at +0x230 is a progress callback (false aborts). The codes are 0 none, 1 queued, 2 being read, 3 done, 4 cancelled, 5 failed.

| Off | Manager field |
|---|---|
| +0x00 | auto-reset event (wakes the thread) |
| +0x04 | u8 |
| +0x08 | CRITICAL_SECTION (guards everything below) |
| +0x20 | |
| +0x24 | thread handle |
| +0x28 | name "hdd thread" |
| +0x2C | u8 stop |
| +0x2D | u8 allow low-priority reads (priority < 0x22); 1 at init |
| +0x2E, +0x2F | u8 flags (+0x2F pauses the service loop) |
| +0x30 | free request list |
| +0x34 | pending list, sorted by priority (highest first) |
| +0x38 | finished list (cancelled requests wait here) |
| +0x3C | request being serviced |
| +0x40 | request array: 256 x 0x48 bytes |

| Off | Request field (0x48 bytes) |
|---|---|
| +0x00 | sequence (bumped on every free; callers pass it back to detect reuse) |
| +0x04 | file name |
| +0x08 | name suffix (for messages) |
| +0x0C | start offset in the file (0 from the streamer) |
| +0x10 | size |
| +0x14 | destination |
| +0x18 | u8 priority (0x70/0x40/0x30 from the streamer; 0x75 when made urgent) |
| +0x1C, +0x20 | 0 from the streamer |
| +0x24 | open file while reading |
| +0x28 | bytes read |
| +0x2C | state: 1 free, 2 queued, 3 reading, 4 cancelling, 5 cancelled, 6 done, 7 open failed, 8, 9 read failed |
| +0x30 | one-shot waiter event (async_io_wait_done) |
| +0x34 | attached wait event (hdd_make_urgent / hdd_attach_wait_event) |
| +0x38 | u8 released by the owner |
| +0x3C | manager |
| +0x40, +0x44 | links |

`async_io_submit` (`0xA73290`, cdecl (mgr, desc[8], bool)) copies the 8-dword description into +0x04..+0x20, sets state 2, inserts it by priority and SetEvents the thread. It returns {request, sequence}.

`hdd_thread_proc` (`0xA72FB0`) calls `hdd_service` (`0xA73C50`, ECX = mgr) with Sleep(5) between passes. One pass:
1. WaitForSingleObject(event).
2. Under the lock, takes the pending head as current, unless its priority is below 0x22 and +0x2D is clear. Sets state 3.
3. Opens the file with `file_open` (0xBFDC20, now Rust), sets the start offset, reads with `file_read` (0xBFE170, Rust) into the destination, and closes with `file_close` (0xBFDF30, Rust). The file's error field (+0x158) picks the final state: 0 → 6, 3 → 5, else 9. An open failure gives 7.
4. `hdd_finish_current` (`0xA73150`) sets the state through `async_io_set_state` (`0xA739A0`, which signals waiters) and recycles the record at once if the owner already released it.
5. Repeats until the pending list is empty or the head is gated.

Cancel/release: `async_io_cancel_impl` (`0xA73430`) moves a queued request to the finished list (state 5), or marks the current one cancelling (state 4) and aborts its file read (`0xBFDAF0`). `async_io_free` (`0xA73650`) waits if the request is still in flight, then bumps the sequence and returns the record to the free list.

Blocking waits: `hdd_make_urgent` (`0xA73AC0`) bumps queued requests to priority 0x75 and attaches events. `async_io_wait` (`0xA738B0`) / `async_io_wait_event` (`0xA73850`) wait on them. `async_io_wait_done` (`0xA73900`) waits using the calling thread's event (TLS +0x78). Wait events come from a small pool object at `0xEB80C0` (vtable slot 1 free, slot 2 alloc).

Debug: flag `0x2529DDB` makes reads of `player.cvtf` copy from memory (`0x2223CC0`); flags `0x2529DD5..DD7` force release modes; `0x2529DDA` changes how files are opened and skips `0xBFE600` after a read.

## Chunk sections

`chunk_load_from_memory` walks the header's section table (`header[4]` entries of 12 bytes, `{type, ?, offset}`). Each type fills fields of the 0x250-byte chunk record ("rec"). Offsets inside a section are relative to the chunk data, and the handlers turn them into pointers. Most handlers follow one of two patterns: `count` then a 16-byte-aligned array, or `-1 = NULL` offset fix-ups. No handler contains a string except section 5, so names are given only where the evidence is direct. Handlers are named `chunk_section_XX[_what]` in functions.csv, and the full field list is in each row's note.

| Type | Handler | What it fills |
|---|---|---|
| 0x00 | `0x4ACDF0` objects | The main section. Object names (rec+0xEC/+0xF0); objects, 0x50 bytes each (rec+0x48 count, +0x4C), each pointing to a 0x60-byte render-info entry (rec+0x40/+0x44) and given a squared fade distance from the table `0x144EA40`; 3-level LOD sets; several typed arrays; 0x54-byte entries flagged 0x80000/0x100000 spawn entities through `0xA69E00` (kinds 0xF / 0x14) |
| 0x01 | `0x4ADCD0` lights | **Lights**: 0x94-byte entries (rec+0x104/+0x108): RGB colour, position, direction, orientation, ranges and fade distance, name, optional intensity values |
| 0x02 | `0x4A35E0` | 0x28-byte entries (rec+0x134/+0x138), then `0x7F63B0` |
| 0x03 | inline | **player starts** (`PlayerStart01`, …): count/pointer at rec+0x10C/+0x110 |
| 0x04 | `0x4ADF20` render_items | 0xC8-byte render items (rec+0x5C/+0x60) and a node tree (rec+0x64); `game_render` collects the visible ones through the tree (`0x5239A0`); dropped for flag-bit-0 chunks |
| 0x05 | `0x4ADF70` collision_models | Collision models via `0x7051C0` (indices rec+0x1F0/+0x1F4); remaps object collision indices (+0x3C/+0x3E). The only section with a string ("city_objects_load: Failed to load the collision models…") |
| 0x06 | `0x4AE0C0` placed_spawns | 0x34-byte placements (rec+0x19C/+0x1A0); type-name hash → table `0x263AF98`; created by `0x4C61C0` at position/orientation, not within 15 m of the player |
| 0x07, 0x08 | `0x4AE240`, `0x4AE280` collision_mopp | **Havok MOPP collision** (`BBPF`/`MOPP` in the data): two count/array pairs each (rec+0x68..+0x74, +0x78..+0x84); cleared when chunk flags & 0x13 |
| 0x09 | inline | count/pointer at rec+0x88/+0x8C |
| 0x0A | `0xA8AC00` | 0x3C- and 0x28-byte entries (rec+0x13C..+0x148), then `0xA8D970` with the traffic globals `0x2225B64`. Probably the road/lane network (unverified) |
| 0x0B | inline | **Path segments**: 0x20-byte {start vec3, end vec3, flags, link}, chained end-to-start at ground height (count/pointer at rec+0x14C/+0x150) |
| 0x0C | `0x4AE320` path_tree | Node tree over the 0x0B segments at rec+0x154, fixed up recursively by `0xBF6A40` |
| 0x0D | `0x4AE380` collision_shapes | Havok collision (`MOPP` in the data): 0x28-byte entries with vec3 lists (rec+0x158/+0x15C), registered with `0x6F9590` → rec+0x160 |
| 0x0E | `0x4AE460` water_volume_geometry | **Water volumes**: N meshes (rec+0x164/+0x168), 16-byte vertices, u16×3 triangles |
| 0x0F | `0x4AE510` render_materials | Render materials (texture references such as `missing-alpha.tga`, `flat-normalmap.tgn`; avg 292 KB): rec+0x174/+0x178; advances a gpu-side offset |
| 0x10 | inline | count/pointer at rec+0x180/+0x184 |
| 0x11 | `0x4AE590` tree_instances | **Tree placements** (`TP09A_161_Tree34`, …): 0x54-byte entries (rec+0x188/+0x18C): name, model hash, position, orientation, scale, distances; names in the tail (rec+0x190) |
| 0x12 | `0x4AE620` | Tagged `DTNC` data: rec+0x1AC if `0x70D740` accepts it |
| 0x13 | `0x4AE640` | Tagged `LTNC`/`ITNC` data: rec+0x1B0/+0x1B4 |
| 0x14 | `0x4AE690` | 0x3C-byte entries (rec+0x194/+0x198) |
| 0x15 | `0x4AE6F0` water_volume_info | Per water volume: descriptor with counts/arrays and a name such as `watervol_001` (rec+0x16C) |
| 0x16 | `0x4AE7E0` water_volume_havok | Per water volume: Havok MOPP object from `0x7281D0` (rec+0x170) |
| 0x17 | `0x4AE8C0` floors | Floor surfaces named by texture (`…_hangar_floor1.tga`); probably reflective floors: 0x34-byte entries (rec+0x1A4/+0x1A8), chunk id stored in each |
| 0x18 | `0x4AE990` slates | Named slates (`slate_srhq_landing`, `slate_adult_store`, …): 0x34-byte entries (rec+0x1B8/+0x1BC); `0x6C7E90` in some modes |
| 0x19 | — | not handled |
| 0x1A | `0x4AEA20` volumes | 0x84-byte volumes (rec+0x1C0/+0x1C4): AABB, flags (8, 0x200), mesh; point-in-volume tests for audio, teleport, vehicle entry, npc respawn (interiors or water, unverified) |
| 0x1B | `0x4AEAB0` | count/array rec+0x1D4/+0x1D8 |
| 0x1C | `0x4AEAE0` object_attachments | 0x48-byte entries (rec+0x1DC/+0x1E0): object, offset, orientation, handle into `0x2AE50E8`; moved with their object each frame (`0x4AB140`) |
| 0x1D | — | not handled |
| 0x1E | `0x4AEB10` cell_quadtrees | 12×12 grid of 30-unit cells (rec+0x204/+0x208), each a quadtree of 20-byte nodes {min/max height, 4 children, leaf index}: a height/surface lookup |
| 0x1F | `0x4AEB80` | count/array rec+0x20C/+0x210 |
| 0x20 | `0x4AEBB0` | 12-byte {object, hash → `0x21F2AA0` entry, hash → `0x27923E0` index} (rec+0x214/+0x218) |
| 0x21 | `0x4AECB0` | 0x38-byte entries with arrays and a mesh (rec+0x21C/+0x220) |
| 0x22 | `0x4AEDB0` | rec+0x224 |
| 0x23 | `0x4AEF00` object_runtime_data | per-object 8-byte array (rec+0x54/+0x58); all zeros in the files (runtime storage) |
| 0x24 | `0x4AEF60` object_runtime_data | per-object array (rec+0x230/+0x234); all zeros in the files (runtime storage) |
| 0x25 | `0x4AEFA0` object_anims | Per-object frame animations (rec+0x238/+0x23C): name, object (flagged +0x34 \|= 0x40000), frame table into rec+0xA0; `0x91C6C0` plays them (flags 1 playing, 2 ping-pong, 4 loop, 8 reversing) |
| 0x26 | `0x4AEDD0` speedtree_models | SpeedTree models, a few per chunk (the placements are 0x11) (rec+0x228/+0x22C): hash → SpeedTree table `0x26B5DE8` (count `0x2527288`), the table the SpeedTree loader `0xA8DAFB` fills |

Versions below 0x78 copy some arrays into the cpu pool instead of pointing into the file (sections 0, 0x0E, 0x15, 0x16). The loader only accepts 0x79, so those paths are dead for shipped data.

Identified from their consumers (2026-10-30). The loader adds a chunk to a per-feature list when a section is present, and each list's walker is a direct consumer:
- **0x04 render items:** `chunk_render_items_collect_visible` (`0x5239A0`) is called from `game_render.cpp`. It walks the node tree with a visibility test and collects the visible 0xC8-byte items.
- **0x1A volumes:** `chunk_point_in_volume_flag8` (`0x4AB430`, 20 callers including `luafn_teleport` and vehicle entry), `audio_point_in_chunk_volume` (`0x46F770`, audio modes 5/6) and `chunk_point_in_volume_flag200` (`0x4AAB50`). All do an AABB check, then a mesh test (`0xBFD510`).
- **0x1C object attachments:** `chunk_object_attachments_update` (`0x4AB140`, from `sim_frame`) moves table entries at `0x2AE50E8` with their objects.
- **0x25 object animations:** `chunk_object_anims_update` (`0x91C6C0`, from `sim_frame`).

The chunk record's bounding box is rec+0x28..+0x3C (min x/y/z, max x/y/z, from header words 0x35..0x3A).

Still unidentified: sections 0x01, 0x02, 0x07, 0x08, 0x0A (probably roads), 0x0C, 0x0D, 0x0F, 0x11–0x14, 0x17, 0x18, 0x1B, 0x1E–0x24. They are not on a per-feature list, so finding their consumers means searching for reads of their record offsets.
Searching for reads of the remaining record offsets in functions that touch chunk records gives false positives. The same small offsets are fields of other objects (for example `general_mover_init`, `0x9F9EA0`, reads mover fields, not the chunk record). The better next step for those sections is to dump them from the shipped `.chunk_pc` files (several carry name strings) and identify them by content.

### Section data from the shipped chunks

`cargo run --release -p sr2-formats --example chunk -- stats <game>/chunks1.vpp_pc ... chunks4.vpp_pc` parses all 751 chunks in those archives (about 16 s). It reports, per section type, how many chunks have it, the sizes, the first u32 and the most common strings. `chunk dump <archive> <chunk>` lists one chunk's sections, and `chunk hex <archive> <chunk> <kind>` hex-dumps one section. The parser is `sr2_formats::chunk`; the section table is at the end of the file (`count` × 12 bytes, count in header word 4).

Identified from the data:
- 0x03 player starts.
- 0x05 collision models (magic `MCKH` in all 751).
- 0x07/0x08/0x0D/0x16 Havok MOPP collision.
- 0x0E/0x15/0x16 water volumes (`watervol_001`).
- 0x0F render materials (texture references).
- 0x17 floors named by texture (probably reflective).
- 0x18 slates.
- 0x25 flipbook animations (`flipfloatc01`, `ai_bannerfb08`, `flagflip`, `windsock_fb`), which confirms the consumer analysis.

Sections 0x12/0x13 carry the magics `DTNC` and `LTNC`/`ITNC`. The objects section also contains material/texture names.

Sections present in every chunk: 0x00, 0x05, 0x22, 0x25. Never seen in these archives: 0x02, 0x19, 0x1C, 0x1D. The header words 0x35..0x3A give boxes that look too large for single chunks (e.g. sr2_chunk161: x −4..1340, z −1751..2.7), so treat the bounding-box reading as unverified.

Still unidentified by data (no strings): 0x01, 0x04, 0x06, 0x09, 0x0A, 0x0B/0x0C (they share data; 0x0C is a tree over 0x0B), 0x10, 0x11, 0x14, 0x1A, 0x1B, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24.

### Control flow (PUMP_RUST)

The pump (`0xA7D6D0`), `process_generic` (`0xA7F100`), `poll_active` (`0xA7F0C0`), `wait`
(`0xA7D600`), `unload` (`0xA7D4F0`), `abort_current` (`0xA7CE70`) and `batch_release` (`0xA7DCD0`)
are in Rust too. They call I/O and virtual functions, so they can't be rewound. `PUMP_RUST`
switches between the port and the originals, and both modes write the same distinct events
(pump transitions, waits, unloads, aborts) to `sr2-streamman.log` in the game folder.

- Boot plus save load, original vs Rust: identical event sets (624 chunk-geom loads, peg and
  extras loads, 639 waits all loaded), except the counts of ambient vehicle/character loads
  (types 2 and 3), which vary between runs.
- 36,000 frames walking out of the crib into Copperton on the Rust pump: no problems, and
  the queue checks stayed at 0 mismatches (559k calls).
- Teleport test (`TELEPORT_TEST` in hooks.rs, `crates/sr2-hook/src/teleport.rs`,
  `tools/ingame/run_teleport.ps1`): after the save loads, the hook teleports the player 600 m at
  a time around a square loop (8 jumps, every 1200 frames), the way `luafn_teleport` does. On
  the Rust pump: 27 aborts (chunk extras, slots 4:0 and 4:1, cancelled with their batch, so
  `batch_release` runs too), 0 queue mismatches over 668k calls, and the game kept running.
  The original gives the same kinds of events (32 aborts; counts depend on how far reads got
  before each jump).
- Still not exercised: `unload` (`0xA7D4F0`). No calls in any run, including the teleports.

`abort_current` cancels a reading slot in mode 1 when its info's destination offset (+8) is
above 2 GB, else mode 2. It cancels the 64-bit read id at +0x18/+0x1C if set (`0xA74150`, then
`0xA74270` with the mode), otherwise the batch at +0x20.

### start_load

`streamman_start_load` (`0xA7DDE0`) is in Rust as well, detoured because the chunk-extras
start (`0xA7E100`, at `0xA7E791`) also calls it. Per read step it runs the filter and begin
callback, aligns the destination cursor, gets the file size (`file_get_size`, `0xBFE0E0`,
ESI = out), takes a read node and runs the issue/after callbacks. Read nodes hold
`{prev, next, +8 file, +0xC 0, +0x10/+0x14 io id, +0x20 dest, +0x24 size, +0x28 priority,
+0x2C step}`; priority is 0x70 for chunk geom/peg and 0x40 otherwise. Each node goes to
`async_io_submit` (`0xA73290`, stdcall `ret 0xC`) as an 8-dword descriptor
`{file, 0, 0, size, dest, priority, 0, 0}`.

Quirk kept from the original: if the step count is 0, or every step is skipped, the batch has
already been taken off the free list and is never put back, and the slot stays in state 1.

The teleport test logs one event per start (type, read count, total bytes, priorities). Original
vs Rust: identical for every chunk geom (627), peg (23), type 5 (16) and type 7 (2) start. Only
the ambient vehicle/character loads (types 2 and 3) differ, and those vary from run to run.

Decoded from the data (2026-10-30; numbers from sr2_chunk161 and sr2_chunk060, read with the chunk example):
- 0x01 lights: warm-white colour (1.0, 0.957, 0.498), positions inside the chunk, ranges 2/5 and fade 150.
- 0x0B: 1,046 chained ~3-unit segments at constant ground height in a waterfront chunk, so a path network. Pedestrian, vehicle or navigation paths: not settled.
- 0x11 vs 0x26: 75 tree placements (`TP…_TreeNN`) against 2 SpeedTree entries, so 0x26 lists the models and 0x11 places them (0x26 was labelled "instances" before).
- 0x1E: per-cell quadtrees with min/max heights (−6.03 = sea level here) and leaf indices.
- 0x23/0x24: zero-filled per-object storage.
- Undecided: 0x09 (224 entries with position, orientation, two 30° angles and distances 5.05/3.0: cones of some kind), 0x0A (78 local transforms with ids 43xx, grouped by offset lists: attachment points?), 0x10 (19 hashed placements with orientation), 0x14, 0x1B, 0x1F, 0x20, 0x21, 0x22.

### check_batch

`streamman_check_batch` (`0xA7ED40`) is in Rust as well, detoured so that the original run logs
the same events. It polls the pending read nodes in order with `async_io_status`:
- 1 (queued): stop.
- 2 (reading): the step's progress callback (+0x230, `(info, node)`; false aborts), then stop.
- 3 (done): for chunk pegs, the after callback (+0x238; false aborts); then the node moves to
  the done list (+0x10).
- 5 (failed): the retry callback (+0x240) must return 1, otherwise the slot is aborted.

With nothing pending, it runs `finish_current` (`0xA7CFC0`, the completion, still original)
and frees each request with `async_io_free` (`0xA73650`, stdcall, mode 0). When a request's
sequence no longer matches, it first copies `*0x2529E24` to `0x2529E28` (a debug last-error
value). The nodes go back to the pool `0x252A020` (count `0x252A024`) and the batch to the free
list. Both `async_io_submit` and `async_io_free` are stdcall, not cdecl.

Teleport test, original vs Rust: identical completions (647 chunk geom, 23 peg, 16 type 5 and
2 type 7 slots, each from reading to loaded) and identical aborts (32 each). Outside the
random ambient vehicle/character loads, the two event logs are the same.


## Batch cleanup helpers and the streamed mesh cache

Callees of the ported functions that were still unnamed:

- `streamman_batch_release` (0xA7DCD0) runs `streamman_batch_cancel_pending` (0xA7D9C0), which calls `async_io_cancel_impl` on every pending node. It then runs `streamman_batch_free_pending` (0xA7DA30) and `streamman_batch_free_finished` (0xA7DB30) with the mode. Both call `async_io_free(request->mgr, request, sequence, mode)` on each node of their list (+0xC and +0x10), clear node +0x10/+0x14 and return the node to the pool 0x252A020.
- The stores to 0x2529E28 in those loops (from `*0x2529E24` when a node's request is null or its sequence does not match) are debug checks with no effect on the logic.
- `streamman_read_nodes_free_list` (0xA7DD50, EAX = list) returns a whole node list to the pool. `start_chunk_extras` uses it to drop its temporary list.
- `path_change_extension` (0xBFF930, ECX = src, stack out/size/ext) is used by `start_chunk_extras` and `check_batch` to build the part file names. It returns false if the result would not fit.

`start_chunk_extras` and the chunk-extras request path go through a **streamed mesh cache** (0x6E6310-0x6E6CB0). Records are keyed by the object template (`object_template_find` 0x931060: table 0x2BC5B00, 0x90 bytes each, found by name hash).

| Off | Mesh cache record |
|---|---|
| +0x00, +0x04 | links |
| +0x08, +0x49, +0x8A, +0xCB | file names (0x41 each): .smesh_pc, .g_smesh_pc, .peg_pc, .g_peg_pc (the g_ ones are emptied if the file does not exist) |
| +0x10C | mesh object (vtable); its +0x54 is the peg object |
| +0x110 | key: object template |
| +0x114 | u8 pool (template +0xD) |
| +0x116 | u16[6] chunk ids that reference it, 0xFFFF free |
| +0x122 | u8 loaded |
| +0x124.. | part data set by `streamman_poll_chunk_extras` |

There are two pools. Pool 0 uses free list 0x15125A0, active list 0x151259C and free count 0x2527D64. Pool 1 uses 0x1512598, 0x147A514 and 0x2527D68. Everything is skipped when 0x2528B48 == 1 and 0x2528B49 == 0.

- **Request:** `smesh_cache_filter_requests` (0x6E6B80) drops objects whose mesh is already resident and adds the chunk to existing records.
- **Start:** `start_chunk_extras` calls `smesh_cache_add_ref` (0x6E63F0). If the pool is full, it evicts an unreferenced record (`smesh_cache_find_unreferenced` + `smesh_cache_free`). For a record already loaded, it only adds the chunk id (`smesh_cache_record_add_chunk` 0x6E6B30).
- **Unload:** `streamman_release_chunk_extras` (0xA84410, chunk id) calls `smesh_cache_release_chunk` (0x6E6720) for each object of the chunk. The last reference to an unloaded record frees it.

## process_sequential and poll_chunk_extras, in detail

### Read request state names

The table at `0xE87BEC` names every read request state (+0x2C), so `async_io_error_code` returns either a state or 10/11. The names are:

| Value | Name |
|---|---|
| 0 | UNKNOWN |
| 1 | UNUSED |
| 2 | PENDING |
| 3 | ACTIVE |
| 4 | CANCEL_PENDING |
| 5 | CANCELED |
| 6 | DONE_SUCCESS |
| 7 | CANNOT_OPEN |
| 8 | VDIR MISMATCH |
| 9 | ERROR_UNKNOWN |
| 10 | NOT_ENOUGH_MEMORY |
| 11 | INVALID_HANDLE |

State 8 was the one left unexplained in the read scheduler table above.

### streamman_process_sequential (0xA7D0C0)

Called by the pump for type 6 (animation). It takes no arguments and works on the current slot `*0x2529FF0`, with info = slot+0x10. There is no batch: one read is in flight at a time, kept at slot +0x18/+0x1C.

**Slot state 1.** Starting at the current step (flags bits 0-9), it looks for a runnable step, then stops at the step count (bits 10-19):
- If the info has a filter (+0x10), it runs it on the step's file and stops at a step where it returns 1.
- Without a filter, it stops at a step whose file is non-zero.

If no step is runnable, it calls `streamman_finish_current` and returns.

Otherwise:
1. Store the step index in bits 0-9 and call `streamman_nop(step file)`.
2. Run the begin callback (+0x22C). If it returns false, call `abort_current(0)`.
3. Unless flag 0x100000 is set, round info+8 up to the step's alignment (+0x214), take the same amount off info+0xC, and store info+8 in step +0x218.
4. Call `streamman_submit_single_read` (0xA7CF50) with EAX = step file, ECX = info+8, EDI = info+4 and stack info+0xC. Store the returned (request, sequence) in slot +0x18/+0x1C.
5. Run the issue callback (+0x234) if set, and set slot state 2.

**Slot state 2.** Take the step at bits 0-9 and call `streamman_nop(file)`, then poll `async_io_status(slot+0x18, slot+0x1C)`. Note that `file_get_size(file)` is called here and its result ignored.
- **Status not 3, 4 or 5 (still running).** If the progress callback (+0x230) is set, call it with `(info, 0)`; false aborts. With no callback it just returns.
- **Status 4 or 5, or 3 followed by a re-poll that is not 3.** Run the retry callback (+0x240). Unless it returns 1, call `streamman_nop(file, name)` and `abort_current(0)`. The name comes from the table above via `async_io_error_code`, or is "No servicing request" when there is no current slot.
- **Done.** Run the after callback (+0x238); false aborts. Then:
  1. Call `async_io_release(request, sequence, 0)` and clear slot +0x18/+0x1C.
  2. Unless flag 0x100000 is set, add step +0x210 (read size) to info+8 and take it off info+0xC.
  3. Advance the step:
     - If step +4 (u8 multi-part) is 0, go to the next step.
     - If it is set, advance only once +0x208 <= +0x20C. Then keep skipping following steps that are themselves multi-part with +0x208 <= 0.
     - If the step is still the same afterwards, the step is re-issued (state 1); this is how a multi-part step reads its parts one at a time.
  4. If the new step index differs from the step count captured at entry, set slot state 1 (the next pump issues the next read). Otherwise call `streamman_finish_current`.

### streamman_poll_chunk_extras (0xA7E900)

Takes `(batch*)`. info = batch+0x14 and slot info = info+0x10, holding chunk id +0x772 and chunk pool +0x778. It walks the pending nodes (batch+0xC) in order and checks `async_io_status`:

| Status | Action |
|---|---|
| 1 (queued) | stop |
| 2 (being read) | `streamman_nop(name)`, where name is `path_change_extension(file, suffix)` if node+0xC is set, else the file; then stop |
| 3 (done) | by node kind (+0x2C), see below; then move the node to the finished list (+0x10) |
| 5 (failed) | move the node to the finished list, nothing else |
| 4 or other | leave it and go on to the next node (no move) |

Node kinds for status 3:
- **0, mesh/peg part.** Look up the record with `object_template_find(name)` then `smesh_cache_find`. If the record is not loaded yet (+0x122), store the data by suffix (stricmp against the table `0xE87B90`):

  | Suffix | Stored at |
  |---|---|
  | .smesh_pc | +0x124 |
  | .g_smesh_pc | +0x128 |
  | .peg_pc | +0x12C (data) / +0x130 (size) |
  | .g_peg_pc | +0x134 / +0x138 |

  The object is complete when the node's raw next link (+4) is 0, or the node it points to has a different object (+0x30). The link is not the next-or-0 value used for the walk, so in a one-node circular list (+4 points to itself) the object never completes (corrected from the main session's port):
  1. `smesh_table_add_streamed(+0x124, +0x128)`.
  2. If there is a g_peg (+0x134 non-zero), call `0xC08B80` with ESI = rec+0x12C (peg data), ECX = rec+0x8A (peg name) and stack (rec+0x130, rec+0x134), caller-clean; it registers the loaded peg. (`smesh_table_add_streamed` in step 1 gets ECX = node+0x30 and EDX = rec+8, stack (+0x124, +0x128).)
  3. Set the record's loaded flag (+0x122 = 1).
- **1, anim.** Call `object_anim_add_loaded(data, size, chunk_pool_index(pool))`.
- **2, text node file.** Call `streamman_chunk_extras_parse_nodes(node, chunk id)`.

When the pending list is empty:
1. Call `streamman_nop()`.
2. Free every finished node: `async_io_free(mgr, request, sequence, 0)`, clear +0x10/+0x14, back to the pool 0x252A020.
3. Clear info+0x20 (the active batch).
4. Unlink the batch from the active list `0x252A01C` and put it on the free list `0x252A018`.
5. Call `streamman_finish_current`.

Unlike `check_batch` there is no progress, after or retry callback, and a failed part is only dropped.

### finish_current and start_chunk_extras in Rust

`streamman_finish_current` (`0xA7CFC0`) and `streamman_start_chunk_extras` (`0xA7E100`) are in Rust
too, both detoured (`finish_current` has 4 callers). The teleport test logs one event per finish
(result, state, the replayed byte counts at slot +8/+0xC) and per extras start (reads and bytes
per kind: object parts, anims, file). Original vs Rust: identical non-ambient event logs (1,030
distinct events each). The only difference was one extras start that landed on slot 4:1
instead of 4:0, because the free extras slot depends on timing.

Extras start details that the port keeps:
- Every part repeats the template/record lookup.
- A record that is already loaded only gets the chunk added and nothing is read.
- A missing or too-big mesh (parts 0/1) drops the object's nodes (`streamman_read_nodes_free_list`).
- Missing pegs are skipped.
- An object's nodes are spliced onto the batch only after all four parts.
- With nothing queued, the batch goes back on the free list and the slot stays requested, so it
  is retried on the next pump.


## Read scheduler API, exact behaviour (for porting)

Manager `mgr = *(*0xEB80E0)`. Request records are 0x48 bytes (layout in the read scheduler section above), 256 of them at mgr+0x40. Lists are circular and doubly linked through request +0x40 (prev) / +0x44 (next). The writes to `0x2529E28` from `*0x2529E24` scattered through these functions are debug checks only; they change no state.

| Function | Convention | Returns |
|---|---|---|
| `async_io_submit` 0xA73290 | stdcall `ret 0xC`: (mgr, desc*, u8 attach_event) | EAX = request, EDX = sequence (after the bump) |
| `async_io_status` 0xA741A0 | cdecl (request, sequence) | EAX: 0 for NULL or state < 2; state 2 → 1; 3, 4 → 2; 6 → 3; 5 → 4; 7, 8, 9 → 5; anything above 9 → 0 |
| `async_io_cancel` 0xA74150 | cdecl (request, sequence) | none; calls `async_io_cancel_impl` (EDI = request->mgr, stack request, sequence) |
| `async_io_release` 0xA74270 | cdecl (request, sequence, mode) | none; calls `async_io_free(request->mgr (+0x3C), request, sequence, mode)` |
| `async_io_free` 0xA73650 | stdcall `ret 0x10`: (mgr, request, sequence, mode) | none |
| `async_io_error_code` 0xA74340 | cdecl (request, sequence) | index into the state-name table `0xE87BEC` |
| `async_io_set_state` 0xA739A0 | EAX = request, stack (mgr, state), caller-clean | none |

A wrong sequence does not change the result anywhere; it only reaches the debug check. `async_io_status` still reads the state.

Correction: the read scheduler section above calls `async_io_submit` cdecl. It is stdcall (`ret 0xC`), as in the table here.

### Submit

The whole of submit runs under mgr+8:
1. Take the head of the free list (+0x30). There is no empty check: with all 256 in use it dereferences NULL.
2. Unlink it, sequence++, and clear +0x24, +0x28, +0x30, +0x3C and the u8 +0x38. Set state 1.
3. Copy the 8 description dwords to +0x04..+0x20 (file name, suffix, offset, size, destination, priority byte at +0x18, +0x1C, +0x20) and set +0x3C = mgr.
4. Set state 2 with `async_io_set_state`.
5. If `attach_event` is set, call `hdd_attach_wait_event`.
6. Insert into the pending list (+0x34) before the first request whose priority byte is lower, so equal priorities keep FIFO order. If it becomes the highest priority, it becomes the head.
7. SetEvent(mgr+0).

### Free

1. If `request` is NULL, return.
2. Under the lock, the debug flags `0x2529DD5` / `0x2529DD6` force mode 2 / 1.
3. In flight means state 2, 3 or 4.
   - Normally (`0x2529DD7` clear) with an in-flight request: mode 0 or 1 only sets released (+0x38 = 1) and returns. `hdd_finish_current` recycles the record when the read ends. Mode 2 waits with `async_io_wait_done`, outside the lock, then carries on.
   - With `0x2529DD7` set, it polls with Sleep(1) until the request is no longer in flight.
4. Remove it from the finished list (+0x38), sequence++, state 1, append to the tail of the free list, released = 1.

### State flow

- Submit → 2 (PENDING).
- `hdd_service` takes the pending head → 3 (ACTIVE).
- The end result is one of:
  - 6 DONE_SUCCESS: read OK, read-cache hit, or the debug player.cvtf copy.
  - 7 CANNOT_OPEN: `file_open` returned NULL.
  - 5 CANCELED: cancelled while queued, cancelled before the read, or file error 3.
  - 9 ERROR_UNKNOWN: any other file error.
- `async_io_cancel_impl` on a request that is ACTIVE sets 4 (CANCEL_PENDING) and aborts the file read; the thread then finishes it as 5.
- Free → 1 (UNUSED).

`async_io_set_state` SetEvents the one-shot waiter (+0x30, then clears it) and the attached event (+0x34) whenever the new state is not 2, 3 or 4.

### hdd_service read path (0xA73C50, ECX = mgr)

Each request is taken under the lock as above. If state is already 4 → finish with 5. Then:
1. **Debug copy.** `0x2529DDB` and name "player.cvtf" → memcpy from `0x2223CC0`, then 6.
2. **Read cache.** `read_cache_lookup` (0xA74A40, ESI = request+4) → served from memory, then 6.
3. **Open.** `file_open(ECX = name, "rb", 0x2529DDA == 0)`. NULL → 7.
4. **Seek.** If state is 4 now: under the lock, `file_close` and finish with 5. Otherwise, if offset (+0xC) is non-zero, seek inline in the file handle:
   - type +0x148 == 2 (memory): set position +0x15C;
   - type 0 (OS file) with no packfile (+0x150 == 0): `os_file_seek(+0x14C, offset)` and on success set +0x15C;
   - type 1 (packfile): no seek.

   A packfile-backed handle (+0x154 == 1) only seeks if offset <= +0x160.
5. **Read.** `file_read(EAX = size (+0x10), dest (+0x14), handle)` → +0x28. Then `read_cache_store` (EAX = request+4), and `0xBFE600(handle)` unless `0x2529DDA` is set.
6. **Close.** Under the lock, `file_close` and +0x24 = 0. The handle's error field +0x158 gives the state: 0 → 6, 3 → 5, else 9.

There is no decompression in the scheduler. Compressed packfile entries are handled inside `file_open` / `file_read`, which are already Rust.

**Read cache** (0xA745F0-0xA74AB0, CS `0x2F9B1E4`): an optional in-memory cache.
- `0x2529E34` is the cache, set or cleared by `0xA74940` / `0xA749E0`, which `0x7F6F50` calls. `0x2529E2C` is its allocator.
- It holds up to 0x20 entries of {name, data, size} at cache+0x28 (count at +0x1A4).
- Only names passing `read_cache_accepts` (prefix list) are stored. A hit skips the disk read entirely.

## Resource slot scheduler (0xA75030-0xA79C00): the other streamman_queue users

The ~20 queue calls in `0xA75900`-`0xA77E90` are not part of the I/O scheduler. They belong to a **resource slot scheduler**. It decides which characters, vehicles and animations stay resident in a fixed number of memory slots (spawning, "multi registration", notoriety groups and component streaming call it). It feeds the streamer through its own job queue. Names use the `rsched_` prefix.

**Six schedulers** at `0x2F9B2E8` (0xD4 bytes each, kind = index = id >> 24), set up once by `rsched_init_all` (0xA78E60) from a config table built on the stack at 0x51F700. The config is 0x30 bytes per kind:

| Off | Config field |
|---|---|
| +0x00, +0x04 | buffer base A and per-slot stride A |
| +0x08, +0x0C | buffer base B and stride B |
| +0x10 | slot count |
| +0x14 | request count |
| +0x18 | group count |
| +0x1C | u8 flag |
| +0x20..+0x2C | callbacks, copied to `0x22248F8 + kind*0x10` |

The values set at boot:

| Kind | What | Slots | Requests | Groups | Buffers |
|---|---|---|---|---|---|
| 0 | characters | 10 | 0x40 | 9 | A `*0x2527338` stride 0x129000, B `*0x252733C` stride 0x400000 |
| 1 | vehicles | 8 | 0x20 | 1 | A `*0x2527324` 0x400000, B `*0x2527328` 0x300000 |
| 2 | characters (no buffer) | 2 | 2 | 1 | none |
| 3 | characters (no buffer) | 10 | 10 | 1 | none |
| 4 | animations | 1 | 0x80 | 1 | A `*0x2527340` 0x400 |
| 5 | vehicle | 1 | 1 | 1 | A `*0x252732C` 0xC8000 |

The callbacks per kind (`0x22248F8 + kind*0x10`):

| Off | Callback |
|---|---|
| +0x0 | (cfg+0x20) |
| +0x4 | available: a slot became loaded (`rsched_job_finish_loaded`) or idle (`rsched_slot_make_idle`) |
| +0x8 | load failed (`rsched_job_finish_failed`) |
| +0xC | unload (`rsched_slot_evict`) |

| Off | Scheduler (0xD4) |
|---|---|
| +0x00 | slot count |
| +0x04 | group count |
| +0x08 | slots (0x3C each) |
| +0x0C | idle-loaded slot list |
| +0x10 | loading slot list |
| +0x14 | slot being loaded by the current job |
| +0x18 | in-use slot list |
| +0x1C | free slot list |
| +0x20..+0x2C | buffers from the config |
| +0x30 | request count |
| +0x34 | requests (0x48 each) |
| +0x38 | active request list |
| +0x3C | free request list |
| +0x40 | per-group request queues (0x18-byte `streamman_queue`, sized by the request count, compare `rsched_request_compare` 0xA752A0) |
| +0x44 | per-group slot queues (sized by the slot count, compare `rsched_slot_compare` 0xA751E0) |
| +0x48, +0x4C.. | deferred releases (count, 0x10-byte {id, level, all, group}) |
| +0xCC | per-group slot quota |
| +0xD0 | flags: 1 disabled, 2 dirty, 4 config flag, 8 updating |

| Off | Slot (0x3C) |
|---|---|
| +0x00 | bound request |
| +0x04 | id (-1 none) |
| +0x14 | float time |
| +0x18 | object using it |
| +0x1C | job index (-1) |
| +0x20 | queue node |
| +0x24 | group |
| +0x28 | state: 0 loaded/idle, 1 loading, 4 free |
| +0x2C, +0x30 | links |

| Off | Request (0x48) |
|---|---|
| +0x00 | bound slot |
| +0x04 | id |
| +0x08 | group count |
| +0x0C | object |
| +0x10 | queue node |
| +0x14 | per-group counts |
| +0x18 | u8 flags (1 evaluated, 2 load now) |
| +0x1C, +0x20 | links |
| +0x24, +0x28 | temp links |
| +0x2C..+0x44 | counts per level 0..6 (best level = highest non-zero) |

**Flow:**
1. **Request.** Game code calls `rsched_add_ref(EAX = sched; id, level, group, load_now)` and `rsched_release(EAX = sched, EDX = id; level, group, all)`. Both mark the scheduler dirty.
2. **Update.** Every frame `rsched_update_all` (0xA79080) runs `rsched_update` on each scheduler: `rsched_assign_group_quotas`, then `rsched_update_group` per group. That pairs the best requests with the worst slots, evicting through `rsched_slot_evict`. Each slot it starts loading becomes a **job**: `rsched_job_add(id, A + i*strideA, strideA, B + i*strideB, strideB, level)`.
3. **Jobs.** Jobs are 0x20 bytes {id, dest, size, dest2, size2, level, links}, at most 50, in the `streamman_queue` at `0x2F9B2D0`. Only one is current at a time (`0x2529E58`, streamer handle `0xEB80F8`).
   - `rsched_job_start_next` requests it from the streamer by kind: character (type 3), vehicle (type 2, `streamman_request_vehicle` 0xA87F80) or animation (type 6, `streamman_request_animation` 0xA88240).
   - `rsched_job_pump` waits for the streamer slot to go idle, then calls `0xA78950` if it is loaded (state 4) or `0xA789B0` if not, frees the job and starts the next.

**Resource registry** (`resource_registry_init` 0xA774E0): the id's low 24 bits index `0x2529E3C`, which holds up to 1000 entries of 0xC bytes:

| Off | Registry entry |
|---|---|
| +0x00 | flags (bit 3: load failed, set by `rsched_job_finish_failed`, skipped by `rsched_update_group`; bits 4-23 name count) |
| +0x04 | name list |
| +0x08 | extra (the animation size) |

The entry count is at `0x2529E38`. Names live in a pool at `0x2529E44` (used count `0x2529E40`), and a name hash table is at `0x2F9B260`/`0x2F9B2BC`.

For a port of the streamer core, none of this needs to move. It only calls `streamman_queue_*`, `streamman_request_character/vehicle/animation`, `streamman_is_idle` and the slot arrays at `0x2225B3C`.

## Request side, step by step (for porting)

Corrections to earlier notes:
- `file_exists` (0xA7D750) takes the name in **ESI**, not ECX.
- `streamman_step_init` (0xA7F770) is **stdcall `ret 0x10`**: ESI = step, EAX = file, CX = alignment, stack (begin → +0x22C, issue → +0x234, progress → +0x230, after → +0x238). It also clears +0x4, +0x208, +0x20C, +0x210, +0x218, the 0x200 buffer, +0x21C bits 0-1, +0x23C and +0x240.
- The multi-part variant is `streamman_step_init_multipart` (0xA80EE0), stdcall `ret 0x18`:
  - Registers: EAX = step, ECX = part count.
  - Stack: (list*, file, u16 align, extra → +0x220, begin, after).
  - It sets +4 = 1 and +0x208 = count, copies the list into the step buffer (+8), and clears +0x20C, +0x228, issue, progress, +0x23C and retry.

The steps of a loading info are at info+0x24 + k*0x244. Step k's fields are therefore info+0x24 + k*0x244 + off. Below, "step k" means that block.

### Pattern shared by every typed request

Character, vehicle and animation are single-slot types:
1. `slot = *(0x2225B3C[type])`. If slot+4 (state) != 0 → `*out = {lo 0, hi 0xFFFF}` (none) and return.
2. Set slot +4 = 1 and clear slot +8 / +0xC. `info = slot+0x10`.
3. `info+0x20 &= 0xFF8FFC00`: clears the current step and bits 20-22. The step count (bits 10-19) is preset by the info's constructor and is **not** set here.
4. Fill the info fields and steps (below).
5. If slot+0x14 != -1, `streamman_queue_remove(EAX = slot+0x14, EDX = 0x2F9B88C)`. Then `slot+0x14 = streamman_queue_push(ECX = 0x2F9B88C, slot)`.
6. `*out = type << 16 | index`.

The chunk types (0, 1, 4) search their slot array for the first slot in state 0, 3 or 4 instead (peg: `0xE87BB4` slots; extras: `0xE87BC0`). Geom (type 0) always uses slot 0 and does not check its state.

### streamman_request_character (0xA7FC30)

Type 3, slot `*0x2225B48`, returns 0x30000.
- **Registers:** EAX = dest A, EDX = size A.
- **Stack:** (out, f0, f1, base, f2, f6, f8, f7, dest B, size B, object), i.e. arg0 = out ... arg10 = object.
- `rsched_job_request_character` passes f0/f1/base as the registry entry's first three names, then the .morph_pc, .pcm_pc, .sim_pc and .cvtf names it finds in the list.

Info fields:

| Off | Value |
|---|---|
| +4 | 0 |
| +8 | dest A |
| +0xC | size A |
| +0x10 | 0 (no filter) |
| +0x18 | object (arg10) |
| +0x1C | `character_on_loaded` (0xA7FA40) |
| +0x1750, +0x1760 | dest A (base, cursor) |
| +0x1754, +0x1764 | dest B (arg8) |
| +0x1758 | size A |
| +0x175C | size B (arg9) |
| +0x1768 | u8: (`0x2527D14` && `0x2527CF7`) or `0x2527D15`, and `0x6C9090(ECX = arg1)` |

Steps (all with begin `character_before_read_began_cb` 0xA7F840, after `character_after_read_cb` 0xA7F9C0, no issue or progress):

| Step | File | Align | Notes |
|---|---|---|---|
| 0 | arg1 | 0x40 | |
| 1 | arg2 | 0x1000 | only if `file_exists(arg2)`, then +0x21C bit 0; else an empty step (all 0) |
| 2 | arg4 | 0x10 | then `file_get_size(ESI = &step2+0x210, arg4)` if arg4 != 0 |
| 3 | info+0x170D = `path_change_extension(arg3, ".g_morph_pc")` | 0x10 | only if it exists (bit 0); else empty |
| 4 | arg3 | 4 | |
| 5 | info+0x16CC = `path_change_extension(arg3, ".g_peg_pc")` | 0x40 | only if it exists (bit 0); else empty |
| 6 | arg5 | 0x10 | |
| 7 | arg7 | 4 | |
| 8 | arg6 | 4 | |
| 9 | 0 | 0 | empty, no callbacks |

Quirk: the begin and after callbacks return **false for step 3**. If a `.g_morph_pc` exists, the load aborts at that step.

### streamman_request_vehicle (0xA87F80)

Type 2, slot `*0x2225B44`, returns 0x20000.
- **Registers:** EDX = size A, ECX = dest B.
- **Stack:** (out, files[11]*, dest A, value → +0x191C, object).

Info fields:

| Off | Value |
|---|---|
| +4 | 0 |
| +8, +0x1910, +0x1920 | dest A |
| +0xC, +0x1918 | size A |
| +0x1914, +0x1924 | dest B |
| +0x191C | arg3 |
| +0x18 | object |
| +0x10 | filter `vehicle_step_filter` (0xA87AF0): file non-NULL and not starting with '!' |
| +0x1C | `vehicle_on_loaded` (0xA87BF0) |

Steps (begin `vehicle_before_read_began_cb` 0xA87B10, after `vehicle_after_read_cb` 0xA87BB0), in the order they are set up:

| Step | File | Align | Notes |
|---|---|---|---|
| 0 | files[0] | 0x40 | |
| 1 | files[1] | 0x1000 | |
| 9 | files[9] | 4 | set up third |
| 2 | files[2] | 4 | |
| 3 | files[3] | 0x20 | |
| 4 | files[4] | 0x20 | only if `file_exists(ESI = files[4])`, else empty with CX = 0x20 |
| 5 | files[5] | 0x20 | |
| 6 | files[6] | 0x20 | only if it exists |
| 7 | files[7] | 0x20 | |
| 8 | files[8] | 0x20 | only if it exists |
| 10 (+0x16CC) | files[10] | 0x40 | only if it exists, then +0x21C bit 0 = NOT bit 14 of `object->+8->+0x5B0` (bit 0 selects buffer B) |

Then the queue push, then `0x4840A0(EDI = object+4)`. `rsched_job_request_vehicle` fills files[] with the registry entry's names ("Empty" for id -1).

### streamman_request_animation (0xA88240)

Type 6, slot `*0x2225B54`, returns 0x60000 (EAX = out as well).
- **Registers:** EDX = dest.
- **Stack:** (out, name, size, object).

Info: +8 = dest, +0xC = size, +4 = 0, +0x10 = 0, +0x18 = object, +0x1C = `animation_on_loaded` (0xA87C50, **always returns false**, so the slot ends in state 3 although the data was read).

Step 0: file = name, align 1, no callbacks.

### streamman_request_chunk_geom (0xA85140)

Type 0, slot 0 of `*0x2225B3C`, `*out = 0` (handle 0x00000).
- **Registers:** EAX = request struct.
- **Stack:** (out).

| Off | Request struct field |
|---|---|
| +0 | chunk_pc name |
| +4 | g_chunk_pc name |
| +8 | .pvs_pc name |
| +0xC | u16 chunk id |
| +0x10 | pool |
| +0x14 | u8 wait |
| +0x15 | u8 req+0x19 |
| +0x16 | u8 req+0x1A |
| +0x18 | pool reservation index |
| +0x1C | u8 no-peg |

1. Unless `packfile_find_entry(ECX = name)`, or `indexed_file_lookup` with a non-zero size (and `0x25272DC` clear), finds the file: `*out = {0, 0xFFFF}` and return.
2. Set info fields:

   | Off | Value |
   |---|---|
   | +0x20 | `(old & 0xFF900800) \| 0x100800`: step count 2, no destination accounting |
   | +0x52E | chunk id |
   | +0x541 | wait |
   | +0x540 | req+0x19 |
   | +0x542 | req+0x1A |
   | +0x543 | no-peg |
   | +4, +8, +0xC, +0x10, +0x18 | 0 |
   | +0x1C | `chunk_geom_on_loaded` |
   | +0x534 | pool |
   | +0x53C | reservation |
   | +0x530 | 0 |

   If reservation == -1: `pool+0x48 = pool+0x44` and call `pool->+0x54->vtable[+0x30]()`, which resets the GPU pool.
3. Copy the names with strncpy, 0x41 each: +0x4AC = name, +0x4ED = g_chunk_pc name.
4. Step 0: file = info+0x4AC, align 0x10, begin `chunk_geom_before_read_began_cb` 0xA84E60, issue 0xA84F80 (an empty `ret`), after `chunk_geom_after_read_cb` 0xA84F90.
5. Step 1, if `file_exists(ESI = g_chunk_pc name)`: file = g_chunk_pc name, align 0x10, begin `chunk_geom_before_read_began_gpu_cb` 0xA84F00, after 0xA84FC0. The after callback calls `pool+0x54` vtable +0x38 with (+0x538 - step1 size).

   Otherwise the step count becomes 1 (`(+0x20 & 0xFFF007FF) | 0x400`) and step1 file / dest are cleared.
6. If +0x540 is set: steps 0 and 1 get +0x21C |= 2.
7. Set slot state 1, push it on the queue, `*out = 0`.

### streamman_request_chunk_peg (0xA85890)

Type 1, `*out = 0x10000 | index`.
- **Stack:** (out, req*) with req = {name (.peg_pc), u16 chunk id, pool, +4 value, u8 wait}.

1. The same file-exists check as geom.
2. Take the first type-1 slot in state 0, 3 or 4 (`0xE87BB4` slots).
3. Set info fields:

   | Off | Value |
   |---|---|
   | +4, +8, +0xC, +0x10, +0x18 | 0 |
   | +0x1C | `chunk_peg_on_loaded` (0xA857A0) |
   | +0x20 | `(old & 0xFF9FFC00) \| 0x100000` |
   | +0x53C | pool |
   | +0x534, +0x538, +0x530 | 0 |
   | +0x52E | chunk id |
   | +0x540 | wait |
   | +0x4AC | name (strncpy 0x41) |

4. Step 0: file = +0x4AC, align 4, begin `chunk_peg_before_read_began_cb` 0xA85450, issue `chunk_peg_read_issued_cb` 0xA85660, after 0xA856B0. The after callback calls `0xC08DE0(EAX = step file)` if the step size > 0.
5. Then info+4 = req[3], and step 0 retry = `chunk_peg_retry_cb` 0xA856E0.
6. +0x4ED = `path_change_extension(name, ".g_peg_pc")`.
7. Step 1: file = +0x4ED, align 0x40, begin `streamman_chunk_peg_before_read_began_peg_gpu_cb` 0xA85510, progress `chunk_peg_progress_cb` 0xA85600, retry 0xA856E0.
8. Both steps get +0x21C |= 2. Set state 1 and push on the queue.

### streamman_request_chunk_extras (0xA84510) and _impl (0xA82EA0)

`0xA84510`:
- **Registers:** ECX = chunk pool.
- **Stack:** (out, chunk id, name, u16 old chunk id).

1. If streaming is off (`0x2528B48` == 1 and `0x2528B49` == 0) → none.
2. If no type-4 slot is free → none.
3. Pool index = (ECX - 0x2771894) / 0x58 must be 0..2, else none.
4. **Network-client path.** Taken when `0x2528C14` is set, (`0x2528B56` == 0 or `0x2528B2B` != 0) does not hold, or `+0x3F8` is 0. It:
   - drops any matching entries from the list `0xEB8148` (count `0xEB8150`, capacity `0xEB814C`) via `0xA84DD0`;
   - appends {chunk id, sequence `0x252A02C`++};
   - sends message 0x7E with {chunk id, sequence, pool index} through `0x814740`;
   - returns none.
5. **Local path.**
   1. If the old id != 0xFFFF: `chunk_extras_list_release(DI = old id, DL = 1)`.
   2. `chunk_extras_list_get(EAX = &count, DX = chunk id, list buf)`.
   3. `chunk_extras_list_store(EAX = count, list, chunk id, 1)`.
   4. `streamman_request_chunk_extras_impl(out, chunk id, name, pool index, list, count)`.
   5. Return EAX = out.

`_impl`:
1. Streaming off → none.
2. Take the first free type-4 slot. `info+0x20 = (old & 0xFF9FFC00) | 0x100000`, clear +4 / +8 / +0xC / +0x10 / +0x18, +0x1C = `chunk_extras_on_loaded` 0xA81620, +0x778 = 0x2771894 + idx*0x58, +0x772 = chunk id. Step 0's +0x208 and file are cleared.
3. If chunk id == 0xFFFF → none (the slot is left at state 0, info half set).
4. `chunk_extras_collect_parts` splits the object list into mesh objects and animations.
5. **Meshes.** `n = smesh_cache_filter_requests(out list, mesh list, count, chunk id)`. If n > 0: step 0 = `step_init_multipart(EAX = step0, ECX = n, list, object_template_find(first name)->+8, *(u16*)0xE8355C, 0, begin 0xA81160, after 0xA81200)`, and step 0 retry = `streamman_retry_never` 0xA81610.
6. Step 1's +0x208 and file are cleared.
7. **Animations.** If the animation count > 0: step 1 = multipart(EAX = step1, ECX = count, anim list, list[0], 0x10, extra 0x2771AA0 + idx*0x54, begin 0xA80FA0, after 0xA81040), retry 0xA81610.
8. +0x6F0 = name (strncpy 0x41, +0x730 = 0).
9. Step 2: file = name if it exists (packfile or indexed lookup), else 0; align 0x40, begin 0xA814D0, after 0xA81530, retry 0xA81610.
10. If the file exists or any parts were added: state 1, queue push, out = 0x40000 | index. Otherwise none.

### streamman_request_chunk (0xA80360)

Cdecl (out, req*).

| Off | Chunk request (req) |
|---|---|
| +0 | name used for the world chunk id |
| +8 | name used to match a pool reservation |
| +0xC | count |
| +0x10 | u16 list of chunk ids allowed to stay |
| +0x14 | pool, or 0 to pick one |
| +0x18 | u8 wait |
| +0x19 | u8 no-history |
| +0x1A | u8 special |
| +0x1B | u8 force-geom-without-peg |

1. Unless `0xE87B4F` is set: if req+0x1A is set → none.
2. `id = world_chunk_find_id(ECX = req+0)`. If id < 0x866 and the world chunk table `0xF7AAE8[id]` is non-zero (already loaded) → none.
3. **History.** Unless req+0x19 is set:
   - If the id is not in the history (`chunk_request_history_find(DX = id)`), add it (`chunk_request_history_add(id)`).
   - Otherwise, if entry+0xC == frame-1, set it to the current frame `0x252A3C0`.
   - Otherwise remove the entry and add it again.
4. If geom slot 0 is not idle and its chunk id (+0x52E) is not in req's list → `streamman_unload(0)`.
5. If req+0x1A is clear and the current slot is geom (type 0) with +0x542 set → `abort_current(0)`.
6. Continue only if geom slot 0 is NULL, or in state 0, 3 or 4. Otherwise → none.
7. If the current slot is type 4 → `abort_current(1)`.
8. If req+0x1A is set and any peg slot is busy → none.
9. Look through the reservations `0xF6FBD8[]` (count `0xF6FBC4`) from the newest down:
   - one in state 3 → none;
   - stop at the first with flag +0x1C bit 0.
10. Find the reservation whose name (+0x14) matches req+8 by stricmp, skipping flagged ones (bit 0). Call its index r.
11. If none matched, or req+0x18 / req+0x1A is set, go to step 13.
12. If a reservation matched, by its state (+0x20):
    - 7 or 8 → set state 9, +0x24 = -1, then `streamman_unload_chunk_pegs_extras(its chunk id)`;
    - 9 → continue to step 13;
    - anything else → none.
13. Need a free peg slot and a free extras slot, else → none.
14. **Pool:** req+0x14, else (no reservation) `chunk_pool_pick(*0x27718D0)` or, for req+0x1A, `0x27732AC` if it has room; else the reservation's pool (+0x10).
15. **Names.** Each is `path_change_extension` of the request name into a local:
    - chunk_pc → `"chunk_pc"`;
    - g_chunk_pc → `"g_chunk_pc"`;
    - pvs → `".pvs_pc"`;
    - extras → `".cts"`;
    - peg → `".peg_pc"`.
16. **Sub-requests** (only if there is a pool):
    1. Unless req+0x1A: `extras = request_chunk_extras(ECX = pool; out, id, cts name, reservation chunk id or 0xFFFF)`.
    2. `peg = request_chunk_peg(out, {peg name, id, pool, 0, wait})`.
    3. `geom = request_chunk_geom(EAX = {chunk_pc, g_chunk_pc, pvs, id, pool, wait, req+0x19, req+0x1A, r, no-peg})`, where no-peg = (peg is none) or req+0x1B.
17. If geom is none: unload peg and extras if they were made, then return none.
18. If wait (req+0x18): `streamman_wait(geom)` and clear the geom slot +4 / +8 / +0xC.
    - If that returned false → `*out = {0, 0xFFFF}` (lo 0, hi 0xFFFF).
    - If it returned true:
      - wait for and clear the peg slot;
      - wait for and clear the extras slot, but only when `0x2527D14` is 0 or `*0x2527D10 + 0xAFF` is set;
      - then `*out = geom`.
19. Without wait, `*out = geom`.

### Unload helpers

| Function | Behaviour |
|---|---|
| `streamman_unload_chunk_pegs_extras` 0xA80A40 | cdecl (u16 id): `streamman_unload` on busy type-1 slots with +0x52E == id and busy type-4 slots with +0x772 == id |
| `streamman_any_chunk_busy` 0xA80B40 | any slot of types 0, 1, 4 in state 1 or 2 |
| `streamman_cancel_chunk_loads` 0xA80BC0 | every busy chunk slot: type 0 → `streamman_unload`; types 1 and 4 → `0x4A9DA0(world chunk entry, -1)` for the chunk id stored in the info (peg +0x52E at `0x2FA45C6` + i*0x544, extras +0x772 at `0x2FA5D1A` + i*0x77C) |
| `streamman_unload_flagged_geom` 0xA80CC0 | AL = flag → `0xE87B4F`; `streamman_unload` of every geom slot with info +0x542 set |
| `streamman_release_chunk_extras` 0xA84410 | see the mesh cache section |

## Completion and step callbacks

All callbacks are cdecl (info*), except progress, which is (info*, node*). Each returns a bool in AL.

| Type | Begin | Issue | Progress | After | Retry | Completion (+0x1C) |
|---|---|---|---|---|---|---|
| 0 geom, step 0 | 0xA84E60 | 0xA84F80 (`ret`) | - | 0xA84F90 | - | 0xA84FF0 |
| 0 geom, step 1 | 0xA84F00 | - | - | 0xA84FC0 | - | |
| 1 peg, step 0 | 0xA85450 | 0xA85660 | - | 0xA856B0 | 0xA856E0 | 0xA857A0 |
| 1 peg, step 1 | 0xA85510 | - | 0xA85600 | - | 0xA856E0 | |
| 2 vehicle | 0xA87B10 | - | - | 0xA87BB0 | - | 0xA87BF0 (filter 0xA87AF0) |
| 3 character | 0xA7F840 | - | - | 0xA7F9C0 | - | 0xA7FA40 |
| 4 extras, steps 0/1/2 | 0xA81160 / 0xA80FA0 / 0xA814D0 | - | - | 0xA81200 / 0xA81040 / 0xA81530 | 0xA81610 (false) | 0xA81620 |
| 6 animation | - | - | - | - | - | 0xA87C50 (false) |

The geom and peg begin callbacks allocate from the chunk pool:
- free = `pool->vtable[+0xC]()` minus the step alignment; if > 0, `dest = pool->vtable[+0x1C](free, align)` → step+0x218, info+8 = dest, info+0xC = free; false if it cannot allocate.
- The after or issue callback gives the unused tail back: `vtable[+0x38](allocated - size)` for geom, `+0x34` with pool+0x48 kept for peg.

**chunk_geom_on_loaded (0xA84FF0):**
1. Compute the chunk:
   - if step0 has a file: `0x4AF280(step0 file, step0 dest, step0 size, +0x540, +0x542, +0x541)`, which is `chunk_load_from_memory`; store the result at +0x530;
   - otherwise use 0.
2. If the chunk is 0: clear the world chunk table entry for `world_chunk_find_id`, call `streamman_unload_chunk_pegs_extras(+0x52E)` and return false.
3. If no-peg (+0x543) is set:
   - `0x4A8BC0(chunk)`;
   - if chunk+0x180 is set, append it to `0xF70BA8[*0xF71B58++]`;
   - `0x4C6C30`, then `0x4AB660`.
4. Otherwise:
   - if wait (+0x541) is set: `0x4AB660`;
   - if not: chunk+0x20 = 4 and then 0. These are two stores to the same field; the result is 0.
5. If chunk+0x1C bit 0 or wait is set: chunk+0x18 = 3, `0x4AA840`, `0x562100`.
6. Return true.

**chunk_peg_on_loaded (0xA857A0):**
1. `e = 0xC086E0(step1 file)`. If e is 0 → false.
2. `0xC08EB0(step1 size)` and e[0] = 0.
3. If step0 has a file and a size:
   - if +0x538 is set, give back the unused GPU tail and set +0x538 = 0;
   - for the world chunk entry (+0x52E): `0x4AA8D0`;
   - if +0x540 is clear and `0xF6FBC0` is set: entry state = 6, +0x24 = -1;
   - otherwise `0x4AB660`.
4. Return true.

**character_on_loaded (0xA7FA40):**
1. `0x6E8A50(EAX = &loader)` finds a free character loader (0x28C-byte records). If there is none → false.
2. Otherwise it builds a descriptor and returns `loader->vtable[+8](&descriptor)`. The descriptor is { object (+0x18), step0 file, step0 dest, step1 dest, step4 file, step4 dest, step4 size, step5 dest, step5 size, step2 file, step2 dest, step3 dest, step6 file, step6 dest, step7 file, step7 dest, step8 file, step8 dest }.

**vehicle_on_loaded (0xA87BF0):** builds 11 x {file, size (+0x210), dest (+0x218)} and returns `0xADB530(EAX = info+0x18; &array)`.

**chunk_extras_on_loaded (0xA81620):**
1. World chunk entry (+0x772): flags |= 0x80.
2. If its state is 7: `0x4AB660(EAX = 8)`.
3. `0x914460(entry chunk id)`, then return true.

## Resource slot scheduler, step by step

Slot states (+0x28):

| State | Meaning | List |
|---|---|---|
| 0 | loaded and idle | +0xC |
| 1 | queued for loading | +0x10 |
| 2 | loading | current, sched+0x14 |
| 3 | in use (bound to a request) | +0x18 |
| 4 | free | +0x1C |

Registry entry +0 bit 3 = **load failed**.

Correction to the scheduler table above: +0x40 holds the per-group **request** queues and +0x44 the per-group **slot** queues. The two compare functions were swapped in the first notes: requests use 0xA752A0 and slots use 0xA751E0.

The callbacks per kind (`0x22248F8 + kind*0x10`) are:
- **+4 available:** called on load success and when a slot goes idle.
- **+8 failed.**
- **+0xC unload.**
- **+0:** not called by this code.

**rsched_add_ref** (0xA757F0): EAX = sched, stack (id, level, group, load_now).
1. Find the request with this id on the active list (+0x38).
   - **Found:** remember its best level (the highest non-zero of +0x2C..+0x44) and best group (the highest index with a non-zero count). Increment level and group, compute both again.
     - If the best level changed, set dirty.
     - If the best group changed, set dirty, re-queue the request with a new +0x10 and update the bound slot's group (+0x24) / requeue it unless it is current.
     - If the request has no slot: `rsched_bind_loaded_slot`.
   - **Not found:** pop the free list (+0x3C); if it is empty, return.
     1. Clear the request, set the id, flag bit 1 = load_now, increment level and group.
     2. Append it to the active list and push it on the request queue.
     3. Call `rsched_bind_loaded_slot` and set dirty.

**rsched_release** (0xA75A60): EAX = sched, EDX = id, stack (level, group, all).
1. If +0xD0 bit 3 (updating) is set: append {id, level, all, group} to the deferred list (+0x48 count, 0x10-byte records at +0x4C) and return.
2. Find the request. If it is absent and streaming is on, `0x347B088` = registry name (debug) and return.
3. Take its best group. Then either:
   - `all`: clear the level counts and the group counts;
   - otherwise: decrement level[level] and group[group].
4. If any level is non-zero: `rsched_request_requeue(old best group, new best group)`, then `rsched_bind_loaded_slot` if it has no slot.
5. If every level is zero:
   1. Unbind from its slot.
   2. If the request has an object (+0xC): `rsched_slot_make_idle`.
   3. Unlink from the active list, remove it from its queue, clear it (`0xA75030`) and append it to the free list.
   4. Set dirty.

**rsched_update** (0xA75730): EAX = sched. Runs only if +0xD0 bit 0 is clear and bit 1 is set.
1. Clear bit 1 and set bit 3.
2. `rsched_bind_loaded_slot` for every active request with no slot.
3. `rsched_assign_group_quotas`.
4. `rsched_update_group` for each group.
5. Clear request flag bit 0 on all of them, then clear bit 3.
6. Replay the deferred releases and zero the count.

**rsched_assign_group_quotas** (0xA76220): for each group g:
1. Pop every slot from the group's slot queue (sched+0x44 + g*0x18) into a temp list with +0x20 = -1. Count them, counting the current slot if its group is g.
2. If the count exceeds the quota (+0xCC[g]), move the excess (taken from the front) to a spare list.
3. Push the rest back.

Then for each group short of its quota, move spare slots into it: set the request's bit 0 and unbind it, push the slot on the group's queue, set slot +0x24 = g.

**rsched_update_group** (0xA764D0): EAX = group, stack (sched).
1. Pop every slot from the group's slot queue (+0x44 + g*0x18) into list S (slot +0x20 = -1, temp links +0x34/+0x38).
2. If S is not empty, pop up to |S| requests from the request queue (+0x40 + g*0x18) into list R (request +0x10 = -1, temp links +0x24/+0x28).
3. **Pass 1.** Re-push any request whose registry id has the failed bit.
4. **Pass 2.** For each request, find a slot in S with the same id. If found:
   - take both out of their lists;
   - if the request's best level > 4 and the slot has an object: `rsched_slot_make_idle`;
   - re-push both.
   - Also re-push the request if it is the current job's slot id.
5. **Pass 3.** Re-push any request whose id is already in a slot whose group (`0xA750A0`) differs.
6. **Pass 4.** Pair the remaining requests (best first) with the remaining slots (worst first):
   - if the request's best level > 4, or the slot is not loaded (+0x10 == 0):
     1. `rsched_slot_evict`, then put the slot on the queued list (+0x10) with state 1.
     2. Set slot fields: id = request id, +0x14 = 0, +8 / +0xC / +0x10 = 0; bind both.
     3. If request bit 1 (load_now) is set: job = `rsched_job_add(id, A + i*strideA, strideA, B + i*strideB, strideB, best level)`, where i = slot index. Otherwise job = -1.
     4. If the request has an object, `rsched_slot_make_idle`.
   - else, if the request has no object and the slot has none: `rsched_slot_use`.
7. Push back everything left in R and S.

**Jobs:**
- `rsched_job_add` (0xA77DD0): cdecl (id, dest, size, dest2, size2, level) → queue index, or -1 when 50 (0x32) jobs are queued. It takes a job from the free list `0x2529E54`, appends it to `0x2529E50`, count++, and pushes it on the queue `0x2F9B2D0`.
- `rsched_job_start_next` (0xA77B90): if the queue is not empty:
  1. Pop the best job and unlink it.
  2. Make it current (`0x2529E58`).
  3. `rsched_job_begin_slot(EAX = id, ECX = scheduler of kind)`.
  4. By kind:
     - 0, 2, 3 → `rsched_job_request_character(out, id, dest, dest2, size, size2)`;
     - 1, 5 → `rsched_job_request_vehicle(out, dest, dest2, size, size2)` with EAX = id;
     - 4 → `streamman_request_animation(out, name, dest2, registry +8)` with EDX = size. The name is "Empty" for id -1.
  5. Store the handle at `0xEB80F8`.
- `rsched_job_pump` (0xA77F10): if a handle is set (not `{0xFFFF hi, 0 lo}`):
  1. If `streamman_is_idle(handle)` is false, return.
  2. If the streamer slot state is 4: `rsched_job_finish_loaded(id, slot+8, 1)` with EDX = slot+0xC.
     Otherwise: `rsched_job_finish_failed(id, 1)`.
  3. Clear the streamer slot (+4, +8, +0xC).
  4. Reset the job (id -1), append it to the free list, count--, current = 0, handle = 0xFFFF0000.
  5. Call `rsched_job_start_next`.

  Because `animation_on_loaded` returns false, kind 4 (animations) always takes the failed path.
- `rsched_job_cancel` (0xA77E90): removes a queued job from the queue and the active list and frees it.

**Every frame**, `rsched_update_all` (0xA79080):
1. `rsched_update` on the six schedulers.
2. `rsched_job_pump`.
3. `0xA78AC0` when streaming is on, `0x2528C14` is set, (`0x2528B56` == 0 or `0x2528B2B` != 0), `*0x2528C14 + 0x3F8` is set and `0x2528BB4` < 1.

### poll_chunk_extras and process_sequential in Rust

`streamman_poll_chunk_extras` (`0xA7E900`) and `streamman_process_sequential` (`0xA7D0C0`) are in
Rust too, both detoured so that the original run logs the same events. With these, everything
the pump does per resource runs in Rust; the io scheduler and the request functions are still
the original.

- Teleport test, poll: 108-111 polls and 17 extras completions per run. The first of two Rust
  runs had 2 chunk-extras starts with different mesh-part/anim counts. Two original runs and the
  second Rust run all matched exactly (659 distinct non-ambient events). The difference came from
  which mesh records and anims were already resident, and was not reproduced.
- process_sequential is not exercised by any test so far (no type-6 slot is processed), so it
  is checked against the disassembly only. Details the port keeps: an already past-the-end step
  is still issued; `file_get_size` in state 2 refreshes step +0x210, which the cursor advance
  then uses; "done" is re-polled before the retry callback; and multi-part steps advance only
  once +0x20C >= +0x208.
- Calls that need EDX as an input (or return EDX:EAX) go through `game_call_edx`. `async_io_free`
  takes four arguments, so it has its own helper (`io_free`).


## Chunk-extras mesh part state machine (0xA81160 / 0xA81200)

**These callbacks never run in the PC build.** `_impl` stores them in the type-4 steps, but only the step-callback paths call step callbacks:
- `streamman_start_load` / `check_batch`, reached through `process_generic` for types 0-3, 5, 7-9;
- `process_sequential`, type 6 only.

Type 4 goes to `start_chunk_extras` / `poll_chunk_extras`. Those read the step data (+0x208 part count, the part list at +8, the allocator at +0x220) but never read +0x22C..+0x240.

There are no other references to 0xA81160, 0xA81200, 0xA80FA0, 0xA81040, 0xA814D0, 0xA81530 or 0xA81610 besides the pushes in `_impl` (0xA83039..0xA8315C). They look like the original sequential design, which `start_chunk_extras` replaced with batched part reads. They are documented here so a port can leave them out knowingly.

Step 0 is a multi-part step (+4 = 1). Its state:

| Off | Field |
|---|---|
| +0x208 | object count |
| +0x20C | current object index |
| +8.. | the object names |
| +0x228 | part state of the current object: 0 smesh, 1 g_smesh, 2 peg, 3 g_peg, 4 done |
| +0x224 | bytes allocated for the part |
| +0x220 | the record's mesh object (record+0x10C); its +0x54 is the GPU allocator |
| +0 (file) | the record's name field for the part |

**Begin, 0xA81160** (cdecl info*):
1. step0+0x224 = 0.
2. Pick the allocator by part state:
   - 0 or 2: `alloc = step0+0x220`;
   - 1 or 3: `alloc = step0+0x220 -> +0x54`;
   - anything else: return false.
3. `free = alloc->vtable[+0xC]() - align` (align = u16 at step0+0x214). If free <= 0, return **true** with nothing allocated.
4. `dest = alloc->vtable[+0x1C](free, align)` → step0+0x218. If dest is 0, return true.
5. Otherwise `memset(dest, 0, free)`, step0+0x224 = free, info+8 = dest, info+0xC = free, return true.

**After, 0xA81200** (cdecl info*):
1. `rec = smesh_cache_find(EDX = object_template_find(ECX = names[+0x20C]))`. If rec+0x122 (loaded) is set, go to step 4.
2. Let `size` = step0+0x210 (bytes read) and `slack` = +0x224 - size.
   - If size == 0: part state = 4.
   - Otherwise, by part state:

   | State | Action | Next state |
   |---|---|---|
   | 2, peg | rec+0x12C = dest, rec+0x130 = size. If rec+0xCB (g_peg name) is empty: `0xC08B80(ESI = rec+0x12C, ECX = rec+0x8A; size, 0)`. Then shrink the CPU allocator: `rec+0x10C` vtable +0x34(alloc+0x44 - slack), keeping alloc+0x48 | 4 if g_peg is empty, else 3 |
   | 3, g_peg | `0xC08B80(ESI = rec+0x12C, ECX = rec+0x8A; rec+0x130, dest)`; `smesh_table_find(path_change_extension(step0 file, ".smesh_pc"))` (result unused); GPU allocator vtable +0x38(slack) | 4 |
   | 0, smesh | rec+0x124 = dest. If rec+0x49 (g_smesh name) is empty: `smesh_table_add_streamed(ECX = names[+0x20C], EDX = step0 file; dest, 0)`, then `0xA84BE0(ESI = rec+0x10C; slack)`, the CPU shrink | 2 if g_smesh is empty, else 1 (with the same CPU shrink) |
   | 1, g_smesh | `smesh_table_add_streamed(ECX = name, EDX = file; rec+0x124, dest)`; GPU allocator vtable +0x38(slack) | 2 |

3. Point the step at the next part and return true:
   - state 1 → step0 file = rec+0x49;
   - state 3 → rec+0xCB;
   - state 2 → rec+0x8A;
   - state 4 → fall through to step 4.
4. **Object done:** rec+0x122 = 1, +0x20C += 1, part state = 0.
   - If +0x20C == +0x208: step0 file = 0, return true.
   - Otherwise `rec = smesh_cache_find(object_template_find(next name))`, file = rec+8 (its .smesh_pc name), +0x220 = rec+0x10C, return true.

Unlike in `poll_chunk_extras`, the g_peg call here passes the data and size swapped between the two peg states. That only matters if the code is ever revived.

`0xA84BE0` takes ESI = allocator, stack (n), and is `ret 4`. It calls `vtable[+0x34](alloc+0x44 - n)` and restores alloc+0x48: a shrink to the used size.

## What requests a type-6 (animation) load

- The only caller of `streamman_request_animation` (0xA88240) is `rsched_job_start_next` (0xA77CCF), for scheduler kind 4 (ids with high byte 4). Nothing else references the type-6 slot pointer `0x2225B54`.
- So `process_sequential` runs only when something calls `rsched_add_ref` on scheduler 4 (`0x2F9B2E8 + 4*0xD4`) with load_now set, and `rsched_update` then queues a job.
- The `rsched_add_ref` call sites (`0x57CF34` vehicle_kill_objective, `0x671BE1` escort, `0x69CB24`/`0x69CDCC`, `0x6D01C5`/`0x6D0695`, `0x888E16`, `0x8A203D`/`0x8A2110`, `0x8AE55F`/`0x8AE588`, `0xA75D33`, `0xA791F4`, `0xA79E33`, `0xA7A03D`) all take the scheduler from the id's high byte at run time, so which of them pass kind-4 ids depends on data.
- No code builds a kind-4 id from a constant. The registry id builder (`0xA77540`, kind << 24 | index) is only reached from `0xA798C0`, which nothing calls or references.

So the kind-4 ids must come from data tables. A runtime log is the practical way to find a trigger: log `rsched_add_ref` when EAX = 0x2F9B638, or calls to 0xA88240, while playing missions and activities. Cutscene and activity scripts are the likely users.

### Request side in Rust (part 1)

In Rust behind `PUMP_RUST`, with the same request event in both modes:
- `step_init` (`0xA7F770`) and `file_exists` (Rust-internal; the original is not detoured).
- The unload helpers `0xA80A40`, `0xA80BC0` and `0xA80CC0`. `any_chunk_busy` (`0xA80B40`) is a pure
  query and is shadow-checked.
- The typed requests: animation `0xA88240`, character `0xA7FC30`, vehicle `0xA87F80`, chunk geom
  `0xA85140` and chunk peg `0xA85890`. Register arguments come in through a naked entry thunk
  that passes EAX/ECX/EDX and a pointer to the stack arguments.

Each request logs its handle, slot state and, for every step, the file name, alignment, the
+0x21C flags and which callbacks are set. Teleport test, original vs Rust:
- All 646 geom and 23 peg requests are identical.
- Character/vehicle requests are ambient (42 appeared identically in both runs, the rest
  differ by spawn).
- No animation request happened in either run.
- 0 mismatches.

Still original: `request_chunk` (`0xA80360`), `request_chunk_extras` (`0xA84510` / `_impl`
`0xA82EA0`), `step_init_multipart` and all the completion and step callbacks.

**Resolved: not a port bug.** In 2 of 3 Rust runs, two chunk-extras starts differed from the
earlier original runs: chunk `0x103` got 0 mesh parts with 227,720 bytes of anims (instead of 8
parts and 189,368 bytes), and chunk `0x16F` got 16 parts (instead of 8). A run of the original
with the start order logged (`sr2-streamman-seq.log`, one line per chunk-extras start, written
next to the event log) shows exactly that pattern too. So the original flips between the two
outcomes from run to run.

The cause is ordering. The request queue compare (`0xA7CF30`) orders by slot type and pop_best
keeps the lower slot address, so extras requests start in slot order (4:0 before 4:1), not
request order. Which slot a request gets depends on which one was free at request time.
Neighbouring chunks share objects, so whichever starts first loads the shared mesh parts. The
sequence log is kept for checking orderings in future comparisons.


## Request order: the queue compare (0xA7CF30)

The request queue `0x2F9B88C` is set up by `streamman_init` (0xA7D400) with 10 nodes and the compare `streamman_request_compare` (0xA7CF30) at +0x14. The compare gets two slot pointers and compares their **type** (slot +0):
- best type < candidate type → 0 (keep best);
- equal → 1;
- best type > candidate type → 2 (take candidate).

With the `pop_best` rule above (a 1 keeps the current best only when best < candidate as values, i.e. by slot address):
1. Lower types always go first: chunk geom 0, peg 1, …, extras 4.
2. Within one type, the slot with the **lowest address**, i.e. the lowest slot index, goes first.

Request time plays no part. Two chunk-extras requests start in the order of their slot indices (4:0 before 4:1), whichever was requested first.

So if a request lands on a different slot, two neighbouring chunks' extras swap their start order. Which slot a request lands on is the first type-4 slot in state 0, 3 or 4 at the time of `_impl`, so it depends on when the previous extras slots went back to idle. This is the effect seen in the teleport test, where an extras start landed on slot 4:1 instead of 4:0.

### Request side in Rust (part 2): request_chunk and the extras request

`streamman_request_chunk` (`0xA80360`), `request_chunk_extras` (`0xA84510`, ECX = pool, through the
register entry thunk), `_impl` (`0xA82EA0`) and `step_init_multipart` (`0xA80EE0`, Rust-internal)
are in Rust now. That covers the whole request side except the step and completion callbacks.

Teleport test, original vs Rust: identical events for all 655 distinct chunk requests (mission
interiors included) and 20 extras requests (2,004 distinct non-ambient events each). The one
differing event was an extras request without meshes. Its step 0 keeps whatever the slot's
previous request left in alignment and callbacks, because only the file and +0x208 are cleared.
Which slot it got is timing, as above. The step has no file, so it is never read.

Corrections to "Request side, step by step" (checked against the disassembly):
- request_chunk, matched reservation: state 7/8 → mark it 9, unload its pegs/extras, return
  none. State 9 → none. Any other state → continue. (The notes had 9 and "other" swapped.)
- The geom is unloaded only when the keep list (req+0xC count) is non-empty and does not
  contain its id; an empty list unloads nothing.
- The peg name is built from req+4 when that is set, else req+0.
- A sub-request counts as none only for exactly 0xFFFF0000. A failed geom wait writes a zero
  low word, then 0xFFFF.
- A special request (req+0x1A) without a pool uses `0x27732AC` only if `vf3() >= *0x27732E8` and
  `vf4() == 0`.
- request_chunk_geom: steps are at info+0x14 (pointer). Without a g_chunk_pc, step 1's size and
  dest are cleared, not its file.
- request_vehicle: the register inputs are EDX = size A and ECX = dest B; EAX is read from the
  stack.

