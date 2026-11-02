# 55: Save / load, progression, inventory and stores (names round 55)

This round maps the save-game serialiser (the 72 section functions behind the table `0xDD3050`), the profile and save-device code, the player customisation code ("pcu_" / "pcr_"), the unlockables / rewards, stats and tutorial tables, and the store code (weapon, music, liquor, discounts, cribs). It adds **202 new names** in `docs/candidates_saveload.csv` (126 confirmed, 76 inferred; none are in `functions.csv`; notes end "(55)") and **10 renames of `vfN` placeholders** in `docs/renames_saveload.csv`. It also gives ten large port maps and fifteen small port recipes.

Of the new names, 72 are the save section functions (every row of the section table, all four columns) and 130 are other functions.

## Method and caveats

- **Function starts.** Every candidate address was checked against `functions.csv` (no address or name is reused, and none of the candidates is in `functions.csv`) and against a decoded sweep of `.text`: a start is a `call rel32` target, a table entry in the save section table, or the first byte after padding / a `ret`. `tools/xrefs.py` reports byte matches inside instructions, so caller counts in the notes come from a decoded sweep of `E8` / `E9` only. The one start whose previous byte is not padding is `0x7C21B0` (`pcu_worn_entry_copy_by_slot`): it directly follows the 44-byte index table (0x7C2184..0x7C21AF) of the jump table used by `pcu_worn_remove_base_slots` (`0x7C2120`), and its 50 `call` sites confirm the start.
- **True sizes.** Bodies are walked by recursive descent (both arms of conditional jumps, jump tables decoded from their bytes, a `jmp` to another function start or farther than 0x6000 bytes is a tail jump). The rule "size = up to the first `int3`" fails here too: the section functions that end in a tail `jmp` (sections 1, 5, 18, 22, 23, 24, 25) are 0xE..0x15 bytes, and a `0xCC` byte can sit inside an immediate.
- **Register conventions** in the notes come from a linear-prefix scan (registers read before written on the first fall-through path, as `tools/regin.py`) and the `ret N` of the walked body. A register that is read only after the first branch is missed. "caller-clean" means every `ret` is a plain `ret`.
- **Confirmed** means the role follows from the instructions (or from a string / a named callee that settles it); **inferred** means the role is a reading of the callees, strings and offsets, with no dispatch site or second source.
- **Port maps are machine generated** from the disassembly (block, callee, argument and polarity pass); roles of sections are inferred from callee names, the calls, addresses, tables and compare polarities are exact. `X87 ... <== f32 store` marks an `fstp/fst dword` (the value is rounded to f32 there). `IF cmp a, b ; jl taken if a < b` states the polarity of every conditional jump; for x87 compares the list gives every outcome (`gt` = ST0 > operand, `lt`, `eq`, `un` = unordered) for which the jump is taken, derived from the flag bits (`test ah, mask`, C0 -> 0x01, C2 -> 0x04, C3 -> 0x40). `regs{...}` after a `CALL` shows the last instruction that wrote each input register of the callee in the same block. Re-check a block against `python tools/x86dis.py` before porting it.

## Surprises

1. **72 section functions, none named.** Doc 13 lists 28 sections; each has a save and a load function, and eight sections (1, 4, 5, 9, 11, 18, 23, 27) also have co-op client variants: 56 + 16 = 72 distinct functions. All are named here (`gamesave_sNN_<what>_save|load[_coop]`). Section 14 (playlists) has a **one-byte load function** (`0x694AD0` is a bare `ret`): the playlist block is written but never read back.
2. **Section 15 is the cutscene-seen bit set, not mission flags.** Its save and load call `cutscene_seen_flags_store_bits` / `cutscene_seen_flags_load_bits` and only run when the mission count `0x145A4AC` is below 0x100 (32 bytes at `+0x39B68`). Doc 13 called it "mission flags"; the callee names settle it.
3. **The player section embeds the whole customisation (wardrobe) state.** Section 3 (`0x692300`) calls `gamesave_player_customisation_save` (`0x6921D0`), a 0xE454-byte block at `+0x314`; when the multiplayer snapshot `0x6A4C90` exists it is a single `memcpy`. This is why the pcu / pcr code (`0x7C....`) is part of the save format.
4. **Money is clamped to +-2,000,000,000 in at least four places** with the same constants `0x77359400` and `0x88CA6C00` (section 3 at `+0xB4`, `savegame_cash_add_clamped` `0x696180`, `cash_widget::set_value_clamped`, `cash_from_dollars`). `0x696180` also multiplies amounts that lie within +-20,000,000 by 100 (the 1/100 money unit) before the saturating add, and replaces larger amounts by +-2e9.
5. **`profile.dat` is three fixed 1000-byte blocks.** The profile struct is 0xBD8 bytes: blocks at `+0`, `+0x3E8`, `+0x7D0` (1000 bytes each), their sizes at `+0xBB8 / +0xBBC / +0xBC0`, a pointer to block 3 (the options) at `+0xBC4`. The file is `(u32 size, size bytes)` three times.
6. **The stats table has 398 named stats and 69 award condition rows.** `stats_xtbl_load` (`0x6B2C30`) parses `stats.xtbl` into 0x44-byte records at `0x1452BE0` (count `0x2527CE8`) against the name table `0xE94F90` (0x18E entries); `stat_award_table_init` (`0x6B3BB0`) then fills 69 rows of six callbacks (`0x27DA820`, stride 0x18) with straight-line stores.
7. **Tutorials are 268 records, only the first 134 are saved.** The table at `0x27DAE98` (0x20 bytes per record) is filled by `tutorials_table_init` (`0x6B8470`): records 0..133 get only their `+8` / `+0xC` dwords there, records 134..267 get full stack-built text records; section 12 saves the dword at `+0xC` (state: 0 locked, 1 unlocked, 2 queued) of records 0..133.
8. **`unlockable_parse_record` is a 19-way dispatch of tiny XML field parsers**, each reading one child name (`Vehicles`, `Items`, `Crib_Name`, `Gang_Type`, `Vehicle_Group`, `Style`, `weapon_class`, `Team`, `Outfit`, ...). The reward application `unlockable_reward_apply` mirrors the same 28-way type switch.
9. **`0x7C21B0` is preceded by data** (the byte index table of the jump table used by `pcu_worn_remove_base_slots`, `0x7C2120`), so a "previous byte is padding" test fails for it; it is still a function start (50 callers).

## Structures

### The profile (`profile.dat`)

| Off | Field |
|---|---|
| +0x000 | block 1 (<= 1000 bytes) |
| +0x3E8 | block 2 (<= 1000 bytes) |
| +0x7D0 | block 3 = options: dword 8 (version), then the 0x15 dwords copied from the defaults `0xE8D0D8` at +4, `+0x58 = 0`, `+0x5C = 1`, `+0x60 = 0xD`, `+0x64 = 0xFFFF`, bytes `+0x66 = 0`, `+0x9D = 0` |
| +0xBB8 / +0xBBC / +0xBC0 | the sizes of blocks 1 / 2 / 3 |
| +0xBC4 | pointer to block 3 (set by `profile_options_defaults_init`) |
| +0xBC8 / +0xBCC | zeroed at init |

`profile_save` (`0x691290`) writes "%s\%s" built from `0x144E650` and `0xE0063C`; `profile_load` (`0x6910A0`, 0x1EE bytes, named earlier) reads the three blocks, copies them with `profile_copy` and runs the defaults when the file is missing. Both bracket the file access with `savedevice_acquire` / `savedevice_release` (critical section `0x27ABB9C`, mounted byte `0x2527BB8`).

### Player section (save section 3, `0x692300`)

Offsets are in the 0x3AC00-byte `bb_save_game`; the player is `[0x21703D4]`.

| Save off | Source | Notes |
|---|---|---|
| +0x9C / +0xA0 | player +0x11FC / +0x1200 | +0xA0 gets `+ 0x6A2650()` when `[0x27B3C60]` is set, `[0x2527C76]` is clear and the stack flag is clear |
| +0xA4 | `[0xE98528]` or 0 | `[0xE98528]` when player +0x11FC < `[0xE98528] * 0x63` (signed), else 0 |
| +0xA8 | `[0xE986F4]` or `[0x27ABFD8]` | `[0xE986F4]` when `[0x27ABFD8] == 3` or the session test holds (`[0x2528C14] != 0`, and either `[0x2528B56] == 0` or `[0x2528B2B] != 0`, and byte `[session+0x3F8] != 0`); otherwise `[0x27ABFD8]` |
| +0xAC | player +0x1208 | |
| +0xB0 | player +0xD8 | |
| +0xB4 | player +0x1114 clamped to [-2e9, 2e9] | cash |
| +0xB8 | player +0x1138, or 0x1F when `[[player+0x5E0]+0xC] != 0` and `byte [player+0x3CC] == 1` | |
| +0xBC / +0xBD | bytes player +0x18E9 / +0x18EA | |
| +0xC0 | `(player +0x10E4 - [0x145040C]) / 12` (multiplier 0x2AAAAAAB, shift 2) or -1 when +0x10E4 is 0 | persona index |
| +0xC4 | player +0x10BC | |
| +0xC8 | player +0x5E4 | |
| +0xCC / +0xD0 | `0x91DA00(&buf+0xD0, player)` | inventory |
| +0x1CC..+0x1DC.. | `gamesave_player_save_party_ids` | id list from player +0x13C0 |
| +0x2CC / +0x2D0 | `0x989D30(&buf+0x2D0, player)` | |
| +0x2E0 (u16) / +0x2E2 (byte) / +0x2E4 (f32) | player +0x18AC / `[0x2527B5A]` (co-op flag) / player +0x3D0 | |
| +0x310 (f32) | `[0x223F1EC]` | garage repair state |
| +0x314 .. | the 0xE454-byte customisation block | see below |
| +0x379CC / +0x379D0 | homie count and ids | `gamesave_homies_save_ids` |

### Customisation (pcu / pcr) data model

- **Owner objects** `[player+0x13B0]` (and the multiplayer copies) carry an owner index at `+0x10`. Per owner: worn entries at `0x1FAC858 + owner*0x1720` (0x50 bytes each, count `0x1FAC828[owner]`), an occupancy byte array whose pointer is `0x1FAC7F8[owner]` (length `[0x25288BC]`), and the avatar record `0x2CD0590 + owner*0x18F0` whose flag bytes `+0x1162 / +0x1163` mirror the occupied slots.
- A worn entry is `{...; +0x10 item*; +0x4C word; ...}`. The item pointed to by `+0x10` has the slot id byte at `+0x15`, the slot-count at `+0x14` and a pointer to its slot bytes at `+0x10` (read by `pcu_worn_rebuild_slot_flags`). Two further record types share the offsets `+0x18..+0x2C` and must not be confused: the **placement record** read by `pcu_item_required_slots_present` / `pcu_item_excluded_slots_absent` / `pcu_item_pick_valid` (required slot list: pointer `+0x18`, count `+0x1C`; excluded slot list: pointer `+0x20`, count `+0x24`; both int arrays), and the **catalogue record** read by the find helpers (variants: pointer `+0x20`, count `+0x24`, 0x28-byte entries with id at `+0xC` and name at `+8`; materials: pointer `+0x28`, count `+0x2C`, 0x70-byte entries with id at `+0x10` and name at `+0xC`).
- **Categories** are 0x78-byte records at `0x29E8CD0` (count `[0x25288D8]`, id at `+0x14`). Items are also kept in the circular list `0x25288B4` (next at `+0x74`, name hash at `+4`).
- **Stored outfits** live in the list `0x21603D0` (count `0x21603C8`); the save block stores them with `pcu_outfit_list_save`.

### Unlockables, stats, tutorials, discounts

| Table | Base | Stride | Count | Key fields |
|---|---|---|---|---|
| Unlockables | `0x27DD018` | 0xD0 | `[0x145A29C]` | `+0` name hash, `+4` type (0x10 = a type tested by `unlockable_is_type_16`), `+0xAC` field, `+0xC4` unlocked byte |
| Stat records | `0x1452BE0` | 0x44 | `[0x2527CE8]` | names at `0xE94F90` (0x18E entries) |
| Award condition rows | `0x27DA820` | 0x18 | 69 | six callbacks per type |
| Tutorials | `0x27DAE98` | 0x20 | 268 | `+0xC` state; `+8` flag; stack-built text records from index 134 |
| Store discounts | `0x29E8468` | 0x30 | `[0x1F99C80]` | `+8` id, `+0xC` value |
| Music store | albums `[0x1F99BF8]` (count `[0x1F91BF4]`, 12 bytes each) | tracks 0x2C | per album | `+0x14` current, `+0x18` committed, `+0x24` hash |
| Weapon store flags | `[0x2528820]` | 8 | `[0x2528824]` | `+0` hash, `+4` bit 0 = unlocked |
| Cribs | `[0x25274C0]` | 0x19C | `[0x25274B8]` | |
| Districts | `[0x2526C94]` | 0xA8 | `[0x2526C90]` (<= 0x3C) | |

## New names by group

### Save sections (72): 72

| Section | Save | Load | Co-op save | Co-op load |
|---|---|---|---|---|
| 0 timers | `0x692e80` | `0x694ab0` |  |  |
| 1 world_grid | `0x693570` | `0x694f30` | `0x693510` | `0x694ee0` |
| 2 core | `0x691ed0` | `0x6937c0` |  |  |
| 3 player_character | `0x692300` | `0x693fe0` |  |  |
| 4 districts | `0x692590` | `0x694390` | `0x6924f0` | `0x6942d0` |
| 5 options | `0x6935c0` | `0x694f80` | `0x693590` | `0x694f50` |
| 6 contacts_homies | `0x692040` | `0x693bf0` |  |  |
| 7 garage | `0x692630` | `0x694470` |  |  |
| 8 mission_state | `0x692a00` | `0x6946d0` |  |  |
| 9 activity_levels | `0x692b40` | `0x6947e0` | `0x692b00` | `0x6947c0` |
| 10 script_triggers | `0x692b80` | `0x694800` |  |  |
| 11 cribs | `0x692d60` | `0x694920` | `0x692c60` | `0x6948b0` |
| 12 tutorials | `0x693470` | `0x694e40` |  |  |
| 13 debug_display_options | `0x6934a0` | `0x694e70` |  |  |
| 14 playlists | `0x692ea0` | `0x694ad0` |  |  |
| 15 cutscene_seen | `0x692ec0` | `0x694b10` |  |  |
| 16 persistent_objects | `0x692f00` | `0x694ae0` |  |  |
| 17 rewards | `0x692f30` | `0x694b50` |  |  |
| 18 activity_instances | `0x692fb0` | `0x694bd0` | `0x692f80` | `0x694ba0` |
| 19 player_state | `0x692fd0` | `0x694bf0` |  |  |
| 20 cheats | `0x693450` | `0x694e20` |  |  |
| 21 diversions | `0x692760` | `0x6944e0` |  |  |
| 22 wiretap | `0x692750` | `0x6944c0` |  |  |
| 23 audio_a | `0x693720` | `0x6950c0` | `0x6935d0` | `0x694f90` |
| 24 audio_b | `0x693730` | `0x6950d0` |  |  |
| 25 audio_c | `0x693740` | `0x6950e0` |  |  |
| 26 music_store | `0x693760` | `0x6950f0` |  |  |
| 27 store_discounts | `0x6937a0` | `0x695130` | `0x693780` | `0x695110` |

### Profile, save device and gamesave helpers: 29

| Address | Name | Status | Role |
|---|---|---|---|
| `0x6909B0` | `savedevice_acquire` | inferred | stack args (flag, name): enters the critical section 0x27ABB9C, creates the save-device object (0xC15470 with 0xE0020C), calls its vtable slot 7 (+0x1C), initialises the pool 0x27ABB48 (mempool::init) and sets the mounted byte 0x2527BB8 = 1; returns AL = 1 on success |
| `0x690A70` | `savedevice_release` | inferred | stack arg (flag): under the critical section 0x27ABB9C frees the device object, clears 0x2527BB8 and calls the completion callback [0x2527BC4](flag, [0x2527BC8]) if set |
| `0x690C00` | `profile_default_init_scratch` | inferred | builds a default profile in a 0xBDC-byte stack scratch (memset 0xBD8, zeroes +0xBC8/+0xBCC, EAX = scratch for profile_options_defaults_init) and discards it |
| `0x690C50` | `profile_options_copy` | confirmed | copies the options block of a profile: EAX = source options, EDX = destination (dword, 0x15 dwords, 3 dwords, word, byte, 0xC dwords + 1 byte, byte, dword, 0xC dwords + 1 byte) |
| `0x690CD0` | `profile_clear_and_init` | confirmed | ESI = profile struct: memset 0xBD8 bytes to 0, zero +0xBC8 and +0xBCC, then EAX = ESI and tail-jump to profile_options_defaults_init |
| `0x690D00` | `profile_options_defaults_init` | confirmed | EAX = profile struct: +0xBC4 = &profile+0x7D0 (block 3), block header 8, copies the 0x15-dword default table 0xE8D0D8, sets defaults (+0x58 = 0, +0x5C = 1, +0x60 = 0xD, +0x64 = 0xFFFF), sets byte 0xE8D100 from [0xE8C470]; returns EAX = options pointer |
| `0x690DA0` | `profile_copy` | confirmed | stdcall (dst, src) ret 8: copies the 0x2F6 dwords (0xBD8 bytes) of a profile, re-points dst +0xBC4 via profile_options_defaults_init and copies the options with profile_options_copy |
| `0x691020` | `profile_write_blocks` | confirmed | EAX = profile struct, ESI = FILE*: writes the three profile blocks as (u32 size from +0xBB8/+0xBBC/+0xBC0, then the block at +0x0 / +0x3E8 / +0x7D0), then _fclose; returns AL = 1 |
| `0x691290` | `profile_save` | confirmed | writes profile.dat: builds "%s\%s" (0x144E650, 0xE0063C) with sprintf, fopen with the write mode 0xE00638, profile_write_blocks, savedevice_release; returns AL = success |
| `0x691600` | `gamesave_start_position_get` | inferred | EAX = save buffer, ECX = out vector 1, stack arg = out vector 2: looks up the nearest-crib record [buffer+0x94] in the crib array 0x25274C0 (stride 0x19C) and copies the 3 floats of its navpoint (+0x1C..+0x24); falls back to navpoint_find of two names |
| `0x6916E0` | `gamesave_stamp_date_time` | inferred | EDI = save buffer: writes the wide date (__wstrdate) and time (__wstrtime) strings used for the file name and list display |
| `0x691750` | `gamesave_compare_stamps` | inferred | ECX = first wide stamp, ESI = second, stack arg: narrows both 9-character wide strings and sscanf()s the digit groups of each with the format 0xE0069C; returns the ordering as an int (>= 0 keeps the entry) |
| `0x691910` | `gamesave_scan_register_entry` | inferred | ECX = file entry, stack = parsed save info: copies the 0x143-byte entry, orders it against the list 0x1438660 by date and time with gamesave_compare_stamps, formats the display text (wsprintf_1024) and stores it |
| `0x691E10` | `gamesave_read_file` | confirmed | stack args (out size*, name): builds "%s\%s%s" from 0x144E650, file_open, reads the whole file into a block from the "gamesave" pool via vtable slot 7 of 0x2527BD4, file_close; returns AL = success |
| `0x692110` | `gamesave_player_save_party_ids` | inferred | EDI = save buffer: copies the player human +0x13C0 id list (count +0x14B0) to buffer+0x1DC, then count to +0x1D8 and fields +0x14C0, +0x14C8, +0x14B8 to +0x1D4, +0x1CC, +0x1D0 |
| `0x692180` | `gamesave_homies_save_ids` | inferred | EDI = save buffer: stores the homie count [0x2528410] at +0x379CC and, for each active entry (stride 0x2C from 0x29DF058, flag at +0x1C), its id [+0xC] at +0x379D0+4i |
| `0x6921D0` | `gamesave_player_customisation_save` | confirmed | cdecl (buffer): copies the 0xE454-byte customisation block at buffer+0x314 from the multiplayer snapshot (0x6A4C90) or builds it from the live player: 0x9EDFF0, 0x7D84C0, 0x7C3CC0, 0x7C2510, 0x9A02B0, 0x7D8980, then 0x7C3230 and 0x7C3310 for the tail |
| `0x692D00` | `gamesave_crib_record_copy` | confirmed | EAX = destination, ECX = source: copies one 0x27-byte crib save record (dword, bit 0 of byte 4, bytes 5 and 6, two 16-byte blocks at +7 and +0x17) |
| `0x693D80` | `gamesave_player_inventory_restore` | confirmed | EAX = save buffer: calls human_inventory_restore(buffer+0xD0, count [buffer+0xCC], player 0x21703D4) and 0x989D70(buffer+0x2D0, [buffer+0x2CC], player) |
| `0x693EB0` | `gamesave_player_customisation_load` | inferred | cdecl (buffer, use_saved_list): restores the customisation block at buffer+0x314 into the player (0x13B0 customisation object): 0x9EE0D0, and either 0x7C3980 + 0x7C26A0 or 0x7C3750 depending on the flag, then 0x7C3E00, 0x7D8530, 0x7D8A50, 0x7C3290, 0x7C3480 |
| `0x6953C0` | `gamesave_parse_digits` | confirmed | EAX = digit count, ECX = pointer to ASCII digits: returns the signed decimal value of that many characters (a leading minus makes it negative); returns 0 for a null pointer |
| `0x695C00` | `gamesave_load_net_announce` | inferred | builds a 0x264-byte packet (memset, 0x69BD30 for the name, copies the 0x143-byte save info) and sends it with 0x69BED0 unless 0x2527BBA is set |
| `0x695D20` | `gamesave_load_from_file` | inferred | ECX = save name: creates the 1 MB "gamesave" pool (0xC15470), reads the file with gamesave_read_file, applies it with gamesave_load_finish unless client in a session, resets player/HUD state, sends the load packet with net_send_packet_to_remotes; callers saveload_continue_game and saveload_coop_load_error |
| `0x695FC0` | `gamesave_load_finish` | confirmed | stack args (save info, apply flag): for a file older than version 0x6B marks the info invalid, otherwise runs gamesave_apply; frees the pool 0x2527BD4 and, when the flag is set, ECX = 2 for 0x4F50E0 |
| `0x696120` | `saveload_poll_status` | inferred | returns EAX = [0x2527BE0] (the pending save/load result); while the autosave notice is up and the timer 0x2526DA0 is not zero it pops saveload_autosaving_text, and calls gamesave_fail when [0x2527BD2] is set |
| `0x696180` | `savegame_cash_add_clamped` | inferred | EAX = amount, ESI = record: clamps the amount (to +-0x77359400 after scaling by 100 when between the limits) and adds it to the signed 32-bit cash field [esi+0xB4], saturating at +-2,000,000,000; ESI = 0 does nothing |
| `0x696200` | `gamesave_set_object_table_bit` | confirmed | EAX = pointer into the 0x498-byte record table 0x22D7BCC, stack arg = save buffer: converts the pointer to its index (division by 0x498; 0xFF when outside the table) and sets that bit in the byte array at buffer+0x3A163 |
| `0x6962F0` | `gamesave_garage_entry_append` | confirmed | EAX = save buffer, stack arg = vehicle record: appends a 0x434-byte entry at buffer+0x175DC (count at +0x175D8, stored from 0x1409998 into +0x37270) copying type, colours and 0xEE dwords of state |
| `0x696B10` | `gamesave_count_unowned_districts` | confirmed | returns EAX = number of entries of the replicated district table 0x27ABD08 (count 0x27ABD00, 12-byte entries) whose first dword is 0; this is the count written into the save file name |

### Customisation (pcu / pcr): 36

| Address | Name | Status | Role |
|---|---|---|---|
| `0x63EF60` | `pcu_purchase_slot_charge` | inferred | ret 4: charges the cash cost of a wardrobe slot purchase; sole caller luafn_pcu_purchase_slot |
| `0x7BEE40` | `pcu_worn_rebuild_slot_flags` | confirmed | EAX = customisation object: clears the occupancy bytes of its owner (pointer table 0x1FAC7F8, 0x25288BC slots), then sets the byte of every slot used by a worn item and the avatar flag bits in [0x2CD0590 + owner*0x18F0 + 0x1162/0x1163] by comparing with the slot ids at 0xE8D3FC, 0xE8D400, 0xE8D428, 0xE8D470, 0xE8D44C, 0xE8D494 |
| `0x7BF550` | `pcu_worn_resolve_conflicts` | inferred | cdecl (object, keep_entry): walks the worn entries and removes (0x7C2000) those that conflict with the slots required or excluded by the entry that was just added, checking the slot mask at [0x29E8CC4] |
| `0x7C0C30` | `pcr_default_clothing_apply` | inferred | cdecl (object): wears the default item for each of the 58 slots (jump tables 0x7C11D4 / 0x7C1218): sprintf slot labels, 0x9E8ED0, pcu_worn_rebuild_slot_flags, pcu_worn_resolve_conflicts, 0x7C52B0, 0x7C5460; callers gameplay_enter, luafn_pcr_add_default_clothing, luafn_pcr_set_identity |
| `0x7C1260` | `pcu_purchase_slot_apply` | inferred | cdecl (object, slot id, item, ...): applies the slot a customisation purchase just paid for: updates the worn entry (pcu_worn_entry_find_by_slot), sprintf label, rebuilds flags, 0x7CE320 refresh; callers luafn_pcu_purchase_slot, 0x7D6B70, 0x7D8EA0 |
| `0x7C1A40` | `pcu_category_find_by_id` | confirmed | ESI = category id: pointer to the category record (stride 0x78 from 0x29E8CD0, count [0x25288D8]) whose +0x14 equals the id, else 0 |
| `0x7C1B50` | `pcu_item_find_variant_by_id` | confirmed | EDX = item record (0 -> 0), ESI = id: pointer to the variant entry (stride 0x28, array [item+0x20], count [item+0x24]) whose +0xC equals the id, else 0 |
| `0x7C1B80` | `pcu_item_find_variant_by_name` | confirmed | EDI = item record (0 -> 0), stack arg = name: pointer to the variant entry (stride 0x28) whose string [+8] equals the name (_stricmp), else 0 |
| `0x7C1BD0` | `pcu_item_find_material_by_id` | confirmed | EDX = item record (0 -> 0), ESI = id: pointer to the material entry (stride 0x70, array [item+0x28], count [item+0x2C]) whose +0x10 equals the id, else 0 |
| `0x7C1C00` | `pcu_item_find_material_by_name` | confirmed | EDI = item record (0 -> 0), stack arg = name: pointer to the material entry (stride 0x70) whose string [+0xC] equals the name (_stricmp), else 0 |
| `0x7C1C50` | `pcu_item_pick_valid` | inferred | cdecl (object, item, fallback list item) -> EAX: returns the item if its required slots are present and excluded slots absent for the object, otherwise tries the fallback item list at [fallback+0x20]; 0 if none fit |
| `0x7C1D40` | `pcu_item_required_slots_present` | confirmed | ECX = item record, stack arg = slot occupancy bytes, EDI = slot count: AL = 1 when every slot listed at [item+0x18] (count [item+0x1C]) is below EDI and occupied (byte != 0) |
| `0x7C1DA0` | `pcu_item_excluded_slots_absent` | confirmed | ECX = item record, stack arg = slot occupancy bytes, EDI = slot count: AL = 1 when none of the slots listed at [item+0x20] (count [item+0x24]) is occupied by a byte equal to 1 |
| `0x7C1DF0` | `pcu_worn_add_item_checked` | inferred | EAX = item (0 -> AL = 0), stack args: calls 0x7BF000 to build the worn entry and, if it returns one, pcu_worn_resolve_conflicts; AL = 1 on success |
| `0x7C2000` | `pcu_worn_entry_remove` | confirmed | cdecl (object, index, no_refresh): removes worn entry `index` by memmove of the tail (0x50-byte entries), decrements the count, calls 0x9E8FE0(entry) and, unless the flag is set, pcu_worn_rebuild_slot_flags and 0x7BF550 |
| `0x7C20B0` | `pcu_worn_remove_by_slot` | confirmed | cdecl (object, slot id, no_refresh) -> EAX = number removed: removes every worn entry whose item slot id matches by calling pcu_worn_entry_remove |
| `0x7C2120` | `pcu_worn_remove_base_slots` | inferred | EDI = customisation object: walks the worn entries and, for every entry whose item slot id is <= 0x2B and whose byte-table class (table 0x7C2184, jump table 0x7C217C) is 0, removes all entries of that slot with pcu_worn_remove_by_slot(object, slot, 0); the class-0 slots are 0..5, 38, 39, 40, 42 and 43 |
| `0x7C21B0` | `pcu_worn_entry_copy_by_slot` | confirmed | EAX = customisation object, EDX = slot id, stack arg = out 0x50-byte buffer: finds the worn entry (table 0x1FAC858 + owner*0x1720, stride 0x50, count 0x1FAC828[owner]) whose item [+0x10]->[+0x15] equals the slot and copies it; AL = found |
| `0x7C2200` | `pcu_worn_entry_set_variant` | inferred | EAX = customisation object, ESI = slot id, stack arg = word: stores the word into the worn entry [+0x4C] for that slot |
| `0x7C22C0` | `pcu_worn_entry_find_by_slot` | confirmed | EAX = customisation object, ESI = slot id, stack arg = out entry**: AL = 1 and *out = pointer to the worn entry for the slot, else AL = 0 |
| `0x7C2510` | `pcu_worn_list_export` | inferred | stack args (out array, max, object), EDI = out count*: memsets max*0x40 bytes and exports one 0x40-byte record per worn entry (item id, material id, variants) of the object |
| `0x7C26A0` | `pcu_worn_list_import` | inferred | cdecl (object, records, count): imports saved 0x40-byte worn records into an object: resets (_memset), resolves each record to an item (pcu_item_find...), wears it (0x7C5030), rebuilds flags and resolves conflicts; counterpart of pcu_worn_list_export; callers 0x610D10, 0x693EB0 (customisation load) |
| `0x7C3980` | `pcu_worn_reset_all` | inferred | EAX = customisation object: calls 0x9E8FE0 for every worn entry, 0x9E8DC0(1), 0x9E9130, 0x9E8DC0(1) and clears the four per-owner tables (memset x4); 8 callers incl. cutscene_stop |
| `0x7C3CC0` | `pcu_outfit_list_save` | inferred | EAX = destination block (0xC000 bytes zeroed), stack arg = out count: copies the stored-outfit list (count 0x21603C8, entries from 0x21603D0) as id + data records |
| `0x7C3E00` | `pcu_outfit_list_load` | inferred | EAX = count, ECX = saved id list: clears 0x21603C8 and re-adds each saved outfit by looking its id up in the category table 0x29E8CD0 (stride 0x78, id +0x14) |
| `0x7C4280` | `pcu_worn_set_from_item_list` | inferred | builds the worn-entry set of a customisation object from a list of items (alloca 16-byte aligned scratch, _memset, 0x7BEF90 slot lookup, _sprintf labels, pcu_worn_rebuild_slot_flags, pcu_worn_resolve_conflicts, _memmove of entries); callers 0x53FDD0, cutscene_begin_callback, 0x7C7D90 |
| `0x7C4E30` | `pcu_item_find_by_name` | confirmed | EAX/stack = name: string_hash_lower, then walks the circular list at 0x25288B4 (next at +0x74) for the item whose hash [+4] matches; 0 if none |
| `0x7CD530` | `pcu_areas_build` | inferred | builds the list of wardrobe areas and their categories for the current player; callers luafn_pcu_get_areas and pcu_category_set_active |
| `0x7CE320` | `pcu_slot_cache_refresh` | inferred | cdecl (slot id or -1, flag): for each of the 0x2B slots in the cache 0x1FC6900 (stride 0x50) that matches the filter and is not worn by the player (pcu_worn_entry_copy_by_slot), clears its 0x50 bytes; jump table 0x7CE3C0 selects slots 0..0x2B |
| `0x7CE420` | `pcu_category_set_active` | inferred | builds the item list of the new active category for the wardrobe: pcu_worn_remove_by_slot for the old area, 0x7CD530 to rebuild areas, sprintf of the label, resolves worn conflicts and refreshes; callers luafn_pcu_set_active_category and luafn_pcu_category_nav_clear_slot |
| `0x7D7180` | `pcr_apply_preset_impl` | inferred | applies a player creation preset: stringifies the preset, applies morphs/hair/clothing, calls 0x7C0C30, 0x95CDB0, net_send_human_pkt_5; stack frame with a security cookie; two jump tables (4 cases each at 0x7D80B0 / 0x7D80C0); callers luafn_pcr_apply_preset and 0x7D81F0 |
| `0x81CB90` | `pcu_net_write_customisation` | inferred | packs the customisation of a human into the multiplayer bit stream (net_bits_write_bit, 0x6DED50, 0x7C1BD0, 0x7C5270, 0x7C58D0); caller 0x9EA1A0 |
| `0x81DFC0` | `pcr_saved_character_apply` | inferred | applies a loaded saved character (multiplayer): 0x7C0370, 0x7D8A50, human model/outfit setters 0x95CDB0/0x968BB0/0x965600, per-slot loop through the jump table 0x81E4F0 (44 cases), 0x7C0C30; caller multi_load_saved_character_impl |
| `0x9DE0C0` | `pcu_outfit_purchase_charge` | inferred | charges and records a clothing / outfit purchase; callers luafn_pcu_purchase_outfit and rewards 0x6BBEB0 |
| `0x9DE200` | `pcu_outfit_delete_impl` | inferred | deletes a stored outfit; sole caller luafn_pcu_delete_outfit |
| `0x9DF370` | `pcu_slot_discard_impl` | inferred | discards the pending change of a slot; sole caller luafn_pcu_discard_slot |

### Unlockables and rewards: 25

