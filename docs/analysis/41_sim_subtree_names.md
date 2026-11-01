# 41 — Names in the gameplay simulation subtree

This round names functions below the gameplay part of `sim_frame` (0x68CA70). The roots were `humans_update_all` / `human_update` (0x9D6D60) / `human_update_core` (0x9A7DF0), the vehicle chain (`vehicles_pre_update` 0xAE3A80, `vehicle_pre_update` 0xAB2AA0, `vehicles_update` 0xAE3C60, `vehicle_update` 0xAB3550, `vehicle_update_light` 0xAB4460), `ai_global_do_frame` (0x437160), `effect_objects_do_frame` (0x5053B0) and `world_spawn_do_frame` (0x93D150). Rendering, UI (the HUD calls from `human_update`, 0x78xxxx-0x7Fxxxx) and streaming were left out.

How the list was made:
- a flow-following scan (direct `call`s, tail `jmp`s, jump tables) to 5 levels from the roots: 4,056 functions;
- the ones missing from `docs/functions.csv` were ranked by size, the number of `E8` call sites in the whole exe, and self samples in `bin/mission01_tss01.log`;
- the most-called and the direct children of the roots were then read by hand.

Every name below is missing from `functions.csv`. `timer_elapsed_ms` (0xBDD2A0) was found here too, but doc 42 added it to `functions.csv` under the same name during this round, so it is not repeated. None of them replaces a `FUN_` or `::vfN` placeholder. Each address is preceded by `int3` / `nop` / `ret` padding or is a call target. None of the names is used by another address.

- 76 names: 41 `documented` (role confirmed: strings, a direct reading of a small body, or a doc that already describes the role) and 35 `named` (role inferred).
- The CSV is `docs/candidates_sim.csv`. Its notes give the registers first, then the evidence.
- Register notation: `EAX = x` means the function reads x from EAX. "cdecl (...)" are stack arguments the caller cleans. "stdcall ... ret N" means the callee cleans them. The return goes in AL / EAX / ST0 as stated.

## Findings that affect existing docs

1. **Vehicle classes.** Doc 16 says that info +0x9C is "0 / 1 / 2 / 5 land vehicles, 3 boat, 4 aircraft". The code says otherwise:
   - **2 = plane**: `vehicle_aircraft_props_update` 0xB03FD0 starts the `_prop` effect for class 2;
   - **3 = helicopter**: the same function starts `_m_rotor` / `_t_rotor` for class 3, and `vehicle_is_heli_fighter` requires class 3;
   - **4 = boat**: `vehicle_boat_effects_update` 0xAD1F10 runs only for class 4 and starts `_spray` / `_motor`.

   So in `vehicle_pre_update`, the "type 3" path (0xB047D0) is the helicopter path. The `cmd_vehicle_ejection_thresholds` (0xA97A90) call for type 4 is a boat speed bonus, which also suggests that name is wrong (doc 16 already doubts it).
2. **`vehicle_seat_occupant` (0x960D60) takes a handle, not `&seat`.** It reads `EAX = handle` and returns the live type-1 object, so it is a general "human from handle". It has 479 callers. Callers load the seat's dword first (`mov eax, [seat]`).
3. **`vehicle_pre_update` step 6 never returns early.** `0xA9DBE0` (`vehicle_damage_over_time_update`) always returns AL = 0 (`xor al, al` on every path), so the `cmp al, 1; je` exit after it is dead. It is also passed the fixed step `[0xE8437C]` (1/60), not the frame dt.
4. **Existing names that look wrong** (not changed here):
   - `human_entering_vehicle` (0x9B9030) and `human_exiting_vehicle` (0x9B9070) both test rider state **3** and differ only by seat +0xD78 == 0 / != 0. They look like "riding as driver" / "riding as passenger".
   - The real state-1 / state-2 tests are 0x9B9320 / 0x9B9350 (named here `human_vehicle_interaction_entering` / `_exiting`).
   - `human_is_local_player` (0x926600) only tests "is a player human" (+0x4C == 1 and +0x3CC == 1). The pointer comparison with `[0x21703D4]` is 0x926650 (`human_equals_local_player`).
5. **Unchecked stack arrays.** `vehicle_part_door_state_is_3` / `vehicle_part_door_is_open` / `vehicle_part_doors_set_state` copy every child part id into an 8-dword stack array (`sub esp, 0x20`) with `rep movsd`. There is no bound check, so a door subtree of more than 8 parts would overwrite the frame. This is the same kind of bug as the vehicle-list copies in doc 16.
6. Doc 15 says 0x412080 measures "within 1 m on the ground plane". 0x412080 is a plain 3D `vec3_length` (EAX = &v). Any ground-plane flattening is done by the caller.
7. `ragdoll_find_for_object` (0x710DB0) never uses its cache for record 0, because the cache test is `jle` on the index. A hit on record 0 still works through the full scan.

