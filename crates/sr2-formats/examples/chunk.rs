//! chunk dump <archive.vpp_pc> <chunk name>   sections of one chunk, with strings
//! chunk stats <archive.vpp_pc>...            per section type over every chunk:
//!                                            how many chunks have it, sizes, first
//!                                            u32 (usually a count), common strings
//! chunk hex <archive.vpp_pc> <chunk name> <kind> [bytes]   hex dump of one section

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;

use sr2_formats::chunk::{self, Chunk};
use sr2_formats::vpp::Archive;

fn load(path: &str, name: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut r = BufReader::new(File::open(path)?);
    let vpp = Archive::read(&mut r)?;
    let e = vpp.find(name).ok_or_else(|| format!("{name} not in {path}"))?;
    Ok(vpp.read_entry(&mut r, e)?)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

#[derive(Default)]
struct Stat {
    chunks: usize,
    bytes: u64,
    max: u32,
    first_u32: Vec<u32>,
    strings: HashMap<String, usize>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = args.split_first().ok_or("usage: chunk <dump|stats|hex> ...")?;
    match cmd.as_str() {
        "dump" => {
            let [path, name] = rest else {
                return Err("usage: chunk dump <archive> <chunk>".into());
            };
            let data = load(path, name)?;
            let c = Chunk::parse(&data)?;
            println!(
                "{name}: {} bytes, version {:#x}, flags {:#x}, {} objects, bounds {:?}",
                data.len(),
                c.version,
                c.flags,
                c.object_count,
                c.bounds
            );
            let mut secs = c.sections.clone();
            secs.sort_by_key(|s| s.offset);
            for s in &secs {
                let body = &data[s.offset as usize..(s.offset + s.size) as usize];
                let strs = chunk::strings(body, 4);
                println!(
                    "  kind {:#04x}  off {:#08x}  size {:>8}  first u32 {:>10}  strings {:>4}  {:?}",
                    s.kind,
                    s.offset,
                    s.size,
                    u32_at(body, 0).map_or(String::new(), |v| v.to_string()),
                    strs.len(),
                    strs.iter().take(6).collect::<Vec<_>>()
                );
            }
        }
        "stats" => {
            let mut stats: BTreeMap<u32, Stat> = BTreeMap::new();
            let mut total = 0;
            for path in rest {
                let mut r = BufReader::new(File::open(path)?);
                let vpp = Archive::read(&mut r)?;
                for e in vpp
                    .entries
                    .iter()
                    .filter(|e| e.extension.eq_ignore_ascii_case("chunk_pc"))
                {
                    let data = vpp.read_entry(&mut r, e)?;
                    let Ok(c) = Chunk::parse(&data) else {
                        eprintln!("skip {}: not a chunk", e.file_name());
                        continue;
                    };
                    total += 1;
                    for s in &c.sections {
                        let body = &data[s.offset as usize..(s.offset + s.size) as usize];
                        let st = stats.entry(s.kind).or_default();
                        st.chunks += 1;
                        st.bytes += s.size as u64;
                        st.max = st.max.max(s.size);
                        if let Some(v) = u32_at(body, 0) {
                            st.first_u32.push(v);
                        }
                        for x in chunk::strings(body, 4) {
                            *st.strings.entry(x).or_default() += 1;
                        }
                    }
                }
            }
            println!("{total} chunks");
            for (kind, st) in &stats {
                let mut f = st.first_u32.clone();
                f.sort_unstable();
                let median = f.get(f.len() / 2).copied().unwrap_or(0);
                let mut common: Vec<_> = st.strings.iter().collect();
                common.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                println!(
                    "kind {:#04x}: in {:>4} chunks, avg {:>8} B, max {:>8} B, first u32 median {:>6} max {:>8}, {} distinct strings",
                    kind,
                    st.chunks,
                    st.bytes / st.chunks as u64,
                    st.max,
                    median,
                    f.last().copied().unwrap_or(0),
                    st.strings.len()
                );
                if !common.is_empty() {
                    let top: Vec<String> = common.iter().take(12).map(|(s, n)| format!("{s} ({n})")).collect();
                    println!("    {}", top.join(", "));
                }
            }
        }
        "hex" => {
            let (path, name, kind) = match rest {
                [p, n, k, ..] => (p, n, u32::from_str_radix(k.trim_start_matches("0x"), 16)?),
                _ => return Err("usage: chunk hex <archive> <chunk> <kind> [bytes]".into()),
            };
            let n: usize = rest.get(3).map_or(Ok(256), |s| s.parse())?;
            let data = load(path, name)?;
            let c = Chunk::parse(&data)?;
            let s = c.sections.iter().find(|s| s.kind == kind).ok_or("no such section")?;
            let body = &data[s.offset as usize..(s.offset + s.size) as usize];
            for (i, row) in body[..n.min(body.len())].chunks(16).enumerate() {
                let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
                let asc: String = row
                    .iter()
                    .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' })
                    .collect();
                println!("{:06x}  {:<48} {asc}", i * 16, hex.join(" "));
            }
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
