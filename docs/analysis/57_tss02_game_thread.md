# 57: Mission tss02 (2026-11-04c log), game thread: phases, hot functions, names, port recipes

Input: `bin/sr2-mission-2026-11-04c.log` (the `-04d` file is the same log plus one extra free-roam session labelled `Test324`, so it adds no mission data).
Scope: the GAME thread (`g` lines) only. Scripts used are in the session scratchpad `log_game` (`sec.py` section loader, `agg.py` unnamed ranking,
`diff.py` / `diff2.py` phase comparison, `tables.py`, `fn.py` / `card.py` / `d.py` disassembly cards). Like doc 56, every sample is mapped with a
call-target sweep (about 31,800 function starts: E8/E9 targets preceded by CC / ret padding, vtable pointers in the data sections, and every row of
`docs/functions.csv`), not by "nearest named function below", because most hot functions were unnamed.

## 1. How the log is laid out, and what the labels mean

The file holds two runs. Run 1 (line 2726 on) is a free-roam walk, `Pre mission start`, then ONE long unlabelled tss02 section (frames 3481-10736). Run 2
(line 10155 on, frame counter restarted) is the labelled mission. A label belongs to the section that STARTS at the F8 press and lasts until the next
press, so `before mission fail` is the stretch in which the failure happened, not the stretch before it. Samples inside Rust-ported code are not recorded.
Rate: 3.2 samples per frame in all, 1.4 per frame on the game thread (46,140 of the 48,228 game samples are in the tss02 sections).

| # | Section (as I call it) | Frames | Game | Game-thread samples | What it is |
|---|---|---|---|---:|---|
| s1 | free roam before F8 | 2965-3403 | none | 186 | walking to the mission |
| s2 | `Pre mission start` | 3403-3481 | none | 76 | at the mission marker |
| s3 | run 1, tss02, unlabelled | 3481-10736 | tss02 | 10447 | mission start, first cutscenes and play (7255 frames) |
| s4 | run 2, tss02 start | 4901-5228 | tss02 | 46 | 327 frames of a fresh load, 46 percent profiler |
| s5 | `cutscene start` | 5228-14629 | tss02 | 9316 | a cutscene, then play (9401 frames) |
| s6 | `before entering courthouse` | 14629-20864 | tss02 | 5277 | approach and interior (6235 frames) |
| s7 | `before mission fail` | 20864-22891 | tss02 | 3810 | the fight in which the mission fails (2027 frames) |
| s8 | fail screen (point 3, game none) | 22891-23161 | none | 87 | fail menu / unload (270 frames) |
| s9 | retry load (point 0, game tss02) | 23161-23303 | tss02 | 74 | restart from the checkpoint (142 frames) |
| s10 | `retried from checkpoint` | 23303-32341 | tss02 | 17170 | the replay, to the mission's end (9038 frames) |
| s11 | after the replay (point 1, game none) | 32341-34731 | none | 1396 | free roam after the mission completes (2390 frames) |
| s12 | `mission ended` | 34731-34946 | none | 343 | immediately after the end mark (215 frames) |

Important limits: s8, s9 and s12 are only 87, 74 and 343 samples, so the "fail" and "retry" machinery itself is seen in a handful of samples; s10 is the
big replay and is the real difference between first pass and retry.

## 2. Hottest game-thread functions per phase

Kind: `named` = already in docs/functions.csv before this round, `rust` = row with status rust, `new(57)` = named in this round, `UNNAMED` = still no name.
Percentages are of that phase's game-thread samples.

#### s1 free roam (before F8): frames 2965-3403 (438), 1024 samples, 186 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 16 | 8.6 | 0xC0C990 | profile_stop | named |
| 8 | 4.3 | 0x520790 | game_main_thread_proc | named |
| 8 | 4.3 | 0xC0BE10 | profile_start | named |
| 7 | 3.8 | 0x50F190 | effect_list_reclaim_dead | named |
| 5 | 2.7 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 4 | 2.2 | 0x4F4E60 | memcmp_regs | named |
| 4 | 2.2 | 0x50F780 | effect_list_expire_timed | named |
| 3 | 1.6 | 0x406810 | vec3_mul_mat33_to | new(57) |

#### s2 "Pre mission start": frames 3403-3481 (78), 224 samples, 76 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 4 | 5.3 | 0x7030B0 | havok_collidable_mesh_subpart_ids_get | new(57) |
| 3 | 3.9 | 0x4AAC40 | chunk_effects_update | named |
| 3 | 3.9 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 2 | 2.6 | 0x406810 | vec3_mul_mat33_to | new(57) |
| 2 | 2.6 | 0x50EF90 | effect_instance_set_transform | new(57) |
| 2 | 2.6 | 0x50F780 | effect_list_expire_timed | named |
| 2 | 2.6 | 0x736D30 | unnamed_00736d30 | UNNAMED |
| 2 | 2.6 | 0x91A840 | net_send_door_event | named |

#### s3 mission tss02 run 1 (unlabelled): frames 3481-10736 (7255), 23200 samples, 10447 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 368 | 3.5 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 279 | 2.7 | 0xC0C990 | profile_stop | named |
| 266 | 2.5 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 193 | 1.8 | 0x520790 | game_main_thread_proc | named |
| 175 | 1.7 | 0x406810 | vec3_mul_mat33_to | new(57) |
| 173 | 1.7 | 0x50F190 | effect_list_reclaim_dead | named |
| 168 | 1.6 | 0x926170 | coop_do_frame | named |
| 159 | 1.5 | 0xC0BE10 | profile_start | named |
| 154 | 1.5 | 0xA020C0 | shape_bounds_overlap_test | named |
| 153 | 1.5 | 0xCE9E10 | rig_pose_evaluate_blended | new(57) |
| 136 | 1.3 | 0x50F780 | effect_list_expire_timed | named |
| 129 | 1.2 | 0xC65DC0 | hkpMoppLongRayVirtualMachine_func | named |

#### s4 run 2 tss02 (unlabelled): frames 4901-5228 (327), 448 samples, 46 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 13 | 28.3 | 0xC0C990 | profile_stop | named |
| 8 | 17.4 | 0xC0BE10 | profile_start | named |
| 5 | 10.9 | 0x520790 | game_main_thread_proc | named |
| 5 | 10.9 | 0xCDEDA0 | heap_malloc_dlmalloc | new(57) |
| 2 | 4.3 | 0x4F4E60 | memcmp_regs | named |
| 2 | 4.3 | 0xA020C0 | shape_bounds_overlap_test | named |
| 1 | 2.2 | 0x4B7700 | city_view_batches_collect | new(57) |
| 1 | 2.2 | 0x4E6010 | unnamed_004e6010 | UNNAMED |

