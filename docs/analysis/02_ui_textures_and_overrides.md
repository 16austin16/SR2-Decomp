# 02: UI textures, sprite layout, and the override archive

This note covers how the main-menu logo is put together and how `decomp.vpp_pc` replaces game files. All of it is Observed from shipped data and in-game tests unless marked otherwise.

## Peg textures (`sr2_formats::peg`)

- A texture is two files.
  - `name.peg_pc` is the header. It sits in `pegs.vpp_pc`.
  - `name.g_peg_pc` holds the pixel data. It sits in `textures.vpp_pc`, `chunks*.vpp_pc`, and others.
- **Header** (version 10): `GEKV`, u16 version, u16 0, u32 header size, u32 data size (= `.g_peg_pc` size), u16 bitmap count, u16 0, u16 entry count, u16 align (16).
- **Entries**: 0x30-byte bitmap entries follow the header, then NUL-terminated `*.tga` names.
- **Bitmap entry**:
  - +0 offset, +4/+6 width/height, +8 format
  - +0x10 frames, +0x12 flags, +0x1A fps, +0x1B mip levels, +0x1C frame size
  - the rest is 0 on disk
- **Formats**: 400–402 DXT1/3/5, 403 R5G6B5, 404 A1R5G5B5, 405 A4R4G4B4, 406 R8G8B8, 407 A8R8G8B8 (bytes B,G,R,A). Format 410 appears once and is unknown.
- **Validation**: `examples/peg.rs check` parses all 6,740 shipped headers. 6,737 round-trip byte-exact and stay in bounds. The three `ige_*` font pegs fail the bounds check, cause unknown.

## Main-menu logo

- The menu draws from the atlas `ui_mainmenu` (`ui_bms_mainmenu.tga`): 1280×768 A8R8G8B8, 1 mip. Its header is in `pegs.vpp_pc` and its data in `chunks4.vpp_pc`.
- Sprites are rectangles in the atlas, listed in `bitmap_sheets*.xtbl` (`common.vpp_pc`). There is one copy per language suffix (`""`, de, dk, en, es, fr, it, nl, pl, ru).
  - `ui_mainmenu_logo_sr2_words`: (1,148) 501×163
  - `ui_mainmenu_logo_sr2_2`: (1178,350) 95×134
- `main_menu.vint_doc` stores each image element's size as two f32 values. `logo_bmp` = (501.0, 163.0) at file offset 0x1CD; `logo_2_bmp` = (95.0, 134.0) at 0x20A.
  - This element size is what clips the sprite on screen. Changing only the xtbl rect had no visible effect.
  - With both changed, the taller sprite draws undistorted and anchored at the top.

## Override archive

- After `patch.vpp_pc` mounts, the Rust `packfile_mount` also mounts `decomp.vpp_pc` from the game folder when that file exists.
- `packfile_find_record` returns the last match, so files in it replace the shipped copies. This works for headers, `.g_peg_pc` data, xtbl, and vint_doc.
- `crates/sr2-mod` builds `decomp.vpp_pc` from the installed game.
  - It paints "Decomp" (Georgia Bold Italic, chrome gradient, black outline, drop shadow) into the transparent atlas rows under the words sprite.
  - It extends the words sprite by 70 rows in every sheet table and in the vint element.
  - Run it with: `cargo run -p sr2-mod --release -- <game_dir> bin/mod`, then copy `bin/mod/decomp.vpp_pc` into the game folder.
- **Verified in-game**: "Decomp" shows at the bottom right of the logo, level with the foot of the "2". The `find_record` check still matches (53,031 records).
