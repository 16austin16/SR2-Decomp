# 42 — Render, UI and streaming: new names and port candidates

This round names 63 functions that `docs/functions.csv` doesn't have yet. They come from three places:
- the render frame (`render_frame`, `game_render_func`, the gfx layer at 0xCF0000-0xD25000);
- the vint UI and the localized-text layer under it (0x7F3000-0x7F5000, 0xB87000-0xB95000);
- streaming (rsched, hdd, peg loading).

How they were found: a call-graph walk from `game_render_thread_frame`, `render_frame`, `game_render_func`, `game_build_render_batches`, `vint_render_documents`, `interface_do_frame` and `gameplay_streaming_frame`. Within each address range the unnamed functions were then ranked by call count. Every address is either a direct call target preceded by int3/ret padding, or a call target right after a jump table (0xB93920, 0xCFDD80, 0x963750).

The rows are in `docs/candidates_render.csv`, in the `functions.csv` format:
- **`documented`**: the role was read from the code (47 rows);
- **`named`**: the role is inferred from callers, lists or strings (16 rows).

## What changed in the picture

- **The `game_render_func` step 3 "fills" collect dynamic lights, not render instances.** Doc 18 guessed render instances. The functions are 0xAE0FD0, 0x505350, 0x94DFA0, 0x9556A0 and 0x963750; 0x4C85D0 is another light source that is still unnamed. Each one walks an object list and appends to a 64-entry light array, `0x33D4DD0` (0x54 bytes each). The array's count `0x252A364` is set to 0 by `game_render_func` just before these calls. Lights are added by `render_light_add_point` (type 0) and `render_light_add_spot` (type 2). Two step-3 calls are not light collection:
  - `render_global_constants_update` (0x524060) sets the shader time / time of day vector c8;
  - 0x4D7160 is the sky (it reaches `sky_sun_moon_setup` and `skybox_parse_colors`).
- **`vint` Lua calls.** C++ calls into the vint Lua state go through a pool of 0x260-byte call records (pool 0x31A4A80, free ring 0x252A1C4, active ring 0x252A1C8):
  1. `vint_lua_call_alloc`;
  2. `vint_lua_call_set_arg` (579 call sites);
  3. `vint_lua_call_execute`;
  4. `vint_lua_call_free`.

  Data items are set with `vint_dataitem_set_value` (510 call sites).
- **Localized text.** `lstr_get` (0x7F49E0) has 1335 call sites, which makes it one of the most-called functions in the game. It and `lstr_get_narrow_fallback` sit on `lstr_lookup` → `lstr_find_by_crc`.
- **Shader lookup by name** (`gfx_shader_find_by_hash`, `gfx_shader_find_constant`, `gfx_shader_find_sampler`). This is how the post passes and Bink video find their shaders. The name strings in `post_shaders_resolve` (0x517300) give the post-process technique names.

## Names

