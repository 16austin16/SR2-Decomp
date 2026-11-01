# 43: Lua bindings 41-80 at instruction level

Continues doc 09 ("Ten top bindings at instruction level", "Bindings 21-40 at instruction level"). The argument rules, the stale slot at index 0 and the truth test are the ones described there. Names new in this doc are in `docs/candidates_script.csv`.

## How they were ranked

- Source: the 177 `.lua` files in the shipped archives (167 in `patch.vpp_pc`, 10 in `chunks4.vpp_pc`; no name is in both, so the "patch overrides" rule changes nothing). They were extracted to a scratch folder by offset / size from `cargo run -p sr2-formats --example vpp list` (the archives are uncompressed, flags 2). No game file was modified.
- Count: strip `--` to the end of the line, then count `\bname\s*\(` for every `luafn_*` name in functions.csv.
- This reproduces every count in doc 09's two tables exactly (4353, 2449, 856, ... 108). Doc 09's "comments stripped" therefore means **line** comments only. The insides of `--[[ ]]` blocks and of strings are still counted.
- A stricter count (block comments and strings removed, `a.f(` / `a:f(` and `function f(` excluded) differs by at most 2 for ranks 41-80 (column "strict"). It changes no rank by more than one place.
- Ties: 45/46 (96), 54-56 (79), 60/61 (74), 65/66 (66), 67/68 (64), 69/70 (63), 77/78 (54) and 80-82 (50) are in alphabetical order. Rank 80 is `character_exists`; `fade_in` and `inv_item_remove_all` also have 50 and are ranks 81 and 82.
- Ranks 1-40 match doc 09. Six of them are still only `named`, not ported: `thread_yield`, `debug_print`, `release_to_world`, `mission_debug`, `audio_play` and `objective_text`.

| # | Binding | Sites | Strict |
|---|---|---|---|
| 41 | `on_take_damage` 0xA3E820 | 107 | 107 |
| 42 | `set_max_hit_points` 0xA44CE0 | 106 | 105 |
| 43 | `set_ignore_ai_flag` 0xA44A90 | 100 | 100 |
| 44 | `effect_stop` 0xA46860 | 98 | 98 |
| 45 | `get_max_hit_points` 0xA308C0 | 96 | 95 |
| 46 | `hud_timer_stop` 0xA31610 | 96 | 96 |
| 47 | `audio_get_audio_id` 0x7ECCD0 | 95 | 95 |
| 48 | `notoriety_set` 0xA3BB10 | 91 | 89 |
| 49 | `mission_end_success` 0xA39990 | 88 | 87 |
| 50 | `objective_text_clear` 0xA47000 | 86 | 85 |
| 51 | `vint_document_find` 0xB8F0E0 | 83 | 83 |
| 52 | `rand_float` 0xD74710 | 81 | 81 |
| 53 | `inv_item_equip` 0xA34B20 | 80 | 79 |
| 54 | `mission_set_checkpoint` 0xA370B0 | 79 | 79 |
| 55 | `on_trigger_exit` 0xA3EC40 | 79 | 79 |
| 56 | `vehicle_chase` 0xA489F0 | 79 | 78 |
| 57 | `set_mission_author` 0xA44F10 | 78 | 78 |
| 58 | `peg_unload` 0x7EC890 | 76 | 76 |
| 59 | `vehicle_speed_override` 0xA4D860 | 75 | 75 |
| 60 | `player_controls_enable` 0xA40CF0 | 74 | 74 |
| 61 | `vint_datagroup_add_subscription` 0xB8E150 | 74 | 74 |
| 62 | `set_unjackable_flag` 0xA45790 | 73 | 73 |
| 63 | `player_controls_disable` 0xA40CB0 | 69 | 69 |
| 64 | `vint_insert_values_in_string` 0xB908C0 | 68 | 67 |
| 65 | `door_open` 0xA2DFF0 | 66 | 66 |
| 66 | `set_current_hit_points` 0xA43EC0 | 66 | 64 |
| 67 | `on_dismiss` 0xA3DBE0 | 64 | 64 |
| 68 | `spawning_pedestrians` 0xA460F0 | 64 | 64 |
| 69 | `vehicle_suppress_npc_exit` 0xA4DBC0 | 63 | 63 |
| 70 | `vint_dataitem_get` 0xB8ECA0 | 63 | 63 |
| 71 | `npc_combat_enable` 0xA2C700 | 59 | 59 |
| 72 | `peg_load` 0x7EC820 | 59 | 59 |
| 73 | `npc_weapon_spread` 0xA3BFC0 | 58 | 58 |
| 74 | `waypoint_remove` 0xA4E5B0 | 57 | 57 |
| 75 | `waypoint_add` 0xA4E450 | 56 | 56 |
| 76 | `hud_timer_set` 0xA31500 | 55 | 54 |
| 77 | `get_current_hit_points` 0xA303A0 | 54 | 54 |
| 78 | `vint_document_unload` 0xB8F260 | 54 | 54 |
| 79 | `on_vehicle_exit` 0xA3F260 | 52 | 51 |
| 80 | `character_exists` 0xA35880 | 50 | 50 |

