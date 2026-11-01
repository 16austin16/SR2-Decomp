# 28: Menus, HUD and stores (game side of the UI)

Names are in `docs/functions.csv` (subsystems `ui` and `script`, notes ending "(28)"). The vint runtime itself (documents, elements, tweens, rendering) is in 06_vint.md. This doc covers the game code that drives the UI screens.

## Lua binding registrars

The UI screens are Lua scripts, and each game feature hands them its bindings. `interface_register_lua_functions` (0x7F35A0, on the vint Lua state) calls one registrar per feature. Each registrar pushes C closures (`lua_pushcclosure` + `luaV_settable`).

They are named `<feature>_lua_register`:

| Registrar | Address | Bindings (examples) |
|---|---|---|
| `pause_menu_lua_register` | 0x77ADC0 | `pause_menu_get_state`, `pause_menu_select_target`, achievements |
| `pause_menu_options_lua_register` | 0x775A40 | control scheme, `pause_menu_update_option` / `_revert_options` / `_accept_options` |
| `playlist_lua_register` | 0x777110 | `playlist_play_track`, `radio_station_preview` |
| `dialog_box_lua_register` | 0x7537A0 | `dialog_box_set_result`, `dialog_box_create_internal` |
| `cellphone_lua_register` | 0x78A220 | `cellphone_dial_number`, `cellphone_activate_cheat` |
| `credits_lua_register` | 0x751E40 | `credits_are_finished`, `credits_force_close` |
| `garage_lua_register` | 0x556D40 | repair cost, preview, retrieve, remove |
| `crib_purchase_lua_register` | 0x5477A0 | crib purchase |
| `crib_customization_lua_register` | 0x7AABB0 | purchase, revert, camera, preview |
| `cdl_lua_register` | 0x7A8460 | car dealership: preview, purchase, store info |
| `weapon_store_lua_register` / `liquor_store_lua_register` / `music_store_lua_register` | 0x7B3C50 / 0x7AE530 / 0x7AF9E0 | store purchases |
| `vcust_lua_register` | 0x7BA220 | vehicle customization |
| `pcu_lua_register` / `pcr_lua_register` | 0x7CD9A0 / 0x7D6020 | player clothing (outfits) / player creation (morphs, presets) |
| `gang_customization_lua_register` | 0x7DE960 | gang customization |
| `demo_derby_lua_register` | 0x5EEA10 | demolition derby menus |
| `mp_popup_lua_register`, `mp_completion_lua_register`, `mp_misc_lua_register`, `mp_object_lua_register` | 0x884B00, 0x891930, 0x89DA60, 0xA4ED70 | multiplayer menus and objects |
| `debug_misc_lua_register`, `script_core_lua_register` | 0xA4EC00, 0xD74D50 | `dump_fps`, `log_append`; `debug_print`, `rand_int`, `get_frame_time` |

The bindings themselves were named earlier (`luafn_*`, 09_script.md).

## Screens

Named from the localisation keys and element names they use:
- **Pause menu**:
  - opening and closing: `pause_menu_open`, `pause_menu_populate`, `pause_menu_exit`, `pause_menu_save_load_exit`;
  - info tabs: activities, stats, messages, objectives, story (`pause_*_populate`);
  - the map: `pause_map_init`, `map_draw_other_waypoint*`, `map_store_info_text`;
  - co-op: `pause_menu_display_coop_tab`.
- **Save / load**:
  - `saveload_populate_list`, `saveload_continue_game`;
  - dialogs: `saveload_save_confirm_dialog`, `saveload_corrupt_file_dialog`, `saveload_blocked_dialog` (during missions or activities), `saveload_error_message`, `saveload_coop_load_error`, `start_game_out_of_save_slots_dialog`.
