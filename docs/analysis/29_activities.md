# 29: Activities and diversions

Names are in `docs/functions.csv` (subsystem `activity`, notes ending "(29)"). Mission and activity start / end are in 22_missions.md, the `diversion_type_info` slots in `docs/vtable_slots.csv`, and the activity packets (`act_packet`) in 25_network.md.

## Classes

**Activities** are the city side jobs (insurance fraud, Mayhem, Escort, Heli assault and so on). Each one is three classes (03_rtti_classes.md):

| Class | Role |
|---|---|
| `*_info` (from `activity_info`) | static data loaded from the activity's xtbl (`load_from_table`) |
| `*_instance` (from `activity_instance`) | one placement in the city |
| `*_level_instance` (from `activity_level_instance`) | a running level: its `update` and slots 5-7 hold the gameplay |

There are 17 level-instance classes: demo derby (and its special variant), escort, fight club, fraud, fuzz, guardian angel, heli, human torch, pimping, piracy, racing, security, sewage, trafficking, vandalism and zombie. Heli assault adds objective classes (`heli_objective`, `vehicle_kill_objective`, `vehicle_protect_objective`, `aircraft_kill_objective`, `gang_fight_objective`, `large_convoy_objective`, `choplifter_objective`, `liberator_objective`, `scripted_gang_fight_objective`) with `vf20` as their start.

**Diversions** are the always-on side challenges (stunts, taxi, ambulance, tagging, races, cards and so on). There are 56 classes, in two shapes:
- `*_diversion_type` / `*_diversion_type_info`: the type, registered once (named slots in `docs/vtable_slots.csv`);
- `*_diversion_instance`: a running one.

The stunt diversions (jumping, drifting, near miss, oncoming traffic, burnout, two wheels / wheelie, hydraulics, low flying, back seat driver) are types only. They score from the player's vehicle state each frame.

`diversion_try_start_offered` (named earlier) starts an offered diversion. `diversion_cds_init` collects the diversions' content (cds) collections for the streamer.

## Text functions

Most activity and diversion code shows its feedback through small helpers. Each one formats or shows one or a few localized strings. They are named from those strings by this rule:
- the function is under 0x180 bytes;
- its string keys start with `ACT_`, `DIVERSION_`, `COOP_`, `ACTIVITY_` or `REPLAY_`;
- it uses no other non-key strings (icons and `SYS_` / `HUD_` keys are allowed).

The name is the keys' shared prefix plus `_text`. Examples:
- `diversion_taxi_customer_bailout_text`;
- `act_security_objective_text` (`ACT_SECURITY_OBJECTIVE_1/2`);
- `diversion_near_miss_text` ("NEAR MISS");
- `act_level_text` (`ACT_LEVEL_1..n`);
- `coop_kicked_text`.

That gives 59 names.

## Data

The activity data comes from `_activities.xtbl` (types: `xtbl_load_activity_types`) and per-activity tables. Examples are `demoderby_sp.xtbl`, `crowd_common.xtbl` (security) and the diversion tables (`xtbl_load_driveby_diversion`, `xtbl_load_tagging_globals_mp`, `xtbl_load_theft_combos`, `xtbl_load_blackjack`). Their section parsers are in 26_data_tables.md.

## Open

- The level-instance `update` / slot 5-7 bodies (large and only partly named).
- The diversion scoring formulas (they use the `Activity_Constants` and `Coop_Meta_Game` sections of `gameplay_constants.xtbl`, 26_data_tables.md).
