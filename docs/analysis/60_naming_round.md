# 60: Naming round on direct callees of hot named functions (188 names, 20 port candidates)

Input: `bin/SR2_pc.unpacked.exe`, `docs/functions.csv` (14,357 rows), docs 40, 45, 51, 52b, 56, 57, 58 and the sample logs `bin/sr2-mission-2026-11-04*.log`.
Output: `docs/candidates_doc60.csv` (address,name,subsystem,status,notes), 188 rows, none of which is in `functions.csv` yet. `functions.csv` and all `.rs` files are untouched.
Scratch scripts are in the session scratchpad `doc60/` (`lib.py` loader and function-start sweep, `graph.py` call graph, `seeds.py`, `small2.py` / `gview.py` / `line.py` card printers).

## 1. Method

1. **Function starts.** About 30,100 starts: every `functions.csv` row plus every E8/E9 target in .text that is preceded by CC, 90, C3 or a `ret n`
   (same sweep as doc 57). A function body is "up to the next start", so a tiny function with no padding in front can hide a sibling right behind it
   (examples: 0x40ea60 is followed by two more 2 to 3 instruction functions at 0x40EA7x; 0x962f50's neighbour is another 0x80-bit test). Only entry points are named.
2. **Seeds.** 1,693 named functions: subsystems cutscene / ai / effects / camera / nav, any name matching weather|shop|store|clothes|frame|lighting|sky|cloud,
   and every named address that appears in docs 40, 45, 51, 52b, 56, 57, 58 (the frame-loop, tss02 game and render thread, cutscene, weather, shop and AI sets).
3. **Targets.** Direct `call` / tail `jmp` targets of the seeds that are not in `functions.csv`: 2,254 functions (1,574 under 300 bytes).
   Each was ranked by sample hits (return addresses from the 04b/04c/04d logs, 503 targets have hits), by number of callers, and by size.
4. **Evidence per name.** Disassembly with capstone (small functions fully read, big ones through strings, callee names and field offsets). Name sources, in order of trust:
   string constants (66 names, the whole "documented" and most "ui / cutscene / net" rows), exact data-structure matches to documented globals
   (object table 0x2149C64, `[obj+0x44]` handle, `+0x4C` type, `+0x3CC` human kind, `+0x84D8` vehicle sim state, `+0x84E4 -> +0x9C` vehicle class, table 0xE90E44 of rig actions),
   the sibling rows around it (`ai_order_kXX_*`, `hkvector4_*`, `cutscene_*`), and the callers' names.
5. **Conventions.** lower snake_case with the subsystem prefix; register-argument leaves get a `_regs` / `_eax` suffix (the existing `vec3_ne_exact` style);
   when the meaning of a field is unknown the name states the field (`human_npc_field_1178_set`, `object_flag_c0_bit10_set`) rather than guessing a role.
   Status `documented` (6 rows) = the strings or the code state the role; `named` (182 rows) = inferred. Every address was checked against the start set
   and against `functions.csv` and against the other new names (no duplicates).

Things that did not work: the hottest unnamed callees (0x527020 render stage, 0x9A1970 / 0x736A10 / 0x72CD60 human movement, 0x50FD40 effect instance update, 0xCB71D9 inside the CRT pow/sqrt tail,
0xB7E090 distant vehicles) are 600 to 3,300 bytes each and need a separate pass; I did not name them from a skim. Sample attribution to "the function containing the address" is also
wrong wherever a start is missing, so counts below use sample numbers only as a ranking.

## 2. Summary by subsystem (188 names)

| csv subsystem | n | Source area / what |
|---|---:|---|
| ui | 27 | script-call wrappers (dialog_box_*, main_menu_*, tutorial_*), HUD text selectors, pause / pushback / cellphone prompts |
| cutscene | 26 | `cutscene_begin_callback` / `cutscene_stop` / `cutscene_shot_start` helpers, XML parsers (Chunks, Emotion, Audio, TODs / Lights), voice / audio / peg loaders |
| human | 15 | action-flag predicates over table 0xE90E44, kind-2 (NPC) predicates, ethnicity, leader tests |
| ai | 13 | ai_order_k04 / k14 helpers, action node lookup, spawn request, damage record, Gang_Saints tables |
| math | 12 | vec3 / mat33 / rng leaves used by 20 to 100 AI and camera callers |
| havok | 11 | `hkvector4_*` SSE leaves in the contact solver, hkThreadMemory allocate / deallocate fast paths (299 + 824 callers), quaternion to rotation |
| vehicle | 11 | class / driver / velocity accessors, license plate, air control parser |
| net | 7 | mp setup / login UI, store discount send, net record by handle |
| debug | 6 | console keyword / number parsing, debug HUD and plane export |
| render | 6 | frame-stage helpers (`render_list_entries_draw_all`, `player_skin_blend_slots_publish`, clip-plane sphere test), mesh record init |
| anim | 6 | rig table accessors, morph and muzzle-flash loaders |
| streamman | 5 | chunk object by id, zone chunk tests, component logo, level number lookup |
| object 4, world 3, crt 3, weapon 3, gamespy 3, script 3, util 3 | 22 | |
| camera 2, progression 2, data 2, activities 2, vint 2, lua 2 | 12 | |
| audio, effects, app, save, missions, physics, nav, road, file | 9 | 1 each |

By the areas asked for: frame loop and render_frame / game_main_thread_proc callees about 16, tss02 game thread (docs 57) about 45 (math, havok, AI predicates, human action flags),
tss02 render thread (docs 58) about 12, cutscene 33, shops / stores / chop shop 10, weather / clock / lighting 8, AI orders and their helpers about 45.

## 3. Names worth a second look

* Moderate confidence (role inferred from callers and shape, not from a string): `damage_record_init` (0x4171C0, 125 callers, copies pos + mat33 into a 0x6C record; could also be a generic "event record"),
  `action_node_from_handle` (table 0x2A4E508, 0x960 x 0x6C; I tied it to the `actn_node` packet 0x63 and `ai_spot` code, not proven),
  `camera_restore_after_cutscene`, `screen_fade_set_alpha`, `cutscene_net_send_play`, `player_skin_blend_slots_publish`, `render_list_entries_draw_all`, `human_is_follower`.
* `rng_state_fill_table` (0xBDCD90): the sibling 0xBDCC70 tests `[state+0x9C0] < 0x270`, so the generator is a Mersenne twister (624 words); `fill_table` refills the 0x800-entry output table.
* The CRT wrappers 0x4912D0 (`0xC96A90`, double), 0x4F1AC0 (`_CI` helper 0xDA0550) and 0x42EF30 (0xDA09F0) are x87 libm calls (probably exp / log family); left unnamed.
  0x4DA560 is `fmodf_wrapper` (helper 0xDA066A loops on `fprem`).

## 4. Port candidates (20): small, hot, tractable

Priority is by caller count and samples. Everything below was read in full. x87 note: where the original keeps products in ST0/ST1 and stores only once, compute in f64 (or f32 if the
control word is single precision: check with a shadow run) and round to f32 at the store exactly where the listing has `fstp dword`.
Every function listed has no globals other than the named constants.

| # | Address | Name | Callers | Samples |
|---|---|---|---:|---:|
| 1 | 0x40B790 | vec3_scale_in_place_eax | 3 | 30 |
| 2 | 0x485C30 | hkvector4_sub_store | 5 | 51 |
| 3 | 0xBB8A10 | hkvector4_equal_store | 4 | 37 |
| 4 | 0x9149A0 | action_node_from_handle | 20 | 51 |
| 5 | 0xCF7760 | world_mesh_record_set_instance_data | 1 | 75 |
| 6 | 0xBDEA80 | vec3_dist_sq_xz_regs | 36 | 25 |
| 7 | 0xA96A70 | vehicle_transform_store | 8 | 63 |
| 8 | 0xD22930 | view_clip_planes_sphere_outside | 12 | 199 |
| 9 | 0xAB5190 | vehicle_is_helicopter | 24 | 21 |
| 10 | 0x40EA60 | vec3_length_sq_eax | 52 | 2 |
| 11 | 0xBE2C60 | mat33_copy_regs | 57 | 11 |
| 12 | 0xBDEAB0 | vec3_dist_xz_regs | 6 | 0 |
| 13 | 0x40B8D0 | rng_chance_f32 | 17 | 0 |
| 14 | 0xBF0EA0 | f32_round_to_int | 15 | 0 |
| 15 | 0x729A60 | hkvector4_dot4_store | 13 | 9 |
| 16 | 0x72A100 | hkvector4_less_store | 7 | 2 |
| 17 | 0x71A0A0 | hkvector4_madd_splat_x | 5 | 17 |
| 18 | 0x485D30 | hkvector4_add_in_place | 5 | 0 |
| 19 | 0xBB8970 | hkvector4_and_mask_table | 3 | 7 |
| 20 | 0xD93F20 | hkrotation_from_quaternion_store | 37 | 57 |

Recipe format as doc 57 section 6: calling convention, outputs, preserved registers, callees, compare polarity, writes.

**1. vec3_scale_in_place_eax (0x40B790).** EAX=&v, one stack f32 s (callee pops, `ret 4`). Writes v.x, v.y, v.z only; EAX preserved, ECX/EDX untouched, x87 empty on exit; no callees.
Order: x = x*s stored (fstp), then y*s, then s*z. One f32 multiply each, so plain f32 multiplication reproduces it (a 24x24-bit product is exact in the x87 mantissa).

**2. hkvector4_sub_store (0x485C30).** ECX=&out, stack (&a, &b) at [ebp+8], [ebp+0xC], `ret 8`; ebp frame, `and esp,-16`. out[0..3] = a[0..3] - b[0..3] (subps, movaps: all three pointers 16-byte aligned).
EAX, EDX, XMM0, XMM1 clobbered; ECX preserved; no callees; no other writes.

**3. hkvector4_equal_store (0xBB8A10).** ECX=&a, stack (&out at [ebp+8], &b at [ebp+0xC]), `ret 8`. out = cmpeqps(b, a) per lane: 0xFFFFFFFF where equal, 0 otherwise (NaN lanes are 0).
EAX, XMM0, XMM1 clobbered; same aligned frame; no callees. (Sibling 0x72A100 `hkvector4_less_store`: same layout, `cmpltps a, b`, lane set when a < b.)

**4. action_node_from_handle (0x9149A0).** EDX=handle, no stack args, plain ret; result EAX = record pointer or 0; ECX clobbered, EDX preserved; no callees. Steps: handle 0 -> 0; idx = handle & 0xFFFF;
idx >= 0x960 (signed `jge`) -> 0; rec = 0x2A4E508 + idx*0x6C; the following `je` after the add is dead; `[rec+0x30]` == 0 -> 0; `[[rec+0x30]+4]` == 0 -> 0; then `ecx = ([rec+0x1C] != handle) - 1` (0 on mismatch,
all ones on match) and EAX = rec & ecx. So: valid only when the stored handle `[rec+0x1C]` equals the argument. No writes.

**5. world_mesh_record_set_instance_data (0xCF7760).** EAX=record, ECX=u32 value, four stack args, `ret 0x10`. Arg order from the reads: a0 (low word) -> `[rec+0x364]`, a1 = &src, a2 -> `[rec+0x584]`, a3 -> `[rec+0x588]`; ECX -> `[rec+0x360]`.
`rep movsd` copies 0x80 dwords (512 bytes) from a1 to `rec+0x368` (DF assumed clear). Then `[rec+0x568]` = `[rec+0x56C]` = -1 and `[rec+0x574]`, `[rec+0x578]`, `[rec+0x57C]` = 0. ESI/EDI are saved and restored, EAX preserved,
ECX = 0 and EDX clobbered on exit; no callees. Only caller `world_mesh_submit_full`, one call per mesh submitted.

**6. vec3_dist_sq_xz_regs (0xBDEA80).** ECX=&a, EAX=&b, no stack args, plain ret; result ST0 (one value left on the x87 stack, no other output). dx = a.x - b.x stored as f32 at `[esp+4]`, dz = a.z - b.z stored at `[esp]`;
then dx*dx + dz*dz is formed in ST0 without intermediate stores and stored/reloaded once as f32 (`fstp [esp+4]; fld [esp+4]`). All GPRs preserved, no callees, y ignored.

**7. vehicle_transform_store (0xA96A70).** EAX=vehicle, EDX=&pos (3 dwords), one stack arg &mat33 (9 dwords), `ret 4`. Copies the matrix to `veh+0xEC` (rep movsd 9) and pos to `veh+0x110..0x118`, then id = `[veh+0xA4]`:
if id == 0 or id > 0x80 (unsigned `ja`) stop; else slot = 0x324F7C4 + id*0x4C and writes pos to `slot+4..+0xC` and the matrix to `slot+0x10` (9 dwords). Compare polarity: `cmp eax,0x80; ja skip`.
EBX/ESI/EDI saved and restored; EAX, ECX clobbered, EDX preserved; no callees. Dword copies, no float conversion.

**8. view_clip_planes_sphere_outside (0xD22930).** Inputs: EAX=plane count (EDI), ECX=planes (ESI), stack (&center vec3, f32 radius) with plain ret (caller pops 8). If ECX == 0 the function replaces both with the per-thread
state's list (`state+0x15DC`, count `state+0x1684`) where state = `gfx_get_thread_state()` (called unconditionally, EAX result; ECX/EDX may be clobbered by it). Planes are 0x1C bytes: normal at plane+4/+8/+0xC, d at plane+0x10.
Per plane: dot = (n.x*c.x + n.y*c.y) + c.z*n.z in x87, stored as f32, then + d stored as f32; `fcomp radius`, `test ah,0x41; je` -> return AL=1 when dist > radius (ordered greater; NaN does not exit). All planes pass -> AL=0.
EBX/ESI/EDI preserved, x87 empty on exit. 199 samples.

**9. vehicle_is_helicopter (0xAB5190).** EAX=vehicle or 0, no stack args; AL=1 iff vehicle non-null and `[[veh+0x84E4]+0x9C] == 3`. ECX clobbered, EAX is the result, no callees, no writes. Sibling `vehicle_class_is_1` (0xAB51B0) compares with 1.

**10. vec3_length_sq_eax (0x40EA60).** EAX=&v, plain ret, result ST0. Order (x*x + y*y) + z*z: loads y, x, z, forms x*x, y*y, adds them, then adds z*z; one `fstp [esp]; fld [esp]` through a pushed ECX slot (so the value is f32-rounded once). ECX preserved (push/pop), nothing else touched; no callees.

**11. mat33_copy_regs (0xBE2C60).** ECX=&src, EAX=&dst, no stack args; copies 9 floats one by one with `fld dword / fstp dword` (bit-identical except that a signalling NaN would become quiet; a plain dword copy is fine for all game data). Nothing clobbered, x87 empty.

**12. vec3_dist_xz_regs (0xBDEAB0).** Same inputs as #6 (ECX=&a, EAX=&b); result ST0 = `sqrtf_wrapper(f32(dx*dx+dz*dz))`. The sum is stored as f32 into the argument slot at `[esp]` before the call (`sub esp,0xC; push ecx` provides the 16 bytes; `add esp,0x10` then plain ret).
Callee: `sqrtf_wrapper` (0x40EA20, f32 at [esp], result ST0). Clobbers whatever the callee clobbers; ECX is pushed/popped by the frame, not restored from the callee.

**13. rng_chance_f32 (0x40B8D0).** One stack f32 p, plain ret; EAX = 1 or 0. Calls `rng_float01` with EAX=0xEC30F8 (the global generator; already a Rust port, so the Rust version can call it directly) which returns r in ST0, then `fld p; fcompp; fnstsw ax; test ah,1; jne zero`.
Polarity: C0 set when ST0 (p) < ST1 (r) -> returns 0; therefore result = 1 iff NOT (p < r), i.e. r <= p, and also 0 when unordered (NaN has C0=1). Writes only the generator state. EAX/ECX/EDX clobbered. Equivalent to `rng_float01() <= p` for non-NaN p.

**14. f32_round_to_int (0xBF0EA0).** One stack f32 x, plain ret, result EAX; tail-jumps to `_ftol2_sse` (ST0 -> EAX, truncating). `movss xmm0,x; fld x; comiss xmm0,[0xDD32C0]` (0.0f), `jb sub` (CF set: x < 0 or unordered): ST0 = x - 0.5 (double 0.5 at 0xE5BF70), else x + 0.5; then truncate toward zero.
So result = (x >= 0) ? trunc(x + 0.5) : trunc(x - 0.5); NaN takes the subtract branch and ends as 0x80000000 (cvttsd2si indefinite), as do overflows. Clobbers EAX, EDX, XMM0.

**15. hkvector4_dot4_store (0x729A60).** ECX=&a, stack (&out at [ebp+8], &b at [ebp+0xC]), `ret 8`, aligned frame. t = a*b lanewise (mulps), u = t + shufps(t,t,0x4E) (adds lane i with lane i^2), out = u + shufps(u,u,0xB1) (adds adjacent lanes) stored to all four lanes of out.
Per lane the result is (t0+t2) + (t1+t3), keep that order in f32. EAX, EDX, XMM0, XMM1 clobbered.

**16. hkvector4_less_store (0x72A100).** See #3: out = (a < b) mask per lane (`cmpltps xmm1=a, xmm0=b`), ECX=&a, stack (&out, &b), `ret 8`.

**17. hkvector4_madd_splat_x (0x71A0A0).** ECX=&out, stack (&s at [ebp+8], &v at [ebp+0xC]), `ret 8`, EAX = ECX (out) on exit. out = out + splat(s.x) * v (shufps 0, mulps, addps); aligned pointers; EDX, XMM0, XMM1 clobbered.

**18. hkvector4_add_in_place (0x485D30).** ECX=&a, stack &b at [ebp+8], `ret 4`, EAX = ECX on exit. a = a + b (addps). XMM0/XMM1 clobbered.

**19. hkvector4_and_mask_table (0xBB8970).** ECX=&v, stack index at [ebp+8], `ret 4`. v = v & mask where the mask is the 16-byte row `0xE538F0 + index*16` (andps). EAX clobbered. Read the table from the image or keep a copy of its rows.

**20. hkrotation_from_quaternion_store (0xD93F20).** ECX=&out (three 16-byte rows at +0, +0x10, +0x20), one stack arg &quat at [ebp+8], `ret 4`, aligned frame (`and esp,-16; sub esp,16`). Pure SSE: loads q, doubles it (addps), builds the products with shufps masks 0x41, 0xA5, 0xFE, 0x1A, 0xB1, 0x8D, applies sign masks from 0xE57460 / 0xE57470 and the constant row 0xDC9C20 (1.0 in lane 0),
and stores three rows. No callees, no globals written, XMM0 to XMM7 clobbered. Port it as an intrinsic-for-intrinsic translation (core::arch::x86 `_mm_*`) so the lane order and rounding match; read the three constant rows from the image at start. Most callers are physics_body_set_transform and shape creation (37), 57 samples.

Skipped on purpose (hot but not tractable yet): 0x4171C0 (damage_record_init, 48 instructions of plain copies, fine to port later but with 125 callers needs a record layout check), 0xD2B7D0 / 0xD2B850 (the two hkThreadMemory fast paths: leaf-like but they touch a live free list shared with Havok's own C++ code, port only together with `hkThreadMemory::vf*`),
0xBE2D90 `mat33_from_euler_angles` (calls the cosf / sinf wrappers three times each).

## 5. Open items

* Merge `docs/candidates_doc60.csv` into `docs/functions.csv` (adds 188 rows to the 14,357 existing).
* The six hottest unnamed callees listed in section 1 are the best next naming target; each needs a half-day style read, not a skim.
* `action_node_from_handle` and `damage_record_init` have the broadest reach (20 and 125 callers): confirming their roles would rename a large set of anonymous AI helpers.
