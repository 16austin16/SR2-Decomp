# 08: Multiplayer object replication (multi_core_object_registration)

Names are in `docs/vtable_slots.csv` (class `multi_core_object_registration`) and applied to every subclass by `tools/harvest_names.py`.

## Registrations

Every replicated object type has one static registration object, a subclass of `multi_core_object_registration` (base vtable `0xE27E7C`, 22 slots):

| Type | Class | Singleton |
|---|---|---|
| 9 | `detour_multi_registration` | `0x2A46038` |
| | `player_multi_registration` | `0x2A4AA10` |
| | item, mover, npc, pipebomb, prop, rocket, trigger, vehicle | one each |

- The constructors (0x8622E0 and friends) run from the static initialiser list (0xDA280C).
- The setup functions (0x869AF0 for the player, then 0x85E6C0, 0x870600, … from 0x81FC70) give each registration a name (+0x14, for example "player") and its replicated fields.
- Fields are added with 0x807780. Each field is (name, bit, size), for example RAGDOLL, POS, ORIENT_H, LINEAR_VEL, MAX_HP, HIT_POINTS and DATA. They are 0x44-byte entries at +0x20, with the count at +0x18. Each field gets one bit of a u16 dirty mask.
- `0x212E25C[type]` holds the registration for each type (10 types). `0x212E284[type]` is the circular list of live multi records of that type, linked through +0x674 / +0x678.

## Multi records

A game object's +0xB0 points to its multi record:

| Off | Field |
|---|---|
| +0 | owner peer id |
| +2 | network id (u16) |
| +4 | object handle |
| +8 | type |
| +0x67C | flags (1 = migration pending, 2 = taken after the owner left) |
| +0x67E | u16 mask of peers that have not been told yet |

- Register: `0x805000` (local objects) and `0x804320` (from a create packet).
- Unregister: `0x804140`.
- Deactivate: `0x8053B0`, from `object::deactivate`. It sends destroy packet 0x0F, then unregisters.

## The slots

| Slot | Method | Called from |
|---|---|---|
| 0 | scalar_deleting_dtor | |
| 1 | on_register | 0x805000, 0x804320 |
| 2 | write_create_data | create packets 0x1C (0x803320), 0x0E to late joiners (0x807840) |
| 3 | create_from_data | 0x804320 |
| 4 | on_unregister | 0x804140 |
| 5 | write_destroy_data | packet 0x0F (0x8053B0) |
| 6 | destroy_from_data | 0x804550 |
| 7 | ? | (player: acts on the local player 0x21703D4) |
| 8 | remote_do_frame | 0x804850, every sim frame, objects owned by others |
| 9 | capture_snapshot | 0x800110, 0x805CD0 |
| 10 | free_snapshot | 0x7FFB70, 0x8005B0 |
| 11 | write_state_delta | 0x805FC0 (per-peer state update) |
| 12 | read_state_delta | 0x805D60 |
| 13 | get_dirty_fields | 0x805E90: field mask against the peer's baseline, used to sort the send queue |
| 14 | get_send_priority | 0x805E40: distance buckets 400/100/25/9 → -3..2 |
| 15 | on_state_sent | 0x805EC0, 0x805FC0 |
| 16 | ? | (player: picks one of three per game mode) |
| 17 | can_migrate | 0x8045F0, 0x806D50 |
| 18 | on_ownership_taken | after packet 0x2A (0x804710), 0x806F70 |
| 19 | write_migration_data | give-ownership packet 0x27 (0x806910) |
| 20 | on_ownership_given | 0x8067D0 (base no-op) |
| 21 | on_owner_left | 0x806F70 / 0x807070 / 0x8072E0: 0 = migrate to another peer, 1 = take it locally |

**State sync** (`0x805FC0`, per peer, from 0x814BB0):
1. For every owned record, capture a snapshot.
2. Compute the dirty fields against what that peer has acknowledged, then sort with qsort.
3. Pack deltas with slot 11 until the 0x514-byte packet is full.
4. Record what was sent with slot 15.

**Ownership migration:**
- `0x8045F0` runs every sim frame. For each pending request it asks `can_migrate`, then `0x806910` gives ownership away (packet 0x27 with the slot-19 payload).
- When the owner leaves, `on_owner_left` decides who takes the object over.
