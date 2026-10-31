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
2. ✅ Ghidra baseline: startup chain, window, D3D and input entry points (docs/analysis/00_initial_overview.md). ⏳ Still to do: RTTI class list.
3. ✅ Rust workspace and toolchain; hook DLL (proxy `dinput8.dll`) loads in-game (log: `exe base 0x400000`).
4. ✅ `sr2-formats::vpp`: parses all 18 shipped archives (`cargo run --release -p sr2-formats --example vpp -- check <dir>/*.vpp_pc`). ⏳ Still to do: map the in-game loader.
5. ✅ First Rust replacement: `hash_filename_ci` @0xBF2BD0 → `sr2_formats::hash` (self-tested against the original in-process; main menu loads).
6. ✅ `packfile_find_record` @0xC0A3B0 → Rust (`hooks.rs`). Trampoline keeps the original; at frame 300 every registered file (53,018) is queried as-is, upper-cased and as a miss through both, and any mismatch flips the hook back to the original. 159,057/159,057 matched in-game.
7. ✅ `packfile_mount` @0xC0B410 → Rust. Boot mounts run before the first `DirectInput8Create`, so this hook is installed from a thread spawned in `DllMain` that waits for SteamStub to decrypt the bytes (~270 ms). Verified by diffing a per-mount fingerprint log (slot fields, directory, record table) against a reference captured from the original: `docs/analysis/golden/boot_mounts.txt`, all 16 identical.
8. ✅ First visible change: an override archive (`decomp.vpp_pc`, mounted after patch) plus a peg reader/writer, used to add "Decomp" to the main-menu logo. See `docs/analysis/02_ui_textures_and_overrides.md`.
9. ✅ City load tested: Continue → crib with every Rust hook live, no fallbacks. Game input works through `SendInput` scancodes.
10. ✅ `packfile_find_entry` @0xC0AD80 → Rust, shadow-checked against the original on every call (212k-query sweep plus 37k live calls, 0 mismatches). `packfile_mount_temp` @0xC0A970 → Rust; its city-load log is identical to `docs/analysis/golden/temp_mounts.txt`.
11. Next: the file layer above (`file_open`, `file_resolve` and the search-path table, `file_read`), and the temp unmount.
