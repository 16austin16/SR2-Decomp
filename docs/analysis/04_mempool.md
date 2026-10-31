# 04 — Memory pools (`mempool_base` family)

The classes come from `docs/rtti_classes.csv` (see 03). Everything here is Observed (capstone decode plus in-game survey) unless marked otherwise.

## Classes and vtables

`mempool_base` is abstract (every slot is `_purecall`). `static_mempool_base` (20 slots) holds the shared implementation, and each concrete pool overrides some slots:

| Class | vtable | Slots | Overrides of note |
|---|---|---|---|
| `mempool` | 0xE58CD4 | 25 | slot 3 `0x4A4AB0` |
| `virtual_mempool` | 0xE593A4 | 25 | slot 3 `0xA7F2B0`, slot 7 `0xBFCA60` (alloc with commit), 13, 16, 19–24 |
| `page_mempool` | 0xE5AC64 | 25 | most slots (`0xC1C9D0`…`0xC1D180`) |
| `split_page_mempool` | 0xE5A6C4 | 30 | most slots (`0xC166B0`…`0xC17230`) |
| `async_mempool` | 0xDE1B94 | 22 | 0, 3–5, 7, 8, 10, 13–16, 19–21 |
| `async_virtual_mempool` | 0xE5A744 | 25 | |
| `compacting_mempool` | 0xDE18D4 | 21 | 3, 7, 8, 10, 12–16, 18–20 |
| `tracked_mempool` | 0xE594DC | 29 | 7 `0xC01040`, 16, 25–28 |
| `string_pool` / `string_hash_table` | 0xDDA834 / 0xE5959C | 25 | 3 `0x4A4AA0`, 13 |
| `split_mempool_template<mempool>` | 0xDE186C | 25 | like `mempool` |

Methods identified so far:

| Slot | Offset | Function | Meaning |
|---|---|---|---|
| 0 | +00 | `0xC00C60` (`ret`) | destructor / no-op in the base |
| 1 | +04 | `0xC00C50` | `is_valid()`: returns 1 (reads a debug byte `0x2527BF7`, then ignores it) |
| 2 | +08 | `0x4A4A90` | returns 0 |
| 3 | +0C | `0x4A4AB0` (mempool) / `0xA7F2B0` (virtual) | `bytes_free()`: `end(+0x4C) − used(+0x44)` / `size(+0x3C) − used` |
| 6 | +18 | `0xC00D40` | `can_alloc(size, align)`: the same checks as alloc, no state change (thiscall, `ret 8`) |
| 7 | +1C | `0xC00DB0` / `0xBFCA60` | `alloc(size, align)` (thiscall, `ret 8`) |

## Object layout (`static_mempool_base`)

| Offset | Field |
|---|---|
| +0x00 | vtable |
| +0x04 | u8 disabled (1 = every alloc fails) |
| +0x0C | `char name[]` inline ("packfile_pool", "temp packfile", "Vint", …) |
| +0x2C | flags; bit 0 = only one allocation allowed (alloc fails once `used != 0`) |
| +0x34 | last allocation |
| +0x38 | base |
| +0x3C | size (reserve size for `virtual_mempool`) |
| +0x40 | default alignment |
| +0x44 | used (bump cursor) |
| +0x4C | end (`mempool`); a CRITICAL_SECTION in `virtual_mempool` |
| +0x68 / +0x6C / +0x70 | `virtual_mempool`: committed bytes / flags (bit 0 = zero new pages) / optional page tracker |

## `static_mempool_base::alloc` (0xC00DB0)

1. Return NULL if disabled, or if `is_valid()` (slot 1) is false.
2. Return NULL if `used != 0` and flag bit 0 is set.
3. Let `a = align` or the default (+0x40). `pad` aligns `base + used` up to `a`.
4. If `pad + size > bytes_free()` (slot 3): `printf("Failed aligned alloc of\n%d bytes in %s pool\n(max pool size is %.2fMB (%d)\nspace left is %d)\n", size, name, size_field / 2^20, size_field, bytes_free())` and return NULL. `0xC99400` is the CRT `printf`, so the message goes to stdout, which a GUI app doesn't show.
5. `last = base + used + pad`, `used += pad + size`, return `last`.

There is no free: these are bump/arena pools that are reset as a whole (the "temp packfile" pool is reset through slot `+0x58` by the temp unmount, see 01 §5f).

## `virtual_mempool::alloc` (0xBFCA60)

thiscall `(size, align)`, `ret 8`, with a C++ EH frame.

