# 26: Data table parsers

Names are in `docs/functions.csv` (subsystem `data`). Their notes start with "auto: section/record parser" and list the first element and fields.

The game reads its tuning data from about 330 `*.xtbl` files (XML tables, 00_initial_overview.md). Each table has a loader, `xtbl_load_<table>`, named in earlier rounds from its file name. Most loaders hand each section or record to a small parser function. Those parsers read elements by name, so their strings show which section they handle.

## Naming rule

An unnamed function is named `<table>_parse_<section>` when:
- every caller chain (up to two levels) leads to the same `xtbl_load_<table>`; and
- it references at least one element-name string.

The section is the first element name it reads, lower-cased. When that first element is a generic field (`Name`, `Number`, `Start`, `Track`), the function parses one record and is named `<table>_parse_record`. Repeats get `_2`, `_3`.

This gives 147 names. One candidate was left out: it calls the `vcust_populate_menu` UI hooks rather than parsing.

## gameplay_constants.xtbl

`xtbl_load_gameplay_constants` (0x550430) has 45 section parsers (0x548140, 0x54D150-0x5503C0), one per tuning group:

| Area | Sections |
|---|---|
| Health and damage | `Death_and_busted`, `player_health`, `Fall_Damage`, `Fire`, `sticky_fire`, `Dripping_Wet`, `hit_by_car`, `vehicle_collision`, `ragdoll_damage_factors` |
| Combat | `Melee_attack`, `Firearm_attack`, `Rocket_accuracy`, `Flamethrower`, `Stungun`, `PepperSpray`, `Kneecapper`, `throwing_constants`, `Wieldable_Prop_Throw`, `Grab_Quickkill_Human_Shield`, `Grab_Offsets`, `Bust_Offsets`, `Fight_Club` |
| AI | `Combat_AI`, `Vehicle_Evade_AI`, `Idle_AI`, `Ho_Pimp_AI`, `TauntReactions`, `ControlPatterns` |
| Vehicles | `Vehicle_Interaction`, `Helicopter`, `Watercraft_Params`, `Siren_Whoop_Params`, `Garage` |
| World and rendering | `Shader_LOD`, `Shadows`, `Object_Glow_Colors`, `Human_Fine_Collision`, `Fat_bones`, `Drunk_Sway` |
| Progress | `Cribs`, `Hood_Awards`, `Item_drops`, `Activity_Constants`, `Coop_Meta_Game`, `Controls` |

For a Rust port this is the list of global tuning structs. Each parser writes one group of globals next to the others, and the field names in the notes give their order.

## Other tables

The other 102 parsers spread over 39 tables. The largest are `cs_camera_list` (9; cutscene cameras), `camera_free` (8), `cheats` (8), `radio_groups` (5), `user_interface` (5) and `customization_items` (4). Smaller groups cover foley, weather and time of day, spawn info, missions, music store, parked vehicles, notoriety spawn and player creation.

## Open

- The in-memory layout each parser fills. The field names are known; the offsets still have to be read from each parser.
- Table parsers that more than one loader shares (left unnamed by the rule).
