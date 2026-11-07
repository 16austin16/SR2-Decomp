# 59: Porting maps for the AI order updates (kinds 0x00, 0x0D) and the navigation core

Scope: the twelve functions `docs/skipped_ports.md` lists as too large to port blind. Everything below was read from `bin/SR2_pc.unpacked.exe` with capstone (no guessing from callers). Block addresses are VAs. "Polarity" always means what the **taken** branch tests, written as the source-level condition. New names are in `docs/candidates_doc59.csv` (not merged).

| Function | Start | End (last `ret`) | Size | Frame | Kind |
|---|---|---|---|---|---|
| `ai_order_k00_update` | 0x448CE0 | 0x449935 | 1,023 insns | ebp frame, `and esp,-8`, locals 0x2C | cdecl (human), void |
| `ai_order_k0d_update` | 0x42D2B0 | 0x42E5DB | 1,512 insns | ebp frame, `and esp,-8`, locals 0x9C | cdecl (human), void |
| `nav_path_search` (A* core) 0xA121B0 | 0xA121B0 | 0xA12AA4 | 764 | esp frame 0xB8 | EAX + 1 stack arg, void |
| `nav_cells_heuristic` (polygon cut / estimate) 0xA11750 | 0xA11750 | 0xA11C0D | 393 | esp frame 0x60 | 5 stack args, ST0 |
| `nav_poly_link_match` (job stage) 0xA16650 | 0xA16650 | 0xA17443 | 1,176 | esp frame 0x5C | 2 stack args, void |
| `nav_cells_reachable` 0xA14230 | 0xA14230 | 0xA144AB | 191 | esp frame 0x40 | 3 stack args, AL |
| `nav_human_path_resume` 0xA14560 | 0xA14560 | 0xA147C4 | 179 | esp frame 0x94 | 2 stack args, AL |
| `nav_path_prepend_start` 0xA147E0 | 0xA147E0 | 0xA14ADA | 245 | esp frame 0x88 | 5 stack args, void |
| `road_query_random_lane_setup` 0xA8A4F0 | 0xA8A4F0 | 0xA8A5CD | 48 | none | EAX, EDX + 6 stack args, void |
| `vehicle_ai_pathfind_to` 0xB27830 | 0xB27830 | 0xB27D85 | 392 | esp frame 0x578 + /GS cookie | ECX + 7 stack args, EAX |
| `vehicle_ai_pathfind_nodes` 0xB27D90 | 0xB27D90 | 0xB2832D | 412 | ebp frame, 0x594 + /GS | ECX + 9 stack args, EAX |
| `vehicle_ai_pathfind_navmesh` 0xB28700 | 0xB28700 | 0xB28ACF | 301 | ebp frame, 0x574 + /GS | ECX + 6 stack args, EAX |

All 12 end at an `int3` pad (checked); the two AI ones are followed immediately by their jump tables (k00: 0x449938..0x4499CF, k0d: 0x42E5DC..0x42E667). None of the nav functions contains a jump table or an indirect call, **except** k0d (one `call edx` virtual call at 0x42DD16 and four tables) and k00 (three tables).

## 0. Conventions used by `port_ai.rs` / `port_nav.rs`, and a decoding cheat sheet

- AI ports: a naked thunk (`regs_thunk!` / `pushad`) calls an `extern "C"` body with the `Pushad` block and a pointer to the stack args; callee calls go through `K { c: Cx{ecx,edx}, esi, edi }.call(id, eax, &[stack args])` (cdecl, caller pops) so ESI/EDI/ECX/EDX keep their original-code values between calls. `br!(DP_x, n)` counts branch coverage; `dc_port` runs the first 3,000 calls against the relocated original. `frame = [0u32; N]` + `black_box` is the idiom for original stack locals whose address is passed to callees (see `k0b_up_body`). `fs(a,b,op)` is the x87-at-53-bit helper (op on f64, round to f32).
- x87 branch polarity. After `fcom/fcomp/fcompp X; fnstsw ax; test ah,M; J`, with ST0 = the left value and X the right:
  - `test ah,0x41; jne` taken when `ST0 <= X` or unordered; `je` taken when `ST0 > X`.
  - `test ah,0x01; jne` taken when `ST0 < X` (or unordered); `je` when `ST0 >= X`.
  - `test ah,0x05; jp` taken when `ST0 >= X` (ordered); `jnp` taken when `ST0 < X` **or unordered**. (the `if (a >= b)` idiom: `jp` skips the body when false.)
  - `test ah,0x44; jp` taken when `ST0 != X` (or unordered); `jnp` taken when equal.
  - `test ah,0x41; jp` taken when `ST0 > X` or unordered; `jnp` taken when `ST0 <= X` (ordered). `test ah,0x41; jne` / `je` are the opposite pair listed first.
  - `fcompp` compares ST0 with ST1 and pops both; ST0 is the **last loaded** operand (so `fld a; fld b; fcompp` tests `b` against `a`).
  - `fucompi` / `lahf` forms in `0xBE6C50` follow the same tests on AH.
- Float stores `fstp dword` round to f32; the game's x87 runs at 53-bit precision (port notes in `frame.rs`: "x87 at 53 bits as f64, f32 round trips kept"). Every intermediate that is `fstp dword` / re-`fld dword` below is a real f32 round trip and must be reproduced.
- Callee conventions below were read from the call sites (registers set before the call, pushes, the `add esp` after) and the callee's `ret` / `ret N`. Callees named in `functions.csv` use that name; anonymous ones are "sub_XXXXXX" and listed in `candidates_doc59.csv` when I could name them.

---

# Group 1: AI order updates

Both are slot 2 (`update`) of the 36 x 28-byte order table at 0xDD5568 (kind 0 at 0xDD5570, kind 0xD at 0xDD5570 + 0xD*0x1C). The caller is `ai_order_update` (0x4372E0, at 0x437579..0x437587): `push esi(human); call eax; add esp,4`; the return value is ignored, so both are `void` with EAX/ECX/EDX garbage on exit. Both save and restore EBX, ESI, EDI, EBP (`push ebx/esi/edi` after the `sub esp`; every `ret` is the same 5-instruction epilogue `pop edi, pop esi, pop ebx, mov esp,ebp, pop ebp, ret`), so any exit can be modelled as `return`.

Shared offsets (human object): +0x14 pos vec3, +0x20 3x3 matrix, +0x38 forward vec3, +0x44 handle, +0x80 flags dword (bit 5 = active), +0xC0/+0xC8 flag dwords, +0x103 / +0x102 flag bytes, +0x148 sub-action byte, +0x149 order kind, +0x150..0x152 flag bytes, +0x154 vec3, +0xAF0 target handle (k0d), +0x10C4 inventory, +0x10CC current item, +0x111C signed role byte, +0x5E0 anim controller.

Entity pool lookup used inline everywhere: `obj = [ (handle & 0xFFF)*16 + 0x2149C64 ]`, valid when `obj != 0 && [obj+0x44] == handle`.

## 1.1 `ai_order_k00_update` 0x448CE0

### Calling convention, frame, registers
- cdecl, `[ebp+8]` = human (kept in EBX). EDI = 0 for the whole function except where a case does `mov edi,ebx` or loads a pointer into it (cases C4, C13, C14, C16 set EDI themselves before use). ESI is set per call (`mov esi,ebx` before callees that read ESI).
- Preserved: EBX, ESI, EDI, EBP. Clobbered: EAX, ECX, EDX. Returns nothing. x87 stack balanced at every exit except the C18 early-out at 0x449929, which pops three values on purpose.
- Locals (offsets are `[esp+X]` after the three register pushes, locals span +0x0C..+0x37): `+0x0F` byte flag (C18), `+0x10` byte-offset counter (C16), `+0x14` occupant (C16), `+0x18` loop index / f32 temp (also the stick `fabs` temp), `+0x1C` saved pointer (C16) / f32 temp (C18), `+0x20/+0x24/+0x28` vec3 A (C4 target, C18 delta), `+0x2C/+0x30/+0x34` vec3 B. All writes are inside the frame; no indexed buffer.

### Globals read
`[0xEBCE9C]` (player human ptr) and `[0xEBCEA0]` (player mode), `[0xE99CDC]` (compared with `(int8)[h+0x111C]`), float consts `[0xDD32C0]`=0.0, `[0xE5D4FC]`=0.5236, `[0xDD1C9C]`=50.0, `[0xE5CF58]`=144.0, `[0xDD522C]`=4.0, `[0xE44850]`=0.25, `[0xE5C330]` (qword)=25.0, `[0xE5C118]`=0.70711, `[0xDD0BB4]`=0.2, `[0xE87998]`=-1.0, zero vec3 `0x252A6D0`, zero vec3 `0xEC5ACC`. Writes only human fields (listed under Risks) and the callees' effects.

### Callees
Columns: regs read, stack args top-first (first pushed is last), who pops, return.

| Address | Name | Regs in | Stack args | Pops | Returns |
|---|---|---|---|---|---|
| 0x9B92E0 | human_entering_or_exiting_vehicle | EAX=human | - | - | AL |
| 0xAE28F0 | vehicle_from_handle | ECX=handle | - | - | EAX |
| 0x456070 | ai_order_k14_start | ECX=human | - | - | ignored |
| 0x960A80 | sub_960a80 | ESI=human | (1, 0) | caller 8 | ignored |
| 0x55A930 | inventory_equip_slot | - | (&h+0x10C4, 0xB, 1, 0) | callee `ret 0x10` | ignored |
| 0x963540 | object_is_player_human | EAX=human | - | - | AL |
| 0x445E90 | sub_445e90 | EAX=human | - | - | AL |
| 0x44A890 | sub_44a890 | EAX=human | (1) | caller 4 | ignored |
| 0x41F150 | ai_order_k0b_start | ECX=human | - | - | ignored |
| 0x438710 | human_from_handle_active | EAX=handle | (1) | caller 4 | EAX=human or 0 |
| 0x97F500 | sub_97f500 | EAX=human | - | - | AL (also sets ECX) |
| 0x409B30 | sub_409b30 | ECX=human | (other, 0) | caller 8 | AL |
| 0x43BCC0 | sub_43bcc0 | - | (human, 0) | caller 8 | EAX |
| 0x445250, 0x44EF70, 0x446250 | sub_* | EAX=human | - | - | ignored |
| 0x9B5FF0 | ai_gaze_clear | EAX=human | - | - | - |
| 0x9B5CE0 | sub_9b5ce0 | EAX=human | (&out vec3) | caller 4 | - |
| 0xBDED80 | sub_bded80 | - | (&v1, &v2, &v3) = (&out of 9B5CE0, &h+0x38, 0xEC5ACC) | caller 0xC | **XMM0** (movss; SSE, not ST0) |
| 0x4449E0 | sub_4449e0 | - | (human, 0x1F4, 1) | caller 0xC | ignored |
| 0x912CF0 | sub_912cf0 | ESI=human | - | - | - |
| 0x4FFE60 | input_stick_action_value | EAX=axis (3 or 4) | - | - | **ST0** f32 |
| 0x458580 | ai_order_k23_start | ESI=human | - | - | - |
| 0x9954C0 | human_state_set | ECX=1, EDX=0 | (human) | caller 4 | - |
| 0x9636F0 | sub_9636f0 | EAX=human | - | - | AL |
| 0x443E80 | net_send_prop_human_ai_data_wander_params | ECX=&h+0xF8 | (&h+0x14, f32 50.0) | **callee `ret 8`** | - |
| 0x449D00 | sub_449d00 (shared continuation, 168 insns, not part of k00) | - | (human) | caller 4 | - |
| 0x444B40 | sub_444b40 | EDI=human | - | - | AL |
| 0x925030 | handle_to_object | ECX=handle | - | - | EAX; leaf, preserves ECX/EDX |
| 0x40B6A0 | vec3_ne_exact | ECX=&A, EDX=&B | - | - | AL |
| 0x994340 | human_pkt_send_09 | ECX=human | (&vec3, 0, 0, 0) | caller 0x10 | AL |
| 0x9149A0 | sub_9149a0 | EDX=[h+0x2F8] | - | - | EAX |
| 0x915990 | sub_915990 | EAX=result of 9149A0 | - | - | AL |
| 0x443E10 | sub_443e10 | ECX=&h+0xF8 | - | - | - |
| 0x9129A0 | sub_9129a0 | - | (a, b, 0) | caller 0xC | - |
| 0x458670, 0x44CC70, 0x44B080, 0x54ADD0 | sub_* | EAX=human | - | - | - |
| 0x4510C0 | sub_4510c0 | EAX=human | - | - | AL, ignored |
| 0x446830 | sub_446830 | ESI=human | - | - | AL |
| 0x444BE0, 0x44A200 | sub_* | ESI=human | - | - | - |
| 0x451550, 0x44F3A0, 0x451170, 0x4520B0 | sub_* | - | (human) | caller 4 | - |
| 0x96F3F0 | sub_96f3f0 | ESI=human | - | - | EAX |
| 0x96F300 | sub_96f300 | EAX=human | (int) | caller 4 | **ST0** |
| 0x4452E0 | sub_4452e0 | EDI=human | - | - | AL |
| 0x43F900 | sub_43f900 | ESI=human | - | - | AL |
| 0x45DDD0 | ai_order_k1c_start | ECX=human | (handle, byte, word, byte, byte, x) | caller 0x18 | AL; the callee reads arg2 as `byte [ebp+0xC]`, arg3 as `word [ebp+0x10]`, arg4/arg5 as `byte [ebp+0x14]/[ebp+0x18]` |
| 0x558D90 | inventory_item_weapon_slot | EAX=&h+0x10C4 | - | - | EAX |
| 0x960D60 | vehicle_seat_occupant | EAX=handle-ish | - | - | EAX |
| 0x44A430, 0x912370 | sub_* | - | (a, b, flag) | caller 0xC | - |
| 0x9122A0 | sub_9122a0 | ECX=thing, EDI=human | - | - | AL |
| 0x9B9320 | human_vehicle_interaction_entering | - | (human) | caller 4 | AL |
| 0xAA7420 | sub_aa7420 | ECX=vehicle | - | - | EAX |
| 0x806720 | human_owned_locally | EAX=obj | - | - | AL |
| 0xBDEA30 | vec3_distance_sq | EAX=&A, ECX=&B | - | - | ST0 |
| 0xBDE570 | vec3_mul_mat33_in_place | EAX=&mat33, ECX=&vec3 | - | - | - |
| 0xBDE150 | vec3_normalize_safe | ESI=&vec3 | - | - | - |
| 0x6EEE20 | human_anim_layer_find | ECX=[h+0x5E0], ESI=id | - | - | EAX (-1 none) |
| 0x9707C0 | net_send_human_pkt_6 | ECX=human | (1, f32 0.2, 0) | caller 0xC | - |
| 0x43F9C0 | ai_human_has_idle_anim_override | EAX=human | - | - | AL; leaf, preserves EDX/ECX |
| 0x96FCA0 | sub_96fca0 (play animation) | ECX=human | (id, 0, 1, 0.0f, 1.0f, -1.0f, 0, -1) | caller 0x20 | AL |
| 0x449F40 | sub_449f40 | ESI=human, EDI=0xBB8 | (handle) | caller 4 | - |
| 0x4375B0 | ai_order_set_action | - | (human, kind, action) | caller 0xC | - |

### Control flow
`act` = `byte [h+0x148]`; `P` = `object_is_player_human(h)`; "exit" = jump to 0x44992F (epilogue). `stick(n)` = `input_stick_action_value(EAX=n)` then `fabs`, stored to f32 `[esp+0x18]`, reloaded, `fcomp [0xDD32C0]` (0.0) then `test ah,0x44`: `jp` (taken) means `!= 0.0`, `jnp` means `== 0.0`.

**K0 gate (0x448CEF-0x448D4E).** Runs only if `h != 0 && [h+0x44] != 0`. Let `p = [0xEBCE9C]`. Skip the block (go to K1) when `p != 0 && [p+0x44] == [h+0x44] && [0xEBCEA0] is neither 7 nor 8` (checks are `cmp eax,7; je body; cmp eax,8; jne skip`). Otherwise: if `human_entering_or_exiting_vehicle(h)` AL != 0 skip; `veh = vehicle_from_handle([h+0xD74])`; if `veh != 0` call `ai_order_k14_start(ECX=h)`. Falls into K1 (no early return).
**K1 (0x448D4F).** `if !((h[0x80] >> 5) & 1) exit`.
**K2 switch A (0x448D60-0x448D84).** `al = [h+0x103]`. If `al & 8`: `idx = act - 1`; `idx > 0x1A (unsigned)` -> TAIL (0x4490F4). Table 0x449938 (3 entries: 0x448D93, 0x448D8B, 0x4490F4), byte map 0x449944[idx]: acts 1,2,3,17,18,27 -> K3; acts 4,5 -> `test al,2; je TAIL` then K3; acts 6..16 and 19..26 -> TAIL. If `!(al & 8)` go straight to K3. **TAIL (0x4490F4)** = `sub_449d00(human)` cdecl, then exit.
**K3 (0x448D93) equip reset.** If `(int8)h[0x111C] == [0xE99CDC]` (movsx byte vs dword): `ESI=h; sub_960a80(1,0)`; `inventory_equip_slot(&h+0x10C4, 0xB, 1, 0)`.
**K4 (0x448DC0).** `if P goto K9`.
**K5 non-player (0x448DCF-0x448E86).**
 1. `sub_445e90(h)` AL != 0: if `h[0xC0] & 0x800` -> `sub_44a890(EAX=h, 1)`, **return**; else `ai_order_k0b_start(ECX=h)`, **return**.
 2. else (0x448E07): if `!(h[0x103] & 8)`: `o = human_from_handle_active([h+0x108], 1)`; if `o != 0` and `sub_97f500(EAX=o)` == 0 and `sub_409b30(ECX=h, o, 0)` != 0 -> exit.
 3. `sub_43bcc0(h, 0)` != 0 -> exit.
 4. switch B: `idx = act - 4`; table 0x449960 (0x448E79 / 0x448E72), map 0x449968[13]: acts 4..6 and 13..16 -> 0x448E79 (skip sub_445250); acts 7..12 and any `idx > 12` -> 0x448E72 (`sub_445250(EAX=h)` first). Then `sub_44ef70(EAX=h)`, `sub_446250(EAX=h)`; fall into K9.
