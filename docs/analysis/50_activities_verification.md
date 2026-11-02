# 50 verification: activities candidates and renames

Checked against `bin/SR2_pc.unpacked.exe` and `docs/functions.csv` (13,474 rows at the final re-check). Originals, `functions.csv` and doc 50 were not edited. Outputs: `docs/candidates_activities_v.csv`, `docs/renames_activities_v.csv`.

## Result

| File | Rows in | Kept as is | Fixed | Dropped | Rows out |
|---|---|---|---|---|---|
| candidates_activities | 177 | 146 | 31 | 0 | 177 |
| renames_activities | 249 | 223 | 24 | 2 | 247 |

What was checked for every row:
- candidates: the address decodes as a function start (preceded by padding or a jump-table / data gap, with a direct caller, a vtable or table pointer, or a tail `jmp`); no row of `functions.csv` at that address; name unique in `functions.csv` plus both files plus the weather files; size (full control-flow walk) against the note; entry registers (control-flow liveness over all paths) against the note; `ret N`; direct caller count; strings and addresses quoted in the note against the body; table slot of the game mode callbacks.
- renames: old name equals the `functions.csv` name at that address; new name unique; the address sits in the `Class` vtable at the slot in `vfN` (from `rtti_classes.csv`); the role is the same across a class family; the slot is backed by a `call [vtable+4*slot]` dispatch site.

## Candidates: 31 fixed, none dropped

- 26 rows had a wrong `entry regs read`. The previous pass counted the callee-save `push ecx` / `push ebx` (a stack-slot or save idiom) and `or reg,-1` as reads of the register. Corrected to the real set; examples: `game_mode_activity_frame` ECX -> none, `activity_instance_phase_start_level` ECX,ESI -> ESI, `racing_activity_level_instance::update` EAX -> none (EAX is a call result), `sewage_level_instance::on_time_step` ECX,EDI -> ECX, `guardian_angel_state_step` EBX -> none.
- `activity_instance_phase_teleport_to_start` (0x53E900): the activity is the stack argument (`mov ebx,[ebp+8]`, `ret 4`), not ESI.
- `button_mashing_hud_open`: the note prefix "ECX:" removed (no ECX read). `button_mashing_hud_fade`: "stdcall (bool show)" is cdecl with the bool on the stack (`ret 0`).
- `button_mashing_hud_is_open` (0x5E4A90): the note said 1 caller; there are none (the function is unreferenced). Kept as a function (clean body, padding both sides).
- `activity_instance_phase_choose`: ACTIVITY_COOP_SKIP_TEXT is not in this body; it is in the callee 0x538E10. `game_mode_diversion_frame`: DIVERSION_CANCELED comes through `diversion_canceled_text` (0x603EC0).
- Three rows said "reached through a table / vtable" but have no table pointer: `activity_instance_phase_wait_timer` (tail `jmp` from 0x53E6E1), `demo_derby_waiting_on_player_text` (tail `jmp` from 0x5EA338), `taxi_table_parse` (tail `jmp` from 0x628E64). They are real function starts, only the "how reached" text changed.
- Counts: sizes matched 177 of 177; `ret N` 177 of 177; caller counts 176 of 177 (raw call-site count, all spot-checked as real calls); the 44 game mode callbacks are in the stated table slots (mode, index) exactly.

## Renames: 24 reworded, 2 dropped

Structural checks passed for all 249 (old names, vtable slots, unique new names, consistent role per slot per family). The event slots all have dispatch sites in the game mode callbacks: activity level slots 4, 6-16, 20 (via `activity_instance` slot 10); diversion instance slots 2, 3, 5-15; `diversion_type_info` 7 (0x6035A0), 17 (0x603310), 19 (0x602C80 / 0x603440), 32 (0x604940); `activity_instance` 6 (0x53B7E0) and 7 (0x53BD2C); objective 16 (0x5772A8), 19 (0x576C45), 20 (inside each objective update).

Dropped:
- `zombie_level_instance::get_state` (0x5B8900): the body is `(int)((float)[this+0x28] * [this+0x41C])`, a scaled score, not a state getter.
- `fuzz_crime::get_target_flags` (0x5BC2A0): the one slot-18 call (0x5C1C00) uses the result as a loop count (`test esi,esi / jle`), so "flags" is unsupported.

Kept but downgraded to inferred, with "role from the body only, no dispatch site found" (24 rows): `activity_instance::get_type_record`, the three `get_state` rows (activity_level, demo_derby_level, fraud_level), `heli_objective::clear_flag_1a`, and all complication roles (`roll_chance`, `get_value`, `remap_value`, `activate`, `deactivate`). The scan of all functions in 0x530000-0x6C0000 found no `call [reg+0x14/0x1c/0x24/0x28/0x2c]` on a complication, no caller of the activity_instance slot 5 or level_instance slot 17, and no direct callers of any of these addresses. The bodies are unambiguous, so the names stay, but the slot is not proven by a caller.

## Errors found in doc 50 (the doc itself was not edited)

1. The register-convention claims are unreliable: 26 rows report ECX/EBX/EDI as read at entry from a `push` of the register (see above). Ports that trust "ECX = this" for these will be wrong; for example `game_mode_activity_frame`, `guardian_angel_state_step` and the diversion message functions take no register arguments.
2. Phase 1 `activity_instance_phase_teleport_to_start` is described with ESI = activity; it is a stack argument.
3. The activity_level_instance slot 17 `get_state` row is wrong for zombie (scaled score), and no dispatch site exists for slot 17 in any class.
4. The fuzz_crime slot 18 `get_target_flags` role is unsupported.
5. Doc table "Slot roles": the complication roles and `activity_instance` slot 5 are marked confirmed but have no caller anywhere; they are body-only.
6. Some strings quoted for a function live in a callee (ACTIVITY_COOP_SKIP_TEXT in 0x538E10, DIVERSION_CANCELED in 0x603EC0).
7. `button_mashing_hud_is_open` has no caller (doc says 1).
8. Three functions are described as table entries but are tail-jump targets.

Scripts: `C:/Users/pkher/AppData/Local/Temp/claude/F--Saints-Row-2-Decomp/97706a03-2f0c-4efb-b3e4-0dae00846538/scratchpad/act_verify`.