#### s5 "cutscene start": frames 5228-14629 (9401), 23936 samples, 9316 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 381 | 4.1 | 0xC0C990 | profile_stop | named |
| 278 | 3.0 | 0x520790 | game_main_thread_proc | named |
| 268 | 2.9 | 0xC0BE10 | profile_start | named |
| 218 | 2.3 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 157 | 1.7 | 0x926170 | coop_do_frame | named |
| 151 | 1.6 | 0xCE9E10 | rig_pose_evaluate_blended | new(57) |
| 140 | 1.5 | 0xCDEDA0 | heap_malloc_dlmalloc | new(57) |
| 139 | 1.5 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 136 | 1.5 | 0x406810 | vec3_mul_mat33_to | new(57) |
| 132 | 1.4 | 0xA020C0 | shape_bounds_overlap_test | named |
| 117 | 1.3 | 0xC65DC0 | hkpMoppLongRayVirtualMachine_func | named |
| 112 | 1.2 | 0xBF4C30 | aabb_transform_by_mat33_extents | new(57) |

#### s6 "before entering courthouse": frames 14629-20864 (6235), 14144 samples, 5277 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 243 | 4.6 | 0xC0C990 | profile_stop | named |
| 207 | 3.9 | 0x926170 | coop_do_frame | named |
| 197 | 3.7 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 174 | 3.3 | 0xC0BE10 | profile_start | named |
| 158 | 3.0 | 0x520790 | game_main_thread_proc | named |
| 133 | 2.5 | 0xA020C0 | shape_bounds_overlap_test | named |
| 121 | 2.3 | 0x4B7A60 | city_submit_level_objects | new(57) |
| 91 | 1.7 | 0xCDEDA0 | heap_malloc_dlmalloc | new(57) |
| 85 | 1.6 | 0xD96670 | hkp_contact_constraint_solve_step | new(57) |
| 79 | 1.5 | 0xB8AC40 | vint_object_tree_update | named |
| 57 | 1.1 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 53 | 1.0 | 0x50F780 | effect_list_expire_timed | named |

#### s7 "before mission fail": frames 20864-22891 (2027), 5888 samples, 3810 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 150 | 3.9 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 107 | 2.8 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 105 | 2.8 | 0xC65DC0 | hkpMoppLongRayVirtualMachine_func | named |
| 72 | 1.9 | 0xC0C990 | profile_stop | named |
| 68 | 1.8 | 0x926170 | coop_do_frame | named |
| 67 | 1.8 | 0x50F780 | effect_list_expire_timed | named |
| 60 | 1.6 | 0xC0BE10 | profile_start | named |
| 54 | 1.4 | 0xBBC4E0 | quicksort_pairs_u32_by_key | new(57) |
| 50 | 1.3 | 0x520790 | game_main_thread_proc | named |
| 50 | 1.3 | 0xBE2F50 | mat33_mul | named |
| 49 | 1.3 | 0xBF4C30 | aabb_transform_by_mat33_extents | new(57) |
| 46 | 1.2 | 0xA020C0 | shape_bounds_overlap_test | named |

#### s8 fail screen (game none): frames 22891-23161 (270), 352 samples, 87 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 11 | 12.6 | 0xC0C990 | profile_stop | named |
| 9 | 10.3 | 0xCDEDA0 | heap_malloc_dlmalloc | new(57) |
| 7 | 8.0 | 0xC0BE10 | profile_start | named |
| 5 | 5.7 | 0x520790 | game_main_thread_proc | named |
| 4 | 4.6 | 0xCA326E | __VEC_memcpy_fastcopy | named |
| 3 | 3.4 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 3 | 3.4 | 0xCA0850 | _memcpy | named |
| 2 | 2.3 | 0x4B7A60 | city_submit_level_objects | new(57) |

#### s9 retry load (tss02, unlabelled): frames 23161-23303 (142), 256 samples, 74 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 7 | 9.5 | 0x4B7A60 | city_submit_level_objects | new(57) |
| 5 | 6.8 | 0xC0BE10 | profile_start | named |
| 4 | 5.4 | 0x520790 | game_main_thread_proc | named |
| 3 | 4.1 | 0x50F190 | effect_list_reclaim_dead | named |
| 3 | 4.1 | 0xBF4C30 | aabb_transform_by_mat33_extents | new(57) |
| 3 | 4.1 | 0xC0C990 | profile_stop | named |
| 2 | 2.7 | 0x4E7A10 | stl_allocate_bytes | new(57) |
| 2 | 2.7 | 0xC4F800 | hkp3AxisSweep::vf21 | named |

#### s10 "retried from checkpoint": frames 23303-32341 (9038), 32640 samples, 17170 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 417 | 2.4 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 359 | 2.1 | 0x406810 | vec3_mul_mat33_to | new(57) |
| 358 | 2.1 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 353 | 2.1 | 0xCE9E10 | rig_pose_evaluate_blended | new(57) |
| 316 | 1.8 | 0xC0C990 | profile_stop | named |
| 303 | 1.8 | 0x50F780 | effect_list_expire_timed | named |
| 268 | 1.6 | 0xBE2F50 | mat33_mul | named |
| 268 | 1.6 | 0xC65DC0 | hkpMoppLongRayVirtualMachine_func | named |
| 258 | 1.5 | 0x926170 | coop_do_frame | named |
| 250 | 1.5 | 0xC4F800 | hkp3AxisSweep::vf21 | named |
| 232 | 1.4 | 0x520790 | game_main_thread_proc | named |
| 223 | 1.3 | 0xC0BE10 | profile_start | named |

#### s11 after the replay (game none): frames 32341-34731 (2390), 4992 samples, 1396 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 86 | 6.2 | 0xC0C990 | profile_stop | named |
| 57 | 4.1 | 0xC0BE10 | profile_start | named |
| 55 | 3.9 | 0x520790 | game_main_thread_proc | named |
| 51 | 3.7 | 0x4B7A60 | city_submit_level_objects | new(57) |
| 49 | 3.5 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 41 | 2.9 | 0xBF4C30 | aabb_transform_by_mat33_extents | new(57) |
| 33 | 2.4 | 0x50F190 | effect_list_reclaim_dead | named |
| 31 | 2.2 | 0xADF430 | vehicle_render_entry_fill | new(57) |
| 30 | 2.1 | 0xA020C0 | shape_bounds_overlap_test | named |
| 28 | 2.0 | 0xAC8040 | ring_list_notify_type7_nodes | new(57) |
| 26 | 1.9 | 0xCDEDA0 | heap_malloc_dlmalloc | new(57) |
| 25 | 1.8 | 0xA024C0 | occluder_encloses_shape_points | new(57) |

