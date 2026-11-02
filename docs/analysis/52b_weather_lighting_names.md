# 52b: Weather, lighting and time of day: names and conventions

Companion to `52_weather_lighting.md` (partial). New files only: `docs/candidates_weather_v.csv` holds the same rows (notes end with `(52)`). `functions.csv` and the original candidates were not edited, and no port maps are given here.

Method: function starts from the call graph and padding (all 80 addresses have a call or tail-jump site, or sit after int3 / ret padding), conventions from the callee prologue plus every call site, field meaning from the xtbl parsers (`time_of_day_parse_record`, `weather_time_of_day_parse_ambients`, `weather_parse_record`) and from the global each blender writes. Confidence: high = the role is stated by structure or by a string; medium = role follows from callers and data; low = role inferred from one caller or from the shape alone.

Counts: 80 rows (9 verified candidates + 71 new). High 21, medium 44, low 15.

## 1. The 9 candidates

All 9 are real function starts and none of the names is used elsewhere in `functions.csv`. Callers were checked one by one. Verdicts:

| Address | Name | Verdict | Convention checked against callers |
|---|---|---|---|
| 0x004B4CE0 | `tod_zone_hdr_params_blend` | RENAMED (was tod_zone_tint_blend) | stack arg0 = out1 f32* (read into EDX at entry, the slot is reused as scratch), ECX = out2, EAX = out3, cdecl ret 0 (caller pops 4): each out = out + (lerp(rec.cur, rec.prev, rec.t) - out) * rec.w for the three scalar zone records 0x2612AB0 / 0x2612AC8 / 0x2612AE0 (stride 0x18); the single caller world_lighting_update (0x4DA267) then blends them with 0xF9E514.. / 0xF9E504.., lets script override 0x2658CC4 replace them, scales by the exposure factors 0xE99690 / 0xE99694 / 0xE99698 and stores them in the 15-dword render block 0x276D0A4, so they are post-process (HDR / tone-mapping style) parameters and not a colour tint |
| 0x004B4DA0 | `tod_zone_ambient_scalars_blend` | RENAMED (was tod_zone_ambient_blend) | same shape as tod_zone_hdr_params_blend but for records 0x2612A50 / 0x2612A68 / 0x2612A80 (zone flags 0x100000 / 0x200000 / 0x400000); stack arg0 = out1 (-> L6C), ECX = out2 (-> L70), EAX = out3 (-> L68), cdecl ret 0; the outputs are three of the four ambient scalars (the fourth, record 0x2612A98, is lerped inline) that world_lighting_update puts into block 0x276CFD8 next to the zone ambient colour (4B51F0 on 0x2612A28); the caller is at 0x4D9FC4 |
| 0x004B4E80 | `tod_zone_fade_start_f32` | CONFIRMED name changed for clarity (was tod_zone_fade_start) | ECX = zone record (0x18 bytes: +0 target, +4 from, +8 t, +0xC active byte, +0x10 weight, +0x14 duration), EDX = &target f32, cdecl ret 0: if 0 < [ecx+0x10] re-bases from = lerp(target, from, t), then target = *edx, t = 0 (or t = 1.0 when the weight is 0), active = 1; exactly 12 call sites in tod_zone_update (0x26128F8, 0x26129B0.., 0x2612AE0); the vec3 twin is tod_zone_fade_start_vec3 |
| 0x004B4EF0 | `tod_zone_fade_step_f32` | CONFIRMED + FIXED (arg byte missing) | ECX = f32 zone record, stack arg0 = byte 'animate', ret 4 (stdcall): animate != 0: d = frametime 0xE8437C / [ecx+0x14]; weight [ecx+0x10] += d when active byte [ecx+0xC] is set, else -= d, clamped to [0,1]; t [ecx+8] += d clamped to 1.0; animate == 0 snaps t = 1 and weight = active ? 1 : 0; tod_zone_update passes its own stack byte (cleared while 0x2527D14 etc. say the zone is not animating); 12 call sites |
| 0x004B5110 | `tod_zone_fade_step_vec3` | CONFIRMED + FIXED (arg byte missing) | same as tod_zone_fade_step_f32 for the 0x28-byte vec3 zone records (+0x18 t, +0x1C active, +0x20 weight, +0x24 duration); ECX = record, stack arg0 byte 'animate', ret 4; 5 call sites in tod_zone_update (0x2612910, 0x2612938, 0x2612960, 0x2612988, 0x2612A28) |
| 0x004DDA50 | `weather_tod_segments_select` | CONFIRMED | cdecl arg0 = out struct (0x20 bytes: two weather-time-of-day segments A/B at +0..+0xC, +0x18 t, +0x1C t2), AL = 1 when a schedule exists; uses the clock 0x140E734 or the table 0xE8AF6C; 13 call sites, all of them the blender functions listed in doc 52b (the second push before the call in 0x4E2380 / 0x4E2860 / 0x4E2AA0 / 0x4E0D70 / 0x4E2D80 is a pass-through register, not an argument) |
| 0x004DE390 | `weather_net_sync_send` | CONFIRMED | no args, cdecl: when hosting (0x2528B48 == 1, 0x2528B49 clear) and the timer 0xE9D460 expired builds packet 0x4D with the current and next schedule entries, net_send_packet_to_remotes, re-arms +3000 ms; 4 call sites: weather_update, net_recv_weather, cellphone_lua_register (0x78A5C9), 0x8327DA and a tail jump from weather_force_stage_and_sync |
| 0x004DEED0 | `weather_transition_select` | CONFIRMED (arg order clarified) | cdecl ret 0, C order (&recA, &recB, &t) pushed right to left (t first): from the schedule 0x2526DE8 (index) / 0x2526DEC (0x28-byte entries) picks the current and next weather stage record pointers and the blend t (entry timer 0x68B350 / entry length, smoothed through 0xBF0F50); callers weather_update and water_waves_update (0x727622, a surprise: the water wave code reads the weather blend) |
| 0x004DF0C0 | `weather_ambients_blend` | CONFIRMED | EAX = default block (30 dwords), ECX = out block (30 dwords), 5036 bytes: weather_tod_segments_select, then bilinear lerps of the weather-time-of-day 'Ambients' group (Level / Tree / Char / Water colours, clouds, front / back light, sky colours, stars ...) with per-record override flags; sole caller is 0x4D7E20 (render_stage1 sky pass) |