| Address | Name | Subsystem | Role | Convention (first clause) | Evidence |
|---|---|---|---|---|---|
| 0x4920E0 | `bink_video_shader_resolve` | render | confirmed | No args | strings 'bink_video', 'tor', 'Luminance', 'Color1', 'Color2'; called by video_stop |
| 0x4B0E50 | `neighborhood_find_by_name` | world | confirmed | cdecl (char* name), caller pops 4 | __stricmp over 0x2526C9C (0x50-byte records); xtbl_load_neighborhoods |
| 0x4B19F0 | `district_index_at_point` | world | confirmed | cdecl (&pos, district* hint), caller pops 8 | point-in-volume 0xBFD510 over districts 0x2526C94 (0xA8 bytes) |
| 0x4B2680 | `cash_from_dollars` | util | confirmed | ESI = int* out, stdcall (f32 dollars), ret 4 | x100, +/-0.5, _ftol2_sse, clamp +/-2e9; cash widgets |
| 0x505350 | `effect_objects_lights_collect` | render | inferred | No args | ring 0xF9F684 (+0xB4) -> 0x508750 -> render_light_add_* |
| 0x5154D0 | `post_pass_desc_init` | render | confirmed | EAX = pass descriptor | 17 post-pass callers; descriptor defaults |
| 0x517300 | `post_shaders_resolve` | render | confirmed | No args | 25 shader / constant name strings; game_init_systems |
| 0x51CF40 | `post_copy_screen_to_target` | render | confirmed | ESI = target index, cdecl (bool clear), caller pops 4 | gfx_cmd_stretch_rect to 0xFA2350[ESI], optional clear |
| 0x5211F0 | `gfx_texture_cache_reset` | render | confirmed | No args, no return | fills 0xE84AA8[16] = -1 (texture cache) |
| 0x522450 | `peg_load_sync` | streamman | confirmed | ECX = file name, EDX = memory pool pair (CPU static_mempool, GPU pool at +0x54) | '.peg_pc' / '.g_peg_pc', file_get_size, hdd_read_file_sync |
| 0x523FA0 | `render_shader_time_seconds` | render | confirmed | No args | stamp 0xE9D7F8, timer_elapsed_ms, * 0.001 |
| 0x523FF0 | `game_time_of_day_fraction` | render | confirmed | No args | clock bytes 0x140E72C..E, 1/24, 1/1440, 1/86400 |
| 0x524060 | `render_global_constants_update` | render | confirmed | No args (push ecx frame) | writes 0x25273C4..D0, gfx_set_global_constants_c8 |
| 0x7F36D0 | `ui_button_image_tag` | ui | confirmed | cdecl (action), caller pops 4 | table 0xE8C980, '{No Image For B...' default |
| 0x7F43C0 | `lstr_prettify_placeholder` | ui | confirmed | EAX = wchar* (in place) | a-z -> A-Z after separators, '_' -> ' ' |
| 0x7F49E0 | `lstr_get` | ui | confirmed | ESI = char* id, EDI = wchar* fallback (may be 0) | 1335 callers; lstr_lookup + wstring_pool_intern |
| 0x7F4AD0 | `lstr_get_narrow_fallback` | ui | confirmed | ESI = char* id, EDI = char* fallback (may be 0) | 454 callers; narrow fallback variant |
| 0x7F4BC0 | `lstr_lookup` | ui | confirmed | ECX = char* string id (0 -> returns 0) | string_crc32_lower, language 0xE8C470 patches |
| 0x7F4D60 | `lstr_find_by_crc` | ui | confirmed | cdecl (u32 crc), caller pops 4 | 8 tables at 0x1FD5338, crc buckets |
| 0x94DFA0 | `projectiles_lights_collect` | render | inferred | No args | projectiles 0x215E07C, flag +0x184 & 8, render_light_add_point |
| 0x9556A0 | `searchlights_lights_collect` | render | inferred | No args | searchlights 0x215E0C0 (+0xB4) -> 0x954D50 -> spot light |
| 0x963750 | `humans_weapon_lights_collect` | render | inferred | No args | humans 0x215E908, weapon +0x10CC, render_light_add_point |
| 0xA78C40 | `rsched_should_forward_to_host` | streamman | inferred | EAX = scheduler index (handle >> 24), cdecl (kind), caller pops 4 | co-op flags 0x2528B48 / 0x2528B49, session 0x2528C14 |
| 0xA79120 | `rsched_add_ref_net` | streamman | inferred | EDI = resource handle (scheduler index in bits 24..31), ECX = group/owner, cdecl (level, bool local_only) | packet 0x73 via net_send_packet, else rsched_add_ref |
| 0xA79240 | `rsched_release_net` | streamman | inferred | EDX = resource handle, cdecl (level, owner, bool local_only) | packet, else rsched_release |
| 0xA796D0 | `rsched_has_request` | streamman | confirmed | EDX = resource handle | ring sched+0xC, next +0x2C, id +4 |
| 0xA7D890 | `hdd_read_file_sync` | streamman | inferred | ECX = file name, EAX = destination buffer, cdecl (size), caller pops 4 | async_io_submit / wait / status / release; priority 0x75 |
| 0xAE0FD0 | `vehicles_lights_collect` | render | inferred | No args | vehicle rings 0x22A84E0 / 0x22A84D4 (+0x84DC) -> headlights |
| 0xB87480 | `vint_bitmap_get_size` | vint | confirmed | EAX = bitmap handle, EDI = &width out, ESI = &height out | atlas 0x22DBFB0 + i*32, callback 0x252A1D4+0xC |
| 0xB878A0 | `vint_draw_bitmap_rect_2d` | vint | confirmed | cdecl (bitmap, f32 x, f32 y, f32 w, f32 h, int src_x, int src_y, int src_w, int src_h, bool flip_x, bool flip_y, state_pair*), caller pops 0x30 | gfx_set_bitmap, im2d strip, deferred type 0x1C |
| 0xB87C10 | `vint_draw_bitmap_rotated_2d` | vint | inferred | cdecl, like vint_draw_bitmap_rect_2d plus a rotation (sinf_wrapper / cosf_wrapper on the corner offsets) and vint_bitmap_get_size for the source size | sinf_wrapper / cosf_wrapper, im2d strip |
| 0xB8A2C0 | `vint_mem_realloc` | vint | confirmed | EDX = old pointer, ECX = new size | callback [0x252A1E4]+0 |
| 0xB8A2E0 | `vint_mem_free` | vint | confirmed | ECX = pointer | callback [0x252A1E4]+4 |
| 0xB91790 | `vint_lua_call_free` | vint | inferred | ESI = record | free ring 0x252A1C4 / active ring 0x252A1C8 |
| 0xB918A0 | `vint_lua_call_execute` | vint | inferred | stdcall (record), ret 4 | script_thread_create / resume on vint Lua state 0x252A1B8 |
| 0xB91B70 | `vint_lua_call_alloc` | vint | inferred | ECX = char* Lua function name (0 -> returns 0) | free ring 0x252A1C4, crc of name, handle at +0x210 |
| 0xB91D20 | `vint_lua_call_set_arg` | vint | inferred | EAX = vint_value* src, stdcall (record, int index, bool release_old), ret 0xC | 579 callers; 32 args at +0xC, count +0x20C |
| 0xB925F0 | `vint_encode_wide_string` | vint | confirmed | EAX = dest bytes, ECX = wchar* src, cdecl (dest size), caller pops 4 | inverse of luafn_vint_debug_decode_wide_string |
| 0xB93920 | `vint_value_equals` | vint | confirmed | EAX = vint_value* a, EDX = vint_value* b | type jump table, used by both setters |
| 0xB94980 | `vint_dataitem_free` | vint | confirmed | stdcall (item), ret 4 | doc 06 (vint_datagroup_deliver frees with it) |
| 0xB94B60 | `vint_dataitem_set_value` | vint | confirmed | EAX = vint_value* src, stdcall (item, int index), ret 8 | 510 callers; values +0x14, count +0x214, changed +0x228 |
| 0xBDD2A0 | `timer_elapsed_ms` | util | confirmed | EAX = &timestamp | clock 0xE843AC, wrap 0x6B49D200; 165 callers |
| 0xC06E90 | `wstring_pool_intern` | ui | confirmed | EAX = pool (0x2A15788: +0x38 data, +0x54 count), stdcall (wchar* s), ret 4 | pool 0x2A15788 (+0x38 data, +0x54 count), ret 4 |
| 0xC08B80 | `peg_register_loaded` | render | inferred | ESI = CPU peg header, cdecl (cpu size, gpu data), caller pops 8 | 0xC081C0 / 0xC085D0 / 0xC083D0; chunk and character peg loaders |
| 0xC08CC0 | `peg_find_by_name` | render | inferred | ECX = char* name | strips the extension, 0xC09940 lookup; 29 callers |
| 0xC11C00 | `input_pc_key_button_image_name` | ui | confirmed | cdecl (int *action, bool), caller pops 8 | 'GetPCKeyboardButtonIMGName: vb=%0x, vk=%0x', 'PCKEY_UNASSIGNED' |
| 0xCF4A40 | `gfx_shader_find_constant` | render | confirmed | EAX = shader record (ctx+0x484 table entry +8), cdecl (u32 name hash), caller pops 4 | two {hash, reg} tables; 'sampling_taps' etc. |
| 0xCF5E40 | `gfx_draw_text_wide_int` | render | confirmed | ESI = wchar* text, EDI, EBX = cdecl arg 4 | deferred type 0x15 / font_draw_string_2d_wide |
| 0xCFDD80 | `render_light_add_spot` | render | confirmed | EAX = &direction, EDX = &colour, cdecl (&pos, f32 a, f32 b, f32 c, f32 d, u32 flags, u32 e, bool keep) | type-2 record in 0x33D4DD0, count 0x252A364 |
| 0xCFDE50 | `render_light_add_point` | render | confirmed | EAX = flags, EDX = &colour (3 floats), cdecl (&pos, f32 a, f32 b, u32 c, bool keep), caller pops | type-0 record in 0x33D4DD0, count 0x252A364 (max 64) |
| 0xD09B20 | `gfx_hud_virtual_width` | render | confirmed | No args | 0x22FDC1C / 0x22FDC20, 1.45, 640 / 1280, 0x252A324 |
| 0xD15DC0 | `gfx_draw_text_int` | render | confirmed | ESI = char* text, EDI = font/style, cdecl (int x, int y), caller pops 8 | deferred type 0x13 / font_draw_string_2d |
| 0xD174E0 | `gfx_reset_sampler_states` | render | confirmed | No args, no return | ring 19 through cache 0x22F52F0, aniso [0xE84A30] |
| 0xD17E20 | `gfx_sampler_state_push` | render | confirmed | EAX = sampler, EDX = D3DSAMPLERSTATETYPE, cdecl (value), caller pops 4 | ctx+4 stack, count ctx+0x304, gfx_cmd_set_sampler_state |
| 0xD19A90 | `gfx_shader_find_sampler` | render | confirmed | EAX = shader id, EDI = name hash | shader +0x1C table; 'Luminance', 'Color1' |
| 0xD19C60 | `gfx_shader_find_by_hash` | render | confirmed | EDX = name hash (string_hash_lower) | hash table 0x2301CF8 (512 slots); post shader names |
| 0xD1A070 | `gfx_set_global_constants_c8` | render | confirmed | ECX = &float4 | ring 6 / 7 register 8; ctx+0x6A0 |
| 0xD1A400 | `gfx_set_screen_size_constants` | render | confirmed | cdecl (f32 w, f32 h), caller pops 8 | ring 6 / 7 register 9; ctx+0x670 |
| 0xD1A4D0 | `gfx_reset_instance_fade` | render | confirmed | cdecl (bool reset), caller pops 4 | ring 7 register 4; ctx+0x6E0 / 0x6E8 / 0x6EC |
| 0xD1A6D0 | `gfx_set_fade_mode_flag` | render | confirmed | CL = flag | ring 7 register 4 on change of ctx+0x6E8 |
| 0xD1A760 | `gfx_bind_shader_technique` | render | confirmed | EDX = shader id, cdecl (technique), caller pops 4 | ring 4 / 3 with caches 0x22F2AEC / 0x22F2AE8 |
| 0xD1CB00 | `gfx_cmd_readback_publish` | render | confirmed | cdecl (arg), caller pops 4 | ring 31 + publish (0x33D62F4 / 0x33D62FC) |
| 0xD1E140 | `gfx_msaa_targets_update` | render | confirmed | No args | device +0x70 / +0x74 (CreateRenderTarget / CreateDepthStencilSurface) |

