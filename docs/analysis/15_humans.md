# 15: The human update

Names are in `docs/functions.csv` (subsystem `human`). Humans are objects of type 1 (12_objects.md). The frame order is in 11_frame.md.

## Lists

| List | Link | Contents |
|---|---|---|
| `0x215E908` | +0xB8 | every human in the world |
| `0x217043C` | +0x1294 | humans with AI (`0x9D8210` also walks it) |
| `0x215FFC0` | +0x1160 | a second group (0x9CF8F0 per entry; probably squads / groups) |

The local player is `0x21703D4`. A human is "a player" when type == 1 and **+0x3CC == 1** (`object_is_player_human`).

## Human fields used by the update

| Off | Field |
|---|---|
| +0x14 / +0x20 | position / orientation (object base) |
| +0xB8 | next in `0x215E908` |
| +0xC8 | flags: 0x100000 = in the world (updated), 0x2 = visible / rendered, 0x800 (secondary animation path), 0x20000000 |
| +0xCC | flags: 0x200 = has a character controller, 0x8000000 = fell out of the world (handled) |
| +0x3CC | u8 player kind (1 = player) |
| +0x570 | Havok character proxy |
| +0x591 / +0x592 / +0x595 | state bytes: +0x592 == 0x12 is a special state (0x9A5100); +0x595 set = not driven by movement (in a vehicle / attached) |
| +0x5C8 … +0x5D0 | velocity |
| +0x5E0 | animation rig: an index into the rig table `0x340D240` (0x5E8-byte records, index % 0xBE) |
| +0xF00 | current neighborhood record (`0x2526C94`, 0xA8 bytes each) |
| +0xF8C | death time (-1 while alive) |
| +0x10CC | held item / weapon (+0xC4 its object) |
| +0x1124 | hit points (float) |
| +0x1554 | revive timer (co-op) |

## Order within one frame

All of this runs on the game thread from `sim_frame`. The animation work also runs on the two job workers.

**sim_frame:1c**

1. **`humans_update_neighborhoods`** (0x9A7080): for each human, look up the neighborhood at its position (0x4B19F0).
   - Players: on a change, call `0x9E1190(neighborhood)` (enter-neighborhood events).
   - Others: only store the neighborhood in +0xF00.

   Then 0x9CFB30, 0x98F680, 0x98DCC0 and 0x986060 (crowd / population upkeep).
2. **`humans_animation_jobs`** (0x9A7150): wake both job workers with work kind 4, run 0xCEDFF0 and 0xCED090 (the animation system, 0xCE…), then set the workers back to kind 3. Animation blending runs in parallel here.
3. **`humans_ai_update`** (0x9A71B0):
   1. For each human in the AI list `0x217043C`: 0x9A9010, then the AI think 0x9C1B90(human).
   2. For each entry of `0x215FFC0`: 0x9CF8F0.

**sim_frame:4 (around the physics step)**

1. **`humans_movement_update`** (0x9A9890, dt), for humans not in a vehicle / attached:
   1. Collect up to 255 of them (0x985630 filters out the ones that do not move themselves).
   2. Apply any turn from the animation rig to the orientation.
   3. Solve the movement (0x9A1970). The result is the desired position (`position + velocity × dt`) written into the rig at +0x1CC.
   4. 0x6F1620 (ground / collision probe at position and orientation).
   5. 0x9A2670 (apply).
2. **`sim_physics_step`**, per substep: **`humans_character_step`** (0x9A9B60)(step, players = 1). For every player human with a character controller (+0xCC & 0x200), gated by 0x6C6D90 / 0x6C6E80 / 0x6C6D20 / 0x9632E0:
   1. 0x736A10(step) per human.
   2. Step the Havok character proxies (+0x570): 0x736BC0(proxy, step) each, or `0x9A6110` (all at once, when `0x252A3E8` is set).
   3. 0x736CC0(proxy, human): copy the result back to the human.
3. After the step: `humans_character_step(dt, players = 0)` does the same for every other human, so NPCs move once per frame and players once per substep.
4. 0x9A9CE0 (after the step).

**sim_frame:5**

**`humans_post_update`** (0x9A7210):
1. **First pass**, humans in the world:
   1. If +0x595 is set (and +0x592 != 0x12), 0x9A4A80.
   2. For live ones (hit points > 0, not dead), 0x983370 for the player in or out of a vehicle.
   3. 0x9A5840 for humans not attached.
