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
| 8 | DoWork | the audio thread, 0x481BF0, `loading_screen_pump` 0x481080, 0x481D20 |
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

**Audio thread.** `audio_thread_proc` (0x46C900) is started with `thread_create` (0xBFA930, a CreateThread wrapper; handle in `0xE99C48`) from startup (0x520054) and 0x465270. A console command "audio_thread_force_to_hw_thread" can pin it to a hardware thread.

Each pass:
1. Measure time with `QueryPerformanceCounter`.
2. While the window is inactive (`0x252A406`), set the mastering voice's volume to 0 (`IXAudio2Voice::SetVolume`, slot 10). Set it back to 1 when the window is active again.
3. `IXACT3Engine::DoWork`, and 0x480DB0.
4. Update the listener and emitters: 0x46C360, 0x47E7C0, `audio_point_in_chunk_volume`, 0x46EDA0, 0x477250, 0x47E850, 0x484760.
5. `DoWork` again.
6. `0x46C7F0` keeps the frame timing (`0x2526A74`).

It stops when `0x2526A4B` is set.

**Game side.** At the end of each game frame, `game_frame_tick` → 0x521A00 → `audio_queue_frame_job` (0x4655B0) queues the job `audio_frame_job` (0x464FE0, tagged gameaudio.cpp line 0x2D5) on the job system. Lua bindings (`audio_play`, `audio_play_for_character_do`, … in `luafn_*`) start cues.

## Port notes

- To keep the original data, a port has to read XACT 3 sound and wave banks (`.xsb` / `.xwb`) and `saintsrow.xgs`, or keep XACT itself.
- Streaming wave banks are read straight out of the `.vpp` by file offset, so a Rust packfile layer must expose the offset and size of each `.xwb` entry.
- DirectSound is only needed for Bink.

## Open

- The exact meaning of SectorRead banks (who supplies their memory).
- The listener / emitter update functions in the thread loop.
- The radio system.
