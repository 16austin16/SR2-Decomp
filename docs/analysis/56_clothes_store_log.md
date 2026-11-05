# 56: Clothes-store mission log (2026-11-04b), shop code, appearance helpers, lifted-loader crash

Input: `bin/sr2-mission-2026-11-04b.log` (29,760 samples: 12,068 game thread, 17,692 render thread). Report made with
`python tools/mission_report.py bin/sr2-mission-2026-11-04b.log --top N`; the address-to-function mapping below was redone with a
call-target sweep (about 20,000 starts, scripts in the session scratchpad `storelog`), because `mission_report.py` credits each
sample to the nearest NAMED function below it, and half of the hot functions are still unnamed.

## 1. Result in one paragraph: this log does not contain a clothes-store visit

The log holds 5 logging sessions and 8 sections. Labels present: `Test123@%$` (a label-box test) and `Pre mission start` (twice).
No label mentions a store, and the code that a purchase must run is absent:

| Section | Frames | Game | Label | Samples | Shop block | pcu/pcr/customisation | Verdict |
|---|---|---|---|---|---|---|---|
| s1 (session 1, also `-test.log`) | 2321-2831 | none | - | 1376 | 1 | 0 | free roam |
| s1 (session 2) | 1699-1989 | none | - | 896 | 0 | 1 | free roam |
| s2 (session 2) | 1989-2434 | none | `Test123@%$` | 1504 | 3 | 1 | label test, free roam |
| s1 (session 3) | 3333-3665 | none | - | 1088 | 0 | 1 | free roam |
| s2 (session 3) | 3665-3781 | none | `Pre mission start` | 448 | 0 | 1 | walking to a mission |
| s1 (session 4) | 2965-3403 | none | - | 1024 | 0 | 1 | free roam |
| s2 (session 4) | 3403-3481 | none | `Pre mission start` | 224 | 1 | 0 | walking to a mission |
| s3 (session 4) | 3481-10736 | tss02 | - | 23200 | 47 | 46 | mission tss02, unlabelled, 7255 frames |

("Shop block" = game-thread samples in 0x9F2830-0x9F7100; "pcu/pcr/customisation" = 0x7B0000-0x7E0000, 0x7BD000-0x7C6000 and
0x9E4000-0x9EA400.) A real store session would show `luafn_pcu_*` (0x7C87B0-0x7CD3F0), `ovl_customize_remote::*`,
`pcu_worn_*` and `pcr_*` with tens or hundreds of samples, plus the loaders `pcu_slot_cache_refresh`, `pcu_category_set_active`,
`pcu_areas_build`. Zero samples hit any `luafn_pcu_*`, `ovl_customize_remote`, `luafn_pcr_*`, `pcu_slot_cache_refresh`,
`pcu_category_set_active`, `pcu_areas_build`, `pcu_outfit_purchase_charge`. The only pcu hits are 2 samples in
`pcu_worn_resolve_conflicts` (0x7BFC93, 0x7BFD3F) and 2 in `pcu_item_find_by_name` (0x7C5627, 0x7C563D) in s3, plus 1 each in
`pcu_lua_register`/`pcr_morph_handshake`; these are the small cost of a cutscene or mission outfit change, not a store. The
section-3 mission (tss02) is the only long run and it has no point labels, so the F8 points that would split a store visit out of
it do not exist. Likely causes: the store sessions were played in a run whose log was not saved (the file is overwritten per
launch: session 1 equals the older `sr2-mission-2026-11-04-test.log`), or the user bought clothes in a build that ran with
logging off. Please re-record: press F8 with a label at each phase (enter, browse, preview, buy, leave).

What the log does show that touches the store domain:

- **Shop proximity code is live every frame** (`shops_do_frame` 0x9F5420 -> 0x9F4150 -> 0x9F3F00): 30 + 8 + 10 game-thread samples,
  47 in s3. That is the "entering" detector for every shop, including clothes stores (section 3).
- **Character appearance** (rendering of humans): morph meshes 277 render samples and 12 game samples in s3 (0xCF6000-0xCFC000:
  `morph_target_apply`, `rl_morph_mesh_inst::render/pre_render`), cloth bones 17 game samples (0x6D9610-0x6DA350), model-part
  visibility `0x9E5DB0` 13 samples, `character_table` lookup `0x6E12A0` 15 samples.
