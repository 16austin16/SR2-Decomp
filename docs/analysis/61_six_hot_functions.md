# 61 - The six hot unnamed functions of doc 60

Scope: `0x527020`, `0x9A1970`, `0x736A10`, `0x72CD60`, `0x50FD40`, `0xB7E090` (doc 60, sec. 1) and their unnamed direct callees.
Method: capstone disassembly of `bin/SR2_pc.unpacked.exe`, callers by scanning for `call rel32`, callees by the same scan, field offsets compared with
docs 15, 18, 47, 57-60. No game run, no `.rs` or `functions.csv` change. Scripts: `scratchpad/doc61/*.py` (disassembler with name annotation, caller scan, CSV builder).
All 68 new rows are in `docs/candidates_doc61.csv` (6 main functions plus 62 callees, of which 15 are second level: callees of the direct callees).
`documented` = the instructions or an earlier doc confirm the role; `named` = inferred (the notes say which parts).

## 1. Names

| Address | Name | Status | Role in one line |
|---|---|---|---|
| 0x527020 | `render_stage4b_list0b_draw` | documented | `render_frame` stage `render4b`: draws the pass-0 flag-0x40 list and its per-light variants |
| 0x9A1970 | `human_movement_solve` | documented | the "solve the movement" step of `humans_movement_update`: desired motion vector, speed, active flag |
| 0x736A10 | `character_controller_update_phantom` | named | per-human, per physics sub-step: place the character phantom / query volume, refresh the contact manifold |
| 0x72CD60 | `hk_custom_character_proxy::check_support` | named | SR2's copy of `hkpCharacterProxy::checkSupport` (supported / unsupported test against the manifold) |
| 0x50FD40 | `effect_emit_particles` | named | spawns `count` particles of an effect instance |
| 0xB7E090 | `distant_vehicle_advance_on_road` | named | moves one distant (far LOD) vehicle along its road lane, choosing the next lane at the end |

### 0x527020 render4b

Doc 18 already places it: "render4b: `[0xE98D74]` = `[0xE5C340]`. If `[0x25273E4]` and `[0xE94D37]`: 0x518810. Then `[0xE94D37]` = 1; 0x527020", with the profile label `render4b` around it
in `render_frame` (0x52AABA, its only caller). Facts from the instructions:

- It first resets the pipeline state (`gfx_sampler_slot_set(1, 3 / 0x12)`, `gfx_set_depth_range_constants`, `gfx_clear(0)`, `render_visibility_list_pass5` = `render(5, 0)`, then a block of cached
  render-state sets), sets `ctx+0x46C..0x474 = -1` and `[ctx] = 1`, calls `gfx_reset_sampler_states`, `gfx_reset_render_state_defaults_b`, and `gfx_set_fade_mode_flag(1)`.
- Then it walks the "pass 0, flag 0x40" list of doc 18 (entries of 0x14 bytes at `0x2784998`, count `[0x2789998]`): `[e+0]` = instance, `[e+4]` = render flags, `[e+0xC]` = light-slot mask.
  Per entry: if flags & 0x400 (the "sorted / alpha" variant) the state block is rewritten (color write, state 0x38 / 0x39 / 0x3A depending on `instance->kind == 9 or 0xD`), then
  `render_instance_draw(8 or 7 or 0, view, (flags >> 5) & 1)` once for the plain view and once per light slot (`for i in [first_slot, [0x2526CCC])` where bit i of `[e+0xC]` is set, the light being
  `0x2612D10 + i * 0x280` bound with `gfx_set_light_constants`). Kind 0xD instances are lists (count `+8`, pointers from `+0xC`), kind 9 / others are single.
- It ends with `jmp gfx_pass_end` (0xD184D0), i.e. the stage's frame is torn down by the callee's `ret`.

**Port map.**
- Convention: cdecl, no args, no result. No frame pointer; `sub esp, 0x24`; pushes EBX/EBP/ESI/EDI at entry, pops them before the tail `jmp 0xD184D0` (so ESI/EDI/EBX/EBP are preserved).
  EAX/ECX/EDX and the flags are clobbered. x87 not used.
