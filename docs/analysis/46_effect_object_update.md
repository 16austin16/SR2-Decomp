# 46: effect_object_update (0x505480) at instruction level

Replaces the coarse map in doc 33 ("effect_object_update (0x505480) porting map"). Doc 33 has several polarity errors, listed at the end. Function range 0x505480..0x50871F (13 088 bytes); jump table at 0x508720 (5 dwords). Everything below was read from the disassembly; the callee register contracts were checked with the recursive register-read checker and by reading the callee bodies.

Notation. `E` = the effect object (ECX on entry), `I = E[0xD0]` (effect info A), `J = E[0xD4]` (effect info B), `P` = a part node, `NOW` = `[0xE843AC]`. All offsets are hex. The CRT leaves the x87 at 53-bit precision, so every intermediate that is not stored is an f64: compute in f64 and round to f32 only at the `fstp dword` points named below. Compare polarity is stated in words for every branch.

## 0. Frame, calling convention, shared locals

- Entry: `ECX = E`, no stack arguments, plain `ret`, no result. EBX, EBP, ESI, EDI are restored.
- Prologue (0x505480): `push ebx; mov ebx,esp; sub esp,8; and esp,-16; add esp,4; push ebp; mov ebp,[ebx+4]; mov [esp+4],ebp; mov ebp,esp; sub esp,0x6A8`; cookie `[0xE80B50] ^ ebp` at `[ebp-4]`; then `push esi; push edi`. Epilogue (0x508707): `lea esp,[ebp-0x6B0]; pop edi; pop esi; cookie check (0xC9604F); mov esp,ebp; pop ebp; mov esp,ebx; pop ebx; ret`. An `__alloca_probe_16` buffer sits below the fixed frame (phase 8).
- Every "return" below is a jump to 0x508707. Nothing done earlier is rolled back.
- ESI = E nearly everywhere (reloaded from `[ebp-0x64]`), EDI = P in phase 8.

Frame slots that live across phases (`[ebp+x]`, x negative):

| slot | holds |
|---|---|
| -0x04 | security cookie |
| -0x64 | E |
| -0x68 | phase 2: `&P+0x38`; phase 6: current part; phase 8: scratch (body-loop index, `&P+0x38`) |
| -0x69 | byte: `cutscene_blocking()` result (phase 6, then phase 8 reuses it as the "ending" flag `(P.204>>3)&1`) |
| -0x6A | byte: vehicle-hit flag (phase 4/5); phase 6: saved P.205 |
| -0x70 | phase 2: `&P+0x2C`; phase 8: **T** (f32 evaluation time, see 8c) |
| -0x74 | phase 8: count of live force-field bodies (dword) |
| -0x78 | current part P (phases 2 and 8) |
| -0x7C | f32 temporary; phase 8: pointer to the zeroed alloca array **BUF** (n f32) |
| -0x80 | phase 2: number of synced parts; phase 8: loop index |
| -0xA0 | **next part** (phases 2, 8): `(P.next != E.head) ? P.next : 0`, computed from the head at loop top, before the body |
| -0x108 | **live total** (dword): sum of `effect_list_expire_timed` results, then `+= effect_instance_update` results |
| -0x144 | byte **looped** flag of the current part |
| -0x150 | dword: parts kept this frame |
| -0x11C | T_prev (f32) |
| -0x9C | frac = P.age / J[0x10] (f32) (earlier in phase 8 the same slot holds the dt and the cue id) |
| -0x94 | prev_frac = old_age / J[0x10] (f32) |
| -0xBC | old age (f32), later the current list pointer L |
| -0xD4..-0xCC | wobble vec3 |

Constants (exact bits), f32 unless "f64":

| addr | value | bits |
|---|---|---|
| 0xE5C7D8 | 100000.0 | 0x47C35000 |
| 0xE5C190 | 100.0 f64 | 0x4059000000000000 |
| 0xE5C2E0 | 0.8f as f64 | 0x3FE99999A0000000 |
| 0xE5BF40 | 1.0 f64 | 0x3FF0000000000000 |
| 0xE5BF70 | 0.5 f64 | 0x3FE0000000000000 |
| 0xE5C478 | -0.5 f64 | 0xBFE0000000000000 |
| 0xE5C408 | 1.5 f64 | 0x3FF8000000000000 |
| 0xE5C180 | -1.0 f64 | 0xBFF0000000000000 |
| 0xE5CB58 | 0.01 f64 | 0x3F847AE147AE147B |
| 0xE5C400 | 1.05f as f64 | 0x3FF0CCCCC0000000 |
| 0xE5BFC8 | 0.25 f64 | 0x3FD0000000000000 |
| 0xE5C0D8 | 1000.0 f64 | 0x408F400000000000 |
| 0xE5BF98 | 10000.0 f64 | 0x40C3880000000000 |
| 0xE5C550 | 0.6f as f64 | 0x3FE3333340000000 |
| 0xDD358C | 40.0 | 0x42200000 |
| 0xDD5984 | 0.001 | 0x3A83126F |
| 0xE98FAC | 0.5 | 0x3F000000 |
| 0xE87998 | -1.0 | 0xBF800000 |
| 0xE8437C, 0xE84380 | frame dt; file value 1/60 = 0x3C888889 (runtime variables) | |
| ints | 900 000 000 = 0x35A4E900; 1 800 000 000 = 0x6B49D200; -1 800 000 000 = 0x94B62E00; 1500 = 0x5DC | |

Globals: `[0x2527280]` byte (global stop); `[0x252A6D0..D8]` the default point (3 f32, a "none" vector); `[0xE843AC]` the game clock in ms with period 1 800 000 000; `[0xE8437C]`, `[0xE84380]`, `[0x27E671C]` frame dts; `[0x21703D4]` local player object; `[0x2528B48]`, `[0x2528B49]` bytes (net flags); `[0x2659DD0]` explosion kind id; `[0x27974D4]` blast radius; `[0x279748C]`, `[0x279749C]` knockdown forces; `[0x2526BD2]`, `[0x2526BDF]` bytes, `[0x2526CB8]` f32 (view range clamp); `[0x2526E2B]` byte, `[0x2526E4C]` f32 (wobble); `[0x25F5B14..1C]` camera position; `[0xEEB254..5C]` listener position; `[0xEE5558 + cue*4]` cue table; `[0xEE9178 + x*4]` sound table; `[0xEEB8F8 + slot*0x19C]` voice slots (generation `+0xB4`, flags words `+0x194`, `+0x196`, `+0x198`); `[0xF5389C]` sound-key sets; entity table `[0x2149C64 + (h & 0xFFF)*16]` -> object (object `+0x44` = handle, `+0x4C` = type, 1 human, 5 vehicle); body objects `[0x28F25C0 + body*24]`; light objects `0x2AE50E0 + (h & 0xFFFF)*0x134` (handle at `+8`); free part list `[0xF9F67C]` and used count `[0xF9F674]`; vehicle lists `[0x22A84E0]`, `[0x22A84D4]` (circular, next link `+0x84DC`).

### Object layouts (only what this function touches)

