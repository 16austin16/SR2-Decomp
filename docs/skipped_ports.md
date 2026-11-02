# Skipped ports

Functions a porting agent looked at and left in the original code, with the reason. Remove a row
when the function is ported (its docs/functions.csv status becomes `rust`).

| Address | Name | Skipped in | Reason | What it needs |
|---|---|---|---|---|
| 0x00c03af0 | spline_eval_tangent | port_nav (round 3) | long mixed SSE / x87 code with comiss branches, not fully read | a full read |
| 0x00c03dc0 | spline_eval_second_derivative | port_nav (round 3) | as spline_eval_tangent | a full read |
| 0x00a0aed0 | nav_detour_release_hull | port_nav (round 3) | unbounded writes into hull rings | port without a shadow, or bound the rings |
| 0x00817d80 | (pool lookup, doc 48) | port_net (round 4) | not reached in the time available; one of three pool lookups that differ only in their globals | port with 0x817F40 / 0x818020 from one body |
| 0x00817f40 | (pool lookup, doc 48) | port_net (round 4) | as 0x817D80 | as 0x817D80 |
| 0x00818020 | (pool lookup, doc 48) | port_net (round 4) | as 0x817D80 | as 0x817D80 |
| 0x00a44f10 | luafn_set_mission_author | port_bindings3 | 3 bytes (`xor eax,eax; ret`); a detour needs 5, and the next bytes are not padding | a 3-byte patch that is not a detour |

## Groups left for a later pass

| Group | Skipped in | Reason | What it needs |
|---|---|---|---|
| ~60 vint property getters with two `ret`s (pattern: vint_element_base::get_alpha 0xB96860, doc 49 section 5) | port_leaves_fp | the sweep only took single-ret bodies | a two-exit leaf template |
| ~55 more trivial stubs (constant returns, `xor eax,eax`, flags, no-ops) | port_leaves (sweep 2) | cut to keep the batch reviewable | the same template as the 255 already ported |
| transcendental vint_ease_* (sin / pow / sqrt / bounce) | port_leaves_fp | they call the CRT maths functions | call the CRT by address |
| coop_wait_text, script_positionable_object copy_transform_from / set_transform | port_leaves (sweep 2) | `rep movsd` | reproduce the copy |

## Skipped in the large rounds (2026-11-01 / 02)

| Function(s) | Skipped in | Reason |
|---|---|---|
| large AI order update / on_event handlers (k0a on_event 0x41C670, k0d update 0x42D2B0, k0b / k00 / k23 / k14 updates), ai_order_update 0x4372E0, k03 net_read 0x405C70, k1b net_read 0x45CAC0 | port_ai | control flow depends on callee results and x87 compares mid-function; needs a dependent-call check framework |
| luafn_explosion_create, luafn_script_profiler_end_section | port_bindings4 | effect structs and x87 vectors on the stack; 3-byte stub |
| net object core (0x805FC0, 0x806910, 0x805000, 0x804140, 0x804320, 0x8053B0, 0x8067D0, 0x804550, 0x806E30, 0x807780), join_req 0x815930, 0x80ED70 | port_net | pool allocation plus virtual calls (a shadow would double side effects); not reached; jump table plus virtuals |
| spline_param_at_distance (+ checked), spline_closest_point (+ tangent), spline_arc_length, spline_curvature, road_lane_walk_forward / back, road_network_query, other road_query_cb_*, nav_tri_closest_point_xz / point_inside_xz / height_at, nav_cell_project_point, vehicle_ai_pathfind_* | port_nav | XMM0 returns; recursive unbounded writes; odd callee conventions; size |
| audio_voice_start / prepare / start_2d_channel, audio_play_foley_cue / voice_line / music_cue, audio_voice_slot_alloc (lock 0x25F5360), weapon_info_parse_record, weapon_reload_from_store | port_weapons_audio | size and audio-thread shared state |
| vint_value_set_int / set_handle / set_name, vint_value_set_color_bytes (fldcw / fistp) | port_vint2, port_leaves_fp | convert and call release / addref; rounding-mode changes |

## Machine-translated ports: test whole, then rewrite by hand later

These run as Rust but are mechanical one-statement-per-instruction translations, not readable rewrites. The user's decision (2026-11-01) is to keep them and count them as Rust once tested, and to rewrite them by hand later.