## Port candidates

These are the 15 easiest to port. Each is a leaf, a list walker or a pure call sequence. All of them sit beside functions that are already in Rust: `gfx_cmd.rs` has the ring writer and the caches, and `frame.rs` has the same list-walker shapes.

Some notes apply to every recipe:
- "Ring" means the D3D command ring. Its write pointer is `0x33D62F8`, and each record is written as dwords at the pointer, which then advances.
- "ctx" is the render context `*(*(fs:[0x2C]) + 0x58)`.
- The x87 rows assume the FPU runs at the precision D3D left it in. Shadow-check them before going live.

### 1. `gfx_texture_cache_reset` (0x5211F0)
- **Threads:** the render thread (end of `render_frame`, `gfx_sampler_states_pop`, `gfx_reset_viewport`); the game thread at init (`game_init_systems`, `gfx_init_request`).
- **Convention:** no arguments, no return. It writes EAX (= -1) and no other register.
- **Body:** store 0xFFFFFFFF to the 16 dwords `0xE84AA8..0xE84AE4`.
- **Shared state:** the per-stage bitmap-handle cache that `gfx_set_texture_cached` reads. `render_deferred_execute` does the same reset inline, so only one thread touches the cache at a time.

### 2. `gfx_set_screen_size_constants` (0xD1A400)
- **Thread:** the render thread (render_frame render3, the post passes, `shadow_combiner_setup`).
- **Convention:** cdecl (f32 w, f32 h); the caller pops. It saves and restores EDI.
- **Body:**
  1. ctx+0x670 = w and ctx+0x674 = h.
  2. ctx+0x678 = 1/w and ctx+0x67C = 1/h. Both are divisions of 1.0 by the f32 value, rounded to f32. That equals `1.0f32 / w` at any FPU precision.
  3. Write the ring record {6, 9, 1, w, h, 1/w, 1/h} (0x1C bytes), then {7, 9, 1, the same four}. Each record copies the four floats back from ctx.