- **E**: +00 vtbl; +14/18/1C centre; +20..+40 own 3x3 matrix (9 f32); +7C physics handle (-1 none); +80 flags (bit1 = destroy requested); +84 flags (bit0 tested); +C4 part list head (circular, next +0, prev +4); +C8 flags (0x80 tested); +CC kind 0..4 (1 ended, 2 never self-destroys, 4 static/always visible); +D0 I; +D4 J; +DC list count n; +E0 list pointers[n]; +33C radius f32; +340 live count (int); +348/34C/350 local offset vec3; +354 lifetime timer (NOW-clock value, -1 none); +358 dword; +35C byte.
- **I**: +08 kind id; +10 / +14 / +18 f32 (far range^2, cutscene far range^2, near range^2); +1C radius f32; +20 wobble scale f32; +24 word sound-key set; +26 word cue id (0xFFFF none); +28 dword; +2E word flags (0x04 vehicle test, 0x08 player test, 0x20 stop-rule, 0x40 second sound, 0x80, 0x100, 0x200, 0x400 sound flags).
- **J**: +10 duration f32; +20 body-slot count; +24 curve table (48-byte records); +38 force-item count; +3C force items (0x2C bytes each); +40 light count; +44 light items (0x2C bytes each).
- **L** (a list, E[0xE0+4i]): +00 group G (G+0x24 curve table of 0x30 records, G+0x2C defs of 0xC0 bytes); +04 def index; +0x9C word (bit0 active); +0xA4 f32 output.
- **P**: +00 next, +04 prev, +0C part id, +14 stamp (ms clock), +18 stamp (ms clock), +1C age f32, +20 owner handle, +24 bone (-1 none), +28 loop counter (int), +2C pos, +38 mat3, +5C prev pos, +68 prev mat3, +90 fade f32, +98.. body handles (32), +118.. per-slot pointers (voice handle at ptr+0x30), +19C light handles, +1DC locked fraction f32 (<0 none), +1E0 ending timer f32, +1E8 voice handle A (3 dwords), +1F4 voice handle B (3 dwords), +200 sound-key time f32, +204 flags byte (0x02 force-field phase, 0x04 physics-body part, 0x08 ending, 0x10 human hand-0 attach, 0x20 human hand-1 attach, 0x40 shown, 0x80 culled-far), +205 flags byte (0x01 fade-out, 0x02 silent).
- **Voice handle** (12 bytes): byte0 slot (0xFF none); dword0 >> 8 generation. **ALIVE(h)** = slot != 0xFF && gen != 0 && gen == `[0xEEB9AC + slot*0x19C]` && `[0xEEB8F8 + slot*0x19C] != -1`.

Shared snippets:

- **LIST_UNLINK(E,P)** (inline): if `E.head == P` { if `P.next == P` { `P.next = P.prev = 0; E.head = 0` and done } else `E.head = P.next` }; then if `P.prev != 0` `P.prev.next = P.next`; if `P.next != 0` `P.next.prev = P.prev`; `P.next = P.prev = 0`.
- **DESTROY_PART(E,P)** = LIST_UNLINK, then `effect_object_part_destroy(E, P)` (cdecl, push P, push E, `add esp,8`), then free-list insert: `[0xF9F674] -= 1`; if `[0xF9F67C] == 0` { `P.next = P.prev = P; [0xF9F67C] = P` } else { `h = [0xF9F67C]; P.next = h; P.prev = h.prev; h.prev.next = P; h.prev = P` }. The caller then continues with the saved next (`[ebp-0xA0]`). Copies at 0x505627, 0x5057CA, 0x5058C6, 0x505949, 0x5085E3.
- **FADE_CLAMP(v)** used by phase 6: `NaN or v <= 0 -> 0.0; v >= 1.0 -> 1.0; else v`.

## 1. Callee register contracts

Named callees (the existing CSV rows were checked; corrections in the last section):

| callee | registers in | stack (cdecl unless noted) | out |
|---|---|---|---|
| 0x50AB60 (`effect_object_stop`, new) | EDI = E | (id_filter, keep) both 0 here; `add esp,8` | none |
| 0x50F780 effect_list_expire_timed | ESI = list L | none | EAX = live count |
| 0x5047D0 effect_object_part_destroy | none | (E, P) pushed P first then E; `add esp,8` | none |
| 0x6FDAC0 physics_body_lookup | EDI = &pos_out | (handle, &mat_out); `add esp,8` | AL = found; writes pos (3 f32) to [EDI], mat (9 f32) to mat_out |
| 0x6FC760 physics_body_get_transform | ESI = &pos_out, **EDI = &mat_out** | (body); `add esp,4` | writes both |
| 0x40B6A0 (`vec3_ne_exact`, new) | ECX = &a, EDX = &b | none | AL = 1 if any component differs (fucompp, `test ah,0x44`), 0 if all three are exactly equal |
| 0xBDE570 vec3_mul_mat33_in_place | ECX = &v, EAX = &m | none | v = x*row0 + y*row1 + z*row2 (m rows at +0/+C/+18). EDX is preserved |
| 0x960D60 (vehicle_seat_occupant) | EAX = handle | none | EAX = live type-1 object (human) or 0 (see doc 41) |
| 0x9C4430 / 0x9C44C0 (new) | EAX = human | (&pos_out, &mat_out); `add esp,8` | AL = ok. They build the hand-0 / hand-1 attach transform (pass 0 / 1 to 0x9C31B0, then 0x6E5DB0) |
| 0x6E5DB0 (new) | EAX = &pos_out, EDX = transform descriptor (0 = none) | 4 stack dwords left by the caller (a 4th one = &mat_out at [esp+0x10]) | AL; EDX == 0 writes the default point and the identity (0xEC5AE4) |
| 0xAAC5C0 (new) | EAX = bone slot, ECX = &mat_out | (&pos_out, vehicle); `add esp,8` | pos, mat |
| 0xCEDF10 rig_bone_transform | ECX = [[human+0x5E0]] | (bone, &ent.pos, &ent.mat, &pos_out, &mat_out); `add esp,0x14` | writes pos_out, mat_out |
| 0x465B90 audio_play | EAX = &params (0x5C bytes), ECX = &out (12 bytes) | none | EAX = &out; handle copied from it |
| 0x47FFC0 audio_voice_position_check | ECX = &pos, EDX = &handle | none | AL = voice still valid |
| 0x47C740 audio_voice_slot_fade_out | EAX = slot idx, EDX = fade ms (0xFA) | none | none |
| 0x6C6D20 cutscene_blocking / 0x6C6D90 cutscene_in_progress | none | none | AL |
| 0x40EA20 sqrtf_wrapper | none | (f32); caller `push ecx; fstp [esp]`, `add esp,4` | ST0 |
| 0x428CD0 / 0x40B820 sinf / cosf wrappers | none | (f32) | ST0; the caller keeps the slot and does `add esp,4` |
| 0x6FE460 physics_query_aabb | ECX = &min | (&max, &out_handles, 10, 0x8005E, 0, 0); `add esp,0x18` | EAX = count (unsigned) |
| 0x98D3C0 human_knockdown | ECX = 0 | (human, f32 force, 0, 0, 0); `add esp,0x14` | none |
| 0x50F880 effect_instance_update | EAX = &cur_pos, EDX = &prev_mat | 11 args, see 8e; `add esp,0x2C` | EAX = live count to add to the live total |
| 0xA020C0 shape_bounds_overlap_test | ECX = &boxA, EDX = &boxB (EDX is still intact afterwards: the caller reuses it) | (shape ptr 0x277CBF0); `add esp,4` | EAX |
| 0x524550 render_views_test_bounds | none | (arg1, arg2); `add esp,8`; the caller passes arg1 = EDX value (&[ebp-0x2FC]), arg2 = &[ebp-0x308] | AL = 1 if any of the views accepts the box (calls 0xA0 23F0, which returns "no plane rejects") |
| 0xBE4170 matrix_orthonormalize | none | stdcall (float m[9]), `ret 4` | none |
| 0xDA0140 _ftol2_sse | ST0 | none | EAX = (int) ST0 (truncation) |
| 0xCA0BC0 memset, 0xCB35C0 __alloca_probe_16 (EAX = bytes) | | | |

New (unnamed, see candidates_effect.csv):

