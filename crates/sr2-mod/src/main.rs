//! Build `decomp.vpp_pc`: the main-menu logo with "Decomp" added at its bottom right.
//!
//! usage: sr2-mod <game_dir> <out_dir> [font.ttf]
//!
//! The menu draws "Saints Row" from the atlas `ui_mainmenu` (sprite
//! `ui_mainmenu_logo_sr2_words`, rect (1,148) 501x163 in `bitmap_sheets.xtbl`) with
//! the "2" sprite under "Row". The text is painted into the unused, transparent
//! atlas area just below the words, and the words sprite is made taller to take it in:
//! its rect in every `bitmap_sheets*.xtbl` and its element size in `main_menu.vint_doc`.
//! The hook mounts the archive after `patch.vpp_pc`, so these copies win.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use sr2_formats::peg::{self, Format, Peg};
use sr2_formats::vpp::{self, Archive};

const ATLAS_W: usize = 1280;
/// `ui_mainmenu_logo_sr2_words` in the atlas.
const WORDS: (usize, usize, usize, usize) = (1, 148, 501, 163);
/// Extra rows added below the words sprite for the text.
const EXTRA_H: usize = 70;
/// Right edge and bottom of the text inside the extended sprite (atlas coords).
const TEXT_RIGHT: usize = 494;
const TEXT_BOTTOM: usize = WORDS.1 + WORDS.3 + EXTRA_H - 4;
const TEXT_PX: f32 = 50.0;
/// Suffixes of the `bitmap_sheets*.xtbl` copies in common.vpp_pc.
const SHEET_LANGS: [&str; 10] = ["", "de", "dk", "en", "es", "fr", "it", "nl", "pl", "ru"];

fn read_from(game: &Path, archive: &str, file: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut r = BufReader::new(File::open(game.join(archive))?);
    let vpp = Archive::read(&mut r)?;
    let e = vpp.find(file).ok_or_else(|| format!("{file} not in {archive}"))?;
    Ok(vpp.read_entry(&mut r, e)?)
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut enc = png::Encoder::new(File::create(path)?, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// A single-channel image, 0..=1.
struct Mask {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Mask {
    fn new(w: usize, h: usize) -> Self {
        Mask { w, h, v: vec![0.0; w * h] }
    }

    fn at(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0.0
        } else {
            self.v[y as usize * self.w + x as usize]
        }
    }

    /// Max over a disc of radius `r`.
    fn dilate(&self, r: isize) -> Mask {
        let mut out = Mask::new(self.w, self.h);
        for y in 0..self.h as isize {
            for x in 0..self.w as isize {
                let mut m = 0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy <= r * r + r {
                            m = m.max(self.at(x + dx, y + dy));
                        }
                    }
                }
                out.v[y as usize * self.w + x as usize] = m;
            }
        }
        out
    }

    /// Box blur of radius `r`, shifted by (`ox`, `oy`).
    fn blur_shift(&self, r: isize, ox: isize, oy: isize) -> Mask {
        let mut out = Mask::new(self.w, self.h);
        let n = ((2 * r + 1) * (2 * r + 1)) as f32;
        for y in 0..self.h as isize {
            for x in 0..self.w as isize {
                let mut s = 0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        s += self.at(x - ox + dx, y - oy + dy);
                    }
                }
                out.v[y as usize * self.w + x as usize] = s / n;
            }
        }
        out
    }
}

/// Coverage mask of `text`, with its ink box's bottom-right at (`right`, `bottom`) of a `w`x`h` canvas.
fn render_text(font: &FontVec, text: &str, px: f32, w: usize, h: usize, right: f32, bottom: f32) -> Mask {
    let sf = font.as_scaled(PxScale::from(px));
    let mut glyphs = Vec::new();
    let mut x = 0f32;
    let mut prev = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(sf.scale(), point(x, sf.ascent())));
        x += sf.h_advance(id);
        prev = Some(id);
    }
    let outlined: Vec<_> = glyphs.into_iter().filter_map(|g| font.outline_glyph(g)).collect();
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for o in &outlined {
        let b = o.px_bounds();
        max_x = max_x.max(b.max.x);
        max_y = max_y.max(b.max.y);
    }
    let (dx, dy) = (right - max_x, bottom - max_y);
    let mut m = Mask::new(w, h);
    for o in &outlined {
        let b = o.px_bounds();
        o.draw(|gx, gy, c| {
            let (x, y) = ((b.min.x + dx) as isize + gx as isize, (b.min.y + dy) as isize + gy as isize);
            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                let v = &mut m.v[y as usize * w + x as usize];
                *v = v.max(c);
            }
        });
    }
    m
}