#### s12 "mission ended": frames 34731-34946 (215), 640 samples, 343 game-thread

| samples | % | address | function | kind |
|---:|---:|---|---|---|
| 22 | 6.4 | 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | new(57) |
| 8 | 2.3 | 0x6E79D0 | mempool_compact_step | named |
| 7 | 2.0 | 0x50F780 | effect_list_expire_timed | named |
| 7 | 2.0 | 0xC0C990 | profile_stop | named |
| 7 | 2.0 | 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | new(57) |
| 7 | 2.0 | 0xD96670 | hkp_contact_constraint_solve_step | new(57) |
| 6 | 1.7 | 0x520790 | game_main_thread_proc | named |
| 6 | 1.7 | 0xBBC4E0 | quicksort_pairs_u32_by_key | new(57) |

Rough subsystem split of the game thread, by keywords in the (now mostly named) function names, so only a guide:
| phase | Havok | animation | render prep/effects | game objects/AI | UI | Lua | profiler | math/libc | other |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| s1 free roam | 12 | 5 | 16 | 8 | 2 | 1 | 13 | 12 | 31 |
| s2 "Pre mission start" | 16 | 3 | 12 | 5 | 5 | 0 | 3 | 24 | 33 |
| s3 mission tss02 run 1 | 22 | 6 | 12 | 8 | 3 | 0 | 4 | 14 | 31 |
| s4 run 2 tss02 | 0 | 0 | 7 | 2 | 2 | 0 | 46 | 20 | 24 |
| s5 "cutscene start" | 17 | 6 | 10 | 8 | 3 | 1 | 7 | 15 | 32 |
| s6 "before entering courthouse" | 17 | 3 | 13 | 10 | 4 | 1 | 8 | 13 | 31 |
| s7 "before mission fail" | 23 | 3 | 12 | 9 | 3 | 0 | 3 | 15 | 31 |
| s8 fail screen | 13 | 2 | 11 | 0 | 1 | 0 | 21 | 26 | 25 |
| s9 retry load | 20 | 1 | 19 | 5 | 1 | 0 | 11 | 15 | 27 |
| s10 "retried from checkpoint" | 18 | 7 | 12 | 9 | 2 | 1 | 3 | 16 | 31 |
| s11 after the replay | 11 | 3 | 23 | 5 | 1 | 1 | 10 | 17 | 28 |
| s12 "mission ended" | 24 | 5 | 14 | 6 | 2 | 1 | 3 | 15 | 31 |

Reading the tables:

- **Flat profile.** No function exceeds 4 percent in any long phase (the maximum is `profile_stop` at 4.6 percent in s6). The top of every phase is the same fifteen: `profile_stop` / `profile_start` (game profiler, 3-8
  percent together, 5.0 overall), `game_main_thread_proc`, `hkpMoppAabbCastVirtualMachine_func` (0xC81C20, new), `hkpEntityAabbUtil_entityBatchRecalcAabb` (0xD81050, new),
  `vec3_mul_mat33_to` (0x406810, new), the animation pose chain (`rig_pose_evaluate_blended`, `rig_pose_resolve_cached`, `rig_pose_sample_tracks`,
  `rig_bone_matrices_transpose_to_palette`, `anim_track_*`), `effect_list_*`, `shape_bounds_overlap_test`, `coop_do_frame`, `vint_object_tree_update`, `mat33_mul`.
- **Havok is 17-24 percent of the game thread in missions** and the MOPP byte-code machines are the core: AabbCast 0xC81C20 (1094 samples), LongRay 0xC65DC0 (680),
  Obb 0xC65180 (332). The broadphase batch recalculation 0xD81050 (1285) is the single hottest function of the log.
- **Animation is 3-7 percent**, highest in the replay (s10) and at the mission end, where many characters are posed (ragdoll, melee).
- **Lua / mission script is 0-1 percent.** `luaV_execute` shows 7, 12, 6, 3 and 26 samples in s3, s5, s6, s7 and s10 (and 1 in s12). Mission logic is not a cost.
- **Heap.** `heap_malloc_dlmalloc` (0xCDEDA0, new) is a Doug Lea malloc: 472 samples overall and 9 of the 87 samples of the fail screen.

## 3. What differs from free roam

Mission phases (s3, s5, s6, s7, s10; 46,020 samples) against free roam (s1, s2, s11; 1,658 samples). The free-roam baseline is small, so only the strong ratios mean anything:

| ratio | in mission | in free roam | address | function |
|---:|---:|---:|---|---|
| x11.0 | 0.33% | 0.00% | 0x9A2670 | human_movement_rig_step |
| x9.7 | 1.46% | 0.12% | 0xC65DC0 | hkpMoppLongRayVirtualMachine_func |
| x9.2 | 0.28% | 0.00% | 0x72F4D0 | hk_memory_pages_in_use_bytes |
| x8.1 | 0.25% | 0.00% | 0xC3C8F0 | hkp_constraint_atoms_build_jacobians |
| x6.3 | 0.19% | 0.00% | 0xBE0950 | segment_segment_distance_sq |
| x6.1 | 0.18% | 0.00% | 0xD40B90 | hkpListShape::vf5_at10 |
| x5.5 | 0.17% | 0.00% | 0xBE0250 | segment_point_distance_sq |
| x5.5 | 0.17% | 0.00% | 0x6D9E40 | human_compute_rig_bounds |
| x5.0 | 0.15% | 0.00% | 0xAA77C0 | vehicle_seat_door_part |

The other way round, free roam after the mission (s11) over-represents the world render prep: `vehicle_render_entry_fill` x4.0 (2.2 percent), `city_submit_level_objects`
x3.8 (3.7 percent), `aabb_transform_by_mat33_extents` x2.7, `effect_list_reclaim_dead` x2.2, profiler x2.1. The mission cost is physics (ray casts, constraint solving, list
and mesh shapes of interiors) plus close-combat geometry (segment distances for melee and capsule tests); the free-roam cost is the city render list.

Signatures of individual phases (share of the phase against the other tss02 phases):

- **`cutscene start` (s5):** `havok_game_filter::vf1_at8` x9.7 (90 samples), 0xD2D080 `hkp_agent_linear_cast_shape_pair` x13 (35), `hkpListShape::vf4_at10` x6, 0xBEA1B0 `view_volume_test_cone_part` x6.5,
  `segment_point_distance_sq` x4.7: collision filtering and linear casts, i.e. the camera / actor collision of a scripted cutscene plus the view volume tests.
