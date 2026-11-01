# 48: Network names, packet table, serialiser API and port recipes

Names are in `docs/candidates_net.csv` (subsystem `net`, notes ending "(48)"). Nothing existing was edited, so these addresses are not yet in `docs/functions.csv`. The packet layer is described in 25_network.md, the packet senders in 27_packet_senders.md, the replicated objects in 08_multi_objects.md and the join flow in 40_frame_loop_porting_map.md.

What this round adds:
- **The bit-stream serialiser API** (writers, readers, vec2 / vec3 / bytes helpers and the noncompressor / compressor constructors). It was completely unnamed although 136 callers use the reader and the compressors use the writers.
- **Handle, net id and ownership helpers** that nearly every packet sender and receiver calls (`net_handle_to_net_id` has 326 callers, `net_handle_owned_locally` 260).
- **The send helper `net_send_packet_to_remotes`** (368 callers), the packet routing / state checks and the join-reply senders.
- **Connection transport internals**: ack window, loss ratio, queues, timeouts, rate meters, the network clock.
- **Session helpers**: the connection list walkers, state predicates, reservations, GUID compare, `net_set_active_session`.
- **The replicated-object core** that doc 08 describes by address but that had no names.
- **The full packet table** with routing class, state mask, channel and payload (below).

Method: addresses came from direct `call` targets plus int3-padded starts; every function in the names table was disassembled. "C" = confirmed from the code (and callers), "I" = inferred (the role follows from callers or context, a field meaning is a guess). 132 functions: 88 confirmed, 44 inferred.

## Packet table

Parsed straight from the two 24-byte-record tables (0xE8BF08 for types 0x00-0x31, 0xE8B6A8 for 0x32-0x82). All 131 receive handlers were already named `net_recv_*`; the columns below add the data the earlier docs left out.

