# 01: Packfile (`.vpp_pc`) loader

- Target: `SR2_pc.unpacked.exe` (sha256 `f76b3ccc…870e`), Ghidra 12.1.4 via REA.
- Evidence IDs (`ev_…`) are REA session evidence.
- **Observed** = read directly from decompiler, assembly or bytes. **Inferred** = my interpretation.
- Calling conventions are taken from the assembly where noted. Many of these functions use MSVC whole-program-optimisation custom register conventions, so any detour needs a naked asm thunk.

## 1. Summary

1. **Mount at boot (Observed).** `FUN_0051d800` is the first init call of `game_main_thread_proc`. It mounts 16 archives from a pointer table at **0xe98a28**, in this order:
   - anims, audio, chunks1–4, common, cutscenes, meshes, music1–4, pegs, textures, and **patch last**.
   - Each path is built as `sprintf("%s%s%s", "", name, "_pc")`, e.g. `anims.vpp` + `_pc`.
   - Each path is passed to `packfile_mount` (0xc0b410).
   - Evidence: ev_d0085630, ev_2ef2b05c, ev_b1e1df48, ev_e15af88c.
2. **Magic check.** `packfile_header_install` (0xc0a6e0) is the only `.text` hit for `CE 0A 89 51` (ev_7310affb, ev_9dfb69b8). It accepts `magic == 0x51890ACE && 3 < version < 5`, so **only version 4**.
3. **Lookup.** All files from all mounted packfiles go into one global open-addressed hash table, keyed by a case-insensitive hash of `"name.ext"`.
   - Lookup takes the **last** match in the probe run (Observed).
   - Because `patch.vpp_pc` is mounted last, its files override earlier archives (Inferred).
4. **Compression.** zlib **1.2.1** is linked in (strings `inflate 1.2.1`, `incorrect header check`, `PC cfile zlib decompress`; ev_024a5518).
   - When packfile flag bit `0x1` is set, entry reads go through a chunked inflate path (0xc17ef0).
   - Shipped PC archives have bit 0 clear, so they are never inflated.

## 2. Functions

The "Conv." column gives the calling convention as observed in the assembly. "LTCG" means a custom register convention.

### Packfile system init and mount

