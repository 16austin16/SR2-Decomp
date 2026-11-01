# 47: Pathfinding, navmesh, road lanes and AI driving (names round 47)

Doc 45 left the navigation block open. This round names the part of it that could be read with confidence: the navmesh node references and cells, the path request and search entry points, the path-record pools, the detour table, the road-lane network (`0x2225B64`), the spline library the vehicle AI uses, and the first layer of the vehicle AI helpers. 95 functions are in `docs/candidates_nav.csv` (none are in `functions.csv`, notes end "(47)"). 59 have the register conventions read from the full disassembly ("confirmed"); 36 were identified from a partial read, their callers and callees ("inferred", marked in the table). Addresses were checked against the call-target / padding function starts; names were checked against `functions.csv` and the other docs.

Method notes: `tools/xrefs.py` byte hits were not trusted. Callers were taken from a raw `E8` scan, then the interesting ones were confirmed by disassembling the containing function (for example `0xA1CCFE` really calls `0xA05250`, `0x99AC07` really calls `0xA14AE0`).

## Structures recovered

### Node reference (u32)

```
bits 31..20  chunk index (0..0x865), 0xFFF with bits below = none
bit  19      kind: 0 = array at chunk rec +0x240 (count +0x6C), 1 = array at rec +0x248 (count +0x7C)
bits 18..0   index into that array
0xFFFFFFFF   no node
```
`nav_node_ref_to_cell` (0xA051D0) turns it into a cell pointer, `nav_cell_to_node_ref` (0xA05250) back. It is the value kept in `[0x2203AC0]` by `player_nav_node_update` and probably in `human+0x14EC` (doc 16, not verified). The index is **not** checked against the count (only `count > 0`).

### Chunk record fields used by navigation (record from `chunk_load_from_memory`, table `0xF7AAE8`, 0x866 slots)

| Offset | Meaning |
|---|---|
| +0x08 | u16 chunk id |
| +0x1C | flags; `& 3` makes the nav lookups fail |
| +0x68 / +0x6C | section 7 first pair: navmesh shape array / count (MOPP, `havok_navmesh_shape`), +0x70 = its shape data; becomes kind 0 |
| +0x78 / +0x7C | section 8 first pair: second navmesh shape array / count, +0x80 = its shape data; becomes kind 1 |
| +0x13C / +0x140 | section 0x0A: road lane count / lane array (**0x3C bytes** each) |
| +0x144 / +0x148 | section 0x0A: junction count / array (**0x28 bytes**: key dword, 2 copied dwords, 7 lane pointers at +0x0C..+0x24, fixed up against +0x140) |
| +0x14C / +0x150 | section 0x0B path links: count / array (**0x20 bytes**: start vec3, end vec3, flags byte +0x18 (bit 1 used by walkability), link ptr +0x1C) |
| +0x154 | section 0x0C path tree over those links |
| +0x240 / +0x244 | `nav_chunk_attach`: kind-0 navmesh array object (= rec+0x68) / Havok handle (0x6F9A00) |
| +0x248 / +0x24C | kind-1 navmesh array object (= rec+0x78) / Havok handle (0x6F9CF0) |

The array object has base pointer `+0x14` (cells), u16 count `+0x10`, and `+0x20` (u16, nonzero queues the timesliced job `0xA17600`).

### Navmesh cell (0x30 bytes, node of the path graph)

| Offset | Meaning |
|---|---|
| +0x00 | three `vec3*` vertex pointers (`nav_tri_*` take this triangle array in ESI/EDI/ECX/EDX/EAX) |
| +0x18 | pointer to the cell's link refs (u32 each = chunk<<16 \| (index+1) into rec+0x150) |
| +0x1C | u8 link count |
| +0x1D | flags: 0x02 has links to test, 0x10 blocked / visited by the current search, 0x20 pair-marked |
| +0x1E | u16 index into the per-search scratch table `0x2E160C4` (24-byte records) |
| +0x20 | generation; valid only when equal to `[0x2E160BC]` |
| +0x2C | u16 chunk id (0xFFFF = none) |

### Path request (0x28 bytes, `nav_path_request_init`)

```
+0x00 int   waypoint count (clamped to 10)
+0x04 ptr   waypoint array, 20 bytes each {vec3 position, int node ref (-1), u16 0xFFFF}
+0x08 u16   flags      (+0x09 bit7 = start position valid, bit6 / bit4 / bit2 steer nav_path_find)
+0x0A u8    extra flags
+0x0C vec3  start position (zero when not given)
+0x18 vec3  goal position  (zero when the radius arg <= 0.0)
+0x24 f32   radius / limit
```
`nav_path_find(EAX = request)` returns the head of a circular list of path nodes (links `+0` next, `+4` prev, recycled through the ring `0x21FC19C`) or 0. The search marks cells with flag 0x10 and records them in `0x21FC1A0[]` (count `0x2529820`), clearing them before it returns.

### Route record pool and route (human / vehicle side)

- Pool: 512 records of **0x34 bytes** at `0x2E400D0`, free ring head `0x2203AB8`. Fields: `+0x10` node ref (bit 19 clear = road lane record, set = ped), `+0x14` f32 length (ped records) / spline (lane records), `+0x1C` next, `+0x20` prev, `+0x24/+0x28/+0x2C` = -1 on allocation, `+0x28 >> 4` lane type (index of the f32 speed table `0x223E3E8`), `+0x30` byte: 0x20 = allocated.
- Route (embedded in the owner): up to **3 entries of 12 bytes** `{f32 cost, record*, u8 flags (+8)}` at `+0`, current index byte `+0x24`, byte `+0x30`, count byte `+0x3C`. Flag bit 4 = the entry owns a record list (freed by `nav_route_clear`).

