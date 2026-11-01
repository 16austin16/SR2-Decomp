# 16: The vehicle update

Names are in `docs/functions.csv` (subsystem `vehicle`). Vehicles are objects of type 5 (12_objects.md); the frame order is in 11_frame.md.

## Lists

All three lists link through vehicle +0x84DC.

| List | Contents |
|---|---|
| `0x22A84E0` and `0x22A84D4` | full vehicles (simulated with a rigid body) |
| `0x22A84DC` | lightweight vehicles: traffic moved along the road network without full physics (`vehicle_update_light`) |

## Vehicle fields used by the update

| Off | Field |
|---|---|
| +0x14 / +0x20 | position / orientation (object base) |
| +0x38 … +0x40 | velocity (used for the miles-driven stat) |
| +0x50 … +0x58 | position last frame |
| +0x80 | object flags. Bits 0, 1, 3 mean skip the update. Bit 4 (0x10) and bit 5 (0x20) select the reduced paths below. |
| +0x84 | flags: bit 0 = has a driver / is in use, bit 1 |
| +0x94 | submerged depth (float; > 0.5 starts the drown timer) |
| +0x538 | wheel / physics data (only used when +0x84D8 == 1) |
| +0x84D8 | u8: 1 = full physics vehicle |
| +0x84DC | list link |
| +0x84E4 | **vehicle info** (from the vehicle xtbls). +0x9C is the vehicle class: 0 / 1 / 2 / 5 land vehicles (car, bike, …), 3 boat, 4 aircraft |
| +0x8554 | the **vehicle AI record** (`vehicle_ai`): the AI orders and `vehicle_ai_drive` use it, and it is cleaned up on destroy (0xB25D60). It was earlier read as effects (fire / explosion); see 25_network.md, "Receivers" |
| +0x8F48 / +0x8F4C / +0x8F50 | flags (+0x8F50: 0x10 / 0x100 networked delete, 0x40 do not count, 0x10000000) |
| +0x9058 | drown / destroy timer (game time; -1 = off) |
| +0x905C | multiplayer flag |
| +0x9060 | owner reference (non-zero = special handling) |

## vehicles_update (0xAE3C60), sim_frame:5

1. Collect up to 32 vehicles from `0x22A84E0`, then `0x22A84D4`. For each one not mounted on a carrier (+0x9050) and not flagged (+0x80 & 0xB), run **`vehicle_update`** (0xAB3550).
2. For each vehicle in the lightweight list `0x22A84DC` (not flagged), run **`vehicle_update_light`** (0xAB4460).
3. If more than 4 lightweight vehicles are counted (+0x8F50 & 0x40 not set), run `vehicles_cull_light` (0xAE1CF0).
4. 0xAC2080 returns a list of 12-byte entries; run 0xAE3B30 on each (vehicle-related events for humans: 0x960D60, 0x9D7DD0, 0x9D06A0).

## vehicle_update (0xAB3550)

1. **Network deletes**: a networked vehicle with the delete flags set is deactivated (vtable +0x10).
2. **Drowning / destruction** (not aircraft, +0x8F50 & 0x40 clear):
   1. Submerged deeper than 0.5 with a driver → arm the drown timer at now + 3000 ms.
   2. When it expires, the vehicle is destroyed: 0xAABCC0(vehicle, 1000, 1, 0) (damage), the explosion effect (0x427410 / 0x601540 → 0xA70110, 0xCE01B0, 0xB2A780), and **`game_event_vehicle_destroyed`**, then 0xB25D60(+0x8554).
3. 0xB04B70 (seat / occupants), multiplayer 0xA98C90, 0xA97E90.
4. **`vehicle_ai_drive`** (0xB2DD90): AI driving (the `vehicle_ai` / "vai" code at 0xB2…). It runs for every vehicle, and AI-driven ones steer here.
5. 0xAAAB60 (attached object), then the **wheel / suspension and engine** model: 0xAD1B30, 0xAB3440, 0xAD1F10 (sounds through 0x50DDB0), 0xD079E0 (+0x20), 0xB6CC40.
6. **Reduced paths**:
   - Object flag bit 4 (sleeping / parked): only 0xAB33E0 (and 0xACC920 for the watched vehicle), then return.
   - Bit 5 clear (not simulated by physics this frame): 0xAA3470. Full physics vehicles (+0x84D8 == 1) run `vehicle_sync_from_physics` (0xAB26E0) and vtable +0x20, and may push the vehicle out of geometry (0x6F4550). Then 0xAB33E0 and return.
7. **Full path** (physics):
   1. `vehicle_sync_from_physics` (0xAB26E0), 0xAB33E0.
   2. By vehicle class: if the vehicle is moving or has input, 0xD58FF0 (activate the rigid body).
   3. 0xAB5CD0, 0xAA68E0.
8. **If this is the player's vehicle** (`[0x21703D4]+0xD74` == handle):
   1. Set the camera parameters `0x25F5CB0` / `0x25F5CB8` by vehicle class and camera mode.
   2. Set flag 0x800 (0xAA8290).
