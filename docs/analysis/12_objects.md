# 12: The object system

Names are in `docs/functions.csv` (subsystem `object`). The classes `object`, `human`, `vehicle`, `object_mover`, `object_item`, `object_prop` and `object_debris` are in `docs/rtti_classes.csv`.

## Handles

**Table.** 4096 slots of 16 bytes at `0x2149C58` (up to `0x2159C58`):

| Off | Field |
|---|---|
| +0x0 | free-list next |
| +0x4 | free-list prev |
| +0x8 | slot index |
| +0xC | object pointer |

The free-list sentinel is `0x2149C44` (head) / `0x2149C48` (tail). The sequence counter is the u16 `0x2149C54`; it starts at 1 and skips 0 when it wraps.

**Handle layout**: `(sequence << 16) | (peer_id & 0xF) << 12 | slot`.
- `peer_id` is the local multiplayer player id (`[[0x2528C14]+0x408]+0x1B0`), 0 in single player.
- Lookup is always inlined: `obj = [0x2149C64 + (h & 0xFFF) * 16]`. It is valid only when `obj != 0 && obj+0x44 == h`. That check makes stale handles fail.

| Function | What it does |
|---|---|
| `handle_table_init` (0x925A20) | puts all 4096 slots on the free list, sequence = 1, then 0x95EA70 and 0x919790 |
| `handle_alloc` (0x924DB0) | takes the head of the free list, stores the object, builds the handle; returns 0 when the table is full |
| handle free | inside `object::deactivate` (below): the slot goes back to the free-list **head**, or to the **tail** when the object's flag 0x100 is set, so the slot is not reused soon |

## The object base

Every game object starts with this layout (`object`, vtable `0xDDBC64`, 9 slots):

| Off | Field |
|---|---|
| +0x00 | vtable |
| +0x04 / +0x08 | links in the **all objects** list (`0x25295E8`) |
| +0x0C / +0x10 | links in the **active** list (`0x25295EC`) |
| +0x14 | position (vec3) |
| +0x20 | orientation (3×3 matrix, up to +0x43) |
| +0x44 | handle |
| +0x48 | grid index (`update_grid_index`, slot 8) |
| +0x4C | type (below) |
| +0x5C … +0x78 | defaults set at registration: +0x60 = 1.0, +0x68 = +0x6C = 4, +0x74 = 10.0 |
| +0x7C | **physics body index**, -1 = none (below) |
| +0x80 | flags: 1 active, 2 (set on activate), 8 deactivated, 0x20 registered, 0x80 networked, 0x100 free the handle to the tail |
| +0x84 | u8 |
| +0x98 / +0x9C | -1 at registration |
| +0xAC | multiplayer snapshot data |
| +0xB0 | multiplayer record (08_multi_objects.md) |

**Virtual slots** (`object`):

| Slot | Method |
|---|---|
| 0 | 0x925750 |
| 1 | 0x4BE1F0 |
| 2, 3 | pure |
| 4 | `activate` |
| 5 | `deactivate` |
| 6 | `set_physics_keyframed` (0x925960) |
| 7 | 0x9259D0 |
| 8 | `update_grid_index` |

`human` adds slots 9-11 (12 in all); `object_mover` adds slots 9-16 (17 in all; slot 15 is take damage, see the diversion event hooks).

## Types (+0x4C)

The types come from the `push type` before each `object_register` call.

| Type | Created by | Class |
|---|---|---|
| 1 | 0x983830 | **human** (`object_is_player_human` tests type 1 and +0x3CC == 1) |
| 2 | 0x931140 (loads `.smesh_pc`) | item / static mesh object |
| 3 | 0x9FBAF0, 0x9FFDD0 | **object_mover** (general movers) |
| 4 | 0x94C1A0, 0x94CD90 | ? (list `0x215E07C`) |
| 5 | 0xAE2A00 | **vehicle** |
| 6 | 0x5043B0 | ? |
| 7 | `chunk_section_00_objects`, 0x4BB250 (train stations), 0x53CC50, 0x615800, 0x6A3180, 0xA68650 | world / trigger objects placed by chunks and missions |
| 8 | 0x92AF70 | ? (physics-driven) |
| 0xA | 0x4BD8B0 | brass / shell casings (`brass`) |
| 0xB | 0x9554A0 / 0x955560 | ? (searchlights) |
| 0xC | 0x934820 | **object_prop** |
| 0xD | 0x55B600, 0x91D7B0 | **inventory_item** |
| 0xE | 0x9196B0 | **detour_hull_object** |

