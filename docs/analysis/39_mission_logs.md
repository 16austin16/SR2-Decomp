# 39 — Mission logs: what runs where

The user plays missions with the hook's mission logger on (`crates/sr2-hook/src/missionlog.rs`). Each log is split into sections by F8 marks. This doc keeps what each run showed and the functions named from it.

Samples were grouped by real function start: `tools/progress.py` function starts plus `docs/functions.csv`. That is finer than `tools/mission_report.py`, which credits each address to the nearest *named* function.

## Mission 1 (tss01), `bin/mission01_tss01.log`

136k samples (game thread 39k, render thread 98k). Sections, from the user's marks:

| Section | Frames | What |
|---|---|---|
| s1 | 2210-2431 | opening cutscene |
| s2 | 2431-13486 | character creator (about 3 min) |
| s3 | 13486-13561 | mission start |
| s4-s6 | 13561-39511 | tss01: cutscenes with gameplay between them |
| s7-s8 | 39511-40675 | after the mission |

Memory stayed flat (about 1.8 of 4 GB) over 40k frames.

### Render thread

- **Character morphs, about 42% of all render samples, mostly in the character creator.**
  - `rl_animated_mesh_inst::pre_render` calls `morph_target_apply` (0xCF6840) once per active morph target. For every vertex the target touches, it adds the weighted position delta (three int16) and the normal / tangent deltas (packed bytes).
  - The byte deltas go through `normal_ubyte4_add` (0xBDB460) = `normal_unpack_ubyte4` (0xBDB4F0) twice, then `normal_pack_ubyte4` (0x9EE620).
  - The pack does four float → int conversions per vertex. That is the single hottest function in the run (35k samples).

  This is the cluster `mission_report.py` showed as "unnamed near xtbl_load_player_creation+0x3202..0x32c6". It runs on the CPU every frame while a morphed character is drawn, so it is a good candidate for an optimised Rust port (integer math, or caching the morphed buffer until the sliders change).
- **`__VEC_memcpy_fastcopy` (0xCA326E), 21% of render samples.** This is the CRT's SSE aligned-copy loop, not `___report_gsfailure`. Sampling the EIP can't tell which caller is copying, but the share is highest in the same sections as the morphs (the morphed vertex buffer upload is the likely caller).
- `rl_morph_mesh_inst::select_lod` / `is_ready`, `frustum_test_aabb` and the city cull helpers under `city_instances_cull_view_2` (0x4B5C40, 0x4B5CE0, 0x4B5FF0) make up most of the rest.

### Game thread

- `frame_end_handoff` (about 10%) is the wait for the render thread.
- `profile_start` / `profile_reset` / **`profile_stop`** (0xC0C990) together take about 9%. `profile_stop` takes a critical section and calls `QueryPerformanceFrequency` and `QueryPerformanceCounter` for each marker. That is a cheap win to compile out in a port.
- **Effect lists** (0x50F780, 0x50F190):
  - `effect_list_expire_timed` drops the active flag on entries whose start + duration has passed;
  - `effect_list_reclaim_dead` moves inactive entries onto the free lists `0xFA20A0` / `0xFA209C` and lowers the counters `0x2527298` / `0x252729C`.
- **Visibility:** `view_bounds_visible` (0x530450) and `view_volume_test` (0xBEA660), called from the city cull.
- **Only during tss01 (s4-s6, not in s2):**
  - two SIMD math functions 0x723F90 / 0x723EB0 (called from 0x724880);
  - Havok narrow-phase code 0xC3C8F0 / 0xC74280;
  - render-list walks 0x536BD0 and 0x4B6B60 (under `city_build_render_batches`).

  They are left unnamed until a second mission shows what they belong to.

### Named from this run

`normal_pack_ubyte4`, `normal_unpack_ubyte4`, `normal_ubyte4_add`, `morph_target_apply`, `__VEC_memcpy_fastcopy`, `profile_stop`, `effect_list_expire_timed`, `effect_list_reclaim_dead`, `view_bounds_visible`, `view_volume_test`.
