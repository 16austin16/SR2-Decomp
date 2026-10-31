# 34: World objects, districts and global allocators

Names are in `docs/functions.csv` (notes ending "(34)"). Objects in general are in 12_objects.md, replicated objects in 08_multi_objects.md, and memory pools in 04_mempool.md.

## Global allocators

Most fixed-size game records live in global `allocator<T>` objects (RTTI `allocator<…>`, base `IAllocator`). Each has a static initialiser:
1. Fill in the vtable, a display name, a category, the record size and an `InitializeCriticalSection`.
2. Call **`allocator_register`** (0x7F6A10). It appends the allocator to the global list `0x212A800` (count `0x2528B40`, at most 96) and finds its category in the category table `0x212A680` (count `0x2528B44`).

The initialisers are named `allocator_init_<name>` (85), for example:
- `allocator_init_item_state_data` (and its `_deltacomp_gb` / `_coop_mp` variants);
- `allocator_init_mover_sim_state`;
- `allocator_init_npc_creation_data`;
- `allocator_init_explosion_event_data`;
- `allocator_init_havok_triggers`.

The categories are `multiplayer`, `competitive-mp`, `coop-mp`, `deltacomp_sa`, `deltacomp_gb`, `deltacomp_global` and `always_allocated`. The `deltacomp_*` ones are the delta-compressed state that replicated objects send (`*_state_data`, `*_sim_state`). The `allocator<>::num_free` / `num_used` / `total_size` virtual slots, named earlier, are the per-allocator statistics.

## Districts, hoods and pushback

- `districts_load_table` / `districts_parse_warp_navpoints` read `districts.xtbl` (names, display names, warp navpoints).
- `cmd_current_hood_info` prints the current hood and its owner. `cmd_gang_notoriety_print` prints the gang notoriety levels.
- **Pushback** is gangs contesting the player's hoods:
  - `pushback_general_load_table` reads `pushback_general.xtbl` (`Contested_Delay_Min` / `_Max`);
  - `pushback_start_contest` picks a hood or refuses ("No hoods are eligible for contesting");
  - `pushback_contested_hood_hud` shows the HUD.

## Movers, level objects and items

- `level_objects_load_table` / `level_objects_parse_record` read `level_objects.xtbl` (the per-object defaults). The multiplayer overrides are `level_objects_mp_overrides.xtbl`.
- `mover_list_load` (0x8B4780) loads a mover list file. `mover_convert_to_prop` (0x9340C0) turns a mover into a wieldable prop. `movers_dump_report` writes `d:\mover_spew.txt` (debug).
- `item_collected_message` (0x92F910) handles collectibles (CDs, partner pickups).
- `human_door_kick` (0x980720) plays the door kick animation.
- `train_ride_prompt` (0x4BB250) is the "ride the train" prompt.

## Vehicles in the world

- `vehicle_despawn_load_tables` / `vehicle_despawn_parse` read `vehicle_despawn.xtbl` / `_mp.xtbl`: in-view and out-of-view despawn and the abandon delay.
- `mp_vehicle_respawn_collect_abandoned` / `mp_vehicle_respawn_update` handle multiplayer vehicle respawn.

## Stores

`store_chains_parse` (0x9F2BF0) reads store chains: ownership trigger and interior elements. `shop_purchase_prompt` (0x9F5ED0) shows the "need hood ownership" and purchase prompts.

## Open

- The trigger system (`trigger_state_data`, mission and activity start triggers) beyond what 22_missions.md lists.
- Door and elevator objects.
