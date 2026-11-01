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

1. If a change is pending, call the current state's **exit** (`game_state_exit` 0x6973E0, table `0x697458`, index state-1). Exit is skipped when the new state is pushed on top. The exit gets `EAX` = state − 1 and one cdecl argument: the pending state, or on a pop the stack entry below the top. It is also skipped while `0x2527BF6` is set (cleared on every transition).
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

## frame_end_handoff at instruction level (for the port)

`frame_end_handoff` (0x521A00):
- **Signature:** no arguments, returns `EAX` = 0. It reserves 0x14 bytes of locals and saves `EBX` / `EBP` / `ESI` / `EDI`. `EBX` is 0 for the whole function.
- **Locals:** `[esp+0x17]` holds "in gameplay"; `EBP` holds the audio job.
- **Imports used:** `GetCurrentThreadId` 0xDB8080, `Sleep` 0xDB80B8, `LeaveCriticalSection` 0xDB80D0.

### Order

1. **Pacing.**
   1. `al` = byte `[0xED4710]`; `[0x2527383]` = 0.
   2. `render_thread_wait_frame` (0x522B80), cdecl (u8 `[0xED4710]` zero-extended).
   3. `[0xED4710]` = 0.
2. **Optional early wait.** If `[0x252737E]` ≠ 0 and `[0x252737F]` == 0: busy-spin, **without Sleep**, until `[0x27741AC]` ≠ 0 (render thread finished its frame), then clear it.
3. **Flags.** `[0x2527363]` = 1; `[0x252737E]` = `[0x252737F]`.
4. **Device and 2D state.**
   1. `gfx_device_acquire` (0xCFBC00).
   2. `[0xE849DB]` = 1.
   3. `gfx_invalidate_render_state_block`, then `gfx_apply_gamma`.
   4. `gfx_set_tint_color` (0xD14700), with `ESI` = &{1.0, 1.0, 1.0} on the stack.
5. **Streaming.**
   1. state = `0x144E760[depth 0xE96B7C]`.
   2. If `0x144E750[state]` bit 0 is set and depth > 0: `game_state_stack_stream_frame` (0x697740), cdecl (depth − 1, (flags >> 1) & 1).
   3. If state == 5: `gameplay_streaming_frame` (0x68E410).
6. **Present.** `frame_present` (0x5217E0).
7. **Audio job.**
   1. in_game = (top state == 5; depth −1 counts as state 0), kept at `[esp+0x17]`.
   2. If in_game: `EBP` = `audio_update_voices_on_workers()` (the queued job or 0). Otherwise `EBP` = 0.
8. **Snapshot setup.**
   1. `[0x252737D]` = 1 (snapshot being written); `[0x27741AD]` = 0.
   2. `temp_batching_pool_release` (0x521970), then `temp_batching_pool_create` (0x5218D0).
   3. `render_snapshot_copy_view` (`EAX` = 0xE9D620).
   4. `render_snapshot_reset_scales` (`EAX` = 0xE9D6D0).
9. **World visible (`[0xE94D3E]` ≠ 0).** All calls take no arguments unless noted.
   1. If `[0x2526DC4]`: `[0xE849D0]` = `time_of_day_in_range([0x2797784], [0x2797788])` ? −1 : 0. The call is cdecl and the result is converted with `neg al` / `sbb`.
   2. The time-of-day block, in order:
      - `tod_light_value_update` 0x4D9DD0
      - `tod_ambient_colors_update` 0x4D4820
      - `tod_light_block_b_update` 0x4D4F90
      - `tod_sun_update` 0x4D5470
      - `tod_fog_color_update` 0x4D5300
      - `tod_misc_params_update` 0x4D5670
      - `lighting_buffers_copy_for_render` 0x4E69D0

      They blend the keyframes in `0x2654210` (0x250 bytes each; keys `0xF9E2F8` / `0xF9E2F4`, t `0xF9E2F0`) into the constants `0x2658DB0..0x2658E7C`. Script overrides are at `0x2658C64..0x2658C90`.
   3. `rep movsd` 0x260 dwords `0x2652C80` → `0x2653600`. This runs before the next test, so it is unconditional inside this branch.
   4. If `[0x27A3F18]` (tagging active): `tagging_paint_buffer_swap` 0x6255A0.
   5. Snapshots: `audio_frame_snapshot` 0x46CCD0 (under CS `0x256DBA8`), `mesh_decals_frame_update` 0xD08310, `glares_render_snapshot` 0x4C7190.
   6. `[0x25273F3]` = `[0x25273F2]`.
   7. Render parameters: `water_sim_frame_snapshot` 0x728AB0, `render_params_publish` 0x51A880, `render_params_reset` 0x51A4A0, `objects_collect_render_flagged` 0x9FAA20.
   8. Camera:
      1. `0x25F5B14..1C` → `0xE9D620..28`, and 9 dwords `0x25F5B38` → `0xE9D62C`.
      2. `camera_get_fov` 0x494080 (returns in ST0) → `fstp [0xE9D650]`.
      3. `matrix_orthonormalize` 0xBE4170, **stdcall** (`ret 4`) (&`0xE9D62C`). Its argument is pushed before the `fstp`; the caller does not pop it.
   9. Snapshot fields:
      - `0xE9D67C` = `[0x25273A0]`; `0xE9D680` = `[0xE98964]`; `0xE9D684` = `[0xE98968]`;
      - `0xE9D670..78` = `0xE9D7D4..DC` (tint); `0xE9D664..6C` = `0xE9D7C8..D0`;
      - `0xE9D68E` = byte `[0xE996E5]`; `0xE9D688` = frame time `[0x252A3C8]`; `0xE9D68C` = 1 (snapshot valid).
   10. `sky_light_snapshot` 0x4D9A50: `ESI` = 0xE9D690 (sun direction), `ECX` = 0xE9D6B0 (sun colour + intensity), `EDI` = 0xE9D69C, `EDX` = 0xE9D6C0. It uses the sky object `0x2526DAC` or the defaults `0xEC5ACC` / `0x252A6F0`, scaled by the time-of-day intensity.
   11. `world_shader_constants_update` 0xB77CA0 (material shader globals).
   12. LOD / listener centre `0x277C9D0` and distance `0xE9896C`:
       - camera mode `0x25F5AE8` == 1: camera position and `[0xDD0550]`;
       - local player `0x21703D4` exists: its position (+0x14), and `[0xDD0678]` when it is in a vehicle, otherwise `[0xDD0794]`. "In a vehicle" means handle +0xD74 resolves through 0xAE28F0, 0x9B92E0 (EAX = player) is false, and `0xEBCE9C` / `0xEBCEA0` are not a mode-7 / 8 override;
       - otherwise `0x252A6D0..D8` and 1.0.
   13. `vehicles_render_prepare` 0xAE1020, `render_lists_reset` 0x52E7C0, `game_render_func` 0x5250E0, `game_build_render_batches` 0x522FC0.
10. **World not visible.** A fixed view, with no calls:
    - `0xE9D620` = `0x252A6D0..D8`; the matrix is 9 dwords from `0xEC5AE4`; FOV `[0xDD3448]`;
    - the same `0xE9D664..0xE9D688` fields as above;
    - `0xE9D6B0..BC` and `0xE9D6C0..CC` = `0x252A6F0..FC` (written with an `fxch` sequence; the end result is the same order);
    - `0xE9D690..98` and `0xE9D69C..A4` = `0xEC5ACC..D4`;
    - `0xE9D68C` = 1.