- **`before entering courthouse` (s6):** `level_lights_collect` x87 (17 samples, the one function that is nearly exclusive to this window), `human::update_grid_index` x5.8,
  `character_table_find_index_by_hash` x5.4, `skybox_setup_for_chunks` x3.5, `view_clip_planes_build` x4: an area change. Entering the interior makes the world re-collect lights, switch the skybox and
  re-index characters.
- **`before mission fail` (s7):** `object_create_renderable` x3.7, `hkp3AxisSweep::vf21` x2.5, `hkpMoppLongRay` x2.3, `level_object_update_mover` x2.7, `vehicle` helpers x2.2: the fight with many
  spawned objects and vehicles.
- **`retried from checkpoint` (s10):** 0xCF2C00 `rig_capsule_sweep_resolve` x21 (31 samples), `mesh_decals_fade_update` x12.5 and `mesh_decals_frame_update` x5.7, `segment_segment_distance_sq` x12 (77),
  `rig_ik_chain_solve` x9.6 (43), `ai_order_k1f_on_event` x4: capsule sweeps, decal fading and inverse-kinematics solves, none of which dominate the first pass. The retry plays a different, more
  melee-heavy stretch (or the same stretch after a state that keeps more decals alive).

## 4. What fail, retry and end exercise that free roam does not

- **Mission fail (s7 then s8).** The fail itself is not visible as a function: nothing named `mission_fail*`, `checkpoint*` or `fade*` appears in s7 or s8. The fail screen window (270 frames, 87 samples) is
  dominated by memory work: `heap_malloc_dlmalloc` 9 and `_memcpy` / `__VEC_memcpy_fastcopy` 7 (18 percent of the window), plus `std_string_copy_realloc`, `stl_allocate_bytes` and `read_string`. That is
  string and buffer allocation (the menu text and the snapshot copy), then ordinary rendering prep. `screen_flash_render`, `glares_render_snapshot`, `time_scale_restore_check` and
  `ambient_zone_effects_update` each appear once, so slow-motion restore and the screen flash/fade are exercised but cheap.
- **Retry from checkpoint (s9 then s10).** The load window (142 frames, 74 samples) is again city submission (`city_submit_level_objects` 7, `aabb_transform_by_mat33_extents` 3, `effect_list_reclaim_dead` 3,
  `stl_allocate_bytes` 2, `tod_zone_fade_step_vec3` 1): the world is rebuilt and the time-of-day zone fade restarts. Nothing in the checkpoint code is hot enough to reach the sampler (the checkpoint
  restore happens inside the 270 + 142 frame transition and costs a few samples of memcpy / malloc). The replay itself (s10) is the first-pass mission plus: capsule sweeps, IK, decals.
- **Mission end (s11, s12).** After the replay the game is back in `game none`. s11 is ordinary free roam (see section 3). The `mission ended` window (215 frames, 343 samples) leads with
  `hkpEntityAabbUtil_entityBatchRecalcAabb` 22 (6.4 percent), `mempool_compact_step` 8, `effect_list_expire_timed` 7 and the MOPP machines: physics settling as the mission objects are released. The
  mission script, the objective HUD and the cutscene code are NOT visible at all in this window; if the user wants the mission-complete path (stats, rewards, `mission_end` Lua callbacks) a log
  with F8 pressed at the moment of the end screen and a window of 1000+ frames is needed.
- **Cutscenes** (s5 `cutscene start`, and the cutscene part of s3): `havok_game_filter`, linear-cast agents and `view_volume_test` rise; `cutscene_object_is_cutscene_character` shows 3 samples (s6) and
  `camera_follow_update` 3-5; the scripted cutscene camera and animation clips are cheap compared with the physics running underneath them.
- **Fades:** `screen_fade_apply` 1 sample in each of s3 / s5 / s11, `mesh_decals_fade_update` 26 only in the replay (decals, not screen fades), `object_fade_state_update` 2-6, `tod_zone_fade_step_vec3` 1-4.
  The screen-fade code is not a cost anywhere.
- **Scripts:** `luaV_execute`, `luaD_call`, `script_thread_create`, `script_thread_kill`, `script_heap_alloc` / `script_heap_free` total under 1 percent in every phase; the thread create / kill pair
  appears in s3, s5, s6 and s10 only (2-4 samples each), so mission script threads are spawned during play, not at fail or end.

## 5. Names added (130 functions)

Every address was checked as a real function start (prologue, CC / ret padding in front, a genuine call target; bodies walked, CC bytes inside immediates ignored) and every name
was checked against `docs/functions.csv` and against the other new names. Confidence: inferred 91, confirmed 39. They cover 17056 of the 48228 game-thread samples. The full
conventions and evidence of each are in `docs/renames_log57.csv`; here are the addresses, samples (all sections of the 04c run 2 + run 1 tss02 data) and subsystem.

Notes on the sweep: 0xDA2845 is a label inside the CRT asin/acos helper (real start 0xDA280C, named); 0xDA1923 is a shared tail of `__ctrandisp1/2`; 0xC7F860 runs straight into a second
function at 0xC7F9C0 (4 stack args, unnamed); `mission_report.py` would have credited many of these to `std::out_of_range::vf2`, `hkp3AxisSweep::vf*` and similar neighbours (the 0x4F1E72 caller
of `heap_malloc_dlmalloc` is inside a function wrongly named `std::out_of_range::vf2`).