- **HUD/menu (vint)**: 419 game samples in s3 in 0xB80000-0xBAA000; mostly `vint_object_tree_update` (95, of which 75 sit on one
  instruction, 0xB8AC52, the return of the per-object vtable slot-3 call), `vint_tween_*`, `vint_draw_bitmap*`. Normal mission HUD.

## 2. Hot functions of the whole log (all sections)

Top game-thread functions by true function start (this sweep). Status: N = named, U = unnamed in functions.csv, R = already Rust.
`mission_report.py` shows these under the nearest named function instead (right-hand column).

| Samples | Start | Name | Status | Report shows it as |
|---|---|---|---|---|
| 400 | 0xD81050 | (Havok mopp/shape walker) | U | hkpMoppBvTreeShape::dtor+0x1320 |
| 350 | 0xC81C20 | (Havok mopp ray VM helper) | U | hkpMoppLongRayVirtualMachine_func+0x1BE60 |
| 329 | 0xC0C990 | profile_stop | N | |
| 234 | 0x520790 | game_main_thread_proc | N | |
| 210 | 0xC0BE10 | profile_start | N | |
| 196 | 0x406810 | (ai human accumulator helper) | U | ai_human_accumulator_mean+0x60 |
| 191 | 0x50F190 | effect_list_reclaim_dead | N | |
| 181 | 0xCE9E10 | (animation track update) | U | anim_tracks_update_frame+0x28F0 |
| 179 | 0x926170 | coop_do_frame | N | |
| 177 | 0xA020C0 | shape_bounds_overlap_test | N | |
| 95 | 0xB8AC40 | vint_object_tree_update | N | |
| 68 | **0x91E040** | chunk-object attach transform (named below) | U | human_inventory_restore+0x300 |
| 60 | 0x4F4E60 | (small 0x8B-byte string/stream helper, 6 callers) | U | |
| 45 | 0x557870 | (SSE vec4 helper, 99 callers, ret 8) | U | menu_widget::vf3+0x1A0 |
| 30 | **0x9F3F00** | shop_find_nearest_for_human (named below) | U | store_chains_parse+0x1310 (a Rust function) |

Render thread: `__VEC_memcpy_fastcopy` 7768 of 17692 (44 %), `frustum_test_aabb` 601, unnamed 0x4B5CE0 (city instance add) 528,
`_memcpy` 406, `r_particle_inst::pre_render` 281, `mesh_render_generic` 277. No store-specific render code.

## 3. New names (15 rows, all in `docs/candidates_storelog.csv`)

All addresses verified as real starts: preceded by `int3` padding (or `ret` + padding), reached by `call rel32`, not in
`functions.csv`; all names checked unused. Evidence level: C = confirmed from the instructions, I = inferred.

### Shop zones (`shops` block 0x9F3B00-0x9F4600, 0x9F6E80-0x9F700A)

Shop record (0xA0 bytes, array `[0x21F2AA8]`, count `[0x21F2AA4]`, global `[0x21603CC]` is read by the update):
`+0x00` name hash (see `0x9F5560` lookup), `+0x04` zone object (`[+0x10]` trigger, `[trigger+0x18]+0x1C` = door position x,y,z),
`+0x08` pointer to pointer (trigger), `+0x0C` trigger, `+0x14` trigger set, `+0x24` byte flags (bit 2 active, bit 3 has +8, bit 4 has +0xC),
`+0x25` byte (bit 0 player inside, bit 2 skip), `+0x26` byte (bit 0, bit 2 enabled, bit 7 excluded), `+0x28` audio record
(`+0` sound id, `+4` voice count/-1, `+0x38` voice handles stride 0xC, `+0x68` count), `+0x2C` audio slot (>= 0 means audio exists),
`+0x94` timer, `+0x9C` byte. Globals: `[0x21703D4]` player human, `[0x217043C]` ring head of humans (next at human `+0x1294`),
`[0x25297EC]` player's current shop, `[0x25297F0]` the other (remote / partner) current shop, `[player+0x13BC]` the player's current shop,
`[0x2528C14]` network session, bytes `0x2528B56` / `0x2528B2B`, session byte `+0x3F8`. Float constants: `[0xE5CAD4]` radius squared
(shop-range), `[0xDD0778]` capture threshold squared (K), `[0xE8725C]` initial "best" distance, double `[0xE5C130]` hysteresis margin.

