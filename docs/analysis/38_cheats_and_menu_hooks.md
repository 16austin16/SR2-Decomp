# 38 — Cheats, ammo, vehicle drop and weather (for the in-game menu)

These are the game's own paths the F7 menu can call instead of patching values. Everything here is from static reading and **has not been tried in-game yet**.

`player` is the local player human (`*(0x21703D4)`).

## Cheat records

`xtbl_load_cheats` (0x6865C0) reads `cheats.xtbl` into a table of 0x68-byte records:
- the records start at `0x27A6A68`;
- the count is at `0x2527B5C` (at most 0xC0).

`cheats_parse_record` (0x686690) fills one record. It then picks an on / off callback pair by the record's `Type` (AI, Gameplay_Physics, Time, Vehicles, Weapon, Weather, Character).

| Off | Field |
|---|---|
| +0 | byte: toggleable. Only toggleable cheats get "active" set when they are turned on; one-shot cheats (cash, repair) don't |
| +1 | byte: active |
| +2 | byte: don't send the network packet |
| +3 | byte: blocked while `0x2527B5B` is set |
| +4 | byte: if clear, turning it on calls 0x687D50 (the "cheats used" text) |
| +8 | id (hash of the name), sent in the network packet |
| +0xC | localized name |
| +0x10 | name (`char *`, the xtbl Name) |
| +0x14 | description (`char[]`) |
| +0x54 | on callback `(rec+0x60, bool notify)` |
| +0x58 | off callback `(rec+0x60, bool notify)` |
| +0x5C | group index (0..4) |
| +0x60 | callback data (vehicle index, weapon name, weather name…) |
| +0x64 | callback data 2 (vehicle variant) |

The cheats the player has dialed in are a separate list: pointers at `0x140E2A8`, count at `0x140E5A8`. `cheat_find_or_add_unlocked` (0x687870) looks a cheat up by name and adds it to that list.

**Turning a cheat on or off directly** (no phone, no unlocked list needed):
- **On:** `ESI` = record, push `notify` (1), call `cheat_turn_on` (0x6862F0; callee pops 4 bytes). It:
  1. sends network packet 0x6D / 0x16 unless +2 is set;
  2. calls +0x54;
  3. sets +1 if +0 is set;
  4. if +4 is clear, calls 0x687D50.
- **Off:** `ESI` = record, push 0, push `notify` (1), call `cheat_turn_off` (0x6863C0; callee pops 8 bytes). It calls +0x58 and clears +1. It only runs when the record is not toggleable or is active.
- **`luafn_cellphone_activate_cheat`** (0x788EE0) does this through `cheat_toggle_unlocked` (0x687A00), with `EAX` = an index into the unlocked list: it turns the cheat off if active, else on.

