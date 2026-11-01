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
3. **+0x18E8 bit 1 test, right after the revive.** When set: step 4, then straight to the HUD (5) and the zone bit (6). Otherwise:
   1. **Look target.** Unless +0x1162 bit 1 is set: unless +0xC0 bit 6 is set, when there is no grab (+0x100C == 0) and not `cutscene_blocking()`: aim point = camera (player) or +0x4A0 (others) + `human_view_offset_point` × 1000; 0x960C30(100.0). Then `human_set_look_at_point` (0x9B5C00; `AL` = bit 0 of 0x960C30's result, `EDX` = &point, `ESI` = 0, cdecl (human, f32)). With +0xC0 bit 0x40 set it is called with `AL` = 0, `EDX` = &human +0x564 and f32 −1.0 (`[0xE87998]`).
      - With +0x1162 bit 1 set instead: a bone transform (`rig_bone_transform`, `ECX` = rig +0xCE4) of the player position into +0x730..0x738.
   2. **AI or player.** If the human is not the local player's object (+0x44 handle check): `human_ai_think(human)`.
      - In single player or as host, when it has a firearm or melee weapon, is not dead and no cutscene holds the game: +0x11C8 = `human_find_aim_target(human, &+0x11CC)`.
      - Online host: return early (after `human_ai_think`), skipping the HUD and the zone bit (5-6) too.
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
4. **Not alive branch** (inside the bit-1 test of step 3, when +0x18D4 == 1): navpoint by hash (0xBDCA30, `navpoint_find`); if +0x591 == 0: 0x99DD20(`EDX` = 0, `EAX` = nav +0x1C, cdecl human, 1, −1, 0). Then the HUD (5).
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

## human_state_machine_update (0x9947F0) at instruction level

State byte +0x591, sub-state byte +0x592. The previous pair is kept in +0x593 / +0x594.

### `human_state_set` (0x9954C0)

`ECX` = state, `EDX` = sub-state, cdecl (human).
- **Lock:** while the human is in 7 / 0x15, only a change to 7 / 0x12 is accepted; any other call returns at once.
- Otherwise: +0x593 = old state, +0x594 = old sub-state, +0x591 = `CL`, +0x592 = `DL`.
- When the state really changes, it runs leave actions for the old state (for example, leaving 6 calls 0x6AEEE0(cdecl 0, 1), or (0xFA, 0) when going to 8). Those actions are not mapped here.

### The dispatcher

`EAX` = human; `ESI` = human throughout.
1. +0x53E = 0.
2. If +0x4A0 has value 4 set, +0x595 == 0, +0x591 == 0 and `timer_elapsed(&+0x4E4)`: 0xA1E5B0(`ECX` = 0, `ESI` = human).
3. `switch` +0x591 through the table 0x9948CC (states 1-8). State 0 and anything above 8 return.

| State | Call |
|---|---|
| 1 | tail `human_state1_update` (`ECX` = human) |
| 2 | Only if `human_owned_locally` (0x806720, `EAX` = human). Then +0x470 bit 0x40 set: `human_anim_set_update(cdecl human, 0, 1, 1, 0)`; clear: tail `human_state2_update` (`EAX` = human) |
| 3 | The same owner test and the same bit 0x40 alternative; otherwise `human_state3_update(cdecl human)` |
| 4 | tail `human_state4_update` (`EAX` = human) |
| 5 | `human_state5_update(cdecl human)` |
| 6 | `human_state6_update` (`ESI` = human, no stack argument) |
| 7 | `human_state7_update(cdecl human)` |
| 8 | tail `human_state8_update` (`EAX` = human) |

Five different calling conventions in one switch: keep each one when calling the originals.

**`human_owned_locally`** (0x806720, `EAX` = human → `AL`):
- 0 when the human is null.
- 1 when it has no net record (+0xB0), there is no session (`[0x2528C14]`), or the session has no +0x408.
- Otherwise 1 only when the record's owner byte equals session+0x408 +0x1B0 **and** record +0x67C bit 0 is clear; 0 in every other case.

### State 1 (0x998790), `ECX` = human

If the human's own handle +0x44 resolves in the object table (`0x2149C64 + (h & 0xFFF) × 16`, with +0x44 matching) to an object of type 1 (+0x4C) with +0x3CC == 1: `human_pkt_send_09(ECX = human, cdecl &+0x564, 0, 0, 1)`. Nothing else.

### State 2 (0x99CFF0), `EAX` = human (`EDI`)

1. Return if +0x470 bit 0x40.
2. m = +0x474. If m ≠ 2 and +0x498 == 0 and +0x49C == 0: **tail 0x99CC30** (`EAX` = human).
3. f = bit 5 of +0x80.
4. m == 0: +0x514 = −1, +0x474 = 1, then as m == 1.
5. m == 1: if `0x99C7B0(cdecl human, f)` is false, tail 0x99CC30 (`EAX` = human); otherwise return.
6. m == 2 with f clear: 0x95F090(`EAX` = human, cdecl &+0x4EC, 0, −1, 0); 0xBE49A0(`EAX` = &+0x4F8, cdecl &+0x20); 0x99CA90.
7. m == 2 with f set (an interpolated move):
   1. ok = 0x993CD0().
   2. t = 1.0. If the timer +0x510 is set and not elapsed: t = f32((200 − `timer_remaining`) × `[0xE5C5D0]`), clamped to [0, 1] (≤ 0 or NaN → 0).
   3. pos = +0x504 + (+0x4EC − +0x504) × t, per component, each step → f32.
   4. Unless `human_entering_or_exiting_vehicle`: pos.y = +0x18 (keeps the current height).
   5. done = (t == 1.0).
   6. 0x95F090(`EAX` = human, cdecl &pos, 0, −1, 0).
   7. `human_pkt_send_09(ECX = human, cdecl &(+0x4F8 + +0x14 … +0x500 + +0x1C), 0, 0, 0)`.
   8. If ok and done: +0x510 = −1. Then if +0x4A0 has value 4 set: 0xA1E5B0(`ECX` = 0, `ESI` = human), and 0x99CA90 only when it returns false. Without that bit: 0x99CA90.
8. Other values of m: return.

### State 3 (0x99E490), cdecl (human)

1. Return if +0x470 bit 0x40.
2. 0x735C80(`ECX` = +0x570, cdecl &T), where T is a 48-byte local. Then 0x9948F0(`EAX` = &T, cdecl human, &human+0x20).
3. r = 0xB19AE0(`EAX` = human, cdecl &T, &T+0x14). If r is 0 or −1: 0x99E650(`ESI` = human, cdecl 0); return.
4. If +0x474 == 2: copy T to the block at +0xE40 (3 dwords, then 9 dwords to +0xE4C), then 0x99DEA0(`EDX` = +0xE40, `ECX` = +0xE64, cdecl human).
5. `human_state2_update` (`EAX` = human): state 3 runs the state 2 move too.
6. If +0x470 has value 4 set:
   - d = 0xBDEA80(`EAX` = &T, `ECX` = +0xE40) (a distance).
   - If d ≤ `[0xE99A34]` or NaN: 0x99E650(`ESI` = human, cdecl 1).
   - Otherwise ok = (+0x474 == 2) ? 0x99E6D0(cdecl human, 0, 0) : (0x99DC70(`ECX` = 0, `EAX` = &T, cdecl human) ≠ 0).
     - ok: `human_anim_set_update(cdecl human, −1, 1, 1, 0)`;
     - not ok: 0x99E650(`ESI` = human, cdecl 0).

### State 4 (0x999980), `EAX` = human (`ESI`)

1. +0xC8 &= ~0x10. If +0x4C == 1 and +0x3CC == 1: `human_pkt_send_09(ECX = human, cdecl &+0x564, 0, 0, 1)`.
2. rig = +0x5E0; track = rig +0x168; p = f32(0x96F280(cdecl human)), the animation progress.
3. `switch` +0x592 (1-4, table 0x999B54):
   - **1:** go on if `anim_track_is_playing(ECX = *rig, cdecl [0x15B8794], track)`, or else if p ≥ K, where K = `[0xDD1B58]`; otherwise return. Then +0xC8 = (+0xC8 & ~0x01000000) | 0x800000; `human_state_set(4, 2)`, then **falls into 2**.
   - **2:** return if 0x999400() is true, or p < K (or NaN). Then 0x96F5F0(`ECX` = human, cdecl id, 0x53E, 0x10000, f32 0.0, 1.0, −1.0), where id = 0x3A if +0x61D or +0x61C is set, else 0x39. Then `human_state_set(4, 3)`.
   - **3:** return if 0x999400() is true, or p < 1.0 (or NaN). Then 0x999190(cdecl 1, 1) and 0x96FCA0(`ECX` = human, cdecl id, 0, 1, f32 0.0, 1.0, −1.0, 0, −1), where id = 0x3E if +0x61D, 0x3D if +0x61C, else 0x3C.
   - **4:** tail 0x999690 (`EAX` = human).

### State 8 (0x9989C0), `EAX` = human

If +0x592 == 0xF: `human_state_set(8, 0x10)`. Nothing else.

### State 5 (0x9976E0), cdecl (human): the climb / vault move

`EBP` = human. `EBX` = −1 in every path below.

Animation calls in this state share one argument shape: `0x96FCA0(ECX = human, cdecl id, 0, 1, f32 0.0, 1.0, −1.0, 0, −1)` returns `AL` (started). It is written **play(id)** below. p = f32(0x96F280(cdecl human)) is the animation progress. The track is rig +0x168, with rig = +0x5E0.

**Prologue.**
1. If the human's own handle +0x44 resolves to the local player `[0x21703D4]`: `[0xE8DD90]` = −1, player +0x1162 &= ~7.
2. **Record check.** handle = +0xF68:
   - it must be non-zero, with high word ≠ 0xFFFF and below 0x866;
   - list = `[0xF7AAE8 + hi × 4]`, which must be non-zero;
   - entry = `[list + 0x150]` + (lo − 1) × 0x20, which must be non-zero;
   - entry +0x18 must have value 2 set.

   If any test fails, go to the **abort tail** 0x997CAA:
   - 0x997420(`ESI` = human, cdecl 0);
   - if +0x570: +0x570 → +0x130 &= ~2;
   - 0x963090(`EAX` = human, cdecl 1, 0);
   - return.
3. `switch` +0x592 − 5 through the table 0x997CE8 (sub-states 5-11). Any other value returns.

**"Start" body**, shared by sub-states 6 and 7 after their play succeeds:
1. 0x735C80(`ECX` = +0x570, cdecl &+0xF04).
2. Copy 9 dwords +0x20 → +0xF1C (the start orientation).
3. 0x6FC610(`ESI` = human, cdecl 0x17, 0): the collision filter.
4. If +0x570: +0x570 → +0x130 |= 2.
5. +0xC0 &= ~0x200; +0xC8 &= ~0x1000.
6. `audio_play_params_init` (0x464D90; `ESI` = &params (local), `EDI` = &+0xF10, cdecl word `[0xE884A0]`, human, (int8) +0xF65).
7. `audio_play(EAX = &params, ECX = &local handle)`.
8. `human_state_set(5, next)`.

**Sub-states:**

| Sub | Does |
|---|---|
| 5 | `human_state_set(5, 7)` |
| 6 | play(0x1A9). Fails: 0x997420(`ESI` = human, cdecl 1); return. Succeeds: the start body with next = 8 |
| 7 | Return unless p ≥ 1.0 (NaN returns). id = 0x997360(`ECX` = human); play(id). Fails: as 6. Succeeds: the start body with next = 9 |
| 8 | p < 1.0 (`[0xDD518C]`): the **move step**. Otherwise: 0x95F090(`EAX` = human, cdecl &+0xF10, 0, −1, 0); copy 9 dwords +0xF40 → +0x20; then if +0xD0 has value 2: play(0x1A8), success → `human_state_set(5, 0xB)`; else play(0x1A4), success → `human_state_set(5, 0xA)`. A failed play: tail 0x997D10 (`EAX` = human) |
| 9 | p < 1.0: the move step. Otherwise: 0x95F090(`EAX` = human, cdecl &+0xF10, 0, −1, 0); copy +0xF40 → +0x20; with +0xD0 value 2: id = 0x997390(`ECX` = human), play, success → (5, 0xB); else id = 0x9973C0(`ECX` = human), play, success → `audio_play_params_init` with cue word `[0xE884A4]` (`EDI` = &+0xF10), `audio_play`, then (5, 0xA). A failed play: tail 0x997D10 |
| 10 | Go on if p ≥ 1.0, or else if 0xCECBC0(`ECX` = *rig, cdecl `[0x15B87B8]`, track) is true; otherwise return. Then the **finish** (below) |
| 11 | p < 1.0: return; otherwise tail 0x997D10 (`EAX` = human) |

**Move step** (8 and 9), `EAX` = human:
1. f = 0x997590() → ST0.
2. 0x9974C0(`EAX` = &+0xF1C, cdecl &out, &+0x20, &+0xF04, &+0xF10, &+0xF40, f32 f).
3. 0x95F090(`EAX` = human, cdecl &out, 0, −1, 0).

The move interpolates between the start (+0xF04 / +0xF1C) and the target (+0xF10 / +0xF40).

**Finish** (10):
1. 0x997D10 (`EAX` = human).
2. +0x1024 = 0.0; +0xC8 = (+0xC8 & ~0x8000) | 0x1000; +0x101C = +0x1020 = −1.
3. 0x999190(`ESI` = human, cdecl 1, 1).
4. play(0x9973F0(`ECX` = human)).
5. v = (0, 0, 0). If 0xBDDAE0(`EAX` = &+0x5D4) is true and 0xBDDA80(`ECX` = &+0x5D4) is false: v = +0x5D4, then `vec3_mul_mat33_in_place(EAX = &+0x20, ECX = &v)`.
6. 0x735E70(`ECX` = +0x570, cdecl &v): the exit velocity.

Constants: `[0xDD518C]` = 1.0, `[0xE87998]` = −1.0.

### State 6 (0x9989E0), `ESI` = human: airborne

16-byte aligned frame (`and esp, −16`) with a security cookie.

**Prologue.**
1. owned = the inline `human_owned_locally` test (0 when `ESI` is 0), kept at `[esp+0x17]`.
2. If +0x10CC is non-zero, its +0xC4 is non-zero, and that record's +0x2C ≥ 0: 0x9C5140(`EAX` = human).
3. If +0x4C == 1, +0x3CC == 1 and owned: q = 0x9D7DD0(`ECX` = +0x44); if q +0x1161 bit 0: 0x9DA290(`ECX` = q, cdecl 0).
4. +0xC8 &= ~0x10.
5. If the own handle resolves to a type-1 object with +0x3CC == 1:
   - `human_pkt_send_09(ECX = human, cdecl &+0x564, 0, 0, 1)`;
   - and when it is the local player: `[0xE8DD90]` = −1, player +0x1162 &= ~7.
6. V = the 16 bytes at (+0x570 → +8) +0x10, copied with `movaps` (the body velocity; V.y is used below).
7. Sub-state +0x592: 12 → A, 13 → B, 14 → 0x998810 (`EAX` = human), anything else → return.

**13 (B), take-off:**
1. If 0x9CBC60(`EAX` = human) and owned: `human_pkt_send_17(ECX = human, cdecl 0, 0)`.
2. G = `[0x2797400]` (a global speed, written elsewhere). If V.y > −G and +0xC3 bit 0 is clear: `human_state_set(6, 12)` and 0x6AE560(cdecl human, 0, 0). Otherwise (V.y ≤ −G or NaN, or the bit set): 0x6AEE30(`EAX` = human).
3. If `object_is_player_human(EAX = human)` and the human is the local player: 0x99F420(`EAX` = human, cdecl f32 `[0xE44850]` = 0.25).

**12 (A), in the air.**

*Ledge check.* It runs unless the previous state +0x593 is 5:
1. r = 0x7F5CD0(`EDX` = human, cdecl &P, &S, &k). P (3 floats) is a ledge point. S is a buffer whose +0x18 holds the normal N. k is a dword. r = 0 skips to the fall test.
2. dy = f32(P.y − human.y). With s = `[0xEB7C20]` (a global, 1.85 at load: the human height), go on only if 0.5·s ≤ dy ≤ 1.25·s. A NaN skips.
3. Build a ray query object at a local Q: 0x418540 (`ECX` = &Q.ray) is the ray constructor, and the result block R (3 dwords) = {0, 0, 0x80000000}.
4. d = human.pos − P, each component → f32. dot = f32(N.x·d.x + N.y·d.y + N.z·d.z), summed in the x87 in that order. If dot > 0.0: off = N × 0.25; otherwise off = −N × 0.25 (each → f32).
5. Ray start = P + off (each → f32); end = start with y − 10.0. Query +0x4C = 0x4C (the mask).
6. hit = 0x6FA330(`ECX` = &query, cdecl &R, 1).
7. If hit and f32(P.y − R.first +4) < 0.75: no ledge. Destroy R (0x4080B0, `ECX` = &R) and go to the fall test.
8. Otherwise (no floor within 0.75 below the ledge): 0x997E20(`EAX` = &S, `ECX` = &P, cdecl human, k, r), destroy R, **return**. 0x997E20 starts the climb, which is state 5.

*Fall test* (V.y on the x87):
1. If +0xC8 bit 0x1000 is set, or |V.y| (→ f32) ≥ 0.1 (or NaN): go to the **dive test**.
2. Otherwise the human is stalled in the air, with timer T = +0xF6C:
   - T unset: T = now + 750 ms, wrapped by −0x6B49D200 above 0x6B49D200; return.
   - T set and elapsed:
     - if owned: `human_apply_hit(cdecl human, 0, 0, 0, 0, 0, 1, 0)` and +0xC0 &= ~2;
     - 0x6AE180(`EAX` = human, cdecl 0x1E7, f32 −1.0, 0xFFFF, 1);
     - +0xC0 &= ~0x200; T = −1;
     - return.
   - T set, not elapsed: return.

*Dive test:* if V.y < −G: `human_state_set(6, 13)` and +0xC8 &= ~0x1000. Otherwise return.

Constants: 0.5 `[0xE5BF70]`, 1.25 `[0xE5BF48]`, 0.0 `[0xDD32C0]`, 0.25 `[0xE5BFC8]`, 10.0 `[0xE5C1B8]`, 0.1 `[0xDD50E8]` (a double, compared with the f32 |V.y|), 0.75 `[0xDD1B58]`.

### State 7 (0x99A2D0), cdecl (human): knockdown / ragdoll

**The role, corrected.** State 7 is the **knocked-down (ragdoll) state**, not an arrest / respawn sequence:
- `human_apply_hit` (0x9AC180) enters it as 7 / 0x11 (three call sites).
- Code right after it chooses 7 / 0x15 instead of 7 / 0x12 when the human is owned locally and its flag argument is non-zero.
- Only sub-state 0x13 is an **arrest**. 0x9AD1E0 (`EDI` = human) enters it, only from a ragdolled human (+0x595 set) in 7 / 0x11:
  - for the player it plays anim 0x286 (0x6AE180), then 0x719510, +0x628 = 0, +0x595 = 3;
  - its one caller is 0x4058E0 (`EDI` = the player), reached from slot 23 of the vtable 0xDD5568 (class not identified);
  - 0x13 ends in `player_busted_handle` ("You got busted.", mission failed).
- 0x15 is a **recovery**: a fade-out, a teleport to the saved point +0x63C, then a fade back in. It is not a respawn.

`EBP` = human. Sub-state +0x592 − 0x11 through the table 0x99A9B4 (0x11-0x15); anything else returns.

**0x11, down:**
1. If owned and f32(+0x654 − +0x18) > 4.0 (it fell more than 4 m):
   - 0x983540(`ECX` = human, cdecl &W);
   - if W.y ≥ −3.0 (or NaN): 0x6AEEE0(`ESI` = human, cdecl 0, 1); otherwise 0x6AEE30(`EAX` = human).
2. Return unless 0x99A1E0(`EAX` = human).
3. b = (0x9ACC30(`EDX` = human) == 0).
4. If 0x9ACC90(cdecl human, 1): +0xAE8 = +0xAEC = −1; return.
5. If b is false: +0xAE8 = now + 750 ms (wrapped); return.
6. Otherwise, an **unstick** probe:
   1. Ray query Q with the result block R = {0, 0, 0x80000000}.
   2. pos = 0x735C80(`ECX` = +0x570). The ray goes from pos to pos + (0, 5.0, 0) (y → f32). Mask +0x4C = 0x60, +0x50 = 1.
   3. hit = 0x6FA330(`ECX` = &Q, cdecl &R, 1).
   4. On a hit, the last hit record is R.ptr + R.count × 0x3C − 0x30. The target is that record's x, y, z plus H × (`[0xEC5ACC]`, `[0xEC5AD0]`, `[0xEC5AD4]`) = H × (0, 1, 0), with H = `[0xEB7C20]`, each product and sum → f32. Then `object_teleport(EDX = &target, ECX = &+0x20, cdecl human, 0, 1, 1, 0)`.
   5. Without a hit: 0x9ACC90(cdecl human, 1).
   6. Then +0xAE8 = +0xAEC = −1; destroy R (0x4080B0); return.

**0x12, get up:**
1. **Not ragdolled** (+0x595 == 0):
   - if 0x717AC0(`ESI` = human): 0x7183D0(`ECX` = human);
   - 0x7152A0; +0xCC |= 0x400;
   - `human_state_set(0, 0)`; return.
2. **Ragdolled.** Find the duration s:
   - default s = 0.5 (`[0xDD1C8C]`); scale m = 0.3 (`[0xE8B048]`), or 1.0 when +0xD0 bit 0x80 is set;
   - scan the 3 rig animation slots (rig +0xD4, stride 0x34) for the first one that is active (+0x10 bit 0) and plays animation 0x54D. An inactive slot after slot 0 ends the scan.
   - If such a slot is found: when its index (slot − 8) is in [0, `[0x2527D6C]`), s = 0xCEC570(`EAX` = `[0x15B8800 + idx × 0x54 + 0xC]`), the animation length; then s ×= m.
   - If no slot is found, s stays 0.5 and is **not** multiplied.
3. t = f32(+0x628 + `[0xE8437C]` / s).
   - **Correction:** an earlier version said the unclamped t is stored. It is not: `fst [+0x628]` at 0x99A63C writes the unclamped t, but the clamped value goes to [esp+0x14] at 0x99A662, and after `push ebp` that slot is reloaded as [esp+0x18] and stored over +0x628 at 0x99A66B. The final value is clamp(t, 0, 1).
4. Return unless p = 0x96F280(cdecl human) ≥ 0.85 (`[0xE5C0E8]`, a double).
5. 0x717AC0 / 0x7183D0 as in step 1; 0x9ACB10(cdecl human, 0); +0xCC |= 0x400.
6. If +0xCC bit 0x20000000 and +0xB70 bit 0x80: 0x5B65D0(`ECX` = human). Otherwise, if owned and (+0x3CC ≠ 2 or 0x43F100() is false): 0x96E7E0(`EDI` = human).
7. If 0x5DDA00(`ESI` = +0x44): 0x415920(`ESI` = human).

**0x13, arrested:**
1. +0x628 = clamp(f32(+0x628 + `[0xE8437C]` / 1.5), 0, 1); here the **clamped** value is stored.
2. Return unless p ≥ 0.75.
3. 0x717AC0 / 0x7183D0.
4. `human_state_set(7, 0x14)`.
5. 0x6FC610(`ESI` = human, cdecl 0x15, 0); 0x735EF0(`ECX` = +0x570, `EDI` = 0x11, cdecl 1); 0x963090(`EAX` = human, cdecl 1, 0).
6. +0xC0 &= ~0x10000000; +0x64C = 0.
7. Only for the local player: `player_busted_handle()`, then 0x493C80(`ECX` = 7).

**0x14:** return. The human stays held (busted) until something else sets its state.

**0x15, recover:**
1. If not online host and the human is the local player and the fade is not full:
   - return if `[0x2526D9C]` == 1 (already fading out);
   - otherwise `screen_fade_start(cdecl 250, 0, f32 1.0)` and +0xCC |= 8; return.
2. If +0x595 (ragdolled):
   1. If 0x925460(`EDX` = &+0x63C, `EDI` = &L): `object_teleport(EDX = &L, ECX = &+0x20, cdecl human, human_equals_local_player(EAX = human), 0, 1, 0)`.
   2. If +0xCC bit 0x10: clear it. For each of the +0xC3C carried records (0x18 bytes, position at +0xC48 + i × 0x18):
      - o = `vehicle_seat_occupant(EAX = record −8)`;
      - if o: dir = (human.x − rec.x, 0, human.z − rec.z) → f32; 0xBDE150(`ESI` = &dir) (normalize); 0xBE49A0(`EAX` = &dir, cdecl &M) builds an orientation;
      - `object_teleport(EDX = &rec pos, ECX = &M, cdecl o, 0, 1, 1, 0)`.
   3. 0x719310(`ECX` = human, cdecl rig, &+0x14, &+0x20, &+0x14, &+0x20). +0x595 = 3. 0x9AD780(`ESI` = human). For the local player: `camera_cut`.
3. `human_state_set(7, 0x12)`. If +0xCC bit 8: clear it and fade back in: 0x4D1270(cdecl 250, f32 0.0, `DL` = 0).

Constants: 4.0 `[0xE5C270]`, −3.0 `[0xE5CE70]`, 5.0 `[0xE5C130]`, 0.85 `[0xE5C0E8]`, 1.5 `[0xE5C408]`, 0.5 `[0xDD1C8C]`, 0.3 `[0xE8B048]`, 0.75 `[0xDD1B58]`. dt = `[0xE8437C]`.

**Port order:**
1. The dispatcher, `human_state_set`, and states 1, 4 and 8.
2. States 2 and 3.
3. State 6, then 5, then 7.
4. Throughout, keep the f32 store after each arithmetic step, and the double constants (0.1, 0.85) compared against f32 values. (State 7 / 0x12 ends with the clamped t in +0x628; see the correction above.)

## Open

- What 0x9C1B90 (AI think) runs per behaviour, and the human state machine values at +0x591 / +0x592.
- The rig record layout (`0x340D240`).