- **Shared state:** ring write pointer only, with no deferral and no publish.

### 3. `gfx_reset_instance_fade` (0xD1A4D0)
- **Thread:** the render thread.
- **Convention:** cdecl (u8 reset).
- **Body:**
  1. If reset: ctx+0x6E8 = 1, ctx+0x6E0 = 1.0, ctx+0x6EC = 1.0.
  2. Always write the ring record {7, 4, 1, x, ctx+0x6EC, 0.0, ctx+0x6E0}, where x = 0.0 if byte ctx+0x6E8 is set and 1.0 otherwise.
- **Note:** it shares the record shape with `gfx_set_instance_fade` (0xD1A620). Share one Rust helper.

### 4. `gfx_set_fade_mode_flag` (0xD1A6D0)
- **Thread:** the render thread.
- **Convention:** CL = flag; no stack arguments.
- **Body:**
  1. changed = (ctx+0x6E8 != CL); ctx+0x6E8 = CL.
  2. If changed, write the same PS c4 record as recipe 3, with x = CL ? 0.0 : 1.0.
- **Shared state:** none apart from the ring.

### 5. `gfx_set_global_constants_c8` (0xD1A070)
- **Threads:**
  - the game thread, from `render_global_constants_update` inside `frame_end_handoff`, while `0xE849DB` (force immediate) is set and the render thread is parked;
  - the render thread, from render_frame render2:a.
