# 35: Multiplayer and co-op front end and modes

Names are in `docs/functions.csv` (subsystem `multiplayer`, notes ending "(35)"). The packet layer is in 25_network.md and 27_packet_senders.md, replicated objects in 08_multi_objects.md, GameSpy in 24_gamespy.md, and the menus' Lua registrars (`mp_popup`, `mp_completion`, …) in 28_menus_hud.md.

## Data

- `multiplayer_load_tables` (0x7F6EB0) loads `multiplayer_levels.xtbl` and `multiplayer_mode_matchmaking.xtbl`.
- `multiplayer_modes_parse` (0x880D20) reads each mode's `Name`, `MatchmakeFlags` and `MaxRankedParty`.
- `allocator_categories_init` (0x7F6F50) fills the allocator category table that the replicated-state allocators register into (34_world_objects.md).

## Joining and the lobby

- **Sign-in**: `gamespy_enter_email_prompt` / `_name_prompt` (keys `MULTI_GAMESPY_*`, named by the text rule as `multi_gamespy_*_text`), `session_password_prompt`, `multi_login_success_message`.
- **Co-op**: `coop_join_request_dialog` (0x831AC0, accept / deny). Co-op mission start prompts are in 30_progression.md.
- **System link**: `syslink_server_list_populate` (0x835F80) lists LAN servers with their ping.
- **Lobby**:
  - `mp_lobby_init_data` (0x83FD60) sets up player, game and variant data;
  - `mp_lobby_gametype_options` and `mp_lobby_time_settings` are the settings menus;
  - `mp_lobby_start_checks` (0x83F5D0) refuses to start while someone is joining, with too many players or with only one;
  - `mp_matchmaking_teams_hud` shows the teams.
- **Loading**: `multi_game_load_level` (0x834880). `mp_time_of_day_name` maps the lobby's time-of-day choice.
- **Debug**: `net_print_connection_info` (0x812610), `cmd_mp_set_team`.

## In game

- **Scoring and messages**: `mp_score_event_text` (kill VIP / pimp / police / ped), `mp_player_left_message`, `mp_host_left_dialog`, `multi_sudden_death_text`.
- **End of a match**: `mp_game_over_announce` (team win / loss voice and sound), then `mp_completion_open` (0x8911F0, the completion screen).
- **Modes**:
  - Gangsta Brawl / team modes (`mp_gb_load_text`);
  - Strong Arm, whose activities rotate (`mp_activity_starting_banner`; turf tags with `mp_turf_tags_reset`; derby, fraud, hitman, mayhem and snatch with their `mp_act_*_text` helpers);
  - racing (`mp_racing_text`).
- The game-specific RTTI classes for Strong Arm are the `mp_bh_activity_*` info / host / participant instances (03_rtti_classes.md).

## Open

- The `mp_bh_activity_*` slot names (the activities' virtual methods).
- How matchmaking talks to GameSpy (the `gamespy_matchmaking_module` glue at 0x8CF000-0x8D3000).