**K9 (0x448E87) gaze.** Skip to K10 if `h[0x758] & 1` or `h[0x103] & 8`. If `h[0x470] & 0x10`: if `!(h[0x140] & 4)` `ai_gaze_clear(EAX=h)`; goto K10. Else if `h[0x150] & 0x40`: `sub_9b5ce0(EAX=h, &vecA)`; `d = sub_bded80(&vecA, &h+0x38, 0xEC5ACC)` (XMM0, `movss [esp+0x24]`, reloaded as f32), `|d|` to f32 `[esp+0x18]`; if `|d| > 0.5236` (`test ah,0x41; jne` skips on `<=`) -> `sub_4449e0(h, 0x1F4, 1)`.
**K10 (0x448F17).** If `h[0xC0] & 0x200000` and `act != 0x11`: `ESI=h; sub_912cf0()`.
**K11 (0x448F33).** `r = (int8)h[0x591] - 2`. If `r <= 1 (unsigned)` and `!(h[0x470] & 0x40)` skip to K12. Otherwise at 0x448F4B: if `[h+0x4C] == 1 && byte [h+0x3CC] == 1` skip; if `h[0x103] & 8` skip; else `h[0x152] &= 0xFD; [h+0x16C] = 0xFFFFFFFF`.
**K12 switch C (0x448F74).** `idx = act - 1`; `idx > 0x2D (unsigned)` -> exit. Table 0x449978 (21 entries), byte map 0x4499CC[46]. Table entries (case -> address) and the acts that select them:

| Case | Target | Acts | Behaviour |
|---|---|---|---|
| C0 | 0x448F95 | 1 | `P`: if `stick(3) != 0.0 || stick(4) != 0.0`: `ESI=h; ai_order_k23_start()`, `human_state_set(ECX=1,EDX=0,h)`, return; else exit. Not `P`: if `sub_9636f0(EAX=h)`: `net_send_prop_human_ai_data_wander_params(ECX=h+0xF8, &h+0x14, 50.0)`, `sub_449d00(h)`, return. Else if `(uint)((int8)h[0x111C]-5) <= 1`: `EDI=h; sub_444b40()`, return. Else exit |
| C1 | 0x4491D2 | 3 | exit if `h[0xC8] & 0x80000`; exit if `[h+0x10CC] != 0 && inventory_item_weapon_slot(EAX=&h+0x10C4) != 0`; `ESI=h; sub_444be0()`; `sub_449d00(h)`; return |
| C2 | 0x449214 | 4,5,6 | `sub_451550(h)`; return |
| C3 | 0x449224 | 7..12, 0x14, 0x15 | `sub_44f3a0(h)`; return |
| C4 | 0x449057 | 0xD | vec3 A = `[h+0x17C..0x184]` (fld/fstp copies); `hnd=[h+0x178]`; if `hnd != 0` and `handle_to_object(ECX=hnd) != 0`: `o = handle_to_object(ECX=hnd)` (second call, same ECX) and A = `o+0x14` (3 dwords). If `vec3_ne_exact(ECX=0x252A6D0, EDX=&A)`: `human_pkt_send_09(ECX=h, &A, 0,0,0)`; return. Else exit |
| C5 | 0x449234 | 0xE,0xF,0x10 | `e=[h+0x2D0]`; if `e != 0 && [e+0xC] != [h+0x44]` exit; `sub_449d00(h)`; return |
| C6 | 0x4490D4 | 0x11 | `edx=[h+0x2F8]`; 0 -> TAIL. `t = sub_9149a0(EDX)`; if `!sub_915990(EAX=t)`: `[h+0x2F8]=0`; TAIL. If `[t+0x2C] != [h+0x44]`: `[h+0x2F8]=0`; TAIL. If `P`: if `stick(3) != 0.0 \|\| stick(4) != 0.0`: `sub_443e10(ECX=h+0xF8)`, `sub_9129a0(t, h, 0)`, `sub_458670(EAX=h)`, return; else exit. If not `P`: `sub_4510c0(EAX=h)`; return |
| C7 | 0x44919A | 0x12 | `al=[h+0x152]`; if `al & 4`: clear it, return. Else `ESI=h; sub_446830()` AL != 0 -> exit; `sub_449d00(h)`; return |
| C8 | 0x44925A | 0x13 | `sub_451170(h)`; return |
| C9 | 0x44926A | 0x16,0x17,0x18 | `sub_4520b0(h)`; return |
| C10 | 0x44927A | 0x1C | `ESI=h; r=sub_96f3f0()`; if `r != [h+0x170]` TAIL. `f = sub_96f300(EAX=h, [[h+0x5E0]+0x168])` (ST0); `fcomp [0xE44850]` (0.25) then `test ah,5; jp exit`: exit when `f >= 0.25`; else `sub_449d00(h)`; return |
| C11 | 0x4492C5 | 0x1D..0x26 | `sub_44cc70(EAX=h)`; return |
| C12 | 0x4492D3 | 0x27 | `sub_44b080(EAX=h)`; return |
| C13 | 0x4492E1 | 0x28 | `EDI=h; if sub_4452e0() exit`; `[h+0x1C4]=0`; `sub_449d00(h)`; return |
| C14 | 0x44930A | 0x29 | `EDI=h; if !sub_4452e0()` -> as C13. Else `v=vehicle_from_handle(ECX=[h+0x1C4])` (EDI=v); `v==0` exit; `vec3_distance_sq(EAX=h+0x14, ECX=v+0x14)` vs 144.0 (`jp exit` = exit when `d2 >= 144`); `ESI=h; b=sub_43f900()`; `[h+0x1C4]=0`; `AL = ai_order_k1c_start(ECX=h, [v+0x44], b, 0,0,0,0)`: pushes are `esi,esi,esi,esi(0),eax,ecx` = top-first `([v+0x44], b-dword, 0, 0, 0, 0)` (b passed as a dword whose upper bytes are stale; callee reads a byte); AL != 0 exit; else `[h+0x1C4]=0`; `sub_449d00(h)`; return |
| C15 | 0x44938E | 0x2A | `sub_54add0(EAX=h)`; return |
| C16 | 0x4493AA | 0x2B | see below |
| C17 | 0x4495DB | 0x2C | `if human_vehicle_interaction_entering(h)` exit. `v=vehicle_from_handle(ECX=[h+0x318])`. If `v != 0 && sub_aa7420(ECX=v) == 0 && human_owned_locally(EAX=v)`: `d2 = vec3_distance_sq(h+0x14, v+0x14)`; if `d2 < 144.0` (not `jp`): `f=(byte[h+0x151]>>2)&1`; `AL=ai_order_k1c_start(ECX=h, [v+0x44], 0, f, 0, 0)` (pushes `edi,edi,edx,edi,edi,eax` -> top-first `([v+0x44], 0, 0, f, 0, 0)`), AL != 0 exit. Then if `h[0x4A0] & 4` exit. Finally `[h+0x318]=0; sub_449d00(h)`; return |
| C18 | 0x449679 | 0x2D | grab / hold animation state, see below |
| C19 | 0x44939C | 0x2E | `ESI=h; sub_44a200()`; return |
| C20 | 0x44992F | 2, 0x19, 0x1A, 0x1B | exit |

(C14 push order: the sequence at 0x44934D is `xor esi,esi; push esi x3; mov [esp+0x24],al; mov eax,[esp+0x24]; push esi; mov [h+0x1C4],esi; mov ecx,[edi+0x44]; push eax; push ecx`, i.e. six arguments `(ecx=veh.handle, eax=dword{stale|AL}, 0, 0, 0, 0)`; C14 puts the byte in **arg2**, C17 puts `f` in **arg4**. Keep them as written.)

**C16 (0x4493AA..0x4495DA).** `t = sub_9149a0(EDX=[h+0x2F8])`; EDI=t, saved to `[esp+0x1C]`. `if !sub_915990(EAX=t)` -> TAIL. `if [t+0x2C] != [h+0x44]` -> TAIL. `edx=[t+0x30]; esi=[edx+4]` (group record). If `!([esi+0x1C] & 0x80)` jump to A. `d2 = vec3_distance_sq(EAX=h+0x14, ECX=t+4)` stored f32 `[esp+0x18]`; on x87 `fld 4.0; fld d2; fcom st1`: if `d2 >= 4.0` -> A; `fcomp [0xE44850]`: if `d2 <= 0.25` -> A. `n = byte [esi+0x48]`; `i=0`; `n == 0` -> A. Loop for `i < n`: `m = [[t+0x30] + 8 + 4*i]`; if `m == t` -> A (leaves the loop entirely); `occ = vehicle_seat_occupant(EAX=[m+0x2C])` -> `[esp+0x14]`; if `occ != 0`: if `!([occ+0xC0] & 0x200000)` and `vec3_distance_sq(EAX=occ+0x14, ECX=m+4) > 25.0` (qword compare, `test ah,0x41; jne` skips on `<=`): `sub_9129a0(t, h, 0)`; `sub_9129a0(m, occ, 0)`; `sub_44a430(m, h, 1)`; `sub_44a430(t, occ, 0)`; next `i`. If `occ == 0`: `sub_9129a0(t, h, 0)`; `sub_44a430(m, h, 1)`; -> A.
 **A (0x4494F6):** if `P`: if `stick(3) != 0.0 || stick(4) != 0.0`: `ESI=h; k23_start()`; `human_state_set(1,0,h)`; `sub_9129a0(t, h, 0)`; return (equal sticks fall through). Then: exit if `h[0x4A0] & 4`; exit if `h[0x591] != 0`; exit if `h[0x470] & 0x40`; `ESI=t; EDI=h; ECX=t; AL=sub_9122a0()`: if AL -> `sub_912370(t, h, 0)`; return; else if `[h+0x2F8] != 0` -> `sub_9129a0(t, h, 0)`; then `sub_449d00(h)`; return.

**C18 (0x449679..0x44992E).** `h[0x470] |= 0x40`; `dl = byte [h+0x178]` (kept in EDX across calls: `handle_to_object` and `ai_human_has_idle_anim_override` are leaves that do not touch EDX, so re-reading `[h+0x178]` is equivalent while nothing writes it; the only write is `or byte [h+0x178],1` after `sub_96fca0`); local byte `[esp+0x0F] = 0`.
 - `dl & 1 == 0` -> path R (0x4497E0). `dl & 3 == 1` -> path S (0x4497BE). `dl & 3 == 3`: `o = human_from_handle_active([h+0x17C], 1)`; if `o != 0` and `role = byte[o+0x590]` is 3 or 2: `local1 = [o+0x5C8..0x5D0]`; `vec3_mul_mat33_in_place(EAX=o+0x20, ECX=&local1)`; `delta = o.pos - h.pos` (f32 `fsub dword`); `vec3_normalize_safe(ESI=&delta)`; `dot1 = delta . local1` accumulated as `((dy*ly) + (dx*lx)) + (dz*lz)`, f32; if `dot1 <= 0.0` (`test ah,0x41; jne`) -> **0x449929: three `fstp st` then exit**; else `dot2 = ((fwd.y*dy)...` built at 0x449757 as `fld [h+0x3C]; fmulp st3; fmul [h+0x38]; faddp st2; fmul [h+0x40]; faddp` (order: y, then x, then z), f32; exit when `dot2 >= 0.70711` (`test ah,5; jp`). Role not 2/3 or `o == 0`: go to 0x44977F.
   0x44977F: `id = [h+0x180]`; `ecx=[h+0x5E0]`; if `id != -1` and `human_anim_layer_find(ECX, ESI=id) != -1`: `net_send_human_pkt_6(ECX=h, 1, 0.2f, 0)`. Then FIN.
 - Path S: `id=[h+0x180]`; if `id == -1` FIN; `human_anim_layer_find`; **if the result != -1 exit** (0x449900 `jne exit`); else FIN.
 - Path R: `has = ai_human_has_idle_anim_override(EAX=h) ? 1 : 0`; `esi=[h+0x17C]`.
   * `esi == 0` (0x4498C4): if `has` FIN; else `AL = play`: `sub_96fca0(ECX=h, [h+0x180], 0, 1, 0.0f, 1.0f, -1.0f, 0, -1)`; `h[0x178] |= 1`; `AL != 0` exit; else FIN.
   * `esi != 0`: if `has` FIN; `o=handle_to_object(ECX=esi)`; `o==0` FIN. If `dl & 2`: `human_pkt_send_09(ECX=h, &h+0x14, 0,0,0)` (result ignored), `AL=play(...)`, `h[0x178]|=1`; `AL == 0` -> FIN; else `EDI=0xBB8; ESI=h; sub_449f40([h+0x17C])`; return. If `!(dl & 2)`: `AL=human_pkt_send_09(ECX=h, &o+0x14, 0,0,0)`; `AL == 0` exit; `play(...)`, `h[0x178]|=1`; `AL != 0` exit; FIN.
 - **FIN (0x449902):** `h[0x470] &= 0xBF`; `a=[h+0x184]`; if `a == 0` TAIL else `ai_order_set_action(h, 0, a)`; return.

### Risks (k00)
- Hidden register inputs: ESI, EDI or EAX are the only argument of about 20 callees (table above); every call must set the register exactly. EDI is a live 0 constant in most blocks.
- EDX carried across two leaf calls in C18; covered above.
- Mixed FP: ST0 returns (`input_stick_action_value`, `vec3_distance_sq`, `sub_96f300`) and one XMM0 return (`sub_bded80`); all compares go through f32 temps.
- `k1c_start` reads arg2 as a byte, so the stale upper 24 bits C14 passes in arg2 (the dword `[esp+0x24]`, only the low byte written) are harmless; pass `b as u32`.
- Cases never fall through into each other, so a Rust `match` with early `return` is faithful. Shared blocks are TAIL, FIN and exit only.
- No unbounded or indexed writes. Human fields written: `[h+0x152]`, `[h+0x16C]`, `[h+0x178]`, `[h+0x1C4]`, `[h+0x2F8]`, `[h+0x318]`, `[h+0x470]`.

### Split into hand-portable pieces
1. `k00_gate(h)` (K0+K1), pure reads except the optional `ai_order_k14_start`.
2. `k00_switch_a(h) -> {Continue, Tail}` (K2).
3. `k00_nonplayer(h) -> {Return, Continue}` (K5, including switch B).
4. `k00_gaze(h)` (K9, the only XMM0 callee) and `k00_flags(h)` (K10, K11).
5. One function per switch-C case; the single-call cases (C2, C3, C5, C8, C9, C11, C12, C15, C19) are one line each.
6. Port order by risk: single-call cases, K0..K2, K10, K11, C1/C7/C13, C0, C10, C14, C17, C4, C6, C16, C18, K9. Use the `seq_port` shadow (recording stubs for every callee, compare, then replay).

## 1.2 `ai_order_k0d_update` 0x42D2B0

Order kind 0x0D is the "attack / shoot at target" order (sub-actions 0x78..0x83 = 120..131; `[h+0xAF0]` is the target handle, `[h+0x150..152]` its flag bytes, `weapon_fire`, `ai_event_*` re-posts with kind 0xD and sub-action in the last argument). Evidence: every action number below appears as the last argument of `ai_event_post(.., 0xD, act)` or `ai_order_set_action(h, 0xD, act)`.

### Calling convention, frame, registers
- cdecl, `[ebp+8]` = human (EBX). Prologue `push ebp; mov ebp,esp; and esp,-8; sub esp,0x9C; push ebx; push esi; push edi`. Preserved: EBX, ESI, EDI, EBP; clobbered EAX, ECX, EDX; returns nothing. Epilogue is the 5-instruction pattern; several exits leave stack junk (`push` args not popped) that `mov esp,ebp` discards.
- Register roles that are **live across blocks**: EBX = human always; ESI = leader (`human_effective_leader`, 0x42D39C) then reused as scratch; EDI = `winfo` early on (`[esp+0x24]` copy), later reused. Do not assume ESI/EDI keep these roles: each block re-reads the stack copies noted below.
- Locals (relative to `[esp+X]` after the three pushes): `+0x13` byte `inVeh`, `+0x14` dword `target` (human*), `+0x18` byte `lostSight`, `+0x1C` f32 temp (**read uninitialised in places**, see Risks), `+0x22` byte `facing`, `+0x23` byte `leaderFlag` (AL of `sub_42bc30(EAX=h)`), `+0x24` dword `winfo` (weapon-info ptr), `+0x28` f64/f32 temp, `+0x2C/0x30/0x34` vec3, `+0x3C/0x40/0x44` vec3 delta, `+0x48/0x4C/0x50` vec3, `+0x54/0x58/0x5C` vec3 `aim`, `+0x60/0x64/0x68` vec3 (muzzle position, filled by 9C4430), `+0x78/0x7C/0x80` vec3 dir, `+0x84..0x8C` and `+0x9C..0xA4` out areas of `human_attach_transform_slot0` and `rig_bone_transform`. 0x9C bytes total; no indexed writes.