Type counts in compares across the binary: type 1 is tested 1,261 times and type 5 1,118 times, the next highest 90. This confirms human and vehicle as the main types.

## Lifetime

1. **Construct**: the class constructor (`human::ctor` 0x9CC980, `vehicle::ctor` 0xAE1300, …) writes the vtable and its own fields.
2. **`object_register`** (0x925500, EDI = object, stack = type):
   1. Link into the all-objects list `0x25295E8`.
   2. `handle_alloc` → +0x44.
   3. Type → +0x4C, flags = 0x20, the defaults, body index -1.
   4. 0x925310.
   5. Clear the multiplayer pointers.

   Network-visible objects then call `multi_register_object` (0x805000).
3. **`object::activate`** (0x925780, slot 4): unless it is a remote multiplayer object, set flags |= 3 and link into the active list `0x25295EC`.
4. **`object::deactivate`** (0x9257F0, slot 5):
   1. For networked objects (0x80), send the multiplayer destroy (`0x8053B0`) or 0x806DB0.
   2. Set flag 8.
   3. 0x70A520 (physics / world removal).
   4. Unlink from both lists and free the handle.

   Subclasses override it (`detour_hull_object`, `inventory_item`, `object_prop`).
5. `objects_deactivate_all` (0x924FE0) calls `deactivate` on everything in the active list.

`objects_reset` (0x925AF0) sets `0x25295F4` and resets every object subsystem in order: multiplayer 0x806E30, 0x92CF50, humans 0x9D8210, 0x9CFB90, vehicles 0xAE31C0, 0xACFEB0, 0x94BE80, 0x92F5D0, 0x9F9DE0, 0x505300, 0x929C20, brass 0x4BD750, searchlights 0x955430.

## Per-type update loops (from sim_frame, see 11_frame.md)

| Loop | List | Notes |
|---|---|---|
| humans: 0x9A7080 / 0x9A7150 / 0x9A71B0 (sim_frame:1c), 0x9A7210 (section 5), human processing 0x9A9B60 (per physics substep and after) | `0x215E908` | human_processing.cpp; 0x9A9B60 collides characters with `havok_custom_character_collector` |
| vehicles: 0xAE3C60 | `0x22A84D4` / `0x22A84DC` / `0x22A84E0` | destroyed / submerged handling (0xAB3550), 0xAB4460, 0xAE1CF0 |
| 0xA8B400 | `0x2225B64` | |
| 0x95A3D0 | `0x215E5C0` | spawning / population |
| type 4 objects: 0x94C0A0 | `0x215E07C` / `0x215E078` | |
| movers 0x9FCF90 (`general_mover_init`), 0x9FA620 / 0x9FA670 / 0x9FF5A0 | | |

The humans list is also walked through each human's +0x1294 link (`0x217043C`, see 0x9D8210). The local player is `0x21703D4`.

## Physics bodies

`+0x7C` is an index into the body table `0x28F25C0`: 0x18-byte entries, +0 = the Havok entity.
- `object::set_physics_keyframed` (slot 6) compares the entity's +0x2C motion field with 0x8002E (keyframed) and switches it through 0x6FD440 / 0x6FD1E0 / 0x6FD700.
- The Havok world is `[0x2527D94]+8`, stepped by `sim_physics_step` (11_frame.md).
- Characters (humans) are not rigid bodies in this table. They go through the character collector in human processing.

## Open

- Types 2, 4, 6, 8 and 0xB need a class each (the creators are listed).
- The body table's own allocate / free functions (0x6FD… family).
- The per-type `update` virtual slots of `human` and `vehicle`.
