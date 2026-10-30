//! peg check <dir-with-.peg_pc-and-.g_peg_pc>...  parse every peg, round-trip, bounds-check
//! peg info <file.peg_pc>                         list bitmaps
//! peg png <file.peg_pc> <file.g_peg_pc> <outdir> decode every bitmap to PNG

use std::fs;
use std::path::Path;

use sr2_formats::peg::{Peg, decode_rgba};

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut enc = png::Encoder::new(fs::File::create(path)?, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = args.split_first().ok_or("usage: peg <check|info|png> ...")?;
    match cmd.as_str() {
        "check" => {
            let (mut ok, mut bad) = (0, 0);
            let mut formats = std::collections::BTreeMap::new();
            for dir in rest {
                for f in fs::read_dir(dir)? {
                    let path = f?.path();
                    if path.extension().is_none_or(|e| e != "peg_pc") {
                        continue;
                    }
                    let b = fs::read(&path)?;
                    let res = Peg::parse(&b).and_then(|p| {
                        for e in &p.bitmaps {
                            *formats.entry(format!("{:?}", e.format)).or_insert(0) += 1;
                            if e.offset as u64 + e.frame_size as u64 > p.header.data_size as u64 {
                                return Err(sr2_formats::peg::Error::DataOutOfBounds { index: 0 });
                            }
                        }
                        if p.to_bytes() != b {
                            return Err(sr2_formats::peg::Error::BadName);
                        }
                        Ok(())
                    });
                    match res {
                        Ok(()) => ok += 1,
                        Err(e) => {
                            bad += 1;
                            if bad <= 10 {
                                println!("FAIL {}: {e}", path.display());
                            }
                        }
                    }
                }
            }
            println!("{ok} ok, {bad} failed; formats {formats:?}");
        }
        "info" => {
            let p = Peg::parse(&fs::read(&rest[0])?)?;
            println!("{:?}", p.header);
            for (i, e) in p.bitmaps.iter().enumerate() {
                let name = p.names.get(i).map_or("", |s| s.as_str());
                println!(
                    "{i}: {name} {}x{} {:?} mips {} frames {} flags {:#x} off {:#x} size {:#x}",
                    e.width, e.height, e.format, e.mip_levels, e.frames, e.flags, e.offset, e.frame_size
                );
            }
        }
        "png" => {
            let [peg, data, out] = rest else { return Err("usage: peg png <peg> <g_peg> <outdir>".into()) };
            let (p, data) = (Peg::parse(&fs::read(peg)?)?, fs::read(data)?);
            fs::create_dir_all(out)?;
            for (i, e) in p.bitmaps.iter().enumerate() {
                let rgba = decode_rgba(e, p.bitmap_data(i, &data)?)?;
                let name = p.names.get(i).cloned().unwrap_or_else(|| format!("frame{i}"));
                let path = Path::new(out).join(format!("{name}.png"));
                write_png(&path, e.width as u32, e.height as u32, &rgba)?;
                println!("{}", path.display());
            }
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