| Address | Name | Conv. | Role | Evidence |
|---|---|---|---|---|
| 0x9F3BB0 | `shop_human_leave` | EAX = human, ESI = shop; plain `ret`; EBX saved | clears the current-shop global of that human (`[0x25297EC]` if human is the player, else `[0x25297F0]`), for the player releases the net reference (`rsched_release_net(5,1,..)`), clears shop bytes `+0x25` and `+0x9C` | C (instructions); name I |
| 0x9F3C00 | `shop_human_enter` | cdecl (human), ESI = shop; EBX, EDI saved | sets `[0x25297EC]` (player, plus `rsched_add_ref_net`) or `[0x25297F0]`; then, if a net session allows it and the shop trigger object (vtable +0x2C false), calls vtable +0x18 / +0x34 and registers (0x912370, `net_object_register_local(5)`) | C / I |
| 0x9F3D00 | `shop_stop_audio_if_player_far` | EAX = shop; ESI, EDI saved | squared distance player -> door (min with `coop_partner_human` position via `vec3_distance_sq`); if it is greater than `[0xE5CAD4]` calls `shop_ambient_audio_stop(shop+0x28)` | C |
| 0x9F3DB0 | `shop_release_if_no_human_near` | cdecl (shop); EBX, EBP, ESI, EDI saved | no-op unless net rule passes; returns if shop is a current shop; if any human of the ring `[0x217043C]` is within radius (d2 < `[0xE5CAD4]`) returns; else tail-calls trigger vtable +0x1C | C / I |
| 0x9F3F00 | `shop_find_nearest_for_human` | cdecl (human); EBX, EBP, ESI, EDI saved; no result | per-human proximity pass: picks the nearest active shop within K, leaves the old one and enters the new one | C (instructions) |
| 0x9F4150 | `shops_update_proximity` | no args; ESI, EDI, ECX saved | loops shops (`[shop+0x2C] >= 0` signed: `shop_stop_audio_if_player_far`), clears `[0x25297F0]` when `[0x21603CC] == 1`, then `shop_find_nearest_for_human` for every human of the ring | C |
| 0x9F41D0 | `shop_triggers_update` | EAX = shop; EBX, ESI, EDI saved | computes "blocked" (BL), then activates or deactivates the shop's triggers (`trigger_activate` ESI = trigger / `trigger_deactivate` EAX = trigger) | C / I |
| 0x9F4480 | `shop_current_event_tick` | no args; ESI, EDI saved | every 1000 ms while the player is in an active shop with `+0x25` bit 0, posts event `{0x11, 5, time}` into the queue `0x279A808` (0x14 bytes each, count `[0x1409B0C]` < 0x1E) | I |
| 0x9F6E80 | `shop_ambient_audio_start` | stack (audio record = shop+0x28), ret 4 | when the record's sound slot is valid and the net/state flags allow, starts the ambience voices (`audio_play_params_init_3d` + `audio_play` per slot, table 0xEEB8F8 stride 0x19C); otherwise calls `shop_ambient_audio_stop` | C |
| 0x9F6FC0 | `shop_ambient_audio_stop` | stack (audio record), ret 4; EBX, ESI, EDI saved | sets `[rec+4] = -1`, `audio_voice_stop` on each of `[rec+0x68]` voice handles (stride 0xC from +0x38), zeroes the count, then 0x9F4AB0(`[rec]`, 0) | C |

### Appearance, model and customisation helpers