| callee | registers in | stack | out / role |
|---|---|---|---|
| 0x503FE0 `effect_object_part_pick_cue` | EAX = I | cdecl (P, looped); `add esp,8` | AX = sound cue id (0xFFFF none). If looped it sets P[0x200] = -1.0 (0xBF800000). Looks up the I[0x24] set at `[0xF5389C]` (12-byte records: +4 key array of 8-byte {f32 time, u16 cue}, +8 count) for the last key whose time passed P[0x200]; keeps P[0x200] = that time |
| 0x514F80 `curve_eval_scalar` | ESI = key count | cdecl (f32 t, keys); `add esp,8` | ST0; count 1 returns key[1] |
| 0x5151E0 `curve_eval_vec3` | EDX = &out (3 f32) | cdecl (f32 t, count, keys); `add esp,0xC` | EAX = &out |
| 0x515340 `curve_eval_quat` | EAX = key count | cdecl (&out(4 f32), f32 t, keys); `add esp,0xC` | EAX = &out (4 f32 quaternion x y z w) |
| 0xC057D0 `quat_to_mat33` | EAX = &quat, ECX = &mat_out | none | writes 9 f32 |
| 0xBE2F50 `mat33_mul` | EAX = &out, ECX = &A | stdcall (&B), `ret 4` | EAX = &out; out = A x B in the row convention of 0xBDE570 |
| 0xBE2E10 `mat33_from_axis_angle` | EDI = &unit axis, ESI = &out mat | stdcall (f32 angle), `ret 4` | writes 9 f32 |
| 0xBE4B20 `mat33_from_forward` | EAX = &dir, ECX = &mat_out | none | basis with the dir as row 2 (+0x18..0x20); handles near-vertical dirs |
| 0xBDE150 `vec3_normalize_safe` | ESI = &v | none | unit v; zero or non-finite length gives (1,0,0) |
| 0xBDE340 `vec3_normalize_safe_len` | ESI = &v | none | like 0xBDE150, XMM0 = the length (read by the caller as a scalar) |
| 0xBDE750 `vec3_random_unit` | ESI = &out | none | random unit vector (rng 0xEC30F8, rng_float01) |
| 0xBDE7C0 `vec3_random_cone` | ESI = &out | stdcall (f32 a, f32 b), `ret 8` | random unit vector with z uniform in [a, b]-ish (cone around +Z) |
| 0xBDDE20 `vec3_mul_mat33` | EAX = &out, ECX = &m | stdcall (&v), `ret 4` | out = v x m (not in place) |
| 0xBF58D0 `obb_to_aabb` | ESI = &extA, EDX = &extB, EDI = &mat (pos at EDI-0xC) | cdecl (&outA, &outB); `add esp,8` | outA = centre + T[0..2], outB = centre + T[3..5] |
| 0x6FC990 / 0x6FCBD0 `physics_body_get_linear_velocity` / `_angular_velocity` | ECX = body | cdecl (&out vec3); `add esp,4` | reads body+0x1A0 / +0x1B0 |
| 0x6FCEC0 `physics_body_apply_linear_impulse` | ECX = body | cdecl (&vec3); `add esp,4` | wakes the body (0xD58FF0), calls vtbl slot 0x12 of the object at body+0xD0 with (x y z 0) |
| 0xD58FF0 `physics_body_wake` | ESI = body object | none | clears +0xDA/+0xDC and calls 0xD86FF0 when the body is asleep |
| 0x70E280 `mat44_from_mat33_pos` | ECX = &mat33, EDX = &out (64 bytes) | cdecl (&pos); `ret` plain | rows 0..2 = rotation, row 3 = (pos, 0) |
| 0xD55980 `shape_set_world_matrix` | ESI = shape, EAX = &4x4 | none | copies 64 bytes to shape+0xA0 and updates the shape |

## 2. Section map (each can be ported alone)

| # | range | what | notes |
|---|---|---|---|
| S0 | 0x505480..0x505503 | prologue, early outs, stop rule | |
| S1 | 0x505504..0x505563 | reset centre, expire the timed lists | |
| S2 | 0x505564..0x505B63 | part loop: sync transforms, destroy dead parts | 8 callee kinds |
| S3 | 0x505B64..0x505CC1 | centre, radius | pure maths |
| S4 | 0x505CC2..0x50639B | "centre inside a vehicle box" early return (two lists) | virtual calls |
| S5 | 0x50639C..0x506447 | "centre inside the local player box" early return | virtual call |
| S6 | 0x506448..0x5067D6 | visibility / fade per part | |
| S7 | 0x5067D7..0x5069D7 | lifetime timer, stop, network range, explosion knockdown | |
| S8 | 0x5069D8..0x5086A3 | main pass per part: 8a age and loops, 8b voices, 8c T and wobble, 8d force field, 8e emitter lists, 8f lights, 8g end of part | the big one |
| S9 | 0x5086A4..0x508706 | tail: live count, list outputs, self destroy | |

Hand-over between sections: S1 writes the live total (-0x108) and zeroes -0x150; S2 produces E.centre (sum), E[0x14..0x1C], the count -0x80 and the part transforms; S3 reads them; S6 writes P.204/P.205/P.90 and (via -0x6A, -0x69) nothing that later phases read except that S8 recomputes -0x69; S7 may return; S8 reads -0x108 and writes it; S9 reads -0x108 and -0x150.

## S0. Entry (0x505480..0x505503)

```
if byte[0x2527280] != 0 return                        ; jne
I = E[0xD0]
if I[0x28] == 0 and E[0xD4] == 0 return               ; both compares are against zero
if E[0x84] & 1 {
    if E[0x358] == 0 { stop = !(I[0x2E] & 0x20) }     ; test byte [I+0x2E],0x20 ; jne -> skip
    else             { stop = (E[0xCC] != 1) }        ; cmp [E+0xCC],1 ; je -> skip
    if stop { effect_object_stop(EDI=E, 0, 0) }       ; push 0 ; push 0 ; add esp,8
}
```
Polarity: the stop call is made when the bit-0x20 flag is CLEAR (E[0x358] == 0) or when the kind is NOT 1 (E[0x358] != 0). Doc 33 has this inverted for the first arm.

## S1. Centre reset and timed lists (0x505504..0x505563)

```
E[0x14] = [0x252A6D0]; E[0x18] = [0x252A6D4]; E[0x1C] = [0x252A6D8]    ; dword copies
live_total (-0x108) = 0; count (-0x80) = 0; kept (-0x150) = 0
n = E[0xDC]
if n > 0 (signed, jle skips) {
    for i in 0..n { ESI = E[0xE0 + 4*i] ; live_total += call 0x50F780 }   ; no null check; ECX is not an input
}
```
`0x50F780` returns the live count of that list. Doc 33 calls the sum "expired".

## S2. Part loop (0x505564..0x505B63)

Loop head 0x505564: `P = E[0xC4]`; `if P == 0 goto S3`. Body (P = EDX at 0x505570):

```
next = (P.next != E.head) ? P.next : 0                  ; sub / neg / sbb / and trick, uses the head as it is NOW
pos = &P[0x2C]; mat = &P[0x38]
P[0x5C..0x64] = P[0x2C..0x34] (3 dwords); P[0x68..0x8B] = P[0x38..0x5B] (9 dwords, rep movsd)
h = P[0x20]
if h != 0 {
    ent = ENT[(h & 0xFFF) << 4]
    if ent != 0 and ent[0x44] == h and ent[0x4C] == 1 and (ent[0xC8] & 0x400000) == 0 {   ; human WITHOUT the 0x400000 bit
        P[0x204] &= 0xCF; P[0x24] = -1
    }
}
flags = P[0x204]  (re-read after the clear)
```

