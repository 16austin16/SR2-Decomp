# 58 - Mission tss02 render thread (logs 2026-11-04c / 04d)

Inputs: `bin/sr2-mission-2026-11-04c.log` and `...04d.log`. The `d` file is the `c` file plus one more short session (two
`game none` sections, frames 3522-4145, label "Test324"), so everything below uses `d`. The render thread is the thread that runs
`game_render_thread_frame` -> `render_frame` (0x52A710) and records the GPU command stream at `[0x33D62F8]`. Scripts:
`scratchpad/log_render/*.py` (call-site sweep, owner mapping, digests). No game was run, no REA tools used.

## 1. Method and caveats

- Samples are mapped to the real containing function, not the nearest named one: a function start is every `functions.csv`
  address plus every 16-aligned address that follows an `int3` run, plus two hand-added starts that are packed without padding
  (0x5240F0, 0xA90B40). Unaligned `0xCC` bytes inside instructions (`d8 cc`) are ignored; with the naive rule 0xCF696F, 0xCF6947,
  0xCB70DB and 0xD0698B look like functions but are inside `morph_target_apply` / the CRT / `mesh_render_*`.
  Every address named below was checked: real prologue, preceded by `int3` (or a `ret`), at least one `call rel32` caller (one
  exception: 0x5240F0 follows a `ret 4` without padding), not in `functions.csv`, name unused.
- Ported Rust code is not sampled: `frustum_test_aabb`, `view_bounds_visible`, `normal_pack_ubyte4`, `city_instance_add / submit /
  batch_begin` and `city_submit_chunk_extras` are absent. The `RUST` rows (153 samples) are ported functions that still run a
  few original instructions.
- Sections are F8 intervals. Two logging sessions are present: session A (`game tss02`, frames 3481-10736, one unlabelled
  section) and session B (frames 4901-34946, labels `cutscene start`, `before entering courthouse`, `before mission fail`,
  `retried from checkpoint`, `mission ended`). A label names the phase that starts at that F8.
- Section sample totals are game + render; the render thread holds 35 % to 90 % of them in the tss02 phases (55 % in P1, 61 % P2, 63 % P3, 47 % P5).

## 2. Render thread per phase

P1 = tss02 intro / free play, P2 = the cutscene (morph and animated mesh heavy), P3 = walk to the courthouse, P4 = fail,
P5 = checkpoint retry, P6 / P7 = after the mission. Classes: `named` = already in `functions.csv`, `NEW` = named in this round
(`docs/candidates_log58.csv`), `unnamed`, `RUST` = ported.

### P1 tss02 start, unlabelled (session A, frames 3481-10736)

Render thread: 12753 samples over 7255 frames (1.76 per frame); game thread 10447.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 47.2 | 6019 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 2.9 | 365 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 2.6 | 326 | 0x00CA0850 | _memcpy | named |
| 1.8 | 225 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 1.7 | 211 | 0x005117D0 | r_particle_inst::pre_render | named |
| 1.6 | 208 | 0x00D06430 | mesh_render_generic | named |
| 1.5 | 191 | 0x00BD5C30 | lower_bound_key64_rec16 | NEW |
| 1.1 | 138 | 0x00D049E0 | gfx_bind_material_textures | named |
| 1.1 | 134 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 1.0 | 129 | 0x00530520 | mesh_material_flags_or | NEW |
| 1.0 | 125 | 0x00D1A860 | gfx_bind_shaders | named |
| 1.0 | 122 | 0x00BDE500 | mat33_mul_vec3_in_place | NEW |

### P1b tss02 start, unlabelled (session B, frames 4901-5228)

Render thread: 402 samples over 327 frames (1.23 per frame); game thread 46.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 69.9 | 281 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 9.7 | 39 | 0x00CF6840 | morph_target_apply | named |
| 6.5 | 26 | 0x00DA0140 | _ftol2_sse | named |
| 5.7 | 23 | 0x00BDB460 | normal_ubyte4_add | named |
| 1.5 | 6 | 0x00CA0850 | _memcpy | named |
| 0.5 | 2 | 0x00BDDFF0 | vec3_normalize_in_place | named |
| 0.5 | 2 | 0x00C0BE60 | ? | unnamed |
| 0.5 | 2 | 0x00CF64A0 | morph_target_apply_positions | NEW |
| 0.5 | 2 | 0x00D097F0 | rl_lod_mesh_inst::select_lod_entry | named |
| 0.5 | 2 | 0x00D1B100 | gfx_set_material_constants | named |
| 0.2 | 1 | 0x0040EA20 | sqrtf_wrapper | named |
| 0.2 | 1 | 0x004B6A20 | city_chunk_tree_cull_frustum | NEW |

### P2 "cutscene start" (frames 5228-14629)

Render thread: 14620 samples over 9401 frames (1.56 per frame); game thread 9316.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 50.1 | 7323 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 4.9 | 723 | 0x00CF6840 | morph_target_apply | named |
| 3.3 | 486 | 0x00DA0140 | _ftol2_sse | named |
| 2.2 | 325 | 0x00BDB460 | normal_ubyte4_add | named |
| 1.7 | 250 | 0x00CA0850 | _memcpy | named |
| 1.5 | 224 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 1.2 | 178 | 0x00BD5C30 | lower_bound_key64_rec16 | NEW |
| 1.0 | 148 | 0x004B72E0 | city_submit_visible_instances | named |
| 1.0 | 145 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 0.9 | 133 | 0x004C5180 | glare_occlusion_query_update | NEW |
| 0.8 | 121 | 0x00D06430 | mesh_render_generic | named |
| 0.8 | 119 | 0x00CF64A0 | morph_target_apply_positions | NEW |

### P3 "before entering courthouse" (frames 14629-20864)