### Detour table (dynamic obstacles that cut the navmesh)

40-byte entries at `0x2DEE0B8` (handle = generation<<12 | index, generation `[0x21F2C00]` masked to 19 bits), `+8` handle, `+0xC` owner handle, `+0x10` owner kind (0 object table `0x2149C64`, 1 chunk attachments `0x2AE50E0`, 2 table `0x2674DE0`), `+0x14` u16, `+0x18` hull pointer. Free ring `0x21F2C04`, active ring `0x21F2C08`.

### Road network (traffic lanes), global `0x2225B64`

`+0` lane count, `+4` lane array (**0x3C bytes**: section 0x0A entries of every loaded chunk), `+0x58/+0x5C` junction table (0x14-byte entries keyed by dword), `+0x70` volume used by `0xA8D010`, `+0x84` spatial tree walked by `0xBF73B0/0xBF77B0/0xBF78B0`. Lane fields seen: `+0x10` flags/ref (bit 19 clear = lane, `0x20000000` skip, `0x10`), `+0x14` f32 length, `+0x2A` u16 head of the lane's claim ring, `+0x36/+0x38` neighbour lane indices. **Lane claims**: 12-byte entries in `0x223E410` (u16 next, u16 prev, +4 owner, +8), free list head u16 `0xE87B4C`. Queries share the globals `0x2FA8CC0..0x2FA8D28`.

### Spline (`spline_eval_position` and friends, math library at 0xC03910..0xC044D0)

`{vec3* points, vec3* tangents_a, vec3* tangents_b, int count @+0xC}`; Hermite evaluation, `t` clamps to the ends, count < 2 returns the default vec3 at `0x252A6D0`.

### Vehicle AI helpers

- Order enum at `vehicle+0x8554`, timestamp at `+0x1C`, **obstacle chain at `+0x856C`** (= block+0x18): 0x1C-byte records `{vec3 position, ptr lane, u32 flags (+0x10: bit0 valid, bit1 not-moving, bit2 order in {0,1,3}), +0x14 next, +0x18 prev}` taken from the pool ring `[0x22B1458]`. `vehicle_ai_drive` writes them, `vehicle_obstacle_records_add` (doc 16) copies the chain to the global obstacle list.
- `vehicle_ai_drive` first calls `vehicle_ai_obstacle_chain_release`, then, at most every 700..1300 ms (`timer_set_random(0x514, 0x2BC)` on the timer at `+0x1C`), marks the vehicle's current lane and neighbours. It is **not** the steering code; see "Doc errors".
- The AI block that the force-flag setters use (EDI) has the vehicle handle at `+0x410`, speed override f32 at `+0x88`, flag bytes `+0x10` (bit 4 = override_speed force flag) and `+0x16` (bit 4 = set by `vehicle_ai_set_speed_override`).
- `veh_pkt` sub-command 0x1C carries a type byte: 0 = `vehicle_ai_pathfind_to` (0xB27830), 1 = `vehicle_ai_pathfind_nodes` (0xB27D90), 2 = `vehicle_ai_pathfind_navmesh` (0xB28700).

## Names

Confidence "confirmed" = full disassembly read; "inferred" = partial read or behaviour deduced from callers/callees. In the CSV the same word is appended in square brackets.

