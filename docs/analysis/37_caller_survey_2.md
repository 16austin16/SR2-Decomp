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

## Reached but left unnamed

Mostly small accessors whose callers don't say what they return:
- the render instance slots 3-8 (`rl_*` / `r_*`; slot 7 is a one-line getter, slot 8 a cull / sort helper);
- the `deltacomp_state_data_impl<>` slots 0 / 1 (one-line accessors used by `capture_snapshot`);
- the vint object slot 1 (0xB971E0);
- the `pc_multi_session` methods (one call each, at session start);
- several `script_*` and diversion event slots (26-29, 33) whose game events have no telling caller yet.

These need either reading each body in context or a targeted in-game trace.
