# 03 — C++ classes from RTTI

The full list is in `docs/rtti_classes.csv`. Regenerate it with:

```
python -I tools/rtti_dump.py bin/SR2_pc.unpacked.exe docs/rtti_classes.csv
```

## Method

The exe is MSVC 2005, and RTTI is on wherever a class has virtual functions, so every polymorphic class leaves a record:

- **TypeDescriptor** (`.data`): `{type_info vftable, spare, ".?AV<name>@@"}`. These give the class names.
- **CompleteObjectLocator** (`.rdata`): `{0, this-offset, cd-offset, TypeDescriptor*, ClassHierarchyDescriptor*}`.
- **Vtable:** the slot just before it (`vtable[-1]`) points to the locator. The virtual count is the number of consecutive `.text` pointers from there.
- **ClassHierarchyDescriptor / BaseClassDescriptors:** give the full list of base classes (`bases`, nearest first) and the multiple-inheritance flag.

Each column in the CSV comes straight from these structures (Observed). Only plain names are demangled (`a@b@@` → `b::a`); templates and local or anonymous-namespace names stay mangled. Classes with several vtables (multiple inheritance) list each one as `vtable@this-offset`.

## Totals

- 1,062 type descriptors, 1,139 object locators, 1,045 classes with a vtable.
- About 305 are Havok (`hkp*`, `hk*`, `havok*`), 199 are templates (mostly `std::allocator<…>`, containers and functors), 19 are `std::`, and about **517 belong to the game**.

## Game subsystems

