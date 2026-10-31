# 25: Network packets (co-op and multiplayer)

Names are in `docs/functions.csv` (subsystem `net`). Replicated objects are in 08_multi_objects.md and GameSpy in 24_gamespy.md.

Every packet is described by a 24-byte record in one of two tables. The records carry debug names, so every receive handler has a meaningful name.

## Packet tables

| Types | Table | Contents |
|---|---|---|
| 0x00-0x31 | `0xE8BF08` (record 0 = `ping`) | engine packets: ping / pong, join, connections, object replication (`rop_*`), migration, QoS, voice, NAT, logging |
| 0x32-0x82 | `0xE8B6A8` | game packets: lobby, respawn, teams, audio, scores, vehicles, movers, stats, weather, matchmaking, missions, doors, streaming, cutscenes, melee, chat |

A record is:

| Off | Field |
|---|---|
| +0x00 | packet type |
| +0x04 | receive handler `(payload, ...)`; payload = packet + 3 |
| +0x08 | debug name (for example `"player_respawn"`) |
| +0x0C | routing class (0, 1 or 2), checked by `net_send_packet_routed`; 1 involves the host connection |
| +0x10 | state mask (`0x1FF00` in game, `0x1FFFF` also in the lobby) |
| +0x14 | ordered channel. When non-zero, byte 3 of the packet is a sequence number and delivery waits for order. |

Each receive handler is named `net_recv_<name>`, for example `net_recv_rop_create` (0x805510) and `net_recv_player_respawn` (0x82B6C0).

## Packet header

| Byte | Field |
|---|---|
| 0 | type |
| 1-2 | total length (u16) |
| 3 | sequence byte (only on ordered channels) |
| 3 or 4 | payload |

The header is 4 bytes when `0xE8BC14` is set and 3 otherwise; the senders check that flag.

## Sending and receiving

- **`net_send_packet`** (0x814670; ESI = connection; (buf, len < 0x1FF, …)):
  1. Find the record and check its state filter (0x814B00).
  2. Stamp the channel's sequence byte from `conn+0x16D4[channel]`.
  3. Hand the packet to the transport (0x7FA400).
- **`net_send_packet_routed`** (0x8145B0) applies the record's routing rule.
- **`net_broadcast_packet`** (0x814620) sends to every connection in the session list (`+0x400`, linked through `+0x22CC`).
- **`net_dispatch_packet`** (0x8138F0) is the receive side:
  1. Look up the record and check the state masks (0x814B40 / 0x814B80).
  2. Call the handler, or queue the packet on its ordered channel (`net_queue_ordered_packet`, 0x813470).

## Sub-command packets

Six game packets carry a second byte that picks a sub-command. Each one has a small dispatcher that jumps through its own table (`net_dispatch_*`, 0x8CA2A0-0x8CA360):

| Type | Name | Sub-commands | Table |
|---|---|---|---|
| 0x67 | `hud_pkt` | 25 | 0xE8A718 |
| 0x68 / 0x69 | `human_pkt` / `human_pkt_in_ord` (same table; 0x69 is the ordered one) | 81 | 0xE8A7A8 |
| 0x6A | `veh_pkt` | 32 | 0xE8A8F8 |
| 0x6B | `scrpt_pkt` | 43 | 0xE8A978 |
| 0x6C | `act_packet` (activities and missions) | 92 | 0xE8AA28 |
| 0x6D | `misc_packet` | 34 | 0xE8AB98 |

The sub-command handlers are named `<packet>_sub_<nn>`. When a handler's only distinctive call is to an already-named function, the name uses that function instead, for example:
- `human_pkt_human_take_damage` (0x9844D0);
- `human_pkt_object_teleport` (0x9A0940);
- `act_packet_mission_set_checkpoint` (0x6A8E40).

Earlier docs cite some of these numbers:
- damage is `human_pkt` (0x68);
- the co-op checkpoint is `act_packet` / 2;
- multiplayer mission end is `misc_packet` / 0x1F.

