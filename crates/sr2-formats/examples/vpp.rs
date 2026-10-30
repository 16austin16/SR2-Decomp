//! vpp list <archive.vpp_pc>...          list entries
//! vpp check <archive.vpp_pc>...         parse and sanity-check every archive
//! vpp extract <archive.vpp_pc> <outdir> extract all entries

use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use sr2_formats::vpp::Archive;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = args.split_first().ok_or("usage: vpp <list|check|extract> ...")?;
    match cmd.as_str() {
        "list" => {
            for path in rest {
                let vpp = Archive::read(&mut BufReader::new(File::open(path)?))?;
                for e in &vpp.entries {
                    println!("{:>10}  {:#010x}  {}", e.size, e.offset, e.file_name());
                }
            }
        }
        "check" => {
            for path in rest {
                let len = fs::metadata(path)?.len();
                let vpp = Archive::read(&mut BufReader::new(File::open(path)?))?;
                let h = &vpp.header;
                let end = vpp.data_start + h.data_size as u64;
                let max = vpp.entries.iter().map(|e| e.offset + e.size as u64).max().unwrap_or(0);
                let ok = h.archive_size as u64 == len && end <= len && max <= end;
                println!(
                    "{} {:<20} entries={:<6} flags={} data_start={:#x} data_end={:#x} file={:#x}",
                    if ok { "ok  " } else { "FAIL" },
                    Path::new(path).file_name().unwrap().to_string_lossy(),
                    vpp.entries.len(),
                    h.flags,
                    vpp.data_start,
                    end,
                    len
                );
            }
        }
        "extract" => {
            let [path, out] = rest else { return Err("usage: vpp extract <archive> <outdir>".into()) };
            let mut r = BufReader::new(File::open(path)?);
            let vpp = Archive::read(&mut r)?;
            fs::create_dir_all(out)?;
            for e in &vpp.entries {
                fs::write(Path::new(out).join(e.file_name()), vpp.read_entry(&mut r, e)?)?;
            }
            println!("extracted {} files", vpp.entries.len());
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
