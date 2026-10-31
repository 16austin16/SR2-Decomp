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
| 0x01 | `0x4ADCD0` | 0x94-byte named entries (rec+0x104/+0x108) with per-entry scale floats and u16 lists |
| 0x02 | `0x4A35E0` | 0x28-byte entries (rec+0x134/+0x138), then `0x7F63B0` |
| 0x03 | inline | **player starts** (`PlayerStart01`, …): count/pointer at rec+0x10C/+0x110 |
| 0x04 | `0x4ADF20` render_items | 0xC8-byte render items (rec+0x5C/+0x60) and a node tree (rec+0x64); `game_render` collects the visible ones through the tree (`0x5239A0`); dropped for flag-bit-0 chunks |
| 0x05 | `0x4ADF70` collision_models | Collision models via `0x7051C0` (indices rec+0x1F0/+0x1F4); remaps object collision indices (+0x3C/+0x3E). The only section with a string ("city_objects_load: Failed to load the collision models…") |
| 0x06 | `0x4AE0C0` placed_spawns | 0x34-byte placements (rec+0x19C/+0x1A0); type-name hash → table `0x263AF98`; created by `0x4C61C0` at position/orientation, not within 15 m of the player |
| 0x07, 0x08 | `0x4AE240`, `0x4AE280` collision_mopp | **Havok MOPP collision** (`BBPF`/`MOPP` in the data): two count/array pairs each (rec+0x68..+0x74, +0x78..+0x84); cleared when chunk flags & 0x13 |
| 0x09 | inline | count/pointer at rec+0x88/+0x8C |
| 0x0A | `0xA8AC00` | 0x3C- and 0x28-byte entries (rec+0x13C..+0x148), then `0xA8D970` with the traffic globals `0x2225B64`. Probably the road/lane network (unverified) |
| 0x0B | inline | count/pointer at rec+0x14C/+0x150 |
| 0x0C | `0x4AE320` | Node tree at rec+0x154, fixed up recursively by `0xBF6A40` |
| 0x0D | `0x4AE380` collision_shapes | Havok collision (`MOPP` in the data): 0x28-byte entries with vec3 lists (rec+0x158/+0x15C), registered with `0x6F9590` → rec+0x160 |
| 0x0E | `0x4AE460` water_volume_geometry | **Water volumes**: N meshes (rec+0x164/+0x168), 16-byte vertices, u16×3 triangles |
| 0x0F | `0x4AE510` render_materials | Render materials (texture references such as `missing-alpha.tga`, `flat-normalmap.tgn`; avg 292 KB): rec+0x174/+0x178; advances a gpu-side offset |
| 0x10 | inline | count/pointer at rec+0x180/+0x184 |
| 0x11 | `0x4AE590` | 0x54-byte entries (rec+0x188/+0x18C), tail at rec+0x190 |
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
| 0x1E | `0x4AEB10` | 0x10-byte entries with 0x14-byte sub-arrays (rec+0x204/+0x208) |
| 0x1F | `0x4AEB80` | count/array rec+0x20C/+0x210 |
| 0x20 | `0x4AEBB0` | 12-byte {object, hash → `0x21F2AA0` entry, hash → `0x27923E0` index} (rec+0x214/+0x218) |
| 0x21 | `0x4AECB0` | 0x38-byte entries with arrays and a mesh (rec+0x21C/+0x220) |
| 0x22 | `0x4AEDB0` | rec+0x224 |
| 0x23 | `0x4AEF00` | per-object 8-byte array (rec+0x54/+0x58) |
| 0x24 | `0x4AEF60` | per-object array (rec+0x230/+0x234) |
| 0x25 | `0x4AEFA0` object_anims | Per-object frame animations (rec+0x238/+0x23C): name, object (flagged +0x34 \|= 0x40000), frame table into rec+0xA0; `0x91C6C0` plays them (flags 1 playing, 2 ping-pong, 4 loop, 8 reversing) |
| 0x26 | `0x4AEDD0` speedtree_instances | Trees (rec+0x228/+0x22C): hash → SpeedTree table `0x26B5DE8` (count `0x2527288`), the table the SpeedTree loader `0xA8DAFB` fills |

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

