# Skipped ports

Functions still left in the original code, grouped by why. Remove a row when the function is ported
(its docs/functions.csv status becomes `rust`). Rewritten 2026-11-03 to drop rows that later rounds
ported.

## Broke the game when installed

| Function(s) | Where | What happened | What it needs |
|---|---|---|---|
| ai_order_k23_update 0x4586F0, ai_order_k0b_update 0x41F750 (saved as scratchpad port_ai_round3.rs) | port_ai (not merged) | their change to the shared record / replay shadow crashed the k14 update's replay stub | review the shadow change, then re-test |

## Failed or not covered by their test

| Function(s) | Where | Reason |
|---|---|---|
| camera_follow_update 0x49B4B0, camera_update 0x4932F0, 0x49F320; cam_498d40 and cam_49d7a0 (machine-translated); 0x4969B0, 0x497220, 0x4A01B0, 0x499B50 | port_big | mismatches or low coverage in the whole-function test; to be rewritten by hand |
| xtbl_load_effects 0x50D000 | port_tables (lifted) | fails its native differential test (an argument to 0xBDCB30 differs) |

## Too large or complex so far (port maps exist for most)

| Function(s) | Area | Reason |
|---|---|---|
| ai_order_k00_update 0x448CE0, ai_order_k0d_update 0x42D2B0 | AI | 1,100 to 1,560 instructions of x87 and callee-driven control flow |
| 8 activity functions: 0x6A0080 mission_restart_from_checkpoint, 0x585C60, 0x5934B0 piracy load, 0x5CC540 hooker_revolt::update, 0x580A40, 0x5AC6D0, 0x6740A0, 0x599F80 security update (16 KB) | activities | 4 to 16 KB each; the difftest kit in port_activities.rs is ready |
| cutscene_stop 0x6C44C0, cutscene_begin_callback 0x6BFB50, cutscene_shot_start 0x6C8130, cutscene_shot_place_characters 0x6C24E0, and cutscene action functions from 0x6CB920 on | cutscenes | 2.5 to 8.5 KB, register-convention-dependent callees |
| 0x4CE230, 0x4CD640, 0x4D7160, 0x4D7E20 (render thread, sin tables) | weather / lighting | 3 to 4 KB each; havok queries, big stack frames |
| physics_linear_cast 0x6FA820, object_item_create 0x931140, ragdoll_build_for_object 0x711D40 | physics | not reached; maps in doc 53 |
| vehicle_ai_pathfind_to / _nodes / _navmesh, A* core 0xA121B0, polygon cut 0xA11750, job stage 0xA16650, path-request wrappers 0xA14230 / 0xA14560 / 0xA147E0, 0xA8A4F0 | navigation | 13 KB of result-driven control flow; needs a memory-peek test harness |
| vehicle_camera_record_parse 0xAED2D0, anim_track_sample_vec3 0xCE3610, anim_tracks_update_frame 0xCE7520, the vehicle / camera_free xtbl parsers 0xAE5C50-0xAEE5F0 and 0x495AC0 | camera / animation | per-callee register mapping needed |
| net_state_sync_send 0x805FC0; 17 packet receivers (drafts in scratchpad/net3/port_net_with_recv.rs) | networking | x87 distance buckets and qsort; receivers not verified |
| weapon_info_parse_record (13.7 KB), ambient_audio_sequences_update 0x490940 (radio) | weapons / audio | size; FPU rounding-mode change, a lock and 13 callees |
| humans_post_update 0x9A7210, object_items_update 0x9305F0 and about 60 other frame-loop callees | frame loop | size |
| spline_eval_tangent 0xC03AF0, spline_eval_second_derivative 0xC03DC0 | navigation | long mixed SSE / x87 code |
| nav_detour_release_hull 0xA0AED0 | navigation | unbounded writes into hull rings |
| luafn_explosion_create | script bindings | effect structs and x87 vectors on the stack |

## Small groups not done yet

| Group | Where | Reason |
|---|---|---|
| about 50 float leaves that call CRT maths; mesh_decals_fade_update, effect_list_expire_timed, occluder_contains_point | port_leaves_fp | not examined; loops over game lists |
| 16 integer leaves that use ESP frames, 18 with unknown conventions, 39 that other code jumps into | port_leaves | calling conventions |
| three pool lookups 0x817D80, 0x817F40, 0x818020 | networking | one body with different globals; not reached |

## Can't be patched (under 5 bytes)

| Function(s) | Reason |
|---|---|
| luafn_set_mission_author 0xA44F10, luafn_script_profiler_start_section 0xA43360, luafn_script_profiler_end_section, dump_fps_tagged, cutscene 1-byte `ret` stubs (0x6D7500, 0x6D76C0, 0x6D8160, 0x6D8230, 0x6D82F0, 0x6CAA60, 0x6CAA70), 0x6829E0, 0x608F00 | a detour needs 5 bytes and the next bytes are not padding |

## Machine-translated ports (installed, to rewrite by hand later)

These run as Rust and pass their tests, but they are one-statement-per-instruction translations. The user's decision (2026-11-01) is to keep them and rewrite them by hand later.

| Group | File |
|---|---|
| 57 startup / load functions (xtbl loaders, load_from_table / init, gameplay constants, cutscene_player_create ...) | port_tables.rs |
| the integer and float leaf sweeps' generated bodies | port_leaves.rs, port_leaves_fp.rs |
