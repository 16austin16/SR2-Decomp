# 06: vint, the Lua-driven UI

vint is Volition's UI runtime. A UI screen is a **document** (`.vint_doc`): an XML tree of **elements** (bitmaps, text, groups, …) and **animations** made of **tweens**, plus a Lua script that drives it. The game ships the documents in packages listed in `vint_file_packages.xtbl` and names them in the project `sr2.vint_proj`.

Names below are in `docs/functions.csv` (subsystem `vint`) and the method names in `docs/vtable_slots.csv`. Status: mapped from the code, nothing ported yet.

## Where it runs

| Function | Address | When |
|---|---|---|
| `interface_init` | 0x7F3010 | once, from game init 0x51F600 |
| `interface_do_frame` | 0x7F3500 | per frame: from `sim_frame` in gameplay, and from the menu game states |
| `interface_render` | 0x7F33B0 | per frame from the menu game states (main menu, multiplayer menus, loading) |
| `hud_render` | 0x75A230 | gameplay: `gameplay_do_frame` → `gameplay_ui_frame` (0x68D2F0) → `hud_render`, which renders the active overlays and then tail-jumps (`jmp`, not `call`) to `interface_render`. The HUD is an ordinary vint document drawn there |

The path from the frame is:
- `game_frame_tick` (0x520450) → `game_state_frame` (0x697780) → `game_state_dispatch` (0x697490).
- The dispatch calls `overlays_update`, then a jump table over 11 game states: main menu 0x73DCB0, `gameplay_do_frame`, 0x830AB0, multiplayer menus 0x834D40, 0x82F570, 0x73B7F0, 0x688FE0. It ends with `overlays_render`.
- The menu states call `interface_do_frame` and `interface_render` themselves. Gameplay updates through `sim_frame`.

**interface_init:**
1. Fills four platform callback tables (function pointers the vint core calls back into):

   | Table | Global | Use |
   |---|---|---|
   | `0x1FD4FEC` | `0x252A1D4` | render |
   | `0x1FD52B0` | `0x252A1D8` | draw: [0] begin document, [+0x18] per element, [+8] mask end |
   | `0x1FD52E8` | `0x252A1DC` | |
   | `0x1FD509C` | `0x252A1E0` | input / text: 0x7EDC00 virtual keyboard, 0x7ED840 |

2. Creates the string pool "Vint" (`0x2528B10`).
3. `vint_init` (0xB91EC0) registers the types and loads the project.
4. `vint_lua_init` (0xB914F0).
5. Screen-state table `0x252A238`, `interface_register_lua_functions` (0x7EFA10, the game-side bindings).
6. Document-unload hook `0x252A1EC` = 0x7543E0.
7. Localised image tags (0x7F1FB0 / 0x7F2040, `%s_TXT` / `%s_IMG`).
8. `vint_file_packages_load` (0x7EC4E0), then sets the ready flag `0x2528AFF`.

**interface_do_frame:**
1. Return if not ready.
2. Refresh the image tags when the player's in-vehicle state flips (`0x347AF34`).
3. Input: 0x7EC170, 0x7F3450.
4. 0xB95290, 0xB94560, 0xB9EA70.
5. `vint_update_documents(*0xE87998)`, the frame time.

**interface_render:**
1. Run the screen-state callback `0xE8CF94 + state*0x10` (state `0xE8CF80` < 0x15).
2. Reset the render params (`0x2A14B28`, `0xEB15BC` = white).
3. `0xD148D0` / `0xD14840`, then `vint_render_documents`.

## Documents

Records live in a fixed table at `0x319F6B0`: 12 documents, 0x57C bytes each. The loaded ones are on a circular list headed by `0x252A1AC` (link at +0). A document **handle** is index | generation << 16, and it is valid while doc+0x4C equals the handle.

| Off | Document field |
|---|---|
| +0x00 | list link |
| +0x28 | name (the `<name>_init` Lua function is formed from it) |
| +0x48 | name hash (`0xBDC9B0`) |
| +0x4C | handle |
| +0x458 | root element |
| +0x45C | float time |
| +0x460 | u8 paused |
| +0x572 | u8 unload pending (set when the reference count drops below 1) |
| +0x578 | reference count |

**Loading** happens in `interface_document_load` (0x7F3720), the target of `vint_document_load(name)`:
1. Find the package for the name in `vint_file_packages` (`0x2528B1C`, count `0x2528B20`, 0xC bytes {name, entries, count}). Collect it and its dependencies, up to 64.
2. Every entry is {file, kind}:
   - **kind 0, document:** if it is already loaded, only increment the reference count. Otherwise resolve the file through the project table `0x22EF628` (0x88 bytes each, count `0x252A244`), then `file_resolve` and allocate.
   - **kind 1, peg:** `.peg_pc` + `.g_peg_pc` via 0x7398A0.
   - **kind 2, Lua script:** resolve and allocate size+1.