2. **Second pass**, per human:
   1. Expire the co-op revive timer +0x1554 for players.
   2. 0x99F6D0, and 0x99F4A0 when flag 0x20000000 is set or +0x591 == 6.
   3. Dead humans: 0x9A4FF0, 0x9A5080. State 0x12: 0x9A5100.
   4. **Animation to render**: when visible (+0xC8 & 2) and not culled, 0x6DACC0 (rig, position, orientation), then 0xCE7D00 and 0x9A4970.
   5. 0x9A5AD0, 0x9A46D0.
   6. **Animation update**: 0x735FE0 for dead / dying humans, or 0x7360F0 when flag 0x800 is set.
   7. 0xB6CDA0.
   8. **Held weapon**: compute its world transform (0x9C31B0) and write it into the weapon object's +0x14 / +0x20. Fire / effects: 0x465B90 (muzzle sound / effect position), 0x9707C0 when out of ammo, 0x9C8EE0. Then 0x9BD360, 0x9BDA80 and 0x9C1A30.
   9. **Fell through the world**: grid index -1 or y < -500.
      - An NPC (when this machine owns it) is destroyed: "Human destroyed because it fell through the world", 0x9847A0, then vtable +0x10.
      - A player is recovered (0x9A3E20 / 0x9A4240, or a fade with 0xD14840 / 0x4D1090 and 0x505430).
   10. Clear +0x5B0 for humans in the world.
3. 0x6DD990, then 0x737C10.

## Player vs AI

- **Player humans** move from input through the character controller (0x736…) once per physics substep. Their neighborhood changes raise events, and they are recovered instead of destroyed when they fall out of the world.
- **AI humans** think in `humans_ai_update` (0x9C1B90), move by animation-driven movement (`humans_movement_update`) and step their character proxies once per frame.

## Animation and physics hookup

- **Animation**: each human's rig is a 0x5E8-byte record in `0x340D240`. It is updated on the job workers (0xCEDFF0 / 0xCED090), feeds movement through +0x1CC, and is posed for rendering in `humans_post_update` (0x6DACC0).
- **Physics**: humans are **Havok character proxies** (+0x570), not rigid bodies. They are stepped by the human code inside `sim_physics_step`, using `havok_custom_character_collector` for contacts. Ragdolls (death state) use 0x735FE0.

## human_update (0x9D6D60) porting map

cdecl (human, f32 dt), from `humans_update_all`. Frame about 0x314 bytes with a security cookie.

1. +0x1160 byte &= ~4; `human_update_core(cdecl human, dt)`.
2. **Co-op revive.** If +0x1161 bit 4 is set and +0x129D == 1 and `human_assets_loaded` (`EDI` = human): when no session player is below state 13, or the session flag `0x2528B56` is set with bit 4:
   1. clear bit 4; call vtbl +0x1C(1, 1);
   2. clear bit 1 of +0x570 → +0x130;
   3. if `ui_blocks_player()`: +0x80 |= 0x14.
3. Unless +0x18E8 bit 1 is set:
   1. **Look target.** Unless +0x1162 bit 1 is set: unless +0xC0 bit 6 is set, when there is no grab (+0x100C == 0) and not `cutscene_blocking()`: aim point = camera (player) or +0x4A0 (others) + `human_view_offset_point` × 1000; 0x960C30(100.0). Then 0x9B5C00(cdecl human, value).
      - With +0x1162 bit 1 set instead: a bone transform (`rig_bone_transform`, `ECX` = rig +0xCE4) of the player position into +0x730..0x738.
   2. **AI or player.** If the human is not the local player's object (+0x44 handle check): `human_ai_think(human)`.
      - In single player or as host, when it has a firearm or melee weapon, is not dead and no cutscene holds the game: +0x11C8 = `human_find_aim_target(human, &+0x11CC)`.
      - Return early, skipping steps 4-6.
   3. **Local player:**
      1. Vehicle push-out: when the timer +0x14DC is set and has elapsed, and the vehicle is not a slow type 0x11, `vehicle_interaction_query`.
      2. Ragdoll / held object (+0x1163 bits 8 / 0x10, 0x9D6CF0, 0x9DA130, 0x9D6BF0, velocity × 20).
      3. Zoom FOV `0x25F5D38`, clamped to [0.1, 179.9] (+0x1161 bit 0, timers +0x11EC).
      4. 0x9D5A60.
      5. For the local player: time counters, timed stat awards (`stat_award_check` when `stats_can_award`), the HUD timer text (0x690710, `wsprintf_1024`).
      6. 0x9D53F0, 0x930F30, `human_pickup_scan`, 0x491AA0.
      7. Lock-on: `player_lockon_update` / `player_lockon_target_find` when it has a weapon.
      8. Low-health heartbeat: `audio_play_params_init` / `audio_play` with timer `0xEB7E14`, tint (0xD14700); stop it with `audio_voice_stop`.
      9. The vehicle-damage timer +0x14E0 and the hit flash +0x1560 (tint, `timer_remaining`).
      10. +0x1163 bit 0x40 (an enemy is targeting the player).