- **Convention:** ECX = &f32[4]; saves and restores EDI.
- **Body:**
  1. Copy the four floats to ctx+0x6A0..+0x6AC.
  2. Write the ring records {6, 8, 1, v0..v3} and {7, 8, 1, v0..v3}.
- **Note:** a single writer by protocol; the hand-off is what makes the game-thread write safe.

### 6. `gfx_sampler_state_push` (0xD17E20)
- **Thread:** the render thread.
- **Convention:** EAX = sampler, EDX = type, cdecl (value); saves and restores ESI and EDI.
- **Body:**
  1. If ctx+0x304 >= 0x40, return **without** setting the state.
  2. Otherwise write entry = ctx + 4 + count*12 as {sampler, type, `0x22F52F0`[sampler*16 + type]}, then count += 1.
  3. Call `gfx_cmd_set_sampler_state` (in Rust) with ESI = sampler, EDX = type, ECX = value. It skips the write when the cache already holds the value or the value is 0xBAADF00D.
- **Pair:** `gfx_sampler_states_pop` (0xD17D40).

### 7. `gfx_reset_sampler_states` (0xD174E0)
- **Thread:** the render thread (the render-pass setups 0x527020, 0x527A60, 0x5293D0, 0x528A10).
- **Convention:** no arguments; saves EBX, EBP, ESI and EDI.
- **Body:**
  1. For sampler s = 0..14, set each of these through the cache (compare `0x22F52F0`[s*16 + type], store it, write ring {0x13, s, type, value}):
     - types 1, 2 and 3 (ADDRESSU/V/W) = 1 (WRAP);
     - with aniso = `[0xE84A30]` > 0: type 7 (MIP) = 1 (POINT), 5 (MAG) = 2 (LINEAR), 6 (MIN) = 3 (ANISOTROPIC), then 0xA (MAXANISOTROPY) = aniso, except when aniso is 0xBAADF00D;
     - otherwise: 7 = 1, 5 = 2, 6 = 2.
  2. Then sampler 15: 7 = 0, 5 = 1, 6 = 1, and 1, 2, 3 = 3 (CLAMP).