3. Read all files synchronously (0xA7D890).
4. In three passes (documents, pegs, scripts), build them:
   - document: `vint_document_alloc` (0xB8AD80), then `vint_document_create(data, allocator)` (0xB8B220), which parses the XML;
   - peg: 0x7395B0;
   - script: NUL-terminate and run with Lua's dobuffer (0xCDE2E0).
5. For each new document, look up the Lua function `<name>_init` (0xCE0120). If found, create a Lua thread (0xCE01E0), set thread+0x14 = document handle, and run it (0xCE0570) unless flag 0x40 is set.
6. Drop the load reference and add a reference to every `doc_dependency`.

The .vint_doc XML tags: `vint_doc_type`, `vint_document`, `vint_doc_version`, `document_depth`, `doc_dependency`, `root_animation`, `safe_frame`, `lua_script_file`, `elements`, `animations`, `object` / `name` / `property`, `critical_resources` / `pegs` / `filename`, `metadata` / `metadata_item`. Property values are parsed by 0xB92F50: "true", `%f,%f,%f`, `%f,%f`.

**Per frame:** `vint_update_documents(dt)` (0xB8BC10):
1. Clamp dt to 0.5.
2. For each document: unless paused, time += dt. Then `vint_object_tree_update` on its two roots, which calls vtable slot 3 `update` on every object. Every vint `update` slot is an empty `ret` in this build (see "The non-render side").
3. Then destroy the documents with unload pending: 0xB91400, the hook `0x252A1EC`, 0xB8A9F0, 0xB8ABD0.

**Rendering:** `vint_render_documents` (0xB8BD40):
1. Sort the documents by depth (0xB8A8B0).
2. For each, `vint_document_render`:
   1. Platform begin callback.
   2. `vint_element_render_tree` (0xB95B10) on the root element, with a default state: white tint, alpha 1, scale 1.
   3. The tree walk for each element:
      - skip it if it is invisible (+0x59) or the alpha is about 0;
      - `pre_render`, combine the state with the element's transform (0xB957B0), the per-element platform callback, then `render`;
      - collect the element children, sort them by depth (0xB95AD0), and handle masks (+0x58) with stencil begin/end 0xB92500 / 0xB92520 (mode `0x252A1F0`);
      - recurse into each child, then `post_render`.

## Objects and the type registry

Every object derives from `vint_object_base`:

| Off | Object field |
|---|---|
| +0x00 | vtable |
| +0x04, +0x08 | links in the name bucket `0x22EF1B8[hash & 0x3F]` |
| +0x0C, +0x10 | links in the handle bucket `0x22EF2B8[handle & 0x3F]` |
| +0x1C | first child |
| +0x20 | next sibling |
| +0x24 | parent |
| +0x28 | handle (what Lua sees: a number) |
| +0x2C | name hash |
| +0x30 | document |

- Lua passes objects as their numeric handle. The bindings look them up through the handle bucket.
- `vint_object_find_by_hash` (0xB97970, EDX = hash, ESI = document) looks an object up by name.
- **Type registry** at `0x22F2370`: 0x10 bytes per type {name, parent type, property set, factory}. The count is `0x252A270`, max 0x20.
  - `vint_init` registers: element, group, animation, tween, bitmap, gradient, text, point, clip, bitmap_circle, sr2_map, video.
  - Each element type is allocated from a `vint_object_factory_imp<T>` (alloc / free / index_of / get).
  - `destroy` (slot 2) frees through the type's factory.

**vint_object_base slots:**

| Slot | Name | Notes |
|---|---|---|
| 2 | `destroy` | children first |
| 3 | `update` | per frame |
| 4 | `on_destroy` | |
| 5 | `clone` | |
| 6 | `is_type(t)` | |
| 7 | `get_type` | |
| 8 | returns false | |
| 9 | empty | |
| 10 | returns false | the earlier auto name `scalar_deleting_dtor` was wrong |

Properties are 0x24-byte descriptors {name, value type, set thunk, get thunk, …, flags (0x100 = can be tweened)}. Each thunk calls one vtable slot: get = even slot, set = the next odd slot. Values are passed as a tagged union {tag, data}:

| Tag | Type |
|---|---|
| 1 | int |
| 2 | handle / crc |
| 3 | float |
| 4 | string |
| 5 | bool |
| 6 | color (3 floats) |
| 7 | vec2 |
| 8 | event |
| 9 | bitmap |
| 10 | font |
| 12 | enum |
| 13 | variant |

