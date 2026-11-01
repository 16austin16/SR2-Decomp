# 22: Missions and activities

Names are in `docs/functions.csv` (subsystem `mission`). The Lua host and `script_load_mission` are in 09_script.md, saves in 13_savegame.md.

## Tables

| Table | Loader | Contents |
|---|---|---|
| `_missions.xtbl` | `xtbl_load__missions` (0x69F120) | 0x14-byte records at `0x145A4A4` (count `0x145A4AC`); flag +8 is saved as one bit per mission (save section 15) |
| `activity_types.xtbl` | `xtbl_load_activity_types` (0x53A820) | activity types |
| per-activity xtbls | `activity_info::load_from_table` and its subclasses (03_rtti_classes.md) | for example `mp_bh_snatch.xtbl` |

Activities are three classes per activity: `*_info` (from `activity_info`), `*_instance` (from `activity_instance`) and `*_level_instance` (from `activity_level_instance`). For example, the `trafficking_*` triple. Diversions use `diversion_type_info` / `diversion_instance`.

## The current mission

`0x27B3C60` points to the active mission / activity record:

| Off | Field |
|---|---|
| +0x00 | id |
| +0x10 | flags |
| +0x14 | name (for example "tss02") |
| +0x3C | **kind**: 0 story mission, 1 activity, 2 multiplayer, 3 mission (co-op variant) |
| +0x90 | activity level data (+0x12C) |
| +0x98 | name of the Lua function to run on completion |

`0x27B3C68` / `0x27B3C69` hold "ended with success" / "first completion".

## Starting

- A mission or activity is loaded by **`mission_load_script`** (0xA24810):
  1. Reset the mission audio (`audio_init` parts, the "audio hash").
  2. Load the custom-match weapons in multiplayer.
  3. Load the mission's **cutscene / content collection** `cds_<name>_collection` into the streamer (0xA57770).
  4. Then 0x477630, 0x46E400, 0x46AE40, 0xA20DD0.
  5. **`script_load_mission`**: `<name>.lua`, then the threads `<name>_init` and `<name>_main`.
- Starting points in the world are the "mission start" triggers (save section 22, 0x54CCA0), checked in `sim_frame`.
- Mission interiors and areas (for example `SR2_IntSRMisChurch_AL`) are ordinary streamer zones / chunks. The mission's Lua script asks for them through the streaming bindings, and `cds_<name>_collection` preloads its content (05_streamer.md).

## Ending

Lua calls `mission_end_success` / `mission_end_failure` (`luafn_mission_end_success` 0xA39990 → **`mission_end`** 0x6A2B40(success)):
1. Only for the active mission.
2. Clear the camera smoothing, switch the game mode to 3 (`0xE96C04`), and reset the mission's HUD / help state.
3. **By kind**:
   - **0 / 3 (missions)**:
     - Success: start the completion Lua function (+0x98) as a thread, plus 0xCE0740. "tss02" also gets a 3 s delay (0x9DBA90).
     - Failure: the mission-failed flow (0x4C46F0; reason `0x1450380` = 5).
   - **1 (activities)**: the next activity level (`0x27B3C6C`). On success, 0x69E440 (unlock / reward) when the level allows it. On failure, the retry flow (0x6A17B0 / 0x6A18A0).
   - **2 (multiplayer)**: send packet 0x6D/0x1F and end on every machine (0x69E440, 0x6A4AA0).
4. Reset notoriety (0x55F330), then the stat callbacks.

`luafn_mission_script_cancel` (→ 0x69DBF0) aborts the mission script.

## Checkpoints

`luafn_mission_set_checkpoint` → **`mission_set_checkpoint`** (0x6A7D90; (name, flag)):
1. Only when no checkpoint is in progress (`0xEAAF44`, `0xEAAF48` < 0).
2. 0x6A1250, then 0x6A1440(`0x27C43AC`, flag), which builds a save with `gamesave_save` into memory. A **checkpoint is an in-memory save game** (13_savegame.md).
3. In co-op, send the checkpoint name to the partner (packet 0x6C/2).
4. 0x6A9F10.

Restarting from a checkpoint loads that buffer back through `gamesave_apply`.

## mission_fail_flow_do_frame (0x6A5BB0) porting map

From `sim_frame` section :1b; no arguments. The state is the failure reason / step `[0x1450380]`, set by `mission_fail_flow_begin` (0x6A1950) and advanced here. The mission record is `M = [0x27B3C60]`; its kind is M+0x3C (0 activity, 1 mission, 2 scripted, 3 other).

**Session gate** (used before the switch and again inside cases 2, 3, 8 and 9). With a network session `S = [0x2528C14]`, the flow waits (returns) unless:
- `[0x2528B56]` is clear or `[0x2528B2B]` is set; and
- S+0x3F8 is set.

This is written "gate" below.

**Prologue.** If (`[0x2527BD2]` == 1 and `[0x2527BD3]`) or `[0x2527BDB]`: call 0x696120. Then the gate. Then `switch [0x1450380]` through the table `0x6A644C` (11 entries; above 10 → return).

The fade helpers named here:
- `screen_fade_out(ESI = &colour, cdecl f32 1.0)` and `screen_fade_in`;
- `screen_fade_is_full` and `screen_fade_is_clear`: exact compares of `[0x2526DA0]` with 1.0 and 0.0;
- `[0x2526D9C]` is the fade direction (1 = out, 2 = in).

"Fade out" below means: if `[0x2526D9C]` ≠ 1, call `gfx_set_color_bytes` and then `screen_fade_out(1.0)`.

