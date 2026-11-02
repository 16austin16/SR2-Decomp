# 51: Cameras, cutscenes and animation (names round 51)

This round maps the camera, cutscene and animation code that earlier rounds left unnamed. It adds **291 names** in `docs/candidates_camanim.csv` (228 confirmed, 63 inferred; none are in `functions.csv`; notes end "(51)"), one rename in `docs/renames_camanim.csv`, ten large port maps and fifteen small port recipes. `functions.csv` itself was not edited.

## Method and caveats

- **Function starts.** Each address was taken from a call target, a data-table slot or a padding start, then checked by a recursive-descent walk (`jcc`/`jmp` targets, jump tables decoded from their bytes). A start that lies inside another function's walk is rejected (this removed `0x6C1000`, a mid-instruction address inside `0x6BFB50`). `tools/xrefs.py` byte hits were not used as callers; callers come from a raw `E8` scan mapped to the containing function. 8 names have no direct caller (they are reached through a table, an immediate pointer or a callback); their prologues were read.
- **True sizes** are start to the highest instruction reached (including the tail after the last `ret`), not the next padding hole: for example `camera_follow_update` 0x49B4B0 is **0x1BA5** bytes (the padding-based "size" 0xCC is the first basic block only), and `cutscene_stop` is 0x2200.
- **Two big tables gave most of the names.** The cutscene *action* table (63 entries) and the *shot function* table (53 entries) carry their own strings, so those 124 names are confirmed by data rather than by reading code.
- **Port maps are machine-generated** by a block-level symbolic pass over the disassembly (calls with the register and stack setup that precedes them, absolute-address reads and writes, `fst`/`fstp` store points with the x87 expression that was stored, comparison polarity, jump tables, address-taken stack objects). They are a map, not a decompilation: `ST0<ret f>` means a value that came back in ST0 from call `f`; `stack[...]` lists values pushed since the previous call (callee-saved register saves are skipped). A few conventions to read the lines by:
  - `X87 STORE f32 [m] <- expr` is an `fstp/fst dword`: the value is rounded to f32 there. A `f64[...]` operand is a double constant (printed with its exact bits). The x87 control word is the game's (53-bit precision as `camera_pre_update` documents), so a port must keep intermediate results in f64 and round to f32 only at these store points.
  - `X87 COMPARE a vs b` followed by `if (...) goto Bn` lists *every* outcome (a greater, a less, equal, unordered) for which that jump is taken; `UNORDERED(NaN)` is part of the condition exactly when the `test ah, mask` / `sahf` pattern makes it so. The polarity was derived from the flag bits, not guessed from the mnemonic.
  - Integer `if (x <u y)` marks an unsigned compare, `(signed)` a sign test.
  Always re-check a block against `python tools/x86dis.py` before porting it.

## Doc errors and surprises found

1. **`0x4959A0 xtbl_load_camera_free` is not a loader.** It is `camera_mode8_activate` (EAX = &vec3, stack &mat33, fov, smooth, duration; 9 callers). The real `camera_free.xtbl` loader is **0x495AC0** (it was missing). Entry in `renames_camanim.csv`.
2. **Mode 4 is the scripted track camera, not mode 2.** `0x49FCA0` (called by `luafn_camera_script_is_finished`) tests `0x25F5AE8 == 4`, and `0x49F320` bakes a spline track (Hermite-style tangents `(next - prev) * 0.1`) for it. Mode 2 (`0x4A0140`) only sets the target fov to 60.0 and leaves the pose to the caller. Doc 19 lists mode 2 as "script / cutscene look-through".
3. **The camera has a mode stack**, not just a mode: `0x25F5AEC[]` with count `0x25F5B0C`, plus a base mode `0x25F5B10` (doc 19 knows only `0x25F5AE8` and `0x25F5B10`).
4. **The follow camera is context driven.** 24 contexts named `CFSM_*` (list below) are chosen per frame by `camera_human_context_classify` and indexed into a 0x2C-byte parameter table at `0x25F6318`. The names come from a debug overlay (`camera_debug_overlay_draw` 0x49E400) that nothing calls.
5. **Doc 32 `anim_track_is_playing` 0xCECB80** tests per-track flag bit `3*group` of the 0x58-byte track records; the sibling `0xCECBC0` tests bit `3*group+1`. "is playing" is the likely meaning of bit 0 only.
6. `0x6BFB50` (installed by `cutscene_play` as `0x2527D18`) is a *begin / first-frame* callback (player prepared: force holster, invulnerable, AI orders 0x13/0x23, spawn), and `0x6C44C0` (installed by `0x6C4470` as the same callback after a fade) is the **stop / teardown**. Doc 23 calls `0x6BFB50` "the cutscene frame callback". Per-frame work is `cutscene_do_frame` 0x6C6870.
7. The vehicle camera parsers live at **0xAE5C50-0xAEE5F0**, between vehicle-info parsers; they were unnamed.
8. The task text says doc 31 covers animation; it is `32_animation.md`.
9. Skipped on purpose: `0x6DAD20` (0x2B72 bytes, 226 blocks, 1537 x87 ops, debug line drawing, 6 stack args), `0x6E13F0` (0x1DEA, character appearance serialisation, not animation), `0x6DD990` (0x7D4, qsort of nearby humans), `0x6F0E10` / `0x6F1620` (pairwise human pose constraint; ownership unclear).

## Structures

### Camera modes and the mode stack

| Address | Meaning |
|---|---|
| `0x25F5AE8` | current mode |
| `0x25F5AEC[]`, count `0x25F5B0C` | mode stack (`camera_mode_push` / `camera_mode_pop`) |
| `0x25F5B10` | base mode (used when the stack is empty); `camera_mode_set_base` / `_default` (default from `0x2526C20`) |
| `0xDD5250 + mode*0x14` | mode record: enter, exit, update, slot 3, slot 4 (14 modes, 0-13) |
| `0x25F5BE8` flag 2 | smooth blend active (`camera_smooth_blend_update` 0x4A33A0 runs each frame) |
| `0x25F6990` quaternion, `0x25F69A0` pos, `0x25F69AC` fov, `0x25F69B0` elapsed, `0x25F69B4` duration, `0x25F69B8` type (7) | the blend start state written by `camera_smooth_blend_begin` |
| `0xF53AE4` count, `0xF53AE8 + 8*i` {fn, arg} | pending callbacks run by `camera_pending_callbacks_update`; an entry is removed when the call returns AL != 0 |

Modes: 0 follow (enter 0x49B2A0, exit 0x49B420, update 0x49B4B0); 1 free (enter 0x4A1560, update stub 0x4A1590); 2 external (enter 0x4A0140, stub 0x4A0150); 4 scripted track (update 0x49F1C0); 5 animation bones (0x494D60); 7 (enter 0x495590, update 0x4956C0); 8 (update 0x495950, activated by 0x4959A0); 9 scripted shot (enter 0x4A2730, update 0x4A2400, slots 0x4A0FB0); 10 (0x4A1020); 11 customisation orbit (0x494650, slots 0x49EF50 / 0x49EF70); 12 (0x49EF80); 13 (0x4A18E0). Identities of 7, 10, 12 and 13 remain unproven.

### Follow camera state (`camera_follow_*`)

| Address | Meaning |
|---|---|
| `0xE9A5B8` | target handle (also written to `0x25F5BB0`) |
| `0xE9A5BC` | context id 0-0x17 (0x19 = none, not stored) |
| `0xE9A5C0` / `0xE9A5C4` / `0xE9A5C8` | pending transition mode / duration (f32) / blend (f32, 1.0 once committed) |
| `0xE9A5CC..0xE9A5DC` | previous handle, ctx, mode, duration, blend |
| `0xE9A5E0`, `0xE9A60C`, `0xE9A638` | three 0x2C-byte copies of a params record; **`0xE9A638` is the active one** |
| `0x25F6318 + ctx*0x2C` | the params table; record = 11 dwords: +0x00 vec3 offset, +0x18 angle, +0x1C radius (`camera_params_set_polar`), the rest unmapped |
| `0xE9A664` | desired position (vec3), `0xE9A670` the saved previous one |
| `0xE9A684` | context flag word (`camera_context_set_flags`) |
| `0x2149C64 + (h & 0xFFF)*16` | object table: dword 0 = object*; valid when `obj+0x44 == h`; `obj+0x4C` = type (1 human, 5 vehicle) |

Contexts (index = `CFSM_` list order): 0 EXTERIOR_CLOSE, 1 INTERIOR_CLOSE, 2 INTERIOR_SPRINT, 3 VEHICLE_DRIVER, 4 VEHICLE_DRIVER_ALT, 5 WATERCRAFT_DRIVER, 6 HELICOPTER_DRIVER, 7 HELICOPTER_FINE_AIM, 8 AIRPLANE_DRIVER, 9 ZOOM, 10 SWIMMING, 11 SPECTATOR, 12 FENCE, 13 RAGDOLL, 14 FALLING, 15 LEAPING, 16 FINE_AIM, 17 FINE_AIM_CROUCH, 18 MELEE_LOCK, 19 FINE_AIM_VEHICLE, 20 FREEFALL, 21 PARACHUTE, 22 HUMAN_SHIELD, 23 SPRINT; also `CFCM_USER/SWINGING/LOCKED/STICKY` and `CF_TM_CUT/DELAYED_CUT/LINEAR/RAMP` (transition modes). `camera_human_context_classify` returns, in this order of tests: 0xA if `human+0xE88`; 0xD if state byte `+0x595` is not 0 or 3; 0xC if `+0x591 == 5`; 0xE (falling predicate); 9 (zoom); 0xF if the model index (`+0x5E0 -> +0x164`) is 0x441; 0x11, 0x10, 0x12; 0x16; 0x17; 2; 0x14; 0x15 (bit 30 of `+0xC0`); else `0x49EDB0`.

### Camera shake

Presets are 24-byte records at `0xF539E8`; the live state is `0xE9A6BC` (end time), `0xE9A6C4` (start time), `0xE9A6C0`, `0xF53AA8..0xF53ABC`, `0x2526C24..0x2526C44`, `0x25F6888..0x25F6894`. Times are the ms clock `0xE843AC` wrapped at `0x6B49D200` (1.8e9 ms). `camera_shake_from_world_point` gates on a distance < 30.0 to the player.

### Cutscene tables

- **Player** `*0x2527D10` (0x43E8 bytes): shot count `+0x153C`, interface document `+0x15F4/+0x15F8`, character list pointer `+0xAC8` count `+0xAD0`, vehicle list `+0xAE0` count `+0xAE8` (each element a pointer to an object with a virtual getter at `+0x34`), flags `+0xAFD`, `+0xAFF`, `+0xB01..+0xB04`, cutscene hashes `+0x1664`. Globals: `0x2527D14` playing, `0x2527D15` stopping, `0x2527CF7`, `0x2527D18` frame callback, `0x2527D1C..0x2527D25` flags, `0xE8DD22` skippable.
- **Cutscene table** `0x145A4A4`, count `0x145A4AC`, 0x14-byte entries (name pointer at +0; byte +8 = seen flag, saved as a bitset by `cutscene_seen_flags_store_bits`).
- **Action table** `0xDD2B28`, 16-byte entries `{name, start fn, update fn or 0, kind}`; every handler is `cdecl (action record*)` with the record's strings at `+8` (subject), `+0xC` (target), `+0x10` ... The 63 start functions are `cutscene_action_<slug>`, the shared updates `cutscene_action_<slug>_update`.
- **Shot function table** `0xDD28A0`, 12-byte entries `{name, fn, nargs}`, 53 entries ("Motion Blur", "Near Clip (DANGER)", "Shot Fade In:" ...); each is `cdecl (record*)` with floats read from `+8`. The first slot has an empty name; 5 handlers are 1-6 byte stubs (`Load Item Mesh`, `Shader LOD Restore`, `Player Outfit`, `DEBUG Console`, `Japan Exit`). Named `cutscene_shotfn_<slug>`.
- Packets `0x6F` (`cutscn_pkt`): the five senders are `cutscn_pkt_send_sub_1..5`; the receiver is `net_recv_cutscn_pkt`.

### Rigs and animation

- **Rig handle** `h`: slot `h % 0xBE`, generation `h / 0xBE`; object = `0x340D240 + slot*0x5E8` (inline division by multiplication with `0xAC769185` in every handle thunk). Object `+0x18` -> rig definition (`+0x24` tag count, `+0x30` bone count, `+0x38` tag array, `+0x3C` bone array of 0x3C-byte records whose first dword is the name pointer); `+0x23C` track count; track records are 0x58 bytes from `+0x254` with the id (`handle >> 4`) in the first dword; per-track flag bits live at `+0x280 + track*0x58` with bit index `3*group + k`.
- **Human animation layers**: three 0x34-byte records from `human+0xCC` (`+0x18` flag byte bit 0 = active, `+0` state or 0xFFFF, `+4` track id).
- **Anim state tables**: 0x2A4 entries of 12 bytes at `0xE8EE90` with hashes at `0x28F05E8` (`anim_state_find_by_name`), 0x539 entries at `0xE90E40` with hashes at `0x28F1078`.

## Names

All 291 rows are in `docs/candidates_camanim.csv` with full notes; this table gives address, name and confidence (C confirmed, I inferred).


### camera (71)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x493B80` | `camera_set_position_track_move` | C | EAX = &vec3, cdecl: sets desired 0x25F5B20 and current 0x25F5B14 = vec, and keeps max(0x2526BE8, vec3_dista... |
| `0x493C10` | `camera_set_orientation_both` | C | EAX = &mat33 (9 floats), cdecl: copies the matrix into the mode-desired orientation 0x25F5B5C and the curre... |
| `0x493C80` | `camera_mode_push` | C | ECX = mode index, cdecl: removes ECX from the mode stack 0x25F5AEC[] (count 0x25F5B0C) if present, pushes i... |
| `0x493E30` | `camera_mode_pop` | C | EDX = mode index, cdecl: removes EDX from the mode stack 0x25F5AEC[]; the new current is the new top or the... |
| `0x493EE0` | `camera_mode_set_base` | C | EAX = mode, stack (u8 smooth, f32 duration), cdecl: stores the base mode 0x25F5B10; with an empty stack and... |
| `0x494650` | `camera_mode11_update` | I | mode 11 update fn: customisation orbit camera driven by the stick (0x4FFCC0 input_stick_action_value_curved... |
| `0x494D60` | `camera_mode5_update_anim_bones` | I | mode 5 update fn: follows animation bones (Use Camera Lookat Bone / Position Bone): 0x6C7A40 / 0x6C7C70 / 0... |
| `0x495590` | `camera_mode7_enter` | I | mode 7 enter fn: normalises vec3 pair at 0xF53928.., uses sqrt and constants 0xE5C3D8 / 0xE9A664..; seeds t... |
| `0x4956C0` | `camera_mode7_update` | I | mode 7 update fn: 0x4938C0 (ray clip), 0x4952D0 / 0x495410 / 0x4954A0 smoothing helpers, 0x4968E0, 0x49AE60... |
| `0x495950` | `camera_mode8_update` | I | mode 8 update fn (0x44 bytes, 2 x87 ops): trivial update with no callers other than the table |
| `0x495A50` | `camera_lookat_bone_resolve` | I | no args: when the 0x2345B68 text is valid ("none" clears 0x2526BAA) else resolves the bone name at 0x2345A6... |
| `0x497220` | `camera_follow_vehicle_chase_update` | I | EDX = input (read before written), cdecl helper of camera_follow_update path 0x498A20: vehicle chase camera... |
| `0x499940` | `camera_params_set_polar` | I | ESI = &params record (0x2C bytes), stack (f32 a, f32 b) ret 8: |
| `0x499AF0` | `camera_follow_request_equals_current` | C | ECX = &{u32 handle, u32 ctx, u32 mode, f32 dur, f32 blend}: AL = 1 when all five match 0xE9A5B8/BC/C0/C4/C8... |
| `0x499B50` | `camera_follow_compute_desired_position` | I | no args, 6 callers (camera_cut, 0x49A260, 0x49A330, 0x49A4C0, 0x49D090): computes the follow camera desired... |
| `0x49A330` | `camera_follow_state_reset` | C | EAX = target handle: resets the follow state 0xE9A5B8.. (ctx 0, mode 2, blend 1.0), loads params[0] (0x25F6... |
| `0x49A670` | `camera_follow_commit_context` | C | no args: clears flag 2 of 0x25F5BE8, blend 0xE9A5C8 = 1.0, copies params[ctx 0xE9A5BC] (0x25F6318 + ctx*0x2... |
| `0x49A6E0` | `camera_follow_set_target_context` | C | EAX = target handle, EDX = transition mode, stack (u32 context, f32 duration) ret 8: sets 0xE9A5B8 and 0x25... |
| `0x49A870` | `camera_ctx_pred_zoom` | C | ECX = human (0 ok): AL = type(+0x4C)==1 && |
| `0x49A8C0` | `camera_ctx_pred_falling` | I | EDX = human: AL = state byte +0x591 == 6 and +0xC8 bit 0x1000 clear, with the model-index exclusions of tab... |
| `0x49A920` | `camera_ctx_pred_fine_aim` | C | ECX = human (0 ok): AL = type==1 && |
| `0x49A950` | `camera_ctx_pred_fine_aim_crouch` | C | ECX = human (0 ok): as camera_ctx_pred_fine_aim and also |
| `0x49A980` | `camera_ctx_pred_melee_lock` | C | ECX = human (0 ok): AL = type==1 && |
| `0x49A9B0` | `camera_ctx_pred_human_shield` | I | EDX = human: AL = 1 when +0xEA8 set and 0x9B3680() false; else state byte +0x149 == 0x22 && +0x158 == 0 && ... |
| `0x49AA00` | `camera_ctx_pred_sprint` | I | EAX = human: AL = 1 when 0x960120() is true, +0xEA8 == 0 and the player (0x21703D4) is NOT inside a flag-8 ... |
| `0x49AA40` | `camera_ctx_pred_interior_sprint` | I | EAX = human: twin of camera_ctx_pred_sprint (same 0x960120 + 0x4AB430 test) for the interior case; true -> ... |
| `0x49AA80` | `camera_ctx_pred_freefall` | C | EAX = human: AL = |
| `0x49AAB0` | `camera_context_for_handle` | C | ECX = object handle: looks it up in the object table (0x2149C64, idx = h&0xFFF, 16 bytes); human (type +0x4... |
| `0x49AB10` | `camera_human_context_classify` | C | ESI = human (may be 0): returns EAX = camera context id 0x00-0x17 from a chain of the 0x49A8xx-0x49AAxx pre... |
| `0x49ACD0` | `camera_context_set_flags` | C | EAX = context id (3..0x16 via byte table 0x49ADB0, jump table 0x49AD94), stack (u32 mode) ret 4: rewrites f... |
| `0x49B2A0` | `camera_follow_mode_enter` | I | mode 0 enter fn of 0xDD5250 (no args): enters the gameplay follow camera (Rust-port neighbour of camera_fol... |
| `0x49B420` | `camera_follow_mode_exit` | I | mode 0 exit fn of 0xDD5250 (no args): runs 0x4981D0 / 0x4982F0 (spring state release), 0xBE2D90, resets 0xE... |
| `0x49D3B0` | `camera_follow_snap_to_object` | C | EAX = object (handle +0x44, pos +0x14, orient +0x20): sets 0x25F5BB0/BB4/BC0 from it then calls 0x49A6E0(ct... |
| `0x49D8F0` | `camera_params4_set` | I | EAX = &vec3, stack (f32,f32,f32) ret 12: derives polar fields (0x499940) and fills params[4] at 0x25F63C8 (... |
| `0x49DAE0` | `camera_params_reload_current` | C | stack (f32,f32) ret 8: 0x499940 on params[current ctx], then copies the 0x2C-byte record into the active pa... |
| `0x49DB30` | `camera_params_set_current_offset` | C | ECX = &vec3: writes params[current ctx].vec3 (+0,+4,+8) and copies the record to the active params 0xE9A638 |
| `0x49E210` | `camera_set_fov_limit` | I | CL = enable, stack (f32 v) cdecl: unless 0x2526BA3 is set and CL clear, clamps v into the range around (-1.... |
| `0x49E380` | `camera_debug_print_line` | C | EAX = x pixel, stack (fmt, ...) cdecl, EBX = &y: _vsprintf into 0x23460E0, measures it (0xBD7730), gfx_set_... |
| `0x49E400` | `camera_debug_overlay_draw` | C | no args (0 callers, debug): draws the camera debug text overlay (gfx_set_color, gfx_draw_text_int, world sp... |
| `0x49E920` | `camera_pitch_restore_pending` | C | no args: when |
| `0x49EEC0` | `camera_mode_set_base_default` | C | no args: base mode 0x25F5B10 = |
| `0x49EF80` | `camera_mode12_update` | I | mode 12 update fn: 0x4938C0 ray clip, vec3_normalize_safe, 0xBDE5E0, 0xBE49A0; state 0xF539DC / 0xF539E0 |
| `0x49F1C0` | `camera_mode4_script_update` | I | mode 4 (scripted / look-through track camera, see camera_script_is_finished_check 0x49FCA0) update fn: eval... |
| `0x49F320` | `camera_script_build_track` | I | EAX = track description* (point arrays at +0x1C / +0x28 / +0x40 with 0x18-byte strides, count in EDI), cdec... |
| `0x49FCA0` | `camera_script_is_finished_check` | C | no args -> AL, called only by luafn_camera_script_is_finished (0xA2A0E0): AL = 1 when camera mode 0x25F5AE8... |
| `0x49FCF0` | `camera_look_through_update` | I | EAX/EDI/EDX inputs, called only by luafn_camera_look_through_do (0xA2A030): computes the look-through camer... |
| `0x4A0140` | `camera_external_mode_enter` | C | mode 2 enter fn: 0x25F5BA8 (target fov) = |
| `0x4A01B0` | `camera_shake_apply` | I | no args (called from camera_effects_update): random shake offset (rng_float01) scaled by remaining time (ti... |
| `0x4A0700` | `camera_effect_pulse_apply` | I | no args: timer_remaining-driven effect with state 0xE9A6C8 / 0xF539E4; one of the three appliers in camera_... |
| `0x4A07A0` | `camera_effect_sway_apply` | I | no args: timer_remaining + 0xBE50F0 driven effect with state 0xE9A6C0 / 0xF53AA8 / 0xF53AB0; second applier... |
| `0x4A0A10` | `camera_shake_start` | I | ECX = fade-in ms, stack (f32 amplitude, i32 hold ms, i32 fade-out ms, f32 rate, u32 flags): writes the shak... |
| `0x4A0B60` | `camera_shake_start_preset` | I | EAX = preset index (24-byte records at 0xF539E8), stack (f32 scale), cdecl: calls camera_shake_start with t... |
| `0x4A0BC0` | `camera_shake_from_world_point` | I | EAX = &vec3 source position, stack (f32 radius): distance to the player (0x21703D4 +0x14) -> falloff -> cam... |
| `0x4A0CA0` | `camera_effect_sway_start` | I | EDI = params, timer_remaining based starter for the 0x4A07A0 effect (state 0xE9A6C0, 0xF53AA8); 4 callers (... |
| `0x4A0EA0` | `camera_effects_update` | C | no args, step 6 of camera_update: runs camera_shake_apply 0x4A01B0 and the effect appliers 0x4A0700 / 0x4A0... |
| `0x4A0FB0` | `camera_blend_state_reset` | I | no args; mode 9 slot-3/4 callback of 0xDD5250: zeroes 0x2526C48, copies the origin vec3 0x252A6D0 into the ... |
| `0x4A1020` | `camera_mode10_update` | I | mode 10 update fn: 0x4938C0 ray clip, 0xBE2D90, 0xBE3150; reads 0xE997E0 / 0xE8B048, ~160 x87 ops |
| `0x4A1560` | `camera_free_mode_enter` | C | mode 1 enter fn: 0x25F5BA8 (target fov) = |
| `0x4A18E0` | `camera_mode13_update` | I | mode 13 update fn: timer_remaining + quat_to_mat33 (0xC057D0) interpolation, state 0xE9A6E4 / 0xE5CC98 |
| `0x4A1B60` | `camera_bone_world_position` | I | EAX = human object: seat occupant 0x960D60 and rig bone transform (0xCEDF10 rig_bone_transform) -> world po... |
| `0x4A1C60` | `camera_look_at_orientation` | I | builds a camera orientation from a look-at (0x4A1A50 angle wraps, 0x4A1B60 bone lookup, normalise, mat33_fr... |
| `0x4A2400` | `camera_mode9_update` | I | mode 9 update fn: scripted / cinematic shot camera; 0x4A1C60 (look-at orient), 0x4A1D60 / 0x4A1F90 / 0x4A20... |
| `0x4A2730` | `camera_mode9_enter` | I | mode 9 enter fn: seeds the scripted shot camera (0xF53AC0 record) via 0x4F1AC0 / 0xBE3380 |
| `0x4A2840` | `camera_script_shot_begin` | I | EAX = shot request: ray 0x6FA330 test, 0x493EE0 camera_mode_set_base to mode 9 and 0x4A29E0 shot set; calle... |
| `0x4A2980` | `camera_script_shot_end` | I | no args: blends back with 0x4A33F0 camera_smooth_blend_begin; called by ai_order_k23_update |
| `0x4A29E0` | `camera_script_shot_set` | I | ESI = shot record: fills the scripted shot state 0xF53AC0 (position / look-at / name 0xF53ADF); 10 callers ... |
| `0x4A2EA0` | `camera_blend_orientation` | I | no args: quaternion blend (0xC053E0 / 0xC05E80 / 0xC057D0 quat_to_mat33) of the saved orientation 0x25F6990... |
| `0x4A3140` | `camera_blend_position` | I | EAX = blend record: position blend using spline_eval_position / spline_eval_tangent (0xC03910 / 0xC03AF0), ... |
| `0x4A33A0` | `camera_smooth_blend_update` | C | no args, called from camera_update when flag 2 of 0x25F5BE8 is set: runs the orientation blend 0x4A2EA0 and... |
| `0x4A33F0` | `camera_smooth_blend_begin` | C | stack (f32 duration), cdecl (41 callers): 0xC053E0(EAX = 0x25F6990, EDI = 0x25F5B38) turns the current orie... |
| `0x4A3540` | `camera_pending_callbacks_update` | C | no args, first call of camera_update: walks the list {count 0xF53AE4, entries 8 bytes at 0xF53AE8} calling ... |

### cutscene (158)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x6BD950` | `cutscene_debug_menu_build` | C | no args (debug): builds the "Select a cutscene to play" list from the cutscene table (prefixes IG_ / SH_ / ... |
| `0x6BDBA0` | `cutscene_draw_letterbox` | I | no args: black fill via gfx_device_acquire, gfx_set_tint_color / gfx_set_color and im2d_fill_rect (bars); 5... |
| `0x6BE320` | `cutscene_stream_wait_loop` | I | EAX = request, called from cutscene_play and 0x6C4050: spins rsched_update_all / streamman_process until rs... |
| `0x6BFB50` | `cutscene_begin_callback` | I | no args (0x1AF5 bytes), installed as the callback |
| `0x6C24E0` | `cutscene_shot_place_characters` | I | no args, called from cutscene_shot_start: positions the shot characters (teleport via object_teleport, 0x6C... |
| `0x6C2EF0` | `cutscene_shot_place_vehicles` | I | no args, called from cutscene_shot_start: positions the shot vehicles (vehicle_sync_from_physics, mat33_fro... |
| `0x6C3970` | `cutscene_table_clear_seen_flags` | C | no args: clears byte +8 of every 0x14-byte entry of the cutscene table (0x145A4A4, count 0x145A4AC), sets s... |
| `0x6C3A20` | `cutscene_table_find_by_hash` | C | stack (u32* name hash) cdecl, returns EAX = entry* (stride 0x14) or 0: hashes each entry name (string_hash_... |
| `0x6C3AD0` | `cutscene_seen_flags_store_bits` | C | ESI = &bitset ptr: for each table entry i sets/clears bit i (mask table 0xDC8558) of the bitset from entry ... |
| `0x6C3B30` | `cutscene_seen_flags_load_bits` | C | ESI = &bitset ptr: entry byte +8 = bit i of the bitset (load side of cutscene_seen_flags_store_bits) |
| `0x6C4370` | `cutscene_remember_player_position` | I | no args: unless 0xE8DD22 (skippable) is set, copies the player (0x21703D4) position +0x14 into 0x2F9B7E4 an... |
| `0x6C43F0` | `cutscene_local_skip_request` | I | no args: reads text CUTSCENE_LOCAL_SKIPPING (lstr_get) and, in co-op (coop_session_active), sends skip stat... |
| `0x6C4470` | `cutscene_fade_out_and_stop` | C | no args: gfx_set_color(0,0,0,255) + screen_fade_start; when 0xE94D08 < 0 installs the frame callback |
| `0x6C44C0` | `cutscene_stop` | I | no args, installed as frame callback |
| `0x6C6D70` | `cutscene_player_flag_b01` | C | no args: AL = player |
| `0x6C6DD0` | `cutscene_player_flags_afd_not_aff` | C | no args: AL = |
| `0x6C6E00` | `cutscene_playing_time_check` | C | no args: AL = |
| `0x6C6E80` | `cutscene_object_is_cutscene_character` | C | EDI = object: AL = 1 when a cutscene is playing and (obj +0xCC & 0x400000 or obj equals a character of the ... |
| `0x6C6EE0` | `cutscene_object_is_cutscene_vehicle` | C | EDI = object: AL = 1 when playing and the object handle (+0x44) equals a vehicle of the player list at +0xA... |
| `0x6C6F50` | `cutscene_object_is_participant` | I | EAX = object: calls cutscene_object_is_cutscene_character and _vehicle (EDI = object); 3 callers |
| `0x6C7D30` | `cutscene_slate_lookup` | I | looks up the navpoint named by the "cutscene slate" (navpoint_find, 0x6C95E0); callers cutscene_shot_start,... |
| `0x6C8130` | `cutscene_shot_start` | I | no args: starts a shot ("shot%u", "safe_frame"): camera_update, tod_zone_update, weather_rain_effects_updat... |
| `0x6CAA60` | `cutscene_action_comment` | C | cdecl (action record*), start fn of cutscene action "COMMENT" (table 0xDD2B28, entry flags 0) |
| `0x6CAA70` | `cutscene_action_message_debug` | C | cdecl (action record*), start fn of cutscene action "MESSAGE (DEBUG)" (table 0xDD2B28, entry flags 4) |
| `0x6CAA80` | `cutscene_action_teleports_to` | C | cdecl (action record*), start fn of cutscene action "TELEPORTS TO" (table 0xDD2B28, entry flags 11); calls ... |
| `0x6CAB00` | `cutscene_action_walk_to` | C | cdecl (action record*), start fn of cutscene action "WALK TO" (table 0xDD2B28, entry flags 3), update fn 0x... |
| `0x6CAB70` | `cutscene_action_run_to` | C | cdecl (action record*), start fn of cutscene action "RUN TO" (table 0xDD2B28, entry flags 3), update fn 0x6... |
| `0x6CABE0` | `cutscene_action_walk_to_run_to_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'WALK TO', 'RUN TO' (table 0xDD2B28); cal... |
| `0x6CACD0` | `cutscene_action_suicides` | C | cdecl (action record*), start fn of cutscene action "SUICIDES" (table 0xDD2B28, entry flags 1); calls human... |
| `0x6CAD20` | `cutscene_action_attack` | C | cdecl (action record*), start fn of cutscene action "ATTACK" (table 0xDD2B28, entry flags 3), update fn 0x6... |
| `0x6CADD0` | `cutscene_action_attack_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'ATTACK' (table 0xDD2B28); calls human_is... |
| `0x6CAE70` | `cutscene_action_fire_at` | C | cdecl (action record*), start fn of cutscene action "FIRE AT" (table 0xDD2B28, entry flags 3), update fn 0x... |
| `0x6CAEE0` | `cutscene_action_fire_at_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'FIRE AT' (table 0xDD2B28); calls human_i... |
| `0x6CAF40` | `cutscene_action_equip_weapon` | C | cdecl (action record*), start fn of cutscene action "EQUIP WEAPON" (table 0xDD2B28, entry flags 5); calls h... |
| `0x6CAFF0` | `cutscene_action_ragdoll` | C | cdecl (action record*), start fn of cutscene action "RAGDOLL" (table 0xDD2B28, entry flags 5); calls human_... |
| `0x6CB090` | `cutscene_action_ragdoll_throw` | C | cdecl (action record*), start fn of cutscene action "RAGDOLL THROW" (table 0xDD2B28, entry flags 7); calls ... |
| `0x6CB110` | `cutscene_action_look_at` | C | cdecl (action record*), start fn of cutscene action "LOOK AT" (table 0xDD2B28, entry flags 3); calls script... |
| `0x6CB1D0` | `cutscene_action_look_at_nav` | C | cdecl (action record*), start fn of cutscene action "LOOK AT NAV" (table 0xDD2B28, entry flags 3); calls hu... |
| `0x6CB250` | `cutscene_action_look_at_nothing` | C | cdecl (action record*), start fn of cutscene action "LOOK AT NOTHING" (table 0xDD2B28, entry flags 1); call... |
| `0x6CB2A0` | `cutscene_action_cam_cut_to` | C | cdecl (action record*), start fn of cutscene action "CAM CUT TO" (table 0xDD2B28, entry flags 2); calls 0x4... |
| `0x6CB2E0` | `cutscene_action_cam_track_char` | C | cdecl (action record*), start fn of cutscene action "CAM TRACK CHAR" (table 0xDD2B28, entry flags 2); calls... |
| `0x6CB330` | `cutscene_action_cam_look_at_char` | C | cdecl (action record*), start fn of cutscene action "CAM LOOK AT CHAR" (table 0xDD2B28, entry flags 2); cal... |
| `0x6CB390` | `cutscene_action_cam_look_at_nav` | C | cdecl (action record*), start fn of cutscene action "CAM LOOK AT NAV" (table 0xDD2B28, entry flags 2); call... |
| `0x6CB490` | `cutscene_action_cam_slide_to` | C | cdecl (action record*), start fn of cutscene action "CAM SLIDE TO" (table 0xDD2B28, entry flags 6), update ... |
| `0x6CB590` | `cutscene_action_cam_slide_to_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'CAM SLIDE TO' (table 0xDD2B28); calls  |
| `0x6CB5E0` | `cutscene_action_cam_stop_slide` | C | cdecl (action record*), start fn of cutscene action "CAM STOP SLIDE" (table 0xDD2B28, entry flags 0) |
| `0x6CB680` | `cutscene_action_cam_stop_track` | C | cdecl (action record*), start fn of cutscene action "CAM STOP TRACK" (table 0xDD2B28, entry flags 0); calls... |
| `0x6CB6A0` | `cutscene_action_cam_follow_target` | C | cdecl (action record*), start fn of cutscene action "CAM FOLLOW TARGET" (table 0xDD2B28, entry flags 4); ca... |
| `0x6CB6E0` | `cutscene_action_hide_character` | C | cdecl (action record*), start fn of cutscene action "HIDE CHARACTER" (table 0xDD2B28, entry flags 9); calls... |
| `0x6CB730` | `cutscene_action_show_character` | C | cdecl (action record*), start fn of cutscene action "SHOW CHARACTER" (table 0xDD2B28, entry flags 9); calls... |
| `0x6CB780` | `cutscene_action_disable_physics` | C | cdecl (action record*), start fn of cutscene action "DISABLE PHYSICS" (table 0xDD2B28, entry flags 1); call... |
| `0x6CB7E0` | `cutscene_action_enable_physics` | C | cdecl (action record*), start fn of cutscene action "ENABLE PHYSICS" (table 0xDD2B28, entry flags 1); calls... |
| `0x6CB850` | `cutscene_action_hide_vehicle` | C | cdecl (action record*), start fn of cutscene action "HIDE VEHICLE" (table 0xDD2B28, entry flags 9); calls s... |
| `0x6CB890` | `cutscene_action_show_vehicle` | C | cdecl (action record*), start fn of cutscene action "SHOW VEHICLE" (table 0xDD2B28, entry flags 9); calls s... |
| `0x6CB8E0` | `cutscene_action_hide_group` | C | cdecl (action record*), start fn of cutscene action "HIDE GROUP" (table 0xDD2B28, entry flags 9); calls gro... |
| `0x6CB900` | `cutscene_action_show_group` | C | cdecl (action record*), start fn of cutscene action "SHOW GROUP" (table 0xDD2B28, entry flags 9); calls gro... |
| `0x6CB920` | `cutscene_action_play_effect` | C | cdecl (action record*), start fn of cutscene action "PLAY EFFECT" (table 0xDD2B28, entry flags 6); calls 0x... |
| `0x6CB9E0` | `cutscene_action_play_voice` | C | cdecl (action record*), start fn of cutscene action "PLAY VOICE" (table 0xDD2B28, entry flags 4); calls aud... |
| `0x6CBB10` | `cutscene_action_play_foley` | C | cdecl (action record*), start fn of cutscene action "PLAY FOLEY" (table 0xDD2B28, entry flags 4); calls aud... |
| `0x6CBC80` | `cutscene_action_speak_voice` | C | cdecl (action record*), start fn of cutscene action "SPEAK VOICE" (table 0xDD2B28, entry flags 5); calls 0x... |
| `0x6CBDE0` | `cutscene_action_speak_persona_line` | C | cdecl (action record*), start fn of cutscene action "SPEAK PERSONA LINE" (table 0xDD2B28, entry flags 5); c... |
| `0x6CBE50` | `cutscene_action_stop_lipsyncing` | C | cdecl (action record*), start fn of cutscene action "STOP LIPSYNCING" (table 0xDD2B28, entry flags 1); call... |
| `0x6CBE90` | `cutscene_action_teleport_car` | C | cdecl (action record*), start fn of cutscene action "TELEPORT CAR" (table 0xDD2B28, entry flags 3); calls n... |
| `0x6CBEF0` | `cutscene_action_enter_car` | C | cdecl (action record*), start fn of cutscene action "ENTER CAR" (table 0xDD2B28, entry flags 3), update fn ... |
| `0x6CBFD0` | `cutscene_action_enter_car_now` | C | cdecl (action record*), start fn of cutscene action "ENTER CAR NOW" (table 0xDD2B28, entry flags 11); calls... |
| `0x6CC070` | `cutscene_action_enter_car_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'ENTER CAR' (table 0xDD2B28); calls 0x6CA... |
| `0x6CC0F0` | `cutscene_action_exit_car` | C | cdecl (action record*), start fn of cutscene action "EXIT CAR" (table 0xDD2B28, entry flags 1), update fn 0... |
| `0x6CC190` | `cutscene_action_exit_car_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'EXIT CAR' (table 0xDD2B28); calls human_... |
| `0x6CC230` | `cutscene_action_drive_to` | C | cdecl (action record*), start fn of cutscene action "DRIVE TO" (table 0xDD2B28, entry flags 3), update fn 0... |
| `0x6CC320` | `cutscene_action_drive_to_drive_to_ex_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'DRIVE TO', 'DRIVE TO EX' (table 0xDD2B28... |
| `0x6CC3A0` | `cutscene_action_car_speed_ovrride` | C | cdecl (action record*), start fn of cutscene action "CAR SPEED OVRRIDE" (table 0xDD2B28, entry flags 5); ca... |
| `0x6CC410` | `cutscene_action_fade_in` | C | cdecl (action record*), start fn of cutscene action "FADE IN" (table 0xDD2B28, entry flags 4), update fn 0x... |
| `0x6CC460` | `cutscene_action_fade_out` | C | cdecl (action record*), start fn of cutscene action "FADE OUT" (table 0xDD2B28, entry flags 4), update fn 0... |
| `0x6CC4C0` | `cutscene_action_fade_in_fade_out_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'FADE IN', 'FADE OUT' (table 0xDD2B28); c... |
| `0x6CC4F0` | `cutscene_action_team_despawn` | C | cdecl (action record*), start fn of cutscene action "TEAM DESPAWN" (table 0xDD2B28, entry flags 5); calls 0... |
| `0x6CC540` | `cutscene_action_combat_enable` | C | cdecl (action record*), start fn of cutscene action "COMBAT ENABLE" (table 0xDD2B28, entry flags 5); calls ... |
| `0x6CC5A0` | `cutscene_action_door_open` | C | cdecl (action record*), start fn of cutscene action "DOOR OPEN" (table 0xDD2B28, entry flags 4); calls 0x91... |
| `0x6CC5E0` | `cutscene_action_door_close` | C | cdecl (action record*), start fn of cutscene action "DOOR CLOSE" (table 0xDD2B28, entry flags 4); calls 0x9... |
| `0x6CC610` | `cutscene_action_car_door_open` | C | cdecl (action record*), start fn of cutscene action "CAR DOOR OPEN" (table 0xDD2B28, entry flags 5); calls ... |
| `0x6CC6D0` | `cutscene_action_car_door_close` | C | cdecl (action record*), start fn of cutscene action "CAR DOOR CLOSE" (table 0xDD2B28, entry flags 5); calls... |
| `0x6CC770` | `cutscene_action_mesh_mover_play` | C | cdecl (action record*), start fn of cutscene action "MESH MOVER PLAY" (table 0xDD2B28, entry flags 5), upda... |
| `0x6CC7A0` | `cutscene_action_mesh_mover_play_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'MESH MOVER PLAY' (table 0xDD2B28); calls... |
| `0x6CC7D0` | `cutscene_action_mesh_mover_stop` | C | cdecl (action record*), start fn of cutscene action "MESH MOVER STOP" (table 0xDD2B28, entry flags 1); call... |
| `0x6CC7F0` | `cutscene_action_mesh_mover_reset` | C | cdecl (action record*), start fn of cutscene action "MESH MOVER RESET" (table 0xDD2B28, entry flags 1); cal... |
| `0x6CC810` | `cutscene_action_animate` | C | cdecl (action record*), start fn of cutscene action "ANIMATE" (table 0xDD2B28, entry flags 5); calls 0x6CA9... |
| `0x6CC950` | `cutscene_action_animate_car` | C | cdecl (action record*), start fn of cutscene action "ANIMATE CAR" (table 0xDD2B28, entry flags 5); calls 0x... |
| `0x6CCA50` | `cutscene_action_animation_state` | C | cdecl (action record*), start fn of cutscene action "ANIMATION STATE" (table 0xDD2B28, entry flags 5); call... |
| `0x6CCB60` | `cutscene_action_human_shield` | C | cdecl (action record*), start fn of cutscene action "HUMAN SHIELD" (table 0xDD2B28, entry flags 3), update ... |
| `0x6CCBE0` | `cutscene_action_human_shield_update` | C | cdecl (action record*), per-frame update fn of cutscene action(s) 'HUMAN SHIELD' (table 0xDD2B28); calls sc... |
| `0x6CCC60` | `cutscene_action_crib_item_hide` | C | cdecl (action record*), start fn of cutscene action "CRIB ITEM HIDE" (table 0xDD2B28, entry flags 13); call... |
| `0x6CCE40` | `cutscene_action_action_node` | C | cdecl (action record*), start fn of cutscene action "ACTION NODE" (table 0xDD2B28, entry flags 5); calls 0x... |
| `0x6CCEC0` | `cutscene_action_char_allow_crouch` | C | cdecl (action record*), start fn of cutscene action "CHAR ALLOW CROUCH" (table 0xDD2B28, entry flags 5); ca... |
| `0x6CCF20` | `cutscene_action_exit` | C | cdecl (action record*), start fn of cutscene action "EXIT" (table 0xDD2B28, entry flags 0) |
| `0x6CCF60` | `cutscene_action_mark_for_cutscene` | C | cdecl (action record*), start fn of cutscene action "MARK FOR CUTSCENE" (table 0xDD2B28, entry flags 1); ca... |
| `0x6CD1D0` | `cutscene_shot_parse_actions` | C | EAX = xml element: parses Actions / Action of a shot (Enabled, UniqueID, Subject, Target, plus per-type att... |
| `0x6CD6D0` | `cutscene_action_voice_prepare` | I | ECX = action: handles "PLAY VOICE" / "SPEAK VOICE" / "PLAY FOLEY" setup (audio_play_params_init, audio_prep... |
| `0x6CDD90` | `cutscene_shot_count_actions` | C | xml element: counts the Action children (xml_count_children) and allocates the array via vector_constructor... |
| `0x6CE6A0` | `cutscene_dynamic_mempool_create` | C | creates "dynamic mempool %d" / "dynamic mempool %d gpu" (a79560, bf2c10); caller cutscene_temp_mempool_init |
| `0x6CE850` | `cutscene_temp_mempool_init` | C | no args: creates "Cutscene temp split mempool" and "... gpu" (mempool::init, static_mempool_base::alloc, 0x... |
| `0x6CEB20` | `cutscene_dynamic_mempool_name` | C | ECX = name: formats "dyn mp %s" / "dyn mp %s gpu" pool names (_strncpy, bf2c10) |
| `0x6D0FE0` | `cutscene_parse_anim_states` | C | EAX = xml element: parses AnimStates / AnimState (Filename, Duration) of a cutscene character; callers cuts... |
| `0x6D3EB0` | `cutscene_anim_register` | I | ECX = anim name: normalises to .anim / .anim_pc (path_basename, _strrchr, path_change_extension), opens it ... |
| `0x6D4070` | `cutscene_anim_register_basic` | I | ECX = anim name: shorter .anim-only variant of 0x6D3EB0 (ce3050 + cec2a0); caller 0x6CD5F0 |
| `0x6D4570` | `cutscene_name_to_text_key` | C | stack (name): builds "CUTSCENE_%s" upper-cased and looks it up with lstr_lookup |
| `0x6D4630` | `cutscene_load_package_and_names` | I | no args: mounts cutscenes.vpp (packfile_find_mounted), loads <name>_pc / .xtbl entries and fills the cutsce... |
| `0x6D5830` | `cutscene_load_bink_movie` | C | no args: builds "./videos/%s%s.bik", logs "cutscene_load_bink_movie" and starts the Bink movie (0x6888D0 / ... |
| `0x6D7500` | `cutscene_shotfn_load_item_mesh` | C | cdecl (shot function record*; params from +8), shot function "Load Item Mesh" (table 0xDD28A0, entry 12 byt... |
| `0x6D7510` | `cutscene_shotfn_spawning_peds_disabled` | C | cdecl (shot function record*; params from +8), shot function "Spawning Peds Disabled" (table 0xDD28A0, entr... |
| `0x6D7550` | `cutscene_shotfn_spawning_cars_disabled` | C | cdecl (shot function record*; params from +8), shot function "Spawning Cars Disabled" (table 0xDD28A0, entr... |
| `0x6D7590` | `cutscene_shotfn_level_lights` | C | cdecl (shot function record*; params from +8), shot function "Level Lights" (table 0xDD28A0, entry 12 bytes... |
| `0x6D75B0` | `cutscene_shotfn_tree_shadows` | C | cdecl (shot function record*; params from +8), shot function "Tree Shadows" (table 0xDD28A0, entry 12 bytes... |
| `0x6D75D0` | `cutscene_shotfn_tod_character_shadows` | C | cdecl (shot function record*; params from +8), shot function "TOD character shadows" (table 0xDD28A0, entry... |
| `0x6D75E0` | `cutscene_shotfn_motion_blur` | C | cdecl (shot function record*; params from +8), shot function "Motion Blur" (table 0xDD28A0, entry 12 bytes ... |
| `0x6D7600` | `cutscene_shotfn_motion_blur_override` | C | cdecl (shot function record*; params from +8), shot function "Motion Blur Override" (table 0xDD28A0, entry ... |
| `0x6D7630` | `cutscene_shotfn_camera_moves_with_target` | C | cdecl (shot function record*; params from +8), shot function "Camera moves with target" (table 0xDD28A0, en... |
| `0x6D7640` | `cutscene_shotfn_disable_trees` | C | cdecl (shot function record*; params from +8), shot function "Disable Trees" (table 0xDD28A0, entry 12 byte... |
| `0x6D7660` | `cutscene_shotfn_city_no_render_al` | C | cdecl (shot function record*; params from +8), shot function "City No Render AL" (table 0xDD28A0, entry 12 ... |
| `0x6D7670` | `cutscene_shotfn_shader_lod_force_highest` | C | cdecl (shot function record*; params from +8), shot function "Shader LOD Force Highest" (table 0xDD28A0, en... |
| `0x6D76B0` | `cutscene_shotfn_city_shadow_dist` | C | cdecl (shot function record*; params from +8), shot function "City Shadow Dist" (table 0xDD28A0, entry 12 b... |
| `0x6D76C0` | `cutscene_shotfn_shader_lod_restore` | C | cdecl (shot function record*; params from +8), shot function "Shader LOD Restore" (table 0xDD28A0, entry 12... |
| `0x6D76D0` | `cutscene_shotfn_spawning_remove_all` | C | cdecl (shot function record*; params from +8), shot function "Spawning Remove All" (table 0xDD28A0, entry 1... |
| `0x6D7720` | `cutscene_shotfn_far_clip` | C | cdecl (shot function record*; params from +8), shot function "Far Clip" (table 0xDD28A0, entry 12 bytes {na... |
| `0x6D7740` | `cutscene_shotfn_near_clip_danger` | C | cdecl (shot function record*; params from +8), shot function "Near Clip (DANGER)" (table 0xDD28A0, entry 12... |
| `0x6D7750` | `cutscene_shotfn_shot_fade_in` | C | cdecl (shot function record*; params from +8), shot function "Shot Fade In:" (table 0xDD28A0, entry 12 byte... |
| `0x6D77A0` | `cutscene_shotfn_shot_fade_out` | C | cdecl (shot function record*; params from +8), shot function "Shot Fade Out:" (table 0xDD28A0, entry 12 byt... |
| `0x6D77E0` | `cutscene_shotfn_shot_fade_out_midway` | C | cdecl (shot function record*; params from +8), shot function "Shot Fade Out Midway:" (table 0xDD28A0, entry... |
| `0x6D7840` | `cutscene_shotfn_shot_cut_out_midway` | C | cdecl (shot function record*; params from +8), shot function "Shot Cut Out Midway:" (table 0xDD28A0, entry ... |
| `0x6D7890` | `cutscene_shotfn_letterbox_fade_in` | C | cdecl (shot function record*; params from +8), shot function "Letterbox Fade In" (table 0xDD28A0, entry 12 ... |
| `0x6D78D0` | `cutscene_shotfn_letterbox_fade_out` | C | cdecl (shot function record*; params from +8), shot function "Letterbox Fade Out" (table 0xDD28A0, entry 12... |
| `0x6D78F0` | `cutscene_shotfn_framebuffer_tint` | C | cdecl (shot function record*; params from +8), shot function "Framebuffer Tint" (table 0xDD28A0, entry 12 b... |
| `0x6D79E0` | `cutscene_shotfn_force_tod_lights` | C | cdecl (shot function record*; params from +8), shot function "Force TOD Lights" (table 0xDD28A0, entry 12 b... |
| `0x6D7A00` | `cutscene_shotfn_vint_shot_function` | C | cdecl (shot function record*; params from +8), shot function "Vint Shot Function" (table 0xDD28A0, entry 12... |
| `0x6D7A60` | `cutscene_shotfn_hide_player_compnt` | C | cdecl (shot function record*; params from +8), shot function "Hide Player Compnt." (table 0xDD28A0, entry 1... |
| `0x6D7AA0` | `cutscene_shotfn_bink_play` | C | cdecl (shot function record*; params from +8), shot function "Bink Play" (table 0xDD28A0, entry 12 bytes {n... |
| `0x6D7AF0` | `cutscene_shotfn_mesh_mover_hide` | C | cdecl (shot function record*; params from +8), shot function "Mesh Mover Hide" (table 0xDD28A0, entry 12 by... |
| `0x6D7B10` | `cutscene_shotfn_mesh_mover_show` | C | cdecl (shot function record*; params from +8), shot function "Mesh Mover Show" (table 0xDD28A0, entry 12 by... |
| `0x6D7C20` | `cutscene_shotfn_level_object_hide` | C | cdecl (shot function record*; params from +8), shot function "Level Object Hide" (table 0xDD28A0, entry 12 ... |
| `0x6D7C80` | `cutscene_shotfn_level_object_deactivate` | C | cdecl (shot function record*; params from +8), shot function "Level Object Deactivate" (table 0xDD28A0, ent... |
| `0x6D7D00` | `cutscene_shotfn_level_object_show` | C | cdecl (shot function record*; params from +8), shot function "Level Object Show" (table 0xDD28A0, entry 12 ... |
| `0x6D7D70` | `cutscene_shotfn_lightning` | C | cdecl (shot function record*; params from +8), shot function "Lightning" (table 0xDD28A0, entry 12 bytes {n... |
| `0x6D7DB0` | `cutscene_shotfn_weather_stage` | C | cdecl (shot function record*; params from +8), shot function "Weather Stage" (table 0xDD28A0, entry 12 byte... |
| `0x6D7DE0` | `cutscene_shotfn_level_effect_hide` | C | cdecl (shot function record*; params from +8), shot function "Level Effect Hide" (table 0xDD28A0, entry 12 ... |
| `0x6D7EC0` | `cutscene_shotfn_effect_hide_type` | C | cdecl (shot function record*; params from +8), shot function "Effect Hide Type" (table 0xDD28A0, entry 12 b... |
| `0x6D7F30` | `cutscene_shotfn_level_effect_show` | C | cdecl (shot function record*; params from +8), shot function "Level Effect Show" (table 0xDD28A0, entry 12 ... |
| `0x6D7FC0` | `cutscene_shotfn_modify_light` | C | cdecl (shot function record*; params from +8), shot function "Modify Light" (table 0xDD28A0, entry 12 bytes... |
| `0x6D8120` | `cutscene_shotfn_glares` | C | cdecl (shot function record*; params from +8), shot function "Glares" (table 0xDD28A0, entry 12 bytes {name... |
| `0x6D8140` | `cutscene_shotfn_time_of_day_set` | C | cdecl (shot function record*; params from +8), shot function "Time Of Day Set" (table 0xDD28A0, entry 12 by... |
| `0x6D8160` | `cutscene_shotfn_player_outfit` | C | cdecl (shot function record*; params from +8), shot function "Player Outfit" (table 0xDD28A0, entry 12 byte... |
| `0x6D8170` | `cutscene_shotfn_fog_distance` | C | cdecl (shot function record*; params from +8), shot function "Fog Distance" (table 0xDD28A0, entry 12 bytes... |
| `0x6D81E0` | `cutscene_shotfn_city_region_blend` | C | cdecl (shot function record*; params from +8), shot function "City Region Blend" (table 0xDD28A0, entry 12 ... |
| `0x6D81F0` | `cutscene_shotfn_city_render_fade_dist_scale` | C | cdecl (shot function record*; params from +8), shot function "City Render Fade Dist Scale" (table 0xDD28A0,... |
| `0x6D8200` | `cutscene_shotfn_drug_effect` | C | cdecl (shot function record*; params from +8), shot function "Drug Effect" (table 0xDD28A0, entry 12 bytes ... |
| `0x6D8230` | `cutscene_shotfn_debug_console` | C | cdecl (shot function record*; params from +8), shot function "DEBUG Console" (table 0xDD28A0, entry 12 byte... |
| `0x6D8240` | `cutscene_shotfn_rain_override` | C | cdecl (shot function record*; params from +8), shot function "Rain Override" (table 0xDD28A0, entry 12 byte... |
| `0x6D8280` | `cutscene_shotfn_chunk_hide` | C | cdecl (shot function record*; params from +8), shot function "Chunk Hide" (table 0xDD28A0, entry 12 bytes {... |
| `0x6D82B0` | `cutscene_shotfn_chunk_show` | C | cdecl (shot function record*; params from +8), shot function "Chunk Show" (table 0xDD28A0, entry 12 bytes {... |
| `0x6D82D0` | `cutscene_shotfn_credits_roll` | C | cdecl (shot function record*; params from +8), shot function "Credits Roll" (table 0xDD28A0, entry 12 bytes... |
| `0x6D82F0` | `cutscene_shotfn_japan_exit` | C | cdecl (shot function record*; params from +8), shot function "Japan Exit" (table 0xDD28A0, entry 12 bytes {... |

### data (27)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x495AC0` | `camera_free_xtbl_load` | C | no args: xtbl_load("camera_free.xtbl") (EAX = name, ECX = 0), finds the "Camera" element and calls camera_f... |
| `0x496110` | `camera_free_read_stationary_times` | C | EDI = xml element (pushed per call): xtbl_get_float(ECX = name, ESI = &dest) for default_time / stationary_... |
| `0x4961A0` | `camera_free_read_default_swing_rate` | C | EAX = xml element: xtbl_get_float(ECX = "default_swing_rate", ESI = 0xE999DC) (no direct caller: reached th... |
| `0x4961C0` | `camera_free_read_default_aggression` | C | EAX = xml element: xtbl_get_float(ECX = "default_aggression", ESI = dest in the 0xE999xx block) (no direct ... |
| `0x6CDBC0` | `cs_shot_parse_interp_mode` | C | EDI = string: maps "Linear" / "Soft Param" / "Hard Param" / "Accel" to a mode (stricmp chain) |
| `0x6CDD00` | `cs_shot_parse_interp_mode_b` | C | ESI = string: same set with different order ("Linear", "Accel", "Soft Param", "Hard Param") |
| `0x6CEE30` | `cs_shot_parse_dof_values` | C | ECX = dest: xtbl_get_float of MaxCOCRadius, PreblurRadius, Near, IdealNear, IdealFar, Far (xml_find_child p... |
| `0x6CFC70` | `cs_camera_list_parse_character` | C | ECX = xml element: one Characters child: Name, Player (#Player# substitution), CharMesh, Variant, Morph (st... |
| `0x6D56B0` | `cutscene_group_xtbl_load` | C | no args: xtbl_load of the cutscene group file ("%s%s" + ".xtbl"), finds the "Cutscene" element and reads CT... |
| `0x6D6A30` | `cs_camera_list_count_lookats` | C | xml element: xml_count_children("LookAt") + vector_constructor_iterator; caller cutscene_parse_characters |
| `0x6D6BE0` | `cs_camera_list_count_shots` | C | xml element: xml_count_children("Shot") + vector_constructor_iterator; caller cs_camera_list_parse_shots |
| `0x6D6C70` | `cs_shot_parse_tween_uint` | C | ECX = dest: parses AmTweened / Value / StartValue / EndValue / StartFrame / EndFrame as integers (xtbl_get_... |
| `0x6D6DA0` | `cs_shot_parse_tween_float` | C | ECX = dest: same fields as float tween (xtbl_get_float), interpolation name 0x6D7100 |
| `0x6D6EB0` | `cs_shot_parse_dof_tween` | C | xml element: DOF tween: AmTweened / Value / StartValue / EndValue / StartFrame; values through 0x6CEE30 (Ma... |
| `0xAE5C50` | `vehicle_camera_angle_parse` | C | EAX = xml element, stack args caller-cleaned and not mapped (dest record*, flag): reads Lookat_Offset, Foll... |
| `0xAE5F20` | `vehicle_camera_angles_parse` | C | EDI = xml element, stack args caller-cleaned and not mapped (u8 flag, record*): children Primary_Camera_Ang... |
| `0xAE5FC0` | `vehicle_camera_swing_parse` | C | EAX = swing index (28-byte records: record+0x454 or the defaults at 0x312DD0C), stack (record*, xml element... |
| `0xAE60F0` | `vehicle_camera_swings_parse` | C | EAX = xml element, ECX = vehicle record* (0 = defaults), stack (u8 flag) caller-cleaned: Camera_Swings with... |
| `0xAE6200` | `vehicle_camera_helicopter_parse` | C | EAX = xml element, stack args caller-cleaned: Helicopter_Specific: Ascention_Multiplier, Ascention_Max, Asc... |
| `0xAE6300` | `vehicle_camera_seat_data_parse` | C | EAX = xml element, EBX = dest record*: Seat_Specific_Datas / Seat_Specific_Data: Seat (string_hash_lower), ... |
| `0xAE6450` | `vehicle_camera_sticky_parse` | C | EAX = xml element, ECX = dest record*, stack args caller-cleaned: Camera_Sticky: Stopped_Time, Default_Time... |
| `0xAE6510` | `vehicle_info_parse_watercraft_defaults` | C | EAX = xml element: Watercraft_Defaults child -> 0xAE7AC0 (record parse); called only from vehicle_info_pars... |
| `0xAED2D0` | `vehicle_camera_record_parse` | C | cdecl, caller-cleaned stack args |
| `0xAED900` | `vehicle_group_camera_parse` | C | cdecl (xml element) |
| `0xAEDA10` | `vehicle_cameras_parse_vehicle_child` | I | EDI = xml element: finds the Vehicle child and passes it to vehicle_info_parse_record; used by the *_veh.xt... |
| `0xAEDAC0` | `vehicle_camera_vehicle_parse` | C | cdecl (xml element) |
| `0xAEE5F0` | `vehicle_xtbl_tables_load` | C | no args, cdecl: loads vehicle_defaults.xtbl (Defaults), vehicle_groups.xtbl (Vehicle_Group), vehicle_group_... |

### anim (27)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x6E91C0` | `anim_pc_load_virtual` | I | ECX = path: .anim_pc loader using virtual_mempool::alloc and file_get_size, registers via 0xCEC2A0 (no dire... |
| `0x6E9A10` | `anim_states_load_anim_pc` | C | ECX = path: opens <state>.anim_pc (path_change_extension, file_open / file_read, static_mempool_base::alloc... |
| `0x6E9C70` | `anim_pc_load_and_register` | C | stack (path): opens .anim_pc, reads it, registers through 0xCEB4A0 / 0xCEB500 / 0xCEC370 (0x6EF270 muzzle_f... |
| `0x6EC430` | `anim_states_parse_iks` | C | EAX = xml element: IKs block (Enable True/False, Location, Situation, Frame; string_hash_lower, xtbl_get_in... |
| `0x6EC6E0` | `anim_states_parse_props` | C | ECX = xml element: OldProps / Props / Prop (Name, scr_%s script names, Set, StartFrame; effect_find_by_name... |
| `0x6ECBA0` | `anim_states_parse_sounds` | C | EAX = xml element: Sounds / Sound (Frame, Looping, Foley; string_hash_lower, xtbl_get_int / bool); called b... |
| `0x6ECD50` | `anim_states_parse_effects` | C | xml element: Effects / Effect (FXName, Props / OldProps); allocates records from a static mempool; called b... |
| `0x6EEE20` | `human_anim_layer_find` | C | ECX = human obj*, ESI = anim state index: returns EAX = layer index 0..2 (layers of 0x34 bytes at obj+0xC8 ... |
| `0x6EF510` | `anim_state_find_by_name` | C | stack (name or 0) cdecl: returns the index of the anim state called name in the table of 0x2A4 entries (12 ... |
| `0x6EF580` | `anim_state_find_by_name_b` | C | stack (name) cdecl: same as anim_state_find_by_name over the second table of 0x539 entries (12 bytes at 0xE... |
| `0x6EF620` | `anim_track_lookup_two_sets` | C | stack (anim set record*, state, flag) cdecl: tries the two anim set slots at +4/+8, calling 0x6EA0A0; retur... |
| `0x6EF9C0` | `human_has_playing_layer_track` | C | stack (human obj*, ?, state) cdecl: AL = 1 if any of the 3 layers (0x34 bytes from +0xCC) with flag bit 0 a... |
| `0x6EFA80` | `human_has_active_layer_track` | C | stack (human obj*, ...) cdecl: like 0x6EF9C0 but via anim_track_is_playing (0xCECB80); callers camera_follo... |
| `0xB160F0` | `vehicle_interaction_anim_tables_load` | C | no args: loads vi_enter.xtbl, vi_exit.xtbl, vi_ride.xtbl (xtbl_load_vehicle_interaction_point_sets, xtbl_lo... |
| `0xCE0E00` | `rig_find_bone_by_name` | C | stdcall (rig obj*, name) ret 8: _stricmp over the bone array (rig def +0x18 -> +0x3C, 0x3C-byte records, co... |
| `0xCE0E60` | `rig_find_tag_by_name` | C | stdcall (rig obj*, name) ret 8: _stricmp over the tag array (rig def +0x18 -> +0x38, count +0x24) -> index ... |
| `0xCE10F0` | `rig_bone_local_transform` | C | EAX = &out mat33 or 0, stack (bone index, &out vec3) ret 8: reads the bone matrix (0xCE0FF0), writes the tr... |
| `0xCE1180` | `rig_bone_transform_impl` | I | ECX = rig obj*, EAX = bone index (-1 = nothing), stack (&out vec3, &base mat33, &out mat33 or 0) ret 0x10: ... |
| `0xCE3610` | `anim_track_sample_vec3` | I | ECX = track object* (count +0x298, +0x2A0 / +0x2A4 / +0x2A8 data), stack arg = &out vec3: samples a vec3 fr... |
| `0xCE71B0` | `rig_track_index_by_handle` | I | EAX = track handle, EDI = rig obj*: scan of obj+0x254 (0x58 stride, count obj+0x23C) for id == handle>>4; r... |
| `0xCE71F0` | `rig_track_find_by_handle` | C | EAX = track handle (-1 -> 0), EDI = rig obj*: returns EAX = &track (0x58-byte records from obj+0x254, count... |
| `0xCE7520` | `anim_tracks_update_frame` | I | stdcall with 5 stack args (ret 0x14): per-frame update of a rig object's track list (obj +0x244/+0x248 time... |
| `0xCEB7D0` | `rig_handle_find_tag_by_name` | C | ECX = rig handle (<0 -> -1; slot = handle % 0xBE, obj = 0x340D240 + slot*0x5E8), stack (name) cdecl: calls ... |
| `0xCEB8B0` | `rig_handle_find_bone_by_name` | C | ECX = rig handle, stack (name) cdecl: calls rig_find_bone_by_name (0xCE0E00); returns the bone index or -1;... |
| `0xCECBC0` | `rig_handle_track_flag_b` | C | ECX = rig handle (<0 -> AL=0), stack (track group, ...) cdecl: thunk to 0xCE6AF0 (tests per-track flag bit ... |
| `0xCED630` | `rig_handle_track_find` | I | rig handle thunk of rig_track_find_by_handle (0xCE71F0); 33 callers |
| `0xCEDEC0` | `rig_handle_bone_local_transform` | C | ECX = rig handle: thunk to rig_bone_local_transform (0xCE10F0); 14 callers |

### net (5)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x6BE9F0` | `cutscn_pkt_send_sub_1` | C | no args: builds packet 0x6F (byte 0x6F + sub-command byte 1, id 3/4 byte header depending on 0xE8BC74) and ... |
| `0x6BEA60` | `cutscn_pkt_send_sub_2` | C | no args: as cutscn_pkt_send_sub_1 with sub-command 2 |
| `0x6BEAD0` | `cutscn_pkt_send_sub_3` | C | no args: as cutscn_pkt_send_sub_1 with sub-command 3 |
| `0x6BEB40` | `cutscn_pkt_send_sub_4` | C | no args: as cutscn_pkt_send_sub_1 with sub-command 4 |
| `0x6BEBB0` | `cutscn_pkt_send_sub_5` | C | ECX = &obj (floats +0x18/+0x1C/+0x20), EDX = &vec3: packet 0x6F sub-command 5 carrying the vec3 and the thr... |

### math (3)

| Address | Name | Conf | Note (first words) |
|---|---|---|---|
| `0x4958F0` | `crt_unary_math_wrapper_da082c` | I | stack (f32) -> ST0 = 0xDA082C(x) (CRT dispatcher with table 0xE817FA, SSE2 path 0xDA1D50); 12 callers |
| `0x49EE60` | `fabsf_wrapper` | C | stack (f32) -> ST0 = /x/ (fld; fabs; fstp/fld through a stack slot); 204 callers |
| `0x4A1A50` | `angle_wrap_in_place` | C | ECX = &f32 (radians): while v > pi subtract 2*pi, while v < -pi add 2*pi (constants 0xE5C160 / 0xE5BF78 / 0... |


## Large functions: port maps

Section purposes below are inferred from callee names; the calls, addresses, tables and compare polarities are exact. The ten largest functions in this area that can be ported whole, by true size. All are deterministic code over game globals, so they can be hooked at the entry address; none uses a jump table that needs special handling beyond the listed tables. Order is by size.

| # | Address | Name | Size | Convention | Portability |
|---|---|---|---|---|---|
| 1 | 0x6C44C0 | `cutscene_stop` | 0x2200 | cdecl, no args, void; frame callback | medium: 114 callees, 25 virtual calls, 3 jump tables, 0xCC4 locals |
| 2 | 0x6BFB50 | `cutscene_begin_callback` | 0x1AF5 | cdecl, no args, void; frame callback | medium: 129 callees, 45 virtual calls, 1 jump table |
| 3 | 0x6C8130 | `cutscene_shot_start` | 0xA83 | cdecl, no args | medium: 47 callees, 7 virtual calls |
| 4 | 0x6C24E0 | `cutscene_shot_place_characters` | 0xA03 | cdecl, stack arg 1 = shot record* (ESI after load) | medium: 32 callees, 26 virtual calls |
| 5 | 0x49F320 | `camera_script_build_track` | 0x976 | EAX = description, EDI = count; no result | easy to port: only x87, 4 callees |
| 6 | 0xCE3610 | `anim_track_sample_vec3` | 0x805 | ECX = object, stack &out vec3 | medium: 205 x87 ops, sin |
| 7 | 0xCE7520 | `anim_tracks_update_frame` | 0x7E0 | stdcall 5 stack args (ret 0x14) | hard: 128 blocks, 226 x87 ops, purpose only inferred |
| 8 | 0xAED2D0 | `vehicle_camera_record_parse` | 0x626 | cdecl (vehicle record*, xml element, u8 flag) | easy: straight parser, 10 callees |
| 9 | 0x497220 | `camera_follow_vehicle_chase_update` | 0x558 | EDX in | medium: 269 x87 ops, two `pow` calls |
| 10 | 0x4A01B0 | `camera_shake_apply` | 0x546 | no args | medium: 164 x87 ops, rng, timers |

For the two biggest only a *digest* is printed (every call with its register / stack setup, switch tables, writes to the cutscene globals, the returns). Their full block maps are about 2100 and 1500 lines and are left out of this document.


### cutscene_stop — 0x6C44C0

**Role.** Tears the cutscene down. Callers: `gameplay_exit`, the debug menu `0x6BD950`, the begin callback `0x6BFB50`, `0x6C6B90` (called from `0x69A090` and `0x8CB260`); `0x6C4470` installs it as the frame callback `0x2527D18` after a fade to black. Guard: returns at once unless `[0x2527D14]` (playing) and `*0x2527D10` (player) are non-zero; then `[0x2527D15] = 1`.

**Sections, in order** (addresses from the digest): (1) prologue, stack cookie, one indirect call through `[0xDB8114]` with `&local` (a timing import); (2) `gamewide_func(1)`; (3) stop the cutscene voices `0xEAB868 / 0xEAB844 / 0xEAB850 / 0xEAB85C` with `audio_voice_stop` and `audio_voices_stop_category` twice, clear `[0x2527D22]`; (4) per-character loop over the list at `player+0xAC8`: `0x6F0660`, rig release `0xCE89A0 / 0xCE8A00`, `human_anim_set_update(-1, 0, 1, 0)`, script follower look-ups (`script_follower_index`, `script_follower_get`, `string_crc32`), virtual calls; (5) per-vehicle loop over `player+0xAE0`; (6) streaming: `render_thread_wait_frame(0)`, `0x4AB5D0`, `0x4A9DA0(-1)`, `0xA7C4F0`; (7) restore the player (`0x9A0700`, `human_set_force_flag_invulnerable(0)`, `0x963090`, `0x95BE10`, `0x9A5AD0`, `chunk_point_in_volume_flag8`, **`camera_request_context 0x49D400`**, customisation `0x7C3980 / 0x7C26A0`, inventory `0x559D40 / 0x55B600`); (8) a packet builder that ends in `net_send_packet` and a **switch on a 12-entry table at `0x6C66C0`** plus 2-entry tables at `0x6C66E8` and `0x6C6708` (weapon slot restore: `0x55AE30`, `0x55B050`, `0x55B310`, `vehicle_interaction_rider_find`, `weapon_usable_from_vehicle`); (9) camera: `camera_smooth_blend_begin(0x4A33F0)`, **`camera_mode_pop(5)` via EDX = 5** (`0x493E30`), `0x4AA7C0`; (10) the interface document is unloaded (`vint_document_run_unload_script`, `_release_pending`, `_unlink`), `.peg_pc` path (`path_change_extension`, `0xC08D40`), `gfx_device_acquire`, indirect calls `[0xDB8080]`, `[0xDB80D0]`; (11) final flag writes: `[0x2527D14] = 0`, `[0x2527CF7] = 0`, `[0x2527D1D] = 0`, voice `0xEAC15C` stop.

**Hazards.** Many register-argument callees (see `regs[...]` in the digest); the three jump tables are bounded by the `cmp reg, N / ja` immediately before each `jmp`; the stack cookie `0xE80B50` check must be kept or dropped consistently. Port it in sections with the original as fallback for any unmapped callee.


**Digest (calls, tables, absolute reads / writes, key branches):**

```
FUNCTION 0x006C44C0  size 0x2200  insns 2363  blocks 580
  entry regs read before write: []
    6c44eb: CALL indirect [0xdb8114]  ecx=? stack[&[esp+0x40]]
    6c44f8: if (i32[0x2527d14] == 0x0) goto B579  else fallthrough   ; je 0x6c66ab
    6c4505: if (i32[0x2527d10] == 0x0) goto B579  else fallthrough   ; je 0x6c66ab
    6c450d: WRITE [0x2527d15] mov= 0x1   ; byte ptr [0x2527d15], 1
    6c4514: CALL 0x69ab80 gamewide_func  regs[]  stack[0x1]
    6c453c: CALL 0x6c1ef0 sub_6c1ef0  regs[edi=(edi add [esp+0x10])]  stack[0x1]
    6c4565: CALL 0x47feb0 audio_voice_stop  regs[ecx=0xeab868]  stack[]
    6c456f: CALL 0x47feb0 audio_voice_stop  regs[ecx=0xeab844]  stack[]
    6c4579: CALL 0x47feb0 audio_voice_stop  regs[ecx=0xeab850]  stack[]
    6c4583: CALL 0x47feb0 audio_voice_stop  regs[ecx=0xeab85c]  stack[]
    6c458d: CALL 0x47f200 audio_voices_stop_category  regs[edi=?, edx=0x1]  stack[]
    6c459e: CALL 0x47f200 audio_voices_stop_category  regs[edi=?, edx=(edx or 0xffffffff)]  stack[]
    6c45ce: WRITE [0x2527d22] mov= 0x0   ; byte ptr [0x2527d22], 0
    6c45fc: CALL 0x6cd9a0 sub_6cd9a0  regs[edi=(edi add [esp+0x10])]  stack[0x1]
    6c4643: CALL 0x6f0660 sub_6f0660  regs[]  stack[ecx, 0xffffffff]
    6c467b: CALL 0xce89a0 sub_ce89a0  regs[]  stack[ecx]
    6c46b2: CALL 0xce8a00 sub_ce8a00  regs[]  stack[ecx, 0x0]
    6c46e1: CALL 0x96cb70 human_anim_set_update  regs[]  stack[(eax or 0xffffffff), 0x0, 0x1, 0x0]
    6c4733: CALL 0xa52bb0 script_find_object_by_name  regs[ecx=esi]  stack[]
    6c4741: CALL 0xa275f0 script_follower_index  regs[eax=esi]  stack[]
    6c474b: CALL 0xa27660 script_follower_get  regs[]  stack[eax]
    6c475c: CALL 0xbdca30 string_crc32  regs[eax=&[esp+0x40]]  stack[]
    6c479c: CALL indirect eax  ecx=? stack[]
    6c47d6: CALL 0x6f0660 sub_6f0660  regs[]  stack[ecx, 0xffffffff]
    6c480e: CALL 0xce89a0 sub_ce89a0  regs[]  stack[ecx]
    6c4845: CALL 0xce8a00 sub_ce8a00  regs[]  stack[ecx, 0x0]
    6c4874: CALL 0x96cb70 human_anim_set_update  regs[]  stack[(eax or 0xffffffff), 0x0, 0x1, 0x0]
    6c48be: CALL indirect eax  ecx=i32[ecx+edi*4] stack[]
    6c48f2: CALL 0x6f0660 sub_6f0660  regs[]  stack[ecx, 0xffffffff]
    6c492a: CALL 0xce89a0 sub_ce89a0  regs[]  stack[ecx]
    6c4961: CALL 0xce8a00 sub_ce8a00  regs[]  stack[ecx, 0x0]
    6c4990: CALL 0x522b80 render_thread_wait_frame  regs[]  stack[0x0]
    6c49ba: CALL 0x4ab5d0 sub_4ab5d0  regs[]  stack[i32[edi+0x14]]
    6c49c9: CALL 0x4a9da0 sub_4a9da0  regs[edi=?]  stack[0xffffffff]
    6c4a2a: CALL 0x4ab5d0 sub_4ab5d0  regs[]  stack[i32[eax+esi*8]]
    6c4a3f: CALL 0xa7c4f0 sub_a7c4f0  regs[]  stack[&[esp+0x28], 0x1, 0x0]
    6c4ab0: CALL 0x95f090 sub_95f090  regs[eax=ecx]  stack[edx, 0x0, 0xffffffff, 0x0]
    6c4ae2: WRITE [0x145a2a0] mov= esi   ; dword ptr [0x145a2a0], esi
    6c4b46: CALL 0x6bf9f0 sub_6bf9f0  regs[]  stack[]
    6c4b9c: CALL 0xce89a0 sub_ce89a0  regs[]  stack[ecx]
    6c4bd8: CALL 0xce8a00 sub_ce8a00  regs[]  stack[ecx]
    6c4be3: CALL 0x946790 sub_946790  regs[]  stack[0x0, 0x0, 0x0]
    6c4bf1: CALL 0x9e0ff0 net_send_prop_player_script_mode_params  regs[ecx=edi]  stack[0x0, 0x0]
    6c4bff: CALL 0x9a0700 sub_9a0700  regs[]  stack[&[edi+0x14], 0x1, 0x0]
    6c4c0c: CALL 0x926860 misc_packet_send_14  regs[ecx=edi]  stack[0x0, 0x1, 0x0]
    6c4c17: CALL 0x965f40 human_set_force_flag_invulnerable  regs[]  stack[0x0]
    6c4c22: CALL 0x963090 sub_963090  regs[eax=edi]  stack[0x1, 0x0]
    6c4c46: CALL 0x95be10 sub_95be10  regs[esi=&[edi+0x106c]]  stack[ret(sub_963090)]
    6c4c5b: CALL 0x9a5ad0 sub_9a5ad0  regs[]  stack[]
    6c4c81: CALL 0x4ab430 chunk_point_in_volume_flag8  regs[]  stack[]
    6c4c92: CALL 0x49d400 sub_49d400  regs[edi=?]  stack[]
    6c4cb0: CALL 0x7c3980 sub_7c3980  regs[eax=i32[edi+0x13b0]]  stack[]
    6c4cce: CALL 0x7c26a0 sub_7c26a0  regs[]  stack[i32[edi+0x13b0], (eax add 0x2398), i32[eax+0x4398]]
    6c4ce4: CALL 0x559d40 human_pkt_send_3e  regs[ecx=(edi add 0x10c4)]  stack[0x0]
    6c4cf5: CALL 0x55b600 sub_55b600  regs[ebx=?]  stack[(edi add 0x10c4), (eax add 0x439c)]
    6c4e8b: CALL 0x814670 net_send_packet  regs[esi=?]  stack[&[esp+0x7bc], ax, 0x0, 0x0]
    6c4ead: SWITCH jump [[eax*4+0x6c66c0]] table 0x6c66c0 12 entries: 0->B130, 1->B137, 2->B141, 3->B139, 4->B142, 5->B148, 6->B149, 7->B136, 8->B150, 9->B151, 10->B147, 11->B139
    6c4ef9: CALL 0x55ae30 sub_55ae30  regs[]  stack[0x1, 0x0, 0x0, 0x0]
    6c4f0d: CALL 0x55b200 sub_55b200  regs[eax=edi]  stack[]
    6c4f24: CALL 0x55ae30 sub_55ae30  regs[]  stack[0x1, 0x0, 0x0, 0x0]
    6c4f59: SWITCH jump [[edx*4+0x6c66e8]] table 0x6c66e8 2 entries: 0->B147, 1->B139
    6c4f6c: CALL 0x55b050 sub_55b050  regs[]  stack[0x0]
    6c4f7d: CALL 0x55b050 sub_55b050  regs[]  stack[0x1]
    6c4f8e: CALL 0x55b310 sub_55b310  regs[]  stack[0x1]
    6c4f9c: CALL 0x55b310 sub_55b310  regs[]  stack[0x0]
    6c5078: CALL 0xb07a30 vehicle_interaction_rider_find  regs[esi=?]  stack[]
    6c508b: CALL 0xb07a30 vehicle_interaction_rider_find  regs[esi=?]  stack[]
    6c5179: CALL 0x960c30 sub_960c30  regs[esi=?]  stack[]
    6c51b4: CALL 0x96cb70 human_anim_set_update  regs[]  stack[0xffffffff, 0x1, 0x1, 0x0]
    6c5276: CALL 0xb6a8b0 weapon_lockon_reticule_update  regs[]  stack[ecx]
    6c530d: CALL 0x9c5140 sub_9c5140  regs[eax=esi]  stack[]
    6c5364: SWITCH jump [[ecx*4+0x6c6708]] table 0x6c6708 2 entries: 0->B292, 1->B245
    6c538f: CALL 0x9c8c20 sub_9c8c20  regs[]  stack[0x1]
    6c53ce: CALL 0xb07a30 vehicle_interaction_rider_find  regs[esi=?]  stack[]
    6c53e1: CALL 0xb07a30 vehicle_interaction_rider_find  regs[esi=?]  stack[]
    6c5458: CALL 0xb72d30 weapon_usable_from_vehicle  regs[eax=?]  stack[edx]
    6c5471: CALL 0x55ae30 sub_55ae30  regs[]  stack[i32[esp+0x10], 0x1, 0x0, 0x0, 0x0]
    6c54fc: CALL indirect eax  ecx=? stack[i32[esi+0x10], eax]
    6c556a: CALL 0x7975e0 sub_7975e0  regs[ecx=esi]  stack[]
    6c5570: CALL 0x798140 sub_798140  regs[]  stack[]
    6c557a: CALL 0x558be0 sub_558be0  regs[esi=edi]  stack[]
    6c558d: CALL 0x559d40 human_pkt_send_3e  regs[ecx=(ecx add 0x439c)]  stack[0x0]
    6c55fb: if (i32[0x145a2a8] == 0) goto B302  else fallthrough   ; je 0x6c561b
    6c5606: CALL 0x87c9a0 sub_87c9a0  regs[esi=?]  stack[]
    6c5611: WRITE [0x145a2a8] mov= 0x0   ; dword ptr [0x145a2a8], 0
    6c561b: CALL 0x9d7bd0 coop_partner_human  regs[]  stack[]
    6c562e: CALL 0x6bf9f0 sub_6bf9f0  regs[]  stack[eax]
    6c5645: CALL 0x4aa7c0 sub_4aa7c0  regs[]  stack[]
    6c568a: CALL 0x4a33f0 sub_4a33f0  regs[]  stack[ecx]
    6c56a5: CALL indirect eax  ecx=? stack[]
    6c56f8: CALL 0x493e30 sub_493e30  regs[edx=0x5]  stack[]
    6c570e: CALL 0xb91400 vint_document_run_unload_script  regs[]  stack[]
    6c5720: CALL indirect eax  ecx=? stack[]
    6c5725: CALL 0xb8a9f0 vint_document_release_pending  regs[esi=?]  stack[]
    6c572c: CALL 0xb8abd0 vint_document_unlink  regs[eax=esi]  stack[]
    6c5746: CALL 0x751960 sub_751960  regs[]  stack[]
    6c578d: CALL 0xbff930 path_change_extension  regs[ecx=?]  stack[&[esp+0x6ac], 0x104, 0xddb0b0 ".peg_pc"]
    6c579c: CALL 0xc08d40 sub_c08d40  regs[ecx=&[esp+0x6a8]]  stack[]
    6c57a1: CALL 0xb874e0 sub_b874e0  regs[]  stack[]
    6c57b4: CALL 0xcfbc00 gfx_device_acquire  regs[]  stack[]
    6c57b9: CALL 0x6888d0 sub_6888d0  regs[]  stack[]
    6c57be: CALL indirect [0xdb8080]  ecx=ret(sub_6888d0) stack[]
    6c57e4: CALL indirect [0xdb80d0]  ecx=? stack[0x33d62d0]
    6c57fe: CALL 0x47feb0 audio_voice_stop  regs[ecx=0xeac15c]  stack[]
    6c582f: WRITE [0x2527d14] mov= 0x0   ; byte ptr [0x2527d14], 0
    6c583c: WRITE [0x2527cf7] mov= 0x0   ; byte ptr [0x2527cf7], 0
    6c5849: WRITE [0x2527d1d] mov= 0x0   ; byte ptr [0x2527d1d], 0
    6c5856: WRITE [0x2527d1c] mov= 0x0   ; byte ptr [0x2527d1c], 0
    6c5890: CALL 0x50ab60 effect_object_stop  regs[edi=?]  stack[0x1, 0x0]
    6c58a1: CALL indirect eax  ecx=edi stack[0x0]
    6c58aa: CALL indirect eax  ecx=edi stack[]
    6c58bf: CALL 0x6cdef0 sub_6cdef0  regs[]  stack[]
    6c58ea: WRITE [0xe94d04] mov= 0xffff   ; dword ptr [0xe94d04], 0xffff
    6c58f4: WRITE [0x2527d25] mov= 0x0   ; byte ptr [0x2527d25], 0
    6c594a: CALL 0xaacc60 sub_aacc60  regs[]  stack[edx, 0x1]
    6c5956: CALL 0x926860 misc_packet_send_14  regs[ecx=?]  stack[0x1, 0x1]
    6c597c: CALL 0x4c96e0 sub_4c96e0  regs[]  stack[eax]
    6c598b: CALL 0x4c99d0 sub_4c99d0  regs[eax=i32[ebx+0xaf0]]  stack[]
    6c59a1: CALL 0x79d7e0 sub_79d7e0  regs[]  stack[0]
    6c59e0: CALL indirect edx  ecx=i32[edx+edi*4] stack[]
    6c5a4f: CALL 0xa79240 rsched_release_net  regs[edx=ebx]  stack[0x6, 0x8, 0x1]
    6c5a6c: CALL indirect eax  ecx=? stack[]
    6c5a73: CALL 0xa79430 sub_a79430  regs[edx=ebx]  stack[]
    6c5a7f: CALL indirect eax  ecx=esi stack[]
    6c5af0: CALL indirect edx  ecx=i32[edx+edi*4] stack[]
    6c5b01: CALL indirect edx  ecx=esi stack[]
    6c5b66: CALL 0xa79240 rsched_release_net  regs[edx=ebx]  stack[0x6, 0x0, 0x1]
    6c5b83: CALL indirect eax  ecx=? stack[]
    6c5b8a: CALL 0xa79430 sub_a79430  regs[edx=ebx]  stack[]
    6c5b96: CALL indirect eax  ecx=esi stack[]
    6c5c03: CALL 0xa579e0 sub_a579e0  regs[ebx=?]  stack[0x0, 0x0]
    6c5cb7: CALL indirect eax  ecx=ebx stack[]
    6c5cc1: CALL 0xaa4240 sub_aa4240  regs[esi=eax]  stack[0x5]
    6c5ce7: CALL indirect eax  ecx=ebx stack[]
    6c5cf4: CALL 0x9ced90 sub_9ced90  regs[edi=0x5, esi=eax]  stack[]
    6c5d26: CALL 0x6d5340 sub_6d5340  regs[]  stack[(ecx add edi)]
    6c5d3b: CALL 0x924fe0 objects_deactivate_all  regs[]  stack[]
    6c5d4f: CALL 0x6cd5f0 sub_6cd5f0  regs[]  stack[]
    6c5d54: CALL 0x6d4ed0 sub_6d4ed0  regs[]  stack[]
    6c5d5f: CALL 0x946ac0 sub_946ac0  regs[]  stack[]
    6c5d64: CALL 0x9473b0 sub_9473b0  regs[]  stack[]
    6c5dcb: CALL 0x50e6b0 sub_50e6b0  regs[eax=i32[eax+0x18]]  stack[i32[eax+edi+0x24], 0x0]
    6c5e10: CALL 0x6d5570 sub_6d5570  regs[]  stack[]
    6c5e1a: CALL 0xa57730 group_find_by_name  regs[]  stack[]
    6c5e2c: CALL indirect eax  ecx=eax stack[]
    6c5e66: CALL 0x924fe0 objects_deactivate_all  regs[]  stack[]
    6c5e8c: WRITE [0x2526dc4] mov= 0x1   ; byte ptr [0x2526dc4], 1
    6c5ea0: CALL 0x6a7ce0 sub_6a7ce0  regs[]  stack[]
    6c5ea8: CALL 0x603f80 sub_603f80  regs[]  stack[]
    6c5ebe: CALL indirect eax  ecx=i32[0x140b1c4] stack[0x1]
    6c5ec0: CALL 0x603f80 sub_603f80  regs[]  stack[]
    6c5ed6: CALL indirect eax  ecx=i32[0x140b170] stack[0x1]
    6c5eda: CALL 0x604540 sub_604540  regs[]  stack[0x1]
    6c5fc6: CALL 0x6887b0 sub_6887b0  regs[]  stack[]
    6c5fd2: WRITE [0x2526cc3] mov= 0x0   ; byte ptr [0x2526cc3], 0
    6c5fd9: WRITE [0x2526cc4] mov= 0x0   ; byte ptr [0x2526cc4], 0
    6c5ff1: WRITE [0x2526cc5] mov= 0x0   ; byte ptr [0x2526cc5], 0
    6c600d: CALL 0x93c660 sub_93c660  regs[]  stack[]
    6c6020: CALL 0xb81130 sub_b81130  regs[]  stack[]
    6c602c: CALL 0xb7da30 sub_b7da30  regs[]  stack[]
    6c6038: if (i32[0x25272ce] != 0x0) goto B470  else fallthrough   ; jne 0x6c6071
    6c6048: CALL 0x93c660 sub_93c660  regs[]  stack[]
    6c6054: CALL 0xb7da30 sub_b7da30  regs[]  stack[]
    6c6060: CALL 0xb81130 sub_b81130  regs[]  stack[]
    6c6069: CALL 0x93d310 sub_93d310  regs[esi=0]  stack[]
    6c6087: WRITE [0x2526e14] mov= ebx   ; dword ptr [0x2526e14], ebx
    6c6095: WRITE [0x2527890] mov= 0x0   ; byte ptr [0x2527890], 0
    6c60d8: CALL 0x5455a0 sub_5455a0  regs[ecx=i32[edi+0x1688]]  stack[(eax add ebx), i32[eax+ebx+0x54]]
    6c6123: CALL 0x6d7b30 sub_6d7b30  regs[]  stack[edx]
    6c615c: CALL 0x4a9770 sub_4a9770  regs[]  stack[]
    6c61bb: CALL 0xc99030 __stricmp  regs[]  stack[i32[edx], i32[edi+ebx]]
    6c6235: CALL 0x4ab100 sub_4ab100  regs[ebx=?, ecx=?, esi=i32[edx+eax*4]]  stack[0x1]
    6c62a6: WRITE [0x2526d78] mov= 0x0   ; byte ptr [0x2526d78], 0
    6c62d9: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c6301: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c630c: CALL 0x79d920 sub_79d920  regs[]  stack[0x0]
    6c6316: WRITE [0x2527cf4] mov= 0x0   ; byte ptr [0x2527cf4], 0
    6c631d: CALL 0x6bfac0 sub_6bfac0  regs[ebx=?]  stack[0x1]
    6c634b: CALL 0xa58e00 sub_a58e00  regs[eax=?]  stack[]
    6c6350: CALL 0x492df0 camera_cut  regs[]  stack[]
    6c63bf: WRITE [0xe8dd22] mov= 0x1   ; byte ptr [0xe8dd22], 1
    6c63fc: CALL 0x7f4990 sub_7f4990  regs[]  stack[0xe0c660 "voice_script"]
    6c6412: WRITE [0x2527d10] mov= ebx   ; dword ptr [0x2527d10], ebx
    6c6418: CALL indirect eax  ecx=0x2771178 stack[]
    6c643a: CALL indirect eax  ecx=0x2771178 stack[]
    6c6458: CALL 0x6c4050 sub_6c4050  regs[]  stack[]
    6c6464: if (i32[0x2527494] == 0x0) goto B531  else fallthrough   ; je 0x6c646d
    6c6466: WRITE [0x2527494] mov= 0x0   ; byte ptr [0x2527494], 0
    6c646d: CALL 0x7f7ad0 coop_session_active  regs[]  stack[]
    6c649e: CALL 0x6be6f0 sub_6be6f0  regs[]  stack[]
    6c64a7: CALL 0x6bead0 sub_6bead0  regs[]  stack[]
    6c64ac: WRITE [0x2527d21] mov= 0x0   ; byte ptr [0x2527d21], 0
    6c64b5: WRITE [0x2527d21] mov= 0x1   ; byte ptr [0x2527d21], 1
    6c64cb: WRITE [0x2527d15] mov= 0x1   ; byte ptr [0x2527d15], 1
    6c64e3: CALL 0x47f080 audio_pause_all  regs[]  stack[]
    6c64e8: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c64f6: CALL 0x4d1270 sub_4d1270  regs[edx=ret(sub_69aed0)]  stack[ret(sub_69aed0)]
    6c6500: WRITE [0x2527d15] mov= 0x0   ; byte ptr [0x2527d15], 0
    6c6536: CALL 0x6c3300 sub_6c3300  regs[]  stack[ecx]
    6c6557: CALL 0x6c3300 sub_6c3300  regs[]  stack[ecx]
    6c6579: if (i32[0x2527d15] != 0x0) goto B551  else fallthrough   ; jne 0x6c6589
    6c657b: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c6581: CALL 0x793d60 sub_793d60  regs[]  stack[]
    6c6590: if (i32[0x2527c5c] > 0 (signed)) goto B553  else fallthrough   ; jg 0x6c659c
    6c6594: CALL 0x6bdba0 sub_6bdba0  regs[]  stack[]
    6c659c: CALL 0x7f7ad0 coop_session_active  regs[]  stack[]
    6c65ad: CALL 0xd14840 gfx_set_color  regs[]  stack[0xff]
    6c65bc: CALL 0x4d1090 screen_fade_start  regs[]  stack[]
    6c6608: if (i32[0x2527d15] != 0x0) goto B566  else fallthrough   ; jne 0x6c661f
    6c6617: CALL 0x4d1270 sub_4d1270  regs[edx=?]  stack[0x4b0, ecx]
    6c663e: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c6669: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c6672: CALL 0x739340 sub_739340  regs[]  stack[]
    6c66a9: CALL indirect edi  ecx=? stack[]
    6c66b7: CALL 0xc9604f __security_check_cookie  regs[ecx=(ecx xor esp)]  stack[]
    6c66bf: RETURN
  call targets: 0x47f080(audio_pause_all), 0x47f200(audio_voices_stop_category), 0x47feb0(audio_voice_stop), 0x492df0(camera_cut), 0x493e30, 0x49d400, 0x4a33f0, 0x4a9770, 0x4a9da0, 0x4aa7c0, 0x4ab100, 0x4ab430(chunk_point_in_volume_flag8), 0x4ab5d0, 0x4c96e0, 0x4c99d0, 0x4d1090(screen_fade_start), 0x4d1270, 0x50ab60(effect_object_stop), 0x50e6b0, 0x522b80(render_thread_wait_frame), 0x5455a0, 0x558be0, 0x559d40(human_pkt_send_3e), 0x55ae30, 0x55b050, 0x55b200, 0x55b310, 0x55b600, 0x603f80, 0x604540, 0x6887b0, 0x6888d0, 0x69ab80(gamewide_func), 0x69aed0, 0x6a7ce0, 0x6bdba0, 0x6be6f0, 0x6bead0, 0x6bf9f0, 0x6bfac0, 0x6c1ef0, 0x6c3300, 0x6c4050, 0x6cd5f0, 0x6cd9a0, 0x6cdef0, 0x6d4ed0, 0x6d5340, 0x6d5570, 0x6d7b30, 0x6f0660, 0x739340, 0x751960, 0x793d60, 0x7975e0, 0x798140, 0x79d1b0, 0x79d7e0, 0x79d920, 0x7c26a0, 0x7c3980, 0x7f4990, 0x7f7ad0(coop_session_active), 0x814670(net_send_packet), 0x87c9a0, 0x924fe0(objects_deactivate_all), 0x926860(misc_packet_send_14), 0x93c660, 0x93d310, 0x946790, 0x946ac0, 0x9473b0, 0x95be10, 0x95f090, 0x960c30, 0x963090, 0x965f40(human_set_force_flag_invulnerable), 0x96cb70(human_anim_set_update), 0x9a0700, 0x9a5ad0, 0x9c5140, 0x9c8c20, 0x9ced90, 0x9d7bd0(coop_partner_human), 0x9e0ff0(net_send_prop_player_script_mode_params), 0xa275f0(script_follower_index), 0xa27660(script_follower_get), 0xa52bb0(script_find_object_by_name), 0xa57730(group_find_by_name), 0xa579e0, 0xa58e00, 0xa79240(rsched_release_net), 0xa79430, 0xa7c4f0, 0xaa4240, 0xaacc60, 0xb07a30(vehicle_interaction_rider_find), 0xb6a8b0(weapon_lockon_reticule_update), 0xb72d30(weapon_usable_from_vehicle), 0xb7da30, 0xb81130, 0xb874e0, 0xb8a9f0(vint_document_release_pending), 0xb8abd0(vint_document_unlink), 0xb91400(vint_document_run_unload_script), 0xbdca30(string_crc32), 0xbff930(path_change_extension), 0xc08d40, 0xc9604f(__security_check_cookie), 0xc99030(__stricmp), 0xce89a0, 0xce8a00, 0xcfbc00(gfx_device_acquire), 0xd14840(gfx_set_color)
  stack objects passed by address: &[esp+0x28] -> sub_a7c4f0; &[esp+0x40] -> string_crc32; &[esp+0x6a8] -> sub_c08d40; &[esp+0x6ac] -> path_change_extension; &[esp+0x7bc] -> net_send_packet
  jump table @0x6c66c0 used by 0x6c4ead: 0:0x6c4eb4, 1:0x6c4ef0, 2:0x6c4f1b, 3:0x6c4f0b, 4:0x6c4f32, 5:0x6c4f69, 6:0x6c4f7a, 7:0x6c4ee3, 8:0x6c4f8b, 9:0x6c4f99, 10:0x6c4f60, 11:0x6c4f0b
  jump table @0x6c66e8 used by 0x6c4f59: 0:0x6c4f60, 1:0x6c4f0b
  jump table @0x6c6708 used by 0x6c5364: 0:0x6c557f, 1:0x6c536b
```

### cutscene_begin_callback — 0x6BFB50

**Role.** The callback `cutscene_play` installs in `0x2527D18`: prepares the world and the player for the first frame of a cutscene. Starts with `render_thread_wait_frame(0)`, `0x739340`, `gamewide_func(1)`, an `__alldiv` on the performance counter, then the player block: `0x988170`, `human_is_dead`, `0x9B04D0(1)`, `0x98CEB0`, `human_set_force_flag_force_holster(1)`, `human_anim_set_update(-1, 1, 1, 0)`, `human_pkt_send_17`, `human_set_force_flag_invulnerable(1)`, `ai_order_k23_start`, `ai_order_k13_exit(0x23)`, `human_state_set(0, 0)`, `memset(...,0,0x300)`, `human_post_update`, `human_ai_pre_think`, `misc_packet_send_14`. Between them: `xtbl_load_cs_camera_list` (0x6D49F0), vehicle interaction query, `ac81e0(0x200)`, an `ftol2` + `memcpy` that copies a name, three groups of indirect calls through `esi / edi / [0xDB81DC] / [0xDB80D0]` with the constant objects `0x33D6404` and `0x33D64D4` (an add / remove pair per object), and the three `cutscn_pkt` senders.

**Hazards.** 45 indirect calls; the objects behind `[0xDB81DC]` / `[0xDB80D0]` (import thunks) and the vtable calls on the player lists must be left to the original. The one jump table is in the digest. Tail of the function (0x6C0550-0x6C1645) is the shot hand-over.


**Digest (calls, tables, absolute reads / writes, key branches):**

```
FUNCTION 0x006BFB50  size 0x1af5  insns 1819  blocks 374
  entry regs read before write: []
    6bfb84: CALL indirect esi  ecx=? stack[&[esp+0x90]]
    6bfb88: WRITE [0x2527cf7] mov= 0x1   ; byte ptr [0x2527cf7], 1
    6bfb8f: CALL 0x522b80 render_thread_wait_frame  regs[]  stack[0x0]
    6bfb97: CALL 0x739340 sub_739340  regs[]  stack[]
    6bfb9e: CALL 0x69ab80 gamewide_func  regs[]  stack[0x1]
    6bfbbe: CALL indirect esi  ecx=&[esp+0x40] stack[&[esp+0x40]]
    6bfbe4: CALL 0xcb2720 __alldiv  regs[]  stack[(eax sub [0x2346508]), i32[esp+0x44], i32[0x23464f8], i32[0x23464fc]]
    6bfc04: WRITE [0x2527cf6] mov= i32[0xe87be0] =i32 16843009   ; byte ptr [0x2527cf6], al
    6bfc13: CALL 0x988170 sub_988170  regs[ebx=i32[0x21703d4]]  stack[i32[0x21703d4]]
    6bfc1c: CALL 0x985630 human_is_dead  regs[]  stack[i32[0x21703d4]]
    6bfc2b: CALL 0x9b04d0 sub_9b04d0  regs[]  stack[0x1]
    6bfc49: CALL 0x98ceb0 sub_98ceb0  regs[edi=i32[ebx+0x44]]  stack[0x1]
    6bfc5d: CALL 0x9acc90 sub_9acc90  regs[]  stack[0x0]
    6bfc65: CALL 0x9acb10 sub_9acb10  regs[]  stack[0x0]
    6bfc6d: CALL 0x9f2100 sub_9f2100  regs[edx=?]  stack[]
    6bfc75: CALL 0x9661e0 human_set_force_flag_force_holster  regs[]  stack[0x1]
    6bfcbc: CALL 0x96cb70 human_anim_set_update  regs[]  stack[0xffffffff, 0x1, 0x1, 0]
    6bfcef: CALL 0x95f320 sub_95f320  regs[esi=ebx]  stack[&[esp+0x80]]
    6bfcf7: CALL 0x782e10 sub_782e10  regs[]  stack[]
    6bfd37: CALL 0x9caf90 human_pkt_send_17  regs[ecx=ebx]  stack[]
    6bfd42: CALL 0x9da290 sub_9da290  regs[ecx=ebx]  stack[]
    6bfd7a: WRITE [0x145a2a0] mov= i32[ebx+0xd74]   ; dword ptr [0x145a2a0], edx
    6bfdbc: CALL 0xaa76f0 sub_aa76f0  regs[ecx=?]  stack[]
    6bfdcd: WRITE [0x145a2a4] mov= ret(sub_aa76f0)   ; dword ptr [0x145a2a4], eax
    6bfe13: CALL 0xb072f0 vehicle_interaction_query  regs[]  stack[&[esp+0x50]]
    6bfe3f: CALL indirect eax  ecx=esi stack[0x1]
    6bfe8b: CALL 0x87c8a0 sub_87c8a0  regs[]  stack[]
    6bfe90: WRITE [0x145a2a8] mov= ret(sub_87c8a0)   ; dword ptr [0x145a2a8], eax
    6bfea3: CALL 0xac81e0 sub_ac81e0  regs[ecx=i32[esi+0x7f9c], edx=&[eax+0x3]]  stack[0x200]
    6bfeba: WRITE [0x145a2a8] mov= edi   ; dword ptr [0x145a2a8], edi
    6bffa7: CALL 0x4912d0 sub_4912d0  regs[]  stack[ecx]
    6bffaf: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    6bffcc: CALL 0xca0850 _memcpy  regs[]  stack[(ecx add 0x8), (esi add 0xd1), ret(_ftol2_sse)]
    6bfff8: CALL 0x6bf200 sub_6bf200  regs[]  stack[]
    6c0014: CALL 0x559d40 human_pkt_send_3e  regs[ecx=(esi add 0x439c)]  stack[0]
    6c0035: CALL 0x55b600 sub_55b600  regs[ebx=?]  stack[(eax add 0x439c), &[ebx+0x10c4]]
    6c0056: CALL 0x965f40 human_set_force_flag_invulnerable  regs[]  stack[0x1]
    6c0146: CALL indirect edx  ecx=? stack[]
    6c0152: CALL 0x926860 misc_packet_send_14  regs[ecx=?]  stack[0x1, 0x1, 0x0]
    6c015f: CALL 0x99f350 sub_99f350  regs[]  stack[]
    6c0168: CALL 0x9d9fd0 sub_9d9fd0  regs[]  stack[]
    6c0172: CALL 0x458580 ai_order_k23_start  regs[esi=ebx]  stack[]
    6c017a: CALL 0x454d70 ai_order_k13_exit  regs[]  stack[0x23]
    6c0184: CALL 0x9954c0 human_state_set  regs[ecx=0, edx=0]  stack[]
    6c01ac: CALL 0x6f0660 sub_6f0660  regs[]  stack[ecx, 0xffffffff]
    6c01e4: CALL 0xce89a0 sub_ce89a0  regs[]  stack[ecx]
    6c021b: CALL 0xce8a00 sub_ce8a00  regs[]  stack[ecx, 0x0]
    6c027e: CALL 0xca0bc0 _memset  regs[]  stack[ecx, 0x0, 0x300]
    6c028f: CALL 0x9a68d0 human_post_update  regs[eax=ebx]  stack[]
    6c02ba: CALL indirect esi  ecx=? stack[0x33d6404]
    6c02cc: CALL indirect esi  ecx=? stack[0x33d6404]
    6c02d4: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6438]]
    6c02e9: CALL indirect edi  ecx=ret(indirect) stack[0x33d6404]
    6c02f7: CALL indirect edi  ecx=? stack[0x33d6404]
    6c0313: CALL indirect esi  ecx=? stack[0x33d64d4]
    6c0325: CALL indirect esi  ecx=? stack[0x33d64d4]
    6c032d: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6508]]
    6c0342: CALL indirect edi  ecx=ret(indirect) stack[0x33d64d4]
    6c0350: CALL indirect edi  ecx=? stack[0x33d64d4]
    6c0367: CALL 0xcedf60 sub_cedf60  regs[esi=?]  stack[]
    6c0394: CALL indirect eax  ecx=? stack[]
    6c03be: CALL indirect eax  ecx=0xec5b98 stack[(esi add 0xfffffff8)]
    6c03ea: CALL indirect esi  ecx=? stack[0x33d6404]
    6c03fc: CALL indirect esi  ecx=? stack[0x33d6404]
    6c0404: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6438]]
    6c0419: CALL indirect [0xdb80d0]  ecx=ret(indirect) stack[0x33d6404]
    6c042b: CALL indirect [0xdb80d0]  ecx=? stack[0x33d6404]
    6c0451: CALL indirect esi  ecx=? stack[0x33d64d4]
    6c0463: CALL indirect esi  ecx=? stack[0x33d64d4]
    6c046b: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6508]]
    6c0480: CALL indirect [0xdb80d0]  ecx=ret(indirect) stack[0x33d64d4]
    6c0492: CALL indirect [0xdb80d0]  ecx=? stack[0x33d64d4]
    6c049a: CALL 0x9a9010 human_ai_pre_think  regs[eax=ebx]  stack[]
    6c04a7: CALL 0x926860 misc_packet_send_14  regs[ecx=ebx]  stack[0x1, 0x1, 0x0]
    6c04e7: CALL 0x9129a0 sub_9129a0  regs[]  stack[eax, 0x0]
    6c0532: CALL 0x926860 misc_packet_send_14  regs[ecx=esi]  stack[0x1, 0x1, 0x0]
    6c057c: CALL 0x9b04d0 sub_9b04d0  regs[]  stack[0x1]
    6c05a8: WRITE [0x2526d24] mov= 0x0   ; dword ptr [0x2526d24], 0
    6c05b2: CALL 0xc14930 sub_c14930  regs[eax=(eax and ecx)]  stack[]
    6c05dd: CALL indirect edx  ecx=? stack[]
    6c05f1: if (i32[0x2527d22] != 0x0) goto B143  else fallthrough   ; jne 0x6c061f
    6c061a: CALL 0x47f200 audio_voices_stop_category  regs[edi=?, edx=(edx or 0xffffffff)]  stack[]
    6c061f: CALL 0x95b2c0 sub_95b2c0  regs[]  stack[]
    6c0624: CALL 0x43f5f0 sub_43f5f0  regs[]  stack[]
    6c0629: CALL 0x483ac0 sub_483ac0  regs[]  stack[]
    6c0630: CALL 0x793d60 sub_793d60  regs[]  stack[0x1]
    6c0648: CALL 0x79ca80 sub_79ca80  regs[]  stack[0x3]
    6c0675: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c06a9: SWITCH jump [[eax*4+0x6c1648]] table 0x6c1648 4 entries: 0->B163, 1->B153, 2->B154, 3->B163
    6c06b0: CALL 0x6ab240 record_input_step  regs[]  stack[]
    6c06d8: CALL 0x6ab380 sub_6ab380  regs[]  stack[]
    6c07b6: CALL indirect edx  ecx=&[esi+0xb4] stack[]
    6c07c3: CALL indirect edx  ecx=edi stack[]
    6c07e4: CALL 0x7a1b10 sub_7a1b10  regs[edi=?]  stack[eax]
    6c07f8: CALL 0x7a5bd0 hud_smoked_busted_message  regs[eax=esi]  stack[]
    6c0827: CALL indirect edx  ecx=i32[esi] stack[]
    6c0834: CALL indirect edx  ecx=i32[esi] stack[]
    6c0843: CALL indirect edx  ecx=i32[esi] stack[0x0]
    6c0858: if (i32[0x2527b2c] <= 0) goto B205  else fallthrough   ; jle 0x6c08ee
    6c0893: CALL indirect eax  ecx=&[esi+0xb4] stack[]
    6c08a0: CALL indirect eax  ecx=edi stack[]
    6c08c1: CALL 0x7a1b10 sub_7a1b10  regs[edi=?]  stack[eax]
    6c08d5: CALL 0x7a5bd0 hud_smoked_busted_message  regs[eax=esi]  stack[]
    6c08e8: if ((ebx add 0x1) < i32[0x2527b2c]) goto B193  else fallthrough   ; jl 0x6c0866
    6c08f1: CALL 0x5bb700 sub_5bb700  regs[]  stack[0]
    6c08ff: CALL 0x600a60 sub_600a60  regs[]  stack[0xe9ec20, 0]
    6c090a: CALL 0x600990 sub_600990  regs[]  stack[0xe9ec20, 0]
    6c0910: CALL 0x63e250 sub_63e250  regs[edi=?]  stack[0]
    6c091c: CALL 0x78fef0 sub_78fef0  regs[ecx=ret(sub_63e250)]  stack[0]
    6c092a: if (i32[0x2527888] == 0) goto B207  else fallthrough   ; je 0x6c0931
    6c092c: CALL 0x603830 sub_603830  regs[]  stack[]
    6c093b: if (i32[0x25274b8] <=u ebx) goto B221  else fallthrough   ; jbe 0x6c09eb
    6c0981: CALL indirect eax  ecx=&[esi+0xb4] stack[]
    6c098e: CALL indirect eax  ecx=edi stack[]
    6c09af: CALL 0x7a1b10 sub_7a1b10  regs[edi=?]  stack[eax]
    6c09c3: CALL 0x7a5bd0 hud_smoked_busted_message  regs[eax=esi]  stack[]
    6c09e5: if ((eax add 0x1) <u i32[0x25274b8]) goto B209  else fallthrough   ; jb 0x6c0947
    6c09f8: CALL indirect eax  ecx=i32[0x140b1c4] stack[0x0]
    6c0a07: CALL indirect eax  ecx=i32[0x140b170] stack[0x0]
    6c0a0b: CALL 0x604540 sub_604540  regs[]  stack[0x0]
    6c0a18: CALL 0x783a70 sub_783a70  regs[esi=0x29dbfe8]  stack[]
    6c0a2f: WRITE [0x2527890] mov= 0x1   ; byte ptr [0x2527890], 1
    6c0a95: CALL 0xbdd1b0 timer_set_random  regs[eax=&[ecx+edx], edi=0xeb7a80]  stack[(ecx sub edx)]
    6c0ab4: CALL 0xb81130 sub_b81130  regs[]  stack[]
    6c0ac0: CALL 0xb7da30 sub_b7da30  regs[]  stack[]
    6c0ad4: CALL 0x79d850 hud_msg_hide_region  regs[]  stack[0x1]
    6c0aeb: WRITE [0x2527cf4] mov= 0x1   ; byte ptr [0x2527cf4], 1
    6c0b00: CALL 0x6b6a40 sub_6b6a40  regs[]  stack[eax]
    6c0b36: CALL 0x6bfac0 sub_6bfac0  regs[ebx=?]  stack[0x0]
    6c0b9c: CALL 0x79d1b0 sub_79d1b0  regs[eax=0x4, ecx=?]  stack[]
    6c0cfa: CALL 0xa57770 sub_a57770  regs[eax=?]  stack[eax]
    6c0d1d: CALL 0xa58130 sub_a58130  regs[ecx=0xe0c644 "-- Cutscene Script Group --"]  stack[0x1]
    6c0d2c: CALL 0xa26380 sub_a26380  regs[]  stack[]
    6c0d36: CALL 0xa575c0 sub_a575c0  regs[]  stack[edx]
    6c0d48: CALL 0x7f4780 sub_7f4780  regs[ecx=0xe0c660 "voice_script"]  stack[0x2771178]
    6c0d5d: CALL 0x6d49f0 xtbl_load_cs_camera_list  regs[]  stack[]
    6c0d72: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c0d77: CALL 0x6c44c0 sub_6c44c0  regs[]  stack[]
    6c0d7c: CALL 0x6be9f0 sub_6be9f0  regs[]  stack[]
    6c0d81: CALL 0x6bea60 sub_6bea60  regs[]  stack[]
    6c0d86: CALL 0x6bead0 sub_6bead0  regs[]  stack[]
    6c0d97: CALL 0xc9604f __security_check_cookie  regs[ecx=(ecx xor esp)]  stack[]
    6c0d9f: RETURN
    6c0db3: CALL 0x6d53e0 sub_6d53e0  regs[]  stack[]
    6c0dcd: CALL 0x6d5260 sub_6d5260  regs[]  stack[(eax add ebx)]
    6c0df8: CALL 0x4c8b80 tod_light_sets_parse  regs[ecx=?, edx=0x2771178]  stack[]
    6c0e0b: CALL 0x6c38a0 sub_6c38a0  regs[esi=ret(tod_light_sets_parse)]  stack[]
    6c0e7a: CALL 0xc99030 __stricmp  regs[]  stack[i32[eax+ecx], i32[ecx+ebx]]
    6c0ecb: if (edi >= i32[0x2526d44]) goto B294  else fallthrough   ; jge 0x6c0f04
    6c0ed2: WRITE [0x2526d44] mov= (eax sub 0x1)   ; dword ptr [0x2526d44], eax
    6c0ee8: CALL 0xc96060 _memmove  regs[]  stack[0x10]
    6c0efc: if ((edi add 0x1) < i32[0x2526d44]) goto B292  else fallthrough   ; jl 0x6c0ee4
    6c0f77: CALL indirect edx  ecx=0x2771178 stack[(ecx or eax), 0x0]
    6c0ff1: CALL 0x6d5950 sub_6d5950  regs[]  stack[]
    6c1004: CALL 0x47f080 audio_pause_all  regs[]  stack[]
    6c1009: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c101a: CALL 0x7f3720 interface_document_load  regs[]  stack[i32[eax+0x15f4]]
    6c1037: CALL 0x69ab80 gamewide_func  regs[]  stack[0x1]
    6c103f: CALL 0x47f170 audio_resume_all  regs[]  stack[]
    6c1055: CALL 0x6d5830 sub_6d5830  regs[]  stack[eax]
    6c1069: CALL indirect [0xdb83d0]  ecx=? stack[eax, 0x1]
    6c1083: CALL 0x522b80 render_thread_wait_frame  regs[]  stack[]
    6c108b: CALL 0x4aa7c0 sub_4aa7c0  regs[]  stack[]
    6c10d5: CALL 0xa80360 streamman_request_chunk  regs[]  stack[&[esp+0x20], &[esp+0x24]]
    6c10fc: CALL 0x6c44c0 sub_6c44c0  regs[]  stack[]
    6c1101: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c1112: CALL 0xc9604f __security_check_cookie  regs[ecx=(ecx xor esp)]  stack[]
    6c111a: RETURN
    6c1123: CALL 0xa80bc0 streamman_cancel_chunk_loads  regs[]  stack[]
    6c1136: CALL 0x6bf6c0 sub_6bf6c0  regs[]  stack[i32[eax+0x1604], 0x2]
    6c1147: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0x84]]  stack[]
    6c115a: CALL 0xa80bc0 streamman_cancel_chunk_loads  regs[]  stack[]
    6c116d: CALL 0xa7b090 sub_a7b090  regs[]  stack[&[esp+0x88], i32[esp+0x18]]
    6c117c: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0xa8]]  stack[]
    6c11ba: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0x9c]]  stack[]
    6c11f3: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0xc4]]  stack[]
    6c11f8: CALL 0x4b3ff0 tod_zone_update  regs[eax=ret(sub_6c8f50)]  stack[]
    6c1200: CALL 0x6c7190 sub_6c7190  regs[]  stack[]
    6c1205: CALL 0x94e070 sub_94e070  regs[]  stack[]
    6c1219: WRITE [0x2526d08] mov= ebx   ; dword ptr [0x2526d08], ebx
    6c121f: WRITE [0x2526d18] mov= ebx   ; dword ptr [0x2526d18], ebx
    6c1262: CALL 0xa57800 sub_a57800  regs[]  stack[ecx]
    6c127a: CALL 0x6bf8d0 sub_6bf8d0  regs[edi=i32[0x21703d4]]  stack[i32[eax+0xaff]]
    6c1290: CALL 0x6bf9a0 sub_6bf9a0  regs[]  stack[i32[edx+0xaff]]
    6c12be: CALL indirect eax  ecx=i32[ecx+edi*4] stack[]
    6c12da: CALL indirect edx  ecx=ret(indirect) stack[(eax and 0xffffff01), 0x1]
    6c12e7: CALL 0xab7800 sub_ab7800  regs[eax=(eax shr 0x1b), esi=ret(indirect)]  stack[]
    6c1319: CALL 0x4c96e0 sub_4c96e0  regs[]  stack[i32[esi+0xaf0], 0x1, 0x1]
    6c132b: CALL 0x4c96e0 sub_4c96e0  regs[]  stack[i32[esi+0xaf0], 0x1]
    6c133d: CALL 0x505430 sub_505430  regs[]  stack[]
    6c1342: CALL 0x9fb830 sub_9fb830  regs[]  stack[]
    6c1347: CALL 0x929c20 sub_929c20  regs[]  stack[]
    6c134c: CALL 0x932fa0 sub_932fa0  regs[]  stack[]
    6c1351: CALL 0x946ac0 sub_946ac0  regs[]  stack[]
    6c1356: CALL 0x9473b0 sub_9473b0  regs[]  stack[]
    6c137b: WRITE [0xe94d04] mov= 0xffff   ; dword ptr [0xe94d04], 0xffff
    6c1385: WRITE [0x2527d25] mov= 0x0   ; byte ptr [0x2527d25], 0
    6c138c: CALL 0x6ce030 sub_6ce030  regs[]  stack[]
    6c13a5: CALL 0x6c1d30 sub_6c1d30  regs[]  stack[i32[eax+0x1534]]
    6c13b9: CALL 0x6cd6d0 sub_6cd6d0  regs[ecx=i32[ecx+0x1534]]  stack[]
    6c13c3: CALL 0xbf9d20 sub_bf9d20  regs[]  stack[0x3e8]
    6c13de: CALL indirect [0xdb80b8]  ecx=? stack[(edx sub eax)]
    6c13e4: CALL 0x7f7ad0 coop_session_active  regs[]  stack[]
    6c13f5: CALL 0x4d1090 screen_fade_start  regs[]  stack[ecx]
    6c13fd: CALL 0x69aed0 sub_69aed0  regs[]  stack[]
    6c1402: CALL 0x739340 sub_739340  regs[]  stack[]
    6c140f: if (i32[0x2527d10] == ebx) goto B359  else fallthrough   ; je 0x6c1505
    6c141a: CALL 0x6d8300 sub_6d8300  regs[]  stack[0xe0c61c "Player Outfit"]
    6c1436: CALL 0x6d8950 sub_6d8950  regs[ecx=i32[esi+0x1534]]  stack[]
    6c144c: CALL 0xbdcb30 string_hash_lower  regs[eax=0, ecx=?]  stack[&[esp+0x78]]
    6c14a7: CALL 0x7c2510 sub_7c2510  regs[edi=&[eax+0x4398]]  stack[(eax add 0x2398), 0x80, i32[ebx+0x13b0]]
    6c14bd: CALL 0x7c4280 sub_7c4280  regs[]  stack[i32[ebx+0x13b0], i32[esi+0x1c], 0x0]
    6c14c2: CALL 0xcfbc00 gfx_device_acquire  regs[]  stack[]
    6c14c9: CALL 0x9e8dc0 sub_9e8dc0  regs[]  stack[0x0]
    6c14d1: CALL indirect [0xdb8080]  ecx=ret(sub_9e8dc0) stack[]
    6c14fd: CALL indirect [0xdb80d0]  ecx=? stack[0x33d62d0]
    6c151a: CALL 0x6d4150 character_load_body_morphs  regs[ecx=i32[0x21703d4]]  stack[]
    6c1573: CALL 0x495240 sub_495240  regs[]  stack[]
    6c1587: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0xc4]]  stack[0x1, 0x1]
    6c1592: CALL 0x4a2840 sub_4a2840  regs[eax=i32[0x21703d4]]  stack[ret(sub_6c8f50)]
    6c15a7: WRITE [0x2527d1c] mov= 0x1   ; byte ptr [0x2527d1c], 1
    6c15ae: CALL 0x7f7ad0 coop_session_active  regs[]  stack[]
    6c15b7: WRITE [0x2527d1d] mov= 0x1   ; byte ptr [0x2527d1d], 1
    6c15e0: CALL 0x6be9f0 sub_6be9f0  regs[]  stack[]
    6c15f3: CALL 0x6bdba0 sub_6bdba0  regs[]  stack[]
    6c162b: CALL 0x47c740 audio_voice_slot_fade_out  regs[eax=?, edx=0x1f4]  stack[]
    6c163c: CALL 0xc9604f __security_check_cookie  regs[ecx=(ecx xor esp)]  stack[]
    6c1644: RETURN
  call targets: 0x43f5f0, 0x454d70(ai_order_k13_exit), 0x458580(ai_order_k23_start), 0x47c740(audio_voice_slot_fade_out), 0x47f080(audio_pause_all), 0x47f170(audio_resume_all), 0x47f200(audio_voices_stop_category), 0x483ac0, 0x4912d0, 0x495240, 0x4a2840, 0x4aa7c0, 0x4b3ff0(tod_zone_update), 0x4c8b80(tod_light_sets_parse), 0x4c96e0, 0x4d1090(screen_fade_start), 0x505430, 0x522b80(render_thread_wait_frame), 0x559d40(human_pkt_send_3e), 0x55b600, 0x5bb700, 0x600990, 0x600a60, 0x603830, 0x604540, 0x63e250, 0x69ab80(gamewide_func), 0x69aed0, 0x6ab240(record_input_step), 0x6ab380, 0x6b6a40, 0x6bdba0, 0x6be9f0, 0x6bea60, 0x6bead0, 0x6bf200, 0x6bf6c0, 0x6bf8d0, 0x6bf9a0, 0x6bfac0, 0x6c1d30, 0x6c38a0, 0x6c44c0, 0x6c7190, 0x6c8f50, 0x6cd6d0, 0x6ce030, 0x6d4150(character_load_body_morphs), 0x6d49f0(xtbl_load_cs_camera_list), 0x6d5260, 0x6d53e0, 0x6d5830, 0x6d5950, 0x6d8300, 0x6d8950, 0x6f0660, 0x739340, 0x782e10, 0x783a70, 0x78fef0, 0x793d60, 0x79ca80, 0x79d1b0, 0x79d850(hud_msg_hide_region), 0x7a1b10, 0x7a5bd0(hud_smoked_busted_message), 0x7c2510, 0x7c4280, 0x7f3720(interface_document_load), 0x7f4780, 0x7f7ad0(coop_session_active), 0x87c8a0, 0x9129a0, 0x926860(misc_packet_send_14), 0x929c20, 0x932fa0, 0x946ac0, 0x9473b0, 0x94e070, 0x95b2c0, 0x95f320, 0x965f40(human_set_force_flag_invulnerable), 0x9661e0(human_set_force_flag_force_holster), 0x96cb70(human_anim_set_update), 0x985630(human_is_dead), 0x988170, 0x98ceb0, 0x9954c0(human_state_set), 0x99f350, 0x9a68d0(human_post_update), 0x9a9010(human_ai_pre_think), 0x9acb10, 0x9acc90, 0x9b04d0, 0x9caf90(human_pkt_send_17), 0x9d9fd0, 0x9da290, 0x9e8dc0, 0x9f2100, 0x9fb830, 0xa26380, 0xa575c0, 0xa57770, 0xa57800, 0xa58130, 0xa7b090, 0xa80360(streamman_request_chunk), 0xa80bc0(streamman_cancel_chunk_loads), 0xaa76f0, 0xab7800, 0xac81e0, 0xb072f0(vehicle_interaction_query), 0xb7da30, 0xb81130, 0xbdcb30(string_hash_lower), 0xbdd1b0(timer_set_random), 0xbf9d20, 0xc14930, 0xc9604f(__security_check_cookie), 0xc96060(_memmove), 0xc99030(__stricmp), 0xca0850(_memcpy), 0xca0bc0(_memset), 0xcb2720(__alldiv), 0xce89a0, 0xce8a00, 0xcedf60, 0xcfbc00(gfx_device_acquire), 0xda0140(_ftol2_sse)
  stack objects passed by address: &[esp+0x20] -> streamman_request_chunk; &[esp+0x24] -> streamman_request_chunk; &[esp+0x50] -> vehicle_interaction_query; &[esp+0x78] -> string_hash_lower; &[esp+0x80] -> sub_95f320; &[esp+0x84] -> sub_6c8f50; &[esp+0x88] -> sub_a7b090; &[esp+0x9c] -> sub_6c8f50; &[esp+0xa8] -> sub_6c8f50; &[esp+0xc4] -> sub_6c8f50
  jump table @0x6c1648 used by 0x6c06a9: 0:0x6c06fc, 1:0x6c06b0, 2:0x6c06b7, 3:0x6c06fc
```

### cutscene_shot_start — 0x6C8130

**Role.** Starts shot number *n* of the running cutscene (`"shot%u"` in the debug label, `"safe_frame"`). Early exit if no player (`*0x2527D10 == 0`) or the shot index is past `player+0x153C`. Sections: (1) per shot character (`script_find_object_by_name` / `script_object_to_human`, `inventory_equip_slot`, `0x6C32A0`, `anim` lookup `0x6EF620`) loop while `index+1 < shotrec+0x108`; (2) per vehicle loop `[player+0xAE8]` with a virtual call, `0x6F0660`, `0xCEDC10 / 0xCEDC50`; (3) `0x6C1770` shot-object update, `render_thread_wait_frame`, `0x6BF6C0` chunk request; (4) `_sprintf("shot%u")`, slate lookup `0x6C7D30`, `0x495240`, `0x4A2840` (camera_script_shot_begin) and `0x4A29E0`; (5) place characters and vehicles (`0x6C24E0`, `0x6C2EF0`), time of day / weather / fade; (6) the audio and effect sections (`0x6C1EF0`, `0x6C1D30`, `0x6CD6D0`, `0x6C1B50`).


**Block map (generated):**

```
FUNCTION 0x006C8130  size 0xa83  insns 707  blocks 134
  entry regs read before write: []

  B0 @0x6c8130:
    6c8136: READ  eax <- i32[0xe80b50] =i32 -1153374642
    6c813d: WRITE [esp+0xac] mov= (eax xor esp)   ; dword ptr [esp + 0xac], eax
    6c8145: READ  ebx <- i32[0x2527d10]
    6c8157: if (i32[0x2527d10] == 0) goto B133  else fallthrough   ; je 0x6c8b9a

  B1 @0x6c815d:
    6c815f: if (esi < 0 (signed)) goto B133  else fallthrough   ; jl 0x6c8b9a

  B2 @0x6c8165:
    6c816b: if (esi >= i32[ebx+0x153c]) goto B133  else fallthrough   ; jge 0x6c8b9a

  B3 @0x6c8171:
    6c8179: if (i32[ebx+0x1540] == 0) goto B5  else fallthrough   ; je 0x6c819c

  B4 @0x6c817b:
    6c8196: WRITE [esp+0x18] mov= (eax add edx)   ; dword ptr [esp + 0x18], eax
    6c819a: goto B6

  B5 @0x6c819c:
    6c819c: WRITE [esp+0x18] mov= 0xffffffff   ; dword ptr [esp + 0x18], 0xffffffff

  B6 @0x6c81a4:
    6c81a6: if (esi < 0 (signed)) goto B29  else fallthrough   ; jl 0x6c836e

  B7 @0x6c81ac:
    6c81ba: WRITE [esp+0x24] mov= 0x0   ; dword ptr [esp + 0x24], 0
    6c81c9: WRITE [esp+0x1c] mov= (eax add [ebx+0x1534])   ; dword ptr [esp + 0x1c], eax
    6c81cd: if (i32[eax+0x108] <= 0x0) goto B20  else fallthrough   ; jle 0x6c82b3

  B8 @0x6c81d3:
    6c81d3: WRITE [esp+0x20] mov= 0x0   ; dword ptr [esp + 0x20], 0
    6c81db: goto B9

  B9 @0x6c81e0:
    6c81f5: if (i32[ebx+0xafd] != 0x0) goto B12  else fallthrough   ; jne 0x6c8207

  B10 @0x6c81f7:
    6c81fb: if (i32[ebp+0x60] != 0x0) goto B12  else fallthrough   ; jne 0x6c8207

  B11 @0x6c81fd:
    6c8201: if (i32[ebp+0x6c] == 0x0) goto B19  else fallthrough   ; je 0x6c8293

  B12 @0x6c8207:
    6c820c: if (i32[ebp] == 0) goto B19  else fallthrough   ; je 0x6c8293

  B13 @0x6c8212:
    6c8218: WRITE [esp+0x17] mov= 0x0   ; byte ptr [esp + 0x17], 0
    6c821d: CALL 0xa52bb0 script_find_object_by_name  regs[ecx=esi]  stack[]
    6c8227: if (i32[esp+0x17] != 0x0) goto B15  else fallthrough   ; jne 0x6c8238

  B14 @0x6c8229:
    6c8229: CALL 0xa52c70 script_object_to_human  regs[esi=?]  stack[]
    6c8230: READ  ebx <- i32[0x2527d10]
    6c8236: if (ret(script_object_to_human) == 0) goto B19  else fallthrough   ; je 0x6c8293

  B15 @0x6c8238:
    6c823c: if (eax == 0) goto B19  else fallthrough   ; je 0x6c8293

  B16 @0x6c823e:
    6c8242: if (i32[ebp+0x36] != 0x1) goto B18  else fallthrough   ; jne 0x6c825c

  B17 @0x6c8244:
    6c8251: CALL 0x55a930 inventory_equip_slot  regs[]  stack[&[edi+0x10c4], 0x0, 0x1, 0x0]
    6c8256: READ  ebx <- i32[0x2527d10]

  B18 @0x6c825c:
    6c8262: CALL 0x6c32a0 sub_6c32a0  regs[esi=i32[edi+0x5e0]]  stack[]
    6c8272: WRITE [edi+0x758] or= 0x1   ; dword ptr [edi + 0x758], eax
    6c8278: WRITE [edx+0x14] mov= 0x1   ; dword ptr [edx + 0x14], eax
    6c8288: CALL 0x6ef620 sub_6ef620  regs[]  stack[i32[edi+0x5e0], i32[edi+0x14], 0x0]
    6c8290: WRITE [edi+0x18] mov= ret(sub_6ef620)   ; dword ptr [edi + 0x18], eax

  B19 @0x6c8293:
    6c829b: WRITE [esp+0x20] add= 0x7c   ; dword ptr [esp + 0x20], 0x7c
    6c82a9: WRITE [esp+0x24] mov= (eax add 0x1)   ; dword ptr [esp + 0x24], eax
    6c82ad: if ((eax add 0x1) < i32[ecx+0x108]) goto B9  else fallthrough   ; jl 0x6c81e0

  B20 @0x6c82b3:
    6c82bb: if (i32[ebx+0xae8] <= 0) goto B27  else fallthrough   ; jle 0x6c833f

  B21 @0x6c82c1:

  B22 @0x6c82c4:
    6c82d2: CALL indirect edx  ecx=i32[edx+edi*4] stack[]
    6c82d6: if (ret(indirect) == 0) goto B26  else fallthrough   ; je 0x6c832e

  B23 @0x6c82d8:
    6c82f6: if (je (flags from earlier instr)) goto B26  else fallthrough   ; je 0x6c832e

  B24 @0x6c82f8:
    6c82fa: if (i32[esi] == ebp) goto B26  else fallthrough   ; je 0x6c832e

  B25 @0x6c82fc:
    6c8300: X87 STORE f32 [esp] <- 0.0  (rounds to f32)
    6c8304: CALL 0x6f0660 sub_6f0660  regs[]  stack[ecx]
    6c830e: CALL 0xcedc10 sub_cedc10  regs[ecx=i32[esi]]  stack[]
    6c8317: CALL 0xcedc50 sub_cedc50  regs[ecx=i32[esi]]  stack[0x0]
    6c831e: X87 STORE f32 [esi+0x24] <- 0.0  (rounds to f32)
    6c8324: WRITE [esi+0x14] mov= ebp   ; dword ptr [esi + 0x14], ebp
    6c8327: WRITE [esi+0x18] mov= 0xffff   ; dword ptr [esi + 0x18], 0xffff

  B26 @0x6c832e:
    6c832e: READ  ebx <- i32[0x2527d10]
    6c833d: if ((edi add 0x1) < i32[ebx+0xae8]) goto B22  else fallthrough   ; jl 0x6c82c4

  B27 @0x6c833f:
    6c8345: if (i32[esp+0x18] < 0 (signed)) goto B29  else fallthrough   ; jl 0x6c836e

  B28 @0x6c8347:
    6c8360: CALL 0x6c1770 sub_6c1770  regs[eax=?, ebx=?]  stack[(edx add 0x64)]
    6c8365: READ  ebx <- i32[0x2527d10]

  B29 @0x6c836e:
    6c8383: WRITE [ebx+0x1540] mov= (ebp add eax)   ; dword ptr [ebx + 0x1540], ebp
    6c8390: WRITE [esp+0x24] mov= (eax imul eax)   ; dword ptr [esp + 0x24], eax
    6c8394: WRITE [esp+0x20] mov= (ebp add eax)   ; dword ptr [esp + 0x20], ebp
    6c8398: if (i32[ebp+0xfc] == 0x0) goto B31  else fallthrough   ; je 0x6c83b5

  B30 @0x6c839a:
    6c839c: CALL 0x522b80 render_thread_wait_frame  regs[]  stack[0x0]
    6c83ad: CALL 0x6bf6c0 sub_6bf6c0  regs[]  stack[i32[ebp+0xfc], 0x1]

  B31 @0x6c83b5:
    6c83d1: WRITE [esp+0x20] mov= &[ebx+0x1]   ; dword ptr [esp + 0x20], eax
    6c83d5: CALL 0xbf2c10 sub_bf2c10  regs[edi=&[esp+0x80], esi=0x40]  stack[0xe0c71c "shot%u", &[ebx+0x1]]
    6c83e5: WRITE [eax] mov= i32[ebp+0x4]   ; dword ptr [eax], ecx
    6c83e7: CALL 0x6c7d30 sub_6c7d30  regs[]  stack[ret(sub_bf2c10), 0x1]
    6c83ec: READ  eax <- i32[0x2527d10]
    6c83fb: if (i32[eax+0xb02] != 0x0) goto B39  else fallthrough   ; jne 0x6c845b

  B32 @0x6c83fd:
    6c8404: if (i32[eax+0xafd] != 0x0) goto B34  else fallthrough   ; jne 0x6c840d

  B33 @0x6c8406:
    6c8406: CALL 0x495240 sub_495240  regs[]  stack[]
    6c840b: goto B35

  B34 @0x6c840d:
    6c8419: CALL 0x6c8f50 sub_6c8f50  regs[eax=&[esp+0x48]]  stack[0x0, 0x0, 0x0, 0x0]
    6c841f: READ  eax <- i32[0x21703d4]
    6c8424: CALL 0x4a2840 sub_4a2840  regs[eax=i32[0x21703d4]]  stack[ret(sub_6c8f50)]

  B35 @0x6c842c:
    6c842c: READ  ecx <- i32[0x2527d10]
    6c843a: if (i32[ecx+0x15f8] == 0) goto B37  else fallthrough   ; je 0x6c8443

  B36 @0x6c843c:
    6c843c: WRITE [eax+0x460] mov= 0x0   ; byte ptr [eax + 0x460], 0

  B37 @0x6c8443:
    6c844a: if (i32[ecx+0x1600] == 0x0) goto B39  else fallthrough   ; je 0x6c845b

  B38 @0x6c844c:
    6c844c: READ  edx <- i32[0x140e670]
    6c8455: CALL indirect [0xdb83d0]  ecx=? stack[i32[0x140e670], 0x0]

  B39 @0x6c845b:
    6c845c: CALL 0x6c24e0 sub_6c24e0  regs[]  stack[]
    6c8465: CALL 0x6c2ef0 sub_6c2ef0  regs[]  stack[]
    6c8471: CALL 0x6c1770 sub_6c1770  regs[eax=ebx, ebx=?]  stack[0x0]
    6c8476: READ  edi <- i32[0x2527d10]
    6c8486: if (i32[edi+0xb02] != 0x0) goto B46  else fallthrough   ; jne 0x6c84c7

  B40 @0x6c8488:
    6c848f: if (i32[0x2527d14] == 0x0) goto B42  else fallthrough   ; je 0x6c849a

  B41 @0x6c8491:
    6c8498: if (i32[0x2527cf7] != 0x0) goto B43  else fallthrough   ; jne 0x6c84a3

  B42 @0x6c849a:
    6c84a1: if (i32[0x2527d15] == 0x0) goto B45  else fallthrough   ; je 0x6c84bd

  B43 @0x6c84a3:
    6c84aa: if (i32[0x2527d25] != 0x0) goto B45  else fallthrough   ; jne 0x6c84bd

  B44 @0x6c84ac:
    6c84b1: CALL 0x47b660 sub_47b660  regs[esi=0xeab83c]  stack[]
    6c84b6: WRITE [0x2527d25] mov= 0x1   ; byte ptr [0x2527d25], 1

  B45 @0x6c84bd:
    6c84bd: WRITE [0x2526a74] mov= 0x0   ; dword ptr [0x2526a74], 0

  B46 @0x6c84c7:
    6c84ce: if (i32[edi+0xafd] != 0x1) goto B54  else fallthrough   ; jne 0x6c856c

  B47 @0x6c84d4:
    6c84d6: if (ebx <= 0 (signed)) goto B49  else fallthrough   ; jle 0x6c84f3

  B48 @0x6c84d8:
    6c84eb: CALL 0x6c1ef0 sub_6c1ef0  regs[edi=&[eax+ecx-0x170]]  stack[0x0]

  B49 @0x6c84f3:
    6c84f4: CALL 0x6c1d30 sub_6c1d30  regs[]  stack[]
    6c84fe: CALL 0x6cd6d0 sub_6cd6d0  regs[ecx=ebp]  stack[]
    6c8504: CALL 0x6c1b50 sub_6c1b50  regs[]  stack[]
    6c8509: READ  edi <- i32[0x2527d10]
    6c851c: if (i32[esp+0x1c] >= i32[edi+0x153c]) goto B54  else fallthrough   ; jge 0x6c856c

  B50 @0x6c851e:
    6c8537: if (i32[esi+0x12c] <= 0) goto B53  else fallthrough   ; jle 0x6c855f

  B51 @0x6c8539:
    6c853b: goto B52

  B52 @0x6c8540:
    6c854c: CALL 0x6c1cb0 sub_6c1cb0  regs[edi=(edi add ebp), edx=i32[esp+0x20]]  stack[]
    6c855d: if ((ebx add 0x1) < i32[esi+0x12c]) goto B52  else fallthrough   ; jl 0x6c8540

  B53 @0x6c855f:
    6c8561: CALL 0x6cd6d0 sub_6cd6d0  regs[ecx=esi]  stack[]
    6c8566: READ  edi <- i32[0x2527d10]

  B54 @0x6c856c:
    6c8576: X87 STORE f32 [0xe997c8] <- f32[0xdd0550] =f32 10 (0x41200000)  (rounds to f32)
    6c857c: WRITE [0x25f5d90] mov= 0x0   ; byte ptr [0x25f5d90], 0
    6c8589: WRITE [edi+0x155c] mov= 0xffffffff   ; dword ptr [edi + 0x155c], 0xffffffff
    6c8593: X87 STORE f32 [0xe997cc] <- f32[0xdd0794] =f32 5 (0x40a00000)  (rounds to f32)
    6c859f: X87 STORE f32 [0xe997d0] <- f32[0xdd1944] =f32 0.150000006 (0x3e19999a)  (rounds to f32)
    6c85ab: X87 STORE f32 [0xe997d4] <- f32[0xdd1c50] =f32 0.400000006 (0x3ecccccd)  (rounds to f32)
    6c85b7: X87 STORE f32 [0xe9898c] <- f32[edi+0x1660]  (rounds to f32)
    6c85bf: X87 STORE f32 [esp+0x28] <- 0.0  (rounds to f32)
    6c85c7: X87 STORE f32 [esp+0x2c] <- 0.0  (rounds to f32)
    6c85cf: X87 STORE f32 [esp+0x30] <- 0.0  (rounds to f32)
    6c85d7: WRITE [edi+0x15e4] mov= i32[esp+0x28]   ; dword ptr [edi + 0x15e4], edx
    6c85dd: WRITE [edi+0x15e8] mov= i32[esp+0x2c]   ; dword ptr [edi + 0x15e8], eax
    6c85e3: WRITE [edi+0x15ec] mov= i32[esp+0x30]   ; dword ptr [edi + 0x15ec], ecx
    6c85eb: CALL 0x6d8900 sub_6d8900  regs[edi=i32[esp+0x20]]  stack[]
    6c85f0: READ  ebp <- i32[0x2527d10]
    6c8600: if (i32[ebp+0xaf0] == 0) goto B92  else fallthrough   ; je 0x6c884b

  B55 @0x6c8606:
    6c860c: if (i32[ebp+0xac0] == esi) goto B92  else fallthrough   ; je 0x6c884b

  B56 @0x6c8612:
    6c861f: CALL 0x6c1660 sub_6c1660  regs[eax=i32[ebp+0xac4], ecx=&[ebx+0x14c]]  stack[eax]
    6c8632: if (i32[eax+0x10] <= esi) goto B61  else fallthrough   ; jle 0x6c8660

  B57 @0x6c8634:

  B58 @0x6c8636:
    6c8647: if (i32[eax+edx+0x84] < esi) goto B60  else fallthrough   ; jl 0x6c864c

  B59 @0x6c8649:
    6c8649: WRITE [eax+0x8] mov= esi   ; dword ptr [eax + 8], esi

  B60 @0x6c864c:
    6c865e: if ((ecx add 0x1) < i32[eax+0x10]) goto B58  else fallthrough   ; jl 0x6c8636

  B61 @0x6c8660:
    6c8668: WRITE [esp+0x18] mov= 0   ; dword ptr [esp + 0x18], eax
    6c866c: if (i32[ebx+0x108] <= esi) goto B78  else fallthrough   ; jle 0x6c8750

  B62 @0x6c8672:
    6c8682: if (i32[ecx+esi+0x34] == 0x1) goto B77  else fallthrough   ; je 0x6c8735

  B63 @0x6c8688:
    6c868f: if (i32[ebp+0x1608] != 0x1) goto B67  else fallthrough   ; jne 0x6c86ad

  B64 @0x6c8691:
    6c8693: if (eax != 0) goto B66  else fallthrough   ; jne 0x6c86a1

  B65 @0x6c8695:
    6c8695: READ  edx <- i32[0x21703d4]
    6c869b: WRITE [esp+0x1c] mov= i32[0x21703d4]   ; dword ptr [esp + 0x1c], edx
    6c869f: goto B69

  B66 @0x6c86a1:
    6c86ab: goto B68

  B67 @0x6c86ad:

  B68 @0x6c86b6:
    6c86bb: CALL indirect eax  ecx=? stack[]
    6c86bd: READ  ebp <- i32[0x2527d10]
    6c86c3: WRITE [esp+0x1c] mov= ret(indirect)   ; dword ptr [esp + 0x1c], eax

  B69 @0x6c86c7:
    6c86d5: if (i32[ecx+esi+0x54] == 0x0) goto B77  else fallthrough   ; je 0x6c8735

  B70 @0x6c86d7:
    6c86eb: if (i32[ebx+0x8] <= 0) goto B77  else fallthrough   ; jle 0x6c8735

  B71 @0x6c86ed:

  B72 @0x6c86f0:
    6c86f9: if (edx >= i32[ecx+0x10]) goto B77  else fallthrough   ; jge 0x6c8735

  B73 @0x6c86fb:
    6c8701: if (i32[edx+esi] != 0x0) goto B76  else fallthrough   ; jne 0x6c8728

  B74 @0x6c8703:
    6c871a: if (&[esi+0x1] > i32[ecx+0x4]) goto B76  else fallthrough   ; jg 0x6c8728

  B75 @0x6c871c:
    6c871c: WRITE [ecx+0x8] mov= edi   ; dword ptr [ecx + 8], edi
    6c8725: WRITE [ecx+esi*4] mov= i32[esp+0x1c]   ; dword ptr [ecx + esi*4], edi

  B76 @0x6c8728:
    6c8733: if ((edx add 0x1) < i32[ebx+0x8]) goto B72  else fallthrough   ; jl 0x6c86f0

  B77 @0x6c8735:
    6c8746: WRITE [esp+0x18] mov= (eax add 0x1)   ; dword ptr [esp + 0x18], eax
    6c874a: if ((eax add 0x1) < i32[ebx+0x108]) goto B62  else fallthrough   ; jl 0x6c8672

  B78 @0x6c8750:
    6c8758: WRITE [esp+0x18] mov= 0   ; dword ptr [esp + 0x18], ecx
    6c875c: if (i32[ebx+0x114] <= 0) goto B92  else fallthrough   ; jle 0x6c884b

  B79 @0x6c8762:
    6c8764: WRITE [esp+0x1c] mov= 0   ; dword ptr [esp + 0x1c], esi

  B80 @0x6c8768:
    6c877a: if (i32[eax+0x54] <= i32[ebp+0xac4]) goto B91  else fallthrough   ; jle 0x6c8825

  B81 @0x6c8780:
    6c8784: if (i32[eax+0x34] == 0x1) goto B91  else fallthrough   ; je 0x6c8825

  B82 @0x6c878a:
    6c8797: if (i32[eax+ecx*4] == 0x0) goto B91  else fallthrough   ; je 0x6c8825

  B83 @0x6c879d:
    6c87a4: CALL indirect eax  ecx=i32[eax] stack[]
    6c87ac: READ  ebp <- i32[0x2527d10]
    6c87b8: WRITE [esp+0x34] mov= ret(indirect)   ; dword ptr [esp + 0x34], eax
    6c87bc: if (i32[ecx+0x54] == 0x0) goto B91  else fallthrough   ; je 0x6c8825

  B84 @0x6c87be:
    6c87d2: if (i32[ebx+0x8] <= 0) goto B91  else fallthrough   ; jle 0x6c8825

  B85 @0x6c87d4:
    6c87d6: goto B86

  B86 @0x6c87e0:
    6c87e9: if (edx >= i32[ecx+0x10]) goto B91  else fallthrough   ; jge 0x6c8825

  B87 @0x6c87eb:
    6c87f1: if (i32[edx+esi] != 0x0) goto B90  else fallthrough   ; jne 0x6c8818

  B88 @0x6c87f3:
    6c880a: if (&[esi+0x1] > i32[ecx+0x4]) goto B90  else fallthrough   ; jg 0x6c8818

  B89 @0x6c880c:
    6c880c: WRITE [ecx+0x8] mov= edi   ; dword ptr [ecx + 8], edi
    6c8815: WRITE [ecx+esi*4] mov= i32[esp+0x34]   ; dword ptr [ecx + esi*4], edi

  B90 @0x6c8818:
    6c8823: if ((edx add 0x1) < i32[ebx+0x8]) goto B86  else fallthrough   ; jl 0x6c87e0

  B91 @0x6c8825:
    6c883d: WRITE [esp+0x18] mov= (ecx add 0x1)   ; dword ptr [esp + 0x18], ecx
    6c8841: WRITE [esp+0x1c] mov= (esi add 0x7c)   ; dword ptr [esp + 0x1c], esi
    6c8845: if ((ecx add 0x1) < i32[ebx+0x114]) goto B80  else fallthrough   ; jl 0x6c8768

  B92 @0x6c884b:
    6c8852: if (i32[ebp+0xafd] != 0x1) goto B113  else fallthrough   ; jne 0x6c8a4c

  B93 @0x6c8858:
    6c8863: X87 STORE f32 [esp] <- 0.0  (rounds to f32)
    6c8866: CALL 0x6c92a0 sub_6c92a0  regs[edi=(esp sub 0xc), esi=&[ebx+0x1c]]  stack[ecx]
    6c8871: CALL 0x6c34c0 sub_6c34c0  regs[eax=ebx, ecx=&[esp+0x44]]  stack[]
    6c8881: WRITE [esp+0x34] mov= i32[eax]   ; dword ptr [esp + 0x34], edx
    6c888d: WRITE [esp+0x2c] mov= i32[eax+0x4]   ; dword ptr [esp + 0x2c], ecx
    6c8891: WRITE [esp+0x30] mov= i32[eax+0x8]   ; dword ptr [esp + 0x30], edx
    6c8895: if (i32[ebx+0xb0] == 0) goto B110  else fallthrough   ; je 0x6c8981

  B94 @0x6c889b:
    6c88a1: WRITE [esp+0x17] mov= 0x0   ; byte ptr [esp + 0x17], 0
    6c88a6: CALL 0xa52bb0 script_find_object_by_name  regs[ecx=esi]  stack[]
    6c88b0: if (i32[esp+0x17] == 0x0) goto B96  else fallthrough   ; je 0x6c88b6

  B95 @0x6c88b2:
    6c88b4: goto B104

  B96 @0x6c88b6:
    6c88b8: CALL 0xa275f0 script_follower_index  regs[eax=esi]  stack[]
    6c88bf: if (ret(script_follower_index) <= 0 (signed)) goto B98  else fallthrough   ; jle 0x6c88cf

  B97 @0x6c88c1:
    6c88c2: CALL 0xa27660 script_follower_get  regs[]  stack[eax]
    6c88ca: goto B108

  B98 @0x6c88cf:
    6c88d6: CALL 0xbdca30 string_crc32  regs[eax=&[esp+0x38]]  stack[]
    6c88db: READ  edx <- i32[0x2f25254]
    6c88e5: if (i32[0x2f25254] == 0) goto B103  else fallthrough   ; je 0x6c8903

  B99 @0x6c88e7:

  B100 @0x6c88f0:
    6c88f3: if (i32[ecx+0xc] == eax) goto B106  else fallthrough   ; je 0x6c896e

  B101 @0x6c88f5:
    6c88fd: if (i32[ecx+0x17c] == edx) goto B103  else fallthrough   ; je 0x6c8903

  B102 @0x6c88ff:
    6c8901: if (ecx != 0) goto B100  else fallthrough   ; jne 0x6c88f0

  B103 @0x6c8903:

  B104 @0x6c8905:
    6c8911: CALL 0x4a29e0 sub_4a29e0  regs[esi=?]  stack[]
    6c891d: if (i32[0x25f5d90] != 0x1) goto B111  else fallthrough   ; jne 0x6c8a13

  B105 @0x6c8923:
    6c892b: X87 STORE f32 [esp+0x3c] <- (f32[esi+0x14] - f32[esp+0x2c])  (rounds to f32)
    6c8936: X87 STORE f32 [esp+0x44] <- (f32[esi+0x1c] - f32[esp+0x34])  (rounds to f32)
    6c8950: X87 STORE f32 [esp+0x38] <- (((f32[esp+0x3c] * f32[esp+0x3c]) + (0.0 * 0.0)) + (f32[esp+0x44] * f32[esp+0x44]))  (rounds to f32)
    6c8958: X87 STORE f32 [esp] <- f32[esp+0x38]  (rounds to f32)
    6c895b: CALL 0x40ea20 sqrtf_wrapper  regs[]  stack[ecx]
    6c8960: X87 STORE f32 [0x25f5d94] <- ST0<ret sqrtf_wrapper>  (rounds to f32)
    6c8969: goto B111

  B106 @0x6c896e:
    6c8970: if (ecx == 0) goto B103  else fallthrough   ; je 0x6c8903

  B107 @0x6c8972:
    6c8977: CALL indirect edx  ecx=? stack[]

  B108 @0x6c8979:
    6c897b: if (eax == 0) goto B103  else fallthrough   ; je 0x6c8903

  B109 @0x6c897d:
    6c897f: goto B104

  B110 @0x6c8981:
    6c8987: CALL 0x6beed0 sub_6beed0  regs[eax=&[esp+0x48]]  stack[]
    6c8997: CALL 0x6c78b0 sub_6c78b0  regs[ecx=&[esp+0x50]]  stack[&[esp+0x38], 0x0]
    6c89ab: X87 STORE f32 [esp+0x48] <- (f32[esp+0x40] - f32[esp+0x30])  (rounds to f32)
    6c89b7: X87 STORE f32 [esp+0x4c] <- (f32[esp+0x3c] - f32[esp+0x2c])  (rounds to f32)
    6c89c3: X87 STORE f32 [esp+0x50] <- (f32[esp+0x40] - f32[esp+0x30])  (rounds to f32)
    6c89d1: X87 STORE f32 [esp+0x38] <- (f32[esp+0x48] + 0.0)  (rounds to f32)
    6c89df: X87 STORE f32 [esp+0x3c] <- (f32[esp+0x4c] + f64[0xe5bfe8] =f64 2 (0x4000000000000000))  (rounds to f32)
    6c89e7: X87 STORE f32 [esp+0x40] <- (0.0 + f32[esp+0x50])  (rounds to f32)
    6c89eb: CALL 0xbde150 vec3_normalize_safe  regs[esi=&[esp+0x38]]  stack[]
    6c89f6: CALL 0xbe4b20 mat33_from_forward  regs[eax=&[esp+0x38], ecx=&[esp+0x54]]  stack[]
    6c8a07: CALL 0x4a29e0 sub_4a29e0  regs[esi=0]  stack[]
    6c8a0c: WRITE [0x25f5d90] mov= 0x0   ; byte ptr [0x25f5d90], 0

  B111 @0x6c8a13:
    6c8a1a: if (i32[ebx+0x9c] != 0x1) goto B113  else fallthrough   ; jne 0x6c8a4c

  B112 @0x6c8a1c:
    6c8a22: X87 STORE f32 [0xe997c8] <- f32[ebx+0xa0]  (rounds to f32)
    6c8a2e: X87 STORE f32 [0xe997cc] <- f32[ebx+0xa4]  (rounds to f32)
    6c8a3a: X87 STORE f32 [0xe997d0] <- f32[ebx+0xa8]  (rounds to f32)
    6c8a46: X87 STORE f32 [0xe997d4] <- f32[ebx+0xac]  (rounds to f32)

  B113 @0x6c8a4c:
    6c8a4c: CALL 0x4932f0 camera_update  regs[]  stack[]
    6c8a58: CALL 0x4b3ff0 tod_zone_update  regs[eax=0x25f5b20]  stack[0x0]
    6c8a65: X87 STORE f32 [esp] <- 0.0  (rounds to f32)
    6c8a69: CALL 0x6befc0 sub_6befc0  regs[]  stack[ret(tod_zone_update), 0x0]
    6c8a71: CALL 0x4c8770 sub_4c8770  regs[]  stack[]
    6c8a76: CALL 0x4cf840 weather_rain_effects_update  regs[]  stack[]
    6c8a7b: READ  ecx <- i32[0x2527d10]
    6c8a8d: CALL 0x6cd9a0 sub_6cd9a0  regs[edi=(edi add [esp+0x24])]  stack[0x0]
    6c8a92: READ  eax <- i32[0x2527d10]
    6c8aa1: if (i32[eax+0xb02] != 0x0) goto B132  else fallthrough   ; jne 0x6c8b93

  B114 @0x6c8aa7:
    6c8aae: if (i32[eax+0xb03] != 0x0) goto B116  else fallthrough   ; jne 0x6c8ac5

  B115 @0x6c8ab0:
    6c8ab3: X87 STORE f32 [esp] <- 0.0  (rounds to f32)
    6c8abd: CALL 0x4d1270 sub_4d1270  regs[edx=?]  stack[0x1f4, ecx]

  B116 @0x6c8ac5:
    6c8ac7: READ  eax <- i32[0x252853c]
    6c8ace: X87 STORE f32 [0x1f76904] <- 1.0  (rounds to f32)
    6c8ad4: WRITE [0x140c85a] mov= 0x1   ; byte ptr [0x140c85a], 1
    6c8adb: WRITE [0xeac468] mov= 0xffffffff   ; dword ptr [0xeac468], 0xffffffff
    6c8ae5: if (i32[0x252853c] == 0) goto B131  else fallthrough   ; je 0x6c8b87

  B117 @0x6c8aeb:
    6c8aef: if (ax >=u 0xc) goto B131  else fallthrough   ; jae 0x6c8b87

  B118 @0x6c8af5:
    6c8b07: if (i32[edi+0x4c] != eax) goto B131  else fallthrough   ; jne 0x6c8b87

  B119 @0x6c8b09:
    6c8b0e: CALL 0xbdc9b0 string_crc32_lower  regs[ecx=0xe0c638 "safe_frame"]  stack[]
    6c8b15: if (ret(string_crc32_lower) == 0) goto B131  else fallthrough   ; je 0x6c8b87

  B120 @0x6c8b17:
    6c8b27: if (i32[ecx*4+0x22ef1b8] == 0) goto B131  else fallthrough   ; je 0x6c8b87

  B121 @0x6c8b29:

  B122 @0x6c8b30:
    6c8b33: if (i32[esi+0x30] != edi) goto B124  else fallthrough   ; jne 0x6c8b3a

  B123 @0x6c8b35:
    6c8b38: if (i32[esi+0x2c] == eax) goto B128  else fallthrough   ; je 0x6c8b49

  B124 @0x6c8b3a:
    6c8b3f: if (i32[esi+0x4] == ecx) goto B127  else fallthrough   ; je 0x6c8b47

  B125 @0x6c8b41:
    6c8b43: if (esi != 0) goto B122  else fallthrough   ; jne 0x6c8b30

  B126 @0x6c8b45:
    6c8b45: goto B128

  B127 @0x6c8b47:

  B128 @0x6c8b49:
    6c8b4b: if (esi == 0) goto B131  else fallthrough   ; je 0x6c8b87

  B129 @0x6c8b4d:
    6c8b4d: READ  eax <- i32[0xebdf6c] =i32 15232652
    6c8b5c: CALL indirect edx  ecx=esi stack[i32[eax]]
    6c8b60: if (al != 0x1) goto B131  else fallthrough   ; jne 0x6c8b87

  B130 @0x6c8b62:
    6c8b62: WRITE [esp+0x3c] mov= al   ; byte ptr [esp + 0x3c], al
    6c8b74: WRITE [esp+0x40] mov= 0x5   ; dword ptr [esp + 0x40], 5
    6c8b7c: CALL indirect edx  ecx=esi stack[&[esp+0x3c], 0x0]
    6c8b82: CALL 0xb93790 vint_value_free  regs[eax=&[esp+0x38]]  stack[]

  B131 @0x6c8b87:
    6c8b87: READ  eax <- i32[0x2527d10]
    6c8b8c: WRITE [eax+0xb02] mov= 0x1   ; byte ptr [eax + 0xb02], 1

  B132 @0x6c8b93:
    6c8b93: WRITE [eax+0xb0c] mov= 0x1   ; byte ptr [eax + 0xb0c], 1

  B133 @0x6c8b9a:
    6c8ba7: CALL 0xc9604f __security_check_cookie  regs[ecx=(ecx xor esp)]  stack[]
    6c8bb2: RETURN

  call targets: 0x40ea20(sqrtf_wrapper), 0x47b660, 0x4932f0(camera_update), 0x495240, 0x4a2840, 0x4a29e0, 0x4b3ff0(tod_zone_update), 0x4c8770, 0x4cf840(weather_rain_effects_update), 0x4d1270, 0x522b80(render_thread_wait_frame), 0x55a930(inventory_equip_slot), 0x6beed0, 0x6befc0, 0x6bf6c0, 0x6c1660, 0x6c1770, 0x6c1b50, 0x6c1cb0, 0x6c1d30, 0x6c1ef0, 0x6c24e0, 0x6c2ef0, 0x6c32a0, 0x6c34c0, 0x6c78b0, 0x6c7d30, 0x6c8f50, 0x6c92a0, 0x6cd6d0, 0x6cd9a0, 0x6d8900, 0x6ef620, 0x6f0660, 0xa275f0(script_follower_index), 0xa27660(script_follower_get), 0xa52bb0(script_find_object_by_name), 0xa52c70(script_object_to_human), 0xb93790(vint_value_free), 0xbdc9b0(string_crc32_lower), 0xbdca30(string_crc32), 0xbde150(vec3_normalize_safe), 0xbe4b20(mat33_from_forward), 0xbf2c10, 0xc9604f(__security_check_cookie), 0xcedc10, 0xcedc50
  stack objects passed by address: &[esp+0x38] -> mat33_from_forward,string_crc32,sub_6c78b0,vec3_normalize_safe,vint_value_free; &[esp+0x44] -> sub_6c34c0; &[esp+0x48] -> sub_6beed0,sub_6c8f50; &[esp+0x50] -> sub_6c78b0; &[esp+0x54] -> mat33_from_forward; &[esp+0x80] -> sub_bf2c10
```

### cutscene_shot_place_characters — 0x6C24E0

**Role.** For each character of the shot record (count at `rec+0x108`, argument 1) resolves the script object, builds a facing matrix (`vec3_normalize_in_place`, `mat33_from_forward`), positions the human (`object_teleport`, flags 0,1,1,0), equips weapon slots (`inventory_equip_slot`), sends `misc_packet_send_14`, and kicks the animation (`0x6E4DE0 / 0x6E4E10`, `0xCEBB60`). Uses 26 virtual calls through object vtables.


**Block map (generated):**

```
FUNCTION 0x006C24E0  size 0xa03  insns 697  blocks 139
  entry regs read before write: []

  B0 @0x6c24e0:
    6c24f7: CALL 0x6beed0 sub_6beed0  regs[eax=&[esp+0x4c]]  stack[]
    6c2503: WRITE [esp+0x20] mov= 0x0   ; dword ptr [esp + 0x20], 0
    6c250b: if (i32[esi+0x108] <= 0x0) goto B88  else fallthrough   ; jle 0x6c2b5c

  B1 @0x6c2511:
    6c252a: if (i32[ebp] == 0) goto B87  else fallthrough   ; je 0x6c2b3e

  B2 @0x6c2530:
    6c2536: WRITE [esp+0x1f] mov= 0x0   ; byte ptr [esp + 0x1f], 0
    6c253b: CALL 0xa52bb0 script_find_object_by_name  regs[ecx=esi]  stack[]
    6c2545: if (i32[esp+0x1f] != 0x0) goto B8  else fallthrough   ; jne 0x6c257d

  B3 @0x6c2547:
    6c2549: CALL 0xa275f0 script_follower_index  regs[eax=esi]  stack[]
    6c2550: if (ret(script_follower_index) <= 0 (signed)) goto B5  else fallthrough   ; jle 0x6c255d

  B4 @0x6c2552:
    6c2553: CALL 0xa27660 script_follower_get  regs[]  stack[eax]
    6c255b: goto B7

  B5 @0x6c255d:
    6c255f: CALL 0xa618a0 script_object_find  regs[]  stack[]
    6c2566: if (ret(script_object_find) == 0) goto B87  else fallthrough   ; je 0x6c2b3e

  B6 @0x6c256c:
    6c2573: CALL indirect eax  ecx=eax stack[]

  B7 @0x6c2575:
    6c2577: if (eax == 0) goto B87  else fallthrough   ; je 0x6c2b3e

  B8 @0x6c257d:
    6c2581: if (eax == 0) goto B87  else fallthrough   ; je 0x6c2b3e

  B9 @0x6c2587:
    6c258b: if (i32[ebp+0x37] != 0x0) goto B15  else fallthrough   ; jne 0x6c26c1

  B10 @0x6c2591:
    6c2595: X87 STORE f32 [esp+0x34] <- f32[ebp+0x4]  (rounds to f32)
    6c259c: X87 STORE f32 [esp+0x38] <- f32[ebp+0x8]  (rounds to f32)
    6c25a3: X87 STORE f32 [esp+0x3c] <- f32[ebp+0xc]  (rounds to f32)
    6c25af: X87 STORE f32 [esp+0x28] <- (f32[esp+0x50] - f32[esp+0x34])  (rounds to f32)
    6c25bb: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x54] - f32[esp+0x38])  (rounds to f32)
    6c25c7: X87 STORE f32 [esp+0x30] <- (f32[esp+0x58] - f32[esp+0x3c])  (rounds to f32)
    6c25e7: X87 STORE f32 [esp+0x18] <- (((f32[esp+0x28] * f32[esp+0x28]) + (f32[esp+0x2c] * f32[esp+0x2c])) + (f32[esp+0x30] * f32[esp+0x30]))  (rounds to f32)
    6c25ef: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    6c25f2: CALL 0x40ea20 sqrtf_wrapper  regs[]  stack[ecx]
    6c25f7: X87 COMPARE ST0<ret sqrtf_wrapper>  vs  f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)   [fcomp]
    6c2605: if ((ST0<ret sqrtf_wrapper> < f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)) OR (ST0<ret sqrtf_wrapper> == f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)) OR UNORDERED(NaN)) goto B13  else fallthrough   ; jne 0x6c2621

  B11 @0x6c2607:
    6c2607: READ  ecx <- i32[0x2527d10]
    6c2614: if (i32[ecx+0xafd] != 0x1) goto B13  else fallthrough   ; jne 0x6c2621

  B12 @0x6c2616:
    6c261a: CALL 0xbddff0 vec3_normalize_in_place  regs[esi=&[esp+0x24]]  stack[]
    6c261f: goto B14

  B13 @0x6c2621:
    6c2623: X87 STORE f32 [esp+0x40] <- 0.0  (rounds to f32)
    6c262b: X87 STORE f32 [esp+0x44] <- 0.0  (rounds to f32)
    6c2635: X87 STORE f32 [esp+0x48] <- 1.0  (rounds to f32)
    6c2639: WRITE [esp+0x24] mov= i32[esp+0x40]   ; dword ptr [esp + 0x24], edx
    6c263d: WRITE [esp+0x28] mov= i32[esp+0x44]   ; dword ptr [esp + 0x28], eax
    6c2645: WRITE [esp+0x2c] mov= i32[esp+0x48]   ; dword ptr [esp + 0x2c], ecx

  B14 @0x6c2649:
    6c2651: CALL 0xbe4b20 mat33_from_forward  regs[eax=&[esp+0x24], ecx=&[esp+0x5c]]  stack[]
    6c265e: X87 STORE f32 [esp] <- f32[ebp+0x10]  (rounds to f32)
    6c2662: CALL 0xbe51d0 sub_be51d0  regs[]  stack[&[esp+0x60], ret(mat33_from_forward)]
    6c2670: CALL 0x6c78b0 sub_6c78b0  regs[ecx=&[esp+0x30]]  stack[&[esp+0x30], 0x0]
    6c267d: CALL 0x6c7b90 sub_6c7b90  regs[]  stack[&[esp+0x64], &[esp+0x64]]
    6c2682: READ  eax <- i32[0x252a6d0]
    6c2687: WRITE [edi+0x3d8] mov= i32[0x252a6d0]   ; dword ptr [edi + 0x3d8], eax
    6c268d: READ  ecx <- i32[0x252a6d4]
    6c2697: WRITE [edi+0x3dc] mov= i32[0x252a6d4]   ; dword ptr [edi + 0x3dc], ecx
    6c269d: READ  edx <- i32[0x252a6d8]
    6c26a5: WRITE [edi+0x3e0] mov= i32[0x252a6d8]   ; dword ptr [edi + 0x3e0], edx
    6c26b9: CALL 0x9a0b00 object_teleport  regs[ecx=&[esp+0x80], edx=&[esp+0x54]]  stack[0x0, 0x1, 0x1, 0x0]

  B15 @0x6c26c1:
    6c26d0: if (i32[ebp+0x14] == 0) goto B17  else fallthrough   ; je 0x6c26d8

  B16 @0x6c26d2:
    6c26d6: goto B18

  B17 @0x6c26d8:

  B18 @0x6c26da:
    6c26db: CALL 0x55a930 inventory_equip_slot  regs[]  stack[]
    6c26e3: WRITE [edi+0x10a4] mov= i32[ebp+0x18]   ; dword ptr [edi + 0x10a4], ecx
    6c26ee: if (i32[ebp+0x1c] == 0) goto B20  else fallthrough   ; je 0x6c26f6

  B19 @0x6c26f0:
    6c26f4: if (i32[eax+0x14] != 0x0) goto B21  else fallthrough   ; jne 0x6c26f8

  B20 @0x6c26f6:

  B21 @0x6c26f8:
    6c26f8: WRITE [edi+0x10a8] mov= eax   ; dword ptr [edi + 0x10a8], eax
    6c2701: WRITE [edi+0x10ac] mov= i32[ebp+0x20]   ; dword ptr [edi + 0x10ac], edx
    6c270a: WRITE [edi+0x10b0] mov= i32[ebp+0x24]   ; dword ptr [edi + 0x10b0], eax
    6c2714: if (i32[ebp+0x2c] != 0x1) goto B23  else fallthrough   ; jne 0x6c2731

  B22 @0x6c2716:
    6c271c: X87 STORE f32 [esp+0x20] <- f32[ebp+0x30]  (rounds to f32)
    6c2724: X87 STORE f32 [esp+0x8] <- f32[esp+0x20]  (rounds to f32)
    6c2728: X87 STORE f32 [esp+0x4] <- f32[esp+0x20]  (rounds to f32)
    6c272c: goto B44

  B23 @0x6c2731:
    6c2735: if (i32[ebp+0x28] != 0x1) goto B32  else fallthrough   ; jne 0x6c27c2

  B24 @0x6c273b:
    6c2740: WRITE [edi+0x1074] mov= 0x1   ; byte ptr [edi + 0x1074], 1
    6c2747: if (i32[edi+0x44] == 0) goto B45  else fallthrough   ; je 0x6c28b3

  B25 @0x6c274d:
    6c275f: if (i32[eax+0x2149c64] == 0) goto B45  else fallthrough   ; je 0x6c28b3

  B26 @0x6c2765:
    6c2768: if (i32[eax+0x44] != ecx) goto B45  else fallthrough   ; jne 0x6c28b3

  B27 @0x6c276e:
    6c2772: if (i32[eax+0x4c] != 0x1) goto B45  else fallthrough   ; jne 0x6c28b3

  B28 @0x6c2778:
    6c277f: if (i32[eax+0x3cc] != 0x1) goto B45  else fallthrough   ; jne 0x6c28b3

  B29 @0x6c2785:
    6c278b: X87 STORE f32 [esp+0x18] <- f32[edi+0x3d0]  (rounds to f32)
    6c2799: X87 COMPARE f32[esp+0x18]  vs  f32[0xdd1b58] =f32 0.75 (0x3f400000)   [fcom]
    6c27a2: if ((f32[esp+0x18] > f32[0xdd1b58] =f32 0.75 (0x3f400000)) OR (f32[esp+0x18] == f32[0xdd1b58] =f32 0.75 (0x3f400000)) OR UNORDERED(NaN)) goto B31  else fallthrough   ; jp 0x6c27b2

  B30 @0x6c27a4:
    6c27aa: X87 STORE f32 [esp+0x18] <- (f32[esp+0x18] - f64[0xe5bfc8] =f64 0.25 (0x3fd0000000000000))  (rounds to f32)

  B31 @0x6c27b2:
    6c27b5: X87 STORE f32 [esp+0x8] <- f32[esp+0x18]  (rounds to f32)
    6c27b9: X87 STORE f32 [esp+0x4] <- f32[esp+0x18]  (rounds to f32)
    6c27bd: goto B44

  B32 @0x6c27c2:
    6c27c6: if (i32[ebp+0x2a] != 0x0) goto B36  else fallthrough   ; jne 0x6c27e0

  B33 @0x6c27c8:
    6c27cc: if (i32[ebp+0x29] != 0x0) goto B36  else fallthrough   ; jne 0x6c27e0

  B34 @0x6c27ce:
    6c27d2: if (i32[ebp+0x2b] != 0x0) goto B36  else fallthrough   ; jne 0x6c27e0

  B35 @0x6c27d4:
    6c27d4: WRITE [edi+0x1074] mov= 0x0   ; byte ptr [edi + 0x1074], 0
    6c27db: goto B45

  B36 @0x6c27e0:
    6c27e5: WRITE [edi+0x1074] mov= 0x1   ; byte ptr [edi + 0x1074], 1
    6c27ec: if (i32[edi+0x44] == 0) goto B45  else fallthrough   ; je 0x6c28b3

  B37 @0x6c27f2:
    6c2805: if (i32[edx+0x2149c64] == 0) goto B45  else fallthrough   ; je 0x6c28b3

  B38 @0x6c280b:
    6c280e: if (i32[eax+0x44] != ecx) goto B45  else fallthrough   ; jne 0x6c28b3

  B39 @0x6c2814:
    6c2818: if (i32[eax+0x4c] != 0x1) goto B45  else fallthrough   ; jne 0x6c28b3

  B40 @0x6c281e:
    6c2825: if (i32[eax+0x3cc] != 0x1) goto B45  else fallthrough   ; jne 0x6c28b3

  B41 @0x6c282b:
    6c2831: X87 STORE f32 [esp+0x18] <- f32[edi+0x3d0]  (rounds to f32)
    6c283f: X87 COMPARE f32[esp+0x18]  vs  f32[0xdd1b58] =f32 0.75 (0x3f400000)   [fcom]
    6c2848: if ((f32[esp+0x18] > f32[0xdd1b58] =f32 0.75 (0x3f400000)) OR (f32[esp+0x18] == f32[0xdd1b58] =f32 0.75 (0x3f400000)) OR UNORDERED(NaN)) goto B43  else fallthrough   ; jp 0x6c2858

  B42 @0x6c284a:
    6c2850: X87 STORE f32 [esp+0x18] <- (f32[esp+0x18] - f64[0xe5bfc8] =f64 0.25 (0x3fd0000000000000))  (rounds to f32)

  B43 @0x6c2858:
    6c2864: WRITE [esp+0x14] mov= i32[ebp+0x2a]   ; dword ptr [esp + 0x14], eax
    6c2871: X87 STORE f32 [esp+0x20] <- ((float)i32[esp+0x20] * f32[esp+0x18])  (rounds to f32)
    6c2879: WRITE [esp+0x20] mov= i32[ebp+0x29]   ; dword ptr [esp + 0x20], ecx
    6c287d: X87 STORE f32 [esp+0x8] <- f32[esp+0x20]  (rounds to f32)
    6c2887: X87 STORE f32 [esp+0x20] <- ((float)i32[esp+0x20] * f32[esp+0x18])  (rounds to f32)
    6c288f: WRITE [esp+0x20] mov= i32[ebp+0x2b]   ; dword ptr [esp + 0x20], edx
    6c2893: X87 STORE f32 [esp+0x4] <- f32[esp+0x20]  (rounds to f32)
    6c289d: X87 STORE f32 [esp+0x20] <- (f32[esp+0x18] * (float)i32[esp+0x20])  (rounds to f32)

  B44 @0x6c28a5:
    6c28ab: X87 STORE f32 [esp] <- f32[esp+0x20]  (rounds to f32)
    6c28ae: CALL 0x95be10 sub_95be10  regs[esi=&[edi+0x106c]]  stack[]

  B45 @0x6c28b3:
    6c28c2: CALL 0x926860 misc_packet_send_14  regs[ecx=edi]  stack[i32[ebp+0x34], 0x1, 0x0]
    6c28ce: if (i32[ebp+0x35] != 0x1) goto B47  else fallthrough   ; jne 0x6c28db

  B46 @0x6c28d0:
    6c28d6: CALL 0x55a930 inventory_equip_slot  regs[]  stack[0xb, 0x0]

  B47 @0x6c28db:
    6c28de: WRITE [edi+0x1089] mov= i32[ebp+0x38]   ; byte ptr [edi + 0x1089], cl
    6c28e8: if (i32[ebp+0x38] != 0x0) goto B56  else fallthrough   ; jne 0x6c2980

  B48 @0x6c28ee:
    6c28f3: if (i32[edi+0x44] == 0) goto B53  else fallthrough   ; je 0x6c291d

  B49 @0x6c28f5:
    6c2908: if (i32[edx+0x2149c64] == 0) goto B53  else fallthrough   ; je 0x6c291d

  B50 @0x6c290a:
    6c290d: if (i32[eax+0x44] != ecx) goto B53  else fallthrough   ; jne 0x6c291d

  B51 @0x6c290f:
    6c2912: if (i32[eax+0x4c] != esi) goto B53  else fallthrough   ; jne 0x6c291d

  B52 @0x6c2914:
    6c291b: if (i32[eax+0x3cc] == 0x1) goto B55  else fallthrough   ; je 0x6c296d

  B53 @0x6c291d:
    6c292a: if ((i32[ecx+0x60] & 0x1) == 0) goto B55  else fallthrough   ; je 0x6c296d

  B54 @0x6c292c:
    6c2938: READ  ecx <- i32[0x21703d4]
    6c294c: X87 STORE f32 [esp+0x14] <- f32[ecx+0x108c]  (rounds to f32)
    6c2963: X87 STORE f32 [eax+0x1c] <- (0.0 * f64[0xe5c2a0] =f64 57.295829772949219 (0x404ca5ddc0000000))  (rounds to f32)
    6c296a: X87 STORE f32 [eax+0x20] <- f32[esp+0x14]  (rounds to f32)

  B55 @0x6c296d:
    6c2970: X87 STORE f32 [edi+0x108c] <- 0.0  (rounds to f32)
    6c2976: CALL 0x9a5ad0 sub_9a5ad0  regs[]  stack[]
    6c297e: goto B57

  B56 @0x6c2980:
    6c2982: WRITE [edi+0x1088] mov= 0x0   ; byte ptr [edi + 0x1088], 0
    6c2989: X87 STORE f32 [edi+0x1094] <- 0.0  (rounds to f32)

  B57 @0x6c298f:
    6c2994: if (i32[edi+0x44] == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B58 @0x6c299a:
    6c29ad: if (i32[edx+0x2149c64] == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B59 @0x6c29b3:
    6c29b6: if (i32[eax+0x44] != ecx) goto B78  else fallthrough   ; jne 0x6c2a5a

  B60 @0x6c29bc:
    6c29bf: if (i32[eax+0x4c] != esi) goto B78  else fallthrough   ; jne 0x6c2a5a

  B61 @0x6c29c5:
    6c29cc: if (i32[eax+0x3cc] != 0x2) goto B78  else fallthrough   ; jne 0x6c2a5a

  B62 @0x6c29d2:
    6c29d4: if (ecx == 0) goto B66  else fallthrough   ; je 0x6c29f4

  B63 @0x6c29d6:
    6c29e8: if (i32[eax+0x2149c64] == 0) goto B66  else fallthrough   ; je 0x6c29f4

  B64 @0x6c29ea:
    6c29ed: if (i32[eax+0x44] != ecx) goto B66  else fallthrough   ; jne 0x6c29f4

  B65 @0x6c29ef:
    6c29f2: if (i32[eax+0x4c] == esi) goto B67  else fallthrough   ; je 0x6c29f8

  B66 @0x6c29f4:
    6c29f6: goto B68

  B67 @0x6c29f8:
    6c2a01: setne cl  ; i32[eax+0x3cc] != 0x2

  B68 @0x6c2a08:
    6c2a0c: if (i32[ebp+0x39] != 0x1) goto B73  else fallthrough   ; jne 0x6c2a32

  B69 @0x6c2a0e:
    6c2a10: if (eax == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B70 @0x6c2a12:
    6c2a1a: if (i32[eax+0x5e0] == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B71 @0x6c2a1c:
    6c2a23: if (i32[eax+0x26c] != 0x1) goto B78  else fallthrough   ; jne 0x6c2a5a

  B72 @0x6c2a25:
    6c2a2b: CALL 0x6e4de0 sub_6e4de0  regs[eax=i32[eax+0x268]]  stack[]
    6c2a30: goto B78

  B73 @0x6c2a32:
    6c2a36: if (i32[ebp+0x3a] != 0x1) goto B78  else fallthrough   ; jne 0x6c2a5a

  B74 @0x6c2a38:
    6c2a3a: if (eax == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B75 @0x6c2a3c:
    6c2a44: if (i32[eax+0x5e0] == 0) goto B78  else fallthrough   ; je 0x6c2a5a

  B76 @0x6c2a46:
    6c2a4d: if (i32[eax+0x26c] != 0x1) goto B78  else fallthrough   ; jne 0x6c2a5a

  B77 @0x6c2a4f:
    6c2a55: CALL 0x6e4e10 sub_6e4e10  regs[eax=i32[eax+0x268]]  stack[]

  B78 @0x6c2a5a:
    6c2a5a: WRITE [ebp+0x3c] mov= 0xffffffff   ; dword ptr [ebp + 0x3c], 0xffffffff
    6c2a6c: if (i32[edx] == 0xffffffff) goto B80  else fallthrough   ; je 0x6c2a73

  B79 @0x6c2a6e:
    6c2a6e: CALL 0xcebb60 sub_cebb60  regs[ecx=?]  stack[]

  B80 @0x6c2a73:
    6c2a76: CALL 0x6c2190 sub_6c2190  regs[edi=?]  stack[0x0]
    6c2a7f: CALL 0x6c2050 sub_6c2050  regs[]  stack[0x0]
    6c2a8c: CALL 0x6c22c0 sub_6c22c0  regs[esi=ebp]  stack[0x0]
    6c2a9f: if (i32[eax] == 0xffffffff) goto B87  else fallthrough   ; je 0x6c2b3e

  B81 @0x6c2aa5:
    6c2ab2: CALL 0xcec6c0 sub_cec6c0  regs[ecx=?]  stack[&[esp+0x40], &[esp+0x80]]
    6c2ac4: if (i32[ecx] < 0 (signed)) goto B86  else fallthrough   ; jl 0x6c2b17

  B82 @0x6c2ac6:
    6c2aef: if (je (flags from earlier instr)) goto B86  else fallthrough   ; je 0x6c2b17

  B83 @0x6c2af1:
    6c2af9: if (i32[esp+0x3c] <= 0 (signed)) goto B86  else fallthrough   ; jle 0x6c2b17

  B84 @0x6c2afb:
    6c2afb: goto B85

  B85 @0x6c2b00:
    6c2b0b: CALL 0xce7f20 sub_ce7f20  regs[ecx=i32[esp+esi*4+0x80], edx=?]  stack[0x1, 0x1]
    6c2b15: if ((esi add 0x1) < ebp) goto B85  else fallthrough   ; jl 0x6c2b00

  B86 @0x6c2b17:
    6c2b1f: WRITE [0x2529686] mov= 0x0   ; byte ptr [0x2529686], 0
    6c2b26: CALL 0xcec840 sub_cec840  regs[ecx=i32[ecx]]  stack[]
    6c2b2d: CALL 0x9a68d0 human_post_update  regs[eax=edi]  stack[]
    6c2b32: CALL 0x9a7150 humans_animation_jobs  regs[]  stack[]
    6c2b39: CALL 0x9a9010 human_ai_pre_think  regs[eax=edi]  stack[]

  B87 @0x6c2b3e:
    6c2b52: WRITE [esp+0x20] mov= (eax add 0x1)   ; dword ptr [esp + 0x20], eax
    6c2b56: if ((eax add 0x1) < i32[edx+0x108]) goto B1  else fallthrough   ; jl 0x6c2511

  B88 @0x6c2b5c:
    6c2b6b: WRITE [0x2529686] mov= 0x0   ; byte ptr [0x2529686], 0
    6c2b72: WRITE [esp+0x20] mov= 0   ; dword ptr [esp + 0x20], eax
    6c2b76: if (i32[edi+0x108] <= 0) goto B138  else fallthrough   ; jle 0x6c2ed1

  B89 @0x6c2b7c:
    6c2b7c: READ  ebx <- i32[0xdb80cc] =i32 11005140
    6c2b82: WRITE [esp+0x14] mov= eax   ; dword ptr [esp + 0x14], eax

  B90 @0x6c2b86:
    6c2b95: if (i32[ebp] == 0) goto B137  else fallthrough   ; je 0x6c2eb5

  B91 @0x6c2b9b:
    6c2ba1: WRITE [esp+0x1e] mov= 0x0   ; byte ptr [esp + 0x1e], 0
    6c2ba6: CALL 0xa52bb0 script_find_object_by_name  regs[ecx=esi]  stack[]
    6c2bb0: if (i32[esp+0x1e] == 0x0) goto B93  else fallthrough   ; je 0x6c2bba

  B92 @0x6c2bb2:
    6c2bb4: WRITE [esp+0x18] mov= eax   ; dword ptr [esp + 0x18], esi
    6c2bb8: goto B105

  B93 @0x6c2bba:
    6c2bbc: CALL 0xa275f0 script_follower_index  regs[eax=esi]  stack[]
    6c2bc3: if (ret(script_follower_index) <= 0 (signed)) goto B95  else fallthrough   ; jle 0x6c2bd0

  B94 @0x6c2bc5:
    6c2bc6: CALL 0xa27660 script_follower_get  regs[]  stack[eax]
    6c2bce: goto B103

  B95 @0x6c2bd0:
    6c2bd7: CALL 0xbdca30 string_crc32  regs[eax=&[esp+0x5c]]  stack[]
    6c2bdc: READ  edx <- i32[0x2f25254]
    6c2be6: if (i32[0x2f25254] == 0) goto B137  else fallthrough   ; je 0x6c2eb5

  B96 @0x6c2bec:

  B97 @0x6c2bf0:
    6c2bf3: if (i32[ecx+0xc] == eax) goto B101  else fallthrough   ; je 0x6c2c0c

  B98 @0x6c2bf5:
    6c2bfd: if (i32[ecx+0x17c] == edx) goto B137  else fallthrough   ; je 0x6c2eb5

  B99 @0x6c2c03:
    6c2c05: if (ecx != 0) goto B97  else fallthrough   ; jne 0x6c2bf0

  B100 @0x6c2c07:
    6c2c07: goto B137

  B101 @0x6c2c0c:
    6c2c0e: if (ecx == 0) goto B137  else fallthrough   ; je 0x6c2eb5

  B102 @0x6c2c14:
    6c2c19: CALL indirect edx  ecx=? stack[]

  B103 @0x6c2c1b:
    6c2c1d: if (eax == 0) goto B137  else fallthrough   ; je 0x6c2eb5

  B104 @0x6c2c23:
    6c2c23: WRITE [esp+0x18] mov= eax   ; dword ptr [esp + 0x18], eax

  B105 @0x6c2c29:
    6c2c2b: if (esi == 0) goto B137  else fallthrough   ; je 0x6c2eb5

  B106 @0x6c2c31:
    6c2c36: CALL 0x6c2340 sub_6c2340  regs[edi=esi]  stack[0x0]
    6c2c49: if (i32[eax] == 0xffffffff) goto B111  else fallthrough   ; je 0x6c2c96

  B107 @0x6c2c4b:
    6c2c4d: if (ecx < 0 (signed)) goto B111  else fallthrough   ; jl 0x6c2c96

  B108 @0x6c2c4f:
    6c2c76: if (je (flags from earlier instr)) goto B111  else fallthrough   ; je 0x6c2c96

  B109 @0x6c2c78:
    6c2c78: WRITE [ecx+0x251] mov= 0x0   ; byte ptr [ecx + 0x251], 0
    6c2c84: if (i32[ecx+0xc] == 0) goto B111  else fallthrough   ; je 0x6c2c96

  B110 @0x6c2c86:
    6c2c8e: CALL 0xca0bc0 _memset  regs[]  stack[ecx, 0x0, 0x300]

  B111 @0x6c2c96:
    6c2c98: CALL 0x9a68d0 human_post_update  regs[eax=esi]  stack[]
    6c2c9d: READ  ecx <- i32[0x33d6428]
    6c2cab: WRITE [0x33d6430] mov= 0x4   ; dword ptr [0x33d6430], edi
    6c2cb1: if (i32[0x33d6428] != 0x2) goto B115  else fallthrough   ; jne 0x6c2d0e

  B112 @0x6c2cb3:
    6c2cb8: CALL indirect ebx  ecx=? stack[0x33d6404]
    6c2cba: READ  edx <- i32[0x33d6428]
    6c2cc3: if (i32[0x33d6428] != 0x2) goto B114  else fallthrough   ; jne 0x6c2cff

  B113 @0x6c2cc5:
    6c2cca: CALL indirect ebx  ecx=? stack[0x33d6404]
    6c2ccc: READ  eax <- i32[0x33d6438]
    6c2cd2: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6438]]
    6c2cd8: READ  esi <- i32[0xdb80d0] =i32 11005164
    6c2cde: WRITE [0x33d6428] mov= 0x3   ; dword ptr [0x33d6428], 3
    6c2ced: CALL indirect esi  ecx=ret(indirect) stack[0x33d6404]
    6c2cf4: WRITE [0x33d6420] mov= 0x0   ; byte ptr [0x33d6420], 0
    6c2cfb: CALL indirect esi  ecx=ret(indirect) stack[0x33d6404]
    6c2cfd: goto B116

  B114 @0x6c2cff:
    6c2cff: READ  esi <- i32[0xdb80d0] =i32 11005164
    6c2d0a: CALL indirect esi  ecx=? stack[0x33d6404]
    6c2d0c: goto B116

  B115 @0x6c2d0e:
    6c2d0e: READ  esi <- i32[0xdb80d0] =i32 11005164

  B116 @0x6c2d14:
    6c2d14: READ  ecx <- i32[0x33d64f8]
    6c2d1d: WRITE [0x33d6500] mov= edi   ; dword ptr [0x33d6500], edi
    6c2d23: if (i32[0x33d64f8] != 0x2) goto B120  else fallthrough   ; jne 0x6c2d69

  B117 @0x6c2d25:
    6c2d2a: CALL indirect ebx  ecx=? stack[0x33d64d4]
    6c2d2c: READ  edx <- i32[0x33d64f8]
    6c2d35: if (i32[0x33d64f8] != 0x2) goto B119  else fallthrough   ; jne 0x6c2d62

  B118 @0x6c2d37:
    6c2d3c: CALL indirect ebx  ecx=? stack[0x33d64d4]
    6c2d3e: READ  eax <- i32[0x33d6508]
    6c2d44: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6508]]
    6c2d4a: WRITE [0x33d64f8] mov= 0x3   ; dword ptr [0x33d64f8], 3
    6c2d59: CALL indirect esi  ecx=ret(indirect) stack[0x33d64d4]
    6c2d5b: WRITE [0x33d64f0] mov= 0x0   ; byte ptr [0x33d64f0], 0

  B119 @0x6c2d62:
    6c2d67: CALL indirect esi  ecx=? stack[0x33d64d4]

  B120 @0x6c2d69:
    6c2d71: if (i32[0x252a634] <= 0) goto B125  else fallthrough   ; jle 0x6c2d91

  B121 @0x6c2d73:

  B122 @0x6c2d78:
    6c2d7c: if ((i32[esi+0x18] & 0x10) == 0) goto B124  else fallthrough   ; je 0x6c2d83

  B123 @0x6c2d7e:
    6c2d7e: CALL 0xcedf60 sub_cedf60  regs[esi=?]  stack[]

  B124 @0x6c2d83:
    6c2d8f: if ((ebp add 0x1) < i32[0x252a634]) goto B122  else fallthrough   ; jl 0x6c2d78

  B125 @0x6c2d91:
    6c2d91: READ  ecx <- i32[0x236f9fc]
    6c2d9d: if (i32[0x236f9fc] == 0) goto B127  else fallthrough   ; je 0x6c2ddb

  B126 @0x6c2d9f:
    6c2da7: WRITE [ebp] mov= 0x33d6328   ; dword ptr [ebp], 0x33d6328
    6c2dae: CALL indirect eax  ecx=? stack[]
    6c2db0: WRITE [esi+0x4] mov= edi   ; dword ptr [esi + 4], edi
    6c2db3: WRITE [esi+0x8] mov= 0x0   ; byte ptr [esi + 8], 0
    6c2db7: WRITE [ebp] mov= edi   ; dword ptr [ebp], edi
    6c2dba: WRITE [esi+0x9] mov= 0x0   ; byte ptr [esi + 9], 0
    6c2dbe: WRITE [esi+0x3c] mov= edi   ; dword ptr [esi + 0x3c], edi
    6c2dc1: WRITE [esi+0x40] mov= edi   ; dword ptr [esi + 0x40], edi
    6c2dc4: WRITE [esi+0x44] mov= edi   ; dword ptr [esi + 0x44], edi
    6c2dc7: READ  edx <- i32[0xec5b98] =i32 14959000
    6c2dd9: CALL indirect eax  ecx=0xec5b98 stack[(esi add 0xfffffff8)]

  B127 @0x6c2ddb:
    6c2ddb: READ  ecx <- i32[0x33d6428]
    6c2de4: WRITE [0x252a634] mov= edi   ; dword ptr [0x252a634], edi
    6c2def: WRITE [0x33d6430] mov= 0x3   ; dword ptr [0x33d6430], edi
    6c2df5: if (i32[0x33d6428] != 0x2) goto B131  else fallthrough   ; jne 0x6c2e4e

  B128 @0x6c2df7:
    6c2dfc: CALL indirect ebx  ecx=? stack[0x33d6404]
    6c2dfe: READ  edx <- i32[0x33d6428]
    6c2e07: if (i32[0x33d6428] != 0x2) goto B130  else fallthrough   ; jne 0x6c2e3f

  B129 @0x6c2e09:
    6c2e0e: CALL indirect ebx  ecx=? stack[0x33d6404]
    6c2e10: READ  eax <- i32[0x33d6438]
    6c2e16: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6438]]
    6c2e1c: READ  esi <- i32[0xdb80d0] =i32 11005164
    6c2e22: WRITE [0x33d6428] mov= edi   ; dword ptr [0x33d6428], edi
    6c2e2d: CALL indirect esi  ecx=ret(indirect) stack[0x33d6404]
    6c2e34: WRITE [0x33d6420] mov= 0x0   ; byte ptr [0x33d6420], 0
    6c2e3b: CALL indirect esi  ecx=ret(indirect) stack[0x33d6404]
    6c2e3d: goto B132

  B130 @0x6c2e3f:
    6c2e3f: READ  esi <- i32[0xdb80d0] =i32 11005164
    6c2e4a: CALL indirect esi  ecx=? stack[0x33d6404]
    6c2e4c: goto B132

  B131 @0x6c2e4e:
    6c2e4e: READ  esi <- i32[0xdb80d0] =i32 11005164

  B132 @0x6c2e54:
    6c2e54: READ  ecx <- i32[0x33d64f8]
    6c2e5d: WRITE [0x33d6500] mov= edi   ; dword ptr [0x33d6500], edi
    6c2e63: if (i32[0x33d64f8] != 0x2) goto B136  else fallthrough   ; jne 0x6c2ea5

  B133 @0x6c2e65:
    6c2e6a: CALL indirect ebx  ecx=? stack[0x33d64d4]
    6c2e6c: READ  edx <- i32[0x33d64f8]
    6c2e75: if (i32[0x33d64f8] != 0x2) goto B135  else fallthrough   ; jne 0x6c2e9e

  B134 @0x6c2e77:
    6c2e7c: CALL indirect ebx  ecx=? stack[0x33d64d4]
    6c2e7e: READ  eax <- i32[0x33d6508]
    6c2e84: CALL indirect [0xdb81dc]  ecx=ret(indirect) stack[i32[0x33d6508]]
    6c2e8a: WRITE [0x33d64f8] mov= edi   ; dword ptr [0x33d64f8], edi
    6c2e95: CALL indirect esi  ecx=ret(indirect) stack[0x33d64d4]
    6c2e97: WRITE [0x33d64f0] mov= 0x0   ; byte ptr [0x33d64f0], 0

  B135 @0x6c2e9e:
    6c2ea3: CALL indirect esi  ecx=? stack[0x33d64d4]

  B136 @0x6c2ea5:
    6c2ea9: CALL 0x9a9010 human_ai_pre_think  regs[eax=i32[esp+0x18]]  stack[]

  B137 @0x6c2eb5:
    6c2eb9: WRITE [esp+0x14] add= 0x7c   ; dword ptr [esp + 0x14], 0x7c
    6c2ec7: WRITE [esp+0x20] mov= (eax add 0x1)   ; dword ptr [esp + 0x20], eax
    6c2ecb: if ((eax add 0x1) < i32[edi+0x108]) goto B90  else fallthrough   ; jl 0x6c2b86

  B138 @0x6c2ed1:
    6c2ed4: WRITE [0x2529686] mov= 0x0   ; byte ptr [0x2529686], 0
    6c2ee2: RETURN

  call targets: 0x40ea20(sqrtf_wrapper), 0x55a930(inventory_equip_slot), 0x6beed0, 0x6c2050, 0x6c2190, 0x6c22c0, 0x6c2340, 0x6c78b0, 0x6c7b90, 0x6e4de0, 0x6e4e10, 0x926860(misc_packet_send_14), 0x95be10, 0x9a0b00(object_teleport), 0x9a5ad0, 0x9a68d0(human_post_update), 0x9a7150(humans_animation_jobs), 0x9a9010(human_ai_pre_think), 0xa275f0(script_follower_index), 0xa27660(script_follower_get), 0xa52bb0(script_find_object_by_name), 0xa618a0(script_object_find), 0xbdca30(string_crc32), 0xbddff0(vec3_normalize_in_place), 0xbe4b20(mat33_from_forward), 0xbe51d0, 0xca0bc0(_memset), 0xce7f20, 0xcebb60, 0xcec6c0, 0xcec840, 0xcedf60
  stack objects passed by address: &[esp+0x24] -> mat33_from_forward,vec3_normalize_in_place; &[esp+0x30] -> sub_6c78b0; &[esp+0x40] -> sub_cec6c0; &[esp+0x4c] -> sub_6beed0; &[esp+0x54] -> object_teleport; &[esp+0x5c] -> mat33_from_forward,string_crc32; &[esp+0x60] -> sub_be51d0; &[esp+0x64] -> sub_6c7b90; &[esp+0x80] -> object_teleport,sub_cec6c0
```

### camera_script_build_track — 0x49F320

**Role.** Bakes the scripted camera track (mode 4). EAX = description (arrays of 0x18-byte control point records at `+0x1C` (position), `+0x28`/`+0x40` (look-at / fov channels), EDI = point count). Steps: `memset(local, 0, 0x4B0)`; accumulate a float sum over the strided array into `[esp+0x24]` (B2 / B5 loops, f32 round trip each add); for each point build the Hermite-style tangent points `p + (next - prev) * 0.1` (the double constant at `0xDD50E8` = 0.10000000149011612) into three arrays 0x180 bytes apart (`[esp+0x40]`, `[esp+0x1C0]`, `[esp+0x340]`); first/last segments use one-sided differences; then sample each segment with `0xC022C0` (spline evaluate, ST0 `t`) to build an arc-length table (`ceilf` thunk `0x4912D0` + `_ftol2_sse`; the count is compared with `0x1E` at `0x49F773`: `>= 0x1E` jumps to B11, otherwise B10 recomputes it), copy baked values back into the description (`+0x0C..`, `+0x188..`, `+0x308..`) and end with the same `*0.1` tangent fix-ups for the last point.

**Port notes.** Pure function of its input; all arithmetic is x87 with f32 store points marked `X87 STORE f32` below (the `0.1` factor is a *double* multiplied with an f32 and rounded on store). Loop-carried accumulators at `[esp+0x24]`, `[esp+0x28]` are f32 round trips.


**Block map (generated):**

```
FUNCTION 0x0049F320  size 0x976  insns 625  blocks 19
  entry regs read before write: ['eax']

  B0 @0x49f320:
    49f33a: CALL 0xca0bc0 _memset  regs[]  stack[0x0, 0x4b0]
    49f348: X87 STORE f32 [esp+0x30] <- 0.0  (rounds to f32)
    49f357: if (&[edi-0x1] < 0x4) goto B3  else fallthrough   ; jl 0x49f39c

  B1 @0x49f359:

  B2 @0x49f369:
    49f375: X87 STORE f32 [esp+0x28] <- (f32[eax] + f32[esp+0x24])  (rounds to f32)
    49f380: X87 STORE f32 [esp+0x28] <- (f32[esp+0x28] + f32[eax-0xc])  (rounds to f32)
    49f38b: X87 STORE f32 [esp+0x28] <- (f32[esp+0x28] + f32[eax-0x8])  (rounds to f32)
    49f396: X87 STORE f32 [esp+0x24] <- (f32[esp+0x28] + f32[eax-0x4])  (rounds to f32)
    49f39a: if (jne (flags from earlier instr)) goto B2  else fallthrough   ; jne 0x49f369

  B3 @0x49f39c:
    49f39e: if (ebx >= ecx) goto B6  else fallthrough   ; jge 0x49f3b4

  B4 @0x49f3a0:

  B5 @0x49f3a2:
    49f3ae: X87 STORE f32 [esp+0x24] <- (f32[eax] + f32[esp+0x24])  (rounds to f32)
    49f3b2: if (jne (flags from earlier instr)) goto B5  else fallthrough   ; jne 0x49f3a2

  B6 @0x49f3b4:
    49f3c0: X87 STORE f32 [esp+0x18] <- (f32[eax+0x1c] - f32[ecx+0x1c])  (rounds to f32)
    49f3ca: X87 STORE f32 [esp+0x1c] <- (f32[eax+0x20] - f32[ecx+0x20])  (rounds to f32)
    49f3d4: X87 STORE f32 [esp+0x20] <- (f32[eax+0x24] - f32[ecx+0x24])  (rounds to f32)
    49f3e6: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f3f0: X87 STORE f32 [esp+0x10] <- (f32[esp+0x1c] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f3fa: X87 STORE f32 [esp+0x14] <- (f32[esp+0x20] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f405: X87 STORE f32 [esp+0x18] <- (f32[eax+0x1c] + f32[esp+0xc])  (rounds to f32)
    49f414: X87 STORE f32 [esp+0x1c] <- (f32[eax+0x20] + f32[esp+0x10])  (rounds to f32)
    49f41b: WRITE [esp+0x40] mov= i32[esp+0x18]   ; dword ptr [esp + 0x40], edx
    49f427: WRITE [esp+0x44] mov= i32[esp+0x1c]   ; dword ptr [esp + 0x44], edx
    49f42b: X87 STORE f32 [esp+0x20] <- (f32[eax+0x24] + f32[esp+0x14])  (rounds to f32)
    49f433: WRITE [esp+0x48] mov= i32[esp+0x20]   ; dword ptr [esp + 0x48], edx
    49f43d: X87 STORE f32 [esp+0xc] <- (f32[eax+0x40] - f32[ecx+0x40])  (rounds to f32)
    49f447: X87 STORE f32 [esp+0x10] <- (f32[eax+0x44] - f32[ecx+0x44])  (rounds to f32)
    49f451: X87 STORE f32 [esp+0x14] <- (f32[eax+0x48] - f32[ecx+0x48])  (rounds to f32)
    49f45b: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f465: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f46f: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f47a: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[eax+0x40])  (rounds to f32)
    49f489: X87 STORE f32 [esp+0x10] <- (f32[eax+0x44] + f32[esp+0x1c])  (rounds to f32)
    49f490: WRITE [esp+0x1c0] mov= i32[esp+0xc]   ; dword ptr [esp + 0x1c0], edx
    49f49f: WRITE [esp+0x1c4] mov= i32[esp+0x10]   ; dword ptr [esp + 0x1c4], edx
    49f4a6: X87 STORE f32 [esp+0x14] <- (f32[eax+0x48] + f32[esp+0x20])  (rounds to f32)
    49f4ae: WRITE [esp+0x1c8] mov= i32[esp+0x14]   ; dword ptr [esp + 0x1c8], edx
    49f4bb: X87 STORE f32 [esp+0xc] <- (f32[eax+0x28] - f32[ecx+0x28])  (rounds to f32)
    49f4c5: X87 STORE f32 [esp+0x10] <- (f32[eax+0x2c] - f32[ecx+0x2c])  (rounds to f32)
    49f4cf: X87 STORE f32 [esp+0x14] <- (f32[eax+0x30] - f32[ecx+0x30])  (rounds to f32)
    49f4d9: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f4e3: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f4ed: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f4f8: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[eax+0x28])  (rounds to f32)
    49f503: X87 STORE f32 [esp+0x10] <- (f32[eax+0x2c] + f32[esp+0x1c])  (rounds to f32)
    49f516: WRITE [esp+0x340] mov= i32[esp+0xc]   ; dword ptr [esp + 0x340], eax
    49f51d: WRITE [esp+0x344] mov= i32[esp+0x10]   ; dword ptr [esp + 0x344], ecx
    49f524: X87 STORE f32 [esp+0x14] <- (f32[eax+0x30] + f32[esp+0x20])  (rounds to f32)
    49f52c: WRITE [esp+0x348] mov= i32[esp+0x14]   ; dword ptr [esp + 0x348], edx
    49f537: if (edi <= 0 (signed)) goto B9  else fallthrough   ; jle 0x49f59c

  B7 @0x49f539:

  B8 @0x49f53b:
    49f542: WRITE [esp+eax+0x4c] mov= i32[ecx+0x1c]   ; dword ptr [esp + eax + 0x4c], ebx
    49f549: WRITE [esp+eax+0x50] mov= i32[ecx+0x20]   ; dword ptr [esp + eax + 0x50], ebx
    49f550: WRITE [esp+eax+0x54] mov= i32[ecx+0x24]   ; dword ptr [esp + eax + 0x54], ebx
    49f557: WRITE [esp+eax+0x1cc] mov= i32[ecx+0x40]   ; dword ptr [esp + eax + 0x1cc], ebx
    49f561: WRITE [esp+eax+0x1d0] mov= i32[ecx+0x44]   ; dword ptr [esp + eax + 0x1d0], ebx
    49f56e: WRITE [esp+eax+0x1d4] mov= i32[ecx+0x48]   ; dword ptr [esp + eax + 0x1d4], ebx
    49f577: WRITE [esp+eax+0x34c] mov= i32[ecx]   ; dword ptr [esp + eax + 0x34c], ebx
    49f581: WRITE [esp+eax+0x350] mov= i32[ecx+0x4]   ; dword ptr [esp + eax + 0x350], ebx
    49f58b: WRITE [esp+eax+0x354] mov= i32[ecx+0x8]   ; dword ptr [esp + eax + 0x354], ecx
    49f59a: if ((edx add 0x1) < edi) goto B8  else fallthrough   ; jl 0x49f53b

  B9 @0x49f59c:
    49f5b1: X87 STORE f32 [esp+0xc] <- (f32[ecx+0x1c] - f32[eax+0x1c])  (rounds to f32)
    49f5bb: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x20] - f32[eax+0x20])  (rounds to f32)
    49f5c5: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x24] - f32[eax+0x24])  (rounds to f32)
    49f5cf: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f5d9: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f5e3: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f5ee: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[ecx+0x1c])  (rounds to f32)
    49f5fd: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x20] + f32[esp+0x1c])  (rounds to f32)
    49f60c: WRITE [esp+ebx+0x4c] mov= i32[esp+0xc]   ; dword ptr [esp + ebx + 0x4c], edx
    49f610: WRITE [esp+ebx+0x50] mov= i32[esp+0x10]   ; dword ptr [esp + ebx + 0x50], ecx
    49f618: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x24] + f32[esp+0x20])  (rounds to f32)
    49f620: WRITE [esp+ebx+0x54] mov= i32[esp+0x14]   ; dword ptr [esp + ebx + 0x54], edx
    49f62a: X87 STORE f32 [esp+0xc] <- (f32[ecx+0x40] - f32[eax+0x40])  (rounds to f32)
    49f634: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x44] - f32[eax+0x44])  (rounds to f32)
    49f63e: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x48] - f32[eax+0x48])  (rounds to f32)
    49f648: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f652: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f65c: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f667: X87 STORE f32 [esp+0xc] <- (f32[ecx+0x40] + f32[esp+0x18])  (rounds to f32)
    49f672: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x44] + f32[esp+0x1c])  (rounds to f32)
    49f685: WRITE [esp+ebx+0x1cc] mov= i32[esp+0xc]   ; dword ptr [esp + ebx + 0x1cc], ecx
    49f68c: WRITE [esp+ebx+0x1d0] mov= i32[esp+0x10]   ; dword ptr [esp + ebx + 0x1d0], edx
    49f693: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x48] + f32[esp+0x20])  (rounds to f32)
    49f69b: WRITE [esp+ebx+0x1d4] mov= i32[esp+0x14]   ; dword ptr [esp + ebx + 0x1d4], ecx
    49f6ac: X87 STORE f32 [esp+0xc] <- (f32[ecx+0x28] - f32[eax+0x28])  (rounds to f32)
    49f6b6: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x2c] - f32[eax+0x2c])  (rounds to f32)
    49f6c0: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x30] - f32[eax+0x30])  (rounds to f32)
    49f6ca: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f6d4: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f6dc: X87 STORE f32 [esp+0x20] <- (f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000) * f32[esp+0x14])  (rounds to f32)
    49f6e7: X87 STORE f32 [esp+0xc] <- (f32[ecx+0x28] + f32[esp+0x18])  (rounds to f32)
    49f6f6: X87 STORE f32 [esp+0x10] <- (f32[ecx+0x2c] + f32[esp+0x1c])  (rounds to f32)
    49f701: WRITE [esp+ebx+0x34c] mov= i32[esp+0xc]   ; dword ptr [esp + ebx + 0x34c], edx
    49f70c: WRITE [esp+ebx+0x350] mov= i32[esp+0x10]   ; dword ptr [esp + ebx + 0x350], eax
    49f71a: X87 STORE f32 [esp+0x14] <- (f32[ecx+0x30] + f32[esp+0x20])  (rounds to f32)
    49f722: WRITE [esp+ebx+0x354] mov= i32[esp+0x14]   ; dword ptr [esp + ebx + 0x354], ecx
    49f729: X87 STORE f32 [esp+0x14] <- 0.0  (rounds to f32)
    49f734: X87 STORE f32 [esp+0x20] <- 0.0  (rounds to f32)
    49f738: WRITE [esp+0xc] mov= &[esp+0x40]   ; dword ptr [esp + 0xc], edx
    49f73c: X87 STORE f32 [esp+0x3c] <- 0.0  (rounds to f32)
    49f74c: X87 STORE f32 [esp] <- f32[esp+0x24]  (rounds to f32)
    49f74f: WRITE [esp+0x14] mov= &[edi+0x2]   ; dword ptr [esp + 0x14], eax
    49f753: WRITE [esp+0x20] mov= &[edi+0x2]   ; dword ptr [esp + 0x20], eax
    49f757: WRITE [esp+0x1c] mov= &[esp+0x1c0]   ; dword ptr [esp + 0x1c], ecx
    49f75b: WRITE [esp+0x3c] mov= &[edi+0x2]   ; dword ptr [esp + 0x3c], eax
    49f75f: WRITE [esp+0x38] mov= &[esp+0x340]   ; dword ptr [esp + 0x38], edx
    49f763: CALL 0x4912d0 sub_4912d0  regs[]  stack[&[esp+0x1c0]]
    49f76b: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    49f773: if (ret(_ftol2_sse) >= 0x1e) goto B11  else fallthrough   ; jge 0x49f78c

  B10 @0x49f775:
    49f77a: X87 STORE f32 [esp] <- f32[esp+0x24]  (rounds to f32)
    49f77d: CALL 0x4912d0 sub_4912d0  regs[]  stack[ecx]
    49f785: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    49f78a: goto B12

  B11 @0x49f78c:

  B12 @0x49f791:
    49f798: WRITE [esp+0x28] mov= &[eax-0x1]   ; dword ptr [esp + 0x28], ecx
    49f7a7: WRITE [esi] mov= eax   ; dword ptr [esi], eax
    49f7a9: WRITE [esp+0x28] mov= 0x0   ; dword ptr [esp + 0x28], 0
    49f7b1: X87 STORE f32 [esi+0x4] <- (f32[esp+0x24] / i32[esp+0x28])  (rounds to f32)
    49f7b6: X87 STORE f32 [esp+0x24] <- 0.0  (rounds to f32)
    49f7ba: if (eax <= 0x1) goto B18  else fallthrough   ; jle 0x49f88e

  B13 @0x49f7c0:
    49f7d3: WRITE [esp+0x2c] mov= &[esi+0x194]   ; dword ptr [esp + 0x2c], edx

  B14 @0x49f7d7:
    49f7ec: X87 STORE f32 [esp+0x34] <- ((f32[esp+0x24] / f32[edi]) + i32[esp+0x2c])  (rounds to f32)
    49f7fa: X87 STORE f32 [esp+0x34] <- (f32[esp+0x34] + f64[0xe5bf40] =f64 1 (0x3ff0000000000000))  (rounds to f32)
    49f802: X87 STORE f32 [esp] <- f32[esp+0x34]  (rounds to f32)
    49f805: CALL 0xc022c0 sub_c022c0  regs[eax=&[esp+0x10], edx=(edx add 0xfffffe80)]  stack[ecx]
    49f817: X87 STORE f32 [esp] <- f32[esp+0x30]  (rounds to f32)
    49f81a: CALL 0xc022c0 sub_c022c0  regs[eax=&[esp+0x1c], edx=i32[esp+0x2c]]  stack[ret(sub_c022c0)]
    49f82a: X87 STORE f32 [esp] <- f32[esp+0x30]  (rounds to f32)
    49f831: CALL 0xc022c0 sub_c022c0  regs[eax=&[esp+0x38], edx=(edx add 0x180)]  stack[ret(sub_c022c0)]
    49f83d: X87 STORE f32 [esp+0x24] <- (f32[esi+0x4] + f32[esp+0x24])  (rounds to f32)
    49f847: X87 COMPARE f32[edi]  vs  f32[esp+0x24]   [fcomp]
    49f84e: if ((f32[edi] > f32[esp+0x24]) OR (f32[edi] == f32[esp+0x24]) OR UNORDERED(NaN)) goto B16  else fallthrough   ; jp 0x49f87a

  B15 @0x49f850:
    49f860: X87 STORE f32 [esp+0x24] <- (f32[esp+0x24] - f32[edi])  (rounds to f32)
    49f86b: WRITE [esp+0x28] mov= (eax add 0x1)   ; dword ptr [esp + 0x28], eax
    49f871: X87 COMPARE f32[edi]  vs  f32[esp+0x24]   [fcomp]
    49f878: if ((f32[edi] < f32[esp+0x24])) goto B15  else fallthrough   ; jnp 0x49f850

  B16 @0x49f87a:
    49f87a: WRITE [esp+0x2c] add= 0xc   ; dword ptr [esp + 0x2c], 0xc
    49f884: if ((ebp add 0x1) < i32[esi]) goto B14  else fallthrough   ; jl 0x49f7d7

  B17 @0x49f88a:

  B18 @0x49f88e:
    49f896: WRITE [esi+edx*4+0x8] mov= i32[esp+ebx+0x40]   ; dword ptr [esi + edx*4 + 8], ecx
    49f8a5: WRITE [eax+0xc] mov= i32[esp+ebx+0x44]   ; dword ptr [eax + 0xc], edx
    49f8af: WRITE [eax+0x188] mov= i32[esp+ebx+0x1c0]   ; dword ptr [eax + 0x188], edx
    49f8bc: WRITE [eax+0x10] mov= i32[esp+ebx+0x48]   ; dword ptr [eax + 0x10], ecx
    49f8c6: WRITE [eax+0x18c] mov= i32[esp+ebx+0x1c4]   ; dword ptr [eax + 0x18c], ecx
    49f8cc: WRITE [eax+0x190] mov= i32[esp+ebx+0x1c8]   ; dword ptr [eax + 0x190], edx
    49f8d9: WRITE [eax+0x308] mov= i32[esp+ebx+0x340]   ; dword ptr [eax + 0x308], edx
    49f8e6: WRITE [eax+0x30c] mov= i32[esp+ebx+0x344]   ; dword ptr [eax + 0x30c], edx
    49f8f6: WRITE [eax+0x310] mov= i32[ecx+0x8]   ; dword ptr [eax + 0x310], ecx
    49f905: X87 STORE f32 [esp+0xc] <- (f32[esi+0x14] - f32[esi+0x20])  (rounds to f32)
    49f90f: X87 STORE f32 [esp+0x10] <- (f32[esi+0x18] - f32[esi+0x24])  (rounds to f32)
    49f919: X87 STORE f32 [esp+0x14] <- (f32[esi+0x1c] - f32[esi+0x28])  (rounds to f32)
    49f92b: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f935: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f93f: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f94a: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[esi+0x14])  (rounds to f32)
    49f959: X87 STORE f32 [esp+0x10] <- (f32[esi+0x18] + f32[esp+0x1c])  (rounds to f32)
    49f960: WRITE [ecx] mov= i32[esp+0xc]   ; dword ptr [ecx], edx
    49f96a: WRITE [ecx+0x4] mov= i32[esp+0x10]   ; dword ptr [ecx + 4], edx
    49f96d: X87 STORE f32 [esp+0x14] <- (f32[esi+0x1c] + f32[esp+0x20])  (rounds to f32)
    49f975: WRITE [ecx+0x8] mov= i32[esp+0x14]   ; dword ptr [ecx + 8], edx
    49f984: X87 STORE f32 [esp+0xc] <- (f32[esi+0x194] - f32[esi+0x1a0])  (rounds to f32)
    49f994: X87 STORE f32 [esp+0x10] <- (f32[esi+0x198] - f32[esi+0x1a4])  (rounds to f32)
    49f9a4: X87 STORE f32 [esp+0x14] <- (f32[esi+0x19c] - f32[esi+0x1a8])  (rounds to f32)
    49f9ae: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f9b8: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f9c2: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49f9d0: X87 STORE f32 [esp+0xc] <- (f32[esi+0x194] + f32[esp+0x18])  (rounds to f32)
    49f9de: X87 STORE f32 [esp+0x10] <- (f32[esi+0x198] + f32[esp+0x1c])  (rounds to f32)
    49f9f0: WRITE [esi+0x188] mov= i32[esp+0xc]   ; dword ptr [esi + 0x188], edi
    49f9fa: WRITE [esi+0x18c] mov= i32[esp+0x10]   ; dword ptr [esi + 0x18c], edi
    49fa00: X87 STORE f32 [esp+0x14] <- (f32[esi+0x19c] + f32[esp+0x20])  (rounds to f32)
    49fa08: WRITE [esi+0x190] mov= i32[esp+0x14]   ; dword ptr [esi + 0x190], edi
    49fa26: X87 STORE f32 [esp+0xc] <- (f32[esi+0x314] - f32[esi+0x320])  (rounds to f32)
    49fa36: X87 STORE f32 [esp+0x10] <- (f32[esi+0x318] - f32[esi+0x324])  (rounds to f32)
    49fa46: X87 STORE f32 [esp+0x14] <- (f32[esi+0x31c] - f32[esi+0x328])  (rounds to f32)
    49fa50: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fa5a: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fa64: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fa72: X87 STORE f32 [esp+0xc] <- (f32[esi+0x314] + f32[esp+0x18])  (rounds to f32)
    49fa84: X87 STORE f32 [esp+0x10] <- (f32[esi+0x318] + f32[esp+0x1c])  (rounds to f32)
    49fa8e: WRITE [edi] mov= i32[esp+0xc]   ; dword ptr [edi], ebx
    49fa98: WRITE [edi+0x4] mov= i32[esp+0x10]   ; dword ptr [edi + 4], ebx
    49fa9b: X87 STORE f32 [esp+0x14] <- (f32[esi+0x31c] + f32[esp+0x20])  (rounds to f32)
    49faa3: WRITE [edi+0x8] mov= i32[esp+0x14]   ; dword ptr [edi + 8], ebx
    49faac: X87 STORE f32 [esp+0xc] <- (f32[eax+0x8] - f32[eax-0x4])  (rounds to f32)
    49fab5: X87 STORE f32 [esp+0x10] <- (f32[eax+0xc] - f32[eax])  (rounds to f32)
    49fabf: X87 STORE f32 [esp+0x14] <- (f32[eax+0x10] - f32[eax+0x4])  (rounds to f32)
    49fac9: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fad3: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fadd: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fae8: X87 STORE f32 [esp+0xc] <- (f32[eax+0x8] + f32[esp+0x18])  (rounds to f32)
    49faf7: X87 STORE f32 [esp+0x10] <- (f32[eax+0xc] + f32[esp+0x1c])  (rounds to f32)
    49fafe: WRITE [eax+0x14] mov= i32[esp+0xc]   ; dword ptr [eax + 0x14], ebx
    49fb09: WRITE [eax+0x18] mov= i32[esp+0x10]   ; dword ptr [eax + 0x18], ebx
    49fb0c: X87 STORE f32 [esp+0x14] <- (f32[eax+0x10] + f32[esp+0x20])  (rounds to f32)
    49fb14: WRITE [eax+0x1c] mov= i32[esp+0x14]   ; dword ptr [eax + 0x1c], ebx
    49fb23: X87 STORE f32 [esp+0xc] <- (f32[eax+0x188] - f32[eax+0x17c])  (rounds to f32)
    49fb33: X87 STORE f32 [esp+0x10] <- (f32[eax+0x18c] - f32[eax+0x180])  (rounds to f32)
    49fb43: X87 STORE f32 [esp+0x14] <- (f32[eax+0x190] - f32[eax+0x184])  (rounds to f32)
    49fb4d: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fb57: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fb61: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fb6f: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[eax+0x188])  (rounds to f32)
    49fb81: X87 STORE f32 [esp+0x10] <- (f32[eax+0x18c] + f32[esp+0x1c])  (rounds to f32)
    49fb8b: WRITE [eax+0x194] mov= i32[esp+0xc]   ; dword ptr [eax + 0x194], ebx
    49fb99: WRITE [eax+0x198] mov= i32[esp+0x10]   ; dword ptr [eax + 0x198], ebx
    49fb9f: X87 STORE f32 [esp+0x14] <- (f32[eax+0x190] + f32[esp+0x20])  (rounds to f32)
    49fba7: WRITE [eax+0x19c] mov= i32[esp+0x14]   ; dword ptr [eax + 0x19c], ebx
    49fbb9: X87 STORE f32 [esp+0xc] <- (f32[eax+0x308] - f32[eax+0x2fc])  (rounds to f32)
    49fbc9: X87 STORE f32 [esp+0x10] <- (f32[eax+0x30c] - f32[eax+0x300])  (rounds to f32)
    49fbd9: X87 STORE f32 [esp+0x14] <- (f32[eax+0x310] - f32[eax+0x304])  (rounds to f32)
    49fbe3: X87 STORE f32 [esp+0x18] <- (f32[esp+0xc] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fbed: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x10] * f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000))  (rounds to f32)
    49fbf5: X87 STORE f32 [esp+0x20] <- (f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000) * f32[esp+0x14])  (rounds to f32)
    49fc03: X87 STORE f32 [esp+0xc] <- (f32[esp+0x18] + f32[eax+0x308])  (rounds to f32)
    49fc15: X87 STORE f32 [esp+0x10] <- (f32[eax+0x30c] + f32[esp+0x1c])  (rounds to f32)
    49fc1f: WRITE [eax+0x314] mov= i32[esp+0xc]   ; dword ptr [eax + 0x314], ebx
    49fc2d: WRITE [eax+0x318] mov= i32[esp+0x10]   ; dword ptr [eax + 0x318], ebx
    49fc33: X87 STORE f32 [esp+0x14] <- (f32[eax+0x310] + f32[esp+0x20])  (rounds to f32)
    49fc3b: WRITE [eax+0x31c] mov= i32[esp+0x14]   ; dword ptr [eax + 0x31c], ebx
    49fc43: WRITE [esi+0x488] mov= &[esi+0x8]   ; dword ptr [esi + 0x488], ecx
    49fc4c: WRITE [esi+0x48c] mov= (eax add 0x2)   ; dword ptr [esi + 0x48c], eax
    49fc52: X87 STORE f32 [esi+0x490] <- 0.0  (rounds to f32)
    49fc5a: WRITE [esi+0x494] mov= &[esi+0x188]   ; dword ptr [esi + 0x494], edx
    49fc63: WRITE [esi+0x498] mov= (ecx add 0x2)   ; dword ptr [esi + 0x498], ecx
    49fc69: X87 STORE f32 [esi+0x49c] <- 0.0  (rounds to f32)
    49fc71: WRITE [esi+0x4a0] mov= &[esi+0x308]   ; dword ptr [esi + 0x4a0], edi
    49fc7b: WRITE [esi+0x4a4] mov= (edx add 0x2)   ; dword ptr [esi + 0x4a4], edx
    49fc81: X87 STORE f32 [esi+0x4a8] <- 0.0  (rounds to f32)
    49fc88: X87 STORE f32 [esi+0x4ac] <- 0.0  (rounds to f32)
    49fc95: RETURN

  call targets: 0x4912d0, 0xc022c0, 0xca0bc0(_memset), 0xda0140(_ftol2_sse)
  stack objects passed by address: &[esp+0x10] -> sub_c022c0; &[esp+0x1c0] -> sub_4912d0; &[esp+0x1c] -> sub_c022c0; &[esp+0x38] -> sub_c022c0
```

### anim_track_sample_vec3 — 0xCE3610

**Role (inferred).** Samples a vec3 from an animation object's key arrays (`obj+0x298` count, `+0x2A0/+0x2A4/+0x2A8` data) into the buffer at stack arg 1; returns zeros when the count is 0. Callers: `0xCE40B0`, `0xCE4400`, `0xCE4520`, `0xCE4610`, `0xCE47B0`, `0xCE4870`, `0xCE9E10`. Uses `0xC22F20` (key lookup), `0xCE2EA0` / `0xCE2EF0` and an eased blend ending in `sinf`. A port should verify the ease term against the original with a range of inputs.


**Block map (generated):**

```
FUNCTION 0x00CE3610  size 0x805  insns 631  blocks 40
  entry regs read before write: ['ecx']

  B0 @0xce3610:
    ce3637: WRITE [esp+0x30] mov= i32[ecx+0x2a4]   ; dword ptr [esp + 0x30], esi
    ce363b: WRITE [esp+0x34] mov= i32[ecx+0x2a8]   ; dword ptr [esp + 0x34], edx
    ce363f: if (i32[ecx+0x298] != 0) goto B2  else fallthrough   ; jne 0xce365c

  B1 @0xce3641:
    ce3647: X87 STORE f32 [eax] <- 0.0  (rounds to f32)
    ce3649: X87 STORE f32 [eax+0x4] <- 0.0  (rounds to f32)
    ce364c: X87 STORE f32 [eax+0x8] <- 0.0  (rounds to f32)
    ce3651: X87 STORE f32 [eax+0xc] <- 1.0  (rounds to f32)
    ce365b: RETURN

  B2 @0xce365c:
    ce365f: if (eax != 0x1) goto B4  else fallthrough   ; jne 0xce3766

  B3 @0xce3665:
    ce366c: CALL 0xc22f20 sub_c22f20  regs[eax=ebp]  stack[&[esp+0x14]]
    ce3699: WRITE [esp+0x38] mov= (dx add ax)   ; dword ptr [esp + 0x38], edx
    ce36b2: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x38] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce36ba: X87 STORE f32 [eax] <- f32[esp+0x34]  (rounds to f32)
    ce36da: WRITE [esp+0x34] mov= (dx add cx)   ; dword ptr [esp + 0x34], edx
    ce36e9: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x34] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce36f1: X87 STORE f32 [eax+0x4] <- f32[esp+0x34]  (rounds to f32)
    ce3711: WRITE [esp+0x34] mov= (cx add dx)   ; dword ptr [esp + 0x34], ecx
    ce3720: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x34] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3728: X87 STORE f32 [eax+0x8] <- f32[esp+0x34]  (rounds to f32)
    ce3749: WRITE [esp+0x34] mov= (dx add cx)   ; dword ptr [esp + 0x34], edx
    ce3753: X87 STORE f32 [esp+0x34] <- (f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000) * (float)i32[esp+0x34])  (rounds to f32)
    ce375b: X87 STORE f32 [eax+0xc] <- f32[esp+0x34]  (rounds to f32)
    ce3765: RETURN

  B4 @0xce3766:
    ce376f: WRITE [esp+0x17] mov= i32[ecx+0x2cc]   ; byte ptr [esp + 0x17], cl
    ce377b: if (i32[ecx+0x2cc] == 0) goto B6  else fallthrough   ; je 0xce3784

  B5 @0xce377d:
    ce377d: CALL 0xce2ea0 sub_ce2ea0  regs[eax=?, edi=?, edx=?, esi=?]  stack[]
    ce3782: goto B7

  B6 @0xce3784:
    ce3784: CALL 0xce2ef0 sub_ce2ef0  regs[eax=?, edi=?, edx=?, esi=?]  stack[]

  B7 @0xce3789:
    ce3794: if (i32[esp+0x2c] != i32[esp+0x2c]) goto B15  else fallthrough   ; jne 0xce38fd

  B8 @0xce379a:
    ce37a7: if (edi <= (ecx and 0xf)) goto B11  else fallthrough   ; jle 0xce37ca

  B9 @0xce37a9:

  B10 @0xce37b0:
    ce37c8: if ((esi add edx) > (ecx and 0xf)) goto B10  else fallthrough   ; jg 0xce37b0

  B11 @0xce37ca:
    ce37d8: CALL 0xc22f20 sub_c22f20  regs[eax=&[eax+edx]]  stack[&[esp+0x14]]
    ce37e8: WRITE [esp+0x1c] mov= ebx   ; dword ptr [esp + 0x1c], ebx
    ce37ec: WRITE [esp+0x18] mov= ebx   ; dword ptr [esp + 0x18], ebx
    ce37f0: if ((ecx sub esi) > edi) goto B14  else fallthrough   ; jg 0xce3823

  B12 @0xce37f2:

  B13 @0xce3800:
    ce380f: WRITE [esp+0x18] add= i32[eax]   ; dword ptr [esp + 0x18], ecx
    ce3817: WRITE [esp+0x1c] add= i32[eax+0x1]   ; dword ptr [esp + 0x1c], ecx
    ce3821: if (jne (flags from earlier instr)) goto B13  else fallthrough   ; jne 0xce3800

  B14 @0xce3823:
    ce388d: WRITE [esp+0x34] mov= (di add bx)   ; dword ptr [esp + 0x34], edx
    ce38a9: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x34] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce38b1: WRITE [esp+0x34] mov= ax   ; dword ptr [esp + 0x34], eax
    ce38b5: X87 STORE f32 [edx] <- f32[esp+0x34]  (rounds to f32)
    ce38c0: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x34] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce38c8: WRITE [esp+0x34] mov= cx   ; dword ptr [esp + 0x34], ecx
    ce38cc: X87 STORE f32 [edx+0x4] <- f32[esp+0x34]  (rounds to f32)
    ce38d5: X87 STORE f32 [esp+0x34] <- ((float)i32[esp+0x34] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce38dd: WRITE [esp+0x34] mov= si   ; dword ptr [esp + 0x34], eax
    ce38e1: X87 STORE f32 [edx+0x8] <- f32[esp+0x34]  (rounds to f32)
    ce38ea: X87 STORE f32 [esp+0x34] <- (f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000) * (float)i32[esp+0x34])  (rounds to f32)
    ce38f2: X87 STORE f32 [edx+0xc] <- f32[esp+0x34]  (rounds to f32)
    ce38fc: RETURN

  B15 @0xce38fd:
    ce3908: WRITE [esp+0x28] mov= edi   ; dword ptr [esp + 0x28], edi
    ce390c: WRITE [esp+0x38] mov= (eax and 0xf)   ; dword ptr [esp + 0x38], eax
    ce3910: if (edi <= (eax and 0xf)) goto B18  else fallthrough   ; jle 0xce3930

  B16 @0xce3912:
    ce392a: if ((ecx add edx) > (eax and 0xf)) goto B16  else fallthrough   ; jg 0xce3912

  B17 @0xce392c:
    ce392c: WRITE [esp+0x28] mov= ecx   ; dword ptr [esp + 0x28], ecx

  B18 @0xce3930:
    ce393e: CALL 0xc22f20 sub_c22f20  regs[eax=&[ebx+ecx]]  stack[&[esp+0x14]]
    ce3950: WRITE [esp+0x1c] mov= 0   ; dword ptr [esp + 0x1c], eax
    ce3954: WRITE [esp+0x18] mov= 0   ; dword ptr [esp + 0x18], eax
    ce3958: WRITE [esp+0x24] mov= 0   ; dword ptr [esp + 0x24], eax
    ce395c: WRITE [esp+0x20] mov= 0   ; dword ptr [esp + 0x20], eax
    ce3960: if ((ecx sub [esp+0x2c]) > edi) goto B21  else fallthrough   ; jg 0xce39a9

  B19 @0xce3962:
    ce3971: WRITE [esp+0x28] mov= (edx add 0x1)   ; dword ptr [esp + 0x28], edx
    ce3975: goto B20

  B20 @0xce3980:
    ce3988: WRITE [esp+0x20] add= i32[eax-0x2]   ; dword ptr [esp + 0x20], ecx
    ce398c: WRITE [esp+0x24] add= i32[eax-0x1]   ; dword ptr [esp + 0x24], edx
    ce3997: WRITE [esp+0x18] add= i32[eax]   ; dword ptr [esp + 0x18], ecx
    ce399b: WRITE [esp+0x1c] add= i32[eax+0x1]   ; dword ptr [esp + 0x1c], edx
    ce39a2: WRITE [esp+0x28] sub= 0x1   ; dword ptr [esp + 0x28], 1
    ce39a7: if (jne (flags from earlier instr)) goto B20  else fallthrough   ; jne 0xce3980

  B21 @0xce39a9:
    ce39cc: WRITE [esp+0x28] mov= (cx add dx)   ; dword ptr [esp + 0x28], eax
    ce39ea: X87 STORE f32 [esp+0x28] <- ((float)i32[esp+0x28] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce39f2: X87 STORE f32 [esp+0x5c] <- f32[esp+0x28]  (rounds to f32)
    ce3a0b: WRITE [esp+0x28] mov= (dx add ax)   ; dword ptr [esp + 0x28], ecx
    ce3a21: X87 STORE f32 [esp+0x28] <- ((float)i32[esp+0x28] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3a29: X87 STORE f32 [esp+0x60] <- f32[esp+0x28]  (rounds to f32)
    ce3a43: WRITE [esp+0x28] mov= (ax add cx)   ; dword ptr [esp + 0x28], edx
    ce3a58: X87 STORE f32 [esp+0x28] <- ((float)i32[esp+0x28] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3a60: X87 STORE f32 [esp+0x64] <- f32[esp+0x28]  (rounds to f32)
    ce3a7a: WRITE [esp+0x28] mov= (cx add dx)   ; dword ptr [esp + 0x28], eax
    ce3a8e: X87 STORE f32 [esp+0x28] <- ((float)i32[esp+0x28] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3a96: WRITE [esp+0x28] mov= i32[esp+0x2c]   ; dword ptr [esp + 0x28], edx
    ce3a9a: X87 STORE f32 [esp+0x68] <- f32[esp+0x28]  (rounds to f32)
    ce3a9e: if (i32[esp+0x2c] <= i32[esp+0x38]) goto B24  else fallthrough   ; jle 0xce3ac6

  B22 @0xce3aa0:
    ce3abe: WRITE [esp+0x28] mov= (ecx add eax)   ; dword ptr [esp + 0x28], ecx
    ce3ac2: if ((ecx add eax) > (eax and 0xf)) goto B22  else fallthrough   ; jg 0xce3aa0

  B23 @0xce3ac4:

  B24 @0xce3ac6:
    ce3ac8: if (esi == ebx) goto B26  else fallthrough   ; je 0xce3aeb

  B25 @0xce3aca:
    ce3ad7: CALL 0xc22f20 sub_c22f20  regs[eax=(eax add esi)]  stack[&[esp+0x14]]

  B26 @0xce3aeb:
    ce3aeb: WRITE [esp+0x1c] mov= ecx   ; dword ptr [esp + 0x1c], ecx
    ce3aef: WRITE [esp+0x18] mov= ecx   ; dword ptr [esp + 0x18], ecx
    ce3af3: WRITE [esp+0x24] mov= ecx   ; dword ptr [esp + 0x24], ecx
    ce3af7: WRITE [esp+0x20] mov= ecx   ; dword ptr [esp + 0x20], ecx
    ce3b03: if ((ecx sub [esp+0x28]) > edx) goto B30  else fallthrough   ; jg 0xce3b3f

  B27 @0xce3b05:

  B28 @0xce3b14:
    ce3b1c: WRITE [esp+0x20] add= i32[eax-0x2]   ; dword ptr [esp + 0x20], ecx
    ce3b20: WRITE [esp+0x24] add= i32[eax-0x1]   ; dword ptr [esp + 0x24], edx
    ce3b2b: WRITE [esp+0x18] add= i32[eax]   ; dword ptr [esp + 0x18], ecx
    ce3b2f: WRITE [esp+0x1c] add= i32[eax+0x1]   ; dword ptr [esp + 0x1c], edx
    ce3b39: if (jne (flags from earlier instr)) goto B28  else fallthrough   ; jne 0xce3b14

  B29 @0xce3b3b:

  B30 @0xce3b3f:
    ce3b76: WRITE [esp+0x38] mov= (si add bx)   ; dword ptr [esp + 0x38], esi
    ce3b92: X87 STORE f32 [esp+0x38] <- ((float)i32[esp+0x38] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3ba0: X87 STORE f32 [esp+0x3c] <- f32[esp+0x38]  (rounds to f32)
    ce3baa: WRITE [esp+0x38] mov= (si add cx)   ; dword ptr [esp + 0x38], ecx
    ce3bc3: X87 STORE f32 [esp+0x38] <- ((float)i32[esp+0x38] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3bd2: X87 STORE f32 [esp+0x40] <- f32[esp+0x38]  (rounds to f32)
    ce3bd9: WRITE [esp+0x38] mov= (cx add si)   ; dword ptr [esp + 0x38], ecx
    ce3bf7: X87 STORE f32 [esp+0x38] <- ((float)i32[esp+0x38] * f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000))  (rounds to f32)
    ce3bff: X87 STORE f32 [esp+0x44] <- f32[esp+0x38]  (rounds to f32)
    ce3c12: WRITE [esp+0x38] mov= (cx add ax)   ; dword ptr [esp + 0x38], ecx
    ce3c20: X87 STORE f32 [esp+0x38] <- (f64[0xe5c298] =f64 6.1038881540298462e-05 (0x3f10004000000000) * (float)i32[esp+0x38])  (rounds to f32)
    ce3c28: X87 STORE f32 [esp+0x48] <- f32[esp+0x38]  (rounds to f32)
    ce3c36: if (i32[esp+0x13] == 0x0) goto B32  else fallthrough   ; je 0xce3c54

  B31 @0xce3c38:
    ce3c40: WRITE [esp+0x38] mov= i32[ecx+edi*2]   ; dword ptr [esp + 0x38], eax
    ce3c4a: WRITE [esp+0x38] mov= (edx sub eax)   ; dword ptr [esp + 0x38], edx
    ce3c52: goto B33

  B32 @0xce3c54:
    ce3c5c: WRITE [esp+0x38] mov= i32[edi+ecx]   ; dword ptr [esp + 0x38], eax
    ce3c66: WRITE [esp+0x38] mov= (ecx sub eax)   ; dword ptr [esp + 0x38], ecx

  B33 @0xce3c6e:
    ce3c6e: X87 STORE f32 [esp+0x2c] <- ((((float)i32[esp+0x70] * f64[0xe5c418] =f64 0.012500000186264515 (0x3f899999a0000000)) - i32[esp+0x38]) / i32[esp+0x38])  (rounds to f32)
    ce3ca2: X87 STORE f32 [esp+0x30] <- ((((f32[esp+0x40] * f32[esp+0x60]) + (f32[esp+0x3c] * f32[esp+0x5c])) + (f32[esp+0x44] * f32[esp+0x64])) + (f32[esp+0x48] * f32[esp+0x68]))  (rounds to f32)
    ce3cac: X87 COMPARE f32[esp+0x30]  vs  0.0   [fcom]
    ce3cb5: if ((f32[esp+0x30] < 0.0) OR UNORDERED(NaN)) goto B35  else fallthrough   ; jne 0xce3ce1

  B34 @0xce3cb7:
    ce3cc9: WRITE [esp+0x4c] mov= i32[esp+0x3c]   ; dword ptr [esp + 0x4c], edx
    ce3cd3: WRITE [esp+0x50] mov= i32[esp+0x40]   ; dword ptr [esp + 0x50], eax
    ce3cd7: WRITE [esp+0x54] mov= i32[esp+0x44]   ; dword ptr [esp + 0x54], ecx
    ce3cdb: WRITE [esp+0x58] mov= i32[esp+0x48]   ; dword ptr [esp + 0x58], edx
    ce3cdf: goto B36

  B35 @0xce3ce1:
    ce3ce5: X87 STORE f32 [esp+0x3c] <- -f32[esp+0x3c]  (rounds to f32)
    ce3cef: WRITE [esp+0x4c] mov= i32[esp+0x3c]   ; dword ptr [esp + 0x4c], eax
    ce3cf5: X87 STORE f32 [esp+0x40] <- -f32[esp+0x40]  (rounds to f32)
    ce3cff: WRITE [esp+0x50] mov= i32[esp+0x40]   ; dword ptr [esp + 0x50], ecx
    ce3d05: X87 STORE f32 [esp+0x44] <- -f32[esp+0x44]  (rounds to f32)
    ce3d0d: WRITE [esp+0x54] mov= i32[esp+0x44]   ; dword ptr [esp + 0x54], edx
    ce3d13: X87 STORE f32 [esp+0x48] <- -f32[esp+0x48]  (rounds to f32)
    ce3d1b: WRITE [esp+0x58] mov= i32[esp+0x48]   ; dword ptr [esp + 0x58], eax
    ce3d21: X87 STORE f32 [esp+0x30] <- -f32[esp+0x30]  (rounds to f32)

  B36 @0xce3d29:
    ce3d33: X87 COMPARE (1.0 - f32[esp+0x30])  vs  f64[0xe5c420] =f64 0.050000000000000003 (0x3fa999999999999a)   [fcomp]
    ce3d3e: if (((1.0 - f32[esp+0x30]) > f64[0xe5c420] =f64 0.050000000000000003 (0x3fa999999999999a)) OR UNORDERED(NaN)) goto B38  else fallthrough   ; jp 0xce3d52

  B37 @0xce3d40:
    ce3d4c: X87 STORE f32 [esp+0x28] <- (1.0 - f32[esp+0x2c])  (rounds to f32)
    ce3d50: goto B39

  B38 @0xce3d52:
    ce3d55: X87 STORE f32 [esp] <- f32[esp+0x30]  (rounds to f32)
    ce3d58: CALL 0x42ef30 sub_42ef30  regs[]  stack[ecx]
    ce3d5d: X87 STORE f32 [esp+0x34] <- ST0<ret sub_42ef30>  (rounds to f32)
    ce3d65: X87 STORE f32 [esp] <- f32[esp+0x34]  (rounds to f32)
    ce3d68: CALL 0x428cd0 sinf_wrapper  regs[]  stack[]
    ce3d75: X87 STORE f32 [esp+0x38] <- 1.0  (rounds to f32)
    ce3d81: X87 STORE f32 [esp+0x3c] <- ST0<ret sinf_wrapper>  (rounds to f32)
    ce3d89: X87 STORE f32 [esp] <- f32[esp+0x3c]  (rounds to f32)
    ce3d8c: CALL 0x428cd0 sinf_wrapper  regs[]  stack[]
    ce3d95: X87 STORE f32 [esp+0x2c] <- ST0<ret sinf_wrapper>  (rounds to f32)
    ce3da1: X87 STORE f32 [esp+0x3c] <- (f32[esp+0x34] * f32[esp+0x30])  (rounds to f32)
    ce3da9: X87 STORE f32 [esp] <- f32[esp+0x3c]  (rounds to f32)
    ce3dac: CALL 0x428cd0 sinf_wrapper  regs[]  stack[]

  B39 @0xce3db8:
    ce3db8: X87 STORE f32 [esp+0x34] <- f32[esp+0x2c]  (rounds to f32)
    ce3dde: X87 STORE f32 [eax] <- ((f32[esp+0x4c] * f32[esp+0x30]) + (f32[esp+0x18] * f32[esp+0x4c]))  (rounds to f32)
    ce3dee: X87 STORE f32 [eax+0x4] <- ((f32[esp+0x40] * f32[esp+0x30]) + (f32[esp+0x18] * f32[esp+0x50]))  (rounds to f32)
    ce3dff: X87 STORE f32 [eax+0x8] <- ((f32[esp+0x44] * f32[esp+0x30]) + (f32[esp+0x18] * f32[esp+0x54]))  (rounds to f32)
    ce3e0e: X87 STORE f32 [eax+0xc] <- ((f32[esp+0x18] * f32[esp+0x58]) + (f32[esp+0x30] * f32[esp+0x48]))  (rounds to f32)
    ce3e14: RETURN

  call targets: 0x428cd0(sinf_wrapper), 0x42ef30, 0xc22f20, 0xce2ea0, 0xce2ef0
  stack objects passed by address: &[esp+0x14] -> sub_c22f20
```

### anim_tracks_update_frame — 0xCE7520

**Role (inferred).** Advances a rig object's tracks one frame: copies `obj+0x244 -> +0x248`, scans the track list, converts times to frame indices with `floorf` + `_ftol2_sse` and calls `0xCE5840` / `0xCE57B0`; stdcall with five stack arguments. 128 blocks; port last.


**Block map (generated):**

```
FUNCTION 0x00CE7520  size 0x7e0  insns 597  blocks 128
  entry regs read before write: []

  B0 @0xce7520:
    ce752b: X87 STORE f32 [esp+0x20] <- 0.0  (rounds to f32)
    ce7532: X87 STORE f32 [esp+0x8] <- 0.0  (rounds to f32)
    ce7539: X87 STORE f32 [esp+0xc] <- 0.0  (rounds to f32)
    ce753d: X87 STORE f32 [esp+0x10] <- 0.0  (rounds to f32)
    ce7541: X87 STORE f32 [esp+0x1c] <- 0.0  (rounds to f32)
    ce755d: X87 STORE f32 [ebp+0x248] <- f32[ebp+0x244]  (rounds to f32)
    ce7566: WRITE [esp+0x14] mov= 0xffffffff   ; dword ptr [esp + 0x14], 0xffffffff
    ce756e: WRITE [esp+0x38] mov= i32[ecx+ebp]   ; dword ptr [esp + 0x38], edx
    ce7572: if (i32[esp+0x4c] <= 0) goto B10  else fallthrough   ; jle 0xce75cf

  B1 @0xce7574:
    ce758a: if (i32[esp+0x54] == 0x0) goto B3  else fallthrough   ; je 0xce7599

  B2 @0xce758c:
    ce7590: CALL 0xce5840 sub_ce5840  regs[eax=ebx, esi=?]  stack[]
    ce7597: goto B6

  B3 @0xce7599:
    ce759b: if (ebx != 0) goto B6  else fallthrough   ; jne 0xce75a9

  B4 @0xce759d:
    ce75a1: if (i32[esi+0x54] != 0x1) goto B6  else fallthrough   ; jne 0xce75a9

  B5 @0xce75a3:
    ce75a6: X87 STORE f32 [esi+0x24] <- f32[esi+0x20]  (rounds to f32)

  B6 @0xce75a9:
    ce75b0: if (i32[esi+0x14] <= i32[esp+0x38]) goto B9  else fallthrough   ; jle 0xce75c6

  B7 @0xce75b2:
    ce75b5: X87 COMPARE f32[esi+0x24]  vs  f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000)   [fcomp]
    ce75c0: if ((f32[esi+0x24] < f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000)) OR (f32[esi+0x24] == f64[0xdd50e8] =f64 0.10000000149011612 (0x3fb99999a0000000)) OR UNORDERED(NaN)) goto B9  else fallthrough   ; jne 0xce75c6

  B8 @0xce75c2:
    ce75c2: WRITE [esp+0x38] mov= ecx   ; dword ptr [esp + 0x38], ecx

  B9 @0xce75c6:
    ce75cd: if ((edi add 0x1) < i32[esp+0x4c]) goto B1  else fallthrough   ; jl 0xce7574

  B10 @0xce75cf:
    ce75cf: READ  ebx <- i32[0x24e1864]
    ce75e0: if (i32[esp+0x4c] < 0x4) goto B70  else fallthrough   ; jl 0xce792c

  B11 @0xce75e6:

  B12 @0xce75ed:
    ce75fa: X87 STORE f32 [esp+0x14] <- f32[eax+ebp+0x278]  (rounds to f32)
    ce7602: X87 COMPARE f32[esp+0x14]  vs  0.0   [fcom]
    ce7610: if ((f32[esp+0x14] < 0.0) OR (f32[esp+0x14] == 0.0) OR UNORDERED(NaN)) goto B25  else fallthrough   ; jne 0xce76b4

  B13 @0xce7616:
    ce761d: if ((i32[ecx+0x10] & 0x2000) != 0) goto B25  else fallthrough   ; jne 0xce76b4

  B14 @0xce7623:
    ce7627: X87 COMPARE f32[esp+0x30]  vs  f32[esp+0x14]   [fcomp]
    ce762e: if ((f32[esp+0x30] > f32[esp+0x14]) OR (f32[esp+0x30] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B25  else fallthrough   ; jp 0xce76b4

  B15 @0xce7634:
    ce763b: if (i32[ecx+0x14] != i32[esp+0x38]) goto B25  else fallthrough   ; jne 0xce76b4

  B16 @0xce763d:
    ce7640: X87 STORE f32 [esp+0x30] <- f32[esp+0x14]  (rounds to f32)
    ce7648: if (i32[ecx+0x4] < 0 (signed)) goto B19  else fallthrough   ; jl 0xce7658

  B17 @0xce764a:
    ce764c: if (eax >= ebx) goto B19  else fallthrough   ; jge 0xce7658

  B18 @0xce764e:
    ce7656: goto B20

  B19 @0xce7658:

  B20 @0xce765a:
    ce765f: if (i32[eax+0x1c] == 0) goto B23  else fallthrough   ; je 0xce7691

  B21 @0xce7661:
    ce7670: WRITE [esp+0x14] mov= (eax shl 0x4)   ; dword ptr [esp + 0x14], eax
    ce7674: if ((eax shl 0x4) <= 0x384) goto B24  else fallthrough   ; jle 0xce7699

  B22 @0xce7676:
    ce7683: X87 STORE f32 [esp+0x18] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x18])  (rounds to f32)
    ce768b: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] + f32[esp+0x1c])  (rounds to f32)
    ce768f: goto B26

  B23 @0xce7691:
    ce7691: WRITE [esp+0x14] mov= 0x0   ; dword ptr [esp + 0x14], 0

  B24 @0xce7699:
    ce76a6: X87 STORE f32 [esp+0x20] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x20])  (rounds to f32)
    ce76ae: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x14] + f32[esp+0x2c])  (rounds to f32)
    ce76b2: goto B26

  B25 @0xce76b4:

  B26 @0xce76b6:
    ce76c3: X87 STORE f32 [esp+0x14] <- f32[ecx+ebp+0x278]  (rounds to f32)
    ce76cb: X87 COMPARE f32[esp+0x14]  vs  0.0   [fcom]
    ce76d9: if ((f32[esp+0x14] < 0.0) OR (f32[esp+0x14] == 0.0) OR UNORDERED(NaN)) goto B39  else fallthrough   ; jne 0xce777e

  B27 @0xce76df:
    ce76e6: if ((i32[ecx+0x10] & 0x2000) != 0) goto B39  else fallthrough   ; jne 0xce777e

  B28 @0xce76ec:
    ce76f0: X87 COMPARE f32[esp+0x30]  vs  f32[esp+0x14]   [fcomp]
    ce76f7: if ((f32[esp+0x30] > f32[esp+0x14]) OR (f32[esp+0x30] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B39  else fallthrough   ; jp 0xce777e

  B29 @0xce76fd:
    ce7704: if (i32[ecx+0x14] != i32[esp+0x38]) goto B39  else fallthrough   ; jne 0xce777e

  B30 @0xce7706:
    ce7709: X87 STORE f32 [esp+0x30] <- f32[esp+0x14]  (rounds to f32)
    ce7712: if (i32[ecx+0x4] < 0 (signed)) goto B33  else fallthrough   ; jl 0xce7722

  B31 @0xce7714:
    ce7716: if (eax >= ebx) goto B33  else fallthrough   ; jge 0xce7722

  B32 @0xce7718:
    ce7720: goto B34

  B33 @0xce7722:

  B34 @0xce7724:
    ce7729: if (i32[eax+0x1c] == 0) goto B37  else fallthrough   ; je 0xce775b

  B35 @0xce772b:
    ce773a: WRITE [esp+0x14] mov= (eax shl 0x4)   ; dword ptr [esp + 0x14], eax
    ce773e: if ((eax shl 0x4) <= 0x384) goto B38  else fallthrough   ; jle 0xce7763

  B36 @0xce7740:
    ce774d: X87 STORE f32 [esp+0x18] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x18])  (rounds to f32)
    ce7755: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] + f32[esp+0x1c])  (rounds to f32)
    ce7759: goto B40

  B37 @0xce775b:
    ce775b: WRITE [esp+0x14] mov= 0x0   ; dword ptr [esp + 0x14], 0

  B38 @0xce7763:
    ce7770: X87 STORE f32 [esp+0x20] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x20])  (rounds to f32)
    ce7778: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x14] + f32[esp+0x2c])  (rounds to f32)
    ce777c: goto B40

  B39 @0xce777e:

  B40 @0xce7780:
    ce778c: X87 STORE f32 [esp+0x14] <- f32[ecx+ebp+0x278]  (rounds to f32)
    ce7794: X87 COMPARE f32[esp+0x14]  vs  0.0   [fcom]
    ce77a2: if ((f32[esp+0x14] < 0.0) OR (f32[esp+0x14] == 0.0) OR UNORDERED(NaN)) goto B53  else fallthrough   ; jne 0xce7847

  B41 @0xce77a8:
    ce77af: if ((i32[ecx+0x10] & 0x2000) != 0) goto B53  else fallthrough   ; jne 0xce7847

  B42 @0xce77b5:
    ce77b9: X87 COMPARE f32[esp+0x30]  vs  f32[esp+0x14]   [fcomp]
    ce77c0: if ((f32[esp+0x30] > f32[esp+0x14]) OR (f32[esp+0x30] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B53  else fallthrough   ; jp 0xce7847

  B43 @0xce77c6:
    ce77cd: if (i32[ecx+0x14] != i32[esp+0x38]) goto B53  else fallthrough   ; jne 0xce7847

  B44 @0xce77cf:
    ce77d2: X87 STORE f32 [esp+0x30] <- f32[esp+0x14]  (rounds to f32)
    ce77db: if (i32[ecx+0x4] < 0 (signed)) goto B47  else fallthrough   ; jl 0xce77eb

  B45 @0xce77dd:
    ce77df: if (eax >= ebx) goto B47  else fallthrough   ; jge 0xce77eb

  B46 @0xce77e1:
    ce77e9: goto B48

  B47 @0xce77eb:

  B48 @0xce77ed:
    ce77f2: if (i32[eax+0x1c] == 0) goto B51  else fallthrough   ; je 0xce7824

  B49 @0xce77f4:
    ce7803: WRITE [esp+0x14] mov= (eax shl 0x4)   ; dword ptr [esp + 0x14], eax
    ce7807: if ((eax shl 0x4) <= 0x384) goto B52  else fallthrough   ; jle 0xce782c

  B50 @0xce7809:
    ce7816: X87 STORE f32 [esp+0x18] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x18])  (rounds to f32)
    ce781e: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] + f32[esp+0x1c])  (rounds to f32)
    ce7822: goto B54

  B51 @0xce7824:
    ce7824: WRITE [esp+0x14] mov= 0x0   ; dword ptr [esp + 0x14], 0

  B52 @0xce782c:
    ce7839: X87 STORE f32 [esp+0x20] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x20])  (rounds to f32)
    ce7841: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x14] + f32[esp+0x2c])  (rounds to f32)
    ce7845: goto B54

  B53 @0xce7847:

  B54 @0xce7849:
    ce7856: X87 STORE f32 [esp+0x14] <- f32[ecx+ebp+0x278]  (rounds to f32)
    ce785e: X87 COMPARE f32[esp+0x14]  vs  0.0   [fcom]
    ce786c: if ((f32[esp+0x14] < 0.0) OR (f32[esp+0x14] == 0.0) OR UNORDERED(NaN)) goto B67  else fallthrough   ; jne 0xce7911

  B55 @0xce7872:
    ce7879: if ((i32[ecx+0x10] & 0x2000) != 0) goto B67  else fallthrough   ; jne 0xce7911

  B56 @0xce787f:
    ce7883: X87 COMPARE f32[esp+0x30]  vs  f32[esp+0x14]   [fcomp]
    ce788a: if ((f32[esp+0x30] > f32[esp+0x14]) OR (f32[esp+0x30] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B67  else fallthrough   ; jp 0xce7911

  B57 @0xce7890:
    ce7897: if (i32[ecx+0x14] != i32[esp+0x38]) goto B67  else fallthrough   ; jne 0xce7911

  B58 @0xce7899:
    ce789c: X87 STORE f32 [esp+0x30] <- f32[esp+0x14]  (rounds to f32)
    ce78a5: if (i32[ecx+0x4] < 0 (signed)) goto B61  else fallthrough   ; jl 0xce78b5

  B59 @0xce78a7:
    ce78a9: if (eax >= ebx) goto B61  else fallthrough   ; jge 0xce78b5

  B60 @0xce78ab:
    ce78b3: goto B62

  B61 @0xce78b5:

  B62 @0xce78b7:
    ce78bc: if (i32[eax+0x1c] == 0) goto B65  else fallthrough   ; je 0xce78ee

  B63 @0xce78be:
    ce78cd: WRITE [esp+0x14] mov= (eax shl 0x4)   ; dword ptr [esp + 0x14], eax
    ce78d1: if ((eax shl 0x4) <= 0x384) goto B66  else fallthrough   ; jle 0xce78f6

  B64 @0xce78d3:
    ce78e0: X87 STORE f32 [esp+0x18] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x18])  (rounds to f32)
    ce78e8: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] + f32[esp+0x1c])  (rounds to f32)
    ce78ec: goto B68

  B65 @0xce78ee:
    ce78ee: WRITE [esp+0x14] mov= 0x0   ; dword ptr [esp + 0x14], 0

  B66 @0xce78f6:
    ce7903: X87 STORE f32 [esp+0x20] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x20])  (rounds to f32)
    ce790b: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x14] + f32[esp+0x2c])  (rounds to f32)
    ce790f: goto B68

  B67 @0xce7911:

  B68 @0xce7913:
    ce7922: if ((edx add 0x4) < (eax add 0xfffffffd)) goto B12  else fallthrough   ; jl 0xce75ed

  B69 @0xce7928:
    ce7928: WRITE [esp+0x14] mov= edi   ; dword ptr [esp + 0x14], edi

  B70 @0xce792c:
    ce7930: if (edx >= i32[esp+0x4c]) goto B88  else fallthrough   ; jge 0xce7a14

  B71 @0xce7936:

  B72 @0xce793a:
    ce7947: X87 STORE f32 [esp+0x14] <- f32[ecx+ebp+0x278]  (rounds to f32)
    ce794f: X87 COMPARE f32[esp+0x14]  vs  0.0   [fcom]
    ce795d: if ((f32[esp+0x14] < 0.0) OR (f32[esp+0x14] == 0.0) OR UNORDERED(NaN)) goto B85  else fallthrough   ; jne 0xce7a01

  B73 @0xce7963:
    ce796a: if ((i32[ecx+0x10] & 0x2000) != 0) goto B85  else fallthrough   ; jne 0xce7a01

  B74 @0xce7970:
    ce7974: X87 COMPARE f32[esp+0x30]  vs  f32[esp+0x14]   [fcomp]
    ce797b: if ((f32[esp+0x30] > f32[esp+0x14]) OR (f32[esp+0x30] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B85  else fallthrough   ; jp 0xce7a01

  B75 @0xce7981:
    ce7988: if (i32[ecx+0x14] != i32[esp+0x38]) goto B85  else fallthrough   ; jne 0xce7a01

  B76 @0xce798a:
    ce798d: X87 STORE f32 [esp+0x30] <- f32[esp+0x14]  (rounds to f32)
    ce7995: if (i32[ecx+0x4] < 0 (signed)) goto B79  else fallthrough   ; jl 0xce79a5

  B77 @0xce7997:
    ce7999: if (eax >= ebx) goto B79  else fallthrough   ; jge 0xce79a5

  B78 @0xce799b:
    ce79a3: goto B80

  B79 @0xce79a5:

  B80 @0xce79a7:
    ce79ac: if (i32[eax+0x1c] == 0) goto B83  else fallthrough   ; je 0xce79de

  B81 @0xce79ae:
    ce79bd: WRITE [esp+0x14] mov= (eax shl 0x4)   ; dword ptr [esp + 0x14], eax
    ce79c1: if ((eax shl 0x4) <= 0x384) goto B84  else fallthrough   ; jle 0xce79e6

  B82 @0xce79c3:
    ce79d0: X87 STORE f32 [esp+0x18] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x18])  (rounds to f32)
    ce79d8: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] + f32[esp+0x1c])  (rounds to f32)
    ce79dc: goto B86

  B83 @0xce79de:
    ce79de: WRITE [esp+0x14] mov= 0x0   ; dword ptr [esp + 0x14], 0

  B84 @0xce79e6:
    ce79f3: X87 STORE f32 [esp+0x20] <- ((((float)i32[esp+0x14] * f32[esp+0x14]) / f32[ecx+0x18]) + f32[esp+0x20])  (rounds to f32)
    ce79fb: X87 STORE f32 [esp+0x2c] <- (f32[esp+0x14] + f32[esp+0x2c])  (rounds to f32)
    ce79ff: goto B86

  B85 @0xce7a01:

  B86 @0xce7a03:
    ce7a0a: if ((edx add 0x1) < i32[esp+0x4c]) goto B72  else fallthrough   ; jl 0xce793a

  B87 @0xce7a10:
    ce7a10: WRITE [esp+0x14] mov= edi   ; dword ptr [esp + 0x14], edi

  B88 @0xce7a14:
    ce7a17: if (edi != 0xffffffff) goto B90  else fallthrough   ; jne 0xce7a2f

  B89 @0xce7a19:
    ce7a19: X87 STORE f32 [ebp+0x244] <- 0.0  (rounds to f32)
    ce7a1f: X87 STORE f32 [ebp+0x248] <- 0.0  (rounds to f32)
    ce7a2c: RETURN 0x14

  B90 @0xce7a2f:
    ce7a3d: X87 STORE f32 [esp+0x38] <- (f32[esp+0x1c] - 0.0)  (rounds to f32)
    ce7a4b: X87 COMPARE f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)  vs  f32[esp+0x38]   [fcom]
    ce7a52: if ((f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000) < f32[esp+0x38]) OR (f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000) == f32[esp+0x38]) OR UNORDERED(NaN)) goto B93  else fallthrough   ; jne 0xce7a6b

  B91 @0xce7a54:
    ce7a58: X87 COMPARE -f32[esp+0x38]  vs  f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)   [fcomp]
    ce7a5f: if ((-f32[esp+0x38] < f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000))) goto B94  else fallthrough   ; jnp 0xce7a77

  B92 @0xce7a61:
    ce7a69: goto B100

  B93 @0xce7a6b:
    ce7a75: goto B100

  B94 @0xce7a77:
    ce7a83: X87 STORE f32 [esp+0x38] <- (f32[esp+0x20] - 0.0)  (rounds to f32)
    ce7a8b: X87 COMPARE f32[esp+0x38]  vs  f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)   [fcom]
    ce7a92: if ((f32[esp+0x38] > f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)) OR (f32[esp+0x38] == f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)) OR UNORDERED(NaN)) goto B98  else fallthrough   ; jp 0xce7aad

  B95 @0xce7a94:
    ce7a96: X87 COMPARE -f32[esp+0x38]  vs  f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)   [fcomp]
    ce7a9f: if ((-f32[esp+0x38] > f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)) OR (-f32[esp+0x38] == f64[0xe5c1a0] =f64 9.9999997473787516e-05 (0x3f1a36e2e0000000)) OR UNORDERED(NaN)) goto B99  else fallthrough   ; jp 0xce7ab1

  B96 @0xce7aa1:

  B97 @0xce7aa3:
    ce7aaa: RETURN 0x14

  B98 @0xce7aad:

  B99 @0xce7ab1:

  B100 @0xce7ab5:
    ce7ab5: X87 STORE f32 [esp+0x18] <- (f32[esp+0x18] / f32[esp+0x1c])  (rounds to f32)
    ce7ac8: X87 STORE f32 [esp+0x3c] <- ((f32[ebp+0x24c] * i32[esp+0x54]) / f32[esp+0x1c])  (rounds to f32)
    ce7ad0: X87 STORE f64 [esp+0x3c] <- f32[esp+0x3c]
    ce7ad4: X87 STORE f32 [esp] <- f32[esp+0x3c]  (rounds to f32)
    ce7ad7: CALL 0x427600 floorf_wrapper  regs[]  stack[ecx]
    ce7ae0: X87 STORE f32 [esp+0x3c] <- ST0<ret floorf_wrapper>  (rounds to f32)
    ce7ae8: X87 STORE f64 [esp+0x3c] <- f32[esp+0x3c]
    ce7aec: X87 STORE f32 [esp] <- f32[esp+0x3c]  (rounds to f32)
    ce7aef: CALL 0x427600 floorf_wrapper  regs[]  stack[]
    ce7af7: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    ce7afc: WRITE [esp+0x50] mov= ret(_ftol2_sse)   ; dword ptr [esp + 0x50], eax
    ce7b04: WRITE [esp+0x50] mov= 0x0   ; byte ptr [esp + 0x50], 0
    ce7b0d: X87 STORE f32 [esp+0x38] <- (f64[esp+0x38] - (float)i32[esp+0x50])  (rounds to f32)
    ce7b1b: X87 STORE f32 [esp+0x30] <- (f32[esp+0x38] + f32[ebp+0x244])  (rounds to f32)
    ce7b23: X87 STORE f32 [ebp+0x244] <- f32[esp+0x30]  (rounds to f32)
    ce7b2b: X87 COMPARE 1.0  vs  f32[esp+0x30]   [fcomp]
    ce7b32: if ((1.0 > f32[esp+0x30]) OR (1.0 == f32[esp+0x30]) OR UNORDERED(NaN)) goto B102  else fallthrough   ; jp 0xce7b62

  B101 @0xce7b34:
    ce7b35: X87 STORE f64 [esp+0x34] <- f32[esp+0x30]
    ce7b39: X87 STORE f32 [esp] <- f32[esp+0x30]  (rounds to f32)
    ce7b3c: CALL 0x427600 floorf_wrapper  regs[]  stack[ecx]
    ce7b44: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    ce7b49: WRITE [esp+0x50] mov= ret(_ftol2_sse)   ; dword ptr [esp + 0x50], eax
    ce7b51: WRITE [esp+0x50] mov= 0x1   ; byte ptr [esp + 0x50], 1
    ce7b5a: X87 STORE f32 [ebp+0x244] <- (f64[esp+0x30] - (float)i32[esp+0x50])  (rounds to f32)
    ce7b60: goto B103

  B102 @0xce7b62:

  B103 @0xce7b64:
    ce7b6a: WRITE [esp+0x2c] mov= 0   ; dword ptr [esp + 0x2c], edi
    ce7b6e: if (i32[esp+0x4c] <= 0) goto B97  else fallthrough   ; jle 0xce7aa3

  B104 @0xce7b74:
    ce7b80: X87 COMPARE 0.0  vs  f32[edx+ebp+0x278]   [fcomp]
    ce7b93: if ((0.0 > f32[edx+ebp+0x278]) OR (0.0 == f32[edx+ebp+0x278]) OR UNORDERED(NaN)) goto B126  else fallthrough   ; jp 0xce7ce5

  B105 @0xce7b99:
    ce7b9e: if (i32[esi+0x4] < 0 (signed)) goto B108  else fallthrough   ; jl 0xce7bb4

  B106 @0xce7ba0:
    ce7ba6: if (eax >= i32[0x24e1864]) goto B108  else fallthrough   ; jge 0xce7bb4

  B107 @0xce7ba8:
    ce7bb2: goto B109

  B108 @0xce7bb4:

  B109 @0xce7bb6:
    ce7bb8: X87 COMPARE 1.0  vs  f32[esi+0x18]   [fcomp]
    ce7bc0: if ((1.0 == f32[esi+0x18])) goto B111  else fallthrough   ; jnp 0xce7c00

  B110 @0xce7bc2:
    ce7bd5: X87 STORE f32 [esp+0x34] <- (f32[esi+0x1c] - ((f32[esi+0x18] - f64[0xe5bf40] =f64 1 (0x3ff0000000000000)) * f32[esp+0x3c]))  (rounds to f32)
    ce7bdd: X87 STORE f64 [esp+0x24] <- f32[esp+0x34]
    ce7be1: X87 STORE f32 [esp] <- f32[esp+0x34]  (rounds to f32)
    ce7be4: CALL 0x427600 floorf_wrapper  regs[]  stack[ecx]
    ce7bec: CALL 0xda0140 _ftol2_sse  regs[]  stack[]
    ce7bf1: WRITE [esp+0x30] mov= ret(_ftol2_sse)   ; dword ptr [esp + 0x30], eax
    ce7bfd: X87 STORE f32 [esi+0x1c] <- (f64[esp+0x20] - (float)i32[esp+0x30])  (rounds to f32)

  B111 @0xce7c00:
    ce7c04: WRITE [esi+0x8] mov= i32[esi+0xc]   ; dword ptr [esi + 8], eax
    ce7c10: X87 STORE f32 [esp] <- f32[ebp+0x244]  (rounds to f32)
    ce7c13: CALL 0xce57b0 sub_ce57b0  regs[ecx=esi]  stack[ecx]
    ce7c1d: WRITE [esi+0xc] mov= ret(sub_ce57b0)   ; dword ptr [esi + 0xc], eax
    ce7c20: if (i32[esp+0x54] == 0x0) goto B126  else fallthrough   ; je 0xce7ce5

  B112 @0xce7c26:
    ce7c2a: if (edi != i32[esp+0x14]) goto B126  else fallthrough   ; jne 0xce7ce5

  B113 @0xce7c30:
    ce7c34: WRITE [esp+0x30] mov= 0x0   ; dword ptr [esp + 0x30], 0
    ce7c3c: if (i32[ebx+0x10] <= 0x0) goto B126  else fallthrough   ; jle 0xce7ce5

  B114 @0xce7c42:
    ce7c46: if ((i32[esi+0x10] & 0x4) == 0) goto B116  else fallthrough   ; je 0xce7c50

  B115 @0xce7c48:
    ce7c4e: goto B117

  B116 @0xce7c50:

  B117 @0xce7c56:
    ce7c66: if (edi < i32[ecx+edx*8+0x4]) goto B119  else fallthrough   ; jl 0xce7c6c

  B118 @0xce7c68:
    ce7c6a: if (eax > ebp) goto B122  else fallthrough   ; jg 0xce7c7b

  B119 @0xce7c6c:
    ce7c71: if (i32[esp+0x50] == 0x0) goto B124  else fallthrough   ; je 0xce7cc9

  B120 @0xce7c73:
    ce7c75: if (eax > ebp) goto B122  else fallthrough   ; jg 0xce7c7b

  B121 @0xce7c77:
    ce7c79: if (eax >= edi) goto B124  else fallthrough   ; jge 0xce7cc9

  B122 @0xce7c7b:
    ce7c96: if ((i32[ecx*4+0xdc8558] & i32[edx+esi+0x2c]) != 0) goto B124  else fallthrough   ; jne 0xce7cc9

  B123 @0xce7c98:
    ce7caa: WRITE [edx+esi+0x2c] or= i32[ecx*4+0xdc8558]   ; byte ptr [edx + esi + 0x2c], cl
    ce7cc1: WRITE [edx+esi+0x2c] or= i32[eax*4+0xdc8558]   ; byte ptr [edx + esi + 0x2c], al

  B124 @0xce7cc9:
    ce7cd3: WRITE [esp+0x30] mov= (eax add 0x1)   ; dword ptr [esp + 0x30], eax
    ce7cd7: if ((eax add 0x1) < i32[ebx+0x10]) goto B114  else fallthrough   ; jl 0xce7c42

  B125 @0xce7cdd:

  B126 @0xce7ce5:
    ce7cec: WRITE [esp+0x2c] mov= (edi add 0x1)   ; dword ptr [esp + 0x2c], edi
    ce7cf0: if ((edi add 0x1) < i32[esp+0x4c]) goto B104  else fallthrough   ; jl 0xce7b74

  B127 @0xce7cf6:
    ce7cfd: RETURN 0x14

  call targets: 0x427600(floorf_wrapper), 0xce57b0, 0xce5840, 0xda0140(_ftol2_sse)
```

### vehicle_camera_record_parse — 0xAED2D0

**Role.** One `vehicle_cameras.xtbl` / `vehicle_group_cameras.xtbl` record. Arguments (cdecl, caller cleans): `[esp+4]` vehicle record* (fields written at `rec+0x27C` ... `rec+0x410` ... and the `0x312DD0C`-based defaults when the pointer is null), `[esp+8]` xml element, `[esp+0xC]` flag (defaults mode). It reads the tags Camera_Proximity_Lock_Y, Camera_Follow_Aggression, Position_Based / Sphere_Based, Camera_Roll, Swing_Rate_Fwd/Rev, From_Sticky_Fwd/Rev, Heading_Retention_Normal/Turning, Slerp_Value, From_Sticky_Slerp_Value, Min_Angle, Roll_Type (`none` and others), Intensity_Multiplier, skip_camera_transition (`yes`), use_alt_freckle_cam, camera_fov_scale and Max_FOV with `xtbl_get_float(_opt)` and `_stricmp`, and calls `vehicle_camera_angles_parse`, `vehicle_camera_swings_parse`, `vehicle_camera_helicopter_parse`, `vehicle_camera_seat_data_parse`, `vehicle_camera_sticky_parse` and `0x4958F0` (an angle conversion through the CRT dispatcher `0xDA082C`).


**Block map (generated):**

```
FUNCTION 0x00AED2D0  size 0x626  insns 486  blocks 98
  entry regs read before write: []

  B0 @0xaed2d0:
    aed2e1: if (i32[esp+0x14] == 0) goto B97  else fallthrough   ; je 0xaed8ec

  B1 @0xaed2e7:
    aed2e9: if (edi == 0) goto B97  else fallthrough   ; je 0xaed8ec

  B2 @0xaed2ef:
    aed2fa: WRITE [esp+0x1c] mov= i32[ebp+0x27c]   ; dword ptr [esp + 0x1c], ebx
    aed304: if (i32[esp+0x24] == 0x0) goto B4  else fallthrough   ; je 0xaed30e

  B3 @0xaed306:
    aed30c: goto B5

  B4 @0xaed30e:

  B5 @0xaed314:
    aed315: X87 STORE f32 [esi] <- f32[0x312dcc8]  (rounds to f32)
    aed31c: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe46b00 "Camera_Proximity_Lock_Y", esi=?]  stack[]
    aed328: CALL 0xae5f20 sub_ae5f20  regs[edi=?]  stack[0x0]
    aed337: CALL 0xae60f0 sub_ae60f0  regs[eax=edi, ecx=ebx]  stack[0x0]
    aed341: CALL 0xae6450 sub_ae6450  regs[eax=edi, ecx=ebp]  stack[]
    aed34d: CALL 0xae6200 sub_ae6200  regs[eax=edi]  stack[]
    aed355: CALL 0xae6300 sub_ae6300  regs[eax=edi, ebx=?]  stack[]
    aed365: if (i32[esp+0x24] == 0x0) goto B11  else fallthrough   ; je 0xaed38d

  B6 @0xaed367:
    aed369: if (esi == 0) goto B14  else fallthrough   ; je 0xaed3ac

  B7 @0xaed36b:
    aed36b: goto B8

  B8 @0xaed370:
    aed378: CALL 0xc99030 __stricmp  regs[]  stack[i32[esi], 0xe47e68 "Camera_Follow_Aggression"]
    aed382: if (ret(__stricmp) == 0) goto B15  else fallthrough   ; je 0xaed3ae

  B9 @0xaed384:
    aed389: if (i32[esi+0x4] != 0) goto B8  else fallthrough   ; jne 0xaed370

  B10 @0xaed38b:
    aed38b: goto B14

  B11 @0xaed38d:
    aed38f: if (esi == 0) goto B14  else fallthrough   ; je 0xaed3ac

  B12 @0xaed391:
    aed399: CALL 0xc99030 __stricmp  regs[]  stack[i32[esi], 0xe47e68 "Camera_Follow_Aggression"]
    aed3a3: if (ret(__stricmp) == 0) goto B15  else fallthrough   ; je 0xaed3ae

  B13 @0xaed3a5:
    aed3aa: if (i32[esi+0x4] != 0) goto B12  else fallthrough   ; jne 0xaed391

  B14 @0xaed3ac:

  B15 @0xaed3ae:
    aed3b2: if (esi == 0) goto B29  else fallthrough   ; je 0xaed428

  B16 @0xaed3b4:
    aed3b9: if (i32[edi+0x8] == 0) goto B20  else fallthrough   ; je 0xaed3df

  B17 @0xaed3bb:
    aed3bb: goto B18

  B18 @0xaed3c0:
    aed3c8: CALL 0xc99030 __stricmp  regs[]  stack[i32[esi], 0xe47e84 "Position_Based"]
    aed3d2: if (ret(__stricmp) == 0) goto B35  else fallthrough   ; je 0xaed50a

  B19 @0xaed3d8:
    aed3dd: if (i32[esi+0x4] != 0) goto B18  else fallthrough   ; jne 0xaed3c0

  B20 @0xaed3df:

  B21 @0xaed3e1:
    aed3e6: if (i32[edi+0x8] == 0) goto B24  else fallthrough   ; je 0xaed407

  B22 @0xaed3e8:
    aed3f0: CALL 0xc99030 __stricmp  regs[]  stack[i32[esi], 0xe47e94 "Sphere_Based"]
    aed3fa: if (ret(__stricmp) == 0) goto B36  else fallthrough   ; je 0xaed511

  B23 @0xaed400:
    aed405: if (i32[esi+0x4] != 0) goto B22  else fallthrough   ; jne 0xaed3e8

  B24 @0xaed407:

  B25 @0xaed409:
    aed40b: if (ebx != 0) goto B37  else fallthrough   ; jne 0xaed518

  B26 @0xaed411:
    aed413: if (edi != 0) goto B74  else fallthrough   ; jne 0xaed73b

  B27 @0xaed419:
    aed41e: if (i32[esp+0x24] != 0x0) goto B88  else fallthrough   ; jne 0xaed854

  B28 @0xaed424:

  B29 @0xaed428:
    aed434: X87 STORE f32 [ebp+0x4b8] <- f32[ebx+0x4b8]  (rounds to f32)
    aed440: X87 STORE f32 [ebp+0x4c8] <- f32[ebx+0x4c8]  (rounds to f32)
    aed44c: X87 STORE f32 [ebp+0x4d8] <- f32[ebx+0x4d8]  (rounds to f32)
    aed458: X87 STORE f32 [ebp+0x4dc] <- f32[ebx+0x4dc]  (rounds to f32)
    aed464: X87 STORE f32 [ebp+0x4bc] <- f32[ebx+0x4bc]  (rounds to f32)
    aed470: X87 STORE f32 [ebp+0x4c0] <- f32[ebx+0x4c0]  (rounds to f32)
    aed47c: X87 STORE f32 [ebp+0x4cc] <- f32[ebx+0x4cc]  (rounds to f32)
    aed488: X87 STORE f32 [ebp+0x4d0] <- f32[ebx+0x4cc]  (rounds to f32)
    aed494: X87 STORE f32 [ebp+0x4d4] <- f32[ebx+0x4d4]  (rounds to f32)
    aed4a6: WRITE [ebp+0x5b0] xor= (ecx and 0x40000000)   ; dword ptr [ebp + 0x5b0], ecx
    aed4c6: WRITE [ebp+0x5b0] mov= (edx xor [ebx+0x5b0])   ; dword ptr [ebp + 0x5b0], edx

  B30 @0xaed4cc:
    aed4d1: if (i32[esp+0x24] != 0x0) goto B88  else fallthrough   ; jne 0xaed854

  B31 @0xaed4d7:
    aed4e0: if (i32[eax+0x8] == 0) goto B34  else fallthrough   ; je 0xaed501

  B32 @0xaed4e2:
    aed4ea: CALL 0xc99030 __stricmp  regs[]  stack[i32[esi], 0xe47f4c "Camera_Roll"]
    aed4f4: if (ret(__stricmp) == 0) goto B77  else fallthrough   ; je 0xaed7a4

  B33 @0xaed4fa:
    aed4ff: if (i32[esi+0x4] != 0) goto B32  else fallthrough   ; jne 0xaed4e2

  B34 @0xaed501:
    aed505: goto B81

  B35 @0xaed50a:
    aed50c: goto B21

  B36 @0xaed511:
    aed513: goto B25

  B37 @0xaed518:
    aed526: CALL 0xb74ef0 xtbl_get_float  regs[ecx=0xe47ea4 "Swing_Rate_Fwd", esi=&[ebp+0x4b8]]  stack[]
    aed53a: CALL 0xb74ef0 xtbl_get_float  regs[ecx=0xe47eb4 "Swing_Rate_Rev", esi=&[ebp+0x4c8]]  stack[]
    aed541: X87 COMPARE 0.0  vs  f32[edi]   [fcomp]
    aed54b: if ((0.0 == f32[edi])) goto B44  else fallthrough   ; jnp 0xaed59e

  B38 @0xaed54d:
    aed556: X87 STORE f32 [esp+0x18] <- (f64[0xe5d620] =f64 -0.038376417011022568 (0xbfa3a612e0000000) / f32[edi])  (rounds to f32)
    aed55e: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    aed561: CALL 0x4958f0 sub_4958f0  regs[]  stack[ecx]
    aed571: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed578: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B40  else fallthrough   ; jp 0xaed585

  B39 @0xaed57a:
    aed57c: X87 COMPARE ST0<ret sub_4958f0>  vs  1.0   [fcomp]
    aed583: if ((ST0<ret sub_4958f0> > 1.0) OR (ST0<ret sub_4958f0> == 1.0) OR UNORDERED(NaN)) goto B42  else fallthrough   ; jp 0xaed596

  B40 @0xaed585:
    aed587: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed58e: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B43  else fallthrough   ; jp 0xaed598

  B41 @0xaed590:
    aed592: X87 STORE f32 [edi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed594: goto B45

  B42 @0xaed596:

  B43 @0xaed598:
    aed59a: X87 STORE f32 [edi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed59c: goto B45

  B44 @0xaed59e:
    aed5a0: X87 STORE f32 [edi] <- 1.0  (rounds to f32)

  B45 @0xaed5a2:
    aed5a4: X87 COMPARE 0.0  vs  f32[ebp+0x4c8]   [fcomp]
    aed5af: if ((0.0 == f32[ebp+0x4c8])) goto B52  else fallthrough   ; jnp 0xaed60e

  B46 @0xaed5b1:
    aed5be: X87 STORE f32 [esp+0x18] <- (f64[0xe5d620] =f64 -0.038376417011022568 (0xbfa3a612e0000000) / f32[ebp+0x4c8])  (rounds to f32)
    aed5c6: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    aed5c9: CALL 0x4958f0 sub_4958f0  regs[]  stack[ecx]
    aed5d9: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed5e0: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B48  else fallthrough   ; jp 0xaed5ed

  B47 @0xaed5e2:
    aed5e4: X87 COMPARE ST0<ret sub_4958f0>  vs  1.0   [fcomp]
    aed5eb: if ((ST0<ret sub_4958f0> > 1.0) OR (ST0<ret sub_4958f0> == 1.0) OR UNORDERED(NaN)) goto B50  else fallthrough   ; jp 0xaed602

  B48 @0xaed5ed:
    aed5ef: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed5f6: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B51  else fallthrough   ; jp 0xaed604

  B49 @0xaed5f8:
    aed5fa: X87 STORE f32 [ebp+0x4c8] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed600: goto B53

  B50 @0xaed602:

  B51 @0xaed604:
    aed606: X87 STORE f32 [ebp+0x4c8] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed60c: goto B53

  B52 @0xaed60e:
    aed610: X87 STORE f32 [ebp+0x4c8] <- 1.0  (rounds to f32)

  B53 @0xaed616:
    aed622: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47ec4 "From_Sticky_Fwd", esi=&[ebp+0x4d8]]  stack[]
    aed62c: if (al == 0) goto B60  else fallthrough   ; je 0xaed67f

  B54 @0xaed62e:
    aed637: X87 STORE f32 [esp+0x18] <- (f64[0xe5d620] =f64 -0.038376417011022568 (0xbfa3a612e0000000) / f32[esi])  (rounds to f32)
    aed63f: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    aed642: CALL 0x4958f0 sub_4958f0  regs[]  stack[ecx]
    aed652: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed659: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B56  else fallthrough   ; jp 0xaed666

  B55 @0xaed65b:
    aed65d: X87 COMPARE ST0<ret sub_4958f0>  vs  1.0   [fcomp]
    aed664: if ((ST0<ret sub_4958f0> > 1.0) OR (ST0<ret sub_4958f0> == 1.0) OR UNORDERED(NaN)) goto B58  else fallthrough   ; jp 0xaed677

  B56 @0xaed666:
    aed668: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed66f: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B59  else fallthrough   ; jp 0xaed679

  B57 @0xaed671:
    aed673: X87 STORE f32 [esi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed675: goto B61

  B58 @0xaed677:

  B59 @0xaed679:
    aed67b: X87 STORE f32 [esi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed67d: goto B61

  B60 @0xaed67f:
    aed681: X87 STORE f32 [esi] <- f32[edi]  (rounds to f32)

  B61 @0xaed683:
    aed68f: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47ed4 "From_Sticky_Rev", esi=&[ebp+0x4dc]]  stack[]
    aed699: if (al == 0) goto B68  else fallthrough   ; je 0xaed6ec

  B62 @0xaed69b:
    aed6a4: X87 STORE f32 [esp+0x18] <- (f64[0xe5d620] =f64 -0.038376417011022568 (0xbfa3a612e0000000) / f32[esi])  (rounds to f32)
    aed6ac: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    aed6af: CALL 0x4958f0 sub_4958f0  regs[]  stack[ecx]
    aed6bf: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed6c6: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B64  else fallthrough   ; jp 0xaed6d3

  B63 @0xaed6c8:
    aed6ca: X87 COMPARE ST0<ret sub_4958f0>  vs  1.0   [fcomp]
    aed6d1: if ((ST0<ret sub_4958f0> > 1.0) OR (ST0<ret sub_4958f0> == 1.0) OR UNORDERED(NaN)) goto B66  else fallthrough   ; jp 0xaed6e4

  B64 @0xaed6d3:
    aed6d5: X87 COMPARE 0.0  vs  ST0<ret sub_4958f0>   [fcom]
    aed6dc: if ((0.0 > ST0<ret sub_4958f0>) OR (0.0 == ST0<ret sub_4958f0>) OR UNORDERED(NaN)) goto B67  else fallthrough   ; jp 0xaed6e6

  B65 @0xaed6de:
    aed6e0: X87 STORE f32 [esi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed6e2: goto B69

  B66 @0xaed6e4:

  B67 @0xaed6e6:
    aed6e8: X87 STORE f32 [esi] <- ST0<ret sub_4958f0>  (rounds to f32)
    aed6ea: goto B69

  B68 @0xaed6ec:
    aed6f2: X87 STORE f32 [esi] <- f32[ebp+0x4c8]  (rounds to f32)

  B69 @0xaed6f4:
    aed700: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47ee4 "Heading_Retention_Normal", esi=&[ebp+0x4bc]]  stack[]
    aed70a: if (al != 0) goto B71  else fallthrough   ; jne 0xaed710

  B70 @0xaed70c:
    aed70e: X87 STORE f32 [esi] <- 1.0  (rounds to f32)

  B71 @0xaed710:
    aed71c: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47f00 "Heading_Retention_Turning", esi=&[ebp+0x4c0]]  stack[]
    aed726: if (al != 0) goto B73  else fallthrough   ; jne 0xaed72c

  B72 @0xaed728:
    aed72a: X87 STORE f32 [esi] <- 1.0  (rounds to f32)

  B73 @0xaed72c:
    aed72c: WRITE [ebp+0x5b0] and= 0xbfffffff   ; dword ptr [ebp + 0x5b0], 0xbfffffff
    aed736: goto B30

  B74 @0xaed73b:
    aed747: CALL 0xb74ef0 xtbl_get_float  regs[ecx=0xe47f1c "Slerp_Value", esi=&[ebp+0x4cc]]  stack[]
    aed757: X87 STORE f32 [eax] <- f32[esi]  (rounds to f32)
    aed761: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47f28 "From_Sticky_Slerp_Value", esi=&[ebp+0x4d0]]  stack[]
    aed775: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47f40 "Min_Angle", esi=&[ebp+0x4d4]]  stack[]
    aed77f: if (al == 0) goto B76  else fallthrough   ; je 0xaed795

  B75 @0xaed781:
    aed783: WRITE [ebp+0x5b0] or= 0x80000000   ; dword ptr [ebp + 0x5b0], 0x80000000
    aed793: X87 STORE f32 [esi] <- (f32[esi] * f64[0xe5c4e8] =f64 0.01745329238474369 (0x3f91df46a0000000))  (rounds to f32)

  B76 @0xaed795:
    aed795: WRITE [ebp+0x5b0] or= 0x40000000   ; dword ptr [ebp + 0x5b0], 0x40000000
    aed79f: goto B30

  B77 @0xaed7a4:
    aed7a8: if (esi == 0) goto B81  else fallthrough   ; je 0xaed7c1

  B78 @0xaed7aa:
    aed7b1: CALL 0xbfbee0 xml_find_child  regs[eax=esi]  stack[]
    aed7b8: if (ret(xml_find_child) != 0) goto B80  else fallthrough   ; jne 0xaed7be

  B79 @0xaed7ba:
    aed7bc: goto B81

  B80 @0xaed7be:

  B81 @0xaed7c1:
    aed7cd: repe cmpsb byte ptr [esi], byte ptr es:[edi]   ; ecx=0x5 esi=? edi=0xdd979c "none"
    aed7d1: setne al  ; jne (flags from earlier instr)
    aed7e0: WRITE [ebp+0x7c8] mov= 0   ; dword ptr [ebp + 0x7c8], eax
    aed7e6: CALL 0xb74ef0 xtbl_get_float  regs[ecx=0xe47f64 "Intensity_Multiplier", esi=&[ebp+0x7cc]]  stack[]
    aed804: WRITE [ebp+0x5b0] xor= (edx and 0x8000)   ; dword ptr [ebp + 0x5b0], edx

  B82 @0xaed80a:
    aed815: CALL 0xbfbee0 xml_find_child  regs[eax=i32[esp+0x20]]  stack[]
    aed81c: if (ret(xml_find_child) == 0) goto B86  else fallthrough   ; je 0xaed841

  B83 @0xaed81e:
    aed823: if (i32[eax+0xc] == 0) goto B86  else fallthrough   ; je 0xaed841

  B84 @0xaed825:
    aed82b: CALL 0xc99030 __stricmp  regs[]  stack[eax, 0xddc0dc "yes"]
    aed835: if (ret(__stricmp) != 0) goto B86  else fallthrough   ; jne 0xaed841

  B85 @0xaed837:
    aed837: WRITE [ebp+0x5b0] or= 0x8000   ; dword ptr [ebp + 0x5b0], 0x8000

  B86 @0xaed841:
    aed846: if (i32[esp+0x24] == 0x0) goto B89  else fallthrough   ; je 0xaed860

  B87 @0xaed848:
    aed848: WRITE [ebp+0x5b0] and= 0xfdffffff   ; dword ptr [ebp + 0x5b0], 0xfdffffff
    aed852: goto B90

  B88 @0xaed854:
    aed854: WRITE [ebp+0x5b0] and= 0xffff7fff   ; dword ptr [ebp + 0x5b0], 0xffff7fff
    aed85e: goto B82

  B89 @0xaed860:
    aed876: WRITE [ebp+0x5b0] xor= (ecx and 0x2000000)   ; dword ptr [ebp + 0x5b0], ecx

  B90 @0xaed87c:
    aed883: CALL 0xbfbee0 xml_find_child  regs[eax=ebx]  stack[]
    aed88a: if (ret(xml_find_child) == 0) goto B94  else fallthrough   ; je 0xaed8af

  B91 @0xaed88c:
    aed891: if (i32[eax+0xc] == 0) goto B94  else fallthrough   ; je 0xaed8af

  B92 @0xaed893:
    aed899: CALL 0xc99030 __stricmp  regs[]  stack[eax, 0xddc0dc "yes"]
    aed8a3: if (ret(__stricmp) != 0) goto B94  else fallthrough   ; jne 0xaed8af

  B93 @0xaed8a5:
    aed8a5: WRITE [ebp+0x5b0] or= 0x2000000   ; dword ptr [ebp + 0x5b0], 0x2000000

  B94 @0xaed8af:
    aed8b8: X87 STORE f32 [esi] <- 1.0  (rounds to f32)
    aed8bf: CALL 0xb74f60 xtbl_get_float_opt  regs[ecx=0xe47fa8 "camera_fov_scale", esi=&[ebp+0x5a4]]  stack[]
    aed8cc: if (i32[esp+0x24] != 0x0) goto B96  else fallthrough   ; jne 0xaed8e2

  B95 @0xaed8ce:
    aed8da: CALL 0xb74ef0 xtbl_get_float  regs[ecx=0xe47fbc "Max_FOV", esi=&[ebp+0x5a8]]  stack[]

  B96 @0xaed8e2:
    aed8eb: RETURN

  B97 @0xaed8ec:
    aed8f5: RETURN

  call targets: 0x4958f0, 0xae5f20, 0xae60f0, 0xae6200, 0xae6300, 0xae6450, 0xb74ef0(xtbl_get_float), 0xb74f60(xtbl_get_float_opt), 0xbfbee0(xml_find_child), 0xc99030(__stricmp)
  stack objects passed by address: &[ebp+0x4b8] -> xtbl_get_float; &[ebp+0x4bc] -> xtbl_get_float_opt; &[ebp+0x4c0] -> xtbl_get_float_opt; &[ebp+0x4c8] -> xtbl_get_float; &[ebp+0x4cc] -> xtbl_get_float; &[ebp+0x4d0] -> xtbl_get_float_opt; &[ebp+0x4d4] -> xtbl_get_float_opt; &[ebp+0x4d8] -> xtbl_get_float_opt; &[ebp+0x4dc] -> xtbl_get_float_opt; &[ebp+0x5a4] -> xtbl_get_float_opt; &[ebp+0x5a8] -> xtbl_get_float; &[ebp+0x7cc] -> xtbl_get_float
```

### camera_follow_vehicle_chase_update — 0x497220

**Role (inferred).** Vehicle chase maths for the follow camera. Looks the target `0x25F5BB0` up in the object table (`0x2149C64`), makes one virtual call at `[obj] +0x90` of the object in `0x27B3C60`, normalises `vec3_normalize_safe` on `ebp+0xC`, reads the vehicle speed helpers `0xAA63A0 / 0xAA8290`, and eases with `timer_remaining(0x25F5CBC)` and two `crt_pow_x87` calls (`0xCA05D0`; operands on ST0 / ST1). Callers: `0x498A20` only.


**Block map (generated):**

```
FUNCTION 0x00497220  size 0x558  insns 441  blocks 58
  entry regs read before write: ['edx']

  B0 @0x497220:
    497226: READ  ecx <- i32[0x25f5bb0]
    497234: if (i32[0x25f5bb0] == 0) goto B3  else fallthrough   ; je 0x49724f

  B1 @0x497236:
    497248: if (i32[eax+0x2149c64] == 0) goto B3  else fallthrough   ; je 0x49724f

  B2 @0x49724a:
    49724d: if (i32[eax+0x44] == ecx) goto B6  else fallthrough   ; je 0x4972c0

  B3 @0x49724f:

  B4 @0x497253:
    497267: X87 STORE f32 [esp+0x1c] <- (f32[0x25f5b50] * f32[0xe9a654] =f32 0 (0x00000000))  (rounds to f32)
    497273: X87 STORE f32 [esp+0x20] <- (f32[0x25f5b54] * f32[0xe9a654] =f32 0 (0x00000000))  (rounds to f32)
    49727d: X87 STORE f32 [esp+0x24] <- (f32[0xe9a654] =f32 0 (0x00000000) * f32[0x25f5b58])  (rounds to f32)
    497289: X87 STORE f32 [esp+0x28] <- (f32[edx] - f32[0x25f5b14])  (rounds to f32)
    497296: X87 STORE f32 [esp+0x2c] <- (f32[edx+0x4] - f32[0x25f5b18])  (rounds to f32)
    4972a3: X87 STORE f32 [esp+0x30] <- (f32[edx+0x8] - f32[0x25f5b1c])  (rounds to f32)
    4972a7: if (bl == 0) goto B8  else fallthrough   ; je 0x4972e2

  B5 @0x4972a9:
    4972b2: WRITE [ebp+0xc] mov= i32[edi+0x38]   ; dword ptr [ebp + 0xc], edx
    4972b5: WRITE [ebp+0x10] mov= i32[edi+0x3c]   ; dword ptr [ebp + 0x10], eax
    4972b8: WRITE [ebp+0x14] mov= i32[edi+0x40]   ; dword ptr [ebp + 0x14], ecx
    4972bb: goto B18

  B6 @0x4972c0:
    4972c4: if (i32[eax+0x4c] != 0x5) goto B3  else fallthrough   ; jne 0x49724f

  B7 @0x4972c6:
    4972da: setbe bl  ; (eax add 0xfffffffe) <=u 0x2
    4972dd: goto B4

  B8 @0x4972e2:
    4972e4: READ  eax <- i32[0x27b3c60]
    4972eb: X87 STORE f32 [esp+0xc] <- 1.0  (rounds to f32)
    4972ef: if (i32[0x27b3c60] == 0) goto B13  else fallthrough   ; je 0x49731a

  B9 @0x4972f1:
    4972f5: if (i32[eax+0x3c] != 0x1) goto B13  else fallthrough   ; jne 0x49731a

  B10 @0x4972f7:
    497302: CALL indirect eax  ecx=i32[eax+0x90] stack[]
    497306: if (ret(indirect) != 0) goto B13  else fallthrough   ; jne 0x49731a

  B11 @0x497308:
    49730e: if (i32[0x2526b9a] == al) goto B13  else fallthrough   ; je 0x49731a

  B12 @0x497310:
    497316: X87 STORE f32 [esp+0xc] <- f32[0xdd14c4] =f32 1600 (0x44c80000)  (rounds to f32)

  B13 @0x49731a:
    497330: X87 STORE f32 [esp+0x14] <- (((0.0 * 0.0) + (f32[ebp+0xc] * f32[ebp+0xc])) + (f32[ebp+0x14] * f32[ebp+0x14]))  (rounds to f32)
    497338: X87 COMPARE f32[esp+0x14]  vs  f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)   [fcomp]
    497343: if ((f32[esp+0x14] > f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)) OR (f32[esp+0x14] == f32[0xdd0584] =f32 0.00999999978 (0x3c23d70a)) OR UNORDERED(NaN)) goto B15  else fallthrough   ; jp 0x497367

  B14 @0x497345:
    497359: WRITE [ebp+0xc] mov= i32[esp+0x1c]   ; dword ptr [ebp + 0xc], ecx
    49735c: WRITE [ebp+0x10] mov= i32[esp+0x20]   ; dword ptr [ebp + 0x10], edx
    49735f: WRITE [ebp+0x14] mov= i32[esp+0x24]   ; dword ptr [ebp + 0x14], eax
    497362: goto B19

  B15 @0x497367:
    497374: X87 STORE f32 [esp+0x14] <- ((f32[ebp+0x14] * f32[ebp+0x14]) + ((f32[ebp+0xc] * f32[ebp+0xc]) + (f32[ebp+0x10] * f32[ebp+0x10])))  (rounds to f32)
    497380: X87 COMPARE f32[esp+0xc]  vs  f32[esp+0x14]   [fcom]
    497387: if ((f32[esp+0xc] < f32[esp+0x14]) OR (f32[esp+0xc] == f32[esp+0x14]) OR UNORDERED(NaN)) goto B17  else fallthrough   ; jne 0x497416

  B16 @0x49738d:
    49738f: X87 STORE f32 [esp+0x14] <- f32[esp+0x14]  (rounds to f32)
    49739d: X87 STORE f32 [esp+0x14] <- (f32[esp+0xc] - f32[esp+0x14])  (rounds to f32)
    4973ab: X87 STORE f32 [esp+0x40] <- (f32[esp+0x14] * f32[esp+0x1c])  (rounds to f32)
    4973b5: X87 STORE f32 [esp+0x44] <- (f32[esp+0x20] * f32[esp+0x14])  (rounds to f32)
    4973bd: X87 STORE f32 [esp+0x48] <- (f32[esp+0x14] * f32[esp+0x24])  (rounds to f32)
    4973c7: X87 STORE f32 [esp+0x34] <- (f32[ebp+0xc] * f32[esp+0x14])  (rounds to f32)
    4973d1: X87 STORE f32 [esp+0x38] <- (f32[ebp+0x10] * f32[esp+0x14])  (rounds to f32)
    4973d7: X87 STORE f32 [esp+0x3c] <- (f32[esp+0x14] * f32[ebp+0x14])  (rounds to f32)
    4973e3: X87 STORE f32 [esp+0x4c] <- (f32[esp+0x34] + f32[esp+0x40])  (rounds to f32)
    4973ef: WRITE [ebp+0xc] mov= i32[esp+0x4c]   ; dword ptr [ebp + 0xc], ecx
    4973f6: X87 STORE f32 [esp+0x50] <- (f32[esp+0x38] + f32[esp+0x44])  (rounds to f32)
    497402: WRITE [ebp+0x10] mov= i32[esp+0x50]   ; dword ptr [ebp + 0x10], edx
    497409: X87 STORE f32 [esp+0x54] <- (f32[esp+0x3c] + f32[esp+0x48])  (rounds to f32)
    497411: WRITE [ebp+0x14] mov= i32[esp+0x54]   ; dword ptr [ebp + 0x14], eax
    497414: goto B18

  B17 @0x497416:

  B18 @0x497420:
    497423: CALL 0xbde150 vec3_normalize_safe  regs[esi=&[ebp+0xc]]  stack[]
    497433: X87 STORE f32 [ebp+0xc] <- (f32[0xe9a654] =f32 0 (0x00000000) * f32[ebp+0xc])  (rounds to f32)
    49743b: X87 STORE f32 [ebp+0x10] <- (f32[ebp+0x10] * f32[0xe9a654] =f32 0 (0x00000000))  (rounds to f32)
    497441: X87 STORE f32 [ebp+0x14] <- (f32[0xe9a654] =f32 0 (0x00000000) * f32[ebp+0x14])  (rounds to f32)

  B19 @0x497444:
    497448: if (edi == 0) goto B28  else fallthrough   ; je 0x4974a3

  B20 @0x49744a:
    49745b: if (i32[ecx+0x9c] != 0x4) goto B24  else fallthrough   ; jne 0x49747f

  B21 @0x49745d:
    49745d: CALL 0xaa8290 sub_aa8290  regs[ecx=?]  stack[]
    497464: if (al == 0) goto B27  else fallthrough   ; je 0x4974a1

  B22 @0x497466:
    497469: CALL 0xaa63a0 sub_aa63a0  regs[eax=i32[edi+0x44]]  stack[]
    497470: X87 COMPARE 0.0  vs  ST0<ret sub_aa63a0>   [fcom]
    497479: if ((0.0 < ST0<ret sub_aa63a0>) OR (0.0 == ST0<ret sub_aa63a0>) OR UNORDERED(NaN)) goto B28  else fallthrough   ; jne 0x4974a3

  B23 @0x49747b:
    49747d: goto B29

  B24 @0x49747f:
    49747f: CALL 0xaa8290 sub_aa8290  regs[ecx=?]  stack[]
    497486: if (al == 0) goto B27  else fallthrough   ; je 0x4974a1

  B25 @0x497488:
    49748b: CALL 0xaa63a0 sub_aa63a0  regs[eax=i32[edi+0x44]]  stack[]
    497492: X87 COMPARE 0.0  vs  ST0<ret sub_aa63a0>   [fucom]
    49749b: if ((0.0 == ST0<ret sub_aa63a0>)) goto B28  else fallthrough   ; jnp 0x4974a3

  B26 @0x49749d:
    49749f: goto B29

  B27 @0x4974a1:

  B28 @0x4974a3:

  B29 @0x4974a5:
    4974a7: if (bl == 0) goto B32  else fallthrough   ; je 0x4974dd

  B30 @0x4974a9:
    4974ab: if (dl == 0) goto B32  else fallthrough   ; je 0x4974dd

  B31 @0x4974ad:
    4974b2: X87 STORE f32 [esp+0x4c] <- -f32[ebp+0xc]  (rounds to f32)
    4974bd: WRITE [ebp+0xc] mov= i32[esp+0x4c]   ; dword ptr [ebp + 0xc], eax
    4974c2: X87 STORE f32 [esp+0x50] <- -f32[ebp+0x10]  (rounds to f32)
    4974cd: WRITE [ebp+0x10] mov= i32[esp+0x50]   ; dword ptr [ebp + 0x10], ecx
    4974d2: X87 STORE f32 [esp+0x54] <- -f32[ebp+0x14]  (rounds to f32)
    4974da: WRITE [ebp+0x14] mov= i32[esp+0x54]   ; dword ptr [ebp + 0x14], eax

  B32 @0x4974dd:
    4974ed: X87 STORE f32 [esp+0x14] <- (f32[ebp+0x18] / f64[0xe5cca8] =f64 0.0083333337679505348 (0x3f81111120000000))  (rounds to f32)
    4974f1: if (i32[0x2527c08] <= 0x0) goto B34  else fallthrough   ; jle 0x4974fb

  B33 @0x4974f3:
    4974f9: goto B35

  B34 @0x4974fb:

  B35 @0x497501:
    497503: X87 STORE f32 [esp+0x10] <- f32[0x25f5cb4]  (rounds to f32)
    497507: if (edi == 0) goto B39  else fallthrough   ; je 0x497525

  B36 @0x497509:
    497509: READ  eax <- i32[0x25f5cbc]
    497510: if (i32[0x25f5cbc] < 0 (signed)) goto B39  else fallthrough   ; jl 0x497525

  B37 @0x497512:
    497512: READ  ecx <- i32[0xe843ac] =i32 1
    49751a: if (eax <= i32[0xe843ac] =i32 1) goto B44  else fallthrough   ; jle 0x49756a

  B38 @0x49751c:
    497523: if ((eax sub ecx) < 0x35a4e900) goto B45  else fallthrough   ; jl 0x497574

  B39 @0x497525:

  B40 @0x49752b:
    49752b: X87 STORE f32 [esp+0xc] <- f32[0x25f5cb8]  (rounds to f32)
    497533: X87 COMPARE f32[esp+0x10]  vs  ST0<ret sub_aa63a0>   [fcom]
    49753a: if ((f32[esp+0x10] > ST0<ret sub_aa63a0>) OR (f32[esp+0x10] == ST0<ret sub_aa63a0>) OR UNORDERED(NaN)) goto B52  else fallthrough   ; jp 0x497690

  B41 @0x497540:
    497546: X87 STORE f32 [esp+0xc] <- (-f32[esp+0x10] * f32[esp+0xc])  (rounds to f32)
    49754e: X87 COMPARE f32[esp+0xc]  vs  ST0<ret sub_aa63a0>   [fcom]
    497555: if ((f32[esp+0xc] < ST0<ret sub_aa63a0>) OR (f32[esp+0xc] == ST0<ret sub_aa63a0>) OR UNORDERED(NaN)) goto B49  else fallthrough   ; jne 0x4975cd

  B42 @0x497557:
    49755b: X87 COMPARE 1.0  vs  ST0<ret sub_aa63a0>   [fcom]
    497564: if ((1.0 < ST0<ret sub_aa63a0>) OR (1.0 == ST0<ret sub_aa63a0>) OR UNORDERED(NaN)) goto B50  else fallthrough   ; jne 0x4975cf

  B43 @0x497566:
    497568: goto B51

  B44 @0x49756a:
    497572: if ((ecx sub eax) < 0x35a4e900) goto B39  else fallthrough   ; jl 0x497525

  B45 @0x497574:
    497576: if (dl == 0) goto B47  else fallthrough   ; je 0x497586

  B46 @0x497578:
    497584: goto B48

  B47 @0x497586:

  B48 @0x497592:
    497597: X87 STORE f32 [esp+0xc] <- f32[ecx+0x4dc]  (rounds to f32)
    49759b: CALL 0xbdd260 timer_remaining  regs[eax=0x25f5cbc]  stack[]
    4975a0: WRITE [esp+0x18] mov= ret(timer_remaining)   ; dword ptr [esp + 0x18], eax
    4975ae: X87 STORE f32 [esp+0x18] <- ((float)i32[esp+0x18] / f64[0xe5bf98] =f64 10000 (0x40c3880000000000))  (rounds to f32)
    4975c8: goto B40

  B49 @0x4975cd:

  B50 @0x4975cf:
    4975cf: X87 STORE f32 [esp+0xc] <- ST0<ret timer_remaining>  (rounds to f32)

  B51 @0x4975d3:
    4975db: X87 STORE f32 [esp+0x18] <- (1.0 - f32[esp+0xc])  (rounds to f32)
    4975e7: CALL 0xca05d0 crt_pow_x87  regs[]  stack[]
    4975ec: X87 STORE f32 [esp+0x18] <- f32[esp+0x14]  (rounds to f32)
    4975fb: X87 STORE f32 [esp+0x18] <- (1.0 - f32[esp+0x18])  (rounds to f32)
    49760d: X87 STORE f32 [esp+0x4c] <- (f32[esp+0x1c] - f32[esp+0x28])  (rounds to f32)
    49761f: X87 STORE f32 [esp+0x50] <- (f32[esp+0x20] - f32[esp+0x2c])  (rounds to f32)
    497631: X87 STORE f32 [esp+0x54] <- (f32[esp+0x24] - f32[esp+0x30])  (rounds to f32)
    497643: X87 STORE f32 [esp+0x40] <- (f32[esp+0x4c] * f32[esp+0x18])  (rounds to f32)
    49764d: X87 STORE f32 [esp+0x44] <- (f32[esp+0x50] * f32[esp+0x18])  (rounds to f32)
    497655: X87 STORE f32 [esp+0x48] <- (f32[esp+0x18] * f32[esp+0x54])  (rounds to f32)
    497661: X87 STORE f32 [esp+0x4c] <- (f32[esp+0x28] + f32[esp+0x40])  (rounds to f32)
    497669: WRITE [eax] mov= i32[esp+0x4c]   ; dword ptr [eax], ecx
    49766f: X87 STORE f32 [esp+0x50] <- (f32[esp+0x2c] + f32[esp+0x44])  (rounds to f32)
    497677: WRITE [eax+0x4] mov= i32[esp+0x50]   ; dword ptr [eax + 4], edx
    49767e: X87 STORE f32 [esp+0x54] <- (f32[esp+0x30] + f32[esp+0x48])  (rounds to f32)
    497686: WRITE [eax+0x8] mov= i32[esp+0x54]   ; dword ptr [eax + 8], ecx
    49768f: RETURN

  B52 @0x497690:
    497694: X87 STORE f32 [esp+0xc] <- (f32[esp+0x10] * f32[esp+0xc])  (rounds to f32)
    49769c: X87 COMPARE f32[esp+0xc]  vs  ST0<ret crt_pow_x87>   [fcom]
    4976a3: if ((f32[esp+0xc] < ST0<ret crt_pow_x87>) OR (f32[esp+0xc] == ST0<ret crt_pow_x87>) OR UNORDERED(NaN)) goto B55  else fallthrough   ; jne 0x4976b8

  B53 @0x4976a5:
    4976a9: X87 COMPARE 1.0  vs  ST0<ret crt_pow_x87>   [fcom]
    4976b2: if ((1.0 < ST0<ret crt_pow_x87>) OR (1.0 == ST0<ret crt_pow_x87>) OR UNORDERED(NaN)) goto B56  else fallthrough   ; jne 0x4976ba

  B54 @0x4976b4:
    4976b6: goto B57

  B55 @0x4976b8:

  B56 @0x4976ba:
    4976ba: X87 STORE f32 [esp+0xc] <- ST0<ret crt_pow_x87>  (rounds to f32)

  B57 @0x4976be:
    4976c6: X87 STORE f32 [esp+0x18] <- (1.0 - f32[esp+0xc])  (rounds to f32)
    4976d2: CALL 0xca05d0 crt_pow_x87  regs[]  stack[]
    4976d7: X87 STORE f32 [esp+0x18] <- f32[esp+0x14]  (rounds to f32)
    4976e9: X87 STORE f32 [esp+0xc] <- (1.0 - f32[esp+0x18])  (rounds to f32)
    4976fa: X87 STORE f32 [esp+0x40] <- (f32[ebp+0xc] - f32[esp+0x1c])  (rounds to f32)
    49770b: X87 STORE f32 [esp+0x44] <- (f32[ebp+0x10] - f32[esp+0x20])  (rounds to f32)
    49771c: X87 STORE f32 [esp+0x48] <- (f32[ebp+0x14] - f32[esp+0x24])  (rounds to f32)
    49772e: X87 STORE f32 [esp+0x34] <- (f32[esp+0x40] * f32[esp+0xc])  (rounds to f32)
    497738: X87 STORE f32 [esp+0x38] <- (f32[esp+0x44] * f32[esp+0xc])  (rounds to f32)
    497740: X87 STORE f32 [esp+0x3c] <- (f32[esp+0xc] * f32[esp+0x48])  (rounds to f32)
    49774c: X87 STORE f32 [esp+0x40] <- (f32[esp+0x1c] + f32[esp+0x34])  (rounds to f32)
    497754: WRITE [eax] mov= i32[esp+0x40]   ; dword ptr [eax], edx
    49775a: X87 STORE f32 [esp+0x44] <- (f32[esp+0x20] + f32[esp+0x38])  (rounds to f32)
    497762: WRITE [eax+0x4] mov= i32[esp+0x44]   ; dword ptr [eax + 4], ecx
    497769: X87 STORE f32 [esp+0x48] <- (f32[esp+0x24] + f32[esp+0x3c])  (rounds to f32)
    497771: WRITE [eax+0x8] mov= i32[esp+0x48]   ; dword ptr [eax + 8], edx
    497777: RETURN

  call targets: 0xaa63a0, 0xaa8290, 0xbdd260(timer_remaining), 0xbde150(vec3_normalize_safe), 0xca05d0(crt_pow_x87)
  stack objects passed by address: &[ebp+0xc] -> vec3_normalize_safe
```

### camera_shake_apply — 0x4A01B0

**Role.** Called by `camera_effects_update` (0x4A0EA0, step 6 of `camera_update`). Uses the shake times `0xE9A6BC / 0xE9A6C4` (ms clock `0xE843AC` wrapped at `0x6B49D200`), `timer_remaining`, `rng_float01` and `0xBE50F0` to add a random offset to the camera pose, gated by `0x960120` and `0x9A0270`. 164 x87 ops.


**Block map (generated):**

```
FUNCTION 0x004A01B0  size 0x546  insns 397  blocks 87
  entry regs read before write: []

  B0 @0x4a01b0:
    4a01b5: READ  eax <- i32[0xe9a6bc] =i32 -1
    4a01bc: X87 STORE f32 [0x25f5c50] <- 0.0  (rounds to f32)
    4a01c3: X87 STORE f32 [0x25f5c68] <- 0.0  (rounds to f32)
    4a01cb: READ  edi <- i32[0xe843ac] =i32 1
    4a01d1: WRITE [esp+0x13] mov= 0x0   ; byte ptr [esp + 0x13], 0
    4a01d8: if (i32[0xe9a6bc] =i32 -1 < 0 (signed)) goto B13  else fallthrough   ; jl 0x4a02ac

  B1 @0x4a01de:
    4a01e0: if (eax <= edi) goto B7  else fallthrough   ; jle 0x4a020d

  B2 @0x4a01e2:
    4a01e9: if ((eax sub edi) >= 0x35a4e900) goto B8  else fallthrough   ; jge 0x4a0219

  B3 @0x4a01eb:
    4a01eb: READ  eax <- i32[0xe9a6c4] =i32 -1
    4a01f4: if (i32[0xe9a6c4] =i32 -1 < 0 (signed)) goto B86  else fallthrough   ; jl 0x4a06ef

  B4 @0x4a01fa:
    4a01fc: if (eax <= edi) goto B9  else fallthrough   ; jle 0x4a0233

  B5 @0x4a01fe:
    4a0205: if ((eax sub edi) < 0x35a4e900) goto B86  else fallthrough   ; jl 0x4a06ef

  B6 @0x4a020b:
    4a020b: goto B10

  B7 @0x4a020d:
    4a0217: if ((ecx sub eax) >= 0x35a4e900) goto B3  else fallthrough   ; jge 0x4a01eb

  B8 @0x4a0219:
    4a021c: X87 STORE f32 [0x25f5c68] <- 0.0  (rounds to f32)
    4a0222: WRITE [0xe9a6bc] mov= (eax or 0xffffffff)   ; dword ptr [0xe9a6bc], eax
    4a0227: WRITE [0xe9a6c4] mov= (eax or 0xffffffff)   ; dword ptr [0xe9a6c4], eax
    4a0232: RETURN

  B9 @0x4a0233:
    4a023d: if ((edx sub eax) >= 0x35a4e900) goto B86  else fallthrough   ; jge 0x4a06ef

  B10 @0x4a0243:
    4a024a: if (i32[0xf53ab4] =i32 856044208 == 0x0) goto B12  else fallthrough   ; je 0x4a029e

  B11 @0x4a024c:
    4a0255: X87 STORE f32 [esp+0x4] <- f32[0xdd1c8c] =f32 0.5 (0x3f000000)  (rounds to f32)
    4a0260: X87 STORE f32 [esp] <- 1.0  (rounds to f32)
    4a0263: CALL 0xbdd260 timer_remaining  regs[eax=0xe9a6bc]  stack[]
    4a0268: WRITE [esp+0x20] mov= ret(timer_remaining)   ; dword ptr [esp + 0x20], eax
    4a0277: X87 STORE f32 [esp+0x24] <- ((float)i32[esp+0x20] / i32[0xf53ab4] =i32 856044208)  (rounds to f32)
    4a027f: X87 STORE f32 [esp] <- f32[esp+0x24]  (rounds to f32)
    4a0282: CALL 0xbf0f50 sub_bf0f50  regs[]  stack[ret(timer_remaining)]
    4a028d: READ  esi <- i32[0x2526c24]
    4a029c: goto B18

  B12 @0x4a029e:
    4a02a4: READ  esi <- i32[0x2526c24]
    4a02aa: goto B18

  B13 @0x4a02ac:
    4a02ac: READ  ecx <- i32[0x21703d4]
    4a02c1: if ((bl & dl) == 0) goto B16  else fallthrough   ; je 0x4a02d3

  B14 @0x4a02c3:
    4a02c8: if ((bl & al) != 0) goto B16  else fallthrough   ; jne 0x4a02d3

  B15 @0x4a02ca:
    4a02ca: CALL 0x9a0270 sub_9a0270  regs[ecx=?]  stack[]
    4a02d1: goto B18

  B16 @0x4a02d3:
    4a02d5: CALL 0x960120 sub_960120  regs[esi=ecx]  stack[]
    4a02dc: if (al == 0) goto B86  else fallthrough   ; je 0x4a06ef

  B17 @0x4a02e2:
    4a02ed: WRITE [esp+0x13] mov= bl   ; byte ptr [esp + 0x13], bl

  B18 @0x4a02f1:
    4a02f1: READ  eax <- i32[0x25f6888]
    4a02f6: X87 STORE f32 [esp+0x14] <- ST0<ret sub_960120>  (rounds to f32)
    4a0300: WRITE [esp+0x18] mov= esi   ; dword ptr [esp + 0x18], esi
    4a0304: if (i32[0x25f6888] < 0 (signed)) goto B23  else fallthrough   ; jl 0x4a0321

  B19 @0x4a0306:
    4a0308: if (eax <= edi) goto B22  else fallthrough   ; jle 0x4a0315

  B20 @0x4a030a:
    4a0311: if ((eax sub edi) < 0x35a4e900) goto B29  else fallthrough   ; jl 0x4a0381

  B21 @0x4a0313:
    4a0313: goto B23

  B22 @0x4a0315:
    4a031f: if ((ecx sub eax) >= 0x35a4e900) goto B29  else fallthrough   ; jge 0x4a0381

  B23 @0x4a0321:
    4a032c: X87 STORE f32 [0x2526c38] <- f32[0x2526c28]  (rounds to f32)
    4a0334: X87 STORE f32 [esp+0x1c] <- -f32[esp+0x14]  (rounds to f32)
    4a0338: CALL 0xbdce30 rng_float01  regs[eax=0xec30f8]  stack[]
    4a034c: WRITE [0x25f6888] mov= &[edi+esi]   ; dword ptr [0x25f6888], eax
    4a035d: X87 STORE f32 [0x2526c28] <- f32[esp+0x14]  (rounds to f32)
    4a0363: if (esi < 0 (signed)) goto B26  else fallthrough   ; jl 0x4a0373

  B24 @0x4a0365:
    4a036a: if (eax <= 0x6b49d200) goto B29  else fallthrough   ; jle 0x4a0381

  B25 @0x4a036c:
    4a0371: goto B28

  B26 @0x4a0373:
    4a0375: if (eax >= 0 (signed)) goto B29  else fallthrough   ; jge 0x4a0381

  B27 @0x4a0377:

  B28 @0x4a037c:
    4a037c: WRITE [0x25f6888] mov= eax   ; dword ptr [0x25f6888], eax

  B29 @0x4a0381:
    4a0383: if (esi == 0) goto B31  else fallthrough   ; je 0x4a03a1

  B30 @0x4a0385:
    4a038a: CALL 0xbdd260 timer_remaining  regs[eax=0x25f6888]  stack[]
    4a038f: WRITE [esp+0x1c] mov= ret(timer_remaining)   ; dword ptr [esp + 0x1c], eax
    4a039f: goto B32

  B31 @0x4a03a1:

  B32 @0x4a03a3:
    4a03a9: READ  eax <- i32[0x25f688c]
    4a03bc: X87 STORE f32 [esp+0x1c] <- (1.0 - ((float)i32[esp+0x1c] / i32[esp+0x18]))  (rounds to f32)
    4a03c6: X87 STORE f32 [0x25f5c50] <- (f32[0x2526c38] + ((f32[0x2526c28] - f32[0x2526c38]) * f32[esp+0x1c]))  (rounds to f32)
    4a03cc: if (i32[0x25f688c] < 0 (signed)) goto B39  else fallthrough   ; jl 0x4a040d

  B33 @0x4a03ce:
    4a03d0: if (eax <= edi) goto B38  else fallthrough   ; jle 0x4a0401

  B34 @0x4a03d2:
    4a03d9: if ((eax sub edi) >= 0x35a4e900) goto B39  else fallthrough   ; jge 0x4a040d

  B35 @0x4a03db:

  B36 @0x4a03dd:
    4a03df: if (esi == 0) goto B44  else fallthrough   ; je 0x4a047a

  B37 @0x4a03e5:
    4a03ea: CALL 0xbdd260 timer_remaining  regs[eax=0x25f688c]  stack[]
    4a03ef: WRITE [esp+0x1c] mov= ret(timer_remaining)   ; dword ptr [esp + 0x1c], eax
    4a03ff: goto B45

  B38 @0x4a0401:
    4a040b: if ((edx sub eax) >= 0x35a4e900) goto B35  else fallthrough   ; jge 0x4a03db

  B39 @0x4a040d:
    4a0418: X87 STORE f32 [0x2526c3c] <- f32[0x2526c2c]  (rounds to f32)
    4a0420: X87 STORE f32 [esp+0x1c] <- -f32[esp+0x14]  (rounds to f32)
    4a0424: CALL 0xbdce30 rng_float01  regs[eax=0xec30f8]  stack[]
    4a0438: WRITE [0x25f688c] mov= &[edi+esi]   ; dword ptr [0x25f688c], eax
    4a0445: X87 STORE f32 [0x2526c2c] <- ST0<ret rng_float01>  (rounds to f32)
    4a044b: if (esi < 0 (signed)) goto B42  else fallthrough   ; jl 0x4a0463

  B40 @0x4a044d:
    4a0452: if (eax <= 0x6b49d200) goto B36  else fallthrough   ; jle 0x4a03dd

  B41 @0x4a0454:
    4a0459: WRITE [0x25f688c] mov= (eax sub 0x6b49d200)   ; dword ptr [0x25f688c], eax
    4a045e: goto B36

  B42 @0x4a0463:
    4a0465: if (eax >= 0 (signed)) goto B36  else fallthrough   ; jge 0x4a03dd

  B43 @0x4a046b:
    4a0470: WRITE [0x25f688c] mov= (eax add 0x6b49d200)   ; dword ptr [0x25f688c], eax
    4a0475: goto B36

  B44 @0x4a047a:

  B45 @0x4a047c:
    4a048f: X87 STORE f32 [esp+0x20] <- (1.0 - ((float)i32[esp+0x1c] / i32[esp+0x18]))  (rounds to f32)
    4a0499: X87 STORE f32 [esp+0x20] <- (f32[0x2526c3c] + ((f32[0x2526c2c] - f32[0x2526c3c]) * f32[esp+0x20]))  (rounds to f32)
    4a04a1: X87 STORE f32 [esp] <- f32[esp+0x20]  (rounds to f32)
    4a04a9: CALL 0xbe50f0 sub_be50f0  regs[]  stack[0x25f5c2c, ecx]
    4a04ae: READ  eax <- i32[0x25f6890]
    4a04b5: if (i32[0x25f6890] < 0 (signed)) goto B50  else fallthrough   ; jl 0x4a04d6

  B46 @0x4a04b7:
    4a04b9: if (eax <= edi) goto B49  else fallthrough   ; jle 0x4a04ca

  B47 @0x4a04bb:
    4a04c2: if ((eax sub edi) < 0x35a4e900) goto B56  else fallthrough   ; jl 0x4a054a

  B48 @0x4a04c8:
    4a04c8: goto B50

  B49 @0x4a04ca:
    4a04d4: if ((ecx sub eax) >= 0x35a4e900) goto B56  else fallthrough   ; jge 0x4a054a

  B50 @0x4a04d6:
    4a04e1: X87 STORE f32 [0x2526c40] <- f32[0x2526c30]  (rounds to f32)
    4a04f3: X87 STORE f32 [esp+0x20] <- (-f32[esp+0x14] * f64[0xe5bf70] =f64 0.5 (0x3fe0000000000000))  (rounds to f32)
    4a04f7: CALL 0xbdce30 rng_float01  regs[eax=0xec30f8]  stack[]
    4a050b: WRITE [0x25f6890] mov= &[edi+esi]   ; dword ptr [0x25f6890], eax
    4a0510: X87 STORE f32 [esp+0x1c] <- (f32[esp+0x14] * f64[0xe5bf70] =f64 0.5 (0x3fe0000000000000))  (rounds to f32)
    4a0526: X87 STORE f32 [0x2526c30] <- ST0<ret rng_float01>  (rounds to f32)
    4a052c: if (esi < 0 (signed)) goto B53  else fallthrough   ; jl 0x4a053c

  B51 @0x4a052e:
    4a0533: if (eax <= 0x6b49d200) goto B56  else fallthrough   ; jle 0x4a054a

  B52 @0x4a0535:
    4a053a: goto B55

  B53 @0x4a053c:
    4a053e: if (eax >= 0 (signed)) goto B56  else fallthrough   ; jge 0x4a054a

  B54 @0x4a0540:

  B55 @0x4a0545:
    4a0545: WRITE [0x25f6890] mov= eax   ; dword ptr [0x25f6890], eax

  B56 @0x4a054a:
    4a054c: if (esi == 0) goto B58  else fallthrough   ; je 0x4a056a

  B57 @0x4a054e:
    4a0553: CALL 0xbdd260 timer_remaining  regs[eax=0x25f6890]  stack[]
    4a0558: WRITE [esp+0x20] mov= ret(timer_remaining)   ; dword ptr [esp + 0x20], eax
    4a0568: goto B59

  B58 @0x4a056a:

  B59 @0x4a056c:
    4a057f: X87 STORE f32 [esp+0x24] <- (1.0 - ((float)i32[esp+0x20] / i32[esp+0x18]))  (rounds to f32)
    4a0589: X87 STORE f32 [esp+0x24] <- (f32[0x2526c40] + ((f32[0x2526c30] - f32[0x2526c40]) * f32[esp+0x24]))  (rounds to f32)
    4a0591: X87 STORE f32 [esp] <- f32[esp+0x24]  (rounds to f32)
    4a0599: CALL 0xbe50f0 sub_be50f0  regs[]  stack[0x25f5c2c, ecx]
    4a059e: READ  eax <- i32[0x25f6894]
    4a05a5: if (i32[0x25f6894] < 0 (signed)) goto B64  else fallthrough   ; jl 0x4a05ca

  B60 @0x4a05a7:
    4a05a9: if (eax <= edi) goto B63  else fallthrough   ; jle 0x4a05ba

  B61 @0x4a05ab:
    4a05b2: if ((eax sub edi) < 0x35a4e900) goto B82  else fallthrough   ; jl 0x4a06ab

  B62 @0x4a05b8:
    4a05b8: goto B64

  B63 @0x4a05ba:
    4a05c4: if ((edx sub eax) >= 0x35a4e900) goto B82  else fallthrough   ; jge 0x4a06ab

  B64 @0x4a05ca:
    4a05ce: X87 COMPARE f32[esp+0x14]  vs  f64[0xe5c2c0] =f64 0.30000001192092896 (0x3fd3333340000000)   [fcom]
    4a05d9: if ((f32[esp+0x14] < f64[0xe5c2c0] =f64 0.30000001192092896 (0x3fd3333340000000)) OR (f32[esp+0x14] == f64[0xe5c2c0] =f64 0.30000001192092896 (0x3fd3333340000000)) OR UNORDERED(NaN)) goto B66  else fallthrough   ; jne 0x4a05e3

  B65 @0x4a05db:

  B66 @0x4a05e3:
    4a05e3: X87 STORE f32 [esp+0x14] <- f32[esp+0x14]  (rounds to f32)
    4a05ed: X87 STORE f32 [0x2526c44] <- f32[0x2526c34]  (rounds to f32)
    4a05f5: X87 COMPARE 0.0  vs  f32[0x2526c34]   [fcompp]
    4a05fc: if ((0.0 < f32[0x2526c34])) goto B68  else fallthrough   ; jnp 0x4a0600

  B67 @0x4a05fe:

  B68 @0x4a0600:
    4a0611: X87 STORE f32 [esp+0x1c] <- (-f32[esp+0x14] * f64[0xe5c120] =f64 20 (0x4034000000000000))  (rounds to f32)
    4a0615: CALL 0xbdce30 rng_float01  regs[eax=0xec30f8]  stack[]
    4a0629: X87 STORE f32 [esp+0x20] <- (f32[esp+0x14] * f64[0xe5c120] =f64 20 (0x4034000000000000))  (rounds to f32)
    4a063f: X87 STORE f32 [0x2526c34] <- ST0<ret rng_float01>  (rounds to f32)
    4a0645: if (i32[esp+0x13] == 0x0) goto B76  else fallthrough   ; je 0x4a0683

  B69 @0x4a0647:
    4a064f: X87 COMPARE f32[0x2526c34]  vs  0.0   [fcom]
    4a0656: if ((f32[0x2526c34] < 0.0) OR (f32[0x2526c34] == 0.0) OR UNORDERED(NaN)) goto B72  else fallthrough   ; jne 0x4a0668

  B70 @0x4a0658:
    4a065a: if (bl == 0) goto B72  else fallthrough   ; je 0x4a0668

  B71 @0x4a065c:
    4a0660: X87 STORE f32 [0x2526c34] <- -f32[0x2526c34]  (rounds to f32)
    4a0666: goto B76

  B72 @0x4a0668:
    4a0668: X87 COMPARE f32[0x2526c34]  vs  0.0   [fcom]
    4a0671: if ((f32[0x2526c34] > 0.0) OR (f32[0x2526c34] == 0.0) OR UNORDERED(NaN)) goto B75  else fallthrough   ; jp 0x4a0681

  B73 @0x4a0673:
    4a0675: if (bl != 0) goto B75  else fallthrough   ; jne 0x4a0681

  B74 @0x4a0677:
    4a0679: X87 STORE f32 [0x2526c34] <- -f32[0x2526c34]  (rounds to f32)
    4a067f: goto B76

  B75 @0x4a0681:

  B76 @0x4a0683:
    4a0688: WRITE [0x25f6894] mov= &[edi+esi]   ; dword ptr [0x25f6894], eax
    4a068d: if (esi < 0 (signed)) goto B79  else fallthrough   ; jl 0x4a069d

  B77 @0x4a068f:
    4a0694: if (eax <= 0x6b49d200) goto B82  else fallthrough   ; jle 0x4a06ab

  B78 @0x4a0696:
    4a069b: goto B81

  B79 @0x4a069d:
    4a069f: if (eax >= 0 (signed)) goto B82  else fallthrough   ; jge 0x4a06ab

  B80 @0x4a06a1:

  B81 @0x4a06a6:
    4a06a6: WRITE [0x25f6894] mov= eax   ; dword ptr [0x25f6894], eax

  B82 @0x4a06ab:
    4a06ad: if (esi == 0) goto B84  else fallthrough   ; je 0x4a06cb

  B83 @0x4a06af:
    4a06b4: CALL 0xbdd260 timer_remaining  regs[eax=0x25f6894]  stack[]
    4a06b9: WRITE [esp+0x20] mov= ret(timer_remaining)   ; dword ptr [esp + 0x20], eax
    4a06c9: goto B85

  B84 @0x4a06cb:

  B85 @0x4a06cd:
    4a06df: X87 STORE f32 [esp+0x20] <- (1.0 - ((float)i32[esp+0x20] / i32[esp+0x18]))  (rounds to f32)
    4a06e9: X87 STORE f32 [0x25f5c68] <- (f32[0x2526c44] + ((f32[0x2526c34] - f32[0x2526c44]) * f32[esp+0x20]))  (rounds to f32)

  B86 @0x4a06ef:
    4a06f5: RETURN

  call targets: 0x960120, 0x9a0270, 0xbdce30(rng_float01), 0xbdd260(timer_remaining), 0xbe50f0, 0xbf0f50
```


## Small functions: recipes

Fifteen small, self-contained functions. All take their arguments in registers or on the stack exactly as in the heading; a Rust replacement therefore needs a register shim (or a naked wrapper) like the existing `detour` ones. Each map below is generated from the disassembly.


### camera_mode_push — 0x493C80

**Convention.** ECX = mode, cdecl, returns nothing (tail-jumps to the enter fn).

1. `n = [0x25F5B0C]`; scan `i = n-1` down to 0 for `stack[i] == mode` (`stack = 0x25F5AEC`).
2. If found and `i < n-1` (the `jge` skips otherwise): shift `stack[k] = stack[k+1]` for `k = i .. n-2`; then, found or not at the top, **only when found** `n -= 1`. (`jl 0x493CD7` jumps over the decrement when the scan fell off the bottom.)
3. `stack[n] = mode`; `[0x25F5B0C] = n + 1`.
4. If `[0x25F5AE8] == mode` return. Otherwise clear flag 2 of `[0x25F5BE8]`, set `[0x25F5AE8] = mode`, and if `[0xDD5250 + mode*0x14] != 0` **jump** (not call) to it, so the enter fn returns to the caller.
Hazard: the stack array has no bound check in the original (assume < 16 entries).

```
FUNCTION 0x00493C80  size 0x8c  insns 36  blocks 12
  entry regs read before write: ['ecx']

  B0 @0x493c80:
    493c80: READ  edx <- i32[0x25f5b0c]
    493c8e: if (&[edx-0x1] < 0 (signed)) goto B8  else fallthrough   ; jl 0x493cd7

  B1 @0x493c90:
    493c97: if (i32[eax*4+0x25f5aec] == ecx) goto B4  else fallthrough   ; je 0x493ca0

  B2 @0x493c99:
    493c9c: if (jns (flags from earlier instr)) goto B1  else fallthrough   ; jns 0x493c90

  B3 @0x493c9e:
    493c9e: goto B8

  B4 @0x493ca0:
    493ca2: if (eax >= esi) goto B7  else fallthrough   ; jge 0x493cce

  B5 @0x493ca4:
    493ca4: goto B6

  B6 @0x493cb0:
    493cb7: WRITE [eax*4+0x25f5aec] mov= i32[eax*4+0x25f5af0]   ; dword ptr [eax*4 + 0x25f5aec], edx
    493cbe: READ  edx <- i32[0x25f5b0c]
    493ccc: if ((eax add 0x1) < &[edx-0x1]) goto B6  else fallthrough   ; jl 0x493cb0

  B7 @0x493cce:
    493cd1: WRITE [0x25f5b0c] mov= (edx sub 0x1)   ; dword ptr [0x25f5b0c], edx

  B8 @0x493cd7:
    493cd7: WRITE [edx*4+0x25f5aec] mov= ecx   ; dword ptr [edx*4 + 0x25f5aec], ecx
    493cde: WRITE [0x25f5b0c] add= 0x1   ; dword ptr [0x25f5b0c], 1
    493cec: if (i32[0x25f5ae8] == ecx) goto B11  else fallthrough   ; je 0x493d0b

  B9 @0x493cee:
    493cee: WRITE [0x25f5be8] and= 0xfffffffd   ; dword ptr [0x25f5be8], 0xfffffffd
    493d01: WRITE [0x25f5ae8] mov= ecx   ; dword ptr [0x25f5ae8], ecx
    493d07: if (i32[eax*4+0xdd5250] == 0) goto B11  else fallthrough   ; je 0x493d0b

  B10 @0x493d09:
    493d09: jmp indirect eax

  B11 @0x493d0b:
    493d0b: RETURN

  call targets:
```

Disassembly:
```
== 493c80
00493c80  mov edx, dword ptr [0x25f5b0c]
00493c86  push esi
00493c87  lea esi, [edx - 1]
00493c8a  mov eax, esi
00493c8c  test eax, eax
00493c8e  jl 0x493cd7
00493c90  cmp dword ptr [eax*4 + 0x25f5aec], ecx
00493c97  je 0x493ca0
00493c99  sub eax, 1
00493c9c  jns 0x493c90
00493c9e  jmp 0x493cd7
00493ca0  cmp eax, esi
00493ca2  jge 0x493cce
00493ca4  jmp 0x493cb0
00493cb0  mov edx, dword ptr [eax*4 + 0x25f5af0]
00493cb7  mov dword ptr [eax*4 + 0x25f5aec], edx
00493cbe  mov edx, dword ptr [0x25f5b0c]
00493cc4  add eax, 1
00493cc7  lea esi, [edx - 1]
00493cca  cmp eax, esi
00493ccc  jl 0x493cb0
00493cce  sub edx, 1
00493cd1  mov dword ptr [0x25f5b0c], edx
00493cd7  mov dword ptr [edx*4 + 0x25f5aec], ecx
00493cde  add dword ptr [0x25f5b0c], 1
00493ce5  cmp dword ptr [0x25f5ae8], ecx
00493ceb  pop esi
00493cec  je 0x493d0b
00493cee  and dword ptr [0x25f5be8], 0xfffffffd
00493cf5  lea eax, [ecx + ecx*4]
00493cf8  mov eax, dword ptr [eax*4 + 0xdd5250]
00493cff  test eax, eax
00493d01  mov dword ptr [0x25f5ae8], ecx
00493d07  je 0x493d0b
00493d09  jmp eax
00493d0b  ret 
; end 00493d0c size 8c
```

### camera_mode_pop — 0x493E30

**Convention.** EDX = mode, cdecl, tail-jumps.

1. `n = [0x25F5B0C]`; return at once if `n <= 0` (`jle`).
2. If `stack[n-1] == mode` (top) go to 4. Else scan `i = n-2` down to 0 for a match; not found -> go straight to 5 without removing anything; found with `i < n-1`: shift the elements above it down.
4. `n -= 1`; `[0x25F5B0C] = n`.
5. New mode = `[0x25F5B10]` (base) if `n == 0` else `stack[n-1]`.
6. Clear flag 2; `old = [0x25F5AE8]`; `[0x25F5AE8] = new`; if `new != old` and `[0xDD5250 + new*0x14] != 0` jump to it (tail).

```
FUNCTION 0x00493E30  size 0x9d  insns 44  blocks 17
  entry regs read before write: ['edx']

  B0 @0x493e30:
    493e30: READ  ecx <- i32[0x25f5b0c]
    493e38: if (i32[0x25f5b0c] <= 0 (signed)) goto B16  else fallthrough   ; jle 0x493ecc

  B1 @0x493e3e:
    493e46: READ  esi <- i32[0x25f5ae8]
    493e4c: if (i32[ecx*4+0x25f5ae8] == edx) goto B9  else fallthrough   ; je 0x493e8e

  B2 @0x493e4e:
    493e53: if (&[ecx-0x2] < 0 (signed)) goto B10  else fallthrough   ; jl 0x493e97

  B3 @0x493e55:
    493e5c: if (i32[eax*4+0x25f5aec] == edx) goto B6  else fallthrough   ; je 0x493e65

  B4 @0x493e5e:
    493e61: if (jns (flags from earlier instr)) goto B3  else fallthrough   ; jns 0x493e55

  B5 @0x493e63:
    493e63: goto B10

  B6 @0x493e65:
    493e6a: if (eax >= &[ecx-0x1]) goto B9  else fallthrough   ; jge 0x493e8e

  B7 @0x493e6c:

  B8 @0x493e70:
    493e77: WRITE [eax*4+0x25f5aec] mov= i32[eax*4+0x25f5af0]   ; dword ptr [eax*4 + 0x25f5aec], ecx
    493e7e: READ  ecx <- i32[0x25f5b0c]
    493e8c: if ((eax add 0x1) < &[ecx-0x1]) goto B8  else fallthrough   ; jl 0x493e70

  B9 @0x493e8e:
    493e91: WRITE [0x25f5b0c] mov= (ecx sub 0x1)   ; dword ptr [0x25f5b0c], ecx

  B10 @0x493e97:
    493e99: if (ecx != 0) goto B12  else fallthrough   ; jne 0x493ea3

  B11 @0x493e9b:
    493e9b: READ  ecx <- i32[0x25f5b10]
    493ea1: goto B13

  B12 @0x493ea3:

  B13 @0x493eaa:
    493eaa: WRITE [0x25f5be8] and= 0xfffffffd   ; dword ptr [0x25f5be8], 0xfffffffd
    493eb3: WRITE [0x25f5ae8] mov= ecx   ; dword ptr [0x25f5ae8], ecx
    493eba: if (ecx == esi) goto B16  else fallthrough   ; je 0x493ecc

  B14 @0x493ebc:
    493ec8: if (i32[eax*4+0xdd5250] == 0) goto B16  else fallthrough   ; je 0x493ecc

  B15 @0x493eca:
    493eca: jmp indirect eax

  B16 @0x493ecc:
    493ecc: RETURN

  call targets:
```

Disassembly:
```
== 493e30
00493e30  mov ecx, dword ptr [0x25f5b0c]
00493e36  test ecx, ecx
00493e38  jle 0x493ecc
00493e3e  cmp dword ptr [ecx*4 + 0x25f5ae8], edx
00493e45  push esi
00493e46  mov esi, dword ptr [0x25f5ae8]
00493e4c  je 0x493e8e
00493e4e  lea eax, [ecx - 2]
00493e51  test eax, eax
00493e53  jl 0x493e97
00493e55  cmp dword ptr [eax*4 + 0x25f5aec], edx
00493e5c  je 0x493e65
00493e5e  sub eax, 1
00493e61  jns 0x493e55
00493e63  jmp 0x493e97
00493e65  lea edx, [ecx - 1]
00493e68  cmp eax, edx
00493e6a  jge 0x493e8e
00493e6c  lea esp, [esp]
00493e70  mov ecx, dword ptr [eax*4 + 0x25f5af0]
00493e77  mov dword ptr [eax*4 + 0x25f5aec], ecx
00493e7e  mov ecx, dword ptr [0x25f5b0c]
00493e84  add eax, 1
00493e87  lea edx, [ecx - 1]
00493e8a  cmp eax, edx
00493e8c  jl 0x493e70
00493e8e  sub ecx, 1
00493e91  mov dword ptr [0x25f5b0c], ecx
00493e97  test ecx, ecx
00493e99  jne 0x493ea3
00493e9b  mov ecx, dword ptr [0x25f5b10]
00493ea1  jmp 0x493eaa
00493ea3  mov ecx, dword ptr [ecx*4 + 0x25f5ae8]
00493eaa  and dword ptr [0x25f5be8], 0xfffffffd
00493eb1  cmp ecx, esi
00493eb3  mov dword ptr [0x25f5ae8], ecx
00493eb9  pop esi
00493eba  je 0x493ecc
00493ebc  lea eax, [ecx + ecx*4]
00493ebf  mov eax, dword ptr [eax*4 + 0xdd5250]
00493ec6  test eax, eax
00493ec8  je 0x493ecc
00493eca  jmp eax
00493ecc  ret 
; end 00493ecd size 9d
```

### camera_mode_set_base — 0x493EE0

**Convention.** EAX = mode, stack (u8 smooth, f32 duration), cdecl.

`[0x25F5B10] = mode`. Only if the mode stack is empty (`[0x25F5B0C] == 0`) **and** `[0x25F5AE8] != mode`: clear flag 2, `[0x25F5AE8] = mode`, if `smooth != 0` call `camera_smooth_blend_begin(duration)` (and reload `eax = [0x25F5AE8]`), then if the mode's enter fn is non-null call it (a normal call here, unlike push/pop). Otherwise only the base is recorded.

```
FUNCTION 0x00493EE0  size 0x51  insns 23  blocks 7
  entry regs read before write: ['eax']

  B0 @0x493ee0:
    493ee8: WRITE [0x25f5b10] mov= eax   ; dword ptr [0x25f5b10], eax
    493eed: if (i32[0x25f5b0c] != 0x0) goto B6  else fallthrough   ; jne 0x493f2f

  B1 @0x493eef:
    493ef5: if (i32[0x25f5ae8] == eax) goto B6  else fallthrough   ; je 0x493f2f

  B2 @0x493ef7:
    493ef7: WRITE [0x25f5be8] and= 0xfffffffd   ; dword ptr [0x25f5be8], 0xfffffffd
    493f03: WRITE [0x25f5ae8] mov= eax   ; dword ptr [0x25f5ae8], eax
    493f08: if (i32[esp+0x8] == 0x0) goto B4  else fallthrough   ; je 0x493f1f

  B3 @0x493f0a:
    493f0f: X87 STORE f32 [esp] <- f32[esp+0xc]  (rounds to f32)
    493f12: CALL 0x4a33f0 sub_4a33f0  regs[]  stack[ecx]
    493f17: READ  eax <- i32[0x25f5ae8]

  B4 @0x493f1f:
    493f2b: if (i32[eax*4+0xdd5250] == 0) goto B6  else fallthrough   ; je 0x493f2f

  B5 @0x493f2d:
    493f2d: CALL indirect eax  ecx=? stack[]

  B6 @0x493f2f:
    493f30: RETURN

  call targets: 0x4a33f0
```

Disassembly:
```
== 493ee0
00493ee0  push ecx
00493ee1  cmp dword ptr [0x25f5b0c], 0
00493ee8  mov dword ptr [0x25f5b10], eax
00493eed  jne 0x493f2f
00493eef  cmp dword ptr [0x25f5ae8], eax
00493ef5  je 0x493f2f
00493ef7  and dword ptr [0x25f5be8], 0xfffffffd
00493efe  cmp byte ptr [esp + 8], 0
00493f03  mov dword ptr [0x25f5ae8], eax
00493f08  je 0x493f1f
00493f0a  fld dword ptr [esp + 0xc]
00493f0e  push ecx
00493f0f  fstp dword ptr [esp]
00493f12  call 0x4a33f0
00493f17  mov eax, dword ptr [0x25f5ae8]
00493f1c  add esp, 4
00493f1f  lea eax, [eax + eax*4]
00493f22  mov eax, dword ptr [eax*4 + 0xdd5250]
00493f29  test eax, eax
00493f2b  je 0x493f2f
00493f2d  call eax
00493f2f  pop ecx
00493f30  ret 
; end 00493f31 size 51
```

### camera_mode_set_base_default — 0x49EEC0

**Convention.** No args.

Same as `camera_mode_set_base` but the mode is `[0x2526C20]`, the blend is always started with duration **0.4f** (`[0xDD1C50]`) and there is no smooth flag. Writes `[0x25F5B10]` first, always.

```
FUNCTION 0x0049EEC0  size 0x51  insns 22  blocks 5
  entry regs read before write: []

  B0 @0x49eec0:
    49eec8: READ  eax <- i32[0x2526c20]
    49eecd: WRITE [0x25f5b10] mov= i32[0x2526c20]   ; dword ptr [0x25f5b10], eax
    49eed2: if (i32[0x25f5b0c] != 0x0) goto B4  else fallthrough   ; jne 0x49ef0f

  B1 @0x49eed4:
    49eeda: if (i32[0x25f5ae8] == eax) goto B4  else fallthrough   ; je 0x49ef0f

  B2 @0x49eedc:
    49eee2: WRITE [0x25f5be8] and= 0xfffffffd   ; dword ptr [0x25f5be8], 0xfffffffd
    49eeea: X87 STORE f32 [esp] <- f32[0xdd1c50] =f32 0.400000006 (0x3ecccccd)  (rounds to f32)
    49eeed: WRITE [0x25f5ae8] mov= eax   ; dword ptr [0x25f5ae8], eax
    49eef2: CALL 0x4a33f0 sub_4a33f0  regs[]  stack[ecx]
    49eef7: READ  eax <- i32[0x25f5ae8]
    49ef0b: if (i32[eax*4+0xdd5250] == 0) goto B4  else fallthrough   ; je 0x49ef0f

  B3 @0x49ef0d:
    49ef0d: CALL indirect eax  ecx=? stack[]

  B4 @0x49ef0f:
    49ef10: RETURN

  call targets: 0x4a33f0
```

Disassembly:
```
== 49eec0
0049eec0  push ecx
0049eec1  cmp dword ptr [0x25f5b0c], 0
0049eec8  mov eax, dword ptr [0x2526c20]
0049eecd  mov dword ptr [0x25f5b10], eax
0049eed2  jne 0x49ef0f
0049eed4  cmp dword ptr [0x25f5ae8], eax
0049eeda  je 0x49ef0f
0049eedc  fld dword ptr [0xdd1c50]  ; f32=0.4
0049eee2  and dword ptr [0x25f5be8], 0xfffffffd
0049eee9  push ecx
0049eeea  fstp dword ptr [esp]
0049eeed  mov dword ptr [0x25f5ae8], eax
0049eef2  call 0x4a33f0
0049eef7  mov eax, dword ptr [0x25f5ae8]
0049eefc  lea eax, [eax + eax*4]
0049eeff  mov eax, dword ptr [eax*4 + 0xdd5250]
0049ef06  add esp, 4
0049ef09  test eax, eax
0049ef0b  je 0x49ef0f
0049ef0d  call eax
0049ef0f  pop ecx
0049ef10  ret 
; end 0049ef11 size 51
```

### camera_smooth_blend_begin — 0x4A33F0

**Convention.** Stack (f32 duration), cdecl, 41 callers.

Writes the blend start record (see Structures): `0xC053E0(EAX = 0x25F6990, EDI = 0x25F5B38)` (orientation to quaternion, register call), pos = `[0x25F5B14..1C]` copied by dwords, fov = `(f32)[0x25F5BA4]`, elapsed = 0.0f, duration = arg, type = 7; `[0x25F5BE8] |= 2`; `[0xF53AE0] = 0`. Float stores use fld/fstp so NaN payloads are preserved as f32.

```
FUNCTION 0x004A33F0  size 0x6f  insns 23  blocks 1
  entry regs read before write: []

  B0 @0x4a33f0:
    4a33fc: CALL 0xc053e0 sub_c053e0  regs[eax=0x25f6990, edi=0x25f5b38]  stack[ecx]
    4a3407: READ  eax <- i32[0x25f5b14]
    4a340c: X87 STORE f32 [0x25f69ac] <- f32[0x25f5ba4]  (rounds to f32)
    4a3414: READ  ecx <- i32[0x25f5b18]
    4a341a: READ  edx <- i32[0x25f5b1c]
    4a3420: X87 STORE f32 [0x25f69b0] <- 0.0  (rounds to f32)
    4a342a: WRITE [0x25f5be8] or= 0x2   ; dword ptr [0x25f5be8], 2
    4a3431: X87 STORE f32 [0x25f69b4] <- f32[esp+0xc]  (rounds to f32)
    4a3437: WRITE [0x25f69a0] mov= i32[0x25f5b14]   ; dword ptr [0x25f69a0], eax
    4a343c: WRITE [0x25f69a4] mov= i32[0x25f5b18]   ; dword ptr [0x25f69a4], ecx
    4a3442: WRITE [0x25f69a8] mov= i32[0x25f5b1c]   ; dword ptr [0x25f69a8], edx
    4a3448: WRITE [0x25f69b8] mov= 0x7   ; dword ptr [0x25f69b8], 7
    4a3452: WRITE [0xf53ae0] mov= 0x0   ; dword ptr [0xf53ae0], 0
    4a345e: RETURN

  call targets: 0xc053e0
```

Disassembly:
```
== 4a33f0
004a33f0  push ecx
004a33f1  push edi
004a33f2  mov edi, 0x25f5b38
004a33f7  mov eax, 0x25f6990
004a33fc  call 0xc053e0
004a3401  fld dword ptr [0x25f5ba4]
004a3407  mov eax, dword ptr [0x25f5b14]
004a340c  fstp dword ptr [0x25f69ac]
004a3412  fldz 
004a3414  mov ecx, dword ptr [0x25f5b18]
004a341a  mov edx, dword ptr [0x25f5b1c]
004a3420  fstp dword ptr [0x25f69b0]
004a3426  fld dword ptr [esp + 0xc]
004a342a  or dword ptr [0x25f5be8], 2
004a3431  fstp dword ptr [0x25f69b4]
004a3437  mov dword ptr [0x25f69a0], eax
004a343c  mov dword ptr [0x25f69a4], ecx
004a3442  mov dword ptr [0x25f69a8], edx
004a3448  mov dword ptr [0x25f69b8], 7
004a3452  mov dword ptr [0xf53ae0], 0
004a345c  pop edi
004a345d  pop ecx
004a345e  ret 
; end 004a345f size 6f
```

### angle_wrap_in_place — 0x4A1A50

**Convention.** ECX = &f32, returns nothing (value in place).

Constants are doubles: `[0xE5C160]` (pi, upper bound), `[0xE5BF78]` (2*pi step), `[0xE5C170]` (-pi lower bound). Loop 1: `while v > pi { v = (f32)(v - 2*pi) }` (the entry test skips the loop when `pi > v`, `pi == v` or NaN; the back edge repeats only while `v > pi`, strict). Loop 2: `while v < -pi { v = (f32)(v + 2*pi) }` (skipped when `-pi < v`, equal or NaN; repeats while strictly `v < -pi`). The comparisons use the f32 value just stored, widened to the double constants, so the limits are the f64 values 3.1415927410125732 and 6.2831854820251465 (f32 pi and 2*pi widened).

```
FUNCTION 0x004A1A50  size 0x55  insns 35  blocks 5
  entry regs read before write: ['ecx']

  B0 @0x4a1a50:
    4a1a58: X87 COMPARE f64[0xe5c160] =f64 3.1415927410125732 (0x400921fb60000000)  vs  f32[ecx]   [fcom]
    4a1a67: if ((f64[0xe5c160] =f64 3.1415927410125732 (0x400921fb60000000) > f32[ecx]) OR (f64[0xe5c160] =f64 3.1415927410125732 (0x400921fb60000000) == f32[ecx]) OR UNORDERED(NaN)) goto B2  else fallthrough   ; jp 0x4a1a7a

  B1 @0x4a1a69:
    4a1a6d: X87 STORE f32 [ecx] <- (f32[ecx] - f64[0xe5bf78] =f64 6.2831854820251465 (0x401921fb60000000))  (rounds to f32)
    4a1a71: X87 COMPARE f32[ecx]  vs  f64[0xe5c160] =f64 3.1415927410125732 (0x400921fb60000000)   [fcomp]
    4a1a78: if ((f32[ecx] > f64[0xe5c160] =f64 3.1415927410125732 (0x400921fb60000000))) goto B1  else fallthrough   ; je 0x4a1a69

  B2 @0x4a1a7a:
    4a1a84: X87 COMPARE f64[0xe5c170] =f64 -3.1415927410125732 (0xc00921fb60000000)  vs  f32[ecx]   [fcom]
    4a1a8d: if ((f64[0xe5c170] =f64 -3.1415927410125732 (0xc00921fb60000000) < f32[ecx]) OR (f64[0xe5c170] =f64 -3.1415927410125732 (0xc00921fb60000000) == f32[ecx]) OR UNORDERED(NaN)) goto B4  else fallthrough   ; jne 0x4a1aa0

  B3 @0x4a1a8f:
    4a1a93: X87 STORE f32 [ecx] <- (f32[ecx] + f64[0xe5bf78] =f64 6.2831854820251465 (0x401921fb60000000))  (rounds to f32)
    4a1a97: X87 COMPARE f32[ecx]  vs  f64[0xe5c170] =f64 -3.1415927410125732 (0xc00921fb60000000)   [fcomp]
    4a1a9e: if ((f32[ecx] < f64[0xe5c170] =f64 -3.1415927410125732 (0xc00921fb60000000))) goto B3  else fallthrough   ; jnp 0x4a1a8f

  B4 @0x4a1aa0:
    4a1aa4: RETURN

  call targets:
```

Disassembly:
```
== 4a1a50
004a1a50  fld dword ptr [ecx]
004a1a52  fld qword ptr [0xe5c160]  ; f32=3.68935e+19
004a1a58  fcom st(1)
004a1a5a  fnstsw ax
004a1a5c  fstp st(1)
004a1a5e  test ah, 5
004a1a61  fld qword ptr [0xe5bf78]  ; f32=3.68935e+19
004a1a67  jp 0x4a1a7a
004a1a69  fld dword ptr [ecx]
004a1a6b  fsub st(1)
004a1a6d  fstp dword ptr [ecx]
004a1a6f  fld dword ptr [ecx]
004a1a71  fcomp st(2)
004a1a73  fnstsw ax
004a1a75  test ah, 0x41
004a1a78  je 0x4a1a69
004a1a7a  fstp st(1)
004a1a7c  fld dword ptr [ecx]
004a1a7e  fld qword ptr [0xe5c170]  ; f32=3.68935e+19
004a1a84  fcom st(1)
004a1a86  fnstsw ax
004a1a88  fstp st(1)
004a1a8a  test ah, 0x41
004a1a8d  jne 0x4a1aa0
004a1a8f  fld dword ptr [ecx]
004a1a91  fadd st(2)
004a1a93  fstp dword ptr [ecx]
004a1a95  fld dword ptr [ecx]
004a1a97  fcomp st(1)
004a1a99  fnstsw ax
004a1a9b  test ah, 5
004a1a9e  jnp 0x4a1a8f
004a1aa0  fstp st(1)
004a1aa2  fstp st(0)
004a1aa4  ret 
; end 004a1aa5 size 55
```

### camera_follow_commit_context — 0x49A670

**Convention.** No args.

`[0x25F5BE8] &= ~2`; blend `[0xE9A5C8] = 1.0f`; copy 11 dwords from `0x25F6318 + [0xE9A5BC]*0x2C` to `0xE9A638`; then `[0xE9A5CC] = [0xE9A5B8]`, `[0xE9A5D0] = [0xE9A5BC]`, `[0xE9A5D4] = [0xE9A5C0]`, `[0xE9A5D8] = [0xE9A5C4]` (dword copy), `[0xE9A5DC] = [0xE9A5C8]`; `byte [0x2526B5A] = 1`.

```
FUNCTION 0x0049A670  size 0x6e  insns 25  blocks 1
  entry regs read before write: []

  B0 @0x49a670:
    49a672: WRITE [0x25f5be8] and= 0xfffffffd   ; dword ptr [0x25f5be8], 0xfffffffd
    49a67a: X87 STORE f32 [0xe9a5c8] <- 1.0  (rounds to f32)
    49a680: READ  esi <- i32[0xe9a5bc] =i32 0
    49a69a: rep movsd dword ptr es:[edi], dword ptr [esi]   ; ecx=0xb esi=(esi add 0x25f6318) edi=0xe9a638
    49a69c: READ  eax <- i32[0xe9a5b8] =i32 0
    49a6a1: READ  ecx <- i32[0xe9a5bc] =i32 0
    49a6a7: READ  edx <- i32[0xe9a5c0] =i32 0
    49a6ad: WRITE [0xe9a5cc] mov= i32[0xe9a5b8] =i32 0   ; dword ptr [0xe9a5cc], eax
    49a6b2: READ  eax <- i32[0xe9a5c4] =i32 0
    49a6b7: WRITE [0xe9a5d0] mov= i32[0xe9a5bc] =i32 0   ; dword ptr [0xe9a5d0], ecx
    49a6bd: READ  ecx <- i32[0xe9a5c8] =i32 0
    49a6c4: WRITE [0xe9a5d4] mov= i32[0xe9a5c0] =i32 0   ; dword ptr [0xe9a5d4], edx
    49a6ca: WRITE [0xe9a5d8] mov= i32[0xe9a5c4] =i32 0   ; dword ptr [0xe9a5d8], eax
    49a6cf: WRITE [0xe9a5dc] mov= i32[0xe9a5c8] =i32 0   ; dword ptr [0xe9a5dc], ecx
    49a6d5: WRITE [0x2526b5a] mov= 0x1   ; byte ptr [0x2526b5a], 1
    49a6dd: RETURN

  call targets:
```

Disassembly:
```
== 49a670
0049a670  fld1 
0049a672  and dword ptr [0x25f5be8], 0xfffffffd
0049a679  push esi
0049a67a  fstp dword ptr [0xe9a5c8]  ; f32=0
0049a680  mov esi, dword ptr [0xe9a5bc]
0049a686  imul esi, esi, 0x2c
0049a689  push edi
0049a68a  add esi, 0x25f6318
0049a690  mov ecx, 0xb
0049a695  mov edi, 0xe9a638
0049a69a  rep movsd dword ptr es:[edi], dword ptr [esi]
0049a69c  mov eax, dword ptr [0xe9a5b8]
0049a6a1  mov ecx, dword ptr [0xe9a5bc]
0049a6a7  mov edx, dword ptr [0xe9a5c0]
0049a6ad  mov dword ptr [0xe9a5cc], eax
0049a6b2  mov eax, dword ptr [0xe9a5c4]
0049a6b7  mov dword ptr [0xe9a5d0], ecx
0049a6bd  mov ecx, dword ptr [0xe9a5c8]
0049a6c3  pop edi
0049a6c4  mov dword ptr [0xe9a5d4], edx
0049a6ca  mov dword ptr [0xe9a5d8], eax
0049a6cf  mov dword ptr [0xe9a5dc], ecx
0049a6d5  mov byte ptr [0x2526b5a], 1
0049a6dc  pop esi
0049a6dd  ret 
; end 0049a6de size 6e
```

### camera_follow_set_target_context — 0x49A6E0

**Convention.** EAX = handle, EDX = transition mode, stack (u32 ctx, f32 duration), `ret 8`.

`[0xE9A5B8] = [0x25F5BB0] = handle`. Resolve the object (`idx = (h & 0xFFF) << 4`, `obj = [0x2149C64 + idx]`; null or `obj+0x44 != h` -> fallback position `[0x252A6D0..D8]` and orientation `0xEC5AE4`); position -> `0x25F5BB4..BBC`, 9 orientation dwords from `obj+0x20` -> `0x25F5BC0` (`rep movsd`). `if ctx != 0x19 { [0xE9A5BC] = ctx }`. Then: if `EDX != 0` and `duration > 0.0` (the `fcom` + `test ah,0x41` + `jnp` skips when duration is **less or equal**; NaN counts as > 0): `[0xE9A5C4] = duration; [0xE9A5C0] = EDX; return` else call `camera_follow_commit_context`.

```
FUNCTION 0x0049A6E0  size 0xbb  insns 55  blocks 12
  entry regs read before write: ['eax']

  B0 @0x49a6e0:
    49a6e9: WRITE [0xe9a5b8] mov= eax   ; dword ptr [0xe9a5b8], eax
    49a6ee: WRITE [0x25f5bb0] mov= eax   ; dword ptr [0x25f5bb0], eax
    49a6f3: if (eax == 0) goto B4  else fallthrough   ; je 0x49a72c

  B1 @0x49a6f5:
    49a708: if (i32[ecx+0x2149c64] == 0) goto B4  else fallthrough   ; je 0x49a72c

  B2 @0x49a70a:
    49a70d: if (i32[ecx+0x44] != eax) goto B4  else fallthrough   ; jne 0x49a72c

  B3 @0x49a70f:
    49a712: WRITE [0x25f5bb4] mov= i32[ecx+0x14]   ; dword ptr [0x25f5bb4], eax
    49a71a: WRITE [0x25f5bb8] mov= i32[ecx+0x18]   ; dword ptr [0x25f5bb8], eax
    49a722: WRITE [0x25f5bbc] mov= i32[ecx+0x1c]   ; dword ptr [0x25f5bbc], eax
    49a72a: goto B5

  B4 @0x49a72c:
    49a72c: READ  ecx <- i32[0x252a6d0]
    49a732: READ  eax <- i32[0x252a6d4]
    49a737: WRITE [0x25f5bb4] mov= i32[0x252a6d0]   ; dword ptr [0x25f5bb4], ecx
    49a73d: READ  ecx <- i32[0x252a6d8]
    49a743: WRITE [0x25f5bb8] mov= i32[0x252a6d4]   ; dword ptr [0x25f5bb8], eax
    49a748: WRITE [0x25f5bbc] mov= i32[0x252a6d8]   ; dword ptr [0x25f5bbc], ecx

  B5 @0x49a753:
    49a760: rep movsd dword ptr es:[edi], dword ptr [esi]   ; ecx=0x9 esi=? edi=0x25f5bc0
    49a762: if (ebx == 0x19) goto B7  else fallthrough   ; je 0x49a76a

  B6 @0x49a764:
    49a764: WRITE [0xe9a5bc] mov= ebx   ; dword ptr [0xe9a5bc], ebx

  B7 @0x49a76a:
    49a76f: if (edx == 0) goto B11  else fallthrough   ; je 0x49a793

  B8 @0x49a771:
    49a777: X87 COMPARE f32[esp+0x8]  vs  0.0   [fcom]
    49a780: if ((f32[esp+0x8] < 0.0) OR (f32[esp+0x8] == 0.0)) goto B10  else fallthrough   ; jnp 0x49a791

  B9 @0x49a782:
    49a782: X87 STORE f32 [0xe9a5c4] <- f32[esp+0x8]  (rounds to f32)
    49a788: WRITE [0xe9a5c0] mov= edx   ; dword ptr [0xe9a5c0], edx
    49a78e: RETURN 8

  B10 @0x49a791:

  B11 @0x49a793:
    49a793: CALL 0x49a670 sub_49a670  regs[]  stack[]
    49a798: RETURN 8

  call targets: 0x49a670
```

Disassembly:
```
== 49a6e0
0049a6e0  test eax, eax
0049a6e2  push ebx
0049a6e3  mov ebx, dword ptr [esp + 8]
0049a6e7  push esi
0049a6e8  push edi
0049a6e9  mov dword ptr [0xe9a5b8], eax
0049a6ee  mov dword ptr [0x25f5bb0], eax
0049a6f3  je 0x49a72c
0049a6f5  mov ecx, eax
0049a6f7  and ecx, 0xfff
0049a6fd  shl ecx, 4
0049a700  mov ecx, dword ptr [ecx + 0x2149c64]
0049a706  test ecx, ecx
0049a708  je 0x49a72c
0049a70a  cmp dword ptr [ecx + 0x44], eax
0049a70d  jne 0x49a72c
0049a70f  mov eax, dword ptr [ecx + 0x14]
0049a712  mov dword ptr [0x25f5bb4], eax
0049a717  mov eax, dword ptr [ecx + 0x18]
0049a71a  mov dword ptr [0x25f5bb8], eax
0049a71f  mov eax, dword ptr [ecx + 0x1c]
0049a722  mov dword ptr [0x25f5bbc], eax
0049a727  lea esi, [ecx + 0x20]
0049a72a  jmp 0x49a753
0049a72c  mov ecx, dword ptr [0x252a6d0]
0049a732  mov eax, dword ptr [0x252a6d4]
0049a737  mov dword ptr [0x25f5bb4], ecx
0049a73d  mov ecx, dword ptr [0x252a6d8]
0049a743  mov dword ptr [0x25f5bb8], eax
0049a748  mov dword ptr [0x25f5bbc], ecx
0049a74e  mov esi, 0xec5ae4
0049a753  cmp ebx, 0x19
0049a756  mov ecx, 9
0049a75b  mov edi, 0x25f5bc0
0049a760  rep movsd dword ptr es:[edi], dword ptr [esi]
0049a762  je 0x49a76a
0049a764  mov dword ptr [0xe9a5bc], ebx
0049a76a  test edx, edx
0049a76c  pop edi
0049a76d  pop esi
0049a76e  pop ebx
0049a76f  je 0x49a793
0049a771  fldz 
0049a773  fld dword ptr [esp + 8]
0049a777  fcom st(1)
0049a779  fnstsw ax
0049a77b  fstp st(1)
0049a77d  test ah, 0x41
0049a780  jnp 0x49a791
0049a782  fstp dword ptr [0xe9a5c4]  ; f32=0
0049a788  mov dword ptr [0xe9a5c0], edx
0049a78e  ret 8
0049a791  fstp st(0)
0049a793  call 0x49a670
0049a798  ret 8
; end 0049a79b size bb
```

### camera_follow_request_equals_current — 0x499AF0

**Convention.** ECX = &{handle, ctx, mode, f32 dur, f32 blend}, returns AL.

Compare handle with `[0xE9A5B8]`, ctx with `[0xE9A5BC]`, mode with `[0xE9A5C0]` as integers; then the two floats with `fucompp` + `test ah,0x44` + `jp`: **`jp` means "not equal or unordered", so AL = 0 for any NaN**; all five equal -> AL = 1.

```
FUNCTION 0x00499AF0  size 0x48  insns 25  blocks 7
  entry regs read before write: ['ecx']

  B0 @0x499af0:
    499af0: READ  eax <- i32[0xe9a5b8] =i32 0
    499af7: if (i32[0xe9a5b8] =i32 0 == i32[ecx]) goto B2  else fallthrough   ; je 0x499afc

  B1 @0x499af9:
    499afb: RETURN

  B2 @0x499afc:
    499afc: READ  edx <- i32[0xe9a5bc] =i32 0
    499b05: if (i32[0xe9a5bc] =i32 0 != i32[ecx+0x4]) goto B1  else fallthrough   ; jne 0x499af9

  B3 @0x499b07:
    499b07: READ  eax <- i32[0xe9a5c0] =i32 0
    499b0f: if (i32[0xe9a5c0] =i32 0 != i32[ecx+0x8]) goto B1  else fallthrough   ; jne 0x499af9

  B4 @0x499b11:
    499b1a: X87 COMPARE f32[ecx+0xc]  vs  f32[0xe9a5c4] =f32 0 (0x00000000)   [fucompp]
    499b21: if ((f32[ecx+0xc] > f32[0xe9a5c4] =f32 0 (0x00000000)) OR (f32[ecx+0xc] < f32[0xe9a5c4] =f32 0 (0x00000000)) OR UNORDERED(NaN)) goto B1  else fallthrough   ; jp 0x499af9

  B5 @0x499b23:
    499b2c: X87 COMPARE f32[ecx+0x10]  vs  f32[0xe9a5c8] =f32 0 (0x00000000)   [fucompp]
    499b33: if ((f32[ecx+0x10] > f32[0xe9a5c8] =f32 0 (0x00000000)) OR (f32[ecx+0x10] < f32[0xe9a5c8] =f32 0 (0x00000000)) OR UNORDERED(NaN)) goto B1  else fallthrough   ; jp 0x499af9

  B6 @0x499b35:
    499b37: RETURN

  call targets:
```

Disassembly:
```
== 499af0
00499af0  mov eax, dword ptr [0xe9a5b8]
00499af5  cmp eax, dword ptr [ecx]
00499af7  je 0x499afc
00499af9  xor al, al
00499afb  ret 
00499afc  mov edx, dword ptr [0xe9a5bc]
00499b02  cmp edx, dword ptr [ecx + 4]
00499b05  jne 0x499af9
00499b07  mov eax, dword ptr [0xe9a5c0]
00499b0c  cmp eax, dword ptr [ecx + 8]
00499b0f  jne 0x499af9
00499b11  fld dword ptr [0xe9a5c4]  ; f32=0
00499b17  fld dword ptr [ecx + 0xc]
00499b1a  fucompp 
00499b1c  fnstsw ax
00499b1e  test ah, 0x44
00499b21  jp 0x499af9
00499b23  fld dword ptr [0xe9a5c8]  ; f32=0
00499b29  fld dword ptr [ecx + 0x10]
00499b2c  fucompp 
00499b2e  fnstsw ax
00499b30  test ah, 0x44
00499b33  jp 0x499af9
00499b35  mov al, 1
00499b37  ret 
; end 00499b38 size 48
```

### camera_context_for_handle — 0x49AAB0

**Convention.** ECX = handle, returns EAX = context.

`ecx == 0` -> 0x19. `obj = [0x2149C64 + (h & 0xFFF)*16]`; null -> 0x19. If `obj+0x44 == h`: type `[obj+0x4C] == 1` (human) -> tail call-ish `camera_human_context_classify(ESI = obj)`; type 5 (vehicle) -> vehicle seat logic at 0x49AC10 (reads `[0x21703D4]` player, `obj+0x84E4` seat table with the `[0x33D630C]` sentinel; returns 5/6/7/8 or `[0xE99854]`). Stale handle (generation mismatch) -> 0xB; else 0x19.

```
FUNCTION 0x0049AAB0  size 0x1b7  insns 58  blocks 24
  entry regs read before write: ['ecx']

  B0 @0x49aab0:
    49aab2: if (ecx == 0) goto B11  else fallthrough   ; je 0x49ab05

  B1 @0x49aab4:
    49aac6: if (i32[eax+0x2149c64] == 0) goto B11  else fallthrough   ; je 0x49ab05

  B2 @0x49aac8:
    49aacb: if (i32[eax+0x44] != ecx) goto B5  else fallthrough   ; jne 0x49aadd

  B3 @0x49aacd:
    49aad1: if (i32[eax+0x4c] != 0x1) goto B5  else fallthrough   ; jne 0x49aadd

  B4 @0x49aad3:
    49aad6: CALL 0x49ab10 sub_49ab10  regs[esi=eax]  stack[]
    49aadc: RETURN

  B5 @0x49aadd:
    49aadf: if (eax == 0) goto B11  else fallthrough   ; je 0x49ab05

  B6 @0x49aae1:
    49aae4: if (i32[eax+0x44] != ecx) goto B9  else fallthrough   ; jne 0x49aaf7

  B7 @0x49aae6:
    49aaea: if (i32[eax+0x4c] != 0x5) goto B9  else fallthrough   ; jne 0x49aaf7

  B8 @0x49aaec:
    49aaec: READ  edx <- i32[0x21703d4]
    49aaf2: goto B13

  B9 @0x49aaf7:
    49aaf9: if (eax == 0) goto B11  else fallthrough   ; je 0x49ab05

  B10 @0x49aafb:
    49ab03: if (i32[eax+0x44] == ecx) goto B12  else fallthrough   ; je 0x49ab0a

  B11 @0x49ab05:

  B12 @0x49ab0a:
    49ab0a: RETURN

  B13 @0x49ac10:
    49ac12: if (eax != 0) goto B19  else fallthrough   ; jne 0x49ac2e

  B14 @0x49ac14:

  B15 @0x49ac17:
    49ac1c: if (je (flags from earlier instr)) goto B23  else fallthrough   ; je 0x49ac61

  B16 @0x49ac1e:
    49ac21: if (je (flags from earlier instr)) goto B22  else fallthrough   ; je 0x49ac5b

  B17 @0x49ac23:
    49ac26: if (je (flags from earlier instr)) goto B21  else fallthrough   ; je 0x49ac55

  B18 @0x49ac28:
    49ac28: READ  eax <- i32[0xe99854] =i32 3
    49ac2d: RETURN

  B19 @0x49ac2e:
    49ac4d: if (i32[eax+edx*4+0x6f4] == i32[0x33d630c]) goto B15  else fallthrough   ; je 0x49ac17

  B20 @0x49ac4f:
    49ac54: RETURN

  B21 @0x49ac55:
    49ac5a: RETURN

  B22 @0x49ac5b:
    49ac60: RETURN

  B23 @0x49ac61:
    49ac66: RETURN

  call targets: 0x49ab10
```

Disassembly:
```
== 49aab0
0049aab0  test ecx, ecx
0049aab2  je 0x49ab05
0049aab4  mov eax, ecx
0049aab6  and eax, 0xfff
0049aabb  shl eax, 4
0049aabe  mov eax, dword ptr [eax + 0x2149c64]
0049aac4  test eax, eax
0049aac6  je 0x49ab05
0049aac8  cmp dword ptr [eax + 0x44], ecx
0049aacb  jne 0x49aadd
0049aacd  cmp dword ptr [eax + 0x4c], 1
0049aad1  jne 0x49aadd
0049aad3  push esi
0049aad4  mov esi, eax
0049aad6  call 0x49ab10
0049aadb  pop esi
0049aadc  ret 
0049aadd  test eax, eax
0049aadf  je 0x49ab05
0049aae1  cmp dword ptr [eax + 0x44], ecx
0049aae4  jne 0x49aaf7
0049aae6  cmp dword ptr [eax + 0x4c], 5
0049aaea  jne 0x49aaf7
0049aaec  mov edx, dword ptr [0x21703d4]
0049aaf2  jmp 0x49ac10
0049aaf7  test eax, eax
0049aaf9  je 0x49ab05
0049aafb  cmp dword ptr [eax + 0x44], ecx
0049aafe  mov eax, 0xb
0049ab03  je 0x49ab0a
0049ab05  mov eax, 0x19
0049ab0a  ret 
0049ac10  test eax, eax
0049ac12  jne 0x49ac2e
0049ac14  or ecx, 0xffffffff
0049ac17  mov eax, ecx
0049ac19  sub eax, 2
0049ac1c  je 0x49ac61
0049ac1e  sub eax, 1
0049ac21  je 0x49ac5b
0049ac23  sub eax, 1
0049ac26  je 0x49ac55
0049ac28  mov eax, dword ptr [0xe99854]
0049ac2d  ret 
0049ac2e  mov eax, dword ptr [eax + 0x84e4]
0049ac34  mov edx, dword ptr [edx + 0xd78]
0049ac3a  mov ecx, dword ptr [eax + 0x9c]
0049ac40  mov eax, dword ptr [eax + edx*4 + 0x6f4]
0049ac47  cmp eax, dword ptr [0x33d630c]
0049ac4d  je 0x49ac17
0049ac4f  mov eax, 7
0049ac54  ret 
0049ac55  mov eax, 5
0049ac5a  ret 
0049ac5b  mov eax, 6
0049ac60  ret 
0049ac61  mov eax, 8
0049ac66  ret 
; end 0049ac67 size 1b7
```

### camera_human_context_classify — 0x49AB10

**Convention.** ESI = human (0 allowed), returns EAX = 0..0x17.

Test order is the contract (first hit wins): `human != 0 && [+0xE88] != 0` -> 0xA; state byte `+0x595` not in {0, 3} -> 0xD; `+0x591 == 5` -> 0xC; `camera_ctx_pred_falling` -> 0xE; `camera_ctx_pred_zoom` -> 9; `[+0x5E0]->[+0x164] == 0x441` -> 0xF; `pred_fine_aim_crouch` -> 0x11; `pred_fine_aim` -> 0x10; `pred_melee_lock` -> 0x12; `pred_human_shield` -> 0x16; `pred_sprint` -> 0x17; `pred_interior_sprint` -> 2; `pred_freefall` -> 0x14; `[+0xC0] bit 30` -> 0x15; else `jmp 0x49EDB0`. The predicates take EAX / ECX / EDX = human as listed in the CSV notes.

```
FUNCTION 0x0049AB10  size 0xfa  insns 80  blocks 31
  entry regs read before write: ['esi']

  B0 @0x49ab10:
    49ab12: if (esi == 0) goto B3  else fallthrough   ; je 0x49ab23

  B1 @0x49ab14:
    49ab1b: if (i32[esi+0xe88] == 0x0) goto B3  else fallthrough   ; je 0x49ab23

  B2 @0x49ab1d:
    49ab22: RETURN

  B3 @0x49ab23:
    49ab2b: if (i32[esi+0x595] == 0) goto B6  else fallthrough   ; je 0x49ab37

  B4 @0x49ab2d:
    49ab2f: if (al == 0x3) goto B6  else fallthrough   ; je 0x49ab37

  B5 @0x49ab31:
    49ab36: RETURN

  B6 @0x49ab37:
    49ab3e: if (i32[esi+0x591] != 0x5) goto B8  else fallthrough   ; jne 0x49ab46

  B7 @0x49ab40:
    49ab45: RETURN

  B8 @0x49ab46:
    49ab48: CALL 0x49a8c0 sub_49a8c0  regs[edx=esi]  stack[]
    49ab4f: if (al == 0) goto B10  else fallthrough   ; je 0x49ab57

  B9 @0x49ab51:
    49ab56: RETURN

  B10 @0x49ab57:
    49ab59: CALL 0x49a870 sub_49a870  regs[ecx=esi]  stack[]
    49ab60: if (al == 0) goto B12  else fallthrough   ; je 0x49ab68

  B11 @0x49ab62:
    49ab67: RETURN

  B12 @0x49ab68:
    49ab78: if (i32[eax+0x164] != 0x441) goto B14  else fallthrough   ; jne 0x49ab80

  B13 @0x49ab7a:
    49ab7f: RETURN

  B14 @0x49ab80:
    49ab80: CALL 0x49a950 sub_49a950  regs[ecx=?]  stack[]
    49ab87: if (al == 0) goto B16  else fallthrough   ; je 0x49ab8f

  B15 @0x49ab89:
    49ab8e: RETURN

  B16 @0x49ab8f:
    49ab91: CALL 0x49a920 sub_49a920  regs[ecx=esi]  stack[]
    49ab98: if (al == 0) goto B18  else fallthrough   ; je 0x49aba0

  B17 @0x49ab9a:
    49ab9f: RETURN

  B18 @0x49aba0:
    49aba0: CALL 0x49a980 sub_49a980  regs[ecx=?]  stack[]
    49aba7: if (al == 0) goto B20  else fallthrough   ; je 0x49abaf

  B19 @0x49aba9:
    49abae: RETURN

  B20 @0x49abaf:
    49abaf: CALL 0x49a9b0 sub_49a9b0  regs[edx=?]  stack[]
    49abb6: if (al == 0) goto B22  else fallthrough   ; je 0x49abbe

  B21 @0x49abb8:
    49abbd: RETURN

  B22 @0x49abbe:
    49abc0: CALL 0x49aa00 sub_49aa00  regs[eax=esi]  stack[]
    49abc7: if (al == 0) goto B24  else fallthrough   ; je 0x49abcf

  B23 @0x49abc9:
    49abce: RETURN

  B24 @0x49abcf:
    49abd1: CALL 0x49aa40 sub_49aa40  regs[eax=esi]  stack[]
    49abd8: if (al == 0) goto B26  else fallthrough   ; je 0x49abe0

  B25 @0x49abda:
    49abdf: RETURN

  B26 @0x49abe0:
    49abe2: CALL 0x49aa80 sub_49aa80  regs[eax=esi]  stack[]
    49abe9: if (al == 0) goto B28  else fallthrough   ; je 0x49abf1

  B27 @0x49abeb:
    49abf0: RETURN

  B28 @0x49abf1:
    49abfd: if ((cl & 0x1) == 0) goto B30  else fallthrough   ; je 0x49ac05

  B29 @0x49abff:
    49ac04: RETURN

  B30 @0x49ac05:
    49ac05: goto 0x49edb0

  call targets: 0x49a870, 0x49a8c0, 0x49a920, 0x49a950, 0x49a980, 0x49a9b0, 0x49aa00, 0x49aa40, 0x49aa80
```

Disassembly:
```
== 49ab10
0049ab10  test esi, esi
0049ab12  je 0x49ab23
0049ab14  cmp dword ptr [esi + 0xe88], 0
0049ab1b  je 0x49ab23
0049ab1d  mov eax, 0xa
0049ab22  ret 
0049ab23  mov al, byte ptr [esi + 0x595]
0049ab29  test al, al
0049ab2b  je 0x49ab37
0049ab2d  cmp al, 3
0049ab2f  je 0x49ab37
0049ab31  mov eax, 0xd
0049ab36  ret 
0049ab37  cmp byte ptr [esi + 0x591], 5
0049ab3e  jne 0x49ab46
0049ab40  mov eax, 0xc
0049ab45  ret 
0049ab46  mov edx, esi
0049ab48  call 0x49a8c0
0049ab4d  test al, al
0049ab4f  je 0x49ab57
0049ab51  mov eax, 0xe
0049ab56  ret 
0049ab57  mov ecx, esi
0049ab59  call 0x49a870
0049ab5e  test al, al
0049ab60  je 0x49ab68
0049ab62  mov eax, 9
0049ab67  ret 
0049ab68  mov eax, dword ptr [esi + 0x5e0]
0049ab6e  cmp dword ptr [eax + 0x164], 0x441
0049ab78  jne 0x49ab80
0049ab7a  mov eax, 0xf
0049ab7f  ret 
0049ab80  call 0x49a950
0049ab85  test al, al
0049ab87  je 0x49ab8f
0049ab89  mov eax, 0x11
0049ab8e  ret 
0049ab8f  mov ecx, esi
0049ab91  call 0x49a920
0049ab96  test al, al
0049ab98  je 0x49aba0
0049ab9a  mov eax, 0x10
0049ab9f  ret 
0049aba0  call 0x49a980
0049aba5  test al, al
0049aba7  je 0x49abaf
0049aba9  mov eax, 0x12
0049abae  ret 
0049abaf  call 0x49a9b0
0049abb4  test al, al
0049abb6  je 0x49abbe
0049abb8  mov eax, 0x16
0049abbd  ret 
0049abbe  mov eax, esi
0049abc0  call 0x49aa00
0049abc5  test al, al
0049abc7  je 0x49abcf
0049abc9  mov eax, 0x17
0049abce  ret 
0049abcf  mov eax, esi
0049abd1  call 0x49aa40
0049abd6  test al, al
0049abd8  je 0x49abe0
0049abda  mov eax, 2
0049abdf  ret 
0049abe0  mov eax, esi
0049abe2  call 0x49aa80
0049abe7  test al, al
0049abe9  je 0x49abf1
0049abeb  mov eax, 0x14
0049abf0  ret 
0049abf1  mov ecx, dword ptr [esi + 0xc0]
0049abf7  shr ecx, 0x1e
0049abfa  test cl, 1
0049abfd  je 0x49ac05
0049abff  mov eax, 0x15
0049ac04  ret 
0049ac05  jmp 0x49edb0
; end 0049ac0a size fa
```

### camera_shake_start_preset — 0x4A0B60

**Convention.** EAX = preset index, stack (f32 scale), cdecl.

`rec = 0xF539E8 + preset*0x18`. Two record floats are multiplied by `scale` and stored as f32 (`[0xF539EC + preset*24] * scale` and `scale * rec[+0]`); the other dwords (`rec[+8]`, `+0xC`, `+0x14`, `+0x10`) are passed through unchanged to `camera_shake_start(ECX = rec[+0x10], stack ...)`. The exact push order is in the disassembly block below; the generated `stack[...]` list is unreliable here because the floats are written with `fstp [esp]` between the pushes.

```
FUNCTION 0x004A0B60  size 0x55  insns 26  blocks 1
  entry regs read before write: ['eax']

  B0 @0x4a0b60:
    4a0b8e: X87 STORE f32 [esp+0xc] <- (f32[eax*8+0xf539ec] * f32[esp+0x4])  (rounds to f32)
    4a0b96: X87 STORE f32 [esp] <- f32[esp+0xc]  (rounds to f32)
    4a0ba1: X87 STORE f32 [esp+0x18] <- (f32[esp+0x4] * f32[eax])  (rounds to f32)
    4a0ba9: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    4a0bac: CALL 0x4a0a10 sub_4a0a10  regs[ecx=i32[eax+0x10]]  stack[i32[eax+0x8], i32[eax+0x8], i32[eax*8+0xf539f4], i32[eax*8+0xf539fc], i32[eax*8+0xf539fc]]
    4a0bb4: RETURN

  call targets: 0x4a0a10
```

Disassembly:
```
== 4a0b60
004a0b60  lea eax, [eax + eax*2]
004a0b63  fld dword ptr [eax*8 + 0xf539ec]
004a0b6a  mov ecx, dword ptr [eax*8 + 0xf539fc]
004a0b71  fld dword ptr [esp + 4]
004a0b75  mov edx, dword ptr [eax*8 + 0xf539f4]
004a0b7c  lea eax, [eax*8 + 0xf539e8]
004a0b83  fld st(0)
004a0b85  fmulp st(2)
004a0b87  push ecx
004a0b88  push ecx
004a0b89  fxch st(1)
004a0b8b  mov ecx, dword ptr [eax + 8]
004a0b8e  fstp dword ptr [esp + 0xc]
004a0b92  fld dword ptr [esp + 0xc]
004a0b96  fstp dword ptr [esp]
004a0b99  push edx
004a0b9a  push ecx
004a0b9b  fmul dword ptr [eax]
004a0b9d  push ecx
004a0b9e  mov ecx, dword ptr [eax + 0x10]
004a0ba1  fstp dword ptr [esp + 0x18]
004a0ba5  fld dword ptr [esp + 0x18]
004a0ba9  fstp dword ptr [esp]
004a0bac  call 0x4a0a10
004a0bb1  add esp, 0x14
004a0bb4  ret 
; end 004a0bb5 size 55
```

### camera_shake_from_world_point — 0x4A0BC0

**Convention.** EAX = &vec3 (world point), stack (f32 scale), cdecl.

`d = |p - player.pos|` where the player is `[0x21703D4]` and the position is `+0x14..+0x1C` (sum of squares as f32, `sqrtf_wrapper`). **Only when `30.0 > d` (double `[0xE5BF68]`; the `test ah,0x41` / `jne` skips when `30.0 <= d` or NaN)**: `x = 1 - d/30.0`; `ms = 0xC8 - ftol(-300.0 * x)`; amplitude `scale * 0.02 / ...` per the map; calls `camera_shake_start` (0x4A0A10) with ECX = `ms`, fade-in constant 0x1F4 and 0.075f (`[0xE99384]`). Returns nothing.

```
FUNCTION 0x004A0BC0  size 0xdb  insns 72  blocks 3
  entry regs read before write: ['eax']

  B0 @0x4a0bc0:
    4a0bc3: READ  ecx <- i32[0x21703d4]
    4a0bcf: X87 STORE f32 [esp+0xc] <- (f32[eax] - f32[ecx+0x14])  (rounds to f32)
    4a0bd9: X87 STORE f32 [esp+0x10] <- (f32[eax+0x4] - f32[ecx+0x18])  (rounds to f32)
    4a0be3: X87 STORE f32 [esp+0x14] <- (f32[eax+0x8] - f32[ecx+0x1c])  (rounds to f32)
    4a0c03: X87 STORE f32 [esp+0x8] <- (((f32[esp+0xc] * f32[esp+0xc]) + (f32[esp+0x10] * f32[esp+0x10])) + (f32[esp+0x14] * f32[esp+0x14]))  (rounds to f32)
    4a0c0b: X87 STORE f32 [esp] <- f32[esp+0x8]  (rounds to f32)
    4a0c0e: CALL 0x40ea20 sqrtf_wrapper  regs[]  stack[i32[0x21703d4]]
    4a0c13: X87 STORE f32 [esp+0x8] <- ST0<ret sqrtf_wrapper>  (rounds to f32)
    4a0c24: X87 COMPARE f64[0xe5bf68] =f64 30 (0x403e000000000000)  vs  f32[esp+0x4]   [fcom]
    4a0c2b: if ((f64[0xe5bf68] =f64 30 (0x403e000000000000) < f32[esp+0x4]) OR (f64[0xe5bf68] =f64 30 (0x403e000000000000) == f32[esp+0x4]) OR UNORDERED(NaN)) goto B2  else fallthrough   ; jne 0x4a0c93

  B1 @0x4a0c2d:
    4a0c36: X87 STORE f32 [esp+0xc] <- (1.0 - (f32[esp+0x4] / f64[0xe5bf68] =f64 30 (0x403e000000000000)))  (rounds to f32)
    4a0c40: X87 STORE f32 [esp] <- f32[0xe99384] =f32 0.075000003 (0x3d99999a)  (rounds to f32)
    4a0c54: CALL 0xda0140 _ftol2_sse  regs[]  stack[0x1f4, ecx, 0x0]
    4a0c7c: X87 STORE f32 [esp+0x18] <- ((_ftol2_sse((f64[0xe5cca0] =f64 -300 (0xc072c00000000000) * f32[esp+0x10])) * (((f32[esp+0x24] * f64[0xe5c300] =f64 0.019999999552965164 (0x3f947ae140000000)) / f32[esp+0x2c]) - 0.0)) + 0.0)  (rounds to f32)
    4a0c84: X87 STORE f32 [esp] <- f32[esp+0x18]  (rounds to f32)
    4a0c87: CALL 0x4a0a10 sub_4a0a10  regs[ecx=0]  stack[(ecx sub eax), (ecx sub eax)]
    4a0c92: RETURN

  B2 @0x4a0c93:
    4a0c9a: RETURN

  call targets: 0x40ea20(sqrtf_wrapper), 0x4a0a10, 0xda0140(_ftol2_sse)
```

Disassembly:
```
== 4a0bc0
004a0bc0  sub esp, 0x14
004a0bc3  mov ecx, dword ptr [0x21703d4]
004a0bc9  fld dword ptr [eax]
004a0bcb  fsub dword ptr [ecx + 0x14]
004a0bce  push ecx
004a0bcf  fstp dword ptr [esp + 0xc]
004a0bd3  fld dword ptr [eax + 4]
004a0bd6  fsub dword ptr [ecx + 0x18]
004a0bd9  fstp dword ptr [esp + 0x10]
004a0bdd  fld dword ptr [eax + 8]
004a0be0  fsub dword ptr [ecx + 0x1c]
004a0be3  fstp dword ptr [esp + 0x14]
004a0be7  fld dword ptr [esp + 0xc]
004a0beb  fld dword ptr [esp + 0x10]
004a0bef  fld dword ptr [esp + 0x14]
004a0bf3  fld st(1)
004a0bf5  fmulp st(2)
004a0bf7  fld st(2)
004a0bf9  fmulp st(3)
004a0bfb  fxch st(1)
004a0bfd  faddp st(2)
004a0bff  fmul st(0), st(0)
004a0c01  faddp st(1)
004a0c03  fstp dword ptr [esp + 8]
004a0c07  fld dword ptr [esp + 8]
004a0c0b  fstp dword ptr [esp]
004a0c0e  call 0x40ea20  ; sqrtf_wrapper
004a0c13  fstp dword ptr [esp + 8]
004a0c17  add esp, 4
004a0c1a  fld dword ptr [esp + 4]
004a0c1e  fld qword ptr [0xe5bf68]  ; f32=0
004a0c24  fcom st(1)
004a0c26  fnstsw ax
004a0c28  test ah, 0x41
004a0c2b  jne 0x4a0c93
004a0c2d  fdivp st(1)
004a0c2f  push 0
004a0c31  push ecx
004a0c32  fld1 
004a0c34  fsubrp st(1)
004a0c36  fstp dword ptr [esp + 0xc]
004a0c3a  fld dword ptr [0xe99384]  ; f32=0.075
004a0c40  fstp dword ptr [esp]
004a0c43  push 0x1f4
004a0c48  fld dword ptr [esp + 0x10]
004a0c4c  fld qword ptr [0xe5cca0]  ; f32=0
004a0c52  fmul st(1)
004a0c54  call 0xda0140  ; _ftol2_sse
004a0c59  fld dword ptr [esp + 0x24]
004a0c5d  fmul qword ptr [0xe5c300]  ; f32=2
004a0c63  mov ecx, 0xc8
004a0c68  sub ecx, eax
004a0c6a  push ecx
004a0c6b  fdiv dword ptr [esp + 0x2c]
004a0c6f  push ecx
004a0c70  xor ecx, ecx
004a0c72  fldz 
004a0c74  fsub st(1), st(0)
004a0c76  fxch st(1)
004a0c78  fmulp st(2)
004a0c7a  faddp st(1)
004a0c7c  fstp dword ptr [esp + 0x18]
004a0c80  fld dword ptr [esp + 0x18]
004a0c84  fstp dword ptr [esp]
004a0c87  call 0x4a0a10
004a0c8c  add esp, 0x14
004a0c8f  add esp, 0x14
004a0c92  ret 
004a0c93  fstp st(1)
004a0c95  fstp st(0)
004a0c97  add esp, 0x14
004a0c9a  ret 
; end 004a0c9b size db
```

### camera_params_set_polar — 0x499940

**Convention.** ESI = &params record, stack (f32 a, f32 b), `ret 8`.

`d = (f32)(a - 0.0)` (the double `[0xE5BFA0]` is 0.0; the result is stored to a stack slot). If `d < 0.001f` **and** `-d < 0.001f` (both ordered; `test ah,5 / jp` jumps to the long path when the compare is `>=` or unordered) then `[ESI+0x18] = [ESI+0x1C] = 0.0` and return. Otherwise (|d| >= 0.001 or NaN) `[ESI+0x18] = (f32)F(b / a)` where `F` = `0xDA0DC0` works on ST0 (an inverse trig function by its use) and `[ESI+0x1C] = sqrtf(a*a + b*b)` (`a*a + b*b` accumulated in x87 and stored once as f32 before the `sqrtf_wrapper` call).

```
FUNCTION 0x00499940  size 0x90  insns 51  blocks 5
  entry regs read before write: ['esi']

  B0 @0x499940:
    499954: X87 STORE f32 [esp+0x4] <- (f32[ebp+0x8] - f64[0xe5bfa0] =f64 0 (0x0000000000000000))  (rounds to f32)
    499962: X87 COMPARE f32[esp+0x4]  vs  f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)   [fcom]
    499969: if ((f32[esp+0x4] > f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)) OR (f32[esp+0x4] == f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)) OR UNORDERED(NaN)) goto B3  else fallthrough   ; jp 0x499986

  B1 @0x49996b:
    49996d: X87 COMPARE -f32[esp+0x4]  vs  f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)   [fcompp]
    499974: if ((-f32[esp+0x4] > f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)) OR (-f32[esp+0x4] == f32[0xdd5984] =f32 0.00100000005 (0x3a83126f)) OR UNORDERED(NaN)) goto B4  else fallthrough   ; jp 0x49998a

  B2 @0x499976:
    49997a: X87 STORE f32 [esi+0x18] <- 0.0  (rounds to f32)
    49997d: X87 STORE f32 [esi+0x1c] <- 0.0  (rounds to f32)
    499983: RETURN 8

  B3 @0x499986:

  B4 @0x49998a:
    49998d: X87 STORE f32 [esp+0x4] <- (f32[ebp+0xc] / f32[ebp+0x8])  (rounds to f32)
    499995: CALL 0xda0dc0 sub_da0dc0  regs[]  stack[]
    49999a: X87 STORE f32 [esp+0x4] <- sub_da0dc0(f32[esp+0x4])  (rounds to f32)
    4999a3: X87 STORE f32 [esi+0x18] <- f32[esp+0x4]  (rounds to f32)
    4999b4: X87 STORE f32 [esp+0x8] <- ((f32[ebp+0x8] * f32[ebp+0x8]) + (f32[ebp+0xc] * f32[ebp+0xc]))  (rounds to f32)
    4999bc: X87 STORE f32 [esp] <- f32[esp+0x8]  (rounds to f32)
    4999bf: CALL 0x40ea20 sqrtf_wrapper  regs[]  stack[ret(sub_da0dc0)]
    4999c4: X87 STORE f32 [esi+0x1c] <- ST0<ret sqrtf_wrapper>  (rounds to f32)
    4999cd: RETURN 8

  call targets: 0x40ea20(sqrtf_wrapper), 0xda0dc0
```

Disassembly:
```
== 499940
00499940  push ebp
00499941  mov ebp, esp
00499943  and esp, 0xfffffff8
00499946  sub esp, 8
00499949  fld dword ptr [ebp + 8]
0049994c  fld qword ptr [0xe5bfa0]  ; f32=0
00499952  fsubr st(1)
00499954  fstp dword ptr [esp + 4]
00499958  fld dword ptr [0xdd5984]  ; f32=0.001
0049995e  fld dword ptr [esp + 4]
00499962  fcom st(1)
00499964  fnstsw ax
00499966  test ah, 5
00499969  jp 0x499986
0049996b  fchs 
0049996d  fcompp 
0049996f  fnstsw ax
00499971  test ah, 5
00499974  jp 0x49998a
00499976  fstp st(0)
00499978  fldz 
0049997a  fst dword ptr [esi + 0x18]
0049997d  fstp dword ptr [esi + 0x1c]
00499980  mov esp, ebp
00499982  pop ebp
00499983  ret 8
00499986  fstp st(1)
00499988  fstp st(0)
0049998a  fdivr dword ptr [ebp + 0xc]
0049998d  fstp dword ptr [esp + 4]
00499991  fld dword ptr [esp + 4]
00499995  call 0xda0dc0
0049999a  fstp dword ptr [esp + 4]
0049999e  fld dword ptr [esp + 4]
004999a2  push ecx
004999a3  fstp dword ptr [esi + 0x18]
004999a6  fld dword ptr [ebp + 8]
004999a9  fld dword ptr [ebp + 0xc]
004999ac  fmul st(0), st(0)
004999ae  fld st(1)
004999b0  fmulp st(2)
004999b2  faddp st(1)
004999b4  fstp dword ptr [esp + 8]
004999b8  fld dword ptr [esp + 8]
004999bc  fstp dword ptr [esp]
004999bf  call 0x40ea20  ; sqrtf_wrapper
004999c4  fstp dword ptr [esi + 0x1c]
004999c7  add esp, 4
004999ca  mov esp, ebp
004999cc  pop ebp
004999cd  ret 8
; end 004999d0 size 90
```

### anim_state_find_by_name — 0x6EF510

**Convention.** Stack (char* name or 0), cdecl, returns EAX = index or -1.

`name == 0` -> -1. `h = string_hash_lower(name)` (EAX = 0, ECX = name, `&h` pushed); for `i = 0 .. 0x29F`: if `hashes[i] (0x28F05E8) == h` and `_stricmp(name, table[i] (0xE8EE90 + i*12, first dword))` is 0 -> return `i`; else continue. Returns -1 after 0x2A4 entries.

```
FUNCTION 0x006EF510  size 0x66  insns 44  blocks 8
  entry regs read before write: ['eax']

  B0 @0x6ef510:
    6ef517: if (i32[esp+0x8] != 0) goto B2  else fallthrough   ; jne 0x6ef51e

  B1 @0x6ef519:
    6ef51d: RETURN

  B2 @0x6ef51e:
    6ef52a: CALL 0xbdcb30 string_hash_lower  regs[eax=0, ecx=ebx]  stack[&[esp+0x14]]

  B3 @0x6ef540:
    6ef547: if (ebp != i32[esi*4+0x28f05e8]) goto B5  else fallthrough   ; jne 0x6ef559

  B4 @0x6ef549:
    6ef54d: CALL 0xc99030 __stricmp  regs[]  stack[i32[edi]]
    6ef557: if (ret(__stricmp) == 0) goto B7  else fallthrough   ; je 0x6ef56f

  B5 @0x6ef559:
    6ef565: if ((esi add 0x1) < 0x2a4) goto B3  else fallthrough   ; jl 0x6ef540

  B6 @0x6ef567:
    6ef56e: RETURN

  B7 @0x6ef56f:
    6ef575: RETURN

  call targets: 0xbdcb30(string_hash_lower), 0xc99030(__stricmp)
  stack objects passed by address: &[esp+0x14] -> string_hash_lower
```

Disassembly:
```
== 6ef510
006ef510  push ebx
006ef511  mov ebx, dword ptr [esp + 8]
006ef515  test ebx, ebx
006ef517  jne 0x6ef51e
006ef519  or eax, 0xffffffff
006ef51c  pop ebx
006ef51d  ret 
006ef51e  push ebp
006ef51f  push esi
006ef520  push edi
006ef521  lea eax, [esp + 0x14]
006ef525  push eax
006ef526  xor eax, eax
006ef528  mov ecx, ebx
006ef52a  call 0xbdcb30  ; string_hash_lower
006ef52f  mov ebp, dword ptr [esp + 0x14]
006ef533  xor esi, esi
006ef535  mov edi, 0xe8ee90
006ef53a  lea ebx, [ebx]
006ef540  cmp ebp, dword ptr [esi*4 + 0x28f05e8]
006ef547  jne 0x6ef559
006ef549  mov ecx, dword ptr [edi]
006ef54b  push ecx
006ef54c  push ebx
006ef54d  call 0xc99030  ; __stricmp
006ef552  add esp, 8
006ef555  test eax, eax
006ef557  je 0x6ef56f
006ef559  add esi, 1
006ef55c  add edi, 0xc
006ef55f  cmp esi, 0x2a4
006ef565  jl 0x6ef540
006ef567  pop edi
006ef568  pop esi
006ef569  pop ebp
006ef56a  or eax, 0xffffffff
006ef56d  pop ebx
006ef56e  ret 
006ef56f  pop edi
006ef570  mov eax, esi
006ef572  pop esi
006ef573  pop ebp
006ef574  pop ebx
006ef575  ret 
; end 006ef576 size 66
```


## Other functions worth a recipe later

`cutscene_object_is_cutscene_character` / `_vehicle` (0x6C6E80 / 0x6C6EE0, EDI = object, loop over the player lists), `camera_pending_callbacks_update` (0x4A3540), `cutscene_table_find_by_hash` (0x6C3A20), `rig_find_bone_by_name` (0xCE0E00), `camera_script_is_finished_check` (0x49FCA0).