## Names

### Vehicles

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0xAA68E0 | `vehicle_get_velocity` | ECX = vehicle, cdecl (out vec3*), no return | copies physics body (+0x554) +0x1A0..+0x1A8 for a full vehicle (+0x84D8 == 1), else the zero vector 0x252A6D0; 77 callers incl. vehicle_speed, vehicle_pre_update, vehicle_update | confirmed |
| 0xAAB6B0 | `vehicle_any_wheel_in_contact` | EDI = vehicle -> AL | false for class 3 / 4 or non-full vehicles; walks the part list +0x6CE0 (count, ptrs), skips parts with +0x11C & 0x600000, asks the physics object +0x53C vtbl +0x40(i) and is true on the first result != -1; feeds +0x8F50 bit 0x40000000 in vehicle_pre_update | inferred |
| 0xAA0D10 | `vehicle_position_history_push` | ESI = vehicle, no stack args | ring of up to 15 16-byte samples at +0x90DC (count +0x90D8): drops the oldest at 15 (memmove), ages every sample's +0xC by 1/60 ([0xE8437C]), appends vtbl +0(&sample, -1) with age 0; called just before the previous-transform copy in vehicle_pre_update | inferred |
| 0xA9DBE0 | `vehicle_damage_over_time_update` | ESI = vehicle, cdecl (f32 dt) -> AL (always 0) | skipped during a cutscene; accumulates +0x8C44 * dt + fraction +0x8C48, floorf_wrapper + _ftol2_sse to an int, keeps the fraction, then 0xA9D530 with the record +0x8C20 and 0xA98070 (kind 7) when the record count +0x8C28 > 0; the only caller vehicle_pre_update (gated by +0x8F48 bit 0x100) passes [0xE8437C] (1/60), not the frame dt | inferred |
| 0xA9D530 | `vehicle_damage_record_apply` | cdecl (vehicle, attacker human, a, b, damage record*, int amount, bool force) -> AL | owner checks (human_owned_locally), picks the blamed occupant (0xAA7380 / vehicle_seat_occupant by attacker type +0x4C), 0xAB6A50 / 0xA9D290 destroy path, effect_start, script callbacks; callers vehicle_pre_update, vehicle_collision_damage_apply, 0xA9DBE0, 0xAB54F0, 0xAB5620 | inferred |
| 0xA98070 | `vehicle_damage_events_raise` | EDX = vehicle, cdecl (human, amount, a, b, kind, c) | raises game_event_15 twice, net handle packing (0x804F70 / 0x814740), script callbacks (script_object_name, script_thread_push_string, lua_pushstring, 0xA70110); called right after 0xA9D530 by the same callers | inferred |
| 0xAB4720 | `vehicle_set_nitro` | EAX = vehicle, cdecl (bool on, bool force) | not for class 3 / 4; on: +0x53C->+0x10C = 1, timer +0x114 = now + 5000, +0x118 = 5000, +0x110 = 1.5, +0x8FFC = -1, 0xAB4A20 (IV__Nitro_Start) for remote owners, four effects into +0x9000 when info +0x7C, camera kick for the local player (0x4A0B60, 0x49E800, 0x4CC080, tail 0x9E35D0); off: clears them, 0x50DDB0 on the effects, audio_voice_stop(+0x8CA4) | confirmed |
| 0xAB4A20 | `vehicle_nitro_start_effects` | EAX = vehicle | string 'IV__Nitro_Start' (audio cue), effect_start; only from vehicle_set_nitro | confirmed |
| 0xAFE2F0 | `vehicle_nitro_expired` | EAX = handling object (vehicle +0x53C) -> AL | +0x10C == 1 and the timer +0x114 has elapsed (same wrap logic as timer_elapsed); vehicle_pre_update then calls vehicle_set_nitro(0, 0) | confirmed |
| 0xAFE2C0 | `vehicle_handling_bonus_speed` | EAX = handling object (vehicle +0x53C) -> ST0 (f32) | +0x120 when +0x158 bit 1 is set, else 0.0; the extra allowed speed for classes 0 / 1 in the speed clamp of vehicle_pre_update | inferred |
| 0xAA77C0 | `vehicle_seat_door_part` | EDX = vehicle, ECX = seat index -> EAX part index or -1 | info+4 +0x78 == 1 and full vehicle; seat table info+4 ->+4 ([0] count, [1+i] part); -1 when the part flag +0x67C+part*0x120 has 0x400000 (detached) | inferred |
| 0xAC0290 | `vehicle_part_is_closed` | EAX = part record (vehicle +0x560 + i*0x120) -> AL | \|part +0x30 - 0.0\| < 0.03 (door angle near zero); seat loop of vehicle_pre_update | inferred |
| 0xAA7E30 | `vehicle_part_door_state_is_3` | EAX = part index, cdecl (vehicle) -> AL | walks the part and its child lists (+0x10 count / +0x14 ids, collected into an 8-slot stack array with no bound check) to the first type-8 part (door) without flags 0xE00000 and tests its state +0x20 == 3 | inferred |
| 0xAA7EE0 | `vehicle_part_door_is_open` | ECX = part index, cdecl (vehicle, bool any) -> AL | same walk; first type-8 part without 0x600000: true when flag 0x800000 is set or state +0x20 == 1 | inferred |
| 0xAA7FB0 | `vehicle_part_doors_set_state` | EAX = part index, cdecl (vehicle, bool open) | same walk; sets state +0x20 of every type-8 part without 0xE00000 to 0 (open = 1) or 2 (open = 0); vehicle_pre_update also applies it to the paired door (0xAA77C0 of the neighbour seat) | inferred |
| 0xAA7380 | `vehicle_seat_occupant_by_index` | ECX = vehicle, EDX = seat -> EAX human or 0 | seat count = +0x6CE8 for a full vehicle (0 -> null), else 8; tail jmp vehicle_seat_occupant(EAX = [vehicle + 0x8ABC + seat*0x24]); 94 callers | confirmed |
| 0xAB5220 | `vehicle_is_heli_fighter` | EAX = vehicle -> AL | class (+0x84E4 -> +0x9C) == 3 (helicopter) and the info name +0x80 equals 'heli_fighter_01' (16-byte repe cmpsb); callers in AI (0x45DDD0, 0xB34000) | confirmed |
| 0xAD1F10 | `vehicle_boat_effects_update` | EAX = vehicle | only class 4 (boat): 0xACE400 (spray / motor effects), vehicle_speed, 0xAA8290, 0xACE660, 0xAA63A0; from vehicle_update and vehicle_update_light | inferred |
| 0xACE400 | `vehicle_boat_spray_effects_start` | EDI = vehicle | builds '<name>_spray' / '<name>_motor' with _strncpy / _strncat, effect_find_by_name, effect_start; only from 0xAD1F10 (class 4) | confirmed |
| 0xB03FD0 | `vehicle_aircraft_props_update` | EAX = vehicle | class 2 (state at +0x2D4) or 3 (state at +0x508) only; starts 0xACFA50 (_prop) for class 2 and 0xACF350 (_m_rotor / _t_rotor) for class 3, then spins them (0xACF6E0, 0xACFC10, 0xB03040); last call of vehicle_update | inferred |
| 0xACF350 | `vehicle_heli_rotor_effects_start` | EDI = vehicle | strings '_m_rotor', '_t_rotor', effect_find_by_name, effect_start; from 0xB03FD0 for class 3 | confirmed |
| 0xACFA50 | `vehicle_plane_prop_effects_start` | EDI = vehicle | string '_prop', effect_find_by_name, effect_start; needs +0x2E8 != -1 and info+4 +0x78 == 1; from 0xB03FD0 for class 2 | confirmed |
| 0xB07A30 | `vehicle_interaction_rider_find` | ESI = human handle -> EAX rider slot or 0 | resolves a live type-1 object, takes its interaction record +0x10E0 and finds the rider slot (+0x34C + i*0xA8, 8 slots) whose handle == ESI; used by human_entering_or_exiting_vehicle (states 1 / 2 at +4); 296 callers | confirmed |
| 0xB07E60 | `vehicle_interaction_play_action` | cdecl (human, ...) | debug string 'Failed to play %s action while interacting with %s vehicle!', rig +0x5E0, 0x6EF6A0, 0x96FCA0; callers 0xB12D30 / 0xB12F70 / 0xB13D90 (vi enter / exit / ride) | confirmed |

