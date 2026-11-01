# 45: AI order table, AI event queue and helpers (names round 45)

This round looked for unnamed functions in the AI area (0x400000-0x463000 is the AI core; 0x98xxxx-0x9Fxxxx are the human AI helpers; 0xB1xxxx-0xB6xxxx the vehicle orders). The big finding is that the AI core is a **table-driven state machine**: every AI human has an *order kind* byte at `+0x149`, and a 36-row table of seven function pointers per kind lives at `0xDD5568`. That alone gives 252 previously anonymous functions their place. Nothing in the pathfinding / navmesh / road-spline area was named this round (not reached; see "Open").

Names are in `docs/candidates_ai.csv` (306 rows, none in `functions.csv`, notes end "(45)"). Sizes quoted below come from function-start heuristics and are approximate.

## Structures recovered

### Per-human AI order (human object)

| Offset | Meaning |
|---|---|
| +0x148 | sub-action byte of the current order (docs 21/40/41 call it "action") |
| +0x149 | **order kind**, signed byte, 0..0x23; 0xFF = none. Old kind kept in +0x14A |
| +0x150 | order payload, 24 dwords (0x60 bytes), layout depends on the kind. Copied in by `ai_order_set` |
| +0x32C | idle animation override id (-1 = none), only valid for kind 0 |
| +0x730..+0x748 | gaze / look-at block (`ai_gaze_set_target`, `ai_gaze_clear`) |
| +0x1120 / +0x1124 | int count / f32 sum, read by `ai_human_accumulator_mean` |

### Order table (0xDD5568, 36 rows x 28 bytes, index = kind)

Seven function pointers per row, index `kind*7`. Slot roles are confirmed from their callers:

| Slot | Address of table field | Signature | Called from |
|---|---|---|---|
| 0 enter | 0xDD5568 | cdecl (human) | `ai_order_set`, after the switch |
| 1 exit | 0xDD556C | cdecl (human, new_kind) | `ai_order_set`, before the switch |
| 2 update | 0xDD5570 | cdecl (human) | `ai_order_update` (0x4372E0), from `human_update_core` |
| 3 on_event | 0xDD5574 | cdecl (human, event node*) -> int | `ai_order_interrupt_query` (0x437890); 1 = default |
| 4 payload_init | 0xDD5578 | cdecl (payload*) | `ai_order_payload_init`, net readers, `ai_order_set` neighbours |
| 5 net_write | 0xDD557C | cdecl (human, buf, u16* pos) | `ai_order_net_write` (0x437920) |
| 6 net_read | 0xDD5580 | cdecl, 4 stack args | `ai_order_net_read` (0x437EB0) |

Rows 0x24 and up are not table rows: the bytes after row 0x23 are float constants and strings ("fear", "Hard Hitting", ...).

Kind meanings are mostly unknown. Evidence for a few:
- **0x00**: default order. `human_fell_through_world_recover` ends with a call into the kind-0 start; the enter calls `rand_int_range`, weapon equip and vehicle entry. Also the only kind with an idle animation override (+0x32C).
- **0x04**: attack. Set by `fuzz_crime_gang_fight::enter_state3`, `fuzz_crime_hooker_revolt::update` and the mission callers 0x5A90E0 / 0x5B65D0; update uses `human_effective_leader` and melee checks. Many starter functions (0x404A90, 0x4051F0, 0x405C70, 0x409B30, 0x40C7A0, ...) are attack sub-behaviours.
- **0x10**: vehicle-related (enter calls `vehicle_ai_order_04` and `vehicle_seat_occupant`).
- **0x0A**: world spot (`ai_spot_users_*` look at "state 10" with a spot index).
- **0x07**, **0x0A**: strings "fear" and emotion override, so fear / panic type orders.
- **0x18**: its enter 0x45A230 is also called directly by the multiplayer `read_state_delta` of NPCs and players.

### AI event queue

Two circular lists and a free list, nodes of at least 0x40 bytes:

| Offset | Meaning |
|---|---|
| +0 / +4 | next / prev |
| +8 | event type (-1 = cancelled / dead) |
| +0xC, +0x10 | two match arguments (usually human handle twice) |
| +0x14 | time posted (clock 0xE843AC, wraps at 0x6B49D200) |
| +0x18 | due time |
| +0x1C, +0x20 | two more payload dwords |
| +0x24..+0x38 | optional 6-dword vector payload, present when +0x3C bit 1 |