## Conventions met in this batch

**Optional-argument kinds.** Each optional flag is one of these. "Count-guarded" means the binding first checks how many arguments are left, so a missing argument never reaches the stale slot.

| Kind | Missing | nil | Otherwise |
|---|---|---|---|
| *def T* | true (count-guarded) | true | truth test |
| *def F* | false (count-guarded) | false | truth test |
| *stale* | the stale slot at index 0 | false | truth test |

**Running count.** Some bindings (`inv_item_equip`, `vehicle_chase`, `hud_timer_set`, `mission_end_success`, `vint_datagroup_add_subscription`) read front to back with an "args left" counter `c` (it starts at n), the way `vint_subscribe_to_input_event` does. An argument is read at `−c`; a nil there takes the default and still decrements `c`.

**`script_lua_opt_int`** (0xA53EB0, and identical copies 0xA53960 / 0xA54020): `EDI` = L, cdecl (&args_left, &default).
- If args_left > 0 and the slot at `−args_left` is not nil: returns `_ftol2(lua_tonumber)`.
- Otherwise returns the default.
- It decrements args_left whenever args_left > 0 (also for nil).
- The default is always 3. In `effect_stop_by_handle` that value is a flag set (bit 0 = act locally, bit 1 = send to the network), so 3 is probably "local + remote".

**Callback blocks.** A callback block is an array of name handles, indexed by callback type (−1 = none). The `*_callback_set` helpers take `ECX` = block and `EAX` = &type, and the name on the stack. They are **`ret 4`**, not cdecl: doc 09 and functions.csv say cdecl. The Rust ports call them through `call_regs`, which restores ESP, so they are unaffected.
- The old handle is released only when `[0x2528C14]` is non-zero, (byte 0x2528B56 == 0 or byte 0x2528B2B != 0), and byte `[0x2528C14]+0x3F8` is set. Otherwise the reference leaks.
- An empty or NULL name just clears the slot.
- Handles come from `script_name_handle_acquire` (0xA26480) and are freed by `script_name_handle_release` (0xA26420).

Callback type ids seen here:

| Binding | Human +0x1234 | Object +0xBC | Vehicle +0x7C | Mover +0x64 | Trigger +0x150 |
|---|---|---|---|---|---|
| `on_take_damage` | 4 | 6 | 2 | 2 (0xA533A0) | |
| `on_dismiss` | | 0xA | | | |
| `on_vehicle_exit` | 0x14 | | 5 | | |
| `on_trigger_exit` | | | | | 1 (enter is 0) |

**Name-lookup chains.** Below, "human chain" means this order:
1. `script_find_object_by_name(ECX = name, EAX = 0)`. It searches the player list and "#PLAYER#". It returns 0 for a NULL name **without writing *found**.
2. Else `script_follower_index(EAX = name)` → `script_follower_get(cdecl idx)`. It only accepts 1-3, so 0 / −1 are safe. `script_follower_index` dereferences the name, so a NULL name crashes here.
3. Else `script_object_find(EAX = name)` → vtbl +0x2C (when noted) → vtbl +0x34 = its human.

`script_door_find_by_name` (0xA5B6E0) returns any **script mover**, not only doors (mover hit points at mover +0x130 → +0, max at +0x130 → +0x6C → +0x34).

## Instruction-level table