/// Chrome ramp like the logo's: bright top, dark horizon line, bright bottom.
fn chrome(t: f32) -> [f32; 3] {
    const STOPS: [(f32, f32); 6] = [(0.0, 1.0), (0.40, 0.86), (0.50, 0.38), (0.56, 0.52), (0.80, 0.80), (1.0, 0.97)];
    let t = t.clamp(0.0, 1.0);
    let i = STOPS.iter().position(|s| s.0 >= t).unwrap_or(STOPS.len() - 1).max(1);
    let (a, b) = (STOPS[i - 1], STOPS[i]);
    let v = a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0).max(1e-6));
    [v * 0.98, v * 0.99, v]
}

/// Straight-alpha "over" of `src` (rgb 0..1, alpha 0..1) onto an RGBA8 pixel.
fn over(dst: &mut [u8], rgb: [f32; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    for i in 0..3 {
        let d = dst[i] as f32 / 255.0;
        dst[i] = (((rgb[i] * a + d * da * (1.0 - a)) / oa) * 255.0).round() as u8;
    }
    dst[3] = (oa * 255.0).round() as u8;
}

/// Paint the text into the atlas (RGBA8, `ATLAS_W` wide).
fn paint_decomp(atlas: &mut [u8], font: &FontVec) -> Result<(), Box<dyn Error>> {
    let (x0, y0, w) = (WORDS.0, WORDS.1, WORDS.2);
    let h = WORDS.3 + EXTRA_H;
    // The added rows must be unused (fully transparent) atlas space.
    for y in y0 + WORDS.3..y0 + h {
        for x in x0..x0 + w {
            if atlas[(y * ATLAS_W + x) * 4 + 3] != 0 {
                return Err(format!("atlas pixel ({x},{y}) below the logo is not transparent").into());
            }
        }
    }
    let local_right = (TEXT_RIGHT - x0) as f32;
    let local_bottom = (TEXT_BOTTOM - y0) as f32;
    let fill = render_text(font, "Decomp", TEXT_PX, w, h, local_right, local_bottom);
    let outline = fill.dilate(3);
    let shadow = outline.blur_shift(2, 3, 4);
    let (top, bottom) = {
        let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| fill.v[y * w + x] > 0.0)).collect();
        (*rows.first().unwrap_or(&0) as f32, *rows.last().unwrap_or(&0) as f32)
    };
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            let px = &mut atlas[((y0 + y) * ATLAS_W + x0 + x) * 4..][..4];
            // Only paint where the original art is absent, so the words stay untouched.
            if y < WORDS.3 && px[3] != 0 {
                continue;
            }
            over(px, [0.0; 3], shadow.v[k] * 0.6);
            over(px, [0.03; 3], outline.v[k]);
            over(px, chrome((y as f32 - top) / (bottom - top).max(1.0)), fill.v[k]);
        }
    }
    Ok(())
}

/// In one sprite sheet table, set `ui_mainmenu_logo_sr2_words`' ImageHeight to the extended height.
/// In `main_menu.vint_doc`, the `logo_bmp` element's size is stored as two f32s
/// (501.0, 163.0) at this offset. Its height is what clips the sprite on screen.
const VINT_LOGO_SIZE_AT: usize = 0x1CD;

fn patch_vint(doc: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let f = |v: f32| v.to_le_bytes();
    let old = [f(WORDS.2 as f32), f(WORDS.3 as f32)].concat();
    if doc.get(VINT_LOGO_SIZE_AT..VINT_LOGO_SIZE_AT + 8) != Some(&old[..]) {
        return Err("main_menu.vint_doc: logo size not at the expected offset".into());
    }
    let mut out = doc.to_vec();
    out[VINT_LOGO_SIZE_AT + 4..][..4].copy_from_slice(&f((WORDS.3 + EXTRA_H) as f32));
    Ok(out)
}