- Stack locals: `[esp+0x17]` = "light views active" byte (`[0x2612D18] bit 13`, only if `[0x2526CCC] != 0`), `[esp+0x18]` = current list entry, `[esp+0x1C]` = last bound light, `[esp+0x20]` = instance,
  `[esp+0x24]` = first light index, `[esp+0x28]` = ctx (`fs:[0x2C][0]`), `[esp+0x2C]` = list item index, `[esp+0x30]` = entry index.
- Callees and their registers: `gfx_sampler_slot_set` (ECX = EDX = slot, stack (slot, kind)), `gfx_set_depth_range_constants` (none), `gfx_clear` (ECX = 0), `render_visibility_list_pass5` (none),
  `gfx_reset_sampler_states`, `gfx_reset_render_state_defaults_b` (none), `gfx_set_fade_mode_flag` (CL = 1), `gfx_set_light_constants` (EAX = light), `render_instance_draw` (EDI = instance, cdecl 3 args), `gfx_pass_end` (none).
- Most of the 600 instructions are the "if the cached state differs, append a 12-byte record {15, state, value} at `[0x33D62F8]` and advance" idiom (cache at `0x22F5xxx`); the Rust port already has that idiom
  as helpers (`gfx_*` in port_render.rs). The ring pointer `[0x33D62F8]` is the render-thread command ring; this function must run on the render thread.
- Risks: (1) list entries are mutated concurrently on the game thread only before the stage, fine; (2) the tail jump means the Rust port must call `gfx_pass_end` (original) and return, not replicate
  the unwinding; (3) the ring record order and the state caches must match bit for bit, otherwise D3D state diverges silently (verify with the ring-record dump already used for `render_stage2_a4`);
  (4) the instance vtable calls inside `render_instance_draw` stay original. Port as: Rust loop + original callees through thunks. Medium risk, pure state plumbing.

### 0x9A1970 human movement solve

Called once per human from `humans_movement_update` (0x9A99E7) with the 0x20-byte per-human record the caller builds on its stack (`{human*, vec3 a, vec3 b, u8 active}`, a = ebx-0xC, b = ebx, flag = ebx+0xC).

**What it does** (doc 15: "Solve the movement (0x9A1970). The result is the desired position (position + velocity x dt)"): zero velocity `human+0x5C8..0x5D0` and `+0x5B4`; test `human_movement_solve_allowed`;
set `+0xCC |= 0x200` (controller active) else clear it and return 0; `human_player_move_input_active` -> `*out_active`. The stick vector `[+0x1370, +0x1374]` is zeroed when both below a dead zone. The target speed is then
selected from the movement state (`+0x590` mode byte: 0, 1, 2, 3 plus carried-object class `human_carried_object_class`, player scripted mode `player_human_scripted_move_active`, `human_has_attachment_e88`,
ground / water flags `+0xC0`) through a long constant ladder (0xE88620, 0xE8862C, 0xE88634, 0xE88648, 0xE88660, 0xEB7D3C, ...), the heading is built from the camera axes (`camera_flat_axes_normalized`),
the orientation matrix (`human+0x20`, `mat33_get_yaw_ecx`, `mat33_to_euler_angles_esi`, `mat33_from_euler_angles`, `mat33_from_forward_up`), and the animation root motion of the rig
(`+0x5E0` -> `rig_motion_deltas_by_handle`, `anim_motion_def_find_index`, `rig_track_time_by_handle`, `anim_clip_motion_sample`) is rotated into world space (`mat33_mul_vec3_in_place`, `vec3_mul_mat33_in_place`).
Special branches: `+0x591 == 4` with `+0x592` in 2..4 (the action-move states, takes `+0x580` as is), cutscene / vehicle transition (`human_entering_or_exiting_vehicle`), local player (`handle_is_local_player_object`).
Final: velocity `+0x5C8..` = world delta / dt, `+0xFC0..0xFC8` = 0, return AL = 1.

