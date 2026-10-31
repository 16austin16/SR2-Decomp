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

## Open

- The activity classes' slot names, which the diversion work only partly covered.
- How the retry / fail screen picks between restarting from a checkpoint and quitting.
