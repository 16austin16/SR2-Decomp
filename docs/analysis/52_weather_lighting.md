# 52: Weather, lighting, time of day (PARTIAL)

Status: the run was stopped early. Only the items below were verified. The 10 large port maps and 15 small recipes are NOT finished.

## Verified or partly verified
- Thread: `weather_update`, `weather_rain_effects_update`, `light_modifiers_update`, `world_lighting_update`, `world_focus_point_update` run on the game thread (sim_frame:6,7). `sky_update` is called from `game_render_func` (game thread); 0x4D7E20 is called from `render_stage1` (render thread).
- The blend family: `weather_tod_segments_select` (0x4DDA50) is called by 13 blenders: 0x4DF0C0 (5036 bytes), 0x4E0470, 0x4E0CA0, 0x4E0D70, 0x4E11E0, 0x4E19E0, 0x4E2380, 0x4E2860, 0x4E2AA0, 0x4E2D80, 0x4E3230, 0x4E3880, 0x4E3A10. All are x87-heavy bilinear blends.
- Bilinear shape (verified in 0x4DF0C0 with versioned stack slots): X = lerp(A,B,t2), Y = lerp(C,D,t2), out = lerp(X,Y,t). Lerp is `(b-a)*t + a` with f32 stores to stack slots between steps.

### weather_update (0x4DE770, 1154 bytes, no args, cdecl)
1. Timestamp check: if [0xE9D45C] >= 0 and the wrapped difference to now [0xE843AC] (wrap 0x6B49D200, half 0x35A4E900) shows it expired, [0x2526E18] = 0.
2. Call 0x4DDD50.
3. If [0x2528C14] set and ([0x2528B56] == 0 or [0x2528B2B] != 0) and [[0x2528C14]+0x3F8] != 0: call weather_net_sync_send (0x4DE390).
4. weather_transition_select(&recA, &recB, &t). Lerps (b-a)*t + a, f32 store each: rain +0x18 -> [0x2526D74]; wind +0x3C -> [0x2526D54] (x10.0 double 0xE5C1B8 if [0x27A6A40] & 8), +0x40 -> [0x2526D58]; vec3 +0x58 -> 0x2659828; +0x64 -> 0x2659834; +0x70 -> [0xF9E5D8]; +0x74 -> 0x2659840; +0x44 -> [0x2526E08]; +0x48 -> 0x265984C; +0x54 -> [0x2526E0C]; +0x90..+0xA4 -> 0xF9E5F0,EC,E8,E4,E0,DC.
5. Sky scroll: [0x2526E00] += lerp(+0xA8)*dt; [0x2526E04] += dt*lerp(+0xAC); both wrapped into [0,1) by repeated subtraction of 1.0 (fcom, `test ah,1`, `je`: loop while value >= 1.0); byte [0xF8480E] = 1.
6. Final: fcompp of recB.rain (ST0) against recA.rain, `test ah,5; jnp` jumps when recB.rain < recA.rain, leaving the byte = 1; otherwise the byte = 0.

### weather_rain_effects_update (0x4CF840, 1018 bytes, no args)
- If byte [0x2526D78]: [0x2526D74] = [0x2526D7C]. If NOT(0 < rain) (`test ah,5; jp` jumps when 0 >= rain or NaN): clear [0x2526D79], [0x2526D80] = 0. Else on first rain frame set [0x2526D80] = 3 and copy the pairs 0xE9D318..0xE9D330 into 0x2652FC4 (stride 0x310), set [0x2526D79] = 1, call 0x4CE230.
- Effect 1 (handle [0xF99900]) exists while 0.5 < rain (`test ah,0x41; jnz` skip when 0.5 > rain is the jump-over case: jumps when a > b or NaN with a = 0.5, b = rain, so created when rain >= 0.5). Effect 2 ([0xF998FC]) while rain >= 0.1 (jumps when rain < 0.1 or NaN). Both made with 0x4171C0 + effect_start (0x50E330); stopped with effect_stop_by_handle (0x50DDB0) otherwise.
- If either exists: call 0x4CF140, then scale each effect: first by (rain-0.5)/0.2, second by (rain-0.1)/0.1 (double constants), via 0x50AA60, 0x50AD20, 0x50AD90(f32).
- Full x87 maps of the remaining functions were not produced.

## Not done
Maps for light_modifiers_update, world_lighting_update, world_focus_point_update, 0x4CE230, 0x4CD640 (rain shelter raycast helper), 0x4D7160, 0x4D7E20, 0x4DF0C0 (beyond the shape above), and all 15 small recipes. Tools in the scratchpad `weather` folder (ana.py, cview.py: symbolic x87 simulator with versioned stack slots and polarity decode) can regenerate listings.
