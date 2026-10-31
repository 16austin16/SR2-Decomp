# 17: The Havok boundary

The game uses **Havok 5.5.0-r1**. This file covers only the game side of the boundary. Names are in `docs/functions.csv` (subsystem `physics`); the game's physics code is `.\havok\havok.cpp` around 0x6F0000-0x720000.

## The world

- `0x2527D94` is the `game_havok_world` object (RTTI); the `hkpWorld` is at +8.
- `havok_world_create` (0x6F63C0, from the game init 0x51F700) builds it and adds the **`GeneralCollisionListener`** (vtable `0xE18838`) as its collision listener.
- `0x2527DC4` is the world lock state: 0 free, 1 adding is deferred, 2 stepping (removes are deferred).

## sim_physics_step (0x6FA0E0) in detail

Profiler label "sim_frame:4-HavokWater", called from `sim_frame` (11_frame.md).

1. `max_substeps = 0x6F5D60()` = round((multiplayer ? 30 : 3) × `0xE8ECAC` + 0.5). `0xE8ECAC` is the slow-motion factor (1.0 normally).
2. `InterlockedIncrement(0x2527DAC)` (a physics step is in progress), then 0x728450, then 0x717B40.
3. Accumulator: `0x2527DC8 += frame dt` (`0xE8437C`). The step is `0x2527DA4 / 0xE8ECAC`; `0x2527DCC` is the time stepped this frame.
4. While the accumulator ≥ step and fewer than `max_substeps` substeps have run:
   1. Clear `world+0xEC`.
   2. 0xAE48A0(step): per-substep game work (vehicles).
   3. 0xD4B4B0(hkpWorld, step): set up the Havok step.
   4. Create a havok.cpp job (line 0x1334) running **`havok_step_job`** (0x6F5BB0) and wait for it. The job:
      1. 0x6FF670 (pre-step).
      2. `0xD4A160(world)` gives the multithreading state.
      3. `0xD4AC40(world, 2 / 1 / 0)` runs the step phases (integrate / collide / continuous), plus 0xD5F890.

      The two job workers (`0x33D63F8` / `0x33D64C8`, 11_frame.md) help with the Havok jobs.
   5. **`humans_character_step(step, players)`** (15_humans.md).
   6. accumulator -= step.
5. If the cap was hit, 0x4DA560 clamps the leftover accumulator.
6. Reset `0xE8ECAC` to 1.0 and stamp the frame counter.
7. Post-step game updates:
   1. 0x6F9FD0.
   2. 0x717E00(0x6FF860()): write body transforms back to objects.
   3. 0x6F57A0, 0x70B200 (deferred work).
   4. `InterlockedDecrement(0x2527DAC)`.
   5. 0x6F5AE0.
8. Outside co-op, if the Havok memory statistics (`[0x22F2ADC]` vtable +0x44) report more than 6.5 MB in use, run the registered memory-pressure callbacks `0x28F256C[0x2527DD0]` until usage drops.

## Bodies: the 0x28F25C0 table

5000 entries of 0x18 bytes:

| Off | Field |
|---|---|
| +0x00 | `hkpRigidBody *` |
| +0x04 | in use |
| +0x08 | the shape (reference-counted; released on destroy) |
| +0x0C … +0x14 | 0 at creation |

Free indices are a stack at `0x1699950`, with the used count in `0x168B734`. A game object stores its index in **+0x7C** (12_objects.md).

**`havok_body_create`** (0x6FBFA0, (body desc *)):
1. Replace a NaN position with the origin; check the orientation.
2. Pop a free index.
3. Build the shape by desc +0x80:

   | Kind | Shape |
   |---|---|
   | 1 | sphere (radius desc +0x58) |
   | 2 | capsule (two points +0x40 / +0x4C, radius +0x58; 0xD3FFB0) |
   | 3 | box (half extents from +0x34; box inertia computed once into desc +0x88) |
   | 4 | mesh / convex (0x705420, optionally wrapped by 0x70EA30) |