Lists: free `0x25269FC`, list A `0x2526A00`, active `0x2526A04`. `ai_event_post` (0x401420) pulls from the free list and links onto 0x2526A04. Random delays use the 2048-entry table at 0xEC3AC0 (cursor 0xEC3ABC), same as `rand_int_range`. A distinct system from `ai_scheduled_tasks_*` (lists 0xEE2C00 / 0xEE2C0C, doc 40).

## Names

The full list is the CSV. Groups:

| Group | Count | Evidence |
|---|---|---|
| `ai_order_kNN_{enter,exit,update,on_event,payload_init,net_write,net_read}` | 252 | the table at 0xDD5568, slot roles confirmed |
| `ai_order_kNN_start` | 31 | callers of `ai_order_set` with an immediate kind; first-path register reads from `regin.py`; confirmed that each calls ai_order_set, args undecoded |
| core | 7 | `ai_order_set` 0x4377A0, `ai_order_set_action` 0x4375B0 (294 callers), `ai_order_update` 0x4372E0, `ai_order_interrupt_query` 0x437890, `ai_order_net_write` 0x437920, `ai_order_net_read` 0x437EB0, `ai_order_payload_init` 0x43F960 |
| event queue | 4 | `ai_event_post` 0x401420 (223 callers), `ai_event_post_random_delay` 0x401580, `ai_event_find` 0x4020F0, `ai_event_cancel` 0x402160 (247 callers) |
| helpers | 11 | `ai_human_is_active_alive` 0x439C00, `ai_human_leader_flag4` 0x439660, `ai_human_stop_if_unflagged` 0x439430, idle anim override family 0x43F9C0 / 0x43FA30 / 0x43FA90 / 0x43FAB0, `ai_human_gaze_at_unless_self` 0x43F5C0, `ai_gaze_set_target` 0x9B5C80, `ai_gaze_clear` 0x9B5FF0, `ai_human_accumulator_mean` 0x4067B0, `vec3_distance_sq_dup` 0x404A20 |

Inferred vs confirmed is in each CSV note: structures and slot roles are confirmed; kind meanings are unknown unless a hint is given; helper names with "inferred" rely on field offsets only.

## Port candidates

Conventions: "kept" means unchanged on every path. All are static leaf or short call sequences. Write sets are what a shadow check should snapshot.

### 1. `ai_event_find` 0x4020F0
- In: EDI = type, ESI = arg0 (0 = wildcard), EDX = arg1 (0 = wildcard). Out: AL = 1 or 0. Kept: EDX, ESI, EDI (EBX, EBP too). Clobbered: EAX, ECX.
- Loop: for list head in (`[0x2526A00]`, `[0x2526A04]`): if head == 0 skip the list. node = head; loop: match test; node = `[node]`; stop when node == head or node == 0.
- Match: `[node+8] == EDI` AND (ESI == 0 OR `[node+0xC] == ESI`) AND (EDX == 0 OR `[node+0x10] == EDX`). Polarity: a failed type test or a failed non-wildcard test goes to the next node; success returns 1 immediately. End of both lists: 0.
- No callees, no x87. Writes: nothing.

### 2. `ai_event_cancel` 0x402160
- In: ESI = type, -1 = wildcard (note: -1, not 0); cdecl stack arg0 (compared with `+0xC`), arg1 (compared with `+0x10`), 0 = wildcard. No return value. Kept: EBX, EBP, EDI (pushed/popped), ESI. Clobbered: EAX, ECX, EDX. `ret` plain (caller cleans the 2 args).
- Two passes (head `0x2526A00`, then `0x2526A04`). Next pointer is computed before the test with the sbb mask: `next = (next != head) ? next : 0` (`neg` sets CF when the difference is non-zero, `sbb` makes the mask -1, `and`). Do not simplify to a null check.
- Test: skip node if ESI != -1 and `[node+8] != ESI`; skip if arg0 != 0 and `[node+0xC] != arg0`; skip if arg1 != 0 and `[node+0x10] != arg1`. Otherwise `[node+8] = 0xFFFFFFFF`.
- Writes: `[node+8]` of matching nodes in both lists.

