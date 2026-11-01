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

## Open

- What 0x9C1B90 (AI think) runs per behaviour, and the human state machine values at +0x591 / +0x592.
- The rig record layout (`0x340D240`).
