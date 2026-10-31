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

## Open

- The wheel / engine model (0xAD1B30 / 0xAD1F10 / 0xAD2590) in detail.
- The exact field meanings of the driver controls written by 0xAA6540.