| Address | Name | Conv. | Role | Evidence |
|---|---|---|---|---|
| 0x9E5DB0 | `pcu_worn_apply_part_visibility` | ECX = customisation object (circular list head `[ecx]`, next `+0x1C`, human `[ecx+0x18]`); plain `ret`; EBX/ESI/EDI saved, frame cookie | memsets a 0x100-byte flag table, marks every slot hidden by a worn item (item `+0x3C` bit 0, slot list `+0x28`, count word `+0x1C`) and every worn slot (`item+0x15`), then toggles each model part of the human (`[item+0x2C]` part list; `0x6E7D50` when the slot flag is set, `0x6E7CC0` otherwise) and re-applies the saved part list from `[human+0x268]` | C for the structure, I for the show/hide naming; 13 samples |
| 0x9EA2C0 | `pcu_worn_list_import_from_net_packet` | cdecl (out_object, packet) ; cookie frame | reads a count byte from the packet, builds that many 0x40-byte worn records with `0x81D690`, calls `0x7C3750(object, records, n)` (worn import), returns AL = 1 | I |
| 0x6E6070 | `transform_table_get_entry` | EAX = index, EDX = out vec3*, stack (table*, out mat33* or 0), plain `ret` | entry stride 0x60 at `[table+0x10]`, count word `[table+0xE]`; copies position (+0x44) and, if the matrix pointer is non-null, `quat_to_mat33` of the quaternion at +0x50 (ECX = out matrix, EAX = quaternion); AL = 1; on bad index copies identity (0xEC5AE4, 9 dwords) and AL = 0 | C; used by cutscene code |
| 0x6E12A0 | `character_table_find_index_by_hash` | EDX = pointer to a u32 name hash, EAX = result (index or -1); clobbers ECX | walks the circular list `[0x145A4FC]` (next at +0, key at +8, value at +0xC), then the list `[0x2527D4C]` (key at +0xC, result +0x10); -1 if absent or value equals `[0x33D630C]` | C; 15 samples |
| 0x91E040 | `chunk_object_attach_world_transform` | EDX = rig/part table (reads `[edx+0x4C]`), stack (object*), plain `ret`; EBX, ESI, EDI saved | composes the attachment transform: bone index `[[obj+0xC]+0x84]` (>= 0), offset and rotation of the part, `vec3_mul_mat33_in_place` x2 and `mat33_mul`, writes position to `[obj+0x54..0x5C]`, scale `+0x60..0x68`, matrix `+0x6C` (9 dwords); bone < 0 copies the unmodified values | C structure, I role; sole caller 0x91EA50; 68 samples (stall right after the pointer load, 0x91E0D7 and 0x91E0FE) |

Not named (no certain role): 0x6D9E40 (EAX = human, ret 8, two out vec3, rig slot table 0x340D240 stride 0x5E8; 17 samples,
likely cloth / hair motion), 0x9A5AD0 (cutscene character placement, calls `pcu_worn_entry_copy_by_slot`; 17 samples),
0x4F4E60, 0x557870 (not store related).

## 4. Port recipes (top 10; hottest store / appearance functions in this log)

Pure CPU functions first. Notation: `[x]` = dword at address x; "polarity" lists every compare as the original executes it.
x87 compare idiom: `fcom`/`fcomp` then `fnstsw ax; test ah, mask; jcc`: mask 0x41 tested with `jne` = "<=" (C0 or C3), with `je` = ">",
mask 5 with `jnp` = "<" (C0 only), with `jp` = ">=" (C0 clear).

1. `0x9F3F00 shop_find_nearest_for_human(human)`, cdecl, caller pops 4. No result; EBX/EBP/ESI/EDI restored; EAX/ECX/EDX clobbered; x87 stack empty on exit.
   - Gate: `s = [0x2528C14]`. If `s == 0` -> position stage. Else if `[0x2528B56] == 0`: if byte `[s+0x3F8] != 0` -> position stage (skip the "player only" rule), else rule; if `[0x2528B56] != 0`: if `[0x2528B2B] == 0` -> rule, else same as before (`[s+0x3F8]` test). Rule: return if `human == 0` or `human != [0x21703D4]`.
   - Position `p`: human == player -> `[0x25F5B14],[0x25F5B18],[0x25F5B1C]`, else human `+0x12A0,+0x12A4,+0x12A8`. `best = [0xE8725C]` (float), `nearest = 0`, `cur = (human == 0 || human == player) ? [0x25297EC] : [0x25297F0]` (code order: `edi=[0x25297EC]`; if human is 0 or not the player `edi=[0x25297F0]`; the "or 0" is `cmp ebp, ebx; je` giving 0x25297F0).
   - Loop i in 0..`[0x21F2AA4]`, shop = `[0x21F2AA8] + 0xA0*i`: skip unless byte `shop+0x24 & 4`. `q = [[[shop+4]+0x10]+0x18] + 0x1C` (x,y,z). `d2 = (px-qx)^2 + (py-qy)^2 + (pz-qz)^2` computed in float with a store/reload after each step (fstp/fld of 32-bit temps, so rounding is to float at every step).
   - If `shop == cur`: if `d2 <= [0xE5CAD4]` (fcom, test 0x41, `jne` taken -> sqrt path) then `d = sqrtf(d2) - (double)[0xE5C130]` (the `fsub qword`; `sqrtf_wrapper` is cdecl with the float on the stack, result in ST0), `d = max(d, 0.0)` (`fldz; fcom d; test ah,5; jnp keep` : keeps d when 0 < d), `d2 = d*d` (stored as float) and continue to the scoring. Else (`d2 > [0xE5CAD4]`): call `shop_human_leave` (EAX = human, ESI = shop), set `cur = 0`, next shop (the scoring is skipped).
   - Scoring (all shops that reach it, including non-current ones): call `shop_release_if_no_human_near(shop)` first (cdecl, 1 arg); then `if d2 <= [0xDD0778]` (je taken when `>`: skip) and `(byte shop+0x26 & 0x80) == 0` and `best > d2` (fcomp best vs d2, `jne` taken when best <= d2: skip) -> `best = d2; nearest = shop`.
   - After the loop: if `nearest != 0 && nearest != cur`: if `cur != 0` call `shop_human_leave` (EAX = human, ESI = cur); if `human != 0 && human == player` set `[0x25297EC] = nearest`, push human, ESI = nearest, `shop_human_enter`; else set `[0x25297F0] = nearest` and enter (the non-player branch also runs when human is null).
   - Writes: `[0x25297EC]` / `[0x25297F0]` directly; everything else via the callees. Pitfall: the callees take ESI / EAX in registers, so a Rust port must pass them by `call_regs`.