### 3. `ai_human_accumulator_mean` 0x4067B0
- In: ECX = human. Out: ST0 = f32. ECX kept (push/pop), EAX untouched.
- `n = i32 [ecx+0x1120]`. If `n <= 0` (signed `jle`): result 0.0. Else `fld f32 [ecx+0x1124]; fidiv i32 n; fstp f32` (rounded to f32 once, via a stack slot), then reloaded into ST0.
- f32 rounding: compute `(sum as f64 / n as f64) as f32`. Writes: only the stack temp. No callees.

### 4. `ai_human_is_active_alive` 0x439C00
- In: ESI = human (may be null). Out: AL (0/1). Clobbers EAX, ECX, EDX (call). ESI kept.
- Returns 0 when: ESI null; `[esi+0x80] & 0xB != 0`; `([esi+0x80] >> 4) & 1 != 0`; `([esi+0x80] >> 5) & 1 == 0`; `[esi+0xC8] & 0x400000 == 0`; `[esi+0x103] & 8 != 0`; `human_is_dead(esi)` (0x985630, cdecl 1 arg, AL) non-zero. Else AL = (`byte [esi+0x595] == 0`). Callee order matters only for side effects (none).

### 5. `ai_human_leader_flag4` 0x439660
- In: EAX = human. Out: AL. EDI saved/restored; EAX and ECX not otherwise guaranteed (EAX = result).
- `leader = human_effective_leader` with EDI = human (0x98E7C0, returns EAX). If leader == human: 0. If `byte [leader+0x3CC] != 2`: 0. Else AL = `([leader+0x470] >> 4) & 1`.

### 6. `ai_human_has_idle_anim_override` 0x43F9C0
- In: EAX = human. Out: AL. Clobbers ECX. If `byte [eax+0x149] != 0` -> 0. Else AL = `dword [eax+0x32C] != -1` (`setne`). No callees.

### 7. `ai_human_gaze_at_unless_self` 0x43F5C0
- In: EAX = human, ECX = handle, stack arg (cdecl one dword) which the function loads into EDX **only on the call path** (`mov edx, [esp+4]`). Out: none. Callee `ai_gaze_set_target` 0x9B5C80 is called cdecl with (human = EAX, ECX value) pushed in that order (`push ecx; push eax`) and `mov cl, 1` after the pushes (CL only, no effect on the stack args); EDX = duration. `add esp, 8` after.
- Skip when `byte [eax+0x140] & 4 != 0` or `dword [eax+0x44] == ECX`. Polarity: bit set -> return; equal -> return; otherwise call. Kept: nothing is guaranteed besides EBX/ESI/EDI/EBP.

### 8. `ai_human_stop_if_unflagged` 0x439430
- In: EAX = human. Prologue `push ebp; mov ebp, esp; and esp, -8` (stack alignment, port as a plain call). If `byte [eax+0x470] & 0x10 == 0`: `0x993BD0(human, 0)` cdecl (`push 0; push eax; call; add esp, 8`). Otherwise nothing. 993BD0 is a 512-byte function (calls `human_state_set`), so this is a call-sequence port: keep the callee as the original.

### 9. `ai_human_set_idle_anim_override` 0x43FA30
- In: ESI = human, stack arg = anim id (cdecl). Out: none; EAX/ECX/EDX clobbered by callees.
- Guard: `byte [esi+0x149] == 0` AND `byte [esi+0x148]` not 0x11 AND not 0x1C AND `(word [[esi+0xD4]+0x3C] & 0x400) == 0`. Any failure: return.
- If `dword [esi+0x32C] != -1`: call 0x43FA90 with EAX = human (which stores -1 and calls `human_anim_set_update(human,-1,1,1,0)`). Then `[esi+0x32C] = arg` and `human_anim_set_update(esi, -1, 1, 1, 0)` (pushes 0,1,1,-1,esi; `add esp, 0x14`). Writes: +0x32C.

### 10. `ai_order_payload_init` 0x43F960
- In: EAX = kind (the whole register is used, `kind*7*4`), ECX = human. If ECX == 0 return. fn = `[0xDD5578 + kind*28]`; if fn != 0: `ECX += 0x150; push ECX; call fn; pop ECX`. The pop restores the *incremented* value, so ECX ends as `human+0x150` on that path; unchanged when no handler. EAX and EDX are clobbered when ECX was non-zero. Writes: whatever the handler writes to the payload.

