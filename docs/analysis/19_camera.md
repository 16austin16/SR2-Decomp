# 19: The camera

Names are in `docs/functions.csv` (subsystem `camera`). The camera code is around 0x490000-0x4A4000.

## State (0x25F5AE8 … 0x25F5BF0)

| Global | Field |
|---|---|
| `0x25F5AE8` | **camera mode** (index into the mode table) |
| `0x25F5B10` | camera sub-mode / view (10 = special) |
| `0x25F5B14` | **current position** (vec3) |
| `0x25F5B20` | the mode's desired position (vec3) |
| `0x25F5B2C` | previous position |
| `0x25F5B38` | **current orientation** (3×3, 9 floats) |
| `0x25F5B5C` | the mode's desired orientation |
| `0x25F5B80` | previous orientation |
| `0x25F5BA4` / `0x25F5BA8` / `0x25F5BAC` | FOV: current / target / previous |
| `0x25F5BB0` | **target object handle** (the player, or what the camera follows) |
| `0x25F5BB4` / `0x25F5BC0` | the target's position / orientation this frame |
| `0x25F5BE4` | the dt used this update |
| `0x25F5BE8` | flags: 2 = smooth towards the desired transform (0x4A33A0); 4 = cut (teleport: `camera_cut` 0x492DF0, and a move of more than 15 m sets `0x2529620`); 8 = blend the FOV (5 per second) |
| `0x25F5C98` / `0x25F5C9C` | mouse look sensitivity x / y (used by `input_map_keyboard_mouse`) |
| `0x25F5CB0` / `0x25F5CB8` | vehicle camera parameters set by `vehicle_update` for the player's vehicle |

## Per-frame update: camera_update (0x4932F0)

`sim_frame` calls it twice: in sections 1,2,3 and again in section 5, after the objects have moved (11_frame.md). It does nothing without a valid target.

1. 0x4A3540. In game mode 0xD (a special state) it also runs 0x8AAA80.
2. Save the current position, orientation and FOV as "previous".
3. **dt** is the frame dt. When the target is a vehicle, or a human in a vehicle, and smoothing is off, it uses the physics step dt (0x6FF860) instead.
4. **Mode update**: `camera_modes[0x25F5AE8].update(dt)`. The table is `0xDD5250`, 0x14 bytes per mode: {enter, exit, update, …}. The mode writes the desired position `0x25F5B20` and orientation `0x25F5B5C`.
5. With flag 2, smooth towards the desired transform (0x4A33A0).
6. Camera shake / effects (0x4A0EA0).
7. With flag 4 (cut), run `camera_cut` and clear the flag.
8. Copy desired → current (`0x25F5B14`, `0x25F5B38`), and blend the FOV with flag 8.
9. If `0xE8825F` (fade humans that block the view) is set, cast from the camera (0x6FA820) and fade the humans hit (0x492D90), except the player and their vehicle's occupants.
10. Record the target's transform, the dt, whether the camera moved more than `0xE88BE8` (`0xE996E5`), and the region (0x4A9BE0 → `0x25F5BEC`).

**Mode table** (update functions; modes without an update are driven by other code):

| Mode | enter / exit / update | Notes |
|---|---|---|
| 0 | 0x49B2A0 / 0x49B420 / **0x49B4B0** | the **gameplay follow camera** (on foot and in vehicles). Uses Havok ray and linear casts for camera collision ("camera_collide", "vehicle_camera_collide"), and the vehicle camera xtbls (`vehicle_cameras.xtbl`, `vehicle_group_cameras.xtbl`) |
| 1 | 0x4A1560 / - / 0x4A1590 (ret) | **free / debug camera**: driven by the free camera 0xC01650 through the free-camera controls (14_input.md). `frame_end_handoff` uses LOD distance 10 and puts the listener at the camera in this mode |
| 2 | 0x4A0140 / - / 0x4A0150 (ret) | externally driven (script / cutscene look-through) |
| 4 | update 0x49F1C0 | |
| 5 | update 0x494D60 | follows animation bones ("Use Camera Lookat Bone / Position Bone") |
| 7 | 0x495590 / - / 0x4956C0 | |
| 8 | update 0x495950 | |
| 9 | 0x4A2730 / - / 0x4A2400 | casts rays (0x6FA330) |
| 10 | update 0x4A1020 | |

## Mouse look and the look stick

- `input_map_keyboard_mouse` turns the mouse delta into the **look stick** (`0x2348534` / `0x2348538`, scaled by `0x25F5C98` / `0x25F5C9C`). The pad's right stick fills the same record (14_input.md).
- `player_controls_update` (0x4FE760, early in `game_frame_tick`) copies the sticks and buttons into the player control block at `0x2659908…`. The follow camera reads its look input from there.

## The snapshot in frame_end_handoff

When the world is visible, `frame_end_handoff` copies:
- the camera position `0x25F5B14` → `0xE9D620`;
- the orientation `0x25F5B38` → `0xE9D62C` (9 floats);
- the FOV from 0x494080 → `0xE9D650`;
- 0xBE4170 on the orientation.

Then it sets the listener / LOD centre `0x277C9D0`:
- the camera in mode 1;
- the origin with no player;
- otherwise the player's position, with LOD distance `0xE9896C` = 5 or 30 depending on the player's vehicle state.

The render thread copies `0x277C9D0` into the render context (18_scene.md).

## Open

- The identity of modes 4, 7, 8, 9 and 10 (aim / shoulder, cutscene, store / customisation cameras are the likely candidates).
- The follow camera's internals (0x49B4B0) and how it uses `vehicle_cameras.xtbl`.