2. `0x9F4150 shops_update_proximity()`: no args, no result; ESI/EDI preserved, ECX pushed. For i in shops: if signed dword `[shop+0x2C] >= 0` (jl skips) call `shop_stop_audio_if_player_far` with EAX = shop. Then `if [0x25297F0] != 0 && [0x21603CC] == 1 { [0x25297F0] = 0 }`. Then `h = [0x217043C]`; while `h != 0`: `shop_find_nearest_for_human(h)`; `h = [h+0x1294]`; stop when `h == [0x217043C]` (ring) or 0. Callee addresses 0x9F3D00, 0x9F3F00.
3. `0x9F41D0 shop_triggers_update(EAX = shop)`: ret plain; EBX/ESI/EDI preserved; stack frame 8 bytes (`[esp+0x13]` = `[0x27B3C60] != 0`).
   - `blocked` (BL) = 1 when ANY of: `shop+0x26 & 4 == 0`; (`[0x2527D14] != 0 && [0x2527CF7] != 0`); `[0x2527D15] != 0`; `(signed char)shop[0x26] < 0`; `ui_blocks_player() != 0` (0x9F6240, AL); or, if `[shop+4] != 0 && z = [[shop+4]+0x38] != 0`, `0x6105D0(vec3 z+0x14..0x1C by value)` returns AL != 0. If `[shop+4] == 0` or `z == 0` blocked = 0.
   - If `shop[0x24] & 4`: `h = [[shop+4]+0x10]->vtable[0x34]()` ; `active = !blocked && !human_is_dead(h) && shop[0x9C] == 0 && (shop[0x25] & 4) == 0 && byte [h+0x149] != 0`... (code: `cmp [esi+0x149], al` with AL=0 from the previous `test al,al`; `je 42f7` means activate path when `[h+0x149] == 0`). So activate path when `blocked == 0 && !dead && [shop+0x9C]==0 && !(shop[0x25]&4) && byte[h+0x149]==0`, else deactivate path. If `shop[0x24] & 4 == 0`: activate path iff `blocked == 0`.
   - Deactivate path: `trigger_deactivate(EAX=trigger)` for: `[[shop+4]+0x38]` (if `shop[0x24]&4`), `[[shop+8]]` (if `&8`), then if `[shop+0x14] != 0`: `[[shop+0x14]+0x10]`, `[+8]`, `[+0xC]` (each only when non-null), then tail-call `trigger_deactivate([shop+0xC])` when non-null.
   - Activate path (first `ui_blocks_player()` must be 0 else deactivate): same trigger set with `trigger_activate(ESI=trigger)`, then if `shop[0x24] & 0x10` and `t = [shop+0xC] != 0`: skip if `shop[0x26] & 1`, or `0x4B1720(EAX=[shop+0x18]) != 0`, or `[esp+0x13] != 0`, or `ui_blocks_player() != 0`; else `trigger_activate(ESI = t)`.
