# 54: Spawning and population (names round 54)

This round maps the spawn and population code around `world_spawn_do_frame` (0x93D150): traffic, ambient pedestrians, district spawn info, spawn groups (cell / skirmish / notoriety / special vehicle), the resource stream budget, parked vehicles, spawn records and the El train. It adds **93 new names** in `docs/candidates_spawning.csv` (35 confirmed, 58 inferred; none are in `functions.csv`; notes end "(54)") and **80 renames of `vfN` placeholders** in `docs/renames_spawning.csv` (30 confirmed, 50 inferred), 173 functions in all. It also gives ten large port maps and fifteen small port recipes.

## Method and caveats

- **Function starts.** Every candidate address was checked against a linear sweep of `.text` (start after `int3` padding or a `call rel32` / `jmp rel32` target) and against `functions.csv` and the other `candidates_*.csv` files (no address or name is reused). `tools/xrefs.py` reports byte matches inside instructions, so the caller counts in the notes come from a decoded sweep of `call E8` and `jmp E9` only.
- **True sizes.** A body is walked with a recursive descent (conditional and unconditional jumps, jump tables decoded from their bytes, tail jumps to other function starts stopped). The old rule 'size = up to the first `int3`' is wrong for many of these functions because a `0xCC` byte can occur inside an immediate: for example `ambient_peds_despawn_check` is **0x8F8** bytes, not 541, and `distant_peds_spawn_update` is **0xBD9**, not 128.
- **Register conventions** in the notes are a linear-prefix scan (registers read before written along the first fall-through path, as `tools/regin.py`), so they can miss a register that is read only after the first branch. Stack argument counts come from the `ret N` instructions of the walked body.
- **Placeholders.** `functions.csv` carries 104 `vfN` entries for the spawn group classes (and the activity classes that share their base). 80 of them belong to the spawn group classes and are renamed here; the `vandalism_info`, `vandalism_level_instance`, `escort_info`, `escort_instance` and `escort_level_instance` slots belong to the activities and are left alone.
- **Port maps are machine generated** by a block-level pass over the disassembly (see the key below). Section purposes are inferred from callee names; the calls, addresses, tables and compare polarities are exact. Always re-check a block against `python tools/x86dis.py` before porting it.
  - `X87 STORE f32 [m] <- expr` is an `fstp/fst dword`: the value is rounded to f32 there. `f64(...)` operands are double constants printed with their exact bits; `f32(...)` likewise. `ret(callee)` marks a value the x87 stack received from a call; `ST(n)` is an untracked register slot.
  - `X87 COMPARE a vs b` followed by `if (x87 compare ...: ST0 outcome in {...})` lists *every* outcome (gt = ST0 > operand, lt, eq, un = unordered) for which the jump is taken. The polarity was derived from the flag bits: `test ah, mask; jne/je` use the mask directly, `jp/jnp` use the parity of `ah & mask` (PF is set when an even number of bits is set), `sahf` forms use C0 -> CF, C2 -> PF, C3 -> ZF.
  - Integer `a < (signed) b` / `(unsigned)` marks the compare type; `x == 0` after `test r, r` is exact.
  - `regs[...]` after a CALL shows the last instruction that wrote each register the callee reads; `callee-reads` is the linear-prefix scan, `callee-ret-imm` the callee's `ret N` (N > 0 means stdcall, the callee pops).

## Surprises

1. **Placeholders were the biggest pool of unnamed code.** 80 of the 104 `vfN` entries are in the spawn group classes. Their slot meanings are now tied down by the manager `spawn_groups_update` (0x957250): slot 5 returns the vehicle info index, slot 8 / 9 / 10 / 11 are reset / update_streaming / update / is_active, slots 12, 13, 14 and 15 are the four candidate filters (see the slot table below).
2. **`spawn_stream_budget_compute` (0x946AC0) is not a clear.** It was read as a 4-word reset from the first instructions, but it zeroes nine words 0xE887BC..0xE887DC, applies per-mode values (mode 0x279AB14), calls 0x946880 and clamps against two tables. `spawn_stream_budget_publish` (0x9473B0) copies those nine words plus four more to their consumers with per-block dirty bytes.
3. **`traffic_vehicle_spawn_try` (0x939F30, already named) reads ESI at entry** (linear scan); the documented signature `cdecl (&spawn, bool)` is incomplete.
4. **0x93D4D0 and 0x93C660 test different gate arrays** (`0x215DA0C` and `0x215DA1C`): the six-byte arrays at stride 8 from 0x215DA04 are separate gates (distant peds 0x215DA04, 0x215DA0C, parked 0x215DA14, 0x215DA1C, humans 0x215DA24).
5. **0x951B40 is a separate function**, not the tail of `spawn_records_reset` (0x951A90): it follows a `ret` with no padding and has its own call site (0x5200AC).
6. **Notoriety spawn groups use the notoriety level as a table index**, not a threshold: slot 1 (scale factor) is 1.2 / 1.4 / 1.6 / 1.8 for levels 2..5 and slot 2 (offset value) is 5 / 10 / 15 / 20, selected through 4-entry jump tables at 0x923D3C / 0x923D8C.

## Structures

### Traffic records

- **Active ring** `0x215DA30` (head), **free ring** `0x215D7AC` (head), **active count** `0x215D7A8`. Records are circular doubly linked lists: `[rec+0]` next, `[rec+4]` prev, `[rec+8]` vehicle handle, `[rec+0x10]` flags (bit 0 set by `traffic_record_activate` from ECX bit 0, bit 1 unused, bits 2+ preserved), `[rec+0x2A]` u16 index into the 12-byte table at `0x223E410`.
- A single-element ring has `next == prev == self`; walking uses the idiom `sub/neg/sbb/and` to turn `next == head` into 0 (`ecx = next & (next != head ? -1 : 0)`).
- Handles resolve through the object table `0x2149C64` (16-byte slots, index `handle & 0xFFF`, `[obj+0x44]` must equal the handle, `[obj+0x4C]` is the type; 5 = vehicle, 1 = human).

### Entity budget (`0x2BA4BC8`)

Two rows (kind 0 and 1) of 0x84 bytes. Counters at `+0x10 .. +0x58` (8-byte stride) are summed by `entity_budget_free_slots`; `+0x5C + 4*subtype` are 'last service' timestamps; the per-subtype limit table is at `0xE889F4` (28-byte rows per kind). `entity_budget_can_add(kind, count, flag)` is called from the human and vehicle creators.

### Spawn groups