Notes on the corrections:

- `tod_zone_tint_blend` is renamed `tod_zone_hdr_params_blend`: the single caller multiplies its three outputs by the exposure factors written by `lighting_exposure_set` (0xE99690 / 0xE99694 / 0xE99698) and stores them in the 15-dword render block 0x276D0A4, with a script override at 0x2658CC4. They are post-process parameters, not a colour tint. The earlier text ('out = out + ... w') shape was right.
- `tod_zone_ambient_blend` is renamed `tod_zone_ambient_scalars_blend`: it only handles three scalar records; the fourth ambient scalar (0x2612A98) is lerped inline in `world_lighting_update`.
- The two fade-step functions take a byte argument (`animate`); the old note missed it. With 0 they snap (t = 1, weight = active ? 1 : 0).
- `weather_transition_select`: the C order is (&recA, &recB, &t), pushed right to left, so `t` is the first push. A second caller exists: `water_waves_update` (0x727622).

## 2. Zone record layouts (new, used by nine of the names)

- Scalar zone record, 0x18 bytes: +0 target, +4 from, +8 t, +0xC active byte, +0x10 weight, +0x14 duration. Instances: 0x26128F8, 0x26129B0, 0x26129C8, 0x26129E0, 0x2612A10, 0x2612A50, 0x2612A68, 0x2612A80, 0x2612A98, 0x2612AB0, 0x2612AC8, 0x2612AE0 (12 `tod_zone_fade_start_f32` sites in `tod_zone_update`).
- Vec3 zone record, 0x28 bytes: +0 target vec3, +0xC from vec3, +0x18 t, +0x1C active byte, +0x20 weight, +0x24 duration. Instances: 0x2612910, 0x2612938, 0x2612960, 0x2612988, 0x2612A28.
- Evaluate: scalar `x = (lerp(target, from, t) - 1) * w + 1` for gates, or `out + (lerp - out) * w` for blends; vec3 `default + (lerp(from, target, t) - default) * w` (tod_zone_vec3_blend).
- Render snapshot: `game_render_func` copies 0x81 dwords from 0x26128F8 to 0x2612B00 (instruction at 0x525350), so `0x2612Bxx` = `0x2612(0)xx + 0x208` is the render-thread copy; the game thread uses 0x26128F8.. and the render thread 0x2612B00... This explains why `tod_occlusion_allowed` (rust) has the twin `tod_occlusion_allowed_snapshot`, and why `4E11E0`, `4B46A0`, `4D8B60` read 0x2612Bxx.