### Globals read/written
Read: `0x2149C64` (entity pool), `0x2526A3E` (byte), `0x2797748` (u32 threshold), `0x2797A44/A48/A4C` (f32 aim jitter params), `0x2797A8C/A90/A94/A98/A9C/AA0` (u32 delay ranges for the event re-posts), consts `[0xE5C380]`=15.0 (qword), `[0xDD1C8C]`=0.5, `[0xE5C2E0]`=0.8 (qword), `[0xE8B048]`=0.3, `[0xE5C700]`=50.0 (qword), `[0xDD04F8]`=0.85, `[0xE8437C]`=1/60, `[0xE5C0D8]`=1000.0 (qword), `[0xDD518C]`=1.0, `[0xE5DC18]`=2.0 (qword), `[0xDD32C0]`=0.0, `[0xE44850]`=0.25, `[0xE5CF58]`=144, `[0xE87998]`=-1.0. Written: human fields `[h+0x150]` (bit 3), `[h+0x151]` (bit 6), `[h+0x152]`, `[h+0x164..0x16C]`, `[h+0x178]` (+= scaled dt, E-block), `[h+0x334]`; plus x87 control word (`fnstcw/fldcw` at 0x42E31B, rounding mode set to truncate for `fistp qword` and restored).

### Callees (condensed; "r" = registers read, "s" = stack args top-first, cdecl unless `ret N` noted)
| Address | Name | r | s | Returns |
|---|---|---|---|---|
| 0x438B30 | sub_438b30 | EDI=h | - | AL |
| 0x9B2FE0 | net_send_human_pkt_11 | ECX=h | (0) | - |
| 0x447CD0 | object_teleport_prepare | - | (h, 0, 0) | - |
| 0xB72800 | weapon_info_is_firearm | EAX=winfo-ish (`[ecx+8]`) | - | AL |
| 0x98E7C0 | human_effective_leader | EDI=h (also EAX) | - | EAX |
| 0x42BC30 / 0x42BD10 / 0x4295D0 / 0x429260 | sub_* | EAX=h | - | AL (all leaves; 0x429260 = `h[0x150] & 0x60 != 0`, preserves ECX) |
| 0x41F150 | ai_order_k0b_start | ECX=h | - | AL |
| 0x426990 | sub_426990 | ESI=h | (0, 0, 0) | AL |
| 0x53C4D0 | activity_current_type | - | - | EAX |
| 0x975B40 | sub_975b40 | EAX=h | (0) | EAX (a human) |
| 0x963540 | object_is_player_human | EAX | - | AL |
| 0x4254A0 | ai_order_k22_start | - | (h, other, 0, 0, 0, 0) | AL |
| 0x4375B0 | ai_order_set_action | - | (h, kind, act) | - |
| 0x96D990 | human_action_flag_bit1 | EAX=h | - | AL |
| 0x994F50 / 0x995250 | net_send_human_pkt_7 / _8 | ECX=h | (0) | - |
| 0x438710 | human_from_handle_active | EAX=handle | (1) | EAX |
| 0x9CE0A0 | sub_9ce0a0 | ECX=h, ESI=target | - | AL |
| 0x42EE10 | sub_42ee10 | EAX=h | - | AL |
| 0x409B30 | sub_409b30 | ECX=h | (x, 0) | AL |
| 0xB72F80 | weapon_info_is_lethal | ECX=winfo | (0) | AL |
| 0x43A190 | sub_43a190 | EAX=h | (target, 0) | AL |
| 0x55A930 | inventory_equip_slot | - | (&h+0x10C4, slot, flag, 0), `ret 0x10` | - |
| 0x42B740 | net_send_human_pkt | ECX=h | - | - |
| 0x43C1F0 | sub_43c1f0 | - | (h) | EAX (human) |
| 0xBDEA30 | vec3_distance_sq | EAX, ECX | - | ST0 |
| 0x453400 | sub_453400 | EAX=handleA, ECX=handleB | (1) | - |
| 0x9B3980, 0x960D60 | sub_9b3980 / vehicle_seat_occupant | ECX=x / EAX=result of 9B3980 | - | EAX |
| 0x95E700 | sub_95e700 | EDI=h | (occupant) | AL |
| 0x963580 | human_npc_field_1178_set (really a bool test) | EAX=occupant | - | AL |
| 0xBDDFF0 | vec3_normalize_in_place | ESI=&vec | - | - |
| 0xBDD1B0 | timer_set_random | EDI=&timer, EAX=base ms | (range) `ret 4` | - |
| 0x401420 | ai_event_post | ECX=type, EAX=delay (-1), EDI=payload ptr or 0 | (h, h, 0xD, sub) caller 0x10 | - |
| 0x402160 | ai_event_cancel | ESI=type | (h, h) | - |
| 0xBDD230 | timer_elapsed | EAX=&timer | - | AL |
| 0x439FA0 | sub_439fa0 | ECX=h, EAX=0 | (1) | - |
| 0x42B140 | sub_42b140 | ESI=h | (f32-sized dword `[esp+0x1C]`) | AL |
| 0x6AE180 | sub_6ae180 | EAX=h | (0x28C, f32 -1.0, 0xFFFF, 1) | - |
| 0x42B680 | sub_42b680 | ESI=h | - | AL |
| 0x439660 | ai_human_leader_flag4 | EAX=h | - | AL |
| 0x429750 / 0x42B4B0 / 0x42B3B0 | sub_* | - | (h) / (h) / (h, target) | - |
| 0x401580 | ai_event_post_random_delay | ECX=max, ESI=min, EDI=0 | (type, h, h, 0xD, sub) caller 0x14 / 0x1C | - |
| 0x55AE30 | sub_55ae30 | - | (&h+0x10C4, flag-dword, 0, 0, 0) `ret 0x14` | EAX |
| 0xB71A60 | weapon_info_ai_range_sq | EAX=winfo | - | ST0 |
| 0x453730 | sub_453730 | ESI=handleT, EDI=handleH | - | EAX (2 = ...) |
| 0xBDCF50 | rand_int_range | ECX=max, ESI=min | - | EAX |
| 0x42A780 | sub_42a780 | EAX=h | - | AL |
| 0x9C5A60 | sub_9c5a60 | - | (h, target, winfo) | AL |
| 0x42AAE0 | sub_42aae0 | EAX=target | - | - |
| 0xCEDF10 | rig_bone_transform | ECX=`[[target+0x5E0]]` | (bone, &pos, &mat, &out, 0) | - |
| 0xBDE060 | vec3_normalize_in_place_ret_len | ESI=&vec | - | **XMM0** |
| 0xBDE750 | vec3_random_unit | ESI=&out | - | - |
| 0x42EEA0 | sub_42eea0 | EAX=&out, ECX=&a | (&b) `ret 4` | - |
| 0xBDCF90 | rng_float_range | - | (f32 lo, f32 hi) caller 8 | ST0 |
| 0x9C4430 | human_attach_transform_slot0 | EAX=h | (&out_mat_or_pos, &out2) | - |
| 0xBDE410 / 0xBDE290 | - | ESI=&vec | - | XMM0 / (in place) |
| 0x9642A0 | sub_9642a0 | ECX=h | - | ST0 |
| 0x9C0500 | weapon_fire | - | (h, target, ...) 7 dwords, caller 0x1C | - |
| 0x9C4ED0 / 0x9C4EF0 | sub_* | EAX=&h+0x154, ECX=h / - | - / (h) | - |
| 0x41B9D0 | sub_41b9d0 | ECX=h | (handle, 0) | AL |
| 0x42B600 | sub_42b600 | EAX=target, ECX=h, ESI=target | - | AL |
| 0x410D70 | ai_order_k06_start | ECX=target | (h, [h+0xF8], [h+0xFC], 0) | - |
| 0x436AF0 | sub_436af0 | ESI=&h+0xF8 | (-1, -1) `ret 8` | - |
| 0x9B8C50 | human_in_vehicle | ESI=target (EDI=0) | - | AL |
| 0x4623E0 | ai_order_k1f_start | ECX=h | (handle, 1) | AL |
| 0x40E420 / 0x40D0F0 | sub_* | EAX=target (`mov eax,edi`), + stack (h, f32 0.85) / ESI=h | (h, 0.85f) / - | AL |
| 0x9B3680 | sub_9b3680 | EDX=h | - | AL |
| 0x98E8A0 | human_is_player_or_player_follower | (EDI=?) | - | AL |
| 0x9B2DA0 | net_send_human_pkt_10 | - | (h) | - |
| 0x40D450 | ai_order_k05_start | ECX=h, EDX=target | (1) | - |
| 0x43BCC0 | sub_43bcc0 | - | (h, 0) | EAX |
| 0x42BB40 | sub_42bb40 | EAX=h | - | - |
| 0x40EA20 | sqrtf_wrapper | - | (f32) | ST0 |
| 0x9C5DD0 | sub_9c5dd0 | EAX=h | (f32) | ST0 |
| 0x9C5E80 | sub_9c5e80 | EAX=target | (h) | ST0 |
| 0x9C8C20 / 0x960C30 | sub_* | - / ESI=h | (h, 0) / - | - / AL |

### Control flow
`act = byte [h+0x148]`; "exit" = the single epilogue at 0x42E5D5.

**B0 (0x42D2C0-0x42D2F3).** If `h != 0 && [h+0x44] != 0`: `obj = pool([h+0x44])`; if `obj != 0 && [obj+0x44]==handle && [obj+0x4C]==1 && byte [obj+0x3CC]==1` exit. (Same "frozen object" gate as `k0b`/0x449D00.)
**B1 (0x42D2F9).** `if !((h[0x80] >> 5) & 1) exit`.
**B2 (0x42D30B).** `EDI=h; if !sub_438b30()`: `net_send_human_pkt_11(ECX=h,0)`; `object_teleport_prepare(h,0,0)`; return. (The second half, 0x42D322, is a shared "abort": `object_teleport_prepare(h,0,0)` then return.)
**B3 (0x42D336).** `if [h+0xE88] != 0` -> **SEND** (0x42E40A: `net_send_human_pkt(ECX=h)`; return).
**B4 (0x42D343).** `item=[h+0x10CC]`; `inst = item ? [item+0xC4] : 0` (ECX); `winfo = inst ? [inst+8] : 0` stored `[esp+0x24]` (the code re-tests `[item+0xC4]` for zero separately). If `act != 0x78`: `inst == 0` -> SEND; `weapon_info_is_firearm(EAX=[inst+8])` == 0 -> SEND.
**B5 (0x42D397).** `esi = human_effective_leader()` (EDI=h); `[esp+0x23] = sub_42bc30(EAX=h)`.
**B6 switch 1 (0x42D3A9).** `idx = act - 0x79`, `idx > 10` -> B7. Table 0x42E5DC (3 entries 0x42D3CA, 0x42D465, 0x42D47C), map 0x42E5E8: acts 0x79,0x7C,0x81,0x82,0x83 -> 0x42D3CA; 0x7E,0x7F -> 0x42D465; 0x7A,0x7B,0x7D,0x80 -> B7.
 * 0x42D3CA: if `leaderFlag` and `ai_order_k0b_start(ECX=h)` != 0 -> exit. If `(uint)(int8)h[0x111C] > 4` -> B7. `ESI=h; sub_426990(0,0,0)` == 0 -> B7. `activity_current_type() == 0xD` -> B7. `x = sub_975b40(EAX=h, 0)`; `x == 0` -> B7; `object_is_player_human(EAX=x)` -> B7; `byte x[0x149] == 0x22` -> B7; `[x+0xEA8] != 0` -> B7; `(byte)x[0x111C] == (byte)h[0x111C]` -> B7; `ai_order_k22_start(h, x, 0,0,0,0)` AL != 0 -> exit. -> B7.
 * 0x42D465: if `leaderFlag`: `ai_order_set_action(h, 0xD, 0x80)`. -> B7.
**B7 (0x42D47C).** `if sub_96d990(EAX=h)` -> B8. Else: `if sub_42bd10(EAX=h)`: (`!(h[0xC0] & 0x80)` -> `net_send_human_pkt_7(ECX=h,0)`), else (`h[0xC0] & 0x80` -> `net_send_human_pkt_8(ECX=h,0)`).
**B8 (0x42D4BB) target acquisition.** `inVeh=0; facing=0; lostSight=0`. If `h[0x150] & 0x20` -> `target=0`, goto B13. Else `target = human_from_handle_active([h+0xAF0], 1)`; `target == 0` -> **DEAD** (0x42E59E: `sub_9c8c20(h,0)`; `ESI=h; if sub_960c30()` `sub_43bcc0(h,0)`; `object_teleport_prepare(h,0,0)`; return). If `h[0x150] & 0x40` -> B13. `ECX=h; ESI=target; sub_9ce0a0()` == 0 -> abort (`object_teleport_prepare(h,0,0)`; return).
**B9 (0x42D51A).** `winfo=[esp+0x24]`; if `winfo != 0` and `winfo[0x34]` is 0x12 or 8 and `sub_42ee10(EAX=h) == 0`: switch 2 on `act - 0x78` (`> 0xB` skip): table 0x42E5F4 (0x42D557, 0x42D56D), map 0x42E5FC: acts 0x78..0x7C and 0x81..0x83 -> 0x42D557 (`sub_409b30(ECX=h, 0, 0)` AL != 0 -> exit); acts 0x7D..0x80 -> B10.
**B10 (0x42D56D).** `if weapon_info_is_lethal(ECX=winfo, 0)` (note: `winfo` may be 0; the callee is entered with ECX=0): `if sub_43a190(EAX=h, target, 0) == 0`: `inventory_equip_slot(&h+0x10C4, 0, 1, 0)`; `net_send_human_pkt(ECX=h)`; return. (`sub_43a190` nonzero skips.)
**B11 (0x42D5AE).** If `act == 0x7C` or `act in {0x81, 0x82}` (cmp 0x7C je; `cmp 0x80; jle skip; cmp 0x82; jg skip`): `d = sub_43c1f0(h)`; if `d != 0 && d != leader(ESI)`: `[esp+0x1C] = f32([esp+0x1C]) + 15.0` (qword add; **[esp+0x1C] is uninitialised here**); if `!(h[0x103] & 0x20)` or `!object_is_player_human(EAX=leader)`: `r2 = [esp+0x1C]^2` (qword stored `[esp+0x28]`); `d2 = vec3_distance_sq(h+0x14, leader+0x14)`; `if d2 <= r2` skip (`test ah,0x41; jne`); then `sub_409b30(ECX=h, d, 0)`; return. (When the `0x20 && player` combination holds the compare is skipped and the call is made unconditionally.)
**B12 (0x42D631).** If `act != 0x83`: `[h+0x164..0x16C] = leader.pos`; `h[0x152] |= 0x10`. `sub_453400(EAX=[leader+0x44], ECX=[h+0x44], 1)`. `ecx=[leader+0x44]`; if `ecx == [h+0x334]`: if `byte [0x2526A3E] == 0` -> B13; if `(uint)((int8)h[0x111C]-5) > 1` -> B13; `leader = target` (ESI=`[esp+0x14]`). `v = sub_9b3980(ECX=leader)`; `occ = vehicle_seat_occupant(EAX=v)` (ESI); `occ == 0 || occ == h` -> B13; `if !sub_95e700(EDI=h, occ)`: `byte occ[0x111C] != 7` -> B13; `(uint)((int8)h[0x111C]-5) > 1` -> B13. Then: `if target.handle == [h+0x334]` `[h+0x334] = 0`; `inVeh = 1`; `lostSight = sub_963580(EAX=occ)`; `delta = h.pos - target.pos` (f32); `vec3_normalize_in_place(ESI=&delta)`; `facing = 1`; `dot = target.fwd(+0x3C,+0x38,+0x40) . delta` (order y, x, z) f32; `if dot > 0.5` (`fcomp [0xDD1C8C]`; `test ah,0x41; jnp B13` skips when `dot <= 0.5`) `facing = 0`.
**B13 (0x42D774) flag sync.** `al=[h+0x152]; cl=inVeh; dl=(al>>1)&1`. If `dl != cl`: if `cl != 0 && !(al & 4)`: `h[0x152] |= 4`; `v=(int8)h[0x111C]`; unless `v == 5` or (`v == 0 && lostSight != 0`): `timer_set_random(EDI=&h+0x17C, EAX=0x1388, 0xBB8)`; then `ai_event_post(ECX=0x6F, EAX=-1, EDI=0, h.handle, h.handle, 0xD, 0)`; `cl = leaderFlag` (reloaded, quirk). Then (both `cl` cases) `h[0x152] = (h[0x152] & ~2) | ((cl & 1) << 1)` (`add cl,cl; xor cl,[h+0x152]; and cl,2; xor [h+0x152],cl`). If `dl == cl` skip all of this.
**B14 (0x42D7F5).** `al = sub_4295d0(EAX=h)`; `h[0x152] = (h[0x152] & ~8) | ((al & 1) << 3)`. `dl=[h+0x152]`; `cl = (inVeh && !facing && !timer_elapsed(EAX=&h+0x17C)) ? 1 : 0` -> `lostSight2` (stored `[esp+0x18]`). `if !(dl & 8)`: `if sub_429260(EAX=h) || cl == 0`: `inVeh = 1; if leaderFlag inVeh = 0;` else `inVeh = 0`. `if dl & 8`: `inVeh = 0`. (Precisely: `if (sub_429260 != 0 || cl == 0) { inVeh = leaderFlag ? 0 : 1 } else { inVeh = 0 }` when `!(dl&8)`.)
 Then `sub_439fa0(ECX=h, EAX=0, 1)`; `AL = sub_42b140(ESI=h, dword [esp+0x1C])` (uninit dword passed); `if byte [h+0x149] != 0xD` exit.
