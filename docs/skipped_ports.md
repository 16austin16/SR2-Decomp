# Skipped ports

Functions a porting agent looked at and left in the original code, with the reason. Remove a row
when the function is ported (its docs/functions.csv status becomes `rust`).

| Address | Name | Skipped in | Reason | What it needs |
|---|---|---|---|---|
| 0x00b908c0 | luafn_vint_insert_values_in_string | doc 43 | vint values on the stack, temp allocator marks, unbounded copy into a 32-entry stack table | the vint value type |
| 0x00b8eca0 | luafn_vint_dataitem_get | doc 43 | 32-value C++ array on the stack (vector constructor iterator) | the vint value type |
| 0x00a39990 | luafn_mission_end_success | doc 43 | portable, but writes mission globals | a play test that reaches a real mission end |
| 0x004586b0 | ai_order_k23_exit | port_ai (round 3) | 0x6AC360's result drives the next call | call the callees by address with a snapshot or no shadow |
| 0x00460a00 | ai_order_k1d_exit | port_ai (round 3) | 0x6AE180 would see the ECX/EDX that gaze_clear clobbers | call the callees by address with a snapshot or no shadow |
| 0x0041c380 | ai_order_k0a_exit | port_ai (round 3) | callees pop their own arguments; a recording stub can't ret N | call the callees by address with a snapshot or no shadow |
| 0x004165c0 | ai_order_k08_exit | port_ai (round 3) | callees pop their own arguments; a recording stub can't ret N | call the callees by address with a snapshot or no shadow |
| 0x0045c070 | ai_order_k1a_exit | port_ai (round 3) | stack-local object via 0x9695F0 | call the callees by address with a snapshot or no shadow |
| 0x00462a80 | ai_order_k20_net_read | port_ai (round 3) | uses vehicle_interaction_rider_find's result | call the callees by address with a snapshot or no shadow |
| 0x00415ec0 | ai_order_k08_net_read | port_ai (round 3) | 0x415DA0 creates the record read next | call the callees by address with a snapshot or no shadow |
| 0x004593a0 | ai_order_k16_update | port_ai (round 3) | 0x963540's result is used | call the callees by address with a snapshot or no shadow |
| 0x0045c0c0 | ai_order_k1a_update | port_ai (round 3) | large; not read | call the callees by address with a snapshot or no shadow |
| 0x00459800 | ai_order_k17_enter | port_ai (round 3) | event dispatch tail; not read fully | call the callees by address with a snapshot or no shadow |
| 0x0045c020 | ai_order_k1a_enter | port_ai (round 3) | not read fully | call the callees by address with a snapshot or no shadow |
| 0x004375b0 | ai_order_set_action | port_ai (round 3) | 294 callers; control flow depends on 0x806720, a virtual call and 0x735FB0, and thiscall callees pop their arguments, so its writes can't be bounded for a shadow | a snapshot design for its writes, or a careful unshadowed port |
| 0x00c03af0 | spline_eval_tangent | port_nav (round 3) | long mixed SSE / x87 code with comiss branches, not fully read | a full read |
| 0x00c03dc0 | spline_eval_second_derivative | port_nav (round 3) | as spline_eval_tangent | a full read |
| 0x00a0aed0 | nav_detour_release_hull | port_nav (round 3) | unbounded writes into hull rings | port without a shadow, or bound the rings |