## 3. All names, by area

### Lighting zones (0x4B....)

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004B4490 | `tod_zone_find_at_point` | high | EDI = &vec3 point (tod_zone_update copies EAX there first), cdecl ret 0, returns EAX = zone entry (0x84 bytes) or 0: loops the chunk list 0xF70B98 (count 0xF6FBD4), per chunk the zone array [chunk+0x1C4] (count [chunk+0x1C0]); x/y/z box test with fcompp, then 0xBFD510 with epsilon 1e-4f (0xDD0970); callers tod_zone_update and 0xA7AA80 |
| 0x004B4590 | `tod_occlusion_allowed_snapshot` | high | no args, -> EAX: same math as tod_occlusion_allowed (0x4B4A60) but on the render-thread snapshot of the zone table: game_render_func copies 0x81 dwords from 0x26128F8 to 0x2612B00 (0x525350), so 0x2612B00 = 0x26128F8 + 0x208; value = (lerp(cur, prev, t) - 1) * w + 1, returns 0 only when value <= 0 and the active byte 0x2612B0C is set; caller 0x4D8B60 (render_views_build side) |
| 0x004B46A0 | `tod_zone_fog_blend` | medium | ESI = out block (11 floats: 5 scalars defaults 0, 0, 100000.0, 150000.0, 200000.0 from 0xE5C7D8..D0, two vec3), EDI = input fog block (5 scalars, vec3 mid at +0x14, vec3 end at +0x20), returns EAX = ESI; the two vec3 are zone-blended with tod_zone_vec3_blend against the snapshot record 0x2612B90 and the scalars with the snapshot records 0x2612BB8..0x2612BF8; sole caller fog_params_update (0x4D6F00) |
| 0x004B5050 | `tod_zone_fade_start_vec3` | high | ECX = vec3 zone record (0x28 bytes), EDX = &target vec3, cdecl ret 0: like tod_zone_fade_start_f32; if 0 < [ecx+0x20] the 'from' vec3 (+0xC) is re-based to from + (target - from) * t (+0x18), then target = *edx, t = 0 (1.0 if the weight was 0), active byte [ecx+0x1C] = 1; 5 call sites in tod_zone_update |
| 0x004B51F0 | `tod_zone_vec3_blend` | high | EAX = out vec3, EDX = default vec3, stack arg0 = zone record (0x28 bytes), ret 4 (stdcall): out = default + (lerp(from, target, t) - default) * weight [rec+0x20]; every step rounded to f32 through the stack; 17 call sites (world_lighting_update, tod_ambient_colors_update, tod_zone_fog_blend, ...) |
| 0x004B52F0 | `city_instance_record_init` | low | EAX = record (0x3C bytes), no stack args: sets +0 = 0, +4 = 1.0, +8 = 0, +0xC / +0xD = 0xFF, +0x10 / +0x14 / +0x18 = 1.0 (0xDD0BB4), +0x28 = 0xE84A04, +0x2C = 4, +0x30 / +0x34 = 0, +0x38 = 0; 26 callers (city_instance_add, 0x4B7870, 0x4B52D0 ...); no string reference, so the existing name cmd_r_force_time_of_day_lights is misplaced |
| 0x004B53F0 | `cmd_r_force_time_of_day_lights_print` | medium | no args, no direct callers (console table): prints 'R_force_time_of_day_lights %s' (string 0xDDB854 pushed at 0x4B545A) with the on / off texts 0xDDB838 / 0xDDB840 / 0xDDB848 chosen by the bytes 0x2526CC3 / 0x2526CC4; this function, not 0x4B52F0, owns the string; the name cmd_r_force_time_of_day_lights in functions.csv sits on 0x4B52F0, which is a record initialiser |