4. `0x9F3D00 shop_stop_audio_if_player_far(EAX = shop)`: ESI/EDI preserved. `P = [0x21703D4]`; `q = [[[shop+4]+0x10]+0x18] + 0x1C`; `d2 = (P.x - qx)^2 ...` using P `+0x14,+0x18,+0x1C`, floats rounded per step. `c = coop_partner_human()` (0x9D7BD0, EAX); if `c != 0`: `e = vec3_distance_sq(EAX = q, ECX = c+0x14)` (result ST0) and `if e <= d2` (`fcom` then `je` taken when `e > d2`: keep own) `d2 = e`. If `[0xE5CAD4] < d2` (fcomp, `test ah,5`, `jp` not taken when C0 set) call `shop_ambient_audio_stop(shop+0x28)` (stack arg, callee pops). Equal -> no stop.
5. `0x9F3DB0 shop_release_if_no_human_near(shop)`: cdecl. Early exit when the net rule fails: `s = [0x2528C14]`; `s != 0` and `([0x2528B56] != 0 ? [0x2528B2B] == 0 : byte [s+0x3F8] == 0)` -> return. `obj = [[shop+4]+0x10]->vtable[0x34]()` (EAX); return if `obj == 0` or `shop == [0x25297EC]` or `shop == [0x25297F0]`. `h = [0x217043C]`; loop humans: `p` = player -> `[0x25F5B14..1C]`, else human `+0x12A0..A8`; `d2 = sum((p - obj[+0x14,+0x18,+0x1C])^2)`; if `d2 < [0xE5CAD4]` (fcomp, `test ah,5`, `jnp` taken when C0: strictly less) return; `h = [h+0x1294]` until `h == head` or 0. If no human is near: `[[shop+4]+0x10]->vtable[0x1C]()` with ECX = that trigger object (tail jmp; the shop arg stays on the stack, caller pops).
6. `0x91E040 chunk_object_attach_world_transform(EDX = table, stack object)`: plain `ret`, frame 0x90, EBX/ESI/EDI saved. `o = [obj+0xC]`, `b = [o+0x84]` (signed). Polarity: `cmp b, -1; jle` -> copy-through path when `b <= -1`. Else: locals `v = o+0x3C..0x44`, `r = o+0x30..0x38`, 9 floats `o+0x48..0x68` (three 3-float rows; read with `fld/fstp` pairs). `rec = [edx+0x4C] + b*0x50`; `m = [rec+0xC]`: if `m != 0` the bone position is `m+0x14..0x1C` and the bone matrix is `m+0x20` (9 dwords), else `q = [rec+0x40]`, position `q+0..8`, matrix `q+0xC` (rep movsd, ECX = 9). Then `vec3_mul_mat33_in_place(ECX = &v, EAX = &bone_matrix)`, `v += position` (three float adds with float rounding), `vec3_mul_mat33_in_place(ECX = &r, ...)`, `mat33_mul(ECX = ..., EAX = ..., stack = ...)` whose returned pointer (EAX) is copied by `rep movsd` x9 to `obj+0x6C`. Writes `obj+0x54,+0x58,+0x5C` (v), `+0x60,+0x64,+0x68` (r), `+0x6C..+0x8C`. Copy-through path: `[o+0x3C..0x44] -> [obj+0x54..0x5C]`, `[o+0x30..0x38] -> [obj+0x60..0x68]`, 9 dwords `o+0x48 -> obj+0x6C`. Check the exact `mat33_mul` operands against the disassembly 0x91E17F-0x91E19A (`push eax; lea ecx,[esp+0x58]; lea eax,[esp+0x7C]`) before porting. 68 samples, a cache-miss stall on the pointer loads (0x91E0D7, 0x91E0FE).
7. `0x6E12A0 character_table_find_index_by_hash(EDX = &hash)`: no stack args, plain `ret`; EAX = result, clobbers ECX, EDX. `h = [edx]`; `n = [0x145A4FC]`; loop (circular, stop at head or null): `if [n+8] == h`: `v = [n+0xC]`; `if v == [0x33D630C]` return -1; then second list `m = [0x2527D4C]`: loop `if [m+0xC] == v` return `[m+0x10]`; next = `[m]`, stop at head/null -> -1. First list exhausted -> -1 (OR EAX, -1).
8. `0x6E6070 transform_table_get_entry(EAX = idx, EDX = out_pos, stack table, stack out_mat)`: ESI/EDI saved, EBX saved around the copy. Fail (AL = 0) when `table == 0` (before touching anything else, leaving EDX untouched), or `idx < 0` (signed `jl`), or `idx >= (int16)[table+0xE]` (signed `jge`); on failure with table non-null: 9 dwords from 0xEC5AE4 into `out_mat` (rep movsd with `ecx = 9, esi = 0xEC5AE4, edi = out_mat`; note it copies even when `out_mat == 0`, which would fault, so callers always pass it, except the success path which tests it) and AL = 0. Success: `rec = [table+0x10] + idx*0x60`, `[edx..edx+8] = rec[0x44..0x4C]` (3 dwords), if `out_mat != 0`: EAX = `rec+0x50`, ECX = out_mat, `quat_to_mat33` (ECX/EAX survive, output via ECX); AL = 1.
9. `0x9E5DB0 pcu_worn_apply_part_visibility(ECX = object)`: cookie frame, ret plain. Two 0x100-byte zeroed tables `A` (hidden-slot flags) and `B` (worn-slot flags). Pass 1 over the circular worn list (`n = [obj]`, next `[n+0x1C]`, stop at head/null): if `[n+8] != 0 && ([[n+8]+0x3C] & 1) && byte [n+4] == 0` then for each of `(int16)[[n]+0x1C]` bytes `s` at `[[n]+0x28]`: `A[s] = 1`. Pass 2 same list: `if byte [n+4] == 0: B[(signed char)[[n]+0x15]] = 1`. Pass 3: for each node with `[n+8] != 0 && ([[n+8]+0x3C] & 1)`: `list = [[n]+0x2C]`, `cnt = [list+4]` (signed `jle` exits), for i < cnt: `part = [[list][i*4]]`; ECX... `edi = [n+8]->[+0x2C]`, `if A[part] != 0` -> `0x6E7D50(part)` else `0x6E7CC0(part)` (both take EDI = parts object and one stack arg). Then `h = [obj+0x18]`; if byte `[h+0x26C] == 1`: look up the part list by `string_hash_lower(..)` + `0x6E3CD0` (EDX = hash, ESI = &count), search 0x2C-stride records for the matching hash (`cmp [rec+4], ebx`), `0x6E49A0([h+0x268])`, alloca count*4, memcpy the saved part-id array and call `0x6E7D50` for each id with `A[id] != 0`. Needs the callee register contracts (EDI) preserved.
10. `0x9F4480 shop_current_event_tick()`: ESI, EDI saved. `p = [0x21703D4]`; `s = [p+0x13BC]`; return unless `s != 0 && (s[0x24] & 4) && (s[0x25] & 1) && timer_elapsed(EAX = s+0x94) != 0` (AL). Sets `[0x1409EA0] = [0x2797410]`, `[0x1409EA8] = 0`, `[0x1409EAC] = 3`, byte `[0x1409EB0] = 0`. If `human_owned_locally(EAX = p)` and not `human_is_dead(p)` and not (`[0x2528B48] == 1 && [0x2528B49] == 0`) and `[0x2526D9C] == 0`: `k = [0x1409B0C]`, `full = k >= 0x1E` (signed `setge`); slot = `full ? scratch : 0x279A808 + k*0x14` and `[0x1409B0C] = k+1` when not full; slot: `[0]=0x11`, `[4]=5`, `[8] = t` where `t = [0x1409EA4] + [0xE843AC]` with the wrap `if t > 0x6B49D200 then t += 0x94B62E00` (positive offset) or, for negative offsets, `if t < 0 then t += 0x6B49D200` (sign decided by `[0x1409EA4]` jl); byte `[slot+0x10] = (b & 0xFD) | 1`; `[slot+0xC] = [0x2528C14] ? [[0x2528C14]+8] : [0x212B160]`; when `full` call `0x55EC40` (ECX = p). Finally `[s+0x94] = [0xE843AC] + 0x3E8` wrapped the same way (`> 0x6B49D200` -> `+ 0x94B62E00`). 0 samples in this log; ranked tenth for being the one other shop code run each frame.