Render thread: 8867 samples over 6235 frames (1.42 per frame); game thread 5277.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 52.5 | 4658 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 9.0 | 800 | 0x00CF6840 | morph_target_apply | named |
| 5.7 | 506 | 0x00DA0140 | _ftol2_sse | named |
| 3.5 | 307 | 0x00CF64A0 | morph_target_apply_positions | NEW |
| 3.4 | 305 | 0x00BDB460 | normal_ubyte4_add | named |
| 1.6 | 138 | 0x004B5C40 | city_cull_chunk_instances | NEW |
| 1.5 | 129 | 0x004B6A20 | city_chunk_tree_cull_frustum | NEW |
| 1.0 | 88 | 0x004B72E0 | city_submit_visible_instances | named |
| 1.0 | 85 | 0x00CA0850 | _memcpy | named |
| 0.8 | 74 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 0.7 | 65 | 0x004B7CC0 | city_build_render_batches | named |
| 0.7 | 60 | 0x00A8FAD0 | ? | unnamed |

### P4 "before mission fail" (frames 20864-22891)

Render thread: 2078 samples over 2027 frames (1.03 per frame); game thread 3810.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 51.1 | 1062 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 1.8 | 38 | 0x004C5180 | glare_occlusion_query_update | NEW |
| 1.7 | 35 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 1.7 | 35 | 0x00532810 | render_env_cubemap | named |
| 1.5 | 32 | 0x004B6A20 | city_chunk_tree_cull_frustum | NEW |
| 1.5 | 32 | 0x005117D0 | r_particle_inst::pre_render | named |
| 1.4 | 29 | 0x004B6160 | city_instances_cull_view | named |
| 1.2 | 25 | 0x00C0BE60 | ? | unnamed |
| 1.2 | 25 | 0x00D06430 | mesh_render_generic | named |
| 1.1 | 23 | 0x004F4E60 | memcmp_regs | NEW |
| 1.1 | 22 | 0x004B7CC0 | city_build_render_batches | named |
| 1.0 | 20 | 0x004B5C40 | city_cull_chunk_instances | NEW |

### P4b mission-fail screen, game none (frames 22891-23161)

Render thread: 265 samples over 270 frames (0.98 per frame); game thread 87.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 30.6 | 81 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 5.3 | 14 | 0x004B6160 | city_instances_cull_view | named |
| 3.8 | 10 | 0x005117D0 | r_particle_inst::pre_render | named |
| 3.4 | 9 | 0x00532810 | render_env_cubemap | named |
| 3.4 | 9 | 0x00A8FAD0 | ? | unnamed |
| 2.6 | 7 | 0x00C0BE60 | ? | unnamed |
| 2.3 | 6 | 0x0052F910 | world_mesh_submit | named |
| 1.9 | 5 | 0x004C5180 | glare_occlusion_query_update | NEW |
| 1.9 | 5 | 0x00D06430 | mesh_render_generic | named |
| 1.9 | 5 | 0x00D18610 | gr_index_buffer::Lock | named |
| 1.5 | 4 | 0x004B5C40 | city_cull_chunk_instances | NEW |
| 1.5 | 4 | 0x004B6E10 | city_instances_pass_a | named |

### P5a retry loading, tss02 (frames 23161-23303)

Render thread: 182 samples over 142 frames (1.28 per frame); game thread 74.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 67.0 | 122 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 5.5 | 10 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 2.7 | 5 | 0x004B72E0 | city_submit_visible_instances | named |
| 2.7 | 5 | 0x005117D0 | r_particle_inst::pre_render | named |
| 2.7 | 5 | 0x00A8FAD0 | ? | unnamed |
| 1.6 | 3 | 0x004B7CC0 | city_build_render_batches | named |
| 1.1 | 2 | 0x004B6A20 | city_chunk_tree_cull_frustum | NEW |
| 1.1 | 2 | 0x0052E840 | render_list_add | named |
| 1.1 | 2 | 0x00CA0BC0 | _memset | named |
| 1.1 | 2 | 0x00D09780 | rl_lod_mesh_inst::get_shadow_lod_entry | named |
| 0.5 | 1 | 0x004B52F0 | city_instance_record_init | named |
| 0.5 | 1 | 0x004B5C40 | city_cull_chunk_instances | NEW |

### P5 "retried from checkpoint" (frames 23303-32341)

Render thread: 15470 samples over 9038 frames (1.71 per frame); game thread 17170.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 45.1 | 6978 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 2.8 | 427 | 0x00CA0850 | _memcpy | named |
| 2.3 | 355 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 1.9 | 300 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 1.4 | 223 | 0x00D06430 | mesh_render_generic | named |
| 1.4 | 219 | 0x005117D0 | r_particle_inst::pre_render | named |
| 1.4 | 214 | 0x00D1B100 | gfx_set_material_constants | named |
| 1.2 | 190 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 1.2 | 190 | 0x00BD5C30 | lower_bound_key64_rec16 | NEW |
| 1.2 | 182 | 0x004C5180 | glare_occlusion_query_update | NEW |
| 1.1 | 177 | 0x00D049E0 | gfx_bind_material_textures | named |
| 1.1 | 163 | 0x00D1A860 | gfx_bind_shaders | named |

### P6 after the mission, game none (frames 32341-34731)

Render thread: 3596 samples over 2390 frames (1.50 per frame); game thread 1396.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 55.3 | 1990 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 3.1 | 111 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 2.2 | 79 | 0x00CA0850 | _memcpy | named |
| 2.1 | 77 | 0x004B72E0 | city_submit_visible_instances | named |
| 1.6 | 56 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 1.4 | 52 | 0x004B6CC0 | city_chunk_tree_cull_view_bounds | NEW |
| 1.1 | 40 | 0x004B5FF0 | city_view_chunk_instances_cull | NEW |
| 1.1 | 39 | 0x004B6B60 | city_chunk_tree_cull_frustum_dual | NEW |
| 1.0 | 36 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 0.9 | 34 | 0x00D1B100 | gfx_set_material_constants | named |
| 0.9 | 32 | 0x00530520 | mesh_material_flags_or | NEW |
| 0.9 | 31 | 0x00D06430 | mesh_render_generic | named |

### P7 "mission ended" (frames 34731-34946)