| Address | Name | Conf | Evidence |
|---|---|---|---|
| 0x00a02a50 | `nav_tri_edge_verts` | confirmed | 5 callers; disassembled |
| 0x00a02a90 | `nav_cell_scratch_record` | confirmed | 2 callers in the nav_path_find body; disassembled; scratch table is per search generation |
| 0x00a02c10 | `nav_cell_locate` | confirmed | only caller nav_cell_to_node_ref; disassembled |
| 0x00a02ca0 | `nav_link_closest_point` | inferred | 4 callers (a03060, a044f0, a04a30); first third disassembled |
| 0x00a02ea0 | `nav_cell_pair_mark` | confirmed | 3 callers in the polygon-cut code (a02ec0, a03060, a03870); disassembled |
| 0x00a03c30 | `nav_segment_intersect_xz` | inferred | 12 callers; first half disassembled |
| 0x00a040f0 | `nav_cell_project_point` | confirmed | 38 callers; disassembled |
| 0x00a04180 | `nav_tri_closest_point_xz` | confirmed | 4 callers (nav_cell_project_point, a03060 ...); disassembled |
| 0x00a04470 | `nav_tri_height_at` | confirmed | 12 callers (0x407800 AI core ...); disassembled |
| 0x00a049f0 | `nav_cell_walkable` | inferred | 2 callers inside the nav_path_find chain; disassembled (meaning of the bits inferred) |
| 0x00a04a10 | `nav_link_flags_to_mask` | confirmed | 1 caller (0xA12543 in nav_path_find body); disassembled |
| 0x00a051d0 | `nav_node_ref_to_cell` | confirmed | 48 callers (AI core 0x41xxxx, humans 0x99xxxx); disassembled; inverse of nav_cell_to_node_ref |
| 0x00a05250 | `nav_cell_to_node_ref` | confirmed | 60 callers; used by player_nav_node_update (doc 40) and nav_path_find; disassembled |
| 0x00a052d0 | `nav_chunk_cell_index` | confirmed | only caller nav_cell_locate; disassembled |
| 0x00a05370 | `nav_cell_chunk_record` | confirmed | 2 callers (vehicle AI 0xB454ED / 0xB455CE); disassembled |
| 0x00a05460 | `nav_cell_has_link_flag2` | confirmed | 12 callers incl. nav_cell_walkable; disassembled; link table = chunk section 0x0B path segments (rec+0x14C count) |
| 0x00a0aed0 | `nav_detour_release_hull` | confirmed | 8 callers incl. nav_detours_update, 0x429D90; disassembled |
| 0x00a0df80 | `nav_detour_create` | confirmed | 9 callers: detour_hull_object 0x9196B0, 0x50A450, 0x983BE3 ...; disassembled |
| 0x00a0e0c0 | `nav_detour_destroy` | confirmed | 16 callers: *::deactivate of effect_object_part, detour_hull_object, object_debris, projectile, human; disassembled |
| 0x00a0e1d0 | `nav_detour_release_hull_by_handle` | confirmed | 1 caller (0xA0B1A1 in nav_detours_update); disassembled |
| 0x00a0f3d0 | `nav_tri_edge_side_xz` | inferred | 27 callers; disassembled; constants 0xE5C1A0 = 1e-4, 0xE5C430 = -1e-4 |
| 0x00a0fda0 | `nav_tri_point_inside_xz` | inferred | 2 callers (nav_tri_closest_point_xz); first third disassembled |
| 0x00a0fe30 | `nav_tri_centroid` | confirmed | 11 callers; disassembled |
| 0x00a13100 | `nav_path_request_init` | confirmed | 27 callers (AI core, humans); builds the request later passed in EAX to nav_path_find; disassembled |
| 0x00a132c0 | `nav_frame_update` | confirmed | disassembled; four named callees; unreferenced |
| 0x00a132e0 | `nav_chunk_attach` | confirmed | sole caller chunk_load_from_memory 0x4AFB8F; disassembled; chunk sections 7 and 8 hold the MOPP navmesh shapes |
| 0x00a13520 | `nav_nearest_node_in_list` | confirmed | 4 callers incl. nav_find_node_near_point; disassembled |
| 0x00a13660 | `nav_path_mark_ends` | inferred | 1 caller (0xA14759 in a14560); disassembled |
| 0x00a136e0 | `nav_find_node_near_point` | confirmed | 27 callers incl. ai_object_interaction_start; disassembled |
| 0x00a14ae0 | `nav_path_find` | confirmed | 8 callers (AI 0x41E980, 0x43B7C0/0x43B8E0, humans 0x99ABD0, 0x99AD80, 0x99DC10, vehicle 0xB2A060); 0x184-byte frame; prologue, request copy and epilogue disassembled |
| 0x00a15b00 | `nav_path_free_all` | confirmed | 21 callers; disassembled |
| 0x00a15b20 | `nav_path_node_recycle` | confirmed | 44 callers (AI 0x41E980, 0x43B8E0, humans 0x993BD0 ...); disassembled |
| 0x00a17f60 | `nav_route_record_valid` | inferred | no direct callers (same test inlined in nav_route_record_free / nav_route_list_free); disassembled |
| 0x00a17f90 | `nav_route_record_alloc` | confirmed | 5 callers (0xA180B4, 0xA18201, 0xA182A1, 0xA18716, net_send_veh_pkt_5); disassembled |
| 0x00a18460 | `nav_route_record_free` | confirmed | 17 callers; disassembled |
| 0x00a18500 | `nav_route_list_free` | confirmed | 68 callers (humans 0x99xxxx, vehicles 0xB2xxxx); disassembled |
| 0x00a18570 | `nav_route_refill_from_list` | confirmed | 6 callers (0xB2A6A0, 0xB31EA0, 0xB49F70 ...); disassembled |
| 0x00a19460 | `nav_route_total_length` | confirmed | 4 callers (0xB375DB, 0xB45984 ...); disassembled |
| 0x00a1ed20 | `spline_closest_point` | confirmed | 10 callers (vehicle orders 0xB27830, 0xB27D90, 0x764310 ...); disassembled |
| 0x00a1eda0 | `spline_closest_point_tangent` | confirmed | 5 callers; disassembled |
| 0x00a1f130 | `spline_param_at_distance` | inferred | 6 callers (a1f3d0, hkSplineParametricCurve::vf11, distant_vehicles_spawn_update) |
| 0x00a1f3d0 | `spline_param_at_distance_checked` | inferred | 11 callers incl. hkSplineParametricCurve::vf11 |
| 0x00a1f430 | `spline_curvature` | inferred | 6 callers (vehicle AI 0xB214E0, 0xB23CD0 ...); x87 only |
| 0x00a1f610 | `nav_route_current_entry` | confirmed | 6 callers incl. vehicle_ai_drive; disassembled |
| 0x00a1f730 | `nav_route_clear` | confirmed | 23 callers (vehicle AI 0xB23720, 0xB253F0, 0xB25D60, a18570); disassembled |
| 0x00a1f770 | `nav_route_push` | confirmed | 18 callers (AI orders 0xB4CC29, 0xB4EBA8 ...); disassembled |
| 0x00a88d10 | `road_claim_alloc` | confirmed | 2 callers (road_lane_claim_add, 0xA8B830); disassembled |
| 0x00a88db0 | `road_claim_free` | inferred | reached by tail jump from road_lane_claim_remove; no direct callers; disassembled |
| 0x00a89a20 | `road_query_tree_cb` | inferred | passed as function pointer by road_network_query / road_network_probe; sole direct caller bf78b0 |
| 0x00a89a50 | `road_query_filter` | inferred | 7 callers (all road_query_cb_*); head disassembled |
| 0x00a89ab0 | `road_query_cb_first_hit` | inferred | pushed by road_network_probe; no direct callers; head disassembled |
| 0x00a89b00 | `road_query_cb_nearest_lane` | inferred | called by road_lane_walk_forward 0xA8A8A0 and used as pointer by road_network_query |
| 0x00a89cc0 | `road_query_cb_nearest_lane_b` | inferred | caller bf78b0; same global set as road_query_cb_nearest_lane |
| 0x00a89e80 | `road_query_cb_collect` | inferred | caller a8a8a0; short, no fp |
| 0x00a89fc0 | `road_query_cb_random_lane` | inferred | address stored in a data table (0x68A582); no direct callers |
| 0x00a8a130 | `road_query_cb_ray_hit` | inferred | address stored in a data table (0x68A776); no direct callers |
| 0x00a8a250 | `road_query_cb_segment_distance` | inferred | address stored in a data table (0x68A85C); no direct callers |
| 0x00a8a3f0 | `road_network_query` | confirmed | 36 callers (AI, vehicles, notoriety 0x5C5D5C ...); disassembled |
| 0x00a8a5d0 | `road_network_probe` | confirmed | 2 callers (vehicle_ai_drive, 0xA8B660); disassembled |
| 0x00a8a8a0 | `road_lane_walk_forward` | inferred | 3 callers (itself, a8bcd0 twice); first lines disassembled |
| 0x00a8a980 | `road_lane_walk_back` | inferred | 3 callers (itself, a8bcd0 twice) |
| 0x00a8ab80 | `road_lane_filter_bit19_clear` | confirmed | 5 callers (vehicle_ai_drive, 0xB22190, 0xB4CAE4, 0xB53B90, 0x44BF6C); disassembled |
| 0x00a8aba0 | `road_lane_index` | confirmed | 3 callers (a8ab20, b1e050 twice); disassembled |
| 0x00a8abe0 | `road_lane_by_index` | confirmed | 2 callers (vehicle AI 0xB1FB1F, 0xB1FC2E); disassembled; lane stride 0x3C = chunk section 0x0A entries |
| 0x00a8b6f0 | `road_lane_claim_add` | confirmed | 31 callers (vehicle AI orders 0xB25CB0, 0xB26325, 0xB2B543, 0xB3465C ...); disassembled |
| 0x00a8b7b0 | `road_lane_claim_remove` | confirmed | 31 callers (paired with road_lane_claim_add); disassembled |
| 0x00a8d970 | `road_network_bind_chunk_entries` | confirmed | sole caller chunk_section_0a 0xA8AD6B with [net+0x58]/[net+0x5C]; disassembled |
| 0x00b1ccd0 | `road_path_node_recycle` | confirmed | 46 callers (0x7654DF, map_draw_other_waypoint, vehicle orders); disassembled |
| 0x00b1e240 | `road_lane_closest_point` | inferred | 3 callers (b1e2c0, human_nav_node_refresh twice); 128 bytes, call list read |
| 0x00b1e2c0 | `road_find_closest_lane_point` | inferred | 3 callers (0xB26570, 0xB269D0, vehicle_ai_pathfind_to); call list read |
| 0x00b1e390 | `road_spline_find_nearest` | confirmed | 13 callers (vehicle_ai_drive, human_nav_node_refresh chain, 0x457130 ...); disassembled |
| 0x00b1e570 | `road_lanes_add_neighbours` | inferred | 5 callers (b1f060, b22c50, a8bcd0 ...); first half disassembled |
| 0x00b1e770 | `road_find_lanes_near` | inferred | 5 callers (b1e950, b1f060, b22c50, b1e8b0); call list read |
| 0x00b1e950 | `road_find_nearest_lane` | inferred | 14 callers (human_nav_node_refresh twice, 0xB27830, 0xB27D90, 0x784A30 ...); call list read |
| 0x00b1ea90 | `road_route_total_length` | confirmed | 7 callers (b1eb00, b26570, b269d0 ...); disassembled |
| 0x00b25170 | `vehicle_ai_pre_update` | inferred | sole caller vehicle_pre_update (ab2aa0); call list read |
| 0x00b26130 | `vehicle_free_path_lists` | confirmed | 8 callers (0xAA474A, 0xB31F91, 0xB420FB ...); disassembled |
| 0x00b26530 | `vehicle_nav_query_mask` | confirmed | 12 callers; fed to road_network_probe as the stack mask by vehicle_ai_drive; disassembled; doc 45 open item |
| 0x00b274f0 | `vehicle_ai_set_speed_override` | confirmed | 30 callers (objectives 0x5735A0, 0x580A40 ...); disassembled |
| 0x00b27530 | `vehicle_ai_clear_speed_override` | confirmed | 17 callers; disassembled |
| 0x00b27830 | `vehicle_ai_pathfind_to` | inferred | called by luafn_vehicle_pathfind_to_do 0xA4B310; 14 callers; type byte 0 at 0xB278BA; head and call list read |
| 0x00b27d90 | `vehicle_ai_pathfind_nodes` | inferred | 19 callers (vehicle_kill_objective, vehicle_protect_objective ...); type byte 1 at 0xB27E0E; call list read |
| 0x00b28700 | `vehicle_ai_pathfind_navmesh` | confirmed | called by luafn_vehicle_pathfind_navmesh_do 0xA4B242; callers also boat_kill_objective, 0x541AB6; disassembled |
| 0x00b2a0f0 | `vehicle_ai_update_controls` | inferred | 10 callers (all vehicle orders b32210, b42790, b48370 ...); 448 instructions, call list read |
| 0x00b2a850 | `vehicle_ai_write_migration_data` | inferred | sole caller vehicle_multi_registration::write_migration_data; calls nav_route_* helpers and b1eef0 |
| 0x00b2af80 | `vehicle_ai_read_migration_data` | inferred | sole caller vehicle_multi_registration::on_ownership_taken; calls a1a6f0, a8b6f0, a8b7b0, b253f0 |
| 0x00b2db90 | `vehicle_ai_obstacle_record_alloc` | confirmed | 3 callers (vehicle_ai_drive twice, 0xB2E0CD); disassembled; doc 45 open item |
| 0x00b2dc20 | `vehicle_ai_order_in_0_1_3` | confirmed | 1 caller (vehicle_ai_drive, result stored in flag bit 1 of the obstacle record); disassembled; doc 45 open item |
| 0x00b2dc40 | `vehicle_ai_obstacle_chain_release` | confirmed | 1 caller (vehicle_ai_drive); disassembled |
| 0x00b2ee60 | `vehicle_ai_goal_install` | confirmed | 9 callers: all vehicle_ai_pathfind_* / order helpers; disassembled |
| 0x00c03910 | `spline_eval_position` | confirmed | 122 callers (AI, vehicles, camera); head and coefficient math disassembled |
| 0x00c03af0 | `spline_eval_tangent` | inferred | 73 callers; always used next to spline_eval_position (b1e390, a1f430); not fully read |
| 0x00c03dc0 | `spline_eval_second_derivative` | inferred | 7 callers; only the curvature routine a1f430 and 0x7FF670 use it |
| 0x00c04110 | `spline_closest_param` | confirmed | 56 callers; a1ed20 stores XMM0 right after the call; head disassembled |
| 0x00c044d0 | `spline_arc_length` | inferred | 33 callers; a18040 calls it as (spline, 5, 0.0, x); uses spline_eval_position + sqrtf_wrapper |