Candidate order by this log's sample count: 0x91E040 (68), 0x9F3F00 (30), 0x6E12A0 (15), 0x9E5DB0 (13), 0x9F41D0 (10),
0x6E6070 (10), 0x9F4150 (8), then the zero-sample ones. None of the ten is on the clothes-store purchase path itself; the real
purchase path (`luafn_pcu_purchase_outfit`/`_slot` -> `pcu_outfit_purchase_charge` 0x9DE0C0, `pcu_purchase_slot_apply` 0x7C1260,
`pcu_worn_*`, `pcu_slot_cache_refresh` 0x7CE320) needs a new log.

## 5. Clothes-store crash (lifted loaders, 2026-11-04)

Cause (commit 229da51): `port_tables.rs` replaces 57 loaders with machine-lifted code that does not keep ECX/EDX across calls. Original
code relies on `quat_to_mat33` 0xC057D0 (EAX = quaternion x,y,z,w, ECX = out 3x3 matrix; it only reads EAX and writes through ECX, never
modifies ECX/EAX/EDX, `ret` with 0xC bytes of locals) and `vec3_mul_mat33` 0xBDDE20 (ECX = matrix, stack arg = input vec3, EAX = out vec3,
`ret 4`; reads ECX on the first instruction after the prologue, `fld [ecx+0xC]`).