| Address | New name | Samples | Subsystem | Confidence |
|---|---|---:|---|---|
| 0xD81050 | hkpEntityAabbUtil_entityBatchRecalcAabb | 1285 | havok | inferred |
| 0xC81C20 | hkpMoppAabbCastVirtualMachine_func | 1094 | havok | confirmed |
| 0x406810 | vec3_mul_mat33_to | 760 | math | confirmed |
| 0xCE9E10 | rig_pose_evaluate_blended | 736 | anim | inferred |
| 0xBF4C30 | aabb_transform_by_mat33_extents | 544 | math | confirmed |
| 0x4B7A60 | city_submit_level_objects | 503 | render | inferred |
| 0xCDEDA0 | heap_malloc_dlmalloc | 472 | memory | confirmed |
| 0xCE7D00 | rig_pose_sample_tracks | 355 | anim | inferred |
| 0xBBC4E0 | quicksort_pairs_u32_by_key | 331 | util | confirmed |
| 0xD96670 | hkp_contact_constraint_solve_step | 311 | havok | inferred |
| 0xCE9990 | rig_pose_resolve_cached | 300 | anim | inferred |
| 0xADF430 | vehicle_render_entry_fill | 286 | vehicle | inferred |
| 0xCE4A40 | rig_bone_matrices_transpose_to_palette | 280 | anim | confirmed |
| 0xCE5140 | anim_track_key_header_decode | 248 | anim | inferred |
| 0xBFD510 | bsp_point_in_volume | 244 | math | confirmed |
| 0xC5D160 | quicksort_u64_pairs | 237 | havok | confirmed |
| 0xBE0400 | segment_closest_t_dist_sq | 202 | math | inferred |
| 0x6E61F0 | transform_table_find_index_by_name | 193 | anim | inferred |
| 0xAC8040 | ring_list_notify_type7_nodes | 184 | vehicle | inferred |
| 0x719FF0 | transform_compose_to_esi | 183 | math | confirmed |
| 0x4B7700 | city_view_batches_collect | 182 | render | inferred |
| 0xDA0DC0 | crt_ci_atan | 176 | crt | confirmed |
| 0x501850 | nav_tri_bounds_xz | 164 | nav | confirmed |
| 0xD961A0 | hkp_constraint_elements_within_limits | 159 | havok | inferred |
| 0xD42410 | hkp_meshshape_find_subpart_entry | 154 | havok | inferred |
| 0x9A2670 | human_movement_rig_step | 154 | human | inferred |
| 0xCA04D0 | crt_ci_sqrt | 151 | crt | confirmed |
| 0x7030B0 | havok_collidable_mesh_subpart_ids_get | 151 | havok | inferred |
| 0xC74280 | hkpGsk_cast_convex_pair | 142 | havok | inferred |
| 0xAE48A0 | vehicles_post_physics_step_update | 139 | vehicle | inferred |
| 0xAC24F0 | vehicle_wheels_update_skid_effects | 136 | vehicle | inferred |
| 0xAC12D0 | human_attachment_pose_update | 133 | human | inferred |
| 0xA024C0 | occluder_encloses_shape_points | 132 | render | inferred |
| 0x72F4D0 | hk_memory_pages_in_use_bytes | 130 | havok | confirmed |
| 0x4E7A10 | stl_allocate_bytes | 127 | stl | confirmed |
| 0xC80EE0 | hkpCapsuleTriangle_closest_points | 126 | havok | inferred |
| 0x91EA50 | chunk_object_velocity_from_transform_delta | 124 | world | inferred |
| 0xDA0B60 | _CIsin | 121 | crt | confirmed |
| 0xDA1923 | crt_trandisp_status_check_tail | 116 | crt | inferred |
| 0x4E74D0 | std_string_assign_ptr_len | 115 | crt | confirmed |
| 0xC3C8F0 | hkp_constraint_atoms_build_jacobians | 114 | havok | inferred |
| 0x509700 | effect_object_submit_render | 111 | effect | inferred |
| 0xC6DE00 | hkp3AxisSweep_narrow_query_node_walk | 110 | havok | inferred |
| 0x557870 | hkvector4_dot3_store | 109 | havok | confirmed |
| 0xCB70B8 | crt_double_hiword_exp_check | 108 | crt | inferred |
| 0x70EC00 | hkshape_aabb_from_halfextents_transform | 107 | havok | confirmed |
| 0xCFFF20 | rl_variant_copy_pass_constants | 105 | render | inferred |
| 0xADC5E0 | vehicle_wheel_model_handle_get | 102 | vehicle | inferred |
| 0xDA02B0 | _CIcos | 101 | crt | confirmed |
| 0xBDE060 | vec3_normalize_in_place_ret_len | 99 | math | confirmed |
| 0x530580 | world_mesh_compute_visibility | 95 | render | inferred |
| 0x70ED00 | hkvector4_splat_lane | 93 | havok | confirmed |
| 0xCE2EF0 | anim_keyframe_bsearch_u8 | 91 | anim | confirmed |
| 0x9F8FA0 | level_object_update_mover | 90 | object | inferred |
| 0xC8DA20 | hkp3AxisSweep_query_1axis_range | 89 | havok | inferred |
| 0xA7A410 | streaming_chunk_requests_collect_sorted | 87 | streaming | inferred |
| 0xBE0950 | segment_segment_distance_sq | 87 | math | inferred |
| 0x731E80 | player_human_move_input_update | 86 | human | inferred |
| 0xAE0350 | vehicle_build_render_instance | 84 | vehicle | inferred |
| 0xD9B500 | hkpSymmetricAgent_linear_cast_swapped | 82 | havok | inferred |
| 0xD42780 | hkpExtendedMeshShape_get_vec3_by_key | 81 | havok | inferred |
| 0x52F620 | object_create_renderable | 81 | render | inferred |
| 0xC22F20 | anim_track_key_code_lookup | 79 | anim | inferred |
| 0xAD4360 | rgb_float_to_bytes_clamped | 78 | util | inferred |
| 0xBE0250 | segment_point_distance_sq | 77 | math | confirmed |
| 0x6D9E40 | human_compute_rig_bounds | 76 | human | inferred |
| 0xA023F0 | view_clip_planes_test_aabb | 74 | render | confirmed |
| 0xA01D30 | clip_planes_set_edge_plane | 74 | render | inferred |
| 0xAF15A0 | vehicle_seat_slot_info_get | 73 | vehicle | inferred |
| 0xAA3420 | copy_48_bytes_ecx_to_eax | 72 | util | confirmed |
| 0x9ADF30 | human_render_attachments_build | 71 | human | confirmed |
| 0xBDDB40 | vec3_normalize_to | 70 | math | confirmed |
| 0x9A5AD0 | human_animation_layer_state_update | 70 | human | inferred |
| 0x72F110 | hk_small_block_alloc | 70 | havok | inferred |
| 0x735C80 | object_get_body_world_position | 68 | physics | inferred |
| 0xD800D0 | hkp_simulation_collide_entity_batch | 67 | havok | inferred |
| 0xC7F860 | hkp_triangle_edge_inv_lengths_build | 67 | havok | inferred |
| 0xCE5780 | anim_track_mask_clear_every_third_bit | 66 | anim | inferred |
| 0xBE9830 | aabb_contains_point | 66 | math | confirmed |
| 0x52EDA0 | world_mesh_submit_full | 65 | render | inferred |
| 0x4E8770 | mat44_mul_regs | 65 | math | inferred |
| 0xC6D090 | hkp_gsk_linear_cast_toi | 64 | havok | inferred |
| 0xCD4990 | lua_table_get_str | 63 | script | confirmed |
| 0xBE4C80 | mat33_from_forward_up | 62 | math | inferred |
| 0xC7F3D0 | hkpGsk_closest_point_simplex_region | 62 | havok | inferred |
| 0xC5BC90 | hkp_toi_event_collect_pairs | 62 | havok | inferred |
| 0xA7C760 | zone_remap_word_by_id | 59 | city | confirmed |
| 0xBF64E0 | point_in_aabb_test | 58 | math | inferred |
| 0x4B9D60 | chunk_visibility_cull | 57 | render | inferred |
| 0x9289A0 | object_ring_count_flag80_members | 55 | coop | confirmed |
| 0x50EF90 | effect_instance_set_transform | 55 | effect | inferred |
| 0xD641B0 | hkpListenerArray_notify_reverse | 54 | havok | inferred |
| 0x9AF360 | humans_submit_render_batches | 54 | human | inferred |
| 0x508750 | effect_instance_add_lights | 53 | effect | confirmed |
| 0xAFF1E0 | hkvector4_mul_hkmatrix3_rows | 52 | havok | confirmed |
| 0xCEAB20 | rig_ik_chain_solve | 51 | anim | inferred |
| 0xC41320 | hkp_constraint_atoms_solve | 51 | havok | inferred |
| 0xAAE940 | vehicle_part_sample_fill | 50 | vehicle | inferred |
| 0x91E1F0 | chunk_object_update_lod_state | 50 | object | inferred |
| 0xA88F80 | streamman_slot_entry_update_state | 50 | streamman | confirmed |
| 0x4E7910 | std_string_copy_realloc | 50 | crt | confirmed |
| 0xA7AA80 | streaming_chunk_candidates_build | 50 | streamman | inferred |
| 0xAAB4D0 | vehicle_has_occupant | 49 | vehicle | inferred |
| 0x508AD0 | effect_object_compute_transform | 49 | effects | inferred |
| 0xD93A10 | hkTransform_set_mul_inv | 49 | havok | inferred |
| 0xA9F370 | vehicle_occupants_load_accumulate | 48 | vehicle | inferred |
| 0x4A9A60 | zone_id_at_position | 48 | city | confirmed |
| 0xD9FA70 | hkp_agent_toi_process_entries | 48 | havok | inferred |
| 0xBE43E0 | mat33_orthonormalize | 47 | math | inferred |
| 0xB80B60 | distant_vehicles_build_render_lists | 45 | render | confirmed |
| 0xD2D080 | hkp_agent_linear_cast_shape_pair | 40 | havok | inferred |
| 0xC82C00 | hkpMoppAabbCastVirtualMachine_query | 39 | havok | inferred |
| 0x6E8440 | relptr_resolve_edx | 37 | memory | inferred |
| 0xCF2C00 | rig_capsule_sweep_resolve | 33 | anim | inferred |
| 0xBEA1B0 | view_volume_test_cone_part | 31 | render | inferred |
| 0xAF1460 | vehicle_scale_vec3_from_tables | 28 | vehicle | inferred |
| 0xAD0370 | vehicle_drive_effects_update | 25 | vehicle | inferred |
| 0xBF40A0 | thread_slot_find_current | 24 | threads | confirmed |
| 0xB7DFF0 | distant_vehicle_light_fade_step | 23 | render | inferred |
| 0xADC4D0 | vehicle_wheel_model_handle_get_checked | 23 | vehicle | inferred |
| 0xBD22B0 | hk_pointer_map_insert | 23 | havok | confirmed |
| 0xA0FB70 | nav_point_in_tri_bounds_xz | 21 | nav | inferred |
| 0xB1BCC0 | vehicle_interaction_anim_lookup_by_key | 21 | vehicle | inferred |
| 0xA06CF0 | nav_orient2d_sign_xz | 20 | nav | inferred |
| 0x4D9740 | light_instance_register_checked | 19 | effects | inferred |
| 0xB1C8B0 | road_path_open_list_sift_down | 18 | vehicle_ai | inferred |
| 0x9BC5D0 | human_ai_pick_target | 16 | ai | inferred |
| 0xABD7C0 | vehicle_world_point_to_local_clamped | 15 | vehicle | inferred |
| 0xA10110 | nav_open_list_sift_down | 15 | nav | inferred |
| 0xDA280C | crt_asin_acos_sqrt_1_minus_x2 | 0 | crt | inferred |