### Light sets, cutscene overrides, lightning (0x4C....)

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004C8100 | `lightset_light_interpolate` | low | EAX = light, 709 bytes with helpers 0x4C8000 / 0x4C80B0 (frame time 0xE87998): evaluates the light's animated parameters; caller cutscene_lightsets_apply |
| 0x004C83D0 | `lighting_overrides_apply` | medium | EAX, ECX = interpolated light params: writes the script override block 0x2658C64..0x2658CE0 (enable bytes and values), 0xE9A764..78 and the flags 0x2526BCA / BCB / 0x2527393 that world_lighting_update reads; caller cutscene_lightsets_apply |
| 0x004C8770 | `cutscene_lightsets_apply` | medium | no args, caller cutscene_shot_start: for every lightset (0x2526D44 entries at 0x26521A0) and every enabled light, picks the active ones with lightset_light_active_at while a cutscene runs, then lightset_light_interpolate and lighting_overrides_apply |
| 0x004C89B0 | `lightset_light_active_at` | medium | ECX = light, stack arg0 = time f32, ret 4 -> AL: true when the time is inside [[ecx+0x1C] / 30, [ecx+0x20] / 30] seconds (frames at 30 fps) and the optional frame bounds [ecx+0x24] / [ecx+0x28]; caller cutscene_lightsets_apply |
| 0x004C8A40 | `lightset_gather_lights_by_tag` | low | ESI = output object (+0x40 count, max 16), stack arg0 = tag: scans every lightset's lights and collects the ones that carry the tag; callers 0x9ADF30, 0xAE0350 |
| 0x004C8B30 | `lightset_find_by_name` | high | EDI = name (hashed with string_hash_lower), returns EAX = entry pointer (16-byte stride, base 0x2652198, count 0x2526D44) or tail-jumps to tod_light_sets_parse; callers xtbl_load_Vehicle_Customization_Lightset, xtbl_load_custom_start_lightsets, store_chains_parse, 0x554A00 |
| 0x004C96E0 | `lighting_overrides_reset` | medium | no args, 12 callers (cutscene_stop, 0x6C0075, 0x5494B0, 0x555C30, 0x7400A0 ...): crc32 test, then clears the enable bytes of the override block 0x2658C64.. and 0x2526BCA / BCB / 0x2527393 |
| 0x004C9900 | `custom_start_lightset_select` | low | stack arg0 = id: walks the table [0x25289D0] (0x50-byte entries, id at +0, flag byte +0x14) and flags the entry whose id matches, or the first one; callers luafn_pcr_apply_preset, pcr_morph_handshake, xtbl_load_custom_start_lightsets |
| 0x004C99D0 | `lightset_remove` | medium | EAX = entry, stack byte: removes the entry from the 16-byte table 0x2652198 (count 0x2526D44, memmove) and resets the hook 0x2652998 when asked; callers cutscene_stop, 0x7D5FD0 |
| 0x004C9AD0 | `lightset_find_light_by_name` | low | stack name, 199 bytes with __stricmp over the lightset table 0x2526D44; caller cutscene_shotfn_modify_light |
| 0x004CA370 | `lightning_strike_schedule` | low | ECX in, run every 5 s by light_modifiers_update: floorf, two _ftol, table 0xE9B214; allocates a pending strike with lightning_pending_strike_alloc and queues it on list 0x2652A04; reads the wind / weather floats 0x2526D48..D68 |
| 0x004CA6C0 | `lightning_reset` | medium | no args, caller 0x698810 (level start): frees both strike pools (lightning_pending_strikes_free_all, lightning_active_strikes_free_all), rng 0xBDCD90, clears 0x2526D5C / 0xF958C0 |
| 0x004CAE10 | `lightning_pending_strike_alloc` | medium | no args, returns EAX = node or 0: takes the head of the free list 0x2652A08 of the pending strike pool (nodes with links at +0x68 / +0x6C) under the critical section 0xDB80CC; fails when the pool is locked (0x2652A18 set and 0x2652A0C == 0); the pool is named pending_strikes by allocator_init_pending_strikes |
| 0x004CAEF0 | `lightning_pending_strike_free` | medium | ESI = node, EDI = pool: unlinks the node and returns it to the free list under the critical section 0xDB80CC / 0xDB80D0; caller light_modifiers_update |
| 0x004CAF90 | `lightning_pending_strikes_free_all` | medium | no args, caller lightning_reset: walks list 0x2652A04 and returns every node to the free list |
| 0x004CB2E0 | `lightning_active_strike_alloc` | medium | EDI = active strike pool (list 0x2652A5C, nodes with links at +0x48 / +0x4C): pops the free head [edi+0x10] under the lock [edi+0x30]; caller light_modifiers_update; pool named active_strikes by allocator_init_active_strikes |
| 0x004CB390 | `lightning_active_strike_free` | medium | ESI = node, EDI = active pool: returns the node to the free list under the pool lock; caller light_modifiers_update |
| 0x004CB430 | `lightning_active_strikes_free_all` | medium | no args, caller lightning_reset: frees every node of list 0x2652A5C |

