# SR2-Decomp

A reverse-engineering project rewriting Saints Row 2 (PC, Steam) in Rust. This repo has
no game files; you need your own copy of the game.

## Progress

![Progress: functions and code running in Rust, by area](docs/img/progress.svg)

The original functions are replaced one at a time while the game runs. Each replacement is checked against the original in-game before it counts. The file system (archives, open/read/write/close) and the main memory-pool allocators are done; see `docs/PLAN.md` for the milestones.

Update the graphic after porting something: `python -I tools/progress.py bin/SR2_pc.unpacked.exe docs/img/progress.svg`

## Layout
- `docs/PLAN.md`: strategy and milestones
- `docs/analysis/`: findings from Ghidra/REA
- `tools/`: analysis helpers (`unsteamstub21.py` strips the SteamStub 2.1 DRM from your own `SR2_pc.exe` so it can be analyzed statically; `rtti_dump.py` lists the C++ classes; `progress.py` draws the progress graphic; `harvest_names.py` adds auto-recovered names to `docs/functions.csv`: script bindings, Ghidra library matches, vtable slots)
- `crates/sr2-formats`: game file formats (`.vpp_pc`, …)
- `crates/sr2-hook`: proxy `dinput8.dll` loaded into the original exe, used to swap original functions for Rust ones one at a time

## Building
Requires rustup (stable, MSVC) and VS 2022 Build Tools with the C++ workload.

```sh
rustup target add i686-pc-windows-msvc
cargo test -p sr2-formats
cargo build -p sr2-hook --target i686-pc-windows-msvc --release
```

## Setup
```sh
python tools/unsteamstub21.py "<SR2 install>/SR2_pc.exe" bin/SR2_pc.unpacked.exe
```