### Humans

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0x9B9320 | `human_vehicle_interaction_entering` | cdecl (human) -> AL | vehicle_interaction_rider_find(ESI = +0x44) slot state +4 == 1 (a null slot gives 0); 91 callers. The existing human_entering_vehicle (0x9B9030) instead tests state 3 with +0xD78 == 0 | confirmed |
| 0x9B9350 | `human_vehicle_interaction_exiting` | cdecl (human) -> AL | same, state +4 == 2; 46 callers (human_entering_or_exiting_vehicle tests 1 or 2). The existing human_exiting_vehicle (0x9B9070) tests state 3 with +0xD78 != 0 | confirmed |
| 0x9855F0 | `human_is_dead_or_invalid` | EDX = human -> AL | 0 only when the handle +0x44 resolves to this object and hit points +0x1124 > 0; 200 callers | confirmed |
| 0x9D06A0 | `human_from_handle_non_scripted` | ECX = handle -> EAX human or 0 | live type-1 object whose +0x3CC kind **is** 2 (the name is inverted; corrected later); 88 callers (vehicles_update pair events, human code) | inferred |
| 0x926650 | `human_equals_local_player` | EAX = human -> AL | EAX != 0 and EAX == [0x21703D4]; 86 callers. The existing human_is_local_player (0x926600, ECX) only tests +0x4C == 1 and +0x3CC == 1 (any player human) | confirmed |
| 0x9B5C00 | `human_set_look_at_point` | cdecl (human, f32 weight), EDX = &point, ESI = duration ms, EAX = flag | +0x730..0x738 = point, bit 0 of +0x760 = EAX bit 0, +0x74C = weight when > 0, clears +0x75C bit 0, timer +0x748 = now + ESI or -1; the look-target step of human_update | confirmed |
| 0x98E2A0 | `human_knockdown_default_force` | no args -> ST0 (f32) | [0x279749C] in single player (0x2528B48 == 1, 0x2528B49 == 0), else [0x279748C]; the same constants as the explosion knockdown (doc 33); human_update_core step 6 | confirmed |
| 0x718260 | `human_ragdoll_velocity` | ECX = human, ESI = out vec3 | zero, then ragdoll_find_for_object -> record +0xF74 -> +4 -> body +0x1A0..+0x1A8; human_update_core ragdoll speed test | confirmed |
| 0x96F3A0 | `human_rig_current_action` | cdecl (human) -> EAX action id or -1 | rig +0x5E0: +0x164 while the track +0x168 is live (0x6EEE20) and not finished (0xCED710); 45 callers | inferred |
| 0x96EF10 | `human_anim_set_select` | EDI = human, cdecl (stance) -> EAX anim set id | 0x19B for kind 2 with +0x1178, 0x9B when dead (human_is_dead), otherwise jump tables on +0x591 / +0x592 / stance (0x90, 0x91, ...); +0x1130 override; called by 0x96CB70 and the rider updates | inferred |
| 0x96CB70 | `human_anim_set_update` | cdecl (human, stance (-1 = keep +0x590), a, b, c) | rig +0x5E0 must exist; sets +0xCC bit 0x40000000 by vehicle class / seat, picks the set with 0x96EF10 and applies it (0x96C670, 0x6EF620, anim_rig_prepare, 0x9B84A0 ...); 234 callers incl. human_update_core and vehicle_pre_update (actions 0x218 / 0x21A) | inferred |
| 0x98E710 | `human_party_leader` | EDX = human -> EAX | the human in handle +0x2A4 when it is a live, unflagged type-1 object, else EDX itself; +0x2A4 / +0x2A8 are written by 0x98EC10 and the AI (0x437EB0); 36 callers | inferred |
| 0x98E750 | `human_party_root_leader` | EDX = human -> EAX | as 0x98E710 but tries +0x2A8 first, then +0x2A4, else EDX | inferred |
| 0x98E8A0 | `human_is_player_or_player_follower` | EDX = human -> AL | false for a player (+0x4C == 1, +0x3CC == 1) itself; true when 0x98E710 or 0x98E750 gives a player; 128 callers | inferred |
| 0x98E7C0 | `human_effective_leader` | EDI = human -> EAX | kind 2 with +0x11C6 bit 0 -> itself; else the +0x2A4 human when live, not dead, not hidden (+0x80 bit 4) and +0xC8 bit 0x400000; else itself; 85 callers | inferred |
| 0x98EC10 | `human_set_party_leader` | cdecl (leader, follower, bool) | remote owners get a 0x68 packet (net_send_packet); locally writes follower +0x2A4 / +0x2A8, npc_set_force_flag_independent_follower, co-op partner checks, human_set_max_hit_points / human_set_hit_points; 33 callers from missions / activities | inferred |
| 0x9A3E20 | `human_fell_through_world_recover` | ESI = human | string 'Scripted NPC "%s" destroyed because it fell through the world. Pos: %0.2f %0.2f %0.2f'; scripted NPCs lose invulnerable / never_die and are killed (human_take_damage, script callback); others go to nav_closest_node_find and object_teleport; from humans_post_update | confirmed |
| 0x9C31B0 | `human_weapon_world_transform` | EDI = human, cdecl args (a bool flag among them) | held-weapon world transform (doc 15, humans_post_update step 8): vehicle_interaction_rider_find, rig_bone_transform, 0x6E5E10, 0xAE53F0; 32 callers incl. weapon_fire, human_ai_think, weapon_reload_step | inferred |
| 0x492D00 | `human_slowmo_rise_list_remove` | EDX = human | finds it in the 3-slot list 0xF53914 (0x493860), shifts the rest down and clears the last slot 0xF5391C; from human_update_core when the per-human time scale passes 1.0 | inferred |
| 0x493860 | `human_slowmo_rise_list_find` | EDX = human, ESI = &index (may be 0) -> AL | only when [0xE8825F]; scans 0xF53914[0..2] | confirmed |
| 0x98E270 | `human_in_attachment_table` | EDX = human -> AL | handle +0x44 in the 24-entry table 0x2C29278 (0x1C-byte rows) kept by human_attachments_update / human_knockdown; human_update_core step 26 | inferred |
| 0x9DD7A0 | `player_busted_handle` | no args | local player (0x21703D4): sets +0x1160 bit 1, clears +0x1162 bits 0..2, stat_award_check, 'MISSION_FAILURE_BUSTED_TEXT' / 'You got busted.', act_packet_send_mission_failed; only from human_state7_update | confirmed |

