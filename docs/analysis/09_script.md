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

## Open

- The four hook setters (`0x252A640/644/668/66C`, written by 0x51F700, 0xA244D0 and 0xCDF780).
- What 0xCE0740 does (it has many callers in mission code; probably find or kill a thread by name).