| Column | Meaning |
|---|---|
| route | record +0xC. Send check `net_packet_route_allows_send` (0x814B00), receive check `net_packet_route_allows_recv` (0x814B40). With `P = [[conn+0x1AC]+0x404]` (the session's designated peer): route 0 sends only to P and accepts only from a connection that is not P; route 1 never sends to P and accepts only from P; route 2 is unrestricted. Which role P plays (host or the local side) is not settled, see the open points. |
| mask | record +0x10. `net_packet_state_allows` (0x814B80) tests bit `[conn+0x3C]` of the low 16 bits; `0xFFFF` skips the test; `0x1FF00` = connection states 8..15. |
| chan | record +0x14. Non-zero = ordered channel; the sender reserves a 4th header byte (sequence), so the header is 4 bytes instead of 3. |

| Type | Name | Handler | Route | Mask | Chan | Payload after the header (from docs/net_payloads.csv) |
|---|---|---|---|---|---|---|
| 0x00 | ping | 0x812C90 net_recv_ping | 2 | 0x100FFFF | 0 | (no sender found with the header pattern) |
| 0x01 | pong | 0x812D30 net_recv_pong | 2 | 0x100FFFF | 0 | (no sender found with the header pattern) |
| 0x02 | join_req | 0x815BC0 net_recv_join_req | 1 | 0xFFFF | 0 | blob, u32, u32, u32/u8, blob, u8, u32, u32, u32 |
| 0x03 | join_acc | 0x8162E0 net_recv_join_acc | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x04 | join_deny | 0x8163F0 net_recv_join_deny | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x05 | join_new_session | 0x8164F0 net_recv_join_new_session | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x06 | join_success | 0x816680 net_recv_join_success | 1 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x07 | join_failed | 0x8167D0 net_recv_join_failed | 1 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x08 | join_complete | 0x816930 net_recv_join_complete | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x09 | remove_reservation | 0x816AC0 net_recv_remove_reservation | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x0A | new_conn | 0x816D00 net_recv_new_conn | 0 | 0x1FFFF | 3 | (no sender found with the header pattern) |
| 0x0B | upd_conn | 0x816CC0 net_recv_upd_conn | 2 | 0x1FFFF | 3 | u8 |
| 0x0C | conn_left | 0x8173C0 net_recv_conn_left | 2 | 0x1FFFF | 3 | u8 |
| 0x0D | badcon | 0x7FBCF0 net_recv_badcon | 1 | 0x1FFFF | 0 | u16 |
| 0x0E | rop_create | 0x805510 net_recv_rop_create | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x0F | rop_destroy | 0x805670 net_recv_rop_destroy | 2 | 0x1FF00 | 0 | u32, u8, u16 |
| 0x10 | rop_update | 0x8056E0 net_recv_rop_update | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x11 | dc_ack_list | 0x800430 net_recv_dc_ack_list | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x12 | voice | 0x818670 net_recv_voice | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x13 | qos | 0x80B5B0 net_recv_qos | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x14 | clock | 0x7FCBF0 net_recv_clock | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x15 | bw_info | 0x80B5E0 net_recv_bw_info | 1 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x16 | snd_lmt | 0x80B610 net_recv_snd_lmt | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x17 | down_lmt | 0x80B620 net_recv_down_lmt | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x18 | setup_request | 0x831500 net_recv_setup_request | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x19 | setup_response | 0x8315E0 net_recv_setup_response | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x1A | migrate | 0x8108F0 net_recv_migrate | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x1B | migrated | 0x810A00 net_recv_migrated | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x1C | post_migration | 0x8037E0 net_recv_post_migration | 2 | 0x1FF00 | 0 | u16, u16 ; u8, u16, u16, u8 |
| 0x1D | modify session | 0x810AC0 net_recv_modify_session | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x1E | modified session | 0x810BD0 net_recv_modified_session | 1 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x1F | ext_data | 0x810C20 net_recv_ext_data | 2 | 0x100FFFF | 0 | (no sender found with the header pattern) |
| 0x20 | upd state | 0x7FAFA0 net_recv_upd_state | 2 | 0x1FFFF | 3 | u8, u32 |
| 0x21 | ack reply | 0x7FACD0 net_recv_ack_reply | 2 | 0xFFFF | 0 | (no sender found with the header pattern) |
| 0x22 | heartbeat | 0x7FAF20 net_recv_heartbeat | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x23 | no connect | 0x7FAF30 net_recv_no_connect | 1 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x24 | next srv | 0x810A20 net_recv_next_srv | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x25 | bull_taco | 0x7FAF90 net_recv_bull_taco | 2 | 0xFFFF | 0 | (no sender found with the header pattern) |
| 0x26 | acks | 0x7FB150 net_recv_acks | 2 | 0xFFFF | 0 | (no sender found with the header pattern) |
| 0x27 | rop_take_own | 0x805A00 net_recv_rop_take_own | 2 | 0x1FF00 | 1 | (no sender found with the header pattern) |
| 0x28 | rop_req_own | 0x805AA0 net_recv_rop_req_own | 2 | 0x1FF00 | 1 | (no sender found with the header pattern) |
| 0x29 | rop_own_deny | 0x805A50 net_recv_rop_own_deny | 2 | 0x1FF00 | 1 | (no sender found with the header pattern) |
| 0x2A | rop_new_owner | 0x805C00 net_recv_rop_new_owner | 2 | 0x1FF00 | 1 | u16 |
| 0x2B | global_sync | 0x801A90 net_recv_global_sync | 0 | 0x1FF00 | 2 | (no sender found with the header pattern) |
| 0x2C | create_ack | 0x805C70 net_recv_create_ack | 2 | 0x1FF00 | 0 | u16 |
| 0x2D | pcjoininfo | 0x80A300 net_recv_pcjoininfo | 2 | 0xFFFF | 0 | (no sender found with the header pattern) |
| 0x2E | gamespyid | 0x812020 net_recv_gamespyid | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x2F | nat | 0x8CD660 net_recv_nat_2 | 2 | 0x1FFFF | 0 | u8, u8 ; u32 |
| 0x30 | nat | 0x80A360 net_recv_nat | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x31 | logging | 0xB827E0 net_recv_logging | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x32 | lbby_upd | 0x841F20 net_recv_lbby_upd | 0 | 0x1FF00 | 0 | u8, u8, u8, u8, u16, u16 |
| 0x33 | player_respawn | 0x82B6C0 net_recv_player_respawn | 2 | 0x1FF00 | 0 | u16, u32, u32, u32, u8/u32, u16/u8, blob |
| 0x34 | mp_respawn | 0x889230 net_recv_mp_respawn | 2 | 0x1FF00 | 0 | u32, u32, u32, u8 ; u32 |
| 0x35 | req_name | 0x82B820 net_recv_req_name | 1 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x36 | set_team | 0x82B9A0 net_recv_set_team | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x37 | req_team | 0x82B940 net_recv_req_team | 1 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x38 | play_audio | 0x818A80 net_recv_play_audio | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x39 | player_score_upd | 0x88CEB0 net_recv_player_score_upd | 0 | 0x1FF00 | 27 | (no sender found with the header pattern) |
| 0x3A | gb_change | 0x87E210 net_recv_gb_change | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x3B | braveheart_packet | 0x8A2740 net_recv_braveheart_packet | 2 | 0x1FF00 | 0 | u8=4, u8, handle, u8 ; u8=0/u8, u8, u32, u32/u8, u32, u8, u8 |
| 0x3C | expl! | 0x83CC70 net_recv_expl | 2 | 0x1FF00 | 0 | u8, blob, handle, handle, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32/u8=1, blob, u8=0, u8, ... |
| 0x3D | veh_dmg | 0x87A940 net_recv_veh_dmg | 0 | 0x1FF00 | 0 | u8, u8, u8, u8, u8, u8, u8 |
| 0x3E | wpn_ammo_change | 0x845B40 net_recv_wpn_ammo_change | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x3F | mover_fade | 0x84E560 net_recv_mover_fade | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x40 | mover_dmg | 0x84E8C0 net_recv_mover_dmg | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x41 | mover_active | 0x84EA60 net_recv_mover_active | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x42 | var upd | 0x83BCF0 net_recv_var_upd | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x43 | rem console | 0x819570 net_recv_rem_console | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x44 | req_score_change | 0x88D230 net_recv_req_score_change | 1 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x45 | player_morph | 0x82BAA0 net_recv_player_morph | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x46 | player_components | 0x82BD00 net_recv_player_components | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x47 | veh_cust | 0x87AF30 net_recv_veh_cust | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x48 | cli_trigger_activate | 0x881BD0 net_recv_cli_trigger_activate | 1 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x49 | stats_upd | 0x83ABA0 net_recv_stats_upd | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4A | stats_final | 0x83AC20 net_recv_stats_final | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4B | game_cond | 0x89B690 net_recv_game_cond | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4C | game_clock | 0x68A450 net_recv_game_clock | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4D | weather | 0x4DE670 net_recv_weather | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4E | avail_lvls | 0x842260 net_recv_avail_lvls | 1 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x4F | trees_destroyed | 0x83B6F0 net_recv_trees_destroyed | 2 | 0x1FF00 | 0 | u8 |
| 0x50 | join_coop | 0x8324F0 net_recv_join_coop | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x51 | matchmaking | 0x827220 net_recv_matchmaking | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x52 | match_info | 0x835670 net_recv_match_info | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x53 | mm_finalize | 0x835580 net_recv_mm_finalize | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x54 | matchp_info | 0x835760 net_recv_matchp_info | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x55 | veto | 0x8357F0 net_recv_veto | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x56 | sessionstart | 0x835930 net_recv_sessionstart | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x57 | sessionend | 0x8359D0 net_recv_sessionend | 0 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x58 | lobbyreturn | 0x82F810 net_recv_lobbyreturn | 2 | 0x1FFFF | 0 | blob |
| 0x59 | ctrl_mode_change | 0x4F5360 net_recv_ctrl_mode_change | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x5A | player_control | 0x9DFDB0 net_recv_player_control | 0 | 0x1FF00 | 22 | handle ; handle, u8 |
| 0x5B | mp_info | 0x7F8E70 net_recv_mp_info | 2 | 0x1FFFF | 0 | (no sender found with the header pattern) |
| 0x5C | gameplay_changed | 0x68FD40 net_recv_gameplay_changed | 2 | 0x1FF00 | 20 | u8/u32, handle, handle, u32, handle, u32, u32 ; u32, u8 |
| 0x5D | gameplay_denied | 0x68FEF0 net_recv_gameplay_denied | 0 | 0x1FF00 | 20 | (no sender found with the header pattern) |
| 0x5E | mssn_req | 0x6A8AD0 net_recv_mssn_req | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x5F | race_req | 0x682770 net_recv_race_req | 2 | 0x1FF00 | 0 | u8, u32, u32 |
| 0x60 | osd_sync | 0x944970 net_recv_osd_sync | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x61 | actn_reqs | 0x683D80 net_recv_actn_reqs | 2 | 0x1FF00 | 0 | u16 |
| 0x62 | door_event | 0x91AB10 net_recv_door_event | 2 | 0x1FF00 | 0 | u16 |
| 0x63 | actn_node | 0x911F30 net_recv_actn_node | 2 | 0x1FF00 | 19 | u16/u32, u16 |
| 0x64 | scrpt_start_game | 0x832840 net_recv_scrpt_start_game | 0 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x65 | gsi | 0x78F320 net_recv_gsi | 0 | 0x1FF00 | 20 | (no sender found with the header pattern) |
| 0x66 | notor_pkt | 0x55EDF0 net_recv_notor_pkt | 2 | 0x1FF00 | 0 | u8, u32, u8, u8 ; u8, u32 |
| 0x67 | hud_pkt | 0x8CA2A0 net_recv_hud_pkt | 0 | 0x1FF00 | 0 | u8 sub-command (0..0x18), then per sub-command: docs/net_subcommands.csv |
| 0x68 | human_pkt | 0x8CA2C0 net_recv_human_pkt | 2 | 0x1FF00 | 0 | u8 sub-command (0..0x50), then per sub-command: docs/net_subcommands.csv |
| 0x69 | human_pkt_in_ord | 0x8CA2E0 net_recv_human_pkt_in_ord | 2 | 0x1FF00 | 20 | u8 sub-command (0..0x50), then per sub-command: docs/net_subcommands.csv |
| 0x6A | veh_pkt | 0x8CA300 net_recv_veh_pkt | 2 | 0x1FF00 | 0 | u8 sub-command (0..0x1f), then per sub-command: docs/net_subcommands.csv |
| 0x6B | scrpt_pkt | 0x8CA320 net_recv_scrpt_pkt | 2 | 0x1FF00 | 17 | u8 sub-command (0..0x2a), then per sub-command: docs/net_subcommands.csv |
| 0x6C | act_packet | 0x8CA340 net_recv_act_packet | 2 | 0x1FF00 | 20 | u8 sub-command (0..0x5b), then per sub-command: docs/net_subcommands.csv |
| 0x6D | misc_packet | 0x8CA360 net_recv_misc_packet | 2 | 0x1FF00 | 0 | u8 sub-command (0..0x21), then per sub-command: docs/net_subcommands.csv |
| 0x6E | memb_pkt | 0x84AE00 net_recv_memb_pkt | 2 | 0x1FF00 | 0 | u32 hash("human_ai_data"), u32 hash("leash_params"), handle, value ; u32 hash("human_ai_data"), u32 hash("w... |
| 0x6F | cutscn_pkt | 0x6BECA0 net_recv_cutscn_pkt | 2 | 0x1FF00 | 18 | (no sender found with the header pattern) |
| 0x70 | chunk_swap_packet | 0xA7C470 net_recv_chunk_swap_packet | 2 | 0x1FF00 | 18 | (no sender found with the header pattern) |
| 0x71 | notor_active | 0x924D30 net_recv_notor_active | 0 | 0x1FF00 | 21 | (no sender found with the header pattern) |
| 0x72 | strm_set_change | 0xA79D60 net_recv_strm_set_change | 0 | 0x1FF00 | 16 | u32, u8, u32, u32, u8, u32, u32 |
| 0x73 | strm_req | 0xA79210 net_recv_strm_req | 1 | 0x1FF00 | 16 | u32, u32 ; u32 |
| 0x74 | strm_rem_req | 0xA79360 net_recv_strm_rem_req | 1 | 0x1FF00 | 16 | (no sender found with the header pattern) |
| 0x75 | osd_char_strm_req | 0x944490 net_recv_osd_char_strm_req | 1 | 0x1FF00 | 16 | (no sender found with the header pattern) |
| 0x76 | osd_char_strm_rem_req | 0x9444D0 net_recv_osd_char_strm_rem_req | 1 | 0x1FF00 | 16 | (no sender found with the header pattern) |
| 0x77 | multi common misc | 0x882240 net_recv_multi_common_misc | 2 | 0x1FF00 | 0 | u16 |
| 0x78 | melee | 0x9810C0 net_recv_melee | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |
| 0x79 | persona | 0x6ACD60 net_recv_persona | 2 | 0x1FF00 | 0 | handle, handle, handle, handle |
| 0x7A | pm_objective | 0x781790 net_recv_pm_objective | 0 | 0x1FF00 | 20 | (no sender found with the header pattern) |
| 0x7B | screen_fade | 0x4D1610 net_recv_screen_fade | 0 | 0x1FF00 | 23 | (no sender found with the header pattern) |
| 0x7C | block_load | 0x69AA80 net_recv_block_load | 0 | 0x1FF00 | 23 | (no sender found with the header pattern) |
| 0x7D | tutorials | 0x6B6E00 net_recv_tutorials | 2 | 0x1FF00 | 24 | (no sender found with the header pattern) |
| 0x7E | streaming_sync | 0xA83F30 net_recv_streaming_sync | 2 | 0x1FFFF | 25 | (no sender found with the header pattern) |
| 0x7F | arb_mover | 0x84EFC0 net_recv_arb_mover | 2 | 0x1FF00 | 0 | u16 ; u32, u8 |
| 0x80 | pausing | 0x8CB110 net_recv_pausing | 2 | 0x1FF00 | 26 | (no sender found with the header pattern) |
| 0x81 | None | 0xB0A460 net_recv_vi | 2 | 0x1FF00 | 0 | handle, handle, u8 |
| 0x82 | chat | 0x819EA0 net_recv_chat | 2 | 0x1FF00 | 0 | (no sender found with the header pattern) |


Notes on the table:
- **Senders now named** for types that showed "no sender found" in 27: 0x04 `net_send_join_deny` (0x816F60), 0x05 `net_send_join_new_session` (0x816460), 0x06 `net_send_join_success` (0x8165F0), 0x07 `net_send_join_failed` (0x816740), 0x08 `net_send_join_complete` (0x8168A0), 0x20 `upd_state` has three shapes: sub 0 `{u8 state}` from `net_conn_set_state_broadcast`, sub 1 `{u32}` from `net_conn_set_phase_broadcast`, sub 3 `{}` from `net_send_upd_state_sub3`; 0x28 `rop_req_own` `{u16 net id, u8}` from `net_request_ownership`; 0x5B `mp_info` `{u8, u8}` from `net_send_mp_info`.
- Type 0x81 has no debug name in its record (null pointer); its handler is `net_recv_vi` (0xB0A460).
- Two records carry the debug name `nat`: 0x2F (`net_recv_nat_2` 0x8CD660) and 0x30 (`net_recv_nat` 0x80A360).
- The packet record array is also indexed by the senders to decide the header size: the byte tested by a sender is `record + 0x14` of its own packet type. Examples: packet 0x20 tests 0xE8C21C, packet 0x28 tests 0xE8C2DC, packet 0x5B tests 0xE8BA94. The doc 25 statement that it is one flag at 0xE8BC14 is wrong (see Doc errors).

### Sub-command packets

`hud_pkt` 0x67 (25 sub-commands), `human_pkt` 0x68 / 0x69 (81), `veh_pkt` 0x6A (32), `scrpt_pkt` 0x6B (43), `act_packet` 0x6C (92), `misc_packet` 0x6D (34). Their dispatch tables and the 307 handlers are in 25_network.md and `docs/net_subcommands.csv`; nothing was renamed here. The `human_pkt` 0x3C / 0x3D / 0x3E / 0x3F / 0x40 / 0x41 receivers (0x558EE0, 0x559380, 0x559C90, 0x559780, 0x55A250, 0x55A450) are do-or-send pairs on the human sub-record at `+0x10C4` (the AI data), with payloads `{u16 handle, u8}` (3C), `{u16}` (3E), `{u16, u8, u16, u8, u8}` (3F), `{handle, u8}` (40) and `{handle, blob, u8}` (41). The senders are `human_pkt_send_3c` / `_3e` / `_3f` / `_40` / `_41`; their callers (`fuzz_crime_littering::init`, `fuzz_level_instance::load_from_table`, `heli_info::ctor`, `mp_bh_activity_hitman_host_instance::vf2`) say these set up crime / activity NPC AI data on every peer.

## Structures

### Bit stream (serialiser)

```
bit_stream   (what ECX/EAX/EDI/ESI point at, depending on the helper)
  +0x00  u8*  buf
  +0x0C  u32  size in bits
  +0x10  u32  cursor in bits
```

- Bits are stored LSB-first inside each byte: bit `i` of the stream is `buf[i>>3] & (1 << (i&7))`; the masks come from the 8-entry stride-4 table at 0xDC8558.
- Writers store the **cursor first**: when `cursor + count > size` (unsigned) the write is skipped but the cursor still advances, so the caller sees an overflow only by comparing cursor and size afterwards.
- Readers return 0 on overflow and still advance the cursor.
- No byte alignment: u8 / u16 / u32 values are written at the current bit position.

API (all in 0x7FD030-0x7FDB70; register conventions are per function in the names table):

| Operation | Writer | Reader |
|---|---|---|
| 1 bit | `net_bits_write_bit` 0x7FD0B0 (ECX stream, DL) | `net_bits_read_bit` 0x7FD960 (ECX stream, AL) |
| u8 / s8 | 0x7FD100 / 0x7FD260 (EAX stream, DL) | 0x7FD9B0 / 0x7FD9D0 (EDI stream, EAX) |
| u16 / s16 | 0x7FD3C0 / 0x7FD490 (ECX stream, DX) | 0x7FD9F0 / 0x7FDA10 (EDI stream, EAX) |
| u32 / s32 | 0x7FD560 / 0x7FD630 (ECX stream, EDX); stack form 0x7FD860 | 0x7FDA30 / 0x7FDA50 |
| f32 | the u32 writer on the raw bits | `net_bits_read_f32` 0x7FDA90 (ST0) |
| vec3 / vec2 | 0x7FD870 / 0x7FD8A0 (ECX stream, ESI float*) | 0x7FDAC0 / 0x7FDB20 (EDI stream, out float* on the stack) |
| bytes | 0x7FD8C0 (EAX stream, EDI count, src on the stack) | 0x7FDB70 (stack: stream, out, count; ret 0xC) |
| at a position (no cursor) | `net_bits_write_at` 0x7FD030 | `net_bits_read_at` 0x7FD8F0, `net_bits_read_n` 0x7FD940 (reads at the cursor) |

The `*_compressor` classes (`byte_data_noncompressor`, `bit_data_noncompressor`, `float_compressor`, `vector3_compressor`, `vector2_compressor`, `vector3_uncompressed`, `orientation_*`) call these helpers from their `compress` / `decompress` slots (named by vtable in functions.csv). Their constructors are now named in the CSV (`net_*_init`, `net_vector3_compressor_ctor`, ...).

### Session (global pointer 0x2528C14, class `pc_multi_session`)

Offsets seen in this round (all confirmed by at least one reader and writer unless marked):

| Off | Field |
|---|---|
| +0x00 | vtable (0xE2698C, 21 slots) |
| +0x04 | session id byte (cached into 0xE88587 by `net_set_active_session`) |
| +0x08 | network clock in ms (`net_clock_ms`) |
| +0x0C | `pc_multi_session_info*` (+0x43, +0x46, +0x47 flag bytes are tested by the join flow) |
| +0x1C | session state (u32; predicates `session_state_in_*`) |
| +0x3F4 | reservation count (entries of 0x50 bytes at +0x5C, GUID at +0, id at +0x24) |
| +0x3F8 | byte: hosting / authoritative flag (inferred; read by `session_hosting_flag`) |
| +0x400 | head of the circular connection list (link at conn +0x22CC) |
| +0x404 | designated peer connection `P` used by the route checks (inferred: host / server) |
| +0x408 | local connection |
| +0xC00 | byte set by `session_dispatch_action` |

Globals: 0x2528B48 network game active, 0x2528B49 joined as client (read by `coop_session_active`), 0x2528B56 and 0x2528B2B select the hosting kind (`session_hosting_flag`), 0x252A3CC the transport clock used by the ack code, 0x212B160 / 0x212B15C offline clock and frame delta, 0xE843AC the wall clock used for the timeout helpers.

### Connection (peer, at least 0x22D0 bytes; list node of the session)

| Off | Field |
|---|---|
| +0x000 | byte: 1 for the local connection (state broadcasts only fire for it) |
| +0x1AC | owning session |
| +0x1B0 | connection id byte (the owner byte stored in every multi record) |
| +0x1B8 | byte: alive flag (cleared by `net_conn_check_timeout`) |
| +0x3C | connection state; compared against the packet state mask (`net_packet_state_allows`) |
| +0x40 | phase value (`net_conn_set_phase_broadcast`) |
| +0x44, +0x45 | the two bytes of `mp_info` (0x5B) |
| +0x15C0 | ack-window count; +0x15C4 u16 epoch / sequence |
| +0x168E | u16 count of received ids, ids at +0x15C6 (max 0x64) |
| +0x1C0.. | 16-byte ack-window entries: u16 id, u32 sent time (+4), byte +8 acked, flags |
| +0x171E | outgoing datagram buffer; fill u16 at +0x1C32 (limit 0x50B) |
| +0x1CBA | u16 list of recent ids, count byte at +0x1DBA (max 0x7F) |
| +0x1DBC | metric u32 used by `session_find_best_conn`; +0x1DCC byte excluded flag |
| +0x1DEC | last send time; +0x1DE8 u32 count |
| +0x1DF0 | u32 list of owned object handles, count at +0x21F0 |
| +0x220C | 10-entry round-trip ring (index byte +0x2234, total +0x2238) |
| +0x2240 | last receive time |
| +0x22CC | next connection (circular) |

### Multi record and remote-handle map

The multi record (doc 08) has `+0 owner id`, `+2 net id`, `+4 object handle`, `+8 type`, `+0x67C` flags (bit 0 = migration pending), `+0x67E` u16 peer mask. The ownership predicates read exactly these: a handle is owned locally when `[rec] == [local conn +0x1B0]` and bit 0 of `[rec+0x67C]` is clear.

The remote-handle map (0x212E258) is a chained hash table: bucket array at `+0x5C`, bucket count at `+0x60`, free-node count at `+0x1C`, node allocator at `+0x20`; node `+0` = payload (the multi record), `+4` next, `+8` prev, `+0xC` key (net id), `+0x10` bucket. `net_remote_handle_map_find` is its only lookup (see recipe 9).

## Names

Columns: address, name, registers and result, evidence, C / I. The same text is in the CSV.

| Address | Name | Registers, result | Evidence | C/I |
|---|---|---|---|---|
| 0x7F7910 | `net_frame_update` | no args; when [0x2528B48] (network mode) is set: net_clock_advance, 0x7FA280, 0x817BA0, 0x8312A0, 0x80AF30, 0x82E210, 0x7F8390 (connection tick), then branches on the session state [session+0x1C] | sole caller net_do_frame (0x809640) | I |
| 0x7F7A00 | `net_local_conn_id` | no args -> AL = byte [[session+0x408]+0x1B0] (local connection id) or 0 without a session | caller 0x771E7F | C |
| 0x7F8020 | `net_id_to_human` | AX=net id -> EAX=human object or 0; id -> map -> handle -> object table; requires [obj+0x4C]==1 (human) and byte [obj+0x3CC]==1, else 0 (sbb mask) | 47 callers; cl=[obj+0x3CC]; cl-=1; neg cl; sbb ecx,ecx; not ecx; eax&=ecx keeps obj only when the byte is exactly 1 | C |
| 0x7F8160 | `net_conn_reset_ack_window` | EAX=conn*; clears the sent-packet window: [conn+0x15C0]=0, words [+0x168E],[+0x1690],[+0x1C0], dword [+0x1C4], bytes [+0x1C8..0x1CA], dword [+0x1CC]; then bumps the u16 epoch [conn+0x15C4] (wraps 0xFFFF to 0) | callers 0x7F83AD, 0x7F9832, 0x7FBD40 | C |
| 0x7F81D0 | `net_conn_ack_window_push` | ECX=conn*; if [conn+0x1DEC] is not negative and within the clock window (/now-t/<0x35A4E900) appends a 16-byte entry (u16 seq, time, flags) at conn+(n+0x1C)*16 and bumps n [conn+0x15C0] and the seq [conn+0x15C4] | sole caller 0x7FA660 (reliable send) | I |
| 0x7F8270 | `net_conn_ack_record` | EAX=conn*, SI=packet seq; finds the window entry (stride 0x10 from conn+0x1C0) with that u16 id, marks it acked (+0x1C8=1) and pushes the round-trip sample ((now-sent)>>1) into the 10-entry ring at conn+0x220C (index byte +0x2234, total +0x2238); needs [conn+0x1B0]<0xC | callers net_recv_acks, 0x80BE03; now=[0x252A3CC] | C |
| 0x7F8300 | `net_conn_loss_ratio` | EAX=conn* -> ST0 (f32); scans [conn+0x15C0] entries (stride 0x10 from conn+0x1C4): for entries with byte+5==0 and time+0x3E8 < now ([0x252A3CC], unsigned jae skips) total++, and if byte+4==0 also unacked++; if total>=0x14 (unsigned jae) returns unacked/total (fild each, fdivp, rounded through fstp dword) else 0.0; null conn or empty window -> 0.0 | callers net_send_badcon, 0x7FB7E0 | C |
| 0x7F8BA0 | `net_conn_set_state_broadcast` | ECX=new state, ESI=conn*; if [conn+0x3C] differs, the old value is >=1 and the new value is >1 (signed jl/jle): stores [conn+0x3C]=ECX and for the local conn ([conn]!=0) broadcasts packet 0x20 upd_state sub 0 {u8 state} via net_send_packet_to_remotes (EDI=[conn+0x1AC]); then calls the session callback [session+0xCAC](conn) if set | 47 callers; header 3 or 4 bytes decided by record 0x20 +0x14 (byte 0xE8C21C) | C |
| 0x7F8C50 | `net_conn_set_phase_broadcast` | ECX=conn*, EDX=value; if [conn+0x40]!=EDX stores it and, for the local conn, broadcasts packet 0x20 upd_state sub 1 {u32 value} | sole caller 0x80D229; payload u8,u32 in net_payloads.csv | C |
| 0x7F8D70 | `net_send_upd_state_sub3` | ECX=conn*; if [conn]==0 builds packet 0x20 upd_state sub 3 (no payload) and net_send_packet(buf,len,0,0) with ESI=conn | sole caller 0x80D229 | C |
| 0x7F8DF0 | `net_send_mp_info` | ECX=conn*; builds packet 0x5B mp_info {u8 [conn+0x44], u8 [conn+0x45]} (header size from byte 0xE8BA94 = record 0x5B +0x14) and broadcasts via net_send_packet_to_remotes with EDI=[conn+0x1AC] | callers 0x7F8B14, 0x81C880, 0x83AF45; receiver net_recv_mp_info | C |
| 0x7FA390 | `net_conn_queue_unreliable` | ESI=conn*, EDI=length, [esp+4]=data ptr (cdecl, plain ret) -> AL; 0 if [conn+0x1B8]==0, length>=0x50B, or length>0x50B-[conn+0x1C32]; else under critical section 0x2A157EC memcpy to conn+0x171E+fill and fill (u16 [conn+0x1C32]) += length, AL=1 | sole caller net_send_packet_routed (transport 0x7FA390 in doc 25) | C |
| 0x7FA400 | `net_conn_queue_reliable` | ECX=conn*, EAX=length, [esp+4]=data ptr, [esp+8]=flag (ret by caller cleanup, plain ret); returns 0 if [conn+0x1B8]!=1 or length>=0x50B; takes the 0x2A157EC critical section, gets a reliable block with 0x7F8810, memcpy at block+2+used (u16 used [blk+0x202] += length, count +0x204), first fill stamps [blk+0x208]=now | sole caller net_send_packet (transport 0x7FA400 in doc 25) | C |
| 0x7FA9E0 | `net_conn_check_timeout` | EDI=conn* -> AL; returns 1 if [0x2528B56]!=0; 0 if [conn+0x1B8]==0; limit = 0x3A98 ms when [[conn+0x1AC]+0x1C]==0xA else 0xEA60; elapsed=now-[conn+0x2240] with wrap correction; if elapsed>limit clears [conn+0x1B8] and returns 0; else AL=(elapsed<=0x3A98) | sole caller 0x81062C; unsigned ja/jbe compares | C |
| 0x7FAA60 | `net_conn_recent_ids_push` | EAX=conn*, SI=u16 id; maintains a de-duplicated list of recent ids at conn+0x1CBA (count byte [conn+0x1DBA], max 0x7F): removes an existing occurrence by shifting down, appends the id | caller 0x813BA0 (receive path) | I |
| 0x7FAEF0 | `net_conn_recent_ids_contains` | EDX=conn*, SI=u16 id -> AL=1 if the id is in conn+0x1CBA[0..count byte [conn+0x1DBA]) | caller 0x813BA0 | C |
| 0x7FB100 | `net_conn_note_received` | EAX=conn*, DX=u16 packet id; if [conn+0x1B8]!=0, id!=0xFFFF and [conn+0x168E]<0x64 appends the id at conn+0x15C6 and bumps the count; always stamps [conn+0x2240]=now ([0x252A3CC]) | caller 0x813BA0 | C |
| 0x7FB190 | `net_conn_ack_window_mark_all` | EDX=conn*; sets byte +1 (offset 0x1C9 + n*0x10) of every window entry | sole caller 0x80A77C | I |
| 0x7FB1C0 | `net_rate_meter_init` | ECX=limit (clamped to 0xE000), DL=window length (clamped to 0x1E), [esp+4]=meter*, [esp+8]=u16 tag, ret 8; stores limit at [m], window at [m+0x7D], tag at [m+0x7E] and zeroes the 30-entry sample ring (+4..+0x78) | callers 0x7F9832, 0x7F9A13 (connection setup) | I |
| 0x7FB2B0 | `net_rate_meter_remaining` | EDX=meter* -> EAX; sums the last [m+0x7D] ring samples ([m+4+idx*4], head [m+0x7C], wrapping 0..0x1D) and returns limit-sum, or 0 if sum>limit (unsigned jbe) | pairs with net_rate_meter_init | I |
| 0x7FC890 | `net_timeout_clear` | no args; [0x2528B89]=0, [0xEB1614]=-1 | leaf | C |
| 0x7FC8B0 | `net_timeout_expired` | no args -> AL; AL=1 when the armed deadline [0xEB1614] is reached by now [0xE843AC] (wrap 0x35A4E900); 0 when disarmed ([0x2528B89]==0 or deadline<0) | caller 0x80AAE0 | C |
| 0x7FC8F0 | `net_timeout_arm_10s` | no args; deadline=[0xE843AC]+10000 (wraps at 0x6B49D200), [0x2528B89]=1, [0xEB1614]=deadline, tail-jumps 0x80B8D0 | callers 0x826621, 0x8304A2 | C |
| 0x7FC920 | `net_clock_reset` | no args; zeroes the frame delta 0x212B15C and the offline clock 0x212B160 | leaf | C |
| 0x7FC930 | `net_clock_advance` | no args; adds the frame delta [0x212B15C] to [0x212B160] and to field +8 of every record of the 0x1044-byte pool at 0x2A18938 (critical section 0x2A18944); skipped when the session is the hosting kind | called from net_frame_update (0x7F7910) | C |
| 0x7FCA90 | `net_clock_ms` | no args -> EAX = [session+8] (network clock in ms) or the offline clock [0x212B160] when there is no session | 23 callers (gsi_timer::vf0, 0x4019AF, 0x80D545) | C |
| 0x7FCAB0 | `net_clock_predicted_ms` | no args -> EAX; if ([0x2528B56]==0 or [0x2528B2B]!=0) and [session+0x3F8]!=0: [session+8] - trunc(f32 [0x213002C] * qword [0xE5C200]) + 0x64 (fistp with RC=truncate); else [session+8] + [local+0x1DE8]*1000 when that count is non-zero, else [session+8] | x87 fldcw round-toward-zero; no callers found | I |
| 0x7FD030 | `net_bits_write_at` | EAX=bit pos, ECX=bit count, ESI=stream, [esp+4]=value, ret 4; writes the low ECX bits of value LSB-first at EAX; cursor untouched; no-op if pos+count>size (unsigned ja), count>0x20 or count==0; preserves EBX/EDI; clobbers EAX/ECX/EDX; no result | random-access writer; callers are the 0x6E1xxx-0x6E3xxx compressor routines | C |
| 0x7FD0B0 | `net_bits_write_bit` | ECX=stream, DL=bit (bit0 only); sets/clears the bit at the cursor then cursor+=1 (also past the end, where the write is skipped); preserves ESI; clobbers EAX/EDX; result unused | callers 0x81D36F, 0x85881D, 0x859BBC (compressors) | C |
| 0x7FD100 | `net_bits_write_u8` | EAX=stream, DL=value; 8 bits LSB-first at the cursor; cursor+=8 even if the write is skipped (cursor+8>size, unsigned ja); clobbers ECX/EDX, EAX stays the stream; preserves EBX/ESI/EDI | callers 0x8584BF, 0x8584F2, 0x85880D and net_bits_write_bytes; unrolled bit test/set; last bit is test dl,2 after six shr | C |
| 0x7FD260 | `net_bits_write_s8` | EAX=stream, DL=value; same as net_bits_write_u8 but movsx edx,dl (writes the same 8 bits) | callers npc_fast_data_compressor::compress, npc_slow_data_compressor::compress, player_data_compressor::compress; one opcode differs from 0x7FD100 | C |
| 0x7FD3C0 | `net_bits_write_u16` | ECX=stream, DX=value; 16 bits LSB-first at the cursor, cursor+=16 always; clobbers EAX/EDX; preserves ESI/EDI/EBX/EBP | callers 0x8584D0, 0x8584E1, 0x85882D | C |
| 0x7FD490 | `net_bits_write_s16` | ECX=stream, DX=value; same as net_bits_write_u16 with movsx edx,dx | caller blocking_animation_compressor_full::compress; one opcode differs from 0x7FD3C0 | C |
| 0x7FD560 | `net_bits_write_u32` | ECX=stream, EDX=value; 32 bits LSB-first at the cursor, cursor+=32 always; floats are written as raw bits; clobbers EAX/EDX | callers 0x7FD864 (thunk), 0x7FD879 (vec3), 0x8586C7, 0x8587FB | C |
| 0x7FD630 | `net_bits_write_u32_dup` | ECX=stream, EDX=value; byte-identical copy of net_bits_write_u32 (not folded by LTCG) | callers player_ragdolling_compressor::compress, vehicle_hit_points_compressor::compress, mp_bh_custom_match_general::vf0; 0xC9 bytes compared equal | C |
| 0x7FD700 | `net_bits_write_u8_dup` | EAX=stream, DL=value; byte-identical copy of net_bits_write_u8; no direct callers | 0x154 bytes compared equal to 0x7FD100 | C |
| 0x7FD860 | `net_bits_write_u32_stack` | ECX=stream, [esp+4]=value, ret 4; thunk to net_bits_write_u32 | body: mov edx,[esp+4]; call 0x7FD560; ret 4 | C |
| 0x7FD870 | `net_bits_write_vec3` | ECX=stream, ESI=float[3]*; writes x,y,z as three 32-bit values (fld/fstp through the stack, bits unchanged); clobbers EAX/EDX | three calls to net_bits_write_u32; counterpart of net_bits_read_vec3 | C |
| 0x7FD8A0 | `net_bits_write_vec2` | ECX=stream, ESI=float[2]*; writes x,y as two 32-bit values; the second write is a tail jump | two net_bits_write_u32 calls; counterpart of net_bits_read_vec2 | C |
| 0x7FD8C0 | `net_bits_write_bytes` | EAX=stream, EDI=byte count, [esp+4]=source u8*, ret 4; loops net_bits_write_u8 per byte | loop dl=[src+i], call 0x7FD100 | C |
| 0x7FD8F0 | `net_bits_read_at` | EAX=bit pos, ESI=bit count, EDX=stream -> EAX=value (bit i = stream bit pos+i); returns 0 if pos+count>size (unsigned ja) or count>0x20; cursor untouched; clobbers ECX/EDX; preserves EBX/EBP/EDI/ESI | 136 callers: every net_bits_read_* below plus 0x550BF0, 0x6E1931, 0x6E288A (compressors) | C |
| 0x7FD940 | `net_bits_read_n` | EAX=bit count, EDI=stream -> EAX=value; reads at the cursor then cursor+=count (also when the read failed); preserves EBX/ESI | calls net_bits_read_at with EAX=cursor; callers 0x6E16B8, 0x6E4395 | C |
| 0x7FD960 | `net_bits_read_bit` | ECX=stream -> AL (0 or 1); if cursor+1>size (unsigned ja) returns 0; cursor+=1 either way; preserves ESI; clobbers EAX/EDX | callers 0x81DCEA, 0x8588F6, 0x859C37, 0x859C6B | C |
| 0x7FD9B0 | `net_bits_read_u8` | EDI=stream -> EAX (8 bits), cursor+=8 | net_bits_read_at with ESI=8 | C |
| 0x7FD9D0 | `net_bits_read_s8` | EDI=stream -> EAX raw 8 bits (no sign extension here), cursor+=8; body identical to net_bits_read_u8 | LTCG duplicate; signedness is applied by callers | I |
| 0x7FD9F0 | `net_bits_read_u16` | EDI=stream -> EAX (16 bits), cursor+=16 | caller 0x85E45D; net_bits_read_at with ESI=0x10 | C |
| 0x7FDA10 | `net_bits_read_s16` | EDI=stream -> EAX raw 16 bits, cursor+=16; body identical to net_bits_read_u16 | LTCG duplicate | I |
| 0x7FDA30 | `net_bits_read_u32` | EDI=stream -> EAX (32 bits), cursor+=32 | net_bits_read_at with ESI=0x20 | C |
| 0x7FDA50 | `net_bits_read_s32` | EDI=stream -> EAX (32 bits), cursor+=32; body identical to net_bits_read_u32 | LTCG duplicate | I |
| 0x7FDA70 | `net_bits_read_u8_dup` | EDI=stream -> EAX (8 bits), cursor+=8; identical to net_bits_read_u8 | LTCG duplicate | I |
| 0x7FDA90 | `net_bits_read_f32` | EDI=stream -> ST0 (32 raw bits reinterpreted as f32), cursor+=32; value goes through [esp+4] with fld dword | fld dword ptr [esp+4] after net_bits_read_at | C |
| 0x7FDAC0 | `net_bits_read_vec3` | EDI=stream, [esp+4]=out float[3]*, ret 4; reads x (raw dword store), y, z (fld/fstp), cursor+=96 | callers vector3_uncompressed::decompress, supported_by_car_compressor::decompress | C |
| 0x7FDB20 | `net_bits_read_vec2` | EDI=stream, [esp+4]=out float[2]*, ret 4; reads x then y, cursor+=64 | body 0x7FDB20-0x7FDB5F; counterpart of net_bits_write_vec2 | C |
| 0x7FDB70 | `net_bits_read_bytes` | [esp+4]=stream, [esp+8]=out u8*, [esp+0xC]=count, ret 0xC; loops 8-bit reads storing byte by byte, cursor+=8*count | callers 0x7FDC2B, 0x7FDD7E, 0x858540 (noncompressor decompress) | C |
| 0x7FDBB0 | `net_byte_noncompressor_init` | EAX=this; writes vtable 0xE264C4 (byte_data_noncompressor) and [this+4]=0; no result | RTTI vtable 0xE264C4; no direct callers | I |
| 0x7FDBC0 | `net_byte_noncompressor_init_bits` | EAX=this, ECX=bit count; vtable 0xE264C4, [this+4]=ECX>>3 (bytes) | shr ecx,3 then store | I |
| 0x7FDC90 | `net_bit_noncompressor_init` | EAX=this; vtable 0xE264D8 (bit_data_noncompressor), [+4]=[+8]=[+0xC]=0 | RTTI vtable 0xE264D8 | I |
| 0x7FDCB0 | `net_bit_noncompressor_init_bits` | EAX=this, ECX=bit count; vtable 0xE264D8, [+4]=ECX, [+8]=ECX>>3, [+0xC]=ECX&7 | shr/and in the body | I |
| 0x7FDE30 | `net_float_compressor_init` | EAX=this, ECX=vtable ptr, floats at [esp+4],[esp+8],[esp+0xC],[esp+0x10], byte at [esp+0x18]; stores them at this+4..+0x10 and this+0x18 | float_compressor field setup | I |
| 0x7FDE60 | `net_float_compressor_init_copy` | EAX=this, 7 dwords on the stack, ret 0x1C; copies them to this+4 and writes vtable 0xE264EC (float_compressor) | rep movsd with ecx=7 | I |
| 0x7FE0B0 | `net_vector3_compressor_ctor` | EAX=this, three float_compressor blocks (7 dwords each) and a float on the stack; vtable 0xE26500 (vector3_compressor); embeds sub-objects at +4/+0x24/+0x44 with vtable 0xE264EC | RTTI vtable 0xE26500; three rep movsd | I |
| 0x7FE110 | `net_float_compressor_copy` | EAX=dst, ECX=src; copies 7 dwords at +4 | rep movsd ecx=7 | I |
| 0x7FE3A0 | `net_vector2_compressor_ctor` | EAX=this, two float_compressor blocks and a float on the stack; vtable 0xE26514 (vector2_compressor) | RTTI vtable 0xE26514 | I |
| 0x7FE5A0 | `net_vector3_uncompressed_ctor` | EAX=this, [esp+4]=float, ret 4; vtable 0xE26528 (vector3_uncompressed), [this+4]=float | RTTI vtable 0xE26528 | I |
| 0x7FE6A0 | `net_orientation_heading_compressor_ctor` | EAX=this, 7 dwords + a float on the stack, ret 0x1C; vtable 0xE2653C (orientation_heading_compressor), float_compressor sub-object at +4/+8, float at +0x24 | RTTI vtable 0xE2653C | I |
| 0x7FFDF0 | `net_id_to_object_ac` | AX=net id -> EAX=[obj+0xAC] of the mapped object, 0 if none/stale; id 0xFFFF returns 0 | same lookup as net_id_to_handle then returns the +0xAC field; 6 callers | I |
| 0x804140 | `net_object_unregister` | EAX/ECX=multi record*; removes it from the per-type circular list (+0x674/+0x678) and the handle map, frees snapshot data (0x7FFB70, 0x807AA0) | callers 0x804550, 0x8054DA, net_recv_rop_create, net_registrations_reset | C |
| 0x804320 | `net_object_register_from_create` | registers an object from a create packet and acks with net_send_create_ack; calls session_find_conn_by_id, 0x804050, net_remote_handle_map_set | callers net_recv_rop_create, 0x807990; doc 08 | C |
| 0x804550 | `net_object_destroy_from_packet` | receive side of rop_destroy: looks the id up (net_remote_handle_map_find), calls 0x836DD0 and 0x8044C0, then net_object_unregister | sole caller net_recv_rop_create path; doc 08 slot 6 | I |
| 0x804940 | `net_conn_object_list_add` | EAX=connection*, EDX=object handle; appends to the u32 list at [conn+0x1DF0], count at [conn+0x21F0] | pair of net_conn_object_list_remove | I |
| 0x804960 | `net_conn_object_list_remove` | EAX=connection*, EDI=handle -> AL (1 removed, 0 absent); swap-with-last removal from [conn+0x1DF0], count [conn+0x21F0] | callers net_object_deactivate_send, 0x806DB0, 0x80755F | C |
| 0x804F70 | `net_handle_to_net_id` | ECX=object handle -> AX=net id (u16) = [[obj+0xB0]+2]; AX=0xFFFF for handle 0, stale handle (slot (h&0xFFF)*16+0x2149C64 -> obj, [obj+0x44]!=h) or no record. NOTE: the handle==0 path only does or ax,0xFFFF so the upper half of EAX keeps its old value; the other paths return a clean EAX | 326 callers: every sender that packs a handle (vehicle_damage_events_raise, human_pkt senders) | C |
| 0x804FB0 | `net_id_to_handle` | AX=net id -> EAX=local object handle (0 if none); AX==0xFFFF returns 0; looks the id up in the remote-handle map 0x212E258 via net_remote_handle_map_find (create=0) and returns [[node]+4]; preserves ESI | inverse of net_handle_to_net_id; node[0] is the multi record whose +4 is the handle (doc 08) | C |
| 0x805000 | `net_object_register_local` | registers a locally created object in the multiplayer record list (doc 08 Register for local objects); calls 0x804050 (alloc record), net_remote_handle_map_set, 0x7FFAC0 | 28 callers (train_ride_prompt, activity code); 0x212E25C[type] list links | C |
| 0x8053B0 | `net_object_deactivate_send` | object deactivate hook: builds packet 0x0F rop_destroy {u32, u8, u16} with net_send_packet to the peers that know the object, drops it from the owner conn list (0x804960) and unregisters it (0x804140) | sole caller object::deactivate; payload in docs/net_payloads.csv; doc 08 | C |
| 0x805CD0 | `net_object_capture_snapshot` | EDX=object handle; validates it then dispatches registration slot 9 (+0x24) | caller 0x800030; doc 08 slot 9 | C |
| 0x805D60 | `net_object_read_state_delta` | EAX=object handle, 4 stack args -> EAX; dispatches registration slot 12 (vtable +0x30); returns 2 when the handle is stale or unregistered | sole caller net_recv_rop_update; doc 08 slot 12 | C |
| 0x805E40 | `net_object_get_send_priority` | EAX=record*, 7 stack args -> AX; dispatches registration slot 14 (+0x38) with [record+4] (handle); AX=0 when null | callers 0x806073 x2; doc 08 slot 14 (distance buckets) | C |
| 0x805E90 | `net_object_get_dirty_fields` | EAX=record*, 2 stack args -> AX; dispatches registration slot 13 (vtable +0x34); AX=0 when the record is null | caller 0x806073; doc 08 slot 13 | C |
| 0x805EC0 | `net_object_on_state_sent` | EAX=object handle, 3 stack args; dispatches registration slot 15 (+0x3C) after validating the handle | callers 0x805FA0, 0x806073; doc 08 slot 15 | C |
| 0x805FC0 | `net_state_sync_send` | per-peer state sync: for every owned record captures a snapshot, computes dirty fields against the peer baseline, sorts, packs deltas (registration slot 11) until the 0x514-byte packet is full; uses vec3_length and _ftol2_sse for the distance buckets | sole caller 0x814DB1 (network upkeep); doc 08 State sync | C |
| 0x8066C0 | `net_handle_owned_locally` | ECX=object handle -> AL; AL=1 for handle 0, stale handle, no multi record, no session ([0x2528C14]) or no local conn ([session+0x408]); otherwise AL=1 only if owner byte [rec]==[local+0x1B0] AND ([rec+0x67C]&1)==0, else 0; clobbers EAX/ECX/EDX | 260 callers (human_ai_set_force_flag_*, human_pkt_send_21); handle form of human_owned_locally 0x806720 | C |
| 0x806760 | `net_record_owned_locally` | ECX=multi record* -> AL; AL=1 for null record, no session or no local conn; AL=0 if [rec]!=[local+0x1B0]; else AL=1 iff ([rec+0x67C]&1)==0 (test [..],al with al=1 then je returns 1) | caller net_request_ownership; same rule as net_handle_owned_locally on a record | C |
| 0x8067D0 | `net_object_send_new_owner` | sends packet 0x2A rop_new_owner (net_send_rop_new_owner) after net_remote_handle_map_find + session_find_conn_by_id + net_object_can_migrate | sole caller net_object_give_ownership | I |
| 0x806910 | `net_object_give_ownership` | hands an owned object to another peer: checks net_object_can_migrate, builds packet 0x27 rop_take_own with the slot-19 migration payload (memcpy), calls net_object_send_new_owner and 0x836C90 | 19 callers (ai_order_k02_enter/update, security_info::vf16); doc 08 | C |
| 0x806B90 | `net_request_ownership` | cdecl (obj*, u8 arg, callback fn(handle, flag)) -> AL; if the record is already owned locally or flag 2 is set calls callback(handle,1) and returns 1; otherwise, with table room (0x2528BAC != 0x20), sends packet 0x28 rop_req_own {u16 net id, u8 arg} to the owner connection with net_send_packet, appends {id, callback} to 0x212BB38 and returns 1; returns 0 without a session/record or when the table is full | 32 callers (0x40630A.., vehicle_protect_objective::vf19); header size from record 0x28 +0x14 (byte 0xE8C2DC) | C |
| 0x806CE0 | `net_ownership_request_pending` | EAX=object* -> AL; AL=1 if the object net id (0xFFFF when it has none) is in the pending request table (u16 ids at 0x212BB38, stride 8, count 0x2528BAC) | written by net_request_ownership; scan is cmp word ptr [eax*8+0x212BB38],dx | C |
| 0x806D50 | `net_object_can_migrate` | EAX=object* -> AL; 0 if null, [obj+0x80]&0xB, no record, or word [rec+0x67E]!=0xFFFF; else result of registration slot 17 (can_migrate) called with the handle; registration = 0x212E25C[rec.type] | 12 callers incl. net_object_give_ownership; doc 08 slot 17 | C |
| 0x806E30 | `net_registrations_reset` | no args; unregisters every record on the global list 0x212E2AC (net_object_unregister) then zeroes the per-type lists 0x212E284..0x212E2A8 and 0x212B4F0..0x212B518, 0x212BA34; sets byte 0xE8860A=1 | 7 callers (net_set_active_session and session teardown) | C |
| 0x807230 | `net_object_owner_conn` | EAX=object* -> EAX=owning connection* or 0 (null object or no record -> 0); DL=[[obj+0xB0]], tail-jumps session_find_conn_by_id with the current session | 26 callers; object-taking sibling of net_handle_owner_conn | C |
| 0x807250 | `net_handle_owner_conn` | ECX=object handle -> EAX=owning connection* (session conn with id == [rec]) or 0 | 198 callers; resolves handle->record->owner byte then tail-jumps to session_find_conn_by_id with EAX=[0x2528C14] | C |
| 0x807680 | `net_registration_find_field` | ESI=registration*, EDI=field key -> EAX=entry* (reg+0x20+idx*0x44, idx found by scanning dword [reg+0x60+i*0x44]==EDI) or 0 | 52 callers: *_multi_registration::get_send_priority / write_state_delta | C |
| 0x807780 | `net_registration_add_field` | EDX=registration*, 4 stack args, ret 0x10 -> AL (0 when 16 fields exist); appends a 0x44-byte field entry at reg+0x20+n*0x44 (n=[reg+0x18]), stores the entry index, n+=1 and recomputes the bit width [reg+0x1C] | 66 callers: registration setup functions 0x85E6C0, 0x869AF0, 0x870600 (doc 08 field list) | C |
| 0x807A20 | `net_remote_handle_map_set` | EAX=net id, [esp+4]=value (multi record*), ret 4 -> AL=1 on success; find-or-create (create=1) in 0x212E258 and stores [node]=value | callers net_object_register_from_create, net_object_register_local | C |
| 0x808E30 | `net_remote_handle_map_find` | ESI=map (e.g. 0x212E258), [esp+4]=key u32, [esp+8]=create flag u8, ret 8 -> EAX=node or 0. Hash: h=key-(key<<15)-1; h^=h>>10 (sar); h*=9; h^=h>>6 (sar); h+=~(h<<11); h^=h>>16 (sar); h is never negative; bucket = h idiv [map+0x60]; chain via node+4, key at node+0xC; on a miss with create!=0 and free count [map+0x1C]>0 allocates (0x809220, EDI=map+0x20), sets node+0xC=key, +0x10=bucket and links at the head of bucket array [map+0x5C] | 160 callers (ai_order_net_read, human_pkt receivers, net_id_to_handle); hash is the Thomas Wang 32-bit mix | C |
| 0x80C350 | `session_guid_in_array` | EAX=array of 0x28-byte entries, [esp+4]=count, [esp+8]=other entry*; AL=1 if any array entry has the same GUID as other+0x14C and the same dword +0x24 | callers 0x80C3A0, 0x80DEC0 | I |
| 0x80C4E0 | `session_find_best_conn` | EDI=first conn*, [esp+4]=excluded conn* -> EAX; walks the circular list and returns the connection with the largest [conn+0x1DBC] among those not excluded, with [conn+0x1DCC]==0 and [conn+0x1B8]!=0 (first maximum wins: ja) | sole caller 0x81062C (host selection after a leave) | I |
| 0x80C560 | `session_info_base_init` | EAX=this; writes vtable 0xDFFD54 (multi_session_info_base) and initialises +0x30,+0x34,+0x38,+0x3C=1,+0x41..+0x49 (not +0x45) | RTTI vtable 0xDFFD54; leaf | I |
| 0x80CEB0 | `session_state_in_set_b` | EAX=session* -> AL; switch on [session+0x1C]-2 (unsigned, 0..10): AL=1 for states 2,4,7,9,10,11,12; AL=0 for 3,5,6,8 and anything below 2 or above 12 (index table 0x80CED8: 0 -> true, 1 -> false) | 9 callers (0x80CB30, 0x80CD70, 0x80F640) | C |
| 0x80CEF0 | `session_state_in_1_6_8` | EAX=session* -> AL; 1 if [session+0x1C] is 1, 6 or 8 | caller 0x833F26 | C |
| 0x80CF10 | `session_state_is_8` | no args; AL=([[0x2528CB4]+0x1C]==8) | tiny leaf | C |
| 0x80CF20 | `session_state_in_0_1` | EAX=session* -> AL; 1 if [session+0x1C] is 0 or 1 | 4 callers (0x69A090, 0x834E45) | C |
| 0x80ED70 | `session_dispatch_action` | EAX=action 0..5 (0xFF or >5 returns 2), ESI=session*; actions call vtable slot 3, 4 or 5 ([vtbl+0xC/+0x10/+0x14]) then set byte [session+0xC00]=1; returns 0 | 5 callers (0x80C9A0, 0x80CA00, net_recv_sessionend) | I |
| 0x80F2E0 | `session_find_reservation` | EAX=session*, stack (entry*, int* out idx) ret 8 -> AL; scans the reservation array (stride 0x50 from +0x5C, count [session+0x3F4]) from the end for an entry whose 16-byte GUID equals the argument (net_guid_equal) and whose dword +0x24 matches; writes the index to *out if non-null | callers 0x80F107, 0x80F6D5, 0x815D66 | C |
| 0x80F340 | `session_remove_reservation` | EAX=session*, EDX=index; count [session+0x3F4]-=1 and moves the last 0x50-byte entry over index unless it was the last | callers 0x80F3B0, 0x80F723 | C |
| 0x80FCF0 | `session_hosting_flag` | stdcall (session*) ret 4 -> AL; AL=0 when [0x2528B56]!=0 and [0x2528B2B]==0, else byte [session+0x3F8] | 281 callers (districts_ownership_refresh, 0x4B1570..); byte +0x3F8 is set to 1 by 0x80C947, 0x80CA7D, 0x80CCEA, 0x80EAD2 | I |
| 0x80FE80 | `session_find_conn_by_id` | EAX=session*, DL=conn id -> EAX=connection* or 0; walks the circular list [session+0x400] via [conn+0x22CC], compares byte [conn+0x1B0]==DL; clobbers EAX/ECX only | 195 callers; loop ends when the head comes round again or on null | C |
| 0x80FF00 | `session_peers_phase_equal` | EDX=session* -> AL; 1 when ([0x2528B56]==0 or [0x2528B2B]!=0) and [session+0x3F8]==0, or when every connection has the same [conn+0x40] as [session+0x404]; 0 if 0x8CD8D0(session) is true or any phase differs | 7 callers (0x80D229, 0x810040..0x810180) | I |
| 0x80FF70 | `session_out_of_phase_mask` | ECX=session* -> EAX bitmask; for each connection except the local one whose [conn+0x40] != [[session+0x404]+0x40] sets bit [conn+0x1B0] | callers 0x80D229, 0x80D3E7 | I |
| 0x810040 | `session_all_conns_state_eq` | ESI=session*, EDI=state -> AL; requires session_peers_phase_equal; 1 only if every connection has [conn+0x3C]==EDI (jne -> 0) | callers 0x826D00, 0x8337D0 | C |
| 0x810080 | `lobby_any_conn_state_4` | no args; walks the connection list of the session at [0x2528CE8]; AL=1 if any [conn+0x3C]==4 else 0 | 3 callers | C |
| 0x8100F0 | `session_all_conns_state_ge` | ESI=session*, EDI=state -> AL; requires session_peers_phase_equal; 0 as soon as any [conn+0x3C] < EDI (signed jl), else 1 | 12 callers (0x8252ED, 0x826DF0, 0x833A75) | C |
| 0x814740 | `net_send_packet_to_remotes` | cdecl (buf, len), EDI=session*; for every connection in the circular list [session+0x400] (link +0x22CC) except the local one ([session+0x408]) calls net_send_packet(buf,len,0,0) with ESI=conn; null buf or session = no-op; preserves EBX/EBP/ESI | 368 callers (almost every packet builder); this is the send-to-everyone helper, not a handle packer as doc 41 implies | C |
| 0x8148C0 | `net_send_packet_to_list_or_remotes` | cdecl (buf, len), session from [0x2528C14]; if the target count [0xEB1FE4]==0 tail-calls net_send_packet_to_remotes, else sends buf to each connection in the array [0xEB1FDC] except the local conn ([session+0x408]) | 84 callers | C |
| 0x814B00 | `net_packet_route_allows_send` | EAX=packet record*, ECX=destination conn* -> AL; r=[rec+0xC], P=[[conn+0x1AC]+0x404]: r==0 -> AL=(conn!=0 && conn==P); r==1 -> AL=(conn==0 // conn!=P); other r -> 1 | sole caller net_send_packet | C |
| 0x814B40 | `net_packet_route_allows_recv` | EAX=packet record*, ECX=source conn* -> AL; r=[rec+0xC], P as above: r==0 -> AL=(conn==0 // conn!=P); r==1 -> AL=(conn!=0 && conn==P); other r -> 1 (mirror image of the send check) | listed as 0x814B40 in doc 25 net_dispatch_packet | C |
| 0x814B80 | `net_packet_state_allows` | EAX=packet record*, ECX=conn* -> AL; word mask=[rec+0x10]; mask==0xFFFF -> 1; conn==0 -> 0; else AL=((mask & (1<<[conn+0x3C])) != 0) so mask bit n lets the packet through while the connection state [conn+0x3C]==n (0x1FF00 = states 8..15) | listed as 0x814B80 in doc 25; setne after test eax,edx | C |
| 0x815930 | `net_send_join_req` | builds packet 0x02 join_req (blob, u32, u32, u32/u8, blob, u8, u32, u32, u32) and sends it to the host | payload in docs/net_payloads.csv; receiver net_recv_join_req | I |
| 0x816460 | `net_send_join_new_session` | ECX=conn*, 1 stack arg; builds packet 0x05 join_new_session (type byte 5 at [esp+8]); header size from record 0x05 +0x14 (byte 0xE8BF94) | mov byte ptr [esp+8],5; receiver net_recv_join_new_session | I |
| 0x8165F0 | `net_send_join_success` | ECX=conn*, 1 stack arg; builds packet 0x06 join_success; flag byte 0xE8BFAC | mov byte ptr [esp+8],6; receiver net_recv_join_success | I |
| 0x816740 | `net_send_join_failed` | ECX=conn*, 1 stack arg; builds packet 0x07 join_failed; flag byte 0xE8BFC4 | mov byte ptr [esp+8],7; 8 callers | I |
| 0x8168A0 | `net_send_join_complete` | ECX=conn*, 1 stack arg; builds packet 0x08 join_complete; flag byte 0xE8BFDC | mov byte ptr [esp+8],8; 4 callers | I |
| 0x816B50 | `net_send_upd_conn` | builds packet 0x0B upd_conn {u8} | docs/net_payloads.csv | I |
| 0x816F60 | `net_send_join_deny` | builds packet 0x04 join_deny {u32 0xC9, u8 stack arg}; flag byte 0xE8BF7C | constant 0xC9 and one byte argument; 3 callers | I |
| 0x8176C0 | `net_guid_id_equal` | ESI=a*, EDI=b* -> AL; AL=1 iff net_guid_equal(a,b) and dword [a+0x24]==[b+0x24] | 2 callers; pairs the 16-byte GUID with the u32 id at +0x24 | C |
| 0x8176E0 | `net_guid_equal` | ECX=a*, EDX=b* (16 bytes each) -> AL; 0 if EDX null; AL=1 iff the 16 bytes are equal (dword loop then byte tail; eax==0 -> sete); preserves ESI/EDI | 30 callers (profile_frame, 0x73D534, session code) | C |
| 0x817810 | `net_guid_entry_read` | EAX=out* (0x28 bytes zeroed, [out+0x24]=-1 first), EDX=ptr to cursor, ESI=base ptr; reads two qwords into out+0/+8 and one dword into out+0x24, advancing the cursor by 8, 8, 4 | 9 callers (net_recv_migrate, net_recv_join_acc, net_recv_new_conn) | I |
| 0x817D80 | `net_pool_find_by_id` | cdecl-style (u8 id on the stack, caller cleans) -> EAX=record* in the 0x1044-byte pool at 0x2A18938 whose byte +4 == id, else 0; holds critical section 0x2A18944 and iterates with 0x69B9D0 | 34 callers (0x8178F0, 0x817980, net_set_active_session) | C |
| 0x817F40 | `net_pool_find_by_id_b` | same code as net_pool_find_by_id on a second pool (23 bytes differ: pool and lock globals) | 13 callers incl. net_recv_matchmaking | I |
| 0x818020 | `net_pool_find_by_id_c` | same code as net_pool_find_by_id on a third pool | 23 callers incl. mp_results_state_frame | I |
| 0x818140 | `net_set_active_session` | EDI=new session* (or 0); tears down the current one (net_pool_find_by_id, 0x836D50 for 0x17 slots, net_registrations_reset), stores [0x2528C14]=EDI, caches the session id byte [EDI+4] in 0xE88587 (0xFF when none) and sets 0x215DA18 from the hosting flag | callers 0x80F9B0, 0x82F3C0, 0x8337D0 | C |
| 0x81EB60 | `net_join_stage_set` | ECX=stage; raises the connect stage [0x2528C34] monotonically (stores only when the current value < ECX, and 0xB only if flag [0x2528C2D]==0), resets byte 0xE88586 and calls 0x81EC70(byte [0x2528C2E]) when [0x2528C30] is set | 47 callers (join flow, doc 40) | I |


## Existing placeholder names worth changing

Not added to the CSV (the file only takes addresses missing from functions.csv):

| Address | Current | Proposed | Evidence |
|---|---|---|---|
| 0x8118A0 | `pc_multi_session::vf5` | `pc_multi_session::call_slot20_return_0` | `mov eax,[ecx]; call [eax+0x50]; xor eax,eax` |
| 0x811930 / 0x811940 / 0x811950 | `pc_multi_session::vf7` / `vf8` / `vf9` | `..::return_0` (three stubs) | `xor eax,eax; ret` each |
| 0x811A80 | `pc_multi_session::vf11` | `pc_multi_session::get_cached_status_1040` | returns `[this+0x1040]`, which `vf10` (0x811960) fills in |
| 0x811A90 | `pc_multi_session::vf12` | `pc_multi_session::nop` | single `ret` |
| 0x558EE0 / 0x559C90 / 0x559780 / 0x55A250 / 0x55A450 | `human_pkt_sub_3c` / `3e` / `3f` / `40` / `41` | `human_pkt_ai_data_*` family | all act on the AI record at human `+0x10C4`, see the note under the packet table |

## Port candidates

Fifteen small leaves and helpers whose behaviour is fully determined by the code. All numbers below are from the disassembly. "Preserved" lists registers the original keeps; the caller-saved registers EAX / ECX / EDX that are not listed as inputs or outputs are scratch. For a shadow snapshot, "writes" says what memory the original touches.

Comparison polarity used in the recipes: `ja` / `jbe` / `jae` / `jb` are **unsigned**; `jl` / `jle` / `jge` are **signed**. A port that uses the wrong signedness on an overflow check will accept out-of-range writes.

### 1. `net_bits_write_bit` (0x7FD0B0)

- Inputs: ECX = stream, DL = bit (only bit 0 is used; `movzx edx,dl` then `test dl,1`).
- Outputs: none that callers use (EAX/EDX are scratch). Preserved: ESI (push/pop), EBX, EDI untouched.
- Callees: none. Leaf.
- Logic: `c = stream.cursor`. If `c + 1 > stream.size` (unsigned `ja`): skip the write. Otherwise `byte = stream.buf[c>>3]`, `m = 1 << (c&7)` (table 0xDC8558[(c&7)*4]); if `bit` is set `byte |= m` else `byte &= ~m` (`not al; and`). In **both** paths `stream.cursor = c + 1` (the final `add [ecx+0x10],1` is executed on the skipped path too).
- Writes: one byte of `buf` (read-modify-write) and the cursor dword.

### 2. `net_bits_write_u8` (0x7FD100; also `_s8` 0x7FD260 and the duplicate 0x7FD700)

- Inputs: EAX = stream, DL = value. Outputs: none (EAX still holds the stream pointer at return, not part of the contract). Preserved: EBX, ESI, EDI (pushed). Scratch: ECX, EDX.
- Callees: none. Fully unrolled: for bit `i` in 0..7, test bit `i` of the value and set / clear stream bit `cursor + i`.
- Overflow: `ecx = cursor + 8; if ecx > size (unsigned ja) -> skip all writes`; `cursor += 8` in both cases.
- `_s8` differs only by `movsx edx,dl`; the stored bits are identical, so a single Rust function covers all three.
- Writes: up to two bytes of `buf` (an 8-bit field can straddle two bytes) and the cursor.

### 3. `net_bits_write_at` (0x7FD030)

- Inputs: EAX = start bit, ECX = bit count, ESI = stream, `[esp+4]` = value, `ret 4`. Outputs: none. Preserved: EBX, EDI, ESI.
- Guard (in order): `edi = eax + ecx; if edi > size (ja) return; if ecx > 0x20 (ja) return; if ecx == 0 (jbe) return`.
- Loop: for `k = 0 .. count-1`: bit `k` of `value` (LSB-first, `test dl,1` then `shr edx,1`) goes to stream bit `start + k` (set with `or [p], m`, clear with `and [p], ~m`). The cursor is **not** modified.
- Writes: the touched bytes of `buf` only.

### 4. `net_bits_read_at` (0x7FD8F0)

- Inputs: EAX = start bit, ESI = count, EDX = stream. Output: EAX = value (bit `k` = stream bit `start + k`). Preserved: EBX, EBP, EDI (pushed), ESI. Scratch: ECX, EDX.
- Guard: `if start + count > size (unsigned ja) -> 0`; `if count > 0x20 (ja) -> 0`; `count == 0` falls out with 0 (`jbe`).
- Loop: `bit = ((buf[p>>3] & mask) != 0)` computed as `and bl,mask; neg bl; sbb ebx,ebx; neg ebx` (so 0 or 1), `value |= bit << k`. The shift count is `cl = k` (valid up to 31).
- Writes: nothing (pure read).

### 5. `net_bits_read_bit` (0x7FD960)

- Input: ECX = stream. Output: AL = bit (`setne al`, 0 or 1). Preserved: ESI. Scratch: EAX, EDX.
- Logic: `c = cursor; if c + 1 > size (unsigned ja) { cursor = c + 1; return 0 }`; else `bit = (buf[c>>3] & mask(c&7)) != 0` (`and dl,mask; neg dl; sbb edx,edx; neg edx; setne al`); `cursor = c + 1`.
- Writes: the cursor only.

### 6. `net_bits_read_n` (0x7FD940) and the typed readers (0x7FD9B0 .. 0x7FDA90)

- `net_bits_read_n`: input EAX = count, EDI = stream; output EAX; `value = read_at(cursor, count); cursor += count` (always, even when the read was rejected). Preserved: EBX, ESI.
- The typed readers are the same with a constant count: u8 / s8 / dup = 8 (0x7FD9B0, 0x7FD9D0, 0x7FDA70), u16 / s16 = 16 (0x7FD9F0, 0x7FDA10), u32 / s32 = 32 (0x7FDA30, 0x7FDA50). They return the **raw** bits in EAX (no sign extension); signedness is applied by the caller.
- `net_bits_read_f32` (0x7FDA90): reads 32 bits, stores them to `[esp+4]`, `fld dword ptr [esp+4]` -> ST0. In Rust: `f32::from_bits(read_u32())`; no rounding step.
- Writes: the cursor only.

### 7. `net_handle_to_net_id` (0x804F70)

- Input: ECX = handle. Output: **AX** = id. Scratch: ECX, EDX. No other registers touched.
- Logic: `if handle == 0 { ax |= 0xFFFF; return }` (upper half of EAX is *not* cleared on this path; callers use AX / `movzx`). Otherwise `obj = table[(handle & 0xFFF) * 16]` where `table = 0x2149C64` (16-byte slots, first dword = object*). `if obj == 0 || obj.handle (+0x44) != handle -> 0xFFFF`. `rec = obj[+0xB0]`; `if rec == 0 -> 0xFFFF`; else `movzx eax, word [rec+2]`.
- Writes: nothing.

### 8. `net_id_to_handle` (0x804FB0) with its lookup `net_remote_handle_map_find` (see 9)

- Input: AX = net id. Output: EAX = handle, 0 when none. Preserved: ESI (push/pop around `ESI = [0x212E258]`).
- Logic: `if ax == 0xFFFF return 0` (`cmp ax,0xffff; jne`). `node = map_find(map=[0x212E258] (in ESI), key = movzx ax, create = 0)`; `if node == 0 -> 0`; `rec = node[0]`; `if rec == 0 -> 0`; return `rec[+4]`.
- Writes: nothing.

### 9. `net_remote_handle_map_find` (0x808E30)

- Inputs: ESI = map, `[esp+4]` = key (i32), `[esp+8]` = create (u8), `ret 8`. Output: EAX = node or 0. Preserved: EBX, EBP, EDI (pushed), ESI.
- Hash (all 32-bit wrapping, `sar` = arithmetic shift): `a = key - (key << 15) - 1`; `b = (a sar 10) ^ a`; `b = b * 9`; `c = (b sar 6) ^ b`; `d = c + ~(c << 11)`; `h = (d sar 16) ^ d`. `h` is never negative (the upper bits cancel), so `idiv` by `[map+0x60]` (bucket count) equals an unsigned modulo; the remainder is the bucket.
- Walk: `node = buckets[bucket]` where `buckets = [map+0x5C]`; while `node != 0`: if `key == node[+0xC]` return node; `node = node[+4]`.
- Create (only when not found, `create != 0` and `[map+0x1C] > 0` signed `jle`): `node = alloc(EDI = map+0x20)` (0x809220); `node[+0xC] = key`, `node[+0x10] = bucket`, `node[+4] = old head`, `node[+8] = 0`, `old head[+8] = node` (if any), `buckets[bucket] = node`; return node. A miss without creation returns 0.
- Writes: only in the create path (node fields, the old head's prev, the bucket slot and whatever 0x809220 touches).

### 10. `net_handle_owned_locally` (0x8066C0) and `net_record_owned_locally` (0x806760)

- Handle form: ECX = handle -> AL. AL = 1 in every "unknown" case (handle 0, stale handle, no record, no session `[0x2528C14]`, no local connection `[session+0x408]`). Otherwise `owner = byte [rec]`; if `owner != byte [local+0x1B0]` -> AL = 0; else `AL = (([rec+0x67C] & 1) == 0)` (`test byte [..],1; je -> mov al,1`; the not-equal path falls to `xor al,al`).
- Record form: ECX = record* -> AL, same rule: null record, no session or no local conn -> 1; `[rec] != [local+0x1B0]` -> 0; else `([rec+0x67C] & 1) == 0`.
- Scratch: EAX, ECX, EDX only. Writes: nothing. This is the pure core of `human_owned_locally` (0x806720) and is what the 260 force-flag setters branch on.

### 11. `session_find_conn_by_id` (0x80FE80)

- Inputs: EAX = session, DL = id. Output: EAX = connection or 0. Scratch: ECX. Nothing else touched.
- Logic: `head = [session+0x400]`; if `head == 0` return 0; `node = head`; loop: if `byte [node+0x1B0] == dl` return node; `node = [node+0x22CC]`; stop (return 0) when `node == head` or `node == 0`.
- Writes: nothing.

### 12. `net_guid_equal` (0x8176E0)

- Inputs: ECX = a*, EDX = b* (16 bytes each). Output: AL = 1 if the 16 bytes are equal, else 0; `edx == 0` returns 0. Preserved: ESI, EDI (pushed).
- Logic: compare dwords in a loop (`sub eax,4` from 0x10, `jae` keeps looping while 4 bytes remain), then a byte tail; the sign of the first difference is computed but only "difference or not" is used (`test eax,eax; sete cl`). A plain `memcmp(a, b, 16) == 0`.
- Writes: nothing.

### 13. `net_packet_state_allows` (0x814B80)

- Inputs: EAX = record*, ECX = conn*. Output: AL.
- Logic: `mask = (u16) [rec+0x10]`; `if mask == 0xFFFF return 1`; `if conn == 0 return 0`; `bit = 1 << ([conn+0x3C] & 31)` (`shl edx, cl`); `AL = (mask as u32 & bit) != 0` (`movzx eax,ax; test eax,edx; setne al`).
- Scratch: EAX, ECX, EDX. Writes: nothing. Only the low 16 bits of the record mask are used, so state 16 can never be selected by the `0x1FF00` masks (they act as 0xFF00).

### 14. `net_packet_route_allows_send` (0x814B00) and `_recv` (0x814B40)

- Inputs: EAX = record*, ECX = connection*. Output: AL. Let `P = [[conn+0x1AC] + 0x404]` (read only when `conn != 0`), `r = [rec+0xC]`.
- Send: `r == 0` -> `AL = (conn != 0 && conn == P)` (`test ecx,ecx; je -> 0`; `cmp ecx,[edx+0x404]; je -> 1`); `r == 1` -> `AL = (conn == 0 || conn != P)`; any other `r` -> 1.
- Receive (mirror): `r == 0` -> `AL = (conn == 0 || conn != P)`; `r == 1` -> `AL = (conn != 0 && conn == P)`; other -> 1.
- Writes: nothing. These are the only gates between the packet table and the transport, so a port gets the packet routing for free by implementing exactly these two.

### 15. `net_conn_loss_ratio` (0x7F8300)

- Input: EAX = conn*. Output: ST0 = f32. Preserved: EBX, ESI, EDI (pushed / popped). Scratch: ECX, EDX.
- Logic: null conn -> 0.0. `n = [conn+0x15C0]` (u32); `now = [0x252A3CC]`; for each entry `e` of 16 bytes starting at `conn+0x1C4` (n entries): if `byte [e+5] == 0` { if `(e.u32 + 0x3E8) < now` (unsigned: the code skips on `jae`) { `total += 1`; if `byte [e+4] == 0` { `lost += 1` } } }. After the loop: if `total >= 0x14` (unsigned `jae`) result = `lost as f32 / total as f32`, else 0.0. Each count goes through `fild` of the signed 32-bit value with `+2^32` added when the value is negative (`fadd [0xE5BFD8]`, irrelevant for these small counts); the quotient is `fdivp` then stored to a dword and reloaded (**round to f32**), so the result is a single-precision value. `n == 0` and `total < 20` both give 0.0.
- Writes: nothing.

Other easy leaves not in the fifteen: `net_clock_ms` (0x7FCA90), `net_clock_reset` (0x7FC920), `net_timeout_clear` (0x7FC890), `session_state_in_1_6_8` (0x80CEF0), `session_state_in_0_1` (0x80CF20), `net_conn_recent_ids_contains` (0x7FAEF0), `net_object_owner_conn` / `net_handle_owner_conn` (0x807230 / 0x807250), `net_registration_find_field` (0x807680).

## Bugs and doc errors

- **Doc 25, "Packet header": "The header is 4 bytes when 0xE8BC14 is set and 3 otherwise".** `0xE8BC14` is `record(0x6B scrpt_pkt) + 0x14` (0xE8B6A8 + 57*24 + 0x14), i.e. the ordered-channel field of one packet type. Every sender tests the channel field of its **own** packet record: 0x20 tests 0xE8C21C, 0x28 tests 0xE8C2DC, 0x5B tests 0xE8BA94, join replies test 0xE8BF7C / 0xE8BF94 / 0xE8BFAC / 0xE8BFC4 / 0xE8BFDC. The doc generalised from the scrpt_pkt senders. A port must derive the header size from the record, not from one global.
- **Doc 41 (`vehicle_damage_events_raise` row): "net handle packing (0x804F70 / 0x814740)".** 0x814740 is not a handle packer; it is the send-to-every-remote-connection helper (`net_send_packet_to_remotes`, EDI = session, cdecl (buf, len)). 0x804F70 is the handle packer. Doc 05 (streamer) uses 0x814740 correctly as a sender.
- **Doc 25, "Sending and receiving".** It lists `net_send_packet_routed` and the state checks but not the routing predicates. The routing check is two functions (0x814B00 for send, 0x814B40 for receive, mirror images, new here), not one. Doc 25 says `net_send_packet` checks "its state filter (0x814B00)"; 0x814B00 is the **route** check, and `net_send_packet` calls only that one. `net_dispatch_packet` calls both 0x814B40 (route) and 0x814B80 (state mask); the send side never applies the state mask.
- **Doc 25, "Packet tables": "state mask (0x1FF00 in game, 0x1FFFF also in the lobby)".** Only the low 16 bits are used (`movzx eax,ax` after loading the word at +0x10), so the effective masks are 0xFF00 and 0xFFFF; bit 16 does nothing. Mask `0xFFFF` is special-cased as "always" (the early `cmp ax,0xFFFF`).
- **`net_handle_to_net_id` (0x804F70) leaves the upper half of EAX untouched for handle 0** (`or ax, 0xFFFF`). Callers use AX or `movzx`; a port that returns a `u32` would change callers that do `cmp eax, 0xFFFF` after this path (none were seen, but the contract is "AX only").
- **LTCG duplicates.** `net_bits_write_u32_dup` (0x7FD630), `net_bits_write_u8_dup` (0x7FD700), `net_bits_read_s8` / `_s16` / `_s32` / `_u8_dup` and three pool lookups (0x817D80, 0x817F40, 0x818020) are byte-identical (or nearly) copies. They are separate functions at the call sites, so each needs its own mapping when hooking.
- The net_payloads.csv extractor marks `rop_destroy` (0x0F) as having `sender_name` empty; the sender is `net_object_deactivate_send` (0x8053B0, called by `object::deactivate`).

## Open points

- **Which peer is `[session+0x404]`.** Both route predicates compare against it. From the packet table (`req_name` route 1 is a client request, `lbby_upd` route 0 is a host broadcast) route 0 behaves as "sent by the host" and route 1 as "sent to the host", which means the sender-side comparison cannot be against the host connection alone; the field may be "the connection that sent / owns the current operation". Needs a live trace of a join (see 40).
- The meaning of `[conn+0x3C]` (state 0..16, gate for the packet masks) versus `[conn+0x40]` (phase, broadcast by upd_state sub 1) is inferred from the writers; the state names are not recovered.
- The session state numbers (`[session+0x1C]`, 0..12) are only known as predicate sets (`session_state_in_*`); the join / lobby code in 0x80C000-0x80FA00 (about 30 large functions) is the place to name them.
- About 1,800 unnamed call targets remain in 0x7F0000-0x8D0000; most belong to the lobby / mode code (0x830000-0x8C0000) and the GameSpy / NAT layer (0x8CA000-0x8DF000), which I did not read.
