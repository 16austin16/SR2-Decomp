# 49: the vint system, value type, structures, names and two binding recipes

This document continues `06_vint.md` (what vint is, the frame path, rendering, input, tweens) and `43_bindings_41_80.md` (the Lua bindings). It fills the three gaps they left open:

1. **The vint value type** (`vint_value`): size, tags, payloads, and every operation on it, including the conversions to and from Lua and to text. With it, the two bindings that `docs/skipped_ports.md` lists as blocked (`luafn_vint_insert_values_in_string` 0xB908C0 and `luafn_vint_dataitem_get` 0xB8ECA0) have instruction-level recipes below.
2. **Names.** `docs/candidates_vint.csv` has 368 new vint functions that were missing from `docs/functions.csv` (334 confirmed, the rest inferred, see the status words in each note). They are ranked below by how much they matter. Four wrong or placeholder names found on the way are listed under "Bugs and doc errors".
3. **Fifteen easy port candidates** with full recipes (section "Port candidates").

Method: `tools/x86dis.py`, `tools/xrefs.py`, `tools/regin.py` plus a scratchpad recursive-descent disassembler (jump tables followed). Each address is a real function start (preceded by `int3`, `ret` or a jump table) and each caller count was checked against decoded call sites, not just byte matches. Nothing was run in game.

Conventions used in this file: `[x]` is a dword read, `+0x14` is a field offset, "AL" means only the low byte is the result. A "stdcall" function pops its own arguments (`ret N`), a "cdecl" one does not. Registers not listed are preserved unless a note says otherwise.

---

## 1. The vint value type

### 1.1 Layout

`vint_value` is **16 bytes**. It is a tagged union: a dword **tag** at +0, the payload from +4.

| Off | Content |
|---|---|
| +0 | `u32 tag` (0 = empty) |
| +4 | payload dword, or the first float |
| +8 | second float (tags 6, 7) |
| +0xC | third float (tag 6) |

Proof of the size: `luafn_vint_dataitem_get` builds the array with `vector_constructor_iterator(base, size = 0x10, count = 0x20, ctor = 0x7903C0)`; a dataitem is 0x22C bytes with 32 values at +0x14 (`0x14 + 0x20 * 0x10 = 0x214` = its value count field); `luafn_vint_insert_values_in_string` zeroes `0x20` tags at stride 0x10 and walks them with `shl ecx, 4`.

It is **not** a Lua `TValue` (those are also 16 bytes: type at +0, double at +8). The two layouts differ: the Lua type codes are 0 nil, 1 boolean, 3 number, 4 string, 5 table; vint tags are below.

### 1.2 Tags

| Tag | Meaning | Payload | Ownership |
|---|---|---|---|
| 0 | empty | none | |
| 1 | signed int | `i32` at +4 | |
| 2 | handle / unsigned / crc | `u32` at +4 | |
| 3 | float | `f32` at +4 | |
| 4 | string | `char*` at +4 | **borrowed**: never copied or freed by assign / free. Dataitems copy it with `vint_mem_realloc` (0xB8A2C0) and free it through the hook `[0x252A1E4]+4` |
| 5 | bool | `u8` at +4 | |
| 6 | colour | 3 x `f32` (r, g, b) at +4, +8, +0xC. **No alpha** | |
| 7 | vec2 | 2 x `f32` at +4, +8 | |
| 8 | event / callback | `u32` callback handle at +4 | refcounted `vint_callback` (`+0x1C` refcount, hash 0x22F2670) |
| 9 | bitmap | handle at +4 | refcounted record in `0x31BC518` (0xF0 x 0x24 bytes; handle +0x14, refcount +0x18) |
| 10 | font | handle at +4 | refcounted record in `0x31BC400` (0xA x 0x1C; handle +8, refcount +0x10) |
| 11 | **sound** (foley) | handle at +4 | refcounted record in `0x31BE700` (0x32 x 0x18; handle +8, refcount +0xC). **Missing from doc 06** |
| 12 | enum | `u32` at +4 | none. Copy only; `vint_value_equals` is always false; `vint_value_init` does not zero it |
| 13 | variant | n/a | **property-descriptor type only** (tween `start_value` / `end_value`); it never appears as a live value tag. Doc 06 lists it as a tag |

A handle is `index | generation << 16`; a record is valid when `(h & 0xFFFF) < table size` **and** the stored handle equals `h` (`cmp / setne / sub 1 / and` mask idiom, so the lookup returns 0 for a stale handle).

### 1.3 Operations

All addresses are in `functions.csv` already unless the name is in `candidates_vint.csv`.

| Op | Function | Registers / stack | Behaviour |
|---|---|---|---|
| construct | `vint_value_ctor` 0x7903C0 | ECX = this; returns EAX = ECX | `tag = 0`. Nothing else |
| destruct | `vint_value_free` 0xB93790 | EAX = &value | Tags 8 / 9 / 10 / 11 release the callback or resource (`0xBA0980` + `0xBA0850`, `0xB92D60`, `0xB92DD0`, `0xB92E50`); tags 9 / 10 / 11 zero the payload. **The tag is not cleared and the tag-8 payload is not zeroed**: every caller writes `[value] = 0` itself (the `luafn` cleanup loops do exactly that) |
| init | `vint_value_init` 0xB93A10 | EAX = &value, ECX = type | Only when the tag is 0: tag = type, payload zeroed for types 1..11 (float 0.0, byte 0, three or two floats 0.0, dword 0). Types 12+ set only the tag |
| copy / assign | `vint_value_assign` 0xB93620 | ESI = dst, EDI = src; returns EAX = ESI | **Type guard**: when `dst.tag != 0` and `src.tag != dst.tag` it returns **without assigning**. Otherwise frees dst, copies the tag, then by tag: 1..5, 12 copy the dword / byte, 3 goes through fld / fstp, 6 three dwords, 7 two dwords, 8 callback addref (`0xBA0980`, `+0x1C += 1`), 9 / 10 / 11 `vint_bitmap_addref` / `vint_font_addref` / `vint_sound_addref`. An empty source tag 0 leaves dst empty |
| compare | `vint_value_equals` 0xB93920 | EAX = a, EDX = b; AL = result | See the recipe in "Port candidates" |
| set | `vint_value_set_int` 0xB93590, `vint_value_set_handle` 0xB93490, `vint_value_set_name` 0xB93800, `vint_value_set_float` 0x7903D0, `vint_value_set_bool` 0x89BFD0, `vint_value_set_color3`, `vint_value_set_color_bytes`, `vint_value_set_vec2`, `vint_value_set_callback` 0xB93720, `vint_value_set_bitmap_record` 0xB93300, `vint_value_set_font_record` 0xB93340 | see the table in section 4 | Each only acts on tag 0 or its own tag (the exceptions, which convert, are `set_int` / `set_handle` / `set_name`) |
| get | `vint_value_to_int` 0x5E4930, `vint_value_to_float` 0x5E4970, `vint_value_to_double` 0xB8CF40, `vint_value_to_bool` 0x773120, `vint_value_to_handle` 0x634BA0, `vint_value_get_vec2`, `vint_value_get_color3`, `vint_value_get_color_bytes`, `vint_value_get_bitmap_record`, `vint_value_get_font_record` | ECX = &value (some EAX) | See section 4 |
| to text | `vint_value_to_text` 0xB93170 | EAX = &value, ECX = char* dest | `%i %u %g %s true/false %g,%g,%g %g,%g` for tags 1..7, empty otherwise |
| to Lua | `vint_value_push_lua` 0xB8D8F0 | EAX = lua_State*, EDX = &value; returns EAX = results pushed | Tags 1, 2: number (tag 2 unsigned via `vint_value_to_double`); tag 3: number; tag 4: string (`lua_pushstring`); tag 5: boolean; tag 6: **3 numbers**, each channel `byte / 255.0` after `vint_value_get_color_bytes` (so the colour is quantised to 8 bits); tag 7: 2 numbers; every other tag: one nil. Returns 1, 3 or 2 |
| from Lua | `vint_value_from_lua` 0xB8D200 | EAX = typed hint (a value whose tag selects the reader), cdecl (`vint_value* out`, `lua_State* L`, `int* args_left`, `bool no_advance`); returns EAX = out | See below |

`vint_value_from_lua` dispatch by the **hint's** tag (the value passed in EAX):

| Hint tag | Lua type read | Result |
|---|---|---|
| 0 (any) | number | integral and >= 0 -> tag 2 (`u32`); integral and < 0 -> tag 1; fractional -> tag 3 (`f32`). The test is `x == floor(x)` (`fucom`, `test ah,0x44 / jp`) then `0 <= x` (`fcomp`, `test ah,0x41 / jne`) |
| 0 (any) | string | tag 4, pointer into the Lua string (default `""` at 0xDC5797) |
| 0 (any) | anything else | stays tag 0 |
| 1, 2 | number | `vint_lua_opt_int`-style read with the current value as default (`vint_value_to_int`), `vint_value_set_int` |
| 3 | number | `vint_lua_opt_float` with the current value as default, `vint_value_set_float` |
| 4 | string / number / nil | string or the stored one, `vint_value_set_name` |
| 5 | boolean | `vint_lua_opt_bool`, `vint_value_set_bool` |
| 6 | 3 numbers (r, g, b: each `vint_lua_opt_float` x 255 truncated to a byte) or a string (colour name via `vint_color_by_name`) | `vint_value_set_color_bytes` / `vint_value_set_color3` |
| 7 | 2 numbers | `vint_value_set_vec2` |
| 8 | string | callback by name: `vint_callback_find_by_hash` (0xBA0950) else `vint_callback_create` (0xB8DBB0), then `vint_value_set_callback` and release |
| 9 | string (name) or number (handle) | `vint_bitmap_acquire_by_name` / `vint_bitmap_record_by_handle`, `vint_value_set_bitmap_record` |
| 10 | string or number | same with fonts |
| 11, 12 | number or string | tag 12: `lua_opt_int` default `0x7FFFFFFF`, or enum name through `vint_value_set_name`; tag 11 shares the "any" reader and `vint_value_set_name` (sound by name) |

After the switch the function always does `out.tag = 0; vint_value_assign(ESI = out, EDI = local); vint_value_free(local)`. Because the assign is guarded by the tag, `out` must be tag 0 or the same tag.

### 1.4 The `vector constructor iterator` array

`luafn_vint_dataitem_get` and `luafn_vint_insert_values_in_string` both keep 32 values on the C stack. The "constructor" is a one-instruction function, `0x7903C0` (`mov dword [ecx], 0`); there is **no destructor**. The cleanup is an explicit loop that runs `vint_value_free(EAX = p)` and `mov [p], 0` from the last element to the first. Rust can model the array as `[VintValue; 32]` of `{tag: u32, payload: [u32; 3]}` and call the originals by address.

### 1.5 Platform tables the value type depends on

| Global | Table | Used for |
|---|---|---|
| `[0x252A1D4]` (`0x1FD4FEC`) | resources | +0 bitmap lookup by name (0x7F0640), +4 acquire (0x7F06A0), +0xC size, +0x18 font unload, +0x1C / +0x20 / +0x24 font, +0x2C **sound by name** (0x7F05C0), +0x30 sound acquire (0x7F05E0), +0x34 sound release (0x7F0620) |
| `[0x252A1E0]` (`0x1FD509C`) | text / localisation | +0 `lstr_lookup` of an ascii id (0x7F0460), +4 by crc (0x7F0470), +8 crc-or-0 (0x7F0480), +0xC symbol (0x7EDC00), +0x10 to-lower (0x7ED840) |
| `[0x252A1E4]` | memory | +0 alloc, +4 free (used for tag-4 payloads in dataitems and lua-call records) |
| `[0x252A250]` | the "Vint" `virtual_mempool` | `vtable +0x1C` alloc(size, align), `+0x30` mark(), `+0x34` release_to(mark) |

---

## 2. Objects, properties, datagroups

### 2.1 Property descriptors and sets

A property descriptor is **0x24 bytes** (listed in static arrays, one per class):

| Off | Field |
|---|---|
| +0 | `char* name` |
| +4 | value type (tag, 1..13; 13 = variant) |
| +8 | **setter thunk** (cdecl `(obj, value*, extra)`) |
| +0xC | **getter thunk** (same signature) |
| +0x10, +0x14 | 0 |
| +0x18 | name hash (`string_crc32_lower`, filled at init by `vint_property_set_hash_names`) |
| +0x1C | index in the set (init-time) |
| +0x20 | flags; **0x100 = tweenable** |

A **property set** is `{descriptor array*, count, parent type index}`. `vint_property_set_find` (0xB98000) scans the set by hash, then the set of the parent type. The **type registry** at `0x22F2370` has 0x10 bytes per type: `{char* name, parent type index (or -1), property set*, factory*}`; count `[0x252A270]` (max 0x20).

A thunk is 0x1D / 0x20 bytes: `if (!obj) return 0; ECX = obj; push extra; push value; call obj->vtable[slot]` with the getter on the even slot and the setter on the next odd slot (names for every slot are in `vtable_slots.csv`). The method has `ret 8` and takes **the value pointer first**, the extra word second. A typical getter (`vint_element_base::get_alpha`, 0xB96860) is: tag 0 -> tag := 3 and store, tag 3 -> store, anything else -> leave the value alone, then `AL = 1`.

The descriptor arrays are all in `.data`; 156 distinct thunks were named (section 4).

### 2.2 Object pools and sizes

Each class has a pool created by its `*_pool_init`, with a ctor / dtor pair (names in `candidates_vint.csv`):

| Class | Object size | Slots | ctor | dtor | pool init | register |
|---|---|---|---|---|---|---|
| `vint_element_bitmap` | 0xc4 | 0x360 | 0xBA0BF0 | 0xBA0C20 | 0xBA14B0 | 0xBA0C90 |
| `vint_element_gradient` | 0x104 | 0x5 | 0xBA1870 | 0xBA18B0 | 0xBA23D0 | 0xBA1920 |
| `vint_element_text` | 0xf4 | 0x158 | 0xBA2790 | 0xBA2800 | 0xBA4350 | 0xBA2890 |
| `vint_element_point` | 0x98 | 0x30 | 0xBA46B0 | 0xBA46D0 | 0xBA4920 | 0xBA4720 |
| `vint_element_clip` | 0xa4 | 0x20 | 0xBA4C60 | 0xBA4C80 | 0xBA5120 | 0xBA4CD0 |
| `vint_element_bitmap_circle` | 0xcc | 0x40 | 0xBA5460 | 0xBA5490 | 0xBA5E30 | 0xBA5500 |
| `vint_element_sr2_map` | 0xa0 | 0x3 | 0xBA61A0 | 0xBA61C0 | 0xBA6610 | 0xBA6210 |
| `vint_element_video` | 0x9c | 0x1 | 0xBA6950 | 0xBA6970 | 0xBA6CE0 | 0xBA69C0 |
| `vint_object_animation` | 0x64 | 0xc4 | 0xB9CDB0 | 0xB9CDD0 | 0xB9D360 | 0xB9CE10 |
| `vint_element_group` | 0x98 | 0x14b | 0xB9C390 | 0xB9C3B0 | 0xB9CA60 | 0xB9C450 |
| `vint_object_tween` | 0xe8 | 0x1f0 | 0xB9E6B0 | 0xB9DC60 | 0xB9FF80 | 0xB9E720 |

