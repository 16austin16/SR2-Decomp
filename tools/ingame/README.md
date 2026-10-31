# In-game test scripts (Windows PowerShell)

- `sr2input.ps1 <shot|key|click|focus> [arg]`: drive the running game. `key` takes hex scancodes (`1C` Enter, `01` Esc, `2A` Shift, `11/1E/1F/20` WASD, `E050` = extended Down); `shot <png>` saves a half-size crop of the game window only. SendKeys and mouse clicks don't reach the game; scancode `SendInput` does.
- `run_city.ps1 <label>`: deploy `target/.../dinput8.dll`, boot, Continue into the save, wait for frame 3600, close; copies the logs to `bin/<label>.*`.
- `run_teleport.ps1 <label>`: needs `TELEPORT_TEST = true` in hooks.rs. Boot, Continue, then the hook teleports the player around 8 points 600 m apart; screenshots after each jump; logs to `bin/<label>.hook.log` and the streamer events to `bin/<label>.events.log`.
- `run_save.ps1 open|save <label>`: boot and open the Save Game list (screenshot to check NEW SAVE GAME is highlighted), then save to a new slot. Back up `%LOCALAPPDATA%\THQ\Saints Row 2` first, and delete test saves while the game is running (Steam Auto-Cloud restores files deleted while it's closed).

Disassembly helpers in `tools/`: `x86dis.py <hexaddr> <hexlen>`, `xrefs.py <hexaddr>...`, `check_prologues.py` (checks the mempool detour prologues), `cmp_fileopen.py` (compares per-call log results between two runs). They need `pip install --user capstone` and must run without `-I`.