**vint_element_base** (46 slots, table `0xEBDD50`):

| Property | Type | Get / set slot | Field |
|---|---|---|---|
| render_mode | enum | 12 / 13 | +0x8C |
| visible | bool | 14 / 15 | +0x59 |
| mask | bool | 16 / 17 | +0x58 |
| anchor | vec2 | 18 / 19 | +0x5C |
| offset | vec2 | 20 / 21 | +0x64 |
| tint | color | 22 / 23 | +0x7C |
| alpha | float | 24 / 25 | +0x88 |
| depth | int | 26 / 27 | +0x54 |
| screen_size | vec2 | 28 / 29 | computed |
| screen_nw | vec2 | 30 / 31 | computed (set returns false) |
| screen_se | vec2 | 32 / 33 | computed (set returns false) |
| rotation | float | 34 / 35 | +0x78 |
| scale | vec2 | 36 / 37 | +0x90 |
| auto_offset | enum | 38 / 39 | +0x6C |
| unscaled_size | vec2 | 40 / 41 | +0x70 (set returns false) |

The other element slots: 11 `build_transform`, 42 `pre_render`, 43 `render`, 44 `post_render`, 45 `apply_auto_offset`.

**Subclass properties:** every type's get/set methods are named in `vtable_slots.csv`.

| Class | Slots | Properties |
|---|---|---|
| `vint_element_bitmap` | 46-55 | image, image_crc, source_nw, source_se, custom_source_coords |
| `vint_element_gradient` (a bitmap) | 56-71 | gradient_nw/ne/sw/se, alpha_nw/ne/sw/se |
| `vint_element_text` | 46-75 | font, text_tag, text_tag_crc, text_scale, word_wrap, wrap_width, horz_align, vert_align, force_case, leading, line_frame_enable/w/m/e, insert_values |
| `vint_element_clip` | | clip_size, clip_enabled |
| `vint_element_bitmap_circle` | | image, source_nw/se, start_angle, end_angle, num_wedges |
| `vint_element_sr2_map` | | zoom, map_mode |
| `vint_element_video` | | vid_id_handle |
| `vint_element_group`, `vint_element_point` | | override screen_size / nw / se / offset only |
| `vint_object_animation` | 11-16 | start_time, is_paused, target_handle |
| `vint_object_tween` | 11-42 | target_handle, target_name, target_property, state, start_time, duration, start_value, end_value, loop_mode, algorithm, max_loops, start_event, end_event, per_frame_event, start_value_type, end_value_type |

Correction: the earlier caller-survey name `vint_object_tween::set_property` (slot 11) is really `get_target_handle`. `luafn_vint_set_property` reads it while resolving the tween's target.

## Lua

- `vint_lua_init` loads `vint_lib.lua` as a system lib with the `_UGGlobals` environment, then `vint_register_lua_functions` (0xB90E90) adds the 57 `vint_*` functions (`luafn_vint_*`). They cover:
  - objects: create, clone, find, destroy, parent / child / sibling, rename;
  - properties: get / set / set_typed;
  - documents: find, load, unload;
  - data groups / items: data binding;
  - input subscriptions, sounds, time index.
- Each document's script runs in its own Lua thread. `vint_lua_current_document` (0xB8E020) maps the running thread to its document through the context stack `0x2526360` (thread +0x14 = document handle), so bindings that take names resolve them in the calling document.
- Callbacks into Lua (input events, data subscriptions, tween events) are `vint_callback_lua` objects. Native ones are `vint_callback_native`.

## HUD render path (resolved)

- In gameplay: `gameplay_do_frame` → `gameplay_ui_frame` (0x68D2F0) → `hud_render` (0x75A230).
- `hud_render`:
  1. For each active overlay (indices at `0x1F768E4`, count `0x2527FF4`), calls the render callback at `0xE8E730 + idx*0x14 + 0xC`.
  2. 0x755B80, 0x6C6BE0, then 0x7552D0(0xEAC208).
  3. Tail-jumps (`jmp`) to `interface_render`, which draws every vint document, the HUD included.
- The earlier graph search missed this path because it only followed `call`.
- `hud_document_render` (0x799010) has no callers at all, by `call`, `jmp` or pointer. It is dead code.

## Screen states

`interface_do_frame` first runs `interface_screen_state_frame` (0x7EC170), a small state machine over 21 UI screen states. Each state has a 0x10-byte entry in `0xE8CF88`:

| Off | Callback |
|---|---|
| +0 | enter |
| +4 | exit |
| +8 | frame |
| +0xC | render (run by `interface_render`) |