Render thread: 297 samples over 215 frames (1.38 per frame); game thread 343.

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 57.6 | 171 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 1.7 | 5 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 1.7 | 5 | 0x004B6B60 | city_chunk_tree_cull_frustum_dual | NEW |
| 1.7 | 5 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 1.7 | 5 | 0x00BF4C30 | ? | unnamed |
| 1.7 | 5 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 1.7 | 5 | 0x00D23660 | gfx_transform_push_cached | named |
| 1.3 | 4 | 0x005117D0 | r_particle_inst::pre_render | named |
| 1.3 | 4 | 0x0052DE10 | ? | unnamed |
| 1.3 | 4 | 0x00BD5C30 | lower_bound_key64_rec16 | NEW |
| 1.3 | 4 | 0x00BE61B0 | mat44_mul_sse | NEW |
| 1.3 | 4 | 0x00CA0850 | _memcpy | named |

### All ten tss02 / post-mission render phases together (58530 render samples)

| % | samples | address | function | class |
|---:|---:|---|---|---|
| 49.0 | 28685 | 0x00CA326E | __VEC_memcpy_fastcopy | named |
| 2.7 | 1584 | 0x00CF6840 | morph_target_apply | named |
| 2.0 | 1194 | 0x00CA0850 | _memcpy | named |
| 1.8 | 1069 | 0x004B5CE0 | city_cull_chunk_instances_split | NEW |
| 1.8 | 1046 | 0x00DA0140 | _ftol2_sse | named |
| 1.4 | 791 | 0x00C094E0 | texture_entry_animated_resolve | NEW |
| 1.1 | 662 | 0x00BDB460 | normal_ubyte4_add | named |
| 1.1 | 659 | 0x00D06430 | mesh_render_generic | named |
| 1.1 | 621 | 0x00BD5C30 | lower_bound_key64_rec16 | NEW |
| 1.0 | 583 | 0x004B7650 | view_distance_sq_scaled | NEW |
| 1.0 | 580 | 0x004B72E0 | city_submit_visible_instances | named |
| 1.0 | 576 | 0x005117D0 | r_particle_inst::pre_render | named |
| 0.9 | 501 | 0x00D1B100 | gfx_set_material_constants | named |
| 0.8 | 478 | 0x004C5180 | glare_occlusion_query_update | NEW |
| 0.8 | 457 | 0x00D049E0 | gfx_bind_material_textures | named |
| 0.7 | 433 | 0x00CF64A0 | morph_target_apply_positions | NEW |
| 0.7 | 425 | 0x00D1A860 | gfx_bind_shaders | named |
| 0.7 | 391 | 0x00530520 | mesh_material_flags_or | NEW |
| 0.6 | 377 | 0x00D05130 | mesh_render_depth | named |
| 0.6 | 369 | 0x00D01DA0 | gfx_material_vs_constants | named |
| 0.6 | 361 | 0x00CF8C90 | rl_mesh_inst::render | named |
| 0.6 | 356 | 0x00BDE500 | mat33_mul_vec3_in_place | NEW |
| 0.6 | 350 | 0x004B6B60 | city_chunk_tree_cull_frustum_dual | NEW |
| 0.6 | 350 | 0x00D01D20 | gfx_material_ps_constants | named |
| 0.6 | 347 | 0x00A8FAD0 | ? | unnamed |
| 0.6 | 333 | 0x00D06940 | mesh_render_pass7 | named |
| 0.6 | 328 | 0x00BF65F0 | aabb_point_dist_sq_x87 | NEW |
| 0.5 | 311 | 0x005240F0 | render_item_less | NEW |
| 0.5 | 308 | 0x00CF8980 | rl_mesh_inst::pre_render | named |
| 0.5 | 300 | 0x00D19E30 | shader_param_write_floats | NEW |
| 0.5 | 296 | 0x00D01480 | gfx_cmd_set_bone_palette | NEW |
| 0.5 | 285 | 0x004B5C40 | city_cull_chunk_instances | NEW |
| 0.5 | 276 | 0x004B7CC0 | city_build_render_batches | named |
| 0.5 | 270 | 0x00D18EE0 | gfx_cmd_set_world_matrices | NEW |
| 0.5 | 264 | 0x004B6CC0 | city_chunk_tree_cull_view_bounds | NEW |
| 0.4 | 250 | 0x00BDDFF0 | vec3_normalize_in_place | named |
| 0.4 | 247 | 0x004B6A20 | city_chunk_tree_cull_frustum | NEW |
| 0.4 | 232 | 0x004B7070 | city_instances_pass_b | named |
| 0.4 | 228 | 0x00D097F0 | rl_lod_mesh_inst::select_lod_entry | named |
| 0.4 | 226 | 0x004B5FF0 | city_view_chunk_instances_cull | NEW |
| 0.4 | 225 | 0x00CFFF20 | ? | unnamed |
| 0.4 | 225 | 0x00D09780 | rl_lod_mesh_inst::get_shadow_lod_entry | named |
| 0.4 | 224 | 0x00D19D00 | gfx_shader_pick_lod | named |
| 0.4 | 208 | 0x00BE61B0 | mat44_mul_sse | NEW |
| 0.3 | 204 | 0x0052F910 | world_mesh_submit | named |