1. If disabled, or if `is_valid()` is false: `printf("ERROR: assert_permission")`, return NULL.
2. `EnterCriticalSection(this+0x4C)`. Every return below leaves it first.
3. If flag bit 0 is set and `used != 0`, return NULL.
4. Align as in the base alloc. `new_used = used + pad + size`. If `new_used > size (+0x3C)` (the reserved range), return NULL with no message.
5. If `new_used > committed (+0x68)`: round `new_used` up to a 4 KB page (`end`), then:
   - If there is a page tracker (+0x70): `0xC152C0(ECX=pages, EDX=tracker)` reserves `(end − committed) / 4096` pages. On failure, `printf("No page pool pages left for virtual_mempool allocation.\n")` and return NULL.
   - `VirtualAlloc(base + committed, end − committed, MEM_COMMIT, PAGE_READWRITE)`. On failure, `printf("Mempool %s is out of memory allocating %d bytes, %d bytes left of %d total! ...", name, size, (tracker.free_pages << 12) − committed, size_field)`, release the tracker pages (`0xC153A0(ECX=pages, ESI=tracker)`), and return NULL. The original reads `tracker+0x24` even without a tracker.
   - If flag +0x6C bit 0 is set, `memset` the new pages to 0. `committed = end`.
6. `last = base + used + pad`, `used += pad + size`, return `last`.

**Page tracker** (the "page pool" behind the virtual pools):
- Layout: CRITICAL_SECTION at +0x00, +0x1C total bytes, +0x24 free pages, +0x28 high-water mark, +0x50 decommit flag, +0x54 reserve base, +0x58 second lock.
- `0xC152C0` reserve: lock the tracker. If more pages are needed than are free, compact (`0xC15810`); if there still aren't enough, fail. Otherwise `free −= pages`, `hwm = max(hwm, total − free·4096)`. If +0x50 is set, `VirtualFree(MEM_DECOMMIT)` that many pages of the tracker's own reserve, so the total committed memory stays within budget.

## Runtime survey (boot plus save load)

Every slot of the 11 pool vtables was routed through a `lock inc; jmp [orig]` stub (`crates/sr2-hook/src/survey.rs`, off by default).

**Calls:**
- `virtual_mempool`: `is_valid` 2.2M, `bytes_free` 1.1M, `can_alloc` 1.1M, `alloc` 1.1M.
- `split_mempool_template`: 435k / 435k / 179k / 256k.
- `mempool`: 58k / 58k / – / 29k.
- `tracked_mempool`: 24k (alloc through its own slot 7, plus slot 28 at 18k).
- `string_pool`, `string_hash_table`, `compacting_mempool`: small.

**Pool objects in `.data`:** "gpu idx/vtx buffers", "cpu idx/vtx buffers", "gr_mesh pool" (async), "string_hash_table", "Bmpman extra filename hash table", "BitmapsMemPool", "packfile_pool", "temp packfile", "cmdline", 10 × "audio - engine slot", per-character and weather `string_pool`s, "light set", "Vint" and "profile_vpool" (`virtual_mempool`), and about 70 unused "Untitled" `virtual_mempool` slots. Pools on the heap were not scanned.

## Other methods (all used over a boot plus save load)

| Slot | Function | Meaning |
|---|---|---|
| 4 | `0x4A4AE0` | `used_bytes()`: `size − bytes_free()` |
| 5 / 11 | `0x4A4B10` / `0x4A4B20` | `size()` (+0x3C) / `base()` (+0x38) |
| 8 | `0xC00EC0` | `resize_last(ptr, size)`: if `ptr` is the last allocation and the growth fits, `used = last − base + size`; returns `last` or NULL |
| 10 | `0xC00FA0` / `0xBF4AF0` (mempool) | `reset()`: `release_to(0)`; mempool also `set_limit(size)` |
| 12 | `0xC00CF0` | `mark()`: mark (+0x48) = used |
| 13 | `0xC00D00` | `release_to(pos)`: used = mark = pos (−1 means the mark) |
| 13 | `0xBFC870` (virtual) | `release_to(pos)`: under the lock, used = pos (the mark is not updated), then unless flag +0x6C bit 1 is set, `VirtualFree(MEM_DECOMMIT)` the whole pages above `used` and return them to the tracker (`0xC153A0`). If bit 0 is set, zero the released bytes after unlocking. |
| 14 | `0xC00F80` | `release(n)`: `release_to(used − n)`, keeping the mark |
| 15 | `0xC00F40` | `align_to(a)`: if `base + used` is unaligned, `alloc(pad, 1)` |
| 17 | `0xBF4A10` | `init(base, size, name, align, flags)` (`ret 0x14`): name copied to +0x0C, flags byte to +0x2C, `limit = saved limit = size`, everything else zero |
| 22 | `0xBF4AB0` | `set_limit(x)`: limit (+0x4C) = saved (+0x50) = x (−1 means the saved value) |
| 7 | `0xC0BC00` (async) | `alloc(size, align)` lock-free: if `align` exceeds the default, over-allocate by `align`. Round up, `InterlockedExchangeAdd(&used, n)`; on overflow set `used = size` and return NULL; otherwise return the aligned `base + old`. |
| 7 / 28 | `0xC01040` / `0xC01060` (tracked) | `alloc` forwards to slot 28 with tracking arguments (`this+0x60`, `[this+0xA0]`); slot 28 ignores them and calls the base alloc directly |

