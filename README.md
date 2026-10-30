# SR2-Decomp

A reverse-engineering project rewriting Saints Row 2 (PC, Steam) in Rust. This repo has
no game files; you need your own copy of the game.

## Layout
- `docs/PLAN.md`: strategy and milestones
- `docs/analysis/`: findings from Ghidra/REA
- `tools/`: analysis helpers (`unsteamstub21.py` strips the SteamStub 2.1 DRM from your own `SR2_pc.exe` so it can be analyzed statically)
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