### Rain

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004CD640 | `rain_shelter_raycast_update` | medium | no args, sole caller rain_start_init: 3048 bytes with havok raycasts (0x6FA330, 0xD2B7D0, group pool 0x6FB040) around the camera, reads the table 0x2652C80..0x2652CB4 |
| 0x004CE230 | `rain_start_init` | low | no args, sole caller weather_rain_effects_update (only on the first rain frame): 3822 bytes / 599 x87 ops; sets up the rain state 0x2652C58.. from the camera 0x25F5B14.., sin / cos, normalisation, then rain_shelter_raycast_update |
| 0x004CF140 | `rain_cover_probe` | low | no args, caller weather_rain_effects_update when an effect exists: three havok raycasts near the camera, sets the bytes 0x2526D7A / 0x2526D7B and 9 floats at 0x26541C0..E0 |
| 0x004CFC60 | `rain_render` | medium | no args, caller render_frame (0x52A710): 3773 bytes, binds the shader technique, textures through gfx_set_texture_cached, reads the rain level 0x2526D74 / 0x2526D80 |

### Time of day, sky, ambient (0x4D....)

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004D27E0 | `tod_object_node_alloc` | medium | no args, EAX = node or 0: pops the head of the free list 0xF99924 (doubly linked, links at +0x10 / +0x14) and clears the node's links |
| 0x004D3CA0 | `tod_objects_fade_update` | medium | no args, only caller tod_keyframes_select: for every time_of_day_objects entry (array 0xF9E258, stride 0x18: On_Time, Off_Time, Variation as day fractions) compares the clock fraction 0xF9E2EC (wrap aware) and computes a 0..1 fade (distance into the window / variation) for the objects registered in the lists 0xF99A28 / 0xF99A40; reads 0x2526E0C (weather value that shortens the fade) |
| 0x004D41C0 | `tod_lighting_keyframe_blend` | high | EAX = out block (14 floats), cdecl ret 0: blends the two active time-of-day keyframes (indices 0xF9E2F4 / 0xF9E2F8 times 0x250 bytes, t = 0xF9E2F0, base 0x2654208 + 0x210..) and passes the groups through the weather blenders 0x4E2380 (7 floats + colour), 0x4E2860 (3 floats) and 0x4E2AA0 (4 floats); the result is multiplied by 1 - 0x2526D98; sole caller world_lighting_update |
| 0x004D4480 | `ambient_override_approach` | medium | stack (current f32, target f32) -> ST0 (leaves the result on the x87 stack): moves current towards target by frametime 0xE8437C * speed (0.5, or (0.75 - cur) + 0.5 below 0.75), clamped to the target; 18 call sites, all inside tod_ambient_override_step |
| 0x004D4560 | `tod_ambient_override_targets_set` | medium | no args, caller tod_ambient_colors_update: writes the 15 target floats 0xE9D408..0xE9D440 for tod_ambient_override_step |
| 0x004D45E0 | `tod_ambient_override_step` | medium | no args, tail-jumped from tod_ambient_colors_update (jmp sites 0x4D4ED7, 0x4D4EF2, 0x4D4F80): eases the five ambient override vec3 slots 0xE9D3CC / D8 / E4 / F0 / FC (15 floats) towards the targets 0xE9D408..0xE9D440 with ambient_override_approach; the slots feed the console commands level_ambient / char_ambient / water_ambient / refl_ambient (strings at 0x9DB586..) |
| 0x004D6330 | `skybox_setup_for_chunks` | medium | cdecl (chunk list, count, flag, bool): for every chunk with [chunk+0x11C] > 0 calls skybox_parse_colors (when flag) and sky_sun_moon_setup (when bool); hashes 'sun1' / 'moon1' once into 0x3490568 / 0x3490564; callers sky_update and 0x4D7E20 |
| 0x004D63E0 | `sun_world_position` | high | ESI = out vec3: camera position 0x25F5B14.. plus the sun direction offset (0xE98FFC length along the sun axis through 0x406810), or camera + (0, 5000, 0) when no sun axis is set; callers sun_flare_render and sky_update |
| 0x004D64A0 | `tod_tables_load` | high | ECX = this-like, caller 0x520054 (game init): calls xtbl_load_time_of_day, xtbl_load_lens_flares, xtbl_load_time_of_day_objects, xtbl_load_custom_tints then _qsort of the loaded table, stores 0x2658D18 / 0x2658D1C |
| 0x004D64F0 | `tod_state_reset` | high | no args, caller 0x6981E0 (level start): clears the time-of-day state: free list 0xF99924, the two registered-object lists 0xF99A28.. / 0xF99A40.. (12 slots each) and the per-category counters 0xF9E264..0xF9E2E4 |
| 0x004D66B0 | `sun_moon_objects_resolve` | medium | no args, caller 0x698810 (level start): hashes the names 'Sun' / 'Moon' (0x9DCF68 area), looks them up (0x9FAC90, glare find 0x4C61C0) and stores the sky object 0x2526DAC / 0x2526DB0.., the glare tints 0xF9E500 / 0xF9E510 and the flags 0xE9D378 / 0xE9D39C |
| 0x004D6F00 | `fog_params_update` | medium | no args, skipped while 0x25273BE, caller render_stage1: blends the two keyframes' fog fields (0x26543B0..0x26543D8), runs weather blender 0x4E11E0 (fog distances), tod_zone_fog_blend, applies the script override 0x2658CA4 (0x2658CA8.. >= 0), and hands the 5 scalars and 2 colours to 0x524B30 (render fog set) |
| 0x004D7E20 | `sky_render_params_update` | medium | no args, caller render_stage1 (render thread): 3380 bytes / 621 x87 ops, sin tables (sinf x4), zeroes the 0x54-byte block with 0x4D22F0, runs weather_ambients_blend (0x4DF0C0) and skybox preset 0x4D6330; writes 0x2658CF0..F8; reads clock keyframes 0xF9E2F0..F8 |
| 0x004D8B60 | `tod_fog_color_render_update` | medium | no args, caller render_views_build: render-thread twin of tod_fog_color_update: runs weather blender 0x4E0D70 and tod_occlusion_allowed_snapshot, the override block 0x2658C64.. and the snapshot 0x2612B00 |
| 0x004D8FD0 | `ambient_color_get_slot3` | low | register outs: the fourth ambient slot (override 0xE9D3F0 / computed 0x2658E0C) plus two extra floats 0x2658E54 / 0x2658E58 (light block B); caller 0xB76180 |
| 0x004D90D0 | `ambient_color_get_slot0` | medium | EAX = out vec3: if 0xE98948 == 0 returns (1,1,1); == 1 returns the console override 0xE9D3CC; else the computed colour 0x2658DE8; callers 0x9ADF30, 0xB7B410 |
| 0x004D9120 | `ambient_color_get_level` | medium | EAX = out vec3: same shape for override 0xE9D3D8 / computed 0x2658DF4; callers lighting_ambient_debug_print, city_build_render_batches, r_particle_inst::render, r_particle_ribbon_inst::render -> the level ambient colour |
| 0x004D9190 | `ambient_color_get_tree` | medium | EAX = out vec3: override 0xE9D3E4 / computed 0x2658E00 (same shape as ambient_color_get_level); callers include r_tree_inst::render, so this is the tree ambient colour |
| 0x004D91E0 | `ambient_color_get_slot4` | medium | EAX = out vec3: override 0xE9D3FC / computed 0x2658E18; callers city_build_render_batches, 0xADF430, 0xAE0350 |
| 0x004D9480 | `tod_object_register` | low | ECX = this, EDI = category, two stack args: allocates a node (4D27E0), compares the clock (0x140E734 or table 0xE8AF6C) with the On / Off times 0xF9E258.. and links the object into the category lists 0xF99A28 / 0xF99A40; 6 callers (searchlight, vehicle, movers) |
| 0x004D9620 | `tod_object_unregister` | low | EDX = category, stack (object, id): finds the node in lists 0xF99A40[cat] / 0xF99A28[cat], unlinks it and decrements the category counter at 0xF9E26C + 0x18 * cat; callers searchlight::vf9, vehicle::activate, 0x4A8A50 ... |