### AI

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0x436B80 | `human_ai_set_force_flag_drunk` | ECX = human, stdcall (bool), ret 4 | strings 'human_ai_data', 'ai_force_flags' + 'drunk', 'Drunk' anim variants (0x968A70 / 0x968BB0 / 0x9690E0), net_send_packet for remote owners; same family as human_ai_set_force_flag_lethal_attack_against_player | confirmed |
| 0x438710 | `human_from_handle_active` | EAX = handle, cdecl (bool require_alive) -> EAX human or 0 | live type-1, not +0x80 & 0xB, not +0x80 bit 4, alive when asked (human_is_dead), +0xC8 bit 0x400000 set; 111 callers | confirmed |
| 0x43A620 | `human_ai_order_target_human` | EAX = human -> EAX | by order kind +0x149: 0 -> 0; 6 -> +0x154 unless sub-kind +0x148 == 0x4B or +0x150 bit 1; 0x1F with sub-kind 0xBC / 0xBD -> +0x154; else +0xAF0; each through human_from_handle_active(1); 117 callers | inferred |
| 0x43A5C0 | `human_ai_choose_weapon` | EAX = human, cdecl (x) -> EAX weapon | +0x102 bit 1 -> the held +0x10CC; else 0x43A190 and 0x42EE10 feed 0x55B2C0 on the inventory +0x10C4; 91 callers incl. ai_human_perception_update | inferred |