| Group | File | State |
|---|---|---|
| 58 startup / load functions (xtbl loaders, load_from_table / init, gameplay_constants_parse_grab_offsets, cutscene_player_create ...), ~111 KB | port_tables.rs (kept out of the repo and out of install() until tested) | native differential test written, never compiled or run |
| 12 camera functions: camera_follow_update 0x49B4B0, camera_update 0x4932F0 and 10 helpers (0x49F320, 0x498D40, 0x499480, 0x499190, 0x497A20, 0x4938C0, 0x492DF0, 0x496BE0, 0x49AE60, 0x49D7A0), ~17 KB; 4 more not installed (0x4969B0, 0x497220, 0x4A01B0, 0x499B50) | port_big.rs camera version (kept aside, not in the repo yet) | instruction forms tested against the CPU; whole functions not yet tested against the original |

## Skipped in the 2026-11-02 round

| Function(s) | Skipped in | Reason |
|---|---|---|
| vint_object_set_name 0xB97620, vint_object_rehash_name 0xB97570 | port_vint2 (ported, NOT installed) | crashed the game at startup in the play test; needs a fix and a differential test |
| port_tables.rs (57 lifted startup / load functions, native-tested 57/58; xtbl_load_effects 0x50D000 failed) | not in the repo, install() commented out | one of them passes a bad pointer into 0xA7CA70 (streaming) in game; find it with the native test widened, then re-enable |
| 9 of the 12 machine-translated camera ports (camera_follow_update 0x49B4B0, camera_update 0x4932F0, 0x499480, 0x4938C0, 0x499190, 0x497A20, 0x492DF0, 0x49AE60, 0x49F320) and 0x4969B0, 0x497220, 0x4A01B0, 0x499B50 | port_big (not installed) | whole-function test: 6 fail, 3 low coverage; the 4 never integrated. Rewrite by hand. |
| cutscene_stop 0x6C44C0, cutscene_begin_callback 0x6BFB50, cutscene_shot_start 0x6C8130, cutscene_shot_place_characters 0x6C24E0, plus the cutscene action functions from 0x6CB920 on | port_cutscene | size and register-convention-dependent callees |
| 4 cutscene and 1 camera-mode port | port_cutscene / port_camanim | another module already replaces the address (replace failed at install) |
| vehicle_camera_record_parse 0xAED2D0, anim_track_sample_vec3 0xCE3610, anim_tracks_update_frame 0xCE7520, vehicle / camera_free xtbl parsers 0xAE5C50-0xAEE5F0, 0x495AC0 | port_camanim | per-callee register mapping and a differential test needed |
| 9 large activity functions (0x5735A0 draft in scratchpad/activities_port2, 0x6A0080, 0x585C60, 0x5934B0, 0x5CC540, 0x580A40, 0x5AC6D0, 0x6740A0, 0x599F80) | port_activities | not reached; use the difftest kit in port_activities.rs |
| ai order k0d / k00 / k23 / k0b updates | port_ai | 750-1560 instructions of x87 and callees each; port_ai's k14 update is the template |
| weather: light_modifiers_update 0x4CA750 (lightning), world_lighting_update 0x4D9F20, world_focus_point_update 0x9DF4C0, 0x4CE230, 0x4CD640, 0x4D7160, 0x4D7E20 | port_weather | size, critical sections, havok raycasts, sin tables |
| vehicle_ai_pathfind_to / _nodes / _navmesh, A* core 0xA121B0, polygon cut 0xA11750, job stage 0xA16650, path-request wrappers 0xA14230 / 0xA14560 / 0xA147E0, 0xA8A4F0 | port_nav | 13 KB of result-driven control flow; needs a memory-peek harness |
| net_state_sync_send 0x805FC0, 17 packet receivers (drafts in scratchpad/net3/port_net_with_recv.rs) | port_net | x87 buckets + qsort; receivers not verified |
| weapon_info_parse_record (13.7 KB), ambient_audio_sequences_update 0x490940 | port_weapons_audio | size; FPU rounding change, lock, 13 callees |
| humans_post_update 0x9A7210, object_items_update 0x9305F0 and ~60 other doc-40 callees | port_frame2 | size |
| ~50 more float leaves calling CRT maths; mesh_decals_fade_update, effect_list_expire_timed, occluder_contains_point | port_leaves_fp | not examined; game-list loops |
| 16 esp-frame, 18 unknown-convention and 39 interior-referenced integer leaves | port_leaves | conventions |