**B15 (0x42D88C).** If `h[0x152] & 2`: `AL != 0` -> B16; `(uint)((int8)h[0x111C]-5) <= 1` -> `sub_6ae180(EAX=h, 0x28C, -1.0f, 0xFFFF, 1)`; then B15b. Else (`!(h[0x152]&2)`): `AL != 0` -> B16. **B15b (0x42D8D2):** `sub_42b680(ESI=h)` == 0 -> B16; `ai_human_leader_flag4(h)`: nonzero -> `sub_429750(h)` -> B16; zero: if `h[0x151] & 0x54` or `h[0x152] & 1` -> B16; `ai_event_cancel(ESI=0x59, h, h)`; `ECX,ESI = leaderFlag ? ([0x2797A90],[0x2797A8C]) : ([0x2797A98],[0x2797A94])`; `ai_event_post_random_delay(ECX=max, ESI=min, EDI=0, 0x59, h, h, 0xD, 0)`; `h[0x151] |= 0x40`.
**B16 switch 3 (0x42D951).** `idx = act - 0x78`, `> 0xB` -> exit. Table 0x42E608 (12 entries) -> per action:

| act | Target | Behaviour |
|---|---|---|
| 0x78 | 0x42D96D | `AL = sub_43a190(EAX=h, target, 0)`; `edi = {stale24 \| AL}` (`mov [esp+0x24],al; mov edi,[esp+0x24]` after two pushes, i.e. the low byte of the `[esp+0x1C]` slot is overwritten); `r = sub_55ae30(&h+0x10C4, edi, 0, 0, 0)`; `r == 0` -> the abort path of 0x42D324 (`object_teleport_prepare(h,0,0)`); else `inventory_equip_slot(&h+0x10C4, 0xC, edi, 0)`; if `[h+0x10CC] == 0` exit; `sub_42b3b0(h, target)`; return |
| 0x79, 0x7C | 0x42DA09 | `h[0x595] != 0` -> exit. If `inVeh == 0`: `lostSight2 != 0` -> `ai_order_set_action(h,0xD,0x7B)`, return; else `sub_409b30(ECX=h, target, 0)`; return. If `inVeh != 0` (0x42DA40): `if sub_429260(h)`: `act == 0x79` exit; `set_action(h,0xD,0x7D)`; return. Else (0x42DA68): `t=target; th=[t+0x44]`; if `th != [h+0x334]`: `r2 = weapon_info_ai_range_sq(EAX=winfo)` f32 `[esp+0x1C]`; `d2=vec3_distance_sq(h+0x14, t+0x14)`; `fcompp`: `d2 > r2` (`test ah,5; jnp`) -> SEND; (equal handle skips this test). Then `sub_453400(EAX=th, ECX=[h+0x44], 1)`; `esi=[t+0x44]; edi=[h+0x44]; if sub_453730(ESI,EDI) != 2` exit. If `act == 0x79`: `ai_event_cancel(ESI=0x48, h.handle, h.handle)`; `r = rand_int_range(ECX=0x9C4, ESI=0x5DC)`; `ai_event_post(ECX=0x48, EAX=r, EDI=&{target.handle}, h, h, 0xD, 0x79)` ; return. Else: if `h[0x151] & 0x3F` and `sub_42a780(EAX=h) == 0` exit; `if sub_9c5a60(h, target, winfo)` -> `set_action(h,0xD,0x7D)`, return; `sub_42b4b0(h)`; `ESI=h; if sub_42b680() == 0` exit; `sub_42aae0(EAX=target)`; return |
| 0x7A | 0x42D9CE | `h[0x591] != 0` -> exit. `inVeh` -> `sub_42b3b0(h, target)`; return. `lostSight2` -> `set_action(h,0xD,0x7B)`, return. Else `sub_409b30(ECX=h, target, 0)`; return |
| 0x7B | 0x42E587 | `lostSight2 != 0` -> exit; `sub_42b4b0(h)`; return |
| 0x7D | 0x42DB67 | `inVeh == 0`: `lostSight2 == 0` -> `sub_409b30(ECX=h, target, 0)`; return (0x42DA28); else `set_action(h,0xD,0x7B)`, return. `inVeh != 0` -> **AIM** (0x42DB8D) |
| 0x7E | 0x42E003 | if `item=[h+0x10CC] != 0 && inst=[item+0xC4] != 0 && inst[8] != 0 && weapon_info_is_firearm(EAX=inst[8]) && inVeh`: if `h[0x150] & 0x20` `sub_9c4ed0(ECX=h, EAX=&h+0x154)` else `weapon_fire(h, target, 0,0,0,0,0)`; if `byte [h+0x149] != 0xD` exit; re-run the same weapon test; if it fails -> F2; `h[0x150] & 0x60 != 0` exit; `winfo2[0x34] == 0x12` exit; `f = sub_9c5e80(EAX=target, h)` (ST0) vs 50.0 (qword `[0xE5C700]`): `fcomp; test ah,5; jp exit` (exit when `f >= 50.0`) . **F2 (0x42E0DA):** `r = rand_int_range(ECX=0x12C, ESI=0x32)` (EAX=r, ESI keeps 0x32); `ai_event_post(ECX=ESI+0x3D=0x6F, EAX=r (delay), EDI=0, h, h, 0xD, 0x7F)`; `ai_order_set_action(h, 0xD, 0x7F)`; return |
| 0x7F | 0x42E110 | same item/winfo/firearm test as 0x7E but any failure -> exit; if `h[0x150] & 0x20` `sub_9c4ed0(ECX=h, EAX=&h+0x154)`; return; else `weapon_fire(h, target, 0,0,0,0,0)`; return |
| 0x80 | 0x42E180 | `sub_9c4ef0(h)`; if `(h[0x103] & 1) && target != 0 && sub_41b9d0(ECX=h, [target+0x44], 0)` -> exit. `if sub_429260(h) && (h[0x150] & 4)` -> abort (0x42D322). `inVeh` -> `0x42E58E` (`sub_42b4b0(h)`; return); `lostSight2` -> `set_action(h,0xD,0x7B)`; return; else `sub_409b30(ECX=h, target, 0)`; return |
| 0x81, 0x82 | 0x42E1F4 | `inVeh == 0`: `lostSight2 == 0` -> 0x42D9F1 (`sub_409b30(ECX=h,target,0)`; return); else `set_action(h,0xD,0x7B)`; return. `inVeh != 0` (0x42E21A): if `h[0x151] & 0x80`: `if sub_42b600(EAX=target, ECX=h)` -> `ai_order_k06_start(ECX=target, h, [h+0xF8], [h+0xFC], 0)`; return; else `h[0x151] &= 0xFB`; `ESI=&h+0xF8; sub_436af0(-1, -1)`. Then (always) `ESI=target`; `if human_in_vehicle()` -> `ai_order_k1f_start(ECX=h, [target+0x44], 1)` != 0 -> exit. `target != 0`: `if sub_40e420(EAX=target, h, 0.85f)` and `ESI=h; sub_40d0f0(EAX=target)`: (`[h+0xEA8] != 0`: `sub_9b3680(EDX=h)` != 0 -> exit; `human_is_player_or_player_follower()` ? `net_send_human_pkt_10(h)` : `net_send_human_pkt_11(ECX=h,0)`; return; else: `[h+0x178] += trunc(1/60 * 1000.0)` via `fistp qword` with the control word forced to truncate (`or eax,0xC00`), `if [h+0x178] >= [0x2797748] (unsigned jb exit when less)` -> `ai_order_k05_start(ECX=h, EDX=target, 1)`; return). Else (any of the tests above failed, or `target == 0`): `[h+0x178] = 0`; `sub_43bcc0(h,0)` != 0 -> exit; `ch=[h+0x150]`; `ch & 0x20` -> `sub_42b3b0(h, EDI)` (EDI = target, 0 only in this case); return; `ch & 0x40` -> exit; `sub_453730(ESI=[target+0x44], EDI=[h+0x44])`: result 1 or 2 -> (0x42E3D1) if `[h+0x334] == ESI` -> (0x42E4AE) `act == 0x82` ? `sub_42b3b0(h,target)` : exit; else `d2 = vec3_distance_sq(h.pos, target.pos)` vs `r2 = weapon_info_ai_range_sq(winfo)` (`fcom; test ah,0x41; jne 0x42E418` when `d2 <= r2`... see note), else (result not 1/2) `sub_42bb40(EAX=h)`; return |
| 0x83 | 0x42E4D0 | `esi=[target+0x44]; edi=[h+0x44]; if sub_453730() == 2` -> `sub_42b3b0(h, target)` (0x42D9B9 shares the tail of 0x78); return. Else `sub_43bcc0(h,0)`; exit if `h[0x150] & 0x80`; exit if `!(h[0x152] & 0x10)`; `p=(h+0x164..16C)`; `d2 = dist_sq(p, target.pos)` vs 25.0 (qword): exit when `d2 <= 25.0`; `ai_event_cancel(ESI=0x12, h, h)`; `ai_event_post_random_delay(ECX=1000, ESI=0, EDI=0, 0x12, h, h, 0xD, 0x83)`; `h[0x150] |= 0x80`; return |

Note on the end of act 0x81/0x82 (0x42E3D1..0x42E4B5): when `d2 > r2` (`fcom st1; test ah,0x41; jne` not taken) falls to SEND via `fstp st0` (0x42E408); when `d2 <= r2` goes to 0x42E418: `ESI=target; EDI=0; if human_in_vehicle()` -> 0x42E457; else `if h[0x470] & 0x10` -> 0x42E457; `fcom qword [0xE5DC18]` (2.0) then `test ah,0x41; jp`: skip to 0x42E457 when `d2 > 2.0` or unordered; when `d2 <= 2.0` call `sub_409b30(ECX=h, 0, 0)` and exit if it returns nonzero. 0x42E457: if `act == 0x82 && winfo[0x34] != 4`: `x = sqrtf(d2)` (`sqrtf_wrapper` f32 arg), `y = sub_9c5dd0(EAX=h, x)` ST0, `y < 1.0` (`fcomp [0xDD518C]; test ah,1; jne exit`) -> exit; `sub_42b3b0(h, target)`; return. (Other acts exit through `fstp st0` at 0x42E5D3.)

