# 37: Second caller survey (in-game)

Names are in `docs/vtable_slots.csv` (applied to every subclass by `tools/harvest_names.py`) and `docs/functions.csv` (notes ending "(37)" or "(caller survey, 37)").

## Runs

Both runs used the caller-survey build of the hook (`--features caller-survey`). It wraps 5,222 unnamed vtable slots of game classes and logs the return address of each call.

| Run | Script | What it covered |
|---|---|---|
| `explore1` | `tools/ingame/run_explore.ps1 -Mode foot` | Continue into the save, then a walk / action sequence in the crib. Pressing E opened crib customization, then the phone, map and pause tabs |
| `explore2` | `run_explore.ps1 -Mode vehicle`, built with `TELEPORT_TEST` | the teleport loop (8 jumps 600 m apart through the city), then on-foot actions, the phone and the map |

The logs are `bin/explore1.callers.log` and `bin/explore2.callers.log`; the reports come from `tools/caller_report.py`.

**Coverage.** 346 surveyed methods were reached. Only 12 weren't already reached by the first survey (`callers1`). About 320 were reached before but never named, so the logs, not game time, are now the limit.

## Named this round

| Family | Slots (on the base class) | Subclass methods named |
|---|---|---|
| `vehicle_light_manager` (head / brake / reverse / turn signal / siren / running / aero lights) | 0 init, 1 reset, 2 update, 3 update_transform, 6 apply_state, 7 pre_render, 8 render, 11 update_time_of_day, 12 find_tags, 13 bind_tag, 19 tag_prefix | 30 |
| `special_vehicle_spawn_group` (notoriety police / heli / gang, escort, cell, skirmish, vandalism news) | 7 on_table_loaded, 8 reset, 9 update_streaming, 10 update, 11 is_active | 26 |
| `object` | 2 get_world_bbox, 3 get_local_bbox | 26 |
| `multi_core_object_registration` | 7 on_net_tick | 10 |
| `diversion_type_info` | 21 on_human_killed | via harvest |

How the slots were read:
- **Vehicle lights.**
  - The per-vehicle setup 0xAD7EC0 calls slot 0 once per vehicle.
  - The per-frame lights update 0xAD81B0 calls slot 2.
  - The render-side loops 0xAD8880 / 0xAD89A0 call slots 7 and 8.
  - The time-of-day light pass calls slot 11.
  - Slot 12 compares mesh tag names (`strncmp`) with the subclass's slot 19 string.
- **Spawn groups.** The spawn manager 0x957250 calls slots 9 and 10 every frame on every group. Slot 7 runs once after `xtbl_load_notoriety_spawn`. Slot 8 resets, and subclasses chain to the base.
- **Objects.** Slot 3 copies the local bounds (min, max) and slot 2 transforms them by the position and orientation.

Game events, next to the three known ones (`game_event_object_killed` 11, `vehicle_destroyed` 12, `object_damaged` 14):
- `game_event_player_joined` (0x68F680) and `game_event_player_left` (0x68F6B0), from the player's multiplayer registration;
- `game_event_human_killed` (0x68F6F0): diversions slot 21 and the co-op kill scoring;
- `game_event_player_respawned` (0x68FA30).

## Accessors read in full

After reading the bodies of the reached accessors, these slots are named too (120 methods across the subclasses):

| Family | Slots |
|---|---|
| `rl_renderable_base_inst` (meshes, particles, decals, water, trees, plates, distant peds) | 3 compute_pass_data, 4 prepare_pass (pass index; meshes keep the result at +0x1E8 + pass*20), 6 is_ready, 7 get_env_handle (picks the environment texture through 0x5336E0), 8 min_plane_distance (FLT_MAX-seeded minimum of dot(n, p) + d) |
| `deltacomp_state_data_impl<>` (every replicated object's state data) | 0 clear_dirty_mask (the u16 field mask after the data), 1 get_data (this+4) |
| `vint_object_base` | 1 init (from the object setup 0xB971E0) |
| `object` | 0 get_attach_point (vec3 *out, index): human bone position / vehicle point table at +0x560 |

Containers forward slots 4-8 to their children, which is how the slots' types were read:
- slot 4 calls every child;
- slot 6 is true only if every child is;
- slot 7 returns the first non-zero child;
- slot 8 is the minimum over the children.

## Third pass: LOD, shadows, movers, script objects, game events

**Meshes.**
- `rl_renderable_base_inst` slot 5 `render_shadow` (pass, ctx).
- `rl_mesh_inst` slots 9-12: `get_lod_entry`, `select_lod_entry`, `get_shadow_lod_entry`, `select_shadow_lod_entry`. These are 16-byte entries of the mesh's render table (`[+0x70]+0x20`) and shadow table (`+0x24`).
- `rl_lod_mesh_inst` slots 13 `select_lod` (cached at +0x348, override +0x349) and 14 `get_lod`.

**Movers.** `object_mover` slot 15 `take_damage` (raises the damage / killed events) and slot 16 `on_destroyed`.

**Script objects** (`script_streamable_object`, which covers items, NPCs, movers and the rest): slot 10 `on_object_spawned`, slot 11 `is_spawned`, slot 13 `get_object`.

**Game events.** The game-event block 0x68F650-0x68FB10 has one small function per event. Each calls the game mode's callback at `0xE96C10 + mode × 0x6C + 4 × n`, so the **event number n is the callback offset / 4**. Some also call a diversion dispatcher.

| Event | Function | Diversion slot |
|---|---|---|
| 3 | `game_event_3` 0x68F650 | - |
| 5 | `game_event_5` 0x68F800 (from 0x969790, near the hit-point code) | 24 |
| 8 / 9 | `game_event_player_joined` / `_left` | - / 0x604940 |
| 10 | `game_event_human_killed` | 21 |
| 11 / 12 / 14 | object killed / vehicle destroyed / object damaged | 22 / 23 / 25 |
| 15 | `game_event_15` 0x68F8C0 (from 0xA98070, vehicle code that reads human hit points) | 26 |
| 16 | `game_event_16` 0x68FA00 | - |
| 17 | `game_event_player_respawned` | - |
| 18 | `game_event_18` 0x68FAE0 | 33 |
| 20 / 21 / 22 | `game_event_20` / `_21` / `_22` | 27 / 28 / 29 |
| 23-26 | `game_event_23` … `_26` | - |

The unnamed events have no direct callers in the binary: they're reached through jumps or pointers. Their meaning needs a trace. The diversion slots are named `on_game_event_<n>` to match.

## Reached but left unnamed

Mostly small accessors whose callers don't say what they return:
- `object_mover` slots 9, 10, 12-14 (physics body helpers);
- `script_streamable_object` slots 0 and 12, which store or return the same fields as their neighbours;
- the `pc_multi_session` methods (one call each, at session start);
- the meaning of game events 3, 5, 15, 16, 18 and 20-26.

These need either reading each body in context or a targeted in-game trace.