11. **Render-list swap.**
    1. Exchange `[0x2305978]` and `[0x230597C]`.
    2. The list that is now `[0x230597C]` (the game thread's next write list) gets its counts +0x5DC0 and +0x1C5C4 zeroed.
    3. `[0x252737D]` = 0.
12. **Cutscene flag.** `[0xE94D37]` = !(( `[0x2527D14]` && `[0x2527CF7]` ) || `[0x2527D15]`).
13. **Audio job.** If in_game and `EBP` ≠ 0:
    1. Set job+0x34 = 0x33D6328, then call `job->vtbl[+0x10]()` (wait).
    2. Clear job +4, +8 (byte), +0x34, +9 (byte), +0x3C, +0x40, +0x44.
    3. Free it with `pool 0xEC5B98 ->vtbl[+4](job − 8)` (thiscall, `ECX` = 0xEC5B98).
14. **End of 2D.** `[0xE849DB]` = 0; `gfx_invalidate_render_state_block`.
15. **Device release** (inlined). If `GetCurrentThreadId()` ≠ `[0x252A370]`: decrement `[0x252A368]`. When it reaches 0, set `[0x252A374]` = 0 and `LeaveCriticalSection(0x33D62D0)`.
16. **Hand-off.** `[0x2774198]->vtbl[+0xC]()` signals the render thread's job object.
17. **Wait for the acknowledgement.** `Sleep(1)` until byte `[0x277419C]` ≠ 0, then clear it.
18. **Without `[0x252737E]`:** `Sleep(1)` until `[0x27741AC]` ≠ 0, then clear it. This is the "game at most one frame ahead" wait.
19. **Render-mode toggle** on `dl` = `[0x2527383]`. Only 0xAE0350 sets it during the frame; step 1 cleared it.
    - If `[0x2527382]` ≠ dl and dl ≠ 0:
      1. If `[0xE98948]` == 2: `[0x2527381]` = 1, `[0xE989D8]` = 2, `[0xE98948]` = 1.
      2. If `[0xE98994]` is 1 or 2: `[0x2527380]` = 1.
    - If `[0x2527382]` ≠ dl and dl == 0: restore `[0xE98948]` from `[0xE989D8]` if `[0x2527381]`, and clear `[0x2527381]` / `[0x2527380]`.
    - In both cases `[0x2527382]` = dl.

### `render_thread_wait_frame` (0x522B80)

cdecl (u8 reset); `ret`. Everything runs under CS `0x27741B4`:

1. If the render thread exists (`[0x252A3EC]` ≠ 0, its thread id):
   - **New render frame number** (`[0x27741B0]` ≠ `[0x2527388]`):
     1. Copy it into `0x2527388`.
     2. Time-stamp with QPC (0 when the timer is off, `0x252A404`).
     3. **Busy-spin** until byte `[0x277419D]` ≠ 0 (set by the render thread at the start of its frame, in 0x522D00).
     4. `[0x2527384]` = elapsed ms of the spin.
     5. `ResetEvent([0x27741A4])`; `[0x277419D]` = 0.
   - **Same frame number and reset ≠ 0:** set `[0x27741B0]` = `[0x2527388]` = `[0x252A3C4]` − 1, then the same `ResetEvent` and clear.
2. If `[0x252737C]` == 0 or `[0x2527363]` ≠ 0, and the caller is not thread `[0x2527E84]`: zero the counts +0x5DC0 / +0x1C5C4 of `[0x230597C]`.

### The render thread's side (`game_render_thread_frame` 0x522640)

**Setup:**
1. Thread id → `0x252A3EC`.
2. `gfx_device_acquire`.
3. The hand-off object `[0x2774198]` = `job_alloc(…)` → vtbl +4 (arm).
4. 0x522D00: `[0x277419D]` = 1, time → `0x27741A0`, `SetEvent([0x27741A4])`.

**Loop:**
1. `pump_window_messages`. When the window is inactive (`0x252A406` == 0): `Sleep(50)` and `SetPriorityClass(GetCurrentProcess(), 0x20)`. Both paths set `[0xED4718]` = 0x20.
2. Release the device (the same inline code as step 15).
3. **Wait:** `job->vtbl[+0x10]()` on `[0x2774198]` returns once the game thread's step 16 signals it. Then the job is released to the pool `0xEC5B98`.
4. Get the next job (pool vtbl +8), `ResetEvent(job+0x2C)`, store it in `[0x2774198]` and arm it (vtbl +4). Record `[0x27741B0]` = `[0x252A3C4]`, the frame number the game thread compares in `render_thread_wait_frame`.
5. Re-acquire the device.
6. Unless quitting (`0x2774195`):
   1. **`[0x277419C]` = 1** (the acknowledgement, step 17); `[0x2527363]` = 0.
   2. Clear the target; set the default render state.
   3. If the snapshot is valid (`0xE9D68C`) and the world is visible (`0xE9D68D`): city / instance batch building (0x52A3B0, `city_build_render_batches`, the city instance jobs).
   4. **`[0x27741AD]` = 1.**
   5. `render_frame` 0x52A710 (or `gfx_command_buffer_wait` when the world is not visible).
   6. `gfx_set_tint_color(ESI = 0xE9D670)`.
   7. `render_deferred_execute`, which runs the list the game thread just swapped into `[0x2305978]`.
   8. Clear the stats at `0x2304D6C..98`; `InterlockedExchangeAdd(0x33D62F4, …)`.
   9. **`[0x27741AC]` = 1** (frame finished, step 18).
   10. 0x522D00 (frame boundary: `[0x277419D]` = 1 and `SetEvent`).
   11. Loop.
7. **Quit** (0x522A90, from the game thread): sets `0x2774195`, signals the job, waits with `Sleep(1)` for `0x2774196`.

**What a port must keep:**
- **Snapshot window.** The game thread may write the snapshot `0xE9D620..0xE9D6D0`, the render-parameter blocks and the deferred list only between the acknowledgement of the previous hand-off and its own step 16. While `[0x252737D]` = 1 the snapshot is being written.
- **Lists the render thread reads after the acknowledgement:** the swapped deferred list `[0x2305978]` and the snapshot.
- **Wait loops.** The two `Sleep(1)` waits (steps 17 / 18) and the busy-spins (step 2 and `render_thread_wait_frame`) are the only waits. There is no event wait on the game-thread side.
- **Device critical section.** The device CS `0x33D62D0` is recursive per thread (count `0x252A368`, owner `0x252A374`). Both threads release it around the hand-off, so it must be released before step 16 or the render thread blocks.

## gameplay_do_frame and gameplay_streaming_frame at instruction level

### `gameplay_do_frame` (0x68E1B0)

The state 5 (gameplay) frame function, from `game_state_dispatch`.
- **Signature:** no arguments, returns nothing.
- **Frame:** `sub esp, 0xC`; saves `EBX` / `EBP` / `ESI` / `EDI`. `[esp+0x18]` is the profile marker slot. `[esp+0x13]` holds the byte "this human is the local player".

1. `profile_start(ECX = 0xDFFB6C, &marker)`; `[0x2527BA2]` = 1; `foreign_character_warning_update()` (no arguments: it shows the warning once when `0x2528B28` is set and `0x2528B29` is clear).
2. **Corrupt stats.** When no cutscene blocks it (`0x2527D14` == 0 → test `0x2527D15`; when `0x2527D14` ≠ 0 the `0x2527CF7` byte is loaded but **not tested**, and the call is skipped): `stats_corrupt_check()` (0x6B5810).
3. **Death callbacks.** For each human on `0x217043C` (`EBP`; links +0x1294; stop at the head or 0) whose timer +0x122C (ms, ≥ 0) has expired:
   1. `bl` = 0; `[esp+0x13]` = (human == local player `0x21703D4`).
   2. If +0xC0 bit 0 is clear:
      1. set it;
      2. `EDI` = `human_callback_thread_begin(ECX = human+0x1234, EDX = &[esp+0x14] (zeroed first))`;
      3. if a thread was made:
         - push argument 1: `script_object_name(EDX = human)` (`lua_pushstring`, or nil with the `[edi+4]`+8 stack-top bump);
         - push argument 2: the killer +0xC0C's name if it is a live human (handle type 1), else nil (`script_thread_push_string`, EDI = thread, EAX = string).

      The thread's argument count +0x1C is bumped by one for each push.
   3. `script_thread_find(cdecl word +0x1290)`. When no thread is running:
      1. +0x1290 = 0xFFFF;
      2. if it is the local player and +0xFA4 bit 0x10 is set: `render_thread_wait_frame(cdecl 0)`, then `player_death_respawn_update(cdecl human)` (0x68D570).
4. If `0x2A1890F`: 0x81EDF0, then `memset(0x2A187D0, 0, 0x144)`.
5. **`sim_frame`** (Rust now).
6. `profile_stop(&marker)`; `profile_start(ECX = 0xDFFB80, &marker)`.
7. If `0x279C0C0`: 0x68C770 (navpoint markers, debug). Then `gameplay_ui_frame` (0x68D2F0).
8. If `0x252976C`: `gfx_set_color(0, 0, 0xFF, 0xFF)` (cdecl, 4 dwords), then `gfx_draw_world_sprite(EAX = 0x2CE30D0 (the focus point), cdecl (f32 [0xDD1C8C], 0xEC2748))`. The f32 is moved through `fld` / `fstp [esp]` after a `push ecx` placeholder.
9. `[0xE9D73D]` = 1; `profile_stop`.

**Quirks:**
- In step 2 the cutscene test is incomplete: whenever `0x2527D14` is set, the corrupt-stats check is skipped, whatever `0x2527CF7` holds.
- The loop re-reads the list head each step (`cmp ebp, [0x217043C]`), so a callback that removes the head ends the walk.

### `gameplay_streaming_frame` (0x68E410)

Called from `game_state_stack_stream_frame` for a state-5 level, from `frame_end_handoff`. It runs on the game thread before `frame_present`.
- **Signature:** no arguments.
- **Frame:** `sub esp, 0x34`; saves `ESI` / `EDI`. It ends with a **tail jump** to 0xA54870.

1. `streaming_chunk_requests_update()` (0xA7B880).
2. Copy the render view of this thread (`gfx_get_thread_state`) to the stack: 9 floats of the matrix from +0xC into `[esp+0x18..0x38]`, then the position from +0 into `[esp+0xC..0x14]`. The last `fstp` happens after a `push`, so it targets `[esp+0x18]` relative to the moved ESP, which is position z.
3. `city_streaming_set_view(cdecl &position, &matrix)` (0x4B39A0).
4. **Chunk states.** For i < `[0xF6FBD4]`, chunk = `0xF70B98[i]`:
   - state +0x18 == 1: `chunk_activate(cdecl chunk)`;
   - state 2: set it to 3, then `chunk_ready_notify(EAX = chunk)` and 0x562100.
5. Streaming and characters:
   - `world_streaming_do_frame` (0x68ED30), `streaming_debug_overlay` (0x68C950), `pcr_morph_handshake` (0x7D6760);
   - `character_textures_stream_update` (0x9E8400), `character_morph_mesh_update` (0x9ECEA0), 0x9EF5C0;
   - clear byte +0x15 of every entry on `0x217068C` (links +0x1C);
   - `vehicle_dirt_update` (0xAC5FB0), `player_outfit_stream_update` (0x6C6C60), `mempool_compact_step` (0x6E79D0).
6. If distant peds are on (`0x2529E20` and all six bytes `0x215DA04`): `distant_peds_stream_update` (0xB7C210).
7. 0x7BFC90; then `jmp script_streamed_assets_release` (0xA54870).

**Thread note.** This runs inside `frame_end_handoff` after `render_thread_wait_frame` and the device acquire, so the render thread is idle or done with its frame. The view it reads is the game thread's own `gfx_get_thread_state`.

## frame_present and the render thread at instruction level

### `frame_present` (0x5217E0), game thread

No arguments; `push ecx` reserves one temporary.

1. If `[0xE94D3F]` (a one-shot clear request):
   1. `gfx_cmd_clear(cdecl 7, 0xFF000000, f32 1.0, 0)`. The 1.0 is written into the slot a `push ecx` reserved, through `fld1` / `fstp [esp]`.
   2. `gfx_invalidate_render_state_block()`; `gfx_set_render_state_block(ECX = 0xEC2748)`.
   3. `[0xE94D3F]` = 0.
2. `gfx_present()` (0xD15110): ends this thread's command list.
3. Screenshot mode:
   1. wanted = `[0x2527392]` ? `[0x2527391]` : `0xE98970[[0xE98974]]`;
   2. store it as 0 / 1 into `[0xE98978]` when it changed.
4. **`screenshot_save()` is called on every path, every frame.** It returns early unless the screenshot keys / flags (`0x2349415`, `0x2348EED`, `0x2527C05`, …) ask for one, so keep the call.

### `game_render_thread_frame` (0x522640), render thread

This is the whole render thread body: it loops until stopped.
- **Frame:** `push ebp / mov ebp, esp / and esp, -8 / sub esp, 0x20`; saves `EBX` / `EBP` / `ESI` / `EDI`. Then `EBP` is reused as the `GetCurrentThreadId` import pointer, and `EBX` = 0. Profile markers live at `[esp+0x14]` / `[esp+0x18]` / `[esp+0x1C]`.
- **Returns** `EAX` = 0 when stopped.

Rows marked **S** touch state shared with the game thread.

**Setup:**
1. **S** `[0x252A3EC]` = render thread id; `render_thread_wait_frame` tests it.
2. `gfx_device_acquire()`.
3. **S** `[0x2774198]` = `job_alloc(cdecl 0x9C, 0xDE1BEC)`, then `job->vtbl[+4]()` (arm).
4. **S** `render_thread_frame_boundary` (0x522D00): `[0x277419D]` = 1, time → `0x27741A0`, `SetEvent([0x27741A4])`.
5. **S** `[0x277419D]` = 1 again, the time again (QPC / ms divisor, 0 when the timer is off); `[0x2774194]` = 1.
6. `render_thread_init_pools()` (0xD14CB0).

**Loop:**
1. `pump_window_messages()`. If the window is inactive (`[0x252A406]` == 0): `Sleep(50)` and `SetPriorityClass(GetCurrentProcess(), 0x20)`. On both paths `[0xED4718]` = 0x20.
2. `profile_start(ECX = 0xDE1C0C, &[esp+0x14])`.
3. **Device release** (inline): unless the caller is thread `[0x252A370]`, decrement `[0x252A368]`; at 0, set `[0x252A374]` = 0 and `LeaveCriticalSection(0x33D62D0)`.
4. **S Wait for the game thread.** With `ESI` = `[0x2774198]` (if non-zero):
   1. `[esi+0x34]` = 0x33D6328; `esi->vtbl[+0x10]()` blocks until `frame_end_handoff` step 16 calls vtbl +0xC.
   2. Clear +4, +8 (byte), +0x34, +9 (byte), +0x3C, +0x40, +0x44.
   3. Free it: `pool 0xEC5B98 ->vtbl[+4](esi − 8)` (thiscall).
5. **S Next job.**
   1. `EDI` = frame number `[0x252A3C4]`; job = `pool->vtbl[+8]()`; `ESI` = job ? job + 8 : 0.
   2. `[0x27741B0]` = `EDI`; `render_thread_wait_frame` compares it.
   3. Set the job fields: +9 = 1, +0x3C = 0xDE1BEC, +0x40 = 0xD0, +0x44 = `EDI`; `ResetEvent([esi+0x2C])`; +0x38 = 0, +0x34 = 0, +0x28 = 0.
   4. `[0x2774198]` = `ESI`; `esi->vtbl[+4]()` (arm).
6. **Device acquire** (inline): unless the caller is `[0x252A370]` or already `[0x252A374]`, `EnterCriticalSection(0x33D62D0)` and set `[0x252A374]` = thread id; then increment `[0x252A368]`.
7. `profile_stop`.
8. **S** If `[0x2774195]` (stop, set by `render_thread_stop`): release the device, set `[0x2774196]` = 1, return 0.
9. `profile_start(ECX = 0xDE1C18, &[esp+0x1C])`.
10. **S Acknowledge:** `[0x277419C]` = 1 (`frame_end_handoff` step 17 waits for it); `[0x2527363]` = 0.
11. `gfx_cmd_clear(cdecl 7, 0xFF000000, f32 1.0, 0)`.
12. **Render target.** Take the render context from TLS (`fs:[0x2C]` → [0]).
    1. target = (ctx +0x5C == 0 and ctx +0x60 == 0xEC26A0) ? 0xEC2610 : ctx +0x60.
    2. target +0x20 = target +0x24 = −1.
    3. **S** If `[0xE849DB]` == 0 (the game thread is outside its 2D bracket) and target is a default one: `render_deferred_queue(cdecl 2)`.
13. `gfx_set_render_state_block(ECX = 0xEC2748)`.
14. `profile_start(ECX = 0xDE1C24)`.
15. **S** If the snapshot is valid and the world is visible (`[0xE9D68C]` and `[0xE9D68D]`, written by the game thread):
    1. `render_viewport_setup` (0x52A3B0), `city_build_render_batches` (0x4B7CC0);
    2. `level_objects_render_collect(EDI = 0xE9D620)`;
    3. 0x533910, 0x533A70, 0x533CD0;
    4. `city_instance_batch_begin(cdecl city_instances_job_a)`, `city_instances_pass_a`, then `city_instance_batch_flush` if `[0xF83AF4]` ≠ 0;
    5. the same with `city_instances_job_b` / `city_instances_pass_b`;
    6. `city_batch_job_release`.
16. `profile_stop`. **S** `[0x27741AD]` = 1: batches built; the game thread clears it in `frame_end_handoff` step 8.
17. If `[0xE9D68D]`: `render_frame` (0x52A710). Otherwise `gfx_command_buffer_wait` (Rust).
18. **S** `gfx_set_tint_color(ESI = 0xE9D670)`, the snapshot tint.
19. `[0x252A2A2]` = 1; **S** `render_deferred_execute` (Rust) runs the list in `[0x2305978]` that the game thread just swapped in; `[0x252A2A2]` = 0.
20. Statistics: zero `0x2304D6C..0x2304D78` and `0x2304D8C..0x2304D98`; `InterlockedExchangeAdd(0x33D62F4, [0x33D62F8] − [0x33D62FC])`; `[0x33D62FC]` = `[0x33D62F8]`.
21. `profile_stop`. **S** `[0x27741AC]` = 1: frame done (`frame_end_handoff` steps 2 / 18).
22. **S** `render_thread_frame_boundary`, then back to step 1.

### Shared state between the two threads

| Global | Written by | Read by | Meaning |
|---|---|---|---|
| `[0x2774198]` job | render (setup, step 5) | game (`frame_end_handoff` step 16 signals it) | the hand-off |
| `0x277419C` | render step 10 (1); game clears | game step 17 | acknowledgement |
| `0x27741AC` | render step 21 (1); game clears | game steps 2 / 18 | frame finished |
| `0x27741AD` | render step 16 (1); game step 8 (0) | either | batches built |
| `0x277419D`, `0x27741A0`, event `0x27741A4` | render (boundary) | `render_thread_wait_frame` | frame boundary and time |
| `0x27741B0` | render step 5 | `render_thread_wait_frame` | frame number of the armed job |
| `0x252A3EC` | render setup | game | render thread id |
| `0x2774195` / `0x2774196` | game (`render_thread_stop`) / render | the other | stop and stopped |
| `0xE9D620..0xE9D6D0` snapshot | game (`frame_end_handoff` steps 8-10) | render steps 15 / 18 | view, lights, tint, valid / visible |
| `0x2305978` / `0x230597C` | game (swap, step 11) | render step 19 / game | deferred state lists |
| `0xE849DB` | game (1 / 0 around its 2D work) | render step 12 | game 2D bracket |
| `0x2527363` | game (1) / render step 10 (0) | `render_thread_wait_frame` | "game wrote the frame" |
| device CS `0x33D62D0`, `0x252A368` / `0x252A374` | both | both | recursive device lock |

A port of either side must keep this order:
- **Game thread:** write the snapshot → release the device → signal → wait for `0x277419C`.
- **Render thread:** wait → re-arm → re-acquire the device → acknowledge → read the snapshot.

The render thread re-arms the next job before the game thread can signal again, because the game thread is still waiting for the acknowledgement.

## Enter and exit of states 2, 5 and 10 (porting maps)

The dispatchers are `game_state_enter` (0x697330) and `game_state_exit` (0x6973E0). Both index a jump table by `EAX` = state − 1.
- **Enter:** saves `EDI`. States 2, 5 and 10 then **tail-jump** to their function after `pop edi`.
- **Exit:** saves `ECX` (a local) and `ESI`, and calls the function. State 5's exit gets one cdecl argument: the next state, read from `[esp+0xC]`, the dispatcher's own argument.

| State | Enter | Exit |
|---|---|---|
| 2 front end | `frontend_enter` 0x73D900 | `frontend_exit` 0x73DB40 |
| 5 gameplay | `gameplay_enter` 0x68DE50 | `gameplay_exit` 0x68E560 (cdecl next state) |
| 10 loading | `loading_enter` 0x73B430 | `loading_exit` 0x73B520 |

### `frontend_enter` (0x73D900)

Corrected from the instructions (the earlier call list was wrong in several places). `EBX` = 0, `ESI` = 1.
1. `[0x2527F14]` = 1; 0x68BE20. `[0xE96C04]` = −1; clear `0x2527BB4`, `0x2527EC3`, `0x2527EB2`, `0x2527EB3`. 0x831300.
2. **Main-menu document handle** `[0x2527EAC]` is live when it is non-zero, its low word is below 0xC, and the slot `0x319F6B0 + (h & 0xFFFF) × 0x57C` has the handle at +0x4C.
   - Not live: 0x73E110.
   - Live: only if `[0x252A263]` is set, clear it and call `options_display_mode_failed_dialog` (0x760B20).
3. `[0xE8DD26]` = 1; 0x7F7830.
4. **Conditional music stop:** if the sound handle `[0xEAC094]` is valid (low byte ≠ 0xFF, high bits ≠ 0 and equal to the slot's id at `0xEEB9AC + idx × 0x19C`, and the slot's +0xEEB8F8 ≠ −1): 0x47C740(`EDX` = 1500).
5. `[0xEAC094]` = 0xFF. Hash: `string_hash_lower` (0xBDCB30), **stdcall (&out)** with `ECX` = 0xE1996C (the name), `EAX` = 0. Then 0x47A2B0(`EDX` = the hash); the word result goes to `[0xE8EAE0]`.
6. `[0x1F6F53C]` = 0; `[0xEAC0A4]` = `[0xE843AC]`, wrapped by −0x6B49D200 when above 0x6B49D200.
7. 0x7EAE00(`EAX` = 0x1FD5028). Clear `0xEAAF6C`; clear `[0xEAAF84]` if set. 0x6AA570, 0x76DE70. Clear `0x2527BD1`, `0xE94D3E`, `0x2528A7F`.
8. If `[0x2527EA0]` == 0: 0x73C2F0(cdecl 0), then `[0x2527EA0]` = 1.
9. 0x835A20. If `[0x2527C03]`: clear it, `[0x2528A7C]` = 1, and 0x7F7830 when `[0x2528B48]` is set.
10. Only if `[0x2526D9C]` == 1: 0x4D1270(cdecl 1000, f32 0.0, `DL` = 0).
11. `[0x2527EB4]` = 0.0; clear `0x2527C04`, `0x2528A7D`, `0x2527EBC`, `0x2527EBD`; `[0xE8EAE4]` = −1.
12. **Main-menu Lua call record.** If `[0x2527EC4]` is a live handle (low word below 0x20, slot `0x31A4A80 + i × 0x260` with the handle at +0x210): `vint_lua_call_free` (0xB91790) with `ESI` = `EAX` = the record, `ECX` = the handle. Then `vint_lua_call_alloc` (0xB91B70, `ECX` = 0xE1993C); if it returns a record: `[0x2527EC4]` = record +0x210, record +0x25C = 0x73D350 (the callback).
13. Only if `[0x2528CF2]`: 0x830F70(`ESI` = 0xEB22E8), then clear it.

### `frontend_exit` (0x73DB40)

1. If `0x2527ECC`: free it (0xC15720) and clear `0x2527ECC` / `0x2527ED0`.
2. `_stricmp(0x1F73870, "sr2_city")`: on a match, `[0xE8DD26]` = 0.
3. `streamman_process`.
4. If `0x2527EA0`:
   1. 0x6888D0, 0x47F170, `[0xE94D3E]` = 1;
   2. if `0x2527B76`: `render_thread_wait_frame(0)`, `streamman_unload_flagged_geom`, clear the flag;
   3. clear `0x2527EA0`.
5. `[0xE94D3E]` = 1; 0x7799D0; 0x73D210; count `0x2528B24` down to 0; `0x2527EE5` = 0.
6. If the pad is −1 and `0x212E2B0` ≠ 0: both become 0.
7. 0x754020.
8. If `0x2527EBC` == 1: 0x813040. Clear it.
9. **Unload the main-menu document** (handle `0x2527EAC`): `vint_document_run_unload_script`, the callback `[0x252A1EC]`, `vint_document_release_pending`, `vint_document_unlink`. This is the same sequence as the unload pass in `vint_update_documents`, done at once here.
10. Clear `0x2527EAC` / `0x2527EBC` / `0x2527EBD`; `0x2527EA3` = 1.
11. Run the Lua function `Main_menu_controller_selected` (0xCDD610, with `0x252A1B8`).

### `gameplay_enter` (0x68DE50)

Corrected from the instructions (the earlier list had unconditional steps that are really gated). `EBX` = 0. "Online host" = `[0x2528B48]` == 1 and `[0x2528B49]` == 0; "co-op" = `[0x2528B49]` set. Session = `[0x2528C14]`; it "takes part" when it is non-zero, `[eax+0x3F8]` is set, and (`[0x2528B56]` == 0 or `[0x2528B2B]` set).
1. Clear `0x2527BA0`, `0x2527BA2`. 0x792B10.
2. **Unless online host:** 0x739930(cdecl 0x21, `ECX` = 0xDFFB48).
3. 0x4D1270(cdecl 250, f32 0.0, `DL` = 0).
4. If `[0xE96C04]` ≠ −1: `net_send_gameplay_changed` (0x68F0D0).
5. Clear `0xE8E72C`, `0x2527FF8`, `0x2527FFC`. **Online host jumps to step 9.**
6. If `[0xE8D6BF]`: `[0x2528989]` = 1. If the session takes part: 0x7F8BA0(`ECX` = 11, `ESI` = session +0x408).
7. `CL` = 0. If the session takes part and (`[0x2527B9E]` or `[0x2527B9F]`):
   - the HUD message: 0x753F40(`EAX` = `lstr_get(ESI = 0xDFFAE0)`, `CL` = 1, cdecl (`lstr_get(0xDFFB10)`, `lstr_get(0xDFFAF4)`, 0x68DC00, 1)), `EDI` = 0; `[0x2527BA4]` = result +0x920;
   - B9E path: a fade 0x4D1270(cdecl 0, f32 0.0, `DL` = 1) only when `[0x2526DA0]` ≠ 0.0 (or NaN); B9F path: the same fade only when `screen_fade_is_clear` is false;
   - `CL` = 1, `[0x2527BA1]` = `[0x2527BA0]` = 1.
   - Then (taken or not, once the session takes part) clear `0x2527B9E` / `0x2527B9F`.
8. If `[0x2528989]`:
   - with `CL` = 1: go to 9;
   - session non-zero and 0x80FCF0(cdecl session) true: if 0x6C8020 false and `_stricmp(0xE99740, 0xDFFB60)` == 0 → 0x68DC80; otherwise → 0x68DC50;
   - session zero or 0x80FCF0 false: 0x68DC50 only when `[0x2528D21]` == 0.
   
   If `[0x2528989]` == 0: unless `[0x2527BD0]`: 0x7C0C30(cdecl player `[0x21703D4]`, 0), 0x7CE320(cdecl −1, 0). Then 0x8CB040.
9. **Only online host:** `weather_set` (0x4DEC00, `EAX` = `[0x2A4D13C]`).
10. **Only if `[0x27AC7B8]` bit 2 (value 2):** `overlay_show(cdecl 1, EDI = 0xEAC488)`.
11. **Only co-op:** `[0x2528B89]` = 1; `[0xEB1614]` = `[0xE843AC]` + 10000 (minus 0x6B49D200 when above it); 0x80B8D0, 0x80B960.
12. 0xB874E0.
13. **Clock clamp.** v = f32(f32(`[0x2527DA4]` / `[0xE8ECAC]`) × 2). If v ≥ the double `[0xE5DC58]` (or NaN): `[0xE84390]` = `[0xE5D9C8]`, `[0xE84394]` = v. Otherwise `[0xE84390]` = v, `[0xE84394]` = `[0xE5D9C8]`. `[0xE98974]` = 0.
14. If `[0x1F768DA]`: 0x699840(cdecl 1, `EDI` = 13), then `++[0x2527CC0]`.
15. Only if `[0x2528ACE]`: 0x8D0CB0.
16. `[0x2527C00]` = 0.

### `gameplay_exit` (0x68E560), cdecl (next state)

This tears the world down. The order matters.
1. **Flags.** `0x2527C0F` = 1 (`time_scale_restore_check` uses it); `0x2527BA2` / `0x2527B9E..A1` = 0.
2. **Audio and render sync.** 0x47F200, 0x47F080, `render_thread_wait_frame(0)`.
3. **Pause and HUD.**
   1. Unless online-host (0x2528B48 set and 0x2528B49 clear): 0x739B70.
   2. 0x754270 (`dialog_pause_hide`), 0x754320, 0x6B7870.
   3. Reset the screen state `0xE8CF7C` unless it is 11 or 13 (or `0x2528ADF`).
   4. 0x755F70; `0x2527FF4` = 0; `0xE8E780` = −1.
   5. `interface_screen_state_frame`, `hud_player_reset_complete`.
4. **Cutscene.** If a cutscene is active: release the device (inline), 0x6C44C0, `gfx_device_acquire`, `player_outfit_stream_update`.
5. **Overlays and device.** `overlays_hide_all`; `gfx_device_acquire`; `[0xE98974]` = 2; device release (inline); `gamewide_func(1)`.
6. **Player.**
   1. If the player is dead-flagged (+0x1161 bit 0): 0x9DA290(0).
   2. If its vehicle is present and 0xAB4960 says so: 0xAB4720(0, 0).
   3. 0x9DED00(player, 0); 0x68F420; 0x6A4230.
7. **Network.** With the session flags set and next ≠ 10: 0x7F7830. In single player: 0xB710C0(1) and `0x2526E14` = 0 (normal weather).
8. **World teardown:**
   1. 0x9F2100, 0x550A00 twice; clear `0x279930C..0x279932C`.
   2. 0xB80020, 0xB7CA10 (distant vehicles and peds), 0x4A8230.
   3. `objects_reset` inside a device acquire / release pair.
   4. 0xA24750, 0xA7D480, 0x514260, 0x5039F0; `0x2527294` = 0; 0xA93FC0, 0x9347B0.
   5. `objects_deactivate_all`, 0x9FD3F0, 0x9EA0E0, 0x9ECD50, 0xAC5D40.
   6. The Havok / physics resets 0x6E7690, 0x6E33B0, 0x6F8E50, 0x6E8560, 0x6E5BB0, 0x6E0A20.
   7. Clear `0xF8478C..94`; 0x4DA850, 0x7938A0.
9. **Saves and HUD.**
   1. Unless one of `0x2527C01..04` is set: 0x6B56A0.
   2. `0xE8DD23` = 1; 0x766320(1, 1) and (0, 1); 0x5480C0.
   3. Clear `0x2526D67`, `0xFA2228` / `0xFA222C`, `0x276CFD0` / `D4`.
   4. 0x747C40, 0x7483E0, 0x7434D0 ×2; clear `0x2527F27` / `0x2527F48`; 0x747930, 0x747680.
10. **Pauses.** While `0x2527C08` > 0: 0x699910 (pops pause levels).
11. **Clocks.**
    1. 0xA79030; `0x2526D2A` / `0x2526D24` = 0; 0xC14930(`EAX` = pad or 0, `ECX` = raw pad, 0.0, 0.0).
    2. Pending online stats: 0x835A20, 0x824F30.
    3. 0x793D60(0).
    4. `0xE84390` / `0xE84394` = the clock defaults `[0xE5C018]` / `[0xE5D9C8]`.
12. **Co-op to loading.** In co-op with next == 10: `0x2527C00` = 1 around 0x7F8BA0.
13. **End.** `0x2527C0F` = 0. If the pending state is 2: 0x73E110. Then 0x69AED0, 0x465470, 0x47F170.

### gameplay_exit (0x68E560) at instruction level

cdecl (next state, read at `[esp+0x14]`). Saves `EBX` (= 0 throughout), `EBP`, `ESI` (= −1 at first), `EDI`. `EBP` = the `GetCurrentThreadId` import `[0xDB8080]`.

**The device lock** (critical section `0x33D62D0`; count `0x252A368`; owner `0x252A374`; the device thread id `0x252A370`):
- **acquire** = `gfx_device_acquire` (0xCFBC00):
  - no-op on the device thread `0x252A370`;
  - count + 1 if the caller already owns it;
  - otherwise `EnterCriticalSection`, owner = caller, count + 1.
- **release** is **inlined** here:
  - `GetCurrentThreadId`; if it equals `[0x252A370]`, skip;
  - otherwise count − 1; at 0: owner = 0 and `LeaveCriticalSection(0x33D62D0)`.
  - It **does not check the owner**, and a release at count 0 drives the count to −1 without leaving.

The game thread **enters `gameplay_exit` holding the device**: `frame_end_handoff` took it. So the first operation can be a release.

`[A]` and `[R]` below mark acquire and release.

1. `0x2527C0F` = 1; `0x2527BA2` and `0x2527B9E..BA1` = 0.
2. 0x47F200, 0x47F080, `render_thread_wait_frame(cdecl 0)`.
3. Unless (`[0x2528B48]` == 1 and `[0x2528B49]` == 0): 0x739B70(`ECX` = 0xDFFB48).
4. `dialog_pause_hide` (0x754270); 0x754320 (`EDI` = 3); 0x6B7870.
5. Unless `[0x2528ADF]` == 1 or `[0xE8CF7C]` is 0xD or 0xB: `[0xE8CF7C]` = −1.
6. 0x755F70; `[0x2527FF4]` = 0; `[0xE8E780]` = −1; `interface_screen_state_frame`; `hud_player_reset_complete`.
7. **Cutscene** (`[0x2527D14]` set and cutscene +0xAFF == 0): **[R]** release, 0x6C44C0 (cutscene teardown, which needs the device free for the render thread), **[A]** `gfx_device_acquire`, `player_outfit_stream_update`.
8. `overlays_hide_all`; **[A]**; `[0xE98974]` = 2; **[R]**. This is an acquire / release sync point around the render-mode write.
9. `gamewide_func(cdecl 1)`.
10. **Player** P = `[0x21703D4]`:
    1. If P+0x1161 bit 0: 0x9DA290(cdecl 0).
    2. If P's vehicle (+0xD74 handle, type 5) and 0xAB4960(`ECX` = vehicle): 0xAB4720(`EAX` = vehicle, cdecl 0, 0).
    3. 0x9DED00(cdecl P, 0); 0x68F420; 0x6A4230.
11. **Network / weather.** With S48 = `[0x2528B48]` and S49 = `[0x2528B49]`:
    - S48 == 1 and S49 == 1 (co-op): 0x7F7830 unless next == 10;
    - S48 == 1 and S49 == 0: unless next is 8, 9 or 10: 0x7F7830, `0xB710C0(EAX = 0xDFFB94, cdecl 1)`, `[0x2526E14]` = 0;
    - otherwise nothing.
12. **World teardown**, in this order:
    1. 0x9F2100; 0x550A00(`ESI` = 0x279930C); 0x550A00(`ESI` = 0x2799390). Clear `0x279930C..0x279931C` and the byte `0x279932C` (floats with `fldz`).
    2. 0xB80020 (distant vehicles), 0xB7CA10 (distant peds), 0x4A8230.
    3. **`objects_reset`** (0x925AF0), **then [A] [R]**: a sync point after it, not around it.
    4. 0xA24750, 0xA7D480, 0x514260, 0x5039F0; `[0x2527294]` = 0; 0xA93FC0, 0x9347B0.
    5. `objects_deactivate_all`, 0x9FD3F0, 0x9EA0E0, 0x9ECD50, 0xAC5D40.
    6. Physics: 0x6E7690, 0x6E33B0, 0x6F8E50, 0x6E8560, 0x6E5BB0, 0x6E0A20.
    7. `0xF8478C..0xF84794` = 0; 0x4DA850; 0x7938A0.
13. Unless any of `0x2527C01..C04` is set: 0x6B56A0.
14. `[0xE8DD23]` = 1; 0x766320(cdecl 1, 1); 0x766320(cdecl 0, 1); 0x5480C0.
15. Clear `0x2526D67`, `0xFA2228` / `0xFA222C`, `0x276CFD0` / `0x276CFD4`. 0x747C40; 0x7483E0; 0x7434D0(`EDI` = 0x29A6520); 0x7434D0(`EDI` = 0x29A66D8); clear `0x2527F27` / `0x2527F48`; 0x747930; 0x747680.
16. While `[0x2527C08]` > 0: 0x699910(`ESI` = −1), which pops a pause level.
17. 0xA79030. `[0x2526D2A]` = 0, `[0x2526D24]` = 0. 0xC14930(`EAX` = `[0xE98DB0]`, or 0 when it is −1; `ECX` = the raw `[0xE98DB0]`; cdecl 0.0, 0.0). (Fixed: the two registers were swapped here before.)
18. If `[0x1F768DA]`: 0x835A20; if also `[0x144EA3B]`: 0x824F30 and clear it. Clear `0x1F768DA`. 0x793D60(cdecl 0).
19. `[0xE84390]` = `[0xE5C018]`, `[0xE84394]` = `[0xE5D9C8]` (the clock defaults).
20. Co-op with next == 10: `[0x2527C00]` = 1, then 0x7F8BA0(`ECX` = 7, `ESI` = session +0x408). Otherwise `[0x2527C00]` = 0.
21. `[0x2527C0F]` = 0. If `[0x144E75C]` == 2: 0x73E110. Then 0x69AED0, 0x465470, **tail `jmp 0x47F170`**.

**Lock balance.** Releases and acquires are matched within the function: 7 is −1 then +1, 8 is +1 then −1, 12.3 is +1 then −1. The thread leaves holding the device exactly as it entered.

**Port:**
- Port the inline release as a helper with the same "skip on the device thread, no owner check" rules.
- Do not move `objects_reset` inside the pair.
- `gameplay_enter` (0x68DE50) has no device acquire or release at all.

### `loading_enter` (0x73B430)

1. `[0xE94D3E]` = 0 (world not visible); 0x7F8BA0(`ECX` = 7, `ESI` = `[[0x2528C14]+0x408]`).
2. `0xE8EB88` = 0 (load result: 0 running, 1 done, −1 failed); `0x2527E81` / `0x2527E80` = 0.
3. Both job workers to kind 4 and `job_worker_wake` each (`ESI` = worker): workers `0x33D63F8` and `0x33D64C8`, kind at worker +0x38 (`0x33D6430` / `0x33D6500`).
4. If `0x2527E84` == 0: 0x739340. Then 0x6981E0.
5. `gfx_set_color(0, 0, 0, 0xFF)`; `screen_fade_start(0, 0, 1.0)`: black at once. Then 0x4D1270(cdecl 250, f32 0.0, `DL` = 0).
6. If the top state (`0x144E760[[0xE96B7C]]`) is 10 and `0xE96C04` < 11: 0x73B650.
7. **`0xE8EB8C` = `thread_create(loading_thread_proc 0x73B320, 0, 0)`**: the load runs on its own thread.
8. `0x2527E8C` = 0; `0x2527E83` = 0.

### `loading_exit` (0x73B520)

1. `CloseHandle` on the loading thread's handle (`0x2347144 + (0xE8EB8C & 0xFFFF) × 20`).
2. If the co-op wait did not finish (`0x2527E8C` == 0) and `0x2527BA0` is set: set `0x2528989` when `0xE8D6BF`, then 0x7F8BA0(`ECX` = 11, `ESI` = session +0x408).
3. On failure or a non-gameplay next state: 0x698700. Here a failure is `0x2527E80` == 1 or `0xE8EB88` == −1; a non-gameplay next state is a pending state (`0x144E75C`) other than 5 whose top is not 8 / 9.
4. If the loading music `0xEAC094` is still valid and not handed over (0x698FF0 false): stop it (0x47C740, `EDX` = 500).
5. `[0xE94D3E]` = 1; **`[0x2527E80]` = 0**.
6. If the top state is 10: **tail `jmp 0x73B790`**; otherwise return.

**Port note:**
- `loading_enter` / `loading_exit` and `frontend_exit` are short, call-order-only functions, so they can be ported as they are.
- `gameplay_exit` is long but has no branches that depend on its own results. A port can keep it as one block of calls in this order. Keep the device acquire / release points exactly; see "gameplay_exit at instruction level". (Correction: `objects_reset` runs **before** the third acquire, not inside a pair.)

## game_state_dispatch, overlays and the gameplay UI at instruction level

### `game_state_dispatch` (0x697490)

**`ESI` = the state (register argument)**; `push ecx` reserves a temporary.

1. Unless state == 4: `overlays_update()`.
2. Jump table at 0x6974EC on `ESI` − 1, for states 1-11 (others do nothing):

   | State | Frame function |
   |---|---|
   | 2 | `frontend_state_frame` |
   | 5 | `gameplay_do_frame` |
   | 7 | `mp_lobby_state_frame` 0x830AB0 |
   | 8 | `mp_game_setup_state_frame` 0x834D40 |
   | 9 | `mp_results_state_frame` 0x82F570 |
   | 10 | `loading_state_frame` |
   | 11 | `video_frame` |
   | 1, 3, 4, 6 | nothing |

3. Unless state is 2 or 6: `overlays_render()`. The front end renders its overlays itself, in `frontend_render`.

### `overlays_update` (0x75B220)

The overlay stack is the array `[0xEAC4E0]`, with count `[0xEAC4E8]`. Byte +4 of an overlay means "the overlay below keeps running".

For i = count − 1 down to 0:
1. `ovl->vtbl[+8]()` (update, thiscall). If it returns 0: `overlay_hide(ESI = ovl)` (0x75B1B0).
2. **Re-read slot i**: a hide can change the array. If that overlay's +4 is 0, stop.

### `overlays_render` (0x75B270)

Frame `sub esp, 0x20`; returns at once with an empty stack.

1. **Start index.** From i = count − 1 down while i > 0 and `[ovl+4]` ≠ 0: i − 1. The result is the lowest overlay that is visible.
2. For j = i .. count − 1: if `ovl->vtbl[+0x10]()` (is "on top") returns 0, call `ovl->vtbl[+0xC]()` (render). The count is re-read each step.
3. Save the render target's tint (+0x70 / +0x74 / +0x78 of the current TLS target, with the 0xEC26A0 → 0xEC2610 swap); `gfx_set_tint_color(ESI = &{1, 1, 1})`.
4. For **j = 0** .. count − 1: if vtbl +0x10 returns exactly 1, call vtbl +0xC.
   - **Quirk:** this loop starts at 0, not i. On-top overlays below the start index are drawn too, with the tint at 1.
5. Restore the saved tint (`gfx_set_tint_color(ESI = &saved)`).

### `gameplay_ui_frame` (0x68D2F0)

Frame `sub esp, 0x3C`; saves `ESI` / `EDI`. It ends with a **tail jump** to `hud_render`.

1. **Debug marker**, only when `0x2527B9D` is set and a local player exists:
   1. The point = 0x9DF410(`ESI` = &tmp) + 0.05 × −(player +0x2C..+0x34).
   2. Two points at ± 0.5 × the camera row `0x25F5B38` around it, each through `gfx_world_to_screen` (0xD22B10, `EAX` = &in, `ESI` = &out).
   3. The rectangle is built from their spread (× 0.5, × 1.2; constants are doubles at 0xE5C368 / 0xE5BF70 / 0xE5C5A0).
   4. `gfx_set_color(cdecl 0, 0, 0, 0xFF)`; `im2d_fill_rect(cdecl _ftol2(x0), _ftol2(y0), _ftol2(x1), _ftol2(y1), 0xEC2740)`. The four `_ftol2_sse` conversions are pushed in reverse order.

   Debug only; a port can drop it or keep it behind the flag.
2. `sun_flare_render` (0x4D6BF0), `screen_flash_render` (0x4D0E80), `screen_fade_apply` (0x4D1790), `world_text_render` (0x92D7B0), `hud_pre_render` (0x793BB0).
3. `jmp hud_render`.

### `hud_render` (0x75A230)

Saves `ESI`. It ends with a **tail jump** to `interface_render`.

1. For i < `[0x2527FF4]`: idx = `0x1F768E4[i]`; cb = `[0xE8E730 + idx × 0x14 + 0xC]`; call it with no arguments if set. These are the active HUD overlay render callbacks.
2. `letterbox_render` (0x755B80), `player_hud_extras_render` (0x6C6BE0), `dialog_boxes_render(cdecl 0xEAC208)` (0x7552D0).
3. `jmp interface_render` (0x7F33B0).

**Port:** all four functions are short call sequences over global arrays. Two things must stay:
- The re-reads of the overlay count and slots, because an overlay can hide itself during update or render.
- `overlays_render`'s second loop starting at 0.

## frontend_render and interface_render at instruction level

### `frontend_render` (0x73CCF0)

Frame `sub esp, 8`; saves `EBX` (`EBX` = 0 from step 3 on). `frontend_state_frame` brackets the whole call with `[0x252A2A2]` = 1 / 0.

1. **Video and device.** If `[0x2527EA0]` == 1:
   1. `video_frame()`;
   2. `[0xE849DB]` = 1 (the game thread's 2D bracket; the render thread then skips `render_deferred_queue(2)`);
   3. `gfx_invalidate_render_state_block()`, `gfx_device_acquire()`.
2. `screen_fade_apply()`.
3. **Fade fill.**
   1. alpha = `_ftol2(f32(fade [0x2526DA0] × 255.0))`. The double 255.0 is at 0xE5C038, and the product goes through `[esp+8]`, so it is rounded to f32 first.
   2. `gfx_set_color(cdecl 0, 0, 0, alpha)`.
   3. `im2d_fill_rect(cdecl 0, 0, width [0x22FDC1C], height [0x22FDC20], 0xEC2740)`. One `add esp, 0x24` cleans both calls.
4. **Cached sampler states** for sampler 0, written inline into the command ring at `[0x33D62F8]` (16-byte records, opcode 0x13 = `SetSamplerState`, doc 07). Each is written only when its cache differs, then the cache is updated:

   | Cache | Value | Record | Meaning |
   |---|---|---|---|
   | `0x22F530C` | 1 | `{0x13, 0, 7, 1}` | mip filter = point |
   | `0x22F5304` | 2 | `{0x13, 0, 5, 2}` | mag filter = linear |
   | `0x22F5308` | 2 | `{0x13, 0, 6, 2}` | min filter = linear |

   The ring pointer advances 0x10 after each record. There is **no bounds check**, as for every ring writer.
5. `interface_render()`.
6. The screen-state render callback: cb = `[0xE8E8A0 + [0x2527F10] × 0x10 + 0xC]`; if set, `cb(cdecl f32 1.0)` (1.0 is pushed via `push ecx` / `fld1` / `fstp [esp]`).
7. `overlays_render()`.
8. If `[0x2527EA0]` == 1:
   1. `[0xE849DB]` = 0; `gfx_invalidate_render_state_block()`;
   2. release the device (inline): unless the caller is thread `[0x252A370]`, decrement `[0x252A368]`; at 0, set `[0x252A374]` = 0 and `LeaveCriticalSection(0x33D62D0)`.

### `interface_render` (0x7F33B0)

`push ecx` (temporary); no arguments.

1. If the screen state `[0xE8CF80]` ≤ 0x14 and `[0xE8CF94 + state × 0x10]` is set: call it (no arguments).
2. Copy 8 bytes `0xEC2740` → `0x2A14B28` (the 2D render-state block header).
3. `[0xE8C548]` = 1; `[0x2528B14]` = 0; the colour bytes `0xEB15BC..BF` = 0xFF.
4. `gfx_set_render_state_block(ECX = 0xEC2740)`.
5. `gfx_set_color(cdecl [0xEB15BC], [0xEB15BD], [0xEB15BE], [0xEB15BF])` (zero-extended bytes, so white).
6. `vint_render_documents()` (0xB8BD40).

**Port:**
- `interface_render` is a straight sequence.
- In `frontend_render`, keep these:
  - the `0xE849DB` bracket and the device acquire / release, only when `0x2527EA0` == 1;
  - the f32 rounding of the alpha;
  - the three cached sampler writes (or call the ring writer that produces them).

## Open

- The exact identity of most unnamed sim_frame calls (the lists above give the order).
- Which job kinds 3 and 4 the workers run.