4. **Not alive branch** (+0x18E8 bit 1, +0x18D4 == 1): an effect (0xBDCA30, 0xA5EB50), then 0x99DD20(human, 1, −1, 0) when not dying.
5. **Local player HUD:**
   - `hud_vehicle_meters_update`, 0x796AB0, `hud_weapon_update`;
   - 0x797CD0 (not online host only);
   - 0x798770, 0x798AF0, 0x78E490, 0x9F1950.
6. +0x1164 bit 1 = 0x9D6C60 (both zone tests 0x4AB430 true).

**Helpers named here.** The four timers are used all over the game; each takes a pointer to a ms timestamp where −1 means unset:
- `timer_elapsed` 0xBDD230 (`EAX` = &ts → `AL`);
- `timer_remaining` 0xBDD260 (`EAX` = &ts → `EAX`);
- `timer_set` 0xBDD100 (`EAX` = ms, `ECX` = &ts);
- `timer_just_elapsed` 0xBDD390.

The others:
- `cutscene_blocking` 0x6C6D20;
- `audio_play_params_init` (`ESI` = params, cdecl cue, a, b) / `audio_play` (`EAX` = params, `ECX` = &handle) / `audio_voice_stop` (`ECX` = &handle);
- `human_has_firearm` / `weapon_info_is_firearm`, `ui_blocks_player`, `stats_can_award` / `stat_award_check`.

**Port:** keep this as a call skeleton. The AI branch returns early, so the local-player part (step 3.3 onward) never runs for NPCs. Several bit tests of +0x1161..+0x1164 decide the branches; copy them as they are.

## human_update_core (0x9A7DF0) porting map

cdecl (human `H`, f32 dt), the first call of `human_update`. No security cookie.

1. **Return** if H is 0 or H+0x80 & 0xB. Clear H+0xCC bits 0x3000 and H+0xC08 bit 0x80. Previous position +0x50..0x58 = +0x14..0x1C.
2. **Passenger test.** A is set when all of these hold:
   - the handle is live and not the cutscene actor `[0xEBCE9C]` (unless `[0xEBCEA0]` is 7 or 8);
   - not `human_entering_or_exiting_vehicle`;
   - `vehicle_from_handle` gives a vehicle;
   - not a player driver;
   - 0x9A62D0 is false.
3. **Time scale H+0xD64** (per-human slow motion), with the minimum M = `[0x2529718]`:
   - H+0xC8 bit 0x400: ramp up, r = `timer_remaining` / H+0xD6C; the bit clears at the end;
   - bit 0x800: ramp down, 1 − r; at the end, M and the bit clears;
   - when `[0xE8825F]`: humans listed in `0xF53908` (3 slots) fall at dt × 1.3333 (f32 constant); humans in `0xF53914` (3 slots) rise at that rate and call 0x492D00 above 1.0;
   - otherwise: if 0x9A62D0 was true in step 2 (in a vehicle), the scale falls by 2·dt down to M; if A, it rises by 2·dt up to 1.0; else it is 1.0;
   - the result is clamped to [M, 1.0]; the compares are written to keep NaN out.
4. **Animation clock** H+0x5B0 +=:
   - dt normally;
   - the **real** frame time `[0xE84380]` while a cutscene plays (`[0x2527D14]`, cutscene +0xAFF clear, real time ≤ 0.5) for script-controlled players (+0x4C == 1 and +0x3CC == 1) or when 0x6C6E80 is true;
   - otherwise dt.