The current state is `0xE8CF80` and the pending one `0xE8CF7C` (-2 = none). `0x7EC060` handles a 0.25 s idle timeout in some states.

## Input path

- `game_frame_tick` calls `vint_input_do_frame` (0xB99550) directly, every frame.
- Lua subscribes with `vint_subscribe_to_input_event(event, function, priority)` (0xB90250):
  1. The event name is looked up in the 34-name table `0x252A238` ("select", …) by `vint_input_event_index` (0xB99F70).
  2. The Lua function is wrapped in a `vint_callback_lua` (0xBA0950 find / 0xB8DBB0 create).
  3. `vint_input_subscribe` (0xB99380) takes a 0x4C-byte record from the free list `0x252A230`, gives it a handle (index | generation `0xE86E88` << 16) and inserts it by priority into the active list `0x252A234` (0xB992D0).
  `vint_subscribe_to_raw_input` does the same with kind 1.

| Off | Subscription field |
|---|---|
| +0x18, +0x1C | callback |
| +0x20 | handle |
| +0x24 | kind (0 event, 1 raw) |
| +0x28 | u8 remove pending |
| +0x2C | event index |
| +0x30 | u8 armed |

**Per frame** (`vint_input_do_frame`):
1. Clear the per-event consumed flags (`0x22EF550`, 34 bytes) and set "dispatching" (`0x252A17E`).
2. In priority order, run each subscription that is not pending removal:
   - **kind 0**, `vint_input_check_event` (0xB99A60): the event's bindings come from the project and are matched against the controller state:

     | Bindings | Entries |
     |---|---|
     | `0x252A218` / `0x252A21C` | buttons, 0x14 bytes |
     | `0x252A220` / `0x252A224` | stick axes with a threshold, 0x18 bytes |
     | `0x252A228` / `0x252A22C` | keys, 8 bytes |

     On a press, past the repeat delay (per-event timestamps in `0x22EF3B8`), it calls the callback's vtable slot 1 with (document handle, 0, subscription, 0x4C) and marks the event consumed, so lower-priority subscribers do not get it.
   - **kind 1**, raw input (0xB996C0): inventory_left / right / up, …
3. `vint_input_mark_consumed` (0xB99DB0) sets the game's "eaten" flags for every binding of a consumed event: buttons `0x23478D8`, axes `0x2348502/3`, keys `0x2348B82`. The game itself then ignores that input.
4. Remove subscriptions marked for removal, then clear the dispatching flag.

**Virtual keyboard:** `interface_vkeyboard_poll` (0x7F3450) delivers a finished on-screen keyboard result to the Lua function named at `0x1FD507C`, in a new thread whose +0x14 is the document handle at `0x2528B08`.

## The non-render side, step by step, for porting

**`interface_do_frame`** (0x7F3500) is the per-frame entry for everything below. It returns at once unless the interface is ready (`0x2528AFF`). Then:
1. If the player's state changed (0x9B9030 / 0x9B9070 on `0x21703D4`, cached in `0x347AF34`), refresh the localised image tags (0x7F2040).
2. 0x7EC170, then `interface_vkeyboard_poll` (0x7F3450).
3. `vint_dataitems_notify_all` (0xB95290).
4. `vint_datagroups_deliver_all` (0xB94560).
5. `vint_tweens_do_frame` (0xB9EA70).
6. `vint_update_documents(*0xE87998)` (the frame time).

`vint_input_do_frame` is called separately, from `game_frame_tick`.

### Input dispatch

**`vint_input_do_frame`** (0xB99550):
1. Clear the 34 per-event "consumed" bytes `0x22EF550` and set "dispatching" `0x252A17E = 1`.
2. Walk the subscription list `0x252A234` in priority order. Skip records with +0x28 = 1 (remove pending). Then:
   - kind (+0x24) 0 → `vint_input_check_event(sub)`;
   - kind 1 → `vint_input_check_raw(sub)` (0xB996C0).
3. Walk the list again: `vint_input_mark_consumed` (`EDI` = sub) for every record.
4. Walk it a third time and delete the records marked for removal (0xB99620).
5. `0x252A17E = 0`.

**Subscription record** (0x4C bytes; the first fields are in the table above):

| Off | Field |
|---|---|
| +0 / +4 | list links |
| +0x18 / +0x1C | callback reference {tag 8, handle} / callback object |
| +0x20 | handle |
| +0x24 | kind |
| +0x28 | remove pending |
| +0x2C | event index (kind 0), or the last value as f32 (kind 1) |
| +0x30 | armed (kind 0), or the raw binding index (kind 1) |
| +0x34 | repeat speed |
| +0x38 | repeat output |
| +0x3C | u8 "first press" |
| +0x3D | u8 accelerate |
| +0x40 | speed multiplier |
| +0x44 | maximum speed |
| +0x48 | base delay (s) |

