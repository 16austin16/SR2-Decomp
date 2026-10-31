# 33: Effects, weather, time of day and shadows

Names are in `docs/functions.csv` (subsystems `render`, `effects` and `vehicle`, notes ending "(33)"). The renderer is in 07_render.md and 18_scene.md.

## Time of day and lighting

| Function | Address | What |
|---|---|---|
| `tod_light_sets_parse` | 0x4C8B80 | light sets with `StartTime` / `EndTime` (lights that switch on at night) |
| `tod_light_shader_constants` | 0xB78190 | sets `TOD_Light_Direction` / `_Color` (two lights) for the shaders |
| `sky_sun_moon_setup` | 0x4D5840 | the `sun1` / `moon1` billboards and glows |
| `skybox_parse_colors` | 0x4D5BB0 | `Skybox_Color_1..4` |
| `light_find_by_name` | 0x91DD40 | named scene light lookup |
| `lighting_ambient_debug_print` | 0x4B26E0 | prints level / character / water / reflection ambient |
| `cmd_r_force_time_of_day_lights` | 0x4B52F0 | console command |

Cutscenes override lights per time of day (`cutscene_parse_tod_lights`).

## Weather

- `weather_set_stage_cmd` (0x4DB2D0) is the console command that sets a weather stage by name or number. The stages come from `weather.xtbl` / `wind.xtbl` (`weather_parse_record`, `wind_parse_record`, 26_data_tables.md).
- **Rain**: `rain_effects_load` (`atmos_rain`, `atmos_rain_ground`) and `rain_cone_setup` (the `bb-raincone-single` billboard cone around the camera).
- **Wind**: SpeedTree's `CSpeedWind`, with `CSpeedWind::Load` / `::Save` and `speedwind_load_ini` (`SpeedWind.ini`).
- Footstep audio switches to rain variants (`PC_WALK_RAIN`, `PC_JUMP_RAIN`).

## Shadows

- Pools: `shadow_inline_mempool_init`, `shadow_job_pool_init`.
- `shadow_combiner_setup` (0x51AE20) sets the shadow combine pass shader constants (`Shadow_map_matrix`, `Inv_view_proj`).
- Console commands `cmd_r_shadow_map_type` (uniform or LiSPSM) and `cmd_r_city_shadow_dist`.
- Day and night shadow distances come from the `Shadows` section of `gameplay_constants.xtbl`.

## Water

`water_sim_shaders_init` (0x727890) loads the water simulation passes (`water_physics`, `_detail`, `water_normal_map_gen`). `water_normal_map_setup` (0xB77730) sets up the composite water normal map and wave speeds. Water physics for bodies is `.\havok\havok_water_physics.cpp` (0x724940).

## Effects

- `effect_object_alloc` (0x5043B0) takes an effect object from a fixed pool ("Out of effect_object slots"). Effects are `effect_pc` / `g_effect_pc` assets.
- `cmd_play_effect_delayed` and `cmd_waterspray_circles` are debug commands.
- `searchlight_create` (0x951E10) creates a sweeping searchlight.
- `human_catch_fire` (0x98D0A0) sets a character on fire: sound, then leg and torso fire effects.
- `explosions_refresh_table` (0x4BEB80) reloads `explosions.xtbl`.
- Vehicles: `vehicle_effects_load` (skids, burnout, sparks), `headlight_glare_setup`, and the vehicle table sections `vehicle_info_parse_effects` (engine fire and smoke), `_automobile_flags` and `_water_physics`.

## Vehicle force flags

`vehicle_set_force_flag_<flag>` covers 26 setters, among them `invulnerable`, `bulletproof_glass`, `never_flatten_tires`, `unenterable`, `untowable`, `sirenlights_on`, `force_lights_on` / `_off`, `mission_critical`, `radio_controls_locked` and the hydraulics flags. 0x8492F0 sends them to the co-op partner. The AI flags are in 31_traffic_vehicle_ai.md and the character flags in 32_animation.md.

## Open

- The particle system itself (emitters, update and render), which has almost no strings.
- How the weather stages blend.
