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
| vhashtable<>::clear_buckets (9 copies: 0x4ac100, 0x70bc20, 0x70be50, 0x9fe7d0, 0x9feb30, 0xa5fdf0, 0xa77940, 0xad33d0, 0xb8a070) | port_leaves (ported, NOT installed) | the translated loop is ~100x slower than the original on big tables: 0.15 s a call dropped gameplay to 5 fps; needs a hand-written loop |
| havok_triggers_update 0x71B910, garage_exit_tutorial 0x554E20, cheats_do_frame 0x687B40, rich_presence_update 0x69A440, triggers_do_frame, vehicle_interactions_update | port_frame2 | 14.7 KB stack frame and stack objects; many register callees and x87; uninitialised ECX push; jump tables; size (~9 KB each) |
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

