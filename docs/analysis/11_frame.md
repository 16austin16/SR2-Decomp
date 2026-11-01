# 11: The frame: game_frame_tick, game states and sim_frame

Names are in `docs/functions.csv` (subsystem `app`). Threads are described in 07_render.md.

## Threads in one frame

| Thread | Runs |
|---|---|
| game thread (`game_main_thread_proc` 0x520790) | `game_frame_tick`: input, timing, the game state machine, `sim_frame`, UI. At the end it hands the frame to the render thread. |
| supervisor / render thread (`app_supervisor_thread_proc` 0x520A30) | `game_render_thread_frame` (0x522640): waits for the hand-off, builds batches, `render_frame`, writes D3D commands |
| WinMain (device) thread | `gfx_execute_commands` |
| two job workers (control blocks `0x33D63F8` and `0x33D64C8`, 0xD0 bytes each) | jobs from the job system (0xBF3F50 creates a job tagged with source file and line). `job_worker_wake` (0xBF3760): if the worker is idle (+0x30 == 2), SetEvent(+0x40) and mark it running (3); +0x38 holds the work kind (4 or 3). |
| audio thread (`audio_thread_proc` 0x46C900) | XACT DoWork, listener and emitters (10_audio.md) |

## game_frame_tick (0x520450)

1. Profiler sections "sim_frame" and "simfr:1,2,3,4,5,6".
2. If the frame counter `0x252A3C0` is past 200 and `0xFA23A0` is set, return 1 (quit).
3. If `0x252A26F` is set, clear it, then run the vint `screen_change_failed_pc` thread for the document whose +0x48 matches (09_script.md).
4. 0x699C90, then 0xC063D0 (input / controller poll).
5. `vint_input_do_frame`, unless input is blocked (`0x2528B24 > 0`, `0x1F768DC == 0`, `0x2527FEB == 0`).
6. 0x4FE760, 0x500CA0, 0xC1F320, 0x753AA0 (dialog boxes), 0x69B160 (pause menu / controller disconnect).
7. **`frame_timer_update`** (0xBF0710), see Timing below.
8. 0x8217F0, 0x809640 (GameSpy / network pump), 0x691430 (profile.dat / .bbsave saves, see 13_savegame.md), 0x69A440.
9. **`game_state_frame`** (0x697780).
10. Profiler "sim-hand", then **`frame_end_handoff`** (0x521A00).

## Timing

`frame_timer_update` (0xBF0710) is called as (fixed, step, extra): `extra` is `0xE98AF0` when `0x2527321` is set, else 0. (Earlier this read (fixed, ?, step); the code at 0x520569 stores the step in the second argument.)

1. Real dt = QPC time since the last frame, in microseconds / 1e6 → `0xE84380`.
2. A non-zero `step` overrides dt with 1/step.
3. **Frame dt** `0xE8437C` = min(dt, max dt `0xE84394`).
4. In fixed mode (`fixed` = 1), it sleeps to the next multiple of the step and adds the sleep time to dt.
5. A 16-frame running average gives fps in `0xE84388`.
6. dt is divided by the time scale `0xE8438C` (slow motion).
7. Game time in ms accumulates into `0x252A3C8` unless paused (`0x252A3E0`). Real ms accumulate into `0x252A3CC`. The frame counter is `0x252A3C0`.

`game_frame_tick` passes `fixed = 1` and `step = 0x2527DA4 / 0xE8ECAC` (the physics step) when `0x25272DE` is set and `0x25273B4` is clear. Otherwise it passes `fixed = 0`.

## Game states

`game_state_frame` (0x697780) runs a stack of states. The stack is `0x144E760[depth]`, with depth in `0xE96B7C`. The pending state is `0x144E75C`; `0x140BBCA` means push and `0x140BBC9` means pop. Per-state flags are in `0x144E750[state]`.

1. If a change is pending, call the current state's **exit** (`game_state_exit` 0x6973E0, table `0x697458`, index state-1). Exit is skipped when the new state is pushed on top.
2. `game_state_apply` (0x697550) pushes, pops or replaces the stack.
3. If an aspect ratio change was flagged (`0x252A26E`), call 0xB8A760(aspect > 1.45).
4. Call the new state's **enter** (`game_state_enter` 0x697330, table `0x6973A8`). Enter is skipped on a pop.
5. If the top state's flag bit 0 is set, the state below keeps running too (0x697700).
6. `game_state_dispatch` (0x697490, ESI = state):
   1. `overlays_update`, except in state 4.
   2. The state's frame function.
   3. `overlays_render`, except in states 2 and 6.