The ctor writes `+0x28 = +0x2C = 0` and the class vtable; the base object layout (vtable, bucket links +4..+0x10, children +0x1C/+0x20/+0x24, handle +0x28, name hash +0x2C, document +0x30) is in doc 06, plus **+0x34: `char name[0x20]`** (set by `vint_object_set_name`, 0xB97620). Elements start at +0x54 (depth). `vint_element_bitmap` embeds a **vint_value** at +0x98 (tag 9 + handle at +0x9C): its ctor writes tag 9 / 0, its dtor runs `vint_value_free` on it.

### 2.3 Dataitems, datagroups, subscribers

**Dataitem** (0x22C bytes, 0x80 of them at `0x31A96F0`; live ring `[0x252A1F4]`, free ring `[0x252A1F8]`, links +8 / +0xC):

| Off | Field |
|---|---|
| +8 / +0xC | prev / next in the live (or free) ring |
| +0x10 | name hash |
| +0x14 | `vint_value values[32]` |
| +0x214 | value count (max of `index + 1` over `vint_dataitem_set_value`) |
| +0x21C | owner datagroup (0 = standalone) |
| +0x220 | subscriber list head (nodes, links +0x14 / +0x18) |
| +0x224 | handle (`index | gen << 16`, gen = `[0x252A1FC]`) |
| +0x228 / +0x229 | `u8` changed / `u8` new-or-dirty (both set at init) |

**Datagroup** (0x20 bytes, 0x20 of them at `0x31BACF0`; live ring `[0x252A204]`, free ring `[0x252A200]`, links +0 / +4):

| Off | Field |
|---|---|
| +8 | name hash |
| +0xC | **open list**: the items currently in the group (circular, item links +0 / +4) |
| +0x10 | **delivery queue**: items moved out of the open list |
| +0x14 | handle (gen `[0x252A208]`) |
| +0x18 | subscriber list head (kinds 3, 4, 5) |
| +0x1C | `u8 closed` (1 = delivered then destroyed by `vint_datagroups_deliver_all`) |

**Subscriber node** (ring free list `[0x252A20C]`, waiting list `[0x252A210]`): `+4` item or group handle, `+8` owner token (a document handle), `+0xC` callback handle, `+0x10` kind, `+0x14` / `+0x18` list links. Kinds seen: **2** dataitem subscriber (receives `callback->vtable[1](owner, 2, item, 0x22C)` once at subscribe and on each change), **3** group "item changed", **4** group "item added" (replayed to a new subscriber for existing items), **5** group "item removed / delivered". A callback is `{vtable, hash ring +4/+8, name hash +0x14, handle +0x18, refcount +0x1C}` in the 64-bucket table `0x22F2670`; vtable[1] = invoke(doc, kind, userdata, size), vtable[5] = free.

Flow: Lua `vint_datagroup_insert_item` -> `vint_datagroup_add_item` (0xB94290) -> `vint_dataitem_set_values`; game code calls `vint_datagroup_queue_item` (0xB942D0) to retire an item, `vint_datagroup_close` (0xB93B40) to retire the whole group; `vint_datagroups_deliver_all` (0xB94560) calls the kind-5 subscribers and frees the items.

### 2.4 Lua call records and documents

A `vint_lua_call` record is 0x260 bytes (32 of them at `0x31A4A80`): `+8` name hash, `+0xC` 32 args (`vint_value`), `+0x20C` arg count, `+0x210` handle, `+0x214` ok byte, `+0x21C` name copy, `+0x25C` native hook. Pool functions: `vint_lua_call_pool_init`, `_init`, `_find_by_handle`, `_find_by_name`, `_clear_args`, `_set_arg_types`, `_invoke_native` (plus the existing `_alloc` / `_set_arg` / `_execute` / `_free`).

Document extras recovered here: clip stack at +0x454.. (rect entries 0x10, depth at +0x4E4, 8 deep), metadata table at +0x4F0 (16 x 8 bytes, count word +0x570), priority at +0x574. XML reading order (`vint_document_parse_sections`): metadata, `critical_resources` / `pegs`, the `elements` and `animations` object lists; `_v2` also runs the `object_properties` / `animation_properties` / `baseline` override passes.

---

## 3. Recipes for the two blocked bindings

Both are cdecl `int f(lua_State* L)` (`ret` plain), frame `push ebp; mov ebp, esp; and esp, -8`. Offsets are relative to `ESP0`, the value of ESP **after** the prologue's three register pushes (`push ebx; push esi; push edi`). The callers' L is `[ebp + 8]`. `n = (L->top - L->base) >> 4` where `L+8` is top and `L+0xC` is base.

### 3.1 `luafn_vint_dataitem_get` 0xB8ECA0

Frame: `sub esp, 0x204`. Locals (ESP0-relative, after the three pushes): `[+0xC]` result count (starts 0), `[+0x10 .. +0x20F]` the 32 values (0x200 bytes). Registers: EDI = L, ESI = dataitem record, EBX = loop count.

1. `EAX = n`; `if (n >= 1) EAX = 1` (so `min(n, 1)`); `EAX = -EAX`; `ECX = L`; `lua_tonumber` (0xCDC250, ECX = L, EAX = stack index; result in ST0); `_ftol2` (0xDA0176) -> `h` in EAX. **The argument read is slot -1 (the last argument), also when n >= 2; with n == 0 it reads index 0.**
2. `if (h == 0) return 0`; `idx = (u16)h`; `if (idx >= 0x80) return 0` (unsigned `jae`); `rec = 0x31A96F0 + idx * 0x22C` (`imul ecx, ecx, 0x22C`); `if (rec->+0x224 != h) return 0`. Return value 0 means **no results**: nothing is pushed.
3. Construct: `vector_constructor_iterator(0x402450)` as `cdecl (array = ESP0+0x10, size = 0x10, count = 0x20, ctor = 0x7903C0)`; the caller pushes `0x7903C0, 0x20, 0x10, array` and does not clean (the helper is stdcall, `ret 0x10`).
4. Copy: `vint_dataitem_copy_values` (0xB94CD0): `EAX = destination array`, stdcall `(rec)`, `ret 4`. It assigns `min(rec->+0x214, 32)` values (`vint_value_assign`, ESI = dst, EDI = rec + 0x14 + i * 0x10).
5. `EBX = rec->+0x214` (the **uncapped** count; it is at most 32 because `set_value` rejects index >= 0x20). `if (EBX <= 0) skip`. Loop (count-down `sub ebx, 1 / jne`): `EAX = L; EDX = &values[i]; call vint_value_push_lua (0xB8D8F0); [ESP0+0xC] += EAX` (1, 2 for a vec2, 3 for a colour, nil = 1 for tags 8+); `ESI += 0x10`.
6. Cleanup: `ESI = ESP0 + 0x210; EDI = 0x1F; do { ESI -= 0x10; vint_value_free(EAX = ESI); EDI--; [ESI] = 0 } while (EDI >= 0)` (`jns`).
7. Return `[ESP0 + 0xC]` as the number of results.

Callee table:

| Callee | Convention |
|---|---|
| `lua_tonumber` 0xCDC250 | ECX = L, EAX = index; result ST0 |
| `_ftol2` 0xDA0176 | ST0 -> EAX (truncate) |
| `vector_constructor_iterator` 0x402450 | stdcall `ret 0x10`, pushed right to left as `(array, size 0x10, count 0x20, ctor 0x7903C0)` |
| `vint_dataitem_copy_values` 0xB94CD0 | EAX = out array, stdcall (rec), ret 4 |
| `vint_value_push_lua` 0xB8D8F0 | EAX = L, EDX = &value; returns EAX = results pushed |
| `vint_value_free` 0xB93790 | EAX = &value |

Port plan: the whole function is portable now as a Rust function that reads `h` exactly as above (use `lua_tonumber` from the existing binding code, `_ftol2` semantics = truncate toward zero), uses a `[VintValue; 32]`, calls the originals by address, and returns the count. The only hazards are the zero results path (return 0 without touching the stack) and the value of `h` for `n == 0`.

### 3.2 `luafn_vint_insert_values_in_string` 0xB908C0

Frame: `sub esp, 0x23C`. Locals (ESP0-relative):

| ESP0+ | Content |
|---|---|
| `[+0xC]` | `n = min(n_args, 2)` first, later the pool **mark** (overwrites n) |
| `[+0x10]` | loop index `i`; later `buf1` (expanded wide text, 0x2000 bytes) |
| `[+0x14]` | `tn`, the table entry count (`-1` when there is no table) |
| `[+0x18 .. +0x27]` | scratch hint `vint_value` (tag 0, "any"); later unused |
| `[+0x28 .. +0x37]` | **value A**: argument 1 as a vint_value (`A.tag` at +0x28, payload at +0x2C) |
| `[+0x38 .. +0x47]` | table-entry temp `T` |
| `[+0x48 .. +0x247]` | `values[32]`, zeroed (tag 0) by a 32-iteration loop |

Steps:

1. `n = (top - base) >> 4; if (n >= 2) n = 2` (`jl` skips the clamp when `n < 2`; signed).
2. **Read argument 1**: `A.tag = 0` (EBX = 0 is stored at the hint); `vint_value_from_lua(EAX = &hint, cdecl (&A, L, &n, 1))`, `add esp, 0x10`; then `vint_value_free(&hint)`. The hint tag is 0, so the reader takes any number (-> tag 1 / 2 / 3) or any string (-> tag 4); anything else leaves `A.tag = 0`. The reader decrements `n` when it consumes a slot (so a following table is at index -1 / `n == 1`).
3. Zero `values[32]`.
4. **Count the table**: `tn = (n > 0) ? lua_table_count(EAX = n, cdecl (L)) : -1`. (`lua_table_count` 0x7762E0 reads the table at index `-n`.)
5. **Copy entries**: `i = 0; while (i < tn)`: `vint_value_from_table_entry(EAX = &hint(tag 0), cdecl (&T, L, n, &i))` (0xB91630: pushes `i` as a number, `lua_gettable` on index `-1 - n`, converts, pops); `ESI = &values[i]` (`EBX = i << 4`, base `ESP0 + 0x48`), `EDI = &T`, `vint_value_assign` (0xB93620); free `T`, `T.tag = 0`; free hint, `hint.tag = 0`; `i++`. **There is no clamp to 32**: a table with more than 32 entries writes past `values[]` onto the frame. The key is `i` itself (**0-based**: `t[0] .. t[tn-1]`), while `tn` is the table's `n` field or `lua_next` pair count. A port must clamp to 32 (or reproduce the overrun, which is a crash in the original).
6. `mark = pool->vtable[+0x30]()` where `pool = [0x252A250]`, `ECX = pool`, no stack args, `EAX = used`; stored in `[+0xC]`.
7. **Choose the format string** (`edi = A.tag`, `esi = 0`):
   - tag 1 or 2 (string id): `hook = [0x252A1E0]`; `if (hook != 0 && hook->+4 != 0)`: `h = vint_value_to_handle(ECX = &A)` (0x634BA0), `esi = hook->+4(h)` (cdecl one arg, `add esp, 4`). **If `esi == 0` (or the hook is missing): release and return 0 results**: `pool->vtable[+0x34](mark)` (ECX = pool, one stack arg, callee pops), free the 32 values (descending, `mov [p], 0`), free A, return 0. This is the `0 results` path doc 43 mentions.
   - tag 4 (string) or anything else: `esi` stays 0.
8. `buf1 = pool->vtable[+0x1C](0x2000, 0)` (ECX = pool, stack `(size, align)`, **callee pops 8**), stored in `[+0x10]`.
9. **If `esi != 0`** (id path): `vint_insert_values_expand(buf1, esi, 0x1000, &values[0], tn)` (cdecl, 5 args, `add esp, 0x14`); wide characters, so the capacity is 0x1000 wchars = the 0x2000 bytes allocated.
10. **Else (string path)**: `buf2 = pool->vtable[+0x1C](0x800, 0)`; `src = (A.tag == 4) ? A.payload : 0` (mask built with `cmp edi,4 / setne al / sub eax,1 / and eax, payload`: `A.tag != 4` gives 0). Then:
    - `vint_decode_wide_string(EAX = src, EDX = buf2, cdecl (0x400))` (0xB92670); if AL != 0 the string was in the 0x80-packed format: the wide text is in `buf2`.
    - else: if `hook != 0 && hook->+0 != 0`: `w = hook->+0(src)` (cdecl, `lstr_lookup`); if `w != 0`, call it **again** (same result) and use it; otherwise widen the bytes of `src` into `buf2` with a 4x unrolled loop (`movzx si, byte` / store word) up to 0x400 chars and force `buf2[0x3FF] = 0`.
    - `vint_insert_values_expand(buf1, wide_src, 0x1000, &values[0], tn)`.
11. **Encode and push**: `buf3 = pool->vtable[+0x1C](0x2000, 0)`; `vint_encode_wide_string(EAX = buf3, ECX = buf1, cdecl (0x2000))` (0xB925F0); `ESI = L; lua_pushstring(cdecl (buf3))` (0xCDCAF0 takes L in ESI).
12. `pool->vtable[+0x34](mark)` (read from `[ESP+0x10]` while the pushstring argument is still on the stack, then `add esp, 4`).
13. Free the 32 values from the last to the first (`vint_value_free`, tag := 0), free A, return **1**.

Callee table:

