# 40 — The frame loop as a porting map

Every direct call made by the four frame-loop functions, in call order, with what each one is today:

- **Rust:** the function itself is ported in `crates/sr2-hook` (a `PORTS` entry, or `rust` in `functions.csv`).
- **mapped:** named in `functions.csv`; the Doc column says where it is described.
- **unmapped:** not named yet.

The frame is `game_main_thread_proc` → `game_frame_tick` (0x520450), once per frame, returning 1 to quit:

1. `game_frame_tick` runs input, pause, timers, saves and networking, then
2. `game_state_frame` (0x697780), which in gameplay reaches `sim_frame` (0x68CA70) through `game_state_dispatch`, then
3. `frame_end_handoff` (0x521A00), which hands the frame to the render thread.

## How to read the tables

- **Profiler section** is the label of the `profile_start` marker the call sits under (the game's own grouping, e.g. `sim_frame:1c`).
- **Reach** is the share of the mission 1 game-thread samples (39k, `bin/mission01_tss01.log`) in functions reachable from the call within 4 levels. It is an upper bound: shared helpers count under every caller, and some subtrees reach debug-draw code that never runs. Use it to rank, not to add up.
- **Rust below** counts ported functions reachable within 4 levels, with their modules. A mapped call with Rust below can usually be ported as a thin caller first.
- Calls are listed once, at their first site. `profile_start` / `profile_stop` are left out.

## Summary

| | Calls | Rust | mapped | unmapped |
|---|---|---|---|---|
| all four functions | 204 | 5 | 176 | 23 |

- `game_frame_tick` and `game_state_frame` are almost fully mapped. They are short call sequences, so their bodies can be ported as Rust skeletons now, calling the original functions by address.
- `frame_end_handoff` is mapped except for two groups:
  - the time-of-day lighting block 0x4D9DD0..0x4E69D0, which reaches about 1% of samples;
  - the render snapshot helpers (0x6255A0, 0x46CCD0, 0xD08310, 0x51A4A0, 0xBE4170).

  It is the next skeleton to port. `render_thread_wait_frame` and the render-list swap are the parts that need care, because both threads touch them.
- `sim_frame` is the large one: 144 calls, 16 of them still unmapped (after the naming rounds up to the item-4 commits). The remaining ones are small (under 0.3% reach) except 0x9A9CE0 and 0x55BA40; see "Sections :1c, :4 and :5". The unmapped calls with the largest reach are the next naming targets:
  - **0x5053B0** (17.4%): now `effect_objects_do_frame`. It runs `effect_objects_budget_bodies` (Havok bodies for the 50 most important effect-object parts), then `effect_object_update` on each object.
  - **0x6A5BB0** (11.1%): now `mission_fail_flow_do_frame`. It switches on the failure reason 0x1450380 (doc 22) and handles fades, revive / teleport, `cutscene_play`, `mission_end` and the stronghold cancel dialog.
  - **0x93D150** (10.9%): now `world_spawn_do_frame` (spawn regions, spawn groups, traffic). It runs every frame; only with 0x2528B48 set and 0x2528B49 clear does it wait for 0xE96C00 >= 0xB first.
  - **0x9D7F90** (10.5%): now `humans_update_all`, which runs `human_update` (0x9D6D60) for every human on the list 0x217043C.
  - Now named: **0xB7CA80** `distant_peds_update` (7.5%), **0x4CF840** `weather_rain_effects_update` (7.5%), **0xA6C350** `triggers_do_frame` (6.9%), **0x4D9F20** `world_lighting_update` (6.1%), **0x9DF4C0** `world_focus_point_update` (6.0%).
  - **0x437160** (5.9%): now `ai_global_do_frame`. It runs 14 global AI updates (not 17) in a fixed order: `ai_order_counters_rebase`, `vehicle_seat_reservations_expire`, the three `ai_spot_users_*`, `ai_pairs_expire`, `ai_groups_release_finished`, `ai_nearby_humans_scan`, `ai_round_robin_advance`, `ai_object_interaction_update`, `ai_human_list_prune`, `ai_vehicle_assignments_prune`, `ai_vehicle_records_prune` and `ai_timesliced_update` (4 AI humans per frame).
  - Most of these roles are inferred from the lists they keep (see each row's note in `functions.csv`).

### `game_frame_tick` (0x520450), Rust (`frame.rs`)

| Profiler section | Call | Status | Reach | Rust below | Doc |
|---|---|---|---|---|---|
| simfr:1,2,3,4,5,6 | `string_crc32_lower` 0xBDC9B0 | mapped | 0.1% |  | 40 |
| simfr:1,2,3,4,5,6 | `script_thread_create` 0xCE01E0 | mapped | 1.2% |  | 09 |
| simfr:1,2,3,4,5,6 | `script_thread_resume` 0xCE0570 | mapped | 1.2% |  | 09 |
| simfr:1,2,3,4,5,6 | — 0x699C90 | unmapped | 0.3% |  |  |
| simfr:1,2,3,4,5,6 | `input_do_frame` 0xC063D0 | Rust | 1.1% | 9 (input) |  |
| simfr:1,2,3,4,5,6 | `vint_input_do_frame` 0xB99550 | mapped | 0.0% | 2 (input) | 06 |
| simfr:1,2,3,4,5,6 | `player_controls_update` 0x4FE760 | mapped | 0.0% |  | 14, 19 |
| simfr:1,2,3,4,5,6 | `input_direction_history_update` 0x500CA0 | mapped | 1.1% |  | 14 |
| simfr:1,2,3,4,5,6 | `mouse_update` 0xC1F320 | Rust | 0.0% |  |  |
| simfr:1,2,3,4,5,6 | `dialog_box_do_frame` 0x753AA0 | mapped | 0.7% |  | 40 |
| simfr:1,2,3,4,5,6 | `game_pause_do_frame` 0x69B160 | mapped | 2.4% | 16 (gfx_cmd, render) | 40 |
| simfr:1,2,3,4,5,6 | `frame_timer_update` 0xBF0710 | mapped | 0.0% |  | 11 |
| simfr:1,2,3,4,5,6 | — 0x8217F0 | unmapped | 0.1% | 1 (mempool) |  |
| simfr:1,2,3,4,5,6 | — 0x809640 | unmapped | 1.7% |  |  |
| simfr:1,2,3,4,5,6 | `profile_frame` 0x691430 | mapped | 1.0% | 2 (mempool) | 13 |
| simfr:1,2,3,4,5,6 | — 0x69A440 | unmapped | 1.8% |  |  |
| simfr:1,2,3,4,5,6 | **`game_state_frame`** (0x697780) | see below | | | |
| sim-hand | **`frame_end_handoff`** (0x521A00) | see below | | | |

### `game_state_frame` (0x697780), Rust (`frame.rs`)

| Profiler section | Call | Status | Reach | Rust below | Doc |
|---|---|---|---|---|---|
|  | `game_state_exit` 0x6973E0 | mapped | 5.3% | 35 (gfx_cmd, hddio, mempool, rsched, streamman) | 11 |
|  | `game_state_apply` 0x697550 | mapped | 2.1% | 14 (gfx_cmd, hddio, mempool, streamman) | 11 |
|  | — 0xB8A760 | unmapped | 0.1% |  |  |
|  | `game_state_enter` 0x697330 | mapped | 0.0% |  | 11 |
|  | `game_state_stack_dispatch` 0x697700 | Rust | 11.4% | 14 (gfx_cmd, mempool, streamman) | 40 |
|  | `game_state_dispatch` 0x697490 | mapped | 20.8% | 50 (gfx_cmd, hddio, mempool, streamman) | 06 |

### `sim_frame` (0x68CA70)

| Profiler section | Call | Status | Reach | Rust below | Doc |
|---|---|---|---|---|---|
| sim_frame:1,2,3 | `cutscene_do_frame` 0x6C6870 | mapped | 5.0% | 57 (gfx_cmd, hddio, rsched, streamman) | 23, 40 |
| sim_frame:1,2,3 | `camera_pre_update` 0x493260 | mapped | 0.1% |  | 19, 40 |
| sim_frame:1,2,3 | `camera_update` 0x4932F0 | mapped | 8.8% | 2 (gfx_cmd) | 19 |
| sim_frame:1,2,3 | `trigger_objects_reap` 0xA70B40 | mapped | 0.4% | 1 (rsched) | 09, 40 |
| sim_frame:1,2,3 | `script_deferred_calls_flush` 0xA55310 | mapped | 1.2% |  | 09, 40 |
| sim_frame:1,2,3 | `cellphone_do_frame` 0xA66340 | mapped | 1.1% |  | 28, 40 |
| sim_frame:1,2,3 | `race_hud_do_frame` 0x543920 | mapped | 0.5% | 8 (gfx_cmd) | 29, 40 |
| sim_frame:1,2,3 | `game_mode_do_frame` 0x68F4B0 | mapped | 2.7% | 2 (gfx_cmd) | 40 |
| sim_frame:1,2,3 | `world_spawn_do_frame` 0x93D150 | mapped | 10.9% | 10 (rsched, streamman) | 40 |
| sim_frame:1,2,3 | `script_do_frame` 0xCDDE70 | mapped | 0.4% |  | 09 |
| sim_frame:1,2,3 | — 0x6DA350 | unmapped | 0.0% |  |  |
| sim_frame:1,2,3 | `wiretap_triggers_update` 0x54CCA0 | mapped | 0.7% |  | 22, 40 |
| sim_frame:1,2,3 | `tutorial_do_frame` 0x6B7020 | mapped | 0.6% | 2 | 30, 40 |
| sim_frame:1,2,3 | — 0x6905E0 | unmapped | 0.1% |  |  |
| sim_frame:1,2,3 | `controller_disconnect_check` 0x75A120 | mapped | 0.9% | 10 (gfx_cmd) | 14, 40 |
| sim_frame:1,2,3 | `input_context_do_frame` 0x4F5040 | mapped | 0.0% |  | 14 |
| sim_frame:1,2,3 | — 0x6AB6B0 | unmapped | 0.0% |  |  |
| sim_frame:1,2,3 | — 0x6AB240 | unmapped | 0.0% |  |  |
| sim_frame:1,2,3 | — 0x6AB2D0 | unmapped | 0.0% |  |  |
| sim_frame:1,2,3 | `gps_route_update` 0x765BB0 | mapped | 3.7% | 11 (gfx_cmd) | 40 |
| sim_frame:1b | `game_clock_advance` 0x68A1E0 | mapped | 0.2% |  | 13, 40 |
| sim_frame:1b | `net_send_periodic_status` 0x68A320 | mapped | 1.0% |  | 35, 40 |
| sim_frame:1b | — 0x68B860 | unmapped | 0.2% | 2 (gfx_cmd) |  |
| sim_frame:1b | `nav_timesliced_jobs_update` 0xA17690 | mapped | 1.7% |  | 21, 40 |
| sim_frame:1b | `player_nav_node_update` 0xA1CCC0 | mapped | 2.4% |  | 21, 40 |
| sim_frame:1b | `nav_detours_update` 0xA0AFF0 | mapped | 2.6% |  | 21, 40 |
| sim_frame:1b | `nav_detour_hulls_update` 0xA0BF10 | mapped | 2.0% |  | 21, 40 |
| sim_frame:1b | `triggers_do_frame` 0xA6C350 | mapped | 6.9% | 5 (gfx_cmd, streamman) | 09, 40 |
| sim_frame:1b | `restricted_zones_update` 0xA58EE0 | mapped | 2.0% |  | 21, 40 |
| sim_frame:1b | `debug_city_plane_export_update` 0x501FB0 | mapped | 4.2% | 37 (gfx_cmd, hddio, streamman) | 40 |
| sim_frame:1b | `mission_fail_flow_do_frame` 0x6A5BB0 | mapped | 11.1% | 18 (gfx_cmd, rsched, streamman) | 40 |
| sim_frame:1b | — 0x7B02E0 | unmapped | 0.3% |  |  |
| sim_frame:1b | `crib_stash_tutorial_nag` 0x548260 | mapped | 0.2% |  | 30 |
| sim_frame:1b | — 0xA93FF0 | unmapped | 0.0% |  |  |
| sim_frame:1c | `net_registered_objects_update` 0x8045F0 | mapped | 0.9% |  | 35, 40 |
| sim_frame:1c | `net_remote_objects_do_frame` 0x804850 | mapped | 0.0% |  | 35, 40 |
| sim_frame:1c | `physics_frame_time` 0x6FF860 | mapped | 0.1% |  | 17, 40 |
| sim_frame:1c | `vehicle_interactions_update` 0xB08530 | mapped | 4.7% | 1 (rsched) | 16, 40 |
| sim_frame:1c | `persona_events_update` 0x6ADB10 | mapped | 0.8% |  | 40 |
| sim_frame:1c | `humans_update_all` 0x9D7F90 | mapped | 10.5% | 2 (gfx_cmd) | 40 |
| sim_frame:1c | `humans_update_neighborhoods` 0x9A7080 | mapped | 6.5% | 1 (gfx_cmd) | 15 |
| sim_frame:1c | `humans_animation_jobs` 0x9A7150 | mapped | 0.6% |  | 15 |
| sim_frame:1c | `humans_ai_update` 0x9A71B0 | mapped | 10.5% | 1 (gfx_cmd) | 15 |
| sim_frame:1c | `object_items_update` 0x9305F0 | mapped | 5.8% |  | 34, 40 |
| sim_frame:1c | `object_props_update` 0x935820 | mapped | 5.1% | 1 (streamman) | 34, 40 |
| sim_frame:1c | `ai_global_do_frame` 0x437160 | mapped | 5.9% |  | 21, 40 |
| sim_frame:1c | — 0x55BA40 | unmapped | 4.4% | 1 (rsched) |  |
| sim_frame:1c | `septic_decals_update` 0x56BD60 | mapped | 3.1% | 8 (gfx_cmd) | 29, 40 |
| sim_frame:1c | `districts_ownership_refresh` 0x4B11F0 | mapped | 0.4% | 4 (mempool) | 30, 40 |
| sim_frame:1c | — 0x5507C0 | unmapped | 0.0% |  |  |
| sim_frame:1c | `shops_do_frame` 0x9F5420 | mapped | 1.6% |  | 30, 40 |
| sim_frame:1c | `holdup_do_frame` 0x655750 | mapped | 4.8% |  | 29, 40 |
| sim_frame:1c | `chunk_swap_sequences_update` 0x542B00 | mapped | 1.1% |  | 05, 40 |
| sim_frame:1c | `pushback_do_frame` 0x565DA0 | mapped | 0.6% | 2 (gfx_cmd) | 29, 40 |
| sim_frame:1c | `diversion_mode_frame` 0x603140 | mapped | 0.1% |  |  |
| sim_frame:1c | `ambulances_update` 0xA95A30 | mapped | 2.9% | 1 (gfx_cmd) | 21, 40 |
| sim_frame:1c | `meter_maids_update` 0xADD120 | mapped | 1.5% |  | 21, 40 |
| sim_frame:1c | `taxis_update` 0xAEF510 | mapped | 1.5% |  | 21, 40 |
| sim_frame:4 | `vehicles_pre_update` 0xAE3A80 | mapped | 8.2% |  | 40 |
| sim_frame:4 | `debris_do_frame` 0x92B3B0 | mapped | 0.9% |  | 34, 40 |
| sim_frame:4 | `level_objects_physics_update` 0x9FA620 | mapped | 2.2% |  | 40 |
| sim_frame:4 | `chunk_object_anims_update` 0x91C6C0 | mapped | 0.2% |  | 05 |
| sim_frame:4 | `distant_vehicles_update` 0xB80070 | mapped | 1.1% |  | 40 |
| sim_frame:4 | `distant_peds_update` 0xB7CA80 | mapped | 7.5% |  | 40 |
| sim_frame:4 | `player_vehicle_edge_check` 0x9DF820 | mapped | 1.6% |  | 16, 40 |
| sim_frame:4 | `job_worker_wake` 0xBF3760 | mapped | 0.0% |  | 11 |
| sim_frame:4 | `garage_exit_tutorial` 0x554E20 | mapped | 7.2% | 2 (gfx_cmd) | 30 |
| sim_frame:4 | `humans_movement_update` 0x9A9890 | mapped | 7.8% |  | 15 |
| sim_frame:4 | `sim_physics_step` 0x6FA0E0 | mapped | 18.0% | 6 (gfx_cmd, mempool, render) | 11, 17 |
| sim_frame:4 | `humans_character_step` 0x9A9B60 | mapped | 3.6% |  | 15 |
| sim_frame:4 | `explosions_flush_queue` 0x4C2BF0 | mapped | 5.8% |  | 33, 40 |
| sim_frame:4 | — 0x9A9CE0 | unmapped | 5.1% |  |  |
| sim_frame:4 | `havok_triggers_update` 0x71B910 | mapped | 0.4% |  | 36, 40 |
| sim_frame:5 | `streamman_slot_entries_update` 0xA8B400 | mapped | 0.1% |  | 05, 40 |
| sim_frame:5 | `population_update` 0x95A3D0 | mapped | 0.9% |  | 21 |
| sim_frame:5 | `vehicles_update` 0xAE3C60 | mapped | 13.8% |  | 16 |
| sim_frame:5 | `vehicle_interaction_riders_update` 0xB095D0 | mapped | 4.9% | 3 | 16, 40 |
| sim_frame:5 | `vehicle_surfing_attach_player` 0x634350 | mapped | 4.5% |  | 29, 40 |
| sim_frame:5 | `humans_post_update` 0x9A7210 | mapped | 13.0% | 9 (gfx_cmd, streamman) | 15 |
| sim_frame:5 | `projectiles_update` 0x94C0A0 | mapped | 7.7% | 7 (gfx_cmd) | 20 |
| sim_frame:5 | `video_frame` 0x688FE0 | mapped | 1.0% |  | 23 |
| sim_frame:5 | `weapons_cooldown_update` 0xB72FD0 | mapped | 0.0% |  | 20, 40 |
| sim_frame:5 | `brass_update` 0x4BD790 | mapped | 1.5% |  | 33, 40 |
| sim_frame:5 | `weapon_effects_update` 0xB6A4A0 | mapped | 3.3% |  | 20, 40 |
| sim_frame:5 | `vehicle_attached_effects_update` 0x4D1D50 | mapped | 0.1% |  | 33, 40 |
| sim_frame:5 | `object_items_physics_sync` 0x930EC0 | mapped | 0.2% |  | 34, 40 |
| sim_frame:5 | `level_objects_do_frame` 0x9FA670 | mapped | 3.0% | 3 | 40 |
| sim_frame:5 | `level_objects_activate_queue` 0x9FF5A0 | mapped | 2.9% |  | 40 |
| sim_frame:5 | `script_objects_release_pending` 0xA5E3E0 | mapped | 1.0% |  | 09, 40 |
| sim_frame:5 | `object_props_post_update` 0x935B80 | mapped | 0.2% |  | 34, 40 |
| sim_frame:5 | `ambient_audio_sequences_update` 0x490940 | mapped | 1.0% |  | 10, 40 |
| sim_frame:5 | `vehicle_siren_update` 0x489CC0 | mapped | 2.4% |  | 10, 16, 40 |
| sim_frame:5 | `chunk_effects_update` 0x4AAC40 | mapped | 2.4% |  | 05, 33, 40 |
| sim_frame:5 | `chunk_object_attachments_update` 0x4AB140 | mapped | 2.2% |  | 05 |
| sim_frame:5 | `detour_hull_objects_update` 0x9193C0 | mapped | 0.3% |  | 34, 40 |
| sim_frame:5 | `timed_object_effects_expire` 0x5585E0 | mapped | 0.3% |  | 40 |
| sim_frame:6,7 | `weapon_target_lock_update` 0xB6AD40 | mapped | 4.0% | 9 (gfx_cmd) | 20, 40 |
| sim_frame:6,7 | `weather_update` 0x4DE770 | mapped | 0.9% | 1 (mempool) | 33, 38, 40 |
| sim_frame:6,7 | `tod_keyframes_select` 0x4D6850 | mapped | 0.0% |  | 40 |
| sim_frame:6,7 | `shadow_cascades_update` 0x4E66A0 | mapped | 5.0% |  | 40 |
| sim_frame:6,7 | `weather_rain_effects_update` 0x4CF840 | mapped | 7.5% | 2 | 33, 40 |
| sim_frame:6,7 | `light_modifiers_update` 0x4CA750 | mapped | 2.9% | 8 (gfx_cmd, mempool) | 33, 40 |
| sim_frame:6,7 | `ambient_zone_effects_update` 0x503AE0 | mapped | 1.8% |  | 33, 40 |
| sim_frame:6,7 | `camera_target_effects_update` 0x4CBFC0 | mapped | 0.2% |  | 19, 40 |
| sim_frame:6,7 | `inventory_find_weapon` 0x55ADF0 | mapped | 0.1% |  | 38 |
| sim_frame:6,7 | `water_waves_update` 0x727610 | mapped | 0.3% |  | 40 |
| sim_frame:6,7 | `searchlights_update` 0x955470 | mapped | 2.5% |  | 34, 40 |
| sim_frame:6,7 | `effect_objects_do_frame` 0x5053B0 | mapped | 17.4% |  | 40 |
| sim_frame:6,7 | `tod_ambient_effects_update` 0x5142A0 | mapped | 2.9% |  | 33, 40 |
| sim_frame:6,7 | `net_send_door_event` 0x91A840 | mapped | 3.6% |  | 27 |
| sim_frame:6,7 | `ai_scheduled_tasks_expire` 0x4525E0 | mapped | 0.2% |  | 21, 40 |
| sim_frame:6,7 | `ai_scheduled_tasks_tick` 0x452820 | mapped | 0.0% |  | 21, 40 |
| sim_frame:6,7 | `ai_scheduled_tasks_run` 0x453120 | mapped | 2.6% |  | 21, 40 |
| sim_frame:6,7 | `world_focus_point_update` 0x9DF4C0 | mapped | 6.0% | 13 (gfx_cmd) | 19, 40 |
| sim_frame:6,7 | `coop_do_frame` 0x926170 | mapped | 5.0% |  | 35, 40 |
| sim_frame:6,7 | `glares_update` 0x4C6CD0 | mapped | 1.7% |  | 40 |
| sim_frame:6,7 | `lighting_exposure_blend_update` 0x4C35C0 | mapped | 2.3% | 4 (gfx_cmd) | 40 |
| sim_frame:6,7 | `mesh_decals_fade_update` 0xD07B40 | mapped | 0.0% |  | 40 |
| sim_frame:6,7 | — 0x4DA880 | unmapped | 2.0% | 1 (mempool) |  |
| sim_frame:6,7 | `object_effect_links_update` 0x92D620 | mapped | 1.5% |  | 33, 40 |
| sim_frame:6,7 | `level_objects_near_camera_update` 0x9FCF90 | mapped | 4.0% | 3 | 40 |
| sim_frame:6,7 | — 0xB82180 | unmapped | 0.1% |  |  |
| sim_frame:6,7 | `gfx_get_thread_state` 0xD22790 | mapped | 0.1% |  |  |
| sim_frame:6,7 | — 0xB821E0 | unmapped | 0.1% |  |  |
| sim_frame:6,7 | `rewards_do_frame` 0x6BCC60 | mapped | 1.3% | 1 (rsched) | 30, 40 |
| sim_frame:6,7 | `difficulty_set_message` 0x54C220 | mapped | 1.5% | 5 | 30 |
| sim_frame:6,7 | `cheats_do_frame` 0x687B40 | mapped | 2.4% | 18 (gfx_cmd, rsched, streamman) | 38 |
| sim_frame:6,7 | `notoriety_update` 0x55F1D0 | mapped | 0.6% | 1 | 21 |
| sim_frame:6,7 | `coop_session_active` 0x7F7AD0 | mapped | 0.0% |  | 09 |
| sim_frame:6,7 | `confessional_do_frame` 0x7A8F70 | mapped | 1.1% |  | 30, 40 |
| sim_frame:6,7 | `world_teleport_load_update` 0x4BC420 | mapped | 7.7% | 21 (gfx_cmd, hddio, streamman) | 40 |
| sim_frame:6,7 | `rumble_update` 0x4C4210 | mapped | 1.1% |  | 14, 40 |
| sim_frame:6,7 | `chunk_object_lists_update` 0x91E9F0 | mapped | 2.4% |  | 05, 40 |
| sim_frame:6,7 | `world_lighting_update` 0x4D9F20 | mapped | 6.1% | 3 | 33, 40 |
| sim_frame:6,7 | `render_param_volumes_update` 0x4C2DD0 | mapped | 1.0% |  | 33, 40 |
| sim_frame:6,7 | — 0x683E00 | unmapped | 0.0% |  |  |
| sim_frame:6,7 | — 0x9E99F0 | unmapped | 0.0% |  |  |
| sim_frame:6,7 | `misc_packet_send_14` 0x926860 | mapped | 0.9% |  | 27 |
| sim_frame:6,7 | `net_timed_callbacks_run` 0x836BB0 | mapped | 0.0% |  | 35, 40 |
| sim_frame:6,7 | `gfx_view_set_hud` 0xD24B70 | mapped | 2.5% |  | 40 |
| sim_frame:6,7 | `hud_do_frame` 0x793A40 | mapped | 3.6% | 12 (gfx_cmd) | 28, 40 |
| sim_frame:6,7 | `interface_do_frame` 0x7F3500 | mapped | 5.3% |  | 06 |
| sim_frame:6,7 | `camera_get_fov` 0x494080 | mapped | 0.6% |  | 11, 19 |
| sim_frame:6,7 | `gfx_view_set_world` 0xD24D80 | mapped | 2.9% |  | 40 |

### `frame_end_handoff` (0x521A00)

| Profiler section | Call | Status | Reach | Rust below | Doc |
|---|---|---|---|---|---|
|  | `render_thread_wait_frame` 0x522B80 | mapped | 0.0% |  | 40 |
|  | `gfx_device_acquire` 0xCFBC00 | mapped | 0.0% |  | 40 |
|  | `gfx_invalidate_render_state_block` 0xD14950 | Rust | 0.0% | 1 (gfx_cmd) |  |
|  | `gfx_apply_gamma` 0xD14770 | mapped | 0.1% |  | 18 |
|  | `gfx_set_tint_color` 0xD14700 | Rust | 0.0% | 1 (gfx_cmd) |  |
|  | `game_state_stack_stream_frame` 0x697740 | mapped | 0.0% |  | 40 |
|  | `gameplay_streaming_frame` 0x68E410 | mapped | 9.2% | 46 (gfx_cmd, hddio, mempool, streamman) | 40 |
|  | `frame_present` 0x5217E0 | mapped | 4.2% | 13 (gfx_cmd, render) | 40 |
|  | `audio_update_voices_on_workers` 0x4655B0 | mapped | 4.7% |  | 10 |
|  | `temp_batching_pool_release` 0x521970 | mapped | 0.0% |  | 40 |
|  | `temp_batching_pool_create` 0x5218D0 | mapped | 0.0% |  | 40 |
|  | `render_snapshot_copy_view` 0x5220B0 | mapped | 0.0% |  | 40 |
|  | `render_snapshot_reset_scales` 0x521870 | mapped | 0.0% |  | 40 |
|  | `time_of_day_in_range` 0x4D9900 | mapped | 0.1% |  | 40 |
|  | `tod_light_value_update` 0x4D9DD0 | mapped | 0.2% |  | 11 |
|  | `tod_ambient_colors_update` 0x4D4820 | mapped | 0.1% |  | 11 |
|  | `tod_light_block_b_update` 0x4D4F90 | mapped | 0.1% |  | 11 |
|  | `tod_sun_update` 0x4D5470 | mapped | 0.1% |  | 11 |
|  | `tod_fog_color_update` 0x4D5300 | mapped | 0.1% |  | 11 |
|  | `tod_misc_params_update` 0x4D5670 | mapped | 0.1% |  | 11 |
|  | `lighting_buffers_copy_for_render` 0x4E69D0 | mapped | 0.0% |  | 11 |
|  | `tagging_paint_buffer_swap` 0x6255A0 | mapped | 0.8% |  | 11 |
|  | `audio_frame_snapshot` 0x46CCD0 | mapped | 3.8% | 1 (render) | 10, 11 |
|  | `mesh_decals_frame_update` 0xD08310 | mapped | 2.2% |  | 11 |
|  | `glares_render_snapshot` 0x4C7190 | mapped | 0.1% |  | 11 |
|  | `water_sim_frame_snapshot` 0x728AB0 | mapped | 0.7% |  | 11 |
|  | `render_params_publish` 0x51A880 | mapped | 0.0% |  | 11 |
|  | `render_params_reset` 0x51A4A0 | mapped | 5.9% | 3 | 11 |
|  | `objects_collect_render_flagged` 0x9FAA20 | mapped | 0.1% |  | 11 |
|  | `camera_get_fov` 0x494080 | mapped | 0.6% |  | 11, 19 |
|  | `matrix_orthonormalize` 0xBE4170 | mapped | 1.8% |  | 11 |
|  | `sky_light_snapshot` 0x4D9A50 | mapped | 0.1% |  | 11 |
|  | `world_shader_constants_update` 0xB77CA0 | mapped | 0.2% |  | 11 |
|  | — 0x9B92E0 | unmapped | 0.1% |  |  |
|  | — 0xAE28F0 | unmapped | 0.0% |  |  |
|  | `vehicles_render_prepare` 0xAE1020 | mapped | 1.1% |  | 11 |
|  | `render_lists_reset` 0x52E7C0 | mapped | 0.0% |  | 18 |
|  | `game_render_func` 0x5250E0 | mapped | 11.0% | 5 (gfx_cmd) | 36 |
|  | `game_build_render_batches` 0x522FC0 | mapped | 19.6% | 6 (gfx_cmd) | 40 |

## Sections :1c, :4 and :5 (porting notes)

All of the calls in these sections are named except three. The `functions.csv` note of each says what list or state it walks.

**Conventions:**
- Every call in the three sections takes no arguments and returns nothing, with these exceptions:
  - `humans_update_all` (0x9D7F90) and 0x9A9CE0 take cdecl (f32 dt), pushed from the dt local of `sim_frame`.
  - `physics_frame_time` (0x6FF860) returns ST0.
- `sim_frame` reads no return values in these sections. Most callees walk one global intrusive list and call a per-entry function with the entry in a register or as a cdecl argument. Those per-entry functions are still original code.

**Still unidentified:**
- **0x9A9CE0** (`:4`), cdecl (f32 dt). When dt ≠ 0, it calls 0x9A2B80 for each entry of the list 0x215E908 (links +0xB8) with +0xCC bit 0x200. The list is shared by AI, cutscene and human code; the bit's meaning is unknown.
- **0x55BA40** (`:1c`). It walks 0x2527514 indices (0x1409A08) into 0x78-byte records with a vehicle handle and a state machine. It calls `vehicle_ai_order_03`, and 0x55BDB0 / 0x55BE80 send human packets 2 / 3.
- **0x5507C0** (`:1c`). Three 20-minute timers (0x2797BD0[1..3]). When one fires and the count 0x1409AA8[i] < 5, it calls 0x55BD10. It shares the counts with 0x55BA40, so the two are probably one system: spawned vehicle groups, perhaps crib or gang deliveries.

**Porting order inside these sections:** the list-walkers are thin, so they can be ported as Rust loops calling the original per-entry functions:
- `ambulances_update`, `meter_maids_update`, `taxis_update`, `weapon_effects_update`, `debris_do_frame`, `brass_update`, `object_props_post_update`, `level_objects_*`, `detour_hull_objects_update`, `vehicle_interaction_riders_update`, `timed_object_effects_expire`.

The heavy ones are `object_items_update`, `object_props_update`, `vehicle_interactions_update`, `havok_triggers_update`, `chunk_effects_update` and `ambient_audio_sequences_update`. They are better ported after their per-entry functions.

## Suggested porting order

1. **Done (`crates/sr2-hook/src/frame.rs`).** **`game_frame_tick` as a Rust skeleton.** It makes 18 calls, 2 of them already Rust (`input_do_frame`, `mouse_update`). The rest are called through their addresses. This gives a Rust-owned frame without changing behaviour, and a place to add timing.
2. **Done (`frame.rs`, with `game_state_stack_dispatch`).** **`game_state_frame`.** It makes 6 calls, all mapped (doc 11 / 06).
3. **Done (`frame.rs`).** **`frame_end_handoff`.** Port the handoff order and the snapshot copies (`render_snapshot_copy_view`, `render_snapshot_reset_scales`, `temp_batching_pool_*`). Keep the unmapped lighting block as original calls.
4. **`sim_frame`, by profiler section**, using the ones whose calls are best mapped:
   1. `sim_frame:4`: humans, physics, vehicles prepare;
   2. `sim_frame:5`: vehicles, humans post, projectiles;
   3. `sim_frame:1c`: the humans entries.

   Sections `1,2,3`, `1b` and `6,7` are mostly unmapped. Name them before porting.

## Named for this map

`string_crc32_lower`, `dialog_box_do_frame`, `game_pause_do_frame`, `game_state_stack_dispatch`, `game_state_stack_stream_frame`, `gameplay_streaming_frame`, `render_thread_wait_frame`, `gfx_device_acquire`, `frame_present`, `screenshot_save`, `temp_batching_pool_create`, `temp_batching_pool_release`, `render_snapshot_copy_view`, `render_snapshot_reset_scales`, `time_of_day_in_range`, `game_build_render_batches`, `vehicles_pre_update`, `world_teleport_load_update`.

The roles of `render_thread_wait_frame`, `gfx_device_acquire`, `time_of_day_in_range`, `vehicles_pre_update` and `world_teleport_load_update` are inferred from one reading of the decompile.

The table generator is `ftmap.py` / `gen10.py` (session scratchpad). It is not in `tools/` yet.