**Script replication (`scrpt_pkt`, 0x6B).** The Lua bindings that change the world in co-op use a wrapper in 0xA20000-0xA70000:
- If this machine owns the object (`[obj+0xB0]` matches the local player and isn't remote), the wrapper does the work locally.
- Otherwise it builds `scrpt_pkt` with a sub-command, the object's network handle and the payload, and sends it with `net_send_packet`.

Each wrapper is usually followed directly by its receive handler. For example, the sender at 0xA2B780 is followed by `scrpt_pkt_sub_25` at 0xA2B930. The receive handler maps the handle back through the remote-handle hash at `0x212E258`.

## Sub-commands in detail

`docs/net_subcommands.csv` lists all 307 sub-commands, one row each, with:
- the receiver and its name;
- every sender found (a function that writes the packet type and that sub-command into a header and calls a send function, any size);
- the named functions that call those senders, up to two levels up: this says *which game feature uses the sub-command*;
- the receiver's named callees and strings.

A sub-command is renamed from `<packet>_sub_<nn>` only when the evidence says what it carries. Its sender gets the matching `<packet>_send_<meaning>`:

| Packet | Sub | Receiver | Evidence |
|---|---|---|---|
| human_pkt | 0x06 | `human_pkt_revive` 0x9B0030 | the sender's local branch clears invulnerability and restores max hit points; payload {handle, u8} |
| human_pkt | 0x0E | `human_pkt_human_take_damage` 0x9844D0 | calls `human_take_damage` |
| human_pkt | 0x12 | `human_pkt_deactivate` 0x97E920 | senders called by `human::deactivate` |
| human_pkt | 0x16 | `human_pkt_reset` 0x4181D0 | sender called by `objects_reset` |
| human_pkt | 0x1B / 0x1C | `human_pkt_object_teleport` / `_player_teleport` | call `object_teleport` / `player_teleport` |
| human_pkt | 0x47 | `human_pkt_bullet_hit` 0x95F660 | sender called by `human_bullet_hit` |
| veh_pkt | 0x07 | `veh_pkt_vehicle_ai_flags` 0xB4E040 | calls the `vehicle_ai_set_force_flag_*` setters |
| act_packet | 0x00 | `act_packet_mission_failed` 0x6A3CC0 | `MISSION_FAILURE_PARTNER_*`; sender called by `mission_end` |
| act_packet | 0x01 | `act_packet_mission_start` 0x6A3DB0 | "mission start" |
| act_packet | 0x02 | `act_packet_mission_set_checkpoint` 0x6A8E40 | calls `mission_set_checkpoint` |
| act_packet | 0x04 / 0x09 | `act_packet_level_complete` / `_2` | "Level Complete" |
| act_packet | 0x06 | `act_packet_coop_gsi_score` 0x6AA030 | co-op score display |
| act_packet | 0x1C | `act_packet_rebroadcast` 0x5E7F70 | forwards the packet with `net_broadcast_packet` |
| act_packet | 0x30 | `act_packet_vandal_start` 0x66D480 | `ACT_VANDAL_START_*` |
| act_packet | 0x42 | `act_packet_fuzz_objective` 0x5C2650 | fuzz objective text |
| act_packet | 0x47 | `act_packet_diversion_progress` 0x53C380 | sender called by the stunt, windshield cannon and vehicle surfing diversions |
| misc_packet | 0x00 | `misc_packet_confessional_forgive` 0x7A94A0 | confessional |
| misc_packet | 0x01 | `misc_packet_tagging_remote_tag` 0x6210B0 | partner tagged a spot |
| misc_packet | 0x02 | `misc_packet_partner_collected_item` 0x92FC00 | `OBJECT_PARTNER_COLLECTED` |
| misc_packet | 0x08 | `misc_packet_saints_hq_upgrade` 0x545530 | calls `saints_hq_set_upgrade_level` |
| misc_packet | 0x16 | `misc_packet_coop_join_cheat` 0x6885A0 | `COOP_JOIN_CHEAT` |
| misc_packet | 0x1A / 0x1B / 0x1C | `misc_packet_coop_mission_start_prompt` / `_mission_partner_reply` / `_coop_mission_confirm` | co-op mission start handshake: prompt, busy / declined, confirm |
| misc_packet | 0x1D / 0x1E | `misc_packet_zombie_invite` / `_zombie_partner_reply` | Zombie Uprising co-op invite |
| misc_packet | 0x1F | `misc_packet_script_thread_create` 0x6A2FF0 | mission end / replay prompt, starts a script thread |
| misc_packet | 0x20 | `misc_packet_mission_meta_game` 0x6A90E0 | "Mission Meta Game is %s" |

**What the rest are used for**, from their senders' callers:
- **`human_pkt` 0x1E-0x41**: human actions done by the crime events (`fuzz_crime_*`: purse snatch, littering, public indecency, arsonist, polygamy, …), activities (security, hitman, holdup customers, taxi) and diversions (mugging, driveby). Most senders are large action functions in 0x401000-0x470000, each next to its receiver.
- **`human_pkt` 0x00-0x05, 0x43-0x4D**: followers, cell-phone spawns and activity NPCs (`followers_update`, `cell_spawn_group`, the activity level instances).
- **`veh_pkt`**: the vehicle AI orders (31_traffic_vehicle_ai.md) from the convoy / heli objectives, road rage, chop shop, pimping and the notoriety spawn groups. 0x08 / 0x09 / 0x1D are a related set.
- **`scrpt_pkt`**: the Lua binding wrappers in 0xA20000-0xA70000. Several receivers there are large blocks shared by many bindings, so their callee lists in the CSV are only hints.

## Payloads

`docs/net_payloads.csv` gives the payload of every sender found (261), field by field after the header (and after the sub-command byte for sub-command packets). The `payload` column of `docs/net_subcommands.csv` repeats it per sub-command.

**How the layouts were read.** Every sender builds its packet the same way in a stack buffer:
1. `buf[0]` = type, then a cursor register = 3 or 4 (`0xE8BC14` and the per-packet ordered flag decide whether a sequence byte is reserved).
2. `buf[cursor]` = the sub-command (sub-command packets only), then `cursor += 1`.
3. Each field: `mov size [esp + cursor + base], value`, then `cursor += size`.
4. `*(u16 *)&buf[1] = cursor - 3`, and the send.

The extractor follows the cursor and records each field's size.

| Notation | Meaning |
|---|---|
| `u8` / `u16` / `u32` | integer of that size |
| `f32` | float (`fstp`) |
| `handle` | 16-bit **network handle**: the object's handle `+0x44` looked up in the handle table `0x2149C64`, then the multiplayer record `[obj+0xB0]+2`, or `0xFFFF` when the object has none |
| `u8=N` | a constant |
| `a\|b` | branches write different sizes there |
| `blob` | a `memcpy` / string copy |

Floats stored through integer registers show as `u32`, so a run of 3 or 12 `u32`s after a handle is usually a position or a position plus a 3×3 orientation (the teleports, `human_pkt` 0x1B / 0x1C).

**Properties: `memb_pkt` (0x6E).** It carries `{u32 hash(class name), u32 hash(member name), handle, value}` (hash 0xBDCA30). The receiver `net_recv_memb_pkt` sets the member on the object. The senders, named `net_send_prop_<class>_<member>`:

| Class | Members |
|---|---|
| `human` | `hit_points`, `max_hit_points`, `adjust_hp`, `cash_adjust`, `override_emotion`, `anim_state_override`, `priority_target_params` |
| `human_ai_data` | `leash_params`, `wander_params`, `sidewalk_leash_params`, `worried_walk` |
| `player` | `player_last_vehicle_handle`, `script_mode_params` |
| `vehicle` | `hit_points`, `max_hit_points`, `adjustment`, `m_force_flagsengine_disabled` |

The force-flag setters (31_traffic_vehicle_ai.md, 32_animation.md, 33_effects_weather.md) use the same `class` + `member` string pairs: `human` / `human_force_flags*`, `vehicle` / `m_force_flags*`, `vehicle_ai` / `vai_force_flags*`. They send the same property packet when this machine doesn't own the object. `human_set_hit_points` and friends (20_weapons.md) are the `human/hit_points` senders.

**Other patterns:**
- Most `human_pkt` senders whose sub-command comes from a register (no fixed sub in the CSV) send `{handle actor, handle target, u8=0}`: the crime events and activity humans (`fuzz_crime_*`) ordering an NPC against a target.
- `veh_pkt` orders mostly start with the vehicle's handle, then a target handle or a position.
- `braveheart_packet` (Strong Arm) starts with an activity id byte (`u8=6` / `u8=7` …) followed by a per-activity message.

## Open

- The ~270 sub-commands still named by number. Their payloads are now known; naming them needs each receiver's use of the fields.
- Payloads of receivers whose senders weren't found (the header pattern misses senders that build the buffer in a caller). The CSV lists who uses each; the payload layouts would name the rest.
- Who sends each packet type, beyond the senders found by the header pattern.
- The transport below `net_send_packet` (0x7FA400 / 0x7FA390), and the GameSpy gt2 / NAT layer under it.
