# Saints Row 2 (PC) → Rust rewrite: plan

## Inputs
| Item | Location |
|---|---|
| Original exe (SteamStub 2.1 wrapped, AppID 9480) | `bin/original/SR2_pc.exe` (sha256 `e20156a0…cfe8`) |
| Unpacked exe used for all analysis | `bin/SR2_pc.unpacked.exe` (sha256 `f76b3ccc…870e`), made by `tools/unsteamstub21.py` |
| Game data | `E:\SteamGames\steamapps\common\Saints Row 2` (not copied; ~25 GB of `.vpp_pc`) |
| Ghidra | `C:\ghidra\ghidra_12.1.4_PUBLIC`, driven through REA |

Unpacked layout: image base 0x400000, `.text` 0x401000–0xDB71F4, OEP 0xC9E22E. The file
keeps the original section layout (the `.bind` stub is left in place but never runs), so file
offsets match the shipped exe. Built with MSVC 8.0 (VS2005), 2009-02-09. The PDB isn't shipped.
About 1000 RTTI class names are present.

## Strategy: incremental replacement (hook DLL)
A 10 MB game can't be rewritten in one go and then debugged. We replace it piece by piece
while the original keeps running:

1. **`sr2-hook` (cdylib, `i686-pc-windows-msvc`).** It loads into the original
   `SR2_pc.unpacked.exe` (via a proxy DLL such as `dinput8.dll`, or a small launcher) and
   detours individual original functions to Rust reimplementations. Each rewritten function is
   tested in the real game straight away.
2. **Shared crates, written as normal portable Rust** (no detours):
   - `sr2-formats`: file formats (`.vpp_pc` archives, `.peg_pc` textures, `.tbl`/`.xtbl`, `.str2_pc`, etc.), testable against the real game data.
   - `sr2-core`: math, containers, and allocator-compatible structs with `#[repr(C)]` layouts that match the original.
   - Subsystem crates as they are reached: `sr2-render` (D3D9), `sr2-audio`, `sr2-input`, `sr2-script` (Lua), `sr2-game`.
3. **`sr2` (bin).** The standalone executable. It takes over once enough of the game is in Rust.

32-bit is required for the hook stage, because struct layouts and pointers must match the
original. The standalone build can move to 64-bit later.

## Analysis workflow
- Ghidra through REA: name functions, classes and vtables (RTTI), and document them in `docs/analysis/`.
- Track every function in `docs/functions.csv` (address, name, subsystem, status: `unknown` / `named` / `documented` / `rust` / `verified`).
- Start with leaf and library code. Identify the statically linked VC8 CRT and third-party code (Lua, Bink glue, zlib?) first, so it can be excluded or replaced with crates.