**Branch A: flags & 4 (physics-body part)** (0x505609):
```
ok = physics_body_lookup(h, &matTmp[ebp-0x584])  with EDI = &posTmp[ebp-0x1B0]
if ok == 0 { DESTROY_PART(E,P); P = next; continue }
*pos = posTmp (3 dwords); *mat = matTmp (9 dwords)
off = (E[0x348], E[0x34C], E[0x350]) -> [ebp-0xE0..-0xD8] (f32 copies)
if off.x != D.x or off.y != D.y or off.z != D.z  (D = default point; fucompp, test ah,0x44, JP = not equal or NaN) {
    vec3_mul_mat33_in_place(ECX=&off, EAX=mat)         ; EDX = pos stays intact
    pos.x = f32(off.x + pos.x); pos.y = f32(pos.y + off.y); pos.z = f32(pos.z + off.z)
}
goto ACCUM
```
**Branch B: flags & 4 == 0** (0x5057A2):
```
if h == 0 {                                              ; 0x505B3E
    if E[0x7C] != -1 { physics_body_get_transform(ESI=pos, EDI=mat, E[0x7C]) }
    goto ACCUM                                           ; (ESI = E)
}
ent = ENT[(h & 0xFFF) << 4]
if ent == 0 or ent[0x44] != h { DESTROY_PART(E,P); continue }
bone = P[0x24]
if bone != -1 {                                          ; 0x505A61
    switch ent[0x4C] {
      case 1 (human):                                    ; 0x505AA7
         local[9 f32 at ebp-0x2C0..] = E[0x20..0x40]     ; E's own matrix
         rig_bone_transform(ECX=[[ent+0x5E0]], bone, &ent.pos(+0x14), &ent.mat(+0x20), pos, mat)
         if byte E[0x35C] == 1 { *mat = local }          ; cmp byte,1 ; jne -> skip
      case 5 (vehicle):                                  ; 0x505A93
         aac5c0(EAX=bone, ECX=mat, pos, ent)
      default:                                           ; 0x505A6E
         d = ent.vtbl[1](ECX=ent; pushes bone, &ent.pos, &ent.mat, mat)   ; args stay on the stack
         0x6E5DB0(EAX=pos, EDX=d, [esp+0x10] = mat) ; add esp,0x10
    }
    goto ACCUM                                           ; (no offset applied)
}
if flags & 0x10 {                                        ; 0x50588C: hand-0 attach
    ent must be live type 1 again (re-read) : else DESTROY_PART
    if 0x9C4430(EAX=ent, pos, mat) == 0 { DESTROY_PART }
    goto ACCUM
}
if flags & 0x20 {                                        ; 0x50591C: hand-1 attach
    hum = 0x960D60(EAX=h) ; if hum == 0 { DESTROY_PART }
    if 0x9C44C0(EAX=hum, pos, mat) == 0 { DESTROY_PART }
    goto ACCUM
}
; plain owner follow (0x505995)
*pos = ent[0x14..0x1C]                                   ; 3 dwords
if byte E[0x35C] == 0 { *mat = ent[0x20..0x40] (9 dwords) }
off = E[0x348..0x350] -> [ebp-0xC8..]
if 0x40B6A0(ECX=0x252A6D0, EDX=&off) != 0 { vec3_mul_mat33_in_place(ECX=&off, EAX=mat); pos += off (same fadd order as branch A: off + pos) }
```
ACCUM (0x505A2D): `E[0x14] = f32(E[0x14] + pos.x)` (order `fld E.x; fadd pos.x`), `E[0x18] = f32(pos.y + E[0x18])`, `E[0x1C] = f32(pos.z + E[0x1C])`; `count += 1`; `P = next`.

Notes: in branch B the ESI value on the h == 0 path is E; the entity check for flag 0x10 re-reads the table (same result). After a DESTROY_PART the loop continues with `next` and does NOT count the part. Dead parts never reach ACCUM. Doc 33 says 0x9C44C0/0x9C4430 are vehicle seat calls: they are human attach-transform calls; 0x960D60 is handle -> human (doc 41).

Portable now: yes, once the callee table above is wrapped. Only the `vtbl[1]` default case needs the live object. The rest is straight code.

## S3. Centre and radius (0x505B64..0x505CC1)

```
if count <= 0 (signed) goto S4                          ; centre stays the default point, E[0x33C] is NOT written
inv = f32(1.0 / (double)count)                          ; fild, fld1, fdivrp, fstp dword [ebp-0x7C]
E[0x14] = f32(inv * E[0x14]); E[0x18] = f32(inv*E[0x18]); E[0x1C] = f32(inv*E[0x1C])
if E[0xCC] == 4 { r = 100000.0f }                       ; fld dword [0xE5C7D8]
else {
    best = default point (3 f32 copies), best_d2 = 0.0f
    for each part P (circular from E.head, 'ecx = P.next; if ecx == head or ecx == 0 break'):
        d = (P.pos.x - E.x, P.pos.y - E.y, P.pos.z - E.z), each f32 stored
        d2 = f32( (dx*dx + dy*dy) + dz*dz )             ; x87 order: dx*dx, dy*dy, add, dz*dz, add; one fstp dword
        if best_d2 < d2 { best_d2 = d2; best = d }      ; fcomp / test ah,5 / jp (best >= d2 or NaN keeps)
    s = f32( (best.x*best.x + best.y*best.y) + best.z*best.z )   ; same order, from the best vector
    r = f32( sqrtf(s) + I[0x1C] )                       ; fadd dword [I+0x1C] then fstp
}
E[0x33C] = r
```
Quirks: the best vector starts at the default point, not zero, so with all parts on the centre the radius is |default|+I[0x1C]. The sqrt arg uses `push ecx / fstp [esp]`, then `add esp,4`.

Portable now: yes (sqrtf_wrapper callable).

## S4. Vehicle boxes (0x505CC2..0x50639B)

Only if `I[0x2E] & 4`. Two identical loops: list A head `[0x22A84E0]` (code 0x505CD2..0x50602E), list B head `[0x22A84D4]` (0x506038..0x506392). The stack offsets differ but the maths is the same. `hit` = byte [ebp-0x6A], cleared before list A only.

```
v = head ; if v == 0 skip the list
do {
    d = (v[0x14]-E[0x14], v[0x18]-E[0x18], v[0x1C]-E[0x1C])  ; each f32 stored
    d2 = f32(dx*dx) ; d2 = f32(d2 + dy*dy) ; d2 = f32(d2 + dz*dz)       ; sequential f32 stores in the code
    if d2 < 100.0 (fcomp qword 0xE5C190, test ah,5, JP skips when d2 >= 100 or NaN) {
        v.vtbl[3](ECX=v, arg1=&A, arg2=&B)     ; local bbox : A = min ([ebp-0x168]), B = max ([ebp-0x174]) (list A)
        v.vtbl[2](ECX=v, arg1=&P0, arg2=&Q0)   ; world bbox: P0 = min ([ebp-0x198]), Q0 = max ([ebp-0x1A4])
        ext  = B - A                           ; f32 each
        a = f32(0.8 * ext.x) ; b = f32(ext.y + 1.0) ; c = f32(0.8 * ext.z)
        C = (P0 + (Q0-P0)*0.5)                 ; each: D=f32(Q0-P0); h=f32(D*0.5); C=f32(P0+h)
        lo = (f32(-0.5*a), f32(b*-0.5), f32(c*-0.5)) ; hi = (f32(a*0.5), f32(b*0.5), f32(c*0.5))
        rel = E.centre - C                     ; f32 each
        loc.x = rel.x*v[0x20] + rel.y*v[0x24] + rel.z*v[0x28]  (one fstp, x87 order: rel.y*m1, rel.x*m0, add, rel.z*m2, add)
        loc.y = rel.x*v[0x2C] + rel.y*v[0x30] + rel.z*v[0x34]
        loc.z = rel.x*v[0x38] + rel.y*v[0x3C] + rel.z*v[0x40]
        if for all axes: !(loc < lo) && !(loc > hi)      ; NaN counts as inside
            hit = 1
    }
    v = v[0x84DC]
} while v != head and v != 0
if hit return          ; after list A, and again after list B
```
The two loops are separate compiler copies (list A: code 0x505CE7..0x506018, list B: 0x506049..0x50637C). The local-space dot products have the same weights but a slightly different x87 operation order (list A rows: x = 0x20/0x24/0x28, y = 0x2C/0x30/0x34, z = 0x38/0x3C/0x40; list B accumulates in a different order). Port each copy from its own listing; the sums are f64 until the one `fstp dword`.

Per-axis polarity: x and y: skip the vehicle when `lo > loc` (fcomp lo vs loc, `test ah,0x41; je`) or when `hi < loc` (`fcompp`, `test ah,5; jnp`). z: `hi < loc` is tested with `test ah,1; jne` and the hit flag is set otherwise. Unordered values fall through as inside.

Effect: the effect is skipped entirely this frame when its centre lies inside the (shrunk x/z, grown y) local box of a vehicle within 10 m. Doc 33 says only "a vehicle that hits the test ends the frame".

## S5. Local player box (0x50639C..0x506447)

```
if (I[0x2E] & 8) and (p = [0x21703D4]) != 0 {
    p.vtbl[2](ECX=p, arg1=&mn [ebp-0x218], arg2=&mx [ebp-0x248])
    inside = mn.x < E.x  &&  mx.x > E.x
          && (mn.y - 1.0) < E.y  &&  mx.y > E.y
          && mn.z < E.z  &&  mx.z > E.z
    if inside return
}
```
Polarity (fcompp / test ah,5 or 0x41): every "less" test is strict. `mn.y - 1.0` is computed in x87 (fsub qword 0xE5BF40) and compared unrounded. For unordered values the first arm of each pair bails out of the test (no return); the z-high test is `je 0x508707` (return when `mx.z > E.z`). Doc 33 says "out of range -> return": the real rule is "centre INSIDE the player's box -> return".