## Port candidates

All fifteen are integer-only or one trivial x87 line, leaves or short call sequences, and most run every frame (human / vehicle AI). "Kept" means unchanged on every path. None of the integer ones set a result through flags. Shadow snapshot = memory writes; the pure readers have none.

### 1. `nav_node_ref_to_cell` 0xA051D0 (48 callers, every AI tick)

- **In** EAX = ref. **Out** EAX = cell pointer or 0. No stack args, plain `ret`. **Kept**: EBX (pushed only after the early checks, popped on every later path), ESI, EDI, EBP. **Clobbered**: EAX, ECX, EDX. Flags irrelevant.
- **Callees** none. **Loop** none.
- Steps, in order:
  1. `cmp eax,-1; je` -> return 0.
  2. `kind = (ref >> 0x13) & 1` (EDX), `chunk = ref >> 0x14` (ECX, logical shift), `idx = ref & 0x7FFFF`.
  3. `chunk == 0xFFFF` -> 0 (cannot happen with a 12-bit shift result, keep anyway); `chunk >= 0x866` **unsigned** (`jae`) -> 0.
  4. `rec = [0xF7AAE8 + chunk*4]`; `rec == 0` -> 0.
  5. `test byte [rec+0x1C],3; jne` -> 0 (any of bit 0 or bit 1 set rejects).
  6. kind != 0: `cmp dword [rec+0x7C],0`, else `cmp dword [rec+0x6C],0`; `setg bl` = **signed greater than 0**; `je` (count <= 0) -> 0.
  7. kind != 0: `obj = [rec+0x248]`, else `[rec+0x240]`; result = `[obj+0x14] + idx*0x30` (computed as `idx*3 << 4`, 32-bit wrap). No null check on `obj`, no `idx < count` check.
