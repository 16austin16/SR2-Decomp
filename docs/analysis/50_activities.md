# 50: Activities, diversions and mission flow (names, slot roles, port maps)

This document covers the activity / diversion / mission-flow code at 0x53xxxx-0x68xxxx plus the mission flow at 0x6Axxxx. It adds:

- `docs/candidates_activities.csv`: new function rows (note suffix `(50)`);
- `docs/renames_activities.csv`: renames of existing `Class::vfN` rows (columns `address,old_name,new_name,note`); nothing in `functions.csv` was edited;
- the vtable slot roles of the activity and diversion families (below);
- the game mode callback table (new, below), which explains most of the "unreached" functions;
- port maps for the ten largest whole-portable functions in the area, and recipes for fifteen small ones.

Method. Function sizes were measured by linear disassembly to the last `ret` after the furthest forward branch / jump-table target. Plain forward `jmp` out of a function into the next known function start counts as a tail call and does not extend it. This matters: the first pass, which counted tail jumps, put `fraud_info::vf15` at 11474 bytes (true size 287, ends in `jmp 0x652860`) and `piracy_level_instance::update` at 7621 bytes (true size 610). All sizes below are the corrected ones. Register conventions come from a read-before-write scan over all paths (so `ESI`/`EDI`/`EBX` read at entry mean a register argument) plus `ret N`; they were cross-checked at one caller for every function that is given a recipe. "Callee reads" in the map tables is the same scan run on each callee; stack argument counts come from the caller's `add esp, N` or the callee's `ret N`. Where a callee takes stack arguments the scan cannot count them, the table says so.

Confidence words: *confirmed* = a dispatcher or caller shows the role and the body agrees; *inferred* = strings / callees / shape only.

## Game mode callback table

`game_mode_do_frame` (0x68F4B0) and the `game_event_*` block (37_caller_survey_2.md) call through a table at `0xE96C10 + mode * 0x6C`, 27 dword entries per mode. The event number n is the entry index. Entries found in this pass:

| Index | Role | Mode 0 (activities) | Mode 2 (diversions) | Mode 4 (missions) | Mode 5 (racing) |
|---|---|---|---|---|---|
| 0 | init (boot, every mode; `0x68F3FB` loops all modes) | 0x53AE10 | 0x603310 | - | 0x6829B0 |
| 1 | reset (`0x68F420` calls it for every mode, or only for the old mode in the MP-host case) | 0x53B010 | 0x603570 | - | - |
| 2 | enter | 0x53B480 | 0x6035A0 | 0x6A49B0 | 0x6829E0 (empty) |
| 3 | leave (`0x68F420` calls the old mode's entry 3) | 0x53B7E0 | 0x603830 | 0x6A4D10 | 0x6829F0 |
| 4 | frame (`game_mode_do_frame` calls `[+0x10]`) | 0x53BCA0 | 0x603610 | - | 0x682A20 |
| 6 | time step (float dt, cdecl) | 0x53BE30 | 0x603800 | - | - |
| 10 | event 10 human killed | 0x53BED0 | 0x603C20 | 0x6A5900 | - |
| 11 | event 11 object killed | 0x53BF50 | 0x603C60 | 0x6A5980 | - |
| 12 | event 12 vehicle destroyed | 0x53BFD0 | 0x603C90 | 0x6A59E0 | - |
| 13 | event 13 (unknown) | 0x53C070 | 0x603CC0 | - | - |
| 14 | event 14 object damaged | 0x53C1E0 | 0x603D00 | - | - |
| 15 | event 15 | 0x53C210 | 0x603D30 | - | - |
| 17 | event 17 player respawned | - | 0x603D70 | - | - |
| 19 | event 19 | 0x53C2A0 | 0x603DD0 | - | - |
| 20 / 21 / 22 | events 20 / 21 / 22 | 0x53C2D0 / 0x53C2F0 / 0x53C310 | 0x603D90 / 0x603DB0 / 0x603E00 | - | - |
| 23 | event 23 | 0x53C330 | - | - | - |
| 26 | event 26 | 0x53C350 | - | - | - |

Mode 1 (entries 0x5665A0, 0x5665B0) is the pushback contest mode, mode 3 has no callbacks (free roam), modes 6 and 7 are multiplayer (0x840B20..., 0x88E020...). Mode ids: the mission code stores 3 / 4 into `[0xE96C04]` as the pending mode (22_missions.md), the diversion manager stores 2 (`0x603274`).

The mode 0 callbacks forward each event to the **current activity** `[0x1408E88]` (an `activity_instance`): through `[activity+0x10]` (the `activity_level_instance`) at vtable offset `4 * slot`; the mission info object `[0x27B3C60]+0x90` (when `[mission+0x3C] == 1`, i.e. the mission is an activity) gets the same event first for events 10-15. The mode 2 callbacks forward to the current diversion instance `[0x2527888]` at offsets `+0x14 ... +0x3C`. Entry 13/15/19 argument counts are in the roles tables.

Surprise: `activity_instance` slot 7 (`update`, 0x53E4D0, shared by 14 classes) is only 0x238 bytes: a fade state machine (`[this+0x20]` 0..3) followed by a 10-way phase dispatch on `[this+0xC]` into `0x53E740 ... 0x53F400`. The earlier size guess (3885) included those phase functions.

## Vtable slot roles

Slot numbers are vtable indices (offset / 4). Renames are in `renames_activities.csv`; only roles with evidence were applied.

### activity_info (26 slots, 17 level types + variants)

Already named in `vtable_slots.csv`: 0 `load_from_table`, 1 `get_activity_type` (constants 0 demo derby ... 16 vandalism). Slots 2-22 are mostly 3-byte stubs shared by 17+ classes (`ret`, `xor eax,eax; ret`): no callers found, left as `vfN`. Slots 23 / 24 (0x53DCB0 / 0x53DD10, 87 bytes, shared by 17 classes) resolve a navpoint by the name hash at `[this+0x12C]` plus a level index (`navpoint_find`); they return `[this+8]` when the name is absent - candidates for `get_start_navpoint` / `get_end_navpoint`, not applied.

### activity_instance (11 slots)

| Slot | Role | Evidence | Confidence |
|---|---|---|---|
| 1 | load_from_table | vtable_slots.csv | confirmed |
| 5 | get_type_record | `[info vtable+4]` then `imul 0x68`, dword at `0x27966C0 + type*0x68` (-1 if no info) | confirmed |
| 6 | stop | frees `[this+0x10]` (level instance, vf2), releases the pool slot, resets 0x2527430 / 0xE9A6BC / 0xE9A6C4 | inferred |
| 7 | update | per-frame state machine called from the activity frame path; jump tables 0x53E708 (fade state) and 0x53E718 (phase) | confirmed |
| 10 | on_game_event_26 | forwards 3 args (ret 0xC) to level_instance slot 20 | confirmed |
| 0, 2, 3, 4, 8, 9 | unknown | `ret N` stubs (N = 4, 8, 4, 4, 0xC, 0xC); fight club overrides slot 8 | open |

Phase functions of `update` (new names): phase 0 `activity_instance_phase_choose` 0x53E740, 1 `..._teleport_to_start` 0x53E900, 2 `..._level_complete` 0x53EBA0, 3 `..._start_level` 0x53ECB0, 4 `..._run_level` 0x53ED30, 5 `..._restart_or_exit` 0x53EDB0, 6 `..._wait_timer` 0x53F3A0 (EDX = this), 7 `..._failed` 0x53F400; phases 8 and 9 end the activity (default case `setge al`).

### activity_level_instance (21-25 slots)

| Slot | Role | Args | Event / evidence | Confidence |
|---|---|---|---|---|
| 0 | load_from_table | ret 0xC | vtable_slots.csv | confirmed |
| 1 | load_xtbl | ret 4 | vtable_slots.csv | confirmed |
| 3 | update | none, returns status -1..3 | activity phase 4 maps it (`+1`, jump table 0x53ED94) | confirmed |
| 4 | on_time_step | float | callback 6 | confirmed |
| 6 | on_human_killed | 4, ret 0x10 | event 10 | confirmed |
| 7 | on_object_killed | 4 | event 11 | confirmed |
| 8 | on_vehicle_destroyed | 4 | event 12 | confirmed |
| 9 | on_game_event_13 | 6, ret 0x18 | event 13 | confirmed |
| 10 | on_object_damaged | 4 | event 14 | confirmed |
| 11 | on_game_event_15 | 5, ret 0x14 | event 15 | confirmed |
| 12-15 | on_game_event_20 / 21 / 22 / 23 | 1 each | events 20-23 | confirmed |
| 16 | on_game_event_19 | 4 | event 19 | confirmed |
| 17 | get_state | none | `mov eax,[ecx+0xC]` | inferred |
| 18 / 19 | unknown bool queries | `xor al,al; ret 8` / `ret 4` | fight club overrides are the same stubs | open |
| 20 | on_game_event_26 | 3, ret 0xC | event 26 via activity_instance slot 10 | confirmed |
| 2, 5 | unknown | 2: 81-byte base; 5: `ret` | - | open |

### diversion_type_info (35-41 slots)

Existing: 0 init, 2 get_type, 4 update, 5 can_start, 6 get_start_message, 8 create_instance, 10 get_title, 11 get_hud_icon_name, 16 get_start_mode, 21-29 and 33 event slots (vtable_slots.csv, 37). New:

| Slot | Role | Evidence | Confidence |
|---|---|---|---|
| 7 | start_instance | base 0x602B80 (38 classes): calls slot 8 with pool 0x2796D98, sets `inst+8 = this`, calls instance `start` and returns it; `game_mode_diversion_enter` stores the result in 0x2527888 | confirmed |
| 17 | set_mode_enabled | `ret 4`; tow truck override sets the +0x14 byte to the argument unless MP-host; `game_mode_diversion_init` calls it | inferred |
| 19 | set_enabled | overrides write the +0x14 byte read by `diversion_mode_frame`; the base is `ret 4` | confirmed |
| 32 | on_player_left | 0x604940 dispatches it from `game_event_player_left` | confirmed |
| 1, 3, 9, 12-15, 18, 20, 30, 31, 34 | unknown | stubs; slot 3 base returns 1 and gates the slot-18/19 calls in the shutdown loop (0x602C80 / 0x603440); slot 34 is reached by tail jumps `jmp 0x604710` at 0x6A2501 / 0x6A2598 / 0x6A25D5 (mission start path); 30 / 31 have dispatchers 0x6048E0 / 0x604910 with no caller | open |

Note: some diversion types have a few real extra slots 35-37 (base jumping, cat and mouse, death tag, ...), but for `ambulance_diversion_type_info` (`n_virtuals` 41) slots 35-40 are not code: values such as 0x610042, 0x740073, 0x69006D are UTF-16 string data after the vtable, so `rtti_classes.csv` over-counts that class (and a few others). Do not name those addresses.

### diversion_instance (19-22 slots)

| Slot | Role | Args | Evidence | Confidence |
|---|---|---|---|---|
| 0 / 1 | start / stop | - | vtable_slots.csv | confirmed |
| 2 | update | none -> status 0..3 | `game_mode_diversion_frame` jump table 0x6037EC; base returns 3 | confirmed |
| 3 | on_time_step | float, ret 4 | callback 6 | confirmed |
| 5 | on_human_killed | 5, ret 0x14 | event 10 | confirmed |
| 6 | on_object_killed | 4, ret 0x10 | event 11 | confirmed |
| 7 | on_vehicle_destroyed | 4 | event 12 | confirmed |
| 8 | on_game_event_13 | 5 | event 13 | confirmed |
| 9 | on_object_damaged | 4 | event 14 | confirmed |
| 10 | on_game_event_15 | 5 | event 15 | confirmed |
| 11 | on_player_respawned | 1 | event 17 | confirmed |
| 12-14 | on_game_event_20 / 21 / 22 | 1 each | events 20-22 | confirmed |
| 15 | on_game_event_19 | 4 | event 19 | confirmed |
| 4, 16-21 | unknown | stubs / per-type | - | open |

### Other families

- `complication` (trafficking / pimping route complications; 14 slots): 2 `load_from_table`, 5 `roll_chance`, 7 `get_value`, 9 `remap_value`, 10 `activate`, 11 `deactivate` (all confirmed by body). 3 is a bool stub (`xor al,al` base, `mov al,1` in button mashing), 13 returns `[this+0xC]`.
- `heli_objective` (22 slots): 2 `load_from_table`, 16 `reset`, 18 `clear_flag_1a`, 19 `update` (inferred), 20 `start` (doc 29), 21 `get_start_message`. Slots 0 / 1 / 7 / 8 are plain getters (`[+0x14]`, `[+0x18]`, `[+8]`, `[+4]`).
- `fuzz_crime` (22 slots, 45 classes): known: 0 init, 1 update, 7 enter_state2, 8 tick_state2, 9 enter_state3, 11 any_participant_near_player, 12 get_bar_title, 14 get_bar_progress. New: 15 `get_state` (`[this+0x9C]`), 17 `get_target_position_ptr` and 18 `get_target_flags` (inferred, both read `[this+0x20]`, the crime's primary human). Slots 2-6 are event handlers with 6 / 1 / 5 / 4 / 4 stack args (`ret 0x18 ... ret 0x10`) shared by 40+ classes.
- `special_vehicle_spawn_group` (escort / skirmish / vandalism-news / cell / notoriety): 7 on_table_loaded, 8 reset, 9 update_streaming, 10 update, 11 is_active already named; no new roles applied.


## Names table (new rows, `candidates_activities.csv`)

177 new rows. Full notes (register conventions, sizes, callers) are in the CSV.

| Address | Name | Subsystem | Note |
|---|---|---|---|
| 0x0053AE10 | `game_mode_activity_init` | activity | game mode 0 callback 0 (table 0xE96C10): resolves the sounds SYS_ACT_COMPLETE / SYS_ACT_FAIL (0xE987F0 / 0xE987F4), resolves the 17 activity type reco (confirmed) |
| 0x0053B010 | `game_mode_activity_reset` | activity | game mode 0 callback 1: zeroes the previous / current activity 0x1408E8C / 0x1408E88, clears the 0x14090B0 block and the per-type words at 0x27966F8 + (confirmed) |
| 0x0053B480 | `game_mode_activity_enter` | activity | game mode 0 callback 2 (enter): calls mission_enter_common (0x6A43F0), builds the activity instance (0x53A640) into 0x1408E88 and starts it (inferred) |
| 0x0053B7E0 | `game_mode_activity_leave` | activity | game mode 0 callback 3 (leave): tears the current activity down (audio_voice_stop, 0x5B7810, 0x93C610) (inferred) |
| 0x0053BCA0 | `game_mode_activity_frame` | activity | game mode 0 callback 4 (frame, called by game_mode_do_frame at [mode*0x6C+0xE96C20]): checks the co-op state and ends the mission through mission_end (inferred) |
| 0x0053BE30 | `game_mode_activity_time_step` | activity | game mode 0 callback 6, cdecl (float dt): level instance on_time_step (+0x10) (confirmed) |
| 0x0053BED0 | `game_mode_activity_on_human_killed` | activity | game mode 0 callback 10 (event 10), cdecl 4 args: if the mission is an activity (kind 1) calls its info vtable +0x4C, then the level instance on_human (confirmed) |
| 0x0053BF50 | `game_mode_activity_on_object_killed` | activity | game mode 0 callback 11 (event 11): mission info vtable +0x50, then level instance on_object_killed (+0x1C) (confirmed) |
| 0x0053BFD0 | `game_mode_activity_on_vehicle_destroyed` | activity | game mode 0 callback 12 (event 12): mission info vtable +0x54, then level instance on_vehicle_destroyed (+0x20) (confirmed) |
| 0x0053C070 | `game_mode_activity_on_game_event_13` | activity | game mode 0 callback 13 (event 13): sets hit points (human_set_hit_points) for the local human, mission info vtable +0x44, then level instance +0x24 (confirmed) |
| 0x0053C1E0 | `game_mode_activity_on_object_damaged` | activity | game mode 0 callback 14 (event 14), cdecl 4 args: forwards to level instance on_object_damaged (+0x28) (confirmed) |
| 0x0053C210 | `game_mode_activity_on_game_event_15` | activity | game mode 0 callback 15 (event 15), cdecl 5 args: mission info vtable +0x48 then level instance +0x2C (confirmed) |
| 0x0053C2A0 | `game_mode_activity_on_game_event_19` | activity | game mode 0 callback 19 (event 19), cdecl 4 args: level instance +0x40 (confirmed) |
| 0x0053C2D0 | `game_mode_activity_on_game_event_20` | activity | game mode 0 callback 20 (event 20), cdecl 1 arg: level instance +0x30 (confirmed) |
| 0x0053C2F0 | `game_mode_activity_on_game_event_21` | activity | game mode 0 callback 21 (event 21), cdecl 1 arg: level instance +0x34 (confirmed) |
| 0x0053C310 | `game_mode_activity_on_game_event_22` | activity | game mode 0 callback 22 (event 22), cdecl 1 arg: level instance +0x38 (confirmed) |
| 0x0053C330 | `game_mode_activity_on_game_event_23` | activity | game mode 0 callback 23 (event 23), cdecl 1 arg: level instance +0x3C (confirmed) |
| 0x0053C350 | `game_mode_activity_on_game_event_26` | activity | game mode 0 callback 26 (event 26), cdecl 3 args: activity_instance on_game_event_26 (+0x28) on 0x1408E88 (confirmed) |
| 0x0053C4D0 | `activity_current_type` | activity | returns the activity type id (info vtable +4 = get_activity_type) when the current mission 0x27B3C60 is an activity (kind [+0x3C] == 1), else -1 (confirmed) |
| 0x0053D610 | `activity_progress_get_levels_done` | activity | ECX = &name hash: finds the 12-byte entry in 0x2796E9C (count 0x279719C) whose key matches and returns its levels-completed dword [+4], or -1 (confirmed) |
| 0x0053D650 | `activity_progress_levels_done_mask` | activity | ECX = &name hash, EDI = level index: 1 when the bits 0..EDI of the entry flag byte [+8] are all set (every level up to EDI done), else 0 (confirmed) |
| 0x0053E740 | `activity_instance_phase_choose` | activity | activity_instance::update phase 0 (ESI = activity): decides between level select, replay prompt (ACTIVITY_COOP_SKIP_TEXT) and finishing when levels_do (inferred) |
| 0x0053E830 | `activity_release_player_control` | activity | calls 0x4F5210 and clears the global 0x2527430 that update phase 0 sets from 0x4F50E0 (player control lock) (inferred) |
| 0x0053E900 | `activity_instance_phase_teleport_to_start` | activity | update phase 1 (ESI = activity): racing special-case string, player_teleport of the player (and co-op partner) to the start, vehicle entering handling (inferred) |
| 0x0053EBA0 | `activity_instance_phase_level_complete` | activity | update phase 2 (EDI = activity): string 'Level Complete', mission checkpoint save (0x6A1440), co-op sync (0x53FDD0), moves to the next phase (inferred) |
| 0x0053ECB0 | `activity_instance_phase_start_level` | activity | update phase 3 (ESI = activity): 0x53F960(level instance, fade state == 2) then starts the level (0x6B7260) and sets phase 5 (inferred) |
| 0x0053ED30 | `activity_instance_phase_run_level` | activity | update phase 4 (ESI = activity): calls level instance update (vtable +0xC) and maps the result -1..3 to the next phase (0 -> clear 0x2526D96, 1 -> [es (confirmed) |
| 0x0053EDB0 | `activity_instance_phase_restart_or_exit` | activity | update phase 5 (stdcall ret 4, EBX / EDI / ESI = activity): 1512 bytes (inferred) |
| 0x0053F3A0 | `activity_instance_phase_wait_timer` | activity | update phase 6 (EDX = activity): arms a 1500 ms game-time timer in [edx+0x2C] (0x5DC), then sets phase 8 once the timer window (0x35A4E900) passes (confirmed) |
| 0x0053F400 | `activity_instance_phase_failed` | activity | update phase 7 (EDI = activity): plays the SYS_ACT_FAIL sound (0xE987F4), sends act_packet_send_mission_failed unless the info type is 0xE, sets phase (confirmed) |
| 0x005735A0 | `guardian_angel_state_step` | activity | 4185 bytes, callers guardian_angel_level_instance::load_from_table and ::update: 48-state jump table at 0x5745FC (enemy waves, celebrity vehicle, foll (inferred) |
| 0x00579880 | `satellite_objective::update` | class | heli_objective slot 19 override of satellite_objective (67 bytes) (inferred) |
| 0x0057C080 | `heli_kill_cars_objective_start` | activity | strings ACT_HELI_KILL_CARS_START_MESSAGE, icon_lg_d (inferred) |
| 0x00580A40 | `vehicle_protect_objective_update_body` | activity | 5170 bytes, only caller vehicle_protect_objective::update (0x57EDB0): 5-phase jump table 0x581E74 (inferred) |
| 0x00583260 | `heli_kill_airplane_objective_start` | activity | strings ACT_HELI_KILL_AIRPLANE_START_MESSAGE (inferred) |
| 0x00585C60 | `large_convoy_objective_update_body` | activity | 4628 bytes, only caller large_convoy_objective::update (0x586EC0): 12-case jump table 0x586E74 plus byte table 0x586EA4 (inferred) |
| 0x00589700 | `liberator_objective::load_from_table` | class | heli_objective slot 2 override of liberator_objective (595 bytes, no csv row before) (inferred) |
| 0x0058C500 | `heli_gang_fight_objective_start` | activity | strings ACT_HELI_GANG_FIGHT_START_MESSAGE (inferred) |
| 0x00597610 | `piracy_rumble_start_message` | activity | strings piracy_dk_$lvl_2_pt, ACT_PIRACY_RUMBLE_START_MESSAGE (inferred) |
| 0x00598060 | `racing_activity_level_instance::update` | class | activity_level_instance slot 3 override of racing_activity_level_instance (87 bytes) (inferred) |
| 0x005AB980 | `security_fail_messages` | activity | strings ACT_SECURITY_FAIL_CELEB_ANNOYED / FAIL_TIME_UP / SPRINT_TIP / HELP_LEASH_NOTIFY (inferred) |
| 0x005B5EE0 | `sewage_level_instance::on_time_step` | class | activity_level_instance slot 4 (on_time_step, float dt) override of sewage_level_instance (277 bytes) (inferred) |
| 0x005B9160 | `zombie_level_cleared_check` | activity | string ACT_ZOMBIE_LEVEL_CLEARED_FORMAT (inferred) |
| 0x005DDA00 | `fight_club_is_fighter_handle` | activity | ESI = object handle: 1 when the current mission is an activity of type 3 (fight club) and ESI is in the fighter list 0x140A5C0 (count 0x25276C8), else (confirmed) |
| 0x005DE0C0 | `fight_club_get_countdown` | activity | returns float [info+0x1A4] when the current activity type is 3 (fight club), else 5.0 (0xDD0794) (inferred) |
| 0x005DE330 | `fight_club_get_player_flag` | activity | EAX = human: 0 for NULL, else byte 0x25276C6 for the local player (0x21703D4) and byte 0x25276C7 for anyone else (inferred) |
| 0x005DF120 | `fight_club_fighter_group_clear` | activity | stdcall (group*): calls vtable +0xC on each of the [group+0x148] entries from +0x48, then clears the byte [+0], [+0x44] and the count (inferred) |
| 0x005DF960 | `road_find_random_point_near` | activity | find a random point on the road splines near a position (calls 0xA8A6F0 for the candidates, rng_float01, spline_eval_position / tangent) (inferred) |
| 0x005E1D30 | `activity_button_is_down` | activity | EAX = button index (unsigned, > 7 returns 0 (confirmed) |
| 0x005E1DE0 | `activity_button_just_pressed` | activity | EAX = button index: same mapping as activity_button_is_down (table 0x5E1E68), calls input_action_just_pressed (0x4FF700) (confirmed) |
| 0x005E1E90 | `activity_button_name` | activity | EAX = button index: same mapping (table 0x5E1EDC), tail-jumps to input_action_button_name (0x4FFBB0) with EAX = action (confirmed) |
| 0x005E2A40 | `critical_vehicle_time_to_return_text` | activity | string CRITICAL_VEHICLE_TIME_TO_RETURN_TEXT with icon_lg_c (confirmed) |
| 0x005E2F10 | `critical_vehicle_destroyed_text` | activity | string CRITICAL_VEHICLE_DESTROYED_VEHICLE_TEXT (confirmed) |
| 0x005E2FE0 | `critical_vehicle_update` | activity | 1684 bytes: monitors the activity's critical vehicle (distance, abandoned timer, string CRITICAL_VEHICLE_ABANDONED_VEHICLE_TEXT), sends net packets to (inferred) |
| 0x005E42F0 | `button_mashing_prompt_update` | activity | strings BUTTON_MASHING_UTIL_MESSAGE (confirmed) |
| 0x005E4550 | `stick_rotation_prompt_update` | activity | strings STICK_ROTATION_UTIL_(COUNTER_)CLOCKWISE_MESSAGE (confirmed) |
| 0x005E49F0 | `button_mashing_hud_open` | activity | ECX: loads the interface document 'hud_btnmash' and creates the data item 'button_mashing_minigame' (confirmed) |
| 0x005E4A90 | `button_mashing_hud_is_open` | activity | returns 1 when the vint data item 'button_mashing_minigame' exists (confirmed) |
| 0x005E4AB0 | `button_mashing_hud_fade` | activity | stdcall (bool show): five hud_element_fade calls (inferred) |
| 0x005E4B90 | `button_mashing_hud_set_progress` | activity | ESI = button index (-1 hides): fades the HUD elements (0x5E4AB0) and sets the data item values (float progress, byte flag) (inferred) |
| 0x005E4C90 | `button_mashing_hud_close` | activity | frees the 'button_mashing_minigame' data item and unloads the hud_btnmash document (confirmed) |
| 0x005E4D20 | `activity_hud_world_cash_popup` | activity | creates the vint datagroup 'mayhem_local_player_world_cash' at a world position (gfx_world_to_screen) for floating cash text (confirmed) |
| 0x005E50F0 | `activity_hud_marker_find` | activity | ESI = marker handle: returns the marker record (stride 0x104) in 0x279C230 (count 0x279C238) whose +8 equals ESI, else 0 (confirmed) |
| 0x005E5130 | `activity_hud_marker_index` | activity | ESI = marker handle: returns the index in 0x279C230 or -1 (confirmed) |
| 0x005E5460 | `activity_hud_markers_update` | activity | per-frame (only caller hud_do_frame): walks the marker table 0x279C230, drops dead owners, projects world positions to the screen (gfx_world_to_screen (inferred) |
| 0x005E6410 | `activity_hud_marker_free` | activity | EAX = marker handle: frees the vint data item (0x31A96F0 table, stride 0x22C) of the marker and removes its slot (0x5E6A80) (confirmed) |
| 0x005E6A80 | `activity_hud_marker_remove_at` | activity | EAX = index: copies the last marker over entry EAX (rep movsd 0x41 dwords) and decrements the count 0x279C238 (confirmed) |
| 0x005E6DF0 | `activity_hud_markers_apply` | activity | only caller hud_do_frame: five vint_dataitem_set_value / vint_value_free pairs per marker (inferred) |
| 0x005E9780 | `demo_derby_hud_stats` | activity | strings ACT_DEMO_DERBY_NITROUS / DAMAGE_DONE / TEAM_DAMAGE_DONE, DEMOLITION DERBY (inferred) |
| 0x005EA570 | `demo_derby_waiting_on_player_text` | activity | string ACT_DEMO_DERBY_WAITING_ON_PLAYER (inferred) |
| 0x005EB120 | `demo_derby_objective_text` | activity | strings ACT_DEMO_DERBY_OBJECTIVE / HOW_TO_PLAY_TEXT (inferred) |
| 0x005F06C0 | `demo_derby_sp_count_special_derbys` | activity | xtbl_load of demoderby_sp.xtbl, counts the Special_Derbys / Special_Derby children (xml_count_children) (confirmed) |
| 0x005F0870 | `demo_derby_sp_find_special_derby` | activity | looks a Special_Derby of demoderby_sp.xtbl up by name hash (string_hash_lower) (inferred) |
| 0x005F4E70 | `ambulance_diversion_show_completed` | diversion | strings DIVERSION_AMBULANCE_COMPLETED_FORMAT + SYS_HUD_DIVERSION_COMPLETE (inferred) |
| 0x005F5B80 | `ambulance_diversion_show_not_enough_victims` | diversion | string DIVERSION_AMBULANCE_FAILED_NOT_ENOUGH_VICTIMS (inferred) |
| 0x005F6680 | `ambulance_diversion_victim_saved` | diversion | 2581 bytes (inferred) |
| 0x005F8100 | `barnstorming_diversion_found_message` | diversion | strings DIVERSION_BARNSTORMING_FOUND / DIVERSION_BARNSTORMING (inferred) |
| 0x00601710 | `combat_tricks_award_text` | diversion | strings DIVERSION_COMBAT_TRICKS_GANG_KILL / MP_PLAYER_KILL / CAR_DESTROYED / ONE_HIT_KILL (inferred) |
| 0x006025C0 | `xtbl_load_combat_tricks` | diversion | stdcall ret 8: parses the Combat_Tricks table (Combat_Tricks_Duration, Bonus, Duration, Multi_Kill_Per_Person_Bonus) (confirmed) |
| 0x00603310 | `game_mode_diversion_init` | diversion | game mode 2 callback 0: set_mode_enabled(0) on every diversion type (0x140B168..0x140B1FC), then re-enables for mode 3 (freeroam) and walks the divers (confirmed) |
| 0x00603570 | `game_mode_diversion_reset` | diversion | game mode 2 callback 1: calls type vf1 on every enabled diversion type (confirmed) |
| 0x006035A0 | `game_mode_diversion_enter` | diversion | game mode 2 callback 2 (enter): starts the offered diversion 0xE97C10 through type start_instance (+0x1C) and stores the instance in 0x2527888 (confirmed) |
| 0x00603610 | `game_mode_diversion_frame` | diversion | game mode 2 callback 4 (frame): runs the current instance update (vf2) and switches on the status 0..3 (jump table 0x6037EC): pays the reward (net_sen (confirmed) |
| 0x00603800 | `game_mode_diversion_time_step` | diversion | game mode 2 callback 6, cdecl (float dt): instance on_time_step (+0xC) on 0x2527888 (confirmed) |
| 0x00603830 | `game_mode_diversion_leave` | diversion | game mode 2 callback 3 (leave): instance vf2 status then vf1 (stop), clears 0x2527888, ends the diversion UI (0x55F330) unless the status was 3 / 0xC  (confirmed) |
| 0x00603C20 | `game_mode_diversion_on_human_killed` | diversion | game mode 2 callback 10: current instance on_human_killed (+0x14), 5 args (confirmed) |
| 0x00603C60 | `game_mode_diversion_on_object_killed` | diversion | game mode 2 callback 11: current instance +0x18, 4 args (confirmed) |
| 0x00603C90 | `game_mode_diversion_on_vehicle_destroyed` | diversion | game mode 2 callback 12: current instance +0x1C, 4 args (confirmed) |
| 0x00603CC0 | `game_mode_diversion_on_game_event_13` | diversion | game mode 2 callback 13: current instance +0x20, 5 args (confirmed) |
| 0x00603D00 | `game_mode_diversion_on_object_damaged` | diversion | game mode 2 callback 14: current instance +0x24, 4 args (confirmed) |
| 0x00603D30 | `game_mode_diversion_on_game_event_15` | diversion | game mode 2 callback 15: current instance +0x28, 5 args (confirmed) |
| 0x00603D70 | `game_mode_diversion_on_player_respawned` | diversion | game mode 2 callback 17: current instance +0x2C (confirmed) |
| 0x00603D90 | `game_mode_diversion_on_game_event_20` | diversion | game mode 2 callback 20: current instance +0x30 (confirmed) |
| 0x00603DB0 | `game_mode_diversion_on_game_event_21` | diversion | game mode 2 callback 21: current instance +0x34 (confirmed) |
| 0x00603DD0 | `game_mode_diversion_on_game_event_19` | diversion | game mode 2 callback 19: current instance +0x3C, 4 args (confirmed) |
| 0x00603E00 | `game_mode_diversion_on_game_event_22` | diversion | game mode 2 callback 22: current instance +0x38 (confirmed) |
| 0x00603E30 | `diversion_current_type_id` | diversion | returns the type id (type vtable +8 get_type) of the current diversion instance 0x2527888, or -1 (confirmed) |
| 0x00603F80 | `diversions_locked_by_tss02` | diversion | returns 1 while diversions are locked: not in the MP-host case, 0x2527891 clear and the mission tss02 record missing or without the completed bit (bit (inferred) |
| 0x00604030 | `diversion_mdd_hud_show` | diversion | 1284 bytes, cdecl (bool, bool): builds / frees the diversion mission display (strings persistent_gsi, hit_mdd, chop_mdd, ACT_HITMAN_NAME, ACT_CHOP_SHO (inferred) |
| 0x006045E0 | `diversion_start_blocked` | diversion | returns 1 when no new diversion may be offered: an instance is running (0x2527888), game state 0x27A3230 in 1..5, 0x27A52A4 set or 0xEA1020 == 2 (confirmed) |
| 0x006049B0 | `vehicle_surfing_parse_balance_bar_params` | diversion | stdcall ret 4: reads Balance_Bar_Params (Height_Percent, Aspect_Ratio, X/Y_Location_Percent, Swing_Bar_Width_Percent) (confirmed) |
| 0x00605380 | `diversion_award_cash_and_respect` | diversion | ECX = dollars (-1 = none), stack arg: cash_from_dollars, net_send_prop_human_cash_adjust, stat_award_check x4 and the respect award (inferred) |
| 0x00605AD0 | `diversion_show_reward_unlocked` | diversion | strings SYS_HUD_CONF_REWARD / REWARD_UNLOCKED: plays the reward sound and shows hud_diversion_complete_message (confirmed) |
| 0x00607EC0 | `driveby_diversion_reward_names` | diversion | strings DIVERSION_DRIVEBY_DIVERSION_GOLD/SILVER/BRONZE_LEVEL_REWARD_NAME + PARAMETERIZED_REWARD_STRING (inferred) |
| 0x00608360 | `driveby_diversion_hud_update` | diversion | strings DIVERSION_DRIVEBY_VEHICLE_HEALTH_WARNING, ..._DRIVE_BY_GANG_MEMBERS_STRING, ..._ONLY_KILLS_FROM_CAR_COUNT (inferred) |
| 0x00608F00 | `driveby_diversion_instance::on_time_step` | class | diversion_instance slot 3 of driveby_diversion_instance: ret 4 (empty) (inferred) |
| 0x0060ADE0 | `exploration_diversion_jump_found` | diversion | strings DIVERSION_EXPLORATION_JUMP, DIVERSION_STUNT_JUMPING_TITLE (inferred) |
| 0x0060B6D0 | `exploration_diversion_hood_found` | diversion | strings DIVERSION_EXPLORATION_HOOD, SYS_HUD_EXPLORE (inferred) |
| 0x0060D6A0 | `firetruck_diversion_level_complete` | diversion | strings DIVERSION_FIRETRUCK_LEVEL_COMPLETE_FORMAT, SYS_HUD_DIVERSION_COMPLETE (inferred) |
| 0x0060F6C0 | `firetruck_diversion_check_timeouts` | diversion | strings DIVERSION_FIRETRUCK_RETURN_TO_VEHICLE / FAILED_VEHICLE_TIMEOUT / RETURN_TO_FIRES / FAILED_GENERAL_TIMEOUT (inferred) |
| 0x006153E0 | `theft_shop_table_load` | diversion | builds theft_%s%s.xtbl, parses Shop / Name (confirmed) |
| 0x00617310 | `back_seat_driver_diversion_score` | diversion | strings BACK SEAT DRIVER / DIVERSION_BACK_SEAT_DRIVER (inferred) |
| 0x00618840 | `stunt_jumping_diversion_score` | diversion | strings BIG AIR / DIVERSION_STUNT_JUMPING (inferred) |
| 0x00619D70 | `drifting_diversion_score` | diversion | strings POWERSLIDE / DIVERSION_DRIFTING (inferred) |
| 0x0061A810 | `hydraulics_diversion_score` | diversion | strings HYDRAULICS / DIVERSION_HYDRAULICS_STYLE_1 (inferred) |
| 0x0061C040 | `low_flying_diversion_score` | diversion | strings ##LOW FLYING / DIVERSION_LOW_FLYING (inferred) |
| 0x0061D450 | `oncoming_traffic_diversion_score` | diversion | strings WRONG WAY / DIVERSION_ONCOMING (inferred) |
| 0x00620590 | `survival_diversion_location_tag` | diversion | strings DIVERSION_SURVIVAL_FIRST_LOCATION_TAG / NEXT_LOCATION_TAG (inferred) |
| 0x00627E60 | `taxi_table_parse` | diversion | file_resolve + xtbl_load of taxi.xtbl: section Taxi_Table / Levels / Level (confirmed) |
| 0x00629980 | `taxi_diversion_new_customer` | diversion | strings DIVERSION_TAXI_RIVAL_TAXI / DIVERSION_TAXI_NEW_CUSTOMER (inferred) |
| 0x00629E30 | `taxi_diversion_customer_enter` | diversion | string DIVERSION_TAXI_CUSTOMER_ENTER (inferred) |
| 0x0062A1C0 | `taxi_diversion_robber_customer` | diversion | strings DIVERSION_TAXI_ROBBER_CUSTOMER / ROBBER_FAIL (inferred) |
| 0x0062A640 | `taxi_diversion_bank_robber_customer` | diversion | strings DIVERSION_TAXI_BANK_ROBBER_CUSTOMER / SLOW_CUSTOMER / FAST_CUSTOMER / FAST_PENALTY (inferred) |
| 0x0062ABA0 | `taxi_diversion_customer_exit` | diversion | strings DIVERSION_TAXI_CUSTOMER_EXIT + SYS_HUD_DIVERSION_COMPLETE (inferred) |
| 0x0062E0B0 | `tow_truck_diversion_hurt_vehicle_text` | diversion | string DIVERSION_TOWTRUCK_DIVERSION_HURT_VEHICLE_TEXT (inferred) |
| 0x0062E7C0 | `tow_truck_diversion_type_info::can_start` | class | diversion_type_info slot 5 override of tow_truck_diversion_type_info (159 bytes) (inferred) |
| 0x0062EFB0 | `tow_truck_diversion_repo_complete` | diversion | strings DIVERSION_TOWTRUCK_DIVERSION_REPO_DESTROYED_CARS, SYS_HUD_DIVERSION_COMPLETE (inferred) |
| 0x0062F320 | `tow_truck_diversion_tow_help` | diversion | 3429 bytes (inferred) |
| 0x006304D0 | `tow_truck_diversion_drop_off_check` | diversion | strings DIVERSION_TOW_TRUCK_VEHICLE_IN_PLACE_TEXT / WRONG_VEHICLE_IN_PLACE_TEXT / WRONG_VEHICLE_DROPOFF_TEXT (inferred) |
| 0x006326C0 | `tow_truck_diversion_hitch_help` | diversion | strings DIVERSION_TOW_TRUCK_START_HITCHING_HELP_TEXT / HOLD_HITCHING_BUTTONS_HELP_TEXT (inferred) |
| 0x00633750 | `vehicle_surfing_balance_message` | diversion | strings DIVERSION_VEHICLE_SURFING_STYLE_TWO_BALANCE_MESSAGE_TEXT / HANDSTAND_USE_TEXT (inferred) |
| 0x00633BB0 | `vehicle_surfing_best_time_text` | diversion | strings DIVERSION_BEST_VEHICLE_SURF_TIME_SECONDS / HANDSTAND_TIME_SECONDS (inferred) |
| 0x006352E0 | `poker_payouts_parse` | diversion | xtbl_load of poker_payouts.xtbl: Poker_Payouts / Hands / Hand (xtbl_get_int) (confirmed) |
| 0x00639CB0 | `whoring_diversion_patience_update` | diversion | strings DIVERSION_WHORING_TIME_UP_TEXT / DIVERSION_WHORING_SUCCESS (inferred) |
| 0x00640000 | `hitman_table_load_for_mission` | diversion | file_resolve + xtbl_load of hitman_%s.xtbl, section Hitman (confirmed) |
| 0x00649A30 | `xtbl_load_fraud_globals_foley` | activity | fraud_globals.xtbl Fraud_Values: Foley_Timeout_Min / Foley_Timeout_Max (xtbl_get_float) (confirmed) |
| 0x00649B20 | `xtbl_load_fraud_globals_adrenaline` | activity | fraud_globals.xtbl Fraud_Values: Adrenaline_Popup, Adrenaline_Speed_Damage (xtbl_get_float) (confirmed) |
| 0x0064C0A0 | `fraud_hud_current_hit` | activity | strings ACT_FRAUD_CURRENT_HIT, ACT_FRAUD_ADRENALINE_TAG, sr2_local_player_fraud_bonus (inferred) |
| 0x0064E920 | `fraud_visit_bonus_area` | activity | strings ACT_FRAUD_VISIT_BONUS_AREA, ACT_FRAUD_FIRST_VISIT_BONUS_STRING (inferred) |
| 0x0064F9F0 | `fraud_objective_text` | activity | strings ACT_FRAUD_GO_SCORING_AREA / ACT_FRAUD_GOAL / ACT_FRAUD_OBJECTIVE / ACT_FRAUD_GO_GET_HURT (inferred) |
| 0x00656BE0 | `pimping_instance::load_from_table` | class | activity_instance slot 1 override of pimping_instance (143 bytes) (inferred) |
| 0x00657270 | `pimping_rescue_trap_messages` | activity | strings ACT_PIMPING_RESCUE_KILL_COPS / KILL_PIMPS / TRAP_ATTACK_KILL_PIMPS (inferred) |
| 0x00657880 | `pimping_need_more_hos_check` | activity | string ACT_PIMPING_YOU_NEED_MORE_HOS (inferred) |
| 0x00659F80 | `pimping_limo_chase_update` | activity | strings ACT_PIMPING_STOP_THE_LIMO, ACT_PIMPING_GET_CLOSER_TO_HOS, RESCUE_COPS_REROUTE (inferred) |
| 0x0065ABA0 | `pimping_return_to_brothel` | activity | string ACT_PIMPING_RETURN_TO_BROTHEL (inferred) |
| 0x0065FFB0 | `trafficking_protect_driver` | activity | strings ACT_TRAFFIC_PROTECT_DRIVER / ACT_TRAFFIC_PROTECT_M, weapon beretta (inferred) |
| 0x00660290 | `trafficking_chase_helicopter` | activity | strings ACT_TRAFFIC_CHASE_HELICOPTER(_NOTORIETY / _KILLED) (inferred) |
| 0x00661600 | `trafficking_dealing_protect` | activity | strings ACT_TRAFFIC_DEALING_PROTECT / ACT_TRAFFIC_BUYER_DEAD (inferred) |
| 0x00661BA0 | `trafficking_buyer_flee` | activity | 3377 bytes (inferred) |
| 0x00668030 | `vandalism_bonus_messages` | activity | strings ACT_VANDAL_BONUS_MULTI_VEHICLE / STATIC_VEHICLE / MULTI_WEAPON / STATIC_WEAPON (inferred) |
| 0x0066A4E0 | `vandalism_bonus_award_messages` | activity | strings ACT_VANDAL_VEHICLE_BONUS_MSG / TEAM_BONUS_MSG / WEAPON_BONUS_MSG (inferred) |
| 0x0066BF60 | `vandalism_start_messages` | activity | strings ACT_VANDAL_START_2_MSG / START_3_MSG (inferred) |
| 0x0066C8C0 | `vandalism_property_message` | activity | string ACT_VANDAL_PROPERTY (inferred) |
| 0x0066F250 | `button_mashing_complication::vf12` | class | complication slot 12 override of button_mashing_complication (109 bytes) (inferred) |
| 0x006740A0 | `escort_level_instance::update` | class | escort level instance vtable slot 3 (update): 7972 bytes, 19-state escort state machine on [this+0x24] (inferred) |
| 0x006829B0 | `game_mode_racing_init` | activity | game mode 5 callback 0: resets the racing state (0xEAA860 = -1, 0x2527B50 = 0, 0x27A64E4 / 0x27A64EC = [0x33D630C], 0x2527B4E = 0) (confirmed) |
| 0x006829E0 | `game_mode_racing_enter` | activity | game mode 5 callback 2: empty (ret) (confirmed) |
| 0x006829F0 | `game_mode_racing_leave` | activity | game mode 5 callback 3: same reset as game_mode_racing_init without the 0x27A64EC store (confirmed) |
| 0x00682A20 | `game_mode_racing_frame` | activity | game mode 5 callback 4 (frame): racing state machine on 0x2527B50 (1 / 4), uses the race record lookup 0x680FF0 and the game-time window 0xEAA860 (0x3 (inferred) |
| 0x006A1440 | `mission_checkpoint_save` | missions | in-memory game save for the 'mission start' checkpoint (gamesave_save) (confirmed) |
| 0x006A37B0 | `mission_start_clear_world_state` | missions | callers net_recv_scrpt_start_game and misc_packet 0x07: trigger_deactivate loop, hud_smoked_busted_message, then the diversion shutdown sequence (0x5B (inferred) |
| 0x006A3DE0 | `mission_start_message_text` | missions | MSN_SOS/BRO/RN/TSS/UL_START and STRONGHOLD_START_MESSAGE_SOS lstr_get texts (confirmed) |
| 0x006A3F40 | `mission_checkpoint_load_by_name` | missions | strings 'mission restart' / 'mission checkpoint' / 'mission start' / tss01: picks the checkpoint by name and calls xtbl_load__missions helpers (inferred) |
| 0x006A4230 | `mission_abort_cleanup` | missions | only caller gameplay_exit: act_packet_send_mission_failed, missions_abort_all, script_heap_free x5, 10 virtual calls (inferred) |
| 0x006A43F0 | `mission_enter_common` | missions | shared enter path of game mode 0 and 4 (callers 0x53B492, 0x6A49B2): conversation_stop, ai_order_k23_start, human_state_set, resets mission globals (confirmed) |
| 0x006A49B0 | `game_mode_mission_enter` | missions | game mode 4 callback 2 (enter): mission_enter_common (0x6A43F0), then creates the mission script thread (script_thread_create + push_string) (confirmed) |
| 0x006A4D10 | `game_mode_mission_leave` | missions | game mode 4 callback 3 (leave): 3041 bytes of mission teardown (script thread stop, vehicles, co-op partner, 13 virtual calls) (inferred) |
| 0x006A5900 | `game_mode_mission_on_human_killed` | missions | game mode 4 callback 10: for a player human / valid victim pushes script_object_name strings (killer, victim, weapon class from table 0xDD0BD0 or 'Non (confirmed) |
| 0x006A5980 | `game_mode_mission_on_object_killed` | missions | game mode 4 callback 11: same script-thread push as on_human_killed (2 strings) (inferred) |
| 0x006A59E0 | `game_mode_mission_on_vehicle_destroyed` | missions | game mode 4 callback 12: same script-thread push (3 strings incl. 'None') (inferred) |
| 0x006A6480 | `missions_resolve_display_strings` | missions | strings MSN_%s_DISPLAY_NAME / MSN_%s_STORY (inferred) |
| 0x006A8190 | `mission_failure_reason_text` | missions | MISSION_FAILURE_PARTNER_BUSTED / BUSTED_TEXT / PARTNER_DIED / DIED_TEXT (confirmed) |
| 0x006A8960 | `mission_partner_response_text` | missions | MISSION_PARTNER_DECLINED / BUSY / REPSECT (confirmed) |
| 0x006A8CC0 | `mission_partner_disconnected_text` | missions | MISSION_PARTNER_DISCONNECTED (confirmed) |
| 0x006A9310 | `coop_metagame_show_result` | missions | strings METAGAME_SUCCESS_WIN / LOSE / TIE: shows the co-op meta game result and pays cash (cash_from_dollars, net_send_prop_human_cash_adjust) (inferred) |
| 0x006A9640 | `coop_gsi_show_score` | missions | DIVERSION_COOP_GSI_MY_SCORE / OTHER_ALT_SCORE lines (inferred) |
| 0x006A9B40 | `coop_metagame_kill_bonus_text` | missions | strings METAGAME_HEADSHOT_KILL / NUTSHOT_KILL / MELEE_KILL / EXPLOSION_KILL / QUICK_KILL (inferred) |

## Renames (`renames_activities.csv`)

249 renames of `Class::vfN` rows to the slot roles above; per-row notes and confidence are in the CSV.

## Ten large port maps

Selection: the ten largest functions in the area that have a single entry, one final `ret` and no `jmp` into other code (true sizes from the corrected measurement). The list, largest first:

| # | Address | Name | Size | Shape |
|---|---|---|---|---|
| 1 | 0x599F80 | `security_instance::update` (was vf7) | 16227 | lazy XML load of the security event table, then tail into `activity_instance::update` |
| 2 | 0x6740A0 | `escort_level_instance::update` | 7972 | 19-state escort state machine, x87-heavy |
| 3 | 0x5AC6D0 | `security_level_instance::load_from_table` | 5936 | xtbl loader plus level setup (weapons, teleport) |
| 4 | 0x580A40 | `vehicle_protect_objective_update_body` | 5170 | 5-phase objective body, 82 virtual calls |
| 5 | 0x5CC540 | `fuzz_crime_hooker_revolt::update` | 4797 | three unrolled participant AI blocks, then `fuzz_crime::update` |
| 6 | 0x5934B0 | `piracy_level_instance::load_from_table` | 4794 | xtbl loader, helicopter circle maths (x87) |
| 7 | 0x585C60 | `large_convoy_objective_update_body` | 4628 | 12-case jump table + byte table |
| 8 | 0x6A0080 | `mission_restart_from_checkpoint` | 4508 | checkpoint restore, 10-way weapon-kind jump table |
| 9 | 0x64C410 | `fraud_level_instance::load_from_table` | 4267 | short loader plus two weapon-restore copies |
| 10 | 0x5735A0 | `guardian_angel_state_step` | 4185 | 48-state jump table |

Alternates if one is dropped: 0x5DA7E0 `fight_club_activity_instance::load_from_table` (4108), 0x58A160 liberator objective body (4030), 0x6218F0 `tagging_diversion_type_info::update` (3778, x87 heavy), 0x6A4D10 `game_mode_mission_leave` (3041).

Portability: all ten are plain code (no inline assembly, no SEH, no self-modification). The porting hazards are the register-argument helpers (`xtbl_get_*`, `coop_partner_human`, ...), the x87 compare idioms and the `/GS` cookie frames. Callers of the ports are vtables or game mode tables, so the hook point is a vtable slot / table entry (write the Rust function's address into the slot) rather than a `call` patch, except 0x6A0080, 0x580A40, 0x585C60, 0x5735A0 which have direct callers.

Reliability: the addresses, callee tables, jump-table decodes, constants and compare polarity below are generated from the bytes and are exact. The section prose is a reading of the disassembly; items marked *inferred* or with a hedge ("looks like", "by use") are semantic guesses and must be checked against `python tools/x86dis.py` before porting.

How to read the generated tables below. `CALLEES`: `callee reads` is the register set read before written on any path of the callee (a register argument), `ret N` is a stdcall callee-pop, `cleanup [N]` is the caller's `add esp, N` after the call (cdecl stack args). `GLOBALS`: reads / writes counted per absolute address. `X87 COMPARES`: for every `fcom*` the `test ah,mask; jcc` that follows and which relations take the branch (ST0 is the stack top at the compare; `fcompp` compares ST0 with ST1). `SIGNED / UNSIGNED INTEGER COMPARES`: text is the condition under which the branch is taken. Equality tests are not listed there (they are in the section text when it matters).

x87 polarity cheat sheet (verified against every site below): after `fcom` the flags are C3 C2 C0 = ST0>op: 000, ST0<op: 001, equal: 100, unordered: 111. `test ah,0x41; jne` is taken for ST0<=op or unordered; `je` for ST0>op. `test ah,0x5; jp` is taken for ST0>=op or unordered; `jnp` for ST0<op. `test ah,0x44; jnp` is taken only for equal; `jp` for not-equal or unordered. `test ah,1; jne` is taken for ST0<op or unordered; `je` for ST0>=op.


### 1. 0x599F80 `security_instance::update` (16227 bytes)

Convention: thiscall, ECX = `security_instance*` (kept in EBX), no stack arguments, plain `ret`. Frame `sub esp,0x3A4` with a /GS cookie. It is `activity_instance` vtable slot 7 for the security activity.

Object fields used: `[this+0x40]` byte "events loaded"; `[this+0x11731]` the xtbl file name buffer (an inline string, not a pointer: `lea eax,[ebx+0x11731]`); `[this+0x111AC]` event count (also read as the loop bound); event records of 0x16C8 bytes start at `this+0x?` (the stores use `imul ecx,ecx,0x16C8` then `[ecx+ebx+disp]` with disp 0x4C, 0x54, 0x58, 0x5C ...); persona counters `[this+idx*8+0x111C4]` (2 genders x 4 races).

Sections:
1. 0x599F80-0x599FB9 prologue; `if [this+0x40] != 0 goto 0x59DEC7` (already loaded, skip to tail).
2. 0x599FB9 `xtbl_load(EAX = &this[0x11731], ECX = 0)` returns the root; walks `root.children` (`[node+8]` first child, `[node+4]` next sibling, `[node+0]` name) with `__stricmp(name, "Activity")`, then child `"Events"`, then each child `"Event"`. A missing node falls through to the persona check with an empty table. Compare results are tested as `eax == 0`.
3. 0x59A071-0x59A2F0 per event: `xml_find_child(EAX = node, EDI = "Event_Name")` -> `string_crc32` -> `[rec+0x4C]`; `Event_Group` crc -> `[rec+0x54]`; `Vehicle_Stop`, `Celebrity_Start`, `Player_Start_1`, `Player_Start_2`, `Player_End_Warp`: crc -> `navpoint_find` -> `[rec+0x58 / 0x5C / ...]`; `Leash_To_Celeb_Dist` (xtbl_get_float -> `[rec+0x94]`), `Hour` (xtbl_get_int -> `[rec+0x50]`), `Crowd_Ambience_Origin`, `Follower_NPC_Type` (`string_hash_lower`).
4. 0x59A309-0x59A860 `Critical_Objects` / `Critical_Object` (`Chunk_Name` via `world_chunk_find_id`, `Object_Name`, bool `Visible_For_Activity` -> `[x+8]`), `Critical_Effects` / `Critical_Effect` (`Effect_Name`, `effect_find_by_name`), `Additional_Sounds` / `Additional_Sound` (`Navpoint`, `Sound`).
5. 0x59A862-0x59ABC9 `Heli_Arrival` (`Heli`, `Driver_Type`, `Path_Navpoints` / `Path_Navpoint`), `Celebrity_Stops` / `Celebrity_Stop` (`Stop_Navpoint`, `Stop_Type`).
6. 0x59ACD2-0x59B6D2 `Enter` / `Exit` sub-tables: `State`, `Duration` (float seconds multiplied by the f64 constant 1000.0 at 0xE5C0D8, stored as an int millisecond value after `_ftol2`-style conversion at 0x59AE76), `Interaction_NPC`, `Possible_Animations` / `Possible_Animation` (random pick through the RNG ring 0xEC3ABC / 0xEC3AC0, modulo the count), `Interaction_Animation_Pairs` / `Animation_Pair` (`Name`, `Sync`, `Celebrity_Action`, `NPC_Action`, `Celebrity_Offset` = three consecutive `xtbl_get_float` into stack slots 0x68 / 0x70 / 0x78), `Scripted_Lines` / `Scripted_Line` (`Celebrity_Persona_Situation`, `Celebrity_Voice_Variant`, `NPC_Persona_Situation`, `NPC_Voice_Variant`).
7. 0x59B6D2-0x59BBC4 `Possible_Actions` / `Possible_Action` and `Celebrity_Actions` with `Action` names matched by a `__stricmp` chain; the matching index drives the 19-entry jump table below.
8. 0x59BBC4-0x59BE6A `Capture_Triggers` / `Capture_Trigger` (`Trigger_Name`, `Annoyance_Refund` int -> `[rec+0x164]`, `Cash` int -> `[rec+0x168]`, bool `Disable_Capture` -> `[rec+0x161]`, `Arrow_Location`, `Parent_Object`).
9. 0x59BE6A-0x59C4C2 `Actions` (`Type` selects a branch, `Position_Type`, `Orient_Type`, `Vehicle` via `script_vehicle_find`, `Wait_Time` float, `Blood_Effect_Navpoint`, `APC`, `Firing_Time`, `Reward` via `xtbl_get_int_opt`).
10. 0x59C5AA-0x59CA33 `Capture_Throws`, `Police_Throw_Overrides` (`Cop` via `script_object_find`, `Throw_Chance`, `Throw_Velocity`, `Dont_Run`, `Forced_Throw_Navpoint`), `Suck_Triggers`, `Oneshot_Sound`.
11. 0x59CAC2-0x59D3A9 `Points` / `Point` (`Acceleration` float), `Presence_Triggers`, `Additional_Limo_Path` / `_Node` (`Limo_Navpoint`, bool `Limo_Offroad` -> `[x+0x1084]`, bool `Limo_Dropoff` -> `[x+0x1085]`), `Fan_Spawn_Areas` (`Fans_Count` int, `Spawn_Edge_Vertices`).
12. 0x59D222-0x59D689 `Special_Movers` / `Special_Mover` (`Mover_Prefix` string copy with `_strncpy`, `Uses`, `Value`), `Train_Spawns` (`Start`, `End`).
13. 0x59D689-0x59DA3A `Ally_Groups` / `Ally_Group`, `NPCs` / `NPC`, `Beatdown_Reward` (int opt), `Equipped_When_Idle`, `Animation_State`.
14. 0x59DA3A-0x59DB02 five base-value ints: `Base_Melee_Annoyance_Refund` `[+0x98]`, `Base_Mover_Annoyance_Refund` `[+0x9C]`, `Base_Melee_Cash` `[+0xA0]`, `Base_Weapon_Cash` `[+0xA4]`, `Base_Mover_Cash` `[+0xA8]`.
15. 0x59DB02-0x59DE0E `Valid_Celebrity_Personas` / `Valid_Fan_Personas`: for gender 0..1 and race 0..3 formats `"Valid_%s_Persona"` (`_sprintf`), counts `Persona` children by `Race` / `Gender`, and prints `"Crowd Control: Too many valid %s personas specified for '%s %s'!"` when the count exceeds the limit.
16. 0x59DE22-0x59DEA4 consistency check: for each of 2 genders x 4 races requires `[this + idx*8 + 0x111C4] > 0` (signed `jg`) else `_sprintf` of the "At least 1 race/gender doesn't have a valid Celebrity / Fan persona" messages (debug text only).
17. 0x59DEA6-0x59DEC7 `[0xE87138]->vtable[0x34/4](EAX = [0xE87134])` through `call edx`, `[0xE87134] = -1`, `[this+0x40] = 1`.
18. 0x59DEC7 tail: `ECX = this; call activity_instance::update (0x53E4D0)`; cookie check; `ret`.

Jump tables (decoded in the table below): `jmp [ecx*4+0x59DEE4]` at 0x59BFA9 has 19 entries (action kinds; entries 7, 8, 10 share 0x59D4BC and 12, 13 share 0x59D4A8); `jmp [ecx*4+0x59DEFC]` at 0x59D3A2 is the 13-entry tail of the same table (index minus 6); `jmp [ecx*4+0x59DF1C]` at 0x59DBF3 has 5 entries indexed through the byte table at 0x59DF30 (values `00 01 04 04 04 04 04 02 04 ... 03`: persona kind -> case).

Stack objects: the xtbl results are held in `[esp+0x30]` (current child), `[esp+0x3C]` (Activity node), `[esp+0x50]` (Events node); `string_crc32` takes its input in `[esp+0xa4...]` buffers built by `xml_find_child` (EAX = node, EDI = name literal, returns the text value pointer in EAX).

String compare idiom: the inline `jmp $+5; sbb eax,eax; sbb eax,-1` blocks (listed under FLAG IDIOMS) are the CRT string-compare sign normaliser (result -1 / 0 / +1); the callers only test the result for zero except where noted.

Port recipe: model the xtbl node as `{name: *const u8, next: *Node, first_child: *Node, text: *const u8}`; implement `children(node)`, `find_child(node, name)` and the `xtbl_get_*` helpers as thin wrappers over the original functions (they are register-ABI: ECX = child name, ESI = out ptr for int / float, stack = element; bool: ECX = name, stack = (out ptr, element)). The routine is long but has no data-dependent control flow beyond the name matches, so a table-driven port (one row per field: name, kind, offset) reproduces it; the field offsets are in the table below and in `loadermap` form: Leash_To_Celeb_Dist +0x94 (f32), Hour +0x50, Annoyance_Refund +0x164, Cash +0x168, Disable_Capture +0x161, Limo_Offroad +0x1084, Limo_Dropoff +0x1085, Beatdown_Reward +0x160C, Equipped_When_Idle +0x1610, Fans_Count +0x10EC, Throw_Chance +0x5AC, Acceleration +0x624 (stride 8), Value +0x12A4, Reward `[this+0x111AC]` target (the `int_opt` writes into the count slot).


Generated tables (exact, from the bytes):

```
FUNCTION 599F80..59DEE3 size 16227 (4862 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x3a4; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0xa4 (lea eax @ 59A087) passed to string_crc32
 esp+0xe8 (lea eax @ 59A0C3) passed to string_crc32
 esp+0xf0 (lea eax @ 59A0FF) passed to string_crc32
 esp+0xf8 (lea eax @ 59A13E) passed to string_crc32
 esp+0xdc (lea eax @ 59A17D) passed to string_crc32
 esp+0x98 (lea eax @ 59A1BC) passed to string_crc32
 esp+0xfc (lea eax @ 59A20F) passed to string_crc32
 esp+0xac (lea eax @ 59A296) passed to string_crc32
 esp+0x78 (lea edx @ 59A2CE) passed to string_hash_lower
 esp+0xe0 (lea edx @ 59A413) passed to string_hash_lower
 esp+0xb0 (lea edx @ 59A5DB) passed to string_hash_lower
 esp+0x108 (lea eax @ 59A72F) passed to string_crc32
 esp+0xb8 (lea eax @ 59A78E) passed to string_hash_lower
 esp+0xec (lea eax @ 59A8BB) passed to string_crc32
 esp+0x88 (lea edx @ 59A929) passed to string_hash_lower
 esp+0xc4 (lea eax @ 59AA10) passed to string_crc32
 esp+0x104 (lea eax @ 59AB63) passed to string_crc32
 esp+0x78 (lea esi @ 59AE5D) passed to xtbl_get_float
 esp+0xcc (lea eax @ 59AEEB) passed to string_crc32
 esp+0x68 (lea esi @ 59B36F) passed to xtbl_get_float
 esp+0x70 (lea esi @ 59B37E) passed to xtbl_get_float
 esp+0x7c (lea eax @ 59B5AC) passed to string_hash_lower
 esp+0x7c (lea eax @ 59B5BA) passed to sub_6abce0
 esp+0x44 (lea esi @ 59B5E2) passed to xtbl_get_int
 esp+0x74 (lea ecx @ 59B648) passed to string_hash_lower
 esp+0x74 (lea eax @ 59B656) passed to sub_6abce0
 esp+0x60 (lea edx @ 59BAEC) passed to string_hash_lower
 esp+0x60 (lea eax @ 59BAFA) passed to sub_6abce0
 esp+0x58 (lea esi @ 59BB22) passed to xtbl_get_int
 esp+0xf4 (lea eax @ 59BC6B) passed to string_crc32
 esp+0xd4 (lea eax @ 59BD91) passed to string_crc32
 esp+0x100 (lea eax @ 59BE0D) passed to string_crc32
 esp+0x98 (lea eax @ 59BFFA) passed to string_hash_lower
 esp+0xa0 (lea eax @ 59C0EB) passed to string_crc32
 esp+0x30 (lea esi @ 59C2F3) passed to xtbl_get_float
 esp+0xa8 (lea eax @ 59C3AE) passed to string_crc32
 esp+0xb0 (lea eax @ 59C64B) passed to string_crc32
 esp+0x33 (lea eax @ 59C80E) passed to xtbl_get_bool
 esp+0xb8 (lea eax @ 59C89B) passed to string_crc32
 esp+0xbc (lea edx @ 59CA59) passed to string_hash_lower
 esp+0xc8 (lea eax @ 59CB61) passed to string_crc32
 esp+0xd0 (lea eax @ 59CF02) passed to string_crc32
 esp+0xd8 (lea eax @ 59D119) passed to string_crc32
 esp+0xe0 (lea eax @ 59D5AE) passed to string_crc32
 esp+0x84 (lea eax @ 59D5F4) passed to string_crc32
 esp+0x90 (lea eax @ 59D996) passed to string_crc32
 esp+0x10c (lea ecx @ 59DB7C) passed to _sprintf
 esp+0x108 (lea edi @ 59DB9C) passed to xml_find_child
 esp+0x134 (lea eax @ 59DDB5) passed to _sprintf
 esp+0x108 (lea ecx @ 59DDE9) passed to __stricmp
 esp+0x1b0 (lea ecx @ 59DE49) passed to _sprintf
 esp+0x2b0 (lea eax @ 59DE7C) passed to _sprintf
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 00B743F0 xtbl_load: x1 first 599FB9; callee reads eax,ecx; ret 0; cleanup -
 00C99030 __stricmp: x132 first 599FD8; callee reads -; ret 0; cleanup [8]
 00BFBEE0 xml_find_child: x32 first 59A078; callee reads eax,edi; ret 0; cleanup -
 00BDCA30 string_crc32: x26 first 59A08E; callee reads eax,edx; ret 0x4; cleanup -
 00A5EB50 navpoint_find: x20 first 59A10B; callee reads eax; ret 0; cleanup -
 00B74EF0 xtbl_get_float: x9 first 59A24D; callee reads ecx,esi; ret 0; cleanup [12]
 00B747D0 xtbl_get_int: x13 first 59A268; callee reads ecx,esi; ret 0; cleanup [4, 8]
 00BDCB30 string_hash_lower: x10 first 59A2D5; callee reads eax,ecx; ret 0x4; cleanup -
 004A5F60 sub_4a5f60: x2 first 59A2E1; callee reads -; ret 0x8; cleanup -
 00BFC110 xml_count_children: x2 first 59A336; callee reads eax; ret 0; cleanup [4]
 004A9840 world_chunk_find_id: x2 first 59A3D4; callee reads ecx; ret 0; cleanup -
 00B75050 xtbl_get_bool: x7 first 59A45D; callee reads ecx; ret 0; cleanup [12]
 006EF580 anim_state_find_by_name_b: x5 first 59ADAF; callee reads eax; ret 0; cleanup -
 006EF510 anim_state_find_by_name: x2 first 59AE39; callee reads eax; ret 0; cleanup -
 006ABCE0 sub_6abce0: x3 first 59B5BE; callee reads eax; ret 0; cleanup -
 0050DB60 effect_find_by_name: x2 first 59C092; callee reads ecx; ret 0; cleanup -
 00A70030 script_vehicle_find: x3 first 59C2C1; callee reads eax,ecx; ret 0; cleanup -
 00B74860 xtbl_get_int_opt: x2 first 59C4C7; callee reads ecx,esi; ret 0; cleanup [8]
 00A618A0 script_object_find: x2 first 59C7AA; callee reads eax,ecx; ret 0; cleanup -
 00B75610 sub_b75610: x1 first 59C809; callee reads ecx; ret 0; cleanup -
 00A6C870 trigger_find_by_name: x3 first 59C9DA; callee reads eax,ecx; ret 0; cleanup -
 00C990F0 _strncpy: x1 first 59D2C4; callee reads -; ret 0; cleanup [12]
 00CA0BC0 _memset: x2 first 59DB4C; callee reads -; ret 0; cleanup [24]
 00C968F6 _sprintf: x4 first 59DB8D; callee reads -; ret 0; cleanup [16, 20]
 006ADD50 sub_6add50: x1 first 59DD57; callee reads -; ret 0; cleanup -
 0053E4D0 activity_instance::update: x1 first 59DEC9; callee reads ecx; ret 0; cleanup -
 00C9604F __security_check_cookie: x1 first 59DEDA; callee reads ecx; ret 0; cleanup -
INDIRECT CALLS: 1 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E45544: r1 w0 first 59B388 addr-of sz4
 00E5C0D8: r3 w0 first 59AE76 fmul sz8
 00E80B50: r1 w0 first 599F8C mov sz4
 00E87134: r1 w1 first 59DEAE mov sz4
 00E87138: r1 w0 first 59DEA6 mov sz4
 00EC3ABC: r4 w8 first 59B006 mov sz4
 00EC3AC0: r4 w0 first 59B00C mov[idx] sz4
 00EE5558: r3 w0 first 59A7B3 mov[idx] sz4
 02526AF0: r3 w0 first 59A79F mov sz2
 02F8E88C: r1 w0 first 59BC77 mov sz4
 02F9B18C: r2 w0 first 59A8C7 mov sz4
STRINGS
 DE2E0C "Activity" @ 599FD2
 DE8330 "Events" @ 59A024
 DD8DFC "Event" @ 59A052
 DEA60C "Event_Name" @ 59A071
 DEB950 "Event_Group" @ 59A0AD
 DEB95C "Vehicle_Stop" @ 59A0E9
 DEB96C "Celebrity_Start" @ 59A128
 DEB97C "Player_Start_1" @ 59A167
 DEB98C "Player_Start_2" @ 59A1A6
 DEB99C "Player_End_Warp" @ 59A1F5
 DEB9AC "Leash_To_Celeb_Dist" @ 59A248
 DEB9C0 "Hour" @ 59A263
 DEB9C8 "Crowd_Ambience_Origin" @ 59A27C
 DEB9E0 "Follower_NPC_Type" @ 59A2B7
 DEB898 "Critical_Objects" @ 59A309
 DEB8AC "Critical_Object" @ 59A32F
 DE3668 "Chunk_Name" @ 59A3BD
 DEB8BC "Object_Name" @ 59A3FC
 DEB8C8 "Visible_For_Activity" @ 59A458
 DEB8E0 "Critical_Effects" @ 59A4D2
 DEB8F4 "Critical_Effect" @ 59A4F8
 DEB904 "Effect_Name" @ 59A5C4
 DEB9F4 "Additional_Sounds" @ 59A695
 DEBA08 "Additional_Sound" @ 59A6C5
 DE8584 "Navpoint" @ 59A706
 DE04B0 "Sound" @ 59A768
 DEBA1C "Heli_Arrival" @ 59A862
 DE8F38 "Heli" @ 59A892
 DEBA2C "Driver_Type" @ 59A903
 DEBA38 "Path_Navpoints" @ 59A972
 DEBA48 "Path_Navpoint" @ 59A9A6
 DEBA58 "Celebrity_Stops" @ 59AAB9
 DEBA68 "Celebrity_Stop" @ 59AB12
 DEBA78 "Stop_Navpoint" @ 59AB3A
 DEBA88 "Stop_Type" @ 59ABC9
 DEBBF4 "Enter" @ 59ACD2
 DEBBFC "Exit" @ 59AD22
 DE2D44 "State" @ 59AE12
 DDC3BC "Duration" @ 59AE58
 DEBB20 "Interaction_NPC" @ 59AEC2
 DEBB30 "Possible_Animations" @ 59AF78
 DEBB44 "Possible_Animation" @ 59AFB2
 DEBB58 "Animation_Pair" @ 59B092
 DEBB68 "Interaction_Animation_Pairs" @ 59B0D7
 DEBB84 "Interaction_Animation_Pair" @ 59B112
 DC5804 "Name" @ 59B152
 DEBBA0 "Sync" @ 59B1F3
 DEB708 "Celebrity_Action" @ 59B295
 DEBBA8 "NPC_Action" @ 59B2F2
 DEBBB4 "Celebrity_Offset" @ 59B34A
 DEBACC "Scripted_Lines" @ 59B482
 DEBADC "Scripted_Line" @ 59B4C2
 DEBAEC "Celebrity_Persona_Situation" @ 59B586
 DEBB08 "Celebrity_Voice_Variant" @ 59B5DD
 DEBBC8 "NPC_Persona_Situation" @ 59B622
 DEBBE0 "NPC_Voice_Variant" @ 59B675
 DEBA94 "Possible_Actions" @ 59B6D2
 DEBAA8 "Possible_Action" @ 59B706
 DE2D34 "Action" @ 59B7E2
 DEBAB8 "Celebrity_Actions" @ 59B827
 DEBC04 "Capture_Triggers" @ 59BBC4
 DEBC18 "Capture_Trigger" @ 59BC02
 DE3DAC "Trigger_Name" @ 59BC42
 DEBC28 "Annoyance_Refund" @ 59BCEA
 DDAB48 "Cash" @ 59BD0E
 DEBC3C "Disable_Capture" @ 59BD33
 DEBC4C "Arrow_Location" @ 59BD68
 DEBC5C "Parent_Object" @ 59BDE4
 DE2D2C "Actions" @ 59BE6A
 DD92D4 "Type" @ 59BEE2
 DEBC6C "Position_Type" @ 59C124
 DEBC7C "Orient_Type" @ 59C1E3
 DDAEC4 "Vehicle" @ 59C299
 DEBC88 "Wait_Time" @ 59C2EE
 DEBC94 "Blood_Effect_Navpoint" @ 59C37A
 DEBCAC "APC" @ 59C3E2
 DEBCB0 "Firing_Time" @ 59C439
 DE306C "Reward" @ 59C4C2
 DEBCBC "Capture_Throws" @ 59C5AA
 DEBCCC "Capture_Throw" @ 59C5DA
 DEBCDC "Throw_Navpoint" @ 59C622
 DEBCEC "Police_Throw_Overrides" @ 59C6F9
 DEBD04 "Police_Throw_Override" @ 59C732
 DEBD1C "Cop" @ 59C782
 DEBD20 "Throw_Chance" @ 59C7DE
 DEBD30 "Throw_Velocity" @ 59C802
 DEBD40 "Dont_Run" @ 59C814
 DEBD4C "Forced_Throw_Navpoint" @ 59C872
 DEBD64 "Suck_Triggers" @ 59C932
 DEBD74 "Suck_Trigger" @ 59C962
 DEBD84 "Oneshot_Sound" @ 59CA33
 DE6C24 "Points" @ 59CAC2
 DEBD94 "Point" @ 59CB02
 DEBD9C "Acceleration" @ 59CBB3
 DEBDAC "Presence_Triggers" @ 59CD22
 DEBDC0 "Presence_Trigger" @ 59CD56
 DEBDD4 "Presence_Trigger_Name" @ 59CD97
 DEBDEC "Additional_Limo_Path" @ 59CE62
 DEBE04 "Additional_Limo_Path_Node" @ 59CE96
 DEBE20 "Limo_Navpoint" @ 59CED3
 DEBE30 "Limo_Offroad" @ 59CF3F
 DEBE40 "Limo_Dropoff" @ 59CF60
 DEBE50 "Fan_Spawn_Areas" @ 59CFE3
 DEBE60 "Fan_Spawn_Area" @ 59D017
 DEBE70 "Fans_Count" @ 59D06F
 DEBE7C "Spawn_Edge_Vertices" @ 59D092
 DEBE90 "Spawn_Edge_Vertex" @ 59D0C6
 DEBEA4 "Special_Movers" @ 59D222
 DEBEB4 "Special_Mover" @ 59D252
 DEBEC4 "Mover_Prefix" @ 59D2A2
 DEBED4 "Uses" @ 59D3DD
 DDAB70 "Value" @ 59D4A8
 DEBEDC "Train_Spawns" @ 59D532
 DEBEEC "Train_Spawn" @ 59D562
 DD7A5C "Start" @ 59D598
 DEBEF8 "End" @ 59D5DE
 DEBEFC "Ally_Groups" @ 59D689
 DEBF08 "Ally_Group" @ 59D6B9
 DEBF14 "NPCs" @ 59D709
 DE5A64 "NPC" @ 59D739
 DEBF1C "Beatdown_Reward" @ 59D8CF
 DEBF2C "Equipped_When_Idle" @ 59D8F2
 DE41E8 "Animation_State" @ 59D907
 DEBF40 "Base_Melee_Annoyance_Refund" @ 59DA3A
 DEBF5C "Base_Mover_Annoyance_Refund" @ 59DA58
 DEBF78 "Base_Melee_Cash" @ 59DA76
 DEBF88 "Base_Weapon_Cash" @ 59DA94
 DEBF9C "Base_Mover_Cash" @ 59DAB2
 DEBFAC "Valid_Celebrity_Personas" @ 59DB02
 DEBFC8 "Valid_Fan_Personas" @ 59DB15
 DEBFDC "Celebrity" @ 59DB36
 DEBFE8 "Fan" @ 59DB41
 DEBFEC "Valid_%s_Persona" @ 59DB83
 DDAAE4 "Race" @ 59DBCA
 DDAADC "Gender" @ 59DC28
 DEC000 "Persona" @ 59DCD6
 DEC008 "Crowd Control: Too many valid %s personas specified for '%s %s'!" @ 59DDBC
 DEC050 "Crowd Control: At least 1 race/gender doesn't have a valid Celebrity persona! (See KevinH - first pair is %d/%d)" @ 59DE50
 DEC0C8 "Crowd Control: At least 1 race/gender doesn't have a valid Fan persona! (See KevinH - first pair is %d/%d)" @ 59DE83
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C0D8] fmul @ 59AE76: f64=1000.0 bits=0x408f400000000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 59B1A0 sbb eax, eax   (prev: jmp 0x59b1a5)
 59B1A2 sbb eax, -1   (prev: sbb eax, eax)
 59B250 sbb eax, eax   (prev: jmp 0x59b255)
 59B252 sbb eax, -1   (prev: sbb eax, eax)
 59B8E4 sbb eax, eax   (prev: jmp 0x59b8e9)
 59B8E6 sbb eax, -1   (prev: sbb eax, eax)
 59C1B0 sbb eax, eax   (prev: jmp 0x59c1b5)
 59C1B2 sbb eax, -1   (prev: sbb eax, eax)
 59C261 sbb eax, eax   (prev: jmp 0x59c266)
 59C263 sbb eax, -1   (prev: sbb eax, eax)
 59C832 sete cl   (prev: cmp byte ptr [esp + 0x2b], cl)
 59D480 sbb eax, eax   (prev: jmp 0x59d485)
 59D482 sbb eax, -1   (prev: sbb eax, eax)
 59D870 sbb eax, eax   (prev: jmp 0x59d875)
 59D872 sbb eax, -1   (prev: sbb eax, eax)
 59DCB0 sbb eax, eax   (prev: jmp 0x59dcb5)
 59DCB2 sbb eax, -1   (prev: sbb eax, eax)
JUMP TABLES
 jmp @ 59BFA9 table 59DEE4 entries(19): 59BFB0 59C4CF 59BFCB 59C061 59C290 59C3D9 59D3A9 59D4BC 59D4BC 59D3DD 59D4BC 59D3E7 59D4A8 59D4A8 59DBFA 59DC04 59DC0E 59DC18 59DC20
 jmp @ 59D3A2 table 59DEFC entries(13): 59D3A9 59D4BC 59D4BC 59D3DD 59D4BC 59D3E7 59D4A8 59D4A8 59DBFA 59DC04 59DC0E 59DC18 59DC20
 jmp @ 59DBF3 table 59DF1C entries(5): 59DBFA 59DC04 59DC0E 59DC18 59DC20 ; byte-index table at 59DF30
   byte table at 59DF30: 00 01 04 04 04 04 04 02 04 04 04 04 04 04 04 04 04 04 04 04 04 04 03 cc cc cc cc cc cc cc cc cc
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 59a340: eax <= (signed) 0 -> 0x59a4b3 [jle]
 59a355: edx >= (signed) 0x200 -> 0x59a4b3 [jge]
 59a509: eax <= (signed) edi -> 0x59a680 [jle]
 59a51e: edx >= (signed) 0x200 -> 0x59a680 [jge]
 59a6ef: dword ptr [esp + 0x14] >= (signed) 4 -> 0x59a82e [jge]
 59a7ab: dx <= (unsigned) 0 -> 0x59a7c6 [jbe]
 59a7c4: cx < (unsigned) dx -> 0x59a7b0 [jb]
 59a9d0: dword ptr [esp + 0x14] >= (signed) 5 -> 0x59aa89 [jge]
 59aaed: dword ptr [esp + 0x1c] >= (signed) 0x24 -> 0x59bb96 [jge]
 59ac71: eax < (signed) 4 -> 0x59ac09 [jl]
 59b022: ecx < (signed) 0x800 -> 0x59b02e [jl]
 59b036: edx <= (signed) 0 -> 0x59b078 [jle]
 59b25f: edx < (signed) 3 -> 0x59b220 [jl]
 59b4e4: dword ptr [esp + 0x10] <= (signed) edi -> 0x59bb53 [jle]
 59b506: ecx < (signed) 0x800 -> 0x59b50e [jl]
 59b545: eax <= (signed) 0 -> 0x59b579 [jle]
 59b5f7: eax > (unsigned) 0xfffe -> 0x59b613 [ja]
 59b68f: eax > (unsigned) 0xfffe -> 0x59bb53 [ja]
 59b77c: ecx < (signed) 0x800 -> 0x59b788 [jl]
 59b790: edx <= (signed) 0 -> 0x59b7cb [jle]
 59ba34: dword ptr [esp + 0x10] <= (signed) edi -> 0x59bb53 [jle]
 59ba56: ecx < (signed) 0x800 -> 0x59ba5e [jl]
 59ba8f: edi <= (signed) 0 -> 0x59bab9 [jle]
 59bb37: eax > (unsigned) 0xfffe -> 0x59bb53 [ja]
 59bc2a: dword ptr [esp + 0x10] >= (signed) 8 -> 0x59c57c [jge]
 59bece: dword ptr [esp + 0x14] >= (signed) 6 -> 0x59c522 [jge]
 59bfa3: esi > (unsigned) 5 -> 0x59c4cf [ja]
 59c017: dx <= (unsigned) 0 -> 0x59c036 [jbe]
 59c034: cx < (unsigned) dx -> 0x59c020 [jb]
 59c1d4: edi < (signed) 2 -> 0x59c180 [jl]
 59c289: edx < (signed) 3 -> 0x59c236 [jl]
 59c4dd: esi < (signed) 7 -> 0x59bf20 [jl]
 59c604: dword ptr [esp + 0x10] >= (signed) 8 -> 0x59c6c9 [jge]
 59c764: dword ptr [esp + 0x14] >= (signed) 0x70 -> 0x59c8ff [jge]
 59c99a: dword ptr [esp + 0x10] >= (signed) 0x4c -> 0x59cced [jge]
 59ca76: dx <= (unsigned) 0 -> 0x59ca96 [jbe]
 59ca94: cx < (unsigned) dx -> 0x59ca80 [jb]
 59cc17: eax < (signed) 3 -> 0x59cb27 [jl]
 59cd80: dword ptr [esp + 0x10] >= (signed) 6 -> 0x59ce30 [jge]
 59cebc: edi >= (signed) 6 -> 0x59cfa6 [jge]
 59d04d: dword ptr [esp + 0x14] >= (signed) 0x70 -> 0x59d1f3 [jge]
 59d0f5: dword ptr [esp + 0x10] >= (signed) 0xc -> 0x59d18d [jge]
 59d28a: eax >= (signed) 0x300 -> 0x59d500 [jge]
 59d358: eax < (signed) 8 -> 0x59d330 [jl]
 59d39c: ecx > (unsigned) 7 -> 0x59d4bc [ja]
 59d4a4: edx < (signed) 3 -> 0x59d452 [jl]
 59d58a: dword ptr [esp + 0x10] >= (signed) 3 -> 0x59d65b [jge]
 59d6ec: dword ptr [esp + 0x20] >= (signed) 0x174 -> 0x59da0b [jge]
 59d76c: dword ptr [esp + 0x10] >= (signed) 0x1600 -> 0x59d7ed [jge]
 59d896: edx < (signed) 2 -> 0x59d840 [jl]
 59dbea: eax > (unsigned) 0x16 -> 0x59dc20 [ja]
 59dcbf: edx < (signed) 4 -> 0x59dc80 [jl]
 59dd26: ecx >= (signed) 5 -> 0x59dda6 [jge]
 59de1c: eax < (signed) 2 -> 0x59db70 [jl]
 59de45: dword ptr [ebx + eax*8 + 0x111c4] > (signed) 0 -> 0x59de63 [jg]
 59de78: dword ptr [ebx + edx*8 + 0x111c4] > (signed) 0 -> 0x59de96 [jg]
 59de9c: edi < (signed) 4 -> 0x59de30 [jl]
 59dea4: esi < (signed) 2 -> 0x59de2e [jl]

```


### 2. 0x6740A0 `escort_level_instance::update` (7972 bytes)

Convention: thiscall, ECX = level instance (kept in EBP), no stack arguments, plain `ret`. Frame `sub esp,0x50` (no /GS cookie). Level-instance vtable slot 3: returns nothing meaningful (AL is not read by the caller `activity_instance_phase_run_level`, which uses `EAX` = status; the exit paths leave EAX = `mov eax,[0x27A57F4]` or 3, see section 1). It first calls the base `activity_level_instance::update` (0x53F850) with ECX unchanged.

Key objects: EBP = escort level instance; EBX = `[0x1408E88]` the activity instance; `[EBX+0x40]`, `[EBX+0x48]`, `[EBX+0x50]` are three holder objects with vtable `+0x2C` = is_valid (bool) and `+0x34` = get_object; by use they are the escorted vehicle, the client and a second escort NPC. `[EBP+0x24]` = state (0..0x12), `[EBP+0xF4]`, `[EBP+0xF8]` = player / client humans, `[EBP+0xFC...]` participant array, `[EBP+0xC4]` game-time anchor, `[EBP+0x50/0x54/0x58/0x5C/0x70/0xB8]` timers, `[EBP+0xFD]` co-op sync byte, `[EBP+0x38..0x3C]` flag bytes.

Sections (all offsets are code addresses):
1. 0x6740A0-0x6740CA: base update; `if ([EBP+0xFC] == 0) { if coop_partner_human() == 0 goto exit 0x675FB7 }`.
2. 0x6740CA-0x67418A: `ESI = [0x21703D4]` (player), `sub_676B90` (state-0 helper), then two progress-scaled values: `p = 1.0f - (float)u32([EBP+0x58]) / (float)u32([EBP+0xC8])` (the u32->f32 conversion adds 4294967296.0f = bits 0x4F800000 when the integer's sign bit is set, `jge` on the loaded dword; stored to `[esp+0x18]`). Then for the two path objects `[0x2527B0C]` and `[0x2527B10]`: `v = lerp(a, b, t)` with `a=[obj+0x34]`, `b=[obj+0x38]`, `t=[EBP+0x20]` (first object) then `p` (second): `fld [b]; fsub [a]; fmul t; fadd [a]; fstp tmp` (f32 store), `fst [obj+0x20]`; if `fucomp` shows `[obj+0x24] != v` (test ah,0x44; jnp = equal -> skip) then `[obj+0x24] = v; [obj+0x1C] = 1`.
3. 0x67418A-0x67423C: client alive check: holder `[EBX+0x48]` vtable +0x2C is_valid / +0x34 get; if invalid or `human_is_dead_or_invalid` -> exit 0x675F97 (sets state 7 and the "CRITICAL_VEHICLE_DESTROYED_VEHICLE_TEXT" message). If the client human has flag `[h+0x84] & 1`: `human_set_force_flag_invulnerable(h, 0)` (stdcall ret 8) and `human_take_damage(h, hp + 1.0 (f64 1.0 at 0xE5BF40), 0, 0)`.
4. 0x67423C-0x674297: same invulnerable-clear / damage for the second human `[EBP+0xF8]` when `[h+0x4C] == 1` and `[h+0x3CC] != 1`.
5. 0x674297-0x674335: partner / second NPC checks (`coop_partner_human`, holder +0x50, state 0x10 test `[EBP+0x24] == 0x10`), failure exit through `[0x27A57F4]`; co-op byte sync: `act_packet_send_1e(0)` and `[EBP+0xFD] = 0`.
6. 0x674335-0x67442D: vehicle holder `[EBX+0x40]` vtable +0x34 -> vehicle ptr; byte-indexed switch `jmp [ecx*4+0x675FC4]` (index = `vehicle[+0x48]`-style kind > 0x12 -> default; byte table at 0x675FCC maps kinds 0, 9-12, 16-18 to 0 and the rest to 1) deciding `[esp+0x10]` (1 = "get back in" prompt). When the player is not in the vehicle: spawns the HUD arrow icon (`sub_7A2030`, `effect_find_by_name("icon_lg_c")`, `sub_92D1A0`) and the message `lstr_get("ACT_ESCORT_GETBACK_MSG")` -> `sub_79C150` stored in `[EBP+0x30]`; when back in: frees both (`sub_7A2130`, `sub_92D4B0`, `sub_79D770`).
7. 0x67442D-0x674464: vehicle contact test: `fld1; fcomp [v+0x94]; test ah,0x41; jp` taken for ST0<=op... (see table: taken when ST0>op or unordered, jumping to the "not on the ground" result 0); if `[v+0x?]&1` then `vehicle_any_wheel_in_contact(EDI = ESI = v)` -> `[esp+0x10]` (AL); store 1 on the not-taken path.
8. 0x674464-0x6744D4: player-in-vehicle / door state gates (`sub_AAB370`, `sub_6A80C0`, `[EBP+0x3B] = 1`).
9. 0x6744D4 `jmp [eax*4 + 0x675FE0]` on `[EBP+0x24]` (19 entries, `ja 0x675F63` for > 0x12): the state machine. Case entries: 0x6744E7, 0x674710, 0x674B1D, 0x674F35, 0x6754F8, 0x675638, 0x6756AD, 0x6756E8 (states 7 and 8), 0x67576E, 0x67591E (10 and 11), 0x675A8F, 0x675BD3, 0x675C0C, 0x675C8C, 0x675D89, 0x675E0C, 0x675EB0.
   - state 0 (0x6744E7): make the escort NPCs enter the vehicle (`ai_order_k1c_start` stdcall-ish 6 args, `vehicle_ai_set_force_flag_force_no_passenger_exit`, `sub_6A8100`, `sub_9E0490`), creates the escort HUD icon (`sub_7A2030(7, ...)`, "icon_green"), `[EBP+0x24] = 1` when everyone is seated; `sub_79C500(0x27A564C, 0xEAA7A4)` message.
   - state 1 (0x674710): travel to the drop-off: updates timers, checks `vehicle_seat_occupant_by_index(vehicle, 3)`, calls `sub_679590/679160/6770F0/6771E0` (path / spawn setup) then state 2/3; later part (0x674775...) updates `[EBP+0x50/0x54/0x58]` timers from `[0xE843AC]`, computes speed-based fractions with the f64 constants 100.0 (0xE5C190), 4.0f (0xDD522C) and compares (see table).
   - state 2 (0x674B1D): door part handling (`vehicle_seat_door_part`, `vehicle_part_door_state_is_3`, `vehicle_part_doors_set_state`), `timer_elapsed([EBP+0xDC])`, co-op request packets `act_packet_send_1d(...)`, `[EBP+0xFD] = 1`; speed ramp with f64 0.1 (0xDD50E8), 0.4 (0xE5C358), -0.15 (0xE5DC40), f32 0.25 (0xE44850), 0.55 (0xE5DBD0), 0.7 (0xDD347C = "333?"), f64 0.85 (0xE5C0E8): a piecewise function of the elapsed time that sets `[0xE974D4]` (`_ftol2_sse`, `sub_66F540(EDX=1, EDI=0xE974C4)`).
   - state 3 (0x674F35): same ramp mirrored for the return leg (compares at 0x674FB9; `sub_6730D0(EBP)`, `sub_672F20(0x4D, 1)`, `sub_673BA0(EBP)`), then state 4.
   - state 4 (0x6754F8) / 5 (0x675638): wait for both humans to be in the vehicle (`human_in_vehicle` x3, `sub_A6C810`, `trigger_deactivate` / `trigger_activate` of the destination trigger `[EBP+0x28]`, `sub_53F940/53F8E0`), `sub_AAF4A0` (5 stack args) for the drop-off, then `[EBP+0x24] = 6`.
   - states 6-8 (0x6756AD, 0x6756E8): exit vehicle prompts, `sub_B07D90`, `sub_756010`, `vehicle_set_force_flag_infinite_vehicle_collision_mass(v, 0)` (stdcall ret 8), fade-out (`gfx_set_color_bytes(0xEAC198)`, `screen_fade_out(0xFA)`) then `[EBP+0x24] = 9`.
   - states 9-11 (0x67576E, 0x67591E): wait `screen_fade_is_full`, move the player (`human_vehicle_interaction_exiting/entering`, `sub_B07AB0`), `game_clock_advance(...)` for the skip-time part (7200.0f at 0xE5DC38, global `[0x2529620]`), then `screen_fade_in(0xFA)` and state 0xC.
   - states 12-15 (0x675A8F ... 0x675D89): post-fade (compare `fcomp [0x2526DA0]` fade progress with `test ah,0x41; jne` = ST0<=op), re-seat, `screen_fade_is_clear`, state 0xD / 0xE / 0xF / 0x10; uses -pi f32 (0xE5C168).
   - states 16-18 (0x675D89, 0x675E0C, 0x675EB0): `object_teleport`, `sub_A941D0`, final re-entry, state 0x11 -> 0x12 -> 0 (loop), `vehicle_set_force_flag_infinite_vehicle_collision_mass`.
10. 0x675F63-0x675FC4 shared exits: `lstr_get("CRITICAL_VEHICLE_DESTROYED_VEHICLE_TEXT")` -> `[EBP+0x28]`, `[EBP+0xC] = 7` (level state "failed"), `localized_string_find` variant at 0x675FA5.

Hazards for the port: (a) the three holder objects are virtual (each `call` is `mov eax,[ecx]; mov eax,[eax+0x34]; call eax` with ECX = holder); keep them as raw vtable calls; (b) every x87 value is stored to a f32 temp before the next use (`fstp dword ptr [esp+..]` then `fld`), so the ramp values must be rounded to f32 at those points; (c) `fucomp st(2)` at 0x674133 / 0x674174 compares the new f32 against the old stored f32 for equality (jnp = equal); (d) two `jmp` tables with different bases (`0x675FC4` for the kind->flag byte switch, `0x675FE0` for the 19 states).


Generated tables (exact, from the bytes):

```
FUNCTION 6740A0..675FC4 size 7972 (2413 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x50; no /GS cookie
 esp+0x1c (lea edx @ 67467D) passed to net_send_prop_human_run_variant
 esp+0x30 (lea eax @ 6748ED) passed to sub_40ea60
 esp+0x20 (lea eax @ 674980) passed to sub_a637a0
 esp+0x40 (lea ecx @ 674985) passed to sub_a637a0
 esp+0x38 (lea ecx @ 67498A) passed to sub_a637a0
 esp+0x20 (lea eax @ 6749BE) passed to sub_40ea60
 esp+0x34 (lea edi @ 674ADF) passed to sub_bed5f0
 esp+0x40 (lea eax @ 674B00) passed to sub_99dd20
 esp+0x40 (lea eax @ 6755E4) passed to sub_aaf4a0
 esp+0x3c (lea eax @ 675678) passed to sub_433190
 esp+0x3c (lea edx @ 675686) passed to vehicle_interaction_query
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0053F850 activity_level_instance::update: x1 first 6740AF; callee reads ecx; ret 0; cleanup -
 009D7BD0 coop_partner_human: x17 first 6740BD; callee reads -; ret 0; cleanup -
 00676B90 sub_676b90: x1 first 6740D9; callee reads -; ret 0; cleanup -
 009855F0 human_is_dead_or_invalid: x3 first 6741A8; callee reads edx; ret 0; cleanup -
 00965F40 human_set_force_flag_invulnerable: x2 first 6741F8; callee reads -; ret 0x8; cleanup -
 009847A0 human_take_damage: x2 first 674232; callee reads ecx,edx; ret 0; cleanup -
 0066FC30 act_packet_send_1e: x4 first 674326; callee reads ecx; ret 0; cleanup [4]
 009B8C50 human_in_vehicle: x10 first 67436D; callee reads edi,esi; ret 0; cleanup -
 007A2030 sub_7a2030: x3 first 674395; callee reads ecx; ret 0; cleanup [24]
 0050DB60 effect_find_by_name: x2 first 6743A7; callee reads ecx; ret 0; cleanup -
 0092D1A0 sub_92d1a0: x2 first 6743BC; callee reads ecx; ret 0; cleanup [20]
 007F49E0 lstr_get: x2 first 6743D9; callee reads edi,esi; ret 0; cleanup -
 0079C150 sub_79c150: x1 first 6743DF; callee reads -; ret 0; cleanup [8]
 007A2130 sub_7a2130: x4 first 6743F3; callee reads esi; ret 0; cleanup -
 0092D4B0 sub_92d4b0: x2 first 67440D; callee reads -; ret 0; cleanup -
 0079D770 sub_79d770: x2 first 67441E; callee reads edx; ret 0; cleanup [4]
 00AAB6B0 vehicle_any_wheel_in_contact: x1 first 674451; callee reads edi; ret 0; cleanup -
 00AAB370 sub_aab370: x1 first 6744A9; callee reads edx; ret 0; cleanup -
 006A80C0 sub_6a80c0: x1 first 6744C0; callee reads -; ret 0; cleanup -
 009B92E0 human_entering_or_exiting_vehicle: x10 first 674507; callee reads eax; ret 0; cleanup -
 00AA7380 vehicle_seat_occupant_by_index: x7 first 674552; callee reads ecx,edx; ret 0; cleanup -
 00AAB290 sub_aab290: x8 first 674562; callee reads ecx; ret 0; cleanup -
 0045DDD0 ai_order_k1c_start: x1 first 674596; callee reads ecx; ret 0; cleanup [24]
 00AB1710 sub_ab1710: x1 first 6745EC; callee reads ecx; ret 0; cleanup -
 00B2C370 vehicle_ai_set_force_flag_force_no_passenger_exit: x1 first 6745F4; callee reads -; ret 0x8; cleanup -
 006A8100 sub_6a8100: x4 first 6745F9; callee reads -; ret 0; cleanup -
 009E0490 sub_9e0490: x2 first 67460A; callee reads -; ret 0x8; cleanup -
 0053F8E0 sub_53f8e0: x3 first 674673; callee reads eax,ecx,esi; ret 0; cleanup -
 009690E0 net_send_prop_human_run_variant: x1 first 674690; callee reads -; ret 0x8; cleanup -
 00440F60 human_ai_set_force_flag_disallow_persona_lines: x2 first 6746B2; callee reads -; ret 0x8; cleanup -
 00AB0D60 veh_pkt_send_19: x1 first 6746CE; callee reads -; ret 0; cleanup [8]
 0079C500 sub_79c500: x5 first 6746FB; callee reads eax,ecx; ret 0; cleanup [8, 24]
 0053F940 sub_53f940: x3 first 674749; callee reads ecx,esi; ret 0; cleanup -
 00679590 sub_679590: x1 first 674751; callee reads ecx,edx; ret 0x4; cleanup -
 00679160 sub_679160: x1 first 674759; callee reads eax; ret 0x4; cleanup -
 006770F0 sub_6770f0: x1 first 67475E; callee reads esi; ret 0; cleanup -
 006771E0 sub_6771e0: x1 first 674767; callee reads esi; ret 0; cleanup -
 0040EA60 sub_40ea60: x3 first 6748F8; callee reads eax,ecx; ret 0; cleanup -
 00673F30 sub_673f30: x6 first 674930; callee reads edi; ret 0x8; cleanup -
 00A637A0 sub_a637a0: x2 first 674990; callee reads ecx,edx; ret 0x8; cleanup -
 00A72100 sub_a72100: x1 first 674A30; callee reads eax,edi; ret 0; cleanup -
 00BED5F0 sub_bed5f0: x1 first 674AE3; callee reads eax,ecx,edi,esi; ret 0; cleanup -
 0099DD20 sub_99dd20: x1 first 674B04; callee reads eax,edx; ret 0; cleanup -
 00AA77C0 vehicle_seat_door_part: x4 first 674B2D; callee reads ecx,edx; ret 0; cleanup -
 00AA7E30 vehicle_part_door_state_is_3: x2 first 674B33; callee reads eax; ret 0; cleanup [4]
 00AA7FB0 vehicle_part_doors_set_state: x2 first 674B4D; callee reads eax; ret 0; cleanup [8]
 00BDD230 timer_elapsed: x12 first 674B68; callee reads eax; ret 0; cleanup -
 004912B0 sub_4912b0: x5 first 674BD6; callee reads ecx; ret 0; cleanup -
 0066F990 act_packet_send_1d: x3 first 674CE7; callee reads ecx; ret 0; cleanup [24]
 00BDD2A0 timer_elapsed_ms: x4 first 674CFB; callee reads eax; ret 0; cleanup -
 00DA0140 _ftol2_sse: x2 first 674D9E; callee reads -; ret 0; cleanup -
 0066F540 sub_66f540: x5 first 674DBE; callee reads edi; ret 0; cleanup -
 006730D0 sub_6730d0: x1 first 674FD1; callee reads -; ret 0x4; cleanup -
 00672F20 sub_672f20: x1 first 675058; callee reads edi; ret 0x8; cleanup -
 00673BA0 sub_673ba0: x1 first 6750AE; callee reads -; ret 0x4; cleanup -
 006770B0 act_escort_client_request_msg_text: x1 first 6752A7; callee reads eax,ecx; ret 0; cleanup -
 009B8CB0 sub_9b8cb0: x1 first 6752F5; callee reads eax,ecx; ret 0; cleanup -
 0066F080 sub_66f080: x1 first 675493; callee reads ecx,esi; ret 0; cleanup -
 00A6C810 sub_a6c810: x2 first 67552D; callee reads ecx; ret 0; cleanup -
 00A6BD60 trigger_deactivate: x1 first 675543; callee reads eax; ret 0; cleanup -
 00A6BCE0 trigger_activate: x1 first 67555C; callee reads esi; ret 0; cleanup -
 00AAF4A0 sub_aaf4a0: x1 first 6755ED; callee reads eax,ecx; ret 0; cleanup [20]
 00433190 sub_433190: x1 first 67567C; callee reads eax,edx; ret 0; cleanup -
 00B072F0 vehicle_interaction_query: x1 first 675691; callee reads ecx; ret 0; cleanup [8]
 00756010 sub_756010: x5 first 6756C4; callee reads -; ret 0; cleanup -
 00AB9BE0 vehicle_set_force_flag_infinite_vehicle_collision_mass: x5 first 6756D5; callee reads -; ret 0x8; cleanup -
 00B07D90 sub_b07d90: x3 first 6756F6; callee reads -; ret 0; cleanup -
 00D14D00 gfx_set_color_bytes: x2 first 675742; callee reads eax; ret 0; cleanup -
 004D13B0 screen_fade_out: x2 first 675752; callee reads ecx,esi; ret 0; cleanup [4]
 004D1920 screen_fade_is_full: x2 first 67576E; callee reads -; ret 0; cleanup -
 009B9350 human_vehicle_interaction_exiting: x3 first 6757AF; callee reads -; ret 0; cleanup [4]
 00679420 sub_679420: x1 first 6757CB; callee reads -; ret 0; cleanup -
 00679AF0 sub_679af0: x1 first 6757D0; callee reads -; ret 0; cleanup -
 00AE28F0 vehicle_from_handle: x1 first 675842; callee reads ecx; ret 0; cleanup -
 0068A1E0 game_clock_advance: x1 first 675902; callee reads ecx; ret 0; cleanup [16]
 009B9320 human_vehicle_interaction_entering: x3 first 67599A; callee reads -; ret 0; cleanup [4]
 00B07AB0 sub_b07ab0: x3 first 6759AB; callee reads -; ret 0; cleanup [4]
 004D15C0 screen_fade_in: x3 first 675A30; callee reads ecx,esi; ret 0; cleanup [4]
 004D1900 screen_fade_is_clear: x3 first 675B66; callee reads -; ret 0; cleanup -
 00BE51D0 sub_be51d0: x1 first 675D04; callee reads ecx; ret 0x8; cleanup -
 00A94140 sub_a94140: x1 first 675D10; callee reads -; ret 0; cleanup -
 00672390 sub_672390: x1 first 675D1B; callee reads -; ret 0x8; cleanup -
 00AAAB60 sub_aaab60: x1 first 675D6A; callee reads ecx,edx; ret 0; cleanup [12]
 0053F960 sub_53f960: x1 first 675D92; callee reads ecx,edi; ret 0x4; cleanup -
 009A0B00 object_teleport: x1 first 675DEC; callee reads ecx,edx; ret 0; cleanup -
 00A941D0 sub_a941d0: x1 first 675DFB; callee reads edi,esi; ret 0; cleanup -
 00612460 localized_string_find: x1 first 675FA5; callee reads eax,ecx,esi; ret 0; cleanup -
INDIRECT CALLS: 62 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E44850: r2 w0 first 674D48 fld sz4
 00E5BF40: r2 w0 first 67420D fadd sz8
 00E5BFD8: r13 w0 first 6740F1 fadd sz4
 00E5C0E8: r2 w0 first 674E13 fld sz8
 00E5C168: r1 w0 first 675CD5 fld sz4
 00E5C190: r3 w0 first 6748FD fcomp sz8
 00E5C358: r2 w0 first 674D74 fld sz8
 00E5CE88: r2 w0 first 674D92 fsub sz8
 00E5DBD0: r2 w0 first 674DCD fld sz4
 00E5DC38: r1 w0 first 6758F2 fld sz4
 00E5DC40: r2 w0 first 674D98 fdiv sz8
 00E843AC: r8 w0 first 674790 mov sz4
 00E974C4: r5 w0 first 674DB9 addr-of sz4
 00E974D0: r1 w0 first 675653 mov sz4
 00E974D4: r5 w3 first 674D42 mov sz4
 00EAA7A4: r5 w0 first 6746BC addr-of sz4
 00EAA7CC: r1 w0 first 6747DD addr-of sz4
 00EAC198: r2 w0 first 67573D addr-of sz4
 00EAD20C: r1 w0 first 6743CB mov sz4
 00EC5AE4: r2 w0 first 6743B7 addr-of sz4
 01408E88: r8 w0 first 6740CF mov sz4
 0140C7EC: r2 w2 first 6757D5 mov sz4
 0140C7F0: r2 w1 first 6757DB mov sz4
 0215E0B4: r0 w1 first 67473E mov sz4
 0215E0B8: r0 w1 first 674744 mov sz4
 021703D4: r4 w0 first 6740CA mov sz4
 02526DA0: r1 w0 first 675A95 fcomp sz4
 02527B0C: r1 w0 first 6740F9 mov sz4
 02527B10: r1 w0 first 674149 mov sz4
 02529620: r0 w2 first 67590A mov sz1
 027A564C: r2 w0 first 6745B7 mov sz4
 027A5650: r1 w0 first 6746E3 mov sz4
 027A569C: r1 w0 first 6746C1 addr-of sz4
 027A56A0: r1 w0 first 6746B7 mov sz4
 027A56B0: r2 w0 first 674FE2 cmp sz4
 027A56B4: r1 w0 first 674FF1 mov sz4
 027A56C4: r1 w0 first 6747E5 addr-of sz4
 027A56C8: r1 w0 first 6747D8 mov sz4
 027A56EC: r1 w0 first 675035 addr-of sz4
 027A56F0: r1 w0 first 675028 mov sz4
 027A57B4: r1 w0 first 675BB9 addr-of sz4
 027A57B8: r1 w0 first 675BAC mov sz4
 027A57E0: r1 w0 first 675F97 mov sz4
 027A57F4: r1 w0 first 6742E3 mov sz4
 027A5830: r1 w0 first 6750E4 mov sz4
STRINGS
 DE8590 "icon_lg_c" @ 67439F
 DFE00C "ACT_ESCORT_GETBACK_MSG" @ 6743D4
 DEC378 "icon_green" @ 67463B
 DD347C "333?" @ 674DF0
 DEAA38 "CRITICAL_VEHICLE_DESTROYED_VEHICLE_TEXT" @ 675F76
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5BFD8] fadd @ 6740F1: f32=4294967296.0 bits=0x4f800000
 [E5BF40] fadd @ 67420D: f64=1.0 bits=0x3ff0000000000000
 [E5C190] fcomp @ 6748FD: f64=100.0 bits=0x4059000000000000
 [DD522C] fld @ 6749CF: f32=4.0 bits=0x40800000
 [E44850] fld @ 674D48: f32=0.25 bits=0x3e800000
 [DD50E8] fcom @ 674D55: f64=0.10000000149011612 bits=0x3fb99999a0000000
 [E5C358] fld @ 674D74: f64=0.4000000059604645 bits=0x3fd99999a0000000
 [E5CE88] fsub @ 674D92: f64=0.1 bits=0x3fb999999999999a
 [E5DC40] fdiv @ 674D98: f64=-0.15 bits=0xbfc3333333333333
 [E5DBD0] fld @ 674DCD: f32=0.550000011920929 bits=0x3f0ccccd
 [DD347C] fld @ 674DF0: f32=0.699999988079071 bits=0x3f333333
 [E5C0E8] fld @ 674E13: f64=0.8500000238418579 bits=0x3feb333340000000
 [DD1C90] fld @ 6755CA: f32=2.0 bits=0x40000000
 [E5DC38] fld @ 6758F2: f32=7200.0 bits=0x45e10000
 [DD1C8C] fld @ 675A8F: f32=0.5 bits=0x3f000000
 [E5C168] fld @ 675CD5: f32=-3.1415927410125732 bits=0xc0490fdb
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 674133 fucomp st(2) ; test ah,0x44; jnp @ 67413C -> 0x674147: taken when ST0==op
 674174 fucomp st(2) ; test ah,0x44; jnp @ 67417D -> 0x674188: taken when ST0==op
 674442 fcomp dword ptr [esi + 0x94] ; test ah,0x41; jp @ 67444D -> 0x67445f: taken when ST0>op | unordered
 6748FD fcomp qword ptr [0xe5c190] ; test ah,0x5; jp @ 674908 -> 0x674946: taken when ST0>op | ST0==op | unordered
 6749D9 fcom st(1) ; test ah,0x41; jne @ 6749E2 -> 0x674a08: taken when ST0<op | ST0==op | unordered
 6749E4 fcom qword ptr [0xe5c190] ; test ah,0x5; jp @ 6749EF -> 0x674a08: taken when ST0>op | ST0==op | unordered
 674A08 fcomp qword ptr [0xe5c190] ; test ah,0x1; jne @ 674A13 -> 0x675fb7: taken when ST0<op | unordered
 674AAC fcomp st(1) ; test ah,0x41; jne @ 674AB3 -> 0x674abf: taken when ST0<op | ST0==op | unordered
 674D55 fcom qword ptr [0xdd50e8] ; test ah,0x41; jne @ 674D60 -> 0x674d6f: taken when ST0<op | ST0==op | unordered
 674D62 fcom st(1) ; test ah,0x5; jnp @ 674D69 -> 0x674e4a: taken when ST0<op
 674D7E fcomp st(2) ; test ah,0x5; jp @ 674D85 -> 0x674dca: taken when ST0>op | ST0==op | unordered
 674D87 fcom st(1) ; test ah,0x41; jne @ 674D8E -> 0x674dca: taken when ST0<op | ST0==op | unordered
 674DD7 fcomp st(2) ; test ah,0x5; jp @ 674DDE -> 0x674ded: taken when ST0>op | ST0==op | unordered
 674DE0 fcom st(1) ; test ah,0x41; je @ 674DE7 -> 0x674d90: taken when ST0>op
 674DFA fcomp st(2) ; test ah,0x5; jp @ 674E01 -> 0x674e10: taken when ST0>op | ST0==op | unordered
 674E03 fcom st(1) ; test ah,0x41; je @ 674E0A -> 0x674d90: taken when ST0>op
 674E1D fcomp st(2) ; test ah,0x5; jp @ 674E24 -> 0x674e37: taken when ST0>op | ST0==op | unordered
 674E26 fcom st(1) ; test ah,0x41; je @ 674E2D -> 0x674d90: taken when ST0>op
 674E3C fcomp st(1) ; test ah,0x5; jp @ 674E43 -> 0x674e53: taken when ST0>op | ST0==op | unordered
 674FB9 fcomp qword ptr [0xdd50e8] ; test ah,0x5; jp @ 674FC4 -> 0x674fd0: taken when ST0>op | ST0==op | unordered
 67535B fcom qword ptr [0xdd50e8] ; test ah,0x41; jne @ 675366 -> 0x675375: taken when ST0<op | ST0==op | unordered
 675368 fcom st(1) ; test ah,0x5; jnp @ 67536F -> 0x675454: taken when ST0<op
 675384 fcomp st(2) ; test ah,0x5; jp @ 67538B -> 0x6753d4: taken when ST0>op | ST0==op | unordered
 67538D fcom st(1) ; test ah,0x41; jne @ 675394 -> 0x6753d4: taken when ST0<op | ST0==op | unordered
 6753E1 fcomp st(2) ; test ah,0x5; jp @ 6753E8 -> 0x6753f7: taken when ST0>op | ST0==op | unordered
 6753EA fcom st(1) ; test ah,0x41; je @ 6753F1 -> 0x675396: taken when ST0>op
 675404 fcomp st(2) ; test ah,0x5; jp @ 67540B -> 0x67541a: taken when ST0>op | ST0==op | unordered
 67540D fcom st(1) ; test ah,0x41; je @ 675414 -> 0x675396: taken when ST0>op
 675427 fcomp st(2) ; test ah,0x5; jp @ 67542E -> 0x675441: taken when ST0>op | ST0==op | unordered
 675430 fcom st(1) ; test ah,0x41; je @ 675437 -> 0x675396: taken when ST0>op
 675446 fcomp st(1) ; test ah,0x5; jp @ 67544D -> 0x67545d: taken when ST0>op | ST0==op | unordered
 675A95 fcomp dword ptr [0x2526da0] ; test ah,0x41; jne @ 675AA0 -> 0x675ad8: taken when ST0<op | ST0==op | unordered
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 6757F1 neg eax   (prev: sub eax, edx)
 6757F3 sbb eax, eax   (prev: neg eax)
JUMP TABLES
 jmp @ 674357 table 675FC4 entries(2): 67435E 674363 ; byte-index table at 675FCC
   byte table at 675FCC: 00 01 01 01 01 01 01 01 01 00 00 00 00 01 01 01 00 00 00 90 e7 44 67 00 10 47 67 00 1d 4b 67 00
 jmp @ 6744E0 table 675FE0 entries(19): 6744E7 674710 674B1D 674F35 6754F8 675638 6756AD 6756E8 6756E8 67576E 67591E 67591E 675A8F 675BD3 675C0C 675C8C 675D89 675E0C 675EB0
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 6740ef: ecx >= (signed) 0 -> 0x6740f7 [jge]
 67434e: eax > (unsigned) 0x12 -> 0x674363 [ja]
 6744da: eax > (unsigned) 0x12 -> 0x675f63 [ja]
 6747a0: eax <= (signed) 0x6b49d200 -> 0x6747aa [jle]
 674a56: eax <= (signed) 0 -> 0x674acd [jle]
 674b66: dword ptr [ebp + 0xdc] < (signed) edi -> 0x674b75 [jl]
 674b9c: eax >= (signed) 0 -> 0x674ba4 [jge]
 674c2f: eax >= (signed) 0 -> 0x674c37 [jge]
 674cae: eax < (signed) ecx -> 0x674cb2 [jl]
 674d51: esi >= (signed) 3 -> 0x674d71 [jge]
 674d7a: esi >= (signed) 5 -> 0x674dc8 [jge]
 674dae: esi >= (signed) edx -> 0x674db9 [jge]
 674dd3: esi >= (signed) 7 -> 0x674deb [jge]
 674df6: esi >= (signed) 9 -> 0x674e0e [jge]
 674e19: esi >= (signed) 0xb -> 0x674e35 [jge]
 674e3a: esi >= (signed) 0xd -> 0x674e51 [jge]
 674e69: eax >= (signed) 0 -> 0x674e71 [jge]
 674edf: eax < (signed) dword ptr [ebp + 0x54] -> 0x675fb7 [jl]
 674f43: dword ptr [ebp + 0xd8] <= (signed) edi -> 0x674fad [jle]
 674f62: dword ptr [ebp + 0xdc] < (signed) 0 -> 0x674f6d [jl]
 674f97: eax < (signed) dword ptr [ebp + 0xd8] -> 0x674f50 [jl]
 674fb4: dword ptr [0xe974d4] >= (signed) 1 -> 0x674fd0 [jge]
 675020: edx >= (signed) 0x1388 -> 0x675042 [jge]
 67509d: eax <= (signed) 0x6b49d200 -> 0x6750a7 [jle]
 6750ab: dword ptr [ebp + 0x54] > (signed) 0 -> 0x6750da [jg]
 6750de: dword ptr [ebp + 0x58] >= (signed) 0 -> 0x675fb7 [jge]
 675145: dword ptr [eax] < (signed) esi -> 0x675154 [jl]
 67517b: eax >= (signed) 0 -> 0x675183 [jge]
 67520e: eax >= (signed) 0 -> 0x675216 [jge]
 67528d: eax < (signed) ecx -> 0x675291 [jl]
 675357: esi >= (signed) 3 -> 0x675377 [jge]
 675380: esi >= (signed) 5 -> 0x6753d2 [jge]
 6753b4: esi >= (signed) ecx -> 0x6753bf [jge]
 6753dd: esi >= (signed) 7 -> 0x6753f5 [jge]
 675400: esi >= (signed) 9 -> 0x675418 [jge]
 675423: esi >= (signed) 0xb -> 0x67543f [jge]
 675444: esi >= (signed) 0xd -> 0x67545b [jge]
 675572: dword ptr [ebp + 0x5c] < (signed) 0 -> 0x67557d [jl]
 675585: eax >= (signed) 0xf -> 0x6755a0 [jge]
 67558a: eax >= (signed) 0xd -> 0x675596 [jge]
 6755ad: dword ptr [ebp + 0xa4] < (signed) 0 -> 0x675fb7 [jl]
 67561e: eax <= (signed) 0x6b49d200 -> 0x675fb7 [jle]
 67563e: dword ptr [ebp + 0x5c] < (signed) edi -> 0x675649 [jl]
 675790: eax <= (signed) 0x6b49d200 -> 0x67579d [jle]
 675812: dword ptr [ecx + 0x70] <= (signed) 0 -> 0x67587b [jle]
 67586d: eax < (signed) dword ptr [ecx + 0x70] -> 0x675820 [jl]
 675aac: dword ptr [ebp + 0xd8] <= (signed) edi -> 0x675ad8 [jle]
 675ad6: eax < (signed) dword ptr [ebp + 0xd8] -> 0x675ab4 [jl]
 675b9c: eax <= (signed) 0x6b49d200 -> 0x675ba6 [jle]
 675c71: eax <= (signed) 0x6b49d200 -> 0x675fb7 [jle]
 675cae: eax <= (signed) 0x6b49d200 -> 0x675cbb [jle]
 675d59: edx >= (unsigned) dword ptr [esi + 0x8c24] -> 0x675d72 [jae]
 675f4c: eax <= (signed) 0x6b49d200 -> 0x675fb7 [jle]

```


### 3. 0x5AC6D0 `security_level_instance::load_from_table` (5936 bytes)

Convention: thiscall, ECX = level instance (EDI), stack args `[ebp+8]`, `[ebp+0xC]` (the xtbl element, ESI), `[ebp+0x10]`, `ret 0xC`. Frame via `push ebp; and esp,-8; sub esp,0x104` with a /GS cookie. Vtable slot 0 of the class.

Sections:
1. 0x5AC6D0-0x5AC70A: forwards its three stack args to `activity_level_instance::load_from_table` (0x53F700, ECX = this).
2. 0x5AC70A-0x5AC7AC: zeroes `[this+0x58, 0x1F10, 0x1F14, 0x2BF4, 0x1FD8, 0x64, 0x6C(byte), 0x2C20, 0x1C]`, `[this+0x48] = -1`; builds a circular doubly-linked list of five nodes (12 bytes each) at `this+0x270C` with head `[this+0x2748]` (`[n+4]`/`[n+8]` prev / next; empty list when head == 0 gets `n.prev = n.next = n`, else inserts after the head); `[this+0x1FDC..0x1FE8] = 0xFF`.
3. 0x5AC7C9-0x5AC9CF choose the event index into `[this+0x7C]`: if `[0xE98104]` (forced event) is in `0 <= idx < [info+0x111AC]` (`jl` / `jge`, signed) use it. Otherwise scan the xtbl `Eligible_Events` / `Eligible_Event` children: `__stricmp(child.name, "Eligible_Events")` then each `Eligible_Event` with `Event_Name` matched by `sub_59DF50(info, text)` (returns an event index, `jl` = not found); keeps a minimum-use filter: `use[0x2527594 + idx*4]` (signed compare `jg`/`jge`: a lower count replaces the candidate list, equal appends) skipping events whose `Disabled_In_Coop` bool is set while a co-op partner exists (the loop over `[0x217043C]` / `[0x21703D4]` parent chain `[+0x1294]` decides "partner present"). The pick is `candidates[rng % n]` using the ring `0xEC3AC0[(*0xEC3ABC)++ & 0x7FF]` (wraps at 0x800 with `jl`), or `rng % [info+0x111AC]` when the list is empty.
4. 0x5AC9CF-0x5ACA23: `use[chosen] += 1`; `[this+0x2648] = (info.event[chosen*0x16C8 + 0x1598] > 0)` via `setg al` (signed) then `test al,al`; when set, `sub_95B2C0()` and `[this+0x264C] = [0xE843AC] + 0x9C40` with the game-time wrap `if (v > 0x6B49D200) v += 0x94B62E00`.
5. 0x5ACA23-0x5ACB3B scalar fields (each `xtbl_get_*` is ECX = field name, stack = element EBX, ESI = out): `Duration` float x f64 constant at 0xE5C0D8 (1000.0) -> `fistp` truncating via a temporarily modified FPU control word (`fnstcw; or 0xC00; fldcw; fistp qword; fldcw`), stored into `[this+0x68]` through `sub_B747D0` (xtbl_get_int variant, ECX = 0xDECAC4 / 0xDECAD4 depending on co-op: strings "Cash_Goal" path); `Cash_Goal` int -> `[this+0x68]`... (the co-op test `[0x2528B49]`, `[0x217043C]` selects which of two names, 0xDECAC4 or 0xDECAD4, is read); `Next_Celebrity_Delay` float `[esp+0x4C]`; `Celebrity_Annoyance_Tolerance` int `[this+0x50]`; `Base_Annoyance_Refund` int `[this+0x54]`; `Throw_Damage_Mult` float `[this+0x4C]`.
6. 0x5ACB3B-0x5ACD5D `_memset(this+0x..., 0, 0x498)` then `Celebrities` / `Celebrity` children: `Celeb_NPC_Type` (`string_hash_lower`), `Celeb_Variant` (`_strncpy(dst, text, 0x40)`), `Celeb_Persona` (`sub_6ADD50`, `sub_59E940`), bool `Loop_Path` -> `[this+0x58]`.
7. 0x5ACD5D-0x5AD1F3 `_memset(.., 0x5D0)`; `Crazed_Fan_Spawn_Levels` / `Crazed_Fan_Spawn_Level`: `Start` float, `Spawn_Interval` float (co-op variant `Spawn_Interval_Coop`, `Max_Crazed_Fans_Coop`), `Max_Crazed_Fans` int `[this+0x2058]`, `Spawn_Dist_Min` `[+0x205C]`, `Spawn_Dist_Max` `[+0x2060]`, `Health_Modifier` `[+0x2064]` (then `[this+0x2098] = 0`), `Spawn_Chances` / `Spawn_Chance` (`Crazed_Fans_Spawned` int `[x + i*8 + 0x2068]`, `Chance` float `[x + i*8 + 0x206C]`), `Fan_Type_Chances` / `Fan_Type_Chance` (`Chance`, `Type` matched against the type-name table at `[edi*4 + 0xDD35E8]`), `Lose_Interest_Delay`, `Annoyance`, `Max_Annoyance`.
8. 0x5AD22C-0x5AD460 per-fan `Weapon` (`weapon_info_find_by_name`), `NPC` and `Vehicle` names (`string_hash_lower`), bounded by `Fan_Type_Chance` count.
9. 0x5AD4C8-0x5AD6BE `Spawnable_Fans` / `Spawnable_Fan` with `Character_Preset` (hash): builds the names `"%s%s_#%i"` = `"crowd_control_" + "level" + n` (`sub_BF2C10` = sprintf-like, 3 stack args + buffer), `string_crc32`, `sub_A58130`, `sub_A26380`, `sub_A575C0` and `sub_5A3E70(level)`.
10. 0x5AD835-0x5ADAA3 player setup: `weapon_info_find_by_name("bean_bag_gun")`, `sub_B71CF0(...5 args)`, `sub_9E0710(player, 1)`, `inventory_equip_slot(player, weapon, 1, 0)` for the player and (when `[0x217043C]` partner exists) the partner, `human_pkt_send_03`, `player_teleport(...)` to the level's start navpoint (6 args, `add esp,0x18`), `net_send_notor_pkt_3 / _4`.
11. 0x5ADABA-0x5ADDFD `sub_59E740`, `sub_A568F0(5 args)`, `sub_5A1C60`, `sub_68AFA0(6 args)`, the HUD string `lstr_get("ACT_SECURITY_ANNOYANCE")` and cookie check; `ret 0xC`.

The `/GS` cookie value `[0xE80B50]` is XORed with `esp` at entry and checked at 0x5ADDF5. xtbl getters: `xtbl_get_float(ECX=name, stack=element, ESI=out)`, `xtbl_get_int(ECX, stack, ESI)`, `xtbl_get_bool(ECX=name, stack=(out*, element))`.


Generated tables (exact, from the bytes):

```
FUNCTION 5AC6D0..5ADE00 size 5936 (1807 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x104; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0x1f (lea eax @ 5AC898) passed to xtbl_get_bool
 esp+0x1c (lea esi @ 5ACA2D) passed to xtbl_get_float
 esp+0x4c (lea esi @ 5ACABC) passed to xtbl_get_float
 esp+0x38 (lea edx @ 5ACBEC) passed to string_hash_lower
 esp+0x84 (lea esi @ 5ACE17) passed to xtbl_get_float
 esp+0x68 (lea esi @ 5ACE9C) passed to xtbl_get_float
 esp+0x90 (lea esi @ 5AD194) passed to xtbl_get_float
 esp+0x84 (lea eax @ 5AD2AA) passed to string_hash_lower
 esp+0x98 (lea eax @ 5AD34E) passed to string_hash_lower
 esp+0x88 (lea eax @ 5AD54E) passed to string_hash_lower
 esp+0xdc (lea edi @ 5AD62F) passed to sub_bf2c10
 esp+0x40 (lea eax @ 5AD643) passed to string_crc32
 esp+0x84 (lea esi @ 5ADC28) passed to sub_68afa0
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0053F700 activity_level_instance::load_from_table: x1 first 5AC705; callee reads ecx; ret 0xC; cleanup -
 00C99030 __stricmp: x28 first 5AC808; callee reads -; ret 0; cleanup [8]
 00B75050 xtbl_get_bool: x2 first 5AC8A3; callee reads ecx; ret 0; cleanup [8]
 0059DF50 sub_59df50: x1 first 5AC8F0; callee reads eax; ret 0x8; cleanup -
 0095B2C0 sub_95b2c0: x1 first 5AC9FC; callee reads -; ret 0; cleanup -
 00B74EF0 xtbl_get_float: x12 first 5ACA31; callee reads ecx,esi; ret 0; cleanup [4]
 00B747D0 xtbl_get_int: x7 first 5ACAAE; callee reads ecx,esi; ret 0; cleanup [4]
 00CA0BC0 _memset: x2 first 5ACB3B; callee reads -; ret 0; cleanup [12]
 00BDCB30 string_hash_lower: x4 first 5ACBF5; callee reads eax,ecx; ret 0x4; cleanup -
 00C990F0 _strncpy: x1 first 5ACC8C; callee reads -; ret 0; cleanup [12]
 006ADD50 sub_6add50: x1 first 5ACCC9; callee reads -; ret 0; cleanup [4]
 0059E940 sub_59e940: x1 first 5ACCDA; callee reads -; ret 0x8; cleanup -
 00B716E0 weapon_info_find_by_name: x2 first 5AD25C; callee reads -; ret 0; cleanup [4]
 00BF2C10 sub_bf2c10: x1 first 5AD636; callee reads edi,esi; ret 0; cleanup [16]
 00BDCA30 string_crc32: x1 first 5AD647; callee reads eax,edx; ret 0x4; cleanup -
 00A58130 sub_a58130: x1 first 5AD650; callee reads ecx; ret 0x4; cleanup -
 00A26380 sub_a26380: x1 first 5AD65F; callee reads -; ret 0; cleanup -
 00A575C0 sub_a575c0: x1 first 5AD669; callee reads -; ret 0; cleanup [4]
 005A3E70 sub_5a3e70: x1 first 5AD6BE; callee reads -; ret 0x4; cleanup -
 00B71CF0 sub_b71cf0: x2 first 5AD85F; callee reads ecx; ret 0; cleanup [20]
 009E0710 sub_9e0710: x2 first 5AD8E8; callee reads -; ret 0x8; cleanup -
 0055A930 inventory_equip_slot: x2 first 5AD8FF; callee reads -; ret 0x10; cleanup -
 0098F460 human_pkt_send_03: x2 first 5AD9CC; callee reads edi; ret 0; cleanup [4]
 009D7BD0 coop_partner_human: x2 first 5ADA0A; callee reads -; ret 0; cleanup -
 009DB480 player_teleport: x1 first 5ADA78; callee reads ecx; ret 0; cleanup [24]
 0055FA40 net_send_notor_pkt_3: x1 first 5ADA8C; callee reads -; ret 0; cleanup -
 0055FB90 net_send_notor_pkt_4: x1 first 5ADAA3; callee reads edx; ret 0; cleanup -
 0059E740 sub_59e740: x1 first 5ADABA; callee reads ecx,edx; ret 0x4; cleanup -
 00A568F0 sub_a568f0: x1 first 5ADB2E; callee reads -; ret 0; cleanup -
 005A1C60 sub_5a1c60: x1 first 5ADB76; callee reads esi; ret 0; cleanup -
 0068AFA0 sub_68afa0: x1 first 5ADC2F; callee reads ebx,esi; ret 0x18; cleanup -
 007F49E0 lstr_get: x1 first 5ADD21; callee reads edi,esi; ret 0; cleanup -
 00C9604F __security_check_cookie: x1 first 5ADDF5; callee reads ecx; ret 0; cleanup -
INDIRECT CALLS: 0 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E5BF40: r1 w0 first 5AD3E2 fld sz8
 00E5BFA0: r1 w0 first 5ADD56 fmul sz8
 00E5C0D8: r5 w0 first 5ACA3A fmul sz8
 00E5C1A0: r1 w0 first 5AD3F2 fld sz8
 00E80B50: r1 w0 first 5AC6DC mov sz4
 00E843AC: r1 w0 first 5ACA01 mov sz4
 00E883A2: r0 w1 first 5ADCD1 mov sz1
 00E98104: r1 w0 first 5AC7C9 mov sz4
 00EC3ABC: r2 w4 first 5AC960 mov sz4
 00EC3AC0: r2 w0 first 5AC966 mov[idx] sz4
 0140E728: r1 w1 first 5ADBA9 mov sz4
 0140E72C: r1 w1 first 5ADBAE mov sz4
 0140E730: r1 w1 first 5ADBB4 mov sz4
 0140E734: r1 w1 first 5ADBA3 mov sz4
 0140E738: r1 w1 first 5ADBBE mov sz4
 021703D4: r14 w0 first 5AC8CC mov sz4
 0217043C: r9 w0 first 5AC8C0 mov sz4
 0223E964: r2 w4 first 5AD6E6 mov sz4
 02527584: r0 w1 first 5ADCD8 mov sz4
 02527588: r0 w1 first 5ADDCA mov sz4
 0252758C: r1 w1 first 5ADD17 mov sz4
 02527594: r1 w1 first 5AC8F9 mov[idx] sz4
 025275C4: r1 w0 first 5AD820 cmp sz1
 02528B49: r9 w0 first 5AC8B6 mov sz1
 0252A0D8: r1 w0 first 5AD35D mov sz4
 0252A3C8: r1 w0 first 5ADCC1 mov sz4
 0260B644: r3 w0 first 5ACC2F mov sz4
 0260B648: r3 w0 first 5ACC29 idiv sz4
 029E3F18: r1 w0 first 5ADCD8 addr-of sz4
 029E3F24: r0 w1 first 5ADCFB mov sz1
 029E3F25: r0 w1 first 5ADD02 mov sz1
 029E3F34: r0 w1 first 5ADD10 mov sz1
 029E3F38: r0 w1 first 5ADCE2 mov sz1
 029E3F3C: r0 w1 first 5ADCE9 mov sz4
 029E3F40: r0 w1 first 5ADCEE mov sz4
 029E3F44: r0 w1 first 5ADD09 mov sz1
 029E3F45: r0 w1 first 5ADCF4 mov sz1
 029E4018: r1 w0 first 5ADDCA addr-of sz4
 029E4024: r0 w1 first 5ADDE0 mov sz1
 029E4025: r0 w1 first 5ADDE7 mov sz1
 029E4034: r0 w1 first 5ADDEE mov sz1
 029E4038: r0 w1 first 5ADDBA mov sz4
 029E403C: r0 w1 first 5ADDC0 mov sz4
 029E4040: r0 w1 first 5ADDD4 mov sz4
 029E4044: r0 w1 first 5ADDD9 mov sz1
 029E40D8: r1 w0 first 5ADD17 addr-of sz4
 02A15768: r1 w0 first 5ADD31 mov sz4
 02A1576C: r1 w0 first 5ADD3B mov sz4
 02E7DF0C: r1 w0 first 5AD67B mov sz4
 02FAD290: r1 w0 first 5AD36B addr-of sz4
 033D6308: r1 w0 first 5ADD93 mov sz4
STRINGS
 DECA90 "Eligible_Events" @ 5AC802
 DECAA0 "Eligible_Event" @ 5AC832
 DEA60C "Event_Name" @ 5AC872
 DECAB0 "Disabled_In_Coop" @ 5AC89E
 DDC3BC "Duration" @ 5ACA28
 DECAC4 "Cash_Goal_Coop" @ 5ACA9A
 DECAD4 "Cash_Goal" @ 5ACAA1
 DECAE0 "Next_Celebrity_Delay" @ 5ACAB7
 DECAF8 "Celebrity_Annoyance_Tolerance" @ 5ACAFA
 DECB18 "Base_Annoyance_Refund" @ 5ACB0F
 DECB30 "Throw_Damage_Mult" @ 5ACB20
 DECB44 "Celebrities" @ 5ACB52
 DEBFDC "Celebrity" @ 5ACB86
 DECB50 "Celeb_NPC_Type" @ 5ACBC6
 DECB60 "Celeb_Variant" @ 5ACC62
 DECB70 "Celeb_Persona" @ 5ACCA2
 DECB80 "Loop_Path" @ 5ACCE4
 DECB8C "Crazed_Fan_Spawn_Levels" @ 5ACD76
 DECBA4 "Crazed_Fan_Spawn_Level" @ 5ACDB4
 DD7A5C "Start" @ 5ACE12
 DECBE8 "Spawn_Interval" @ 5ACE97
 DECBF8 "Max_Crazed_Fans" @ 5ACEA5
 DECC08 "Spawn_Dist_Min" @ 5ACEFF
 DECC18 "Spawn_Dist_Max" @ 5ACF17
 DECC28 "Health_Modifier" @ 5ACF2B
 DECC38 "Spawn_Chances" @ 5ACF52
 DECBBC "Spawn_Interval_Coop" @ 5ACF7E
 DECBD0 "Max_Crazed_Fans_Coop" @ 5ACF8C
 DECC48 "Spawn_Chance" @ 5ACFA7
 DECC58 "Crazed_Fans_Spawned" @ 5ACFF1
 DDDB54 "Chance" @ 5AD010
 DECC6C "Fan_Type_Chances" @ 5AD082
 DECC80 "Fan_Type_Chance" @ 5AD0B2
 DD92D4 "Type" @ 5AD123
 DECC90 "Lose_Interest_Delay" @ 5AD18F
 DECCA4 "Annoyance" @ 5AD1CE
 DECCB0 "Max_Annoyance" @ 5AD1EE
 DE3B74 "Weapon" @ 5AD226
 DE5A64 "NPC" @ 5AD282
 DDAEC4 "Vehicle" @ 5AD326
 DECCC0 "Spawnable_Fans" @ 5AD4C2
 DECCD0 "Spawnable_Fan" @ 5AD4F2
 DECCE0 "Character_Preset" @ 5AD528
 DECCF4 "level" @ 5AD61B
 DECCFC "crowd_control_" @ 5AD620
 DE2F90 "%s%s_#%i" @ 5AD625
 DECD0C "bean_bag_gun" @ 5AD830
 DECD34 "ACT_SECURITY_ANNOYANCE" @ 5ADCCC
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C0D8] fmul @ 5ACA3A: f64=1000.0 bits=0x408f400000000000
 [E5BF40] fld @ 5AD3E2: f64=1.0 bits=0x3ff0000000000000
 [E5C1A0] fld @ 5AD3F2: f64=9.999999747378752e-05 bits=0x3f1a36e2e0000000
 [E5BFA0] fmul @ 5ADD56: f64=0.0 bits=0x0000000000000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 5AD3F8 fcom st(1) ; test ah,0x41; jne @ 5AD3FF -> 0x5ad410: taken when ST0<op | ST0==op | unordered
 5AD405 fcompp  ; test ah,0x5; jnp @ 5AD40C -> 0x5ad439: taken when ST0<op
 5ADD77 fucomp st(2) ; test ah,0x44; jnp @ 5ADD80 -> 0x5add9e: taken when ST0==op
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 5AC9EF setg al   (prev: cmp dword ptr [eax + ecx + 0x1598], 0)
JUMP TABLES
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 5ac7d0: eax < (signed) ebx -> 0x5ac7e6 [jl]
 5ac7dc: eax >= (signed) dword ptr [edx + 0x111ac] -> 0x5ac7e6 [jge]
 5ac8f7: eax < (signed) 0 -> 0x5ac924 [jl]
 5ac906: ecx > (signed) edx -> 0x5ac924 [jg]
 5ac95e: dword ptr [esp + 0x20] <= (signed) esi -> 0x5ac996 [jle]
 5ac97c: ecx < (signed) 0x800 -> 0x5ac984 [jl]
 5ac9bc: ecx < (signed) 0x800 -> 0x5ac9c8 [jl]
 5aca16: eax <= (signed) 0x6b49d200 -> 0x5aca23 [jle]
 5acd35: eax < (signed) 3 -> 0x5acbb5 [jl]
 5ace0f: eax <= (signed) 0 -> 0x5ace65 [jle]
 5acec1: eax < (signed) 0x40 -> 0x5acec8 [jl]
 5ad05c: eax < (signed) 6 -> 0x5acfcc [jl]
 5ad183: edi < (signed) 6 -> 0x5ad140 [jl]
 5ad201: eax < (signed) 2 -> 0x5ad386 [jl]
 5ad20a: eax <= (signed) 3 -> 0x5ad26c [jle]
 5ad367: edx <= (signed) 0 -> 0x5ad380 [jle]
 5ad37e: ecx < (signed) edx -> 0x5ad370 [jl]
 5ad3d8: eax < (signed) 6 -> 0x5ad0f0 [jl]
 5ad41c: dword ptr [ebx + 0x2144] <= (signed) ecx -> 0x5ad439 [jle]
 5ad437: ecx < (signed) dword ptr [ebx + 0x2144] -> 0x5ad424 [jl]
 5ad482: eax < (signed) 6 -> 0x5acde0 [jl]
 5ad614: eax < (signed) 4 -> 0x5ad517 [jl]
 5ada9a: edi < (signed) 4 -> 0x5ada87 [jl]
 5adab1: edi < (signed) 4 -> 0x5adaa0 [jl]
 5adadb: ecx <= (signed) 0 -> 0x5adb52 [jle]
 5adb50: eax < (signed) dword ptr [esp + 0x3c] -> 0x5adaf0 [jl]
 5adc76: dword ptr [ecx + edi + 0x1710] <= (signed) esi -> 0x5adcbd [jle]
 5adcbb: esi < (signed) dword ptr [edx + edi + 0x1710] -> 0x5adc80 [jl]
 5add39: eax < (unsigned) edx -> 0x5add47 [jb]
 5add45: eax < (unsigned) edx -> 0x5add8b [jb]

```


### 4. 0x580A40 `vehicle_protect_objective_update_body` (5170 bytes)

Convention: register-only call from `vehicle_protect_objective::update` (0x57EDB0); `ret 4` (one stack arg). EBP = the objective (kept; `[EBP+0x40]` is a holder object, `[EBP+0x3C8]` weapon name id, `[EBP+0x3D8 + i*4]` / `[EBP+0x400 + i*4]` / `[EBP+0x428 + i*4]` three arrays of spawned enemy handles, `[EBP+0x4C0]`, `[EBP+0x4C8]` counters).

Sections:
1. 0x580A40-0x580BB3 gate: seven virtual `holder->get()` calls (`mov eax,[ecx]; mov edx,[eax+0x2C/0x34]; call edx`: +0x2C = is_valid, +0x34 = get_object) on the protected vehicle holder, `sub_AB55B0`, `sub_AAB370` (EDX = vehicle), `sub_4AB370`; then `jmp [ecx*4 + 0x581E74]` on the phase `[EBP+?]` (5 entries: 0x580BBA, 0x580C4A, 0x580DE1, 0x58109F, 0x581A3D).
2. phase 0 (0x580BBA): clear the vehicle speed override (`vehicle_ai_clear_speed_override`), `sub_B44110`, `sub_AA6100`, `sub_580780`, timer check `timer_elapsed`, `net_send_veh_pkt`, set phase.
3. phase 1 (0x580C4A): `sub_AA6100`, `vehicle_ai_clear_speed_override`, `net_send_veh_pkt`, `sub_580780`; distance (`sqrtf_wrapper`) between the player and the vehicle; `sub_5759D0`, `sub_5807B0`; `ret 4`.
4. phase 2 (0x580DE1): `sub_B19060`, `vec3_distance_sq` against the drop-off, `vehicle_set_force_flag_unenterable`, `sub_443880`, `sub_41CB40`, `human_in_vehicle`, `object_teleport_prepare`, `sub_99DD20`, timers (`timer_elapsed` with `[EBP+0x134]`), `vehicle_ai_set_speed_override`, `vehicle_ai_pathfind_nodes`.
5. phase 3 (0x58109F): enemy spawn: `weapon_info_find_by_name([0x252A6D8 table], [EBP+0x3C8])`, `sub_AC7420(.., "SS03")`, `sub_955D00` (spawn human at a navpoint), `sub_4A9A60` / `sub_BE2C60` (position lookups), `sub_5E13D0`, `net_send_veh_pkt_5`, `npc_set_force_flag_dont_affect_notoriety`, `net_send_prop_human_ai_data_forced_target_handle`, `sub_A62710` / `sub_A600C0` / `sub_A26380` / `sub_A61940` (script-object creation, `string_crc32`), `human_pkt_send_3C`, HUD arrow `sub_7A2030(.., "icon_lg_d")`, `effect_find_by_name`, `sub_92D1A0`, `sub_92DA30`; two spawn loops write handles into the three arrays above (the second copy at 0x58174B-0x581A3D is the same code for the second wave, index base `[EBP+0x4C8]`).
6. phase 4 (0x581A3D): finish: `sub_575860`, trigger updates `sub_A69D60` x2, `coop_partner_human`, distance checks (`vec3_distance_sq_dup`), `human_is_player_or_player_follower`, `human_set_party_leader`, message `act_heli_text` (0x581C72), `sub_79D770`, `sub_79C390/79C150`, `sub_BDEA80`, `human_set_force_flag_invulnerable`, `human_take_damage`, `human_ai_set_force_flag_ignore_attacks`, cleanup `sub_7A2130`, `sub_92D4B0`, final `ret 4` at 0x581E6F (virtual call `[edx+0x50]` on EBP).

Port notes: the unknown helpers (`sub_AA6100`, `sub_B19060`, ...) are plain game functions with register ABIs listed in the table; the function is mostly "call these in this order under these flags", dominated by 82 virtual calls through holder vtables. Port it as straight-line Rust with `unsafe extern "C"` thunks; only 121 x87 instructions (distance and timer maths).


Generated tables (exact, from the bytes):

```
FUNCTION 580A40..581E72 size 5170 (1444 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x110; no /GS cookie
 esp+0x50 (lea edx @ 580D0D) passed to net_send_veh_pkt
 esp+0x3c (lea eax @ 580E76) passed to vec3_distance_sq
 esp+0x18 (lea edx @ 580EB2) passed to sub_443880
 esp+0x6c (lea eax @ 580ECB) passed to sub_41cb40
 esp+0x6c (lea edx @ 581020) passed to sub_b07090
 esp+0x78 (lea ecx @ 581296) passed to vehicle_ai_pathfind_nodes
 esp+0xa8 (lea edx @ 58135B) passed to sub_955d00
 esp+0x20 (lea ecx @ 5813AF) passed to sub_4a9a60
 esp+0x40 (lea edx @ 5813B8) passed to sub_4a9a60
 esp+0x94 (lea ecx @ 5813DE) passed to sub_be2c60
 esp+0x44 (lea ecx @ 581542) passed to string_crc32
 esp+0x24 (lea eax @ 581552) passed to string_crc32
 esp+0xcc (lea esi @ 58178A) passed to sub_4a9a60
 esp+0x70 (lea edi @ 581791) passed to sub_4a9a60
 esp+0x6c (lea ecx @ 581CCF) passed to sub_79c390
 esp+0x6c (lea edx @ 581CDE) passed to sub_79c150
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 00AB55B0 sub_ab55b0: x1 first 580AF0; callee reads esi; ret 0; cleanup -
 00AAB370 sub_aab370: x2 first 580B05; callee reads edx; ret 0; cleanup -
 004AB370 sub_4ab370: x2 first 580B6B; callee reads -; ret 0; cleanup -
 00B27530 vehicle_ai_clear_speed_override: x2 first 580BE3; callee reads eax; ret 0; cleanup -
 00B44110 sub_b44110: x1 first 580BFA; callee reads ecx; ret 0; cleanup -
 00AA6100 sub_aa6100: x4 first 580C0E; callee reads ecx,edx; ret 0; cleanup -
 00580780 sub_580780: x3 first 580C1A; callee reads eax,ecx; ret 0; cleanup -
 00BDD230 timer_elapsed: x3 first 580C5B; callee reads eax; ret 0; cleanup -
 00B28330 net_send_veh_pkt: x1 first 580D11; callee reads ecx,edx; ret 0; cleanup -
 0040EA20 sqrtf_wrapper: x1 first 580DAB; callee reads -; ret 0; cleanup -
 005759D0 sub_5759d0: x1 first 580DC6; callee reads ecx; ret 0; cleanup -
 005807B0 sub_5807b0: x1 first 580DCD; callee reads eax; ret 0; cleanup -
 00B19060 sub_b19060: x2 first 580E3F; callee reads edx,esi; ret 0; cleanup -
 00BDEA30 vec3_distance_sq: x1 first 580E86; callee reads eax,ecx; ret 0; cleanup -
 00ABA230 vehicle_set_force_flag_unenterable: x2 first 580EA9; callee reads -; ret 0x8; cleanup -
 00443880 sub_443880: x2 first 580EC6; callee reads -; ret 0x8; cleanup -
 0041CB40 sub_41cb40: x1 first 580ED9; callee reads eax,ecx; ret 0; cleanup -
 009B8C50 human_in_vehicle: x1 first 580F12; callee reads edi,esi; ret 0; cleanup -
 00447CD0 object_teleport_prepare: x2 first 580F28; callee reads -; ret 0; cleanup [12]
 00404A20 vec3_distance_sq_dup: x3 first 580F44; callee reads eax,ecx; ret 0; cleanup -
 0099DD20 sub_99dd20: x1 first 581009; callee reads eax,edx; ret 0; cleanup [16]
 00B07090 sub_b07090: x1 first 581027; callee reads -; ret 0; cleanup [12]
 00581E90 sub_581e90: x1 first 5811B3; callee reads -; ret 0x4; cleanup -
 00806720 human_owned_locally: x1 first 5811D2; callee reads eax; ret 0; cleanup -
 00806B90 net_request_ownership: x1 first 5811EA; callee reads -; ret 0; cleanup [12]
 00B274F0 vehicle_ai_set_speed_override: x1 first 58126B; callee reads eax,ecx; ret 0; cleanup -
 00B27D90 vehicle_ai_pathfind_nodes: x1 first 58129A; callee reads ecx; ret 0; cleanup -
 00B716E0 weapon_info_find_by_name: x1 first 58132E; callee reads -; ret 0; cleanup -
 00AC7420 sub_ac7420: x1 first 581350; callee reads eax,ecx; ret 0; cleanup -
 00955D00 sub_955d00: x2 first 581374; callee reads -; ret 0; cleanup [8]
 004A9A60 sub_4a9a60: x2 first 5813C9; callee reads eax; ret 0; cleanup -
 00BE2C60 sub_be2c60: x2 first 5813E7; callee reads eax,ecx; ret 0; cleanup -
 005E13D0 sub_5e13d0: x2 first 581407; callee reads eax,ecx; ret 0; cleanup -
 00B383A0 net_send_veh_pkt_5: x2 first 581488; callee reads -; ret 0; cleanup -
 005E0460 sub_5e0460: x2 first 5814B7; callee reads edi; ret 0; cleanup -
 009D2010 npc_set_force_flag_dont_affect_notoriety: x3 first 5814E3; callee reads -; ret 0x8; cleanup -
 00443740 net_send_prop_human_ai_data_forced_target_handle: x3 first 581512; callee reads -; ret 0x8; cleanup -
 00A62710 sub_a62710: x1 first 58151B; callee reads -; ret 0x4; cleanup -
 00A600C0 sub_a600c0: x1 first 58152F; callee reads eax,ecx,edi; ret 0; cleanup -
 00BDCA30 string_crc32: x1 first 581556; callee reads eax,edx; ret 0x4; cleanup -
 00A26380 sub_a26380: x1 first 58156E; callee reads -; ret 0; cleanup -
 00A61940 sub_a61940: x1 first 58157D; callee reads ecx; ret 0; cleanup -
 005E0150 sub_5e0150: x1 first 581627; callee reads eax,edx,esi; ret 0; cleanup -
 00558FB0 human_pkt_send_3c: x1 first 58165C; callee reads ecx,edx; ret 0xC; cleanup -
 007A2030 sub_7a2030: x2 first 581691; callee reads ecx; ret 0; cleanup -
 0050DB60 effect_find_by_name: x2 first 5816BA; callee reads ecx; ret 0; cleanup -
 0092D1A0 sub_92d1a0: x2 first 5816D6; callee reads ecx; ret 0; cleanup -
 0092DA30 sub_92da30: x2 first 581703; callee reads esi; ret 0; cleanup [12]
 00575860 sub_575860: x1 first 581A33; callee reads -; ret 0xC; cleanup -
 00A69D60 sub_a69d60: x2 first 581A58; callee reads eax,ecx,edi; ret 0; cleanup -
 009D7BD0 coop_partner_human: x1 first 581AAA; callee reads -; ret 0; cleanup -
 0098E8A0 human_is_player_or_player_follower: x2 first 581AF8; callee reads edx; ret 0; cleanup -
 0098EC10 human_set_party_leader: x2 first 581B61; callee reads edi; ret 0; cleanup [16]
 009B8CB0 sub_9b8cb0: x2 first 581C1C; callee reads eax,ecx; ret 0; cleanup -
 0079D770 sub_79d770: x2 first 581C4E; callee reads edx; ret 0; cleanup [4]
 00576D30 sub_576d30: x1 first 581C67; callee reads eax; ret 0; cleanup -
 00576E60 act_heli_text: x1 first 581C72; callee reads eax; ret 0; cleanup -
 0079C390 sub_79c390: x1 first 581CD7; callee reads edx; ret 0; cleanup -
 0079C150 sub_79c150: x1 first 581CE4; callee reads -; ret 0; cleanup -
 00BDEA80 sub_bdea80: x2 first 581CF5; callee reads eax,ecx; ret 0; cleanup -
 00965F40 human_set_force_flag_invulnerable: x2 first 581D2C; callee reads -; ret 0x8; cleanup -
 009847A0 human_take_damage: x1 first 581D91; callee reads ecx,edx; ret 0; cleanup -
 00441E60 human_ai_set_force_flag_ignore_attacks: x2 first 581DBD; callee reads -; ret 0x8; cleanup -
 007A2130 sub_7a2130: x2 first 581E14; callee reads esi; ret 0; cleanup -
 0092D4B0 sub_92d4b0: x2 first 581E2D; callee reads -; ret 0; cleanup -
INDIRECT CALLS: 82 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E38AE0: r1 w0 first 581CA1 fld sz4
 00E5BF40: r2 w0 first 581D37 fadd sz8
 00E5C190: r1 w0 first 580DB0 fcomp sz8
 00E5C408: r1 w0 first 580F81 fld sz8
 00E5C540: r2 w0 first 581B20 fcomp sz8
 00E5CA98: r1 w0 first 581205 fmul sz8
 00E843AC: r2 w0 first 580C1F mov sz4
 00E87998: r1 w0 first 581270 fld sz4
 00E9802C: r2 w0 first 581D1C fcomp sz4
 00EAD208: r1 w0 first 581CA7 mov sz4
 00EC5AE4: r2 w0 first 5816D1 addr-of sz4
 01408E88: r3 w0 first 580DC0 mov sz4
 0140A430: r1 w0 first 581122 mov sz4
 021703D4: r4 w0 first 580D6B mov sz4
 0252755C: r1 w0 first 58131C cmp sz1
 0252755D: r1 w0 first 5811A9 cmp sz1
 0252A6D0: r2 w0 first 580D34 mov sz4
 0252A6D4: r2 w0 first 580D3F mov sz4
 0252A6D8: r2 w0 first 580D4B mov sz4
 033D6308: r3 w0 first 581526 addr-of sz4
STRINGS
 DE8280 "mcmanus2010" @ 581329
 DE9E40 "SS03" @ 58134B
 E1D454 "homies" @ 58154B
 DE8428 "icon_lg_d" @ 5816B5
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C190] fcomp @ 580DB0: f64=100.0 bits=0x4059000000000000
 [E5C408] fld @ 580F81: f64=1.5 bits=0x3ff8000000000000
 [E5CA98] fmul @ 581205: f64=0.44703999161720276 bits=0x3fdc9c4da0000000
 [E87998] fld @ 581270: f32=-1.0 bits=0xbf800000
 [DD04F4] fld @ 5816DB: f32=1.5 bits=0x3fc00000
 [E5C540] fcomp @ 581B20: f64=225.0 bits=0x406c200000000000
 [E38AE0] fld @ 581CA1: f32=3.0 bits=0x40400000
 [E9802C] fcomp @ 581D1C: f32=10000.0 bits=0x461c4000
 [E5BF40] fadd @ 581D37: f64=1.0 bits=0x3ff0000000000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 580DB0 fcomp qword ptr [0xe5c190] ; test ah,0x5; jp @ 580DBE -> 0x580dcb: taken when ST0>op | ST0==op | unordered
 580E91 fcompp  ; test ah,0x5; jp @ 580E98 -> 0x580ea6: taken when ST0>op | ST0==op | unordered
 580F5D fcompp  ; test ah,0x1; jne @ 580F64 -> 0x580f6a: taken when ST0<op | unordered
 581B20 fcomp qword ptr [0xe5c540] ; test ah,0x5; jp @ 581B2B -> 0x581b69: taken when ST0>op | ST0==op | unordered
 581B74 fcomp qword ptr [0xe5c540] ; test ah,0x5; jp @ 581B7F -> 0x581bc4: taken when ST0>op | ST0==op | unordered
 581D1C fcomp dword ptr [0xe9802c] ; test ah,0x41; jne @ 581D27 -> 0x581d9b: taken when ST0<op | ST0==op | unordered
 581D5D fcomp dword ptr [esp + 0x20] ; test ah,0x5; jp @ 581D66 -> 0x581d9b: taken when ST0>op | ST0==op | unordered
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 580AAE sete bl   (prev: mov eax, dword ptr [edx + 0x34])
 580ACD sete al   (prev: mov edi, 4)
 581565 sete dl   (prev: cmp ecx, dword ptr [eax])
JUMP TABLES
 jmp @ 580BB3 table 581E74 entries(5): 580BBA 580C4A 580DE1 58109F 581A3D
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 580bad: ecx > (unsigned) edi -> 0x580d66 [ja]
 580c34: eax <= (signed) 0x6b49d200 -> 0x580d66 [jle]
 580c67: dword ptr [edx] >= (signed) 0 -> 0x580d66 [jge]
 58106a: eax <= (signed) 0x6b49d200 -> 0x581077 [jle]
 581088: eax <= (signed) 0x6b49d200 -> 0x581095 [jle]
 5810d0: dword ptr [ebp + 0x12c] > (signed) 0 -> 0x5810ef [jg]
 5811c2: ecx >= (unsigned) eax -> 0x580d66 [jae]

```


### 5. 0x5CC540 `fuzz_crime_hooker_revolt::update` (4797 bytes)

Convention: thiscall, ECX = crime (stored `[esp+0x20]`), no stack args, plain `ret`. Frame `sub esp,0x59C` with a /GS cookie. `fuzz_crime` slot 1 override; ends with `call fuzz_crime::update (0x5C3BB0)` (ECX = this, from `[esp+0x20]`) at 0x5CD7E3.

Sections:
1. 0x5CC540-0x5CC565 `if ((s32)[this+0x24] < 3) goto 0x5CD7DF` (crime state below 3: nothing to do except the base update).
2. 0x5CC570-0x5CC5BF choose the reference human: `ref = coop_partner_human()`; if a partner exists, `d1 = vec3_distance_sq_dup(this+0x10, player+0x14)`, `d2 = vec3_distance_sq_dup(this+0x10, partner+0x14)`; `fcompp` then `test ah,5; jp` jumps when ST0>=ST1 or unordered, i.e. the partner replaces `ref` (`[esp+0x18]`) only when `d_player < d_partner` is FALSE... exact: ST0 = `d_partner` (loaded second), ST1 = `d_player`; the jump is taken (keep the player) when `d_partner >= d_player` or unordered; otherwise `ref = coop_partner_human()`.
3. 0x5CC5E0-0x5CD7DF participant loop, unrolled three times (the three copies start at 0x5CC5E0, 0x5CCA38 and 0x5CCF4A; each copy is selected by `[esp+0x24]`): `h = [this+0xFC + i*4]->vtable[0x34/4]()` (participant holder get); skip when `h == 0` or the object is not alive (`[h+0x1124]` hit points: `fldz; fcomp [h+0x1124]; test ah,1; je` = jump when 0 >= hp, i.e. dead, `ja 0x5CCEDF`); skip when `[h+0x149] != 0` (state byte).
   - If `human_is_dead_or_invalid(EDX = victim)` is false for the current target: `human_owned_locally(EAX = h, ECX = victim handle?)`; if the human is not locally owned, build and send a net packet (`session_find_conn_by_id`, `net_send_packet(ecx, edx, 0, 0)`, 4 stack args, `add esp,0x10`) and bail.
   - Otherwise: `object_teleport_prepare(h, 0, 0)`, `sub_96D880`, `net_send_human_pkt_6(1, ecx, 0)`, `human_pkt_send_21(h)`, `sub_5DDA00` x2 (fight club handle test, inert for non-fight-club), `object_is_player_human`, `sub_97F500`, `sub_9654B0(h)`, `sub_438A90`, `sub_962F50`, `human_is_dead(eax)`, `human_from_handle_active(1)` of `[h+0xAF0]` (current attack target), `sub_415B40`, `human_is_local_player`, `sub_43B320`, `sub_9CE0A0`, `human_vehicle_interaction_entering(h)` on `[h+0x108]`, `sub_98E250`, weapon choice: `human_ai_choose_weapon(h, target)` with `[h+0x110C]`, `weapon_info_is_melee(weapon+8)`, `ai_human_leader_flag4`, `weapon_info_is_firearm`, `sub_42C920(-1, -1)`.
   - Range test: `weapon_info_is_firearm` -> `weapon_info_ai_range_sq`; `fcomp [esp+0x1c]` (`test ah,5; jp`: taken for range_sq >= dist_sq or unordered) then `vec3_distance_sq` and a second `fcomp st(1)` (`test ah,1; jne`: taken for ST0<ST1 or unordered); when in range: `[h+0xAF0] = new target handle`, `ai_order_k04_payload_init(ECX = payload)`, `ai_order_set(h, 4, payload)` (order 4 = attack target).
4. 0x5CD7DF `ECX = this; call fuzz_crime::update`; cookie check; `ret`.

Port notes: all five of `sub_5DDA00` calls are `fight_club_is_fighter_handle(ESI = handle)` (inert here, but must be called: it only reads). The three blocks differ only in which participant slot they use, so a loop is behaviour-identical as long as the order of calls (and the per-block packet sends) is kept.


Generated tables (exact, from the bytes):

```
FUNCTION 5CC540..5CD7FD size 4797 (1377 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x59c; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0x94 (lea ecx @ 5CC7CF) passed to net_send_packet
 esp+0x28 (lea ecx @ 5CCEA5) passed to ai_order_k04_payload_init
 esp+0x2c (lea ecx @ 5CD7C4) passed to ai_order_set
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 009D7BD0 coop_partner_human: x3 first 5CC574; callee reads -; ret 0; cleanup -
 00404A20 vec3_distance_sq_dup: x2 first 5CC596; callee reads eax,ecx; ret 0; cleanup -
 009855F0 human_is_dead_or_invalid: x1 first 5CC64E; callee reads edx; ret 0; cleanup -
 00806720 human_owned_locally: x4 first 5CC6A0; callee reads eax; ret 0; cleanup -
 0080FE80 session_find_conn_by_id: x2 first 5CC7C2; callee reads eax,edx; ret 0; cleanup -
 00814670 net_send_packet: x2 first 5CC7D9; callee reads esi; ret 0; cleanup -
 00447CD0 object_teleport_prepare: x4 first 5CC7F1; callee reads -; ret 0; cleanup [12]
 0096D880 sub_96d880: x4 first 5CC800; callee reads eax; ret 0; cleanup -
 009707C0 net_send_human_pkt_6: x4 first 5CC819; callee reads ecx; ret 0; cleanup [12]
 00403E50 human_pkt_send_21: x4 first 5CC82F; callee reads ecx; ret 0; cleanup [4]
 005DDA00 fight_club_is_fighter_handle: x8 first 5CC83F; callee reads esi; ret 0; cleanup -
 00963540 object_is_player_human: x4 first 5CC865; callee reads eax; ret 0; cleanup -
 0097F500 sub_97f500: x4 first 5CC874; callee reads eax,ecx; ret 0; cleanup -
 009654B0 sub_9654b0: x3 first 5CC87E; callee reads -; ret 0; cleanup [4]
 00438A90 sub_438a90: x4 first 5CC89F; callee reads eax; ret 0; cleanup -
 00962F50 sub_962f50: x4 first 5CC8B4; callee reads eax; ret 0; cleanup -
 00985630 human_is_dead: x4 first 5CC8D8; callee reads -; ret 0; cleanup [4]
 00438710 human_from_handle_active: x3 first 5CC906; callee reads eax; ret 0; cleanup [4]
 00415B40 sub_415b40: x4 first 5CC929; callee reads eax,ecx; ret 0; cleanup -
 00926600 human_is_local_player: x4 first 5CC93A; callee reads ecx; ret 0; cleanup -
 0043B320 sub_43b320: x4 first 5CC945; callee reads -; ret 0; cleanup [8]
 009CE0A0 sub_9ce0a0: x4 first 5CC95D; callee reads eax,ecx; ret 0; cleanup -
 009B9320 human_vehicle_interaction_entering: x4 first 5CC982; callee reads -; ret 0; cleanup [4]
 0098E250 sub_98e250: x4 first 5CC997; callee reads -; ret 0; cleanup [4]
 0043A5C0 human_ai_choose_weapon: x8 first 5CC9B4; callee reads eax,ecx; ret 0; cleanup [4]
 00B72740 weapon_info_is_melee: x4 first 5CC9CD; callee reads eax; ret 0; cleanup -
 00439660 ai_human_leader_flag4: x4 first 5CC9F6; callee reads eax; ret 0; cleanup -
 00B72800 weapon_info_is_firearm: x7 first 5CCA14; callee reads eax; ret 0; cleanup -
 0042C920 sub_42c920: x7 first 5CCA2B; callee reads ecx,esi; ret 0; cleanup [8]
 00994340 human_pkt_send_09: x2 first 5CCC51; callee reads ecx; ret 0; cleanup -
 00964C20 sub_964c20: x2 first 5CCC69; callee reads edx; ret 0; cleanup [52]
 0043A620 human_ai_order_target_human: x3 first 5CCE19; callee reads eax; ret 0; cleanup -
 00B71A60 weapon_info_ai_range_sq: x3 first 5CCE50; callee reads eax,ecx; ret 0; cleanup -
 00BDEA30 vec3_distance_sq: x3 first 5CCE70; callee reads eax,ecx; ret 0; cleanup -
 004097F0 ai_order_k04_payload_init: x3 first 5CCEB0; callee reads -; ret 0; cleanup -
 004377A0 ai_order_set: x2 first 5CCECF; callee reads -; ret 0; cleanup [16]
 005C3BB0 fuzz_crime::update: x1 first 5CD7E3; callee reads ecx; ret 0; cleanup -
 00C9604F __security_check_cookie: x1 first 5CD7F4; callee reads ecx; ret 0; cleanup -
INDIRECT CALLS: 2 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E5C2E0: r3 w0 first 5CCE79 fmul sz8
 00E80B50: r1 w0 first 5CC54C mov sz4
 00E8BBCC: r4 w0 first 5CC6AD cmp sz1
 021703D4: r2 w0 first 5CC56B mov sz4
 02528C14: r2 w0 first 5CC7BD mov sz4
STRINGS
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [DD0BB4] fld @ 5CC809: f32=0.20000000298023224 bits=0x3e4ccccd
 [E5C2E0] fmul @ 5CCE79: f64=0.800000011920929 bits=0x3fe99999a0000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 5CC5AD fcompp  ; test ah,0x5; jp @ 5CC5B4 -> 0x5cc5bf: taken when ST0>op | ST0==op | unordered
 5CC62C fcomp dword ptr [edi + 0x1124] ; test ah,0x1; je @ 5CC637 -> 0x5ccedf: taken when ST0>op | ST0==op
 5CCE5B fcomp dword ptr [esp + 0x1c] ; test ah,0x5; jp @ 5CCE64 -> 0x5cce9e: taken when ST0>op | ST0==op | unordered
 5CCE7F fcompp  ; test ah,0x1; jne @ 5CCE86 -> 0x5cce9e: taken when ST0<op | unordered
 5CD2D7 fcomp dword ptr [esp + 0x1c] ; test ah,0x5; jp @ 5CD2E0 -> 0x5cd320: taken when ST0>op | ST0==op | unordered
 5CD2F7 fcompp  ; test ah,0x1; jne @ 5CD2FE -> 0x5cd320: taken when ST0<op | unordered
 5CD75D fcomp dword ptr [esp + 0x1c] ; test ah,0x5; jp @ 5CD766 -> 0x5cd7a4: taken when ST0>op | ST0==op | unordered
 5CD781 fcompp  ; test ah,0x1; jne @ 5CD788 -> 0x5cd7a4: taken when ST0<op | unordered
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
JUMP TABLES
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 5cc565: dword ptr [ecx + 0x24] < (signed) 3 -> 0x5cd7df [jl]
 5cce15: eax > (unsigned) 1 -> 0x5cce28 [ja]
 5cceed: eax < (unsigned) 5 -> 0x5cc5e0 [jb]
 5cd28c: eax > (unsigned) 1 -> 0x5cd2a4 [ja]
 5cd712: eax > (unsigned) 1 -> 0x5cd72a [ja]

```


### 6. 0x5934B0 `piracy_level_instance::load_from_table` (4794 bytes)

Convention: thiscall, ECX = level instance (EDI/EBP), stack args `(a, b, c)` forwarded to the base loader, `ret 0xC`; /GS cookie. Slot 0.

Sections (string order = parse order):
1. 0x5934B0-0x5934D3 base `activity_level_instance::load_from_table (0x53F700)` with the three stack args.
2. 0x5934D3-0x593648 sound / vehicle names: `SFX_PIRACY_ALARM` (`string_hash_lower` -> `audio_foley_find_by_hash`), `Watercraft`, `Start_Location`, `End_Location` (`string_crc32` -> `navpoint_find`).
3. 0x593648-0x593666 `Time` int (`xtbl_get_int`, ESI = `[esp+0x14]`) and `[0x29E3F38] = 0`.
4. 0x5936C6-0x593963 `Attack_Helicopters` / `Helicopter` children, each with a `Pivot` navpoint, `Pivot_Y_Offset`, `Circle_Time`, `Circle_Theta`, `Major_Radius`, `Minor_Radius` (floats into `[esp+0x7C..0x8C]`), bool `CCW`. The circle maths use `cosf_wrapper`/`sinf_wrapper`-style x87 (177 x87 instructions in the function) after the loader to precompute the orbit; see the constants table.
5. 0x593963-0x593B49 `Chunk_Swap` / `Swap` (`Swap_Collection`, `Swap_End`, `Swap_Name`).
6. 0x593B49-0x593DD7 `Boarding_Data`: `Guard_Weapons` / `Weapons` / `Weapon` (names `npc_cop` default, `Dropped_Item`), ints `First_Notoriety` -> `[this+0x4A4]`, `Second_Notoriety` -> `[this+0x4A8]`.
7. 0x593DD7-0x594003 location prefixes: `Guard_Location_Prefix`, `Package_Spawn_Location`, `Boarding_Location`, `Alarm_Location`, `Waiting_Cone`, `Waiting_Vehicle`, `Waiting_NPC`.
8. 0x594003-0x59476A `Return_Attackers` / `Return_Attacker`: `Attacker_Preset`, `Attacker_Boat`, `Attacker_Weapon`, `Attacker`, `Navpoint`; defaults `boat_wavecraft01` (0x5942C2) and `Gal43` (0x5942F3) are used when a name is missing.
9. 0x59476A `ret 0xC` after the cookie check.

Port notes: structure is a nested for-each over xtbl children with `__stricmp` name tests (102 calls, 8 virtual); all fields are written into the level instance, so a field table plus the five hand-written loops (helicopters, swaps, guard weapons, return attackers, locations) is enough. Keep the order of `string_crc32` / `navpoint_find` calls (they have no side effects but the xtbl children are consumed in order).


Generated tables (exact, from the bytes):

```
FUNCTION 5934B0..59476A size 4794 (1503 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0xcc; no /GS cookie
 esp+0x14 (lea eax @ 593524) passed to string_hash_lower
 esp+0xc8 (lea eax @ 59358E) passed to string_crc32
 esp+0xcc (lea eax @ 593634) passed to string_crc32
 esp+0x14 (lea esi @ 59364D) passed to xtbl_get_int
 esp+0xc4 (lea eax @ 593763) passed to string_crc32
 esp+0x7c (lea esi @ 59377A) passed to xtbl_get_float
 esp+0x80 (lea esi @ 593790) passed to xtbl_get_float
 esp+0x84 (lea esi @ 5937A5) passed to xtbl_get_float
 esp+0x88 (lea esi @ 5937BA) passed to xtbl_get_float
 esp+0x8c (lea esi @ 5937CF) passed to xtbl_get_float
 esp+0x8c (lea edx @ 5937DE) passed to xtbl_get_bool
 esp+0x28 (lea eax @ 593AD2) passed to string_crc32
 esp+0x24 (lea ecx @ 593ADD) passed to sub_a7c4f0
 esp+0x18 (lea ecx @ 593CD9) passed to string_hash_lower
 esp+0xb0 (lea eax @ 593E3F) passed to string_crc32
 esp+0xb8 (lea eax @ 593E8C) passed to string_crc32
 esp+0xc0 (lea eax @ 593EDB) passed to string_crc32
 esp+0xac (lea eax @ 593F2B) passed to string_crc32
 esp+0xd0 (lea eax @ 593F7B) passed to string_crc32
 esp+0xd8 (lea eax @ 593FCB) passed to string_crc32
 esp+0xc (lea edx @ 594061) passed to string_hash_lower
 esp+0xb0 (lea edx @ 5940BA) passed to string_hash_lower
 esp+0xd4 (lea eax @ 5941DB) passed to string_crc32
 esp+0x1c (lea eax @ 594248) passed to string_hash_lower
 esp+0xb8 (lea edx @ 5942B8) passed to string_hash_lower
 esp+0x54 (lea ecx @ 5944C4) passed to vehicle_create
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0053F700 activity_level_instance::load_from_table: x1 first 5934D3; callee reads ecx; ret 0xC; cleanup -
 009D7BD0 coop_partner_human: x1 first 5934D8; callee reads -; ret 0; cleanup -
 00D14840 gfx_set_color: x1 first 5934FE; callee reads -; ret 0; cleanup [12]
 004D1090 screen_fade_start: x1 first 593512; callee reads -; ret 0; cleanup [12]
 0078DAD0 sub_78dad0: x1 first 59351F; callee reads eax; ret 0; cleanup -
 00BDCB30 string_hash_lower: x6 first 593530; callee reads eax,ecx; ret 0x4; cleanup -
 00479A50 audio_foley_find_by_hash: x1 first 593537; callee reads edx; ret 0; cleanup -
 00BFBEE0 xml_find_child: x4 first 593555; callee reads eax,edi; ret 0; cleanup -
 00AE4090 vehicle_info_find: x1 first 593565; callee reads ecx; ret 0; cleanup -
 00BDCA30 string_crc32: x11 first 593595; callee reads eax,edx; ret 0x4; cleanup -
 00A5EB50 navpoint_find: x10 first 59359A; callee reads eax; ret 0; cleanup -
 00591AB0 sub_591ab0: x2 first 593612; callee reads esi; ret 0x10; cleanup -
 00B747D0 xtbl_get_int: x3 first 593666; callee reads ecx,esi; ret 0; cleanup [4]
 00C99030 __stricmp: x32 first 5936CC; callee reads -; ret 0; cleanup [8]
 00BFC110 xml_count_children: x1 first 5936E8; callee reads eax; ret 0; cleanup [4]
 00B74EF0 xtbl_get_float: x5 first 593782; callee reads ecx,esi; ret 0; cleanup [4]
 00B75050 xtbl_get_bool: x1 first 5937EC; callee reads ecx; ret 0; cleanup -
 0040B820 cosf_wrapper: x1 first 59381B; callee reads -; ret 0; cleanup -
 00428CD0 sinf_wrapper: x1 first 59382B; callee reads -; ret 0; cleanup -
 00591DD0 sub_591dd0: x1 first 593918; callee reads ecx,edi; ret 0x1C; cleanup -
 00C968F6 _sprintf: x2 first 593A18; callee reads -; ret 0; cleanup [12]
 00A7C4F0 sub_a7c4f0: x1 first 593AF8; callee reads -; ret 0; cleanup -
 00B716E0 weapon_info_find_by_name: x3 first 593C89; callee reads -; ret 0; cleanup [4]
 00C990F0 _strncpy: x2 first 593D8C; callee reads -; ret 0; cleanup [12]
 004A5F60 sub_4a5f60: x1 first 594074; callee reads -; ret 0x8; cleanup -
 00AE2A00 vehicle_create: x2 first 5944D1; callee reads eax,ecx; ret 0; cleanup [44]
 00A6FE60 sub_a6fe60: x1 first 594510; callee reads ecx,esi; ret 0; cleanup -
 009DB480 player_teleport: x2 first 59452F; callee reads ecx; ret 0; cleanup [24]
 00AEFC40 sub_aefc40: x1 first 5945A8; callee reads eax,ecx; ret 0; cleanup -
 0068A160 sub_68a160: x1 first 59474A; callee reads eax,ecx; ret 0; cleanup -
 00596440 sub_596440: x1 first 59475C; callee reads ecx; ret 0; cleanup -
INDIRECT CALLS: 8 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E5BF78: r1 w0 first 593806 fdivr sz8
 00E5BFE8: r2 w0 first 594447 fld sz8
 00E5BFF8: r2 w0 first 594482 fadd sz8
 00E5C130: r2 w0 first 5945FE fld sz8
 00E5C190: r1 w0 first 5935E7 fadd sz8
 00E5C380: r1 w0 first 593854 fmul sz8
 00E5D688: r1 w0 first 593830 fld sz8
 00E843AC: r1 w0 first 59433B mov sz4
 00E8EBB4: r1 w0 first 594340 fld sz4
 00EAC198: r1 w0 first 5934F1 mov sz4
 00EAC19A: r1 w0 first 5934E8 movzx sz1
 00EAC19B: r1 w0 first 5934DD movzx sz1
 00EC5AE4: r2 w0 first 594523 addr-of sz4
 021703D4: r3 w0 first 59438C mov sz4
 02527578: r0 w1 first 5936B8 mov sz4
 0252757C: r0 w1 first 594386 mov sz4
 02528B48: r1 w0 first 594544 mov sz1
 02528B49: r1 w0 first 594557 mov sz1
 0252A0D8: r2 w0 first 5940C9 mov sz4
 0252A3C8: r1 w0 first 59366F mov sz4
 0260B644: r2 w0 first 593D1F mov sz4
 0260B648: r2 w0 first 593D19 idiv sz4
 029E3F18: r1 w0 first 5936B8 addr-of sz4
 029E3F24: r0 w1 first 59369B mov sz1
 029E3F25: r0 w1 first 5936A2 mov sz1
 029E3F34: r0 w1 first 5936A9 mov sz1
 029E3F38: r0 w1 first 59367B mov sz1
 029E3F3C: r0 w1 first 593682 mov sz4
 029E3F40: r0 w1 first 593687 mov sz4
 029E3F44: r0 w1 first 593694 mov sz1
 029E3F45: r0 w1 first 59368D mov sz1
 02FAD290: r2 w0 first 5940D7 addr-of sz4
STRINGS
 DEAC58 "SFX_PIRACY_ALARM" @ 59352B
 DEAC6C "Watercraft" @ 59354E
 DE2FD0 "Start_Location" @ 593578
 DEAC78 "End_Location" @ 5935A6
 DD9514 "Time" @ 593648
 DEAC88 "Attack_Helicopters" @ 5936C6
 DE560C "Helicopter" @ 5936E1
 DEAC9C "Pivot" @ 59374D
 DEACA4 "Pivot_Y_Offset" @ 593775
 DEACB4 "Circle_Time" @ 59378B
 DEACC0 "Circle_Theta" @ 5937A0
 DEACD0 "Major_Radius" @ 5937B5
 DEACE0 "Minor_Radius" @ 5937CA
 DEACF0 "CCW" @ 5937E7
 DEACF4 "Chunk_Swap" @ 593963
 DEAD00 "Swap" @ 5939A2
 DEAD08 "Swap_Collection" @ 5939D8
 DEAD18 "Swap_End" @ 593A32
 DEAD24 "Swap_Name" @ 593A92
 DEAD30 "Boarding_Data" @ 593B49
 DEAD40 "Guard_Weapons" @ 593B86
 DD7B70 "Weapons" @ 593BBA
 DE3B74 "Weapon" @ 593C62
 DEAD50 "npc_cop" @ 593CE0
 DEAD58 "Dropped_Item" @ 593D62
 DEAD68 "First_Notoriety" @ 593DA7
 DEAD78 "Second_Notoriety" @ 593DBB
 DEAD8C "Guard_Location_Prefix" @ 593DD7
 DEADA4 "Package_Spawn_Location" @ 593E16
 DEADBC "Boarding_Location" @ 593E63
 DEADD0 "Alarm_Location" @ 593EB2
 DEADE0 "Waiting_Cone" @ 593F02
 DEADF0 "Waiting_Vehicle" @ 593F52
 DEAE00 "Waiting_NPC" @ 593FA2
 DEAE0C "Return_Attackers" @ 594003
 DEAE20 "Attacker_Preset" @ 594039
 DEAE30 "Attacker_Boat" @ 594092
 DEAE40 "Attacker_Weapon" @ 594102
 DEAE50 "Return_Attacker" @ 594142
 DEAE60 "Attacker" @ 594176
 DE8584 "Navpoint" @ 5941B2
 DEAE6C "boat_wavecraft01" @ 5942C2
 DE9E38 "Gal43" @ 5942F3
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C190] fadd @ 5935E7: f64=100.0 bits=0x4059000000000000
 [E5BF78] fdivr @ 593806: f64=6.2831854820251465 bits=0x401921fb60000000
 [E5D688] fld @ 593830: f64=75.0 bits=0x4052c00000000000
 [E5C380] fmul @ 593854: f64=15.0 bits=0x402e000000000000
 [E8EBB4] fld @ 594340: f32=500.0 bits=0x43fa0000
 [E5BFE8] fld @ 594447: f64=2.0 bits=0x4000000000000000
 [E5BFF8] fadd @ 594482: f64=3.0 bits=0x4008000000000000
 [E5C130] fld @ 5945FE: f64=5.0 bits=0x4014000000000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 594506 setne al   (prev: cmp dword ptr [ecx + 0x4c], edi)
JUMP TABLES
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 5940d3: edx <= (signed) 0 -> 0x5940f0 [jle]
 5940ee: ecx < (signed) edx -> 0x5940e0 [jl]
 5942d6: edx <= (signed) 0 -> 0x5942f0 [jle]
 5942ee: ecx < (signed) edx -> 0x5942e0 [jl]
 594339: eax < (unsigned) 0x30 -> 0x594320 [jb]
 59436f: eax <= (signed) 0x6b49d200 -> 0x59437c [jle]

```


### 7. 0x585C60 `large_convoy_objective_update_body` (4628 bytes)

Convention: register-only call from `large_convoy_objective::update` (0x586EC0); no stack args, plain `ret`. EBP / ESI = objective; `[obj+0x34]` etc. holder objects; strings `icon_lg_d` (0x585FE6), `homies` (0x586259), `rpg_launcher` (0x58635E) and `ACT_HELI_KILL_CARS_START_MESSAGE` (0x586E3D).

Sections:
1. 0x585C60-0x586539 setup of the convoy: spawn / validity of the lead vehicles, weapon lookup `rpg_launcher`, gang members (`homies`) creation through `sub_A26380` / script-object helpers, HUD icon.
2. 0x586539 `jmp [eax*4 + 0x586E74]` (12 entries: 0x586540, 0x586580, 0x58659F, 0x5865BA, 0x5865CE, 0x5865F8, 0x58660D, 0x586622, 0x586633, 0x586648, 0x586C39, 0x586A1D): per-convoy-state handlers (waypoint progression, damage checks).
3. 0x586A16 `jmp [ecx*4 + 0x586E9C]` (2 entries 0x586C39 / 0x586A1D) indexed by the byte table at 0x586EA4 (`00 00 01 00 00 00 00 00 00 01 01 01 01 00` for indices 0..13: indices 2, 9, 10, 11, 12 select entry 1).
4. 0x586C39-0x586E74 completion: message `ACT_HELI_KILL_CARS_START_MESSAGE` via `lstr_get` and `sub_79C150`, final `ret`.

Port notes: only 62 x87 instructions and 57 direct calls; the bulk is branchy integer code (206 conditional jumps). Port from the call/compare tables; the two jump tables share the target 0x586C39 (common exit).


Generated tables (exact, from the bytes):

```
FUNCTION 585C60..586E74 size 4628 (1297 instructions)
STACK FRAME / STACK OBJECTS
 frame: no sub esp in prologue; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0x148 (lea ecx @ 585DE0) passed to sub_a25610
 esp+0x144 (lea ecx @ 585DF1) passed to sub_a70500
 esp+0x84 (lea ecx @ 585E05) passed to sub_a26380
 esp+0x68 (lea edx @ 585E0D) passed to sub_a26380
 esp+0x1c4 (lea ecx @ 585F04) passed to net_send_packet_to_list_or_remotes
 esp+0x58 (lea edx @ 585FD8) passed to string_hash_lower
 esp+0x108 (lea ecx @ 58615E) passed to sub_a25610
 esp+0x104 (lea ecx @ 58616F) passed to sub_a70500
 esp+0xac (lea eax @ 586183) passed to sub_a26380
 esp+0x78 (lea ecx @ 58618B) passed to sub_a26380
 esp+0x188 (lea ecx @ 58621E) passed to sub_a25610
 esp+0x180 (lea ecx @ 58622D) passed to sub_a620a0
 esp+0xd8 (lea eax @ 586260) passed to string_crc32
 esp+0x30 (lea eax @ 586348) passed to sub_aefc40
 esp+0xbfc (lea ecx @ 586513) passed to net_send_packet
 esp+0x6dc (lea ecx @ 586D04) passed to net_send_packet_to_list_or_remotes
 esp+0xd8 (lea edi @ 586E27) passed to lstr_get
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 00CA0820 __alloca_probe: x1 first 585C65; callee reads eax,ecx; ret 0; cleanup -
 00AE3F10 sub_ae3f10: x2 first 585DC1; callee reads eax; ret 0; cleanup [4]
 00A25610 sub_a25610: x3 first 585DE7; callee reads ecx,edx; ret 0; cleanup [8]
 00A70500 sub_a70500: x2 first 585DF8; callee reads ecx; ret 0x4; cleanup -
 00A26380 sub_a26380: x3 first 585E19; callee reads -; ret 0; cleanup -
 00A6FD40 sub_a6fd40: x2 first 585E24; callee reads ecx; ret 0; cleanup -
 005E0460 sub_5e0460: x2 first 585E5F; callee reads edi; ret 0; cleanup [12]
 008148C0 net_send_packet_to_list_or_remotes: x2 first 585F14; callee reads -; ret 0; cleanup [8]
 00BDCB30 string_hash_lower: x1 first 585FEB; callee reads eax,ecx; ret 0x4; cleanup -
 0092D1A0 sub_92d1a0: x2 first 586034; callee reads ecx; ret 0; cleanup -
 0092DA30 sub_92da30: x2 first 58605B; callee reads esi; ret 0; cleanup -
 00A620A0 sub_a620a0: x1 first 586234; callee reads ecx; ret 0; cleanup -
 00BDCA30 string_crc32: x1 first 586267; callee reads eax,edx; ret 0x4; cleanup -
 00A61940 sub_a61940: x1 first 58628E; callee reads ecx; ret 0; cleanup -
 009ACB10 sub_9acb10: x1 first 586340; callee reads ecx; ret 0; cleanup [8]
 00AEFC40 sub_aefc40: x1 first 58634C; callee reads eax,ecx; ret 0; cleanup -
 0091D380 sub_91d380: x1 first 586363; callee reads esi; ret 0; cleanup -
 00558FB0 human_pkt_send_3c: x1 first 58637D; callee reads ecx,edx; ret 0xC; cleanup -
 00814670 net_send_packet: x1 first 58651B; callee reads esi; ret 0; cleanup [16]
 0055AE30 sub_55ae30: x3 first 58658D; callee reads -; ret 0x14; cleanup -
 0055B200 sub_55b200: x1 first 5865C0; callee reads eax; ret 0; cleanup -
 00B72800 weapon_info_is_firearm: x1 first 5865E6; callee reads eax; ret 0; cleanup -
 0055B050 sub_55b050: x2 first 5865FF; callee reads -; ret 0x8; cleanup -
 0055B310 sub_55b310: x2 first 58663A; callee reads edi; ret 0x8; cleanup -
 00B07A30 vehicle_interaction_rider_find: x4 first 586733; callee reads esi; ret 0; cleanup -
 00960C30 sub_960c30: x1 first 586833; callee reads esi; ret 0; cleanup -
 0096CB70 human_anim_set_update: x1 first 58686C; callee reads -; ret 0; cleanup [20]
 00B6A8B0 weapon_lockon_reticule_update: x1 first 586928; callee reads ecx,edx; ret 0; cleanup -
 009C5140 sub_9c5140: x1 first 5869BF; callee reads eax; ret 0; cleanup -
 009C8C20 sub_9c8c20: x1 first 586A41; callee reads -; ret 0; cleanup [8]
 00B72D30 weapon_usable_from_vehicle: x1 first 586B0E; callee reads eax,edx; ret 0; cleanup [8]
 007975E0 sub_7975e0: x1 first 586C24; callee reads ecx; ret 0; cleanup -
 00798140 sub_798140: x1 first 586C2A; callee reads -; ret 0; cleanup [4]
 00558BE0 sub_558be0: x1 first 586C34; callee reads ecx,esi; ret 0; cleanup -
 007A1EA0 sub_7a1ea0: x1 first 586D2B; callee reads -; ret 0; cleanup -
 0050DB60 effect_find_by_name: x1 first 586D48; callee reads ecx; ret 0; cleanup -
 007F49E0 lstr_get: x1 first 586E4D; callee reads edi,esi; ret 0; cleanup -
 0079C150 sub_79c150: x1 first 586E53; callee reads -; ret 0; cleanup -
 00C9604F __security_check_cookie: x1 first 586E68; callee reads ecx; ret 0; cleanup [4356]
INDIRECT CALLS: 18 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E80B50: r1 w0 first 585C6A mov sz4
 00E843AC: r1 w0 first 586B48 mov sz4
 00E8BBB4: r2 w0 first 585E78 cmp sz1
 00E8BBCC: r1 w0 first 586412 cmp sz1
 00EAD208: r1 w0 first 586E1C mov sz4
 00EBCE9C: r2 w0 first 586712 mov sz4
 00EBCEA0: r2 w0 first 586720 mov sz4
 00EBD05C: r1 w0 first 586939 mov sz4
 00EC3ABC: r2 w4 first 585C78 mov sz4
 00EC3AC0: r2 w0 first 585C7E mov[idx] sz4
 00EC5AE4: r2 w0 first 58602F addr-of sz4
 01F91020: r1 w2 first 585F19 mov sz4
 01F91040: r4 w1 first 585F64 mov sz4
 021703D4: r2 w0 first 586907 cmp sz4
 02527288: r1 w0 first 585FF0 mov sz4
 02528B48: r1 w0 first 5862DD mov sz1
 02528B49: r1 w0 first 5862E8 mov sz1
 02528C14: r1 w0 first 5863DF mov sz4
 0252A12C: r1 w1 first 586915 cmp sz4
 0252A130: r0 w1 first 58692F mov sz4
 026B5DE8: r1 w0 first 586000 addr-of sz4
 033D6308: r2 w0 first 58626C mov sz4
STRINGS
 DE8428 "icon_lg_d" @ 585FE6
 E1D454 "homies" @ 586259
 DE9AE4 "rpg_launcher" @ 58635E
 DE88C8 "ACT_HELI_KILL_CARS_START_MESSAGE" @ 586E3D
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [DD04F4] fld @ 586039: f32=1.5 bits=0x3fc00000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 586276 sete dl   (prev: cmp ecx, dword ptr [eax])
 5867C8 setne cl   (prev: cmp dword ptr [eax + 0x4c], 5)
 5867D1 setne al   (prev: test eax, eax)
 586AD4 setne dl   (prev: cmp dword ptr [eax + 0x4c], 5)
 586ADE setne al   (prev: test eax, eax)
 586B09 sete dl   (prev: cmp dword ptr [edi + 0xd78], 0)
 586C18 neg al   (prev: sub al, 1)
 586C1A sbb eax, eax   (prev: neg al)
JUMP TABLES
 jmp @ 586539 table 586E74 entries(12): 586540 586580 58659F 5865BA 5865CE 5865F8 58660D 586622 586633 586648 586C39 586A1D
 jmp @ 586A16 table 586E9C entries(2): 586C39 586A1D ; byte-index table at 586EA4
   byte table at 586EA4: 00 00 01 00 00 00 00 00 00 01 01 01 01 00 cc cc cc cc cc cc cc cc cc cc cc cc cc cc 55 8b ec 83
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 585ca9: edx < (signed) 0x800 -> 0x585cb1 [jl]
 585ce3: edx < (signed) 0x800 -> 0x585ceb [jl]
 585d0a: dword ptr [esi + eax*4 + 0x84] <= (unsigned) edi -> 0x586e02 [jbe]
 585ffa: edx <= (signed) edi -> 0x586013 [jle]
 586011: eax < (signed) edx -> 0x586005 [jl]
 58607d: dword ptr [ebx + ecx*4 + 0x88] <= (unsigned) edi -> 0x586dba [jbe]
 586533: eax > (unsigned) 9 -> 0x58665a [ja]
 5869b2: dword ptr [eax + 0x2c] < (signed) 0 -> 0x5869c4 [jl]
 586a0d: ecx > (unsigned) 0xd -> 0x586a1d [ja]
 586b57: ecx <= (signed) 0x6b49d200 -> 0x586b62 [jle]
 586b73: ecx <= (signed) 0x6b49d200 -> 0x586b7d [jle]
 586c60: eax < (unsigned) 2 -> 0x586200 [jb]
 586db4: ecx < (unsigned) dword ptr [edi + edx*4 + 0x88] -> 0x586090 [jb]
 586dfa: edx < (unsigned) dword ptr [ecx + esi*4 + 0x84] -> 0x585d20 [jb]

```


### 8. 0x6A0080 `mission_restart_from_checkpoint` (4508 bytes)

Convention: cdecl, one stack arg `[ebp+8]` (bool "force player restore"); frame `sub esp,0xFAC` with a /GS cookie; no callee-saved assumptions beyond the usual push/pop. Called from the mission fail flow (state 8 of `mission_fail_flow_do_frame`, 22_missions.md), `activity_instance_phase_restart_or_exit` and the checkpoint load path.

Data: the checkpoint array starts at `0x27B3C90` with `0x1071C` bytes per slot (`[esp+0x18]` = slot base, index in EAX from `[..]` at 0x6A00D9); inside a slot: `+0x44C/0x450/0x454` position, `+0x458` heading dword, `+0x470/0x474/0x478` orientation vector, `+0x47C` vehicle handle, `+0x480` weapon count, `+0x484 + n*0x68` weapon records (stride 0x68, key compare at 0x6A0365), `+0x5EC` vehicle id, `+0x5F0/0x5F1/0x5F2/0x5F3` flag bytes, `+0x5F4` equipped slot, `+0x5F8`, `+0x618 + n*0x200` entity records (count `[slot+0x10718]`).

Sections:
1. 0x6A0080-0x6A00CB "mission start" test: `sub_69E1C0(ECX=&name hash)`; `repe cmpsb` of the slot name against `"mission start"` (0xE00D9C, 13 bytes); `[esp+0x1C] = 1` when equal else 0.
2. 0x6A00CB-0x6A018C co-op announce: `edi = [0x2528C14]` (session); when a session exists, `[0x2528B56]`/`[0x2528B2B]` flags and `[session+0x3F8]` pass: builds packet type 0x6C sub 3 in a stack buffer (`[esp+0x580..]`, `[0xE8BC2C]` gates a 1-byte prefix) and `net_send_packet_to_remotes(buf, len)` (cdecl 2 args, EDI = session).
3. 0x6A018C-0x6A0256 `sub_D07C90()`, then if the mission is an activity of type 0xD (zombie): `sub_9E18F0(1)`, clear `[player+0x1161] &= 0xBF`, `sub_4D1270(0, 0.0f)`; `gamewide_func(1)`; `[0x140BE8F] = 0`; if `[player+0xF8C] >= 0` (signed): `[player+0xF8C] = -1`, `sub_92D4B0(1, ecx, 1, 0)` and the 60-entry table lookup `0x29E6C28/0x29E6C48` (stride 0x3C, index masked `& 0x1FFF`, bound `< 0x3C`, `jge` = skip) -> `sub_79D1B0`; `[player+0xC8C] = 0`.
4. 0x6A0256-0x6A02D6 same for the co-op partner (`coop_partner_human`).
5. 0x6A02D6-0x6A03A7 re-add the player's pets / followers: loop over `[player+0xC3C]` entries (unsigned `jb`), each 0x18 bytes; for entries whose object still exists and is a human with kind byte (`setne cl` on `[h+0x3CC] == 2`) matching, searches the checkpoint weapon table, `human_pkt_send_02(player, h)`.
6. 0x6A03A7-0x6A03EC `p = [[slot_base]+4]` (EAX); `if ([0x2527BBA] != 0 || [p+4] >= 0x6B (unsigned: jb skips)) gamesave_apply(0)` (cdecl, 1 stack arg, EAX = p, ESI/EDI also read by the callee), then free the pending object `[0x2527BD4]` via virtual `[vtable+0]` + `sub_C15720` and set it to 0.
7. 0x6A03EC-0x6A04FC net restore for non-local players: `net_handle_owned_locally(...)`; when not owned locally builds packet 0x68 sub 0x0A with the object's connection (`session_find_conn_by_id`) and `net_send_packet(buf, len, 0, 0)` then jumps to 0x6A09AB.
8. 0x6A04FC-0x6A06C2 weapon restore: `kind = [slot+0x5F4]` -> `jmp [eax*4 + 0x6A121C]` (10 entries: 0x6A051A, 0x6A0529, 0x6A054B, 0x6A053F, 0x6A055B, 0x6A057E, 0x6A0588, 0x6A0592, 0x6A059C, 0x6A05A6; `ja 0x6A05B0` for > 9): kinds look weapons up with `inventory_find_weapon`, `sub_55AE30(weapon, 1, 0, 0, 0)` (stdcall ret 0x14), `sub_55B200`, `sub_55B050(weapon, 0|1)` / `sub_55B310(weapon, 0|1)` (stdcall ret 8), `weapon_info_is_firearm`. Result weapon in `[esp+0x10]`.
9. 0x6A05D4-0x6A0809: unequip current weapon safely: vehicle gates (`human_in_vehicle`, kind byte 0xF-0x12 test), `sub_960C30`, clear `[v+0xC0] &= ~0x20`, `[v+0xB44] = -1`, `human_anim_set_update(h, -1, 1, 1, 0)` (cdecl 5 args), `weapon_target_lock_clear()`, `sub_9C5140`, `weapon_info_is_melee`, `sub_9C8C20(h, 1)`.
10. 0x6A0809-0x6A0919 re-equip with vehicle rules (`human_entering_or_exiting_vehicle(EAX=h)`, `vehicle_from_handle(ECX=handle)`, `weapon_usable_from_vehicle(h, ecx)`, `sub_55AE30`), set weapon timestamps with the game-time wrap (`> 0x6B49D200` signed -> `+= 0x94B62E00`) into `[w+0x24]`, `[w+0x28]`.
11. 0x6A0919-0x6A09AB `[ebx+0xC] = ecx; [ebx+8] = weapon`; virtual `[ebx+0x14]`-style callback `(edx, edi, eax)` (`call eax`, 3 stack args); `sub_7975E0`, `sub_798140`, `sub_558BE0`.
12. 0x6A09AB-0x6A0AD7 `sub_6A4C90()`; list walks over `[0x22A84D4]` and `[0x22A84E0]` (two intrusive lists; `sub esi,[list]; neg esi; sbb esi,esi; and esi,eax` is the "next or null" idiom: ESI = (next != list head) ? next : 0), calling virtual `[obj+0x10](0)` (ret to `sub_AE2790`, `sub_82BC50`).
13. 0x6A0AD7-0x6A0F46 restore the player position / vehicle: `if [ebp+8] == 0 && [mission+0x3C] != 1 && [esp+0x1C] == 0 goto 0x6A0F46`; vehicle lookup in the list `0x14099B0`, `sub_555920(...)`, `sub_928E80`, `net_send_prop_player_player_last_vehicle_handle`, `sub_AEFC40` stack object (`[esp+0x34..0x4E]`), position = saved pos + orientation * 5.0 (f64 constant 0xE5C130; `fld [slot+0x470]; fld 5.0; fmul st(1),st; ...` each component stored f32 to `[esp+0x38/0x44/0x48]`, then added to `[slot+0x44C/0x450/0x454]` and stored to `[esp+0x74/0x78/0x7C]`), `player_teleport(h, pos, 1, 1, 1, 0)` (cdecl, `add esp,0x18`), partner packet 0x6A sub 0x1B (`[0xE8BBFC]`), `or [player+0x8F74], 4`.
14. 0x6A0F46-0x6A1129 final per-human fix-ups: `sub_69EB00(slot)`, loops over the party (`human_party_root_leader`, `vehicle_interaction_rider_find` x2 with `[type] == 1/2` tests, the `sete dl; shl edx,0x19; and edx,0x2000000; xor [h+0xC8], edx` toggle that flips bit 25 of the human flags to "in vehicle party").
15. 0x6A1129-0x6A121C `sub_9FB9F0`, `sub_932F20`, `sub_932FF0`, `sub_5601E0`, `[0x29DACF0] = 0`, `[0x25283C1] = 0`, `sub_99F2D0` / `sub_99ED60` depending on `[slot+0x5F2/0x5F3]`, `sub_69AED0`, `screen_fade_start(0.0?)`: `fld1; fstp [esp]` then `screen_fade_start(0, 0, f32 1.0)`, `sub_4D1270(0x1F4, 0.0f)`, then for each entity record `[slot+0x618 + n*0x200]` (loop bound `[slot+0x10718]`, unsigned): `sub_781850(9, 0xE00FF0, 1, 1, record)`; cookie; `ret`.

Polarity notes: the checkpoint-name test is `repe cmpsb` followed by `jne` (equal -> keep `[esp+0x1C] = 1`); item 9's `setne`/`sbb` idioms are in the table; the game-time wrap test is `cmp ecx,0x6B49D200; jle skip` (signed).


Generated tables (exact, from the bytes):

```
FUNCTION 6A0080..6A121C size 4508 (1281 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0xfac; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0x584 (lea eax @ 6A0174) passed to net_send_packet_to_remotes
 esp+0x74 (lea edx @ 6A04E8) passed to net_send_packet
 esp+0xa9c (lea ecx @ 6A0DBA) passed to net_send_packet_to_remotes
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0069E1C0 sub_69e1c0: x1 first 6A00A6; callee reads ecx; ret 0; cleanup -
 00814740 net_send_packet_to_remotes: x2 first 6A0184; callee reads edi; ret 0; cleanup [8]
 00D07C90 sub_d07c90: x1 first 6A018C; callee reads -; ret 0; cleanup -
 009E18F0 sub_9e18f0: x1 first 6A01B7; callee reads esi; ret 0; cleanup -
 004D1270 sub_4d1270: x2 first 6A01CC; callee reads edx; ret 0; cleanup [8]
 0069AB80 gamewide_func: x1 first 6A01D6; callee reads -; ret 0; cleanup -
 0092D4B0 sub_92d4b0: x2 first 6A0207; callee reads -; ret 0; cleanup [16]
 0079D1B0 sub_79d1b0: x2 first 6A0245; callee reads eax,ecx; ret 0; cleanup -
 007F7AD0 coop_session_active: x1 first 6A0256; callee reads -; ret 0; cleanup -
 009D7BD0 coop_partner_human: x5 first 6A025F; callee reads -; ret 0; cleanup -
 0098F1B0 human_pkt_send_02: x1 first 6A0373; callee reads -; ret 0; cleanup -
 00695F00 gamesave_apply: x1 first 6A03BF; callee reads eax; ret 0; cleanup [4]
 00C15720 sub_c15720: x1 first 6A03DD; callee reads ecx,esi; ret 0; cleanup -
 008066C0 net_handle_owned_locally: x1 first 6A0404; callee reads ecx; ret 0; cleanup -
 0080FE80 session_find_conn_by_id: x1 first 6A04DB; callee reads eax,edx; ret 0; cleanup -
 00814670 net_send_packet: x1 first 6A04EF; callee reads esi; ret 0; cleanup [16]
 0055ADF0 inventory_find_weapon: x3 first 6A051F; callee reads eax,esi; ret 0; cleanup -
 0055AE30 sub_55ae30: x3 first 6A0532; callee reads -; ret 0x14; cleanup -
 0055B200 sub_55b200: x1 first 6A0541; callee reads eax; ret 0; cleanup -
 00B72800 weapon_info_is_firearm: x1 first 6A056F; callee reads eax; ret 0; cleanup -
 0055B050 sub_55b050: x2 first 6A0581; callee reads -; ret 0x8; cleanup -
 0055B310 sub_55b310: x2 first 6A059F; callee reads edi; ret 0x8; cleanup -
 009B8C50 human_in_vehicle: x1 first 6A0613; callee reads edi,esi; ret 0; cleanup -
 00960C30 sub_960c30: x1 first 6A067F; callee reads esi; ret 0; cleanup -
 0096CB70 human_anim_set_update: x1 first 6A06BA; callee reads -; ret 0; cleanup [20]
 00B6B460 weapon_target_lock_clear: x1 first 6A075E; callee reads ecx; ret 0; cleanup -
 009C5140 sub_9c5140: x1 first 6A078C; callee reads eax; ret 0; cleanup -
 00B72740 weapon_info_is_melee: x1 first 6A07CB; callee reads eax; ret 0; cleanup -
 009C8C20 sub_9c8c20: x1 first 6A07FC; callee reads -; ret 0; cleanup [8]
 009B92E0 human_entering_or_exiting_vehicle: x1 first 6A083A; callee reads eax; ret 0; cleanup -
 00AE28F0 vehicle_from_handle: x1 first 6A084B; callee reads ecx; ret 0; cleanup -
 00B72D30 weapon_usable_from_vehicle: x1 first 6A0884; callee reads eax,edx; ret 0; cleanup [8]
 007975E0 sub_7975e0: x1 first 6A0996; callee reads ecx; ret 0; cleanup -
 00798140 sub_798140: x2 first 6A099C; callee reads -; ret 0; cleanup [4]
 00558BE0 sub_558be0: x1 first 6A09A6; callee reads ecx,esi; ret 0; cleanup -
 006A4C90 sub_6a4c90: x1 first 6A09AB; callee reads -; ret 0; cleanup -
 0082BC50 sub_82bc50: x1 first 6A09ED; callee reads eax,ecx; ret 0; cleanup -
 00AE2790 sub_ae2790: x1 first 6A0AB9; callee reads ecx; ret 0; cleanup -
 00555920 sub_555920: x1 first 6A0B83; callee reads eax,ecx; ret 0; cleanup -
 00928E80 sub_928e80: x1 first 6A0BF9; callee reads ecx,edx; ret 0; cleanup [8]
 009D5E30 net_send_prop_player_player_last_vehicle_handle: x1 first 6A0C24; callee reads ecx; ret 0x8; cleanup -
 00AEFC40 sub_aefc40: x1 first 6A0CA1; callee reads eax,ecx; ret 0; cleanup -
 009DB480 player_teleport: x3 first 6A0D18; callee reads ecx; ret 0; cleanup [24]
 00A6F2B0 sub_a6f2b0: x1 first 6A0E10; callee reads eax,ecx; ret 0x8; cleanup -
 00836C90 sub_836c90: x1 first 6A0F3C; callee reads eax,esi; ret 0; cleanup [16]
 0069EB00 sub_69eb00: x1 first 6A0F4B; callee reads -; ret 0; cleanup -
 0098E750 human_party_root_leader: x1 first 6A0FDC; callee reads edx; ret 0; cleanup -
 00B07A30 vehicle_interaction_rider_find: x2 first 6A105C; callee reads esi; ret 0; cleanup -
 009FB9F0 sub_9fb9f0: x1 first 6A1129; callee reads -; ret 0; cleanup -
 00932F20 sub_932f20: x1 first 6A112E; callee reads -; ret 0; cleanup -
 00932FF0 sub_932ff0: x1 first 6A1133; callee reads -; ret 0; cleanup -
 005601E0 sub_5601e0: x1 first 6A1162; callee reads eax,edx; ret 0; cleanup -
 0099F2D0 sub_99f2d0: x1 first 6A1187; callee reads ecx,esi; ret 0; cleanup -
 0099ED60 sub_99ed60: x1 first 6A119A; callee reads eax; ret 0; cleanup -
 0069AED0 sub_69aed0: x1 first 6A119F; callee reads ecx,edi; ret 0; cleanup -
 004D1090 screen_fade_start: x1 first 6A11AE; callee reads -; ret 0; cleanup -
 00781850 sub_781850: x1 first 6A11EE; callee reads edx; ret 0; cleanup -
 00C9604F __security_check_cookie: x1 first 6A1213; callee reads ecx; ret 0; cleanup -
INDIRECT CALLS: 6 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E00FF0: r1 w0 first 6A11E5 addr-of sz4
 00E5C130: r2 w0 first 6A0CB6 fld sz8
 00E80B50: r1 w0 first 6A008C mov sz4
 00E843AC: r1 w0 first 6A08B8 mov sz4
 00E8BBCC: r1 w0 first 6A0411 cmp sz1
 00E8BBFC: r1 w0 first 6A0D2D cmp sz1
 00E8BC2C: r1 w0 first 6A0127 cmp sz1
 00EBCE9C: r2 w0 first 6A081A mov sz4
 00EBCEA0: r2 w0 first 6A0829 mov sz4
 014099B0: r1 w0 first 6A0B10 mov sz4
 0140BE8F: r0 w1 first 6A01EF mov sz1
 0212B160: r1 w0 first 6A0F25 mov sz4
 0215FFC0: r2 w0 first 6A0F50 mov sz4
 021703D4: r19 w0 first 6A01AD mov sz4
 022A84D4: r2 w0 first 6A09FE mov sz4
 022A84E0: r2 w0 first 6A0A6D mov sz4
 02527BBA: r1 w0 first 6A03A7 cmp sz1
 02527BD4: r2 w1 first 6A03C7 mov sz4
 02527C80: r1 w0 first 6A0108 mov sz4
 02527CA8: r1 w0 first 6A00AF mov sz4
 025283C1: r0 w1 first 6A1178 mov sz1
 02528B2B: r3 w0 first 6A00D1 mov sz1
 02528B48: r1 w0 first 6A0C36 mov sz1
 02528B49: r1 w0 first 6A0C41 mov sz1
 02528B56: r3 w0 first 6A00EA mov sz1
 02528C14: r10 w0 first 6A00CB mov sz4
 027B3C60: r1 w0 first 6A0100 mov sz4
 029DACF0: r0 w1 first 6A116E mov sz4
 029E6C28: r2 w0 first 6A0239 lea[idx] sz4
 029E6C48: r2 w0 first 6A0232 cmp[idx] sz4
 02E7DF0C: r1 w0 first 6A0E15 mov sz4
 02F9B18C: r1 w0 first 6A0DEB mov sz4
 033D6308: r1 w0 first 6A0DDF cmp sz4
STRINGS
 E00D9C "mission start" @ 6A00B5
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C130] fld @ 6A0CB6: f64=5.0 bits=0x4014000000000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 6A033C setne cl   (prev: cmp byte ptr [eax + 0x3cc], 2)
 6A0852 setne al   (prev: test eax, eax)
 6A087D sete cl   (prev: cmp dword ptr [esi + 0xd78], 0)
 6A098A neg al   (prev: sub al, 1)
 6A098C sbb eax, eax   (prev: neg al)
 6A0A22 neg esi   (prev: sub esi, dword ptr [0x22a84d4])
 6A0A24 sbb esi, esi   (prev: neg esi)
 6A0A85 neg esi   (prev: sub esi, dword ptr [0x22a84e0])
 6A0A87 sbb esi, esi   (prev: neg esi)
 6A0F7A neg ecx   (prev: mov ebx, 1)
 6A0F7C sbb ecx, ecx   (prev: neg ecx)
 6A10A2 setne al   (prev: test eax, eax)
 6A10AD setne cl   (prev: cmp dword ptr [eax + 0x4c], 5)
 6A10B7 setne al   (prev: test eax, eax)
 6A10C6 sete dl   (prev: test al, al)
JUMP TABLES
 jmp @ 6A0513 table 6A121C entries(10): 6A051A 6A0529 6A054B 6A053F 6A055B 6A057E 6A0588 6A0592 6A059C 6A05A6
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 6a01f6: dword ptr [esi + 0xf8c] < (signed) ebx -> 0x6a0256 [jl]
 6a0229: eax >= (signed) 0x3c -> 0x6a0250 [jge]
 6a026b: dword ptr [eax] < (signed) ebx -> 0x6a02d6 [jl]
 6a02a8: ecx >= (signed) 0x3c -> 0x6a02cb [jge]
 6a02e6: dword ptr [esi + 0xc3c] <= (unsigned) ebx -> 0x6a03a7 [jbe]
 6a0354: edx <= (signed) 0 -> 0x6a0371 [jle]
 6a036f: eax < (signed) edx -> 0x6a0363 [jl]
 6a03a1: eax < (unsigned) dword ptr [esi + 0xc3c] -> 0x6a0300 [jb]
 6a03bb: dword ptr [eax + 4] < (unsigned) 0x6b -> 0x6a03c7 [jb]
 6a050d: eax > (unsigned) 9 -> 0x6a05b0 [ja]
 6a077f: dword ptr [eax + 0x2c] < (signed) 0 -> 0x6a0791 [jl]
 6a08c7: ecx <= (signed) 0x6b49d200 -> 0x6a08d2 [jle]
 6a08e7: ecx <= (signed) 0x6b49d200 -> 0x6a08f1 [jle]
 6a11d2: dword ptr [ebx + 0x10718] <= (unsigned) esi -> 0x6a1207 [jbe]
 6a1205: esi < (unsigned) dword ptr [ebx + 0x10718] -> 0x6a11e0 [jb]

```


### 9. 0x64C410 `fraud_level_instance::load_from_table` (4267 bytes)

Convention: thiscall, ECX = level instance (EBP), stack args `(a, b, c)`, `ret 0xC`, /GS cookie.

Sections:
1. 0x64C410-0x64C4E4 `sub_68A160()`; base `activity_level_instance::load_from_table(ECX = this, EDI = element, EAX = ...)`.
2. 0x64C4E4-0x64C5FF unsigned fields via `xtbl_get_uint`: `Goal` -> `[this+0x60]`, `Coop_Goal` (default `0x7FFFFFFF` -> `[this+0x64]` when absent or in single player), `Duration` -> `[this+0x68]`, `Max_Spots` -> `[this+0x6C]`; floats `Crazy_Chance` -> `0x2527970`, `Crazy_Hit_Chance` -> `[this+0x2C]` then copied to `0x140C4BC`, `Ring_Money_Percent` -> `0x140C4BC`; `[0xEAA56C] = -1`.
3. 0x64C5FF-0x64C613 `sub_64B370(&0xEAA3F0)`, `sub_968690(player, 0x140C514)`.
4. 0x64C613-0x64CCC9 weapon restore for the player (inlined copy of the unequip / re-equip logic of `mission_restart_from_checkpoint`, sections 9-11 there): `vehicle_interaction_rider_find` x2, `sub_960C30`, `human_anim_set_update(-1,1,1,0)`, `weapon_lockon_reticule_update(0,0,0.0)`, `audio_voice_stop`, `sub_9C5140`, `weapon_info_is_melee`, `sub_9C8C20(h, 1)`, `vehicle_from_handle`, `weapon_usable_from_vehicle`, `sub_55AE30`, virtual `(edx, ebx, eax)` call, `sub_7975E0`, `sub_798140`, `sub_558BE0`. Net resend: `net_send_packet(buf, len, 0, 0)` at 0x64CE62.
5. 0x64CE76-0x64D354 the same block for the co-op partner (`coop_partner_human`).
6. 0x64D3A0-0x64D4B8 `sub_68BAC0(...)`, `sub_64C0A0()` (fraud HUD current hit), `coop_partner_human`, `sub_649D10(2, 0)`, cookie, `ret 0xC`.

Port notes: sections 4 and 5 are the two copies of one routine with `human` as the only varying input; factor them into one function called twice (the order player -> partner must be kept).


Generated tables (exact, from the bytes):

```
FUNCTION 64C410..64D4BB size 4267 (1235 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0xa44; /GS cookie (xor with esp, __security_check_cookie at exit)
 esp+0x18 (lea esi @ 64C542) passed to xtbl_get_uint
 esp+0x34 (lea ecx @ 64C7B3) passed to net_send_packet
 esp+0x548 (lea ecx @ 64CE5A) passed to net_send_packet
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0068A160 sub_68a160: x1 first 64C4CC; callee reads eax,ecx; ret 0; cleanup -
 0053F700 activity_level_instance::load_from_table: x1 first 64C4E4; callee reads ecx; ret 0xC; cleanup -
 00B74910 xtbl_get_uint: x4 first 64C4F2; callee reads ecx,esi; ret 0; cleanup [4]
 00B74EF0 xtbl_get_float: x3 first 64C578; callee reads ecx,esi; ret 0; cleanup -
 0064B370 sub_64b370: x1 first 64C5FF; callee reads eax; ret 0x4; cleanup -
 00968690 sub_968690: x1 first 64C613; callee reads -; ret 0x8; cleanup -
 00814670 net_send_packet: x2 first 64C7B8; callee reads esi; ret 0; cleanup [16]
 00B07A30 vehicle_interaction_rider_find: x6 first 64C886; callee reads esi; ret 0; cleanup -
 00960C30 sub_960c30: x2 first 64C983; callee reads esi; ret 0; cleanup -
 0096CB70 human_anim_set_update: x2 first 64C9BE; callee reads -; ret 0; cleanup [20]
 00B6A8B0 weapon_lockon_reticule_update: x2 first 64CA73; callee reads ecx,edx; ret 0; cleanup [4]
 0047FEB0 audio_voice_stop: x2 first 64CA8C; callee reads ecx; ret 0; cleanup -
 009C5140 sub_9c5140: x2 first 64CAB6; callee reads eax; ret 0; cleanup -
 00B72740 weapon_info_is_melee: x2 first 64CAF5; callee reads eax; ret 0; cleanup -
 009C8C20 sub_9c8c20: x2 first 64CB26; callee reads -; ret 0; cleanup -
 00AE28F0 vehicle_from_handle: x2 first 64CB7C; callee reads ecx; ret 0; cleanup -
 00B72D30 weapon_usable_from_vehicle: x2 first 64CBB1; callee reads eax,edx; ret 0; cleanup [8]
 0055AE30 sub_55ae30: x2 first 64CBCA; callee reads -; ret 0x14; cleanup -
 007975E0 sub_7975e0: x2 first 64CCB9; callee reads ecx; ret 0; cleanup -
 00798140 sub_798140: x2 first 64CCBF; callee reads -; ret 0; cleanup [4]
 00558BE0 sub_558be0: x2 first 64CCC9; callee reads ecx,esi; ret 0; cleanup -
 0055ADF0 inventory_find_weapon: x1 first 64CE76; callee reads eax,esi; ret 0; cleanup -
 009B92E0 human_entering_or_exiting_vehicle: x1 first 64D1EE; callee reads eax; ret 0; cleanup -
 0068BAC0 sub_68bac0: x1 first 64D3A0; callee reads eax; ret 0; cleanup -
 0064C0A0 fraud_hud_current_hit: x1 first 64D446; callee reads eax; ret 0; cleanup -
 009D7BD0 coop_partner_human: x1 first 64D48F; callee reads -; ret 0; cleanup -
 00649D10 sub_649d10: x1 first 64D49C; callee reads -; ret 0; cleanup [8]
 00C9604F __security_check_cookie: x1 first 64D4AD; callee reads ecx; ret 0; cleanup [2628]
INDIRECT CALLS: 2 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E5D100: r1 w0 first 64C618 fld sz4
 00E80B50: r1 w0 first 64C416 mov sz4
 00E843AC: r4 w0 first 64C426 mov sz4
 00E888AC: r0 w1 first 64D439 fstp sz4
 00E8B217: r0 w1 first 64D3C2 mov sz1
 00E8BBCC: r2 w0 first 64C6BE cmp sz1
 00E8E2EF: r0 w1 first 64D381 mov sz1
 00E976D0: r0 w1 first 64C46C mov sz4
 00E9A128: r0 w1 first 64D3AB fstp sz4
 00EAA3F0: r1 w1 first 64C5FA addr-of sz4
 00EAA400: r0 w1 first 64C642 mov sz4
 00EAA404: r0 w1 first 64C647 mov sz4
 00EAA408: r0 w1 first 64C64D mov sz4
 00EAA55C: r0 w1 first 64D394 mov sz4
 00EAA560: r0 w2 first 64C472 mov sz4
 00EAA56C: r0 w1 first 64C5B6 mov sz4
 00EAA58C: r0 w2 first 64D45A mov sz4
 00EAA590: r0 w1 first 64D38E mov sz4
 00EAA6F8: r0 w1 first 64D39A mov sz4
 00EBCE9C: r4 w0 first 64C864 mov sz4
 00EBCEA0: r4 w0 first 64C86B mov sz4
 00EBD05C: r2 w0 first 64CA7D addr-of sz4
 0140BB81: r0 w1 first 64C4AF mov sz1
 0140C44C: r0 w1 first 64D388 mov sz4
 0140C498: r0 w1 first 64C5C3 mov sz4
 0140C4A4: r0 w1 first 64C43B fstp sz4
 0140C4A8: r0 w1 first 64C434 fst sz4
 0140C4AC: r0 w1 first 64C42C fst sz4
 0140C4B0: r1 w0 first 64D3A5 fld sz4
 0140C4BC: r1 w1 first 64C58E fstp sz4
 0140C514: r1 w0 first 64C607 addr-of sz4
 0215D7A0: r0 w3 first 64D41B fstp sz4
 0215D7A4: r0 w3 first 64D3DE fstp sz4
 021703D4: r7 w0 first 64C510 mov sz4
 0217043C: r2 w0 first 64C504 mov sz4
 02527965: r0 w1 first 64C4BF mov sz1
 02527966: r0 w1 first 64D37A mov sz1
 02527970: r2 w0 first 64C573 addr-of sz4
 02527974: r0 w1 first 64C466 mov sz4
 0252798C: r0 w1 first 64C4B6 mov sz4
 02527990: r0 w1 first 64C485 mov sz4
 02527994: r0 w1 first 64C48A mov sz4
 02527998: r0 w1 first 64C48F mov sz4
 0252799C: r0 w1 first 64C494 mov sz4
 025279A4: r0 w1 first 64C499 mov sz4
 025279A8: r0 w1 first 64C49E mov sz4
 025279AC: r0 w1 first 64C4A3 mov sz4
 025279B0: r0 w1 first 64C4A8 mov sz4
 025279BC: r0 w1 first 64D366 fst sz4
 025279FC: r0 w1 first 64C4C6 mov sz1
 02528B2B: r1 w0 first 64D47D cmp sz1
 02528B49: r2 w0 first 64C4F7 mov sz1
 02528B56: r1 w0 first 64D474 cmp sz1
 02528C14: r3 w0 first 64C61E mov sz4
 02529623: r0 w1 first 64D43F mov sz1
 0252A12C: r2 w2 first 64CA60 cmp sz4
 0252A130: r0 w2 first 64CA82 mov sz4
STRINGS
 DF6C18 "Goal" @ 64C4ED
 DFB448 "Coop_Goal" @ 64C52F
 DDC3BC "Duration" @ 64C53D
 DFB454 "Max_Spots" @ 64C560
 DFB460 "Crazy_Chance" @ 64C56E
 DFB470 "Crazy_Hit_Chance" @ 64C581
 DFB484 "Ring_Money_Percent" @ 64C594
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5D100] fld @ 64C618: f32=-2000.0 bits=0xc4fa0000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 64D3BD fcom st(1) ; (no test ah + jcc found nearby)
 64D3D3 fcom st(1) ; test ah,0x41; jne @ 64D3DA -> 0x64d3f2: taken when ST0<op | ST0==op | unordered
 64D405 fcom st(2) ; test ah,0x41; jne @ 64D40C -> 0x64d423: taken when ST0<op | ST0==op | unordered
 64D410 fcom st(1) ; test ah,0x41; jne @ 64D417 -> 0x64d42f: taken when ST0<op | ST0==op | unordered
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 64C916 setne cl   (prev: cmp dword ptr [eax + 0x4c], 5)
 64C920 setne al   (prev: test eax, eax)
 64CB83 setne al   (prev: test eax, eax)
 64CBAA sete cl   (prev: cmp dword ptr [edi + 0xd78], 0)
 64CCAD neg al   (prev: sub al, 1)
 64CCAF sbb eax, eax   (prev: neg al)
 64CFA6 setne cl   (prev: cmp dword ptr [eax + 0x4c], 5)
 64CFB0 setne al   (prev: test eax, eax)
 64D206 setne al   (prev: test eax, eax)
 64D22D sete cl   (prev: cmp dword ptr [esi + 0xd78], 0)
 64D338 neg al   (prev: sub al, 1)
 64D33A sbb eax, eax   (prev: neg al)
JUMP TABLES
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 64c477: eax <= (signed) 0x6b49d200 -> 0x64c483 [jle]
 64caa9: dword ptr [eax + 0x2c] < (signed) 0 -> 0x64cabb [jl]
 64cbee: ecx <= (signed) 0x6b49d200 -> 0x64cbf9 [jle]
 64cc0a: ecx <= (signed) 0x6b49d200 -> 0x64cc14 [jle]
 64d13e: dword ptr [eax + 0x2c] < (signed) edi -> 0x64d150 [jl]
 64d277: ecx <= (signed) 0x6b49d200 -> 0x64d282 [jle]
 64d293: ecx <= (signed) 0x6b49d200 -> 0x64d29d [jle]
 64d45f: eax <= (signed) 0x6b49d200 -> 0x64d46b [jle]

```


### 10. 0x5735A0 `guardian_angel_state_step` (4185 bytes)

Convention: register-only; called by `guardian_angel_level_instance::load_from_table` (0x56E210) and `::update` (0x56F590) (the two callers run it with ESI/EBP = level instance); plain `ret`. Frame uses `[ebp+0x31D4]`, `[ebp+0x319C + n*4]` (enemy array), `[ebp+0x204]`, `[ebp+0x24]` (current state).

Sections:
1. 0x5735A0-0x57360E dispatch: `jmp [eax*4 + 0x5745FC]`, 48 entries decoded in the jump table below (entries 0x573615, 0x57366C, 0x5736E2, 0x5737B3, 0x5738C2, 0x5738DC, 0x5739B0, 0x573AAA, 0x573AD1, ...).
2. States 0-2 (0x573615-0x5737B3): spawn / validate the celebrity vehicle (`sub_43F380(EAX=vehicle)`, `navpoint_find`, `net_send_veh_pkt`, `vehicle_ai_clear_speed_override`, `vehicle_ai_set_speed_override`, `vehicle_ai_set_force_flag_override_speed`).
3. States 3-6 (0x5738C2-0x573AAA): chase / attack: `sub_A57770`, `sub_A56F80`, `object_visibility_cached`, `vehicle_take_damage`, `human_apply_damage`, `sub_B19060`, `vec3_distance_sq_dup`, `sub_5E00F0`, `sub_5E01D0`.
4. States 7-12 (0x573AD1-0x573C5E): enemy wave spawn: `human_ai_set_force_flag_attack_pedestrians_on_sight`, `_ftol2_sse`, `timer_remaining` / `timer_set`.
5. States 13-17 (0x573C52-0x573CEB): script hand-off: `string_crc32` + `sub_53FA10` + `misc_packet_send_14` (twice), `sub_5747E0`.
6. States 18-24 (0x573CEB-0x573F84): `_ftol2_sse`, `timer_set`, vehicle chase helpers `sub_B3BB70`, `sub_B3C8A0`, `sub_B3B640`, `navpoint_find(.., 0x19)`.
7. States 25-47 (0x573F84-0x5745F9): follower setup (`sub_56FF40`, `human_set_party_leader`, `npc_set_force_flag_independent_follower`, `sub_56D7D0`, `sub_56DE50`, `human_ai_set_force_flag_combat_disabled`, `human_ai_set_force_flag_attack_non_player_enemies_on_sight`, `net_send_prop_human_ai_data_leash_params`, `human_pkt_send_3C`, `inventory_equip_slot`), HUD (`sub_7A2030`, `effect_find_by_name("icon_lg_c")`, `sub_92D1A0`, `sub_7A2180`, `sub_92D4B0`), vehicle flags (`vehicle_set_force_flag_infinite_vehicle_collision_mass`, `..._immune_to_explosion_fling`), `human_pkt_send_44`, `sub_B71CF0`, `sub_53D1F0`, `sub_56FFE0`, `sub_56C900`, `sub_5746C0`, `sub_AAAB60`, `sub_95B0B0`, `sub_95B130`.

Port notes: 128 x87 instructions (timers and distances), 60 virtual calls. A 48-entry table with many shared targets: decode with the table below; port state by state and keep unlisted states as the original fall-through.


Generated tables (exact, from the bytes):

```
FUNCTION 5735A0..5745F9 size 4185 (1229 instructions)
STACK FRAME / STACK OBJECTS
 frame: sub esp,0x1f8; no /GS cookie
 esp+0x20 (lea edx @ 573659) passed to sub_43f380
 esp+0x28 (lea edx @ 5736CF) passed to sub_43f380
 esp+0xac (lea edx @ 573753) passed to net_send_veh_pkt
 esp+0x12c (lea edx @ 57384F) passed to net_send_veh_pkt
 esp+0x24 (lea edx @ 573A97) passed to sub_43f380
 esp+0x38 (lea ecx @ 573F0E) passed to sub_b3b640
CALLEES (target: count, first site; callee reads = regs read before written; ret N = stdcall bytes; cleanup = caller add esp)
 0043F380 sub_43f380: x3 first 57365F; callee reads eax,edx; ret 0; cleanup [12]
 00A5EB50 navpoint_find: x4 first 573694; callee reads eax; ret 0; cleanup -
 00B28330 net_send_veh_pkt: x2 first 57375A; callee reads ecx,edx; ret 0; cleanup -
 00B27530 vehicle_ai_clear_speed_override: x1 first 57377C; callee reads eax; ret 0; cleanup -
 00B274F0 vehicle_ai_set_speed_override: x2 first 5737A6; callee reads eax,ecx; ret 0; cleanup [4]
 00B2B8A0 vehicle_ai_set_force_flag_override_speed: x1 first 57388B; callee reads edi; ret 0x4; cleanup -
 00A57770 sub_a57770: x1 first 5738D2; callee reads eax; ret 0x4; cleanup -
 00A56F80 sub_a56f80: x1 first 573926; callee reads eax; ret 0; cleanup -
 009255C0 object_visibility_cached: x1 first 57392D; callee reads esi; ret 0x4; cleanup -
 00AABCC0 vehicle_take_damage: x1 first 573965; callee reads -; ret 0; cleanup [16]
 009645A0 human_apply_damage: x1 first 573984; callee reads ecx,edx,esi; ret 0; cleanup [4]
 00B19060 sub_b19060: x1 first 5739E4; callee reads edx,esi; ret 0; cleanup -
 00404A20 vec3_distance_sq_dup: x1 first 573A0B; callee reads eax,ecx; ret 0; cleanup -
 005E00F0 sub_5e00f0: x1 first 573A37; callee reads edi,esi; ret 0; cleanup -
 005E01D0 sub_5e01d0: x1 first 573AC4; callee reads eax,edi; ret 0; cleanup [4]
 004401A0 human_ai_set_force_flag_attack_pedestrians_on_sight: x2 first 573B07; callee reads -; ret 0x8; cleanup -
 00DA0140 _ftol2_sse: x3 first 573B57; callee reads -; ret 0; cleanup -
 00BDD260 timer_remaining: x1 first 573B85; callee reads eax; ret 0; cleanup -
 00BDD100 timer_set: x2 first 573B92; callee reads eax,ecx; ret 0; cleanup -
 00BDCA30 string_crc32: x2 first 573C80; callee reads eax,edx; ret 0x4; cleanup -
 0053FA10 sub_53fa10: x2 first 573C85; callee reads edi; ret 0; cleanup [4]
 00926860 misc_packet_send_14: x2 first 573C95; callee reads ecx; ret 0; cleanup [12]
 005747E0 sub_5747e0: x1 first 573CE1; callee reads eax,ecx; ret 0x4; cleanup -
 00B3BB70 sub_b3bb70: x1 first 573D49; callee reads -; ret 0; cleanup [12]
 00B3C8A0 sub_b3c8a0: x1 first 573DA5; callee reads eax; ret 0; cleanup [12]
 00B3B640 sub_b3b640: x2 first 573E18; callee reads ecx; ret 0; cleanup [4]
 0056FF40 sub_56ff40: x2 first 573FE9; callee reads ecx,esi; ret 0x4; cleanup -
 0098EC10 human_set_party_leader: x1 first 57400E; callee reads edi; ret 0; cleanup -
 009D23E0 npc_set_force_flag_independent_follower: x1 first 57402D; callee reads -; ret 0x8; cleanup -
 0056D7D0 sub_56d7d0: x1 first 574050; callee reads eax,ecx; ret 0; cleanup -
 0056DE50 sub_56de50: x1 first 5740AD; callee reads ecx,esi; ret 0; cleanup -
 00440CE0 human_ai_set_force_flag_combat_disabled: x2 first 5740E8; callee reads -; ret 0x8; cleanup -
 00440060 human_ai_set_force_flag_attack_non_player_enemies_on_sight: x2 first 5740FE; callee reads -; ret 0x8; cleanup -
 00443C30 net_send_prop_human_ai_data_leash_params: x1 first 574134; callee reads ecx; ret 0xC; cleanup -
 00558FB0 human_pkt_send_3c: x1 first 574160; callee reads ecx,edx; ret 0xC; cleanup -
 0055A930 inventory_equip_slot: x1 first 574185; callee reads -; ret 0x10; cleanup -
 007A2030 sub_7a2030: x1 first 574257; callee reads ecx; ret 0; cleanup -
 0050DB60 effect_find_by_name: x1 first 574273; callee reads ecx; ret 0; cleanup -
 0092D1A0 sub_92d1a0: x1 first 57428F; callee reads ecx; ret 0; cleanup [20]
 007A2180 sub_7a2180: x1 first 5742D6; callee reads ecx; ret 0; cleanup -
 0092D4B0 sub_92d4b0: x1 first 5742F1; callee reads -; ret 0; cleanup [16]
 00AB9BE0 vehicle_set_force_flag_infinite_vehicle_collision_mass: x1 first 574341; callee reads -; ret 0x8; cleanup -
 00AB9440 vehicle_set_force_flag_immune_to_explosion_fling: x1 first 574349; callee reads -; ret 0x8; cleanup -
 00B723D0 human_pkt_send_44: x2 first 5743B1; callee reads ecx,edx; ret 0; cleanup [4]
 009D7BD0 coop_partner_human: x4 first 5743B9; callee reads -; ret 0; cleanup -
 00B71CF0 sub_b71cf0: x1 first 5743FB; callee reads ecx; ret 0; cleanup [20]
 0053D1F0 sub_53d1f0: x1 first 574425; callee reads eax,ecx; ret 0; cleanup [12]
 0056FFE0 sub_56ffe0: x1 first 57445A; callee reads edi; ret 0; cleanup -
 0056C900 sub_56c900: x1 first 5744B3; callee reads ecx; ret 0x4; cleanup -
 005746C0 sub_5746c0: x1 first 5744D8; callee reads esi; ret 0x4; cleanup -
 00AAAB60 sub_aaab60: x1 first 574555; callee reads ecx,edx; ret 0; cleanup [12]
 0095B0B0 sub_95b0b0: x1 first 574580; callee reads ecx; ret 0; cleanup [12]
 0095B130 sub_95b130: x1 first 5745CA; callee reads esi; ret 0; cleanup -
INDIRECT CALLS: 60 (call reg / call [reg+off]; see section text for the vtables)
GLOBALS (reads, writes, first site, op, size)
 00E5C0D8: r1 w0 first 5735E6 fld sz8
 00E5CA98: r3 w0 first 57378A fmul sz8
 00E843AC: r1 w0 first 573C28 mov sz4
 00EC5AE4: r1 w0 first 57428A addr-of sz4
 01408E88: r1 w0 first 574192 mov sz4
 021703D4: r3 w0 first 574000 mov sz4
 02527558: r1 w0 first 573BDF mov sz4
 027B3C60: r1 w0 first 573936 cmp sz4
STRINGS
 DE8590 "icon_lg_c" @ 57426E
X87 CONSTANT LOADS (rdata operand: address, op, site, value, exact bits)
 [E5C0D8] fld @ 5735E6: f64=1000.0 bits=0x408f400000000000
 [E5CA98] fmul @ 57378A: f64=0.44703999161720276 bits=0x3fdc9c4da0000000
X87 COMPARES (site, op, test ah mask, jcc, condition for the TAKEN branch)
 573761 fcomp dword ptr [edi + 0xc] ; test ah,0x44; jp @ 573771 -> 0x573786: taken when ST0>op | ST0<op | unordered
 57385D fcomp dword ptr [edi + 0xc] ; test ah,0x44; jp @ 57386D -> 0x573895: taken when ST0>op | ST0<op | unordered
 573A10 fcomp qword ptr [esp + 0x30] ; test ah,0x5; jp @ 573A19 -> 0x573a58: taken when ST0>op | ST0==op | unordered
FLAG IDIOMS (setcc / sbb / adc / neg+sbb / cmov)
 5743DF setle cl   (prev: push 0)
JUMP TABLES
 jmp @ 57360E table 5745FC entries(48): 573615 57366C 5736E2 5737B3 5738C2 5738DC 5739B0 573AAA 573AD1 573AE9 573B22 573B54 573B64 573B99 573C52 573C5E 573C6A 573CA2 573CDA 573CEB 573D2C 573DB2 573E5A 573F84 573FB4 573FC0 574061 5740D2 57413E 57418F 5741A0 5741B0 5741C0 5741E3 5742A3 574316 57435A 574369 574395 5743A4 574432 57443E 57444A 5744BD 574538 5744F2 574562 574593
SIGNED / UNSIGNED INTEGER COMPARES (non-equality; text is the TAKEN-branch condition)
 5735cd: dword ptr [eax + ebp + 0x494] <= (signed) 0 -> 0x573bc8 [jle]
 573608: eax > (unsigned) 0x2f -> 0x573ba2 [ja]
 57368b: dword ptr [edi + 0x118] > (unsigned) 0 -> 0x573691 [ja]
 5737d5: eax <= (unsigned) 0 -> 0x573825 [jbe]
 5737e8: ebx >= (unsigned) 0x14 -> 0x573825 [jae]
 5737ec: ebx < (unsigned) eax -> 0x5737f2 [jb]
 573823: ebx < (unsigned) eax -> 0x5737e5 [jb]
 573830: ecx < (unsigned) 0x14 -> 0x573837 [jb]
 573b81: dword ptr [ebp + 0xb0] < (signed) 0 -> 0x573ba4 [jl]
 573b8c: eax <= (signed) esi -> 0x573ba4 [jle]
 573bc2: eax < (signed) dword ptr [ecx + ebp + 0x494] -> 0x5735e0 [jl]
 573bdd: dword ptr [eax + ebp + 0x510] <= (signed) edx -> 0x573c28 [jle]
 573c26: edx < (signed) dword ptr [eax + ebp + 0x510] -> 0x573bf0 [jl]
 573c38: eax <= (signed) 0x6b49d200 -> 0x573c45 [jle]
 573cf4: esi >= (unsigned) 0xf -> 0x573ba2 [jae]
 573e7d: esi <= (signed) 0x19 -> 0x573e84 [jle]
 573e88: esi <= (signed) 0 -> 0x573ec1 [jle]
 573e9f: ebx < (unsigned) dword ptr [ecx + 0x118] -> 0x573ea5 [jb]
 573ebf: ebx < (signed) esi -> 0x573e95 [jl]
 573f37: ecx < (unsigned) eax -> 0x573f3d [jb]
 573f8c: ecx >= (unsigned) 0xa -> 0x573ba4 [jae]
 57406b: dword ptr [ebp + 0x318c] <= (unsigned) ebx -> 0x573ba4 [jbe]
 574090: ebx < (unsigned) dword ptr [ebp + 0x318c] -> 0x574080 [jb]
 5741ce: edi <= (signed) 0x14 -> 0x573ba4 [jle]
 5742ad: dword ptr [ebp + 0x31bc] <= (unsigned) edi -> 0x574307 [jbe]
 574305: edi < (unsigned) dword ptr [ebp + 0x31bc] -> 0x5742b5 [jb]
 574374: eax >= (unsigned) 5 -> 0x573ba4 [jae]

```

## Fifteen small port candidates

All fifteen are unported (checked against `port_*.rs`; 0x53C4D0 is only called from `port_big.rs`), under 150 bytes, have no unknown callees (only vtable calls or the named helpers listed) and were checked against one real caller for the register convention. "Hook" means how the original is replaced: since most take register arguments, use a naked shim (`jmp` detour after loading the Rust `extern "C"` arguments from the registers) or patch every caller; the existing `detour.rs` helpers do this for the ported leaves. Names are the rows in `candidates_activities.csv` unless noted.

### R1. 0x53C4D0 `activity_current_type` (32 bytes, 111 callers)
- ABI: no inputs; returns EAX (i32). Clobbers EAX, ECX, EDX only.
- Code: `m = *(0x27B3C60); if m == 0 -> -1; if *(m+0x3C) != 1 -> -1; info = *(m+0x90); return info.vtable[1](ECX = info)` (tail `jmp edx`: the vtable call returns directly to the caller). No null check on `info`.
- Polarity: `test eax,eax; je` (null -> -1), `cmp [eax+0x3C],1; jne` (not kind 1 -> -1). Result -1 is `or eax,-1`.
- Callee: `activity_info::get_activity_type` (slot 1, 3-6 bytes, returns a constant 0..16).
- Hook: plain `extern "C" fn() -> i32` detour at the entry (all 111 callers use it as a function call, no flags dependence).

### R2. 0x5DDA00 `fight_club_is_fighter_handle` (71 bytes, 91 callers)
- ABI: ESI = handle (u32); returns AL (0 / 1); clobbers EAX, ECX, EDX. Callers set ESI from `[obj+0x44]` (0x404C7D).
- Code: `m = *(0x27B3C60)`; false if `m == 0`, `*(m+0x3C) != 1`, `info.vtable[1](info) != 3` (fight club type 3, `cmp eax,3; jne`), or `esi == 0`. Else scan `i in 0..*(0x25276C8)` (u32 count, loop `jb`) and return true if `*(0x140A5C0 + 4*i) == esi`.
- Note: `info = *(m+0x90)` is dereferenced without a null test (`mov ecx,[eax+0x90]; mov eax,[ecx]`).

### R3. 0x5DE330 `fight_club_get_player_flag` (26 bytes, 10 callers)
- ABI: EAX = human pointer; returns AL.
- Code: `if eax == 0 -> 0; return (eax == *(0x21703D4)) ? *(u8*)0x25276C6 : *(u8*)0x25276C7`. The `mov al,[0x25276C6]` executes before the `je` test result is used (mov does not change flags), so equality selects 0x25276C6.
- Callers: 0x416917 (`mov esi,eax; call`), 0x5D9694...

### R4. 0x5DE0C0 `fight_club_get_countdown` (56 bytes, 4 callers)
- ABI: no inputs; **returns f32 in ST0** (callers do `fstp dword ptr [esp+..]` at 0x416C71). Clobbers EAX, EDX, ECX, ESI saved/restored.
- Code: if `m == 0 || *(m+0x3C) != 1 || (info = *(m+0x90)) == 0 || info.vtable[1](ECX = info) != 3` return the constant 5.0f (`fld dword [0xDD0794]`, bits 0x40A00000) else return `*(f32*)(info + 0x1A4)`.
- Hook: an `extern "C" fn() -> f32` detour is correct on i686 (cdecl returns floats in ST0); if the port is written as asm, end with `fld dword ptr [..]; ret`.

### R5. 0x5E50F0 `activity_hud_marker_find` (53 bytes, 52 callers)
- ABI: ESI = marker handle; returns EAX = pointer or 0. EDI saved/restored (push/pop).
- Code: `n = *(i32*)0x279C238; base = *(0x279C230); for i in 0..n (signed compare: `jle` skips when n <= 0, loop `jl`): if *(u32*)(base + i*0x104 + 8) == esi return base + i*0x104; return 0`.
- Callers: 0x5AF8F5 (`mov esi,[edi*4+0x25275C8]; call`).

### R6. 0x5E5130 `activity_hud_marker_index` (42 bytes, 1 caller 0x5E641C)
- ABI: ESI = handle; returns EAX = index or -1. Same scan as R5 but returns `i`; not found (or n <= 0) -> `or eax,-1`.

### R7. 0x5E6A80 `activity_hud_marker_remove_at` (74 bytes, 5 callers)
- ABI: EAX = index (kept in EDI); returns AL. Saves/restores EDI, ESI.
- Code: `n = *(0x279C238); last = n - 1; if (index >= last) (signed: jge) { *(0x279C238) = last; return 0 } else { memmove 0x41 dwords from base + n*0x104 - 0x104 to base + index*0x104 (rep movsd); *(0x279C238) -= 1; return 1 }`. Note the `index >= last` branch only decrements the count (index == last or beyond).
- Callers: 0x5B3630 (`mov eax,esi; call`), 0x5E6461.

### R8. 0x53D610 `activity_progress_get_levels_done` (63 bytes, 11 callers)
- ABI: ECX = pointer to the u32 name hash; returns EAX. ESI saved/restored.
- Code: `key = *(u32*)ecx; n = *(u32*)0x279719C; for i in 0..n (unsigned `jb`): if *(u32*)(0x2796E9C + 12*i) == key return *(u32*)(0x2796E9C + 12*i + 4); return -1`. An empty table (`test edx,edx; jbe`) also returns -1.
- Caller: 0x538B53 (`lea ecx,[esp+4]` holding `[info+0x12C]`).

### R9. 0x53D650 `activity_progress_levels_done_mask` (88 bytes, 10 callers)
- ABI: ECX = pointer to the name hash, **EDI = level index (i32)**; returns AL. ESI saved/restored.
- Code: find the 12-byte entry as in R8 (not found -> 0). Then `if (edi < 0) return 1` (`test edi,edi; jl`); `for cl in 0..=edi { bit = (u32)*(u8*)(entry+8) & (1 << cl); if bit == 0 return 0 }` (`shl edx,cl` with `edx = 1`; the byte is zero-extended so any cl >= 8 gives 0 -> false); return 1. Loop condition is signed `jle`.
- Caller: 0x539C2F (`lea ecx,[esp+0x14]`).

### R10. 0x6045E0 `diversion_start_blocked` (66 bytes, 10 callers)
- ABI: no inputs; returns EAX 0 / 1 (callers test AL).
- Code: return 1 if `*(0x2527888) != 0` (a diversion is running) or `*(0x27A3230)` is one of {4, 5, 1, 2, 3} (a chain of `cmp eax,k; je`) or `*(0x27A52A4) != 0` or `*(0xEA1020) == 2`; else 0.
- Caller: 0x56279D (pushback contest start) and `diversion_mode_frame` (0x6031C0).

### R11. 0x603E30 `diversion_current_type_id` (23 bytes, 22 callers)
- ABI: no inputs; returns EAX. Code: `inst = *(0x2527888); if inst == 0 -> -1; type = *(inst+8); return type.vtable[2](ECX = type)` (tail `jmp edx`).
- Hook: as R1.

### R12. 0x5E1D30 `activity_button_is_down` (135 bytes, 3 callers 0x5E4299 ...)
- ABI: EAX = button index (u32); returns EAX 0 / 1.
- Code: `if index > 7 (unsigned: ja) return 0 (xor al,al, EAX upper bytes kept)`; `jmp [index*4 + 0x5E1DB8]` entries `0x5E1D3C, 0x5E1D50, 0x5E1D64, 0x5E1D78, 0x5E1DB4, 0x5E1DB4, 0x5E1DA0, 0x5E1D8C`, i.e. the mapping index -> action: 0 -> 0x5E, 1 -> 0x5F, 2 -> 0x60, 3 -> 0x61, 6 -> 6, 7 -> 7 and **4, 5 -> default (return 0)**. Careful: the table order is not the action order (index 6 -> 0x5E1DA0 = action 7, index 7 -> 0x5E1D8C = action 6). Each case: `ECX = action; call input_action_down (0x4FF640, ECX = action, returns AL); return AL == 1` (idiom `sub al,1; neg al; sbb eax,eax; add eax,1` = `(al == 1)`; any other nonzero AL gives 0).
- Companion: 0x5E1DE0 `activity_button_just_pressed` (same table, `input_action_just_pressed`) and 0x5E1E90 `activity_button_name` (tail jump to `input_action_button_name`).

### R13. 0x66DC90 `drive_complication::roll_chance` (47 bytes, vtable slot 5, 8 classes, also `button_mashing_complication` 0x66F020)
- ABI: ECX = this; returns EAX 0 / 1. ESI saved/restored.
- Code: `r = rng_float01(EAX = 0xEC30F8)` (0xBDCE30 returns f32 in ST0; the code stores it to `[esp+4]` as f32 and reloads); `thr = *(f32*)(this+0x14)`; stack: `fld r; fld thr; fcompp` compares ST0 = thr with ST1 = r; `test ah,0x41; jne -> return 0` is taken for thr <= r or unordered. So the result is `thr > r` strictly (equivalently `r < thr`); NaN gives 0.
- Same shape: `heli_objective` slot 6 (0x574EC0) with threshold `[this+0xC]`.

### R14. 0x66DA90 `complication::remap_value` (22 bytes, slot 9, 13 classes)
- ABI: ECX = this, three f32 stack args (a, b, c), `ret 0xC`.
- Code: `fld [this+4]; fdiv a; fmul b; fadd c; fstp dword [this+4]`: all four operations in the x87 stack, rounded to f32 only at the final store. In Rust use f64 intermediates (or 80-bit if available) and `as f32` at the end; division by zero yields inf/NaN exactly as hardware.
- Companion: 0x66DAB0 `complication::activate` (`this[4] = value; this[8] = 1; this[9..0xB] = 0; *(0x27A57B4) = 0`, `ret 4`).

### R15. 0x53F3A0 `activity_instance_phase_wait_timer` (93 bytes, reached only by the phase jump table)
- ABI: EDX = activity instance; returns AL = 1 always.
- Code (`t = this[0x2C]` signed): if `t < 0`: `v = *(i32*)0xE843AC + 0x5DC; this[0x2C] = v; if (v > 0x6B49D200) this[0x2C] = v + 0x94B62E00` (wrap of the game-time counter, `jle` skips); return 1. Else `now = *(i32*)0xE843AC`: if `t > now` (signed): `d = t - now; if (d < 0x35A4E900) return 1 else this[0xC] = 8; return 1` (`jl` = still waiting; a huge difference means the clock wrapped); if `t <= now`: `d = now - t; if (d >= 0x35A4E900) return 1 (timer is in the future across a wrap) else this[0xC] = 8; return 1`.
- Port note: all comparisons are signed (`jl`/`jle`/`jge`); keep the 32-bit wrapping arithmetic (`wrapping_sub`).


## Surprises and doc errors

- Size over-estimates from tail jumps: `fraud_info::vf15` is 287 bytes, `piracy_level_instance::update` 610, `activity_instance::vf7` 0x238 (not 3885).
- `rtti_classes.csv` `n_virtuals` over-counts some diversion types (ambulance 41): slots past the real vtable are UTF-16 data.
- 37_caller_survey_2.md gives only the dispatcher for event 9; the diversion slot is 32 (`on_player_left`).
- `xtbl_load_taxi` / `poker_payouts` / `hitman` / `theft` loaders already exist at other addresses; 0x627E60 / 0x6352E0 / 0x640000 / 0x6153E0 are the table parsers.

## Unfinished (stopped early, out of budget)

- Port-map section prose for maps 2-10 is a first reading and partly hedged; the generated tables are exact.
- Many slot roles remain open (listed per family above); roles of `activity_info` slots 2-22 and `diversion_type_info` 1, 3, 9, 12-15, 18, 20, 30, 31, 34 were not resolved.
- Many names are `inferred` from strings only; verify before relying on them.