9. **`vehicle_drive_update`** (0xA9DCF0, (vehicle, frame dt)): applies the driver's controls to the vehicle body (0xD61400 rigid body, 0x484A40, 0xAD0370).
10. **Player stats**: distance moved this frame, in miles (× 3.2808 / 5280), goes to the "miles by car / bike / watercraft / aircraft / vehicle" stats through the stat callbacks `0x27DA820`.
11. 0xB23AF0 (multiplayer), horn / siren (0xAB17D0, 0xAB54F0, 0xAB5620).
12. **Lights, sound and effects**: 0x483450, 0xA9EA10, 0xAD81B0(dt), boats 0xACF7C0, 0xA9EB00, 0xADE730, 0xAD1050, 0xAD09E0, 0xAD0CF0.
13. If in use: 0xAD1EA0, and 0xAD2F50 unless flag bit 1.
14. Aircraft (class 1) update flags 0x10000-0x80000 from 0xADD870 / 0xADD8C0.
15. The watched vehicle `0xEBCE98`: 0xACC920.
16. 0xB03FD0.

## vehicle_update_light (0xAB4460), EAX = vehicle

1. Remember the position.
2. `vehicle_sync_from_physics`.
3. Network delete, as above.
4. `vehicle_ai_drive`.
5. 0xAB41D0 → damage when needed.
6. Full physics vehicles: 0x488C50.
7. Then the wheels and engine (0xAD1B30, 0xAD1F10, 0xAD2590), and 0xAD2C50 / 0xAD1EA0 when in use.

## Player driving vs AI driving

- **Player**: input is turned into game actions before the simulation, in the player controls code from `game_frame_tick` (0x4FE760, 0x500CA0). Game actions read buttons through `input_action_value` (0x4FF770) and its siblings 0x4FF7C0 / 0x4FF810 / 0x4FF8B0 / 0x4FF930. These map a game action to a button record through the table `0xF9EA38` (16-byte entries) and the current pad `0xE98DB0`, and return 0 when the button is eaten. The vehicle part is 0xAA6540 (from 0x4F9C80). It stores the driver's throttle / steering / brake in the vehicle, and `vehicle_drive_update` applies them.
- **AI**: `vehicle_ai_drive` (0xB2DD90) computes the same controls from the road network and AI state. Lightweight traffic only takes this path.

## Havok hookup

- A full vehicle is **one Havok rigid body** from the body table (17_physics.md) plus the game's own wheel, suspension, engine and transmission model (0xAD…).
- Havok's `hkpVehicleInstance` and its parts (`hkpVehicleDefaultEngine`, `hkpVehicleDefaultAnalogDriverInput`, …) are linked in, but only Havok's own class factory (0xCF20A0) references them. The game does not use Havok's vehicle kit.
- `vehicle_sync_from_physics` (0xAB26E0) copies the body's transform and velocity back into the vehicle (0x70DED0, 0xD93F20).
- `vehicle_drive_update` pushes forces into the body (0xD61400, 0xAD0370).

## vehicles_pre_update (0xAE3A80) and vehicles_update (0xAE3C60) porting map

Both are called from `sim_frame` (sections :4 and :5). Neither takes arguments.

### `vehicles_pre_update` (0xAE3A80)

Frame `sub esp, 0x84`.

1. `humans_nav_node_refresh_all()` (0xB26340).
2. `vehicle_obstacles_rebuild()` (0xB2E1E0).
3. `vehicle_recent_list_prune()` (0xB2F370).
4. `[0x252A10C]` = 0.
5. `player_vehicle_tracking_update()` (0x9E32D0).
6. `vehicle_deferred_calls_flush()` (0x4C2D30).
7. Copy the vehicles of the lists 0x22A84E0 and 0x22A84D4 (circular, link +0x84DC) into a **stack array at [esp+0xC]**.
   - The array holds **32** pointers. **There is no bound check**, so more than 32 vehicles would overwrite the frame.
   - The copy is taken first because the update can unlink vehicles.
8. dt = `physics_frame_time()` → `fstp [esp+8]` (rounded to f32).
9. For each copied vehicle: `vehicle_pre_update(cdecl vehicle, f32 dt)` (0xAB2AA0).
10. Tail `jmp object_deferred_packets_flush` (0x737C10).

### `vehicle_pre_update` (0xAB2AA0)

cdecl (vehicle, f32 dt); frame with a security cookie.

