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

## Open

- About 70 more UI functions in 0x760000-0x7C0000 whose strings were too generic to name safely (`.tga`, `visible`, `Hud_has_focus`, …).
- The HUD document set and how `hud_render` chooses between `mp_hud`, `mp_hud_lobby` and activity HUDs (0x793D60).