5. Alive with a hold H+0xC20: release it, `human_take_damage(H)`, clear +0xC20 / +0xC28.
6. Pending knockdown (H+0xC24 == 1 and +0xC0 bit 0x400 clear): `human_knockdown(H, 0x98E2A0(0, 0, 0))`. Then +0xC24 = 0.
7. **Collision-ignore window:** +0xC8 bit 0x40 (and not 0x200000): 0x7190C0(0), clear the bit, timer +0xF94 = now + 100 ms; when it passes: 0x7190C0(1), +0xF94 = −1.
8. 0x96E400(H).
9. **Reload:** when 0x9C81E0, for the held weapon (+0x10CC → +0xC4) of a firearm type: `weapon_reload_step` once, or (clip size − ammo) times when the reload animation (+0xD0C, `anim_track_is_playing`) finished.
10. Grab release: +0xD0C ≠ −1 with no live weapon → +0xD0C = −1, and 0x960A80(1, 0) when `human_owned_locally` and not the player (or +0x1164 bit 4).
11. **Cutscene LOD** (+0x3CC == 2, while no blocking cutscene or +0x1074 is set, and the rig flag +0x26C == 1):
    - distance to the camera `0x25F5B14`: ≥ 20 m clears +0x1074;
    - otherwise the weight w = 1 − (d − 15) × 0.2 above 15 m, then 0x95BE10(...), and +0x1074 = 1.
12. 0x989630, 0x9B6C70(H), 0x9B0F10(H), 0x9A4CB0.
13. **Anim override clear:** for a local or unflagged human with +0xC0 bit 0x400000 and no rig action (+0x164 == −1): `net_send_prop_human_anim_state_override(&−NaN)`; clear +0xC0 bits 0xC00000.
14. 0x9C9F80(H) when +0xC0 bit 0x40000 and handle +0x374 is live.
15. Rig action 0x1AD / 0x1AE with the anim finished and +0xEF8 ≠ −1: 0x91AF30(H, action == 0x1AD).
16. 0x96F3A0(H) action with table `0xE90E48` bit 0x400 and its anim playing: +0x101C = −1, +0x1024 = 0, +0x1020 = −1.
17. Not a passenger (the inverse of step 2's handle / vehicle part): 0x9947F0.
18. **Not ragdolled** (+0x595 == 0): an animation-driven hit (anim `0x15B87C4` playing, local or flagged +0xCC bit 0x10000000):
    - `0x9AC180(H, ...)`;
    - `human_take_damage(H, amount, 0, weapon)` when the hit has damage;
    - 0x6AE560(H, 0x714, 0) for the flag.

    **Ragdolled:** 0x710DB0 / 0x7152E0 (impulse), then 0x718260 speed → +0xC0 bit 0x80000 = speed² ≥ 64 (or ≥ 9 with bit 0x10000000).
19. The local player with +0xC0 bit 0x400 and not 0x7C5770: 0x6AE180(0x1E2, −1.0, 0xFFFF, 1).
20. 0x4372E0. **Auto-fire** (+0xC8 bit 0x10, not +0x10C8, `human_has_firearm`, 0x994BC0, 0x9C2FD0): `weapon_fire(H, ...)`.
21. **Combat timer** +0xF78: in state 3 (+0x590), now + 5000 ms. Otherwise, once elapsed and not in a vehicle transition: firing / reload checks (`weapon_clip_size`, `weapon_reserve_ammo`, 0x9C53C0) may clear it (−1), or it is re-armed for 5 s.
22. **Nearby-threat scan** (0x98E8A0, 0x98E710): over the human list 0x215E908 (next +0xB8), find a live, active (+0x80 bit 0x20), non-dead human within 1 m on the ground plane (0x412080). The result bit drives 0x735EF0(1) when it disagrees with the move state (index 5 in the 28-entry table at +0x570).
23. 0x96CB70(H, −1, 1, 1, 0) for weapon type 10 with +0xC0 bit 0x40.
24. **Animation:** `anim_rig_prepare(rig, 1)` unless hidden (+0x80 bit 4, or a model flag); then **`anim_rig_update(rig, dt)`**.
25. 0x9A68D0 (the large post-update: visibility, LOD, attachments).
26. In a network session (gate as in doc 22): clear +0xC0 bit 0x400 unless 0x98E270.

**Port:** a call skeleton of 26 steps. Steps 3-4 (time scale and clock) are the only x87-heavy parts; keep their f32 stores. The animation update (24) is where most time goes.

## Open

- What 0x9C1B90 (AI think) runs per behaviour, and the human state machine values at +0x591 / +0x592.
- The rig record layout (`0x340D240`).
