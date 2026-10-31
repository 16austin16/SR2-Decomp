# 31: Traffic and vehicle AI

Names are in `docs/functions.csv` (subsystems `ai` and `vehicle`, notes ending "(31)"). The overview of AI is in 21_ai.md, vehicles in 16_vehicles.md, and the `veh_pkt` sub-commands in 25_network.md.

## Force flags

Scripts change AI behaviour by flag name, through the `ai_force_flags*` / `vai_force_flags*` bindings. Each flag has its own small setter. The setter finds the record (`human_ai_data` or `vehicle_ai`) and sets or clears one bit. They are named `human_ai_set_force_flag_<flag>` (46) and `vehicle_ai_set_force_flag_<flag>` (27, 0xB2B8A0-0xB2DA30).

The vehicle flags are:

| Group | Flags |
|---|---|
| Driving style | `override_speed`, `drunk_driver`, `sunday_driver`, `crazy_rails`, `use_short_cuts`, `no_signals`, `obey_traffic_lights`, `lane_type_preference` |
| Reactions | `no_flee`, `always_flee`, `no_chase`, `dont_scare_peds`, `allow_ram_ped`, `upset_smoke`, `upset_fire` |
| Goals | `script_hard_goto`, `goal_stop_override`, `leashed`, `never_go_ambient`, `degrade_info`, `spawned_vehicle_despawning` |
| Passengers | `force_no_passenger_exit`, `no_stop_enter_with_driver`, `unjackable_by_ai` |
| Other | `ignore_repulsors`, `ignore_rail_obstacles`, `use_lockon_system` |

The human flags include `always_run`, `cant_flee`, `deaf`, `driver`, `kamikaze`, `sidewalking`, `wandering`, `sidewalk_leashed` and `always_follow_in_alternate`. 0x849A10 / 0x84A750 send both flag sets to co-op partners.

## Vehicle orders (0xB30000-0xB62000)

This block has no strings. It holds the vehicle AI's orders: go to, follow, chase, flee, escort and so on. Each one is used by activities, diversions and the notoriety spawn groups. For example:
- `notoriety_spawn_group::vf16` and `special_vehicle_spawn_group::vf16` call 0xB345B0 / 0xB398B0;
- the heli and convoy objectives call 0xB487A0 / 0xB42D20;
- road rage calls 0xB5C2E0.

Most orders replicate themselves to the co-op partner with a `veh_pkt` sub-command. The senders are named in 27_packet_senders.md (`veh_pkt_send_00`, `_08`, `_09`, `_0d`, …), and the order bodies call them. A port should name these once the `veh_pkt` sub-commands are decoded. The receiving side has the same numbers (`veh_pkt_sub_*`).

## Road network and vehicle data

- `traffic_spline_validate` (0xB318F0): checks that each road spline links to a next and a previous spline. Traffic follows splines from the chunks' road data plus `traffic_lanes.xtbl` (21_ai.md).
- `vehicle_info_parse_record` (0xAEB850): the vehicle table parser that 00_initial_overview.md flagged. It works with `vehicle_info_parse_physics` (0xAE6560: hit points, centre of mass, friction), `vehicle_anim_modifiers_parse` (0xAEDBD0) and `vehicle_type_from_name` (0xAE8870: Automobile, Motorcycle, Airplane, …).

## Open

- The meaning of each vehicle order, which needs the `veh_pkt` payloads or an in-game trace.
- `vehicle_ai_drive` (0xB2DD90) and the spline follower.