### 11. Slot-1 stub `ai_order_k01_exit` 0x4028E0 (same shape: `ai_order_k0e_exit` 0x430100)
- cdecl (human, new_kind). `eax = [esp+4]`. If `byte [eax+0x140] & 4 != 0`: return (EAX = human). Else tail `jmp 0x9B5FF0` with EAX = human (`ai_gaze_clear`; null check inside; plain `ret`). So the port is: `if !(h.flags140 & 4) { ai_gaze_clear(h) }`. 0x9B5FF0 writes +0x730, +0x734, +0x738 from the globals 0x252A6D0/D4/D8, clears bit 0 of dword +0x760 and bit 0 of byte +0x75C, sets +0x748 = -1.

### 12. Payload init `ai_order_k01_payload_init` 0x4025D0
- cdecl (payload*). `fldz; fst [p]; fst [p+4]; fstp [p+8]`: three f32 zeros (a vec3). No return. EAX = p. Writes 12 bytes. Same pattern but different layouts: 0x45AAF0 (kind 0x19: two f32 zeros at +0, +4), 0x461270 (kind 0x1E: dword 0 at +0, f32 zeros at +4..+0x10).

### 13. `ai_order_k05_payload_init` 0x40D2F0
- cdecl (payload*). Byte writes only: `[p+0] = 8` (the code does `mov word [p], 0; or byte [p], 8`, so `[p+0] = 8` and `[p+1] = 0`), `[p+2] = 0`, `[p+3] = 0`, `[p+4] = 0`, `[p+5] = 0xFF`. ECX clobbered (0). No x87.

### 14. `ai_order_k07_payload_init` 0x4125B0
- cdecl (payload*). `byte [p] = 0; dword [p+4] = 0`. Identical bodies at 0x4327C0 (kind 0x11), 0x455FD0 (kind 0x14), 0x462270 (kind 0x1F). Each is a separate function in the table, so port one body and register all four addresses. No registers clobbered except EAX = p.

### 15. `ai_order_k17_exit` 0x459850 (same body: kind 0x1C exit 0x45E600 and kind 0x21 exit 0x4636C0)
- cdecl (human, new_kind). `eax = [human+0x44]` (handle); `esi` is preserved (push/pop) and set to 0x59; `push eax; push eax; call 0x402160; add esp, 8` = `ai_event_cancel(type 0x59, arg0 = handle, arg1 = handle)`. Note 0x432200 and 0x431010 first test `byte [human+0x140] & 4` (skip when set) and call `ai_gaze_clear`, so they are not identical: only 0x459850, 0x45E600 and 0x4636C0 are the plain form. Depends on port 2.

Everything above is integer except 3 (x87 divide) and 12 (x87 zeroing of f32 slots; a plain 0.0 store). Run frequency: 1, 2 and the order calls fire on every AI order change and the `ai_order_update` think; 4/5/6 are called from dozens of AI action bodies.

## Bugs and doc errors noticed

- Doc 21 / 15 say `humans_ai_update` is at 0x9C1B90 in one place and 0x9A71B0 in another; `0x9C1B90` is `human_ai_think` (the per-human think), `0x9A71B0` the loop.
- Docs 15, 40, 41 call `+0x149` the "state" and `+0x148` the "action". The seven-slot table shows `+0x149` is an **order kind** with enter / exit / update handlers; `+0x148` is a sub-action. Doc 41's "human state machine at +0x591/+0x592" is the separate body state machine (`human_state_set`), not the AI order.
- `tools/xrefs.py` reports byte-aligned matches inside instructions; for a table entry like 0xDD5570 it shows `lea ebp, [eax+0x55]`. Use the surrounding function start (bisect against function starts) to read the real instruction.
- Caller counts from a raw `e8` scan include false hits inside data and instructions, so 'callers' attributed to class methods (for example `npc_multi_registration::write_migration_data` for 0x437920) can be region artefacts. Treat them as hints.

## Open

- Kind names for the 36 orders, and the payload layouts.
- Pathfinding: `nav_closest_node_find`, `havok_navmesh_shape` users, `luafn_vehicle_pathfind_to_do`, road splines and the vehicle `vehicle_ai_drive` (0xB2DD90) helper chain (0xB2DB90, 0xB2DC20, 0xB2DC40, 0xB26530, 0xB1E390, 0xA8A5D0) were scanned but not named. The vehicle record is `vehicle+0x8554` with a timestamp at +0x1C and an order enum at +0x0; 0xB2DB90 allocates a record from ring 0x22B1458. Next round should start there.
