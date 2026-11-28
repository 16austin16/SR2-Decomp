# SR2-Decomp

Saints Row 2 (PC, Steam) being rewritten in Rust, one function at a time, while the game runs.

This repo contains no game files. You need your own copy of the game.

## Progress

![Progress: functions and code running in Rust, by area](docs/img/progress.svg)

- **4,388 functions run in Rust**, about 15% of the game's functions and 7% of its code.
- **14,633 functions are identified** (named), about half of the game.

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

## In-game menu (F7)

Press **F7** in game to open the test menu. The game ignores the controls while the menu is open.

| Key | Action |
|---|---|
| Up / Down | move between items |
| Left / Right | change a value (weather, time, mission, wanted level) |
| Enter | choose or apply |
| Backspace | back to the main page, or close |

**Player options**
- **God mode:** the player can't be hurt. It stays on through cutscenes and mission starts.
- **Infinite ammo:** endless reserve ammo and grenades. Clips still empty and reload.
- **Refill health.**
- **Add $10,000.**
- **Add 5,000 respect.**
- **Clear wanted level.** This clears police and gang notoriety.
- **Set wanted level:** pick 1 to 5 stars of police notoriety, then press Enter.
- **Teleport to waypoint:** goes to the map waypoint.
- **Time:** moves the clock an hour forward (Right or Enter) or back (Left).

**World and cheats**
- **Weather:** pick a weather type and apply it, or choose "normal" to hand weather back to the game.
- **Game cheats:** the game's own cheat list. Toggle cheats on and off; one-shot cheats are marked "(once)".

**Shops**
- Lists every shop chain, such as Brown Baggers, Friendly Fire, Impressions and Rusty's Needle. The list comes from the game's own shop table, 98 shops in all.
- Pick a chain to see its locations with their distance from you. Enter teleports you to the shop's purchase spot.

**Mission logging** records what the game does during a mission, for analysis.
- **Logging:** turns the logger on or off.
- **Mission:** picks which mission the log is labelled with.
- **Mark point:** writes a labelled marker into the log. **F8** does the same from anywhere: it pauses the game and opens a box to type the label. Enter saves it; Esc cancels.

The log is saved when the game exits or crashes.

### Testing switches

- **Turning off ports:** create `sr2-skip-modules.txt` in the game folder, with one name per line. Each line can be any of these:
  - a module name, such as `port_small_a`;
  - a single function name or address, such as `vehicle_is_helicopter` or `0xab5190`;
  - `all_ports`, for a nearly vanilla game that keeps the menu.
- **Crash reports:** after a crash, `python tools/crash_report.py` reads `sr2-hook.log`. It shows the last ported functions that ran and names every address on the stack.

## Repository layout

| Path | What it is |
|---|---|
| `crates/sr2-hook` | the hook DLL and all the Rust ports (`port_*.rs`) |
| `crates/sr2-formats` | readers for the game's file formats (`.vpp_pc`, `.peg_pc`, `.chunk_pc`, ...) |
| `docs/functions.csv` | every known function: address, name, area and status |
| `docs/analysis/` | write-ups of how each part of the game works |
| `docs/skipped_ports.md` | functions not ported yet, and why |
| `docs/TODO.md` | work agreed for later |
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