## S6. Visibility and fade (0x506448..0x5067D6)

```
x87 stack: fld1 (kept for the clamp; one pop at the end of the section)
if E[0xCC] == 4 {
    for each part P (circular from E.head): P[0x204] |= 0x40; P[0x90] = 1.0f; P[0x205] &= 0xFE
}
else if E.head != 0 {
    step = f32( (double)[0xE8437C] / 1.5 )                   ; fld dword, fdiv qword 0xE5C408, fstp dword [ebp-0xA0] (not cutscene-adjusted)
    for each part P (circular from the head, ends when next == head or 0) {
        P[0x205] &= 0xFE ; P[0x204] |= 0x40
        s205 = P[0x205] ; s204 = P[0x204]                   ; saved (bytes at -0x6A, -0x89)
        dx = f32(P.x - [0x25F5B14]) ; sq = f32(dx*dx) ; dy = f32(P.y - [0x25F5B18]) ; sq = f32(sq + dy*dy)
        dz = f32(P.z - [0x25F5B1C]) ; camd2 = f32(sq + dz*dz)         ; sequential f32 stores
        lim = I[0x10]
        cb = cutscene_blocking() ; [ebp-0x69] = cb ; if cb == 1 { lim = I[0x14] }       ; cmp al,1 : only the value 1
        if byte[0x2526BD2] != 0 and byte[0x2526BDF] != 0 {
            s = [0x2526CB8]^2 (x87, not rounded) ; if !(lim < s) { lim = f32(s) }          ; fcom: lim < s keeps lim ; lim >= s or NaN takes s
        }
        if lim < camd2 {                                         ; too far (JP not taken)
            if (E[0xC8] & 0x80) == 0 or E[0x340] == 0 { P[0x205] = s205 | 1 }
            if  E[0xC8] & 0x80 and (s204 & 0x80) == 0 { P[0x204] = s204 | 0x80 }
        }
        else if I[0x18] > camd2 {                                ; near: fcompp, test ah,0x41, jne not taken
            P[0x205] = s205 | 1                                  ; too near: fade out
        }
        else {
            r = I[0x1C] ; ext_min = (-r,-r,-r) (f32 by x87 mul with -1.0) ; ext_max = (r,r,r)
            obb_to_aabb(ESI=&ext_min, EDX=&ext_max, EDI=&P[0x38], &AABB_A [ebp-0x2FC], &AABB_B [ebp-0x308])
            visible = true
            if cb == 0 {                                         ; cmp byte [ebp-0x69],0 ; jne -> visible
                if a020c0(ECX=&AABB_B, EDX=&AABB_A, shape 0x277CBF0) == 0 { visible = false }
                else if render_views_test_bounds(arg1=&AABB_A, arg2=&AABB_B) != 0 { visible = false }  ; sete al ; test ; jne -> visible path
            }
            if !visible { P[0x205] = s205 | 1 }
            else if (E[0xC8] & 0x80) and (s204 & 0x80) { P[0x204] = s204 & 0x7F }
        }
        ; fade (x87), P[0x90] is f32
        if P[0x205] & 1 {                                        ; fade out
            v = f32(P[0x90] - step) ; P[0x90] = v
            if v <= 0.01 (fcomp qword 0xE5CB58 ; test ah,0x41 ; JP not taken) { P[0x204] &= 0xBF }   ; NaN skips
            P[0x90] = FADE_CLAMP(v)
        } else {                                                 ; fade in
            v = f32(P[0x90] + step) ; P[0x90] = v ; P[0x90] = FADE_CLAMP(v)
        }
    }
}
x87: one pop (fstp st0) at 0x5067D9
```
FADE_CLAMP detail: `v <= 0 or NaN -> 0.0; v >= 1.0 -> 1.0 (also v == 1.0); else v`. The clamp results are stored through `[ebp-0x74]` as f32, so they are exact.

Open question: the visible/hidden meaning of the two box tests is the opposite of what the callee names suggest (hidden when `render_views_test_bounds` returns 1). The code is decoded as written; do not "fix" it. The `-1.0` that the loop pushes (`fld qword 0xE5C180`) is the multiplier that builds `ext_min`.

Portable now: yes, with the three callees `obb_to_aabb`, `a020c0`, `render_views_test_bounds` called through their addresses. The x87 stack juggling around `fld1` is a compiler artifact.

## S7. Lifetime timer and explosion (0x5067D7..0x5069D7)

```
t = E[0x354]
if t < 0 (signed) goto S8                                        ; no timer
elapsed:
  if t > NOW (signed) { d = t - NOW ; elapsed = (d >= 900000000) }   ; jl (d < 9e8) skips
  else               { d = NOW - t ; elapsed = (d <  900000000) }   ; jl 0x506805
if elapsed { effect_object_stop(EDI=E,0,0); E[0x354] = -1; return }  ; 0x506805
c = byte[0x2528B48]
if c == 1 and byte[0x2528B49] == 0 { goto CHECK_STOP }          ; single-player-like case, skips the distance rule
; else (0x506841): distance to the local player (no null check on [0x21703D4])
p = [0x21703D4] ; d = p.pos - E.centre (f32 each)
dist = sqrtf( f32((dx*dx + dy*dy) + dz*dz) )                     ; one f32 store before sqrt
if dist > 40.0 (fcomp dword 0xDD358C ; test ah,0x41 ; je) -> goto 0x506805   ; stop + clear timer + return   (NaN does not stop)
CHECK_STOP (0x5068AF):
if E[0x84] & 1 -> goto 0x506805                                  ; stop + clear timer + return
if I[8] != [0x2659DD0] goto S8                                   ; not the explosion kind
; explosion knockdown
r = [0x27974D4]
min = (E.x - r, E.y - r, E.z - r) -> [ebp-0x20C]; max = (E.x + r, ...) -> [ebp-0x230]     ; f32 stores; the sums/differences use the x87 value of r
cnt = physics_query_aabb(ECX=&min, &max, &handles[ebp-0x5AC], 10, 0x8005E, 0, 0)
for i in 0..cnt (unsigned compare, cnt == 0 skips) {
    h = handles[i]; if h == 0 continue
    ent = ENT[(h & 0xFFF)<<4]; if ent == 0 or ent[0x44] != h or ent[0x4C] != 1 continue
    force = (byte[0x2528B48]==1 && byte[0x2528B49]==0) ? f32[0x279749C] : f32[0x279748C]
    human_knockdown(ECX=0, ent, force, 0, 0, 0)
}
```
Doc 33: "In a network game, return if the local player is more than 40 m away" is incomplete: the return goes through the stop path (stop(0,0) and E[0x354] = -1), and the check runs in every case except `[0x2528B48] == 1 and [0x2528B49] == 0`. The explosion branch needs `E[0x84] bit 0` clear.

Portable now: yes.

## S8. Main pass (0x5069D8..0x5086A3)

Setup (0x5069D8): `bytes = E[0xDC]*4; __alloca_probe_16(EAX=bytes); BUF = esp; memset(BUF, 0, bytes) (cdecl, add esp,0xC)`; `P = E[0xC4]`; if `P == 0` goto S9. BUF is an f32 array (one per list); its element is passed by pointer to `effect_instance_update` and copied to `L+0xA4` in S9.

Loop top (0x506A0B): `next = (P.next != E.head) ? P.next : 0` (stored at -0xA0); `old_age = P[0x1C]` (-0xBC, f32 copy); `looped = 0` (-0x144); `dt = cutscene_in_progress() ? [0x27E671C] : [0xE8437C]` (-0x9C); `ending = (P[0x204] >> 3) & 1` (-0x69, dl).

### 8a. Age and loop wrap (0x506A63..0x506B30)