4. Set the mass properties (mass +0x6C, inertia).
5. Build the rigid body cinfo: position +4, orientation +0x10, friction / restitution / damping +0x5C / +0x60 / +0x64 / +0x68, motion type +0x7C, quality +0x84.
6. 0x6FD8F0, then `0x6FDA80` creates the `hkpRigidBody`.
7. 0x6F5A80(layer +0x7C, +0x84, the object's handle +0xBC) sets the collision filter info and attaches the owner's handle as entity property 1.
8. Add it to the world: 0xD5CB00, or deferred (0x6F5CD0) while the world is locked.

Callers: brass 0x4BD8B0, 0x504C90, 0x50E880, 0x92ABE0, items 0x931140, props 0x934AD0, type-4 objects 0x94C1A0, movers 0x9FA3A0, 0xA8F590.

**`havok_body_destroy`** (0x6FBE90, index):
1. Clear in-use, and deactivate the body if needed.
2. While the world is stepping (lock 2), queue the removal: `0x168B738[]` / `0x168DE80[]`, count `0x2527DC0`. Past 2000 entries, flush with 0x6F5C10. Otherwise remove it now (0xD5C420).
3. Release the shape.
4. Push the index back on the free stack.

**Helpers:**
- `havok_body_get_velocities` (0x6FD700): motion +0xD0, vtable +0x40 / +0x44.
- `havok_body_set_velocity_from_move` (0x6FD2C0): velocity = delta / dt.
- `object::set_physics_keyframed` (slot 6, 0x925960) switches motion types (0x6FD440 / 0x6FD1E0 / 0x6FD700).

Humans are character proxies, not bodies (15_humans.md). A vehicle is one body (16_vehicles.md).

## Contacts into game code

**`physics_contact_point_added`** (0x70A730, `GeneralCollisionListener` slot 0), under a critical section:
1. Find both collidables' root bodies. Read each one's owner handle from entity property 1 (0x7031C0), and resolve the objects and their **types** (+0x4C).
2. **Type-pair table** `0x1743D80`: 5 dwords per unordered pair, indexed `(hi + 1) × hi / 2 + lo`. When the pair has an entry, call its callback(objA, objB). Returning true **disables the contact** (+0x20 = 1).
3. Vehicles (type 5) get special handling (0x703230).
4. Each side gets 0x70A230 (impulse / damage from the separating velocity), then 0x70A650.
5. When the bodies approach each other (relative normal velocity > 0), 0x709CA0 builds a collision event. Each object's **per-type handler** `0x1765140[type]` (function, context) then runs; returning false rejects the contact.

| Slot | Method |
|---|---|
| 0 | contact point added (0x70A730) |
| 1, 2 | stubs (0x70ABB0 / 0x70ABC0) |
| 3 | the Havok base (0x7026A0) |

**`havok_game_filter`** (vtable slots 3-7, 0x70F930 … 0x70FA40) is the collision filter (the `isCollisionEnabled` overloads for collidables, shape collections and rays). Its layers come from the bodies' filter info set by 0x6F5A80. Two other filters, `NeverCollideFilter` and `Clear1nTracksFilter`, are used in special cases.

**Queries**:
- ray casts: `havok_closest_ray_hit_collector`, `havok_all_ray_hit_collector`;
- linear casts: `havok_closest_linear_cast_collector`, `havok_all_linear_cast_collector`;
- character contacts: `havok_custom_character_collector`;
- body pairs: `object_collector`, `contact_collector`.

**Actions** (`hkpUnaryAction`): `multi_game_havok_corrector` (multiplayer position correction) and `MyAction` (0x6F53F0).

## Open

- What each step phase in `havok_step_job` maps to in Havok.
- The contents of the pair table `0x1743D80` and the handler table `0x1765140`, and who registers them.