| Address | Name | Status | Role |
|---|---|---|---|
| `0x6BACD0` | `unlockable_parse_record` | confirmed | thiscall (xml node) ret 8: parses one <Unlockable> record (Category, Type, HomieSlot1..3) and dispatches on the type to the unlockable_parse_* helpers 0x6BB190..0x6BBD10; callers xtbl_load_unlockables and 0x6BC5E0 |
| `0x6BB190` | `unlockable_parse_vehicles` | confirmed | EAX = xml node, stack arg = out record: parses <Vehicles>/<Vehicle> entries (Type, Variant) of a vehicle unlock; ret 4 |
| `0x6BB390` | `unlockable_parse_contact` | inferred | EAX = xml node, stack arg = out record: reads Name and resolves it through 0x786FB0 (cell phone contact); ret 4 |
| `0x6BB3E0` | `unlockable_parse_weapon_primary` | inferred | EAX = xml node, stack arg = out record: Name resolved with weapon_info_find_by_name; ret 4 |
| `0x6BB420` | `unlockable_parse_weapon_secondary` | inferred | EAX = xml node, stack arg = out record: Name resolved with weapon_info_find_by_name; ret 4 |
| `0x6BB460` | `unlockable_parse_item` | inferred | EAX = xml node, stack arg = out record: Name hashed with string_hash_lower and resolved with 0x91C290 (inventory item info); ret 4 |
| `0x6BB4D0` | `unlockable_parse_crib` | confirmed | EAX = xml node, stack arg = out record: Crib_Name hashed, Level via xtbl_get_int; ret 4 |
| `0x6BB530` | `unlockable_parse_named_amounts` | inferred | thiscall-like (ECX = xml node) ret 4: reads name_%d and Amount children (sprintf, xtbl_get_float) for a list of named amounts |
| `0x6BB610` | `unlockable_parse_items` | confirmed | EAX = xml node, stack arg = out record: parses <Items>/<Item> with ItemName, ItemVariant, Color1..3, resolving items with pcu_item_find_by_name / pcu_item_find_material_by_name (clothing unlock); ret 4 |
| `0x6BB970` | `unlockable_parse_amount` | confirmed | EAX = xml node, stack arg = out record: Amount via xtbl_get_int; ret 4 |
| `0x6BB9B0` | `unlockable_parse_source_amount` | confirmed | EAX = xml node, stack arg = out record: Source resolved by 0x9DF2C0 and Amount; ret 4 |
| `0x6BBA10` | `unlockable_parse_score_amount` | confirmed | EAX = xml node, stack arg = out record: Score resolved with notoriety_team_from_name and Amount; ret 4 |
| `0x6BBA90` | `unlockable_parse_crib_ref` | inferred | EAX = xml node, stack arg = out record: Name resolved by 0x548330 (crib lookup by hash); ret 4 |
| `0x6BBAD0` | `unlockable_parse_music_track` | inferred | EAX = xml node, stack arg = out record: Name hashed and resolved through 0x7AEB60 (music store track lookup); ret 4 |
| `0x6BBB40` | `unlockable_parse_gang_type` | confirmed | EAX = xml node, stack arg = out record: Gang_Type hashed; ret 4 |
| `0x6BBB80` | `unlockable_parse_vehicle_group` | confirmed | EAX = xml node, stack arg = out record: Vehicle_Group hashed; ret 4 |
| `0x6BBBC0` | `unlockable_parse_style` | confirmed | EAX = xml node, stack arg = out record: Style hashed; ret 4 |
| `0x6BBC20` | `unlockable_parse_weapon_class` | confirmed | EAX = xml node, stack arg = out record: weapon_class child resolved by 0xB71730; ret 4 |
| `0x6BBCB0` | `unlockable_parse_team` | confirmed | EAX = xml node, stack arg = out record: Team compared with _stricmp and resolved by 0xB75580; ret 4 |
| `0x6BBD10` | `unlockable_parse_outfit` | confirmed | EAX = xml node, stack arg = out record: Outfit child resolved with pcu_item_find_by_name; ret 4 |
| `0x6BBEB0` | `unlockable_reward_apply` | inferred | stdcall (reward record) ret 4: applies an unlocked reward by type through the 28-entry jump table 0x6BC548 (case bodies call vehicle_create + vehicle_from_handle, 0x787380 contact, weapon_info_is_firearm/is_thrown + ammo_store_add_type, 0x7B35E0 weapon-store unlock, 0x7ADA20 music, saints_hq_set_upgrade_level crib upgrade, 0x9DE0C0/0x7D9590 clothing, float stores to the player +0x10C0/+0x18B4/+0x18B8); caller rewards_do_frame |
| `0x6BC5E0` | `unlockable_reward_unlocked_notify` | inferred | queues the reward-unlocked message: 0x555610, vint document 0x2527F90, 0x6962F0 x2, stat_award_check; caller rewards_do_frame |
| `0x6BCE10` | `unlockable_get_field_ac` | confirmed | ECX = pointer to a name hash: EAX = dword [rec+0xAC] of the unlockable record, 0 when absent |
| `0x6BCF10` | `unlockable_is_type_16` | confirmed | ECX = pointer to a name hash: AL = 1 when the unlockable record has type [+4] == 0x10; 11 callers |
| `0x6BCFB0` | `unlockable_is_unlocked` | confirmed | ECX = pointer to a name hash: AL = byte [rec+0xC4] of the record (table 0x27DD018, stride 0xD0, count 0x145A29C) whose hash [+0] matches, 0 when absent; 13 callers |

### Stats, awards, tutorials: 14

| Address | Name | Status | Role |
|---|---|---|---|
| `0x6B2C30` | `stats_xtbl_load` | confirmed | loads stats.xtbl (string 0xE0983C) and parses every <Stat> element (_stricmp on field names, xtbl_get_int/float, lstr_get): fills the stat records at 0x1452BE8 (stride 0x44) and counts 0x2527CE8 / 0x2527CEC; jump tables 0x6B38B0 (197 cases) and 0x6B3A58 (123 cases); caller 0x6B3BB0 |
| `0x6B3BB0` | `stat_award_table_init` | inferred | calls stats_xtbl_load and then fills the award condition table 0x27DA820 (24-byte records: callback at +4 .. +0x10 per award type) with straight-line stores; caller game_init_systems |
| `0x6B4C80` | `stat_apply_award` | inferred | EAX = stat id: ignores ids <= -1, >= 0x2527CE8 and some by network/co-op state, then looks up the stat record (0x1452BE8, stride 0x44) and awards it through stat_award_check; 11 callers (diversion_award_cash_and_respect, 0x605780, ...) |
| `0x6B4D50` | `stat_award_if_allowed` | inferred | ESI = stat id: stats_can_award, then stat_apply_award unless co-op client / 0x2527B5A / 0x27B3C60 state blocks it |
| `0x6B4E30` | `stat_award_by_type` | inferred | EAX = type argument, ESI = stat id: stats_can_award, 0x6B08C0 (is the type countable) then stat_apply_award |
| `0x6B4EA0` | `stat_award_float_type` | inferred | ESI = stat id: stats_can_award, 0x6B0940 type test (51-entry jump table) then stat_apply_award |
| `0x6B4F20` | `stat_award_and_send` | inferred | ECX = value, ESI = stat id: in a co-op session as a network client sends the stat to the host (0x528-byte stack packet via net_send_packet_to_remotes) after range and type checks (0x6B09F0, 0x6B53E0, 0x6B08C0); 35 callers |
| `0x6B53E0` | `stat_record_get` | confirmed | EAX = stat id, stack arg = out record**: AL = 1 and *out = 0x1452BE0 + id*0x44 when 0 <= id < [0x2527CE8], else AL = 0 |
| `0x6B56A0` | `stats_flush_pending` | inferred | under the critical section 0x27DA804: copies the pending stat values (0x1452BE0 records, 0x140FFA0 buffers) to the profile store; returns AL; callers gameplay_exit, gamesave_fail, gamesave_save |
| `0x6B5990` | `stat_awards_check_group` | inferred | checks a group of award conditions in sequence: for each of a fixed list of stat ids reads the player/human state (0x145779C.. condition tables, object table 0x2149C64), calls stats_can_award and stat_award_check; callers 0x87DD50 and 0x8A27F0 (multiplayer); jump table 0x6B6360 (4 cases) |
| `0x6B6C50` | `tutorial_queue_push` | confirmed | EAX = tutorial index, stack args (a, b): moves record 0x27DAE98 + index*0x20 onto the circular list headed at 0x1451ADC, state [+0xC] = 2, [+0x14] = a, [+0x18] = b |
| `0x6B71B0` | `tutorial_mark_unlocked` | confirmed | EAX = tutorial index (<= 0x85): sets the unlock dword [0x27DAEA4 + index*0x20] = 1 unless disabled by 0x2527CF5 or by the network session state |
| `0x6B7260` | `tutorial_trigger` | inferred | EAX = tutorial index: network-aware trigger: tutorial_mark_unlocked, 0x574DD0, 0x6B6550, 0x79C150, net_clock_ms, net_send_packet_to_remotes, tutorial_queue_push; 69 callers |
| `0x6B8470` | `tutorials_table_init` | confirmed | straight-line initialiser of the tutorial table at 0x27DAE98 (0x20-byte records, 268 of them): records 0..133 get their +8 / +0xC dwords, records 134..267 get an 8-dword text record built on the stack (text id string, flags, callback, title string) and copied with rep movsd; no calls; callers 0x51F600, 0x6B65D0, net_recv_tutorials |

### Stores, music store, discounts: 19

| Address | Name | Status | Role |
|---|---|---|---|
| `0x63ECE0` | `liquor_store_purchase_check` | inferred | ret 4: checks the session / mode state (0x2528B48, 0x2528B49, 0x27B3C60, 0x2527C76) and the table 0xEA298C (count 0xEA1210) before a liquor purchase; sole caller luafn_liquor_store_make_purchase |
| `0x7A65C0` | `car_dealership_parse_record` | inferred | thiscall (ECX = out record) parses one car dealership xml record (_stricmp on field names, string_hash_lower, xtbl_get_int, xml_count_children, _atol, string_crc32, _strncpy x3); callers car_dealerships_load_table and 0x7A77D0 |
| `0x7AD290` | `liquor_store_purchase_apply` | inferred | applies a liquor store purchase (sole caller luafn_liquor_store_make_purchase) |
| `0x7AEA50` | `music_store_selection_reset` | inferred | for every album in 0x1F99BF8 (count 0x1F91BF4) and every 0x2C-byte track copies [track+0x18] to [track+0x14]; callers 0x698810 and music_store_purchases_load |
| `0x7AEB60` | `music_store_find_track_by_hash` | inferred | stack arg = pointer to a hash: scans all albums/tracks (stride 0x2C) for [track+0x24] == hash; 5 callers (rewards, store scripts) |
| `0x7AEBD0` | `music_store_purchases_save` | confirmed | ECX/stack = buffer: writes the 0x5A-byte music store block (version 0xC, 2 bits per track from 0x1F99BF8) via memset/memcpy; called by save section 26 |
| `0x7AED60` | `music_store_purchases_load` | confirmed | stack = buffer: reads the music store block back (0x7AEA50 then memcpy); called by save section 26 load |
| `0x7AFE60` | `store_discount_send_packet` | inferred | builds a stack packet and sends it with net_send_packet_to_remotes; callers 0x7AFF00 and scheduled_world_events_update |
| `0x7AFF00` | `store_discount_set_and_send` | inferred | applies one discount (0x48B1C0, 0x68B240) and broadcasts it with store_discount_send_packet; callers misc_packet_sub_11, scheduled_world_events_update, luafn_store_discount |
| `0x7B01E0` | `store_discounts_expire` | inferred | removes discount entries whose expiry is before the game date 0x140E728; callers 0x698810, 0x7B08C0, self |
| `0x7B0560` | `store_discount_lookup_clamp` | inferred | ECX = pointer to a store id, stack float: finds the discount entry (0x29E8468 + 0x30*i, id at +8) and returns the value clamped against the entry value [+0xC] |
| `0x7B0720` | `store_price_discount_factor` | inferred | x87: returns ST0 = the price factor derived from the global discount floats 0x25287F0 / 0x25287F4 clamped to 0..1; 28 callers (every purchase path) |
| `0x7B07F0` | `store_discounts_save` | confirmed | saves the discount table (0x29E8480, 0x30-byte entries, count 0x1F99C80) into the buffer; called by save section 27 |
| `0x7B08C0` | `store_discounts_load` | confirmed | loads the discount table from the save, then expires old entries with 0x7B01E0 and 0x68B240; called by save section 27 load |
| `0x7B1A80` | `weapon_store_give_weapon` | inferred | EAX = store item: inventory_find_weapon, 0x91D380 x4, 0x55A740: adds the weapon to the inventory with default ammo; caller 0x7B1E30 |
| `0x7B1E30` | `weapon_store_ensure_weapon` | inferred | ensures the player holds the weapon being bought (calls 0x7B1A80); callers weapon_store_purchase_weapon_impl and the two luafn purchase bindings |
| `0x7B2020` | `weapon_store_purchase_weapon_impl` | confirmed | finds the weapon in the player inventory (inventory_find_weapon), compares the store item name (_stricmp), charges cash (cash_from_dollars, net_send_prop_human_cash_adjust) with store_price_discount_factor, adds the weapon or ammo reserve; pushes script strings for the result; sole caller luafn_weapon_store_purchase_weapon |
| `0x7B25B0` | `weapon_store_purchase_ammo_impl` | confirmed | inventory_find_weapon, store_price_discount_factor, ammo_store_add_type, cash_from_dollars, net_send_prop_human_cash_adjust, 0x9F5AE0; sole caller luafn_weapon_store_purchase_ammo |
| `0x7B35E0` | `store_weapon_unlock_by_name` | confirmed | ECX = name string (0 -> nothing): hashes it and sets bit 0 of the flags of the matching entry in the weapon-store table (0x2528820, 8-byte entries, count 0x2528824); callers 0x6BBEB0 and 0x6BD2D0 |

### Cribs and save helpers (rest): 7

| Address | Name | Status | Role |
|---|---|---|---|
| `0x5455A0` | `crib_upgrade_apply_and_send` | inferred | ECX = crib upgrade request: applies the upgrade (world_chunk_find_id, 0x545200, 0x545110, 0x545260, 0x549720) and sends a stack-built packet with net_send_packet_to_remotes; callers crib_lookup, misc_packet_saints_hq_upgrade, saints_hq_set_upgrade_level |
| `0x545B80` | `crib_customization_charge_a` | inferred | EAX = amount: cash_from_dollars then 0x545C70 (afford check); callers confessionals_get_nearest_trigger_impl and luafn_crib_customization_make_purchase |
| `0x545BD0` | `crib_customization_charge_b` | inferred | sibling of 0x545B80 for the second price tier |
| `0x548590` | `crib_purchase_apply` | inferred | EDX = crib: marks the crib bought (session_hosting_flag, trigger_deactivate, 0x5487C0, stat_award_check) and tells the remotes; callers luafn_crib_purchase_purchase_crib, misc_packet_sub_05, 0x5498C0 |
| `0x5487C0` | `crib_triggers_refresh` | inferred | activates / deactivates the nine crib-related triggers according to ownership (trigger_activate x9, trigger_deactivate x8); callers 0x5484D0, 0x548590, 0x5498C0 |
| `0x549010` | `crib_current_index_for_save` | inferred | EDI = crib or district context: computes the crib index written into the save file name (field 6 of gamesave_build_name); callers gamesave_build_name, 0x76E180, 0x777DE0 |
| `0x549C90` | `crib_purchase_triggers_set` | inferred | activates or deactivates the purchase trigger of a crib; callers crib_purchase_lua_register, 0x693EB0 (save load), luafn_crib_purchasing_unlock |

## Ten large port maps

Sizes are true sizes (recursive descent). The ten are the largest functions of this area that are portable as Rust (no unrecoverable inline-asm tricks; the x87 code is small and listed).

## Map: `tutorials_table_init` (0x6B8470, 0x26B7 bytes)

Straight-line initialiser: for 134 records it builds an 8-dword record on the stack (text id string, three zero dwords, a flag dword, a callback pointer, a title string, 0) and copies it with `rep movsd` (ecx = 8) to `0x27DAE98 + index*0x20`; for the first records it only stores the `+8` / `+0xC` dwords (`0x27DAEA0`, `0x27DAEA4`). Port as a table-driven loop; the strings and the 29 callbacks are exe addresses that other code compares by pointer, so keep them as constants.

### tutorials_table_init (0x6B8470)

- **True size** 0x26B7 (9911) bytes by recursive descent; 1916 instructions; ret imm [0]; entry regs read (linear prefix) none.
- Frame: frame sub esp,0x1a4.
- **Callees** (0 call sites, 0 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
- **Globals read**: none
- **Globals written**: 0x27DAEA0, 0x27DAEA4, 0x27DAEC0, 0x27DAEC4, 0x27DAEE0, 0x27DAEE4, 0x27DAF00, 0x27DAF04, 0x27DAF20, 0x27DAF24, 0x27DAF40, 0x27DAF44, 0x27DAF60, 0x27DAF64, 0x27DAF80, 0x27DAF84, 0x27DAFA0, 0x27DAFA4, 0x27DAFC0, 0x27DAFC4, 0x27DAFE0, 0x27DAFE4, 0x27DB000, 0x27DB004, 0x27DB020, 0x27DB024, 0x27DB040, 0x27DB044, 0x27DB060, 0x27DB064, 0x27DB080, 0x27DB084, 0x27DB0A0, 0x27DB0A4, 0x27DB0C0, 0x27DB0C4, 0x27DB0E0, 0x27DB0E4, 0x27DB100, 0x27DB104, 0x27DB120, 0x27DB124, 0x27DB140, 0x27DB144, 0x27DB160, 0x27DB164, 0x27DB180, 0x27DB184, 0x27DB1A0, 0x27DB1A4, 0x27DB1C0, 0x27DB1C4, 0x27DB1E0, 0x27DB1E4, 0x27DB200, 0x27DB204, 0x27DB220, 0x27DB224, 0x27DB240, 0x27DB244, 0x27DB260, 0x27DB264, 0x27DB280, 0x27DB284, 0x27DB2A0, 0x27DB2A4, 0x27DB2C0, 0x27DB2C4, 0x27DB2E0, 0x27DB2E4, 0x27DB300, 0x27DB304, 0x27DB320, 0x27DB324, 0x27DB340, 0x27DB344, 0x27DB360, 0x27DB364, 0x27DB380, 0x27DB384, 0x27DB3A0, 0x27DB3A4, 0x27DB3C0, 0x27DB3C4, 0x27DB3E0, 0x27DB3E4, 0x27DB400, 0x27DB404, 0x27DB420, 0x27DB424, 0x27DB440, 0x27DB444, 0x27DB460, 0x27DB464, 0x27DB480, 0x27DB484, 0x27DB4A0, 0x27DB4A4, 0x27DB4C0, 0x27DB4C4, 0x27DB4E0, 0x27DB4E4, 0x27DB500, 0x27DB504, 0x27DB520, 0x27DB524, 0x27DB540, 0x27DB544, 0x27DB560, 0x27DB564, 0x27DB580, 0x27DB584, 0x27DB5A0, 0x27DB5A4, 0x27DB5C0, 0x27DB5C4, 0x27DB5E0, 0x27DB5E4, 0x27DB600, 0x27DB604, 0x27DB620, 0x27DB624, 0x27DB640, 0x27DB644, 0x27DB660, 0x27DB664, 0x27DB680, 0x27DB684, 0x27DB6A0, 0x27DB6A4, 0x27DB6C0, 0x27DB6C4, 0x27DB6E0, 0x27DB6E4, 0x27DB700, 0x27DB704, 0x27DB720, 0x27DB724, 0x27DB740, 0x27DB744, 0x27DB760, 0x27DB764, 0x27DB780, 0x27DB784, 0x27DB7A0, 0x27DB7A4, 0x27DB7C0, 0x27DB7C4, 0x27DB7E0, 0x27DB7E4, 0x27DB800, 0x27DB804, 0x27DB820, 0x27DB824, 0x27DB840, 0x27DB844, 0x27DB860, 0x27DB864, 0x27DB880, 0x27DB884, 0x27DB8A0, 0x27DB8A4, 0x27DB8C0, 0x27DB8C4, 0x27DB8E0, 0x27DB8E4, 0x27DB900, 0x27DB904, 0x27DB920, 0x27DB924, 0x27DB940, 0x27DB944, 0x27DB960, 0x27DB964, 0x27DB980, 0x27DB984, 0x27DB9A0, 0x27DB9A4, 0x27DB9C0, 0x27DB9C4, 0x27DB9E0, 0x27DB9E4, 0x27DBA00, 0x27DBA04, 0x27DBA20, 0x27DBA24, 0x27DBA40, 0x27DBA44, 0x27DBA60, 0x27DBA64, 0x27DBA80, 0x27DBA84, 0x27DBAA0, 0x27DBAA4, 0x27DBAC0, 0x27DBAC4, 0x27DBAE0, 0x27DBAE4, 0x27DBB00, 0x27DBB04, 0x27DBB20, 0x27DBB24, 0x27DBB40, 0x27DBB44, 0x27DBB60, 0x27DBB64, 0x27DBB80, 0x27DBB84, 0x27DBBA0, 0x27DBBA4, 0x27DBBC0, 0x27DBBC4, 0x27DBBE0, 0x27DBBE4, 0x27DBC00, 0x27DBC04, 0x27DBC20, 0x27DBC24, 0x27DBC40, 0x27DBC44, 0x27DBC60, 0x27DBC64, 0x27DBC80, 0x27DBC84, 0x27DBCA0, 0x27DBCA4, 0x27DBCC0, 0x27DBCC4, 0x27DBCE0, 0x27DBCE4, 0x27DBD00, 0x27DBD04, 0x27DBD20, 0x27DBD24, 0x27DBD40, 0x27DBD44, 0x27DBD60, 0x27DBD64, 0x27DBEA0, 0x27DBEA4, 0x27DBEC0, 0x27DBEC4, 0x27DBEE0, 0x27DBEE4, 0x27DBF00, 0x27DBF04, 0x27DBF20, 0x27DBF24, 0x27DBF40, 0x27DBF44, 0x27DBD80, 0x27DBD84, 0x27DBDA0, 0x27DBDA4, 0x27DBDC0, 0x27DBDC4, 0x1459A58, 0x27DBDE4, 0x27DBE04, 0x27DBE24, 0x27DBE44, 0x27DBE64, 0x27DBE84, 0x27DBDE0, 0x27DBE00, 0x27DBE20, 0x27DBE40, 0x27DBE60, 0x27DBE80, 0x27DC7D4, 0x27DCD34, 0x27DCD54, 0x27DD014, 0x27DCFF4, 0x27DCE54, 0x27DCE74, 0x27DCE94, 0x27DCEB4, 0x27DCED4, 0x27DCEF4, 0x27DCF14, 0x27DCF34, 0x27DCD14, 0x27DCCF4, 0x27DC7F4, 0x27DCF54
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x10], [esp+0x110], [esp+0x130], [esp+0x150], [esp+0x170], [esp+0x190], [esp+0x30], [esp+0x50], [esp+0x70], [esp+0x90], [esp+0xb0], [esp+0xd0], [esp+0xf0]
- **x87 / SSE**: none.


**Record table** (decoded by symbolic execution of the stores; `dst` = `0x27DAE98 + idx*0x20`; fields are the 8 dwords copied by `rep movsd`; strings are resolved from the exe):

```
6B84B2 idx 134  text=TUT_SAVE_GAME title=TUT_TITLE_SAVE cb=0 f4=0
6B84E6 idx 135  text=0 title=TUT_TITLE_AUTOSAVE cb=0x6b7a90 f4=?
6B851E idx 136  text=TUT_HUD_NOTORIETY_GANG_1 title=TUT_TITLE_NOTORIETY_GANG cb=0 f4=0
6B8556 idx 137  text=TUT_HUD_NOTORIETY_POLICE_1 title=TUT_TITLE_NOTORIETY_POLICE cb=0 f4=0
6B8590 idx 138  text=TUT_STORE_INTRO_FNF title=TUT_TITLE_STORE_INTRO_FNF cb=0 f4=0
6B8613 idx 139  text=TUT_RECIEVE_CRIB title=TUT_TITLE_CRIB_RECEIVE cb=0 f4=0
6B864B idx 140  text=TUT_CRIB_CUSTOMIZE title=TUT_TITLE_CRIB_CUSTOMIZE cb=0 f4=0
6B8683 idx 141  text=TUT_CRIB_CARSTORAGE_1 title=TUT_TITLE_CRIB_GARAGE cb=0 f4=0
6B86BB idx 142  text=TUT_CRIB_CASH title=TUT_TITLE_CRIB_CASH cb=0 f4=0
6B86F3 idx 143  text=TUT_CRIB_CLOSET title=TUT_TITLE_CRIB_CLOSET cb=0 f4=0
6B872B idx 144  text=TUT_CRIB_WEAPONS title=TUT_TITLE_CRIB_WEAPONS cb=0 f4=0
6B87BD idx 145  text=TUT_CRIB_NEWSPAPER title=TUT_TITLE_CRIB_NEWSPAPER cb=0 f4=0
6B87F5 idx 146  text=TUT_CRIB_TV title=TUT_TITLE_CRIB_TV cb=0 f4=0
6B8829 idx 147  text=0 title=TUT_TITLE_COMBAT cb=0x6b7b30 f4=?
6B8861 idx 148  text=TUT_GENERAL_BUSTED title=TUT_TITLE_BUSTED cb=0 f4=0
6B8899 idx 149  text=TUT_GENERAL_SMOKED title=TUT_TITLE_SMOKED cb=0 f4=0
6B88CD idx 150  text=0 title=TUT_TITLE_HOMIE_REVIVE cb=0x6b7c00 f4=?
6B895B idx 151  text=0 title=TUT_TITLE_WAYPOINT cb=0x6b7c50 f4=?
6B89C3 idx 152  text=0 title=TUT_TITLE_EXPLORE cb=0x6b7cf0 f4=?
6B89D3 idx 153  text=TUT_PUSHBACK_1 title=TUT_TITLE_PUSHBACK cb=0 f4=0
6B8A0B idx 154  text=TUT_STRONGHOLD_START title=TUT_TITLE_STRONGHOLD_START cb=0 f4=0
6B8A43 idx 155  text=TUT_LC_STRONGHOLD title=TUT_TITLE_STRONGHOLD_RESPECT cb=0 f4=0
6B8AC6 idx 156  text=TUT_ACT_INTRO_CROWD_CONTROL title=TUT_TITLE_ACT_INTRO_CROWD_CONTROL cb=0 f4=0
6B8B0E idx 157  text=TUT_ACT_INTRO_DEMO_DERBY title=TUT_TITLE_ACT_INTRO_DEMO_DERBY cb=0 f4=0
6B8B42 idx 158  text=TUT_ACT_INTRO_DEMO_DERBY_SP title=TUT_TITLE_ACT_INTRO_DEMO_DERBY cb=0 f4=0
6B8B7A idx 159  text=TUT_ACT_INTRO_DRUG_TRAFFICKING title=TUT_TITLE_ACT_INTRO_DRUG_TRAFFICKING cb=0 f4=0
6B8BB2 idx 160  text=TUT_ACT_INTRO_ESCORT title=TUT_TITLE_ACT_INTRO_ESCORT cb=0 f4=0
6B8BEA idx 161  text=TUT_ACT_INTRO_FIGHT_CLUB title=TUT_TITLE_ACT_INTRO_FIGHT_CLUB cb=0 f4=0
6B8C6D idx 162  text=TUT_ACT_INTRO_FUZZ title=TUT_TITLE_ACT_INTRO_FUZZ cb=0 f4=0
6B8CA5 idx 163  text=TUT_ACT_INTRO_HELI_ASSAULT title=TUT_TITLE_ACT_INTRO_HELI_ASSAULT cb=0 f4=0
6B8CDD idx 164  text=TUT_ACT_INTRO_HUMAN_TORCH title=TUT_TITLE_ACT_INTRO_HUMAN_TORCH cb=0 f4=0
6B8D15 idx 165  text=TUT_ACT_INTRO_INS_FRAUD title=TUT_TITLE_ACT_INTRO_INS_FRAUD cb=0 f4=0
6B8D4D idx 166  text=TUT_ACT_INTRO_MAYHEM title=TUT_TITLE_ACT_INTRO_MAYHEM cb=0 f4=0
6B8D85 idx 167  text=TUT_ACT_INTRO_PIRACY title=TUT_TITLE_ACT_INTRO_PIRACY cb=0 f4=0
6B8E17 idx 168  text=TUT_ACT_INTRO_SEPTIC_TRUCK title=TUT_TITLE_ACT_INTRO_SEPTIC_TRUCK cb=0 f4=0
6B8E4F idx 169  text=TUT_ACT_INTRO_SNATCH title=TUT_TITLE_ACT_INTRO_SNATCH cb=0 f4=0
6B8E87 idx 170  text=TUT_DIV_INTRO_AMBULANCE title=TUT_TITLE_DIV_INTRO_AMBULANCE cb=0 f4=0
6B8EBF idx 171  text=TUT_DIV_INTRO_CHOP_SHOP title=TUT_TITLE_DIV_INTRO_CHOP_SHOP cb=0 f4=0
6B8EF7 idx 172  text=TUT_DIV_INTRO_DRIVE_BY title=TUT_TITLE_DIV_INTRO_DRIVE_BY cb=0 f4=0
6B8F2F idx 173  text=TUT_DIV_INTRO_FIRE_TRUCK title=TUT_TITLE_DIV_INTRO_FIRE_TRUCK cb=0 f4=0
6B8FC1 idx 174  text=TUT_FLASHING title=TUT_TITLE_FLASHING cb=0 f4=0
6B8FF9 idx 175  text=TUT_DIV_INTRO_HITMAN title=TUT_TITLE_DIV_INTRO_HITMAN cb=0 f4=0
6B9031 idx 176  text=TUT_ESCORT_HOING title=TUT_TITLE_DIV_INTRO_HOING cb=0 f4=0
6B9069 idx 177  text=TUT_DIV_INTRO_HOSTAGE title=TUT_TITLE_DIV_INTRO_HOSTAGE cb=0 f4=0
6B909D idx 178  text=0 title=TUT_TITLE_DIV_INTRO_MUGGING cb=0x6b81d0 f4=?
6B90D5 idx 179  text=TUT_DIV_INTRO_RACING title=TUT_TITLE_DIV_INTRO_RACING cb=0 f4=0
6B9167 idx 180  text=TUT_STREAKING title=TUT_TITLE_STREAKING cb=0 f4=0
6B919F idx 181  text=TUT_DIV_INTRO_STREAKING title=TUT_TITLE_DIV_INTRO_STREAKING cb=0 f4=0
6B91D7 idx 182  text=TUT_DIV_INTRO_TAGGING title=TUT_TITLE_DIV_INTRO_TAGGING cb=0 f4=0
6B920F idx 183  text=TUT_DIV_INTRO_TAXI title=TUT_TITLE_DIV_INTRO_TAXI cb=0 f4=0
6B9247 idx 184  text=TUT_DIV_INTRO_TOW_TRUCK title=TUT_TITLE_DIV_INTRO_TOW_TRUCK cb=0 f4=0
6B92CA idx 185  text=TUT_DIV_INTRO_BASE_JUMPING title=TUT_TITLE_DIV_INTRO_BASE_JUMPING cb=0 f4=0
6B930C idx 186  text=TUT_COLLECTION_CD title=TUT_TITLE_COLLECTION_CD cb=0 f4=0
6B9344 idx 187  text=TUT_COLLECTION_STUNT_JUMP title=TUT_TITLE_COLLECTION_STUNT_JUMP cb=0 f4=0
6B937C idx 188  text=TUT_STORE_INTRO_CLOTHES title=TUT_TITLE_STORE_INTRO_CLOTHES cb=0 f4=0
6B93B4 idx 189  text=TUT_STORE_INTRO_MECHANIC title=TUT_TITLE_STORE_INTRO_MECHANIC cb=0 f4=0
6B93EC idx 190  text=TUT_STORE_INTRO_WEAPONS title=TUT_TITLE_STORE_INTRO_WEAPONS cb=0 f4=0
6B9474 idx 191  text=TUT_STORE_INTRO_MELEE_WEAPONS title=TUT_TITLE_STORE_INTRO_MELEE_WEAPONS cb=0 f4=0
6B94AC idx 192  text=TUT_STORE_INTRO_LIQUOR title=TUT_TITLE_STORE_INTRO_LIQUOR cb=0 f4=0
6B94DC idx 193  text=TUT_STORE_INTRO_FNF title=TUT_TITLE_STORE_INTRO_FNF cb=0 f4=0
6B9514 idx 194  text=TUT_STORE_INTRO_MUSIC title=TUT_TITLE_STORE_INTRO_MUSIC cb=0 f4=0
6B954C idx 195  text=TUT_STORE_INTRO_SURGEON title=TUT_TITLE_STORE_INTRO_SURGEON cb=0 f4=0
6B9584 idx 196  text=TUT_STORE_INTRO_FOOD title=TUT_TITLE_STORE_INTRO_FOOD cb=0 f4=0
6B9616 idx 197  text=TUT_STORE_INTRO_TATTOO title=TUT_TITLE_STORE_INTRO_TATTOO cb=0 f4=0
6B964E idx 198  text=TUT_STORE_INTRO_JEWELRY title=TUT_TITLE_STORE_INTRO_JEWELRY cb=0 f4=0
6B9686 idx 199  text=TUT_STORE_INTRO_MECHANIC_LARGE title=TUT_TITLE_STORE_INTRO_MECHANIC_LARGE cb=0 f4=0
6B96BE idx 200  text=TUT_STORE_INTRO_VEHICLE_DEALER title=TUT_TITLE_STORE_INTRO_VEHICLE_DEALER cb=0 f4=0
6B96F6 idx 201  text=TUT_FINAL_ACT_LEVEL_COMPLETE title=TUT_TITLE_ACTIVITY_COMPLETE cb=0 f4=0
6B977B idx 202  text=TUT_DIVERSION_COMPLETE title=TUT_TITLE_DIVERSION_COMPLETE cb=0 f4=0
6B97BE idx 203  text=0 title=TUT_TITLE_HUMAN_SHIELD cb=0x6b7d50 f4=?
6B97F2 idx 204  text=0 title=TUT_TITLE_THROW_KILL cb=0x6b7d90 f4=?
6B9826 idx 205  text=0 title=TUT_TITLE_IMPROVISED_WEAPONS cb=0x6b7dd0 f4=?
6B985E idx 206  text=TUT_MINIMAP_ICONS title=TUT_TITLE_MINIMAP_ICONS cb=0 f4=0
6B9892 idx 207  text=0 title=TUT_TITLE_COMBAT_RANGED cb=0x6b7ad0 f4=?
6B9911 idx 208  text=0 title=TUT_TITLE_COMBAT_WEAPONS cb=0x6b7e10 f4=?
6B9958 idx 209  text=TUT_RN06_SWORDS title=TUT_TITLE_RN06_SWORDS cb=0 f4=0
6B998C idx 210  text=0 title=TUT_TITLE_COMBAT_FINEAIM cb=0x6b7e70 f4=?
6B99C0 idx 211  text=0 title=TUT_TITLE_JUMP_CLIMB cb=0x6b7eb0 f4=?
6B99F4 idx 212  text=0 title=TUT_TITLE_CROUCH cb=0x6b7ef0 f4=?
6B9A2C idx 213  text=TUT_DRIVING_STUNTS title=TUT_TITLE_DRIVING_STUNTS cb=0 f4=0
6B9A64 idx 214  text=TUT_SHOOTING_STUNTS title=TUT_TITLE_SHOOTING_STUNTS cb=0 f4=0
6B9AF6 idx 215  text=TUT_AMBULANCE_SHOCK_PADDLES title=TUT_TITLE_AMBULANCE_SHOCK_PADDLES cb=0 f4=0
6B9B2E idx 216  text=TUT_AMBULANCE_CPR title=TUT_TITLE_AMBULANCE_CPR cb=0 f4=0
6B9B62 idx 217  text=0 title=TUT_TITLE_CRUISE_CONTROL cb=0x6b7f50 f4=?
6B9B9A idx 218  text=TUT_HELICOPTER_ENTRY title=TUT_TITLE_HELICOPTER_CONTROL cb=0 f4=0
6B9BCE idx 219  text=0 title=TUT_TITLE_AIRPLANE_CONTROL cb=0x6b7fd0 f4=?
6B9C06 idx 220  text=TUT_DEMO_DERBY_COMPLETE title=TUT_TITLE_DEMO_DERBY_COMPLETE cb=0 f4=0
6B9C98 idx 221  text=TUT_MISSION_CHECKPOINT title=TUT_TITLE_MISSION_CHECKPOINT cb=0 f4=0
6B9CCC idx 222  text=0 title=TUT_TITLE_TAUNTING cb=0x6b8070 f4=?
6B9D04 idx 223  text=TUT_RESPECT_METER title=TUT_TITLE_RESPECT_METER cb=0 f4=0
6B9D3C idx 224  text=TUT_HEALTH_SPRINT_METER title=TUT_TITLE_HEALTH_SPRINT_METER cb=0 f4=0
6B9D70 idx 225  text=0 title=TUT_TITLE_PAUSE_MAP cb=0x6b80d0 f4=?
6B9DA4 idx 226  text=0 title=TUT_TITLE_MINIMAP cb=0x6b8110 f4=?
6B9E32 idx 227  text=0 title=TUT_TITLE_RECRUITING cb=0x6b8150 f4=?
6B9E6A idx 228  text=TUT_DIVERSION_HUD title=TUT_TITLE_DIVERSION_HUD cb=0 f4=0
6B9EA2 idx 229  text=TUT_HUD_TEXT title=TUT_TITLE_HUD_TEXT cb=0 f4=0
6B9ED6 idx 230  text=0 title=TUT_TITLE_UNLOCKABLES cb=0x6b8190 f4=?
6B9F0E idx 231  text=TUT_STYLE_RANK title=TUT_TITLE_STYLE_RANK cb=0 f4=0
6B9F46 idx 232  text=TUT_STORE_OWNERSHIP title=TUT_TITLE_STORE_OWNERSHIP cb=0 f4=0
6B9FD8 idx 233  text=TUT_GANG_CUSTOMIZE title=TUT_TITLE_GANG_CUSTOMIZE cb=0 f4=0
6BA010 idx 234  text=TUT_GANG_VEHICLE_CUSTOMIZE title=TUT_TITLE_GANG_VEHICLE_CUSTOMIZE cb=0 f4=0
6BA048 idx 235  text=TUT_SHORTCUT_SPLINES title=TUT_TITLE_SHORTCUT_SPLINES cb=0 f4=0
6BA07C idx 236  text=0 title=TUT_TITLE_SATCHEL_CHARGES cb=0x6b8210 f4=?
6BA0B0 idx 237  text=0 title=TUT_TITLE_AR50 cb=0x6b8250 f4=?
6BA133 idx 238  text=TUT_ANNIHILATOR title=TUT_TITLE_ANNIHILATOR cb=0 f4=0
6BA176 idx 239  text=0 title=TUT_TITLE_ESCORT_HOING cb=0x6b8290 f4=?
6BA1AF idx 240  text=TUT_FIGHT_CLUB_NECK_BREAKER title=TUT_TITLE_FIGHT_CLUB_NECK_BREAKER cb=0 f4=0
6BA1E8 idx 241  text=TUT_ACT_FIGHT_CLUB_FINISHER title=TUT_TITLE_FIGHT_CLUB_NECK_BREAKER cb=0 f4=0
6BA21A idx 242  text=0 title=TUT_TITLE_NONE cb=0x6b82c0 f4=?
6BA24C idx 243  text=0 title=TUT_TITLE_NONE cb=0x6b8330 f4=?
6BA2E3 idx 244  text=0 title=TUT_TITLE_NONE cb=0x6b83a0 f4=?
6BA349 idx 245  text=0 title=TUT_TITLE_NONE cb=0x6b8410 f4=?
6BA359 idx 246  text=TUT_ACT_INS_FRAUD_FIRST_HIT title=TUT_TITLE_ACT_INTRO_INS_FRAUD cb=0 f4=0
6BA391 idx 247  text=TUT_ACT_INS_FRAUD_ANDRENALINE title=TUT_TITLE_ACT_FRAUD_ADRENALINE cb=0 f4=0
6BA3C9 idx 248  text=TUT_ACT_CROWD_CONTROL_THROWING title=TUT_TITLE_ACT_INTRO_CROWD_CONTROL cb=0 f4=0
6BA401 idx 249  text=TUT_ACT_CROWD_CONTROL_LB title=TUT_TITLE_ACT_INTRO_CROWD_CONTROL cb=0 f4=0
6BA489 idx 250  text=TUT_DIV_INTRO_COOP_DEATH_TAG title=TUT_TITLE_DIV_INTRO_COOP_DEATH_TAG cb=0 f4=0
6BA4C1 idx 251  text=TUT_DIV_INTRO_COOP_CAT_AND_MOUSE title=TUT_TITLE_DIV_INTRO_COOP_CAT_AND_MOUSE cb=0 f4=0
6BA4F9 idx 252  text=TUT_ACT_INTRO_DEMO_DERBY_GARAGE title=TUT_TITLE_ACT_INTRO_DEMO_DERBY cb=0 f4=0
6BA531 idx 262  text=TUT_TERRITORY title=TUT_TITLE_EXPLORE cb=0 f4=0
6BA569 idx 263  text=TUT_TSS02_COMPLETE title=TUT_TITLE_RESPECT cb=0 f4=0
6BA5A1 idx 264  text=TUT_SPRINT title=TUT_TITLE_SPRINT cb=0 f4=0
6BA633 idx 265  text=TUT_HYDRAULICS title=TUT_TITLE_HYDRAULICS cb=0 f4=0
6BA686 idx 266  text=TUT_VEHICLE_ENTRY title=TUT_TITLE_VEHICLE_ENTRY, cb=0 f4=0
6BA6D9 idx 267  text=ACT_DEMO_DERBY_HOW_TO_PLAY_TEXT_2 title=TUT_TITLE_NONE cb=0 f4=0
6BA72C idx 253  text=MULTI_TIP_TAGGING title=HELP_TEXT_TAGGING_TITLE cb=0 f4=0
6BA77F idx 254  text=MULTI_TIP_DEMO_DERBY title=MULTI_ACTIVITY_DERBY cb=0 f4=0
6BA81D idx 255  text=MULTI_TIP_HITMAN title=MULTI_ACTIVITY_HITMAN cb=0 f4=0
6BA889 idx 256  text=MULTI_TIP_FRAUD title=MULTI_ACTIVITY_FRAUD cb=0 f4=0
6BA8C1 idx 257  text=MULTI_TIP_MAYHEM title=MULTI_ACTIVITY_MAYHEM cb=0 f4=0
6BA905 idx 258  text=MULTI_TIP_RACING title=0xdffef0 cb=0 f4=0
6BA958 idx 259  text=MULTI_TIP_SNATCH title=MULTI_ACTIVITY_SNATCH cb=0 f4=0
6BA9AB idx 260  text=MULTI_TIP_THEFT title=MULTI_ACTIVITY_THEFT cb=0 f4=0
6BAA33 idx 261  text=MULTI_TIP_BRAWL title=MP_ACT_BRAWL cb=0 f4=0
```

