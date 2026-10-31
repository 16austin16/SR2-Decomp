# 30: Cribs, garages, stores, customization and front-end menus

Names are in `docs/functions.csv` (subsystems `progression` and `ui`, notes ending "(30)"). The Lua binding registrars for these screens are in 28_menus_hud.md, the tuning sections (`Cribs`, `Garage`) in 26_data_tables.md, and saves in 13_savegame.md.

## Cribs

| Function | Address | What |
|---|---|---|
| `crib_lookup` | 0x544F10 | finds a crib by name ("The crib '%s' does not exist") |
| `saints_hq_set_upgrade_level` | 0x545870 | swaps the Saints HQ upgrade chunks (`redlight_$SHQ_UG_*`, `level%d`) |
| `crib_activity_menu` | 0x547870 | the crib's options: change radio station, view clipbook, watch television |
| `crib_stash_found_cash_message`, `crib_stash_tutorial_nag` | 0x548C60, 0x548260 | the cash stash (`Money_Storage_Element`, `Max_Stash` in `gameplay_constants`) |
| `crib_purchase_lua_register`, `crib_customization_lua_register` | 28 | buying and decorating cribs |

## Garages

`garages_load_table` (0x550D60) reads the `garage_*.xtbl` tables (`Garages`, `Name`, `Type`). The menu and its messages:
- `garage_menu_populate` (0x5524D0, "NO CARS IN THE GARAGE");
- `garage_cant_store_message` (too small, helicopters only at a helipad);
- `garage_full_message`, `garage_cant_open_message`, `garage_vehicle_added_message`;
- `garage_exit_tutorial`.

Repair, preview, retrieve and remove are Lua bindings (`garage_lua_register`, 28).

## Stores and customization

- **Clothing and wardrobe**: `wardrobe_populate` (0x7C8450; underwear, suits, outfits). The `pcu_*` bindings handle outfits.
- **Player creation and plastic surgeon**: `pcr_*` bindings, `pcr_morph_handshake` (0x7D6760, co-op morph sync), `plastic_surgeon_confirm` (0x7D96A0).
- **Gang customization**: `gang_customization_setup_cameras` (0x7DE2F0; the `underground_$gang_cust_*` cameras).
- **Other stores**: car dealership (`cdl_*`), weapon, liquor, music, food and hair salon (28_menus_hud.md).

## Front-end menus

- **Multiplayer**:
  - main and party: `mp_menu_main_populate`, `mp_menu_create_party`, `mp_menu_customization`;
  - joining: `mp_menu_join_game`, `mp_menu_coop_game`, `mp_games_search_status`, `mp_network_check_dialog`;
  - players: `mp_players_list_populate`, `mp_players_kick_player`;
  - `main_menu_downloads`.
- **Options**: `options_audio_menu`, `options_display_menu`, `options_controls_setup_menu`, `options_keep_discard_dialog`, `options_vibration_notice`, `options_control_disabled_text`, and the slider tuning debug `menu_num_slider_accel_debug`.
- **HUD race start**: `hud_race_start_countdown`, `hud_race_start_pause_all` / `_unpause_all`.
- **Co-op**: `coop_mission_start_prompt` (0x543C70, the partner's start / replay prompt).

The text-rule names (`*_text`, as in 29_activities.md) for this area are the saveload / crib / store keys: 9 functions.

## Open

- The clothing store (0x7C95F0, 13 KB) and player creation (0x7D0E60, 11 KB) bodies.
- The crib and garage save sections (13_savegame.md lists them by number only).
