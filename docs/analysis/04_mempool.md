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

`virtual_mempool::alloc` (0xBFCA60) does the same under its lock (+0x4C). It also commits new pages with `VirtualAlloc(MEM_COMMIT, PAGE_READWRITE)` once `used` passes `committed` (+0x68), zero-fills them if flag +0x6C bit 0 is set, and goes through the page tracker at +0x70 when present (`0xC152C0` / `0xC153A0`). Not ported yet.

## Runtime survey (boot plus save load)

Every slot of the 11 pool vtables was routed through a `lock inc; jmp [orig]` stub (`crates/sr2-hook/src/survey.rs`, off by default).

**Calls:**
- `virtual_mempool`: `is_valid` 2.2M, `bytes_free` 1.1M, `can_alloc` 1.1M, `alloc` 1.1M.
- `split_mempool_template`: 435k / 435k / 179k / 256k.
- `mempool`: 58k / 58k / – / 29k.
- `tracked_mempool`: 24k (alloc through its own slot 7, plus slot 28 at 18k).
- `string_pool`, `string_hash_table`, `compacting_mempool`: small.

**Pool objects in `.data`:** "gpu idx/vtx buffers", "cpu idx/vtx buffers", "gr_mesh pool" (async), "string_hash_table", "Bmpman extra filename hash table", "BitmapsMemPool", "packfile_pool", "temp packfile", "cmdline", 10 × "audio - engine slot", per-character and weather `string_pool`s, "light set", "Vint" and "profile_vpool" (`virtual_mempool`), and about 70 unused "Untitled" `virtual_mempool` slots. Pools on the heap were not scanned.

## Ported

- **`static_mempool_base::alloc` → Rust** (`crates/sr2-hook/src/mempool.rs`, 2026-10-31). Slot 7 is swapped in every vtable that uses `0xC00DB0`: `mempool`, `string_pool`, `string_hash_table`, `split_mempool_template<mempool>`. No code bytes are patched. The `is_valid` and `bytes_free` calls stay virtual, so subclass overrides still apply.
- **Verification:** on every call the original runs first. Its result and the first 0x50 bytes of the pool are kept, the header is rewound, the port runs, and both outcomes are compared; on a mismatch the original's outcome is restored. Boot plus save load: 428,079 calls, 0 mismatches. The failure path (printf) has not been exercised.

## Next

`can_alloc` (slot 6) and `bytes_free` (slot 3) are trivial. `virtual_mempool::alloc` is the hottest (1.1M calls), followed by `tracked_mempool` and `page_mempool`, and the reset paths (slots +0x30/+0x34/+0x58).