```
if ending { v = f32(P[0x1E0] - dt) ; P[0x1E0] = v ; if v < 0 { P[0x1E0] = 0.0 } }     ; v == 0 and NaN keep v (jne on 0x41)
age = f32(dt + P[0x1C]) ; P[0x1C] = age                        ; fadd dword then fst
if J[0x10] < age (strict; JP skips) {                          ; expired
    kind = E[0xCC]
    if kind > 4 (unsigned) skip
    switch kind {                                              ; jump table 0x508720 = [0x506ACC, 0x506ACC, 0x506ACC, 0x506AE5, 0x506AE5]
      case 0,1,2:                                              ; 0x506ACC
         c = P[0x28]
         if c > 0 (signed) { P[0x28] = c - 1 ; WRAP }
         else { P[0x1C] = J[0x10] }                            ; clamp, no wrap, looped stays 0
      case 3,4:                                                ; 0x506AE5
         P[0x28] += 1
         if ending == 0 { WRAP }                               ; al = dl ^ 1
    }
}
WRAP: while J[0x10] <= P[0x1C] (re-read, fcompp; test ah,0x41; loop on 'no JP' = J < age or J == age) { P[0x1C] = f32(P[0x1C] - J[0x10]) }
      looped = 1     ; also set when the first compare already fails (jp to 0x506B27)
```
WRAP detail: at 0x506AF1 the loop test is `fcompp` of J[0x10] vs age with `test ah,0x41; jp exit`; JP (both flags equal) means `J > age` or NaN: exit. The loop never ends when J[0x10] <= 0 and age >= 0 (original behaviour).

### 8b. Voices (0x506B32..0x506EF8)

If `P[0x205] & 2` (silent) go to 8b-silent (0x507051): for each of `P+0x1E8` and `P+0x1F4` that is ALIVE, `audio_voice_slot_fade_out(EAX=slot, EDX=250)`; then continue at 8c (0x506EFC, ESI = E).

Otherwise:
```
dlA = ALIVE(P+0x1E8)
audible = 0
if I != 0 and (cue = I[0x26] as u16) != 0xFFFF {
    v = ([0xEEB254]-P.x, [0xEEB258]-P.y, [0xEEB25C]-P.z) (f32 each)
    R = f32( f32[ [ [0xEE9178 + (( [0xEE5558 + cue*4] -> [+0x10] ) & 0xFFF)*4 ] ] + 0x28 ] * 1.05 )      ; fmul qword 0xE5C400, fstp dword
    d2 = f32((vx*vx + vy*vy) + vz*vz) ; R2 = f32(R*R)
    audible = (R2 > d2)                                            ; fcompp, test ah,0x41, jne skips: R2 < d2, R2 == d2 and NaN give 0
}
if dlA {                                                           ; 0x506D2B
    if audible { if audio_voice_position_check(ECX=&P.pos, EDX=&P[0x1E8]) == 0 { P[0x1E8] = 0xFF (dword) } }
    else       { STOP_VOICE_A }
} else if audible {
    if ending == 0 { START_VOICE_A }                               ; cmp [ebp-0x69],dl(=0)
} else { STOP_VOICE_A }
```
STOP_VOICE_A (0x506D45): `if byte0 != 0xFF and gen != 0 and gen == tab+0xB4 and tab[0] != -1 and !(tab+0x196 & 0x80) { w = word tab+0x198 ; if ((w | (w >> 1)) & 0x800) == 0 { tab+0x198 = w | 0x400 ; word tab+0x194 &= 0xFF7F } }`.

START_VOICE_A (0x506C82): memset a 0x5C block to 0; `+0x00 (word) = I[0x26]`; `+0x0C = 1.0f`; `+0x10 = 1.0f`; `+0x1C/+0x20/+0x24 = P.pos`; `+0x30 (word) = 0xFFFF`; `+0x32 (word) = 1`; `+0x4C (dword) = P`; `+0x58 (word) |= 1`; the other fields 0. `audio_play(EAX=&block, ECX=&tmp)` then copy 3 dwords from the returned EAX into P+0x1E8.

Second voice (B at P+0x1F4), only if `I != 0 and (I[0x2E] & 0x40)` (0x506DA0):
```
cue2 = 0x503FE0(EAX=I, P, looped) as u16
if cue2 != 0xFFFF and (P[0x204] & 8) == 0 {
    if ALIVE(P+0x1F4) { audio_voice_slot_fade_out(EAX=slot, EDX=250) }
    f = (u16) I[0x2E]
    if f & 0x100 {                           ; 2-D cue
        block: +0 word cue2 ; +0x0C = 1.0 ; +0x10 = 1.0 ; +0x1C..0x24 = 0 ; +0x30 word 0xFFFF ; +0x32 word 1 ; +0x4C = P
               +0x58 word = ((old ^ f) & 0x400 ^ old) | 0x2B0     (old = the zeroed word; i.e. bit 0x400 comes from f, 0x2B0 is set)
        audio_play -> copy into P+0x1F4
        if audio_voice_position_check(ECX=&P.pos, EDX=&P[0x1F4]) == 0 { P[0x1F4] = 0xFF }
    } else {                                 ; 3-D cue
        block: +0 word cue2 ; +0x0C = 1.0 ; +0x10 = 1.0 ; +0x1C..0x24 = P.pos ; +0x30 = 0xFFFF ; +0x32 = 1 ; +0x4C = P
               +0x58 word = ((f & 0x80) << 1) | (f & 0x400) | 1
        audio_play -> copy into P+0x1F4
    }
} else {                                     ; 0x507023
    f = (u16) I[0x2E]
    if (f & 0x100) == 0 and (f & 0x200) != 0 { if audio_voice_position_check(ECX=&P.pos, EDX=&P[0x1F4]) == 0 { P[0x1F4] = 0xFF } }
}
```
The `&0xFFFF`-style note: the cue returned by 0x503FE0 is `movzx ax`, stored in -0x9C.

### 8c. T and wobble (0x506EF9..0x5071F1)

```
Jt = E[0xD4] ; frac = f32(P[0x1C] / J[0x10]) -> -0x9C ; prev_frac = f32(old_age / J[0x10]) -> -0x94
if P[0x1DC] >= 0 (fldz ; fcomp ; test ah,0x41 ; JP not taken: 0 < v or 0 == v) {
    T = T_prev = f32(P[0x1DC] * J[0x10])          ; -0x70 and -0x11C
} else { T = P[0x1C] ; T_prev = old_age }          ; JP path (negative or NaN)
wobble (-0xD4..-0xCC) = default point
if byte[0x2526E2B] != 0 {
    a = [0x2526E4C] ; s = sinf(a) ; c = cosf(a)
    k = f32( 0.5f * 0.25 ) = 0.125
    w = ( s*k , 0*k , c*k ) each fstp f32 ; then each * I[0x20] (f32 stores)
}
```
Note: only `T` and `T_prev` are used as curve times later.

### 8d. Force-field phase (0x5071F2..0x507AB1)

