# In-game test scripts (Windows PowerShell)

- `sr2input.ps1 <shot|key|click|focus> [arg]`: drive the running game. `key` takes hex scancodes (`1C` Enter, `01` Esc, `2A` Shift, `11/1E/1F/20` WASD, `E050` = extended Down); `shot <png>` saves a half-size crop of the game window only. SendKeys and mouse clicks don't reach the game; scancode `SendInput` does.
- `run_city.ps1 <label>`: deploy `target/.../dinput8.dll`, boot, Continue into the save, wait for frame 3600, close; copies the logs to `bin/<label>.*`.
- `run_teleport.ps1 <label>`: needs `TELEPORT_TEST = true` in hooks.rs. Boot, Continue, then the hook teleports the player around 8 points 600 m apart; screenshots after each jump; logs to `bin/<label>.hook.log` and the streamer events to `bin/<label>.events.log`.
- `run_callers.ps1 <label>`: caller survey. Build with `--features caller-survey` first; boots, Continues, walks and opens the pause menu, waits for the survey log, closes; copies `sr2-callers.log` to `bin/<label>.callers.log` for `tools/caller_report.py`. Redeploy the normal build afterwards.
- `run_save.ps1 open|save <label>`: boot and open the Save Game list (screenshot to check NEW SAVE GAME is highlighted), then save to a new slot. Back up `%LOCALAPPDATA%\THQ\Saints Row 2` first, and delete test saves while the game is running (Steam Auto-Cloud restores files deleted while it's closed).

- `run_menu.ps1`: boot, Continue, then drive the F7 menu (god mode, cash, respect, logging on, two F8 marks); screenshots to `bin/menu_*.png`, logs to `bin/menu.hook.log` / `bin/menu.mission.log`.

## In-game menu (F7) and mission logger

The hook DLL has a menu (crates/sr2-hook/src/menu.rs) in the bottom-right corner. F7 opens it; Up / Down move, Enter selects, Left / Right change a value, Backspace goes back. While it is open the game gets no buttons or sticks.
- **Player options**: god mode (invulnerable flag plus full health, re-set if a script clears it), infinite ammo (the game's own Infinite_Ammo and Unlimited Clip flags at 0x27A6A40, set directly so no cheat is recorded), refill health, add $10,000, add 5,000 respect (the spendable respect at player +0x1200), clear the wanted level (`notoriety_set` 0x55F870 to 0 for all 4 teams), teleport to the waypoint set on the map (Tab opens it; the GPS destination at 0x29BA748; tested on foot and in a vehicle), time of day (Left / Right = -1 / +1 hour, through the clock advance 0x68A1E0 that `set_time_of_day` uses). The status line shows the address space in use: the game is not large-address-aware, so it has 2 GB, and about 1.8 GB is in use once the city has loaded (the hook log also gets a `memory:` line every 3600 frames).
- **World and cheats**: weather (Left / Right pick, Enter applies; "normal" goes back to the weather cycle) and the game's own cheats as a scrolling list (Enter toggles; cheats may affect achievements). From docs/analysis/38_cheats_and_menu_hooks.md.
- **Mission logging**: logging on / off, the mission number you are on (Left / Right), and Mark point. **F8 marks a point without opening the menu.**

While logging is on, a thread samples where the game and render threads are about once a millisecond (crates/sr2-hook/src/missionlog.rs). A new section starts at every marked point, mission number change, and change of the game's own active mission. Sections are appended to `sr2-mission.log` in the game folder. `python tools/mission_report.py <log>` lists the busiest functions per section, and names the unnamed ones by the nearest named function below them.

## Display: any resolution and borderless (`sr2-hook.ini`)

The hook DLL reads `sr2-hook.ini` in the game folder (written with these defaults if missing):

```
display = borderless      # borderless | fullscreen | windowed | game (leave it to the game's options)
resolution = desktop      # desktop, or WIDTHxHEIGHT such as 2560x1440
```

At load the DLL makes the process DPI-aware and writes the resolution and fullscreen flag into `settings.dat` before the game reads it (backup: `settings.dat.bak`), since the game only picks its 4:3 or widescreen UI at boot. Borderless is windowed mode with the window restyled to a bare popup covering the screen. Changing the resolution in the game's own options is overridden at the next start while `display` is not `game`. Code: crates/sr2-hook/src/display.rs.

## Game exe: large-address-aware (4 GB)

The game folder's `SR2_pc.exe` is our DRM-stripped `bin/SR2_pc.unpacked.exe` (made by `tools/unsteamstub21.py`) with the large-address-aware flag set, so the game gets 4 GB of address space instead of 2 (it uses about 1.76 GB in the city; the hook log shows `memory: X of Y MB`). Setting the flag on the Steam exe does not work: SteamStub rejects the modified file. The original is `SR2_pc.exe.bak` next to it (sha256 e20156a0…); copy it back to undo. A Steam "verify files" also restores it.

Disassembly helpers in `tools/`: `x86dis.py <hexaddr> <hexlen>`, `xrefs.py <hexaddr>...`, `check_prologues.py` (checks the mempool detour prologues), `cmp_fileopen.py` (compares per-call log results between two runs). They need `pip install --user capstone` and must run without `-I`.
