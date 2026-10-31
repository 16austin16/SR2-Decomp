# 13: Save games

Names are in `docs/functions.csv` (subsystem `save`).

## Where

`WinMain` builds the save folder:
1. `SHGetFolderPathA(NULL, CSIDL_LOCAL_APPDATA (0x1C), NULL, 1, 0x144E650)`.
2. `CreateDirectoryA("%LOCALAPPDATA%\THQ")`, then `"%LOCALAPPDATA%\THQ\Saints Row 2"`.

`0x144E650` is the folder every save path is built from (`"%s\\%s%s"`).

| File | Contents |
|---|---|
| `<name>.bbsave` | one save game (below) |
| `profile.dat` | the player profile: three blocks, each a `u32 size` then `size` bytes. Read and written with `fopen` by `profile_load` (0x6910A0) and 0x691290. When the file is missing it is created with defaults. |

## The .bbsave file

A save is a **raw dump of one fixed-size struct** (`bb_save_game`, the name comes from RTTI `save_triggers(bb_save_game *, bool)`). It is `[0xEAAC38]` = **0x3AC00 bytes** (240,640).

| Off | Field |
|---|---|
| +0x0 | **CRC32** of bytes 4 … size-1: the standard reflected table at `0xE843B8`, initial value 0, no final xor (`gamesave_write_file` 0x6951B0) |
| +0x4 | **version**, currently 0x6C (108) |
| +0x4E … +0x6E | date and time as wide digits (used for the file name) |
| +0x74 | current mission / location id (looked up in `0x27AC860`, 0xE8-byte records) |
| +0x78 | progress counter (file name field 3) |
| +0x98 | the allocator used while saving (a runtime pointer that is written to the file; ignored on load) |
| +0xA8 | flag (file name field 9) |
| +0x2E2 | co-op flag |
| +0x17304 / +0x1730C | a count and 12-byte entries (the file name counts entries whose first dword is 0) |
| … | the 28 system sections (below) |

Writing:
1. 0x6951B0 computes the CRC.
2. `file_open` the path with the mode at 0xE00638, then `file_write` (0xBFE360) the whole struct.

There is no compression and no per-section header. The offsets are fixed by the struct.

**The file name carries the save list metadata.** `gamesave_build_name` (0x695450) writes it as:

```
_%03i%02i%03i%02i%03i%02i%01i%05i%01i%s
 version, mission index, +0x78, 0x6A7A60(-1,-1), count of zero entries at +0x1730C,
 crib index (0x549010), flags (bit 0 co-op +0x2E2, bit 1 autosave), play time in minutes (0x140E738/60),
 +0xA8, 12-character date/time
```

- The display name is "`<mission> <date> <time>`", or "`Autosave <date> <time>`".
- The save menu lists saves from the names alone. `gamesave_scan` (0x691C50) runs `findfirst("*.bbsave")` (up to 0x200 files) and parses each name with `gamesave_parse_name` (0x695740): '_' prefix, digit fields read by 0x6953C0, the mission name from `0x144DFC0`, the photo name (default "photo_prison_lighthouse"), and date / time. It then reads the file into a "gamesave" pool and records it with 0x691910.
- `gamesave_delete` (0x695150) calls `DeleteFileA` on `<name>.bbsave`.

## Save and load flow

**`gamesave_save`** (0x695A60): (info, allocator or NULL, autosave)
1. 0x6B56A0, 0x691290, 0xA917E0 / 0xA91780.
2. Allocate the buffer from a "gamesave_saving" pool (1 MB) and zero it. The co-op path reuses `0x27B3C94` (0x6916E0).
3. `gamesave_collect` (0x6959E0) calls each section's save function.
4. `gamesave_build_name`.
5. `gamesave_write_file`, then release the pool.
6. On failure, `gamesave_fail` (0x695280) sets the error state `0xE96BB8` and deletes the partial file.

Callers include 0x696090 (autosave, "SAVELOAD_AUTOSAVING"), 0x6A1440 (mission start / end), the save menu 0x7787C0 / 0x778A40, and the co-op host.

**`gamesave_apply`** (0x695F00): EAX = the loaded buffer.
1. For each section whose **minimum version ≤ the file's version (+4)**, call its load function.
2. Then 0x9F58F0 for every item in `0x21F2AA4`, and 0x6B5310.

When `0x2527BBA` is set it runs 0x693EB0 instead.

## The section table

`0xDD3050` holds 28 entries of 0x14 bytes:

| Off | Field |
|---|---|
| +0x0 | save function (buffer in ECX / stack) |
| +0x4 | load function |
| +0x8 | save for a co-op client |
| +0xC | load for a co-op client |
| +0x10 | minimum file version |