- **f32** none. **Writes** none (no shadow snapshot needed).

### 2. `nav_cell_to_node_ref` 0xA05250 together with `nav_cell_locate` 0xA02C10 and `nav_chunk_cell_index` 0xA052D0 (60 callers)

- **In** EAX = cell*. **Out** EAX = ref or `0xFFFFFFFF`. Plain `ret`. **Kept**: EDI (pushed around the call), EBX/EBP/ESI. **Clobbered**: EAX, ECX, EDX. Locals preset to chunk=0xFFF, kind=1, idx=0x7FFFF (only used on failure paths the code never reads).
- Steps:
  1. `cell == 0` -> -1. `cmp word [cell+0x2C],0xFFFF; je` -> -1.
  2. `nav_cell_locate(EAX=cell, EDI=&idx, stack(&chunk,&kind))`: `c = u16[cell+0x2C]`; `c == 0xFFFF` or `c >= 0x866` unsigned -> fail; `rec = [0xF7AAE8+c*4]` null -> fail; `r = nav_chunk_cell_index(EAX=cell, rec, &tmp)`; `r == 0` -> fail, `r == 1` -> kind 0, `r == 2` -> kind 1, anything else -> fail; on success chunk = `u16[rec+8]` (zero-extended), idx = tmp, AL = 1. Failing path returns AL = 0.
  3. `nav_chunk_cell_index`: `u16[cell+0x2C] != u16[rec+8]` -> 0. Then for obj = `[rec+0x240]` (if non-null) and then `[rec+0x248]`: `d = cell - [obj+0x14]` (i32), `q = hi32(d * 0x2AAAAAAB) >> 3` (arithmetic), `q += (q >>> 31)` (sign fix); accept when `q >= 0` (the `js` is on the corrected value) **and** `q < zero_extend(u16[obj+0x10])` signed (`jge` rejects). First array hit returns 1, second returns 2, `*out = q`. Clobbers EAX/ECX/EDX only (EBX, EBP, ESI, EDI pushed).
  4. Final: `(chunk << 0x14) | ((kind & 1) << 0x13) | (idx & 0x7FFFF)`.
- **Writes** none outside the stack temps. Fails to -1 whenever the cell is not inside either array.

### 3. `nav_cell_chunk_record` 0xA05370 (leaf)

