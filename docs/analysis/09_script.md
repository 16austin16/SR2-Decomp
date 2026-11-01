# 09: The Lua host

SR2 embeds a statically linked **Lua 5.0**. The runtime functions are named from its own error messages (`luaV_*`, `luaD_*`, `lua_*`, `luaB_*`); see `docs/functions.csv`. This file covers the game's host layer around it. Names are in `docs/functions.csv` (subsystem `script`).

## States

There are up to **4 Lua states**, created by `lua_load_system_lib` (0xCDE400):
1. `lua_open` (0xCD87F0).
2. Record the state in the table `0x25268D8`: 8-byte entries {L, per-state callback}, count `0x252A664`, first state `0x252A660`.
3. `luaopen_base` (0xD730A0), then 0xD74D50 (more standard libraries).
4. Load and run `system_lib.lua` (`script_load_file` + `script_run_includes`).

| State | Global | Created by | Library script | Bindings |
|---|---|---|---|---|
| game ("ug") | `0x252983C` | `script_main_state_create` (0xA23EB0), from `script_system_init` (0xA24700) | `ug_lib.lua` (`script_main_state_load_libs` 0xA23E40) | `script_register_game_functions` (0xA4EE00) |
| vint (UI) | `0x252A1B8` | `vint_lua_init` (0xB914F0), from `interface_init` | `vint_lib.lua` | `vint_register_lua_functions` (0xB90E90), 0xB8DAD0, and `interface_register_lua_functions` (0x7F35A0) |

The game state's per-state callback is 0xA23E30, which just returns. The vint state passes 0.

## Bindings

There is no global binding table. Each subsystem has a **registrar** that builds a `{const char *name, lua_CFunction fn}` array on the stack (`mov [esp+X], "name"; mov [esp+X+4], fn`). It then loops over the array, with L in ESI:

```
lua_pushstring(L, name); lua_pushcclosure(L, fn, 0); luaV_settable(L, globals (L+0x40), top-2, top-1); pop 2
```

31 functions do this. `tools/harvest_names.py` (`script_bindings`) reads exactly this pattern, which is how the 1,136 `luafn_*` names were found.

- `script_register_game_functions` (0xA4EE00): the big gameplay set, on the game state. It allocates the array on the stack (`__alloca_probe`) and also calls 0xA4EC00 / 0xA4ED70 / 0x89DA60.
- `interface_register_lua_functions` (0x7F35A0): calls 19 subsystem registrars (cribs 0x5477A0, garage 0x556D40, 0x5EEA10, 0x750360, … 0x891930) **on the vint state**, so UI scripts can call gameplay functions.
- `vint_register_lua_functions` (0xB90E90): the vint_* API.

To port a binding:
1. Read its arguments with the Lua 5.0 C API (`lua_tonumber` 0xCDC250, `lua_tostring` 0xCDCA80, …).
2. Push the results.
3. Return the count.

Bindings that start or wait on script threads use the thread table below.

## Loading scripts

`script_load_file` (0xCDDF30) loads a named script. Its inputs are the name on the stack, ECX = L, and EDI = a temporary allocator with vtable slots +0x30 mark, +0x1C alloc, +0x34 release.
1. `script_check_name` (0xCDD2F0) normalises and checks the name (stricmp / strncpy).
2. `file_open` (0xBFDC20) with mode "rb" goes through the **packfile file API** (01_packfile.md), not the streamer.
3. Allocate the file size (+0x160 of the file handle) from the temporary allocator.
4. `file_read` into it, then `file_close`.
5. If the GC debt is over its threshold, run `luaC_collectgarbage`.
6. `script_run_chunk` (0xCD9FD0) parses (0xCD97A0) and runs (0xCDCFB0) the chunk.
7. Release the temporary memory.

**Includes.** While a chunk runs, its include/require binding queues names in `0x25268F8[]` (count `0x252A65C`). `script_run_includes` (0xCDE110) then loads each queued file and frees the names. `script_load_with_includes` (0xCDE1A0) is load + includes.

**Dynamic globals.** `script_load_dynamic_globals` (0xCDE2E0, from `interface_document_load`):
1. Call the Lua function `_PrepareForDynamicGlobals`.
2. Load the chunk.
3. Run includes.
4. Call `_DynamicGlobalsLoadComplete`.