| State | Enter | Exit | Frame | What it is |
|---|---|---|---|---|
| 1 | - | - | - | (boot / none) |
| 2 | 0x73D900 (front-end music) | 0x73DB40 | 0x73DCB0 | main menu / front end |
| 3 | 0x699840 (pauses audio) | 0x699910 | - | suspended / paused |
| 4 | - | - | - | (no overlays) |
| 5 | 0x68DE50 | 0x68E560 | `gameplay_do_frame` 0x68E1B0 | **gameplay** |
| 6 | interface screen 0xB or 0xD (0x7EC280) | screen -1 | - | full-screen UI screen |
| 7 | 0x830A40 ("COOP_WAIT") | 0x830DA0 | 0x830AB0 | co-op waiting |
| 8 | 0x834960 (matchmaking) | 0x8352F0 | 0x834D40 | multiplayer lobby / matchmaking |
| 9 | 0x82F3C0 | 0x82F740 | 0x82F570 | multiplayer setup |
| 10 | 0x73B430 (starts a load thread) | 0x73B520 | 0x73B7F0 | loading screen |
| 11 | 0x47F080 (pauses audio), clears `0xE94D3E` | 0x689130 | 0x688FE0 (Bink) | full-screen video |

The state names come from their strings and are best guesses where marked by a dash.

## gameplay_do_frame (0x68E1B0, state 5)

1. Profiler "gameplay_do_frame:1".
2. Warning popups: 0x7F4E90 (foreign characters), 0x6B5810 (corrupt stats).
3. Follower / net-id Lua callbacks.
4. 0x522B80, then 0x68D570 (quit / co-op prompts) and 0x81EDF0 (multiplayer errors).
5. **`sim_frame`** (0x68CA70).
6. Profiler "gameplay_do_frame:2".
7. 0x68C770 (navpoints), `gameplay_ui_frame` (→ `hud_render`), 0xD14840 / 0xD13A60 (2D flush).

## sim_frame (0x68CA70)

The order of calls, grouped by the profiler sections. Unnamed calls are listed with what their strings or callees show.

**"sim_frame:1,2,3": world and script**
1. 0x6C6870: resource scheduler / streamer update (`rsched_update_all`, `streamman_process_generic`).
2. 0x493260, 0x4932F0 (camera), 0xA70B40, 0xA55310, 0xA66340 (cell phone).
3. 0x543920 (race / derby HUD), 0x68F4B0 ("mission start") and 0x93D150, twice.
4. **`script_do_frame`**: Lua threads (09_script.md).
5. 0x6DA350, 0x54CCA0 (mission start triggers), 0x6B7020 (tutorials), 0x6905E0, 0x75A120 (control prompts).
6. 0x4F5040, 0x6AB6B0 / 0x6AB240 / 0x6AB2D0 (debug script files), 0x765BB0 (map waypoints).

**"sim_frame:1b"**
- 0x68A1E0, 0x68A320, 0x68B860.
- 0xA17690, 0xA1CCC0, 0xA0AFF0, 0xA0BF10 (detour hulls).
- 0xA6C350 (teleports), 0xA58EE0 (restricted zones).
- 0x501FB0 (debug console commands), 0x6A5BB0 (strongholds), 0x7B02E0, 0x548260 (crib tutorial), 0xA93FF0.

**"sim_frame:1c": multiplayer and players**
1. `multi` ownership migration (0x8045F0) and `remote_do_frame` for every object owned by another peer (0x804850), twice (08_multi_objects.md).
2. 0x6FF860, 0xB08530, 0x6ADB10, 0x9D7F90 (co-op revive).
3. 0x9A7080 / 0x9A7150 / 0x9A71B0, 0x9305F0, 0x935820.
4. 0x437160, 0x55BA40, 0x56BD60, 0x4B11F0 (neighborhoods), 0x5507C0, 0x9F5420, 0x655750, 0x542B00, 0x565DA0 (phone calls).
5. **`diversion_mode_frame`**.
6. 0xA95A30, 0xADD120, 0xAEF510.

**"sim_frame:4": animation, physics, humans and vehicles**
1. 0xAE3A80, 0x92B3B0, 0x9FA620.
2. `chunk_object_anims_update`, 0xB80070, 0xB7CA80, 0x9DF820.
3. Wake both job workers (kind 4).
4. 0x554E20, 0x6FF860, 0x9A9890 (players).
5. **`sim_physics_step`** (0x6FA0E0, profiler "sim_frame:4-HavokWater"):
   1. Add the frame dt to the accumulator `0x2527DC8`. The step is `0x2527DA4 / 0xE8ECAC`.
   2. Up to N substeps (N from 0x6F5D60), each one:
      1. 0xAE48A0(step).
      2. `0xD4B4B0(hkpWorld = [0x2527D94]+8, step)`, the Havok world step.
      3. A havok.cpp job running 0x6F5BB0.
      4. Human processing 0x9A9B60(step, 1).
   3. Leftover time is clamped by 0x4DA560.
   4. Post-step: 0x6F9FD0, 0x717E00, 0x6F57A0, 0x70B200 and 0x6F5AE0. `0x2527DAC` counts the steps in progress.