- **Options**: `options_display_mode_failed_dialog`, `options_display_change_messages`, and the rebinding prompts `controls_press_any_key_dialog` / `controls_press_any_button_dialog`. `quit_game_dialog` asks before quitting.
- **Cellphone and homies**: `cellphone_call_service` (taxi, ambulance, news), `cellphone_dial_result`, and the homie messages (`homie_rank_up_message`, `homie_down_message`, `homie_hospital_bill_message`, …).
- **HUD**:
  - helpers: `hud_element_fade` / `_fade_all`, `hud_hit_clear_all`, `hud_msg_hide_region`;
  - player state: `hud_busted_fade_in`, `hud_player_reset_complete`, `hud_smoked_busted_message`;
  - diversion messages: `hud_diversion_*_message`;
  - `hud_console_toggle` ("HUD is %s", "Minimap is %s").
- **Stores**:
  - table loaders: `car_dealerships_load_table`, `store_ammo_load_table`, `customization_logos_load_table`;
  - `hair_salon_confirm_purchase`, `food_store_play_voice`, `confessional_check_requirements`.

## Options menu and display settings (for porting and the 1440p entry)

The pause menu's option screens are Lua: `pause_menu.lua`, in `patch.vpp_pc`. That script builds the items and their slider labels. The exe only stores the chosen values.

### `pause_menu_update_option` (0x774E50)

Lua args are `(menu, option id, value)`. Every change also sets bit 0 of the dirty byte `0x1F7A960`.

**Menu 2, display.** Ids and labels are from `pause_menu.lua`. Values go to the working copy at `0x1F7A960`.