Callbacks by cheat name (from `cheats_parse_record`'s type handlers):

| Cheat | On | Off | Effect |
|---|---|---|---|
| Evil Cars / Evil Cars 2 | 0x684CB0 / 0x684CE0 | 0x685880 / 0x6858C0 | flag bits |
| Ped War / Drunk | 0x685510 / 0x685550 | 0x685B50 / 0x685C20 | |
| Instant Cash (+$1000) | 0x684D10 | – | cash adjust |
| Remove / Add Police, Gang Notoriety | 0x684D40, 0x684D70, 0x684DE0, 0x684E30 | – | |
| Infinite Sprint, Max Health, Player Pratfalls, Super Beer Muscles | 0x684EC0, 0x684ED0, 0x684EF0, 0x684F00 | 0x685900, –, 0x685910, 0x685920 | |
| Giant / Tiny Player, Bushwick Bill | 0x684F50, 0x685040, 0x685130 | 0x685940, 0x685940, 0x6859D0 | |
| Raining Peds | 0x685500 (`0x27A6A40 |= 0x100`) | 0x685B40 | |
| Never Die, Super Saints | 0x6855E0, 0x6855F0 | 0x685CA0, 0x685CB0 | |
| **Unlimited Clip** | 0x685690: `0x27A6A40 |= 0x1000` | 0x685D40 | the magazine never empties |
| Super Explosions, Low Gravity, Elevator to Heaven | 0x6857F0, 0x685800, 0x685870 | 0x685E80, 0x685E90, 0x685EF0 | |
| Time: Stop_Time / Set_Hour | 0x685220 / 0x685260 | – / 0x685AC0 | |
| Character: Set / Restore_Player_Character | 0x6856A0 | 0x685D50 | |
| Vehicles: **Drop** | 0x685270 | – | spawn request (below) |
| Vehicles: Infinite_Mass | 0x6852A0 | 0x685AD0 | |
| Vehicles: Repair_Vehicle | 0x6852E0 | – | repairs the player's vehicle (0xAAAB60) |
| Weapon: **Give** / **Ammo** | 0x685370 | – | give a weapon by name, or top up its ammo |
| Weapon: **Infinite_Ammo** | 0x685470: `0x27A6A40 |= 0x40` | 0x685B10 | reserve ammo never runs out |
| Weather: **Set_To** | 0x685480 / 0x6854C0 | 0x685B20 | forced weather (below) |

## Ammo

### Where ammo lives

- **Magazine:** weapon instance +0x10. `weapon_clip_size` (0xB73CC0; `EAX` = human, `ESI` = instance) is info+0xDC, doubled when dual-wielding.
- **Reserve (non-player humans, and thrown weapons):** instance +0x14 (primary) / +0x1C (alt). The maximum is in the ammo record at info+0xD4 (primary) / +0xD8 (alt): +0x10 in the mode where `0x2528B48 == 1 && 0x2528B49 == 0`, else +0xC.
- **Reserve (player):** shared by ammo type in the player's ammo store `*(human+0x11DC)`:
  - `store+0` → array of 12-byte entries `{ammo type *, count, recently added}`;
  - the number of entries is `0x22D7B50`;
  - an entry is matched on `entry.type+4` == the weapon's ammo record.

  The store is used when human+0x4C == 1 and +0x3CC == 1 (`human_is_local_player`, 0x926600) and the weapon category (info+0x34) isn't 0x16.

### The functions

- **`weapon_add_reserve_ammo`** (0x9C3D40; `EAX` = amount, `CL` = alt; cdecl `(human, instance, 0)`): adds up to the maximum and returns how much it added. For the player it goes to `ammo_store_add` (0xB69150 → 0xB690C0).
- **`weapon_reserve_ammo`** (0x9C3EA0; `ESI` = human, `EDI` = instance; cdecl `(alt)`): the reserve count, or 0x7FFFFFFF when unlimited.
- **`weapon_has_unlimited_ammo`** (0xB72CE0; `EAX` = instance; cdecl `(alt)`). True if any of these hold:
  - `0x252973A` is set;
  - the cheat flag `0x27A6A40 & 0x40` (Infinite_Ammo) is set;
  - the mode flags `0x2528B48 == 1 && !0x2528B49 && 0x2A4D2C5`;
  - instance+0xC bit 0x10 (primary) or 0x20 (alt).

  `weapon_fire` and `human_ai_think` check this too.
- **`inventory_find_weapon`** (0x55ADF0; `EAX` = &inventory (human+0x10C4), `ESI` = weapon info id): the item whose instance's info+0x3C matches.
- **`weapon_refill_after_add`** (0x558DB0; cdecl `(human, item)`, `EAX` = amount): adds the reserve, clamps the magazine to the clip size, and handles thrown weapons (magazine 1, count in the reserve).
- **`cheat_weapon_give`** (0x685370; cdecl `(&name ptr)`). The name is looked up with 0xB716E0.
  - If the player has the weapon, it tops up the reserve (amount 0x9C3F50).
  - Otherwise it adds the weapon with full ammo (0x559890).
  - If the info has flag +0x10 bit 0x20, it also tops up the alt reserve.

### For the menu

1. **Infinite reserve ammo and grenades:** set bit 0x40 in `0x27A6A40` (the game's own Infinite_Ammo). Clearing the bit turns it off.
2. **Infinite magazine:** bit 0x1000 in `0x27A6A40` (Unlimited Clip) could replace pinning instance+0x10.
3. **Fill reserve once:** for each inventory item call `weapon_add_reserve_ammo` with a large amount (it clamps to the maximum). Do it for alt too when the weapon has one.

Setting the bits directly skips 0x687D50 (the "cheats used" text). Turning the real cheat on with `cheat_turn_on` doesn't.

## Vehicle drop (spawn near the player)

The `Drop` cheat's record holds:
- the vehicle index: `vehicle_info_find(name)` (0xAE4090, `ECX` = name; the info table is `0x2FAD1F8`, 0x7D0 bytes each);
- the variant: 0xAC7420, `EAX` = info, `ECX` = variant name; −1 for none.

`cheat_vehicle_drop` (0x685270) only posts a request, and it does nothing while the state is 2:
- `0xEAA8B0` = vehicle index;
- `0xEAA8B4` = variant;
- `0xEAA8AC` = 1.

`cheat_vehicle_drop_update` (0x6875B0) runs every frame from `cheats_do_frame` (0x687B40, called by `sim_frame`).
- **State 1:** `cheat_vehicle_drop_spawn` (0x687630):
  - calls `vehicle_create` (0xAE2A00) with `EAX` = 2, `ECX` = index, and cdecl args (7, zero vector `0x252A6D0`, identity `0xEC5AE4`, 0, 0.0, variant, 0…). Mode 7 places it by the player;
  - stores the handle in `0xEAA8B8`;
  - sets state 2.
- **State 2:** once the vehicle has streamed in (+0x84D8), it shows the garage text (0x687750) and resets.

**For the menu:** call 0xAE4090 for the vehicle name, then write the three globals (only when `0xEAA8AC != 2`). `sim_frame` does the rest.

## Weather

- Weather records: base `*(0x2526DD0)`, count `0x2526DCC`, 0xD8 bytes each, name `char *` at +0. `weather_find` (0x4DEDD0) returns the record for a name.
- **`weather_set`** (0x4DEC00; `EAX` = index, `CL` = immediate): sets the forced record `0x2526E14` = base + index × 0xD8.
  - With `CL` = 1, it applies the record at once (0x4DE390).
  - With `CL` = 0, it blends through the time-of-day slots.
- The Weather cheat's off callback (0x685B20) clears `0x2526E14` and bit 8 of `0x27A6A40`, which returns control to the normal weather cycle.

**For the menu:** list the names at `*(0x2526DD0) + i × 0xD8`. Set with `EAX` = i, `CL` = 1, call 0x4DEC00. "Normal weather" writes 0 to `0x2526E14`.

## Open

- What mode the `0x2528B48` / `0x2528B49` / `0x2A4D2C5` flags are (they also select the higher ammo maximum).
- `0x252973A` (another unlimited-ammo switch) and the full meaning of each bit of `0x27A6A40`.
