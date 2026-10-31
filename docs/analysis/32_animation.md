# 32: Animation, rigs, morphs and character flags

Names are in `docs/functions.csv` (subsystems `anim` and `human`, notes ending "(32)"). Humans are in 15_humans.md, cutscenes in 23_cutscenes.md and streaming of animation assets in 05_streamer.md.

## Files

| File | Loader | Contents |
|---|---|---|
| `*.rig_pc` | `rig_load` (0x6DEE40); the player rig `SR2_Main_Rig.rig` by `main_rig_load` (0xB7BE80) | skeleton |
| `*.anim_pc` (`.animx` in asset lists) | streamed with the chunk / action-node assets (0xA819D0-0xA841C0); `anim_list_load` (0xA83400) reads `.anim_list` files | animation clips |
| `*.cpose` | `cpose_load` (0x531140, "CPOSE VER 1", `BONES`) | character poses |
| `*.morph_pc` / `.morphx` | `character_load_body_morphs` (0x6D4150, `cmBod_human_pc_CTE` / `_fac`), `flasher_coat_load_morph` | body and face morph targets |
| `character.xtbl` | `character_table_load` (0x6DFC60: Model, Filename, Morph) | character models |
| `anim_states.xtbl` | `anim_states_load` (0x6EDAB0), `anim_states_parse_states` (0x6ED520) | the animation state machine: states, animation file per state, `Anim_Flags` |

Other pieces:
- `character_parse_cloth_bones` (0x6D9610) reads the cloth bones and tags.
- `anim_trigger_define` (0xCE2BB0) registers named triggers on an animation and rejects duplicates.
- `anim_usage_report` (0x6EBD30) writes the debug `anim_usage_report.txt` of unused animations.

## Vehicles and cutscenes

- `vehicle_interaction_animation_set_parse` (0xB15D00) reads the enter / exit animation grids per vehicle. `vehicle_interaction_find_anim` (0xB1C3E0) looks one up ("Couldn't find animation for %s -> %s -> …").
- Cutscene definitions have sections parsed by `cutscene_parse_characters`, `cutscene_parse_animations`, `cutscene_parse_audios` and `cutscene_parse_options` (0x6D0C00-0x6D2380).

## Player creation morphs

- `player_skin_blend_setup` (0x9EEE10) loads the skin blend normal maps (base / female / male / skinny).
- `morph_values_dump_xml` (0x9EA820) is a debug dump to `d:\morph_values.xml`.

## Human and NPC force flags

Like the AI flags in 31_traffic_vehicle_ai.md, each script-settable character flag has a small setter:
- `human_set_force_flag_<flag>`: `invulnerable`, `never_die`, `never_ragdoll`, `perfect_aim`, `force_holster`, `cannot_exit_vehicles`, `unrevivable`, `dont_regen_health`, `beer_muscles`, …;
- `npc_set_force_flag_<flag>`: `unrecruitable`, `undismissable`, `ho`, `pimp`, `respawn_ally`, `invulnerable_to_gunfire`, `dont_affect_notoriety`, …

That is 55 setters, which a port can treat as one bit table per character record.

## Open

- The animation state machine at runtime (who reads `anim_states.xtbl` each frame), and the blend tree.
- The `.anim_pc` format.
