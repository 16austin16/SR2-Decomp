# 05 — World streamer (`streamman`)

Status: mapping started 2026-10-30. Nothing ported yet.

The streamer sits on top of the file API (all Rust now). It decides which world chunks,
pegs (textures), meshes, characters, vehicles and animations are resident, and loads them
through the packfile layer.

## Where the code is

| Area | Address range | Evidence |
|---|---|---|
| streamman2 core + loading-info classes | ~`0xA84000`–`0xA89000` | vtables below; strings `streamman2 reading resource %s` (`0xE44504`), `… chunk extras reading resource %s` (`0xE444B4`), `… vehicles reading resource %s` (`0xE44540`) |
| Chunk file loader (`.chunk_pc`, BBCHUNK header + CHUNK_VERSION check) | ~`0x4AF2E0`–`0x4AF630` | "does not appear to be a BBCHUNK file", "CHUNK_VERSION is too low/high" |
| Chunk / superchunk pool set-up (cpu/gpu pools: chunk0..2, always loaded, anim, mp) | ~`0x51E300`–`0x51E930` | pool name strings `chunk0 cpu` … `mp_gb_chunk gpu` |
| `g_chunk_pc` / `_ml.chunk` / `_al.chunk` names | ~`0x6992EF`, `0x69C5BE`–`0x69C720` | |
| Chunk swap sequences (`chunk_swap_sequences.xtbl`) | ~`0x542005` | |
| Component streaming + "Temp component stream pool" | ~`0x9E4364`–`0x9E82BB` | |
| Chunk limits lookup (`sr2_chunk%s`, "Unable to find chunk limit") | ~`0x9D8E20`–`0x9D8F66` | |

## Loading-info classes (2 virtuals each)

| Class | vtable | slot 0 | slot 1 |
|---|---|---|---|
| `streamman_resource_loading_info_base` | `0xE44714` | `0xA887C0` | `0xA887D0` |
| `…chunk_geom_loading_info` | `0xE44720` | `0xA85330` | `0xA887D0` |
| `…chunk_peg_loading_info` | `0xE4472C` | `0xA85B10` | `0xA85AB0` |
| `…chunk_extras_loading_info` | `0xE44738` | `0xA84900` | `0xA84860` |
| `…peg`, `…static_mesh`, `…resource_character`, `…resource_logo`, `…resource_vehicle`, `…animation` | `0xE44744`…`0xE44780` | `0xA887C0` | `0xA887D0` |

Only the three chunk loaders override the base. Slot 1 of `chunk_extras` (`0xA84860`) strips
the extension from the name at `this+0x6F0` (64-char buffer), calls vtable slot 0, then
`0xA84510(buf, *(u16*)(this+0x772), name, 0xFFFF)`.

Ghidra hasn't made functions at most of these slot targets yet (vtable-only references), so
they need disassembling by hand or defining first.

## Side note: D3D wrapper

`IDirect3DDevice9Lost` (vtable `0xE584A4`, 119 slots) is a small null-object device: all its
methods together are about 1.3 KB of stubs. It is not the renderer. `gr_vertex_buffer` /
`gr_index_buffer` are about 270 bytes each.