**Port map.**
- Convention: cdecl, stack args `[ebp+8]` human, `[ebp+0xC]` vec3* out a (EDI inside), `[ebp+0x10]` vec3* out b (normalised direction in the walking case), `[ebp+0x14]` u8* out active, `[ebp+0x18]` f32 dt;
  caller pops 0x14. Returns AL: 0 only for the early "not allowed" exit (flag `+0xCC & 0x200` cleared), 1 otherwise (the caller tests `== 1`).
- Preserved: EBX, ESI, EDI (pushed), EBP frame (`and esp, -8`, 0x11C bytes locals). EAX/ECX/EDX/XMM0-2 clobbered. x87 is used heavily (f32 stores between steps, constants are f64 from the image); it
  is empty at both exits.
- Callees and register inputs: `human_movement_solve_allowed` (EAX = human -> AL), `human_player_move_input_active` (EDX = human -> AL), `human_has_attachment_e88` (stack human), `human_carried_object_class` (EDX = human -> EAX),
  `player_human_scripted_move_active` (stack human), `human_on_foot_free` (EAX = human), `human_entering_or_exiting_vehicle` (EAX = human), `handle_is_local_player_object` (ECX = handle),
  `action_id_flags_test` (EAX = action word, ECX = mask), `mat33_get_yaw_ecx` (ECX = &mat33 -> XMM0), `mat33_to_euler_angles_esi` (ESI = &mat33, stack outs), `mat33_from_euler_angles` (EAX/stack as documented, ret 0x10),
  `mat33_from_forward_up` (cdecl 3 args ret 0xC), `mat33_from_forward_up_pose` (stack, ret 8), `vec3_normalize_in_place` / `vec3_normalize_safe` / `vec3_normalize_safe_len` (ESI = &v), `vec3_normalize_to` (ESI in, EDI out),
  `vec3_mul_mat33_in_place` (ECX = &v, EAX = &m), `mat33_mul_vec3_in_place` (ECX = &v, EAX = &m), `rig_motion_deltas_by_handle` (ECX = rig handle, 3 stack outs), `rig_track_time_by_handle` (ECX = handle, stack track),
  `anim_clip_motion_sample` (EAX = clip index, stack time / scale / out), `anim_motion_def_find_index` (cdecl), `human_move_leash_speed_scale` (EDI = human, stack f32, f32*), `camera_flat_axes_normalized` (EAX / EDI = outs),
  `object_get_body_world_position` (ECX = controller, stack out), `vec3_ne_exact` (cdecl), `vec3_in_range_check` (ECX), `vec3_is_finite_eax` (EAX).
- Buffers: 0x11C-byte frame holding two mat33 (0x24 each), several vec3 temporaries (`esp+0x18..0x54`, `esp+0x98` body position), no heap.
- Risks: (1) about 900 instructions with six distinct movement-state branches, most of them reading `[0xE88xxx]` tuning constants (read the f32 constants from the image once; do not hard-code);
  (2) f32 store rounding between each x87 step is observable (the velocity is integrated every frame), so port the expressions literally, including `fxch` ordering; (3) `human+0x5C8..` and `+0xFC0..` are the only
  non-out writes, and `+0xCC & 0x200` is read by `humans_character_step`; (4) the rig-table lookups (index % 0xBE, 0x5E8 stride, 0x340D240) must stay on the original helpers.
  Hard: do not port first. Port the leaf helpers (`human_movement_solve_allowed`, `human_player_move_input_active`, `human_on_foot_free`, `human_carried_object_class`, `vec3_*` checks) now, the solver last.

### 0x736A10 character_controller_update_phantom

Only caller: `humans_character_step` 0x9A9C4B, `ECX = [human+0x570]`, one stack f32 (the sub-step dt) - doc 15 "0x736A10(step) per human".