| Address | Proposed name | Conv. / signature | Role | Basis / evidence |
|---|---|---|---|---|
| 0x00bfd9e0 | `fs_init` | ? | File-system init; called twice from 0x51d800 with a 5-dword params struct. Calls `packfile_system_init`. | Observed caller of 0xc0aa50, ev_7f476589 |
| 0x00c0aa50 | `packfile_system_init` | LTCG: `EAX = const params*`, returns void | Allocates the global arrays (§4) and two allocators, `"packfile_pool"` (`params[2]` bytes) and `"temp packfile"` (`params[4]` bytes). PC values: `{max_files=0xF000, max_packfiles=0x19, pool=0x400000, max_temp=2, temp_pool=0x100000}`. | Observed, ev_60160a0c |
| 0x00c0b410 | `packfile_mount` | LTCG: `ECX = const char* path`, returns `bool` (AL), no stack args (behaves like `__fastcall(path)`) | Returns 1 if already mounted (`FUN_00c0ae90`). Otherwise opens with `file_open(path,"rb",0)`, takes the next slot in the permanent array, and calls `packfile_load(h,1,1)`. Sets packfile flags 0x8/0x10/0x4 from `handle+0x150` = 0/1/2, then closes the handle (`FUN_00bfdf30`). | Observed asm, ev_7e45afa9, ev_247b107c |
| 0x00c0a970 | `packfile_mount_temp` | LTCG: `EAX = packfile** out`, stack `int* out_handle`, returns bool | Same flow, but into the temp array (`DAT_02348968`) with `packfile_load(h,0,1)`. Remembers the file count at mount time (for unmount). Keeps the handle open. Callers: 0x69d250, 0xc0b640, 0xc0b6f0. | Observed, ev_87e358bf, ev_787d9c7e |
| 0x00c0a7e0 | `packfile_load` | LTCG: `ESI = packfile* slot`, stack `(file_handle* h, bool permanent, bool register)`, caller-cleans, returns bool | Copies name/path from the handle, reads 0x800 header bytes, then calls `packfile_header_install`, then `packfile_alloc_tables(permanent)`. Reads the dir, names and ext into `+0x170/+0x174/+0x178` and calls `packfile_fixup_and_register`. Sets `pf+0x150 = h+0x15C` (stream position = data start). On failure calls `FUN_00bfdf30(h)`. | Observed, ev_f7011864 |
| 0x00c0a6e0 | `packfile_header_install` | cdecl-ish `(packfile* pf, const void* raw_hdr)`, returns bool | Saves `pf` name (+0x8), path (+0x49), flags (+0x14C) and +0x150. Copies the 0x180-byte raw header over `pf`. **Validates magic 0x51890ACE and version 4.** ORs the saved flags back in, restores the name and path, keeps the old +0x150 if flag 0x10, and zeroes +0x170..+0x17C. | Observed, ev_9dfb69b8 |
| 0x00c0a600 | `packfile_alloc_tables` | LTCG: `EDI = packfile*`, stack `bool permanent` | Picks the allocator (0xec5e00 = packfile_pool if permanent, otherwise 0xec5e58 = temp packfile). Reserves `dir+names+ext+16` bytes, then allocates dir/names/ext (align 16) into +0x170/+0x174/+0x178. | Observed, ev_c2ec2b46 |
| 0x00c0a150 | `file_read_sector_aligned` | LTCG: `ECX = size`, stack `void* dst` (stream from context) | Reads `size` bytes as whole 0x800 sectors plus a bounced tail. Used for the header, dir, names and ext. | Observed, ev_bc483ba2 |
| 0x00c0a500 | `packfile_fixup_and_register` | stack `(packfile* pf, bool register)` | Calls `packfile_fixup_string_ptrs`. If `!(flags & 2)` it also calls `packfile_compute_entry_sectors`. Sets every `entry+0x18 = pf`. If `register` is set, appends `{hash("name.ext"), entry*}` to the file-record array and inserts it into the hash. | Observed, ev_5b4b3e15 |
| 0x00c0a1e0 | `packfile_fixup_string_ptrs` | LTCG: `EAX = packfile*` | `entry+0 = names + name_off`, `entry+4 = ext + ext_off`; an offset of `-1` becomes `NULL`. | Observed, ev_ccc890f2 |
| 0x00c0a290 | `packfile_compute_entry_sectors` | LTCG: `EAX = packfile*` | Only when `!(flags & 2)`. Starts at sector `1 + ceil(dir/0x800) + ceil(names/0x800) + ceil(ext/0x800)`. For each entry, `entry+8 = sector`, then `sector += max(1, ceil(sz/0x800))`, where `sz = (flags & 1) ? compressed_size : size`. | Observed, ev_1f2add6c |

### Lookup

| Address | Proposed name | Conv. / signature | Role | Basis / evidence |
|---|---|---|---|---|
| 0x00bf2bd0 | `hash_filename_ci` | LTCG: `EDX = const char*`, returns `u32` | `h = 0; for c: c = tolower_ascii(c); h = rotl(h, 6) ^ c`. A NULL string returns `0xFFFFFFFF`. | Observed, ev_6c664e2d |
| 0x00c0a350 | `packfile_hash_insert` | cdecl `(file_record* rec)` | Bucket = `hash % (bucket_count - 0x67)`, then linear probe to the next empty slot. Tracks the maximum probe length in `DAT_0252a568`. | Observed, ev_2cf43132 |
| 0x00c0a3b0 | `packfile_find_record` | cdecl `(const char* name_ext)`, returns `file_record*` or NULL | Hashes the name, probes until an empty slot, and on each hash match rebuilds `"name.ext"` (`FUN_00c0ad10`, into a per-thread TLS buffer at `[fs:0x2C][0]+0x18`) and `_stricmp`s it. Entries whose full name is empty are skipped. **Returns the last match.** The probe runs linearly from the bucket to the end of the slot array with **no wrap-around**; if it runs off the end without meeting an empty slot it returns **NULL even if it matched** (Observed, hand-decoded asm). Replaced by Rust. | Observed, ev_0d7fb2a6 |
| 0x00c0ad80 | `packfile_find_entry` | LTCG: `ECX = const char* path`, caller-clean, returns `vpp_entry*` | Normalises the path into a 64-char buffer (`FUN_00bff700`). First linear-scans the "priority" packfile `DAT_02348944`, then falls back to `packfile_find_record(...)->entry`. | Observed asm, ev_464ee056 |
| 0x00c0ad10 | `vpp_entry_full_name` | LTCG: `ECX = entry*`, `ESI = out buf` | `sprintf("%s.%s")`, or just the name when ext is empty | Observed, ev_136e2d15 |

### Generic file layer (vlib-style)