**Mission / activity scripts.** `script_load_mission` (0xA25370, from 0xA24810 / 0x8A3710):
1. Only when allowed (no network game, or host).
2. Load `<name>.lua` into the game state.
3. Run includes, then `script_after_load` (0xCDE080: 3 × 0xCDA000 and a GC).
4. Start the thread `<base>_init` and resume it once right away.
5. Create the thread `<base>_main`.

## Script threads

There is one global table at `0x2524360`: 256 entries of 0x20 bytes. The count is `0x252A638`; ids come from `0x252A63C`.

| Off | Field |
|---|---|
| +0x00 | u16 thread id (unique among live threads) |
| +0x04 | `lua_State *` coroutine (`lua_newthread` 0xCDCED0) |
| +0x08 | owning state |
| +0x0C | u8 "not started yet" |
| +0x10 | function name (heap copy) |
| +0x14 | vint document handle (vint threads), else 0 |
| +0x18 | u16 flags: 1 = ran this frame, 2 = waiting, 4 = paused, 8 = (create mode 1), 0x40 = dead |
| +0x1C | number of arguments for the first resume |

**Create.** `script_thread_create` (0xCE01E0) takes EAX = state (or the parent's state), the function name, a parent, a mode and a flag.
1. Look the function up in the globals: mode 0 uses `luaV_gettable`, modes 1/2 use `lua_rawget`.
2. Require it to be a Lua function.
3. Create a coroutine and fill a table entry.

`script_thread_create_for_document` (0xCE03F0) does the same for a vint document. It takes the state from doc+8 and stores the document handle (doc+0x14).

**Resume.** `script_thread_resume` (0xCE0570) takes EDI = &entry.
1. Skip the thread if it is dead (0x40) or paused (4), or if the hook `0x252A640` returns 1.
2. Stop if it is waiting (flags & 3), or if the hook `0x252A644` says it is blocked.
3. On the first run, push the function by name plus its arguments.
4. Set flag 1, then `lua_resume` (0xCDAC30) inside 0xCDFD20 / 0xCDFD00.
5. If the thread is still suspended (`script_thread_is_suspended` 0xCDFD40), keep it. On an error, print it when `0x252A613` is set (0xCDDBD0).
6. When it ends, run the state's callback (`script_state_get_callback` 0xCDD1E0) and remove the entry (`script_thread_kill` 0xCDFF10).

**Per frame.** `script_do_frame` (0xCDDE70) runs from `sim_frame` and 0x73CB30.
1. If script memory (hook `0x252A66C`) exceeds 85% of the budget (hook `0x252A668`), run a full GC on every state.
2. `script_threads_resume_all` (0xCE06D0): clear flag 1 on every thread, and skip threads whose state is in the 4-entry paused-state list `0x2526848`. Resume the rest in table order. A thread that ended is removed and the index steps back.

The supervisor (0x68BE50) also calls `script_threads_resume_all` during loading.

## vint and Lua

- vint documents run on the vint state.
- Element callbacks and input subscriptions (06_vint.md) hold `vint_callback_lua` objects that call Lua functions by name.
- Document scripts start threads through `script_thread_create_for_document`, so they carry the document handle in +0x14.
- `game_frame_tick` uses `script_thread_create` on the vint state directly for the "screen_change_failed_pc" thread of the document whose +0x48 matches, then resumes it.
- vint and game threads share the one table and the same scheduler.

## The VM interface, for porting

SR2 links **Lua 5.0** with `lua_Number = double`.

**Values.** A `TObject` is 16 bytes: `+0 tt`, `+8 value`.
- Tags: 0 nil, 1 boolean (value +8 = 0 / 1), 2 light userdata, 3 number (double at +8), 4 string, 5 table, 6 function, 7 userdata, 8 thread.
- `lua_State`: `+8 top`, `+0xC base`, `+0x10` global state (+0x20 / +0x24 the GC threshold / count checked before allocating).
- Stack index → address: `luaA_index` (0xCDBE80; `EAX` = index, `ECX` = L). Positive indices are `base + (i − 1) × 16`. Negative ones are `top + i × 16`. −10000 (`0xFFFFD8F0`) is the registry, −10001 the globals, and lower values are upvalues.

**Bindings.** Every `luafn_*` (1,128 named) is a plain `lua_CFunction`: cdecl `int (lua_State *L)`.
- It reads its arguments with `lua_tonumber` / `lua_tostring`.
- It writes results **inline**: `L->top->tt = 3; L->top->value = (double)x; L->top += 16`. Nil is tt 0 and booleans tt 1.
- It returns the number of results.

So a Rust binding is a C-ABI `extern "C" fn(*mut lua_State) -> i32` writing the same 16-byte values.

**API functions the game calls** (outside the bindings), with their LTCG conventions:

| Function | Convention | Game call sites |
|---|---|---|
| `lua_tostring` 0xCDCA80 | `EAX` = index, `ECX` = L → `char *` | 900 (and most bindings) |
| `lua_tonumber` 0xCDC250 | `EAX` = index, `ECX` = L → double in st0 | 532 |
| `lua_pushstring` 0xCDCAF0 | `ESI` = L, cdecl `(char *)`; NULL pushes nil | 229 |
| `lua_pushcclosure` 0xCDCD30 | `ESI` = L, cdecl `(fn, n upvalues)` | 32 (the registrars) |
| `lua_rawget` / `lua_rawset` 0xCD4FE0 / 0xCD5040 | | 25 / 14 |
| `lua_pcall` 0xCDA480 | `ESI` = L, cdecl `(nargs, nresults, errfunc)` | 6 |
| `luaL_argerror` / `luaL_error` / `lua_error` | | 18 / 7 / 5 |
| `lua_newthread` 0xCDCED0, `lua_resume` 0xCDAC30, `lua_yield` 0xCDAAC0 | the script thread scheduler (above) | |

**Pushing arguments into a script thread before its first resume.** These take a thread table entry (+4 = the coroutine's `lua_State`, +0x1C = the argument count):

| Helper | Address | Arguments | Does |
|---|---|---|---|
| `script_thread_push_string` | 0xCE01B0 | `EDI` = entry, `EAX` = string | pushes a string, or nil when NULL; count += 1 (125 call sites) |
| `script_thread_push_bool` | 0xCDFCB0 | `ECX` = entry, cdecl `(u8)`, `ret 4` | tt 1 |
| `script_thread_push_number` | 0xCDFC60 | `ECX` = entry, cdecl `(double)`, `ret 8` | tt 3 |
| `script_thread_resume_if_alive` | 0xCE0740 | `EAX` = &entry pointer | if the entry exists and isn't dead (flag 0x40), `script_thread_resume(0)` (this was "Open") |

**Memory.** The VM allocates through `script_heap_alloc` (0xCDF2C0: `EAX` = size, under the critical section `0x347ADE4`). It frees through `script_heap_free` (0xCDE870, 260 call sites), with size-class pools at `0x25263A4`.

**Registration chain:**
1. `script_main_state_load_libs` loads the Lua base libraries (`luaB_*`, `lua_load_system_lib`).
2. The game bindings are registered by tables of `{name, function}` pairs, harvested as `luafn_*`.
3. On the vint state, `interface_register_lua_functions` (0x7F35A0) calls the 24 subsystem registrars (`*_lua_register`, 28_menus_hud.md), each a run of `lua_pushstring(name)` + `lua_pushcclosure(fn, 0)` + `luaV_settable`.

**Per frame.** `script_do_frame` → `script_threads_resume_all` (above).

**Porting order:**
1. The value layout and the `lua_State` fields above.
2. The five API functions the bindings use (`tostring`, `tonumber`, `pushstring`, `pushcclosure`, `luaA_index`).
3. Bindings one by one (each is independent).
4. The thread table and its push / resume helpers.
5. The VM itself last: it can stay original while everything around it is Rust.

## script_do_frame and the thread scheduler at instruction level (for the port)

### `script_do_frame` (0xCDDE70)

No arguments; aligned frame (`and esp, −8`); saves `ESI`.

1. used = `[0x252A66C]` ? `call [0x252A66C]` : −1 → `fild` → f32 A.
2. budget = `[0x252A668]` ? `call [0x252A668]` : −1 → `fild`, × 0.85 (double at 0xE5C0E8) → f32 B.
3. **If B ≤ 0** (`fcom` with 0, `test ah, 0x41` / `jnp`): **return at once, without resuming any thread.**
   - So with no budget callback installed (B = −0.85), no script thread ever runs.
   - An unordered compare (NaN) continues.
4. If A > B: for each script state `0x25268D8[i × 8]` (i < `[0x252A664]`):
   1. set `G = state +0x10`; `G+0x20` (GC threshold) = 0;
   2. if `G+0x24` (blocks) ≥ `G+0x20`: `luaC_collectgarbage(cdecl state)`.

   The threshold was just set to 0, so the test is always true: a full collection on every state.
5. `script_threads_resume_all()`.

### `script_threads_resume_all` (0xCE06D0)

Saves `EBX` / `ESI` / `EDI`; `[esp+0xF]` is an error byte.

- The thread records live at `0x2524360`, **32 bytes each**, with count `[0x252A638]` re-read every step:

  | Offset | Field |
  |---|---|
  | +4 | `lua_State` |
  | +8 | owner / state id |
  | +0xC | u8 first run |
  | +0x10 | function name |
  | +0x18 | u16 flags |
  | +0x1C | argument count |

For i = 0 while i < count:
1. Flags &= ~1.
2. If the owner +8 is in the paused list (4 dwords `0x2526848..0x2526854`): skip this record.
3. Otherwise `ok = script_thread_resume(EDI = &(record pointer on the stack), cdecl &error byte)`.
4. If it returns 0 (the record was killed): **i − 1 and the pointer − 0x20**, so the same index runs again, because the kill compacts the array.

### `script_thread_resume` (0xCE0570)

`EDI` = **pointer to** the record pointer, cdecl (u8 *error or 0); returns `AL`: 1 = keep, 0 = removed. Aligned frame; saves `EBX` / `ESI`; `BL` = error.

1. Flags & 0x40 → **return 0 without killing** (already dying).
2. Flags & 4 (kill requested) → kill.
3. If the pre-check hook `[0x252A640](cdecl record)` exists and returns 1 → kill.
4. Flags & 3 (paused / waiting) → return 1.
5. If the skip hook `[0x252A644](cdecl record)` exists and returns 1 → return 1.
6. **First run** (byte +0xC):
   1. `lua_pushstring(ESI = L, cdecl name +0x10)`;
   2. `luaV_gettable(cdecl L, L+0x40 (globals), top − 0x10, 0)`, and copy the result over the top slot (16 bytes);
   3. if the argument count +0x1C > 0: 0xCDC530(`ESI` = L, `EAX` = −1 − nargs), which moves the function below its arguments;
   4. clear +0xC.
7. Flags |= 1; 0xCDFD20 (current thread on).
8. `BL` = `lua_resume(ESI = L, cdecl nargs +0x1C)` ≠ 0. The count is **passed on every resume**, not only the first.
9. 0xCDFD00 (current thread off), 0xCDD100. If the error pointer is set: `*error = BL`.
10. If `script_thread_is_suspended(cdecl record)` (it yielded):
    - an error goes to the error path;
    - otherwise, if `[0x252A613]`: 0xCDDBD0(cdecl L, 0);
    - **return 1**.
11. Finished or error. On error, with a live L: the owner's error callback `script_state_get_callback(EDX = owner +8)` → `cb(cdecl record)`.
12. **Kill:** `script_thread_kill(cdecl EDI)`; return 0.

**Port:**
- The order of the checks and the two hooks (`0x252A640` / `0x252A644`) matter for missions that pause threads.
- Keep the "repeat the index after a kill" step: `script_thread_kill` compacts the array, so the next thread moves into the current slot.

## Open

- The four hook setters (`0x252A640/644/668/66C`, written by 0x51F700, 0xA244D0 and 0xCDF780).

## The 20 most-used Lua bindings, for porting

**How they were ranked.** Call sites in the shipped scripts: all 177 `.lua` files in the `.vpp_pc` archives, with `patch.vpp_pc` overriding the others, and comments stripped. The mission 1 log can't rank bindings, because a binding's own code is too short for the 1 ms EIP sampler. Only 6 bindings showed up, led by `vint_set_property` (37 samples).

**Binding conventions (LTCG).**
- Every binding is cdecl `(lua_State *L)` and returns how many results it pushed.
- Most read the argument count first: `n = (L->top − L->base) >> 4`, clamped to the most they use. Arguments are then read relative to the top, as index `i − 1 − n` (so missing trailing arguments come out as nil).
- `lua_tostring` (0xCDCA80) and `lua_tonumber` (0xCDC250) take `ECX` = L and `EAX` = index. `lua_tonumber` returns on the FPU, and `_ftol2` converts it.
- Optional arguments are tested with `luaA_index` (0xCDBE80): type 0 is nil, type 1 with value 0 is false.
- Results are written straight to `L->top` (type, value), then top += 0x10:
  - number: type 3, double at +8;
  - boolean: type 1, value at +8.

**vint handles.** The vint bindings look objects up by handle in the hash table `0x22EF2B8[handle & 0x3F]`, chained through +0xC, with the handle at +0x28.

| # | Binding | Sites | Args → results | What it calls |
|---|---|---|---|---|
| 1 | `vint_set_property` 0xB8FAC0 | 4353 | (handle, property name, value…) up to 6 → 0 | finds the object, looks up the property (0xB978A0), type 0xD with `start_value` / `end_value` is a tween's value (rebinds the target, 0x634BA0); otherwise sets the value through the property's setter |
| 2 | `vint_object_find` 0xB8F400 | 2449 | (name, [parent handle], [document]) → handle number | `vint_lua_current_document` or the given document, then `vint_object_find_by_hash` under the parent (0xB98340) or from the root (0xB97970) |
| 3 | `trigger_enable` 0xA47BB0 | 856 | (trigger name, [enable = true]) → 0 | `trigger_find_by_name` (0xA6C870), `trigger_set_enabled` (0xA6D0C0); on enable 0xA6BCE0 and stamps +0x11C with the game time `0xE843AC`; on disable 0xA6BD60 |
| 4 | `thread_new` 0xD74C70 | 773 | (function name, args…) → thread id (65535 on failure) | `script_thread_create` (0xCE01E0) in the current script slot (`0x2526364[0x2526360]`), copies the arguments (0xCDBE40), `script_thread_resume` (0xCE0570) |
| 5 | `thread_yield` 0xD74BE0 | 670 | () → yields | `lua_yield(L, 0)` |
| 6 | `on_trigger` 0xA3EBA0 | 609 | (callback name, trigger name) → 0 | `trigger_find_by_name`, then `script_set_callback` (0xA530F0) on the trigger's callback block (+0x150) |
| 7 | `vint_get_property` 0xB8F570 | 394 | (handle, property name) → the value(s) | finds the object, 0xB97680 looks up the property, 0xB8D8F0 pushes it |
| 8 | `sizeof_table` 0xD74C00 | 390 | (table) → count (−1 with no argument) | `lua_table_count` (0x7762E0) |
| 9 | `on_death` 0xA3DAA0 | 380 | (callback name, object name) → 0 | `script_find_object_by_name`, then sets the callback on the human (+0x1234 block) or on the object's script info (+0xBC) |
| 10 | `debug_print` 0xD74BA0 | 368 | (category, text) → 0 | reads both strings and does nothing (stripped in release) |
| 11 | `coop_is_active` 0x7EF490 | 353 | () → boolean | `coop_session_active` (0x7F7AD0) |
| 12 | `release_to_world` 0xA42040 | 340 | (group or object name) → 0 | group lookup (0xA57730), then each member is handed back to the world (0xA5B6E0 / 0xA70030, virtual release) |
| 13 | `mission_debug` 0xA398C0 | 311 | (…) → 0 (number) | pushes 0; stripped |
| 14 | `thread_kill` 0xD745C0 | 304 | (thread id) → 0 | `script_thread_find` (0xCDFEB0), sets flag 4 (kill) at +0x18; the thread table removes it on its next pass |
| 15 | `group_show` 0xA2D7E0 | 253 | (group name) → 0 | `group_find_by_name` (0xA57730), `group_set_hidden(0)` (0xA58070) |
| 16 | `character_is_dead` 0xA35700 | 251 | (name) → boolean | name lookup (0xA52BB0, 0xA275F0 / 0xA27660, else the object's virtual +0x34), then `human_is_dead` (0x985630) |
| 17 | `audio_play` 0x7ECDF0 | 243 | (cue name, [type string], [volume], …) up to 3 → handle number | 0x7ECCA0 picks the cue, then the audio play path 0x465AB0 / 0x464920 / 0x465B90 (10_audio.md) |
| 18 | `vint_object_destroy` 0xB8F310 | 225 | (handle, [delay]) → 0 | finds the object; unlinks it from its parent (`vint_object_unlink_from_parent` 0xB97A20, skipped for an element of a document with no root) and destroys it (slot 2). The delay is converted and ignored |
| 19 | `objective_text` 0xA47050 | 179 | (tag, text id, [arg], [arg], [index]) up to 5 → 0 | formats the localized text into a 258-char buffer (0x7F4BC0), then the HUD objective (0x78E250) |
| 20 | `door_lock` 0xA2DED0 | 178 | (door name, [locked = true]) → 0 | the object lookup (0xA5B6E0), then 0x91B690 (sets the lock) |

**Porting order:**
1. The trivial ones: `thread_yield`, `debug_print`, `mission_debug`, `sizeof_table`, `coop_is_active`, `thread_kill`.
2. The name-lookup ones that set a flag or callback: `trigger_enable`, `on_trigger`, `on_death`, `group_show`, `door_lock`, `character_is_dead`.
3. The vint four (`vint_set_property`, `vint_get_property`, `vint_object_find`, `vint_object_destroy`). Together they are 64% of all binding call sites, and they run every frame in the HUD scripts, so they are where a port saves the most. They need the vint property table (06_vint.md) first.
4. `thread_new`, `audio_play`, `objective_text` and `release_to_world`, which reach into larger systems.

## Ported: the vint four

`crates/sr2-hook/src/vint_lua.rs` replaces `vint_set_property`, `vint_object_find`, `vint_get_property` and `vint_object_destroy`, plus `vint_lua_current_document` (0xB8E020) and `vint_object_unlink_from_parent` (0xB97A20). The Lua API and the vint value / property helpers stay the game's:

| Helper | Address | Convention |
|---|---|---|
| `string_crc32_lower` | 0xBDC9B0 | `ECX` = string → CRC-32 of the lowercased string (0 for NULL) |
| `vint_object_find_child_by_hash` | 0xB98340 | `ECX` = parent, (hash), `ret 4` |
| `vint_object_find_property` | 0xB978A0 | `ESI` = object, `ECX` = name → descriptor (value type at +4) |
| `vint_object_get_property_by_hash` | 0xB97680 | `ESI` = object, `EDI` = &value, (hash), `ret 4` → al |
| `vint_object_set_property_by_name` | 0xB97810 | `ESI` = object, `ECX` = name, (&value, extra), `ret 8` |
| `vint_value_init` / `_free` / `_assign` | 0xB93A10 / 0xB93790 / 0xB93620 | `EAX` = &value, `ECX` = type / `EAX` = &value / `ESI` = dst, `EDI` = src |
| `vint_value_from_lua` | 0xB8D200 | `EAX` = &typed value, cdecl (&scratch, L, &args left, 0) → &value read |
| `vint_value_push_lua` | 0xB8D8F0 | `EAX` = L, `EDX` = &value → results pushed |
| `vint_value_to_handle` | 0x634BA0 | `ECX` = &value |
| `lua_opt_int_arg` | 0x5575E0 | `EDI` = &args left, cdecl (L, 0, &default) |

`vint_set_property`, step by step: the handle is read only if argument 1 is a number. The property's value type comes from its descriptor. A tween's `start_value` / `end_value` (type 0xD) instead takes the type of the property it animates (the tween's target handle, slot 11, and its `target_property` string). The value is initialised to that type (types up to 0xB), read from the remaining arguments, and set with an optional trailing int.

Tested in game: HUD, pause menu and map, 727 / 532 / 110 / 77 calls in about 2 minutes; `vint_object_find` and `vint_get_property` matched the originals on every shadowed call.