**What it does.** `mat44_from_mat33_pos` builds a world pose from the controller's rotation (+0x188) and position (+0x194). The query extent is `radius = |v1| + |v2| + sqrt(|v3|^2) ...` where
v1 = `+0x140` (xyz, SSE length), v2 = `+0x160` (xyz) and three scalars `+0x170/+0x174/+0x178` (shape half extents) and `+0x1B8/+0x1BC/+0x1C0`, then `+ (phantom +0x68 + 0x6C) * dt-like term`.
The phantom (`[ctrl+0x10]`) is moved: `call [vtbl+0x40](phantom, &mat44, radius)` (callee pops 8). If `[ctrl+0x1C4] & 0x80` it runs the contact refresh:
local `havok_custom_character_collector::ctor`, `hk_custom_character_proxy::refresh_manifold(EDI = [ctrl+8] = proxy, collector)`, `...::dtor`.

**Port map.**
- Convention: thiscall (ECX = controller), one stack f32 `[ebp+8]`, `ret 4`. Result: none (side effects in the phantom and proxy).
- Preserved: EBX, ESI, EDI (pushed / popped); EBP frame, `and esp, -16`, `sub esp, 0x854`, stack cookie at `[esp+0x850]` (xor esp), checked with `__security_check_cookie`.
- Frame: mat44 at `esp+0x5C` (64 bytes, aligned), collector object at `esp+0x90` (about 0x7C0 bytes, constructed only on the flag path), f64 temporaries at `esp+0x24..0x3C`.
- Callees: `mat44_from_mat33_pos` (ECX = &pos, EDX = &out, stack &mat33), `sqrtf_wrapper` x2 (stack f32), phantom vtable slot (stdcall 2), `havok_custom_character_collector::ctor` (ESI = object),
  `hk_custom_character_proxy::refresh_manifold` (EDI = proxy, stack collector, ret 4), `havok_custom_character_collector::dtor` (ESI = object).
- Risks: the math is small (it is the port-friendly part, about 110 instructions), but the phantom call goes into Havok C++ code that reads the matrix as `hkTransform` (needs a 16-byte aligned 64-byte buffer), and the
  collector is a stack C++ object with a vtable. Keep the collector construct / destruct and the vtable call as original calls. Easy to medium: port the math, keep the three calls.

### 0x72CD60 hk_custom_character_proxy::check_support

