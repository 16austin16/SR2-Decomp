# SR2-Decomp

A reverse-engineering project rewriting Saints Row 2 (PC, Steam) in Rust. This repo has
no game files; you need your own copy of the game.

## Progress

![Progress: functions and code running in Rust, by area](docs/img/progress.svg)

The original functions are replaced one at a time while the game runs. Each replacement is checked against the original in-game before it counts. The file system (archives, open/read/write/close) and the main memory-pool allocators are done; see `docs/PLAN.md` for the milestones. The world streamer (`streamman`: request queue, per-frame pump, loading infos, chunk loader and its 39 section types, and the "hdd thread" read scheduler underneath) is mapped in `docs/analysis/05_streamer.md` and is the next system to port.

The graphic also tracks virtual methods of the game's own classes separately (Havok left out), since renaming a `Class::vfN` placeholder does not change the identified count. Functions are identified mostly by `tools/harvest_names.py`: Lua script bindings, Ghidra library matches, RTTI vtable slots and constructors/destructors. Virtual methods worked out by hand are recorded per base class in `docs/vtable_slots.csv`, and the name is applied to every subclass's override (for example `fuzz_crime::get_bar_title` names that method in all 44 crime types). The game's own Direct3D 9 wrappers (`IDirect3DDevice9Lost`, `gr_vertex_buffer`, `gr_index_buffer`) are named from the COM method order in `d3d9.h`. The statically linked Lua 5.0 runtime is named from its own error messages (for example "loop in settable" is `luaV_settable`). The statically linked GameSpy SDK is matched against its published source by unique strings and constants, call order and link order (`docs/analysis/24_gamespy.md`). Network packet handlers are named from the game's own packet tables, which carry a debug name per packet type (`docs/analysis/25_network.md`).

Methods that can't be named statically can be named from their callers: `python -I tools/gen_caller_targets.py bin/SR2_pc.unpacked.exe` lists the unnamed virtual slots, and building the hook with `--features caller-survey` (after adding `mod callers; mod caller_targets;` and `crate::callers::install()` at the end of `install_all`) logs every caller to `sr2-callers.log` in the game folder. `python tools/caller_report.py bin/SR2_pc.unpacked.exe <log>` then shows each method's callers with their names and strings.

Update the graphic after porting or naming something: `python -I tools/progress.py bin/SR2_pc.unpacked.exe docs/img/progress.svg`

## Layout
- `docs/PLAN.md`: strategy and milestones
- `docs/analysis/`: findings from Ghidra/REA
- `tools/`: analysis helpers (`unsteamstub21.py` strips the SteamStub 2.1 DRM from your own `SR2_pc.exe` so it can be analyzed statically; `rtti_dump.py` lists the C++ classes; `progress.py` draws the progress graphic; `harvest_names.py` adds auto-recovered names to `docs/functions.csv`: script bindings, Ghidra library matches, vtable slots)
- `crates/sr2-formats`: game file formats (`.vpp_pc`, `.peg_pc`, `.chunk_pc`, …); `cargo run -p sr2-formats --example chunk -- stats <chunks*.vpp_pc>` summarises the world chunk sections
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