`mempool`'s +0x4C is a **limit** (bytes from base, which `bytes_free` uses), not an address; +0x50 is its saved copy.

## Ported (2026-10-31): every pool method the game calls

**25 methods → Rust** (`crates/sr2-hook/src/mempool.rs`):
- `is_valid`, the five `bytes_free` variants, `used_bytes`, `size`, `base`, `mark`, `release_to`, `can_alloc`, `alloc`, `resize_last`, `release`, `align_to`;
- both `reset`s, `set_limit` and `init`;
- `virtual_mempool::alloc` and `::release_to`, `async_mempool::alloc`, and the two `tracked_mempool` allocs.

This is every pool method called over a boot plus save load (re-surveyed with the first two ports in place); each one ran at least once.

**Inline detours, not vtable swaps.**
- The first two ports swapped vtable slots. The exe also calls these methods directly: 290 call sites for `alloc`, 57 for `virtual_mempool::alloc`, 33 for `virtual_mempool::release_to` and 7 for `async_mempool::alloc`. Those direct calls were still running the original.
- Every method is now detoured at its body, so both virtual and direct calls reach Rust. The original stays callable through its trampoline.
- All 25 prologues were checked against the exe with capstone: bytes match, they end on an instruction boundary, and there are no relative branches. The one-line getters steal their own `ret` plus one byte of int3 padding.

**Register-exact leaf thunks.**
- The first build crashed at boot, in a text parser at `0xBF82F3`. With LTCG, a direct caller of a leaf function may keep values in the registers the leaf happens not to touch.
- The leaves now enter through naked thunks that reproduce the original's exact outputs:
  - `is_valid` changes only AL;
  - `size`/`base` change only EAX;
  - `mark` leaves EAX = `used`;
  - the `mempool` `bytes_free` leaves ECX = `used`, the `size − used` variants leave EDX = `used`;
  - `used_bytes` leaves ECX = EAX.
- Methods that call a virtual function clobber EAX/ECX/EDX anyway.

**Verification on every call.**
- Pure methods run both versions and compare the results. The original is read a second time before reporting a difference, since another thread may have allocated in between.
- State-changing methods use snapshot → original → rewind → port → compare over the first 0x54 bytes, keeping the original's outcome on a mismatch.
- The `virtual_mempool` methods hold the pool and tracker locks, skip the lock bytes, and rewind the tracker counters. `MEM_COMMIT`/`MEM_DECOMMIT`, the tracker's recommit and the zero-fill are all safe to repeat.
- The lock-free async pool is serialized by a mutex in the hook while it is checked. That is possible because every call now reaches the hook.

**Boot plus save load:** 20,473,919 calls, 0 mismatches, 31,923 page commits and decommits done by Rust, and no tracker compaction left to the original.

| Method | Calls |
|---|---|
| `is_valid` | 8.8M |
| `async_mempool::alloc` | 2.1M |
| `virtual_mempool::bytes_free` | 2.4M |
| `can_alloc` | 1.4M |
| `mempool::bytes_free` | 1.4M |
| `virtual_mempool::alloc` | 1.26M |
| `mark` | 1.19M |
| `virtual_mempool::release_to` | 1.08M |
| `alloc` | 516k |
| `release_to` | 121k |
| `size` | 57k |
| `tracked_mempool::alloc_impl` | 31k |
| `tracked_mempool::alloc` | 13k |
| `set_limit` | 6.8k |
| `mempool::reset` | 3.3k |
| `release` | 1.2k |
| `init` | 901 |
| `align_to` | 14 |
| `resize_last` | 3 |
| `used_bytes` | 1 |

**Not exercised:** the failure paths (out of space or reserve, no tracker pages, `VirtualAlloc` failure) and tracker compaction.

## Not ported (never called in the survey)

- The overrides in `page_mempool` (`0xC1C9D0`…), `split_page_mempool` (`0xC166B0`…) and `compacting_mempool` (beyond `bytes_free`).
- `async_mempool`'s other slots, and `virtual_mempool` init/destroy/attach-tracker (`0xBFC630`, `0xBFC790`, `0xBFC830`, `0xBFC850`).
- The string-pool `release_to` variants (`0xC07040`, `0xC078C0`, the hash table).
- The page tracker itself (`0xC152C0`, `0xC153A0`, `0xC15810`).

These need a scenario that actually reaches them (multiplayer, other game modes) before they can be verified.