**AIM block (0x42DB8D..0x42DF4A) for act 0x7D with `inVeh`.** FP-heavy; the algorithm:
 1. `aim = (0,0,0)`; `if h[0x150] & 0x20`: `aim = [h+0x154..0x15C]`; goto S2.
 2. Else bone transform on the target: `kind = winfo[0x34]`; `bone = byte [[target+0xD4]+8 + (kind==0x12 ? 0x57 : 0x54)]` (movsx); `rig_bone_transform(ECX=[[target+0x5E0]], bone, &target+0x14, &target+0x20, &aim, 0)` (the five pushes top-first are `(bone, &pos, &mat, &aim, 0)`; `&aim` is `lea ecx,[esp+0x58]` evaluated after the leading `push 0`, i.e. `[base+0x54..0x5C]`, the same slot as step 1's `aim`).
 3. Only `kind == 4`: `lostSight2 = !((h[0x150] >> 3) & 1)`; `dir = aim - h.pos` (f32 sub) -> `[esp+0x78..0x80]`; `len = vec3_normalize_in_place_ret_len(ESI=&dir, EAX=...)` (**XMM0**, stored `[esp+0x1C]` then reloaded f32 into `[esp+0x28]`); `m = max(winfo[0x234], winfo[0x238])` (`fld [+0x234]; fld [+0x238]; fcompp; test ah,5; jp` picks `[0x238]` when `[0x238] >= [0x234]`, else `[0x234]`); `f = lostSight2 ? 1.0 : 0.0` (`fld1`/`fldz`); `s = len/m + f`; call the **virtual method** `(*(**target))(ECX=target, stack (&velOut, -1))`: `eax=[target]` (vtable), `edx=[eax+0]`, pushes `-1` then `&velOut` (= `[base+0x60]`, the slot the muzzle uses later); `call edx` is a thiscall with no `add esp` after it, so the callee pops 8 (`ret 8`); velocity `v` is read back from `[base+0x60..0x68]` in x, y, z order; `vel = v * s` -> `[esp+0x48..0x50]`; if `!((h[0x110C] >> 9) & 1)`: `r = vec3_random_unit(ESI=&[esp+0x28])`; `sub_42eea0(EAX=&[esp+0x40], ECX=&[esp+0x7C], &[esp+0x28])`; `vec3_normalize_in_place(ESI=&[esp+0x3C])`; `scale = lostSight2 ? [0x2797A44] : rng_float_range([0x2797A48], [0x2797A4C])`; `vel += unit * scale` per component with f32 round trips; `aim.x = vel.x + aim.x` etc (`fld [0x48]; fadd [0x54]; fstp [0x3C]; ... [0x54]=[0x3C]`).
 4. S2 (0x42DE28): `human_attach_transform_slot0(EAX=h, &muzzle=[esp+0x60], &mat=[esp+0x84])` (cdecl, args pushed second-out first; its `add esp,8` is deferred, so the following `[esp+X]` reads inside the first instructions are 8 lower than they look): muzzle vec3 at `[0x60..0x68]`, a 3x3 matrix at `[0x84..0xA7]` whose third row `[0x9C..0xA4]` is the forward vector; `d = aim - muzzle` componentwise (`[0x54]-[0x60]`, `[0x58]-[0x64]`, `[0x5C]-[0x68]`) -> `[0x48..0x50]`; `len = sub_bde410(ESI=&d)` (**XMM0**, `movss [esp+0x28]`); if `len < 0.3` (`test ah,1; jne`) go to FIRE-CHECK; else copy the forward row `[0x9C..0xA4]` to `[0x3C..0x44]`, `sub_bde290(ESI=&that)` (normalises in place), `dot = d . fwd` accumulated `d.y*f.y + d.x*f.x + d.z*f.z` (order y, x, z; f32 stores); `fcomp qword [0xE5C2E0]` (0.8): `test ah,5; jnp exit` -> exit when `dot < 0.8` (or unordered).
 5. FIRE-CHECK (0x42DEE2): `q = [h+0xF8..0xFF]` as a qword; `(q >> 0x1D) & 7 == 3` (via `shrd ecx,edx,0x1D`): `f = sub_9642a0(ECX=h)` ST0; `f != 0.0` (`test ah,0x44; jp exit`) -> exit. Then `if (h[0x150] & 0x20) || winfo[0x34] == 4`: `weapon_fire(h, 0, &[esp+0x64]?, ...)`: the pushes are `0,0,0,0,&aim,0,ebx` (the `lea ecx,[esp+0x64]` is evaluated after four pushes, so it is `&[base+0x54]` = `aim`) -> top-first `(h, 0, &aim, 0, 0, 0, 0)`; else `weapon_fire(h, target, 0,0,0,0,0)`. Afterwards `if byte [h+0x149] != 0xD` exit; `item/inst` null -> `ai_order_set_action(h,0xD,0x81)`; return; `!(inst[0xC] & 1)` exit; `h[0x150] |= 8`; `t = winfo2[0xCC]`; switch 4 (table 0x42E638, `t > 3` skips): `t==0` -> `set_action(h,0xD,0x80)`; `t in 1..3` -> `set_action(h,0xD,0x7E)`. Then `if winfo[0x34] == 0x12`: `ai_event_cancel(ESI=0x6F, h, h)`; `ai_event_post_random_delay(ECX=[0x2797AA0], ESI=[0x2797A9C], EDI=0, 0x6F, h, h, 0xD, 0x7E)`; return.

### Risks (k0d)
1. **Uninitialised f32 local `[esp+0x1C]`** is read in B11 (`fld [esp+0x1C]; fadd 15.0`) and passed as a dword to `sub_42b140` in B14, and has its low byte overwritten in the act-0x78 case. The original uses whatever the previous stack contents were. A faithful port cannot reproduce arbitrary garbage; shadow-test with the original to see whether the value is constant in practice (probably a leftover from the caller `ai_order_update`) and read it from a `frame` array initialised from the real stack if needed.
2. One **virtual call** (`call edx` at 0x42DD16, vtable slot 0 of the target, ECX=this). Call it through the original object's vtable, do not reimplement.
3. **XMM0 returns** (`vec3_normalize_in_place_ret_len`, `sub_bde410`) mixed with x87 code: the Rust side must call them via the register shim and read XMM0 (`movss` follows each call immediately).
4. **x87 control word** changed with `fldcw` (truncate) around one `fistp qword`, then restored; do not leave it changed. The truncation of `(1/60)*1000.0` = 16.666668 gives 16.
5. ECX = `act` (CL) is held live from 0x42D951 across `sub_429260` (a leaf that preserves ECX); re-reading `[h+0x148]` is equivalent.
6. Stack junk: the abort jumps to 0x42D322/0x42D324 with un-popped argument pushes; irrelevant for a port.
7. ESI/EDI are reassigned freely (ESI = leader, then `h`, then `target`), and several callees (`ESI=h` with `sub_42b680`, `sub_42b140`, `sub_40d0f0`; `EDI=h` with `sub_438b30`, `sub_95e700`, `human_effective_leader`) read them implicitly.
8. No indexed or unbounded writes; the frame writes are fixed offsets below 0xA8. The one out-of-bound-looking `lea edi,[esp+0x78]` in the 0x79 event post passes the address of a local containing the target handle as the event payload.

### Split into hand-portable pieces
1. `k0d_gate(h)` = B0..B3 (pure; one `net_send_human_pkt`/teleport call on the false branch).
2. `k0d_switch1(h, leaderFlag)` = B6 (two cases).
3. `k0d_acquire(h) -> Option<target>` = B7, B8.
4. `k0d_flags(h, ...) -> (inVeh, facing, lostSight, lostSight2)` = B12, B13, B14 (the stateful bit twiddling; these reads/writes are exact and easy to shadow).
5. `k0d_events(h)` = B15 (calls only).
6. One function per switch-3 case; the single-call cases are 0x7A, 0x7B, 0x7F, 0x83. Cases 0x79/0x7C, 0x80, 0x81/0x82 are medium; 0x7D (AIM) and 0x7E (fire) are the hard ones.
7. `k0d_aim(h, target, winfo) -> aim` = AIM steps 1..3 (pure FP + two XMM0 helpers + the virtual call); `k0d_fire(h, target, aim)` = step 5.
Port order: B0..B3, B6, B12..B14, then 0x7B/0x7A/0x7F/0x83, 0x78, 0x79/0x7C, 0x80, 0x81/0x82, 0x7E, 0x7D.

---

# Group 2: navigation

Read first: doc 47 (structures: node reference, cell, chunk record, path request, route records) and `port_nav.rs`. Facts established here and needed by every function below:

## 2.0 Shared nav search state (all addresses absolute)

| Address | Meaning |
|---|---|
| `0x2E160BC` | u32 search generation. A cell's scratch record is valid only if `[cell+0x20] == [0x2E160BC]`. Incremented (and forced to 1 on wrap to 0) by `0xA0FFA0` and inline in `0xA14230` |
| `0x2E160C0` | u32 scratch record count (next free), capacity 0x1000; `0xA0FFE0` fails (AL=0) at 0x1000 |
| `0x2E160C4` | scratch records, **24 bytes** each, 0x1000 of them (ends exactly at `0x2E2E0C4`). Fields: `+0 f32 g`, `+4 f32 h`, `+8 vec3 entry position`, `+0x14 s8 edge index the cell was entered through (-1 none)`, `+0x15 u8 open (1) / closed (0)`, `+0x16 u16 heap index`. Selected by `u16 [cell+0x1E]`, i.e. `0x2E160C4 + idx*24` (the code computes it as `lea eax,[eax+eax*2]; lea eax,[eax*8+0x2E160C4]`) |
| `0x2E160B8` | byte "search active" flag (1 during a search, 0 after) |
| `0x2E2E0C4` | s32 open-heap count (1-based heap; array `0x2E2E0C8 + i*4` holds cell pointers; the root is index 0 in the code's `lea ebp,[ebx*4+0x2E2E0C8]` arithmetic with parent `(i-1)>>1`). Key = `rec.g + rec.h` of the cell's scratch record |
| `0x2529820` | visited-cell count (max 0x20) and `0x21FC1A0[32]` visited cell pointers; bit 0x10 of `[cell+0x1D]` marks "in visited list"; `0xA12AE0` clears all marks and the count |
| `0x21FC19C` | head of the free ring of path nodes (0x38 bytes: `+0 next, +4 prev, +8 vec3 pos, +0x14 node ref, +0x18..+0x2C zeroed floats, +0x30 -1, +0x34 u16 -1, +0x36 u8 -1, +0x37 u8 flags` where `|1` first node of a run, `|2` last, `|4`, `|8` = "needs re-plan"). `0xA159D0` pops one |
| `0x252A6D0` | the zero vec3 (also the "no start position" constant of `nav_path_request_init`) |
| `0xF7AAE8` | chunk record table (0x866 pointers); `0x4A9740(EAX=chunk index) -> chunk record`; `0x4A9A60(stack (&pos, &out))`+`0xA7C760` give the zone id for a position |

Cell (0x30 bytes) as used here: `+0/+4/+8` three `vec3*` vertex pointers, `+0xC/+0x10/+0x14` three **neighbour cell pointers** (edge i joins vertex i and i+1 mod 3), `+0x18` link refs, `+0x1C` link count, `+0x1D` flags (`1` = one-way-region bit used by the direction tests, `2` has link flags to test, `0x10` visited mark, `0x20` pair mark), `+0x1E` u16 scratch index, `+0x20` generation, `+0x28` u16 detour-table index (`0xFFFF` = none; `0xA0E510` reads `0x21F2C18 + idx*20`), `+0x2A` byte: bit k = edge k is closed to this search, `+0x2C` u16 chunk id.

Path node, request, waypoint (20 bytes: `pos vec3 @0, node ref @0xC, u32 0xFFFF @0x10`) and the 3 flag bytes `{+0, +1, +2}` (the `0x253B130` default block, `ebp+0x471` for humans) are as doc 47. Bits used below:
- flag byte 0: `0x20`, `0x10` (skip cells with link flag2 unless it is the origin cell), `0x08` (ignore the one-way-region test), `0x04`, `0x02`, `0x01`.
- flag byte 1: bit 6 (`0x40`) = "wide" (also passed to `nav_find_node_near_point`), bit 5 (`0x20`) = use the request radius as search budget, bit 4 (`0x10`) = "mark ends" (see `nav_path_mark_ends`), bit 0.

## 2.1 `nav_path_search` 0xA121B0 (the A* core; also called as "path build")

### Calling convention, frame
- **EAX = pointer to a 0x2C-byte output block** (becomes ESI), stack arg 1 = request pointer (`nav_path_request_init` layout, 0x28 bytes). Plain `ret`, caller pops 4. EBX, EBP, ESI, EDI saved/restored; EAX/ECX/EDX clobbered; returns nothing (read `[out+0x28]`).
- The function starts with `fldz` **before** `sub esp,0xB8`: x87 depth +1 on entry; that 0.0 is consumed at 0xA1227A (`fstp [esp+0x1C]`) or discarded (`fstp st0` at 0xA1226F / 0xA12219). Net x87 depth at every `ret` is 0.
- Frame 0xB8 + 4 pushes. Slots (relative to `[esp+X]` after the prologue; many are reused): `+0x16` byte `swapped` (initially request flag byte0 bit1 `mirror`), `+0x17` byte flag byte0 bit3, `+0x18` byte "wide" = flag byte1 bit6 (later overwritten by the edge cost f32 at +0x18), `+0x1C` f32 radius for the node lookup (0.0 or 0.425, constant `0xDD0C40`), `+0x20` dword goal node ref out (later: f32 budget), `+0x27` byte flag byte0 bit4, `+0x28` f32 temp, `+0x2C..+0x34` vec3 (goal position copy / "src" position of the loop), `+0x38` goal ref, `+0x44` start node ref out, `+0x48` f32 `h0` = result of `0xA11750`, `+0x4C..` entry position out of `0xA10420`, `+0x58..+0x60` start position copy, `+0x64` start ref, `+0x70..` ray-query block for `physics_raycast`.
- Output block (ESI): `+0` head pointer of the circular path-node list (0 = none), `+4` vec3 start position (EBP), `+0x10` start cell, `+0x14` vec3 goal position (EDI), `+0x20` goal cell, `+0x24` byte, `+0x28` **status**: 0 initial; **1** path built; **2** straight two-node path (set by `0xA120F0`, used when the mirror flag is set and a node lookup fails); **3** no start node; **4** no goal node; **5** start equals goal (nothing to do, list empty); **6** goal blocked by a detour; **7** one-way-region violation; **8** failure / no path / path nodes exhausted.

### Request fields read
`+0` unused here; `+4` waypoint array (uses waypoint 0 and 1: positions `+0x00/+0x14`, node refs `+0x0C/+0x20`); `+8` flag byte A (`bit1`->mirror, `bit2`->skip ray test, `bit3`, `bit4`, `bit6`), `+9` flag byte B (`bit0`, `bit5` radius budget, `bit6` wide), `+0x18` vec3 radius center, `+0x24` f32 radius.

### Callees
| Address | Name | Regs in | Stack args (top-first) | Returns |
|---|---|---|---|---|
| 0xA13860 | nav_closest_node_find | - | (&out point, node ref, &pos, wide-dword, &out node ref, 0, A.bit6-dword, f32 r), caller pops 0x20 | EAX = cell or 0 |
| 0xA120F0 | sub_a120f0 (straight 2-node path) | ESI=out, EAX=&wp0.pos, EDI=&wp1.pos | (wide) | - |
| 0x418540, 0x6FA330, 0x4080B0 | ray query ctor / `physics_raycast` / dtor | ECX=&query | (&hit, 1) for the cast | AL (hit) |
| 0xA0E510 | sub_a0e510 | EAX=goal cell, ECX=&goal pos | - | EAX |
| 0xA0E4E0 | sub_a0e4e0 | EAX=&start pos | (x) | AL |
| 0x5E6B20 | sub_5e6b20 | ECX=&goal pos, EDX=&start pos | - | AL (positions equal within tolerance) |
| 0xA05250 | nav_cell_to_node_ref | EAX=cell | - | EAX |
| 0xA04A10 | nav_link_flags_to_mask | EDX=&flags | - | EAX |
| 0xA04A30 | sub_a04a30 (direct link walk) | - | (&A{pos,ref,0,0}, &B{pos,ref,0,0}, mask) | AL |
| 0xA159D0 | nav_path_node_alloc | EAX=&pos | (ref, wide, flag8) caller pops 0xC (`flag8 != 0` sets `[node+0x37] \|= 8`) | EAX = node or 0 |
| 0xA15B00 | nav_path_free_all | EDI=&head slot | - | - |
| 0x4A9740 | sub_4a9740 | EAX=chunk index | - | EAX = chunk record |
| 0xA11750 | nav_cells_heuristic | - | (startcell, &startpos, goalcell, &goalpos, wide) caller pops 0x14 | ST0 |
| 0xA0FFA0 | nav_search_begin | - | - | - |
| 0xA04790 | nav_open_relax | EAX=cell, ECX=&position | (parent cell, f32 g, &goal pos) caller pops 0xC | AL |
| 0xA10080 | nav_open_list_pop | - | - | EAX = cell or 0 |
| 0xA02A90 | nav_cell_scratch_record | EAX=cell | - | EAX |
| 0xA049F0 | nav_cell_walkable | EAX=cell | - | AL |
| 0xA05460 | nav_cell_has_link_flag2 | EAX=cell | - | AL |
| 0xA055B0 | sub_a055b0 | - | (cell, &vec3) caller pops 8 | ST0 f32 |
| 0xA10420 | nav_tri_edge_closest_point (`sub_a10420`) | ECX=edge index k, **EDI=cell** (the current cell `cur`; it is `EDI` for the whole loop) | (&out vec3, &src vec3) caller pops 8 | - (out = closest point of edge k, vertices `v[k]`, `v[(k+1)%3]`, to src, via `0xBE0250`) |
| 0xA044F0 | sub_a044f0 (edge traversal cost) | EAX=cell | unswapped: EAX=nb, stack (&entry, cur, &src); swapped: EAX=cur, stack (&src, nb, &entry); caller pops 0xC | ST0 f32 |
| 0xBDEA30 / 0xBDE980 | vec3_distance_sq / vec3_distance | EAX,ECX / stack (&a,&b) | - | ST0 |
| 0xA10C10 | sub_a10c10 (path reconstruction) | EAX=&start pos, ECX=start cell | (out, wide) | - |

### Control flow
1. **Init (0xA121B0-0xA121E9).** Zero/0.0-fill the output block as listed. `wp = [req+4]`.
2. **Same position (0xA121EA-0xA1221B).** Compare `wp[0].pos` with `wp[1].pos` with three `fucompp` (`test ah,0x44; jp` = differs). All equal -> `status = 5`, return (after popping the entry `fldz`).
3. **Flags (0xA1222D-0xA12269).** `A=[req+8]`, `B=[req+9]`: `[0x20]=A.bit6`, `[0x16]=A.bit1`, `[0x17]=A.bit3`, `[0x27]=A.bit4`, `[0x18]=B.bit6`. `r = B.bit6 ? 0.0 : 0.425` (`[0x1C]`).
4. **Start node (0xA12277-0xA122F1).** `nav_closest_node_find(&out+4, wp0.ref, &wp0.pos, dword [0x18], &[0x44], 0, dword [0x20], r)` -> `[esi+0x10]`. If 0: if `mirror`: `sub_a120f0` straight path (it builds two nodes and sets `status = 2`) (EAX=`wp`, EDI=`wp+0x14`, push `B.bit6`) and return; else `status=3`, return.
5. **Goal node (0xA122F2-0xA1241C).** Same call with `(&out+0x14, wp1.ref, &wp1.pos, ..., &[0x20], 0, ...)` -> `[esi+0x20]`. If 0: if `!mirror`: `status=4`, return. Else if `!(A & 4)` do the ray test: `0x418540(ECX=&query)` ctor; a **vertical ray** from `(wp1.x, wp1.y + 200.0, wp1.z)` to `(wp1.x, wp1.y - 200.0, wp1.z)` (qword `[0xE5C638]`=200.0, `fadd`/`fsubr`, f32 stores into the query at `[0x70..0x84]`, query word `[0xC4]=0x5D`, `[0x3C]=0x80000000`), `physics_raycast(ECX=&query, &hit, 1)` (cdecl, pops 8), `sete bl`, `0x4080B0(ECX=&hit)` dtor; ray hit (`bl == 0`) -> `status=4`, return. Not hit, or A.bit2 set: straight path as in step 4.
6. **Blocked goal (0xA1241D-0xA1245E).** If `!(A & 1) && !(B & 1)`: `e = sub_a0e510(EAX=goalcell, ECX=&goalpos)`; if `e != 0 && !sub_a0e4e0(EAX=&startpos, e)` -> `status=6`, return.
7. **Positions equal (0xA1245E).** `sub_5e6b20(ECX=goalpos, EDX=startpos)` AL != 0 -> `status=5`, return (same as step 2, 0xA1221B).
8. **One-way-region test (0xA1246F).** If `!A.bit3 && !(startcell.f & 1) && (goalcell.f & 1)` -> `status=7`, return.
9. **Radius test (0xA12496).** `fldz; fcomp [req+0x24]; test ah,5; jp` skips when `0.0 >= radius`. Else `d2 = vec3_distance_sq(EAX=goalpos, ECX=req+0x18)`; `r2 = radius^2` (qword round trip, stored f32 `[0x28]`); `fcompp` (ST0=r2): if `r2 < d2` (jnp not taken... `jp` taken when `r2 >= d2` continues) -> `status=8`, return.
10. **Direct link attempt (0xA124E3-0xA12656).** Build `A={startpos, ref=nav_cell_to_node_ref(startcell), 0, 0}`, `B={goalpos, ref(goalcell), 0, 0}`, `mask=nav_link_flags_to_mask(EDX=req+8)`; `if sub_a04a30(&A, &B, mask)`: `out.head = 0`; allocate node 1 (`nav_path_node_alloc(EAX=&A.pos, A.ref, B.bit6, 0)`), insert it into the circular list at `[esi]`; allocate node 2 from B; insert; `status = (head != 0) ? 1 : 8` (`neg/sbb/and 0xFFFFFFF9/add 8`); if the second allocation fails after the first succeeded call `nav_path_free_all(EDI=esi)`; return.
11. **Direction swap (0xA12657).** `[0x16]` is reset to 0. If `!B.bit6`: `ca = sub_4a9740(EAX = startref>>20 [0x44])`; if `ca.flags(+0x1C) & 2` and `!(sub_4a9740(goalref>>20 [0x20]).flags & 2)`: swap start/goal positions (three dwords each, via locals `[0x48..0x50]`), swap `[esi+0x10]`<->`[esi+0x20]`, `[0x16]=1` ("swapped"). 
12. **Budget (0xA126D8).** `[0x20] = B.bit5 ? req.radius^2 (fmul st,st) : -1.0 (qword [0xE5C180])`. `h0 = nav_cells_heuristic(startcell, &startpos, goalcell, &goalpos, wide)` -> f32 `[0x48]`. `nav_search_begin()`. **The search is seeded at the goal cell**: `nav_open_relax(EAX=goalcell, ECX=&goalpos, 0 (parent), 0.0 (g), &startpos)`; `AL == 0` or `heap count <= 0` -> failure.
13. **Main loop (0xA1273C-0xA129AC).** `cur = nav_open_list_pop()`. `cur == startcell` -> **success** (0xA129B1): `sub_a10c10(EAX=&out+4, ECX=startcell, out, B.bit6)`; `status=1`; goto 15. `rec = nav_cell_scratch_record(cur)`; `src = (rec.edge >= 0 ? &rec.pos : &out.goalpos)` copied to `[0x2C..0x34]`; for `k = 0..2`: skip if `k == (s8)rec.edge`; skip if `cur[0x2A] & (1<<k)`; `nb = cur.nb[k]`, skip if 0; `nr = scratch(nb)`: skip if `nr != 0 && nr[0x15] == 0`; skip if `!nav_cell_walkable(nb)`; skip if `A.bit4 && nb != startcell && nav_cell_has_link_flag2(nb)`; direction test (skipped when `A.bit3`): not swapped: skip iff `(cur.f&1) && !(nb.f&1)`; swapped: skip iff `(nb.f&1) && !(cur.f&1)`; if budget `> 0.0`: `d = sub_a055b0(nb, req+0x18)`; skip if `budget < d`; `nav_tri_edge_closest_point(ECX=k, EDI=cur, &entry[0x58..], &src)`; `c = sub_a044f0(...)` (argument order depends on `swapped`, see table) stored f32 `[0x18]`; skip if `c < 0.0` (`fldz; fcomp c; test ah,0x41; je skip`); if `nb == startcell` `c += vec3_distance(&entry, &out.startpos)`; `g = rec.g + c` (f32); skip if `h0 < g` (`fcomp; test ah,5; jnp`: prune when the new cost exceeds `h0`); `if !nav_open_relax(EAX=nb, ECX=&entry, cur, g, &out.startpos)` -> failure. After the 3 neighbours: loop while heap count > 0.
14. **Failure (0xA128F7).** `status = 8`.
15. **Swap back and reverse (0xA12900-0xA12A93)** (also reached after success): if `[0x16]` (swapped): exchange start/goal positions and cells again and **reverse the circular doubly-linked path list in place** (the loop at 0xA1295D..0xA12A91, building the reversed list with `ebx` as new head and finally relinking `[esi]`).
16. **Epilogue (0xA12A93).** `byte [0x2E160B8] = 0` (search inactive) on every path that went through step 12; early exits (steps 2-11) return before this write.

### Risks
- x87 depth discipline (entry `fldz`), SSE-free. All compares use `fucompp`/`fcom`; pay attention to NaN polarity (cheat sheet).
- Reads the byte locals `[0x18]` and `[0x20]` as **dwords** when pushing them for `nav_closest_node_find` (`mov ecx,[esp+0x18]`): upper bytes are stale; the callee uses the low byte (it feeds `nav_find_node_near_point`, where the other callers normalise with `and edx,0xFFFFFF01`). Pass `b as u32`.
- Aliasing: `[esp+0x18]` is both the "wide" byte and the edge-cost float; `[esp+0x20]` is both the goal-ref out and the budget; port with a locals struct, not shared names.
- Writes: only the output block, the scratch table, the heap, the visited list and the path-node pool; no write is unbounded (scratch capped at 0x1000 by `nav_open_relax`, nodes by the ring).
- `physics_raycast` (Havok) is the only call that reaches outside nav memory (besides `nav_closest_node_find` -> `0x6FE800`).

### Split
1. `nav_path_prepare` = steps 1-9 (all early returns with a status code; pure except `nav_closest_node_find`, the ray test and the tolerance helpers).
2. `nav_path_direct_link` = step 10.
3. `nav_path_search_core` = steps 11-14 (the A* loop: keep as one function, but its inner neighbour test is a pure function of (cur, k, nb, flags) that can be unit-tested).
4. `nav_path_reverse_if_swapped` = step 15 (pure list manipulation, easy to test alone).

## 2.2 `nav_cells_heuristic` 0xA11750 (the "polygon cut" is not a cut: it is the A* distance bound)

- **Stack args (cdecl, caller pops 0x14): `(cellA, &posA, cellB, &posB, bool wide)` -> ST0 (f32 stored/reloaded).** Saves EBX, EBP, ESI, EDI; clobbers EAX, ECX, EDX. Frame 0x60.
- Helper `0xA103B0(ECX=&out vec3, EDI=cell, ESI=edge index)`: `out = (v[(i+1)%3] + v[i]) * 0.5` (qword constant `[0xE5BF70]`), i.e. the **midpoint of edge i** (idiv by 3 for the wrap). The loop at 0xA11760 fills `midA[0..2]` at `[esp+0x4C+12i]` and `midB[0..2]` at `[esp+0x28+12i]`.
- Three maxima of squared distances (each only over edges whose neighbour pointer `[cell+0xC+4i]` is non-null):
  - `M0 = max_i |posA - midA_i|^2` (blocks at 0xA11783-0xA118C1)
  - `M1 = max_i |posB - midB_i|^2` (0xA118C1-0xA119A0 and 0xA119A0-0xA11A0E)
  - `M2 = max over (i,j) with both edges open of |midA_i - midB_j|^2` (the `esi=3` loop at 0xA11A1D-0xA11B54)
  Each max is "`fcom; test ah,5; jnp keep-old`" i.e. replace when the new value is `>=` the old (ties and NaN-free values replace).
- `est = (sqrtf(M0) + sqrtf(M1) + sqrtf(M2)) * 1.5 + 100.0` (constants qword `[0xE5C408]`=1.5, `[0xE5C190]`=100.0; `sqrtf_wrapper` = `0x40EA20`, f32 arg on the stack, ST0 result; the first sum is carried as a qword `fstp qword [esp+0x24]`). Stored f32 `[esp+0x20]`.
- Clamp: `wide != 0` (`cmp byte [esp+0x84],0`): `min(est, 300.0f [0xE8E0D8])` via `est < cap ? est : cap`; `wide == 0`: `est >= 200.0 (qword [0xE5C638]) ? 200.0f ([0xDD0C4C]) : est`. Note the two clamps use different comparison constants/precision (f32 300.0 vs f64 200.0).
- Callers: `nav_path_search` (as the cost bound `h0`) and `nav_cells_reachable`. Pure FP, no globals, no writes outside its frame: **easiest to port and to test natively**. Port helpers: `edge_midpoint(cell, i)` (A103B0, 29 insns) and the three max loops. The compiler fully unrolled/hoisted the loops with partly register-resident operands (`st(2)..st(5)` hold vB[1], vB[2] coordinates in the M2 loop); the port should recompute them from memory, but round each product/sum through f32 exactly where the original does `fstp dword` (every `fsub dword`/`fmul st,st`/`fadd` result is stored f32 before the compare).

## 2.3 `nav_cells_reachable` 0xA14230

- **Stack args (cdecl, caller pops 0xC): `(cellA, cellB, flags*)`** with `flags` = the 3 flag bytes. Returns **AL** (1 = B reachable from A within the bound). Saves EBX, EBP, ESI, EDI; frame 0x40. Callers: 0x4204CF (flags block `0x253B130`), 0x43A113.
- Steps: `nav_tri_centroid(EDX=cell, ESI=&out)` for both cells (`cA` at `[esp+0x38]`, `cB` at `[esp+0x2C]`); `h0 = nav_cells_heuristic(cellA, &cA, cellB, &cB, (flags[1]>>6)&1)` -> f32 `[esp+0x28]`; **inline `nav_search_begin`** (generation++, wrap to 1, `[0x2E160C0]=0`, heap count 0, busy flag 1); byte `[esp+0x17]` result = 0; seed with `nav_open_relax(EAX=cellA, ECX=&cA, 0, 0.0, &cB)`; `AL == 0` or heap empty -> return 0.
- Loop: `cur = nav_open_list_pop()`; `cur == cellB` -> result 1 (0xA14494). `rec = scratch(cur)` or 0; `src = (rec.edge >= 0) ? &rec.pos : &cB` into `[esp+0x38..0x40]`; for `k=0..2`: skip if `k == (s8)rec.edge`; `nb = cur.nb[k]` (`edx = cur+0xC` walking), skip if 0; `nb == cellB` -> result 1; scratch of `nb` (only if its generation matches): skip if closed (`[+0x15] == 0`); skip if `nb.f & 0x10`; if `nb.f & 2 && !nav_cell_has_link_flag2(nb)` skip; if `flags[0] & 0x10 && nb != cellA && nav_cell_has_link_flag2(nb)` skip; if `!(flags[0] & 8) && (cur.f & 1) && !(nb.f & 1)` skip; `nav_tri_centroid(EDX=cur, ESI=&c2)`; `d = sqrtf(vec3_distance_sq_dup(EAX=&c2, ECX=&src))` (0x404A20); `g = rec.g + d`; **prune when `h0 <= g`** (`fld h0; fcomp g; test ah,0x41`... the code is `fld g; fld h0; fcomp st(1); test ah,0x41; jnp` -> skip when `h0 < g` or equal); `nav_open_relax(EAX=nb, ECX=&c2, cur, g, &cB)`, `AL == 0` -> return 0 (open list/scratch full).
- Epilogue clears `[0x2E160B8]` (busy) on every exit; the result byte is returned in AL (note the success path `mov byte [esp+0x17],1` at 0xA14494 falls into the shared exit at 0xA14499).
- Note the cost used (`dist(centroid(cur), src)`) is not the A* edge cost of `nav_path_search`; this routine is a cheaper bounded reachability test. Pure given the nav state; no writes outside nav scratch/heap.

## 2.4 `nav_human_path_resume` 0xA14560 and `nav_path_prepend_start` 0xA147E0

Both rebuild a human's path list incrementally; callers: `0x99C47F` and `0x99C115` (via `0x99C020`).

**`nav_human_path_resume(slot, human)`** cdecl, 2 stack args (`slot` = `u32*` holding the list head, `human`), AL = 1 if the planner finished a path, 0 otherwise. Frame 0x94; saves EBX, EBP, ESI, EDI.
1. `head = *slot`; `head == 0` -> return 0. `*slot = 0`.
2. Build a 2-waypoint request on the stack with `vector_constructor_iterator(arr, 0x14, 2, 0x99B090)`: wp0 = `{head.pos (+8..+0x10), head.ref (+0x14), 0xFFFF}`, wp1 = `{tail.pos (tail = head.prev, +8..), tail.ref, 0xFFFF}`; `cell0 = nav_node_ref_to_cell(EAX=head.ref)`, `cell1 = nav_node_ref_to_cell(EAX=tail.ref)`.
3. `nav_path_request_init(ECX=&req, EDX=&human+0x471, ESI=0x252A6D0, EDI=human+0x47C, 2, &wp, f32 [human+0x488])` (callee pops 0xC); `req[+9] |= 0x10`.
4. `0xA12AE0` clear visited marks. Map flags: `fl = ([flags0]&0x20 ? 1:0) | ([flags0]&0x10 ? 2:0) | ([flags0]&8 ? 4:0)` where `flags0 = [human+0x471]`.
5. Detour release: `hd = [human+0x58C]`; if `hd != -1` and `[0x2DEE0C0 + (hd&0xFFF)*40] == hd`: `nav_detour_release_hull(EDI = 0x2DEE0B8 + (hd&0xFFF)*40)`.
6. Free the old list: `*slot` (zero here) loop `nav_path_node_recycle(EAX=node, EDX=slot)`; `done=0`.
7. Loop: `r = sub_a0e840(&partial, &lastcell, &done_b, fl)` (cdecl 4 args; AL = finished). If finished -> 0xA14791: if `done_b` `sub_a11520(EAX=flags ptr, &partial, 1, 1)`; `*slot = partial`; `sub_a12ae0()`; return 1. Not finished: `*slot = partial`; `partial = 0`; if `lastcell.f & 0x10 == 0` and `lastcell != cell0 && lastcell != cell1` and visited count `< 0x20`: mark `0x10`, push on `0x21FC1A0`, `count++`; `nav_path_mark_ends(ESI=&partial, EDI=&req)`; `partial != 0` -> loop again. Otherwise: if `*slot != 0` `sub_a11520(EAX=flags ptr, slot, 1, 0)`; `sub_a12ae0()`; return 0.
Risks: `sub_a0e840` is an 802-insn state machine (not in scope); everything here is list/flag plumbing and is portable once its conventions are fixed. Callee-saved only; x87 touched only for the radius float.

**`nav_path_prepend_start(slot, startPos*, flags*, goalPos*, f32 radius)`** cdecl, 5 stack args (caller pops 0x14), no return value (EAX garbage). Frame 0x88; saves EBX, EBP, ESI, EDI. `[esp+0x14]` is a local list-head slot (the "detached tail" list), `[esp+0x2C..]` a 2-waypoint scratch request.
1. `ebp = esi = *slot`; `ebp == 0` -> return. Scan from `node = ebp`: while `byte [node+0x37] & 8 == 0`: `nav_node_ref_to_cell(EAX=[node+0x14])`; non-zero -> `node = node.next`; stop (return) when it wraps back to the head or hits null; zero -> `[node+0x37] |= 8` and use this node (0xA1482C).
2. Zone check: the scratch waypoints are initialised from the zero vec3 `0x252A6D0` (ref -1, 0xFFFF); `zone_id_at_position(&node.pos(+8), &tmp)` (cdecl, 2 args) -> `zone_remap_word_by_id(EAX=zone)` (u16); return if the result is `0xFFFF`, `>= 0x866`, or `[0xF7AAE8 + chunk*4] == 0` (chunk not loaded).
3. **Split the circular list at `node`** (0xA148B6-0xA148E3): `*slot == 0` -> `edi = 0`; `*slot == node` -> `edi = node; *slot = 0` (the whole list is detached); otherwise the part from `node` to the old tail becomes its own circular list with head `node` (`edi = node`) and the leading part stays in `*slot` (pointer swaps of `next`/`prev` exactly as in the listing: `edx=[ebp+4]; ecx=[eax+4]; [edx]=eax; ...; [ecx]=ebp`).
4. `cell = nav_find_node_near_point(EAX = node.pos, stack (wide=(flags[1]>>6)&1, direct=0, 1, &outpos))` (cdecl, pops 0x10); `cell == 0` -> recycle every node of the detached list (`nav_path_node_recycle(EAX=n, EDX=&[esp+0x14])` loop) and return. Else `ref = nav_cell_to_node_ref(EAX=cell)` -> `[esp+0x38]`.
5. If `*slot == 0`: a second `nav_find_node_near_point` on `startPos` (arg 2; zone id of it first); failing -> `nav_path_free_all(EDI=&[esp+0x14])`, return; success -> `ref2 = nav_cell_to_node_ref`. Else take the position / ref of the previous list's tail node (`[tail+8..0x10]`, `[tail+0x14]`).
6. `nav_path_request_init(ECX=&req, EDX=flags (arg 3), ESI=0x252A6D0, EDI=goalPos (arg 4), 2, &wp, f32 radius)` (callee pops 0xC); `req[+9] |= 0x10`; `nav_path_search(EAX=&out, &req)`; `esi = out.head`; `esi == 0` -> free the detached list and return. The node `[esi+4]` (the new list's tail) gets `[+0x37] |= 1` and then `|= 2`; since `req[+9]` bit 4 was just set, `sub_a11520(EAX=&req flags, &out, 0, 0)` runs and `esi = out.head` is reloaded; `esi == 0` -> free and return.
7. `nav_path_node_recycle(EAX=ebp(original node), EDX=&[esp+0x14])`; append the new list after the remaining `*slot` list (`*slot == 0` -> `*slot = esi`; else circular splice at 0xA14A79-0xA14A93); then if the detached list still has nodes (`[esp+0x14] != 0`): its head `[+0x37] |= 8` and it is appended after the result (0xA14A95-0xA14ACD, same splice shape).
Risks: pure pointer surgery on circular lists plus 5 callee calls. Port it with raw pointers and a shadow that walks both lists (`next` order, node `pos/ref/flags`) after the original and the port have run on a copy of the same list. The `[esp+0x14]`/`[esp+0x18]` slots are reused for the temporary head and the request scratch.

## 2.5 `nav_poly_link_match` 0xA16650 (the timesliced job stage)

- **cdecl, caller pops 8: `(item*, rec*)`**; no return value; saves EBX, EBP, ESI, EDI; frame 0x5C. Called only from `nav_timesliced_jobs_update` (0xA176EA) with `item = [owner+0x240]->array(+0x24) + index*12` and `rec = 0x21FF104[stage]` (the "stage function" in `frame.rs` is in fact a **chunk record pointer**: the code reads `rec+0x08`, `rec+0x28..0x3C`, `rec+0x14C`, `rec+0x150`).
- `item` (12 bytes): `+0` `vec3**` polygon vertex pointer array, `+4` `u32*` output slot (two dwords), `+8` u16 vertex count `n`.
- `rec`: `+0x08` u16 chunk id, `+0x28/+0x30/+0x34/+0x3C` f32 bounds (minX, minZ, maxX, maxZ), `+0x14C` link count, `+0x150` link array, **0x20-byte** links: `{vec2-ish start (x @+0, z @+8), end (x @+0xC, z @+0x14), +0x18 flags, +0x1C partner key (s32)}`.
- Steps:
  1. **Bounding box (0xA16665-0xA16895).** x/z min and max of the n vertices (y ignored), loop unrolled by 4 with a scalar tail; stored `[0x58]=minX,[0x5C]=minZ,[0x60]=maxX,[0x64]=maxZ`. Min updates when `new <= cur` (`test ah,0x41; je skip` -> replace on `<=`), max updates when `new >= cur` (`test ah,5; jnp skip`).
  2. **Reject (0xA16895-0xA16903).** Exit unless the box overlaps the chunk bounds: exit if `rec.maxX(0x34) < minX`, or `maxX < rec.minX(0x28)`, or `rec.maxZ(0x3C) < minZ`, or `maxZ < rec.minZ(0x30)` (each is `fcompp; test ah,5; jnp exit`).
  3. **Longest edge (0xA16909-0xA16C2D).** `best = -1.0`, `edge = (0,0)`: for each i in 0..n-1: `e = V[(i+1)%n] - V[i]` (x,z), `l2 = ex*ex + ez*ez` (the sum is built as `ex*ex` first then `+ ez*ez` via x87 stack ops; f32 round trip); replace `(best, edge)` when `l2 >= best` (ties replace). The `% n` uses `idiv` (not a mask).
  4. **Normal (0xA16C2F-0xA16C4B).** `nrm = normalize((edge.z, 0, -edge.x))` with `vec3_normalize_in_place` (ESI=&vec, in place).
  5. **Extent along the normal (0xA16C50-0xA16F0A).** `d_k = (V[k]-V[0]) . nrm` (full 3-vector dot, y included, order `y, x, z` accumulation) for k = 1..n-1 (unrolled by 4); `mx = max d_k`, `mn = min d_k` (first sets both). `width = mn - mx` -> f32 `[0x20]` (a **non-positive** number).
  6. **Centroid (0xA16F12-0xA17017).** sum of x and z over the n vertices (unrolled 4), times `1/n` (`fild n; fld1; fdivrp`), stored to `[0x58]/[0x60]` and copied to `[0x3C]/[0x40]` as the query point (vec2).
  7. **Radius (0xA17022-0xA17038).** `r = (width * 0.5 + 0.25)`; `r2 = r*r` (qword consts `[0xE5BF70]`=0.5, `[0xE5BFC8]`=0.25), f32 `[0x1C]`; `rec.linkcount <= 0` -> return.
  8. **Nearest link (0xA17040-0xA170EF).** For each link `L` (`ebx` walks the array, stride 0x20): `p0 = (L.x0, L.z0)`, `p1 = (L.x1, L.z1)` copied as f32 to `[0x24..0x30]`; skip if `p0 == p1` (both `fucompp ... test ah,0x44; jp/jnp` pairs: continues only when x or z differs); `q = closest_point_on_segment_xz(p0, p1, centroid)` with `0xBE6C50` (**registers: ECX=&p0, EDX=&p1, EDI=&centroid, ESI=&out vec2; 8 bytes written**); `d2 = (q.x - cx)^2 + (q.z - cz)^2`; **take it when `d2 < best`** (`fld best; fcomp d2` with `jne` on `<=` -> skip when `best <= d2`); `best` starts at `r2`; track the index in `esi` (-1 initially).
  9. **Write refs (0xA170F5-0xA1743C).** `esi < 0` -> return. `ref1 = (esi < count) ? (rec.id << 16) | ((esi+1) & 0xFFFF) : 0` written to `item.out[0]` (`[ [item+4] + 0 ]`). Let `key = L[esi].key (+0x1C)`. If `key >= 0` (`jl` not taken): search the same rec for another link `j != esi` with the same key: found -> `item.out[1] = (rec.id<<16) | ((j+1)&0xFFFF)` (`j >= count` impossible), not found -> return without writing `[1]`. If `key < 0`: `dir = normalize(L.end - L.start)` (3D), `len = normalize_ret_len(...)` (**XMM0**, `movss` after the call), then scan all other links `j != esi`: take the first whose endpoints match the reversed segment: first test `|L.start - M.start|^2` vs ... (`S`/`E` = start/end of `L`, `M` = candidate, all 3-D; `w2 = width*width` with `width = [esp+0x20]` from step 5): accept `M` when [`|S-M.start|^2 <= w2` and `|E-M.end|^2 <= w2`] (same direction, 0xA171FB..0xA1733E; each sum is `dx^2 + dy^2` first then `+ dz^2`, stored f32 in `[0x1C]`) **or** [`|S-M.end|^2 <= w2` and `|E-M.start|^2 <= w2`] (reversed, 0xA172A3..0xA1733E), i.e. skip when the first squared distance is `> w2` (`test ah,0x41; jp` for the first test, `je` for the second); then `Mdir = M.end - M.start`, `len2 = vec3_normalize_in_place_ret_len(ESI=&Mdir)` (**XMM0**), require `|len - len2| <= 0.25` ([0xE44850]; `len` is the first normalisation's result kept in `[0x24]`, combined as `fsubr qword`), and `|Mdir . dirL| >= 0.9994` (qword [0xE5CDA0], dot accumulated x, y, z, `jnp` skips when `|dot| < 0.9994`); on success writes `item.out[1] = (rec.id<<16)|(j+1)` provided the running index `[esp+0x10] < count` else writes `0` to `out[1]` (`xor eax,eax; mov [edx+4],eax`), and returns; if the scan ends without a match nothing is written to `out[1]`.
- Risks: (a) mixed SSE (`movss xmm0` after `0xBDE060`) and x87; (b) several `fcom` pairs with `jp` after `fstp st` pops: ties/NaN behave as in the cheat sheet; (c) `idiv` by `n` where `n` is read from a u16 (never 0 in practice; `n == 0` would fault: the original would crash, keep or guard identically); (d) unrolled loops visit vertices in an order that does not change the result except for float accumulation order in steps 5 and 6: **the sums must be accumulated in the same order** (vertex 0,1,2,... sequentially into one f32 accumulator; step 5 `max/min` order-independent, step 6 sequential); (e) no globals touched except through callees; the only memory written is `item.out[0..1]`.
- Pieces: `poly_bbox`, `longest_edge_normal`, `extent_along`, `centroid_xz`, `nearest_link` (needs `closest_point_xz`), `twin_link_search` (step 9 second half: the least obvious one, port last). The first five are pure functions over slices of f32 and can be fuzzed against the original natively.

## 2.6 `road_query_random_lane_setup` 0xA8A4F0

- **EAX = `vec3*` center, EDX = `int*` out; stack (cdecl, caller pops 0x18): `(f32 radius, u32 a, u32 b, u32 c, f32 d, f32 e)`** at `[esp+4..+0x18]`. Void. Saves/restores EDI only; clobbers EAX, ECX, EDX; x87 depth +0/-0.
- Writes `*edx = 0` first (always). `net = [0x2225B64]`; `net == 0` or `[net+0x84] == 0` -> return.
- Writes the query globals (the shared road-query block): `[0x2FA8D24]=edx(out)`, `[0x2FA8D28]=0`, `[0x2FA8CC8]=1`, `[0x2FA8CD8..0x2FA8CE0]=*center (3 dwords)`, `[0x2FA8CFC]=radius*radius` (f32, `fld st0; fmul st0,st0; fstp`), `[0x2FA8D14]=d`, `[0x2FA8D18]=e` (f32 round trips), `[0x2FA8CC0]=b`, `[0x2FA8CC4]=a`, `[0x2FA8D0C]=0`, `[0x2FA8D10]=c`, `byte [0x2FA8CCC]=1`, `[0x2FA8CD0]=-1`, `[0x2FA8CD4]=0`.
- Then `tree = [net+0x84]`; calls `0xBF77B0` with six pushes (top-first): `([tree+8], center, f32 radius (stored over a placeholder push), [tree], 0xA89A20 (road_query_tree_cb), 0xA89FC0 (road_query_cb_random_lane))`, `ret 0x18` (callee pops). It returns nothing used here.
- Its only caller is the wrapper at 0xA8BEF0 (`road_query_random_lane`: shuffles its own ECX/stack args into this layout and adds `0x18` itself). Pure global writes plus one call: **the easiest function in this document**; the callbacks (`0xA89A20`, `0xA89FC0`) are already ported and read exactly these globals.

---

## 2.7 Vehicle pathfind entry points (0xB27830 / 0xB27D90 / 0xB28700)

Common: all three have a `/GS` cookie (`[0xE80B50] ^ esp`), saved/restored EBX, ESI, EDI (EBP for the two ebp-framed ones), return **EAX** (0 = rejected/no route, 1 = local route installed, 2 = packet sent), cdecl with the caller popping. They share one **packet builder** (remote vehicles) and one **local path** (owned vehicles), selected by `human_owned_locally(EAX=vehicle)` (AL != 0 = local).

### Blocks shared by the local branches
- **Lane query block** (consumed by `road_find_nearest_lane(EAX=&block)`), 0x1A bytes: `+0 vec3* pos (=&veh+0x14)`, `+4 u32 mask (vehicle_nav_query_mask)`, `+8 u32 0x104`, `+0xC vec3* dir (=&veh+0x38)`, `+0x10 f32 200.0 ([0xDD0C4C])`, `+0x18 u8 q0`, `+0x19 u8 q1`; `ab5340(EAX=vehicle)` AL != 0 sets `q1 = 1`, else `q0 = 1`. (In `_to` it lives at `[esp+0x28]`, in `_nodes` at `[esp+0x20]`.)
- **Route query block** (for `sub_b26570(&block, 0)` / `sub_b269d0(&block)`): `+0 count`, `+4 nodes`, `+8 u8`, `+0xC dword (lane or 0)`, `+0x10 u8`, `+0x14 f32 ([0xE38AE0] or the float arg when >= 0.0)`, `+0x18 mask`, `+0x1C u8 0`, `+0x20 dword 0`; the callee fills a packed tail at `+0x1C` (byte flag) / `+0x1D` (unaligned dword) / `+0x20` (dword).
- **Goal block** (`vehicle_ai_goal_install(EAX=&goal, ai, 1, u16 as dword)`), 0x34 bytes: `+0 u8 flags`, `+4 vec3 pos`, `+0x10 f32 [0xE38AE0]`, `+0x18 f32 [0xE8725C]`, `+0x24 u8 0`, `+0x28/+0x2C/+0x30 f32 0.0`. `_to`: flags `= (a3 & 1) | 4`; `_nodes`: `sub_459110(EAX=&goal)` pre-initialises, then flags `= (flags & 0xFE) | (a4 & 1) | 4`; `_navmesh`: flags `= a4 & 1` only (no `|4`). The vec3 is the end of the spline route (`spline_closest_point` output) in `_to`/`_nodes`, and the last node's position in `_navmesh`.

### Packet format (remote branch)
Buffer on the stack at `[esp+0x68]` (to), `[esp+0x80]` (nodes), `[esp+0x60]` (navmesh). Layout, `di` = write offset (u16 arithmetic; `movzx` before each store):
```
buf[0]   = 0x6A                      (net message id)
di       = ([0xE8BBFC] != 0) ? 4 : 3 (one extra header byte when that global byte is set)
buf[di++] = 0x1C                     (veh_pkt sub-command)
buf[di++] = type                     (0 pathfind_to, 1 pathfind_nodes, 2 pathfind_navmesh)
u16 buf[di] = last u16 argument; di += 2
u16 buf[di] = vehicle net id; di += 2     (= pool obj -> [obj+0xB0] -> u16 [+2]; falls back to u16 [vehicle+0x44]; 0xFFFF when vehicle == 0)
u32 buf[di] = count; di += 4
u8  buf[di] = flags; di += 1
[type 0 and 1: f32 buf[di] = float arg; di += 4]
per node: 3 x f32 (x,y,z) = 12 bytes (type 2: from [ptr+0x1C], [ptr+0x20], [ptr+0x24] of each node pointer)
u16 buf[1] = di - 3                  (written once at the end)
conn = vehicle ? ( [vehicle+0xB0] ? session_find_conn_by_id(EAX=[0x2528C14], DL=byte [[vehicle+0xB0]]) : 0 ) : 0
net_send_packet(&buf, di, 0, 0)  with ESI = conn         (cdecl, caller pops 0x10)
return 2
```
The vehicle net id helper is identical to `net_ref_id` in `port_ai.rs`. The node copy is an unrolled loop of 4 (with a scalar tail); the order of stores is simply sequential, so a plain loop is equivalent. The buffer is 0x578 - 0x68 = 1,296 bytes (to) at most; max payload = 4 + 2 + 2 + 2 + 4 + 1 + 4 + 25*12 = 319 bytes: bounded by the `count <= 0x19` check (so no overflow).

### 2.7.1 `vehicle_ai_pathfind_to` 0xB27830
- **ECX = vehicle**; stack (cdecl, caller pops 0x1C): `(vec3* nodes, int count, bool a3, bool a4, bool a5, f32 a6, u16 a7)`. `count > 0x19` -> return 0 (jump to the common epilogue).
- Frame 0x578 + 4 pushes (EBX, EBP, ESI, EDI): `[esp+0x14]` = vehicle copy, `[esp+0x1C]` = nodes, `[esp+0x20]` temp/lane, `[esp+0x1B]` byte `usedClosest`, `[esp+0x28..0x48]` lane-query block, `[esp+0x5C..0x64]` vec3 out of `spline_closest_point`, `[esp+0x68..]` packet buffer.
- Remote: packet type 0, flags = `(a3 != 0) | (a4 != 0 ? 2 : 0) | (a5 != 0 ? 4 : 0)` (`setne`; `neg al; sbb al,al; and 2`), float = `a6`, u16 = `a7`, nodes are `vec3[count]` (12-byte stride, read from `nodes+0, +4, +8`).
- Local (0xB27B4D): `m = vehicle_nav_query_mask(EAX=vehicle)` (ported); `r = sub_b21f70(ECX=vehicle)` (AL/EAX; `r != 0` skips the lane lookup).
  1. `r == 0`: query block `{pos=&veh+0x14, dir=&veh+0x38, 0x104, mask=m, f32 200.0 [0xDD0C4C], bytes q40/q41=0}`; `ab5340(EAX=vehicle)` AL ? `q41=1` : `q40=1`; `lane = road_find_nearest_lane(EAX=&block)`; `lane != 0` -> use it. Else `sub_b264c0(EAX=&veh+0x14, EDX=m)` AL != 0 -> use `lane = 0`. Else `spline = road_find_closest_lane_point(ECX=m, EDX=0, &veh+0x14, f32 [0xE1D904])` (cdecl 2 args), `usedClosest = 1`; `spline == 0` -> return 0; `spline_eval_position(EAX=spline, ESI=nodes (writes nodes[0]), f32 0.5)` (callee pops 4).
  2. `r != 0` or no lane: `nodes[0] = vehicle.pos` (3 dwords from `+0x14`).
  3. Route query block at `[esp+0x28]`: `[0x28]=count, [0x2C]=nodes, byte [0x30]=a4, [0x34]=lane/ecx, byte [0x38]=a5, f32 [0x3C] = (a6 >= 0.0 ? a6 : [0xE38AE0])`, `[0x40]=m`, byte `[0x44]=0`, `[0x48]=0`; `route = sub_b26570(&block, 0)` (cdecl 2); `route == 0` -> return 0.
  4. `spline_closest_point(EDI=[[route+4]+0xC], EAX=&nodes[count-1], ESI=&out)` (ST0 popped) -> `out` at `[esp+0x5C]`. `ai = vehicle + 0x8554`. `usedClosest ? vehicle_ai_order_0d(ECX=route, ai, 0) : vehicle_ai_order_09(ECX=route, ESI=[route+0xC], ai)`; in the `order_09` branch, if `byte [esp+0x44] != 0`: `sub_b29940(dword [esp+0x48], esi, dword [esp+0x45])` (top-first, cdecl 3 args; the second member is an **unaligned dword at +0x45** overlapping the dword at +0x48: a packed struct the route builder fills).
  5. Goal block at `[esp+0x28]` (`vehicle_ai_goal_install`): `[0x2C..0x34]=out`, f32 `[0x38]=[0xE38AE0]`, f32 `[0x40]=[0xE8725C]`, byte `[0x4C]=0`, f32 `[0x50],[0x54],[0x58]=0.0`, byte `[0x28] = (a3 & 1) | 4`; `vehicle_ai_goal_install(EAX=&goal, ai, 1, dword a7)` (`mov edx,[esp+0x5A4]`: a **dword** read of a u16 arg; the upper half is whatever the caller pushed, normally 0) cdecl caller pops 0xC. Return 1.

### 2.7.2 `vehicle_ai_pathfind_nodes` 0xB27D90
- **ECX = vec3* nodes** (not the vehicle); stack (cdecl, caller pops 0x24): `(vehicle*, int count, bool a3, bool a4, bool a5, bool a6, bool a7, f32 a8, u16 a9)`. Valid count is **2..0x19** (`lea ecx,[esi-2]; cmp ecx,0x17; ja fail`). ebp-framed (`and esp,-8`, `sub 0x594`); the cookie is at `[esp+0x590]` and the failure path reads it at `[esp+0x59C]` (three pushes deeper) - both are the same slot.
- Remote: type **1**, u16 = `a9`, flags = `(a3 ? 2 : 0) | (a4 != 0) | (a5 ? 4 : 0) | (a6 ? 0x10 : 0) | (a7 ? 8 : 0)`, float `a8`, nodes `vec3[count]`.
- Local (0xB28110): `m = vehicle_nav_query_mask(vehicle)`; if `a7 == 0`: `sub_b264c0(EAX=&veh+0x14, EDX=m)` AL != 0 -> `nodes[0] = vehicle.pos` (0xB281A7). Otherwise (or when `sub_b264c0` is 0): query block `{pos=&veh+0x14, dir=&veh+0x38, 0x104, m, 200.0, q38/q39 via ab5340}`; `lane = road_find_nearest_lane(EAX=&block)`; `lane == 0` -> return 0; `spline_eval_position(EAX=lane, ESI=nodes, 0.5f)`. Route block at `[esp+0x40]`: `[0x40]=count, [0x44]=nodes, byte [0x48]=a3, [0x4C]=0, byte [0x50]=a5, f32 [0x54]=[0xE38AE0] (or `a8` when `a8 >= 0.0`), [0x58]=m | (byte [veh+0x8566] & 0x20 ? 0x8000 : 0), byte [0x5C]=0, [0x60]=0`; `route = a6 ? sub_b269d0(&block) (1 arg) : sub_b26570(&block, 0)`; `route == 0` -> return 0. `spline_closest_point(EDI=[[route+4]+0xC], EAX=&nodes[count-1], ESI=&out(esp+0x74))`; `vehicle_ai_order_0e(ECX=&tmp, ai)` with tmp = `{byte 0, [+4]=1, [+8]=0, [+0xC]=0, [+0x10]=route}`; if `byte [esp+0x5C] != 0`: `sub_b29940(dword [esp+0x60], [route+0xC], dword [esp+0x5D])` (top-first; same packed layout as in `_to`). Then `sub_459110(EAX=&goal)` initialises the goal block (`[esp+0x40]`), `goal.pos = out`, `goal.byte0 = (goal.byte0 & 0xFE) | (a4 & 1) | 4`; `vehicle_ai_goal_install(EAX=&goal, ai, 1, a9)`. Return 1.

### 2.7.3 `vehicle_ai_pathfind_navmesh` 0xB28700 (csv note "ECX = vehicle" is wrong)
- **ECX = `u32* nodePtrs`** (array of `count` pointers to path nodes; positions are read at `[ptr+0x1C], [ptr+0x20], [ptr+0x24]`); stack (cdecl, caller pops 0x18): `(vehicle*, int count, bool a3, bool a4, bool a5, u16 a6)`. `count > 0x19` -> return 0 (here the early return does the cookie check + `pop ebp; ret` without restoring ESP through EBP: the sequence `mov esp,ebp` precedes `pop ebp`).
- Remote: type **2**, u16 = `a6`, flags = `(a3 ? 0x20 : 0) | (a4 != 0) | (a5 ? 4 : 0)`; nodes are the three floats of each node pointer (unrolled by 4 + tail).
- Local (0xB28A1D): `opts = {byte 0, dword (a3 != 0), byte a5, f32 0.0}` at `[esp+0x1C]`; `vehicle_ai_order_05(ai = vehicle+0x8554, ECX... )` is called as cdecl `(ai, nodePtrs, count, &opts)` (**no register arguments**; pushes `edx(&opts), esi(count), edi(nodePtrs), ebx(ai)`). Goal block at `[esp+0x40]` from the last node pointer `p = nodePtrs[count-1]`: `[0x40..0x48] = p[+0x1C..0x24]` (3 dwords), f32 `[0x4C]=[0xE38AE0]`, f32 `[0x54]=[0xE8725C]`, f32 `[0x64],[0x68],[0x6C]=0.0`, byte `[0x60]=0`, byte `[0x48]... goal.byte0 = (0 ^ a4) & 1` (i.e. `a4 & 1`; no `|4` here); `vehicle_ai_goal_install(EAX=&goal, ai, 1, a6)`. Return 1. 
- This is the simplest of the three (301 instructions) and has no Havok/road lookups.

### Risks (vehicle)
- The ECX/stack-argument roles differ per function (vehicle in ECX for `_to`, nodes in ECX for `_nodes` and `_navmesh`); the three callers pass them accordingly (call sites `0x541A93`, `0x541AB6`, `0xA4C57B`). Get it from this section, not from the CSV.
- `a7` of `_to` and `a9` of `_nodes` are u16 arguments pushed as dwords and re-read as a dword (`mov edx,[...]`) when forwarded to `vehicle_ai_goal_install`; the callers zero-extend (`movzx eax,ax; push eax`), so forward `a as u32`.
- Remote branch requires `net_send_packet`/`session_find_conn_by_id` register conventions: `session_find_conn_by_id(EAX=[0x2528C14], DL=conn id)` -> EAX; `net_send_packet(buf, len, 0, 0)` with **ESI = connection**. The packet length `di` is a u16 and the u16 length field is `di - 3` regardless of the extra header byte.
- Float stores into the packet go through `fld dword`/`fstp dword` temporaries, which is bit-exact for non-signalling values; a signalling NaN would be quieted: copy with integer moves only if the shadow shows mismatches (the original does `fld/fstp`, so NaN payload bit 22 gets set).
- `/GS` cookie: the check is `call 0xC9604F` with ECX = cookie ^ esp; a naked-thunk port must not corrupt the original's cookie. A Rust port needs no cookie.

### Split
- Packet builder: `build_pathfind_packet(kind, vehicle, ...)` shared by all three (pure, byte-exact, easy to unit-test and to shadow by comparing `net_send_packet` buffers).
- `local_pathfind_to`, `local_pathfind_nodes`, `local_pathfind_navmesh` call only already-named originals (`vehicle_nav_query_mask`, `road_find_*`, `spline_*`, `vehicle_ai_order_*`, `vehicle_ai_goal_install`); use the recording-stub `seq_port` shadow.
- Port order: `navmesh` (smallest), `nodes`, `to`.

---

# 3. Memory-peek differential test harness for the navigation group

## 3.1 Why the existing rigs are not enough
`port_big_difftest.rs` maps the exe at 0x400000, starts every `.data/.bss/arena` page as `PAGE_NOACCESS` and fills it on first touch from a hash of `(scenario, address)`. That is fine for code that treats memory as numbers. The nav functions chase **typed pointers** (chunk table -> chunk record -> cell array -> cell -> vertex / neighbour / scratch index) and a hash-filled world turns the first `nav_node_ref_to_cell` into a wild read or an abort. They also keep **state across calls** (generation counter, scratch table, open heap, visited list, path-node ring). So the harness needs a *structured* world behind the same lazy-page mechanism: "memory-peek" = the first touch of a page is answered by a `World` object that knows what that address means, instead of by a hash.

Keep from `port_big_difftest.rs`: exe image mapped at its real addresses, callee taps (logger + trampoline), the same emulated stack for original and port, vectored-exception page filling, comparison of every page written, call logs, x87 stack and xmm0. Add:
1. `World::peek(addr) -> Option<[u8; 4096]>`: asks, in order, (a) the synthetic nav world (below), (b) a snapshot file, (c) the old hash filler (for unrelated pages). A page both runs touch is generated once, cached by address, and handed identically to the original and to the port.
2. A **reset-to-checkpoint** between the two runs: copy the written pages back from a pristine copy (the existing harness already compares "every page written"; the nav search mutates cell flags `+0x1D`, `+0x1E`, `+0x20`, the scratch table and the heap, so the pristine copy must include them).
3. **Fakes (shared by both runs, installed through the callee tap)** for everything that reaches outside nav memory:
   - `0xA13860` (`nav_closest_node_find`) and `0xA136E0` (`nav_find_node_near_point`): both end in Havok (`0x6FE800`) and box tests (`0xA10460`). Replace with a brute-force point-in-triangle search over the synthetic mesh returning `(cell, snapped point, node ref)`; deterministic and identical on both sides. (To test those two functions themselves later, run them against the Havok-free snapshot path instead.)
   - `0x6FA330` (`physics_raycast`): scenario flag "hit / no hit"; `0x418540` / `0x4080B0` (query ctor / dtor): leave original.
   - `0x4A9A60` + `0xA7C760` (zone lookup): return the chunk id of the cell grid.
   - `0xA0E840` (the 802-instruction human planner state machine) stays original but its own callees are tapped.
   - Network (`0x814670` `net_send_packet`, `0x80FE80` `session_find_conn_by_id`) and `0x806720` (`human_owned_locally`): fakes that record their arguments / return a scenario value; this is how the vehicle packet builders are compared byte for byte.
   - `__security_check_cookie` (`0xC9604F`): tap and ignore; `[0xE80B50]` is fixed to a constant so the cookie XOR is reproducible.
4. **x87/SSE state**: before each run set the x87 control word to what the game thread uses (read it once from a live thread with `fnstcw`; the ports assume 53-bit precision) and MXCSR to the default; after each run compare the x87 TOP (stack depth must equal the expected depth: 0 for all functions here, except `0xA11750` and `0xA14230`'s callee `sqrtf`, which return ST0) and XMM0 for the functions that return it indirectly.

## 3.2 World state that has to be mapped, per function

Common to every search (`0xA121B0`, `0xA14230`, `0xA14560`, `0xA147E0`):

| Region | Size | Content |
|---|---|---|
| `0xF7AAE8` | 0x2198 | chunk record table (0x866 pointers); non-null only for the test chunks |
| chunk record(s) | >= 0x260 each | `+0x08` u16 id, `+0x1C` flags (`& 3` must be 0 for lookups to succeed; bit `2` also selects the swap in `nav_path_search`), `+0x14C` link count / `+0x150` link array (0x20-byte links), `+0x240` -> navmesh array object (kind 0), `+0x248` -> array object (kind 1, may be empty), `+0x28..0x3C` bounds |
| array object | 0x28 | `+0x10` u16 count, `+0x14` cell array pointer, `+0x20` u16 job flag |
| cells | 0x30 each | vertex pointers, neighbour pointers (consistent: if A's edge i points to B then one of B's edges points to A), `+0x1C/0x1D` count/flags, `+0x1E/+0x20` scratch index/generation (start at generation 0), `+0x2A` closed-edge mask, `+0x2C` chunk id; `nav_cell_to_node_ref` / `nav_cell_locate` need the cell to lie inside one of the two arrays |
| vertex pool | 12 bytes per vertex | vec3 floats (include y values that differ so the 3-D dot products in `0xA16650` matter) |
| link tables | 0x20 each | for `nav_cell_has_link_flag2` / `nav_cell_walkable` and for `0xA16650` |
| `0x2E160B8..0x2E2E0C8+0x4000` | about 0x1F000 | the scratch/heap block of section 2.0 (zero-filled is a valid start; the generation counter at `0x2E160BC` should also be tested near `0xFFFFFFFF` to cover the wrap to 1) |
| `0x2529820` + `0x21FC1A0` | 4 + 0x80 | visited count / list |
| `0x21FC19C` + node pool | 0x38 per node, >= 64 nodes | build a free ring (circular list through `+0/+4`) in the arena |
| `0x252A6D0`, `0x253B130` | 0xC / 3 | zero vec3, default flag bytes (both are in the exe's data; keep the real bytes) |
| `0x2DEE0B8..` | 40 x 0x1000 | detour table (for `0xA14560`; entry `+8` handle must be -1 or match) |
| `0x2149C64` | 16 x 0x1000 | entity pool, only for the AI ports and the vehicle ports |

Per function, the extra requirements:
- `0xA8A4F0`: `[0x2225B64]` = a block whose `+0x84` points to a tree object `{+0 ptr, +8 ptr}`; tap `0xBF77B0` (log its 6 stack args) and compare the 16 globals at `0x2FA8CC0..0x2FA8D28` plus `*edx`. No navmesh needed.
- `0xA11750`: two cells (vertex pointers, neighbour pointers, any combination of null/non-null neighbours: all 8 x 8 patterns) and two positions; compare ST0 bits. No globals.
- `0xA16650`: item (12 bytes), vertex-pointer array, vertices, one chunk record with bounds and 0x20-byte links; compare the 8 bytes at `item.out` and the whole link table (must be unchanged). Fuzz: polygons with 3..40 vertices (the unroll boundaries 4, 5, 8, 9), degenerate zero-length links, links whose key is `-1` with and without a reversed twin, bounds exactly touching the polygon box.
- `0xA14230`, `0xA121B0`: a mesh of at least 6 x 6 quads (72 triangles) so searches have alternatives, one-way-region bits, closed-edge bits, link-flag cells, and a scenario knob that limits the scratch capacity (preload `[0x2E160C0] = 0xFF8`) to hit the "scratch full" returns.
- `0xA14560` / `0xA147E0`: a human block (only `+0x471..0x473` flag bytes, `+0x47C` vec3, `+0x488` f32, `+0x58C` detour handle are read), a pre-built circular path list (use `nav_path_search` itself to create it on the original), and the zone fakes.
- Vehicle: `[0xE8BBFC]` (header byte switch), `[0x2528C14]`, vehicle block with `+0x14` pos, `+0x38` dir, `+0x44` handle, `+0xB0` -> `{u8 conn at +0, u16 id at +2}`, `+0x8554` AI block, byte `+0x8566`; entity pool entry for the vehicle handle; the callee taps of section 2.7 (`road_find_*`, `spline_*`, `vehicle_ai_*`) return scripted values and log their arguments; compare the packet buffer handed to `net_send_packet`, EAX, `nodes[0]`, the logs.

## 3.3 Snapshot mode ("peek the live game")
The structured generator proves equivalence on mazes we construct; real data has shapes we will not think of. A second mode feeds the same harness from the running game:
1. A debug hook (later; not part of this task) walks `0xF7AAE8`, and for every loaded chunk record writes out: the record (0x260 bytes), both array objects, the cell arrays, every page that a cell vertex / neighbour / link pointer lands in, plus the scratch/heap/visited/ring block. File format: `(address, 4096 bytes)` pairs. Loading it in the harness is exactly "answer page faults from the file".
2. A second hook records requests: at entry to `nav_path_search` / `nav_cells_reachable` / `nav_human_path_resume` / `nav_path_prepend_start` / `vehicle_ai_pathfind_*`, copy the register block, the stack args and the first 0x100 bytes behind each pointer argument; the harness replays them against the snapshot with the original, stores the original's result as the golden file, and then checks the port against it.
3. The snapshot must be taken with the search state idle (`[0x2E160B8] == 0`) and with the frame's nav jobs finished; otherwise the generation/heap content is mid-flight.

## 3.4 What to compare
- `0xA121B0`: the 0x2C-byte output block, the path-node ring (every node, both the list handed out and the free ring), scratch records `0..[0x2E160C0]`, heap count + array, visited count + list, every cell flag byte `+0x1D` and the `+0x1E/+0x20` fields of the touched cells, `[0x2E160B8]`, `[0x2E160BC]`, and the ordered log of tapped calls (`0xA04790` args: cell, position, parent, g bits, goal).
- `0xA14230`: AL + the same state blocks (the search leaves the generation incremented and the scratch populated).
- `0xA11750`: ST0 as bits, nothing else. `0xA16650`: the 8 output bytes only.
- Vehicle functions: EAX, the exact packet buffer (all `di` bytes) and length passed to `net_send_packet`, ESI at that call, the tapped call logs with arguments (including the bits of every f32).
- Everything: ESP balance (stack args popped by the right party), EBX/ESI/EDI/EBP preserved, x87 depth.

## 3.5 Fuzz inputs that matter
Random start/goal inside the mesh; start == goal; goal on the mesh border; goal outside every cell (exercises the raycast fake both ways); all 256 values of the two flag bytes (bits above 0x70 are ignored by the code, bits 0x10, 0x08, 0x04, 0x02, 0x01 each change a branch); NaN and infinity coordinates (polarity of every `jp/jnp` listed in section 0 is exercised only by NaN); coordinates around +-1e6 (f32 round trips); `radius` = 0, negative, tiny, huge; swapped (chunk flag 2 on the start chunk only); closed-edge masks; cells with 0, 1, 2, 3 non-null neighbours; scratch full; heap duplicates (same cell pushed twice with different `g`).

## 3.6 Order of work
1. Port and test the pure/small helpers first because everything else calls them: `nav_tri_edge_midpoint` (0xA103B0), `nav_open_list_pop` (0xA10080), `nav_open_list_sift_up` (0xA10260, `nav_open_list_sift_down` 0xA10110 is already named), `nav_open_push_new` (0xA0FFE0), `nav_open_relax` (0xA04790), `nav_search_begin` (0xA0FFA0), `nav_visited_clear` (0xA12AE0), `vec2_closest_point_on_segment_xz` (0xBE6C50). With these ported (or just tapped), the harness for 0xA14230 / 0xA121B0 needs only the mesh.
2. `road_query_random_lane_setup`, `nav_cells_heuristic`, `nav_poly_link_match` (pure).
3. `nav_cells_reachable`, then `nav_path_search` (the A* core), then the two list wrappers (`nav_path_prepend_start`, `nav_human_path_resume`).
4. Vehicle packet builder + the three entry points.

---

# 4. Consolidated risk table

| Risk | Where | Handling |
|---|---|---|
| Implicit register arguments (ESI / EDI / EAX only) | k00 (about 20 callees), k0d (about 15), `nav_path_search` callees, `be6c50` (4 regs) | always use the register-setting call shim; never "tidy" a call into a plain function |
| EDX / ECX alive across leaf calls | k00 C18 (`dl`), k0d (`cl` = action across `429260`) | re-read the source field, verified equivalent while nothing writes it |
| Uninitialised stack local read | k0d `[esp+0x1C]` (B11, B14 and the 0x78 case) | shadow-test to learn what the stale value is in practice; cannot be reproduced exactly |
| x87 stack residue / balanced pops | `nav_path_search` (entry `fldz`), k00 C18 (0x449929), `nav_cells_heuristic` clean-up pops | a Rust port has no x87 state to leave; only the results matter |
| Mixed SSE (`movss xmm0` after helpers) and x87 | k00 K9 (`0xBDED80`), k0d AIM (`0xBDE060`, `0xBDE410`), `nav_poly_link_match` (`0xBDE060`) | call the helper through the shim and read XMM0 |
| Virtual call | k0d `call edx` at 0x42DD16 | call through the object's vtable slot 0 with ECX=this and args (&out, -1); callee pops 8 |
| `fldcw` rounding mode change | k0d E-block (`fistp qword`) | restore exactly; truncation of `1000/60` = 16 |
| `fcomp` flag polarity (`jp`/`jnp` with masks 5 / 0x41 / 0x44) | every FP branch | section 0 cheat sheet; use `!(a < b)` forms for the `jp` ones (NaN-correct) |
| Byte locals read as dwords | `nav_path_search` (`[0x18]`, `[0x20]`), k0d 0x78 case (`edi`) | pass the zero-extended byte; callees use the low byte |
| u16 args pushed as dwords and re-read as dwords | `vehicle_ai_pathfind_to/_nodes` | forward `a as u32` (callers zero-extend) |
| Unaligned / overlapping dword reads of a packed struct | `_to` `[esp+0x45]`/`[esp+0x48]`, `_nodes` `[esp+0x5D]`/`[esp+0x60]` | read the bytes exactly as the original does (`ptr::read_unaligned`) |
| Unrolled loops (x4 + tail) | `nav_poly_link_match`, packet node copy, polygon sums | a plain loop is equivalent for max/min and copies; **float sums must keep the same accumulation order** (sequential into one accumulator) |
| `idiv` by a runtime count | `nav_poly_link_match` (`% n`), `nav_tri_edge_midpoint` (`% 3`) | keep the modulo (never mask) and let `n == 0` behave as the original (fault); do not "fix" |
| Havok / physics side effects | `physics_raycast` in `nav_path_search`, `nav_closest_node_find` | stay original behind the shim; fake in the harness |
| Global shared scratch (not re-entrant) | `0x2E160B8..`, `0x2FA8CC0..` | all of these functions assume single-threaded access from the game thread; the shadow runner must not run on another thread |
| `/GS` cookie | the three vehicle functions | none in Rust; keep when relocating a copy for the shadow (the relocated copy keeps its own check) |

# 5. Which function to port next (judgement)

1. `road_query_random_lane_setup` 0xA8A4F0 (48 insns; global stores + one call).
2. `nav_cells_heuristic` 0xA11750 + `nav_tri_edge_midpoint` 0xA103B0 (pure FP, one 6-instruction idiv helper; exhaustive native fuzzing possible with no world at all).
3. `nav_poly_link_match` 0xA16650 (pure, regular, 1,176 insns but five simple pieces; needs only slices of floats).
4. `vehicle_ai_pathfind_navmesh` 0xB28700, then `_nodes`, then `_to`: the packet builder is mechanical and byte-comparable; the local branches are call sequences.
5. `nav_cells_reachable` 0xA14230 after the open-list helpers.
6. `ai_order_k00_update` (piecewise: single-call cases first), then `nav_path_search`, `nav_path_prepend_start`, `nav_human_path_resume`.
7. `ai_order_k0d_update` last (uninitialised local, virtual call, SSE helpers, `fldcw`).