- **In** EAX = cell*. **Out** EAX = chunk record or 0. Only EAX modified.
- `c = movzx word [eax+0x2C]; c == 0xFFFF -> 0; c >= 0x866 (jae, unsigned) -> 0; return [0xF7AAE8 + c*4]` (a null table slot returns 0).

### 4. `nav_cell_walkable` 0xA049F0 (calls recipe 5)

- **In** EAX = cell*. **Out** AL (EAX upper bits are whatever was there: the function sets only AL; callers use `test al,al`). **Clobbered** CL, and EAX/ECX/EDX through the callee.
- `cl = [eax+0x1D]`. `test cl,0x10; je L1` -> **bit 0x10 set returns 0**. L1: `test cl,2; je L2` -> **bit 2 clear skips the call and returns 1**; otherwise `AL = nav_cell_has_link_flag2(EAX = cell)`; `test al,al; je` -> returns 0 when it is 0. L2: AL = 1.
- So: `walkable = !(f & 0x10) && (!(f & 2) || has_link_flag2(cell))`. **Writes** none.

### 5. `nav_cell_has_link_flag2` 0xA05460 (12 callers)

- **In** EAX = cell*. **Out** AL. **Kept** ESI, EDI (pushed). **Clobbered** EAX, ECX, EDX.
- `n = zero_extend(u8[cell+0x1C])`; `n <= 0` (after `test edi,edi; jle`, i.e. only 0) -> AL = 0. `p = [cell+0x18]`. Loop `i = 0..n-1`, `p += 4` each:
  - `ref = [p]`; `ref == 0` -> next. `lo = ref & 0xFFFF; hi = ref >> 16; lo -= 1`.
  - `hi == 0xFFFF` -> next; `hi >= 0x866` unsigned -> next; `rec = [0xF7AAE8 + hi*4]` null -> next.
  - `link = (lo << 5) + [rec+0x150]` (i32 add; the `je` right after the `add` tests the **sum == 0** -> next).
  - `test byte [link+0x18],2; jne` -> return AL = 1 immediately (early exit).
  - After the loop AL = 0. Loop condition `edx < edi` signed (`jl`).
- **Writes** none. **f32** none.

### 6. `nav_cell_scratch_record` 0xA02A90 (leaf)

- **In** EAX = cell*. **Out** EAX. Only EAX/ECX.
- `ecx = [eax+0x20]; cmp ecx,[0x2E160BC]; jne -> 0` (equal required). `return 0x2E160C4 + zero_extend(u16[eax+0x1E]) * 24` (`lea eax,[eax+eax*2]; lea eax,[eax*8+0x2E160C4]`). **Writes** none.

### 7. `nav_cell_pair_mark` 0xA02EA0 (leaf)

- **In** ECX = cellA, EDX = cellB. **Out** none. **Clobbered** AL only (EAX low byte; callers do not rely on it).
- `al = [edx+0x1D]; test al,0x20; jne ret` (B already marked: nothing). `test byte [ecx+0x1D],0x20; jne ret` (A marked: nothing). Else `[edx+0x1D] = al | 0x20; [ecx+0x1D] |= 0x20`.
- **Writes** byte `edx+0x1D`, byte `ecx+0x1D` (snapshot both).

### 8. `nav_link_flags_to_mask` 0xA04A10 (leaf)

- **In** EDX = pointer to two flag bytes. **Out** EAX. **Clobbered** EAX, CL.
- `m = 0; if (b0 & 8) m = 2; if (b0 & 0x10) m |= 1; if (b1 & 1) m |= 4;` where `b0 = [edx]`, `b1 = [edx+1]`. Each test is `test; je` skip. **Writes** none.

### 9. `nav_route_record_free` 0xA18460 + `nav_route_list_free` 0xA18500 (68 + 17 callers)

- **valid(rec)** (inlined twice): `d = rec - 0x2E400D0` (`js` -> invalid); `d >= 0x6800` unsigned (`jae`) -> invalid; `d % 0x34 != 0` (computed as `d - 0x34 * (hi32(d * 0x4EC4EC4F) >> 4)`) -> invalid; `test byte [rec+0x30],0x20; jne` -> valid else invalid.
- **nav_route_record_free**: **In** ECX = rec, EDI = &slot. **Kept** ESI (pushed), EDI. **Clobbered** EAX, EDX. Not valid -> return, nothing written. Valid:
  1. If `[edi] == rec`: `next = [rec+0x1C]`; if `next == rec` (single node): `[rec+0x1C] = 0; [rec+0x20] = 0; [edi] = 0` and go to step 3 (skipping step 2); else `[edi] = next`.
  2. `prev = [rec+0x20]; if (prev != 0) [prev+0x1C] = [rec+0x1C]; nxt = [rec+0x1C]; if (nxt != 0) [nxt+0x20] = [rec+0x20]; [rec+0x1C] = 0; [rec+0x20] = 0`.
  3. `h = [0x2203AB8]`: if `h == 0`: `[rec+0x30] &= 0xDF; [rec+0x1C] = rec; [rec+0x20] = rec; [0x2203AB8] = rec`. Else `[rec+0x20] = [h+0x20]; [rec+0x1C] = h; [[h+0x20] + 0x1C] = rec; [h+0x20] = rec; [rec+0x30] &= 0xDF` (inserted before the head, head pointer unchanged).
- **nav_route_list_free**: **In** EAX = &slot. **Kept** EDI, ESI. `slot == 0` -> return. `rec = [slot]`, `rec == 0` -> return. If `!valid(rec)`: `[slot] = 0`, return. Loop while `rec != 0`: `next = [rec+0x1C]`; `esi = (next != [slot]) ? next : 0` (the `neg/sbb/and` idiom: **sbb gives -1 when the subtraction was non-zero**, and with `esi &= next` yields `next` or 0), evaluated **before** the free; `nav_route_record_free(ECX = rec, EDI = slot)`; `rec = esi`.
- **Writes**: `[slot]`, the unlinked record's `+0x1C/+0x20/+0x30`, its old neighbours' `+0x1C/+0x20`, free head `0x2203AB8`, the free-ring head's `+0x20` and the old tail's `+0x1C`.