## Milestones
1. ✅ Unpack DRM and get a clean exe for static analysis.
2. ✅ Ghidra baseline: startup chain, window, D3D and input entry points (docs/analysis/00_initial_overview.md). RTTI class list: `docs/rtti_classes.csv` (1,062 classes with vtables and bases, `tools/rtti_dump.py`), grouped by subsystem in `docs/analysis/03_rtti_classes.md`.
3. ✅ Rust workspace and toolchain; hook DLL (proxy `dinput8.dll`) loads in-game (log: `exe base 0x400000`).
4. ✅ `sr2-formats::vpp`: parses all 18 shipped archives (`cargo run --release -p sr2-formats --example vpp -- check <dir>/*.vpp_pc`). ⏳ Still to do: map the in-game loader.
5. ✅ First Rust replacement: `hash_filename_ci` @0xBF2BD0 → `sr2_formats::hash` (self-tested against the original in-process; main menu loads).
6. ✅ `packfile_find_record` @0xC0A3B0 → Rust (`hooks.rs`). Trampoline keeps the original; at frame 300 every registered file (53,018) is queried as-is, upper-cased and as a miss through both, and any mismatch flips the hook back to the original. 159,057/159,057 matched in-game.
7. ✅ `packfile_mount` @0xC0B410 → Rust. Boot mounts run before the first `DirectInput8Create`, so this hook is installed from a thread spawned in `DllMain` that waits for SteamStub to decrypt the bytes (~270 ms). Verified by diffing a per-mount fingerprint log (slot fields, directory, record table) against a reference captured from the original: `docs/analysis/golden/boot_mounts.txt`, all 16 identical.
8. ✅ First visible change: an override archive (`decomp.vpp_pc`, mounted after patch) plus a peg reader/writer, used to add "Decomp" to the main-menu logo. See `docs/analysis/02_ui_textures_and_overrides.md`.
9. ✅ City load tested: Continue → crib with every Rust hook live, no fallbacks. Game input works through `SendInput` scancodes.
10. ✅ `packfile_find_entry` @0xC0AD80 → Rust, shadow-checked against the original on every call (212k-query sweep plus 37k live calls, 0 mismatches). `packfile_mount_temp` @0xC0A970 → Rust; its city-load log is identical to `docs/analysis/golden/temp_mounts.txt`.
11. ✅ `file_resolve` @0xBFD880 → Rust (kinds 0 and 1; kind 2 shimmed), shadow-checked on every call: 22,159 calls in a boot plus city load, 0 mismatches. The search order is packfile, disk, packfile, so archives win over loose files.
12. ✅ `file_open` @0xBFDC20 → Rust for reads. 9,843 distinct calls over a boot plus save load gave the same results as the original. Writes and appends stay on the original until a save is tested.
13. ✅ `file_read` @0xBFE170 → Rust. All 2,170,333 distinct reads in a boot plus save load are identical to the original, including a hash of the bytes. The EOF and compressed paths have not been exercised yet.
14. ✅ Save tested: a new save slot written through the Rust `file_open` loads back fine, so writes are on Rust too. Menu input: Esc opens the pause menu, Ctrl/Shift switch tabs, arrow keys are extended scancodes.
15. ✅ Temp unmount (`0xC0B8E0` plus record unregister `0xC0B2A0`) → Rust. All three unmounts in a save load leave the same state as the original's (`golden/temp_unmounts.txt`).
16. ✅ `file_write` @0xBFE360 and `file_close` @0xBFDF30 → Rust. A save written and closed by Rust loads back, and 9,770 distinct closes match the original. The whole file API (open/resolve/read/write/close) is now Rust.
17. ✅ RTTI class map (`docs/analysis/03_rtti_classes.md`): 517 game classes across objects, memory pools, streaming, renderer, D3D wrappers, vint UI, scripting, activities, multiplayer; about 305 Havok.
18. ✅ Memory pools started (`docs/analysis/04_mempool.md`): vtables, layout, and a runtime survey of method calls and pool objects. `static_mempool_base::alloc` → Rust by vtable-slot swap in 4 pool classes, shadow-checked on every call (428k allocations, 0 mismatches).
19. ✅ `virtual_mempool::alloc` → Rust (page commits through the page tracker included), shadow-checked under the pool and tracker locks: 1.26M allocations and 18,574 commits, 0 mismatches.
20. ✅ Memory pools done for everything the game calls: 25 methods → Rust as inline detours, covering direct calls as well as virtual ones, with register-exact thunks for leaf methods. Shadow-checked on every call: 20.5M calls over boot plus save load, 0 mismatches. Untouched: the pool types never used in single player, and the page tracker internals.
21. ✅ Streamer core mapped (`docs/analysis/05_streamer.md`): resource types and slots, request queue, the pump, loading infos, the chunk loader.
22. ✅ First streamer code in Rust: the request queue (init/push/remove/pop_best/node_index) and the slot queries, shadow-checked on every call (76,687 calls over a boot plus save load, 0 mismatches).
23. Next: the rest of the streamer (unload/abort, the pump, start_load, poll_active, the read scheduler at `0xA75900`–`0xA77E90`), then the vint UI and the D3D wrappers.