The first 134 records get only `[0x27DAEA0 + 0x20*i] = a` and `[0x27DAEA4 + 0x20*i] = b` (285 stores).

## Map: `stat_award_table_init` (0x6B3BB0, 0xFDC bytes)

Calls `stats_xtbl_load` (`0x6B2C30`) and then stores 69 rows x 6 callbacks at `0x27DA820 + row*0x18`. Port as a static table; the callback addresses are the award condition functions (`stat_award_check` calls them by type).

### stat_award_table_init (0x6B3BB0)

- **True size** 0xFDC (4060) bytes by recursive descent; 754 instructions; ret imm [0]; entry regs read (linear prefix) none.
- **Callees** (1 call sites, 1 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0x6B2C30 |  | 6B3BB3 | ret [0], reads [] |
- **Globals read**: none
- **Globals written**: 0x27DA820, 0x27DA824, 0x27DA828, 0x27DA82C, 0x27DA830, 0x27DA834, 0x27DA838, 0x27DA83C, 0x27DA840, 0x27DA844, 0x27DA848, 0x27DA84C, 0x27DA850, 0x27DA854, 0x27DA858, 0x27DA85C, 0x27DA860, 0x27DA864, 0x27DA868, 0x27DA86C, 0x27DA870, 0x27DA874, 0x27DA878, 0x27DA87C, 0x27DA880, 0x27DA884, 0x27DA888, 0x27DA88C, 0x27DA890, 0x27DA894, 0x27DA898, 0x27DA89C, 0x27DA8A0, 0x27DA8A4, 0x27DA8A8, 0x27DA8AC, 0x27DA8B0, 0x27DA8B4, 0x27DA8B8, 0x27DA8BC, 0x27DA8C0, 0x27DA8C8, 0x27DA8CC, 0x27DA8D0, 0x27DA8D4, 0x27DA8C4, 0x27DA8D8, 0x27DA8E0, 0x27DA8E4, 0x27DA8E8, 0x27DA8EC, 0x27DA8DC, 0x27DA8F8, 0x27DA8FC, 0x27DA900, 0x27DA904, 0x27DA8F0, 0x27DA8F4, 0x27DA910, 0x27DA914, 0x27DA918, 0x27DA91C, 0x27DA908, 0x27DA90C, 0x27DA928, 0x27DA92C, 0x27DA930, 0x27DA934, 0x27DA920, 0x27DA924, 0x27DA940, 0x27DA944, 0x27DA948, 0x27DA94C, 0x27DA938, 0x27DA93C, 0x27DA950, 0x27DA954, 0x27DA958, 0x27DA95C, 0x27DA960, 0x27DA964, 0x27DA970, 0x27DA974, 0x27DA978, 0x27DA97C, 0x27DA988, 0x27DA98C, 0x27DA990, 0x27DA994, 0x27DA9A0, 0x27DA9A4, 0x27DA9A8, 0x27DA9AC, 0x27DA9B8, 0x27DA9BC, 0x27DA9C0, 0x27DA9C4, 0x27DA9D0, 0x27DA9D4, 0x27DA9D8, 0x27DA9DC, 0x27DA9E8, 0x27DA9EC, 0x27DA9F0, 0x27DA9F4, 0x27DAA00, 0x27DAA04, 0x27DAA08, 0x27DAA0C, 0x27DA968, 0x27DA96C, 0x27DA980, 0x27DA984, 0x27DA998, 0x27DA99C, 0x27DA9B0, 0x27DA9B4, 0x27DA9C8, 0x27DA9CC, 0x27DA9E0, 0x27DA9E4, 0x27DA9F8, 0x27DA9FC, 0x27DAA10, 0x27DAA14, 0x27DAA18, 0x27DAA1C, 0x27DAA20, 0x27DAA24, 0x27DAA30, 0x27DAA34, 0x27DAA38, 0x27DAA3C, 0x27DAA48, 0x27DAA4C, 0x27DAA50, 0x27DAA54, 0x27DAA60, 0x27DAA64, 0x27DAA68, 0x27DAA6C, 0x27DAA78, 0x27DAA7C, 0x27DAA80, 0x27DAA84, 0x27DAA90, 0x27DAA94, 0x27DAA98, 0x27DAA9C, 0x27DAAA8, 0x27DAAAC, 0x27DAAB0, 0x27DAAB4, 0x27DAAC0, 0x27DAAC4, 0x27DAAC8, 0x27DAACC, 0x27DAA28, 0x27DAA2C, 0x27DAA40, 0x27DAA44, 0x27DAA58, 0x27DAA5C, 0x27DAA70, 0x27DAA74, 0x27DAA88, 0x27DAA8C, 0x27DAAA0, 0x27DAAA4, 0x27DAAB8, 0x27DAABC, 0x27DAAD0, 0x27DAAD4, 0x27DAAD8, 0x27DAADC, 0x27DAAE0, 0x27DAAE4, 0x27DAAF0, 0x27DAAF4, 0x27DAAF8, 0x27DAAFC, 0x27DAC58, 0x27DAC5C, 0x27DAC60, 0x27DAC64, 0x27DAC70, 0x27DAC74, 0x27DAC78, 0x27DAC7C, 0x27DAB38, 0x27DAAE8, 0x27DAB00, 0x27DAAEC, 0x27DAB04, 0x27DAB3C, 0x27DAB40, 0x27DAB44, 0x27DAB50, 0x27DAC68, 0x27DAC6C, 0x27DAB54, 0x27DAB58, 0x27DAB5C, 0x27DAB68, 0x27DAC80, 0x27DAC84, 0x27DAB6C, 0x27DAB70, 0x27DAB74, 0x27DAB80, 0x27DAB48, 0x27DAB4C, 0x27DAB60, 0x27DAB64, 0x27DAB78, 0x27DAB7C, 0x27DAB84, 0x27DAB88, 0x27DAB8C, 0x27DAB90, 0x27DAB94, 0x27DAB98, 0x27DAB9C, 0x27DABA0, 0x27DABA4, 0x27DABB0, 0x27DABB4, 0x27DABB8, 0x27DABBC, 0x27DABC8, 0x27DABCC, 0x27DABD0, 0x27DABD4, 0x27DABE0, 0x27DABE4, 0x27DABE8, 0x27DABEC, 0x27DABF8, 0x27DABFC, 0x27DAC00, 0x27DAC04, 0x27DAC28, 0x27DAC2C, 0x27DAC30, 0x27DAC34, 0x27DAC40, 0x27DAC44, 0x27DAC48, 0x27DAC4C, 0x27DAC10, 0x27DAC14, 0x27DABA8, 0x27DABAC, 0x27DABC0, 0x27DABC4, 0x27DABD8, 0x27DABDC, 0x27DABF0, 0x27DABF4, 0x27DAC08, 0x27DAC0C, 0x27DAC38, 0x27DAC3C, 0x27DAC50, 0x27DAC54, 0x27DAC18, 0x27DAC1C, 0x27DAC20, 0x27DAC24, 0x27DAB20, 0x27DAB24, 0x27DAB28, 0x27DAB2C, 0x27DAB08, 0x27DAB0C, 0x27DAB10, 0x27DAB14, 0x27DAD18, 0x27DAD1C, 0x27DAD20, 0x27DAD24, 0x27DAC88, 0x27DAC8C, 0x27DAC90, 0x27DAC94, 0x27DACA0, 0x27DACA4, 0x27DACA8, 0x27DACAC, 0x27DAB30, 0x27DAB18, 0x27DAD28, 0x27DAC98, 0x27DACB0, 0x27DACB8, 0x27DACBC, 0x27DACC0, 0x27DACC4, 0x27DACC8, 0x27DAB34, 0x27DAB1C, 0x27DAD2C, 0x27DAC9C, 0x27DACB4, 0x27DACCC, 0x27DACD0, 0x27DACD4, 0x27DACD8, 0x27DACDC, 0x27DACE0, 0x27DACE4, 0x27DACE8, 0x27DACEC, 0x27DACF0, 0x27DACF4, 0x27DACF8, 0x27DACFC, 0x27DAD00, 0x27DAD04, 0x27DAD08, 0x27DAD0C, 0x27DAD10, 0x27DAD14, 0x27DAD30, 0x27DAD34, 0x27DAD38, 0x27DAD3C, 0x27DAD40, 0x27DAD44, 0x27DAD48, 0x27DAD4C, 0x27DAD50, 0x27DAD54, 0x27DAD58, 0x27DAD5C, 0x27DAD60, 0x27DAD64, 0x27DAD68, 0x27DAD6C, 0x27DAD70, 0x27DAD74, 0x27DAD78, 0x27DAD7C, 0x27DAD80, 0x27DAD84, 0x27DAD88, 0x27DAD8C, 0x27DAD90, 0x27DAD94, 0x27DAD98, 0x27DAD9C, 0x27DADA0, 0x27DADA4, 0x27DADA8, 0x27DADAC, 0x27DADB0, 0x27DADB4, 0x27DADC0, 0x27DADC4, 0x27DADC8, 0x27DADCC, 0x27DADD8, 0x27DADDC, 0x27DADE0, 0x27DADE4, 0x27DADF0, 0x27DADF4, 0x27DADF8, 0x27DADFC, 0x27DAE08, 0x27DADB8, 0x27DADBC, 0x27DAE0C, 0x27DAE10, 0x27DAE14, 0x27DAE20, 0x27DADD0, 0x27DADD4, 0x27DAE24, 0x27DAE28, 0x27DAE2C, 0x27DAE38, 0x27DADE8, 0x27DADEC, 0x27DAE3C, 0x27DAE40, 0x27DAE44, 0x27DAE50, 0x27DAE00, 0x27DAE04, 0x27DAE18, 0x27DAE1C, 0x27DAE30, 0x27DAE34, 0x27DAE48, 0x27DAE4C, 0x27DAE54, 0x27DAE58, 0x27DAE5C, 0x27DAE60, 0x27DAE64, 0x27DAE78, 0x27DAE74, 0x27DAE7C, 0x27DAE90, 0x27DAE68, 0x27DAE6C, 0x27DAE70, 0x27DAE8C, 0x27DAE94, 0x27DAE80, 0x27DAE84, 0x27DAE88
- **Stack objects (lea of a frame slot, passed by address)**: none
- **x87 / SSE**: none.


**Callback table** (row = award type, six callbacks per row at `0x27DA820 + row*0x18`):

```
row  0: 6B0A20 6B1360 6B1700 6B25C0 6B1620 6B2AB0
row  1: 6B0A30 6B13B0 6B1730 6B25D0 6B1640 6B2AE0
row  2: 6B0A30 6B13B0 6B1760 6B25D0 6B1640 6B2AE0
row  3: 6B0A50 6B1400 6B17D0 6B25E0 6B1660 6B2B10
row  4: 6B0A30 6B13B0 6B1930 6B25D0 6B1640 6B2AE0
row  5: 6B0A70 6B1450 6B1870 6B25F0 6B1680 6B2B40
row  6: 6B0A80 6B1480 6B18B0 6B2600 6B16A0 6B2B70
row  7: 6B0A90 6B1360 6B1960 6B2610 0 0
row  8: 6B0AA0 6B1500 6B19A0 6B2620 6B1660 6B2B10
row  9: 6B0AA0 6B1500 6B19A0 6B2620 6B1660 6B2B10
row 10: 6B0B30 6B1400 6B17D0 6B25E0 6B1660 6B2B10
row 11: 6B0E00 6B1360 6B1FA0 6B28E0 0 0
row 12: 6B0E30 6B1360 6B1700 6B2910 0 0
row 13: 6B0E60 6B1360 6B1700 6B2940 0 0
row 14: 6B0E90 6B1360 6B1700 6B2970 0 0
row 15: 6B0F00 6B1360 6B1700 6B2980 0 0
row 16: 6B0F70 6B1360 6B1700 6B2990 0 0
row 17: 6B0C70 6B1360 6B1AA0 6B2680 0 0
row 18: 6B0C60 6B1360 6B1AA0 6B2670 0 0
row 19: 6B11E0 6B1360 6B1AF0 6B26E0 0 0
row 20: 6B12E0 6B1360 6B1C70 6B27E0 0 0
row 21: 6B0FE0 6B1360 6B1A60 6B26C0 0 0
row 22: 6B0C90 6B1360 6B1D20 6B26A0 0 0
row 23: 6B0CE0 6B1360 6B1D60 6B26B0 0 0
row 24: 6B0BE0 6B1360 6B1CE0 6B2650 0 0
row 25: 6B0C20 6B1360 6B1DC0 6B2660 0 0
row 26: 6B0D40 6B1590 6B1E20 6B2820 0 0
row 27: 6B0D70 6B15B0 6B1E70 6B2850 0 0
row 28: 6B1320 6B1360 6B1EC0 6B2AA0 0 0
row 29: 6B0DA0 6B1360 6B1F00 6B2880 0 0
row 30: 6B0DD0 6B1360 6B1F50 6B28B0 0 0
row 31: 6B1010 6B1360 6B1FD0 6B29A0 0 0
row 32: 6B1000 6B1360 6B2090 6B29D0 0 0
row 33: 6B1180 6B1360 6B2250 6B2A40 0 0
row 34: 6B1170 6B1360 6B2210 6B2A30 0 0
row 35: 6B1160 6B1360 6B21D0 6B2A20 0 0
row 36: 6B1130 6B1360 6B2110 6B29F0 0 0
row 37: 6B1190 6B1360 6B2290 6B2A50 0 0
row 38: 6B11A0 6B1360 6B22D0 6B2A60 0 0
row 39: 6B11B0 6B1360 6B2310 6B2A70 0 0
row 40: 6B11C0 6B1360 6B2350 6B2A80 0 0
row 41: 6B1140 6B1360 6B2150 6B2A00 0 0
row 42: 6B1120 6B1360 6B20D0 6B29E0 0 0
row 43: 6B11D0 6B1360 6B2390 6B2A90 0 0
row 44: 6B1150 6B1360 6B2190 6B2A10 0 0
row 45: 6B0C80 6B1560 6B1A40 6B2690 6B16E0 6B2BD0
row 46: 6B0BB0 6B1550 6B19B0 6B2630 6B16C0 6B2BA0
row 47: 6B11E0 6B1360 6B1AF0 6B26E0 0 0
row 48: 6B12E0 6B1360 6B1C70 6B27E0 0 0
row 49: 6B0FE0 6B1360 6B1A60 6B26C0 0 0
row 50: 6B0AA0 6B1500 6B19A0 6B2620 6B1660 6B2B10
row 51: 6B0AA0 6B1500 6B19A0 6B2620 6B1660 6B2B10
row 52: 6B0B30 6B1400 6B17D0 6B25E0 6B1660 6B2B10
row 53: 6B1030 6B1360 6B2010 6B29C0 0 0
row 54: 6B0A20 6B1360 6B23D0 6B25C0 6B1620 6B2AB0
row 55: 6B0A20 6B1360 6B2410 6B25C0 6B1620 6B2AB0
row 56: 6B0A20 6B1360 6B2450 6B25C0 6B1620 6B2AB0
row 57: 6B0A20 6B1360 6B2490 6B25C0 6B1620 6B2AB0
row 58: 6B0A20 6B1360 6B24D0 6B25C0 6B1620 6B2AB0
row 59: 6B0A20 6B1360 6B2510 6B25C0 6B1620 6B2AB0
row 60: 6B0A20 6B1360 6B2550 6B25C0 6B1620 6B2AB0
row 61: 6B0A20 6B1360 6B2590 6B25C0 6B1620 6B2AB0
row 62: 6B1220 6B1360 6B1B50 6B2720 0 0
row 63: 6B1260 6B1360 6B1BB0 6B2760 0 0
row 64: 6B12A0 6B1360 6B1C10 6B27A0 0 0
row 65: 6B1220 6B1360 6B1B50 6B2720 0 0
row 66: 6B1260 6B1360 6B1BB0 6B2760 0 0
row 67: 6B12A0 6B1360 6B1C10 6B27A0 0 0
row 68: 819300 6B15D0 0 819310 6B1620 6B2AB0
```

## Map: `pcr_apply_preset_impl` (0x7D7180, 0xF2D bytes)

Applies a player-creation preset. Two 4-case jump tables, `sprintf` calls to build slot labels, `memmove` of worn entries, network update (`net_send_human_pkt_5`). Frame with a security cookie.

### pcr_apply_preset_impl (0x7D7180)

- **True size** 0xF2D (3885) bytes by recursive descent; 1174 instructions; ret imm [0]; entry regs read (linear prefix) ['ecx'].
- Frame: esp aligned (and esp,-8) with ebp frame; frame sub esp,0x3c; security cookie [0xE80B50].
- **Callees** (38 call sites, 19 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0x9EDCD0 |  | 7D71B2 | ret [0], reads ['esi'] |
| 0x7D8EA0 |  | 7D71D1 | ret [0], reads ['eax'] |
| 0x7DAE30 |  | 7D71FC 7D7223 | ret [0], reads [] |
| 0xC968F6 | _sprintf | 7D730A 7D767E 7D7A72 7D7D7E | ret [0], reads [] |
| 0x7C20B0 |  | 7D792E | ret [0], reads [] |
| 0x7D8440 |  | 7D7943 | ret [0], reads ['edi'] |
| 0x7C1D40 |  | 7D7D24 | ret [0], reads ['ecx'] |
| 0x7D8150 |  | 7D7FF8 | ret [0], reads ['eax'] |
| 0x95CDB0 |  | 7D8003 | ret [4], reads ['ecx', 'edx'] |
| 0x7C0C30 |  | 7D801D | ret [0], reads [] |
| 0x96F940 | net_send_human_pkt_5 | 7D8083 | ret [0], reads [] |
| 0x9E8DC0 |  | 7D8093 | ret [0], reads [] |
| 0xC9604F | __security_check_cookie | 7D80A4 | ret [], reads ['ecx'] |
| 0x9E8ED0 |  | 7D7FA0 7D7C3A 7D7C5F 7D7869 7D74FA 7D7532 | ret [0], reads [] |
| 0x7BEE40 |  | 7D7FAA 7D7C6D 7D7873 7D7540 | ret [0], reads ['eax'] |
| 0x7BF550 |  | 7D7E55 7D7C78 7D787E 7D754B | ret [0], reads [] |
| 0xC96060 | _memmove | 7D7EB7 7D7B69 7D776E 7D7428 | ret [0], reads [] |
| 0x7D8910 |  | 7D78D2 7D7579 | ret [0], reads ['edi'] |
| 0x7D5570 |  | 7D7561 | ret [0], reads ['edi'] |
- **Globals read**: 0x21703D4, 0x25288BCx4, 0x252890Cx2, 0x25289C8
- **Globals written**: 0x25289C8
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x0], [esp+0x3c]
- **Jump tables** (decoded from the exe bytes):
  - `jmp [idx*4+0x7D80B0]` at 0x7D7E82 (4 entries): 0:7D7FB4 1:7D7FB4 2:7D7FBB 3:7D7FC2
  - `jmp [idx*4+0x7D80C0]` at 0x7D7FCE (4 entries): 0:7D7FE0 1:7D7FE0 2:7D7FD5 3:7D7FDC
- **x87 sequence** (4 instructions; `fstp/fst dword` = f32 rounding point):

```
7D71C3 fld dword ptr [eax + esi*8 + 4]
7D71CD fstp dword ptr [esp]   <== f32 store
7D7554 fld dword ptr [esi + 0x20]
7D755E fstp dword ptr [esp]   <== f32 store
```

**Section-by-section walk**

```
B7D7180 (0x7D7180..0x7D71AC):
    IF test edi,edi ; je taken if edi == 0  -> 0x7D809B
B7D71AC (0x7D71AC..0x7D71BE):
    CALL 0x9EDCD0   stack(push order)=[]  regs{esi=mov dword ptr [ebx + 0x13b0]}
    IF cmp dword ptr [edi + 0x10], esi ; jle taken if dword ptr [edi + 0x10] <= esi  (signed less-or-equal)  -> 0x7D71E1
B7D71C0 (0x7D71C0..0x7D71E1):
    f32 store dword ptr [esp]
    CALL 0x7D8EA0   stack(push order)=[ebx, ecx]  regs{eax=mov dword ptr [eax]}
    IF cmp esi, dword ptr [edi + 0x10] ; jl taken if esi < dword ptr [edi + 0x10]  (signed less)  -> 0x7D71C0
B7D71E1 (0x7D71E1..0x7D71E8):
    IF cmp dword ptr [edi + 0x18], esi ; jle taken if dword ptr [edi + 0x18] <= esi  (signed less-or-equal)  -> 0x7D720C
B7D71E8 (0x7D71E8..0x7D71EA):
    JMP 0x7d71f0
B7D71F0 (0x7D71F0..0x7D720C):
    CALL 0x7DAE30   stack(push order)=[ebx]  regs{eax=mov dword ptr [eax] ecx=mov dword ptr [eax + 4]}
    IF cmp esi, dword ptr [edi + 0x18] ; jl taken if esi < dword ptr [edi + 0x18]  (signed less)  -> 0x7D71F0
B7D720C (0x7D720C..0x7D7214):
    IF cmp eax, -1 ; je taken if eax == -1  (equal / zero)  -> 0x7D722B
B7D7214 (0x7D7214..0x7D721C):
    IF cmp ecx, -1 ; je taken if ecx == -1  (equal / zero)  -> 0x7D722B
B7D721C (0x7D721C..0x7D722B):
    CALL 0x7DAE30   stack(push order)=[edx]  regs{edx=mov dword ptr [0x21703d4]}
B7D722B (0x7D722B..0x7D7244):
    IF cmp eax, 1 ; jl taken if eax < 1  (signed less)  -> 0x7D809B
B7D7244 (0x7D7244..0x7D724E):
    IF cmp dword ptr [ecx + 0x2c], 1 ; jl taken if dword ptr [ecx + 0x2c] < 1  (signed less)  -> 0x7D809B
B7D724E (0x7D724E..0x7D725C):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7D7583
B7D7260 (0x7D7260..0x7D7281):
    IF test edx,edx ; je taken if edx == 0  -> 0x7D7316
B7D7281 (0x7D7281..0x7D7292):
    IF test esi,esi ; jle taken if esi <= 0 (signed)  -> 0x7D72BD
B7D7295 (0x7D7295..0x7D729F):
    IF cmp ecx, dword ptr [0x25288bc] ; jge taken if ecx >= dword ptr [0x25288bc]  (signed greater-or-equal)  -> 0x7D72B3
B7D729F (0x7D729F..0x7D72B3):
    IF cmp byte ptr [ecx + ebx], 0 ; je taken if byte ptr [ecx + ebx] == 0  (equal / zero)  -> 0x7D7316
B7D72B3 (0x7D72B3..0x7D72BD):
    IF cmp eax, esi ; jl taken if eax < esi  (signed less)  -> 0x7D7295
B7D72E0 (0x7D72E0..0x7D72E4):
    IF cmp dword ptr [ebx], eax ; je taken if dword ptr [ebx] == eax  (equal / zero)  -> 0x7D72F4
B7D72E4 (0x7D72E4..0x7D72F2):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7D72E0
B7D72F2 (0x7D72F2..0x7D72F4):
    JMP 0x7d72f8
B7D72F4 (0x7D72F4..0x7D72F8):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7D7337
B7D72F8 (0x7D72F8..0x7D7316):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', eax, edx]  regs{eax=mov dword ptr [esi + 8] edx=mov dword ptr [edx + 0xc]}
B7D7316 (0x7D7316..0x7D7332):
    IF cmp eax, dword ptr [ecx + 0x24] ; jl taken if eax < dword ptr [ecx + 0x24]  (signed less)  -> 0x7D7260
B7D7332 (0x7D7332..0x7D7337):
    JMP 0x7d7583
B7D7337 (0x7D7337..0x7D7354):
    IF cmp dword ptr [edi*4 + 0x1fac828], eax ; jle taken if dword ptr [edi*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7D737A
B7D7360 (0x7D7360..0x7D7367):
    IF cmp byte ptr [edx + 0x15], cl ; je taken if byte ptr [edx + 0x15] == cl  (equal / zero)  -> 0x7D73D1
B7D7367 (0x7D7367..0x7D7376):
    IF cmp eax, dword ptr [edi*4 + 0x1fac828] ; jl taken if eax < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D7360
B7D737A (0x7D737A..0x7D7389):
    IF cmp dword ptr [edi*4 + 0x1fac828], ecx ; jle taken if dword ptr [edi*4 + 0x1fac828] <= ecx  (signed less-or-equal)  -> 0x7D7454
B7D7390 (0x7D7390..0x7D7396):
    IF test esi,esi ; je taken if esi == 0  -> 0x7D73B5
B7D7396 (0x7D7396..0x7D73A2):
    IF test dl,dl ; je taken if dl == 0  -> 0x7D73B5
B7D73A2 (0x7D73A2..0x7D73AD):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7D73B5
B7D73AD (0x7D73AD..0x7D73B5):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7D73FC
B7D73B5 (0x7D73B5..0x7D73CC):
    IF cmp ecx, dword ptr [edi*4 + 0x1fac828] ; jl taken if ecx < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D7390
B7D73CC (0x7D73CC..0x7D73D1):
    JMP 0x7d7454
B7D73D1 (0x7D73D1..0x7D73EE):
    IF cmp dword ptr [esi], edi ; jne taken if dword ptr [esi] != edi  (not equal / non-zero)  -> 0x7D747C
B7D73EE (0x7D73EE..0x7D73F7):
    IF cmp dword ptr [esi + 0x10], ebx ; je taken if dword ptr [esi + 0x10] == ebx  (equal / zero)  -> 0x7D7545
B7D73F7 (0x7D73F7..0x7D73FC):
    JMP 0x7d747c
B7D73FC (0x7D73FC..0x7D740E):
    IF cmp ecx, eax ; jge taken if ecx >= eax  (signed greater-or-equal)  -> 0x7D744C
B7D7421 (0x7D7421..0x7D744C):
    CALL 0xC96060 _memmove  stack(push order)=[esi, eax, 0x50]  regs{eax=lea [esi + 0x50]}
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7D7421
B7D747C (0x7D747C..0x7D74AC):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7D74B1
B7D74B1 (0x7D74B1..0x7D74B8):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7D74D2
B7D74C0 (0x7D74C0..0x7D74D2):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7D74C0
B7D74D2 (0x7D74D2..0x7D74DD):
    IF test al,al ; je taken if al == 0  -> 0x7D7501
B7D74DD (0x7D74DD..0x7D7501):
    CALL 0x9E8ED0   stack(push order)=[edi, ebx, 2, 0, 0x7bedb0]  regs{edx=mov dword ptr [edi + 0x10] edi=mov dword ptr [esp + 0x10]}
    JMP 0x7d753b
B7D7501 (0x7D7501..0x7D750A):
    IF test eax,eax ; jne taken if eax != 0  -> 0x7D7523
B7D750A (0x7D750A..0x7D751B):
    IF cmp eax, 4 ; jbe taken if eax <= 4  (unsigned below-or-equal)  -> 0x7D7523
B7D7523 (0x7D7523..0x7D753B):
    CALL 0x9E8ED0   stack(push order)=[edx, ebx, 2, 0, ecx]  regs{edx=mov dword ptr [esp + 0x10] ecx=mov dword ptr [esp + 0x30]}
B7D753B (0x7D753B..0x7D7545):
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov edi}
B7D7545 (0x7D7545..0x7D7583):
    CALL 0x7BF550   stack(push order)=[eax, esi]  regs{eax=mov dword ptr [esp + 0x10]}
    f32 store dword ptr [esp]
    CALL 0x7D5570   stack(push order)=[]  regs{edi=mov dword ptr [esp + 0x40] esi=mov dword ptr [esp + 0x34]}
    CALL 0x7D8910   stack(push order)=[0, edx, ecx]  regs{edi=mov dword ptr [edi + 0x13b0] edx=mov dword ptr [esi + 0x24] ecx=mov dword ptr [esi + 0x28]}
B7D7583 (0x7D7583..0x7D75AE):
    IF jle (flags from before the block)  -> 0x7D7908
B7D75C0 (0x7D75C0..0x7D75E1):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7D78EB
B7D75E1 (0x7D75E1..0x7D75F4):
    IF test esi,esi ; jle taken if esi <= 0 (signed)  -> 0x7D762C
B7D75F4 (0x7D75F4..0x7D75F9):
    JMP 0x7d7600
B7D7600 (0x7D7600..0x7D760A):
    IF cmp ecx, dword ptr [0x25288bc] ; jge taken if ecx >= dword ptr [0x25288bc]  (signed greater-or-equal)  -> 0x7D7622
B7D760A (0x7D760A..0x7D7622):
    IF cmp byte ptr [ecx + edi], 0 ; je taken if byte ptr [ecx + edi] == 0  (equal / zero)  -> 0x7D78EB
B7D7622 (0x7D7622..0x7D762C):
    IF cmp eax, esi ; jl taken if eax < esi  (signed less)  -> 0x7D7600
B7D762C (0x7D762C..0x7D764D):
    JMP 0x7d7650
B7D7650 (0x7D7650..0x7D7654):
    IF cmp dword ptr [ebx], eax ; je taken if dword ptr [ebx] == eax  (equal / zero)  -> 0x7D7664
B7D7654 (0x7D7654..0x7D7662):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7D7650
B7D7662 (0x7D7662..0x7D7664):
    JMP 0x7d766c
B7D7664 (0x7D7664..0x7D766C):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7D768B
B7D766C (0x7D766C..0x7D768B):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', edx, ecx]  regs{edx=mov dword ptr [esi + 8] ecx=mov dword ptr [edx + 0xc]}
    JMP 0x7d78eb
B7D768B (0x7D768B..0x7D76A8):
    IF cmp dword ptr [edi*4 + 0x1fac828], eax ; jle taken if dword ptr [edi*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7D76CA
B7D76B0 (0x7D76B0..0x7D76B7):
    IF cmp byte ptr [esi + 0x15], dl ; je taken if byte ptr [esi + 0x15] == dl  (equal / zero)  -> 0x7D771C
B7D76B7 (0x7D76B7..0x7D76C6):
    IF cmp eax, dword ptr [edi*4 + 0x1fac828] ; jl taken if eax < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D76B0
B7D76CA (0x7D76CA..0x7D76D9):
    IF cmp dword ptr [edi*4 + 0x1fac828], esi ; jle taken if dword ptr [edi*4 + 0x1fac828] <= esi  (signed less-or-equal)  -> 0x7D7796
B7D76E0 (0x7D76E0..0x7D76EA):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7D7709
B7D76EA (0x7D76EA..0x7D76F6):
    IF test dl,dl ; je taken if dl == 0  -> 0x7D7709
B7D76F6 (0x7D76F6..0x7D7701):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7D7709
B7D7701 (0x7D7701..0x7D7709):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7D7744
B7D7709 (0x7D7709..0x7D771A):
    IF cmp esi, dword ptr [edi*4 + 0x1fac828] ; jl taken if esi < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D76E0
B7D771A (0x7D771A..0x7D771C):
    JMP 0x7d7796
B7D771C (0x7D771C..0x7D7739):
    IF cmp ecx, dword ptr [esp + 0x30] ; jne taken if ecx != dword ptr [esp + 0x30]  (not equal / non-zero)  -> 0x7D77BA
B7D7739 (0x7D7739..0x7D7742):
    IF cmp dword ptr [esi + 0x10], ebx ; je taken if dword ptr [esi + 0x10] == ebx  (equal / zero)  -> 0x7D7878
B7D7742 (0x7D7742..0x7D7744):
    JMP 0x7d77ba
B7D7744 (0x7D7744..0x7D7754):
    IF cmp esi, eax ; jge taken if esi >= eax  (signed greater-or-equal)  -> 0x7D778A
B7D7767 (0x7D7767..0x7D778A):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp ebx, eax ; jl taken if ebx < eax  (signed less)  -> 0x7D7767
B7D77BA (0x7D77BA..0x7D77EE):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7D77F3
B7D77F3 (0x7D77F3..0x7D77FA):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7D7816
B7D7804 (0x7D7804..0x7D7816):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7D7804
B7D7816 (0x7D7816..0x7D7821):
    IF test al,al ; je taken if al == 0  -> 0x7D783A
B7D7821 (0x7D7821..0x7D783A):
    JMP 0x7d7863
B7D783A (0x7D783A..0x7D7843):
    IF test eax,eax ; jne taken if eax != 0  -> 0x7D785A
B7D7843 (0x7D7843..0x7D784C):
    IF cmp eax, 4 ; ja taken if eax > 4  (unsigned above)  -> 0x7D7853
B7D784C (0x7D784C..0x7D7853):
    JMP 0x7d785e
B7D7853 (0x7D7853..0x7D785A):
    JMP 0x7d785e
B7D7863 (0x7D7863..0x7D7878):
    CALL 0x9E8ED0   stack(push order)=[edi, ebx, 2, 0]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov edi}
B7D7878 (0x7D7878..0x7D78A0):
    CALL 0x7BF550   stack(push order)=[ecx, esi]  regs{ecx=mov dword ptr [esp + 0x10]}
B7D78A0 (0x7D78A0..0x7D78A4):
    IF cmp dword ptr [esi], eax ; je taken if dword ptr [esi] == eax  (equal / zero)  -> 0x7D78B4
B7D78A4 (0x7D78A4..0x7D78B2):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7D78A0
B7D78B2 (0x7D78B2..0x7D78B4):
    JMP 0x7d78eb
B7D78B4 (0x7D78B4..0x7D78B8):
    IF test esi,esi ; je taken if esi == 0  -> 0x7D78EB
B7D78B8 (0x7D78B8..0x7D78E6):
    CALL 0x7D8910   stack(push order)=[eax, edx, ecx]  regs{edi=mov dword ptr [ecx + 0x13b0] ecx=mov dword ptr [esp + 0x3c] edx=mov dword ptr [eax - 0x10] eax=mov dword ptr [esp + 0x1c]}
    IF cmp eax, 4 ; ja taken if eax > 4  (unsigned above)  -> 0x7D78EB
B7D78EB (0x7D78EB..0x7D7908):
    IF cmp eax, dword ptr [edx + 0x5c] ; jl taken if eax < dword ptr [edx + 0x5c]  (signed less)  -> 0x7D75C0
B7D7908 (0x7D7908..0x7D791D):
    JMP 0x7d7920
B7D7920 (0x7D7920..0x7D7926):
    IF cmp byte ptr [ebx + esi], 0 ; jne taken if byte ptr [ebx + esi] != 0  (not equal / non-zero)  -> 0x7D794B
B7D7926 (0x7D7926..0x7D794B):
    CALL 0x7C20B0   stack(push order)=[edi, esi, 0]  regs{edi=mov dword ptr [esp + 0x10]}
    CALL 0x7D8440   stack(push order)=[esi, edx, ecx]  regs{edx=mov dword ptr [eax + 0x24] ecx=mov dword ptr [eax + 0x28] eax=mov dword ptr [esp + 0x38]}
B7D794B (0x7D794B..0x7D7955):
    IF sub dword ptr [esp + 0x18], 1 ; jne taken on not equal / non-zero of the result  -> 0x7D7920
B7D7955 (0x7D7955..0x7D7974):
    IF je (flags from before the block)  -> 0x7D7C80
B7D7974 (0x7D7974..0x7D797C):
    IF test edi,edi ; je taken if edi == 0  -> 0x7D7C80
B7D797C (0x7D797C..0x7D7988):
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x7D7997
B7D7988 (0x7D7988..0x7D798D):
    IF cmp dword ptr [ecx + 0xc], esi ; je taken if dword ptr [ecx + 0xc] == esi  (equal / zero)  -> 0x7D7997
B7D798D (0x7D798D..0x7D7997):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7D7988
B7D7997 (0x7D7997..0x7D79B1):
    IF test ecx,ecx ; jle taken if ecx <= 0 (signed)  -> 0x7D79C0
B7D79B1 (0x7D79B1..0x7D79B6):
    IF cmp dword ptr [edx + 0x10], ebx ; je taken if dword ptr [edx + 0x10] == ebx  (equal / zero)  -> 0x7D79C0
B7D79B6 (0x7D79B6..0x7D79C0):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7D79B1
B7D79C0 (0x7D79C0..0x7D79D4):
    IF test esi,esi ; je taken if esi == 0  -> 0x7D7C80
B7D79D4 (0x7D79D4..0x7D79DC):
    IF test eax,eax ; je taken if eax == 0  -> 0x7D7C80
B7D79DC (0x7D79DC..0x7D79E5):
    IF test edi,edi ; jle taken if edi <= 0 (signed)  -> 0x7D7A24
B7D79E5 (0x7D79E5..0x7D79EA):
    JMP 0x7d79f0
B7D79F0 (0x7D79F0..0x7D79FA):
    IF cmp eax, dword ptr [0x25288bc] ; jge taken if eax >= dword ptr [0x25288bc]  (signed greater-or-equal)  -> 0x7D7A16
B7D79FA (0x7D79FA..0x7D7A12):
    IF cmp byte ptr [eax + esi], 0 ; je taken if byte ptr [eax + esi] == 0  (equal / zero)  -> 0x7D7C80
B7D7A16 (0x7D7A16..0x7D7A20):
    IF cmp ecx, edi ; jl taken if ecx < edi  (signed less)  -> 0x7D79F0
B7D7A24 (0x7D7A24..0x7D7A3D):
    JMP 0x7d7a40
B7D7A40 (0x7D7A40..0x7D7A44):
    IF cmp dword ptr [ebx], eax ; je taken if dword ptr [ebx] == eax  (equal / zero)  -> 0x7D7A54
B7D7A44 (0x7D7A44..0x7D7A52):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7D7A40
B7D7A52 (0x7D7A52..0x7D7A54):
    JMP 0x7d7a5c
B7D7A54 (0x7D7A54..0x7D7A5C):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7D7A7F
B7D7A5C (0x7D7A5C..0x7D7A7F):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', ecx, eax]  regs{ecx=mov dword ptr [esi + 8] eax=mov dword ptr [edx + 0xc] edx=mov dword ptr [esp + 0x14]}
    JMP 0x7d7c80
B7D7A7F (0x7D7A7F..0x7D7A9C):
    IF cmp dword ptr [edi*4 + 0x1fac828], eax ; jle taken if dword ptr [edi*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7D7ABC
B7D7AA2 (0x7D7AA2..0x7D7AA9):
    IF cmp byte ptr [esi + 0x15], dl ; je taken if byte ptr [esi + 0x15] == dl  (equal / zero)  -> 0x7D7B13
B7D7AA9 (0x7D7AA9..0x7D7AB8):
    IF cmp eax, dword ptr [edi*4 + 0x1fac828] ; jl taken if eax < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D7AA2
B7D7ABC (0x7D7ABC..0x7D7ACB):
    IF cmp dword ptr [edi*4 + 0x1fac828], edx ; jle taken if dword ptr [edi*4 + 0x1fac828] <= edx  (signed less-or-equal)  -> 0x7D7B91
B7D7AD6 (0x7D7AD6..0x7D7ADC):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7D7AFF
B7D7ADC (0x7D7ADC..0x7D7AE8):
    IF test bl,bl ; je taken if bl == 0  -> 0x7D7AFB
B7D7AE8 (0x7D7AE8..0x7D7AF3):
    IF cmp eax, ebx ; jl taken if eax < ebx  (signed less)  -> 0x7D7AFB
B7D7AF3 (0x7D7AF3..0x7D7AFB):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7D7B3F
B7D7AFF (0x7D7AFF..0x7D7B0E):
    IF cmp edx, dword ptr [edi*4 + 0x1fac828] ; jl taken if edx < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7D7AD6
B7D7B0E (0x7D7B0E..0x7D7B13):
    JMP 0x7d7b91
B7D7B13 (0x7D7B13..0x7D7B31):
    IF cmp dword ptr [esi], eax ; jne taken if dword ptr [esi] != eax  (not equal / non-zero)  -> 0x7D7BB5
B7D7B31 (0x7D7B31..0x7D7B3A):
    IF cmp dword ptr [esi + 0x10], ebx ; je taken if dword ptr [esi + 0x10] == ebx  (equal / zero)  -> 0x7D7C72
B7D7B3A (0x7D7B3A..0x7D7B3F):
    JMP 0x7d7bb5
B7D7B3F (0x7D7B3F..0x7D7B4F):
    IF cmp edx, eax ; jge taken if edx >= eax  (signed greater-or-equal)  -> 0x7D7B85
B7D7B62 (0x7D7B62..0x7D7B85):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp ebx, eax ; jl taken if ebx < eax  (signed less)  -> 0x7D7B62
B7D7BB5 (0x7D7BB5..0x7D7BE9):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7D7BEE
B7D7BEE (0x7D7BEE..0x7D7BF5):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7D7C12
B7D7C00 (0x7D7C00..0x7D7C12):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7D7C00
B7D7C12 (0x7D7C12..0x7D7C1D):
    IF test al,al ; je taken if al == 0  -> 0x7D7C41
B7D7C1D (0x7D7C1D..0x7D7C41):
    CALL 0x9E8ED0   stack(push order)=[edi, ebx, 2, 0, 0x7bedb0]  regs{edx=mov dword ptr [edi + 0x10] edi=mov dword ptr [esp + 0x10]}
    JMP 0x7d7c68
B7D7C41 (0x7D7C41..0x7D7C4F):
    IF cmp eax, 4 ; jbe taken if eax <= 4  (unsigned below-or-equal)  -> 0x7D7C54
B7D7C54 (0x7D7C54..0x7D7C68):
    CALL 0x9E8ED0   stack(push order)=[eax, ebx, 2, 0, eax]  regs{eax=mov dword ptr [esp + 0x14]}
B7D7C68 (0x7D7C68..0x7D7C72):
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov edi}
B7D7C72 (0x7D7C72..0x7D7C80):
    CALL 0x7BF550   stack(push order)=[ecx, esi]  regs{ecx=mov dword ptr [esp + 0x10]}
B7D7C80 (0x7D7C80..0x7D7C93):
    IF test esi,esi ; je taken if esi == 0  -> 0x7D7E5D
B7D7C93 (0x7D7C93..0x7D7CA4):
    IF cmp dword ptr [esp + 0x10], 0 ; je taken if dword ptr [esp + 0x10] == 0  (equal / zero)  -> 0x7D7E5D
B7D7CA4 (0x7D7CA4..0x7D7CB0):
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x7D7CBF
B7D7CB0 (0x7D7CB0..0x7D7CB5):
    IF cmp dword ptr [ecx + 0xc], edi ; je taken if dword ptr [ecx + 0xc] == edi  (equal / zero)  -> 0x7D7CBF
B7D7CB5 (0x7D7CB5..0x7D7CBF):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7D7CB0
B7D7CBF (0x7D7CBF..0x7D7CD9):
    IF test ecx,ecx ; jle taken if ecx <= 0 (signed)  -> 0x7D7CEF
B7D7CE0 (0x7D7CE0..0x7D7CE5):
    IF cmp dword ptr [edx + 0x10], ebx ; je taken if dword ptr [edx + 0x10] == ebx  (equal / zero)  -> 0x7D7CEF
B7D7CE5 (0x7D7CE5..0x7D7CEF):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7D7CE0
B7D7CEF (0x7D7CEF..0x7D7D03):
    IF test edi,edi ; je taken if edi == 0  -> 0x7D7E5D
B7D7D03 (0x7D7D03..0x7D7D0B):
    IF test eax,eax ; je taken if eax == 0  -> 0x7D7E5D
B7D7D0B (0x7D7D0B..0x7D7D34):
    CALL 0x7C1D40   stack(push order)=[eax]  regs{edi=mov dword ptr [0x25288bc] ecx=mov dword ptr [esp + 0x14] eax=mov dword ptr [ebx*4 + 0x1fac7f8] edx=mov dword ptr [esp + 0x10]}
    IF test al,al ; je taken if al == 0  -> 0x7D7E5D
B7D7D50 (0x7D7D50..0x7D7D54):
    IF cmp dword ptr [edx], eax ; je taken if dword ptr [edx] == eax  (equal / zero)  -> 0x7D7D64
B7D7D54 (0x7D7D54..0x7D7D62):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7D7D50
B7D7D62 (0x7D7D62..0x7D7D64):
    JMP 0x7d7d6c
B7D7D64 (0x7D7D64..0x7D7D6C):
    IF test edx,edx ; jne taken if edx != 0  -> 0x7D7D8B
B7D7D6C (0x7D7D6C..0x7D7D8B):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', edx, ecx]  regs{edx=mov dword ptr [edi + 8] ecx=mov dword ptr [esi + 0xc]}
    JMP 0x7d7e5d
B7D7D8B (0x7D7D8B..0x7D7DA8):
    IF cmp dword ptr [ebx*4 + 0x1fac828], eax ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7D7DC8
B7D7DB2 (0x7D7DB2..0x7D7DB9):
    IF cmp byte ptr [edi + 0x15], cl ; je taken if byte ptr [edi + 0x15] == cl  (equal / zero)  -> 0x7D7E29
B7D7DB9 (0x7D7DB9..0x7D7DC8):
    IF cmp eax, dword ptr [ebx*4 + 0x1fac828] ; jl taken if eax < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7D7DB2
B7D7DC8 (0x7D7DC8..0x7D7DD7):
    IF cmp dword ptr [ebx*4 + 0x1fac828], edi ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= edi  (signed less-or-equal)  -> 0x7D7EDB
B7D7DF0 (0x7D7DF0..0x7D7DF6):
    IF test esi,esi ; je taken if esi == 0  -> 0x7D7E15
B7D7DF6 (0x7D7DF6..0x7D7E02):
    IF test cl,cl ; je taken if cl == 0  -> 0x7D7E15
B7D7E02 (0x7D7E02..0x7D7E0D):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7D7E15
B7D7E0D (0x7D7E0D..0x7D7E15):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7D7E89
B7D7E15 (0x7D7E15..0x7D7E24):
    IF cmp edi, dword ptr [ebx*4 + 0x1fac828] ; jl taken if edi < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7D7DF0
B7D7E24 (0x7D7E24..0x7D7E29):
    JMP 0x7d7edb
B7D7E29 (0x7D7E29..0x7D7E46):
    IF cmp dword ptr [esi], edi ; jne taken if dword ptr [esi] != edi  (not equal / non-zero)  -> 0x7D7F07
B7D7E46 (0x7D7E46..0x7D7E4F):
    IF cmp dword ptr [esi + 0x10], edx ; jne taken if dword ptr [esi + 0x10] != edx  (not equal / non-zero)  -> 0x7D7F07
B7D7E53 (0x7D7E53..0x7D7E5D):
    CALL 0x7BF550   stack(push order)=[ebx, esi]  regs{}
B7D7E5D (0x7D7E5D..0x7D7E70):
    IF sub eax, 0 ; je taken on equal / zero of the result  -> 0x7D7FC9
B7D7E70 (0x7D7E70..0x7D7E79):
    IF sub eax, 1 ; jne taken on not equal / non-zero of the result  -> 0x7D7FE0
B7D7E79 (0x7D7E79..0x7D7E82):
    IF cmp ecx, 3 ; ja taken if ecx > 3  (unsigned above)  -> 0x7D7FB4
B7D7E82 (0x7D7E82..0x7D7E89):
    JMP dword ptr [ecx*4 + 0x7d80b0]
B7D7E89 (0x7D7E89..0x7D7E97):
    IF cmp edi, eax ; jge taken if edi >= eax  (signed greater-or-equal)  -> 0x7D7ED3
B7D7EB0 (0x7D7EB0..0x7D7ED3):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp edi, eax ; jl taken if edi < eax  (signed less)  -> 0x7D7EB0
B7D7F07 (0x7D7F07..0x7D7F37):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7D7F3C
B7D7F3C (0x7D7F3C..0x7D7F43):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7D7F62
B7D7F50 (0x7D7F50..0x7D7F62):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7D7F50
B7D7F62 (0x7D7F62..0x7D7F71):
    IF test cl,cl ; je taken if cl == 0  -> 0x7D7F86
B7D7F71 (0x7D7F71..0x7D7F86):
    JMP 0x7d7f9a
B7D7F86 (0x7D7F86..0x7D7F94):
    IF cmp ecx, 4 ; jbe taken if ecx <= 4  (unsigned below-or-equal)  -> 0x7D7F99
B7D7F9A (0x7D7F9A..0x7D7FB4):
    CALL 0x9E8ED0   stack(push order)=[ebx, eax, 2, 0]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov ebx}
    JMP 0x7d7e53
B7D7FB4 (0x7D7FB4..0x7D7FBB):
    JMP 0x7d7fe5
B7D7FBB (0x7D7FBB..0x7D7FC2):
    JMP 0x7d7fe5
B7D7FC2 (0x7D7FC2..0x7D7FC9):
    JMP 0x7d7fe5
B7D7FC9 (0x7D7FC9..0x7D7FCE):
    IF cmp ecx, 3 ; ja taken if ecx > 3  (unsigned above)  -> 0x7D7FE0
B7D7FCE (0x7D7FCE..0x7D7FD5):
    JMP dword ptr [ecx*4 + 0x7d80c0]
B7D7FD5 (0x7D7FD5..0x7D7FDC):
    JMP 0x7d7fe5
B7D7FDC (0x7D7FDC..0x7D7FE0):
    JMP 0x7d7fe5
B7D7FE5 (0x7D7FE5..0x7D8038):
    CALL 0x7D8150   stack(push order)=[]  regs{eax=mov dword ptr [edi + 0x7c] ecx=mov dword ptr [edi + 0x78] edx=mov dword ptr [esi + 0x13b0] esi=mov dword ptr [esp + 0x38]}
    CALL 0x95CDB0   stack(push order)=[0]  regs{edx=mov esi ecx=mov eax}
    CALL 0x7C0C30   stack(push order)=[esi, 1]  regs{}
    IF test eax,eax ; jle taken if eax <= 0 (signed)  -> 0x7D8046
B7D8038 (0x7D8038..0x7D803C):
    IF cmp dword ptr [edx], esi ; je taken if dword ptr [edx] == esi  (equal / zero)  -> 0x7D8074
B7D803C (0x7D803C..0x7D8046):
    IF cmp ecx, eax ; jl taken if ecx < eax  (signed less)  -> 0x7D8038
B7D8046 (0x7D8046..0x7D804B):
    IF cmp eax, 0xc ; jge taken if eax >= 0xc  (signed greater-or-equal)  -> 0x7D805F
B7D804B (0x7D804B..0x7D805F):
    G[0x25289C8] <- (mov dword ptr [0x25289c8], eax)
B7D805F (0x7D805F..0x7D8067):
    IF sub edi, 0 ; je taken on equal / zero of the result  -> 0x7D8079
B7D8067 (0x7D8067..0x7D806C):
    IF sub edi, 1 ; jne taken on not equal / non-zero of the result  -> 0x7D808B
B7D806C (0x7D806C..0x7D8074):
    JMP 0x7d8080
B7D8074 (0x7D8074..0x7D8079):
    JMP 0x7d805f
B7D8080 (0x7D8080..0x7D808B):
    CALL 0x96F940 net_send_human_pkt_5  stack(push order)=[esi, 0]  regs{}
B7D808B (0x7D808B..0x7D8091):
    IF cmp byte ptr [ebp + 0xc], 0 ; je taken if byte ptr [ebp + 0xc] == 0  (equal / zero)  -> 0x7D809B
B7D8091 (0x7D8091..0x7D809B):
    CALL 0x9E8DC0   stack(push order)=[1]  regs{}
B7D809B (0x7D809B..0x7D80AD):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 
```

## Map: `stats_xtbl_load` (0x6B2C30, 0xC80 bytes)

Parses `stats.xtbl`; the name table `0xE94F90` (0x18E entries) is searched with `_stricmp` per stat; two jump tables (197 and 123 cases).

### stats_xtbl_load (0x6B2C30)

- **True size** 0xC80 (3200) bytes by recursive descent; 926 instructions; ret imm [0]; entry regs read (linear prefix) none.
- Frame: frame sub esp,0x3c.
- **Callees** (52 call sites, 13 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0xB743F0 | xtbl_load | 6B2C3E 6B3490 | ret [0], reads ['eax', 'ecx'] |
| 0xC99030 | __stricmp | 6B2C68 6B2D39 6B34B8 6B35F8 6B362B 6B36BE 6B3761 6B382F ... | ret [0], reads [] |
| indirect | `call reg/mem` | 6B2CFD 6B347D 6B356B 6B389C | - |
| 0xC990F0 | _strncpy | 6B2D17 6B3585 | ret [0], reads [] |
| 0x6B2BE0 |  | 6B3599 | ret [0], reads ['edi'] |
| 0x7F49E0 | lstr_get | 6B35DA 6B2DBA | ret [0], reads ['esi'] |
| 0x6B08C0 |  | 6B37AA | ret [0], reads ['eax'] |
| 0xB747D0 | xtbl_get_int | 6B37C0 | ret [0], reads ['ecx'] |
| 0x6B0940 |  | 6B37CC | ret [0], reads ['eax'] |
| 0xB74EF0 | xtbl_get_float | 6B37E2 | ret [0], reads ['ecx'] |
| 0xC968F6 | _sprintf | 6B37FD 6B37A0 6B3342 | ret [0], reads [] |
| 0xB74860 | xtbl_get_int_opt | 6B340F 6B341F | ret [0], reads ['ecx'] |
| 0xBFBEE0 | xml_find_child | 6B2F5E | ret [0], reads ['eax'] |
- **Globals read**: 0x2771318x2, 0x2771278x2, 0x2527CEC, 0x2527CE8x2
- **Globals written**: 0x2527CE8x2, 0x1451AC4, 0x1451AC8, 0x1451AD4, 0x1451AD8, 0x1451ACC, 0x1451AD0, 0x2527CECx2
- **Stack objects (lea of a frame slot, passed by address)**: [ebp+0x3c], [ebp+0x40], [esp+0x0]
- **Jump tables** (decoded from the exe bytes):
  - `jmp [idx*4+0x6B38B0]` at 0x6B300B, byte index table 0x6B3990 (197 entries): 0 55 55 55 55 55 55 1 2 3 55 55 55 4 55 5 6 55 55 55 55 55 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 55 55 26 55 55 27 55 28 55 29 55 30 55 31 55 32 55 55 33 55 55 55 34 55 55 55 55 35 55 55 36 55 55 37 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 38 55 55 55 55 55 55 55 55 55 39 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 40 41 42 55 55 43 44 45 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 55 46 55 55 47 55 48 49 49 49 49 49 49 49 49 49 55 55 55 50 55 55 51 55 55 55 55 55 55 55 55 55 55 55 55 55 52 55 55 53 55 54; targets: 0:6B3012 1:6B301E 2:6B302A 3:6B3036 4:6B3042 5:6B304E 6:6B305A 7:6B3066 8:6B3072 9:6B307E 10:6B308A 11:6B3096 12:6B30A2 13:6B30AE 14:6B30BA 15:6B30C6 16:6B30D2 17:6B30DE 18:6B30EA 19:6B30F6 20:6B3102 21:6B310E 22:6B311A 23:6B3126 24:6B3132 25:6B313E 26:6B314A 27:6B3156 28:6B3162 29:6B316E 30:6B317A 31:6B3186 32:6B3192 33:6B319E 34:6B31AA 35:6B31B6 36:6B31C2 37:6B31CE 38:6B31DA 39:6B31ED 40:6B322F 41:6B323B 42:6B3247 43:6B3253 44:6B325F 45:6B326B 46:6B3283 47:6B328F 48:6B329B 49:6B3277 50:6B32A7 51:6B32B3 52:6B32BF 53:6B32CB 54:6B32D4 55:6B3334
  - `jmp [idx*4+0x6B3A58]` at 0x6B32F7, byte index table 0x6B3A74 (123 entries): 0 1 2 3 4 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 6 5; targets: 0:6B32FE 1:6B3307 2:6B3310 3:6B3319 4:6B3322 5:6B332B 6:6B3334
- **x87 / SSE**: none.

**Section-by-section walk**

```
B6B2C30 (0x6B2C30..0x6B2C55):
    CALL 0xB743F0 xtbl_load  stack(push order)=[edi, esi, ebp, ebx]  regs{eax=mov 0xe0983c 'stats.xtbl' ecx=0}
    G[0x2527CE8] <- (mov dword ptr [0x2527ce8], 0)
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2C7B
B6B2C55 (0x6B2C55..0x6B2C5C):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B2C7B
B6B2C60 (0x6B2C60..0x6B2C74):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xdf37d8 'Stat']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2C8D
B6B2C74 (0x6B2C74..0x6B2C7B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2C60
B6B2C7D (0x6B2C7D..0x6B2C8B):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B346B
B6B2C8B (0x6B2C8B..0x6B2C8D):
    JMP 0x6b2c95
B6B2C8D (0x6B2C8D..0x6B2C91):
    JMP 0x6b2c7d
B6B2C95 (0x6B2C95..0x6B2C99):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B2C9D
B6B2C99 (0x6B2C99..0x6B2C9D):
    JMP 0x6b2cce
B6B2C9D (0x6B2C9D..0x6B2CA4):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B2CC7
B6B2CA4 (0x6B2CA4..0x6B2CB8):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xdc5804 'Name']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2CC3
B6B2CB8 (0x6B2CB8..0x6B2CBF):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2CA4
B6B2CBF (0x6B2CBF..0x6B2CC3):
    JMP 0x6b2cce
B6B2CC3 (0x6B2CC3..0x6B2CC7):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2CCB
B6B2CC7 (0x6B2CC7..0x6B2CCB):
    JMP 0x6b2cce
B6B2CD3 (0x6B2CD3..0x6B2CDC):
    IF test cl,cl ; jne taken if cl != 0  -> 0x6B2CD3
B6B2CDC (0x6B2CDC..0x6B2D06):
    CALL indirect edx  stack(push order)=[eax, 1, 0x27712d8, edx]  regs{ecx=mov 0x2771278 edx=mov dword ptr [eax + 0x70] eax=mov dword ptr [0x2771278]}
B6B2D06 (0x6B2D06..0x6B2D0F):
    IF test dl,dl ; jne taken if dl != 0  -> 0x6B2D06
B6B2D0F (0x6B2D0F..0x6B2D30):
    CALL 0xC990F0 _strncpy  stack(push order)=[esi, ebp, edi]  regs{edi=lea [eax + 1]}
B6B2D30 (0x6B2D30..0x6B2D45):
    CALL 0xC99030 __stricmp  stack(push order)=[esi, eax]  regs{eax=mov dword ptr [edi*8 + 0xe94f90]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2D59
B6B2D45 (0x6B2D45..0x6B2D54):
    IF cmp edi, 0x18e ; jl taken if edi < 0x18e  (signed less)  -> 0x6B2D30
B6B2D54 (0x6B2D54..0x6B2D59):
    JMP 0x6b346b
B6B2D59 (0x6B2D59..0x6B2D76):
    IF test ebp,ebp ; je taken if ebp == 0  -> 0x6B346B
B6B2D76 (0x6B2D76..0x6B2D84):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B2D88
B6B2D84 (0x6B2D84..0x6B2D88):
    JMP 0x6b2db8
B6B2D88 (0x6B2D88..0x6B2D8F):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B2DB1
B6B2D90 (0x6B2D90..0x6B2DA4):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xddb3d4 'DisplayName']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2DAD
B6B2DA4 (0x6B2DA4..0x6B2DAB):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2D90
B6B2DAB (0x6B2DAB..0x6B2DAD):
    JMP 0x6b2db8
B6B2DAD (0x6B2DAD..0x6B2DB1):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2DB5
B6B2DB1 (0x6B2DB1..0x6B2DB5):
    JMP 0x6b2db8
B6B2DB8 (0x6B2DB8..0x6B2E0A):
    CALL 0x7F49E0 lstr_get  stack(push order)=[]  regs{edi=0}
    IF je (flags from before the block)  -> 0x6B2E2C
B6B2E0A (0x6B2E0A..0x6B2E11):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B2E2C
B6B2E11 (0x6B2E11..0x6B2E25):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xe09880 'Value_Type']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2E69
B6B2E25 (0x6B2E25..0x6B2E2C):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2E11
B6B2E34 (0x6B2E34..0x6B2E3D):
    JMP 0x6b2e40
B6B2E40 (0x6B2E40..0x6B2E48):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2E73
B6B2E48 (0x6B2E48..0x6B2E4F):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B2E73
B6B2E50 (0x6B2E50..0x6B2E60):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, edi]  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2E6F
B6B2E60 (0x6B2E60..0x6B2E67):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2E50
B6B2E67 (0x6B2E67..0x6B2E69):
    JMP 0x6b2e73
B6B2E69 (0x6B2E69..0x6B2E6F):
    JMP 0x6b2e34
B6B2E6F (0x6B2E6F..0x6B2E73):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2E83
B6B2E73 (0x6B2E73..0x6B2E7E):
    IF test edi,edi ; jne taken if edi != 0  -> 0x6B2E40
B6B2E7E (0x6B2E7E..0x6B2E83):
    JMP 0x6b334a
B6B2E83 (0x6B2E83..0x6B2E99):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09848 'integer', edi]  regs{edi=mov dword ptr [esp + ebx*4 + 0x28]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2EA1
B6B2E99 (0x6B2E99..0x6B2EA1):
    JMP 0x6b334a
B6B2EA1 (0x6B2EA1..0x6B2EB3):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09850 'float', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2EBF
B6B2EB3 (0x6B2EB3..0x6B2EBF):
    JMP 0x6b334a
B6B2EBF (0x6B2EBF..0x6B2ED1):
    CALL 0xC99030 __stricmp  stack(push order)=[0xdd9d5c 'distance', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2EDD
B6B2ED1 (0x6B2ED1..0x6B2EDD):
    JMP 0x6b334a
B6B2EDD (0x6B2EDD..0x6B2EEF):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09858 'time', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2EFB
B6B2EEF (0x6B2EEF..0x6B2EFB):
    JMP 0x6b334a
B6B2EFB (0x6B2EFB..0x6B2F0D):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09860 'money', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2F19
B6B2F0D (0x6B2F0D..0x6B2F19):
    JMP 0x6b334a
B6B2F19 (0x6B2F19..0x6B2F2B):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09868 'boolean', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2F37
B6B2F2B (0x6B2F2B..0x6B2F37):
    JMP 0x6b334a
B6B2F37 (0x6B2F37..0x6B2F49):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09870 'percent', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2FC4
B6B2F49 (0x6B2F49..0x6B2F67):
    CALL 0xBFBEE0 xml_find_child  stack(push order)=[]  regs{eax=mov esi edi=mov 0xe0988c 'Percentage_Of_Stat'}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B2F6B
B6B2F67 (0x6B2F67..0x6B2F6B):
    JMP 0x6b2f6e
B6B2F75 (0x6B2F75..0x6B2F7B):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2FA9
B6B2F80 (0x6B2F80..0x6B2F86):
    IF cmp dl, byte ptr [ecx] ; jne taken if dl != byte ptr [ecx]  (not equal / non-zero)  -> 0x6B2FA0
B6B2F86 (0x6B2F86..0x6B2F8A):
    IF test dl,dl ; je taken if dl == 0  -> 0x6B2F9C
B6B2F8A (0x6B2F8A..0x6B2F92):
    IF cmp dl, byte ptr [ecx + 1] ; jne taken if dl != byte ptr [ecx + 1]  (not equal / non-zero)  -> 0x6B2FA0
B6B2F92 (0x6B2F92..0x6B2F9C):
    IF test dl,dl ; jne taken if dl != 0  -> 0x6B2F80
B6B2F9C (0x6B2F9C..0x6B2FA0):
    JMP 0x6b2fa5
B6B2FA5 (0x6B2FA5..0x6B2FA9):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B2FBC
B6B2FA9 (0x6B2FA9..0x6B2FB7):
    IF cmp esi, 0x1459598 ; jl taken if esi < 0x1459598  (signed less)  -> 0x6B2F75
B6B2FB7 (0x6B2FB7..0x6B2FBC):
    JMP 0x6b334a
B6B2FBC (0x6B2FBC..0x6B2FC4):
    JMP 0x6b334a
B6B2FC4 (0x6B2FC4..0x6B2FDA):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09878 'complex', edi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B334A
B6B2FDA (0x6B2FDA..0x6B2FF0):
    IF cmp eax, 0x112 ; jg taken if eax > 0x112  (signed greater)  -> 0x6B32E6
B6B2FF0 (0x6B2FF0..0x6B2FF6):
    IF je (flags from before the block)  -> 0x6B32DD
B6B2FF6 (0x6B2FF6..0x6B3004):
    IF cmp eax, 0xc4 ; ja taken if eax > 0xc4  (unsigned above)  -> 0x6B3334
B6B3004 (0x6B3004..0x6B3012):
    JMP dword ptr [eax*4 + 0x6b38b0]
B6B3012 (0x6B3012..0x6B301E):
    JMP 0x6b334a
B6B301E (0x6B301E..0x6B302A):
    JMP 0x6b334a
B6B302A (0x6B302A..0x6B3036):
    JMP 0x6b334a
B6B3036 (0x6B3036..0x6B3042):
    JMP 0x6b334a
B6B3042 (0x6B3042..0x6B304E):
    JMP 0x6b334a
B6B304E (0x6B304E..0x6B305A):
    JMP 0x6b334a
B6B305A (0x6B305A..0x6B3066):
    JMP 0x6b334a
B6B3066 (0x6B3066..0x6B3072):
    JMP 0x6b334a
B6B3072 (0x6B3072..0x6B307E):
    JMP 0x6b334a
B6B307E (0x6B307E..0x6B308A):
    JMP 0x6b334a
B6B308A (0x6B308A..0x6B3096):
    JMP 0x6b334a
B6B3096 (0x6B3096..0x6B30A2):
    JMP 0x6b334a
B6B30A2 (0x6B30A2..0x6B30AE):
    JMP 0x6b334a
B6B30AE (0x6B30AE..0x6B30BA):
    JMP 0x6b334a
B6B30BA (0x6B30BA..0x6B30C6):
    JMP 0x6b334a
B6B30C6 (0x6B30C6..0x6B30D2):
    JMP 0x6b334a
B6B30D2 (0x6B30D2..0x6B30DE):
    JMP 0x6b334a
B6B30DE (0x6B30DE..0x6B30EA):
    JMP 0x6b334a
B6B30EA (0x6B30EA..0x6B30F6):
    JMP 0x6b334a
B6B30F6 (0x6B30F6..0x6B3102):
    JMP 0x6b334a
B6B3102 (0x6B3102..0x6B310E):
    JMP 0x6b334a
B6B310E (0x6B310E..0x6B311A):
    JMP 0x6b334a
B6B311A (0x6B311A..0x6B3126):
    JMP 0x6b334a
B6B3126 (0x6B3126..0x6B3132):
    JMP 0x6b334a
B6B3132 (0x6B3132..0x6B313E):
    JMP 0x6b334a
B6B313E (0x6B313E..0x6B314A):
    JMP 0x6b334a
B6B314A (0x6B314A..0x6B3156):
    JMP 0x6b334a
B6B3156 (0x6B3156..0x6B3162):
    JMP 0x6b334a
B6B3162 (0x6B3162..0x6B316E):
    JMP 0x6b334a
B6B316E (0x6B316E..0x6B317A):
    JMP 0x6b334a
B6B317A (0x6B317A..0x6B3186):
    JMP 0x6b334a
B6B3186 (0x6B3186..0x6B3192):
    JMP 0x6b334a
B6B3192 (0x6B3192..0x6B319E):
    JMP 0x6b334a
B6B319E (0x6B319E..0x6B31AA):
    JMP 0x6b334a
B6B31AA (0x6B31AA..0x6B31B6):
    JMP 0x6b334a
B6B31B6 (0x6B31B6..0x6B31C2):
    JMP 0x6b334a
B6B31C2 (0x6B31C2..0x6B31CE):
    JMP 0x6b334a
B6B31CE (0x6B31CE..0x6B31DA):
    JMP 0x6b334a
B6B31DA (0x6B31DA..0x6B31ED):
    JMP 0x6b334a
B6B31ED (0x6B31ED..0x6B322F):
    G[0x1451AC4] <- (mov dword ptr [0x1451ac4], 1)
    G[0x1451AC8] <- (mov dword ptr [0x1451ac8], eax)
    G[0x1451AD4] <- (mov dword ptr [0x1451ad4], 2)
    G[0x1451AD8] <- (mov dword ptr [0x1451ad8], eax)
    G[0x1451ACC] <- (mov dword ptr [0x1451acc], 3)
    G[0x1451AD0] <- (mov dword ptr [0x1451ad0], eax)
    JMP 0x6b334a
B6B322F (0x6B322F..0x6B323B):
    JMP 0x6b334a
B6B323B (0x6B323B..0x6B3247):
    JMP 0x6b334a
B6B3247 (0x6B3247..0x6B3253):
    JMP 0x6b334a
B6B3253 (0x6B3253..0x6B325F):
    JMP 0x6b334a
B6B325F (0x6B325F..0x6B326B):
    JMP 0x6b334a
B6B326B (0x6B326B..0x6B3277):
    JMP 0x6b334a
B6B3277 (0x6B3277..0x6B3283):
    JMP 0x6b334a
B6B3283 (0x6B3283..0x6B328F):
    JMP 0x6b334a
B6B328F (0x6B328F..0x6B329B):
    JMP 0x6b334a
B6B329B (0x6B329B..0x6B32A7):
    JMP 0x6b334a
B6B32A7 (0x6B32A7..0x6B32B3):
    JMP 0x6b334a
B6B32B3 (0x6B32B3..0x6B32BF):
    JMP 0x6b334a
B6B32BF (0x6B32BF..0x6B32CB):
    JMP 0x6b334a
B6B32CB (0x6B32CB..0x6B32D4):
    JMP 0x6b334a
B6B32D4 (0x6B32D4..0x6B32DD):
    JMP 0x6b334a
B6B32DD (0x6B32DD..0x6B32E6):
    JMP 0x6b334a
B6B32E6 (0x6B32E6..0x6B32F0):
    IF cmp eax, 0x7a ; ja taken if eax > 0x7a  (unsigned above)  -> 0x6B3334
B6B32F0 (0x6B32F0..0x6B32FE):
    JMP dword ptr [ecx*4 + 0x6b3a58]
B6B32FE (0x6B32FE..0x6B3307):
    JMP 0x6b334a
B6B3307 (0x6B3307..0x6B3310):
    JMP 0x6b334a
B6B3310 (0x6B3310..0x6B3319):
    JMP 0x6b334a
B6B3319 (0x6B3319..0x6B3322):
    JMP 0x6b334a
B6B3322 (0x6B3322..0x6B332B):
    JMP 0x6b334a
B6B332B (0x6B332B..0x6B3334):
    JMP 0x6b334a
B6B3334 (0x6B3334..0x6B334A):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe098a0 'Handlers not set up for complex stat %s.', edx]  regs{edx=mov dword ptr [ebp]}
B6B334A (0x6B334A..0x6B3352):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B3356
B6B3352 (0x6B3352..0x6B3356):
    JMP 0x6b3388
B6B3356 (0x6B3356..0x6B335D):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B3381
B6B3360 (0x6B3360..0x6B3374):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xe098cc 'Display_Stat']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B337D
B6B3374 (0x6B3374..0x6B337B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3360
B6B337B (0x6B337B..0x6B337D):
    JMP 0x6b3388
B6B337D (0x6B337D..0x6B3381):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3385
B6B3381 (0x6B3381..0x6B3385):
    JMP 0x6b3388
B6B3388 (0x6B3388..0x6B339A):
    CALL 0xC99030 __stricmp  stack(push order)=[0xddb838 'true', esi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B33A0
B6B339A (0x6B339A..0x6B33A0):
    JMP 0x6b33a4
B6B33A4 (0x6B33A4..0x6B33A8):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B33AC
B6B33A8 (0x6B33A8..0x6B33AC):
    JMP 0x6b33db
B6B33AC (0x6B33AC..0x6B33B3):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B33D4
B6B33B3 (0x6B33B3..0x6B33C7):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xe098dc 'Allow_Update_By_Server']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B33D0
B6B33C7 (0x6B33C7..0x6B33CE):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B33B3
B6B33CE (0x6B33CE..0x6B33D0):
    JMP 0x6b33db
B6B33D0 (0x6B33D0..0x6B33D4):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B33D8
B6B33D4 (0x6B33D4..0x6B33D8):
    JMP 0x6b33db
B6B33DB (0x6B33DB..0x6B33ED):
    CALL 0xC99030 __stricmp  stack(push order)=[0xddb838 'true', esi]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B33F3
B6B33ED (0x6B33ED..0x6B33F3):
    JMP 0x6b33f7
B6B33F7 (0x6B33F7..0x6B3437):
    CALL 0xB74860 xtbl_get_int_opt  stack(push order)=[ebx]  regs{ecx=mov 0xe098f4 'LivePropertyID' edi=lea [ebp + 0x40] esi=lea [ebp + 0x3c]}
    CALL 0xB74860 xtbl_get_int_opt  stack(push order)=[ebx]  regs{esi=mov edi ecx=mov 0xe09904 'LiveLeaderboardID'}
    G[0x2527CE8] <- (add dword ptr [0x2527ce8], 1)
    IF cmp dword ptr [esp + 0x1c], 0 ; je taken if dword ptr [esp + 0x1c] == 0  (equal / zero)  -> 0x6B345D
B6B3437 (0x6B3437..0x6B343B):
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x6B345D
B6B343B (0x6B343B..0x6B343D):
    JMP 0x6b3440
B6B3440 (0x6B3440..0x6B3447):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B345D
B6B3447 (0x6B3447..0x6B345B):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xdf37d8 'Stat']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B3440
B6B345B (0x6B345B..0x6B345D):
    JMP 0x6b345f
B6B345F (0x6B345F..0x6B346B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B2C91
B6B346B (0x6B346B..0x6B34A7):
    CALL indirect eax  stack(push order)=[edx]  regs{eax=mov dword ptr [eax + 0x34] edx=mov dword ptr [0xe87134] ecx=mov dword ptr [0xe87138]}
    CALL 0xB743F0 xtbl_load  stack(push order)=[]  regs{eax=mov 0xe09918 'achievements.xtbl' ecx=0}
    G[0x2527CEC] <- (mov dword ptr [0x2527cec], 0)
    IF test eax,eax ; je taken if eax == 0  -> 0x6B34CB
B6B34A7 (0x6B34A7..0x6B34AE):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B34CB
B6B34B0 (0x6B34B0..0x6B34C4):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xe0992c 'Achievement']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B34CD
B6B34C4 (0x6B34C4..0x6B34CB):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B34B0
B6B34CD (0x6B34CD..0x6B34D9):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B388B
B6B34E0 (0x6B34E0..0x6B3503):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B3507
B6B3503 (0x6B3503..0x6B3507):
    JMP 0x6b353a
B6B3507 (0x6B3507..0x6B350E):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B3533
B6B3510 (0x6B3510..0x6B3524):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xdc5804 'Name']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B352F
B6B3524 (0x6B3524..0x6B352B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3510
B6B352B (0x6B352B..0x6B352F):
    JMP 0x6b353a
B6B352F (0x6B352F..0x6B3533):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3537
B6B3533 (0x6B3533..0x6B3537):
    JMP 0x6b353a
B6B3540 (0x6B3540..0x6B3549):
    IF test cl,cl ; jne taken if cl != 0  -> 0x6B3540
B6B3549 (0x6B3549..0x6B3574):
    CALL indirect eax  stack(push order)=[eax, 1, 0x27712d8, ecx]  regs{ecx=mov 0x2771278 eax=mov dword ptr [edx + 0x70] edx=mov dword ptr [0x2771278]}
B6B3574 (0x6B3574..0x6B357D):
    IF test dl,dl ; jne taken if dl != 0  -> 0x6B3574
B6B357D (0x6B357D..0x6B35A5):
    CALL 0xC990F0 _strncpy  stack(push order)=[edi, ebp, esi]  regs{esi=lea [eax + 1]}
    CALL 0x6B2BE0   stack(push order)=[]  regs{}
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B35A9
B6B35A5 (0x6B35A5..0x6B35A9):
    JMP 0x6b35d8
B6B35A9 (0x6B35A9..0x6B35B0):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B35D1
B6B35B0 (0x6B35B0..0x6B35C4):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xddb3d4 'DisplayName']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B35CD
B6B35C4 (0x6B35C4..0x6B35CB):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B35B0
B6B35CB (0x6B35CB..0x6B35CD):
    JMP 0x6b35d8
B6B35CD (0x6B35CD..0x6B35D1):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B35D5
B6B35D1 (0x6B35D1..0x6B35D5):
    JMP 0x6b35d8
B6B35D8 (0x6B35D8..0x6B35E6):
    CALL 0x7F49E0 lstr_get  stack(push order)=[]  regs{edi=0}
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x6B360B
B6B35E6 (0x6B35E6..0x6B35ED):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B360B
B6B35F0 (0x6B35F0..0x6B3604):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe09938 'Requirements']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B360D
B6B3604 (0x6B3604..0x6B360B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B35F0
B6B360D (0x6B360D..0x6B361C):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B363E
B6B361C (0x6B361C..0x6B3623):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B363E
B6B3623 (0x6B3623..0x6B3637):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xe09948 'Requirement']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B3640
B6B3637 (0x6B3637..0x6B363E):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3623
B6B3640 (0x6B3640..0x6B364A):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B3849
B6B3650 (0x6B3650..0x6B3668):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B366C
B6B3668 (0x6B3668..0x6B366C):
    JMP 0x6b369d
B6B366C (0x6B366C..0x6B3673):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B3696
B6B3673 (0x6B3673..0x6B3687):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xdf37d8 'Stat']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B3692
B6B3687 (0x6B3687..0x6B368E):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3673
B6B368E (0x6B368E..0x6B3692):
    JMP 0x6b369d
B6B3692 (0x6B3692..0x6B3696):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B369A
B6B3696 (0x6B3696..0x6B369A):
    JMP 0x6b369d
B6B369D (0x6B369D..0x6B36B1):
    IF cmp dword ptr [0x2527ce8], esi ; jle taken if dword ptr [0x2527ce8] <= esi  (signed less-or-equal)  -> 0x6B380D
B6B36B6 (0x6B36B6..0x6B36BC):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B36CA
B6B36BC (0x6B36BC..0x6B36CA):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, ebp]  regs{}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B36DA
B6B36CA (0x6B36CA..0x6B36D8):
    IF cmp esi, dword ptr [0x2527ce8] ; jl taken if esi < dword ptr [0x2527ce8]  (signed less)  -> 0x6B36B6
B6B36D8 (0x6B36D8..0x6B36DA):
    JMP 0x6b36ee
B6B36EE (0x6B36EE..0x6B36FC):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B380D
B6B36FC (0x6B36FC..0x6B3727):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x6B372B
B6B3727 (0x6B3727..0x6B372B):
    JMP 0x6b375b
B6B372B (0x6B372B..0x6B3732):
    IF test ebp,ebp ; je taken if ebp == 0  -> 0x6B3754
B6B3732 (0x6B3732..0x6B3747):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xdfa558 'Condition']  regs{ecx=mov dword ptr [ebp]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6B3750
B6B3747 (0x6B3747..0x6B374E):
    IF test ebp,ebp ; jne taken if ebp != 0  -> 0x6B3732
B6B374E (0x6B374E..0x6B3750):
    JMP 0x6b375b
B6B3750 (0x6B3750..0x6B3754):
    IF test ebp,ebp ; jne taken if ebp != 0  -> 0x6B3758
B6B3754 (0x6B3754..0x6B3758):
    JMP 0x6b375b
B6B375B (0x6B375B..0x6B376D):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09954 'at least', ebp]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B3776
B6B376D (0x6B376D..0x6B3776):
    JMP 0x6b37a8
B6B3776 (0x6B3776..0x6B3788):
    CALL 0xC99030 __stricmp  stack(push order)=[0xe09960 'at most', ebp]  regs{}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B3795
B6B3788 (0x6B3788..0x6B3795):
    JMP 0x6b37a8
B6B3795 (0x6B3795..0x6B37A8):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe09968 'Unknown requirement condition: %s', ebp]  regs{}
B6B37A8 (0x6B37A8..0x6B37B3):
    CALL 0x6B08C0   stack(push order)=[]  regs{eax=mov esi}
    IF test al,al ; je taken if al == 0  -> 0x6B37CA
B6B37B3 (0x6B37B3..0x6B37CA):
    CALL 0xB747D0 xtbl_get_int  stack(push order)=[ebx]  regs{ecx=mov 0xddab70 'Value' esi=mov dword ptr [esp + 0x14]}
    JMP 0x6b3805
B6B37CA (0x6B37CA..0x6B37D5):
    CALL 0x6B0940   stack(push order)=[]  regs{eax=mov esi}
    IF test al,al ; je taken if al == 0  -> 0x6B37EC
B6B37D5 (0x6B37D5..0x6B37EC):
    CALL 0xB74EF0 xtbl_get_float  stack(push order)=[ebx]  regs{ecx=mov 0xddab70 'Value' esi=mov dword ptr [esp + 0x14]}
    JMP 0x6b3805
B6B37EC (0x6B37EC..0x6B3805):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe0998c 'Unexpected tracked stat requirement valu', ecx]  regs{ecx=mov dword ptr [edi + 0x1452be8]}
B6B380D (0x6B380D..0x6B3816):
    IF cmp dword ptr [esp + 0x24], 0 ; je taken if dword ptr [esp + 0x24] == 0  (equal / zero)  -> 0x6B383D
B6B3816 (0x6B3816..0x6B381A):
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x6B383D
B6B3820 (0x6B3820..0x6B3827):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B383D
B6B3827 (0x6B3827..0x6B383B):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe09948 'Requirement']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B3820
B6B383B (0x6B383B..0x6B383D):
    JMP 0x6b383f
B6B383F (0x6B383F..0x6B3849):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B3650
B6B3849 (0x6B3849..0x6B385B):
    G[0x2527CEC] <- (add dword ptr [0x2527cec], 1)
    IF cmp dword ptr [esp + 0x1c], 0 ; je taken if dword ptr [esp + 0x1c] == 0  (equal / zero)  -> 0x6B387D
B6B385B (0x6B385B..0x6B385F):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B387D
B6B3860 (0x6B3860..0x6B3867):
    IF test esi,esi ; je taken if esi == 0  -> 0x6B387D
B6B3867 (0x6B3867..0x6B387B):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xe0992c 'Achievement']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x6B3860
B6B387B (0x6B387B..0x6B387D):
    JMP 0x6b387f
B6B387F (0x6B387F..0x6B388B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x6B34E0
B6B388B (0x6B388B..0x6B38B0):
    CALL indirect edx  stack(push order)=[eax]  regs{edx=mov dword ptr [edx + 0x34] eax=mov dword ptr [0xe87134] ecx=mov dword ptr [0xe87138]}
    RET 
```

## Map: `pcu_net_write_customisation` (0x81CB90, 0xAF5 bytes)

Writes a human's customisation into the network bit stream; mostly `net_bits_write_bit` and lookups by category / slot id.

### pcu_net_write_customisation (0x81CB90)

- **True size** 0xAF5 (2805) bytes by recursive descent; 975 instructions; ret imm [0]; entry regs read (linear prefix) none.
- Frame: frame sub esp,0x1c.
- **Callees** (6 call sites, 5 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0x7C1BD0 |  | 81CFE9 | ret [0], reads ['edx'] |
| 0x6DED50 |  | 81D1E9 81D511 | ret [0], reads ['ecx'] |
| 0x7FD0B0 | net_bits_write_bit | 81D36F | ret [0], reads ['ecx', 'edx'] |
| 0x7C5270 |  | 81D37E | ret [0], reads ['eax'] |
| 0x7C58D0 |  | 81D38B | ret [0], reads ['eax'] |
- **Globals read**: 0x25288D8, 0x33D630C
- **Globals written**: none
- **Stack objects (lea of a frame slot, passed by address)**: [ebp+0x1], [ebp+0x2], [ebp+0x3], [ebp+0x4], [ebp+0x5], [ebp+0x6], [ebp+0x7], [ebp+0x8], [esp+0x0], [esp+0x18]
- **x87 / SSE**: none.

**Section-by-section walk**

```
B81CB90 (0x81CB90..0x81CBC5):
    IF cmp ecx, ebp ; jae taken if ecx >= ebp  (unsigned above-or-equal)  -> 0x81CBC7
B81CBC7 (0x81CBC7..0x81CBD8):
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x81CBEF
B81CBE0 (0x81CBE0..0x81CBE5):
    IF cmp dword ptr [ebx + 0x14], esi ; je taken if dword ptr [ebx + 0x14] == esi  (equal / zero)  -> 0x81CBF9
B81CBE5 (0x81CBE5..0x81CBEF):
    IF cmp ecx, edx ; jl taken if ecx < edx  (signed less)  -> 0x81CBE0
B81CBEF (0x81CBEF..0x81CBF9):
    RET 
B81CBF9 (0x81CBF9..0x81CC0A):
    IF cmp esi, ebp ; ja taken if esi > ebp  (unsigned above)  -> 0x81CCF0
B81CC12 (0x81CC12..0x81CC2C):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CC30
B81CC2C (0x81CC2C..0x81CC30):
    JMP 0x81cc34
B81CC34 (0x81CC34..0x81CC4E):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CC52
B81CC4E (0x81CC4E..0x81CC52):
    JMP 0x81cc56
B81CC56 (0x81CC56..0x81CC71):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CC75
B81CC71 (0x81CC71..0x81CC75):
    JMP 0x81cc79
B81CC79 (0x81CC79..0x81CC94):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CC98
B81CC94 (0x81CC94..0x81CC98):
    JMP 0x81cc9c
B81CC9C (0x81CC9C..0x81CCB7):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CCBB
B81CCB7 (0x81CCB7..0x81CCBB):
    JMP 0x81ccbf
B81CCBF (0x81CCBF..0x81CCDA):
    IF test bl, 1 ; je taken if no masked bit set  -> 0x81CCDE
B81CCDA (0x81CCDA..0x81CCDE):
    JMP 0x81cce2
B81CCE2 (0x81CCE2..0x81CCF0):
    IF sub ebp, 1 ; jne taken on not equal / non-zero of the result  -> 0x81CC12
B81CCF0 (0x81CCF0..0x81CD02):
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x81CD1F
B81CD10 (0x81CD10..0x81CD15):
    IF cmp dword ptr [eax + 0xc], ebp ; je taken if dword ptr [eax + 0xc] == ebp  (equal / zero)  -> 0x81CD1F
B81CD15 (0x81CD15..0x81CD1F):
    IF cmp ecx, edx ; jl taken if ecx < edx  (signed less)  -> 0x81CD10
B81CD1F (0x81CD1F..0x81CD27):
    IF cmp ecx, edx ; je taken if ecx == edx  (equal / zero)  -> 0x81CE15
B81CD27 (0x81CD27..0x81CD2F):
    IF test eax,eax ; je taken if eax == 0  -> 0x81CE15
B81CD2F (0x81CD2F..0x81CD51):
    IF cmp ebp, dword ptr [esp + 0x24] ; ja taken if ebp > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81CE0E
B81CD51 (0x81CD51..0x81CD57):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81CD6D
B81CD57 (0x81CD57..0x81CD6D):
    JMP 0x81cd82
B81CD82 (0x81CD82..0x81CD9C):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81CDA0
B81CD9C (0x81CD9C..0x81CDA0):
    JMP 0x81cda4
B81CDA4 (0x81CDA4..0x81CDBE):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81CDC2
B81CDBE (0x81CDBE..0x81CDC2):
    JMP 0x81cdc6
B81CDC6 (0x81CDC6..0x81CDE0):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81CDE4
B81CDE0 (0x81CDE0..0x81CDE4):
    JMP 0x81cde8
B81CDE8 (0x81CDE8..0x81CE01):
    IF test al, 2 ; je taken if no masked bit set  -> 0x81CE0A
B81CE01 (0x81CE01..0x81CE0A):
    JMP 0x81ce9e
B81CE0E (0x81CE0E..0x81CE15):
    JMP 0x81ce9e
B81CE15 (0x81CE15..0x81CE1E):
    IF cmp edx, dword ptr [esp + 0x24] ; ja taken if edx > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81CE9C
B81CE9E (0x81CE9E..0x81CEAE):
    IF cmp ecx, -1 ; je taken if ecx == -1  (equal / zero)  -> 0x81CBEF
B81CEAE (0x81CEAE..0x81CEBB):
    IF cmp ebx, dword ptr [esp + 0x24] ; ja taken if ebx > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81CFDC
B81CEBB (0x81CEBB..0x81CEC4):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CED9
B81CEC4 (0x81CEC4..0x81CED9):
    JMP 0x81ceee
B81CEEE (0x81CEEE..0x81CF0A):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CF0F
B81CF0A (0x81CF0A..0x81CF0F):
    JMP 0x81cf14
B81CF14 (0x81CF14..0x81CF30):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CF35
B81CF30 (0x81CF30..0x81CF35):
    JMP 0x81cf3a
B81CF3A (0x81CF3A..0x81CF56):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CF5B
B81CF56 (0x81CF56..0x81CF5B):
    JMP 0x81cf60
B81CF60 (0x81CF60..0x81CF7C):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CF81
B81CF7C (0x81CF7C..0x81CF81):
    JMP 0x81cf86
B81CF86 (0x81CF86..0x81CFA2):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81CFA7
B81CFA2 (0x81CFA2..0x81CFA7):
    JMP 0x81cfac
B81CFAC (0x81CFAC..0x81CFB6):
    IF test cl, 2 ; je taken if no masked bit set  -> 0x81CFC9
B81CFB6 (0x81CFB6..0x81CFC9):
    JMP 0x81cfdc
B81CFDC (0x81CFDC..0x81CFF6):
    CALL 0x7C1BD0   stack(push order)=[]  regs{edx=mov ebp esi=mov dword ptr [eax + 4] eax=mov dword ptr [esp + 0x34]}
    IF test eax,eax ; je taken if eax == 0  -> 0x81D0BD
B81CFF6 (0x81CFF6..0x81D01B):
    IF cmp ecx, dword ptr [esp + 0x24] ; ja taken if ecx > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81D0B9
B81D01B (0x81D01B..0x81D033):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81D037
B81D033 (0x81D033..0x81D037):
    JMP 0x81d03b
B81D03B (0x81D03B..0x81D056):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81D05B
B81D056 (0x81D056..0x81D05B):
    JMP 0x81d060
B81D060 (0x81D060..0x81D07B):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81D080
B81D07B (0x81D07B..0x81D080):
    JMP 0x81d085
B81D085 (0x81D085..0x81D08E):
    IF test al, 2 ; je taken if no masked bit set  -> 0x81D0A6
B81D08E (0x81D08E..0x81D0A6):
    JMP 0x81d12c
B81D0B9 (0x81D0B9..0x81D0BD):
    JMP 0x81d12c
B81D0BD (0x81D0BD..0x81D0C6):
    IF cmp esi, dword ptr [esp + 0x24] ; ja taken if esi > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81D12A
B81D12C (0x81D12C..0x81D13C):
    IF cmp edx, 3 ; ja taken if edx > 3  (unsigned above)  -> 0x81CBEF
B81D13C (0x81D13C..0x81D145):
    IF cmp esi, dword ptr [esp + 0x24] ; ja taken if esi > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81D199
B81D145 (0x81D145..0x81D15D):
    IF test dl, 1 ; je taken if no masked bit set  -> 0x81D161
B81D15D (0x81D15D..0x81D161):
    JMP 0x81d165
B81D165 (0x81D165..0x81D16D):
    IF test dl, 2 ; je taken if no masked bit set  -> 0x81D184
B81D16D (0x81D16D..0x81D184):
    JMP 0x81d199
B81D199 (0x81D199..0x81D1A8):
    IF test eax,eax ; jbe taken if eax <=  -> 0x81D356
B81D1A8 (0x81D1A8..0x81D1B2):
    IF cmp ecx, 0x57 ; ja taken if ecx > 0x57  (unsigned above)  -> 0x81D1BF
B81D1B2 (0x81D1B2..0x81D1BF):
    JMP 0x81d1c7
B81D1C7 (0x81D1C7..0x81D1D1):
    IF test eax,eax ; jbe taken if eax <=  -> 0x81D356
B81D1E0 (0x81D1E0..0x81D1F9):
    CALL 0x6DED50   stack(push order)=[ecx]  regs{ecx=mov dword ptr [esp + 0x18]}
    IF test eax,eax ; jl taken if eax < 0 (signed)  -> 0x81D3D0
B81D1F9 (0x81D1F9..0x81D206):
    IF cmp ebp, dword ptr [esp + 0x24] ; ja taken if ebp > dword ptr [esp + 0x24]  (unsigned above)  -> 0x81D33B
B81D206 (0x81D206..0x81D20C):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81D222
B81D20C (0x81D20C..0x81D222):
    JMP 0x81d237
B81D237 (0x81D237..0x81D254):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D258
B81D254 (0x81D254..0x81D258):
    JMP 0x81d25c
B81D25C (0x81D25C..0x81D277):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D27B
B81D277 (0x81D277..0x81D27B):
    JMP 0x81d27f
B81D27F (0x81D27F..0x81D29A):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D29E
B81D29A (0x81D29A..0x81D29E):
    JMP 0x81d2a2
B81D2A2 (0x81D2A2..0x81D2BD):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D2C1
B81D2BD (0x81D2BD..0x81D2C1):
    JMP 0x81d2c5
B81D2C5 (0x81D2C5..0x81D2E0):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D2E4
B81D2E0 (0x81D2E0..0x81D2E4):
    JMP 0x81d2e8
B81D2E8 (0x81D2E8..0x81D303):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D307
B81D303 (0x81D303..0x81D307):
    JMP 0x81d30b
B81D30B (0x81D30B..0x81D315):
    IF test cl, 2 ; je taken if no masked bit set  -> 0x81D328
B81D315 (0x81D315..0x81D328):
    JMP 0x81d33b
B81D33B (0x81D33B..0x81D352):
    IF cmp ebx, dword ptr [eax + 0xc] ; jb taken if ebx < dword ptr [eax + 0xc]  (unsigned below)  -> 0x81D1E0
B81D356 (0x81D356..0x81D37C):
    CALL 0x7FD0B0 net_bits_write_bit  stack(push order)=[]  regs{ecx=lea [esp + 0x18] esi=mov dword ptr [esp + 0x34]}
    IF test bl,bl ; je taken if bl == 0  -> 0x81D671
B81D37C (0x81D37C..0x81D38B):
    CALL 0x7C5270   stack(push order)=[]  regs{eax=mov esi}
    IF test eax,eax ; je taken if eax == 0  -> 0x81CBEF
B81D38B (0x81D38B..0x81D39C):
    CALL 0x7C58D0   stack(push order)=[]  regs{}
    IF cmp ecx, -1 ; je taken if ecx == -1  (equal / zero)  -> 0x81CBEF
B81D39C (0x81D39C..0x81D3B3):
    IF cmp ebp, ebx ; ja taken if ebp > ebx  (unsigned above)  -> 0x81D507
B81D3B3 (0x81D3B3..0x81D3CC):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D3DE
B81D3CC (0x81D3CC..0x81D3D0):
    JMP 0x81d3e2
B81D3D0 (0x81D3D0..0x81D3DE):
    RET 
B81D3E2 (0x81D3E2..0x81D3FD):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D401
B81D3FD (0x81D3FD..0x81D401):
    JMP 0x81d405
B81D405 (0x81D405..0x81D420):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D424
B81D420 (0x81D420..0x81D424):
    JMP 0x81d428
B81D428 (0x81D428..0x81D443):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D447
B81D443 (0x81D443..0x81D447):
    JMP 0x81d44b
B81D44B (0x81D44B..0x81D466):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D46A
B81D466 (0x81D466..0x81D46A):
    JMP 0x81d46e
B81D46E (0x81D46E..0x81D489):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D48D
B81D489 (0x81D489..0x81D48D):
    JMP 0x81d491
B81D491 (0x81D491..0x81D4AC):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D4B0
B81D4AC (0x81D4AC..0x81D4B0):
    JMP 0x81d4b4
B81D4B4 (0x81D4B4..0x81D4CF):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D4D3
B81D4CF (0x81D4CF..0x81D4D3):
    JMP 0x81d4d7
B81D4D7 (0x81D4D7..0x81D4E1):
    IF test cl, 2 ; je taken if no masked bit set  -> 0x81D4F4
B81D4E1 (0x81D4E1..0x81D4F4):
    JMP 0x81d507
B81D507 (0x81D507..0x81D521):
    CALL 0x6DED50   stack(push order)=[eax]  regs{ecx=0 eax=mov dword ptr [esp + 0x34]}
    IF test eax,eax ; jl taken if eax < 0 (signed)  -> 0x81CBEF
B81D521 (0x81D521..0x81D52C):
    IF cmp edi, ebx ; ja taken if edi > ebx  (unsigned above)  -> 0x81D65F
B81D52C (0x81D52C..0x81D543):
    IF test al, 1 ; je taken if no masked bit set  -> 0x81D547
B81D543 (0x81D543..0x81D547):
    JMP 0x81d54b
B81D54B (0x81D54B..0x81D568):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D56C
B81D568 (0x81D568..0x81D56C):
    JMP 0x81d570
B81D570 (0x81D570..0x81D58B):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D58F
B81D58B (0x81D58B..0x81D58F):
    JMP 0x81d593
B81D593 (0x81D593..0x81D5AE):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D5B2
B81D5AE (0x81D5AE..0x81D5B2):
    JMP 0x81d5b6
B81D5B6 (0x81D5B6..0x81D5D1):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D5D5
B81D5D1 (0x81D5D1..0x81D5D5):
    JMP 0x81d5d9
B81D5D9 (0x81D5D9..0x81D5F4):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D5F8
B81D5F4 (0x81D5F4..0x81D5F8):
    JMP 0x81d5fc
B81D5FC (0x81D5FC..0x81D617):
    IF test cl, 1 ; je taken if no masked bit set  -> 0x81D61B
B81D617 (0x81D617..0x81D61B):
    JMP 0x81d61f
B81D61F (0x81D61F..0x81D629):
    IF test cl, 2 ; je taken if no masked bit set  -> 0x81D64C
B81D629 (0x81D629..0x81D64C):
    RET 
B81D65F (0x81D65F..0x81D671):
    RET 
B81D671 (0x81D671..0x81D685):
    RET 
```

## Map: `stat_awards_check_group` (0x6B5990, 0x9CD bytes)

A fixed sequence of award checks guarded by `stats_can_award`; reads the human object through the object table `0x2149C64` (16-byte slots, handle & 0xFFF, `[obj+0x44]` must equal the handle, `[obj+0x4C]` type).

### stat_awards_check_group (0x6B5990)

- **True size** 0x9CD (2509) bytes by recursive descent; 596 instructions; ret imm [0]; entry regs read (linear prefix) none.
- **Callees** (56 call sites, 7 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0x6B6460 | stats_can_award | 6B5A07 6B5A2E 6B5B95 6B5BCC 6B5C57 6B5C88 6B5D20 6B5D4F ... | ret [0], reads [] |
| indirect | `call reg/mem` | 6B5A65 6B5C0F 6B5CBB 6B5D8E 6B5F8C 6B603C | - |
| 0x6B3B30 | stat_award_check | 6B5A7B 6B5C29 6B5CD7 6B5DA8 6B5FA7 6B6057 6B6117 6B61DB ... | ret [0], reads [] |
| 0x880410 |  | 6B5A99 | ret [0], reads ['esi'] |
| 0xB72F10 |  | 6B606D | ret [0], reads ['eax'] |
| 0xB72F30 |  | 6B61F2 | ret [0], reads ['eax'] |
| 0xB72F50 |  | 6B61FD | ret [0], reads ['ecx'] |
- **Globals read**: 0x21703D4, 0x2527CE8x13, 0x27B3C60x26, 0x2527C76x18, 0x2527B5Ax13, 0x1457A44, 0x1457A4Cx2, 0x1457A88, 0x1457A90x2, 0x1457ACC, 0x1457AD4x2, 0x145779C, 0x14577A4x2, 0x14577E0, 0x14577E8x2, 0x1457824, 0x145782Cx2, 0x1457868, 0x1457870x2, 0x1457B98, 0x1457BA0x2, 0x1457B54, 0x1457B5Cx2, 0x1458ABC, 0x1458AC4x2, 0x1458B00, 0x1458B08x2, 0x1458F84, 0x1458F8Cx2, 0x1458FC8, 0x1458FD0x2
- **Globals written**: none
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x0], [esp+0x10], [esp+0x18]
- **Jump tables** (decoded from the exe bytes):
  - `jmp [idx*4+0x6B6360]` at 0x6B5AAD (4 entries): 0:6B5AB4 1:6B5AB4 2:6B5C3F 3:6B5B7D
- **x87 / SSE**: none.

**Section-by-section walk**

```
B6B5990 (0x6B5990..0x6B599D):
    IF test edx,edx ; je taken if edx == 0  -> 0x6B635B
B6B599D (0x6B599D..0x6B59A8):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x6B635B
B6B59A8 (0x6B59A8..0x6B59C0):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B635B
B6B59C0 (0x6B59C0..0x6B59C9):
    IF cmp dword ptr [eax + 0x44], ecx ; jne taken if dword ptr [eax + 0x44] != ecx  (not equal / non-zero)  -> 0x6B635B
B6B59C9 (0x6B59C9..0x6B59D8):
    IF cmp dword ptr [eax + 0x4c], esi ; jne taken if dword ptr [eax + 0x4c] != esi  (not equal / non-zero)  -> 0x6B635A
B6B59D8 (0x6B59D8..0x6B59E5):
    IF cmp byte ptr [eax + 0x3cc], 1 ; jne taken if byte ptr [eax + 0x3cc] != 1  (not equal / non-zero)  -> 0x6B635A
B6B59E5 (0x6B59E5..0x6B59F7):
    IF cmp edx, eax ; jne taken if edx != eax  (not equal / non-zero)  -> 0x6B5A8A
B6B59F7 (0x6B59F7..0x6B5A07):
    IF cmp dword ptr [0x2527ce8], 0x127 ; jle taken if dword ptr [0x2527ce8] <= 0x127  (signed less-or-equal)  -> 0x6B5A8A
B6B5A07 (0x6B5A07..0x6B5A10):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5A8A
B6B5A10 (0x6B5A10..0x6B5A19):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5A21
B6B5A19 (0x6B5A19..0x6B5A21):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B5A8A
B6B5A21 (0x6B5A21..0x6B5A2A):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5A8A
B6B5A2A (0x6B5A2A..0x6B5A37):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5A8A
B6B5A37 (0x6B5A37..0x6B5A40):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5A48
B6B5A40 (0x6B5A40..0x6B5A48):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B5A8A
B6B5A48 (0x6B5A48..0x6B5A5B):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5A8A
B6B5A5B (0x6B5A5B..0x6B5A74):
    CALL indirect eax  stack(push order)=[0x1457a3c, edx]  regs{edx=lea [esp + 0x10]}
    IF cmp dword ptr [0x1457a4c], ebp ; jle taken if dword ptr [0x1457a4c] <= ebp  (signed less-or-equal)  -> 0x6B5A8A
B6B5A74 (0x6B5A74..0x6B5A8A):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x1457a50]}
    IF cmp ebp, dword ptr [0x1457a4c] ; jl taken if ebp < dword ptr [0x1457a4c]  (signed less)  -> 0x6B5A74
B6B5A8A (0x6B5A8A..0x6B5AAD):
    CALL 0x880410   stack(push order)=[eax]  regs{eax=mov edi esi=mov dword ptr [esp + 0x18] edi=mov dword ptr [esp + 0x1c]}
    IF cmp eax, 3 ; ja taken if eax > 3  (unsigned above)  -> 0x6B6357
B6B5AAD (0x6B5AAD..0x6B5AB4):
    JMP dword ptr [eax*4 + 0x6b6360]
B6B5AB4 (0x6B5AB4..0x6B5ABC):
    IF cmp esi, ebx ; jne taken if esi != ebx  (not equal / non-zero)  -> 0x6B6357
B6B5ABC (0x6B5ABC..0x6B5ACC):
    IF cmp dword ptr [0x2527ce8], 0x128 ; jle taken if dword ptr [0x2527ce8] <= 0x128  (signed less-or-equal)  -> 0x6B6357
B6B5ACC (0x6B5ACC..0x6B5AD9):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B5AD9 (0x6B5AD9..0x6B5AE2):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5AEE
B6B5AE2 (0x6B5AE2..0x6B5AEE):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B5AEE (0x6B5AEE..0x6B5AFB):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B6357
B6B5AFB (0x6B5AFB..0x6B5B10):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B5B10 (0x6B5B10..0x6B5B19):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5B25
B6B5B19 (0x6B5B19..0x6B5B25):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B5B25 (0x6B5B25..0x6B5B3C):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B6357
B6B5B3C (0x6B5B3C..0x6B5B59):
    CALL indirect eax  stack(push order)=[0x1457a80, edx]  regs{edx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1457a90], esi ; jle taken if dword ptr [0x1457a90] <= esi  (signed less-or-equal)  -> 0x6B6357
B6B5B60 (0x6B5B60..0x6B5B77):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1457a94]}
    IF cmp esi, dword ptr [0x1457a90] ; jl taken if esi < dword ptr [0x1457a90]  (signed less)  -> 0x6B5B60
B6B5B77 (0x6B5B77..0x6B5B7D):
    RET 
B6B5B7D (0x6B5B7D..0x6B5B85):
    IF cmp edi, ebx ; jne taken if edi != ebx  (not equal / non-zero)  -> 0x6B6357
B6B5B85 (0x6B5B85..0x6B5B95):
    IF cmp dword ptr [0x2527ce8], 0x129 ; jle taken if dword ptr [0x2527ce8] <= 0x129  (signed less-or-equal)  -> 0x6B6357
B6B5B95 (0x6B5B95..0x6B5BA2):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B5BA2 (0x6B5BA2..0x6B5BAB):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5BB7
B6B5BAB (0x6B5BAB..0x6B5BB7):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B5BB7 (0x6B5BB7..0x6B5BC4):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B6357
B6B5BC4 (0x6B5BC4..0x6B5BD9):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B5BD9 (0x6B5BD9..0x6B5BE2):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5BEE
B6B5BE2 (0x6B5BE2..0x6B5BEE):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B5BEE (0x6B5BEE..0x6B5C05):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B6357
B6B5C05 (0x6B5C05..0x6B5C22):
    CALL indirect eax  stack(push order)=[0x1457ac4, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1457ad4], esi ; jle taken if dword ptr [0x1457ad4] <= esi  (signed less-or-equal)  -> 0x6B6357
B6B5C22 (0x6B5C22..0x6B5C39):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1457ad8]}
    IF cmp esi, dword ptr [0x1457ad4] ; jl taken if esi < dword ptr [0x1457ad4]  (signed less)  -> 0x6B5C22
B6B5C39 (0x6B5C39..0x6B5C3F):
    RET 
B6B5C3F (0x6B5C3F..0x6B5C47):
    IF cmp edi, ebx ; jne taken if edi != ebx  (not equal / non-zero)  -> 0x6B6357
B6B5C47 (0x6B5C47..0x6B5C57):
    IF cmp dword ptr [0x2527ce8], 0x11d ; jle taken if dword ptr [0x2527ce8] <= 0x11d  (signed less-or-equal)  -> 0x6B5CE7
B6B5C57 (0x6B5C57..0x6B5C64):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5CE7
B6B5C64 (0x6B5C64..0x6B5C73):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5C77
B6B5C73 (0x6B5C73..0x6B5C77):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5CED
B6B5C77 (0x6B5C77..0x6B5C80):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5CED
B6B5C80 (0x6B5C80..0x6B5C91):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5CED
B6B5C91 (0x6B5C91..0x6B5C9A):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5C9E
B6B5C9A (0x6B5C9A..0x6B5C9E):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5CED
B6B5C9E (0x6B5C9E..0x6B5CB1):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5CED
B6B5CB1 (0x6B5CB1..0x6B5CCA):
    CALL indirect eax  stack(push order)=[0x1457794, ecx]  regs{ecx=lea [esp + 0x10]}
    IF cmp dword ptr [0x14577a4], ebp ; jle taken if dword ptr [0x14577a4] <= ebp  (signed less-or-equal)  -> 0x6B5CE7
B6B5CD0 (0x6B5CD0..0x6B5CE7):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x14577a8]}
    IF cmp ebp, dword ptr [0x14577a4] ; jl taken if ebp < dword ptr [0x14577a4]  (signed less)  -> 0x6B5CD0
B6B5CED (0x6B5CED..0x6B5CFB):
    IF sub eax, 0xb ; je taken on equal / zero of the result  -> 0x6B5E6B
B6B5CFB (0x6B5CFB..0x6B5D08):
    IF sub eax, esi ; je taken on equal / zero of the result  -> 0x6B5DBC
B6B5D08 (0x6B5D08..0x6B5D10):
    IF sub eax, esi ; jne taken on not equal / non-zero of the result  -> 0x6B5F0D
B6B5D10 (0x6B5D10..0x6B5D20):
    IF cmp dword ptr [0x2527ce8], 0x11e ; jle taken if dword ptr [0x2527ce8] <= 0x11e  (signed less-or-equal)  -> 0x6B5F0D
B6B5D20 (0x6B5D20..0x6B5D2D):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5D2D (0x6B5D2D..0x6B5D36):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5D3E
B6B5D36 (0x6B5D36..0x6B5D3E):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5D3E (0x6B5D3E..0x6B5D4B):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5F0D
B6B5D4B (0x6B5D4B..0x6B5D5C):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5D5C (0x6B5D5C..0x6B5D65):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5D6D
B6B5D65 (0x6B5D65..0x6B5D6D):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5D6D (0x6B5D6D..0x6B5D84):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5F0D
B6B5D84 (0x6B5D84..0x6B5DA1):
    CALL indirect eax  stack(push order)=[0x14577d8, ecx]  regs{ecx=lea [esp + 0x10]}
    IF cmp dword ptr [0x14577e8], ebp ; jle taken if dword ptr [0x14577e8] <= ebp  (signed less-or-equal)  -> 0x6B5F07
B6B5DA1 (0x6B5DA1..0x6B5DB7):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x14577ec]}
    IF cmp ebp, dword ptr [0x14577e8] ; jl taken if ebp < dword ptr [0x14577e8]  (signed less)  -> 0x6B5DA1
B6B5DB7 (0x6B5DB7..0x6B5DBC):
    JMP 0x6b5f07
B6B5DBC (0x6B5DBC..0x6B5DCC):
    IF cmp dword ptr [0x2527ce8], 0x11f ; jle taken if dword ptr [0x2527ce8] <= 0x11f  (signed less-or-equal)  -> 0x6B5F0D
B6B5DCC (0x6B5DCC..0x6B5DD9):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5DD9 (0x6B5DD9..0x6B5DE2):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5DEA
B6B5DE2 (0x6B5DE2..0x6B5DEA):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5DEA (0x6B5DEA..0x6B5DF7):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5F0D
B6B5DF7 (0x6B5DF7..0x6B5E08):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5E08 (0x6B5E08..0x6B5E11):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5E19
B6B5E11 (0x6B5E11..0x6B5E19):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5E19 (0x6B5E19..0x6B5E30):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5F0D
B6B5E30 (0x6B5E30..0x6B5E4D):
    CALL indirect eax  stack(push order)=[0x145781c, ecx]  regs{ecx=lea [esp + 0x10]}
    IF cmp dword ptr [0x145782c], ebp ; jle taken if dword ptr [0x145782c] <= ebp  (signed less-or-equal)  -> 0x6B5F07
B6B5E50 (0x6B5E50..0x6B5E66):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x1457830]}
    IF cmp ebp, dword ptr [0x145782c] ; jl taken if ebp < dword ptr [0x145782c]  (signed less)  -> 0x6B5E50
B6B5E66 (0x6B5E66..0x6B5E6B):
    JMP 0x6b5f07
B6B5E6B (0x6B5E6B..0x6B5E7B):
    IF cmp dword ptr [0x2527ce8], 0x120 ; jle taken if dword ptr [0x2527ce8] <= 0x120  (signed less-or-equal)  -> 0x6B5F0D
B6B5E7B (0x6B5E7B..0x6B5E88):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5E88 (0x6B5E88..0x6B5E91):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5E95
B6B5E91 (0x6B5E91..0x6B5E95):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5E95 (0x6B5E95..0x6B5E9E):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5F0D
B6B5E9E (0x6B5E9E..0x6B5EAF):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5F0D
B6B5EAF (0x6B5EAF..0x6B5EB8):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5EBC
B6B5EB8 (0x6B5EB8..0x6B5EBC):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5F0D
B6B5EBC (0x6B5EBC..0x6B5ECF):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5F0D
B6B5ECF (0x6B5ECF..0x6B5EE8):
    CALL indirect eax  stack(push order)=[0x1457860, ecx]  regs{ecx=lea [esp + 0x10]}
    IF cmp dword ptr [0x1457870], ebp ; jle taken if dword ptr [0x1457870] <= ebp  (signed less-or-equal)  -> 0x6B5F07
B6B5EE8 (0x6B5EE8..0x6B5EEA):
    JMP 0x6b5ef0
B6B5EF0 (0x6B5EF0..0x6B5F07):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x1457874]}
    IF cmp ebp, dword ptr [0x1457870] ; jl taken if ebp < dword ptr [0x1457870]  (signed less)  -> 0x6B5EF0
B6B5F0D (0x6B5F0D..0x6B5F1E):
    IF test byte ptr [edx + 0xc08], 2 ; je taken if no masked bit set  -> 0x6B5FBD
B6B5F1E (0x6B5F1E..0x6B5F2E):
    IF cmp dword ptr [0x2527ce8], 0x12c ; jle taken if dword ptr [0x2527ce8] <= 0x12c  (signed less-or-equal)  -> 0x6B5FBD
B6B5F2E (0x6B5F2E..0x6B5F3B):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5FBD
B6B5F3B (0x6B5F3B..0x6B5F44):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5F48
B6B5F44 (0x6B5F44..0x6B5F48):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5FBD
B6B5F48 (0x6B5F48..0x6B5F51):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B5FBD
B6B5F51 (0x6B5F51..0x6B5F62):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B5FBD
B6B5F62 (0x6B5F62..0x6B5F6B):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5F6F
B6B5F6B (0x6B5F6B..0x6B5F6F):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B5FBD
B6B5F6F (0x6B5F6F..0x6B5F82):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B5FBD
B6B5F82 (0x6B5F82..0x6B5F9B):
    CALL indirect eax  stack(push order)=[0x1457b90, ecx]  regs{ecx=lea [esp + 0x10]}
    IF cmp dword ptr [0x1457ba0], ebp ; jle taken if dword ptr [0x1457ba0] <= ebp  (signed less-or-equal)  -> 0x6B5FB7
B6B5F9B (0x6B5F9B..0x6B5F9D):
    JMP 0x6b5fa0
B6B5FA0 (0x6B5FA0..0x6B5FB7):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [ebp*4 + 0x1457ba4]}
    IF cmp ebp, dword ptr [0x1457ba0] ; jl taken if ebp < dword ptr [0x1457ba0]  (signed less)  -> 0x6B5FA0
B6B5FBD (0x6B5FBD..0x6B5FCE):
    IF test byte ptr [edx + 0xc08], 1 ; je taken if no masked bit set  -> 0x6B6067
B6B5FCE (0x6B5FCE..0x6B5FDE):
    IF cmp dword ptr [0x2527ce8], 0x12b ; jle taken if dword ptr [0x2527ce8] <= 0x12b  (signed less-or-equal)  -> 0x6B6067
B6B5FDE (0x6B5FDE..0x6B5FEB):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6067
B6B5FEB (0x6B5FEB..0x6B5FF4):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B5FF8
B6B5FF4 (0x6B5FF4..0x6B5FF8):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B6067
B6B5FF8 (0x6B5FF8..0x6B6001):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B6067
B6B6001 (0x6B6001..0x6B6012):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6067
B6B6012 (0x6B6012..0x6B601B):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B601F
B6B601B (0x6B601B..0x6B601F):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B6067
B6B601F (0x6B601F..0x6B6032):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B6067
B6B6032 (0x6B6032..0x6B604B):
    CALL indirect eax  stack(push order)=[0x1457b4c, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1457b5c], esi ; jle taken if dword ptr [0x1457b5c] <= esi  (signed less-or-equal)  -> 0x6B6067
B6B604B (0x6B604B..0x6B604D):
    JMP 0x6b6050
B6B6050 (0x6B6050..0x6B6067):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1457b60]}
    IF cmp esi, dword ptr [0x1457b5c] ; jl taken if esi < dword ptr [0x1457b5c]  (signed less)  -> 0x6B6050
B6B6067 (0x6B6067..0x6B607F):
    CALL 0xB72F10   stack(push order)=[]  regs{eax=mov ebp}
    IF cmp byte ptr [esp + 0x28], 0 ; je taken if byte ptr [esp + 0x28] == 0  (equal / zero)  -> 0x6B6127
B6B607F (0x6B607F..0x6B6087):
    IF test bl,bl ; jne taken if bl != 0  -> 0x6B6127
B6B6087 (0x6B6087..0x6B6097):
    IF cmp dword ptr [0x2527ce8], 0x165 ; jle taken if dword ptr [0x2527ce8] <= 0x165  (signed less-or-equal)  -> 0x6B6127
B6B6097 (0x6B6097..0x6B60A4):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6127
B6B60A4 (0x6B60A4..0x6B60AD):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B60B5
B6B60AD (0x6B60AD..0x6B60B5):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6127
B6B60B5 (0x6B60B5..0x6B60BE):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B6127
B6B60BE (0x6B60BE..0x6B60CF):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6127
B6B60CF (0x6B60CF..0x6B60D8):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B60E0
B6B60D8 (0x6B60D8..0x6B60E0):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6127
B6B60E0 (0x6B60E0..0x6B60F3):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B6127
B6B60F3 (0x6B60F3..0x6B610C):
    CALL indirect eax  stack(push order)=[0x1458ab4, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1458ac4], esi ; jle taken if dword ptr [0x1458ac4] <= esi  (signed less-or-equal)  -> 0x6B6127
B6B6110 (0x6B6110..0x6B6127):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1458ac8]}
    IF cmp esi, dword ptr [0x1458ac4] ; jl taken if esi < dword ptr [0x1458ac4]  (signed less)  -> 0x6B6110
B6B6127 (0x6B6127..0x6B612F):
    IF test ebp,ebp ; je taken if ebp == 0  -> 0x6B62A7
B6B612F (0x6B612F..0x6B6137):
    IF test bl,bl ; je taken if bl == 0  -> 0x6B61F0
B6B6137 (0x6B6137..0x6B6147):
    IF cmp dword ptr [0x2527ce8], 0x166 ; jle taken if dword ptr [0x2527ce8] <= 0x166  (signed less-or-equal)  -> 0x6B62A7
B6B6147 (0x6B6147..0x6B6154):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B62A7
B6B6154 (0x6B6154..0x6B615D):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B6169
B6B615D (0x6B615D..0x6B6169):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B62A7
B6B6169 (0x6B6169..0x6B6176):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B62A7
B6B6176 (0x6B6176..0x6B618B):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B62A7
B6B618B (0x6B618B..0x6B6194):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B61A0
B6B6194 (0x6B6194..0x6B61A0):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B62A7
B6B61A0 (0x6B61A0..0x6B61B7):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B62A7
B6B61B7 (0x6B61B7..0x6B61D4):
    CALL indirect eax  stack(push order)=[0x1458af8, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1458b08], esi ; jle taken if dword ptr [0x1458b08] <= esi  (signed less-or-equal)  -> 0x6B62A7
B6B61D4 (0x6B61D4..0x6B61EB):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1458b0c]}
    IF cmp esi, dword ptr [0x1458b08] ; jl taken if esi < dword ptr [0x1458b08]  (signed less)  -> 0x6B61D4
B6B61EB (0x6B61EB..0x6B61F0):
    JMP 0x6b62a7
B6B61F0 (0x6B61F0..0x6B61FB):
    CALL 0xB72F30   stack(push order)=[]  regs{eax=mov ebp}
    IF test al,al ; jne taken if al != 0  -> 0x6B620A
B6B61FB (0x6B61FB..0x6B620A):
    CALL 0xB72F50   stack(push order)=[]  regs{ecx=mov ebp}
    IF test al,al ; je taken if al == 0  -> 0x6B62A7
B6B620A (0x6B620A..0x6B621A):
    IF cmp dword ptr [0x2527ce8], 0x177 ; jle taken if dword ptr [0x2527ce8] <= 0x177  (signed less-or-equal)  -> 0x6B62A7
B6B621A (0x6B621A..0x6B6227):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B62A7
B6B6227 (0x6B6227..0x6B6230):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B6238
B6B6230 (0x6B6230..0x6B6238):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B62A7
B6B6238 (0x6B6238..0x6B6241):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B62A7
B6B6241 (0x6B6241..0x6B6252):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B62A7
B6B6252 (0x6B6252..0x6B625B):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B6263
B6B625B (0x6B625B..0x6B6263):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B62A7
B6B6263 (0x6B6263..0x6B6276):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B62A7
B6B6276 (0x6B6276..0x6B628F):
    CALL indirect eax  stack(push order)=[0x1458f7c, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1458f8c], esi ; jle taken if dword ptr [0x1458f8c] <= esi  (signed less-or-equal)  -> 0x6B62A7
B6B6290 (0x6B6290..0x6B62A7):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1458f90]}
    IF cmp esi, dword ptr [0x1458f8c] ; jl taken if esi < dword ptr [0x1458f8c]  (signed less)  -> 0x6B6290
B6B62A7 (0x6B62A7..0x6B62B2):
    IF cmp dword ptr [esp + 0x20], 0 ; je taken if dword ptr [esp + 0x20] == 0  (equal / zero)  -> 0x6B6357
B6B62B2 (0x6B62B2..0x6B62C2):
    IF cmp dword ptr [0x2527ce8], 0x178 ; jle taken if dword ptr [0x2527ce8] <= 0x178  (signed less-or-equal)  -> 0x6B6357
B6B62C2 (0x6B62C2..0x6B62CF):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B62CF (0x6B62CF..0x6B62D8):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B62E0
B6B62D8 (0x6B62D8..0x6B62E0):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B62E0 (0x6B62E0..0x6B62E9):
    IF cmp byte ptr [0x2527b5a], 0 ; jne taken if byte ptr [0x2527b5a] != 0  (not equal / non-zero)  -> 0x6B6357
B6B62E9 (0x6B62E9..0x6B62FA):
    CALL 0x6B6460 stats_can_award  stack(push order)=[]  regs{}
    IF test al,al ; jne taken if al != 0  -> 0x6B6357
B6B62FA (0x6B62FA..0x6B6303):
    IF cmp dword ptr [0x27b3c60], 0 ; je taken if dword ptr [0x27b3c60] == 0  (equal / zero)  -> 0x6B630B
B6B6303 (0x6B6303..0x6B630B):
    IF cmp byte ptr [0x2527c76], al ; jne taken if byte ptr [0x2527c76] != al  (not equal / non-zero)  -> 0x6B6357
B6B630B (0x6B630B..0x6B631E):
    IF test eax,eax ; je taken if eax == 0  -> 0x6B6357
B6B631E (0x6B631E..0x6B6337):
    CALL indirect eax  stack(push order)=[0x1458fc0, ecx]  regs{ecx=lea [esp + 0x18]}
    IF cmp dword ptr [0x1458fd0], esi ; jle taken if dword ptr [0x1458fd0] <= esi  (signed less-or-equal)  -> 0x6B6357
B6B6337 (0x6B6337..0x6B6339):
    JMP 0x6b6340
B6B6340 (0x6B6340..0x6B6357):
    CALL 0x6B3B30 stat_award_check  stack(push order)=[]  regs{edi=mov dword ptr [esi*4 + 0x1458fd4]}
    IF cmp esi, dword ptr [0x1458fd0] ; jl taken if esi < dword ptr [0x1458fd0]  (signed less)  -> 0x6B6340
B6B635B (0x6B635B..0x6B635D):
    RET 
```

## Map: `pcu_worn_set_from_item_list` (0x7C4280, 0x885 bytes)

Builds a worn set from an item list: alloca scratch, `_memset`, per-item label via `_sprintf`, conflict resolution, entry `_memmove`.

### pcu_worn_set_from_item_list (0x7C4280)

- **True size** 0x885 (2181) bytes by recursive descent; 697 instructions; ret imm [0]; entry regs read (linear prefix) none.
- Frame: frame sub esp,0xfc; security cookie [0xE80B50].
- **Callees** (17 call sites, 13 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0xCB35C0 | __alloca_probe_16 | 7C4315 7C4321 | ret [], reads ['eax'] |
| 0xCA0BC0 | _memset | 7C432D | ret [0], reads [] |
| 0x7BEF90 |  | 7C439D | ret [0], reads ['eax'] |
| 0xC968F6 | _sprintf | 7C44D0 | ret [0], reads [] |
| indirect | `call reg/mem` | 7C4ADB | - |
| 0xC9604F | __security_check_cookie | 7C4AFC | ret [], reads ['ecx'] |
| 0x9E8ED0 |  | 7C46AE 7C4922 | ret [0], reads [] |
| 0x7BEE40 |  | 7C46B8 7C492C | ret [0], reads ['eax'] |
| 0x7BF550 |  | 7C4936 | ret [0], reads [] |
| 0x7C5030 |  | 7C49CC | ret [0], reads ['eax', 'edi'] |
| 0x7C52B0 |  | 7C4A46 | ret [0], reads [] |
| 0x7C5460 |  | 7C4A60 | ret [0], reads ['eax'] |
| 0xC96060 | _memmove | 7C45BB 7C483B | ret [0], reads [] |
- **Globals read**: 0x21703D4, 0x25288BCx3
- **Globals written**: none
- **Stack objects (lea of a frame slot, passed by address)**: [ebp-0x108], [ebp-0x58], [ebp-0xa8], [ebp-0xf8], [esp+0x0]
- **x87 / SSE**: none.

**Section-by-section walk**

```
B7C4280 (0x7C4280..0x7C42A2):
    IF test ecx,ecx ; jle taken if ecx <= 0 (signed)  -> 0x7C430D
B7C42B0 (0x7C42B0..0x7C42B7):
    IF cmp dword ptr [edi + 0x1c], esi ; je taken if dword ptr [edi + 0x1c] == esi  (equal / zero)  -> 0x7C42C3
B7C42B7 (0x7C42B7..0x7C42C1):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7C42B0
B7C42C1 (0x7C42C1..0x7C42C3):
    JMP 0x7c430d
B7C42C3 (0x7C42C3..0x7C430D):
    rep movsd dword ptr es:[edi], dword ptr [esi]
    rep movsd dword ptr es:[edi], dword ptr [esi]
    rep movsd dword ptr es:[edi], dword ptr [esi]
B7C430D (0x7C430D..0x7C4345):
    CALL 0xCB35C0 __alloca_probe_16  stack(push order)=[]  regs{eax=mov esi esi=mov dword ptr [ebx + 0x18]}
    CALL 0xCB35C0 __alloca_probe_16  stack(push order)=[]  regs{eax=mov esi edi=mov esp}
    CALL 0xCA0BC0 _memset  stack(push order)=[edi, 0, esi]  regs{}
    IF cmp esi, edi ; jle taken if esi <= edi  (signed less-or-equal)  -> 0x7C43C7
B7C4380 (0x7C4380..0x7C4384):
    IF cmp dword ptr [ecx], edx ; je taken if dword ptr [ecx] == edx  (equal / zero)  -> 0x7C4393
B7C4384 (0x7C4384..0x7C4391):
    IF cmp eax, 0x1388 ; jl taken if eax < 0x1388  (signed less)  -> 0x7C4380
B7C4393 (0x7C4393..0x7C43A9):
    CALL 0x7BEF90   stack(push order)=[ecx, eax]  regs{eax=mov dword ptr [ebp - 0x2c]}
    IF test al,al ; jne taken if al != 0  -> 0x7C43B4
B7C43B4 (0x7C43B4..0x7C43C5):
    IF cmp edi, esi ; jl taken if edi < esi  (signed less)  -> 0x7C4366
B7C43D3 (0x7C43D3..0x7C43DC):
    IF cmp dword ptr [ebp - 0x24], esi ; jge taken if dword ptr [ebp - 0x24] >= esi  (signed greater-or-equal)  -> 0x7C4AC6
B7C43DC (0x7C43DC..0x7C43EB):
    IF test esi,esi ; jle taken if esi <= 0 (signed)  -> 0x7C4AB4
B7C43EB (0x7C43EB..0x7C43ED):
    JMP 0x7c43f0
B7C43F0 (0x7C43F0..0x7C4400):
    IF cmp byte ptr [edx + eax], 1 ; je taken if byte ptr [edx + eax] == 1  (equal / zero)  -> 0x7C4A95
B7C4400 (0x7C4400..0x7C443B):
    IF cmp dword ptr [ebp - 0x34], edx ; jne taken if dword ptr [ebp - 0x34] != edx  (not equal / non-zero)  -> 0x7C46C7
B7C443B (0x7C443B..0x7C4448):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7C4A95
B7C4448 (0x7C4448..0x7C445A):
    IF test esi,esi ; jle taken if esi <= 0 (signed)  -> 0x7C4487
B7C4460 (0x7C4460..0x7C4466):
    IF cmp ecx, edi ; jge taken if ecx >= edi  (signed greater-or-equal)  -> 0x7C447D
B7C4466 (0x7C4466..0x7C447D):
    IF cmp byte ptr [ecx + ebx], 0 ; je taken if byte ptr [ecx + ebx] == 0  (equal / zero)  -> 0x7C4A95
B7C447D (0x7C447D..0x7C4487):
    IF cmp eax, esi ; jl taken if eax < esi  (signed less)  -> 0x7C4460
B7C44A3 (0x7C44A3..0x7C44A7):
    IF cmp dword ptr [ebx], eax ; je taken if dword ptr [ebx] == eax  (equal / zero)  -> 0x7C44B7
B7C44A7 (0x7C44A7..0x7C44B5):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7C44A3
B7C44B5 (0x7C44B5..0x7C44B7):
    JMP 0x7c44be
B7C44B7 (0x7C44B7..0x7C44BE):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7C44DD
B7C44C6 (0x7C44C6..0x7C44DD):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat']  regs{}
    JMP 0x7c4a8f
B7C44DD (0x7C44DD..0x7C44F9):
    IF cmp dword ptr [edi*4 + 0x1fac828], eax ; jle taken if dword ptr [edi*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7C4519
B7C4500 (0x7C4500..0x7C4507):
    IF cmp byte ptr [esi + 0x15], dl ; je taken if byte ptr [esi + 0x15] == dl  (equal / zero)  -> 0x7C456A
B7C4507 (0x7C4507..0x7C4516):
    IF cmp eax, dword ptr [edi*4 + 0x1fac828] ; jl taken if eax < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7C4500
B7C4519 (0x7C4519..0x7C4528):
    IF cmp dword ptr [edi*4 + 0x1fac828], esi ; jle taken if dword ptr [edi*4 + 0x1fac828] <= esi  (signed less-or-equal)  -> 0x7C45E2
B7C4530 (0x7C4530..0x7C4539):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7C4558
B7C4539 (0x7C4539..0x7C4545):
    IF test dl,dl ; je taken if dl == 0  -> 0x7C4558
B7C4545 (0x7C4545..0x7C4550):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7C4558
B7C4550 (0x7C4550..0x7C4558):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7C4591
B7C4558 (0x7C4558..0x7C4568):
    IF cmp esi, dword ptr [edi*4 + 0x1fac828] ; jl taken if esi < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7C4530
B7C4568 (0x7C4568..0x7C456A):
    JMP 0x7c45e2
B7C456A (0x7C456A..0x7C4586):
    IF cmp eax, dword ptr [ebp - 8] ; jne taken if eax != dword ptr [ebp - 8]  (not equal / non-zero)  -> 0x7C4606
B7C4586 (0x7C4586..0x7C458F):
    IF cmp dword ptr [esi + 0x10], ebx ; je taken if dword ptr [esi + 0x10] == ebx  (equal / zero)  -> 0x7C46BD
B7C458F (0x7C458F..0x7C4591):
    JMP 0x7c4606
B7C4591 (0x7C4591..0x7C45A1):
    IF cmp esi, eax ; jge taken if esi >= eax  (signed greater-or-equal)  -> 0x7C45D7
B7C45B4 (0x7C45B4..0x7C45D7):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp ebx, eax ; jl taken if ebx < eax  (signed less)  -> 0x7C45B4
B7C4606 (0x7C4606..0x7C4636):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7C463B
B7C463B (0x7C463B..0x7C4642):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7C4662
B7C4642 (0x7C4642..0x7C464D):
    JMP 0x7c4650
B7C4650 (0x7C4650..0x7C4662):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7C4650
B7C4662 (0x7C4662..0x7C466D):
    IF test al,al ; je taken if al == 0  -> 0x7C4684
B7C466D (0x7C466D..0x7C4684):
    JMP 0x7c46a8
B7C4684 (0x7C4684..0x7C468A):
    IF cmp dword ptr [ebp + 0x14], 0 ; jne taken if dword ptr [ebp + 0x14] != 0  (not equal / non-zero)  -> 0x7C46A1
B7C468A (0x7C468A..0x7C4693):
    IF cmp eax, 4 ; ja taken if eax > 4  (unsigned above)  -> 0x7C469A
B7C4693 (0x7C4693..0x7C469A):
    JMP 0x7c46a4
B7C469A (0x7C469A..0x7C46A1):
    JMP 0x7c46a4
B7C46A8 (0x7C46A8..0x7C46BD):
    CALL 0x9E8ED0   stack(push order)=[edi, ebx, 2, 0]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov edi}
B7C46BD (0x7C46BD..0x7C46C7):
    JMP 0x7c4936
B7C46C7 (0x7C46C7..0x7C46D4):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7C4A95
B7C46D4 (0x7C46D4..0x7C46E0):
    IF test esi,esi ; jle taken if esi <= 0 (signed)  -> 0x7C470A
B7C46E3 (0x7C46E3..0x7C46E9):
    IF cmp eax, edi ; jge taken if eax >= edi  (signed greater-or-equal)  -> 0x7C4700
B7C46E9 (0x7C46E9..0x7C4700):
    IF cmp byte ptr [eax + ebx], 0 ; je taken if byte ptr [eax + ebx] == 0  (equal / zero)  -> 0x7C4A95
B7C4700 (0x7C4700..0x7C470A):
    IF cmp ecx, esi ; jl taken if ecx < esi  (signed less)  -> 0x7C46E3
B7C4726 (0x7C4726..0x7C472A):
    IF cmp dword ptr [ebx], eax ; je taken if dword ptr [ebx] == eax  (equal / zero)  -> 0x7C4745
B7C472A (0x7C472A..0x7C4738):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7C4726
B7C4738 (0x7C4738..0x7C4745):
    JMP 0x7c44c6
B7C4745 (0x7C4745..0x7C474C):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7C4759
B7C474C (0x7C474C..0x7C4759):
    JMP 0x7c44c6
B7C4759 (0x7C4759..0x7C4775):
    IF cmp dword ptr [edi*4 + 0x1fac828], eax ; jle taken if dword ptr [edi*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7C4799
B7C4775 (0x7C4775..0x7C477D):
    JMP 0x7c4780
B7C4780 (0x7C4780..0x7C4787):
    IF cmp byte ptr [esi + 0x15], dl ; je taken if byte ptr [esi + 0x15] == dl  (equal / zero)  -> 0x7C47EA
B7C4787 (0x7C4787..0x7C4796):
    IF cmp eax, dword ptr [edi*4 + 0x1fac828] ; jl taken if eax < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7C4780
B7C4799 (0x7C4799..0x7C47A8):
    IF cmp dword ptr [edi*4 + 0x1fac828], esi ; jle taken if dword ptr [edi*4 + 0x1fac828] <= esi  (signed less-or-equal)  -> 0x7C4862
B7C47B0 (0x7C47B0..0x7C47B9):
    IF test ecx,ecx ; je taken if ecx == 0  -> 0x7C47D8
B7C47B9 (0x7C47B9..0x7C47C5):
    IF test dl,dl ; je taken if dl == 0  -> 0x7C47D8
B7C47C5 (0x7C47C5..0x7C47D0):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7C47D8
B7C47D0 (0x7C47D0..0x7C47D8):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7C4811
B7C47D8 (0x7C47D8..0x7C47E8):
    IF cmp esi, dword ptr [edi*4 + 0x1fac828] ; jl taken if esi < dword ptr [edi*4 + 0x1fac828]  (signed less)  -> 0x7C47B0
B7C47E8 (0x7C47E8..0x7C47EA):
    JMP 0x7c4862
B7C47EA (0x7C47EA..0x7C4806):
    IF cmp ecx, dword ptr [ebp - 0xc] ; jne taken if ecx != dword ptr [ebp - 0xc]  (not equal / non-zero)  -> 0x7C4886
B7C4806 (0x7C4806..0x7C480F):
    IF cmp dword ptr [esi + 0x10], ebx ; je taken if dword ptr [esi + 0x10] == ebx  (equal / zero)  -> 0x7C4931
B7C480F (0x7C480F..0x7C4811):
    JMP 0x7c4886
B7C4811 (0x7C4811..0x7C4821):
    IF cmp esi, eax ; jge taken if esi >= eax  (signed greater-or-equal)  -> 0x7C4857
B7C4834 (0x7C4834..0x7C4857):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp ebx, eax ; jl taken if ebx < eax  (signed less)  -> 0x7C4834
B7C4886 (0x7C4886..0x7C48B6):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7C48BB
B7C48BB (0x7C48BB..0x7C48C2):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7C48E2
B7C48C2 (0x7C48C2..0x7C48CD):
    JMP 0x7c48d0
B7C48D0 (0x7C48D0..0x7C48E2):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7C48D0
B7C48E2 (0x7C48E2..0x7C48ED):
    IF test al,al ; je taken if al == 0  -> 0x7C4905
B7C48ED (0x7C48ED..0x7C4905):
    JMP 0x7c491c
B7C4905 (0x7C4905..0x7C4913):
    IF cmp eax, 4 ; jbe taken if eax <= 4  (unsigned below-or-equal)  -> 0x7C4918
B7C491C (0x7C491C..0x7C4931):
    CALL 0x9E8ED0   stack(push order)=[edi, ebx, 2, 0]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov edi}
B7C4936 (0x7C4936..0x7C4975):
    CALL 0x7BF550   stack(push order)=[]  regs{}
    IF cmp dword ptr [eax*4 + 0x1fac828], ecx ; jle taken if dword ptr [eax*4 + 0x1fac828] <= ecx  (signed less-or-equal)  -> 0x7C4A68
B7C4975 (0x7C4975..0x7C4981):
    IF cmp edi, dword ptr [ebp - 0x14] ; je taken if edi == dword ptr [ebp - 0x14]  (equal / zero)  -> 0x7C4995
B7C4981 (0x7C4981..0x7C4990):
    IF cmp ecx, dword ptr [eax*4 + 0x1fac828] ; jl taken if ecx < dword ptr [eax*4 + 0x1fac828]  (signed less)  -> 0x7C4975
B7C4990 (0x7C4990..0x7C4995):
    JMP 0x7c4a68
B7C4995 (0x7C4995..0x7C49AB):
    rep movsd dword ptr es:[edi], dword ptr [esi]
    IF cmp dword ptr [edx + 8], 0 ; je taken if dword ptr [edx + 8] == 0  (equal / zero)  -> 0x7C49D4
B7C49AB (0x7C49AB..0x7C49C1):
    IF cmp edi, 9 ; jg taken if edi > 9  (signed greater)  -> 0x7C49D4
B7C49C1 (0x7C49C1..0x7C49D4):
    CALL 0x7C5030   stack(push order)=[ebx, eax]  regs{eax=lea [ebp - 0xa8]}
B7C49D4 (0x7C49D4..0x7C49E9):
    IF test edx,edx ; je taken if edx == 0  -> 0x7C4A68
B7C49E9 (0x7C49E9..0x7C4A11):
    IF cmp dword ptr [eax*4 + 0x1fac828], ecx ; jle taken if dword ptr [eax*4 + 0x1fac828] <= ecx  (signed less-or-equal)  -> 0x7C4A68
B7C4A11 (0x7C4A11..0x7C4A1D):
    IF cmp edi, dword ptr [ebp - 0x14] ; je taken if edi == dword ptr [ebp - 0x14]  (equal / zero)  -> 0x7C4A2E
B7C4A1D (0x7C4A1D..0x7C4A2C):
    IF cmp ecx, dword ptr [eax*4 + 0x1fac828] ; jl taken if ecx < dword ptr [eax*4 + 0x1fac828]  (signed less)  -> 0x7C4A11
B7C4A2C (0x7C4A2C..0x7C4A2E):
    JMP 0x7c4a68
B7C4A2E (0x7C4A2E..0x7C4A68):
    rep movsd dword ptr es:[edi], dword ptr [esi]
    CALL 0x7C52B0   stack(push order)=[ebx, edx, edx, 0]  regs{edi=lea [ebp - 0xf8] ecx=mov 0x14 edx=lea [ebp - 0xf8]}
    CALL 0x7C5460   stack(push order)=[ebx, eax]  regs{eax=lea [ebp - 0xf8] edx=mov dword ptr [ebp - 0x10] ecx=mov dword ptr [eax + 0x14]}
B7C4A68 (0x7C4A68..0x7C4A81):
    IF cmp byte ptr [eax + edx], cl ; jne taken if byte ptr [eax + edx] != cl  (not equal / non-zero)  -> 0x7C4A84
B7C4A84 (0x7C4A84..0x7C4A8F):
    IF cmp dword ptr [ebp - 0x24], esi ; je taken if dword ptr [ebp - 0x24] == esi  (equal / zero)  -> 0x7C4AAE
B7C4A95 (0x7C4A95..0x7C4AAC):
    IF cmp eax, esi ; jl taken if eax < esi  (signed less)  -> 0x7C43F0
B7C4AAC (0x7C4AAC..0x7C4AAE):
    JMP 0x7c4ab4
B7C4AB4 (0x7C4AB4..0x7C4AC6):
    IF cmp eax, 4 ; jl taken if eax < 4  (signed less)  -> 0x7C43D3
B7C4AC6 (0x7C4AC6..0x7C4ACC):
    IF cmp dword ptr [ebp - 0x28], 0 ; jne taken if dword ptr [ebp - 0x28] != 0  (not equal / non-zero)  -> 0x7C4AE0
B7C4ACC (0x7C4ACC..0x7C4AD3):
    IF test eax,eax ; je taken if eax == 0  -> 0x7C4AE0
B7C4AD3 (0x7C4AD3..0x7C4AE0):
    CALL indirect eax  stack(push order)=[edx, -1, 0]  regs{edx=mov dword ptr [ebp + 8]}
B7C4AE0 (0x7C4AE0..0x7C4B05):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor ebp eax=0 edx=mov dword ptr [ebp - 0x24]}
    RET 
```

## Map: `car_dealership_parse_record` (0x7A65C0, 0x7A2 bytes)

XML record parser (string compares on field names, hashing, `_atol`, `_strncpy`).

### car_dealership_parse_record (0x7A65C0)

- **True size** 0x7A2 (1954) bytes by recursive descent; 666 instructions; ret imm [0]; entry regs read (linear prefix) ['ecx'].
- Frame: frame sub esp,0xb0; security cookie [0xE80B50].
- **Callees** (37 call sites, 11 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0xC99030 | __stricmp | 7A6608 7A66DA 7A6788 7A67CB 7A6823 7A6A08 7A6A7E 7A6ACB ... | ret [0], reads [] |
| 0xBDCB30 | string_hash_lower | 7A6678 7A6B0B 7A6BC1 | ret [4], reads ['eax', 'ecx'] |
| indirect | `call reg/mem` | 7A6730 7A6A66 7A6885 | - |
| 0xB747D0 | xtbl_get_int | 7A675C | ret [0], reads ['ecx'] |
| 0xC9604F | __security_check_cookie | 7A6D56 | ret [], reads ['ecx'] |
| 0xBFC110 | xml_count_children | 7A6A3D | ret [0], reads ['eax'] |
| 0xC985A0 | _atol_2 | 7A6C4A | ret [], reads [] |
| 0xBDCA30 | string_crc32 | 7A6C5B | ret [4], reads [] |
| 0xC990F0 | _strncpy | 7A6BA3 7A6B54 7A6AF3 7A694B 7A6900 7A6702 | ret [0], reads [] |
| 0xAC7FC0 |  | 7A6982 | ret [0], reads ['eax'] |
| 0xC968F6 | _sprintf | 7A69A4 | ret [0], reads [] |
- **Globals read**: 0x252A0D8, 0x252A078, 0x252A07C, 0x252A080, 0x252A084
- **Globals written**: none
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x0], [esp+0x28], [esp+0x2c], [esp+0x34], [esp+0x38], [esp+0x3c], [esp+0x40], [esp+0x7c], [esp+0x80]
- **x87 sequence** (2 instructions; `fstp/fst dword` = f32 rounding point):

```
7A6761 fild dword ptr [esp + 0x34]
7A676D fstp dword ptr [ebx + eax + 0x18]   <== f32 store
```

**Section-by-section walk**

```
B7A65C0 (0x7A65C0..0x7A65F2):
    IF test eax,eax ; je taken if eax == 0  -> 0x7A661B
B7A65F2 (0x7A65F2..0x7A65F9):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A661B
B7A6600 (0x7A6600..0x7A6614):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xdff94c 'Element']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A661D
B7A6614 (0x7A6614..0x7A661B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6600
B7A661D (0x7A661D..0x7A6629):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6D4C
B7A6630 (0x7A6630..0x7A663C):
    IF test ebp,ebp ; jne taken if ebp != 0  -> 0x7A6640
B7A663C (0x7A663C..0x7A6640):
    JMP 0x7a6671
B7A6640 (0x7A6640..0x7A6647):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A666A
B7A6647 (0x7A6647..0x7A665B):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xddaec4 'Vehicle']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6666
B7A665B (0x7A665B..0x7A6662):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6647
B7A6662 (0x7A6662..0x7A6666):
    JMP 0x7a6671
B7A6666 (0x7A6666..0x7A666A):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A666E
B7A666A (0x7A666A..0x7A666E):
    JMP 0x7a6671
B7A6671 (0x7A6671..0x7A6689):
    CALL 0xBDCB30 string_hash_lower  stack(push order)=[edx]  regs{eax=0 edx=lea [esp + 0x2c]}
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x7A66A7
B7A6692 (0x7A6692..0x7A669A):
    IF cmp dword ptr [ecx + 0x98], esi ; je taken if dword ptr [ecx + 0x98] == esi  (equal / zero)  -> 0x7A66A9
B7A669A (0x7A669A..0x7A66A7):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7A6692
B7A66A9 (0x7A66A9..0x7A66CB):
    IF test ebp,ebp ; je taken if ebp == 0  -> 0x7A670F
B7A66CB (0x7A66CB..0x7A66D2):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A670F
B7A66D2 (0x7A66D2..0x7A66E6):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xddaecc 'Variant']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A66EF
B7A66E6 (0x7A66E6..0x7A66ED):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A66D2
B7A66ED (0x7A66ED..0x7A66EF):
    JMP 0x7a670f
B7A66EF (0x7A66EF..0x7A66F3):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A670F
B7A66F3 (0x7A66F3..0x7A66FA):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A670F
B7A66FA (0x7A66FA..0x7A670F):
    CALL 0xC990F0 _strncpy  stack(push order)=[edx, esi, 0x40]  regs{edx=lea [esp + 0x40]}
B7A6716 (0x7A6716..0x7A671F):
    IF test cl,cl ; jne taken if cl != 0  -> 0x7A6716
B7A671F (0x7A671F..0x7A6744):
    CALL indirect eax  stack(push order)=[eax, 1]  regs{eax=mov dword ptr [edx + 0x1c] edx=mov dword ptr [ecx] ecx=mov dword ptr [esp + 0x18]}
B7A6744 (0x7A6744..0x7A6752):
    IF test al,al ; jne taken if al != 0  -> 0x7A6744
B7A6752 (0x7A6752..0x7A6773):
    CALL 0xB747D0 xtbl_get_int  stack(push order)=[ebp]  regs{esi=lea [esp + 0x34] ecx=mov 0xe00e18 'Cost'}
    f32 store dword ptr [ebx + eax + 0x18]
    IF test ebp,ebp ; je taken if ebp == 0  -> 0x7A679F
B7A6773 (0x7A6773..0x7A677A):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A679F
B7A6780 (0x7A6780..0x7A6798):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xe1f514 'StockOptions']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6834
B7A6798 (0x7A6798..0x7A679F):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6780
B7A67AF (0x7A67AF..0x7A67B6):
    IF cmp dword ptr [esp + 0x14], 0 ; je taken if dword ptr [esp + 0x14] == 0  (equal / zero)  -> 0x7A67E2
B7A67B6 (0x7A67B6..0x7A67C1):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A67E2
B7A67C3 (0x7A67C3..0x7A67DB):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xe1f55c 'ColorData']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A69F1
B7A67DB (0x7A67DB..0x7A67E2):
    IF cmp esi, ebp ; jne taken if esi != ebp  (not equal / non-zero)  -> 0x7A67C3
B7A67F6 (0x7A67F6..0x7A6805):
    IF cmp dword ptr [esp + 0x28], 0 ; je taken if dword ptr [esp + 0x28] == 0  (equal / zero)  -> 0x7A6D33
B7A6805 (0x7A6805..0x7A680D):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6D33
B7A6810 (0x7A6810..0x7A681B):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6D33
B7A681B (0x7A681B..0x7A682F):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xdff94c 'Element']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x7A6810
B7A682F (0x7A682F..0x7A6834):
    JMP 0x7a6d35
B7A6834 (0x7A6834..0x7A683C):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A679F
B7A683C (0x7A683C..0x7A6845):
    IF test edi,edi ; je taken if edi == 0  -> 0x7A6863
B7A6845 (0x7A6845..0x7A6859):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe0d79c 'Option']  regs{edx=mov dword ptr [edi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x7A685C
B7A685C (0x7A685C..0x7A6863):
    IF test edi,edi ; jne taken if edi != 0  -> 0x7A6845
B7A6863 (0x7A6863..0x7A6895):
    CALL indirect edx  stack(push order)=[eax, 4]  regs{edx=mov dword ptr [edx + 0x1c] eax=mov dword ptr [ebx + eax + 0xc] ecx=mov dword ptr [esp + 0x18] edi=mov dword ptr [esp + 0x10]}
    IF test esi,esi ; je taken if esi == 0  -> 0x7A68B0
B7A6895 (0x7A6895..0x7A68A9):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe0d79c 'Option']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A68B2
B7A68A9 (0x7A68A9..0x7A68B0):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6895
B7A68B2 (0x7A68B2..0x7A68BC):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A67AF
B7A68C0 (0x7A68C0..0x7A68C8):
    IF test edi,edi ; je taken if edi == 0  -> 0x7A695B
B7A68C8 (0x7A68C8..0x7A68CF):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A690D
B7A68D0 (0x7A68D0..0x7A68E4):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xdd9224 'Slot']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A68ED
B7A68E4 (0x7A68E4..0x7A68EB):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A68D0
B7A68EB (0x7A68EB..0x7A68ED):
    JMP 0x7a690d
B7A68ED (0x7A68ED..0x7A68F1):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A690D
B7A68F1 (0x7A68F1..0x7A68F8):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A690D
B7A68F8 (0x7A68F8..0x7A690D):
    CALL 0xC990F0 _strncpy  stack(push order)=[ecx, esi, 0x40]  regs{ecx=lea [esp + 0x40]}
B7A690D (0x7A690D..0x7A6911):
    IF test edi,edi ; je taken if edi == 0  -> 0x7A695B
B7A6911 (0x7A6911..0x7A6918):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A695B
B7A6918 (0x7A6918..0x7A692C):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xddaecc 'Variant']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6935
B7A692C (0x7A692C..0x7A6933):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6918
B7A6933 (0x7A6933..0x7A6935):
    JMP 0x7a695b
B7A6935 (0x7A6935..0x7A6939):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A695B
B7A6939 (0x7A6939..0x7A6940):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A695B
B7A6940 (0x7A6940..0x7A695B):
    CALL 0xC990F0 _strncpy  stack(push order)=[eax, esi, 0x40]  regs{eax=lea [esp + 0x80]}
B7A695B (0x7A695B..0x7A6990):
    CALL 0xAC7FC0   stack(push order)=[edx]  regs{ecx=lea [esp + 0x40] eax=mov dword ptr [eax + 0x74] edx=lea [esp + 0x7c]}
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A69AC
B7A6990 (0x7A6990..0x7A69AC):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe1f524 'Unable to parse a component for slot %s ', ecx, eax]  regs{ecx=lea [esp + 0x40] eax=lea [esp + 0x7c]}
B7A69AC (0x7A69AC..0x7A69C0):
    IF je (flags from before the block)  -> 0x7A69DD
B7A69C0 (0x7A69C0..0x7A69C7):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A69DD
B7A69C7 (0x7A69C7..0x7A69DB):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe0d79c 'Option']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x7A69C0
B7A69DB (0x7A69DB..0x7A69DD):
    JMP 0x7a69df
B7A69DF (0x7A69DF..0x7A69EC):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A68C0
B7A69EC (0x7A69EC..0x7A69F1):
    JMP 0x7a67af
B7A69F1 (0x7A69F1..0x7A69F9):
    IF cmp esi, ebp ; je taken if esi == ebp  (equal / zero)  -> 0x7A67E2
B7A69F9 (0x7A69F9..0x7A6A00):
    IF cmp esi, ebp ; je taken if esi == ebp  (equal / zero)  -> 0x7A6A1B
B7A6A00 (0x7A6A00..0x7A6A14):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xe1f568 'Colors']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6A32
B7A6A14 (0x7A6A14..0x7A6A1B):
    IF cmp esi, ebp ; jne taken if esi != ebp  (not equal / non-zero)  -> 0x7A6A00
B7A6A1B (0x7A6A1B..0x7A6A32):
    JMP 0x7a67f6
B7A6A32 (0x7A6A32..0x7A6A36):
    IF cmp esi, ebp ; je taken if esi == ebp  (equal / zero)  -> 0x7A6A1B
B7A6A36 (0x7A6A36..0x7A6A76):
    CALL 0xBFC110 xml_count_children  stack(push order)=[0xddc6dc 'Color']  regs{eax=mov esi}
    CALL indirect edx  stack(push order)=[eax, 4]  regs{edx=mov dword ptr [edx + 0x1c] eax=mov dword ptr [ebx + eax + 0x14] ecx=mov dword ptr [esp + 0x1c] edi=mov dword ptr [esp + 0x14]}
    IF cmp esi, ebp ; je taken if esi == ebp  (equal / zero)  -> 0x7A6A91
B7A6A76 (0x7A6A76..0x7A6A8A):
    CALL 0xC99030 __stricmp  stack(push order)=[edx, 0xddc6dc 'Color']  regs{edx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6A93
B7A6A8A (0x7A6A8A..0x7A6A91):
    IF cmp esi, ebp ; jne taken if esi != ebp  (not equal / non-zero)  -> 0x7A6A76
B7A6A93 (0x7A6A93..0x7A6A9D):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A67F6
B7A6AA5 (0x7A6AA5..0x7A6ABC):
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x7A6B00
B7A6ABC (0x7A6ABC..0x7A6AC3):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B00
B7A6AC3 (0x7A6AC3..0x7A6AD7):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xe1f570 'ColorSlot']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6AE0
B7A6AD7 (0x7A6AD7..0x7A6ADE):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6AC3
B7A6ADE (0x7A6ADE..0x7A6AE0):
    JMP 0x7a6b00
B7A6AE0 (0x7A6AE0..0x7A6AE4):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B00
B7A6AE4 (0x7A6AE4..0x7A6AEB):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B00
B7A6AEB (0x7A6AEB..0x7A6B00):
    CALL 0xC990F0 _strncpy  stack(push order)=[ecx, esi, 0x40]  regs{ecx=lea [esp + 0x40]}
B7A6B00 (0x7A6B00..0x7A6B1D):
    CALL 0xBDCB30 string_hash_lower  stack(push order)=[edx]  regs{ecx=lea [esp + 0x40] eax=0 edx=lea [esp + 0x38]}
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x7A6BB3
B7A6B1D (0x7A6B1D..0x7A6B24):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B61
B7A6B24 (0x7A6B24..0x7A6B38):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xdd92d4 'Type']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6B41
B7A6B38 (0x7A6B38..0x7A6B3F):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6B24
B7A6B3F (0x7A6B3F..0x7A6B41):
    JMP 0x7a6b61
B7A6B41 (0x7A6B41..0x7A6B45):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B61
B7A6B45 (0x7A6B45..0x7A6B4C):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6B61
B7A6B4C (0x7A6B4C..0x7A6B61):
    CALL 0xC990F0 _strncpy  stack(push order)=[edx, esi, 0x40]  regs{edx=lea [esp + 0x40]}
B7A6B61 (0x7A6B61..0x7A6B65):
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x7A6BB3
B7A6B65 (0x7A6B65..0x7A6B6C):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6BB3
B7A6B70 (0x7A6B70..0x7A6B84):
    CALL 0xC99030 __stricmp  stack(push order)=[eax, 0xdc5804 'Name']  regs{eax=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6B8D
B7A6B84 (0x7A6B84..0x7A6B8B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6B70
B7A6B8B (0x7A6B8B..0x7A6B8D):
    JMP 0x7a6bb3
B7A6B8D (0x7A6B8D..0x7A6B91):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6BB3
B7A6B91 (0x7A6B91..0x7A6B98):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6BB3
B7A6B98 (0x7A6B98..0x7A6BB3):
    CALL 0xC990F0 _strncpy  stack(push order)=[ecx, esi, 0x40]  regs{ecx=lea [esp + 0x80]}
B7A6BB3 (0x7A6BB3..0x7A6BD2):
    CALL 0xBDCB30 string_hash_lower  stack(push order)=[edx]  regs{ecx=lea [esp + 0x80] eax=0 edx=lea [esp + 0x34]}
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x7A6BEE
B7A6BE0 (0x7A6BE0..0x7A6BE4):
    IF cmp dword ptr [ecx], esi ; je taken if dword ptr [ecx] == esi  (equal / zero)  -> 0x7A6BFB
B7A6BE4 (0x7A6BE4..0x7A6BEE):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7A6BE0
B7A6BF0 (0x7A6BF0..0x7A6BF7):
    IF test ebx,ebx ; jne taken if ebx != 0  -> 0x7A6C03
B7A6BF7 (0x7A6BF7..0x7A6BFB):
    JMP 0x7a6c38
B7A6BFB (0x7A6BFB..0x7A6C03):
    JMP 0x7a6bf0
B7A6C03 (0x7A6C03..0x7A6C0A):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6C31
B7A6C10 (0x7A6C10..0x7A6C24):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xdea970 'Index']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6C2D
B7A6C24 (0x7A6C24..0x7A6C2B):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6C10
B7A6C2B (0x7A6C2B..0x7A6C2D):
    JMP 0x7a6c38
B7A6C2D (0x7A6C2D..0x7A6C31):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6C35
B7A6C31 (0x7A6C31..0x7A6C35):
    JMP 0x7a6c38
B7A6C38 (0x7A6C38..0x7A6C45):
    IF cmp al, 0x30 ; jl taken if al < 0x30  (signed less)  -> 0x7A6C54
B7A6C45 (0x7A6C45..0x7A6C49):
    IF cmp al, 0x39 ; jg taken if al > 0x39  (signed greater)  -> 0x7A6C54
B7A6C49 (0x7A6C49..0x7A6C54):
    CALL 0xC985A0 _atol_2  stack(push order)=[esi]  regs{}
    JMP 0x7a6cbd
B7A6C54 (0x7A6C54..0x7A6C6C):
    CALL 0xBDCA30 string_crc32  stack(push order)=[esi]  regs{eax=lea [esp + 0x28] edx=0}
    IF test edx,edx ; jle taken if edx <= 0 (signed)  -> 0x7A6CC0
B7A6C80 (0x7A6C80..0x7A6C84):
    IF cmp dword ptr [ecx], esi ; je taken if dword ptr [ecx] == esi  (equal / zero)  -> 0x7A6C90
B7A6C84 (0x7A6C84..0x7A6C8E):
    IF cmp eax, edx ; jl taken if eax < edx  (signed less)  -> 0x7A6C80
B7A6C8E (0x7A6C8E..0x7A6C90):
    JMP 0x7a6cc0
B7A6C90 (0x7A6C90..0x7A6CA0):
    IF test eax,eax ; je taken if eax == 0  -> 0x7A6CC0
B7A6CC0 (0x7A6CC0..0x7A6CD4):
    repe cmpsb byte ptr [esi], byte ptr es:[edi]
    IF xor edx, edx ; jne taken on not equal / non-zero of the result  -> 0x7A6CD9
B7A6CD4 (0x7A6CD4..0x7A6CD9):
    JMP 0x7a6cf4
B7A6CD9 (0x7A6CD9..0x7A6CED):
    repe cmpsb byte ptr [esi], byte ptr es:[edi]
    IF xor eax, eax ; jne taken on not equal / non-zero of the result  -> 0x7A6CF4
B7A6CF4 (0x7A6CF4..0x7A6CFA):
    IF test ebx,ebx ; je taken if ebx == 0  -> 0x7A6D1D
B7A6D00 (0x7A6D00..0x7A6D07):
    IF test esi,esi ; je taken if esi == 0  -> 0x7A6D1D
B7A6D07 (0x7A6D07..0x7A6D1B):
    CALL 0xC99030 __stricmp  stack(push order)=[ecx, 0xddc6dc 'Color']  regs{ecx=mov dword ptr [esi]}
    IF test eax,eax ; jne taken if eax != 0  -> 0x7A6D00
B7A6D1B (0x7A6D1B..0x7A6D1D):
    JMP 0x7a6d1f
B7A6D1F (0x7A6D1F..0x7A6D2E):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6AA5
B7A6D2E (0x7A6D2E..0x7A6D33):
    JMP 0x7a67f6
B7A6D35 (0x7A6D35..0x7A6D49):
    IF test esi,esi ; jne taken if esi != 0  -> 0x7A6630
B7A6D4C (0x7A6D4C..0x7A6D62):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 
```

## Map: `pcu_purchase_slot_apply` (0x7C1260, 0x79D bytes)

Applies a customisation purchase to the worn set; two near-identical halves (item with and without variant).

### pcu_purchase_slot_apply (0x7C1260)

- **True size** 0x79D (1949) bytes by recursive descent; 586 instructions; ret imm [0]; entry regs read (linear prefix) none.
- Frame: esp aligned (and esp,-8) with ebp frame; frame sub esp,0x84.
- **Callees** (20 call sites, 10 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0x7C21B0 |  | 7C1313 7C1357 7C15CB | ret [0], reads ['eax'] |
| 0x7C1D40 |  | 7C13FC 7C1474 | ret [0], reads ['ecx'] |
| 0xC968F6 | _sprintf | 7C175B 7C14D2 | ret [0], reads [] |
| 0x9E8ED0 |  | 7C19E0 7C172D | ret [0], reads [] |
| 0x7BEE40 |  | 7C19EA 7C1737 | ret [0], reads ['eax'] |
| 0x7BF550 |  | 7C1831 7C15AB | ret [0], reads [] |
| 0x9DDA80 |  | 7C1851 | ret [0], reads [] |
| 0x7C5030 |  | 7C187B 7C15F1 | ret [0], reads ['eax', 'edi'] |
| 0x7CE320 |  | 7C1891 7C1607 | ret [], reads [] |
| 0xC96060 | _memmove | 7C18F7 7C1647 | ret [0], reads [] |
- **Globals read**: 0x25297B8, 0x25288E0x2, 0x25288DC, 0x21703D4x3, 0x21603C8, 0x25288BCx2
- **Globals written**: none
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x0], [esp+0x40], [esp+0x48]
- **x87 sequence** (4 instructions; `fstp/fst dword` = f32 rounding point):

```
7C1291 fld dword ptr [eax + ecx*4 + 4]
7C1295 fstp dword ptr [esp + 0x2c]   <== f32 store
7C1299 fld dword ptr [0xdd1c8c]
7C129F fcomp dword ptr [esp + 0x2c]
```

**Section-by-section walk**

```
B7C1260 (0x7C1260..0x7C127E):
    IF cmp eax, edx ; je taken if eax == edx  (equal / zero)  -> 0x7C12AA
B7C127E (0x7C127E..0x7C1288):
    IF cmp ecx, edx ; je taken if ecx == edx  (equal / zero)  -> 0x7C12AA
B7C1288 (0x7C1288..0x7C12AA):
    f32 store dword ptr [esp + 0x2c]
    IF fcomp dword ptr [esp + 0x2c] ; test ah,0x41 ; jne taken when ST0 ? operand is in {lt, eq, un}  (gt = ST0 > operand, lt, eq, un = unordered)  -> 0x7C12B4
B7C12AA (0x7C12AA..0x7C12B4):
    JMP 0x7c12b8
B7C12B8 (0x7C12B8..0x7C12C3):
    IF cmp eax, edx ; je taken if eax == edx  (equal / zero)  -> 0x7C12C5
B7C12C5 (0x7C12C5..0x7C12DF):
    IF cmp dword ptr [0x25288e0], edx ; jle taken if dword ptr [0x25288e0] <= edx  (signed less-or-equal)  -> 0x7C18C4
B7C12DF (0x7C12DF..0x7C12FA):
    IF cmp eax, 1 ; ja taken if eax > 1  (unsigned above)  -> 0x7C18A8
B7C12FA (0x7C12FA..0x7C131F):
    CALL 0x7C21B0   stack(push order)=[edx]  regs{edx=mov 0x29 eax=mov dword ptr [ecx + 0x13b0] ecx=mov dword ptr [ebp + 8]}
    IF cmp al, 1 ; jne taken if al != 1  (not equal / non-zero)  -> 0x7C1339
B7C131F (0x7C131F..0x7C132F):
    IF cmp ecx, edx ; jl taken if ecx < edx  (signed less)  -> 0x7C1339
B7C132F (0x7C132F..0x7C1337):
    IF cmp ecx, eax ; jg taken if ecx > eax  (signed greater)  -> 0x7C1339
B7C1339 (0x7C1339..0x7C134A):
    IF cmp dword ptr [edx + 0x50], ecx ; jne taken if dword ptr [edx + 0x50] != ecx  (not equal / non-zero)  -> 0x7C18A8
B7C134A (0x7C134A..0x7C1367):
    CALL 0x7C21B0   stack(push order)=[eax]  regs{eax=mov dword ptr [esp + 0x1c] edx=mov dword ptr [esp + 0x30]}
    IF test al,al ; jne taken if al != 0  -> 0x7C18A8
B7C1367 (0x7C1367..0x7C136F):
    IF test bl,bl ; jne taken if bl != 0  -> 0x7C18A8
B7C136F (0x7C136F..0x7C138E):
    IF test ecx,ecx ; jle taken if ecx <= 0 (signed)  -> 0x7C13C3
B7C1393 (0x7C1393..0x7C139E):
    IF cmp dword ptr [ebp + 8], edi ; jne taken if dword ptr [ebp + 8] != edi  (not equal / non-zero)  -> 0x7C13B9
B7C139E (0x7C139E..0x7C13A2):
    IF test eax,eax ; jl taken if eax < 0 (signed)  -> 0x7C13B9
B7C13A2 (0x7C13A2..0x7C13A6):
    IF cmp eax, edx ; jge taken if eax >= edx  (signed greater-or-equal)  -> 0x7C13B9
B7C13A6 (0x7C13A6..0x7C13AA):
    IF test esi,esi ; je taken if esi == 0  -> 0x7C13B9
B7C13AA (0x7C13AA..0x7C13B9):
    IF cmp edi, dword ptr [esp + 0x30] ; je taken if edi == dword ptr [esp + 0x30]  (equal / zero)  -> 0x7C143F
B7C13B9 (0x7C13B9..0x7C13C3):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7C1393
B7C13C3 (0x7C13C3..0x7C13E3):
    IF je (flags from before the block)  -> 0x7C18A8
B7C13E3 (0x7C13E3..0x7C140C):
    CALL 0x7C1D40   stack(push order)=[edx]  regs{edi=mov dword ptr [0x25288bc] ecx=mov dword ptr [esp + 0x1c] edx=mov dword ptr [ebx*4 + 0x1fac7f8]}
    IF test al,al ; je taken if al == 0  -> 0x7C18A8
B7C1424 (0x7C1424..0x7C142C):
    IF cmp dword ptr [edx], eax ; je taken if dword ptr [edx] == eax  (equal / zero)  -> 0x7C1741
B7C142C (0x7C142C..0x7C143A):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7C1424
B7C143A (0x7C143A..0x7C143F):
    JMP 0x7c1749
B7C143F (0x7C143F..0x7C145F):
    IF je (flags from before the block)  -> 0x7C15B7
B7C145F (0x7C145F..0x7C1484):
    CALL 0x7C1D40   stack(push order)=[eax]  regs{edi=mov dword ptr [0x25288bc] eax=mov dword ptr [ebx*4 + 0x1fac7f8] edx=mov dword ptr [esp + 0x18]}
    IF test al,al ; je taken if al == 0  -> 0x7C15B7
B7C14A0 (0x7C14A0..0x7C14A4):
    IF cmp dword ptr [edx], eax ; je taken if dword ptr [edx] == eax  (equal / zero)  -> 0x7C14B4
B7C14A4 (0x7C14A4..0x7C14B2):
    IF cmp ecx, 0x1388 ; jl taken if ecx < 0x1388  (signed less)  -> 0x7C14A0
B7C14B2 (0x7C14B2..0x7C14B4):
    JMP 0x7c14bc
B7C14B4 (0x7C14B4..0x7C14BC):
    IF test edx,edx ; jne taken if edx != 0  -> 0x7C14DF
B7C14BC (0x7C14BC..0x7C14DF):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', eax, edx]  regs{eax=mov dword ptr [edi + 8] edx=mov dword ptr [ecx + 0xc] ecx=mov dword ptr [esp + 0x20]}
    JMP 0x7c15b7
B7C14DF (0x7C14DF..0x7C14FC):
    IF cmp dword ptr [ebx*4 + 0x1fac828], eax ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7C151C
B7C1506 (0x7C1506..0x7C150D):
    IF cmp byte ptr [edi + 0x15], cl ; je taken if byte ptr [edi + 0x15] == cl  (equal / zero)  -> 0x7C157D
B7C150D (0x7C150D..0x7C151C):
    IF cmp eax, dword ptr [ebx*4 + 0x1fac828] ; jl taken if eax < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7C1506
B7C151C (0x7C151C..0x7C152B):
    IF cmp dword ptr [ebx*4 + 0x1fac828], edi ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= edi  (signed less-or-equal)  -> 0x7C166B
B7C1540 (0x7C1540..0x7C1546):
    IF test esi,esi ; je taken if esi == 0  -> 0x7C1569
B7C1546 (0x7C1546..0x7C1552):
    IF test cl,cl ; je taken if cl == 0  -> 0x7C1569
B7C1552 (0x7C1552..0x7C155D):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7C1569
B7C155D (0x7C155D..0x7C1569):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7C1619
B7C1569 (0x7C1569..0x7C1578):
    IF cmp edi, dword ptr [ebx*4 + 0x1fac828] ; jl taken if edi < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7C1540
B7C1578 (0x7C1578..0x7C157D):
    JMP 0x7c166b
B7C157D (0x7C157D..0x7C159C):
    IF cmp dword ptr [esi], edi ; jne taken if dword ptr [esi] != edi  (not equal / non-zero)  -> 0x7C1697
B7C159C (0x7C159C..0x7C15A5):
    IF cmp dword ptr [esi + 0x10], edx ; jne taken if dword ptr [esi + 0x10] != edx  (not equal / non-zero)  -> 0x7C1697
B7C15A9 (0x7C15A9..0x7C15B7):
    CALL 0x7BF550   stack(push order)=[ebx, esi]  regs{}
B7C15B7 (0x7C15B7..0x7C15E3):
    CALL 0x7C21B0   stack(push order)=[eax]  regs{eax=mov ebx edx=mov dword ptr [esp + 0x30]}
    IF cmp dword ptr [esi + edi*4 + 8], 0 ; je taken if dword ptr [esi + edi*4 + 8] == 0  (equal / zero)  -> 0x7C19F4
B7C15E3 (0x7C15E3..0x7C15E8):
    IF cmp edi, 9 ; jg taken if edi > 9  (signed greater)  -> 0x7C15F9
B7C15E8 (0x7C15E8..0x7C15F9):
    CALL 0x7C5030   stack(push order)=[ebx, esi]  regs{eax=lea [esp + 0x48]}
B7C15F9 (0x7C15F9..0x7C1619):
    CALL 0x7CE320   stack(push order)=[ecx, 0]  regs{eax=mov dword ptr [edx + 0x10] edx=mov dword ptr [esp + 0x14]}
    JMP 0x7c18a8
B7C1619 (0x7C1619..0x7C1627):
    IF cmp edi, eax ; jge taken if edi >= eax  (signed greater-or-equal)  -> 0x7C1663
B7C1640 (0x7C1640..0x7C1663):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp edi, eax ; jl taken if edi < eax  (signed less)  -> 0x7C1640
B7C1697 (0x7C1697..0x7C16C7):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7C16CC
B7C16CC (0x7C16CC..0x7C16D3):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7C16F4
B7C16E0 (0x7C16E0..0x7C16F2):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7C16E0
B7C16F4 (0x7C16F4..0x7C1703):
    IF test cl,cl ; je taken if cl == 0  -> 0x7C1714
B7C1703 (0x7C1703..0x7C1714):
    JMP 0x7c1728
B7C1714 (0x7C1714..0x7C1722):
    IF cmp ecx, 4 ; jbe taken if ecx <= 4  (unsigned below-or-equal)  -> 0x7C1727
B7C1728 (0x7C1728..0x7C1741):
    CALL 0x9E8ED0   stack(push order)=[ebx, eax, 2, edx]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov ebx}
    JMP 0x7c15a9
B7C1741 (0x7C1741..0x7C1749):
    IF test edx,edx ; jne taken if edx != 0  -> 0x7C1768
B7C1749 (0x7C1749..0x7C1768):
    CALL 0xC968F6 _sprintf  stack(push order)=[0x23460e0, 0xe20e18 'Could not find player component informat', ecx, eax]  regs{ecx=mov dword ptr [edi + 8] eax=mov dword ptr [esi + 0xc]}
    JMP 0x7c18a8
B7C1768 (0x7C1768..0x7C1785):
    IF cmp dword ptr [ebx*4 + 0x1fac828], eax ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= eax  (signed less-or-equal)  -> 0x7C17A6
B7C1790 (0x7C1790..0x7C1797):
    IF cmp byte ptr [edi + 0x15], cl ; je taken if byte ptr [edi + 0x15] == cl  (equal / zero)  -> 0x7C1805
B7C1797 (0x7C1797..0x7C17A6):
    IF cmp eax, dword ptr [ebx*4 + 0x1fac828] ; jl taken if eax < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7C1790
B7C17A6 (0x7C17A6..0x7C17B5):
    IF cmp dword ptr [ebx*4 + 0x1fac828], edi ; jle taken if dword ptr [ebx*4 + 0x1fac828] <= edi  (signed less-or-equal)  -> 0x7C191B
B7C17C8 (0x7C17C8..0x7C17CE):
    IF test esi,esi ; je taken if esi == 0  -> 0x7C17F1
B7C17CE (0x7C17CE..0x7C17DA):
    IF test cl,cl ; je taken if cl == 0  -> 0x7C17F1
B7C17DA (0x7C17DA..0x7C17E5):
    IF cmp eax, ecx ; jl taken if eax < ecx  (signed less)  -> 0x7C17F1
B7C17E5 (0x7C17E5..0x7C17F1):
    IF cmp eax, ecx ; jle taken if eax <= ecx  (signed less-or-equal)  -> 0x7C18CF
B7C17F1 (0x7C17F1..0x7C1800):
    IF cmp edi, dword ptr [ebx*4 + 0x1fac828] ; jl taken if edi < dword ptr [ebx*4 + 0x1fac828]  (signed less)  -> 0x7C17C8
B7C1800 (0x7C1800..0x7C1805):
    JMP 0x7c191b
B7C1805 (0x7C1805..0x7C1822):
    IF cmp dword ptr [esi], edi ; jne taken if dword ptr [esi] != edi  (not equal / non-zero)  -> 0x7C1947
B7C1822 (0x7C1822..0x7C182B):
    IF cmp dword ptr [esi + 0x10], edx ; jne taken if dword ptr [esi + 0x10] != edx  (not equal / non-zero)  -> 0x7C1947
B7C182F (0x7C182F..0x7C1848):
    CALL 0x7BF550   stack(push order)=[ebx, esi]  regs{}
    IF cmp edx, dword ptr [0x21703d4] ; jne taken if edx != dword ptr [0x21703d4]  (not equal / non-zero)  -> 0x7C1859
B7C1848 (0x7C1848..0x7C1859):
    CALL 0x9DDA80   stack(push order)=[ecx, eax, 0]  regs{ecx=mov dword ptr [esi] eax=mov dword ptr [esi + 0xc]}
B7C1859 (0x7C1859..0x7C185F):
    IF cmp dword ptr [esi + 0x38], 0 ; je taken if dword ptr [esi + 0x38] == 0  (equal / zero)  -> 0x7C1887
B7C185F (0x7C185F..0x7C1869):
    IF cmp edi, 9 ; jg taken if edi > 9  (signed greater)  -> 0x7C1887
B7C1869 (0x7C1869..0x7C1887):
    CALL 0x7C5030   stack(push order)=[eax, eax]  regs{eax=mov dword ptr [esp + 0x1c] edx=mov dword ptr [ebp + 8]}
B7C1887 (0x7C1887..0x7C18A5):
    CALL 0x7CE320   stack(push order)=[edx, 0]  regs{ecx=mov dword ptr [esi + 0x10]}
    IF test eax,eax ; je taken if eax == 0  -> 0x7C18A8
B7C18A8 (0x7C18A8..0x7C18C4):
    IF cmp eax, dword ptr [0x25288e0] ; jl taken if eax < dword ptr [0x25288e0]  (signed less)  -> 0x7C12DF
B7C18C4 (0x7C18C4..0x7C18CF):
    RET 
B7C18CF (0x7C18CF..0x7C18DD):
    IF cmp edi, eax ; jge taken if edi >= eax  (signed greater-or-equal)  -> 0x7C1913
B7C18F0 (0x7C18F0..0x7C1913):
    CALL 0xC96060 _memmove  stack(push order)=[esi, edx, 0x50]  regs{edx=lea [esi + 0x50]}
    IF cmp edi, eax ; jl taken if edi < eax  (signed less)  -> 0x7C18F0
B7C1947 (0x7C1947..0x7C1977):
    IF cmp eax, 3 ; jle taken if eax <= 3  (signed less-or-equal)  -> 0x7C197C
B7C197C (0x7C197C..0x7C1983):
    IF cmp eax, edx ; jle taken if eax <= edx  (signed less-or-equal)  -> 0x7C19A2
B7C1990 (0x7C1990..0x7C19A2):
    IF cmp edx, dword ptr [esi + 0x38] ; jl taken if edx < dword ptr [esi + 0x38]  (signed less)  -> 0x7C1990
B7C19A2 (0x7C19A2..0x7C19B1):
    IF test cl,cl ; je taken if cl == 0  -> 0x7C19C6
B7C19B1 (0x7C19B1..0x7C19C6):
    JMP 0x7c19da
B7C19C6 (0x7C19C6..0x7C19D4):
    IF cmp ecx, 4 ; jbe taken if ecx <= 4  (unsigned below-or-equal)  -> 0x7C19D9
B7C19DA (0x7C19DA..0x7C19F4):
    CALL 0x9E8ED0   stack(push order)=[ebx, eax, 2, 0]  regs{}
    CALL 0x7BEE40   stack(push order)=[]  regs{eax=mov ebx}
    JMP 0x7c182f
B7C19F4 (0x7C19F4..0x7C19FD):
    RET 
```

## Map: `unlockable_reward_apply` (0x6BBEB0, 0x698 bytes)

stdcall reward application, 28-way jump table at `0x6BC548` (cases 16 and 17 share `0x6BC534`, the default).

### unlockable_reward_apply (0x6BBEB0)

- **True size** 0x698 (1688) bytes by recursive descent; 576 instructions; ret imm [4]; entry regs read (linear prefix) none.
- Frame: esp aligned (and esp,-8) with ebp frame; frame sub esp,0x1c; security cookie [0xE80B50].
- **Callees** (62 call sites, 34 distinct):

| Callee | Name | Sites | Callee `ret` / regs read |
|---|---|---|---|
| 0xC9604F | __security_check_cookie | 6BBEED 6BC53D 6BC522 6BC4D6 6BC41C 6BC43F 6BC3F9 6BC3DB ... | ret [], reads ['ecx'] |
| 0x9DE0C0 |  | 6BC501 | ret [0], reads [] |
| 0x7D9590 |  | 6BC4C8 | ret [0], reads [] |
| 0x9CC500 |  | 6BC49A | ret [0], reads [] |
| 0x7DE290 |  | 6BC476 | ret [0], reads [] |
| 0x7DE1F0 |  | 6BC452 | ret [0], reads [] |
| 0xAB8250 |  | 6BC3A7 | ret [0], reads [] |
| 0xBDCB30 | string_hash_lower | 6BC382 | ret [4], reads ['eax', 'ecx'] |
| 0x7AEBB0 |  | 6BC387 | ret [0], reads [] |
| 0x80FCF0 | session_hosting_flag | 6BC32A | ret [4], reads [] |
| 0x548590 |  | 6BC337 | ret [0], reads ['edx'] |
| 0x9DF320 |  | 6BC307 | ret [0], reads ['eax'] |
| 0x560050 |  | 6BC2E3 | ret [0], reads ['eax'] |
| 0x9DF280 |  | 6BC2BD | ret [0], reads ['edx'] |
| 0x9F19B0 |  | 6BC25D | ret [0], reads [] |
| 0x7C1A40 |  | 6BC1BB | ret [0], reads [] |
| 0x9DDA80 |  | 6BC1E2 | ret [0], reads [] |
| 0x6A7640 |  | 6BC1F4 | ret [0], reads [] |
| 0x549300 |  | 6BC122 | ret [0], reads [] |
| 0x545870 | saints_hq_set_upgrade_level | 6BC135 | ret [0], reads [] |
| 0x7B0560 |  | 6BC173 | ret [0], reads [] |
| 0x7ADA20 |  | 6BC0E7 | ret [0], reads ['edi'] |
| 0x7B35E0 |  | 6BC0C9 | ret [0], reads ['ecx'] |
| 0x6A7600 |  | 6BC050 | ret [0], reads [] |
| 0xB72800 | weapon_info_is_firearm | 6BC05A | ret [0], reads ['eax'] |
| 0xB72C40 | weapon_info_is_thrown | 6BC06F | ret [0], reads ['eax'] |
| 0xB690C0 | ammo_store_add_type | 6BC0AB | ret [8], reads [] |
| 0x787380 |  | 6BC026 | ret [0], reads ['eax'] |
| 0xAE2A00 | vehicle_create | 6BBF96 | ret [0], reads ['ecx'] |
| 0xAE28F0 | vehicle_from_handle | 6BBFA9 | ret [0], reads ['ecx'] |
| 0xAA4210 |  | 6BBFC4 | ret [4], reads [] |
| indirect | `call reg/mem` | 6BBFD4 | - |
| 0x926860 | misc_packet_send_14 | 6BBFDE | ret [0], reads ['ecx'] |
| 0xAC7420 |  | 6BBF62 | ret [0], reads ['eax'] |
- **Globals read**: 0x145A298x2, 0x21703D4x9, 0x21F2A90, 0x2528C14
- **Globals written**: 0x145A27C, 0x145A280x2, 0x145A281x2, 0x145A298x2, 0x2528BB4x2, 0x25274B4, 0x25297E4, 0x25283FB
- **Stack objects (lea of a frame slot, passed by address)**: [esp+0x0], [esp+0x1c], [esp+0x20]
- **Jump tables** (decoded from the exe bytes):
  - `jmp [idx*4+0x6BC548]` at 0x6BBF12 (28 entries): 0:6BBF19 1:6BC023 2:6BC03F 3:6BC0C4 4:6BC0E2 5:6BC151 6:6BC100 7:6BC11D 8:6BC197 9:6BC22E 10:6BC2A1 11:6BC2D9 12:6BC2FF 13:6BC320 14:6BC375 15:6BC3A0 16:6BC534 17:6BC534 18:6BC44A 19:6BC46E 20:6BC492 21:6BC3C3 22:6BC3E6 23:6BC427 24:6BC404 25:6BC4C5 26:6BC4E1 27:6BC52D
- **x87 sequence** (32 instructions; `fstp/fst dword` = f32 rounding point):

```
6BBF67 fldz 
6BBF80 fstp dword ptr [esp]   <== f32 store
6BC100 fld dword ptr [ebx + 8]
6BC103 fstp dword ptr [0x25274b4]   <== f32 store
6BC164 fld dword ptr [ebx + 0x2c]
6BC16C fstp dword ptr [esp]   <== f32 store
6BC22E fldz 
6BC230 fcomp dword ptr [ebx + 8]
6BC23A fld dword ptr [0x21f2a90]
6BC241 fsub qword ptr [0xe5bf40] ; f64(0x3FF0000000000000 = 1.0)
6BC247 fstp dword ptr [esp + 0x18]   <== f32 store
6BC24B fld dword ptr [esp + 0x18]
6BC24F fadd dword ptr [ebx + 8]
6BC252 fstp dword ptr [esp + 0x18]   <== f32 store
6BC256 fld dword ptr [esp + 0x18]
6BC25A fstp dword ptr [esp]   <== f32 store
6BC2A1 fld dword ptr [ebx + 0xc]
6BC2AA fld1 
6BC2B0 fsubrp st(1)
6BC2B2 fstp dword ptr [esp + 0x18]   <== f32 store
6BC2B6 fld dword ptr [esp + 0x18]
6BC2BA fstp dword ptr [esp]   <== f32 store
6BC2D9 fld dword ptr [ebx + 0xc]
6BC2E0 fstp dword ptr [esp]   <== f32 store
6BC3A0 fld dword ptr [ebx + 8]
6BC3A4 fstp dword ptr [esp]   <== f32 store
6BC3C3 fld dword ptr [ebx + 8]
6BC3CC fstp dword ptr [edx + 0x10c0]   <== f32 store
6BC404 fld dword ptr [ebx + 8]
6BC40D fstp dword ptr [ecx + 0x18b4]   <== f32 store
6BC427 fld dword ptr [ebx + 8]
6BC430 fstp dword ptr [edx + 0x18b8]   <== f32 store
```

**Section-by-section walk**

```
B6BBEB0 (0x6BBEB0..0x6BBED6):
    G[0x145A27C] <- (mov dword ptr [0x145a27c], ebx)
    IF cmp ebx, edi ; jne taken if ebx != edi  (not equal / non-zero)  -> 0x6BBEF8
B6BBED6 (0x6BBED6..0x6BBEF8):
    G[0x145A280] <- (mov byte ptr [0x145a280], 1)
    G[0x145A281] <- (mov byte ptr [0x145a281], 1)
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BBEF8 (0x6BBEF8..0x6BBF12):
    G[0x145A280] <- (mov byte ptr [0x145a280], 0)
    G[0x145A281] <- (mov byte ptr [0x145a281], 0)
    IF cmp eax, 0x1b ; ja taken if eax > 0x1b  (unsigned above)  -> 0x6BC534
B6BBF12 (0x6BBF12..0x6BBF19):
    JMP dword ptr [eax*4 + 0x6bc548]
B6BBF19 (0x6BBF19..0x6BBF2E):
    G[0x145A298] <- (mov dword ptr [0x145a298], edi)
    IF cmp dword ptr [ebx + 0x30], edi ; jbe taken if dword ptr [ebx + 0x30] <= edi  (unsigned below-or-equal)  -> 0x6BC534
B6BBF2E (0x6BBF2E..0x6BBF3B):
    IF test eax,eax ; je taken if eax == 0  -> 0x6BBF3D
B6BBF3D (0x6BBF3D..0x6BBF5D):
    IF test ecx,ecx ; jne taken if ecx != 0  -> 0x6BBF62
B6BBF5D (0x6BBF5D..0x6BBF62):
    JMP 0x6bbf67
B6BBF62 (0x6BBF62..0x6BBF67):
    CALL 0xAC7420   stack(push order)=[]  regs{}
B6BBF67 (0x6BBF67..0x6BBFB4):
    G[0x2528BB4] <- (add dword ptr [0x2528bb4], 1)
    f32 store dword ptr [esp]
    CALL 0xAE2A00 vehicle_create  stack(push order)=[7, 0x252a6d0, 0xec5ae4, 0, ecx, eax, 0, 0, 0, 0, 0]  regs{eax=mov 2 ecx=mov dword ptr [ebx + esi*8 + 8]}
    G[0x2528BB4] <- (sub dword ptr [0x2528bb4], 1)
    CALL 0xAE28F0 vehicle_from_handle  stack(push order)=[]  regs{ecx=mov eax}
    IF test edi,edi ; je taken if edi == 0  -> 0x6BBFFF
B6BBFB4 (0x6BBFB4..0x6BBFFB):
    CALL 0xAA4210   stack(push order)=[edi]  regs{esi=mov 5}
    CALL indirect edx  stack(push order)=[0, 1]  regs{ecx=mov edi edx=mov dword ptr [eax + 0x18] eax=mov dword ptr [edi]}
    CALL 0x926860 misc_packet_send_14  stack(push order)=[1, 1, 0]  regs{ecx=mov edi}
    G[0x145A298] <- (mov dword ptr [0x145a298], eax)
    IF cmp eax, esi ; jae taken if eax >= esi  (unsigned above-or-equal)  -> 0x6BC534
B6BBFFF (0x6BBFFF..0x6BC00F):
    IF cmp esi, dword ptr [ebx + 0x30] ; jb taken if esi < dword ptr [ebx + 0x30]  (unsigned below)  -> 0x6BBF2E
B6BC00F (0x6BC00F..0x6BC023):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC023 (0x6BC023..0x6BC03F):
    CALL 0x787380   stack(push order)=[]  regs{eax=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC03F (0x6BC03F..0x6BC046):
    IF cmp eax, edi ; je taken if eax == edi  (equal / zero)  -> 0x6BC04D
B6BC04D (0x6BC04D..0x6BC063):
    CALL 0x6A7600   stack(push order)=[]  regs{esi=mov dword ptr [ebx + 8]}
    CALL 0xB72800 weapon_info_is_firearm  stack(push order)=[]  regs{eax=mov ebx}
    IF test al,al ; je taken if al == 0  -> 0x6BC06D
B6BC063 (0x6BC063..0x6BC06D):
    IF cmp word ptr [ebx + 0xdc], 1 ; je taken if word ptr [ebx + 0xdc] == 1  (equal / zero)  -> 0x6BC07C
B6BC06D (0x6BC06D..0x6BC07C):
    CALL 0xB72C40 weapon_info_is_thrown  stack(push order)=[]  regs{eax=mov ebx}
    IF test al,al ; je taken if al == 0  -> 0x6BC534
B6BC07C (0x6BC07C..0x6BC089):
    IF cmp eax, edi ; je taken if eax == edi  (equal / zero)  -> 0x6BC534
B6BC089 (0x6BC089..0x6BC091):
    IF cmp ebx, edi ; je taken if ebx == edi  (equal / zero)  -> 0x6BC534
B6BC091 (0x6BC091..0x6BC09F):
    IF cmp ebx, edi ; je taken if ebx == edi  (equal / zero)  -> 0x6BC534
B6BC09F (0x6BC09F..0x6BC0C4):
    CALL 0xB690C0 ammo_store_add_type  stack(push order)=[3, edi]  regs{edi=mov dword ptr [ebx + 4] ecx=mov dword ptr [eax + 0x11dc]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC0C4 (0x6BC0C4..0x6BC0E2):
    CALL 0x7B35E0   stack(push order)=[]  regs{ecx=mov dword ptr [eax] eax=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC0E2 (0x6BC0E2..0x6BC100):
    CALL 0x7ADA20   stack(push order)=[]  regs{edi=mov dword ptr [ecx] ecx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC100 (0x6BC100..0x6BC11D):
    G[0x25274B4] <- (fstp dword ptr [0x25274b4])
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC11D (0x6BC11D..0x6BC12F):
    CALL 0x549300   stack(push order)=[]  regs{ecx=mov ebx}
    IF cmp eax, edi ; je taken if eax == edi  (equal / zero)  -> 0x6BC534
B6BC12F (0x6BC12F..0x6BC151):
    CALL 0x545870 saints_hq_set_upgrade_level  stack(push order)=[edx]  regs{edi=mov eax edx=mov dword ptr [ebx + 4]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC151 (0x6BC151..0x6BC15C):
    IF cmp dword ptr [ebx + 0x28], edi ; jle taken if dword ptr [ebx + 0x28] <= edi  (signed less-or-equal)  -> 0x6BC534
B6BC160 (0x6BC160..0x6BC183):
    f32 store dword ptr [esp]
    CALL 0x7B0560   stack(push order)=[ecx]  regs{ecx=lea [esp + 0x1c] eax=mov dword ptr [ebx + esi*4 + 8]}
    IF cmp esi, dword ptr [ebx + 0x28] ; jl taken if esi < dword ptr [ebx + 0x28]  (signed less)  -> 0x6BC160
B6BC183 (0x6BC183..0x6BC197):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC197 (0x6BC197..0x6BC1B1):
    IF cmp dword ptr [ebx + 0xa8], edi ; jbe taken if dword ptr [ebx + 0xa8] <= edi  (unsigned below-or-equal)  -> 0x6BC534
B6BC1B8 (0x6BC1B8..0x6BC1C8):
    CALL 0x7C1A40   stack(push order)=[]  regs{esi=mov dword ptr [edi - 4]}
    IF test eax,eax ; je taken if eax == 0  -> 0x6BC200
B6BC1C8 (0x6BC1C8..0x6BC1D4):
    IF cmp dword ptr [esp + 0x1c], edx ; jne taken if dword ptr [esp + 0x1c] != edx  (not equal / non-zero)  -> 0x6BC1EA
B6BC1D4 (0x6BC1D4..0x6BC1EA):
    CALL 0x9DDA80   stack(push order)=[eax, edx, ecx]  regs{ecx=lea [edi + 4] edx=mov dword ptr [edi]}
B6BC1EA (0x6BC1EA..0x6BC200):
    CALL 0x6A7640   stack(push order)=[eax]  regs{edi=mov dword ptr [edi] esi=lea [edi + 4] eax=mov dword ptr [esp + 0x14]}
B6BC200 (0x6BC200..0x6BC21A):
    IF cmp eax, dword ptr [ebx + 0xa8] ; jb taken if eax < dword ptr [ebx + 0xa8]  (unsigned below)  -> 0x6BC1B8
B6BC21A (0x6BC21A..0x6BC22E):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC22E (0x6BC22E..0x6BC23A):
    IF fcomp dword ptr [ebx + 8] ; test ah,0x05 ; jp taken when ST0 ? operand is in {gt, eq, un}  (gt = ST0 > operand, lt, eq, un = unordered)  -> 0x6BC279
B6BC23A (0x6BC23A..0x6BC279):
    f32 store dword ptr [esp + 0x18]
    f32 store dword ptr [esp + 0x18]
    f32 store dword ptr [esp]
    CALL 0x9F19B0   stack(push order)=[ecx]  regs{}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC279 (0x6BC279..0x6BC2A1):
    G[0x25297E4] <- (mov dword ptr [0x25297e4], 3)
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC2A1 (0x6BC2A1..0x6BC2D9):
    f32 store dword ptr [esp + 0x18]
    f32 store dword ptr [esp]
    CALL 0x9DF280   stack(push order)=[ecx]  regs{ecx=mov dword ptr [ebx + 8] edx=mov dword ptr [0x21703d4]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC2D9 (0x6BC2D9..0x6BC2FF):
    f32 store dword ptr [esp]
    CALL 0x560050   stack(push order)=[ecx]  regs{eax=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC2FF (0x6BC2FF..0x6BC320):
    CALL 0x9DF320   stack(push order)=[]  regs{eax=mov dword ptr [0x21703d4] edi=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC320 (0x6BC320..0x6BC329):
    IF cmp eax, edi ; je taken if eax == edi  (equal / zero)  -> 0x6BC353
B6BC329 (0x6BC329..0x6BC333):
    CALL 0x80FCF0 session_hosting_flag  stack(push order)=[eax]  regs{}
    IF test al,al ; je taken if al == 0  -> 0x6BC353
B6BC333 (0x6BC333..0x6BC353):
    CALL 0x548590   stack(push order)=[edi]  regs{edx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC353 (0x6BC353..0x6BC375):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp eax=lea [ebx + ebx*4]}
    RET 4
B6BC375 (0x6BC375..0x6BC3A0):
    CALL 0xBDCB30 string_hash_lower  stack(push order)=[ecx]  regs{ecx=mov 0xdc5797 eax=mov dword ptr [ebx + 8]}
    CALL 0x7AEBB0   stack(push order)=[]  regs{}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC3A0 (0x6BC3A0..0x6BC3C3):
    f32 store dword ptr [esp]
    CALL 0xAB8250   stack(push order)=[ecx]  regs{}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC3C3 (0x6BC3C3..0x6BC3E6):
    f32 store dword ptr [edx + 0x10c0]
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp edx=mov dword ptr [0x21703d4]}
    RET 4
B6BC3E6 (0x6BC3E6..0x6BC404):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp eax=mov dword ptr [ebx + 8]}
    RET 4
B6BC404 (0x6BC404..0x6BC427):
    f32 store dword ptr [ecx + 0x18b4]
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC427 (0x6BC427..0x6BC44A):
    f32 store dword ptr [edx + 0x18b8]
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp edx=mov dword ptr [0x21703d4]}
    RET 4
B6BC44A (0x6BC44A..0x6BC46E):
    CALL 0x7DE1F0   stack(push order)=[ecx]  regs{eax=mov esp ecx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC46E (0x6BC46E..0x6BC492):
    CALL 0x7DE290   stack(push order)=[ecx]  regs{eax=mov esp edx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC492 (0x6BC492..0x6BC4C5):
    CALL 0x9CC500   stack(push order)=[ecx]  regs{eax=mov esp ecx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp eax=lea [eax + 0x215ff18]}
    RET 4
B6BC4C5 (0x6BC4C5..0x6BC4E1):
    CALL 0x7D9590   stack(push order)=[]  regs{edx=mov dword ptr [ebx + 8]}
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC4E1 (0x6BC4E1..0x6BC4E8):
    IF cmp esi, edi ; je taken if esi == edi  (equal / zero)  -> 0x6BC534
B6BC4E8 (0x6BC4E8..0x6BC4ED):
    IF cmp dword ptr [esi + 0x20], edi ; jle taken if dword ptr [esi + 0x20] <= edi  (signed less-or-equal)  -> 0x6BC534
B6BC4F1 (0x6BC4F1..0x6BC519):
    CALL 0x9DE0C0   stack(push order)=[eax, edx, 1]  regs{eax=mov dword ptr [0x21703d4] edx=mov dword ptr [esi + 0x1c]}
    IF cmp edi, dword ptr [esi + 0x20] ; jl taken if edi < dword ptr [esi + 0x20]  (signed less)  -> 0x6BC4F1
B6BC519 (0x6BC519..0x6BC52D):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
B6BC52D (0x6BC52D..0x6BC534):
    G[0x25283FB] <- (mov byte ptr [0x25283fb], 1)
B6BC534 (0x6BC534..0x6BC548):
    CALL 0xC9604F __security_check_cookie  stack(push order)=[]  regs{ecx=xor esp}
    RET 4
```

## Fifteen small port recipes

All are cdecl-looking in Rust terms, but note the LTCG register inputs; install each with an inline detour that moves the registers as listed and shadow-check against the original on every call.

1. **`gamesave_count_unowned_districts`** (0x696B10, 0x22 bytes). Inputs: none. Output: EAX. `n = *(i32*)0x27ABD00`; if `n <= 0` return 0 (signed `jle`). Otherwise walk `n` 12-byte entries starting at `0x27ABD08` and count those whose dword is 0. Clobbers ECX, EDX. Rust: `(0..n).filter(|i| rd32(0x27ABD08 + 12*i) == 0).count()`.
2. **`gamesave_parse_digits`** (0x6953C0, 0x6A bytes). Inputs: EAX = digit count `n` (copied to ESI), ECX = pointer to ASCII. If the pointer is 0 return 0. `sign = 1`, `value = 0`. Loop: `c = *p` (signed byte); `n -= 1`; if `c == '-'` then `sign = -1` and skip the digit step; else if `c < '0'` or `c > '9'` (signed compares `jl` / `jg`) leave the loop; else `value += (c - '0') * 10^n` (the power is rebuilt by a loop that multiplies 1 by 10 `n` times after the decrement, so the first character is worth `10^(count-1)`); then `p += 1` and repeat while `n > 0` (signed). Return `sign * value` (`imul`). Preserves EBX, EBP, ESI, EDI.
3. **`tutorial_mark_unlocked`** (0x6B71B0, 0x4E bytes). Input: EAX = tutorial index. Return at once if byte `[0x2527CF5] != 0`. Let `s = [0x2528C14]`. If `s != 0`: return if (`byte [0x2528B56] != 0` and `byte [0x2528B2B] == 0`); then return if `byte [s+0x3F8] == 0`. Return if `index > 0x85` (unsigned `ja`). `p = 0x27DAEA4 + index*0x20`; if `*p == 0` set `*p = 1`. No result.
4. **`unlockable_is_unlocked`** (0x6BCFB0, 0x31 bytes). Input: ECX = pointer to a name hash (u32). Output: AL. `n = [0x145A29C]` (unsigned, `jbe` exits with 0). Scan records from `0x27DD018` with stride 0xD0 for `dword [rec] == *ecx`; on a match return the byte `[rec + 0xC4]`; else return 0. ESI is saved and restored; EAX, ECX, EDX are clobbered.
5. **`pcu_category_find_by_id`** (0x7C1A40, 0x23 bytes). Input: ESI = id. Output: EAX = record pointer or 0. `n = [0x25288D8]` (signed, `jle` gives 0). Records at `0x29E8CD0`, stride 0x78; the first with `dword [rec+0x14] == esi` is returned. Clobbers ECX, EDX.
6. **`pcu_item_find_variant_by_id`** (0x7C1B50, 0x28 bytes). Inputs: EDX = item record, ESI = id. Output: EAX. EDX == 0 returns 0. Variant array `[edx+0x20]`, count `[edx+0x24]` (signed), stride 0x28, id at `+0xC`. The result is the entry pointer on a match; the not-found path returns `0` through the `sub / neg / sbb / and` idiom (`index - count`, so it is 0 only when the loop ran to the end). Note: a negative count would return the array pointer (the idiom masks with ECX).
7. **`pcu_item_find_material_by_id`** (0x7C1BD0, 0x28 bytes). Same as the previous recipe with the array at `[edx+0x28]`, count `[edx+0x2C]`, stride 0x70, id at `+0x10`.
8. **`pcu_item_required_slots_present`** (0x7C1D40, 0x32 bytes). Inputs: ECX = record, stack arg = pointer to the occupancy bytes, EDI = slot count. `count = [ecx+0x1C]` (signed), `list = [ecx+0x18]`. For each `s = list[i]`: if `s < edi` (signed `jge` skips) and `bytes[s] == 0`, return 0. Return 1 (AL). Preserves EBX, ESI. (`0x7C1DA0` is identical with `[ecx+0x20]`, `[ecx+0x24]` and a test for `bytes[s] == 1` returning 0.)
9. **`pcu_worn_entry_copy_by_slot`** (0x7C21B0, 0x4A bytes). Inputs: EAX = owner object, EDX = slot id (compared as a sign-extended byte), stack arg = destination (0x50 bytes). `owner = [eax+0x10]`; `n = [owner*4 + 0x1FAC828]` (signed); entries at `owner*0x1720 + 0x1FAC858`, stride 0x50. For each entry compare `movsx byte [[entry+0x10]+0x15]` with EDX; on a match `rep movsd` 0x14 dwords to the destination and return AL = 1; else AL = 0. Note the function is preceded by the byte table of `pcu_worn_remove_base_slots`.
10. **`pcu_worn_entry_find_by_slot`** (0x7C22C0, 0x43 bytes). Same walk as the previous recipe with the slot in ESI and the stack arg `out: *mut *mut Entry`; on a match store the entry pointer and return AL = 1 (EAX is reused as the iterator; ECX counts).
11. **`gamesave_set_object_table_bit`** (0x696200, 0x5E bytes). Inputs: EAX = pointer, stack arg = save buffer. If the pointer is 0, or below `[0x22D7BCC]`, or at or above `[0x22D7BCC] + [0x22D7BD0]*0x498` (unsigned `jb` / `jae`), the index is 0xFF; else `index = (ptr - base) / 0x498` computed by `imul 0x6F74AE27; sar 9; add sign` (a signed division). Then `buffer[0x3A163 + (index >> 3)] |= table_0xDC8558[index & 7]` (a dword table holding 1, 2, 4, ... 0x80). Returns ECX = the address of the byte (not meaningful).
12. **`stat_record_get`** (0x6B53E0, 0x27 bytes). Inputs: EAX = id, stack arg = `out: *mut *mut Stat`. If `id <= -1` (signed `jle`) or `id >= [0x2527CE8]` (signed `jl` fails) return AL = 0. Else `*out = 0x1452BE0 + id*0x44` (computed as `(id*16 + id)*4`) and return AL = 1.
13. **`cash_widget::set_value_clamped`** (0x7ACEC0, 0x2C bytes). thiscall, ECX = this, stack arg = i32 value, `ret 4`. If `value > 0x77359400` (signed `jle` falls through otherwise) store 0x77359400 at `[this+0x10]`; else if `value < 0x88CA6C00` (signed, `jge` stores the value) store 0x88CA6C00; else store the value. These are +-2,000,000,000.
14. **`store_weapon_unlock_by_name`** (0x7B35E0, 0x3F bytes). Input: ECX = name (0 returns). `hash = string_hash_lower(ECX = name, EAX = 0, stack = &local)`; scan the table `[0x2528820]` (8-byte entries, count `[0x2528824]`, unsigned `jbe` skips) for `dword [e] == hash`; on a match set bit 0 of the byte `[e+4]`. ESI is saved; ECX is clobbered by the hash call.
15. **`gamesave_s12_tutorials_save / _load`** (0x693470, 0x26 bytes). Save: `dst = buf + 0x385F8`; for `off = 0; off < 0x10C0; off += 0x20` copy the dword at `0x27DAEA4 + off` to `*dst++` (134 dwords; `jb` unsigned). Load (`0x694E40`) is the same loop with the copy reversed. Both take the buffer as the stack arg, return nothing and clobber EAX, ECX, EDX. The dword is the state at record `+0xC`.

## Open

- The exact meaning of the player record fields `+0x9C / +0xA0 / +0xA4 / +0xAC / +0xB8` (they are saved in section 3 but only their offsets are decoded here).
- The customisation record layouts (`pcu_worn_resolve_conflicts`, `pcu_purchase_slot_apply` and `pcu_worn_list_import` are inferred from their callees; the structure sections above hold what the helpers settle).
- Roles of the award condition callbacks in the 69 rows, the stat name table, and the unlockable type numbers (the 28-way switch).
- The `0x7B6190` vehicle customisation state function (0x698 bytes, `vcust_*` code) and the `customers_*` shop customer classes were left alone (vehicle customisation and shop customers belong to the vehicle / AI areas).