### 10. `nav_path_node_recycle` 0xA15B20 and `nav_path_free_all` 0xA15B00 (44 + 21 callers); twin `road_path_node_recycle` 0xB1CCD0 uses the ring `0x22A9C7C`

- **nav_path_node_recycle**: **In** EAX = node, EDX = &owner head slot. **Out** none; **Kept** EAX (node), ESI, EDI. **Clobbered** ECX, EDX.
  1. `[edx] == eax` (node is head): `c = [eax]; c == eax` (single) -> `[eax] = 0; [eax+4] = 0; [edx] = 0` and jump to step 3; else `[edx] = c`.
  2. `p = [eax+4]; if (p != 0) [p] = [eax]; n = [eax]; if (n != 0) [n+4] = [eax+4]; [eax] = 0; [eax+4] = 0`.
  3. `h = [0x21FC19C]`: `h == 0` -> `[eax] = eax; [eax+4] = eax; [0x21FC19C] = eax`. Else `[eax+4] = [h+4]; [eax] = h; [[h+4]] = eax; [h+4] = eax` (before head, head unchanged).
- **nav_path_free_all**: **In** EDI = &slot. `eax = [edi]` loop `while eax != 0`: `c = [eax]; esi = (c != [edi]) ? c : 0` (same `sub/neg/sbb/and` idiom, using the head value **before** the call); `edx = edi; nav_path_node_recycle(eax, edx); eax = esi`. **Kept** ESI (pushed), EDI; **Clobbered** EAX, ECX, EDX.
- **Writes** as listed, plus the global ring head `0x21FC19C`.

### 11. `nav_route_current_entry` 0xA1F610 (leaf, used by `vehicle_ai_drive`)

- **In** ECX = route. **Out** EAX. **Clobbered** EAX, DL.
- `dl = [ecx+0x3C]; dl == 0 -> 0`. `al = [ecx+0x24]; al >= dl` **unsigned** (`jae`) -> 0. Result `ecx + zero_extend(al) * 12` (`lea eax,[eax+eax*2]; lea eax,[ecx+eax*4]`). **Writes** none.

### 12. `nav_route_clear` 0xA1F730 (23 callers)

- **In** ESI = route (0 -> return, nothing changed). **Kept** EDI, EBP (pushed), ESI. **Clobbered** EAX, ECX, EDX.
- `edi = 0`; if `u8[esi+0x3C] != 0` (`cmp byte; jbe` skips on 0): `ebp = esi + 4`; loop: if `byte [ebp+4] & 4`: `nav_route_list_free(EAX = ebp)`; then `edi++`, `ebp += 12`, repeat while `edi < zero_extend(u8[esi+0x3C])` signed (`jl`; the count is re-read each iteration). After the loop (and also when the count was 0): `[esi+0x3C] = 0; [esi+0x24] = 0; [esi+0x30] = 0`.
- **Writes** the three route bytes plus what `nav_route_list_free` writes.

### 13. `vehicle_ai_order_in_0_1_3` 0xB2DC20 (leaf, every `vehicle_ai_drive`)

- **In** EAX = vehicle. **Out** AL (upper EAX is the order value / garbage; the caller only does `test al,al`). Only EAX modified.
- `o = [eax+0x8554]` signed: `o < 0` (`jl`) -> 0; `o <= 1` (`jle`) -> 1; `o == 3` -> 1; else 0. So {0, 1, 3} -> 1.
- **Writes** none.

### 14. `vehicle_nav_query_mask` 0xB26530 (leaf, 12 callers)

- **In** EAX = vehicle or 0. **Out** EAX. Only EAX modified.
- `v == 0`: `t = -1` (`or eax,-1`). Else `t = [[v+0x84E4]+0x9C]` (no null check on `[v+0x84E4]`). Then `t - 2 == 0` -> `0x200000`; `t - 4 == 0` (i.e. `t == 4`) -> `0x40000`; everything else (including -1) -> `0x400`.
- **Writes** none.

### 15. `vehicle_ai_set_speed_override` 0xB274F0 (30 callers) with its twin `vehicle_ai_clear_speed_override` 0xB27530

- **In** EAX = AI block (copied to EDI), CL = bool (bit 0 used), one stack f32 (caller cleans 4, plain `ret`). **Kept** ECX (pushed/popped at the very start/end) and EDI. **Clobbered** EAX (`fnstsw ax`), EDX and flags through the callee.
- x87: `fld dword [arg]; fcomp qword [0xE5C0D8]` (= 1000.0 double); `fnstsw ax; test ah,5; jnp keep`. The jump is taken only when exactly one of C0 / C2 is set, which for `fcomp` of `speed` against 1000.0 means **speed < 1000.0** (C0 only). Equal or greater leaves `ah&5 == 0` (PF set -> not taken) and **NaN** sets C0 and C2 (even parity -> not taken): both overwrite the stack argument slot with the f32 1000.0 (`0xE98030`).
- Then `speed = arg` (f32 load / f32 store, no rounding step: `[ai+0x88] = speed` exactly), `cl = (CL << 4) ^ [ai+0x16]; cl &= 0x10; [ai+0x16] ^= cl` (sets bit 4 of byte +0x16 to the bool). Finally `push 1; call 0xB2B8A0` (`vehicle_ai_set_force_flag_override_speed`, EDI = block, callee pops with `ret 4`) - keep this call as an original call; it builds a `veh_pkt` when the vehicle is not local.
- Clear variant: `[ai+0x88] = 0.0f; [ai+0x16] &= 0xEF; push 0; call 0xB2B8A0`.
- **Writes** `[ai+0x88]` (f32), byte `[ai+0x16]`, then whatever 0xB2B8A0 writes (`[ai+0x10]` bit 4, network packet).