17 group objects at `0x215E578 .. 0x215E5B8`; `0x215E5BC` is the *current* group chosen by `spawn_groups_update`. Common members: `+0x2C` current vehicle info pointer (index = `(ptr - 0x2FAD1F8) / 0x7D0`), `+0x30 + 4i` member objects, `+0x5C + 0x30 i` eight member entries (in-use byte at +0, object id at +4 of the entry's +0x60), `+0x1DC / +0x1E0` counters, notoriety groups add `+0x1E4` faction, `+0x1F8` type id, `+0x200 / +0x204` min / max level, `+0x224 + 4i` nine slot nodes.

Vtable slot meanings (slots 7..11 were already named in `vtable_slots.csv`):

| Slot | Offset | Name | Meaning | How it is known |
|---|---|---|---|---|
| 0 | +0x0 | `find_member_by_object` | stdcall (obj) ret 4: member entry for the object or 0 | identical 8-entry scan in the special, notoriety and cell groups |
| 1 | +0x4 | `get_scale_factor` | f32 in ST0; base returns 1.0, notoriety 1.2..1.8 by level | fld1 stub and the level jump table |
| 2 | +0x8 | `get_offset_value` | f32 in ST0; base returns 0.0, notoriety 5..20 by level | fldz stub and the level jump table |
| 3 | +0xC | `set_params_ignored` | stdcall ret 8, no-op in the base | stub |
| 4 | +0x10 | `get_override_value` | base 0; cell returns [this+0x1F0] | bodies |
| 5 | +0x14 | `get_vehicle_info_index` | index into the 0x7D0-byte vehicle info records | called by spawn_groups_update (+0x14) |
| 6 | +0x18 | `get_member_slot` | stdcall (i) ret 4: i-th member object (notoriety: request slot resources) | bodies |
| 7 | +0x1C | `on_table_loaded` | named earlier | 37 |
| 8 | +0x20 | `reset` | named earlier | 37 |
| 9 | +0x24 | `update_streaming` | named earlier | 37 |
| 10 | +0x28 | `update` | named earlier | 37 |
| 11 | +0x2C | `is_active` | named earlier | 37 |
| 12 | +0x30 | `is_eligible_to_spawn` | bool filter 1 (called after is_active) | spawn_groups_update loop at 0x957364 |
| 13 | +0x34 | `is_resource_loaded` | bool filter 2 | spawn_groups_update loop at 0x957382; special body checks the resource table 0x2F9B2E8 |
| 14 | +0x38 | `get_free_member_count` | int filter 3; must be > 0 | loop at 0x957394; special body counts empty member entries |
| 15 | +0x3C | `needs_player_slot_valid` | bool; when true the host player [+0x14E4] must not be -1 | loop at 0x9573A3..0x9573B3 |
| 16 | +0x40 | `register_member_human` | stdcall (human) ret 4 | special body adds the streaming reference |
| 17 | +0x44 | `unregister_member_human` | stdcall (human) ret 4; base no-op | stubs and bodies |
| 18 | +0x48 | `get_streaming_priority` | int 1 or 2 (notoriety: 1 if byte +0x2AA) | stubs |
| 19 | +0x4C | `flag19` | bool (special true, skirmish false) | stubs; consumer not found |
| 20 | +0x50 | `flag20` | bool (true except vandalism news) | stubs; consumer not found |
| 21 | +0x54 | `get_type_id` | int member (skirmish +0x234, cell +0x238, notoriety +0x1F8) | bodies |
| 22 | +0x58 | `get_template_override` | base 0; cell resolves a template for type 0xB | bodies |
| 23 | +0x5C | `get_member_limit` | special 5, skirmish 4 | stubs |
| 24 | +0x60 | `restart_timers` | base no-op; vandalism sets a 30 s cooldown, police heli resets three timers | bodies |
| 25 | +0x64 | `flag25` | bool | stubs; consumer not found |
| 26 | +0x68 | `flag26` | bool | stubs; consumer not found |
| 27 | +0x6C | `pick_member_if_below` | stdcall (obj, a, b) ret 0xC | body at 0x922E10 |
| 28 | +0x70 | `flag28` | bool (gang: mission target faction) | bodies |
| 29 | +0x74 | `can_spawn_now` | bool; base false | police, gang and heli bodies |

### District spawn info and spawner nodes

`spawn_info_for_player_district` returns the `+0x74` field of the 0xA8-byte district record (table `[0x2526C94]`, index from `district_index_at_point`). A spawn info has a row count at `+4`, rows at `[+8]` (0x1C bytes, first dword = category pointer) and a default category at `+0x10`. A *spawner* (key at `+0`, count `+0xC`, up to 0x14 categories at `+0x10`, selected category `+8`) collects the categories whose key array (`[cat+0x20]`, count `[cat+0x1C]`) contains its key. Spawn nodes (0x1F8 bytes) live in the rings `0xEB7AC4` (pool `0xEB7AB8`) and `0xEB7B14` (pool `0xEB7B08`), link at `+0x1F8`; the spawner ring is `0x2BE0A0C` (link `+0x60`).

### Stream budget

Nine words `0xE887BC..0xE887DC` (plus four at `0xE887E0..EC`) are the per-type streaming budget computed by `spawn_stream_budget_compute` and handed to the resource scheduler through `spawn_stream_budget_publish`; `spawn_stream_budget_add` spreads an extra amount over the nine words.

## Names

### New names (docs/candidates_spawning.csv)

#### ai (80)

| Address | Name | Notes | Conf |
|---|---|---|---|
| 0x55BA40 | `vehicle_groups_update` | per-frame (caller 0x68CD0B in sim_frame): vehicle group bookkeeping that shares the counts at 0x1409AA8 with vehicle_groups_timed_spawn | I |
| 0x55BD10 | `vehicle_group_spawn_named` | builds the group's vehicle / persona names with wsprintf_1024 (format at 0x347AE54) and spawns one vehicle group member; called by vehicle_groups_timed_spawn | I |
| 0x55F670 | `notoriety_level_for_faction` | EAX = faction (1..6) -> EAX: notoriety level of that faction: jump table at 0x55F6D0 selects the record in the array at 0x279AA60 (stride 0x3C) and returns its first dword | C |
| 0x5600B0 | `notoriety_active_faction` | no args -> EAX: the faction currently used for the notoriety spawn groups (reads 0xE985D4; calls 0x55F7B0 to derive it); -1 when none | I |
| 0x9288E0 | `entity_budget_timers_service` | EDI = subtype index: for both kinds (row stride 0x84 from 0x2BA4BC8) re-arms the elapsed timer [row+idx*4+0x5C] (wrap-safe against 0xE843AC, period constant 0x6B49D200) and calls the per-subtype callback table at [0xE889F4 + idx*0x1C + 4 + kind*12] with (row+...,idx) | I |
| 0x9290F0 | `entity_budget_free_slots` | EAX = kind -> EAX: for kind 1 returns [row+8]; otherwise min(limit - used, 0x4B - used) where used = sum of the row's counters (+0x10..+0x58) and the limit is 0x4B, or the co-op scaled value from 0x93E8D0(f32 0xE5C6A0, 0x96) | I |
| 0x929540 | `entity_budget_can_add` | EAX = kind (0 or 1), stack (count, flag) cdecl -> AL: 1 when the current total for the kind plus count stays within the per-kind limit table at 0xE889F4 (28-byte rows) | I |
| 0x936080 | `spawn_bounds_visible_in_views` | bounds test: tests an AABB against the camera view block 0xEB7A50..0xEB7A7C with shape_bounds_overlap_test and render_views_test_bounds; AL = visible | I |
| 0x936240 | `spawn_bounds_visible_for_partner` | single-player returns at once; in co-op repeats spawn_bounds_visible_in_views for the partner | I |
| 0x936310 | `ambient_ped_try_place` | tries to place one ambient pedestrian at a candidate point: rejects visible points (0x936080 / 0x936240), checks the entity budget (0x929540) and builds the spawn through 0x935EE0; AL = placed | I |
| 0x9367B0 | `ambient_ped_try_random_offset` | draws three rng_float01 values (offset from a point, vec3_normalize_safe_len) and calls ambient_ped_try_place | I |
| 0x9368F0 | `ambient_ped_in_view_check` | view test used before despawning an ambient ped: object_visibility_cached(1) plus the 0x9DFBD0 / 0x98E880 range tests | I |
| 0x936A10 | `ambient_ped_out_of_view_check` | wraps ambient_ped_in_view_check with a vcall and the 0x215FFC0 camera position; AL | I |
| 0x9373C0 | `ambient_ped_pick_spawn_point` | picks a candidate spawn point for an ambient pedestrian (random direction around the camera, spline / navmesh checks through 0xA717F0 / 0xBE7740); the point the doc of ambient_ped_spawn calls 0x9373C0 | I |
| 0x9377F0 | `ambient_ped_spawn_search` | recursive search for a valid ambient spawn point: rng_float01 x2, vec3_normalize_safe, then 0x9373C0 / itself / 0x935E70 / coop_spawn_count_scale; AL | I |
| 0x937E60 | `ambient_ped_spawn_rate_limit` | checks the spawn rate timers 0xEB7A4C / 0x2BE0814..1C against game time 0xE843AC (via 0x937DD0) and the ped count 0x215D940; AL | I |
| 0x937FF0 | `ambient_peds_spawn_pass` | per-frame pass that tries to fill the ambient pedestrian budget: reads 0x2529E20, 0x2529585 and the ped count 0x215D940, calls 0x937E60, 0x935E70, 0x937F80 and ambient_ped_try_random_offset | I |
| 0x938C30 | `traffic_record_activate` | EAX = traffic record, ECX bit0 = flag: stores flags into [rec+0x10] (keeps bits 2+), unlinks the record from the free ring 0x215D7AC, links it at the tail of the active ring 0x215DA30 and increments the active count 0x215D7A8; no result | C |
| 0x938CB0 | `traffic_record_release` | EAX = traffic record: unlinks it from the active ring 0x215DA30, decrements 0x215D7A8 and returns it to the free ring 0x215D7AC; no result | C |
| 0x938D20 | `traffic_vehicle_hidden_check` | EAX = vehicle -> AL: 1 when flags [veh+0x8F50] show bit 0x10 together with bit 0x40 or 0x100; in single-player (0x2528B48 == 1 and 0x2528B49 == 0) otherwise passes the vehicle position (3 floats, copied to the stack) to 0x828CE0 | I |
| 0x938D80 | `traffic_vehicle_removable` | EAX = vehicle -> AL: 0 when the vehicle is driven by a human that belongs to a session player (byte [driver] == [session+0x1B0] and flag 1 of [driver+0x67C]) or has a live seat occupant, else continues with the despawn tests | I |
| 0x938FA0 | `traffic_record_find_by_vehicle` | EDI = vehicle handle -> EAX = record in the active ring 0x215DA30 whose [rec+8] equals the handle, else 0; no stack args | C |
| 0x938FD0 | `traffic_record_start_despawn` | cdecl (record*) -> void: resolves the record's vehicle (type 5) and sets its vehicle-ai force flag spawned_vehicle_despawning via vehicle_ai_set_force_flag_spawned_vehicle_despawning(&veh+0x8554, 1) when bit 0x40 of [veh+0x8565] is clear, then walks the seats (8 or [veh+0x6CE8]) to release the occupants | C |
| 0x9392A0 | `current_spawn_group_vehicle_index` | no args -> EAX: asks the current spawn group (0x215E5BC) for vtable slot 5 (vehicle info index) when 0x957660 allows it; -1 when there is no group, the index is negative or >= [0x252A0D8] | C |
| 0x9392E0 | `entity_budget_both_kinds_have_room` | ESI = count, no stack args -> AL: 0x929540(kind 0) must succeed, then (kind 1) must also succeed; AL = second result | C |
| 0x939930 | `traffic_vehicle_create_and_activate` | creates the traffic vehicle (vehicle_create), looks the object up (vehicle_from_handle) and registers its record with traffic_record_activate; cdecl | I |
| 0x93A2E0 | `traffic_spawn_attempt` | EAX = spawn request (read at entry), cdecl stack args; tries to create one traffic vehicle for the current spawn group 0x215E5BC: asks the group (vtable slot 5) for the vehicle info index, picks a position (0x939450 / 0x939670 / 0x9396C0), and creates the vehicle through 0x943DE0; returns AL | I |
| 0x93A6F0 | `traffic_spawn_attempt_at_spline_param` | cdecl: projects a point on a road spline (spline_closest_param) and tail-calls traffic_spawn_attempt with the parameter | I |
| 0x93A730 | `traffic_spawn_attempt_on_spline` | cdecl: evaluates a spline position (spline_eval_position), runs 0x929540 twice (budget kind 0 and 1) and calls traffic_spawn_attempt | I |
| 0x93A8A0 | `traffic_despawn_far_vehicles` | no args; walks the ring 0x215DA30 and calls traffic_record_start_despawn for vehicles that pass traffic_vehicle_removable and traffic_vehicle_hidden_check and whose vec3_distance_sq to the player exceeds a limit | I |
| 0x93BE50 | `traffic_despawn_update` | per-frame traffic despawn pass: timer 0xEB7A8C/90/94, 0x938EB0 candidate scan (twice), traffic_record_start_despawn and vehicle_take_damage for vehicles that are stuck or far | I |
| 0x93C1D0 | `ambient_vehicle_budget_remaining` | no args -> EAX: 0x20 - [0x252A0D4], minus 4 when a co-op partner exists (non-single-player branch) | C |
| 0x93C360 | `traffic_records_recount_valid` | no args: zeroes 0x2529624 then walks the ring 0x215DA30 counting records whose vehicle handle still resolves (type 5) | I |
| 0x93C610 | `ambient_spawn_timer_rearm` | no args: re-arms the spawn timer at 0xEB7A80 with timer_set_random (EDI = timer): host / single player 1900..2100 ms (center 2000 +- 100), clients 200..300 ms (250 +- 50) | C |
| 0x93C660 | `ambient_spawn_timer_update` | no args: if any of the six bytes at 0x215DA1C is clear it sets the timer 0xEB7A80 to -1, else when the timer is negative it re-arms it (tail jmp to ambient_spawn_timer_rearm) | C |
| 0x93CE20 | `spawn_system_init` | no args: clears the six-byte gate flags 0x215DA04..0x215DA24, sets the timer 0xEB7A80, loads the generic tables (0x946610), the distant ped / vehicle systems (0xB7DA30 / 0xB81130) and the spawn district data (0x943230) | I |
| 0x93D2B0 | `spawn_budget_class` | no args -> EAX: 0xC when single player or no co-op partner; 0 when the game mode object 0x27B3C60 reports mode 0xD; otherwise partner dependent | I |
| 0x93D4D0 | `traffic_spawn_gate_open` | no args -> AL: 1 when all six flag bytes at 0x215DA0C are set | C |
| 0x93D910 | `traffic_record_release_by_vehicle` | EDI = vehicle handle: finds the record like traffic_record_find_by_vehicle and tail-jumps to traffic_record_release (EAX = record); returns nothing when not found | C |
| 0x93D9B0 | `traffic_record_vehicle_removable` | EAX = traffic record -> AL: resolves the vehicle handle [rec+8] through the object table 0x2149C64 (type 5); returns 0 when the driver [veh+0xB0] belongs to a session player or when the handle is stale, else decides on the vehicle state | I |
| 0x93DBF0 | `traffic_records_despawn_removable` | no args: walks the active ring 0x215DA30; for every record whose vehicle passes traffic_record_vehicle_removable (EAX = record) it calls traffic_record_start_despawn(record) (cdecl) | I |
| 0x93DE60 | `spawn_points_search_in_range` | cdecl (stack: ebp+8 = position/out ptr, +0xC = f32 range, +0x10 = f32 range2, +0x18 ptr, +0x28 filter id, ...) -> AL: searches the spawn point table 0x2529E20 (count [+0x10], array [+0x14]) for points inside the two squared ranges (fmul st0,st0 of the f32 args) honouring the optional filter (0xFFFF = none); 0x584 bytes of locals | I |
| 0x93E8D0 | `coop_spawn_count_scale` | stack (f32 factor, int base) cdecl -> EAX: base value scaled for the camera / co-op partner distance (reads 0x25F5B14..1C and the partner's position at +0x12A0) | I |
| 0x93EC20 | `spawn_node_refresh_for_human` | stack (node, human) cdecl: refreshes one spawn node (key [0], category list) for one human against the district spawn info [0x2529634]; uses the human's position at +0x14 unless it is the local player | I |
| 0x93EDA0 | `spawn_node_refresh_all_humans` | stack (node) cdecl: calls spawn_node_refresh_for_human for every human in the ring 0x217043C (link +0x1294) | C |
| 0x93EFA0 | `spawner_add_category` | EAX = spawner (key [0], list count [+0xC], max 0x14 entries at +0x10), ECX = category: appends the category when its key array ([cat+0x20], [cat+0x1C] entries) contains the spawner key and it is not listed yet; sets [spawner+8] = category when [cat+0x28] != -1 | C |
| 0x93F000 | `spawner_add_info_categories` | EAX = spawner, ESI = spawn info (row count [+4], rows [+8] stride 0x1C, default [+0x10]): calls spawner_add_category for each row's first dword and tail-jumps with the default category | C |
| 0x93F030 | `spawner_rebuild_categories` | EDI = spawner: clears [+0xC] / [+8], adds the categories of the current district spawn info [0x2529634], of the co-op partner's [0x2529638] if different, and of the four fixed infos at [0xE888B8 + 4i] | C |
| 0x93F0B0 | `spawners_rebuild_all` | no args: walks the ring of spawn nodes 0xEB7AC4 (link +0x1F8) against every human (ring 0x217043C, link +0x1294) with 0x93EC20, the second ring 0xEB7B14 with 0x93EDD0, and the spawner ring 0x2BE0A0C (link +0x60) with spawner_rebuild_categories | C |
| 0x93F150 | `spawn_info_refresh_timer_check` | no args: when the 20000 ms timer 0xEB7B54 has elapsed (wrap-safe compare against 0xE843AC, modulus 0x6B49D200) it re-arms it (+0x4E20) and sets the byte 0x2529622 | C |
| 0x93F1A0 | `spawn_phase_step_mod3` | no args -> AL: returns 1 when [0x252962C] % 3 == 0 (value before the step), then stores ([0x252962C] + 1) % 3 | C |
| 0x93F4D0 | `spawn_info_for_player_district` | EDI = human (player) -> EAX = spawn info of the district containing the human: district_index_at_point(pos, 0) -> district record (0xA8 bytes from [0x2526C94]) +0x74; falls back to [0x2526C84] / [0x2526C7C] (single player city defaults); the camera position is used for the local player | C |
| 0x9406C0 | `spawn_nodes_service` | cdecl (flag): updates the spawner ring 0x2BE0A0C: 0x940600 / 0x93FEB0 / 0x93F9F0 / 0x93FE70 / 0x940690 / 0x940570 / 0x9401A0 (add, update and release of spawner nodes); uses allocator num_free_73 | I |
| 0x940860 | `spawn_node_update_gate_counts` | ECX = node, stack (bool add): marks which of six categories the node's slots belong to (array at +0x18 / +0x10C, field +0x28) and adds or subtracts 1 to the six counters at 0x215DA44, 0x215DA5C, 0x215DA74, 0x215DA8C, 0x215DAA4, 0x215DABC | C |
| 0x940990 | `spawn_node_release` | ESI = node: releases its streaming reference (rsched_release_net) and frees it to the pool (0x9450C0) when the use count reaches zero; also touches 0x2528B2B/56 session flags | I |
| 0x940AB0 | `spawn_node_acquire_ped` | stack (key) cdecl: finds the node with this key in the ring 0xEB7AC4 and increments its use count [+0xC]; otherwise allocates one from the pool 0xEB7AB8 (0x944FD0), clears 0x1F8 bytes, sets [+0xC] = 1 and [+0] = key, refreshes it for all humans, updates the gate counters (0x940860) and adds a streaming reference with rsched_add_ref_net | C |
| 0x940B40 | `spawn_node_acquire_vehicle` | stack (key) cdecl: vehicle counterpart of spawn_node_acquire_ped: ring 0xEB7B14, pool 0xEB7B08, spawn_node_refresh via 0x93EDD0 and a streaming reference | I |
| 0x940F00 | `spawn_node_acquire_by_kind` | dispatches on spawn_kind_to_budget_slot to one of 0x940C00 / 0x940CD0 / 0x940DE0 / 0x940E70 (acquire or release variants for each node kind) | I |
| 0x942060 | `spawn_nodes_refresh_pass` | cdecl (bool): per-node pass over the ped node ring 0xEB7AC4: 0x941D10 (placement), allocator<>::num_used_72, 0x941990, 0x940990, 0x942010 | I |
| 0x943230 | `spawn_group_names_hash_init` | no args: hashes the district spawn group names ('City - Gang - Saints', 'City - Gang - Brotherhood', ...) with string_hash_lower and stores the ids at 0x252963C..0x2529648 and 0x2BE0A4C.. | C |
| 0x9432F0 | `spawn_system_init_districts` | no args: resolves the first district spawn info (93F4D0), builds the ped/vehicle spawn nodes, hosts the session sync (session_hosting_flag) and runs spawners_rebuild_all; part of spawn_system_init | I |
| 0x9433F0 | `spawn_streaming_update` | cdecl (bool force): per-frame district/streaming driver: stores spawn_info_for_player_district for the local player in 0x2529634 and for the co-op partner in 0x2529638, calls spawners_rebuild_all, handles the 5 s spawn-info timer 0xEB7B58 and, when the byte 0x2529622 or `force` is set, runs the node updates 0x942060, 0x9406C0 and 0x9410F0 | C |
| 0x946790 | `spawn_stream_budget_add` | cdecl (bool a, bool b, int amount): distributes `amount` extra budget over the nine words 0xE887BC..0xE887DC after querying 0xA77000 (resource size for the slot, with 0x2F9B2E8); publishes through 0xA79970 | I |
| 0x946AC0 | `spawn_stream_budget_compute` | no args: unless a cutscene blocks the game it zeroes the nine budget words 0xE887BC..0xE887DC, then (single player, or no host session player) sets word 0 = 10 and returns; otherwise sets word 3 = 1, words by the notoriety tier 0x279AB14 (2..4 -> word 6 = 1; 5 -> word 6 = 3), calls 0x946880 and clamps words 0..2 between the tables 0x215DFB8 / 0x215DFDC (stride 0xC), finally word 0 = 10 - (sum of the other words + 1) when that sum is <= 10 | C |
| 0x9473B0 | `spawn_stream_budget_publish` | no args: copies the nine budget words 0xE887BC..0xE887DC into the block at [0x2F9B3B4] and four more words 0xE887E0/E4/E8/EC into the blocks at [0x2F9B488], [0x2F9B55C], [0x2F9B630], [0x2F9B704]; for every word that changed it stores the value and ORs 2 into the block's dirty byte (0x2F9B3B8, 0x2F9B48C, 0x2F9B560, 0x2F9B634, 0x2F9B708) | C |
| 0x947690 | `spawn_kind_to_budget_slot` | EAX = kind (0..6) -> EAX slot: table via jmp [eax*4+0x9476C0]: 0->2, 1->6, 2->4, 3->5, 4->0, 5->3, 6->3, others 0 | C |
| 0x94EA50 | `spawn_records_cull_out_of_view` | no args: for the spawn record list 0x215E090 calls spawn_record_any_member_visible and culls the records with no visible member (via 0x94E820) | I |
| 0x94EB60 | `spawn_response_slot_pick` | stack (int) cdecl -> EAX slot index or -1: picks which response slot to use from the record list state (0x215E0A4/0xAC) | I |
| 0x94EC60 | `spawn_response_slots_update` | cdecl (bool): updates the 4 response slots at 0xE88768 (stride 0x10: vehicle type index, character type index, flags): requests or releases the vehicle / character streaming for each slot with 0x944120 and rsched_add_ref_net / rsched_release_net; host only | I |
| 0x9502B0 | `spawn_record_create_members` | creates the member humans / vehicles of one spawn record at its placement (team_from_name through 0x946710, 0x9CFCE0, 0xAEFDA0 / 0xAEFC40, 0xAB1000, 0xA214C0, vec3_normalize_safe, 0x9466D0) | I |
| 0x9511B0 | `spawn_records_update` | cdecl: per-frame update of the spawn records: 0x94F450, 0x94F150, 0x950D00 (spline walk), 0x6775B0; reads 0x215E090/094 | I |
| 0x951A90 | `spawn_records_reset` | cdecl (bool): resets the spawn record list 0x215E090, the 4-entry response slot table 0xE8876C (stride 0x10), the ring sentinel 0x2C180E8 and the id slots 0x215E0A4..0x215E0B8; the bool registers the two reset callbacks via 0x801810 | C |
| 0x951B40 | `spawn_records_resolve_templates` | no args: resolves the object template 0xE3A71C (spike strip / roadblock props) and the four template names 0xE3A728..0xE3A740 through 0x6ADD50, stores the results at 0x215E0A0 / 0x215E08C / 0x215E088 / 0x215E084 / 0x215E080 and the template extents at 0x215E09C / 0x215E098 | I |
| 0x955830 | `spawn_group_prune_members` | cdecl (group*): walks the 8 member entries, drops members whose object is dead (hit points [obj+0x1124] <= 0) or gone, and recomputes the counters [group+0x1DC] / [group+0x1E0] | I |
| 0x955B10 | `spawn_groups_find_active_priority` | no args -> EAX: index of the first active group in the order 2, 3, 8 (vtable slot 11 is_active), else -1 | C |
| 0x956F00 | `spawn_groups_find_member_by_object` | EAX = object: scans all 17 groups 0x215E578.. for a member entry whose handle (+0x60 + 0x30 i) equals [obj+0x44]; EAX = entry or 0 | I |
| 0x957000 | `current_spawn_group_alloc_member` | stdcall-free cdecl (stack: object*) -> EAX: takes the first free member entry (in-use byte 0, stride 0x30 from +0x5C, 8 entries) of the CURRENT spawn group [0x215E5BC], marks it in use, stores the object id [obj+0x44] into it (+0x60), clears +0x68..+0x88 (relative to the entry's group offset 0x30 i) and sets +0x64 = -1; returns the entry pointer (group+0x5C+0x30 i) or 0 when the group is null or full | C |
| 0x957550 | `spawn_groups_reset_all` | no args: calls vtable slot 8 (reset) on each of the 17 spawn groups 0x215E578..0x215E5B8 and clears the current group 0x215E5BC | C |
| 0x957660 | `current_spawn_group_query` | no args -> AL: false without a current group; otherwise combines vtable slots 13 (+0x34) and 14 (+0x38) of the current group | I |
| 0x9576C0 | `current_spawn_group_vehicle_is_type3` | no args -> AL: current group's vehicle info (slot 5) < [0x252A0D8] and the vehicle info record (stride 0x7D0 at 0x2FAD1F8, field +0x9C) equals 3 | C |

#### data (2)

| Address | Name | Notes | Conf |
|---|---|---|---|
| 0x93CC20 | `xtbl_parse_abandon_despawn_info` | xtbl element parser: strings 'abandon_despawn_info' and 'num_cars' (xtbl_get_int into 0x215D9F0..F8, 0xE87134/38); recursive through the element tree | C |
| 0x946610 | `spawn_load_generic_tables` | no args: looks up two names with string_hash_lower and loads generic_characters.xtbl and generic_vehicles.xtbl | C |

#### util (2)

| Address | Name | Notes | Conf |
|---|---|---|---|
| 0x944FD0 | `pool_alloc_node_edi` | EDI = pool allocator (+0x20 enabled flag, +0x14 count, +0x30 body) -> EAX = node or 0 | I |
| 0x9450C0 | `pool_free_node_edi_esi` | EDI = pool allocator, ESI = node: returns the node to the pool; no result | I |

#### world (9)

| Address | Name | Notes | Conf |
|---|---|---|---|
| 0x947990 | `parked_vehicle_count_other_than` | stack (record*) cdecl -> EAX: number of parked vehicle records in the lists 0x215E06C / 0x215E070 (next at +0x50) other than the argument; counts one more for a record whose vehicle (+0x6C) is alive | I |
| 0x947A40 | `parked_vehicle_array_count_valid` | no args -> EAX: counts the entries of the array 0x215E030 (count 0x215E060, 8-byte entries) whose record vehicle (+0x6C) resolves to a live vehicle | I |
| 0x948D80 | `parked_vehicles_quick_update` | no args: parked-vehicle release / visibility step (0x9480A0, 0x947F40) plus parked_vehicles_spawn_update(0); used by the load-time spawn passes | I |
| 0x948DB0 | `parked_vehicles_flush_all` | no args: releases every parked vehicle record (five 0x947AE0 calls with -1) and resets the lists 0x215E064..0x215E074 | I |
| 0x948F90 | `parked_vehicle_create` | cdecl: creates a parked vehicle for a parking spot (vehicle_create, vehicle_from_handle, Havok / physics setup) | I |
| 0x9581B0 | `el_train_place_cars_on_spline` | places the train cars along the spline (spline_eval_position / spline_eval_tangent, vec3_distance, sqrtf_wrapper) | I |
| 0x958840 | `el_train_spawn` | spawns the El train (strings 'El Train Front', 'El Train Rear'): vehicle_create for both cars, vehicle_set_force_flag_no_accel_limit, links them to the track spline and registers with 0x958050 | C |
| 0x959B50 | `el_train_update` | per-train movement: spline car positions (0x9581B0 / 0x958150), vec3_distance_sq to the camera and _ftol2_sse | I |
| 0x95B2C0 | `el_trains_update_one` | per-train update step over the list 0x215E5C0: 0x95AFD0, vcalls and the spline car updates 0x958150 | I |

### Renames of placeholders (docs/renames_spawning.csv)

| Address | Old name | New name | Conf |
|---|---|---|---|
| 0x567070 | `special_vehicle_spawn_group::vf1` | `special_vehicle_spawn_group::get_scale_factor_default` | C |
| 0x567080 | `special_vehicle_spawn_group::vf2` | `special_vehicle_spawn_group::get_offset_value_default` | C |
| 0x567090 | `special_vehicle_spawn_group::vf3` | `special_vehicle_spawn_group::set_params_ignored_default` | C |
| 0x5670A0 | `special_vehicle_spawn_group::vf15` | `special_vehicle_spawn_group::needs_player_slot_valid_default` | C |
| 0x5670B0 | `special_vehicle_spawn_group::vf22` | `special_vehicle_spawn_group::get_template_override_default` | I |
| 0x5670C0 | `special_vehicle_spawn_group::vf24` | `special_vehicle_spawn_group::restart_timers_default` | C |
| 0x5670D0 | `special_vehicle_spawn_group::vf25` | `special_vehicle_spawn_group::flag25_default` | I |
| 0x5670E0 | `special_vehicle_spawn_group::vf26` | `special_vehicle_spawn_group::flag26_default` | I |
| 0x5670F0 | `skirmish_spawn_group::vf20` | `skirmish_spawn_group::flag20_default` | I |
| 0x567100 | `skirmish_spawn_group::vf19` | `skirmish_spawn_group::flag19_default` | I |
| 0x567110 | `skirmish_spawn_group::vf21` | `skirmish_spawn_group::get_type_id` | C |
| 0x567120 | `skirmish_spawn_group::vf23` | `skirmish_spawn_group::get_member_limit` | I |
| 0x5672D0 | `skirmish_spawn_group::vf16` | `skirmish_spawn_group::register_member_human` | I |
| 0x567730 | `skirmish_spawn_group::vf12` | `skirmish_spawn_group::is_eligible_to_spawn` | C |
| 0x567750 | `skirmish_spawn_group::vf18` | `skirmish_spawn_group::get_streaming_priority_default` | I |
| 0x567F50 | `skirmish_spawn_group::vf17` | `skirmish_spawn_group::unregister_member_human` | I |
| 0x667CB0 | `special_vehicle_spawn_group::vf17` | `special_vehicle_spawn_group::unregister_member_human_default` | C |
| 0x667CC0 | `special_vehicle_spawn_group::vf19` | `special_vehicle_spawn_group::flag19_default` | I |
| 0x667CD0 | `special_vehicle_spawn_group::vf21` | `special_vehicle_spawn_group::get_type_id_default` | I |
| 0x667CE0 | `special_vehicle_spawn_group::vf23` | `special_vehicle_spawn_group::get_member_limit` | I |
| 0x667D00 | `vandalism_news_spawn_group::vf1` | `vandalism_news_spawn_group::get_scale_factor` | C |
| 0x667D10 | `vandalism_news_spawn_group::vf2` | `vandalism_news_spawn_group::get_offset_value` | C |
| 0x667D20 | `vandalism_news_spawn_group::vf20` | `vandalism_news_spawn_group::flag20_default` | I |
| 0x669960 | `vandalism_news_spawn_group::vf12` | `vandalism_news_spawn_group::is_eligible_to_spawn` | C |
| 0x669970 | `vandalism_news_spawn_group::vf24` | `vandalism_news_spawn_group::restart_timers` | C |
| 0x669B50 | `vandalism_news_spawn_group::vf18` | `vandalism_news_spawn_group::get_streaming_priority_default` | I |
| 0x669B60 | `vandalism_news_spawn_group::vf16` | `vandalism_news_spawn_group::register_member_human` | I |
| 0x6785A0 | `escort_spawn_group::vf20` | `escort_spawn_group::flag20_default` | I |
| 0x6785C0 | `escort_enemy_spawn_group::vf20` | `escort_enemy_spawn_group::flag20_default` | I |
| 0x678810 | `escort_spawn_group::vf18` | `escort_spawn_group::get_streaming_priority_default` | I |
| 0x678820 | `escort_spawn_group::vf17` | `escort_spawn_group::unregister_member_human` | I |
| 0x6789E0 | `escort_spawn_group::vf16` | `escort_spawn_group::register_member_human` | I |
| 0x678FA0 | `escort_spawn_group::vf12` | `escort_spawn_group::is_eligible_to_spawn` | I |
| 0x6794F0 | `escort_enemy_spawn_group::vf12` | `escort_enemy_spawn_group::is_eligible_to_spawn` | I |
| 0x679DE0 | `escort_enemy_spawn_group::vf16` | `escort_enemy_spawn_group::register_member_human` | I |
| 0x67A120 | `escort_enemy_spawn_group::vf18` | `escort_enemy_spawn_group::get_streaming_priority_default` | I |
| 0x67A130 | `escort_enemy_spawn_group::vf17` | `escort_enemy_spawn_group::unregister_member_human` | I |
| 0x782FF0 | `cell_spawn_group::vf20` | `cell_spawn_group::flag20_default` | I |
| 0x784180 | `cell_spawn_group::vf16` | `cell_spawn_group::register_member_human` | I |
| 0x7841E0 | `cell_spawn_group::vf0` | `cell_spawn_group::find_member_by_object` | C |
| 0x784590 | `cell_spawn_group::vf18` | `cell_spawn_group::get_streaming_priority_default` | I |
| 0x7845A0 | `cell_spawn_group::vf21` | `cell_spawn_group::get_type_id` | C |
| 0x7845B0 | `cell_spawn_group::vf22` | `cell_spawn_group::get_template_override` | I |
| 0x7845E0 | `cell_spawn_group::vf12` | `cell_spawn_group::is_eligible_to_spawn` | C |
| 0x784620 | `cell_spawn_group::vf4` | `cell_spawn_group::get_override_value` | C |
| 0x921F20 | `notoriety_spawn_group::vf25` | `notoriety_spawn_group::flag25_default` | I |
| 0x921F30 | `notoriety_spawn_group::vf20` | `notoriety_spawn_group::flag20_default` | I |
| 0x921F40 | `notoriety_spawn_group::vf26` | `notoriety_spawn_group::flag26_default` | I |
| 0x921F50 | `notoriety_spawn_group_police_heli::vf15` | `notoriety_spawn_group_police_heli::needs_player_slot_valid` | C |
| 0x922E10 | `notoriety_spawn_group::vf27` | `notoriety_spawn_group::pick_member_if_below` | C |
| 0x923050 | `notoriety_spawn_group::vf28` | `notoriety_spawn_group::flag28_default` | I |
| 0x923060 | `notoriety_spawn_group::vf29` | `notoriety_spawn_group::can_spawn_now_default` | I |
| 0x923610 | `notoriety_spawn_group::vf18` | `notoriety_spawn_group::get_streaming_priority` | C |
| 0x923620 | `notoriety_spawn_group::vf16` | `notoriety_spawn_group::register_member_human` | I |
| 0x923960 | `notoriety_spawn_group::vf0` | `notoriety_spawn_group::find_member_by_object` | C |
| 0x923C10 | `notoriety_spawn_group::vf5` | `notoriety_spawn_group::get_vehicle_info_index` | C |
| 0x923C30 | `notoriety_spawn_group::vf6` | `notoriety_spawn_group::request_slot_resources` | I |
| 0x923C90 | `notoriety_spawn_group::vf3` | `notoriety_spawn_group::set_params_ignored_default` | C |
| 0x923CF0 | `notoriety_spawn_group::vf21` | `notoriety_spawn_group::get_type_id` | C |
| 0x923D00 | `notoriety_spawn_group::vf1` | `notoriety_spawn_group::get_scale_factor` | C |
| 0x923D50 | `notoriety_spawn_group::vf2` | `notoriety_spawn_group::get_offset_value` | C |
| 0x923DB0 | `notoriety_spawn_group_police_heli::vf29` | `notoriety_spawn_group_police_heli::can_spawn_now` | I |
| 0x923E30 | `notoriety_spawn_group_police_heli::vf12` | `notoriety_spawn_group_police_heli::is_eligible_to_spawn` | I |
| 0x923F00 | `notoriety_spawn_group_police_heli::vf24` | `notoriety_spawn_group_police_heli::restart_timers` | C |
| 0x923F50 | `notoriety_spawn_group_police_heli::vf27` | `notoriety_spawn_group_police_heli::pick_member_if_below` | I |
| 0x923FA0 | `notoriety_spawn_group_police::vf29` | `notoriety_spawn_group_police::can_spawn_now` | I |
| 0x924050 | `notoriety_spawn_group_police::vf21` | `notoriety_spawn_group_police::get_type_id` | I |
| 0x9240B0 | `notoriety_spawn_group_police::vf12` | `notoriety_spawn_group_police::is_eligible_to_spawn` | I |
| 0x924230 | `notoriety_spawn_group_gang::vf28` | `notoriety_spawn_group_gang::flag28` | I |
| 0x924310 | `notoriety_spawn_group_gang::vf29` | `notoriety_spawn_group_gang::can_spawn_now` | I |
| 0x924370 | `notoriety_spawn_group_gang::vf12` | `notoriety_spawn_group_gang::is_eligible_to_spawn` | I |
| 0x9244A0 | `notoriety_spawn_group::vf13` | `notoriety_spawn_group::is_resource_loaded` | I |
| 0x949E60 | `parking_spot::vf1` | `parking_spot::dtor` | I |
| 0x9561F0 | `special_vehicle_spawn_group::vf0` | `special_vehicle_spawn_group::find_member_by_object` | C |
| 0x956410 | `special_vehicle_spawn_group::vf16` | `special_vehicle_spawn_group::register_member_human` | I |
| 0x956590 | `special_vehicle_spawn_group::vf13` | `special_vehicle_spawn_group::is_resource_loaded` | I |
| 0x956620 | `special_vehicle_spawn_group::vf14` | `special_vehicle_spawn_group::get_free_member_count` | C |
| 0x956690 | `special_vehicle_spawn_group::vf5` | `special_vehicle_spawn_group::get_vehicle_info_index` | C |
| 0x9566B0 | `special_vehicle_spawn_group::vf6` | `special_vehicle_spawn_group::get_member_slot` | C |
| 0x9570B0 | `special_vehicle_spawn_group::vf4` | `special_vehicle_spawn_group::get_override_value_default` | I |

## Large functions: port maps

The ten largest portable functions in this area, by true size. They are deterministic code over game globals, so they can be hooked with a register shim and shadow-checked like the earlier ports.

| # | Address | Name | True size | Callees | Blocks | Tables |
|---|---|---|---|---|---|---|
| 1 | 0xB7CD00 | `distant_peds_spawn_update` | 0xBD9 (3033) | 21 | 146 | 0 |
| 2 | 0xB80130 | `distant_vehicles_spawn_update` | 0x9DD (2525) | 26 | 105 | 0 |
| 3 | 0x936AB0 | `ambient_peds_despawn_check` | 0x8F8 (2296) | 13 | 84 | 1 |
| 4 | 0x93DE60 | `spawn_points_search_in_range` | 0x8BB (2235) | 18 | 74 | 0 |
| 5 | 0x93C690 | `traffic_spawn_update` | 0x53A (1338) | 26 | 82 | 0 |
| 6 | 0x9502B0 | `spawn_record_create_members` | 0x52F (1327) | 10 | 62 | 0 |
| 7 | 0x93A2E0 | `traffic_spawn_attempt` | 0x405 (1029) | 31 | 55 | 0 |
| 8 | 0x939F30 | `traffic_vehicle_spawn_try` | 0x393 (915) | 19 | 34 | 0 |
| 9 | 0x93BE50 | `traffic_despawn_update` | 0x368 (872) | 7 | 75 | 1 |
| 10 | 0x937FF0 | `ambient_peds_spawn_pass` | 0x332 (818) | 8 | 66 | 1 |

### distant_peds_spawn_update - 0xB7CD00

**Role.** Distant (impostor) pedestrians: allocates the distant-ped record pool (`allocator<>::num_used_88`), updates up to the frame budget from the rng table `0xEC3ABC`, removes or creates records by distance to the player (`vec3_distance`, `floorf`, `sqrtf`, `_ftol2_sse`) and the streaming state `0x25295FF`; it is the tail-jumped last call of `world_spawn_do_frame`.

**Convention.** Entry registers read (linear scan): none. True size 0xBD9, 917 instructions, 146 blocks, 0 jump table(s). Callers: none direct (tail jump or table).

**Sections, in order of the calls (address and callee):** 0xB7CD45 sub_B7C250; 0xB7CD9C sub_BDE880; 0xB7CE7D timer_elapsed; 0xB7CED2 sub_B7B2D0; 0xB7CEE0 sub_B7DC00; 0xB7CEEE allocator<>::num_used_88; 0xB7CF46 __alloca_probe_16; 0xB7CF68 sub_A72800; 0xB7CF7D sub_4A7C60; 0xB7CFA5 vec3_distance; 0xB7CFBF _ftol2_sse; 0xB7CFEE sub_B7D8E0; 0xB7D032 sub_B7D8E0; 0xB7D04E sub_4A7C90; 0xB7D0A8 sub_B7D8E0; 0xB7D0B9 sub_B7D8E0; 0xB7D137 _ftol2_sse; 0xB7D168 vec3_distance; 0xB7D17E _ftol2_sse; 0xB7D27C sub_BE7A10; 0xB7D2D5 sqrtf_wrapper; 0xB7D2E0 _ftol2_sse; 0xB7D337 sqrtf_wrapper; 0xB7D342 _ftol2_sse; 0xB7D3B6 sqrtf_wrapper; 0xB7D3C1 _ftol2_sse; 0xB7D437 sub_B7CCA0; 0xB7D45B sub_BE7A10; 0xB7D4C6 sqrtf_wrapper; 0xB7D4D1 _ftol2_sse; 0xB7D519 sqrtf_wrapper; 0xB7D524 _ftol2_sse; 0xB7D53F sub_B7CCA0; 0xB7D557 sub_B7CCA0; 0xB7D567 sub_B7CCA0; 0xB7D57C sub_B7CCA0; 0xB7D5C9 sqrtf_wrapper; 0xB7D5D4 _ftol2_sse; 0xB7D64A sub_B7CCA0; 0xB7D68D floorf_wrapper; 0xB7D695 _ftol2_sse; 0xB7D80C sub_B7DB50; 0xB7D860 sub_B7B170; 0xB7D874 sub_68A500; 0xB7D8D0 __security_check_cookie.

**Hazards.** Large `alloca` frame (`__alloca_probe_16`) and the stack cookie `0xE80B50`; the random table index `0xEC3ABC` advances by one per draw and wraps at 0x800, so the draw order must be kept to stay deterministic; 21 direct callees, 17 of them unnamed (0xB7C250, 0xB7B2D0, 0xB7DC00, 0xB7D8E0, 0xB7DB50, 0xB7B170 and so on) that must be ported or called by address.

**Globals read:** 0xE80B50, 0xE843AC, 0xEC3ABC, 0x21703D4, 0x22DBC08, 0x22FD858, 0x22FD864, 0x25295FF, 0x2529E20, 0x319BF84.

**Globals written:** 0xEBD0BC, 0xEC3ABC, 0x25295FF.

```
FUNCTION 0x00B7CD00  true size 0xBD9 (3033 bytes)  insns 917  blocks 146
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x174

  B0 @0xb7cd00:
    b7cd09: READ i32[0xe80b50]   ; mov eax, dword ptr [0xe80b50]
    b7cd13: READ i32[0x2529e20]   ; cmp dword ptr [0x2529e20], 0
    b7cd1d: if (dword ptr [0x2529e20] == 0) goto B145  else B1   ; je 0xb7d8c2

  B1 @0xb7cd23:
    b7cd25: goto B2 @0xb7cd30

  B2 @0xb7cd30:
    b7cd37: if (byte ptr [eax + 0x215da04] == 0) goto B145  else B3   ; je 0xb7d8c2

  B3 @0xb7cd3d:
    b7cd43: if (eax < (unsigned) 6) goto B2  else B4   ; jb 0xb7cd30

  B4 @0xb7cd45:
    b7cd45: CALL 0xb7c250 sub_b7c250  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7cd56: STACK-OBJECT &[ebp-0x7c] -> eax
    b7cd5a: STACK-OBJECT &[ebp-0x58] -> eax
    b7cd5d: X87 STORE f32 [ebp - 0x3c] <- (f32[0x25f5b14] - f32[0x25f5b2c])
    b7cd60: STACK-OBJECT &[ebp-0x3c] -> ecx
    b7cd6f: X87 STORE f32 [ebp - 0x38] <- (f32[0x25f5b18] - f32[0x25f5b30])
    b7cd7e: X87 STORE f32 [ebp - 0x34] <- (f32[0x25f5b1c] - f32[0x25f5b34])
    b7cd87: X87 STORE f32 [ebp - 0x58] <- f32[0x25f5b50]
    b7cd90: X87 STORE f32 [ebp - 0x54] <- f32[0x25f5b54]
    b7cd99: X87 STORE f32 [ebp - 0x50] <- f32[0x25f5b58]
    b7cd9c: CALL 0xbde880 sub_bde880  callee-reads[eax, ecx]  regs[eax<-(lea eax, [ebp - 0x58]), ecx<-(lea ecx, [ebp - 0x3c])]  stack(first arg first)[eax]  callee-ret-imm=None
    b7cda3: caller cleans 4 bytes
    b7cda6: READ i32[0x21703d4]   ; cmp dword ptr [0x21703d4], 0
    b7cdb9: X87 STORE f32 [ebp - 0x14] <- (f32[0xe87054] / f32[0xe8437c])
    b7cdc8: X87 STORE f32 [ebp - 0x48] <- f32[ebp - 0x7c]
    b7cdce: X87 fmul st(1)  (st(i) dest, untracked)
    b7cdd0: X87 STORE f32 [ebp - 0x44] <- f32[ebp - 0x78]
    b7cdd6: X87 STORE f32 [ebp - 0x40] <- ((f32[ebp - 0x14] * ST(st(0))) * f32[ebp - 0x74])
    b7cde2: X87 STORE f32 [ebp - 0x3c] <- (f32[ebp - 0x48] + f32[0x25f5b14])
    b7cdee: X87 STORE f32 [ebp - 0x38] <- (f32[ebp - 0x44] + f32[0x25f5b18])
    b7cdfa: X87 STORE f32 [ebp - 0x34] <- (f32[ebp - 0x40] + f32[0x25f5b1c])
    b7cdfd: if (dword ptr [0x21703d4] == 0) goto B6  else B5   ; je 0xb7ce07

  B5 @0xb7cdff:
    b7ce05: goto B7 @0xb7ce0d

  B6 @0xb7ce07:

  B7 @0xb7ce0d:
    b7ce0d: X87 STORE f32 [ebp - 0x24] <- ret(?)
    b7ce27: X87 STORE f32 [ebp - 0x2c] <- f32[0xe8704c]
    b7ce33: X87 COMPARE f32[0xe87048] vs STst(1)   ; fcom st(1)
    b7ce3c: if (x87 compare ('f32[0xe87048]', 'STst(1)'): ST0 outcome in {gt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B9  else B8   ; je 0xb7ce43

  B8 @0xb7ce3e:
    b7ce3e: X87 STORE f32 [ebp - 0x2c] <- ret(?)
    b7ce41: goto B10 @0xb7ce45

  B9 @0xb7ce43:

  B10 @0xb7ce45:
    b7ce45: READ u8[0x25295ff]   ; cmp byte ptr [0x25295ff], 0
    b7ce52: X87 fmul st(1)  (st(i) dest, untracked)
    b7ce54: X87 STORE f32 [ebp - 0x4c] <- f64(110.0; bits 0x405B800000000000)
    b7ce60: X87 STORE f32 [0x252a170] <- (f32[0xe87050] + f32[ebp - 0x2c])
    b7ce6c: X87 STORE f32 [0x252a16c] <- (ret(?) * f64(90.0; bits 0x4056800000000000))
    b7ce72: if (byte ptr [0x25295ff] != 0) goto B145  else B11   ; jne 0xb7d8c2

  B11 @0xb7ce78:
    b7ce7d: CALL 0xbdd230 timer_elapsed  callee-reads[eax]  regs[eax<-(mov eax, 0xebd0bc)]  stack(first arg first)[]  callee-ret-imm=0
    b7ce84: if (al == 0) goto B145  else B12   ; je 0xb7d8c2

  B12 @0xb7ce8a:
    b7ce8a: READ i32[0xe843ac]   ; mov eax, dword ptr [0xe843ac]
    b7ce99: WRITE i32[0xebd0bc] mov = eax   ; dword ptr [0xebd0bc], eax
    b7ce9e: if (eax <= (signed) 0x6b49d200) goto B14  else B13   ; jle 0xb7ceaa

  B13 @0xb7cea0:
    b7cea5: WRITE i32[0xebd0bc] mov = eax   ; dword ptr [0xebd0bc], eax

  B14 @0xb7ceaa:
    b7ceaa: READ i32[0x319bf84]   ; mov esi, dword ptr [0x319bf84]
    b7ceb0: WRITE u8[0x25295ff] mov = 0x1   ; byte ptr [0x25295ff], 1

  B15 @0xb7ceb7:
    b7ceb9: if (esi == 0) goto B19  else B16   ; je 0xb7cee9

  B16 @0xb7cebb:
    b7cec0: READ i32[0x319bf84]   ; sub ebx, dword ptr [0x319bf84]
    b7cec6: STACK-OBJECT &[ebp-0x58] -> ecx
    b7cecf: STACK-OBJECT &[ebp-0x3c] -> eax
    b7ced2: CALL 0xb7b2d0 sub_b7b2d0  callee-reads[esi]  regs[esi<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    b7ced9: if (al == 0) goto B18  else B17   ; je 0xb7cee5

  B17 @0xb7cedb:
    b7cee0: CALL 0xb7dc00 sub_b7dc00  callee-reads[edi]  regs[edi<-(mov edi, 0x319bf78)]  stack(first arg first)[]  callee-ret-imm=None

  B18 @0xb7cee5:
    b7cee7: goto B15 @0xb7ceb7

  B19 @0xb7cee9:
    b7ceee: CALL 0xb7dd70 allocator<>::num_used_88  callee-reads[ecx]  regs[ecx<-(mov ecx, 0x319bf78)]  stack(first arg first)[]  callee-ret-imm=0
    b7cefa: READ i32[0x22fd858]   ; mov eax, dword ptr [0x22fd858]
    b7cf04: if (eax == 0) goto B21  else B20   ; je 0xb7cf0b

  B20 @0xb7cf06:
    b7cf09: if (eax != 1) goto B22  else B21   ; jne 0xb7cf10

  B21 @0xb7cf0b:

  B22 @0xb7cf10:
    b7cf13: if (eax != 3) goto B24  else B23   ; jne 0xb7cf22

  B23 @0xb7cf15:
    b7cf15: READ i32[0x22fd864]   ; cmp dword ptr [0x22fd864], 2
    b7cf1c: if (dword ptr [0x22fd864] < (signed) 2) goto B145  else B24   ; jl 0xb7d8c2

  B24 @0xb7cf22:
    b7cf24: if (ecx <= 0 (signed)) goto B145  else B25   ; jle 0xb7d8c2

  B25 @0xb7cf2a:
    b7cf2d: if (ecx <= (signed) 0x19) goto B27  else B26   ; jle 0xb7cf36

  B26 @0xb7cf2f:

  B27 @0xb7cf36:
    b7cf36: READ i32[0x2529e20]   ; mov ecx, dword ptr [0x2529e20]
    b7cf46: CALL 0xcb35c0 __alloca_probe_16  callee-reads[eax]  regs[eax<-(lea eax, [esi*4])]  stack(first arg first)[]  callee-ret-imm=0
    b7cf53: X87 STORE f32 [esp + 4] <- f32[ebp - 0x2c]
    b7cf57: STACK-OBJECT &[ebp-0x58] -> edi
    b7cf60: X87 STORE f32 [esp] <- f32[ebp - 0x4c]
    b7cf65: STACK-OBJECT &[ebp-0x3c] -> esi
    b7cf68: CALL 0xa72800 sub_a72800  callee-reads[]  regs[]  stack(first arg first)[eax, esi]  callee-ret-imm=0
    b7cf6f: caller cleans 16 bytes
    b7cf77: if (ebx <= 0 (signed)) goto B145  else B28   ; jle 0xb7d8c2

  B28 @0xb7cf7d:
    b7cf7d: CALL 0x4a7c60 sub_4a7c60  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7cf86: if (ebx <= 0 (signed)) goto B43  else B29   ; jle 0xb7d010

  B29 @0xb7cf8c:
    b7cf8c: STACK-OBJECT &[esp+0x0] -> esp

  B30 @0xb7cf90:
    b7cf98: if (esi != 0) goto B32  else B31   ; jne 0xb7cf9e

  B31 @0xb7cf9a:
    b7cf9c: goto B33 @0xb7cfad

  B32 @0xb7cf9e:
    b7cfa5: CALL 0xbde980 vec3_distance  callee-reads[]  regs[]  stack(first arg first)[ecx, eax]  callee-ret-imm=0
    b7cfaa: caller cleans 8 bytes

  B33 @0xb7cfad:
    b7cfad: X87 STORE f32 [ebp - 0xc] <- ret(?)
    b7cfbf: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7cfc6: if (eax > 0 (signed)) goto B35  else B34   ; jg 0xb7cfd1

  B34 @0xb7cfc8:
    b7cfcf: goto B38 @0xb7cfde

  B35 @0xb7cfd1:
    b7cfd4: if (eax < (signed) 8) goto B37  else B36   ; jl 0xb7cfdb

  B36 @0xb7cfd6:

  B37 @0xb7cfdb:

  B38 @0xb7cfde:
    b7cfe6: X87 STORE f32 [ebp - 0x14] <- ((float)i32[ebp - 8] * f32[ebp - 0xc])

  B39 @0xb7cfec:
    b7cfee: CALL 0xb7d8e0 sub_b7d8e0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    b7cff5: if (eax == 0) goto B41  else B40   ; je 0xb7cfff

  B40 @0xb7cff7:
    b7cffa: X87 fadd st(1)  (st(i) dest, untracked)
    b7cffc: X87 STORE f32 [eax + 0x54] <- f32[eax + 0x54]

  B41 @0xb7cfff:
    b7d005: if (edx < (signed) 2) goto B39  else B42   ; jl 0xb7cfec

  B42 @0xb7d007:
    b7d00e: if (edi < (signed) ebx) goto B30  else B43   ; jl 0xb7cf90

  B43 @0xb7d010:
    b7d010: READ i32[0x319bf84]   ; mov esi, dword ptr [0x319bf84]
    b7d018: if (esi == 0) goto B50  else B44   ; je 0xb7d04e

  B44 @0xb7d01a:

  B45 @0xb7d020:
    b7d024: if ((byte ptr [esi + 0x11] & 1) != 0) goto B48  else B46   ; jne 0xb7d03f

  B46 @0xb7d026:
    b7d032: CALL 0xb7d8e0 sub_b7d8e0  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [esi])]  stack(first arg first)[]  callee-ret-imm=0
    b7d039: if (eax == 0) goto B48  else B47   ; je 0xb7d03f

  B47 @0xb7d03b:

  B48 @0xb7d03f:
    b7d042: READ i32[0x319bf84]   ; cmp esi, dword ptr [0x319bf84]
    b7d048: if (esi == dword ptr [0x319bf84]) goto B50  else B49   ; je 0xb7d04e

  B49 @0xb7d04a:
    b7d04c: if (esi != 0) goto B45  else B50   ; jne 0xb7d020

  B50 @0xb7d04e:
    b7d04e: CALL 0x4a7c90 sub_4a7c90  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=None

  B51 @0xb7d053:
    b7d05e: if (dword ptr [ebp - 0x30] <= (signed) ebx) goto B144  else B52   ; jle 0xb7d8c0

  B52 @0xb7d064:
    b7d064: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    b7d07a: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b7d080: if (ecx < (signed) 0x800) goto B54  else B53   ; jl 0xb7d088

  B53 @0xb7d082:
    b7d082: WRITE i32[0xec3abc] mov = ebx   ; dword ptr [0xec3abc], ebx

  B54 @0xb7d088:
    b7d08d: INT div ecx
    b7d0a8: CALL 0xb7d8e0 sub_b7d8e0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    b7d0b9: CALL 0xb7d8e0 sub_b7d8e0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    b7d0cb: if (edi == ebx) goto B58  else B55   ; je 0xb7d0eb

  B55 @0xb7d0cd:
    b7d0d0: X87 COMPARE f32[edi + 0x58] vs STst(1)   ; fcomp st(1)
    b7d0d6: if ((dl & ah) == 0) goto B57  else B56   ; je 0xb7d0e6

  B56 @0xb7d0d8:
    b7d0da: X87 COMPARE ret(?) vs f32[edi + 0x54]   ; fcom dword ptr [edi + 0x54]
    b7d0e4: if (x87 compare ('ret(?)', 'f32[edi + 0x54]'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B58  else B57   ; jne 0xb7d0eb

  B57 @0xb7d0e6:

  B58 @0xb7d0eb:
    b7d0ed: if (ecx == ebx) goto B63  else B59   ; je 0xb7d115

  B59 @0xb7d0ef:
    b7d0f2: X87 COMPARE f32[ecx + 0x58] vs ST(1)   ; fcompp 
    b7d0f9: if (x87 compare ('f32[ecx + 0x58]', 'ST(1)'): ST0 outcome in {gt, eq}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B62  else B60   ; je 0xb7d10c

  B60 @0xb7d0fb:
    b7d0fb: X87 COMPARE ret(?) vs f32[ecx + 0x54]   ; fcomp dword ptr [ecx + 0x54]
    b7d103: if (x87 compare ('ret(?)', 'f32[ecx + 0x54]'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B64  else B61   ; jne 0xb7d119

  B61 @0xb7d105:
    b7d10a: goto B64 @0xb7d119

  B62 @0xb7d10c:
    b7d113: goto B64 @0xb7d119

  B63 @0xb7d115:

  B64 @0xb7d119:
    b7d11b: if (edi != ebx) goto B67  else B65   ; jne 0xb7d128

  B65 @0xb7d11d:
    b7d11f: if (ecx == ebx) goto B142  else B66   ; je 0xb7d8b4

  B66 @0xb7d125:

  B67 @0xb7d128:
    b7d137: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d13e: if (eax > 0 (signed)) goto B69  else B68   ; jg 0xb7d147

  B68 @0xb7d140:
    b7d145: goto B73 @0xb7d15e

  B69 @0xb7d147:
    b7d14a: if (eax < (signed) 8) goto B71  else B70   ; jl 0xb7d151

  B70 @0xb7d14c:

  B71 @0xb7d151:
    b7d155: if (eax <= 0 (signed)) goto B142  else B72   ; jle 0xb7d8b4

  B72 @0xb7d15b:

  B73 @0xb7d15e:
    b7d168: CALL 0xbde980 vec3_distance  callee-reads[]  regs[]  stack(first arg first)[esi, eax]  callee-ret-imm=0
    b7d16d: X87 STORE f32 [ebp - 0x14] <- ret(vec3_distance)
    b7d173: caller cleans 8 bytes
    b7d17c: X87 fmul st(1)  (st(i) dest, untracked)
    b7d17e: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d189: if (cx <= 0 (signed)) goto B141  else B74   ; jle 0xb7d8b2

  B74 @0xb7d18f:
    b7d194: INT cdq 
    b7d198: INT idiv esi
    b7d1a2: if (ecx < (signed) eax) goto B76  else B75   ; jl 0xb7d1a7

  B75 @0xb7d1a4:

  B76 @0xb7d1a7:
    b7d1b0: X87 fidiv dword ptr [ebp - 0x14]  (untracked)
    b7d1c3: X87 STORE f32 [ebp - 0x18] <- ret(?)
    b7d1c6: if (di <= (signed) si) goto B89  else B77   ; jle 0xb7d260

  B77 @0xb7d1cc:

  B78 @0xb7d1d2:
    b7d1d6: if (ebx <= 0 (signed)) goto B88  else B79   ; jle 0xb7d249

  B79 @0xb7d1d8:

  B80 @0xb7d1da:
    b7d1f1: if (dword ptr [ebp - 0x44] != 0) goto B82  else B81   ; jne 0xb7d1ff

  B81 @0xb7d1f3:
    b7d1fd: goto B83 @0xb7d207

  B82 @0xb7d1ff:

  B83 @0xb7d207:
    b7d21a: if (dword ptr [ebp - 0x40] != 0) goto B85  else B84   ; jne 0xb7d228

  B84 @0xb7d21c:
    b7d226: goto B86 @0xb7d234

  B85 @0xb7d228:

  B86 @0xb7d234:
    b7d244: if (eax < (signed) ebx) goto B80  else B87   ; jl 0xb7d1da

  B87 @0xb7d246:

  B88 @0xb7d249:
    b7d25a: if (ax < (signed) di) goto B78  else B89   ; jl 0xb7d1d2

  B89 @0xb7d260:
    b7d26d: X87 STORE f32 [esp] <- f32[ebp - 0x4c]
    b7d276: STACK-OBJECT &[ebp-0x70] -> ecx
    b7d27c: CALL 0xbe7a10 sub_be7a10  callee-reads[edi]  regs[edi<-(mov edi, dword ptr [ebp - 0x1c])]  stack(first arg first)[ecx, eax, 0x25f5b14, ecx]  callee-ret-imm=None
    b7d281: caller cleans 16 bytes
    b7d287: if (result == 0 of `sub eax, 1`) goto B94  else B90   ; je 0xb7d364

  B90 @0xb7d28d:
    b7d290: if (result != 0 of `sub eax, 1`) goto B99  else B91   ; jne 0xb7d441

  B91 @0xb7d296:
    b7d29e: X87 STORE f32 [ebp - 0x58] <- (f32[ebp - 0x70] - f32[esi])
    b7d2a7: X87 STORE f32 [ebp - 0x54] <- (f32[ebp - 0x6c] - f32[esi + 4])
    b7d2b0: X87 STORE f32 [ebp - 0x50] <- (f32[ebp - 0x68] - f32[esi + 8])
    b7d2cc: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0x58] + ((((f32[ebp - 0x50] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x54]) * (((f32[ebp - 0x50] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x54])))
    b7d2d2: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d2d5: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d2dd: caller cleans 4 bytes
    b7d2e0: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d2ee: X87 STORE f32 [ebp - 0xa4] <- (f32[ebp - 0x70] - f32[esi])
    b7d2fa: X87 STORE f32 [ebp - 0xa0] <- (f32[ebp - 0x6c] - f32[esi + 4])
    b7d306: X87 STORE f32 [ebp - 0x9c] <- (f32[ebp - 0x68] - f32[esi + 8])
    b7d32e: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0xa4] + ((((f32[ebp - 0x9c] * ST(st(1))) * ST(st(2))) + f32[ebp - 0xa0]) * (((f32[ebp - 0x9c] * ST(st(1))) * ST(st(2))) + f32[ebp - 0xa0])))
    b7d334: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d337: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d33f: caller cleans 4 bytes
    b7d342: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d34e: STACK-OBJECT &[ebp-0x170] -> edx
    b7d354: if (di > (signed) cx) goto B93  else B92   ; jg 0xb7d35c

  B92 @0xb7d356:
    b7d357: goto B98 @0xb7d437

  B93 @0xb7d35c:
    b7d35f: goto B98 @0xb7d437

  B94 @0xb7d364:
    b7d36d: X87 STORE f32 [ebp - 0x8c] <- (f32[ebp - 0x70] - f32[esi])
    b7d379: X87 STORE f32 [ebp - 0x88] <- (f32[ebp - 0x6c] - f32[esi + 4])
    b7d385: X87 STORE f32 [ebp - 0x84] <- (f32[ebp - 0x68] - f32[esi + 8])
    b7d3ad: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0x8c] + ((((f32[ebp - 0x84] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x88]) * (((f32[ebp - 0x84] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x88])))
    b7d3b3: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d3b6: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d3be: caller cleans 4 bytes
    b7d3c1: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d3d2: X87 STORE f32 [ebp - 0xc] <- (f32[esi] - f32[0x25f5b14])
    b7d3da: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0xc] * f32[ebp - 0xc])
    b7d3e6: X87 STORE f32 [ebp - 0xc] <- (f32[esi + 4] - f32[0x25f5b18])
    b7d3f1: X87 STORE f32 [ebp - 8] <- ((f32[ebp - 0xc] * f32[ebp - 0xc]) + f32[ebp - 8])
    b7d3fd: X87 STORE f32 [ebp - 0xc] <- (f32[esi + 8] - f32[0x25f5b1c])
    b7d411: X87 STORE f32 [ebp - 8] <- f32[ebp - 0xc]
    b7d41b: X87 COMPARE (f32[ebp - 8] * ST(st(1))) vs ST(1)   ; fcompp 
    b7d422: if (x87 compare ('(f32[ebp - 8] * ST(st(1)))', 'ST(1)'): ST0 outcome in {gt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B96  else B95   ; jp 0xb7d42a

  B95 @0xb7d424:
    b7d428: goto B97 @0xb7d431

  B96 @0xb7d42a:

  B97 @0xb7d431:
    b7d431: STACK-OBJECT &[ebp-0x170] -> edx

  B98 @0xb7d437:
    b7d437: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    b7d43e: caller cleans 8 bytes

  B99 @0xb7d441:
    b7d44e: X87 STORE f32 [esp] <- f32[ebp - 0x2c]
    b7d457: STACK-OBJECT &[ebp-0x70] -> eax
    b7d45b: CALL 0xbe7a10 sub_be7a10  callee-reads[edi]  regs[edi<-(mov edi, dword ptr [ebp - 0x1c])]  stack(first arg first)[eax, eax, 0x25f5b14, ecx]  callee-ret-imm=None
    b7d460: caller cleans 16 bytes
    b7d466: if (result == 0 of `sub eax, 1`) goto B104  else B100   ; je 0xb7d589

  B100 @0xb7d46c:
    b7d46f: if (result != 0 of `sub eax, 1`) goto B109  else B101   ; jne 0xb7d654

  B101 @0xb7d475:
    b7d47d: X87 STORE f32 [ebp - 0x98] <- (f32[ebp - 0x70] - f32[eax])
    b7d489: X87 STORE f32 [ebp - 0x94] <- (f32[ebp - 0x6c] - f32[eax + 4])
    b7d495: X87 STORE f32 [ebp - 0x90] <- (f32[ebp - 0x68] - f32[eax + 8])
    b7d4bd: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0x98] + ((((f32[ebp - 0x90] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x94]) * (((f32[ebp - 0x90] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x94])))
    b7d4c3: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d4c6: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d4ce: caller cleans 4 bytes
    b7d4d1: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d4e2: X87 STORE f32 [ebp - 0x7c] <- (f32[ebp - 0x70] - f32[eax])
    b7d4eb: X87 STORE f32 [ebp - 0x78] <- (f32[ebp - 0x6c] - f32[eax + 4])
    b7d4f4: X87 STORE f32 [ebp - 0x74] <- (f32[ebp - 0x68] - f32[eax + 8])
    b7d510: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0x7c] + ((((f32[ebp - 0x74] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x78]) * (((f32[ebp - 0x74] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x78])))
    b7d516: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d519: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d521: caller cleans 4 bytes
    b7d524: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d533: STACK-OBJECT &[ebp-0x170] -> edx
    b7d53b: if (di > (signed) ax) goto B103  else B102   ; jg 0xb7d564

  B102 @0xb7d53d:
    b7d53f: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(mov ecx, edi)]  stack(first arg first)[]  callee-ret-imm=0
    b7d54f: STACK-OBJECT &[ebp-0x170] -> edx
    b7d557: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(add ecx, -1)]  stack(first arg first)[edx, ebx]  callee-ret-imm=0
    b7d55c: caller cleans 16 bytes
    b7d55f: goto B108 @0xb7d652

  B103 @0xb7d564:
    b7d567: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(mov ecx, dword ptr [ebp - 8])]  stack(first arg first)[]  callee-ret-imm=0
    b7d574: STACK-OBJECT &[ebp-0x170] -> edx
    b7d57c: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(add ecx, -1)]  stack(first arg first)[edi, ebx]  callee-ret-imm=0
    b7d581: caller cleans 16 bytes
    b7d584: goto B108 @0xb7d652

  B104 @0xb7d589:
    b7d592: X87 STORE f32 [ebp - 0x3c] <- (f32[ebp - 0x70] - f32[edi])
    b7d59b: X87 STORE f32 [ebp - 0x38] <- (f32[ebp - 0x6c] - f32[edi + 4])
    b7d5a4: X87 STORE f32 [ebp - 0x34] <- (f32[ebp - 0x68] - f32[edi + 8])
    b7d5c0: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0x3c] + ((((f32[ebp - 0x34] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x38]) * (((f32[ebp - 0x34] * ST(st(1))) * ST(st(2))) + f32[ebp - 0x38])))
    b7d5c6: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d5c9: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d5d1: caller cleans 4 bytes
    b7d5d4: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d5e5: X87 STORE f32 [ebp - 0xc] <- (f32[edi] - f32[0x25f5b14])
    b7d5ed: X87 STORE f32 [ebp - 8] <- (f32[ebp - 0xc] * f32[ebp - 0xc])
    b7d5f9: X87 STORE f32 [ebp - 0xc] <- (f32[edi + 4] - f32[0x25f5b18])
    b7d604: X87 STORE f32 [ebp - 8] <- ((f32[ebp - 0xc] * f32[ebp - 0xc]) + f32[ebp - 8])
    b7d610: X87 STORE f32 [ebp - 0xc] <- (f32[edi + 8] - f32[0x25f5b1c])
    b7d624: X87 STORE f32 [ebp - 8] <- f32[ebp - 0xc]
    b7d62e: X87 COMPARE (f32[ebp - 8] * ST(st(1))) vs ST(1)   ; fcompp 
    b7d635: if (x87 compare ('(f32[ebp - 8] * ST(st(1)))', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B106  else B105   ; jne 0xb7d63d

  B105 @0xb7d637:
    b7d63b: goto B107 @0xb7d644

  B106 @0xb7d63d:

  B107 @0xb7d644:
    b7d644: STACK-OBJECT &[ebp-0x170] -> edx
    b7d64a: CALL 0xb7cca0 sub_b7cca0  callee-reads[ecx]  regs[ecx<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    b7d64f: caller cleans 8 bytes

  B108 @0xb7d652:

  B109 @0xb7d654:
    b7d659: if (esi >= (signed) edi) goto B142  else B110   ; jge 0xb7d8b4

  B110 @0xb7d65f:
    b7d65f: READ i32[0x319bf84]   ; mov esi, dword ptr [0x319bf84]
    b7d667: if (esi == 0) goto B122  else B111   ; je 0xb7d705

  B111 @0xb7d66d:

  B112 @0xb7d670:
    b7d677: if ((byte ptr [esi + 0x11] & 4) != 0) goto B118  else B113   ; jne 0xb7d6d2

  B113 @0xb7d679:
    b7d67b: if (dword ptr [esi] != eax) goto B120  else B114   ; jne 0xb7d6f2

  B114 @0xb7d67d:
    b7d684: X87 STORE f32 [ebp - 0x10] <- (f32[esi + 8] / f32[ebp - 0x18])
    b7d68a: X87 STORE f32 [esp] <- f32[ebp - 0x10]
    b7d68d: CALL 0x427600 floorf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b7d692: caller cleans 4 bytes
    b7d695: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d69e: if ((byte ptr [esi + 0x10] & 0x40) != 0) goto B116  else B115   ; jne 0xb7d6aa

  B115 @0xb7d6a0:

  B116 @0xb7d6aa:
    b7d6c4: if (eax >= (signed) 0x32) goto B120  else B117   ; jge 0xb7d6f2

  B117 @0xb7d6c6:
    b7d6d0: goto B120 @0xb7d6f2

  B118 @0xb7d6d2:
    b7d6d5: if (dword ptr [esi + 4] != eax) goto B120  else B119   ; jne 0xb7d6f2

  B119 @0xb7d6d7:

  B120 @0xb7d6f2:
    b7d6f5: READ i32[0x319bf84]   ; cmp esi, dword ptr [0x319bf84]
    b7d6fb: if (esi == dword ptr [0x319bf84]) goto B122  else B121   ; je 0xb7d705

  B121 @0xb7d6fd:
    b7d6ff: if (esi != 0) goto B112  else B122   ; jne 0xb7d670

  B122 @0xb7d705:
    b7d709: if (edi <= 0 (signed)) goto B128  else B123   ; jle 0xb7d737

  B123 @0xb7d70b:

  B124 @0xb7d710:
    b7d718: if (word ptr [ebp + eax*4 - 0x16e] != cx) goto B126  else B125   ; jne 0xb7d72e

  B125 @0xb7d71a:

  B126 @0xb7d72e:
    b7d733: if (eax < (signed) edi) goto B124  else B127   ; jl 0xb7d710

  B127 @0xb7d735:

  B128 @0xb7d737:
    b7d737: if (je (flags from an earlier instruction)) goto B142  else B129   ; je 0xb7d8b4

  B129 @0xb7d73d:
    b7d73d: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    b7d753: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b7d759: if (ecx < (signed) 0x800) goto B131  else B130   ; jl 0xb7d763

  B130 @0xb7d75b:
    b7d75d: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B131 @0xb7d763:
    b7d765: INT div edi
    b7d788: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b7d797: X87 STORE f32 [ebp - 8] <- (((float)i32[ebp - 0x10] + f64(0.5; bits 0x3FE0000000000000)) * f32[ebp - 0x18])
    b7d79a: if (ecx < (signed) 0x800) goto B133  else B132   ; jl 0xb7d7a4

  B132 @0xb7d79c:
    b7d79e: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B133 @0xb7d7a4:
    b7d7a4: READ i32[0x22dbc08]   ; mov edx, dword ptr [0x22dbc08]
    b7d7ae: INT div esi
    b7d7c0: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b7d7c9: if (ecx < (signed) 0x800) goto B135  else B134   ; jl 0xb7d7d3

  B134 @0xb7d7cb:
    b7d7cd: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B135 @0xb7d7d3:
    b7d7d8: INT mul esi
    b7d7f5: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b7d7fb: if (ecx < (signed) 0x800) goto B137  else B136   ; jl 0xb7d807

  B136 @0xb7d7fd:
    b7d7fd: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B137 @0xb7d807:
    b7d80c: CALL 0xb7db50 sub_b7db50  callee-reads[edi]  regs[edi<-(mov edi, 0x319bf78)]  stack(first arg first)[]  callee-ret-imm=0
    b7d815: if (edi == 0) goto B145  else B138   ; je 0xb7d8c2

  B138 @0xb7d81b:
    b7d846: X87 STORE f32 [esp] <- f32[ebp - 8]
    b7d860: CALL 0xb7b170 sub_b7b170  callee-reads[eax, edx]  regs[eax<-(setne al), edx<-(mov edx, dword ptr [ebp - 0x20])]  stack(first arg first)[edi, ecx, edx, edx, ecx, edx]  callee-ret-imm=24
    b7d874: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b7d87e: X87 STORE f32 [ebp - 0x10] <- ret(sub_68a500)
    b7d884: if (eax >= 0 (signed)) goto B140  else B139   ; jge 0xb7d88c

  B139 @0xb7d886:

  B140 @0xb7d88c:
    b7d8a2: X87 STORE f32 [ebp - 0x10] <- (f32[ebp - 0x10] + (((1.0 - ST(st(0))) * f32[edx + 0x28]) * f32[edx + 0x24]))
    b7d8ad: X87 STORE f32 [edx + 0x58] <- (ret(?) / (f32[ebp - 0x10] * f32[edx + 0x54]))
    b7d8b0: goto B142 @0xb7d8b4

  B141 @0xb7d8b2:

  B142 @0xb7d8b4:
    b7d8b8: if (dword ptr [ebp - 0x24] > (signed) 0) goto B51  else B143   ; jg 0xb7d053

  B143 @0xb7d8be:
    b7d8be: goto B145 @0xb7d8c2

  B144 @0xb7d8c0:

  B145 @0xb7d8c2:
    b7d8c2: STACK-OBJECT &[ebp-0x180] -> esp
    b7d8d0: CALL 0xc9604f __security_check_cookie  callee-reads[ecx]  regs[ecx<-(xor ecx, ebp)]  stack(first arg first)[]  callee-ret-imm=None
    b7d8d8: RETURN

  --- summary ---
  absolute globals read: 0xe80b50, 0xe843ac, 0xec3abc, 0x21703d4, 0x22dbc08, 0x22fd858, 0x22fd864, 0x25295ff, 0x2529e20, 0x319bf84
  absolute globals written: 0xebd0bc, 0xec3abc, 0x25295ff
  direct callees: 0xb7c250 ?, 0xbde880 ?, 0xbdd230 timer_elapsed, 0xb7b2d0 ?, 0xb7dc00 ?, 0xb7dd70 allocator<>::num_used_88, 0xcb35c0 __alloca_probe_16, 0xa72800 ?, 0x4a7c60 ?, 0xbde980 vec3_distance, 0xda0140 _ftol2_sse, 0xb7d8e0 ?, 0x4a7c90 ?, 0xbe7a10 ?, 0x40ea20 sqrtf_wrapper, 0xb7cca0 ?, 0x427600 floorf_wrapper, 0xb7db50 ?, 0xb7b170 ?, 0x68a500 ?, 0xc9604f __security_check_cookie
```

### distant_vehicles_spawn_update - 0xB80130

**Role.** Distant vehicles: string 'distant vehicles temporary mempool' creates a temporary pool, collects vehicle candidates, `_qsort`s them (comparator is a function pointer on the stack, see the `push` before the call), places them along road splines (`spline_param_at_distance`, `spline_eval_position`) and unregisters time-of-day objects (`tod_object_unregister`).

**Convention.** Entry registers read (linear scan): none. True size 0x9DD, 698 instructions, 105 blocks, 0 jump table(s). Callers: 0x93D240.

**Sections, in order of the calls (address and callee):** 0xB80175 sub_B7EFB0; 0xB801FB timer_elapsed; 0xB80283 sub_BDE880; 0xB8030E sub_B7E770; 0xB8031C sub_A8B8F0; 0xB8032C tod_object_unregister; 0xB80339 sub_B81840; 0xB80347 allocator<>::num_used_89; 0xB803B3 sub_C15470; 0xB80400 sub_B7F020; 0xB80416 sub_4A7A50; 0xB80426 sub_B800C0; 0xB80486 sub_4A7A80; 0xB804D8 sub_B800C0; 0xB80594 sub_68A500; 0xB805D3 sub_4912D0; 0xB805DB _ftol2_sse; 0xB8068A floorf_wrapper; 0xB80692 _ftol2_sse; 0xB80754 spline_param_at_distance; 0xB80765 spline_eval_position; 0xB80808 sqrtf_wrapper; 0xB8088C _qsort; 0xB809C7 sub_B81790; 0xB80A3D sub_B7FDA0; 0xB80A95 sub_B7E580; 0xB80AA1 sub_68A500; 0xB80AEF sub_C15720; 0xB80B01 __security_check_cookie.

**Hazards.** `_qsort` comparator address must be kept; spline math is x87 with f32 stores; stack cookie; 26 callees; the temporary pool is freed at the end (`0xB819B0 allocator<>::num_used_89` bracket).

**Globals read:** 0xE80B50, 0xE843AC, 0xE86FA0, 0xEC3ABC, 0x21703D4, 0x2225B64, 0x22DBC10, 0x22DBC1C, 0x22DBC20, 0x22FD858, 0x22FD864, 0x25295FF, 0x252A16B, 0x252A190, 0x252A198, 0x319F60C.

**Globals written:** 0xEBD0C0, 0xEC3ABC, 0x25295FF.

```
FUNCTION 0x00B80130  true size 0x9DD (2525 bytes)  insns 698  blocks 105
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x84

  B0 @0xb80130:
    b80136: READ i32[0xe80b50]   ; mov eax, dword ptr [0xe80b50]
    b80144: READ u8[0x252a16b]   ; cmp byte ptr [0x252a16b], 0
    b8014f: if (byte ptr [0x252a16b] == 0) goto B104  else B1   ; je 0xb80af4

  B1 @0xb80155:
    b80157: goto B2 @0xb80160

  B2 @0xb80160:
    b80167: if (byte ptr [eax + 0x215d9fc] == 0) goto B104  else B3   ; je 0xb80af4

  B3 @0xb8016d:
    b80173: if (eax < (unsigned) 6) goto B2  else B4   ; jb 0xb80160

  B4 @0xb80175:
    b80175: CALL 0xb7efb0 sub_b7efb0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b8017a: READ i32[0x21703d4]   ; cmp dword ptr [0x21703d4], 0
    b80181: if (dword ptr [0x21703d4] == 0) goto B6  else B5   ; je 0xb8018b

  B5 @0xb80183:
    b80189: goto B7 @0xb80191

  B6 @0xb8018b:

  B7 @0xb80191:
    b80191: READ u8[0x25295ff]   ; cmp byte ptr [0x25295ff], 0
    b80198: X87 STORE f32 [esp + 0x30] <- ret(?)
    b801b4: X87 STORE f32 [esp + 0x30] <- f32[0xe86fac]
    b801be: X87 fmul st(1)  (st(i) dest, untracked)
    b801c0: X87 STORE f32 [esp + 0x58] <- f64(150.0; bits 0x4062C00000000000)
    b801ce: X87 STORE f32 [0x252a178] <- (f32[0xe86fb0] + f32[esp + 0x30])
    b801ea: X87 STORE f32 [0x252a174] <- ((f32[0x252a040] * f64(0.5005000233650208; bits 0x3FE00418A0000000)) + (((f32[esp + 0x30] * ST(st(0))) * f32[0xe86f94]) * f64(140.0; bits 0x4061800000000000)))
    b801f0: if (byte ptr [0x25295ff] != 0) goto B104  else B8   ; jne 0xb80af4

  B8 @0xb801f6:
    b801fb: CALL 0xbdd230 timer_elapsed  callee-reads[eax]  regs[eax<-(mov eax, 0xebd0c0)]  stack(first arg first)[]  callee-ret-imm=0
    b80202: if (al == 0) goto B104  else B9   ; je 0xb80af4

  B9 @0xb80208:
    b80208: READ i32[0xe843ac]   ; mov eax, dword ptr [0xe843ac]
    b80217: WRITE i32[0xebd0c0] mov = eax   ; dword ptr [0xebd0c0], eax
    b8021c: if (eax <= (signed) 0x6b49d200) goto B11  else B10   ; jle 0xb80228

  B10 @0xb8021e:
    b80223: WRITE i32[0xebd0c0] mov = eax   ; dword ptr [0xebd0c0], eax

  B11 @0xb80228:
    b8022e: STACK-OBJECT &[esp+0x4c] -> eax
    b80239: STACK-OBJECT &[esp+0x60] -> eax
    b8023d: STACK-OBJECT &[esp+0x40] -> ecx
    b80241: X87 STORE f32 [esp + 0x40] <- (f32[0x25f5b14] - f32[0x25f5b2c])
    b80251: X87 STORE f32 [esp + 0x44] <- (f32[0x25f5b18] - f32[0x25f5b30])
    b80261: X87 STORE f32 [esp + 0x48] <- (f32[0x25f5b1c] - f32[0x25f5b34])
    b8026b: X87 STORE f32 [esp + 0x60] <- f32[0x25f5b50]
    b80275: X87 STORE f32 [esp + 0x64] <- f32[0x25f5b54]
    b8027f: X87 STORE f32 [esp + 0x68] <- f32[0x25f5b58]
    b80283: CALL 0xbde880 sub_bde880  callee-reads[eax, ecx]  regs[eax<-(lea eax, [esp + 0x60]), ecx<-(lea ecx, [esp + 0x40])]  stack(first arg first)[eax]  callee-ret-imm=None
    b8028a: READ i32[0x319f60c]   ; mov esi, dword ptr [0x319f60c]
    b80296: caller cleans 4 bytes
    b8029f: X87 STORE f32 [esp + 0x18] <- (f32[0xe86fb4] / f32[0xe8437c])
    b802b1: X87 STORE f32 [esp + 0x3c] <- f32[esp + 0x4c]
    b802b9: X87 fmul st(1)  (st(i) dest, untracked)
    b802bb: X87 STORE f32 [esp + 0x40] <- f32[esp + 0x50]
    b802c3: X87 STORE f32 [esp + 0x44] <- ((f32[esp + 0x18] * ST(st(0))) * f32[esp + 0x54])
    b802d1: X87 STORE f32 [esp + 0x4c] <- (f32[esp + 0x3c] + f32[0x25f5b14])
    b802df: X87 STORE f32 [esp + 0x50] <- (f32[esp + 0x40] + f32[0x25f5b18])
    b802ed: X87 STORE f32 [esp + 0x54] <- (f32[esp + 0x44] + f32[0x25f5b1c])

  B12 @0xb802f1:
    b802f3: if (esi == 0) goto B16  else B13   ; je 0xb80342

  B13 @0xb802f5:
    b802fa: READ i32[0x319f60c]   ; sub ebp, dword ptr [0x319f60c]
    b80300: STACK-OBJECT &[esp+0x5c] -> ecx
    b8030a: STACK-OBJECT &[esp+0x4c] -> eax
    b8030e: CALL 0xb7e770 sub_b7e770  callee-reads[esi]  regs[esi<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    b80315: if (al == 0) goto B15  else B14   ; je 0xb8033e

  B14 @0xb80317:
    b8031c: CALL 0xa8b8f0 sub_a8b8f0  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [esi + 0x18])]  stack(first arg first)[]  callee-ret-imm=0
    b8032c: CALL 0x4d9620 tod_object_unregister  callee-reads[edx]  regs[edx<-(mov edx, 5)]  stack(first arg first)[esi, 0xffff]  callee-ret-imm=0
    b80331: caller cleans 8 bytes
    b80339: CALL 0xb81840 sub_b81840  callee-reads[edi]  regs[edi<-(mov edi, 0x319f600)]  stack(first arg first)[]  callee-ret-imm=None

  B15 @0xb8033e:
    b80340: goto B12 @0xb802f1

  B16 @0xb80342:
    b80347: CALL 0xb819b0 allocator<>::num_used_89  callee-reads[ecx]  regs[ecx<-(mov ecx, 0x319f600)]  stack(first arg first)[]  callee-ret-imm=0
    b8034c: READ i32[0xe86fa0]   ; mov ecx, dword ptr [0xe86fa0]
    b80358: if (result >= 0 (sign clear) of `sub ecx, eax`) goto B18  else B17   ; jns 0xb8035e

  B17 @0xb8035a:
    b8035c: goto B20 @0xb80368

  B18 @0xb8035e:
    b80361: if (ecx <= (signed) 0x14) goto B21  else B19   ; jle 0xb8036c

  B19 @0xb80363:

  B20 @0xb80368:

  B21 @0xb8036c:
    b8036c: READ i32[0x22fd858]   ; mov eax, dword ptr [0x22fd858]
    b80373: if (eax == 0) goto B23  else B22   ; je 0xb8037a

  B22 @0xb80375:
    b80378: if (eax != 1) goto B24  else B23   ; jne 0xb80380

  B23 @0xb8037a:

  B24 @0xb80380:
    b80383: if (eax != 3) goto B26  else B25   ; jne 0xb80392

  B25 @0xb80385:
    b80385: READ i32[0x22fd864]   ; cmp dword ptr [0x22fd864], 2
    b8038c: if (dword ptr [0x22fd864] < (signed) 2) goto B104  else B26   ; jl 0xb80af4

  B26 @0xb80392:
    b80394: if (ecx <= 0 (signed)) goto B104  else B27   ; jle 0xb80af4

  B27 @0xb8039a:
    b8039a: READ i32[0x2225b64]   ; mov ecx, dword ptr [0x2225b64]
    b803ac: WRITE u8[0x25295ff] mov = 0x1   ; byte ptr [0x25295ff], 1
    b803b3: CALL 0xc15470 sub_c15470  callee-reads[]  regs[]  stack(first arg first)[0xe4b418, edx]  callee-ret-imm=None
    b803b8: READ i32[0x2225b64]   ; mov ecx, dword ptr [0x2225b64]
    b803d2: CALL indirect edx  regs[ecx<-(mov ecx, eax) eax<-(?)]  stack(first arg first)[ecx, 0]
    b803d8: READ i32[0x2225b64]   ; mov edx, dword ptr [0x2225b64]
    b803e1: X87 STORE f32 [esp + 4] <- f32[esp + 0x30]
    b803eb: STACK-OBJECT &[esp+0x64] -> eax
    b803ef: X87 STORE f32 [esp] <- f32[esp + 0x60]
    b803f3: STACK-OBJECT &[esp+0x58] -> ecx
    b80400: CALL 0xb7f020 sub_b7f020  callee-reads[]  regs[]  stack(first arg first)[ecx, eax]  callee-ret-imm=0
    b80407: caller cleans 16 bytes
    b80410: if (ebp <= 0 (signed)) goto B103  else B28   ; jle 0xb80aeb

  B28 @0xb80416:
    b80416: CALL 0x4a7a50 sub_4a7a50  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b8041f: if (ebp <= 0 (signed)) goto B38  else B29   ; jle 0xb80486

  B29 @0xb80421:
    b80426: CALL 0xb800c0 sub_b800c0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    b8042d: if (eax == 0) goto B37  else B30   ; je 0xb8047f

  B30 @0xb8042f:
    b80435: X87 STORE f32 [eax + 0x48] <- (f32[esi + 0x14] + f32[eax + 0x48])
    b80441: if (cx == 0xffff) goto B37  else B31   ; je 0xb8047f

  B31 @0xb80443:
    b80452: if (ecx == 0) goto B37  else B32   ; je 0xb8047f

  B32 @0xb80454:
    b80459: if (edx == 0) goto B35  else B33   ; je 0xb80465

  B33 @0xb8045b:
    b8045f: if ((byte ptr [edx + 0x33] & 1) != 0) goto B35  else B34   ; jne 0xb80465

  B34 @0xb80461:

  B35 @0xb80465:
    b8046c: if (cx == word ptr [esi + 0x2a]) goto B37  else B36   ; je 0xb8047f

  B36 @0xb8046e:
    b8047d: if (ecx != 0) goto B32  else B37   ; jne 0xb80454

  B37 @0xb8047f:
    b80484: if (ebx < (signed) ebp) goto B29  else B38   ; jl 0xb80421

  B38 @0xb80486:
    b80486: CALL 0x4a7a80 sub_4a7a80  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=None
    b8048b: goto B40 @0xb80494

  B39 @0xb80490:

  B40 @0xb80494:
    b80499: if (dword ptr [esp + 0x34] <= (signed) 0) goto B103  else B41   ; jle 0xb80aeb

  B41 @0xb8049f:
    b8049f: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    b804b5: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b804bb: if (ecx < (signed) 0x800) goto B43  else B42   ; jl 0xb804c7

  B42 @0xb804bd:
    b804bd: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B43 @0xb804c7:
    b804c9: INT div dword ptr [esp + 0x48]
    b804d8: CALL 0xb800c0 sub_b800c0  callee-reads[eax]  regs[eax<-(mov eax, ebx)]  stack(first arg first)[]  callee-ret-imm=0
    b804f2: READ i32[0x252a198]   ; cmp dword ptr [0x252a198], eax
    b804f8: if (dword ptr [0x252a198] <= (signed) eax) goto B47  else B44   ; jle 0xb8051e

  B44 @0xb804fa:
    b804fa: READ i32[0x22dbc10]   ; mov ecx, dword ptr [0x22dbc10]
    b80503: READ i32[0x252a198]   ; mov edx, dword ptr [0x252a198]
    b8050c: STACK-OBJECT &[esp+0x0] -> esp

  B45 @0xb80510:
    b80512: if ((dword ptr [ecx] & ebx) != 0) goto B48  else B46   ; jne 0xb80521

  B46 @0xb80514:
    b8051c: if (eax < (signed) edx) goto B45  else B47   ; jl 0xb80510

  B47 @0xb8051e:

  B48 @0xb80521:
    b80521: READ i32[0x22dbc10]   ; mov ecx, dword ptr [0x22dbc10]
    b8053f: X87 COMPARE f32[ebx + ecx + 8] vs ST(1)   ; fcompp 
    b80546: if (x87 compare ('f32[ebx + ecx + 8]', 'ST(1)'): ST0 outcome in {lt, eq}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B102  else B49   ; jnp 0xb80ae0

  B49 @0xb8054c:
    b8055b: X87 COMPARE (f32[ebx + eax + 0x10] * f64(10.0; bits 0x4024000000000000)) vs ST(1)   ; fcompp 
    b80562: if (x87 compare ('(f32[ebx + eax + 0x10] * f64(10.0; bits 0x4024000000000000))', 'ST(1)'): ST0 outcome in {gt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B102  else B50   ; je 0xb80ae0

  B50 @0xb80568:
    b8057a: X87 STORE f32 [esp + 0x20] <- (f32[eax + 0x14] - (f32[ebx + ecx + 0xc] + f32[ebx + ecx + 0xc]))
    b80580: X87 COMPARE 0.0 vs f32[esp + 0x20]   ; fcomp dword ptr [esp + 0x20]
    b80589: if (x87 compare ('0.0', 'f32[esp + 0x20]'): ST0 outcome in {gt, eq}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B102  else B51   ; je 0xb80ae0

  B51 @0xb8058f:
    b80594: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b80599: X87 STORE f32 [esp + 0x18] <- ret(sub_68a500)
    b805b2: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x18] + (((1.0 - ST(st(0))) * f32[ebp + 0x20]) * f32[ebp + 0x1c]))
    b805bc: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x1c] * f32[edx])
    b805c8: X87 STORE f32 [esp + 0x28] <- (f32[esp + 0x1c] * f32[esp + 0x24])
    b805d0: X87 STORE f32 [esp] <- f32[esp + 0x28]
    b805d3: CALL 0x4912d0 sub_4912d0  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b805d8: caller cleans 4 bytes
    b805db: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b805e3: if (eax >= (signed) 5) goto B53  else B52   ; jge 0xb805ed

  B52 @0xb805e5:
    b805eb: goto B54 @0xb805f6

  B53 @0xb805ed:

  B54 @0xb805f6:
    b805fe: X87 fidiv dword ptr [esp + 0x2c]  (untracked)
    b80602: X87 STORE f32 [esp + 0x24] <- f32[esp + 0x20]
    b80606: if (edx <= 0 (signed)) goto B57  else B55   ; jle 0xb80620

  B55 @0xb80608:

  B56 @0xb80610:
    b8061e: if (ecx < (signed) edx) goto B56  else B57   ; jl 0xb80610

  B57 @0xb80620:
    b80634: if (ax == 0xffff) goto B69  else B58   ; je 0xb806d4

  B58 @0xb8063a:
    b80649: if (esi == 0) goto B69  else B59   ; je 0xb806d4

  B59 @0xb8064f:
    b8064f: READ i32[0x22dbc10]   ; mov edi, dword ptr [0x22dbc10]

  B60 @0xb80655:
    b8065a: if (edx == 0) goto B67  else B61   ; je 0xb806b6

  B61 @0xb8065c:
    b8066a: X87 COMPARE f32[ebx + edi + 0xc] vs ST(1)   ; fcompp 
    b80671: if (x87 compare ('f32[ebx + edi + 0xc]', 'ST(1)'): ST0 outcome in {gt, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B66  else B62   ; jp 0xb806ae

  B62 @0xb80673:
    b8067f: X87 STORE f32 [esp + 0x18] <- ((f32[edx + 0x20] - f32[ebx + edi + 0xc]) / f32[esp + 0x28])
    b80687: X87 STORE f32 [esp] <- f32[esp + 0x18]
    b8068a: CALL 0x427600 floorf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b8068f: caller cleans 4 bytes
    b80692: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b806a2: if (ecx >= 0 (signed)) goto B64  else B63   ; jge 0xb806a8

  B63 @0xb806a4:
    b806a6: goto B66 @0xb806ae

  B64 @0xb806a8:
    b806aa: if (ecx <= (signed) eax) goto B66  else B65   ; jle 0xb806ae

  B65 @0xb806ac:

  B66 @0xb806ae:

  B67 @0xb806b6:
    b806c1: if (ax == word ptr [ecx + 0x2a]) goto B69  else B68   ; je 0xb806d4

  B68 @0xb806c3:
    b806d2: if (esi != 0) goto B60  else B69   ; jne 0xb80655

  B69 @0xb806d4:
    b806e0: X87 COMPARE ((float)i32[esp + 0x28] / f32[esp + 0x18]) vs ST(1)   ; fcompp 
    b806e7: if (x87 compare ('((float)i32[esp + 0x28] / f32[esp + 0x18])', 'ST(1)'): ST0 outcome in {gt, eq}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B102  else B70   ; je 0xb80ae0

  B70 @0xb806ed:
    b806ed: READ i32[0x22dbc10]   ; mov edx, dword ptr [0x22dbc10]
    b80711: X87 STORE f32 [esp + 0x20] <- (f32[ebx + edx + 0xc] + (f32[esp + 0x24] * f64(0.5; bits 0x3FE0000000000000)))
    b80715: if (dword ptr [esp + 0x2c] <= (signed) eax) goto B102  else B71   ; jle 0xb80ae0

  B71 @0xb8071b:
    b8071b: goto B72 @0xb80720

  B72 @0xb80720:
    b80733: if (byte ptr [esp + eax + 0x88] < (signed) 0) goto B79  else B73   ; jl 0xb8084a

  B73 @0xb80739:
    b80746: X87 STORE f32 [esp + 4] <- f32(0.004999999888241291; bits 0x3BA3D70A)
    b80751: X87 STORE f32 [esp] <- f32[esp + 0x28]
    b80754: CALL 0xa1f130 spline_param_at_distance  callee-reads[edi]  regs[edi<-(mov edi, dword ptr [esp + 0x38])]  stack(first arg first)[]  callee-ret-imm=0
    b80759: caller cleans 4 bytes
    b8075c: STACK-OBJECT &[esp+0x74] -> esi
    b80760: X87 STORE f32 [esp] <- ret(spline_param_at_distance)
    b80765: CALL 0xc03910 spline_eval_position  callee-reads[eax]  regs[eax<-(mov eax, edi)]  stack(first arg first)[]  callee-ret-imm=4
    b80772: X87 STORE f32 [esp + 0x3c] <- (f32[esp + 0x70] - f32[esp + 0x4c])
    b8077e: X87 STORE f32 [esp + 0x40] <- (f32[esp + 0x74] - f32[esp + 0x50])
    b8078a: X87 STORE f32 [esp + 0x44] <- (f32[esp + 0x78] - f32[esp + 0x54])
    b807b4: X87 STORE f32 [esp + 0x14] <- (((((ST(st(0)) * ST(st(2))) * ST(st(4))) + f32[esp + 0x44]) * ST(st(1))) + ST(st(0)))
    b807c4: X87 COMPARE (f32[esp + 0x14] * ST(st(1))) vs STst(1)   ; fcom st(1)
    b807cd: if (x87 compare ('(f32[esp + 0x14] * ST(st(1)))', 'STst(1)'): ST0 outcome in {gt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B78  else B74   ; je 0xb8083b

  B74 @0xb807cf:
    b807d5: X87 COMPARE (f32[esp + 0x58] * f32[esp + 0x58]) vs STst(1)   ; fcomp st(1)
    b807dc: if (x87 compare ('(f32[esp + 0x58] * f32[esp + 0x58])', 'STst(1)'): ST0 outcome in {gt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B78  else B75   ; je 0xb8083b

  B75 @0xb807de:
    b807f9: X87 STORE f32 [esp + 0x18] <- (ret(?) + ((ret(?) + ((ret(?) * f32[esp + 0x5c]) * f32[esp + 0x64])) * f32[esp + 0x68]))
    b80801: X87 STORE f32 [esp + 0x18] <- f32[esp + 0x18]
    b80805: X87 STORE f32 [esp] <- ret(?)
    b80808: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    b80811: caller cleans 4 bytes
    b80814: X87 STORE f32 [esp + 0x14] <- (f32[esp + 0x18] / ret(sqrtf_wrapper))
    b80822: X87 COMPARE f32[0xe86fb8] vs ST(1)   ; fcompp 
    b80829: if (x87 compare ('f32[0xe86fb8]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B77  else B76   ; jne 0xb80834

  B76 @0xb8082b:
    b80832: goto B79 @0xb8084a

  B77 @0xb80834:
    b80839: goto B79 @0xb8084a

  B78 @0xb8083b:

  B79 @0xb8084a:
    b80863: X87 STORE f32 [esp + 0x20] <- (f32[esp + 0x20] + f32[esp + 0x24])
    b80867: if (eax < (signed) dword ptr [esp + 0x2c]) goto B72  else B80   ; jl 0xb80720

  B80 @0xb8086d:
    b80872: if (dword ptr [esp + 0x28] == 0) goto B102  else B81   ; je 0xb80ae0

  B81 @0xb80878:
    b80884: STACK-OBJECT &[esp+0x94] -> eax
    b8088c: CALL 0xc96490 _qsort  callee-reads[]  regs[]  stack(first arg first)[eax, edx, 1, 0xb7f070]  callee-ret-imm=0
    b80891: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    b808a1: caller cleans 16 bytes
    b808aa: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b808b0: if (ecx < (signed) 0x800) goto B83  else B82   ; jl 0xb808ba

  B82 @0xb808b2:
    b808b4: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B83 @0xb808ba:
    b808ba: READ i32[0x22dbc10]   ; mov esi, dword ptr [0x22dbc10]
    b808ca: INT div dword ptr [esp + 0x28]
    b808d0: X87 COMPARE (f32[ebx + esi + 0x10] + f32[ebx + esi + 0x10]) vs STst(1)   ; fcomp st(1)
    b808de: if (x87 compare ('(f32[ebx + esi + 0x10] + f32[ebx + esi + 0x10])', 'STst(1)'): ST0 outcome in {gt, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B89  else B84   ; jp 0xb80957

  B84 @0xb808e0:
    b808ed: X87 STORE f32 [esp + 0x24] <- f32[ebx + esi + 0x10]
    b808f7: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b808fd: X87 STORE f32 [esp + 0x18] <- (ST(st(0)) - f32[ebx + esi + 0x10])
    b80901: if (ecx != 0x800) goto B86  else B85   ; jne 0xb8090d

  B85 @0xb80903:
    b80903: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B86 @0xb8090d:
    b80917: if (eax >= 0 (signed)) goto B88  else B87   ; jge 0xb8091f

  B87 @0xb80919:

  B88 @0xb8091f:
    b8092d: X87 STORE f32 [esp + 0x14] <- (ret(?) * f64(2.3283064365386963e-10; bits 0x3DF0000000000000))
    b80945: X87 STORE f32 [esp + 0x14] <- ((f32[esp + 0x24] - ST(st(0))) + (f32[esp + 0x18] * f32[esp + 0x14]))
    b80955: goto B94 @0xb809b4

  B89 @0xb80957:
    b80960: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b80966: if (ecx != 0x800) goto B91  else B90   ; jne 0xb80972

  B90 @0xb80968:
    b80968: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B91 @0xb80972:
    b8097c: if (eax >= 0 (signed)) goto B93  else B92   ; jge 0xb80984

  B92 @0xb8097e:

  B93 @0xb80984:
    b80992: X87 STORE f32 [esp + 0x14] <- (ret(?) * f64(2.3283064365386963e-10; bits 0x3DF0000000000000))
    b8099a: X87 fsub st(1), st(0)  (st(i) dest, untracked)
    b809a4: X87 STORE f32 [esp + 0x14] <- (0.0 + (ST(st(0)) * f32[esp + 0x14]))

  B94 @0xb809b4:
    b809c3: X87 STORE f32 [esp + 0x20] <- (ret(?) + ((ret(?) * ret(?)) + f32[ebx + esi + 0xc]))
    b809c7: CALL 0xb81790 sub_b81790  callee-reads[edi]  regs[edi<-(mov edi, 0x319f600)]  stack(first arg first)[]  callee-ret-imm=0
    b809d2: if (eax == 0) goto B103  else B95   ; je 0xb80aeb

  B95 @0xb809d8:
    b809d8: READ i32[0x22dbc10]   ; mov eax, dword ptr [0x22dbc10]
    b809dd: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    b809fa: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b80a00: if (ecx < (signed) 0x800) goto B97  else B96   ; jl 0xb80a0a

  B96 @0xb80a02:
    b80a04: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B97 @0xb80a0a:
    b80a0f: INT div esi
    b80a27: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    b80a2d: if (ecx < (signed) 0x800) goto B99  else B98   ; jl 0xb80a39

  B98 @0xb80a2f:
    b80a2f: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B99 @0xb80a39:
    b80a39: STACK-OBJECT &[esp+0x7c] -> esi
    b80a3d: CALL 0xb7fda0 sub_b7fda0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b80a47: READ i32[0x252a190]   ; mov ecx, dword ptr [0x252a190]
    b80a4d: X87 STORE f32 [esp] <- f32[esp + 0x20]
    b80a59: INT div ecx
    b80a5b: READ i32[0x22dbc1c]   ; mov eax, dword ptr [0x22dbc1c]
    b80a66: READ i32[0x22dbc10]   ; mov edx, dword ptr [0x22dbc10]
    b80a74: READ i32[0x22dbc20]   ; sub ecx, dword ptr [0x22dbc20]
    b80a7f: INT imul ecx
    b80a95: CALL 0xb7e580 sub_b7e580  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [esp + 0x44])]  stack(first arg first)[edx, ecx, ecx, eax, ecx]  callee-ret-imm=20
    b80aa1: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    b80aa8: X87 STORE f32 [esp + 0x14] <- ret(sub_68a500)
    b80ab4: if (edx >= 0 (signed)) goto B101  else B100   ; jge 0xb80abc

  B100 @0xb80ab6:

  B101 @0xb80abc:
    b80ad0: X87 STORE f32 [esp + 0x14] <- (f32[esp + 0x14] + (((1.0 - ST(st(0))) * f32[ebp + 0x20]) * f32[ebp + 0x1c]))
    b80add: X87 STORE f32 [ebp + 0x4c] <- (ret(?) / (f32[esp + 0x14] * f32[ebp + 0x48]))

  B102 @0xb80ae0:
    b80ae5: if (dword ptr [esp + 0x48] > (signed) 0) goto B39  else B103   ; jg 0xb80490

  B103 @0xb80aeb:
    b80aef: CALL 0xc15720 sub_c15720  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=None

  B104 @0xb80af4:
    b80b01: CALL 0xc9604f __security_check_cookie  callee-reads[ecx]  regs[ecx<-(xor ecx, esp)]  stack(first arg first)[]  callee-ret-imm=None
    b80b06: caller cleans 132 bytes
    b80b0c: RETURN

  --- summary ---
  absolute globals read: 0xe80b50, 0xe843ac, 0xe86fa0, 0xec3abc, 0x21703d4, 0x2225b64, 0x22dbc10, 0x22dbc1c, 0x22dbc20, 0x22fd858, 0x22fd864, 0x25295ff, 0x252a16b, 0x252a190, 0x252a198, 0x319f60c
  absolute globals written: 0xebd0c0, 0xec3abc, 0x25295ff
  direct callees: 0xb7efb0 ?, 0xbdd230 timer_elapsed, 0xbde880 ?, 0xb7e770 ?, 0xa8b8f0 ?, 0x4d9620 tod_object_unregister, 0xb81840 ?, 0xb819b0 allocator<>::num_used_89, 0xc15470 ?, 0xb7f020 ?, 0x4a7a50 ?, 0xb800c0 ?, 0x4a7a80 ?, 0x68a500 ?, 0x4912d0 ?, 0xda0140 _ftol2_sse, 0x427600 floorf_wrapper, 0xa1f130 spline_param_at_distance, 0xc03910 spline_eval_position, 0x40ea20 sqrtf_wrapper, 0xc96490 _qsort, 0xb81790 ?, 0xb7fda0 ?, 0xb7e580 ?, 0xc15720 ?, 0xc9604f __security_check_cookie
```

### ambient_peds_despawn_check - 0x936AB0

**Role.** Ambient pedestrian despawn: for each ped in the list `0x215D7B0` (count `0x215D940`) tests the view (`0xBE6FA0`, `object_visibility_cached`, camera `0x25F5B14..1C`) and the distance (squared radius from `f64 90.0` scaled by a per-frame factor), and despawns peds that are out of view and far enough; the local player's height test uses `fabs(dy) > 6.0` through the flag bit 2 of `[player+0x1164]`.

**Convention.** Entry registers read (linear scan): none. True size 0x8F8, 576 instructions, 84 blocks, 1 jump table(s). Callers: 0x938AE3, 0x938AF5.

**Sections, in order of the calls (address and callee):** 0x936AD5 gfx_get_thread_state; 0x936AE2 gfx_get_thread_state; 0x936BC6 sub_BE6FA0; 0x936C58 sub_A717F0; 0x936CB6 sub_BE6FA0; 0x936D17 sub_BE6FA0; 0x936DE4 sub_BE6FA0; 0x936E63 sub_BED5F0; 0x936E85 sub_BED5F0; 0x936EF9 cosf_wrapper; 0x936F0B sinf_wrapper; 0x937071 vec3_normalize_safe; 0x9370D0 sub_BDE290; 0x937165 sub_935E70; 0x93720E coop_spawn_count_scale; 0x937225 sub_68A500; 0x937353 ambient_ped_out_of_view_check; 0x937369 ambient_ped_try_place.

**Jump table** at 0x9373A8 used by the `jmp` at 0x937192: 0->0x9371A0, 1->0x9371A0, 2->0x9371A0.

**Hazards.** One jump table (a switch on the ped state); `fcompp` / `fcomp` polarity is spelled out in the map; stack frame 0x5F0 bytes with the cookie; mixes f32 stores with f64 constants (90.0), so keep intermediates in f64 and round to f32 at each `X87 STORE f32`.

**Globals read:** 0xEC3ABC, 0x140E72C, 0x215D940, 0x21703D4, 0x217043C, 0x2526DE8, 0x2526DEC, 0x2528B48, 0x2528B49, 0x2529634, 0x252963C.

**Globals written:** 0xEC3ABC.

```
FUNCTION 0x00936AB0  true size 0x8F8 (2296 bytes)  insns 576  blocks 84
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x5f0

  B0 @0x936ab0:
    936ac4: X87 STORE f32 [esp + 0x1c] <- f32[0x25f5b14]
    936ad0: X87 STORE f32 [esp + 0x28] <- f32[0x25f5b1c]
    936ad5: CALL 0xd22790 gfx_get_thread_state  callee-reads[]  regs[]  stack(first arg first)[edi]  callee-ret-imm=0
    936ae2: CALL 0xd22790 gfx_get_thread_state  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    936aff: X87 STORE f32 [esp + 0xa4] <- f32[esi + eax*4 + 4]
    936b09: X87 STORE f32 [esp + 0xa8] <- f32[eax + 4]
    936b13: X87 STORE f32 [esp + 0xac] <- f32[eax + 8]
    936b1f: READ i32[0x21703d4]   ; cmp dword ptr [0x21703d4], eax
    936b25: X87 STORE f32 [esp + 0xb0] <- f32[eax + 0xc]
    936b2c: if (dword ptr [0x21703d4] == eax) goto B2  else B1   ; je 0x936b36

  B1 @0x936b2e:
    936b34: goto B3 @0x936b3c

  B2 @0x936b36:

  B3 @0x936b3c:
    936b3c: READ i32[0x215d940]   ; cmp dword ptr [0x215d940], eax
    936b42: X87 STORE f32 [esp + 0x24] <- ret(?)
    936b58: X87 STORE f32 [esp + 0x18] <- (f32[esp + 0x24] * f64(90.0; bits 0x4056800000000000))
    936b62: X87 STORE f32 [esp + 0x58] <- (f32[esp + 0x18] * f32[esp + 0x18])
    936b66: if (dword ptr [0x215d940] <= (signed) eax) goto B19  else B4   ; jle 0x936ed4

  B4 @0x936b6c:
    936b6c: STACK-OBJECT &[esp+0x150] -> ecx

  B5 @0x936b77:
    936b8a: STACK-OBJECT &[esp+0xa4] -> esi
    936b93: STACK-OBJECT &[esp+0xc0] -> ecx
    936b9a: STACK-OBJECT &[esp+0xe4] -> edi
    936ba5: X87 STORE f32 [esp + 0xc0] <- (f32[ebp] - f32[edx])
    936bb2: X87 STORE f32 [esp + 0xc4] <- (f32[ebp + 4] - f32[edx + 4])
    936bbf: X87 STORE f32 [esp + 0xc8] <- (f32[ebp + 8] - f32[edx + 8])
    936bc6: CALL 0xbe6fa0 sub_be6fa0  callee-reads[ecx, esi]  regs[ecx<-(lea ecx, [esp + 0xc0]), esi<-(lea esi, [esp + 0xa4])]  stack(first arg first)[]  callee-ret-imm=0
    936bcd: if (al == 0) goto B18  else B6   ; je 0x936ebd

  B6 @0x936bd3:
    936bd3: READ i32[0x21703d4]   ; mov eax, dword ptr [0x21703d4]
    936bdf: if ((byte ptr [eax + 0x1164] & 2) == 0) goto B8  else B7   ; je 0x936c09

  B7 @0x936be1:
    936bf0: X87 STORE f32 [esp + 0x1c] <- fabs((f32[esp + 0xe8] - f32[0x25f5b18]))
    936bf8: X87 COMPARE f32[esp + 0x1c] vs f32(6.0; bits 0x40C00000)   ; fcomp dword ptr [0xdd32b4]
    936c03: if (x87 compare ('f32[esp + 0x1c]', 'f32(6.0; bits 0x40C00000)'): ST0 outcome in {gt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B18  else B8   ; jp 0x936ebd

  B8 @0x936c09:
    936c14: X87 STORE f32 [esp + 0x50] <- (f32[esp + 0xe4] - f32[esp + 0x28])
    936c23: X87 STORE f32 [esp + 0x54] <- (f32[esp + 0xec] - f32[esp + 0x2c])
    936c37: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x50] + ((f32[esp + 0x54] * f32[esp + 0x54]) * ST(st(1))))
    936c43: X87 COMPARE f32[esp + 0x58] vs ST(1)   ; fcompp 
    936c4a: if (x87 compare ('f32[esp + 0x58]', 'ST(1)'): ST0 outcome in {lt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B18  else B9   ; jnp 0x936ebd

  B9 @0x936c50:
    936c51: STACK-OBJECT &[esp+0xf8] -> edi
    936c58: CALL 0xa717f0 sub_a717f0  callee-reads[]  regs[]  stack(first arg first)[ebx]  callee-ret-imm=0
    936c6d: caller cleans 4 bytes
    936c70: STACK-OBJECT &[esp+0xa4] -> esi
    936c77: X87 STORE f32 [esp + 0xd8] <- (f32[esp + 0x104] - f32[esp + 0xf8])
    936c7e: STACK-OBJECT &[esp+0xd8] -> ecx
    936c8c: STACK-OBJECT &[esp+0x140] -> edi
    936c9a: X87 STORE f32 [esp + 0xdc] <- (f32[esp + 0x104] - f32[esp + 0xf8])
    936caf: X87 STORE f32 [esp + 0xe0] <- (f32[esp + 0x108] - f32[esp + 0xfc])
    936cb6: CALL 0xbe6fa0 sub_be6fa0  callee-reads[ecx, esi]  regs[ecx<-(lea ecx, [esp + 0xd8]), esi<-(lea esi, [esp + 0xa4])]  stack(first arg first)[]  callee-ret-imm=0
    936cbd: if (al == 0) goto B18  else B10   ; je 0x936ebd

  B10 @0x936cc3:
    936cca: STACK-OBJECT &[esp+0xcc] -> ecx
    936cd8: STACK-OBJECT &[esp+0x118] -> edx
    936cdf: STACK-OBJECT &[esp+0x130] -> edi
    936ce6: X87 STORE f32 [esp + 0xcc] <- (f32[esp + 0x10c] - f32[esp + 0x118])
    936cfb: X87 STORE f32 [esp + 0xd0] <- (f32[esp + 0x110] - f32[esp + 0x11c])
    936d10: X87 STORE f32 [esp + 0xd4] <- (f32[esp + 0x114] - f32[esp + 0x120])
    936d17: CALL 0xbe6fa0 sub_be6fa0  callee-reads[ecx, esi]  regs[ecx<-(lea ecx, [esp + 0xcc]), esi<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    936d1e: if (al == 0) goto B18  else B11   ; je 0x936ebd

  B11 @0x936d24:
    936d3b: X87 fmul st(1), st(0)  (st(i) dest, untracked)
    936d4d: X87 STORE f32 [esp + 0x38] <- f32[0x25f5b50]
    936d51: STACK-OBJECT &[esp+0xb4] -> ecx
    936d5e: STACK-OBJECT &[esp+0x80] -> edx
    936d65: X87 fmul st(1)  (st(i) dest, untracked)
    936d67: STACK-OBJECT &[esp+0xe4] -> edi
    936d70: X87 STORE f32 [esp + 0x3c] <- f32[0x25f5b54]
    936d7a: X87 STORE f32 [esp + 0x40] <- (f64(2.0; bits 0x4000000000000000) * f32[0x25f5b58])
    936d88: X87 STORE f32 [esp + 0x80] <- (f32[esp + 0x38] + f32[0x25f5b14])
    936d99: X87 STORE f32 [esp + 0x84] <- (f32[esp + 0x3c] + f32[0x25f5b18])
    936daa: X87 STORE f32 [esp + 0x88] <- (f32[esp + 0x40] + f32[0x25f5b1c])
    936dbb: X87 STORE f32 [esp + 0xb4] <- (f32[ebp] - f32[esp + 0x80])
    936dcc: X87 STORE f32 [esp + 0xb8] <- (f32[ebp + 4] - f32[esp + 0x84])
    936ddd: X87 STORE f32 [esp + 0xbc] <- (f32[ebp + 8] - f32[esp + 0x88])
    936de4: CALL 0xbe6fa0 sub_be6fa0  callee-reads[ecx, esi]  regs[ecx<-(lea ecx, [esp + 0xb4]), esi<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    936dee: if (al == 0) goto B13  else B12   ; je 0x936e1f

  B12 @0x936df0:
    936e1d: goto B14 @0x936e4c

  B13 @0x936e1f:

  B14 @0x936e4c:
    936e4c: STACK-OBJECT &[esp+0x140] -> ecx
    936e54: STACK-OBJECT &[esp+0x78] -> eax
    936e58: STACK-OBJECT &[esp+0x6c] -> esi
    936e5c: STACK-OBJECT &[esp+0x128] -> edi
    936e63: CALL 0xbed5f0 sub_bed5f0  callee-reads[eax, esi]  regs[eax<-(lea eax, [esp + 0x78]), esi<-(lea esi, [esp + 0x6c])]  stack(first arg first)[ecx]  callee-ret-imm=0
    936e68: SSE movss dword ptr [esp + 0x38], xmm0
    936e72: STACK-OBJECT &[esp+0x134] -> edx
    936e79: caller cleans 4 bytes
    936e7c: X87 STORE f32 [esp + 0x1c] <- f32[esp + 0x38]
    936e81: STACK-OBJECT &[esp+0x78] -> eax
    936e85: CALL 0xbed5f0 sub_bed5f0  callee-reads[eax, esi]  regs[eax<-(lea eax, [esp + 0x78]), esi<-(lea esi, [esp + 0x6c])]  stack(first arg first)[edx]  callee-ret-imm=0
    936e8a: SSE movss dword ptr [esp + 0x18], xmm0
    936e94: caller cleans 4 bytes
    936e97: X87 STORE f32 [esp + 0x18] <- f32[esp + 0x18]
    936ea3: X87 COMPARE f32[esp + 0x18] vs STst(1)   ; fcom st(1)
    936eaa: if (x87 compare ('f32[esp + 0x18]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B16  else B15   ; jne 0x936eb0

  B15 @0x936eac:
    936eae: goto B17 @0x936eb2

  B16 @0x936eb0:

  B17 @0x936eb2:
    936eb2: X87 STORE f32 [esp + 0x18] <- ret(?)
    936eba: X87 STORE f32 [ebx + 4] <- f32[esp + 0x18]

  B18 @0x936ebd:
    936ec4: READ i32[0x215d940]   ; cmp eax, dword ptr [0x215d940]
    936ece: if (eax < (signed) dword ptr [0x215d940]) goto B5  else B19   ; jl 0x936b77

  B19 @0x936ed4:

  B20 @0x936edc:
    936ee1: if (dword ptr [esp + 0x24] <= (signed) 0) goto B83  else B21   ; jle 0x93739d

  B21 @0x936ee7:
    936eee: X87 STORE f32 [esp] <- f32[0xedb7f4]
    936ef9: CALL 0x40b820 cosf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    936efe: X87 STORE f32 [esp + 0x20] <- ret(cosf_wrapper)
    936f08: X87 STORE f32 [esp] <- f32[0xedb7f4]
    936f0b: CALL 0x428cd0 sinf_wrapper  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    936f10: X87 STORE f32 [esp + 0x5c] <- ret(sinf_wrapper)
    936f14: caller cleans 4 bytes

  B22 @0x936f17:
    936f17: READ i32[0xec3abc]   ; mov ecx, dword ptr [0xec3abc]
    936f2d: if (ecx < (signed) 0x800) goto B24  else B23   ; jl 0x936f31

  B23 @0x936f2f:

  B24 @0x936f31:
    936f33: INT div dword ptr [esp + 0x24]
    936f40: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    936f57: if (ecx != 0x800) goto B26  else B25   ; jne 0x936f61

  B25 @0x936f59:
    936f5b: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B26 @0x936f61:
    936f6e: X87 STORE f32 [esp + 0x34] <- f32[eax + 4]
    936f78: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    936f7e: if (ecx != 0x800) goto B28  else B27   ; jne 0x936f88

  B27 @0x936f80:
    936f82: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx

  B28 @0x936f88:
    936f92: if (esi >= 0 (signed)) goto B30  else B29   ; jge 0x936f9a

  B29 @0x936f94:

  B30 @0x936f9a:
    936fa7: X87 fmul st(1), st(0)  (st(i) dest, untracked)
    936fb4: WRITE i32[0xec3abc] mov = ecx   ; dword ptr [0xec3abc], ecx
    936fba: X87 STORE f32 [esp + 0x14] <- f64(2.3283064365386963e-10; bits 0x3DF0000000000000)
    936fc4: X87 fsub st(1), st(0)  (st(i) dest, untracked)
    936fce: X87 STORE f32 [esp + 0x34] <- (0.0 + (f32[esp + 0x34] * f32[esp + 0x14]))
    936fd2: if (ecx != 0x800) goto B32  else B31   ; jne 0x936fde

  B31 @0x936fd4:
    936fd4: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B32 @0x936fde:
    936fe8: if (esi >= 0 (signed)) goto B34  else B33   ; jge 0x936ff0

  B33 @0x936fea:

  B34 @0x936ff0:
    936ff6: X87 STORE f32 [esp + 0x14] <- (ret(?) * ret(?))
    937004: X87 STORE f32 [esp + 0x14] <- (f32[esp + 0x14] - f64(0.5; bits 0x3FE0000000000000))
    93700f: X87 STORE f32 [esp + 0x10] <- (f32[esp + 0x14] * f32[edx + 8])
    937013: if (byte ptr [eax + 8] == 0) goto B36  else B35   ; je 0x93701c

  B35 @0x937015:
    93701a: goto B37 @0x937021

  B36 @0x93701c:

  B37 @0x937021:
    937049: X87 STORE f32 [esp + 0x28] <- (f32[esp + 0x38] - f32[esp + 0x44])
    93705d: STACK-OBJECT &[esp+0x28] -> esi
    937061: X87 STORE f32 [esp + 0x2c] <- (f32[esp + 0x3c] - f32[esp + 0x48])
    93706d: X87 STORE f32 [esp + 0x30] <- (f32[esp + 0x40] - f32[esp + 0x4c])
    937071: CALL 0xbde150 vec3_normalize_safe  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x28])]  stack(first arg first)[]  callee-ret-imm=0
    937084: X87 STORE f32 [esp + 0x8c] <- f32[esp + 0x28]
    93708f: X87 fmul st(1)  (st(i) dest, untracked)
    937091: X87 STORE f32 [esp + 0x90] <- f32[esp + 0x2c]
    93709c: X87 STORE f32 [esp + 0x94] <- ((f32[esp + 0x34] * ST(st(0))) * f32[esp + 0x30])
    9370ae: X87 STORE f32 [esp + 0x5c] <- (f32[esp + 0x8c] + f32[esp + 0x44])
    9370bd: X87 STORE f32 [esp + 0x60] <- (f32[esp + 0x90] + f32[esp + 0x48])
    9370cc: X87 STORE f32 [esp + 0x64] <- (f32[esp + 0x94] + f32[esp + 0x4c])
    9370d0: CALL 0xbde290 sub_bde290  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x28])]  stack(first arg first)[]  callee-ret-imm=0
    9370d9: X87 STORE f32 [esp + 0x14] <- f32[esp + 0x28]
    9370f9: X87 STORE f32 [esp + 0x28] <- ST(st(0))
    937107: X87 STORE f32 [esp + 0x30] <- (((f32[esp + 0x30] * ((f32[esp + 0x1c] * ST(st(0))) + (f32[esp + 0x58] * ST(st(0))))) * f32[esp + 0x14]) - f32[esp + 0x28])
    937119: X87 STORE f32 [esp + 0x98] <- f32[esp + 0x28]
    937124: X87 fmul st(1)  (st(i) dest, untracked)
    937126: X87 STORE f32 [esp + 0x9c] <- f32[esp + 0x2c]
    937131: X87 STORE f32 [esp + 0xa0] <- ((f32[esp + 0x10] * ST(st(0))) * f32[esp + 0x30])
    937143: X87 STORE f32 [esp + 0x5c] <- (f32[esp + 0x98] + f32[esp + 0x5c])
    937152: X87 STORE f32 [esp + 0x60] <- (f32[esp + 0x9c] + f32[esp + 0x60])
    937161: X87 STORE f32 [esp + 0x64] <- (f32[esp + 0xa0] + f32[esp + 0x64])
    937165: CALL 0x935e70 sub_935e70  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93716c: READ i32[0x2529634]   ; mov eax, dword ptr [0x2529634]
    937175: if (eax == 0) goto B78  else B38   ; je 0x93734f

  B38 @0x93717b:
    93717b: READ u8[0x140e72c]   ; movzx eax, byte ptr [0x140e72c]
    937185: if (result >= 0 (sign clear) of `sub eax, 6`) goto B40  else B39   ; jns 0x93718a

  B39 @0x937187:

  B40 @0x93718a:
    937190: if (eax > (unsigned) 5) goto B42  else B41   ; ja 0x9371a0

  B41 @0x937192:
    937192: SWITCH dword ptr [eax*4 + 0x9373a8] table 0x9373a8, 3 entries: 0->B42, 1->B42, 2->B42

  B42 @0x9371a0:
    9371a2: READ u8[0x2528b48]   ; mov dl, byte ptr [0x2528b48]
    9371ab: if (dl != 1) goto B45  else B43   ; jne 0x9371bd

  B43 @0x9371ad:
    9371ad: READ u8[0x2528b49]   ; mov al, byte ptr [0x2528b49]
    9371b4: if (al != 0) goto B45  else B44   ; jne 0x9371bd

  B44 @0x9371b6:
    9371bb: goto B55 @0x937218

  B45 @0x9371bd:
    9371bd: READ u8[0x2528b49]   ; mov cl, byte ptr [0x2528b49]
    9371c5: if (cl == 0) goto B53  else B46   ; je 0x9371fb

  B46 @0x9371c7:
    9371c7: READ i32[0x217043c]   ; mov ecx, dword ptr [0x217043c]
    9371d1: if (eax == 0) goto B53  else B47   ; je 0x9371fb

  B47 @0x9371d3:
    9371d3: READ i32[0x21703d4]   ; mov edx, dword ptr [0x21703d4]
    9371d9: STACK-OBJECT &[esp+0x0] -> esp

  B48 @0x9371e0:
    9371e2: if (eax != edx) goto B52  else B49   ; jne 0x9371f7

  B49 @0x9371e4:
    9371ec: if (eax == ecx) goto B53  else B50   ; je 0x9371fb

  B50 @0x9371ee:
    9371f0: if (eax != 0) goto B48  else B51   ; jne 0x9371e0

  B51 @0x9371f2:
    9371f5: goto B55 @0x937218

  B52 @0x9371f7:
    9371f9: if (eax != 0) goto B54  else B53   ; jne 0x937202

  B53 @0x9371fb:
    937200: goto B55 @0x937218

  B54 @0x937202:
    93720b: X87 STORE f32 [esp] <- f32[0xe849a8]
    93720e: CALL 0x93e8d0 coop_spawn_count_scale  callee-reads[]  regs[]  stack(first arg first)[ecx, 0x1a]  callee-ret-imm=0
    937213: caller cleans 8 bytes

  B55 @0x937218:
    937218: READ i32[0x252963c]   ; mov eax, dword ptr [0x252963c]
    93721f: if (eax != 0) goto B71  else B56   ; jne 0x937305

  B56 @0x937225:
    937225: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93722a: X87 STORE f32 [esp + 0x14] <- ret(sub_68a500)
    937232: READ i32[0x2526de8]   ; mov eax, dword ptr [0x2526de8]
    93723e: READ i32[0x2526dec]   ; mov eax, dword ptr [0x2526dec]
    937254: X87 STORE f32 [esp + 0x10] <- (f32[esp + 0x14] + (((1.0 - ST(st(0))) * f32[ebx + 0x28]) * f32[ebx + 0x24]))
    937258: if (eax == 0) goto B58  else B57   ; je 0x937265

  B57 @0x93725a:
    937261: X87 STORE f32 [esp + 0x10] <- (f32[eax + 0x14] * f32[esp + 0x10])

  B58 @0x937265:
    93726c: STACK-OBJECT &[esp+0x0] -> esp

  B59 @0x937270:
    937274: if (ecx <= (signed) edx) goto B61  else B60   ; jle 0x937278

  B60 @0x937276:

  B61 @0x937278:
    937280: if (eax < (signed) 0x279ab50) goto B59  else B62   ; jl 0x937270

  B62 @0x937282:
    93728d: X87 STORE f32 [esp + 0x10] <- (f32[edx*4 + 0xe888f4] * f32[esp + 0x10])
    93729b: X87 COMPARE f32[esp + 0x10] vs STst(1)   ; fcom st(1)
    9372a2: if (x87 compare ('f32[esp + 0x10]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B64  else B63   ; jne 0x9372a8

  B63 @0x9372a4:
    9372a6: goto B65 @0x9372aa

  B64 @0x9372a8:

  B65 @0x9372aa:
    9372aa: X87 STORE f32 [esp + 0x10] <- ret(?)
    9372b6: X87 COMPARE f32[0xe888f0] vs STst(1)   ; fcom st(1)
    9372bf: if (x87 compare ('f32[0xe888f0]', 'STst(1)'): ST0 outcome in {lt, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B67  else B66   ; jne 0x9372c7

  B66 @0x9372c1:
    9372c1: X87 STORE f32 [esp + 0x10] <- ret(?)
    9372c5: goto B68 @0x9372c9

  B67 @0x9372c7:

  B68 @0x9372c9:
    9372d5: if (ecx >= 0 (signed)) goto B70  else B69   ; jge 0x9372dd

  B69 @0x9372d7:

  B70 @0x9372dd:
    9372f7: X87 STORE int64 [esp + 0x50] <- round((ret(?) * f32[esp + 0x10]))
    937303: goto B74 @0x93733f

  B71 @0x937305:
    93730f: if (edx >= 0 (signed)) goto B73  else B72   ; jge 0x937317

  B72 @0x937311:

  B73 @0x937317:
    937333: X87 STORE int64 [esp + 0x50] <- round((ret(?) * f32[0xe888b0]))

  B74 @0x93733f:
    937342: if (eax > (unsigned) 1) goto B76  else B75   ; ja 0x937349

  B75 @0x937344:

  B76 @0x937349:
    93734b: if (eax < (unsigned) esi) goto B78  else B77   ; jb 0x93734f

  B77 @0x93734d:

  B78 @0x93734f:
    937351: if (ebp < (signed) eax) goto B80  else B79   ; jl 0x93735c

  B79 @0x937353:
    937353: CALL 0x936a10 ambient_ped_out_of_view_check  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93735a: if (al == 0) goto B81  else B80   ; je 0x937375

  B80 @0x93735c:
    937362: STACK-OBJECT &[esp+0x68] -> eax
    937369: CALL 0x936310 ambient_ped_try_place  callee-reads[]  regs[]  stack(first arg first)[eax, 0, 0, 0, 0]  callee-ret-imm=0
    93736e: caller cleans 20 bytes
    937373: if (eax != 0) goto B82  else B81   ; jne 0x937389

  B81 @0x937375:
    937383: if (eax < (signed) 3) goto B22  else B82   ; jl 0x936f17

  B82 @0x937389:
    937397: if (eax < (signed) 1) goto B20  else B83   ; jl 0x936edc

  B83 @0x93739d:
    9373a1: caller cleans 1520 bytes
    9373a7: RETURN

  --- summary ---
  absolute globals read: 0xec3abc, 0x140e72c, 0x215d940, 0x21703d4, 0x217043c, 0x2526de8, 0x2526dec, 0x2528b48, 0x2528b49, 0x2529634, 0x252963c
  absolute globals written: 0xec3abc
  direct callees: 0xd22790 gfx_get_thread_state, 0xbe6fa0 ?, 0xa717f0 ?, 0xbed5f0 ?, 0x40b820 cosf_wrapper, 0x428cd0 sinf_wrapper, 0xbde150 vec3_normalize_safe, 0xbde290 ?, 0x935e70 ?, 0x93e8d0 coop_spawn_count_scale, 0x68a500 ?, 0x936a10 ambient_ped_out_of_view_check, 0x936310 ambient_ped_try_place
```

### spawn_points_search_in_range - 0x93DE60

**Role.** Spawn point search: finds spawn points of the table `0x2529E20` (count `[+0x10]`, array `[+0x14]`) inside two squared ranges, with optional filter id (0xFFFF = none) and a random pick (`rng_float01` through 0x937F10); 0x584 bytes of locals.

**Convention.** Entry registers read (linear scan): none. True size 0x8BB, 618 instructions, 74 blocks, 0 jump table(s). Callers: 0x59E0EC, 0x59E71F, 0x5DFFF4, 0x5F987A, 0x62B891, 0x62C287, 0x65C019, 0x6725B9.

**Sections, in order of the calls (address and callee):** 0x93DED4 sub_BED5F0; 0x93DF89 vec3_distance; 0x93DFAA sub_4912D0; 0x93DFB2 _ftol2_sse; 0x93DFD6 sub_4A9A60; 0x93E0C6 sqrtf_wrapper; 0x93E185 sub_937F10; 0x93E1D7 sub_BE49A0; 0x93E209 vec3_normalize_safe_len; 0x93E29A sub_BDE290; 0x93E2A9 cosf_wrapper; 0x93E2BB sinf_wrapper; 0x93E3A3 sub_4A9A60; 0x93E431 sub_BDF050; 0x93E44D sub_BE4C80; 0x93E474 sub_A8B9F0; 0x93E49E spline_eval_position; 0x93E522 nav_find_node_near_point; 0x93E536 nav_find_node_near_point; 0x93E5CD sub_A04A30; 0x93E67A sqrtf_wrapper.

**Hazards.** Null table returns 0 at once; first section squares the two f32 range arguments (`fmul st0, st0`) and stores them as f32 before any compare; six float args on the stack above `ebp+8`; `and esp, -8` alignment.

**Globals read:** 0xEC3ABC, 0x2529E20.

**Globals written:** 0xEC3ABC.

```
FUNCTION 0x0093DE60  true size 0x8BB (2235 bytes)  insns 618  blocks 74
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x584

  B0 @0x93de60:
    93de66: READ i32[0x2529e20]   ; mov eax, dword ptr [0x2529e20]
    93de7b: if (eax != edi) goto B2  else B1   ; jne 0x93de86

  B1 @0x93de7d:
    93de85: RETURN

  B2 @0x93de86:
    93de9d: X87 STORE f32 [esp + 0x2c] <- (f32[ebp + 0xc] * f32[ebp + 0xc])
    93deb7: X87 STORE f32 [esp + 0x14] <- (f32[ebp + 0x10] * f32[ebp + 0x10])
    93debb: if (dword ptr [eax + 0x10] <= (signed) edi) goto B1  else B3   ; jle 0x93de7d

  B3 @0x93debd:
    93dec1: if (dword ptr [ecx + 0xc] != 0) goto B25  else B4   ; jne 0x93e053

  B4 @0x93dec7:
    93ded0: STACK-OBJECT &[esp+0x50] -> edi
    93ded4: CALL 0xbed5f0 sub_bed5f0  callee-reads[eax, esi]  regs[eax<-(mov eax, dword ptr [ecx + 4]), esi<-(mov esi, dword ptr [ecx])]  stack(first arg first)[eax]  callee-ret-imm=0
    93dee4: caller cleans 4 bytes
    93dee9: X87 STORE f32 [esp + 0x18] <- (ST(st(0)) - f32[eax])
    93def3: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x18] * f32[esp + 0x18])
    93df00: X87 STORE f32 [esp + 0x18] <- (ST(st(0)) - f32[eax + 4])
    93df0e: X87 STORE f32 [esp + 0x1c] <- ((f32[esp + 0x18] * f32[esp + 0x18]) + f32[esp + 0x1c])
    93df1b: X87 STORE f32 [esp + 0x18] <- (ST(st(0)) - f32[eax + 8])
    93df29: X87 STORE f32 [esp + 0x1c] <- ((f32[esp + 0x18] * f32[esp + 0x18]) + f32[esp + 0x1c])
    93df35: X87 COMPARE f32[esp + 0x14] vs STst(1)   ; fcomp st(1)
    93df3c: if (x87 compare ('f32[esp + 0x14]', 'STst(1)'): ST0 outcome in {lt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B22  else B5   ; jnp 0x93e043

  B5 @0x93df42:
    93df46: X87 COMPARE f32[esp + 0x2c] vs ST(1)   ; fcompp 
    93df4d: if (x87 compare ('f32[esp + 0x2c]', 'ST(1)'): ST0 outcome in {gt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B23  else B6   ; je 0x93e045

  B6 @0x93df53:
    93df58: if (ecx == 0) goto B9  else B7   ; je 0x93df78

  B7 @0x93df5a:
    93df61: X87 STORE f32 [eax] <- ret(?)
    93df63: X87 STORE f32 [eax + 4] <- ret(?)
    93df66: X87 STORE f32 [eax + 8] <- ret(?)
    93df69: CALL indirect ecx  regs[ecx<-(?) eax<-(mov eax, esp)]  stack(first arg first)[]
    93df6b: caller cleans 12 bytes
    93df70: if (al == 0) goto B24  else B8   ; je 0x93e04b

  B8 @0x93df76:
    93df76: goto B10 @0x93df7e

  B9 @0x93df78:

  B10 @0x93df7e:
    93df89: CALL 0xbde980 vec3_distance  callee-reads[]  regs[]  stack(first arg first)[edx, ecx]  callee-ret-imm=0
    93df8e: X87 STORE f32 [esp + 0x24] <- ret(vec3_distance)
    93df96: caller cleans 4 bytes
    93df9f: X87 STORE f32 [esp + 0x20] <- (f32[esp + 0x24] * f64(0.10000000149011612; bits 0x3FB99999A0000000))
    93dfa7: X87 STORE f32 [esp] <- f32[esp + 0x20]
    93dfaa: CALL 0x4912d0 sub_4912d0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93dfaf: caller cleans 4 bytes
    93dfb2: CALL 0xda0140 _ftol2_sse  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93dfb9: if (eax <= 0 (signed)) goto B24  else B11   ; jle 0x93e04b

  B11 @0x93dfbf:
    93dfc4: if (byte ptr [esp + 0x2b] == 0) goto B14  else B12   ; je 0x93dff8

  B12 @0x93dfc6:
    93dfc8: STACK-OBJECT &[esp+0x1c] -> eax
    93dfd6: CALL 0x4a9a60 sub_4a9a60  callee-reads[]  regs[]  stack(first arg first)[ecx, eax]  callee-ret-imm=0
    93dfe2: caller cleans 8 bytes
    93dfef: if (eax == dword ptr [ebp + 0x28]) goto B15  else B13   ; je 0x93dffc

  B13 @0x93dff1:
    93dff6: goto B15 @0x93dffc

  B14 @0x93dff8:

  B15 @0x93dffc:
    93e00d: if (ecx < (signed) 0x64) goto B24  else B16   ; jl 0x93e04b

  B16 @0x93e00f:

  B17 @0x93e011:
    93e013: if (edi == 0) goto B1  else B18   ; je 0x93de7d

  B18 @0x93e019:
    93e01f: X87 STORE f32 [esp + 0x18] <- 0.0
    93e023: if (edi <= 0 (signed)) goto B29  else B19   ; jle 0x93e0ef

  B19 @0x93e029:
    93e02e: if (byte ptr [esp + 0x13] == 0) goto B27  else B20   ; je 0x93e076

  B20 @0x93e030:
    93e03a: if (dword ptr [esp + esi*4 + 0x400] == eax) goto B27  else B21   ; je 0x93e076

  B21 @0x93e03c:
    93e03e: goto B28 @0x93e0ce

  B22 @0x93e043:

  B23 @0x93e045:

  B24 @0x93e04b:

  B25 @0x93e053:
    93e057: READ i32[0x2529e20]   ; mov edx, dword ptr [0x2529e20]
    93e06e: if (eax < (signed) dword ptr [edx + 0x10]) goto B3  else B26   ; jl 0x93debd

  B26 @0x93e074:
    93e074: goto B17 @0x93e011

  B27 @0x93e076:
    93e087: X87 STORE f32 [esp + 0x44] <- (f32[ecx] - f32[eax])
    93e091: X87 STORE f32 [esp + 0x48] <- (f32[ecx + 4] - f32[eax + 4])
    93e09b: X87 STORE f32 [esp + 0x4c] <- (f32[ecx + 8] - f32[eax + 8])
    93e0bb: X87 STORE f32 [esp + 0x30] <- (f32[esp + 0x44] + ((((f32[esp + 0x4c] * ST(st(1))) * ST(st(2))) + f32[esp + 0x48]) * (((f32[esp + 0x4c] * ST(st(1))) * ST(st(2))) + f32[esp + 0x48])))
    93e0c3: X87 STORE f32 [esp] <- f32[esp + 0x30]
    93e0c6: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    93e0cb: caller cleans 4 bytes

  B28 @0x93e0ce:
    93e0ce: X87 STORE f32 [esp + esi*4 + 0xe0] <- ret(?)
    93e0e5: X87 STORE f32 [esp + 0x18] <- (f32[esp + esi*4 + 0xdc] + f32[esp + 0x18])
    93e0e9: if (esi < (signed) edi) goto B19  else B29   ; jl 0x93e029

  B29 @0x93e0ef:
    93e0f8: if (edi < (signed) 4) goto B32  else B30   ; jl 0x93e144

  B30 @0x93e0fa:

  B31 @0x93e0fd:
    93e109: X87 fdiv st(1)  (st(i) dest, untracked)
    93e10b: X87 STORE f32 [esp + eax*4 + 0xd0] <- f32[esp + eax*4 + 0xe0]
    93e119: X87 fdiv st(1)  (st(i) dest, untracked)
    93e11b: X87 STORE f32 [esp + eax*4 + 0xd4] <- f32[esp + eax*4 + 0xd4]
    93e129: X87 fdiv st(1)  (st(i) dest, untracked)
    93e12b: X87 STORE f32 [esp + eax*4 + 0xd8] <- f32[esp + eax*4 + 0xd8]
    93e139: X87 fdiv st(1)  (st(i) dest, untracked)
    93e13b: X87 STORE f32 [esp + eax*4 + 0xdc] <- f32[esp + eax*4 + 0xdc]
    93e142: if (eax < (signed) ecx) goto B31  else B32   ; jl 0x93e0fd

  B32 @0x93e144:
    93e146: if (eax >= (signed) edi) goto B34  else B33   ; jge 0x93e15f

  B33 @0x93e148:
    93e154: X87 fdiv st(1)  (st(i) dest, untracked)
    93e156: X87 STORE f32 [esp + eax*4 + 0xdc] <- f32[esp + eax*4 + 0xe0]
    93e15d: if (eax < (signed) edi) goto B33  else B34   ; jl 0x93e148

  B34 @0x93e15f:
    93e16c: STACK-OBJECT &[esp+0x0] -> esp

  B35 @0x93e170:

  B36 @0x93e172:
    93e172: STACK-OBJECT &[esp+0x270] -> ecx
    93e17a: STACK-OBJECT &[esp+0x28] -> edi
    93e17e: STACK-OBJECT &[esp+0xe4] -> esi
    93e185: CALL 0x937f10 sub_937f10  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    93e18c: caller cleans 4 bytes
    93e191: if (edi == 0) goto B40  else B37   ; je 0x93e1ae

  B37 @0x93e193:
    93e197: if (dword ptr [edi + 0xc] == 0) goto B40  else B38   ; je 0x93e1ae

  B38 @0x93e199:
    93e1a3: if (byte ptr [esp + 0x13] == 0) goto B36  else B39   ; je 0x93e172

  B39 @0x93e1a5:
    93e1ad: RETURN

  B40 @0x93e1ae:
    93e1bb: X87 STORE f32 [esp + 0x34] <- (f32[ecx] - f32[eax])
    93e1c5: X87 STORE f32 [esp + 0x38] <- (f32[ecx + 4] - f32[eax + 4])
    93e1cf: STACK-OBJECT &[esp+0x34] -> eax
    93e1d3: X87 STORE f32 [esp + 0x3c] <- (f32[ecx + 8] - f32[eax + 8])
    93e1d7: CALL 0xbe49a0 sub_be49a0  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0x34])]  stack(first arg first)[edx]  callee-ret-imm=4
    93e1dc: READ i32[0xec3abc]   ; mov eax, dword ptr [0xec3abc]
    93e1f4: WRITE i32[0xec3abc] mov = eax   ; dword ptr [0xec3abc], eax
    93e1f9: if (eax != 0x800) goto B42  else B41   ; jne 0x93e205

  B41 @0x93e1fb:
    93e1fb: WRITE i32[0xec3abc] mov = 0x0   ; dword ptr [0xec3abc], 0

  B42 @0x93e205:
    93e205: STACK-OBJECT &[esp+0x30] -> esi
    93e209: CALL 0xbde340 vec3_normalize_safe_len  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x30])]  stack(first arg first)[]  callee-ret-imm=0
    93e214: SSE movss dword ptr [esp + 0x2c], xmm0
    93e222: if (edx >= 0 (signed)) goto B44  else B43   ; jge 0x93e22a

  B43 @0x93e224:

  B44 @0x93e22a:
    93e232: STACK-OBJECT &[esp+0x30] -> esi
    93e236: X87 STORE f32 [esp + 0x14] <- (ret(?) * f64(2.3283064365386963e-10; bits 0x3DF0000000000000))
    93e23e: X87 STORE f32 [esp + 0x14] <- (ret(?) * f32[esp + 0x14])
    93e250: X87 STORE f32 [esp + 0x64] <- f32[esp + 0x30]
    93e258: X87 fmul st(1)  (st(i) dest, untracked)
    93e25a: X87 STORE f32 [esp + 0x68] <- f32[esp + 0x34]
    93e262: X87 STORE f32 [esp + 0x6c] <- ((f32[esp + 0x14] * ST(st(0))) * f32[esp + 0x38])
    93e26c: X87 STORE f32 [esp + 0x70] <- (f32[eax] + f32[esp + 0x64])
    93e277: X87 STORE f32 [esp + 0x74] <- (f32[eax + 4] + f32[esp + 0x68])
    93e28f: X87 STORE f32 [esp + 0x78] <- (f32[eax + 8] + f32[esp + 0x6c])
    93e29a: CALL 0xbde290 sub_bde290  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x30])]  stack(first arg first)[]  callee-ret-imm=0
    93e2a6: X87 STORE f32 [esp] <- f32[0xedb7f4]
    93e2a9: CALL 0x40b820 cosf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    93e2ae: X87 STORE f32 [esp + 0x40] <- ret(cosf_wrapper)
    93e2b8: X87 STORE f32 [esp] <- f32[0xedb7f4]
    93e2bb: CALL 0x428cd0 sinf_wrapper  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93e2c0: X87 STORE f32 [esp + 0x18] <- ret(sinf_wrapper)
    93e2c4: READ i32[0xec3abc]   ; mov eax, dword ptr [0xec3abc]
    93e2d4: X87 STORE f32 [esp + 0x20] <- f32[esp + 0x34]
    93e2df: caller cleans 4 bytes
    93e2f1: WRITE i32[0xec3abc] mov = eax   ; dword ptr [0xec3abc], eax
    93e306: X87 STORE f32 [esp + 0x30] <- ST(st(0))
    93e314: X87 STORE f32 [esp + 0x38] <- (((f32[esp + 0x3c] * ((f32[esp + 0x3c] * ST(st(0))) + (f32[esp + 0x14] * ST(st(0))))) * f32[esp + 0x1c]) - f32[esp + 0x34])
    93e318: if (eax != 0x800) goto B46  else B45   ; jne 0x93e320

  B45 @0x93e31a:
    93e31a: WRITE i32[0xec3abc] mov = esi   ; dword ptr [0xec3abc], esi

  B46 @0x93e320:
    93e32a: if (ecx >= 0 (signed)) goto B48  else B47   ; jge 0x93e332

  B47 @0x93e32c:

  B48 @0x93e332:
    93e338: STACK-OBJECT &[esp+0x14] -> eax
    93e33e: X87 STORE f32 [esp + 0x1c] <- (ret(?) * f64(2.3283064365386963e-10; bits 0x3DF0000000000000))
    93e34c: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x1c] - f64(0.5; bits 0x3FE0000000000000))
    93e357: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x1c] * f32[edi + 8])
    93e371: X87 STORE f32 [esp + 0x60] <- f32[esp + 0x38]
    93e379: X87 fmul st(1)  (st(i) dest, untracked)
    93e37b: X87 STORE f32 [esp + 0x64] <- f32[esp + 0x3c]
    93e383: X87 STORE f32 [esp + 0x68] <- ((f32[esp + 0x1c] * ST(st(0))) * f32[esp + 0x40])
    93e38d: X87 STORE f32 [ebx] <- (f32[ebx] + f32[esp + 0x60])
    93e396: X87 STORE f32 [ebx + 4] <- (f32[ebx + 4] + f32[esp + 0x64])
    93e3a0: X87 STORE f32 [ebx + 8] <- (f32[esp + 0x68] + f32[ebx + 8])
    93e3a3: CALL 0x4a9a60 sub_4a9a60  callee-reads[]  regs[]  stack(first arg first)[ebx, eax]  callee-ret-imm=0
    93e3b0: READ i32[0xec3abc]   ; mov eax, dword ptr [0xec3abc]
    93e3bf: caller cleans 8 bytes
    93e3c7: WRITE i32[0xec3abc] mov = eax   ; dword ptr [0xec3abc], eax
    93e3cc: if (eax < (signed) 0x800) goto B50  else B49   ; jl 0x93e3d4

  B49 @0x93e3ce:
    93e3ce: WRITE i32[0xec3abc] mov = esi   ; dword ptr [0xec3abc], esi

  B50 @0x93e3d4:
    93e3d7: if ((cl & 1) == 0) goto B52  else B51   ; je 0x93e454

  B51 @0x93e3d9:
    93e3e2: X87 STORE f32 [esp + 0x98] <- f32[ecx + 0xc]
    93e3ee: X87 STORE f32 [esp + 0xa4] <- f32[eax + 4]
    93e3fb: STACK-OBJECT &[esp+0xdc] -> eax
    93e402: X87 STORE f32 [esp + 0xa8] <- f32[eax + 8]
    93e409: STACK-OBJECT &[esp+0x94] -> esi
    93e412: X87 STORE f32 [esp + 0x94] <- 0.0
    93e419: X87 STORE f32 [esp + 0x98] <- 0.0
    93e420: X87 STORE f32 [esp + 0x9c] <- 0.0
    93e42d: X87 STORE f32 [esp] <- f32(3.1415927410125732; bits 0x40490FDB)
    93e431: CALL 0xbdf050 sub_bdf050  callee-reads[]  regs[]  stack(first arg first)[eax, ecx, eax]  callee-ret-imm=0
    93e439: caller cleans 12 bytes
    93e43c: STACK-OBJECT &[esp+0x98] -> ecx
    93e444: STACK-OBJECT &[esp+0xd8] -> edx
    93e44d: CALL 0xbe4c80 sub_be4c80  callee-reads[]  regs[]  stack(first arg first)[eax, edx, ecx]  callee-ret-imm=12

  B52 @0x93e454:
    93e458: if (byte ptr [ebp + 0x14] == 0) goto B61  else B53   ; je 0x93e5df

  B53 @0x93e45e:
    93e45f: STACK-OBJECT &[esp+0x88] -> ecx
    93e467: STACK-OBJECT &[esp+0x88] -> edx
    93e474: CALL 0xa8b9f0 sub_a8b9f0  callee-reads[]  regs[]  stack(first arg first)[edx, ecx, ebx]  callee-ret-imm=0
    93e479: caller cleans 12 bytes
    93e47e: if (al == 0) goto B64  else B54   ; je 0x93e5ee

  B54 @0x93e484:
    93e48d: if (eax == esi) goto B56  else B55   ; je 0x93e4a5

  B55 @0x93e48f:
    93e497: STACK-OBJECT &[esp+0x44] -> esi
    93e49b: X87 STORE f32 [esp] <- f32[esp + 0x84]
    93e49e: CALL 0xc03910 spline_eval_position  callee-reads[eax]  regs[eax<-(?)]  stack(first arg first)[ecx]  callee-ret-imm=4

  B56 @0x93e4a5:
    93e4ab: X87 STORE f32 [esp + 0x18] <- (f32[ebx] - f32[esp + 0x40])
    93e4b5: X87 STORE f32 [esp + 0x1c] <- (f32[esp + 0x18] * f32[esp + 0x18])
    93e4c0: X87 STORE f32 [esp + 0x18] <- (f32[ebx + 4] - f32[esp + 0x44])
    93e4ce: X87 STORE f32 [esp + 0x1c] <- ((f32[esp + 0x18] * f32[esp + 0x18]) + f32[esp + 0x1c])
    93e4d9: X87 STORE f32 [esp + 0x18] <- (f32[ebx + 8] - f32[esp + 0x48])
    93e4e7: X87 STORE f32 [esp + 0x14] <- ((f32[esp + 0x18] * f32[esp + 0x18]) + f32[esp + 0x1c])
    93e4f5: X87 COMPARE f32[esp + 0x14] vs STst(1)   ; fcom st(1)
    93e4fe: if (x87 compare ('f32[esp + 0x14]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B63  else B57   ; jne 0x93e5ec

  B57 @0x93e504:
    93e504: X87 COMPARE ret(?) vs f32(64.0; bits 0x42800000)   ; fcomp dword ptr [0xe5c378]
    93e50f: if (x87 compare ('ret(?)', 'f32(64.0; bits 0x42800000)'): ST0 outcome in {gt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B64  else B58   ; jp 0x93e5ee

  B58 @0x93e515:
    93e517: STACK-OBJECT &[esp+0x84] -> eax
    93e522: CALL 0xa136e0 nav_find_node_near_point  callee-reads[eax]  regs[eax<-(mov eax, ebx)]  stack(first arg first)[esi, eax, esi, esi]  callee-ret-imm=0
    93e529: STACK-OBJECT &[esp+0xa0] -> ecx
    93e532: STACK-OBJECT &[esp+0x60] -> eax
    93e536: CALL 0xa136e0 nav_find_node_near_point  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0x60])]  stack(first arg first)[esi, ecx, esi, esi]  callee-ret-imm=0
    93e53b: caller cleans 32 bytes
    93e542: if (byte ptr [ebp + 0x24] == 0) goto B39  else B59   ; je 0x93e1a5

  B59 @0x93e548:
    93e585: STACK-OBJECT &[esp+0xc0] -> ecx
    93e5a2: STACK-OBJECT &[esp+0xac] -> edx
    93e5cd: CALL 0xa04a30 sub_a04a30  callee-reads[]  regs[]  stack(first arg first)[edx, ecx, 4]  callee-ret-imm=0
    93e5d2: caller cleans 12 bytes
    93e5d7: if (al != 0) goto B39  else B60   ; jne 0x93e1a5

  B60 @0x93e5dd:
    93e5dd: goto B64 @0x93e5ee

  B61 @0x93e5df:
    93e5e4: if (byte ptr [esp + 0x13] != 0) goto B39  else B62   ; jne 0x93e1a5

  B62 @0x93e5ea:
    93e5ea: goto B64 @0x93e5ee

  B63 @0x93e5ec:

  B64 @0x93e5ee:
    93e607: X87 STORE f32 [esp + eax*4 + 0xe0] <- f32[esp + edi*4 + 0xdc]
    93e619: if (result == 0 of `sub edi, 1`) goto B1  else B65   ; je 0x93de7d

  B65 @0x93e61f:
    93e623: X87 STORE f32 [esp + 0x18] <- 0.0
    93e627: if (edi <= 0 (signed)) goto B68  else B66   ; jle 0x93e6a0

  B66 @0x93e629:
    93e629: STACK-OBJECT &[esp+0x0] -> esp

  B67 @0x93e630:
    93e641: X87 STORE f32 [esp + 0x50] <- (f32[ecx] - f32[eax])
    93e64b: X87 STORE f32 [esp + 0x54] <- (f32[ecx + 4] - f32[eax + 4])
    93e655: X87 STORE f32 [esp + 0x58] <- (f32[ecx + 8] - f32[eax + 8])
    93e66f: X87 STORE f32 [esp + 0x18] <- (((f32[esp + 0x54] * f32[esp + 0x54]) + (f32[esp + 0x50] * f32[esp + 0x50])) + (f32[esp + 0x58] * f32[esp + 0x58]))
    93e677: X87 STORE f32 [esp] <- f32[esp + 0x18]
    93e67a: CALL 0x40ea20 sqrtf_wrapper  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    93e67f: X87 STORE f32 [esp + 0x18] <- ret(sqrtf_wrapper)
    93e68a: caller cleans 4 bytes
    93e68f: X87 STORE f32 [esp + esi*4 + 0xdc] <- f32[esp + 0x18]
    93e69a: X87 STORE f32 [esp + 0x18] <- (f32[esp + 0x18] + f32[esp + 0x18])
    93e69e: if (esi < (signed) edi) goto B67  else B68   ; jl 0x93e630

  B68 @0x93e6a0:
    93e6a9: if (edi < (signed) 4) goto B71  else B69   ; jl 0x93e6f5

  B69 @0x93e6ab:

  B70 @0x93e6ae:
    93e6ba: X87 fdiv st(1)  (st(i) dest, untracked)
    93e6bc: X87 STORE f32 [esp + eax*4 + 0xd0] <- f32[esp + eax*4 + 0xe0]
    93e6ca: X87 fdiv st(1)  (st(i) dest, untracked)
    93e6cc: X87 STORE f32 [esp + eax*4 + 0xd4] <- f32[esp + eax*4 + 0xd4]
    93e6da: X87 fdiv st(1)  (st(i) dest, untracked)
    93e6dc: X87 STORE f32 [esp + eax*4 + 0xd8] <- f32[esp + eax*4 + 0xd8]
    93e6ea: X87 fdiv st(1)  (st(i) dest, untracked)
    93e6ec: X87 STORE f32 [esp + eax*4 + 0xdc] <- f32[esp + eax*4 + 0xdc]
    93e6f3: if (eax < (signed) ecx) goto B70  else B71   ; jl 0x93e6ae

  B71 @0x93e6f5:
    93e6f7: if (eax >= (signed) edi) goto B35  else B72   ; jge 0x93e170

  B72 @0x93e6fd:
    93e70b: X87 fdiv st(1)  (st(i) dest, untracked)
    93e70d: X87 STORE f32 [esp + eax*4 + 0xdc] <- f32[esp + eax*4 + 0xe0]
    93e714: if (eax < (signed) dword ptr [esp + 0x20]) goto B72  else B73   ; jl 0x93e6fd

  B73 @0x93e716:
    93e716: goto B35 @0x93e170

  --- summary ---
  absolute globals read: 0xec3abc, 0x2529e20
  absolute globals written: 0xec3abc
  direct callees: 0xbed5f0 ?, 0xbde980 vec3_distance, 0x4912d0 ?, 0xda0140 _ftol2_sse, 0x4a9a60 ?, 0x40ea20 sqrtf_wrapper, 0x937f10 ?, 0xbe49a0 ?, 0xbde340 vec3_normalize_safe_len, 0xbde290 ?, 0x40b820 cosf_wrapper, 0x428cd0 sinf_wrapper, 0xbdf050 ?, 0xbe4c80 ?, 0xa8b9f0 ?, 0xc03910 spline_eval_position, 0xa136e0 nav_find_node_near_point, 0xa04a30 ?
```

### traffic_spawn_update - 0x93C690

**Role.** Traffic car spawning (`traffic_spawn_update`): timers `0xEB7A80 / 0xEB7A84`, despawn pass (`traffic_despawn_update`), budget (`ambient_vehicle_budget_remaining`, `entity_budget_both_kinds_have_room`), spawn point search (`spawn_point_find`), spline attempts (`traffic_spawn_attempt_at_spline_param`) and `traffic_vehicle_spawn_try`; strings num_cars / time are parsed elsewhere.

**Convention.** Entry registers read (linear scan): none. True size 0x53A, 385 instructions, 82 blocks, 0 jump table(s). Callers: 0x93D1EC.

**Sections, in order of the calls (address and callee):** 0x93C6FD ambient_spawn_timer_rearm; 0x93C702 traffic_despawn_update; 0x93C707 sub_93C3F0; 0x93C70C traffic_records_recount_valid; 0x93C711 current_spawn_group_vehicle_index; 0x93C716 sub_AB5280; 0x93C72A traffic_spawn_step; 0x93C745 entity_budget_both_kinds_have_room; 0x93C780 sub_68A4C0; 0x93C793 spawn_budget_class; 0x93C79A sub_68A500; 0x93C873 spawn_budget_class; 0x93C87C spawn_budget_class; 0x93C8DB sub_937F80; 0x93C90C sub_828600; 0x93C923 sub_939310; 0x93C92B ambient_vehicle_budget_remaining; 0x93C950 sub_B2E790; 0x93C961 timer_elapsed; 0x93C97D timer_set_random; 0x93C98F sub_BDE880; 0x93C9FB spawn_point_find; 0x93CA0E timer_elapsed; 0x93CA3C rng_float01; 0x93CA53 rand_int_range; 0x93CA62 traffic_spawn_attempt_at_spline_param; 0x93CA90 spawn_point_find; 0x93CA9C traffic_spawn_gate_open; 0x93CABD sub_93AEA0; 0x93CB10 rand_int_range; 0x93CB29 traffic_spawn_attempt_at_spline_param; 0x93CB76 traffic_vehicle_spawn_try.

**Hazards.** Uses `rng_float01` and `rand_int_range`: draw order matters; polarity of the 0x2528B48/49 single-player test appears many times (`== 1` and `== 0`); writes `0x212A679` and `0x25295FF`.

**Globals read:** 0xE843AC, 0xEB7A80, 0x215D7A8, 0x2225B64, 0x2528B48, 0x2528B49, 0x2528CF0, 0x2529623, 0x2529634, 0x252963C, 0x27B3C60.

**Globals written:** 0xEB7A84, 0x212A679, 0x25295FF.

```
FUNCTION 0x0093C690  true size 0x53A (1338 bytes)  insns 385  blocks 82
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x484

  B0 @0x93c690:
    93c69c: STACK-OBJECT &[esp+0x0] -> esp

  B1 @0x93c6a0:
    93c6a7: if (byte ptr [eax + 0x215da1c] == 0) goto B81  else B2   ; je 0x93cbbf

  B2 @0x93c6ad:
    93c6b3: if (eax < (unsigned) 6) goto B1  else B3   ; jb 0x93c6a0

  B3 @0x93c6b5:
    93c6b5: READ i32[0xeb7a80]   ; mov eax, dword ptr [0xeb7a80]
    93c6bc: if (eax < 0 (signed)) goto B81  else B4   ; jl 0x93cbbf

  B4 @0x93c6c2:
    93c6c2: READ i32[0xe843ac]   ; mov ecx, dword ptr [0xe843ac]
    93c6ca: if (eax <= (signed) ecx) goto B7  else B5   ; jle 0x93c6db

  B5 @0x93c6cc:
    93c6d3: if (eax < (signed) 0x35a4e900) goto B81  else B6   ; jl 0x93cbbf

  B6 @0x93c6d9:
    93c6d9: goto B8 @0x93c6e9

  B7 @0x93c6db:
    93c6e3: if (ecx >= (signed) 0x35a4e900) goto B81  else B8   ; jge 0x93cbbf

  B8 @0x93c6e9:
    93c6e9: READ i32[0x2225b64]   ; cmp dword ptr [0x2225b64], 0
    93c6f0: if (dword ptr [0x2225b64] == 0) goto B81  else B9   ; je 0x93cbbf

  B9 @0x93c6f6:
    93c6f6: WRITE u8[0x25295ff] mov = 0x1   ; byte ptr [0x25295ff], 1
    93c6fd: CALL 0x93c610 ambient_spawn_timer_rearm  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c702: CALL 0x93be50 traffic_despawn_update  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c707: CALL 0x93c3f0 sub_93c3f0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c70c: CALL 0x93c360 traffic_records_recount_valid  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c711: CALL 0x9392a0 current_spawn_group_vehicle_index  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c716: CALL 0xab5280 sub_ab5280  callee-reads[eax]  regs[eax<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    93c71d: if (al == 1) goto B81  else B10   ; je 0x93cbbf

  B10 @0x93c723:
    93c723: WRITE u8[0x212a679] mov = 0x0   ; byte ptr [0x212a679], 0
    93c72a: CALL 0x93c210 traffic_spawn_step  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c731: if (al == 1) goto B81  else B11   ; je 0x93cbbf

  B11 @0x93c737:
    93c73e: WRITE u8[0x212a679] mov = 0x1   ; byte ptr [0x212a679], 1
    93c745: CALL 0x9392e0 entity_budget_both_kinds_have_room  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c74c: if (al == 0) goto B81  else B12   ; je 0x93cbbf

  B12 @0x93c752:
    93c752: READ u8[0x2528b48]   ; mov al, byte ptr [0x2528b48]
    93c759: if (al != bl) goto B15  else B13   ; jne 0x93c771

  B13 @0x93c75b:
    93c75b: READ u8[0x2528b49]   ; mov cl, byte ptr [0x2528b49]
    93c763: if (cl != 0) goto B15  else B14   ; jne 0x93c771

  B14 @0x93c765:
    93c765: READ u16[0x2528cf0]   ; movzx esi, word ptr [0x2528cf0]
    93c76c: goto B42 @0x93c8c7

  B15 @0x93c771:
    93c771: READ i32[0x2529634]   ; mov eax, dword ptr [0x2529634]
    93c77a: if (eax == 0) goto B42  else B16   ; je 0x93c8c7

  B16 @0x93c780:
    93c780: CALL 0x68a4c0 sub_68a4c0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c785: READ i32[0x252963c]   ; mov ecx, dword ptr [0x252963c]
    93c78d: if (ecx != 0) goto B33  else B17   ; jne 0x93c86f

  B17 @0x93c793:
    93c793: CALL 0x93d2b0 spawn_budget_class  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c79a: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c79f: X87 STORE f32 [esp + 0x20] <- ret(sub_68a500)
    93c7a3: READ u8[0x2529623]   ; cmp byte ptr [0x2529623], 0
    93c7aa: if (byte ptr [0x2529623] == 0) goto B19  else B18   ; je 0x93c7b2

  B18 @0x93c7ac:
    93c7ae: X87 STORE f32 [esp + 0x20] <- 1.0

  B19 @0x93c7b2:
    93c7c6: X87 STORE f32 [esp + 0x20] <- (f32[esp + 0x20] + (((1.0 - ST(st(0))) * f32[esi + 0x20]) * f32[esi + 0x1c]))
    93c7d4: X87 COMPARE f32[esp + 0x20] vs STst(1)   ; fcom st(1)
    93c7db: if (x87 compare ('f32[esp + 0x20]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B21  else B20   ; jne 0x93c7e1

  B20 @0x93c7dd:
    93c7df: goto B22 @0x93c7e3

  B21 @0x93c7e1:

  B22 @0x93c7e3:
    93c7e3: READ i32[0x27b3c60]   ; mov eax, dword ptr [0x27b3c60]
    93c7e8: X87 STORE f32 [esp + 0x18] <- ret(?)
    93c7ee: if (eax == 0) goto B30  else B23   ; je 0x93c835

  B23 @0x93c7f0:
    93c7f3: if (dword ptr [eax + 0x3c] != ebx) goto B26  else B24   ; jne 0x93c80e

  B24 @0x93c7f5:
    93c802: CALL indirect eax  regs[ecx<-(mov ecx, eax) eax<-(mov eax, dword ptr [edx + 4])]  stack(first arg first)[]
    93c807: if (eax == 0xd) goto B29  else B25   ; je 0x93c82b

  B25 @0x93c809:
    93c809: READ i32[0x27b3c60]   ; mov eax, dword ptr [0x27b3c60]

  B26 @0x93c80e:
    93c810: if (eax == 0) goto B30  else B27   ; je 0x93c835

  B27 @0x93c812:
    93c815: if (dword ptr [eax + 0x3c] != ebx) goto B30  else B28   ; jne 0x93c835

  B28 @0x93c817:
    93c824: CALL indirect eax  regs[ecx<-(mov ecx, eax) eax<-(mov eax, dword ptr [edx + 4])]  stack(first arg first)[]
    93c829: if (eax != 4) goto B30  else B29   ; jne 0x93c835

  B29 @0x93c82b:
    93c831: X87 STORE f32 [esp + 0x18] <- f32[0xe888ac]

  B30 @0x93c835:
    93c83f: if (edi >= 0 (signed)) goto B32  else B31   ; jge 0x93c847

  B31 @0x93c841:

  B32 @0x93c847:
    93c861: X87 STORE int64 [esp + 0x20] <- round((ret(?) * f32[esp + 0x18]))
    93c86d: goto B39 @0x93c8bf

  B33 @0x93c86f:
    93c873: CALL 0x93d2b0 spawn_budget_class  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c87a: if (eax >= (unsigned) esi) goto B35  else B34   ; jae 0x93c883

  B34 @0x93c87c:
    93c87c: CALL 0x93d2b0 spawn_budget_class  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c881: goto B36 @0x93c885

  B35 @0x93c883:

  B36 @0x93c885:
    93c88f: if (eax >= 0 (signed)) goto B38  else B37   ; jge 0x93c897

  B37 @0x93c891:

  B38 @0x93c897:
    93c8b3: X87 STORE int64 [esp + 0x20] <- round((ret(?) * f32[0xe888ac]))

  B39 @0x93c8bf:
    93c8c1: if (eax > (unsigned) ebx) goto B41  else B40   ; ja 0x93c8c5

  B40 @0x93c8c3:

  B41 @0x93c8c5:

  B42 @0x93c8c7:
    93c8c9: READ i32[0x215d7a8]   ; sub eax, dword ptr [0x215d7a8]
    93c8d1: if (eax >= (signed) ebx) goto B44  else B43   ; jge 0x93c8db

  B43 @0x93c8d3:
    93c8d5: if (eax <= 0 (signed)) goto B81  else B44   ; jle 0x93cbbf

  B44 @0x93c8db:
    93c8db: CALL 0x937f80 sub_937f80  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c8e2: if (eax <= 0 (signed)) goto B81  else B45   ; jle 0x93cbbf

  B45 @0x93c8e8:
    93c8e8: READ u8[0x2528b48]   ; mov cl, byte ptr [0x2528b48]
    93c8f1: if (cl != 1) goto B48  else B46   ; jne 0x93c916

  B46 @0x93c8f3:
    93c8f3: READ u8[0x2528b49]   ; mov dl, byte ptr [0x2528b49]
    93c8fb: if (dl != 0) goto B48  else B47   ; jne 0x93c916

  B47 @0x93c8fd:
    93c8fd: STACK-OBJECT &[esp+0x54] -> eax
    93c902: STACK-OBJECT &[esp+0x4c] -> ecx
    93c907: STACK-OBJECT &[esp+0x44] -> edx
    93c90c: CALL 0x828600 sub_828600  callee-reads[]  regs[]  stack(first arg first)[edx, ecx, eax]  callee-ret-imm=None
    93c911: caller cleans 12 bytes
    93c914: goto B49 @0x93c92b

  B48 @0x93c916:
    93c916: STACK-OBJECT &[esp+0x3c] -> eax
    93c91b: STACK-OBJECT &[esp+0x58] -> eax
    93c91f: STACK-OBJECT &[esp+0x4c] -> ecx
    93c923: CALL 0x939310 sub_939310  callee-reads[eax, ecx]  regs[eax<-(lea eax, [esp + 0x58]), ecx<-(lea ecx, [esp + 0x4c])]  stack(first arg first)[eax]  callee-ret-imm=0
    93c928: caller cleans 4 bytes

  B49 @0x93c92b:
    93c92b: CALL 0x93c1d0 ambient_vehicle_budget_remaining  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c930: READ i32[0x215d7a8]   ; cmp dword ptr [0x215d7a8], esi
    93c936: if (dword ptr [0x215d7a8] >= (signed) esi) goto B81  else B50   ; jge 0x93cbbf

  B50 @0x93c93c:
    93c93e: if (eax <= 0 (signed)) goto B81  else B51   ; jle 0x93cbbf

  B51 @0x93c944:
    93c94c: STACK-OBJECT &[esp+0x4c] -> edi
    93c950: CALL 0xb2e790 sub_b2e790  callee-reads[]  regs[]  stack(first arg first)[4, 0, 0, 0]  callee-ret-imm=0
    93c955: caller cleans 16 bytes
    93c95a: if (al == 0) goto B54  else B52   ; je 0x93c982

  B52 @0x93c95c:
    93c961: CALL 0xbdd230 timer_elapsed  callee-reads[eax]  regs[eax<-(mov eax, 0xeb7a88)]  stack(first arg first)[]  callee-ret-imm=0
    93c968: if (al == 0) goto B81  else B53   ; je 0x93cbbf

  B53 @0x93c96e:
    93c97d: CALL 0xbdd1b0 timer_set_random  callee-reads[eax]  regs[eax<-(mov eax, 0x1770)]  stack(first arg first)[0xfa0]  callee-ret-imm=4

  B54 @0x93c982:
    93c982: STACK-OBJECT &[esp+0x20] -> ecx
    93c987: STACK-OBJECT &[esp+0x4c] -> eax
    93c98b: STACK-OBJECT &[esp+0x58] -> ecx
    93c98f: CALL 0xbde880 sub_bde880  callee-reads[eax, ecx]  regs[eax<-(lea eax, [esp + 0x4c]), ecx<-(lea ecx, [esp + 0x58])]  stack(first arg first)[ecx]  callee-ret-imm=None
    93c99a: caller cleans 4 bytes
    93c9a2: X87 STORE f32 [esp + 0x1c] <- (ret(sub_bde880) / f32[0xe8437c])
    93c9b0: X87 COMPARE f32[esp + 0x1c] vs STst(1)   ; fcom st(1)
    93c9b9: if (x87 compare ('f32[esp + 0x1c]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B56  else B55   ; jne 0x93c9bf

  B55 @0x93c9bb:

  B56 @0x93c9bf:
    93c9bf: READ u8[0x2528b48]   ; mov dl, byte ptr [0x2528b48]
    93c9c8: if (dl != 1) goto B59  else B57   ; jne 0x93c9d7

  B57 @0x93c9ca:
    93c9ca: READ u8[0x2528b49]   ; mov al, byte ptr [0x2528b49]
    93c9d1: if (al != 0) goto B59  else B58   ; jne 0x93c9d7

  B58 @0x93c9d3:

  B59 @0x93c9d7:
    93c9de: X87 STORE f32 [esp] <- ret(?)
    93c9e1: STACK-OBJECT &[esp+0x54] -> ecx
    93c9e6: STACK-OBJECT &[esp+0x4c] -> edx
    93c9eb: STACK-OBJECT &[esp+0x2a4] -> eax
    93c9fb: CALL 0x93b380 spawn_point_find  callee-reads[]  regs[]  stack(first arg first)[eax, edx, ecx, ecx, edi, ebx]  callee-ret-imm=0
    93ca02: caller cleans 24 bytes
    93ca07: if (esi <= 0 (signed)) goto B66  else B60   ; jle 0x93ca73

  B60 @0x93ca09:
    93ca0e: CALL 0xbdd230 timer_elapsed  callee-reads[eax]  regs[eax<-(mov eax, 0xeb7a84)]  stack(first arg first)[]  callee-ret-imm=0
    93ca15: if (al != 1) goto B66  else B61   ; jne 0x93ca73

  B61 @0x93ca17:
    93ca17: READ i32[0xe843ac]   ; mov eax, dword ptr [0xe843ac]
    93ca26: WRITE i32[0xeb7a84] mov = eax   ; dword ptr [0xeb7a84], eax
    93ca2b: if (eax <= (signed) 0x6b49d200) goto B63  else B62   ; jle 0x93ca37

  B62 @0x93ca2d:
    93ca32: WRITE i32[0xeb7a84] mov = eax   ; dword ptr [0xeb7a84], eax

  B63 @0x93ca37:
    93ca3c: CALL 0xbdce30 rng_float01  callee-reads[eax]  regs[eax<-(mov eax, 0xec30f8)]  stack(first arg first)[]  callee-ret-imm=0
    93ca41: X87 COMPARE ret(rng_float01) vs f32(0.5; bits 0x3F000000)   ; fcomp dword ptr [0xdd1c8c]
    93ca4c: if (x87 compare ('ret(rng_float01)', 'f32(0.5; bits 0x3F000000)'): ST0 outcome in {gt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B66  else B64   ; jp 0x93ca73

  B64 @0x93ca4e:
    93ca53: CALL 0xbdcf50 rand_int_range  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93ca62: CALL 0x93a6f0 traffic_spawn_attempt_at_spline_param  callee-reads[eax]  regs[eax<-(lea eax, [esp + eax + 0x290])]  stack(first arg first)[]  callee-ret-imm=0
    93ca69: if (eax != 0) goto B81  else B65   ; jne 0x93cbbf

  B65 @0x93ca6f:

  B66 @0x93ca73:
    93ca7b: X87 STORE f32 [esp] <- f32[esp + 0x1c]
    93ca7e: STACK-OBJECT &[esp+0x54] -> ecx
    93ca83: STACK-OBJECT &[esp+0x4c] -> edx
    93ca88: STACK-OBJECT &[esp+0xa4] -> eax
    93ca90: CALL 0x93b380 spawn_point_find  callee-reads[]  regs[]  stack(first arg first)[eax, edx, ecx, ecx, edi, 0]  callee-ret-imm=0
    93ca95: caller cleans 24 bytes
    93ca9c: CALL 0x93d4d0 traffic_spawn_gate_open  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93caa3: if (al == 0) goto B68  else B67   ; je 0x93cac7

  B67 @0x93caa5:
    93caab: X87 STORE f32 [esp] <- f32[esp + 0x1c]
    93caae: STACK-OBJECT &[esp+0x50] -> ecx
    93cab3: STACK-OBJECT &[esp+0x48] -> edx
    93cab8: STACK-OBJECT &[esp+0x70] -> eax
    93cabd: CALL 0x93aea0 sub_93aea0  callee-reads[]  regs[]  stack(first arg first)[eax, edx, ecx, ecx, edi]  callee-ret-imm=None
    93cac2: caller cleans 20 bytes

  B68 @0x93cac7:
    93cacc: if (dword ptr [esp + 0x18] >= (unsigned) 3) goto B81  else B69   ; jae 0x93cbbf

  B69 @0x93cad2:
    93cad2: STACK-OBJECT &[ebp-0x1] -> edx
    93cafb: goto B70 @0x93cb00

  B70 @0x93cb00:
    93cb05: if (eax <= 0 (signed)) goto B81  else B71   ; jle 0x93cbbf

  B71 @0x93cb0b:
    93cb10: CALL 0xbdcf50 rand_int_range  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93cb19: if (esi >= (signed) ebp) goto B76  else B72   ; jge 0x93cb67

  B72 @0x93cb1b:
    93cb29: CALL 0x93a6f0 traffic_spawn_attempt_at_spline_param  callee-reads[eax]  regs[eax<-(mov eax, edi)]  stack(first arg first)[]  callee-ret-imm=0
    93cb30: if (eax != 0) goto B81  else B73   ; jne 0x93cbbf

  B73 @0x93cb36:
    93cb3e: if (esi == dword ptr [esp + 0x1c]) goto B75  else B74   ; je 0x93cb56

  B74 @0x93cb40:

  B75 @0x93cb56:
    93cb65: goto B80 @0x93cbab

  B76 @0x93cb67:
    93cb76: CALL 0x939f30 traffic_vehicle_spawn_try  callee-reads[esi]  regs[esi<-(sub esi, ebp)]  stack(first arg first)[edi, edx]  callee-ret-imm=0
    93cb7b: caller cleans 8 bytes
    93cb80: if (eax != 0) goto B81  else B77   ; jne 0x93cbbf

  B77 @0x93cb82:
    93cb8a: if (esi == dword ptr [esp + 0x34]) goto B79  else B78   ; je 0x93cb9c

  B78 @0x93cb8c:

  B79 @0x93cb9c:

  B80 @0x93cbab:
    93cbb9: if (eax < (unsigned) 3) goto B70  else B81   ; jb 0x93cb00

  B81 @0x93cbbf:
    93cbc3: caller cleans 1156 bytes
    93cbc9: RETURN

  --- summary ---
  absolute globals read: 0xe843ac, 0xeb7a80, 0x215d7a8, 0x2225b64, 0x2528b48, 0x2528b49, 0x2528cf0, 0x2529623, 0x2529634, 0x252963c, 0x27b3c60
  absolute globals written: 0xeb7a84, 0x212a679, 0x25295ff
  direct callees: 0x93c610 ambient_spawn_timer_rearm, 0x93be50 traffic_despawn_update, 0x93c3f0 ?, 0x93c360 traffic_records_recount_valid, 0x9392a0 current_spawn_group_vehicle_index, 0xab5280 ?, 0x93c210 traffic_spawn_step, 0x9392e0 entity_budget_both_kinds_have_room, 0x68a4c0 ?, 0x93d2b0 spawn_budget_class, 0x68a500 ?, 0x937f80 ?, 0x828600 ?, 0x939310 ?, 0x93c1d0 ambient_vehicle_budget_remaining, 0xb2e790 ?, 0xbdd230 timer_elapsed, 0xbdd1b0 timer_set_random, 0xbde880 ?, 0x93b380 spawn_point_find, 0xbdce30 rng_float01, 0xbdcf50 rand_int_range, 0x93a6f0 traffic_spawn_attempt_at_spline_param, 0x93d4d0 traffic_spawn_gate_open, 0x93aea0 ?, 0x939f30 traffic_vehicle_spawn_try
```

### spawn_record_create_members - 0x9502B0

**Role.** Spawn record member creation: builds the members of one record (`team_from_name` lookups through 0x946710, human / vehicle creation via 0x9CFCE0, 0xAEFDA0, 0xAEFC40, 0xAB1000), orients them with `vec3_normalize_safe` and registers them.

**Convention.** Entry registers read (linear scan): none. True size 0x52F, 354 instructions, 62 blocks, 0 jump table(s). Callers: 0x95112E.

**Sections, in order of the calls (address and callee):** 0x9503A5 sub_946710; 0x9503E6 sub_9CFCE0; 0x95040F sub_AEFDA0; 0x950432 sub_AEFC40; 0x9504EA sub_AB1000; 0x950542 sub_A214C0; 0x95056F vec3_normalize_safe; 0x950724 sub_9466D0; 0x950734 sub_946710; 0x95073D sub_950280; 0x950773 sub_950040.

**Hazards.** Reads `0x33D630C`; ten callees; stack args are not decoded by the generator for the indirect calls.

**Globals read:** 0x33D630C.

**Globals written:** none.

```
FUNCTION 0x009502B0  true size 0x52F (1327 bytes)  insns 354  blocks 62
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x198

  B0 @0x9502b0:
    9502ce: if (eax < (signed) esi) goto B61  else B1   ; jl 0x9507c9

  B1 @0x9502d4:
    9502d7: if (eax <= (signed) 1) goto B14  else B2   ; jle 0x950457

  B2 @0x9502dd:
    9502e0: if (eax != 2) goto B61  else B3   ; jne 0x9507c9

  B3 @0x9502e6:
    9502ed: if (dword ptr [ebx + 0x70] <= (signed) esi) goto B61  else B4   ; jle 0x9507c9

  B4 @0x9502f3:

  B5 @0x950300:
    950308: if (ecx == esi) goto B8  else B6   ; je 0x950324

  B6 @0x95030a:
    95031d: if (eax == esi) goto B8  else B7   ; je 0x950324

  B7 @0x95031f:
    950322: if (dword ptr [eax + 0x44] == ecx) goto B9  else B8   ; je 0x950328

  B8 @0x950324:
    950326: goto B10 @0x950338

  B9 @0x950328:

  B10 @0x950338:
    95033a: READ i32[0x33d630c]   ; mov edx, dword ptr [0x33d630c]
    950345: X87 STORE f32 [esp + 0x74] <- 0.0
    9503a5: CALL 0x946710 sub_946710  callee-reads[eax]  regs[eax<-(mov eax, 0x11)]  stack(first arg first)[]  callee-ret-imm=0
    9503ca: STACK-OBJECT &[ebp+0x20] -> esi
    9503d2: STACK-OBJECT &[esp+0x84] -> edi
    9503e2: STACK-OBJECT &[esp+0x58] -> eax
    9503e6: CALL 0x9cfce0 sub_9cfce0  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    9503ef: if (esi == 0) goto B12  else B11   ; je 0x950437

  B11 @0x9503f1:
    950408: STACK-OBJECT &[esp+0xc4] -> eax
    95040f: CALL 0xaefda0 sub_aefda0  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0xc4])]  stack(first arg first)[]  callee-ret-imm=0
    950432: CALL 0xaefc40 sub_aefc40  callee-reads[eax, ecx]  regs[eax<-(?), ecx<-(?)]  stack(first arg first)[]  callee-ret-imm=0

  B12 @0x950437:
    95044c: if (eax < (signed) dword ptr [ebx + 0x70]) goto B5  else B13   ; jl 0x950300

  B13 @0x950452:
    950452: goto B61 @0x9507c9

  B14 @0x950457:
    95045e: if (dword ptr [ebx + 0x70] <= (signed) esi) goto B61  else B15   ; jle 0x9507c9

  B15 @0x950464:
    95046b: goto B16 @0x950470

  B16 @0x950470:
    950477: if (dword ptr [ebx + 0x84] >= (signed) 8) goto B61  else B17   ; jge 0x9507c9

  B17 @0x95047d:
    950485: if (eax == esi) goto B60  else B18   ; je 0x9507b0

  B18 @0x95048b:
    9504a2: if (edi == esi) goto B60  else B19   ; je 0x9507b0

  B19 @0x9504a8:
    9504ab: if (dword ptr [edi + 0x44] != eax) goto B60  else B20   ; jne 0x9507b0

  B20 @0x9504b1:
    9504b5: if (dword ptr [edi + 0x4c] != 5) goto B60  else B21   ; jne 0x9507b0

  B21 @0x9504bb:
    9504c2: if ((byte ptr [edi + 0x80] & 0xb) != 0) goto B60  else B22   ; jne 0x9507b0

  B22 @0x9504c8:
    9504cf: if (byte ptr [edi + 0x84d8] == 0) goto B60  else B23   ; je 0x9507b0

  B23 @0x9504d5:
    9504d5: STACK-OBJECT &[esp+0x20] -> eax
    9504da: STACK-OBJECT &[esp+0x58] -> ecx
    9504df: STACK-OBJECT &[esp+0x44] -> edx
    9504e6: STACK-OBJECT &[esp+0x20] -> ecx
    9504ea: CALL 0xab1000 sub_ab1000  callee-reads[eax]  regs[eax<-(mov eax, edi)]  stack(first arg first)[edx, ecx, eax]  callee-ret-imm=0
    9504f3: caller cleans 12 bytes
    950502: if (dword ptr [esp + 0x3c] <= (signed) ebp) goto B50  else B24   ; jle 0x95071c

  B24 @0x950508:
    95050c: STACK-OBJECT &[esp+0xe0] -> eax
    95051b: goto B25 @0x950520

  B25 @0x950520:
    95052f: INT cdq 
    95053d: STACK-OBJECT &[esp+0x64] -> eax
    950542: CALL 0xa214c0 sub_a214c0  callee-reads[]  regs[]  stack(first arg first)[eax, edx, eax, edx]  callee-ret-imm=None
    95054b: caller cleans 16 bytes
    950551: STACK-OBJECT &[esp+0x48] -> esi
    950555: X87 STORE f32 [esp + 0x48] <- (f32[esp + 0x68] - f32[edi + 0x14])
    950560: X87 STORE f32 [esp + 0x4c] <- (f32[esp + 0x5c] - f32[edi + 0x18])
    95056b: X87 STORE f32 [esp + 0x50] <- (f32[esp + 0x60] - f32[edi + 0x1c])
    95056f: CALL 0xbde150 vec3_normalize_safe  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x48])]  stack(first arg first)[]  callee-ret-imm=0
    950594: X87 STORE f32 [esp + 0x14] <- (((f32[eax + 0x30] * f32[esp + 0x48]) + (f32[eax + 0x34] * f32[esp + 0x4c])) + (f32[eax + 0x38] * f32[esp + 0x50]))
    95059e: X87 STORE f32 [esp + 0x14] <- fabs(f32[esp + 0x14])
    9505a6: X87 STORE f32 [esp + 0x14] <- f32[esp + 0x14]
    9505b4: X87 COMPARE f32[esp + 0x14] vs STst(1)   ; fcom st(1)
    9505bd: if (x87 compare ('f32[esp + 0x14]', 'STst(1)'): ST0 outcome in {lt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B48  else B26   ; jnp 0x950704

  B26 @0x9505c3:
    9505c6: if (ebx >= (signed) 2) goto B28  else B27   ; jge 0x9505f4

  B27 @0x9505c8:
    9505cc: X87 STORE f32 [esp + ebx*4 + 0x30] <- ret(?)
    9505da: STACK-OBJECT &[esp+0x58] -> esi
    9505ef: goto B49 @0x950706

  B28 @0x9505f4:
    9505ff: if (edx < (signed) 4) goto B40  else B29   ; jl 0x9506a4

  B29 @0x950605:
    950605: STACK-OBJECT &[esp+0x38] -> esi

  B30 @0x950624:
    950637: X87 COMPARE f32[esp + edx + 0x30] vs ST(1)   ; fcompp 
    95063e: if (x87 compare ('f32[esp + edx + 0x30]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B32  else B31   ; jne 0x950644

  B31 @0x950640:

  B32 @0x950644:
    95064a: X87 COMPARE f32[esp + edx + 0x30] vs ST(1)   ; fcompp 
    950651: if (x87 compare ('f32[esp + edx + 0x30]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B34  else B33   ; jne 0x950658

  B33 @0x950653:

  B34 @0x950658:
    95065f: X87 COMPARE f32[esp + edx + 0x30] vs ST(1)   ; fcompp 
    950666: if (x87 compare ('f32[esp + edx + 0x30]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B36  else B35   ; jne 0x950672

  B35 @0x950668:

  B36 @0x950672:
    950679: X87 COMPARE f32[esp + edx + 0x30] vs ST(1)   ; fcompp 
    950680: if (x87 compare ('f32[esp + edx + 0x30]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B38  else B37   ; jne 0x95068b

  B37 @0x950682:

  B38 @0x95068b:
    95069a: if (ecx < (signed) eax) goto B30  else B39   ; jl 0x950624

  B39 @0x95069c:

  B40 @0x9506a4:
    9506a6: if (ecx >= (signed) ebx) goto B45  else B41   ; jge 0x9506d2

  B41 @0x9506a8:

  B42 @0x9506af:
    9506be: X87 COMPARE f32[esp + esi + 0x30] vs ST(1)   ; fcompp 
    9506c5: if (x87 compare ('f32[esp + esi + 0x30]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B44  else B43   ; jne 0x9506cb

  B43 @0x9506c7:

  B44 @0x9506cb:
    9506d0: if (ecx < (signed) ebx) goto B42  else B45   ; jl 0x9506af

  B45 @0x9506d2:
    9506da: X87 COMPARE f32[esp + edi*4 + 0x30] vs STst(1)   ; fcomp st(1)
    9506e1: if (x87 compare ('f32[esp + edi*4 + 0x30]', 'STst(1)'): ST0 outcome in {gt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B47  else B46   ; jp 0x950700

  B46 @0x9506e3:
    9506e6: X87 STORE f32 [edx] <- ret(?)
    9506f4: STACK-OBJECT &[esp+0x58] -> esi
    9506fe: goto B49 @0x950706

  B47 @0x950700:

  B48 @0x950704:

  B49 @0x950706:
    950716: if (ebp < (signed) dword ptr [esp + 0x3c]) goto B25  else B50   ; jl 0x950520

  B50 @0x95071c:
    950721: STACK-OBJECT &[ebp+0x7] -> eax
    950724: CALL 0x9466d0 sub_9466d0  callee-reads[eax]  regs[eax<-(lea eax, [ebp + 7])]  stack(first arg first)[]  callee-ret-imm=0
    95072f: if (dword ptr [edi + 0x84e4] != eax) goto B52  else B51   ; jne 0x95073d

  B51 @0x950731:
    950731: STACK-OBJECT &[ebp+0x10] -> eax
    950734: CALL 0x946710 sub_946710  callee-reads[eax]  regs[eax<-(lea eax, [ebp + 0x10])]  stack(first arg first)[]  callee-ret-imm=0
    95073b: goto B53 @0x95074a

  B52 @0x95073d:
    95073d: CALL 0x950280 sub_950280  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0

  B53 @0x95074a:
    95074e: if (ebp <= 0 (signed)) goto B59  else B54   ; jle 0x9507a7

  B54 @0x950750:
    950750: STACK-OBJECT &[esp+0xe0] -> esi
    950757: goto B55 @0x950760

  B55 @0x950760:
    95076e: if (dword ptr [eax + 0x84] >= (signed) 8) goto B59  else B56   ; jge 0x9507a7

  B56 @0x950770:
    950773: CALL 0x950040 sub_950040  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[ebx]  callee-ret-imm=0
    950778: caller cleans 4 bytes
    95077d: if (eax == 0) goto B58  else B57   ; je 0x95079d

  B57 @0x95077f:

  B58 @0x95079d:
    9507a5: if (edi < (signed) ebp) goto B55  else B59   ; jl 0x950760

  B59 @0x9507a7:

  B60 @0x9507b0:
    9507c3: if (eax < (signed) dword ptr [ebx + 0x70]) goto B16  else B61   ; jl 0x950470

  B61 @0x9507c9:
    9507d8: caller cleans 408 bytes
    9507de: RETURN

  --- summary ---
  absolute globals read: 0x33d630c
  absolute globals written: 
  direct callees: 0x946710 ?, 0x9cfce0 ?, 0xaefda0 ?, 0xaefc40 ?, 0xab1000 ?, 0xa214c0 ?, 0xbde150 vec3_normalize_safe, 0x9466d0 ?, 0x950280 ?, 0x950040 ?
```

### traffic_spawn_attempt - 0x93A2E0

**Role.** Traffic spawn attempt: asks the current group (`0x215E5BC`) for its vehicle info (vtable slot 5), picks a position (0x939450 / 0x939670 / 0x9396C0), checks the entity budget, creates and activates a traffic vehicle (`0x943DE0`, `0x951C20`).

**Convention.** Entry registers read (linear scan): eax. True size 0x405, 305 instructions, 55 blocks, 0 jump table(s). Callers: 0x93A71C, 0x93A886, 0x93C341, 0x93D8DD.

**Sections, in order of the calls (address and callee):** 0x93A2ED sub_951C20; 0x93A317 current_spawn_group_vehicle_index; 0x93A32E sub_AE2960; 0x93A36B sub_939450; 0x93A406 sub_939670; 0x93A446 sub_943DE0; 0x93A48B sub_9396C0; 0x93A49B sub_939AC0; 0x93A4C9 sub_9397B0; 0x93A4EB sub_93E810; 0x93A4F7 sub_93E850; 0x93A529 sub_555920; 0x93A547 sub_AE2BB0; 0x93A553 spawn_carjack_vehicle; 0x93A55E activity_current_type; 0x93A580 traffic_record_activate; 0x93A588 vehicle_from_handle; 0x93A5AC sub_AC8D50; 0x93A5C0 sub_AC8DA0; 0x93A5CA sub_AB6C70; 0x93A5EB sub_957580; 0x93A601 sub_939B70; 0x93A618 traffic_vehicle_create_and_activate; 0x93A64A sub_D227E0; 0x93A65D sub_AABC80; 0x93A684 sub_939E20; 0x93A692 sub_AE11F0; 0x93A69E rng_float01; 0x93A6B2 activity_current_type; 0x93A6C5 vehicle_ai_set_force_flag_crazy_rails; 0x93A6CC sub_687E30; 0x93A6D3 sub_687EB0.

**Hazards.** Entry EAX is read (the request); indirect `call edx` / `call eax` sites are vtable calls of the group; 0xE8 bytes of locals.

**Globals read:** 0x212A679, 0x215D7AC, 0x215E5BC, 0x252A0D8.

**Globals written:** none.

```
FUNCTION 0x0093A2E0  true size 0x405 (1029 bytes)  insns 305  blocks 55
  entry regs read before write (linear prefix scan): ['eax']
  stack frame: esp, 0xe8

  B0 @0x93a2e0:
    93a2ed: CALL 0x951c20 sub_951c20  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93a2f2: caller cleans 4 bytes
    93a2f7: if (al == 0) goto B2  else B1   ; je 0x93a306

  B1 @0x93a2f9:
    93a2f9: READ i32[0x215e5bc]   ; cmp dword ptr [0x215e5bc], 0
    93a300: if (dword ptr [0x215e5bc] == 0) goto B34  else B2   ; je 0x93a569

  B2 @0x93a306:
    93a30b: READ u8[0x212a679]   ; cmp byte ptr [0x212a679], bl
    93a315: if (byte ptr [0x212a679] != bl) goto B4  else B3   ; jne 0x93a32a

  B3 @0x93a317:
    93a317: CALL 0x9392a0 current_spawn_group_vehicle_index  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0

  B4 @0x93a32a:
    93a32a: STACK-OBJECT &[esp+0x68] -> eax
    93a32e: CALL 0xae2960 sub_ae2960  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0x68])]  stack(first arg first)[]  callee-ret-imm=0
    93a341: X87 STORE f32 [esp + 0xb4] <- f32[esp + 0xfc]
    93a34c: STACK-OBJECT &[esp+0x68] -> edx
    93a36b: CALL 0x939450 sub_939450  callee-reads[]  regs[]  stack(first arg first)[edx]  callee-ret-imm=0
    93a370: caller cleans 4 bytes
    93a375: if (al == 0) goto B34  else B5   ; je 0x93a569

  B5 @0x93a37b:
    93a37d: READ i32[0x215e5bc]   ; mov esi, dword ptr [0x215e5bc]
    93a383: if (bl == 0) goto B9  else B6   ; je 0x93a3a2

  B6 @0x93a385:
    93a389: if (esi != eax) goto B8  else B7   ; jne 0x93a38f

  B7 @0x93a38b:
    93a38d: goto B11 @0x93a3a6

  B8 @0x93a38f:
    93a396: CALL indirect edx  regs[ecx<-(mov ecx, esi) eax<-(mov eax, dword ptr [esi])]  stack(first arg first)[]
    93a398: READ i32[0x215e5bc]   ; mov esi, dword ptr [0x215e5bc]
    93a3a0: goto B10 @0x93a3a4

  B9 @0x93a3a2:

  B10 @0x93a3a4:

  B11 @0x93a3a6:
    93a3b6: X87 STORE f32 [esp + 0x58] <- 0.0
    93a3be: X87 STORE f32 [esp + 0x5c] <- 0.0
    93a406: CALL 0x939670 sub_939670  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=0
    93a40b: caller cleans 4 bytes
    93a414: if (ebx == 0) goto B13  else B12   ; je 0x93a41c

  B12 @0x93a416:
    93a41a: goto B19 @0x93a47a

  B13 @0x93a41c:
    93a421: if (byte ptr [esp + 0x14] == 0) goto B17  else B14   ; je 0x93a442

  B14 @0x93a423:
    93a429: if (esi != 0) goto B16  else B15   ; jne 0x93a433

  B15 @0x93a42b:
    93a431: goto B19 @0x93a47a

  B16 @0x93a433:
    93a43a: CALL indirect eax  regs[ecx<-(mov ecx, esi) eax<-(mov eax, dword ptr [edx + 0x54])]  stack(first arg first)[]
    93a440: goto B19 @0x93a47a

  B17 @0x93a442:
    93a442: STACK-OBJECT &[esp+0x20] -> esi
    93a446: CALL 0x943de0 sub_943de0  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x20])]  stack(first arg first)[]  callee-ret-imm=0
    93a44d: if (al == 0) goto B34  else B18   ; je 0x93a569

  B18 @0x93a453:
    93a462: INT imul ecx

  B19 @0x93a47a:
    93a484: if (ebx != 0) goto B26  else B20   ; jne 0x93a503

  B20 @0x93a486:
    93a486: STACK-OBJECT &[esp+0x68] -> eax
    93a48b: CALL 0x9396c0 sub_9396c0  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=0
    93a490: caller cleans 4 bytes
    93a493: STACK-OBJECT &[esp+0x68] -> edi
    93a49b: CALL 0x939ac0 sub_939ac0  callee-reads[edi]  regs[edi<-(lea edi, [esp + 0x68])]  stack(first arg first)[]  callee-ret-imm=0
    93a4a6: if (eax < 0 (signed)) goto B25  else B21   ; jl 0x93a4dd

  B21 @0x93a4a8:
    93a4a8: READ i32[0x252a0d8]   ; cmp eax, dword ptr [0x252a0d8]
    93a4ae: if (eax >= (signed) dword ptr [0x252a0d8]) goto B25  else B22   ; jge 0x93a4dd

  B22 @0x93a4b0:
    93a4c4: if (cl == 0) goto B25  else B23   ; je 0x93a4dd

  B23 @0x93a4c6:
    93a4c9: CALL 0x9397b0 sub_9397b0  callee-reads[]  regs[]  stack(first arg first)[edx]  callee-ret-imm=0
    93a4ce: caller cleans 4 bytes
    93a4d3: if (al == 0) goto B34  else B24   ; je 0x93a569

  B24 @0x93a4d9:

  B25 @0x93a4dd:
    93a4dd: STACK-OBJECT &[esp+0xe0] -> ecx
    93a4e4: STACK-OBJECT &[esp+0xec] -> esi
    93a4eb: CALL 0x93e810 sub_93e810  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93a4f3: STACK-OBJECT &[esp+0x6c] -> edx
    93a4f7: CALL 0x93e850 sub_93e850  callee-reads[edx]  regs[edx<-(lea edx, [esp + 0x6c])]  stack(first arg first)[eax]  callee-ret-imm=0
    93a4fc: caller cleans 4 bytes
    93a501: if (al == 1) goto B34  else B26   ; je 0x93a569

  B26 @0x93a503:
    93a503: READ i32[0x215d7ac]   ; mov edi, dword ptr [0x215d7ac]
    93a50b: if (edi == 0) goto B34  else B27   ; je 0x93a569

  B27 @0x93a50d:
    93a50f: if (ebx == 0) goto B29  else B28   ; je 0x93a539

  B28 @0x93a511:
    93a519: STACK-OBJECT &[esp+0x8c] -> ecx
    93a522: STACK-OBJECT &[esp+0xa0] -> eax
    93a529: CALL 0x555920 sub_555920  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0xa0])]  stack(first arg first)[ebx, ecx, 1, 0, 1, 1]  callee-ret-imm=0
    93a531: caller cleans 24 bytes
    93a537: goto B33 @0x93a55e

  B29 @0x93a539:
    93a541: if (byte ptr [esp + 0xc0] == 0) goto B31  else B30   ; je 0x93a54e

  B30 @0x93a543:
    93a543: STACK-OBJECT &[esp+0x68] -> esi
    93a547: CALL 0xae2bb0 sub_ae2bb0  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x68])]  stack(first arg first)[]  callee-ret-imm=0
    93a54c: goto B32 @0x93a55b

  B31 @0x93a54e:
    93a54e: STACK-OBJECT &[esp+0x68] -> eax
    93a553: CALL 0xae2c80 spawn_carjack_vehicle  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=None
    93a558: caller cleans 4 bytes

  B32 @0x93a55b:

  B33 @0x93a55e:
    93a55e: CALL 0x53c4d0 activity_current_type  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93a567: if (dword ptr [edi + 8] != 0) goto B35  else B34   ; jne 0x93a576

  B34 @0x93a569:
    93a56f: caller cleans 232 bytes
    93a575: RETURN

  B35 @0x93a576:
    93a578: if (ebx != 0) goto B37  else B36   ; jne 0x93a585

  B36 @0x93a57a:
    93a580: CALL 0x938c30 traffic_record_activate  callee-reads[eax, ecx]  regs[eax<-(mov eax, edi), ecx<-(mov cl, byte ptr [esp + 0x14])]  stack(first arg first)[]  callee-ret-imm=0

  B37 @0x93a585:
    93a588: CALL 0xae28f0 vehicle_from_handle  callee-reads[ecx]  regs[ecx<-(mov ecx, dword ptr [edi + 8])]  stack(first arg first)[]  callee-ret-imm=0
    93a5ac: CALL 0xac8d50 sub_ac8d50  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [esp + 0x28])]  stack(first arg first)[]  callee-ret-imm=0
    93a5b3: if (al == 0) goto B39  else B38   ; je 0x93a5c8

  B38 @0x93a5b5:
    93a5c0: CALL 0xac8da0 sub_ac8da0  callee-reads[ecx]  regs[ecx<-(mov ecx, dword ptr [ebp + 0x7f9c])]  stack(first arg first)[edx]  callee-ret-imm=None
    93a5c5: caller cleans 4 bytes

  B39 @0x93a5c8:
    93a5ca: CALL 0xab6c70 sub_ab6c70  callee-reads[esi]  regs[esi<-(?)]  stack(first arg first)[0]  callee-ret-imm=0
    93a5d1: caller cleans 4 bytes
    93a5d4: X87 STORE f32 [esp + 0x18] <- 0.0
    93a5dd: if (byte ptr [esp + 0x14] == 0) goto B41  else B40   ; je 0x93a5f6

  B40 @0x93a5df:
    93a5eb: CALL 0x957580 sub_957580  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=None
    93a5f0: X87 STORE f32 [esp + 0x18] <- ret(sub_957580)
    93a5f4: goto B42 @0x93a609

  B41 @0x93a5f6:
    93a5f6: STACK-OBJECT &[esp+0x68] -> ecx
    93a5fb: STACK-OBJECT &[esp+0x24] -> edx
    93a601: CALL 0x939b70 sub_939b70  callee-reads[]  regs[]  stack(first arg first)[esi, edx, ecx]  callee-ret-imm=0
    93a606: caller cleans 12 bytes

  B42 @0x93a609:
    93a60b: if (ebx != 0) goto B44  else B43   ; jne 0x93a620

  B43 @0x93a60d:
    93a618: CALL 0x939930 traffic_vehicle_create_and_activate  callee-reads[]  regs[]  stack(first arg first)[esi, ecx, eax]  callee-ret-imm=0
    93a61d: caller cleans 12 bytes

  B44 @0x93a620:
    93a628: if (byte ptr [esp + 0x104] != 0) goto B47  else B45   ; jne 0x93a662

  B45 @0x93a62a:
    93a62a: STACK-OBJECT &[esp+0x88] -> edx
    93a632: STACK-OBJECT &[esp+0x80] -> eax
    93a63a: STACK-OBJECT &[esp+0xe8] -> ecx
    93a642: STACK-OBJECT &[esp+0xf8] -> edx
    93a64a: CALL 0xd227e0 sub_d227e0  callee-reads[]  regs[]  stack(first arg first)[edx, ecx, eax, edx]  callee-ret-imm=0
    93a64f: caller cleans 16 bytes
    93a654: if (al != 0) goto B47  else B46   ; jne 0x93a662

  B46 @0x93a656:
    93a65d: CALL 0xaabc80 sub_aabc80  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0

  B47 @0x93a662:
    93a668: if (bl == 0) goto B49  else B48   ; je 0x93a67d

  B48 @0x93a66a:
    93a676: X87 STORE f32 [esp + 0xbc] <- f32[esp + 0x18]

  B49 @0x93a67d:
    93a67d: STACK-OBJECT &[esp+0x68] -> eax
    93a684: CALL 0x939e20 sub_939e20  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=None
    93a689: caller cleans 4 bytes
    93a68e: if (bl != 0) goto B51  else B50   ; jne 0x93a699

  B50 @0x93a690:
    93a692: CALL 0xae11f0 sub_ae11f0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    93a697: goto B54 @0x93a6ca

  B51 @0x93a699:
    93a69e: CALL 0xbdce30 rng_float01  callee-reads[eax]  regs[eax<-(mov eax, 0xec30f8)]  stack(first arg first)[]  callee-ret-imm=0
    93a6a9: X87 COMPARE f32[0x215d7a4] vs ST(1)   ; fcompp 
    93a6b0: if (x87 compare ('f32[0x215d7a4]', 'ST(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B54  else B52   ; jne 0x93a6ca

  B52 @0x93a6b2:
    93a6b2: CALL 0x53c4d0 activity_current_type  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93a6ba: if (eax == 4) goto B54  else B53   ; je 0x93a6ca

  B53 @0x93a6bc:
    93a6c5: CALL 0xb2d020 vehicle_ai_set_force_flag_crazy_rails  callee-reads[]  regs[]  stack(first arg first)[ecx, 1]  callee-ret-imm=None

  B54 @0x93a6ca:
    93a6cc: CALL 0x687e30 sub_687e30  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0
    93a6d3: CALL 0x687eb0 sub_687eb0  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=None
    93a6de: caller cleans 232 bytes
    93a6e4: RETURN

  --- summary ---
  absolute globals read: 0x212a679, 0x215d7ac, 0x215e5bc, 0x252a0d8
  absolute globals written: 
  direct callees: 0x951c20 ?, 0x9392a0 current_spawn_group_vehicle_index, 0xae2960 ?, 0x939450 ?, 0x939670 ?, 0x943de0 ?, 0x9396c0 ?, 0x939ac0 ?, 0x9397b0 ?, 0x93e810 ?, 0x93e850 ?, 0x555920 ?, 0xae2bb0 ?, 0xae2c80 spawn_carjack_vehicle, 0x53c4d0 activity_current_type, 0x938c30 traffic_record_activate, 0xae28f0 vehicle_from_handle, 0xac8d50 ?, 0xac8da0 ?, 0xab6c70 ?, 0x957580 ?, 0x939b70 ?, 0x939930 traffic_vehicle_create_and_activate, 0xd227e0 ?, 0xaabc80 ?, 0x939e20 ?, 0xae11f0 ?, 0xbdce30 rng_float01, 0xb2d020 vehicle_ai_set_force_flag_crazy_rails, 0x687e30 ?, 0x687eb0 ?
```

### traffic_vehicle_spawn_try - 0x939F30

**Role.** Traffic vehicle spawn try (documented): picks a vehicle type from the current spawn group (`0x215E5BC` vtable slot 5, `current_spawn_group_query`), creates it (`0xAE2960`, `0xAE3E80`), activates its record (`traffic_record_activate`) and populates the occupants (`0x939B70`).

**Convention.** Entry registers read (linear scan): esi. True size 0x393, 258 instructions, 34 blocks, 0 jump table(s). Callers: 0x93C2F2, 0x93CB76.

**Sections, in order of the calls (address and callee):** 0x939F5C current_spawn_group_query; 0x939FAE sub_AE2960; 0x93A051 sub_939670; 0x93A08E sub_943DE0; 0x93A0D9 sub_939450; 0x93A0F5 sub_AE3E80; 0x93A151 sub_93E850; 0x93A199 sub_6FEA40; 0x93A1BE sub_AE2F30; 0x93A1D2 traffic_record_activate; 0x93A1DA vehicle_from_handle; 0x93A1FE sub_AC8D50; 0x93A212 sub_AC8DA0; 0x93A21C sub_AB6C70; 0x93A23A sub_957580; 0x93A253 sub_939B70; 0x93A275 sub_D227E0; 0x93A288 sub_AABC80; 0x93A2AE sub_939E20.

**Hazards.** ESI is read at entry (surprise 3); cdecl stack args (&spawn, bool); `traffic_record_activate` takes EAX = record and ECX bit 0.

**Globals read:** 0x212A679, 0x215D7AC, 0x215E5BC, 0x252A0D8.

**Globals written:** none.

```
FUNCTION 0x00939F30  true size 0x393 (915 bytes)  insns 258  blocks 34
  entry regs read before write (linear prefix scan): ['esi']
  stack frame: esp, 0xec

  B0 @0x939f30:
    939f38: READ i32[0x215e5bc]   ; mov ebp, dword ptr [0x215e5bc]
    939f47: READ u8[0x212a679]   ; cmp byte ptr [0x212a679], bl
    939f51: if (byte ptr [0x212a679] != bl) goto B12  else B1   ; jne 0x939fa7

  B1 @0x939f53:
    939f55: if (ebp != edi) goto B3  else B2   ; jne 0x939f5c

  B2 @0x939f57:
    939f5a: goto B11 @0x939f9d

  B3 @0x939f5c:
    939f5c: CALL 0x957660 current_spawn_group_query  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    939f63: READ i32[0x215e5bc]   ; mov ebp, dword ptr [0x215e5bc]
    939f69: if (al != 0) goto B5  else B4   ; jne 0x939f70

  B4 @0x939f6b:
    939f6e: goto B11 @0x939f9d

  B5 @0x939f70:
    939f72: if (ebp == edi) goto B9  else B6   ; je 0x939f90

  B6 @0x939f74:
    939f7c: CALL indirect edx  regs[ecx<-(mov ecx, ebp) eax<-(mov eax, dword ptr [ebp])]  stack(first arg first)[]
    939f80: if (eax < (signed) edi) goto B8  else B7   ; jl 0x939f8a

  B7 @0x939f82:
    939f82: READ i32[0x252a0d8]   ; cmp eax, dword ptr [0x252a0d8]
    939f88: if (eax < (signed) dword ptr [0x252a0d8]) goto B10  else B8   ; jl 0x939f95

  B8 @0x939f8a:
    939f8a: READ i32[0x215e5bc]   ; mov ebp, dword ptr [0x215e5bc]

  B9 @0x939f90:
    939f93: goto B11 @0x939f9d

  B10 @0x939f95:
    939f95: READ i32[0x215e5bc]   ; mov ebp, dword ptr [0x215e5bc]

  B11 @0x939f9d:

  B12 @0x939fa7:
    939fa7: STACK-OBJECT &[esp+0x80] -> eax
    939fae: CALL 0xae2960 sub_ae2960  callee-reads[eax]  regs[eax<-(lea eax, [esp + 0x80])]  stack(first arg first)[]  callee-ret-imm=0
    939fbc: X87 STORE f32 [esp + 0x60] <- 0.0
    939fc3: X87 STORE f32 [esp + 0x64] <- 0.0
    939fc9: X87 STORE f32 [esp + 0x68] <- 0.0
    939fd0: X87 STORE f32 [esp + 0x70] <- 0.0
    939fdb: X87 STORE f32 [esp + 0x74] <- 0.0
    93a051: CALL 0x939670 sub_939670  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=0
    93a056: caller cleans 4 bytes
    93a05f: if (bl == 0) goto B16  else B13   ; je 0x93a08a

  B13 @0x93a061:
    93a06a: if (ebp != edi) goto B15  else B14   ; jne 0x93a077

  B14 @0x93a06c:
    93a075: goto B19 @0x93a0d1

  B15 @0x93a077:
    93a07f: CALL indirect eax  regs[ecx<-(mov ecx, ebp) eax<-(mov eax, dword ptr [edx + 0x54])]  stack(first arg first)[]
    93a088: goto B19 @0x93a0d1

  B16 @0x93a08a:
    93a08a: STACK-OBJECT &[esp+0x38] -> esi
    93a08e: CALL 0x943de0 sub_943de0  callee-reads[esi]  regs[esi<-(lea esi, [esp + 0x38])]  stack(first arg first)[]  callee-ret-imm=0
    93a095: if (al != 0) goto B18  else B17   ; jne 0x93a0a4

  B17 @0x93a097:
    93a09d: caller cleans 236 bytes
    93a0a3: RETURN

  B18 @0x93a0a4:
    93a0b3: INT imul ecx

  B19 @0x93a0d1:
    93a0d1: STACK-OBJECT &[esp+0x80] -> eax
    93a0d9: CALL 0x939450 sub_939450  callee-reads[]  regs[]  stack(first arg first)[eax]  callee-ret-imm=0
    93a0de: caller cleans 4 bytes
    93a0e3: if (al == 0) goto B17  else B20   ; je 0x93a097

  B20 @0x93a0e5:
    93a0ec: STACK-OBJECT &[esp+0x14] -> ecx
    93a0f1: STACK-OBJECT &[esp+0x24] -> ecx
    93a0f5: CALL 0xae3e80 sub_ae3e80  callee-reads[eax, ecx]  regs[eax<-(mov eax, dword ptr [esp + 0x88]), ecx<-(lea ecx, [esp + 0x24])]  stack(first arg first)[ecx]  callee-ret-imm=0
    93a102: STACK-OBJECT &[esp+0x18] -> edx
    93a106: caller cleans 4 bytes
    93a10a: X87 STORE f32 [esp + 0x28] <- (f32[esp + 0x28] - f32[esp + 0x1c])
    93a10e: STACK-OBJECT &[esp+0x84] -> edx
    93a11b: X87 fsub st(1), st(0)  (st(i) dest, untracked)
    93a11f: X87 STORE f32 [esp + 0x18] <- f32[esp + 0x18]
    93a125: X87 fsub st(1)  (st(i) dest, untracked)
    93a127: X87 STORE f32 [esp + 0x1c] <- 0.0
    93a12f: X87 fsub st(1)  (st(i) dest, untracked)
    93a131: X87 STORE f32 [esp + 0x20] <- f32[esp + 0x20]
    93a139: X87 fadd st(1)  (st(i) dest, untracked)
    93a13b: X87 STORE f32 [esp + 0x24] <- f32[esp + 0x24]
    93a143: X87 fadd st(1)  (st(i) dest, untracked)
    93a145: X87 STORE f32 [esp + 0x28] <- f32[esp + 0x28]
    93a14d: X87 STORE f32 [esp + 0x2c] <- (1.0 + f32[esp + 0x2c])
    93a151: CALL 0x93e850 sub_93e850  callee-reads[edx]  regs[edx<-(lea edx, [esp + 0x84])]  stack(first arg first)[edx]  callee-ret-imm=0
    93a156: caller cleans 4 bytes
    93a15b: if (al == 1) goto B17  else B21   ; je 0x93a097

  B21 @0x93a161:
    93a167: X87 STORE f32 [esp + 0x30] <- f32[esp + 0x14]
    93a16b: STACK-OBJECT &[esp+0x24] -> eax
    93a174: X87 STORE f32 [esp + 0x3c] <- f32[esp + 0x20]
    93a178: STACK-OBJECT &[esp+0x34] -> ecx
    93a187: STACK-OBJECT &[esp+0xac] -> edx
    93a18e: STACK-OBJECT &[esp+0xa0] -> ecx
    93a195: X87 STORE f32 [esp + 0x3c] <- (f32[esp + 0x20] - f64(2.0; bits 0x4000000000000000))
    93a199: CALL 0x6fea40 sub_6fea40  callee-reads[ecx, edx]  regs[ecx<-(lea ecx, [esp + 0xa0]), edx<-(lea edx, [esp + 0xac])]  stack(first arg first)[ecx, eax, 1]  callee-ret-imm=None
    93a19e: caller cleans 12 bytes
    93a1a3: if (al == 0) goto B17  else B22   ; je 0x93a097

  B22 @0x93a1a9:
    93a1a9: READ i32[0x215d7ac]   ; mov ebp, dword ptr [0x215d7ac]
    93a1b1: if (ebp == edi) goto B17  else B23   ; je 0x93a097

  B23 @0x93a1b7:
    93a1b7: STACK-OBJECT &[esp+0x80] -> edi
    93a1be: CALL 0xae2f30 sub_ae2f30  callee-reads[edi]  regs[edi<-(lea edi, [esp + 0x80])]  stack(first arg first)[]  callee-ret-imm=0
    93a1c8: if (eax == 0) goto B17  else B24   ; je 0x93a097

  B24 @0x93a1ce:
    93a1d2: CALL 0x938c30 traffic_record_activate  callee-reads[eax, ecx]  regs[eax<-(mov eax, ebp), ecx<-(mov cl, bl)]  stack(first arg first)[]  callee-ret-imm=0
    93a1da: CALL 0xae28f0 vehicle_from_handle  callee-reads[ecx]  regs[ecx<-(mov ecx, dword ptr [ebp + 8])]  stack(first arg first)[]  callee-ret-imm=0
    93a1fe: CALL 0xac8d50 sub_ac8d50  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [esp + 0x40])]  stack(first arg first)[]  callee-ret-imm=0
    93a205: if (al == 0) goto B26  else B25   ; je 0x93a21a

  B25 @0x93a207:
    93a212: CALL 0xac8da0 sub_ac8da0  callee-reads[ecx]  regs[ecx<-(mov ecx, dword ptr [edi + 0x7f9c])]  stack(first arg first)[eax]  callee-ret-imm=None
    93a217: caller cleans 4 bytes

  B26 @0x93a21a:
    93a21c: CALL 0xab6c70 sub_ab6c70  callee-reads[esi]  regs[esi<-(?)]  stack(first arg first)[0]  callee-ret-imm=0
    93a223: caller cleans 4 bytes
    93a226: X87 STORE f32 [esp + 0x10] <- 0.0
    93a22c: if (bl == 0) goto B28  else B27   ; je 0x93a245

  B27 @0x93a22e:
    93a23a: CALL 0x957580 sub_957580  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=None
    93a23f: X87 STORE f32 [esp + 0x10] <- ret(sub_957580)
    93a243: goto B29 @0x93a25b

  B28 @0x93a245:
    93a245: STACK-OBJECT &[esp+0x80] -> edx
    93a24d: STACK-OBJECT &[esp+0x3c] -> eax
    93a253: CALL 0x939b70 sub_939b70  callee-reads[]  regs[]  stack(first arg first)[esi, eax, edx]  callee-ret-imm=0
    93a258: caller cleans 12 bytes

  B29 @0x93a25b:
    93a25b: STACK-OBJECT &[esp+0xa0] -> ecx
    93a263: STACK-OBJECT &[esp+0x98] -> edx
    93a26b: STACK-OBJECT &[esp+0x28] -> eax
    93a270: STACK-OBJECT &[esp+0x20] -> ecx
    93a275: CALL 0xd227e0 sub_d227e0  callee-reads[]  regs[]  stack(first arg first)[ecx, eax, edx, ecx]  callee-ret-imm=0
    93a27a: caller cleans 16 bytes
    93a27f: if (al != 0) goto B31  else B30   ; jne 0x93a28d

  B30 @0x93a281:
    93a288: CALL 0xaabc80 sub_aabc80  callee-reads[eax]  regs[eax<-(mov eax, esi)]  stack(first arg first)[]  callee-ret-imm=0

  B31 @0x93a28d:
    93a28f: if (bl == 0) goto B33  else B32   ; je 0x93a2a4

  B32 @0x93a291:
    93a29d: X87 STORE f32 [esp + 0xd4] <- f32[esp + 0x10]

  B33 @0x93a2a4:
    93a2a4: STACK-OBJECT &[esp+0x80] -> edx
    93a2ae: CALL 0x939e20 sub_939e20  callee-reads[]  regs[]  stack(first arg first)[edx]  callee-ret-imm=None
    93a2b3: caller cleans 4 bytes
    93a2bc: caller cleans 236 bytes
    93a2c2: RETURN

  --- summary ---
  absolute globals read: 0x212a679, 0x215d7ac, 0x215e5bc, 0x252a0d8
  absolute globals written: 
  direct callees: 0x957660 current_spawn_group_query, 0xae2960 ?, 0x939670 ?, 0x943de0 ?, 0x939450 ?, 0xae3e80 ?, 0x93e850 ?, 0x6fea40 ?, 0xae2f30 ?, 0x938c30 traffic_record_activate, 0xae28f0 vehicle_from_handle, 0xac8d50 ?, 0xac8da0 ?, 0xab6c70 ?, 0x957580 ?, 0x939b70 ?, 0xd227e0 ?, 0xaabc80 ?, 0x939e20 ?
```

### traffic_despawn_update - 0x93BE50

**Role.** Traffic despawn update: timers `0xEB7A8C/90/94`, candidate scans `0x938EB0` (twice), `traffic_record_start_despawn` and `vehicle_take_damage` for stuck or far vehicles; the scan arguments are stack (list, count, bool, ...).

**Convention.** Entry registers read (linear scan): none. True size 0x368, 263 instructions, 75 blocks, 1 jump table(s). Callers: 0x93C702.

**Sections, in order of the calls (address and callee):** 0x93BEBA timer_set_random; 0x93BF39 sub_AA75E0; 0x93C00E sub_AABFF0; 0x93C01E timer_elapsed_ms; 0x93C0C5 sub_938EB0; 0x93C0DA traffic_record_start_despawn; 0x93C157 sub_938EB0; 0x93C175 vehicle_take_damage.

**Jump table** at 0x93C1B8 used by the `jmp` at 0x93BFCF: .

**Hazards.** One jump table; many f32 compares against camera and player positions; calls `aa75e0 / aabff0` for the AI state.

**Globals read:** 0xE843AC, 0xEB7A8C, 0xEB7A90, 0xEB7A94, 0x215D948, 0x215D998, 0x215D99C, 0x215D9EC, 0x215D9F0, 0x215D9F4, 0x215DA30, 0x21703D4, 0x2528B48, 0x2528B49, 0x2528C14.

**Globals written:** 0xEB7A90, 0xEB7A94.

```
FUNCTION 0x0093BE50  true size 0x368 (872 bytes)  insns 263  blocks 75
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0xfc

  B0 @0x93be50:
    93be50: READ i32[0xeb7a8c]   ; mov eax, dword ptr [0xeb7a8c]
    93be61: if (eax < 0 (signed)) goto B74  else B1   ; jl 0x93c1ad

  B1 @0x93be67:
    93be67: READ i32[0xe843ac]   ; mov esi, dword ptr [0xe843ac]
    93be6f: if (eax <= (signed) esi) goto B4  else B2   ; jle 0x93be80

  B2 @0x93be71:
    93be78: if (eax < (signed) 0x35a4e900) goto B74  else B3   ; jl 0x93c1ad

  B3 @0x93be7e:
    93be7e: goto B5 @0x93be90

  B4 @0x93be80:
    93be8a: if (ecx >= (signed) 0x35a4e900) goto B74  else B5   ; jge 0x93c1ad

  B5 @0x93be90:
    93be90: READ i32[0x21703d4]   ; mov eax, dword ptr [0x21703d4]
    93be9f: if (eax == 0) goto B7  else B6   ; je 0x93beab

  B6 @0x93bea1:

  B7 @0x93beab:
    93beba: CALL 0xbdd1b0 timer_set_random  callee-reads[eax]  regs[eax<-(mov eax, 0x258)]  stack(first arg first)[0x190]  callee-ret-imm=4
    93bebf: READ i32[0x215da30]   ; mov edi, dword ptr [0x215da30]
    93becf: if (edi == 0) goto B23  else B8   ; je 0x93bf5f

  B8 @0x93bed5:
    93bed5: STACK-OBJECT &[esp+0x1c] -> ebx
    93bed9: STACK-OBJECT &[esp+0x0] -> esp

  B9 @0x93bee0:
    93bee5: if (eax == 0) goto B21  else B10   ; je 0x93bf4f

  B10 @0x93bee7:
    93befa: if (edx == 0) goto B21  else B11   ; je 0x93bf4f

  B11 @0x93befc:
    93bf01: if (ebp != eax) goto B21  else B12   ; jne 0x93bf4f

  B12 @0x93bf03:
    93bf07: if (dword ptr [edx + 0x4c] != 5) goto B21  else B13   ; jne 0x93bf4f

  B13 @0x93bf09:
    93bf11: if (esi == 0) goto B18  else B14   ; je 0x93bf39

  B14 @0x93bf13:
    93bf13: READ i32[0x2528c14]   ; mov eax, dword ptr [0x2528c14]
    93bf1a: if (eax == 0) goto B18  else B15   ; je 0x93bf39

  B15 @0x93bf1c:
    93bf24: if (eax == 0) goto B18  else B16   ; je 0x93bf39

  B16 @0x93bf26:
    93bf2e: if (cl != byte ptr [eax + 0x1b0]) goto B20  else B17   ; jne 0x93bf49

  B17 @0x93bf30:
    93bf37: if ((byte ptr [esi + 0x67c] & 1) != 0) goto B20  else B18   ; jne 0x93bf49

  B18 @0x93bf39:
    93bf39: CALL 0xaa75e0 sub_aa75e0  callee-reads[edx]  regs[edx<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    93bf40: if (al == 0) goto B31  else B19   ; je 0x93bfb5

  B19 @0x93bf42:

  B20 @0x93bf49:
    93bf49: READ i32[0xe843ac]   ; mov esi, dword ptr [0xe843ac]

  B21 @0x93bf4f:
    93bf51: READ i32[0x215da30]   ; cmp eax, dword ptr [0x215da30]
    93bf57: if (eax == dword ptr [0x215da30]) goto B23  else B22   ; je 0x93bf5f

  B22 @0x93bf59:
    93bf5d: if (edi != 0) goto B9  else B23   ; jne 0x93bee0

  B23 @0x93bf5f:
    93bf5f: READ i32[0x215d948]   ; mov eax, dword ptr [0x215d948]
    93bf64: READ i32[0x215d99c]   ; mov ecx, dword ptr [0x215d99c]
    93bf6c: if (eax < (signed) ecx) goto B25  else B24   ; jl 0x93bf70

  B24 @0x93bf6e:

  B25 @0x93bf70:
    93bf76: if (ebx < (signed) eax) goto B73  else B26   ; jl 0x93c1a3

  B26 @0x93bf7c:
    93bf7c: READ i32[0xeb7a90]   ; mov eax, dword ptr [0xeb7a90]
    93bf83: if (eax >= 0 (signed)) goto B44  else B27   ; jge 0x93c044

  B27 @0x93bf89:
    93bf91: WRITE i32[0xeb7a90] mov = eax   ; dword ptr [0xeb7a90], eax
    93bf96: if (esi <= (signed) 0x6b49d200) goto B29  else B28   ; jle 0x93bfa3

  B28 @0x93bf98:
    93bf9e: WRITE i32[0xeb7a90] mov = eax   ; dword ptr [0xeb7a90], eax

  B29 @0x93bfa3:
    93bfa5: if (eax >= 0 (signed)) goto B44  else B30   ; jge 0x93c044

  B30 @0x93bfab:
    93bfb0: goto B50 @0x93c071

  B31 @0x93bfb5:
    93bfb9: if (ebp == dword ptr [esp + 0x14]) goto B20  else B32   ; je 0x93bf49

  B32 @0x93bfbb:
    93bfc6: if (eax > (unsigned) 6) goto B34  else B33   ; ja 0x93bfd8

  B33 @0x93bfc8:
    93bfcf: SWITCH dword ptr [eax*4 + 0x93c1b8] table 0x93c1b8, 0 entries: 

  B34 @0x93bfd8:
    93bfd8: READ u8[0x2528b48]   ; mov al, byte ptr [0x2528b48]
    93bfdf: if (al != 1) goto B36  else B35   ; jne 0x93bfea

  B35 @0x93bfe1:
    93bfe1: READ u8[0x2528b49]   ; mov al, byte ptr [0x2528b49]
    93bfe8: if (al == 0) goto B37  else B36   ; je 0x93bfee

  B36 @0x93bfea:
    93bfec: if (cl == 0) goto B42  else B37   ; je 0x93c02e

  B37 @0x93bfee:
    93bff5: if (dword ptr [edi + 0xc] >= (signed) 0) goto B40  else B38   ; jge 0x93c00c

  B38 @0x93bff7:
    93bff7: READ i32[0xe843ac]   ; mov eax, dword ptr [0xe843ac]
    93c003: if (eax <= (signed) 0x6b49d200) goto B40  else B39   ; jle 0x93c00c

  B39 @0x93c005:

  B40 @0x93c00c:
    93c00e: CALL 0xaabff0 sub_aabff0  callee-reads[eax]  regs[eax<-(mov eax, edx)]  stack(first arg first)[]  callee-ret-imm=0
    93c015: if (al != 0) goto B42  else B41   ; jne 0x93c02e

  B41 @0x93c017:
    93c01e: CALL 0xbdd2a0 timer_elapsed_ms  callee-reads[eax]  regs[eax<-(mov eax, ecx)]  stack(first arg first)[]  callee-ret-imm=0

  B42 @0x93c02e:
    93c033: READ i32[0xe843ac]   ; mov esi, dword ptr [0xe843ac]
    93c039: if (dword ptr [esp + 0x10] == 0x14) goto B23  else B43   ; je 0x93bf5f

  B43 @0x93c03f:
    93c03f: goto B21 @0x93bf4f

  B44 @0x93c044:
    93c046: if (eax <= (signed) esi) goto B47  else B45   ; jle 0x93c058

  B45 @0x93c048:
    93c04f: if (eax <= (signed) 0x35a4e900) goto B50  else B46   ; jle 0x93c071

  B46 @0x93c051:
    93c056: goto B50 @0x93c071

  B47 @0x93c058:
    93c062: if (ecx <= (signed) 0x35a4e900) goto B49  else B48   ; jle 0x93c06d

  B48 @0x93c064:
    93c06b: goto B50 @0x93c071

  B49 @0x93c06d:

  B50 @0x93c071:
    93c071: READ i32[0x215d9ec]   ; mov edx, dword ptr [0x215d9ec]
    93c07f: if (edx <= 0 (signed)) goto B55  else B51   ; jle 0x93c099

  B51 @0x93c081:

  B52 @0x93c086:
    93c089: if (ebx < (signed) dword ptr [ecx - 4]) goto B54  else B53   ; jl 0x93c08f

  B53 @0x93c08b:
    93c08d: if (edi >= (signed) dword ptr [ecx]) goto B59  else B54   ; jge 0x93c0b6

  B54 @0x93c08f:
    93c097: if (eax < (signed) edx) goto B52  else B55   ; jl 0x93c086

  B55 @0x93c099:
    93c099: READ i32[0xeb7a94]   ; mov eax, dword ptr [0xeb7a94]
    93c0a0: if (eax < 0 (signed)) goto B65  else B56   ; jl 0x93c114

  B56 @0x93c0a2:
    93c0a4: if (eax <= (signed) esi) goto B61  else B57   ; jle 0x93c0ed

  B57 @0x93c0a6:
    93c0ad: if (eax <= (signed) 0x35a4e900) goto B64  else B58   ; jle 0x93c106

  B58 @0x93c0af:
    93c0b4: goto B64 @0x93c106

  B59 @0x93c0b6:
    93c0b6: READ i32[0x215d9f0]   ; mov ecx, dword ptr [0x215d9f0]
    93c0bf: STACK-OBJECT &[esp+0x20] -> edx
    93c0c5: CALL 0x938eb0 sub_938eb0  callee-reads[]  regs[]  stack(first arg first)[edx, ebx, 1, ecx]  callee-ret-imm=0
    93c0ca: caller cleans 16 bytes
    93c0d0: if (eax == -1) goto B55  else B60   ; je 0x93c099

  B60 @0x93c0d2:
    93c0da: CALL 0x938fd0 traffic_record_start_despawn  callee-reads[]  regs[]  stack(first arg first)[ecx]  callee-ret-imm=0
    93c0df: caller cleans 4 bytes
    93c0e6: caller cleans 252 bytes
    93c0ec: RETURN

  B61 @0x93c0ed:
    93c0f7: if (ecx <= (signed) 0x35a4e900) goto B63  else B62   ; jle 0x93c102

  B62 @0x93c0f9:
    93c100: goto B64 @0x93c106

  B63 @0x93c102:

  B64 @0x93c106:
    93c108: READ i32[0x215d9f4]   ; cmp eax, dword ptr [0x215d9f4]
    93c10e: if (eax < (signed) dword ptr [0x215d9f4]) goto B74  else B65   ; jl 0x93c1ad

  B65 @0x93c114:
    93c114: READ i32[0x215d998]   ; mov ecx, dword ptr [0x215d998]
    93c11e: if (ecx <= 0 (signed)) goto B74  else B66   ; jle 0x93c1ad

  B66 @0x93c124:
    93c12b: if (ebx < (signed) dword ptr [eax*8 + 0x215d948]) goto B68  else B67   ; jl 0x93c136

  B67 @0x93c12d:
    93c134: if (edi >= (signed) dword ptr [eax*8 + 0x215d94c]) goto B70  else B68   ; jge 0x93c148

  B68 @0x93c136:
    93c13b: if (eax < (signed) ecx) goto B66  else B69   ; jl 0x93c124

  B69 @0x93c13d:
    93c141: caller cleans 252 bytes
    93c147: RETURN

  B70 @0x93c148:
    93c148: READ i32[0x215d9f0]   ; mov edx, dword ptr [0x215d9f0]
    93c151: STACK-OBJECT &[esp+0x20] -> eax
    93c157: CALL 0x938eb0 sub_938eb0  callee-reads[]  regs[]  stack(first arg first)[eax, ebx, 0, edx]  callee-ret-imm=0
    93c15c: caller cleans 16 bytes
    93c162: if (eax == -1) goto B74  else B71   ; je 0x93c1ad

  B71 @0x93c164:
    93c175: CALL 0xaabcc0 vehicle_take_damage  callee-reads[]  regs[]  stack(first arg first)[edx, 0x3e8, 1, 0]  callee-ret-imm=None
    93c17a: READ i32[0xe843ac]   ; mov eax, dword ptr [0xe843ac]
    93c17f: caller cleans 16 bytes
    93c187: WRITE i32[0xeb7a94] mov = eax   ; dword ptr [0xeb7a94], eax
    93c18c: if (eax <= (signed) 0x6b49d200) goto B74  else B72   ; jle 0x93c1ad

  B72 @0x93c18e:
    93c193: WRITE i32[0xeb7a94] mov = eax   ; dword ptr [0xeb7a94], eax
    93c19c: caller cleans 252 bytes
    93c1a2: RETURN

  B73 @0x93c1a3:
    93c1a3: WRITE i32[0xeb7a90] mov = 0xFFFFFFFF   ; dword ptr [0xeb7a90], 0xffffffff

  B74 @0x93c1ad:
    93c1b1: caller cleans 252 bytes
    93c1b7: RETURN

  --- summary ---
  absolute globals read: 0xe843ac, 0xeb7a8c, 0xeb7a90, 0xeb7a94, 0x215d948, 0x215d998, 0x215d99c, 0x215d9ec, 0x215d9f0, 0x215d9f4, 0x215da30, 0x21703d4, 0x2528b48, 0x2528b49, 0x2528c14
  absolute globals written: 0xeb7a90, 0xeb7a94
  direct callees: 0xbdd1b0 timer_set_random, 0xaa75e0 ?, 0xaabff0 ?, 0xbdd2a0 timer_elapsed_ms, 0x938eb0 ?, 0x938fd0 traffic_record_start_despawn, 0xaabcc0 vehicle_take_damage
```

### ambient_peds_spawn_pass - 0x937FF0

**Role.** Ambient pedestrian spawn pass: squared distance checks (`vec3_distance`), co-op scaling (`coop_spawn_count_scale`), spawn rate limit (`ambient_ped_spawn_rate_limit`), ped counts `0x215D940`, then up to the budget tries `ambient_ped_try_random_offset`.

**Convention.** Entry registers read (linear scan): none. True size 0x332, 246 instructions, 66 blocks, 1 jump table(s). Callers: 0x539DB3, 0x685BB1, 0x698F0D, 0x93D3AB.

**Sections, in order of the calls (address and callee):** 0x938034 ambient_ped_spawn_rate_limit; 0x938061 vec3_distance; 0x9380F4 coop_partner_human; 0x93810E coop_spawn_count_scale; 0x938125 sub_68A500; 0x938253 sub_935E70; 0x93825A sub_937F80; 0x9382FD ambient_ped_try_random_offset.

**Jump table** at 0x938324 used by the `jmp` at 0x9380CA: 0->0x9380D8, 1->0x9380D8, 2->0x9380D8.

**Hazards.** One jump table; rng draws; writes the byte `0x2529585`; co-op branch tests `0x2528B48/49`.

**Globals read:** 0xEC3ABC, 0x140E72C, 0x215D940, 0x2526DE8, 0x2526DEC, 0x2528B48, 0x2528B49, 0x2529634, 0x252963C, 0x2529E20.

**Globals written:** 0xEC3ABC, 0x2529585.

```
FUNCTION 0x00937FF0  true size 0x332 (818 bytes)  insns 246  blocks 66
  entry regs read before write (linear prefix scan): []
  stack frame: esp, 0x19c

  B0 @0x937ff0:
    937fff: READ i32[0x2529e20]   ; cmp dword ptr [0x2529e20], ebx
    938007: if (dword ptr [0x2529e20] == ebx) goto B65  else B1   ; je 0x93831b

  B1 @0x93800d:

  B2 @0x938010:
    938016: if (byte ptr [eax + 0x215da24] == bl) goto B65  else B3   ; je 0x93831b

  B3 @0x93801c:
    938022: if (eax < (unsigned) 6) goto B2  else B4   ; jb 0x938010

  B4 @0x938024:
    938026: if (edx != ebx) goto B6  else B5   ; jne 0x93802d

  B5 @0x938028:

  B6 @0x93802d:
    93802d: WRITE u8[0x2529585] mov = 0x1   ; byte ptr [0x2529585], 1
    938034: CALL 0x937e60 ambient_ped_spawn_rate_limit  callee-reads[edx]  regs[edx<-(?)]  stack(first arg first)[]  callee-ret-imm=0
    938039: READ i32[0x215d940]   ; mov edi, dword ptr [0x215d940]
    938041: if (edi <= (signed) ebx) goto B65  else B7   ; jle 0x93831b

  B7 @0x938047:
    93804d: X87 STORE f32 [esp + 0xc] <- 0.0
    938051: if (edi <= (signed) ebx) goto B9  else B8   ; jle 0x938084

  B8 @0x938053:
    938061: CALL 0xbde980 vec3_distance  callee-reads[]  regs[]  stack(first arg first)[edx, ecx]  callee-ret-imm=0
    938066: X87 STORE f32 [esp + 0x18] <- ret(vec3_distance)
    938071: X87 STORE f32 [esp + esi*4 + 0x1c] <- f32[esp + 0x18]
    938075: caller cleans 8 bytes
    93807e: X87 STORE f32 [esp + 0xc] <- (f32[esp + 0x18] + f32[esp + 0xc])
    938082: if (esi < (signed) edi) goto B8  else B9   ; jl 0x938053

  B9 @0x938084:
    938088: if (edi <= (signed) ebx) goto B13  else B10   ; jle 0x9380a1

  B10 @0x93808a:

  B11 @0x93808e:
    938097: X87 fdiv st(1)  (st(i) dest, untracked)
    938099: X87 STORE f32 [esp + eax*4 + 0x14] <- f32[esp + eax*4 + 0x18]
    93809d: if (eax < (signed) edi) goto B11  else B12   ; jl 0x93808e

  B12 @0x93809f:

  B13 @0x9380a1:
    9380a1: READ i32[0x2529634]   ; mov eax, dword ptr [0x2529634]
    9380aa: if (eax != ebx) goto B15  else B14   ; jne 0x9380b3

  B14 @0x9380ac:
    9380ae: goto B49 @0x938253

  B15 @0x9380b3:
    9380b3: READ u8[0x140e72c]   ; movzx eax, byte ptr [0x140e72c]
    9380bd: if (result >= 0 (sign clear) of `sub eax, 6`) goto B17  else B16   ; jns 0x9380c2

  B16 @0x9380bf:

  B17 @0x9380c2:
    9380c8: if (eax > (unsigned) 5) goto B19  else B18   ; ja 0x9380d8

  B18 @0x9380ca:
    9380ca: SWITCH dword ptr [eax*4 + 0x938324] table 0x938324, 3 entries: 0->B19, 1->B19, 2->B19

  B19 @0x9380d8:
    9380da: READ u8[0x2528b48]   ; mov al, byte ptr [0x2528b48]
    9380e1: if (al != 1) goto B22  else B20   ; jne 0x9380f4

  B20 @0x9380e3:
    9380e3: READ u8[0x2528b49]   ; mov cl, byte ptr [0x2528b49]
    9380eb: if (cl != 0) goto B22  else B21   ; jne 0x9380f4

  B21 @0x9380ed:
    9380f2: goto B25 @0x938118

  B22 @0x9380f4:
    9380f4: CALL 0x9d7bd0 coop_partner_human  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    9380fb: if (eax != 0) goto B24  else B23   ; jne 0x938102

  B23 @0x9380fd:
    938100: goto B25 @0x938118

  B24 @0x938102:
    93810b: X87 STORE f32 [esp] <- f32[0xe849a8]
    93810e: CALL 0x93e8d0 coop_spawn_count_scale  callee-reads[]  regs[]  stack(first arg first)[ecx, 0x1a]  callee-ret-imm=0
    938113: caller cleans 8 bytes

  B25 @0x938118:
    938118: READ i32[0x252963c]   ; mov eax, dword ptr [0x252963c]
    93811f: if (eax != 0) goto B41  else B26   ; jne 0x938205

  B26 @0x938125:
    938125: CALL 0x68a500 sub_68a500  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93812a: X87 STORE f32 [esp + 0x10] <- ret(sub_68a500)
    938132: READ i32[0x2526de8]   ; mov eax, dword ptr [0x2526de8]
    93813e: READ i32[0x2526dec]   ; mov eax, dword ptr [0x2526dec]
    938154: X87 STORE f32 [esp + 0xc] <- (f32[esp + 0x10] + (((1.0 - ST(st(0))) * f32[esi + 0x28]) * f32[esi + 0x24]))
    938158: if (eax == 0) goto B28  else B27   ; je 0x938165

  B27 @0x93815a:
    938161: X87 STORE f32 [esp + 0xc] <- (f32[eax + 0x14] * f32[esp + 0xc])

  B28 @0x938165:
    93816c: STACK-OBJECT &[esp+0x0] -> esp

  B29 @0x938170:
    938174: if (ecx <= (signed) edx) goto B31  else B30   ; jle 0x938178

  B30 @0x938176:

  B31 @0x938178:
    938180: if (eax < (signed) 0x279ab50) goto B29  else B32   ; jl 0x938170

  B32 @0x938182:
    93818d: X87 STORE f32 [esp + 0xc] <- (f32[edx*4 + 0xe888f4] * f32[esp + 0xc])
    93819b: X87 COMPARE f32[esp + 0xc] vs STst(1)   ; fcom st(1)
    9381a2: if (x87 compare ('f32[esp + 0xc]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B34  else B33   ; jne 0x9381a8

  B33 @0x9381a4:
    9381a6: goto B35 @0x9381aa

  B34 @0x9381a8:

  B35 @0x9381aa:
    9381aa: X87 STORE f32 [esp + 0xc] <- ret(?)
    9381b6: X87 COMPARE f32[0xe888f0] vs STst(1)   ; fcom st(1)
    9381bf: if (x87 compare ('f32[0xe888f0]', 'STst(1)'): ST0 outcome in {lt, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B37  else B36   ; jne 0x9381c7

  B36 @0x9381c1:
    9381c1: X87 STORE f32 [esp + 0xc] <- ret(?)
    9381c5: goto B38 @0x9381c9

  B37 @0x9381c7:

  B38 @0x9381c9:
    9381d5: if (ecx >= 0 (signed)) goto B40  else B39   ; jge 0x9381dd

  B39 @0x9381d7:

  B40 @0x9381dd:
    9381f7: X87 STORE int64 [esp + 0x10] <- round((ret(?) * f32[esp + 0xc]))
    938203: goto B44 @0x93823f

  B41 @0x938205:
    93820f: if (edx >= 0 (signed)) goto B43  else B42   ; jge 0x938217

  B42 @0x938211:

  B43 @0x938217:
    938233: X87 STORE int64 [esp + 0x10] <- round((ret(?) * f32[0xe888b0]))

  B44 @0x93823f:
    938242: if (eax > (unsigned) 1) goto B46  else B45   ; ja 0x938249

  B45 @0x938244:

  B46 @0x938249:
    93824d: if (eax < (unsigned) ebx) goto B48  else B47   ; jb 0x938251

  B47 @0x93824f:

  B48 @0x938251:

  B49 @0x938253:
    938253: CALL 0x935e70 sub_935e70  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93825a: CALL 0x937f80 sub_937f80  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    938261: if (esi >= (signed) eax) goto B51  else B50   ; jge 0x938265

  B50 @0x938263:

  B51 @0x938265:
    938267: if (eax <= (signed) ebx) goto B65  else B52   ; jle 0x93831b

  B52 @0x93826d:

  B53 @0x938270:
    938272: goto B54 @0x938280

  B54 @0x938280:
    938280: READ i32[0xec3abc]   ; mov eax, dword ptr [0xec3abc]
    938298: WRITE i32[0xec3abc] mov = eax   ; dword ptr [0xec3abc], eax
    93829d: if (eax != 0x800) goto B56  else B55   ; jne 0x9382a5

  B55 @0x93829f:
    93829f: WRITE i32[0xec3abc] mov = ebx   ; dword ptr [0xec3abc], ebx

  B56 @0x9382a5:
    9382af: if (ecx >= 0 (signed)) goto B58  else B57   ; jge 0x9382b7

  B57 @0x9382b1:

  B58 @0x9382b7:
    9382bf: X87 STORE f32 [esp + 0x10] <- (ret(?) * f64(2.3283064365386963e-10; bits 0x3DF0000000000000))
    9382c3: X87 STORE f32 [esp + 0xc] <- ret(?)
    9382cb: X87 COMPARE f32[esp + 0x10] vs STst(1)   ; fcom st(1)
    9382d4: if (x87 compare ('f32[esp + 0x10]', 'STst(1)'): ST0 outcome in {lt, eq, un}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B62  else B59   ; jne 0x9382f4

  B59 @0x9382d6:

  B60 @0x9382da:
    9382e1: X87 STORE f32 [esp + 0xc] <- (ret(?) + f32[esp + ecx*4 + 0x1c])
    9382e9: X87 COMPARE f32[esp + 0xc] vs STst(1)   ; fcom st(1)
    9382f0: if (x87 compare ('f32[esp + 0xc]', 'STst(1)'): ST0 outcome in {lt}  (gt = ST0>op, lt = ST0<op, un = unordered)) goto B60  else B61   ; jnp 0x9382da

  B61 @0x9382f2:

  B62 @0x9382f4:
    9382fd: CALL 0x9367b0 ambient_ped_try_random_offset  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [ecx*4 + 0x215d7b0])]  stack(first arg first)[]  callee-ret-imm=0
    938304: if (eax != 0) goto B64  else B63   ; jne 0x938312

  B63 @0x938306:
    93830c: if (esi < (signed) 3) goto B54  else B64   ; jl 0x938280

  B64 @0x938312:
    938315: if (result != 0 of `sub edi, 1`) goto B53  else B65   ; jne 0x938270

  B65 @0x93831b:
    938321: RETURN

  --- summary ---
  absolute globals read: 0xec3abc, 0x140e72c, 0x215d940, 0x2526de8, 0x2526dec, 0x2528b48, 0x2528b49, 0x2529634, 0x252963c, 0x2529e20
  absolute globals written: 0xec3abc, 0x2529585
  direct callees: 0x937e60 ambient_ped_spawn_rate_limit, 0xbde980 vec3_distance, 0x9d7bd0 coop_partner_human, 0x93e8d0 coop_spawn_count_scale, 0x68a500 ?, 0x935e70 ?, 0x937f80 ?, 0x9367b0 ambient_ped_try_random_offset
```

## Small functions: recipes

Fifteen small, self-contained functions. Arguments are in registers or on the stack exactly as in each heading, so a Rust replacement needs a register shim like the existing `detour` ones.

### spawn_phase_step_mod3 - 0x93F1A0

**Convention.** cdecl, no args; result in AL.

1. `v = i32[0x252962C]`. `bl = (v % 3 == 0)` using the signed multiply by 0x55555556 (the compiler's divide by 3; `edx - 3*(v/3) == 0`).
2. `i32[0x252962C] = (v + 1) % 3` with a real signed `idiv` (remainder keeps the sign of the dividend).
3. Return AL = bl (1 when the value *before* the step was a multiple of 3). Registers clobbered: EAX, ECX, EDX (EBX saved).

```
FUNCTION 0x0093F1A0  true size 0x37 (55 bytes)  insns 21  blocks 3
  entry regs read before write (linear prefix scan): []

  B0 @0x93f1a0:
    93f1a0: READ i32[0x252962c]   ; mov ecx, dword ptr [0x252962c]
    93f1ab: INT imul ecx
    93f1be: if (result != 0 of `sub edx, eax`) goto B2  else B1   ; jne 0x93f1c2

  B1 @0x93f1c0:

  B2 @0x93f1c2:
    93f1c5: INT cdq 
    93f1cb: INT idiv ecx
    93f1d0: WRITE i32[0x252962c] mov = edx   ; dword ptr [0x252962c], edx
    93f1d6: RETURN

  --- summary ---
  absolute globals read: 0x252962c
  absolute globals written: 0x252962c
  direct callees:
```

Disassembly:
```
0093f1a0  mov ecx, dword ptr [0x252962c]
0093f1a6  mov eax, 0x55555556
0093f1ab  imul ecx
0093f1ad  mov eax, edx
0093f1af  shr eax, 0x1f
0093f1b2  add eax, edx
0093f1b4  push ebx
0093f1b5  lea eax, [eax + eax*2]
0093f1b8  mov edx, ecx
0093f1ba  xor bl, bl
0093f1bc  sub edx, eax
0093f1be  jne 0x93f1c2
0093f1c0  mov bl, 1
0093f1c2  lea eax, [ecx + 1]
0093f1c5  cdq 
0093f1c6  mov ecx, 3
0093f1cb  idiv ecx
0093f1cd  mov al, bl
0093f1cf  pop ebx
0093f1d0  mov dword ptr [0x252962c], edx
0093f1d6  ret 
```

### spawn_info_refresh_timer_check - 0x93F150

**Convention.** cdecl, no args, no result.

1. `t = i32[0xEB7B54]`, `now = i32[0xE843AC]`. If `t < 0` (signed) go to step 3.
2. If `t > now` (signed, `jle` falls to the other branch): `d = t - now`; if `d < 0x35A4E900` (signed `jl`) return (not yet due). Else (`t <= now`): `d = now - t`; if `d >= 0x35A4E900` (signed `jge`) return.
3. Due: `new = now + 0x4E20`; `u8[0x2529622] = 1`; `i32[0xEB7B54] = new`; if `new > 0x6B49D200` (signed `jg`) store `new - 0x6B49D200` instead. (The store of 1 happens before the wrap; the timestamp store before the compare result is used.)

```
FUNCTION 0x0093F150  true size 0x4E (78 bytes)  insns 22  blocks 8
  entry regs read before write (linear prefix scan): []

  B0 @0x93f150:
    93f150: READ i32[0xeb7b54]   ; mov eax, dword ptr [0xeb7b54]
    93f157: READ i32[0xe843ac]   ; mov ecx, dword ptr [0xe843ac]
    93f15d: if (eax < 0 (signed)) goto B5  else B1   ; jl 0x93f17a

  B1 @0x93f15f:
    93f161: if (eax <= (signed) ecx) goto B4  else B2   ; jle 0x93f16e

  B2 @0x93f163:
    93f16a: if (eax < (signed) 0x35a4e900) goto B7  else B3   ; jl 0x93f19d

  B3 @0x93f16c:
    93f16c: goto B5 @0x93f17a

  B4 @0x93f16e:
    93f178: if (edx >= (signed) 0x35a4e900) goto B7  else B5   ; jge 0x93f19d

  B5 @0x93f17a:
    93f185: WRITE u8[0x2529622] mov = 0x1   ; byte ptr [0x2529622], 1
    93f18c: WRITE i32[0xeb7b54] mov = eax   ; dword ptr [0xeb7b54], eax
    93f191: if (eax <= (signed) 0x6b49d200) goto B7  else B6   ; jle 0x93f19d

  B6 @0x93f193:
    93f198: WRITE i32[0xeb7b54] mov = eax   ; dword ptr [0xeb7b54], eax

  B7 @0x93f19d:
    93f19d: RETURN

  --- summary ---
  absolute globals read: 0xe843ac, 0xeb7b54
  absolute globals written: 0xeb7b54, 0x2529622
  direct callees:
```

Disassembly:
```
0093f150  mov eax, dword ptr [0xeb7b54]
0093f155  test eax, eax
0093f157  mov ecx, dword ptr [0xe843ac]
0093f15d  jl 0x93f17a
0093f15f  cmp eax, ecx
0093f161  jle 0x93f16e
0093f163  sub eax, ecx
0093f165  cmp eax, 0x35a4e900
0093f16a  jl 0x93f19d
0093f16c  jmp 0x93f17a
0093f16e  mov edx, ecx
0093f170  sub edx, eax
0093f172  cmp edx, 0x35a4e900
0093f178  jge 0x93f19d
0093f17a  lea eax, [ecx + 0x4e20]
0093f180  cmp eax, 0x6b49d200
0093f185  mov byte ptr [0x2529622], 1
0093f18c  mov dword ptr [0xeb7b54], eax
0093f191  jle 0x93f19d
0093f193  sub eax, 0x6b49d200
0093f198  mov dword ptr [0xeb7b54], eax
0093f19d  ret 
```

### ambient_spawn_timer_rearm - 0x93C610

**Convention.** cdecl, no args; clobbers EAX, ECX, EDX (EDI saved).

1. If `u8[0x2528B48] == 1 && u8[0x2528B49] == 0` (host / single player): `edx = 0x64`, `ecx = 0x7D0`; else `edx = 0x32`, `ecx = 0xFA`.
2. `eax = ecx + edx` (max), push `ecx - edx` (min), `edi = 0xEB7A80` (timer), call `timer_set_random` (0xBDD1B0; EDI = &stamp, EAX = max, stdcall min). Result ignored; EDI restored.

```
FUNCTION 0x0093C610  true size 0x4D (77 bytes)  insns 26  blocks 4
  entry regs read before write (linear prefix scan): []

  B0 @0x93c610:
    93c610: READ u8[0x2528b48]   ; mov al, byte ptr [0x2528b48]
    93c617: if (al != 1) goto B3  else B1   ; jne 0x93c640

  B1 @0x93c619:
    93c619: READ u8[0x2528b49]   ; mov cl, byte ptr [0x2528b49]
    93c621: if (cl != 0) goto B3  else B2   ; jne 0x93c640

  B2 @0x93c623:
    93c639: CALL 0xbdd1b0 timer_set_random  callee-reads[eax]  regs[eax<-(lea eax, [ecx + edx])]  stack(first arg first)[ecx]  callee-ret-imm=4
    93c63f: RETURN

  B3 @0x93c640:
    93c656: CALL 0xbdd1b0 timer_set_random  callee-reads[eax]  regs[eax<-(lea eax, [ecx + edx])]  stack(first arg first)[ecx, edi]  callee-ret-imm=4
    93c65c: RETURN

  --- summary ---
  absolute globals read: 0x2528b48, 0x2528b49
  absolute globals written: 
  direct callees: 0xbdd1b0 timer_set_random
```

Disassembly:
```
0093c610  mov al, byte ptr [0x2528b48]
0093c615  cmp al, 1
0093c617  jne 0x93c640
0093c619  mov cl, byte ptr [0x2528b49]
0093c61f  test cl, cl
0093c621  jne 0x93c640
0093c623  mov edx, 0x64
0093c628  mov ecx, 0x7d0
0093c62d  push edi
0093c62e  lea eax, [ecx + edx]
0093c631  sub ecx, edx
0093c633  push ecx
0093c634  mov edi, 0xeb7a80
0093c639  call 0xbdd1b0 <timer_set_random>
0093c63e  pop edi
0093c63f  ret 
0093c640  mov edx, 0x32
0093c645  mov ecx, 0xfa
0093c64a  push edi
0093c64b  lea eax, [ecx + edx]
0093c64e  sub ecx, edx
0093c650  push ecx
0093c651  mov edi, 0xeb7a80
0093c656  call 0xbdd1b0 <timer_set_random>
0093c65b  pop edi
0093c65c  ret 
```

### ambient_spawn_timer_update - 0x93C660

**Convention.** cdecl, no args.

1. Loop `i = 0..5`: if `u8[0x215DA1C + i] == 0` go to step 3 (some gate clear).
2. All six set: if `i32[0xEB7A80] >= 0` (signed `jge`) return; else tail-jump to `ambient_spawn_timer_rearm` (0x93C610).
3. Some gate clear: `i32[0xEB7A80] = -1`.

```
FUNCTION 0x0093C660  true size 0x2C (44 bytes)  insns 11  blocks 7
  entry regs read before write (linear prefix scan): []

  B0 @0x93c660:

  B1 @0x93c662:
    93c669: if (byte ptr [eax + 0x215da1c] == 0) goto B5  else B2   ; je 0x93c681

  B2 @0x93c66b:
    93c671: if (eax < (unsigned) 6) goto B1  else B3   ; jb 0x93c662

  B3 @0x93c673:
    93c673: READ i32[0xeb7a80]   ; cmp dword ptr [0xeb7a80], 0
    93c67a: if (dword ptr [0xeb7a80] >= (signed) 0) goto B6  else B4   ; jge 0x93c68b

  B4 @0x93c67c:
    93c67c: TAIL-JMP 0x93c610 ambient_spawn_timer_rearm  regs[ecx<-(?) eax<-(?)] stack[]

  B5 @0x93c681:
    93c681: WRITE i32[0xeb7a80] mov = 0xFFFFFFFF   ; dword ptr [0xeb7a80], 0xffffffff

  B6 @0x93c68b:
    93c68b: RETURN

  --- summary ---
  absolute globals read: 0xeb7a80
  absolute globals written: 0xeb7a80
  direct callees:
```

Disassembly:
```
0093c660  xor eax, eax
0093c662  cmp byte ptr [eax + 0x215da1c], 0
0093c669  je 0x93c681
0093c66b  add eax, 1
0093c66e  cmp eax, 6
0093c671  jb 0x93c662
0093c673  cmp dword ptr [0xeb7a80], 0
0093c67a  jge 0x93c68b
0093c67c  jmp 0x93c610 <ambient_spawn_timer_rearm>
0093c681  mov dword ptr [0xeb7a80], 0xffffffff
0093c68b  ret 
```

### ambient_vehicle_budget_remaining - 0x93C1D0

**Convention.** cdecl, no args -> EAX.

1. `esi = 0x20 - i32[0x252A0D4]`.
2. If `u8[0x2528B48] == 1 && u8[0x2528B49] == 0` return `esi`.
3. Otherwise call `coop_partner_human` (0x9D7BD0); return `esi` when it returns 0, else `esi - 4`.

```
FUNCTION 0x0093C1D0  true size 0x33 (51 bytes)  insns 19  blocks 6
  entry regs read before write (linear prefix scan): []

  B0 @0x93c1d0:
    93c1d0: READ u8[0x2528b48]   ; mov al, byte ptr [0x2528b48]
    93c1db: READ i32[0x252a0d4]   ; sub esi, dword ptr [0x252a0d4]
    93c1e3: if (al != 1) goto B3  else B1   ; jne 0x93c1f3

  B1 @0x93c1e5:
    93c1e5: READ u8[0x2528b49]   ; mov cl, byte ptr [0x2528b49]
    93c1ed: if (cl != 0) goto B3  else B2   ; jne 0x93c1f3

  B2 @0x93c1ef:
    93c1f2: RETURN

  B3 @0x93c1f3:
    93c1f3: CALL 0x9d7bd0 coop_partner_human  callee-reads[]  regs[]  stack(first arg first)[]  callee-ret-imm=0
    93c1fc: if (eax == 0) goto B5  else B4   ; je 0x93c201

  B4 @0x93c1fe:

  B5 @0x93c201:
    93c202: RETURN

  --- summary ---
  absolute globals read: 0x2528b48, 0x2528b49, 0x252a0d4
  absolute globals written: 
  direct callees: 0x9d7bd0 coop_partner_human
```

Disassembly:
```
0093c1d0  mov al, byte ptr [0x2528b48]
0093c1d5  push esi
0093c1d6  mov esi, 0x20
0093c1db  sub esi, dword ptr [0x252a0d4]
0093c1e1  cmp al, 1
0093c1e3  jne 0x93c1f3
0093c1e5  mov cl, byte ptr [0x2528b49]
0093c1eb  test cl, cl
0093c1ed  jne 0x93c1f3
0093c1ef  mov eax, esi
0093c1f1  pop esi
0093c1f2  ret 
0093c1f3  call 0x9d7bd0 <coop_partner_human>
0093c1f8  test eax, eax
0093c1fa  mov eax, esi
0093c1fc  je 0x93c201
0093c1fe  lea eax, [esi - 4]
0093c201  pop esi
0093c202  ret 
```

### traffic_spawn_gate_open - 0x93D4D0

**Convention.** cdecl, no args -> AL.

1. Loop `i = 0..5`: if `u8[0x215DA0C + i] == 0` return 0.
2. After six set bytes return 1.

```
FUNCTION 0x0093D4D0  true size 0x19 (25 bytes)  insns 10  blocks 5
  entry regs read before write (linear prefix scan): []

  B0 @0x93d4d0:

  B1 @0x93d4d2:
    93d4d9: if (byte ptr [eax + 0x215da0c] == 0) goto B4  else B2   ; je 0x93d4e6

  B2 @0x93d4db:
    93d4e1: if (eax < (unsigned) 6) goto B1  else B3   ; jb 0x93d4d2

  B3 @0x93d4e3:
    93d4e5: RETURN

  B4 @0x93d4e6:
    93d4e8: RETURN

  --- summary ---
  absolute globals read: 
  absolute globals written: 
  direct callees:
```

Disassembly:
```
0093d4d0  xor eax, eax
0093d4d2  cmp byte ptr [eax + 0x215da0c], 0
0093d4d9  je 0x93d4e6
0093d4db  add eax, 1
0093d4de  cmp eax, 6
0093d4e1  jb 0x93d4d2
0093d4e3  mov al, 1
0093d4e5  ret 
0093d4e6  xor al, al
0093d4e8  ret 
```

### traffic_record_find_by_vehicle - 0x938FA0

**Convention.** EDI = vehicle handle; result EAX; no stack args.

1. If `edi == 0` return 0. `head = i32[0x215DA30]`; `p = head`.
2. While `p != 0`: `next = [p]`; `next' = (next == head) ? 0 : next` (the `sub/neg/sbb/and` idiom); if `[p+8] == edi` return `p`; else `p = next'`.
3. Return 0 at the end (ESI saved/restored; EAX/ECX/EDX clobbered).

```
FUNCTION 0x00938FA0  true size 0x2D (45 bytes)  insns 22  blocks 8
  entry regs read before write (linear prefix scan): ['edi']

  B0 @0x938fa0:
    938fa2: if (edi != 0) goto B2  else B1   ; jne 0x938fa7

  B1 @0x938fa4:
    938fa6: RETURN

  B2 @0x938fa7:
    938fa8: READ i32[0x215da30]   ; mov esi, dword ptr [0x215da30]

  B3 @0x938fb0:
    938fb2: if (eax == 0) goto B6  else B4   ; je 0x938fc9

  B4 @0x938fb4:
    938fc3: if (dword ptr [eax + 8] == edi) goto B7  else B5   ; je 0x938fcb

  B5 @0x938fc5:
    938fc7: goto B3 @0x938fb0

  B6 @0x938fc9:

  B7 @0x938fcb:
    938fcc: RETURN

  --- summary ---
  absolute globals read: 0x215da30
  absolute globals written: 
  direct callees:
```

Disassembly:
```
00938fa0  test edi, edi
00938fa2  jne 0x938fa7
00938fa4  xor eax, eax
00938fa6  ret 
00938fa7  push esi
00938fa8  mov esi, dword ptr [0x215da30]
00938fae  mov eax, esi
00938fb0  test eax, eax
00938fb2  je 0x938fc9
00938fb4  mov edx, dword ptr [eax]
00938fb6  mov ecx, edx
00938fb8  sub ecx, esi
00938fba  neg ecx
00938fbc  sbb ecx, ecx
00938fbe  and ecx, edx
00938fc0  cmp dword ptr [eax + 8], edi
00938fc3  je 0x938fcb
00938fc5  mov eax, ecx
00938fc7  jmp 0x938fb0
00938fc9  xor eax, eax
00938fcb  pop esi
00938fcc  ret 
```

### traffic_record_release_by_vehicle - 0x93D910

**Convention.** EDI = vehicle handle; no stack args; returns nothing meaningful.

1. Same ring walk as `traffic_record_find_by_vehicle` over `0x215DA30` (no `edi == 0` early-out here).
2. On a match `pop esi` then `jmp traffic_record_release` with EAX = the record; on no match `pop esi; ret`.

```
FUNCTION 0x0093D910  true size 0x31 (49 bytes)  insns 20  blocks 6
  entry regs read before write (linear prefix scan): []

  B0 @0x93d910:
    93d911: READ i32[0x215da30]   ; mov esi, dword ptr [0x215da30]
    93d919: STACK-OBJECT &[esp+0x0] -> esp

  B1 @0x93d920:
    93d922: if (eax == 0) goto B5  else B2   ; je 0x93d93f

  B2 @0x93d924:
    93d933: if (dword ptr [eax + 8] == edi) goto B4  else B3   ; je 0x93d939

  B3 @0x93d935:
    93d937: goto B1 @0x93d920

  B4 @0x93d939:
    93d93a: TAIL-JMP 0x938cb0 traffic_record_release  regs[ecx<-(?) eax<-(?)] stack[]

  B5 @0x93d93f:
    93d940: RETURN

  --- summary ---
  absolute globals read: 0x215da30
  absolute globals written: 
  direct callees:
```

Disassembly:
```
0093d910  push esi
0093d911  mov esi, dword ptr [0x215da30]
0093d917  mov eax, esi
0093d919  lea esp, [esp]
0093d920  test eax, eax
0093d922  je 0x93d93f
0093d924  mov edx, dword ptr [eax]
0093d926  mov ecx, edx
0093d928  sub ecx, esi
0093d92a  neg ecx
0093d92c  sbb ecx, ecx
0093d92e  and ecx, edx
0093d930  cmp dword ptr [eax + 8], edi
0093d933  je 0x93d939
0093d935  mov eax, ecx
0093d937  jmp 0x93d920
0093d939  pop esi
0093d93a  jmp 0x938cb0 <traffic_record_release>
0093d93f  pop esi
0093d940  ret 
```

### traffic_record_activate - 0x938C30

**Convention.** EAX = record, ECX = flag (bit 0 used); no stack args; no result.

1. `[rec+0x10] = (ecx & 1) | ([rec+0x10] & 0xFFFFFFFC)` (note: bit 1 is cleared).
2. Unlink from the free ring `0x215D7AC`: if it is the head, head = next, or 0 when `next == rec` (alone); then `prev->next = next` if prev, `next->prev = prev` if next (the code uses the unusual null checks `cmp ecx, edx` with edx = 0).
3. `[0x215D7A8] += 1`; clear `[rec]` and `[rec+4]`; insert at the tail of the active ring `0x215DA30`: empty ring -> `rec.next = rec.prev = rec; head = rec`; otherwise `rec.prev = head.prev; rec.next = head; head.prev->next = rec; head.prev = rec`.

```
FUNCTION 0x00938C30  true size 0x79 (121 bytes)  insns 44  blocks 12
  entry regs read before write (linear prefix scan): ['eax', 'ecx']

  B0 @0x938c30:
    938c3b: READ i32[0x215d7ac]   ; cmp dword ptr [0x215d7ac], eax
    938c44: if (dword ptr [0x215d7ac] != eax) goto B4  else B1   ; jne 0x938c5c

  B1 @0x938c46:
    938c4a: if (ecx != eax) goto B3  else B2   ; jne 0x938c56

  B2 @0x938c4c:
    938c4e: WRITE i32[0x215d7ac] mov = edx   ; dword ptr [0x215d7ac], edx
    938c54: goto B9 @0x938c77

  B3 @0x938c56:
    938c56: WRITE i32[0x215d7ac] mov = ecx   ; dword ptr [0x215d7ac], ecx

  B4 @0x938c5c:
    938c64: if (ecx == edx) goto B6  else B5   ; je 0x938c6a

  B5 @0x938c66:

  B6 @0x938c6a:
    938c6e: if (ecx == edx) goto B8  else B7   ; je 0x938c76

  B7 @0x938c70:

  B8 @0x938c76:

  B9 @0x938c77:
    938c77: READ i32[0x215da30]   ; mov ecx, dword ptr [0x215da30]
    938c7d: WRITE i32[0x215d7a8] add   ; dword ptr [0x215d7a8], 1
    938c8b: if (ecx != edx) goto B11  else B10   ; jne 0x938c98

  B10 @0x938c8d:
    938c92: WRITE i32[0x215da30] mov = eax   ; dword ptr [0x215da30], eax
    938c97: RETURN

  B11 @0x938c98:
    938ca8: RETURN

  --- summary ---
  absolute globals read: 0x215d7a8, 0x215d7ac, 0x215da30
  absolute globals written: 0x215d7a8, 0x215d7ac, 0x215da30
  direct callees:
```

Disassembly:
```
00938c30  mov edx, dword ptr [eax + 0x10]
00938c33  and ecx, 1
00938c36  and edx, 0xfffffffc
00938c39  or ecx, edx
00938c3b  cmp dword ptr [0x215d7ac], eax
00938c41  mov dword ptr [eax + 0x10], ecx
00938c44  jne 0x938c5c
00938c46  mov ecx, dword ptr [eax]
00938c48  cmp ecx, eax
00938c4a  jne 0x938c56
00938c4c  xor edx, edx
00938c4e  mov dword ptr [0x215d7ac], edx
00938c54  jmp 0x938c77
00938c56  mov dword ptr [0x215d7ac], ecx
00938c5c  mov ecx, dword ptr [eax + 4]
00938c5f  xor edx, edx
00938c61  cmp ecx, edx
00938c63  push edi
00938c64  je 0x938c6a
00938c66  mov edi, dword ptr [eax]
00938c68  mov dword ptr [ecx], edi
00938c6a  mov ecx, dword ptr [eax]
00938c6c  cmp ecx, edx
00938c6e  je 0x938c76
00938c70  mov edi, dword ptr [eax + 4]
00938c73  mov dword ptr [ecx + 4], edi
00938c76  pop edi
00938c77  mov ecx, dword ptr [0x215da30]
00938c7d  add dword ptr [0x215d7a8], 1
00938c84  cmp ecx, edx
00938c86  mov dword ptr [eax + 4], edx
00938c89  mov dword ptr [eax], edx
00938c8b  jne 0x938c98
00938c8d  mov dword ptr [eax], eax
00938c8f  mov dword ptr [eax + 4], eax
00938c92  mov dword ptr [0x215da30], eax
00938c97  ret 
00938c98  mov edx, dword ptr [ecx + 4]
00938c9b  mov dword ptr [eax + 4], edx
00938c9e  mov dword ptr [eax], ecx
00938ca0  mov edx, dword ptr [ecx + 4]
00938ca3  mov dword ptr [edx], eax
00938ca5  mov dword ptr [ecx + 4], eax
00938ca8  ret 
```

### traffic_record_release - 0x938CB0

**Convention.** EAX = record; no stack args; no result.

1. Unlink from the active ring `0x215DA30` (same pattern as `traffic_record_activate`, head fix-up first when `head == rec`).
2. `[0x215D7A8] -= 1`; clear `[rec]`, `[rec+4]`.
3. Insert at the tail of the free ring `0x215D7AC` (empty -> self-linked head; else before the head).

```
FUNCTION 0x00938CB0  true size 0x6B (107 bytes)  insns 39  blocks 12
  entry regs read before write (linear prefix scan): ['eax']

  B0 @0x938cb0:
    938cb0: READ i32[0x215da30]   ; cmp dword ptr [0x215da30], eax
    938cb6: if (dword ptr [0x215da30] != eax) goto B4  else B1   ; jne 0x938cce

  B1 @0x938cb8:
    938cbc: if (ecx != eax) goto B3  else B2   ; jne 0x938cc8

  B2 @0x938cbe:
    938cc0: WRITE i32[0x215da30] mov = edx   ; dword ptr [0x215da30], edx
    938cc6: goto B9 @0x938ce9

  B3 @0x938cc8:
    938cc8: WRITE i32[0x215da30] mov = ecx   ; dword ptr [0x215da30], ecx

  B4 @0x938cce:
    938cd6: if (ecx == edx) goto B6  else B5   ; je 0x938cdc

  B5 @0x938cd8:

  B6 @0x938cdc:
    938ce0: if (ecx == edx) goto B8  else B7   ; je 0x938ce8

  B7 @0x938ce2:

  B8 @0x938ce8:

  B9 @0x938ce9:
    938ce9: READ i32[0x215d7ac]   ; mov ecx, dword ptr [0x215d7ac]
    938cef: WRITE i32[0x215d7a8] sub   ; dword ptr [0x215d7a8], 1
    938cfd: if (ecx != edx) goto B11  else B10   ; jne 0x938d0a

  B10 @0x938cff:
    938d04: WRITE i32[0x215d7ac] mov = eax   ; dword ptr [0x215d7ac], eax
    938d09: RETURN

  B11 @0x938d0a:
    938d1a: RETURN

  --- summary ---
  absolute globals read: 0x215d7a8, 0x215d7ac, 0x215da30
  absolute globals written: 0x215d7a8, 0x215d7ac, 0x215da30
  direct callees:
```

Disassembly:
```
00938cb0  cmp dword ptr [0x215da30], eax
00938cb6  jne 0x938cce
00938cb8  mov ecx, dword ptr [eax]
00938cba  cmp ecx, eax
00938cbc  jne 0x938cc8
00938cbe  xor edx, edx
00938cc0  mov dword ptr [0x215da30], edx
00938cc6  jmp 0x938ce9
00938cc8  mov dword ptr [0x215da30], ecx
00938cce  mov ecx, dword ptr [eax + 4]
00938cd1  xor edx, edx
00938cd3  cmp ecx, edx
00938cd5  push esi
00938cd6  je 0x938cdc
00938cd8  mov esi, dword ptr [eax]
00938cda  mov dword ptr [ecx], esi
00938cdc  mov ecx, dword ptr [eax]
00938cde  cmp ecx, edx
00938ce0  je 0x938ce8
00938ce2  mov esi, dword ptr [eax + 4]
00938ce5  mov dword ptr [ecx + 4], esi
00938ce8  pop esi
00938ce9  mov ecx, dword ptr [0x215d7ac]
00938cef  sub dword ptr [0x215d7a8], 1
00938cf6  cmp ecx, edx
00938cf8  mov dword ptr [eax + 4], edx
00938cfb  mov dword ptr [eax], edx
00938cfd  jne 0x938d0a
00938cff  mov dword ptr [eax], eax
00938d01  mov dword ptr [eax + 4], eax
00938d04  mov dword ptr [0x215d7ac], eax
00938d09  ret 
00938d0a  mov edx, dword ptr [ecx + 4]
00938d0d  mov dword ptr [eax + 4], edx
00938d10  mov dword ptr [eax], ecx
00938d12  mov edx, dword ptr [ecx + 4]
00938d15  mov dword ptr [edx], eax
00938d17  mov dword ptr [ecx + 4], eax
00938d1a  ret 
```

### current_spawn_group_alloc_member - 0x957000

**Convention.** cdecl, one stack arg (object*) -> EAX (ESI/EDI saved).

1. `g = i32[0x215E5BC]`; if `g == 0` return 0.
2. Find the first `i` in 0..7 with `u8[g + 0x5C + 0x30*i] == 0`; none -> return 0.
3. Set that byte to 1, `i32[g + 0x60 + 0x30*i] = i32[obj + 0x44]`, clear the nine dwords `e+0x68 .. e+0x88` (entry-relative +0x88 is `group+0x30*i+0x88`), `i32[e+0x64] = -1`; return `e = g + 0x5C + 0x30*i`.

```
FUNCTION 0x00957000  true size 0x7A (122 bytes)  insns 40  blocks 6
  entry regs read before write (linear prefix scan): []

  B0 @0x957000:
    957001: READ i32[0x215e5bc]   ; mov esi, dword ptr [0x215e5bc]
    957009: if (esi == 0) goto B4  else B1   ; je 0x957020

  B1 @0x95700b:

  B2 @0x957010:
    957013: if (byte ptr [eax] == 0) goto B5  else B3   ; je 0x957024

  B3 @0x957015:
    95701e: if (edx < (signed) 8) goto B2  else B4   ; jl 0x957010

  B4 @0x957020:
    957023: RETURN

  B5 @0x957024:
    957079: RETURN

  --- summary ---
  absolute globals read: 0x215e5bc
  absolute globals written: 
  direct callees:
```

Disassembly:
```
00957000  push esi
00957001  mov esi, dword ptr [0x215e5bc]
00957007  test esi, esi
00957009  je 0x957020
0095700b  xor edx, edx
0095700d  lea eax, [esi + 0x5c]
00957010  cmp byte ptr [eax], 0
00957013  je 0x957024
00957015  add edx, 1
00957018  add eax, 0x30
0095701b  cmp edx, 8
0095701e  jl 0x957010
00957020  xor eax, eax
00957022  pop esi
00957023  ret 
00957024  lea eax, [edx + edx*2]
00957027  shl eax, 4
0095702a  lea ecx, [eax + esi]
0095702d  mov byte ptr [ecx + 0x5c], 1
00957031  push edi
00957032  mov edi, dword ptr [esp + 0xc]
00957036  mov edi, dword ptr [edi + 0x44]
00957039  lea eax, [ecx + 0x5c]
0095703c  lea edx, [edx + edx*2 + 6]
00957040  mov dword ptr [ecx + 0x88], 0
0095704a  shl edx, 4
0095704d  mov dword ptr [edx + esi], edi
00957050  xor edx, edx
00957052  mov dword ptr [ecx + 0x68], edx
00957055  mov dword ptr [ecx + 0x6c], edx
00957058  mov dword ptr [ecx + 0x70], edx
0095705b  mov dword ptr [ecx + 0x74], edx
0095705e  mov dword ptr [ecx + 0x78], edx
00957061  mov dword ptr [ecx + 0x7c], edx
00957064  mov dword ptr [ecx + 0x80], edx
0095706a  pop edi
0095706b  mov dword ptr [ecx + 0x84], edx
00957071  mov dword ptr [ecx + 0x64], 0xffffffff
00957078  pop esi
00957079  ret 
```

### spawn_kind_to_budget_slot - 0x947690

**Convention.** EAX = kind; result EAX; no stack args.

1. `if (kind > 6 (unsigned)) return 0`.
2. Table (decoded from bytes at 0x9476C0): `{0:2, 1:6, 2:4, 3:5, 4:0, 5:3, 6:3}`.

```
FUNCTION 0x00947690  true size 0x2D (45 bytes)  insns 5  blocks 3
  entry regs read before write (linear prefix scan): ['eax']

  B0 @0x947690:
    947693: if (eax > (unsigned) 6) goto B2  else B1   ; ja 0x9476ba

  B1 @0x947695:
    947695: SWITCH dword ptr [eax*4 + 0x9476c0] table 0x9476c0, 0 entries: 

  B2 @0x9476ba:
    9476bc: RETURN

  --- summary ---
  absolute globals read: 
  absolute globals written: 
  direct callees:
```

Disassembly:
```
00947690  cmp eax, 6
00947693  ja 0x9476ba
00947695  jmp dword ptr [eax*4 + 0x9476c0]
009476ba  xor eax, eax
009476bc  ret 
```

### spawner_add_category - 0x93EFA0

**Convention.** EAX = spawner, ECX = category; no stack args; no result.

1. Scan `spawner.list[0 .. [eax+0xC])` (dwords at +0x10): if `list[j] == ecx` return.
2. For `k = 0 .. [ecx+0x1C]` (unsigned): `if [[ecx+0x20] + 4k] == [eax]`: if `[eax+0xC] >= 0x14` (signed `jge`) return; else `list[count++] = ecx`; and if `[ecx+0x28] != -1` set `[eax+8] = ecx`. The loop continues after a match (it does not break).

```
FUNCTION 0x0093EFA0  true size 0x57 (87 bytes)  insns 35  blocks 13
  entry regs read before write (linear prefix scan): ['eax']

  B0 @0x93efa0:
    93efa6: if (dword ptr [eax + 0xc] <= (signed) edx) goto B4  else B1   ; jle 0x93efbf

  B1 @0x93efa8:
    93efab: goto B2 @0x93efb0

  B2 @0x93efb0:
    93efb2: if (ecx == dword ptr [esi]) goto B12  else B3   ; je 0x93eff5

  B3 @0x93efb4:
    93efbd: if (edx < (signed) dword ptr [eax + 0xc]) goto B2  else B4   ; jl 0x93efb0

  B4 @0x93efbf:
    93efc4: if (dword ptr [ecx + 0x1c] <= (unsigned) esi) goto B12  else B5   ; jbe 0x93eff5

  B5 @0x93efc6:

  B6 @0x93efc7:
    93efcf: if (edx != dword ptr [eax]) goto B10  else B7   ; jne 0x93efec

  B7 @0x93efd1:
    93efd7: if (edx >= (signed) 0x14) goto B11  else B8   ; jge 0x93eff4

  B8 @0x93efd9:
    93efe7: if (dword ptr [ecx + 0x28] == -1) goto B10  else B9   ; je 0x93efec

  B9 @0x93efe9:

  B10 @0x93efec:
    93eff2: if (esi < (unsigned) dword ptr [ecx + 0x1c]) goto B6  else B11   ; jb 0x93efc7

  B11 @0x93eff4:

  B12 @0x93eff5:
    93eff6: RETURN

  --- summary ---
  absolute globals read: 
  absolute globals written: 
  direct callees:
```

Disassembly:
```
0093efa0  xor edx, edx
0093efa2  cmp dword ptr [eax + 0xc], edx
0093efa5  push esi
0093efa6  jle 0x93efbf
0093efa8  lea esi, [eax + 0x10]
0093efab  jmp 0x93efb0
0093efb0  cmp ecx, dword ptr [esi]
0093efb2  je 0x93eff5
0093efb4  add edx, 1
0093efb7  add esi, 4
0093efba  cmp edx, dword ptr [eax + 0xc]
0093efbd  jl 0x93efb0
0093efbf  xor esi, esi
0093efc1  cmp dword ptr [ecx + 0x1c], esi
0093efc4  jbe 0x93eff5
0093efc6  push ebx
0093efc7  mov edx, dword ptr [ecx + 0x20]
0093efca  mov edx, dword ptr [edx + esi*4]
0093efcd  cmp edx, dword ptr [eax]
0093efcf  jne 0x93efec
0093efd1  mov edx, dword ptr [eax + 0xc]
0093efd4  cmp edx, 0x14
0093efd7  jge 0x93eff4
0093efd9  lea ebx, [edx + 1]
0093efdc  mov dword ptr [eax + 0xc], ebx
0093efdf  mov dword ptr [eax + edx*4 + 0x10], ecx
0093efe3  cmp dword ptr [ecx + 0x28], -1
0093efe7  je 0x93efec
0093efe9  mov dword ptr [eax + 8], ecx
0093efec  add esi, 1
0093efef  cmp esi, dword ptr [ecx + 0x1c]
0093eff2  jb 0x93efc7
0093eff4  pop ebx
0093eff5  pop esi
0093eff6  ret 
```

### spawn_stream_budget_publish - 0x9473B0

**Convention.** cdecl, no args.

1. For the nine words `w[i] = i32[0xE887BC + 4i]` and `blk = i32[0x2F9B3B4]`: if `w[i] != blk[i]` then `u8[0x2F9B3B8] |= 2; blk[i] = w[i]`; the block pointer is re-read after every store.
2. Then four singles: `0xE887E0 -> [0x2F9B488]` (dirty `0x2F9B48C`), `0xE887E4 -> [0x2F9B55C]` (`0x2F9B560`), `0xE887E8 -> [0x2F9B630]` (`0x2F9B634`), `0xE887EC -> [0x2F9B704]` (`0x2F9B708`), each guarded by the same compare.

```
FUNCTION 0x009473B0  true size 0x13E (318 bytes)  insns 80  blocks 27
  entry regs read before write (linear prefix scan): []

  B0 @0x9473b0:
    9473b0: READ i32[0xe887bc]   ; mov ecx, dword ptr [0xe887bc]
    9473b6: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]
    9473bf: if (ecx == dword ptr [eax]) goto B2  else B1   ; je 0x9473ce

  B1 @0x9473c1:
    9473c1: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    9473c9: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B2 @0x9473ce:
    9473ce: READ i32[0xe887c0]   ; mov ecx, dword ptr [0xe887c0]
    9473d7: if (ecx == dword ptr [eax + 4]) goto B4  else B3   ; je 0x9473e7

  B3 @0x9473d9:
    9473d9: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    9473e2: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B4 @0x9473e7:
    9473e7: READ i32[0xe887c4]   ; mov ecx, dword ptr [0xe887c4]
    9473f0: if (ecx == dword ptr [eax + 8]) goto B6  else B5   ; je 0x947400

  B5 @0x9473f2:
    9473f2: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    9473fb: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B6 @0x947400:
    947400: READ i32[0xe887c8]   ; mov ecx, dword ptr [0xe887c8]
    947409: if (ecx == dword ptr [eax + 0xc]) goto B8  else B7   ; je 0x947419

  B7 @0x94740b:
    94740b: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    947414: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B8 @0x947419:
    947419: READ i32[0xe887cc]   ; mov ecx, dword ptr [0xe887cc]
    947422: if (ecx == dword ptr [eax + 0x10]) goto B10  else B9   ; je 0x947432

  B9 @0x947424:
    947424: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    94742d: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B10 @0x947432:
    947432: READ i32[0xe887d0]   ; mov ecx, dword ptr [0xe887d0]
    94743b: if (ecx == dword ptr [eax + 0x14]) goto B12  else B11   ; je 0x94744b

  B11 @0x94743d:
    94743d: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    947446: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B12 @0x94744b:
    94744b: READ i32[0xe887d4]   ; mov ecx, dword ptr [0xe887d4]
    947454: if (ecx == dword ptr [eax + 0x18]) goto B14  else B13   ; je 0x947464

  B13 @0x947456:
    947456: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    94745f: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B14 @0x947464:
    947464: READ i32[0xe887d8]   ; mov ecx, dword ptr [0xe887d8]
    94746d: if (ecx == dword ptr [eax + 0x1c]) goto B16  else B15   ; je 0x94747d

  B15 @0x94746f:
    94746f: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl
    947478: READ i32[0x2f9b3b4]   ; mov eax, dword ptr [0x2f9b3b4]

  B16 @0x94747d:
    94747d: READ i32[0xe887dc]   ; mov ecx, dword ptr [0xe887dc]
    947486: if (ecx == dword ptr [eax + 0x20]) goto B18  else B17   ; je 0x947491

  B17 @0x947488:
    947488: WRITE u8[0x2f9b3b8] or   ; byte ptr [0x2f9b3b8], dl

  B18 @0x947491:
    947491: READ i32[0xe887e0]   ; mov eax, dword ptr [0xe887e0]
    947496: READ i32[0x2f9b488]   ; mov ecx, dword ptr [0x2f9b488]
    94749e: if (eax == dword ptr [ecx]) goto B20  else B19   ; je 0x9474a8

  B19 @0x9474a0:
    9474a0: WRITE u8[0x2f9b48c] or   ; byte ptr [0x2f9b48c], dl

  B20 @0x9474a8:
    9474a8: READ i32[0xe887e4]   ; mov eax, dword ptr [0xe887e4]
    9474ad: READ i32[0x2f9b55c]   ; mov ecx, dword ptr [0x2f9b55c]
    9474b5: if (eax == dword ptr [ecx]) goto B22  else B21   ; je 0x9474bf

  B21 @0x9474b7:
    9474b7: WRITE u8[0x2f9b560] or   ; byte ptr [0x2f9b560], dl

  B22 @0x9474bf:
    9474bf: READ i32[0xe887e8]   ; mov eax, dword ptr [0xe887e8]
    9474c4: READ i32[0x2f9b630]   ; mov ecx, dword ptr [0x2f9b630]
    9474cc: if (eax == dword ptr [ecx]) goto B24  else B23   ; je 0x9474d6

  B23 @0x9474ce:
    9474ce: WRITE u8[0x2f9b634] or   ; byte ptr [0x2f9b634], dl

  B24 @0x9474d6:
    9474d6: READ i32[0xe887ec]   ; mov eax, dword ptr [0xe887ec]
    9474db: READ i32[0x2f9b704]   ; mov ecx, dword ptr [0x2f9b704]
    9474e3: if (eax == dword ptr [ecx]) goto B26  else B25   ; je 0x9474ed

  B25 @0x9474e5:
    9474e5: WRITE u8[0x2f9b708] or   ; byte ptr [0x2f9b708], dl

  B26 @0x9474ed:
    9474ed: RETURN

  --- summary ---
  absolute globals read: 0xe887bc, 0xe887c0, 0xe887c4, 0xe887c8, 0xe887cc, 0xe887d0, 0xe887d4, 0xe887d8, 0xe887dc, 0xe887e0, 0xe887e4, 0xe887e8, 0xe887ec, 0x2f9b3b4, 0x2f9b3b8, 0x2f9b488, 0x2f9b48c, 0x2f9b55c, 0x2f9b560, 0x2f9b630, 0x2f9b634, 0x2f9b704, 0x2f9b708
  absolute globals written: 0x2f9b3b8, 0x2f9b48c, 0x2f9b560, 0x2f9b634, 0x2f9b708
  direct callees:
```

Disassembly:
```
009473b0  mov ecx, dword ptr [0xe887bc]
009473b6  mov eax, dword ptr [0x2f9b3b4]
009473bb  cmp ecx, dword ptr [eax]
009473bd  mov dl, 2
009473bf  je 0x9473ce
009473c1  or byte ptr [0x2f9b3b8], dl
009473c7  mov dword ptr [eax], ecx
009473c9  mov eax, dword ptr [0x2f9b3b4]
009473ce  mov ecx, dword ptr [0xe887c0]
009473d4  cmp ecx, dword ptr [eax + 4]
009473d7  je 0x9473e7
009473d9  or byte ptr [0x2f9b3b8], dl
009473df  mov dword ptr [eax + 4], ecx
009473e2  mov eax, dword ptr [0x2f9b3b4]
009473e7  mov ecx, dword ptr [0xe887c4]
009473ed  cmp ecx, dword ptr [eax + 8]
009473f0  je 0x947400
009473f2  or byte ptr [0x2f9b3b8], dl
009473f8  mov dword ptr [eax + 8], ecx
009473fb  mov eax, dword ptr [0x2f9b3b4]
00947400  mov ecx, dword ptr [0xe887c8]
00947406  cmp ecx, dword ptr [eax + 0xc]
00947409  je 0x947419
0094740b  or byte ptr [0x2f9b3b8], dl
00947411  mov dword ptr [eax + 0xc], ecx
00947414  mov eax, dword ptr [0x2f9b3b4]
00947419  mov ecx, dword ptr [0xe887cc]
0094741f  cmp ecx, dword ptr [eax + 0x10]
00947422  je 0x947432
00947424  or byte ptr [0x2f9b3b8], dl
0094742a  mov dword ptr [eax + 0x10], ecx
0094742d  mov eax, dword ptr [0x2f9b3b4]
00947432  mov ecx, dword ptr [0xe887d0]
00947438  cmp ecx, dword ptr [eax + 0x14]
0094743b  je 0x94744b
0094743d  or byte ptr [0x2f9b3b8], dl
00947443  mov dword ptr [eax + 0x14], ecx
00947446  mov eax, dword ptr [0x2f9b3b4]
0094744b  mov ecx, dword ptr [0xe887d4]
00947451  cmp ecx, dword ptr [eax + 0x18]
00947454  je 0x947464
00947456  or byte ptr [0x2f9b3b8], dl
0094745c  mov dword ptr [eax + 0x18], ecx
0094745f  mov eax, dword ptr [0x2f9b3b4]
00947464  mov ecx, dword ptr [0xe887d8]
0094746a  cmp ecx, dword ptr [eax + 0x1c]
0094746d  je 0x94747d
0094746f  or byte ptr [0x2f9b3b8], dl
00947475  mov dword ptr [eax + 0x1c], ecx
00947478  mov eax, dword ptr [0x2f9b3b4]
0094747d  mov ecx, dword ptr [0xe887dc]
00947483  cmp ecx, dword ptr [eax + 0x20]
00947486  je 0x947491
00947488  or byte ptr [0x2f9b3b8], dl
0094748e  mov dword ptr [eax + 0x20], ecx
00947491  mov eax, dword ptr [0xe887e0]
00947496  mov ecx, dword ptr [0x2f9b488]
0094749c  cmp eax, dword ptr [ecx]
0094749e  je 0x9474a8
009474a0  or byte ptr [0x2f9b48c], dl
009474a6  mov dword ptr [ecx], eax
009474a8  mov eax, dword ptr [0xe887e4]
009474ad  mov ecx, dword ptr [0x2f9b55c]
009474b3  cmp eax, dword ptr [ecx]
009474b5  je 0x9474bf
009474b7  or byte ptr [0x2f9b560], dl
009474bd  mov dword ptr [ecx], eax
009474bf  mov eax, dword ptr [0xe887e8]
009474c4  mov ecx, dword ptr [0x2f9b630]
009474ca  cmp eax, dword ptr [ecx]
009474cc  je 0x9474d6
009474ce  or byte ptr [0x2f9b634], dl
009474d4  mov dword ptr [ecx], eax
009474d6  mov eax, dword ptr [0xe887ec]
009474db  mov ecx, dword ptr [0x2f9b704]
009474e1  cmp eax, dword ptr [ecx]
009474e3  je 0x9474ed
009474e5  or byte ptr [0x2f9b708], dl
009474eb  mov dword ptr [ecx], eax
009474ed  ret 
```

### notoriety_spawn_group::get_scale_factor - 0x923D00

**Convention.** ECX = notoriety group; result in ST0 (f32).

1. `eax = notoriety_level_for_faction([ecx+0x1E4])` (0x55F670); `eax -= 2`; if `eax > 3 (unsigned)` return 1.0 (`fld1`).
2. Jump table at 0x923D3C: 0->0x923D1A (f32 at 0xE87628 = 1.2), 1->0x923D21 (0xE88150 = 1.4), 2->0x923D28 (0xE5CB28 = 1.6), 3->0x923D2F (0xE88364 = 1.8); each does `fld dword ptr [...]; ret`.

```
FUNCTION 0x00923D00  true size 0x39 (57 bytes)  insns 8  blocks 3
  entry regs read before write (linear prefix scan): ['ecx']

  B0 @0x923d00:
    923d06: CALL 0x55f670 notoriety_level_for_faction  callee-reads[eax]  regs[eax<-(mov eax, dword ptr [ecx + 0x1e4])]  stack(first arg first)[]  callee-ret-imm=0
    923d11: if (eax > (unsigned) 3) goto B2  else B1   ; ja 0x923d36

  B1 @0x923d13:
    923d13: SWITCH dword ptr [eax*4 + 0x923d3c] table 0x923d3c, 0 entries: 

  B2 @0x923d36:
    923d38: RETURN

  --- summary ---
  absolute globals read: 
  absolute globals written: 
  direct callees: 0x55f670 notoriety_level_for_faction
```

Disassembly:
```
00923d00  mov eax, dword ptr [ecx + 0x1e4]
00923d06  call 0x55f670 <notoriety_level_for_faction>
00923d0b  add eax, -2
00923d0e  cmp eax, 3
00923d11  ja 0x923d36
00923d13  jmp dword ptr [eax*4 + 0x923d3c]
00923d36  fld1 
00923d38  ret 
```

## Other functions worth a recipe or port later

- `spawn_node_acquire_ped` / `spawn_node_acquire_vehicle` (0x940AB0 / 0x940B40) and the pool helpers `pool_alloc_node_edi` / `pool_free_node_edi_esi`: small, register-driven, good first ports once the pool layout (`+0x14` count, `+0x20` enable, `+0x30` body) is decoded.
- `entity_budget_can_add` (0x929540), `entity_budget_free_slots` (0x9290F0): pure arithmetic over the `0x2BA4BC8` rows and the `0xE889F4` limit table.
- `spawn_stream_budget_compute` (0x946AC0): about 0x1A7 bytes, integer only, nine words, but it calls the unnamed 0x946880.
- `notoriety_level_for_faction` (0x55F670, 30 callers): jump table over six faction records at `0x279AA60` (stride 0x3C); a prerequisite for porting every notoriety spawn group slot.
- The unported spawn group slot bodies: `is_eligible_to_spawn` of the police / gang / heli groups (0x9240B0, 0x924370, 0x923E30) depend on the timers 0xEB6680 / 0xEB6940 / 0xEB7998 and `timer_elapsed_ms`.

## Not covered

- The train system under `population_update` is only named at its top (`el_train_spawn`, `el_trains_update_one`, `el_train_update`, `el_train_place_cars_on_spline`); the spline car code at 0x958550 and 0x959760 and the constraint code at 0x959300 are left for a later round.
- The activity-owned vtable placeholders (`vandalism_info`, `vandalism_level_instance`, `escort_info`, `escort_instance`, `escort_level_instance`) belong to the activities round.
- Projectile code at 0x94A7B0..0x94E6E0 shares the neighbourhood of the spawn code but is a different system (list `0x215E07C`).