| # | Binding | Exact behaviour |
|---|---|---|
| 41 | `on_take_damage` | n ≤ 2; callback = `tostring(−n)`, name = `tostring(1 − n)` (stale with n = 1). First match wins: human (`script_find_object_by_name`, EAX = 0) → `human_script_callback_set(+0x1234, type 4)`; `script_object_find` → `object_script_callback_set(+0xBC, 6)`; `script_vehicle_find` → `vehicle_script_callback_set(+0x7C, 2)`; `script_door_find_by_name` → `mover_script_callback_set` 0xA533A0(`ECX` = +0x64, `EAX` = &2, name; `ret 4`). Returns 0. |
| 42 | `set_max_hit_points` | n ≤ 3; name; value = `_ftol2(tonumber(1 − n))` (stale with n = 1); full = arg 3 *def F*. Human chain (with +0x2C). **Human**: ratio = f32 hp +0x1124 / i32 max +0x1120 (x87; 0 when max ≤ 0); `human_set_max_hit_points(ECX = h, &int value)` (ret 4); full → `human_set_hit_points(ECX = h, &f32(new max +0x1120))`; else hp = f32(f32(ratio) × value). **An old max ≤ 0 sets hp to 0.** Not a human: `script_vehicle_find` → vtbl +0x34; a null vehicle → return; `net_send_prop_vehicle_max_hit_points(ECX = v, value)`, full → `net_send_prop_vehicle_hit_points(ECX = v, value)` (both ret 4). No vehicle: `script_object_find` → object +0x74 = value (unspawned max). Returns 0. |
| 43 | `set_ignore_ai_flag` | n ≤ 2; name; flag *def T*. NULL name → return. Follower index > 0 → follower; else `script_object_find` → vtbl +0x34 (**no +0x2C test; the player is not looked up**). No human → return. If bit 3 of byte +0x103 already equals the flag → return. Set: `human_ai_set_force_flag_ignore_ai(&h+0xF8, 1)` (stdcall); then unless `human_vehicle_interaction_exiting(cdecl h)`: in a vehicle (`human_in_vehicle(ESI = h, EDI = 0)`) → 0x456070(`ECX` = h); else if byte +0x149 ≠ 0 and byte +0x148 ≠ 1 → `object_teleport_prepare(h, 0, 0)`. Clear: `ignore_ai(&h+0xF8, 0)`. Returns 0. |
| 44 | `effect_stop` | n ≤ 2; handle = `_ftol2(arg 1)`; flags = `script_lua_opt_int` (default 3) from arg 2. `effect_stop_by_handle(ESI = handle, cdecl 1, flags, 0)`. Returns 0. |
| 45 | `get_max_hit_points` | n ≤ 1; `script_get_max_hit_points(EDI = name)` → ST0, pushed as a number. Order: human chain (**no +0x2C test**; vtbl +0x34 is called twice) → i32 +0x1120 (signed); vehicle → +0x8C24 **unsigned**; mover (+0x2C, +0x34) → +0x130 → +0x6C → +0x34 unsigned; none → 0. |
| 46 | `hud_timer_stop` | n ≤ 1; i = `_ftol2(arg 1)`, **no bounds check**. `hud_element_stop(cdecl 0x29E3F18 + i×64, 5, i)`. The callback handle `[0xEB7F3C + i×4]` is released under the callback-block condition above, then set to −1. An out-of-range i writes out of bounds. Returns 0. |
| 47 | `audio_get_audio_id` | n ≤ 2; result defaults to 0xFFFF. The type of arg 1 (n = 0 → nil): **string** → name; category = arg 2, "foley" when missing or nil; `audio_category_from_name(EDI)` (Foley 0, Voice 1, Music 2, Ambient 3, else −1); if the name is non-NULL, `audio_find_id_by_name(ECX = name, cdecl category)` (0x465AB0). **Ambient or unknown always gives 0xFFFF.** **Number** → u16(`_ftol2`). Pushes the u16 as a number (1 result). |
| 48 | `notoriety_set` | n ≤ 2; team = `tostring(−n)`; level = f32 `tonumber(1 − n)` (stale with n = 1). `net_send_notor_pkt_2(cdecl notoriety_team_from_name(EDI = team), f32 level)`. Team ids: brotherhood / los_carnales 0, ronin / vice_kings 1, samedi / rollers 2, police 3, else −1 (passed on). Returns 0. |
| 49 | `mission_end_success` | n ≤ 3, running count. mission = arg 1. cutscene = arg 2 (nil → NULL). arg 3: a **string** is navpoint 1; a **table** gives `t[1]`, `t[2]` (pushes the number key, `luaV_gettable(L, t, key, 0)`, copies the result over the key, `tostring(−1)` unless nil, pops). Each name → `string_crc32(EAX = &h, EDX = 0, name)`, `navpoint_find(EAX = &h)` → nav +0xC, default `[0x33D6308]`. Then `string_hash_lower(ECX = mission, EAX = 0, &h)`; `[0x27B3C60]` must exist with +0 == h, else return. Diversion: byte 0x140BF8E and mission +0x3C == 1 and (+0x90)->vtbl+4() == 0xE → `diversion_canceled_text_2()`, return. Return if `[0x1450380]` is 5 or 6, or 0xEAAF48 ≥ 0 or 0xEAAF44 ≥ 0, or 0x750500() (document `[0x2527F90]` live). Else byte 0x2527C86 = 1; 0x6A13C0(cdecl mission); player +0x1163: **tests bit 0x08 but clears 0x04**, then 0x9D9FD0(cdecl player); `conversation_stop(ESI = 0x2527C90)` when set. Kind +0x3C not 0 / 3 → `mission_end(ECX = mission, cdecl 1)`. Kind 0 / 3: 0xEAAF48 = now + 2000 (wrapped at 1.8e9), 0x27B3C74 / 78 = the nav hashes, and if a cutscene was given `[0x27B3C70]` = `script_heap_alloc(len + 1)` copy (unchecked; any old pointer is not freed). The success flow (doc 22) plays it later. Returns 0. |
| 50 | `objective_text_clear` | n ≤ 1; i = `_ftol2`, no bounds check; `hud_element_stop(cdecl 0x29E41F8 + i×0x12C, 8, i)`. Returns 0. |
| 51 | `vint_document_find` | n ≤ 1. Missing, nil, or `tostring` NULL → `vint_lua_current_document()`. Else `string_crc32_lower(ECX)`, then walk the ring `[0x252A1AC]` (next at +0) for +0x48 == hash. Pushes doc +0x4C as **unsigned**, or 0. Always 1 result. |
| 52 | `rand_float` | n ≤ 2; a = f32 arg 1, b = f32 arg 2 (stale with n = 1); swap if b < a (NaN: no swap). r = `rng_float01(EAX = 0xEC30F8)`; push f32(min + r × (max − min)) (x87, then rounded to f32). **0xEC30F8 is rand_int's generator**: table +0x9C8 = 0xEC3AC0, cursor +0x9C4 = 0xEC3ABC, so both bindings advance the same cursor. |
| 53 | `inv_item_equip` | n ≤ 3, running count (arg 3 is never read). weapon = arg 1 (nil → NULL); who = arg 2, "#PLAYER1#" when missing or nil; NULL → return. found byte = 0; `script_find_object_by_name(ECX, EAX = &found)`; not found → `script_object_to_human(ESI = name)`; none → return. weapon NULL → `inventory_equip_slot(&h+0x10C4, 0, 1, 0)` (stdcall, ret 0x10). Else `weapon_info_find_by_name(cdecl)` (none → return) → `inventory_equip_slot(&h+0x10C4, info +0x3C, 1, 0)`. Returns 0. |
| 54 | `mission_set_checkpoint` | n ≤ 2; name; flag *def F*. `mission_set_checkpoint(cdecl name, flag)`. The flag dword is the byte stored in **L's argument slot**, so its upper 3 bytes are L's. Returns 0. |
| 55 | `on_trigger_exit` | As `on_trigger` with type 1: `trigger_find_by_name`; only if +0x13C == 0: `script_set_callback(ECX = +0x150, EAX = &1, name)`. Then if **+0x154** ≠ −1: +0x11C = now (minus 1.8e9 if above it). Returns 0. |
| 56 | `vehicle_chase` | n ≤ 6, running count: vehicle; target ("#PLAYER1#" when missing or nil); A *def T*; B *def F*; C *def T*; D *def F*. Target human: `script_find_object_by_name(&found)`, else `script_object_to_human(ESI)`. v = `script_vehicle_from_name(EAX = vehicle)`. Both needed. If (v+0x84E4)+0x9C == 3: 0xB398B0(`ECX` = v, cdecl target +0x44). Else an order on the stack (POD, every field the callee reads is written): +0 target handle, +4 byte (B ? 3 : 0), +8 dword (B == 0), +0xC flags A \| C<<3 \| D<<7 \| 0x12, +0x10 word 0 → `vehicle_ai_order_03(cdecl v+0x8554, &order)`. Then for each seat (count = v+0x6CE8 if byte v+0x84D8 == 1, else 8): `vehicle_seat_occupant(EAX = v+0x8ABC + i×0x24)`; an occupant with byte +0x3CC == 2 gets `human_ai_set_force_flag_scripted_attack(&h+0xF8, 1)`. Returns 0. |
| 57 | `set_mission_author` | `xor eax, eax; ret`: does nothing, 0 results. |
| 58 | `peg_unload` | n ≤ 1; missing, nil or NULL → nothing; `peg_unload_by_name(ECX = name)`: `render_thread_wait_frame(0)`, strips the extension, finds the slot in 10 × 0x80 at 0x29A6018, cancels its async reads and releases it. Returns 0. |
| 59 | `vehicle_speed_override` | n ≤ 2; name; mph = f32 arg 2 (stale with n = 1). v = `script_vehicle_from_name`; s = f32(mph × 0.44704); s ≥ 1000 **or NaN** → 1000; negatives pass. ai = v+0x8554: byte +0x16 &= ~0x10, f32 +0x88 = s, `vehicle_ai_set_force_flag_override_speed(EDI = ai, 1)` (ret 4). Returns 0. |
| 60 | `player_controls_enable` | n ≤ 1; `net_send_player_control_2(ECX = script_find_object_by_name(name, EAX = 0), cdecl 0)`. A NULL human is passed through (the callee tests ECX). Returns 0. |
| 61 | `vint_datagroup_add_subscription` | n ≤ 3. Arg 1 type (n = 0 → nil): **number** h: idx = h & 0xFFFF < 32, rec 0x31BACF0 + idx × 32 with +0x14 == h **and byte +0x1C ≠ 0** (06 says +0x1C == 1 marks a group for destruction, so this path only accepts a group marked for destruction; the hash path below requires 0). **String**: `vint_datagroup_find_by_hash(EDX = string_crc32_lower(name))`. Other types → return. Count −1, then kind = `tostring(−c)`, func = `tostring(1 − c)`: with n = 2, func is the **stale slot**; with n = 1, kind is stale and func is **index 1 = argument 1**. Needs the current document, kind and func. kind: "update" 3, "remove" 5, "insert" 4, else return. cb = `vint_callback_find_by_hash(EDX = crc(func))` (+0x1C += 1), or `vint_callback_create(EDX = func)`. Group found → `vint_datagroup_subscribe(EDX = cb+0x18, stdcall group, kind, doc +0x4C)`. A string name that didn't resolve → `vint_datagroup_subscribe_by_hash(EDI = cb+0x18, cdecl hash, kind, doc +0x4C)` (pending). Release one reference (vtbl +0x14 when it was 1). **0 results.** |
| 62 | `set_unjackable_flag` | n ≤ 2; name; flag *def T*. `script_vehicle_find`: script object byte +0x74 bit 1 = flag; if vtbl +0x2C: `vehicle_set_force_flag_unenterable(stdcall vtbl+0x34(), flag)`. The flag dword's upper bytes come from L's slot. Returns 0. |
| 63 | `player_controls_disable` | As `player_controls_enable` with cdecl 1. |
| 64 | `vint_insert_values_in_string` | **Not portable yet**: stack vint_value objects, the temp allocator `[0x252A250]` (vtbl +0x30 mark, +0x1C alloc, +0x34 release), localisation hooks `[0x252A1E0]`. n ≤ 2. Arg 1 → `vint_value_from_lua` (any type). 32 values zeroed at esp+0x48; count = `lua_table_count(EAX = args left)` (−1 without arg 2); each entry via 0xB91630, **with no limit of 32** (a larger table overruns the stack). Types 1 / 2 (id): string = `[0x252A1E0]+4(id)`; **none → release, 0 results**. String: 0xB92670 ASCII→wide, or `[0x252A1E0]+0(str)`. Then 0xB926D0(out, src, 0x1000, values, count), `vint_encode_wide_string(ECX, 0x2000)`, `lua_pushstring` → 1 result. |
| 65 | `door_open` | n ≤ 2; name; flag *def F*. Mover = `script_door_find_by_name(EAX)`; none → push false. Else push bool(`script_door_set_open(ECX = door, stdcall flag)` AL). The flag dword's upper bytes come from L's slot. 1 result. |
| 66 | `set_current_hit_points` | n ≤ 2; name; hp = f32 arg 2 (stale with n = 1). Human chain (with +0x2C) → `human_set_hit_points(ECX = h, &f32)`. Vehicle: `fistp` with chop → int; `net_send_prop_vehicle_hit_points(ECX = vtbl+0x34(), int)`, **with no null check on the vehicle**. Mover: `script_mover_set_hit_points(ESI = mover, stdcall _ftol2_sse(hp))`. Returns 0. |
| 67 | `on_dismiss` | n ≤ 2; callback, name (stale with n = 1). `script_object_find(EAX = name)` only (no player lookup) → `object_script_callback_set(+0xBC, type 0xA)`. Returns 0. |
| 68 | `spawning_pedestrians` | n ≤ 2. enable = **truth test of arg 1** (with n = 0, the stale slot); despawn = arg 2 *def T*. Enable: bytes 0x215DA25 = 0x215DA05 = 1, 0xB7DA30() (re-evaluates the 6 ambient-spawn flags). Disable: both 0, 0xB7DA30(), and if despawn: 0x93D310(`ESI` = 0, cdecl 1, 0). Returns 0. |
| 69 | `vehicle_suppress_npc_exit` | n ≤ 2; name; flag *def T*. v = `script_vehicle_from_name`; ai = v+0x8554: `vehicle_ai_set_force_flag_no_stop_enter_with_driver(ai, flag)`, then `..._force_no_passenger_exit(ai, flag)` (stdcall, ret 8). Returns 0. |
| 70 | `vint_dataitem_get` | **Not portable yet**: 32 stack vint_values built with `vector constructor iterator` (ctor 0x7903C0). n ≤ 1; h = `_ftol2`; h == 0, (h & 0xFFFF) ≥ 0x80, or rec (0x31A96F0 + idx × 0x22C) +0x224 ≠ h → **0 results**. Else `vint_dataitem_copy_values(EAX = values, stdcall rec)`; for each of rec +0x214 values, `vint_value_push_lua(EAX = L, EDX = &v)` adds to the count; free all 32; returns the count. |
| 71 | `npc_combat_enable` | n ≤ 2; name; enable *def T*. NULL name → return. Follower > 0, else `script_object_find` → vtbl +0x34 (no +0x2C). `human_ai_set_force_flag_combat_disabled(&h+0xF8, !enable)`. Returns 0. |
| 72 | `peg_load` | n ≤ 1; missing, nil or NULL → nothing; `peg_load_by_name(ECX = name, cdecl 0x75)`. Returns 0. |
| 73 | `npc_weapon_spread` | n ≤ 2; name; spread = f32 arg 2, **1.0 when missing or nil**. NULL name → return; follower or `script_object_find` → vtbl +0x34; `human_set_npc_spread_multiplier(stdcall h, &spread)`. Returns 0. |
| 74 | `waypoint_remove` | n ≤ 1, stored in **L's argument slot**; sync = `script_lua_opt_int` (default 3); `waypoint_remove(ESI = sync)`. Returns 0. |
| 75 | `waypoint_add` | n ≤ 2; name; sync = opt int arg 2 (default 3). Order: `script_vehicle_find` (+0x2C) or else `script_object_find` (+0x2C) → vtbl +0x34 +0x44 handle → push `waypoint_add_object(cdecl handle, 1, sync)` as a signed int. Else `trigger_find_by_name` → `waypoint_add_position(ESI = sync, vec3 by value trigger +0x14)` → **push 0**. Else navpoint (`string_crc32` + `navpoint_find`) → the same with nav +0x1C → push 0. None → push −1. 1 result. |
| 76 | `hud_timer_set` | n ≤ 5, running count. i = arg 1; ms = arg 2 (stale with n = 1); callback = arg 3 (missing or nil → "" at 0xDC5797); **arg 4 is skipped** (its slot is fetched and dropped); sync = opt int (default 3). `hud_timer_start(cdecl i, ms, sync)` (ignores i ≥ 4); `hud_timer_set_callback(EAX = &i, EDI = callback)` (releases the old handle, acquires a new one when non-empty, kind from a table, then a network sync through 0xA55160). Returns 0. |
| 77 | `get_current_hit_points` | n ≤ 1. Human chain (no +0x2C) → push f32 +0x1124. Vehicle → +0x8C28 unsigned. Mover: (mover +0x8C → +0x130 → +0) if spawned, else +0xCC, **unsigned**, so an unspawned destroyed mover (−1) gives 4294967295. None → 0. |
| 78 | `vint_document_unload` | n ≤ 1. Missing or nil → current document. Number h ≠ 0 with (h & 0xFFFF) < 12 and doc (0x319F6B0 + idx × 0x57C) +0x4C == h. Other types → nothing. doc byte +0x572 = 1 (unload request). Returns 0. |
| 79 | `on_vehicle_exit` | n ≤ 2; callback, name (stale with n = 1). Human → type 0x14; else `script_vehicle_find` → `vehicle_script_callback_set(+0x7C, 5)`. Returns 0. |
| 80 | `character_exists` | n ≤ 1. found = the byte in **L's argument slot** (not initialised). `script_find_object_by_name(ECX = name, EAX = &found)`. If found ≠ 0 → result = object ≠ 0. Else `script_object_find(EAX)` → vtbl +0x34 ≠ 0. A NULL name leaves found as L's low byte, but the result is false either way. Pushes a boolean. |