| Case | What it does |
|---|---|
| 0 (activity retry) | 0x6A17B0(cdecl 0), 0x6A18A0, 0x832970; return. |
| 1 | 0x4C46F0; return. |
| 2 (fade, then the fail cutscene) | 0x4C46F0. Cutscene record = M kind 1 ? (M+0x90)+0x154 : M+0x98. Fade out. Once `screen_fade_is_full`: if there is a record, `cutscene_play(cdecl *rec, kind == 2, 0)`; `[0x2527C9C]` = 0; `conversation_stop`. For kind 1: the position from M+0x90 vtbl +0x5C (+0x28, or the player +0x20 when null) → 0x6C8FB0; if vtbl +0x60 is non-zero and `[0x2527D23]` ≠ 1: 0x6BEBB0. Kind 2: `[0x2526D96]` = 0. Then, if the gate passes: **state = 3**; 0x4C46F0. |
| 3 (wait for the cutscene) | Unless `cutscene_blocking` or `cutscene_running`: **kind 2**: call the callback M+0x90(cdecl M+0x94) if set, then `mission_end(1)`, return. **Otherwise**, if the gate passes: **state = 4**. Then by kind: kind 1 → `0x14091B0` = M+0x90, `0x14091B4/B5` = 0; if `[0xE96C00]` ≠ 0, also `[0xE96C04]` = 0 and `[0x2527BB4]` = 0x14091B0. Kinds 0 and 3 → if `[0xE96C00]` ≠ 4: `[0xE96C04]` = 4, `[0x2527BB4]` = 0. Other kinds: nothing more. |
| 4 (retry / revive) | Return while `cutscene_blocking`. **(a)** If kind is 2, or 0x6045E0() is true, or 0x4FB4C0() is false: <br> 1. If the timer `0xEAAF44` is set and has elapsed: `conversation_stop` when `[0x2527C90]`; `mission_end(0)`; `0xEAAF44` = −1; return. <br> 2. Return unless the timer `0xEAAF48` is set and has elapsed. If `[0x2526D9C]` == 1 and the fade is not full: return. <br> 3. Fade not full: `conversation_stop` when `[0x2527C90]`; fade out; `[0x2526D96]` = 0; return. <br> 4. Fade full: with no revive record `[0x27B3C70]`: when `[0x2527B14]` == 0, `mission_end(1)` and `0xEAAF48` = −1. With one: `cutscene_play(cdecl rec, 0, 1)`, then the position of 0xA5EB50() +0x28 → 0x6C8FB0, 0x6BEBB0 (as in case 2), `0xEAAF48` = now + 500 (wrapped), 0xD91F70, `[0x27B3C70]` = 0. <br> **(b)** Otherwise (the retry prompt): `[0x2527260]` = 0; unless `coop_session_active`: 0x699840(1). Build the failure conversation by kind (0, 1 and 3 with different line sets; kind 1 with 0x53C4D0() == 0xD uses 0x7F49E0 twice), using `string_hash_lower` into 0x145036C and 0x7F4AD0 into 0x1450370; `slot = conversation_start(...)`; `[0x2527C90]` = slot +0x920. For a kind other than 0, 1 or 3, no conversation is started and **`[0 + 0x920]` is read: a null read**. Kind 2 never gets here, because it always takes (a). |
| 5 | **Nothing** (the table entry is the default return). The "mission failed" reason 5 is driven elsewhere. |
| 6 (revive in place) | 0x4C46F0; fade out. Once full: if 0x98E250(player): 0x98CEB0(1). Player +0x18A4 = −1; +0x1160 &= ~0x20; `[0x2527C80]` +8 \|= 1; 0x6A70D0(cdecl `[0x2527C80]`, `[0x2527CA0]` == handle +0x44); 0x493120; 0x4F5210; player +0x1163 &= ~0x20. The state is **not** changed here. |
| 7 | 0x4C46F0; fade out. Once full: 0x9F2100, 0x9DF230, 0x4F5210, then **case 10**. |
| 8 (restart from the checkpoint) | 0x4C46F0; fade out. Once full: player +0x18A4 = −1, +0x1160 &= ~0x20. If the gate passes: **state = 9**, `mission_restart_from_checkpoint(1)`, `missions_abort_all`, 0x98E250 → 0x98CEB0(1), 0x988170(player); if player +0x595: 0x9ACC90(player, 0), 0x9ACB10(player, 0); `render_thread_wait_frame(0)`, 0x4F5210, `screen_fade_in(0)`; `[0x25F5BE8]` &= ~2; 0x544E60. |
| 9 (fade back in) | 0x4C46F0. If `[0x2526D9C]` ≠ 2: `gfx_set_color_bytes`, `screen_fade_in(0)`. Return until `screen_fade_is_clear`. Gate. **state = 0**; 0x6A17B0(cdecl 1); 0x6A18A0; then **falls into case 1** (0x4C46F0 a second time). |
| 10 | `mission_fail_flow_begin(EAX = 1)` (0x6A1950): restarts the flow as reason 1; return. |

**Links to other docs.**
- `0xEAAF44` is the case-4 timer before `mission_end(0)`. While it is set (≥ 0), the Lua `character_is_dead` reports **every living character as dead** (doc 09, "Ten top bindings"). So during the fail wait, scripts see their characters as dead.
- `0xEAAF48` paces the revive: it is re-armed to now + 500 ms after each revive cutscene.

**Port.**
- A state machine with the 11 entries above. Keep the case 9 → case 1 fall-through and case 7 → case 10.
- The fade-completion tests compare exactly with 1.0 / 0.0. A port of the fade must land on exactly those values. Whether the fade code clamps to them was not checked here.

## Open

- The activity classes' slot names, which the diversion work only partly covered.
- How the retry / fail screen picks between restarting from a checkpoint and quitting.