Names that another agent put into docs/functions.csv while this round ran (kept, not renamed; the roles agree with my reading):
- 0xD098B0: my name `world_mesh_set_batch_params`, name now in docs/functions.csv: `world_mesh_record_set_lod_list`
- 0x4B7650: my name `vec3_distance_sq_thread_guard`, name now in docs/functions.csv: `view_distance_sq_scaled`
- 0x4F4E60: my name `memcmp_regs_eax_len`, name now in docs/functions.csv: `memcmp_regs`
- 0xBF65F0: my name `point_aabb_outside_offset_dist_sq`, name now in docs/functions.csv: `aabb_point_dist_sq_x87`

Still unnamed and hot: 0xD45C40 (45), 0xDA2845 (inside a CRT helper, 130), 0x9A1970 (44), 0x946E90 (46), 0x50FD40 (46), 0x72CD60 (45); everything below 40 samples is a long tail (the unnamed remainder is 19
percent of the game thread, spread over about 1,500 functions).

## 6. Fifteen port candidates (hot and tractable)

All are leaves or near-leaves, no globals except constants, so a differential test on live calls is straightforward. Priority order by samples.

| # | Address | Name | Game samples |
|---|---|---|---:|
| 1 | 0x406810 | vec3_mul_mat33_to | 760 |
| 2 | 0xBF4C30 | aabb_transform_by_mat33_extents | 544 |
| 3 | 0xBBC4E0 | quicksort_pairs_u32_by_key | 331 |
| 4 | 0xCE4A40 | rig_bone_matrices_transpose_to_palette | 280 |
| 5 | 0xBFD510 | bsp_point_in_volume | 244 |
| 6 | 0xC5D160 | quicksort_u64_pairs | 237 |
| 7 | 0x4F4E60 | memcmp_regs | 224 |
| 8 | 0xBE0400 | segment_closest_t_dist_sq | 202 |
| 9 | 0x719FF0 | transform_compose_to_esi | 183 |
| 10 | 0x501850 | nav_tri_bounds_xz | 164 |
| 11 | 0xD098B0 | world_mesh_record_set_lod_list | 146 |
| 12 | 0xA024C0 | occluder_encloses_shape_points | 132 |
| 13 | 0xBDE060 | vec3_normalize_in_place_ret_len | 99 |
| 14 | 0xBE0250 | segment_point_distance_sq | 77 |
| 15 | 0xBE9830 | aabb_contains_point | 66 |