- **Note:** the `cmp ecx, 0x14` guard in the loop is dead code (the loop stops at 15). Equivalent to 16 calls of the Rust sampler-state writer.

### 8. `gfx_shader_find_by_hash` (0xD19C60)
- **Thread:** the game thread at init (`post_shaders_resolve`, `bink_video_shader_resolve`, `rain_cone_setup`).
- **Convention:** EDX = hash; returns EAX.
- **Body:**
  1. i = hash & 0x1FF.
  2. While slot (`0x2301CF8` + i*8) +4 != -1: if slot+0 == hash, return slot+4; else i = (i + 1) & 0x1FF.
  3. Return `[0x2302CF8]`.
- **Shared state:** the table is written while shaders load (around 0xD1BFC0) and is read-only afterwards.

### 9. `gfx_shader_find_constant` (0xCF4A40)
- **Thread:** the game thread at init.
- **Convention:** EAX = shader record, cdecl (hash); the caller pops. Saves EBX, ESI and EDI.
- **Body:**
  1. For i < s16 [rec+0xC]: if `[rec+0x20][i*8]` == hash, return the s16 at `[rec+0x20][i*8 + 4]`, sign-extended.
  2. Then the same over [rec+0xE] / [rec+0x24].
  3. Return -1.
- **Note:** a pure leaf.

### 10. `gfx_hud_virtual_width` (0xD09B20)
- **Threads:** the game thread (HUD layout) and the render thread (the tagging HUD 0x625620).
- **Shared state:** reads the back-buffer size `0x22FDC1C` / `0x22FDC20` (written on device reset) and the scale `0x252A324`.
- **Convention:** no arguments; returns EAX (int).
- **Body:**
  1. ratio = fild w / fild h (extended). base = 1280 if ratio > 1.45 (double compare), else 640.
  2. p = (f32)(base * `[0x252A324]`). The value is stored to f32 and reloaded.
  3. p += 0.5 if p >= 0, else p -= 0.5 (double constant).
  4. Truncate (`_ftol2_sse`, a tail jump).
- **In Rust:** `((p as f64) + 0.5f64.copysign(p as f64)).trunc() as i32`, with p computed in f32.

### 11. `game_time_of_day_fraction` (0x523FF0)
- **Thread:** the game thread (`render_global_constants_update`).
- **Shared state:** reads the clock bytes `0x140E72C` / `D` / `E` (h, m, s), written by `game_clock_advance` on the same thread.
- **Convention:** no arguments; returns ST0.
- **Body:**
  1. If `[0x2528B48]` == 1 and `[0x2528B49]` == 0 and `[0x2527B77]` == 0: return the f32 `0xE8AF6C`[`[0x2A4D138]`].
  2. Otherwise compute h * c24 + m * c1440 + s * c86400. The c values are the double constants at `0xE5C780` / `0xE5C778` / `0xE5C770`; copy their bits, don't retype 1/24. The sum is evaluated left to right in x87 and stored to f32.
