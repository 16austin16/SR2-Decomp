# SR2-Decomp

Saints Row 2 (PC, Steam) being rewritten in Rust, one function at a time, while the game runs.

This repo contains no game files. You need your own copy of the game.

## Progress

![Progress: functions and code running in Rust, by area](docs/img/progress.svg)

- **2,392 functions run in Rust**, about 8% of the game's functions and 6% of its code.
- **14,565 functions are identified** (named), about half of the game.

## How it works

A proxy `dinput8.dll` loads into the original game and swaps original functions for Rust versions one at a time. A Rust version only counts as done once it matches the original. Two kinds of checks prove that:

- **In the game:** for its first few thousand calls, the Rust version runs side by side with the original and their results are compared.
- **Offline:** it is tested against the original machine code on thousands of random inputs.

Each batch of new Rust functions is also play-tested before it is merged.

## Building

Requirements:
- rustup (stable, MSVC toolchain)
- Visual Studio 2022 Build Tools with the C++ workload

```sh
rustup target add i686-pc-windows-msvc
cargo build -p sr2-hook --target i686-pc-windows-msvc --release
```

Copy `target/i686-pc-windows-msvc/release/dinput8.dll` into your Saints Row 2 folder. Delete it to go back to the unmodified game.

## Repository layout

| Path | What it is |
|---|---|
| `crates/sr2-hook` | the hook DLL and all the Rust ports (`port_*.rs`) |
| `crates/sr2-formats` | readers for the game's file formats (`.vpp_pc`, `.peg_pc`, `.chunk_pc`, ...) |
| `docs/functions.csv` | every known function: address, name, area and status |
| `docs/analysis/` | write-ups of how each part of the game works |
| `docs/skipped_ports.md` | functions not ported yet, and why |
| `docs/PLAN.md` | strategy and milestones |
| `tools/` | analysis scripts |

## Analysing the game yourself

Static analysis needs a copy of the exe without the Steam DRM. To make one from your own copy:

```sh
python tools/unsteamstub21.py "<SR2 install>/SR2_pc.exe" bin/SR2_pc.unpacked.exe
```

To redraw the progress graphic:

```sh
python -I tools/progress.py bin/SR2_pc.unpacked.exe docs/img/progress.svg
```