fn patch_xtbl(xtbl: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let s = String::from_utf8_lossy(xtbl);
    let name = "<Name>ui_mainmenu_logo_sr2_words</Name>";
    let at = s.find(name).ok_or("words sprite not in bitmap_sheets.xtbl")?;
    let old = format!("<ImageHeight>{}</ImageHeight>", WORDS.3);
    let rel = s[at..].find(&old).ok_or("ImageHeight not found")?;
    if rel > 200 {
        return Err("ImageHeight is not in the words sprite's block".into());
    }
    let new = format!("<ImageHeight>{}</ImageHeight>", WORDS.3 + EXTRA_H);
    let mut out = xtbl.to_vec();
    out.splice(at + rel..at + rel + old.len(), new.bytes());
    Ok(out)
}

/// Approximate on-screen composite (words + "2" over the menu's gold) for checking by eye.
fn preview(atlas: &[u8], out: &Path) -> Result<(), Box<dyn Error>> {
    let (pw, ph) = (520usize, 230usize);
    let mut img: Vec<u8> = (0..pw * ph).flat_map(|_| [92u8, 70, 22, 255]).collect();
    let mut blit = |sx: usize, sy: usize, w: usize, h: usize, dx: usize, dy: usize| {
        for y in 0..h {
            for x in 0..w {
                let s = &atlas[((sy + y) * ATLAS_W + sx + x) * 4..][..4];
                if dx + x < pw && dy + y < ph {
                    let d = &mut img[((dy + y) * pw + dx + x) * 4..][..4];
                    over(d, [s[0] as f32 / 255.0, s[1] as f32 / 255.0, s[2] as f32 / 255.0], s[3] as f32 / 255.0);
                }
            }
        }
    };
    blit(WORDS.0, WORDS.1, WORDS.2, WORDS.3 + EXTRA_H, 8, 8);
    blit(1178, 350, 95, 134, 8 + 213, 8 + 135);
    write_png(out, pw, ph, &img)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, out, rest @ ..] = &args[..] else { return Err("usage: sr2-mod <game_dir> <out_dir> [font.ttf]".into()) };
    let font_path = rest.first().map_or("C:/Windows/Fonts/georgiaz.ttf", |s| s.as_str());
    let (game, out) = (Path::new(game), Path::new(out));
    fs::create_dir_all(out)?;

    let peg_bytes = read_from(game, "pegs.vpp_pc", "ui_mainmenu.peg_pc")?;
    let data = read_from(game, "chunks4.vpp_pc", "ui_mainmenu.g_peg_pc")?;

    let peg = Peg::parse(&peg_bytes)?;
    let bm = &peg.bitmaps[0];
    if (bm.width, bm.height, bm.format, bm.mip_levels, peg.bitmaps.len())
        != (ATLAS_W as u16, 768, Format::A8R8G8B8, 1, 1)
    {
        return Err(format!("unexpected ui_mainmenu layout: {bm:?}").into());
    }
    let mut atlas = peg::decode_rgba(bm, peg.bitmap_data(0, &data)?)?;

    let font = FontVec::try_from_vec(fs::read(font_path)?)?;
    paint_decomp(&mut atlas, &font)?;
    preview(&atlas, &out.join("decomp_logo_preview.png"))?;

    // Same layout and size as the original, so the peg header is unchanged.
    let new_data = peg::encode_a8r8g8b8(&atlas);
    assert_eq!(new_data.len(), data.len());
    // Each language has its own copy of the sprite sheet table (English: "en").
    let mut sheets = Vec::new();
    for lang in SHEET_LANGS {
        let name = format!("bitmap_sheets{lang}");
        let xtbl = read_from(game, "common.vpp_pc", &format!("{name}.xtbl"))?;
        sheets.push((name, patch_xtbl(&xtbl)?));
    }
    let vint = patch_vint(&read_from(game, "common.vpp_pc", "main_menu.vint_doc")?)?;
    let peg_header = peg.to_bytes();
    let mut files: Vec<(&str, &str, &[u8])> =
        vec![("ui_mainmenu", "peg_pc", &peg_header), ("ui_mainmenu", "g_peg_pc", &new_data)];
    files.extend(sheets.iter().map(|(n, b)| (n.as_str(), "xtbl", b.as_slice())));
    files.push(("main_menu", "vint_doc", &vint));
    let archive = vpp::write(&files);
    fs::write(out.join("decomp.vpp_pc"), &archive)?;
    println!("wrote {} ({} bytes)", out.join("decomp.vpp_pc").display(), archive.len());
    Ok(())
}
