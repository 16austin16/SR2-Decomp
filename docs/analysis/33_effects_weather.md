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

## effect_object_update (0x505480) porting map

`ECX` = effect object (`E`), no stack arguments. Frame about 0x6C0 bytes with a security cookie, plus an `alloca` sized by the list count. Called from `effect_objects_do_frame` for each effect object. The object's info is `I` = E+0xD0.

1. **Return** if `[0x2527280]` is set, or (I+0x28 == 0 and E+0xD4 == 0).
2. If E+0x84 bit 0: unless (E+0x358 == 0 and I+0x2E bit 0x20 is clear) or (E+0x358 set and kind E+0xCC == 1): 0x50AB60(0, 0).
3. Centre E+0x14..0x1C = `0x252A6D0..D8` (reset to the default point).
4. **Timed lists.** For each of the E+0xDC lists (E+0xE0..): `effect_list_expire_timed`; the sum is the expired count.
5. **Part loop** (list E+0xC4, circular, next +0; next computed before the body):
   1. Previous transform: +0x5C..0x64 = +0x2C..0x34; 9 dwords +0x68 = +0x38.
   2. Owner handle +0x20: a human without +0xC8 bit 0x400000 clears bits 0x30 of +0x204 and sets +0x24 = −1.
   3. Attached parts (+0x204 bit 4 clear): follow the owner, through a bone (`rig_bone_transform`), a vehicle seat (`vehicle_seat_occupant`, 0x9C44C0 / 0x9C4430) or the owner transform (0xAAC5C0, 0x6E5DB0, 0x40B6A0 / 0xBDE570). An owner that is gone → `effect_object_part_destroy`.
   4. Physics parts: `physics_body_lookup`; gone → `effect_object_part_destroy`; else `physics_body_get_transform`.
   5. Add the part position to the centre and count it.
6. **Centre and radius.** Centre /= count (× 1/count, f32). Radius E+0x33C = 100000.0 for kind 4; otherwise the largest part distance from the centre (`sqrt` 0x40EA20) + I+0x1C.
7. If I+0x2E bit 4: walk the vehicle lists 0x22A84E0 and 0x22A84D4; a vehicle that hits the test **ends the frame** (return).
8. If I+0x2E bit 8 and there is a local player: get its position (vtbl +8) and compare with the centre; out of range → return.
9. **Visibility.** Kind 4: every part gets +0x204 \|= 0x40, +0x90 = 1.0, +0x205 &= ~1. Otherwise, per part: a fade step of frame dt / 1.5, `cutscene_blocking`, `shape_bounds_overlap_test` against the view, then `render_views_test_bounds`.
10. **Lifetime timer** E+0x354:
    - set and **elapsed**: 0x50AB60(0, 0), E+0x354 = −1, return;
    - otherwise:
      1. In a network game, return if the local player is more than 40 m away.
      2. If E+0x84 bit 0 is clear and I+8 == `[0x2659DD0]` (the explosion kind): `physics_query_aabb` on the centre ± `[0x27974D4]`, and `human_knockdown(human, force)` for each human found. The force is `[0x279748C]`, or `[0x279749C]` in single player.
11. **Main pass.**
    1. A zeroed `alloca` array of one count per list.
    2. Per part: age +0x1C += dt (frame dt, adjusted while `cutscene_in_progress`). Expire when age > (E+0xD4)+0x10.
    3. Looping sound: `audio_voice_position_check`, `audio_play`, 0x47C740 (move the voice).
    4. An optional wobble with `sinf_wrapper` / `cosf_wrapper` when `[0x2526E2B]`.
    5. Attachment transforms (0x6FC760, 0xBDE150, 0xBE4B20, 0xBDE7C0, 0x6FCEC0, 0xBDE750, 0xD58FF0).
    6. The instances: `effect_instance_update` (11 arguments), with the transforms built by 0x5151E0 / 0x515340 / 0xC057D0 / 0xBE2F50; `matrix_orthonormalize`.
    7. Dead parts → `effect_object_part_destroy`.
12. **Tail:**
    1. E+0x340 = the expired count.
    2. Each list's +0xA4 = its alloca count.
    3. If nothing is alive or expired and the kind ≠ 2: call `(*E)[4]()`. The object destroys itself through its own vtable slot 4.

**Port:** keep this as one function with the 12 phases. The part loop uses the "next computed before the body" shape, because parts can be destroyed inside it.

## Open

- The particle system itself (emitters, update and render), which has almost no strings.
- How the weather stages blend.