### Weather (0x4DA.. .. 0x4DF....)

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004DA690 | `weather_wtr_splash_init` | low | no args, caller game_init_systems: hashes the string 'wtr_splash_direction' and 2 more names, clears 0xF9E2FC / 0xF9E550 / 0xF9E554 (the list updated by list_f9e550_update); role inferred from the hashed names only |
| 0x004DD5A0 | `weather_pick_random_stage` | high | returns EAX = weather record (the incoming EAX is returned unchanged when a forced weather 0x2526E14 exists): rng_float01 (0xBDCE30) walked over the cumulative weights [rec+8] (stride 0xD8, count 0x2526DCC, base 0x2526DD0); the last record when the sum is never reached; 2 call sites in weather_schedule_update |
| 0x004DD630 | `weather_stage_random_duration` | high | EAX = weather record, returns ST0 (f32): with a forced weather, clock 0x140E73C * 0xE5C380; else rng between [rec+0xC] - [rec+0xC] * [rec+0x10] and [rec+0xC] (base duration and variance); 4 call sites |
| 0x004DD6C0 | `weather_schedule_generate` | medium | no args (alloca_probe_16 frame with security cookie): fills the 0x28-byte schedule entries 0x2526DEC (count 0x2526DE4) from random stages (weather_pick_random_stage, weather_stage_random_duration); sole caller weather_schedule_update |
| 0x004DDD50 | `weather_schedule_update` | medium | no args, caller weather_update (first call): rebuilds or advances the forced / random schedule when the stage changed or the schedule ran out (reads clock 0x140E728..73C, 0x2526E10..E24, host flags 0x2528B2B / B56 / C14); 1557 bytes |
| 0x004DE5D0 | `weather_state_reset` | high | no args, caller 0x698810 (level start): memset of the schedule area, finds the record 'clear skies' (weather_find), sets 0x2526DE4 / DE8 and the timers 0xE9D45C / 0x2526E18 |
| 0x004DED80 | `weather_force_stage_and_sync` | high | EAX = stage index, caller weather_set_stage_cmd, no return: 0x2526E10 = &record[index] (stride 0xD8, base 0x2526DD0), re-arms the net timer 0xE9D460 from 0xE843B4 (wrapped at 0x6B49D200) and tail-jumps to weather_net_sync_send |
| 0x004DEE30 | `weather_find_index` | high | cdecl (name) -> EAX = index into the weather table or -1 (stricmp over stride 0xD8 records); callers xtbl_load_skybox_effects, cutscene_shotfn_weather_stage, mp_bh_custom_match_general::load_from_table; index twin of weather_find |
| 0x004DF030 | `sky_weather_params_gather` | medium | EAX, ECX, stack = three out vec3: copies the nine weather_update results 0x2659828..0x2659848 and calls 0x4E3230; caller sky_update |