- **Rounding:** at 53/64-bit precision, computing in f64 and rounding once to f32 matches except at rare ties. At 24-bit precision control each operation rounds to a 24-bit mantissa. Shadow-check which one the game runs at.

### 12. `render_global_constants_update` (0x524060)
- **Thread:** the game thread, `game_render_func` step 3 (last fill).
- **Convention:** no arguments; a call sequence.
- **Body:**
  1. `[0x25273C4]` = `render_shader_time_seconds()` (ST0 → f32).
  2. `[0x25273C8]` = `game_time_of_day_fraction()`.
  3. `[0x25273CC]` = 0x4D9230(cdecl f32 `[0x25273C8]`), an x87 curve. Keep it original.
  4. `[0x25273D0]` = `[0xE9852C]` if `[0x2527520]` == 1, else 1.0.
  5. `gfx_set_global_constants_c8`(ECX = 0x25273C4).
- **Shared state:** render_frame reads 0x25273C4 on the render thread (render2:a), safe only inside the snapshot window (doc 11).

### 13. `vehicles_lights_collect` (0xAE0FD0)
- **Thread:** the game thread, `game_render_func` step 3.
- **Body:** for head in (`[0x22A84E0]`, `[0x22A84D4]`): v = head; while v: call 0xAD89A0(EAX = v); v = [v + 0x84DC]; stop when v == head. Each stop test re-reads its head global.
- **Note:** the same shape B walker as `vehicles_render_prepare`, which is already in Rust (frame.rs).
- **Shared state:** the light array (through the callee) and the vehicle lists (game thread only).

### 14. `effect_objects_lights_collect` (0x505350)
- **Thread:** the game thread, `game_render_func` step 3.
- **Convention:** saves ESI and EDI.
- **Body:**
  1. head = `[0xF9F684]`, read once into EDI. o = head.
  2. While o: 0x508750(cdecl o), add esp 4; o = [o + 0xB4]; stop when o == head.
- **Note:** the same list as `effect_objects_do_frame`, which is in Rust.

### 15. `searchlights_lights_collect` (0x9556A0)
- **Thread:** the game thread, `game_render_func` step 3.
- **Body:**
  1. head = `[0x215E0C0]`, read once. o = head.
  2. While o: 0x954D50(EAX = o); o = [o + 0xB4]; stop when o == head.
- **Note:** the same list and shape as `searchlights_update` (Rust).

**Next in line:**
- `projectiles_lights_collect` (0x94DFA0): one x87 offset (pos − vel × 0.25) and a constant light;
- `render_light_add_point` (0xCFDE50): stores only, but every light the renderer reads goes through it;
- `timer_elapsed_ms` (0xBDD2A0);
- `cash_from_dollars` (0x4B2680);
- `post_pass_desc_init` (0x5154D0).

## Quirks found

- **`lstr_get` (0x7F49E0) never uses its wide fallback.** It copies and prettifies the fallback into the placeholder buffer, then falls through and overwrites it: with the widened id, or with `(null)` when the id is 0. There is no jump after the prettify call at 0x7F4A43. The narrow variant 0x7F4AD0 has that jump. A missing string therefore shows its id, not the fallback text.
- **`gfx_sampler_state_push` drops the push when its 64-entry stack is full.** It then also skips setting the state. A later `gfx_sampler_states_pop(count)` would then restore entries that this caller never pushed.
- **`gfx_reset_instance_fade`, `gfx_set_screen_size_constants` and `gfx_set_global_constants_c8` write the ring with no `gfx_should_defer` check.** Unlike the deferred API functions, they are only correct on the render thread, or on the game thread while `0xE849DB` is set.
- **Doc 18 error.** It says the `game_render_func` step 3 fills add render instances. They add dynamic lights (see above).