**`vint_input_check_event`** (0xB99A60; cdecl `(sub)`):
1. Return if input is disabled (`0x252A54D`) or the event is already consumed this frame.
2. Event 26 is special: call 0xC06C30, mark it consumed and return.
3. Repeat timing: the per-event record is `0x22EF3B8 + event × 12` = {last fire ms, delay ms, handle}. Elapsed = `0x252A3CC` (ms clock) − last, with wrap handling.
4. Match the event's bindings. The first hit goes to **fire**:

   | Binding table | Entry | Fields | Hit when |
   |---|---|---|---|
   | buttons `0x252A218`, count `0x252A21C` | 0x14 | +0 controller, +4 button, +8 event, +0xC u8 mode, +0x10 u8 repeat | mode 1: `0xC067F0` (edge); also sets sub+0x3C when the button is down and not eaten. Otherwise: down (`0x23478D4 + controller × 0x104C + button × 0x24`) and not eaten (+4) |
   | sticks `0x252A220`, count `0x252A224` | 0x18 | +0 controller, +4 stick, +8 direction, +0xC f32 threshold, +0x10 event | read the stick (0xC066F0 → x, y). Direction 0: y > threshold; 1: y < threshold (y negated when 0xC06A10, invert); 2: x < threshold; 3: x > threshold (x negated when 0xC069F0) |
   | keys `0x252A228`, count `0x252A22C` | 8 | +0 key, +4 event | `input_key_down` (0xC111D0) |
5. **Fire.** If the subscription is armed (+0x30) and elapsed > delay:
   1. `cb = vint_callback_from_ref(&sub+0x18)` (0xB93750).
   2. `vint_input_repeat_next(repeat byte)` (0xB99980; `ECX` = sub, `ESI` = timing record) computes the next delay.
   3. `cb->vtable[1](sub+0x1C → +0x4C (document handle), 0, sub, 0x4C)`.
   4. Mark the event consumed, then set last = now and handle = sub+0x20.

   Either way, call 0xC06C30 and return.
6. **No hit:** armed = 1, delay = 0, +0x3C = 1, so the next press fires at once.

**`vint_input_repeat_next`** (0xB99980): the hold-to-repeat curve.
- `base` = sub+0x48 when the binding repeats, else 50000 (no repeat).
- On the first press (+0x3C), speed = 1. Otherwise speed ×= multiplier (+0x40), or with acceleration (+0x3D) by `(dt × base + 1) × multiplier`.
- Speed is clamped to the maximum (+0x44).
- Accelerating repeats set +0x38 = `base × dt × speed` and delay 0. Otherwise +0x38 = 1 and delay = `round(base × 1000 / speed)` ms.
- Finally clear +0x3C.

**`vint_input_check_raw`** (0xB996C0). The binding is `0x252A240 + sub+0x30 × 0x18` (count `0x252A23C`): +0 controller, +4 kind (0 button, 1 stick), +8 button / stick, +0xC u8 axis, +0x14 name.
- **Button:**
  - value = 0 if eaten, else `input_button_analog` (0xC06870);
  - mark it eaten (controller 0);
  - names "inventory_left / right / up / down" also eat the matching d-pad bytes (`0x2348070` / `0x23483F4`, `0x2348094` / `0x23483AC`, `0x2348028` / `0x23483D0`, `0x234804C` / `0x2348418`).
- **Stick:**
  - read it (0xC066F0, 0xC06BF0);
  - for stick kind 2, mark axis eaten bytes `0x2348502/3` and `0x2348542/3`;
  - value = x (axis 0) or y (axis 1).
- **Fire** when the previous value (sub+0x2C) is negative or differs by ≥ 0.05: store the value and call `cb->vtable[1](doc handle, 0, sub, 0x4C)`. There is no consumed flag and no repeat.

**`vint_input_mark_consumed`** (0xB99DB0; `EDI` = sub). For kind 0 subscriptions whose event was consumed, every binding of that event on controller 0 sets the game's eaten byte:
- buttons: `0x23478D8 + button × 0x24` (button < 0x57);
- stick directions 0 / 1: `0x2348503 + stick × 0x40`;
- stick directions 2 / 3: `0x2348502 + stick × 0x40` (stick < 8);
- keys: `0x2348B82 + key × 0xC` (key < 0x100).

The game's own input code (Rust `input.rs`) then reads these as eaten.