| Callee | Convention |
|---|---|
| `vint_value_from_lua` 0xB8D200 | EAX = hint, cdecl (out, L, &args_left, no_advance) -> EAX = out |
| `lua_table_count` 0x7762E0 | EAX = k (index from the top), cdecl (L) -> count / -1 |
| `vint_value_from_table_entry` 0xB91630 | EAX = hint, cdecl (out, L, index, &i) -> EAX = out |
| `vint_value_assign` 0xB93620 | ESI = dst, EDI = src |
| `vint_value_free` 0xB93790 | EAX = &value |
| `vint_value_to_handle` 0x634BA0 | ECX = &value -> EAX |
| `vint_decode_wide_string` 0xB92670 | EAX = packed src, EDX = wide dst, cdecl (cap chars) -> AL |
| `vint_insert_values_expand` 0xB926D0 | cdecl (out, fmt, cap_chars, values, count) -> EAX = chars written |
| `vint_encode_wide_string` 0xB925F0 | EAX = out, ECX = wide src, cdecl (cap bytes) |
| `lua_pushstring` 0xCDCAF0 | ESI = L, cdecl (char*) |
| pool `[0x252A250]` vtable | +0x30 mark (ECX = pool; EAX = used), +0x1C alloc (ECX = pool, stack (size, align), `ret 8`), +0x34 release_to (ECX = pool, stack (mark), `ret 4`) |
| localisation hooks `[0x252A1E0]` | +0 text by id string, +4 text by id handle; both cdecl one argument, return a wide string or 0 |

Temp allocator marks: three allocations (0x2000, 0x800 on the string path only, 0x2000) between one `mark()` and one `release_to(mark)`; the early-exit path releases without allocating.