Only caller `player_human_move_input_update` 0x731F7D (the player's Havok ground test). Method of the custom proxy (`hk_custom_character_proxy`, ctor 0x72CB60); the SDK's own copy is `hkpCharacterProxy::vf3` (0xD65060),
which calls the same `hkSimplexSolver_solve`.

**What it does** (follows Havok's `checkSupport`): sets bit 0 of `proxy+0xD0` from `manifold_count + extra + 10 > 0x20`; builds `min(count, 0x20)` contact planes by calling the proxy's vtable +0x10 per manifold entry
and `hk_custom_character_proxy_add_max_slope_plane(up = +0x50, slope = +0xB4)`; grows three local `hkArray`s with `hkArray_reserve_exact` (stack small-buffer arrays of 0x20 elements, sizes 0x10 and 0x40);
allocates the solver work area from the thread's hkThreadMemory (TLS index `[0x31BF2BC]`, `0xDB807C` = `TlsGetValue`); calls the listener array (`hkpListenerArray_notify_reverse`, EDI = proxy+0x84 area, 2 stack args)
so game code can add or edit constraints; `hkSimplexSolver_solve(input, output)`; then compares the solved velocity with the input per component (`cmpleps` with the tolerance `[0xDD5984]`, `movmskps & 7 == 7` -> unsupported).
If different it averages the support normals (`rsqrtss`-based normalise), writes the surface info and sets `*info = 2`; else `*info = 0`. All three arrays are released at the end with `hkThreadMemory_deallocateChunk`.

**Port map.**
- Convention: thiscall ECX = proxy, one stack arg `[ebp+8]` = info* (offset +0 = supported state dword: 2 or 0; the caller also pre-fills +0x38 = 7 and a vec4 pair), `ret 4`. No return value (EAX is garbage).
- Preserved: EBX, ESI, EDI. EBP frame, `and esp, -16`, `sub esp, 0x8F4`, cookie `[esp+0x8F0]`; arrays at `esp+0xF0` (0x20 x 0x40, small buffer), `esp+0x30..` (planes / flags, 0x10-byte elements).
- Callees: `hkArray_reserve_exact` x4 (cdecl (array, capacity, elem size)), `hk_custom_character_proxy_add_max_slope_plane` (EDI = plane array, ECX = index, stack slope + &up), `hkpListenerArray_notify_reverse` (EDI = container, 2 stack args, ret 8),
  `hkSimplexSolver_solve` (cdecl 2 pointers), `hkvector4_dot3_store` (cdecl (&out, &a ...)), `hkThreadMemory_deallocateChunk` x3 (ECX = memory, stack (ptr, size, class 0x17), ret 0xC), `TlsGetValue` x5,
  `proxy vtable +0x10` (stdcall (index, ?, f32)).
- Risks: it drives live Havok objects (listener callbacks, thread memory free lists shared with Havok's C++ code, doc 60 item "skipped on purpose"), SSE with exact lane order, and the solver output layout of
  `hkSimplexSolver_solve` which is 0xC2AE90 (a second 1,000-instruction function). Hard: port after `hkSimplexSolver_solve` and the memory helpers; shadow-test against recorded (proxy, info) pairs.

### 0x50FD40 effect_emit_particles

Callers: `effect_instance_update` (0x50FD0D, with `count`, `pos` and zero arrays) and `effect_object_stop` (0x50B0A9). Sibling `0x5105E0` (`effect_emit_secondary_particles`, list at `inst+0xB4`) is the same pattern.

**What it does.** inst = `[esp+0xC4]` (effect instance), template = `inst[0]->+0x2C + inst[1] * 0xC0` (`ebx`). Particle life and size come from two curves (`curve_eval_scalar` on `template+0xA4/+0xA8` and `+0xAC/+0xB0`
over `inst[0]->+0x10 * inst+0x98`). For each of `count` particles: pops a record from the free list `[0xFA20A0]` (doubly linked, +0x3C / +0x40, empty -> stop), links it into the emitter list `inst+0xB0`,
increments `inst+0x9E` and the global live count `[0x2527298]`, picks a random value (`rng_float01`, or the pregenerated table `0xEC3AC0[0x800]` with index `[0xEC3ABC]`) for life, offset, spin and size,
fills position (+0..+8, from `pos` or the per-particle array `pos_array`, else from the template shape: shape kind `[template]` 0 = cone / sphere using `vec3_random_unit`, `vec3_random_cone`, `vec3_random_cone_around_dir`; 1 = ...),
velocity direction (+0x14..+0x1C), life (+0x24), age (+0x28), flags (+0xC, bit 5 random flip via `_rand`), spin vec (+0x30..+0x38), and, when the template flag `+6 & 0x10` and `pos_array == 0`
transforms position and velocity into world space by the instance matrix (`inst+0x14`, `mat33_mul_vec3_in_place`).

**Port map.**
- Convention: cdecl, 7 stack args: `[esp+0xC4]` inst, `+0xC8` count (int, `<= 0` returns), `+0xCC` pos* (vec3), `+0xD0` pos_array* (stride 0xC, may be 0), `+0xD4` dir_array*, `+0xD8` life_array*, `+0xDC` age_array*; the caller pops 0x1C. No result.
  (offsets are after the 4 pushes; at entry the first arg is `[esp+0xC4]`.)
- Preserved: EBX, EBP, ESI, EDI (pushed / popped); no frame pointer, `sub esp, 0xB0`. x87 empty at exit; mixed f32 / f64 (f64 constants at 0xE5BF40 / 0xE5BF60 / 0xE5BF70 / 0xE5BF78 / 0xE5C0D8 / 0xE5D210).
- Locals: `esp+0x10..0x58` scalars (life / size / spin randoms), `esp+0x30..0x38` direction, `esp+0x5C..0x64` second vec, `esp+0x74..0x7C` normalised copy, `esp+0x84..0x8C` world position.
- Callees: `curve_eval_scalar` (stack (x, curve ptr, ...), cdecl), `rng_float01` (EAX = 0xEC30F8 -> ST0), `_ftol2_sse` (ST0 -> EAX), `_rand`, `vec3_random_unit` (ESI = out), `vec3_random_cone` (ESI = out, stdcall 2 f32),
  `vec3_random_cone_around_dir` (ECX = out, EAX = dir, stdcall 2 f32), `mat33_from_forward` (EAX = &dir, ECX = &mat), `vec3_mul_mat33_in_place` (ECX = &v, EAX = &m), `mat33_mul_vec3_in_place`, `vec3_normalize_in_place` (ESI).
- Risks: (1) **random order**: the particle effect consumes `rng_float01` and `_rand` in a fixed order; a port must call the same generators in the same order or replays and the shared RNG stream diverge (the global
  generator 0xEC30F8 is also used by gameplay). (2) free list and counters are shared with `effect_object_stop` / `effect_instance_update` / the particle renderer (`r_particle_inst::pre_render`); update them with the same
  pointer order. (3) pure game thread, no TLS. Medium: long but straight line, no vtables; the shared RNG is the real risk.

### 0xB7E090 distant_vehicle_advance_on_road

Callers: `distant_vehicles_update` (0xB800A8, per vehicle) and `distant_vehicle_place_on_lane` (0xB7E580 at 0xB7E769, once at spawn). Record layout (from both):
`+0..+8` position, `+0xC..+0x14` unit tangent, `+0x18` lane*, `+0x1C` speed, `+0x20` distance on the lane, `+0x24` spline parameter t, `+0x28` fade, `+0x32` type byte, `+0x33` flags (bit 0 = "needs lights update").

**What it does.** Lane = `rec+0x18` (0 -> return). The speed limit is the lane-type speed (`[0x223E3E8 + (lane+0x28 >> 4)*4]`, 1000.0 for a no-road lane, 10.0 for type >= 3). The record's own speed `rec+0x1C`
eases toward it by a clamped fraction (0xE5C130). Distance `rec+0x20 += speed * dt` (dt = `rec+0x1C` times a second factor). When the distance passes the lane length (`lane+0x14`): the type table entry
(`0x22DBC20 + type*0x2C`, global limit `[0x252A18C]`) and the lane-type filter `[0x22DBC10]` select the lane links (up to 5 lane records, `lane+0x2C` count, ids u16 from `[0x2225B64 + 0x24]` at `lane+0x30`),
each linked lane contributing a weight (`traffic_lane_type_speed` and zone limits); a random pick (`rng_float01 * total`) selects the next lane; the traffic counters `zone+0x44` of the old and new zone
(`road_lane_zone_traffic_info`) are updated (each `+0x4C = (rand-mix of +0x1C / +0x20) * (0..1 day-night factor from game_clock_day_night_factor) / +0x48`), the lane claim is moved
(`road_lane_claim_remove_b` / `road_lane_claim_add_b`), the distance carries over (`rec+0x20 -= lane length`) and `rec+0x33 |= 1`. If no link is usable the vehicle's lane is cleared (`rec+0x18 = 0`, zone counter
decremented). After either path the spline parameter is recomputed: `spline_param_at_distance(lane+0x14, rec+0x20, 0.05125)` for a fresh lane, else `spline_param_advance_by_distance` from `rec+0x24`,
then `spline_eval_position(EAX = lane, ESI = rec, stack t)` and `spline_eval_tangent(EAX = lane, ECX = rec+0xC, stack t)`, `vec3_normalize_safe(ESI = rec+0xC)`.

**Port map.**
- Convention: stdcall, one stack arg `[esp+0x4C]` (record*), `ret 4`, no result.
- Preserved: EBX, EBP, ESI, EDI (pushed / popped). No frame pointer; `sub esp, 0x40`; scratch `esp+0x10..0x54` (candidate lane array: pairs at `esp+0x28 + i*8` = (lane*, remaining weight)); x87 empty at exit.
- Callees: `road_lane_zone_traffic_info` (EAX = lane -> EAX zone), `road_lane_claim_remove_b` (EAX = lane, EDI = owner), `road_lane_claim_add_b` (ESI = lane, stack owner), `traffic_lane_type_speed` (ECX = lane -> ST0),
  `game_clock_day_night_factor` (no args -> ST0, EDX = the live count is set up by the caller of the pair), `spline_param_advance_by_distance` (EDI = lane, 3 stack f32 -> ST0), `spline_param_at_distance` (ECX = &spline, 2 stack f32),
  `spline_eval_position` (EAX = lane, ESI = out rec, stack t), `spline_eval_tangent` (EAX = lane, ECX = out, stack t), `vec3_normalize_safe` (ESI), `rng_float01` (EAX = 0xEC30F8).
- Risks: (1) one `rng_float01` per lane change on the shared generator (order matters); (2) the claim pool `0x223E410` and zone counters are shared with the traffic AI (`road_lane_claim_*` are Rust ports; keep calling them through
  their Rust entry points); (3) x87 ordering with f32 stores; (4) both callers are Rust or about to be: `distant_vehicles_update` is already Rust, so this is the natural next leaf. Medium: no Havok, no vtables,
  about 400 instructions, mostly float comparisons and table indexing; port with the helpers above and the shadow harness used for `nav_*`.

## 2. Easiest to port first

1. `character_controller_update_phantom` (0x736A10): about 110 instructions of math plus three original calls.
2. `distant_vehicle_advance_on_road` (0xB7E090): self-contained, helpers already Rust, one RNG call per lane change.
3. `effect_emit_particles` (0x50FD40): long straight-line code, no vtables; the shared RNG order is the only trap.
4. `render_stage4b_list0b_draw` (0x527020): large but mechanical (state cache idiom), needs the render-thread harness.
5. `hk_custom_character_proxy::check_support` and `human_movement_solve`: hard; do their leaves first (list in sec. 3).

## 3. Callee port candidates found on the way (all small leaves)

`human_movement_solve_allowed` (0x9A1870), `human_player_move_input_active` (0x9A17B0), `human_has_attachment_e88` (0x9B7880), `human_on_foot_free` (0x95F500), `human_carried_object_class` (0x9CBB00),
`vec3_in_range_check` (0xBDDA80, 86 callers), `vec3_is_finite_eax` (0xBDDAE0, 28 callers), `gfx_fade_pass_active` (0xD755F0, 18 callers), `traffic_lane_type_speed` (0xB81090), `road_lane_zone_traffic_info` (0xB800C0),
`game_clock_day_night_factor` (0x68A500, 8+ callers), `hkpArray_remove_ptr_unordered` (0xD53200), `asinf_wrapper` (0x4F1AC0).
`road_lane_claim_add_b` / `road_lane_claim_remove_b` (0xA8B830 / 0xA8B8F0) are byte-identical copies of the Rust-ported 0xA8B6F0 / 0xA8B7B0 apart from the early-out branch targets, so the existing ports can be installed on them too.

## 4. Open items

- Merge `docs/candidates_doc61.csv` into `docs/functions.csv` (68 rows; none collides with an existing address or name).
- The exact meaning of `human+0xE88` (`human_has_attachment_e88`) and of the `+0x590` mode values 0..3 in `human_movement_solve` needs one in-game trace (log `+0x590`, `+0xE88`, the out vectors for a few seconds of walking, carrying, aiming).
- `0xD586E0` / `0xD58790` (small node allocate / link, 18 and 12 callers), `0xCE40D0`, `0xCE5070`, `0xCE32C0`, `0x72C150` internals and `0xBE49A0` are unnamed second-level helpers left for the next round.