- **The one lifted body with that exact pair is `tod_light_sets_parse` 0x4C8B80** (in the Rust-held list as `tod_light_sets_parse`):
  `lea ecx,[esp+0xAC]; lea eax,[esp+0x80]; call quat_to_mat33 (0x4C9061); lea eax,[esp+0x74]; push eax; lea eax,[esp+0xA4];
  call vec3_mul_mat33 (0x4C9072)` - ECX set before the first call and read by the second. A lift that clobbers ECX after the first call
  hands the second call a garbage matrix -> crash.
- It runs at runtime, not only at startup: `lightset_find_by_name` 0x4C8B30 (EDI = name; hashes the name, searches the cache at
  `0x2652198` (count `[0x2526D44]`, 0x10 stride) and **tail-jumps to `tod_light_sets_parse`** on a miss). Store-related callers of
  `lightset_find_by_name`: **`store_chains_parse` 0x9F3586** (store interior lightsets; this is the clothes-store path), **`xtbl_load_custom_start_lightsets` 0x7D6A0C** (player creation / `pcr`),
  **`xtbl_load_Vehicle_Customization_Lightset` 0x7B939A and 0x7B9D3E** (vehicle customisation), and 0x554AD9 (garage code, unnamed 0x554A00). `tod_light_sets_parse` is also reached from `cutscene_begin_callback` 0x6C0DF8 (cutscene lights).
- Other lifted loaders (list in `port_tables.rs`) that call these helpers directly in their own body: none by direct sweep of the
  array bodies except `tod_light_sets_parse`; the byte arrays of `xtbl_load_rain` (0x4CD230) cover a following function that contains
  `vec3_mul_mat33_in_place` at 0x4CDA6D (the sweep passes the real end), so treat it as a false positive. Callees the lifts call by
  address but which preserve registers in the original: `quat_to_mat33`, `vec3_mul_mat33` (ECX), `vec3_mul_mat33_in_place` 0xBDE570
  (EAX = matrix, ECX = vec3), `mat33_mul` 0xBE2F50 (ECX/EAX + stack). Callers of those in the shop / appearance code (not lifted, listed because a
  future lift must preserve the same registers): `transform_table_get_entry` 0x6E6070 (`quat_to_mat33`), `chunk_object_attach_world_transform`
  0x91E040 (`vec3_mul_mat33_in_place` x2, `mat33_mul`), `xtbl_load_items_inventory` 0x91CF50 region 0x91E122, `garages_load_table` 0x550D60 (0x5564C0).
- Customisation loaders in the lifted list that the store opens or reads (all run once at start via `0x7BF790` and `0x7D6390`, so they only
  matter if re-entered): `xtbl_load_customization_categories`, `_default_items`, `_stores`, `_outfits`, `customization_logos_load_table`,
  `xtbl_load_player_creation_hair_color`, `xtbl_load_player_regional_presets` (0x7DA630, also re-entered from 0x7DAC61), `store_chains_parse`
  (0x9F3A10 caller; its body contains the `lightset_find_by_name` call), `xtbl_load_store_weapons` (also called from `weapon_store_lua_register` 0x7B3D11, i.e. at store-script registration).
  None of those call `quat_to_mat33` directly.
- Hot-path conclusion: `store_chains_parse` is listed as Rust (status `rust`) but the log shows its runtime proximity code (0x9F3F00 etc.) runs
  in the ORIGINAL code, which is a separate, still-unported set of functions. Only the parser (record reading) is Rust.
- Fix hint: for each lifted `call`, preserve ECX/EDX when the callee is a known register-preserving leaf (list above), or hand-write
  `tod_light_sets_parse` (it has no more than the `sscanf` + two matrix calls in the hot block 0x4C9040-0x4C9097) with `call_regs`.