Port plan: portable now. Keep the pool calls and the localisation hooks as real calls (they are the game's), build `values[32]` as an array, clamp `i` to 32 (document it as a deliberate fix), and copy the result handling exactly (return 0 results only on the id path with no text; every other path pushes one string). Check with the shadow harness on a call from the HUD (text with `%s`).

---

## 4. Names

Everything below is in `docs/candidates_vint.csv` (368 rows: 212 hand-analysed, 156 property thunks, 25 of them the 25 tween interpolators). The status column there is `named` or `documented`; every note ends with `Confirmed` or `Inferred`.

### 4.1 The ten most important

| Address | Name | Why |
|---|---|---|
| 0x5E4930 / 0x773120 / 0x5E4970 | `vint_value_to_int` / `vint_value_to_bool` / `vint_value_to_float` | the vint value getters used all over HUD and mission code; 0x773120 was misnamed `quit_game_dialog` |
| 0x7903D0 / 0x89BFD0 / 0x7903C0 | `vint_value_set_float` / `vint_value_set_bool` / `vint_value_ctor` | the setters and constructor (0x7903D0 was `hud_console_toggle`) |
| 0xB93590 | `vint_value_set_int` | 39 callers |
| 0xB94FE0 | `vint_dataitem_get_or_create` | 35 callers: the HUD's way to publish data |
| 0xB93B40 | `vint_datagroup_close` | 35 callers; explains the "validate handle then call" idiom in game code |
| 0xB94140 | `vint_datagroup_create` | 28 callers |
| 0xB93800 | `vint_value_set_name` | 28 callers; the string -> bitmap / font / sound / colour resolver |
| 0xB9DD80 .. 0xB9E5F0 | `vint_ease_*` | the 25 tween interpolators behind the `algorithm` enum |
| 0xB926D0 | `vint_insert_values_expand` | the `%s` / `{N}` text expander behind `insert_values` (element text and the Lua binding) |
| 0xB9BD20 / 0xB9B6F0 / 0xBA03F0 | `vint_*_acquire_by_name` | bitmap, font and sound resource caches |

### 4.2 Hand-analysed functions

| Address | Name | Notes (conventions and evidence) | Status |
|---|---|---|---|
| 0x005e4930 | `vint_value_to_int` | ECX = &value; returns EAX = int: tags 1 and 2 the payload dword, tag 3 the float truncated (fld, tail jump to _ftol2_sse 0xDA0140), tag 5 bool -> -1 / 0 (neg / sbb), every other tag 0. Called from vint_value_from_lua (default for integer arguments) and by HUD / mission code (many callers) | Confirmed |
| 0x005e4970 | `vint_value_to_float` | ECX = &value, result in ST0: tag 1 fild signed, tag 2 unsigned (+ 2^32 from [0xE5BFD8] when negative), tag 3 float, any other tag 0.0. Twin of vint_value_to_double (different constant address, so not folded) | Confirmed |
| 0x00751ef0 | `vint_lua_opt_bool` | EDI = lua_State* (registers swapped compared to the other opt readers), cdecl (int* args_left, bool force, u8* default), returns AL: nil / missing -> *default, Lua type 1 (boolean) with payload 0 -> false, other non-nil -> true. Consumes one slot. Used by vint_value_from_lua for bools | Confirmed |
| 0x00751fb0 | `vint_lua_opt_string` | EDI = &args_left, cdecl (lua_State* L, bool force, const char** default), returns EAX = char* (lua_tostring of slot -args_left; consumes one slot; nil / missing -> *default). Used by vint_value_from_lua for strings, names and callbacks | Confirmed |
| 0x007730a0 | `vint_lua_opt_float` | EDI = &args_left, cdecl (lua_State* L, bool force, float* default), result in ST0 (f32): same running-count discipline as script_lua_opt_int; used for colour channels and floats in vint_value_from_lua | Confirmed |
| 0x007903c0 | `vint_value_ctor` | ECX = this, returns EAX = ECX: tag = 0 (the whole constructor). Passed to vector_constructor_iterator (0x402450) for the 32-value arrays of luafn_vint_dataitem_get. There is no destructor: callers loop vint_value_free and clear the tag | Confirmed |
| 0x0089bfd0 | `vint_value_set_bool` | EAX = &value, stdcall (u8 b), ret 4: only when the tag is 0 (becomes 5) or 5, payload byte = b | Confirmed |
| 0x00b8a0f0 | `vint_value_set_color_bytes` | EAX = &value, stdcall (u32 packed rgb), ret 4. Only when the tag is 0 or 6: tag = 6, r / g / b = byte0 / 1 / 2 as double divided by 255.0 ([0xE5C038]) stored as f32. Byte 3 ignored | Confirmed |
| 0x00b8a150 | `vint_value_set_vec2` | EAX = &value, EDX = float[2]*, no return. Only when the tag is 0 or 7: tag = 7 and payload x, y | Confirmed |
| 0x00b8a2a0 | `vint_property_call_setter` | EAX = descriptor*, stdcall (object*, vint_value*), ret 8: calls the setter thunk (+8 of the descriptor, cdecl 3 args); returns AL (0 when there is no setter). Caller: vint_tween_step | Confirmed |
| 0x00b8a8e0 | `vint_object_create_in_document` | EAX = type index, EDI = document*, cdecl (const char* name); returns EAX = new object* or 0: type registry 0x22F2370[type].factory vtable[1] (alloc) then 0xB971E0 (set name and document). Callers: root animation and safe frame setup (0xB8B5BB, 0xB8B5D3) | Confirmed |
| 0x00b8a980 | `vint_document_destroy` | EAX = document*; returns EAX: run_unload_script(doc), hook [0x252A1EC](doc), vint_document_release_pending(). Callers: load error paths, vint_document_load_with_dependencies | Confirmed |
| 0x00b8ae10 | `vint_document_parse_object_list` | EAX = xml node, ECX = list tag name ('elements' / 'animations'), stdcall (document*), ret 4: finds the child list and calls vint_object_parse_xml (0xB984E0) per 'object' | Inferred |
| 0x00b8ae80 | `vint_document_parse_critical_resources` | EAX = xml node, stdcall (document*), ret 4: 'critical_resources' / 'pegs' / 'filename' children, each peg name recorded for pre-loading | Confirmed |
| 0x00b8af20 | `vint_document_parse_metadata` | EAX = xml node: children 'metadata' / 'metadata_item' with attribute 'name', each stored in the document's table (+0x4F0, 16 entries of 8 bytes, count word +0x570) by 0xB8B030 | Confirmed |
| 0x00b8b030 | `vint_document_add_metadata` | ESI = string, EDI = document*: if the count word +0x570 < 0x10 copies the string into entry +0x4F0 + n * 8 and increments the count | Confirmed |
| 0x00b8b0c0 | `vint_document_free_metadata` | stdcall (document*): frees each stored metadata string through [0x252A1E4]+4 and zeroes the count +0x570. 3 callers (document teardown) | Inferred |
| 0x00b8b130 | `vint_document_parse_sections` | EAX = document*, EDI = xml node: runs metadata (0xB8AF20), critical resources (0xB8AE80), then the 'elements' and 'animations' object lists (0xB8AE10). Flat format | Inferred |
| 0x00b8b170 | `vint_document_load_file` | EAX = document*, stdcall (path, pool), ret 8: file_get_size + hdd_read_file_sync into a pool buffer, then vint_document_create. Returns AL | Inferred |
| 0x00b8b3f0 | `vint_document_find_by_name` | ECX = const char* name (0 -> 0); crc32_lower then walks the live document ring 0x252A1AC (link +0) comparing the name hash +0x48; returns EAX = document* or 0. 18 callers (luafn_vint_document_find, HUD, missions) | Confirmed |
| 0x00b8b450 | `vint_document_load_with_dependencies` | cdecl (const char* name): finds the project entry (0xB9AF50), allocates a document, loads it (0xB8B170), parses 'document_depth' and every 'doc_dependency' (recursion into itself), starts the Lua script via 0xB91330. Uses the pool [0x252A250] | Inferred |
| 0x00b8b580 | `vint_document_init_roots` | EAX = name, ECX = document*: copies the name into +0x28 (0x20 chars), hash into +0x48, and creates the 'root_animation' and root group objects with vint_object_create_in_document | Inferred |
| 0x00b8b650 | `vint_documents_init` | No args: runs vint_document_release_pending over the 12 records 0x319F6B0 (stride 0x57C), clears 0x252A1A8 / 0x252A1AC and rebuilds the free ring. Caller: vint_init | Inferred |
| 0x00b8b860 | `vint_document_clip_push` | ECX = int rect[4]*, stdcall (document*), ret 4: pushes a clip rectangle on the document's clip stack (up to 8 entries at +0x454.., depth +0x4E4), intersecting it with the previous one. Caller: vint_element_clip::render (0xBA4F1E) | Inferred |
| 0x00b8b990 | `vint_document_clip_pop` | EDX = document*: depth +0x4E4 -= 1; at 0 tail-calls the platform clip-end slot [0x252A1D8]+0x14 | Inferred |
| 0x00b8b9e0 | `vint_documents_unload_all` | No args: walks the live ring; for each document: vint_document_run_unload_script, the hook [0x252A1EC], vint_document_release_pending | Confirmed |
| 0x00b8cc60 | `vint_document_parse_sections_v2` | EAX = document*, EDI = xml node: metadata, critical resources, elements (0xB8CA90), animations (0xB8CB90) and two property override passes (0xB8C3A0, strings 'object_properties' / 'animation_properties' / 'baseline') | Inferred |
| 0x00b8ccc0 | `vint_stream_read_u32` | ECX = stream {base, size, cur}, stdcall (u32* out), ret 4; returns AL: false when cur + 4 >= base + size (strict), else little-endian u32 and cur += 4. 16 callers (binary vint documents) | Confirmed |
| 0x00b8cda0 | `vint_stream_read_u32_b` | same contract as vint_stream_read_u32 (compiled twin, cur stored before the byte shuffle). 13 callers | Confirmed |
| 0x00b8cdf0 | `vint_stream_read_u8` | EAX = stream, stdcall (u8* out), ret 4; returns AL: false when cur + 1 >= base + size (the last byte is not readable), else one byte | Confirmed |
| 0x00b8ce20 | `vint_args_consume` | EAX = &args_left, cdecl (u8 force): decrements *EAX when force != 0 or *EAX > 0; returns AL = [0xDC59F5]. Used by the string path of vint_value_from_lua to skip a slot | Inferred |
| 0x00b8cf10 | `vint_value_set_color3` | EAX = &value, EDX = float[3]*, no return. Only when the tag is 0 or 6: tag = 6 and copy three dwords to +4, +8, +0xC. Caller: Lua colour-name reader (after 0xB92130) | Confirmed |
| 0x00b8cf40 | `vint_value_to_double` | ECX = &value, result in ST0 (no stack args): tag 1 fild signed, tag 2 unsigned (fild, + 2^32 from [0xE5C0E0] when negative), tag 3 float, any other tag 0.0. Used by vint_value_push_lua for tags 1 and 2 | Confirmed |
| 0x00b8cf80 | `vint_value_get_vec2` | ECX = &value, EAX = float[2] out, no return: writes (0, 0) then the payload x, y when the tag is 7. 3 callers in the Lua readers | Confirmed |
| 0x00b8cfa0 | `vint_value_get_color_bytes` | ECX = u8[4] out, EDX = &value; returns EAX = ECX. Zeroes the four bytes; for tag 6 stores trunc(r*255), trunc(g*255), trunc(b*255) (x87 control word forced to chop, 0xC00, restored per channel) and alpha byte 0xFF. The payload has no alpha | Confirmed |
| 0x00b8d100 | `vint_subscription_event_name` | ECX = subscription*: kind +0x24 == 0 -> name = [0x252A238][event index +0x2C] (event < 0x22); kind 1 -> raw binding name (0x252A240 + [+0x30] * 0x18 + 0x14); else 0. Caller: a Lua binding (0xB8DDEC) | Confirmed |
| 0x00b8d150 | `vint_subscription_value` | ECX = subscription*; returns ST0: kind == 1 -> f32 +0x2C (raw value), else the constant [0xE87998] (-1.0). Caller: Lua binding (0xB8DE2F) | Confirmed |
| 0x00b91330 | `vint_document_start_lua` | EDI = document*, cdecl (xml node): reads the 'lua_script_file' attribute, builds '%s_init', looks the Lua function up (0xCE0120) and starts a script thread with script_thread_create / script_thread_resume_once (thread +0x14 = document handle) | Confirmed |
| 0x00b915a0 | `vint_lua_opt_number` | EDI = &args_left (int*), cdecl (lua_State* L, bool no_advance, double* default), double returned in ST0: reads Lua slot -args_left; nil / missing -> *default; consumes one slot. 1 caller (vint_value_from_lua any-number path); twin of script_lua_opt_int | Confirmed |
| 0x00b91630 | `vint_value_from_table_entry` | EAX = typed value (type hint), cdecl (vint_value* out, lua_State* L, int index, int* position): pushes position as a number, indexes the table at -index (0xCDC7E0), calls vint_value_from_lua(EAX, scratch, L, &args=1, no_advance=1) on the result, pops it, copies to out. Per-entry reader of the insert_values table | Confirmed |
| 0x00b91740 | `vint_lua_call_init` | ESI = record, ECX = name: +0x20C = 0, +8 = crc32_lower(name), +0x210 handle = (ESI - 0x31A4A80) / 0x260 + (++[0x252A1CC] << 16) | Confirmed |
| 0x00b91850 | `vint_lua_call_invoke_native` | ESI = record, EAX = name / source pointer (0 -> flag only), stdcall (a, b, c), ret 0xC: sets +0x214 = 1; when the name is non-NULL and +0x25C is non-zero copies the name into +0x21C and calls the native hook +0x25C(a, b, c), storing its result byte in +0x214. Caller 0xB8F036 | Inferred |
| 0x00b91a90 | `vint_lua_call_pool_init` | No args: clears the active ring 0x252A1C8 and chains the 0x20 lua-call records (0x31A4A80, stride 0x260) into the free ring 0x252A1C4. Caller 0xB9158F | Confirmed |
| 0x00b91c20 | `vint_lua_call_find_by_name` | ECX = const char* name; crc32_lower, walk the ring 0x252A1C8 (link +0) comparing +8; returns EAX = record or 0. Caller 0xB90629 | Confirmed |
| 0x00b91c60 | `vint_lua_call_find_by_handle` | ECX = handle; returns EAX = lua-call record (0x31A4A80 + (h & 0xFFFF) * 0x260) when h != 0, index < 0x20 and +0x210 == h, else 0 | Confirmed |
| 0x00b91c90 | `vint_lua_call_set_arg_types` | EDI = count, stdcall (int* types, record*), ret 8: for i < count initialises value i (record + 0xC + i * 0x10) to types[i] when its tag is 0, then +0x20C = count. 18 callers | Confirmed |
| 0x00b91cd0 | `vint_lua_call_clear_args` | stdcall (record*), ret 4: for the 32 values at +0xC: string payloads (tag 4) go through the free hook [0x252A1E4]+4, then vint_value_free, tag = 0; +0x20C count = 0. 34 callers | Confirmed |
| 0x00b91e70 | `vint_parse_decimal_wide` | ESI = wide string at the digits; returns EAX = value of the leading run of '0'..'9' chars (scans to the last digit, then accumulates right to left with place value). No error handling. Caller: vint_insert_values_expand ({N} index) | Confirmed |
| 0x00b92090 | `vint_color_table_add` | EDI = wide name (wchar*), stdcall-ish (float[3]* rgb on the stack, callee does not pop), no return: when [0x252A1D0] < 0x40 copies the name into the pool 0x31A9680 (static_mempool_base::alloc) and appends {name, r, g, b}. Caller 0x7F2FED (game side colour registration) | Confirmed |
| 0x00b92130 | `vint_color_by_name` | EAX = const char* name (0 -> 0); lower-cases it into a 0x100-byte stack buffer (0xC14DF0, EDX = 0x100) and returns 0xB92170(buf, -1): EAX = float[3]* of the named colour or 0. 3 callers (vint_value_set_name, Lua reader, xml property parse) | Confirmed |
| 0x00b92170 | `vint_color_table_find` | cdecl (const char* name, int len): scans the colour table 0x22ECFB8 (count [0x252A1D0], 0x10 bytes each {name, r, g, b}); len < 1 means a full-string compare (0xBF2D10), else a prefix compare of len chars (0xBF2D80); returns EAX = &entry.rgb (entry + 4) or 0 | Confirmed |
| 0x00b921e0 | `vint_loc_lookup_name` | ECX = ascii id; cdecl wrapper of the [0x252A1E0] hook +0 (0x7F0460 -> lstr_lookup): returns EAX = wide text or 0 (also 0 when the hook table is missing). Used for 'text_tag' items | Confirmed |
| 0x00b92200 | `vint_loc_lookup_id` | ECX = string id (crc); hook +4 (0x7F0470 -> lstr_find_by_crc); returns EAX = wide text or 0 | Confirmed |
| 0x00b92240 | `vint_loc_to_lower` | ECX = u16 char; hook +0x10 (0x7ED840) when present, else for ch <= 0xFF the table 0xDC81D8[ch]; returns EAX = lower-cased char. Callers: text layout case forcing | Confirmed |
| 0x00b92270 | `vint_loc_lookup_symbol` | ECX = length, EDX = wide name ptr; both pushed (cdecl) to hook +0xC (0x7EDC00); returns EAX = wide text or 0. Used for '{name}' items in vint_insert_values_expand | Confirmed |
| 0x00b92670 | `vint_decode_wide_string` | EAX = encoded src (must start with 0x80), EDX = wide output buffer, cdecl (u32 cap in chars); returns AL = 1 on success, 0 when src / dst is NULL or the header is not 0x80. Inverse of vint_encode_wide_string; writes a terminating 0 char. 5 callers (insert_values, text set) | Confirmed |
| 0x00b926d0 | `vint_insert_values_expand` | cdecl (wchar* out, wchar* fmt, u32 cap_chars, vint_value* values, u32 count) -> EAX = chars written (0 when out / fmt NULL or cap 0); 0x3028-byte frame (__alloca_probe). Expands '%s' / '%S' (next positional value as text), '%%', '{{', '{N}', '{N:mod}', '{}' and '{name}' (loc symbol); the mod strings are 'text_tag_crc' (index 0) and 'not_localized' (index 1) from 0xE86F48; a 0x0001 char toggles raw mode. Error texts: {No insertion value specified!}, {Hash not found!}, {Invalid insertion value for a...}, {Symbol not found!}. Callers: luafn_vint_insert_values_in_string, vint_element_text layout, 0x54662F | Confirmed |
| 0x00b92cc0 | `vint_vec2_equal` | ECX = float[2] a, EDX = float[2] b; returns EAX = 1 when both components compare equal (fucompp, test ah,0x44 / jp: NaN is not equal), else 0. vint_value_equals tag 7 | Confirmed |
| 0x00b92cf0 | `vint_vec3_equal` | ECX = float[3] a, EDX = float[3] b; returns EAX = 1 when all three components are equal (same idiom as vint_vec2_equal), else 0. vint_value_equals tag 6 | Confirmed |
| 0x00b92d30 | `vint_bitmap_addref` | ECX = bitmap handle: if h != 0, (h & 0xFFFF) < 0xF0 and 0x31BC518[idx].handle (+0x14) == h then record +0x18 += 1. No return. Called by vint_value_assign for tag 9 | Confirmed |
| 0x00b92d60 | `vint_bitmap_release_field` | ESI = &handle field: validates the handle as above and calls record vtable[1] (vint_resource_bitmap::vf1 = release, ECX = record), then stores 0 into [ESI]. Used by vint_value_free (tag 9) and the setters | Confirmed |
| 0x00b92da0 | `vint_font_addref` | ECX = font handle: valid when h != 0, (h & 0xFFFF) < 0xA and 0x31BC400[idx * 0x1C].handle (+8) == h -> record +0x10 += 1. vint_value_assign tag 10 | Confirmed |
| 0x00b92dd0 | `vint_font_release_field` | EDI = &handle field: valid handle -> ESI = record, call 0xB9B8C0 (record release); then [EDI] = 0. vint_value_free tag 10 | Confirmed |
| 0x00b92e20 | `vint_sound_addref` | ECX = sound handle: valid when h != 0, (h & 0xFFFF) < 0x32 and 0x31BE700[idx * 0x18].handle (+8) == h -> record +0xC += 1. vint_value_assign tag 11 | Confirmed |
| 0x00b92e50 | `vint_sound_release_handle` | ECX = sound handle: valid handle -> ESI = record, call 0xBA0570 (record release). vint_value_free tag 11 | Confirmed |
| 0x00b93170 | `vint_value_to_text` | EAX = &value, ECX = char* dest (0 -> no-op), ESI forced to 0x1000 (snprintf size, 0xBF2C10 takes ESI = size). Formats: tag 1 '%i', 2 '%u', 3 '%g', 4 '%s', 5 'true' / 'false', 6 '%g,%g,%g', 7 '%g,%g'; other tags write an empty string. Caller: vint_insert_values_expand | Confirmed |
| 0x00b93290 | `vint_value_reset_payload` | ESI = &value, no args: vint_value_free then zero the payload of the current tag (0 for ints / pointers, 0.0f for tag 3, byte 0 for tag 5, 3 floats for tag 6, 2 floats for tag 7); the tag is kept. 3 callers (0xB9E8A0..B6, tween state helpers) | Confirmed |
| 0x00b93300 | `vint_value_set_bitmap_record` | EDI = &value, stdcall (bitmap record* or 0), ret 4; returns EAX = EDI. Tag 0 -> 9, tag 9 -> releases the old handle first (0xB92D60), any other tag unchanged; stores record +0x14 (handle) and refcount +0x18 += 1, 0 for NULL. 4 callers in the bitmap property setters | Confirmed |
| 0x00b93340 | `vint_value_set_font_record` | ESI = &value, stdcall (font record* or 0), ret 4; returns EAX = ESI. Tag 0 -> 10, tag 10 -> release old (0xB92DD0); stores record +8 (handle) and refcount +0x10 += 1. Callers: text property setter, Lua reader | Confirmed |
| 0x00b933e0 | `vint_value_get_bitmap_record` | EAX = &value; returns EAX = bitmap record* (0x31BC518 + (h & 0xFFFF) * 0x24) when the tag is 9, h != 0, (h & 0xFFFF) < 0xF0 and record +0x14 == h; otherwise 0 (cmp / setne / sub 1 / and mask). Pure getter | Confirmed |
| 0x00b93420 | `vint_value_get_font_record` | EAX = &value; returns EAX = font record* (0x31BC400 + idx * 0x1C) when the tag is 10, h != 0, idx < 0xA (16-bit compare) and record +8 == h; otherwise 0. Pure getter | Confirmed |
| 0x00b93490 | `vint_value_set_handle` | stdcall (&value, u32 h), ret 8; returns EAX = &value. Switch on the tag: 0 or 2 -> tag 2 and store, 1 -> store, 3 -> float (unsigned: fild + 2^32 when negative), 9 / 10 / 11 -> look the record up by handle (0xB9BE50 / 0xB9B860 / 0xBA0540), release the old one, addref the new one (0 when unknown), 4..8 untouched. 17 callers | Confirmed |
| 0x00b93590 | `vint_value_set_int` | ESI = &value (implicit), stdcall (int v), ret 4; returns EAX = ESI. Switch on the current tag: 0 or 1 -> tag 1 and store, 2 -> store only, 3 -> float (fild), 8 -> callback rebind by handle (release old via 0xBA0850, addref new, 0 if unknown), 4..7 untouched. 39 callers (Lua int / enum arguments, HUD and mission code) | Confirmed |
| 0x00b93720 | `vint_value_set_callback` | EAX = &value, EDX = vint_callback* (may be 0), no return. Only when the tag is 0 or 8: tag = 8, payload = callback +0x18 (handle), refcount +0x1C += 1; 0 stores handle 0. Does not release the previous callback | Confirmed |
| 0x00b93800 | `vint_value_set_name` | stdcall (&value, const char* s), ret 8; returns EAX = &value. Switch on the tag: 0 -> tag 4 and store the pointer (not copied), 4 -> store the pointer, 6 -> colour by name (0xB92130, 3 floats), 9 -> bitmap by name (0xB9BD20), 10 -> font (0xB9B6F0), 11 -> sound (0xBA03F0); the old handle is released and an unknown name stores 0; other tags untouched. 28 callers | Confirmed |
| 0x00b93b00 | `vint_datagroup_init` | ESI = datagroup*, ECX = name: +0x18 = 0, +0xC = 0, +8 = crc32_lower(name), +0x14 handle = ((ESI - 0x31BACF0) >> 5) + (++[0x252A208] << 16), byte +0x1C = 0 | Confirmed |
| 0x00b93b40 | `vint_datagroup_close` | EDX = datagroup* (callers check handle, index < 0x20, +0x14 == h and byte +0x1C == 0 first): moves every item from the open list +0xC to the delivery queue +0x10 (circular-list splice) and sets byte +0x1C = 1; vint_datagroups_deliver_all then delivers and destroys the group. 35 callers, always followed by clearing the stored handle | Confirmed |
| 0x00b93bc0 | `vint_datagroup_destroy` | cdecl (datagroup*): releases every subscriber node in the list +0x18 (callback refcount, node back to the ring 0x252A20C), then returns the group to the free ring 0x252A200. Called by vint_datagroups_deliver_all after delivery (also at 0xB946CD) | Confirmed |
| 0x00b93da0 | `vint_datagroup_first_item` | EAX = datagroup*; returns EAX = 0 when closed (byte +0x1C == 1) else the open-list head (+0xC). 3 callers (0x834F35, 0x84131F, 0x891775) | Confirmed |
| 0x00b93ee0 | `vint_datagroup_unsubscribe` | ECX = datagroup*, EDI = kind, EDX = owner, stdcall (u32 callback_handle), ret 4: finds the node with +0xC == handle, +0x10 == kind, +8 == owner in list +0x18, unlinks it, releases the callback (vtable[5] on the last reference) and returns the node to the free ring 0x252A20C. Caller: luafn_vint_datagroup_remove_subscription (0xB8E4B9) | Inferred |
| 0x00b93fd0 | `vint_datagroup_pool_init` | No args: empties the rings and chains the 0x20 datagroups (0x31BACF0, stride 0x20) into the free ring 0x252A200, resets 0x252A204 / 0x252A20C / 0x252A210 and the input-event timing table 0x22EE3B8. Called from vint_init | Inferred |
| 0x00b94140 | `vint_datagroup_create` | ECX = const char* name (0 -> 0); returns EAX = datagroup* (0x20 bytes at 0x31BACF0 + i * 0x20): takes the head of the free ring 0x252A200, links it into the live ring 0x252A204, 0xB93B00 initialises (hash, handle with generation [0x252A208]), then moves every waiting subscriber node in 0x252A210 whose +0 matches the hash into the group's list +0x18. 28 callers | Confirmed |
| 0x00b94260 | `vint_datagroup_find_by_handle` | ECX = handle; returns EAX = datagroup* (0x31BACF0 + (h & 0xFFFF) * 0x20) when h != 0, index < 0x20, +0x14 == h and byte +0x1C == 0 (not closed), else 0. Caller 0x8412EC | Confirmed |
| 0x00b94290 | `vint_datagroup_add_item` | EAX = datagroup*; returns EAX = new dataitem* or 0 (0 when the group is closed, byte +0x1C != 0, or no free item): vint_dataitem_alloc(EDI = group) and links it at the tail of the open list +0xC. 17 callers | Confirmed |
| 0x00b942d0 | `vint_datagroup_queue_item` | ECX = datagroup*, EDX = item handle; returns AL = 1 when the item (index < 0x80, +0x224 == h) belongs to the group (+0x21C == ECX): unlinks it from the open list +0xC and pushes it on the delivery queue +0x10. 15 callers | Confirmed |
| 0x00b94380 | `vint_datagroup_splice_open_to_queue` | EDX = group record: same splice as vint_datagroup_close (list +0xC into +0x10) without setting the +0x1C flag. 6 callers (0x78DA0A, 0x78E5DC, 0x793A8F, 0x796097, 0x835E72, 0x9DC9B8) | Inferred |
| 0x00b94420 | `vint_dataitem_notify_changed` | ECX = item handle, stdcall (datagroup*), ret 4: validates the item, then calls every kind-3 subscriber of the group (list +0x18): callback vtable[1](owner, 3, item, 0x22C). Caller: vint_dataitem_notify (changed-flag path) | Inferred |
| 0x00b944c0 | `vint_dataitem_notify_added` | ECX = item handle, stdcall (datagroup*), ret 4: like vint_dataitem_notify_changed but for kind-4 subscribers (item added; the +0x229 flag is set by vint_dataitem_init). Caller: vint_dataitem_notify (0xB9530A) | Inferred |
| 0x00b947d0 | `vint_datagroups_remove_owner` | stdcall (u32 owner): walks every live datagroup and removes the subscriber nodes (list +0x18) whose +8 equals owner | Inferred |
| 0x00b94910 | `vint_dataitem_init` | ESI = item, ECX = name (0 -> hash 0): count +0x214 = 0, +0x10 = crc32_lower(name), flags +0x228 = +0x229 = 1, owner +0x21C = 0, subscribers +0x220 = 0, handle +0x224 = (item - 0x31A96F0) / 0x22C + (++[0x252A1FC] << 16) | Confirmed |
| 0x00b94c90 | `vint_dataitem_set_values` | EAX = vint_value* values array (0 -> no-op), stdcall (dataitem* item, int count), ret 8: for i < count (capped at 0x20) vint_dataitem_set_value(EAX = &values[i], stdcall(item, i)); values advance 0x10 per step. 18 callers (HUD / game code feeding dataitems) | Confirmed |
| 0x00b94d20 | `vint_dataitem_subscribe` | EDI = dataitem*, stdcall (u32 callback_handle, u32 owner), ret 8. Validates the callback in 0x22F2670, takes a node from the free ring 0x252A20C, callback refcount +0x1C += 1, fills {+8 owner, +0xC callback handle, +0x10 kind 2, +4 item handle}, links it into the item's list +0x220 and calls callback vtable[1](owner, 2, item, 0x22C) once | Confirmed |
| 0x00b94ee0 | `vint_dataitem_pool_init` | No args: empties the live ring 0x252A1F4 and chains all 0x80 dataitems (0x31A96F0, stride 0x22C, links +8 / +0xC) into the free ring 0x252A1F8. Called from vint_init | Confirmed |
| 0x00b94fe0 | `vint_dataitem_get_or_create` | EAX = const char* name (0 -> 0); returns EAX = dataitem* (0x22C bytes): 0xB951E0 looks it up by crc32 hash; on a miss takes a node from the free ring 0x252A1F8, links it into the live ring 0x252A1F4, inits it via 0xB94910 and adopts matching waiting subscribers from 0x252A210. 35 callers (HUD, missions, vehicles) | Confirmed |
| 0x00b95120 | `vint_dataitem_alloc` | EDI = owner datagroup* (0 -> returns 0); takes a node from the free ring 0x252A1F8, links it into the live ring 0x252A1F4, 0xB94910 with no name, +0x21C = EDI; returns EAX = item or 0 | Confirmed |
| 0x00b951b0 | `vint_dataitem_find_by_hash` | EDX = hash (0 -> 0); walks 0x252A1F4 (link +0xC) comparing +0x10; returns EAX = item or 0. Caller: vint_dataitem_subscribe_by_hash | Confirmed |
| 0x00b951e0 | `vint_dataitem_find_by_name` | ECX = const char* name (0 -> 0): crc32_lower then walk the ring 0x252A1F4 (link +0xC) comparing +0x10; returns EAX = item or 0. 10 callers | Confirmed |
| 0x00b95220 | `vint_dataitem_find_by_handle` | ECX = handle; returns EAX = item (0x31A96F0 + (h & 0xFFFF) * 0x22C) when h != 0, index < 0x80 and item +0x224 == h, else 0. 8 callers (luafn bindings, game). Pure lookup | Confirmed |
| 0x00b953a0 | `vint_dataitem_subscribe_by_hash` | stdcall (u32 name_hash, u32 callback_handle, u32 owner): finds the item (0xB951B0); present -> vint_dataitem_subscribe; absent -> queues a waiting node in 0x252A210 keyed by the hash (adopted later by vint_dataitem_get_or_create) | Inferred |
| 0x00b95480 | `vint_dataitems_remove_owner` | stdcall (u32 owner): same sweep over every live dataitem's subscriber list +0x220 | Inferred |
| 0x00b955a0 | `vint_subscriptions_remove_owner` | stdcall (u32 owner): vint_datagroups_remove_owner + vint_dataitems_remove_owner + the waiting list 0x252A210. Caller: vint_document_release_pending (0xB8AA35) | Inferred |
| 0x00b956b0 | `vint_value_get_color3` | ECX = &value, EAX = float[3] out (3 dword copies), no return; tag 6 copies payload +4, +8, +0xC, otherwise writes 0.0 x3. Caller: vint_tween_step (colour tweens) | Confirmed |
| 0x00b95a60 | `vint_element_base_init_defaults` | ECX = element*: vint_object_register_handle; then offset / anchor +0x5C..+0x68 = 0, +0x70 / +0x74 = 1.0, rotation +0x78 = 0, auto_offset +0x6C = 0, mask +0x58 = 0, depth +0x54 = 0, visible +0x59 = 1, tint and alpha +0x7C..+0x88 = 1.0. 8 callers (every element init) | Confirmed |
| 0x00b96130 | `vint_element_get_world_matrix` | ECX = element*, stdcall (float* position, float* matrix, float* extra): recurses up the parent chain (is_type element via vtable +0x18) building the accumulated 2x3 transform (sinf / cosf / matrix2x3_mul). 7 callers (text, group, Lua screen helpers) | Inferred |
| 0x00b96340 | `vint_element_get_world_scale` | ECX = element*, stdcall (float[2] out): recursive parent product: out = parent scale * (scale +0x90 / +0x94). 5 callers | Inferred |
| 0x00b97100 | `vint_writer_write_string` | ESI = writer {buffer +0, pos +0xC}, EDX = char*: stores strlen as a dword at pos, pos += 4, memcpy the string with its NUL, pos += len + 1 | Confirmed |
| 0x00b97150 | `vint_property_call_getter` | EAX = descriptor*, stdcall (object*, vint_value*), ret 8: calls the getter thunk (+0xC) with the same arguments, returns AL; 0 without a getter | Inferred |
| 0x00b971e0 | `vint_object_set_name_and_document` | EAX = const char* name, ESI = object*, stdcall (document*), ret 4; returns AL: calls the object's vtable[1], vint_object_set_name(name), then +0x30 = document. Callers: vint_object_create_in_document, clone paths | Inferred |
| 0x00b97220 | `vint_object_register_handle` | ECX = object*; returns AL = 1: handle +0x28 = ++[0x252A214]; clears +0x1C child, +0x20 sibling, +0x24 parent, +0x2C name hash, +0x30 document; links the object into the handle bucket 0x22EF2B8[handle & 0x3F] (ring +0xC / +0x10). 3 callers (element init, object create) | Confirmed |
| 0x00b97570 | `vint_object_rehash_name` | EAX = object*, ECX = new name hash: unlinks from the old name bucket 0x22EF1B8[old & 0x3F] (ring +4 / +8) and links into the new one, stores +0x2C | Confirmed |
| 0x00b97620 | `vint_object_set_name` | EDI = const char* name, stdcall (object*), ret 4: strncpy 0x20 into object +0x34 (NUL at +0x53), then vint_object_rehash_name(crc32_lower(name)) | Confirmed |
| 0x00b979b0 | `vint_object_set_document_rec` | ECX = object*, stdcall (document*), ret 4: sets +0x30 on the object and recursively on every child (+0x1C / +0x20 chain) unless it already matches | Confirmed |
| 0x00b979f0 | `vint_object_attach_child` | ESI = parent, ECX = child; returns AL = 0 for a NULL child: unlinks the child from its old parent (0xB97A20) if it has one, pushes it as first child of ESI (+0x1C, +0x20 sibling, +0x24 parent) and sets the subtree document with 0xB979B0. Caller: luafn_vint_object_add_child | Confirmed |
| 0x00b98000 | `vint_property_set_find` | EAX = property set* {descriptors, count, parent type}, stdcall (u32 name_hash); returns EAX = descriptor* (0x24 bytes, hash at +0x18) or 0: scans the set, then follows the parent type's set (type registry 0x22F2370[parent].set). 7 callers | Confirmed |
| 0x00b98150 | `vint_property_set_value_by_desc` | ECX = vint_value* v, EAX = property set*, stdcall (u32 name_hash, object*, extra), ret 0xC; returns AL: finds the descriptor (0xB98000); for an enum descriptor (type 0xC) with a string value converts name -> index (0xB9D970) and calls the setter thunk (+8). Callers: 0xB97860, 0xB98120 | Inferred |
| 0x00b982e0 | `vint_property_set_hash_names` | ESI = property set* {descriptors, count}: for each descriptor (0x24 bytes) +0x18 = crc32_lower(name at +0), +0x1C = index. 14 callers (every register_type function, 0xB9D6A0) | Confirmed |
| 0x00b984e0 | `vint_object_parse_xml` | one <object> node: reads 'type' and 'name', creates the object (0xB97EA0), applies its properties (0xB97AA0) and its 'children' (0xB98560). Registers (EAX = node) not recovered | Inferred |
| 0x00b98560 | `vint_object_parse_children_xml` | iterates 'children' / 'object' entries and calls vint_object_parse_xml for each | Inferred |
| 0x00b99060 | `vint_stream_read_u32_c` | ECX = stream, stdcall (u32* out), ret 4: third compiled copy of vint_stream_read_u32. 2 callers | Confirmed |
| 0x00b990f0 | `vint_writer_write_16_bytes` | EAX = writer {buffer, ?, ?, pos at +0xC}, EDX = src: copies four dwords to buffer + pos and pos += 0x10. 3 callers | Confirmed |
| 0x00b99280 | `vint_input_unsubscribe_document` | cdecl (u32 document handle): walks 0x252A234; every subscription whose callback's document handle (+0x1C -> +0x4C) matches is flagged remove-pending (+0x28 = 1) while dispatching ([0x252A17E] == 1), else removed at once. Caller: vint_document_release_pending (0xB8AA2C) | Confirmed |
| 0x00b992d0 | `vint_input_subscription_insert` | ECX = subscription*: inserts it into the circular list 0x252A234 ordered by its document's priority (document +0x574); ties by kind / event. 3 callers | Inferred |
| 0x00b99460 | `vint_input_subscription_reset` | ECX = subscription*, EDX = event index: speed fields +0x40 / +0x44 / +0x48 from constants, kind +0x24 = 0, event +0x2C, byte +0x3C = 1, +0x30 armed = 0, +0x3D = 0, then inserts it by priority into the active list 0x252A234 (0xB992D0). Caller: luafn_vint_subscribe_to_input_event (0xB904B0) | Confirmed |
| 0x00b99fe0 | `vint_raw_input_index` | cdecl (const char* name): scans the raw binding table (0x252A240, stride 0x18, count [0x252A23C]) for a case-insensitive name at +0x14; returns EAX = index or -1 (NULL -> -1). Caller: luafn_vint_subscribe_to_raw_input (0xB90629) | Confirmed |
| 0x00b9ac60 | `vint_project_find_resolution` | cdecl (const char* name); returns EAX = index of the first resolution string (0x22EF578, count [0x252A248]) equal to name (case-sensitive byte compare), else -1. Caller 0xB97B85 (override parse) | Inferred |
| 0x00b9af50 | `vint_project_find_document` | ECX = const char* name: crc32_lower, linear scan of the project table 0x22EF628 (0x88 bytes each, count [0x252A244]) comparing the first dword; returns EAX = &entry + 8 (0x22EF630 + i * 0x88) or 0. Callers: 0xB8A5DA, vint_document_load_with_dependencies | Confirmed |
| 0x00b9b6f0 | `vint_font_acquire_by_name` | cdecl (const char* name); returns EAX = font record* or 0 (same scheme as vint_bitmap_acquire_by_name with rings 0x252A258 / 0x252A25C, crc32_lower of the name). Callers: vint_value_set_name tag 10, the Lua reader | Inferred |
| 0x00b9b860 | `vint_font_record_by_handle` | ECX = handle; returns EAX = font record* (0x31BC400 + idx * 0x1C) when h != 0, (h & 0xFFFF) < 0xA (cmp cx,0xA) and record +8 == h, else 0. Pure lookup | Confirmed |
| 0x00b9b890 | `vint_font_find_by_platform_id` | EDX = platform font id; returns EAX = the font record whose first dword equals EDX (linear walk 0x31BC400..0x31BC518, step 0x1C), else 0 | Confirmed |
| 0x00b9b8c0 | `vint_font_record_release` | ESI = font record: refcount +0x10; when it is 1, call platform table [0x252A1D4]+0x18(record[0]) to unload, set +8 = 0 and [0] = -1, unlink from the in-use ring 0x252A258 and push on the free ring 0x252A25C; otherwise just decrement | Confirmed |
| 0x00b9bbc0 | `vint_bitmap_lookup_hash` | EDI = &hash (u32*); returns EAX = record*: first the in-use ring 0x252A264 by +0x10 (refcount +0x18 += 1, return), otherwise the atlas hashtable 0xEBDD3C (vhashtable find_9) and, when found, a new record whose texture id comes from the atlas entry (0x22DBFB0 + i * 0x20, platform id 0x22EB9B4[...]). 0 when absent | Inferred |
| 0x00b9bd20 | `vint_bitmap_acquire_by_name` | cdecl (const char* name); returns EAX = bitmap record* or 0. string_hash_lower(name) (0xBDCB30), 0xB9BBC0 finds a cached record (refcount +0x18 += 1) else takes a free record from 0x252A268, asks the platform [0x252A1D4]+0 for the texture id (-1 = fail), stamps handle +0x14 = index \| gen << 16 (gen = [0x252A26C] + 1), refcount 1, and links it into the in-use ring 0x252A264 | Confirmed |
| 0x00b9be50 | `vint_bitmap_record_by_handle` | ECX = handle; returns EAX = record* (0x31BC518 + (h & 0xFFFF) * 0x24) when h != 0, index < 0xF0 and record +0x14 == h, else 0 (setne / sub 1 / and mask). Pure lookup | Confirmed |
| 0x00b9c390 | `vint_element_group_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_group vtable (0x98-byte objects, 0x14b slots) | Confirmed |
| 0x00b9c3b0 | `vint_element_group_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00b9ca60 | `vint_element_group_pool_init` | No args: allocates the object pool (0x98-byte objects, 0x14b slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_group register function | Confirmed |
| 0x00b9cdb0 | `vint_object_animation_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_object_animation vtable (0x64-byte objects, 0xc4 slots) | Confirmed |
| 0x00b9cdd0 | `vint_object_animation_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00b9ce10 | `vint_object_animation_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00b9d360 | `vint_object_animation_pool_init` | No args: allocates the object pool (0x64-byte objects, 0xc4 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the object_animation register function | Confirmed |
| 0x00b9d6a0 | `vint_property_sets_hash_all` | No args: for every registered type [0x252A270] with a property set (0x22F2378 + i * 0x10) call vint_property_set_hash_names. Caller: vint_init (0xB91F19) | Confirmed |
| 0x00b9d760 | `vint_type_by_index` | EAX = type index; returns EAX = &0x22F2370[index] when index < [0x252A270], else 0 | Confirmed |
| 0x00b9d780 | `vint_type_find_by_name` | cdecl (const char* name); returns EAX = type record* (0x22F2370 + i * 0x10) whose name matches (__stricmp), else 0 | Confirmed |
| 0x00b9d7e0 | `vint_enum_register` | EDX = type index, cdecl (const char* property_name, enum_table*) : appends {EDX, name, ?, table} to 0x22F2170 + n * 0x10 when [0x252A274] < 0x20 and bumps the count. 12 callers (register_type functions: state / loop_mode / algorithm / horz_align / vert_align / force_case / map_mode ...) | Inferred |
| 0x00b9d860 | `vint_enum_value_by_name` | EDI = enum table {entries (8 bytes: name, value), count word +4}, stdcall (const char* s), ret 4; returns EAX = value of the first case-insensitive match or 0x7FFFFFFF (also for a NULL s) | Confirmed |
| 0x00b9d970 | `vint_enum_resolve_string` | stdcall (type index, const char* s, ...): walks the registered enums (0x22F2170 entries, count [0x252A274]) along the type chain (type +4 parent) for the property and resolves the string through vint_enum_value_by_name | Inferred |
| 0x00b9e6b0 | `vint_object_tween_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_object_tween vtable (0xe8-byte objects, 0x1f0 slots) | Confirmed |
| 0x00b9e720 | `vint_object_tween_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00b9e930 | `vint_tween_resolve_target` | EDI = tween*: revalidates the cached target object (+0x84 against handle +0x88), re-finds it by handle (+0x88 via 0xB97910) or by name hash (+0x8C via 0xB97970) in the tween's document (+0x30), then caches the property descriptor (+0x90) and flag (+0x94). Used by vint_tween_step (missing target -> skip). 4 callers | Confirmed |
| 0x00b9ff80 | `vint_object_tween_pool_init` | No args: allocates the object pool (0xe8-byte objects, 0x1f0 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the object_tween register function | Confirmed |
| 0x00ba03f0 | `vint_sound_acquire_by_name` | cdecl (const char* name); returns EAX = sound record* or 0. crc32_lower(name) matched against ring 0x252A298 (+4); a hit does refcount +0xC += 1; otherwise platform [0x252A1D4]+0x2C(name) (= 0x7F05C0, audio_foley_find_by_hash) gives a u16 id (0xFFFF = none) and a record from 0x252A29C is filled (+8 handle with generation [0x252A2A0], +0xC = 1). Caller: vint_value_set_name tag 11 | Confirmed |
| 0x00ba0540 | `vint_sound_record_by_handle` | ECX = handle; returns EAX = sound record* (0x31BE700 + idx * 0x18) when h != 0, idx < 0x32 and record +8 == h, else 0. Pure lookup | Confirmed |
| 0x00ba0570 | `vint_sound_record_release` | ESI = sound record: refcount +0xC; when it is 1 call platform [0x252A1D4]+0x34(u16 [record]) (release foley), handle +8 = 0, id word = 0xFFFF, move from the in-use ring 0x252A298 to the free ring 0x252A29C; otherwise decrement | Confirmed |
| 0x00ba0850 | `vint_callback_release` | ECX = vint_callback*: if refcount +0x1C == 1 tail-jumps to vtable[5] (+0x14, unlink and destroy) else refcount -= 1. 7 callers | Confirmed |
| 0x00ba0980 | `vint_callback_find_by_handle` | EDX = callback handle; returns EAX = vint_callback* or 0: walks the 64-bucket hash 0x22F2670[h & 0x3F] (ring through +8) comparing +0x18. The sibling 0xBA0950 matches +0x14 (name hash) instead. 9 callers (vint_value_assign tag 8, set_int, subscribe paths) | Confirmed |
| 0x00ba0bf0 | `vint_element_bitmap_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_bitmap vtable (0xc4-byte objects, 0x360 slots) | Confirmed |
| 0x00ba0c20 | `vint_element_bitmap_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba0c90 | `vint_element_bitmap_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba14b0 | `vint_element_bitmap_pool_init` | No args: allocates the object pool (0xc4-byte objects, 0x360 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_bitmap register function | Confirmed |
| 0x00ba1870 | `vint_element_gradient_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_gradient vtable (0x104-byte objects, 0x5 slots) | Confirmed |
| 0x00ba18b0 | `vint_element_gradient_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba1920 | `vint_element_gradient_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba23d0 | `vint_element_gradient_pool_init` | No args: allocates the object pool (0x104-byte objects, 0x5 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_gradient register function | Confirmed |
| 0x00ba2790 | `vint_element_text_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_text vtable (0xf4-byte objects, 0x158 slots) | Confirmed |
| 0x00ba2800 | `vint_element_text_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba2890 | `vint_element_text_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba4350 | `vint_element_text_pool_init` | No args: allocates the object pool (0xf4-byte objects, 0x158 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_text register function | Confirmed |
| 0x00ba46b0 | `vint_element_point_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_point vtable (0x98-byte objects, 0x30 slots) | Confirmed |
| 0x00ba46d0 | `vint_element_point_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba4720 | `vint_element_point_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba4920 | `vint_element_point_pool_init` | No args: allocates the object pool (0x98-byte objects, 0x30 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_point register function | Confirmed |
| 0x00ba4c60 | `vint_element_clip_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_clip vtable (0xa4-byte objects, 0x20 slots) | Confirmed |
| 0x00ba4c80 | `vint_element_clip_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba4cd0 | `vint_element_clip_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba5120 | `vint_element_clip_pool_init` | No args: allocates the object pool (0xa4-byte objects, 0x20 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_clip register function | Confirmed |
| 0x00ba5460 | `vint_element_bitmap_circle_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_bitmap_circle vtable (0xcc-byte objects, 0x40 slots) | Confirmed |
| 0x00ba5490 | `vint_element_bitmap_circle_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba5500 | `vint_element_bitmap_circle_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba5e30 | `vint_element_bitmap_circle_pool_init` | No args: allocates the object pool (0xcc-byte objects, 0x40 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_bitmap_circle register function | Confirmed |
| 0x00ba61a0 | `vint_element_sr2_map_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_sr2_map vtable (0xa0-byte objects, 0x3 slots) | Confirmed |
| 0x00ba61c0 | `vint_element_sr2_map_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba6210 | `vint_element_sr2_map_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba6610 | `vint_element_sr2_map_pool_init` | No args: allocates the object pool (0xa0-byte objects, 0x3 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_sr2_map register function | Confirmed |
| 0x00ba6950 | `vint_element_video_ctor` | ECX = this (thiscall, vector-constructor-iterator callback); returns EAX = this: clears +0x28 / +0x2C and writes the vint_element_video vtable (0x9c-byte objects, 0x1 slots) | Confirmed |
| 0x00ba6970 | `vint_element_video_dtor` | ECX = this (thiscall); frees the embedded vint_value (vint_value_free) where the class has one and rewrites the base vtable; reached through vf0 (vector deleting dtor) | Confirmed |
| 0x00ba69c0 | `vint_element_video_register_type` | No args: fills the type's property set (descriptor array + count), hashes the names (vint_property_set_hash_names), appends {name, parent, set, factory} to the type registry 0x22F2370 (max 0x20), runs the pool init and registers enums with vint_enum_register. Called from vint_init | Confirmed |
| 0x00ba6ce0 | `vint_element_video_pool_init` | No args: allocates the object pool (0x9c-byte objects, 0x1 slots) from the Vint pool [0x252A250] (vtable +0x1C alloc), runs vector_constructor_iterator with the ctor above and chains the free list in the factory (fields +4 / +0x14 / +0x18). Caller: the element_video register function | Confirmed |
| 0x00b9dd80 | `vint_ease_linear` | entry 0 of the tween interpolator table 0xE86E18[algorithm]; t*c+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9dda0 | `vint_ease_in_exp` | entry 1 of the tween interpolator table 0xE86E18[algorithm]; 2^(10*(t-1)) via crt_pow_x87. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9de10 | `vint_ease_out_exp` | entry 2 of the tween interpolator table 0xE86E18[algorithm]; 1-2^(-10*t) via crt_pow_x87. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9de90 | `vint_ease_in_out_exp` | entry 3 of the tween interpolator table 0xE86E18[algorithm]; piecewise 2^(+-10*..) via crt_pow_x87. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9df70 | `vint_ease_in_quad` | entry 4 of the tween interpolator table 0xE86E18[algorithm]; c*t*t+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9df90 | `vint_ease_out_quad` | entry 5 of the tween interpolator table 0xE86E18[algorithm]; -c*t*(t-2)+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9dfc0 | `vint_ease_in_out_quad` | entry 6 of the tween interpolator table 0xE86E18[algorithm]; piecewise quadratic. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e030 | `vint_ease_in_cubic` | entry 7 of the tween interpolator table 0xE86E18[algorithm]; c*t^3+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e050 | `vint_ease_out_cubic` | entry 8 of the tween interpolator table 0xE86E18[algorithm]; c*((t-1)^3+1)+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e080 | `vint_ease_in_out_cubic` | entry 9 of the tween interpolator table 0xE86E18[algorithm]; piecewise cubic. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e100 | `vint_ease_in_quart` | entry 10 of the tween interpolator table 0xE86E18[algorithm]; c*t^4+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e120 | `vint_ease_out_quart` | entry 11 of the tween interpolator table 0xE86E18[algorithm]; -c*((t-1)^4-1)+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e160 | `vint_ease_in_out_quart` | entry 12 of the tween interpolator table 0xE86E18[algorithm]; piecewise quartic. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e1e0 | `vint_ease_in_quint` | entry 13 of the tween interpolator table 0xE86E18[algorithm]; c*t^5+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e200 | `vint_ease_out_quint` | entry 14 of the tween interpolator table 0xE86E18[algorithm]; c*((t-1)^5+1)+b. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e230 | `vint_ease_in_out_quint` | entry 15 of the tween interpolator table 0xE86E18[algorithm]; piecewise quintic. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e2b0 | `vint_ease_in_sin` | entry 16 of the tween interpolator table 0xE86E18[algorithm]; -c*cos(t*pi/2)+c+b via cosf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e2f0 | `vint_ease_out_sin` | entry 17 of the tween interpolator table 0xE86E18[algorithm]; c*sin(t*pi/2)+b via sinf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e320 | `vint_ease_in_out_sin` | entry 18 of the tween interpolator table 0xE86E18[algorithm]; -c/2*(cos(pi*t)-1)+b via cosf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e360 | `vint_ease_in_circ` | entry 19 of the tween interpolator table 0xE86E18[algorithm]; -c*(sqrt(1-t*t)-1)+b via sqrtf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e3a0 | `vint_ease_out_circ` | entry 20 of the tween interpolator table 0xE86E18[algorithm]; c*sqrt(1-(t-1)^2)+b via sqrtf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e3e0 | `vint_ease_in_out_circ` | entry 21 of the tween interpolator table 0xE86E18[algorithm]; piecewise circular via sqrtf_wrapper. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e4a0 | `vint_ease_in_bounce` | entry 22 of the tween interpolator table 0xE86E18[algorithm]; c - out_bounce(1-t) + b (calls 0xB9E4F0). Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e4f0 | `vint_ease_out_bounce` | entry 23 of the tween interpolator table 0xE86E18[algorithm]; four-segment parabolic bounce. Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |
| 0x00b9e5f0 | `vint_ease_in_out_bounce` | entry 24 of the tween interpolator table 0xE86E18[algorithm]; in / out bounce halves (calls 0xB9E4F0 twice). Name from the Penner shape and the ease_* string order; shape Confirmed, enum slot | Confirmed shape |

### 4.3 Property thunks

Every property has a setter thunk (descriptor +8) and a getter thunk (+0xC); each is a 0x1D / 0x20-byte stub that calls one vtable slot (cdecl `(obj, value*, extra)`, returns AL, 0 for a NULL object). The rows below are the distinct thunks that were missing from `functions.csv`; shared thunks (the element's `offset`, `screen_size`, ... also used by group, bitmap_circle and text) are listed under the first class.

| Class | Property | Type | Kind | Thunk | vtable slot / method |
|---|---|---|---|---|---|
| element | render_mode | enum | set | 0xB96470 | 13, `set_render_mode` |
| element | render_mode | enum | get | 0xB96430 | 12, `get_render_mode` |
| element | visible | bool | set | 0xB96550 | 15, `set_visible` |
| element | visible | bool | get | 0xB96500 | 14, `get_visible` |
| element | mask | bool | set | 0xB965E0 | 17, `set_mask` |
| element | mask | bool | get | 0xB96590 | 16, `get_mask` |
| element | offset | vec2 | set | 0xB96720 | 21, `set_offset` |
| element | offset | vec2 | get | 0xB966D0 | 20, `get_offset` |
| element | anchor | vec2 | set | 0xB96670 | 19, `set_anchor` |
| element | anchor | vec2 | get | 0xB96620 | 18, `get_anchor` |
| element | tint | colour | set | 0xB967D0 | 23, `set_tint` |
| element | tint | colour | get | 0xB96780 | 22, `get_tint` |
| element | alpha | float | set | 0xB968A0 | 25, `set_alpha` |
| element | alpha | float | get | 0xB96840 | 24, `get_alpha` |
| element | depth | int | set | 0xB96980 | 27, `set_depth` |
| element | depth | int | get | 0xB96940 | 26, `get_depth` |
| element | screen_size | vec2 | set | 0xB96BC0 | 29, `set_screen_size` |
| element | screen_size | vec2 | get | 0xB96B30 | 28, `get_screen_size` |
| element | screen_nw | vec2 | set | 0xB96A70 | 31, `set_screen_nw` |
| element | screen_nw | vec2 | get | 0xB96A10 | 30, `get_screen_nw` |
| element | screen_se | vec2 | set | 0xB96B00 | 33, `set_screen_se` |
| element | screen_se | vec2 | get | 0xB96AA0 | 32, `get_screen_se` |
| element | rotation | float | set | 0xB96E00 | 35, `set_rotation` |
| element | rotation | float | get | 0xB96DA0 | 34, `get_rotation` |
| element | scale | vec2 | set | 0xB96EF0 | 37, `set_scale` |
| element | scale | vec2 | get | 0xB96E90 | 36, `get_scale` |
| element | auto_offset | enum | set | 0xB96F90 | 39, `set_auto_offset` |
| element | auto_offset | enum | get | 0xB96F50 | 38, `get_auto_offset` |
| element | unscaled_size | vec2 | set | 0xB97070 | 41, `set_unscaled_size` |
| element | unscaled_size | vec2 | get | 0xB97010 | 40, `get_unscaled_size` |
| group | screen_size | vec2 | set | 0xB9C880 | 29, `slot 29` |
| group | screen_size | vec2 | get | 0xB9C850 | 28, `slot 28` |
| group | screen_se | vec2 | set | 0xB9C910 | 33, `slot 33` |
| group | screen_se | vec2 | get | 0xB9C8B0 | 32, `slot 32` |
| group | screen_nw | vec2 | set | 0xB9C9A0 | 31, `slot 31` |
| group | screen_nw | vec2 | get | 0xB9C940 | 30, `slot 30` |
| animation | start_time | float | set | 0xB9CEF0 | 12, `set_start_time` |
| animation | start_time | float | get | 0xB9CF90 | 11, `get_start_time` |
| animation | is_paused | bool | set | 0xB9CFF0 | 14, `set_is_paused` |
| animation | is_paused | bool | get | 0xB9D140 | 13, `get_is_paused` |
| animation | target_handle | handle | set | 0xB9D190 | 16, `set_target_handle` |
| animation | target_handle | handle | get | 0xB9D2B0 | 15, `get_target_handle` |
| tween | target_handle | handle | set | 0xB9F3F0 | 12, `set_target_handle` |
| tween | target_handle | handle | get | 0xB9F3A0 | 11, `get_target_handle` |
| tween | target_name | string | set | 0xB9F490 | 14, `set_target_name` |
| tween | target_name | string | get | 0xB9F440 | 13, `get_target_name` |
| tween | target_property | string | set | 0xB9F530 | 16, `set_target_property` |
| tween | target_property | string | get | 0xB9F4E0 | 15, `get_target_property` |
| tween | state | enum | set | 0xB9F5C0 | 18, `set_state` |
| tween | state | enum | get | 0xB9F580 | 17, `get_state` |
| tween | start_time | float | set | 0xB9F7A0 | 20, `set_start_time` |
| tween | start_time | float | get | 0xB9F740 | 19, `get_start_time` |
| tween | duration | float | set | 0xB9F6B0 | 22, `set_duration` |
| tween | duration | float | get | 0xB9F650 | 21, `get_duration` |
| tween | start_value | variant | set | 0xB9F860 | 24, `set_start_value` |
| tween | start_value | variant | get | 0xB9F820 | 23, `get_start_value` |
| tween | end_value | variant | set | 0xB9F8F0 | 26, `set_end_value` |
| tween | end_value | variant | get | 0xB9F8B0 | 25, `get_end_value` |
| tween | loop_mode | enum | set | 0xB9F980 | 28, `set_loop_mode` |
| tween | loop_mode | enum | get | 0xB9F940 | 27, `get_loop_mode` |
| tween | algorithm | enum | set | 0xB9FA50 | 30, `set_algorithm` |
| tween | algorithm | enum | get | 0xB9FA10 | 29, `get_algorithm` |
| tween | max_loops | float | set | 0xB9FB40 | 32, `set_max_loops` |
| tween | max_loops | float | get | 0xB9FAE0 | 31, `get_max_loops` |
| tween | start_event | event | set | 0xB9FBE0 | 34, `set_start_event` |
| tween | start_event | event | get | 0xB9FC20 | 33, `get_start_event` |
| tween | end_event | event | set | 0xB9FC60 | 36, `set_end_event` |
| tween | end_event | event | get | 0xB9FCA0 | 35, `get_end_event` |
| tween | per_frame_event | event | set | 0xB9FCE0 | 38, `set_per_frame_event` |
| tween | per_frame_event | event | get | 0xB9FD20 | 37, `get_per_frame_event` |
| tween | start_value_type | enum | set | 0xB9FDA0 | 40, `set_start_value_type` |
| tween | start_value_type | enum | get | 0xB9FD60 | 39, `get_start_value_type` |
| tween | end_value_type | enum | set | 0xB9FE70 | 42, `set_end_value_type` |
| tween | end_value_type | enum | get | 0xB9FE30 | 41, `get_end_value_type` |
| bitmap | custom_source_coords | bool | set | 0xBA13A0 | 55, `set_custom_source_coords` |
| bitmap | custom_source_coords | bool | get | 0xBA13E0 | 54, `get_custom_source_coords` |
| bitmap | image | bitmap | set | 0xBA0EB0 | 47, `set_image` |
| bitmap | image | bitmap | get | 0xBA0E40 | 46, `get_image` |
| bitmap | image_crc | handle | set | 0xBA1000 | 49, `set_image_crc` |
| bitmap | image_crc | handle | get | 0xBA0FD0 | 48, `get_image_crc` |
| bitmap | source_nw | vec2 | set | 0xBA1140 | 51, `set_source_nw` |
| bitmap | source_nw | vec2 | get | 0xBA10E0 | 50, `get_source_nw` |
| bitmap | source_se | vec2 | set | 0xBA12A0 | 53, `set_source_se` |
| bitmap | source_se | vec2 | get | 0xBA1240 | 52, `get_source_se` |
| gradient | gradient_nw | colour | set | 0xBA1C70 | 57, `set_gradient_nw` |
| gradient | gradient_nw | colour | get | 0xBA1C10 | 56, `get_gradient_nw` |
| gradient | gradient_ne | colour | set | 0xBA1D40 | 59, `set_gradient_ne` |
| gradient | gradient_ne | colour | get | 0xBA1CE0 | 58, `get_gradient_ne` |
| gradient | gradient_sw | colour | set | 0xBA1E10 | 61, `set_gradient_sw` |
| gradient | gradient_sw | colour | get | 0xBA1DB0 | 60, `get_gradient_sw` |
| gradient | gradient_se | colour | set | 0xBA1EE0 | 63, `set_gradient_se` |
| gradient | gradient_se | colour | get | 0xBA1E80 | 62, `get_gradient_se` |
| gradient | alpha_nw | float | set | 0xBA1FB0 | 65, `set_alpha_nw` |
| gradient | alpha_nw | float | get | 0xBA1F50 | 64, `get_alpha_nw` |
| gradient | alpha_ne | float | set | 0xBA20B0 | 67, `set_alpha_ne` |
| gradient | alpha_ne | float | get | 0xBA2050 | 66, `get_alpha_ne` |
| gradient | alpha_sw | float | set | 0xBA21B0 | 69, `set_alpha_sw` |
| gradient | alpha_sw | float | get | 0xBA2150 | 68, `get_alpha_sw` |
| gradient | alpha_se | float | set | 0xBA22B0 | 71, `set_alpha_se` |
| gradient | alpha_se | float | get | 0xBA2250 | 70, `get_alpha_se` |
| text | font | font | set | 0xBA3250 | 47, `set_font` |
| text | font | font | get | 0xBA31E0 | 46, `get_font` |
| text | text_tag | string | set | 0xBA3010 | 49, `set_text_tag` |
| text | text_tag | string | get | 0xBA2FD0 | 48, `get_text_tag` |
| text | text_tag_crc | handle | set | 0xBA3170 | 51, `set_text_tag_crc` |
| text | text_tag_crc | handle | get | 0xBA3130 | 50, `get_text_tag_crc` |
| text | text_scale | vec2 | set | 0xBA33B0 | 53, `set_text_scale` |
| text | text_scale | vec2 | get | 0xBA3350 | 52, `get_text_scale` |
| text | word_wrap | bool | set | 0xBA35D0 | 55, `set_word_wrap` |
| text | word_wrap | bool | get | 0xBA3580 | 54, `get_word_wrap` |
| text | wrap_width | float | set | 0xBA3680 | 57, `set_wrap_width` |
| text | wrap_width | float | get | 0xBA3620 | 56, `get_wrap_width` |
| text | vert_align | enum | set | 0xBA3750 | 61, `set_vert_align` |
| text | vert_align | enum | get | 0xBA3710 | 60, `get_vert_align` |
| text | horz_align | enum | set | 0xBA3840 | 59, `set_horz_align` |
| text | horz_align | enum | get | 0xBA3800 | 58, `get_horz_align` |
| text | force_case | enum | set | 0xBA3930 | 63, `set_force_case` |
| text | force_case | enum | get | 0xBA38F0 | 62, `get_force_case` |
| text | leading | int | set | 0xBA41D0 | 65, `set_leading` |
| text | leading | int | get | 0xBA4290 | 64, `get_leading` |
| text | insert_values | variant | set | 0xBA3A40 | 75, `set_insert_values` |
| text | insert_values | variant | get | 0xBA39E0 | 74, `get_insert_values` |
| text | screen_size | vec2 | set | 0xBA3470 | 29, `slot 29` |
| text | screen_size | vec2 | get | 0xBA3420 | 28, `slot 28` |
| text | line_frame_enable | bool | set | 0xBA3C80 | 67, `set_line_frame_enable` |
| text | line_frame_enable | bool | get | 0xBA3CD0 | 66, `get_line_frame_enable` |
| text | line_frame_w | bitmap | set | 0xBA3D20 | 69, `set_line_frame_w` |
| text | line_frame_w | bitmap | get | 0xBA3E10 | 68, `get_line_frame_w` |
| text | line_frame_m | bitmap | set | 0xBA3EB0 | 71, `set_line_frame_m` |
| text | line_frame_m | bitmap | get | 0xBA3FA0 | 70, `get_line_frame_m` |
| text | line_frame_e | bitmap | set | 0xBA4040 | 73, `set_line_frame_e` |
| text | line_frame_e | bitmap | get | 0xBA4130 | 72, `get_line_frame_e` |
| point | screen_size | vec2 | set | 0xBA4870 | 29, `slot 29` |
| point | screen_size | vec2 | get | 0xBA4820 | 28, `slot 28` |
| clip | clip_size | vec2 | set | 0xBA4FB0 | 47, `set_clip_size` |
| clip | clip_size | vec2 | get | 0xBA4F50 | 46, `get_clip_size` |
| clip | clip_enabled | bool | set | 0xBA5060 | 49, `set_clip_enabled` |
| clip | clip_enabled | bool | get | 0xBA5010 | 48, `get_clip_enabled` |
| bitmap_circle | image | bitmap | set | 0xBA5760 | 47, `set_image` |
| bitmap_circle | image | bitmap | get | 0xBA56F0 | 46, `get_image` |
| bitmap_circle | source_nw | vec2 | set | 0xBA58D0 | 49, `set_source_nw` |
| bitmap_circle | source_nw | vec2 | get | 0xBA5870 | 48, `get_source_nw` |
| bitmap_circle | source_se | vec2 | set | 0xBA5A30 | 51, `set_source_se` |
| bitmap_circle | source_se | vec2 | get | 0xBA59D0 | 50, `get_source_se` |
| bitmap_circle | start_angle | float | set | 0xBA5B90 | 53, `set_start_angle` |
| bitmap_circle | start_angle | float | get | 0xBA5B30 | 52, `get_start_angle` |
| bitmap_circle | end_angle | float | set | 0xBA5C90 | 55, `set_end_angle` |
| bitmap_circle | end_angle | float | get | 0xBA5C30 | 54, `get_end_angle` |
| bitmap_circle | num_wedges | handle | set | 0xBA5D70 | 57, `set_num_wedges` |
| bitmap_circle | num_wedges | handle | get | 0xBA5D30 | 56, `get_num_wedges` |
| sr2_map | zoom | float | set | 0xBA6440 | 47, `set_zoom` |
| sr2_map | zoom | float | get | 0xBA63E0 | 46, `get_zoom` |
| sr2_map | map_mode | enum | set | 0xBA6500 | 49, `set_map_mode` |
| sr2_map | map_mode | enum | get | 0xBA64C0 | 48, `get_map_mode` |
| video | vid_id_handle | handle | set | 0xBA6BD0 | 47, `set_vid_id_handle` |
| video | vid_id_handle | handle | get | 0xBA6B90 | 46, `get_vid_id_handle` |

### 4.4 Placeholder names in `functions.csv` worth renaming

These addresses already exist (so they are not in the CSV), but the name is a `vfN` placeholder. Evidence is in the cells.

| Address | Current | Better name | Evidence |
|---|---|---|---|
| 0xBA0780 | `vint_callback::vf2` | `vint_callback_set_name` | `ECX = this`, stdcall (char*): handle `+0x18 = ++[0x252A2A4]`, hash `+0x14`, refcount `+0x1C = 1`, inserts into 0x22F2670 |
| 0xBA0870 | `vint_callback::vf5` | `vint_callback_unlink` | removes the node from its hash bucket and the list, clears links; vtable[5] used by `vint_callback_release` |
| 0xB8DCB0 | `vint_callback_lua::vf2` | `vint_callback_lua_set_name` | calls `vf2` then `strncpy` of the name into +0x28 (0x20 chars) |
| 0xB8DC40 / 0xBA0AA0 | `vint_callback_lua::vf4` / `vint_callback_native::vf4` | `..._unlink_from_list` | remove from the Lua list `0x252A1BC/C0` or native list `0x252A2A8/AC` |
| 0xBA0A40 | `vint_callback_native::vf1` | `vint_callback_native_invoke` | `ret 0x10`: forwards four stack args to the function pointer at `+0x28` |
| 0xBA0A70 | `vint_callback_native::vf6` | `vint_callback_native_set` | `vf2(name)` then `+0x28 = fn` |
| 0xBA2860 | `vint_element_text::vf8` | `vint_element_text_needs_layout` | returns bit 1 of byte `+0xF0`; `0xBA2CC0` clears it |
| 0xBA3C10 / 0xBA3C50 | `vint_element_text::vf76` / `vf77` | `..._insert_values_first` / `..._insert_values_next` | iterator over the 32-slot `insert_values` list (index in a dword: 0 -> first, ++ while < 0x20) |
| `*::vf0` (11 classes) | `vint_element_bitmap::vf0` etc. | `vint_<class>_vector_deleting_dtor` | each calls `__vec_dtor` with the class dtor listed in section 2.2 |

---

## 5. Port candidates

Fifteen low-risk functions. For each: inputs, output, registers preserved, callees, loop shape, rounding, what it writes, and the polarity of every comparison. "Unsigned" means `jae` / `jb`, "signed" means `jl` / `jge`. None of these writes memory unless stated.

**1. `vint_value_equals` 0xB93920** (EAX = &a, EDX = &b; result in AL). Clobbers ECX; EAX bits 8..31 differ by path, so compare AL only. No callees except the tail `jmp` to `vint_vec3_equal` / `vint_vec2_equal`. Writes nothing.
   - `a.tag != b.tag` -> false; `a.tag == 0` -> false; `a.tag - 1 > 10` (tags 12, 13+) -> false (`ja`).
   - tags 1, 2, 8, 9, 10, 11: dword equal (`sete`).
   - tag 3: `fld a; fld b; fucompp; fnstsw ax; test ah, 0x44; jp -> 0`. `jp` is taken when the number of set bits of `C3|C2` is even, i.e. both clear (a != b, ordered) **or both set (unordered, NaN)**; equal gives C3 = 1, C2 = 0 and returns 1. So `-0.0 == 0.0` is true and NaN is false.
   - tag 4: pointers equal -> true; either null -> false; else a byte-pair strcmp loop (case sensitive, equal iff identical, `sbb eax,eax / sbb eax,-1` only normalises the sign).
   - tag 5: the byte at +4 equal. Tag 6: `vint_vec3_equal(ECX = &a+4, EDX = &b+4)`; tag 7: `vint_vec2_equal` (same `fucompp / test ah,0x44 / jp` idiom per component).

**2. `vint_value_init` 0xB93A10** (EAX = &value, ECX = type; no return). Clobbers ECX, EDX. Guard: `if (value.tag != 0) return`. Writes `tag = type`, then for `type - 1 <= 10` (unsigned `ja` skips) the payload: types 3 -> `fldz; fstp [+4]`; 5 -> `[+4] = 0` (one byte, `mov [eax+4], dl`); 6 -> three floats 0.0 (`fst / fst / fstp`); 7 -> two floats; types 1, 2, 4, 8, 9, 10, 11 -> dword 0. **Type 12 (and above) writes only the tag.**

**3. `vint_value_get_bitmap_record` 0xB933E0** (EAX = &value; returns EAX = record or 0). Clobbers ECX, EDX. `tag != 9` -> 0; `h = [+4]`; `h == 0` -> 0; `(h & 0xFFFF) >= 0xF0` (unsigned) -> 0; `rec = 0x31BC518 + idx * 0x24` computed as `lea eax,[eax+eax*8]; lea eax,[eax*4 + 0x31BC518]`; returns `rec` iff `rec->+0x14 == h`, else 0 (`xor edx,edx; cmp [rec+0x14], ecx; setne dl; sub edx, 1; and eax, edx`: not equal gives dl = 1, `edx - 1` = 0, mask 0 -> result 0; equal gives mask 0xFFFFFFFF -> the record).

**4. `vint_bitmap_record_by_handle` 0xB9BE50** (ECX = handle -> EAX). Same body without the tag test: `h == 0` -> 0; `(u16)h >= 0xF0` unsigned -> 0; record at `0x31BC518 + idx*0x24`; handle at +0x14 must equal. Clobbers EDX.

**5. `vint_font_record_by_handle` 0xB9B860** (ECX = handle -> EAX). `cx == 0` -> 0; **16-bit compare** `cmp cx, 0xA; jae` -> 0 (so the index test uses CX, not the full handle); `rec = 0x31BC400 + idx * 0x1C` (`edx = idx*8 - idx; lea [edx*4 + ...]`); handle at +8 compared. Clobbers EDX.

**6. `vint_sound_record_by_handle` 0xBA0540** (ECX = handle -> EAX). `h == 0` -> 0; `(u16)h >= 0x32` unsigned -> 0; `rec = 0x31BE700 + idx * 0x18` (`lea eax,[eax+eax*2]; lea [eax*8 + ...]`); handle at +8 compared (`setne / sub 1 / and`).

**7. `vint_dataitem_find_by_handle` 0xB95220** (ECX = handle -> EAX). `h == 0` -> 0; `(u16)h >= 0x80` unsigned -> 0; `rec = 0x31A96F0 + idx * 0x22C` (`imul eax, eax, 0x22C`); handle at +0x224 compared as above. Clobbers EDX.

**8. `vint_datagroup_find_by_handle` 0xB94260** (ECX = handle -> EAX). `h == 0` -> 0; `(u16)h >= 0x20` unsigned -> 0; `rec = 0x31BACF0 + (idx << 5)`; **two** conditions: `rec->+0x14 == h` (else 0) **and** `byte rec->+0x1C == 0` (a closed group returns 0). No EDX use.

**9. `vint_callback_find_by_handle` 0xBA0980** (EDX = handle -> EAX). `h == 0` -> 0. Loop: `head = [0x22F2670 + (h & 0x3F) * 4]`; `if (head == 0) return 0`; `node = head; do { if (node->+0x18 == h) return node; node = node->+8; } while (node != head && node != 0)`; return 0. Clobbers ECX. Read-only walk of a circular list; stop when it wraps to the head.

**10. `vint_document_find_by_name` 0xB8B3F0** (ECX = name -> EAX). `name == 0` -> 0. Calls `string_crc32_lower` (0xBDC9B0, ECX = string, returns EAX = hash; clobbers ECX/EDX by convention). `head = [0x252A1AC]`; `head == 0` -> 0. Loop: `if (doc->+0x48 == hash) return doc; doc = doc->+0; while (doc != head && doc != 0)`. No special case for a zero hash. Returns EAX (0 when not found).

**11. `vint_dataitem_find_by_hash` 0xB951B0** (EDX = hash -> EAX). `hash == 0` -> 0. `head = [0x252A1F4]`; ring through +0xC, compare +0x10. Same loop shape as 9 (`cmp eax, ecx` wrap test then `test eax, eax`). Clobbers ECX. (The by-name variant 0xB951E0 first calls `string_crc32_lower`.)

**12. `vint_value_to_double` 0xB8CF40** (ECX = &value; result in ST0, nothing else touched; `push ecx / pop ecx` is a dummy slot). Tag 1: `fild dword`; tag 2: `fild dword` then `if ((i32)v < 0) fadd qword [0xE5C0E0]` (2^32, unsigned conversion, `jge` signed test); tag 3: `fld dword` (exact widening); anything else `fldz`. No f32 rounding: the value is returned in extended / double. The twin `vint_value_to_float` 0x5E4970 differs only in the constant address ([0xE5BFD8], a **dword** 2^32).

**13. `vint_value_get_color_bytes` 0xB8CFA0** (ECX = u8[4] out, EDX = &value; returns EAX = ECX). Always zeroes the four bytes first. If `tag == 6`: byte 3 = 0xFF; for r, g, b: `x * 255.0` (double constant [0xE5C038]), then `fnstcw`, OR with 0xC00 (round toward zero), `fldcw`, `fistp dword`, store the **low byte only** (no clamp: 256 wraps to 0, a negative product stores the low byte of its two's complement), restore the control word each time. The multiplications happen in x87 after loading the f32 channel (`fld dword`), r first, then g, then b via `fmul dword [edx+0xC]`.

**14. `vint_element_base::get_alpha` 0xB96860** (ECX = element, stdcall (value*, extra), `ret 8`; AL = 1 always). Pushes a dummy slot, reads the f32 at `+0x88` into it (round trip through memory), then: `value.tag == 0` -> `tag = 3` and store; `tag == 3` -> store the float at +4; any other tag -> do nothing; return 1. The sibling `get_visible` 0xB96520 does the same with the byte at `+0x59` and tag 5 (`mov [edx+4], cl`). All ~60 getters in `vtable_slots.csv` follow this pattern, differing only in the field and the tag.

**15. `vint_ease_*` (25 functions, one candidate)** 0xB9DD80 .. 0xB9E5F0: cdecl `(f32 t, f32 start, f32 delta)`, result in ST0, no registers. All round the result to f32 with `fstp dword [esp+4] / fld dword [esp+4]` before returning. `linear` = `t * delta + start`; `in_quad` = `(t * (t * delta)) + start`; `out_quad` = `start - (t - 2.0) * (t * delta)` (constant [0xE5BFE8] = 2.0); `in_out_quad` stores `t2 = t * 2` as f32, then `t2 - 1` as f32 and tests its **sign bit** (`shr eax, 0x1F; test al, 1; je`): sign set (t < 0.5) -> `0.5 * delta * t2 * t2 + start`; sign clear (t >= 0.5) -> `-0.5 * delta * ((t2 - 1) * (t2 - 3) - 1) + start` (constants 0.5 [0xE5BF70], 3.0 [0xE5BFF8], both double). The `exp`, `sin`, `circ` forms call the CRT (`crt_pow_x87`, `sinf_wrapper` / `cosf_wrapper`, `sqrtf_wrapper`), `bounce` forms call `out_bounce` 0xB9E4F0 (`in_bounce` = `delta - out_bounce(1-t, 0, delta) + start`). Port order: linear, quad, cubic, quart, quint (polynomial, no calls), then sin / circ / exp, bounce last. The table is `0xE86E18[algorithm]` (25 entries, enum order = `linear, in_exp, out_exp, in_out_exp, in_quad, ..., in_out_bounce`); check the enum slot against `vint_tween_step`.

---

## 6. Bugs and doc errors

1. **`functions.csv`: 0x773120 is `quit_game_dialog` and 0x7903D0 is `hud_console_toggle`; both are vint value helpers.** The bodies are `vint_value_to_bool` (tag switch with `fldz / fcomp / test ah, 0x44`) and `vint_value_set_float` (tag 0 -> 3, store the float); they are called from `vint_value_from_lua` and `vint_value_push_lua`. The UI strings the old names cite do not appear in the bodies (identical-code folding is the likely cause of the mislabel). Suggested rename: `vint_value_to_bool`, `vint_value_set_float`.
2. **Doc 06, tag table:** tag **11 (sound / foley)** is missing; tag 13 ("variant") is a descriptor-only type, not a live value tag. Tag 4 (string) is a borrowed pointer, not an owned string.
3. **Doc 43 / `skipped_ports.md`:** correct but incomplete for `insert_values_in_string`. The first argument can also be a string already in the 0x80-packed encoding (decoded with `0xB92670`), and the table is read **0-based** (`t[0] .. t[n-1]`). The localisation hook `+0` is called twice on success (harmless). `values[32]` has no bound check on the table entry count: more than 32 entries corrupt the frame.
4. **`vint_value_free` does not clear the tag**, and for tag 8 does not clear the payload either: reusing a freed value without writing `tag = 0` leaves a dangling callback handle. `vint_value_assign` silently does nothing when the destination tag differs from the source tag (typed destinations ignore wrong-typed values).
5. **`vint_value_equals` is always false for tag 12 (enum)**, and `vint_value_init` leaves its payload undefined (also for tag 13).
6. **`vint_value_to_int` (0x5E4930) returns -1 for a true bool**, not 1 (`neg al / sbb eax, eax`).
7. **`vint_stream_read_u8` / `_u32`**: the bounds test is strict (`cur + n < end`), so the last byte (or the last 4 bytes) of a stream can never be read; binary vint documents are therefore one element short of the buffer end.
8. **`vint_value_push_lua` for a colour** pushes the quantised bytes (`r/255.0`), so a `get_property` on a tint returns values rounded to 1/255, not the stored floats.
9. **`luafn_vint_dataitem_get`** reads its handle from the **last** argument slot (index -1) even with two arguments; with zero arguments it reads index 0.
10. **Latent crash in `luafn_vint_insert_values_in_string`:** when argument 1 is neither a string nor a number tag 1 / 2 (nil, a fractional number, a bool, a table), `src` is NULL; `vint_decode_wide_string` and the lstr hook both return 0 for NULL, so the widening loop at 0xB90B00 reads byte `[0]` and faults. A port should return 0 results (or an empty string) for that case and say so.
11. **Doc 06**: `vint_input_subscription` defaults (repeat multiplier, max speed, base delay) are the constants 1.3 / 5.0 / 0.4 s (floats at 0xE8762C, 0xDD0794, 0xDD1C50) written by `vint_input_subscription_reset`.

## 7. Open

- The exact `{N}` modifier grammar of `vint_insert_values_expand` beyond `text_tag_crc` and `not_localized` (only two modifier strings at 0xE86F48 exist).
- `vint_bitmap_lookup_hash` (atlas hashtable 0xEBDD3C) and the XML object loader (0xB97EA0, 0xB97AA0, 0xB98F00) are only partly mapped; the loader's register conventions are inferred.
- The tween `algorithm` enum order was derived from the shapes of the 25 functions and the `ease_*` strings at 0xE4C4F4..0xE4C688; confirm one entry (for example `in_quad` = 4) against a real tween before relying on the numbering.