Class share of those samples (after this round's names):

| class | samples | share |
|---|---:|---:|
| named | 44420 | 75.9 % |
| NEW | 9342 | 16.0 % |
| unnamed | 4615 | 7.9 % |
| RUST | 153 | 0.3 % |

Aggregates by address range (same samples): the copy family (`__VEC_memcpy_fastcopy` + `_memcpy`) 51 %; the D3D command layer
`gfx_*` (0xD00000-0xD30000, without the copy) 12.4 % (7232 samples); the city visibility / submit chain (0x4B5000-0x4BA000)
8.9 % (5182); math leaves 0xBD0000-0xC10000 7.7 % (4480); animated / morph mesh classes 0xCF0000-0xD00000 6.2 % (3604);
`render_frame` stage code 0x520000-0x540000 4.5 % (2638); chunk-section-0A tree renderers 1.8 % (1033); glares 1.3 % (787).
The morph chain (`morph_target_apply` 1584, `morph_target_apply_positions` 433, `normal_ubyte4_add` 662, `_ftol2_sse` 1046) is 6.4 %
overall, 11 % of P2, 22 % of P3 and 22 % of the first section of session B (frames 4901-5228): the cutscene characters are rebuilt on
the CPU every frame, and outside the cutscene the chain is 0.1 % to 0.3 %.

## 3. The memcpy

`__VEC_memcpy_fastcopy` (0xCA326E, the 16-byte-aligned inner loop of `_memcpy` 0xCA0850 -> `__VEC_memcpy` 0xCA32F5) is
28685 of 58530 render samples (49.0 %). It is 45 % to 55 % in every long phase (P1 47 %, P2 50 %, P3 53 %, P5 45 %, P6 55 %)
and 70 % only where the rest of the thread is idle (first part of session B, retry loading).

The loop itself: 22980 of the 28685 samples (80 %) sit on two addresses, 0xCA329C (11003) and 0xCA32C3 (11977), the instruction
after `movdqa xmm2,[esi+0x20]` and after `movdqa xmm7,[esi+0x70]` of the 128-byte iteration. That is the skid of a source load
that misses the cache (one miss per 64-byte line), so the loop is bound by the source memory and the write-combined destination,
not by ALU work.

How it is reached: `_memcpy` and `_memmove` (0xC96060) jump to `__VEC_memcpy` only when length >= 0x100, `[0x34A536C]` (SSE2) is
set and `(dst & 15) == (src & 15)`. It copies a misaligned head with `rep movsb`, the `len & ~0x7F` body through 0xCA326E and the tail
with `rep movsb`. Callers on the render thread (static sweep of all 473 `_memcpy` call sites joined with the render samples;
the log has no call stacks, so who calls how often is inferred, not counted):

| rank | caller (real function) | sites | size | destination | fast path | render samples in the caller |
|---:|---|---|---|---|---|---:|
| 1 | `dynamic_buffers_upload` 0xD00F20 (NEW), once per frame from `render_frame` 0x52A82E | 0xD00F58, 0xD00FA8 | `[0x2305948]` bytes (all dynamic vertex data of the frame, capacity 0xA00000) and `[0x252A340]*2` bytes (index data, capacity 0x400000) | locked D3D buffers: `Lock(0, size, &p, 0x2800 = DISCARD + NOSYSLOCK)`, vtable +0x2C / +0x30 (write-combined memory) | yes | 30 |
| 2 | `rl_animated_mesh_inst::pre_render` 0xCF7DE0 | 0xCF7F3D (n*12 bytes), 0xCF8043 (chunks of up to 0x400 vertices = 12 KB from the scratch 0x3490D70), 0xCF80EB, 0xCF818B (n*8), 0xCF825D | 0.1 KB to 12 KB | blocks from `dynamic_vertex_alloc` 0xD016E0 | the 12 KB chunks | 80 (+ the morph chain) |
| 3 | `rl_morph_mesh_inst::pre_render` 0xCF6EC0, `r_dynamic_water_simple/detailed_inst::pre_render` 0xB76B90 / 0xB788B0, 0xCFC550, 0xCF5B20 | producers into the same staging buffer through `dynamic_vertex_alloc` | tens of KB | `[0x230593C]` staging buffer | filled with SSE / x87 stores, not by the copy | 100 / 11 / 30 / 145 / 4 |
| 4 | command writers `gfx_material_vs/ps_constants` 0xD01DA0 / 0xD01D20, `gfx_bind_material_textures` 0xD049E0, 0xD01480, 0xD18EE0, `r_tree_inst::render` (0x100, 0x100, 0x300 bytes) | many | 16 B to 0xC00 B | the command stream (payload at stream + 0xC, low nibble 0xC) | rarely | 367 / 348 / 456 / 296 / 267 / 48 |

Whom the memcpy serves: the render thread's dynamic geometry. Producers (water meshes, morph meshes, animated meshes,
0xCFC550) take space from the CPU staging buffer with an interlocked add on `[0x2305948]` (`dynamic_vertex_alloc`), and
`render_frame` then copies the whole used range once into the locked D3D vertex and index buffers (`dynamic_buffers_upload`).
This is an inference from the static code and the flat per-frame profile, with three supports:

1. The command-stream copies are at most 0xC00 bytes with payload at `stream + 0xC`; only copies of >= 0x100 bytes with equal low
   nibbles reach the fast path. The only copy that is MB sized is the buffer upload.
2. The per-frame cost is constant across scenes (table below): 0.71 to 0.92 fastcopy samples per frame in 17 of 19 sections,
   whether the screen is the character creator, a cutscene (morph chain 0.18 to 0.31 per frame) or free play (morph chain 0.01).
   Only the mission-fail screen drops (0.52 and 0.30), where the dynamic meshes are not built. A copy that followed the cutscene
   content would move with the morph chain; a fixed per-frame upload does not.
3. The two hot source-load addresses match data written a moment earlier by other cores (the pre-render jobs), so every line comes
   from a remote cache or from memory.

Per section, render samples per frame and the fastcopy cost per frame:

| section | game | frames | render samples | per frame | fastcopy per frame | fastcopy share | morph chain per frame |
|---|---|---:|---:|---:|---:|---:|---:|
| 0 point 0 | none | 510 | 1025 | 2.01 | 0.71 | 36 % | 0.01 |
| 1 point 0 | none | 290 | 767 | 2.64 | 0.84 | 32 % | 0.00 |
| 2 point 1 "Test123@%$" | none | 445 | 944 | 2.12 | 0.75 | 35 % | 0.00 |
| 3 point 0 | none | 332 | 910 | 2.74 | 0.84 | 31 % | 0.00 |
| 4 point 1 "Pre mission start" | none | 116 | 307 | 2.65 | 0.84 | 32 % | 0.00 |
| 5 point 0 | none | 438 | 838 | 1.91 | 0.82 | 43 % | 0.01 |
| 6 point 1 "Pre mission start" | none | 78 | 148 | 1.90 | 0.92 | 49 % | 0.01 |
| 7 point 0 | tss02 | 7255 | 12753 | 1.76 | 0.83 | 47 % | 0.01 |
| 8 point 0 | tss02 | 327 | 402 | 1.23 | 0.86 | 70 % | 0.28 |
| 9 point 1 "cutscene start" | tss02 | 9401 | 14620 | 1.56 | 0.78 | 50 % | 0.18 |
| 10 point 2 "before entering courthouse" | tss02 | 6235 | 8867 | 1.42 | 0.75 | 53 % | 0.31 |
| 11 point 3 "before mission fail" | tss02 | 2027 | 2078 | 1.03 | 0.52 | 51 % | 0.01 |
| 12 point 3 | none | 270 | 265 | 0.98 | 0.30 | 31 % | 0.00 |
| 13 point 0 | tss02 | 142 | 182 | 1.28 | 0.86 | 67 % | 0.00 |
| 14 point 1 "retried from checkpoint" | tss02 | 9038 | 15470 | 1.71 | 0.77 | 45 % | 0.01 |
| 15 point 1 | none | 2390 | 3596 | 1.50 | 0.83 | 55 % | 0.01 |
| 16 point 2 "mission ended" | none | 215 | 297 | 1.38 | 0.80 | 58 % | 0.00 |
| 17 point 0 | none | 408 | 596 | 1.46 | 0.77 | 53 % | 0.00 |
| 18 point 1 "Test324" | none | 215 | 325 | 1.51 | 0.81 | 54 % | 0.01 |

Hooks for the next round: log `[0x2305948]` and `[0x252A340]` at 0xD00F20 (bytes per frame) and a caller histogram on 0xCA0850
for length >= 0x100; that turns the inference above into a count.

## 4. Comparison with the earlier free-roam log (doc 56, clothes store, 17692 render samples)

| item | doc 56 (free roam) | this log (58530) | note |
|---|---|---|---|
| `__VEC_memcpy_fastcopy` | 7768 (43.9 %) | 28685 (49.0 %) | P1 (the free-play part of tss02) alone is 47 % |
| `_memcpy` | 406 (2.3 %) | 1194 (2.0 %) | unchanged |
| `frustum_test_aabb` | 601 (3.4 %) | 0 | ported this week |
| 0x4B5CE0 (doc 56: "city instance add") | 528 (3.0 %) | 1069 (1.8 %), now `city_cull_chunk_instances_split` | doc 56 mislabeled it: `city_instance_add` is 0x4B5A20 (ported) |
| `r_particle_inst::pre_render` | 281 (1.6 %) | 576 (1.0 %) | |
| `mesh_render_generic` | 277 (1.6 %) | 659 (1.1 %) | |
| morph chain (`morph_target_apply`, `morph_target_apply_positions`, `normal_ubyte4_add`, `_ftol2_sse`) | absent | 3725 (6.4 %), 11 % to 22 % of the cutscene phases | tss02 specific |
| `glare_occlusion_query_update` | not in the top | 478 (0.8 %) | a D3D occlusion-query poll per glare per frame |
| `lower_bound_key64_rec16`, `texture_entry_animated_resolve`, `render_item_less` | absent | 1.1 %, 1.4 %, 0.5 % | lookup and sort leaves |

The copy share is the same in both logs (44 % to 49 %): it belongs to the engine's fixed per-frame work (dynamic buffer upload),
not to the clothes store or to the mission. What tss02 adds is the morph / animated mesh chain in the cutscene phases.

## 5. New names (41 rows, `docs/candidates_log58.csv`)

Conventions: lower snake_case, subsystem prefix as in the existing render names (`city_*`, `glare_*`, `gfx_cmd_*` for functions that
append to the command stream at `[0x33D62F8]`, `shader_param_*`, `world_mesh_record_*`), math leaves named by what they compute
(`aabb_point_dist_sq_sse`, `mat44_mul_sse`). Status `documented` = confirmed from the instructions, `named` = inferred from the
shape and callers. Sorted by render samples in the ten phases above.

| address | name | samples (all tss02 + post phases) | confirmed / inferred |
|---|---|---:|---|
| 0x004B5CE0 | city_cull_chunk_instances_split | 1069 | confirmed |
| 0x00C094E0 | texture_entry_animated_resolve | 791 | inferred |
| 0x00BD5C30 | lower_bound_key64_rec16 | 621 | confirmed |
| 0x004B7650 | view_distance_sq_scaled | 583 | confirmed |
| 0x004C5180 | glare_occlusion_query_update | 478 | confirmed |
| 0x00CF64A0 | morph_target_apply_positions | 433 | inferred |
| 0x00530520 | mesh_material_flags_or | 391 | confirmed |
| 0x00BDE500 | mat33_mul_vec3_in_place | 356 | confirmed |
| 0x004B6B60 | city_chunk_tree_cull_frustum_dual | 350 | inferred |
| 0x00BF65F0 | aabb_point_dist_sq_x87 | 328 | inferred |
| 0x005240F0 | render_item_less | 311 | confirmed |
| 0x00D19E30 | shader_param_write_floats | 300 | inferred |
| 0x00D01480 | gfx_cmd_set_bone_palette | 296 | inferred |
| 0x004B5C40 | city_cull_chunk_instances | 285 | confirmed |
| 0x00D18EE0 | gfx_cmd_set_world_matrices | 270 | inferred |
| 0x004B6CC0 | city_chunk_tree_cull_view_bounds | 264 | inferred |
| 0x004B6A20 | city_chunk_tree_cull_frustum | 247 | inferred |
| 0x004B5FF0 | city_view_chunk_instances_cull | 226 | confirmed |
| 0x00BE61B0 | mat44_mul_sse | 208 | confirmed |
| 0x00D098B0 | world_mesh_record_set_lod_list | 174 | inferred |
| 0x004B7870 | city_submit_chunk_object_list | 171 | inferred |
| 0x004C7050 | glares_render_collect | 156 | confirmed |
| 0x004F4E60 | memcmp_regs | 134 | confirmed |
| 0x00B779C0 | dynamic_water_submit | 115 | inferred |
| 0x004C7290 | glares_render_draw | 109 | inferred |
| 0x004B54A0 | city_view_instance_filter_test | 95 | inferred |
| 0x00A90B10 | plane_eval_xyz_ptrs | 78 | inferred |
| 0x00530150 | world_mesh_record_set_params | 65 | inferred |
| 0x00BF63B0 | aabb_point_dist_sq_sse | 60 | confirmed |
| 0x00D18E50 | gfx_cmd_set_const5_vec3_cached | 53 | inferred |
| 0x00D1AD50 | shader_param_set | 51 | inferred |
| 0x005340C0 | scene_handle_table_find_or_add | 49 | inferred |
| 0x00CF2600 | morph_mesh_color_fill_indexed | 45 | inferred |
| 0x00D1ACA0 | shader_param_set_by_id | 41 | inferred |
| 0x004C54E0 | glare_build_draw_entry | 40 | inferred |
| 0x00D00F20 | dynamic_buffers_upload | 30 | confirmed |
| 0x00D22A00 | gfx_eye_facing_test | 27 | inferred |
| 0x00BF0E00 | float_near_equal | 14 | inferred |
| 0x00D01000 | gfx_sampler_slot_set | 13 | inferred |
| 0x00BF60D0 | vec3_sign_octant | 13 | inferred |
| 0x00D016E0 | dynamic_vertex_alloc | 2 | confirmed |