Recipes (calling convention, every output, preserved registers, callees, compare polarity, writes). x87 code rounds to f32 on every `fstp dword`: keep that in Rust by computing in f32 and
storing after each operation; where the original mixes x87 and SSE the polarity is given as the branch.

**1. vec3_mul_mat33_to (0x406810, 760 samples).** Inputs ECX=&mat33 (row r at +0xC*r), EDX=&in vec3, one stack arg &out (caller pops, plain ret). Output memory out[0..2] and EAX=&out; nothing in XMM0/ST0 (x87 empty on exit). All other registers preserved, no callees. Maths in f32 with a store after every product: xt=in.x*row0, yt=in.y*row1, zt=in.z*row2 (component-wise), out=(xt+yt)+zt. All of in is read before out is written, so out may alias in. No compares. 51 callers, so differential-test with 3000 calls against the original. (57)

**2. aabb_transform_by_mat33_extents (0xBF4C30, 544 samples).** Inputs EDI=&mat33 (M[i][j]=edi[3i+j]), ESI=&lo vec3, EDX=&hi vec3, ECX=&out[6] (min at +0, max at +0xC); no stack args, plain ret. Outputs only memory out[0..5]; EAX, ESI, EDI, EDX untouched (obb_to_aabb relies on EAX surviving); x87 empty. No callees. For j in 0..2: min_j = sum_i min(M[i][j]*lo[i], M[i][j]*hi[i]), max_j = sum_i max(...) accumulated i=0,1,2 in f32 (per-store rounding); the first term is selected by fcomi on (a-b) with jb (a<b picks a for the min, jae keeps a for the max), later terms by fcomi against the running values: copy the compare operands from 0xBF4D2B/0xBF4D8C blocks when porting. Test against obb_to_aabb results. (57)

**3. quicksort_pairs_u32_by_key (0xBBC4E0, 331 samples).** cdecl (array, lo, hi, unused) at [ebp+8..0x14]; caller pops 0x10 after the recursive call and the outer caller cleans the 4 args; returns nothing. 8-byte records, u32 key at +0 compared UNSIGNED, u32 value at +4 carried along. Pivot = key of arr[(lo+hi)>>1] (signed sar). Hoare loop: i=lo, j=hi; while arr[i].key < pivot (jb) i++; while pivot < arr[j].key (jb after cmp ecx,[..]) j--; if i<j swap both dwords; i++, j--; repeat while i<=j. Then if lo<j recurse(arr,lo,j,arg4); if i<hi set lo=i and loop. Preserves EBX/ESI/EDI/EBP; writes only the array. Not stable, so port the exact algorithm (equal keys must end up in the same order). Callers 0xD81050 (+0xB3B) and 0xBBBC70. (57)

**4. rig_bone_matrices_transpose_to_palette (0xCE4A40, 280 samples).** Inputs ECX=rig object (source = [ecx], array of 12-float bone matrices, 0x30 stride), one stack arg &dst (callee pops, ret 4). No outputs besides memory. n=[[rig+0x228]+4]; for i<n: if the signed word [[rig+0x228]+0x108+2i] >= 0 write dst[c*4+r]=src[r*3+c] for r<4, c<3 (src floats 0,3,6,9 -> dst 0..3, 1,4,7,10 -> dst 4..7, 2,5,8,11 -> dst 8..11); both pointers advance 0x30 per bone whether written or not. Preserves ESI/EDI (clobbers EAX/ECX/EDX); no callees; no compares except the signed-word test (jl skips). Sole caller 0xCEB570. (57)

**5. bsp_point_in_volume (0xBFD510, 244 samples).** Inputs ECX=tree (planes at [ecx+8], 16 B: nx,ny,nz,d; nodes at [ecx+0xC], 6 B: u16 plane, u16 child_a, u16 child_b), EAX=&point vec3, one stack f32 margin (plain ret, caller pops); output AL (1 inside, 0 outside); preserves ESI, EBX untouched, x87 empty. Loop: dist=(nx*px+ny*py+nz*pz)+d with f32 stores (sum then +d stored through [esp+0x14]); compare -margin (xmm0 = -0.0 - margin) against dist with fcomi: if -margin <= dist (jbe) take child_b else child_a; child index c is a node number (byte offset c*6 from the node base, via lea x3 / lea x2); c==0 terminates: AL=1 when it came from child_b, AL=0 from child_a. No callees, no writes. 39 callers. (57)

**6. quicksort_u64_pairs (0xC5D160, 237 samples).** Same shape as quicksort_pairs_u32_by_key (0xBBC4E0) but orders 8-byte records lexicographically by (dword0, dword1), both UNSIGNED: i-loop while arr[i] < pivot (cmp k0 jb; jne exit; cmp k1 jae exit), j-loop while pivot < arr[j] (cmp pivot.k0 jb; jne; cmp pivot.k1 jae). cdecl (array, lo, hi, arg4) at [ebp+8..0x14], returns nothing, recursion pushes the 4 args and adds 0x10. Writes only the array; preserves EBX/ESI/EDI/EBP. Callers all inside 0xC5BC90 / itself. (57)

**7. memcmp_regs (0x4F4E60, 224 samples).** Inputs EAX=length, ECX=ptr a, one stack arg ptr b (plain ret, caller pops); output EAX: 0 when equal, else +1 when the first differing byte of b exceeds that of a (jg on b[i]-a[i], zero-extended) and -1 otherwise; clobbers EAX, ECX, EDX; preserves ESI/EDI (pushed); no memory writes. Compare dword by dword while length>=4 (jne leaves to the byte loop at the differing dword), then bytes. Zero length returns 0. 6 callers. Concurrently named memcmp_regs in docs/functions.csv. (57)

**8. segment_closest_t_dist_sq (0xBE0400, 202 samples).** Inputs EAX=&P, EDX=&A, one stack arg &B (plain ret, caller pops), ESI=&out_t (float); outputs ST0 = squared distance from P to the closest point on segment A-B (f32) and [ESI]=t clamped to [0,1] (constant 1.0 at 0xDD518C); all other registers preserved (EBX/EDI/EBP untouched), x87 holds exactly the result. d=B-A, t=dot(P-A,d)/dot(d,d); degenerate d (dot below the double at 0xE5C460) tail-jumps to vec3_distance_sq (0xBDEA30, ECX=&a, EAX=&b) after t=0. Every intermediate goes through an f32 stack slot. Callers road query callbacks and spline_closest_param. Compare polarity: t<0 -> 0 (jbe), t>1 -> 1. Differential test mandatory. (57)

