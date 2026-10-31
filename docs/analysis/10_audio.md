# 10: Audio

Names are in `docs/functions.csv` (subsystem `audio`). The game sources were under `.\audio\` (`gameaudio.cpp`, `other_audio_systems\vehicle_engine_audio.cpp`, `radio_tuner_system.cpp`, …).

## APIs

| API | How | Used for |
|---|---|---|
| **XACT3** (COM) | `CoCreateInstance(CLSID {d3332f02-3dd0-4de9-9aec-20d85c4111b6}, IID {e72c1b9a-d717-41c0-81a6-50eb56e80649})` → `0x2526B48`. If `HKLM\Software\Microsoft\XACT\DebugEngine` is set, a debug-engine CLSID is tried first, falling back to the normal one (0xDD89AC). | All game sound: cues from sound banks, wave banks, categories |
| **XAudio2** (COM) | `CoCreateInstance(CLSID {b802058a-464a-42db-bc10-b650d6f2586a}, IID_IXAudio2 {8bcf1f58-…})`, then `Initialize(0, XAUDIO2_ANY_PROCESSOR)` → `0xF52E34`. `CreateMasteringVoice(&0xF52E30, 6 channels, …)`, and submix voices (slot 9) in 0x482AE0. | The output graph; the mastering voice volume is muted and restored by the audio thread |
| **X3DAudio** | `X3DAudioInitialize` (0x47B8A0) / `X3DAudioCalculate` (0x4821C0) | 3D positioning |
| **DirectSound 8** | `DirectSoundCreate8` in `game_init_allocators_and_packfiles` → `0x2527B6C`, `SetCooperativeLevel(hwnd, DSSCL_NORMAL)` | **Only for Bink video**: `BinkSetSoundSystem(BinkOpenDirectSound, ds)`. Without DirectSound it falls back to `BinkOpenWaveOut`. |

The XACT engine methods the game uses, by vtable slot:

| Slot | Method | Callers |
|---|---|---|
| 6 | Initialize | `xact_init` |
| 7 | ShutDown | 0x481A00 |
| 8 | DoWork | the audio thread (`0x2526B48`, vtable +0x20), 0x481BF0, `loading_screen_pump` 0x481080, 0x481D20 |
| 9 | CreateSoundBank | 0x481220 |
| 10 | CreateInMemoryWaveBank | 0x4810E0, 0x481BF0 |
| 11 | CreateStreamingWaveBank | 0x481170 |
| 17 | GetCategory | 0x481E90 |
| 20 | Pause | 0x481E90 |

## Initialisation

**Memory pools.** `game_init_allocators_and_packfiles` (0x51D800) creates two audio pools with `VirtualAlloc`: "audio - wavebank" (`0x2773360`) and "audio - soundbank" (`0x27733B8`). Each pool has a vtable: +0x1C alloc, +0x30 mark, +0x34 release, +0x58 reset.

**`audio_init`** (0x465020, from the game thread's init 0x51F700):
1. `xtbl_load_audio_constants` (`audio_constants.xtbl`), then 0x46EA00 and 0x46F460.
2. Reset both pools.
3. 0x483FA0, then `xact_init` (0x4818C0).
4. Mark the pools, then X3DAudio init (0x47B8A0), 0x47DF80, and 0x46FA20 (loads `audio_boot.idx_map` / `audio_level.idx_map`).
5. Allocate two lookup tables (0x4000 and 0x300000 bytes, filled with 0xFF) from the allocator at `0x277307C`.
6. Run a large set of subsystem inits: vehicle engine/tire audio, the radio tuner (0x4866D0, 0x487E20, 0x48A880, 0x4883E0), 0x491330..0x491E50 and 0x46A760.
7. Set `0x2526A4A`.

**`xact_init`** (0x4818C0):
1. 0x480C90, then `xaudio2_create` (0x482FF0).
2. `CreateMasteringVoice` (6 channels).
3. 0x482AE0 (submix voices).
4. `xact_engine_create` (0x4828E0).
5. Read `saintsrow.xgs` (the XACT global settings) through `file_open` / `file_read` into the soundbank pool.
6. `IXACT3Engine::Initialize` with a lookahead of 1500 ms (0x5DC) and the settings buffer.
7. On success, `xtbl_load_audio_banks` (0x481450).

## Banks

`audio_banks.xtbl` lists the banks. Each bank's load mode is one of three strings next to the xtbl name: "Streaming", "SectorRead" and "Managed". The bank table is `0xEF8704`: 0x20-byte entries, count `0x2526B50`. Each entry holds a file handle or data pointer (+0x10, +0x14), the XACT bank (+0x18) and flags (+0x1C).

**`audio_load_bank`** (0x4812A0) loads `<name>.xwb` + `<name>.xsb`:

| Wave bank flags | Mode | How it loads |
|---|---|---|
| 1 | **Streaming** | `audio_create_streaming_wavebank` (0x481170). It looks the file up in the packfiles (0xBFD880) and opens the **.vpp packfile itself** with `CreateFileA(GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING, FILE_FLAG_OVERLAPPED \| FILE_FLAG_NO_BUFFERING)`. Then `CreateStreamingWaveBank` with the file's offset inside the packfile. **XACT reads the audio data directly, not through the game's streamer or file API.** |
| 2 | **SectorRead** | Deferred. The data arrives later in memory, and `audio_create_wavebank_from_memory` (0x481BF0; ECX = data, EDX = size) runs `CreateInMemoryWaveBank` and a `DoWork` under the critical section `0x25F53EC`. |
| 0 | **Managed** | `file_open` / `file_read` (the packfile file API, 01_packfile.md) into the wavebank pool, then `audio_create_in_memory_wavebank` (0x4810E0) |

The sound bank (`.xsb`) is always read with `file_open` into the soundbank pool, then `audio_create_soundbank` (0x481220) runs `CreateSoundBank`. If either half fails, both pools are released and the bank is dropped (0x480FA0). The bank archive is `real_soundbanks.vpp`.

## Per frame

Audio runs on two threads: its own audio thread, and the game thread, which hands voice updates to the job workers once a frame.

**Audio thread.** `audio_thread_proc` (0x46C900) is started with `thread_create` (0xBFA930, a CreateThread wrapper; handle in `0xE99C48`) from startup (0x520054) and 0x465270. A console command "audio_thread_force_to_hw_thread" can pin it to a hardware thread.

Each pass, step by step:
1. If `0x2526A4B` is set, clear it and return (the thread exits).
2. **Focus mute.** While the window is inactive (`0x252A406` clear), set the mastering voice (`0xF52E30`) volume to 0 (`IXAudio2Voice::SetVolume`, vtable +0x28). Set it back to 1.0 when the window is active again. `0x2526B58` caches the current state.
3. Enter the critical section `0x256D9C4` and take a start time in ms.
4. `IXACT3Engine::DoWork` (`0x2526B48`, vtable +0x20).
5. `audio_stream_issue_reads` (0x480DB0): issue the queued overlapped `ReadFile`s for streaming wave banks.
6. `audio_listener_select` (0x46C360): copy the game's listener snapshot and pick the listener.
7. `audio_set_listener` (0x47E7C0) with that position, orientation and velocity → the X3DAudio listener globals (position at `0xEEB254`).
8. `0x2526ABE = audio_point_in_chunk_volume(listener)` (interior check).
9. `audio_volume_zones_update` (0x46EDA0): voice-attached zones and the four category volume floors.
10. If no cutscene is playing (`0x2526B44 < 1`, `0x2527C08 < 1`) and the audio mode `0x144E760[0xE96B7C]` is 5: `audio_ambient_emitters_update` (0x477250).
11. `audio_voices_update` (0x47E850): the voice slot state machine.
12. `audio_stream_requests_update` (0x484760): poll the streaming reads.
13. `DoWork` and `audio_stream_issue_reads` again.
14. Clear the per-pass bit 2 on every node of the list `0xF52220` (+0x14) and on `0xF5221C`.
15. `audio_thread_record_overrun(start)` (0x46C7F0): `0x2526A74` = how many ms over 30 the pass took.
16. Leave the critical section, `Sleep(10)`.
17. `0xE99C44` = time since the last pass, in seconds. Every timer and fade in the audio code uses it.

Times come from `QueryPerformanceCounter` (enabled by `0x252A404`), with base `0x2346508` and two divisors: `0x23464F8` for ms and `0x2346500` for µs.

**Voice slots.** 128 records of 0x19C bytes from `0xEEB8F8`:

| Off | Field |
|---|---|
| +0 | XACT cue handle (−1 free, 0 not yet prepared) |
| +0x20 | volume |
| +0x2C / +0x34 / +0x38 | fade values |
| +0x5C / +0x60 / +0x64 | position |
| +0xA4 | category (0-3) |
| +0xB4 | generation: a voice handle is `slot \| generation << 8` |
| +0xC4 / +0xC8 / +0xF4 | timers in ms, counted down by `dt × 1000`; +0xF4 is also the sort key for the 3D update |
| +0x194 / +0x196 / +0x198 | u16 flag words: 0x80 active, 0x100 forced update, 0x400 (on +0x198) stopping, … |

**Listener.** `audio_listener_select` copies 0xC0 bytes from `0x256DAE8` (written by the game thread) to `0x256DA28` under the critical section `0x256DBA8`. It then picks one of five sources (recorded in `0x256DA24`):
- 4 when the object handle `0x2526A78` is valid: the snapshot block at +0x78 (`0x256DAA0`).
- 0 / 1 when `0x21703D4` is set and no override is on: 0x46C2C0(`0x21703D4`) false → block +0 (`0x256DA28`), true → block +0x1C (`0x256DA44`).
- 2 when `0x21703D4` is clear, 3 when the override flags `0x2527D14`/`0x2527CF7` or `0x2527D15` are set: block +0x38 (`0x256DA60`).

Each block is a position and a previous position. The position goes to `0x256D9E0`, position − previous to `0x256D9EC`, and the orientation matrix to `0x256D9F8` (identity unless 0xBE37D0 / 0xBE3A60 provide one). *Which game state each source is (camera, vehicle, cutscene) is still open.*

**Volume zones.** `audio_volume_zones_update` walks the nodes in `0xEE3508` {+8 voice handle, +0x14 zone index, +0x18 radius}:
- A node whose voice is dead goes back to the free list `0xEE350C`.
- A node whose radius is positive and whose listener distance² exceeds 0.7 × radius² stops its voice (0x47C740).
- Otherwise its zone (`0xEE3510[zone]`) contributes four category volumes. The minimum of each, clamped by the base `0xEE34F8..0xEE3504`, goes to `0xEE34E8..0xEE34F4`.

**Voice update.** `audio_voices_update`:
1. Ramp the four category volumes `0xEEB2E8[i]` toward the floors `0xEE34E8[i]` by `dt` (down) or back up to 1.0. `0xEEB284[i]` = "below 1.0".
2. Mixer updates: 0x46E4B0, `audio_ramp_toward` (0x46FE30; `ECX` = &target, `EDX` = &value, cdecl rate per second), 0x47C350, 0x47D4E0, 0x46F210, 0x46E790.
3. `0x2526AC0 = 1 − (1 − 0x2526AB0)(1 − 0x2526A90)`, clamped to 0..1.
4. For each of the 128 slots with a handle, run its state machine on the +0x198 flags: 0x1000 / 0x800 stop paths (0x4827B0, 0x482730), 0x400 stopping (0x47D170 / 0x47C4D0), 0x100 prepare (0x481F60, 0x47D040), 0x200 play (0x4820B0 / 0x482030, 0x47D0F0), 0x2000 / 0x4000 pause and resume (0x47D250), then the delay and lifetime timers and the fades. Changed voices are committed with 0x47D380 → 0x482600.

**Game side.** At the end of each game frame, `frame_end_handoff` (0x521A00) calls `audio_update_voices_on_workers` (0x4655B0):
1. `audio_voices_dispatch` (0x47E220; cdecl `(desc *, job **)` → bool):
   - Scans the 128 slots. An active slot goes to `0xEEB6F8` if it has the forced flag 0x100. Otherwise it becomes a candidate `{index, key}` in `0xEEB2F8`.
   - Sorts the candidates by key (qsort with 0x47D4B0) and appends them until there are 32 entries (at least 4 candidates).
   - Gets a job object from the pool `0xEC5BB8` and runs `audio_voice_job` (0x47D420) over each group of 8, with the descriptor `{fn, job, start, end, 4}`.
   - Returns whether anything was queued.
2. If it was:
   - `job_alloc("gameaudio.cpp", 0x2D5)` (0xBF3F50).
   - Set both job workers' mode (+0x38 of `0x33D63F8` / `0x33D64C8`) to 4 and `job_worker_wake`.
   - `audio_job_workers_mode_reset` (0x464FE0) puts the mode back to 3.
   - Kick and sync the job (vtable +4 / +0xC), then release the dispatch job object (vtable +0x10).

`audio_voice_job` (0x47D420) runs `audio_voice_update_one` (0x47D7F0) for each listed slot that is neither free nor stopping. That is probably the per-voice 3D calculation.

**Porting order:**
1. `audio_ramp_toward`, `audio_thread_record_overrun`, `audio_set_listener` (pure data).
2. `audio_stream_issue_reads` and `audio_stream_requests_update` (they sit on the already ported async I/O in `hddio.rs`).
3. `audio_listener_select` and `audio_volume_zones_update`.
4. `audio_voices_dispatch` / `audio_voice_job`. These can run synchronously at first.
5. `audio_voices_update`, the largest piece. It needs the voice-slot layout above and the XACT cue calls behind it.
6. `audio_ambient_emitters_update` (its five shape tests and five sound types).
7. The thread loop itself. Keep XACT (`DoWork`) original until the cue layer is ported.

Lua bindings (`audio_play`, `audio_play_for_character_do`, … in `luafn_*`) start cues.

## Port notes

- To keep the original data, a port has to read XACT 3 sound and wave banks (`.xsb` / `.xwb`) and `saintsrow.xgs`, or keep XACT itself.
- Streaming wave banks are read straight out of the `.vpp` by file offset, so a Rust packfile layer must expose the offset and size of each `.xwb` entry.
- DirectSound is only needed for Bink.

## Open

- The exact meaning of SectorRead banks (who supplies their memory).
- The voice-slot flag bits in full, and `audio_voice_update_one` (0x47D7F0).
- The radio system.