| Id | Label | Working copy |
|---|---|---|
| 0 | brightness | `0xFA2234` through 0x51A1B0 |
| 1 | MENU_RESOLUTION_TEXT | index → `0x1F7A9A0` = `0xE8DF14[i]` (width), `0x1F7A9A4` = `0xE8DF4C[i]` (height) |
| 2 | MENU_FULLSCREEN_TEXT | byte `0x1F7A9AC` |
| 3 | MENU_GRAPHICS_QUALITY_TEXT | `0x1F7A9A8` (0-2 presets, 3 custom). A change loads the preset with `options_apply_quality_preset` |
| 4 | MENU_CONTROLS_SUBTITLES | `0xE8DD1E` |
| 5 | CONTROLS_MINIMAP_VIEW | `0x2528615` |
| 6 | Ambient_Occlussion | `0x1F7A9B0` bits 12-15 |
| 7 | Fullscreen_Antialiasing | byte `0x1F7A9AE` |
| 8 | HDR | byte `0x1F7A9AF` |
| 9 | Shadow_Maps | `0x1F7A9B0` bits 0-3 |
| 10 | View_Distance | `0x1F7A9B4` |
| 11 | MENU_MOTION_BLUR | `0x1F7A9AD` bit 1 |
| 12 | MENU_DYNAMIC_LIGHTS | `0x1F7A9B0` bits 4-7 |
| 13 | MENU_BLUR | `0x1F7A9AD` bit 2 |
| 14 | MENU_DEPTH_OF_FIELD | `0x1F7A9AD` bit 3 |
| 15 | MENU_VSYNC | `0x1F7A9AD` bit 4 (doesn't switch to custom) |
| 16 | ANISOTROPY_FILTERING | `0x1F7A9B0` bits 8-11 |

Ids 6-14 and 16 switch the quality to 3 (custom) first.

**Menu 3, audio:** 0 SFX (`0xEE34E4` / `0xEE34D8`), 1 music (`0xEE34E0`), 2 voice (`0xEE34DC`), 3 radio (`0x2526B70`). Each one calls 0x47F2E0.

**Menu 5:** key binding `0x234E768[id]`, then `input_save_bindings`.

**Menu 1, controls:**
- 0 invert Y, 1 invert rotation, 2 stick, 6 crouch toggle: bits of `0xF8549B` (and `0xE97D5A` for id 5, force feedback);
- 3 / 4 stick sensitivity V / H: `0x25F5C94` / `0x25F5C90`;
- 7 / 8 mouse sensitivity V / H: `0x25F5C9C` / `0x25F5C98`.

### Where the values live

There are three copies of the display settings:
1. **Working copy** at `0x1F7A960..0x1F7A9B7`, edited by the menu.
2. **Live graphics config** at `0x22FD848`:
   - +0 "mode changed" flag;
   - +4 width, +8 height, +0xC fullscreen;
   - +0x10 quality;
   - +0x14 flag byte (bit 1 motion blur, 2 blur, 3 depth of field, 4 vsync);
   - +0x15 antialiasing, +0x16 HDR;
   - +0x18 bitfield (shadows 0-3, dynamic lights 4-7, anisotropy 8-11, ambient occlusion 12-15);
   - +0x1C view distance.

   `game_init_allocators_and_packfiles` reads it at boot.
3. **Settings-file image** at `*(0x140FFAC)`, written by `options_store_display` (0x773C70). Its offset is the working-copy offset from `0x1F7A960`, and the `settings.dat` offset is 4 less:

| Image | `settings.dat` | Field |
|---|---|---|
| +0x2C | +0x28 | subtitles |
| +0x40 / +0x44 | +0x3C / +0x40 | width / height |
| +0x48 | +0x44 | quality |
| +0x4C | +0x48 | fullscreen |
| +0x4D | +0x49 | flags (motion blur, blur, depth of field, vsync) |
| +0x4E / +0x4F | +0x4A / +0x4B | antialiasing / HDR |
| +0x50 | +0x4C | bitfield (as the live config) |
| +0x54 | +0x50 | view distance |

The rest of the working copy:
- `0x1F7A964` / `0x1F7A968` and `0x1F7A970..0x1F7A980`: controls (`options_capture_controls`, 0x773BE0);
- `0x1F7A984..0x1F7A98B`: brightness, subtitles, minimap (`options_commit_display` captures them);
- `0x1F7A98C..0x1F7A998`: audio (`options_capture_audio`, 0x7743F0);
- `0x1F7A99C`: `0xE986F4`.

**Functions:**
- **`options_commit_display`** (0x773EB0; cdecl `(bool accept)`):
  - **Accept:** copies the working copy into the live config. If the resolution, fullscreen or vsync changed, it sets `0x22FD848` = 1 to ask for a device reset. Then it calls 0x7735C0 and `options_store_display`.
  - **Revert:** copies the live config back into the working copy.
- **`options_apply_quality_preset`** (0x773E00; `EAX` = preset): fills the custom fields from the 12-byte preset table at `0xE8DE9C`.
- **`options_init_from_settings`** (0x7759F0): captures controls, reverts the display, captures audio.

### The resolution list

The exe has two 14-entry tables, `0xE8DF14` (widths) and `0xE8DF4C` (heights):

640×480, 800×600, 1024×768, 1280×720, 1280×800, 1280×960, 1280×1024, 1440×900, 1600×1200, 1680×1050, 1920×1080, 1920×1200, 1920×1440, 2048×1536.

They are read in two places:
- `pause_menu_update_option` id 1 (0x77519E / 0x7751A5) indexes them with the Lua value. **There is no bounds check.**
- The options init (0x775F40..0x775F57) searches them for the current width and height to find the slider position. Its bound is the byte **14 at 0x775F56** (`cmp eax, 0xE`).

The labels are only in Lua: `resolution_slider_values` in `pause_menu.lua`, with `[0]`..`[13]` and `num_values = 14`.

**Adding 2560×1440 (or any list):**
1. In the hook, make new width / height arrays with the extra entries. Point the four disp32 operands at them:
   - `0x7751A1` and `0x775F43` → widths;
   - `0x7751A8` and `0x775F4C` → heights.
2. Set the byte at `0x775F56` to the new count.
3. Ship a `pause_menu.lua` in `decomp.vpp_pc` (it is mounted after `patch.vpp_pc`) with the extra labels, for example `[14] = { label = "2560x1440" }`, and `num_values` raised to match.

Replacing entry 13 instead would need only two dword writes, but the label would still say 2048x1536 unless the Lua is overridden too.

## Open

- About 70 more UI functions in 0x760000-0x7C0000 whose strings were too generic to name safely (`.tga`, `visible`, `Hud_has_focus`, …).
- The HUD document set and how `hud_render` chooses between `mp_hud`, `mp_hud_lobby` and activity HUDs (0x793D60).
