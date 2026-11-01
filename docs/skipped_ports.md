# Skipped ports

Functions a porting agent looked at and left in the original code, with the reason. Remove a row
when the function is ported (its docs/functions.csv status becomes `rust`).

| Address | Name | Skipped in | Reason | What it needs |
|---|---|---|---|---|
| 0x009976e0 | human_state5_update | port_humans2, port_human_states | builds an audio params block on the stack; 1,573 bytes | port the params block layout (doc 15 has the steps) |
| 0x009989e0 | human_state6_update | port_humans2, port_human_states | builds ray-query objects on the stack (ctor 0x418540, dtor 0x4080B0) | ray-query object layout, then call ctor / dtor by address |
| 0x0099a2d0 | human_state7_update | port_humans2, port_human_states | same ray-query objects as state 6 | as state 6 |
| 0x00a049f0 | nav_cell_walkable | port_nav (run 1) | calls 0xA05460, not a leaf | call 0xA05460 by address, or port it first |
| 0x00a05250 | nav_cell_to_node_ref | port_nav (run 1) | calls nav_cell_locate 0xA02C10, not a leaf | call 0xA02C10 by address |
| 0x00a05460 | nav_cell_has_link_flag2 | port_nav (run 1) | loop whose leftover upper EAX depends on the last iteration | reproduce the per-iteration EAX |
| 0x00b274f0 | vehicle_ai_set_speed_override | port_nav (run 2) | calls 0xB2B8A0 | call 0xB2B8A0 by address; snapshot its writes |
| 0x00b27530 | vehicle_ai_clear_speed_override | port_nav (run 2) | calls 0xB2B8A0 | as above |
| 0x00a1f730 | nav_route_clear | port_nav (run 2) | calls nav_route_list_free 0xA18500 | call 0xA18500 by address |
| 0x00b908c0 | luafn_vint_insert_values_in_string | doc 43 | vint values on the stack, temp allocator marks, unbounded copy into a 32-entry stack table | the vint value type |
| 0x00b8eca0 | luafn_vint_dataitem_get | doc 43 | 32-value C++ array on the stack (vector constructor iterator) | the vint value type |
| 0x00a39990 | luafn_mission_end_success | doc 43 | portable, but writes mission globals | a play test that reaches a real mission end |