6. 0x9A9B60(dt, 0) (human processing after the step), 0x4C2BF0.
7. Wake both job workers (kind 3), then 0x9A9CE0 and 0x71B910.

**"sim_frame:5": objects**
- 0xA8B400, 0x95A3D0.
- 0xAE3C60 (vehicles: submerged, destroyed), 0xB095D0, 0x634350, 0x9A7210 (humans), 0x94C0A0.
- 0x688FE0 (video), 0xB72FD0, 0x4BD790, 0xB6A4A0, 0x4D1D50, 0x930EC0, 0x9FA670, 0x9FF5A0, 0xA5E3E0, 0x935B80.
- 0x490940 (police / FBI radio), 0x489CC0, 0x4AAC40 (fire truck spray).
- `chunk_object_attachments_update`, 0x9193C0, 0x5585E0.
- The camera and cell-phone group again (0x4932F0 … 0xA66340).

**"sim_frame:6,7": effects, world and UI**
- 0xB6AD40 (target lock), 0x4DE770 (weather / time of day), 0x4D6850, 0x4E66A0, 0x4CF840, 0x4CA750 (lightning), 0x503AE0, 0x4CBFC0 (motion blur).
- 0x55ADF0, 0x727610, 0x955470, 0x5053B0, 0x5142A0, 0x91A840 (interaction icons), 0x4525E0 / 0x452820 / 0x453120, 0x9DF4C0.
- 0x926170 (co-op), 0x4C6CD0, 0x4C35C0, 0xD07B40, 0x4DA880, 0x92D620, 0x9FCF90 (movers), 0xB82180.
- 0xD22790 ×2 (CPU virtual pool), 0xB821E0.
- 0x6BCC60 (rewards), 0x54C220 (difficulty), 0x687B40, 0x55F1D0 (notoriety), 0x7F7AD0, 0x7A8F70 (confession / forgiveness).
- 0x4BC420 (streaming wait helpers), 0x4C4210 (`XInputSetState` rumble), 0x91E9F0, 0x4D9F20 (explosions / weapons refresh), 0x4C2DD0, 0x683E00, 0x9E99F0, 0x926860, 0x836BB0.
- 0xD24B70 / 0xD24D80 with 0x793A40 and **`interface_do_frame`**: the UI update, once per viewport.

## frame_end_handoff (0x521A00): render submit

1. 0x522B80. If the render thread has not finished the last frame (`0x27741AC`), spin until it has.
2. 0xCFBC00, then 0xD14950 / 0xD14770 / 0xD14700 (begin 2D).
3. In gameplay (state 5): `world_streaming_do_frame` (0x68E410).
4. 0x5217E0 → `gfx_present` (end of the frame's D3D commands).
5. In gameplay: `audio_update_voices_on_workers` (10_audio.md).
6. 0x521970, 0x5218D0, 0x5220B0, 0x521870.
7. **Render snapshot.** When the world is visible (`0xE94D3E`):
   1. Copy the camera (`0x25F5B14..`, matrix `0x25F5B38`) and FOV into the render frame block `0xE9D620..`.
   2. Lighting, sky and weather (0x4D9DD0, 0x4D4820 … 0x4E69D0), effects (0x46CCD0, 0xD08310, 0x4C7190), 0x728AB0, 0x51A880, 0x51A4A0, 0x9FAA20.
   3. Pick the listener / LOD distance by the camera mode.
   4. Then 0xAE1020, 0x52E7C0, 0x5250E0, 0x522FC0.
8. **Double buffer swap**: exchange `0x2305978` and `0x230597C` (the deferred render-state command lists, 18_scene.md) and clear their counts.
9. Finish the audio job (`job->wait`) and release it.
10. Signal the render thread: `[0x2774198]` vtable +0xC.
11. `Sleep(1)` until it acknowledges (`0x277419C`). Without `0x252737E`, also wait for `0x27741AC`.

The game thread is therefore at most one frame ahead of the render thread. The render thread builds frame N while the game simulates N+1.

## Open

- The exact identity of most unnamed sim_frame calls (the lists above give the order).
- Which job kinds 3 and 4 the workers run.