| Address | Proposed name | Conv. / signature | Role | Basis / evidence |
|---|---|---|---|---|
| 0x00bfdc20 | `file_open` | LTCG: `ECX = const char* path`, stack `(const char* mode, bool flag)`, caller-clean, returns `file_handle*` | `mode` must be 2 chars from `[rwa][tb]`. Allocates a handle (`FUN_00bfd710`), resolves the source (`file_resolve`), opens it (`FUN_00bfd930`), and fills the handle fields (§3.3). | Observed, ev_0889d284 |
| 0x00bfd880 | `file_resolve` | LTCG: `EAX = location*`, stack `name` | Walks the search-path table `DAT_0234767c` (`{kind, arg}` pairs, count `DAT_0252a540`). Kind 0 = loose disk file (`fopen`/`ftell`, 0xc18130). Kind 1 = packfile (0xbfd7a0, which calls `packfile_find_entry`). Kind 2 = memory (0xbfd800). First hit wins. | Observed, ev_437f94d3 |
| 0x00bfd7a0 | `file_resolve_in_packfile` | LTCG: `ESI = location*`, stack `(name, arg)` | Sets `loc.kind = 1`, `loc.entry = packfile_find_entry(name)`, `loc.size = entry->size` | Observed, ev_7dffc087 |
| 0x00c18130 | `file_resolve_on_disk` | LTCG: `EDI = location*`, stack `(path, int)` | `fopen(path,"rb")` succeeds, then `loc.kind = 0` and `loc.size = ftell` | Observed, ev_7ca00190 |
| 0x00bfe170 | `file_read` | LTCG: `EAX = size`, stack `(void* dst, file_handle* h)`, returns bytes read | Clamps to the size. For an OS file calls `FUN_00c18500`. For a packfile entry calls `packfile_stream_read` (0xc18450). For memory does a `memcpy`. Advances `h+0x15C` and sets the EOF/error state at `h+0x158`. | Observed, ev_783a162e |
| 0x00c18450 | `packfile_stream_read` | LTCG: `EAX = {os_file*, entry*}`, `ECX = mode (0=seek+read, 1=continue)`, stack `dst` | File offset = `entry->parent->data_start (+0x150) + entry->data_off (+0xC)`. With mode 0 it seeks (`os_file_seek` 0xc18590). If the parent's `flags & 1` it calls `os_file_read_inflate` (0xc17ef0), otherwise `os_file_read` (0xc17e00). | Observed, ev_8bf67ac3 |
| 0x00c17e00 | `os_file_read` | LTCG: `ESI = os_file*`, `ECX = dst`, `EDI = size`, stack `offset` | `ReadFile`. When `os_file.flags & 1` it passes an OVERLAPPED with `Offset = offset` (async). Errors set `flags |= 2`. | Observed, ev_3bf42255 |
| 0x00c17ef0 | `os_file_read_inflate` | LTCG: `EDX = os_file*`, `ECX = compressed size`, stack `(dst, offset)` | `inflateInit` (`FUN_00c20a60`). Reads in 0x8000-byte double-buffered chunks and calls inflate (`FUN_00c20ad0`) into dst, with an output limit of 0x6400000. | Observed, ev_3a17049f |
| 0x00c181c0 | `os_file_open` | stack `(LPCSTR path, const char* mode, bool async)`, returns `os_file*` | `CreateFileA(path, r ? GENERIC_READ : GENERIC_WRITE, FILE_SHARE_READ, …, 0x08000080 SEQUENTIAL_SCAN|NORMAL)`. `async` creates an event for the OVERLAPPED. Guarded by CS `DAT_0340cbe4`. | Observed, ev_95c39604 |
| 0x00c18590 | `os_file_seek` | ? | `SetFilePointer` wrapper | Observed (caller list), ev_96957fbd |

## 3. Recovered structures

### 3.1 `vpp_packfile` (0x180 bytes; the first 0x180 bytes of the on-disk header, patched at runtime)