`vint_callback_from_ref` (0xB93750; `EAX` = &{tag, handle}) returns the callback whose +0x18 equals the handle in the 64-bucket hash `0x22F2670[handle & 0x3F]` (chain +8). It returns 0 unless tag = 8.

### Data items and data groups

- **`vint_dataitems_notify_all`** (0xB95290) walks the data items `0x252A1F4` (next +0xC). For each, `vint_dataitem_notify` (0xB952F0; `ESI` = item):
  - If "dirty" (+0x229) and it has a value (+0x21C), 0xB944C0(value).
  - If "changed" (+0x228), call each subscriber (list +0x220, next +0x14) as `callback(+0xC handle)->vtable[1](+8, 2)`, then 0xB94420(value).
  - Clear both flags.
- **`vint_datagroups_deliver_all`** (0xB94560) walks the data groups `0x252A204`. For each, `vint_datagroup_deliver` (0xB945E0) takes every queued item (list +0x10):
  - calls each subscriber of kind 5 (list +0x18, next +0x14; handle +0xC) as `vtable[1](+8, 5, item, 0x22C)`;
  - unlinks the item and frees it (0xB94980).

  A group with +0x1C set is then destroyed (0xB93BC0).

These are the Lua `vint_dataitem_*` / `vint_datagroup_*` bindings' delivery side. Items are 0x22C bytes.

### Tweens

All `update` slots (vtable +0xC) of the vint classes are an empty `ret`: base, group, animation, tween, and every element type. So the `vint_object_tree_update` walk in `vint_update_documents` calls nothing in this build. A port can keep only the time step and the unload pass. The animation work is all in `vint_tweens_do_frame`.

**`vint_tweens_do_frame`** (0xB9EA70):
1. Point three event queues at stack buffers: start `0x252A278` (count `0x252A284`), per-frame `0x252A280` (`0x252A28C`), end `0x252A27C` (`0x252A288`), 64 `{tween handle, callback handle}` pairs each. Also point the tween list `0x252A290` (count `0x252A294`) at a stack array.
2. From the type registry (`0x22F2370 + type × 0x10`, type `*0xEBE2F8` = tween), call the pool's for-each (vtable +0x14) with `vint_tween_collect` (0xB9EA00). That appends every object that is a tween with a live handle (+0x28).
3. qsort the list by +0x78 (`vint_tween_compare` 0xB9EA50), then `vint_tween_step` on each.
4. Deliver the events: start, then per-frame, then end. Each entry's tween is found in the object hash `0x22EF2B8` (handle at +0x28, chain +0xC), its callback with `vint_callback_from_ref`. Then `callback->vtable[1](tween+0x30 → +0x4C (document handle), 1, …, 8)`.
5. Clear the queue pointers and the count.

**Tween fields** used by the step:

| Off | Field |
|---|---|
| +0x24 | parent animation (or 0) |
| +0x28 | own handle (0 = dead) |
| +0x30 | document |
| +0x54 | target property type |
| +0x64 | start value type (must equal +0x54) |
| +0x74 | start time |
| +0x78 | sort key |
| +0x7C | duration |
| +0x94 | end value (its type at +4 must equal +0x54, or be 0xD) |
| +0x98 | loop mode: 0 none, 1 cycle, 2 bounce |
| +0x9C | algorithm: index into the interpolators `0xE86E18` |
| +0xA0 | max loops |
| +0xA4 | state: 0 idle, 1 running, 2 / 3 not stepped, 4 done |
| +0xA8 | progress (negative while a bounce runs backwards) |
| +0xE4 | started |

**`vint_tween_step`** (0xB9EDB0; cdecl `(tween)`):
1. **Skip** it if:
   - its handle is 0;
   - it is in state 2 or 3;
   - the value types don't match;
   - the target (0xB9E930) is missing.
2. **Local time**:
   - With a parent animation (`is_type(animation)`, vtable +0x18): `doc time (+0x45C) + anim+0x5C − anim+0x54`.
   - Otherwise: the document's time.

   Then subtract the start time (+0x74).
3. **State**:
   - **No loop:**
     - Before the end: state 1 and started; this is a start event (unless it was already running).
     - Past the duration: state 4, an end event, plus a start event if it was idle.
   - **Cycle / bounce:**
     - With max loops > 0 and time > duration × loops: state 4 and an end event.
     - Else, when time > 0 and not yet running: state 1 and a start event.
4. If the time is negative, return. If the tween was already done, return. If idle, return.
5. **Fraction `t`**:
   - **No loop:** `time / duration`, capped at 1.
   - **Cycle:** `fmod(time, duration) / duration` (0x4DA540), stored in +0xA8. Every step counts as a start event.
   - **Bounce:** the same fraction. On odd periods (0x427600 / 0xDA0140) use `1 − t` and store it negated. A change of direction raises the end or start event.