### Extra leaves with the same style (not counted above)

- `road_lane_by_index` 0xA8ABE0: `eax < 0` (`jl`) -> 0; `eax >= [[0x2225B64]]` signed (`jge`) -> 0; result `[net+4] + eax*0x3C` (`shl 4; sub; *4`). Needs `[0x2225B64] != 0` (no null check).
- `road_lane_index` 0xA8ABA0: `lane == 0` -> -1; `[lane+0x10]` bit 19 set -> -1; network null -> -1; else `(lane - [net+4]) / 0x3C` as `hi32(d * 0x88888889)` + d, `sar 5`, + sign (signed divide semantics).
- `road_lane_filter_bit19_clear` 0xA8AB80: `eax == 0` -> 0; `(([eax+0x10] >> 0x13) & 1) == 0` -> keep EAX, else 0 (ECX saved with push/pop).
- `nav_route_total_length` 0xA19460: `sum = 0.0f; p = ecx; loop { sum = f32(sum + [p+0x14]); p = [p+0x1C]; stop when p == ecx or p == 0 }` (the sum is stored back to a stack f32 each step, so every add rounds to f32); `ecx == 0` -> 0.0.


## Doc errors and surprises

1. **`traffic_spline_validate` (0xB318F0) is not a spline validator.** Its 48 bytes do `[eax+0x8630] = f32 [0xE5C664]; [eax+0x8634] = f32 [0xE8B048]; push eax; call 0xB2F830` with EAX = vehicle: it initialises two vehicle AI floats and calls a helper. The strings "Spline: %d has no next spline" (0xE49504 / 0xE49524) belong to a different debug code path that was never matched to it. The function should be renamed after `0xB2F830` is read (suggest `vehicle_ai_init_defaults`).
2. **`vehicle_ai_drive` (0xB2DD90, docs 16 / 45) does not compute driving controls.** It releases the obstacle chain, picks the vehicle's current road lane (`road_network_probe` + `road_spline_find_nearest`) and appends obstacle records with the order / motion flags. The control code is in `vehicle_ai_update_controls` (0xB2A0F0) and its helpers (0xB23FD0, 0xB258D0, 0xB24350). Doc 45's description of the record at `+0x8554` is right; `0xB2DB90` allocates from the ring into the **vehicle chain `+0x856C`**.
3. **Doc 40, `player_nav_node_update`:** "`[0x2203AC0]` = 0xA05250(`ESI` = node)". `nav_cell_to_node_ref` takes its input in **EAX** (EAX still holds the `nav_closest_node_find` result at the call; ESI is only a copy). A port that passes the node in ESI only would read the wrong register.
4. **Doc 05, chunk sections 7 and 8** are described as "Havok MOPP collision". They are the two **navmesh shapes** (`havok_navmesh_shape` RTTI exists; the debug strings "ped navmesh" / "veh navmesh" at 0xE44571 / 0xE44585 are unreferenced, so which one is kind 0 is not proven, the string order suggests ped = section 7 = kind 0): `nav_chunk_attach` copies rec+0x68 / rec+0x78 into the navmesh array slots rec+0x240 / rec+0x248 and creates Havok handles from rec+0x70 / rec+0x80. Section 0x0A is the road lane network (0x3C-byte lanes), confirming "probably the road network".
5. `nav_frame_update` (0xA132C0) is a dead copy of the four `sim_frame:1b` calls (no references anywhere in the image); the real calls are inline in the frame loop.
6. `nav_node_ref_to_cell` never checks the index against the array count; a stale ref from a reloaded chunk can return a pointer past the array. A faithful port must keep this (not clamp), and must keep the `count > 0` signed test.
7. `0xB274F0` stores `1000.0` also for **NaN** speeds, because the NaN case falls through the `test ah,5 / jnp` branch (see recipe 15).
8. `nav_route_list_free` clears the caller's slot only when the head fails validation; valid lists leave the slot untouched until `nav_route_record_free` zeroes it while unlinking the last node.

## Open

- `0xA121B0` (2.3 KB) and `0xA11750`, `0xA15100`, `0xA16650` (3.5 KB, the timesliced job stage functions) are the heart of the A* / polygon-cut search and were only mapped by their call lists.
- `0xA14230`, `0xA14560`, `0xA147E0`, `0xA1A6F0`, `0xA189A0` are the path-request wrappers used by humans (via `0x99C020`) and vehicles; the arguments of the callers were not yet decoded.
- The vehicle order handlers `0xB32210`, `0xB42790`, `0xB48370` ... (callers of `vehicle_ai_update_controls`) and the 40 other `0xB2xxxx` helpers (`0xB23xxx`-`0xB26xxx`) are untouched.
- `0xB2F830` (and the real meaning of the `+0x8630` / `+0x8634` floats), `0xB1E050`, `0xB1EB00`, `0xB1ECF0`, `0xB1EEF0`, `0xB1F060`, `0xB1F630` (3.2 KB) in the lane routing code.
- Ped navigation proper (`0xA1D990`, `0xA1E5B0`, the `human_state*` path-follow code) and the road-query variants `0xA8A4F0`, `0xA8A660`, `0xA8A6F0`, `0xA8A7B0`.