### Weather to time-of-day blenders (0x4E....)

| Address | Name | Conf | Evidence and calling convention |
|---|---|---|---|
| 0x004E0470 | `tod_ambient_colors_weather_blend` | medium | EAX, ECX register in/out blocks + 3 floats on the stack, ret 0: bilinear weather blend (2 KB) of the ambient colours; sole caller tod_ambient_colors_update; starts with weather_tod_segments_select |
| 0x004E0CA0 | `tod_misc_params_weather_blend` | medium | no register args, 208 bytes: weather blend of the misc parameters; sole caller tod_misc_params_update, which writes 0x2658E68..E78 |
| 0x004E0D70 | `tod_fog_color_weather_blend` | medium | 1131 bytes: weather blend of the fog colour; callers tod_fog_color_update and tod_fog_color_render_update |
| 0x004E11E0 | `tod_fog_distances_weather_blend` | medium | EAX = in (5 floats), ECX = out (11 floats), 2 KB: weather blend of the fog distances and strengths, combined with the snapshot zone record 0x2612B00; sole caller fog_params_update |
| 0x004E19E0 | `tod_light_block_b_weather_blend` | medium | EAX / ECX in-out blocks, 2454 bytes: weather blend for block B; sole caller tod_light_block_b_update |
| 0x004E2380 | `tod_lighting_colors_weather_blend` | medium | EAX = in/out block (7 floats + colour), stack arg = block, cdecl: first weather blender called by tod_lighting_keyframe_blend |
| 0x004E2860 | `tod_lighting_scalars_weather_blend` | medium | 3-float blender called second by tod_lighting_keyframe_blend, stack arg = in/out block |
| 0x004E2AA0 | `tod_lighting_ambient_scalars_weather_blend` | medium | ESI = in (4 floats), EDI = out (4 floats): third blender called by tod_lighting_keyframe_blend; its output is the ambient scalar group at out+0x28.. |
| 0x004E2D80 | `tod_sun_weather_blend` | medium | 1189 bytes: weather blend of the sun colour / glow; reads 0xF9E5DC..F9E5F0 and the scroll values 0x2526E00 / 0x2526E04 that weather_update advances; sole caller tod_sun_update |
| 0x004E3230 | `sky_weather_params_blend` | low | EAX, ECX register block pointers, 1605 bytes: weather blend of the sky parameters gathered by sky_weather_params_gather |
| 0x004E3880 | `sky_param_weather_blend_a` | low | stack arg = in/out float, 386 bytes: single-scalar weather blend (record fields +0x17C / +0x19C); caller sky_update at 0x4D73EC |
| 0x004E3A10 | `sky_param_weather_blend_b` | low | stack arg = vec3 out (ECX): single-value weather blend; callers sky_update (0x4D71EA) and sky_weather_params_gather |