| Off | Type | On disk | Runtime meaning | Basis |
|---|---|---|---|---|
| 0x000 | u32 | magic 0x51890ACE | same | Observed |
| 0x004 | u32 | version 4 | same | Observed |
| 0x008 | char[0x41] | (zeros/junk) | short name, copied from the file handle | Observed (0xc0a7e0, 0xc0a6e0) |
| 0x049 | char[0x100] | — | full path | Observed |
| 0x14C | u32 | flags 0/2 | see §3.4 | Observed |
| 0x150 | u32 | (0) | **absolute file offset of the data section** = stream position after reading header+dir+names+ext | Observed (0xc0a7e0 sets it; 0xc18450 adds it) |
| 0x154 | u32 | entry_count | same | Observed |
| 0x158 | u32 | archive_size | (not touched here) | — |
| 0x15C | u32 | dir_size | same | Observed |
| 0x160 | u32 | names_size | same | Observed |
| 0x164 | u32 | ext_size | same | Observed |
| 0x168 | u32 | data_size | — | — |
| 0x16C | u32 | compressed_data_size (-1) | — | — |
| 0x170 | `vpp_entry*` | 0 | **directory array** (allocated from the packfile pool) | Observed |
| 0x174 | char* | 0 | **names blob** | Observed |
| 0x178 | char* | 0 | **extensions blob** | Observed |
| 0x17C | u32 | 0 | zeroed on install; use unknown | Observed zeroing |

Because of the 0x800-byte sector read, everything past 0x180 in the header is read into a stack buffer and discarded.

### 3.2 `vpp_entry` (28 bytes)

| Off | On disk | Runtime | Basis |
|---|---|---|---|
| 0x00 | name_off | `char* name` (NULL if -1) | Observed 0xc0a1e0 |
| 0x04 | ext_off | `char* ext` (NULL if -1) | Observed |
| 0x08 | unk_08 (0) | **start sector** (0x800 units from the file start). Only computed when the packfile is *not* `flags & 2`. Not used by the sync read path, which uses data_off. | Observed 0xc0a290 |
| 0x0C | data_off | offset relative to `packfile+0x150` | Observed 0xc18450 |
| 0x10 | size | uncompressed size | Observed |
| 0x14 | compressed_size (-1) | used as the read length when `flags & 1` | Observed |
| 0x18 | unk_18 (0) | **`vpp_packfile* parent`** | Observed 0xc0a500 |

### 3.3 Other runtime records

- **`file_record`** (8 bytes, array `DAT_02348958`): `{u32 hash; vpp_entry* entry;}`.
- **`file_handle`**, partial (from `file_open`/`file_read`):

  | Off | Meaning |
  |---|---|
  | +0x000 | name[0x41] |
  | +0x041 | path |
  | +0x144 | location kind |
  | +0x148 | source: 0 = OS file, 1 = packfile, 2 = memory |
  | +0x14C | `{os_file*, entry*}`* or memory base |
  | +0x150 | OS file / sub-kind |
  | +0x154 | open mode: 1 = read, 2 = write |
  | +0x158 | state: 0 = ok, 1 = EOF, 3 = aborted |
  | +0x15C | position |
  | +0x160 | size |
  | +0x168 | flags (bit 8 = abort) |

- **`os_file`**:

  | Off | Meaning |
  |---|---|
  | +0x00 | HANDLE |
  | +0x04 | CreateFile flags |
  | +0x08 | state (1 = async, 2 = I/O pending, 8 = abort) |
  | +0x0C | OVERLAPPED (Offset at +0x14, hEvent at +0x1C) |

### 3.4 Packfile flags (+0x14C)

| Bit | Meaning | Basis |
|---|---|---|
| 0x01 | **compressed**: entries are zlib streams of `compressed_size` bytes, read through `os_file_read_inflate`; sector sizing uses compressed_size | Observed code paths. Not set in shipped PC data. |
| 0x02 | **condensed**: no per-file sector alignment, so `packfile_compute_entry_sectors` is skipped and files are packed back-to-back from data_off | The skip is Observed; "condensed" is Inferred (matches the shipped flag=2 and the Volition v4 naming) |
| 0x04 / 0x08 / 0x10 | runtime-only, set by mount from `handle+0x150` (2 / 0 / 1). 0x10 also makes `packfile_header_install` keep the old +0x150. | Observed; meaning unknown |

## 4. Globals

| Address | Meaning |
|---|---|
| `DAT_02348978` / `…970` / `…96C` | permanent packfile array (0x180 each) / count / max (0x19) |
| `DAT_02348968` / `…960` / `…95C` | temp packfile array / count / max (2) |
| `DAT_02348974` | per-permanent 12-byte record (allocator marks before/after; Inferred) |
| `DAT_02348964` | per-temp u32 = file-record count at mount |
| `DAT_02348958` / `…954` / `…950` | file_record array / count / max (0xF000) |
| `DAT_02348948` / `…94C` | hash slots (u32 `file_record*`) / slot count = `2*max_files + 100` |
| `DAT_02348944` | priority packfile, scanned linearly before the hash |
| `0xec5e00`, `0xec5e58` | allocator objects "packfile_pool" / "temp packfile" (vtable: +0x00 reset, +0x18 reserve, +0x1C alloc(size, align), +0x30 mark) |
| `0xe98a28` | `const char* boot_packfiles[16]` |

