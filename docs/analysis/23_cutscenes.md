# 23: Cutscenes and video

Names are in `docs/functions.csv` (subsystem `cutscene`). Game states are in 11_frame.md and audio / Bink sound in 10_audio.md.

## In-engine cutscenes

**Starting.** Lua `cutscene_play_do` (`luafn_cutscene_play_do` 0xA2CE90) builds the request and calls **`cutscene_play`** (0x6C3BA0; ECX = up to 8 cutscene name hashes, (cutscene, flags, skippable)).

1. **When a cutscene is already playing** (`0x2527D23`): stop the frame hand-off (0x522B80), unload the 8 cutscenes' content collections from the streamer (0xA57770, 0x6BE320), clean up (0x6CDEF0), and fade in (0x4D1270). Return.
2. **Otherwise**:
   1. `cutscene_find` (0x6C39D0) finds the cutscene in `cs_camera_list.xtbl` / the cutscene table.
   2. Holster every human's weapon (0xB72EF0 / 0xB72800, 0x9C8C20).
   3. Set skippable `0xE8DD22` and the flags `0xE8DD1F`.
   4. In single player / as host, **stream the cutscene** (`cutscene_stream_request` 0x6BE8F0).
   5. Allocate the **cutscene player** (0x43E8 bytes) from the pool `0x2771178`: `cutscene_player_create` (0x6BDC80) → `0x2527D10`. Its +0x1664 gets the 8 cutscene hashes.
   6. Set the play state: `0x2527D14` = playing; clear `0x2527CF7`, `0x2527D15`, `0x2527D1C`-`0x2527D21`.
   7. Close the pause menu if it is open ("pause_menu_exit" thread).
   8. Fade to black (0xD14840, 0x4D1090).
   9. Install the cutscene frame callback `0x2527D18` = 0x6BFB50.

`luafn_cutscene_play_check_done` reports when it has finished.

**While playing**, `frame_end_handoff` and the renderer check `0x2527D14` / `0x2527CF7` / `0x2527D15` (the "cutscene active" test that turns off some world processing, `0xE94D37`).

**Content and format**:
- Cutscene data streams in through the normal streamer as content collections (`cds_<name>_collection`, 22_missions.md) and the `.cts` files seen in the streamer.
- The camera track uses "camera pos start / end", "camera lookat start / end", "Use Camera Lookat Bone / Position Bone", "Camera moves with target".
- `cutscenes.vpp` holds the debug cutscene list.
- `credits_in_cutscene`, `should_skip_intro_cutscene` and `start_intro_cutscene` are related Lua bindings.

## Pre-rendered video (Bink)

- Game **state 11** (11_frame.md) plays full-screen Bink video:
  - enter 0x47F080: pauses the audio categories and clears `0xE94D3E`;
  - frame `video_frame` (0x688FE0): `binkw32` `BinkDoFrame` / `BinkNextFrame` / `BinkWait`, and copies the frame into a texture;
  - exit `video_stop` (0x689130).
- Videos come from `./videos/` (`fullscreen_video.xtbl`, `xtbl_load_fullscreen_video`). Lua uses `bink_play_movie_do` / `bink_movie_done`.
- Bink's sound goes through DirectSound (`BinkSetSoundSystem(BinkOpenDirectSound)`), or waveOut as a fallback (10_audio.md).
- `video_frame` is also called from `sim_frame:5`, for in-world video surfaces.

## Open

- The `.cts` / cutscene data layout and the per-frame player (0x6BFB50 and the 0x43E8-byte player).
- How cutscene skipping is handled.