6. **Interpolate** with `f = 0xE86E18[algorithm]` (`f(t, start, delta)`):
   - **Type 3** (an angle): the start and end go through 0x5E4970 (wrap), interpolate, then 0x7903D0.
   - **Type 6** (three components): read both values (0xB956B0) and interpolate each component, then 0xB9DBE0.
   - **Type 7** (two components): read with 0xB8CF80 and interpolate each, then 0xB9DC10.
   - **Other types, or no interpolator:** 0xB93620 (take the end value).
7. Write the value to the target property (0xB8A2A0).
8. **Queue events** (64 each). The callback handles come through `vint_callback_from_ref` on the tween's event references:
   - a start event when flagged;
   - an end event when flagged;
   - a per-frame event every step.
9. 0xB93790.

**Porting order:**
1. `vint_callback_from_ref` and the object-handle hash.
2. Input:
   1. `vint_input_repeat_next`;
   2. `vint_input_check_event` and `vint_input_check_raw` (they need the controller tables that `input.rs` already owns);
   3. `vint_input_mark_consumed`;
   4. `vint_input_do_frame`.
3. Data delivery (`vint_dataitem_notify`, `vint_datagroup_deliver`) once the item layout is ported.
4. Tweens: the interpolator table (`0xE86E18`, one function per algorithm), `vint_tween_step`, `vint_tweens_do_frame`. The property write (0xB8A2A0) goes through the property descriptors described above.
5. `vint_update_documents` reduces to the time step and the unload pass.

## Platform draw callbacks

vint never draws directly. Everything goes through the table at `0x1FD52B0` (global `0x252A1D8`), filled by `interface_init`, which calls the game's 2D layer (0xD0xxxx / 0xD14xxx, render state `0x2A14B28`). The vint-side wrappers are 0xB92340-0xB925D0, one per slot.

| Slot | Target | Use |
|---|---|---|
| +0x00 | 0x7F0AB0 | begin document: resets the clip state, 0x7F0950, sets the full-screen viewport (0xD14D20) |
| +0x04 | 0x7F0BA0 | set colour (see "Rendering, step by step") |
| +0x08 | 0x7F0B00 | set mask mode |
| +0x0C | 0x7F0B20 | clear the mask |
| +0x10 | 0x7F0A70 | clip begin (viewport/scissor 0xD14D20); used by `vint_element_clip` |
| +0x14 | 0x7F0A90 | clip end (full screen) |
| +0x18 | 0x7F0A50 | set blend mode (per element) |
| +0x1C | 0x7F1350 | bitmap quad: `vint_element_bitmap` render → 0xB9C030 |
| +0x20 | 0x7F0D20 | bitmap circle: `vint_element_bitmap_circle` → 0xB9C190 |
| +0x24 | 0x7F16A0 | gradient: `vint_element_gradient` → 0xB9BF80 → 0xB92420 |
| +0x28 | 0x7F1BD0 | text: `vint_element_text` → 0xBA8A90, glyph runs through 0xD15E80 |
| +0x2C | 0x7F1C50 | text helper (0xD0B020) |
| +0x30 | 0x7F1C90 | filled rectangle (0xD0E7F0) |

The other three tables:
- `0x1FD4FEC` (global `0x252A1D4`): resources, such as bitmap lookup and pegs.
- `0x1FD52E8` (global `0x252A1DC`): sound.
- `0x1FD509C` (global `0x252A1E0`): text input.

## Rendering, step by step, for porting

**`vint_document_render`** (0xB8ACF0; stdcall `(doc)`, `ret 4`):
1. Build a default **render state** of 0x70 bytes on the stack:

   | Off | Default | Field |
   |---|---|---|
   | +0 | u32 1 | blend mode |
   | +4 / +8 / +0xC | 1.0 | tint r, g, b |
   | +0x10 | 1.0 | alpha |
   | +0x14 | 0 | byte "no masks" |
   | +0x24 / +0x28 | 1.0 | scale x, y |
   | +0x2C / +0x30 | 0 | translation |
   | +0x34 | 1.0 | |
   | +0x38 / +0x3C / +0x40 / +0x48 | 0 | |
   | +0x44 | 1.0 | |

   From +0x24 on it's a 2D transform.
2. Call draw slot +0x00 (begin).
3. If the document has a root element (+0x458), `vint_element_render_tree(ECX = root; &state, scratch 0x22ECBB0, 256)`.