### Spawning and world

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0xB7CD00 | `distant_peds_spawn_update` | no args | tail jmp at the end of world_spawn_do_frame; distant-ped allocator (allocator<>::num_used_88), flags 0x215DA04, params 0xE87038.., rand table 0xEC3AC0, floorf_wrapper / _ftol2_sse; partner of distant_peds_update | inferred |
| 0xB80130 | `distant_vehicles_spawn_update` | no args | string 'distant vehicles temporary mempool', allocator<>::num_used_89, flags 0x215D9FC, _qsort; last call of world_spawn_do_frame before the tail jmp | inferred |
| 0x948D30 | `parked_vehicles_do_frame` | no args | skips while 0x9E1EA0(player) or any of the six flags 0x215DA14 is clear; 0x9484A0, 0x947F40, 0x9480A0, 0x948120, parked_vehicles_spawn_update(0); from world_spawn_do_frame | inferred |
| 0x9413A0 | `spawn_groups_check_categories` | no args | debug strings 'AnoopS bug:  We need at least one %s spawn group in (either) spawn category %s ...', table 0x2529634 (0x1C-byte rows); from 0x941700 | confirmed |
| 0x94E760 | `spawn_record_any_member_visible` | cdecl (record) -> AL | record vehicles (+0x70 count, +0x74 handles, type 5) and kind-2 humans (+0x84, +0x88) through object_visibility_cached(2); 0x94EA50 culls records of list 0x215E090 that fail this and 0x9DFCB0(8100) | inferred |
| 0xAE2C80 | `spawn_carjack_vehicle` | cdecl (spawn request) | string 'CarJack', object_template_find, vehicle_create, vehicle_ai_order_08; from 0x93A2E0 (traffic spawn) | inferred |