## 4. Findings worth knowing

1. `cmd_r_force_time_of_day_lights` is attached to the wrong address. The string 'R_force_time_of_day_lights %s' is pushed at 0x4B545A, inside the function at 0x4B53F0 (no direct callers, console table). The function at 0x4B52F0 is a 0x3C-byte record initialiser with 26 callers (`city_instance_add`, ...). Both rows are in the CSV so the names can be swapped.
2. `light_modifiers_update` (0x4CA750) works on the pools the allocator initialisers call `pending_strikes` (list 0x2652A04, 0x70-byte nodes) and `active_strikes` (list 0x2652A5C, 0x50-byte nodes), and on `lightning.xtbl` (Probability, TOD_Overrides). The 'light modifiers' are lightning strikes; the eight pool helpers are named `lightning_*`.
3. The 13 blenders called by `weather_tod_segments_select` are the weather layer on top of every time-of-day keyframe blend: the keyframe interpolation (0x4D41C0, 0x4D4820, 0x4D4F90, 0x4D5300, 0x4D5470, 0x4D5670, 0x4D6F00) is done first and then passed through a `*_weather_blend`. The weather-time-of-day record groups are the xtbl fields Ambients (Level / Tree / Char / Water), Clouds, Front / Back lighting, Sky_Color_1..4 with Color_Height_1..4, Stars, Mountains, Fog, Water, Main_Light.
4. `water_waves_update` calls `weather_transition_select`: wave strength follows the weather blend.
5. The five 'ambient colour' slots 0xE9D3CC / D8 / E4 / F0 / FC (override) against 0x2658DE8 / DF4 / E00 / E0C / E18 (computed) are switched by the console flag 0xE98948 (0 = white, 1 = override, 2 = computed). Level (0x2658DF4) and tree (0x2658E00) are fixed by their callers (`r_particle_inst::render` / `city_build_render_batches`, `r_tree_inst::render`); the other three slots are named by index because only 4 of the 5 have console names (level, char, water, refl).
6. Rain intensity 0x2526D74 is read by many more consumers than the rain code: `audio_ambient_emitters_update` (via 0x473930 and 0x474440), `ai_order_update` (0x435970), the ambient spawn code (0x913E80, 0x936310), a ped idle picker (0x943BB0), three footstep / foley sites (0x9932F0, 0x999800, 0x9A5540), 0x9A53D0 and `vehicle_update` (0xAD0CF0). They are good names for the next round (they are not named here because their non-rain half was not analysed).
7. `tod_ambient_override_step` (0x4D45E0) is reached only by `jmp` from `tod_ambient_colors_update`, so the usual call-graph tools miss it.

## 5. Not done

- Exact field meaning inside the 13 blenders (which float is which weather field) was not mapped; the names follow the caller and the written global.
- Low-confidence names (15) are marked in the table and should be re-checked before they are used in a port.