## Port notes

- **Not portable yet:** `vint_insert_values_in_string` (stack vint_value objects, temp allocator marks, localisation hooks, unbounded table copy) and `vint_dataitem_get` (C++ array construction on the stack). Port them after the vint value type (06).
- **Portable but large:** `mission_end_success`. Every callee convention is known (0x6A13C0 cdecl(mission), 0x9D9FD0 cdecl(player), 0x750500 no arguments → AL, `mission_end` ECX = mission, cdecl(1)). It writes mission globals (0xEAAF48, 0x27B3C70/74/78, 0x2527C86), so test it with a real mission end. Its table read uses `luaV_gettable`, which needs the binding's exact stack discipline (push the key, overwrite it with the result, pop).
- **Result counts to keep:** `vint_datagroup_add_subscription` returns 0. `vint_dataitem_get` returns 0 for a bad handle. `vint_insert_values_in_string` returns 0 when a string id has no text. `waypoint_add` returns 0 (not a handle) for trigger and navpoint waypoints. `door_open` and `character_exists` always return 1.
- **Unsigned pushes:** `vint_document_find`, the vehicle and mover branches of `get_max_hit_points` / `get_current_hit_points`. The human max hit points is signed.
- **f32 round trips:** `rand_float`, `notoriety_set`, `vehicle_speed_override`, `set_current_hit_points`, `npc_weapon_spread`, `set_max_hit_points` (ratio).
- **Dwords with L's upper bytes:** `mission_set_checkpoint`, `set_unjackable_flag` and `door_open` pass a flag byte stored in L's argument slot as a whole dword. Pass `(L & 0xFFFFFF00) | flag` for an exact shadow, or a clean 0 / 1 if the callees only read the byte. They do: `mission_set_checkpoint` and the force-flag setters read a byte.
- **Shared RNG:** `rand_float` advances the same cursor (0xEC3ABC) as `rand_int` and every `rng_float01(0xEC30F8)` caller. A shadow check must snapshot and restore it.
- **Crash paths to keep or guard:** `hud_timer_stop` and `objective_text_clear` don't check the index. `set_current_hit_points` passes a null vehicle to `net_send_prop_vehicle_hit_points`. `set_max_hit_points` with a NULL name reaches `script_follower_index(NULL)`. A port can guard these, but must say so.

## Doc 09 corrections found

- `sizeof_table` / `lua_table_count`: the index is **not** base-relative. `lua_table_count(EAX = k, cdecl L)` reads the table at `−k` (top-relative) and returns −1 for nil. It returns `t.n` when that is a number, otherwise the `lua_next` pair count. So `sizeof_table` (EAX = 1) counts the **last** argument, not argument 1. The comment in `lua_bindings.rs` repeats the error, but its behaviour is right, because it calls the original with EAX = 1.
- `human_script_callback_set`, `object_script_callback_set`, `vehicle_script_callback_set` and `script_set_callback` are `ret 4` (callee pops the name), not cdecl.
- 0xA53EB0 (`effect_play`'s optional int) also takes `EDI` = L.
- `rand_int`'s "pre-made random table" is the generic generator state 0xEC30F8, shared with `rand_float` and `rng_float01`.