### Physics

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0xAAB5E0 | `object_has_static_contact` | EAX = object -> AL | walks the contact list 0x175CD80[handle & 0xFFF] (filled by physics_contact_pair_record, next +0x10, circular) and is true when a node's other object (+0) is 0, i.e. touching static world; 29 callers | inferred |
| 0x710DB0 | `ragdoll_find_for_object` | ECX = object -> EAX ragdoll record or 0 | table 0x29600F0 (0x12A0-byte records, handle at +0xF70, end 0x296EFE0) with the last-hit cache 0xED7428 (-1 on miss); 56 callers | confirmed |

### Network property senders

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0x968A70 | `net_send_prop_human_stand_variant` | stdcall (human, value), ret 8 | strings 'human' / 'stand_variant' hashed with 0xBDCA30, local write or net_send_packet; same shape as net_send_prop_human_ai_data_worried_walk; 23 callers | confirmed |
| 0x968BB0 | `net_send_prop_human_walk_variant` | stdcall (human, value), ret 8 | strings 'human' / 'walk_variant'; 21 callers | confirmed |
| 0x9690E0 | `net_send_prop_human_run_variant` | stdcall (human, value), ret 8 | strings 'human' / 'run_variant'; 17 callers | confirmed |
| 0x968930 | `net_send_prop_human_team` | stdcall (human, value), ret 8 | strings 'human' / 'team'; 49 callers | confirmed |
| 0x443740 | `net_send_prop_human_ai_data_forced_target_handle` | stdcall (ai data, &handle), ret 8 | strings 'human_ai_data' / 'forced_target_handle'; local owner writes +0x10, else a 0x6E packet; 38 callers | confirmed |
| 0x80BA60 | `net_bandwidth_over_budget` | no args -> AL | sum of 0x7FB080 over the session players (0x2528C14 +0x400, next +0x22CC) / (+0x408 -> +0x1DC4 / 8) > 0.05; when true world_spawn_do_frame skips spawn groups, traffic and the ambient budget | inferred |

### Weapons and audio

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0xB716E0 | `weapon_info_find_by_name` | cdecl (name) -> EAX weapon info or 0 | __stricmp over 0x22D7BCC (0x498-byte records, count 0x22D7BD0); callers pass 'm16', 'pipe_bomb', 'beretta', 'mcmanus2010', crib / spawn tables; 126 callers | confirmed |
| 0xB72740 | `weapon_info_is_melee` | EAX = weapon info -> AL | class +0x34 in {6, 7, 9..14, 19} (disjoint from weapon_info_is_firearm); 86 callers | inferred |
| 0x465EC0 | `audio_voice_handle_valid` | ECX = &voice handle -> AL | low byte = slot in 0xEEB8F8 (0x19C-byte records), upper 24 bits = serial at +0xB4; true when the serial matches and the slot is in use; 73 callers | confirmed |

### Shared helpers (math, timers, random)

| Address | Name | Registers / return | Evidence | Role |
|---|---|---|---|---|
| 0xBDD1B0 | `timer_set_random` | EDI = &timestamp, EAX = max ms, stack min ms (ret 4) | ts = now [0xE843AC] + min + rand % (max - min + 1) from the 2048-entry table 0xEC3AC0 (index 0xEC3ABC), wrapped like timer_set; 193 callers | confirmed |
| 0x40EA20 | `sqrtf_wrapper` | cdecl (f32) -> ST0 (f32) | 0xCA04D0 (CRT sqrt) then a f32 round trip; 605 callers (vec3_length, vec3_distance, vec3_normalize_in_place) | confirmed |
| 0xBDEA30 | `vec3_distance_sq` | ECX = &a, EAX = &b -> ST0 | sum of squared component differences, each difference and partial sum rounded to f32; 333 callers | confirmed |
| 0xBDE980 | `vec3_distance` | cdecl (&a, &b) -> ST0 | sqrtf_wrapper of the squared difference; 100 callers | confirmed |
| 0x412080 | `vec3_length` | EAX = &v -> ST0 | sqrtf_wrapper(x*x + y*y + z*z); 84 callers | confirmed |
| 0xBDDFF0 | `vec3_normalize_in_place` | ESI = &v | v *= 1 / sqrtf_wrapper(\|v\|^2), no zero check; 212 callers | confirmed |
| 0xBDE570 | `vec3_mul_mat33_in_place` | ECX = &v, EAX = &m (3x3 rows at +0 / +0xC / +0x18) | v = x*row0 + y*row1 + z*row2; 171 callers | confirmed |
| 0xBDCE30 | `rng_float01` | EAX = rng (index +0x9C4, 2048-entry table +0x9C8) -> ST0 | next u32 * 2^-32 as f32, index wraps at 0x800; 393 callers | confirmed |
| 0xBDCF50 | `rand_int_range` | ESI = min, ECX = max -> EAX | min + table[0xEC3ABC] % (max - min + 1), global table 0xEC3AC0; 209 callers | confirmed |