Clock helper (3 copies, 0x507207, 0x507AB2, 0x50857x): the signed circular difference `CD(pt)`:
```
if pt < 0 : d = 1800000000
else if pt > NOW : d = pt - NOW ; if d > 900000000 { d += 0x94B62E00 }       ; add -1.8e9
else : c = NOW - pt ; if c > 900000000 { d = (pt - NOW) + 1800000000 } else { d = -c }
```
```
alive = 0 (-0x74) ; NOW = [0xE843AC]
if P[0x204] & 2 {
    d = CD(P[0x14])
    lim = (int) ftol( J[0x10]*1000.0 + 10000.0 )                         ; _ftol2_sse (EAX), truncates
    if !(-d < lim) { P[0x204] &= ~2 ; P[0x18] = (NOW > 1800000000) ? NOW - 1800000000 : NOW }
}
if P[0x204] & 2 {                                                         ; 0x507296
    for s in 0..J[0x20] (signed) {
        h = P[0x98 + 4*s] ; if h == -1 continue
        b = BODY[h*24] ; q = b[0xB8] ; if q != 0 and ((q[0x26] >> 2) & 3) != 0 { alive += 1 }
        if (vp = P[0x118 + 4*s]) != 0 {
            physics_body_get_transform(ESI=&tmpPos[ebp-0x444], EDI=&tmpMat[ebp-0x6A8], h)
            if audio_voice_position_check(ECX=&tmpPos, EDX=vp+0x30) == 0 { *(dword*)(vp+0x30) = 0xFF }
        }
    }
    if 1.0 > frac {                                                       ; fcomp ... test ah,0x41 ; jne skips (frac >= 1.0 or NaN skips)
        for k in 0..J[0x38] (signed) {                                    ; records at J[0x3C] + k*0x2C
            r = J[0x3C] + k*0x2C
            A  = curve_eval_scalar(ESI=r[4], T, r[8])                      ; -0x14C
            B  = curve_eval_scalar(ESI=r[0xC], T, r[0x10])                 ; -0x98
            if 0.001 > A or 0.001 > B continue                             ; fcom ... test ah,0x41 ; je (skips when strictly greater than; NaN passes)
            C  = f32( curve_eval_scalar(ESI=r[0x14], T, r[0x18]) / 100.0 ) ; -0x148 (fdiv qword 0xE5C190)
            B2 = f32(B*B)                                                  ; -0x1B4
            W1 = curve_eval_vec3(T, r[0x1C], r[0x20]) -> EDX=&[ebp-0xB8]
            W2 = curve_eval_vec3(T, r[0x24], r[0x28]) -> EDX=&[ebp-0xF8]
            W1w = W1 x P.mat + P.pos ;  W2w = W2 x P.mat                   ; row convention of vec3_mul_mat33: x' = m0*x + m3*y + m6*z with m at P+0x38
            for s in 0..J[0x20] {
                h = P[0x98 + 4*s] ; if h == -1 continue
                pb = physics_body_get_transform(h).pos                      ; ESI=&[ebp-0x23C], EDI=&[ebp-0x63C]
                dv = pb - W1w (f32 each) ; d2 = f32((dx*dx + dy*dy) + dz*dz)  ; -0x98
                if B2 < d2 continue                                         ; fcompp ; test ah,5 ; jnp skips ; equal and NaN fall through
                V = default point
                switch r[0] {                                               ; sub 1 / sub 1
                  case 1: V = dv ; vec3_normalize_safe(ESI=&V) ; M = mat33_from_forward(EAX=&V, ECX=&M) ; vec3_random_cone(ESI=&V, a = f32(1.0 - 0.5*C)) ... ; V = V' x M
                  case 2: V = W2w ; mat33_from_forward(EAX=&V, ECX=&M) ; vec3_random_cone ; V = V' x M
                  default: V stays the default point
                }
                g = 1 - clamp(d2/B2 , 0, 1)       ; x87 ratio of the f32 values; f<=0 or NaN -> 1.0 ; f>=1 -> 0.0
                dt2 = cutscene_blocking() ? [0xE84380] : [0xE8437C]
                R  = V*g ; R = R*A ; R = R*dt2                              ; three f32 steps ([ebp-0x15C], [ebp-0x198], [ebp-0x1F4])
                physics_body_apply_linear_impulse(ECX=h, &R)
                u = vec3_random_unit(ESI=&U) ; s2 = f32(f32(f32(g*A)*C)*dt2) ; U = U*s2 ; W=(U,0)
                body = BODY[h*24] ; physics_body_wake(ESI=body) ; body[0xD0].vtbl[0x14](ECX=body+0xD0, &W)
            }
        }
    }
}
if (P[0x204] & 2) and alive == 0 { P[0x204] &= ~2 ; P[0x18] = (NOW > 1800000000) ? NOW-1800000000 : NOW }     ; 0x507AB2
```
The cone call (type 1 and 2): the code does `sub esp,8; [esp+4] = 1.0f; [esp] = f32(1.0 - 0.5*C); call bde7c0 (ESI=&V)`; so the args are (1.0 - 0.5*C, 1.0). The direction is then multiplied by the basis (the 3x3 built by `mat33_from_forward`) with the 0xBE4B20 layout. In type 1 the basis comes from the normalised `dv`; in type 2 from `W2w` (not normalised). The basis multiply is the explicit x87 block at 0x5076BB..0x5077AB (type 1: rows at [ebp-0x2E4..]) and 0x5077B0..0x5078A5 (type 2: [ebp-0x29C..]); the expanded formulas (V' = (x,y,z) of the cone vector, M rows m0..m8): `V.x = m0*x + m3*y + m6*z`, `V.y = m1*x + m4*y + m7*z`, `V.z = m2*x + m5*y + m8*z` (check each fld offset in the listing before porting; the x87 order differs per component but each result is one fstp).

Portability: the whole of 8d depends on 11 unnamed or unported callees. Port last or leave to the original (see below).

### 8e. Emitter lists (0x507AB2..0x5080EE)

```
if P[0x204] & 0x80 goto 8f                                      ; culled-far
n = E[0xDC] ; if n <= 0 goto 8f
for i in 0..n {   ; -0x98 = &E[0xE0+4i], -0x84 = BUF + 4i
    L = E[0xE0 + 4*i] ; if L == 0 continue
    G = L[0] ; def = G[0x2C] + L[4]*0xC0     (lea ecx,[ecx+ecx*2] ; shl ecx,6)
    if E[0xCC] == 4 continue
    bi = def[0xC] (signed)
    if bi < 0 { NOBODY }
    else { h = P[0x98 + 4*bi]
           if h != -1 { PHYS } else { CURVE } }
    ; common: al = (0 <= P[0x1DC]) (fldz ; fcomp ; test ah,0x41 ; JP -> 0)
    live_total += effect_instance_update(...)   ; see below
}
```
CURVE (0x507B74): `rec2 = G[0x24] + bi*0x30`.
```
off_cur  = curve_eval_vec3(T,      rec2[0xC], rec2[0x10]) ; off_prev = curve_eval_vec3(T_prev, rec2[0xC], rec2[0x10])
q_cur    = curve_eval_quat(EAX=rec2[0x14], &out, T,      rec2[0x18]) ; q_prev = same with T_prev
mat_cur  = quat_to_mat33(q_cur) ; mat_prev = quat_to_mat33(q_prev)
cur_pos  = off_cur  x P.mat + P.pos(+0x2C)     ; [ebp-0x128]
prev_pos = off_prev x P.mat + P.pos(+0x2C)     ; [ebp-0x134]   (current P matrix and position for both)
mat_cur  = mat33_mul(A=P.mat (ECX), B=mat_cur)  ; (EAX=&[ebp-0x5F4], copied back to [ebp-0x32C])
mat_prev = mat33_mul(A=P.mat, B=mat_prev)       ; (EAX=&[ebp-0x618], copied to [ebp-0x414])
```
PHYS (0x507E23):
```
physics_body_get_transform(ESI=&cur_pos [ebp-0x128], EDI=&mat_cur [ebp-0x32C], h)
vel = physics_body_get_linear_velocity(ECX=h) ; ang = physics_body_get_angular_velocity(ECX=h)
dt3 = cutscene_blocking() ? [0xE84380] : [0xE8437C]
prev_pos = cur_pos - vel*dt3  (f32 steps)
len = vec3_normalize_safe_len(ESI=&ang)          ; XMM0
angle = f32(len * dt3)
R = mat33_from_axis_angle(EDI=&ang, ESI=&R, angle)
mat_prev = mat33_mul(A=R (ECX), B=mat_cur)       ; EAX=&[ebp-0x660]
if (G[0x2C] + L[4]*0xC0)[6] & 0x10 { prev_pos = cur_pos ; mat_prev = mat_cur }
```
NOBODY (0x50802B): `if def[6] & 0x10 { P[0x5C..0x64] = P[0x2C..0x34] ; P[0x68..0x8B] = P[0x38..0x5B] }` and the call uses P's own transforms: cur_pos = &P[0x2C], cur_mat = &P[0x38], prev_pos = &P[0x5C], prev_mat = &P[0x68].

`effect_instance_update` call (all three paths), stack arguments from the top (arg1 first):

| arg | value |
|---|---|
| 1 | L |
| 2 | &cur_mat |
| 3 | frac (f32, -0x9C) |
| 4 | &prev_pos |
| 5 | prev_frac (f32, -0x94) |
| 6 | looped (dword load of -0x144; only the low byte is defined) |
| 7 | &BUF[i] (the f32 accumulator for the list) |
| 8 | &wobble (-0xD4) |
| 9 | al = (P[0x1DC] >= 0), pushed as a full EAX (upper bytes stale) |
| 10 | T (f32) |
| 11 | P[0x0C] (part id) |

registers EAX = &cur_pos, EDX = &prev_mat; `add esp,0x2C`; result EAX added to the live total (-0x108).

### 8f. Lights (0x5080EF..0x508539)

```
; always runs: the `P[0x204] & 0x80` skip and the `n <= 0` skip of 8e both jump to 0x5080EF (esi = [0xE843AC] is reloaded there)
nl = J[0x40] ; if nl <= 0 goto 8g
for k in 0..nl {            ; handle ptr -0x68 = &P[0x19C + 4k], record -0x84 = J[0x44] + k*0x2C
    h = P[0x19C + 4k] ; obj = (h != 0 and LIGHT[(h & 0xFFFF)*0x134 + 0x2AE50E0][+8] == h) ? that : 0
    if obj == 0 continue
    rec = J[0x44] + k*0x2C ; bi = rec[0] (signed)
    if bi < 0 { pos = P.pos ; mat = &P.mat }
    else { b = P[0x98 + 4*bi]
           if b == -1 { rec2 = J[0x24] + bi*0x30
                         off = curve_eval_vec3(T, rec2[0xC], rec2[0x10]) ; q = curve_eval_quat(EAX=rec2[0x14], T, rec2[0x18]) ; m = quat_to_mat33(q)
                         pos = off x P.mat + P.pos ; mat = mat33_mul(A=P.mat, B=m) }
           else { physics_body_get_transform(ESI=&pos [ebp-0xAC], EDI=&mat [ebp-0x1DC], b) } }
    off2 = curve_eval_vec3(T, rec[0x14], rec[0x18]) -> world = off2 x mat + pos          ; stored at [ebp-0xE0..-0xD8]
    q2 = curve_eval_quat(EAX=rec[0x1C], T, rec[0x20]) ; m2 = quat_to_mat33 ; m3 = mat33_mul(A=mat, B=m2) ; matrix_orthonormalize(&m3)
    re-validate obj (same handle test) ;
        obj[0x10..0x18] = world ; obj[0x1C..0x3F] = m3 (9 dwords)
        if (s = obj[0x12C]) != 0 { mat44_from_mat33_pos(ECX=&m3, EDX=&tmp64, &off2) ; shape_set_world_matrix(ESI=[s], EAX=&tmp64) }
    inten = curve_eval_scalar(ESI=rec[0x24], T, rec[0x28])
    if P[0x204] & 8 { inten = f32( f32(P[0x1E0] / 0.6) * inten ) }               ; fdiv qword 0xE5C550 then fmul
    re-validate obj ; obj[0x64] = inten
}
```
(The mat44 builder is given `&off2` at [ebp-0x140], i.e. the light's world position copy; the 0x508482/0x50848B call has ECX = &m3, EDX = &[ebp-0x60].)

### 8g. End of the part (0x50853A..0x5086A3)

```
if J[0x10] > P[0x1C] -> keep                                    ; fcompp ; test ah,0x41 ; je keep (strict >, NaN goes on)
if alive > 0 (unsigned, -0x74) -> keep
if P.next == P and live_total > 0 (signed) -> keep               ; sole part still has live instances
if P[0x18] >= 0 {
    d = CD(P[0x18]) (same helper) ; elapsed = -d
    if elapsed < 1500 (signed) -> keep
}
if 0 < P[0x1E0] (fldz ; fcomp ; test ah,5 ; jnp) -> keep         ; ending timer still running (NaN does not keep)
if E[0xCC] == 4 -> keep
DESTROY_PART(E,P) ; P = next ; goto loop top
keep: kept += 1 ; P = next ; goto loop top
```
Loop top is 0x506A03: `test edi,edi; je S9`.

## S9. Tail (0x5086A4..0x508706)

```
E[0x340] = live_total
for i in 0..E[0xDC]: if E[0xE0+4i] != 0 { *(f32*)(E[0xE0+4i] + 0xA4) = BUF[i] }       ; bit copy via fld/fstp
if kept <= 0 (signed) and live_total <= 0 (signed) and E[0xCC] != 2 { (*E.vtbl[4])(ECX=E, 0) }   ; push 0 ; call [vtbl+0x10] ; callee pops 4
```
vtbl slot 4 is 0x50B3A0 (named `effect_object::activate`, wrong: if `!(E[0x80] & 0xB)` it sets `E[0x80] |= 2` and tail-calls 0x925780; it requests destruction). `E[0x340]` is therefore the LIVE count.

## 3. Porting order and what is not portable yet

| section | status | reason |
|---|---|---|
| S0, S1, S3, S5, S7, S9 | portable now | only named callees; the stop callee 0x50AB60 is called through its address with EDI = E |
| S2 | portable now | callees are called through their addresses with the registers in the table; the default-owner case needs the live vtbl[1] call |
| S4, S6 | portable now | the box maths is plain f32/f64; three callees by address |
| S8a, S8b, S8c, S8g | portable now | audio/cue callees are unported but have complete register contracts |
| S8e, S8f | portable, long | call the curve / matrix helpers through their addresses; port the helpers later |
| S8d | not yet | 11 helpers (curves, random, basis matrices, velocity / impulse, body wake) and the type-1 / type-2 basis maths; the per-component fld offsets of 0x5076BB..0x5078A5 should be re-read from the listing when porting |
| frame as a whole | one replacement | the frame is 0x6A8 bytes, ESI/EDI/EBX are callee-saved, the alloca buffer is below the fixed part; port as one Rust function with the phases in order, keeping `next` computed before the body in S2 and S8 |

## 4. Errors found in existing docs

- Doc 33, "2.": the 0x50AB60 condition is inverted. The call is made when (`E[0x358] == 0` and `I[0x2E] & 0x20` is CLEAR) or (`E[0x358] != 0` and `E[0xCC] != 1`).
- Doc 33, "4.", "12.2", and the CSV note for 0x50F780: the values summed and stored at E+0x340 are live counts, not expired counts.
- Doc 33, "8.": the player box test returns when the centre is INSIDE the box (all six strict compares pass, the lower y bound is `min.y - 1.0`).
- Doc 33, "7.": a vehicle list hit needs the centre within 10 m (d2 < 100.0) and inside a 0.8 x (ext.y + 1.0) x 0.8 local box; the check is skipped when `I[0x2E] & 4` is clear.
- Doc 33, "10.": the 40 m rule also stops the effect (stop(0,0), E+0x354 = -1) and applies unless `[0x2528B48] == 1 and [0x2528B49] == 0`.
- Doc 33, "11.2": expiry is `J[0x10] < age` with a kind-dependent branch (kinds 0..2 count P+0x28 down, kinds 3 and 4 count it up and wrap); the doc's "age > (E+0xD4)+0x10" omits the loop counter and the wrap loop.
- CSV 0x50B3A0 `effect_object::activate` (vtbl slot 4): it requests destruction (sets E+0x80 bit 1 and tail-calls 0x925780).
- CSV 0x6FC760 `physics_body_get_transform`: it has two outputs, ESI = &pos and EDI = &mat.
- CSV 0x6FE460 `physics_query_aabb`: ECX = &min; stack = (&max, &out, max count 10, filter 0x8005E, 0, 0).
- CSV 0x524550 `render_views_test_bounds` argument order: the caller here passes (arg1 = &second box, arg2 = &first box) relative to the a020c0 call; check which one is the minimum before relying on the "(&min, &max)" wording. Also the result polarity: this function marks the part hidden when it returns 1.
- CSV 0xCEDF10 `rig_bone_transform`: the fifth argument is a matrix output pointer, not a flag.
- CSV 0x98D3C0 `human_knockdown`: 5 stack args (human, force, 0, 0, 0), ECX = 0.
- CSV 0x960D60 `vehicle_seat_occupant`: doc 41 already corrects it (handle -> human); doc 33 and the old port note still call 0x9C4430 / 0x9C44C0 seat calls.
