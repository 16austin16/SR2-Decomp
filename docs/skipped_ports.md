# Skipped ports

Functions a porting agent looked at and left in the original code, with the reason. Remove a row
when the function is ported (its docs/functions.csv status becomes `rust`).

| Address | Name | Skipped in | Reason | What it needs |
|---|---|---|---|---|
| 0x00b908c0 | luafn_vint_insert_values_in_string | doc 43 | vint values on the stack, temp allocator marks, unbounded copy into a 32-entry stack table | the vint value type |
| 0x00b8eca0 | luafn_vint_dataitem_get | doc 43 | 32-value C++ array on the stack (vector constructor iterator) | the vint value type |
| 0x00a39990 | luafn_mission_end_success | doc 43 | portable, but writes mission globals | a play test that reaches a real mission end |
| 0x00c03af0 | spline_eval_tangent | port_nav (round 3) | long mixed SSE / x87 code with comiss branches, not fully read | a full read |
| 0x00c03dc0 | spline_eval_second_derivative | port_nav (round 3) | as spline_eval_tangent | a full read |
| 0x00a0aed0 | nav_detour_release_hull | port_nav (round 3) | unbounded writes into hull rings | port without a shadow, or bound the rings |
| 0x00817d80 | (pool lookup, doc 48) | port_net (round 4) | not reached in the time available; one of three pool lookups that differ only in their globals | port with 0x817F40 / 0x818020 from one body |
| 0x00817f40 | (pool lookup, doc 48) | port_net (round 4) | as 0x817D80 | as 0x817D80 |
| 0x00818020 | (pool lookup, doc 48) | port_net (round 4) | as 0x817D80 | as 0x817D80 |