1. Return when the vehicle is 0, +0x80 & 0xB, or +0x8F50 bit 0 is set.
2. +0x8EF8 = 0. Physics object = +0x538 for a full vehicle (+0x84D8 == 1); otherwise 0.
3. Unless the type (+0x84E4 → +0x9C) is 3 or 4: 0xAB4720(0, 0) when 0xAFE2F0 returns 1; 0xAB4720(1, 0) when the timer +0x8FFC has elapsed.
4. **Speed clamp.** Velocity = physics body +0x1A0..+0x1AC. Extra = 0xAFE2C0() for types 0 / 1, 0xA97A90() for type 4 (currently named `cmd_vehicle_ejection_thresholds`; that name may be wrong), else 0.
   - Max = info +0x5B8, replaced by +0x9540 when that is **≥ 0**.
   - Except for types 3 / 4: if the driver seat flag (+4 → +0x10C) is 1, max = info +0x5BC and the blend +0x903C = 1.0. Otherwise the blend drops by `[0xE8437C]` × 0.5 (not the dt argument), clamped to [0, 1], and max += (info +0x5BC − max) × blend.
   - If |v| > max + extra: v is scaled to that length with `rsqrtss` and **one Newton step** (`r × 0.5 × (3 − x·r·r)`), then set through the body vtbl +0x40. Keep this SSE form to match exactly.
5. **Fall check**, skipped when online host (`[0x2528B48]` == 1 and `[0x2528B49]` == 0):
   - when v.y < −22.352 m/s (50 mph): timer +0x91D4 = −1;
   - otherwise start the timer at now + 500 ms when it is unset, **and write 2.0 that frame too**;
   - when falling faster, or the timer is set and not yet elapsed: **`[0xE8ECAC]` = 2.0**. This is the divisor in `physics_frame_time`, so the physics step is halved while a vehicle falls fast.
6. 0xAA0850(vehicle, dt). If +0x8F48 bit 0x100 is set and 0xA9DBE0(frame dt) returns 1: **return** (the vehicle was removed).
7. If +0x8C38 > 0 (pending damage): 0xA9D530 / 0xA98070, then +0x8C38 = 0.
8. **Seats.** For i = 0..7 (seat i at +0x8ABC + i × 0x24): occupant = `vehicle_seat_occupant` while i is below the seat count (+0x6CE8 for a full vehicle, 8 otherwise). Exit and passenger handling through 0xAA77C0, 0xAC0290, 0xAA7E30, 0xAA7EE0, 0xAA7FB0 and 0x96CB70 (types 0x218 / 0x21A).
9. Driver tests: is the driver the local player (0x21703D4), is it a human (+0x4C == 1) with +0x3CC == 1. Then 0xA9F370; 0xA9F170 unless that; 0xB25170 by the driver kind (+0x8554).
10. 0xB047D0 for type 3, else 0xA9DE30. +0x8F50 &= ~0x80. Clear the timer +0x90D0 once it has passed.
11. 0xAD3690, 0xAC24F0, 0xAA0D10. Copy the position +0x14..+0x1C to +0x50..+0x58 and the 9 dwords +0x20.. to +0x84F8 (the previous transform); 0xAA68E0(+0x851C).
12. +0x8F50 &= ~0x20000000. In single player (`[0x2528B48]` == 1 and `[0x2528B49]` == 0): the "stopped" check on the speeds +0x851C / +0x8528 against 2.2352 m/s (5 mph) and 4.4704 m/s, with the 250 ms timer +0x8534.
13. +0x8F50 bit 0x40000000 = !(0xAAB5E0() or 0xAAB6B0()).

### `vehicles_update` (0xAE3C60)

Frame `sub esp, 0x88`.

1. Collect the same two lists **and the light list 0x22A84DC** into a stack array at [esp+0x14] (32 entries). Same **unchecked size**.
2. For each: `vehicle_update(cdecl v)`, **skipped** when (+0x9050 is set and its +0x1C ≠ 0) or +0x80 & 0xB.
3. For each vehicle on the light list 0x22A84DC: `vehicle_update_light(EAX = v)` unless +0x80 & 0xB.
4. Count light vehicles without +0x8F50 bit 0x40. If more than 4: `vehicles_cull_light()`.
5. `vehicle_collision_pairs_collect(cdecl &array, &count)` (0xAC2080). It builds unique pairs from the contact list 0x2FA8E34 (52-byte entries, count 0x252A074) into 0x223F3F0 (12 bytes each, **at most 128**, a cap inside 0xAC2080 itself: at 128 it stops early and clears the source count).
   - A pair is {vehicle, other vehicle (from the contact's +0x14 handle) or 0, flag byte}; {A, B} and {B, A} count as the same pair.
6. For each pair: `vehicle_collision_pair_react(cdecl &pair)` (0xAE3B30). With the flag set, it makes the drivers react to the crash (0x9DBCF0, 0x44F0A0).

### Port notes
- Keep the stack-copy-then-iterate shape: updates may unlink vehicles from the lists.
- `[0xE8ECAC]` is written here and read by `physics_frame_time`. Port the fall check together with the physics timing, or the step count will differ.
- The order of the ten steps above matters: the nav and obstacle data must be rebuilt before any vehicle update reads it.

## Open

- The wheel / engine model (0xAD1B30 / 0xAD1F10 / 0xAD2590) in detail.
- The exact field meanings of the driver controls written by 0xAA6540.