## Port candidates

These are 15 small functions from the list above. Each is a leaf or a fixed call sequence, and none needs x87 beyond plain f32 round trips. Handle lookups use the usual slot table: the object is the dword at `0x2149C64 + (h & 0xFFF) * 16`, it must be non-null, and its +0x44 must equal h. Below this is written `resolve(h)`.

1. **0x926650 `human_equals_local_player`** (EAX = human → AL).
   - `AL = (EAX != 0 && EAX == [0x21703D4])`.
   - Thunk: EAX in, AL out. Leave the upper bits of EAX as they are: the original only writes AL.

2. **0x9855F0 `human_is_dead_or_invalid`** (EDX = human → AL).
   - If EDX == 0: return 1.
   - o = `resolve([EDX+0x44])` (0 when the handle is 0). If o != EDX: return 1.
   - Return 0 if `0.0 < f32 [EDX+0x1124]`, else 1. A NaN gives 1.

3. **0x9D06A0 `human_from_handle_non_scripted`** (ECX = handle → EAX).
   - o = `resolve(ECX)`. Return 0 unless o is valid and o+0x4C == 1.
   - Return o if byte o+0x3CC == 2, else 0. **Correction:** this line said the opposite. `setne cl` gives 0 when the kind is 2; `sub ecx, 1` turns that into −1 (all bits set), so `and eax, ecx` keeps o. Any other kind gives `ecx` = 0 and returns 0. The name `human_from_handle_non_scripted` is therefore inverted; the row is a ported (rust) row, so the rename is left to the porting session.
   - Branch-free in the original (`setne` / `sub 1` / `and`); a plain `if` gives the same value.

4. **0x438710 `human_from_handle_active`** (EAX = handle, cdecl (u8 require_alive) → EAX; the caller cleans 4).
   - o = `resolve(EAX)`; it must be type 1 (+0x4C == 1).
   - Then f = o+0x80. Return 0 if f & 0xB, or (require_alive and `human_is_dead(cdecl o)`), or f & 0x10, or `!(o+0xC8 & 0x400000)`.
   - Otherwise return o.

5. **0xB07A30 `vehicle_interaction_rider_find`** (ESI = human handle → EAX; ECX and EDX are clobbered).
   - Return 0 if ESI == 0.
   - o = `resolve(ESI)`; it must be type 1. r = o+0x10E0; return 0 if r == 0.
   - For i in 0..8: if dword `r + 0x34C + i*0xA8` == ESI, return that address.
   - Otherwise return 0.

6. **0x9B9320 `human_vehicle_interaction_entering`** (cdecl human → AL). 0x9B9350 is the same with 2.
   - s = `vehicle_interaction_rider_find(ESI = [human+0x44])`. ESI must be preserved around the call: the original pushes and pops it.
   - `AL = (s != 0 ? [s+4] : 0) == 1`. The original compares EAX itself with 1 when s == 0, which can never match.

7. **0xAA7380 `vehicle_seat_occupant_by_index`** (ECX = vehicle, EDX = seat → EAX).
   - Return 0 for a null vehicle.
   - n = (byte +0x84D8 == 1) ? +0x6CE8 : 8. With +0x84D8 == 1 and n == 0, return 0.
   - If (signed) EDX >= n: return 0.
   - Otherwise tail call `vehicle_seat_occupant(EAX = [ECX + 0x8ABC + EDX*0x24])`, which is a handle → human lookup. Call it through an EAX thunk or port it inline: `resolve(h)`, then type 1 → o, else 0.

8. **0xAA68E0 `vehicle_get_velocity`** (ECX = vehicle, cdecl (out) — the caller cleans 4, frame aligned to 16).
   - If byte +0x84D8 == 1: out = the three f32 at `[[ECX+0x554] + 0x1A0]` (loaded with `movaps` and stored through x87 `fld` / `fstp` f32).
   - Otherwise out = the three dwords at 0x252A6D0..0x252A6D8 (a plain integer copy).
   - A bit copy matches except that a signalling NaN would be quietened by `fld`. Use f32 loads and stores if that matters.