**`vint_element_render_tree`** (0xB95B10; `ECX` = element, stdcall `(state *, scratch **, scratch count)`, `ret 0xC`):
1. Return unless visible (+0x59) and `state.alpha > eps` (0xE5C1B0).
2. `element->vf42()` (vtable +0xA8, `pre_render`).
3. Copy the 0x70-byte state, then combine it with the element's (0xB957B0; `EDI` = &state, cdecl `(element)`): tint, alpha, transform.
4. Draw slot +0x18 (blend mode = `state[0]`).
5. `element->vf43(&state)` (+0xAC, `render`). The element emits its draws through the draw slots.
6. Collect the children (first child +0x1C, next sibling +0x20) that pass `is_type(*0xEBDF6C)` (vtable +0x18, "is an element") into the scratch array.
7. Sort them by depth (`qsort` with 0xB95AD0).
8. If `state+0x14` is set, recurse into each child with the remaining scratch.
9. Otherwise, handle masks with the global stencil mode `0x252A1F0`. For each child, before recursing:
   - if the mode is 0 and the child is a mask (+0x58 == 1): draw slot +0x0C (clear mask), then slot +0x08 with mode 2, and remember that a mask was opened;
   - if the mode is 2 and the child isn't a mask: slot +0x08 with mode 1.

   After the children, if a mask was opened: slot +0x08 with 0, and the mode goes back to 0.
10. `element->vf44()` (+0xB0, `post_render`).

**Draw callbacks** (table `0x1FD52B0`, global `0x252A1D8`). These replace the table above, and give the arguments:

| Slot | Function | Arguments | Does |
|---|---|---|---|
| +0x00 | 0x7F0AB0 | - | begin: blend `0xE8C548 = 1`, mask mode `0x2528B14 = 0`, colour cache `0xEB15BC = FFFFFFFF`, apply (0x7F0950), full-screen scissor `gfx_set_scissor_rect(ESI = w 0x22FDC1C, EDI = 0; 0, h 0x22FDC20)` |
| +0x04 | 0x7F0BA0 | `u8 rgba *` | set colour: if different from the cache `0xEB15BC`, `gfx_set_color(r, g, b, a)` and update the cache |
| +0x08 | 0x7F0B00 | `int mode` | set mask mode `0x2528B14` and apply (0 normal, 1 test the mask, 2 write the mask) |
| +0x0C | 0x7F0B20 | - | clear the mask: state pair from `0xEC27B0` (flags & ~0xE \| 1), 0xA93670, colour 0, full-screen `im2d_fill_rect` |
| +0x10 | 0x7F0A70 | `int rect[4]` (x, y, w, h) | `gfx_set_scissor_rect(ESI = rect[2], EDI = rect[0]; rect[1], rect[3])` (clip begin) |
| +0x14 | 0x7F0A90 | - | full-screen scissor (clip end) |
| +0x18 | 0x7F0A50 | `int blend` | if different from `0xE8C548`, store it and apply |
| +0x1C | 0x7F1350 | element draw data | bitmap quad (`vint_element_bitmap`): `gfx_set_bitmap`, strip quad through `gfx_2d_alloc` / deferred type 0x1C |
| +0x20 | 0x7F0D20 | | bitmap circle (sin / cos segments) |
| +0x24 | 0x7F16A0 | | gradient quad |
| +0x28 | 0x7F1BD0 | `u16 *text, …, rect` | text: `gfx_draw_text_vint` (0xD15E80, deferred type 0x29) with the state pair `0x2A14B28` |
| +0x2C | 0x7F1C50 | `vec2 *pos, vec2 *size` | `im2d_draw_17(ECX = 0x2A14B28; pos.x, pos.y, size.x, size.y)` |
| +0x30 | 0x7F1C90 | `vec2 *pos, vec2 *off, vec2 *size` | filled rectangle in ints (0xD0E7F0) |

**`vint_apply_state`** (0x7F0950) turns the mask mode `0x2528B14` and blend mode `0xE8C548` into vint's render-state pair `0x2A14B28 / 0x2A14B2C`, from the table `0xEC27B0`. It also sets the colour where the mode needs it (mask write: colour 0, 0, 0, 255). Every draw passes `0x2A14B28` as its pair.

**Porting order:**
1. The draw callbacks (they only call the already ported `gfx_*` / `im2d_*`).
2. `vint_apply_state`.
3. `vint_element_render_tree` and `vint_document_render`.
4. Each element type's `render` (vf43), which fills the per-slot draw data.
5. The update walk (`vint_object_tree_update`) and input (`vint_input_do_frame`, above) can be ported independently of rendering.

## Open

- The data item / group layouts beyond the delivery side.
