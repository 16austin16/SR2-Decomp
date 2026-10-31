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

## Open

- What each sub-command does (most are named only by number).
- Who sends each packet type. The senders build the header inline, so they have to be found by the type byte.
- The transport below `net_send_packet` (0x7FA400 / 0x7FA390), and the GameSpy gt2 / NAT layer under it.