9. **0xAAB5E0 `object_has_static_contact`** (EAX = object → AL; ECX is clobbered).
   - h = [EAX+0x44] & 0xFFF.
   - first = [0x175CD84 + h*8]; n = first.
   - While n != 0:
     - if [n] == 0, return 1;
     - n = [n+0x10];
     - if n == first, break.
   - Return 0.

10. **0xAAB6B0 `vehicle_any_wheel_in_contact`** (EDI = vehicle → AL).
    - Return 0 for a null vehicle, for class `[[EDI+0x84E4]+0x9C]` 3 or 4, when byte +0x84D8 != 1, or when the list L = [EDI+0x6CE0] is 0.
    - For i in 0..[L]: skip parts with `[[L+4+i*4]+0x11C] & 0x600000`. Otherwise call the thiscall `vtbl[0x40/4]` of p = [EDI+0x53C] with ECX = p and one stack arg i; stop when the result != -1.
    - `AL = (i != [[EDI+0x6CE0]])`. The count is read again at the end.

11. **0x710DB0 `ragdoll_find_for_object`** (ECX = object → EAX; EDX is clobbered).
    - c = [0xED7428]. If c > 0 and dword `0x2961060 + c*0x12A0` == [ECX+0x44]: return 0x29600F0 + c*0x12A0. Index 0 is never served from the cache.
    - Otherwise scan i = 0.. while `0x2961060 + i*0x12A0 < 0x296EFE0` (12 records):
      - on a match, set [0xED7428] = i and return the record address;
      - after the scan, set [0xED7428] = -1 and return 0.

12. **0xBDD2A0 `timer_elapsed_ms`** (named by doc 42 during this round) (EAX = &ts → EAX; EDX is clobbered, and ECX in one branch).
    - t = [EAX]. If t < 0: return −1,800,000,000.
    - n = [0xE843AC].
    - If t > n: d = t − n; return −d when d ≤ 900,000,000, else −(d − 1,800,000,000).
    - Otherwise: d = n − t; return d when d ≤ 900,000,000, else −(t − n + 1,800,000,000).
    - Use 32-bit wrapping arithmetic.

13. **0xBDD1B0 `timer_set_random`** (EDI = &ts, EAX = max, stdcall arg min, `ret 4`; ECX and EDX are clobbered).
    - Advance the shared random table first: k = [0xEC3ABC]; v = [0xEC3AC0 + k*4]; k += 1; if k >= 0x800 then k = 0; store k.
    - r = min + (v % (max − min + 1)), using an unsigned `div`.
    - If r ≥ 0 (as i32): ts = now + r; if ts > 1,800,000,000 (signed `jle` fails), ts −= 1,800,000,000.
    - If r < 0: ts = now + r; if ts < 0, ts += 1,800,000,000.
    - Store ts to [EDI]. The table index must advance exactly as here, because `rand_int_range` and others share 0xEC3ABC.

14. **0xBDEA30 `vec3_distance_sq`** (ECX = &a, EAX = &b → ST0).
    - Per axis: d = f32(a − b), stored through a f32 temporary.
    - acc0 = f32(dx*dx); acc1 = f32(f64(dy)*f64(dy) + acc0); acc2 = f32(f64(dz)*f64(dz) + acc1).
    - Return acc2 widened (loaded from the f32 temporary).
    - Do the products and sums in f64 and round only at the stores. Rounding after every operation in f32 can differ in the last bit.
    - Return it through the ST0 return shim the f32-returning ports already use.

15. **0x948D30 `parked_vehicles_do_frame`** (no args, plain call sequence).
    - If `0x9E1EA0(EAX = [0x21703D4])` returns AL != 0: return.
    - If any byte 0x215DA14..0x215DA19 is 0: return.
    - Call 0x9484A0, 0x947F40, 0x9480A0 and 0x948120 (no register inputs; the `lea ecx, [ecx]` at 0x94812D is padding), then `parked_vehicles_spawn_update(cdecl 0)` (0x948770).
    - It is a direct child of `world_spawn_do_frame`, which is already a Rust skeleton in `frame.rs`, so it can hang off it.

Also easy, but not in the 15:
- `aa0d10` (memmove plus one x87 subtraction loop and a virtual call);
- `9b5c00` (field stores and a timer);
- `98e2a0` (a two-way f32 constant);
- `b716e0` (stricmp table walk);
- `465ec0` (handle check).