`renames_log58.csv` has no rows: no existing name was found to be wrong. Corrections to notes, not names: doc 56 calls
0x4B5CE0 "city instance add"; and `functions.csv` gives `frustum_test_aabb` as "ECX = &min, EDX = &max", but all three cull
functions call it with ECX = record + 0x10 and EDX = record, while `aabb_point_dist_sq_sse` shows that record + 0x10 is the
maximum corner (check the Rust port's argument order against the original once).

## 6. Fifteen port candidates (render thread unless noted)

Common facts. The thread view state is `fs:[0x2c]` -> slot 0 -> `+0x58` (planes at `+0x740`, eye at `+0x710`). `[0xDB809C]` is an
import slot (value 0xA7EBEE) used as `lock xadd`: stdcall `(int *p, int v)` -> old value. `[0xE8E88A]` selects the SSE3
(`haddps`) variants. Every candidate was checked to be a real function start. "Preserved" lists what the original keeps.

**R1. `__VEC_memcpy_fastcopy` 0xCA326E** (49 % of the thread). Inputs: cdecl `(dst, src, len)`, caller pops (0xCA3347 `add esp,0xC`);
dst and src 16-aligned, len a multiple of 0x80 and non-zero (the count is `len >> 7`, zero would run 2^32 times). Outputs: none (EAX
untouched); memory only. Preserved: ESI, EDI (saved in its frame), EBP, EBX; clobbers ECX, XMM0-XMM7. Callee: none. Compare
polarity: only the `dec ecx / jne` loop. Writes: `dst[0..len)`. Thread: render (and any `_memcpy` caller). Loop: 128 bytes per
iteration, eight `movdqa` loads then eight `movdqa` stores. Recipe: Rust loop with `prefetchnta` 512 bytes ahead on the source and
`movntdq` stores plus one `sfence` at the end (the destination is write-combined D3D memory); keep plain `movdqa` stores when the
destination is not a D3D lock (the function cannot know, so the port belongs in `dynamic_buffers_upload`, R3, with the plain
version as the fallback). Expected gain: this is the biggest single item (up to about half the thread) but memory bound, so
judge it on frame time, not on instruction count.

**R2. `_memcpy` 0xCA0850** (also `_memmove` 0xC96060, `__VEC_memcpy` 0xCA32F5). cdecl `(dst, src, len)` -> EAX = dst (every exit
does `mov eax,[ebp+8]`); overlap-safe (dst above src and below src + len copies backwards via 0xCA0A14); preserved EBP, ESI, EDI;
clobbers ECX, EDX, flags; no x87 / XMM0 output. Callees: `__VEC_memcpy` for len >= 0x100 and equal low nibbles. Writes
`dst[0..len)`. All threads (473 call sites; 1194 samples in `_memcpy` itself on the render thread). Recipe:
`core::ptr::copy` with the above return; zero length must return dst untouched. Cheap, safe, and it removes the CRT dispatch for
the many 16 to 0xC00 byte command copies.

**R3. `dynamic_buffers_upload` 0xD00F20** (NEW). No inputs but the globals `[0x2305948]` (vertex bytes used), `[0x230593C]`
(staging), `[0x230594C]` (D3D vertex buffer object), `[0x252A340]` (index count), `[0x230592C]`, `[0x2305950]`. Outputs: none; EAX, EDX
clobbered, `ECX` pushed / popped. Calls: vertex-buffer `Lock` (`[vtbl+0x2C]`, stdcall `(obj, 0, size, &ptr, 0x2800)`), `_memcpy`, `Unlock`
(`[vtbl+0x30]`), then the same for the index buffer with `count * 2` bytes. Compare polarity: none (no early-out when the sizes are
zero: the original still locks size 0 - keep that or check the device). Writes: the two locked buffers. Thread: render, once per
frame, from `render_frame` 0x52A82E. Recipe: call the same vtable slots from Rust and replace both `_memcpy` calls by R1's streaming
copy; this is the one place where non-temporal stores are valid.

**R4. `city_cull_chunk_instances_split` 0x4B5CE0** (1069 samples). Inputs: cdecl `(entry*)`, caller pops; entry = 0x10 bytes
(`+4` ctx, `+8` chunk index). Outputs: none; byte `ctx[+0x18] + idx*8 + 6` and, through `lock xadd`, the dword at `ctx[+0x18] + idx*8`.
Preserved: EBX, EBP, ESI, EDI; clobbers EAX, ECX, EDX, XMM0-3. Callees: `frustum_test_aabb` (Rust), `[0xDB809C]`, `0xBF63B0` (when
`[0xE8E88A]` is clear). Per instance of the chunk's u16 list (`ctx[+0x10]` node array of 0x30 bytes at `+4`, indices at `+0xC` + node
`+0x1C` * 2, count node `+0xC`): skip if `rec[+0x30] & 0x8000` or `rec[+0x34] & 0x40` (rec = `ctx[+0xC] + idx*0x50`); `vis[6] = frustum(rec+0x10,
rec, planes, 0x27741D0, [0x2527398])`; if `rec[+0x34] & 0x200` is clear and (`[0x2612D74] != 0` or a second frustum test on 0x2612D80 /
0x27785D0 / `[0x252739C]` passes) then `xadd(&vis[0], 0x10000)`; then the squared distance from the eye to the box `(max(min - eye, 0) +
max(eye - max, 0))^2` summed over x, y, z (w masked by 0xE996F0): increment `xadd(&vis[0], 1)` when `dist2 <= [0xE8499C]^2` (x87 `fcompp`, test `ah & 1`
clear; NaN skips). The sum order differs by variant: SSE3 path `(x2 + y2) + (z2 + w2)` (`haddps` twice), fallback `(x2 + z2) +
(y2 + w2)` through 0xBF63B0; port both and select on `[0xE8E88A]`. Loop: per-instance, no SSE per iteration beyond the distance. Thread: the
cull job threads (called by `city_cull_chunks_split_job` 0x4B64F0, a job callback, not necessarily the render thread; uses the
calling thread's view state).

**R5. `city_cull_chunk_instances` 0x4B5C40** (478). Inputs: EDI = entry* (no stack args), plain ret; called in a loop by
`city_cull_chunks_job` 0x4B64C0 with EDI += 0x10. Outputs: `vis[idx*8 + 6]` byte only. Preserved: EBX, EBP, ESI, EDI (ECX
pushed / popped); clobbers EAX, ECX, EDX. Callee: `frustum_test_aabb(ECX = rec+0x10, EDX = rec, planes, 0x27741D0, [0x2527398])` -> AL
stored. Skip filter as R4. Compare polarity: `(flags30 & 0x8000) | (flags34 & 0x40)` non-zero skips. Thread: job.

**R6. `city_view_chunk_instances_cull` 0x4B5FF0** (226). Inputs: cdecl `(entry*)`, entry `+2` u16 = chunk id, `+4` ctx, `+8` node index, `+0xC`
= view index. Output: `lock xadd` on `vis[idx*8]` with `1 << view` (or `0x10001 << view` when `[0x2612D18 + view*0x280] & 0x400` and rec bit
0x200 is clear). Preserved: EBX, EBP, ESI, EDI; clobbers EAX, ECX, EDX. Callees: `city_view_instance_filter_test` 0x4B54A0 (non-zero skips),
`view_bounds_visible(rec)` (ESI = view; AL zero skips), `[0xDB809C]`. The filter index is derived from an object pointer: P =
`rec[+0x48]` when it is set and `rec[+0x30] & 0x10000` is clear, else `rec[+0x40]`; index = `(P - ctx[+4]) / 0x60` (multiply by 0x2AAAAAAB then `sar 4`; low confidence on the stride).

**R7. `aabb_point_dist_sq_sse` 0xBF63B0** (25 callers). Inputs: ECX = &max, EDX = &point, stack = &min (all unaligned vec4, w ignored),
cdecl (caller pops). Output: XMM0.x = `sum_xyz (max(p - max, 0) + max(min - p, 0))^2`, summed as `(x2 + z2) + y2` (w masked to 0 by the
0xE83E50 mask: `movhlps` then `shufps 0xE5`, `addps`); ST0 untouched. Preserved: all GPRs but EAX (loaded with the stack arg); clobbers
XMM0-3; frame uses `and esp,-16`. Compare polarity: `maxps` with 0 (NaN in an operand returns the second operand). No writes beyond
its own frame. Rust: four `_mm_*` ops; reproduce the summation order for bit-exact results.

**R8. `mat44_mul_sse` 0xBE61B0** (17 callers). Inputs: ECX = &A, EDX = &B, stack = &out, `ret 4` (callee pops). Output: 16 floats at out,
`out.row[i] = A.row[i].x*B.row0 + A.row[i].y*B.row1 + A.row[i].z*B.row2 + A.row[i].w*B.row3`, accumulated `((y*B1 + x*B0) + z*B2) + w*B3`
(first `mulps` of the y term, then add the x term, then z, then w). Preserved: all GPRs (EAX used for out); clobbers XMM0-6. Safe when out
aliases A: row i of A is loaded before row i of out is stored. Unaligned loads and stores. No compare, no ST0.

**R9. `mat33_mul_vec3_in_place` 0xBDE500** (134 callers, 356 samples). Inputs: ECX = &v (in / out), EAX = &m (nine floats, row-major),
no stack args, plain ret. Output: v = M * v; `x' = (m1*y + m0*x) + m2*z` (the order of the x87 code: the y term is multiplied first and the x term added to it; y' and z' use rows 1 and 2 in the same shape); x87 with f32 stores (`fstp dword`) of the three inputs first, so v is read once; the sums stay in registers and
are rounded once when stored, so run with 53-bit or 24-bit precision as the process does (the engine sets single precision; verify with a
shadow check). Preserved: all GPRs (EAX, ECX read-only), ESP; x87 stack balanced. No compare. Writes 12 bytes at ECX. Any thread.

**R10. `mesh_material_flags_or` 0x530520** (391). Inputs: ECX = mesh ref `{+0 mesh*, +4 flag, +8 u16 count, +0xC list*}`, no stack args, plain ret.
Output: EAX = OR over the list of `byte[mat_base + idx*0x18 + 0x12] & 0x28`; `mat_base = mesh[+0xC][+4]`; when `ref[+4] == 0` the count is the u16
at `mesh[+2]` and the list is `mesh[+4]`; list entries are 0x10 bytes apart, the u16 index at entry `+0xE`. Preserved: ESI, EDI, EBX,
EBP; clobbers ECX, EDX. Compare: count (zero-extended u16) `<= 0` returns 0. Pure read; scalar loop; SSE no. Callers `city_instance_submit`
(ported; check it still calls the original), 0x508AD0, 0x530580.

**R11. `view_distance_sq_scaled` 0x4B7650** (583). Inputs: EAX = &p, ESI = &q, no stack args, plain ret. Output: ST0 (one x87 value left)
= `float(float(float(dx*dx) + float(dy*dy)) + float(dz*dz)) * 0.5925...` where the double constant is at 0xE5CF28 and each partial is stored
as f32 (`fstp dword`), the final product is stored as f32 and reloaded. Side effect: when `[0x25273B6] != 0` it calls
`gfx_get_thread_state` first (return value unused; keeps TLS init order; the call clobbers ECX, EDX, EAX, so a port must still
clobber EAX like the original or keep calling it). Preserved: ESI, EDI, EBX, EBP. No compare. The `p` copy is taken before the call
(EAX would be clobbered). Fast scalar math; the Rust port must return the value through ST0 (naked / asm shim) - check the existing
`vec3_distance_sq` port for the shim pattern.

**R12. `lower_bound_key64_rec16` 0xBD5C30**. Inputs: ECX = first, EAX = last (byte pointers to 16-byte records), stack = &{u32 lo, u32 hi}
key, plain ret (caller pops; wrapper 0xBD5C00 forwards EAX from its stack arg). Output: EAX = first record not less than the key where
`less(rec, key) = rec.hi < key.hi || (rec.hi == key.hi && rec.lo < key.lo)` (unsigned compares, `setb`), searched by halving the
count `(last - first) >> 4` (signed shift; half = count / 2 rounded toward zero). Preserved: EBX, ESI, EDI, EBP; clobbers ECX, EDX.
No writes. Loop: branchy binary search, ~4 to 8 iterations. Callers: shader parameter setters (55 callers of `shader_param_set`).

**R13. `texture_entry_animated_resolve` 0xC094E0** (791, 61 callers). Inputs: EAX = index (signed), no stack args, plain ret. Output: EAX = pointer
`[0x234890C] + idx*12`, or 0 when `idx < 0` or `idx >= [0x2348908]` (signed compares); for animated entries (`u16 info[+0x10] > 1` and
`u16 entry[+8] == 0`, info = `entry[+4]`) idx is advanced by the signed remainder of `trunc(float(rate * 0.001) * tick)` divided by the frame
count (`idiv`; `rate` = byte `info[+0x1A]`, `tick` = `[0x252A3C8]`, plus 4294967296.0 when negative, the add is skipped when the integer is >= 0). The
truncation is `_ftol2_sse` (`cvttsd2si`, values outside int32 return 0x80000000). Preserved: EBX, ESI, EDI, ECX (pushed); clobbers EDX, EAX,
flags; x87 stack balanced. No writes. Reads shared globals that the game thread updates: take the same loads in the same order.

**R14. `render_item_less` 0x5240F0** (311; 123 of them on the x87 compare). Inputs: EDI = a, ESI = b (items; flags dword at `+4`, f32 key at `+0x10`),
no stack args, plain ret. Output: AL = 1 when a sorts before b (callers test AL only; the other bits of EAX are left over): `ka = ((fa & 0x180)
| ((fa >> 4) & 0x200)) >> 7`; `ka < kb` -> 1, `ka > kb` -> 0; equal: if `fa & 0x20` differs, the item without the flag sorts first
(a without, b with -> 1; a with, b without -> 0); both or neither: `a.key < b.key` strictly (`fcompp; test ah,0x41` clear; NaN or equal -> 0). Preserved:
all GPRs; no writes; no ST0 left. Hot because it runs in insertion-style loops over 0x14-byte items (16 call sites); port the callers
too (0x52B970, 0x52BEA0, 0x52D610 loops) so the compare inlines.

**R15. `morph_target_apply` 0xCF6840** (named, 1584 samples + `normal_ubyte4_add` 662 + `_ftol2_sse` 1046 behind it). Inputs: ECX = target index, EDX =
&weight byte, cdecl stack args `(vertices, morph set, vertex group)` as in the existing note (confirm the argument order from the
two callers at 0xCF7F80 / 0xCF8090 before writing; the body checks `set != 0`, `0 <= group <= set[+0xC]`, `0 <= target < set[+8]`, `set[+0x18] == 2`).
Output: AL (1 on success, 0 on the validation failures: the 0xCF6ACE exit). Preserved: ESI, EDI, EBX, EBP (cookie frame). Callees:
`_ftol2_sse` (ST0 -> EAX), `normal_ubyte4_add` (ESI = &packed), `normal_pack_ubyte4` (Rust). Loop: per vertex of the group, int16 position
deltas scaled by the weight byte (`fild`, `fmul` by the double at 0xE5C2B0; doubles at 0xE5C360, 0xE5C040, 0xE5C790 are loaded as scale / divide constants), three `_ftol2_sse` per vertex, added to
word positions with `add word [..], dx`. All x87 with f32 stores; the whole chain is worth porting together with `morph_target_apply_positions`
0xCF64A0 (433) because it can replace the three `_ftol2_sse` calls with `cvttss2si`. Thread: render (inside `rl_animated_mesh_inst::pre_render`).

## 7. Open items

- Not named although they show in the logs (each below 0.6 %): 0xA8FAD0 (347, tree renderer called from `render_stage1`), 0xA8EAB0, 0xA8D240,
  0x527020 (627 instructions in `render_frame`), 0x524290, 0x526070, 0x52B970, 0xCF32D0 (material/sampler setup, 0x820 bytes),
  0xD10510 / 0xD10100 (sprite drawing), 0xCFC550 (dynamic mesh builder using `normal_pack_ubyte4`), 0xB76090, 0xC0BE60 (no direct callers),
  0x5240D0 (4-float copy, no `call rel32` caller, so left unnamed).
- The memcpy attribution needs the hook counters named in section 3.
- The frustum argument-order note in section 5 should be settled before the next frustum-related port.