The co-op variants (+0x8 / +0xC) are used when a multiplayer session is active and `0x80FCF0` says this machine is not the host.

| # | Save / load | Buffer offsets | Subsystem (from the callee and its strings) |
|---|---|---|---|
| 0 | 0x692E80 / 0x694AB0 | +0x37678 count, +0x3767C 12-byte entries | game timers: 0x542940 saves each active timer as time left relative to the clock `0xE843AC` (wraps at 1.8e9 ms) |
| 1 | 0x693570 / 0x694F30 | +0x38504 / +0x38508 | world streaming state (0xA7C910, streamer area); has co-op variants |
| 2 | 0x691ED0 / 0x6937C0 | +0x6C … +0x19C | game and player core ("sr2_city"; 0x6916E0) |
| 3 | 0x692300 / 0x693FE0 | +0x10BC … +0x18AC | |
| 4 | 0x692590 / 0x694390 | +0x17304 / +0x17310, +0xA8 | the 12-byte entry list from the file name; has co-op variants |
| 5 | 0x6935C0 / 0x694F80 | +0x3A148 | interface / options (0x7DECD0); has co-op variants |
| 6 | 0x692040 / 0x693BF0 | +0x17270 … +0x1727C | |
| 7 | 0x692630 / 0x694470 | +0x175D8, +0x37270 | garage vehicles (0x5554F0, load 0x5558F0 / 0x5556F0) |
| 8 | 0x692A00 / 0x6946D0 | +0x37274 / +0x37278, +0x376B8, +0x379C0 | mission state (load 0x6A7710 starts script threads, 0x53C860) |
| 9 | 0x692B40 / 0x6947E0 | +0x74 / +0x78 | activity levels ("level", "crowd_control_", 0x696B50 / 0x69E1C0); has co-op variants |
| 10 | 0x692B80 / 0x694800 | | 0xA6EA00 (player / teleport area) |
| 11 | 0x692D60 / 0x694920 | +0x382A4 … +0x38500 | cribs (load 0x549C90); has co-op variants |
| 12 | 0x693470 / 0x694E40 | +0x385F8 | |
| 13 | 0x6934A0 / 0x694E70 | +0x38810 … | |
| 14 | 0x692EA0 / 0x694AD0 | +0x38C38 | 0x6AB5C0 |
| 15 | 0x692EC0 / 0x694B10 | +0x39B68 | 0x6C3AD0 / 0x6C3B30 |
| 16 | 0x692F00 / 0x694AE0 | +0x39B88 | 0xA8CE10 / 0xA8CEB0 |
| 17 | 0x692F30 / 0x694B50 | +0x39CB8, +0x3A0C8 … | rewards / unlocks (load 0x605CA0: "REWARD_UNLOCKED", phone messages) |
| 18 | 0x692FB0 / 0x694BD0 | +0x39DB8 | activity instances (0x563040, guardian angel); has co-op variants |
| 19 | 0x692FD0 / 0x694BF0 | +0x120 … +0x1894 | large; stats |
| 20 | 0x693450 / 0x694E20 | +0x39DC4 / +0x39DC8 | 0x688220 / 0x688260 |
| 21 | 0x692760 / 0x6944E0 | +0x38830 … +0x388C8 | diversions (load 0x680CD0, 0x5C2530, 0x5E7480, 0x601330) |
| 22 | 0x692750 / 0x6944C0 | +0x379C8 | mission start triggers (load 0x54CCA0) |
| 23 | 0x693720 / 0x6950C0 | +0x3A16C | radio (0x48B7D0); has co-op variants |
| 24 | 0x693730 / 0x6950D0 | +0x3A1CC | radio / audio player (0x491140) |
| 25 | 0x693740 / 0x6950E0 | +0x3A9CC | audio player playlist (0x491200) |
| 26 | 0x693760 / 0x6950F0 | +0x3AA0C | 0x7AEBD0 / 0x7AED60 |
| 27 | 0x6937A0 / 0x695130 | +0x3AA68 | 0x7B07F0 / 0x7B08C0; **version 0x6C only**; has co-op variants |

Sections write to fixed offsets of the struct and do not check each other's ranges. A Rust port can treat the save as a `#[repr(C)]` struct of 0x3AC00 bytes, check the CRC and version, then apply the sections in table order.

## Open

- The subsystems of sections 3, 6, 10, 12-16, 20, 26 and 27 (the save and load functions are listed).
- The exact field layout inside each section.
- `profile.dat`'s three blocks (0x690DA0 parses them).