| Area | Classes (examples) | Notes |
|---|---|---|
| **World objects** | `object` (9 virtuals) → `human` → `npc`, `player`; `vehicle`, `object_mover` (17), `object_prop`, `object_item`, `object_debris`, `projectile`, `effect_object`, `searchlight` | The core entity tree. `human`/`npc`/`player` add 3 virtuals to `object`. |
| **Characters / animation** | `character` → `human_character`, `distant_human_character`; `character_controller`, the `blocking_animation_compressor_*` classes | Visual character models, separate from the gameplay `human`. |
| **Memory** | `mempool_base` (17) → `static_mempool_base` → `virtual_mempool`, `page_mempool`, `split_page_mempool`, `compacting_mempool`, `tracked_mempool`, `async_mempool`, `async_virtual_mempool`, `string_pool`; `IAllocator` (10) | A small, self-contained family that everything else allocates through. The packfile allocators (`0xEC5E00`, `0xEC5E58`) are probably instances of these. |
| **Streaming** | `streamman_resource_loading_info_base` → `…chunk_geom`, `…chunk_peg`, `…chunk_extras`, `…static_mesh`, `…peg`, `…animation`, `…resource_character`, `…resource_vehicle`, `…resource_logo`; `stream_resource_post_load_data`, `vehicle_post_load_data`, `character_post_load_data` | The world streamer that sits on top of the file layer now in Rust. |
| **Renderer** | `rl_renderable_base_inst` → `rl_renderable_inst` → `rl_mesh_inst` (13), `rl_lod_mesh_inst`, `rl_morph_mesh_inst`, `rl_variant_mesh_inst`, `rl_animated_mesh_inst`, `rl_decal_inst`, `rl_container_inst`; `r_particle_inst`, `r_particle_ribbon_inst`, `r_tree_inst`, `r_dynamic_water_*`, `r_distant_*`, `reflection_data_*`, `CSpeedWind` | `rl_` is the render-layer base and `r_` the game-specific renderables. SpeedTree supplies the trees. |
| **D3D wrappers** | `IDirect3DDevice9Lost` (119) : `IDirect3DDevice9`; `gr_vertex_buffer` : `IDirect3DVertexBuffer9`; `gr_index_buffer` | The game implements the D3D9 COM interfaces itself, probably to survive device loss. Hooking or replacing these is a clean way into rendering. |
| **UI: vint** | `vint_object_base` (11) → `vint_element_base` (46) → `vint_element_text` (78), `…bitmap`, `…gradient`, `…clip`, `…group`, `…video`, `…sr2_map`, `…point`, `…bitmap_circle`; `vint_object_tween`, `vint_object_animation`, `vint_object_factory`, `vint_callback` → `vint_callback_lua` / `…_native`; `vint_resource_bitmap` | The Lua-driven UI (`.vint_doc`) whose logo we modded. |
| **UI: overlays and menus** | `ovl_base` (7) → 15 `ovl_*` screens (main menu, garage, buy/sell, hair salon, text entry…); `menu_widget` → `widget_*`, `slider_widget`…; `thin_control` → `…list_box`, `…slider`, `…text_button`…; `gsi_widget` → `gsi_meter`, `gsi_timer`…; `cash_widget`, `audio_player_widget` | `gsi` = the HUD's activity indicators. `thin_control` looks like the PC-only front end. |
| **Scripting** | `script_object` → `script_positionable_object` → `script_streamable_object` → `script_group_object` → `script_npc`, `script_vehicle`, `script_item`, `script_mover`, `script_trigger`, `script_navpoint`, `script_path`, `script_spawn_region`, `script_interior`… | Mission script entities. |
| **Activities / diversions** | `activity_info`/`_instance`/`_level_instance` for each activity (fraud, heli, escort, pimping, piracy, security, sewage, trafficking, vandalism, zombie, fight club, racing, demo derby, human torch…); `diversion_type_info` → about 40 diversions (stunts, taxi, ambulance, poker, blackjack…); `complication` → `crash_`, `drift_`, `near_miss_`… ; `fuzz_crime` (22) → 40 crimes (FUZZ) | Gameplay content, each activity a small class family. |
| **AI spawning** | `cell_spawn_group`, `notoriety_spawn_group_*`, `escort_*_spawn_group`, `skirmish_spawn_group`, `special_vehicle_spawn_group` | |
| **Multiplayer** | `multi_session_base` (21) → `pc_multi_session`, `stub_multi_session`; `multi_*_state`; about 40 `*_compressor`s (network delta compression of npc/vehicle/player state); `*_multi_registration`; `mp_bh_*` (custom match settings, per-activity host/participant) | `stub_multi_session` hints at an offline build path. |
| **GameSpy** | `gamespy_module` → `…login`, `…matchmaking`, `…presence`, `…sake`, `…voice`, `…atlas` | Defunct online service. |
| **Physics glue** | `game_havok_world` : `hkpWorld`; `GeneralCollisionListener`, `MyCharacterListener`, `MyAction`, `contact_collector`, `detour_hull_object`, `constraint_instance_breakable_listener` | The game's subclasses of Havok types. Havok itself is third-party (about 305 classes). |
| **Misc** | `string_hash_table`, `list_interface`, `fober_lock_free_queue`, `bitmap_writer`, `image_writer`, `mp3_playlist`, `inventory_item`, `parking_spot`, `stdext::exception`/`bad_alloc`, the `DName*` classes (the CRT's undecorator) | |

## What this means for the port

- **Third-party code is out of scope.** That means Havok (about 30% of the classes), SpeedTree, GameSpy and the CRT. A Rust build would link a replacement physics engine, or leave physics in the original code longest.
- **Good next subsystems:**
  1. **Memory pools**: small vtables (17–25 virtuals), no dependencies, used by everything, including the packfile allocators we already call. They can be verified with the same before/after logging.
  2. **The streamer** (`streamman_*`): the next layer above the Rust file system.
  3. **The vint UI**: well contained and directly visible. Its Lua bindings are the `vint_callback_lua` vtable.
  4. **The D3D wrappers**: a natural seam for taking over rendering later.
- **Vtables are the hook points.** Swapping one vtable entry replaces a single virtual method for every instance of a class, without patching code bytes. The addresses are in the CSV.