**9. transform_compose_to_esi (0x719FF0, 183 samples).** Inputs EDI=&A (mat33 at +0, pos at +0x24), ESI=&out (12 floats), one stack arg &B (callee pops, ret 4); output memory out[0..11] and EAX=ESI; EBX saved/restored, EDI/ESI preserved. Steps: tmp=vec3_mul_mat33_to(ECX=EDI, EDX=B+0x24, stack &tmp) (0x406810) ; pos=tmp+A.pos ([edi+0x24..0x2C]) stored into [esp..esp+8]; EAX=&tmp2 ([esp+0x20]), ECX=EDI, stack B -> mat33_mul (0xBE2F50, returns EAX=&out_rot); copy the 9 rotation floats to out[0..8] then pos to out[9..11] at +0x24/+0x28/+0x2C. out.rot = A.rot x B.rot, out.pos = B.pos*A.rot + A.pos. No compares. Callers ragdoll_apply_hit_impulse, vehicle_body_rig_update, rig code. (57)

**10. nav_tri_bounds_xz (0x501850, 164 samples).** Inputs ECX=&out_min (x at +0, z at +4), EDX=&out_max (x at +0, z at +4), one stack arg &tri whose first three dwords are vertex pointers (vertex x at +0, z at +8); plain ret; preserves EBX/ESI/EDI; no outputs but memory; x87 empty. min pass: start from v0, then for v1 and v2 keep the smaller (fld [ecx]; fld [v]; fcompp; test ah,0x41; jne picks the new vertex when new<=cur); max pass starts from v0 and replaces when the vertex is larger (fcompp; test ah,5; jp/jnp: C0|C2). Pure leaf. Callers nav_tri_point_inside_xz and siblings 0xA0FB70/0xA0FC40/0xA0FD00 plus 0x502610. (57)

**11. world_mesh_record_set_lod_list (0xD098B0, 146 samples).** Concurrently named world_mesh_record_set_lod_list. Inputs ECX=count, EDX=&record, ESI=&u32 source list, stack (byte flag, f32 value) with ret 8; no return value, EAX clobbered; EBX/EBP/EDI preserved. If count>0 (signed) copy ESI[i] to record+0x338+4i; then store the f32 at +0x330, count at +0x334, the byte at +0x349 and 0xFF at +0x348. No compares beyond the count test, no callees. Called only from the world_mesh_submit family. (57)

**12. occluder_encloses_shape_points (0xA024C0, 132 samples).** Inputs ECX=&occluder (plane count at [ecx], planes from +0x18, 0x10 stride: nx,ny,nz,d), one stack arg &shape (u32 table index at [shape]; point count in table 0xE82680 (0x80 per entry, +4), points at shape+4, 0xC stride); output AL (1 only if every shape point is behind every plane); EBX/EBP/ESI/EDI preserved; no callees, no memory writes. For each plane and each point: dist=nx*px+ny*py+nz*pz+d with f32 stores; fcomp then test ah,1: dist<0 continues, dist>=0 returns 0. Caller occluders_select (stride 0x220 occluders). (57)

**13. vec3_normalize_in_place_ret_len (0xBDE060, 99 samples).** Inputs ESI=&v (vec3); output v normalised in place and XMM0 = length (f32, movss from the stored sqrt result); no zero guard; ESI preserved; frame sub 0x14 + push ecx. Steps: len_sq in the x87 order of 0xBDE06F-0xBDE094 (each load stored to f32), call sqrtf_wrapper (0x40EA20, f32 argument at [esp], returns ST0; no stack cleanup needed because the arg slot is part of the frame), XMM0=len, inv=1.0/len via fdivrp, v.x=x*inv, v.y=y*inv, v.z=z*inv with x,y,z read before any write. 40 callers. Sibling vec3_normalize_in_place (0xBDDFF0) is the same body without the XMM0 result. (57)

**14. segment_point_distance_sq (0xBE0250, 77 samples).** Inputs EAX=&S1 (segment end), EDX=&S0 (segment start), one stack arg &P (plain ret, caller pops), ESI=&out closest point or 0; output ST0 = squared distance |P - closest|^2 (f32) and, when ESI!=0, out[0..2]=S0+t*(S1-S0); preserves EBX/EDI/EBP. t=dot(P-S0,d)/dot(d,d) clamped to [0,1] (1.0 at 0xDD518C); degenerate when dot(d,d) < double 0xE5C460: closest=S0 and tail-jump to vec3_distance_sq (0xBDEA30). Intermediates through f32 stack slots. 33 callers, 3 of them inside segment_segment_distance_sq (0xBE0950). (57)

**15. aabb_contains_point (0xBE9830, 66 samples).** Inputs ECX=&min, ESI=&max, EDX=&point (all vec3 of f32); output AL (1 inside); ECX is pushed/popped, ESI/EDX preserved; x87 left empty; plain ret; no writes. Per axis (x, y, z in order): outside if min>p (fcompi, ja) or p>max (ja after the swap/compare); the point value is first stored through [esp] as f32. Return 1 only when all three axes pass; inclusive on both bounds. 29 callers in AI and vehicles. (57)

## 7. Surprises

1. The game thread has no real hot spot: the biggest function is `profile_stop` at 3.0 percent (the biggest real work is `hkpEntityAabbUtil_entityBatchRecalcAabb` at 2.7), and the top 30 together are 36 percent. A port round gets its win from many small leaves (vec3 and matrix
   math, box and segment tests) rather than from one function.
2. The two Havok MOPP byte-code machines (`AabbCast`, `LongRay`) cost more than all of the mission script, HUD and cutscene code together.
3. `profile_start` / `profile_stop` cost 5.0 percent of the game thread (3-8 per mission phase, 13 percent in the idle free-roam second); they are the game's own profiler: `profile_stop` calls timer imports through the slots 0xDB80CC, 0xDB8110 and 0xDB8114, then updates a table. A stub or a cheaper port is a cheap 4 percent.
4. The retry is not a replay of the first pass: IK, capsule sweeps and decal fading show up only there.
5. `0xCDEDA0` is a Doug Lea malloc with a 64-byte-aligned block header; it is a clean port target if the allocator ever has to move to Rust.
6. The fail, checkpoint and mission-end code is almost invisible: it costs a handful of samples (memcpy, malloc) in windows of 142-270 frames. To see it, the F8 point must be pressed at the
   start of the fail screen and the window must be longer (1,000 frames), or the logger needs a per-frame trigger on the mission-state change.
7. Doc 56 said `mission_report.py` credits samples to the nearest named function; that error was large here: 0xD81050, 0xC81C20, 0xCE9E10 and 0xCDEDA0 all sit after named Havok vtable slots
   or script heap functions of unrelated roles.
