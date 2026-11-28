# Skipped ports

Functions still left in the original code, grouped by why. Remove a row when the function is ported
(its docs/functions.csv status becomes `rust`). Rewritten 2026-11-03 to drop rows that later rounds
ported.

## Broke the game when installed

| Function(s) | Where | What happened | What it needs |
|---|---|---|---|
| aabb_point_dist_sq_x87 0xBF65F0 | port_render4 (excluded in portlib EXCLUDED) | crashed loading a save: a return landed on a float (2026-11-26) | find the stack / x87 imbalance |
| hkvector4_equal_store 0xBB8A10, hkvector4_less_store 0x72A100 | port_hot60 (excluded) | crash in the Havok contact solver while loading (2026-11-26) | check the mask results and the registers the originals leave |
| port_nav2 (doc 59 nav functions) | port_nav2 (not installed) | its agent never finished its tests | run its difftests, then play-test with a cop chase |
| 115 IDirect3DDevice9Lost stubs | port_small_b D3D_PORTS (not installed) | were naked asm copied from the original bytes | rewrite as real Rust functions |
| tod_light_sets_parse 0x4C8B80 | port_tables (lifted, excluded) | lit the player solid red in the clothes store (2026-11-07) | hand port |
| cutscene_player_create 0x6BDC80, cutscene_parse_characters 0x6D16C0 | port_tables (lifted, excluded) | cutscenes played with no characters (2026-11-07) | hand port |

## Failed or not covered by their test

| Function(s) | Where | Reason |
|---|---|---|
| camera_follow_update 0x49B4B0, camera_update 0x4932F0, 0x49F320; cam_498d40 and cam_49d7a0 (machine-translated); 0x4969B0, 0x497220, 0x4A01B0, 0x499B50 | port_big | mismatches or low coverage in the whole-function test; to be rewritten by hand |

## Too large or complex so far (port maps exist for most)

| Function(s) | Area | Reason |
|---|---|---|
| ai_order_k00_update 0x448CE0, ai_order_k0d_update 0x42D2B0 | AI | 1,100 to 1,560 instructions of x87 and callee-driven control flow; port maps in doc 59 |
| 8 activity functions: 0x6A0080 mission_restart_from_checkpoint, 0x585C60, 0x5934B0 piracy load, 0x5CC540 hooker_revolt::update, 0x580A40, 0x5AC6D0, 0x6740A0, 0x599F80 security update (16 KB) | activities | 4 to 16 KB each; the difftest kit in port_activities.rs is ready |
| cutscene_stop 0x6C44C0 (8.7 KB), cutscene_begin_callback 0x6BFB50 (6.9 KB), and cutscene action functions from 0x6CB920 on | cutscenes | long chains of unrelated subsystem setup / teardown with register-convention callees; too big for one agent session (2026-11-06) |
| rain_start_init 0x4CE230 | weather | 3.8 KB of x87 and sin / cos; runs once per rain start |
| glare_occlusion_query_update 0x4C5180; tree walks 0x4B6A20, 0x4B6B60, 0x4B6CC0; aabb_point_dist_sq_x87 0xBF65F0; 0xCF64A0, 0xD19E30 | render thread | D3D query polling; explicit-stack tree walks; mixed x87 / SSE with writes through ESI; last two only inferred (2026-11-06) |
| physics_linear_cast 0x6FA820, object_item_create 0x931140, ragdoll_build_for_object 0x711D40 | physics | not reached; maps in doc 53 |
| vehicle_ai_pathfind_to / _nodes / _navmesh, nav_path_search 0xA121B0, nav_cells_heuristic 0xA11750, nav_poly_link_match 0xA16650, 0xA14230 / 0xA14560 / 0xA147E0, road_query_random_lane_setup 0xA8A4F0 | navigation | 13 KB of result-driven control flow; port maps and a test-harness plan in doc 59 |
| vehicle_camera_record_parse 0xAED2D0, anim_track_sample_vec3 0xCE3610, anim_tracks_update_frame 0xCE7520, the vehicle / camera_free xtbl parsers 0xAE5C50-0xAEE5F0 and 0x495AC0 | camera / animation | per-callee register mapping needed |
| net_state_sync_send 0x805FC0; 17 packet receivers (drafts in scratchpad/net3/port_net_with_recv.rs) | networking | x87 distance buckets and qsort; receivers not verified |
| weapon_info_parse_record (13.7 KB), ambient_audio_sequences_update 0x490940 (radio) | weapons / audio | size; FPU rounding-mode change, a lock and 13 callees |
| about 50 frame-loop callees (activity_hud_markers_apply, pushback_phone_call_hud_text, remote_do_frame ...) | frame loop | size; not reached yet |
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

## Known issues (not tied to a port yet)

| Issue | Notes |
|---|---|
| Police helicopters hover or only make fly-bys instead of chasing (2026-11-26) | Vanilla (all ports off) chases properly. vehicle_is_helicopter 0xAB5190 now leaves ECX like the original, which did not fix it. Not bisected; the user asked to note it for later. Use the skip file (module or function names, `all_ports`) and 5 stars from the F7 menu. |
| Ports in port_hot60 that restore ECX / EDX where the original leaves a value | 0xCF7760 (ECX = 0), 0xA96A70 (ECX, EDX = pos.z), 0x71A0A0 (ECX = &s), 0x485D30 (ECX = &b): reproduce the original's values (rule added to docs/PORTING_RULES.md) |

## Not ported in the 2026-11-23 round

| Function(s) | Reason |
|---|---|
| remote_do_frame bodies: item 0x843C20, mover 0x84C0A0, npc 0x852580, pipebomb 0x85CCA0, player 0x8633D0, prop 0x86C0F0, rocket 0x86F310, vehicle 0x874310 | 0.9-8 KB of SSE quaternion maths on overlapping stack buffers |
| distant_peds_update 0xB7CA80 | __alloca_probe and stack out-buffers filled by callees |

## Machine-lifted AI ports (2026-11-27)

The judge fix round replaced 71 hand-written AI ports in port_ai.rs with machine-lifted bodies (`mod lifted`;
lifter at scratchpad/fix/fix_ai/lift.py). They pass the strict judge and the 2026-11-27 play test, but they are
one-statement-per-instruction translations. The hand versions are still in the file, bypassed by
`lifted::thunk_for`. To do: hand-rewrite them with the judge as referee, then delete `mod lifted`.