## 5. Notes for the Rust side / detours

- `vpp.rs` matches the loader's view: version 4 only, sections at 0x800 alignment, entry offset = `align(...)` data start + data_off. The loader effectively computes `data_start` the same way, by reading each section padded to whole sectors.
- Suggested additions to `vpp.rs`:
  - Rename `unknown_08` to "start_sector (runtime)" and `unknown_18` to "parent ptr (runtime)".
  - Name flag bit 1 = compressed, bit 2 = condensed.
  - Implement zlib entries if modded archives need them.
- File-name matching is case-insensitive on `"name.ext"`, with later mounts overriding earlier ones. Mirror this in a VFS layer: archive list order with patch last, plus loose disk files depending on the search-path table order (to be determined).
- Detour candidates:
  - `packfile_mount` is the cleanest: path in ECX, bool in AL, no stack args, so a `__fastcall` hook works.
  - `packfile_find_record` is plain cdecl.
  - `file_open`, `packfile_find_entry` and `file_read` use custom register conventions and need naked thunks.

## 5a. `packfile_mount` (0xC0B410) hand decode, for the Rust port

Hand-decoded from the bytes (Observed). The stack frame has an `/GS` cookie (`[0xE80B50]`).

1. `edi = ecx = path`. Calls `0xC0B120(ECX=path)`. If it returns AL=0, return 0.
   - `0xC0B120` returns 0 for a NULL path, then normalises the path into a 0x40 stack buffer through `0xBFF700`. The rest is not decoded yet.
2. If `count (0x2348970) >= max (0x234896C)`, return 0.
3. `0xBFF700(buf[0x41], 0x41)` (cdecl, path passed in EDI/implicitly; strips the directory at the last `\` or `/` and copies the name). Then `0xC0AE90(buf)` (cdecl, `_stricmp` against `pf+0x8` of each mounted packfile; returns `packfile*` or 0). If already mounted, return 1.
4. `h = file_open(ECX=path, "rb", 0)`. If NULL, return 0.
5. `slot = array + count*0x180`, `count += 1`, then `bl = packfile_load(ESI=slot, h, 1, 1)`.
6. On failure (`bl == 0`): `count -= 1` and return 0. The handle is not closed here; `packfile_load` closes it on failure.
7. On success, switch on `h+0x150`: 0 → `flags |= 8`, 1 → `flags |= 0x10`, 2 → `flags |= 4` (other values: no flag). Then `0xBFDF30(h)` (cdecl close) and return `bl`.

**Ported (hooks.rs).** `0xC0B120`, `0xBFF700` and `0xC0AE90` are reimplemented in Rust. `file_open`, `packfile_load` and `file_close` are called through asm shims.

- **Lesson:** the first port crashed inside `packfile_load` (0xBFE611, bad `os_file*` at handle+0x14C) until the shims saved and restored EBX/ESI/EDI/EBP around the call. LTCG callees are not guaranteed to preserve callee-saved registers, so **every shim into game code must save all four**.
- **Verification:** an observe-only trampoline logged each mount's result, slot fields, an FNV fingerprint of the directory, and the record table (`golden/boot_mounts.txt`). The Rust build's log is byte-identical.
- **Not yet exercised:** the already-mounted path, the failure paths, and later mounts (city load).

## 6. Open questions

- Who calls `packfile_mount_temp`: 0x69d250 (also calls `packfile_mount`), 0xc0b640, 0xc0b6f0. These are probably the streaming/city-load paths (`city_load.vpp`, `misc_tables.vpp`). Not yet traced.
- How the remaining archives get mounted (`shaders`, `startup`, `vint_docs`, `misc_tables_startup`, which are not in the 0xe98a28 table).
- The search-path table `DAT_0234767c` contents and order (does disk override packfile?).
- `FUN_00bff700` (path normalisation: strip directory? lowercase?), `FUN_00c0ae90` (is-mounted check), and `FUN_00bfd930`/`FUN_00bfdf30` (open/close).
- The async streaming path (OVERLAPPED reads, `entry+8` sector use) needs a separate pass.
