# 27: Packet senders

Names are in `docs/functions.csv` (subsystem `net`, notes ending "(27)"). The packet tables and receive handlers are in 25_network.md.

Senders don't share a "begin packet" helper. Each one builds the header inline in a stack buffer (often 0x520 bytes):
- `mov byte [esp+o], type`: the type byte;
- `mov word [esp+o+1], len`: the total length;
- for sub-command packets, `mov byte [esp+reg+o], sub` at offset 3 or 4. That depends on `0xE8BC14` (the ordered channels' sequence byte, 25_network.md).

## Rule

A function is named as a sender when it:
1. calls `net_send_packet`, `net_send_packet_routed` or `net_broadcast_packet` directly;
2. writes exactly one known packet type into a header (a type byte followed by a length word, or a type byte followed by a sub-command byte);
3. is smaller than 0x400 bytes, so it is really a sender and not a gameplay function that happens to send one packet.

The names are:
- `<packet>_send_<sub>` when the sub-command is a constant, for example `scrpt_pkt_send_25` (0xA2B780). It is followed directly by its receiver `scrpt_pkt_sub_25`.
- `net_send_<packet name>` otherwise, for example `net_send_player_respawn`, `net_send_door_event` or `net_send_global_sync`. Repeats get `_2`, `_3`.

This gives 133 senders:
- 525 functions call a send function directly.
- 270 of them build a recognisable header.
- 261 of those send a single type.

The ones left out are either named already or larger than the size limit.

## What the larger ones are

About 70 larger functions send `human_pkt` sub-commands 0x16-0x4B. Most are in 0x401000-0x470000, the human action and state code, next to their receive handlers (`human_pkt_sub_*`, which call `object_teleport_prepare`). Each one does its gameplay work locally and replicates it with one sub-command. They keep their unnamed status for now; their sub-command numbers are in 25_network.md.

## Open

- What each sub-command carries, from the payload writes after the header.
- Senders that build the buffer in a caller and pass it in. The rule only sees headers written in the same function as the send.
